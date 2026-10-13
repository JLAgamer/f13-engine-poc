//! Materials from the exporter's `materials.json`. Most become a StandardMaterial (diffuse + normal); two masters get
//! shaders ported from the game's compiled ones (see F13-decomp MODLOG, "Materials"):
//! - `Master_Blend_01` (cabin walls, roofs, floors...), `Master_Tint_Blend_01` and the curtains: texture sets blended
//!   by painted vertex alpha
//!   ([`BlendMaterial`], shaders/blend.wgsl);
//! - landscape materials (layers in `materials.json`): height-blended layers from the painted weight maps
//!   ([`TerrainMaterial`], shaders/terrain.wgsl);
//! - props with an RMA map (the `M_Prop_Master` family): roughness/metallic/AO from it ([`PropMaterial`],
//!   shaders/prop.wgsl);
//! - the lake (`M_Lake_Ripples_*`): a first-pass water surface ([`WaterMaterial`], shaders/water.wgsl);
//! - small one-off masters: the phone box's and CB tower's lights, the cabin name signs ([`MasterMaterial`],
//!   shaders/masters.wgsl).

use bevy::asset::{embedded_asset, RenderAssetUsages};
use bevy::gltf::GltfMaterialName;
use bevy::image::{CompressedImageFormats, ImageLoaderSettings, ImageSampler, ImageSamplerDescriptor, ImageType};
use bevy::pbr::{ExtendedMaterial, MaterialExtension};
use bevy::prelude::*;
use bevy::render::render_resource::{AsBindGroup, Extent3d, ShaderType, TextureDimension, TextureFormat};
use bevy::shader::ShaderRef;
use bevy::world_serialization::WorldAssetRoot;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

pub(crate) type BlendMaterial = ExtendedMaterial<StandardMaterial, BlendExt>;
pub(crate) type TerrainMaterial = ExtendedMaterial<StandardMaterial, TerrainExt>;
pub(crate) type WaterMaterial = ExtendedMaterial<StandardMaterial, WaterExt>;
pub(crate) type PropMaterial = ExtendedMaterial<StandardMaterial, PropExt>;
/// Counselors' clothes (ClothesMultiGore): shaders/clothes.wgsl.
pub(crate) type ClothesMaterial = ExtendedMaterial<StandardMaterial, ClothesExt>;
pub(crate) type SkinMaterial = ExtendedMaterial<StandardMaterial, SkinMaskExt>;
pub(crate) type MasterMaterial = ExtendedMaterial<StandardMaterial, MasterExt>;

/// The phone box's light material while it's broken (its mesh's own, SCPhoneJunctionBox BrokenLightMaterial) and
/// once repaired (RepairedLightMaterial). No mesh uses the second, so the export's materials.json lacks it.
pub(crate) const PHONE_BOX_BROKEN_LIGHT: &str = "M_Interactive_TelephoneBox_01_Emissive_Red";
pub(crate) const PHONE_BOX_REPAIRED_LIGHT: &str = "M_Interactive_TelephoneBox_01_Emissive_Blue";

const TERRAIN_LAYERS: usize = 6;
const WATER_MASTER: &str = "M_Lake_Ripples";

pub(crate) struct MaterialsPlugin {
    pub export: PathBuf,
}

impl Plugin for MaterialsPlugin {
    fn build(&self, app: &mut App) {
        app.add_observer(bloody_weapon);
        // (F13_WEAPON_BLOOD: his weapon bloodied 8 s in, testing.)
        if std::env::var("F13_WEAPON_BLOOD").is_ok() {
            app.add_systems(Update, |mut commands: Commands, time: Res<Time>, mut done: Local<bool>| {
                if !*done && time.elapsed_secs() > 8.0 {
                    *done = true;
                    commands.trigger(BloodyWeapon(1.0));
                }
            });
        }
        embedded_asset!(app, "shaders/blend.wgsl");
        embedded_asset!(app, "shaders/terrain.wgsl");
        embedded_asset!(app, "shaders/water.wgsl");
        embedded_asset!(app, "shaders/prop.wgsl");
        embedded_asset!(app, "shaders/clothes.wgsl");
        embedded_asset!(app, "shaders/gorecap.wgsl");
        embedded_asset!(app, "shaders/hair.wgsl");
        embedded_asset!(app, "shaders/hair_prepass.wgsl");
        embedded_asset!(app, "shaders/skin.wgsl");
        embedded_asset!(app, "shaders/skin_prepass.wgsl");
        embedded_asset!(app, "shaders/masters.wgsl");
        let entries = std::fs::read_to_string(self.export.join("materials.json"))
            .ok()
            .and_then(|t| match serde_json::from_str(&t) {
                Ok(m) => Some(m),
                Err(e) => {
                    warn!("materials.json: {e} (run tools/f13-export again)");
                    None
                }
            })
            .unwrap_or_else(|| {
                warn!("no materials.json in the export; meshes will be untextured");
                HashMap::new()
            });
        // Unreal names are case-insensitive (meshes say M_rocks_scattered, the asset is M_Rocks_Scattered).
        let mut entries: HashMap<String, MaterialInfo> = entries.into_iter().map(|(k, v): (String, MaterialInfo)| (k.to_lowercase(), v)).collect();
        // A master with no parameters, built from its compiled shader alone (master_material).
        entries.entry(PHONE_BOX_REPAIRED_LIGHT.to_lowercase()).or_insert_with(|| MaterialInfo {
            file: String::new(),
            blend: "BLEND_Opaque".into(),
            two_sided: false,
            master: PHONE_BOX_REPAIRED_LIGHT.into(),
            scalars: HashMap::from([("Power".into(), 8.0)]),
            vectors: HashMap::new(),
            textures: HashMap::new(),
            landscape: None,
        });
        app.insert_resource(MaterialIndex { root: self.export.clone(), entries, ..default() })
            .add_plugins((
                MaterialPlugin::<BlendMaterial>::default(),
                MaterialPlugin::<TerrainMaterial>::default(),
                MaterialPlugin::<WaterMaterial>::default(),
                MaterialPlugin::<PropMaterial>::default(),
                MaterialPlugin::<SkinMaterial>::default(),
                MaterialPlugin::<MasterMaterial>::default(),
                MaterialPlugin::<ClothesMaterial>::default(),
                MaterialPlugin::<GoreCapMaterial>::default(),
                MaterialPlugin::<HairMaterial>::default(),
            ))
            .add_systems(Update, ((apply_materials, strip_vertex_colors).chain(), sky_lightning, clothes_rain, skin_rain));
    }
}

/// Material overrides for one placed mesh (slot material name, lowercase -> material used instead), on the entity
/// holding its scene.
#[derive(Component, Clone, Debug)]
pub(crate) struct MaterialOverrides(std::sync::Arc<HashMap<String, String>>);

impl MaterialOverrides {
    pub(crate) fn new(map: &HashMap<String, String>) -> Option<Self> {
        (!map.is_empty()).then(|| Self(std::sync::Arc::new(map.iter().map(|(k, v)| (k.to_lowercase(), v.clone())).collect())))
    }
}

/// The skin masks of a counselor's body (export-relative texture paths), on the entity holding the body's scene: its
/// skin materials are clipped under the clothes (shaders/skin.wgsl).
#[derive(Component, Clone)]
pub(crate) struct SkinMasks(pub(crate) Vec<String>);

/// One entry of the exporter's `materials.json`.
#[derive(serde::Deserialize)]
#[serde(rename_all = "PascalCase")]
struct MaterialInfo {
    file: String,
    blend: String,
    two_sided: bool,
    #[serde(default)]
    master: String,
    #[serde(default)]
    scalars: HashMap<String, f32>,
    #[serde(default)]
    vectors: HashMap<String, [f32; 4]>,
    #[serde(default)]
    textures: HashMap<String, String>,
    #[serde(default)]
    landscape: Option<LandscapeInfo>,
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "PascalCase")]
struct LandscapeInfo {
    mapping_scale: f32,
    layers: Vec<LandscapeLayer>,
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "PascalCase")]
struct LandscapeLayer {
    name: String,
    diffuse: Option<String>,
    normal: Option<String>,
    rma: Option<String>,
}

impl MaterialInfo {
    fn scalar(&self, name: &str, default: f32) -> f32 {
        self.scalars.get(name).copied().unwrap_or(default)
    }
    fn vector(&self, name: &str) -> Vec4 {
        self.vectors.get(name).map_or(Vec4::ZERO, |v| Vec4::from_array(*v))
    }
}

#[derive(Clone)]
enum Built {
    Standard(Handle<StandardMaterial>),
    Blend(Handle<BlendMaterial>),
    Terrain(Handle<TerrainMaterial>),
    Water(Handle<WaterMaterial>),
    Prop(Handle<PropMaterial>),
    Master(Handle<MasterMaterial>),
    Clothes(Handle<ClothesMaterial>),
    Skin(Handle<SkinMaterial>),
    GoreCap(Handle<GoreCapMaterial>),
    Hair(Handle<HairMaterial>),
    /// Not drawn: translucent planes with no textures (rain fixes, bed and exit blockers, light planes), which the
    /// game draws invisible (M_rainFix_invisible's compiled shader outputs zero) or as faint effects.
    Hidden,
}

