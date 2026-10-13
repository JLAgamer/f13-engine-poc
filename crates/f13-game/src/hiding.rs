//! Hiding spots (SCHidingSpot: wardrobes and barn closets, outhouses, beds, tents).
//!
//! From the game:
//! - A counselor uses a spot through its `Interactable` component (within its `DistanceLimit`) when nobody is in it
//!   and it isn't destroyed (`CanCounselorInteractWith`). She plays the enter special move's montage
//!   (`Counselor_EnterArmoire`: AM_normal_stand_hidespot_closet_enter; beds have a left and a right side), from the
//!   special move component's spot, its root motion carrying her in; the montage's HideIdle section loops while she
//!   hides and its HideExit section takes her out (pressing interact again, `OnExitHidingPressed`).
//! - While hiding (`bInHiding`) she pushes the HiddenFearEvent, her flashlight goes off, and the view is the spot's
//!   `Hiding Camera` (its FieldOfView), turned at most `MaxHidingRotationOffset` degrees each way.
//! - Jason uses the spot's `KillerInteractable`: with a counselor inside he plays the hiding spot kill
//!   (`HidingSpotKill`, by his weapon: the pickaxe uses the axe's), else the search (`SearchComponent`, by weapon),
//!   after which the spot is destroyed (`bDestroyOnSearch`, constructor default true; tents keep it false) when
//!   `bDestructable`. Destroyed spots can't be used. jason.rs drives his side.
//!
//! Holding her breath (f13-core::breath): Space while hidden holds it (CSLR_HoldBreath), letting go releases it; her
//! breath values are Base_Counselor's (MaxBreath, thresholds, BreathHold/RechargeModifier). Her voice lines
//! (VoiceOverObjectClass): HoldBreath / HoldBreathEnd as she holds and lets go while hiding; KillerIsNearWhileHiding
//! when Jason is in her view near her spot while she hides not holding her breath, could hold it, and her fear is at
//! least 70 (the voice component's check).
//!
//! The voice component's constructor (SCVoiceOverComponent_Counselor, unreflected fields): the scared line plays
//! with Jason within 3 m, then not again for 20 s. Out of hiding she screams (Screams) as he comes into view within
//! 23 m, once, until he's out of view and farther than that, and then not for 30 s.
//! AI choice: Jason hears the scared line within its audible range (inner radius plus falloff) and learns her hiding
//! spot (the game's Jason is a player who hears it). Holding her breath doesn't change
//! her noise here: she's still while hidden, so it's 0 (BreathNoiseMod only lowers a moving counselor's).
//!
//! Doors (the wardrobe's two, the outhouse's), as Armoire_C / Outhouse_C drive them: while she enters, their Door
//! Driver follows the enter montage's "Driver: OpenCloseDoor" window (0 to 3.1 s) and turns the left door (DoorPivot;
//! the outhouse's Mesh) to 90 x the montage's DoorRotation curve, then sets it back to 0. As she comes out
//! (BlueprintExitStarted) ExitOuthouseDoorTimeline swings them open (the right door the other way) and, when it ends,
//! CloseDoorTimeline closes them. Nothing else moves them: Jason's search and kill don't (the Blueprints' search and
//! kill events only set the DestructionFXDriver's owner and turn the hiding camera off, and no spot's
//! WeaponSpecificSearchAnims is set, so HideSpotSearchStart plays nothing on its mesh). Barn closets' doors have no
//! pivots and don't move. The tent flaps are prop_anim.rs's.
//! The hidden counselor's model is hidden (the camera is inside the spot).

use bevy::prelude::*;
use f13_data::classes::ClassDb;
use f13_data::level::ComponentInfo;

use crate::character::{flat, CounselorCharacter, PlayAction, Rig};
use crate::player::{Player, PlayerCamera};

/// The scared hiding line: Jason within this many metres, and then not for this many seconds (the voice component's
/// constructor: 300 cm, a 20 s timer).
const SCARED_LINE_M: f32 = 3.0;
const SCARED_LINE_SECONDS: f32 = 20.0;
/// Her scream as Jason comes into view: within this many metres; re-armed after he's out of view and farther, and
/// then not for this many seconds (2300 cm, a 30 s timer).
const SCREAM_M: f32 = 23.0;
const SCREAM_SECONDS: f32 = 30.0;
/// SCCounselorCharacter's voice component: the scared hiding line needs this much fear.
const SCARED_LINE_FEAR: f32 = 70.0;

