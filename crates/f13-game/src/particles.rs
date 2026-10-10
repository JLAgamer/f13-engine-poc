//! Cascade particle systems (UE 4.18 UParticleSystem, `particles/<name>.json`; f13-data particles.rs): the level's
//! ParticleSystemComponents that start active (bAutoActivate), each emitter's first LOD simulated on the CPU as
//! FParticleEmitterInstance does, drawn as camera-facing quads ([`ParticleMaterial`], shaders/particles.wgsl).
//!
//! The tick, per emitter (FParticleEmitterInstance::Tick): the emitter time (looping over EmitterDuration,
//! EmitterLoops times; 0 forever); dead particles (RelativeTime > 1) killed; each particle's velocity, size,
//! rotation rate and colour reset to their base values and RelativeTime advanced by OneOverMaxLifetime x dt; the
//! modules' updates in order; the spawn (Rate x RateScale at the emitter time, with the leftover fraction carried
//! and each new particle's spawn time within the tick; bursts as their times pass); then every particle moved by its
//! velocity and turned by its rotation rate (UpdateBoundingBox). Spawn: each module's spawn in order, then PostSpawn
//! moves it by its spawn time x velocity. Simulated in Unreal space (cm, Z up), drawn in Bevy's.
//!
//! Not run (logged once per system): attractors, collision, event generators and receivers, source movement, light
//! modules (their particle lights), spawn per unit, the GPU emitters' own simulation (TypeDataGpu emitters run on the
//! CPU like the others); sub-image blending (PSUVIM_Linear_Blend draws the current image). The phone box's smoke waits
//! for its destroyed state, the rain drips for rain (both start inactive or with their owner).

use bevy::asset::{embedded_asset, RenderAssetUsages};
use bevy::camera::visibility::NoFrustumCulling;
use bevy::light::{NotShadowCaster, NotShadowReceiver};
use bevy::mesh::{Indices, MeshVertexAttribute, MeshVertexBufferLayoutRef, PrimitiveTopology, VertexFormat};
use bevy::pbr::{Material, MaterialPipeline, MaterialPipelineKey};
use bevy::prelude::*;
use bevy::render::render_resource::{AsBindGroup, RenderPipelineDescriptor, ShaderType, SpecializedMeshPipelineError};
use bevy::shader::ShaderRef;
use f13_data::particles::{Emitter, Module, System};
use std::collections::HashMap;
use std::f32::consts::TAU;
use std::sync::Arc;

pub struct ParticlesPlugin;

impl Plugin for ParticlesPlugin {
    fn build(&self, app: &mut App) {
        embedded_asset!(app, "shaders/particles.wgsl");
        app.add_plugins(MaterialPlugin::<ParticleMaterial>::default())
            .add_systems(Startup, spawn_level_particles)
            .add_systems(PostUpdate, (simulate, draw).chain().after(bevy::transform::TransformSystems::Propagate));
    }
}

/// The particle's dynamic parameter (Float32x4), for the material's DynamicParameter.
const ATTRIBUTE_DYNAMIC: MeshVertexAttribute = MeshVertexAttribute::new("ParticleDynamic", 0x5f13_0001, VertexFormat::Float32x4);

#[derive(Clone, Copy, ShaderType, Default)]
struct ParticleParams {
    mode: Vec4,
    p: [Vec4; 12],
}

/// A particle material: the master's port (`mode`), its uniform buffer as the compiled shader lays it out, and its
/// textures in the shader's order.
#[derive(Asset, TypePath, AsBindGroup, Clone)]
pub struct ParticleMaterial {
    #[uniform(0)]
    params: ParticleParams,
    #[texture(1)]
    #[sampler(2)]
    t0: Handle<Image>,
    #[texture(3)]
    t1: Handle<Image>,
    #[texture(4)]
    t2: Handle<Image>,
    alpha: AlphaMode,
}

impl Material for ParticleMaterial {
    fn vertex_shader() -> ShaderRef {
        "embedded://f13_game/shaders/particles.wgsl".into()
    }
    fn fragment_shader() -> ShaderRef {
        "embedded://f13_game/shaders/particles.wgsl".into()
    }
    fn alpha_mode(&self) -> AlphaMode {
        self.alpha
    }
    fn enable_prepass() -> bool {
        false
    }
    fn enable_shadows() -> bool {
        false
    }
    fn specialize(_: &MaterialPipeline, descriptor: &mut RenderPipelineDescriptor, layout: &MeshVertexBufferLayoutRef, _: MaterialPipelineKey<Self>) -> Result<(), SpecializedMeshPipelineError> {
        let l = layout.0.get_layout(&[
            Mesh::ATTRIBUTE_POSITION.at_shader_location(0),
            Mesh::ATTRIBUTE_UV_0.at_shader_location(1),
            Mesh::ATTRIBUTE_COLOR.at_shader_location(2),
            ATTRIBUTE_DYNAMIC.at_shader_location(3),
        ])?;
        descriptor.vertex.buffers = vec![l];
        // Sprites have no back (velocity-aligned ones turn either way).
        descriptor.primitive.cull_mode = None;
        Ok(())
    }
}

