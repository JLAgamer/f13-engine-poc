//! Character models and animations, from the export's `counselors/<Blueprint>.json`, `killers/<Blueprint>.json`
//! and their `.anim` files. Jason (jason.rs) is assembled the same way: his mask and hair follow the body as
//! master-pose parts and his weapon hangs off its hand socket.
//!
//! Assembled as the game does it: the bare base body (CharacterMesh0) carries the skeleton, the outfit follows it as
//! a master-pose component (its skin is re-bound to the body's bones of the same names), the face hangs off the head
//! bone and the hair off the face. The face and hair stay in their bind pose (their own anim Blueprints, facial
//! animation and hair dynamics, are not ported).
//!
//! Animations are retargeted as Unreal does on the shared counselor skeleton: rotations from the animation,
//! translations from the animation only for the bones whose retargeting mode is "Animation" (root, pelvis, weapon
//! attach points); every other bone keeps the body's own bind-pose translation (the exporter only writes the former).
//!
//! Blending follows Athlete_AnimBP's female blendspaces: BS_F_Normal_Move (SpeedState 0 idle, 1 walk, 2 run,
//! 3 sprint), BS_F_Combat (direction x speed: combat idle, eight walk directions), crouch idle/walk, the wounded idle,
//! and full-body montages (dodges, stumbles, first aid). Weights move toward their targets at the blendspaces'
//! TargetWeightInterpolationSpeedPerSec; cycles are synced by normalised time (the game syncs by foot markers).

use bevy::animation::{animated_field, AnimatedBy, AnimationTargetId};
use bevy::math::curve::{ConstantCurve, Interval, SampleAutoCurve};
use bevy::prelude::*;
use bevy::world_serialization::{WorldAssetRoot, WorldInstanceReady};
use f13_core::counselor::Stance;
use f13_data::character::{AnimData, CharacterInfo};
use std::collections::HashMap;

use crate::player::{Player, PlayerTuning};

/// Weight change per second (BS_F_Normal_Move's and BS_F_Combat's TargetWeightInterpolationSpeedPerSec).
const BLEND_RATE: f32 = 4.0;
/// Montage blend in/out (Unreal's default 0.25 s): weight change per second.
const MONTAGE_BLEND_RATE: f32 = 4.0;

pub(crate) const IDLE: &str = "F_normal_stand_idle_01";
const HURT_IDLE: &str = "F_hurt_stand_idle_01";
const MOVE: [&str; 3] = ["F_normal_stand_walk_forward", "F_normal_stand_run_forward", "F_normal_stand_sprint_forward"];
const CROUCH_IDLE: &str = "f_normal_crouch_idle_01";
const CROUCH_WALK: &str = "f_normal_crouch_walk_forward";
/// Swimming: idle, slow, fast, sprint.
const SWIM: [&str; 4] = ["f_normal_swim_idle", "f_normal_swim_slow", "f_normal_swim_fast", "f_normal_swim_sprint"];
const COMBAT_IDLE: &str = "f_combat_stand_idle_1h";
/// AM_F_Combat_Stand_Block_Idle.
const BLOCK_IDLE: &str = "F_combat_stand_block_idle";
/// BS_F_Combat's walk samples by direction (degrees, + is right).
const COMBAT_WALK: [(f32, &str); 9] = [
    (-180.0, "F_normal_stand_walk_back"),
    (-135.0, "F_normal_stand_walk_back_45_left"),
    (-90.0, "F_normal_stand_walk_left"),
    (-45.0, "F_normal_stand_walk_forward_45_left"),
    (0.0, "F_normal_stand_walk_forward"),
    (45.0, "F_normal_stand_walk_forward_45_right"),
    (90.0, "F_normal_stand_walk_right"),
    (135.0, "F_normal_stand_walk_back_45_right"),
    (180.0, "F_normal_stand_walk_back"),
];
const FIRST_AID: &str = "f_normal_stand_use_firstaid_self";

/// The female sequence Vanessa plays for a montage the rules start (the shared montages hold the male takes; the
/// female ones have the same length and root motion).
fn female_sequence(montage: &str) -> Option<&'static str> {
    Some(match montage {
        "AM_combat_stand_Dodge_Left" => "f_combat_stand_dodge_left_01",
        "AM_combat_stand_Dodge_Right" => "f_combat_stand_dodge_right_01",
        "AM_combat_stand_dodge_back" => "f_combat_stand_dodge_back_01",
        "AM_normal_stand_run_stumble_short" => "f_normal_stand_run_stumble_short_01",
        "AM_normal_stand_sprint_stumble_short" => "f_normal_stand_sprint_stumble_short_01",
        _ => return None,
    })
}

/// Loads the MVP counselor (Vanessa) and Jason from the export before anything spawns, then assembles their models
/// and animates the counselor (Jason's animation is driven by jason.rs).
pub struct CharacterPlugin {
    pub export: std::path::PathBuf,
    /// Testing: hold the counselor in one animation at a time (seconds) instead of driving it from her state.
    pub pose: Option<(String, f32)>,
    /// The counselor's outfit (`--outfit <class or name>[:option,...]`, CharacterInfo::choose_outfit); None: her
    /// default outfit.
    pub outfit: Option<String>,
}

#[derive(Resource)]
pub(crate) struct ForcedPose(String, f32);

/// Vanessa's and Jason's Blueprints, as the exporter names their files.
const COUNSELOR: &str = "Athlete_Counselor";
const KILLER: &str = "Jason_SackHead";

impl Plugin for CharacterPlugin {
    fn build(&self, app: &mut App) {
        let export = self.export.clone();
        let outfit = self.outfit.clone();
        if let Some((name, at)) = self.pose.clone() {
            app.insert_resource(ForcedPose(name, at));
        }
        app.add_systems(PreStartup, move |mut commands: Commands, mut clips: ResMut<Assets<AnimationClip>>, mut graphs: ResMut<Assets<AnimationGraph>>| {
            if let Some(mut c) = CharacterModel::load(&export.join("counselors").join(format!("{COUNSELOR}.json")), &export, &mut clips, &mut graphs) {
                if let Some(spec) = &outfit {
                    match c.info.choose_outfit(spec) {
                        Ok(name) => info!("counselor outfit: {name}"),
                        Err(e) => warn!("--outfit {spec}: {e}"),
                    }
                }
                // Every exported counselor's looks (counselors/<Class>_Counselor.json), on the same skeleton and
                // animations (the female counselors share them).
                let mut classes = HashMap::new();
                let mut male: Option<CharacterModel> = None;
                for e in std::fs::read_dir(export.join("counselors")).into_iter().flatten().flatten() {
                    let path = e.path();
                    let Some(class) = path.file_stem().and_then(|s| s.to_str()).and_then(|s| s.strip_suffix("_Counselor")).map(String::from) else { continue };
                    if format!("{class}_Counselor") == COUNSELOR {
                        classes.insert(class, std::sync::Arc::new(c.info.clone()));
                    } else if let Ok(i) = CharacterInfo::load(&path) {
                        if i.bones.iter().map(|b| &b.name).eq(c.info.bones.iter().map(|b| &b.name)) {
                            // The male counselors' animations: the first male class's (each male class exports the
                            // same male assets).
                            if is_male(&i) && male.is_none() {
                                male = CharacterModel::load(&path, &export, &mut clips, &mut graphs);
                            }
                            classes.insert(class, std::sync::Arc::new(i));
                        } else {
                            info!("counselor {class}: another skeleton, not playable yet");
                        }
                    }
                }
                info!("counselors: {:?}", { let mut k: Vec<_> = classes.keys().collect(); k.sort(); k });
                commands.insert_resource(CounselorCharacter(c, classes, male));
            }
            if let Some(c) = CharacterModel::load(&export.join("killers").join(format!("{KILLER}.json")), &export, &mut clips, &mut graphs) {
                commands.insert_resource(KillerCharacter(c));
            }
        })
        .add_systems(Update, (assemble, animate).chain())
        .add_systems(Update, time_dilation.after(animate))
        .add_systems(Startup, load_anim_props)
        .add_systems(PostUpdate, anim_props.before(bevy::transform::TransformSystems::Propagate))
        .init_resource::<LayerRestore>()
        .add_systems(PostUpdate, (restore_unanimated, run_lean, layered_blend, item_hand_layer, aim_offset, head_look, jason_twists, item_arm_layer, crate::blood::hide_detached).chain().after(bevy::app::AnimationSystems).before(bevy::transform::TransformSystems::Propagate));
    }
}

/// A loaded character: its description and one graph node per animation (with its length).
pub struct CharacterModel {
    pub info: CharacterInfo,
    graph: Handle<AnimationGraph>,
    clips: HashMap<String, (AnimationNodeIndex, f32)>,
    /// Animations that keep their root motion: the root bone's pose at each frame (rig space), and the length.
    roots: HashMap<String, (Vec<Transform>, f32)>,
    /// The takes played as a layer over the pose ([`LAYER_TAKES`]): their samples and length.
    layers: HashMap<String, (AnimData, f32)>,
}

/// Takes the anim Blueprints play in a layered bone blend's layer (sampled here: [`layered_blend`]): Jason_AnimBP's
/// GrabState machine's Idle (its Struggle state needs the counselor's bIsStruggling, which nothing sets: its only
/// setter, SCCharacter::SetStruggling, is called by no code or Blueprint); her block idle's CombatUpperBody track;
/// his (Jason_AnimBP's CombatUpperBody).
const LAYER_TAKES: [&str; 8] = ["normal_grab_idle", BLOCK_IDLE, crate::jason::BLOCK, AIM_EXIT, AIM_POSES[0], AIM_POSES[1], AIM_POSES[2], AIM_POSES[3]];

/// The counselor items' idle poses (their IdleAsset, a loaded gun's LoadedIdle), played in the held-item layers.
const ITEM_POSES: [&str; 12] = [
    "2x4Grip",
    "BatGrip",
    "FlareGunGrip",
    "PropGrip",
    "UniversalGrip",
    "macheteGrip",
    "pokerGrip",
    "potGrip",
    "stickGrip",
    "normal_stand_carry_heavy_01",
    "normal_stand_idle_carry_beartrap_add",
    "normal_stand_idle_hunter_01",
];
const ITEM_HAND_FILTERS: &[(&str, i32)] = &[("hand_r", 1), ("weapon_attach_r", 1)];
/// BS_Run_Lean (1D, Direction -120..120, no weight interpolation): its samples by direction (+ right of her facing;
/// the left leans at +, as the asset has them), local-space additives against frame 0 of M_normal_stand_run_forward
/// (RefPoseType ABPT_AnimFrame); cooked as full poses, so their deltas are taken here.
const LEAN_SAMPLES: [(f32, &str); 5] = [
    (-120.0, "normal_stand_sprint_lean_right"),
    (-60.0, "normal_stand_run_lean_right"),
    (0.0, "normal_stand_run_lean_neutral"),
    (60.0, "normal_stand_run_lean_left"),
    (120.0, "normal_stand_sprint_lean_left"),
];
const LEAN_BASE: &str = "M_normal_stand_run_forward";
/// SCAnimInstance's MaxLeanSpeed (cm/s) and LeanSmoothSpeed (MODLOG "Running lean").
const MAX_LEAN_SPEED: f32 = 300.0;
const LEAN_SMOOTH_SPEED: f32 = 5.0;
const ITEM_ARM_FILTERS: &[(&str, i32)] = &[("clavicle_R", 3)];
/// SCCounselorAnimInstance's ItemAlphaCurveName (its constructor 0x1402f3630).
const ITEM_ALPHA_CURVE: &str = "ItemWeightAlpha";

/// Base_Counselor_AnimBP's gun branch (Standing stance only: BlendListByEnum on CurrentStance, then bHasWeapon, then
/// bHasGun): the RangedAiming machine under the AimOffsets aim offset, all under the DefaultFullBody slot. States:
/// Idle (her locomotion) -> StartAiming (bIsAiming) -> Enter (two-handed: hunter_Aiming_idle_To_Aim) or Aiming;
/// Enter -> Aiming by its remaining time; Aiming (by GunType: the flare gun's m_normal_stand_flare_aim_idle, the
/// shotgun's combat_stand_idle_hunter_01) -> DoneAiming (!bIsAiming) -> Exit (two-handed: hunter_Aiming_To_Idle over
/// her locomotion from spine_01, BlendDepth 0, local blend) or Idle; Exit -> Idle by its remaining time. Every
/// crossfade is 0.2 s (HermiteCubic); the port eases the weights at BLEND_RATE.
const AIM_ENTER: &str = "hunter_Aiming_idle_To_Aim";
const AIM_EXIT: &str = "hunter_Aiming_To_Idle";
const AIM_IDLE_FLARE: &str = "m_normal_stand_flare_aim_idle";
const AIM_IDLE_SHOTGUN: &str = "combat_stand_idle_hunter_01";
const AIM_EXIT_FILTERS: &[(&str, i32)] = &[("spine_01", 0)];
const AIM_CROSSFADE_S: f32 = 0.2;
/// AimOffsets (AAT_RotationOffsetMeshSpace poses against Aim_C_C) at yaw 0 (the node's X is 0): Aim_B_C, Aim_C_C,
/// Aim_U_L, Aim_U_R.
const AIM_POSES: [&str; 4] = ["Aim_B_C", "Aim_C_C", "Aim_U_L", "Aim_U_R"];
/// The grid's yaw-0 column by pitch (-90, -45, 0, 45, 90): each point's sample weights.
const AIM_GRID: [[f32; 4]; 5] = [[1.0, 0.0, 0.0, 0.0], [0.5, 0.5, 0.0, 0.0], [0.0, 1.0, 0.0, 0.0], [0.0, 0.5, 0.25, 0.25], [0.0, 0.0, 0.5, 0.5]];
/// AimOffsets' TargetWeightInterpolationSpeedPerSec.
const AIM_WEIGHT_RATE: f32 = 4.0;

