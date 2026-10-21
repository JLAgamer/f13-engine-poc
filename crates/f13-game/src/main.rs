//! f13-game: currently a viewer for the MVP map (Crystal Lake, small) built from the player's own export.
//!
//!   cargo run -p f13-game --release -- --export <dir made by tools/f13-export> [--layout 1-4 (default: the game's random pick)] [--seed n] [--near-car] [--bot-escape] [--bot-heal] [--bot-dodge] [--bot-door] [--grass]
//!       [--fly] [--capsule] [--demo] [--fear 0-100] [--health 1-100] [--speed x] [--tuning file] [--aerial] [--no-foliage] [--screenshot out.png [--screenshot-after s]]
//!       [--bot-tommy]  (calls Tommy on the CB radio, dies, comes back as him: with --counselors 3)
//!       [--counselor <class>]  (the counselor played: Athlete (Vanessa), Hero, Bookworm, ...)
//!       [--pose <animation>@<seconds>]  (hold the counselor (and Jason) in one exported animation, for screenshots)
//!       [--no-jason] [--jason-delay s] [--bot-jason]  (Jason hunts the counselor unless left out; counselor bots leave him out)
//!       [--bot-abilities]  (Jason, every ability ready and told where she is, Morphs, Stalks and Shifts to her)
//!       [--particles System]  (that particle system 1.5 m ahead every 0.3 s (testing))
//!       [--circle DegPerSec]  (hold W, the camera turning: she runs a circle (testing))
//!       [--aim PitchDeg[:FireS]]  (hold right mouse, the camera at this pitch: aim the gun given, and fire it at FireS (testing))
//!       [--give Class,Class...] [--bot-fight]  (start with these items (testing); a bot that uses them on Jason)
//!       [--map-open]  (her objectives and map screen open from the start (testing))
//!       [--bot-block]  (block Jason's swings with the weapon given)
//!       [--bot-trap]  (walk into the bear trap given, wiggle free; without Jason)
//!       [--nav-debug]  (draw the navmesh around the counselor)
//!       [--bot-hide] [--bot-hide-seen]  (hide out of Jason's sight and come out; hide where he sees: he kills her)
//!       [--bot-locked-in] [--bot-barred-in] [--bot-raged-in]  (behind a locked / barricaded door: he breaks it down,
//!       or raged bursts through it, and kills her)
//!       [--camera x,y,z,tx,ty,tz]  (--fly start: eye and look-at point; playing: both relative to the counselor's feet)
//!       [--round-time s]  (the match's length instead of the game's RoundTime)
//!       [--flashlight]  (start with the flashlight on; F toggles it)
//!   Default: play as Vanessa (see player.rs for controls). --fly: free camera instead.   (screenshot: wait for loading, save one frame, exit)

mod audio;
mod audio_settings;
mod emotes;
mod perks;
mod blood;
mod enlighten;
mod bloom;
mod bot;
mod cabinets;
mod cinematic;
mod counselor_preview;
mod killer_profile;
mod car_lights;
mod character;
mod controls;
mod lobby;
mod net;
mod spectate;
mod net_world;
mod collision;
mod doors;
mod fear_events;
mod interact;
mod hiding;
mod host;
mod items;
mod jason_player;
mod counselor_camera;
mod screen_effects;
mod sense_view;
mod circle_dof;
mod decals;
mod dof;
mod face;
mod hair;
mod audio_volumes;
mod ssao;
mod killer_fade;
mod tonemap;
mod interaction_camera;
mod kill_camera;
mod jason;
mod lighting;
mod light_shafts;
mod match_flow;
mod materials;
mod navigation;
mod perf;
mod mipmaps;
mod player;
mod particles;
mod placed_kills;
mod police;
mod power;
mod radio;
mod tommy;
mod sweater;
mod jason_death;
mod voice;
mod wardrobe;
mod prop_anim;
mod props;
mod ragdoll;
mod senses;
mod shadow_cull;
mod smoothing;
mod spline_meshes;
mod ui;
mod windows;
mod vehicle;
mod water;
mod weather;

use bevy::asset::AssetPlugin;
use bevy::camera_controller::free_camera::{FreeCamera, FreeCameraPlugin};
use bevy::prelude::*;
use bevy::render::view::screenshot::{save_to_disk, Screenshot};
use bevy::world_serialization::WorldAssetRoot;
use f13_data::level::{LevelLayout, Xform};
use std::collections::HashMap;
use std::path::PathBuf;

const MAP: &str = "CrystalLake_Small";
const LAYOUTS: u32 = 4;