/// Her breath (f13-core::breath) and when her scared hiding line may play again.
#[derive(Component)]
pub struct Breathing {
    pub breath: f13_core::breath::Breath,
    scared_next: f32,
    screamed: bool,
    scream_next: f32,
}

#[derive(Resource)]
pub(crate) struct BreathTuning(pub(crate) f13_core::breath::BreathTuning);

/// Jason's weapon class: it picks the weapon-specific search and kill moves.
pub const KILLER_WEAPON: &str = "PickAxe_C";
/// The enter montages' "Driver: OpenCloseDoor" window (both start at 0).
const DOOR_DRIVER_SECONDS: f32 = 3.1;
/// The timelines' lengths (TimelineLength).
const EXIT_TIMELINE_SECONDS: f32 = 0.3;
const CLOSE_TIMELINE_SECONDS: f32 = 0.7;

/// Curve keys: time, value, arrive and leave tangents, linear (else cubic).
type Keys = &'static [(f32, f32, f32, f32, bool)];

fn rich(keys: Keys) -> f13_core::curve::RichCurve {
    use f13_core::curve::{Interp, Key};
    let keys = keys
        .iter()
        .map(|&(time, value, arrive_tangent, leave_tangent, linear)| Key { time, value, interp: if linear { Interp::Linear } else { Interp::Cubic }, arrive_tangent, leave_tangent })
        .collect();
    f13_core::curve::RichCurve { keys }
}

/// A spot's door curves: the enter montage's DoorRotation (0..1) and its Blueprint's exit and close timelines
/// (DoorRotation, degrees).
struct DoorCurves {
    enter: f13_core::curve::RichCurve,
    exit: f13_core::curve::RichCurve,
    close: f13_core::curve::RichCurve,
}

/// AM_normal_stand_hidespot_closet_enter's and Armoire_C's.
static ARMOIRE_DOORS: std::sync::LazyLock<DoorCurves> = std::sync::LazyLock::new(|| DoorCurves {
    enter: rich(&[
        (0.2, 0.0, 0.0, 0.0, false), (0.3, 0.03, 0.5, 0.5, false), (0.4, 0.1, 1.0, 1.0, false), (0.5, 0.23, 1.45, 1.45, false),
        (0.6, 0.39, 1.65, 1.65, false), (0.7, 0.56, 1.55, 1.55, false), (0.8, 0.7, 1.35, 1.35, false), (0.9, 0.83, 1.15, 1.15, false),
        (1.0, 0.93, 0.8, 0.8, false), (1.1, 0.99, 0.42, 0.42, false), (1.1666666, 1.0, 0.0, 0.0, false), (1.6666666, 1.0, 0.0, 0.0, false),
        (1.8, 0.97, -0.428571, -0.428571, false), (1.9, 0.9, -0.8, -0.8, false), (2.0, 0.81, -0.95, -0.95, false), (2.1, 0.71, -1.15, -1.15, false),
        (2.2, 0.58, -1.3, -1.3, false), (2.3, 0.45, -1.3, -1.3, false), (2.4, 0.32, -1.3, -1.3, false), (2.5, 0.19, -1.25, -1.25, false),
        (2.6, 0.07, -1.14, -1.14, false), (2.6666667, 0.0, -0.984147, 0.0, false),
    ]),
    exit: rich(&[(-0.0012225956, 0.0, 0.0, 0.0, true), (0.2, 100.0, 497.67105, 497.6705, false), (0.3, 120.0, 0.0, 31.61652, false)]),
    close: rich(&[(0.0, 120.0, 0.0, 0.0, false), (0.7, 0.0, -257.89227, -257.88046, false)]),
});

