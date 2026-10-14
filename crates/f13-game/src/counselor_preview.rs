//! The menus' 3D previews: Counselor_Menu's (CounselorCustomizationPreview) and the wardrobe's
//! (CounselorWardrobePreview), SCActorPreviews (decompiled).
//!
//! SCCounselorCharacterWidget::SetActorPreviewClass (0x14043fd60) spawns the menu's MenuPreviewActorClass once, then per
//! counselor sets its PreviewClass and SpawnPreviewActor (0x140444da0): the counselor class spawned, attached to the
//! preview's RootComponent and added to its CaptureComponent's ShowOnlyActors; back in SetActorPreviewClass its relative
//! transform is the class's CharacterClassList MenuTransform (her actor: the capsule's centre, facing +X) and her outfit
//! her profile's (SetCurrentOutfit(.., bUseMenuPose): SCCounselorCharacter::DeferredLoadOutfit then plays the outfit's
//! MenuAnimation, a montage whose section loops) or, with none, RevertToDefaultOutfit (SetCurrentOutfit(.., false): no
//! menu pose, the AnimBP's stand idle). The wardrobe's SCCounselorWardrobeWidget::SetPreviewClass (0x140442060) spawns
//! her at its preview's root (no MenuTransform) and SetPreviewOutfit (0x140442e80) dresses her with the menu pose. The
//! emote menu's (CounselorEmotePreview) is set up as the wardrobe's; PlayPreviewEmote plays the emote highlighted in its
//! panel on her (none: her pose as above).
//!
//! Each capture (SceneCaptureComponent2D) sees only her, lit by its preview's spot lights and (Counselor_Menu's) its
//! PostProcess component's ambient cubemap (f13-rs: its mean radiance, as lighting.rs does the map's); the menus'
//! preview materials draw its colour with opacity 1 - its alpha (where she is).
//!
//! f13-rs: the preview far under the map on its own render layer, a camera rendering it into an image the menu's
//! CharacterPreviewImage shows (ui.rs). The capture's alpha is a cleared transparent background; its colour untonemapped
//! at the game's exposure (lighting.rs), as SCS_SceneColorHDR's. PLACEHOLDER: the wardrobe preview has no ambient (the
//! world's at the preview's place in the game's menu level, not read).

use bevy::app::{HierarchyPropagatePlugin, Propagate};
use bevy::camera::visibility::RenderLayers;
use bevy::camera::{Hdr, RenderTarget};
use bevy::core_pipeline::tonemapping::{DebandDither, Tonemapping};
use bevy::light::AmbientLight;
use bevy::prelude::*;
use bevy::render::render_resource::TextureFormat;
use serde_json::Value;

use crate::character::{CounselorCharacter, Model, FACING};

/// The preview's render layer (the sense mask's is 7).
const PREVIEW_LAYER: usize = 6;
/// Where the preview stands (m): far under the map, out of its lights' reach.
const PLACE: Vec3 = Vec3::new(0.0, -2000.0, 0.0);

pub struct CounselorPreviewPlugin;

impl Plugin for CounselorPreviewPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(HierarchyPropagatePlugin::<RenderLayers>::new(PostUpdate)).add_systems(Startup, make_targets).add_systems(Update, (show_preview, pose).chain());
    }
}

/// The captures' images (the menus' CharacterPreviewImage brushes: ui.rs): Counselor_Menu's, the wardrobe's and the
/// emote menu's.
#[derive(Resource)]
pub struct PreviewImage {
    pub counselor: Handle<Image>,
    pub wardrobe: Handle<Image>,
    pub emote: Handle<Image>,
    /// Jason_Select_Widget's (KillerCustomizationPreviewActor's KillerPreviewRenderTexture).
    pub killer: Handle<Image>,
}

/// The capture camera (no game lighting, fog, listener or tonemapping on it).
#[derive(Component)]
pub struct PreviewCamera;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Which {
    Counselor,
    Wardrobe,
    Emote,
    Killer,
}

/// What the preview shows (which menu's, the class and her outfit) and its root.
#[derive(Resource)]
struct Preview {
    shown: (Which, String, String),
    root: Entity,
}

/// Her rig, and the animation it plays: an emote (the emote menu's highlighted one), her outfit's MenuAnimation, or
/// (none) the stand idle; and the menu pose it falls back to.
#[derive(Component)]
pub(crate) struct PreviewRig(Option<String>, Option<String>);

/// The Jason menu's Jason (SCKillerCharacterWidget::SetActorPreviewClass: the class at its MenuTransform), standing in his
/// anim Blueprint's stand idle.
#[derive(Component)]
struct KillerPreviewRig;

/// Jason's stand idle (Jason_AnimBP's).
const KILLER_IDLE: &str = "normal_stand_idle_01";

