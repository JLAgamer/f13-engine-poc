//! Sense's highlights on the played Jason's screen: the counselor's (SenseCounselorMaterial,
//! PP_Jason_Sense_Counselor_01_MASKED: shaders/sense_counselor.wgsl) and her cabin's (SenseCabinMaterial,
//! PP_Jason_Sense_Cabin_01: shaders/sense_cabin.wgsl).
//!
//! From the game: Sense (FUN_14040a230) adds the material's instance to his ForcedPostProcessVolume's blendables (weight
//! 1) and its end (FUN_1403ea820) removes it. While it's on, his tick (FUN_1404111f0) turns on the custom depth of a
//! sensed counselor's meshes (FUN_1403d3cf0: her Mesh and three more parts; SetRenderCustomDepth) when she's within
//! the Sense radius, alive, not hiding and not indoors (indoors her cabin's SetSenseEffects instead; hiding, the
//! spot's): senses.rs's `highlight`. Custom depth draws her meshes alone, so the stencil the material reads has her
//! through walls (stencil 2). Indoors, her cabin's SetSenseEffects (Cabin_Base) calls SetSenseSettings on each of its
//! child actors that is a door (BP_DoorBase) or window (Cabin_Window_BP): custom depth on all their primitives,
//! stencil 1 (senses.rs's `sensed_cabin`; not while he's in that cabin too).
//!
//! Here the custom stencil is a mask: a second camera at his camera's view, drawing copies of those meshes (same mesh
//! and skin, unlit: red for hers, green for the cabin's) on their own render layer, before his camera. The passes read
//! it, his camera's depth (bound as a texture) and the scene; hers first (the order Sense adds them).

use bevy::asset::{embedded_asset, load_embedded_asset};
use bevy::camera::visibility::RenderLayers;
use bevy::camera::RenderTarget;
use bevy::core_pipeline::schedule::{Core3d, Core3dSystems};
use bevy::core_pipeline::tonemapping::{DebandDither, Tonemapping};
use bevy::core_pipeline::FullscreenShader;
use bevy::light::NotShadowCaster;
use bevy::mesh::morph::MeshMorphWeights;
use bevy::mesh::skinning::SkinnedMesh;
use bevy::prelude::*;
use bevy::render::extract_component::{ComponentUniforms, DynamicUniformIndex, ExtractComponent, ExtractComponentPlugin, UniformComponentPlugin};
use bevy::render::render_asset::RenderAssets;
use bevy::render::render_resource::binding_types::{sampler, texture_2d, texture_depth_2d, texture_depth_2d_multisampled, uniform_buffer};
use bevy::render::render_resource::*;
use bevy::render::renderer::{RenderContext, RenderDevice, ViewQuery};
use bevy::render::sync_component::SyncComponent;
use bevy::render::texture::GpuImage;
use bevy::render::view::{ExtractedView, Msaa, ViewDepthTexture, ViewTarget};
use bevy::render::{GpuResourceAppExt, Render, RenderApp, RenderStartup, RenderSystems};
use bevy::shader::{Shader, ShaderDefVal};
use std::collections::HashMap;

/// The mask camera's render layer.
const SENSE_LAYER: usize = 7;
/// The material's WispySmoke1A/1B/1C.
const SMOKE: &str = "SummerCamp/Content/Effects/Textures/smoke/T_Wispy_Smoke_Tiled_01.dds";

pub struct SenseViewPlugin;

impl Plugin for SenseViewPlugin {
    fn build(&self, app: &mut App) {
        embedded_asset!(app, "shaders/sense_counselor.wgsl");
        embedded_asset!(app, "shaders/sense_cabin.wgsl");
        app.add_plugins((ExtractComponentPlugin::<SenseHighlight>::default(), UniformComponentPlugin::<SenseUniform>::default()))
            .add_systems(Update, (mask_camera, mirrors, highlight).chain());
        let Some(render_app) = app.get_sub_app_mut(RenderApp) else { return };
        render_app
            .init_gpu_resource::<SpecializedRenderPipelines<SensePipeline>>()
            .add_systems(RenderStartup, init_pipeline)
            .add_systems(Render, prepare_pipelines.in_set(RenderSystems::Prepare))
            // In his volume's blendables after the stun's (from the start) and Shift's.
            .add_systems(Core3d, sense.after(crate::screen_effects::shift).before(crate::screen_effects::vhs).in_set(Core3dSystems::PostProcess));
    }
}

/// The custom stencil's camera (the other 3D-camera systems leave it alone).
#[derive(Component)]
pub struct SenseMaskCamera;

