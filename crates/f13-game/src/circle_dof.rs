//! The cine cameras' Circle DOF (F13-decomp MODLOG "The cine cameras' Circle DOF"): UCineCameraComponent's constructor
//! (0x141070f50) sets its post-process DepthOfFieldMethod to DOFM_CircleDOF, and its view (FUN_141072040) the f-stop
//! (CurrentAperture), the focal distance (CurrentFocusDistance: ManualFocusDistance) and the sensor width (the
//! filmback's). The post-process chain (FUN_140c54a50) adds the Circle DOF (FUN_140c4ca00) when r.DepthOfFieldQuality
//! (2) is above 0: Setup<1> (r.DepthOfField.FarBlur 1; half size), its temporal AA only with the view's temporal AA
//! (the game has none: r.DefaultFeature.AntiAliasing 0), Dilate<0>, CircleDOFPS0 (quality < 3) and RecombinePS0
//! (quality < 3), before the bloom. The view uniform's CircleDOFParams (CircleDofHalfCoc, FUN_140c4e4a0): f = sensor
//! width x 0.5 x Proj[0][0], x = f^2 / ((focal distance x 10 - f) x f-stop) x view width / sensor width x 0.25, y = 1e-5
//! / DepthBlurAmount (1), z = view width x DepthBlurRadius (0) / 1920, w = view width / 1920. The passes' random offset
//! (FUN_140c64a30): the Halton base-2 radical inverse of the view's FrameIndex & 1023.

use bevy::asset::{embedded_asset, load_embedded_asset};
use bevy::core_pipeline::schedule::{Core3d, Core3dSystems};
use bevy::core_pipeline::FullscreenShader;
use bevy::prelude::*;
use bevy::render::camera::ExtractedCamera;
use bevy::render::extract_component::{ExtractComponent, ExtractComponentPlugin};
use bevy::render::render_resource::binding_types::{texture_2d, texture_depth_2d, texture_depth_2d_multisampled, uniform_buffer_sized};
use bevy::render::render_resource::*;
use bevy::render::renderer::{RenderContext, RenderDevice, ViewQuery};
use bevy::render::sync_component::SyncComponent;
use bevy::render::texture::{CachedTexture, TextureCache};
use bevy::render::view::{Msaa, ViewDepthTexture, ViewTarget};
use bevy::render::{GpuResourceAppExt, Render, RenderApp, RenderStartup, RenderSystems};
use bevy::shader::{Shader, ShaderDefVal};

/// The half-size textures' format (the engine's PF_FloatRGBA).
const FORMAT: TextureFormat = TextureFormat::Rgba16Float;

pub struct CircleDofPlugin;

impl Plugin for CircleDofPlugin {
    fn build(&self, app: &mut App) {
        embedded_asset!(app, "shaders/circle_dof.wgsl");
        app.add_plugins(ExtractComponentPlugin::<CircleDof>::default()).add_systems(PostUpdate, depth_binding);
        let Some(render_app) = app.get_sub_app_mut(RenderApp) else { return };
        render_app
            .init_gpu_resource::<SpecializedRenderPipelines<CircleDofPipelines>>()
            .add_systems(RenderStartup, init_pipelines)
            .add_systems(Render, prepare_textures.in_set(RenderSystems::PrepareResources))
            .add_systems(Core3d, circle_dof.before(crate::bloom::game_bloom).in_set(Core3dSystems::PostProcess));
    }
}

/// A camera's Circle DOF: the focal distance (cm), the f-stop, the sensor width (mm), the horizontal field of view
/// (radians), the near plane (m) and the frame index.
#[derive(Component, Clone)]
pub struct CircleDof {
    pub focal_distance: f32,
    pub fstop: f32,
    pub sensor_width: f32,
    pub hfov: f32,
    pub near: f32,
    pub frame: u32,
}

impl SyncComponent for CircleDof {
    type Target = CircleDof;
}

impl ExtractComponent for CircleDof {
    type QueryData = &'static CircleDof;
    type QueryFilter = ();
    type Out = CircleDof;

    fn extract_component(d: bevy::ecs::query::QueryItem<'_, '_, Self::QueryData>) -> Option<Self::Out> {
        Some(d.clone())
    }
}

/// The passes read the view's depth.
fn depth_binding(mut cameras: Query<&mut Camera3d, With<CircleDof>>) {
    for mut c in &mut cameras {
        let usage = TextureUsages::from(c.depth_texture_usages);
        if !usage.contains(TextureUsages::TEXTURE_BINDING) {
            c.depth_texture_usages = (usage | TextureUsages::TEXTURE_BINDING).into();
        }
    }
}

/// The Halton sequence's radical inverse in a base.
fn halton(mut i: u32, base: u32) -> f32 {
    let (mut f, mut r) = (1.0 / base as f32, 0.0);
    while i > 0 {
        r += (i % base) as f32 * f;
        i /= base;
        f /= base as f32;
    }
    r
}

#[derive(Clone, Copy, ShaderType)]
struct CircleDofUniform {
    coc: Vec4,
    focus: Vec4,
}

#[derive(Resource)]
pub(crate) struct CircleDofPipelines {
    layout: BindGroupLayoutDescriptor,
    layout_ms: BindGroupLayoutDescriptor,
    fullscreen_shader: FullscreenShader,
    shader: Handle<Shader>,
}

