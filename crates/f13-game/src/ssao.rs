//! The engine's screen-space ambient occlusion (UE 4.18's, as this build runs it; decompiled, MODLOG "SSAO"), in place
//! of Bevy's own: its result is written into Bevy's SSAO texture, which the materials' shaders multiply into their
//! ambient light (the game's ambient cubemap pass multiplies its diffuse and specular by the AO and the material's own:
//! FPostProcessAmbientPS).
//!
//! Levels (FUN_140a733e0): r.AmbientOcclusionLevels is -1, so 1 + (q > 35) + (q > 70) with q = min(the volume's
//! AmbientOcclusionQuality 50, r.AmbientOcclusionMaxQuality 100): 2. The shaders' sample set (FUN_140a7ff30): 2. So:
//! - the HZB (BuildHZB 0x140d4bd90; THZBBuildPS<0> / <1>): R16F device Z, RoundUpToPowerOfTwo(view) / 2 in size, each
//!   texel the smallest (reversed Z: farthest) of four, mip by mip;
//! - the setup (FPostProcessAmbientOcclusionSetupPS<1>): half size, PF_FloatRGBA;
//! - the half-size occlusion (PS<1, 0, 2>), B8G8R8A8;
//! - the full-size occlusion (PS<0, 1, 2>) upsampling it, into the scene's ScreenSpaceAO.
//! The shaders (`shaders/ssao.wgsl`) were checked against their assembly (interpreted) on random inputs.
//!
//! The settings: the level's unbound post-process volume sets none of the ambient occlusion ones, so FPostProcessSettings'
//! defaults (intensity 0.5, static fraction 1, radius 200, not in world space, fade distance 8000 and radius 5000,
//! power 2, bias 3, quality 50, mip blend 0.6, mip scale 1.7, mip threshold 0.01); r.AmbientOcclusionMipLevelFactor
//! 0.4 (Epic scalability). Not ported: hair pixels' normal (PS<0, 1, 2> zeroes it for the hair shading model: no
//! shading model here). PLACEHOLDER: the temporal offset ((frame & 7) x 2.48 / 64, x 7.52 / 64 in the game) is
//! kept at frame 0: whether the view state's frame index advances without temporal AA (the game's default is none) wasn't
//! read.

use bevy::asset::{embedded_asset, load_embedded_asset};
use bevy::core_pipeline::prepass::{DepthPrepass, NormalPrepass, ViewPrepassTextures};
use bevy::core_pipeline::schedule::{Core3d, Core3dSystems};
use bevy::core_pipeline::FullscreenShader;
use bevy::pbr::{ScreenSpaceAmbientOcclusion, ScreenSpaceAmbientOcclusionResources};
use bevy::prelude::*;
use bevy::render::camera::ExtractedCamera;
use bevy::render::extract_component::{ExtractComponent, ExtractComponentPlugin};
use bevy::render::render_resource::binding_types::{sampler, texture_2d, texture_depth_2d, texture_storage_2d, uniform_buffer_sized};
use bevy::render::render_resource::*;
use bevy::render::renderer::{RenderContext, RenderDevice, RenderQueue, ViewQuery};
use bevy::render::sync_component::SyncComponent;
use bevy::render::texture::{CachedTexture, TextureCache};
use bevy::render::view::Msaa;
use bevy::render::{GpuResourceAppExt, Render, RenderApp, RenderStartup, RenderSystems};
use bevy::shader::{Shader, ShaderDefVal};

/// FPostProcessSettings' defaults (the level's volume overrides none).
const INTENSITY: f32 = 0.5;
const STATIC_FRACTION: f32 = 1.0;
const RADIUS: f32 = 200.0;
const FADE_DISTANCE: f32 = 8000.0;
const FADE_RADIUS: f32 = 5000.0;
const DISTANCE: f32 = 80.0;
const POWER: f32 = 2.0;
const BIAS: f32 = 3.0;
const MIP_BLEND: f32 = 0.6;
const MIP_SCALE: f32 = 1.7;
const MIP_THRESHOLD: f32 = 0.01;
/// r.AmbientOcclusionMipLevelFactor (BaseScalability's Epic post-process quality).
const MIP_LEVEL_FACTOR: f32 = 0.4;
/// The SSAORandomization texture's size (64 x 64, a 4 x 4 pattern repeated).
const RANDOM_SIZE: u32 = 64;