/// A copy of one of her (or the cabin's) meshes on the mask's layer.
#[derive(Component)]
struct SenseMirror;

/// A door or window spawned by a cabin (Cabin_Base): the cabin's match actor index.
#[derive(Component)]
pub struct CabinPart(pub usize);

/// The highlights on a camera (hers, the cabin's): the mask, the smoke texture, the view's forward axis (Unreal's)
/// and time, its near plane.
#[derive(Component, Clone)]
pub struct SenseHighlight {
    counselor: bool,
    cabin: bool,
    mask: Handle<Image>,
    smoke: Handle<Image>,
    forward_time: Vec4,
    near: f32,
}

#[derive(Component, ShaderType, Clone)]
pub struct SenseUniform {
    forward_time: Vec4,
    near: Vec4,
}

impl SyncComponent for SenseHighlight {
    type Target = (SenseHighlight, SenseUniform);
}

impl ExtractComponent for SenseHighlight {
    type QueryData = &'static SenseHighlight;
    type QueryFilter = ();
    type Out = (SenseHighlight, SenseUniform);

    fn extract_component(s: bevy::ecs::query::QueryItem<'_, '_, Self::QueryData>) -> Option<Self::Out> {
        Some((s.clone(), SenseUniform { forward_time: s.forward_time, near: Vec4::new(s.near, 0.0, 0.0, 0.0) }))
    }
}

/// The mask: its image, size and camera.
#[derive(Resource)]
struct Mask {
    image: Handle<Image>,
    size: UVec2,
    camera: Entity,
}

/// The played Jason's camera gets the mask camera (its image sized as his view) and its depth as a texture.
fn mask_camera(
    mut commands: Commands,
    mut images: ResMut<Assets<Image>>,
    play: Option<Res<crate::jason_player::PlayJason>>,
    mut cameras: Query<(Entity, &Camera, &mut Camera3d, Ref<Projection>), (With<crate::player::PlayerCamera>, Without<SenseMaskCamera>)>,
    mut masks: Query<&mut Projection, (With<SenseMaskCamera>, Without<crate::player::PlayerCamera>)>,
    mask: Option<ResMut<Mask>>,
) {
    if play.is_none() {
        return;
    }
    let Some((camera_entity, camera, mut camera3d, projection)) = cameras.iter_mut().next() else { return };
    let Some(size) = camera.physical_target_size().filter(|s| s.x > 0 && s.y > 0) else { return };
    let usage = TextureUsages::RENDER_ATTACHMENT | TextureUsages::TEXTURE_BINDING;
    if TextureUsages::from(camera3d.depth_texture_usages) != usage {
        camera3d.depth_texture_usages = usage.into();
    }
    if let Some(m) = mask.as_ref().filter(|m| m.size == size) {
        if let Ok(mut p) = masks.get_mut(m.camera) {
            if projection.is_changed() {
                *p = projection.as_ref().clone();
            }
        }
        return;
    }
    if let Some(m) = mask {
        commands.entity(m.camera).despawn();
    }
    let image = images.add(Image::new_target_texture(size.x, size.y, TextureFormat::Rgba8Unorm, None));
    let camera = commands
        .spawn((
            Camera3d::default(),
            Camera { order: -1, clear_color: ClearColorConfig::Custom(Color::BLACK), ..default() },
            RenderTarget::Image(image.clone().into()),
            projection.as_ref().clone(),
            Tonemapping::None,
            DebandDither::Disabled,
            Msaa::Off,
            RenderLayers::layer(SENSE_LAYER),
            Transform::IDENTITY,
            SenseMaskCamera,
            ChildOf(camera_entity),
        ))
        .id();
    commands.insert_resource(Mask { image, size, camera });
}

/// What Sense highlights now: her (outside), and the cabin she's in.
fn sensed(play: &Option<Res<crate::jason_player::PlayJason>>, knowledge: &Query<&crate::senses::Knowledge>) -> (bool, Option<usize>) {
    if play.is_none() {
        return (false, None);
    }
    (knowledge.iter().any(|k| k.highlight), knowledge.iter().find_map(|k| k.sensed_cabin))
}

