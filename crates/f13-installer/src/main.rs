//! f13-installer: sets f13-rs up from the player's own unpacked copy of Friday the 13th: The Game, then plays.
//!
//! The player picks the game's folder (its content extracted to loose files under `SummerCamp/Content`) and
//! UAssetGUI.exe; Install then runs, in order:
//! 1. `tools/extract_tuning.py` (Python): the gameplay values, into `~/.f13-rs/tuning.json`;
//! 2. `tools/f13-export/f13-export.exe` (self-contained .NET) `mvp`, `sequences` and `particles`: the map, characters, sounds, HUD, the intro and outro,
//!    the particle systems, into `export/` next to this executable (where f13-game.exe looks without `--export`).
//!
//! Play starts f13-game.exe. The folders picked are kept in `installer.json` beside the executable.
//!
//! In the release zip the tools sit in `tools/` next to this executable; run from a source checkout
//! (`target/release`), the repository's own (`tools/extract_tuning.py`, the exporter's Release build) are used.
#![windows_subsystem = "windows"]

use eframe::egui;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc::{channel, Receiver, Sender};

/// The exporter's commands, in order (particles reads the levels mvp writes).
const EXPORTS: [(&str, &str); 3] = [("mvp", "the map, characters, sounds and HUD"), ("sequences", "the intro and outro"), ("particles", "the particle effects")];

/// Windows: the tools' console windows stay hidden (CREATE_NO_WINDOW).
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

/// A tool: in `tools/` beside the installer (the release zip), else the source checkout's (`target/release` up two).
fn tool(release: &str, checkout: &str) -> Option<PathBuf> {
    let h = home();
    [h.join("tools").join(release), h.join("../..").join(checkout)].into_iter().find(|p| p.is_file())
}

fn exporter() -> Option<PathBuf> {
    tool(if cfg!(windows) { "f13-export/f13-export.exe" } else { "f13-export/f13-export" }, "tools/f13-export/bin/Release/net10.0/f13-export.exe")
}

fn tuning_script() -> Option<PathBuf> {
    tool("extract_tuning.py", "tools/extract_tuning.py")
}

fn game_exe() -> PathBuf {
    home().join(if cfg!(windows) { "f13-game.exe" } else { "f13-game" })
}

fn export_dir() -> PathBuf {
    home().join("export")
}