/// The previews' setups (ui/counselor_preview.json, ui/wardrobe_preview.json) and Counselor_Menu's CharacterClassList.
struct Setup {
    counselor: Value,
    wardrobe: Value,
    emote: Value,
    menu: Value,
    killer: Value,
    killer_menu: Value,
}

fn make_targets(mut commands: Commands, mut images: ResMut<Assets<Image>>, export: Res<crate::ExportRoot>) {
    // CustomizationPreviewRenderTarget 1080^2, WardrobePreviewRenderTarget 2048^2 (their TargetSize).
    let size = |f: &str, d: u32| std::fs::read_to_string(export.0.join("ui").join(f)).ok().and_then(|t| serde_json::from_str::<Value>(&t).ok()).and_then(|v| v["TargetSize"].as_u64()).map_or(d, |s| s as u32);
    let mut target = |s: u32| images.add(Image::new_target_texture(s, s, TextureFormat::Rgba16Float, None));
    let counselor = target(size("counselor_preview.json", 1080));
    let wardrobe = target(size("wardrobe_preview.json", 2048));
    let emote = target(size("emote_preview.json", 2048));
    let killer = target(size("killer_preview.json", 1080));
    commands.insert_resource(PreviewImage { counselor, wardrobe, emote, killer });
}

/// A component's RelativeLocation / RelativeRotation (Unreal) in Bevy's space under its root.
fn relative(c: &Value) -> Transform {
    let n = |v: &Value, k: &str| v[k].as_f64().unwrap_or(0.0) as f32;
    let (l, r) = (&c["RelativeLocation"], &c["RelativeRotation"]);
    let x = f13_data::level::Xform { t: [n(l, "X"), n(l, "Y"), n(l, "Z")], r: f13_data::world::rotator_to_quat(n(r, "Pitch"), n(r, "Yaw"), n(r, "Roll")), s: [1.0; 3] };
    crate::to_transform(&x)
}