/// AM_normal_stand_hidespot_outhouse_enter's and Outhouse_C's.
static OUTHOUSE_DOORS: std::sync::LazyLock<DoorCurves> = std::sync::LazyLock::new(|| DoorCurves {
    enter: rich(&[
        (0.2, 0.0, 0.0, 0.0, false), (0.3, 0.03, 0.5, 0.5, false), (0.4, 0.1, 1.0, 1.0, false), (0.5, 0.23, 1.45, 1.45, false),
        (0.6, 0.39, 1.65, 1.65, false), (0.7, 0.56, 1.55, 1.55, false), (0.8, 0.7, 1.35, 1.35, false), (0.9, 0.83, 1.15, 1.15, false),
        (1.0, 0.93, 0.8, 0.8, false), (1.1, 0.99, 0.42, 0.42, false), (1.1666666, 1.0, 0.0, 0.0, false), (1.4, 1.0, 0.0, 0.0, false),
        (1.5, 0.99, 0.0, 0.0, true), (1.6, 0.97, 0.0, 0.0, true), (1.7, 0.92, 0.0, 0.0, true), (1.8, 0.87, -0.6, -0.6, false),
        (1.9, 0.8, -0.75, -0.75, false), (2.0, 0.72, -0.85, -0.85, false), (2.1, 0.63, -0.95, -0.95, false), (2.2, 0.53, -1.0, -1.0, false),
        (2.3, 0.43, -1.05, -1.05, false), (2.4, 0.32, -1.15, -1.15, false), (2.5, 0.2, -1.2, -1.2, false), (2.6, 0.08, -1.2, -1.2, false),
        (2.6666667, 0.0, -0.984147, 0.0, false),
    ]),
    exit: rich(&[(0.01, 0.0, 355.91776, 0.0, true), (0.2, 100.0, 487.9916, 487.9993, false), (0.3, 120.0, 0.0, 0.0, false)]),
    close: rich(&[(0.0, 120.0, 0.0, 0.0, false), (0.7, 0.0, -287.33765, -287.33435, false)]),
});

/// Where a spot's doors are in their moves.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum DoorMotion {
    #[default]
    Still,
    /// Seconds into the enter move's door driver window.
    Entering(f32),
    /// Seconds into the exit timeline, then the close timeline.
    Exiting(f32),
    Closing(f32),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SpotKind {
    /// Wardrobes and barn closets (`ESCHidingSpotType` default and Barn).
    Closet,
    Outhouse,
    Bed,
    Tent,
}

/// One way in: where she enters from, where Jason searches and kills from (spot space), and the animations.
#[derive(Clone, Debug)]
pub struct Side {
    pub enter: Transform,
    pub search: Transform,
    pub kill: Transform,
    pub enter_anim: &'static str,
    pub exit_anim: &'static str,
    pub search_anim: Option<&'static str>,
    pub killer_kill: Option<&'static str>,
    pub victim_kill: Option<&'static str>,
    /// Its kill's component (HidingSpotKill or RightSideContextKill: its KillInfo, the killer's score).
    pub kill_component: &'static str,
}

#[derive(Component, Debug)]
pub struct HidingSpot {
    pub class: String,
    pub kind: SpotKind,
    /// Its own animated part (the tent), if any.
    pub prop: Option<Entity>,
    pub sides: Vec<Side>,
    pub idle_anim: &'static str,
    /// The interact point (spot space) and its reach (cm).
    pub interact: Vec3,
    pub reach: f32,
    /// The hiding view (spot space; looks along its +X in Unreal terms), its horizontal field of view and how far
    /// it turns (degrees: yaw, pitch).
    pub camera: Transform,
    pub fov_deg: f32,
    pub max_look: Vec2,
    pub occupant: Option<Entity>,
    pub destroyed: bool,
    pub destructible: bool,
    pub destroy_on_search: bool,
    /// Door pivots: entity, closed transform, opening direction (+1 / -1: the right door, against the left one and the
    /// outhouse's).
    pub doors: Vec<(Entity, Transform, f32)>,
    pub door: DoorMotion,
    door_angle: f32,
}

/// The montage each hiding special move plays (its ContextMontage; for the counselor's enter moves, the female
/// sequences of its Default and HideExit sections).
fn special_move_anims(class: &str) -> Option<(&'static str, Option<&'static str>)> {
    Some(match class {
        "Counselor_EnterArmoire_C" => ("f_normal_stand_hidespot_closet_enter", Some("f_normal_stand_hidespot_closet_exit")),
        "Counselor_EnterOuthouse_C" => ("f_normal_stand_hidespot_outhouse_enter", Some("f_normal_stand_hidespot_closet_exit")),
        "Counselor_EnterBedLeft_C" => ("f_normal_stand_hidespot_bed_left_enter", Some("f_normal_stand_hidespot_bed_left_exit")),
        "Counselor_EnterBedRight_C" => ("f_normal_stand_hidespot_bed_right_enter", Some("f_normal_stand_hidespot_bed_right_exit")),
        "Counselor_EnterTent_C" => ("f_normal_stand_hidespot_tent_enter", Some("f_normal_stand_hidespot_tent_exit")),
        "Jason_SearchCloset_Axe_C" => ("cinematic_hidespot_axe_closet_search", None),
        "Jason_SearchBed_Axe_C" => ("cinematic_hidespot_axe_bed_search", None),
        "Jason_ClosetAxe_CK_C" => ("cinematic_hidespot_axe_closet_kill", None),
        "Jason_BedAxe_CK_C" => ("cinematic_hidespot_axe_bed_kill", None),
        "Jason_SleepingbagKill_C" => ("cinematic_hidespot_hand_sleepingbag_kill", None),
        "Counselor_Axe_Closet_CK_C" => ("cinematic_hidespot_closet_kill_Axe", None),
        "Counselor_Axe_Bed_CK_C" => ("cinematic_hidespot_bed_kill_Axe", None),
        "Counselor_SleepingBagKill_C" => ("cinematic_hidespot_hand_Sleepingbag_kill", None),
        _ => return None,
    })
}

