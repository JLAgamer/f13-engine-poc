//! The game's Gaussian depth of field, as the stun and Sense set it on the played Jason's ForcedPostProcessVolume
//! (MODLOG "Fringe"): OnStunned (0x1403f6070) and Sense (FUN_14040a230) override DepthOfFieldMethod (Gaussian),
//! FocalDistance (2000 / 1 cm), Near/FarTransitionRegion (0, 1000 / 10000), Near/FarBlurSize (0, 1 / 2.5) and Sense
//! also FocalRegion (7000); the stun's end (0x1403f66a0) and Sense's (FUN_1403ea820) clear those overrides. The
//! level's unbound volume sets no depth of field, so outside them there is none, and the stun's FocalRegion is the
//! default 0.
//!
//! AddPostProcessDepthOfFieldGaussian (FUN_140c49ba0, from FUN_140c54e82): the far blur when min(FarBlurSize,
//! r.DepthOfField.MaxSize) reaches 0.01, the near one when NearBlurSize reaches its threshold (never here):
//! - DOFSetup (FPostProcessDOFSetupPS<1, 0>): half size; each full-size pixel's colour weighted by
//!   saturate((SceneDepth - (FocalDistance + FocalRegion)) / max(FarTransitionRegion, 0.01)) (the view uniforms'
//!   DOF values, FUN_140db69cc), less where the view looks up past the sky distance (DepthOfFieldParams[0].x:
//!   SkyFocusDistance, else 1e8; ComputeDepthOfFieldParams 0x140c4e720);
//! - FarDOFBlurX / Y: RenderGaussianBlur of FarBlurSize, white (bloom.rs's);
//! - DOFRecombine (FPostProcessDOFRecombinePS<1, 0, 0>): full size, over the scene.
//! Before the bloom and the tonemapper, as the engine runs it.

use bevy::asset::{embedded_asset, load_embedded_asset};
use bevy::core_pipeline::schedule::{Core3d, Core3dSystems};
use bevy::core_pipeline::FullscreenShader;
use bevy::prelude::*;
use bevy::render::camera::ExtractedCamera;
use bevy::render::extract_component::{ExtractComponent, ExtractComponentPlugin};
use bevy::render::render_resource::binding_types::{sampler, texture_2d, texture_depth_2d, texture_depth_2d_multisampled, uniform_buffer_sized};
use bevy::render::render_resource::*;
use bevy::render::renderer::{RenderContext, RenderDevice, ViewQuery};
use bevy::render::sync_component::SyncComponent;
use bevy::render::texture::{CachedTexture, TextureCache};
use bevy::render::view::{Msaa, ViewDepthTexture, ViewTarget};
use bevy::render::{GpuResourceAppExt, Render, RenderApp, RenderStartup, RenderSystems};
use bevy::shader::{Shader, ShaderDefVal};

use crate::bloom::{BloomPipelines, Pass};

/// The half-size textures' format (the engine's PF_FloatRGBA: the weights in alpha).
const FORMAT: TextureFormat = TextureFormat::Rgba16Float;
/// DepthOfFieldParams[0].x without a SkyFocusDistance.
const NO_SKY_FOCUS: f32 = 100000000.0;

pub struct DofPlugin;

impl Plugin for DofPlugin {
    fn build(&self, app: &mut App) {
        embedded_asset!(app, "shaders/dof.wgsl");
        app.add_plugins(ExtractComponentPlugin::<GaussianDof>::default()).add_systems(Update, killer_dof);
        let Some(render_app) = app.get_sub_app_mut(RenderApp) else { return };
        render_app
            .init_gpu_resource::<SpecializedRenderPipelines<DofPipelines>>()
            .add_systems(RenderStartup, init_pipelines)
            .add_systems(Render, prepare_textures.in_set(RenderSystems::PrepareResources))
            .add_systems(Core3d, game_dof.before(crate::bloom::game_bloom).in_set(Core3dSystems::PostProcess));
    }
}

/// A camera's Gaussian depth of field: the focus's end and far transition (cm), FarBlurSize, and the view (for the
/// sky's rays).
#[derive(Component, Clone)]
pub struct GaussianDof {
    pub focus_end: f32,
    pub far_transition: f32,
    pub far_size: f32,
    pub world_from_clip: Mat4,
    pub camera: Vec3,
    pub near: f32,
}

impl SyncComponent for GaussianDof {
    type Target = GaussianDof;
}

impl ExtractComponent for GaussianDof {
    type QueryData = &'static GaussianDof;
    type QueryFilter = ();
    type Out = GaussianDof;

    fn extract_component(d: bevy::ecs::query::QueryItem<'_, '_, Self::QueryData>) -> Option<Self::Out> {
        Some(d.clone())
    }
}