/// One particle (FBaseParticle and the payloads f13-game keeps), Unreal space.
#[derive(Clone, Default)]
struct Particle {
    location: Vec3,
    velocity: Vec3,
    base_velocity: Vec3,
    size: Vec3,
    base_size: Vec3,
    /// Radians.
    rotation: f32,
    rotation_rate: f32,
    base_rotation_rate: f32,
    color: Vec4,
    base_color: Vec4,
    relative_time: f32,
    one_over_max_lifetime: f32,
    sub_image: f32,
    dynamic: [f32; 4],
    /// The random values its dynamic parameters drew (each re-evaluated over its life with the same one).
    dynamic_random: [f32; 4],
    orbit_offset: Vec3,
    orbit_rotation: Vec3,
    orbit_rate: Vec3,
}

/// A running emitter.
struct EmitterState {
    def: Emitter,
    seconds: f32,
    spawn_fraction: f32,
    particles: Vec<Particle>,
    entity: Entity,
    mesh: Handle<Mesh>,
    pivot: Vec2,
    /// Its material reads the particle's sphere (centre and radius, Bevy space) from the dynamic attribute (the flies'
    /// sphere normals; their material has no DynamicParameter).
    sphere: bool,
    /// The bursts fired in the current loop.
    bursts_fired: Vec<bool>,
    loops_done: u32,
}

/// A placed ParticleSystemComponent.
#[derive(Component)]
pub struct ParticleSystemInstance {
    emitters: Vec<EmitterState>,
    /// The component's world transform, Unreal space (cm).
    location: Vec3,
    rotation: Quat,
    scale: Vec3,
    /// LDMaxDrawDistance x the view distance scale (m in Bevy); 0: any distance.
    max_draw: f32,
    rng: u64,
}

fn rand(state: &mut u64) -> f32 {
    // xorshift64*: the particles' own stream (the game's GameRng stays the AI's).
    let mut x = *state;
    x ^= x >> 12;
    x ^= x << 25;
    x ^= x >> 27;
    *state = x;
    ((x.wrapping_mul(0x2545_f491_4f6c_dd1d) >> 40) as f32) / (1u64 << 24) as f32
}

fn rand3(state: &mut u64) -> [f32; 3] {
    [rand(state), rand(state), rand(state)]
}

/// Unreal space (cm, Z up) to Bevy's (m, Y up).
fn to_bevy(v: Vec3) -> Vec3 {
    Vec3::new(v.x, v.z, v.y) * 0.01
}

/// Loaded systems by name.
#[derive(Default)]
struct Systems(HashMap<String, Option<Arc<System>>>);

impl Systems {
    fn get(&mut self, root: &std::path::Path, name: &str) -> Option<Arc<System>> {
        self.0
            .entry(name.to_string())
            .or_insert_with(|| {
                let s = System::load(&root.join("particles").join(format!("{name}.json"))).map(Arc::new);
                if let Some(s) = &s {
                    let unported: std::collections::BTreeSet<&str> = s.emitters.iter().flat_map(|e| &e.modules).filter_map(|m| if let Module::Unported(t) = m { Some(t.as_str()) } else { None }).collect();
                    if !unported.is_empty() {
                        info!("particles: {name}: modules not run: {}", unported.into_iter().collect::<Vec<_>>().join(", "));
                    }
                } else {
                    warn!("particles: no particles/{name}.json (run f13-export particles)");
                }
                s
            })
            .clone()
    }
}

/// One exported material's entry (materials.json).
#[derive(serde::Deserialize, Clone)]
#[serde(rename_all = "PascalCase")]
struct MaterialEntry {
    blend: String,
    #[serde(default)]
    master: String,
    #[serde(default)]
    scalars: HashMap<String, f32>,
    #[serde(default)]
    vectors: HashMap<String, [f32; 4]>,
    #[serde(default)]
    textures: HashMap<String, String>,
}

/// Builds the particle materials by name.
struct Materials {
    entries: HashMap<String, MaterialEntry>,
    /// Each material and its master's port (`ParticleParams::mode`).
    built: HashMap<String, Option<(Handle<ParticleMaterial>, u32)>>,
}

