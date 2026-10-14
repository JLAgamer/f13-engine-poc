//! Placed (environment) kills (SCContextKillActor: Wall_FaceSmash_PlacedKill, Table_Placedkill, ...): Jason holding
//! a counselor uses one he's at: he takes its ContextKillComponent's place and facing (InteractorPositionRequirement
//! LocationAndRotation), the two play its special moves' paired montages (InteractorSpecialClass Jason_*,
//! VictimSpecialClass Counselor_*: bDieOnFinish), and its own camera spline (under the ContextKillComponent) films
//! it. Most can be used once (bOnlyUseableOnce). Their KillInfo scores Jason's Resourceful.
//!
//! Reach (the Interactable's range test, FUN_14022e940, vtable slot 0x518): his actor's location (his capsule's centre)
//! within DistanceLimit (ILLInteractableComponent's 200 cm unless set) of the Interactable, in 3D; with bUseYawLimit,
//! the direction from it to him within MaxYawOffset (15 unless set) of its facing, and (his manager's
//! bUseCameraRotation, the constructor's true) his view within MaxYawOffset of facing back at it; with bUsePitchLimit
//! the same in pitch against MaxPitchOffset (90 unless set; here his view's pitch is taken as level). Its
//! bCheckForOcclusion isn't set on any placed kill.
//!
//! The bird bath, the water pump and the chair animate as the kill starts (their Blueprints' BeginPlay sets the
//! KillMesh and KillMeshAnimation, SetAnimatedKillMesh 0x14035a8f0, which pauses it; KillStarted 0x14034d180 plays it
//! once and pauses it again at its end); the bird bath's Blueprint also turns its mesh to (0, 0, -90) on KillStarted.
//! Placed kills come from the level and from the cabins' spawners (runtime actors, props.rs).
//!
//! bShouldOrientTowardKiller (the bird bath, the tree and light post arm ripoffs, the campfire stick, the stump slam):
//! SCContextKillActor's tick (FUN_140361770) turns the actor, while the local killer holds a counselor outside a
//! context kill within its Interactable's HighlightDistance (flat), to face along the line from him to it (yaw only),
//! so its kill spot, camera and Interactable come round to him. Its IgnoredRotationComponents (each Blueprint's
//! visible mesh: BirdBathSM, the trees, CampFire, Cylinder) get the actor's negated rotation as their relative one.
//! Not ported: those meshes' own turn (to world yaw 0) as the actor turns.
//!
//! Its prompt (ui/prompts.rs) when Jason is played: SCContextKillActor's CanInteractWith (vtable 0x5f0,
//! FUN_140339dc0): not used up (0x381) and his CanGrabKill (0x1403e6f10: bGrabKillsActive with a counselor held), then
//! its ContextKillComponent's (FUN_14033a130: enabled, bIsGrabKill wanting the same). Highlighted by the Interactable's
//! highlight test (its HighlightDistance: 20 to 40 m on these) from his capsule's centre, occluded by what blocks its
//! channel between his eyes (BaseEyeHeight 64 cm up) and its point; the best when he can use it ([`PlacedKill::in_reach`]).

use bevy::prelude::*;
use f13_data::classes::ClassDb;
use f13_data::level::Xform;

pub struct PlacedKillsPlugin;

impl Plugin for PlacedKillsPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, spawn_placed_kills)
            .add_systems(FixedUpdate, orient_toward_killer.before(crate::jason::update_jason).run_if(crate::net::simulating))
            .add_systems(Update, (prompts, door_prompts, counselor_prompts));
    }
}