struct Args {
    /// The netcode role (--server <port>, --connect <host:port>).
    net: Option<net::Role>,
    /// --bot-net: as a client, the network test bot (net.rs); as a lobby's playing server, its lobby part only.
    bot_net: bool,
    bot_net_abilities: bool,
    /// --bot-spectate: the spectating test (spectate.rs; with --counselors 2).
    bot_spectate: bool,
    /// --lobby (a server): the players gather and ready up first (lobby.rs); --dedicated: the server plays no one.
    lobby: bool,
    dedicated: bool,
    /// --name: this player's name in the lobby.
    name: Option<String>,
    /// --prefer counselor|none|jason: what the net bot asks the lobby for.
    prefer: lobby::Preference,
    /// Counselors in the match (--counselors n): the local one and n - 1 others at the next starts (testing).
    counselors: usize,
    export: PathBuf,
    /// None: picked as the game picks it.
    layout: Option<u32>,
    seed: Option<u64>,
    near_car: bool,
    bot_escape: bool,
    bot_escape4: bool,
    /// --bot-passenger (with --counselors 2): the escape bot drives with the other counselor as its passenger.
    bot_passenger: bool,
    bot_heal: bool,
    bot_drop: bool,
    bot_dodge: bool,
    bot_door: bool,
    bot_jason: bool,
    /// --bot-abilities: Jason Morphs, Stalks and Shifts to her.
    bot_abilities: bool,
    /// --bot-block: block Jason's swings.
    bot_block: bool,
    /// --bot-trap: walk into her own bear trap and wiggle free.
    bot_trap: bool,
    bot_fight: bool,
    bot_hide: bool,
    nav_debug: bool,
    bot_hide_seen: bool,
    /// --bot-locked-in (false) or --bot-barred-in (true).
    bot_locked_in: Option<bool>,
    /// --bot-raged-in: locked in, with Jason raged.
    bot_raged: bool,
    /// Log frame time, entity counts and GPU pass times every few seconds.
    perf: bool,
    /// Play Jason (jason_player.rs); a bot plays the counselor.
    play_jason: bool,
    /// Every spawner spawns, as with the world settings' bOverrideSpawning (testing: the radios).
    override_spawning: bool,
    no_voice: bool,
    /// A client's netcode id (kept across a rematch's relaunch: net.rs `relaunch`).
    net_id: Option<u64>,
    /// A lobby's last match's Jason (a rematch's relaunch passes it on).
    last_jason: Option<u64>,
    /// A server: its players' scenario bots set their scenes up (testing).
    bot_hooks: bool,
    /// A lobby's server: the last match's results (a rematch's relaunch passes them on).
    last_match: Option<PathBuf>,
    /// Voice without push-to-talk (the game's default is push-to-talk: T).
    open_mic: bool,
    bot_radio: bool,
    /// Jason's grab kill, by name (its take's end: jason::kill_by_name, e.g. curbstomp); else one at random.
    kill: Option<String>,
    /// Every fuse box starts destroyed (power.rs; implied by --bot-power).
    cut_power: bool,
    bot_power: bool,
    bot_police: bool,
    bot_placed_kill: bool,
    bot_drown: bool,
    bot_car_grab: bool,
    /// Scripted Jason input (testing --play-jason).
    play_jason_demo: bool,
    /// Take the host's commands on this localhost port (host.rs, `f13-host`).
    host: Option<u16>,
    give: Vec<String>,
    aim: Option<(f32, Option<f32>)>,
    circle: Option<f32>,
    particles: Option<String>,
    /// Her map screen open from the start (testing).
    map_open: bool,
    /// Leave Jason out (the counselor-only bots always do).
    no_jason: bool,
    no_intro: bool,
    /// Seconds before Jason starts hunting.
    jason_delay: f32,
    /// --jason-health: his health at the start (testing his mask).
    jason_health: Option<f32>,
    unlit: bool,
    grass: bool,
    aerial: bool,
    fly: bool,
    capsule: bool,
    demo: bool,
    /// --fear: fear held at this level (no fear events).
    fear: Option<f32>,
    /// Game clock multiplier (testing: run bots faster than real time).
    speed: f32,
    health: Option<f32>,
    /// Start the counselor here (Bevy metres) instead of at a counselor start (testing).
    spawn: Option<Vec3>,
    tuning: Option<PathBuf>,
    foliage: bool,
    screenshot: Option<PathBuf>,
    screenshot_after: f32,
    pose: Option<(String, f32)>,
    /// The counselor's outfit (`--outfit <class or name>[:option,...]`).
    outfit: Option<String>,
    /// The local counselor's class (`--counselor Athlete|Hero|...`; the lobby's pick, C cycles it there).
    counselor_class: String,
    /// --bot-tommy: she calls Tommy on the CB radio, dies with another counselor and comes back as him (run with
    /// --counselors 3).
    bot_tommy: bool,
    /// --bot-sweater: she picks Pamela's sweater up and uses it on Jason.
    bot_sweater: bool,
    /// --bot-kill-jason (--give CounselorMachete_C --jason-health 5): the sweater, his mask, his kill stance, his death.
    bot_kill_jason: bool,
    bot_mask: bool,
    /// --fly start: eye position and a point to look at (Bevy space, metres).
    camera: Option<(Vec3, Vec3)>,
    /// --round-time: the match's length (s) instead of the game's.
    round_time: Option<i32>,
    /// `--rain on|off`: the weather, as the game's `rain=` URL option (else rolled).
    rain: Option<bool>,
    /// --flashlight: start with the flashlight on.
    flashlight: bool,
}