/// Material name (as written into the glTF) -> exported material info, and the materials built so far.
#[derive(Resource, Default)]
pub(crate) struct MaterialIndex {
    root: PathBuf,
    entries: HashMap<String, MaterialInfo>,
    /// Keyed by material name (plus the landscape file for terrain materials, whose weight maps are per landscape).
    built: HashMap<String, Built>,
    /// Skin materials clipped by a body's masks, keyed like `built` plus "|skin".
    skins: HashMap<String, Handle<SkinMaterial>>,
    fallback: Option<Fallback>,
}

/// 1x1 stand-ins for textures a material names but the export lacks.
#[derive(Clone)]
struct Fallback {
    white: Handle<Image>,
    black: Handle<Image>,
    /// All channels 0 (alpha too).
    clear: Handle<Image>,
    normal: Handle<Image>,
    /// Roughness 1, metallic 0, AO 1, specular 0.5.
    rmas: Handle<Image>,
}

impl Fallback {
    fn new(images: &mut Assets<Image>) -> Self {
        let mut px = |rgba: [u8; 4]| {
            images.add(Image::new_fill(
                Extent3d::default(),
                TextureDimension::D2,
                &rgba,
                TextureFormat::Rgba8Unorm,
                RenderAssetUsages::RENDER_WORLD,
            ))
        };
        Self { white: px([255; 4]), black: px([0, 0, 0, 255]), clear: px([0; 4]), normal: px([128, 128, 255, 255]), rmas: px([255, 0, 255, 128]) }
    }
}

#[derive(Asset, AsBindGroup, Reflect, Debug, Clone)]
pub(crate) struct BlendExt {
    #[uniform(100)]
    params: BlendParams,
    #[texture(101)]
    #[sampler(102)]
    color_a: Handle<Image>,
    #[texture(103)]
    color_b: Handle<Image>,
    #[texture(104)]
    normal_a: Handle<Image>,
    #[texture(105)]
    normal_b: Handle<Image>,
    #[texture(106)]
    detail_a: Handle<Image>,
    #[texture(107)]
    detail_b: Handle<Image>,
    #[texture(108)]
    rmas_a: Handle<Image>,
    #[texture(109)]
    rmas_b: Handle<Image>,
    #[texture(110)]
    blend_map: Handle<Image>,
    /// Master_Blend_01_POM's Height (its Channel picks the component).
    #[texture(111)]
    height: Handle<Image>,
}

#[derive(ShaderType, Reflect, Debug, Clone)]
struct BlendParams {
    tiling: Vec4,
    blend: Vec4,
    roughness: Vec4,
    detail_strength_a: Vec4,
    detail_strength_b: Vec4,
    tint_a: Vec4,
    tint_b: Vec4,
    /// Desaturation A, B, Specular A, B (tint and curtain modes).
    tint: Vec4,
    /// Curtains: the colour where vertex alpha is 0.
    dirt: Vec4,
    /// Master_Blend_01_POM: Height of Texture, -(1 - ref plane), MinSteps, MaxSteps (0 height: no parallax).
    pom: Vec4,
    /// Its Tiling (x), and Channel.
    pom_tiling: Vec4,
    channel: Vec4,
}

impl MaterialExtension for BlendExt {
    fn fragment_shader() -> ShaderRef {
        "embedded://f13_game/shaders/blend.wgsl".into()
    }
}

#[derive(Asset, AsBindGroup, Reflect, Debug, Clone)]
pub(crate) struct TerrainExt {
    #[uniform(100)]
    params: TerrainParams,
    #[texture(101)]
    weights0: Handle<Image>,
    #[texture(102)]
    weights1: Handle<Image>,
    #[texture(104)]
    #[sampler(103)]
    d0: Handle<Image>,
    #[texture(105)]
    d1: Handle<Image>,
    #[texture(106)]
    d2: Handle<Image>,
    #[texture(107)]
    d3: Handle<Image>,
    #[texture(108)]
    d4: Handle<Image>,
    #[texture(109)]
    d5: Handle<Image>,
    #[texture(110)]
    n0: Handle<Image>,
    #[texture(111)]
    n1: Handle<Image>,
    #[texture(112)]
    n2: Handle<Image>,
    #[texture(113)]
    n3: Handle<Image>,
    #[texture(114)]
    n4: Handle<Image>,
    #[texture(115)]
    n5: Handle<Image>,
    #[texture(116)]
    r0: Handle<Image>,
    #[texture(117)]
    r1: Handle<Image>,
    #[texture(118)]
    r2: Handle<Image>,
    #[texture(119)]
    r3: Handle<Image>,
    #[texture(120)]
    r4: Handle<Image>,
    #[texture(121)]
    r5: Handle<Image>,
}

#[derive(ShaderType, Reflect, Debug, Clone)]
struct TerrainParams {
    mapping_scale: f32,
    layer_count: u32,
    rma_mask: u32,
    _pad: u32,
}

impl MaterialExtension for TerrainExt {
    fn fragment_shader() -> ShaderRef {
        "embedded://f13_game/shaders/terrain.wgsl".into()
    }
}

/// The ported materials, for waiting on their textures (screenshots).
#[derive(bevy::ecs::system::SystemParam)]
pub(crate) struct PortedMaterials<'w> {
    blends: Res<'w, Assets<BlendMaterial>>,
    terrains: Res<'w, Assets<TerrainMaterial>>,
    props: Res<'w, Assets<PropMaterial>>,
}

impl PortedMaterials<'_> {
    pub(crate) fn images(&self) -> impl Iterator<Item = &Handle<Image>> {
        let b = self.blends.iter().flat_map(|(_, m)| {
            let e = &m.extension;
            [&e.color_a, &e.color_b, &e.normal_a, &e.normal_b, &e.detail_a, &e.detail_b, &e.rmas_a, &e.rmas_b, &e.blend_map]
        });
        let t = self.terrains.iter().flat_map(|(_, m)| {
            let e = &m.extension;
            [
                &e.d0, &e.d1, &e.d2, &e.d3, &e.d4, &e.d5, &e.n0, &e.n1, &e.n2, &e.n3, &e.n4, &e.n5, &e.r0, &e.r1, &e.r2,
                &e.r3, &e.r4, &e.r5,
            ]
        });
        let p = self.props.iter().flat_map(|(_, m)| [m.base.base_color_texture.as_ref(), m.base.normal_map_texture.as_ref(), Some(&m.extension.rma)].into_iter().flatten());
        b.chain(t).chain(p)
    }
}

#[derive(Asset, AsBindGroup, Reflect, Debug, Clone)]
pub(crate) struct WaterExt {
    #[uniform(100)]
    params: WaterParams,
    #[texture(101)]
    #[sampler(102)]
    wave_1: Handle<Image>,
    #[texture(103)]
    wave_2: Handle<Image>,
    #[texture(104)]
    wave_far: Handle<Image>,
    #[texture(105)]
    foam: Handle<Image>,
    #[texture(106, dimension = "cube")]
    reflection: Handle<Image>,
}

/// See shaders/water.wgsl.
#[derive(ShaderType, Reflect, Debug, Clone)]
struct WaterParams {
    near: Vec4,
    far: Vec4,
    foam: Vec4,
    color: Vec4,
    fresnel: Vec4,
    edge: Vec4,
    cube_tint: Vec4,
    misc: Vec4,
    surface: Vec4,
}

impl MaterialExtension for WaterExt {
    fn fragment_shader() -> ShaderRef {
        "embedded://f13_game/shaders/water.wgsl".into()
    }
}

fn water_material(info: &MaterialInfo, loader: &Loader, fb: &Fallback, images: &mut Assets<Image>) -> WaterMaterial {
    let tex = |k: &str, or: &Handle<Image>| info.textures.get(k).and_then(|p| loader.load(p, Tex::Data)).unwrap_or_else(|| or.clone());
    let v = |k: &str, or: [f32; 4]| Vec4::from_array(info.vectors.get(k).copied().unwrap_or(or));
    let s = |k: &str, or: f32| info.scalar(k, or);
    // The master's defaults (its uniform expressions) where the instance doesn't set one.
    let params = WaterParams {
        near: Vec4::new(s("SmallWaveTime", 1.0), (s("WaterScale", 1.0) * s("Small Wave Scale", 512.0)).max(1.0), s("NormalPowerNear", 0.5), s("LargeWaveTime", 1.0)),
        far: Vec4::new(
            (s("LArgeWaterScale", 1.0) * s("LArge Wave Scale", 512.0)).max(1.0),
            s("NormalPowerFAR", 0.3),
            s("FarCamerablend", 1500.0).max(1e-3),
            s("FarCamerablendContrast", 3.0),
        ),
        foam: Vec4::new(s("EdgeFoamTime", 0.2), s("edgeFade", 50.0), s("edgeContrast_Power", 0.6), s("edgeContrast", 0.6)),
        color: v("Ocean Water Color", [0.0324, 0.045167, 0.24, 1.0]),
        fresnel: v("Ocean Water Fresnel Color", [0.1975, 0.209646, 0.395, 1.0]),
        edge: v("EdgeColor", [1.0, 0.092422, 0.0, 1.0]),
        cube_tint: v("Reflection CubemapTint", [1.0; 4]),
        misc: Vec4::new(s("Reflection Amount", 0.0), s("epacityDepth", 1e6).max(1e-5), s("depthContrast", 0.1), 0.0),
        surface: Vec4::new(s("rough", 0.01), s("specular", 1.0), s("metalic", 0.08), 0.0),
    };
    // Without its cubemap, no reflection (a black cube).
    let reflection = info.textures.get("Reflection Cubemap").and_then(|p| loader.load(p, Tex::Data)).unwrap_or_else(|| images.add(black_cube()));
    ExtendedMaterial {
        base: StandardMaterial { alpha_mode: AlphaMode::Blend, ..default() },
        extension: WaterExt {
            params,
            wave_1: tex("WaveTexture01", &fb.normal),
            wave_2: tex("WaveTexture02", &fb.normal),
            wave_far: tex("WaveTextureFar01", &fb.normal),
            foam: tex("foam", &fb.black),
            reflection,
        },
    }
}