/// A special move component's move for `weapon` (its weapon-specific list when it uses one), else its default.
fn special_move(c: &ComponentInfo, list: &str, default: &str, weapon: &str) -> Option<String> {
    let specific = c.props[list].as_array().into_iter().flatten().find(|m| m["WeaponClass"].as_str() == Some(weapon));
    let uses_specific = list != "WeaponSpecificMoves" || c.props["bUseWeaponSpecificMoves"].as_bool().unwrap_or(false);
    specific
        .filter(|_| uses_specific)
        .and_then(|m| m["SpecialMove"].as_str())
        .or_else(|| c.props[default].as_str())
        .map(str::to_string)
}

impl HidingSpot {
    /// Builds a spot from its class description; `local` turns a component into spot space.
    pub fn new(classes: &ClassDb, class: &str, local: impl Fn(&ComponentInfo) -> Transform, doors: Vec<(Entity, Transform, f32)>) -> Option<Self> {
        let info = classes.get(class)?;
        let comp = |name: &str| info.components.iter().find(|c| c.name == name);
        let kind = match classes.prop(class, "HidingSpotType").as_str() {
            Some("ESCHidingSpotType::Bed") => SpotKind::Bed,
            Some("ESCHidingSpotType::Tent") => SpotKind::Tent,
            Some("ESCHidingSpotType::Outhouse") => SpotKind::Outhouse,
            _ => SpotKind::Closet,
        };
        let anim = |m: Option<String>| m.as_deref().and_then(special_move_anims);
        let side = |enter: &str, search: &str, kill: &'static str| -> Option<Side> {
            let (e, s, k) = (comp(enter)?, comp(search)?, comp(kill)?);
            let (enter_anim, exit_anim) = anim(e.props["SpecialMove"].as_str().map(str::to_string))?;
            Some(Side {
                enter: local(e),
                search: local(s),
                kill: local(k),
                enter_anim,
                exit_anim: exit_anim?,
                search_anim: anim(special_move(s, "WeaponSpecificMoves", "SpecialMove", KILLER_WEAPON)).map(|a| a.0),
                killer_kill: anim(special_move(k, "KillerWeaponKillClasses", "InteractorSpecialClass", KILLER_WEAPON)).map(|a| a.0),
                victim_kill: anim(special_move(k, "VictimWeaponKillClasses", "VictimSpecialClass", KILLER_WEAPON)).map(|a| a.0),
                kill_component: kill,
            })
        };
        let mut sides: Vec<Side> = side("EnterSpecialMove", "SearchComponent", "HidingSpotKill").into_iter().collect();
        sides.extend(side("EnterRight", "RightSideSearch", "RightSideContextKill"));
        if sides.is_empty() {
            warn!("{class}: hiding spot without a usable enter move");
            return None;
        }
        let interact = comp("Interactable")?;
        let cam = comp("Hiding Camera");
        let look = &classes.prop(class, "MaxHidingRotationOffset");
        Some(HidingSpot {
            class: class.to_string(),
            kind,
            sides,
            idle_anim: match kind {
                SpotKind::Closet | SpotKind::Outhouse => "f_normal_stand_hidespot_closet_idle",
                SpotKind::Bed => "f_normal_stand_hidespot_bed_idle",
                SpotKind::Tent => "f_normal_stand_hidespot_tent_loop",
            },
            interact: local(interact).translation,
            reach: interact.props["DistanceLimit"].as_f64().map_or(100.0, |v| v as f32),
            camera: cam.map_or(Transform::IDENTITY, &local),
            fov_deg: cam.and_then(|c| c.props["FieldOfView"].as_f64()).map_or(90.0, |v| v as f32),
            max_look: Vec2::new(look["X"].as_f64().unwrap_or(0.0) as f32, look["Y"].as_f64().unwrap_or(0.0) as f32),
            occupant: None,
            destroyed: false,
            destructible: classes.prop(class, "bDestructable").as_bool().unwrap_or(false),
            destroy_on_search: classes.prop(class, "bDestroyOnSearch").as_bool().unwrap_or(true),
            doors,
            door: DoorMotion::Still,
            prop: None,
            door_angle: 0.0,
        })
    }

    /// The side whose way in is nearest `p` (world), given the spot's transform.
    pub fn nearest_side(&self, spot: &GlobalTransform, p: Vec3) -> usize {
        (0..self.sides.len())
            .min_by(|&a, &b| {
                let d = |i: usize| spot.transform_point(self.sides[i].enter.translation).distance_squared(p);
                d(a).total_cmp(&d(b))
            })
            .unwrap_or(0)
    }

    /// A special move component's transform as a character's in the world: kept upright, and turned so the
    /// character faces the component's forward (Unreal +X, Bevy +X; characters here face +Z).
    pub fn world(spot: &GlobalTransform, t: &Transform) -> Transform {
        flat(spot.mul_transform(*t * Transform::from_rotation(Quat::from_rotation_y(std::f32::consts::FRAC_PI_2))).compute_transform())
    }

    pub fn usable(&self) -> bool {
        !self.destroyed && self.occupant.is_none()
    }

    /// Searched by Jason: destroyed when the spot breaks on a search.
    pub fn searched(&mut self, commands: &mut Commands) {
        if self.destroy_on_search && self.destructible && !self.destroyed {
            self.destroyed = true;
            for (e, _, _) in &self.doors {
                commands.entity(*e).insert(Visibility::Hidden);
            }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum HidePhase {
    Entering,
    Hidden,
    Exiting,
    /// Jason found her: his hiding spot kill plays (jason.rs ends it).
    Killed,
}

/// The counselor is in (or going into, or coming out of) a hiding spot.
#[derive(Component, Debug)]
pub struct Hiding {
    pub spot: Entity,
    pub side: usize,
    pub phase: HidePhase,
    pub t: f32,
    /// Where the current animation leaves her.
    pub end: Transform,
    wants_exit: bool,
}

impl Hiding {
    /// On a client: a counselor the server has in a hiding spot (it runs her hiding; net.rs).
    pub fn remote(spot: Entity, side: usize, phase: HidePhase) -> Hiding {
        Hiding { spot, side, phase, t: 0.0, end: Transform::IDENTITY, wants_exit: false }
    }
}

impl HidePhase {
    pub fn index(self) -> u8 {
        match self {
            HidePhase::Entering => 0,
            HidePhase::Hidden => 1,
            HidePhase::Exiting => 2,
            HidePhase::Killed => 3,
        }
    }

    pub fn from_index(i: u8) -> HidePhase {
        match i {
            1 => HidePhase::Hidden,
            2 => HidePhase::Exiting,
            3 => HidePhase::Killed,
            _ => HidePhase::Entering,
        }
    }
}

pub struct HidingPlugin;

impl Plugin for HidingPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(FixedUpdate, update_hiding.before(crate::player::move_player).run_if(crate::net::simulating))
            .add_systems(Update, (read_exit.run_if(crate::net::simulating), swing_doors, setup_breath))
            .add_systems(FixedUpdate, breathe.after(update_hiding).run_if(crate::net::simulating))
            // Her cameras, not when Jason's played (jason_player.rs has the view).
            .add_systems(Update, hiding_camera.after(crate::counselor_camera::counselor_camera).run_if(not(resource_exists::<crate::jason_player::PlayJason>)));
    }
}

