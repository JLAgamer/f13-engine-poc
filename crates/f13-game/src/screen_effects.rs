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
//! - The stun's blur and red veins (StunScreenEffectMaterial, PP_Player_StunEffect_01: shaders/stun.wgsl), in his
//!   ForcedPostProcessVolume's blendables from the start (FUN_1403fb4a0). His tick (FUN_14040d900) eases
//!   DistortionDelta and AlphaDelta (FInterpTo) toward 4 and 5 at speed 8 while he's stunned (OnStunned, 0x1403f6070,
//!   sets the flag; 0x1403f66a0 clears it as the stun ends) and back to 0 at speed 1. At 0 it changes nothing, so the
//!   pass runs only while either is above it.

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
        embedded_asset!(app, "shaders/stun.wgsl");
        embedded_asset!(app, "shaders/blood.wgsl");
        app.add_plugins((
            ExtractComponentPlugin::<VhsGlitch>::default(),
            UniformComponentPlugin::<VhsUniform>::default(),
            ExtractComponentPlugin::<ShiftBlur>::default(),
            ExtractComponentPlugin::<StunEffect>::default(),
            UniformComponentPlugin::<StunUniform>::default(),
            ExtractComponentPlugin::<BloodScreen>::default(),
            UniformComponentPlugin::<BloodUniform>::default(),
        ))
        .add_systems(Startup, load_blendables)
        .add_systems(Update, (teleport_static, shift_blur, stun_effect, screen_blood));
        let Some(render_app) = app.get_sub_app_mut(RenderApp) else { return };
        render_app
            .init_gpu_resource::<SpecializedRenderPipelines<VhsPipeline>>()
            .init_gpu_resource::<SpecializedRenderPipelines<ShiftPipeline>>()
            .init_gpu_resource::<SpecializedRenderPipelines<StunPipeline>>()
            .init_gpu_resource::<SpecializedRenderPipelines<BloodPipeline>>()
            .add_systems(RenderStartup, (init_pipeline, init_shift_pipeline, init_stun_pipeline, init_blood_pipeline))
            .add_systems(Render, (prepare_pipelines, prepare_shift_pipelines, prepare_stun_pipelines, prepare_blood_pipelines).in_set(RenderSystems::Prepare))
            // The volume's blendables in the order they were added: the stun's (from the start), then Shift's.
            .add_systems(
                Core3d,
                (
                    vhs.after(crate::tonemap::game_tonemap),
                    shift.after(crate::tonemap::game_tonemap).before(vhs),
                    stun.after(crate::tonemap::game_tonemap).before(shift),
                    // (The camera's own blendables, after the volume's.)
                    blood.after(vhs),
                )
                    .in_set(Core3dSystems::PostProcess),
            );
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

/// The stun's effect on a camera: DistortionDelta, AlphaDelta, the view's time, and its textures (Stun4; Stun2 for
/// BaseTexture1A and 1B).
#[derive(Component, Clone)]
pub struct StunEffect {
    pub distortion: f32,
    pub alpha: f32,
    pub time: f32,
    pub stun4: Handle<Image>,
    pub stun2: Handle<Image>,
}

#[derive(Component, ShaderType, Clone)]
pub struct StunUniform {
    v: Vec4,
}

impl SyncComponent for StunEffect {
    type Target = (StunEffect, StunUniform);
}

impl ExtractComponent for StunEffect {
    type QueryData = &'static StunEffect;
    type QueryFilter = ();
    type Out = (StunEffect, StunUniform);

    fn extract_component(s: bevy::ecs::query::QueryItem<'_, '_, Self::QueryData>) -> Option<Self::Out> {
        Some((s.clone(), StunUniform { v: Vec4::new(s.distortion, s.alpha, s.time, 0.0) }))
    }
}

const STUN4: &str = "SummerCamp/Content/Effects/Textures/PostProcess/Stun4.dds";
const STUN2: &str = "SummerCamp/Content/Effects/Textures/PostProcess/Stun2.dds";

