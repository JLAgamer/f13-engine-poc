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
        app.add_systems(Startup, load).add_systems(Update, (solve, light_meshes).run_if(resource_exists::<Enlighten>));
    }
}

/// One system: its transport (None for the systems with no output), its input samples, where its output goes.
struct System {
    package: String,
    transport: Option<Transport>,
    workspace: InputWorkspace,
    atlas_base: (u32, u32),
    /// Its input points' box (Unreal space, cm).
    bounds: (Vec3, Vec3),
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
    /// Per (lamp, system): each cluster's packets seen from the lamp.
    visibility: HashMap<(Entity, usize), Vec<Vec<bool>>>,
}

/// A local light's IndirectLightingIntensity (lighting.rs): its share of the indirect light.
#[derive(Component, Clone, Copy)]
pub struct IndirectIntensity(pub f32);

/// A local light as an Enlighten input: Unreal space (cm), its UE intensity (lumens / 4 pi), its falloff (Some
/// exponent: UE's radial falloff; None: inverse square), its range (cm), its spot cone (axis, cos outer, cos inner).
struct Lamp {
    at: Vec3,
    intensity: Vec4,
    falloff: Option<f32>,
    range: f32,
    cone: Option<(Vec3, f32, f32)>,
}

impl Lamp {
    /// The irradiance it gives a point (cm) facing `n`, unoccluded.
    fn at_point(&self, p: Vec3, n: Vec3) -> Vec4 {
        let d = self.at - p;
        let dist = d.length();
        if dist >= self.range || dist < 1e-3 {
            return Vec4::ZERO;
        }
        let l = d / dist;
        let lambert = n.dot(l).max(0.0);
        if lambert <= 0.0 {
            return Vec4::ZERO;
        }
        let x = dist / self.range;
        let falloff = match self.falloff {
            // (Unreal: Intensity x (1 - (d/R)^2)^exponent lux.)
            Some(e) => (1.0 - x * x).max(0.0).powf(e),
            // (Inverse square in metres, windowed to the range: (1 - (d/R)^4)^2.)
            None => (1.0 - x.powi(4)).max(0.0).powi(2) / (dist / 100.0).powi(2).max(1e-4),
        };
        let cone = self.cone.map_or(1.0, |(axis, cos_outer, cos_inner)| (((-l).dot(axis) - cos_outer) / (cos_inner - cos_outer).max(1e-4)).clamp(0.0, 1.0).powi(2));
        self.intensity * (falloff * cone * lambert)
    }
}

/// A single placement's component (its level package and its path after the level's ':'), for its meshes' lightmap.
#[derive(Component, Clone, Debug)]
pub struct EnlightenPrimitive {
    pub level: String,
    pub component: String,
}

/// A mesh given its Enlighten lightmap (or found to have none).
#[derive(Component)]
struct Lightmapped;