fn black_cube() -> Image {
    let mut image = Image::new_fill(
        Extent3d { width: 1, height: 1, depth_or_array_layers: 6 },
        TextureDimension::D2,
        &[0, 0, 0, 255],
        TextureFormat::Rgba8Unorm,
        RenderAssetUsages::RENDER_WORLD,
    );
    image.texture_view_descriptor = Some(bevy::render::render_resource::TextureViewDescriptor {
        dimension: Some(bevy::render::render_resource::TextureViewDimension::Cube),
        ..default()
    });
    image
}

#[derive(Asset, AsBindGroup, Reflect, Debug, Clone)]
pub(crate) struct PropExt {
    #[uniform(100)]
    params: PropParams,
    #[texture(101)]
    #[sampler(102)]
    rma: Handle<Image>,
    /// The Chevette's lights mask (black for the other props).
    #[texture(103)]
    #[sampler(104)]
    lights: Handle<Image>,
}

#[derive(ShaderType, Reflect, Debug, Clone)]
struct PropParams {
    mults: Vec4,
    extra: Vec4,
    /// The Chevette's Headlights, Tailights, Backup Lights; w: 1 with its lights mask.
    lights: Vec4,
}

/// Jason's weapon bloodied (his AnimNotify_BloodyWeapon: SCKillerCharacter::ApplyBloodToWeapon(0, 'ShowWeaponBlood',
/// 1) -> MULTICAST_ApplyBloodToWeapon 0x1403f5f40: his weapon's mesh element 0's dynamic material instance's scalar).
#[derive(Event, Clone, Copy, Debug)]
pub struct BloodyWeapon(pub f32);

pub(crate) fn bloody_weapon(ev: On<BloodyWeapon>, mut props: ResMut<Assets<PropMaterial>>) {
    info!("Jason's weapon: ShowWeaponBlood {}", ev.0);
    for (_, m) in props.iter_mut() {
        if m.extension.params.extra.w > 0.5 && m.extension.params.extra.z != ev.0 {
            m.extension.params.extra.z = ev.0;
        }
    }
}

impl PropExt {
    /// M_Prop_Master_Lights' Glow (0 for the other masters).
    pub(crate) fn glow(&self) -> f32 {
        self.params.extra.y
    }

    pub(crate) fn set_glow(&mut self, glow: f32) {
        self.params.extra.y = glow;
    }

    /// The Chevette's exterior (its lights mask).
    pub(crate) fn has_lights(&self) -> bool {
        self.params.lights.w > 0.0
    }

    /// The Chevette's Headlights, Tailights and Backup Lights parameters.
    pub(crate) fn lights(&self) -> Vec4 {
        self.params.lights
    }

    pub(crate) fn set_lights(&mut self, head: f32, tail: f32, backup: f32) {
        self.params.lights = Vec4::new(head, tail, backup, self.params.lights.w);
    }
}

/// M_Vehicle_Chevette_Exterior's lights mask, a texture of the material's own (not a parameter).
const CHEVETTE_LIGHTS: &str = "SummerCamp/Content/Props/Manmade/Vehicles/T_Vehicle_Chevette_Exterior_Lights";

/// The RMA/RSS map and parameters for prop and foliage masters (channel layouts read from M_Prop_Master's and
/// M_Leaves_Master's compiled shaders), or None for other materials.
fn prop_params(info: &MaterialInfo, loader: &Loader) -> Option<(Handle<Image>, PropParams, Option<Handle<Image>>)> {
    // Same RMA sampling (R roughness, G metallic, B AO, times the mults) in M_Item_Master and the Chevette's exterior.
    if info.master.starts_with("M_Prop_Master") || info.master == "M_Item_Master" || info.master == "M_Vehicle_Chevette_Exterior" {
        let rma = loader.load(info.textures.get("RMA")?, Tex::Data)?;
        let mults = Vec4::new(info.scalar("RoughnessMult", 1.0), info.scalar("MetalnessMult", 1.0), info.scalar("Spec", 0.5), 0.0);
        // M_Prop_Master_Lights glows: emissive = Diffuse.rgb * Diffuse.a * Glow (its compiled base pass).
        let glow = if info.master == "M_Prop_Master_Lights" { info.scalar("Glow", 1.0) } else { 0.0 };
        // The Chevette's exterior: emissive = Diffuse.rgb * (mask.r * Headlights + mask.g * Tailights + mask.b * Backup
        // Lights) (its compiled base pass).
        let lights = (info.master == "M_Vehicle_Chevette_Exterior").then(|| loader.load(CHEVETTE_LIGHTS, Tex::Data)).flatten();
        let l = Vec4::new(info.scalar("Headlights", 0.0), info.scalar("Tailights", 0.0), info.scalar("Backup Lights", 0.0), lights.is_some() as u8 as f32);
        return Some((rma, PropParams { mults, extra: Vec4::new(0.0, glow, 0.0, 0.0), lights: l }, lights));
    }
    // Jason's pickaxe (M_JasonV2_PickAxe: its compiled base pass s025, F13-decomp shaders/M_JasonV2_PickAxe): the
    // prop masters' RMA (unscaled), specular 0.5, and its blood layer (MatLayerBlend_Simple of Blood_Layer) by
    // WeaponBloodMask.r x ShowWeaponBlood (extra.z; extra.w 1: the mask in the lights' slot).
    if info.master == "M_JasonV2_PickAxe" {
        info!("material: Jason's pickaxe, with its blood layer");
        let rma = loader.load(info.textures.get("RMA")?, Tex::Data)?;
        let mask = loader.load(info.textures.get("WeaponBloodMask")?, Tex::Data)?;
        let extra = Vec4::new(0.0, 0.0, info.scalar("ShowWeaponBlood", 0.0), 1.0);
        return Some((rma, PropParams { mults: Vec4::new(1.0, 1.0, 0.5, 0.0), extra, lights: Vec4::ZERO }, Some(mask)));
    }
    let rss = ["RSS", "RSS Map"].iter().find_map(|k| info.textures.get(*k))?;
    let rss = loader.load(rss, Tex::Data)?;
    let mults = Vec4::new(info.scalar("RoughnessMultiplier", 1.0), 0.0, info.scalar("Specular", 0.5), 1.0);
    Some((rss, PropParams { mults, extra: Vec4::new(info.scalar("RoughnessMin", 0.0), 0.0, 0.0, 0.0), lights: Vec4::ZERO }, None))
}

/// The two skin masks (see shaders/skin.wgsl).
#[derive(Asset, AsBindGroup, Reflect, Debug, Clone)]
pub(crate) struct SkinMaskExt {
    #[texture(100)]
    #[sampler(101)]
    mask_a: Handle<Image>,
    #[texture(102)]
    mask_b: Handle<Image>,
    #[uniform(103)]
    params: SkinParams,
    /// The body parts' damage, combined (blood.rs; see shaders/skin.wgsl).
    #[texture(104)]
    pub(crate) damage: Handle<Image>,
    #[texture(105)]
    gore_normal: Handle<Image>,
    #[texture(106)]
    gore_rgb: Handle<Image>,
    #[texture(107)]
    colormask: Handle<Image>,
    #[texture(108)]
    rma: Handle<Image>,
}

#[derive(ShaderType, Reflect, Debug, Clone)]
struct SkinParams {
    dark_tint: Vec4,
    /// RoughMin, RoughMax, RoughnessMultiplier, RainGloss.
    rough: Vec4,
    /// 0.35 + Sweat, PoreSpecWeight, Rain_MPC's Rain.
    spec: Vec4,
    selection: Vec4,
}

