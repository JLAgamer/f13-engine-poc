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
//! CPU like the others); sub-image blending (PSUVIM_Linear_Blend draws the current image).
//!
//! Systems that start inactive (bAutoActivate off: the rain drips, the fuse boxes' sparks) wait for a
//! [`ParticleActivation`] naming their actor: UParticleSystemComponent::Activate(true) resets one, Deactivate stops its
//! spawning and lets its particles finish (the phone box's smoke: police.rs, PhoneJunctionBox_BP's
//! OnPhoneBoxDestroyed / OnPhoneBoxRepaired).

use bevy::asset::{embedded_asset, RenderAssetUsages};
use bevy::camera::visibility::NoFrustumCulling;
use bevy::light::{NotShadowCaster, NotShadowReceiver};
use bevy::mesh::{Indices, MeshVertexAttribute, MeshVertexBufferLayoutRef, PrimitiveTopology, VertexFormat};
use bevy::pbr::{ExtendedMaterial, Material, MaterialExtension, MaterialPipeline, MaterialPipelineKey};
use bevy::prelude::*;
use bevy::render::render_resource::{AsBindGroup, RenderPipelineDescriptor, ShaderType, SpecializedMeshPipelineError};
use bevy::render::storage::ShaderBuffer;
use bevy::shader::ShaderRef;
use f13_data::particles::{Dist, Emitter, Module, System};
use std::collections::HashMap;
use std::f32::consts::TAU;
use std::sync::Arc;

pub struct ParticlesPlugin;

impl Plugin for ParticlesPlugin {
    fn build(&self, app: &mut App) {
        embedded_asset!(app, "shaders/particles.wgsl");
        embedded_asset!(app, "shaders/mesh_particles.wgsl");
        embedded_asset!(app, "shaders/mesh_particles_prepass.wgsl");
        app.add_plugins((MaterialPlugin::<ParticleMaterial>::default(), MaterialPlugin::<MeshParticleMaterial>::default()))
            .add_message::<ParticleActivation>()
            .add_message::<StopMeshParticles>()
            .add_message::<SpawnParticles>()
            .add_systems(Startup, (no_sprites, spawn_level_particles))
            .add_systems(Update, test_particles)
            .add_systems(PostUpdate, (rain_drips, rain_grid, rain_mpc_materials, spawn_requested, activate, stop_mesh_particles, follow_and_finish, simulate, draw).chain().after(bevy::transform::TransformSystems::Propagate));
    }
}

/// Activates (Activate(true): reset) or deactivates (Deactivate: no more spawning) the systems on a level actor.
#[derive(Message, Clone)]
pub struct ParticleActivation {
    pub actor: String,
    pub active: bool,
    /// Activate(bReset): from the start. Without it (SetActive(true, false)) an inactive system's spawning resumes
    /// and its particles are kept (PLACEHOLDER: UParticleSystemComponent::ActivateSystem's bReset false path unread).
    pub reset: bool,
}

/// A mesh emitter's override material, ported: M_Firecracker_Debris_01 (the grass impact's flying chunks). The
/// particle's colour and dynamic parameters (the mesh-particle vertex factory's instance attributes: Color, its
/// DynamicParameter) go into the StandardMaterial per particle; the extension's shaders add the mask's dither and clip
/// (shaders/mesh_particles.wgsl, its prepass).
pub(crate) type MeshParticleMaterial = ExtendedMaterial<StandardMaterial, MeshParticleExt>;

#[derive(Asset, AsBindGroup, Reflect, Debug, Clone)]
pub(crate) struct MeshParticleExt {
    /// x: the opacity mask's clip value.
    #[uniform(100)]
    clip: Vec4,
}

impl MaterialExtension for MeshParticleExt {
    fn fragment_shader() -> ShaderRef {
        "embedded://f13_game/shaders/mesh_particles.wgsl".into()
    }

    fn prepass_fragment_shader() -> ShaderRef {
        "embedded://f13_game/shaders/mesh_particles_prepass.wgsl".into()
    }
}

/// M_Firecracker_Debris_01's parameters (uniexpr.py: cb1[0] SelectionColor, [2] (EmissiveMultiplier, MetalnessMult,
/// Spec, RoughnessMult), [3].x OpacityMultiplier).
#[derive(Clone, Copy, Debug)]
struct DebrisParams {
    selection: Vec3,
    emissive_multiplier: f32,
    metalness: f32,
    spec: f32,
    roughness: f32,
    opacity: f32,
}

impl DebrisParams {
    /// Its compiled mesh-particle base pass (s049): Add_0 = Color.rgb x EmissiveMultiplier + DynamicParameter.y; base
    /// colour saturate(Add_0), emissive Add_0 + SelectionColor (clamped at 0), metallic saturate(MetalnessMult),
    /// specular saturate(Spec), roughness saturate(RoughnessMult); the opacity mask Color.a x OpacityMultiplier x
    /// DynamicParameter.x, clipped at 0.8333 after the dither (the shader). Two-sided; the mesh's own normal.
    fn material(&self, color: Vec4, dynamic: [f32; 4]) -> StandardMaterial {
        let add = color.truncate() * self.emissive_multiplier + Vec3::splat(dynamic[1]);
        let base = add.clamp(Vec3::ZERO, Vec3::ONE);
        let emissive = (add + self.selection).max(Vec3::ZERO);
        StandardMaterial {
            base_color: LinearRgba::new(base.x, base.y, base.z, color.w * self.opacity * dynamic[0]).into(),
            emissive: LinearRgba::rgb(emissive.x, emissive.y, emissive.z),
            metallic: self.metalness.clamp(0.0, 1.0),
            perceptual_roughness: self.roughness.clamp(0.0, 1.0),
            // UE's specular s: F0 = 0.08 s = 0.16 reflectance^2.
            reflectance: (0.5 * self.spec.clamp(0.0, 1.0)).sqrt(),
            double_sided: true,
            cull_mode: None,
            alpha_mode: AlphaMode::Mask(DEBRIS_CLIP),
            ..default()
        }
    }
}

/// M_Firecracker_Debris_01's OpacityMaskClipValue (the compiled clip: mask + dither - 0.8333 < 0 discards).
const DEBRIS_CLIP: f32 = 0.8333;

/// A mesh emitter drawn with its ported override material: the mesh cut into the chunks its material's World Position
/// Offset selects (M_Firecracker_Debris_01's vertex shader: a vertex whose ceil(TexCoord[2].x) isn't
/// ceil(DynamicParameter.z x 4), the particle's ChunkRandom, moves to the particle's origin, so each particle shows one
/// of the mesh's four shards), one mesh per chunk index 1..4 with the other chunks' vertices at the origin; per drawn
/// particle a slot entity and its own material.
struct ChunkedMesh {
    chunks: Arc<Vec<Handle<Mesh>>>,
    params: DebrisParams,
    cap: usize,
    materials: Vec<Handle<MeshParticleMaterial>>,
}

/// The particle's dynamic parameter (Float32x4), for the material's DynamicParameter.
const ATTRIBUTE_DYNAMIC: MeshVertexAttribute = MeshVertexAttribute::new("ParticleDynamic", 0x5f13_0001, VertexFormat::Float32x4);

#[derive(Clone, Copy, ShaderType, Default)]
struct ParticleParams {
    mode: Vec4,
    p: [Vec4; 32],
}

/// A particle material: the master's port (`mode`), its uniform buffer as the compiled shader lays it out, and its
/// textures in the shader's order. `distort`: its distortion pass (refraction), drawn in Bevy's transmissive pass.
#[derive(Asset, TypePath, AsBindGroup, Clone)]
#[bind_group_data(ParticleKey)]
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
    #[texture(5)]
    t3: Handle<Image>,
    /// A batch's sprites ([`GpuSprite`]), expanded to their quads by the vertex shader; the others bind [`NO_SPRITES`].
    #[storage(6, read_only)]
    sprites: Handle<ShaderBuffer>,
    alpha: AlphaMode,
    distort: bool,
    /// Drawn from `sprites` (a batch) rather than from its mesh's vertices.
    stored: bool,
    /// The material's bDisableDepthTest: drawn over everything (Jason's sound blips).
    no_depth_test: bool,
    /// The rain grid's GPU emitter ([`RainGpu`]): its sprites computed in the vertex shader from `sprites`.
    rain: bool,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct ParticleKey {
    distort: bool,
    stored: bool,
    no_depth_test: bool,
    rain: bool,
}

impl From<&ParticleMaterial> for ParticleKey {
    fn from(m: &ParticleMaterial) -> Self {
        ParticleKey { distort: m.distort, stored: m.stored, no_depth_test: m.no_depth_test, rain: m.rain }
    }
}

/// The placeholder sprite buffer the materials drawn from their meshes bind.
const NO_SPRITES: Handle<ShaderBuffer> = bevy::asset::uuid_handle!("6b1f6a4e-2f0c-4d52-9a35-6f1f0d6f2a11");


/// The most particles one emitter spawns in a tick (PLACEHOLDER: a guard, not the engine's; see the spawn).
const MAX_SPAWN_PER_TICK: u32 = 10_000;

fn no_sprites(mut buffers: ResMut<Assets<ShaderBuffer>>) {
    let _ = buffers.insert(&NO_SPRITES, ShaderBuffer::new(&[0u8; GPU_SPRITE_BYTES], RenderAssetUsages::RENDER_WORLD));
}

/// One batched sprite as the vertex shader reads it (particles.wgsl `sprite_vertex`), six vec4s: its centre and
/// rotation; size and pivot; colour; dynamic parameter; velocity direction and alignment (0 facing the view, 1
/// PSA_Velocity, 2 EPAL_ROTATE_Z); sub-image cell and grid. As UE's sprite vertex factory, each particle goes up once
/// (FParticleSpriteVertex: position, size, rotation, sub-image, colour) and the GPU expands it to its quad's corners.
type GpuSprite = [[f32; 4]; 6];
const GPU_SPRITE_BYTES: usize = std::mem::size_of::<GpuSprite>();

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
    fn reads_view_transmission_texture(&self) -> bool {
        self.distort
    }
    fn enable_prepass() -> bool {
        false
    }
    fn enable_shadows() -> bool {
        false
    }
    fn specialize(_: &MaterialPipeline, descriptor: &mut RenderPipelineDescriptor, layout: &MeshVertexBufferLayoutRef, key: MaterialPipelineKey<Self>) -> Result<(), SpecializedMeshPipelineError> {
        // The distortion pass: an opaque draw over the scene that leaves its depth alone.
        if key.bind_group_data.distort {
            if let Some(ds) = descriptor.depth_stencil.as_mut() {
                ds.depth_write_enabled = Some(false);
            }
        }
        if key.bind_group_data.no_depth_test {
            if let Some(ds) = descriptor.depth_stencil.as_mut() {
                ds.depth_compare = Some(bevy::render::render_resource::CompareFunction::Always);
            }
        }
        let l = if key.bind_group_data.stored {
            descriptor.vertex.shader_defs.push("PARTICLE_SPRITES".into());
            if key.bind_group_data.rain {
                descriptor.vertex.shader_defs.push("RAIN_ANALYTIC".into());
            }
            if let Some(f) = descriptor.fragment.as_mut() {
                f.shader_defs.push("PARTICLE_SPRITES".into());
            }
            layout.0.get_layout(&[Mesh::ATTRIBUTE_POSITION.at_shader_location(0)])?
        } else {
            layout.0.get_layout(&[
                Mesh::ATTRIBUTE_POSITION.at_shader_location(0),
                Mesh::ATTRIBUTE_UV_0.at_shader_location(1),
                Mesh::ATTRIBUTE_COLOR.at_shader_location(2),
                ATTRIBUTE_DYNAMIC.at_shader_location(3),
            ])?
        };
        descriptor.vertex.buffers = vec![l];
        // Sprites have no back (velocity-aligned ones turn either way).
        descriptor.primitive.cull_mode = None;
        Ok(())
    }
}

/// One particle (FBaseParticle and the payloads f13-game keeps), Unreal space.
/// FBaseParticle flags (Unreal's STATE_Particle_*).
const FREEZE: u32 = 1;
const IGNORE_COLLISIONS: u32 = 2;
const FREEZE_TRANSLATION: u32 = 4;
const FREEZE_ROTATION: u32 = 8;
const DELAY_COLLISIONS: u32 = 16;

/// A line trace in Unreal space (cm): the hit's fraction of the way and its normal.
type Trace<'a> = &'a dyn Fn(Vec3, Vec3) -> Option<(f32, Vec3)>;

/// A particle event (UParticleSystemComponent's SpawnEvents / DeathEvents): its type, name, where, the particle's
/// velocity, the emitter's time and the particle's.
struct ParticleEvent {
    kind: u8,
    name: String,
    location: Vec3,
    velocity: Vec3,
    emitter_time: f32,
    particle_time: f32,
}

/// What ParticleModuleCollision's update reads besides its traces (Unreal space, cm): the players' view locations
/// (the component's PlayerLocations; their LOD distance factors taken as 1, PLACEHOLDER), the world's bDropDetail,
/// whether the system went unrendered for over 0.1 s (World time - LastRenderTime > 0x142dd3420), and the system's
/// bounds (its particles' box, as the component's dynamic Bounds).
struct Collider<'a> {
    trace: Trace<'a>,
    views: &'a [Vec3],
    drop_detail: bool,
    unrendered: bool,
    bounds: Option<(Vec3, Vec3)>,
}

#[derive(Clone, Default)]
struct Particle {
    /// A ribbon's point: its up (Unreal space; FRibbonTypeDataPayload's UpVector, the source's at spawn).
    ribbon_up: Vec3,
    /// Its state flags (FBaseParticle::Flags): STATE_Particle_Freeze, IgnoreCollisions, FreezeTranslation,
    /// FreezeRotation, DelayCollisions.
    state: u32,
    /// FParticleCollisionPayload: UsedDampingFactor, UsedDampingFactorRotation, UsedCollisions, Delay.
    collision: (Vec3, Vec3, i32, f32),
    /// FAttractorParticlePayload per ParticleModuleAttractorParticle (in module order, at most two): its source's
    /// index in the attracting emitter and the source's last velocity.
    attractor: [(i32, Vec3); 2],
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
    /// A mesh emitter's FMeshRotationPayloadData (degrees, Euler as FRotator::MakeFromEuler reads it):
    /// InitialOrientation, InitRotation, Rotation, CurContinuousRotation, RotationRate, RotationRateBase.
    mesh_orientation: Vec3,
    mesh_init_rotation: Vec3,
    mesh_rotation: Vec3,
    mesh_continuous: Vec3,
    mesh_rate: Vec3,
    mesh_rate_base: Vec3,
    /// FLightParticlePayload: ColorScale, RadiusScale, LightExponent, bValid.
    light_color: Vec3,
    light_radius: f32,
    light_exponent: f32,
    light_valid: bool,
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
    /// A mesh emitter's mesh (its scene) and how many it draws (MaxDrawCount, at most 32); its drawn particles, one
    /// scene each, spawned as particles first need them (an idle emitter holds none).
    mesh_scene: Option<(Handle<WorldAsset>, usize)>,
    slots: Vec<Entity>,
    /// Or its mesh drawn with its override material ([`ChunkedMesh`]), into `slots` likewise.
    chunked: Option<ChunkedMesh>,
    /// A ParticleModuleLight emitter's simple lights (MaxDrawCount of them, at most 16).
    lights: Vec<Entity>,
    /// Drawn (an emitter kept only as another's attractor, its material unported or DefaultParticle, isn't).
    drawn: bool,
    /// Each ParticleModuleAttractorParticle's LastSelIndex (the module's own, in module order).
    attractor_last: [i32; 2],
    /// ParticleModuleCollision's instance byte: the bounds check runs when it is 0, counting 0..30.
    collision_check: u8,
    /// ParticleModuleSpawnPerUnit: the source's place last tick (Unreal space) and the travel left over.
    last_source: Option<Vec3>,
    unit_fraction: f32,
    /// A sprite emitter's material and its distortion pass (to draw it in a shared batch).
    material: Option<Handle<ParticleMaterial>>,
    distortion: Option<Handle<ParticleMaterial>>,
}

/// The view a sprite faces: the camera's place and axes (Bevy space).
struct SpriteView {
    pos: Vec3,
    right: Vec3,
    up: Vec3,
    fwd: Vec3,
}

/// A sprite particle's quad (world space): its centre (with its orbit), size x the component's scale, the emitter's
/// screen alignment (PSA_Velocity along its velocity; EPAL_ROTATE_Z axis lock upright, turned to the camera; else
/// facing the view), its rotation, its sub-image's cell, colour and dynamic parameter.
fn sprite_quad(e: &EmitterState, p: &Particle, scale: Vec3, view: &SpriteView, lock_z: bool) -> Quad {
    let def = &e.def;
    let [cols, rows] = [def.sub_images[0].max(1), def.sub_images[1].max(1)];
    let orbit = if p.orbit_offset != Vec3::ZERO {
        let r = p.orbit_rotation * TAU;
        Quat::from_euler(EulerRot::ZYX, r.z, r.y, r.x) * p.orbit_offset
    } else {
        Vec3::ZERO
    };
    let center = to_bevy(p.location + orbit);
    let size = Vec2::new(p.size.x * scale.x, p.size.y * scale.y) * 0.01;
    let (right, up) = match def.screen_alignment.as_str() {
        "PSA_Velocity" => {
            let v = to_bevy(p.velocity).normalize_or(view.up);
            let r = v.cross(view.pos - center).normalize_or(view.right);
            (r, v)
        }
        _ if lock_z => {
            // EPAL_ROTATE_Z: upright, turned about the world's up to face the camera.
            let up = Vec3::Y;
            let r = (view.pos - center).cross(up).normalize_or(view.right);
            (r, up)
        }
        _ => (view.right, view.up),
    };
    let (sin, cos) = p.rotation.sin_cos();
    let (r, u) = (right * cos + up * sin, up * cos - right * sin);
    let image = p.sub_image.max(0.0).floor() as u32 % (cols * rows);
    let cell = Vec2::new((image % cols) as f32, (image / cols) as f32);
    let grid = Vec2::new(cols as f32, rows as f32);
    let corners = [Vec2::new(0.0, 0.0), Vec2::new(1.0, 0.0), Vec2::new(1.0, 1.0), Vec2::new(0.0, 1.0)];
    // ParticlePositionAndSize: the centre and half the larger side.
    let dyn_value = if e.sphere { center.extend(0.5 * size.max_element()).to_array() } else { p.dynamic };
    Quad {
        positions: corners.map(|c| {
            let o = c + e.pivot;
            (center + r * (o.x * size.x) - u * (o.y * size.y)).to_array()
        }),
        uvs: corners.map(|c| ((cell + c) / grid).to_array()),
        colors: [p.color.to_array(); 4],
        dynamic: [dyn_value; 4],
        depth: (center - view.pos).dot(view.fwd),
    }
}

