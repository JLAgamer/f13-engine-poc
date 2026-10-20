//! f13-installer: sets f13recomp up from the player's own unpacked copy of Friday the 13th: The Game, then plays.
//!
//! The player picks the game's folder (its content extracted to loose files under `SummerCamp/Content`); Install then
//! runs the exporter (`tools/f13recomp-export/f13recomp-export.exe`, self-contained .NET) once per step (STEPS):
//! 1. `tuning`: the gameplay values, into `~/.f13recomp/tuning.json`;
//! 2. the rest (`mvp`, the counselors, Jasons, sounds, menus, the intro and outro, the particle systems, the
//!    precomputed lighting) into `export/` next to this executable (where the game looks without `--export`).
//!
//! Play starts the game. The folder picked is kept in `installer.json` beside the executable.
//!
//! The release zip names the programs for players (tools/package.py): f13recomp.exe (f13-game), f13recomp-setup.exe
//! (this), tools/f13recomp-export/f13recomp-export.exe; a source checkout keeps the f13- names.
//!
//! In the release zip the exporter sits in `tools/` next to this executable; run from a source checkout
//! (`target/release`), the exporter's own Release build is used.
#![windows_subsystem = "windows"]

use eframe::egui;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc::{channel, Receiver, Sender};

/// The exporter's commands, in order (particles reads the levels mvp writes), and what each is shown as.
const STEPS: [(&str, &str); 10] = [
    ("tuning", "Reading the gameplay values"),
    ("mvp", "Exporting the map, characters, sounds and HUD"),
    ("counselors", "Exporting the counselors"),
    ("killers", "Exporting the Jasons and their weapons"),
    // (Again after them: their voices and their animations' sounds.)
    ("sounds", "Exporting the counselors' and Jasons' sounds"),
    ("ui", "Exporting the menus"),
    ("sequences", "Exporting the intro and outro"),
    ("particles", "Exporting the particle effects"),
    // (The precomputed lighting's: the placed meshes' vertices, and the map's Enlighten data, which read the levels mvp writes.)
    ("enlighten-meshes", "Exporting the lighting's meshes"),
    ("enlighten", "Exporting the precomputed lighting"),
];

/// Windows: the exporter's console window stays hidden (CREATE_NO_WINDOW).
#[cfg(windows)]
fn hidden(cmd: &mut Command) -> &mut Command {
    use std::os::windows::process::CommandExt;
    cmd.creation_flags(0x0800_0000)
}
#[cfg(not(windows))]
fn hidden(cmd: &mut Command) -> &mut Command {
    cmd
}

/// Where this executable lives (the install).
fn home() -> PathBuf {
    std::env::current_exe().ok().and_then(|e| e.parent().map(Path::to_path_buf)).unwrap_or_else(|| PathBuf::from("."))
}

/// The exporter: in `tools/` beside the installer (the release zip), else the source checkout's (`target/release`
/// up two).
fn exporter() -> Option<PathBuf> {
    let h = home();
    let release = h.join("tools").join("f13recomp-export").join(if cfg!(windows) { "f13recomp-export.exe" } else { "f13recomp-export" });
    [release, h.join("../../tools/f13-export/bin/Release/net10.0/f13-export.exe")].into_iter().find(|p| p.is_file())
}

/// The game: f13recomp.exe (the release zip), else f13-game.exe (a source checkout's target/release).
fn game_exe() -> PathBuf {
    let ext = if cfg!(windows) { ".exe" } else { "" };
    let release = home().join(format!("f13recomp{ext}"));
    if release.is_file() {
        release
    } else {
        home().join(format!("f13-game{ext}"))
    }
}

fn export_dir() -> PathBuf {
    home().join("export")
}

/// What the picked game folder holds.
#[derive(PartialEq)]
enum GameFolder {
    Empty,
    /// Loose files under SummerCamp/Content: usable.
    Unpacked,
    /// The game, but only its paks: it must be unpacked first.
    Packed,
    NotTheGame,
}

fn check_game(dir: &str) -> GameFolder {
    if dir.trim().is_empty() {
        return GameFolder::Empty;
    }
    let content = Path::new(dir.trim()).join("SummerCamp").join("Content");
    if content.join("Maps").is_dir() || content.join("Characters").is_dir() {
        GameFolder::Unpacked
    } else if content.join("Paks").is_dir() {
        GameFolder::Packed
    } else {
        GameFolder::NotTheGame
    }
}