/// The RangedAiming machine's states (the conduits StartAiming / DoneAiming resolve at once).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum AimState {
    #[default]
    Idle,
    Enter,
    Aiming,
    Exit,
}

/// The AimOffsets samples' target weights at `pitch` (degrees; bilinear on the grid, X at 0).
fn aim_targets(pitch: f32) -> [f32; 4] {
    let y = (pitch.clamp(-90.0, 90.0) + 90.0) / 45.0;
    let (i, f) = ((y.floor() as usize).min(3), y - (y.floor()).min(3.0));
    std::array::from_fn(|k| AIM_GRID[i][k] * (1.0 - f) + AIM_GRID[i + 1][k] * f)
}

/// Base_Counselor_AnimBP's CombatUpperBody slot: a LayeredBoneBlend over her stance pose (spine_01, BlendDepth 2,
/// bMeshSpaceRotationBlend), chosen over the CombatFullBody slot while she moves (BlendListByBool on bIsMoving:
/// speed over 1e-4 cm/s, blending over 0.1 s).
const BLOCK_LAYER_FILTERS: &[(&str, i32)] = &[("spine_01", 2)];
const BLOCK_LAYER_BLEND_S: f32 = 0.1;

/// The counselor model (its skeleton and animations: Vanessa's), and every playable counselor's looks by class
/// ("Athlete", "Hero", ...).
#[derive(Resource, Deref)]
pub struct CounselorCharacter(#[deref] pub CharacterModel, pub HashMap<String, std::sync::Arc<CharacterInfo>>, pub Option<CharacterModel>);

/// The default counselor class (Vanessa).
pub const DEFAULT_CLASS: &str = "Athlete";

/// A male counselor (her looks' Gender: bIsFemale off).
#[derive(Component)]
pub struct Male;

/// A master-pose part's name (its scene's root: the outfit, Jason's mask).
#[derive(Component, Clone, Debug)]
pub struct PartOf(pub String);

/// The local player's counselor pick (--counselor).
#[derive(Resource, Clone, Debug)]
pub struct LocalClass(pub String);

/// A counselor's class ("Athlete", "Hero", ...): her looks and stats.
#[derive(Component, Clone, Debug, PartialEq, Eq)]
pub struct CounselorClass(pub String);

/// A counselor's outfit (her player's profile: wardrobe.rs), as CharacterInfo::choose_outfit takes it ("<outfit
/// class>:<option>,..."); none or empty: her class's default.
#[derive(Component, Clone, Debug, PartialEq, Eq)]
pub struct CounselorOutfit(pub String);

/// On a counselor's rig: the class and outfit it was dressed in (player.rs restyle).
#[derive(Component, Clone, Debug, PartialEq, Eq)]
pub struct Styled(pub String, pub String);

impl CounselorCharacter {
    /// A class's looks (unknown: the default's).
    pub fn looks(&self, class: &str) -> std::sync::Arc<CharacterInfo> {
        self.1.get(class).cloned().unwrap_or_else(|| std::sync::Arc::new(self.0.info.clone()))
    }

    /// A class's looks in an outfit (CounselorOutfit's spec; empty, or one that doesn't fit: her default).
    pub fn dressed(&self, class: &str, outfit: &str) -> std::sync::Arc<CharacterInfo> {
        let looks = self.looks(class);
        if outfit.is_empty() {
            return looks;
        }
        let mut l = (*looks).clone();
        match l.choose_outfit(outfit) {
            Ok(_) => std::sync::Arc::new(l),
            Err(e) => {
                warn!("counselor {class}: outfit {outfit}: {e}");
                looks
            }
        }
    }

    /// The model (the animations) a counselor plays: the male counselors' (Base_Counselor_AnimBP's male assets) or
    /// the female ones' (Vanessa's).
    pub fn model(&self, male: bool) -> &CharacterModel {
        if male {
            self.2.as_ref().unwrap_or(&self.0)
        } else {
            &self.0
        }
    }

    /// The model for a class's looks, and whether it's the male one.
    pub fn model_for(&self, looks: &CharacterInfo) -> (&CharacterModel, bool) {
        let male = is_male(looks) && self.2.is_some();
        (self.model(male), male)
    }
}

/// A counselor's looks are a male counselor's (the export's Gender: bIsFemale off).
pub fn is_male(looks: &CharacterInfo) -> bool {
    looks.names.get("Gender").is_some_and(|g| g == "Male")
}

impl CounselorCharacter {
}

#[derive(Resource, Deref)]
pub struct KillerCharacter(pub CharacterModel);

/// Which loaded model a rig shows.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Model {
    Counselor,
    Killer,
}

impl CharacterModel {
    /// Loads a character file and its animations. None if the export predates character export.
    pub fn load(path: &std::path::Path, export: &std::path::Path, clips: &mut Assets<AnimationClip>, graphs: &mut Assets<AnimationGraph>) -> Option<Self> {
        let info = match CharacterInfo::load(path) {
            Ok(i) => i,
            Err(e) => {
                warn!("no character model {} ({e}); re-export with tools/f13-export counselor / killer", path.display());
                return None;
            }
        };
        let animated: Vec<&str> = info.bones.iter().filter(|b| b.retarget == "Animation").map(|b| b.name.as_str()).collect();
        let mut graph = AnimationGraph::new();
        let mut nodes = HashMap::new();
        let mut roots = HashMap::new();
        let mut layers = HashMap::new();
        for (name, a) in &info.animations {
            let data = match AnimData::load(&export.join(&a.file)) {
                Ok(d) => d,
                Err(e) => {
                    warn!("{name}: {e}");
                    continue;
                }
            };
            if a.keep_root {
                if let Some(t) = data.tracks.iter().find(|t| t.bone == "root") {
                    let at = |i: usize| Transform {
                        translation: t.translation.as_ref().and_then(|v| v.get(i)).map_or(Vec3::ZERO, |v| Vec3::from_array(*v)),
                        rotation: t.rotation.get(i).map_or(Quat::IDENTITY, |q| Quat::from_array(*q).normalize()),
                        ..default()
                    };
                    let frames = t.rotation.len().max(t.translation.as_ref().map_or(0, |v| v.len())).max(1);
                    roots.insert(name.clone(), ((0..frames).map(at).collect(), a.length));
                }
            }
            let clip = build_clip(&data, a.length, &animated);
            nodes.insert(name.clone(), (graph.add_clip(clips.add(clip), 1.0, graph.root), a.length));
            if LAYER_TAKES.contains(&name.as_str()) || ITEM_POSES.contains(&name.as_str()) || name == LEAN_BASE || LEAN_SAMPLES.iter().any(|(_, n)| n == name) {
                layers.insert(name.clone(), (data, a.length));
            }
        }
        info!("character model {}: {} animations", info.name, nodes.len());
        Some(Self { info, graph: graphs.add(graph), clips: nodes, roots, layers })
    }

    /// The rig's place under its owner (see [`spawn_model`]).
    pub fn rig_transform(&self) -> Transform {
        let mut mesh = crate::to_transform(&self.info.mesh_transform);
        mesh.translation.y += self.info.capsule_half_height / 100.0;
        Transform::from_rotation(FACING) * mesh
    }

    /// Root motion of an animation that keeps it: its root's first and last pose as moves of the owner (identity
    /// when the root is where it normally is).
    pub fn root_motion(&self, name: &str) -> Option<(Transform, Transform)> {
        let (frames, _) = self.roots.get(name)?;
        Some((self.owner_move(&frames[0]), self.owner_move(frames.last()?)))
    }

    /// An animation's root (one that keeps it) at `t` s as a move of the owner, like [`Self::root_motion`]; its last
    /// pose once over.
    pub fn root_at(&self, name: &str, t: f32) -> Option<Transform> {
        let (frames, length) = self.roots.get(name)?;
        let x = if *length > 0.0 { (t / length).clamp(0.0, 1.0) * (frames.len() - 1) as f32 } else { 0.0 };
        let (i, f) = (x.floor() as usize, x.fract());
        let (a, b) = (frames[i], frames[(i + 1).min(frames.len() - 1)]);
        Some(self.owner_move(&Transform { translation: a.translation.lerp(b.translation, f), rotation: a.rotation.slerp(b.rotation, f), ..default() }))
    }

    fn owner_move(&self, root: &Transform) -> Transform {
        let rig = self.rig_transform().to_matrix();
        Transform::from_matrix(rig * root.to_matrix() * rig.inverse())
    }

    /// The owner transform that plays `name` (keeping its root motion) so its first frame stands at `at`, and where
    /// it leaves the character when it ends. Animations without root motion play at `at` and end there.
    pub fn place(&self, name: &str, at: Transform) -> (Transform, Transform) {
        match self.root_motion(name) {
            Some((first, last)) => {
                let origin = Transform::from_matrix(at.to_matrix() * first.to_matrix().inverse());
                (origin, flat(Transform::from_matrix(origin.to_matrix() * last.to_matrix())))
            }
            None => (at, at),
        }
    }

    /// An animation's length, if it was exported.
    pub fn length(&self, name: &str) -> Option<f32> {
        self.clips.get(name).map(|c| c.1)
    }
}

fn build_clip(data: &AnimData, length: f32, animated: &[&str]) -> AnimationClip {
    let mut clip = AnimationClip::default();
    let interval = Interval::new(0.0, length.max(1e-3)).unwrap();
    for t in &data.tracks {
        let target = AnimationTargetId::from_name(&Name::new(t.bone.clone()));
        if !t.rotation.is_empty() {
            let samples: Vec<Quat> = t.rotation.iter().map(|q| Quat::from_array(*q).normalize()).collect();
            if samples.len() >= 2 {
                let curve = SampleAutoCurve::new(interval, samples).unwrap();
                clip.add_curve_to_target(target, AnimatableCurve::new(animated_field!(Transform::rotation), curve));
            } else {
                clip.add_curve_to_target(target, AnimatableCurve::new(animated_field!(Transform::rotation), ConstantCurve::new(interval, samples[0])));
            }
        }
        if let Some(tr) = t.translation.as_ref().filter(|_| animated.contains(&t.bone.as_str())) {
            let samples: Vec<Vec3> = tr.iter().map(|v| Vec3::from_array(*v)).collect();
            if samples.len() >= 2 {
                let curve = SampleAutoCurve::new(interval, samples).unwrap();
                clip.add_curve_to_target(target, AnimatableCurve::new(animated_field!(Transform::translation), curve));
            } else {
                clip.add_curve_to_target(target, AnimatableCurve::new(animated_field!(Transform::translation), ConstantCurve::new(interval, samples[0])));
            }
        }
    }
    clip.set_duration(length);
    clip
}

/// Unreal actors face +X, characters here face +Z: the rig turns a quarter about Y on top of the mesh's own yaw, so
/// the owner's transform times this is the Unreal actor's.
pub const FACING: Quat = Quat::from_xyzw(0.0, -std::f32::consts::FRAC_1_SQRT_2, 0.0, std::f32::consts::FRAC_1_SQRT_2);

/// A character transform kept upright (yaw only, unit scale).
pub fn flat(t: Transform) -> Transform {
    let f = t.rotation * Vec3::Z;
    Transform { translation: t.translation, rotation: Quat::from_rotation_y(f.x.atan2(f.z)), scale: Vec3::ONE }
}

impl Rig {
    /// The entity holding an attached part's scene (the face, the hair), by the part's index.
    pub(crate) fn part(&self, index: usize) -> Option<Entity> {
        self.attached.iter().find(|(i, _)| *i == index).map(|(_, e)| *e)
    }
}

/// The animated model under a character: holds the AnimationPlayer every bone is animated by.
#[derive(Component)]
pub struct Rig {
    pub model: Model,
    /// Its looks when not the model's own (a counselor class's).
    pub visual: Option<std::sync::Arc<CharacterInfo>>,
    /// A male counselor: it plays the male model's animations ([`CounselorCharacter::model`]).
    pub male: bool,
    body: Entity,
    outfit: Vec<Entity>,
    /// Parts hanging off a bone or another part, spawned once the body is in: (part index, entity holding its scene).
    attached: Vec<(usize, Entity)>,
    bones_tagged: bool,
    /// The body's bones by name (filled once its scene is in).
    pub bones: HashMap<String, Entity>,
    outfit_bound: Vec<Entity>,
    /// Current weight per animation.
    pub(crate) weights: HashMap<String, f32>,
    /// Shared phase (0..1) of the synced locomotion cycles, and free-running time of the idles.
    pub(crate) phase: f32,
    /// The phase last frame (for the footstep notifies it passed).
    pub(crate) prev_phase: f32,
    idle_time: f32,
    /// The montage or action playing: its animation and time since it started.
    pub action: Option<(String, f32)>,
    /// Actions blending out: their time, still advancing (a montage instance plays on through its BlendOut).
    fading: HashMap<String, f32>,
    /// Jump straight to the next pose instead of blending (after the owner moved to where a root motion ended).
    snap: bool,
    /// Whether her last footstep was the left.
    left_last: bool,
    /// The RangedAiming machine: its state and the time in it.
    pub(crate) aim_state: AimState,
    aim_t: f32,
    /// The AimOffsets samples' weights ([`AIM_POSES`]), and whether the aim offset is in the pose.
    aim_weights: [f32; 4],
    aim_on: bool,
    /// A state's sequence player (restarts as its state is entered): its animation, time (s) and whether it loops.
    timed: Option<(&'static str, f32, bool)>,
    /// A ShotgunHold notify state is active (an animation in her pose is inside its window; items.rs).
    pub(crate) shotgun_hold: bool,
    /// Each synced cycle's time (s): the sync group's leader's, and its followers' by marker.
    sync_t: HashMap<String, f32>,
    /// A state machine crossfade in progress: time into it, its length, and the weights it started from (blended to
    /// the targets by HermiteCubic, as FAnimNode_StateMachine's transitions blend the poses).
    crossfade: Option<(f32, f32, HashMap<String, f32>)>,
    /// The running lean (SCAnimInstance's update FUN_140311d00): LeanAlpha, the smoothed world heading (degrees, Bevy
    /// yaw), LeanDirectionFiltered, and whether the lean's additive is in the pose (her standing locomotion).
    lean_alpha: f32,
    lean_heading: f32,
    lean_dir: f32,
    lean_on: bool,
    /// The held item's pose on the hand layer, and on the arm layer with its weight (UpperBodyItemAlpha).
    item_hand: Option<String>,
    item_arm: Option<(String, f32)>,
    /// The head look (SCCharacter +0x10c4: the rotator's pitch, already negated, and yaw; degrees) and
    /// TargetLookRotationAlpha (SCCounselorAnimInstance +0x624), or Jason's: the special move switch's blend progress
    /// and the grab enter switch's look weight.
    look: Vec2,
    look_alpha: f32,
    look_grab: f32,
}

impl Rig {
    /// Spawns a skeletal mesh following the body's pose (its skin bound to the body's bones once in, as the outfit's).
    pub(crate) fn follow(&mut self, commands: &mut Commands, assets: &AssetServer, rig: Entity, file: String) {
        let ready = |ev: On<WorldInstanceReady>, mut commands: Commands| {
            commands.entity(ev.entity).insert(SceneReady);
        };
        let e = commands.spawn((WorldAssetRoot(assets.load(GltfAssetLabel::Scene(0).from_asset(file))), ChildOf(rig))).observe(ready).id();
        self.outfit.push(e);
    }

