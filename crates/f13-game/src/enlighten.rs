//! Enlighten's indirect light at run time (F13-decomp MODLOG "Enlighten GI"; the data f13-data's enlighten.rs reads
//! from `export/enlighten/`). The game runs Enlighten 3's CPU solver every frame, its lights being movable: each
//! system's input samples (clusters of points on its surfaces) take the light falling on them (DoDirectInputLighting),
//! times their albedo (DoIndirectInputLighting); each system's transport gathers its own and its neighbours' clusters
//! and the environment (the sky) into its output pixels (SolveIrradianceTask), placed in its level's atlas.
//!
//! f13recomp: the systems of the loaded levels, re-solved a few times a second off the main thread's hot path; their
//! output in one atlas image per level (`EnlightenAtlases`). The clusters' albedo is each system's saved albedo
//! buffer; the moon's visibility its precomputed visibility (GEVS); the environment is the weather level's (uniform).
//! The lamps' visibility is a ray per input point (the game's a shadow-map test per point: the precomputed data serves
//! directional lights only).

use bevy::asset::RenderAssetUsages;
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use f13_data::enlighten::{InputWorkspace, ProbeOctree, ProbeSh, ProbeTransport, Resampling, Transport, Visibility};
use std::collections::HashMap;

/// How often the systems are re-solved (s).
const SOLVE_EVERY_S: f32 = 0.25;
/// PLACEHOLDER: the albedo of a system's clusters when its saved albedo buffer is missing.
const ALBEDO: f32 = 0.35;

pub struct EnlightenPlugin;

impl Plugin for EnlightenPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<LandscapeWeightUv>()
            .add_systems(Startup, load)
            .add_systems(Update, (solve, light_meshes, landscape_weight_uv).run_if(resource_exists::<Enlighten>));
    }
}

/// One system: its transport (None for the systems with no output), its input samples, where its output goes.
struct System {
    package: String,
    transport: Option<Transport>,
    workspace: InputWorkspace,
    atlas_base: (u32, u32),
    /// Its output resampled onto its clusters (the bounce), and its last output.
    resampling: Option<Resampling>,
    output: Vec<Vec4>,
    /// Its input points' box (Unreal space, cm).
    bounds: (Vec3, Vec3),
    /// Its clusters' light this solve (rgba).
    input: Vec<Vec4>,
    /// Its clusters' albedo (linear rgb): the system's saved albedo buffer (`<name>.alb.bin`, f13-export enlighten), an 8-bit
    /// rgba per cluster, linear (x / 255)^2 as DoIndirectInputLighting takes it.
    albedo: Vec<Vec4>,
    /// Its clusters' precomputed visibility of directional lights (GEVS), and the moon's last (its direction, each
    /// cluster's), kept while the moon doesn't turn.
    visibility: Option<Visibility>,
    moon_seen: Option<(Vec3, Vec<f32>)>,
}

/// The solver: its state here between solves, moved into a task on the async compute pool for each (the main thread
/// only gathers the lights before and copies the atlases and the probe volume in after).
#[derive(Resource)]
pub struct Enlighten {
    state: Option<Box<Solver>>,
    task: Option<bevy::tasks::Task<(Box<Solver>, Solved)>>,
    since: f32,
    dumped: bool,
}

struct Solver {
    systems: Vec<System>,
    /// A system by its Geo GUID's second u32 (UE SystemGuid's first).
    by_id: HashMap<u32, usize>,
    /// Per (lamp, system): each cluster's packets' points seen from the lamp.
    visibility: HashMap<(Entity, usize), Vec<Vec<[bool; 4]>>>,
    /// The environment's radiance, every texel of its cube alike (see `environment`).
    environment: Vec4,
    /// The loaded levels' probe sets, and per 64 m cell (Unreal space) the sets over it.
    probe_sets: Vec<ProbeSet>,
    probe_cells: HashMap<IVec3, Vec<usize>>,
}

/// What a solve takes from the world: the moon (Unreal direction, colour x lux), the lamps, the static collision (for
/// the lamps' visibility), the probe volume's centre, the debug scale.
struct Frame {
    moon: Option<(Vec3, Vec4)>,
    lamps: Vec<(Entity, Lamp)>,
    world: Option<std::sync::Arc<f13_core::collision::TriGrid>>,
    volume_at: Option<Vec3>,
    debug_scale: f32,
}

/// A solve's results: per system with output its package, atlas base, size and output (times the debug scale), and
/// the probe volume's voxels (its centre, its texels as Rgba16Float).
struct Solved {
    outputs: Vec<(String, (u32, u32), (usize, usize), Vec<Vec4>)>,
    volume: Option<(Vec3, Vec<u8>)>,
    ms: f32,
}

/// A probe set (EnlightenProbeSet: the light for what has no lightmap, moving or not; f13-data ProbeTransport,
/// ProbeOctree), its probes' last light (the real ones, then the virtual).
struct ProbeSet {
    transport: ProbeTransport,
    octree: ProbeOctree,
    real: usize,
    values: Vec<ProbeSh>,
}

/// A probe set's cell: its octree's 64 m cube (every set's origin on a 6400 cm grid).
const PROBE_CELL: f32 = 6400.0;