/// Copies of the highlighted meshes on the mask's layer.
#[allow(clippy::too_many_arguments)]
fn mirrors(
    mut commands: Commands,
    mut materials: ResMut<Assets<StandardMaterial>>,
    play: Option<Res<crate::jason_player::PlayJason>>,
    knowledge: Query<&crate::senses::Knowledge>,
    rigs: Query<(Entity, &crate::character::Rig)>,
    parts: Query<(Entity, &CabinPart)>,
    children: Query<&Children>,
    meshes: Query<(&Mesh3d, Option<&SkinnedMesh>, Option<&MeshMorphWeights>), Without<SenseMirror>>,
    mut mask_cameras: Query<&mut Camera, With<SenseMaskCamera>>,
    mut mirrored: Local<HashMap<Entity, Entity>>,
    mut colours: Local<Option<(Handle<StandardMaterial>, Handle<StandardMaterial>)>>,
) {
    let (her, cabin) = sensed(&play, &knowledge);
    let on = her || cabin.is_some();
    // The mask draws only while there's something to draw.
    for mut c in &mut mask_cameras {
        if c.is_active != on {
            c.is_active = on;
        }
    }
    let (red, green) = colours
        .get_or_insert_with(|| {
            let unlit = |c: Color| StandardMaterial { base_color: c, unlit: true, ..default() };
            (materials.add(unlit(Color::linear_rgb(1.0, 0.0, 0.0))), materials.add(unlit(Color::linear_rgb(0.0, 1.0, 0.0))))
        })
        .clone();
    // The meshes to mirror, and their mask colour.
    let mut wanted: HashMap<Entity, Handle<StandardMaterial>> = HashMap::new();
    if her {
        for (rig_entity, rig) in &rigs {
            if matches!(rig.model, crate::character::Model::Counselor) {
                wanted.extend(children.iter_descendants(rig_entity).filter(|e| meshes.contains(*e)).map(|e| (e, red.clone())));
            }
        }
    }
    if let Some(c) = cabin {
        for (root, part) in &parts {
            if part.0 == c {
                wanted.extend(children.iter_descendants(root).filter(|e| meshes.contains(*e)).map(|e| (e, green.clone())));
            }
        }
    }
    mirrored.retain(|source, mirror| {
        let keep = wanted.contains_key(source);
        if !keep {
            if let Ok(mut e) = commands.get_entity(*mirror) {
                e.despawn();
            }
        }
        keep
    });
    for (e, material) in wanted {
        if mirrored.contains_key(&e) {
            continue;
        }
        let Ok((mesh, skin, morphs)) = meshes.get(e) else { continue };
        {
            let mut m = commands.spawn((
                Mesh3d(mesh.0.clone()),
                MeshMaterial3d(material),
                Transform::IDENTITY,
                RenderLayers::layer(SENSE_LAYER),
                NotShadowCaster,
                SenseMirror,
                ChildOf(e),
            ));
            if let Some(s) = skin {
                m.insert(s.clone());
            }
            if let Some(w) = morphs {
                m.insert(w.clone());
            }
            mirrored.insert(e, m.id());
        }
    }
}

/// The passes on his camera while the mirrors are up.
fn highlight(
    mut commands: Commands,
    time: Res<Time>,
    assets: Res<AssetServer>,
    mask: Option<Res<Mask>>,
    knowledge: Query<&crate::senses::Knowledge>,
    play: Option<Res<crate::jason_player::PlayJason>>,
    cameras: Query<(Entity, &GlobalTransform, &Projection, Has<SenseHighlight>), With<crate::player::PlayerCamera>>,
    mut smoke: Local<Option<Handle<Image>>>,
) {
    let (her, cabin) = sensed(&play, &knowledge);
    let on = her || cabin.is_some();
    for (e, xf, projection, has) in &cameras {
        match mask.as_deref().filter(|_| on) {
            Some(m) => {
                let f = xf.forward();
                // Unreal's axes: x forward, y right, z up (Bevy's x, z, y).
                let forward_time = Vec4::new(f.x, f.z, f.y, time.elapsed_secs());
                let near = match projection {
                    Projection::Perspective(p) => p.near,
                    _ => 0.1,
                };
                let smoke = smoke.get_or_insert_with(|| assets.load(SMOKE)).clone();
                commands.entity(e).insert(SenseHighlight { counselor: her, cabin: cabin.is_some(), mask: m.image.clone(), smoke, forward_time, near });
            }
            None if has => {
                commands.entity(e).remove::<SenseHighlight>();
            }
            None => {}
        }
    }
}

#[derive(Resource)]
struct SensePipeline {
    layout: BindGroupLayoutDescriptor,
    layout_ms: BindGroupLayoutDescriptor,
    sampler: Sampler,
    wrap: Sampler,
    fullscreen_shader: FullscreenShader,
    shader: Handle<Shader>,
    cabin_shader: Handle<Shader>,
}