impl Materials {
    fn get(&mut self, name: &str, root: &std::path::Path, assets: &AssetServer, materials: &mut Assets<ParticleMaterial>) -> Option<(Handle<ParticleMaterial>, u32)> {
        if let Some(h) = self.built.get(name) {
            return h.clone();
        }
        let built = self.build(name, root, assets).map(|m| {
            let mode = m.params.mode.x as u32;
            (materials.add(m), mode)
        });
        self.built.insert(name.to_string(), built.clone());
        built
    }

    fn build(&self, name: &str, root: &std::path::Path, assets: &AssetServer) -> Option<ParticleMaterial> {
        // The engine's default particle (the turbulence attractors' emitters): drawn nowhere here. PLACEHOLDER: those
        // emitters only move the others in the game (their attractors aren't run here either).
        if name == "DefaultParticle" {
            return None;
        }
        let e = self.entries.get(name).or_else(|| self.entries.iter().find(|(k, _)| k.eq_ignore_ascii_case(name)).map(|(_, v)| v))?;
        let master = self.entries.get(&e.master);
        // A scalar: the instance's, else its master's, else the default the compiled shader was built with.
        let s = |n: &str, d: f32| e.scalars.get(n).or_else(|| master.and_then(|m| m.scalars.get(n))).copied().unwrap_or(d);
        // Flow and distortion maps are data (linear), the rest colour.
        let tex = |n: &str| {
            let color = !n.contains("Flow") && !n.contains("Distortion") && !n.contains("Normal");
            e.textures.get(n).or_else(|| master.and_then(|m| m.textures.get(n))).and_then(|p| crate::materials::load_texture(root, assets, p, color))
        };
        let mut p = [Vec4::ZERO; 12];
        let (mode, t) = match e.master.as_str() {
            "M_Procedural_Fire_01" => {
                // uniexpr.py M_Procedural_Fire_01: cb1[1] SelectionColor.rgb (0), [2..9] below.
                let hard = s("SphereMask_Hardness", 0.0);
                let soft = (1.0 - hard).max(1e-5);
                p[2] = Vec4::new(s("blackBodyTemp", 3100.0), s("0_FlowMap_Multiplier", 1.0), s("0_Flowmap1_XY_Time_Multiply", 0.5), s("Flowmap1_UV_MASTER_Multiply", 0.75));
                p[3] = Vec4::new(s("Flowmaps_BasemapTileMultiply", 0.25), s("Basemap1_X_Speed_Multiply", 0.1), s("Basemap1_U_Coord_Multiply", 1.0), s("Basemap1_U_Coord_Multiply", 1.0));
                p[4] = Vec4::new(s("Basemap1_Y_Speed_Multiply", 1.0), s("Basemap1_V_Coord_Multiply", 1.0), s("Basemap1_UV_MASTER_Multiply", 3.0), s("0_Flowmap2_XY_Time_Multiply", -0.6));
                p[5] = Vec4::new(s("Flowmap2_UV_MASTER_Multiply", 0.65), s("Basemap2_X_Speed_Multiply", -0.15), s("Basemap2_U_Coord_Multiply", 1.3), s("Basemap2_U_Coord_Multiply", 1.3));
                p[6] = Vec4::new(s("Basemap2_Y_Speed_Multiply", 3.0), s("Basemap2_V_Coord_Multiply", 1.0), s("Basemap2_UV_MASTER_Multiply", 7.0), s("0_EmissiveMasterLevel", 3.0));
                p[7] = Vec4::new(s("0_FadeDistance", 5.0), s("0_FadeDistance", 5.0).max(1e-5), s("0_PixelDepthDistance", 0.0), s("0_Opacity_MASTER", 1.0));
                p[8] = Vec4::new(s("SphereMaskGlobalDistortionLevel", 1.0), s("SphereMask_Radius", 0.5), hard, 1.0 - hard);
                p[9] = Vec4::new(soft, 1.0 / soft, s("0_Alpha_Contrast_Opacity_MASTER", 6.0), s("RefractionDepthBias", 0.0));
                (1.0, [tex("FlowMap1"), tex("BaseTexture1"), tex("BaseTexture2")])
            }
            "M_Procedural_Fire_02" => {
                // uniexpr.py M_Procedural_Fire_02: cb1[3].x = 0 - 0_AlphaContrast, [4..9] below.
                let contrast = s("0_AlphaContrast", 0.0);
                p[3] = Vec4::new(-contrast, -contrast, -contrast, 0.0);
                p[4] = Vec4::new(s("0_MasterScrollMultiplier", 1.0), s("Distortion1_Scroll_Multiplier", 0.05), s("Distortion1_UV_Coord_Multiplier", 0.2), s("DistortionTexturePower", 5.0));
                p[5] = Vec4::new(s("Distortion2_Scroll_Multiplier", 0.08), s("Distortion2_UV_Coord_Multiplier", 0.3), s("0_DistortionMultiplier", 1.0), s("DistortionMax", 1.0));
                p[6] = Vec4::new(s("DistortionMin", 0.0), s("Diffuse_Max", 1.0), s("Diffuse_Min", 0.0), s("Emissive_Multiplier", 4.0));
                p[7] = Vec4::new(contrast, -contrast, s("0_MasterRotationMultiplier", 0.5), s("Texture3_Scroll_Multiplier", 0.5));
                p[8] = Vec4::new(s("Texture3_UV_Coord_Multiplier", 0.1), s("Texture3_Max", 1.0), s("Texture3_Min", 0.5), s("FadeDistance", 3.0));
                p[9] = Vec4::new(s("FadeDistance", 3.0).max(1e-5), s("0_PixelDepthDistance", 0.0), s("0_Opacity_Multiplier", 1.0), contrast + 1.0);
                // t0 the flame (Base_ALPHA_Texture1), t1 the smoke tile its opacity turns (Base_ALPHA_Texture3):
                // by how the compiled shader samples them (t0's colour and alpha, t1's red).
                (2.0, [tex("Base_ALPHA_Texture1"), tex("Base_ALPHA_Texture3"), None])
            }
            "M_MasterParticle_Emissive" => {
                // uniexpr.py M_MasterParticle_Emissive: cb1[2] the vector parameter (named "None"; "" in the export),
                // cb1[3] SelectionColor.rgb (0), [4..6] below.
                let v = e.vectors.get("").or_else(|| master.and_then(|m| m.vectors.get(""))).copied().unwrap_or([1.0; 4]);
                p[2] = Vec4::new(v[0], v[1], v[2], 0.0);
                p[4] = Vec4::new(s("EmissiveLevel", 1.0), s("TimeMultiplier", 1.0), s("Texture3_Scroll_Multiplier", 0.5), s("Texture3_UV_Coord_Multiplier", 1.0));
                p[5] = Vec4::new(s("TimeMultiplier_2", 0.83), s("Texture3_Scroll_Multiplier_2", -0.4), s("Texture3_UV_Coord_Multiplier_2", 1.0), s("fadeAmount", 0.0));
                p[6] = Vec4::new(s("fadeAmount", 0.0).max(1e-5), s("Opacity_Power", 2.0), s("RefractionDepthBias", 0.0), 0.0);
                (3.0, [tex("ParticleTexture"), tex("Texture1"), tex("Texture1_1")])
            }
            "M_Fly_Animated_01" => {
                // uniexpr.py M_Fly_Animated_01: cb1[1] (UV_COLUMNS, UV_ROWS), [2] (columns x rows, UV_ROWS), [5]
                // (FLIPBOOK_Speed, ScaleByDistance, MaximumDistanceSize, UV_ROWS), [6] (UV_COLUMNS, columns x rows,
                // FLIPBOOK_Speed: the instance's one value for both of the graph's parameters of that name).
                let (cols, rows) = (s("UV_COLUMNS", 2.0), s("UV_ROWS", 2.0));
                let speed = s("FLIPBOOK_Speed", 30.0);
                p[1] = Vec4::new(cols, rows, cols, rows);
                p[2] = Vec4::new(cols * rows, rows, rows, rows);
                p[5] = Vec4::new(speed, s("ScaleByDistance", 700.0), s("MaximumDistanceSize", 0.5), rows);
                p[6] = Vec4::new(cols, cols * rows, speed, 0.0);
                (4.0, [tex("Normal1"), tex("Texture1"), None])
            }
            "M_MasterParticle_Diffuse_nearFade" => {
                // uniexpr.py M_MasterParticle_Diffuse_nearFade: cb1[2] SelectionColor.rgb + 0, [3] (fadeDistance,
                // max(1e-5, fadeDistance), 1 / that, fadeAmount), [4] (max(fadeAmount, 1e-5), RefractionDepthBias).
                let near = s("fadeDistance", 200.0).max(1e-5);
                let amount = s("fadeAmount", 0.0);
                p[3] = Vec4::new(s("fadeDistance", 200.0), near, 1.0 / near, amount);
                p[4] = Vec4::new(amount.max(1e-5), s("RefractionDepthBias", 0.0), 0.0, 0.0);
                (5.0, [tex("ParticleTexture"), None, None])
            }
            // Masters whose graphs aren't ported yet aren't drawn (F13_PARTICLE_STANDINS=1 draws them as their first
            // texture x the particle colour, PLACEHOLDER, to check the simulation).
            _ if std::env::var("F13_PARTICLE_STANDINS").is_ok() => {
                let first = ["ParticleTexture", "Texture", "BaseTexture1", "Base_ALPHA_Texture1"].iter().find_map(|n| tex(n)).or_else(|| {
                    let mut names: Vec<&String> = e.textures.keys().collect();
                    names.sort();
                    names.first().and_then(|n| tex(n))
                });
                (0.0, [first, None, None])
            }
            _ => {
                info!("particles: {name} ({}): its master isn't ported, not drawn", e.master);
                return None;
            }
        };
        let additive = e.blend == "BLEND_Additive";
        let alpha = match e.blend.as_str() {
            "BLEND_Additive" => AlphaMode::Add,
            "BLEND_Masked" => AlphaMode::Mask(0.3333),
            _ => AlphaMode::Blend,
        };
        let [t0, t1, t2] = t;
        let t0 = t0?;
        Some(ParticleMaterial {
            params: ParticleParams { mode: Vec4::new(mode, if additive { 1.0 } else { 0.0 }, 0.0, 0.0), p },
            t1: t1.unwrap_or_else(|| t0.clone()),
            t2: t2.unwrap_or_else(|| t0.clone()),
            t0,
            alpha,
        })
    }
}

