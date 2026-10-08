//! The game's tonemapper in place of Bevy's (MODLOG "Post-processing"): the map's unbound PostProcessVolume baked into
//! the engine's 32³ colour LUT on the CPU (f13-core::color_grading, the game's CombineLUTs shader read from its
//! GlobalShaderCache), applied by a port of its tonemap pass: FPostProcessTonemapPS4, the cheapest permutation in this
//! build's TonemapperConfBitmaskPC (0x142d55530) with what the volume needs (bloom, vignette at its 0.4 default, grain
//! quantization; no grain, colour fringe or sharpening at their defaults).
//!
//! Per pixel (shaders/tonemap.wgsl): scene colour x ColorScale0 (SceneColorTint) + bloom (bloom.rs) x ColorScale1
//! (BloomIntensity); exposure (Bevy's camera exposure, 1 / 1.2 as the volume's fixed auto exposure gives); the
//! vignette (1 / (1 + |v|²)², v the screen position scaled to a circle x VignetteIntensity); the log encoding into the
//! LUT; x 1.05; the 1/256 grain quantization dither. The result is display-encoded sRGB, decoded here for Bevy's sRGB
//! output.
//!
//! Stalk's colour grade (the played Jason's): OnRep_Stalk (0x1403fd970) sets his ForcedPostProcessVolume's
//! ColorGradingLUT to StalkColorGrade (CG_Stalk_B) and his tick (FUN_14040d900) eases its ColorGradingIntensity
//! (FInterpTo at speed 1) to 1 while he stalks and back to 0, the volume's LUT overrides off while he's in a context
//! kill. Blending a volume's LUT at weight i scales the others by 1 - i; after the weighted sum CombineLUTs is the
//! identity here (sRGB decode then encode, the identity polynomial, gamma 1), so the pass blends this LUT's table
//! (the Stalk LUT alone) with the map's by i.

use bevy::asset::{embedded_asset, load_embedded_asset, RenderAssetUsages};
use bevy::core_pipeline::schedule::{Core3d, Core3dSystems};
use bevy::core_pipeline::tonemapping::{tonemapping, DebandDither, Tonemapping};
use bevy::core_pipeline::FullscreenShader;
use bevy::image::{CompressedImageFormats, ImageSampler, ImageType};
use bevy::prelude::*;
use bevy::render::extract_component::{ComponentUniforms, DynamicUniformIndex, ExtractComponent, ExtractComponentPlugin, UniformComponentPlugin};
use bevy::render::render_asset::RenderAssets;
use bevy::render::render_resource::binding_types::{sampler, texture_2d, texture_3d, uniform_buffer};
use bevy::render::render_resource::*;
use bevy::render::renderer::{RenderContext, RenderDevice, ViewQuery};
use bevy::render::sync_component::SyncComponent;
use bevy::render::texture::GpuImage;
use bevy::render::view::{ExtractedView, ViewTarget};
use bevy::render::{GpuResourceAppExt, Render, RenderApp, RenderStartup, RenderSystems};
use bevy::shader::Shader;
use f13_core::color_grading::{CombineLuts, Correction, LUT_SIZE};
use serde_json::Value;

pub struct TonemapPlugin;

impl Plugin for TonemapPlugin {
    fn build(&self, app: &mut App) {
        embedded_asset!(app, "shaders/tonemap.wgsl");
        app.add_plugins((ExtractComponentPlugin::<GameTonemap>::default(), UniformComponentPlugin::<TonemapUniform>::default()))
            .add_systems(Update, (setup, stalk_grade));
        let Some(render_app) = app.get_sub_app_mut(RenderApp) else { return };
        render_app
            .init_gpu_resource::<SpecializedRenderPipelines<TonemapPipeline>>()
            .add_systems(RenderStartup, init_pipeline)
            .add_systems(Render, prepare_pipelines.in_set(RenderSystems::Prepare))
            .add_systems(Core3d, game_tonemap.after(tonemapping).in_set(Core3dSystems::PostProcess));
    }
}