    /// Puts another mesh in a master-pose part's place (`old`, its PartOf `name`), with its material map.
    pub(crate) fn swap_part(&mut self, commands: &mut Commands, assets: &AssetServer, rig: Entity, old: Entity, name: &str, file: String, map: &HashMap<String, String>) {
        let ready = |ev: On<WorldInstanceReady>, mut commands: Commands| {
            commands.entity(ev.entity).insert(SceneReady);
        };
        commands.entity(old).despawn();
        self.outfit.retain(|&e| e != old);
        self.outfit_bound.retain(|&e| e != old);
        let mut e = commands.spawn((WorldAssetRoot(assets.load(GltfAssetLabel::Scene(0).from_asset(file))), ChildOf(rig), PartOf(name.to_string())));
        if let Some(o) = crate::materials::MaterialOverrides::new(map) {
            e.insert(o);
        }
        self.outfit.push(e.observe(ready).id());
    }
}

/// The game's speed (--speed: testing), what time dilation scales.
#[derive(Resource, Clone, Copy)]
pub struct GameSpeed(pub f32);

/// SCAnimNotifyState_SloMo (NotifyBegin 0x140315540: the world settings' SetTimeDilation(SloMoSpeed), clamped to
/// Min/MaxGlobalTimeDilation (0x1417e52b0); NotifyEnd 0x14031eb30: back to 1): while a character's action is in a
/// "SloMo <speed>" window, the whole game runs at that speed (WorldSettings.TimeDilation replicates: every player's
/// game slows). Here the server (or a lone game) slows its own clock; a client's follows it: lightyear's tick sync
/// sets a client's virtual time speed every frame (to the server's slowed ticks).
pub(crate) fn time_dilation(
    speed: Option<Res<GameSpeed>>,
    counselor: Option<Res<CounselorCharacter>>,
    killer: Option<Res<KillerCharacter>>,
    rigs: Query<&Rig>,
    mut time: ResMut<Time<Virtual>>,
    mut last: Local<Option<f32>>,
    client: Option<Res<crate::net::NetClient>>,
) {
    if client.is_some() {
        return;
    }
    let mut dilation = 1.0;
    for rig in &rigs {
        let Some((name, t)) = &rig.action else { continue };
        let model = match rig.model {
            Model::Counselor => counselor.as_deref().map(|c| c.model(rig.male)),
            _ => killer.as_deref().map(|k| &k.0),
        };
        let Some(info) = model.and_then(|m| m.info.animations.get(name.as_str())) else { continue };
        for w in info.windows.iter().filter(|w| w.start <= *t && *t < w.end) {
            if let Some(v) = w.name.strip_prefix("SloMo ").and_then(|v| v.parse::<f32>().ok()) {
                dilation = v.clamp(0.0001, 20.0);
            }
        }
    }
    if *last != Some(dilation) {
        *last = Some(dilation);
        if dilation != 1.0 {
            info!("time dilation x{dilation}");
        }
        time.set_relative_speed(speed.map_or(1.0, |s| s.0) * dilation);
    }
}

/// A full-body animation a character plays now (hit reactions, grabs, kills, death), whatever its movement state.
/// Restarts when the name changes; holds its last frame once over.
#[derive(Component, Clone, Debug, PartialEq)]
pub struct PlayAction(pub String);

/// Puts the PlayAction this far in (s), once (a montage switched to another at the same fraction).
#[derive(Component)]
pub struct SeekAction(pub f32);

/// Plays the current PlayAction again from its start (a montage section that repeats its segment).
#[derive(Component)]
pub struct RestartAction;

/// A part's scene finished spawning.
#[derive(Component)]
struct SceneReady;

/// Spawns the model under `owner` (feet at its origin, facing +Z), with `marker` on the rig. Returns the rig entity.
pub fn spawn_model(commands: &mut Commands, assets: &AssetServer, character: &CharacterModel, model: Model, owner: Entity, marker: impl Bundle) -> Entity {
    spawn_model_as(commands, assets, character, None, model, owner, marker)
}

/// [`spawn_model`] with another character's looks (a counselor class's body, parts and skin masks) on the model's
/// skeleton and animations.
pub fn spawn_model_as(commands: &mut Commands, assets: &AssetServer, character: &CharacterModel, visual: Option<std::sync::Arc<CharacterInfo>>, model: Model, owner: Entity, marker: impl Bundle) -> Entity {
    let info = visual.as_deref().unwrap_or(&character.info);
    // CharacterMesh0 sits at MeshTransform from the capsule's centre; the player's origin is the capsule's bottom.
    // Unreal actors face +X, the player here faces +Z: a quarter turn about Y on top of the mesh's own yaw.
    let mut mesh = crate::to_transform(&info.mesh_transform);
    mesh.translation.y += info.capsule_half_height / 100.0;
    let rig_xf = Transform::from_rotation(FACING) * mesh;

    let ready = |ev: On<WorldInstanceReady>, mut commands: Commands| {
        commands.entity(ev.entity).insert(SceneReady);
    };
    let rig = commands
        .spawn((rig_xf, Visibility::default(), AnimationPlayer::default(), AnimationGraphHandle(character.graph.clone()), marker, ChildOf(owner)))
        .id();
    let mut body = commands.spawn((WorldAssetRoot(assets.load(GltfAssetLabel::Scene(0).from_asset(info.body.clone()))), ChildOf(rig)));
    if info.skin_masks.len() == 2 {
        body.insert(crate::materials::SkinMasks(info.skin_masks.clone()));
    }
    let body = body.observe(ready).id();
    let mut outfit = Vec::new();
    for p in info.parts.iter().filter(|p| p.master_pose()) {
        let mut e = commands.spawn((WorldAssetRoot(assets.load(GltfAssetLabel::Scene(0).from_asset(p.file.clone()))), ChildOf(rig), PartOf(p.name.clone())));
        if let Some(o) = crate::materials::MaterialOverrides::new(&p.material_map) {
            e.insert(o);
        }
        outfit.push(e.observe(ready).id());
    }
    let male = model == Model::Counselor && visual.as_deref().is_some_and(is_male);
    commands.entity(rig).insert(Rig {
        model,
        male,
        visual,
        body,
        outfit,
        attached: Vec::new(),
        bones_tagged: false,
        bones: HashMap::new(),
        outfit_bound: Vec::new(),
        weights: HashMap::new(),
        phase: 0.0,
        prev_phase: 0.0,
        idle_time: 0.0,
        action: None,
        snap: false,
        left_last: false,
        aim_state: AimState::Idle,
        aim_t: 0.0,
        aim_weights: [0.0; 4],
        aim_on: false,
        timed: None,
        fading: HashMap::new(),
        shotgun_hold: false,
        crossfade: None,
        sync_t: HashMap::new(),
        lean_alpha: 0.0,
        lean_heading: 0.0,
        lean_dir: 0.0,
        lean_on: false,
        item_hand: None,
        item_arm: None,
        look: Vec2::ZERO,
        look_alpha: 0.0,
        look_grab: 1.0,
    });
    rig
}

/// Once the body's scene is in: tags its bones as animation targets and hangs the attached parts off them; once an
/// outfit's scene is in, re-binds its skin to the body's bones.
fn assemble(
    mut commands: Commands,
    assets: Res<AssetServer>,
    counselor: Option<Res<CounselorCharacter>>,
    killer: Option<Res<KillerCharacter>>,
    mut rigs: Query<(Entity, &mut Rig)>,
    ready: Query<(), With<SceneReady>>,
    children: Query<&Children>,
    names: Query<&Name>,
    mut skins: Query<&mut bevy::mesh::skinning::SkinnedMesh>,
) {
    for (rig_entity, mut rig) in &mut rigs {
        if !ready.contains(rig.body) {
            continue;
        }
        let character = match rig.model {
            Model::Counselor => counselor.as_deref().map(|c| c.model(rig.male)),
            Model::Killer => killer.as_deref().map(|c| &c.0),
        };
        let Some(model_info) = character.map(|c| &c.info) else { continue };
        let visual = rig.visual.clone();
        let info = visual.as_deref().unwrap_or(model_info);
        let bones: HashMap<&str, Entity> = children
            .iter_descendants(rig.body)
            .filter_map(|e| names.get(e).ok().map(|n| (n.as_str(), e)))
            .filter(|(n, _)| info.bones.iter().any(|b| b.name == *n))
            .collect();
        if !rig.bones_tagged {
            for (name, &e) in &bones {
                commands.entity(e).insert((AnimationTargetId::from_name(&Name::new(name.to_string())), AnimatedBy(rig_entity)));
            }
            rig.bones_tagged = true;
            rig.bones = bones.iter().map(|(n, e)| (n.to_string(), *e)).collect();
            // Parts attached to a bone first, then parts attached to those parts.
            let mut pending: Vec<usize> = (0..info.parts.len()).filter(|&i| !info.parts[i].master_pose()).collect();
            while !pending.is_empty() {
                let before = pending.len();
                pending.retain(|&i| {
                    let p = &info.parts[i];
                    let parent = bones.get(p.attach.as_str()).copied().or_else(|| {
                        rig.attached.iter().find(|(j, _)| info.parts[*j].name == p.attach).map(|(_, e)| *e)
                    });
                    let Some(parent) = parent else { return true };
                    let xf = p.transform.as_ref().map(crate::to_transform).unwrap_or_default();
                    let mut e = commands.spawn((xf, Visibility::default(), ChildOf(parent)));
                    if !p.file.is_empty() {
                        e.insert(WorldAssetRoot(assets.load(GltfAssetLabel::Scene(0).from_asset(p.file.clone()))));
                    }
                    let e = e.id();
                    rig.attached.push((i, e));
                    false
                });
                if pending.len() == before {
                    warn!("character parts with nothing to attach to: {:?}", pending.iter().map(|&i| &info.parts[i].name).collect::<Vec<_>>());
                    break;
                }
            }
        }
        for outfit in rig.outfit.clone() {
            if rig.outfit_bound.contains(&outfit) || !ready.contains(outfit) {
                continue;
            }
            for e in children.iter_descendants(outfit) {
                let Ok(mut skin) = skins.get_mut(e) else { continue };
                for joint in skin.joints.iter_mut() {
                    if let Some(&b) = names.get(*joint).ok().and_then(|n| bones.get(n.as_str())) {
                        *joint = b;
                    }
                }
            }
            rig.outfit_bound.push(outfit);
        }
    }
}

/// Picks the target weight of each animation from the counselor's state, eases the weights toward them, and poses
/// the AnimationPlayer (every clip is positioned by hand: speed 0, seek time set here).
pub(crate) fn animate(
    mut commands: Commands,
    time: Res<Time>,
    character: Option<Res<CounselorCharacter>>,
    killer: Option<Res<KillerCharacter>>,
    tuning: Res<PlayerTuning>,
    forced: Option<Res<ForcedPose>>,
    anim_sounds: Option<Res<crate::audio::AnimSounds>>,
    players: Query<(Entity, &Player, &Transform, Option<&crate::interact::UsingItem>, Option<&PlayAction>, Has<crate::items::Blocking>, Has<RestartAction>, Option<&SeekAction>, (Option<&crate::items::GunAim>, Option<&crate::interact::Inventory>))>,
    items: Query<(&crate::props::Item, Has<crate::items::Spent>)>,
    mut rigs: Query<(&ChildOf, &mut Rig, &mut AnimationPlayer), Without<crate::ragdoll::Ragdolled>>,
    mut layers: Query<&mut LayeredBlend, With<Player>>,
) {
    let Some(character) = character else { return };
    let dt = time.delta_secs();
    if let Some(f) = forced {
        for (_, mut rig, mut anim) in &mut rigs {
            // (As the action, so its props show.)
            rig.action = Some((f.0.clone(), f.1));
            let model = match rig.model {
                Model::Counselor => character.model(rig.male),
                Model::Killer => match killer.as_deref() {
                    Some(k) => &k.0,
                    None => continue,
                },
            };
            let Some(&(node, len)) = model.clips.get(&f.0) else { continue };
            anim.play(node).set_speed(0.0).set_weight(1.0).set_seek_time(f.1.min(len));
        }
        return;
    }
    for (parent, mut rig, mut anim) in &mut rigs {
        let Ok((pe, p, xf, using, scripted, blocking, restart, seek, (gun_aim, inv))) = players.get(parent.parent()) else { continue };
        // Her animations (a male counselor's are the male ones).
        let character = character.model(rig.male);
        if restart {
            rig.action = None;
            commands.entity(pe).remove::<RestartAction>();
        }
        let c = &p.counselor;
        let t = &tuning.counselor;
        let speed = p.velocity.length();

        // A full-body action: a scripted one (hit, grab, kill, death), the montage the rules started (dodge,
        // stumble) or the first aid spray.
        let action = match (scripted, &p.montage, using) {
            (Some(a), _, _) => Some(a.0.as_str()),
            (_, Some(m), _) if p.root_motion.is_some() => female_sequence(m),
            (_, _, Some(_)) => Some(FIRST_AID),
            _ => None,
        };
        // No start or stop takes: SCAnimInstance's update (FUN_140311d00) runs its start/stop states only with
        // bUseStartStopTransitions, which nothing sets (no constructor, Blueprint or package), so CurrentMoveState is
        // just moving (3) or idle (0) and the AnimBP's Default Locomotion goes straight between Idle and Locomotion.
        rig.set_action(action, dt);
        if let Some(s) = seek {
            rig.seek_action(s.0);
            commands.entity(pe).remove::<SeekAction>();
        }
        // Paired takes (kills, the car grab: the same names on both) sound from Jason's side; her lines are hers.
        let paired = rig.action.as_ref().is_some_and(|(a, _)| a.starts_with("cinematic_"));
        crate::audio::play_anim_sounds(&mut commands, anim_sounds.as_deref(), rig.action_span(dt), xf.translation, Some(pe), Some(pe), paired);

        // The gun branch: the gun in her hands (two-handed, a flare gun), standing.
        let held = inv.and_then(|i| i.hand).and_then(|h| items.get(h).ok());
        let gun = held.and_then(|(i, _)| match &i.kind {
            crate::props::ItemKind::Gun { stats, projectile, .. } => Some((stats.two_handed, *projectile)),
            _ => None,
        });
        // The stance switch above it (BlendListByEnum on CurrentStance) has no EnumToPoseIndex: every stance takes its
        // first pose, the weapon and gun branch.
        let gun_branch = gun;
        let aiming = gun_aim.is_some_and(|a| a.aiming);
        rig.timed = None;
        match gun_branch {
            // Not relevant: the machine starts over at Idle when it is again (bReinitializeOnBecomingRelevant), and
            // the blend space's samples with it.
            None => {
                rig.aim_state = AimState::Idle;
                rig.aim_t = 0.0;
                rig.aim_weights = [0.0; 4];
            }
            Some((two_handed, _)) => {
                rig.aim_t += dt;
                let len = |n: &str| character.length(n).unwrap_or(0.0);
                let next = match rig.aim_state {
                    AimState::Idle if aiming => Some(if two_handed { AimState::Enter } else { AimState::Aiming }),
                    AimState::Enter if rig.aim_t >= len(AIM_ENTER) - AIM_CROSSFADE_S => Some(AimState::Aiming),
                    AimState::Aiming if !aiming => Some(if two_handed { AimState::Exit } else { AimState::Idle }),
                    AimState::Exit if rig.aim_t >= len(AIM_EXIT) - AIM_CROSSFADE_S => Some(AimState::Idle),
                    _ => None,
                };
                if let Some(n) = next {
                    rig.aim_state = n;
                    rig.aim_t = 0.0;
                    // Every RangedAiming transition: a 0.2 s HermiteCubic crossfade.
                    rig.crossfade = Some((0.0, AIM_CROSSFADE_S, rig.weights.clone()));
                }
                // AimPitch moves the blend space's samples toward their weights at AIM_WEIGHT_RATE each (the 4.18
                // InterpolateWeightOfSampleData), normalised.
                let target = aim_targets(gun_aim.map_or(0.0, |a| a.pitch));
                let step = AIM_WEIGHT_RATE * dt;
                for (w, t) in rig.aim_weights.iter_mut().zip(target) {
                    *w += (t - *w).clamp(-step, step);
                    if *w <= 1e-5 {
                        *w = 0.0;
                    }
                }
                let sum: f32 = rig.aim_weights.iter().sum();
                if sum > 0.0 {
                    rig.aim_weights.iter_mut().for_each(|w| *w /= sum);
                }
            }
        }
        rig.aim_on = gun_branch.is_some() && rig.action.is_none();
        // The running lean: out of combat stance (bUseStartStopTransitions is never set), LeanAlpha toward
        // Speed / MaxLeanSpeed and the world heading toward the velocity's (CalculateDirection against a zero rotator:
        // 0 when still; unwrapped toward the last) by FInterpTo at LeanSmoothSpeed; in combat stance the heading is
        // her facing and LeanAlpha 0. LeanDirectionFiltered: the heading against her facing, + right.
        let (yaw, _, _) = xf.rotation.to_euler(EulerRot::YXZ);
        let facing = yaw.to_degrees();
        if c.stance != Stance::Combat {
            rig.lean_alpha = crate::items::finterp_to(rig.lean_alpha, (speed / MAX_LEAN_SPEED).clamp(0.0, 1.0), dt, LEAN_SMOOTH_SPEED);
            let mut target = if speed > 1e-4 { p.velocity.x.atan2(p.velocity.y).to_degrees() } else { 0.0 };
            while target - rig.lean_heading > 180.0 {
                target -= 360.0;
            }
            while target - rig.lean_heading < -180.0 {
                target += 360.0;
            }
            rig.lean_heading = wrap180(crate::items::finterp_to(rig.lean_heading, target, dt, LEAN_SMOOTH_SPEED));
        } else {
            rig.lean_heading = facing;
            rig.lean_alpha = 0.0;
        }
        rig.lean_dir = wrap180(facing - rig.lean_heading);
        rig.lean_on = rig.action.is_none() && c.stance == Stance::Standing && !(gun_branch.is_some() && matches!(rig.aim_state, AimState::Enter | AimState::Aiming));
        // The held item's layers: a weapon's idle pose on her hand, a gun's in the aim machine's Idle state (its
        // LoadedIdle while loaded); anything else's on her right arm, by its montage's ItemWeightAlpha in one.
        rig.item_hand = None;
        rig.item_arm = None;
        if let Some((item, spent)) = held {
            match &item.kind {
                crate::props::ItemKind::Weapon(_) => rig.item_hand = item.idle_pose.clone(),
                crate::props::ItemKind::Gun { .. } if rig.aim_state == AimState::Idle => {
                    rig.item_hand = if spent { item.idle_pose.clone() } else { item.loaded_idle.clone().or_else(|| item.idle_pose.clone()) };
                }
                crate::props::ItemKind::Gun { .. } => {}
                _ => {
                    let w = match &rig.action {
                        Some((a, at)) => character.info.animations.get(a).and_then(|x| x.curve(ITEM_ALPHA_CURVE)).map_or(0.0, |c| c.eval(*at)),
                        None => 1.0,
                    };
                    rig.item_arm = item.idle_pose.clone().map(|p| (p, w));
                }
            }
        }
        let aim_clip = match (gun_branch, rig.aim_state) {
            (Some(_), AimState::Enter) => Some((AIM_ENTER, false)),
            (Some((_, flare)), AimState::Aiming) => Some((if flare { AIM_IDLE_FLARE } else { AIM_IDLE_SHOTGUN }, true)),
            _ => None,
        };

        let mut targets: HashMap<&str, f32> = HashMap::new();
        if rig.action.as_ref().is_some_and(|(a, _)| !character.clips.contains_key(a.as_str())) {
            targets.insert(IDLE, 1.0);
        } else if let (None, Some((clip, looping))) = (&rig.action, aim_clip) {
            targets.insert(clip, 1.0);
            rig.timed = Some((clip, rig.aim_t, looping));
        } else if rig.action.is_none() {
            match c.stance {
                Stance::Crouching => {
                    let s = (speed / t.max_walk_speed_crouched.max(1.0)).clamp(0.0, 1.0);
                    targets.insert(CROUCH_IDLE, 1.0 - s);
                    targets.insert(CROUCH_WALK, s);
                }
                Stance::Swimming => {
                    // Idle, slow at swimming speed, fast at running, sprint at sprinting (each in water).
                    let m = t.water_speed_mod;
                    let marks = [0.0, m * t.max_swim_speed, m * t.max_run_speed, m * t.max_sprint_speed];
                    let state = speed_state(speed, &marks);
                    for (k, name) in SWIM.iter().enumerate() {
                        let w = (1.0 - (state - k as f32).abs()).max(0.0);
                        if w > 0.0 {
                            *targets.entry(name).or_default() += w;
                        }
                    }
                }
                Stance::Combat => {
                    let s = (speed / t.max_combat_speed.max(1.0)).clamp(0.0, 1.0);
                    // Blocking: standing, the block idle's full-body track; moving, her combat locomotion with its
                    // upper-body track over it (the block layer, below).
                    let idle = if blocking && speed <= 1e-4 && character.clips.contains_key(BLOCK_IDLE) { BLOCK_IDLE } else { COMBAT_IDLE };
                    targets.insert(idle, 1.0 - s);
                    // Direction of travel relative to facing, + to the right (Unreal's CalculateDirection).
                    let (yaw, _, _) = xf.rotation.to_euler(EulerRot::YXZ);
                    let fwd = Vec2::new(yaw.sin(), yaw.cos());
                    let right = Vec2::new(-fwd.y, fwd.x);
                    let dir = p.velocity.dot(right).atan2(p.velocity.dot(fwd)).to_degrees();
                    let i = COMBAT_WALK.iter().position(|(a, _)| *a >= dir).unwrap_or(COMBAT_WALK.len() - 1).max(1);
                    let (a0, n0) = COMBAT_WALK[i - 1];
                    let (a1, n1) = COMBAT_WALK[i];
                    let f = ((dir - a0) / (a1 - a0)).clamp(0.0, 1.0);
                    *targets.entry(n0).or_default() += s * (1.0 - f);
                    *targets.entry(n1).or_default() += s * f;
                }
                _ => {
                    // BS_F_Normal_Move's SpeedState: 0 idle, 1 walk, 2 run, 3 sprint, linear in between.
                    let marks = [0.0, t.max_walk_speed, t.max_run_speed, t.max_sprint_speed];
                    let state = speed_state(speed, &marks);
                    let idle = if c.wounded { HURT_IDLE } else { IDLE };
                    let names = [idle, MOVE[0], MOVE[1], MOVE[2]];
                    for (k, name) in names.iter().enumerate() {
                        let w = (1.0 - (state - k as f32).abs()).max(0.0);
                        if w > 0.0 {
                            *targets.entry(name).or_default() += w;
                        }
                    }
                }
            }
        }

        let synced = |n: &str| MOVE.contains(&n) || SWIM[1..].contains(&n) || n == CROUCH_WALK || COMBAT_WALK.iter().any(|(_, m)| *m == n);
        rig.drive(character, &targets, &synced, dt, &mut anim);
        // The block layer while she blocks on the move.
        let block_layer = if blocking && c.stance == Stance::Combat && speed > 1e-4 { 1.0 } else { 0.0 };
        match layers.get_mut(pe) {
            Ok(mut l) => {
                // The aim's Exit state takes the node's place while it plays and fades (her block needs combat stance,
                // the gun branch standing).
                let exiting = gun_branch.is_some() && rig.aim_state == AimState::Exit;
                let (target, blend_s) = if exiting || (l.take == AIM_EXIT && l.weight > 0.0) {
                    if l.take != AIM_EXIT {
                        l.switch(AIM_EXIT_FILTERS, AIM_EXIT);
                        l.mesh_space = false;
                        l.hold = true;
                    }
                    (if exiting { 1.0 } else { 0.0 }, AIM_CROSSFADE_S)
                } else {
                    if l.take != BLOCK_IDLE {
                        l.switch(BLOCK_LAYER_FILTERS, BLOCK_IDLE);
                        l.mesh_space = true;
                        l.hold = false;
                    }
                    (block_layer, BLOCK_LAYER_BLEND_S)
                };
                let w = l.weight + (target - l.weight).clamp(-dt / blend_s, dt / blend_s);
                if l.weight != w {
                    l.weight = w;
                }
            }
            Err(_) => {
                commands.entity(pe).insert(LayeredBlend::new(BLOCK_LAYER_FILTERS, true, BLOCK_IDLE));
            }
        }

        // ShotgunHold windows (the hunter aim takes, the shotgun's fire) in the action, the state's sequence or the
        // Exit layer, while they weigh in.
        let in_hold = |name: &str, t: f32| character.info.animations.get(name).and_then(|a| a.window("ShotgunHold")).is_some_and(|(s, e)| t >= s && t < e);
        let len = |n: &str| character.length(n).unwrap_or(1.0).max(1e-3);
        let layer = layers.get(pe).ok().filter(|l| l.take == AIM_EXIT && l.weight > 0.0).map(|l| l.t);
        rig.shotgun_hold = rig.action.as_ref().is_some_and(|(a, t)| in_hold(a, *t))
            || rig.timed.is_some_and(|(n, t, looping)| rig.weights.get(n).is_some_and(|w| *w > 1e-4) && in_hold(n, if looping { t % len(n) } else { t }))
            || layer.is_some_and(|t| in_hold(AIM_EXIT, t));

        // The Footstep_L / Footstep_R notifies the locomotion cycle that weighs most passed this frame (audio.rs).
        let (before, after) = (rig.prev_phase, rig.phase);
        rig.prev_phase = after;
        if rig.action.is_none() && speed > 1.0 {
            let main = rig.weights.iter().filter(|(n, _)| synced(n)).max_by(|a, b| a.1.total_cmp(b.1)).map(|(n, _)| n.clone());
            // The cycle's own sounds (the swim strokes), as its phase passes them.
            if let (Some(name), Some(info)) = (&main, main.as_ref().and_then(|n| character.info.animations.get(n))) {
                let len = info.length;
                let spans: &[(f32, f32)] = if after >= before { &[(before, after)] } else { &[(before, 1.0), (-1.0, after)] };
                for &(a, b) in spans {
                    let from = if a < 0.0 { -1.0 } else { a * len };
                    crate::audio::play_anim_sounds(&mut commands, anim_sounds.as_deref(), Some((name.as_str(), from, b * len)), xf.translation, Some(pe), Some(pe), false);
                }
            }
            if let Some(info) = main.and_then(|n| character.info.animations.get(&n)) {
                for w in info.windows.iter().filter(|w| w.name == "Footstep_L" || w.name == "Footstep_R") {
                    let t = w.start / info.length.max(1e-3);
                    let passed = if after >= before { before < t && t <= after } else { t > before || t <= after };
                    if passed {
                        rig.left_last = w.name == "Footstep_L";
                        commands.trigger(crate::audio::Footstep { owner: parent.parent(), left: rig.left_last, land: false });
                    }
                }
            }
        }
    }
}

/// The animations' props (anim_props.json: SCPropSpawnerNotifyState): "killer/<take>" (Jason's) or "counselor/<take>"
/// -> each window's start and end (s in the take), its mesh (glTF), the socket or bone it's on and bHideEquippedItem.
#[derive(Resource, Default)]
pub struct AnimProps(HashMap<String, Vec<(f32, f32, String, String, bool)>>);

pub(crate) fn load_anim_props(mut commands: Commands, export: Res<crate::ExportRoot>) {
    let props = std::fs::read_to_string(export.0.join("anim_props.json")).ok().and_then(|t| serde_json::from_str(&t).ok());
    if props.is_none() {
        warn!("no anim_props.json (re-export: f13-export sounds): the animations' props aren't shown");
    }
    commands.insert_resource(AnimProps(props.unwrap_or_default()));
}

/// The props a character's action shows now (their windows' indexes and entities).
#[derive(Component, Default)]
pub struct ShownProps(String, Vec<(usize, Entity)>);

/// SCPropSpawnerNotifyState: while its window lasts, its SkeletalPropMesh on the character's mesh at SocketName (a
/// socket, else a bone of that name), snapped to it (no offset of its own); gone at its end (or when the action is).
/// bHideEquippedItem (its default: true) hides the equipped item meanwhile: Jason's Weapon part (a window with no
/// mesh only does that: the choke's, bare-handed).
#[allow(clippy::too_many_arguments)]
pub(crate) fn anim_props(
    mut commands: Commands,
    props: Option<Res<AnimProps>>,
    assets: Res<AssetServer>,
    (counselor, killer): (Option<Res<CounselorCharacter>>, Option<Res<crate::character::KillerCharacter>>),
    rigs: Query<(&ChildOf, &Rig)>,
    mut shown: Query<&mut ShownProps>,
    mut visibility: Query<&mut Visibility>,
) {
    let Some(props) = props else { return };
    for (parent, rig) in &rigs {
        let owner = parent.parent();
        let info = match rig.model {
            Model::Killer => killer.as_ref().map(|k| &k.info),
            _ => counselor.as_ref().map(|c| &c.info),
        };
        let key = rig.action.as_ref().map(|(take, _)| if matches!(rig.model, Model::Killer) { format!("killer/{take}") } else { format!("counselor/{take}") });
        let at = rig.action.as_ref().map_or(0.0, |(_, t)| *t);
        let windows = key.as_ref().and_then(|k| props.0.get(k));
        let want: Vec<usize> = windows.map_or_else(Vec::new, |w| w.iter().enumerate().filter(|(_, w)| w.0 <= at && at < w.1).map(|(i, _)| i).collect());
        // His weapon hidden while a window says so.
        if let Some(info) = info {
            let hide = windows.is_some_and(|w| want.iter().any(|&i| w[i].4));
            let weapon = rig.attached.iter().find(|(i, _)| info.parts[*i].name == "Weapon").map(|(_, e)| *e);
            if let Some(mut v) = weapon.and_then(|e| visibility.get_mut(e).ok()) {
                v.set_if_neq(if hide { Visibility::Hidden } else { Visibility::Inherited });
            }
        }
        let Ok(mut have) = shown.get_mut(owner) else {
            if !want.is_empty() {
                commands.entity(owner).insert(ShownProps::default());
            }
            continue;
        };
        let key = key.unwrap_or_default();
        // Gone: past their window, or another action's.
        let stale = have.0 != key;
        have.1.retain(|(i, e)| {
            let keep = !stale && want.contains(i);
            if !keep && *e != Entity::PLACEHOLDER {
                commands.entity(*e).despawn();
            }
            keep
        });
        have.0 = key;
        let (Some(windows), Some(info)) = (windows, info) else { continue };
        for i in want {
            if have.1.iter().any(|(j, _)| *j == i) {
                continue;
            }
            let (_, _, mesh, socket, _) = &windows[i];
            if mesh.is_empty() {
                have.1.push((i, Entity::PLACEHOLDER));
                continue;
            }
            let (bone, offset) = match info.sockets.get(socket) {
                Some(s) => (rig.bones.get(&s.attach), s.transform.as_ref().map(crate::to_transform).unwrap_or_default()),
                None => (rig.bones.get(socket), Transform::IDENTITY),
            };
            let Some(&bone) = bone else {
                warn_once!("prop {mesh}: no socket or bone {socket}");
                continue;
            };
            let e = commands.spawn((WorldAssetRoot(assets.load(GltfAssetLabel::Scene(0).from_asset(mesh.clone()))), offset, Visibility::default(), ChildOf(bone))).id();
            have.1.push((i, e));
        }
    }
}

impl Rig {
    /// A socket's world transform (its bone as last posed, times the socket's offset).
    pub fn socket(&self, info: &CharacterInfo, name: &str, globals: &Query<&GlobalTransform>) -> Option<Transform> {
        let s = info.sockets.get(name)?;
        let bone = globals.get(*self.bones.get(&s.attach)?).ok()?;
        let offset = s.transform.as_ref().map(crate::to_transform).unwrap_or_default();
        Some(bone.mul_transform(offset).compute_transform())
    }