fn probe_cell(p: Vec3) -> IVec3 {
    (p / PROBE_CELL).floor().as_ivec3()
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

/// An instanced placement's instance (its level package, its component's path after ':', its index), for its meshes'
/// lightmap: the instanced vertex shader's Enlighten UV is the mesh's stream x the instance's UVScaleBias scale + its
/// bias (M_Leaves_Master's s061: mad o5.xy, v13.xy, v14.xy, v14.zw). The instances' biases fall in the systems whose
/// boxes hold them (instances.py: all 48,528 Terrain instances checked): the light where the instance stands.
/// PLACEHOLDER: the mesh's stream taken as zero (the foliage meshes have no Enlighten UVs in their package's UV buffer
/// collection): one atlas point per instance. (Every instance's scale is (1, 1) and its bias inside the atlas: a
/// non-zero stream would spread an instance over the whole atlas, so the stream bound must be zero.)
#[derive(Component, Clone, Debug)]
pub struct EnlightenInstance {
    pub level: String,
    pub component: String,
    pub index: usize,
}

/// A landscape (its level package), for its components' lightmap. A landscape component isn't one of its atlas's
/// primitives: its system's output covers it flat, one Terrain system per one to four components (one 63-quad
/// component per 18 x 18-ish chart). Each component's map from world XY to the atlas is fitted from the data
/// (f13-export enlighten: its clusters' places, GEIW, against the output pixels each resamples, GERD;
/// residuals mostly under a pixel) into `<level>.land.json`. The game's has the same form: its terrain
/// shaders' Enlighten UV (M_Terrain_CrystalLake s008: mad o3.xy, v5.xy, cb0[6].xy, cb0[6].zw) is a UV stream times a
/// per-draw scale and bias, set per component at run time; PLACEHOLDER: those constants fitted rather than read.
#[derive(Component, Clone, Debug)]
pub struct EnlightenLandscape {
    pub level: String,
}

/// The landscape's weight-map UV as a function of its UV0 (scale u, v, offset u, v), its glb's UV1 being exactly
/// UV0 x scale + offset (Landscape_0: 1/316, 1/379, -63/316, -63/379): found as its meshes take the lightmap UV in
/// their UV1, then handed to the terrain materials (materials.rs TerrainParams::weight_uv). One landscape per map.
#[derive(Resource, Default)]
struct LandscapeWeightUv(Option<Vec4>);

fn landscape_weight_uv(uv: Res<LandscapeWeightUv>, mut terrains: ResMut<Assets<crate::materials::TerrainMaterial>>) {
    let Some(v) = uv.0 else { return };
    let stale: Vec<_> = terrains.iter().filter(|(_, m)| m.extension.params.weight_uv != v).map(|(id, _)| id).collect();
    for id in stale {
        if let Some(mut m) = terrains.get_mut(id) {
            m.extension.params.weight_uv = v;
        }
    }
}

/// A level's landscape cells: the grid (UE cm) and per cell its world XY -> atlas UV map.
struct Land {
    origin: f32,
    size: f32,
    cells: HashMap<(i32, i32), [[f32; 3]; 2]>,
}

/// A mesh given its Enlighten lightmap (or found to have none).
#[derive(Component)]
struct Lightmapped;

/// Each primitive's atlas UVs per Unreal vertex (packed u16 u, v), by (level, component): `<level>.prims.bin`
/// (f13-export enlighten, from the atlas's records); and each mesh's vertices in Unreal's order (position,
/// UV0), by its glb: `meshes/<glb>.uvb` (f13-export enlighten-meshes).
#[derive(Resource, Default)]
struct Primitives {
    uvs: HashMap<(String, String), Vec<u32>>,
    /// Each instanced component's instances' UVScaleBias (scale u, v, bias u, v), by (level, component):
    /// `<level>.inst.bin` (f13-export enlighten).
    instances: HashMap<(String, String), Vec<[f32; 4]>>,
    /// Per mesh, its copy with a zero UV1 (the instances' lightmap UV comes from their Lightmap's rect).
    zero_uv1: HashMap<AssetId<Mesh>, Handle<Mesh>>,
    /// Each level's landscape cells (`<level>.land.json`).
    land: HashMap<String, Land>,
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
        info!("enlighten: no export/enlighten (setup's Reinstall, or f13-export enlighten-meshes and enlighten): no indirect light");
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
        let resampling = std::fs::read(dir.join(format!("{name}.gerd.bin"))).ok().and_then(|b| Resampling::read(&b));
        let n = workspace.clusters.len();
        let visibility = std::fs::read(dir.join(format!("{name}.gevs.bin"))).ok().and_then(|b| Visibility::read(&b));
        let albedo = std::fs::read(dir.join(format!("{name}.alb.bin")))
            .ok()
            .filter(|b| b.len() == 4 * n)
            .map(|b| b.chunks_exact(4).map(|c| Vec4::new(c[0] as f32, c[1] as f32, c[2] as f32, 0.0) / 255.0).map(|c| c * c).collect())
            .unwrap_or_else(|| vec![Vec4::new(ALBEDO, ALBEDO, ALBEDO, 0.0); n]);
        let alloc = &s["atlas"][0];
        let atlas_base = (alloc["AtlasBaseX"].as_u64().unwrap_or(0) as u32, alloc["AtlasBaseY"].as_u64().unwrap_or(0) as u32);
        // (UE SystemGuid's first u32, its hex's first eight digits little-endian.)
        let id = s["guid"].as_str().and_then(|g| u32::from_str_radix(g.get(0..8)?, 16).ok()).map(u32::swap_bytes);
        if let Some(id) = id {
            by_id.insert(id, systems.len());
        }
        let points = workspace.clusters.iter().flatten().flat_map(|p| p.points);
        let bounds = points.fold((Vec3::splat(f32::MAX), Vec3::splat(f32::MIN)), |(lo, hi), p| (lo.min(p), hi.max(p)));
        systems.push(System { package, transport, workspace, resampling, output: Vec::new(), atlas_base, bounds, input: Vec::new(), albedo, visibility, moon_seen: None });
    }
    let mut atlases = EnlightenAtlases::default();
    for (package, a) in index["atlases"].as_object().into_iter().flatten() {
        let (w, h) = (a["width"].as_u64().unwrap_or(0) as u32, a["height"].as_u64().unwrap_or(0) as u32);
        if !packages.contains(package) || w == 0 || h == 0 {
            continue;
        }
        // (Half floats: filterable on every adapter (Rgba32Float needs FLOAT32_FILTERABLE), for the bilinear sample the
        // lightmap shader divides by the coverage in alpha, as the game's base pass does.)
        let image = Image::new_fill(Extent3d { width: w, height: h, depth_or_array_layers: 1 }, TextureDimension::D2, &[0; 8], TextureFormat::Rgba16Float, RenderAssetUsages::default());
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
    for package in &packages {
        let Ok(b) = std::fs::read(dir.join(format!("{package}.inst.bin"))) else { continue };
        let u32_at = |o: usize| b.get(o..o + 4).map(|x| u32::from_le_bytes(x.try_into().unwrap()));
        let (Some(n), mut o) = (u32_at(0), 4) else { continue };
        for _ in 0..n {
            let Some(len) = b.get(o..o + 2).map(|x| u16::from_le_bytes(x.try_into().unwrap()) as usize) else { break };
            let comp = String::from_utf8_lossy(&b[o + 2..o + 2 + len]).to_string();
            o += 2 + len;
            let Some(count) = u32_at(o).map(|c| c as usize) else { break };
            o += 4;
            let Some(data) = b.get(o..o + 16 * count) else { break };
            let v = data.chunks_exact(16).map(|c| std::array::from_fn(|k| f32::from_le_bytes(c[4 * k..4 * k + 4].try_into().unwrap()))).collect();
            o += 16 * count;
            prims.instances.insert((package.clone(), comp), v);
        }
    }
    for package in &packages {
        let Some(j) = std::fs::read_to_string(dir.join(format!("{package}.land.json"))).ok().and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok()) else { continue };
        let f = |v: &serde_json::Value| v.as_f64().unwrap_or(0.0) as f32;
        let cells = j["cells"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|c| ((c["cell"][0].as_i64().unwrap_or(0) as i32, c["cell"][1].as_i64().unwrap_or(0) as i32), std::array::from_fn(|r| std::array::from_fn(|k| f(&c["m"][r][k])))))
            .collect();
        prims.land.insert(package.clone(), Land { origin: f(&j["origin"]), size: f(&j["size"]), cells });
    }
    info!("enlighten: {} primitives with atlas UVs, {} instanced, {} landscapes", prims.uvs.len(), prims.instances.len(), prims.land.len());
    commands.insert_resource(prims);
    let solved = systems.iter().filter(|s| s.transport.is_some()).count();
    info!("enlighten: {} systems ({solved} with output) in {} levels' atlases", systems.len(), atlases.0.len());
    let mut probe_sets = Vec::new();
    let mut probe_cells: HashMap<IVec3, Vec<usize>> = HashMap::new();
    for p in index["probe_sets"].as_array().into_iter().flatten() {
        let package = p["package"].as_str().unwrap_or("");
        if !packages.iter().any(|k| k == package) {
            continue;
        }
        let name = format!("{package}.{}", p["name"].as_str().unwrap_or(""));
        let f = |v: &serde_json::Value| v.as_f64().unwrap_or(0.0) as f32;
        let origin = Vec3::new(f(&p["origin"][0]), f(&p["origin"][1]), f(&p["origin"][2]));
        let real = p["probes"].as_u64().unwrap_or(0) as usize;
        let (Some(transport), Some(octree)) = (
            std::fs::read(dir.join(format!("{name}.gesh.bin"))).ok().and_then(|b| ProbeTransport::read(&b)),
            std::fs::read(dir.join(format!("{name}.gepo.bin"))).ok().and_then(|b| ProbeOctree::read(&b, std::fs::read(dir.join(format!("{name}.gepv.bin"))).ok().as_deref(), origin, f(&p["size"]), real)),
        ) else {
            continue;
        };
        probe_cells.entry(probe_cell(origin + Vec3::splat(octree.size * 0.5))).or_default().push(probe_sets.len());
        probe_sets.push(ProbeSet { transport, octree, real, values: Vec::new() });
    }
    info!("enlighten: {} probe sets ({} probes)", probe_sets.len(), probe_sets.iter().map(|p| p.real).sum::<usize>());
    let environment = environment(&level);
    info!("enlighten: environment {:.3} {:.3} {:.3}", environment.x, environment.y, environment.z);
    commands.insert_resource(Enlighten {
        state: Some(Box::new(Solver { systems, by_id, visibility: HashMap::new(), environment, probe_sets, probe_cells })),
        task: None,
        since: SOLVE_EVERY_S,
        dumped: false,
    });
    commands.insert_resource(atlases);
}