/// The game's tonemapper on a camera: its combined LUT and the tonemap pass's settings.
#[derive(Component, Clone)]
pub struct GameTonemap {
    pub lut: Handle<Image>,
    /// The same baked with the Stalk colour grade alone, and its weight now.
    pub stalk_lut: Handle<Image>,
    pub stalk: f32,
    /// SceneColorTint.
    pub color_scale0: Vec3,
    pub vignette_intensity: f32,
    pub bloom: crate::bloom::BloomSettings,
}

#[derive(Component, ShaderType, Clone)]
pub struct TonemapUniform {
    /// ColorScale0 (rgb).
    color_scale0: Vec4,
    /// VignetteIntensity, the view's height / width.
    vignette: Vec4,
    /// GrainRandomFull (xy): this frame's offset of the dither's seed.
    grain: Vec4,
    /// ColorScale1: BloomIntensity (white x it).
    color_scale1: Vec4,
    /// x: the Stalk LUT's weight.
    stalk: Vec4,
}

impl SyncComponent for GameTonemap {
    type Target = (GameTonemap, TonemapUniform);
}

impl ExtractComponent for GameTonemap {
    type QueryData = (&'static GameTonemap, &'static Camera);
    type QueryFilter = ();
    type Out = (GameTonemap, TonemapUniform);

    fn extract_component((t, camera): bevy::ecs::query::QueryItem<'_, '_, Self::QueryData>) -> Option<Self::Out> {
        let size = camera.physical_viewport_size()?;
        let aspect = size.y as f32 / size.x.max(1) as f32;
        // GrainRandomFromFrame (the tonemapper's SetPS 0x140c90190, decompiled): Halton (2, 3) of the view state's
        // FrameIndexMod8 (vfunc 0x78, a byte) & 1023.
        let frame = (FRAME.fetch_add(1, std::sync::atomic::Ordering::Relaxed) % 8) & 1023;
        let grain = Vec4::new(halton(frame, 2), halton(frame, 3), 0.0, 0.0);
        let color_scale1 = Vec4::new(t.bloom.intensity, t.bloom.intensity, t.bloom.intensity, 0.0);
        let stalk = Vec4::new(t.stalk, 0.0, 0.0, 0.0);
        Some((t.clone(), TonemapUniform { color_scale0: t.color_scale0.extend(0.0), vignette: Vec4::new(t.vignette_intensity, aspect, 0.0, 0.0), grain, color_scale1, stalk }))
    }
}

static FRAME: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

fn halton(mut i: u32, base: u32) -> f32 {
    let (mut f, mut r) = (1.0f32, 0.0f32);
    while i > 0 {
        f /= base as f32;
        r += f * (i % base) as f32;
        i /= base;
    }
    r
}

/// The level's unbound PostProcessVolume's settings: its overridden values, else FPostProcessSettings' defaults
/// (its constructor 0x141665980).
struct Volume<'a>(&'a serde_json::Map<String, Value>);

