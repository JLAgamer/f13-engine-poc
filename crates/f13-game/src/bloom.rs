//! The game's bloom (MODLOG "Post-processing"), as this build's AddBloom (FUN_140c4ac10) builds it at the Epic
//! scalability level (r.BloomQuality 5: six stages; r.Filter.SizeScale 1):
//!
//! - SceneColorHalfRes: the scene colour downsampled to half size (FPostProcessDownsamplePS1, r.Downsample.Quality's 3
//!   clamped to 1: four bilinear taps one texel out, averaged, negatives clamped);
//! - BloomSetup (FPostProcessBloomSetupPS): colour x saturate((luminance (0.3, 0.59, 0.11) x exposure - BloomThreshold)
//!   x 0.5) (the scene colour here is already exposed);
//! - BloomDownsample1..5: halving again each time (the same downsample);
//! - from the smallest: each stage's Gaussian (FRCPassPostProcessWeightedSampleSum: X, then Y with the stage's tint / 6
//!   and the previous stage's result added; TFilterPS's weighted sum) of BloomNSize x BloomSizeScale, its radius
//!   size x (view width / the source's scale) x 0.005 texels, its kernel Compute1DGaussianFilterKernel's
//!   (FUN_140d18b40: bilinear pairs, exp(-16.7 (i / r)²) with r.Filter.NewMethod 1, at most 32 samples on SM5,
//!   normalised).
//!
//! The tonemapper adds the result x BloomIntensity (its ColorScale1; the dirt mask's intensity is 0).
//!
//! The fast blur (FRCPassPostProcessWeightedSampleSum's DoFastBlur FUN_140d19fc0 and ComputeOutputDesc
//! FUN_140d19260): an X pass whose 0.02 x source width x size reaches r.FastBlurThreshold (100 at Epic) writes half
//! the width; its Y pass, seeing a source narrower than 0.75 x the buffer's aspect, writes twice the width, its radius
//! doubled (the large stages at the usual resolutions). The textures are R11G11B10 floats, as the engine's
//! PF_FloatRGB.

use bevy::asset::{embedded_asset, load_embedded_asset};
use bevy::core_pipeline::schedule::{Core3d, Core3dSystems};
use bevy::core_pipeline::FullscreenShader;
use bevy::prelude::*;
use bevy::render::camera::ExtractedCamera;
use bevy::render::render_resource::binding_types::{sampler, texture_2d, uniform_buffer_sized};
use bevy::render::render_resource::*;
use bevy::render::renderer::{RenderContext, RenderDevice, ViewQuery};
use bevy::render::texture::{CachedTexture, TextureCache};
use bevy::render::view::ViewTarget;
use bevy::render::{GpuResourceAppExt, Render, RenderApp, RenderStartup, RenderSystems};
use bevy::shader::Shader;

pub const FORMAT: TextureFormat = TextureFormat::Rg11b10Ufloat;
/// The most samples a weighted sample sum takes on SM5 (r.Filter.LoopMode 0).
const MAX_SAMPLES: usize = 32;
/// r.FastBlurThreshold at the Epic scalability level.
const FAST_BLUR_THRESHOLD: f32 = 100.0;

/// The bloom's settings (FPostProcessSettings', the volume's where it overrides them).
#[derive(Clone, Debug, PartialEq)]
pub struct BloomSettings {
    pub intensity: f32,
    pub threshold: f32,
    pub size_scale: f32,
    /// Bloom1Size..Bloom6Size.
    pub sizes: [f32; 6],
    /// Bloom1Tint..Bloom6Tint.
    pub tints: [Vec4; 6],
}

impl Default for BloomSettings {
    /// FPostProcessSettings' constructor (0x141665980).
    fn default() -> Self {
        let t = |v: f32| Vec4::new(v, v, v, 1.0);
        BloomSettings {
            intensity: 0.675,
            threshold: -1.0,
            size_scale: 4.0,
            sizes: [0.3, 1.0, 2.0, 10.0, 30.0, 64.0],
            tints: [t(0.3465), t(0.138), t(0.1176), t(0.066), t(0.066), t(0.061)],
        }
    }
}