/// Each kind: its actor class, Jason's take, hers, and its KillInfo's KillName.
pub const KINDS: [(&str, &str, &str, &str); 15] = [
    ("Birdbath_PlacedKill_C", "cinematic_hand_birdbath", "cinematic_hand_birdbath", "Bird Bath"),
    ("HeadSmash_PlacedKill_C", "cinematic_f_gravestone_headsmash_01", "cinematic_gravestone_headsmash", "Head Smash"),
    ("IronFence_PlacedKill_C", "cinematic_hand_ironfence", "cinematic_hand_ironfence", "Iron Fence"),
    ("TreeArmRipoff_PlacedKill_C", "cinematic_hand_tree_arm_ripoff", "cinematic_hand_tree_arm_ripoff", "Tree Arm Ripoff"),
    ("LightPostArmRipoff_PlacedKill_C", "cinematic_hand_tree_arm_ripoff", "cinematic_hand_tree_arm_ripoff", "Tree Arm Ripoff"),
    ("Vertical_BackBreak_PlacedKil_C", "cinematic_surface_vertical_backbreak", "cinematic_surface_vertical_backbreak", "Vertical Back Break"),
    ("Wall_FaceSmash_PlacedKill_C", "cinematic_m_wall_facesmash_01", "cinematic_wall_facesmash", "Wall Face Smash"),
    ("WaterPump_PlacedKill_C", "cinematic_hand_waterpump", "cinematic_hand_waterpump", "Water Pump"),
    ("AdirondackChair_PlacedKill_C", "cinematic_f_adirondackchair_kill_01", "cinematic_adirondackchair_kill", "Chair Kill"),
    ("Stick_Campfire_PlacedKill_C", "cinematic_stick_campfire", "cinematic_stick_campfire", "Campfire"),
    ("TreeStumpSlam_PlacedKill_C", "cinematic_hand_treestump_faceslam", "cinematic_hand_treestump_faceslam", "Tree Stump"),
    ("Fireplace_PlacedKill_C", "cinematic_fireplace", "cinematic_fireplace", "Fireplace"),
    ("Table_Placedkill_C", "cinematic_hand_table", "cinematic_hand_table", "Table Smash"),
    ("TreeBranch_PlacedKill_C", "cinematic_treebranch_chesthole_01", "cinematic_treebranch_chesthole_01", "Tree Branch"),
    ("WallHeadPunch_PlacedKill_C", "cinematic_wall_headpunch_01", "cinematic_wall_headpunch_01", "Wall Head Punch"),
];

/// ILLInteractableComponent's constructor: DistanceLimit (cm), MaxYawOffset and MaxPitchOffset (degrees).
const DEFAULT_REACH_CM: f32 = 200.0;
const DEFAULT_MAX_YAW: f64 = 15.0;
const DEFAULT_MAX_PITCH: f64 = 90.0;

/// A placed kill's actor: where the level put it, where it is now (turned toward Jason: bShouldOrientTowardKiller),
/// its components' places in it (Bevy: the ContextKillComponent, the camera spline, the Interactable), and how near
/// he must be for it to turn (HighlightDistance, m).
#[derive(Debug, Clone)]
pub struct KillActor {
    pub root: Transform,
    pub now: Transform,
    kill: Transform,
    camera: Transform,
    interact: Transform,
    pub orient_m: Option<f32>,
}