/// A batched sprite for the GPU ([`GpuSprite`]): what [`sprite_quad`] computes before its corners, and its depth.
fn gpu_sprite(e: &EmitterState, p: &Particle, scale: Vec3, view: &SpriteView, lock_z: bool) -> (GpuSprite, f32) {
    let def = &e.def;
    let [cols, rows] = [def.sub_images[0].max(1), def.sub_images[1].max(1)];
    let orbit = if p.orbit_offset != Vec3::ZERO {
        let r = p.orbit_rotation * TAU;
        Quat::from_euler(EulerRot::ZYX, r.z, r.y, r.x) * p.orbit_offset
    } else {
        Vec3::ZERO
    };
    let center = to_bevy(p.location + orbit);
    let size = Vec2::new(p.size.x * scale.x, p.size.y * scale.y) * 0.01;
    let (align, dir) = match def.screen_alignment.as_str() {
        "PSA_Velocity" => (1.0, to_bevy(p.velocity).normalize_or_zero()),
        _ if lock_z => (2.0, Vec3::ZERO),
        _ => (0.0, Vec3::ZERO),
    };
    let image = p.sub_image.max(0.0).floor() as u32 % (cols * rows);
    let dyn_value = if e.sphere { center.extend(0.5 * size.max_element()).to_array() } else { p.dynamic };
    let sprite = [
        center.extend(p.rotation).to_array(),
        [size.x, size.y, e.pivot.x, e.pivot.y],
        p.color.to_array(),
        dyn_value,
        dir.extend(align).to_array(),
        [(image % cols) as f32, (image / cols) as f32, cols as f32, rows as f32],
    ];
    (sprite, (center - view.pos).dot(view.fwd))
}

/// One camera-facing quad, world space (Bevy), and its distance along the view.
struct Quad {
    positions: [[f32; 3]; 4],
    uvs: [[f32; 2]; 4],
    colors: [[f32; 4]; 4],
    dynamic: [[f32; 4]; 4],
    depth: f32,
}

/// The quads of every sprite emitter that draws with one material (the rain grid's many systems), drawn as one mesh.
struct Batch {
    material: Handle<ParticleMaterial>,
    distortion: Option<Handle<ParticleMaterial>>,
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
    /// Spawning (bAutoActivate, or since its Activate).
    active: bool,
    /// The level actor it's on, and that actor's level.
    pub(crate) actor: String,
    pub(crate) level: String,
    /// The component it's attached to on that actor (the cabins' roof drips hang under "Rain").
    parent: Option<String>,
    /// The entity it follows and its offset from it (Bevy space).
    pub(crate) follow: Option<(Entity, Transform)>,
    /// Despawned once its emitters are done.
    auto_destroy: bool,
    /// SetVisibility(false): simulated, not drawn (the rain grid's occluded cells).
    hidden: bool,
    /// The transform it was last ticked at: local-space emitters (bUseLocalSpace) carry their particles with it.
    ticked_at: Option<(Vec3, Quat)>,
    /// When it was last drawn in the view (game seconds: the component's LastRenderTime, -1000 until drawn).
    last_render: f32,
    /// The component's CustomTimeDilation: its tick's time x this (UParticleSystemComponent::TickComponent).
    time_dilation: f32,
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
    /// Its own JSON (the export's path under the root).
    #[serde(default)]
    file: String,
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
    /// Each material's distortion pass, if it refracts.
    distortions: HashMap<String, Option<Handle<ParticleMaterial>>>,
}

impl Materials {
    fn get(&mut self, name: &str, root: &std::path::Path, assets: &AssetServer, materials: &mut Assets<ParticleMaterial>) -> Option<(Handle<ParticleMaterial>, u32)> {
        if let Some(h) = self.built.get(name) {
            return h.clone();
        }
        let built = self.build(name, root, assets).map(|m| {
            let mode = m.params.mode.x as u32;
            // A master with its distortion pass ported (the Refraction input's graph): M_Rain_Master's (mode 30), the
            // knife trail's (32).
            let pass = match mode {
                7 => Some(30.0),
                31 => Some(32.0),
                33 => Some(34.0),
                35 => Some(36.0),
                _ => None,
            };
            let distortion = pass.map(|d| ParticleMaterial { params: ParticleParams { mode: Vec4::new(d, 0.0, 0.0, 0.0), ..m.params }, alpha: AlphaMode::Opaque, distort: true, ..m.clone() });
            self.distortions.insert(name.to_string(), distortion.map(|d| materials.add(d)));
            (materials.add(m), mode)
        });
        self.built.insert(name.to_string(), built.clone());
        built
    }

    /// A mesh emitter's override material, if ported ([`DebrisParams`]).
    fn mesh_particle(&self, name: &str) -> Option<DebrisParams> {
        let e = self.entries.get(name).or_else(|| self.entries.iter().find(|(k, _)| k.eq_ignore_ascii_case(name)).map(|(_, v)| v))?;
        if !e.master.eq_ignore_ascii_case("M_Firecracker_Debris_01") {
            return None;
        }
        let s = |n: &str, d: f32| e.scalars.get(n).copied().unwrap_or(d);
        let sel = e.vectors.get("SelectionColor").copied().unwrap_or([0.0; 4]);
        Some(DebrisParams {
            selection: Vec3::new(sel[0], sel[1], sel[2]),
            emissive_multiplier: s("EmissiveMultiplier", 0.3),
            metalness: s("MetalnessMult", 1.0),
            spec: s("Spec", 0.1),
            roughness: s("RoughnessMult", 0.3),
            opacity: s("OpacityMultiplier", 1.0),
        })
    }