/// The overrides in force: (FocalDistance, FocalRegion, FarTransitionRegion, FarBlurSize).
type Overrides = (f32, f32, f32, f32);
const STUN: Overrides = (2000.0, 0.0, 1000.0, 1.0);
const SENSE: Overrides = (1.0, 7000.0, 10000.0, 2.5);

/// The played Jason's camera: the stun's or Sense's depth of field, whichever set it last, until either ends.
fn killer_dof(
    mut commands: Commands,
    time: Res<Time>,
    play: Option<Res<crate::jason_player::PlayJason>>,
    jasons: Query<(&crate::jason::Jason, Option<&crate::senses::Knowledge>)>,
    cameras: Query<(Entity, &GlobalTransform, &Projection, Has<GaussianDof>), With<crate::player::PlayerCamera>>,
    mut state: Local<(bool, bool, Option<Overrides>)>,
) {
    let Some((j, know)) = jasons.iter().next().filter(|_| play.is_some()) else { return };
    let (was_stunned, was_sensing, overrides) = &mut *state;
    let stunned = j.stunned();
    let sensing = know.is_some_and(|k| k.sensing(time.elapsed_secs()));
    if (*was_stunned && !stunned) || (*was_sensing && !sensing) {
        *overrides = None;
    }
    if stunned && !*was_stunned {
        *overrides = Some(STUN);
    }
    if sensing && !*was_sensing {
        *overrides = Some(SENSE);
    }
    (*was_stunned, *was_sensing) = (stunned, sensing);
    for (e, xf, projection, has) in &cameras {
        match *overrides {
            // min(FarBlurSize, r.DepthOfField.MaxSize 100) >= 0.01.
            Some((focal, region, far, size)) if size.min(100.0) >= 0.01 => {
                let world_from_clip = xf.to_matrix() * projection.get_clip_from_view().inverse();
                let near = match projection {
                    Projection::Perspective(p) => p.near,
                    _ => 0.1,
                };
                commands.entity(e).insert(GaussianDof {
                    focus_end: focal + region,
                    far_transition: far.max(0.009999999776482582),
                    far_size: size.min(100.0),
                    world_from_clip,
                    camera: xf.translation(),
                    near,
                });
            }
            _ if has => {
                commands.entity(e).remove::<GaussianDof>();
            }
            _ => {}
        }
    }
}

#[derive(Clone, Copy, ShaderType)]
struct DofUniform {
    world_from_clip: Mat4,
    camera: Vec4,
    focus: Vec4,
}

#[derive(Resource)]
pub(crate) struct DofPipelines {
    layout: BindGroupLayoutDescriptor,
    layout_ms: BindGroupLayoutDescriptor,
    sampler: Sampler,
    fullscreen_shader: FullscreenShader,
    shader: Handle<Shader>,
}

fn init_pipelines(mut commands: Commands, device: Res<RenderDevice>, fullscreen_shader: Res<FullscreenShader>, asset_server: Res<AssetServer>) {
    let layout = |label: &'static str, ms: bool| {
        let depth = if ms { texture_depth_2d_multisampled() } else { texture_depth_2d() };
        BindGroupLayoutDescriptor::new(
            label,
            &BindGroupLayoutEntries::sequential(
                ShaderStages::FRAGMENT,
                (
                    texture_2d(TextureSampleType::Float { filterable: true }),
                    texture_2d(TextureSampleType::Float { filterable: true }),
                    sampler(SamplerBindingType::Filtering),
                    uniform_buffer_sized(false, Some(DofUniform::min_size())),
                    depth,
                ),
            ),
        )
    };
    let sampler = device.create_sampler(&SamplerDescriptor {
        address_mode_u: AddressMode::ClampToEdge,
        address_mode_v: AddressMode::ClampToEdge,
        mag_filter: FilterMode::Linear,
        min_filter: FilterMode::Linear,
        ..default()
    });
    commands.insert_resource(DofPipelines {
        layout: layout("game_dof_layout", false),
        layout_ms: layout("game_dof_layout_ms", true),
        sampler,
        fullscreen_shader: fullscreen_shader.clone(),
        shader: load_embedded_asset!(asset_server.as_ref(), "shaders/dof.wgsl"),
    });
}

impl SpecializedRenderPipeline for DofPipelines {
    /// Recombine (else setup), the target's format, and whether the depth is multisampled.
    type Key = (bool, TextureFormat, bool);

    fn specialize(&self, (recombine, format, ms): Self::Key) -> RenderPipelineDescriptor {
        let shader_defs: Vec<ShaderDefVal> = if ms { vec!["MULTISAMPLED".into()] } else { vec![] };
        let entry = if recombine { "recombine" } else { "setup" };
        RenderPipelineDescriptor {
            label: Some(format!("game_dof_{entry}").into()),
            layout: vec![if ms { self.layout_ms.clone() } else { self.layout.clone() }],
            vertex: self.fullscreen_shader.to_vertex_state(),
            fragment: Some(FragmentState {
                shader: self.shader.clone(),
                shader_defs,
                entry_point: Some(entry.into()),
                targets: vec![Some(ColorTargetState { format, blend: None, write_mask: ColorWrites::ALL })],
                ..default()
            }),
            ..default()
        }
    }
}

