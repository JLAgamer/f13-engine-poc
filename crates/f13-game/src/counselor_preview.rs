//! Counselor_Menu's 3D preview (CharacterPreviewImage): CounselorCustomizationPreview, an SCActorPreview (decompiled).
//!
//! SCCounselorCharacterWidget::SetActorPreviewClass (0x14043fd60) spawns the menu's MenuPreviewActorClass once, then per
//! counselor sets its PreviewClass and SpawnPreviewActor (0x140444da0): the counselor class spawned, attached to the
//! preview's RootComponent and added to its CaptureComponent's ShowOnlyActors; back in SetActorPreviewClass its relative
//! transform is the class's CharacterClassList MenuTransform (her actor: the capsule's centre, facing +X) and her outfit
//! her profile's (RevertToDefaultOutfit without one: f13-rs has no profiles). The capture (SceneCaptureComponent2D,
//! FOVAngle 40, CustomizationPreviewRenderTarget 1080 x 1080) sees only her, lit by the preview's three spot lights and
//! its PostProcess component's ambient cubemap (HigginsCubeMap x 2: f13-rs its mean radiance, as lighting.rs does the
//! map's); CustomizationPreviewMaterial draws the capture's colour with opacity 1 - its alpha (where she is).
//!
//! f13-rs: the preview far under the map on its own render layer, a camera rendering it into an image the menu's
//! CharacterPreviewImage shows (ui.rs). PLACEHOLDER: her animation, the counselor's stand idle (the menu's AnimBP state
//! not read); the capture's alpha is a cleared transparent background; its colour untonemapped at the game's exposure
//! (lighting.rs), as SCS_SceneColorHDR's.

use bevy::app::{HierarchyPropagatePlugin, Propagate};
use bevy::camera::visibility::RenderLayers;
use bevy::camera::RenderTarget;
use bevy::core_pipeline::tonemapping::{DebandDither, Tonemapping};
use bevy::light::AmbientLight;
use bevy::prelude::*;
use bevy::render::render_resource::TextureFormat;
use bevy::camera::Hdr;
use serde_json::Value;

use crate::character::{CounselorCharacter, Model, FACING};

/// The preview's render layer (the sense mask's is 7).
const PREVIEW_LAYER: usize = 6;
/// Where the preview stands (m): far under the map, out of its lights' reach.
const PLACE: Vec3 = Vec3::new(0.0, -2000.0, 0.0);
/// CustomizationPreviewRenderTarget's size.
const SIZE: u32 = 1080;

pub struct CounselorPreviewPlugin;

impl Plugin for CounselorPreviewPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(HierarchyPropagatePlugin::<RenderLayers>::new(PostUpdate)).add_systems(Startup, make_target).add_systems(Update, (show_preview, idle).chain());
    }
}

/// The capture's image (CharacterPreviewImage's brush: ui.rs).
#[derive(Resource)]
pub struct PreviewImage(pub Handle<Image>);

/// The capture camera (no game lighting, fog, listener or tonemapping on it).
#[derive(Component)]
pub struct PreviewCamera;

/// The counselor shown and the preview's root.
#[derive(Resource)]
struct Preview {
    class: String,
    root: Entity,
}

#[derive(Component)]
struct PreviewRig;

fn make_target(mut commands: Commands, mut images: ResMut<Assets<Image>>) {
    let image = images.add(Image::new_target_texture(SIZE, SIZE, TextureFormat::Rgba16Float, None));
    commands.insert_resource(PreviewImage(image));
}

/// A component's RelativeLocation / RelativeRotation (Unreal) in Bevy's space under its root.
fn relative(c: &Value) -> Transform {
    let n = |v: &Value, k: &str| v[k].as_f64().unwrap_or(0.0) as f32;
    let (l, r) = (&c["RelativeLocation"], &c["RelativeRotation"]);
    let x = f13_data::level::Xform { t: [n(l, "X"), n(l, "Y"), n(l, "Z")], r: f13_data::world::rotator_to_quat(n(r, "Pitch"), n(r, "Yaw"), n(r, "Roll")), s: [1.0; 3] };
    crate::to_transform(&x)
}