    /// The distortion pass of a material [`Materials::get`] built.
    fn distortion(&self, name: &str) -> Option<Handle<ParticleMaterial>> {
        self.distortions.get(name).cloned().flatten()
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
        let mut p = [Vec4::ZERO; 32];
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
                (1.0, [tex("FlowMap1"), tex("BaseTexture1"), tex("BaseTexture2"), None])
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
                (2.0, [tex("Base_ALPHA_Texture1"), tex("Base_ALPHA_Texture3"), None, None])
            }
            "M_MasterParticle_Emissive" => {
                // uniexpr.py M_MasterParticle_Emissive: cb1[2] the vector parameter (named "None"; "" in the export),
                // cb1[3] SelectionColor.rgb (0), [4..6] below.
                let v = e.vectors.get("").or_else(|| master.and_then(|m| m.vectors.get(""))).copied().unwrap_or([1.0; 4]);
                p[2] = Vec4::new(v[0], v[1], v[2], 0.0);
                p[4] = Vec4::new(s("EmissiveLevel", 1.0), s("TimeMultiplier", 1.0), s("Texture3_Scroll_Multiplier", 0.5), s("Texture3_UV_Coord_Multiplier", 1.0));
                p[5] = Vec4::new(s("TimeMultiplier_2", 0.83), s("Texture3_Scroll_Multiplier_2", -0.4), s("Texture3_UV_Coord_Multiplier_2", 1.0), s("fadeAmount", 0.0));
                p[6] = Vec4::new(s("fadeAmount", 0.0).max(1e-5), s("Opacity_Power", 2.0), s("RefractionDepthBias", 0.0), 0.0);
                (3.0, [tex("ParticleTexture"), tex("Texture1"), tex("Texture1_1"), None])
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
                (4.0, [tex("Normal1"), tex("Texture1"), None, None])
            }
            "M_MasterParticle_Diffuse_nearFade" => {
                // uniexpr.py M_MasterParticle_Diffuse_nearFade: cb1[2] SelectionColor.rgb + 0, [3] (fadeDistance,
                // max(1e-5, fadeDistance), 1 / that, fadeAmount), [4] (max(fadeAmount, 1e-5), RefractionDepthBias).
                let near = s("fadeDistance", 200.0).max(1e-5);
                let amount = s("fadeAmount", 0.0);
                p[3] = Vec4::new(s("fadeDistance", 200.0), near, 1.0 / near, amount);
                p[4] = Vec4::new(amount.max(1e-5), s("RefractionDepthBias", 0.0), 0.0, 0.0);
                (5.0, [tex("ParticleTexture"), None, None, None])
            }
            "M_Procedural_Smoke_05_no-self-shadow" => {
                // uniexpr.py M_Procedural_Smoke_05_no-self-shadow, its second uniform set (the instance's parameters):
                // [2] Base_ALPHA_Texture1_Scale.rgb, [3] that x BaseMask_Master_Scale, [5] Texture2_Scale.rgb, [6]
                // BaseMask_Master_Scale x that, [7] SelectionColor.rgb (0), [8..14] below.
                let vec = |n: &str, d: [f32; 4]| e.vectors.get(n).or_else(|| master.and_then(|m| m.vectors.get(n))).copied().unwrap_or(d);
                let (s1, s2) = (vec("Base_ALPHA_Texture1_Scale", [0.95, 0.95, 0.0, 1.0]), vec("Base_ALPHA_Texture2_Scale", [1.0, 1.0, 0.0, 1.0]));
                let mask = s("BaseMask_Master_Scale", 1.5);
                let combined = s("1_TexturesCombinedTimeMultiplier", 1.0);
                let hard = s("SphereMask_Hardness", 0.0);
                let soft = (1.0 - hard).max(1e-5);
                p[2] = Vec4::new(s1[0], s1[1], s1[2], 0.0);
                p[3] = Vec4::new(s1[0] * mask, s1[1] * mask, s1[2] * mask, 0.0);
                p[5] = Vec4::new(s2[0], s2[1], s2[2], 0.0);
                p[6] = Vec4::new(s2[0] * mask, s2[1] * mask, s2[2] * mask, 0.0);
                p[8] = Vec4::new(s("0_BaseContrast", 0.0), s("0_AlphaContrast", 0.0), s("0_Texture3Contrast", 0.0), s("0_ScrollTexture3TimeMultiplier", 0.05));
                p[9] = Vec4::new(s("ScrollTextureDistanceMultiplier", 0.75), s("Texture3_UV_Coord_Multiplier", 0.75), s("0_DistortionMultiplier", 0.0), s("Distortion_UV_Coord_Multiplier", 1.0));
                p[10] = Vec4::new(combined, s("0_Texture1TimeMultiplier", 0.5), s("0_Texture1TimeMultiplier", 0.5) * combined, mask);
                p[11] = Vec4::new(s("0_Texture2TimeMultiplier", 0.61), s("0_Texture2TimeMultiplier", 0.61) * combined, s("SphereMask_Radius", 0.5), hard);
                p[12] = Vec4::new(1.0 - hard, soft, 1.0 / soft, s("BaseMax", 1.0));
                p[13] = Vec4::new(s("BaseMin", 0.1), s("Emissive_Multiplier", 0.01), s("FadeDistance", 3.0), s("FadeDistance", 3.0).max(1e-5));
                p[14] = Vec4::new(s("0_PixelDepthDistance", 0.0), s("0_Opacity_Multiplier", 1.0), s("RefractionDepthBias", 0.0), 0.0);
                (6.0, [tex("DistortionTexture"), tex("Texture3"), tex("Texture1"), tex("Texture2")])
            }
            "M_Procedural_Smoke_01B_no-self-shadow" => {
                // uniexpr.py M_Procedural_Smoke_01B_no-self-shadow: cb1[5] Base_ALPHA_Texture1_Scale.rg x
                // BaseMask_Master_Scale, [8] Texture2's, [9].x -Diffuse_Contrast, [10].x -0_BaseContrast, [11]
                // SelectionColor.rgb (0), [13].x -0_AlphaContrast, [15..24] below.
                let vec = |n: &str, d: [f32; 4]| e.vectors.get(n).or_else(|| master.and_then(|m| m.vectors.get(n))).copied().unwrap_or(d);
                let (t1s, t2s) = (vec("Base_ALPHA_Texture1_Scale", [0.95, 0.95, 0.0, 1.0]), vec("Base_ALPHA_Texture2_Scale", [1.0, 1.0, 0.0, 1.0]));
                let mask = s("BaseMask_Master_Scale", 1.3);
                let (bc, dc, ac, hard) = (s("0_BaseContrast", 0.1), s("Diffuse_Contrast", 0.0), s("0_AlphaContrast", 0.0), s("SphereMask_Hardness", 0.0));
                let soft = (1.0 - hard).max(1e-5);
                p[5] = Vec4::new(t1s[0] * mask, t1s[1] * mask, t1s[2] * mask, 0.0);
                p[8] = Vec4::new(t2s[0] * mask, t2s[1] * mask, t2s[2] * mask, 0.0);
                p[9] = Vec4::splat(-dc);
                p[10] = Vec4::splat(-bc);
                p[13] = Vec4::splat(-ac);
                p[15] = Vec4::new(s("0_MasterScrollMultiplier", 0.001), s("Distortion1_Scroll_Multiplier", 0.05), s("Distortion1_UV_Coord_Multiplier", 0.5), s("Distortion2_Scroll_Multiplier", 0.08));
                p[16] = Vec4::new(s("Distortion2_UV_Coord_Multiplier", 0.5), s("z_DistortionBaseMapPower", 5.0), s("0_DistortionMultiplier", 1.0), s("DistortionMax", 5.0));
                p[17] = Vec4::new(s("DistortionMin", 0.0), s("0_MasterRotationMultiplier", 0.5), mask, s("Texture3_Scroll_Multiplier", 0.15));
                p[18] = Vec4::new(s("Texture3_UV_Coord_Multiplier", 0.5), s("Texture3_Max", 1.0), s("Texture3_Min", 0.5), s("z_DistortionSphereMaskPower", 1.0));
                p[19] = Vec4::new(s("SphereMask_Distotrion_Multiplier", 1.0), s("SM_U_Offset", 0.5), s("SM_V_Offset", 0.5), s("SphereMask_Radius", 0.5));
                p[20] = Vec4::new(hard, 1.0 - hard, soft, 1.0 / soft);
                p[21] = Vec4::new(dc + 1.0, s("Diffuse_Max", 1.0), s("Diffuse_Min", 0.2), bc + 1.0);
                p[22] = Vec4::new(s("FadeDistance", 3.0), s("FadeDistance", 3.0).max(1e-5), s("0_PixelDepthDistance", 0.0), s("Emissive_Multiplier", 0.05));
                p[23] = Vec4::new(ac, -ac, ac + 1.0, s("0_Opacity_Multiplier", 3.0));
                p[24] = Vec4::new(s("0_AlphaPower", 2.0), s("RefractionDepthBias", 0.0), 0.0, 0.0);
                (9.0, [tex("DistortionTexture"), tex("Base_ALPHA_Texture1"), tex("Base_ALPHA_Texture2"), tex("Base_ALPHA_Texture3")])
            }
            "M_Procedural_Smoke_01B_no-self-shadow_separate-transparency_off" => {
                // uniexpr.py of it: cb1[5] Base_ALPHA_Texture1_Scale.rg x BaseMask_Master_Scale, [8] Texture2's, [9].x
                // -Diffuse_Contrast, [10].x -0_BaseContrast, [11] SelectionColor.rgb (0), [13].x -0_AlphaContrast,
                // [15..23] below.
                let vec = |n: &str, d: [f32; 4]| e.vectors.get(n).or_else(|| master.and_then(|m| m.vectors.get(n))).copied().unwrap_or(d);
                let (t1s, t2s) = (vec("Base_ALPHA_Texture1_Scale", [0.95, 0.95, 0.0, 1.0]), vec("Base_ALPHA_Texture2_Scale", [1.0, 1.0, 0.0, 1.0]));
                let mask = s("BaseMask_Master_Scale", 2.0);
                let (bc, dc, ac, hard) = (s("0_BaseContrast", 0.1), s("Diffuse_Contrast", 0.0), s("0_AlphaContrast", 0.0), s("SphereMask_Hardness", 0.0));
                let soft = (1.0 - hard).max(1e-5);
                p[5] = Vec4::new(t1s[0] * mask, t1s[1] * mask, t1s[2] * mask, 0.0);
                p[8] = Vec4::new(t2s[0] * mask, t2s[1] * mask, t2s[2] * mask, 0.0);
                p[9] = Vec4::splat(-dc);
                p[10] = Vec4::splat(-bc);
                p[13] = Vec4::splat(-ac);
                p[15] = Vec4::new(s("0_MasterScrollMultiplier", 0.001), s("Distortion1_Scroll_Multiplier", 0.05), s("Distortion1_UV_Coord_Multiplier", 0.5), s("DistortionTexturePower", 5.0));
                p[16] = Vec4::new(s("Distortion2_Scroll_Multiplier", 0.08), s("Distortion2_UV_Coord_Multiplier", 0.4), s("0_DistortionMultiplier", 0.3), s("DistortionMax", 1.0));
                p[17] = Vec4::new(s("DistortionMin", 0.0), s("0_MasterRotationMultiplier", 0.5), mask, s("Texture3_Scroll_Multiplier", 0.15));
                p[18] = Vec4::new(s("Texture3_UV_Coord_Multiplier", 0.5), s("Texture3_Max", 1.0), s("Texture3_Min", 0.5), s("SphereMask_Distotrion_Multiplier", 1.0));
                p[19] = Vec4::new(s("SphereMask_Radius", 0.6), hard, 1.0 - hard, soft);
                p[20] = Vec4::new(1.0 / soft, dc + 1.0, s("Diffuse_Max", 1.0), s("Diffuse_Min", 0.2));
                p[21] = Vec4::new(bc + 1.0, s("FadeDistance", 3.0), s("FadeDistance", 3.0).max(1e-5), s("0_PixelDepthDistance", 0.0));
                p[22] = Vec4::new(s("Emissive_Multiplier", 0.05), ac, -ac, ac + 1.0);
                p[23] = Vec4::new(s("0_Opacity_Multiplier", 3.0), s("RefractionDepthBias", 0.0), 0.0, 0.0);
                (10.0, [tex("DistortionTexture"), tex("Base_ALPHA_Texture1"), tex("Base_ALPHA_Texture2"), tex("Base_ALPHA_Texture3")])
            }
            "M_Enviroment_Fog_02" => {
                // uniexpr.py M_Enviroment_Fog_02 (FoldedMath 0 add, 1 sub, 2 mul, 3 div): [1] SelectionColor.rgb (0),
                // [3].x -0_BaseContrast, [5].x -0_AlphaContrast, [6] (SM_U_Offset + dms / 30, SM_V_Offset + dms /
                // 20), [7..18] below (dms 0_Distotrion_Multiplier_SphereMask).
                let (bc, ac, dms, r, hard) = (s("0_BaseContrast", 0.0), s("0_AlphaContrast", 0.0), s("0_Distotrion_Multiplier_SphereMask", 0.0), s("SphereMask_Radius", 0.5), s("SphereMask_Hardness", 0.0));
                let inner = (r - dms / 8.0).clamp(0.0, 0.7).max(1e-5);
                p[3] = Vec4::splat(-bc);
                p[5] = Vec4::splat(-ac);
                p[6] = Vec4::new(s("SM_U_Offset", 0.5) + dms / 30.0, s("SM_V_Offset", 0.5) + dms / 20.0, 0.0, 0.0);
                p[7] = Vec4::new(s("z_Master_Time", 0.013), s("Base_UV_Scale", 0.5), s("z_Distortion_Time", 0.013), s("Distortion_UV_Scale", 0.5));
                p[8] = Vec4::new(s("z_DistortionSphereMaskPower", 1.0), s("0_Distotrion_Multiplier_Base", 0.0), s("DistortionMax", 5.0), s("DistortionMin", 0.0));
                p[9] = Vec4::new(s("z_Base2_Time_Multiplier", 2.0), s("EmissiveMultiplier", 0.3), bc, -bc);
                p[10] = Vec4::new(s("Base_Add_Subtract", 0.0), s("BaseMultiplier", 1.0), bc + 1.0, ac);
                p[11] = Vec4::new(-ac, s("0_AlphaBaseMax", 1.0), s("0_AlphaBaseMin", 0.0), ac + 1.0);
                p[12] = Vec4::new(s("0_Opacity_MASTER", 1.0), s("0_Opacity_Max", 1.0), dms, dms);
                p[15] = Vec4::new(r - dms / 8.0, (r - dms / 8.0).clamp(0.0, 0.7), inner, 1.0 / inner);
                p[16] = Vec4::new(hard, 1.0 - hard, (1.0 - hard).max(1e-5), 1.0 / (1.0 - hard).max(1e-5));
                p[17] = Vec4::new(r.max(1e-5), 1.0 / r.max(1e-5), s("SphereMask_Fade_Power", 0.5), s("0_FadeDistance", 512.0));
                p[18] = Vec4::new(s("0_FadeDistance", 512.0).max(1e-5), s("0_PixelDepthDistance", 768.0), s("Fade Exponent", 1.0), s("RefractionDepthBias", 0.0));
                (11.0, [tex("DistortionTexture"), tex("BaseTexture1"), tex("BaseTexture2"), None])
            }
            // M_Procedural_Tire_Dust_01_Inst's own shader map (static switches: 0_NRM_switch off); the master's other
            // instances aren't ported. M_Procedural_Dust_01_self-shadow's (the walls' hit dust) has the same layout
            // (uniexpr.py; [14].w 0_BaseContrastLevel); M_Savini_Jason_EyeGlow_01_Inst's own permutation (its sprite
            // pass samples NormalTexture1, for the lighting only) the same graph with its layout one slot on
            // (NRM_Intensity at [9].w): filled by name, so the same arm.
            "M_Procedural_Smoke_04_no-self-shadow" | "M_Procedural_Dust_01_self-shadow" if name == "M_Procedural_Tire_Dust_01_Inst" || e.master == "M_Procedural_Dust_01_self-shadow" => {
                let dust = e.master == "M_Procedural_Dust_01_self-shadow";
                // uniexpr.py M_Procedural_Tire_Dust_01_Inst: cb1[1] SelectionColor.rgb (0), [3].x
                // -BaseTextureContrast, [5].x -0_MasterAlphaContrast, [6..14] below.
                let (btc, mac, hard) = (s("BaseTextureContrast", 0.3), s("0_MasterAlphaContrast", 0.0), s("SphereMask_Hardness", 0.0));
                let soft = (1.0 - hard).max(1e-5);
                p[3] = Vec4::splat(-btc);
                p[5] = Vec4::splat(-mac);
                p[6] = Vec4::new(s("0_EmissiveMasterLevel", 0.01), btc, -btc, s("0_FlowMap_Multiply", 0.1));
                p[7] = Vec4::new(s("0_Flowmap1_XY_Time_Multiply", -0.3), s("Flowmap1_U_Coord_Multiply", 0.75), s("Flowmap1_V_Coord_Multiply", 0.75), s("Flowmaps_BasemapTileMultiply", 0.25));
                p[8] = Vec4::new(s("0_Basemap1_XY_Time_Multiply", 0.1), s("Basemap1_U_Coord_Multiply", 1.0), s("Basemap1_V_Coord_Multiply", 1.0), s("0_SmokeTexSize", 5.0));
                p[9] = Vec4::new(s("0_Flowmap2_XY_Time_Multiply", 0.251), s("Flowmap2_U_Coord_Multiply", 0.75), s("Flowmap2_V_Coord_Multiply", 0.75), s("0_Basemap2_XY_Time_Multiply", -0.05));
                p[10] = Vec4::new(s("Basemap2_U_Multiply", 1.0), s("Basemap2_V_Multiply", 1.0), s("0_BaseTextureDarken", 5.0), btc + 1.0);
                p[11] = Vec4::new(s("SPEC", 0.1), s("0_FadeDistance", 25.0), s("0_FadeDistance", 25.0).max(1e-5), s("0_PixelDepthDistance", 0.0));
                p[12] = Vec4::new(s("0_Opacity_MASTER", 1.0), mac, -mac, s("SphereMaskGlobalDistortionLevel", 1.0));
                p[13] = Vec4::new(s("SphereMask_Radius", 0.5), hard, 1.0 - hard, soft);
                p[14] = Vec4::new(1.0 / soft, mac + 1.0, s("0_Alpha_Contrast_Opacity_MASTER", 3.0), s("0_BaseContrastLevel", 1.0));
                p[15] = if dust { Vec4::new(0.0, s("0_BaseContrastLevel", 1.0), 0.0, 0.0) } else { Vec4::new(1.0, 1.0, 0.0, 0.0) };
                (8.0, [tex("FlowMap1"), tex("BaseTexture1"), None, None])
            }
            "M_Savini_Jason_Ashes_01" => {
                // uniexpr.py M_Savini_Jason_Ashes_01 (its instances use the master's shader map; M_Savini_Smoke_01_Inst
                // has its own, its sprite pass the same): the layout as the shader reads it, [0..20].
                let vec = |n: &str, d: [f32; 4]| e.vectors.get(n).or_else(|| master.and_then(|m| m.vectors.get(n))).copied().unwrap_or(d);
                let (ec, bc) = (vec("EmissiveColor", [1.0; 4]), vec("BaseColor", [0.0, 0.0, 0.0, 1.0]));
                let scroll = s("ScrollSpeed_Texture1", 0.1);
                let (radius, hard, el) = (s("SphereMaskRadius", 0.5), s("SphereMaskHardness", 0.0), s("EmissiveLevel", 10.0));
                let (r, soft) = (radius.max(1e-5), (1.0 - hard).max(1e-5));
                p[2] = Vec4::new(s("TexCoordRand1_Xspeed", 1.0) * scroll, s("TexCoordRand1_Yspeed", 1.0) * scroll, 0.0, 0.0);
                p[4] = Vec4::new(s("TexCoordRand2_Xspeed", 1.0) * scroll, s("TexCoordRand2_Yspeed", 1.0) * scroll, 0.0, 0.0);
                p[6] = Vec4::new(ec[0], ec[1], ec[2], 0.0);
                p[7] = Vec4::new(ec[0] * el, ec[1] * el, ec[2] * el, 0.0);
                p[10] = Vec4::new(bc[0], bc[1], bc[2], 0.0);
                p[11] = Vec4::new(s("ScrollTexture1a_RotationAngle", 0.0), 0.0, 0.0, scroll);
                p[12] = Vec4::new(s("TexCoordRand1_U", 1.0), s("UV_MasterScaleMultiplier", 1.0), s("TexCoordRand1_V", 1.0), s("NRM1_Multiplier", 1.0));
                p[13] = Vec4::new(s("ScrollTexture1b_RotationAngle", 0.0), 0.0, 0.0, s("TexCoordRand2_U", -1.0));
                p[14] = Vec4::new(s("TexCoordRand2_V", -1.0), s("NRM2_Multiplier", 1.0), s("NRM_Intensity", 1.0), s("ScrollSpeed_Emissive1", 0.3));
                p[15] = Vec4::new(s("ScrollSpeed_Emissive1", 0.3), s("UV_EmissiveScaleMultiplier", 1.0), s("z_U_Emissive_OFFSET", 0.0), s("z_V_Emissive_OFFSET", 0.0));
                p[16] = Vec4::new(s("EmissiveTexture1_RotationAngle", 0.0), el, s("Texture1a_Multiplier", 1.0), s("Texture1b_Multiplier", 1.0));
                p[17] = Vec4::new(s("Texture1_Add_Multiply_Lerp", 1.0), s("BaseTextureMask_UV_Scale", 1.0), s("EmissiveLevelScrollTexture", 0.0), s("0_Alpha_Contrast_Opacity_MASTER", 10.0));
                p[18] = Vec4::new(radius, r, 1.0 / r, hard);
                p[19] = Vec4::new(1.0 - hard, soft, 1.0 / soft, s("BaseTextureMask1_Distortion", 2.0));
                p[20] = Vec4::new(s("0_ALPHA_Opacity_MASTER", 1.0), s("FadeDistance", 32.0), s("FadeDistance", 32.0).max(1e-5), s("0_PixelDepthDistance", 0.0));
                // EmissiveTexture1 second: the Smoke and Distortion instances name Engine/EngineMaterials/DefaultDiffuse, which
                // the export doesn't carry; they draw it x EmissiveLevel 0.
                (12.0, [tex("ScrollTexture1"), tex("EmissiveTexture1"), tex("BaseTextureMask1"), None])
            }
            // M_Blood_Burst_01's shader map (M_Blood_Spray_10's own is the same, diffed; M_Blood_Bursts_03_Inst has none).
            // M_Blood_Squirt_01 has its own permutation (s003 diffed): the same shading, its UV distorted (mode 14).
            "M_Gradient_Graphic_Ring_01" => {
                // Jason's sound blips (P_Sound_Wave_Indicator_01). uniexpr.py: cb1[2] ParticleColorParameter.rgb (2), cb1[3]
                // SelectionColor.rgb (0); its sprite base pass s006 samples Normal_Main (BC1, linear) as colour.
                let vec = |n: &str, d: [f32; 4]| e.vectors.get(n).or_else(|| master.and_then(|m| m.vectors.get(n))).copied().unwrap_or(d);
                let (c, sel) = (vec("ParticleColorParameter", [2.0, 2.0, 2.0, 1.0]), vec("SelectionColor", [0.0, 0.0, 0.0, 1.0]));
                p[2] = Vec4::new(c[0], c[1], c[2], 0.0);
                p[3] = Vec4::new(sel[0], sel[1], sel[2], 0.0);
                (15.0, [tex("Normal_Main"), None, None, None])
            }
            "M_Blood_Burst_01" => {
                // uniexpr.py M_Blood_Burst_01: [1] (FLIPBOOK_COLUMNS, ROWS), [2] (COLUMNS x ROWS, ROWS), [4].x
                // -EmissiveHighlightContrast, [6] BloodColor.rgb, [7] SelectionColor.rgb (0), [8..12] below.
                let vec = |n: &str, d: [f32; 4]| e.vectors.get(n).or_else(|| master.and_then(|m| m.vectors.get(n))).copied().unwrap_or(d);
                let (cols, rows, ehc) = (s("FLIPBOOK_COLUMNS", 8.0), s("FLIPBOOK_ROWS", 8.0), s("EmissiveHighlightContrast", 20.0));
                let blood = vec("BloodColor", [0.1, 0.0, 0.0, 1.0]);
                p[1] = Vec4::new(cols, rows, 0.0, 0.0);
                p[2] = Vec4::new(cols * rows, rows, 0.0, 0.0);
                p[4] = Vec4::splat(-ehc);
                p[6] = Vec4::new(blood[0], blood[1], blood[2], 0.0);
                p[8] = Vec4::new(rows, cols, cols * rows, s("FLIPBOOK_Global_Time", 1.0));
                p[9] = Vec4::new(s("FLIPBOOK_Master_Speed", 1.0), s("DynamicFrameAnimation_ON_OFF", 1.0), s("0_FLIPBOOK_PLAY_ON_OFF", 1.0), s("NRM_Intensity", 0.5));
                p[10] = Vec4::new(ehc, -ehc, ehc + 1.0, s("EmmissiveMultiplier", 0.1));
                p[11] = Vec4::new(s("MET", 0.0), s("SPEC", 0.01), s("ROUGH", 0.05), s("FadeDistance", 3.0));
                p[12] = Vec4::new(s("FadeDistance", 3.0).max(1e-5), s("0_PixelDepthDistance", 0.0), s("0_Opacity_MASTER", 2.0), s("RefractionDepthBias", 0.0));
                if name == "M_Blood_Squirt_01" {
                    // Its sprite pass: o = DynamicParameter.y + 0_DistortionMultiplier off the flipbook's UV; two
                    // DistortionTexture samples circling (radius 0.75) at RealTime x 0_MasterScrollMultiplier x each
                    // scroll x 2 pi, at their UV multipliers (the second mirrored), each ^ DistortionTexturePower,
                    // summed x o and clamped to DistortionMin..Max, added to the UV.
                    p[13] = Vec4::new(s("0_DistortionMultiplier", 0.0), s("0_MasterScrollMultiplier", 1.0), s("Distortion1_Scroll_Multiplier", 0.05), s("Distortion1_UV_Coord_Multiplier", 0.5));
                    p[14] = Vec4::new(s("DistortionTexturePower", 5.0), s("Distortion2_Scroll_Multiplier", 0.08), s("Distortion2_UV_Coord_Multiplier", 0.4), s("DistortionMax", 1.0));
                    p[15] = Vec4::new(s("DistortionMin", 0.0), 0.0, 0.0, 0.0);
                    (14.0, [tex("DIFF"), tex("DistortionTexture"), None, None])
                } else {
                    (13.0, [tex("DIFF"), None, None, None])
                }
            }
            "M_Water_Splash_03_masked-blend" | "M_Water_Splash_02_masked-blend_separate-transparency_off" => {
                // uniexpr.py M_Water_Splash_03_masked-blend (its _02_..._separate-transparency_off sibling's set is
                // the same): cb1[1] SelectionColor.rgb (0), [2..5].
                p[2] = Vec4::new(s("NRM_Intensity", 1.0), s("DiffuseFresnel", 2.0), s("DiffuseMultiplier", 0.5), s("EmissiveBaseLevel", 0.05));
                p[3] = Vec4::new(s("EmissiveScroll_UV_Multiplier", 0.2), s("EmissiveScrollLevel", 0.025), s("MET", 0.03), s("SPEC", 0.1));
                p[4] = Vec4::new(s("ROUGH", 0.02), s("FadeDistance", 3.0), s("FadeDistance", 3.0).max(1e-5), s("0_PixelDepthDistance", 0.0));
                p[5] = Vec4::new(s("OpacityMultiplier", 1.0), s("Index of Refraction", 1.333), s("RefractionDepthBias", 0.0), 0.0);
                (33.0, [tex("normal"), tex("CloudMask"), tex("mask"), None])
            }
            "M_Water_Splash_02_masked-blend" => {
                // uniexpr.py M_Water_Splash_02_masked-blend: cb1[3] SelectionColor.rgb (0), [4..9].
                let c = s("EmissiveScroll_Contrast", 0.5);
                p[4] = Vec4::new(s("NRM_Intensity", 1.0), s("DiffuseFresnel", 2.0), s("DiffuseMultiplier", 0.5), s("EmissiveMasterLevel", 0.1));
                p[5] = Vec4::new(c, -c, s("EmissiveScroll_Time_Multiplier", 1.0), s("EmissiveScroll_UV_Multiplier", 0.5));
                p[6] = Vec4::new(1.0 + c, s("EmissiveScrollBrightnessLevel", 0.05), s("MET", 0.03), s("SPEC", 0.1));
                p[7] = Vec4::new(s("ROUGH", 0.02), s("0_GradMask_Multiplier", 1.0), s("0_GradMask_Multiplier", 1.0), s("FadeDistance", 20.0));
                p[8] = Vec4::new(s("FadeDistance", 20.0).max(1e-5), s("0_PixelDepthDistance", 0.0), s("OpacityMultiplier", 1.0), s("Index of Refraction", 1.333));
                p[9] = Vec4::new(s("RefractionDepthBias", 0.0), 0.0, 0.0, 0.0);
                (35.0, [tex("normal"), tex("CloudMask"), tex("mask"), None])
            }
            "M_Jason_ThrowingKnife_Trail_02" => {
                // uniexpr.py M_Jason_ThrowingKnife_Trail_02: cb1[2] GlowColor.rgb, [3] SelectionColor.rgb (0), [4..6].
                let vec = |n: &str, d: [f32; 4]| e.vectors.get(n).or_else(|| master.and_then(|m| m.vectors.get(n))).copied().unwrap_or(d);
                let glow = vec("GlowColor", [0.3, 0.3, 0.3, 1.0]);
                p[2] = Vec4::new(glow[0], glow[1], glow[2], 0.0);
                p[4] = Vec4::new(s("Basemap1_X_Speed_Multiply", 0.05), s("Basemap1_U_Coord_Multiply", 3.0), s("Basemap1_Y_Speed_Multiply", 0.0), s("Basemap1_V_Coord_Multiply", 1.0));
                p[5] = Vec4::new(s("NRM_MainTextureIntensity", 1.0), s("0_EdgeTransparencyMultiplier", 1.5), s("OPACITY_Multiplier", 0.025), s("Index of Refraction", 1.7));
                p[6] = Vec4::new(s("RefractionDepthBias", 0.0), 0.0, 0.0, 0.0);
                (31.0, [tex("Normal_Main"), tex("SpriteTexture"), None, None])
            }
            "M_Rain_Master" => {
                // uniexpr.py M_Rain_Master: cb1[2] EmissiveColor.rgb, [3] SelectionColor.rgb (0), [5] BaseColor.rgb,
                // [6..10] below; Rain_MPC's Rain in p[15].x (rain_mpc_materials).
                let vec = |n: &str, d: [f32; 4]| e.vectors.get(n).or_else(|| master.and_then(|m| m.vectors.get(n))).copied().unwrap_or(d);
                let (ec, bc) = (vec("EmissiveColor", [1.0; 4]), vec("BaseColor", [0.0, 0.0, 0.0, 1.0]));
                let radius = s("SphereMaskRadius", 0.5).max(1e-5);
                let hard = s("SphereMaskHardness", 0.0);
                let soft = (1.0 - hard).max(1e-5);
                p[2] = Vec4::new(ec[0], ec[1], ec[2], 0.0);
                p[5] = Vec4::new(bc[0], bc[1], bc[2], 0.0);
                p[6] = Vec4::new(s("ScrollSpeed_Texture1", 0.01), s("ScrollSpeed_Texture1", 0.01), s("UV_MasterScaleMultiplier", 0.3), s("NRM_Intensity", 0.0));
                p[7] = Vec4::new(s("ScrollSpeed_Emissive1", 0.3), s("ScrollSpeed_Emissive1", 0.3), s("UV_EmissiveScaleMultiplier", 1.0), s("z_U_Emissive_OFFSET", 0.0));
                p[8] = Vec4::new(s("z_V_Emissive_OFFSET", 0.0), s("EmissiveLevel", 10.0), s("SphereMaskRadius", 0.5), radius);
                p[9] = Vec4::new(1.0 / radius, hard, 1.0 - hard, soft);
                p[10] = Vec4::new(1.0 / soft, s("0_Alpha_Contrast_Opacity_MASTER", 10.0), s("Index of Refraction", 1.333), s("RefractionDepthBias", 0.0));
                (7.0, [tex("Normal1"), tex("EmissiveTexture1"), tex("ScrollTexture1"), None])
            }
            // Masters whose graphs aren't ported yet aren't drawn (F13_PARTICLE_STANDINS=1 draws them as their first
            // texture x the particle colour, PLACEHOLDER, to check the simulation).
            _ if std::env::var("F13_PARTICLE_STANDINS").is_ok() => {
                let first = ["ParticleTexture", "Texture", "BaseTexture1", "Base_ALPHA_Texture1"].iter().find_map(|n| tex(n)).or_else(|| {
                    let mut names: Vec<&String> = e.textures.keys().collect();
                    names.sort();
                    names.first().and_then(|n| tex(n))
                });
                (0.0, [first, None, None, None])
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
        let [t0, t1, t2, t3] = t;
        let t0 = t0?;
        // bDisableDepthTest, from the material's own JSON (materials.json doesn't carry it).
        let no_depth_test = std::fs::read_to_string(root.join(&e.file)).is_ok_and(|j| j.contains("\"bDisableDepthTest\": true"));
        Some(ParticleMaterial {
            params: ParticleParams { mode: Vec4::new(mode, if additive { 1.0 } else { 0.0 }, 0.0, 0.0), p },
            t1: t1.unwrap_or_else(|| t0.clone()),
            t2: t2.unwrap_or_else(|| t0.clone()),
            t3: t3.unwrap_or_else(|| t0.clone()),
            t0,
            sprites: NO_SPRITES,
            alpha,
            distort: false,
            stored: false,
            no_depth_test,
            rain: false,
        })
    }
}