/// The played Jason's DistortionDelta and AlphaDelta (see the module docs).
fn stun_effect(
    mut commands: Commands,
    time: Res<Time>,
    assets: Res<AssetServer>,
    play: Option<Res<crate::jason_player::PlayJason>>,
    jasons: Query<&crate::jason::Jason>,
    cameras: Query<(Entity, Has<StunEffect>), With<crate::player::PlayerCamera>>,
    mut deltas: Local<(f32, f32)>,
    mut textures: Local<Option<(Handle<Image>, Handle<Image>)>>,
) {
    let Some(j) = jasons.iter().next().filter(|_| play.is_some()) else { return };
    let dt = time.delta_secs();
    let (targets, speed) = if j.stunned() { ((4.0, 5.0), 8.0) } else { ((0.0, 0.0), 1.0) };
    let finterp = crate::tonemap::finterp_to;
    *deltas = (finterp(deltas.0, targets.0, dt, speed), finterp(deltas.1, targets.1, dt, speed));
    let on = deltas.0 > 0.0 || deltas.1 > 0.0;
    for (e, has) in &cameras {
        if on {
            let (stun4, stun2) = textures.get_or_insert_with(|| (assets.load(STUN4), assets.load(STUN2))).clone();
            commands.entity(e).insert(StunEffect { distortion: deltas.0, alpha: deltas.1, time: time.elapsed_secs(), stun4, stun2 });
        } else if has {
            commands.entity(e).remove::<StunEffect>();
        }
    }
}

#[derive(Resource)]
struct StunPipeline {
    layout: BindGroupLayoutDescriptor,
    sampler: Sampler,
    wrap: Sampler,
    fullscreen_shader: FullscreenShader,
    shader: Handle<Shader>,
}

