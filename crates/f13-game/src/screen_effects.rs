//! Screen effects (post-process materials as blendables on the camera's view, after tonemapping), from the game's
//! compiled material shaders.
//!
//! - The teleport static (Jason_Base's TeleportScreenEffect, PP_VHS_GlitchEffect: shaders/vhs.wgsl). PlayDistortionEffect
//!   (0x1403ffd50) adds his TeleportScreenEffectInst to the counselor's ForcedPostProcessVolume blendables (weight 1)
//!   as she hears him Morph or Stalk near her (jason.rs `distortion`); his tick (FUN_14040d900) sets its OffsetAmount to
//!   TeleportJitterCurve (VHSJitterCurve) at the time since, and removes it after TeleportScreenTime.
//! - Shift's blur (ShiftScreenEffect, PP_Player_Jason_Shift_01: shaders/shift.wgsl). OnRep_ShiftActive (0x1403fd2d0) adds
//!   his ShiftScreenEffectInst to his own ForcedPostProcessVolume's blendables while he shifts and removes it after:
//!   on the played Jason's camera.

use bevy::asset::{embedded_asset, load_embedded_asset};
use bevy::core_pipeline::schedule::{Core3d, Core3dSystems};
use bevy::core_pipeline::FullscreenShader;
use bevy::prelude::*;
use bevy::render::extract_component::{ComponentUniforms, DynamicUniformIndex, ExtractComponent, ExtractComponentPlugin, UniformComponentPlugin};
use bevy::render::render_asset::RenderAssets;
use bevy::render::render_resource::binding_types::{sampler, texture_2d, uniform_buffer};
use bevy::render::texture::GpuImage;
use bevy::render::render_resource::*;
use bevy::render::renderer::{RenderContext, RenderDevice, ViewQuery};
use bevy::render::sync_component::SyncComponent;
use bevy::render::view::{ExtractedView, ViewTarget};
use bevy::render::{GpuResourceAppExt, Render, RenderApp, RenderStartup, RenderSystems};
use bevy::shader::Shader;

pub struct ScreenEffectsPlugin;

impl Plugin for ScreenEffectsPlugin {
    fn build(&self, app: &mut App) {
        embedded_asset!(app, "shaders/vhs.wgsl");
        embedded_asset!(app, "shaders/shift.wgsl");
        app.add_plugins((ExtractComponentPlugin::<VhsGlitch>::default(), UniformComponentPlugin::<VhsUniform>::default(), ExtractComponentPlugin::<ShiftBlur>::default()))
            .add_systems(Update, (teleport_static, shift_blur));
        let Some(render_app) = app.get_sub_app_mut(RenderApp) else { return };
        render_app
            .init_gpu_resource::<SpecializedRenderPipelines<VhsPipeline>>()
            .init_gpu_resource::<SpecializedRenderPipelines<ShiftPipeline>>()
            .add_systems(RenderStartup, (init_pipeline, init_shift_pipeline))
            .add_systems(Render, (prepare_pipelines, prepare_shift_pipelines).in_set(RenderSystems::Prepare))
            .add_systems(Core3d, (vhs.after(crate::tonemap::game_tonemap), shift.after(crate::tonemap::game_tonemap).before(vhs)).in_set(Core3dSystems::PostProcess));
    }
}

/// The teleport static on a camera: OffsetAmount and the view's time.
#[derive(Component, Clone, Copy)]
pub struct VhsGlitch {
    pub offset: f32,
    pub time: f32,
}

#[derive(Component, ShaderType, Clone)]
pub struct VhsUniform {
    v: Vec4,
}

impl SyncComponent for VhsGlitch {
    type Target = (VhsGlitch, VhsUniform);
}

impl ExtractComponent for VhsGlitch {
    type QueryData = &'static VhsGlitch;
    type QueryFilter = ();
    type Out = (VhsGlitch, VhsUniform);

    fn extract_component(g: bevy::ecs::query::QueryItem<'_, '_, Self::QueryData>) -> Option<Self::Out> {
        Some((*g, VhsUniform { v: Vec4::new(g.offset, g.time, 0.0, 0.0) }))
    }
}