/// The loaded systems and materials, and the next system's random seed.
#[derive(Resource)]
pub(crate) struct ParticleAssets {
    root: std::path::PathBuf,
    systems: Systems,
    mats: Materials,
    seed: u64,
    /// [`ChunkedMesh`]es' chunk meshes by mesh file (None when it can't be read).
    chunk_meshes: HashMap<String, Option<Arc<Vec<Handle<Mesh>>>>>,
}

impl ParticleAssets {
    /// A mesh's chunk meshes for [`ChunkedMesh`]: chunk k (1..4) keeps the vertices whose ceil(TexCoord[2].x) is k and
    /// moves the others to the origin, as the material's World Position Offset does.
    fn chunks(&mut self, file: &str, meshes: &mut Assets<Mesh>) -> Option<Arc<Vec<Handle<Mesh>>>> {
        let root = self.root.clone();
        self.chunk_meshes
            .entry(file.to_string())
            .or_insert_with(|| {
                let prims = f13_data::glb::read_primitives(&root.join(file))?;
                let p = prims.into_iter().next()?;
                let ids = p.uvs.get(2)?;
                let made = (1..=4)
                    .map(|k| {
                        let positions: Vec<[f32; 3]> = p.positions.iter().zip(ids).map(|(v, uv)| if uv[0].ceil() as i32 == k { *v } else { [0.0; 3] }).collect();
                        let mut m = Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::default()).with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, positions);
                        if p.normals.len() == p.positions.len() {
                            m.insert_attribute(Mesh::ATTRIBUTE_NORMAL, p.normals.clone());
                        }
                        if let Some(uv) = p.uvs.first() {
                            m.insert_attribute(Mesh::ATTRIBUTE_UV_0, uv.clone());
                        }
                        m.insert_indices(Indices::U32(p.indices.clone()));
                        meshes.add(m)
                    })
                    .collect();
                Some(Arc::new(made))
            })
            .clone()
    }

    /// A running system named `name` at an Unreal-space transform (cm, Z up), its emitters' entities spawned.
    #[allow(clippy::too_many_arguments)]
    fn instance(
        &mut self,
        commands: &mut Commands,
        meshes: &mut Assets<Mesh>,
        materials: &mut Assets<ParticleMaterial>,
        assets: &AssetServer,
        name: &str,
        (location, rotation, scale): (Vec3, Quat, Vec3),
        active: bool,
        actor: &str,
        max_draw: f32,
        warmup: Option<f32>,
    ) -> Option<ParticleSystemInstance> {
        let system = self.systems.get(&self.root, name)?;
        self.seed = self.seed.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut emitters = vec![];
        for def in &system.emitters {
            if let Some(md) = &def.mesh {
                // FDynamicMeshEmitterData: the mesh's own materials (bOverrideMaterial off); UE's default transform
                // only (CalculateParticleTransform without camera facing, axis lock or velocity alignment).
                let aligned = matches!(def.screen_alignment.as_str(), "PSA_Velocity" | "PSA_AwayFromCenter" | "PSA_TypeSpecific");
                // Its override material (bOverrideMaterial: the emitter's RequiredModule material), when ported.
                let ported = md.override_material && !(md.camera_facing || md.axis_lock != "EPAL_NONE" || aligned);
                if let Some(params) = def.material.as_deref().and_then(|m| self.mats.mesh_particle(m)).filter(|_| ported) {
                    if let Some(chunks) = self.chunks(&md.file, meshes) {
                        let cap = def.max_draw_count.unwrap_or(500).min(32) as usize;
                        let entity = commands.spawn((Transform::default(), Visibility::Hidden)).id();
                        let chunked = Some(ChunkedMesh { chunks, params, cap, materials: vec![] });
                        emitters.push(EmitterState { def: def.clone(), seconds: 0.0, spawn_fraction: 0.0, particles: vec![], entity, mesh: Handle::default(), pivot: Vec2::ZERO, bursts_fired: vec![false; def.bursts.len()], loops_done: 0, sphere: false, mesh_scene: None, slots: vec![], lights: vec![], drawn: true, attractor_last: [0; 2], collision_check: 0, last_source: None, unit_fraction: 0.0, material: None, distortion: None, chunked });
                        continue;
                    }
                }
                if md.override_material || md.camera_facing || md.axis_lock != "EPAL_NONE" || aligned || !self.root.join(&md.file).exists() {
                    info!("particles: {name}.{}: mesh emitter {} not drawn (override material, facing, axis lock or alignment, or not exported)", def.name, md.file);
                    continue;
                }
                let scene = assets.load(GltfAssetLabel::Scene(0).from_asset(md.file.clone()));
                let n = def.max_draw_count.unwrap_or(500).min(32) as usize;
                let entity = commands.spawn((Transform::default(), Visibility::Hidden)).id();
                emitters.push(EmitterState { def: def.clone(), seconds: 0.0, spawn_fraction: 0.0, particles: vec![], entity, mesh: Handle::default(), pivot: Vec2::ZERO, bursts_fired: vec![false; def.bursts.len()], loops_done: 0, sphere: false, mesh_scene: Some((scene, n)), slots: vec![], lights: vec![], drawn: true, attractor_last: [0; 2], collision_check: 0, last_source: None, unit_fraction: 0.0, material: None, distortion: None, chunked: None });
                continue;
            }
            let Some((material, mode)) = def.material.as_deref().and_then(|m| self.mats.get(m, &self.root, assets, materials)) else {
                // Not drawn, but still simulated when another emitter's ParticleModuleAttractorParticle names it, when it
                // sends events (an EventGenerator), or for its lights (a ParticleModuleLight).
                let named = system.emitters.iter().flat_map(|o| &o.modules).any(|m| matches!(m, Module::AttractorParticle { emitter, .. } if *emitter == def.name));
                let sends = def.modules.iter().any(|m| matches!(m, Module::EventGenerator(_) | Module::Light { .. }));
                if named || sends {
                    let entity = commands.spawn((Transform::default(), Visibility::Hidden)).id();
                    emitters.push(EmitterState { def: def.clone(), seconds: 0.0, spawn_fraction: 0.0, particles: vec![], entity, mesh: Handle::default(), pivot: Vec2::ZERO, bursts_fired: vec![false; def.bursts.len()], loops_done: 0, sphere: false, mesh_scene: None, slots: vec![], lights: light_pool(commands, def, name), drawn: false, attractor_last: [0; 2], collision_check: 0, last_source: None, unit_fraction: 0.0, material: None, distortion: None, chunked: None });
                }
                continue;
            };
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
                    MeshMaterial3d(material.clone()),
                    Transform::from_translation(to_bevy(location)),
                    Visibility::Hidden,
                    NoFrustumCulling,
                    NotShadowCaster,
                    NotShadowReceiver,
                    Name::new(format!("{name}.{}", def.name)),
                ))
                .id();
            let distortion = def.material.as_deref().and_then(|m| self.mats.distortion(m));
            if let Some(d) = &distortion {
                commands.spawn((Mesh3d(mesh.clone()), MeshMaterial3d(d.clone()), Transform::default(), Visibility::Inherited, NoFrustumCulling, NotShadowCaster, NotShadowReceiver, ChildOf(entity), Name::new(format!("{name}.{} distortion", def.name))));
            }
            let pivot = def.modules.iter().find_map(|m| if let Module::PivotOffset(p) = m { Some(Vec2::from(*p)) } else { None }).unwrap_or(Vec2::ZERO) + Vec2::splat(-0.5);
            emitters.push(EmitterState { def: def.clone(), seconds: 0.0, spawn_fraction: 0.0, particles: vec![], entity, mesh, pivot, bursts_fired: vec![false; def.bursts.len()], loops_done: 0, sphere: mode == 4, mesh_scene: None, slots: vec![], lights: light_pool(commands, def, name), drawn: true, attractor_last: [0; 2], collision_check: 0, last_source: None, unit_fraction: 0.0, material: Some(material), distortion, chunked: None });
        }
        if emitters.is_empty() {
            return None;
        }
        let mut inst = ParticleSystemInstance { emitters, location, rotation, scale, max_draw, rng: self.seed, active, actor: actor.to_string(), level: String::new(), parent: None, follow: None, auto_destroy: false, hidden: false, ticked_at: None, last_render: -1000.0, time_dilation: 1.0 };
        // WarmupTime: ticked in 0.032 s steps (UParticleSystemComponent's default warmup step).
        let warmup = warmup.unwrap_or(system.warmup);
        let mut t = 0.0;
        while active && t < warmup {
            tick(&mut inst, 0.032, None);
            t += 0.032;
        }
        Some(inst)
    }
}

/// An emitter's simple lights (its ParticleModuleLight's): MaxDrawCount point lights at most 16, hidden until drawn.
fn light_pool(commands: &mut Commands, def: &Emitter, name: &str) -> Vec<Entity> {
    if !def.modules.iter().any(|m| matches!(m, Module::Light { .. })) {
        return vec![];
    }
    let n = def.max_draw_count.unwrap_or(500).min(16) as usize;
    (0..n)
        .map(|i| commands.spawn((PointLight { intensity: 0.0, shadow_maps_enabled: false, ..default() }, Transform::default(), Visibility::Hidden, Name::new(format!("{name}.{} light {i}", def.name)))).id())
        .collect()
}

/// Spawns a system (UGameplayStatics::SpawnEmitterAtLocation: auto-destroyed once its emitters are done), or one that
/// follows an entity (a component on a runtime actor: the knife's trail, the cars' tyre dust), with an `actor` name
/// for [`ParticleActivation`].
#[derive(Message, Clone)]
pub struct SpawnParticles {
    pub system: String,
    /// Bevy space: where, or (following) the offset from the entity.
    pub transform: Transform,
    pub follow: Option<Entity>,
    pub actor: String,
    pub active: bool,
    /// The component's CustomTimeDilation (1 unless its Blueprint sets one).
    pub time_dilation: f32,
}

impl SpawnParticles {
    /// A weapon's impact effect (SCImpactEffect's spawn, FUN_14035e130 -> FUN_14033cbc0): SpawnEmitterAtLocation at
    /// the hit's ImpactPoint, turned to Conv_VectorToRotator(ImpactNormal) (pitch and yaw, no roll), scale 1. Bevy
    /// space in, the normal pointing out of the hit surface.
    pub fn impact(system: &str, point: Vec3, normal: Vec3) -> Self {
        let n = Vec3::new(normal.x, normal.z, normal.y);
        let rotator = Vec3::new(n.z.atan2((n.x * n.x + n.y * n.y).sqrt()).to_degrees(), n.y.atan2(n.x).to_degrees(), 0.0);
        let q = f13_core::camera_spline::quaternion(rotator);
        Self::at(system, Transform::from_translation(point).with_rotation(Quat::from_xyzw(-q.x, -q.z, -q.y, q.w)))
    }

    pub fn at(system: &str, transform: Transform) -> Self {
        SpawnParticles { system: system.to_string(), transform, follow: None, actor: String::new(), active: true, time_dilation: 1.0 }
    }
}

/// Testing (--particles): this system, spawned 1.5 m in front of the counselor's chest every 0.3 s, facing her.
#[derive(Resource)]
pub struct TestParticles(pub String);

fn test_particles(test: Option<Res<TestParticles>>, time: Res<Time>, player: Query<&Transform, (With<crate::player::Player>, With<crate::controls::LocalPlayer>)>, mut out: MessageWriter<SpawnParticles>, mut next: Local<f32>) {
    let Some(test) = test else { return };
    let Ok(xf) = player.single() else { return };
    if time.elapsed_secs() < *next {
        return;
    }
    *next = time.elapsed_secs() + 0.3;
    debug!("test particles: {}", test.0);
    let fwd = xf.forward().as_vec3();
    out.write(SpawnParticles::impact(&test.0, xf.translation + Vec3::Y * 1.2 + fwd * 1.5, -fwd));
}

/// Bevy space (m, Y up) to Unreal's (cm, Z up): the inverse of the export's glTF conversion.
fn to_unreal(t: &Transform) -> (Vec3, Quat, Vec3) {
    let p = t.translation * 100.0;
    let q = t.rotation;
    (Vec3::new(p.x, p.z, p.y), Quat::from_xyzw(-q.x, -q.z, -q.y, q.w), Vec3::new(t.scale.x, t.scale.z, t.scale.y))
}

/// Spawns the systems asked for.
fn spawn_requested(
    mut commands: Commands,
    mut requests: MessageReader<SpawnParticles>,
    mut pa: Option<ResMut<ParticleAssets>>,
    assets: Res<AssetServer>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<ParticleMaterial>>,
    globals: Query<&GlobalTransform>,
) {
    let Some(pa) = pa.as_mut() else { return };
    for r in requests.read() {
        let world = match r.follow {
            Some(e) => match globals.get(e) {
                Ok(g) => g.mul_transform(r.transform).compute_transform(),
                Err(_) => continue,
            },
            None => r.transform,
        };
        let Some(mut inst) = pa.instance(&mut commands, &mut meshes, &mut materials, &assets, &r.system, to_unreal(&world), r.active, &r.actor, 0.0, Some(0.0)) else { continue };
        inst.follow = r.follow.map(|e| (e, r.transform));
        inst.auto_destroy = r.follow.is_none();
        inst.time_dilation = r.time_dilation;
        commands.spawn((inst, Name::new(format!("particles {}", r.system))));
    }
}

/// Spawns the level's particle systems (inactive ones waiting for their [`ParticleActivation`]).
fn spawn_level_particles(
    mut commands: Commands,
    level: Option<Res<crate::Level>>,
    export: Res<crate::ExportRoot>,
    assets: Res<AssetServer>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<ParticleMaterial>>,
) {
    let root = export.0.clone();
    let entries: HashMap<String, MaterialEntry> = std::fs::read_to_string(root.join("materials.json")).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default();
    let mut pa = ParticleAssets { root, systems: Systems::default(), mats: Materials { entries, built: HashMap::new(), distortions: HashMap::new() }, seed: 0x9e37_79b9_7f4a_7c15, chunk_meshes: HashMap::new() };
    let mut n = 0;
    for a in level.iter().flat_map(|l| &l.layouts).flat_map(|l| &l.actors) {
        for c in &a.components {
            if c.kind != "ParticleSystemComponent" || c.hidden {
                continue;
            }
            let p = &c.props;
            if p["bVisible"].as_bool() == Some(false) || p["bHiddenInGame"].as_bool() == Some(true) {
                continue;
            }
            let active = p["bAutoActivate"].as_bool() != Some(false);
            let Some(name) = p["Template"].as_str().and_then(|t| t.split('\'').nth(1)) else { continue };
            let Some(world) = &c.world else { continue };
            let xf = (Vec3::from(world.t), Quat::from_array(world.r).normalize(), Vec3::from(world.s));
            let max_draw = p["LDMaxDrawDistance"].as_f64().unwrap_or(0.0) as f32 * crate::VIEW_DISTANCE_SCALE / 100.0;
            let warmup = p["WarmupTime"].as_f64().map(|w| w as f32);
            let Some(mut inst) = pa.instance(&mut commands, &mut meshes, &mut materials, &assets, name, xf, active, &a.name, max_draw, warmup) else { continue };
            inst.parent = c.parent.clone();
            inst.level = a.level.clone();
            inst.time_dilation = p["CustomTimeDilation"].as_f64().map_or(1.0, |d| d as f32);
            commands.spawn((inst, Name::new(format!("particles {name}"))));
            n += 1;
        }
    }
    info!("particles: {n} systems");
    commands.insert_resource(pa);
}