fn init_stun_pipeline(mut commands: Commands, device: Res<RenderDevice>, fullscreen_shader: Res<FullscreenShader>, asset_server: Res<AssetServer>) {
    let layout = BindGroupLayoutDescriptor::new(
        "stun_layout",
        &BindGroupLayoutEntries::sequential(
            ShaderStages::FRAGMENT,
            (
                texture_2d(TextureSampleType::Float { filterable: true }),
                sampler(SamplerBindingType::Filtering),
                uniform_buffer::<StunUniform>(true),
                texture_2d(TextureSampleType::Float { filterable: true }),
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
    // The material's texture samples: wrapped, trilinear.
    let wrap = device.create_sampler(&SamplerDescriptor {
        address_mode_u: AddressMode::Repeat,
        address_mode_v: AddressMode::Repeat,
        mag_filter: FilterMode::Linear,
        min_filter: FilterMode::Linear,
        mipmap_filter: MipmapFilterMode::Linear,
        ..default()
    });
    commands.insert_resource(StunPipeline { layout, sampler, wrap, fullscreen_shader: fullscreen_shader.clone(), shader: load_embedded_asset!(asset_server.as_ref(), "shaders/stun.wgsl") });
}

#[derive(Component)]
struct StunPipelineId(CachedRenderPipelineId);

impl SpecializedRenderPipeline for StunPipeline {
    type Key = TextureFormat;

    fn specialize(&self, format: TextureFormat) -> RenderPipelineDescriptor {
        RenderPipelineDescriptor {
            label: Some("stun".into()),
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

fn prepare_stun_pipelines(
    mut commands: Commands,
    cache: Res<PipelineCache>,
    mut pipelines: ResMut<SpecializedRenderPipelines<StunPipeline>>,
    pipeline: Res<StunPipeline>,
    views: Query<(Entity, &ExtractedView), With<StunEffect>>,
) {
    for (e, view) in &views {
        let id = pipelines.specialize(&cache, &pipeline, view.target_format);
        commands.entity(e).insert(StunPipelineId(id));
    }
}

fn stun(
    view: ViewQuery<(&ViewTarget, &StunEffect, &StunPipelineId, &DynamicUniformIndex<StunUniform>)>,
    pipeline: Res<StunPipeline>,
    cache: Res<PipelineCache>,
    images: Res<RenderAssets<GpuImage>>,
    uniforms: Res<ComponentUniforms<StunUniform>>,
    mut ctx: RenderContext,
) {
    let (target, effect, id, index) = view.into_inner();
    let (Some(render_pipeline), Some(stun4), Some(stun2), Some(binding)) =
        (cache.get_render_pipeline(id.0), images.get(&effect.stun4), images.get(&effect.stun2), uniforms.binding())
    else {
        return;
    };
    let post = target.post_process_write();
    let bind_group = ctx.render_device().create_bind_group(
        None,
        &cache.get_bind_group_layout(&pipeline.layout),
        &BindGroupEntries::sequential((post.source, &pipeline.sampler, binding.clone(), &stun4.texture_view, &stun2.texture_view, &pipeline.wrap)),
    );
    let mut pass = ctx.command_encoder().begin_render_pass(&RenderPassDescriptor {
        label: Some("stun"),
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

#[derive(Resource)]
pub(crate) struct ShiftPipeline {
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
pub(crate) struct ShiftPipelineId(CachedRenderPipelineId);

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

pub(crate) fn shift(
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
pub(crate) struct VhsPipeline {
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
pub(crate) struct VhsPipelineId(CachedRenderPipelineId);

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

pub(crate) fn vhs(
    // With VhsGlitch: the render world keeps the camera's entity, and its pipeline id and uniform index outlive the
    // effect (they kept the static on after it ended).
    view: ViewQuery<(&ViewTarget, &VhsPipelineId, &DynamicUniformIndex<VhsUniform>), With<VhsGlitch>>,
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

/// A kill montage's SCPostProcessBlendableNotify window (anim_blendables.json; s in its take).
#[derive(serde::Deserialize, Clone, Debug)]
#[serde(rename_all = "PascalCase")]
struct BlendWindow {
    start: f32,
    end: f32,
    material: String,
    textures: std::collections::HashMap<String, String>,
    scalars: std::collections::HashMap<String, f32>,
    blend_in: f32,
    blend_out: f32,
    min: f32,
    max: f32,
    hooks: Vec<(String, bool, bool)>,
}

impl BlendWindow {
    /// The notify's intensity `t` s into it, and whether its blend-in phase (NotifyBegin 0x1403155e0, NotifyTick
    /// 0x1403164e0): MinIntensity to MaxIntensity over BlendInTime, MaxIntensity, back to MinIntensity over the last
    /// BlendOutTime.
    fn intensity(&self, t: f32) -> (f32, bool) {
        let fade = (self.end - self.start - self.blend_out).max(0.0);
        if t < self.blend_in && self.blend_in > 0.0 {
            ((self.max - self.min) * (t / self.blend_in) + self.min, true)
        } else if t > fade && self.blend_out > 0.0 {
            (((t - fade) / self.blend_out) * (self.min - self.max) + self.max, false)
        } else {
            (self.max, false)
        }
    }

    /// A scalar parameter's value `t` s in: its intensity hook's (UseBlendIn while blending in, UseBlendOut after), else
    /// the material's own.
    fn scalar(&self, name: &str, t: f32) -> f32 {
        let (v, blending_in) = self.intensity(t);
        match self.hooks.iter().find(|h| h.0 == name) {
            Some(h) if (blending_in && h.1) || (!blending_in && h.2) => v,
            _ => self.scalars.get(name).copied().unwrap_or(0.0),
        }
    }
}

/// The animations' screen blendables: "killer/<take>" (Jason's) or "counselor/<take>" -> their windows.
#[derive(Resource, Default)]
struct AnimBlendables(std::collections::HashMap<String, Vec<BlendWindow>>);

fn load_blendables(mut commands: Commands, export: Res<crate::ExportRoot>) {
    let b = std::fs::read_to_string(export.0.join("anim_blendables.json")).ok().and_then(|t| serde_json::from_str(&t).ok());
    if b.is_none() {
        warn!("no anim_blendables.json (re-export: f13-export sounds): the kills' screen blood isn't shown");
    }
    commands.insert_resource(AnimBlendables(b.unwrap_or_default()));
}

/// The screen blood on a camera: its BloodTexture01 and bloodContrast.
#[derive(Component, Clone)]
pub struct BloodScreen {
    pub texture: Handle<Image>,
    pub contrast: f32,
}

#[derive(Component, ShaderType, Clone)]
pub struct BloodUniform {
    v: Vec4,
}

impl SyncComponent for BloodScreen {
    type Target = (BloodScreen, BloodUniform);
}

impl ExtractComponent for BloodScreen {
    type QueryData = &'static BloodScreen;
    type QueryFilter = ();
    type Out = (BloodScreen, BloodUniform);

    fn extract_component(b: bevy::ecs::query::QueryItem<'_, '_, Self::QueryData>) -> Option<Self::Out> {
        Some((b.clone(), BloodUniform { v: Vec4::new(b.contrast, 0.0, 0.0, 0.0) }))
    }
}

/// The local player's character's take's blood window, on their camera: the montage's owner locally controlled (its
/// PlayForAllClients is off in every kill), its MID's BloodTexture01 and its bloodContrast hook.
#[allow(clippy::type_complexity)]
fn screen_blood(
    mut commands: Commands,
    assets: Res<AssetServer>,
    blendables: Option<Res<AnimBlendables>>,
    play: Option<Res<crate::jason_player::PlayJason>>,
    rigs: Query<(&ChildOf, &crate::character::Rig)>,
    (local, jasons): (Query<(), (With<crate::controls::LocalPlayer>, With<crate::player::Player>)>, Query<(), With<crate::jason::Jason>>),
    cameras: Query<(Entity, Option<&BloodScreen>), With<crate::player::PlayerCamera>>,
) {
    let Some(blendables) = blendables else { return };
    let mine = |owner: Entity| if play.is_some() { jasons.contains(owner) } else { local.contains(owner) };
    let mut screen = None;
    for (parent, rig) in &rigs {
        if !mine(parent.parent()) {
            continue;
        }
        let Some((take, at)) = rig.action.as_ref() else { continue };
        let side = if jasons.contains(parent.parent()) { "killer" } else { "counselor" };
        let Some(windows) = blendables.0.get(&format!("{side}/{take}")) else { continue };
        for w in windows.iter().filter(|w| w.start <= *at && *at < w.end && w.material.starts_with("PP_Player_BloodEffect")) {
            let Some(tex) = w.textures.get("BloodTexture01") else { continue };
            screen = Some((tex.clone(), w.scalar("bloodContrast", at - w.start)));
        }
    }
    for (e, have) in &cameras {
        match &screen {
            Some((tex, contrast)) => {
                let texture = match have {
                    Some(b) if assets.get_path(b.texture.id()).is_some_and(|p| p.path().to_string_lossy() == tex.as_str()) => b.texture.clone(),
                    _ => assets.load(tex.clone()),
                };
                commands.entity(e).insert(BloodScreen { texture, contrast: *contrast });
            }
            None if have.is_some() => {
                commands.entity(e).remove::<BloodScreen>();
            }
            None => {}
        }
    }
}

#[derive(Resource)]
struct BloodPipeline {
    layout: BindGroupLayoutDescriptor,
    sampler: Sampler,
    wrap: Sampler,
    fullscreen_shader: FullscreenShader,
    shader: Handle<Shader>,
}

fn init_blood_pipeline(mut commands: Commands, device: Res<RenderDevice>, fullscreen_shader: Res<FullscreenShader>, asset_server: Res<AssetServer>) {
    let layout = BindGroupLayoutDescriptor::new(
        "blood_layout",
        &BindGroupLayoutEntries::sequential(
            ShaderStages::FRAGMENT,
            (
                texture_2d(TextureSampleType::Float { filterable: true }),
                sampler(SamplerBindingType::Filtering),
                uniform_buffer::<BloodUniform>(true),
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
    // The material's texture sample: wrapped, trilinear.
    let wrap = device.create_sampler(&SamplerDescriptor {
        address_mode_u: AddressMode::Repeat,
        address_mode_v: AddressMode::Repeat,
        mag_filter: FilterMode::Linear,
        min_filter: FilterMode::Linear,
        mipmap_filter: MipmapFilterMode::Linear,
        ..default()
    });
    commands.insert_resource(BloodPipeline { layout, sampler, wrap, fullscreen_shader: fullscreen_shader.clone(), shader: load_embedded_asset!(asset_server.as_ref(), "shaders/blood.wgsl") });
}

#[derive(Component)]
struct BloodPipelineId(CachedRenderPipelineId);

impl SpecializedRenderPipeline for BloodPipeline {
    type Key = TextureFormat;

    fn specialize(&self, format: TextureFormat) -> RenderPipelineDescriptor {
        RenderPipelineDescriptor {
            label: Some("blood".into()),
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

fn prepare_blood_pipelines(
    mut commands: Commands,
    cache: Res<PipelineCache>,
    mut pipelines: ResMut<SpecializedRenderPipelines<BloodPipeline>>,
    pipeline: Res<BloodPipeline>,
    views: Query<(Entity, &ExtractedView), With<BloodScreen>>,
) {
    for (e, view) in &views {
        let id = pipelines.specialize(&cache, &pipeline, view.target_format);
        commands.entity(e).insert(BloodPipelineId(id));
    }
}

fn blood(
    view: ViewQuery<(&ViewTarget, &BloodScreen, &BloodPipelineId, &DynamicUniformIndex<BloodUniform>)>,
    pipeline: Res<BloodPipeline>,
    cache: Res<PipelineCache>,
    images: Res<RenderAssets<GpuImage>>,
    uniforms: Res<ComponentUniforms<BloodUniform>>,
    mut ctx: RenderContext,
) {
    let (target, effect, id, index) = view.into_inner();
    let (Some(render_pipeline), Some(texture), Some(binding)) = (cache.get_render_pipeline(id.0), images.get(&effect.texture), uniforms.binding()) else {
        return;
    };
    let post = target.post_process_write();
    let bind_group = ctx.render_device().create_bind_group(
        None,
        &cache.get_bind_group_layout(&pipeline.layout),
        &BindGroupEntries::sequential((post.source, &pipeline.sampler, binding.clone(), &texture.texture_view, &pipeline.wrap)),
    );
    let mut pass = ctx.command_encoder().begin_render_pass(&RenderPassDescriptor {
        label: Some("blood"),
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blood_blends_in_and_out() {
        let w = BlendWindow {
            start: 3.0,
            end: 5.0,
            material: "PP_Player_BloodEffect_01".into(),
            textures: Default::default(),
            scalars: [("bloodContrast".to_string(), 1.7)].into(),
            blend_in: 0.25,
            blend_out: 0.5,
            min: 0.0,
            max: 1.0,
            hooks: vec![("bloodContrast".into(), true, true)],
        };
        assert!((w.scalar("bloodContrast", 0.125) - 0.5).abs() < 1e-5);
        assert_eq!(w.scalar("bloodContrast", 1.0), 1.0);
        assert!((w.scalar("bloodContrast", 1.75) - 0.5).abs() < 1e-5);
        // A parameter without a hook keeps the material's.
        assert_eq!(w.scalar("other", 1.0), 0.0);
    }
}