/// The counselor's camera shows the static while his teleport effect runs (jason.rs TeleportEffect): OffsetAmount
/// from TeleportJitterCurve at the time since it began.
fn teleport_static(
    mut commands: Commands,
    time: Res<Time>,
    effect: Option<Res<crate::jason::TeleportEffect>>,
    killer: Option<Res<crate::character::KillerCharacter>>,
    cameras: Query<(Entity, Has<VhsGlitch>), With<crate::player::PlayerCamera>>,
    mut curve: Local<Option<f13_core::curve::RichCurve>>,
) {
    if curve.is_none() {
        *curve = killer.and_then(|k| k.0.info.curve("TeleportJitterCurve"));
    }
    for (e, has) in &cameras {
        match effect.as_deref() {
            Some(t) => {
                let since = t.total - t.left;
                let offset = curve.as_ref().map_or(0.0, |c| c.eval(since));
                commands.entity(e).insert(VhsGlitch { offset, time: time.elapsed_secs() });
            }
            None if has => {
                commands.entity(e).remove::<VhsGlitch>();
            }
            None => {}
        }
    }
}

/// Shift's blur on a camera: BaseTexture1A (T_Vignette_Simple_01, half-float RGBA: its .dds keeps the alpha).
#[derive(Component, Clone)]
pub struct ShiftBlur {
    pub vignette: Handle<Image>,
}

impl SyncComponent for ShiftBlur {
    type Target = ShiftBlur;
}

impl ExtractComponent for ShiftBlur {
    type QueryData = &'static ShiftBlur;
    type QueryFilter = ();
    type Out = ShiftBlur;

    fn extract_component(s: bevy::ecs::query::QueryItem<'_, '_, Self::QueryData>) -> Option<Self::Out> {
        Some(s.clone())
    }
}

/// The material's BaseTexture1A.
const SHIFT_VIGNETTE: &str = "SummerCamp/Content/Effects/Textures/PostProcess/T_Vignette_Simple_01.dds";

/// The played Jason's camera shows Shift's blur while he shifts.
fn shift_blur(
    mut commands: Commands,
    assets: Res<AssetServer>,
    play: Option<Res<crate::jason_player::PlayJason>>,
    jasons: Query<&crate::jason::Jason>,
    cameras: Query<(Entity, Has<ShiftBlur>), With<crate::player::PlayerCamera>>,
    mut vignette: Local<Option<Handle<Image>>>,
) {
    let on = play.is_some() && jasons.iter().any(|j| j.shifting());
    for (e, has) in &cameras {
        if on && !has {
            let v = vignette.get_or_insert_with(|| assets.load(SHIFT_VIGNETTE)).clone();
            commands.entity(e).insert(ShiftBlur { vignette: v });
        } else if !on && has {
            commands.entity(e).remove::<ShiftBlur>();
        }
    }
}

#[derive(Resource)]
struct ShiftPipeline {
    layout: BindGroupLayoutDescriptor,
    sampler: Sampler,
    fullscreen_shader: FullscreenShader,
    shader: Handle<Shader>,
}