/// Spawns the level's active particle systems.
#[allow(clippy::too_many_arguments)]
fn spawn_level_particles(
    mut commands: Commands,
    level: Option<Res<crate::Level>>,
    export: Res<crate::ExportRoot>,
    assets: Res<AssetServer>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<ParticleMaterial>>,
) {
    let Some(level) = level else { return };
    let root = export.0.clone();
    let entries: HashMap<String, MaterialEntry> = std::fs::read_to_string(root.join("materials.json")).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default();
    let mut mats = Materials { entries, built: HashMap::new() };
    let mut systems = Systems::default();
    let mut n = 0;
    let mut seed = 0x9e37_79b9_7f4a_7c15u64;
    for layout in &level.layouts {
        for a in &layout.actors {
            for c in &a.components {
                if c.kind != "ParticleSystemComponent" || c.hidden {
                    continue;
                }
                let p = &c.props;
                if p["bAutoActivate"].as_bool() == Some(false) || p["bVisible"].as_bool() == Some(false) || p["bHiddenInGame"].as_bool() == Some(true) {
                    continue;
                }
                // The phone box's smoke shows once it's destroyed (police.rs); not yet.
                if a.class.starts_with("PhoneJunctionBox") {
                    continue;
                }
                let Some(name) = p["Template"].as_str().and_then(|t| t.split('\'').nth(1)) else { continue };
                let Some(world) = &c.world else { continue };
                let Some(system) = systems.get(&root, name) else { continue };
                seed = seed.wrapping_add(0x9e37_79b9_7f4a_7c15);
                let location = Vec3::from(world.t);
                let rotation = Quat::from_array(world.r).normalize();
                let scale = Vec3::from(world.s);
                let max_draw = p["LDMaxDrawDistance"].as_f64().unwrap_or(0.0) as f32 * crate::VIEW_DISTANCE_SCALE / 100.0;
                let warmup = p["WarmupTime"].as_f64().map_or(system.warmup, |w| w as f32);
                let mut emitters = vec![];
                for def in &system.emitters {
                    let Some((material, mode)) = def.material.as_deref().and_then(|m| mats.get(m, &root, &assets, &mut materials)) else { continue };
                    // Every attribute from the start (a degenerate quad), so its layout never changes.
                    let mesh = meshes.add(
                        Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::default())
                            .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, vec![[0.0f32; 3]; 4])
                            .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, vec![[0.0f32; 2]; 4])
                            .with_inserted_attribute(Mesh::ATTRIBUTE_COLOR, vec![[0.0f32; 4]; 4])
                            .with_inserted_attribute(ATTRIBUTE_DYNAMIC, vec![[0.0f32; 4]; 4])
                            .with_inserted_indices(Indices::U32(vec![0, 1, 2, 0, 2, 3])),
                    );
                    let entity = commands
                        .spawn((
                            Mesh3d(mesh.clone()),
                            MeshMaterial3d(material),
                            Transform::from_translation(to_bevy(location)),
                            Visibility::Hidden,
                            NoFrustumCulling,
                            NotShadowCaster,
                            NotShadowReceiver,
                            Name::new(format!("{name}.{}", def.name)),
                        ))
                        .id();
                    let pivot = def.modules.iter().find_map(|m| if let Module::PivotOffset(p) = m { Some(Vec2::from(*p)) } else { None }).unwrap_or(Vec2::ZERO) + Vec2::splat(-0.5);
                    emitters.push(EmitterState { def: def.clone(), seconds: 0.0, spawn_fraction: 0.0, particles: vec![], entity, mesh, pivot, bursts_fired: vec![false; def.bursts.len()], loops_done: 0, sphere: mode == 4 });
                }
                if emitters.is_empty() {
                    continue;
                }
                let mut inst = ParticleSystemInstance { emitters, location, rotation, scale, max_draw, rng: seed };
                // WarmupTime: ticked in 0.032 s steps (UParticleSystemComponent's default warmup step).
                let mut t = 0.0;
                while t < warmup {
                    tick(&mut inst, 0.032);
                    t += 0.032;
                }
                commands.spawn((inst, Name::new(format!("particles {name}"))));
                n += 1;
            }
        }
    }
    info!("particles: {n} systems");
}