pub struct SsaoPlugin;

impl Plugin for SsaoPlugin {
    fn build(&self, app: &mut App) {
        embedded_asset!(app, "shaders/ssao.wgsl");
        app.add_plugins(ExtractComponentPlugin::<GameSsao>::default()).add_systems(PostUpdate, view_params.after(TransformSystems::Propagate));
        let Some(render_app) = app.get_sub_app_mut(RenderApp) else { return };
        render_app
            .init_gpu_resource::<SpecializedRenderPipelines<SsaoPipelines>>()
            .add_systems(RenderStartup, init_pipelines)
            .add_systems(Render, prepare_textures.in_set(RenderSystems::PrepareResources))
            // After Bevy's own (which runs before the main pass), before the opaque pass reads it.
            .add_systems(Core3d, game_ssao.in_set(Core3dSystems::MainPass).before(bevy::core_pipeline::core_3d::main_opaque_pass_3d));
    }
}

/// A camera's view for the occlusion: its rotation (world to view), its projection's [0][0] and aspect, and its near
/// plane (m).
#[derive(Component, Clone)]
pub struct GameSsao {
    view_from_world: Mat3,
    p00: f32,
    ratio: f32,
    near: f32,
}

impl SyncComponent for GameSsao {
    type Target = GameSsao;
}

impl ExtractComponent for GameSsao {
    type QueryData = &'static GameSsao;
    type QueryFilter = ();
    type Out = GameSsao;

    fn extract_component(s: bevy::ecs::query::QueryItem<'_, '_, Self::QueryData>) -> Option<Self::Out> {
        Some(s.clone())
    }
}

/// The player's camera: Bevy's SSAO (for its texture and the materials' use of it, with the prepasses it needs and no
/// MSAA: the game's default anti-aliasing is none, r.DefaultFeature.AntiAliasing 0), and our view values each frame.
fn view_params(mut commands: Commands, cameras: Query<(Entity, &GlobalTransform, &Projection, Has<ScreenSpaceAmbientOcclusion>), With<crate::player::PlayerCamera>>, unlit: Option<Res<crate::lighting::Unlit>>) {
    // F13_NO_SSAO: without it (comparisons).
    if unlit.is_some() || std::env::var_os("F13_NO_SSAO").is_some() {
        return;
    }
    for (e, xf, projection, has) in &cameras {
        let Projection::Perspective(p) = projection else { continue };
        if !has {
            commands.entity(e).insert((ScreenSpaceAmbientOcclusion::default(), DepthPrepass, NormalPrepass, Msaa::Off));
        }
        let clip = projection.get_clip_from_view();
        commands.entity(e).insert(GameSsao {
            view_from_world: Mat3::from_quat(xf.rotation().inverse()),
            p00: clip.x_axis.x,
            ratio: clip.y_axis.y / clip.x_axis.x,
            near: p.near,
        });
    }
}