fn parse_args() -> Args {
    let mut export = None;
    let mut layout = None;
    let mut seed = None;
    let mut near_car = false;
    let mut bot_escape = false;
    let mut bot_escape4 = false;
    let mut bot_passenger = false;
    let mut bot_heal = false;
    let mut bot_drop = false;
    let mut bot_net = false;
    let mut bot_net_abilities = false;
    let mut bot_spectate = false;
    let (mut lobby, mut dedicated, mut name, mut prefer) = (false, false, None, lobby::Preference::NoPreference);
    let mut bot_dodge = false;
    let mut bot_door = false;
    let mut bot_jason = false;
    let mut bot_abilities = false;
    let mut bot_block = false;
    let mut bot_trap = false;
    let mut bot_fight = false;
    let mut bot_hide = false;
    let mut nav_debug = false;
    let mut bot_hide_seen = false;
    let mut bot_locked_in = None;
    let mut bot_raged = false;
    let mut perf = false;
    let mut play_jason = false;
    let mut override_spawning = false;
    let mut no_voice = false;
    let (mut net_id, mut last_jason, mut bot_hooks, mut last_match) = (None, None, false, None);
    let mut open_mic = false;
    let mut bot_radio = false;
    let mut kill = None;
    let mut cut_power = false;
    let mut bot_power = false;
    let mut bot_police = false;
    let mut bot_placed_kill = false;
    let mut bot_drown = false;
    let mut bot_car_grab = false;
    let mut play_jason_demo = false;
    let mut host = None;
    let mut give = Vec::new();
    let mut aim = None;
    let mut circle = None;
    let mut particles = None;
    let mut map_open = false;
    let mut no_jason = false;
    let mut no_intro = false;
    let mut jason_delay = 0.0;
    let mut jason_health = None;
    let mut unlit = false;
    let mut round_time = None;
    let mut rain = None;
    let mut flashlight = false;
    let mut grass = false;
    let mut aerial = false;
    let mut fly = false;
    let mut capsule = false;
    let mut demo = false;
    let mut fear = None;
    let mut speed = 1.0f32;
    let mut health = None;
    let mut spawn = None;
    let mut counselors = 1usize;
    let mut net = None;
    let mut tuning = None;
    let mut foliage = true;
    let mut screenshot = None;
    let mut screenshot_after = 0.0;
    let mut pose = None;
    let mut outfit = None;
    let mut counselor_class = character::DEFAULT_CLASS.to_string();
    let mut bot_tommy = false;
    let mut bot_sweater = false;
    let mut bot_kill_jason = false;
    let mut bot_mask = false;
    let mut camera = None;
    let mut it = std::env::args().skip(1);
    while let Some(a) = it.next() {
        match a.as_str() {
            "--export" => export = it.next().map(PathBuf::from),
            "--layout" => layout = it.next().and_then(|v| v.parse::<u32>().ok()).map(|n| n.clamp(1, LAYOUTS)),
            "--seed" => seed = it.next().and_then(|v| v.parse().ok()),
            "--near-car" => near_car = true,
            "--bot-escape" => bot_escape = true,
            "--bot-escape4" => (bot_escape, bot_escape4) = (true, true),
            "--bot-passenger" => (bot_escape, bot_passenger) = (true, true),
            "--bot-heal" => bot_heal = true,
            "--bot-drop" => bot_drop = true,
            "--bot-net" => bot_net = true,
            "--bot-net-abilities" => {
                bot_net = true;
                bot_net_abilities = true;
            }
            "--bot-spectate" => bot_spectate = true,
            "--lobby" => lobby = true,
            "--dedicated" => dedicated = true,
            "--name" => name = it.next(),
            "--prefer" => {
                prefer = match it.next().as_deref() {
                    Some("counselor") => lobby::Preference::Counselor,
                    Some("jason") => lobby::Preference::Killer,
                    _ => lobby::Preference::NoPreference,
                }
            }
            "--bot-dodge" => bot_dodge = true,
            "--bot-door" => bot_door = true,
            "--bot-jason" => bot_jason = true,
            "--bot-abilities" => bot_abilities = true,
            "--bot-block" => bot_block = true,
            "--bot-trap" => bot_trap = true,
            "--bot-fight" => bot_fight = true,
            "--bot-hide" => bot_hide = true,
            "--nav-debug" => nav_debug = true,
            "--bot-hide-seen" => bot_hide_seen = true,
            "--bot-locked-in" => bot_locked_in = Some(false),
            "--bot-barred-in" => bot_locked_in = Some(true),
            "--bot-raged-in" => {
                bot_locked_in = Some(false);
                bot_raged = true;
            }
            "--perf" => perf = true,
            "--play-jason" => play_jason = true,
            "--jason" => {
                if let Some(name) = it.next() {
                    character::set_killer_class(&name);
                }
            }
            "--override-spawning" => override_spawning = true,
            "--no-voice" => no_voice = true,
            "--net-id" => net_id = it.next().and_then(|v| v.parse().ok()),
            "--last-jason" => last_jason = it.next().and_then(|v| v.parse().ok()),
            "--bot-hooks" => bot_hooks = true,
            "--last-match" => last_match = it.next().map(PathBuf::from),
            "--open-mic" => open_mic = true,
            "--bot-radio" => bot_radio = true,
            "--bot-tommy" => bot_tommy = true,
            "--bot-sweater" => bot_sweater = true,
            "--bot-kill-jason" => bot_kill_jason = true,
            "--bot-mask" => bot_mask = true,
            "--kill" => {
                kill = it.next();
            }
            "--cut-power" => cut_power = true,
            "--bot-power" => (bot_power, cut_power) = (true, true),
            "--bot-police" => bot_police = true,
            "--bot-placed-kill" => bot_placed_kill = true,
            "--bot-drown" => bot_drown = true,
            "--bot-car-grab" => bot_car_grab = true,
            "--play-jason-demo" => (play_jason, play_jason_demo) = (true, true),
            "--host" => host = Some(host.unwrap_or(f13_host_port())),
            "--host-port" => host = it.next().and_then(|v| v.parse().ok()).or(host),
            "--particles" => particles = it.next(),
            "--circle" => circle = it.next().and_then(|v| v.parse().ok()),
            "--aim" => {
                aim = it.next().and_then(|v| {
                    let mut p = v.split(':');
                    Some((p.next()?.parse().ok()?, p.next().and_then(|t| t.parse().ok())))
                })
            }
            "--give" => give = it.next().map(|v| v.split(',').map(String::from).collect()).unwrap_or_default(),
            "--map-open" => map_open = true,
            "--no-jason" => no_jason = true,
            "--no-intro" => no_intro = true,
            "--jason-health" => jason_health = it.next().and_then(|v| v.parse().ok()),
            "--jason-delay" => jason_delay = it.next().and_then(|v| v.parse().ok()).unwrap_or(0.0f32).max(0.0),
            "--grass" => grass = true,
            "--aerial" => aerial = true,
            "--fly" => fly = true,
            "--capsule" => capsule = true,
            "--demo" => demo = true,
            "--speed" => speed = it.next().and_then(|v| v.parse().ok()).unwrap_or(1.0f32).clamp(0.1, 16.0),
            "--fear" => fear = Some(it.next().and_then(|v| v.parse().ok()).unwrap_or(0.0f32).clamp(0.0, 100.0)),
            "--health" => health = it.next().and_then(|v| v.parse().ok()),
            "--server" => net = it.next().and_then(|v| v.parse().ok()).map(net::Role::Server),
            "--connect" => net = it.next().and_then(|v| std::net::ToSocketAddrs::to_socket_addrs(&v).ok()).and_then(|mut a| a.next()).map(net::Role::Client),
            "--counselors" => counselors = it.next().and_then(|v| v.parse().ok()).unwrap_or(1usize).clamp(1, 8),
            "--spawn" => {
                let v: Vec<f32> = it.next().unwrap_or_default().split(',').filter_map(|x| x.trim().parse().ok()).collect();
                if v.len() == 3 {
                    spawn = Some(Vec3::new(v[0], v[1], v[2]));
                }
            }
            "--tuning" => tuning = it.next().map(PathBuf::from),
            "--flashlight" => flashlight = true,
            "--round-time" => round_time = it.next().and_then(|v| v.parse().ok()).map(|t: i32| t.max(1)),
            "--rain" => rain = it.next().map(|v| v == "on" || v == "true"),
            "--no-foliage" => foliage = false,
            "--unlit" => unlit = true,
            "--screenshot" => screenshot = it.next().map(PathBuf::from),
            "--outfit" => outfit = it.next(),
            "--counselor" => counselor_class = it.next().unwrap_or_else(|| character::DEFAULT_CLASS.to_string()),
            "--pose" => {
                let v = it.next().unwrap_or_default();
                let (name, at) = v.split_once('@').unwrap_or((v.as_str(), "0"));
                pose = Some((name.to_string(), at.parse().unwrap_or(0.0)));
            }
            "--screenshot-after" => screenshot_after = it.next().and_then(|s| s.parse().ok()).unwrap_or(0.0),
            "--camera" => {
                let v: Vec<f32> = it.next().unwrap_or_default().split(',').filter_map(|x| x.trim().parse().ok()).collect();
                if v.len() == 6 {
                    camera = Some((Vec3::new(v[0], v[1], v[2]), Vec3::new(v[3], v[4], v[5])));
                }
            }
            other => {
                eprintln!("unknown argument {other}");
                std::process::exit(2);
            }
        }
    }
    // Without --export: the installer's, the `export` folder next to the executable.
    let beside = std::env::current_exe().ok().and_then(|e| Some(e.parent()?.join("export"))).filter(|d| d.join("levels").is_dir());
    let Some(export) = export.or(beside) else {
        eprintln!("usage: f13-game --export <dir from tools/f13-export> [--layout 1-{LAYOUTS}] [--grass] (or run the installer, f13-engine-poc-setup)");
        std::process::exit(2);
    };
    Args { net, bot_net, bot_net_abilities, bot_spectate, lobby, dedicated, name, prefer, counselors, export, layout, seed, near_car, bot_escape, bot_escape4, bot_passenger, bot_heal, bot_drop, bot_dodge, bot_door, bot_jason, bot_abilities, bot_block, bot_trap, bot_fight, bot_hide, bot_hide_seen, bot_locked_in, bot_raged, perf, nav_debug, give, aim, circle, particles, map_open, no_jason, no_intro, jason_delay, jason_health, unlit, grass, aerial, fly, capsule, demo, fear, speed, health, spawn, host, play_jason, override_spawning, no_voice, net_id, last_jason, bot_hooks, last_match, open_mic, bot_radio, kill, cut_power, bot_power, bot_police, bot_placed_kill, bot_drown, bot_car_grab, play_jason_demo, tuning, foliage, screenshot, screenshot_after, pose, outfit, counselor_class, bot_tommy, bot_sweater, bot_kill_jason, bot_mask, camera, round_time, flashlight, rain }
}