impl Volume<'_> {
    fn set(&self, key: &str) -> Option<&Value> {
        (self.0.get(&format!("bOverride_{key}")).and_then(Value::as_bool) == Some(true)).then(|| self.0.get(key)).flatten()
    }

    fn f(&self, key: &str, default: f32) -> f32 {
        self.set(key).map_or(default, |v| v.as_f64().map_or(default, |v| v as f32))
    }

    /// A Vector4 (X, Y, Z, W).
    fn v4(&self, key: &str, default: [f32; 4]) -> [f32; 4] {
        let mut out = default;
        if let Some(v) = self.set(key) {
            for (i, k) in ["X", "Y", "Z", "W"].iter().enumerate() {
                if let Some(x) = v.get(*k).and_then(Value::as_f64) {
                    out[i] = x as f32;
                }
            }
        }
        out
    }

    /// A LinearColor (R, G, B), white by default.
    fn color(&self, key: &str) -> Vec3 {
        let Some(v) = self.set(key) else { return Vec3::ONE };
        let c = |k: &str| v.get(k).and_then(Value::as_f64).unwrap_or(1.0) as f32;
        Vec3::new(c("R"), c("G"), c("B"))
    }

    fn bloom(&self) -> crate::bloom::BloomSettings {
        let d = crate::bloom::BloomSettings::default();
        let tint = |i: usize| {
            let key = format!("Bloom{}Tint", i + 1);
            match self.set(&key) {
                Some(v) => {
                    let c = |k: &str, d: f32| v.get(k).and_then(Value::as_f64).map_or(d, |x| x as f32);
                    Vec4::new(c("R", d.tints[i].x), c("G", d.tints[i].y), c("B", d.tints[i].z), c("A", d.tints[i].w))
                }
                None => d.tints[i],
            }
        };
        crate::bloom::BloomSettings {
            intensity: self.f("BloomIntensity", d.intensity),
            threshold: self.f("BloomThreshold", d.threshold),
            size_scale: self.f("BloomSizeScale", d.size_scale),
            sizes: std::array::from_fn(|i| self.f(&format!("Bloom{}Size", i + 1), d.sizes[i])),
            tints: std::array::from_fn(tint),
        }
    }

    fn correction(&self, range: &str) -> Correction {
        let c = Correction::default();
        let k = |what: &str| format!("Color{what}{range}");
        Correction {
            saturation: self.v4(&k("Saturation"), c.saturation),
            contrast: self.v4(&k("Contrast"), c.contrast),
            gamma: self.v4(&k("Gamma"), c.gamma),
            gain: self.v4(&k("Gain"), c.gain),
            offset: self.v4(&k("Offset"), c.offset),
        }
    }

    fn combine_luts(&self, has_lut: bool) -> CombineLuts {
        let d = CombineLuts::default();
        let intensity = if has_lut { self.f("ColorGradingIntensity", 1.0) } else { 0.0 };
        CombineLuts {
            white_temp: self.f("WhiteTemp", d.white_temp),
            white_tint: self.f("WhiteTint", d.white_tint),
            global: self.correction(""),
            shadows: self.correction("Shadows"),
            midtones: self.correction("Midtones"),
            highlights: self.correction("Highlights"),
            shadows_max: self.f("ColorCorrectionShadowsMax", d.shadows_max),
            highlights_min: self.f("ColorCorrectionHighlightsMin", d.highlights_min),
            film_slope: self.f("FilmSlope", d.film_slope),
            film_toe: self.f("FilmToe", d.film_toe),
            film_shoulder: self.f("FilmShoulder", d.film_shoulder),
            film_black_clip: self.f("FilmBlackClip", d.film_black_clip),
            film_white_clip: self.f("FilmWhiteClip", d.film_white_clip),
            weights: [1.0 - intensity, intensity],
            ..d
        }
    }
}

