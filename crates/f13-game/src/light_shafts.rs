//! The engine's light shaft occlusion (UE 4.18 as this build runs it; decompiled, MODLOG "Light shafts"): the fog
//! (FExponentialHeightFogPS) multiplies its whole inscattering by it, so the clear night's fog darkens toward
//! OcclusionMaskDarkness where trees and terrain stand between the camera and the moon.
//!
//! The passes (0x140bdff90, for a light with bEnableLightShaftOcclusion, per view, gated by 0x140c1f260: the light's
//! position for light shafts in front of the camera, clip w > 0; the moon behind it draws nothing):
//! - the downsample (0x140b6f430; TDownsampleLightShaftsPixelShader<Directional, true>): r.LightShaftDownSampleFactor
//!   2, each texel max((1 - 8 x(1-x) y(1-y))^4, saturate(SceneDepth / OcclusionDepthRange));
//! - r.LightShaftBlurPasses 3 blurs (0x140b97260; FBlurLightShaftsPixelShader): r.LightShaftNumSamples 12 samples from
//!   the texel toward the light's screen position, min(exp2(pass x 2.263035) x r.LightShaftFirstPassDistance 0.1, 1)
//!   of the way;
//! - the finish (0x140b96e10; FFinishOcclusionPixelShader): lerp(lerp(OcclusionMaskDarkness, 1, o^2), 1,
//!   min(0.2 |origin - uv|, 1)), DistanceFade 0 for a directional light.
//! The constants (FUN_140b89ae0): TextureSpaceBlurOrigin the light's screen UV times AspectRatioAndInvAspectRatio.zw
//! = (1, H / W), UVMinMax (0, 0, (w - 1) / w, (h - 1) / h) of the half size, LightShaftParameters (1 /
//! OcclusionDepthRange, BloomScale, 1, OcclusionMaskDarkness). The cvars are the engine's defaults in this build
//! (0x142d553e8...0x142d553f4) and the user's PostProcessQuality 2 runs them (r.LightShaftQuality 1). The temporal
//! history pass (0x140b97980) runs only under temporal AA (the game's default is none).
//! Not ported: the light shaft bloom (the moon's BloomScale 0.3, BloomThreshold 0.03). PLACEHOLDER: a view without
//! one (moon behind, no light shafts, or no depth prepass) binds white to the fog (UE's GWhiteTexture fallback; the
//! fog's binding not read in the binary).

use bevy::asset::{embedded_asset, load_embedded_asset};
use bevy::core_pipeline::prepass::{DepthPrepass, ViewPrepassTextures};
use bevy::core_pipeline::schedule::{Core3d, Core3dSystems};
use bevy::core_pipeline::FullscreenShader;
use bevy::pbr::FogOcclusion;
use bevy::prelude::*;
use bevy::render::camera::ExtractedCamera;
use bevy::render::extract_component::{ExtractComponent, ExtractComponentPlugin};
use bevy::render::render_resource::binding_types::{sampler, texture_2d, texture_depth_2d, uniform_buffer_sized};
use bevy::render::render_resource::*;
use bevy::render::renderer::{RenderContext, RenderDevice, ViewQuery};
use bevy::render::sync_component::SyncComponent;
use bevy::render::texture::{CachedTexture, TextureCache};
use bevy::render::view::Msaa;
use bevy::render::{GpuResourceAppExt, Render, RenderApp, RenderStartup, RenderSystems};
use bevy::shader::Shader;

/// r.LightShaftDownSampleFactor.
const DOWNSAMPLE: u32 = 2;
/// r.LightShaftBlurPasses.
const BLUR_PASSES: u32 = 3;
/// r.LightShaftFirstPassDistance.
const FIRST_PASS_DISTANCE: f32 = 0.1;

/// A directional light's light shaft occlusion (DirectionalLightComponent: bEnableLightShaftOcclusion with its
/// OcclusionMaskDarkness and OcclusionDepthRange, m).
#[derive(Component, Clone, Copy)]
pub struct LightShaftOcclusion {
    pub darkness: f32,
    pub depth_range: f32,
}

pub struct LightShaftsPlugin;