/// The environment the transports gather (their input entry [0, 0, 0xfffffffe, 0]): a cube's texels, 16 x 16 per face
/// then its 8, 4, 2 and 1 texel mips (2046 in all; faces +X -X +Y -Y +Z (up) -Z). The loaded levels' environment
/// components add into it (their proxy 0x1409c0200): each its Brightness x EnvironmentColor (linear; the constructors'
/// defaults 1 and white) on every texel, times its CubemapTexture's when it has one. The weather levels' (CLsm_RainOn /
/// CLsm_RainOff's EnlightenTextureEnvironment) name no texture: a uniform 0.01 in the rain, 0.05 when clear.
fn environment(level: &crate::Level) -> Vec4 {
    let mut sum = Vec4::ZERO;
    for c in level.layouts.iter().flat_map(|l| &l.actors).flat_map(|a| &a.components).filter(|c| c.kind == "EnlightenEnvironmentTextureComponent" || c.kind == "EnlightenEnvironmentComponent") {
        let b = c.props.get("Brightness").and_then(|v| v.as_f64()).unwrap_or(1.0) as f32;
        // (An FColor, sRGB: Conv_ColorToLinearColor.)
        let ch = |k: &str| c.props.get("EnvironmentColor").and_then(|v| v.get(k)).and_then(|v| v.as_f64()).map_or(1.0, |v| Color::srgb_u8(v as u8, 0, 0).to_linear().red);
        sum += Vec4::new(ch("R"), ch("G"), ch("B"), 0.0) * b;
    }
    sum
}