/// Starts hiding in `spot` (interact.rs): she plays the enter move from its spot.
pub fn enter(
    commands: &mut Commands,
    player: Entity,
    xf: &mut Transform,
    p: &mut Player,
    spot_entity: Entity,
    spot: &mut HidingSpot,
    gxf: &GlobalTransform,
    model: &CounselorCharacter,
    _now: f32,
    move_params: impl Fn(&str) -> crate::interact::MoveParams,
) {
    let side = spot.nearest_side(gxf, xf.translation);
    let s = &spot.sides[side];
    let enter_spot = HidingSpot::world(gxf, &s.enter);
    let (origin, end) = model.place(s.enter_anim, enter_spot);
    let enter_anim = s.enter_anim;
    p.velocity = Vec2::ZERO;
    p.counselor.set_sprinting(false);
    if p.counselor.stance != f13_core::counselor::Stance::Standing {
        p.counselor.stance = f13_core::counselor::Stance::Standing;
    }
    spot.occupant = Some(player);
    info!("Vanessa: hiding ({:?}, side {side})", spot.kind);
    // Its enter move (Counselor_EnterArmoire etc., SCSpecialMove_SoloMontage): she slides onto its spot, then the
    // take plays and the spot's door and flaps move.
    let tent = (spot.kind == SpotKind::Tent).then_some(spot.prop).flatten();
    // Its enter component: EnterSpecialMove, or a bed's EnterRight.
    let params = move_params(if side == 0 { "EnterSpecialMove" } else { "EnterRight" });
    crate::interact::move_to_rooted_spot(commands, player, *xf, enter_spot, origin, params, move |w| {
        if let Some(mut s) = w.get_mut::<HidingSpot>(spot_entity) {
            s.door = DoorMotion::Entering(0.0);
        }
        if let Some(prop) = tent {
            w.entity_mut(prop).insert(crate::prop_anim::PropAnim::new(crate::prop_anim::TENT_ENTER));
        }
        w.entity_mut(player).insert((Hiding { spot: spot_entity, side, phase: HidePhase::Entering, t: 0.0, end, wants_exit: false }, PlayAction(enter_anim.into())));
    });
}

