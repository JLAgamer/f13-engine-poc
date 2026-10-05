//! f13-game: currently a viewer for the MVP map (Crystal Lake, small) built from the player's own export.
//!
//!   cargo run -p f13-game --release -- --export <dir made by tools/f13-export> [--layout 1-4] [--seed n] [--near-car] [--bot-escape] [--grass]
//!       [--fly] [--model] [--demo] [--tuning file] [--aerial] [--no-foliage] [--screenshot out.png]
//!   Default: play as Vanessa (see player.rs for controls). --fly: free camera instead.   (screenshot: wait for loading, save one frame, exit)

mod bot;
mod interact;
mod player;
mod props;
mod vehicle;

use bevy::asset::AssetPlugin;
use bevy::camera_controller::free_camera::{FreeCamera, FreeCameraPlugin};
use bevy::gltf::GltfMaterialName;
use bevy::image::{ImageLoaderSettings, ImageSampler};
use bevy::light::GlobalAmbientLight;
use bevy::prelude::*;
use bevy::render::view::screenshot::{save_to_disk, Screenshot};
use bevy::world_serialization::WorldAssetRoot;
use f13_data::level::{LevelLayout, Xform};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

const MAP: &str = "CrystalLake_Small";
const LAYOUTS: u32 = 4;

struct Args {
    export: PathBuf,
    layout: u32,
    seed: Option<u64>,
    near_car: bool,
    bot_escape: bool,
    grass: bool,
    aerial: bool,
    fly: bool,
    model: bool,
    demo: bool,
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
    let mut grass = false;
    let mut aerial = false;
    let mut fly = false;
    let mut model = false;
    let mut demo = false;
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
            "--grass" => grass = true,
            "--aerial" => aerial = true,
            "--fly" => fly = true,
            "--model" => model = true,
            "--demo" => demo = true,
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
    Args { export, layout: layout.clamp(1, LAYOUTS), seed, near_car, bot_escape, grass, aerial, fly, model, demo, tuning, foliage, screenshot }
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

/// One entry of the exporter's `materials.json`.
#[derive(serde::Deserialize)]
#[serde(rename_all = "PascalCase")]
struct MaterialInfo {
    file: String,
    blend: String,
    two_sided: bool,
}

/// Material name (as written into the glTF) -> exported material info.
#[derive(Resource, Default)]
struct MaterialIndex {
    root: PathBuf,
    entries: HashMap<String, MaterialInfo>,
    built: HashMap<String, Handle<StandardMaterial>>,
}

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
    let entries = std::fs::read_to_string(args.export.join("materials.json"))
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_else(|| {
            warn!("no materials.json in the export; meshes will be untextured");
            HashMap::new()
        });
    let materials = MaterialIndex { root: args.export.clone(), entries, ..default() };
    let spawn = counselor_start(&layouts);