/// Compute1DGaussianFilterKernel (FUN_140d18b40): (offset in texels, weight) pairs for a radius.
pub fn gaussian_kernel(radius: f32) -> Vec<(f32, f32)> {
    let size_scale = 1.0f32.clamp(0.1, 10.0);
    let clamped = if radius >= 9.999999747378752e-06 { radius.min((MAX_SAMPLES - 1) as f32) } else { 9.999999747378752e-06 };
    let r = (size_scale * radius).max(9.999999747378752e-06).min((MAX_SAMPLES - 1) as f32);
    let integer = (r.ceil() as i32).min(MAX_SAMPLES as i32 - 1);
    let scale = 1.0 / clamped;
    // NormalDistributionUnscaled with r.Filter.NewMethod 1 and r.Bloom.Cross 0.
    let w = |x: f32| (x.abs() * scale * x.abs() * scale * -16.700000762939453).exp();
    let mut out = Vec::new();
    let mut sum = 0.0;
    let mut i = -integer;
    while i <= integer {
        let w0 = w(i as f32);
        let w1 = if i != integer { w((i + 1) as f32) } else { 0.0 };
        let total = w0 + w1;
        out.push((i as f32 + w1 / total, total));
        sum += total;
        i += 2;
    }
    for p in &mut out {
        p.1 /= sum;
    }
    out
}

pub struct BloomPlugin;

impl Plugin for BloomPlugin {
    fn build(&self, app: &mut App) {
        embedded_asset!(app, "shaders/bloom.wgsl");
        let Some(render_app) = app.get_sub_app_mut(RenderApp) else { return };
        render_app
            .init_gpu_resource::<SpecializedRenderPipelines<BloomPipelines>>()
            .add_systems(RenderStartup, init_pipelines)
            .add_systems(Render, prepare_textures.in_set(RenderSystems::PrepareResources))
            .add_systems(Core3d, game_bloom.before(crate::tonemap::game_tonemap).in_set(Core3dSystems::PostProcess));
    }
}

/// The passes' shader entry points.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub(crate) enum Pass {
    Downsample,
    Setup,
    Filter,
}

#[derive(Resource)]
pub(crate) struct BloomPipelines {
    layout: BindGroupLayoutDescriptor,
    sampler: Sampler,
    fullscreen_shader: FullscreenShader,
    shader: Handle<Shader>,
}

impl SpecializedRenderPipeline for BloomPipelines {
    type Key = Pass;

    fn specialize(&self, pass: Pass) -> RenderPipelineDescriptor {
        let entry = match pass {
            Pass::Downsample => "downsample",
            Pass::Setup => "setup",
            Pass::Filter => "weighted_sum",
        };
        RenderPipelineDescriptor {
            label: Some(format!("game_bloom_{entry}").into()),
            layout: vec![self.layout.clone()],
            vertex: self.fullscreen_shader.to_vertex_state(),
            fragment: Some(FragmentState {
                shader: self.shader.clone(),
                entry_point: Some(entry.into()),
                targets: vec![Some(ColorTargetState { format: FORMAT, blend: None, write_mask: ColorWrites::ALL })],
                ..default()
            }),
            ..default()
        }
    }
}

/// One pass's uniforms: the source's texel size, the sample count, flags, then the samples' UV offsets and weights.
#[derive(Clone, Copy, ShaderType)]
struct PassUniform {
    texel: Vec4,
    /// x: samples, y: 1 to add the second texture, z: BloomThreshold.
    params: Vec4,
    offsets: [Vec4; MAX_SAMPLES],
    weights: [Vec4; MAX_SAMPLES],
}

fn init_pipelines(mut commands: Commands, device: Res<RenderDevice>, fullscreen_shader: Res<FullscreenShader>, asset_server: Res<AssetServer>) {
    let layout = BindGroupLayoutDescriptor::new(
        "game_bloom_layout",
        &BindGroupLayoutEntries::sequential(
            ShaderStages::FRAGMENT,
            (
                texture_2d(TextureSampleType::Float { filterable: true }),
                texture_2d(TextureSampleType::Float { filterable: true }),
                sampler(SamplerBindingType::Filtering),
                uniform_buffer_sized(false, Some(PassUniform::min_size())),
            ),
        ),
    );
    let sampler = device.create_sampler(&SamplerDescriptor {
        address_mode_u: AddressMode::ClampToEdge,
        address_mode_v: AddressMode::ClampToEdge,
        mag_filter: FilterMode::Linear,
        min_filter: FilterMode::Linear,
        ..default()
    });
    commands.insert_resource(BloomPipelines { layout, sampler, fullscreen_shader: fullscreen_shader.clone(), shader: load_embedded_asset!(asset_server.as_ref(), "shaders/bloom.wgsl") });
}

/// A view's bloom textures: the half-size scene colour, the six levels, and each stage's X and Y results.
#[derive(Component)]
pub struct BloomTextures {
    half: CachedTexture,
    levels: Vec<CachedTexture>,
    x: Vec<CachedTexture>,
    /// The fast blur's X results (half the level's width).
    x_half: Vec<CachedTexture>,
    y: Vec<CachedTexture>,
    /// A 1 x 1 black texture standing in for "no second input".
    black: CachedTexture,
}

