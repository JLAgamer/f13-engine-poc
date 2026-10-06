//! Materials from the exporter's `materials.json`. Most become a StandardMaterial (diffuse + normal); two masters get
//! shaders ported from the game's compiled ones (see F13-decomp MODLOG, "Materials"):
//! - `Master_Blend_01` (cabin walls, roofs, floors...), `Master_Tint_Blend_01` and the curtains: texture sets blended
//!   by painted vertex alpha
//!   ([`BlendMaterial`], shaders/blend.wgsl);
//! - landscape materials (layers in `materials.json`): height-blended layers from the painted weight maps
//!   ([`TerrainMaterial`], shaders/terrain.wgsl);
//! - props with an RMA map (the `M_Prop_Master` family): roughness/metallic/AO from it ([`PropMaterial`],
//!   shaders/prop.wgsl);
//! - the lake (`M_Lake_Ripples_*`): a first-pass water surface ([`WaterMaterial`], shaders/water.wgsl).

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

const TERRAIN_LAYERS: usize = 6;
const WATER_MASTER: &str = "M_Lake_Ripples";

pub(crate) struct MaterialsPlugin {
    pub export: PathBuf,
}

impl Plugin for MaterialsPlugin {
    fn build(&self, app: &mut App) {
        embedded_asset!(app, "shaders/blend.wgsl");
        embedded_asset!(app, "shaders/terrain.wgsl");
        embedded_asset!(app, "shaders/water.wgsl");
        embedded_asset!(app, "shaders/prop.wgsl");
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
        let entries = entries.into_iter().map(|(k, v): (String, MaterialInfo)| (k.to_lowercase(), v)).collect();
        app.insert_resource(MaterialIndex { root: self.export.clone(), entries, ..default() })
            .add_plugins((
                MaterialPlugin::<BlendMaterial>::default(),
                MaterialPlugin::<TerrainMaterial>::default(),
                MaterialPlugin::<WaterMaterial>::default(),
                MaterialPlugin::<PropMaterial>::default(),
            ))
            .add_systems(Update, (apply_materials, strip_vertex_colors).chain());
    }
}

/// Material overrides for one placed mesh (slot material name, lowercase -> material used instead), on the entity
/// holding its scene.
#[derive(Component, Clone)]
pub(crate) struct MaterialOverrides(std::sync::Arc<HashMap<String, String>>);

impl MaterialOverrides {
    pub(crate) fn new(map: &HashMap<String, String>) -> Option<Self> {
        (!map.is_empty()).then(|| Self(std::sync::Arc::new(map.iter().map(|(k, v)| (k.to_lowercase(), v.clone())).collect())))
    }
}

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
    fallback: Option<Fallback>,
}

/// 1x1 stand-ins for textures a material names but the export lacks.
#[derive(Clone)]
struct Fallback {
    white: Handle<Image>,
    black: Handle<Image>,
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
        Self { white: px([255; 4]), black: px([0, 0, 0, 255]), normal: px([128, 128, 255, 255]), rmas: px([255, 0, 255, 128]) }
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
    ripples: Handle<Image>,
}

#[derive(ShaderType, Reflect, Debug, Clone)]
struct WaterParams {
    color: Vec4,
    waves: Vec4,
    surface: Vec4,
}

impl MaterialExtension for WaterExt {
    fn fragment_shader() -> ShaderRef {
        "embedded://f13_game/shaders/water.wgsl".into()
    }
}

fn water_material(info: &MaterialInfo, loader: &Loader, fb: &Fallback) -> WaterMaterial {
    // The far ripple normal map is the one that reads as water from walking height.
    let ripples = ["WaveTextureFar01", "normal"]
        .iter()
        .find_map(|k| info.textures.get(*k).and_then(|p| loader.load(p, Tex::Data)))
        .unwrap_or_else(|| fb.normal.clone());
    let color = info.vectors.get("Ocean Water Color").or(info.vectors.get("Color (Reg)")).map_or(Vec4::new(0.02, 0.02, 0.02, 1.0), |v| Vec4::from_array(*v));
    ExtendedMaterial {
        base: StandardMaterial::default(),
        extension: WaterExt {
            params: WaterParams {
                color,
                waves: Vec4::new(
                    info.scalar("Small Wave Scale", 300.0).max(1.0),
                    info.scalar("LArge Wave Scale", 1500.0).max(1.0),
                    info.scalar("SmallWaveTime", 0.3),
                    info.scalar("LargeWaveTime", 0.03),
                ),
                surface: Vec4::new(info.scalar("rough", 0.1), info.scalar("specular", 0.02).max(0.02), info.scalar("NormalPowerNear", 0.6), 0.0),
            },
            ripples,
        },
    }
}