/// While the counselor menu is open: its counselor in the preview (respawned as it changes); gone as it closes.
#[allow(clippy::too_many_arguments)]
fn show_preview(
    mut commands: Commands,
    assets: Res<AssetServer>,
    export: Res<crate::ExportRoot>,
    image: Option<Res<PreviewImage>>,
    menu: Option<Res<crate::lobby::CounselorMenu>>,
    list: Option<Res<crate::lobby::CounselorList>>,
    character: Option<Res<CounselorCharacter>>,
    preview: Option<Res<Preview>>,
    mut data: Local<Option<(Value, Value)>>,
) {
    let class = menu.and_then(|m| list.as_ref()?.0.get(m.index).cloned());
    let Some(class) = class else {
        if let Some(p) = preview {
            commands.entity(p.root).despawn();
            commands.remove_resource::<Preview>();
        }
        return;
    };
    if preview.as_ref().is_some_and(|p| p.class == class) {
        return;
    }
    let (Some(image), Some(character)) = (image, character) else { return };
    let (setup, menu_json) = data.get_or_insert_with(|| {
        let read = |f: &str| std::fs::read_to_string(export.0.join("ui").join(f)).ok().and_then(|t| serde_json::from_str::<Value>(&t).ok()).unwrap_or_default();
        let setup = read("counselor_preview.json");
        if setup.is_null() {
            warn!("no ui/counselor_preview.json (re-export: f13-export ui): no counselor preview");
        }
        (setup, read("Counselor_Menu.json"))
    });
    if let Some(p) = preview {
        commands.entity(p.root).despawn();
    }
    let layers = RenderLayers::layer(PREVIEW_LAYER);
    let root = commands.spawn((Transform::from_translation(PLACE), Visibility::default(), Propagate(layers.clone()), Name::new("CounselorPreview"))).id();
    // The capture: Unreal's FOVAngle is horizontal (the target is square), looking along its +X.
    let capture = &setup["Capture"];
    let fov = capture["FOVAngle"].as_f64().unwrap_or(90.0) as f32;
    let ambient = setup["AmbientCubemapAverage"].as_array().map(|a| a.iter().filter_map(Value::as_f64).map(|v| v as f32).collect::<Vec<_>>()).unwrap_or_default();
    let scale = setup["AmbientCubemapIntensity"].as_f64().unwrap_or(1.0) as f32;
    let ambient = if ambient.len() == 3 { Color::linear_rgb(ambient[0] * scale, ambient[1] * scale, ambient[2] * scale) } else { Color::BLACK };
    commands.spawn((
        Camera3d::default(),
        Camera { order: -2, clear_color: ClearColorConfig::Custom(Color::NONE), ..default() },
        RenderTarget::Image(image.0.clone().into()),
        Projection::Perspective(PerspectiveProjection { fov: fov.to_radians(), aspect_ratio: 1.0, near: 0.01, ..default() }),
        Hdr,
        Tonemapping::None,
        DebandDither::Disabled,
        bevy::camera::Exposure { ev100: 0.0 },
        AmbientLight { color: ambient, brightness: 1.0, ..default() },
        relative(capture) * Transform::from_rotation(FACING),
        PreviewCamera,
        ChildOf(root),
    ));
    // Its spot lights (Unreal's shine along +X), as lighting.rs places the map's.
    for l in setup["SpotLights"].as_array().into_iter().flatten() {
        let n = |k: &str, d: f32| l[k].as_f64().map_or(d, |v| v as f32);
        let byte = |k: &str| l["LightColor"][k].as_u64().unwrap_or(255) as u8;
        let outer = n("OuterConeAngle", 44.0).to_radians();
        commands.spawn((
            SpotLight {
                color: Color::srgb_u8(byte("R"), byte("G"), byte("B")),
                intensity: crate::lighting::inverse_squared_lumens(n("Intensity", 5000.0)),
                range: n("AttenuationRadius", 1000.0) / 100.0,
                radius: 0.0,
                inner_angle: n("InnerConeAngle", 0.0).to_radians().min(outer),
                outer_angle: outer,
                shadow_maps_enabled: true,
                ..default()
            },
            relative(l) * Transform::from_rotation(FACING),
            ChildOf(root),
        ));
    }
    // Her actor at the class's MenuTransform under the root (Unreal: the capsule's centre, facing +X); her model's owner
    // is her feet, facing +Z.
    let entry = menu_json["Defaults"]["CharacterClassList"].as_array().into_iter().flatten().find(|e| {
        e.as_object().and_then(|o| o.iter().find(|(k, _)| k.starts_with("CharacterClass"))).and_then(|(_, v)| v["AssetPathName"].as_str()).is_some_and(|p| p.ends_with(&format!("/{class}_Counselor.{class}_Counselor_C")))
    });
    let menu_transform = entry.and_then(|e| e.as_object()?.iter().find(|(k, _)| k.starts_with("MenuTransform")).map(|(_, v)| v.clone())).unwrap_or_default();
    let n = |v: &Value, k: &str, d: f32| v[k].as_f64().map_or(d, |x| x as f32);
    let (t, r) = (&menu_transform["Translation"], &menu_transform["Rotation"]);
    let actor = crate::to_transform(&f13_data::level::Xform { t: [n(t, "X", 0.0), n(t, "Y", 0.0), n(t, "Z", 0.0)], r: [n(r, "X", 0.0), n(r, "Y", 0.0), n(r, "Z", 0.0), n(r, "W", 1.0)], s: [1.0; 3] });
    let looks = character.looks(&class);
    let feet = actor * Transform::from_translation(Vec3::NEG_Y * looks.capsule_half_height / 100.0) * Transform::from_rotation(FACING.inverse());
    let owner = commands.spawn((feet, Visibility::default(), ChildOf(root))).id();
    let (model, _) = character.model_for(&looks);
    crate::character::spawn_model_as(&mut commands, &assets, model, Some(looks.clone()), Model::Counselor, owner, PreviewRig);
    commands.insert_resource(Preview { class, root });
}

/// Her stand idle (PLACEHOLDER: the preview's animation not read).
fn idle(time: Res<Time>, character: Option<Res<CounselorCharacter>>, mut rigs: Query<(&mut crate::character::Rig, &mut AnimationPlayer), With<PreviewRig>>) {
    let Some(character) = character else { return };
    let targets = std::collections::HashMap::from([(crate::character::IDLE, 1.0)]);
    for (mut rig, mut anim) in &mut rigs {
        let model = character.model(rig.male);
        rig.drive(model, &targets, &|_| false, time.delta_secs(), &mut anim);
    }
}