impl Plugin for LightShaftsPlugin {
    fn build(&self, app: &mut App) {
        embedded_asset!(app, "shaders/light_shafts.wgsl");
        app.add_plugins(ExtractComponentPlugin::<LightShafts>::default()).add_systems(PostUpdate, view_params.after(TransformSystems::Propagate));
        let Some(render_app) = app.get_sub_app_mut(RenderApp) else { return };
        render_app
            .init_gpu_resource::<SpecializedRenderPipelines<ShaftPipelines>>()
            .add_systems(RenderStartup, init_pipelines)
            .add_systems(Render, prepare_textures.in_set(RenderSystems::PrepareResources))
            .add_systems(Core3d, light_shafts.in_set(Core3dSystems::MainPass).before(bevy::core_pipeline::core_3d::main_opaque_pass_3d));
    }
}

/// A view's light shafts this frame: the light's screen UV (in front of the camera), its darkness and 1 /
/// OcclusionDepthRange (per cm), the camera's near plane (m).
#[derive(Component, Clone)]
pub struct LightShafts {
    screen_uv: Vec2,
    darkness: f32,
    inv_range: f32,
    near: f32,
    active: bool,
}

impl SyncComponent for LightShafts {
    type Target = LightShafts;
}

impl ExtractComponent for LightShafts {
    type QueryData = &'static LightShafts;
    type QueryFilter = ();
    type Out = LightShafts;

    fn extract_component(s: bevy::ecs::query::QueryItem<'_, '_, Self::QueryData>) -> Option<Self::Out> {
        s.active.then(|| s.clone())
    }
}

/// Each depth-prepassed view: the light's position for light shafts (from the view along the light's reverse
/// direction, at infinity) projected, drawn only in front of the camera (0x140c1f260).
fn view_params(
    mut commands: Commands,
    cameras: Query<(Entity, &GlobalTransform, &Projection, &Msaa), (With<Camera3d>, With<DepthPrepass>)>,
    lights: Query<(&GlobalTransform, &LightShaftOcclusion, &DirectionalLight)>,
) {
    // F13_NO_LIGHT_SHAFTS: without them (comparisons).
    let light = lights.iter().find(|(_, _, l)| l.illuminance > 0.0).filter(|_| std::env::var_os("F13_NO_LIGHT_SHAFTS").is_none());
    for (e, xf, projection, msaa) in &cameras {
        let Projection::Perspective(p) = projection else { continue };
        let mut shafts = LightShafts { screen_uv: Vec2::ZERO, darkness: 1.0, inv_range: 0.0, near: p.near, active: false };
        if let Some((light_xf, occlusion, _)) = light.filter(|_| *msaa == Msaa::Off) {
            let to_light = -light_xf.forward().as_vec3();
            let view = xf.rotation().inverse() * to_light;
            let clip = projection.get_clip_from_view() * view.extend(0.0);
            if clip.w > 0.0 {
                let ndc = clip.xy() / clip.w;
                shafts = LightShafts {
                    screen_uv: Vec2::new(ndc.x * 0.5 + 0.5, 0.5 - ndc.y * 0.5),
                    darkness: occlusion.darkness,
                    inv_range: 1.0 / (occlusion.depth_range * 100.0).max(1.0),
                    near: p.near,
                    active: true,
                };
            }
        }
        commands.entity(e).insert(shafts);
    }
}