fn read_exit(pads: crate::controls::Pads, mut hiding: Query<(Entity, &mut Hiding)>) {
    for (pe, mut h) in &mut hiding {
        if h.phase == HidePhase::Hidden && pads.just_pressed(pe, crate::controls::Button::Interact) {
            h.wants_exit = true;
        }
    }
}

/// Exit at the next step (bots).
/// Her breath values, once her character is loaded.
fn setup_breath(mut commands: Commands, counselor: Option<Res<CounselorCharacter>>, tuning: Option<Res<BreathTuning>>, players: Query<Entity, (With<Player>, Without<Breathing>)>) {
    let Some(c) = counselor else { return };
    let t = match tuning {
        Some(t) => t.0.clone(),
        None => {
            let v = |k: &str, d: f32| c.info.values.get(k).copied().unwrap_or(d);
            let stat = |k: &str, d: f32| c.info.stat(k).unwrap_or(f13_core::stats::StatModifiedValue::constant(d));
            let t = f13_core::breath::BreathTuning {
                max: v("MaxBreath", 100.0),
                min_hold_threshold: v("MinHoldBreathThreshold", 50.0),
                min_hold_timer: v("MinHoldBreathTimer", 1.5),
                hold: stat("BreathHoldModifier", 10.0),
                recharge: stat("BreathRechargeModifier", 10.0),
            };
            commands.insert_resource(BreathTuning(t.clone()));
            t
        }
    };
    // Her voice lines play as her class's sounds (audio.rs).
    let lines: std::collections::HashMap<String, String> =
        c.info.sounds.iter().filter_map(|(k, v)| Some((k.clone(), crate::audio::cue_name(&serde_json::Value::String(v.clone()))?))).collect();
    for e in &players {
        commands.entity(e).insert((Breathing { breath: f13_core::breath::Breath::new(&t), scared_next: 0.0, screamed: false, scream_next: 0.0 }, crate::audio::ClassSounds(lines.clone())));
    }
}