/// A view's half-size textures: the setup's, the blur's X passes (full and half width), its result, and a black one.
#[derive(Component)]
pub struct DofTextures {
    setup: CachedTexture,
    x: CachedTexture,
    x_half: CachedTexture,
    blurred: CachedTexture,
    black: CachedTexture,
}

fn prepare_textures(mut commands: Commands, mut cache: ResMut<TextureCache>, device: Res<RenderDevice>, views: Query<(Entity, &ExtractedCamera), With<GaussianDof>>) {
    for (e, camera) in &views {
        let Some(size) = camera.physical_viewport_size else { continue };
        let mut tex = |label: &'static str, s: UVec2| {
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
        // DivideAndRoundUp by 2.
        let half = (size + 1) / 2;
        commands.entity(e).insert(DofTextures {
            setup: tex("dof_setup", half),
            x: tex("dof_x", half),
            x_half: tex("dof_x_half", UVec2::new(half.x.div_ceil(2), half.y)),
            blurred: tex("dof_blurred", half),
            black: tex("dof_black", UVec2::ONE),
        });
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn game_dof(
    view: ViewQuery<(&ViewTarget, &ExtractedCamera, &ViewDepthTexture, &Msaa, &GaussianDof, &DofTextures)>,
    pipelines: Res<DofPipelines>,
    mut specialized: ResMut<SpecializedRenderPipelines<DofPipelines>>,
    bloom: Res<BloomPipelines>,
    mut bloom_specialized: ResMut<SpecializedRenderPipelines<BloomPipelines>>,
    cache: Res<PipelineCache>,
    mut ctx: RenderContext,
) {
    let (target, camera, depth, msaa, dof, textures) = view.into_inner();
    let Some(view_size) = camera.physical_viewport_size else { return };
    let ms = msaa.samples() > 1;
    let setup_id = specialized.specialize(&cache, &pipelines, (false, FORMAT, ms));
    let recombine_id = specialized.specialize(&cache, &pipelines, (true, target.main_texture_format(), ms));
    let filter_id = bloom_specialized.specialize(&cache, &bloom, (Pass::Filter, FORMAT));
    let (Some(setup), Some(recombine), Some(filter)) = (cache.get_render_pipeline(setup_id), cache.get_render_pipeline(recombine_id), cache.get_render_pipeline(filter_id)) else {
        return;
    };
    let uniform = DofUniform {
        world_from_clip: dof.world_from_clip,
        camera: dof.camera.extend(dof.near),
        focus: Vec4::new(dof.focus_end, dof.far_transition, NO_SKY_FOCUS, 0.0),
    };
    let mut buffer = encase::UniformBuffer::new(Vec::<u8>::new());
    buffer.write(&uniform).expect("dof uniform");
    let device = ctx.render_device().clone();
    let gpu = device.create_buffer_with_data(&BufferInitDescriptor { label: Some("game_dof"), contents: buffer.as_ref(), usage: BufferUsages::UNIFORM });
    let layout = cache.get_bind_group_layout(if ms { &pipelines.layout_ms } else { &pipelines.layout });
    let draw = |ctx: &mut RenderContext, pipeline: &RenderPipeline, scene: &TextureView, far: &TextureView, out: &TextureView| {
        let bind_group = device.create_bind_group(None, &layout, &BindGroupEntries::sequential((scene, far, &pipelines.sampler, gpu.as_entire_binding(), depth.view())));
        let mut pass = ctx.command_encoder().begin_render_pass(&RenderPassDescriptor {
            label: Some("game_dof"),
            color_attachments: &[Some(RenderPassColorAttachment { view: out, depth_slice: None, resolve_target: None, ops: Operations::default() })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_pipeline(pipeline);
        pass.set_bind_group(0, &bind_group, &[]);
        pass.draw(0..3, 0..1);
    };
    // DOFSetup.
    draw(&mut ctx, setup, target.main_texture_view(), &textures.black.default_view, &textures.setup.default_view);
    // FarDOFBlurX / Y.
    let blur = crate::bloom::Blur { x: &textures.x, x_half: &textures.x_half, out: &textures.blurred, black: &textures.black };
    crate::bloom::gaussian_blur(&mut ctx, &cache, &bloom, filter, view_size, &textures.setup, blur, dof.far_size, Vec4::ONE, None);
    // DOFRecombine.
    let post = target.post_process_write();
    draw(&mut ctx, recombine, post.source, &textures.blurred.default_view, post.destination);
}