/// Python 3 on the PATH: the launcher `py -3` (Windows), else `python3` or `python`.
fn find_python() -> Option<Vec<String>> {
    let candidates: [&[&str]; 3] = [&["py", "-3"], &["python3"], &["python"]];
    candidates.into_iter().find_map(|c| {
        let mut cmd = Command::new(c[0]);
        cmd.args(&c[1..]).arg("--version").stdout(Stdio::piped()).stderr(Stdio::piped());
        let out = hidden(&mut cmd).output().ok()?;
        let v = String::from_utf8_lossy(&out.stdout).to_string() + &String::from_utf8_lossy(&out.stderr);
        (out.status.success() && v.contains("Python 3")).then(|| c.iter().map(|s| s.to_string()).collect())
    })
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

/// The folders picked, kept in installer.json (two strings: no serde for so little).
#[derive(Default)]
struct Settings {
    game: String,
    uassetgui: String,
}

impl Settings {
    fn path() -> PathBuf {
        home().join("installer.json")
    }

    fn load() -> Self {
        let text = std::fs::read_to_string(Self::path()).unwrap_or_default();
        let field = |k: &str| {
            let at = text.find(&format!("\"{k}\""))?;
            let rest = &text[at + k.len() + 2..];
            let start = rest.find('"')? + 1;
            let end = start + rest[start..].find('"')?;
            Some(rest[start..end].replace("\\\\", "\\"))
        };
        Settings { game: field("game").unwrap_or_default(), uassetgui: field("uassetgui").unwrap_or_default() }
    }

    fn save(&self) {
        let esc = |s: &str| s.replace('\\', "\\\\").replace('"', "");
        let _ = std::fs::write(Self::path(), format!("{{\n  \"game\": \"{}\",\n  \"uassetgui\": \"{}\"\n}}\n", esc(&self.game), esc(&self.uassetgui)));
    }
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

fn install(tx: Sender<Progress>, ctx: egui::Context, game: PathBuf, uassetgui: PathBuf, python: Vec<String>) {
    let result = (|| {
        let script = tuning_script().ok_or("tools/extract_tuning.py is missing from the install")?;
        let export = exporter().ok_or("tools/f13-export/f13-export.exe is missing from the install")?;
        let _ = tx.send(Progress::Step(1, "Reading the gameplay values (tuning)".into()));
        let mut cmd = Command::new(&python[0]);
        cmd.args(&python[1..]).arg(&script).arg("--game").arg(&game).arg("--uassetgui").arg(&uassetgui);
        run(&tx, &ctx, cmd)?;
        for (i, (what, about)) in EXPORTS.iter().enumerate() {
            let _ = tx.send(Progress::Step(2 + i, format!("Exporting {about}")));
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
    settings: Settings,
    python: Option<Vec<String>>,
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
        let Some(python) = self.python.clone() else { return };
        self.settings.save();
        let (tx, rx) = channel();
        self.rx = Some(rx);
        self.log.clear();
        self.outcome = None;
        let (game, uassetgui) = (PathBuf::from(self.settings.game.trim()), PathBuf::from(self.settings.uassetgui.trim()));
        let ctx = ctx.clone();
        std::thread::spawn(move || install(tx, ctx, game, uassetgui, python));
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

/// A path field with a Browse button.
fn path_row(ui: &mut egui::Ui, label: &str, value: &mut String, enabled: bool, pick: impl FnOnce() -> Option<PathBuf>) {
    ui.label(label);
    ui.horizontal(|ui| {
        let browse = ui.add_enabled(enabled, egui::Button::new("Browse…"));
        ui.add_enabled(enabled, egui::TextEdit::singleline(value).desired_width(f32::INFINITY));
        if browse.clicked() {
            if let Some(p) = pick() {
                *value = p.display().to_string();
            }
        }
    });
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
            ui.heading("f13-rs setup");
            ui.label("Builds the game from your own copy of Friday the 13th: The Game.");
            ui.add_space(8.0);
            let busy = self.running();

            path_row(ui, "1. Your Friday the 13th: The Game folder (unpacked: its files under SummerCamp/Content)", &mut self.settings.game, !busy, || {
                rfd::FileDialog::new().set_title("The unpacked game folder (the one with SummerCamp in it)").pick_folder()
            });
            let game = check_game(&self.settings.game);
            match game {
                GameFolder::Empty => {}
                GameFolder::Unpacked => status(ui, true, "Unpacked game files found"),
                GameFolder::Packed => status(ui, false, "This is the game, but its files are still packed: unpack it first (see the README)"),
                GameFolder::NotTheGame => status(ui, false, "No SummerCamp/Content here: pick the folder that has SummerCamp in it"),
            }
            ui.add_space(6.0);

            path_row(ui, "2. UAssetGUI.exe (reads the gameplay values)", &mut self.settings.uassetgui, !busy, || {
                rfd::FileDialog::new().set_title("UAssetGUI.exe").add_filter("UAssetGUI", &["exe"]).pick_file()
            });
            let uassetgui = Path::new(self.settings.uassetgui.trim()).is_file();
            if !self.settings.uassetgui.trim().is_empty() {
                status(ui, uassetgui, if uassetgui { "UAssetGUI found" } else { "No file there" });
            }
            ui.hyperlink_to("Get UAssetGUI (its releases page)", "https://github.com/atenfyr/UAssetGUI/releases");
            ui.add_space(6.0);

            ui.label("3. Python 3");
            match &self.python {
                Some(p) => status(ui, true, &format!("Found ({})", p.join(" "))),
                None => {
                    status(ui, false, "Not found: install Python 3 (tick \"Add python.exe to PATH\"), then Check again");
                    ui.horizontal(|ui| {
                        ui.hyperlink_to("python.org downloads", "https://www.python.org/downloads/");
                        if ui.add_enabled(!busy, egui::Button::new("Check again")).clicked() {
                            self.python = find_python();
                        }
                    });
                }
            }
            let tools = exporter().is_some() && tuning_script().is_some();
            if !tools {
                status(ui, false, "The install is missing its tools folder: unzip the whole release again");
            }
            ui.add_space(10.0);

            let ready = game == GameFolder::Unpacked && uassetgui && self.python.is_some() && tools && !busy;
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
                    ui.label(format!("Step {n} of {}: {what}…", 1 + EXPORTS.len()));
                }
            });
            match &self.outcome {
                Some(Ok(())) => status(ui, true, "Installed. Press Play (or start f13-game.exe)."),
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
    // `--install <game folder> <UAssetGUI.exe>`: the same install without the window (its log to stdout; testing and
    // scripted setups).
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().map(String::as_str) == Some("--install") {
        let (Some(game), Some(uassetgui)) = (args.get(1), args.get(2)) else {
            eprintln!("usage: f13-installer --install <game folder> <UAssetGUI.exe>");
            std::process::exit(2);
        };
        let Some(python) = find_python() else {
            eprintln!("Python 3 not found");
            std::process::exit(1);
        };
        let (tx, rx) = channel();
        let (game, uassetgui) = (PathBuf::from(game), PathBuf::from(uassetgui));
        std::thread::spawn(move || install(tx, egui::Context::default(), game, uassetgui, python));
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
        viewport: egui::ViewportBuilder::default().with_title("f13-rs setup").with_inner_size([720.0, 560.0]).with_min_inner_size([560.0, 420.0]),
        ..Default::default()
    };
    let app = Installer { settings: Settings::load(), python: find_python(), rx: None, step: None, log: Vec::new(), outcome: None };
    eframe::run_native("f13-rs setup", options, Box::new(|_| Ok(Box::new(app))))
}