/// Ticks every system (game time; on during cinematics too, as the engine's).
fn simulate(time: Res<Time<Real>>, virtual_time: Res<Time<Virtual>>, mut systems: Query<&mut ParticleSystemInstance>) {
    let dt = (time.delta_secs() * virtual_time.relative_speed()).min(0.1);
    if dt <= 0.0 {
        return;
    }
    for mut s in &mut systems {
        tick(&mut s, dt);
    }
}

fn tick(s: &mut ParticleSystemInstance, dt: f32) {
    let (location, rotation, scale) = (s.location, s.rotation, s.scale);
    let mut rng = s.rng;
    for e in &mut s.emitters {
        tick_emitter(e, dt, location, rotation, scale, &mut rng);
    }
    s.rng = rng;
}

fn tick_emitter(e: &mut EmitterState, dt: f32, location: Vec3, rotation: Quat, scale: Vec3, rng: &mut u64) {
    let def = &e.def;
    // Tick_EmitterTimeSetup.
    let old_seconds = e.seconds;
    e.seconds += dt;
    let duration = def.duration.max(0.0);
    let mut emitter_time = e.seconds;
    let _ = old_seconds;
    if duration > 0.0 {
        let loops = (e.seconds / duration).floor() as u32;
        if loops > e.loops_done {
            e.loops_done = loops;
            e.bursts_fired.iter_mut().for_each(|b| *b = false);
        }
        emitter_time = e.seconds - loops as f32 * duration;
    }
    let finished = def.loops > 0 && duration > 0.0 && e.loops_done >= def.loops;
    let delay = if def.delay_first_loop_only && e.loops_done > 0 { 0.0 } else { def.delay };
    emitter_time -= delay;
    // KillParticles.
    e.particles.retain(|p| p.relative_time <= 1.0);
    // ResetParticleParameters.
    for p in &mut e.particles {
        p.velocity = p.base_velocity;
        p.size = p.base_size;
        p.rotation_rate = p.base_rotation_rate;
        p.color = p.base_color;
        p.relative_time += p.one_over_max_lifetime * dt;
    }
    // The modules' updates.
    for m in &def.modules {
        for p in &mut e.particles {
            update(m, p, dt, emitter_time);
        }
    }
    // Spawn.
    if !finished && emitter_time >= 0.0 {
        let rate = def.rate.eval(emitter_time, rand(rng)) * def.rate_scale.eval(emitter_time, rand(rng)).max(0.0);
        let mut count = 0;
        let mut increment = 0.0;
        let mut start = 0.0;
        if rate > 0.0 {
            let leftover = e.spawn_fraction + dt * rate;
            count = leftover.floor() as u32;
            increment = 1.0 / rate;
            start = dt + e.spawn_fraction * increment - increment;
            e.spawn_fraction = leftover - count as f32;
        }
        // Bursts: their Time is a fraction of the emitter's duration, each once a loop.
        let mut burst = 0u32;
        for (i, b) in def.bursts.iter().enumerate() {
            if !e.bursts_fired[i] && b.time * duration <= emitter_time {
                e.bursts_fired[i] = true;
                let n = if b.count_low >= 0 { b.count_low as f32 + (b.count as f32 - b.count_low as f32) * rand(rng) } else { b.count as f32 };
                burst += (n * def.burst_scale.eval(emitter_time, rand(rng))).round().max(0.0) as u32;
            }
        }
        for i in 0..count + burst {
            let spawn_time = if i < count { start - i as f32 * increment } else { 0.0 };
            let p = spawn(def, spawn_time.max(0.0), emitter_time, location, rotation, scale, rng);
            e.particles.push(p);
        }
    }
    // UpdateBoundingBox: the linear integrator.
    for p in &mut e.particles {
        p.location += p.velocity * dt;
        p.rotation += p.rotation_rate * dt;
    }
}