/// Each primitive's atlas UVs per Unreal vertex (packed u16 u, v), by (level, component): `<level>.prims.bin`
/// (F13-decomp/enlighten/prims.py, from the atlas's records); and each mesh's vertices in Unreal's order (position,
/// UV0), by its glb: `meshes/<glb>.uvb` (f13-export enlighten-meshes).
#[derive(Resource, Default)]
struct Primitives {
    uvs: HashMap<(String, String), Vec<u32>>,
    meshes: HashMap<String, Option<Vec<[f32; 5]>>>,
    /// Meshes lit, not matched, of placements not in the data (logged as they change).
    counts: [usize; 3],
    logged: [usize; 3],
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
        let points = workspace.clusters.iter().flatten().flat_map(|p| p.points);
        let bounds = points.fold((Vec3::splat(f32::MAX), Vec3::splat(f32::MIN)), |(lo, hi), p| (lo.min(p), hi.max(p)));
        systems.push(System { package, transport, workspace, atlas_base, bounds, input: Vec::new() });
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
    let mut prims = Primitives::default();
    for package in &packages {
        let Ok(b) = std::fs::read(dir.join(format!("{package}.prims.bin"))) else { continue };
        let mut o = 4;
        let n = b.get(0..4).map_or(0, |x| u32::from_le_bytes(x.try_into().unwrap()));
        for _ in 0..n {
            let Some(len) = b.get(o..o + 2).map(|x| u16::from_le_bytes(x.try_into().unwrap()) as usize) else { break };
            let comp = String::from_utf8_lossy(&b[o + 2..o + 2 + len]).to_string();
            o += 2 + len;
            let count = u32::from_le_bytes(b[o..o + 4].try_into().unwrap()) as usize;
            o += 4;
            let uvs = b[o..o + 4 * count].chunks_exact(4).map(|x| u32::from_le_bytes(x.try_into().unwrap())).collect();
            o += 4 * count;
            prims.uvs.insert((package.clone(), comp), uvs);
        }
    }
    info!("enlighten: {} primitives with atlas UVs", prims.uvs.len());
    commands.insert_resource(prims);
    let solved = systems.iter().filter(|s| s.transport.is_some()).count();
    info!("enlighten: {} systems ({solved} with output) in {} levels' atlases", systems.len(), atlases.0.len());
    commands.insert_resource(Enlighten { systems, by_id, since: SOLVE_EVERY_S, dumped: false, visibility: HashMap::new() });
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
    (points, spots): (Query<(Entity, &PointLight, &GlobalTransform, &IndirectIntensity)>, Query<(Entity, &SpotLight, &GlobalTransform, &IndirectIntensity)>),
    collision: Option<crate::collision::Collision>,
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
    // The lamps (their indirect share in): Unreal space, their UE intensity (lumens / 4 pi).
    let ue = |p: Vec3| Vec3::new(p.x, p.z, p.y) * 100.0;
    let rgb = |c: Color, k: f32| {
        let c = LinearRgba::from(c);
        Vec4::new(c.red, c.green, c.blue, 0.0) * k
    };
    let mut lamps: Vec<(Entity, Lamp)> = Vec::new();
    for (e, l, g, k) in &points {
        if l.intensity > 0.0 && k.0 > 0.0 {
            lamps.push((e, Lamp { at: ue(g.translation()), intensity: rgb(l.color, l.intensity / (4.0 * std::f32::consts::PI) * k.0), falloff: (l.radius < 0.0).then_some(-l.radius), range: l.range * 100.0, cone: None }));
        }
    }
    for (e, l, g, k) in &spots {
        if l.intensity > 0.0 && k.0 > 0.0 {
            let axis = to_unreal_dir(g.forward().as_vec3()).normalize_or_zero();
            lamps.push((e, Lamp { at: ue(g.translation()), intensity: rgb(l.color, l.intensity / (4.0 * std::f32::consts::PI) * k.0), falloff: (l.radius < 0.0).then_some(-l.radius), range: l.range * 100.0, cone: Some((axis, l.outer_angle.cos(), l.inner_angle.cos())) }));
        }
    }
    let world = collision.as_ref().map(|c| c.all());
    let gi_ref = &mut *gi;
    for (si, s) in gi_ref.systems.iter_mut().enumerate() {
        s.input = match moon {
            Some((dir, color)) => s.workspace.directional(dir, color),
            None => vec![Vec4::ZERO; s.workspace.clusters.len()],
        };
        for (le, lamp) in &lamps {
            // (Only the lamps whose reach meets the system's box.)
            if lamp.at.clamp(s.bounds.0, s.bounds.1).distance(lamp.at) >= lamp.range {
                continue;
            }
            // (Its packets' visibility from the lamp, one ray each from the packet's centre, cached: the lamps don't
            // move. PLACEHOLDER for the visibility data, GEVS.)
            let vis = gi_ref.visibility.entry((*le, si)).or_insert_with(|| {
                s.workspace
                    .clusters
                    .iter()
                    .map(|packets| {
                        packets
                            .iter()
                            .map(|pk| {
                                let c = pk.points.iter().copied().sum::<Vec3>() / 4.0 + pk.normal * 5.0;
                                if c.distance(lamp.at) >= lamp.range {
                                    return false;
                                }
                                let b = |v: Vec3| Vec3::new(v.x, v.z, v.y) / 100.0;
                                world.as_ref().is_none_or(|w| f13_core::collision::segment_hit(w, b(c), b(lamp.at), 0.1).is_none_or(|t| t > 0.98))
                            })
                            .collect()
                    })
                    .collect()
            });
            for ((packets, seen), light) in s.workspace.clusters.iter().zip(vis.iter()).zip(s.input.iter_mut()) {
                if packets.is_empty() {
                    continue;
                }
                let mut sum = Vec4::ZERO;
                for (pk, &v) in packets.iter().zip(seen) {
                    if v {
                        sum += pk.points.iter().map(|p| lamp.at_point(*p, pk.normal)).sum::<Vec4>() / 4.0;
                    }
                }
                *light += sum / packets.len() as f32;
            }
        }
        // (The input in the environment's units, radiance: lit by a sky of radiance 1 an open pixel gathers 1
        // (the transport's test). A cluster's light is irradiance E: its radiance ALBEDO x E / pi.)
        for v in &mut s.input {
            *v *= ALBEDO * std::f32::consts::FRAC_1_PI;
        }
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
    // (F13_ENLIGHTEN_SCALE=<k>: the output times k, to see which surfaces take the lightmap: debugging.)
    let debug_scale = std::env::var("F13_ENLIGHTEN_SCALE").ok().and_then(|v| v.parse::<f32>().ok()).unwrap_or(1.0);
    let gi = &*gi;
    let mut per_atlas: HashMap<&str, Vec<(usize, Vec<Vec4>)>> = HashMap::new();
    for (i, s) in gi.systems.iter().enumerate() {
        let Some(t) = &s.transport else { continue };
        let inputs: Vec<Option<&[Vec4]>> = t
            .inputs
            .iter()
            .map(|id| if id[2] == 0xffff_fffe { Some(&env[..]) } else { gi.by_id.get(&id[1]).map(|&k| &gi.systems[k].input[..]) })
            .collect();
        per_atlas.entry(&s.package).or_default().push((i, t.solve(&inputs, debug_scale)));
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

/// Unreal's LOD 0 vertices of a mesh (position in cm, UV0): `meshes/<glb without .glb>.uvb`.
fn read_uvb(path: &std::path::Path) -> Option<Vec<[f32; 5]>> {
    let b = std::fs::read(path).ok()?;
    let f = |o: usize| f32::from_le_bytes(b[o..o + 4].try_into().unwrap());
    let n = u32::from_le_bytes(b.get(4..8)?.try_into().ok()?) as usize;
    (b.len() >= 8 + 20 * n).then(|| (0..n).map(|i| [f(8 + 20 * i), f(12 + 20 * i), f(16 + 20 * i), f(20 + 20 * i), f(24 + 20 * i)]).collect())
}

/// A vertex's key for matching a glb's vertices to Unreal's: its position (cm) to 0.01 and UV0 to 1e-4.
fn vertex_key(p: [f32; 3], uv: [f32; 2]) -> [i64; 5] {
    [(p[0] * 100.0).round() as i64, (p[1] * 100.0).round() as i64, (p[2] * 100.0).round() as i64, (uv[0] * 1e4).round() as i64, (uv[1] * 1e4).round() as i64]
}

/// A single placement's meshes, once loaded, take its atlas UVs as their UV1 (each glb vertex matched to its Unreal
/// vertex by position and UV0: the glbs' are merged and reordered) and its level's atlas as their lightmap. Bevy adds
/// lightmap x the diffuse colour to the indirect light: the atlas holds the solve's output as it is, the radiance
/// gathered (a Lambertian under it reflects albedo x that). PLACEHOLDER: a vertex with several Unreal matches (the same position and UV, other normals) takes the
/// first.
#[allow(clippy::type_complexity)]
fn light_meshes(
    mut commands: Commands,
    time: Res<Time>,
    export: Res<crate::ExportRoot>,
    mut prims: ResMut<Primitives>,
    atlases: Res<EnlightenAtlases>,
    assets: Res<AssetServer>,
    mut meshes: ResMut<Assets<Mesh>>,
    new: Query<(Entity, &Mesh3d), Without<Lightmapped>>,
    parents: Query<&ChildOf>,
    owners: Query<&EnlightenPrimitive>,
) {
    let Primitives { uvs: all_uvs, meshes: ue_meshes, counts, logged } = &mut *prims;
    for (e, mesh3d) in &new {
        let Some(owner) = parents.iter_ancestors(e).find_map(|a| owners.get(a).ok()) else {
            commands.entity(e).insert(Lightmapped);
            continue;
        };
        let (Some(uvs), Some(atlas)) = (all_uvs.get(&(owner.level.clone(), owner.component.clone())), atlases.0.get(&owner.level)) else {
            counts[2] += 1;
            commands.entity(e).insert(Lightmapped);
            continue;
        };
        let Some(mesh) = meshes.get(&mesh3d.0) else { continue };
        let Some(glb) = assets.get_path(mesh3d.0.id()).map(|p| p.path().to_string_lossy().replace('\\', "/")) else {
            commands.entity(e).insert(Lightmapped);
            continue;
        };
        let ue = ue_meshes
            .entry(glb.clone())
            .or_insert_with(|| read_uvb(&export.0.join("enlighten/meshes").join(glb.trim_end_matches(".glb").to_string() + ".uvb")));
        let (Some(ue), Some(bevy::mesh::VertexAttributeValues::Float32x3(pos)), Some(bevy::mesh::VertexAttributeValues::Float32x2(uv0))) =
            (ue.as_ref(), mesh.attribute(Mesh::ATTRIBUTE_POSITION), mesh.attribute(Mesh::ATTRIBUTE_UV_0))
        else {
            commands.entity(e).insert(Lightmapped);
            continue;
        };
        let mut index: HashMap<[i64; 5], usize> = HashMap::new();
        for (i, v) in ue.iter().enumerate() {
            index.entry(vertex_key([v[0] / 100.0, v[1] / 100.0, v[2] / 100.0], [v[3], v[4]])).or_insert(i);
        }
        // (A glb position (x, y, z) is Unreal's (x, z, y) / 100.)
        let mut matched = 0;
        let uv1: Vec<[f32; 2]> = pos
            .iter()
            .zip(uv0)
            .map(|(p, t)| {
                let i = index.get(&vertex_key([p[0], p[2], p[1]], *t)).copied();
                matched += i.is_some() as usize;
                i.and_then(|i| uvs.get(i)).map_or([0.0, 0.0], |&w| [(w & 0xffff) as f32 / 65535.0, (w >> 16) as f32 / 65535.0])
            })
            .collect();
        if matched * 10 < pos.len() * 9 {
            counts[1] += 1;
            warn!("enlighten: {glb} ({}): {matched} of {} vertices matched; no lightmap", owner.component, pos.len());
            commands.entity(e).insert(Lightmapped);
            continue;
        }
        let lit = mesh.clone().with_inserted_attribute(Mesh::ATTRIBUTE_UV_1, uv1);
        counts[0] += 1;
        commands.entity(e).insert((Mesh3d(meshes.add(lit)), bevy::pbr::Lightmap { image: atlas.clone(), uv_rect: Rect::new(0.0, 0.0, 1.0, 1.0), bicubic_sampling: false }, Lightmapped));
    }
    if *counts != *logged && time.elapsed_secs() > 1.0 && (time.elapsed_secs() as u32) != (time.elapsed_secs() - time.delta_secs()) as u32 {
        info!("enlighten: meshes lit {}, not matched {}, of placements not in the data {}", counts[0], counts[1], counts[2]);
        *logged = *counts;
    }
}