/// Whether Enlighten's indirect light runs (on unless F13_ENLIGHTEN=0).
pub(crate) fn enlighten_enabled() -> bool {
    std::env::var("F13_ENLIGHTEN").map_or(true, |v| v != "0")
}

#[derive(Resource)]
pub(crate) struct Level {
    pub(crate) layouts: Vec<LevelLayout>,
    grass: bool,
    foliage: bool,
    aerial: bool,
    fly: bool,
    camera: Option<(Vec3, Vec3)>,
    scenes: Vec<UntypedHandle>,
}

/// The export directory (asset root).
#[derive(Resource)]
pub(crate) struct ExportRoot(pub(crate) PathBuf);

#[derive(Resource)]
struct ScreenshotPlan {
    path: PathBuf,
    frames_loaded: u32,
    /// Extra game seconds to wait once everything has loaded (--screenshot-after).
    after: f32,
    shot_at: Option<f32>,
}

/// Montage root motion from the export's anims.json: name -> displacement per second in the character's
/// local frame (x = right, y = forward), cm/s.
#[derive(Resource, Default)]
pub(crate) struct Anims(pub HashMap<String, Vec2>);

impl Anims {
    fn load(export: &std::path::Path) -> Self {
        #[derive(serde::Deserialize)]
        #[serde(rename_all = "PascalCase")]
        struct Entry {
            length: f32,
            root_motion: [f32; 3],
            #[serde(default)]
            segments: Vec<Segment>,
        }
        #[derive(serde::Deserialize)]
        #[serde(rename_all = "PascalCase")]
        struct Segment {
            sequence: String,
            length: f32,
            root_motion: [f32; 3],
        }
        let Some(entries) = std::fs::read_to_string(export.join("anims.json")).ok().and_then(|t| serde_json::from_str::<HashMap<String, Entry>>(&t).ok()) else {
            warn!("no anims.json in the export (run tools/f13-export again): dodges and stumbles won't move you");
            return Self::default();
        };
        // Counselor meshes face +Y in their own space: forward = +Y, right = -X. Each segment's take as
        // "<montage>/<take>" (its first slot's), for montages whose sections move differently (Jason's grab).
        let v = |r: [f32; 3], len: f32| Vec2::new(-r[0], r[1]) / len;
        let mut all = HashMap::new();
        for (k, e) in entries.into_iter().filter(|(_, e)| e.length > 0.0) {
            for s in e.segments.iter().filter(|s| s.length > 0.0) {
                all.entry(format!("{k}/{}", s.sequence)).or_insert(v(s.root_motion, s.length));
            }
            all.insert(k, v(e.root_motion, e.length));
        }
        Self(all)
    }
}

/// Random numbers for gameplay rolls.
#[derive(Resource)]
pub(crate) struct GameRng(pub f13_core::spawn::Rng);