/// A SkinGoreMaster instance (the master's defaults: uniexpr.py on it), nothing clipped (black masks: the face; the
/// body takes its own: [`MaterialIndex::skin_material`]) and no damage yet.
fn skin_gore_material(root: &Path, info: &MaterialInfo, loader: &Loader, fallback: &Fallback) -> SkinMaterial {
    let v = |name: &str, d: [f32; 4]| info.vectors.get(name).map_or(Vec4::from_array(d), |v| Vec4::from_array(*v));
    let tex = |name: &str, kind: Tex, d: &Handle<Image>| info.textures.get(name).and_then(|p| loader.load(p, kind)).unwrap_or_else(|| d.clone());
    let fixed = |path: &str, kind: Tex, d: &Handle<Image>| loader.load(path, kind).unwrap_or_else(|| d.clone());
    let mut base = standard_material(root, info, loader);
    if !matches!(base.alpha_mode, AlphaMode::Mask(_)) {
        base.alpha_mode = AlphaMode::Mask(0.3333);
    }
    ExtendedMaterial {
        base,
        extension: SkinMaskExt {
            mask_a: fallback.black.clone(),
            mask_b: fallback.black.clone(),
            params: SkinParams {
                dark_tint: v("Skin_diffuse_DarkTint", [1.0; 4]),
                rough: Vec4::new(info.scalar("RoughMin", 0.1), info.scalar("RoughMax", 0.9), v("RoughnessMultiplier", [1.0; 4]).x, info.scalar("RainGloss", 0.55)),
                spec: Vec4::new(0.35 + info.scalar("Sweat", 0.2), info.scalar("PoreSpecWeight", 0.0), 0.0, 0.0),
                selection: v("SelectionColor", [0.0; 4]).with_w(0.0),
            },
            damage: fallback.clear.clone(),
            gore_normal: fixed(GORE_NORMAL, Tex::Data, &fallback.normal),
            gore_rgb: tex("Gore_RGB", Tex::Data, &fixed(GORE_RGB, Tex::Data, &fallback.black)),
            colormask: fixed(GORE_COLORMASK, Tex::Data, &fallback.black),
            rma: tex("Skin RMA", Tex::Data, &fallback.rmas),
        },
    }
}

/// Rain_MPC's Rain on the skin (glossier).
fn skin_rain(mpc: Option<Res<crate::weather::RainMpc>>, mut materials: ResMut<Assets<SkinMaterial>>) {
    let Some(mpc) = mpc.filter(|m| m.is_changed()) else { return };
    for (_, m) in materials.iter_mut() {
        m.extension.params.spec.z = mpc.rain;
    }
}

impl MaterialExtension for SkinMaskExt {
    fn fragment_shader() -> ShaderRef {
        "embedded://f13_game/shaders/skin.wgsl".into()
    }

    fn prepass_fragment_shader() -> ShaderRef {
        "embedded://f13_game/shaders/skin_prepass.wgsl".into()
    }
}

impl MaterialIndex {
    /// The opacity mask's alpha cutoff of one of Jason's masked materials (its masters' 0_AlphaDelta fade) at an
    /// AlphaDelta, from their compiled masked depth shaders (MODLOG "Jason's fade"); None for any other material.
    /// With a = AlphaDelta x 4.5, B = 0_BaseContrastLevel, F = 0_ContrastFadeConstant, M = 0_AlphaContrastMultiplier:
    /// - ClothMaster_Jason_m, SkinMaster_jason_m: alpha x saturate(saturate(B F (1 + 2a) - a) x M) >= 0.3333;
    /// - MiscMaster_jason_m, EyeMaster_02_jason_m: saturate(saturate(B F (1 + 2a) - a) x M) >= 0.3333 (no texture);
    /// - HairMaster_jason_m: saturate(saturate(alpha^AlphaContrast B F (1 + 2a) - a) x M) >= 0.3333.
    /// (Their base passes add a temporal dither, DitherTemporalAA, before the clip: without temporal AA here the hard
    /// cutoff of the shadow pass is used for both.)
    pub(crate) fn fade_cutoff(&self, name: &str, alpha_delta: f32) -> Option<f32> {
        let info = self.entries.get(&name.to_lowercase())?;
        let a = alpha_delta * 4.5;
        let (b, f, m) = (info.scalar("0_BaseContrastLevel", 1.0), info.scalar("0_ContrastFadeConstant", 0.45), info.scalar("0_AlphaContrastMultiplier", 20.0));
        let clip = 0.3333;
        let gate = ((b * f * (1.0 + 2.0 * a) - a).clamp(0.0, 1.0) * m).clamp(0.0, 1.0);
        let never = 2.0;
        match info.master.to_lowercase().as_str() {
            "clothmaster_jason_m" | "skinmaster_jason_m" => Some(if gate > 0.0 { clip / gate } else { never }),
            "miscmaster_jason_m" | "eyemaster_02_jason_m" => Some(if gate >= clip { 0.0 } else { never }),
            "hairmaster_jason_m" => {
                // saturate(y) x M >= clip  <=>  y >= clip / M; y = alpha^C B F (1 + 2a) - a.
                let need = (clip / m + a) / ((1.0 + 2.0 * a) * b * f);
                Some(if need > 1.0 { never } else { need.max(0.0).powf(1.0 / info.scalar("AlphaContrast", 2.0)) })
            }
            _ => None,
        }
    }

    /// A skin material clipped by two exported masks (export-relative texture paths), on top of `base` (the skin's
    /// own material as built from the export). None if a mask is missing from the export.
    /// A skin material clipped by the body's masks (its own, then the outfit's).
    pub(crate) fn skin_material(&self, skin: SkinMaterial, masks: &[String], assets: &AssetServer) -> Option<SkinMaterial> {
        let loader = Loader { root: &self.root, assets };
        let [a, b] = masks else { return None };
        let mut skin = skin;
        skin.extension.mask_a = loader.load(a, Tex::Data)?;
        skin.extension.mask_b = loader.load(b, Tex::Data)?;
        Some(skin)
    }
}

#[derive(Asset, AsBindGroup, Reflect, Debug, Clone)]
pub(crate) struct MasterExt {
    #[uniform(100)]
    params: MasterParams,
    #[texture(101)]
    #[sampler(102)]
    tex: Handle<Image>,
    #[texture(103)]
    tex2: Handle<Image>,
    #[texture(104)]
    tex3: Handle<Image>,
}

#[derive(ShaderType, Reflect, Debug, Clone)]
struct MasterParams {
    /// Mode, Power, FlickerOffset (shaders/masters.wgsl).
    mode: Vec4,
    a: Vec4,
    b: Vec4,
}

impl MaterialExtension for MasterExt {
    fn fragment_shader() -> ShaderRef {
        "embedded://f13_game/shaders/masters.wgsl".into()
    }
}

/// The sky's material takes Rain_MPC's LightiningIntensity (weather.rs) as it changes.
fn sky_lightning(mpc: Option<Res<crate::weather::RainMpc>>, mut materials: ResMut<Assets<MasterMaterial>>) {
    let Some(mpc) = mpc.filter(|m| m.is_changed()) else { return };
    let skies: Vec<_> = materials.iter().filter(|(_, m)| m.extension.params.mode.x == 3.0).map(|(id, _)| id).collect();
    for id in skies {
        if let Some(mut m) = materials.get_mut(id) {
            m.extension.params.mode.w = mpc.lightning;
        }
    }
}