/// Unreal space from Bevy's (x, y, z) = (ue x, ue z, ue y) / 100: directions only.
fn to_unreal_dir(v: Vec3) -> Vec3 {
    Vec3::new(v.x, v.z, v.y)
}

/// Each solve's lights gathered and handed to a task; its results, when ready, copied into the atlases and the probe
/// volume (spawned with the first).
#[allow(clippy::too_many_arguments)]
fn solve(
    mut commands: Commands,
    time: Res<Time>,
    mut gi: ResMut<Enlighten>,
    atlases: Res<EnlightenAtlases>,
    mut images: ResMut<Assets<Image>>,
    moons: Query<(&DirectionalLight, &GlobalTransform), With<crate::weather::Moon>>,
    (points, spots): (Query<(Entity, &PointLight, &GlobalTransform, &IndirectIntensity)>, Query<(Entity, &SpotLight, &GlobalTransform, &IndirectIntensity)>),
    world: Option<Res<crate::collision::WorldCollision>>,
    cameras: Query<(&GlobalTransform, &Camera), With<Camera3d>>,
    mut volumes: Query<(&ProbeVolume, &mut Transform)>,
) {
    if let Some(task) = gi.task.as_mut() {
        let Some((state, solved)) = bevy::tasks::futures::check_ready(task) else { return };
        gi.task = None;
        gi.state = Some(state);
        let dump = (!gi.dumped).then(|| std::env::var_os("F13_ENLIGHTEN_DUMP")).flatten();
        gi.dumped = true;
        apply(&mut commands, &atlases, &mut images, &mut volumes, solved, dump);
        return;
    }
    gi.since += time.delta_secs();
    if gi.since < SOLVE_EVERY_S {
        return;
    }
    let Some(mut state) = gi.state.take() else { return };
    gi.since = 0.0;
    // The clusters' light: the moon's (its illuminance, as lux on a surface facing it).
    let moon = moons.iter().next().map(|(l, g)| {
        let c = LinearRgba::from(l.color);
        (to_unreal_dir(g.forward().as_vec3()), Vec4::new(c.red, c.green, c.blue, 0.0) * l.illuminance)
    });
    // The lamps (their indirect share in): Unreal space, their UE intensity (lumens / 4 pi). PLACEHOLDER: an
    // inverse-square lamp's input takes the deferred lighting's x16 (lighting.rs); the UE integration's fill of
    // Enlighten's light (its colour at +0x90) isn't read.
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
    // (The view's camera: the first active 3D one, the player's or --fly's; the volume on its voxel grid.)
    let volume_at = cameras.iter().find(|(_, c)| c.is_active).map(|(g, _)| (g.translation() / PROBE_VOXEL_M).round() * PROBE_VOXEL_M);
    // (F13_ENLIGHTEN_SCALE=<k>: what's drawn times k, to see which surfaces take the lightmap and the probes: debugging.)
    let debug_scale = std::env::var("F13_ENLIGHTEN_SCALE").ok().and_then(|v| v.parse::<f32>().ok()).unwrap_or(1.0);
    // (The lamps' visibility traces the static world: the lamps' rays are cached, so moving colliders would only be
    // seen at their first solve.)
    let frame = Frame { moon, lamps, world: world.map(|w| w.grid.clone()), volume_at, debug_scale };
    gi.task = Some(bevy::tasks::AsyncComputeTaskPool::get().spawn(async move {
        let solved = state.step(frame);
        (state, solved)
    }));
}