fn main() {
    let args = parse_args();
    // The grab kills (grab_kills.json), before anything asks for them.
    jason::load_kills(&args.export);
    let load = |name: &str| {
        let p = args.export.join("levels").join(format!("{name}.json"));
        LevelLayout::load(&p).unwrap_or_else(|e| {
            eprintln!("{}: {e} (run tools/f13-export first)", p.display());
            std::process::exit(1);
        })
    };
    // A client builds the server's world (its seed, layout and weather).
    let joined = match args.net {
        Some(net::Role::Client(addr)) => net::query_world(addr),
        _ => None,
    };
    if let Some(w) = &joined {
        println!("the server's world: seed {}, {}, {}", w.seed, w.layout, if w.rain { "rain" } else { "clear" });
    }
    let seed = joined.as_ref().map(|w| w.seed).or(args.seed).unwrap_or_else(|| {
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs())
    });
    let main = load(MAP);
    let layout = match (&joined, args.layout) {
        (Some(w), _) => w.layout.clone(),
        (None, Some(n)) => format!("CLsm_Design_SL{n}"),
        (None, None) => random_level(&main, seed).unwrap_or_else(|| "CLsm_Design_SL1".into()),
    };
    println!("cabin layout {layout}");
    // The weather (SCWorldSettings: rolled after the layout).
    let rain = joined.as_ref().map(|w| w.rain).or(args.rain).unwrap_or_else(|| rain_roll(&main, seed));
    if let Some(net::Role::Server(port)) = args.net {
        net::serve_world(port, net::WorldInfo { seed, layout: layout.clone(), rain });
    }
    let weather = if rain { "CLsm_RainOn" } else { "CLsm_RainOff" };
    println!("weather: {}", if rain { "rain" } else { "clear" });
    let mut layouts = vec![main, load(&layout)];
    match LevelLayout::load(&args.export.join("levels").join(format!("{weather}.json"))) {
        Ok(w) => layouts.push(w),
        Err(e) => eprintln!("{weather}: {e} (re-export: f13-export levels): no weather level"),
    }
    let spawn = args.spawn.unwrap_or_else(|| counselor_start(&layouts));
    let mut app = App::new();
    // Gameplay randomness (stumbles, ...), separate from the match layout so both repeat with --seed.
    app.insert_resource(Anims::load(&args.export));
    app.insert_resource(character::GameSpeed(args.speed));
    if args.speed != 1.0 {
        // Virtual time drives Update and FixedUpdate, so the whole simulation (and the bots) runs faster.
        let speed = args.speed;
        app.add_systems(Startup, move |mut time: ResMut<Time<Virtual>>| {
            time.set_relative_speed(speed);
            time.set_max_delta(std::time::Duration::from_secs_f32(0.25 * speed));
            info!("game speed x{speed}");
        });
    }
    app.insert_resource(GameRng(f13_core::spawn::Rng::new(seed ^ 0x5eed_f13)));
    app.insert_resource(weather::Weather { rain }).init_resource::<weather::RainMpc>().add_systems(Update, weather::rain_mpc);
    if !(args.fly || args.aerial) {
        let path = args.tuning.clone().unwrap_or_else(f13_data::default_path);
        let tuning = f13_data::load(&path).unwrap_or_else(|e| {
            eprintln!("{}: {e}", path.display());
            std::process::exit(1);
        });
        let stats = *tuning.characters.get("Vanessa").expect("Vanessa missing from tuning.json");
        app.insert_resource(fear_events::FearTuning {
            update_delay: tuning.update_fear_delay,
            fear_modifier: tuning.fear_modifier.clone(),
            events: tuning.fear_events.clone(),
            jason_near_range: tuning.jason_near_range,
        });
        let mut rules = tuning.match_rules.clone();
        if let Some(t) = args.round_time {
            rules.round_time = t;
        }
        app.insert_resource(match_flow::MatchTuning { rules, events: tuning.score_events.clone() });
        // The level intro (cinematic.rs): not for the bots and demos (tests), the camera tools, or --no-intro.
        let scripted = std::env::args().any(|a| a.starts_with("--bot") || a == "--demo" || a == "--play-jason-demo");
        if !args.no_intro && !scripted && !args.fly && args.camera.is_none() && !args.unlit && cinematic::available(&args.export, &cinematic::INTRO) {
            app.insert_resource(cinematic::IntroEnabled);
        }
        let bot = args.bot_car_grab || args.bot_power || args.bot_police || args.bot_placed_kill || args.bot_drown || args.bot_escape || args.bot_heal || args.bot_drop || args.bot_dodge || args.bot_door || args.bot_jason || args.bot_abilities || args.bot_block || args.bot_trap || args.bot_fight || args.bot_hide || args.bot_hide_seen || args.bot_locked_in.is_some();
        app.insert_resource(player::PlayerTuning {
            counselor: tuning.counselor,
            movement: tuning.counselor_movement,
            stats,
            characters: tuning.characters.clone(),
            first_aid: tuning.first_aid,
            minigame: tuning.minigame.clone(),
            repair_hold_time: tuning.repair_hold_time,
            vehicle_start: tuning.vehicle_start.clone(),
        })
            .insert_resource(senses::KillerAbilityTuning(tuning.jason.clone()))
            .add_plugins(controls::ControlsPlugin)
            .insert_resource(net::FreeStarts(counselor_starts(&layouts).into_iter().skip(args.counselors).collect()))
            .add_plugins(player::PlayerPlugin { others: counselor_starts(&layouts).into_iter().skip(1).take(args.counselors - 1).collect(), spawn, capsule: args.capsule, camera: args.camera, demo: args.demo, fear: args.fear.unwrap_or(0.0), health: args.health, class: args.counselor_class.clone() })
            .insert_resource(character::LocalClass(args.counselor_class.clone()))
            .add_plugins(fear_events::FearEventsPlugin { locked: args.fear })
            // Playing Jason, the bot playing her doesn't decide when the run ends.
            .add_plugins(match_flow::MatchPlugin { leave: !bot || args.play_jason })
            .add_plugins((interact::InteractPlugin, vehicle::VehiclePlugin, character::CharacterPlugin { export: args.export.clone(), pose: args.pose.clone(), outfit: args.outfit.clone(), defer_killer: args.lobby || matches!(args.net, Some(net::Role::Client(_))) }, collision::CollisionPlugin, audio::AudioPlugin, water::WaterPlugin, power::PowerPlugin { cut: args.cut_power }, kill_camera::KillCameraPlugin, prop_anim::PropAnimPlugin, doors::DebrisPlugin, ragdoll::RagdollPlugin, items::ItemsPlugin, hiding::HidingPlugin, senses::SensesPlugin, navigation::NavigationPlugin { debug: args.nav_debug }))
            .add_plugins(radio::RadioPlugin)
            .add_plugins((spectate::SpectatePlugin { bot: args.bot_spectate }, police::PolicePlugin, tommy::TommyPlugin, sweater::SweaterPlugin, jason_death::JasonDeathPlugin, placed_kills::PlacedKillsPlugin, interaction_camera::InteractionCameraPlugin, counselor_camera::CounselorCameraPlugin, cabinets::CabinetsPlugin, weather::WeatherPlugin, car_lights::CarLightsPlugin, cinematic::CinematicPlugin, hair::HairPlugin, audio_volumes::AudioVolumesPlugin));
        if args.bot_mask {
            app.init_resource::<jason_death::BotMask>().add_systems(Update, jason_death::bot_mask);
        }
        if args.bot_kill_jason {
            app.init_resource::<jason_death::BotKillJason>().add_systems(Update, jason_death::bot_kill_jason.before(jason_death::start_kill));
        }
        if args.bot_sweater {
            app.init_resource::<sweater::BotSweater>().add_systems(Update, sweater::bot_sweater);
        }
        if args.bot_tommy {
            app.init_resource::<tommy::BotTommy>().add_systems(Update, tommy::bot_tommy);
        }
        // Playing Jason without a --bot-*, she stands idle (time to try him out).
        if args.bot_escape {
            app.add_plugins(bot::BotPlugin(bot::Goal::Escape));
            if args.bot_escape4 {
                app.insert_resource(bot::FourSeat);
            }
            if args.bot_passenger {
                app.insert_resource(bot::Passenger).add_systems(Update, bot::seat_passenger);
            }
        } else if args.bot_car_grab {
            app.add_plugins(bot::BotPlugin(bot::Goal::CarGrab));
        } else if args.bot_power {
            app.add_plugins(bot::BotPlugin(bot::Goal::Power));
        } else if args.bot_police {
            app.add_plugins(bot::BotPlugin(bot::Goal::Police));
        } else if args.bot_placed_kill {
            app.add_plugins(bot::BotPlugin(bot::Goal::PlacedKill));
        } else if args.bot_drown {
            app.add_plugins(bot::BotPlugin(bot::Goal::Drown));
        } else if args.bot_drop {
            app.add_plugins(bot::BotPlugin(bot::Goal::Drop));
        } else if args.bot_heal {
            app.add_plugins(bot::BotPlugin(bot::Goal::Heal));
        } else if args.bot_dodge {
            app.add_plugins(bot::BotPlugin(bot::Goal::Dodge));
        } else if args.bot_door {
            app.add_plugins(bot::BotPlugin(bot::Goal::Door));
        } else if args.bot_jason {
            app.add_plugins(bot::BotPlugin(bot::Goal::Jason));
        } else if args.bot_abilities {
            app.add_plugins(bot::BotPlugin(bot::Goal::Abilities));
        } else if args.bot_block {
            app.add_plugins(bot::BotPlugin(bot::Goal::Block));
        } else if args.bot_trap {
            app.add_plugins(bot::BotPlugin(bot::Goal::Trap));
        } else if args.bot_fight {
            app.add_plugins(bot::BotPlugin(bot::Goal::Fight));
        } else if args.bot_hide {
            app.add_plugins(bot::BotPlugin(bot::Goal::Hide));
        } else if args.bot_hide_seen {
            app.add_plugins(bot::BotPlugin(bot::Goal::HideSeen));
        } else if let Some(barred) = args.bot_locked_in {
            app.add_plugins(bot::BotPlugin(bot::Goal::LockedIn { barred, raged: args.bot_raged }));
        }
        if args.map_open {
            app.insert_resource(items::MapScreen(true));
        }
        if args.bot_radio {
            app.init_resource::<radio::RadioBot>();
        }
        if args.no_voice {
            app.insert_resource(voice::NoDevices);
        }
        if args.open_mic || std::env::var("F13_VOICE_TONE").is_ok() {
            app.insert_resource(audio_settings::AudioSettings { use_push_to_talk: false, ..audio_settings::AudioSettings::load() });
        }
        wardrobe::plugin(&mut app);
        app.add_plugins((emotes::EmotesPlugin, perks::PerksPlugin, killer_profile::KillerProfilePlugin));
        if args.override_spawning {
            app.insert_resource(props::OverrideSpawning);
        }
        if let Some(name) = args.particles.clone() {
            app.insert_resource(particles::TestParticles(name));
        }
        if let Some(rate) = args.circle {
            app.insert_resource(items::CircleRun(rate));
        }
        if let Some((pitch, fire)) = args.aim {
            app.insert_resource(items::AimHold(pitch, fire));
        }
        if !args.give.is_empty() {
            app.insert_resource(items::Give(args.give.clone()));
        }
        // The counselor bots test her alone; --bot-jason tests him.
        let bot_alone = args.bot_escape || args.bot_heal || args.bot_drop || args.bot_dodge || args.bot_door || args.bot_trap;
        // A client gets Jason from the server.
        if bot_alone {
            // On a client, the server's Jason goes too (a game of its own ignores it).
            bot::setup(bot::BotSetup::NoJason);
        }
        if args.cut_power {
            bot::setup(bot::BotSetup::CutPower);
        }
        if !args.no_jason && !bot_alone {
            if let Some(name) = args.kill.as_deref().filter(|n| jason::kill_by_name(n).is_none()) {
                eprintln!("--kill {name}: none of {:?}", jason::kills().iter().map(|k| k.his.as_str()).collect::<Vec<_>>());
            }
            app.add_plugins(jason::JasonPlugin { delay: args.jason_delay, kill: args.kill.clone(), health: args.jason_health });
        }
    }
    if let Some(path) = args.screenshot.clone() {
        app.insert_resource(ScreenshotPlan { path, frames_loaded: 0, after: args.screenshot_after, shot_at: None }).add_systems(Update, take_screenshot);
    }
    app.add_plugins(
            DefaultPlugins
                .set(AssetPlugin { file_path: args.export.to_string_lossy().into_owned(), ..default() })
                // --perf measures the frame's real cost: no waiting for the display.
                .set(WindowPlugin {
                    primary_window: Some(bevy::window::Window {
                        title: "f13-engine-poc".into(),
                        present_mode: if args.perf { bevy::window::PresentMode::AutoNoVsync } else { bevy::window::PresentMode::AutoVsync },
                        ..default()
                    }),
                    ..default()
                })
                // A dedicated server plays no one and hears nothing (its sounds still run for the clients' shouts).
                .set(bevy::audio::AudioPlugin {
                    global_volume: bevy::audio::GlobalVolume::new(bevy::audio::Volume::Linear(if args.dedicated { 0.0 } else { 1.0 })),
                    ..default()
                })
                // Unreal meshes carry up to 8 UV sets; Bevy warns once per extra set per mesh.
                .set(bevy::log::LogPlugin {
                    filter: "wgpu=error,naga=warn,bevy_gltf::loader=error".into(),
                    #[cfg(feature = "profile")]
                    custom_layer: perf::systems::layer,
                    ..default()
                }),
        )
        .add_plugins((FreeCameraPlugin, bevy::diagnostic::FrameTimeDiagnosticsPlugin::default()))
        .insert_resource(Level { layouts, grass: args.grass, foliage: args.foliage, aerial: args.aerial, fly: args.fly || args.aerial, camera: args.camera, scenes: vec![] })
        .add_plugins((materials::MaterialsPlugin { export: args.export.clone() }, mipmaps::MipmapsPlugin, tonemap::TonemapPlugin, bloom::BloomPlugin, screen_effects::ScreenEffectsPlugin, sense_view::SenseViewPlugin, dof::DofPlugin, circle_dof::CircleDofPlugin, face::FacePlugin, (ssao::SsaoPlugin, light_shafts::LightShaftsPlugin), killer_fade::KillerFadePlugin, spline_meshes::SplineMeshPlugin, particles::ParticlesPlugin, decals::DecalPlugin, blood::BloodPlugin))
        .insert_resource(ExportRoot(args.export.clone()))
        .add_plugins(props::PropsPlugin { seed, near_car: args.near_car && !(args.fly || args.aerial) })
        .add_plugins(lighting::LightingPlugin { flashlight: args.flashlight, unlit: args.unlit })
        .add_plugins(shadow_cull::ShadowCullPlugin)
        .add_plugins(smoothing::SmoothingPlugin)
        .add_systems(Startup, spawn_level)
        .add_systems(PostUpdate, apply_placed.before(bevy::camera::visibility::VisibilitySystems::CheckVisibility));
    // The HUD, hers or Jason's when he's played (embedded shader and UI material: after the asset and render plugins).
    if !(args.fly || args.aerial) {
        app.add_plugins(ui::UiPlugin);
    }
    // Playing Jason (its input comes off the raw events, before the input plugin's systems): from the start, or once
    // the host makes the player Jason.
    if !(args.fly || args.aerial) {
        app.add_plugins(jason_player::PlayJasonPlugin { active: args.play_jason, demo: args.play_jason_demo });
    }
    if let Some(role) = args.net {
        app.add_plugins(net::NetPlugin {
            role,
            bot: args.bot_net,
            bot_abilities: args.bot_net_abilities,
            prefer: args.prefer,
            lobby: args.lobby,
            dedicated: args.dedicated,
            name: args.name.clone().unwrap_or_else(|| if matches!(role, net::Role::Server(_)) { "host".into() } else { String::new() }),
            class: args.counselor_class.clone(),
            net_id: args.net_id,
            last_jason: args.last_jason,
            bot_hooks: args.bot_hooks,
            last_match: args.last_match.clone(),
        });
    }
    if let Some(port) = args.host.filter(|_| !(args.fly || args.aerial)) {
        app.add_plugins(host::HostPlugin { port });
    }
    if args.perf {
        app.add_plugins(perf::PerfPlugin);
    }
    // Enlighten's indirect light, solved into its atlases and on the meshes as lightmaps, as the game always has it
    // (F13_ENLIGHTEN=0: off, for comparisons; none for --unlit or a dedicated server, which draws nothing).
    if enlighten_enabled() && !args.unlit && !args.dedicated {
        app.add_plugins(enlighten::EnlightenPlugin);
    }
    let exit = app.run();
    // Bots report success or failure through the exit code.
    if exit.is_error() {
        std::process::exit(1);
    }
}