#[derive(Clone, Copy, ShaderType, Default)]
struct Params {
    origin: Vec2,
    near: f32,
    inv_range: f32,
    uv_min_max: Vec4,
    aspect: Vec4,
    darkness: f32,
    step: f32,
    pad: Vec2,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum Pass {
    Downsample,
    Blur,
    Finish,
}

#[derive(Resource)]
pub(crate) struct ShaftPipelines {
    downsample_layout: BindGroupLayoutDescriptor,
    blur_layout: BindGroupLayoutDescriptor,
    linear: Sampler,
    fullscreen_shader: FullscreenShader,
    shader: Handle<Shader>,
}

fn init_pipelines(mut commands: Commands, device: Res<RenderDevice>, fullscreen_shader: Res<FullscreenShader>, asset_server: Res<AssetServer>) {
    let uniform = || uniform_buffer_sized(false, Some(Params::min_size()));
    let fragment = ShaderStages::FRAGMENT;
    // The blurs' and the finish's bilinear, clamped sampler; the fog's too.
    let linear = device.create_sampler(&SamplerDescriptor {
        address_mode_u: AddressMode::ClampToEdge,
        address_mode_v: AddressMode::ClampToEdge,
        mag_filter: FilterMode::Linear,
        min_filter: FilterMode::Linear,
        ..default()
    });
    commands.insert_resource(ShaftPipelines {
        downsample_layout: BindGroupLayoutDescriptor::new("light_shafts_downsample", &BindGroupLayoutEntries::sequential(fragment, (uniform(), texture_depth_2d()))),
        blur_layout: BindGroupLayoutDescriptor::new(
            "light_shafts_blur",
            &BindGroupLayoutEntries::sequential(fragment, (uniform(), texture_2d(TextureSampleType::Float { filterable: true }), sampler(SamplerBindingType::Filtering))),
        ),
        linear,
        fullscreen_shader: fullscreen_shader.clone(),
        shader: load_embedded_asset!(asset_server.as_ref(), "shaders/light_shafts.wgsl"),
    });
}

impl SpecializedRenderPipeline for ShaftPipelines {
    type Key = Pass;

    fn specialize(&self, pass: Pass) -> RenderPipelineDescriptor {
        let (def, entry, layout) = match pass {
            Pass::Downsample => ("DOWNSAMPLE", "downsample", &self.downsample_layout),
            Pass::Blur => ("BLUR", "blur", &self.blur_layout),
            Pass::Finish => ("FINISH", "finish", &self.blur_layout),
        };
        RenderPipelineDescriptor {
            label: Some(format!("light_shafts_{entry}").into()),
            layout: vec![layout.clone()],
            vertex: self.fullscreen_shader.to_vertex_state(),
            fragment: Some(FragmentState {
                shader: self.shader.clone(),
                shader_defs: vec![def.into()],
                entry_point: Some(entry.into()),
                targets: vec![Some(ColorTargetState { format: TextureFormat::R16Float, blend: None, write_mask: ColorWrites::ALL })],
                ..default()
            }),
            ..default()
        }
    }
}

/// A view's two ping-pong targets (LightShafts0 / 1) and the finished occlusion the fog reads.
#[derive(Component)]
pub struct ShaftTextures {
    a: CachedTexture,
    b: CachedTexture,
    finished: CachedTexture,
}

fn prepare_textures(
    mut commands: Commands,
    mut cache: ResMut<TextureCache>,
    device: Res<RenderDevice>,
    pipelines: Option<Res<ShaftPipelines>>,
    views: Query<(Entity, &ExtractedCamera), With<LightShafts>>,
    stale: Query<Entity, (With<FogOcclusion>, Without<LightShafts>)>,
) {
    // Views without light shafts this frame: the fog's white fallback.
    for e in &stale {
        commands.entity(e).remove::<(FogOcclusion, ShaftTextures)>();
    }
    let Some(pipelines) = pipelines else { return };
    for (e, camera) in &views {
        let Some(size) = camera.physical_viewport_size else { continue };
        let half = (size / DOWNSAMPLE).max(UVec2::ONE);
        let mut tex = |label: &'static str| {
            cache.get(
                &device,
                TextureDescriptor {
                    label: Some(label),
                    size: Extent3d { width: half.x, height: half.y, depth_or_array_layers: 1 },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: TextureDimension::D2,
                    format: TextureFormat::R16Float,
                    usage: TextureUsages::RENDER_ATTACHMENT | TextureUsages::TEXTURE_BINDING,
                    view_formats: &[],
                },
            )
        };
        let textures = ShaftTextures { a: tex("light_shafts_0"), b: tex("light_shafts_1"), finished: tex("light_shafts_occlusion") };
        commands.entity(e).insert((FogOcclusion { view: textures.finished.default_view.clone(), sampler: pipelines.linear.clone() }, textures));
    }
}