fn init_pipeline(mut commands: Commands, device: Res<RenderDevice>, fullscreen_shader: Res<FullscreenShader>, asset_server: Res<AssetServer>) {
    let layout = |label: &'static str, ms: bool| {
        let depth = if ms { texture_depth_2d_multisampled() } else { texture_depth_2d() };
        BindGroupLayoutDescriptor::new(
            label,
            &BindGroupLayoutEntries::sequential(
                ShaderStages::FRAGMENT,
                (
                    texture_2d(TextureSampleType::Float { filterable: true }),
                    sampler(SamplerBindingType::Filtering),
                    uniform_buffer::<SenseUniform>(true),
                    texture_2d(TextureSampleType::Float { filterable: false }),
                    texture_2d(TextureSampleType::Float { filterable: true }),
                    sampler(SamplerBindingType::Filtering),
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
    // The material's texture samples: wrapped, trilinear.
    let wrap = device.create_sampler(&SamplerDescriptor {
        address_mode_u: AddressMode::Repeat,
        address_mode_v: AddressMode::Repeat,
        mag_filter: FilterMode::Linear,
        min_filter: FilterMode::Linear,
        mipmap_filter: MipmapFilterMode::Linear,
        ..default()
    });
    commands.insert_resource(SensePipeline {
        layout: layout("sense_layout", false),
        layout_ms: layout("sense_layout_ms", true),
        sampler,
        wrap,
        fullscreen_shader: fullscreen_shader.clone(),
        shader: load_embedded_asset!(asset_server.as_ref(), "shaders/sense_counselor.wgsl"),
        cabin_shader: load_embedded_asset!(asset_server.as_ref(), "shaders/sense_cabin.wgsl"),
    });
}

/// The passes' pipelines: hers and the cabin's (when on).
#[derive(Component)]
struct SensePipelineIds(Option<CachedRenderPipelineId>, Option<CachedRenderPipelineId>);

impl SpecializedRenderPipeline for SensePipeline {
    /// The target format, whether the depth is multisampled, and whether it's the cabin's.
    type Key = (TextureFormat, bool, bool);

    fn specialize(&self, (format, ms, cabin): Self::Key) -> RenderPipelineDescriptor {
        let shader_defs: Vec<ShaderDefVal> = if ms { vec!["MULTISAMPLED".into()] } else { vec![] };
        RenderPipelineDescriptor {
            label: Some(if cabin { "sense_cabin" } else { "sense_counselor" }.into()),
            layout: vec![if ms { self.layout_ms.clone() } else { self.layout.clone() }],
            vertex: self.fullscreen_shader.to_vertex_state(),
            fragment: Some(FragmentState {
                shader: if cabin { self.cabin_shader.clone() } else { self.shader.clone() },
                shader_defs,
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
    mut pipelines: ResMut<SpecializedRenderPipelines<SensePipeline>>,
    pipeline: Res<SensePipeline>,
    views: Query<(Entity, &ExtractedView, &Msaa, &SenseHighlight)>,
) {
    for (e, view, msaa, h) in &views {
        let mut id = |cabin: bool| pipelines.specialize(&cache, &pipeline, (view.target_format, msaa.samples() > 1, cabin));
        let ids = SensePipelineIds(h.counselor.then(|| id(false)), h.cabin.then(|| id(true)));
        commands.entity(e).insert(ids);
    }
}

fn sense(
    view: ViewQuery<(&ViewTarget, &ViewDepthTexture, &Msaa, &SenseHighlight, &SensePipelineIds, &DynamicUniformIndex<SenseUniform>)>,
    pipeline: Res<SensePipeline>,
    cache: Res<PipelineCache>,
    images: Res<RenderAssets<GpuImage>>,
    uniforms: Res<ComponentUniforms<SenseUniform>>,
    mut ctx: RenderContext,
) {
    let (target, depth, msaa, highlight, ids, index) = view.into_inner();
    let (Some(mask), Some(smoke), Some(binding)) = (images.get(&highlight.mask), images.get(&highlight.smoke), uniforms.binding()) else { return };
    let layout = if msaa.samples() > 1 { &pipeline.layout_ms } else { &pipeline.layout };
    for render_pipeline in [ids.0, ids.1].into_iter().flatten().filter_map(|id| cache.get_render_pipeline(id)) {
        let post = target.post_process_write();
        let bind_group = ctx.render_device().create_bind_group(
            None,
            &cache.get_bind_group_layout(layout),
            &BindGroupEntries::sequential((post.source, &pipeline.sampler, binding.clone(), &mask.texture_view, &smoke.texture_view, &pipeline.wrap, depth.view())),
        );
        let mut pass = ctx.command_encoder().begin_render_pass(&RenderPassDescriptor {
            label: Some("sense"),
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
}