/// `--host`'s default port (f13-host's too).
fn f13_host_port() -> u16 {
    7713
}

pub(crate) fn to_transform(x: &Xform) -> Transform {
    let g = x.to_gltf_space();
    Transform {
        translation: Vec3::from_array(g.translation),
        rotation: Quat::from_array(g.rotation).normalize(),
        scale: Vec3::from_array(g.scale),
    }
}

/// The cabin layout, as SCWorldSettings picks it when the match's level loads: one of its RandomLevels, at
/// `min((int)(n * rand() / RAND_MAX), n - 1)` (the game's `rand()`; here the match seed's).
fn random_level(main: &LevelLayout, seed: u64) -> Option<String> {
    let levels: Vec<&str> = main
        .actors
        .iter()
        .filter(|a| a.class == "SCWorldSettings")
        .find_map(|a| a.props.get("RandomLevels").and_then(|l| l.as_array()))?
        .iter()
        .filter_map(|l| l.as_str())
        .collect();
    let mut rng = f13_core::spawn::Rng::new(seed ^ 0x1a40_u64);
    levels.get(rng.index(levels.len())).map(|l| l.to_string())
}

/// SCWorldSettings' rain roll: rand()/32767 <= RainChance (its constructor's 0.5 unless the map sets it).
fn rain_roll(main: &LevelLayout, seed: u64) -> bool {
    let chance = main.actors.iter().filter(|a| a.class == "SCWorldSettings").find_map(|a| a.props.get("RainChance").and_then(|v| v.as_f64())).unwrap_or(0.5) as f32;
    let mut rng = f13_core::spawn::Rng::new(seed ^ 0x7a17_u64);
    weather::rains(chance, rng.frand())
}

