//! f13-game: currently a viewer for the MVP map (Crystal Lake, small) built from the player's own export.
//!
//!   cargo run -p f13-game --release -- --export <dir made by tools/f13-export> [--layout 1-4] [--seed n] [--near-car] [--bot-escape] [--bot-heal] [--bot-dodge] [--bot-door] [--grass]
//!       [--fly] [--model] [--demo] [--fear 0-100] [--health 1-100] [--speed x] [--tuning file] [--aerial] [--no-foliage] [--screenshot out.png]
//!   Default: play as Vanessa (see player.rs for controls). --fly: free camera instead.   (screenshot: wait for loading, save one frame, exit)

mod bot;
mod doors;
mod interact;
mod materials;
mod mipmaps;
mod player;
mod props;
mod windows;
mod vehicle;

use bevy::asset::AssetPlugin;
use bevy::camera_controller::free_camera::{FreeCamera, FreeCameraPlugin};
use bevy::light::GlobalAmbientLight;
use bevy::prelude::*;
use bevy::render::view::screenshot::{save_to_disk, Screenshot};
use bevy::world_serialization::WorldAssetRoot;
use f13_data::level::{LevelLayout, Xform};
use std::collections::HashMap;
use std::path::PathBuf;

const MAP: &str = "CrystalLake_Small";
const LAYOUTS: u32 = 4;

struct Args {
    export: PathBuf,
    layout: u32,
    seed: Option<u64>,
    near_car: bool,
    bot_escape: bool,
    bot_heal: bool,
    bot_dodge: bool,
    bot_door: bool,
    grass: bool,
    aerial: bool,
    fly: bool,
    model: bool,
    demo: bool,
    fear: f32,
    /// Game clock multiplier (testing: run bots faster than real time).
    speed: f32,
    health: Option<f32>,
    tuning: Option<PathBuf>,
    foliage: bool,
    screenshot: Option<PathBuf>,
}

fn parse_args() -> Args {
    let mut export = None;
    let mut layout = 1;
    let mut seed = None;
    let mut near_car = false;
    let mut bot_escape = false;
    let mut bot_heal = false;
    let mut bot_dodge = false;
    let mut bot_door = false;
    let mut grass = false;
    let mut aerial = false;
    let mut fly = false;
    let mut model = false;
    let mut demo = false;
    let mut fear = 0.0;
    let mut speed = 1.0f32;
    let mut health = None;
    let mut tuning = None;
    let mut foliage = true;
    let mut screenshot = None;
    let mut it = std::env::args().skip(1);
    while let Some(a) = it.next() {
        match a.as_str() {
            "--export" => export = it.next().map(PathBuf::from),
            "--layout" => layout = it.next().and_then(|v| v.parse().ok()).unwrap_or(1),
            "--seed" => seed = it.next().and_then(|v| v.parse().ok()),
            "--near-car" => near_car = true,
            "--bot-escape" => bot_escape = true,
            "--bot-heal" => bot_heal = true,
            "--bot-dodge" => bot_dodge = true,
            "--bot-door" => bot_door = true,
            "--grass" => grass = true,
            "--aerial" => aerial = true,
            "--fly" => fly = true,
            "--model" => model = true,
            "--demo" => demo = true,
            "--speed" => speed = it.next().and_then(|v| v.parse().ok()).unwrap_or(1.0f32).clamp(0.1, 16.0),
            "--fear" => fear = it.next().and_then(|v| v.parse().ok()).unwrap_or(0.0f32).clamp(0.0, 100.0),
            "--health" => health = it.next().and_then(|v| v.parse().ok()),
            "--tuning" => tuning = it.next().map(PathBuf::from),
            "--no-foliage" => foliage = false,
            "--screenshot" => screenshot = it.next().map(PathBuf::from),
            other => {
                eprintln!("unknown argument {other}");
                std::process::exit(2);
            }
        }
    }
    let Some(export) = export else {
        eprintln!("usage: f13-game --export <dir from tools/f13-export> [--layout 1-{LAYOUTS}] [--grass]");
        std::process::exit(2);
    };
    Args { export, layout: layout.clamp(1, LAYOUTS), seed, near_car, bot_escape, bot_heal, bot_dodge, bot_door, grass, aerial, fly, model, demo, fear, speed, health, tuning, foliage, screenshot }
}