/// One module's per-tick update.
fn update(m: &Module, p: &mut Particle, dt: f32, emitter_time: f32) {
    let t = p.relative_time;
    match m {
        Module::ColorOverLife { color, alpha } => {
            let c = color.eval3(t, [0.0; 3]);
            p.color = c.extend(alpha.eval(t, 0.0));
        }
        Module::SizeMultiplyLife { life, axes } => {
            let s = life.eval3(t, [0.0; 3]);
            for i in 0..3 {
                if axes[i] {
                    p.size[i] *= s[i];
                }
            }
        }
        Module::SizeScaleBySpeed { speed, max } => {
            let v = p.velocity.length();
            p.size.x *= (v * speed[0]).clamp(1.0, max[0].max(1.0));
            p.size.y *= (v * speed[1]).clamp(1.0, max[1].max(1.0));
        }
        Module::RotationRateMultiplyLife(d) => p.rotation_rate *= d.eval(t, 0.0),
        Module::AccelerationConstant { accel, .. } => {
            p.velocity += *accel * dt;
            p.base_velocity += *accel * dt;
        }
        Module::Drag(d) => {
            let drag = p.velocity * -d.eval(t, 0.0) * dt;
            p.velocity += drag;
            p.base_velocity += drag;
        }
        Module::SubUV(d) => p.sub_image = d.eval(t, 0.0),
        Module::Dynamic(params) => {
            for (i, d) in params.iter().enumerate() {
                if let Some((dist, spawn_only, emitter)) = d {
                    if !spawn_only {
                        p.dynamic[i] = dist.eval(if *emitter { emitter_time } else { t }, p.dynamic_random[i]);
                    }
                }
            }
        }
        Module::Orbit { .. } => p.orbit_rotation += p.orbit_rate * dt,
        _ => {}
    }
}