/// A solve's results into the atlases and the probe volume.
fn apply(
    commands: &mut Commands,
    atlases: &EnlightenAtlases,
    images: &mut Assets<Image>,
    volumes: &mut Query<(&ProbeVolume, &mut Transform)>,
    solved: Solved,
    dump: Option<std::ffi::OsString>,
) {
    let mut by_atlas: HashMap<&str, Vec<&((u32, u32), (usize, usize), Vec<Vec4>)>> = HashMap::new();
    let outs: Vec<(String, ((u32, u32), (usize, usize), Vec<Vec4>))> = solved.outputs.into_iter().map(|(p, b, s, o)| (p, (b, s, o))).collect();
    for (package, o) in &outs {
        by_atlas.entry(package).or_default().push(o);
    }
    for (package, outputs) in by_atlas {
        let Some(mut image) = atlases.0.get(package).and_then(|h| images.get_mut(h)) else { continue };
        let (w, h) = (image.width() as usize, image.height() as usize);
        let Some(data) = image.data.as_mut() else { continue };
        for ((bx, by), (tw, th), out) in outputs {
            for y in 0..*th {
                for x in 0..*tw {
                    let v = out[y * tw + x];
                    let o = ((*by as usize + y) * w + *bx as usize + x) * 8;
                    if let Some(px) = data.get_mut(o..o + 8) {
                        for (k, c) in v.to_array().iter().enumerate() {
                            px[2 * k..2 * k + 2].copy_from_slice(&f16_bits(*c).to_le_bytes());
                        }
                    }
                }
            }
        }
        // (F13_ENLIGHTEN_DUMP=<dir>: the first solve's atlases as PPMs, scaled to their brightest pixel, for checking.)
        if let Some(dir) = &dump {
            let px: Vec<f32> = data.chunks_exact(2).map(|b| f16_value(u16::from_le_bytes([b[0], b[1]]))).collect();
            let max = px.iter().copied().fold(1e-9f32, f32::max);
            let rgb: Vec<u8> = px.chunks_exact(4).flat_map(|c| [c[0], c[1], c[2]].map(|v| ((v / max).clamp(0.0, 1.0).sqrt() * 255.0) as u8)).collect();
            let path = std::path::Path::new(dir).join(format!("{package}.ppm"));
            let mut file = format!("P6\n{w} {h}\n255\n").into_bytes();
            file.extend(rgb);
            match std::fs::write(&path, file) {
                Ok(()) => info!("enlighten: {} (brightest {max})", path.display()),
                Err(e) => warn!("enlighten: {}: {e}", path.display()),
            }
        }
    }
    if let Some((at, bytes)) = solved.volume {
        let extent = PROBE_VOXELS.as_vec3() * PROBE_VOXEL_M;
        match volumes.single_mut() {
            Ok((v, mut t)) => {
                if let Some(mut img) = images.get_mut(&v.image) {
                    img.data = Some(bytes);
                }
                t.translation = at;
            }
            Err(_) => {
                let r = PROBE_VOXELS;
                let size = Extent3d { width: r.x, height: r.y * 2, depth_or_array_layers: r.z * 3 };
                let mut image = Image::new_fill(size, TextureDimension::D3, &[0; 8], TextureFormat::Rgba16Float, RenderAssetUsages::default());
                image.data = Some(bytes);
                let image = images.add(image);
                commands.spawn((
                    ProbeVolume { image: image.clone() },
                    bevy::light::IrradianceVolume { voxels: image, intensity: 1.0, affects_lightmapped_meshes: false },
                    Transform::from_translation(at).with_scale(extent),
                ));
                info!("enlighten: probe volume {} x {} x {} voxels of {PROBE_VOXEL_M} m round the camera", r.x, r.y, r.z);
            }
        }
    }
    if !SOLVE_LOGGED.swap(true, std::sync::atomic::Ordering::Relaxed) {
        info!("enlighten: a solve takes {:.1} ms (on the async compute pool)", solved.ms);
    }
}

/// Each of a system's input points seen from `lamp` (see [`Solver::step`]): one ray from the point, 5 cm off its
/// surface, to the lamp through the static world.
fn lamp_visibility(workspace: &InputWorkspace, lamp: &Lamp, world: Option<&f13_core::collision::TriGrid>) -> Vec<Vec<[bool; 4]>> {
    let b = |v: Vec3| Vec3::new(v.x, v.z, v.y) / 100.0;
    workspace
        .clusters
        .iter()
        .map(|packets| {
            packets
                .iter()
                .map(|pk| {
                    pk.points.map(|q| {
                        let c = q + pk.normal * 5.0;
                        c.distance(lamp.at) < lamp.range && world.is_none_or(|w| f13_core::collision::segment_hit(w, b(c), b(lamp.at), 0.1).is_none_or(|t| t > 0.98))
                    })
                })
                .collect()
        })
        .collect()
}