#[derive(Clone, Copy, ShaderType, Default)]
struct Params {
    to_view: [Vec4; 3],
    devz: Vec4,
    ao: [Vec4; 5],
    hzb_remap: Vec4,
    texel: Vec4,
    setup: Vec4,
    size: Vec4,
    hzb_uv: Vec4,
    hzb_in: Vec4,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum Pass {
    HzbFirst,
    HzbMip,
    Setup,
    Half,
    Full,
}

#[derive(Resource)]
pub(crate) struct SsaoPipelines {
    layouts: [BindGroupLayoutDescriptor; 5],
    copy_layouts: [BindGroupLayoutDescriptor; 2],
    copy: [CachedComputePipelineId; 2],
    hzb_sampler: Sampler,
    random: TextureView,
    fullscreen_shader: FullscreenShader,
    shader: Handle<Shader>,
}

/// The SSAORandomization texture (generated at startup, 0x140de3d61): 16 rotations, Reorder's order, each the angle
/// w x pi / 16 with its length 1 - (sin(1.98 w) x 0.5 x 0.23 + 0.115), (cos, sin) quantised (x + 1) / 2 to bytes,
/// laid out 4 x 4 and repeated over 64 x 64.
fn random_texels() -> Vec<u8> {
    const REORDER: [i32; 16] = [0, 11, 7, 3, 10, 4, 15, 12, 6, 8, 1, 14, 13, 2, 9, 5];
    let bases: Vec<[u8; 2]> = REORDER
        .iter()
        .map(|&w| {
            let angle = w as f32 * 0.19634954631328583;
            let len = 1.0 - ((w as f32 * 1.9799998998641968).sin() * 0.5 * 0.22999998927116394 + 0.11499999463558197);
            let q = |v: f32| ((v * len + 1.0) * 0.5 * 255.99899291992188) as i32 as u8;
            [q(angle.cos()), q(angle.sin())]
        })
        .collect();
    let mut out = Vec::with_capacity((RANDOM_SIZE * RANDOM_SIZE * 2) as usize);
    for y in 0..RANDOM_SIZE {
        for x in 0..RANDOM_SIZE {
            out.extend_from_slice(&bases[((x % 4) + (y % 4) * 4) as usize]);
        }
    }
    out
}

fn init_pipelines(
    mut commands: Commands,
    device: Res<RenderDevice>,
    queue: Res<RenderQueue>,
    cache: Res<PipelineCache>,
    fullscreen_shader: Res<FullscreenShader>,
    asset_server: Res<AssetServer>,
) {
    let uniform = || uniform_buffer_sized(false, Some(Params::min_size()));
    let float = || texture_2d(TextureSampleType::Float { filterable: true });
    let fragment = ShaderStages::FRAGMENT;
    let layouts = [
        BindGroupLayoutDescriptor::new("game_ssao_hzb_first", &BindGroupLayoutEntries::sequential(fragment, (uniform(), texture_depth_2d()))),
        BindGroupLayoutDescriptor::new("game_ssao_hzb_mip", &BindGroupLayoutEntries::sequential(fragment, (uniform(), float()))),
        BindGroupLayoutDescriptor::new("game_ssao_setup", &BindGroupLayoutEntries::sequential(fragment, (uniform(), float(), texture_depth_2d()))),
        BindGroupLayoutDescriptor::new(
            "game_ssao_half",
            &BindGroupLayoutEntries::sequential(fragment, (uniform(), float(), float(), sampler(SamplerBindingType::NonFiltering), float())),
        ),
        BindGroupLayoutDescriptor::new(
            "game_ssao_full",
            &BindGroupLayoutEntries::sequential(
                fragment,
                (uniform(), float(), float(), sampler(SamplerBindingType::NonFiltering), float(), texture_depth_2d(), float(), float()),
            ),
        ),
    ];
    let copy_layout = |label: &'static str, format: TextureFormat| {
        BindGroupLayoutDescriptor::new(label, &BindGroupLayoutEntries::sequential(ShaderStages::COMPUTE, (float(), texture_storage_2d(format, StorageTextureAccess::WriteOnly))))
    };
    let copy_layouts = [copy_layout("game_ssao_copy16", TextureFormat::R16Float), copy_layout("game_ssao_copy32", TextureFormat::R32Float)];
    let shader: Handle<Shader> = load_embedded_asset!(asset_server.as_ref(), "shaders/ssao.wgsl");
    let copy = [false, true].map(|r32| {
        let mut defs: Vec<ShaderDefVal> = vec!["COPY".into()];
        if r32 {
            defs.push("R32".into());
        }
        cache.queue_compute_pipeline(ComputePipelineDescriptor {
            label: Some("game_ssao_copy".into()),
            layout: vec![copy_layouts[r32 as usize].clone()],
            shader: shader.clone(),
            shader_defs: defs,
            entry_point: Some("copy".into()),
            ..default()
        })
    });
    let hzb_sampler = device.create_sampler(&SamplerDescriptor {
        address_mode_u: AddressMode::ClampToEdge,
        address_mode_v: AddressMode::ClampToEdge,
        mag_filter: FilterMode::Nearest,
        min_filter: FilterMode::Nearest,
        mipmap_filter: MipmapFilterMode::Nearest,
        ..default()
    });
    let random = device.create_texture_with_data(
        &queue,
        &TextureDescriptor {
            label: Some("game_ssao_random"),
            size: Extent3d { width: RANDOM_SIZE, height: RANDOM_SIZE, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: TextureDimension::D2,
            format: TextureFormat::Rg8Unorm,
            usage: TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        },
        TextureDataOrder::default(),
        &random_texels(),
    );
    commands.insert_resource(SsaoPipelines {
        layouts,
        copy_layouts,
        copy,
        hzb_sampler,
        random: random.create_view(&TextureViewDescriptor::default()),
        fullscreen_shader: fullscreen_shader.clone(),
        shader,
    });
}

impl SpecializedRenderPipeline for SsaoPipelines {
    type Key = Pass;