    /// The next pose replaces the current one at once (no blend).
    pub fn snap(&mut self) {
        self.snap = true;
    }

    /// The action and the span of its time this frame (from: negative on its first frame), for its sounds.
    pub fn action_span(&self, dt: f32) -> Option<(&str, f32, f32)> {
        self.action.as_ref().map(|(a, at)| (a.as_str(), if *at == 0.0 { -1.0 } else { at - dt }, *at))
    }

    /// Puts the action playing this far in (s).
    pub fn seek_action(&mut self, at: f32) {
        if let Some((_, t)) = &mut self.action {
            *t = at;
        }
    }

    /// Starts, continues or ends the full-body action.
    pub fn set_action(&mut self, action: Option<&str>, dt: f32) {
        match (&mut self.action, action) {
            (Some((name, at)), Some(a)) if name == a => *at += dt,
            (old, Some(a)) => {
                if let Some((n, t)) = old.take() {
                    self.fading.insert(n, t);
                }
                self.fading.remove(a);
                self.action = Some((a.to_string(), 0.0));
            }
            (old, None) => {
                if let Some((n, t)) = old.take() {
                    self.fading.insert(n, t);
                }
            }
        }
    }

    /// Eases every animation's weight toward `targets` (the action, when one plays, gets all of it), advances the
    /// shared locomotion phase of the `synced` cycles, and poses the player.
    pub fn drive(&mut self, model: &CharacterModel, targets: &HashMap<&str, f32>, synced: &dyn Fn(&str) -> bool, dt: f32, anim: &mut AnimationPlayer) {
        let action_target: HashMap<&str, f32>;
        let targets = match &self.action {
            Some((a, _)) if model.clips.contains_key(a.as_str()) => {
                action_target = HashMap::from([(a.as_str(), 1.0)]);
                &action_target
            }
            _ => targets,
        };
        // A state machine crossfade (not under an action): from the weights it started with to the targets.
        if let Some((t, len, from)) = self.crossfade.as_mut().filter(|_| self.action.is_none()) {
            *t += dt;
            let a = (*t / len.max(1e-4)).clamp(0.0, 1.0);
            let a = a * a * (3.0 - 2.0 * a);
            let mut names: Vec<String> = from.keys().cloned().collect();
            names.extend(targets.keys().filter(|k| !from.contains_key(**k)).map(|k| k.to_string()));
            let done = *t >= *len;
            let from = from.clone();
            for name in names {
                let w = from.get(&name).copied().unwrap_or(0.0) * (1.0 - a) + targets.get(name.as_str()).copied().unwrap_or(0.0) * a;
                if w <= 1e-4 {
                    self.weights.remove(&name);
                } else {
                    self.weights.insert(name, w);
                }
            }
            if done {
                self.crossfade = None;
            }
        } else {
            self.crossfade = None;
        }
        // Ease every weight toward its target (absent = 0), or take them at once after a snap.
        let rate = if self.crossfade.is_some() {
            0.0
        } else if std::mem::take(&mut self.snap) {
            f32::MAX
        } else {
            (if self.action.is_some() { MONTAGE_BLEND_RATE } else { BLEND_RATE }) * dt
        };
        let mut names: Vec<String> = self.weights.keys().cloned().collect();
        for k in targets.keys() {
            if !self.weights.contains_key(*k) {
                names.push(k.to_string());
            }
        }
        for name in names {
            let target = targets.get(name.as_str()).copied().unwrap_or(0.0);
            let w = self.weights.entry(name.clone()).or_insert(0.0);
            *w += (target - *w).clamp(-rate, rate);
            if *w <= 1e-4 && target == 0.0 {
                self.weights.remove(&name);
            }
        }

        // The locomotion cycles' sync group (marker-based, as Unreal's): the highest-weighted cycle leads, advancing by
        // dt; a follower with sync markers takes the leader's place between the same two markers (its previous one
        // and the next), at the same fraction; without markers (on either), the leader's normalised time.
        let leader = self.weights.iter().filter(|(n, w)| synced(n) && **w > 1e-4).max_by(|a, b| a.1.total_cmp(b.1)).map(|(n, _)| n.clone());
        let markers = |n: &str| model.info.animations.get(n).map_or(&[][..], |a| a.sync_markers.as_slice());
        match &leader {
            Some(lead) => {
                let len = model.clips.get(lead).map_or(1.0, |c| c.1).max(1e-3);
                let t = (self.sync_t.get(lead).copied().unwrap_or(self.phase * len) + dt).rem_euclid(len);
                self.sync_t.insert(lead.clone(), t);
                self.phase = t / len;
                let at = marker_position(markers(lead), t, len);
                let followers: Vec<String> = self.weights.iter().filter(|(n, w)| synced(n) && **w > 1e-4 && *n != lead).map(|(n, _)| n.clone()).collect();
                for name in followers {
                    let flen = model.clips.get(&name).map_or(1.0, |c| c.1).max(1e-3);
                    let ft = at.as_ref().and_then(|(prev, next, f)| follow_markers(markers(&name), prev, next, *f, flen)).unwrap_or(self.phase * flen);
                    self.sync_t.insert(name, ft);
                }
            }
            None => self.sync_t.clear(),
        }
        self.idle_time += dt;
        for t in self.fading.values_mut() {
            *t += dt;
        }
        let weights = &self.weights;
        self.fading.retain(|n, _| weights.get(n).is_some_and(|w| *w > 1e-4));

        for (name, &(node, len)) in &model.clips {
            let w = self.weights.get(name).copied().unwrap_or(0.0);
            if w <= 1e-4 {
                if anim.is_playing_animation(node) {
                    anim.stop(node);
                }
                continue;
            }
            let at = if self.action.as_ref().is_some_and(|(a, _)| a == name) {
                // Loop sections (a stun's middle, a drawer's Idle) repeat; the rest hold their last frame, or the frame
                // of their FreezeAnimation notify (the anim Blueprints' AnimNotify_FreezeAnimation: Jason's
                // Montage_Pause, a counselor's Mesh.bPauseAnims).
                let t = self.action.as_ref().unwrap().1;
                let freeze = model.info.animations.get(name.as_str()).and_then(|a| a.window("FreezeAnimation")).map(|w| w.0);
                if name.contains("_loop") || name.contains("_cycle") || name.ends_with("_idle") { t % len.max(1e-3) } else { t.min(freeze.unwrap_or(len)).min(len) }
            } else if let Some(&t) = self.fading.get(name) {
                // (Blending out: on from where it was, held at its end; a loop section repeats.)
                if name.contains("_loop") || name.contains("_cycle") || name.ends_with("_idle") { t % len.max(1e-3) } else { t.min(len) }
            } else if let Some((_, t, looping)) = self.timed.filter(|(n, _, _)| n == name) {
                if looping { t % len.max(1e-3) } else { t.min(len) }
            } else if synced(name) {
                self.sync_t.get(name).copied().unwrap_or(self.phase * len)
            } else {
                self.idle_time % len.max(1e-3)
            };
            anim.play(node).set_speed(0.0).set_weight(w).set_seek_time(at);
        }
    }
}

/// Where `t` falls among a cycle's sync markers: the previous marker's name, the next one's, and the fraction of the way
/// between them (looping).
fn marker_position(markers: &[f13_data::character::Window], t: f32, len: f32) -> Option<(String, String, f32)> {
    if markers.is_empty() {
        return None;
    }
    let i = markers.iter().rposition(|m| m.start <= t).unwrap_or(markers.len() - 1);
    let prev = &markers[i];
    let next = &markers[(i + 1) % markers.len()];
    let pt = if prev.start <= t { prev.start } else { prev.start - len };
    let mut nt = next.start;
    while nt <= pt {
        nt += len;
    }
    let f = ((t - pt) / (nt - pt).max(1e-4)).clamp(0.0, 1.0);
    Some((prev.name.clone(), next.name.clone(), f))
}

/// A follower's time at that marker position: between its own marker named `prev` and the next, named `next`.
fn follow_markers(markers: &[f13_data::character::Window], prev: &str, next: &str, f: f32, len: f32) -> Option<f32> {
    let n = markers.len();
    (0..n).find(|&i| markers[i].name == prev && markers[(i + 1) % n].name == next).map(|i| {
        let a = markers[i].start;
        let mut b = markers[(i + 1) % n].start;
        while b <= a {
            b += len;
        }
        (a + f * (b - a)).rem_euclid(len)
    })
}

/// An upper-body layer over a character's pose (AnimNode_LayeredBoneBlend, one layer): `take` ([`LAYER_TAKES`],
/// looping) over the bones its branch filters pick, by `weight` (the node's BlendWeights, bound to a variable).
#[derive(Component, Clone, Debug)]
pub struct LayeredBlend {
    pub weight: f32,
    /// BranchFilters: (bone, BlendDepth).
    pub filters: &'static [(&'static str, i32)],
    /// bMeshSpaceRotationBlend.
    pub mesh_space: bool,
    /// Holds the take's last frame instead of looping.
    pub hold: bool,
    take: &'static str,
    t: f32,
}

