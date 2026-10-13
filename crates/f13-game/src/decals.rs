//! The impact effects' decals (SCImpactEffect's, spawned with its FX at a hit: FUN_14035e130; F13-decomp MODLOG
//! "Impact decals"): each a box projecting its material onto what lies inside it (shaders/decals.wgsl), gone after its
//! LifeSpan.

use bevy::asset::embedded_asset;
use bevy::mesh::MeshVertexBufferLayoutRef;
use bevy::pbr::{ExtendedMaterial, MaterialExtension, MaterialExtensionKey, MaterialExtensionPipeline};
use bevy::prelude::*;
use bevy::render::render_resource::{AsBindGroup, CompareFunction, Face, RenderPipelineDescriptor, ShaderType, SpecializedMeshPipelineError};
use bevy::shader::ShaderRef;
use std::collections::HashMap;

pub(crate) type DecalMaterial = ExtendedMaterial<StandardMaterial, DecalExt>;

pub struct DecalPlugin;

impl Plugin for DecalPlugin {
    fn build(&self, app: &mut App) {
        embedded_asset!(app, "shaders/decals.wgsl");
        app.add_plugins(MaterialPlugin::<DecalMaterial>::default())
            .add_message::<SpawnDecal>()
            .add_systems(Update, (spawn_decals, expire_decals, fade_decals));
    }
}

#[derive(Asset, AsBindGroup, Reflect, Debug, Clone)]
pub(crate) struct DecalExt {
    #[uniform(100)]
    params: DecalParams,
    #[texture(101)]
    #[sampler(102)]
    tex_a: Option<Handle<Image>>,
    #[texture(103)]
    #[sampler(104)]
    tex_b: Option<Handle<Image>>,
    #[texture(105)]
    #[sampler(106)]
    tex_c: Option<Handle<Image>>,
}

/// shaders/decals.wgsl's parameters.
#[derive(ShaderType, Reflect, Debug, Clone, Default)]
struct DecalParams {
    mode: Vec4,
    rot: Vec4,
    rot2: Vec4,
    uv: Vec4,
    color: Vec4,
    alpha: Vec4,
    surface: Vec4,
    overlay: Vec4,
    base: Vec4,
    normal_base: Vec4,
}

impl MaterialExtension for DecalExt {
    fn fragment_shader() -> ShaderRef {
        "embedded://f13_game/shaders/decals.wgsl".into()
    }

    fn alpha_mode() -> Option<AlphaMode> {
        Some(AlphaMode::Blend)
    }

    fn enable_prepass() -> bool {
        false
    }

    fn enable_shadows() -> bool {
        false
    }

    // The box's back faces with no depth test, as the engine draws a decal it may be inside of (each pixel then takes
    // the scene's depth).
    fn specialize(_pipeline: &MaterialExtensionPipeline, descriptor: &mut RenderPipelineDescriptor, _layout: &MeshVertexBufferLayoutRef, _key: MaterialExtensionKey<Self>) -> Result<(), SpecializedMeshPipelineError> {
        descriptor.primitive.cull_mode = Some(Face::Front);
        if let Some(ds) = descriptor.depth_stencil.as_mut() {
            ds.depth_compare = Some(CompareFunction::Always);
            ds.depth_write_enabled = Some(false);
        }
        Ok(())
    }
}

/// One of an SCImpactEffect's decals (its <surface>Decal: GetImpactDecal FUN_1403413b0's record).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ImpactDecal {
    pub material: String,
    /// Unreal's DecalSize (cm: the box's half extents along X, the projection, then Y and Z).
    pub size: Vec3,
    pub random_rotation: bool,
    /// Seconds (0: it stays).
    pub lifespan: f32,
    pub fade_screen_size: f32,
    pub splashes: Vec<SplashDecal>,
    /// bSingleSplashDecal: no more splashes once one lands.
    pub single_splash: bool,
}