/// The blur's distance for a pass.
fn pass_step(pass: u32) -> f32 {
    ((pass as f32 * 2.263035).exp2() * FIRST_PASS_DISTANCE).min(1.0)
}

pub(crate) fn light_shafts(
    view: ViewQuery<(&ExtractedCamera, &LightShafts, &ShaftTextures, &ViewPrepassTextures)>,
    pipelines: Res<ShaftPipelines>,
    mut specialized: ResMut<SpecializedRenderPipelines<ShaftPipelines>>,
    cache: Res<PipelineCache>,
    mut ctx: RenderContext,
) {
    let (camera, shafts, textures, prepass) = view.into_inner();
    let Some(size) = camera.physical_viewport_size else { return };
    let Some(depth) = prepass.depth_view() else { return };
    let ids = [Pass::Downsample, Pass::Blur, Pass::Finish].map(|p| specialized.specialize(&cache, &pipelines, p));
    let Some(pipes) = ids.iter().map(|id| cache.get_render_pipeline(*id)).collect::<Option<Vec<_>>>() else { return };
    let device = ctx.render_device().clone();
    let half = (size / DOWNSAMPLE).max(UVec2::ONE).as_vec2();
    let (w, h) = (size.x as f32, size.y as f32);
    let aspect = Vec4::new(1.0, w / h, 1.0, h / w);
    let base = Params {
        origin: shafts.screen_uv * aspect.zw(),
        near: shafts.near,
        inv_range: shafts.inv_range,
        uv_min_max: Vec4::new(0.0, 0.0, (half.x - 1.0) / half.x, (half.y - 1.0) / half.y),
        aspect,
        darkness: shafts.darkness,
        ..default()
    };
    let buffer = |p: &Params| {
        let mut b = encase::UniformBuffer::new(Vec::<u8>::new());
        b.write(p).expect("light shafts uniform");
        device.create_buffer_with_data(&BufferInitDescriptor { label: Some("light_shafts"), contents: b.as_ref(), usage: BufferUsages::UNIFORM })
    };
    let draw = |ctx: &mut RenderContext, pipeline: &RenderPipeline, group: &BindGroup, out: &TextureView| {
        let mut pass = ctx.command_encoder().begin_render_pass(&RenderPassDescriptor {
            label: Some("light_shafts"),
            color_attachments: &[Some(RenderPassColorAttachment { view: out, depth_slice: None, resolve_target: None, ops: Operations::default() })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_pipeline(pipeline);
        pass.set_bind_group(0, group, &[]);
        pass.draw(0..3, 0..1);
    };
    let downsample_layout = cache.get_bind_group_layout(&pipelines.downsample_layout);
    let blur_layout = cache.get_bind_group_layout(&pipelines.blur_layout);
    {
        let b = buffer(&base);
        let g = device.create_bind_group(None, &downsample_layout, &BindGroupEntries::sequential((b.as_entire_binding(), depth)));
        draw(&mut ctx, pipes[0], &g, &textures.a.default_view);
    }
    // The blurs, ping-ponging; the last result in `src`.
    let (mut src, mut dst) = (&textures.a, &textures.b);
    for pass in 0..BLUR_PASSES {
        let b = buffer(&Params { step: pass_step(pass), ..base });
        let g = device.create_bind_group(None, &blur_layout, &BindGroupEntries::sequential((b.as_entire_binding(), &src.default_view, &pipelines.linear)));
        draw(&mut ctx, pipes[1], &g, &dst.default_view);
        std::mem::swap(&mut src, &mut dst);
    }
    let b = buffer(&base);
    let g = device.create_bind_group(None, &blur_layout, &BindGroupEntries::sequential((b.as_entire_binding(), &src.default_view, &pipelines.linear)));
    draw(&mut ctx, pipes[2], &g, &textures.finished.default_view);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blur_passes_reach_the_light() {
        assert!((pass_step(0) - 0.1).abs() < 1e-6);
        assert!((pass_step(1) - 0.48).abs() < 0.01);
        assert_eq!(pass_step(2), 1.0);
    }
}