/// Space while hidden holds her breath; her breath ticks (UpdateBreath); her hiding voice lines.
#[allow(clippy::type_complexity)]
fn breathe(
    mut commands: Commands,
    time: Res<Time>,
    pads: crate::controls::Pads,
    tuning: Option<Res<BreathTuning>>,
    mut players: Query<(Entity, &Player, &GlobalTransform, &mut Breathing, Option<&Hiding>, Option<&crate::fear_events::Pushed>, Option<&crate::audio::ClassSounds>)>,
    mut jasons: Query<(&GlobalTransform, Option<&mut crate::senses::Knowledge>), With<crate::jason::Jason>>,
    mut sounds: Option<ResMut<crate::audio::Sounds>>,
) {
    let Some(t) = tuning else { return };
    for (pe, p, pxf, mut b, hiding, pushed, lines) in &mut players {
    let dt = time.delta_secs();
    let now = time.elapsed_secs();
    let hidden = hiding.is_some_and(|h| h.phase == HidePhase::Hidden);
    let say = |commands: &mut Commands, line: &'static str| crate::net_world::shout(commands, crate::net_world::Shout::ActorSound(crate::audio::ActorSound { actor: pe, prop: line }));
    if hidden && pads.pressed(pe, crate::controls::Button::Space) {
        if b.breath.hold() {
            info!("holding her breath");
            say(&mut commands, "VO:HoldBreath");
        }
    } else if b.breath.release(&t.0) && hiding.is_some() {
        info!("lets her breath go");
        say(&mut commands, "VO:HoldBreathEnd");
    }
    if b.breath.tick(dt, &p.counselor.stats, &t.0) {
        info!("out of breath");
        if hiding.is_some() {
            say(&mut commands, "VO:HoldBreathEnd");
        }
    }
    // Her scared line: Jason in view and near, not holding her breath though she could, fear at least 70.
    let Some((jxf, knowledge)) = jasons.iter_mut().next() else { continue };
    let distance = jxf.translation().distance(pxf.translation());
    let in_view = pushed.is_some_and(|p| p.spotted);
    if !in_view {
        if b.screamed && distance > SCREAM_M {
            b.screamed = false;
            b.scream_next = now + SCREAM_SECONDS;
        }
    } else if hiding.is_none() && !b.screamed && distance <= SCREAM_M && now >= b.scream_next {
        b.screamed = true;
        info!("screams ({distance:.0} m from Jason)");
        say(&mut commands, "VO:Screams");
    }
    if hidden && in_view && !b.breath.holding && b.breath.can_hold && p.fear >= SCARED_LINE_FEAR && distance <= SCARED_LINE_M && now >= b.scared_next {
        b.scared_next = now + SCARED_LINE_SECONDS;
        info!("whimpering in her hiding spot ({distance:.0} m from Jason)");
        say(&mut commands, "VO:KillerIsNearWhileHiding");
        // He hears it within its audible range: her hiding spot gives itself away.
        let cue = lines.and_then(|l| l.0.get("VO:KillerIsNearWhileHiding")).cloned();
        let range = sounds.as_deref_mut().zip(cue).and_then(|(s, cue)| s.audible_range(&cue));
        if let (Some(range), Some(mut k), Some(h)) = (range, knowledge, hiding) {
            if distance * 100.0 <= range {
                info!("Jason: heard her in her hiding spot");
                k.hiding_spot = Some(h.spot);
                k.last_known = Some((pxf.translation(), now, crate::senses::Clue::Heard));
            }
        }
    }
    }
}

pub fn request_exit(h: &mut Hiding) {
    if h.phase == HidePhase::Hidden {
        h.wants_exit = true;
    }
}

#[allow(clippy::type_complexity)]
fn update_hiding(
    mut commands: Commands,
    time: Res<Time>,
    model: Option<Res<CounselorCharacter>>,
    mut players: Query<(Entity, &mut Hiding, &mut Transform, &mut Player, &mut Visibility)>,
    mut spots: Query<(&mut HidingSpot, &GlobalTransform)>,
    mut rigs: Query<(&ChildOf, &mut Rig)>,
    mut camera: Query<&mut PlayerCamera>,
) {
    let Some(model) = model else { return };
    let dt = time.delta_secs();
    for (pe, mut h, mut xf, mut p, mut vis) in &mut players {
        h.t += dt;
        p.velocity = Vec2::ZERO;
        let Ok((mut spot, gxf)) = spots.get_mut(h.spot) else { continue };
        let side = spot.sides[h.side].clone();
        let len = |n: &str| model.length(n).unwrap_or(1.0);
        let phase = h.phase;
        match phase {
            HidePhase::Entering if h.t >= len(side.enter_anim) => {
                // In: the idle loop from where the enter move left her, out of sight.
                *xf = model.place(spot.idle_anim, h.end).0;
                h.phase = HidePhase::Hidden;
                h.t = 0.0;
                *vis = Visibility::Hidden;
                commands.entity(pe).insert(PlayAction(spot.idle_anim.into()));
                if let Ok(mut c) = camera.single_mut() {
                    c.pitch = 0.0;
                    c.yaw = 0.0;
                }
                info!("Vanessa: hidden");
            }
            HidePhase::Hidden if std::mem::take(&mut h.wants_exit) => {
                let (origin, end) = model.place(side.exit_anim, h.end);
                *xf = origin;
                h.end = end;
                h.phase = HidePhase::Exiting;
                h.t = 0.0;
                *vis = Visibility::Inherited;
                // BlueprintExitStarted.
                spot.door = DoorMotion::Exiting(0.0);
                if let (SpotKind::Tent, Some(prop)) = (spot.kind, spot.prop) {
                    commands.entity(prop).insert(crate::prop_anim::PropAnim::new(crate::prop_anim::TENT_EXIT));
                }
                commands.entity(pe).insert(PlayAction(side.exit_anim.into()));
                info!("Vanessa: leaving the hiding spot");
            }
            HidePhase::Exiting if h.t >= len(side.exit_anim) => {
                *xf = h.end;
                spot.occupant = None;
                commands.entity(pe).remove::<(Hiding, PlayAction)>();
                if let Some((_, mut rig)) = rigs.iter_mut().find(|(c, _)| c.parent() == pe) {
                    rig.snap();
                }
                // Look the way she now faces.
                if let Ok(mut c) = camera.single_mut() {
                    let f = xf.rotation * Vec3::Z;
                    c.yaw = (-f.x).atan2(-f.z);
                    c.pitch = -0.2;
                }
                info!("Vanessa: out of hiding");
            }
            _ => {}
        }
        let _ = gxf;
    }
}