impl LayeredBlend {
    pub fn new(filters: &'static [(&'static str, i32)], mesh_space: bool, take: &'static str) -> Self {
        LayeredBlend { weight: 0.0, filters, mesh_space, hold: false, take, t: 0.0 }
    }

    /// Another layer in the same node's place (its take, from the start, and its branch filters).
    pub fn switch(&mut self, filters: &'static [(&'static str, i32)], take: &'static str) {
        if self.take != take {
            self.take = take;
            self.t = 0.0;
        }
        self.filters = filters;
    }
}

/// FAnimationRuntime::CreateMaskWeights: each bone's weight from the branch filters, by its depth under the filter
/// bone: BlendDepth 0, the whole branch; else (depth + 1) / BlendDepth, summed over the filters and clamped to 0..1.
fn mask_weights(bones: &[f13_data::character::Bone], filters: &[(&str, i32)]) -> Vec<f32> {
    let mut w = vec![0.0f32; bones.len()];
    for &(name, blend_depth) in filters {
        let Some(f) = bones.iter().position(|b| b.name.eq_ignore_ascii_case(name)) else { continue };
        let per_depth = if blend_depth != 0 { 1.0 / blend_depth as f32 } else { 1.0 };
        for (i, wi) in w.iter_mut().enumerate() {
            // GetDepthBetweenBones: parent steps from the bone up to the filter bone (-1: not under it).
            let (mut at, mut depth) = (i as i32, 0);
            while at >= 0 && at as usize != f {
                at = bones[at as usize].parent;
                depth += 1;
            }
            if at >= 0 {
                *wi = (*wi + per_depth * (depth + 1) as f32).clamp(0.0, 1.0);
            }
        }
    }
    w
}