/// Bakes the volume's LUT once the level is in, and puts the game's tonemapper on the 3D cameras.
fn setup(
    mut commands: Commands,
    level: Option<Res<crate::Level>>,
    export: Res<crate::ExportRoot>,
    mut images: ResMut<Assets<Image>>,
    mut built: Local<Option<GameTonemap>>,
    cameras: Query<Entity, (With<Camera3d>, Without<GameTonemap>)>,
) {
    if built.is_none() {
        let Some(level) = level else { return };
        let empty = serde_json::Map::new();
        let volume = level
            .layouts
            .iter()
            .flat_map(|l| &l.actors)
            .find(|a| a.class == "PostProcessVolume" && a.props["bUnbound"].as_bool() == Some(true));
        let settings = volume.and_then(|v| v.props["Settings"].as_object()).unwrap_or(&empty);
        let v = Volume(settings);
        // The colour grading LUT (a 256 x 16 strip; not sRGB: its bytes are the values), sampled bilinear, clamped.
        let lut = volume.and_then(|a| a.props["ColorGradingLUTFile"].as_str()).filter(|_| v.set("ColorGradingLUT").is_some()).and_then(|f| {
            let bytes = std::fs::read(export.0.join(format!("{f}.png"))).ok()?;
            let img = Image::from_buffer(&bytes, ImageType::Extension("png"), CompressedImageFormats::NONE, false, ImageSampler::Default, RenderAssetUsages::default()).ok()?;
            let size = img.size();
            let data = img.data?;
            Some((size.x as usize, size.y as usize, data))
        });
        if volume.is_some() && v.set("ColorGradingLUT").is_some() && lut.is_none() {
            warn!("tonemap: the volume's colour grading LUT isn't in the export (re-run f13-export levels)");
        }
        let params = v.combine_luts(lut.is_some());
        let sample = |u: f32, t: f32| -> Vec3 {
            let Some((w, h, data)) = &lut else { return Vec3::ZERO };
            let texel = |x: i32, y: i32| {
                let (x, y) = (x.clamp(0, *w as i32 - 1) as usize, y.clamp(0, *h as i32 - 1) as usize);
                let i = (y * w + x) * 4;
                Vec3::new(data[i] as f32, data[i + 1] as f32, data[i + 2] as f32) / 255.0
            };
            let (fx, fy) = (u * *w as f32 - 0.5, t * *h as f32 - 0.5);
            let (x0, y0) = (fx.floor(), fy.floor());
            let (ax, ay) = (fx - x0, fy - y0);
            let (x0, y0) = (x0 as i32, y0 as i32);
            let top = texel(x0, y0).lerp(texel(x0 + 1, y0), ax);
            let bottom = texel(x0, y0 + 1).lerp(texel(x0 + 1, y0 + 1), ax);
            top.lerp(bottom, ay)
        };
        let started = std::time::Instant::now();
        let image = bake(&f13_core::color_grading::combine_luts(&params, &sample));
        // The Stalk grade alone (Jason_Base's StalkColorGrade), the volume's other settings as they are.
        let stalk = read_strip(&export.0.join(STALK_LUT));
        if stalk.is_none() {
            warn!("tonemap: {STALK_LUT} isn't in the export (f13-export mvp): no Stalk colour grade");
        }
        let stalk_sample = |u: f32, t: f32| stalk.as_ref().map_or(Vec3::ZERO, |s| s.sample(u, t));
        let stalk_params = CombineLuts { weights: [0.0, 1.0], ..params };
        let stalk_image = if stalk.is_some() { bake(&f13_core::color_grading::combine_luts(&stalk_params, &stalk_sample)) } else { image.clone() };
        info!(
            "tonemap: the volume's LUT (grading {:.2}, white {:.0} K) in {:.0} ms",
            params.weights[1],
            params.white_temp,
            started.elapsed().as_secs_f32() * 1000.0
        );
        *built = Some(GameTonemap {
            lut: images.add(image),
            stalk_lut: images.add(stalk_image),
            stalk: 0.0,
            color_scale0: v.color("SceneColorTint"),
            vignette_intensity: v.f("VignetteIntensity", 0.4),
            bloom: v.bloom(),
        });
    }
    let Some(t) = built.as_ref() else { return };
    for e in &cameras {
        commands.entity(e).insert((t.clone(), Tonemapping::None, DebandDither::Disabled));
    }
}

/// Jason_Base's StalkColorGrade.
const STALK_LUT: &str = "SummerCamp/Content/Effects/ColorGrades/CG_Stalk_B.png";

/// A colour grading LUT texture (its 256 x 16 strip; not sRGB: its bytes are the values), sampled bilinear, clamped.
struct Strip {
    w: usize,
    h: usize,
    data: Vec<u8>,
}