#[derive(Resource)]
pub(crate) struct Level {
    pub(crate) layouts: Vec<LevelLayout>,
    grass: bool,
    foliage: bool,
    aerial: bool,
    fly: bool,
    scenes: Vec<UntypedHandle>,
}

/// The export directory (asset root).
#[derive(Resource)]
pub(crate) struct ExportRoot(pub(crate) PathBuf);

#[derive(Resource)]
struct ScreenshotPlan {
    path: PathBuf,
    frames_loaded: u32,
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
        }
        let Some(entries) = std::fs::read_to_string(export.join("anims.json")).ok().and_then(|t| serde_json::from_str::<HashMap<String, Entry>>(&t).ok()) else {
            warn!("no anims.json in the export (run tools/f13-export again): dodges and stumbles won't move you");
            return Self::default();
        };
        // Counselor meshes face +Y in their own space: forward = +Y, right = -X.
        Self(entries.into_iter().filter(|(_, e)| e.length > 0.0).map(|(k, e)| (k, Vec2::new(-e.root_motion[0], e.root_motion[1]) / e.length)).collect())
    }
}

/// Random numbers for gameplay rolls.
#[derive(Resource)]
pub(crate) struct GameRng(pub f13_core::spawn::Rng);

fn main() {
    let args = parse_args();
    let load = |name: &str| {
        let p = args.export.join("levels").join(format!("{name}.json"));
        LevelLayout::load(&p).unwrap_or_else(|e| {
            eprintln!("{}: {e} (run tools/f13-export first)", p.display());
            std::process::exit(1);
        })
    };
    let layouts = vec![load(MAP), load(&format!("CLsm_Design_SL{}", args.layout))];
    let spawn = counselor_start(&layouts);

    let seed = args.seed.unwrap_or_else(|| {
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs())
    });
    let mut app = App::new();
    // Gameplay randomness (stumbles, ...), separate from the match layout so both repeat with --seed.
    app.insert_resource(Anims::load(&args.export));
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
    if !(args.fly || args.aerial) {
        let path = args.tuning.clone().unwrap_or_else(f13_data::default_path);
        let tuning = f13_data::load(&path).unwrap_or_else(|e| {
            eprintln!("{}: {e}", path.display());
            std::process::exit(1);
        });
        let stats = *tuning.characters.get("Vanessa").expect("Vanessa missing from tuning.json");
        app.insert_resource(player::PlayerTuning {
            counselor: tuning.counselor,
            movement: tuning.counselor_movement,
            stats,
            first_aid: tuning.first_aid,
            minigame: tuning.minigame.clone(),
            repair_hold_time: tuning.repair_hold_time,
            vehicle_start: tuning.vehicle_start.clone(),
        })
            .add_plugins(player::PlayerPlugin { spawn, show_model: args.model, demo: args.demo, fear: args.fear, health: args.health })
            .add_plugins((interact::InteractPlugin, vehicle::VehiclePlugin));
        if args.bot_escape {
            app.add_plugins(bot::BotPlugin(bot::Goal::Escape));
        } else if args.bot_heal {
            app.add_plugins(bot::BotPlugin(bot::Goal::Heal));
        } else if args.bot_dodge {
            app.add_plugins(bot::BotPlugin(bot::Goal::Dodge));
        } else if args.bot_door {
            app.add_plugins(bot::BotPlugin(bot::Goal::Door));
        }
    }
    if let Some(path) = args.screenshot.clone() {
        app.insert_resource(ScreenshotPlan { path, frames_loaded: 0 }).add_systems(Update, take_screenshot);
    }
    app
        .add_plugins(
            DefaultPlugins
                .set(AssetPlugin { file_path: args.export.to_string_lossy().into_owned(), ..default() })
                // Unreal meshes carry up to 8 UV sets; Bevy warns once per extra set per mesh.
                .set(bevy::log::LogPlugin { filter: "wgpu=error,naga=warn,bevy_gltf::loader=error".into(), ..default() }),
        )
        .add_plugins((FreeCameraPlugin, bevy::diagnostic::FrameTimeDiagnosticsPlugin::default()))
        .insert_resource(Level { layouts, grass: args.grass, foliage: args.foliage, aerial: args.aerial, fly: args.fly || args.aerial, scenes: vec![] })
        .add_plugins((materials::MaterialsPlugin { export: args.export.clone() }, mipmaps::MipmapsPlugin))
        .insert_resource(ExportRoot(args.export.clone()))
        .add_plugins(props::PropsPlugin { seed, near_car: args.near_car && !(args.fly || args.aerial) })
        .insert_resource(GlobalAmbientLight { brightness: 400.0, ..default() })
        .add_systems(Startup, spawn_level)
        .run();
}