impl BloomTextures {
    /// The final bloom (Bloom1's Y result, half size).
    pub fn result(&self) -> &TextureView {
        &self.y[0].default_view
    }
}

fn prepare_textures(
    mut commands: Commands,
    mut cache: ResMut<TextureCache>,
    device: Res<RenderDevice>,
    views: Query<(Entity, &ExtractedCamera), With<crate::tonemap::GameTonemap>>,
) {
    for (e, camera) in &views {
        let Some(size) = camera.physical_viewport_size else { continue };
        let tex = |cache: &mut TextureCache, label: &'static str, s: UVec2| {
            cache.get(
                &device,
                TextureDescriptor {
                    label: Some(label),
                    size: Extent3d { width: s.x.max(1), height: s.y.max(1), depth_or_array_layers: 1 },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: TextureDimension::D2,
                    format: FORMAT,
                    usage: TextureUsages::RENDER_ATTACHMENT | TextureUsages::TEXTURE_BINDING,
                    view_formats: &[],
                },
            )
        };
        // DivideAndRoundUp by 2 each level.
        let half_size = (size + 1) / 2;
        let mut sizes = vec![half_size];
        for _ in 1..6 {
            let s = *sizes.last().unwrap();
            sizes.push((s + 1) / 2);
        }
        let half = tex(&mut cache, "bloom_half", half_size);
        let levels = sizes.iter().map(|s| tex(&mut cache, "bloom_level", *s)).collect();
        let x = sizes.iter().map(|s| tex(&mut cache, "bloom_x", *s)).collect();
        let x_half = sizes.iter().map(|s| tex(&mut cache, "bloom_x_half", UVec2::new(s.x.div_ceil(2), s.y))).collect();
        let y = sizes.iter().map(|s| tex(&mut cache, "bloom_y", *s)).collect();
        let black = tex(&mut cache, "bloom_black", UVec2::ONE);
        commands.entity(e).insert(BloomTextures { half, levels, x, x_half, y, black });
    }
}

#[allow(clippy::too_many_arguments)]
fn run_pass(
    ctx: &mut RenderContext,
    cache: &PipelineCache,
    pipelines: &BloomPipelines,
    pipeline: &RenderPipeline,
    source: &TextureView,
    second: &TextureView,
    target: &TextureView,
    uniform: &PassUniform,
) {
    let mut buffer = encase::UniformBuffer::new(Vec::<u8>::new());
    buffer.write(uniform).expect("bloom uniform");
    let device = ctx.render_device().clone();
    let gpu = device.create_buffer_with_data(&BufferInitDescriptor { label: Some("game_bloom_pass"), contents: buffer.as_ref(), usage: BufferUsages::UNIFORM });
    let bind_group = device.create_bind_group(
        None,
        &cache.get_bind_group_layout(&pipelines.layout),
        &BindGroupEntries::sequential((source, second, &pipelines.sampler, gpu.as_entire_binding())),
    );
    let mut pass = ctx.command_encoder().begin_render_pass(&RenderPassDescriptor {
        label: Some("game_bloom"),
        color_attachments: &[Some(RenderPassColorAttachment { view: target, depth_slice: None, resolve_target: None, ops: Operations::default() })],
        depth_stencil_attachment: None,
        timestamp_writes: None,
        occlusion_query_set: None,
        multiview_mask: None,
    });
    pass.set_pipeline(pipeline);
    pass.set_bind_group(0, &bind_group, &[]);
    pass.draw(0..3, 0..1);
}

fn empty_uniform(texel: Vec2) -> PassUniform {
    PassUniform { texel: texel.extend(0.0).extend(0.0), params: Vec4::ZERO, offsets: [Vec4::ZERO; MAX_SAMPLES], weights: [Vec4::ZERO; MAX_SAMPLES] }
}