fn init_shift_pipeline(mut commands: Commands, device: Res<RenderDevice>, fullscreen_shader: Res<FullscreenShader>, asset_server: Res<AssetServer>) {
    let layout = BindGroupLayoutDescriptor::new(
        "shift_layout",
        &BindGroupLayoutEntries::sequential(
            ShaderStages::FRAGMENT,
            (
                texture_2d(TextureSampleType::Float { filterable: true }),
                sampler(SamplerBindingType::Filtering),
                texture_2d(TextureSampleType::Float { filterable: true }),
                sampler(SamplerBindingType::Filtering),
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
    commands.insert_resource(ShiftPipeline { layout, sampler, fullscreen_shader: fullscreen_shader.clone(), shader: load_embedded_asset!(asset_server.as_ref(), "shaders/shift.wgsl") });
}

#[derive(Component)]
struct ShiftPipelineId(CachedRenderPipelineId);

impl SpecializedRenderPipeline for ShiftPipeline {
    type Key = TextureFormat;

    fn specialize(&self, format: TextureFormat) -> RenderPipelineDescriptor {
        RenderPipelineDescriptor {
            label: Some("shift".into()),
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

fn prepare_shift_pipelines(
    mut commands: Commands,
    cache: Res<PipelineCache>,
    mut pipelines: ResMut<SpecializedRenderPipelines<ShiftPipeline>>,
    pipeline: Res<ShiftPipeline>,
    views: Query<(Entity, &ExtractedView), With<ShiftBlur>>,
) {
    for (e, view) in &views {
        let id = pipelines.specialize(&cache, &pipeline, view.target_format);
        commands.entity(e).insert(ShiftPipelineId(id));
    }
}

fn shift(
    view: ViewQuery<(&ViewTarget, &ShiftBlur, &ShiftPipelineId)>,
    pipeline: Res<ShiftPipeline>,
    cache: Res<PipelineCache>,
    images: Res<RenderAssets<GpuImage>>,
    mut ctx: RenderContext,
) {
    let (target, blur, id) = view.into_inner();
    let (Some(render_pipeline), Some(vignette)) = (cache.get_render_pipeline(id.0), images.get(&blur.vignette)) else { return };
    let post = target.post_process_write();
    let bind_group = ctx.render_device().create_bind_group(
        None,
        &cache.get_bind_group_layout(&pipeline.layout),
        &BindGroupEntries::sequential((post.source, &pipeline.sampler, &vignette.texture_view, &pipeline.sampler)),
    );
    let mut pass = ctx.command_encoder().begin_render_pass(&RenderPassDescriptor {
        label: Some("shift"),
        color_attachments: &[Some(RenderPassColorAttachment { view: post.destination, depth_slice: None, resolve_target: None, ops: Operations::default() })],
        depth_stencil_attachment: None,
        timestamp_writes: None,
        occlusion_query_set: None,
        multiview_mask: None,
    });
    pass.set_pipeline(render_pipeline);
    pass.set_bind_group(0, &bind_group, &[]);
    pass.draw(0..3, 0..1);
}

#[derive(Resource)]
struct VhsPipeline {
    layout: BindGroupLayoutDescriptor,
    sampler: Sampler,
    fullscreen_shader: FullscreenShader,
    shader: Handle<Shader>,
}

fn init_pipeline(mut commands: Commands, device: Res<RenderDevice>, fullscreen_shader: Res<FullscreenShader>, asset_server: Res<AssetServer>) {
    let layout = BindGroupLayoutDescriptor::new(
        "vhs_layout",
        &BindGroupLayoutEntries::sequential(
            ShaderStages::FRAGMENT,
            (texture_2d(TextureSampleType::Float { filterable: true }), sampler(SamplerBindingType::Filtering), uniform_buffer::<VhsUniform>(true)),
        ),
    );
    // Bilinear, clamped (the post-process input's sampler).
    let sampler = device.create_sampler(&SamplerDescriptor {
        address_mode_u: AddressMode::ClampToEdge,
        address_mode_v: AddressMode::ClampToEdge,
        mag_filter: FilterMode::Linear,
        min_filter: FilterMode::Linear,
        ..default()
    });
    commands.insert_resource(VhsPipeline { layout, sampler, fullscreen_shader: fullscreen_shader.clone(), shader: load_embedded_asset!(asset_server.as_ref(), "shaders/vhs.wgsl") });
}

#[derive(Component)]
struct VhsPipelineId(CachedRenderPipelineId);

impl SpecializedRenderPipeline for VhsPipeline {
    type Key = TextureFormat;

    fn specialize(&self, format: TextureFormat) -> RenderPipelineDescriptor {
        RenderPipelineDescriptor {
            label: Some("vhs".into()),
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
    mut pipelines: ResMut<SpecializedRenderPipelines<VhsPipeline>>,
    pipeline: Res<VhsPipeline>,
    views: Query<(Entity, &ExtractedView), With<VhsGlitch>>,
) {
    for (e, view) in &views {
        let id = pipelines.specialize(&cache, &pipeline, view.target_format);
        commands.entity(e).insert(VhsPipelineId(id));
    }
}

fn vhs(
    view: ViewQuery<(&ViewTarget, &VhsPipelineId, &DynamicUniformIndex<VhsUniform>)>,
    pipeline: Res<VhsPipeline>,
    cache: Res<PipelineCache>,
    uniforms: Res<ComponentUniforms<VhsUniform>>,
    mut ctx: RenderContext,
) {
    let (target, id, index) = view.into_inner();
    let (Some(render_pipeline), Some(binding)) = (cache.get_render_pipeline(id.0), uniforms.binding()) else { return };
    let post = target.post_process_write();
    let bind_group =
        ctx.render_device().create_bind_group(None, &cache.get_bind_group_layout(&pipeline.layout), &BindGroupEntries::sequential((post.source, &pipeline.sampler, binding.clone())));
    let mut pass = ctx.command_encoder().begin_render_pass(&RenderPassDescriptor {
        label: Some("vhs"),
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