/// An impact decal's SplashDecals entry: traced from the hit, a random one of its decals where the trace lands.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SplashDecal {
    pub decals: Vec<String>,
    pub random_rotation: bool,
    pub size: Vec3,
    pub lifespan: f32,
    pub fade_screen_size: f32,
    /// Unreal space, relative to the instigator's facing when `instigator_forward`.
    pub ray_direction: Vec3,
    /// cm.
    pub ray_length: f32,
    pub instigator_forward: bool,
}

impl ImpactDecal {
    /// An impact decal record from classes.json, or None without a material.
    pub fn from_json(v: &serde_json::Value) -> Option<Self> {
        let material = asset_name(&v["DecalMaterial"])?;
        Some(ImpactDecal {
            material,
            size: vec3(&v["DecalSize"]).unwrap_or(DEFAULT_SIZE),
            // Not a reflected property: always the constructor's.
            random_rotation: DEFAULT_RANDOM_ROTATION,
            single_splash: v["bSingleSplashDecal"].as_bool().unwrap_or(false),
            lifespan: v["LifeSpan"].as_f64().map_or(DEFAULT_LIFESPAN, |x| x as f32),
            fade_screen_size: v["FadeScreenSize"].as_f64().map_or(DEFAULT_FADE_SCREEN_SIZE, |x| x as f32),
            splashes: v["SplashDecals"].as_array().map_or_else(Vec::new, |a| {
                a.iter()
                    .map(|s| SplashDecal {
                        decals: s["Decals"].as_array().map_or_else(Vec::new, |d| d.iter().filter_map(asset_name).collect()),
                        random_rotation: s["bApplyRandomRotation"].as_bool().unwrap_or(DEFAULT_RANDOM_ROTATION),
                        size: vec3(&s["DecalSize"]).unwrap_or(DEFAULT_SIZE),
                        lifespan: s["LifeSpan"].as_f64().map_or(DEFAULT_LIFESPAN, |x| x as f32),
                        fade_screen_size: s["FadeScreenSize"].as_f64().map_or(DEFAULT_FADE_SCREEN_SIZE, |x| x as f32),
                        ray_direction: vec3(&s["RayDirection"]).unwrap_or(Vec3::ZERO),
                        ray_length: s["RayLength"].as_f64().unwrap_or(0.0) as f32,
                        instigator_forward: s["bUseInstigatorForwardVector"].as_bool().unwrap_or(false),
                    })
                    .collect()
            }),
        })
    }
}

// The records' defaults (FDecalData's, as SCImpactEffect's constructor 0x14032ca50 writes each: +8 1, DecalSize (5,
// 32, 32), LifeSpan 10, FadeScreenSize 0.01).
const DEFAULT_SIZE: Vec3 = Vec3::new(5.0, 32.0, 32.0);
const DEFAULT_RANDOM_ROTATION: bool = true;
const DEFAULT_LIFESPAN: f32 = 10.0;
const DEFAULT_FADE_SCREEN_SIZE: f32 = 0.01;

fn asset_name(v: &serde_json::Value) -> Option<String> {
    let s = v.as_str()?;
    let name = s.split('\'').nth(1).unwrap_or(s);
    Some(name.rsplit(['.', '/']).next().unwrap_or(name).to_string())
}

fn vec3(v: &serde_json::Value) -> Option<Vec3> {
    Some(Vec3::new(v["X"].as_f64()? as f32, v["Y"].as_f64()? as f32, v["Z"].as_f64()? as f32))
}

/// Spawns a decal (the game's spawn FUN_140332110: UGameplayStatics::SpawnDecalAttached to the hit's component at its
/// ImpactPoint, keeping that world position, with the LifeSpan, then the component's FadeScreenSize): Bevy space
/// position, Unreal rotator (pitch, yaw, roll degrees) and DecalSize (cm).
#[derive(Message, Clone, Debug)]
pub struct SpawnDecal {
    pub material: String,
    pub at: Vec3,
    pub rotator: Vec3,
    pub size: Vec3,
    pub lifespan: f32,
    pub fade_screen_size: f32,
    /// A character it's on: it follows the bone nearest where it lands (SpawnDecalAttached to the hit component's
    /// bone, KeepWorldPosition).
    pub on: Option<Entity>,
}