impl Strip {
    fn sample(&self, u: f32, t: f32) -> Vec3 {
        let texel = |x: i32, y: i32| {
            let (x, y) = (x.clamp(0, self.w as i32 - 1) as usize, y.clamp(0, self.h as i32 - 1) as usize);
            let i = (y * self.w + x) * 4;
            Vec3::new(self.data[i] as f32, self.data[i + 1] as f32, self.data[i + 2] as f32) / 255.0
        };
        let (fx, fy) = (u * self.w as f32 - 0.5, t * self.h as f32 - 0.5);
        let (x0, y0) = (fx.floor(), fy.floor());
        let (ax, ay) = (fx - x0, fy - y0);
        let (x0, y0) = (x0 as i32, y0 as i32);
        let top = texel(x0, y0).lerp(texel(x0 + 1, y0), ax);
        let bottom = texel(x0, y0 + 1).lerp(texel(x0 + 1, y0 + 1), ax);
        top.lerp(bottom, ay)
    }
}

fn read_strip(path: &std::path::Path) -> Option<Strip> {
    let bytes = std::fs::read(path).ok()?;
    let img = Image::from_buffer(&bytes, ImageType::Extension("png"), CompressedImageFormats::NONE, false, ImageSampler::Default, RenderAssetUsages::default()).ok()?;
    let size = img.size();
    Some(Strip { w: size.x as usize, h: size.y as usize, data: img.data? })
}

/// The engine renders the SDR LUT to PF_A2B10G10R10: 10 bits a channel, round to nearest.
fn bake(table: &[Vec3]) -> Image {
    let mut bytes = Vec::with_capacity(table.len() * 4);
    for c in table {
        let q = |x: f32| (x.clamp(0.0, 1.0) * 1023.0).round() as u32;
        bytes.extend_from_slice(&(q(c.x) | q(c.y) << 10 | q(c.z) << 20 | 3 << 30).to_le_bytes());
    }
    let n = LUT_SIZE as u32;
    let mut image = Image::new(Extent3d { width: n, height: n, depth_or_array_layers: n }, TextureDimension::D3, bytes, TextureFormat::Rgb10a2Unorm, RenderAssetUsages::RENDER_WORLD);
    image.sampler = ImageSampler::linear();
    image
}

/// UKismetMathLibrary::FInterpTo.
fn finterp_to(current: f32, target: f32, dt: f32, speed: f32) -> f32 {
    if speed <= 0.0 {
        return target;
    }
    let dist = target - current;
    if dist * dist < 1e-8 {
        return target;
    }
    current + dist * (dt * speed).clamp(0.0, 1.0)
}

/// Stalk's colour grade on the played Jason's camera (see the module docs).
fn stalk_grade(
    time: Res<Time>,
    play: Option<Res<crate::jason_player::PlayJason>>,
    jasons: Query<&crate::jason::Jason>,
    mut cameras: Query<&mut GameTonemap>,
    mut intensity: Local<f32>,
) {
    let Some(j) = jasons.iter().next().filter(|_| play.is_some()) else { return };
    let target = if j.stalking() { 1.0 } else { 0.0 };
    *intensity = finterp_to(*intensity, target, time.delta_secs(), 1.0);
    let weight = if j.in_context_kill() { 0.0 } else { *intensity };
    for mut t in &mut cameras {
        if t.stalk != weight {
            t.stalk = weight;
        }
    }
}

#[derive(Resource)]
pub(crate) struct TonemapPipeline {
    layout: BindGroupLayoutDescriptor,
    sampler: Sampler,
    lut_sampler: Sampler,
    fullscreen_shader: FullscreenShader,
    shader: Handle<Shader>,
}

