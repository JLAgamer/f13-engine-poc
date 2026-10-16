//! Enlighten's indirect light at run time (F13-decomp MODLOG "Enlighten GI"; the data f13-data's enlighten.rs reads
//! from `export/enlighten/`). The game runs Enlighten 3's CPU solver every frame, its lights being movable: each
//! system's input samples (clusters of points on its surfaces) take the light falling on them (DoDirectInputLighting),
//! times their albedo (DoIndirectInputLighting); each system's transport gathers its own and its neighbours' clusters
//! and the environment (the sky) into its output pixels (SolveIrradianceTask), placed in its level's atlas.
//!
//! f13-rs: the systems of the loaded levels, re-solved a few times a second off the main thread's hot path; their
//! output in one atlas image per level (`EnlightenAtlases`). PLACEHOLDERS: the clusters' albedo is one grey (the
//! game renders it from the materials: EnlightenAlbedoPS); every point sees every light (the visibility data, GEVS,
//! isn't read); the lamps aren't inputs yet, nor the bounce (the output resampled back to the clusters, GERD); the
//! environment's clusters all take the sky's mean colour.

use bevy::asset::RenderAssetUsages;
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use f13_data::enlighten::{InputWorkspace, Transport};
use std::collections::HashMap;

/// How often the systems are re-solved (s).
const SOLVE_EVERY_S: f32 = 0.25;
/// PLACEHOLDER: every cluster's albedo.
const ALBEDO: f32 = 0.35;

pub struct EnlightenPlugin;

impl Plugin for EnlightenPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, load).add_systems(Update, solve.run_if(resource_exists::<Enlighten>));
    }
}

/// One system: its transport (None for the systems with no output), its input samples, where its output goes.
struct System {
    package: String,
    transport: Option<Transport>,
    workspace: InputWorkspace,
    atlas_base: (u32, u32),
    /// Its clusters' light this solve (rgba).
    input: Vec<Vec4>,
}

#[derive(Resource)]
pub struct Enlighten {
    systems: Vec<System>,
    /// A system by its Geo GUID's second u32 (UE SystemGuid's first).
    by_id: HashMap<u32, usize>,
    since: f32,
    dumped: bool,
}

/// Each loaded level's Enlighten atlas (its systems' output irradiance, rgba32 float).
#[derive(Resource, Default)]
pub struct EnlightenAtlases(pub HashMap<String, Handle<Image>>);

fn load(mut commands: Commands, export: Res<crate::ExportRoot>, level: Res<crate::Level>, mut images: ResMut<Assets<Image>>) {
    let dir = export.0.join("enlighten");
    let Ok(text) = std::fs::read_to_string(dir.join("index.json")) else {
        info!("enlighten: no export/enlighten (F13-decomp/enlighten/extract.py): no indirect light");
        return;
    };
    let Ok(index) = serde_json::from_str::<serde_json::Value>(&text) else { return };
    // The loaded levels' packages (their names' last part).
    let packages: Vec<String> = level.layouts.iter().flat_map(|l| &l.levels).map(|l| l.rsplit('/').next().unwrap_or(l).to_string()).collect();
    let mut systems = Vec::new();
    let mut by_id = HashMap::new();
    for s in index["systems"].as_array().into_iter().flatten() {
        let package = s["package"].as_str().unwrap_or("").to_string();
        if !packages.contains(&package) {
            continue;
        }
        let name = format!("{package}.{}", s["name"].as_str().unwrap_or(""));
        let Some(workspace) = std::fs::read(dir.join(format!("{name}.geiw.bin"))).ok().and_then(|b| InputWorkspace::read(&b)) else { continue };
        let transport = std::fs::read(dir.join(format!("{name}.gesg.bin"))).ok().and_then(|b| Transport::read(&b));
        let alloc = &s["atlas"][0];
        let atlas_base = (alloc["AtlasBaseX"].as_u64().unwrap_or(0) as u32, alloc["AtlasBaseY"].as_u64().unwrap_or(0) as u32);
        // (UE SystemGuid's first u32, its hex's first eight digits little-endian.)
        let id = s["guid"].as_str().and_then(|g| u32::from_str_radix(g.get(0..8)?, 16).ok()).map(u32::swap_bytes);
        if let Some(id) = id {
            by_id.insert(id, systems.len());
        }
        systems.push(System { package, transport, workspace, atlas_base, input: Vec::new() });
    }
    let mut atlases = EnlightenAtlases::default();
    for (package, a) in index["atlases"].as_object().into_iter().flatten() {
        let (w, h) = (a["width"].as_u64().unwrap_or(0) as u32, a["height"].as_u64().unwrap_or(0) as u32);
        if !packages.contains(package) || w == 0 || h == 0 {
            continue;
        }
        let image = Image::new_fill(Extent3d { width: w, height: h, depth_or_array_layers: 1 }, TextureDimension::D2, &[0; 16], TextureFormat::Rgba32Float, RenderAssetUsages::default());
        atlases.0.insert(package.clone(), images.add(image));
    }
    let solved = systems.iter().filter(|s| s.transport.is_some()).count();
    info!("enlighten: {} systems ({solved} with output) in {} levels' atlases", systems.len(), atlases.0.len());
    commands.insert_resource(Enlighten { systems, by_id, since: SOLVE_EVERY_S, dumped: false });
    commands.insert_resource(atlases);
}