#[derive(Component)]
struct Decal {
    /// Seconds left (None: it stays).
    left: Option<f32>,
    fade_screen_size: f32,
    /// Its box's largest half extent, cm.
    radius: f32,
}

/// The decal materials by name, built once from materials.json (each decal then gets its own copy for its fade).
#[derive(Default)]
struct Built(HashMap<String, Option<DecalMaterial>>);

#[derive(serde::Deserialize, Clone)]
#[serde(rename_all = "PascalCase")]
struct Entry {
    #[serde(default)]
    master: String,
    #[serde(default)]
    scalars: HashMap<String, f32>,
    #[serde(default)]
    vectors: HashMap<String, [f32; 4]>,
    #[serde(default)]
    textures: HashMap<String, String>,
}

/// A 2 x 2 rotation's rows from a material's rotation parameter (turns: CustomRotator, angle x 2 pi).
fn rotation(turns: f32) -> Vec4 {
    let (s, c) = (turns * std::f32::consts::TAU).sin_cos();
    Vec4::new(c, -s, s, c)
}

fn build(root: &std::path::Path, assets: &AssetServer, e: &Entry) -> Option<DecalMaterial> {
    let s = |n: &str, d: f32| e.scalars.get(n).copied().unwrap_or(d);
    let v = |n: &str| e.vectors.get(n).map(|c| Vec4::from_array(*c));
    let t = |n: &str, color: bool| e.textures.get(n).and_then(|p| crate::materials::load_texture(root, assets, p, color));
    let mut p = DecalParams { uv: Vec4::new(s("0_U_Multiplier", 1.0), s("1_U_Offset", 0.0), s("0_V_Multiplier", 1.0), s("1_V_Offset", 0.0)), ..default() };
    let ac = |d: f32| s("ALPHA_Contrast", d);
    let (tex_a, tex_b, tex_c);
    match e.master.as_str() {
        "M_Impact_Decal_MASTER_01" => {
            p.mode.x = 0.0;
            p.rot = rotation(s("Texture1_Rotation", 0.0));
            p.rot2 = rotation(s("Overlay_Rotation", 0.0));
            p.color = v("Decal_Color").unwrap_or(Vec4::new(0.455, 0.299558, 0.249223, 1.0));
            p.alpha = Vec4::new(-ac(0.0), 1.0 + ac(0.0), s("OpacityLevel", 1.0), s("0_AlphaClamp_Min", 0.0));
            let z = s("z_Normal_Multiplier", 1.0);
            p.surface = Vec4::new(s("0_AlphaClamp_Max", 1.0), s("Roughness", 1.0), s("SpecularLevel", 0.1), z);
            let otc = s("0_OverlayTextureContrast", 0.2);
            p.overlay = Vec4::new(-otc, 1.0 + otc, s("0_OverlayTextureMultiplier", 1.0), s("0_BaseMasterMultiplier", 1.5));
            let lerp = (z.clamp(0.0, 0.5) * 2.0).clamp(0.0, 1.0);
            p.base = Vec4::new(s("0_BaseClamp_Min", 0.0), s("0_BaseClamp_Max", 1.0), lerp, (lerp * 2.0 - 1.0).clamp(0.0, 1.0));
            p.normal_base = v("Normal_Base").unwrap_or(Vec4::new(0.0, 0.0, 1.0, 1.0));
            (tex_a, tex_b, tex_c) = (t(DIFFUSE, true), t(NORMAL, false), t(OVERLAY, false));
        }
        "M_Wall_Impact_Default_01" => {
            p.mode.x = 1.0;
            p.rot = rotation(s("Rotation", 0.0));
            p.color = v("Decal_Color").unwrap_or(Vec4::new(0.2, 0.2, 0.2, 1.0));
            p.alpha = Vec4::new(-ac(0.1), 1.0 + ac(0.1), s("OpacityLevel", 1.5), 0.0);
            p.surface = Vec4::new(1.0, s("roughLow", 1.0), s("SpecularLevel", 0.1), s("Normal_Levels", 0.15));
            p.overlay.x = s("RoughHigh", 0.3);
            (tex_a, tex_b, tex_c) = (t(DIFFUSE, true), t(NORMAL, false), None);
        }
        "M_Blood_Decal_01" => {
            p.mode.x = 2.0;
            p.rot = rotation(s("Rotation", 0.0));
            p.color = v("Decal_Color").unwrap_or(Vec4::ONE);
            p.alpha = Vec4::new(-ac(5.0), 1.0 + ac(5.0), s("OpacityLevel", 1.5), 0.0);
            p.surface = Vec4::new(1.0, s("Roughness", 0.3), 0.5, 0.0);
            (tex_a, tex_b, tex_c) = (t(DIFFUSE, true), None, None);
        }
        m => {
            warn!("decal master {m} not ported");
            return None;
        }
    }
    p.mode.y = 1.0;
    Some(ExtendedMaterial { base: StandardMaterial { alpha_mode: AlphaMode::Blend, ..default() }, extension: DecalExt { params: p, tex_a, tex_b, tex_c } })
}