/// Inside: the spot's hiding camera, turned by the mouse within its limits.
pub(crate) fn hiding_camera(
    players: Query<&Hiding>,
    spots: Query<(&HidingSpot, &GlobalTransform)>,
    mut camera: Single<(&mut PlayerCamera, &mut Transform, &mut Projection), Without<Player>>,
    windows: Query<&Window>,
) {
    let (ref mut cam, ref mut xf, ref mut proj) = *camera;
    let Projection::Perspective(persp) = &mut **proj else { return };
    // Not hidden: her cameras (counselor_camera.rs) have the view.
    let hidden = players.iter().find(|h| h.phase == HidePhase::Hidden);
    let Some((spot, gxf)) = hidden.and_then(|h| spots.get(h.spot).ok()) else { return };
    // follow_camera turned yaw/pitch by the mouse; here they are offsets, clamped.
    let max = spot.max_look * std::f32::consts::PI / 180.0;
    cam.yaw = cam.yaw.clamp(-max.x, max.x);
    cam.pitch = cam.pitch.clamp(-max.y, max.y);
    // The camera component looks along Unreal +X (Bevy +X); a Bevy camera looks along -Z.
    let base = gxf.mul_transform(spot.camera).compute_transform();
    xf.translation = base.translation;
    xf.rotation = base.rotation * Quat::from_rotation_y(-std::f32::consts::FRAC_PI_2) * Quat::from_euler(EulerRot::YXZ, cam.yaw, cam.pitch, 0.0);
    // Unreal's FieldOfView is horizontal.
    let aspect = windows.iter().next().map_or(16.0 / 9.0, |w| w.width() / w.height().max(1.0));
    persp.fov = 2.0 * ((spot.fov_deg.to_radians() / 2.0).tan() / aspect).atan();
}

fn swing_doors(time: Res<Time>, mut spots: Query<&mut HidingSpot>, mut transforms: Query<&mut Transform>) {
    let dt = time.delta_secs();
    for mut s in &mut spots {
        let curves: &DoorCurves = match s.kind {
            SpotKind::Closet => &ARMOIRE_DOORS,
            SpotKind::Outhouse => &OUTHOUSE_DOORS,
            SpotKind::Bed | SpotKind::Tent => continue,
        };
        // The door driver turns only the left door (DoorPivot, the outhouse's Mesh); the timelines turn both.
        let (angle, left_only) = match s.door {
            DoorMotion::Still => continue,
            DoorMotion::Entering(t) => {
                let t = t + dt;
                if t >= DOOR_DRIVER_SECONDS {
                    s.door = DoorMotion::Still;
                    (0.0, true)
                } else {
                    s.door = DoorMotion::Entering(t);
                    (curves.enter.eval(t) * 90.0, true)
                }
            }
            DoorMotion::Exiting(t) => {
                let t = t + dt;
                if t >= EXIT_TIMELINE_SECONDS {
                    s.door = DoorMotion::Closing(0.0);
                    (curves.close.eval(0.0), false)
                } else {
                    s.door = DoorMotion::Exiting(t);
                    (curves.exit.eval(t), false)
                }
            }
            DoorMotion::Closing(t) => {
                let t = (t + dt).min(CLOSE_TIMELINE_SECONDS);
                s.door = if t >= CLOSE_TIMELINE_SECONDS { DoorMotion::Still } else { DoorMotion::Closing(t) };
                (curves.close.eval(t), false)
            }
        };
        if angle == s.door_angle {
            continue;
        }
        s.door_angle = angle;
        for (e, closed, dir) in s.doors.clone() {
            if left_only && dir > 0.0 {
                continue;
            }
            if let Ok(mut t) = transforms.get_mut(e) {
                t.rotation = closed.rotation * Quat::from_rotation_y(dir * angle.to_radians());
            }
        }
    }
}