/// A new particle: PreSpawn, each module's spawn, PostSpawn.
fn spawn(def: &Emitter, spawn_time: f32, emitter_time: f32, location: Vec3, rotation: Quat, scale: Vec3, rng: &mut u64) -> Particle {
    let mut p = Particle { color: Vec4::ONE, base_color: Vec4::ONE, ..default() };
    // PreSpawn: the component's location (world-space emitters) and the emitter origin.
    p.location = location + rotation * (scale * def.origin);
    let to_world = |v: Vec3| rotation * (scale * v);
    for m in &def.modules {
        match m {
            Module::Lifetime(d) => {
                let life = d.eval(emitter_time, rand(rng));
                p.one_over_max_lifetime = if life > 0.0 { 1.0 / life } else { 0.0 };
                p.relative_time = spawn_time * p.one_over_max_lifetime;
            }
            Module::Location(d) => p.location += to_world(d.eval3(emitter_time, rand3(rng))),
            Module::LocationSphere { radius, velocity, velocity_scale, origin } => {
                // A random direction within the sphere (both hemispheres on every axis).
                let dir = loop {
                    let v = Vec3::new(rand(rng) * 2.0 - 1.0, rand(rng) * 2.0 - 1.0, rand(rng) * 2.0 - 1.0);
                    if v.length_squared() > 1e-4 && v.length_squared() <= 1.0 {
                        break v.normalize();
                    }
                };
                let offset = dir * radius.eval(emitter_time, rand(rng)) * rand(rng) + origin.eval3(emitter_time, rand3(rng));
                p.location += to_world(offset);
                if *velocity {
                    let v = to_world(offset * velocity_scale.eval(emitter_time, rand(rng)));
                    p.velocity += v;
                    p.base_velocity += v;
                }
            }
            Module::Velocity { start, radial, world } => {
                let mut v = start.eval3(emitter_time, rand3(rng));
                if !*world {
                    v = to_world(v);
                }
                let from_origin = (p.location - location).normalize_or_zero();
                v += from_origin * radial.eval(emitter_time, rand(rng));
                p.velocity += v;
                p.base_velocity += v;
            }
            Module::Size(d) => {
                let s = d.eval3(emitter_time, rand3(rng));
                p.size += s;
                p.base_size += s;
            }
            Module::Rotation(d) => p.rotation += TAU * d.eval(emitter_time, rand(rng)),
            Module::RotationRate(d) => {
                let r = TAU * d.eval(emitter_time, rand(rng));
                p.rotation_rate += r;
                p.base_rotation_rate += r;
            }
            Module::AccelerationConstant { accel, .. } => {
                p.velocity += *accel * spawn_time;
                p.base_velocity += *accel * spawn_time;
            }
            Module::Orbit { offset, rotation: rot, rate, .. } => {
                p.orbit_offset = offset.eval3(emitter_time, rand3(rng));
                p.orbit_rotation = rot.eval3(emitter_time, rand3(rng));
                p.orbit_rate = rate.eval3(emitter_time, rand3(rng));
            }
            Module::Dynamic(params) => {
                for (i, d) in params.iter().enumerate() {
                    if let Some((dist, _, emitter)) = d {
                        p.dynamic_random[i] = rand(rng);
                        p.dynamic[i] = dist.eval(if *emitter { emitter_time } else { p.relative_time }, p.dynamic_random[i]);
                    }
                }
            }
            _ => update(m, &mut p, 0.0, emitter_time),
        }
    }
    // PostSpawn.
    p.location += spawn_time * p.velocity;
    p
}