/// The one-off masters read from their compiled base passes (F13-decomp shaders/<master>, MODLOG "Emissive masters"),
/// or None for any other material. Their colours are the shaders' constants (linear).
fn master_material(root: &Path, info: &MaterialInfo, loader: &Loader) -> Option<MasterMaterial> {
    let mut base = standard_material(root, info, loader);
    base.perceptual_roughness = 0.5;
    let flicker = || loader.load("SummerCamp/Content/Art/Textures/Flicker2", Tex::Data);
    let v = |c: [f32; 3], w: f32| Vec4::new(c[0], c[1], c[2], w);
    let power = info.scalar("Power", 0.0);
    // The pulse: c = sin(1.256637 t) * a + b; the phone box's base colour is c (a.w = 1).
    let pulse = |a: [f32; 3], b: [f32; 3], own_base: bool| MasterParams { mode: Vec4::new(0.0, power, 0.0, 0.0), a: v(a, own_base as u8 as f32), b: v(b, 0.0) };
    let mut extra: Option<(Handle<Image>, Handle<Image>)> = None;
    let (params, tex) = match info.master.as_str() {
        "M_Interactive_TelephoneBox_01_Emissive_Blue" => (pulse([0.0, 0.007812, 0.44], [0.0, 0.007812, 0.44], true), flicker()?),
        "M_Interactive_TelephoneBox_01_Emissive_Yellow" => (pulse([0.176, 0.102667, 0.0], [0.704, 0.410666, 0.0], true), flicker()?),
        "M_Camp_CBRadio_Tower_01_Emissive" => {
            base.base_color = LinearRgba::rgb(0.88, 0.008238, 0.0).into();
            (pulse([0.264, 0.002471, 0.0], [0.616, 0.005767, 0.0], false), flicker()?)
        }
        "M_Interactive_TelephoneBox_01_Emissive_Red" => {
            base.base_color = Color::BLACK;
            let mode = Vec4::new(1.0, 0.0, info.scalar("FlickerOffset", 0.0), 0.0);
            (MasterParams { mode, a: Vec4::ZERO, b: Vec4::ZERO }, flicker()?)
        }
        "M_Signs_Cabin_Names_01" => {
            // Specular 0.1: F0 0.08 x 0.1 = 0.16 reflectance^2.
            base.reflectance = (0.5f32 * 0.1).sqrt();
            let rma = loader.load("SummerCamp/Content/Props/Manmade/Signs/Textures/T_Signs_Cabin_Names_01_RMA", Tex::Data)?;
            (MasterParams { mode: Vec4::new(2.0, 0.0, 0.0, 0.0), a: Vec4::ZERO, b: Vec4::ZERO }, rma)
        }
        "M_Sky" => {
            // uniexpr.py M_Sky: ColorTint, ColorTint2, Overall, Desaturation, Overall2, StarBrightness; its Texture,
            // AlphaMask and the fixed T_Stars.
            base.unlit = true;
            let tex = |n: &str| info.textures.get(n).and_then(|p| loader.load(p, Tex::Color));
            let (t, mask) = (tex("Texture")?, info.textures.get("AlphaMask").and_then(|p| loader.load(p, Tex::Data))?);
            let stars = loader.load("SummerCamp/Content/SkyDome/_OldSky/Textures/T_Stars", Tex::Color)?;
            let c = |n: &str| info.vectors.get(n).copied().unwrap_or([1.0; 4]);
            let (c1, c2) = (c("ColorTint"), c("ColorTint2"));
            extra = Some((mask, stars));
            let params = MasterParams {
                mode: Vec4::new(3.0, info.scalar("Desaturation", 0.8), info.scalar("StarBrightness", 1.0), 0.0),
                a: Vec4::new(c1[0], c1[1], c1[2], info.scalar("Overall", 0.01)),
                b: Vec4::new(c2[0], c2[1], c2[2], info.scalar("Overall2", 0.08)),
            };
            (params, t)
        }
        "EyeMaster_LowerLid" => {
            // BaseColor, Roughness, Specular (its Subsurface shading's SubsurfaceColor is black: lit as default).
            let c = info.vectors.get("BaseColor").copied().unwrap_or([0.5, 0.5, 0.5, 1.0]);
            base.base_color = LinearRgba::rgb(c[0], c[1], c[2]).into();
            base.perceptual_roughness = info.scalar("Roughness", 0.5);
            base.reflectance = (0.5f32 * info.scalar("Specular", 0.5)).sqrt();
            base.alpha_mode = AlphaMode::Blend;
            (MasterParams { mode: Vec4::new(4.0, info.scalar("FresnelPower", 1.0), 0.0, 0.0), a: Vec4::ZERO, b: Vec4::ZERO }, flicker()?)
        }
        _ => return None,
    };
    let (tex2, tex3) = extra.unwrap_or_else(|| (tex.clone(), tex.clone()));
    Some(ExtendedMaterial { base, extension: MasterExt { params, tex, tex2, tex3 } })
}

#[derive(Asset, AsBindGroup, Reflect, Debug, Clone)]
pub(crate) struct ClothesExt {
    #[uniform(100)]
    params: ClothesParams,
    #[texture(101)]
    #[sampler(102)]
    gore_normal: Handle<Image>,
    /// The body parts' damage, combined (character.rs blood; see shaders/clothes.wgsl).
    #[texture(103)]
    pub(crate) damage: Handle<Image>,
    #[texture(104)]
    gore_rgb: Handle<Image>,
    #[texture(105)]
    pattern: Handle<Image>,
    #[texture(106)]
    pattern2: Handle<Image>,
    #[texture(107)]
    colormask: Handle<Image>,
    #[texture(108)]
    rma: Handle<Image>,
}

#[derive(ShaderType, Reflect, Debug, Clone)]
struct ClothesParams {
    hue: Vec4,
    tile_tint: Vec4,
    logo_tint: Vec4,
    extra_tint: Vec4,
    tints: Vec4,
    pattern: Vec4,
    /// addGreenChannel_TP2, TilePatternToggle, RainGloss, Rain_MPC's Rain.
    more: Vec4,
    selection: Vec4,
}

impl MaterialExtension for ClothesExt {
    fn fragment_shader() -> ShaderRef {
        "embedded://f13_game/shaders/clothes.wgsl".into()
    }
}

/// ClothesMultiGore's textures of its own (its layer functions' samples, the Gore_Layer's and Gore_Burn_Layer's).
const GORE_NORMAL: &str = "SummerCamp/Content/Characters/Counselors/Gore/DismembermentMasks/UV2_Gore_normal";
const GORE_COLORMASK: &str = "SummerCamp/Content/Characters/Counselors/Gore/DismembermentMasks/UV2_Gore_Colormask";
const GORE_RGB: &str = "SummerCamp/Content/Characters/Counselors/Gore/Textures/gore_burn_RGB_mask_01";

/// A ClothesMultiGore instance: its parameters (the master's defaults: uniexpr.py on the master), the cloth's
/// textures, and no damage yet.
fn clothes_material(root: &Path, info: &MaterialInfo, loader: &Loader, fallback: &Fallback) -> ClothesMaterial {
    let v = |name: &str, d: [f32; 4]| info.vectors.get(name).map_or(Vec4::from_array(d), |v| Vec4::from_array(*v));
    let tex = |name: &str, kind: Tex, d: &Handle<Image>| info.textures.get(name).and_then(|p| loader.load(p, kind)).unwrap_or_else(|| d.clone());
    let fixed = |path: &str, kind: Tex, d: &Handle<Image>| loader.load(path, kind).unwrap_or_else(|| d.clone());
    let mut base = standard_material(root, info, loader);
    // (Its masked opacity, RMA.a with the temporal dither, is 1 on the counselors' clothes: drawn opaque.)
    base.alpha_mode = AlphaMode::Opaque;
    ClothesMaterial {
        base,
        extension: ClothesExt {
            params: ClothesParams {
                hue: v("Cloth_diffuse_HueTint", [0.5, 0.5, 0.5, 1.0]),
                tile_tint: v("TileTint", [0.87, 0.562009, 0.030654, 1.0]),
                logo_tint: v("TileTint_logo", [0.87, 0.214361, 0.657086, 1.0]),
                extra_tint: v("Tint_AdditionalElements", [0.87, 0.214361, 0.657086, 1.0]),
                tints: Vec4::new(info.scalar("Cloth_Diffuse_Desaturation", 0.0), info.scalar("Cloth_diffuse_LIghtTint", 0.0), info.scalar("Cloth_diffuse_DarkTint", 1.0), info.scalar("TilePatternTile", 1.0)),
                pattern: Vec4::new(info.scalar("PAttern_IsolationMAsk", 0.0), info.scalar("InvertTilePatternMask", 0.0), info.scalar("addGreenChannel", 0.0), info.scalar("addblueChannel", 0.0)),
                more: Vec4::new(info.scalar("addGreenChannel_TP2", 0.0), info.scalar("TilePatternToggle", 0.0), info.scalar("RainGloss", 0.4), 0.0),
                selection: v("SelectionColor", [0.0; 4]).with_w(0.0),
            },
            gore_normal: fixed(GORE_NORMAL, Tex::Data, &fallback.normal),
            damage: fallback.clear.clone(),
            gore_rgb: tex("Gore_RGB", Tex::Data, &fixed(GORE_RGB, Tex::Data, &fallback.black)),
            pattern: tex("TilePAttern", Tex::Color, &fallback.white),
            pattern2: tex("TilePAttern2", Tex::Data, &fallback.white),
            colormask: fixed(GORE_COLORMASK, Tex::Data, &fallback.black),
            rma: tex("Clothing RMA", Tex::Data, &fallback.rmas),
        },
    }
}

pub(crate) type HairMaterial = ExtendedMaterial<StandardMaterial, HairExt>;

#[derive(Asset, AsBindGroup, Reflect, Debug, Clone)]
pub(crate) struct HairExt {
    #[uniform(100)]
    params: HairParams,
    #[sampler(101)]
    #[texture(102)]
    tex_a: Handle<Image>,
    #[texture(103)]
    tex_b: Handle<Image>,
    #[texture(104)]
    tex_c: Handle<Image>,
    #[texture(105)]
    tex_d: Handle<Image>,
}

#[derive(ShaderType, Reflect, Debug, Clone)]
struct HairParams {
    color1: Vec4,
    color2: Vec4,
    color3: Vec4,
    /// Diffuse Brightness, Haircolor 1 Specular, Haircolor 2 Specular, Roughness.
    a: Vec4,
    /// HairAlphaContrast, mode (0 HairMaster_03_m, 1 M_Athlete_Girl_Hair_Inst).
    b: Vec4,
}

impl MaterialExtension for HairExt {
    fn fragment_shader() -> ShaderRef {
        "embedded://f13_game/shaders/hair.wgsl".into()
    }

    fn prepass_fragment_shader() -> ShaderRef {
        "embedded://f13_game/shaders/hair_prepass.wgsl".into()
    }
}