// The masters' texture parameters (their uniform expression sets' texture order: the impact master's NRM t0,
// Texture_Overlay t1, Diffuse t2; the wall's NRM t0, Diffuse t1; the blood's Diffuse t0).
const DIFFUSE: &str = "Diffuse";
const NORMAL: &str = "NRM";
const OVERLAY: &str = "Texture_Overlay";

#[allow(clippy::too_many_arguments)]
fn spawn_decals(
    mut commands: Commands,
    mut requests: MessageReader<SpawnDecal>,
    export: Res<crate::ExportRoot>,
    assets: Res<AssetServer>,
    mut materials: ResMut<Assets<DecalMaterial>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut built: Local<Built>,
    mut entries: Local<Option<HashMap<String, Entry>>>,
    mut cube: Local<Option<Handle<Mesh>>>,
    (children, skins, transforms): (Query<&Children>, Query<&bevy::mesh::skinning::SkinnedMesh>, Query<&GlobalTransform>),
) {
    for r in requests.read() {
        let entries = entries.get_or_insert_with(|| std::fs::read_to_string(export.0.join("materials.json")).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default());
        let mat = built
            .0
            .entry(r.material.clone())
            .or_insert_with(|| {
                let m = entries.get(&r.material).and_then(|e| build(&export.0, &assets, e));
                if m.is_none() {
                    warn!("decal material {} not exported or not ported", r.material);
                }
                m
            })
            .clone();
        let Some(mat) = mat else { continue };
        let cube = cube.get_or_insert_with(|| meshes.add(Cuboid::new(2.0, 2.0, 2.0))).clone();
        debug!("decal {} at {} rotator {} size {}", r.material, r.at, r.rotator, r.size);
        let q = f13_core::camera_spline::quaternion(r.rotator);
        let world = Transform { translation: r.at, rotation: Quat::from_xyzw(-q.x, -q.z, -q.y, q.w), scale: Vec3::new(r.size.x, r.size.z, r.size.y) / 100.0 };
        // On a character: under its skeleton's joint nearest the point, where it lands (kept in the world).
        let bone = r.on.and_then(|c| {
            children
                .iter_descendants(c)
                .filter_map(|e| skins.get(e).ok())
                .flat_map(|s| s.joints.iter().copied())
                .filter_map(|j| Some((j, transforms.get(j).ok()?)))
                .min_by(|a, b| a.1.translation().distance(r.at).total_cmp(&b.1.translation().distance(r.at)))
        });
        let transform = bone.map_or(world, |(_, g)| Transform::from_matrix(g.to_matrix().inverse() * world.to_matrix()));
        let mut e = commands.spawn((
            Mesh3d(cube),
            MeshMaterial3d(materials.add(mat)),
            transform,
            Decal { left: (r.lifespan > 0.0).then_some(r.lifespan), fade_screen_size: r.fade_screen_size, radius: r.size.max_element() },
            bevy::light::NotShadowCaster,
            Name::new(format!("decal {}", r.material)),
        ));
        if let Some((j, _)) = bone {
            e.insert(ChildOf(j));
        }
    }
}