/// Systems that follow an entity take its transform; with it gone they stop spawning and go once done, as one-shots do.
fn follow_and_finish(mut commands: Commands, mut systems: Query<(Entity, &mut ParticleSystemInstance)>, globals: Query<&GlobalTransform>) {
    for (e, mut s) in &mut systems {
        if let Some((target, local)) = s.follow {
            match globals.get(target) {
                Ok(g) => {
                    let (location, rotation, scale) = to_unreal(&g.mul_transform(local).compute_transform());
                    (s.location, s.rotation, s.scale) = (location, rotation, scale);
                    for em in &s.emitters {
                        commands.entity(em.entity).insert(Transform::from_translation(to_bevy(location)));
                    }
                }
                Err(_) => {
                    s.follow = None;
                    s.active = false;
                    s.auto_destroy = true;
                }
            }
        }
        // Done: every emitter finished (its loops, or the system deactivated) and empty.
        let done = s.emitters.iter().all(|em| em.particles.is_empty() && (!s.active || (em.def.loops > 0 && em.loops_done >= em.def.loops)));
        if s.auto_destroy && done && s.emitters.iter().all(|em| em.seconds > 0.0) {
            for em in &s.emitters {
                commands.entity(em.entity).despawn();
                for &slot in em.slots.iter().chain(&em.lights) {
                    commands.entity(slot).despawn();
                }
            }
            commands.entity(e).despawn();
        }
    }
}

/// The cabins' roof drips follow the weather: SCCabin's BeginPlay (FUN_1403e6010) calls Cabin_Base_C's
/// SetRaining(the world settings' rain flag), which SetActive(bRaining)s each child of its Rain component.
fn rain_drips(weather: Option<Res<crate::weather::Weather>>, mut systems: Query<Mut<ParticleSystemInstance>>, mut seen: Local<Option<bool>>) {
    let Some(w) = weather else { return };
    let changed = *seen != Some(w.rain);
    *seen = Some(w.rain);
    for mut s in systems.iter_mut().filter(|s| s.parent.as_deref() == Some("Rain") && (changed || s.is_added())) {
        s.active = w.rain;
    }
}

/// The falling rain: SCPlayerCameraManager's RainGrid (PlayerCameraManager_BP's RainZOffset 1000, RainRayTraceTickDelay
/// 0.125, RainOcclusionHeight 100, RainGridSize 7, RainParticleSize 300, RainParticleSystem
/// P_Rain_Environmental_01A_GPU; RainResetDistance 500 from the ctor; MODLOG "Rain").
const RAIN_Z_OFFSET: f32 = 1000.0;
const RAIN_RESET_DISTANCE: f32 = 500.0;
const RAIN_TRACE_DELAY: f32 = 0.125;
const RAIN_OCCLUSION_HEIGHT: f32 = 100.0;
const RAIN_GRID_SIZE: i32 = 7;
const RAIN_PARTICLE_SIZE: f32 = 300.0;
const RAIN_SYSTEM: &str = "P_Rain_Environmental_01A_GPU";

/// One of the rain grid's systems: its offset from the camera (Unreal cm), its next trace time (game seconds) and
/// whether the last trace found it occluded; where it stands (Unreal cm) and when its emitter last started (game
/// seconds: Activate(true)'s reset clears its particles, bClearExistingParticlesOnInit).
#[derive(Component)]
struct RainCell {
    index: usize,
    offset: Vec3,
    next_trace: f32,
    occluded: bool,
    /// Deactivated by a reset, activated (reset) on the next tick.
    reactivate: bool,
    location: Vec3,
    start: f32,
    active: bool,
}

/// The camera location the grid was last moved for (0x1afc, zero at first).
#[derive(Default)]
struct RainGridState {
    on: bool,
    last: Vec3,
}

/// The rain grid's draw: every cell's GPU emitter (P_Rain_Environmental_01A_GPU's RainDrops: ParticleModuleTypeDataGpu,
/// no collision) simulated in the vertex shader (particles.wgsl RAIN_ANALYTIC) from the emitter's modules and each
/// cell's place and start, one sprite slot per particle a cell can have alive (rate x the longest life). Each particle
/// is a closed form of its age: spawned at start + n / rate (slot n mod slots), StartLocation, StartVelocity, StartSize
/// and Lifetime uniform per component, ParticleModuleAccelerationConstant, ColorOverLife and AlphaOverLife over its
/// relative time, in its cell's space (bUseLocalSpace) and velocity-aligned. PLACEHOLDER: the game's GPU simulation
/// integrates each frame (its curves from QuantizedColorSamples) and sorts by distance (PSORTMODE_DistanceToView);
/// here the motion is exact, the random numbers a hash, and a cell's sprites draw in slot order.
#[derive(Resource)]
struct RainGpu {
    buffer: Handle<ShaderBuffer>,
    entities: Vec<Entity>,
    header: Vec<[f32; 4]>,
    slots: usize,
}

/// [`RainGpu`]'s buffer: RAIN_HEADER vec4s of the emitter (the shader's layout), then two a cell.
const RAIN_HEADER: usize = 7 + RAIN_CURVE_SAMPLES;
const RAIN_CURVE_SAMPLES: usize = 64;

/// The emitter's header for [`RainGpu`], or why it can't be drawn so.
fn rain_header(def: &Emitter, pivot: Vec2) -> Result<(Vec<[f32; 4]>, usize), String> {
    let (mut life, mut loc, mut vel, mut size, mut accel, mut color) = (None, None, None, None, Vec3::ZERO, None);
    for m in &def.modules {
        match m {
            Module::Lifetime(d) => life = Some((d.eval(0.0, 0.0), d.eval(0.0, 1.0))),
            Module::Location(d) => loc = Some((d.eval3(0.0, [0.0; 3]), d.eval3(0.0, [1.0; 3]))),
            Module::Velocity { start, world: false, radial } if !radial.random() && radial.eval(0.0, 0.0) == 0.0 => vel = Some((start.eval3(0.0, [0.0; 3]), start.eval3(0.0, [1.0; 3]))),
            Module::Size(d) => size = Some((d.eval3(0.0, [0.0; 3]), d.eval3(0.0, [1.0; 3]))),
            Module::AccelerationConstant { accel: a, .. } => accel += *a,
            Module::ColorOverLife { color: c, alpha } if !c.random() && !alpha.random() => color = Some((c.clone(), alpha.clone())),
            other => return Err(format!("module {other:?}")),
        }
    }
    if !def.gpu || def.gpu_collision.is_some() || !def.local_space || def.screen_alignment != "PSA_Velocity" {
        return Err("not a local-space, velocity-aligned GPU emitter without collision".into());
    }
    let (Some(life), Some(loc), Some(vel), Some(size)) = (life, loc, vel, size) else { return Err("missing spawn modules".into()) };
    let rate = def.rate.eval(0.0, 0.0) * def.rate_scale.eval(0.0, 0.0);
    let slots = ((rate * life.0.max(life.1)).ceil() as usize + 1).min(def.max_draw_count.unwrap_or(u32::MAX) as usize);
    let mut h = vec![
        [0.0, rate, slots as f32, 0.0],
        [life.0, life.1, size.0.x, size.0.y],
        [loc.0.x, loc.0.y, loc.0.z, size.1.x],
        [loc.1.x, loc.1.y, loc.1.z, size.1.y],
        [vel.0.x, vel.0.y, vel.0.z, pivot.x],
        [vel.1.x, vel.1.y, vel.1.z, pivot.y],
        [accel.x, accel.y, accel.z, 0.0],
    ];
    for j in 0..RAIN_CURVE_SAMPLES {
        let t = j as f32 / (RAIN_CURVE_SAMPLES - 1) as f32;
        let (c, a) = color.as_ref().map_or((Vec3::ONE, 1.0), |(c, a)| (c.eval3(t, [0.0; 3]), a.eval(t, 0.0)));
        h.push([c.x, c.y, c.z, a]);
    }
    Ok((h, slots))
}

/// OnRainUpdatedInWorldSettings (0x1403fb480) and the tick's grid update (FUN_140410ba0). On: GridSize^2 systems,
/// cell (i, j) at ((0.5 - N/2 + i) x size, (0.5 - N/2 + j) x size, RainZOffset) from the camera, each first traced at
/// once (its next trace time -RainRayTraceTickDelay). Off (FUN_1403e9fb0): the grid destroyed. Each tick, only once
/// the camera has moved half a cell (RainParticleSize x 0.5) in the plane since the last update: every cell due a
/// trace (delay <= 1e-4, or its time passed) moves to camera + offset; moved more than RainResetDistance, it's
/// Deactivated and Activate(true)d on the next tick (a timer for the next tick); its next trace is (1 + rand) x the
/// delay away; then a trace on the Camera channel from its inner corner (the offset's x and y each moved half a cell
/// towards the camera) down to 100 cm under the camera: a hit more than RainOcclusionHeight from the camera's height
/// occludes it, a nearer hit occludes it if a second trace from 1 cm above the hit to the camera hits too. A cell
/// whose occlusion changes SetVisibility(!occluded). Drawn by [`RainGpu`].
#[allow(clippy::too_many_arguments)]
fn rain_grid(
    mut commands: Commands,
    weather: Option<Res<crate::weather::Weather>>,
    pa: Option<ResMut<ParticleAssets>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<ParticleMaterial>>,
    mut buffers: ResMut<Assets<ShaderBuffer>>,
    assets: Res<AssetServer>,
    camera: Query<&GlobalTransform, (With<Camera3d>, With<crate::lighting::Lit>)>,
    mut cells: Query<(Entity, &mut RainCell)>,
    world: Option<Res<crate::collision::WorldCollision>>,
    dynamic: Option<Res<crate::collision::DynamicColliders>>,
    blockers: Query<(), With<crate::collision::CameraBlocker>>,
    time: Res<Time>,
    gpu: Option<Res<RainGpu>>,
    mut state: Local<RainGridState>,
) {
    let (Some(w), Some(mut pa)) = (weather, pa) else { return };
    let now = time.elapsed_secs();
    if w.rain != state.on {
        state.on = w.rain;
        if w.rain {
            let root = pa.root.clone();
            let Some(system) = pa.systems.get(&root, RAIN_SYSTEM) else { return };
            let Some(def) = system.emitters.first() else { return };
            let pivot = def.modules.iter().find_map(|m| if let Module::PivotOffset(p) = m { Some(Vec2::from(*p)) } else { None }).unwrap_or(Vec2::ZERO) + Vec2::splat(-0.5);
            let (header, slots) = match rain_header(def, pivot) {
                Ok(h) => h,
                Err(e) => {
                    warn!("rain: {RAIN_SYSTEM} can't be drawn on the GPU ({e})");
                    return;
                }
            };
            let Some((base, _)) = def.material.as_deref().and_then(|m| pa.mats.get(m, &root, &assets, &mut materials)) else { return };
            let n = RAIN_GRID_SIZE.max(1);
            let count = (n * n) as usize;
            let buffer = buffers.add(ShaderBuffer::new(&vec![0u8; (RAIN_HEADER + 2 * count) * 16], RenderAssetUsages::default()));
            let quads = meshes.add(sprite_quads(count * slots));
            let mut entities = vec![];
            for m in std::iter::once(Some(base)).chain(std::iter::once(def.material.as_deref().and_then(|m| pa.mats.distortion(m)))).flatten() {
                let Some(base) = materials.get(&m).cloned() else { continue };
                let material = materials.add(ParticleMaterial { sprites: buffer.clone(), stored: true, rain: true, ..base });
                entities.push(commands.spawn((Mesh3d(quads.clone()), MeshMaterial3d(material), Transform::default(), Visibility::Visible, NoFrustumCulling, NotShadowCaster, NotShadowReceiver, Name::new("rain (GPU)"))).id());
            }
            let first = (0.5 - n as f32 * 0.5) * RAIN_PARTICLE_SIZE;
            for i in 0..n {
                for j in 0..n {
                    let offset = Vec3::new(first + i as f32 * RAIN_PARTICLE_SIZE, first + j as f32 * RAIN_PARTICLE_SIZE, RAIN_Z_OFFSET);
                    let index = (i * n + j) as usize;
                    commands.spawn((RainCell { index, offset, next_trace: -RAIN_TRACE_DELAY, occluded: false, reactivate: false, location: offset, start: now, active: true }, Name::new(format!("rain {i},{j}"))));
                }
            }
            info!("rain: {count} cells on the GPU, {slots} sprites each, {} draws", entities.len());
            commands.insert_resource(RainGpu { buffer, entities, header, slots });
        } else {
            for (e, _) in &cells {
                commands.entity(e).despawn();
            }
            if let Some(g) = gpu {
                for &e in &g.entities {
                    commands.entity(e).despawn();
                }
            }
            commands.remove_resource::<RainGpu>();
        }
        return;
    }
    if !state.on {
        return;
    }
    for (_, mut c) in &mut cells {
        if std::mem::take(&mut c.reactivate) {
            c.start = now;
            c.active = true;
        }
    }
    let cam = camera.iter().next().map(|c| to_unreal(&c.compute_transform()).0);
    if let Some(cam) = cam.filter(|cam| (state.last.truncate() - cam.truncate()).length() >= RAIN_PARTICLE_SIZE * 0.5) {
        let moved = (state.last.truncate() - cam.truncate()).length();
        let half = RAIN_PARTICLE_SIZE * 0.5;
        let lists: Vec<&f13_core::collision::TriList> = dynamic.iter().flat_map(|d| d.iter()).filter(|(e, _)| blockers.contains(*e)).map(|(_, l)| l).collect();
        let hit = |from: Vec3, to: Vec3| -> Option<Vec3> {
            let world = world.as_deref()?;
            let f = f13_core::collision::segment_hit(&(&*world.camera_grid, lists.as_slice()), to_bevy(from), to_bevy(to), 4.0)?;
            Some(from + (to - from) * f)
        };
        for (_, mut c) in &mut cells {
            if RAIN_TRACE_DELAY > 1e-4 && c.next_trace > now {
                continue;
            }
            let mut at = cam + c.offset;
            c.location = at;
            if moved > RAIN_RESET_DISTANCE {
                c.active = false;
                c.reactivate = true;
            }
            if 1e-4 < RAIN_TRACE_DELAY {
                c.next_trace = (rand(&mut pa.seed) + 1.0) * RAIN_TRACE_DELAY + now;
            }
            c.occluded = false;
            for k in 0..2 {
                if c.offset[k].abs() > 1e-4 {
                    at[k] += if c.offset[k] >= 0.0 { -half } else { half };
                }
            }
            let end = at - Vec3::Z * (RAIN_Z_OFFSET + 100.0);
            if let Some(h) = hit(at, end) {
                c.occluded = (h.z - cam.z).abs() > RAIN_OCCLUSION_HEIGHT || hit(h + Vec3::Z, cam).is_some();
            }
        }
        state.last = cam;
    }
    // The buffer: the emitter, the clock, each cell's place, start and whether it draws.
    let Some(g) = gpu else { return };
    let mut data = g.header.clone();
    data[0][0] = now;
    data[0][3] = cells.iter().len() as f32;
    data.resize(RAIN_HEADER + 2 * cells.iter().len(), [0.0; 4]);
    for (_, c) in &cells {
        let i = RAIN_HEADER + 2 * c.index;
        if i + 1 < data.len() {
            data[i] = [c.location.x, c.location.y, c.location.z, c.start];
            data[i + 1] = [if c.active && !c.occluded { 1.0 } else { 0.0 }, 0.0, 0.0, 0.0];
        }
    }
    if let Some(mut b) = buffers.get_mut(&g.buffer) {
        let bytes: Vec<u8> = data.iter().flatten().flat_map(|f| f.to_le_bytes()).collect();
        if b.data.as_ref().is_some_and(|d| d.len() == bytes.len()) {
            b.data = Some(bytes);
        }
    }
}

/// Rain_MPC's Rain into the rain materials (M_Rain_Master's opacity).
fn rain_mpc_materials(mpc: Option<Res<crate::weather::RainMpc>>, mut materials: ResMut<Assets<ParticleMaterial>>) {
    // Each rain material as it's made too (they come after the weather's first write).
    let Some(mpc) = mpc else { return };
    let rain: Vec<_> = materials.iter().filter(|(_, m)| (m.params.mode.x == 7.0 || m.params.mode.x == 30.0) && m.params.p[15].x != mpc.rain).map(|(id, _)| id).collect();
    for id in rain {
        if let Some(mut m) = materials.get_mut(id) {
            m.params.p[15].x = mpc.rain;
        }
    }
}

/// A runtime actor's systems stop (Deactivate) and lose their mesh particles at once (a thrown knife stuck: its
/// spinning dart).
#[derive(Message, Clone)]
pub struct StopMeshParticles {
    pub actor: String,
}

fn stop_mesh_particles(mut messages: MessageReader<StopMeshParticles>, mut systems: Query<&mut ParticleSystemInstance>) {
    for m in messages.read() {
        for mut s in systems.iter_mut().filter(|s| s.actor == m.actor) {
            s.active = false;
            for e in &mut s.emitters {
                if e.mesh_scene.is_some() || e.chunked.is_some() {
                    e.particles.clear();
                }
            }
        }
    }
}

/// Activations by actor.
fn activate(mut messages: MessageReader<ParticleActivation>, mut systems: Query<&mut ParticleSystemInstance>) {
    for m in messages.read() {
        for mut s in systems.iter_mut().filter(|s| s.actor == m.actor) {
            // Activate(true) on an inactive system: from the start (the sequences' Activate keys on one that's running
            // leave it be).
            if m.active && m.reset && !s.active {
                reset(&mut s);
            }
            s.active = m.active;
        }
    }
}