impl Solver {
    /// One solve: the clusters' light, the transports, the probe sets, the probe volume's voxels.
    fn step(&mut self, f: Frame) -> Solved {
        let started = std::time::Instant::now();
        let world = f.world.as_deref();
        // The lamps' visibility not yet cached (new lamps: the intro's staged lights all at once), traced across the
        // threads before the solve (a single thread took ~12 s for the intro's first).
        let todo: Vec<(Entity, usize, &Lamp)> = f
            .lamps
            .iter()
            .flat_map(|(le, lamp)| self.systems.iter().enumerate().map(move |(si, s)| (le, si, s, lamp)))
            .filter(|(le, si, s, lamp)| lamp.at.clamp(s.bounds.0, s.bounds.1).distance(lamp.at) < lamp.range && !self.visibility.contains_key(&(**le, *si)))
            .map(|(le, si, _, lamp)| (*le, si, lamp))
            .collect();
        if !todo.is_empty() {
            let threads = std::thread::available_parallelism().map_or(4, |n| n.get()).min(todo.len());
            let systems = &self.systems;
            let traced: Vec<((Entity, usize), Vec<Vec<[bool; 4]>>)> = std::thread::scope(|sc| {
                let handles: Vec<_> = (0..threads)
                    .map(|t| {
                        let todo = &todo;
                        sc.spawn(move || {
                            todo.iter()
                                .skip(t)
                                .step_by(threads)
                                .map(|&(le, si, lamp)| ((le, si), lamp_visibility(&systems[si].workspace, lamp, world)))
                                .collect::<Vec<_>>()
                        })
                    })
                    .collect();
                handles.into_iter().flat_map(|h| h.join().unwrap_or_default()).collect()
            });
            self.visibility.extend(traced);
        }
        for (si, s) in self.systems.iter_mut().enumerate() {
            s.input = match f.moon {
                Some((dir, color)) => {
                    // (Its visibility, SetDirectionalVisibility's: towards the light.)
                    if let Some(v) = &s.visibility {
                        if s.moon_seen.as_ref().is_none_or(|(d, _)| d.distance_squared(dir) > 1e-8) {
                            s.moon_seen = Some((dir, v.directional(-dir, s.workspace.clusters.len())));
                        }
                    }
                    s.workspace.directional(dir, color, s.moon_seen.as_ref().map(|(_, v)| &v[..]))
                }
                None => vec![Vec4::ZERO; s.workspace.clusters.len()],
            };
            for (le, lamp) in &f.lamps {
                // (Only the lamps whose reach meets the system's box.)
                if lamp.at.clamp(s.bounds.0, s.bounds.1).distance(lamp.at) >= lamp.range {
                    continue;
                }
                // (Each input point's visibility from the lamp, a bit per point as the game's (CalcLightVisibilitySize
                // 0x141d86350: (points + 7) / 8 bytes, set by the engine through the system's light visibility
                // 0x141ddb2f0, from its Enlighten shadow pass, FEnlightenShadowPixelShader): here one ray each from the
                // point (5 cm off its surface) to the lamp, cached, the lamps not moving. PLACEHOLDER: a ray for the
                // shadow map's test.)
                let vis = self.visibility.entry((*le, si)).or_insert_with(|| lamp_visibility(&s.workspace, lamp, world));
                for ((packets, seen), light) in s.workspace.clusters.iter().zip(vis.iter()).zip(s.input.iter_mut()) {
                    if packets.is_empty() {
                        continue;
                    }
                    let mut sum = Vec4::ZERO;
                    for (pk, v) in packets.iter().zip(seen) {
                        sum += pk.points.iter().zip(v).filter(|(_, v)| **v).map(|(p, _)| lamp.at_point(*p, pk.normal)).sum::<Vec4>() / 4.0;
                    }
                    *light += sum / packets.len() as f32;
                }
            }
            // (A cluster's input is its albedo x the irradiance E, as DoDirectInputLighting's kernels add it: no 1 / pi
            // (MODLOG "Enlighten GI: first look"); its bounce the same. PLACEHOLDER: the UE integration's fill of
            // Enlighten's lights isn't read; with E / pi (a radiance, as the environment's) every lamp-lit and moonlit
            // interior was about pi too dark against the user's shots, with E they match: Zane's cabin, Jason's shack,
            // Rod Fielder's logs (MODLOG "Too dark overall").)
            for (v, a) in s.input.iter_mut().zip(&s.albedo) {
                *v *= *a;
            }
            // The bounce (DoIndirectInputLighting): the last solve's output resampled onto the clusters (GERD), already
            // a radiance gathered, times the albedo; each solve feeds the next, as the game's every frame.
            if let (Some(r), Some(t)) = (&s.resampling, &s.transport) {
                if !s.output.is_empty() {
                    for ((v, b), a) in s.input.iter_mut().zip(r.bounce(&s.output, t.width as usize)).zip(&s.albedo) {
                        *v += b * *a;
                    }
                }
            }
        }
        // The environment: uniform (`environment`).
        let env = vec![self.environment; 2046];
        let mut outputs = Vec::new();
        let mut new_outputs = Vec::new();
        for (i, s) in self.systems.iter().enumerate() {
            let Some(t) = &s.transport else { continue };
            let inputs: Vec<Option<&[Vec4]>> = t
                .inputs
                .iter()
                .map(|id| if id[2] == 0xffff_fffe { Some(&env[..]) } else { self.by_id.get(&id[1]).map(|&k| &self.systems[k].input[..]) })
                .collect();
            let out = t.solve(&inputs, 1.0);
            // (The debug scale on what's drawn only: the output itself feeds the bounce.)
            outputs.push((s.package.clone(), s.atlas_base, (t.width as usize, t.height as usize), out.iter().map(|v| (v.truncate() * f.debug_scale).extend(v.w)).collect()));
            new_outputs.push((i, out));
        }
        for (i, out) in new_outputs {
            self.systems[i].output = out;
        }
        // The probe sets, from the same inputs (SolveEntireProbeSetTask), then their virtual probes.
        for ps in &mut self.probe_sets {
            let inputs: Vec<Option<&[Vec4]>> = ps
                .transport
                .inputs
                .iter()
                .map(|id| if id[2] == 0xffff_fffe { Some(&env[..]) } else { self.by_id.get(&id[1]).map(|&k| &self.systems[k].input[..]) })
                .collect();
            let real = ps.transport.solve(&inputs, ps.real);
            ps.values = ps.octree.with_virtuals(&real);
        }
        let volume = f.volume_at.filter(|_| !self.probe_sets.is_empty()).map(|at| (at, self.volume(at, f.debug_scale)));
        Solved { outputs, volume, ms: started.elapsed().as_secs_f32() * 1000.0 }
    }