fn init_pipelines(mut commands: Commands, fullscreen_shader: Res<FullscreenShader>, asset_server: Res<AssetServer>) {
    let layout = |label: &'static str, ms: bool| {
        let depth = if ms { texture_depth_2d_multisampled() } else { texture_depth_2d() };
        BindGroupLayoutDescriptor::new(
            label,
            &BindGroupLayoutEntries::sequential(
                ShaderStages::FRAGMENT,
                (
                    texture_2d(TextureSampleType::Float { filterable: true }),
                    texture_2d(TextureSampleType::Float { filterable: true }),
                    uniform_buffer_sized(false, Some(CircleDofUniform::min_size())),
                    depth,
                ),
            ),
        )
    };
    commands.insert_resource(CircleDofPipelines {
        layout: layout("circle_dof_layout", false),
        layout_ms: layout("circle_dof_layout_ms", true),
        fullscreen_shader: fullscreen_shader.clone(),
        shader: load_embedded_asset!(asset_server.as_ref(), "shaders/circle_dof.wgsl"),
    });
}

impl SpecializedRenderPipeline for CircleDofPipelines {
    /// The pass's entry point, the target's format, and whether the depth is multisampled.
    type Key = (&'static str, TextureFormat, bool);

    fn specialize(&self, (entry, format, ms): Self::Key) -> RenderPipelineDescriptor {
        let shader_defs: Vec<ShaderDefVal> = if ms { vec!["MULTISAMPLED".into()] } else { vec![] };
        RenderPipelineDescriptor {
            label: Some(format!("circle_dof_{entry}").into()),
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

/// A view's half-size textures: the setup's, the dilated CoC and the DOF's.
#[derive(Component)]
pub struct CircleDofTextures {
    setup: CachedTexture,
    dilated: CachedTexture,
    dof: CachedTexture,
}

fn prepare_textures(mut commands: Commands, mut cache: ResMut<TextureCache>, device: Res<RenderDevice>, views: Query<(Entity, &ExtractedCamera), With<CircleDof>>) {
    for (e, camera) in &views {
        let Some(size) = camera.physical_viewport_size else { continue };
        // DivideAndRoundUp by 2.
        let half = (size + 1) / 2;
        let mut tex = |label: &'static str| {
            cache.get(
                &device,
                TextureDescriptor {
                    label: Some(label),
                    size: Extent3d { width: half.x.max(1), height: half.y.max(1), depth_or_array_layers: 1 },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: TextureDimension::D2,
                    format: FORMAT,
                    usage: TextureUsages::RENDER_ATTACHMENT | TextureUsages::TEXTURE_BINDING,
                    view_formats: &[],
                },
            )
        };
        commands.entity(e).insert(CircleDofTextures { setup: tex("circle_dof_setup"), dilated: tex("circle_dof_dilated"), dof: tex("circle_dof") });
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn circle_dof(
    view: ViewQuery<(&ViewTarget, &ExtractedCamera, &ViewDepthTexture, &Msaa, &CircleDof, &CircleDofTextures)>,
    pipelines: Res<CircleDofPipelines>,
    mut specialized: ResMut<SpecializedRenderPipelines<CircleDofPipelines>>,
    cache: Res<PipelineCache>,
    mut ctx: RenderContext,
) {
    let (target, camera, depth, msaa, dof, textures) = view.into_inner();
    let Some(view_size) = camera.physical_viewport_size else { return };
    let ms = msaa.samples() > 1;
    let ids = [("setup", FORMAT), ("dilate", FORMAT), ("gather", FORMAT), ("recombine", target.main_texture_format())].map(|(entry, format)| specialized.specialize(&cache, &pipelines, (entry, format, ms)));
    let Some([setup, dilate, gather, recombine]) = ids.iter().map(|id| cache.get_render_pipeline(*id)).collect::<Option<Vec<_>>>().and_then(|v| <[&RenderPipeline; 4]>::try_from(v).ok()) else {
        return;
    };
    // CircleDofHalfCoc.
    let width = view_size.x as f32;
    let f = dof.sensor_width * 0.5 / (dof.hfov * 0.5).tan();
    let x = f * f / ((dof.focal_distance * 10.0 - f) * dof.fstop) * width / dof.sensor_width * 0.25;
    let uniform = CircleDofUniform {
        coc: Vec4::new(x, 9.999999747378752e-06 / 1.0, width * 0.0 / 1920.0, width * 0.0005208333333333333),
        focus: Vec4::new(dof.focal_distance, halton(dof.frame & 0x3ff, 2), dof.near, 0.0),
    };
    let mut buffer = encase::UniformBuffer::new(Vec::<u8>::new());
    buffer.write(&uniform).expect("circle dof uniform");
    let device = ctx.render_device().clone();
    let gpu = device.create_buffer_with_data(&BufferInitDescriptor { label: Some("circle_dof"), contents: buffer.as_ref(), usage: BufferUsages::UNIFORM });
    let layout = cache.get_bind_group_layout(if ms { &pipelines.layout_ms } else { &pipelines.layout });
    let draw = |ctx: &mut RenderContext, pipeline: &RenderPipeline, a: &TextureView, b: &TextureView, out: &TextureView| {
        let bind_group = device.create_bind_group(None, &layout, &BindGroupEntries::sequential((a, b, gpu.as_entire_binding(), depth.view())));
        let mut pass = ctx.command_encoder().begin_render_pass(&RenderPassDescriptor {
            label: Some("circle_dof"),
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
    let scene = target.main_texture_view();
    draw(&mut ctx, setup, scene, scene, &textures.setup.default_view);
    draw(&mut ctx, dilate, &textures.setup.default_view, &textures.setup.default_view, &textures.dilated.default_view);
    draw(&mut ctx, gather, &textures.setup.default_view, &textures.dilated.default_view, &textures.dof.default_view);
    let post = target.post_process_write();
    draw(&mut ctx, recombine, post.source, &textures.dof.default_view, post.destination);
}