/// Activate(true)'s ResetParticles: every emitter from the start.
fn reset(s: &mut ParticleSystemInstance) {
    for e in &mut s.emitters {
        e.particles.clear();
        e.seconds = 0.0;
        e.spawn_fraction = 0.0;
        e.loops_done = 0;
        e.bursts_fired.iter_mut().for_each(|b| *b = false);
    }
}

/// Ticks every system (game time; on during cinematics too, as the engine's).
fn simulate(
    time: Res<Time<Real>>,
    virtual_time: Res<Time<Virtual>>,
    mut systems: Query<&mut ParticleSystemInstance>,
    collision: Option<Res<crate::collision::WorldCollision>>,
    dynamic: Option<Res<crate::collision::DynamicColliders>>,
    cameras: Query<&GlobalTransform, (With<Camera3d>, With<crate::lighting::Lit>)>,
) {
    let dt = (time.delta_secs() * virtual_time.relative_speed()).min(0.1);
    if dt <= 0.0 {
        return;
    }
    // ParticleModuleCollision's traces against the world's collision (Unreal space in and out).
    let world = collision.as_deref().zip(dynamic.as_deref()).map(|(c, d)| c.with(d));
    let trace = |from: Vec3, to: Vec3| -> Option<(f32, Vec3)> {
        let w = world.as_ref()?;
        let (t, n) = f13_core::collision::segment_hit_normal(w, to_bevy(from), to_bevy(to), 2.0)?;
        Some((t, Vec3::new(n.x, n.z, n.y)))
    };
    // The players' view locations, and the world's bDropDetail: PLACEHOLDER, UE 4.18 UWorld::Tick's rule (DeltaSeconds
    // > 1 / clamp(MinDesiredFrameRate, 1, 100); BaseEngine.ini 35), not traced in the binary.
    let views: Vec<Vec3> = cameras.iter().map(|c| to_unreal_point(c.translation())).collect();
    let drop_detail = dt > 1.0 / 35.0;
    let now = virtual_time.elapsed_secs();
    for mut s in &mut systems {
        let unrendered = now - s.last_render > 0.1;
        let dt = dt * s.time_dilation;
        tick(&mut s, dt, Some((&trace, views.as_slice(), drop_detail, unrendered)));
    }
}

/// A Bevy point to Unreal's space (cm).
fn to_unreal_point(p: Vec3) -> Vec3 {
    Vec3::new(p.x, p.z, p.y) * 100.0
}

fn tick(s: &mut ParticleSystemInstance, dt: f32, world: Option<(Trace, &[Vec3], bool, bool)>) {
    let (location, rotation, scale, active) = (s.location, s.rotation, s.scale, s.active);
    // The component's move since its last tick (ParticleModuleSourceMovement).
    let moved = s.ticked_at.map_or(Vec3::ZERO, |(l, _)| location - l);
    // The attracting emitters' particles as they stand before this tick (each AttractorParticle reads its source
    // emitter's particles by index; the sources here come after the emitters they attract, as in the assets).
    let mut sources: HashMap<String, Vec<(Vec3, Vec3, f32)>> = HashMap::new();
    for e in &s.emitters {
        for m in &e.def.modules {
            if let Module::AttractorParticle { emitter, .. } = m {
                if let Some(src) = s.emitters.iter().find(|o| o.def.name == *emitter) {
                    sources.entry(emitter.clone()).or_insert_with(|| src.particles.iter().map(|p| (p.location, p.velocity, p.relative_time)).collect());
                }
            }
        }
    }
    // Local-space emitters: their particles move with the component.
    if let Some((was_at, was_rot)) = s.ticked_at.filter(|&(l, r)| l != location || r != rotation) {
        let turn = rotation * was_rot.inverse();
        for e in s.emitters.iter_mut().filter(|e| e.def.local_space) {
            for p in &mut e.particles {
                p.location = location + turn * (p.location - was_at);
                p.velocity = turn * p.velocity;
                p.base_velocity = turn * p.base_velocity;
            }
        }
    }
    s.ticked_at = Some((location, rotation));
    let bounds = s.emitters.iter().flat_map(|e| e.particles.iter()).fold(None, |b: Option<(Vec3, Vec3)>, p| Some(b.map_or((p.location, p.location), |(lo, hi)| (lo.min(p.location), hi.max(p.location)))));
    let collider = world.map(|(trace, views, drop_detail, unrendered)| Collider { trace, views, drop_detail, unrendered, bounds });
    let mut rng = s.rng;
    let mut events = vec![];
    for e in &mut s.emitters {
        tick_emitter(e, dt, location, rotation, scale, active, &mut rng, &sources, moved, collider.as_ref(), &mut events);
    }
    // The events, once every emitter has ticked: each receiver module's (UParticleModuleEventReceiverSpawn::
    // ProcessParticleEvent 0x1415da050) ForceSpawn of round(SpawnCount) at the event (or the system), with the
    // event's velocity x InheritVelocityScale when bInheritVelocity; the count at the emitter's time, or the
    // particle's for a death event with bUseParticleTime.
    if !events.is_empty() {
        for e in &mut s.emitters {
            let def = e.def.clone();
            for m in &def.modules {
                let Module::EventReceiverSpawn { kind, name, count, particle_time, psys_location, inherit, inherit_scale } = m else { continue };
                for ev in events.iter().filter(|ev| ev.name == *name && (*kind == 0 || *kind == ev.kind)) {
                    let at_time = if ev.kind == 2 && *particle_time { ev.particle_time } else { ev.emitter_time };
                    let n = count.eval(at_time, rand(&mut rng)).round().max(0.0) as u32;
                    let velocity = if *inherit { inherit_scale.eval3(at_time, rand3(&mut rng)) * ev.velocity } else { Vec3::ZERO };
                    let at = if *psys_location { location } else { ev.location };
                    for _ in 0..n {
                        let mut p = spawn(&def, 0.0, e.seconds, at, rotation, scale, &mut rng);
                        p.velocity += velocity;
                        p.base_velocity += velocity;
                        e.particles.push(p);
                    }
                }
            }
        }
    }
    s.rng = rng;
}

#[allow(clippy::too_many_arguments)]
fn tick_emitter(
    e: &mut EmitterState,
    dt: f32,
    location: Vec3,
    rotation: Quat,
    scale: Vec3,
    active: bool,
    rng: &mut u64,
    sources: &HashMap<String, Vec<(Vec3, Vec3, f32)>>,
    moved: Vec3,
    collider: Option<&Collider>,
    events: &mut Vec<ParticleEvent>,
) {
    let def = &e.def;
    // Tick_EmitterTimeSetup.
    let old_seconds = e.seconds;
    e.seconds += dt;
    let duration = def.duration.max(0.0);
    let mut emitter_time = e.seconds;
    let _ = old_seconds;
    // SetupEmitterDuration (0x1415aaa40): a loop lasts EmitterDuration + the delay (EmitterDelay; less the delay after
    // the first loop with bDelayFirstLoopOnly on a looping emitter); the emitter's time is the time into the loop less
    // the delay (none after the first loop with bDelayFirstLoopOnly).
    let delay_once = def.delay_first_loop_only && def.loops != 1;
    let looped = |n: u32| duration + if delay_once && n >= 1 { 0.0 } else { def.delay };
    let length = looped(e.loops_done);
    if length > 1e-4 {
        // Tick_EmitterTimeSetup: the time modulo the loop's length; a loop counted (once a tick) when the time since
        // creation passes length x the loops so far.
        emitter_time = e.seconds % length;
        if e.seconds - length * e.loops_done as f32 >= length {
            e.loops_done += 1;
            e.bursts_fired.iter_mut().for_each(|b| *b = false);
        }
    }
    let finished = !active || (def.loops > 0 && duration > 0.0 && e.loops_done >= def.loops);
    let delay = if def.delay_first_loop_only && e.loops_done > 0 { 0.0 } else { def.delay };
    emitter_time -= delay;
    // KillParticles (an EventGenerator's death events for each: UParticleModuleEventGenerator::HandleParticleKilled;
    // every Frequency-th, 1 in every emitter here).
    let deaths: Vec<&String> = def.modules.iter().flat_map(|m| if let Module::EventGenerator(evs) = m { evs.iter().filter(|e| e.0 == 2).map(|e| &e.1).collect() } else { vec![] }).collect();
    if !deaths.is_empty() {
        for p in e.particles.iter().filter(|p| p.relative_time > 1.0) {
            for name in &deaths {
                events.push(ParticleEvent { kind: 2, name: (*name).clone(), location: p.location, velocity: p.velocity, emitter_time: e.seconds, particle_time: p.relative_time });
            }
        }
    }
    e.particles.retain(|p| p.relative_time <= 1.0);
    // ResetParticleParameters.
    for p in &mut e.particles {
        p.velocity = p.base_velocity;
        p.size = p.base_size;
        p.rotation_rate = p.base_rotation_rate;
        p.color = p.base_color;
        p.relative_time += p.one_over_max_lifetime * dt;
    }
    // FParticleMeshEmitterInstance::Tick (bMeshRotationActive): RotationRate from its base, Rotation = InitRotation +
    // CurContinuousRotation (before the modules; the continuous rotation advances after them).
    if def.mesh.is_some() {
        for p in &mut e.particles {
            p.mesh_rate = p.mesh_rate_base;
            p.mesh_rotation = p.mesh_init_rotation + p.mesh_continuous;
        }
    }
    // The modules' updates.
    let mut attractor_k = 0;
    for m in &def.modules {
        match m {
            Module::AttractorPoint { position, range, strength, flags } => {
                attract_point(&mut e.particles, def.local_space, location, rotation, scale, emitter_time, dt, position, range, strength, *flags, rng);
            }
            Module::AttractorParticle { emitter, range, strength, by_distance, affect_base, inherit, .. } => {
                let k = attractor_k.min(1);
                attractor_k += 1;
                let src = sources.get(emitter).map(Vec::as_slice).unwrap_or(&[]);
                for p in &mut e.particles {
                    attract_particle(p, k, src, range, strength, *by_distance, *affect_base, *inherit, dt);
                }
            }
            Module::Collision { completion, only_vertical, vertical_fudge, dir_scalar, drop_detail, only_if_visible, max_distance, .. } => {
                // UParticleModuleCollision::Update (0x1415e4920)'s culls: none with bDropDetail on a dropping world; with
                // bCollideOnlyIfVisible on a system unrendered for 0.1 s, or (a MaxCollisionDistance under 2097152 and 8
                // or more particles, every 31st update) bounds no view comes within MaxCollisionDistance x 1.075 of,
                // every particle not frozen stops colliding for good (IgnoreCollisions).
                let Some(c) = collider.filter(|c| !(*drop_detail && c.drop_detail)) else { continue };
                let culls = *max_distance < 2_097_152.0;
                let mut ignore = *only_if_visible && c.unrendered;
                if !ignore && culls && e.particles.len() > 7 {
                    if e.collision_check == 0 {
                        let r = Vec3::splat(*max_distance * 1.075);
                        ignore = c.bounds.is_some_and(|(lo, hi)| !c.views.iter().any(|v| v.cmpgt(lo - r).all() && v.cmplt(hi + r).all()));
                    }
                    e.collision_check = if e.collision_check >= 30 { 0 } else { e.collision_check + 1 };
                }
                if ignore {
                    for p in e.particles.iter_mut().filter(|p| p.state & FREEZE == 0) {
                        p.state |= IGNORE_COLLISIONS;
                    }
                    continue;
                }
                let views = culls.then_some((c.views, *max_distance));
                for p in &mut e.particles {
                    collide(p, dt, scale, *completion, *only_vertical, *vertical_fudge, *dir_scalar, c.trace, views);
                }
            }
            _ => {
                // BEGIN_UPDATE_LOOP skips frozen particles.
                for p in e.particles.iter_mut().filter(|p| p.state & FREEZE == 0) {
                    update(m, p, dt, emitter_time);
                }
            }
        }
    }
    // Spawn.
    let alive_before = e.particles.len();
    if !finished && emitter_time >= 0.0 {
        let rate = def.rate.eval(emitter_time, rand(rng)) * def.rate_scale.eval(emitter_time, rand(rng)).max(0.0);
        let mut count = 0;
        let mut increment = 0.0;
        let mut start = 0.0;
        if rate > 0.0 {
            let leftover = e.spawn_fraction + dt * rate;
            count = if leftover.is_finite() { leftover.floor().max(0.0) as u32 } else { 0 };
            increment = 1.0 / rate;
            start = dt + e.spawn_fraction * increment - increment;
            e.spawn_fraction = if leftover.is_finite() { leftover - leftover.floor() } else { 0.0 };
            // (PLACEHOLDER: the same guard on the rate's spawns.)
            if count > MAX_SPAWN_PER_TICK || !leftover.is_finite() {
                warn!("particles: {} wanted {leftover} spawned this tick (rate {rate}, dt {dt}): capped", def.name);
                count = count.min(MAX_SPAWN_PER_TICK);
            }
        }
        // ParticleModuleSpawnPerUnit along the source's move this tick: (distance / UnitScalar) x SpawnPerUnit, the
        // fraction carried over, spaced along the move (a ribbon's points: FUN_1416098c0 spawns each at its share of
        // the way, ResolveSource FUN_1415fc7f0). PLACEHOLDER: GetSpawnPerUnitAmount's MovementTolerance and
        // bIgnoreSpawnRateWhenMoving aren't read; the source is the component (the knife's trail names an actor
        // parameter, ProjMesh, that nothing sets).
        let mut along: Vec<Vec3> = vec![];
        if let Some((unit, per_unit)) = &def.spawn_per_unit {
            let from = e.last_source.unwrap_or(location);
            let units = from.distance(location) / unit.max(1e-3) * per_unit.eval(emitter_time, rand(rng)) + e.unit_fraction;
            // PLACEHOLDER: at most MAX_SPAWN_PER_TICK a tick, none for a non-finite amount (its source teleported or
            // broke; UE's MaxFrameDistance, in the export, isn't read): an unbounded amount once allocated GBs and hung
            // the network tests' servers.
            let n = if units.is_finite() { units.floor().max(0.0) as u32 } else { 0 };
            if n > MAX_SPAWN_PER_TICK || !units.is_finite() {
                warn!("particles: {} wanted {units} spawned per unit (moved {:.0} from {from} to {location}): capped", def.name, from.distance(location));
            }
            let n = n.min(MAX_SPAWN_PER_TICK);
            e.unit_fraction = if units.is_finite() { units - units.floor() } else { 0.0 };
            along = (1..=n).map(|i| from.lerp(location, i as f32 / n as f32)).collect();
        }
        e.last_source = Some(location);
        for (i, at) in along.iter().enumerate() {
            let spawn_time = dt * (1.0 - (i + 1) as f32 / along.len() as f32);
            let mut p = spawn(def, spawn_time, emitter_time, *at, rotation, scale, rng);
            // Trails_SourceUp: the component's up, scaled (the spawn's rotate of (0, 0, scale.z)).
            p.ribbon_up = rotation * Vec3::new(0.0, 0.0, scale.z);
            e.particles.push(p);
        }
        if let Some(max) = def.ribbon.as_ref().map(|r| r.max_particles as usize).filter(|&m| m > 0) {
            let over = e.particles.len().saturating_sub(max);
            e.particles.drain(..over);
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
            let mut p = spawn(def, spawn_time.max(0.0), emitter_time, location, rotation, scale, rng);
            // UParticleModuleAttractorParticle::Spawn (0x1415adf20): the module's LastSelIndex if the attracting
            // emitter has a particle there (then the next one), else a new pick (random: SRand x its active count;
            // sequential: its first particle).
            let mut k = 0;
            for m in &def.modules {
                if let Module::AttractorParticle { emitter, random, .. } = m {
                    let k_ = k.min(1);
                    k += 1;
                    let n = sources.get(emitter).map_or(0, Vec::len) as i32;
                    let last = &mut e.attractor_last[k_];
                    if *last >= 0 && *last < n {
                        p.attractor[k_] = (*last, Vec3::ZERO);
                        *last += 1;
                    } else {
                        let pick = if *random { (rand(rng) * n as f32) as i32 } else { 0 };
                        *last = pick;
                        let vel = sources.get(emitter).and_then(|v| v.get(pick as usize)).map_or(Vec3::ZERO, |s| s.1);
                        p.attractor[k_] = (pick, if !*random && n > 0 { vel } else { Vec3::ZERO });
                    }
                }
            }
            e.particles.push(p);
        }
    }
    // Spawn events for the particles spawned this tick (HandleParticleSpawned: each one, Frequency 1 here).
    let spawns: Vec<&String> = def.modules.iter().flat_map(|m| if let Module::EventGenerator(evs) = m { evs.iter().filter(|e| e.0 == 1).map(|e| &e.1).collect() } else { vec![] }).collect();
    if !spawns.is_empty() {
        for p in e.particles.iter().skip(alive_before) {
            for name in &spawns {
                events.push(ParticleEvent { kind: 1, name: (*name).clone(), location: p.location, velocity: p.velocity, emitter_time: e.seconds, particle_time: p.relative_time });
            }
        }
    }
    // The FinalUpdate modules: ParticleModuleSourceMovement (0x14159bc20) moves each particle older than two ticks by
    // its scale (at its relative time) times the component's move since the last tick.
    for m in &def.modules {
        if let Module::SourceMovement(d) = m {
            for p in &mut e.particles {
                if 2.0 * dt * p.one_over_max_lifetime < p.relative_time {
                    p.location += d.eval3(p.relative_time, [0.0; 3]) * moved;
                }
            }
        }
    }
    // A GPU emitter's collision (its simulation pass's, before it integrates).
    if let (Some(gc), Some(c)) = (&def.gpu_collision, collider) {
        for p in e.particles.iter_mut().filter(|p| p.state & FREEZE == 0) {
            gpu_collide(p, dt, scale, gc, c.trace);
        }
    }
    // UpdateBoundingBox: the linear integrator.
    for p in &mut e.particles {
        if p.state & (FREEZE | FREEZE_TRANSLATION) == 0 {
            p.location += p.velocity * dt;
        }
        if p.state & (FREEZE | FREEZE_ROTATION) == 0 {
            p.rotation += p.rotation_rate * dt;
        }
    }
    if def.mesh.is_some() {
        for p in &mut e.particles {
            p.mesh_continuous += p.mesh_rate * dt;
        }
    }
}