fn counselor_start(layouts: &[LevelLayout]) -> Vec3 {
    counselor_starts(layouts).first().copied().unwrap_or(Vec3::ZERO)
}

/// The layouts' counselor starts, in the level's order. PLACEHOLDER: which start each counselor gets (the game mode's
/// choice isn't read); the first goes to the local counselor, the next to the others.
fn counselor_starts(layouts: &[LevelLayout]) -> Vec<Vec3> {
    layouts.iter().flat_map(|l| l.actors_of("SCCounselorPlayerStart")).filter_map(|a| a.transform.as_ref()).map(|x| to_transform(x).translation).collect()
}

fn is_grass(mesh: &str) -> bool {
    ["/Foliage/Grass/", "/Foliage/Flowers/"].iter().any(|s| mesh.contains(s))
}

/// A cabin's Sense mesh (its match actor index): the window and crack light, or the stencil box.
#[derive(Clone, Copy)]
enum SenseMesh {
    Beams(usize),
    Stencil(usize),
}

fn spawn_level(mut commands: Commands, assets: Res<AssetServer>, mut level: ResMut<Level>, export: Res<ExportRoot>) {
    let mut scenes: HashMap<&str, Handle<WorldAsset>> = HashMap::new();
    let mut spawned = 0usize;
    // Hiding spots placed in the level are runtime actors (props.rs): their parts are spawned there, not here.
    let classes = f13_data::classes::ClassDb::load(&export.0.join("classes.json")).unwrap_or_default();
    let runtime: std::collections::HashSet<(&str, &str)> = level
        .layouts
        .iter()
        .flat_map(|l| &l.actors)
        .filter(|a| props::is_level_runtime_actor(&classes, &a.class))
        .map(|a| (a.level.as_str(), a.name.as_str()))
        .collect();
    let actor_of = |component: &str| component.split("PersistentLevel.").nth(1).and_then(|s| s.split('.').next()).map(str::to_string);
    // The cabins, by level and actor name: their match actor index (counted as lighting::placed_components counts the
    // level's actors) and the component their Sense meshes (SM_Sense_*) hang off, which their SetSenseEffects lights
    // (sense_view.rs).
    let mut cabins: HashMap<(&str, &str), (usize, Option<&str>)> = HashMap::new();
    for (i, a) in level.layouts.iter().flat_map(|l| &l.actors).filter(|a| a.transform.is_some()).enumerate() {
        if classes.is_child_of(&a.class, "Cabin_Base_C") {
            let lit = classes.get(&a.class).and_then(|c| c.components.iter().find(|c| c.name.starts_with("SM_Sense_"))?.parent.as_deref());
            cabins.insert((a.level.as_str(), a.name.as_str()), (i, lit));
        }
    }
    // (The placements' Enlighten tags, for their lightmaps: enlighten.rs.)
    let enlighten = enlighten_enabled();
    for layout in &level.layouts {
        for p in &layout.placements {
            let component = p.component.rsplit('.').next().unwrap_or_default();
            let cabin = actor_of(&p.component).and_then(|a| cabins.get(&(p.level.as_str(), a.as_str())).copied());
            // A cabin's Sense meshes: hidden until Sense shows the cabin.
            let sense = cabin.filter(|_| component.starts_with("SM_Sense_")).map(|(i, _)| if component.ends_with("_Stencil") { SenseMesh::Stencil(i) } else { SenseMesh::Beams(i) });
            if (p.hidden && sense.is_none()) || (!level.grass && is_grass(&p.mesh)) || (!level.foliage && p.instances.is_some()) {
                continue;
            }
            let lit = cabin.filter(|(_, lit)| *lit == Some(component)).map(|(i, _)| i);
            if actor_of(&p.component).is_some_and(|a| runtime.contains(&(p.level.as_str(), a.as_str()))) {
                continue;
            }
            let scene = scenes
                .entry(p.file.as_str())
                .or_insert_with(|| assets.load(GltfAssetLabel::Scene(0).from_asset(p.file.clone())))
                .clone();
            let xforms = p.instances.iter().flatten().chain(p.transform.iter());
            let overrides = materials::MaterialOverrides::new(&p.material_map);
            // The sky sphere casts no shadow: its surface (9 km out) is past the moon's dynamic shadow distance (150 m),
            // which leaves it out of the engine's cascades; Bevy's would take it in.
            let sky = p.file.ends_with("SM_SkySphere.glb");
            let placed = Placed { cast_shadow: p.cast_shadow && !sky, cull_m: p.cull_distance * VIEW_DISTANCE_SCALE / 100.0 };
            // A phone box's mesh (its broken one swapped in by police.rs).
            let box_mesh = actor_of(&p.component)
                .filter(|a| p.component.ends_with(".Mesh") && layout.actors.iter().any(|x| x.name == *a && x.level == p.level && classes.is_child_of(&x.class, "SCPhoneJunctionBox")));
            let instances = p.instances.as_ref().map_or(0, Vec::len);
            for (j, x) in xforms.enumerate() {
                let mut e = commands.spawn((WorldAssetRoot(scene.clone()), to_transform(x)));
                if let Some(o) = &overrides {
                    e.insert(o.clone());
                }
                if placed != Placed::default() {
                    e.insert(placed);
                }
                if let Some(s) = &p.spline {
                    e.insert(spline_meshes::Bend(s.clone()));
                }
                if let Some(a) = &box_mesh {
                    e.insert(police::BoxMesh { actor: a.clone(), file: p.file.clone(), materials: p.material_map.clone() });
                }
                match sense {
                    Some(SenseMesh::Beams(i)) => {
                        e.insert((sense_view::SenseBeams(i), Visibility::Hidden));
                    }
                    Some(SenseMesh::Stencil(i)) => {
                        e.insert((sense_view::SenseStencil(i), Visibility::Hidden));
                    }
                    None => {}
                }
                if let Some(i) = lit {
                    e.insert(sense_view::CabinPart(i));
                }
                // (A single placement's component, for its Enlighten lightmap: enlighten.rs.)
                let component = || p.component.rsplit_once(':').map_or(p.component.as_str(), |(_, c)| c).to_string();
                if enlighten && p.instances.is_none() {
                    e.insert(enlighten::EnlightenPrimitive { level: p.level.clone(), component: component() });
                } else if enlighten && j < instances {
                    e.insert(enlighten::EnlightenInstance { level: p.level.clone(), component: component(), index: j });
                }
                spawned += 1;
            }
        }
        for l in &layout.landscapes {
            let xf = layout
                .actors
                .iter()
                .find(|a| a.level == l.level && a.name == l.actor)
                .and_then(|a| a.transform.as_ref())
                .map(to_transform)
                .unwrap_or_default();
            let mut e = commands.spawn((WorldAssetRoot(assets.load(GltfAssetLabel::Scene(0).from_asset(l.file.clone()))), xf));
            if enlighten {
                e.insert(enlighten::EnlightenLandscape { level: l.level.clone() });
            }
        }
    }
    info!("spawned {spawned} mesh instances from {} unique meshes", scenes.len());
    level.scenes = scenes.values().map(|h| h.clone().untyped()).collect();

    if !level.fly {
        return;
    }
    // Start the camera at the first counselor spawn point.
    let start = counselor_start(&level.layouts);
    let camera = if let Some((eye, target)) = level.camera {
        Transform::from_translation(eye).looking_at(target, Vec3::Y)
    } else if level.aerial {
        Transform::from_translation(start + Vec3::new(-120.0, 120.0, -120.0)).looking_at(start, Vec3::Y)
    } else {
        Transform::from_translation(start + Vec3::Y * 2.0).looking_to(Vec3::X, Vec3::Y)
    };
    commands.spawn((
        Camera3d::default(),
        camera,
        FreeCamera { walk_speed: 6.0, run_speed: 30.0, ..default() },
    ));
}