/// The game folder picked, kept in installer.json (one string: no serde for so little).
fn load_game_folder() -> String {
    let text = std::fs::read_to_string(home().join("installer.json")).unwrap_or_default();
    let field = || {
        let at = text.find("\"game\"")?;
        let rest = &text[at + 6..];
        let start = rest.find('"')? + 1;
        let end = start + rest[start..].find('"')?;
        Some(rest[start..end].replace("\\\\", "\\"))
    };
    field().unwrap_or_default()
}

fn save_game_folder(game: &str) {
    let esc = game.replace('\\', "\\\\").replace('"', "");
    let _ = std::fs::write(home().join("installer.json"), format!("{{\n  \"game\": \"{esc}\"\n}}\n"));
}

/// A message from the install thread.
enum Progress {
    Step(usize, String),
    Line(String),
    Done(Result<(), String>),
}

/// Runs `cmd`, its output line by line to the log; an error if it fails.
fn run(tx: &Sender<Progress>, ctx: &egui::Context, mut cmd: Command) -> Result<(), String> {
    cmd.stdout(Stdio::piped()).stderr(Stdio::piped()).stdin(Stdio::null());
    let mut child = hidden(&mut cmd).spawn().map_err(|e| format!("couldn't start {:?}: {e}", cmd.get_program()))?;
    let err = child.stderr.take().map(|e| {
        let tx = tx.clone();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            for line in BufReader::new(e).lines().map_while(Result::ok) {
                let _ = tx.send(Progress::Line(line));
                ctx.request_repaint();
            }
        })
    });
    if let Some(out) = child.stdout.take() {
        for line in BufReader::new(out).lines().map_while(Result::ok) {
            let _ = tx.send(Progress::Line(line));
            ctx.request_repaint();
        }
    }
    if let Some(t) = err {
        let _ = t.join();
    }
    let status = child.wait().map_err(|e| e.to_string())?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("{:?} failed ({status})", cmd.get_program()))
    }
}

fn install(tx: Sender<Progress>, ctx: egui::Context, game: PathBuf) {
    let result = (|| {
        let export = exporter().ok_or("tools/f13recomp-export/f13recomp-export.exe is missing from the install")?;
        for (i, (what, about)) in STEPS.iter().enumerate() {
            let _ = tx.send(Progress::Step(1 + i, about.to_string()));
            let mut cmd = Command::new(&export);
            cmd.arg("--game").arg(&game).arg("--out").arg(export_dir()).arg(what);
            run(&tx, &ctx, cmd)?;
        }
        Ok(())
    })();
    let _ = tx.send(Progress::Done(result));
    ctx.request_repaint();
}

struct Installer {
    game: String,
    rx: Option<Receiver<Progress>>,
    step: Option<(usize, String)>,
    log: Vec<String>,
    outcome: Option<Result<(), String>>,
}

impl Installer {
    fn installed() -> bool {
        export_dir().join("levels").is_dir()
    }

    fn running(&self) -> bool {
        self.rx.is_some()
    }

    fn start(&mut self, ctx: &egui::Context) {
        save_game_folder(self.game.trim());
        let (tx, rx) = channel();
        self.rx = Some(rx);
        self.log.clear();
        self.outcome = None;
        let game = PathBuf::from(self.game.trim());
        let ctx = ctx.clone();
        std::thread::spawn(move || install(tx, ctx, game));
    }

    fn poll(&mut self) {
        let Some(rx) = &self.rx else { return };
        let mut done = false;
        while let Ok(p) = rx.try_recv() {
            match p {
                Progress::Step(n, what) => {
                    self.log.push(format!("== {what}"));
                    self.step = Some((n, what));
                }
                Progress::Line(l) => {
                    self.log.push(l);
                    // Long exports: only the last lines matter on screen.
                    if self.log.len() > 4000 {
                        self.log.drain(..1000);
                    }
                }
                Progress::Done(r) => {
                    self.outcome = Some(r);
                    done = true;
                }
            }
        }
        if done {
            self.rx = None;
            self.step = None;
        }
    }

    fn play(&mut self) {
        let mut cmd = Command::new(game_exe());
        cmd.current_dir(home());
        if let Err(e) = cmd.spawn() {
            self.outcome = Some(Err(format!("couldn't start {}: {e}", game_exe().display())));
        }
    }
}