/// Unreal space from Bevy's (x, y, z) = (ue x, ue z, ue y) / 100: directions only.
fn to_unreal_dir(v: Vec3) -> Vec3 {
    Vec3::new(v.x, v.z, v.y)
}

fn solve(
    time: Res<Time>,
    mut gi: ResMut<Enlighten>,
    atlases: Res<EnlightenAtlases>,
    mut images: ResMut<Assets<Image>>,
    moons: Query<(&DirectionalLight, &GlobalTransform), With<crate::weather::Moon>>,
    ambient: Option<Res<bevy::light::GlobalAmbientLight>>,
) {
    gi.since += time.delta_secs();
    if gi.since < SOLVE_EVERY_S {
        return;
    }
    gi.since = 0.0;
    // The clusters' light: the moon's (its illuminance, as lux on a surface facing it).
    let moon = moons.iter().next().map(|(l, g)| {
        let c = LinearRgba::from(l.color);
        (to_unreal_dir(g.forward().as_vec3()), Vec4::new(c.red, c.green, c.blue, 0.0) * l.illuminance)
    });
    for s in &mut gi.systems {
        s.input = match moon {
            Some((dir, color)) => s.workspace.directional(dir, color).into_iter().map(|v| v * ALBEDO).collect(),
            None => vec![Vec4::ZERO; s.workspace.clusters.len()],
        };
    }
    // The environment: the sky's mean colour on every environment cluster (PLACEHOLDER: the captured sky cube, x the
    // environment's Brightness).
    let sky = ambient.map_or(Vec4::ZERO, |a| {
        let c = LinearRgba::from(a.color) * a.brightness;
        Vec4::new(c.red, c.green, c.blue, 0.0)
    });
    let env = vec![sky; 1 << 16];
    let dump = (!gi.dumped).then(|| std::env::var_os("F13_ENLIGHTEN_DUMP")).flatten();
    gi.dumped = true;
    let gi = &*gi;
    let mut per_atlas: HashMap<&str, Vec<(usize, Vec<Vec4>)>> = HashMap::new();
    for (i, s) in gi.systems.iter().enumerate() {
        let Some(t) = &s.transport else { continue };
        let inputs: Vec<Option<&[Vec4]>> = t
            .inputs
            .iter()
            .map(|id| if id[2] == 0xffff_fffe { Some(&env[..]) } else { gi.by_id.get(&id[1]).map(|&k| &gi.systems[k].input[..]) })
            .collect();
        per_atlas.entry(&s.package).or_default().push((i, t.solve(&inputs, 1.0)));
    }
    for (package, outputs) in per_atlas {
        let Some(mut image) = atlases.0.get(package).and_then(|h| images.get_mut(h)) else { continue };
        let (w, h) = (image.width() as usize, image.height() as usize);
        let Some(data) = image.data.as_mut() else { continue };
        for (i, out) in outputs {
            let s = &gi.systems[i];
            let t = s.transport.as_ref().unwrap();
            let (bx, by) = (s.atlas_base.0 as usize, s.atlas_base.1 as usize);
            for y in 0..t.height as usize {
                for x in 0..t.width as usize {
                    let v = out[y * t.width as usize + x];
                    let o = ((by + y) * w + bx + x) * 16;
                    if let Some(px) = data.get_mut(o..o + 16) {
                        for (k, c) in v.to_array().iter().enumerate() {
                            px[4 * k..4 * k + 4].copy_from_slice(&c.to_le_bytes());
                        }
                    }
                }
            }
        }
        // (F13_ENLIGHTEN_DUMP=<dir>: the first solve's atlases as PPMs, scaled to their brightest pixel, for checking.)
        if let Some(dir) = &dump {
            let px: Vec<f32> = data.chunks_exact(4).map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]])).collect();
            let max = px.iter().copied().fold(1e-9f32, f32::max);
            let rgb: Vec<u8> = px.chunks_exact(4).flat_map(|c| [c[0], c[1], c[2]].map(|v| ((v / max).clamp(0.0, 1.0).sqrt() * 255.0) as u8)).collect();
            let path = std::path::Path::new(dir).join(format!("{package}.ppm"));
            let mut file = format!("P6
{w} {h}
255
").into_bytes();
            file.extend(rgb);
            match std::fs::write(&path, file) {
                Ok(()) => info!("enlighten: {} (brightest {max})", path.display()),
                Err(e) => warn!("enlighten: {}: {e}", path.display()),
            }
        }
    }
}