/// A placed kill in the level.
#[derive(Component, Debug)]
pub struct PlacedKill {
    pub actor: KillActor,
    /// Index into [`KINDS`].
    pub kind: usize,
    /// Its kill camera's key (kill_cameras.json: the actor's package name).
    pub camera_key: String,
    /// Where Jason stands for it (his transform: the ContextKillComponent's, turned to face as characters do).
    pub spot: Transform,
    /// Its camera spline's frame (world).
    pub camera: Transform,
    /// Its Interactable: where, facing which way, how far it reaches (m), and its yaw and pitch limits (degrees).
    pub interact: Vec3,
    pub forward: Vec3,
    pub reach_m: f32,
    pub yaw_limit: Option<f32>,
    pub pitch_limit: Option<f32>,
    pub only_once: bool,
    pub used: bool,
    /// Its Interactable's prompt and highlight test.
    pub prompt: KillPrompt,
    /// Its KillMesh, its KillMeshAnimation, and the transform its Blueprint gives the mesh as the kill starts.
    pub prop: Option<(Entity, &'static str, Option<Transform>)>,
}

/// A placed kill's Interactable's prompt (InteractionWidgetClass, InteractionIconWidgetClass) and highlight test
/// (FUN_140240fb0: bCanHighlight, HighlightDistance, HighlightZDistance with bUseSeperateZBounds, YawHighlightLimit with
/// bUseYawHighlightLimit; ILLInteractableComponent's constructor's 500 cm, 100 cm and 90 degrees unless set), and its
/// occlusion trace (bCheckForOcclusion, SCInteractComponent's constructor's true; RayTraceChannel).
#[derive(Debug, Clone)]
pub struct KillPrompt {
    pub widget: String,
    pub icon: Option<String>,
    pub can_highlight: bool,
    pub distance_m: f32,
    pub z_m: Option<f32>,
    pub yaw: Option<f32>,
    pub occlusion: bool,
    pub visibility: bool,
}

/// The placed kills whose prop plays its own animation as the kill starts: the class, its KillMesh component, its
/// KillMeshAnimation, and the relative rotation (pitch, yaw, roll) its Blueprint sets on KillStarted.
pub const KILL_PROPS: [(&str, &str, &str, Option<[f32; 3]>); 3] = [
    ("Birdbath_PlacedKill_C", "BirdBathSM", "cinematic_hand_birdbath", Some([0.0, 0.0, -90.0])),
    ("WaterPump_PlacedKill_C", "WaterPumpSM", "A_Interactive_WaterPump_01_Kill", None),
    ("AdirondackChair_PlacedKill_C", "AdirondackChair", "cinematic_f_adirondackchair_kill_01", None),
];

/// A placed kill of `class` at `t` (its actor's own properties `actor_props`): None for other classes. `prop` is the
/// entity of its KillMesh component, when it has one.
pub fn placed_kill(classes: &ClassDb, class: &str, t: &Xform, actor_props: &serde_json::Value, prop: impl Fn(&str) -> Option<Entity>) -> Option<PlacedKill> {
    let kind = KINDS.iter().position(|k| k.0 == class)?;
    let info = classes.get(class)?;
    let comp = |name: &str| info.components.iter().find(|c| c.name == name);
    let local = |c: &f13_data::level::ComponentInfo| crate::to_transform(&c.local.unwrap_or(Xform::IDENTITY));
    let kill = info.components.iter().find(|c| c.kind == "SCContextKillComponent")?;
    let camera = info.components.iter().find(|c| c.kind == "SCCameraSplineComponent")?;
    let interact = comp("Interactable")?;
    let root = crate::to_transform(t);
    let orient = info.props["bShouldOrientTowardKiller"].as_bool().unwrap_or(false);
    let actor = KillActor {
        root,
        now: root,
        kill: local(kill),
        camera: local(camera),
        interact: local(interact),
        orient_m: orient.then(|| interact.props["HighlightDistance"].as_f64().unwrap_or(0.0) as f32 / 100.0),
    };
    let prop = KILL_PROPS.iter().find(|p| p.0 == class).and_then(|&(_, component, anim, rotation)| {
        let at_start = rotation.zip(comp(component)).map(|(r, c)| {
            let x = c.local.unwrap_or(Xform::IDENTITY);
            crate::to_transform(&Xform { r: f13_core::camera_spline::quaternion(Vec3::from_array(r)).to_array(), ..x })
        });
        Some((prop(component)?, anim, at_start))
    });
    let mut k = PlacedKill {
        actor,
        kind,
        camera_key: class.trim_end_matches("_C").to_string(),
        spot: Transform::IDENTITY,
        camera: Transform::IDENTITY,
        interact: Vec3::ZERO,
        forward: Vec3::X,
        reach_m: interact.props["DistanceLimit"].as_f64().map_or(DEFAULT_REACH_CM, |d| d as f32) / 100.0,
        yaw_limit: interact.props["bUseYawLimit"].as_bool().unwrap_or(false).then(|| interact.props["MaxYawOffset"].as_f64().unwrap_or(DEFAULT_MAX_YAW) as f32),
        pitch_limit: interact.props["bUsePitchLimit"].as_bool().unwrap_or(false).then(|| interact.props["MaxPitchOffset"].as_f64().unwrap_or(DEFAULT_MAX_PITCH) as f32),
        only_once: actor_props["bOnlyUseableOnce"].as_bool().or(info.props["bOnlyUseableOnce"].as_bool()).unwrap_or(false),
        used: false,
        prompt: KillPrompt::of(classes, interact),
        prop,
    };
    k.place(root);
    Some(k)
}

impl KillPrompt {
    /// An Interactable's prompt and highlight test from its component's properties (the constructors' defaults
    /// unless set; bCanHighlight's is off).
    pub fn of(classes: &ClassDb, interact: &f13_data::level::ComponentInfo) -> Self {
        let p = &interact.props;
        let f = |k: &str, d: f32| p[k].as_f64().map_or(d, |v| v as f32);
        let class = |k: &str| p[k].as_str().map(|s| s.trim_end_matches("_C").to_string());
        KillPrompt {
            widget: class("InteractionWidgetClass").unwrap_or_else(|| "SCInteractionWidget".into()),
            icon: class("InteractionIconWidgetClass"),
            can_highlight: p["bCanHighlight"].as_bool().unwrap_or(false),
            distance_m: f("HighlightDistance", 500.0) / 100.0,
            z_m: p["bUseSeperateZBounds"].as_bool().unwrap_or(false).then(|| f("HighlightZDistance", 100.0) / 100.0),
            yaw: p["bUseYawHighlightLimit"].as_bool().unwrap_or(false).then(|| f("YawHighlightLimit", 90.0)),
            occlusion: p["bCheckForOcclusion"].as_bool().unwrap_or(classes.is_child_of(&interact.kind, "SCInteractComponent")),
            visibility: p["RayTraceChannel"].as_str() == Some("ECC_Visibility"),
        }
    }