/// A counselor hair material (shaders/hair.wgsl): HairMaster_03_m's parameters (the master's defaults: uniexpr.py on
/// it), or M_Athlete_Girl_Hair_Inst's own textures; two-sided, its mesh keeping its vertex colours.
fn hair_material(root: &Path, info: &MaterialInfo, loader: &Loader, fallback: &Fallback) -> HairMaterial {
    let v = |name: &str, d: [f32; 4]| info.vectors.get(name).map_or(Vec4::from_array(d), |v| Vec4::from_array(*v));
    let tex = |name: &str, kind: Tex, d: &Handle<Image>| info.textures.get(name).and_then(|p| loader.load(p, kind)).unwrap_or_else(|| d.clone());
    let fixed = |path: &str, kind: Tex, d: &Handle<Image>| loader.load(path, kind).unwrap_or_else(|| d.clone());
    let mut base = standard_material(root, info, loader);
    base.base_color_texture = None;
    base.alpha_mode = AlphaMode::Opaque;
    base.double_sided = true;
    base.cull_mode = None;
    let athlete = info.master == "M_Athlete_Girl_Hair_Inst";
    const OWN: &str = "SummerCamp/Content/Characters/Counselors/Athlete/Textures/T_Athlete_Girl_Hair";
    if athlete {
        base.normal_map_texture = loader.load(&format!("{OWN}_N"), Tex::Data);
    }
    ExtendedMaterial {
        base,
        extension: HairExt {
            params: HairParams {
                color1: v("Diffuse Hair Color 1", [0.185, 0.095784, 0.047175, 1.0]),
                color2: v("Diffuse Hair Color 2", [0.95, 0.399, 0.656799, 1.0]),
                color3: v("HairColor3", [1.0, 0.605633, 0.12, 1.0]),
                a: Vec4::new(info.scalar("Diffuse Brightness", 3.0), info.scalar("Haircolor 1 Specular", 0.12), info.scalar("Haircolor 2 Specular", 0.1), info.scalar("Roughness", 0.35)),
                b: Vec4::new(info.scalar("HairAlphaContrast", 2.0), if athlete { 1.0 } else { 0.0 }, 0.0, 0.0),
            },
            tex_a: if athlete { fixed(&format!("{OWN}_D"), Tex::Color, &fallback.white) } else { tex("Diffuse 2 Hair Mask", Tex::Data, &fallback.white) },
            tex_b: tex("Mask", Tex::Data, &fallback.white),
            tex_c: if athlete { fixed(&format!("{OWN}_RMA"), Tex::Data, &fallback.rmas) } else { tex("Hair Diffuse Texture", Tex::Color, &fallback.white) },
            tex_d: tex("RootMask", Tex::Data, &fallback.white),
        },
    }
}

pub(crate) type GoreCapMaterial = ExtendedMaterial<StandardMaterial, GoreCapExt>;

#[derive(Asset, AsBindGroup, Reflect, Debug, Clone)]
pub(crate) struct GoreCapExt {
    #[texture(100)]
    #[sampler(101)]
    diffuse: Handle<Image>,
    #[texture(102)]
    ao: Handle<Image>,
    #[uniform(103)]
    selection: Vec4,
}

impl MaterialExtension for GoreCapExt {
    fn fragment_shader() -> ShaderRef {
        "embedded://f13_game/shaders/gorecap.wgsl".into()
    }
}

/// A GoreCap instance (shaders/gorecap.wgsl): its normal and two-sidedness on the StandardMaterial part, the meat's
/// colour and AO (GoreCap_Meat_Layer's own samples); its mesh keeps its vertex colours (the blood).
fn gore_cap_material(root: &Path, info: &MaterialInfo, loader: &Loader, fallback: &Fallback) -> GoreCapMaterial {
    let fixed = |path: &str, kind: Tex, d: &Handle<Image>| loader.load(path, kind).unwrap_or_else(|| d.clone());
    let mut base = standard_material(root, info, loader);
    base.base_color_texture = None;
    base.alpha_mode = AlphaMode::Opaque;
    ExtendedMaterial {
        base,
        extension: GoreCapExt {
            diffuse: fixed("SummerCamp/Content/Characters/Counselors/Gore/GoreCAps/gorecap_meat_D", Tex::Color, &fallback.white),
            ao: fixed("SummerCamp/Content/Characters/Counselors/Gore/GoreCAps/gorecap_meat_AO", Tex::Data, &fallback.white),
            selection: info.vectors.get("SelectionColor").map_or(Vec4::ZERO, |v| Vec4::from_array(*v)).with_w(0.0),
        },
    }
}

/// Rain_MPC's Rain on the clothes (Clothes_Layer's Rain: darker, glossier).
fn clothes_rain(mpc: Option<Res<crate::weather::RainMpc>>, mut materials: ResMut<Assets<ClothesMaterial>>) {
    let Some(mpc) = mpc.filter(|m| m.is_changed()) else { return };
    for (_, m) in materials.iter_mut() {
        m.extension.params.more.w = mpc.rain;
    }
}

impl MaterialExtension for PropExt {
    fn fragment_shader() -> ShaderRef {
        "embedded://f13_game/shaders/prop.wgsl".into()
    }
}

/// How a texture is sampled: colour (sRGB) or data (linear), both repeating.
#[derive(Clone, Copy, PartialEq)]
enum Tex {
    Color,
    Data,
}

struct Loader<'a> {
    root: &'a Path,
    assets: &'a AssetServer,
}

impl Loader<'_> {
    /// Loads an exported texture (export-relative path without extension), or None if the export lacks it. Prefers
    /// the DDS copy (the game's compressed data with mips, sRGB as the game flags it) over the PNG.
    fn load(&self, path: &str, kind: Tex) -> Option<Handle<Image>> {
        let dds = format!("{path}.dds");
        let (file, srgb) = match dds_srgb(&self.root.join(&dds)) {
            Some(srgb) => (dds, srgb),
            None => {
                let png = format!("{path}.png");
                if !self.root.join(&png).exists() {
                    return None;
                }
                (png, kind == Tex::Color)
            }
        };
        // The texture's AddressX/Y (texture_address.json: those that don't wrap).
        let [u, v] = texture_address(self.root, path);
        Some(self.assets.load_builder().with_settings(move |s: &mut ImageLoaderSettings| {
            s.is_srgb = srgb;
            s.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor {
                address_mode_u: u,
                address_mode_v: v,
                anisotropy_clamp: 8,
                ..ImageSamplerDescriptor::linear()
            });
        }).load(file))
    }
}

/// A texture's address modes (UTexture2D AddressX/Y; f13-export's texture_address.json lists those not TA_Wrap).
fn texture_address(root: &Path, path: &str) -> [bevy::image::ImageAddressMode; 2] {
    use bevy::image::ImageAddressMode as A;
    static INDEX: std::sync::OnceLock<HashMap<String, [String; 2]>> = std::sync::OnceLock::new();
    let index = INDEX.get_or_init(|| std::fs::read_to_string(root.join("texture_address.json")).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default());
    let mode = |m: &str| match m {
        "TA_Clamp" => A::ClampToEdge,
        "TA_Mirror" => A::MirrorRepeat,
        _ => A::Repeat,
    };
    index.get(path).map_or([A::Repeat; 2], |[x, y]| [mode(x), mode(y)])
}

/// An exported texture (export-relative path without extension) as the materials load it (see [`Loader::load`]):
/// `color` for colour data (sRGB when it comes as a PNG).
pub(crate) fn load_texture(root: &Path, assets: &AssetServer, path: &str, color: bool) -> Option<Handle<Image>> {
    Loader { root, assets }.load(path, if color { Tex::Color } else { Tex::Data })
}

/// Whether an exported DDS holds sRGB data (its DX10 header's DXGI format), or None if there is no readable DDS.
/// Bevy's DDS loader takes sRGB from the load settings, not the file.
fn dds_srgb(path: &Path) -> Option<bool> {
    use std::io::Read;
    let mut header = [0u8; 132];
    std::fs::File::open(path).ok()?.read_exact(&mut header).ok()?;
    if &header[0..4] != b"DDS " || &header[84..88] != b"DX10" {
        return None;
    }
    let dxgi = u32::from_le_bytes(header[128..132].try_into().ok()?);
    // BC1/BC2/BC3/BC7/BGRA8 sRGB variants.
    Some(matches!(dxgi, 72 | 75 | 78 | 99 | 91))
}