    /// The probe volume's texels round `at` (Bevy space, on the voxel grid): per voxel its L1 probe in the layout of
    /// Bevy's ambient cube (see [`ProbeVolume`]): the +X slot its L0, -X / +Y / -Y its moment along Bevy's x / y / z;
    /// +Z / -Z zero.
    fn volume(&self, at: Vec3, debug_scale: f32) -> Vec<u8> {
        let r = PROBE_VOXELS;
        let extent = r.as_vec3() * PROBE_VOXEL_M;
        let lo = at - extent * 0.5;
        let (nx, ny, nz) = (r.x as usize, r.y as usize, r.z as usize);
        let cube = |x: usize, y: usize, z: usize| -> [Vec3; 6] {
            // (Bevy (x, y, z) = Unreal (x, z, y) / 100.)
            let b = lo + (Vec3::new(x as f32, y as f32, z as f32) + 0.5) * PROBE_VOXEL_M;
            let ue = Vec3::new(b.x, b.z, b.y) * 100.0;
            let best = self
                .probe_cells
                .get(&probe_cell(ue))
                .into_iter()
                .flatten()
                .filter_map(|&i| self.probe_sets[i].octree.sample(&self.probe_sets[i].values, ue))
                .min_by(|a, b| a.1.total_cmp(&b.1));
            let Some((sh, _)) = best else { return [Vec3::ZERO; 6] };
            let sh = sh.map(|c| c * debug_scale);
            // Bevy's axes: +X = Unreal's x, +Y = its z (up), +Z = its y.
            [sh[0], sh[1], sh[3], sh[2], Vec3::ZERO, Vec3::ZERO]
        };
        // (A slice of z per thread.)
        let threads = std::thread::available_parallelism().map_or(4, |n| n.get()).min(nz);
        let cubes: Vec<[Vec3; 6]> = std::thread::scope(|sc| {
            let handles: Vec<_> = (0..threads)
                .map(|t| {
                    let cube = &cube;
                    sc.spawn(move || (t * nz / threads..(t + 1) * nz / threads).flat_map(|z| (0..ny).flat_map(move |y| (0..nx).map(move |x| cube(x, y, z)))).collect::<Vec<_>>())
                })
                .collect();
            handles.into_iter().flat_map(|h| h.join().unwrap_or_default()).collect()
        });
        let (w, h) = (nx, ny * 2);
        let mut data = vec![0u8; w * h * nz * 3 * 8];
        let mut put = |x: usize, y: usize, z: usize, c: Vec3| {
            let o = ((z * h + y) * w + x) * 8;
            for (k, v) in c.extend(1.0).to_array().iter().enumerate() {
                data[o + 2 * k..o + 2 * k + 2].copy_from_slice(&f16_bits(*v).to_le_bytes());
            }
        };
        for (k, c) in cubes.iter().enumerate() {
            let (x, y, z) = (k % nx, k / nx % ny, k / (nx * ny));
            for axis in 0..3 {
                put(x, y, z + axis * nz, c[2 * axis]);
                put(x, y + ny, z + axis * nz, c[2 * axis + 1]);
            }
        }
        data
    }
}

/// The probe volume: a box of voxels round the camera (`PROBE_VOXELS` of `PROBE_VOXEL_M`), each the probes' light
/// there as an ambient cube (Bevy's IrradianceVolume, for the meshes without a lightmap: characters, props, the
/// Blueprint parts), refilled by each solve where the camera then was. A voxel takes the probe set whose cell under it
/// is the smallest (sets of several levels overlap). Each voxel holds its L1 probe, and the vendored irradiance volume
/// shader reconstructs it per pixel as the game's movable base pass does (Enlighten's R0: M_Prop_Master s001).
/// PLACEHOLDER for the game's per-pixel probe interpolation's own volume (PPPI and its clipmap: here a box of 2 m
/// voxels round the camera, filled per solve).
#[derive(Component)]
struct ProbeVolume {
    image: Handle<Image>,
}

static SOLVE_LOGGED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

const PROBE_VOXELS: UVec3 = UVec3::new(48, 24, 48);
const PROBE_VOXEL_M: f32 = 2.0;

/// A half float's value (the atlas dump).
pub(crate) fn f16_value(h: u16) -> f32 {
    let (sign, exp, man) = ((h >> 15) as u32, ((h >> 10) & 0x1f) as i32, (h & 0x3ff) as f32);
    let v = match exp {
        0 => man / 1024.0 * 2f32.powi(-14),
        31 => f32::INFINITY,
        e => (1.0 + man / 1024.0) * 2f32.powi(e - 15),
    };
    if sign == 1 { -v } else { v }
}

pub(crate) fn f16_bits(v: f32) -> u16 {
    let x = v.to_bits();
    let sign = ((x >> 16) & 0x8000) as u16;
    let exp = ((x >> 23) & 0xff) as i32 - 127 + 15;
    let man = x & 0x7f_ffff;
    if v.is_nan() {
        return 0x7e00;
    }
    if exp >= 31 {
        sign | 0x7c00
    } else if exp <= 0 {
        if exp < -10 { sign } else { sign | (((man | 0x80_0000) >> (14 - exp)) as u16) }
    } else {
        sign | ((exp as u16) << 10) | ((man >> 13) as u16)
    }
}