/// The emitters' quads, facing the camera.
fn draw(
    camera: Query<&GlobalTransform, (With<Camera3d>, With<crate::lighting::Lit>)>,
    systems: Query<&ParticleSystemInstance>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut vis: Query<&mut Visibility>,
) {
    let Some(cam) = camera.iter().next() else { return };
    let cam_pos = cam.translation();
    let (cam_right, cam_up, cam_fwd) = (cam.right().as_vec3(), cam.up().as_vec3(), cam.forward().as_vec3());
    for s in &systems {
        let origin = to_bevy(s.location);
        let near = s.max_draw <= 0.0 || origin.distance(cam_pos) <= s.max_draw;
        for e in &s.emitters {
            let shown = near && !e.particles.is_empty();
            if let Ok(mut v) = vis.get_mut(e.entity) {
                v.set_if_neq(if shown { Visibility::Inherited } else { Visibility::Hidden });
            }
            if !shown {
                continue;
            }
            let def = &e.def;
            // MaxDrawCount: the first so many.
            let mut order: Vec<usize> = (0..e.particles.len().min(def.max_draw_count.map_or(usize::MAX, |m| m as usize))).collect();
            // Translucent sprites back to front (PSORTMODE_ViewProjDepth / DistanceToView).
            order.sort_by(|&a, &b| {
                let da = (to_bevy(e.particles[a].location) - cam_pos).dot(cam_fwd);
                let db = (to_bevy(e.particles[b].location) - cam_pos).dot(cam_fwd);
                db.total_cmp(&da)
            });
            let n = order.len();
            let mut positions = Vec::with_capacity(n * 4);
            let mut uvs = Vec::with_capacity(n * 4);
            let mut colors = Vec::with_capacity(n * 4);
            let mut dynamic = Vec::with_capacity(n * 4);
            let mut indices = Vec::with_capacity(n * 6);
            let [cols, rows] = [def.sub_images[0].max(1), def.sub_images[1].max(1)];
            let lock_z = def.modules.iter().any(|m| matches!(m, Module::AxisLock(a) if a == "EPAL_ROTATE_Z"));
            for i in order {
                let p = &e.particles[i];
                let orbit = if p.orbit_offset != Vec3::ZERO {
                    let r = p.orbit_rotation * TAU;
                    Quat::from_euler(EulerRot::ZYX, r.z, r.y, r.x) * p.orbit_offset
                } else {
                    Vec3::ZERO
                };
                let center = to_bevy(p.location + orbit);
                let size = Vec2::new(p.size.x * s.scale.x, p.size.y * s.scale.y) * 0.01;
                let (right, up) = match def.screen_alignment.as_str() {
                    "PSA_Velocity" => {
                        let v = to_bevy(p.velocity).normalize_or(cam_up);
                        let r = v.cross(cam_pos - center).normalize_or(cam_right);
                        (r, v)
                    }
                    _ if lock_z => {
                        // EPAL_ROTATE_Z: upright, turned about the world's up to face the camera.
                        let up = Vec3::Y;
                        let r = (cam_pos - center).cross(up).normalize_or(cam_right);
                        (r, up)
                    }
                    _ => (cam_right, cam_up),
                };
                let (sin, cos) = p.rotation.sin_cos();
                let (r, u) = (right * cos + up * sin, up * cos - right * sin);
                let image = p.sub_image.max(0.0).floor() as u32 % (cols * rows);
                let cell = Vec2::new((image % cols) as f32, (image / cols) as f32);
                let base = positions.len() as u32;
                for corner in [Vec2::new(0.0, 0.0), Vec2::new(1.0, 0.0), Vec2::new(1.0, 1.0), Vec2::new(0.0, 1.0)] {
                    let o = corner + e.pivot;
                    let pos = center + r * (o.x * size.x) - u * (o.y * size.y);
                    positions.push((pos - origin).to_array());
                    uvs.push(((cell + corner) / Vec2::new(cols as f32, rows as f32)).to_array());
                    colors.push(p.color.to_array());
                    // ParticlePositionAndSize: the centre and half the larger side.
                    dynamic.push(if e.sphere { center.extend(0.5 * size.max_element()).to_array() } else { p.dynamic });
                }
                indices.extend_from_slice(&[base, base + 2, base + 1, base, base + 3, base + 2]);
            }
            if let Some(mut mesh) = meshes.get_mut(&e.mesh) {
                mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
                mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, uvs);
                mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, colors);
                mesh.insert_attribute(ATTRIBUTE_DYNAMIC, dynamic);
                mesh.insert_indices(Indices::U32(indices));
            }
        }
    }
}