#[derive(Asset, AsBindGroup, Reflect, Debug, Clone)]
pub(crate) struct PropExt {
    #[uniform(100)]
    params: PropParams,
    #[texture(101)]
    #[sampler(102)]
    rma: Handle<Image>,
}

#[derive(ShaderType, Reflect, Debug, Clone)]
struct PropParams {
    mults: Vec4,
    extra: Vec4,
}

/// The RMA/RSS map and parameters for prop and foliage masters (channel layouts read from M_Prop_Master's and
/// M_Leaves_Master's compiled shaders), or None for other materials.
fn prop_params(info: &MaterialInfo, loader: &Loader) -> Option<(Handle<Image>, PropParams)> {
    // Same RMA sampling (R roughness, G metallic, B AO, times the mults) in M_Item_Master and the Chevette's exterior.
    if info.master.starts_with("M_Prop_Master") || info.master == "M_Item_Master" || info.master == "M_Vehicle_Chevette_Exterior" {
        let rma = loader.load(info.textures.get("RMA")?, Tex::Data)?;
        let mults = Vec4::new(info.scalar("RoughnessMult", 1.0), info.scalar("MetalnessMult", 1.0), info.scalar("Spec", 0.5), 0.0);
        return Some((rma, PropParams { mults, extra: Vec4::ZERO }));
    }
    let rss = ["RSS", "RSS Map"].iter().find_map(|k| info.textures.get(*k))?;
    let rss = loader.load(rss, Tex::Data)?;
    let mults = Vec4::new(info.scalar("RoughnessMultiplier", 1.0), 0.0, info.scalar("Specular", 0.5), 1.0);
    Some((rss, PropParams { mults, extra: Vec4::new(info.scalar("RoughnessMin", 0.0), 0.0, 0.0, 0.0) }))
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
    /// Loads an exported texture (export-relative path without extension), or None if the export lacks it.
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
        Some(self.assets.load_builder().with_settings(move |s: &mut ImageLoaderSettings| {
            s.is_srgb = srgb;
            s.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor {
                address_mode_u: bevy::image::ImageAddressMode::Repeat,
                address_mode_v: bevy::image::ImageAddressMode::Repeat,
                anisotropy_clamp: 8,
                ..ImageSamplerDescriptor::linear()
            });
        }).load(file))
    }
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
    mut index: ResMut<MaterialIndex>,
    (mut standard, mut blends, mut terrains, mut waters, mut images): (
        ResMut<Assets<StandardMaterial>>,
        ResMut<Assets<BlendMaterial>>,
        ResMut<Assets<TerrainMaterial>>,
        ResMut<Assets<WaterMaterial>>,
        ResMut<Assets<Image>>,
    ),
    mut props: ResMut<Assets<PropMaterial>>,
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
                } else if info.master.starts_with(WATER_MASTER) {
                    Built::Water(waters.add(water_material(info, &loader, &fallback)))
                } else if let Some(kind) = blend_kind(&info.master) {
                    // (The POM variant's parallax is not ported: drawn flat.)
                    Built::Blend(blends.add(blend_material(info, kind, &loader, &fallback)))
                } else if let Some((rma, params)) = prop_params(info, &loader) {
                    Built::Prop(props.add(ExtendedMaterial { base: standard_material(&index.root, info, &loader), extension: PropExt { params, rma } }))
                } else {
                    Built::Standard(standard.add(standard_material(&index.root, info, &loader)))
                };
                index.built.insert(key, b.clone());
                b
            }
        };
        let mut e = commands.entity(entity);
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
    m.base_color_texture = tex(&["Diffuse", "BaseColor", "Diffuse/Opacity", "PM_Diffuse"]).and_then(|p| loader.load(&p, Tex::Color));
    if let Some(n) = tex(&["Normal", "normalMap", "PM_Normals"]).and_then(|p| loader.load(&p, Tex::Data)) {
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