fn init_pipeline(mut commands: Commands, device: Res<RenderDevice>, fullscreen_shader: Res<FullscreenShader>, asset_server: Res<AssetServer>) {
    let layout = BindGroupLayoutDescriptor::new(
        "game_tonemap_layout",
        &BindGroupLayoutEntries::sequential(
            ShaderStages::FRAGMENT,
            (
                texture_2d(TextureSampleType::Float { filterable: true }),
                sampler(SamplerBindingType::Filtering),
                texture_3d(TextureSampleType::Float { filterable: true }),
                sampler(SamplerBindingType::Filtering),
                uniform_buffer::<TonemapUniform>(true),
                texture_2d(TextureSampleType::Float { filterable: true }),
                texture_3d(TextureSampleType::Float { filterable: true }),
            ),
        ),
    );
    let sampler = device.create_sampler(&SamplerDescriptor { mag_filter: FilterMode::Linear, min_filter: FilterMode::Linear, ..default() });
    // The LUT: trilinear, clamped.
    let lut_sampler = device.create_sampler(&SamplerDescriptor {
        address_mode_u: AddressMode::ClampToEdge,
        address_mode_v: AddressMode::ClampToEdge,
        address_mode_w: AddressMode::ClampToEdge,
        mag_filter: FilterMode::Linear,
        min_filter: FilterMode::Linear,
        ..default()
    });
    commands.insert_resource(TonemapPipeline {
        layout,
        sampler,
        lut_sampler,
        fullscreen_shader: fullscreen_shader.clone(),
        shader: load_embedded_asset!(asset_server.as_ref(), "shaders/tonemap.wgsl"),
    });
}

#[derive(Component)]
pub(crate) struct TonemapPipelineId(CachedRenderPipelineId);

impl SpecializedRenderPipeline for TonemapPipeline {
    type Key = TextureFormat;

    fn specialize(&self, format: TextureFormat) -> RenderPipelineDescriptor {
        RenderPipelineDescriptor {
            label: Some("game_tonemap".into()),
            layout: vec![self.layout.clone()],
            vertex: self.fullscreen_shader.to_vertex_state(),
            fragment: Some(FragmentState {
                shader: self.shader.clone(),
                targets: vec![Some(ColorTargetState { format, blend: None, write_mask: ColorWrites::ALL })],
                ..default()
            }),
            ..default()
        }
    }
}

fn prepare_pipelines(
    mut commands: Commands,
    cache: Res<PipelineCache>,
    mut pipelines: ResMut<SpecializedRenderPipelines<TonemapPipeline>>,
    pipeline: Res<TonemapPipeline>,
    views: Query<(Entity, &ExtractedView), With<GameTonemap>>,
) {
    for (e, view) in &views {
        let id = pipelines.specialize(&cache, &pipeline, view.target_format);
        commands.entity(e).insert(TonemapPipelineId(id));
    }
}

pub(crate) fn game_tonemap(
    view: ViewQuery<(&ViewTarget, &GameTonemap, &TonemapPipelineId, &DynamicUniformIndex<TonemapUniform>, &crate::bloom::BloomTextures)>,
    pipeline: Res<TonemapPipeline>,
    cache: Res<PipelineCache>,
    uniforms: Res<ComponentUniforms<TonemapUniform>>,
    images: Res<RenderAssets<GpuImage>>,
    mut ctx: RenderContext,
) {
    let (target, settings, id, index, bloom) = view.into_inner();
    let (Some(render_pipeline), Some(lut), Some(stalk_lut), Some(binding)) = (cache.get_render_pipeline(id.0), images.get(&settings.lut), images.get(&settings.stalk_lut), uniforms.binding()) else { return };
    let post = target.post_process_write();
    let bind_group = ctx.render_device().create_bind_group(
        None,
        &cache.get_bind_group_layout(&pipeline.layout),
        &BindGroupEntries::sequential((post.source, &pipeline.sampler, &lut.texture_view, &pipeline.lut_sampler, binding.clone(), bloom.result(), &stalk_lut.texture_view)),
    );
    let mut pass = ctx.command_encoder().begin_render_pass(&RenderPassDescriptor {
        label: Some("game_tonemap"),
        color_attachments: &[Some(RenderPassColorAttachment { view: post.destination, depth_slice: None, resolve_target: None, ops: Operations::default() })],
        depth_stencil_attachment: None,
        timestamp_writes: None,
        occlusion_query_set: None,
        multiview_mask: None,
    });
    pass.set_pipeline(render_pipeline);
    pass.set_bind_group(0, &bind_group, &[index.index()]);
    pass.draw(0..3, 0..1);
}