/// While the wardrobe or the counselor menu is open: its counselor in its preview (respawned as she or her outfit
/// changes); gone as they close.
#[allow(clippy::too_many_arguments)]
fn show_preview(
    mut commands: Commands,
    assets: Res<AssetServer>,
    export: Res<crate::ExportRoot>,
    image: Option<Res<PreviewImage>>,
    (menu, list, wardrobe, profiles): (Option<Res<crate::lobby::CounselorMenu>>, Option<Res<crate::lobby::CounselorList>>, Option<Res<crate::wardrobe::WardrobeMenu>>, Option<Res<crate::wardrobe::CounselorProfiles>>),
    (emote_menu, emotes): (Option<Res<crate::emotes::EmoteMenu>>, Option<Res<crate::emotes::Emotes>>),
    (jason_menu, jason_list, killer): (Option<Res<crate::lobby::JasonMenu>>, Option<Res<crate::lobby::JasonList>>, Option<Res<crate::character::KillerCharacter>>),
    character: Option<Res<CounselorCharacter>>,
    preview: Option<Res<Preview>>,
    mut rigs: Query<&mut PreviewRig>,
    mut data: Local<Option<Setup>>,
) {
    let Some(character) = character else { return };
    let outfit = |c: &str| profiles.as_ref().map_or("", |p| p.outfit(c)).to_string();
    // (The perk menu has no preview.)
    let jason = jason_menu.as_ref().and_then(|m| jason_list.as_ref()?.0.get(m.index)).map(|(k, _)| (Which::Killer, k.clone(), String::new()));
    let shown = match (&emote_menu, &wardrobe, &menu) {
        _ if jason.is_some() => jason,
        (Some(e), _, _) => Some((Which::Emote, e.class.clone(), outfit(&e.class))),
        (None, Some(w), _) => Some((Which::Wardrobe, w.class.clone(), w.spec(&character))),
        (None, None, Some(m)) => list.as_ref().and_then(|l| l.0.get(m.index)).map(|c| (Which::Counselor, c.clone(), outfit(c))),
        _ => None,
    };
    // The emote menu's panel: its highlighted emote plays (PlayPreviewEmote).
    let emote = emote_menu.as_ref().and_then(|m| emotes.as_ref()?.available().get(m.list?).and_then(|e| emotes.as_ref()?.emotes.get(e)).map(|d| d.take.clone()));
    for mut r in &mut rigs {
        let pose = emote.clone().or_else(|| r.1.clone());
        if r.0 != pose {
            r.0 = pose;
        }
    }
    let Some(shown) = shown else {
        if let Some(p) = preview {
            commands.entity(p.root).despawn();
            commands.remove_resource::<Preview>();
        }
        return;
    };
    if preview.as_ref().is_some_and(|p| p.shown == shown) {
        return;
    }
    let Some(image) = image else { return };
    let setup = data.get_or_insert_with(|| {
        let read = |f: &str| std::fs::read_to_string(export.0.join("ui").join(f)).ok().and_then(|t| serde_json::from_str::<Value>(&t).ok()).unwrap_or_default();
        let s = Setup {
            counselor: read("counselor_preview.json"),
            wardrobe: read("wardrobe_preview.json"),
            emote: read("emote_preview.json"),
            menu: read("Counselor_Menu.json"),
            killer: read("killer_preview.json"),
            killer_menu: read("Jason_Select_Widget.json"),
        };
        if s.counselor.is_null() || s.wardrobe.is_null() {
            warn!("no ui/counselor_preview.json or ui/wardrobe_preview.json (re-export: f13-export ui): no counselor preview");
        }
        s
    });
    if let Some(p) = preview {
        commands.entity(p.root).despawn();
    }
    let (which, class, outfit) = &shown;
    let (json, target) = match which {
        Which::Counselor => (&setup.counselor, image.counselor.clone()),
        Which::Wardrobe => (&setup.wardrobe, image.wardrobe.clone()),
        Which::Emote => (&setup.emote, image.emote.clone()),
        Which::Killer => (&setup.killer, image.killer.clone()),
    };
    let root = commands.spawn((Transform::from_translation(PLACE), Visibility::default(), Propagate(RenderLayers::layer(PREVIEW_LAYER)), Name::new("CounselorPreview"))).id();
    // The capture: Unreal's FOVAngle is horizontal (the targets are square), looking along its +X.
    let capture = &json["Capture"];
    let fov = capture["FOVAngle"].as_f64().unwrap_or(90.0) as f32;
    let ambient = json["AmbientCubemapAverage"].as_array().map(|a| a.iter().filter_map(Value::as_f64).map(|v| v as f32).collect::<Vec<_>>()).unwrap_or_default();
    let scale = json["AmbientCubemapIntensity"].as_f64().unwrap_or(1.0) as f32;
    let ambient = if ambient.len() == 3 { Color::linear_rgb(ambient[0] * scale, ambient[1] * scale, ambient[2] * scale) } else { Color::BLACK };
    commands.spawn((
        Camera3d::default(),
        Camera { order: -2, clear_color: ClearColorConfig::Custom(Color::NONE), ..default() },
        RenderTarget::Image(target.into()),
        Projection::Perspective(PerspectiveProjection { fov: fov.to_radians(), aspect_ratio: 1.0, near: 0.01, ..default() }),
        Hdr,
        Tonemapping::None,
        DebandDither::Disabled,
        // (As the game's camera: ssao.rs; the counselors' materials are built for it.)
        Msaa::Off,
        bevy::camera::Exposure { ev100: 0.0 },
        AmbientLight { color: ambient, brightness: 1.0, ..default() },
        relative(capture) * Transform::from_rotation(FACING),
        PreviewCamera,
        ChildOf(root),
    ));
    // Its spot lights (Unreal's shine along +X), as lighting.rs places the map's.
    for l in json["SpotLights"].as_array().into_iter().flatten() {
        let n = |k: &str, d: f32| l[k].as_f64().map_or(d, |v| v as f32);
        let byte = |k: &str| l["LightColor"][k].as_u64().unwrap_or(255) as u8;
        let mut color = LinearRgba::from(Color::srgb_u8(byte("R"), byte("G"), byte("B")));
        if l["bUseTemperature"].as_bool() == Some(true) {
            let t = f13_core::light::temperature_color(n("Temperature", 6500.0));
            color = LinearRgba::rgb(color.red * t[0], color.green * t[1], color.blue * t[2]);
        }
        let outer = n("OuterConeAngle", 44.0).to_radians();
        commands.spawn((
            SpotLight {
                color: color.into(),
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
    // Her actor (Unreal: the capsule's centre, facing +X) at the class's MenuTransform under the root (the wardrobe's:
    // at the root); her model's owner is her feet, facing +Z.
    let menu_transform = match which {
        Which::Killer => setup.killer_menu["Defaults"]["JasonClassList"]
            .as_array()
            .into_iter()
            .flatten()
            .find(|e| {
                e.as_object().and_then(|o| o.iter().find(|(k, _)| k.starts_with("CharacterClass"))).and_then(|(_, v)| v["AssetPathName"].as_str()).is_some_and(|p| p.ends_with(&format!("/{class}.{class}_C")))
            })
            .and_then(|e| e.as_object()?.iter().find(|(k, _)| k.starts_with("MenuTransform")).map(|(_, v)| v.clone()))
            .unwrap_or_default(),
        Which::Counselor => setup.menu["Defaults"]["CharacterClassList"]
            .as_array()
            .into_iter()
            .flatten()
            .find(|e| {
                e.as_object().and_then(|o| o.iter().find(|(k, _)| k.starts_with("CharacterClass"))).and_then(|(_, v)| v["AssetPathName"].as_str()).is_some_and(|p| p.ends_with(&format!("/{class}_Counselor.{class}_Counselor_C")))
            })
            .and_then(|e| e.as_object()?.iter().find(|(k, _)| k.starts_with("MenuTransform")).map(|(_, v)| v.clone()))
            .unwrap_or_default(),
        // PLACEHOLDER: turned to face its capture. SCCounselorWardrobeWidget::SetPreviewClass spawns her at the preview's
        // root (SpawnPreviewActor: identity, snapped), facing its +X, which the capture (at -222 cm) looks along; the
        // wardrobe's only turns are the player's (its NativeTick and mouse move: AddActorLocalRotation by
        // ControllerInputDelta or the drag), so what faces her front is not found.
        Which::Wardrobe | Which::Emote => serde_json::json!({ "Rotation": { "X": 0.0, "Y": 0.0, "Z": 1.0, "W": 0.0 } }),
    };
    let n = |v: &Value, k: &str, d: f32| v[k].as_f64().map_or(d, |x| x as f32);
    let (t, r) = (&menu_transform["Translation"], &menu_transform["Rotation"]);
    let actor = crate::to_transform(&f13_data::level::Xform { t: [n(t, "X", 0.0), n(t, "Y", 0.0), n(t, "Z", 0.0)], r: [n(r, "X", 0.0), n(r, "Y", 0.0), n(r, "Z", 0.0), n(r, "W", 1.0)], s: [1.0; 3] });
    if *which == Which::Killer {
        // His looks (the class's killer JSON) on the loaded Jason's skeleton and animations (every Jason's: Jason_Base's).
        let looks = jason_list.as_ref().and_then(|l| l.0.iter().find(|(k, _)| k == class)).and_then(|(_, v)| serde_json::from_value::<f13_data::character::CharacterInfo>(v.clone()).ok());
        if let (Some(looks), Some(killer)) = (looks, killer.as_ref()) {
            let feet = actor * Transform::from_translation(Vec3::NEG_Y * looks.capsule_half_height / 100.0) * Transform::from_rotation(FACING.inverse());
            let owner = commands.spawn((feet, Visibility::default(), ChildOf(root))).id();
            crate::character::spawn_model_as(&mut commands, &assets, &killer.0, Some(std::sync::Arc::new(looks)), Model::Killer, owner, KillerPreviewRig);
        }
        commands.insert_resource(Preview { shown, root });
        return;
    }
    let looks = character.dressed(class, outfit);
    let feet = actor * Transform::from_translation(Vec3::NEG_Y * looks.capsule_half_height / 100.0) * Transform::from_rotation(FACING.inverse());
    let owner = commands.spawn((feet, Visibility::default(), ChildOf(root))).id();
    // The menu pose with an outfit set (her profile's, the wardrobe's), its MenuAnimation.
    let pose = (*which == Which::Wardrobe || !outfit.is_empty()).then(|| looks.current_outfit().and_then(|(o, _)| looks.outfits[o].menu_animation.clone())).flatten();
    let (model, _) = character.model_for(&looks);
    crate::character::spawn_model_as(&mut commands, &assets, model, Some(looks.clone()), Model::Counselor, owner, PreviewRig(emote.or(pose.clone()), pose));
    commands.insert_resource(Preview { shown, root });
}

/// Her animation: her outfit's MenuAnimation, looping (its section's next is itself), or the stand idle; his, his stand
/// idle.
fn pose(
    time: Res<Time>,
    character: Option<Res<CounselorCharacter>>,
    mut rigs: Query<(&mut crate::character::Rig, &mut AnimationPlayer, &PreviewRig)>,
    killer: Option<Res<crate::character::KillerCharacter>>,
    mut killers: Query<(&mut crate::character::Rig, &mut AnimationPlayer), (With<KillerPreviewRig>, Without<PreviewRig>)>,
) {
    if let Some(killer) = &killer {
        let targets = std::collections::HashMap::from([(KILLER_IDLE, 1.0)]);
        for (mut rig, mut anim) in &mut killers {
            rig.drive(&killer.0, &targets, &|_| false, time.delta_secs(), &mut anim);
        }
    }
    let Some(character) = character else { return };
    for (mut rig, mut anim, PreviewRig(pose, _)) in &mut rigs {
        let model = character.model(rig.male);
        let clip = pose.as_deref().filter(|p| model.length(p).is_some()).unwrap_or(crate::character::IDLE);
        let targets = std::collections::HashMap::from([(clip, 1.0)]);
        rig.drive(model, &targets, &|_| false, time.delta_secs(), &mut anim);
    }
}
