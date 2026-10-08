//! Screen effects (post-process materials as blendables on the camera's view, after tonemapping), from the game's
//! compiled material shaders.
//!
//! - The teleport static (Jason_Base's TeleportScreenEffect, PP_VHS_GlitchEffect: shaders/vhs.wgsl). PlayDistortionEffect
//!   (0x1403ffd50) adds his TeleportScreenEffectInst to the counselor's ForcedPostProcessVolume blendables (weight 1)
//!   as she hears him Morph or Stalk near her (jason.rs `distortion`); his tick (FUN_14040d900) sets its OffsetAmount to
//!   TeleportJitterCurve (VHSJitterCurve) at the time since, and removes it after TeleportScreenTime.

use bevy::asset::{embedded_asset, load_embedded_asset};
use bevy::core_pipeline::schedule::{Core3d, Core3dSystems};
use bevy::core_pipeline::FullscreenShader;
use bevy::prelude::*;
use bevy::render::extract_component::{ComponentUniforms, DynamicUniformIndex, ExtractComponent, ExtractComponentPlugin, UniformComponentPlugin};
use bevy::render::render_resource::binding_types::{sampler, texture_2d, uniform_buffer};
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
        app.add_plugins((ExtractComponentPlugin::<VhsGlitch>::default(), UniformComponentPlugin::<VhsUniform>::default()))
            .add_systems(Update, teleport_static);
        let Some(render_app) = app.get_sub_app_mut(RenderApp) else { return };
        render_app
            .init_gpu_resource::<SpecializedRenderPipelines<VhsPipeline>>()
            .add_systems(RenderStartup, init_pipeline)
            .add_systems(Render, prepare_pipelines.in_set(RenderSystems::Prepare))
            .add_systems(Core3d, vhs.after(crate::tonemap::game_tonemap).in_set(Core3dSystems::PostProcess));
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