    fn specialize(&self, pass: Pass) -> RenderPipelineDescriptor {
        let (def, entry, layout, format) = match pass {
            Pass::HzbFirst => ("HZB_FIRST", "hzb", 0, TextureFormat::R16Float),
            Pass::HzbMip => ("HZB_MIP", "hzb", 1, TextureFormat::R16Float),
            Pass::Setup => ("SETUP", "setup", 2, TextureFormat::Rgba16Float),
            Pass::Half => ("HALF", "occlusion_half", 3, TextureFormat::Rgba8Unorm),
            Pass::Full => ("FULL", "occlusion_full", 4, TextureFormat::R8Unorm),
        };
        let mut defs: Vec<ShaderDefVal> = vec![def.into()];
        if matches!(pass, Pass::Half | Pass::Full) {
            defs.push("OCCLUSION".into());
        }
        RenderPipelineDescriptor {
            label: Some(format!("game_ssao_{entry}").into()),
            layout: vec![self.layouts[layout].clone()],
            vertex: self.fullscreen_shader.to_vertex_state(),
            fragment: Some(FragmentState {
                shader: self.shader.clone(),
                shader_defs: defs,
                entry_point: Some(entry.into()),
                targets: vec![Some(ColorTargetState { format, blend: None, write_mask: ColorWrites::ALL })],
                ..default()
            }),
            ..default()
        }
    }
}

/// A view's textures: the HZB (with its size and mip count), the setup, the half-size and full-size occlusion.
#[derive(Component)]
pub struct SsaoTextures {
    hzb: CachedTexture,
    hzb_size: UVec2,
    hzb_mips: u32,
    setup: CachedTexture,
    half: CachedTexture,
    full: CachedTexture,
}

/// BuildHZB's size: per axis 1 << max(1, ceil(log2(view)) - 1), the mips the larger one's.
fn hzb_size(view: UVec2) -> (UVec2, u32) {
    let mips = |n: u32| (n.max(1).next_power_of_two().trailing_zeros() as i32 - 1).max(1) as u32;
    let (x, y) = (mips(view.x), mips(view.y));
    (UVec2::new(1 << x, 1 << y), x.max(y))
}

fn prepare_textures(mut commands: Commands, mut cache: ResMut<TextureCache>, device: Res<RenderDevice>, views: Query<(Entity, &ExtractedCamera), With<GameSsao>>) {
    for (e, camera) in &views {
        let Some(size) = camera.physical_viewport_size else { continue };
        let mut tex = |label: &'static str, s: UVec2, format: TextureFormat, mips: u32| {
            cache.get(
                &device,
                TextureDescriptor {
                    label: Some(label),
                    size: Extent3d { width: s.x.max(1), height: s.y.max(1), depth_or_array_layers: 1 },
                    mip_level_count: mips,
                    sample_count: 1,
                    dimension: TextureDimension::D2,
                    format,
                    usage: TextureUsages::RENDER_ATTACHMENT | TextureUsages::TEXTURE_BINDING,
                    view_formats: &[],
                },
            )
        };
        let (hzb_size, hzb_mips) = hzb_size(size);
        let half = (size + 1) / 2;
        commands.entity(e).insert(SsaoTextures {
            hzb: tex("game_ssao_hzb", hzb_size, TextureFormat::R16Float, hzb_mips),
            hzb_size,
            hzb_mips,
            setup: tex("game_ssao_setup", half, TextureFormat::Rgba16Float, 1),
            half: tex("game_ssao_half", half, TextureFormat::Rgba8Unorm, 1),
            full: tex("game_ssao_full", size, TextureFormat::R8Unorm, 1),
        });
    }
}