/// A layer take's local pose of `bone` at `t` (looping): its rotation and, for a bone that takes its translation from
/// the animation, its translation.
fn sample_take(data: &AnimData, length: f32, bone: &str, t: f32) -> Option<(Quat, Option<Vec3>)> {
    let track = data.tracks.iter().find(|tr| tr.bone == bone)?;
    let n = track.rotation.len().max(1);
    let x = if length > 0.0 && n > 1 { (t.rem_euclid(length) / length) * (n - 1) as f32 } else { 0.0 };
    let (i, f) = (x.floor() as usize, x.fract());
    let j = (i + 1).min(n - 1);
    let r = |k: usize| track.rotation.get(k).map_or(Quat::IDENTITY, |q| Quat::from_array(*q).normalize());
    let rotation = r(i).slerp(r(j), f);
    let translation = track.translation.as_ref().map(|v| {
        let p = |k: usize| v.get(k).map_or(Vec3::ZERO, |a| Vec3::from_array(*a));
        p(i).lerp(p(j), f)
    });
    Some((rotation, translation))
}

/// FQuat::FastLerp (the shortest way), normalised.
fn fast_lerp(a: Quat, b: Quat, alpha: f32) -> Quat {
    let bias = if a.dot(b) >= 0.0 { 1.0 } else { -1.0 };
    (b * alpha + a * (bias * (1.0 - alpha))).normalize()
}

/// FAnimationRuntime::BlendPosesPerBoneFilter after the animation is evaluated: each bone's pose from its locomotion
/// (the base) and the layer take by its mask weight times the layer's weight; translations blend in the bone's own
/// space, rotations in component space when bMeshSpaceRotationBlend (a bone outside the branch keeps its component
/// rotation, so its local one follows its blended parent). Bones the animation doesn't write every frame get back
/// what they had before the blend (so it doesn't build up).
#[allow(clippy::type_complexity)]
fn layered_blend(
    time: Res<Time>,
    counselor: Option<Res<CounselorCharacter>>,
    killer: Option<Res<KillerCharacter>>,
    mut owners: Query<(Entity, &mut LayeredBlend)>,
    rigs: Query<(&ChildOf, &Rig)>,
    mut transforms: Query<&mut Transform>,
    mut written: ResMut<LayerRestore>,
) {
    let dt = time.delta_secs();
    for (owner, mut l) in &mut owners {
        l.t += dt;
        if l.weight <= 0.0 {
            continue;
        }
        let Some((_, rig)) = rigs.iter().find(|(c, _)| c.parent() == owner) else { continue };
        // A full-body action (a montage in a slot over the layer, or a scripted take) has the whole pose.
        if rig.action.is_some() {
            continue;
        }
        let model = match rig.model {
            Model::Counselor => counselor.as_deref().map(|c| c.model(rig.male)),
            Model::Killer => killer.as_deref().map(|k| &k.0),
        };
        let Some(model) = model else { continue };
        let Some((data, length)) = model.layers.get(l.take) else { continue };
        let t = if l.hold { l.t.min(*length - 1e-4).max(0.0) } else { l.t };
        blend_layer(model, rig, data, *length, t, l.filters, l.mesh_space, l.weight, &mut transforms, &mut written.0);
    }
}

/// (-180, 180].
fn wrap180(a: f32) -> f32 {
    let a = (a + 180.0).rem_euclid(360.0) - 180.0;
    if a == -180.0 { 180.0 } else { a }
}

/// BS_Run_Lean's weights at `dir` (linear between its neighbouring samples).
fn lean_weights(dir: f32) -> [f32; 5] {
    let x = dir.clamp(-120.0, 120.0);
    let mut w = [0.0; 5];
    for i in 0..4 {
        let (a, b) = (LEAN_SAMPLES[i].0, LEAN_SAMPLES[i + 1].0);
        if x <= b || i == 3 {
            let f = ((x - a) / (b - a)).clamp(0.0, 1.0);
            w[i] = 1.0 - f;
            w[i + 1] = f;
            break;
        }
    }
    w
}

/// The Locomotion state's ApplyAdditive (BS_Run_Lean at LeanDirectionFiltered over her move blendspace, Alpha
/// LeanAlpha): each sample's local delta (rotation pose x base^-1, translation pose - base, for the bones retargeted
/// from the animation), blended by the blend space's weights, applied as FTransform::BlendFromIdentityAndAccumulate.
fn run_lean(counselor: Option<Res<CounselorCharacter>>, rigs: Query<&Rig>, mut transforms: Query<&mut Transform>, mut restore: ResMut<LayerRestore>) {
    let Some(cc) = counselor.as_deref() else { return };
    for rig in &rigs {
        let model = cc.model(rig.male);
        let Some((base, base_len)) = model.layers.get(LEAN_BASE) else { continue };
        if !rig.lean_on || rig.lean_alpha <= 1e-4 || !matches!(rig.model, Model::Counselor) {
            continue;
        }
        let weights = lean_weights(rig.lean_dir);
        for b in &model.info.bones {
            let Some(&e) = rig.bones.get(&b.name) else { continue };
            let Some((bq, bt)) = sample_take(base, *base_len, &b.name, 0.0) else { continue };
            let mut q = Vec4::ZERO;
            let mut t = Vec3::ZERO;
            for ((_, name), w) in LEAN_SAMPLES.iter().zip(weights) {
                if w <= 0.0 {
                    continue;
                }
                let Some((data, len)) = model.layers.get(*name) else { continue };
                let Some((sq, st)) = sample_take(data, *len, &b.name, 0.0) else { continue };
                let d = (sq * bq.inverse()).normalize();
                let d = if Vec4::from(d).dot(q) < 0.0 { -d } else { d };
                q += Vec4::from(d) * w;
                if b.retarget == "Animation" {
                    t += st.zip(bt).map_or(Vec3::ZERO, |(st, bt)| st - bt) * w;
                }
            }
            let delta = Quat::from_vec4(q).normalize();
            let Ok(mut xf) = transforms.get_mut(e) else { continue };
            let before = *xf;
            let out = Transform { rotation: (Quat::IDENTITY.slerp(delta, rig.lean_alpha) * xf.rotation).normalize(), translation: xf.translation + t * rig.lean_alpha, ..*xf };
            *xf = out;
            restore.0.entry(e).or_insert((before, out)).1 = out;
        }
    }
}

/// What the post-animation passes (the layers, the aim offset) changed this frame: each bone's value before them and
/// after the last.
#[derive(Resource, Default)]
struct LayerRestore(HashMap<Entity, (Transform, Transform)>);

/// Bones the animation didn't write since the passes last ran get back what they had before them (so they don't build
/// up frame over frame).
fn restore_unanimated(mut restore: ResMut<LayerRestore>, mut transforms: Query<&mut Transform>) {
    for (e, (before, after)) in restore.0.drain() {
        if let Ok(mut xf) = transforms.get_mut(e) {
            if *xf == after {
                *xf = before;
            }
        }
    }
}

/// One LayeredBoneBlend layer onto the rig's pose (see [`layered_blend`]): `data` at `t` (looping) over the bones the
/// branch filters pick, by `weight`.
#[allow(clippy::too_many_arguments)]
fn blend_layer(
    model: &CharacterModel,
    rig: &Rig,
    data: &AnimData,
    length: f32,
    t: f32,
    filters: &[(&str, i32)],
    mesh_space: bool,
    weight: f32,
    transforms: &mut Query<&mut Transform>,
    written: &mut HashMap<Entity, (Transform, Transform)>,
) {
    let bones = &model.info.bones;
    let mask = mask_weights(bones, filters);
    let n = bones.len();
    let (mut base, mut layer) = (vec![Transform::IDENTITY; n], vec![Transform::IDENTITY; n]);
    let mut ents = vec![None; n];
    for (i, b) in bones.iter().enumerate() {
        let Some(&e) = rig.bones.get(&b.name) else { continue };
        let Ok(xf) = transforms.get(e) else { continue };
        let current = *xf;
        ents[i] = Some(e);
        base[i] = current;
        // A bone the take doesn't key is at its reference pose.
        let mut p = match &b.local {
            Some(x) => {
                let g = x.to_gltf_space();
                Transform { translation: Vec3::from_array(g.translation), rotation: Quat::from_array(g.rotation).normalize(), scale: current.scale }
            }
            None => current,
        };
        if let Some((r, tr)) = sample_take(data, length, &b.name, t) {
            p.rotation = r;
            if let Some(tr) = tr.filter(|_| b.retarget == "Animation") {
                p.translation = tr;
            }
        }
        layer[i] = p;
    }
    // Component-space rotations (parents come first in the reference skeleton).
    let mesh = |pose: &[Transform]| {
        let mut m = vec![Quat::IDENTITY; n];
        for i in 0..n {
            let parent = bones[i].parent;
            m[i] = if parent >= 0 { m[parent as usize] * pose[i].rotation } else { pose[i].rotation };
        }
        m
    };
    let (base_mesh, layer_mesh) = if mesh_space { (mesh(&base), mesh(&layer)) } else { (vec![], vec![]) };
    let mut out_mesh = vec![Quat::IDENTITY; n];
    for i in 0..n {
        let w = (mask[i] * weight).clamp(0.0, 1.0);
        let mut out = base[i];
        out.translation = base[i].translation.lerp(layer[i].translation, w);
        if mesh_space {
            out_mesh[i] = if w <= 1e-5 { base_mesh[i] } else if w >= 1.0 - 1e-5 { layer_mesh[i] } else { fast_lerp(base_mesh[i], layer_mesh[i], w) };
            let parent = bones[i].parent;
            out.rotation = if parent >= 0 { (out_mesh[parent as usize].inverse() * out_mesh[i]).normalize() } else { out_mesh[i] };
        } else {
            out.rotation = fast_lerp(base[i].rotation, layer[i].rotation, w);
        }
        if let Some(e) = ents[i] {
            if let Ok(mut xf) = transforms.get_mut(e) {
                if *xf != out {
                    *xf = out;
                }
                written.entry(e).or_insert((base[i], out)).1 = out;
            }
        }
    }
}