/// UParticleModuleCollision::Update (0x1415e4920) for one particle, skipping frozen, halted or held ones (a delay
/// passes once its relative time reaches it): a trace from where it is to where it's going, pushed on by its size
/// (x the component's scale) / DirScalar along the way; a hit counts down its collisions (with
/// bOnlyVerticalNormalsDecrementCount, only hits on a normal within VerticalFudgeFactor of vertical); out of them it
/// takes the completion option there (killed, frozen, collisions halted, translation, rotation or both frozen); else
/// its velocity and base velocity mirror on the normal x the damping factor (per axis) and its rotation rate scales by
/// the rotation damping's X, at the hit. With `views` (MaxCollisionDistance under 2097152), a particle whose trace end
/// is no view's MaxCollisionDistance away (x the view's LOD distance factor, 1 here) stops colliding for good instead.
/// The physics impulses (bApplyPhysics: ParticleMass at its RelativeTime along -its velocity, on a hit component that
/// simulates) have nothing to act on: the traces here hit only the static world.
#[allow(clippy::too_many_arguments)]
fn collide(p: &mut Particle, dt: f32, scale: Vec3, completion: u8, only_vertical: bool, vertical_fudge: f32, dir_scalar: f32, trace: Trace, views: Option<(&[Vec3], f32)>) {
    if p.state & (FREEZE | IGNORE_COLLISIONS | FREEZE_TRANSLATION | FREEZE_ROTATION) != 0 {
        return;
    }
    if p.state & DELAY_COLLISIONS != 0 {
        if p.relative_time < p.collision.3 {
            return;
        }
        p.state &= !DELAY_COLLISIONS;
    }
    let from = p.location;
    let to = from + p.velocity * dt;
    let dir = (to - from).normalize_or_zero();
    let end = to + dir * (scale * p.size) / dir_scalar.max(1e-3);
    if let Some((views, max)) = views {
        if !views.iter().any(|v| v.distance_squared(end) < max * max) {
            p.state |= IGNORE_COLLISIONS;
            return;
        }
    }
    let Some((t, n)) = trace(from, end) else { return };
    let hit = from + (end - from) * t;
    let vertical = n.abs().max_element() <= 1e-4 || n.z.abs() + vertical_fudge >= 1.0;
    if !only_vertical || vertical {
        p.collision.2 -= 1;
    }
    p.location = hit;
    if p.collision.2 < 1 {
        p.state |= match completion {
            0 => {
                p.relative_time = 2.0;
                0
            }
            1 => FREEZE,
            2 => IGNORE_COLLISIONS,
            3 => FREEZE_TRANSLATION,
            4 => FREEZE_ROTATION,
            _ => FREEZE_TRANSLATION | FREEZE_ROTATION,
        };
        return;
    }
    let mirror = |v: Vec3| v - 2.0 * v.dot(n) * n;
    p.velocity = mirror(p.velocity) * p.collision.0;
    p.base_velocity = mirror(p.base_velocity) * p.collision.0;
    p.rotation_rate *= p.collision.1.x;
    p.base_rotation_rate *= p.collision.1.x;
}

/// UParticleModuleAttractorPoint::Update (0x1415b2a70): Position at the emitter time (a random pick each tick for a
/// uniform distribution), in world space unless the emitter is local or bUseWorldSpacePosition; Range scaled by the
/// component's scale vector's length (that of (1, 1, 1) when not transformed); within it, the strength (by
/// (range - distance) / range with bStrengthByDistance, else at the emitter time; also scaled when transformed),
/// toward the point (away and positive if negative), each axis clamped by its Positive_ / Negative_ flags, x dt onto
/// the velocity (and the base velocity with bAffectBaseVelocity). Particles are kept in world space here.
#[allow(clippy::too_many_arguments)]
fn attract_point(particles: &mut [Particle], local: bool, location: Vec3, rotation: Quat, scale: Vec3, emitter_time: f32, dt: f32, position: &Dist, range: &Dist, strength: &Dist, flags: u32, rng: &mut u64) {
    let world_flag = flags & 8 != 0;
    let transformed = !local && !world_flag;
    let pos = position.eval3(emitter_time, rand3(rng));
    let pos = if world_flag { pos } else { location + rotation * (scale * pos) };
    let scale_len = if transformed { scale.length() } else { Vec3::ONE.length() };
    let range = range.eval(emitter_time, rand(rng)) * scale_len;
    let bound = |pos_bit: u32, neg_bit: u32| (if flags & 1 << neg_bit != 0 { -1.0 } else { 0.0 }, if flags & 1 << pos_bit != 0 { 1.0 } else { 0.0 });
    let bounds = [bound(4, 7), bound(5, 8), bound(6, 9)];
    for p in particles {
        let dir = pos - p.location;
        let d2 = dir.length_squared();
        let d = d2.sqrt();
        if d > range {
            continue;
        }
        let mut s = if flags & 1 != 0 {
            if range != 0.0 { strength.eval((range - d) / range, rand(rng)) } else { 0.0 }
        } else {
            strength.eval(emitter_time, rand(rng))
        };
        if transformed {
            s *= scale_len;
        }
        let mut n = if d2 > 1e-8 { dir / d } else { dir };
        if s < 0.0 {
            n = -n;
            s = -s;
        }
        for (i, (lo, hi)) in bounds.iter().enumerate() {
            n[i] = n[i].max(*lo).min(*hi);
        }
        let delta = n * s * dt;
        p.velocity += delta;
        if flags & 2 != 0 {
            p.base_velocity += delta;
        }
    }
}