    /// The highlight test (FUN_140240fb0, after bCanHighlight) of the Interactable at `at` facing `forward`, for a
    /// character at `center`.
    pub fn highlights(&self, at: Vec3, forward: Vec3, center: Vec3) -> bool {
        if !self.can_highlight {
            return false;
        }
        let d = center - at;
        let near = match self.z_m {
            Some(z) => d.y.abs() <= z && Vec2::new(d.x, d.z).length() <= self.distance_m,
            None => d.length() <= self.distance_m,
        };
        // The direction from it to him (yaw) within the limit of its facing.
        let off = |a: f32| ((a + 180.0).rem_euclid(360.0) - 180.0).abs();
        near && self.yaw.is_none_or(|l| off(d.z.atan2(d.x).to_degrees() - forward.z.atan2(forward.x).to_degrees()) <= l)
    }
}

impl PlacedKill {
    /// Puts the actor at `now`: its kill spot (turned to face as characters do), camera and Interactable with it.
    pub fn place(&mut self, now: Transform) {
        self.actor.now = now;
        let spot = now.mul_transform(self.actor.kill);
        self.spot = Transform { rotation: spot.rotation * Quat::from_rotation_y(std::f32::consts::FRAC_PI_2), ..spot };
        self.camera = now.mul_transform(self.actor.camera);
        let iw = now.mul_transform(self.actor.interact);
        self.interact = iw.translation;
        self.forward = (iw.rotation * Vec3::X).normalize_or(Vec3::X);
    }

    /// The kill mesh's place (under the actor's entity, which stays where the level put it) as KillStarted sets its
    /// relative transform: relative to the actor as it is now.
    pub fn prop_start(&self, relative: Transform) -> Transform {
        Transform::from_matrix(self.actor.root.to_matrix().inverse() * self.actor.now.mul_transform(relative).to_matrix())
    }