/// Base_Counselor_AnimBP's held-item layers (ItemIdleAddtive: the item's idle pose, SCCounselorAnimInstance's update
/// 0x1403133b0): with a weapon, LayeredBoneBlend_0C12 (hand_r and weapon_attach_r, BlendDepth 1, local, weight 1)
/// over the stance pose; with a gun, the same in the RangedAiming machine's Idle state. Under the aim offset and the
/// DefaultFullBody slot (not during a full-body action).
fn item_hand_layer(
    time: Res<Time>,
    counselor: Option<Res<CounselorCharacter>>,
    rigs: Query<&Rig>,
    mut transforms: Query<&mut Transform>,
    mut written: ResMut<LayerRestore>,
    mut t: Local<f32>,
) {
    *t += time.delta_secs();
    let Some(cc) = counselor.as_deref() else { return };
    for rig in &rigs {
        let model = cc.model(rig.male);
        let Some(pose) = rig.item_hand.as_deref().filter(|_| rig.action.is_none() && matches!(rig.model, Model::Counselor)) else { continue };
        let Some((data, length)) = model.layers.get(pose) else { continue };
        blend_layer(model, rig, data, *length, *t, ITEM_HAND_FILTERS, false, 1.0, &mut transforms, &mut written.0);
    }
}

/// LayeredBoneBlend_DC1C at the graph's root (over every slot): with a held item that isn't a weapon, its idle pose on
/// the right arm (clavicle_R, BlendDepth 3, mesh-space rotation blend) by UpperBodyItemAlpha (1, or in a montage its
/// ItemWeightAlpha curve: 0 where the montage has none).
fn item_arm_layer(
    time: Res<Time>,
    counselor: Option<Res<CounselorCharacter>>,
    rigs: Query<&Rig>,
    mut transforms: Query<&mut Transform>,
    mut written: ResMut<LayerRestore>,
    mut t: Local<f32>,
) {
    *t += time.delta_secs();
    let Some(cc) = counselor.as_deref() else { return };
    for rig in &rigs {
        let model = cc.model(rig.male);
        let Some((pose, weight)) = rig.item_arm.as_ref().filter(|(_, w)| *w > 1e-4 && matches!(rig.model, Model::Counselor)) else { continue };
        let Some((data, length)) = model.layers.get(pose.as_str()) else { continue };
        blend_layer(model, rig, data, *length, *t, ITEM_ARM_FILTERS, true, *weight, &mut transforms, &mut written.0);
    }
}

/// The AimOffsets RotationOffsetBlendSpace over the gun branch (Alpha 1): the samples' additives blended by their
/// weights (as additive poses: weighted sums, rotations sign-aligned and normalised), then
/// FAnimationRuntime::AccumulateMeshSpaceRotationAdditiveToLocalPose: her pose's rotations to component space, the
/// additive's rotation before each, back to local; translations added. The samples (AAT_RotationOffsetMeshSpace against
/// Aim_C_C) are cooked with the additive baked into their raw data: their tracks are already the deltas (Aim_C_C keeps
/// only an identity root; a bone without a track is identity), component-space rotations (every spine delta turns
/// about the component's lateral axis: a pitch), local translations. Bones the animation doesn't write every frame get
/// back what they had before (so it doesn't build up).
#[allow(clippy::type_complexity)]
fn aim_offset(
    counselor: Option<Res<CounselorCharacter>>,
    rigs: Query<&Rig>,
    mut transforms: Query<&mut Transform>,
    mut written: ResMut<LayerRestore>,
) {
    let Some(cc) = counselor.as_deref() else { return };
    for rig in &rigs {
        let model = cc.model(rig.male);
        if !matches!(rig.model, Model::Counselor) || !rig.aim_on || rig.aim_weights[1] >= 1.0 - 1e-5 {
            continue;
        }
        let bones = &model.info.bones;
        let n = bones.len();
        // Each sample's delta per bone (translations only for the bones retargeted from the animation).
        let mut deltas: Vec<Vec<(Quat, Vec3)>> = Vec::new();
        for name in AIM_POSES {
            let Some((data, length)) = model.layers.get(name) else { return };
            deltas.push(
                bones
                    .iter()
                    .map(|b| match sample_take(data, *length, &b.name, 0.0) {
                        Some((q, tr)) => (q, tr.filter(|_| b.retarget == "Animation").unwrap_or(Vec3::ZERO)),
                        None => (Quat::IDENTITY, Vec3::ZERO),
                    })
                    .collect(),
            );
        }
        // The blended additive per bone.
        let mut delta = vec![(Quat::IDENTITY, Vec3::ZERO); n];
        for (i, d) in delta.iter_mut().enumerate() {
            let mut q = Vec4::ZERO;
            let mut t = Vec3::ZERO;
            let mut first: Option<Quat> = None;
            for (s, &w) in rig.aim_weights.iter().enumerate() {
                if w <= 0.0 {
                    continue;
                }
                let r = deltas[s][i].0;
                let r = match first {
                    Some(f) if f.dot(r) < 0.0 => -r,
                    Some(_) => r,
                    None => {
                        first = Some(r);
                        r
                    }
                };
                q += Vec4::from(r) * w;
                t += deltas[s][i].1 * w;
            }
            *d = (Quat::from_vec4(q).normalize(), t);
        }
        // Her pose.
        let mut local = vec![Transform::IDENTITY; n];
        let mut ents = vec![None; n];
        for (i, b) in bones.iter().enumerate() {
            let Some(&e) = rig.bones.get(&b.name) else { continue };
            let Ok(xf) = transforms.get(e) else { continue };
            local[i] = *xf;
            ents[i] = Some(e);
        }
        let mut now = vec![Quat::IDENTITY; n];
        for i in 0..n {
            let parent = bones[i].parent;
            now[i] = if parent >= 0 { now[parent as usize] * local[i].rotation } else { local[i].rotation };
        }
        let out_mesh: Vec<Quat> = (0..n).map(|i| delta[i].0 * now[i]).collect();
        for i in 0..n {
            let Some(e) = ents[i] else { continue };
            let parent = bones[i].parent;
            let mut out = local[i];
            out.rotation = if parent >= 0 { (out_mesh[parent as usize].inverse() * out_mesh[i]).normalize() } else { out_mesh[i] };
            out.translation += delta[i].1;
            if let Ok(mut xf) = transforms.get_mut(e) {
                if *xf != out {
                    *xf = out;
                }
                written.0.entry(e).or_insert((local[i], out)).1 = out;
            }
        }
    }
}

/// SCCounselorAnimInstance's MaxLookAtFear (its constructor; no Blueprint sets it).
const MAX_LOOK_AT_FEAR: f32 = 50.0;
/// TargetLookRotationAlpha's FInterpTo speed (the anim update FUN_1403133b0).
const LOOK_ALPHA_SPEED: f32 = 4.0;
/// The head look's RInterpTo speed (DAT_14207a20c).
const LOOK_INTERP_SPEED: f32 = 3.0;
/// Jason_AnimBP's look switches' blend times: BlendListByBool_3F6B (bIsInSpecialMove, back to the look) and
/// BlendListByBool_F4E4 (bIsGrabEnterPlaying).
const JASON_LOOK_SPECIAL_BLEND_S: f32 = 0.1;
const JASON_LOOK_GRAB_BLEND_S: f32 = 0.5;
/// Within this of side-on (|cos| of the look against her facing, DAT_14209aa60) the head looks ahead.
const LOOK_SIDE_COS: f32 = 0.2;
/// ClampAngle limits (DAT_1420f5954..60): yaw +-55, pitch +-25 (from 0 when the look is behind her).
const LOOK_MAX_YAW: f32 = 55.0;
const LOOK_MAX_PITCH: f32 = 25.0;

/// FRotator::NormalizeAxis.
fn normalize_axis(a: f32) -> f32 {
    let a = a.rem_euclid(360.0);
    if a > 180.0 { a - 360.0 } else { a }
}

/// FMath::ClampAngle.
fn clamp_angle(angle: f32, min: f32, max: f32) -> f32 {
    let max_delta = (max - min).rem_euclid(360.0) * 0.5;
    let center = (min + max_delta).rem_euclid(360.0);
    let d = normalize_axis(angle - center);
    if d > max_delta {
        normalize_axis(center + max_delta)
    } else if d < -max_delta {
        normalize_axis(center - max_delta)
    } else {
        normalize_axis(angle)
    }
}

/// UKismetMathLibrary::RInterpTo on (pitch, yaw) (roll stays 0).
fn rinterp_to(current: Vec2, target: Vec2, dt: f32, speed: f32) -> Vec2 {
    if dt == 0.0 || current == target {
        return current;
    }
    let delta = Vec2::new(normalize_axis(target.x - current.x), normalize_axis(target.y - current.y));
    if delta.abs().max_element() <= 1e-4 {
        return target;
    }
    let m = current + delta * (speed * dt).clamp(0.0, 1.0);
    Vec2::new(normalize_axis(m.x), normalize_axis(m.y))
}

/// FRotator::Quaternion in Unreal's axes, then to the rig's (glTF: (x, z, y), rotation (-x, -z, -y, w)).
fn rotator_quat(pitch: f32, yaw: f32, roll: f32) -> Quat {
    gltf_ue(rotator_quat_ue(pitch, yaw, roll))
}

/// FRotator::Quaternion, Unreal's axes.
fn rotator_quat_ue(pitch: f32, yaw: f32, roll: f32) -> Quat {
    let (sp, cp) = (pitch.to_radians() * 0.5).sin_cos();
    let (sy, cy) = (yaw.to_radians() * 0.5).sin_cos();
    let (sr, cr) = (roll.to_radians() * 0.5).sin_cos();
    let x = cr * sp * sy - sr * cp * cy;
    let y = -cr * sp * cy - sr * cp * sy;
    let z = cr * cp * sy - sr * sp * cy;
    let w = cr * cp * cy + sr * sp * sy;
    Quat::from_xyzw(x, y, z, w)
}

/// The look rotator's target (SCCharacter's tick helper FUN_1402ecdf0, locally controlled): `d` the direction from
/// the eyes to the view location and `fwd` the actor's forward, both in the mesh's component space in Unreal's axes.
/// Flattened against the forward: within 0.2 of side-on none, behind the flat direction mirrored (where the camera
/// looks), in front the full one (at the camera); its rotator against the actor's, yaw ClampAngle'd to +-55, pitch to
/// +-25 (0..25 behind) and negated. (pitch, yaw), degrees.
fn look_target(d: Vec3, fwd: Vec3) -> Vec2 {
    let flat = Vec3::new(d.x, d.y, 0.0).normalize_or_zero();
    let cos = flat.dot(fwd);
    if cos.abs() < LOOK_SIDE_COS {
        return Vec2::ZERO;
    }
    let dir = if cos < 0.0 { -flat } else { d };
    let yaw = normalize_axis(dir.y.atan2(dir.x).to_degrees() - fwd.y.atan2(fwd.x).to_degrees());
    let pitch = normalize_axis(dir.z.atan2(dir.truncate().length()).to_degrees());
    let min_pitch = if cos >= 0.0 { -LOOK_MAX_PITCH } else { 0.0 };
    Vec2::new(-clamp_angle(pitch, min_pitch, LOOK_MAX_PITCH), clamp_angle(yaw, -LOOK_MAX_YAW, LOOK_MAX_YAW))
}

/// FAnimNode_ModifyBone with BMM_Additive rotation in component space: `rotation` before the bone's component
/// rotation, blended with it by `alpha` (FastLerp); the bone's local rotation follows (its children keep theirs).
/// Returns the bone and its transform before.
fn modify_bone_additive(model: &CharacterModel, rig: &Rig, bone: &str, rotation: Quat, alpha: f32, transforms: &mut Query<&mut Transform>) -> Option<(Entity, Transform)> {
    let bones = &model.info.bones;
    let index = bones.iter().position(|b| b.name == bone)?;
    let mut chain = Vec::new();
    let mut i = bones[index].parent;
    while i >= 0 {
        chain.push(i as usize);
        i = bones[i as usize].parent;
    }
    let mut parent_cs = Quat::IDENTITY;
    for &b in chain.iter().rev() {
        parent_cs *= rig.bones.get(&bones[b].name).and_then(|e| transforms.get(*e).ok())?.rotation;
    }
    let e = *rig.bones.get(bone)?;
    let mut xf = transforms.get_mut(e).ok()?;
    let before = *xf;
    let cs = parent_cs * before.rotation;
    xf.rotation = (parent_cs.inverse() * fast_lerp(cs, rotation * cs, alpha)).normalize();
    Some((e, before))
}