pub(crate) fn to_transform(x: &Xform) -> Transform {
    let g = x.to_gltf_space();
    Transform {
        translation: Vec3::from_array(g.translation),
        rotation: Quat::from_array(g.rotation).normalize(),
        scale: Vec3::from_array(g.scale),
    }
}

fn counselor_start(layouts: &[LevelLayout]) -> Vec3 {
    layouts
        .iter()
        .flat_map(|l| l.actors_of("SCCounselorPlayerStart"))
        .find_map(|a| a.transform.as_ref())
        .map(|x| to_transform(x).translation)
        .unwrap_or(Vec3::ZERO)
}

fn is_grass(mesh: &str) -> bool {
    ["/Foliage/Grass/", "/Foliage/Flowers/"].iter().any(|s| mesh.contains(s))
}

fn spawn_level(mut commands: Commands, assets: Res<AssetServer>, mut level: ResMut<Level>) {
    let mut scenes: HashMap<&str, Handle<WorldAsset>> = HashMap::new();
    let mut spawned = 0usize;
    for layout in &level.layouts {
        for p in &layout.placements {
            if p.hidden || (!level.grass && is_grass(&p.mesh)) || (!level.foliage && p.instances.is_some()) {
                continue;
            }
            let scene = scenes
                .entry(p.file.as_str())
                .or_insert_with(|| assets.load(GltfAssetLabel::Scene(0).from_asset(p.file.clone())))
                .clone();
            let xforms = p.instances.iter().flatten().chain(p.transform.iter());
            let overrides = materials::MaterialOverrides::new(&p.material_map);
            for x in xforms {
                let mut e = commands.spawn((WorldAssetRoot(scene.clone()), to_transform(x)));
                if let Some(o) = &overrides {
                    e.insert(o.clone());
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
            commands.spawn((WorldAssetRoot(assets.load(GltfAssetLabel::Scene(0).from_asset(l.file.clone()))), xf));
        }
    }
    info!("spawned {spawned} mesh instances from {} unique meshes", scenes.len());
    level.scenes = scenes.values().map(|h| h.clone().untyped()).collect();

    commands.spawn((
        DirectionalLight { illuminance: 8_000.0, shadow_maps_enabled: true, ..default() },
        Transform::from_rotation(Quat::from_euler(EulerRot::YXZ, 0.6, -0.9, 0.0)),
    ));

    if !level.fly {
        return;
    }
    // Start the camera at the first counselor spawn point.
    let start = counselor_start(&level.layouts);
    let camera = if level.aerial {
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
fn take_screenshot(
    mut commands: Commands,
    mut plan: ResMut<ScreenshotPlan>,
    level: Res<Level>,
    assets: Res<AssetServer>,
    materials: Res<Assets<StandardMaterial>>,
    ported: materials::PortedMaterials,
    mips: Res<mipmaps::Pending>,
    mut exit: MessageWriter<AppExit>,
) {
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
        commands.spawn(Screenshot::primary_window()).observe(save_to_disk(plan.path.clone()));
    } else if plan.frames_loaded == 160 {
        exit.write(AppExit::Success);
    }
}