    /// Jason, his capsule's centre at `center` and looking along `view` (ground plane), can use it.
    pub fn in_reach(&self, center: Vec3, view: Vec2) -> bool {
        if self.used && self.only_once {
            return false;
        }
        let d = center - self.interact;
        if d.length() > self.reach_m {
            return false;
        }
        // Yaw and pitch (degrees) of a direction, as Unreal's rotator of it.
        let yaw = |v: Vec3| v.z.atan2(v.x).to_degrees();
        let pitch = |v: Vec3| v.y.atan2(Vec2::new(v.x, v.z).length()).to_degrees();
        let off = |a: f32| ((a + 180.0).rem_euclid(360.0) - 180.0).abs();
        let back = -self.forward;
        let view = Vec3::new(view.x, 0.0, view.y);
        let within = |v: Vec3, f: Vec3| {
            self.yaw_limit.is_none_or(|l| off(yaw(v) - yaw(f)) <= l) && self.pitch_limit.is_none_or(|l| off(pitch(v) - pitch(f)) <= l)
        };
        within(d, self.forward) && (view == Vec3::ZERO || within(view, back))
    }

    /// The highlight test's distance and facing parts for a character whose capsule's centre is at `center`.
    pub fn highlights(&self, center: Vec3) -> bool {
        !(self.used && self.only_once) && self.prompt.highlights(self.interact, self.forward, center)
    }