/// The head look (SCCharacter's tick helper FUN_1402ecdf0, [`look_target`]) RInterpTo'd at 3 into +0x10c4; the anim
/// update (FUN_140311d00 0x140312045) makes it TargetLookAtRotation (Pitch 0, Yaw, Roll = the negated pitch) and the
/// graphs' additive component-space ModifyBones take it:
/// - Base_Counselor_AnimBP: ModifyBone_B9E86 on neck_01 (after the aim offset, under the Cinematic slot) by
///   TargetLookRotationAlpha (FUN_1403133b0: FInterpTo at 4 toward 0 in a special move, stunned, out of the standing
///   stance or at MaxLookAtFear, else 1);
/// - Jason_AnimBP: neck_01 then spine_02 by GrabWeight_Neck01 / GrabWeight_Spine02 (SCKillerAnimInstance's update
///   0x140314310: 1 / 0 at GrabWeight 1, else 0.6 / 0.4), switched off by BlendListByBool_F4E4 on
///   bIsGrabEnterPlaying (0.5 s, linear) under BlendListByBool_3F6B on bIsInSpecialMove (at once; 0.1 s ExpOut back),
///   both under the Cinematic slot.
/// The view location is the camera's (ILLCharacter +0x7d0, cached by CalcCamera 0x14025ac71), the eyes BaseEyeHeight
/// above the capsule's centre (GetActorEyesViewPoint). An AI Jason looks at his AIController's focal point, which
/// only his path following sets (no game Blueprint calls SetFocus; UE's move focus is 100 m ahead along the path):
/// straight ahead, so no look.
#[allow(clippy::type_complexity, clippy::too_many_arguments)]
fn head_look(
    time: Res<Time>,
    counselor: Option<Res<CounselorCharacter>>,
    killer: Option<Res<KillerCharacter>>,
    play_jason: Option<Res<crate::jason_player::PlayJason>>,
    players: Query<(&Player, &GlobalTransform, Option<&PlayAction>)>,
    jasons: Query<(&crate::jason::Jason, &GlobalTransform)>,
    cameras: Query<&GlobalTransform, With<crate::player::PlayerCamera>>,
    mut rigs: Query<(&ChildOf, &mut Rig, &GlobalTransform)>,
    mut transforms: Query<&mut Transform>,
    mut written: ResMut<LayerRestore>,
) {
    let Ok(camera) = cameras.single() else { return };
    let dt = time.delta_secs();
    for (parent, mut rig, rig_xf) in &mut rigs {
        let (model, owner_xf) = match rig.model {
            Model::Counselor => match (counselor.as_deref(), players.get(parent.parent())) {
                (Some(c), Ok((_, xf, _))) => (c.model(rig.male), xf),
                _ => continue,
            },
            Model::Killer => match (killer.as_deref(), jasons.get(parent.parent())) {
                (Some(k), Ok((_, xf))) if play_jason.is_some() => (&k.0, xf),
                _ => continue,
            },
        };
        // Into the mesh's component space, Unreal's axes.
        let inv = rig_xf.rotation().inverse();
        let ue = |v: Vec3| {
            let l = inv * v;
            Vec3::new(l.x, l.z, l.y)
        };
        let eyes = owner_xf.translation() + Vec3::Y * (model.info.capsule_half_height / 100.0 + crate::interact::BASE_EYE_HEIGHT_M);
        let target = look_target(ue(camera.translation() - eyes).normalize_or_zero(), ue(owner_xf.rotation() * Vec3::Z));
        rig.look = rinterp_to(rig.look, target, dt, LOOK_INTERP_SPEED);
        let cinematic = rig.action.as_ref().is_some_and(|(a, _)| a.starts_with("cinematic_"));
        let rotation = rotator_quat(0.0, rig.look.y, rig.look.x);
        match rig.model {
            Model::Counselor => {
                let Ok((p, _, scripted)) = players.get(parent.parent()) else { continue };
                // PLACEHOLDER: IsInSpecialMove / IsStunned as a scripted action (special moves, hits, grabs) or hiding.
                let off = scripted.is_some() || p.hidden || p.counselor.stance != Stance::Standing || p.fear >= MAX_LOOK_AT_FEAR;
                rig.look_alpha = crate::items::finterp_to(rig.look_alpha, if off { 0.0 } else { 1.0 }, dt, LOOK_ALPHA_SPEED);
                if rig.look_alpha <= 1e-4 || cinematic {
                    continue;
                }
                if let Some((e, before)) = modify_bone_additive(model, &rig, "neck_01", rotation, rig.look_alpha, &mut transforms) {
                    let Ok(out) = transforms.get(e).copied() else { continue };
                    written.0.entry(e).or_insert((before, out)).1 = out;
                }
            }
            Model::Killer => {
                let Ok((j, _)) = jasons.get(parent.parent()) else { continue };
                // The outer switch: off at once, back over 0.1 s ExpOut (its progress in look_alpha); the inner: 0.5 s
                // linear (its weight in look_grab).
                rig.look_alpha = if j.in_special_move() { 0.0 } else { (rig.look_alpha + dt / JASON_LOOK_SPECIAL_BLEND_S).min(1.0) };
                let grab_enter = rig.action.as_ref().is_some_and(|(a, _)| crate::jason::Jason::grab_enter_playing(a));
                let step = dt / JASON_LOOK_GRAB_BLEND_S;
                rig.look_grab = (rig.look_grab + if grab_enter { -step } else { step }).clamp(0.0, 1.0);
                let outer = if rig.look_alpha >= 1.0 { 1.0 } else { 1.0 - 2f32.powf(-10.0 * rig.look_alpha) };
                let w = outer * rig.look_grab;
                if w <= 1e-4 || cinematic {
                    continue;
                }
                let (neck, spine) = if j.holding() { (1.0, 0.0) } else { (0.6, 0.4) };
                let mut changed = Vec::new();
                changed.extend(modify_bone_additive(model, &rig, "neck_01", rotation, neck, &mut transforms));
                changed.extend(modify_bone_additive(model, &rig, "spine_02", rotation, spine, &mut transforms));
                // The switches blend the looked pose with the one before it, in local space.
                for (e, before) in changed {
                    let Ok(mut xf) = transforms.get_mut(e) else { continue };
                    xf.rotation = fast_lerp(before.rotation, xf.rotation, w);
                    let out = *xf;
                    written.0.entry(e).or_insert((before, out)).1 = out;
                }
            }
        }
    }
}

/// Jason_AnimBP's BoneDrivenControllers (over his look, under the Cinematic slot): each lowerarm_twist_01 takes half
/// its hand's local roll against the reference pose (SourceComponent RotationX, Multiplier 0.5, ReplaceComponent on
/// RotationX). Their source and target bones.
const JASON_TWISTS: [(&str, &str); 2] = [("hand_l", "lowerarm_twist_01_l"), ("hand_r", "lowerarm_twist_01_r")];
const JASON_TWIST_MULTIPLIER: f32 = 0.5;

/// FQuat::Rotator (UE 4: the singularities at +-90 pitch), Unreal's axes: (pitch, yaw, roll), degrees.
fn quat_rotator(q: Quat) -> Vec3 {
    let (x, y, z, w) = (q.x, q.y, q.z, q.w);
    let singularity = z * x - w * y;
    let yaw_y = 2.0 * (w * z + x * y);
    let yaw_x = 1.0 - 2.0 * (y * y + z * z);
    let yaw = yaw_y.atan2(yaw_x).to_degrees();
    if singularity < -0.4999995 {
        Vec3::new(-90.0, yaw, normalize_axis(-yaw - 2.0 * x.atan2(w).to_degrees()))
    } else if singularity > 0.4999995 {
        Vec3::new(90.0, yaw, normalize_axis(yaw - 2.0 * x.atan2(w).to_degrees()))
    } else {
        let pitch = (2.0 * singularity).clamp(-1.0, 1.0).asin().to_degrees();
        let roll = (-2.0 * (w * x + y * z)).atan2(1.0 - 2.0 * (x * x + y * y)).to_degrees();
        Vec3::new(pitch, yaw, roll)
    }
}

/// The rig's (glTF) rotation in Unreal's axes and back (the same swap both ways).
fn gltf_ue(q: Quat) -> Quat {
    Quat::from_xyzw(-q.x, -q.z, -q.y, q.w)
}

/// FAnimNode_BoneDrivenController (EvaluateSkeletalControl 0x140f3ad50, its source value 0x140f47460): the source
/// bone's local rotation against its reference pose (current x ref^-1), FQuat::Euler's X (roll), times the
/// Multiplier; ReplaceComponent (mode 1) puts it in the target's local Euler X (FQuat::Euler, then MakeFromEuler).
#[allow(clippy::type_complexity)]
fn jason_twists(killer: Option<Res<KillerCharacter>>, rigs: Query<&Rig>, mut transforms: Query<&mut Transform>, mut written: ResMut<LayerRestore>) {
    let Some(model) = killer.as_deref().map(|k| &k.0) else { return };
    for rig in &rigs {
        if !matches!(rig.model, Model::Killer) || rig.action.as_ref().is_some_and(|(a, _)| a.starts_with("cinematic_")) {
            continue;
        }
        for (source, target) in JASON_TWISTS {
            let Some(reference) = model.info.bones.iter().find(|b| b.name == source).and_then(|b| b.local.as_ref()) else { continue };
            let reference = Quat::from_array(reference.r).normalize();
            let (Some(&se), Some(&te)) = (rig.bones.get(source), rig.bones.get(target)) else { continue };
            let Ok(current) = transforms.get(se).map(|x| gltf_ue(x.rotation)) else { continue };
            // FQuat::Euler is (roll, pitch, yaw): its X is the rotator's roll.
            let value = quat_rotator(current * reference.inverse()).z * JASON_TWIST_MULTIPLIER;
            let Ok(mut xf) = transforms.get_mut(te) else { continue };
            let before = *xf;
            let r = quat_rotator(gltf_ue(before.rotation));
            xf.rotation = gltf_ue(rotator_quat_ue(r.x, r.y, value)).normalize();
            let out = *xf;
            written.0.entry(te).or_insert((before, out)).1 = out;
        }
    }
}

/// Position of `speed` along the blendspace's speed marks (0 at marks[0], 1 at marks[1], ...).
fn speed_state(speed: f32, marks: &[f32]) -> f32 {
    for i in 1..marks.len() {
        if speed <= marks[i] {
            let span = (marks[i] - marks[i - 1]).max(1e-3);
            return (i - 1) as f32 + ((speed - marks[i - 1]) / span).clamp(0.0, 1.0);
        }
    }
    (marks.len() - 1) as f32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn head_look_math() {
        // As FRotationMatrix: yaw turns Unreal's X toward its Y (the rig's +Z), roll its Y toward -Z (the rig's -Y).
        let q = rotator_quat(0.0, 90.0, 0.0);
        assert!((q * Vec3::X - Vec3::Z).length() < 1e-5);
        let q = rotator_quat(0.0, 0.0, 90.0);
        assert!((q * Vec3::Z + Vec3::Y).length() < 1e-5);
        assert_eq!(clamp_angle(80.0, -55.0, 55.0), 55.0);
        assert_eq!(clamp_angle(-170.0, 0.0, 25.0), 25.0);
        assert_eq!(clamp_angle(10.0, -25.0, 25.0), 10.0);
        // FQuat::Rotator undoes FRotator::Quaternion.
        for (p, y, r) in [(10.0, -40.0, 25.0), (-60.0, 170.0, -90.0), (0.0, 0.0, 120.0)] {
            let back = quat_rotator(rotator_quat_ue(p, y, r));
            assert!((back - Vec3::new(p, y, r)).abs().max_element() < 1e-2, "{back:?}");
        }
        let r = rinterp_to(Vec2::ZERO, Vec2::new(0.0, 170.0), 0.1, 3.0);
        assert!((r.y - 51.0).abs() < 1e-3);
    }

    #[test]
    fn followers_take_the_leaders_marker_phase() {
        let m = |v: &[(&str, f32)]| v.iter().map(|(n, t)| f13_data::character::Window { name: n.to_string(), start: *t, end: *t }).collect::<Vec<_>>();
        let walk = m(&[("LeftPlant", 0.1), ("RightPlant", 0.7)]);
        let run = m(&[("LeftPlant", 0.15), ("RightPlant", 0.6)]);
        // Halfway from the walk's LeftPlant to its RightPlant: halfway on the run.
        let (p, n, f) = marker_position(&walk, 0.4, 1.3).unwrap();
        assert_eq!((p.as_str(), n.as_str()), ("LeftPlant", "RightPlant"));
        assert!((f - 0.5).abs() < 1e-5);
        assert!((follow_markers(&run, &p, &n, f, 0.8).unwrap() - 0.375).abs() < 1e-5);
        // Past the last marker: from RightPlant around to LeftPlant.
        let (p, n, f) = marker_position(&walk, 1.0, 1.3).unwrap();
        assert_eq!((p.as_str(), n.as_str()), ("RightPlant", "LeftPlant"));
        assert!((f - 0.3 / 0.7).abs() < 1e-4);
        let t = follow_markers(&run, &p, &n, f, 0.8).unwrap();
        assert!((t - (0.6 + 0.35 * 0.3 / 0.7)).abs() < 1e-4);
    }

    #[test]
    fn lean_weights_are_linear_between_samples() {
        assert_eq!(lean_weights(0.0), [0.0, 0.0, 1.0, 0.0, 0.0]);
        assert_eq!(lean_weights(30.0), [0.0, 0.0, 0.5, 0.5, 0.0]);
        assert_eq!(lean_weights(-90.0), [0.5, 0.5, 0.0, 0.0, 0.0]);
        assert_eq!(lean_weights(200.0), [0.0, 0.0, 0.0, 0.0, 1.0]);
        assert_eq!(wrap180(190.0), -170.0);
    }

    #[test]
    fn aim_targets_follow_the_grid() {
        assert_eq!(aim_targets(0.0), [0.0, 1.0, 0.0, 0.0]);
        assert_eq!(aim_targets(-90.0), [1.0, 0.0, 0.0, 0.0]);
        assert_eq!(aim_targets(90.0), [0.0, 0.0, 0.5, 0.5]);
        assert_eq!(aim_targets(-22.5), [0.25, 0.75, 0.0, 0.0]);
        assert_eq!(aim_targets(67.5), [0.0, 0.25, 0.375, 0.375]);
    }

    #[test]
    fn speed_state_is_piecewise_linear() {
        let marks = [0.0, 100.0, 300.0, 500.0];
        assert_eq!(speed_state(0.0, &marks), 0.0);
        assert_eq!(speed_state(50.0, &marks), 0.5);
        assert_eq!(speed_state(200.0, &marks), 1.5);
        assert_eq!(speed_state(500.0, &marks), 3.0);
        assert_eq!(speed_state(900.0, &marks), 3.0);
    }

    #[test]
    fn every_montage_the_rules_play_has_a_female_take() {
        for m in ["AM_combat_stand_Dodge_Left", "AM_combat_stand_Dodge_Right", "AM_combat_stand_dodge_back", "AM_normal_stand_run_stumble_short", "AM_normal_stand_sprint_stumble_short"] {
            assert!(female_sequence(m).is_some(), "{m}");
        }
    }
}