/// UParticleModuleAttractorParticle::Update (0x1415b2480) for one particle: its source (the attracting emitter's
/// particle at its index, as that emitter stood before this tick) pulls it when within Range (at the source's relative
/// time), by Strength at (range - distance) / range (bStrengthByDistance) or at the source's relative time, x dt onto
/// the velocity (and the base velocity with bAffectBaseVelocity); the source's velocity is kept. A lost source (index
/// -1, never set here: the spawn's SourcePointer stays 0, so every index is accepted) adds its last velocity with
/// bInheritSourceVel.
#[allow(clippy::too_many_arguments)]
fn attract_particle(p: &mut Particle, k: usize, sources: &[(Vec3, Vec3, f32)], range: &Dist, strength: &Dist, by_distance: bool, affect_base: bool, inherit: bool, dt: f32) {
    let (index, last_vel) = p.attractor[k];
    if index < 0 {
        if inherit {
            p.velocity += last_vel;
        }
        return;
    }
    let Some(&(src, src_vel, src_time)) = sources.get(index as usize) else { return };
    let range = range.eval(src_time, 0.0);
    let dir = src - p.location;
    let d2 = dir.length_squared();
    let d = d2.sqrt();
    if d > range {
        return;
    }
    let s = if by_distance { strength.eval(if range != 0.0 { (range - d) / range } else { 0.0 }, 0.0) } else { strength.eval(src_time, 0.0) };
    let n = if d2 > 1e-8 { dir / d } else { dir };
    let delta = n * s * dt;
    p.velocity += delta;
    p.attractor[k].1 = src_vel;
    if affect_base {
        p.base_velocity += delta;
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
        // UParticleModuleLight::Update (0x1415b3580): ColorScale = BrightnessOverLife x ColorScaleOverLife at the
        // particle's relative time.
        Module::Light { color, brightness, .. } => p.light_color = color.eval3(t, [0.0; 3]) * brightness.eval(t, 0.0),
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
            // UParticleModuleCollision::Spawn (0x1415df650): the payload at the emitter time; collisions held off
            // while the delay is past the spawn time.
            Module::Collision { damping, damping_rotation, max_collisions, delay, .. } => {
                let d = delay.eval(emitter_time, rand(rng));
                p.collision = (damping.eval3(emitter_time, rand3(rng)), damping_rotation.eval3(emitter_time, rand3(rng)), max_collisions.eval(emitter_time, rand(rng)).round() as i32, d);
                if spawn_time < d {
                    p.state |= DELAY_COLLISIONS;
                }
            }
            Module::LocationCylinder { radius, height, origin, axis, surface_only, velocity, velocity_scale } => {
                // UParticleModuleLocationPrimitiveCylinder::SpawnEx (0x1415e1ca0): the radial axes and the height
                // axis by HeightAxis; a unit direction from LocationPrimitiveBase's DetermineUnitDirection (each axis
                // in -1..1) until its radial part falls in the circle (50 tries); the height within half StartHeight;
                // SurfaceOnly puts it on the side's circle (or anywhere on a cap).
                let h = *axis as usize;
                let (r0, r1) = match h {
                    0 => (1, 2),
                    1 => (0, 2),
                    _ => (0, 1),
                };
                let r = radius.eval(emitter_time, rand(rng));
                let half = height.eval(emitter_time, rand(rng)) * 0.5;
                let mut unit = Vec3::ZERO;
                for _ in 0..50 {
                    unit = Vec3::new(rand(rng) * 2.0 - 1.0, rand(rng) * 2.0 - 1.0, rand(rng) * 2.0 - 1.0);
                    if (unit[r0] * r).powi(2) + (unit[r1] * r).powi(2) <= r * r {
                        break;
                    }
                }
                let flat = Vec2::new(unit[r0], unit[r1]).normalize_or_zero();
                let mut offset = Vec3::ZERO;
                offset[h] = unit[h] * half;
                let on_cap = (offset[h].abs() - half.abs()).abs() <= 1e-4;
                if *surface_only && !on_cap {
                    offset[r0] = flat.x * r;
                    offset[r1] = flat.y * r;
                } else {
                    offset[r0] = unit[r0] * r;
                    offset[r1] = unit[r1] * r;
                }
                if *velocity {
                    let mut dir = unit.normalize_or_zero();
                    dir[r0] = flat.x;
                    dir[r1] = flat.y;
                    let v = to_world(dir * velocity_scale.eval(emitter_time, rand(rng)));
                    p.velocity += v;
                    p.base_velocity += v;
                }
                p.location += to_world(offset + origin.eval3(emitter_time, rand3(rng)));
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
            // UParticleModuleMeshRotation / MeshRotationRate Spawn (FUN_1415aea90 / FUN_1415aec40): x 360. Not run:
            // bInheritParent (the component's rotation added).
            Module::MeshRotation(d) => {
                let r = d.eval3(emitter_time, rand3(rng)) * 360.0;
                p.mesh_init_rotation = r;
                p.mesh_rotation += r;
            }
            Module::MeshRotationRate(d) => {
                let r = d.eval3(emitter_time, rand3(rng)) * 360.0;
                p.mesh_rate += r;
                p.mesh_rate_base += r;
            }
            // UParticleModuleLight::Spawn (FUN_1415ae8b0, r.ParticleLightQuality > 0): ColorScale at the particle's
            // relative time, RadiusScale and LightExponent (0 with inverse-squared falloff) at the emitter's,
            // bValid = rand < SpawnFraction. bHighQualityLights' point light components aren't spawned.
            Module::Light { color, brightness, radius, exponent, inverse_squared, spawn_fraction, high_quality } => {
                p.light_color = color.eval3(p.relative_time, rand3(rng)) * brightness.eval(p.relative_time, rand(rng));
                p.light_radius = radius.eval(emitter_time, rand(rng));
                p.light_exponent = if *inverse_squared { 0.0 } else { exponent.eval(emitter_time, rand(rng)) };
                p.light_valid = rand(rng) < *spawn_fraction && !*high_quality;
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
    // PostSpawn; a mesh emitter's (FParticleMeshEmitterInstance, unaligned): InitialOrientation from RollPitchYawRange.
    p.location += spawn_time * p.velocity;
    if let Some(md) = &def.mesh {
        p.mesh_orientation = md.roll_pitch_yaw.eval3(emitter_time, rand3(rng));
    }
    p
}

/// The emitters' quads, facing the camera.
/// A mesh particle's transform (Bevy space) at FDynamicMeshEmitterData::CalculateParticleTransform's default
/// (FUN_1415be6e0): v x PreRot(MakeFromEuler(InitialOrientation)) x Scale(Size) x Rot(MakeFromEuler(Rotation x 180 /
/// pi on every axis) + MakeFromEuler(payload Rotation)) + its location, then the component's rotation for local-space
/// emitters (Unreal row vectors).
fn mesh_particle_transform(p: &Particle, local_space: bool, rotation: Quat, scale: Vec3) -> bevy::math::Affine3A {
    let euler = |v: Vec3| Vec3::new(v.y, v.z, v.x);
    let pre = Mat3::from_quat(f13_core::camera_spline::quaternion(euler(p.mesh_orientation)));
    let rotator = euler(Vec3::splat(p.rotation.to_degrees())) + euler(p.mesh_rotation);
    let rot = Mat3::from_quat(f13_core::camera_spline::quaternion(rotator));
    let local = if local_space { Mat3::from_quat(rotation) } else { Mat3::IDENTITY };
    let lin = local * rot * Mat3::from_diagonal(p.size * scale) * pre;
    // Unreal to Bevy: the export's basis change (x, z, y) on both sides; the mesh is already in metres.
    let swap = Mat3::from_cols(Vec3::X, Vec3::Z, Vec3::Y);
    bevy::math::Affine3A::from_mat3_translation(swap * lin * swap, to_bevy(p.location))
}

fn draw(
    mut commands: Commands,
    camera: Query<(&GlobalTransform, &bevy::camera::primitives::Frustum), (With<Camera3d>, With<crate::lighting::Lit>)>,
    virtual_time: Res<Time<Virtual>>,
    mut systems: Query<&mut ParticleSystemInstance>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut vis: Query<&mut Visibility>,
    mut slot_xf: Query<&mut Transform>,
    mut point_lights: Query<&mut PointLight>,
    mut batch_entities: Local<HashMap<AssetId<ParticleMaterial>, BatchDraw>>,
    mut buffers: ResMut<Assets<ShaderBuffer>>,
    mut materials: ResMut<Assets<ParticleMaterial>>,
    mut mesh_materials: ResMut<Assets<MeshParticleMaterial>>,
    mut slot_meshes: Query<&mut Mesh3d>,
) {
    let Some((cam, frustum)) = camera.iter().next() else { return };
    let cam_pos = cam.translation();
    let (cam_right, cam_up, cam_fwd) = (cam.right().as_vec3(), cam.up().as_vec3(), cam.forward().as_vec3());
    // Sprite emitters sharing a material across the live systems (the rain grid's cells) draw as one mesh per
    // material: each emitter's own mesh is a draw (and an upload) of its own, and the rain has a hundred.
    let mut users: HashMap<AssetId<ParticleMaterial>, u32> = HashMap::new();
    for s in systems.iter() {
        for e in s.emitters.iter().filter(|e| e.mesh_scene.is_none() && e.def.ribbon.is_none() && e.drawn) {
            if let Some(m) = &e.material {
                *users.entry(m.id()).or_default() += 1;
            }
        }
    }
    // The batched emitters' quads (world space), built in parallel across the systems.
    let view = SpriteView { pos: cam_pos, right: cam_right, up: cam_up, fwd: cam_fwd };
    let mut built: bevy::utils::Parallel<Vec<(AssetId<ParticleMaterial>, (GpuSprite, f32))>> = bevy::utils::Parallel::default();
    systems.par_iter().for_each(|s| {
        let origin = to_bevy(s.location);
        if s.hidden || (s.max_draw > 0.0 && origin.distance(cam_pos) > s.max_draw) {
            return;
        }
        let mut out = built.borrow_local_mut();
        for e in s.emitters.iter().filter(|e| e.drawn && e.mesh_scene.is_none() && e.chunked.is_none() && e.def.ribbon.is_none()) {
            let Some(m) = e.material.as_ref().filter(|m| users.get(&m.id()).copied().unwrap_or(0) > 1) else { continue };
            let lock_z = e.def.modules.iter().any(|m| matches!(m, Module::AxisLock(a) if a == "EPAL_ROTATE_Z"));
            let count = e.particles.len().min(e.def.max_draw_count.map_or(usize::MAX, |m| m as usize));
            out.extend(e.particles[..count].iter().map(|p| (m.id(), gpu_sprite(e, p, s.scale, &view, lock_z))));
        }
    });
    let mut batches: HashMap<AssetId<ParticleMaterial>, Batch> = HashMap::new();
    for mut s in &mut systems {
        let s = &mut *s;
        let origin = to_bevy(s.location);
        let near = !s.hidden && (s.max_draw <= 0.0 || origin.distance(cam_pos) <= s.max_draw);
        // Its LastRenderTime (ParticleModuleCollision's bCollideOnlyIfVisible): drawn this frame, within its draw
        // distance and its particles' box in the view's frustum (the engine's occlusion culling not modelled).
        if near && in_frustum(s, frustum) {
            s.last_render = virtual_time.elapsed_secs();
        }
        for e in s.emitters.iter_mut() {
            if let Some(c) = &mut e.chunked {
                // An override-material mesh emitter: as below, each drawn particle its chunk's mesh and its material.
                let want = if near { e.particles.len().min(c.cap) } else { 0 };
                while e.slots.len() < want {
                    let m = mesh_materials.add(MeshParticleMaterial { base: c.params.material(Vec4::ONE, [1.0; 4]), extension: MeshParticleExt { clip: Vec4::new(DEBRIS_CLIP, 0.0, 0.0, 0.0) } });
                    let slot = commands.spawn((Mesh3d(c.chunks[0].clone()), MeshMaterial3d(m.clone()), Transform::default(), Visibility::Hidden, NotShadowCaster, Name::new(format!("{} {}", e.def.name, e.slots.len())))).id();
                    e.slots.push(slot);
                    c.materials.push(m);
                }
                for (i, &slot) in e.slots.iter().enumerate() {
                    // ceil(ChunkRandom x 4) (round_pi): 0 only for a draw of exactly 0, every vertex then at the origin.
                    let shown = e.particles.get(i).filter(|_| near).map(|p| (p, (p.dynamic[2] * 4.0).ceil() as i32)).filter(|(_, k)| (1..=4).contains(k));
                    let Some((p, k)) = shown else {
                        if let Ok(mut v) = vis.get_mut(slot) {
                            v.set_if_neq(Visibility::Hidden);
                        }
                        continue;
                    };
                    if let Ok(mut m) = slot_meshes.get_mut(slot) {
                        let chunk = &c.chunks[k as usize - 1];
                        if m.0 != *chunk {
                            m.0 = chunk.clone();
                        }
                    }
                    if let Some(mut m) = mesh_materials.get_mut(&c.materials[i]) {
                        m.base = c.params.material(p.color, p.dynamic);
                    }
                    if let Ok(mut xf) = slot_xf.get_mut(slot) {
                        *xf = Transform::from_matrix(mesh_particle_transform(p, e.def.local_space, s.rotation, s.scale).into());
                    }
                    if let Ok(mut v) = vis.get_mut(slot) {
                        v.set_if_neq(Visibility::Inherited);
                    }
                }
                continue;
            }
            if let Some((scene, cap)) = &e.mesh_scene {
                // A mesh emitter: its first MaxDrawCount particles, each its mesh ([`mesh_particle_transform`]).
                // Scenes for the particles drawn now (kept for the emitter's life once spawned).
                let want = if near { e.particles.len().min(*cap) } else { 0 };
                let name = e.def.name.clone();
                let more: Vec<Entity> = (e.slots.len()..want)
                    // Unshadowed: a particle component casts none (UPrimitiveComponent's constructor 0x141350d70 clears
                    // CastShadow; UParticleSystemComponent's doesn't set it), applied to the scene's meshes as a
                    // placement's (main.rs apply_placed).
                    .map(|i| commands.spawn((bevy::world_serialization::WorldAssetRoot(scene.clone()), Transform::default(), Visibility::Hidden, crate::Placed { cast_shadow: false, cull_m: 0.0 }, Name::new(format!("{name} {i}")))).id())
                    .collect();
                if !more.is_empty() {
                    debug!("particles: {name}: {} mesh slots", e.slots.len() + more.len());
                }
                e.slots.extend(more);
                for (i, &slot) in e.slots.iter().enumerate() {
                    let Some(p) = e.particles.get(i).filter(|_| near) else {
                        if let Ok(mut v) = vis.get_mut(slot) {
                            v.set_if_neq(Visibility::Hidden);
                        }
                        continue;
                    };
                    if let Ok(mut xf) = slot_xf.get_mut(slot) {
                        *xf = Transform::from_matrix(mesh_particle_transform(p, e.def.local_space, s.rotation, s.scale).into());
                    }
                    if let Ok(mut v) = vis.get_mut(slot) {
                        v.set_if_neq(Visibility::Inherited);
                    }
                }
                continue;
            }
            if !e.lights.is_empty() {
                // FDynamicSpriteEmitterDataBase::GatherSimpleLights (FUN_1415cee00): each valid particle (of the first
                // MaxDrawCount) a light of colour Color.rgb x Color.a x ColorScale, radius (|Size.x| + |Size.y|) / 2 x
                // RadiusScale (Size.y = Size.x for the square alignments), its exponent, at its location; skipped when
                // both radius and colour are about 0. Unshadowed. Their colour is the deferred light's LightColor
                // (cm-based inverse square: candela = colour / 100^2), so Bevy lumens = colour x 4pi / 100^2; with an
                // exponent, the radial falloff (lighting.rs's patched shading: the colour as lux at the centre).
                let square = matches!(e.def.screen_alignment.as_str(), "PSA_FacingCameraPosition" | "PSA_Square" | "PSA_FacingCameraDistanceBlend");
                let max = e.def.max_draw_count.map_or(usize::MAX, |m| m as usize);
                let mut lit = e.particles.iter().take(max).filter_map(|p| {
                    let sx = (p.size.x * s.scale.x).abs();
                    let sy = if square { sx } else { (p.size.y * s.scale.y).abs() };
                    let radius = (sx + sy) * 0.5 * p.light_radius;
                    let color = p.color.truncate() * p.color.w * p.light_color;
                    (p.light_valid && (radius > 1e-4 || color.max_element() > 1e-4)).then_some((p, radius, color))
                });
                for &l in &e.lights {
                    let next = if near { lit.next() } else { None };
                    if let Ok(mut v) = vis.get_mut(l) {
                        v.set_if_neq(if next.is_some() { Visibility::Inherited } else { Visibility::Hidden });
                    }
                    let Some((p, radius, color)) = next else { continue };
                    let peak = color.max_element().max(1e-6);
                    if let Ok(mut pl) = point_lights.get_mut(l) {
                        pl.color = Color::linear_rgb(color.x / peak, color.y / peak, color.z / peak);
                        (pl.intensity, pl.radius) = if p.light_exponent == 0.0 {
                            (peak * 4.0 * std::f32::consts::PI / 10_000.0, 0.0)
                        } else {
                            (crate::lighting::falloff_lumens(peak), crate::lighting::falloff_radius(p.light_exponent))
                        };
                        pl.range = radius * 0.01;
                    }
                    if let Ok(mut xf) = slot_xf.get_mut(l) {
                        xf.translation = to_bevy(p.location);
                    }
                }
            }
            let shown = near && e.drawn && !e.particles.is_empty();
            let batched = e.mesh_scene.is_none() && e.def.ribbon.is_none() && e.material.as_ref().is_some_and(|m| users.get(&m.id()).copied().unwrap_or(0) > 1);
            if let Ok(mut v) = vis.get_mut(e.entity) {
                v.set_if_neq(if shown && !batched { Visibility::Inherited } else { Visibility::Hidden });
            }
            if !shown {
                continue;
            }
            let def = &e.def;
            if let Some(ribbon) = &def.ribbon {
                ribbon_mesh(e, ribbon, s.scale, origin, cam_pos, &mut meshes);
                continue;
            }
            // A batched emitter's quads were built by the parallel pass above; its batch is made here.
            if let Some(m) = e.material.as_ref().filter(|_| batched) {
                batches.entry(m.id()).or_insert_with(|| Batch { material: m.clone(), distortion: e.distortion.clone() });
                continue;
            }
            // MaxDrawCount: the first so many, translucent sprites back to front (PSORTMODE_ViewProjDepth /
            // DistanceToView).
            let count = e.particles.len().min(def.max_draw_count.map_or(usize::MAX, |m| m as usize));
            let mut keyed: Vec<(f32, usize)> = (0..count).map(|i| ((to_bevy(e.particles[i].location) - cam_pos).dot(cam_fwd), i)).collect();
            keyed.sort_unstable_by(|a, b| b.0.total_cmp(&a.0));
            let n = keyed.len();
            let mut positions = Vec::with_capacity(n * 4);
            let mut uvs = Vec::with_capacity(n * 4);
            let mut colors = Vec::with_capacity(n * 4);
            let mut dynamic = Vec::with_capacity(n * 4);
            let mut indices = Vec::with_capacity(n * 6);
            let view = SpriteView { pos: cam_pos, right: cam_right, up: cam_up, fwd: cam_fwd };
            let lock_z = def.modules.iter().any(|m| matches!(m, Module::AxisLock(a) if a == "EPAL_ROTATE_Z"));
            for (_, i) in keyed {
                let q = sprite_quad(e, &e.particles[i], s.scale, &view, lock_z);
                let base = positions.len() as u32;
                positions.extend(q.positions.map(|c| (Vec3::from(c) - origin).to_array()));
                uvs.extend_from_slice(&q.uvs);
                colors.extend_from_slice(&q.colors);
                dynamic.extend_from_slice(&q.dynamic);
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
    // Each batch's sprites by (depth, thread, index) into the threads' lists, gathered after the sort.
    let locals: Vec<Vec<(AssetId<ParticleMaterial>, (GpuSprite, f32))>> = built.iter_mut().map(std::mem::take).collect();
    let mut keys: Vec<(AssetId<ParticleMaterial>, Vec<(f32, u32, u32)>)> = vec![];
    for (t, list) in locals.iter().enumerate() {
        for (i, (id, (_, depth))) in list.iter().enumerate() {
            let k = match keys.iter().position(|k| k.0 == *id) {
                Some(k) => k,
                None => {
                    keys.push((*id, vec![]));
                    keys.len() - 1
                }
            };
            keys[k].1.push((*depth, t as u32, i as u32));
        }
    }
    // The batches, back to front, into their sprite buffers (a fixed capacity, so the GPU buffer and the bind group
    // stay: Bevy's GpuShaderBuffer rewrites a buffer of the same size in place); a batch with nothing to draw hidden.
    let live: Vec<AssetId<ParticleMaterial>> = batches.keys().copied().collect();
    for (id, b) in batches {
        let mut keyed = keys.iter_mut().find(|k| k.0 == id).map(|k| std::mem::take(&mut k.1)).unwrap_or_default();
        keyed.sort_unstable_by(|a, c| c.0.total_cmp(&a.0));
        let n = keyed.len();
        // A batch's draw: its own copies of the materials, drawn from its sprite buffer, and its quad list; made anew
        // when it outgrows them (a buffer of another size is a new GPU buffer, which the copies' bind groups only
        // pick up when they are new assets themselves).
        let grow = batch_entities.get(&id).is_none_or(|d| n > d.capacity);
        if grow {
            let (Some(base), distortion) = (materials.get(&b.material).cloned(), b.distortion.as_ref().and_then(|d| materials.get(d).cloned())) else { continue };
            let capacity = n.next_power_of_two().max(1024);
            // (Extraction takes its bytes without copying them; the asset stays for the next frame's.)
            let sprites = buffers.add(ShaderBuffer::new(&vec![0u8; capacity * GPU_SPRITE_BYTES], RenderAssetUsages::default()));
            let material = materials.add(ParticleMaterial { sprites: sprites.clone(), stored: true, ..base });
            let distortion = distortion.map(|d| materials.add(ParticleMaterial { sprites: sprites.clone(), stored: true, ..d }));
            let mesh = meshes.add(sprite_quads(capacity));
            match batch_entities.get_mut(&id) {
                Some(draw) => {
                    commands.entity(draw.entity).insert((Mesh3d(mesh.clone()), MeshMaterial3d(material)));
                    if let (Some(child), Some(d)) = (draw.distortion_entity, distortion) {
                        commands.entity(child).insert((Mesh3d(mesh.clone()), MeshMaterial3d(d)));
                    }
                    draw.sprites = sprites;
                    draw.capacity = capacity;
                }
                None => {
                    let entity = commands.spawn((Mesh3d(mesh.clone()), MeshMaterial3d(material), Transform::IDENTITY, Visibility::Inherited, NoFrustumCulling, NotShadowCaster, NotShadowReceiver, Name::new("particles batch"))).id();
                    let distortion_entity = distortion.map(|d| commands.spawn((Mesh3d(mesh.clone()), MeshMaterial3d(d), Transform::default(), Visibility::Inherited, NoFrustumCulling, NotShadowCaster, NotShadowReceiver, ChildOf(entity), Name::new("particles batch distortion"))).id());
                    batch_entities.insert(id, BatchDraw { entity, distortion_entity, sprites, capacity });
                }
            }
        }
        let Some(draw) = batch_entities.get_mut(&id) else { continue };
        let mut bytes = vec![0u8; draw.capacity * GPU_SPRITE_BYTES];
        for (slot, k) in keyed.iter().enumerate() {
            let sprite = &locals[k.1 as usize][k.2 as usize].1 .0;
            let at = slot * GPU_SPRITE_BYTES;
            for (r, row) in sprite.iter().enumerate() {
                for (c, v) in row.iter().enumerate() {
                    bytes[at + (r * 4 + c) * 4..at + (r * 4 + c) * 4 + 4].copy_from_slice(&v.to_le_bytes());
                }
            }
        }
        if let Some(mut buffer) = buffers.get_mut(&draw.sprites) {
            buffer.data = Some(bytes);
        }
        if let Ok(mut v) = vis.get_mut(draw.entity) {
            v.set_if_neq(if n > 0 { Visibility::Inherited } else { Visibility::Hidden });
        }
    }
    for (id, draw) in batch_entities.iter() {
        if !live.contains(id) {
            if let Ok(mut v) = vis.get_mut(draw.entity) {
                v.set_if_neq(Visibility::Hidden);
            }
        }
    }
}

/// A batch's entity (and its distortion pass's), its sprite buffer, and how many sprites it holds.
struct BatchDraw {
    entity: Entity,
    distortion_entity: Option<Entity>,
    sprites: Handle<ShaderBuffer>,
    capacity: usize,
}

/// `count` quads for the vertex shader to place: sprite i's four vertices, their positions (i, corner, 0).
fn sprite_quads(count: usize) -> Mesh {
    let indices: Vec<u32> = (0..count as u32).flat_map(|q| {
        let b = q * 4;
        [b, b + 2, b + 1, b, b + 3, b + 2]
    }).collect();
    Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::default())
        .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, (0..count * 4).map(|v| [(v / 4) as f32, (v % 4) as f32, 0.0]).collect::<Vec<_>>())
        .with_inserted_indices(Indices::U32(indices))
}

/// TParticleSimulationPS<PCM_DepthBuffer>'s collision (global shader cache), for one particle this tick: radius
/// min(|size.x|, |size.y|) x the component's scale x CollisionRadiusScale + CollisionRadiusBias; probe the move's end
/// pushed on by the radius; against the surface found there (point S, normal N; d = N.S): in = radius + N.position - d,
/// out = N.(position + velocity dt) - radius - d. With in >= 0, out <= 0 and the move into the surface: velocity =
/// OneMinusFriction x its part along the surface - Resilience x its part along N, position + (N.position - radius - d)
/// x N, then a tenth of the new move, and the relative time + CollisionTimeBias (Response Kill: 1.1, so it dies);
/// with both below 0 (behind the surface) the relative time is 1.1. PLACEHOLDER: the game finds the surface in the
/// scene depth buffer and its normal in the G-buffer (within CollisionDepthBounds of the probe); the world collision
/// stands in. CollisionRandomSpread's cone isn't ported (no emitter here sets it).
fn gpu_collide(p: &mut Particle, dt: f32, scale: Vec3, gc: &f13_data::particles::GpuCollision, trace: Trace) {
    let v = p.velocity;
    let delta = v * dt;
    if delta.length_squared() < 1e-12 {
        return;
    }
    let radius = (p.size.x * scale.x).abs().min((p.size.y * scale.y).abs()) * gc.radius_scale + gc.radius_bias;
    let from = p.location;
    let end = from + delta + delta.normalize() * radius;
    let Some((t, n)) = trace(from, end) else { return };
    let d = n.dot(from + (end - from) * t);
    let in_d = radius + n.dot(from) - d;
    let out_d = n.dot(from + delta) - radius - d;
    if in_d >= 0.0 && out_d <= 0.0 && n.dot(delta) < 0.0 {
        let vn = v.dot(n) * n;
        let v2 = gc.one_minus_friction * (v - vn) - gc.resilience * vn;
        p.velocity = v2;
        p.base_velocity = v2;
        // The integrator adds v2 x dt after this: leave a tenth of it.
        p.location = from + (n.dot(from) - radius - d) * n + v2 * dt * 0.1 - v2 * dt;
        p.relative_time += gc.time_bias;
    } else if in_d < 0.0 && out_d < 0.0 {
        p.relative_time = 1.1;
    }
}

/// A ribbon's strip (FDynamicRibbonEmitterData's vertex fill 0x1415cc230): from the trail's newest point to its oldest,
/// each point two vertices at its location +/- its up x Size.X x the component's scale.X (V 0 and 1), U stepping 2 /
/// the trail's triangle count from 0 (so 0 to 1 along it), the point's colour. The up is the point's own (source or
/// world up), or for Trails_CameraUp the normalized cross of the point's direction from the view and its tangent (the
/// direction to the next point here; the payload's tangent PLACEHOLDER). Points with two or more interpolation steps
/// would be tessellated (Hermite between neighbours); the knife's trail has no tessellation (its step size and
/// tangent scalar 0), so it isn't ported.
fn ribbon_mesh(e: &EmitterState, ribbon: &f13_data::particles::Ribbon, scale: Vec3, origin: Vec3, cam_pos: Vec3, meshes: &mut Assets<Mesh>) {
    let points: Vec<&Particle> = e.particles.iter().rev().collect();
    let n = points.len();
    let mut positions = Vec::with_capacity(n * 2);
    let mut uvs = Vec::with_capacity(n * 2);
    let mut colors = Vec::with_capacity(n * 2);
    let mut dynamic = Vec::with_capacity(n * 2);
    let mut indices = Vec::with_capacity(n * 6);
    let step = if n > 1 { 1.0 / (n - 1) as f32 } else { 0.0 };
    for (i, p) in points.iter().enumerate() {
        let at = to_bevy(p.location);
        let up = match ribbon.render_axis {
            1 => to_bevy_dir(p.ribbon_up),
            2 => Vec3::Y,
            _ => {
                let next = points.get(i + 1).or_else(|| i.checked_sub(1).and_then(|j| points.get(j))).map_or(at, |q| to_bevy(q.location));
                let tangent = (next - at).normalize_or_zero();
                (at - cam_pos).normalize_or_zero().cross(tangent).normalize_or_zero()
            }
        };
        let half = up * p.size.x * scale.x * 0.01;
        let u = i as f32 * step;
        for (v, pos) in [(0.0, at + half), (1.0, at - half)] {
            positions.push((pos - origin).to_array());
            uvs.push([u, v]);
            colors.push(p.color.to_array());
            dynamic.push(p.dynamic);
        }
        if i + 1 < n {
            let b = (i * 2) as u32;
            indices.extend_from_slice(&[b, b + 2, b + 1, b + 1, b + 2, b + 3]);
        }
    }
    if let Some(mut mesh) = meshes.get_mut(&e.mesh) {
        mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
        mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, uvs);
        mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, colors);
        mesh.insert_attribute(ATTRIBUTE_DYNAMIC, dynamic);
        mesh.insert_indices(Indices::U32(indices));
    }
}

/// An Unreal direction (Z up) in Bevy's axes (Y up).
fn to_bevy_dir(v: Vec3) -> Vec3 {
    Vec3::new(v.x, v.z, v.y)
}

/// Whether a system's particles' box (Bevy space) meets the view's frustum.
fn in_frustum(s: &ParticleSystemInstance, frustum: &bevy::camera::primitives::Frustum) -> bool {
    let mut points = s.emitters.iter().filter(|e| e.drawn).flat_map(|e| e.particles.iter()).map(|p| to_bevy(p.location));
    let Some(first) = points.next() else { return false };
    let (lo, hi) = points.fold((first, first), |(lo, hi), p| (lo.min(p), hi.max(p)));
    let aabb = bevy::camera::primitives::Aabb::from_min_max(lo, hi);
    frustum.intersects_obb(&aabb, &bevy::math::Affine3A::IDENTITY, true, true)
}