/// Waits until every mesh scene (and its dependencies) has loaded, lets materials settle, saves a frame, exits.
/// A placement's draw settings, for the meshes its scene spawns: no shadow (CastShadow off), and not drawn past
/// `cull_m` from the camera (0: always). Most of the map's shrubs, ivy and small rocks cast none and vanish within
/// 20-60 m; without this every one would be drawn, into every shadow cascade too.
#[derive(Component, Clone, Copy, PartialEq, Debug)]
struct Placed {
    cast_shadow: bool,
    cull_m: f32,
}

impl Default for Placed {
    fn default() -> Self {
        Self { cast_shadow: true, cull_m: 0.0 }
    }
}

/// r.ViewDistanceScale: the engine multiplies every max draw distance and foliage cull distance by it
/// (FScalabilityCVars::ViewDistanceScale, cached by FUN_1417a1250; SceneVisibility and the HISM proxy's culling).
/// The game's DefaultScalability.ini sets it per sg.ViewDistanceQuality (0.85, 1, 1.5, 3); its Windows device profile
/// (DefaultDeviceProfiles.ini) defaults the level to 2.
pub(crate) const VIEW_DISTANCE_SCALE: f32 = 1.5;

fn apply_placed(mut commands: Commands, added: Query<Entity, Added<Mesh3d>>, parents: Query<&ChildOf>, placed: Query<&Placed>) {
    for e in &added {
        let Some(p) = parents.iter_ancestors(e).find_map(|a| placed.get(a).ok()) else { continue };
        let mut ec = commands.entity(e);
        if !p.cast_shadow {
            ec.insert(bevy::light::NotShadowCaster);
        }
        if p.cull_m > 0.0 {
            // Fading over the last tenth (UE dithers its cull distance fade too).
            ec.insert(bevy::camera::visibility::VisibilityRange { start_margin: 0.0..0.0, end_margin: p.cull_m * 0.9..p.cull_m, use_aabb: true });
        }
    }
}

fn take_screenshot(
    mut commands: Commands,
    mut plan: ResMut<ScreenshotPlan>,
    level: Res<Level>,
    assets: Res<AssetServer>,
    materials: Res<Assets<StandardMaterial>>,
    ported: materials::PortedMaterials,
    mips: Res<mipmaps::Pending>,
    time: Res<Time>,
    mut exit: MessageWriter<AppExit>,
) {
    if let Some(at) = plan.shot_at {
        // Loaded: shoot once the extra wait is over, then give the capture a few frames before exiting.
        let now = time.elapsed_secs();
        if now >= at && plan.frames_loaded < 120 + 1 {
            plan.frames_loaded = 121;
            commands.spawn(Screenshot::primary_window()).observe(save_to_disk(plan.path.clone()));
        } else if plan.frames_loaded > 120 {
            plan.frames_loaded += 1;
            if plan.frames_loaded == 160 {
                exit.write(AppExit::Success);
            }
        }
        return;
    }
    if level.scenes.is_empty() || !level.scenes.iter().all(|h| assets.is_loaded_with_dependencies(h.id())) {
        return;
    }
    // Meshes whose material textures are still loading are not drawn at all, so wait for every texture too.
    // Images made in memory (fallbacks, landscape weights) have no load state: they are ready.
    let pending = |t: &Handle<Image>| {
        matches!(assets.get_load_state(t.id()), Some(bevy::asset::LoadState::Loading | bevy::asset::LoadState::NotLoaded))
    };
    let textures_pending = materials.iter().any(|(_, m)| [&m.base_color_texture, &m.normal_map_texture].into_iter().flatten().any(pending))
        || ported.images().any(pending)
        || mips.busy();
    if textures_pending {
        plan.frames_loaded = 0;
        return;
    }
    plan.frames_loaded += 1;
    if plan.frames_loaded == 120 {
        plan.shot_at = Some(time.elapsed_secs() + plan.after);
    }
}