fn atlas_of(atlases: &EnlightenAtlases, level: &str) -> Option<Handle<Image>> {
    atlases.0.get(level).cloned()
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
    instance_owners: Query<&EnlightenInstance>,
    landscapes: Query<&EnlightenLandscape>,
    transforms: bevy::transform::helper::TransformHelper,
    mut weight_uv: ResMut<LandscapeWeightUv>,
) {
    let Primitives { uvs: all_uvs, instances, zero_uv1, land, meshes: ue_meshes, counts, logged } = &mut *prims;
    let instance_lightmaps = std::env::var_os("F13_ENLIGHTEN_NO_INSTANCES").is_none();
    for (e, mesh3d) in &new {
        if let Some(owner) = parents.iter_ancestors(e).find_map(|a| landscapes.get(a).ok()) {
            // (A component's vertices, in Unreal space (cm): its cell by their centre.)
            let (Some(l), Some(atlas), Some(mesh), Ok(g)) = (land.get(&owner.level), atlas_of(&atlases, &owner.level), meshes.get(&mesh3d.0), transforms.compute_global_transform(e)) else {
                if meshes.get(&mesh3d.0).is_some() {
                    counts[2] += 1;
                    commands.entity(e).insert(Lightmapped);
                }
                continue;
            };
            let Some(bevy::mesh::VertexAttributeValues::Float32x3(pos)) = mesh.attribute(Mesh::ATTRIBUTE_POSITION) else {
                commands.entity(e).insert(Lightmapped);
                continue;
            };
            let world: Vec<Vec2> = pos.iter().map(|p| g.transform_point(Vec3::from(*p))).map(|b| Vec2::new(b.x, b.z) * 100.0).collect();
            let centre = world.iter().copied().sum::<Vec2>() / world.len().max(1) as f32;
            let cell = (((centre.x - l.origin) / l.size).floor() as i32, ((centre.y - l.origin) / l.size).floor() as i32);
            let Some(m) = l.cells.get(&cell) else {
                counts[2] += 1;
                commands.entity(e).insert(Lightmapped);
                continue;
            };
            // (Its weight maps' UV from UV0, before UV1 takes the lightmap's: the extremes' line, the map being affine.)
            if weight_uv.0.is_none() {
                if let (Some(bevy::mesh::VertexAttributeValues::Float32x2(u0)), Some(bevy::mesh::VertexAttributeValues::Float32x2(u1))) = (mesh.attribute(Mesh::ATTRIBUTE_UV_0), mesh.attribute(Mesh::ATTRIBUTE_UV_1)) {
                    let axis = |k: usize| {
                        let (lo, hi) = (0..u0.len()).fold((0, 0), |(lo, hi), i| (if u0[i][k] < u0[lo][k] { i } else { lo }, if u0[i][k] > u0[hi][k] { i } else { hi }));
                        let scale = (u1[hi][k] - u1[lo][k]) / (u0[hi][k] - u0[lo][k]).max(1e-6);
                        (scale, u1[lo][k] - scale * u0[lo][k])
                    };
                    let ((su, ou), (sv, ov)) = (axis(0), axis(1));
                    weight_uv.0 = Some(Vec4::new(su, sv, ou, ov));
                }
            }
            let uv1: Vec<[f32; 2]> = world.iter().map(|w| [m[0][0] * w.x + m[0][1] * w.y + m[0][2], m[1][0] * w.x + m[1][1] * w.y + m[1][2]]).collect();
            let lit = mesh.clone().with_inserted_attribute(Mesh::ATTRIBUTE_UV_1, uv1);
            counts[0] += 1;
            commands.entity(e).insert((Mesh3d(meshes.add(lit)), bevy::pbr::Lightmap { image: atlas, uv_rect: Rect::new(0.0, 0.0, 1.0, 1.0), bicubic_sampling: false }, Lightmapped));
            continue;
        }
        if let Some(owner) = parents.iter_ancestors(e).find_map(|a| instance_owners.get(a).ok()) {
            // (F13_ENLIGHTEN_NO_INSTANCES: the instances take the probe volume's light instead. Each instance's
            // Lightmap costs little since vendor/bevy_pbr shares a slot between the entities of one image: stock Bevy's
            // slot per entity made ~3,700 slabs, 11 ms a frame.)
            if !instance_lightmaps {
                commands.entity(e).insert(Lightmapped);
                continue;
            }
            let (Some(uvsb), Some(atlas)) = (instances.get(&(owner.level.clone(), owner.component.clone())).and_then(|v| v.get(owner.index)), atlases.0.get(&owner.level)) else {
                counts[2] += 1;
                commands.entity(e).insert(Lightmapped);
                continue;
            };
            let lit = match zero_uv1.get(&mesh3d.0.id()) {
                Some(h) => h.clone(),
                None => {
                    let Some(mesh) = meshes.get(&mesh3d.0).cloned() else { continue };
                    let n = mesh.count_vertices();
                    let h = meshes.add(mesh.with_inserted_attribute(Mesh::ATTRIBUTE_UV_1, vec![[0.0f32, 0.0]; n]));
                    zero_uv1.insert(mesh3d.0.id(), h.clone());
                    h
                }
            };
            // (Its UV: the stream (zero) x scale + bias: the rect from the bias.)
            let at = Vec2::new(uvsb[2], uvsb[3]);
            counts[0] += 1;
            commands.entity(e).insert((Mesh3d(lit), bevy::pbr::Lightmap { image: atlas.clone(), uv_rect: Rect::from_corners(at, at), bicubic_sampling: false }, Lightmapped));
            continue;
        }
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