/// Swaps the placeholder glTF materials for ones built from the export.
fn apply_materials(
    mut commands: Commands,
    added: Query<(Entity, &GltfMaterialName), Added<GltfMaterialName>>,
    parents: Query<&ChildOf>,
    roots: Query<&WorldAssetRoot>,
    overrides: Query<&MaterialOverrides>,
    skin_masks: Query<&SkinMasks>,
    mut skins: ResMut<Assets<SkinMaterial>>,
    mut index: ResMut<MaterialIndex>,
    (mut standard, mut blends, mut terrains, mut waters, mut images): (
        ResMut<Assets<StandardMaterial>>,
        ResMut<Assets<BlendMaterial>>,
        ResMut<Assets<TerrainMaterial>>,
        ResMut<Assets<WaterMaterial>>,
        ResMut<Assets<Image>>,
    ),
    (mut props, mut masters, mut clothes, mut gore_caps, mut hairs): (ResMut<Assets<PropMaterial>>, ResMut<Assets<MasterMaterial>>, ResMut<Assets<ClothesMaterial>>, ResMut<Assets<GoreCapMaterial>>, ResMut<Assets<HairMaterial>>),
    assets: Res<AssetServer>,
) {
    let index = &mut *index;
    for (entity, name) in &added {
        // A placement or class component can swap the mesh's own materials.
        let mut name = name.0.as_str();
        if let Some(o) = parents.iter_ancestors(entity).find_map(|e| overrides.get(e).ok()) {
            name = o.0.get(&name.to_lowercase()).map_or(name, String::as_str);
        }
        let Some(info) = index.entries.get(&name.to_lowercase()) else { continue };
        let fallback = index.fallback.get_or_insert_with(|| Fallback::new(&mut images)).clone();
        let loader = Loader { root: &index.root, assets: &assets };
        // A landscape's weight maps sit next to its glTF, so terrain materials are built per landscape file.
        let landscape_file = info.landscape.as_ref().and_then(|_| {
            parents.iter_ancestors(entity).find_map(|e| roots.get(e).ok()).and_then(|r| assets.get_path(r.0.id())).map(|p| p.path().to_path_buf())
        });
        let key = match &landscape_file {
            Some(f) => format!("{}|{}", name.to_lowercase(), f.display()),
            None => name.to_lowercase(),
        };
        let built = match index.built.get(&key) {
            Some(b) => b.clone(),
            None => {
                let b = if let (Some(land), Some(file)) = (&info.landscape, &landscape_file) {
                    Built::Terrain(terrains.add(terrain_material(info, land, &index.root.join(file).with_extension(""), &loader, &fallback, &mut images)))
                } else if info.master.contains("Glass") {
                    Built::Standard(standard.add(glass_material(info)))
                } else if info.blend != "BLEND_Opaque" && info.blend != "BLEND_Masked" && info.textures.is_empty() && !info.master.starts_with(WATER_MASTER) {
                    Built::Hidden
                } else if info.master.starts_with("EyeMaster_") && info.textures.is_empty() && info.master != "EyeMaster_LowerLid" {
                    // EyeMaster_Iris: every compiled pass of it discards (discard_nz l(-1): its OpacityMask a constant
                    // 0); the game never draws it either.
                    Built::Hidden
                } else if info.master.starts_with(WATER_MASTER) {
                    Built::Water(waters.add(water_material(info, &loader, &fallback, &mut images)))
                } else if let Some(kind) = blend_kind(&info.master) {
                    // (The POM variant's parallax is not ported: drawn flat.)
                    Built::Blend(blends.add(blend_material(info, kind, &loader, &fallback)))
                } else if info.master == "SkinGoreMaster" {
                    Built::Skin(skins.add(skin_gore_material(&index.root, info, &loader, &fallback)))
                } else if info.master == "HairMaster_03_m" || info.master == "M_Athlete_Girl_Hair_Inst" {
                    Built::Hair(hairs.add(hair_material(&index.root, info, &loader, &fallback)))
                } else if info.master == "GoreCap" {
                    Built::GoreCap(gore_caps.add(gore_cap_material(&index.root, info, &loader, &fallback)))
                } else if info.master == "ClothesMultiGore" {
                    Built::Clothes(clothes.add(clothes_material(&index.root, info, &loader, &fallback)))
                } else if let Some(m) = master_material(&index.root, info, &loader) {
                    Built::Master(masters.add(m))
                } else if let Some((rma, params, lights)) = prop_params(info, &loader) {
                    let lights = lights.unwrap_or_else(|| fallback.black.clone());
                    Built::Prop(props.add(ExtendedMaterial { base: standard_material(&index.root, info, &loader), extension: PropExt { params, rma, lights } }))
                } else {
                    let mut m = standard_material(&index.root, info, &loader);
                    if info.master == "M_Lightsources_Ceiling_Bulbs" {
                        // Its compiled base pass: the textures' colour and normal, roughness 0.5, metallic 0, specular 0.5.
                        m.perceptual_roughness = 0.5;
                    }
                    Built::Standard(standard.add(m))
                };
                index.built.insert(key.clone(), b.clone());
                b
            }
        };
        let mut e = commands.entity(entity);
        if let (Built::Skin(h), Some(masks)) = (&built, parents.iter_ancestors(entity).find_map(|a| skin_masks.get(a).ok())) {
            let key = format!("{key}|skin");
            let skin = match index.skins.get(&key) {
                Some(s) => Some(s.clone()),
                None => {
                    let s = skins.get(h).cloned().and_then(|skin| index.skin_material(skin, &masks.0, &assets)).map(|m| skins.add(m));
                    if let Some(s) = &s {
                        index.skins.insert(key, s.clone());
                    }
                    s
                }
            };
            if let Some(skin) = skin {
                e.remove::<MeshMaterial3d<StandardMaterial>>().insert(MeshMaterial3d(skin));
                continue;
            }
        }
        match built {
            Built::Standard(h) => {
                e.insert(MeshMaterial3d(h));
            }
            Built::Blend(h) => {
                e.remove::<MeshMaterial3d<StandardMaterial>>().insert((MeshMaterial3d(h), KeepVertexColors));
            }
            Built::Terrain(h) => {
                e.remove::<MeshMaterial3d<StandardMaterial>>().insert(MeshMaterial3d(h));
            }
            Built::Water(h) => {
                e.remove::<MeshMaterial3d<StandardMaterial>>().insert(MeshMaterial3d(h));
            }
            Built::Prop(h) => {
                e.remove::<MeshMaterial3d<StandardMaterial>>().insert(MeshMaterial3d(h));
            }
            Built::Master(h) => {
                e.remove::<MeshMaterial3d<StandardMaterial>>().insert(MeshMaterial3d(h));
            }
            Built::Clothes(h) => {
                e.remove::<MeshMaterial3d<StandardMaterial>>().insert(MeshMaterial3d(h));
            }
            Built::Skin(h) => {
                e.remove::<MeshMaterial3d<StandardMaterial>>().insert(MeshMaterial3d(h));
            }
            Built::GoreCap(h) => {
                e.remove::<MeshMaterial3d<StandardMaterial>>().insert((MeshMaterial3d(h), KeepVertexColors));
            }
            Built::Hair(h) => {
                e.remove::<MeshMaterial3d<StandardMaterial>>().insert((MeshMaterial3d(h), KeepVertexColors));
            }
            Built::Hidden => {
                e.insert(Visibility::Hidden);
            }
        }
    }
}