/// The values one pass's shader reads: the view's, then ScreenSpaceAOParams for an input `input` texels in size at
/// `scale` (BufferSize / input, integer) from the full size (FUN_140a5c790), HZBRemapping (FUN_140aa85c0).
fn params(s: &GameSsao, view: UVec2, hzb: UVec2, input: UVec2, scale: u32) -> Params {
    let v = s.view_from_world;
    // The engine's view space: z forward (Bevy's looks along -z).
    let row = |c: Vec3| Vec4::new(c.x, c.y, -c.z, 0.0);
    let x = scale as f32;
    let radius = RADIUS * 0.0024999999441206455 * MIP_SCALE.powf(x.log2()) * 0.25;
    let fade = FADE_RADIUS.max(1.0);
    let (w, h) = (view.x as f32, view.y as f32);
    let (hx, hy) = (w / (hzb.x as f32 * 2.0) * 0.5, h / (hzb.y as f32 * 2.0) * 0.5);
    Params {
        to_view: [row(v.x_axis), row(v.y_axis), row(v.z_axis)],
        devz: Vec4::new(0.0, 0.0, 1.0 / (s.near * 100.0), 0.0),
        ao: [
            Vec4::new(POWER, BIAS * 0.0010000000474974513, 1.0 / DISTANCE, INTENSITY),
            Vec4::new(input.x as f32 / RANDOM_SIZE as f32, input.y as f32 / RANDOM_SIZE as f32, radius, s.ratio),
            Vec4::new(x, MIP_THRESHOLD / x, 0.0, MIP_BLEND),
            Vec4::new(0.0, 0.0, STATIC_FRACTION.clamp(0.0, 1.0), s.p00),
            Vec4::new(1.0 / fade, -(FADE_DISTANCE - fade) / fade, MIP_LEVEL_FACTOR.clamp(0.0, 100.0), 0.0),
        ],
        hzb_remap: Vec4::new(hx, -hy, hx, hy),
        ..default()
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn game_ssao(
    view: ViewQuery<(&ExtractedCamera, &GameSsao, &SsaoTextures, &ViewPrepassTextures, &ScreenSpaceAmbientOcclusionResources)>,
    pipelines: Res<SsaoPipelines>,
    mut specialized: ResMut<SpecializedRenderPipelines<SsaoPipelines>>,
    cache: Res<PipelineCache>,
    mut ctx: RenderContext,
) {
    let (camera, ssao, textures, prepass, bevy_ssao) = view.into_inner();
    let Some(size) = camera.physical_viewport_size else { return };
    let (Some(depth), Some(normals)) = (prepass.depth_view(), prepass.normal_view()) else { return };
    let ids = [Pass::HzbFirst, Pass::HzbMip, Pass::Setup, Pass::Half, Pass::Full].map(|p| specialized.specialize(&cache, &pipelines, p));
    let Some(pipes) = ids.iter().map(|id| cache.get_render_pipeline(*id)).collect::<Option<Vec<_>>>() else { return };
    let target_format = bevy_ssao.screen_space_ambient_occlusion_texture.texture.format();
    let r32 = target_format == TextureFormat::R32Float;
    let Some(copy) = cache.get_compute_pipeline(pipelines.copy[r32 as usize]) else { return };
    let device = ctx.render_device().clone();
    let buffer = |p: &Params| {
        let mut b = encase::UniformBuffer::new(Vec::<u8>::new());
        b.write(p).expect("ssao uniform");
        device.create_buffer_with_data(&BufferInitDescriptor { label: Some("game_ssao"), contents: b.as_ref(), usage: BufferUsages::UNIFORM })
    };
    let layout = |i: usize| cache.get_bind_group_layout(&pipelines.layouts[i]);
    let draw = |ctx: &mut RenderContext, pipeline: &RenderPipeline, group: &BindGroup, out: &TextureView| {
        let mut pass = ctx.command_encoder().begin_render_pass(&RenderPassDescriptor {
            label: Some("game_ssao"),
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
    let half = (size + 1) / 2;
    let base = params(ssao, size, textures.hzb_size, size, 1);
    // The HZB: mip 0 from the depth (the view's buffer is the depth's size), then each from the last.
    let mip_view = |m: u32| {
        textures.hzb.texture.create_view(&TextureViewDescriptor { base_mip_level: m, mip_level_count: Some(1), ..default() })
    };
    {
        let (fw, fh) = (size.x as f32, size.y as f32);
        let p = Params {
            hzb_uv: Vec4::new(textures.hzb_size.x as f32 * 2.0 / fw, textures.hzb_size.y as f32 * 2.0 / fh, 0.0, 0.0),
            hzb_in: Vec4::new(1.0 / fw, 1.0 / fh, (fw - 0.5) / fw, (fh - 0.5) / fh),
            ..base
        };
        let b = buffer(&p);
        let g = device.create_bind_group(None, &layout(0), &BindGroupEntries::sequential((b.as_entire_binding(), depth)));
        draw(&mut ctx, pipes[0], &g, &mip_view(0));
    }
    let mut src = textures.hzb_size;
    for m in 1..textures.hzb_mips {
        let p = Params { hzb_in: Vec4::new(1.0 / src.x as f32, 1.0 / src.y as f32, 0.0, 0.0), ..base };
        let b = buffer(&p);
        let from = mip_view(m - 1);
        let g = device.create_bind_group(None, &layout(1), &BindGroupEntries::sequential((b.as_entire_binding(), &from)));
        draw(&mut ctx, pipes[1], &g, &mip_view(m));
        src = (src / 2).max(UVec2::ONE);
    }
    // The setup: AmbientOcclusionSetupParams (ScaleToFullRes 1 from the full-size input, MipThreshold / it).
    {
        let p = Params {
            texel: Vec4::new(1.0 / size.x as f32, 1.0 / size.y as f32, 0.0, 0.0),
            setup: Vec4::new(1.0, MIP_THRESHOLD, size.x as f32, size.y as f32),
            ..base
        };
        let b = buffer(&p);
        let g = device.create_bind_group(None, &layout(2), &BindGroupEntries::sequential((b.as_entire_binding(), normals, depth)));
        draw(&mut ctx, pipes[2], &g, &textures.setup.default_view);
    }
    let hzb_all = &textures.hzb.default_view;
    // The half-size occlusion (its input the setup: scale BufferSize / half).
    {
        let p = params(ssao, size, textures.hzb_size, half, (size.x / half.x).max(1));
        let b = buffer(&p);
        let g = device.create_bind_group(
            None,
            &layout(3),
            &BindGroupEntries::sequential((b.as_entire_binding(), &textures.setup.default_view, hzb_all, &pipelines.hzb_sampler, &pipelines.random)),
        );
        draw(&mut ctx, pipes[3], &g, &textures.half.default_view);
    }
    // The full-size occlusion, upsampling the half-size one.
    {
        let p = Params { texel: Vec4::new(0.0, 0.0, 1.0 / half.x as f32, 1.0 / half.y as f32), ..base };
        let b = buffer(&p);
        let g = device.create_bind_group(
            None,
            &layout(4),
            &BindGroupEntries::sequential((
                b.as_entire_binding(),
                normals,
                hzb_all,
                &pipelines.hzb_sampler,
                &pipelines.random,
                depth,
                &textures.setup.default_view,
                &textures.half.default_view,
            )),
        );
        draw(&mut ctx, pipes[4], &g, &textures.full.default_view);
    }
    // Into Bevy's SSAO texture.
    let copy_layout = cache.get_bind_group_layout(&pipelines.copy_layouts[r32 as usize]);
    let g = device.create_bind_group(None, &copy_layout, &BindGroupEntries::sequential((&textures.full.default_view, &bevy_ssao.screen_space_ambient_occlusion_texture.default_view)));
    let mut pass = ctx.command_encoder().begin_compute_pass(&ComputePassDescriptor { label: Some("game_ssao_copy"), timestamp_writes: None });
    pass.set_pipeline(copy);
    pass.set_bind_group(0, &g, &[]);
    pass.dispatch_workgroups(size.x.div_ceil(8), size.y.div_ceil(8), 1);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hzb_is_half_the_power_of_two() {
        assert_eq!(hzb_size(UVec2::new(1600, 900)), (UVec2::new(1024, 512), 10));
        assert_eq!(hzb_size(UVec2::new(1920, 1080)), (UVec2::new(1024, 1024), 10));
    }

    #[test]
    fn random_texture_repeats_four_by_four() {
        let t = random_texels();
        assert_eq!(t.len(), (RANDOM_SIZE * RANDOM_SIZE * 2) as usize);
        // Reorder[0] = 0: angle 0, length 1 - 0.115: cos -> (0.885 + 1) / 2 x 255.999 = 241.
        assert_eq!(t[0], 241);
        assert_eq!(&t[0..2], &t[8..10]);
    }
}