/// Removes each decal when its LifeSpan runs out.
fn expire_decals(mut commands: Commands, time: Res<Time>, mut decals: Query<(Entity, &mut Decal)>) {
    for (e, mut d) in &mut decals {
        let Some(left) = d.left.as_mut() else { continue };
        *left -= time.delta_secs();
        if *left <= 0.0 {
            commands.entity(e).despawn();
        }
    }
}

/// Each decal's fade with its size on screen (the renderer's decal gathering 0x140aeb6b0, as UE 4.18's
/// FDeferredDecalProxy): with a FadeScreenSize, a perspective view's coefficient c = FadeScreenSize x 2 / P[0][0] /
/// the view's width x 600, and alpha = min((radius / distance x r.Decal.FadeScreenSizeMult (1) - c) / (c x 0.5), 1),
/// the decal not drawn at 0 or less. The radius is its transform's largest axis scale (DecalSize's largest, cm).
/// FDeferredDecalPS's DecalParams.y stays 1 (nothing sets a fade-out: the LifeSpan removes it).
fn fade_decals(
    cameras: Query<(&GlobalTransform, &Projection, &Camera), With<crate::player::PlayerCamera>>,
    mut decals: Query<(&Decal, &GlobalTransform, &MeshMaterial3d<DecalMaterial>, &mut Visibility)>,
    mut materials: ResMut<Assets<DecalMaterial>>,
) {
    let Some((eye, projection, camera)) = cameras.iter().find(|c| c.2.is_active) else { return };
    let Projection::Perspective(_) = projection else { return };
    let p00 = projection.get_clip_from_view().x_axis.x;
    let width = camera.physical_viewport_size().map_or(1920.0, |s| s.x as f32);
    for (d, xf, mat, mut vis) in &mut decals {
        let mut alpha = 1.0;
        if d.fade_screen_size != 0.0 {
            let c = d.fade_screen_size * (2.0 / p00) / width * 600.0;
            let distance = (eye.translation().distance(xf.translation()) * 100.0).max(1e-3);
            alpha = ((d.radius / distance - c) / (c * 0.5)).min(1.0);
        }
        let shown = if alpha > 0.0 { Visibility::Inherited } else { Visibility::Hidden };
        vis.set_if_neq(shown);
        // Only when it changes (a material write re-uploads it).
        let alpha = alpha.max(0.0);
        if materials.get(&mat.0).is_some_and(|m| m.extension.params.mode.y != alpha) {
            if let Some(mut m) = materials.get_mut(&mat.0) {
                m.extension.params.mode.y = alpha;
            }
        }
    }
}

/// Unreal's Conv_VectorToRotator (pitch, yaw, roll 0, degrees) of an Unreal-space vector.
fn rotator_of(v: Vec3) -> Vec3 {
    Vec3::new(v.z.atan2((v.x * v.x + v.y * v.y).sqrt()).to_degrees(), v.y.atan2(v.x).to_degrees(), 0.0)
}

/// Bevy space to Unreal's axes (Y and Z swapped; the scale is the caller's).
fn ue(v: Vec3) -> Vec3 {
    Vec3::new(v.x, v.z, v.y)
}