fn standard_material(root: &Path, info: &MaterialInfo, loader: &Loader) -> StandardMaterial {
    let mut m = StandardMaterial { perceptual_roughness: 0.8, ..default() };
    match info.blend.as_str() {
        // The game's mask threshold (foliage masters expose it as "Opacity"; Unreal's default clip value is 0.3333).
        "BLEND_Masked" => m.alpha_mode = AlphaMode::Mask(info.scalars.get("Opacity").copied().unwrap_or(0.3333)),
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
    // The material's own parameters first (prop masters call them Diffuse/Normal), then CUE4Parse's guesses (PM_*),
    // which sometimes pick another layer's texture (rocks: the cliff texture of a world-aligned blend).
    let tex = |keys: &[&str]| {
        keys.iter().find_map(|k| {
            info.textures.get(*k).cloned().or_else(|| {
                json["Textures"][*k]["ObjectPath"].as_str().map(|p| p.rsplit_once('.').map_or(p, |(a, _)| a).to_owned())
            })
        })
    };
    m.base_color_texture = tex(&["Diffuse", "BaseColor", "Diffuse/Opacity", "Skin Diffuse", "Clothing Diffuse", "EyeDiffuse", "HairAlpha", "PM_Diffuse"]).and_then(|p| loader.load(&p, Tex::Color));
    if let Some(n) = tex(&["Normal", "normalMap", "Skin Normal", "Clothing Normal", "PM_Normals"]).and_then(|p| loader.load(&p, Tex::Data)) {
        m.normal_map_texture = Some(n);
        m.flip_normal_map_y = true; // Unreal normal maps are DirectX-style (green down)
    }
    m
}

/// Glass (windows, car windows, lamp glass): see-through, dark and glossy. The glass masters' reflections
/// (cubemaps, raindrops) are not ported.
fn glass_material(info: &MaterialInfo) -> StandardMaterial {
    StandardMaterial {
        base_color: Color::srgba(0.05, 0.06, 0.06, 0.25),
        perceptual_roughness: 0.05,
        alpha_mode: AlphaMode::Blend,
        double_sided: info.two_sided,
        cull_mode: (!info.two_sided).then_some(bevy::render::render_resource::Face::Back),
        ..default()
    }
}

/// Which of the blend family a master is (shaders/blend.wgsl `mode`).
#[derive(Clone, Copy, PartialEq)]
enum BlendKind {
    /// Master_Blend_01(_POM): blend by vertex alpha, blend map and B's height.
    Blend = 0,
    /// Master_Tint_Blend_01: no height term; each set tinted by its alpha and desaturated; specular parameters.
    Tint = 1,
    /// Master_Tint_Blend_Curtains_01: one set (B), tinted, faded from DirtColor by vertex alpha.
    Curtains = 2,
}

fn blend_kind(master: &str) -> Option<BlendKind> {
    match master {
        "Master_Blend_01" | "Master_Blend_01_POM" => Some(BlendKind::Blend),
        "Master_Tint_Blend_01" => Some(BlendKind::Tint),
        "Master_Tint_Blend_Curtains_01" => Some(BlendKind::Curtains),
        _ => None,
    }
}

fn blend_material(info: &MaterialInfo, kind: BlendKind, loader: &Loader, fb: &Fallback) -> BlendMaterial {
    let tex = |names: &[&str], t: Tex, or: &Handle<Image>| {
        names.iter().find_map(|n| info.textures.get(*n).and_then(|p| loader.load(p, t))).unwrap_or_else(|| or.clone())
    };
    let curtains = kind == BlendKind::Curtains;
    let s = |name: &str, curtain_name: &str, default: f32| info.scalar(if curtains { curtain_name } else { name }, default);
    let spec_b = s("Specular B", "Specular", 0.5);
    ExtendedMaterial {
        base: StandardMaterial { double_sided: info.two_sided, cull_mode: (!info.two_sided).then_some(bevy::render::render_resource::Face::Back), ..default() },
        extension: BlendExt {
            params: BlendParams {
                tiling: Vec4::new(
                    info.scalar("TilingRate A", 1.0),
                    info.scalar("Detail Tiling A", 1.0),
                    s("TilingRate B", "TilingRate", 1.0),
                    info.scalar("Detail Tiling B", 1.0),
                ),
                blend: Vec4::new(info.scalar("Blend TilingRate", 1.0), info.scalar("Blend Sharpness", 0.0), kind as u32 as f32, 0.0),
                roughness: Vec4::new(
                    info.scalar("Roughness Min A", 0.0),
                    info.scalar("Roughness Max A", 1.0),
                    s("Roughness Min B", "Roughness Min", 0.0),
                    s("Roughness Max B", "Roughness Max", 1.0),
                ),
                detail_strength_a: info.vector("Detail Strength A"),
                detail_strength_b: if curtains { info.vector("Detail Strength") } else { info.vector("Detail Strength B") },
                tint_a: info.vector("Tint A"),
                tint_b: info.vector("Tint b"),
                tint: Vec4::new(info.scalar("Desaturation A", 0.0), info.scalar("Desaturation B", 0.0), info.scalar("Specular A", 0.5), spec_b),
                dirt: info.vector("DirtColor"),
                // F13_NO_POM: drawn flat (comparisons).
                pom: if info.master == "Master_Blend_01_POM" && std::env::var_os("F13_NO_POM").is_none() {
                    Vec4::new(
                        info.scalar("Height of Texture", 0.04),
                        -(1.0 - info.scalar("ref plane", 1.0)),
                        info.scalar("MinSteps", 4.0),
                        info.scalar("MaxSteps", 40.0),
                    )
                } else {
                    Vec4::ZERO
                },
                pom_tiling: Vec4::new(info.scalar("Tiling", 1.0), 0.0, 0.0, 0.0),
                channel: info.vectors.get("Channel").map_or(Vec4::X, |c| Vec4::from_array(*c)),
            },
            color_a: tex(&["ColorMap A"], Tex::Color, &fb.white),
            color_b: tex(&["ColorMap B"], Tex::Color, &fb.white),
            normal_a: tex(&["Normal Map A"], Tex::Data, &fb.normal),
            normal_b: tex(&["Normal Map B", "Normal Map"], Tex::Data, &fb.normal),
            detail_a: tex(&["Detail Normal A"], Tex::Data, &fb.normal),
            detail_b: tex(&["Detail Normal B", "Detail Normal"], Tex::Data, &fb.normal),
            rmas_a: tex(&["RMAS Map A"], Tex::Data, &fb.rmas),
            rmas_b: tex(&["RMAS Map B", "RSSSA Map"], Tex::Data, &fb.rmas),
            blend_map: tex(&["Blend Map"], Tex::Data, &fb.black),
            height: tex(&["Height"], Tex::Data, &fb.white),
        },
    }
}

fn terrain_material(
    info: &MaterialInfo,
    land: &LandscapeInfo,
    weight_dir: &Path,
    loader: &Loader,
    fb: &Fallback,
    images: &mut Assets<Image>,
) -> TerrainMaterial {
    let layers = &land.layers[..land.layers.len().min(TERRAIN_LAYERS)];
    if land.layers.len() > TERRAIN_LAYERS {
        warn!("{}: {} landscape layers, only the first {TERRAIN_LAYERS} are drawn", info.file, land.layers.len());
    }
    let [weights0, weights1] = weight_maps(weight_dir, layers).map(|w| images.add(w));
    let pick = |i: usize, f: fn(&LandscapeLayer) -> &Option<String>, kind: Tex, or: &Handle<Image>| {
        layers.get(i).and_then(|l| f(l).as_deref()).and_then(|p| loader.load(p, kind)).unwrap_or_else(|| or.clone())
    };
    let d = |i| pick(i, |l| &l.diffuse, Tex::Color, &fb.white);
    let n = |i| pick(i, |l| &l.normal, Tex::Data, &fb.normal);
    let r = |i| pick(i, |l| &l.rma, Tex::Data, &fb.rmas);
    let rma_mask = layers.iter().enumerate().filter(|(_, l)| l.rma.is_some()).fold(0u32, |m, (i, _)| m | 1 << i);
    ExtendedMaterial {
        base: StandardMaterial::default(),
        extension: TerrainExt {
            params: TerrainParams { mapping_scale: land.mapping_scale.max(0.001), layer_count: layers.len() as u32, rma_mask, _pad: 0 },
            weights0,
            weights1,
            d0: d(0),
            d1: d(1),
            d2: d(2),
            d3: d(3),
            d4: d(4),
            d5: d(5),
            n0: n(0),
            n1: n(1),
            n2: n(2),
            n3: n(3),
            n4: n(4),
            n5: n(5),
            r0: r(0),
            r1: r(1),
            r2: r(2),
            r3: r(3),
            r4: r(4),
            r5: r(5),
        },
    }
}

/// Packs the exported per-layer weight maps (`<landscape>/<layer>_LayerInfo.png`, greyscale) four to an RGBA image.
/// A layer without a weight map weighs 0 everywhere.
fn weight_maps(dir: &Path, layers: &[LandscapeLayer]) -> [Image; 2] {
    let decoded: Vec<Option<Image>> = layers
        .iter()
        .map(|l| {
            let path = dir.join(format!("{}_LayerInfo.png", l.name));
            let bytes = std::fs::read(&path).ok()?;
            Image::from_buffer(&bytes, ImageType::Extension("png"), CompressedImageFormats::NONE, false, ImageSampler::Default, RenderAssetUsages::MAIN_WORLD)
                .map_err(|e| warn!("{}: {e}", path.display()))
                .ok()
        })
        .collect();
    let size = decoded.iter().flatten().next().map_or(UVec2::ONE, |i| i.size());
    std::array::from_fn(|k| {
        let mut data = vec![0u8; (size.x * size.y * 4) as usize];
        for c in 0..4 {
            let Some(Some(img)) = decoded.get(k * 4 + c) else { continue };
            if img.size() != size {
                warn!("landscape weight map {} has a different size", layers[k * 4 + c].name);
                continue;
            }
            let Some(src) = img.data.as_ref() else { continue };
            // Greyscale PNGs decode to one byte per pixel; anything else, take the first channel.
            let stride = src.len() / (size.x * size.y) as usize;
            for (i, px) in src.chunks_exact(stride.max(1)).enumerate() {
                data[i * 4 + c] = px[0];
            }
        }
        let mut img = Image::new(
            Extent3d { width: size.x, height: size.y, depth_or_array_layers: 1 },
            TextureDimension::D2,
            data,
            TextureFormat::Rgba8Unorm,
            RenderAssetUsages::RENDER_WORLD,
        );
        img.sampler = ImageSampler::linear();
        img
    })
}

/// Marks meshes whose material reads vertex colours (painted blend masks).
#[derive(Component)]
struct KeepVertexColors;

/// Unreal vertex colours are material data (masks, wind weights), not colour; StandardMaterial would multiply the
/// base colour and alpha by them, turning many meshes black or invisible. Drop them, except where a ported shader
/// reads them.
fn strip_vertex_colors(
    added: Query<(&Mesh3d, Has<KeepVertexColors>), Added<Mesh3d>>,
    keep_added: Query<&Mesh3d, Added<KeepVertexColors>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut keep: Local<HashSet<AssetId<Mesh>>>,
    mut pending: Local<Vec<AssetId<Mesh>>>,
) {
    // Runs after apply_materials (chained, so its commands are applied): this frame's meshes are already marked.
    keep.extend(keep_added.iter().map(|m| m.0.id()));
    pending.extend(added.iter().filter(|(_, k)| !k).map(|(m, _)| m.0.id()));
    for id in std::mem::take(&mut *pending) {
        if keep.contains(&id) {
            continue;
        }
        let Some(mesh) = meshes.get(id) else {
            pending.push(id); // not loaded yet
            continue;
        };
        if mesh.contains_attribute(Mesh::ATTRIBUTE_COLOR) {
            if let Some(mut mesh) = meshes.get_mut(id) {
                mesh.remove_attribute(Mesh::ATTRIBUTE_COLOR);
            }
        }
    }
}