    pub fn takes(&self) -> (&'static str, &'static str, &'static str) {
        let (_, his, hers, name) = KINDS[self.kind];
        (his, hers, name)
    }
}

/// bShouldOrientTowardKiller (FUN_140361770): Jason holding a counselor (not in a kill) within HighlightDistance (flat)
/// of the actor turns it to face along the line from him to it.
fn orient_toward_killer(jasons: Query<(&crate::jason::Jason, &Transform)>, mut kills: Query<&mut PlacedKill>) {
    let Some((_, jxf)) = jasons.iter().find(|(j, _)| j.holding()) else { return };
    for mut k in &mut kills {
        let Some(reach) = k.actor.orient_m else { continue };
        let d = k.actor.root.translation - jxf.translation;
        let flat = Vec2::new(d.x, d.z);
        if flat.length() > reach || flat == Vec2::ZERO {
            continue;
        }
        // MakeRotFromX of the flat line: the actor's Unreal +X (Bevy +X) along it.
        let rotation = Quat::from_rotation_y((-flat.y).atan2(flat.x));
        if k.actor.now.rotation.angle_between(rotation) > 1e-4 {
            let now = Transform { rotation, ..k.actor.root };
            k.place(now);
        }
    }
}

/// Jason's placed kill prompts when he's played (see the module docs).
#[allow(clippy::type_complexity)]
fn prompts(
    play: Option<Res<crate::jason_player::PlayJason>>,
    controls: Option<Res<crate::jason_player::JasonControls>>,
    killer: Option<Res<crate::character::KillerCharacter>>,
    jasons: Query<(&crate::jason::Jason, &Transform)>,
    kills: Query<(Entity, &PlacedKill, Option<&Name>)>,
    mut list: ResMut<crate::ui::Prompts>,
    occlusion: Occlusion,
) {
    use crate::ui::{Prompt, PromptSource};
    let enter = killer.as_ref().and_then(|k| k.length(crate::jason::GRAB_ENTER)).unwrap_or(0.0);
    // CanGrabKill: holding her, his grab's enter played out (bGrabKillsActive).
    let holder = jasons.iter().find(|(j, _)| j.holding_for().is_some_and(|t| t >= enter));
    let (Some(_), Some((_, xf))) = (play, holder) else {
        list.set(PromptSource::PlacedKill, []);
        return;
    };
    let half = killer.as_ref().map_or(0.88, |k| k.info.capsule_half_height / 100.0);
    let center = xf.translation + Vec3::Y * half;
    let eyes = center + Vec3::Y * JASON_EYE_HEIGHT_M;
    let view = controls.as_ref().map_or(Vec2::ZERO, |c| Vec2::new(-c.yaw.sin(), -c.yaw.cos()));
    let occluded = |e: Entity, k: &PlacedKill, name: Option<&Name>| occlusion.blocked(&k.prompt, e, name, eyes, k.interact);
    // The best: the one he'd use (update_jason takes the first in reach).
    let best = kills.iter().find(|(_, k, _)| k.in_reach(center, view)).map(|(e, _, _)| e);
    let prompts: Vec<Prompt> = kills
        .iter()
        .filter_map(|(e, k, name)| {
            let highlighted = k.highlights(center) && !occluded(e, k, name);
            let is_best = best == Some(e);
            (highlighted || is_best).then(|| Prompt {
                source: PromptSource::PlacedKill,
                id: (e, 0),
                at: k.interact,
                widget: k.prompt.widget.clone(),
                icon: k.prompt.icon.clone(),
                highlighted,
                best: is_best,
                shown: vec![],
                hold: None,
            })
        })
        .collect();
    list.set(PromptSource::PlacedKill, prompts);
}

/// IsOccluded (bCheckForOcclusion): what blocks the Interactable's channel (RayTraceChannel) between the character's
/// eyes and its point, the actor's own collision aside.
#[derive(bevy::ecs::system::SystemParam)]
pub struct Occlusion<'w, 's> {
    collision: Option<Res<'w, crate::collision::WorldCollision>>,
    dynamic: Res<'w, crate::collision::DynamicColliders>,
    no_world_dynamic: Query<'w, 's, (), With<crate::collision::NoWorldDynamic>>,
    no_visibility: Query<'w, 's, (), With<crate::collision::NoVisibility>>,
    actor_tris: Option<Res<'w, crate::collision::ActorTris>>,
    not_visibility: Option<Res<'w, crate::collision::NotVisibility>>,
    not_world_dynamic: Option<Res<'w, crate::collision::NotWorldDynamic>>,
}

impl Occlusion<'_, '_> {
    /// Whether `prompt`'s Interactable on `actor` (named `name`) at `at` is occluded from `eyes`.
    pub fn blocked(&self, prompt: &KillPrompt, actor: Entity, name: Option<&Name>, eyes: Vec3, at: Vec3) -> bool {
        let Some(world) = self.collision.as_ref().filter(|_| prompt.occlusion) else { return false };
        let passes = |c: Entity| if prompt.visibility { self.no_visibility.contains(c) } else { self.no_world_dynamic.contains(c) };
        let (grid, lists) = world.without(&self.dynamic, |c| passes(c) || c == actor);
        let own = name.and_then(|n| self.actor_tris.as_ref()?.0.get(n.as_str()));
        let glass = if prompt.visibility { self.not_visibility.as_ref().map(|g| &g.0) } else { self.not_world_dynamic.as_ref().map(|g| &g.0) };
        let level = crate::collision::Excluding(grid, own);
        f13_core::collision::segment_hit(&(&crate::collision::Excluding(&level, glass), lists.as_slice()), eyes, at, 1.0).is_some()
    }
}

/// An interactable's prompt for the played Jason: a door's InteractComponent (SCInteractionDoorWidget with
/// DoorInteractionIcons: SCDoor's constructor 0x140329a10 sets its bCanHighlight (+0x358) and HighlightDistance 300,
/// the Blueprints' 400 over it), a throwing knife pickup's Interactable (SCInteractionWidget with
/// KnifePickupInteractionIcon: the Blueprint's bCanHighlight, HighlightDistance 2000).
#[derive(Component, Debug, Clone)]
pub struct InteractPrompt(pub KillPrompt);

/// The played Jason's door and throwing knife prompts. SCDoor's CanInteractWith (0x140339f10) gives the killer a closed door (not broken)
/// while he isn't holding her; highlighted by the door's highlight test from his capsule's centre, occluded as the
/// placed kills' are; the best his interactable manager's ([`crate::jason::Jason::focus_door`]). SCInteractionDoorWidget
/// (its Blueprint's Get_ActionMapWidgetUse / Get_ActionMapWidgetAttack_Visibility_0, for a killer): the attack key
/// when the door's locked or barricaded and his rage isn't on, else the use key.
#[allow(clippy::type_complexity)]
fn door_prompts(
    play: Option<Res<crate::jason_player::PlayJason>>,
    killer: Option<Res<crate::character::KillerCharacter>>,
    jasons: Query<(&crate::jason::Jason, &Transform, Option<&crate::senses::Knowledge>)>,
    doors: Query<(Entity, &crate::doors::Door, &InteractPrompt, &GlobalTransform, Option<&Name>)>,
    (knives, knife_prompts, limits, controls): (
        Query<(Entity, &crate::props::KnifePickup, &GlobalTransform, Has<crate::props::PickedUp>)>,
        Query<(&InteractPrompt, Option<&Name>)>,
        Query<&crate::interact::InteractLimits>,
        Option<Res<crate::jason_player::JasonControls>>,
    ),
    barricades: Query<&crate::windows::Barricade>,
    mut list: ResMut<crate::ui::Prompts>,
    occlusion: Occlusion,
) {
    use crate::ui::{Prompt, PromptSource};
    let (Some(_), Some((j, xf, know))) = (play, jasons.iter().next()) else {
        list.set(PromptSource::Door, []);
        return;
    };
    if j.holding() {
        list.set(PromptSource::Door, []);
        return;
    }
    let half = killer.as_ref().map_or(0.88, |k| k.info.capsule_half_height / 100.0);
    let center = xf.translation + Vec3::Y * half;
    let eyes = center + Vec3::Y * JASON_EYE_HEIGHT_M;
    let rage = know.is_some_and(|k| k.abilities.rage_active);
    let best = j.focus_door();
    let mut prompts: Vec<Prompt> = doors
        .iter()
        .filter(|(_, d, ..)| !d.open && !d.broken)
        .filter_map(|(e, d, p, g, name)| {
            let at = g.transform_point(d.interact);
            let forward = g.rotation() * Vec3::X;
            let highlighted = p.0.highlights(at, forward, center) && !occlusion.blocked(&p.0, e, name, eyes, at);
            let is_best = best == Some(e);
            let barred = barricades.iter().any(|b| b.enabled && b.door == Some(e));
            let attack = (d.locked || barred) && !rage;
            (highlighted || is_best).then(|| Prompt {
                source: PromptSource::Door,
                id: (e, 0),
                at,
                widget: p.0.widget.clone(),
                icon: p.0.icon.clone(),
                highlighted,
                best: is_best,
                shown: vec![("ActionMapWidgetUse".into(), !attack), ("ActionMapWidgetAttack".into(), attack)],
                hold: None,
            })
        })
        .collect();
    // The throwing knife pickups: the best the one his E takes.
    let look = controls.as_ref().map_or(xf.rotation * Vec3::Z, |c| Quat::from_euler(EulerRot::YXZ, c.yaw, c.pitch, 0.0) * Vec3::NEG_Z);
    let best_knife = crate::jason::knife_pickup_at(&knives, &limits, center, look);
    for (e, k, g, taken) in &knives {
        if taken {
            continue;
        }
        let Ok((p, name)) = knife_prompts.get(e) else { continue };
        let at = g.transform_point(k.interact);
        let highlighted = p.0.highlights(at, g.rotation() * Vec3::X, center) && !occlusion.blocked(&p.0, e, name, eyes, at);
        let is_best = best_knife == Some(e);
        if highlighted || is_best {
            prompts.push(Prompt { source: PromptSource::Door, id: (e, 0), at, widget: p.0.widget.clone(), icon: p.0.icon.clone(), highlighted, best: is_best, shown: vec![], hold: None });
        }
    }
    list.set(PromptSource::Door, prompts);
}

/// APawn's BaseEyeHeight (Jason_Base doesn't set it): his eyes above his capsule's centre.
const JASON_EYE_HEIGHT_M: f32 = 0.64;

fn spawn_placed_kills(mut commands: Commands, level: Res<crate::Level>, export: Res<crate::ExportRoot>, assets: Res<AssetServer>) {
    let Ok(classes) = ClassDb::load(&export.0.join("classes.json")) else { return };
    let (mut n, mut props) = (0, 0);
    for a in level.layouts.iter().flat_map(|l| &l.actors) {
        if !KINDS.iter().any(|k| k.0 == a.class) {
            continue;
        }
        let (Some(t), Some(info)) = (a.transform, classes.get(&a.class)) else { continue };
        let root = commands.spawn((crate::to_transform(&t), Visibility::default(), Name::new(a.name.clone()))).id();
        // Its skeletal meshes (the level's placements hold only its static ones).
        let mut parts = std::collections::HashMap::new();
        for c in info.components.iter().filter(|c| c.kind.contains("SkeletalMeshComponent") && !c.hidden) {
            let Some(mesh) = &c.mesh else { continue };
            let scene = assets.load(GltfAssetLabel::Scene(0).from_asset(mesh.clone()));
            let local = crate::to_transform(&c.local.unwrap_or(Xform::IDENTITY));
            parts.insert(c.name.clone(), commands.spawn((bevy::world_serialization::WorldAssetRoot(scene), local, ChildOf(root))).id());
            props += 1;
        }
        match placed_kill(&classes, &a.class, &t, &a.props, |c| parts.get(c).copied()) {
            Some(k) => {
                commands.entity(root).insert(k);
                n += 1;
            }
            None => {
                commands.entity(root).despawn();
            }
        }
    }
    info!("placed kills: {n} ({props} skeletal props)");
}

/// The local counselor's prompt: her interactable manager's best (her interaction target, interact.rs) shows its
/// InteractionWidgetClass with its InteractionIconWidgetClass (the actor's interactable component, from its class),
/// at its interaction point. A door's SCInteractionDoorWidget shows a counselor its Use key, not its Attack
/// (Get_ActionMapWidgetUse / Attack_Visibility_0). Her hold on it without pips fills its hold ring (SCInteractWidget's
/// tick: the hold's progress into DynamicHoldMaterial's Alpha; ShowHoldRing only for an interactable without pips).
/// PLACEHOLDER: only the best shows (the others' highlight previews within their HighlightDistance aren't listed for
/// her), and the ring's progress is her hold's (vt+0xd68 not read further).
fn counselor_prompts(
    export: Res<crate::ExportRoot>,
    mut classes: Local<Option<ClassDb>>,
    play: Option<Res<crate::jason_player::PlayJason>>,
    local: Query<&crate::interact::Interaction, (With<crate::controls::LocalPlayer>, With<crate::player::Player>)>,
    actors: Query<(&GlobalTransform, Option<&Name>)>,
    tuning: Res<crate::player::PlayerTuning>,
    mut list: ResMut<crate::ui::Prompts>,
) {
    use crate::ui::{Prompt, PromptSource};
    let target = local.iter().next().filter(|_| play.is_none()).and_then(|i| Some((i, i.prompt_target()?)));
    let Some((interaction, (e, which))) = target else {
        list.set(PromptSource::Counselor, []);
        return;
    };
    let classes = classes.get_or_insert_with(|| ClassDb::load(&export.0.join("classes.json")).unwrap_or_default());
    let Ok((gxf, name)) = actors.get(e) else {
        list.set(PromptSource::Counselor, []);
        return;
    };
    let Some(info) = name.and_then(|n| classes.get(n.as_str())) else {
        list.set(PromptSource::Counselor, []);
        return;
    };
    let interactables: Vec<_> = info.components.iter().filter(|c| classes.is_child_of(&c.kind, "ILLInteractableComponent")).collect();
    let Some((i, comp)) = interactables.iter().enumerate().find(|(_, c)| which.is_none_or(|w| c.name == w)).or_else(|| interactables.iter().enumerate().next()) else {
        list.set(PromptSource::Counselor, []);
        return;
    };
    let kp = KillPrompt::of(classes, comp);
    let at = gxf.transform_point(crate::to_transform(&comp.local.unwrap_or(Xform::IDENTITY)).translation);
    let hold = interaction.hold.as_ref().filter(|h| h.started && h.pips.is_empty()).map(|h| h.progress(&tuning.minigame).clamp(0.0, 1.0));
    let shown = if kp.widget == "SCInteractionDoorWidget" { vec![("ActionMapWidgetUse".into(), true), ("ActionMapWidgetAttack".into(), false)] } else { vec![] };
    list.set(PromptSource::Counselor, [Prompt { source: PromptSource::Counselor, id: (e, i as u8), at, widget: kp.widget, icon: kp.icon, highlighted: true, best: true, shown, hold }]);
}