/// An impact's decals (FUN_14035e130 after its FX and sound): the record's material at the hit, turned to
/// Conv_VectorToRotator(-ImpactNormal) with a roll of rand / 32767 x 360 - 180 when bApplyRandomRotation; then each
/// SplashDecals entry with decals traces (FUN_140331c60, "SplashDecalTrace") from the decal along RayDirection turned
/// by Conv_VectorToRotator of the instigator's forward (bUseInstigatorForwardVector) or of the ImpactNormal, RayLength
/// out, and where it lands spawns decal [min(rand / 32767 x count, count - 1)] the same way, stopping after the first
/// with bSingleSplashDecal. Bevy space in (`normal` out of the hit surface); `rand` the game's rand() / 32767; `trace`
/// the world's first hit on a segment (point, normal).
/// `on`: the character hit (its decal follows it; the splashes land on the world).
#[allow(clippy::too_many_arguments)]
pub fn spawn_impact(commands: &mut Commands, d: &ImpactDecal, at: Vec3, normal: Vec3, instigator_forward: Vec3, on: Option<Entity>, rand: &mut impl FnMut() -> f32, trace: impl Fn(Vec3, Vec3) -> Option<(Vec3, Vec3)>) {
    let mut rotator = rotator_of(ue(-normal));
    if d.random_rotation {
        rotator.z = rand() * 360.0 - 180.0;
    }
    // (No material: its splashes only.)
    if !d.material.is_empty() {
        commands.write_message(SpawnDecal { material: d.material.clone(), at, rotator, size: d.size, lifespan: d.lifespan, fade_screen_size: d.fade_screen_size, on });
    }
    for s in d.splashes.iter().filter(|s| !s.decals.is_empty()) {
        let basis = if s.instigator_forward { rotator_of(ue(instigator_forward)) } else { rotator_of(ue(normal)) };
        let dir = f13_core::camera_spline::quaternion(basis) * s.ray_direction;
        let end = at + ue(dir * s.ray_length) / 100.0;
        let Some((p, n)) = trace(at, end) else { continue };
        let n = if n.dot(end - at) > 0.0 { -n } else { n };
        let mut rotator = rotator_of(ue(-n));
        if s.random_rotation {
            rotator.z = rand() * 360.0 - 180.0;
        }
        let count = s.decals.len();
        let i = ((rand() * count as f32) as usize).min(count - 1);
        commands.write_message(SpawnDecal { material: s.decals[i].clone(), at: p, rotator, size: s.size, lifespan: s.lifespan, fade_screen_size: s.fade_screen_size, on: None });
        if d.single_splash {
            return;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn impact_decal_record() {
        // CounselorDecal as the impact classes have it: material, size and one splash; the rest the constructor's.
        let v: serde_json::Value = serde_json::from_str(
            r#"{"DecalMaterial": "MaterialInstanceConstant'M_Wall_Impact_Default_01_gash_Inst'", "DecalSize": {"X": 5.0, "Y": 12.0, "Z": 12.0},
                "SplashDecals": [{"Decals": ["MaterialInstanceConstant'M_BloodSplatterShot_ComboSheet_01A'"], "bApplyRandomRotation": true,
                "DecalSize": {"X": 2.0, "Y": 50.0, "Z": 50.0}, "LifeSpan": 20.0, "FadeScreenSize": 0.01, "RayDirection": {"X": 1.0, "Y": 0.0, "Z": -1.5},
                "RayLength": 300.0, "bUseInstigatorForwardVector": true}]}"#,
        )
        .unwrap();
        let d = ImpactDecal::from_json(&v).unwrap();
        assert_eq!(d.material, "M_Wall_Impact_Default_01_gash_Inst");
        assert_eq!(d.size, Vec3::new(5.0, 12.0, 12.0));
        assert!(d.random_rotation && !d.single_splash);
        assert_eq!((d.lifespan, d.fade_screen_size), (10.0, 0.01));
        assert_eq!(d.splashes[0].decals, ["M_BloodSplatterShot_ComboSheet_01A"]);
        assert_eq!(d.splashes[0].ray_direction, Vec3::new(1.0, 0.0, -1.5));
        assert!(ImpactDecal::from_json(&serde_json::json!({"DecalSize": {"X": 1.0, "Y": 1.0, "Z": 1.0}})).is_none());
        // Facing down onto the ground: pitch -90.
        assert!((rotator_of(ue(-Vec3::Y)).x + 90.0).abs() < 1e-4);
    }
}