    let mut app = App::new();
    if !(args.fly || args.aerial) {
        let path = args.tuning.clone().unwrap_or_else(f13_data::default_path);
        let tuning = f13_data::load(&path).unwrap_or_else(|e| {
            eprintln!("{}: {e}", path.display());
            std::process::exit(1);
        });
        let stats = *tuning.characters.get("Vanessa").expect("Vanessa missing from tuning.json");
        app.insert_resource(player::PlayerTuning { counselor: tuning.counselor, movement: tuning.counselor_movement, stats })
            .add_plugins(player::PlayerPlugin { spawn, show_model: args.model, demo: args.demo })
            .add_plugins((interact::InteractPlugin, vehicle::VehiclePlugin));
        if args.bot_escape {
            app.add_plugins(bot::BotPlugin);
        }
    }
    let seed = args.seed.unwrap_or_else(|| {
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs())
    });
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
        .add_plugins(FreeCameraPlugin)
        .insert_resource(Level { layouts, grass: args.grass, foliage: args.foliage, aerial: args.aerial, fly: args.fly || args.aerial, scenes: vec![] })
        .insert_resource(materials)
        .insert_resource(ExportRoot(args.export.clone()))
        .add_plugins(props::PropsPlugin { seed, near_car: args.near_car && !(args.fly || args.aerial) })
        .insert_resource(GlobalAmbientLight { brightness: 400.0, ..default() })
        .add_systems(Startup, spawn_level)
        .add_systems(Update, (apply_materials, strip_vertex_colors))
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
            for x in xforms {
                commands.spawn((WorldAssetRoot(scene.clone()), to_transform(x)));
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

/// Unreal vertex colours are material data (masks, wind weights), not colour; Bevy would multiply the base colour
/// and alpha by them, turning many meshes black or invisible. Drop them as meshes load.
fn strip_vertex_colors(mut events: MessageReader<AssetEvent<Mesh>>, mut meshes: ResMut<Assets<Mesh>>) {
    for e in events.read() {
        if let AssetEvent::Added { id } = e {
            if let Some(mut mesh) = meshes.get_mut(*id) {
                if mesh.contains_attribute(Mesh::ATTRIBUTE_COLOR) {
                    mesh.remove_attribute(Mesh::ATTRIBUTE_COLOR);
                }
            }
        }
    }
}

/// Swaps the placeholder glTF materials for ones built from the exported material JSON (diffuse + normal).
fn apply_materials(
    mut commands: Commands,
    added: Query<(Entity, &GltfMaterialName), Added<GltfMaterialName>>,
    mut index: ResMut<MaterialIndex>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    assets: Res<AssetServer>,
) {
    for (entity, name) in &added {
        let handle = if let Some(h) = index.built.get(&name.0) {
            h.clone()
        } else {
            let Some(info) = index.entries.get(&name.0) else { continue };
            let h = materials.add(build_material(&index.root, info, &assets));
            index.built.insert(name.0.clone(), h.clone());
            h
        };
        commands.entity(entity).insert(MeshMaterial3d(handle));
    }
}

fn build_material(root: &Path, info: &MaterialInfo, assets: &AssetServer) -> StandardMaterial {
    let mut m = StandardMaterial { perceptual_roughness: 0.8, ..default() };
    match info.blend.as_str() {
        "BLEND_Masked" => m.alpha_mode = AlphaMode::Mask(0.33),
        "BLEND_Translucent" => m.alpha_mode = AlphaMode::Blend,
        "BLEND_Additive" => m.alpha_mode = AlphaMode::Add,
        _ => {}
    }
    if info.two_sided {
        m.double_sided = true;
        m.cull_mode = None;
    }
    let Ok(text) = std::fs::read_to_string(root.join(&info.file)) else { return m };
    let Ok(json) = serde_json::from_str::<serde_json::Value>(&text) else { return m };
    let tex = |key: &str| {
        json["Textures"][key]["ObjectPath"].as_str().map(|p| format!("{}.png", p.rsplit_once('.').map_or(p, |(a, _)| a)))
    };
    if let Some(p) = tex("PM_Diffuse") {
        m.base_color_texture = Some(
            assets.load_builder().with_settings(|s: &mut ImageLoaderSettings| s.sampler = ImageSampler::linear()).load(p),
        );
    }
    if let Some(p) = tex("PM_Normals") {
        m.normal_map_texture =
            Some(assets.load_builder().with_settings(|s: &mut ImageLoaderSettings| s.is_srgb = false).load(p));
        m.flip_normal_map_y = true; // Unreal normal maps are DirectX-style (green down)
    }
    m
}

/// Waits until every mesh scene (and its dependencies) has loaded, lets materials settle, saves a frame, exits.
fn take_screenshot(
    mut commands: Commands,
    mut plan: ResMut<ScreenshotPlan>,
    level: Res<Level>,
    assets: Res<AssetServer>,
    materials: Res<Assets<StandardMaterial>>,
    mut exit: MessageWriter<AppExit>,
) {
    if level.scenes.is_empty() || !level.scenes.iter().all(|h| assets.is_loaded_with_dependencies(h.id())) {
        return;
    }
    // Meshes whose material textures are still loading are not drawn at all, so wait for every texture too.
    let textures_pending = materials.iter().any(|(_, m)| {
        [&m.base_color_texture, &m.normal_map_texture]
            .into_iter()
            .flatten()
            .any(|t| !assets.is_loaded_with_dependencies(t.id()) && !matches!(assets.get_load_state(t.id()), Some(bevy::asset::LoadState::Failed(_))))
    });
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