pub(crate) fn game_bloom(
    view: ViewQuery<(&ViewTarget, &ExtractedCamera, &crate::tonemap::GameTonemap, &BloomTextures)>,
    pipelines: Res<BloomPipelines>,
    mut specialized: ResMut<SpecializedRenderPipelines<BloomPipelines>>,
    cache: Res<PipelineCache>,
    mut ctx: RenderContext,
) {
    let (target, camera, settings, textures) = view.into_inner();
    let Some(view_size) = camera.physical_viewport_size else { return };
    let ids = [Pass::Downsample, Pass::Setup, Pass::Filter].map(|p| specialized.specialize(&cache, &pipelines, p));
    let (Some(down), Some(setup), Some(filter)) = (cache.get_render_pipeline(ids[0]), cache.get_render_pipeline(ids[1]), cache.get_render_pipeline(ids[2])) else { return };
    let b = &settings.bloom;
    let black = &textures.black.default_view;
    let size_of = |t: &CachedTexture| Vec2::new(t.texture.width() as f32, t.texture.height() as f32);
    // SceneColorHalfRes: from the scene colour (its texel size).
    let scene = target.main_texture_view();
    let scene_texel = Vec2::ONE / view_size.as_vec2();
    run_pass(&mut ctx, &cache, &pipelines, down, scene, black, &textures.half.default_view, &empty_uniform(scene_texel));
    // BloomSetup.
    let mut u = empty_uniform(Vec2::ONE / size_of(&textures.half));
    u.params.z = b.threshold;
    run_pass(&mut ctx, &cache, &pipelines, setup, &textures.half.default_view, black, &textures.levels[0].default_view, &u);
    // BloomDownsample1..5.
    for i in 1..6 {
        let texel = Vec2::ONE / size_of(&textures.levels[i - 1]);
        run_pass(&mut ctx, &cache, &pipelines, down, &textures.levels[i - 1].default_view, black, &textures.levels[i].default_view, &empty_uniform(texel));
    }
    // The stages, Bloom6 on the smallest level to Bloom1 on the largest; each adds the one before.
    let mut previous: Option<usize> = None;
    for stage in (0..6).rev() {
        let size = b.sizes[stage];
        if size <= 9.99999993922529e-09 {
            continue;
        }
        let level = &textures.levels[stage];
        let src = size_of(level);
        let size = size * b.size_scale;
        // The source's scale against the full-size buffer (DivideAndRoundUp), and the radius from the view's width
        // (twice that for a fast blur's Y pass).
        let radius = |input: Vec2, fast_y: bool| {
            let scale = view_size.x.div_ceil((input.x as u32).max(1)).max(1) as f32;
            size * (view_size.x as f32 / scale) * if fast_y { 2.0 } else { 1.0 } * 0.004999999888241291
        };
        let fill = |u: &mut PassUniform, kernel: &[(f32, f32)], axis: Vec2, tint: Vec4| {
            u.params.x = kernel.len() as f32;
            for (k, (o, w)) in kernel.iter().enumerate() {
                u.offsets[k] = (axis * *o).extend(0.0).extend(0.0);
                u.weights[k] = tint * *w;
            }
        };
        // DoFastBlur, X: 0.02 x the source's width x size against the threshold.
        let fast_x = 0.02 * src.x * size >= FAST_BLUR_THRESHOLD;
        let x_out = if fast_x { &textures.x_half[stage] } else { &textures.x[stage] };
        let mut ux = empty_uniform(Vec2::ONE / src);
        fill(&mut ux, &gaussian_kernel(radius(src, false)), Vec2::new(1.0 / src.x, 0.0), Vec4::ONE);
        run_pass(&mut ctx, &cache, &pipelines, filter, &level.default_view, black, &x_out.default_view, &ux);
        // DoFastBlur, Y: a source narrower than 0.75 x the buffer's aspect (the half-width X result).
        let y_src = size_of(x_out);
        let fast_y = y_src.x / y_src.y < view_size.x as f32 / view_size.y.max(1) as f32 * 0.75;
        let mut uy = empty_uniform(Vec2::ONE / y_src);
        fill(&mut uy, &gaussian_kernel(radius(y_src, fast_y)), Vec2::new(0.0, 1.0 / y_src.y), b.tints[stage] * 0.1666666716337204);
        let second = match previous {
            Some(p) => {
                uy.params.y = 1.0;
                &textures.y[p].default_view
            }
            None => black,
        };
        run_pass(&mut ctx, &cache, &pipelines, filter, &x_out.default_view, second, &textures.y[stage].default_view, &uy);
        previous = Some(stage);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kernel_is_normalised_and_symmetric() {
        let k = gaussian_kernel(5.76);
        let sum: f32 = k.iter().map(|p| p.1).sum();
        assert!((sum - 1.0).abs() < 1e-5);
        // ceil(5.76) = 6: pairs from -6 to 6 step 2 (7 samples), the last unpaired.
        assert_eq!(k.len(), 7);
        assert_eq!(k.last().unwrap().0, 6.0);
    }

    #[test]
    fn big_radii_are_clamped_to_the_sample_limit() {
        // 31 at most: -31 to 31 in steps of 2.
        assert_eq!(gaussian_kernel(200.0).len(), 32);
    }
}