fn status(ui: &mut egui::Ui, ok: bool, text: &str) {
    let (mark, colour) = if ok { ("✔", egui::Color32::from_rgb(90, 190, 90)) } else { ("✖", egui::Color32::from_rgb(220, 90, 80)) };
    ui.horizontal(|ui| {
        ui.colored_label(colour, mark);
        ui.label(text);
    });
}

impl eframe::App for Installer {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.poll();
        egui::CentralPanel::default().show(ui, |ui| {
            ui.heading("f13recomp setup");
            ui.label("Builds the game from your own copy of Friday the 13th: The Game.");
            ui.add_space(8.0);
            let busy = self.running();

            ui.label("Your Friday the 13th: The Game folder (unpacked: its files under SummerCamp/Content)");
            ui.horizontal(|ui| {
                let browse = ui.add_enabled(!busy, egui::Button::new("Browse…"));
                ui.add_enabled(!busy, egui::TextEdit::singleline(&mut self.game).desired_width(f32::INFINITY));
                if browse.clicked() {
                    if let Some(p) = rfd::FileDialog::new().set_title("The unpacked game folder (the one with SummerCamp in it)").pick_folder() {
                        self.game = p.display().to_string();
                    }
                }
            });
            let game = check_game(&self.game);
            match game {
                GameFolder::Empty => {}
                GameFolder::Unpacked => status(ui, true, "Unpacked game files found"),
                GameFolder::Packed => status(ui, false, "This is the game, but its files are still packed: unpack it first (see the README)"),
                GameFolder::NotTheGame => status(ui, false, "No SummerCamp/Content here: pick the folder that has SummerCamp in it"),
            }
            let tool = exporter().is_some();
            if !tool {
                status(ui, false, "The install is missing its tools folder: unzip the whole release again");
            }
            ui.add_space(10.0);

            let ready = game == GameFolder::Unpacked && tool && !busy;
            ui.horizontal(|ui| {
                let label = if Self::installed() { "Reinstall" } else { "Install" };
                if ui.add_enabled(ready, egui::Button::new(label).min_size(egui::vec2(110.0, 30.0))).clicked() {
                    self.start(ui.ctx());
                }
                let can_play = Self::installed() && !busy && game_exe().is_file();
                if ui.add_enabled(can_play, egui::Button::new("Play").min_size(egui::vec2(110.0, 30.0))).clicked() {
                    self.play();
                }
                if let Some((n, what)) = &self.step {
                    ui.spinner();
                    ui.label(format!("Step {n} of {}: {what}…", STEPS.len()));
                }
            });
            match &self.outcome {
                Some(Ok(())) => status(ui, true, "Installed. Press Play (or start f13recomp.exe)."),
                Some(Err(e)) => status(ui, false, &format!("Install failed: {e} (the log below says why)")),
                None if Self::installed() && !busy => status(ui, true, "Already installed: Play, or Reinstall after a game update."),
                None => {}
            }
            ui.add_space(6.0);
            if !self.log.is_empty() {
                ui.separator();
                egui::ScrollArea::vertical().stick_to_bottom(true).auto_shrink([false, false]).show(ui, |ui| {
                    for l in &self.log {
                        ui.monospace(l);
                    }
                });
            }
        });
        if self.running() {
            ui.ctx().request_repaint_after(std::time::Duration::from_millis(200));
        }
    }
}

fn main() -> eframe::Result {
    // `--install <game folder>`: the same install without the window (its log to stdout; testing and scripted setups).
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().map(String::as_str) == Some("--install") {
        let Some(game) = args.get(1) else {
            eprintln!("usage: f13-installer --install <game folder>");
            std::process::exit(2);
        };
        let (tx, rx) = channel();
        let game = PathBuf::from(game);
        std::thread::spawn(move || install(tx, egui::Context::default(), game));
        for p in rx {
            match p {
                Progress::Step(n, what) => println!("== step {n}: {what}"),
                Progress::Line(l) => println!("{l}"),
                Progress::Done(Ok(())) => {
                    println!("installed: {}", export_dir().display());
                    return Ok(());
                }
                Progress::Done(Err(e)) => {
                    eprintln!("install failed: {e}");
                    std::process::exit(1);
                }
            }
        }
        return Ok(());
    }
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_title("f13recomp setup").with_inner_size([680.0, 420.0]).with_min_inner_size([520.0, 320.0]),
        ..Default::default()
    };
    let app = Installer { game: load_game_folder(), rx: None, step: None, log: Vec::new(), outcome: None };
    eframe::run_native("f13recomp setup", options, Box::new(|_| Ok(Box::new(app))))
}
