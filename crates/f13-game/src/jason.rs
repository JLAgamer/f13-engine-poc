//! Jason (Part 2, `Jason_SackHead`), played by the computer: he hunts the counselor, opens doors in his way and
//! breaks locked or barricaded ones, swings his pickaxe, grabs her and kills her with the pickaxe's grab kill.
//!
//! From the game's data (killers/Jason_SackHead.json, written by tools/f13-export killer):
//! - Movement: J2's SCCharacterMovement (walk 160, run 294 cm/s; J2's anim Blueprint sets bCanRun), Jason_Base's
//!   MaxAcceleration 4096 and turn rate 540 deg/s; capsule ACharacter's 34 x 88 cm.
//! - Attack: the pickaxe's NeutralAttack montage (AM_combat_stand_2h_attack_light); it hits during its
//!   AttackTraceNotify window, for WeaponStats.Damage (20) x DamageCharacterModifier. One attack per AttackDelay (1.3 s).
//! - Attack reach and angle: MaxAttackInteractCounselorDistance / MaxAttackInteractAngle (200 cm, 45 deg).
//! - Grab: counselors between MinGrabDistance and MaxGrabDistance (100..235 cm) within MinGrabAngle (35 deg).
//! - Kill: the pickaxe's DefaultGrabKill (Pickaxe_ThroatHack_GrabKill): Jason and the counselor play the paired
//!   cinematic_pickaxe_throathack around one origin; the counselor dies when it ends (bDieOnFinish).
//! - Doors: he opens a closed one with Jason_OpenDoorInside / Jason_OpenDoorOutside (the leaf swings during the
//!   montage's OpenCloseDoor driver window). A locked or barricaded one he breaks down with the pickaxe's
//!   Jason_BreakDoor_Axe: its Enter section, then its Loop until the door is down, then Success. Each BreakDownSync
//!   window checks whether this hit finishes the door (health <= the hit: on to Success) and hits it for
//!   WeaponStats.Damage x DamageWorldModifier, halved through a barricade (SCDoor's TakeDamage): a door's
//!   BaseDoorHealth 60 takes three hits, six barricaded. Raged (bIsRageActive), he bursts through any closed door
//!   instead (BurstDoorFrom*: Jason_WallSmash): the door is gone at the montage's BurstDoor driver window, and its
//!   root motion carries him on through the doorway (anims.json). Each hit swaps the leaf for the door's damage
//!   state mesh as SCDoor's TakeDamage does (doors.rs).
//!
//! - Hiding spots (hiding.rs): with the counselor inside he plays the spot's kill (the pickaxe uses the axe's
//!   Jason_ClosetAxe_CK / Jason_BedAxe_CK, she the matching Counselor_Axe_*_CK, dying at its end), else its search,
//!   from the special move's spot; the search breaks the spot.
//!
//! PLACEHOLDERS (not from the game): the AI itself (the game's Jason is a player). He only knows what senses.rs
//! gives him: he chases her while he sees her; otherwise he goes to where he last saw, heard or sensed her and
//! searches the hiding spots within [`SEARCH_RADIUS_M`] of it (the spot he watched her get into first), and
//! otherwise wanders between cabins (hiding spots and cabinets). He picks grab or swing with even odds and holds a
//! grabbed counselor [`HOLD_BEFORE_KILL`] seconds before killing her. Paths come from f13-core::nav over the
//! collision (the game uses Recast). The grabbed counselor is placed as SCCounselorCharacter::UpdateGrabTransform
//! does (see [`place_held`]). No character-to-character collision. Stuns with a minigame (weapon, flare, shotgun,
//! bear trap) he shakes off with f13-core::stun, wiggling every SCCharacterAIProperties' Min..MaxWiggleInterval.
//!
//! His kit: throwing knives (J2's NumKnives 2; the knife's 30 damage; ThrowingKnifeCooldown 3.5 s; the
//! AM_normal_stand_knife_throw montage's select, aim and throw sections, the knife leaving at its ThrowKnife notify;
//! SCJasonAIProperties' Min/MaxThrowingKnifeDistance 5..25 m for when the computer Jason throws) and traps (J2's
//! InitialTrapCount 7; KillerTrap_C: armed at once, not triggered by him, not picked up; set TrapPlacementDistance
//! 75 cm in front of him with AM_normal_stand_trap_set). PLACEHOLDERS: the knife's speed ([`KNIFE_SPEED`]; the
//! knife Blueprint sets it) with its ProjectileGravityScale 8, aimed to arc onto her chest; how long he aims
//! ([`KNIFE_AIM_SECONDS`]); he sets a trap in a doorway he has just opened when he isn't after her and has none of his
//! within [`TRAP_SPACING_M`]; his trap looks like the counselors' bear trap (A_Trap_Jason isn't exported).
//!
//! Shift, Morph and Stalk run as the game runs them (f13-core::killer_moves; J2's ShiftSpeed 1000 cm/s and ShiftTime
//! 4 s, StalkActiveTime, IdleStalkModifier, TeleportScreenTime; the map's SCKillerMorphPoints), once his abilities
//! unlock and recharge. When he uses them is the AI's (PLACEHOLDERS): Shift when he sees her [`SHIFT_RANGE_M`] away,
//! steering at her and grabbing her when she's in reach (the wind-up stands him still); Morph when the clue or hiding
//! spot he's after is more than [`MORPH_MIN_M`] away; Stalk when he's closing on one within [`STALK_RANGE_M`]. The
//! teleport effect's fade hides him below half (the game dissolves his materials); Stalk silences nothing yet (no
//! audio).

use bevy::prelude::*;
use f13_core::collision::{Capsule, WalkSettings};
use f13_core::health::Hit;
use f13_core::killer_abilities::{Ability, Blockers};
use f13_core::killer_moves::{morph_point, Shift, ShiftPhase, Stalk, TeleportFade};
use f13_core::movement::{calc_velocity, input_acceleration, MovementSettings};
use f13_core::nav::NavGrid;
use std::collections::HashMap;

use crate::character::{KillerCharacter, Model, PlayAction, Rig};
use crate::player::{Player, PlayerTuning};
use crate::match_flow::Score;

/// Unreal units (cm) per metre.
const CM: f32 = 100.0;
const WALK: WalkSettings = WalkSettings { max_step: 0.45, walkable_normal_y: 0.71, gravity: 9.8 };

const IDLE: &str = "normal_stand_idle_01";
/// BS_Jason_Normal_Move's SpeedState 1..3.
const MOVE: [&str; 3] = ["normal_stand_walk_forward", "normal_stand_walk_forward_fast", "normal_stand_run_forward_02"];
const COMBAT_IDLE: &str = "combat_stand_idle_2h";
const ATTACK: &str = "combat_stand_2h_attack_light";
/// The pickaxe's ForwardAttack: his heavy attack (it ends a counselor's block).
const HEAVY_ATTACK: &str = "combat_stand_2h_attack_heavy";
const GRAB_STARTUP: &str = "normal_grab_startup";
const GRAB_ENTER: &str = "normal_grab_enter_1";
const GRAB_IDLE: &str = "normal_grab_idle";
const GRAB_MISS: &str = "normal_grab_miss";
const KILL: &str = "cinematic_pickaxe_throathack";

const HIT_BACK: &str = "f_normal_stand_hitreaction_back_01";
const HIT_LEFT: &str = "f_normal_stand_hitreaction_left_01";
const HIT_RIGHT: &str = "f_normal_stand_hitreaction_right_01";
/// AM_F_Combat_Stand_Block_React.
const BLOCK_REACT: &str = "f_combat_stand_2h_attack_block";
const DEATH: &str = "f_normal_stand_death_01";
const GRABBED_ENTER: &str = "f_normal_grab_enter";
const GRABBED_IDLE: &str = "f_normal_grab_idle";
const COUNSELOR_KILL: &str = "cinematic_pickaxe_throathack";
const DOOR_OPEN_INSIDE: &str = "normal_stand_door_open_inside_forward";
const DOOR_OPEN_OUTSIDE: &str = "normal_stand_door_open_outside_forward";
const DOOR_BURST: &str = "uber_stand_wall_smash_02";
/// Its montage (anims.json: its root motion).
const DOOR_BURST_MONTAGE: &str = "AM_Jason_uber_stand_wall_smash_02";
/// AM_normal_stand_door_break_axe's Enter, Loop and Success sections.
const BREAK_DOOR: [&str; 3] = ["normal_stand_door_break_axe_enter", "normal_stand_door_break_axe_cycle", "normal_stand_door_break_axe_success"];

/// Sockets of SCCounselorCharacter::UpdateGrabTransform: Jason_Base's CharacterAttachmentSocket, SCKillerCharacter's
/// hand socket (constructor), the counselor's neck socket (native FName).
const GRAB_SOCKET: &str = "GrabSocket";
const HAND_GRAB_SOCKET: &str = "handGrabSocket";
const NECK_GRAB_SOCKET: &str = "NeckGrabSocket";
/// Fallback when the sockets aren't posed yet: how far in front of Jason a held counselor stands.
const GRAB_HOLD_M: f32 = 0.65;
/// Seconds Jason holds a grabbed counselor before the kill (a player's choice in the game).
const HOLD_BEFORE_KILL: f32 = 4.0;
/// Odds of going for a grab rather than a swing, rolled after each attempt (AI choice).
const GRAB_ODDS: f32 = 0.5;
/// Paths: re-planned this often (s), with this many cells explored at most.
const REPLAN_SECONDS: f32 = 0.5;
const PATH_BUDGET: usize = 4000;
const NAVMESH_BUDGET: usize = 20000;
/// A closed door this close (m) in the direction Jason walks is opened (or broken).
const DOOR_REACH_M: f32 = 1.4;
/// Within this range (m) of a goal he can't get closer to for this long (s), he counts as there.
const STALL_RANGE_M: f32 = 4.0;
const STALL_SECONDS: f32 = 1.5;
/// How long, and how near, clues around one he couldn't reach are ignored.
const UNREACHABLE_SECONDS: f32 = 15.0;
const UNREACHABLE_RADIUS_M: f32 = 5.0;
/// AI choices (not the game's): Shift at her from this far (m); Morph to a clue or hiding spot this far away (m);
/// Stalk when closing on one this near (m).
const SHIFT_RANGE_M: (f32, f32) = (6.0, 30.0);
/// SCJasonAIProperties (constructor): Min/MaxThrowingKnifeDistance (m).
const KNIFE_RANGE_M: (f32, f32) = (5.0, 25.0);
/// PLACEHOLDERS (see the module docs): the knife's speed (m/s), how long he aims, his traps' spacing (m).
const KNIFE_SPEED: f32 = 40.0;
const KNIFE_AIM_SECONDS: f32 = 0.6;
const TRAP_SPACING_M: f32 = 10.0;
/// ThrowingKnife_C's ProjectileGravityScale; SCThrowable's TimeToDestroy (s).
const KNIFE_GRAVITY: f32 = 9.8 * 8.0;
const KNIFE_LIFE: f32 = 5.0;
/// AM_normal_stand_knife_throw's sections; when the knife leaves (s into the throw section: its ThrowKnife notify).
const KNIFE_SELECT: &str = "normal_stand_knife_select_01";
const KNIFE_AIM: &str = "normal_stand_knife_aim_01";
const KNIFE_THROW: &str = "normal_stand_knife_throw_01";
/// AM_normal_stand_trap_set.
const TRAP_SET: &str = "normal_stand_trap_set";
const MORPH_MIN_M: f32 = 50.0;
const STALK_RANGE_M: f32 = 40.0;
/// He starts a hiding spot's search or kill this close (m) to its spot.
const SPECIAL_MOVE_REACH_M: f32 = 1.5;
/// Losing her, he searches the hiding spots this close (m) to where she was last known.
pub const SEARCH_RADIUS_M: f32 = 8.0;
/// Wandering: places this far (m) at most, a new one after this long (s).
const WANDER_RANGE_M: f32 = 90.0;
const WANDER_SECONDS: f32 = 40.0;

pub struct JasonPlugin {
    /// Seconds before he starts hunting.
    pub delay: f32,
}

#[derive(Resource)]
struct JasonConfig {
    delay: f32,
}

impl Plugin for JasonPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(JasonConfig { delay: self.delay })
            .add_systems(Startup, spawn_jason)
            .add_systems(FixedUpdate, (update_jason, recover_counselor, fly_knives, pick_up_knives))
            .add_observer(spawn_knife)
            .add_systems(Update, (tag_jason_parts, animate_jason));
    }
}

/// Jason's values from the export (see the module docs).
#[derive(Resource, Clone, Debug)]
pub struct KillerTuning {
    pub walk_speed: f32,
    pub run_speed: f32,
    pub can_run: bool,
    pub movement: MovementSettings,
    pub turn_rate_deg: f32,
    pub capsule: Capsule,
    pub attack_delay: f32,
    pub attack_range_m: f32,
    pub attack_angle_deg: f32,
    pub grab_min_m: f32,
    pub grab_max_m: f32,
    pub grab_angle_deg: f32,
    /// Per hit on a counselor, and on a door.
    pub damage: f32,
    pub world_damage: f32,
    /// The swing's length and its AttackTraceNotify window (s).
    pub attack_length: f32,
    pub attack_window: (f32, f32),
    /// The heavy swing's (ForwardAttack) length and AttackTraceNotify window.
    pub heavy_length: f32,
    pub heavy_window: (f32, f32),
    /// Knives he starts with, their cooldown (s) and damage; traps he starts with, set this far in front (m).
    pub num_knives: u32,
    pub knife_cooldown: f32,
    pub knife_damage: f32,
    pub trap_count: u32,
    pub trap_placement_m: f32,
    /// Shift's max speed (cm/s) and length (s); Stalk's length and idle rate; Morph's delay (s).
    pub shift_speed: f32,
    pub shift_time: f32,
    pub stalk_active_time: f32,
    pub idle_stalk_modifier: f32,
    pub teleport_screen_time: f32,
}

impl KillerTuning {
    fn new(info: &f13_data::character::CharacterInfo) -> Self {
        let v = |k: &str, d: f32| info.values.get(k).copied().unwrap_or(d);
        let attack = info.animations.get(ATTACK);
        let weapon = v("WeaponDamage", 20.0);
        KillerTuning {
            walk_speed: v("MaxWalkSpeed", 160.0),
            run_speed: v("MaxRunSpeed", 294.0),
            can_run: v("CanRun", 1.0) != 0.0,
            movement: MovementSettings { max_acceleration: v("MaxAcceleration", 4096.0), ..Default::default() },
            turn_rate_deg: v("RotationRateYaw", 540.0),
            capsule: Capsule { radius: info.capsule_radius / CM, half_height: info.capsule_half_height / CM },
            attack_delay: v("AttackDelay", 1.3),
            attack_range_m: v("MaxAttackInteractCounselorDistance", 200.0) / CM,
            attack_angle_deg: v("MaxAttackInteractAngle", 45.0),
            grab_min_m: v("MinGrabDistance", 100.0) / CM,
            grab_max_m: v("MaxGrabDistance", 235.0) / CM,
            grab_angle_deg: v("MinGrabAngle", 35.0),
            damage: weapon * v("DamageCharacterModifier", 1.0),
            world_damage: weapon * v("DamageWorldModifier", 1.0),
            attack_length: attack.map_or(1.0, |a| a.length),
            attack_window: attack.and_then(|a| a.window("AttackTraceNotify")).unwrap_or((0.2, 0.5)),
            heavy_length: info.animations.get(HEAVY_ATTACK).map_or(1.2, |a| a.length),
            heavy_window: info.animations.get(HEAVY_ATTACK).and_then(|a| a.window("AttackTraceNotify")).unwrap_or((0.3, 0.6)),
            num_knives: v("NumKnives", 0.0) as u32,
            knife_cooldown: v("ThrowingKnifeCooldown", 3.5),
            knife_damage: v("ThrowingKnifeDamage", 30.0),
            trap_count: v("InitialTrapCount", 5.0) as u32,
            trap_placement_m: v("TrapPlacementDistance", 75.0) / CM,
            shift_speed: v("ShiftSpeed", 1000.0),
            shift_time: v("ShiftTime", 4.0),
            stalk_active_time: v("StalkActiveTime", 20.0),
            idle_stalk_modifier: v("IdleStalkModifier", 0.1),
            teleport_screen_time: v("TeleportScreenTime", 1.0),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum JasonState {
    /// Before the hunt starts (s left).
    Waiting(f32),
    Hunting,
    /// A pickaxe swing at the counselor.
    Swinging { t: f32, hit: bool, heavy: bool },
    /// Opening a closed door in his way (s into the montage).
    OpeningDoor { door: Entity, t: f32, anim: &'static str },
    /// Breaking down a locked or barricaded door: s into `section` of [`BREAK_DOOR`], and whether its hit landed.
    BreakingDoor { door: Entity, t: f32, section: usize, hit: bool },
    /// Raged: bursting through a closed door (s into it).
    BurstingDoor { door: Entity, t: f32 },
    /// Reaching for a grab; it connects (or misses) when the startup ends.
    Reaching { t: f32 },
    Missed { t: f32 },
    Holding { t: f32 },
    Killing { t: f32 },
    /// Stunned by a counselor's item (s into it).
    Stunned { t: f32, stun: Stun },
    /// Searching an empty hiding spot (from its search spot): where the search leaves him.
    Searching { spot: Entity, t: f32, end: Transform, anim: &'static str },
    /// Killing the counselor in a hiding spot (his animation, and hers).
    HideKill { spot: Entity, t: f32, end: Transform, anim: &'static str, victim: &'static str },
    /// Hit without being stunned: a short flinch.
    Flinching { t: f32 },
    /// Shifting (f13-core::killer_moves).
    Shifting { shift: Shift },
    /// Morphing: s into it, and where he'll be.
    Morphing { t: f32, to: Transform },
    /// Throwing a knife: s into it, and whether it has left his hand.
    Throwing { t: f32, thrown: bool },
    /// Setting a trap: s into it.
    PlacingTrap { t: f32 },
    /// The counselor is dead.
    Done,
}

#[derive(Component)]
pub struct Jason {
    pub state: JasonState,
    /// Horizontal velocity, cm/s.
    pub velocity: Vec2,
    vel_y: f32,
    grounded: bool,
    path: Vec<Vec3>,
    /// Whether the path gets to the goal (else it ends at the nearest point it can reach).
    path_reaches: bool,
    replan: f32,
    cooldown: f32,
    wants_grab: bool,
    /// Where he was at the last progress check, and the time since.
    progress: (Vec3, f32),
    /// Walking a fixed direction for a moment to get unstuck.
    unstick: Option<(Vec2, f32)>,
    /// Seconds left of immunity to stuns.
    stun_immune: f32,
    /// The hiding spot he's going to search, the ones he searched, the last clue he followed up (its time), and the
    /// place he's wandering to (with when he picked it).
    target_spot: Option<Entity>,
    /// The side of the target spot he's going to (picked once, nearest to him then).
    target_side: Option<usize>,
    searched: Vec<Entity>,
    clue_done: f32,
    wander: Option<(Vec3, f32)>,
    /// Seconds he's been close to his goal without getting closer (furniture or walls in the way): counts as there.
    stall: f32,
    /// Distance to the goal at the last progress check.
    last_gdist: f32,
    /// A clue he couldn't get to (where, and when he gave up): clues near it are ignored for a while.
    unreachable: Option<(Vec3, f32)>,
    /// Next goal log (game time).
    next_log: f32,
    pub stalk: Option<Stalk>,
    pub fade: TeleportFade,
    /// Knives and traps left; the knife cooldown; where his traps are.
    pub knives: u32,
    pub traps: u32,
    knife_cooldown: f32,
    trap_spots: Vec<Vec3>,
    /// Hidden by the fade.
    faded: bool,
    /// The stun minigame, and when he got out of it (s into the stun).
    stun_wiggle: f13_core::stun::Wiggle,
    stun_done: Option<f32>,
    /// The AI's wiggle axis and when it flips next (s into the stun).
    next_wiggle: (f32, f32),
}

impl Jason {
    /// Moved by the host (host.rs): plan again from here, on his feet.
    pub(crate) fn teleported(&mut self) {
        self.path.clear();
        self.replan = 0.0;
        self.vel_y = 0.0;
        self.progress = (Vec3::ZERO, 0.0);
    }

    pub fn stalking(&self) -> bool {
        self.stalk.is_some()
    }

    pub fn shifting(&self) -> bool {
        matches!(self.state, JasonState::Shifting { .. })
    }
}

/// The map's SCKillerMorphPoints: where each is (for picking one), and where Jason ends up (on the floor, facing its
/// way).
#[derive(Resource, Default)]
pub struct MorphPoints(pub Vec<(Vec3, Transform)>);

impl MorphPoints {
    /// Morph's destination for a spot (f13-core::killer_moves::morph_point).
    pub fn pick(&self, spot: Vec3) -> Option<Transform> {
        let cm = |v: Vec3| [v.x * CM, v.z * CM, v.y * CM];
        let points: Vec<[f32; 3]> = self.0.iter().map(|(p, _)| cm(*p)).collect();
        morph_point(&points, cm(spot)).map(|i| self.0[i].1)
    }
}

/// AM_normal_stand_door_break_axe's timing, per section of [`BREAK_DOOR`] (s into it): its length (where the next
/// section starts: the montage's BreakDownLoop driver windows mark the Loop and Success starts), and when its hit lands
/// (the end of its BreakDownSync window; Success's AttackTraceNotify).
struct BreakTiming {
    lengths: [f32; 3],
    hits: [f32; 3],
}

fn break_timing(killer: &KillerCharacter) -> BreakTiming {
    let windows = killer.info.animations.get(BREAK_DOOR[0]).map(|a| a.windows.as_slice()).unwrap_or_default();
    let named = |name: &str| windows.iter().filter(|w| w.name == name).map(|w| (w.start, w.end)).collect::<Vec<_>>();
    let (loops, syncs, traces) = (named("Driver: BreakDownLoop"), named("Driver: BreakDownSync"), named("AttackTraceNotify"));
    let (loop_at, success_at) = match loops.as_slice() {
        [a, b, ..] => (a.0, b.0),
        _ => (2.167, 4.167),
    };
    let success_len = killer.length(BREAK_DOOR[2]).unwrap_or(1.567);
    // A window's end, in the section that starts at `from` and ends at `to`.
    let in_section = |w: &[(f32, f32)], from: f32, to: f32, default: f32| w.iter().find(|w| w.0 >= from && w.0 < to).map_or(default, |w| w.1 - from);
    BreakTiming {
        lengths: [loop_at, success_at - loop_at, success_len],
        hits: [in_section(&syncs, 0.0, loop_at, 1.11), in_section(&syncs, loop_at, success_at, 0.96), in_section(&traces, success_at, f32::MAX, 0.2)],
    }
}

/// How a counselor stunned Jason. Times are the stun classes' TotalStunTime base values (the AI doesn't wiggle out
/// early); animations are their JasonStunAnimMontage / JasonDirectionalStunAnimMontages sections.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Stun {
    /// BaseWeaponStun_C, by the side he was hit on.
    Weapon { left: bool },
    /// FlareGunLightStun_C.
    Flare,
    /// HeavyShotgunStun_C.
    Shotgun,
    /// FirecrackerKillerStun_C.
    Firecracker,
    /// BreakGrabKillerStunPocketKnife_C.
    PocketKnife,
    /// BreakGrabKillerStun_C: she struggled free.
    GrabBreak,
    /// BearTrapLeftFootStun_C.
    BearTrap,
}

impl Stun {
    pub fn seconds(self) -> f32 {
        match self {
            Stun::Weapon { .. } | Stun::Flare => 5.0,
            Stun::Shotgun => 7.0,
            Stun::Firecracker => 4.0,
            Stun::PocketKnife | Stun::GrabBreak => 6.0,
            Stun::BearTrap => 10.0,
        }
    }

    /// The score event for stunning him this way:
    /// - the item's own: Shotgun's StunScoreEventClass, BP_Flare_Projectile's FlareStunScoreEventClass,
    ///   BP_Firecracker_Projectile's StunJasonScoreEventClass, BearTrap's TrapScoreEvent;
    /// - else the counselor's StunJasonScoreEvent;
    /// - breaking his grab is her EscapeJasonGrabScoreEvent.
    pub fn score_event(self) -> &'static str {
        match self {
            Stun::Weapon { .. } => "StunJason",
            Stun::Shotgun => "StunJasonShotgun",
            Stun::Flare => "StunJasonFlare",
            Stun::Firecracker => "FirecrackerJasonStun",
            Stun::BearTrap => "Trapper",
            Stun::PocketKnife | Stun::GrabBreak => "EscapeJasonGrab",
        }
    }

    /// The stun's minigame (bPlayMinigame), which he can wiggle out of early: TotalStunTime is [`Stun::seconds`],
    /// PercentPerWiggle SCStunDamageType's constructor 5, DepletionRate the class's (BaseWeaponStun 0.3, the flare and
    /// shotgun stuns 0.2, the bear trap's the constructor 0.02). The firecracker and grab-break stuns have none.
    pub fn minigame(self) -> Option<f13_core::stun::WiggleTuning> {
        let depletion_rate = match self {
            Stun::Weapon { .. } => 0.3,
            Stun::Flare | Stun::Shotgun => 0.2,
            Stun::BearTrap => 0.02,
            Stun::Firecracker | Stun::PocketKnife | Stun::GrabBreak => return None,
        };
        Some(f13_core::stun::WiggleTuning { total_time: self.seconds(), percent_per_wiggle: 5.0, depletion_rate })
    }

    /// Enter, loop (until the exit is due) and exit animations; a single animation plays once and holds.
    fn anims(self) -> (&'static str, Option<&'static str>, Option<&'static str>) {
        match self {
            Stun::Weapon { left: true } => ("combat_stand_stunned_left_enter_01", Some("combat_stand_stunned_left_loop_01"), Some("combat_stand_stunned_left_exit_01")),
            Stun::Weapon { left: false } => ("combat_stand_stunned_right_enter_01", Some("combat_stand_stunned_right_loop_01"), Some("combat_stand_stunned_right_exit_01")),
            Stun::Flare => ("combat_stand_stunned_back_enter_01", Some("combat_stand_stunned_back_loop_01"), Some("combat_stand_stunned_back_exit_01")),
            Stun::Shotgun => ("combat_stand_stunned_down_enter_01", Some("combat_stand_stunned_down_loop_01"), Some("combat_stand_stunned_down_exit_01")),
            Stun::Firecracker => ("combat_stand_stunned_firecracker_enter_01", Some("combat_stand_stunned_firecracker_loop_01"), Some("combat_stand_stunned_firecracker_exit_01")),
            Stun::PocketKnife => ("normal_stand_knife_pullout", None, None),
            Stun::GrabBreak => ("normal_grab_break", None, None),
            Stun::BearTrap => ("normal_stand_trap01_enter_lf", Some("normal_stand_trap01_cycle_lf"), Some("normal_stand_trap01_exit_lf")),
        }
    }
}

/// Jason takes the throwing knives he passes within reach of (SCKillerCharacter::AddKnives: NumKnives += the pickup's
/// StackSize, no cap). The game has him press to pick them up (a special move); here he takes them in passing, and
/// the computer Jason doesn't go looking for them.
fn pick_up_knives(mut commands: Commands, mut jasons: Query<(&mut Jason, &Transform)>, pickups: Query<(Entity, &crate::props::KnifePickup, &GlobalTransform)>) {
    for (mut j, xf) in &mut jasons {
        if !matches!(j.state, JasonState::Hunting) {
            continue;
        }
        for (e, p, gxf) in &pickups {
            let d = gxf.translation() - xf.translation;
            if Vec2::new(d.x, d.z).length() * 100.0 <= p.reach && d.y.abs() < 2.0 {
                j.knives += p.count;
                info!("Jason picked up {} throwing knife(s) ({} now)", p.count, j.knives);
                commands.entity(e).despawn();
            }
        }
    }
}

/// A knife leaves Jason's hand (`fly_knives` takes it from there).
#[derive(Event, Clone, Copy, Debug)]
pub struct ThrowKnife {
    pub from: Vec3,
    pub vel: Vec3,
    pub damage: f32,
}

/// A thrown knife: its velocity, age, damage; stuck once it hits something.
#[derive(Component)]
struct Knife {
    vel: Vec3,
    age: f32,
    damage: f32,
    stuck: bool,
}

/// The velocity that arcs a knife from `from` onto `to` at [`KNIFE_SPEED`] under its gravity (the low arc); straight
/// at it when it's out of reach.
fn knife_velocity(from: Vec3, to: Vec3) -> Vec3 {
    let d = to - from;
    let flat = Vec2::new(d.x, d.z);
    let x = flat.length().max(1e-3);
    let (v, g, y) = (KNIFE_SPEED, KNIFE_GRAVITY, d.y);
    let disc = v.powi(4) - g * (g * x * x + 2.0 * y * v * v);
    if disc < 0.0 {
        return d.normalize_or_zero() * v;
    }
    let angle = ((v * v - disc.sqrt()) / (g * x)).atan();
    let h = flat / x * v * angle.cos();
    Vec3::new(h.x, v * angle.sin(), h.y)
}

fn spawn_knife(ev: On<ThrowKnife>, mut commands: Commands, mut meshes: ResMut<Assets<Mesh>>, mut materials: ResMut<Assets<StandardMaterial>>) {
    // PLACEHOLDER look: a thin blade (the knife's mesh isn't exported).
    let mesh = meshes.add(Cuboid::new(0.03, 0.01, 0.3));
    let material = materials.add(StandardMaterial { base_color: Color::srgb(0.55, 0.55, 0.6), metallic: 0.8, perceptual_roughness: 0.3, ..default() });
    commands.spawn((
        Mesh3d(mesh),
        MeshMaterial3d(material),
        Transform::from_translation(ev.from).looking_to(ev.vel.normalize_or(Vec3::Z), Vec3::Y),
        Knife { vel: ev.vel, age: 0.0, damage: ev.damage, stuck: false },
    ));
}

/// Knives fly (ProjectileGravityScale 8), hit her (its damage, a hit reaction) or stick in what they hit, and go after
/// TimeToDestroy.
#[allow(clippy::type_complexity)]
fn fly_knives(
    mut commands: Commands,
    time: Res<Time>,
    ptuning: Res<PlayerTuning>,
    counselor_model: Option<Res<crate::character::CounselorCharacter>>,
    (collision, dynamic): (Res<crate::collision::WorldCollision>, Res<crate::collision::DynamicColliders>),
    mut knives: Query<(Entity, &mut Knife, &mut Transform), Without<Player>>,
    mut players: Query<(Entity, &mut Player, &Transform)>,
) {
    let dt = time.delta_secs();
    let world = collision.with(&dynamic);
    for (ke, mut k, mut xf) in &mut knives {
        k.age += dt;
        if k.age >= KNIFE_LIFE {
            commands.entity(ke).despawn();
            continue;
        }
        if k.stuck {
            continue;
        }
        k.vel.y -= KNIFE_GRAVITY * dt;
        let from = xf.translation;
        let to = from + k.vel * dt;
        // Her: the segment passing within her capsule (a 0.3 m radius around her from her feet to 1.8 m).
        if let Ok((pe, mut p, pxf)) = players.single_mut() {
            let (a, b) = (pxf.translation + Vec3::Y * 0.3, pxf.translation + Vec3::Y * 1.5);
            if p.counselor.alive() && segment_distance(from, to, a, b) < 0.35 {
                let r = p.counselor.take_damage(Hit { amount: k.damage, blocked_with_weapon: false }, &ptuning.counselor);
                info!("Jason's knife hit Vanessa for {:.0} (health {:.0})", r.dealt, p.counselor.health);
                if r.died {
                    commands.entity(pe).insert(PlayAction(DEATH.into())).remove::<(Staggered, crate::items::ItemAction)>();
                } else {
                    let reaction = hit_reaction(pxf, from);
                    let len = counselor_model.as_ref().and_then(|c| c.length(reaction)).unwrap_or(0.67);
                    commands.entity(pe).insert((PlayAction(reaction.into()), Staggered(len))).remove::<crate::items::ItemAction>();
                }
                commands.entity(ke).despawn();
                continue;
            }
        }
        if let Some(f) = f13_core::collision::segment_hit(&world, from, to, 1.0) {
            xf.translation = from + (to - from) * f;
            k.stuck = true;
            continue;
        }
        xf.translation = to;
        xf.look_to(k.vel.normalize_or(Vec3::Z), Vec3::Y);
    }
}

/// The closest distance between segments p0..p1 and q0..q1.
fn segment_distance(p0: Vec3, p1: Vec3, q0: Vec3, q1: Vec3) -> f32 {
    let (d1, d2, r) = (p1 - p0, q1 - q0, p0 - q0);
    let (a, e, f) = (d1.dot(d1), d2.dot(d2), d2.dot(r));
    let (s, t) = if a <= 1e-9 && e <= 1e-9 {
        (0.0, 0.0)
    } else if a <= 1e-9 {
        (0.0, (f / e).clamp(0.0, 1.0))
    } else {
        let c = d1.dot(r);
        if e <= 1e-9 {
            ((-c / a).clamp(0.0, 1.0), 0.0)
        } else {
            let b = d1.dot(d2);
            let denom = a * e - b * b;
            let mut s = if denom > 1e-9 { ((b * f - c * e) / denom).clamp(0.0, 1.0) } else { 0.0 };
            let mut t = (b * s + f) / e;
            if t < 0.0 {
                t = 0.0;
                s = (-c / a).clamp(0.0, 1.0);
            } else if t > 1.0 {
                t = 1.0;
                s = ((b - c) / a).clamp(0.0, 1.0);
            }
            (s, t)
        }
    };
    (p0 + d1 * s).distance(q0 + d2 * t)
}

/// A counselor's item hit Jason (items.rs): `stun` if it stunned him, else he flinches.
#[derive(Component, Clone, Debug)]
pub struct HitJason {
    pub stun: Option<Stun>,
    pub from: Vec3,
}

/// The counselor's wiggle axis (A -1, D +1: GLB_Wiggle) and her grab struggle's minigame (f13-core::stun).
#[derive(Resource, Default)]
pub struct Struggle {
    pub input: f32,
    pub wiggle: f13_core::stun::Wiggle,
}
/// SCCharacterAIProperties (constructor): an AI character wiggles every Min..MaxWiggleInterval seconds.
const AI_WIGGLE_INTERVAL: (f32, f32) = (0.15, 0.2);
/// SCCharacter InvulnerableStunDuration (constructor): no new stun this long after one ends.
const STUN_IMMUNE_SECONDS: f32 = 3.0;
const FLINCH_FRONT: &str = "combat_stand_hit_reaction_front";
const FLINCH_BACK: &str = "combat_stand_hit_reaction_back";
const GRAB_BREAK: &str = "f_normal_grab_break_rev";
const KNIFE_BREAK: &str = "f_normal_grab_pocketknife_break";

/// The counselor is in Jason's hands: no movement of her own (jason.rs places her).
#[derive(Component)]
pub struct Held;

/// The counselor is reeling from a hit (s left): no input.
#[derive(Component)]
pub struct Staggered(pub f32);

/// Jason's model parts (ray casts for the camera skip them).
#[derive(Component)]
pub struct JasonPart;

#[derive(Resource)]
pub(crate) struct Nav(NavGrid);

fn spawn_jason(
    mut commands: Commands,
    mut rng: ResMut<crate::GameRng>,
    assets: Res<AssetServer>,
    level: Res<crate::Level>,
    config: Res<JasonConfig>,
    killer: Option<Res<KillerCharacter>>,
    collision: Res<crate::collision::WorldCollision>,
    dynamic: Res<crate::collision::DynamicColliders>,
) {
    let Some(killer) = killer else {
        warn!("no Jason: his model isn't in the export (re-export with tools/f13-export killer)");
        return;
    };
    let tuning = KillerTuning::new(&killer.info);
    let start = level.layouts.iter().flat_map(|l| l.actors_of("SCKillerPlayerStart")).find_map(|a| a.transform.as_ref()).map(crate::to_transform);
    let Some(start) = start else {
        warn!("no SCKillerPlayerStart in the level: no Jason");
        return;
    };
    let world = collision.with(&dynamic);
    // Unreal actors face +X; characters here face +Z.
    let stand = |at: &Transform| {
        let feet = f13_core::collision::spawn_floor(&world, at.translation, 1.5, tuning.capsule.half_height * 2.0, &WALK).unwrap_or(at.translation);
        let (yaw, _, _) = at.rotation.to_euler(EulerRot::YXZ);
        Transform { translation: feet, rotation: Quat::from_rotation_y(yaw - std::f32::consts::FRAC_PI_2), ..default() }
    };
    let Transform { translation: feet, rotation, .. } = stand(&start);
    let morph_points: Vec<(Vec3, Transform)> = level
        .layouts
        .iter()
        .flat_map(|l| l.actors_of("SCKillerMorphPoint"))
        .filter_map(|a| a.transform.as_ref())
        .map(|x| {
            let at = crate::to_transform(x);
            (at.translation, stand(&at))
        })
        .collect();
    info!("{} morph points", morph_points.len());
    commands.insert_resource(MorphPoints(morph_points));
    info!("Jason at {feet} ({:?})", tuning);
    let jason = commands
        .spawn((
            Jason {
                state: JasonState::Waiting(config.delay),
                velocity: Vec2::ZERO,
                vel_y: 0.0,
                grounded: true,
                path: Vec::new(),
                path_reaches: true,
                replan: 0.0,
                cooldown: 0.0,
                wants_grab: rng.0.frand() < GRAB_ODDS,
                progress: (feet, 0.0),
                unstick: None,
                stun_immune: 0.0,
                target_spot: None,
                target_side: None,
                searched: Vec::new(),
                clue_done: f32::MIN,
                wander: None,
                stall: 0.0,
                last_gdist: f32::MAX,
                unreachable: None,
                next_log: 0.0,
                stalk: None,
                fade: TeleportFade::default(),
                knives: tuning.num_knives,
                traps: tuning.trap_count,
                knife_cooldown: 0.0,
                trap_spots: Vec::new(),
                faded: false,
                stun_wiggle: default(),
                stun_done: None,
                next_wiggle: (-1.0, 0.0),
            },
            Transform { translation: feet, rotation, ..default() },
            Visibility::default(),
            Name::new("Jason"),
        ))
        .id();
    crate::character::spawn_model(&mut commands, &assets, &killer.0, Model::Killer, jason, JasonPart);
    commands.insert_resource(Nav(NavGrid::new(0.5, tuning.capsule, WALK)));
    commands.insert_resource(tuning);
}

/// Everything spawned under Jason's model: skip frustum culling (skinned bounds stay in bind pose) and tag it.
fn tag_jason_parts(mut commands: Commands, roots: Query<Entity, (With<JasonPart>, With<Rig>)>, children: Query<&Children>, tagged: Query<(), With<JasonPart>>) {
    for root in &roots {
        for e in children.iter_descendants(root) {
            if !tagged.contains(e) {
                commands.entity(e).insert((JasonPart, bevy::camera::visibility::NoFrustumCulling));
            }
        }
    }
}

/// Facing (yaw) of a character that faces +Z.
fn yaw_of(rotation: Quat) -> f32 {
    rotation.to_euler(EulerRot::YXZ).0
}

/// Horizontal angle (degrees) between where `rotation` faces and the direction `to`.
fn angle_to(rotation: Quat, to: Vec2) -> f32 {
    let yaw = yaw_of(rotation);
    let fwd = Vec2::new(yaw.sin(), yaw.cos());
    to.normalize_or_zero().dot(fwd).clamp(-1.0, 1.0).acos().to_degrees()
}

fn turn_toward(xf: &mut Transform, dir: Vec2, rate_deg: f32, dt: f32) {
    if dir.length_squared() < 1e-6 {
        return;
    }
    let target = dir.x.atan2(dir.y);
    let current = yaw_of(xf.rotation);
    let delta = (target - current + std::f32::consts::PI).rem_euclid(std::f32::consts::TAU) - std::f32::consts::PI;
    let max = rate_deg.to_radians() * dt;
    xf.rotation = Quat::from_rotation_y(current + delta.clamp(-max, max));
}

/// Where Jason wants to go next (unit vector, XZ) on his way to `to`: along the navmesh (navigation.rs) once it's
/// built, else the collision grid.
fn steer(j: &mut Jason, nav: &mut NavGrid, navmesh: Option<&f13_core::navmesh::NavMesh>, world: &(impl f13_core::collision::TriSource + ?Sized), from: Vec3, to: Vec3, dt: f32) -> Vec2 {
    if let Some((dir, left)) = &mut j.unstick {
        *left -= dt;
        let d = *dir;
        if *left <= 0.0 {
            j.unstick = None;
        }
        return d;
    }
    j.replan -= dt;
    if j.replan <= 0.0 || j.path.is_empty() {
        j.replan = REPLAN_SECONDS;
        (j.path, j.path_reaches) = match navmesh {
            Some(m) => m.find_path(from, to, NAVMESH_BUDGET).unwrap_or_default(),
            None => nav.find_path(world, from, to, PATH_BUDGET),
        };
    }
    while j.path.first().is_some_and(|p| Vec2::new(p.x - from.x, p.z - from.z).length() < 0.35) {
        j.path.remove(0);
    }
    let next = j.path.first().copied().unwrap_or(to);
    Vec2::new(next.x - from.x, next.z - from.z).normalize_or_zero()
}

/// Moves Jason's capsule by his velocity toward `wish` at up to `max_speed` (cm/s).
fn walk(j: &mut Jason, xf: &mut Transform, wish: Vec2, max_speed: f32, face: Option<Vec2>, dt: f32, t: &KillerTuning, world: &(impl f13_core::collision::TriSource + ?Sized)) {
    let accel = input_acceleration(wish.to_array(), &t.movement);
    let mut v = j.velocity.to_array();
    calc_velocity(&mut v, accel, max_speed, dt, &t.movement);
    j.velocity = Vec2::from_array(v);
    let mut pos = xf.translation;
    let r = f13_core::collision::move_capsule(world, &mut pos, &mut j.vel_y, j.grounded, j.velocity * dt / CM, dt, &t.capsule, &WALK);
    j.grounded = r.grounded;
    if let Some(n) = r.wall_normal.try_normalize() {
        let into = j.velocity.dot(n).min(0.0);
        j.velocity -= n * into;
    }
    xf.translation = pos;
    let dir = face.unwrap_or(j.velocity);
    if face.is_some() || j.velocity.length_squared() > 1.0 {
        turn_toward(xf, dir, t.turn_rate_deg, dt);
    }
}

/// The hit reaction for a blow from `from` on a counselor at `xf`.
fn hit_reaction(xf: &Transform, from: Vec3) -> &'static str {
    let yaw = yaw_of(xf.rotation);
    let fwd = Vec2::new(yaw.sin(), yaw.cos());
    let right = Vec2::new(-fwd.y, fwd.x);
    let to = Vec2::new(from.x - xf.translation.x, from.z - xf.translation.z).normalize_or_zero();
    if to.dot(fwd) > 0.5 {
        HIT_BACK
    } else if to.dot(right) > 0.0 {
        HIT_LEFT
    } else {
        HIT_RIGHT
    }
}

/// Places the counselor Jason holds, as SCCounselorCharacter::UpdateGrabTransform: turned to his GrabSocket
/// (yaw only here); during the grab her NeckGrabSocket is put on his handGrabSocket, during a kill her mesh origin on
/// the GrabSocket. Both sockets follow his animation (GrabSocket sits on target_jnt). The poses are last frame's.
/// False when the rigs aren't assembled yet.
fn place_held(
    owner: Entity,
    pxf: &mut Transform,
    killing: bool,
    jason: &Rig,
    counselor: (&Rig, &Transform),
    killer: &f13_data::character::CharacterInfo,
    victim: &f13_data::character::CharacterInfo,
    globals: &Query<&GlobalTransform>,
) -> bool {
    let Some(grab) = jason.socket(killer, GRAB_SOCKET, globals) else { return false };
    // The socket's actor-forward axis, flattened (Euler yaw flips when the socket pitches or rolls).
    let fwd = (grab.rotation * crate::character::FACING.inverse()) * Vec3::Z;
    let rotation = Quat::from_rotation_y(fwd.x.atan2(fwd.z));
    if killing {
        // Her mesh (the rig's origin) on the socket.
        pxf.translation = grab.translation - rotation * counselor.1.translation;
    } else {
        let (Some(hand), Some(neck)) = (jason.socket(killer, HAND_GRAB_SOCKET, globals), counselor.0.socket(victim, NECK_GRAB_SOCKET, globals)) else {
            return false;
        };
        // Her neck relative to her, from the same posed frame as the neck (several fixed steps can run per frame),
        // turned with her, onto his hand.
        let Ok(her) = globals.get(owner) else { return false };
        let her = her.compute_transform();
        let local = her.rotation.inverse() * (neck.translation - her.translation);
        pxf.translation = hand.translation - rotation * local;
    }
    pxf.rotation = rotation;
    true
}

/// A closed door within reach ahead of `wish` (any, when `any_way`): what Jason does about it (burst through it
/// raged, break it down when it's locked or barricaded, else open it) and its animation.
fn door_in_way(
    doors: &Query<(Entity, &mut crate::doors::Door, &GlobalTransform)>,
    barricades: &Query<&mut crate::windows::Barricade>,
    here: Vec3,
    wish: Vec2,
    any_way: bool,
    raged: bool,
) -> Option<(JasonState, &'static str)> {
    let (de, d, g) = doors.iter().filter(|(_, d, _)| !d.open && !d.broken).find(|(_, _, g)| {
        let to_door = Vec2::new(g.translation().x - here.x, g.translation().z - here.z);
        to_door.length() < DOOR_REACH_M && (any_way || to_door.normalize_or_zero().dot(wish) > 0.3)
    })?;
    let barred = barricades.iter().any(|b| b.door == Some(de) && b.enabled);
    Some(if raged {
        info!("Jason: bursting through a door (raged)");
        (JasonState::BurstingDoor { door: de, t: 0.0 }, DOOR_BURST)
    } else if d.locked || barred {
        info!("Jason: breaking down a {} door (health {:.0})", if barred { "barricaded" } else { "locked" }, d.health);
        (JasonState::BreakingDoor { door: de, t: 0.0, section: 0, hit: false }, BREAK_DOOR[0])
    } else {
        let anim = if crate::doors::Door::is_inside(g, here) { DOOR_OPEN_INSIDE } else { DOOR_OPEN_OUTSIDE };
        info!("Jason: opening a door");
        (JasonState::OpeningDoor { door: de, t: 0.0, anim }, anim)
    })
}

#[allow(clippy::too_many_arguments, clippy::type_complexity)]
pub(crate) fn update_jason(
    mut commands: Commands,
    (time, anims): (Res<Time>, Res<crate::Anims>),
    tuning: Option<Res<KillerTuning>>,
    ptuning: Res<PlayerTuning>,
    nav: Option<ResMut<Nav>>,
    (killer, counselor_model): (Option<Res<KillerCharacter>>, Option<Res<crate::character::CounselorCharacter>>),
    (collision, dynamic): (Res<crate::collision::WorldCollision>, Res<crate::collision::DynamicColliders>),
    (mut rng, mut struggle, mut controls): (ResMut<crate::GameRng>, ResMut<Struggle>, Option<ResMut<crate::jason_player::JasonControls>>),
    (mut jasons, morph_points): (Query<(Entity, &mut Jason, &mut Transform, Option<&HitJason>, Option<&mut crate::senses::Knowledge>), Without<Player>>, Option<Res<MorphPoints>>),
    (inventories, items, mut fuse_boxes, mut grids): (
        Query<&crate::interact::Inventory>,
        Query<&crate::props::Item>,
        Query<&mut crate::power::FuseBox>,
        Option<ResMut<crate::power::PowerGrids>>,
    ),
    mut players: Query<(Entity, &mut Player, &mut Transform, Has<crate::vehicle::Driving>, Option<&mut crate::hiding::Hiding>, Has<crate::items::Blocking>), Without<Jason>>,
    mut doors: Query<(Entity, &mut crate::doors::Door, &GlobalTransform)>,
    mut barricades: Query<&mut crate::windows::Barricade>,
    (mut spots, cabinets): (Query<(Entity, &mut crate::hiding::HidingSpot, &GlobalTransform)>, Query<&GlobalTransform, With<crate::props::Cabinet>>),
    mut rigs: Query<(&mut Rig, &ChildOf, &Transform), (Without<Jason>, Without<Player>)>,
    (globals, navmesh, leaves, parents): (
        Query<&GlobalTransform>,
        Option<Res<crate::navigation::Navmesh>>,
        Query<(Entity, &ChildOf), With<crate::navigation::NotOnNavmesh>>,
        Query<&ChildOf>,
    ),
) {
    let navmesh = navmesh.as_deref().map(|n| &*n.0);
    let (Some(t), Some(mut nav), Some(killer)) = (tuning, nav, killer) else { return };
    let dt = time.delta_secs();
    let now = time.elapsed_secs();
    // Broken doors' leaves are gone, and a swinging leaf doesn't shove him; an open one stops him like a wall.
    let open_leaves: Vec<Entity> = leaves
        .iter()
        .filter(|(_, pivot)| parents.get(pivot.parent()).ok().and_then(|root| doors.get(root.parent()).ok()).is_some_and(|(_, d, _)| d.broken || d.angle != d.target))
        .map(|(e, _)| e)
        .collect();
    let (grid, lists) = collision.without(&dynamic, |e| open_leaves.contains(&e));
    let world = (grid, lists.as_slice());
    let Ok((pe, mut p, mut pxf, driving, mut hiding, blocking)) = players.single_mut() else { return };
    for (je, mut j, mut xf, hit, mut know) in &mut jasons {
        j.stun_immune = (j.stun_immune - dt).max(0.0);
        j.knife_cooldown = (j.knife_cooldown - dt).max(0.0);
        // Stalk runs down (slower standing still); the teleport effect fades him while he shifts or morphs.
        let moving = j.velocity.length_squared() > 0.0;
        if let Some(s) = j.stalk.as_mut() {
            if !s.tick(dt, moving, t.idle_stalk_modifier) {
                info!("Jason: Stalk over");
                j.stalk = None;
            }
        }
        let effect = matches!(j.state, JasonState::Shifting { .. } | JasonState::Morphing { .. });
        j.fade.tick(dt, effect);
        let faded = j.fade.0 < 0.5;
        if faded != j.faded {
            j.faded = faded;
            commands.entity(je).insert(if faded { Visibility::Hidden } else { Visibility::Inherited });
        }
        // A counselor's item hit him.
        if let Some(h) = hit {
            commands.entity(je).remove::<HitJason>();
            // Her weapon's damage (the item stuns below score their own).
            if matches!(h.stun, None | Some(Stun::Weapon { .. })) {
                commands.trigger(Score("DamageJason"));
            }
            let can_stun = j.stun_immune <= 0.0
                && matches!(
                    j.state,
                    JasonState::Hunting
                        | JasonState::Swinging { .. }
                        | JasonState::Reaching { .. }
                        | JasonState::Missed { .. }
                        | JasonState::Holding { .. }
                        | JasonState::Flinching { .. }
                        | JasonState::OpeningDoor { .. }
                        | JasonState::BreakingDoor { .. }
                        | JasonState::BurstingDoor { .. }
                        | JasonState::Shifting { .. }
                        | JasonState::Throwing { .. }
                        | JasonState::PlacingTrap { .. }
                );
            match h.stun {
                Some(stun) if can_stun => {
                    info!("Jason: stunned ({stun:?}, {:.0}s)", stun.seconds());
                    commands.trigger(Score(stun.score_event()));
                    if matches!(j.state, JasonState::Holding { .. }) {
                        release(&mut commands, pe, GRAB_BREAK, counselor_model.as_deref());
                    }
                    j.state = JasonState::Stunned { t: 0.0, stun };
                    j.velocity = Vec2::ZERO;
                    commands.entity(je).insert(PlayAction(stun.anims().0.into()));
                }
                _ if matches!(j.state, JasonState::Hunting) => {
                    let front = angle_to(xf.rotation, Vec2::new(h.from.x - xf.translation.x, h.from.z - xf.translation.z)) < 90.0;
                    info!("Jason: hit, not stunned");
                    j.state = JasonState::Flinching { t: 0.0 };
                    commands.entity(je).insert(PlayAction(if front { FLINCH_FRONT } else { FLINCH_BACK }.into()));
                }
                _ => info!("Jason: hit, shrugged it off ({:?})", j.state),
            }
        }
        let to = pxf.translation;
        let flat = Vec2::new(to.x - xf.translation.x, to.z - xf.translation.z);
        let dist = flat.length();
        let angle = angle_to(xf.rotation, flat);
        let alive = p.counselor.alive();
        // The player's Jason: this step's input; he faces and aims where they look.
        let player_input = controls.as_mut().map(|c| c.take());
        let face_dir = player_input.map_or(flat, |i| i.forward);
        let state = j.state.clone();
        match state {
            JasonState::Waiting(left) => {
                j.state = if left > dt { JasonState::Waiting(left - dt) } else { JasonState::Hunting };
                if j.state == JasonState::Hunting {
                    info!("Jason: hunting ({:.0} m away)", dist);
                }
            }
            JasonState::Hunting => {
                if !alive {
                    j.state = JasonState::Done;
                    continue;
                }
                j.cooldown -= dt;
                // The player's Jason (jason_player.rs): what they pressed, else the AI below.
                if let Some(i) = player_input {
                    let p = i.presses;
                    let ready = |know: &Option<Mut<crate::senses::Knowledge>>, a: Ability| know.as_ref().is_some_and(|k| k.abilities.is_available(a, &Blockers::default()));
                    if p.shift && ready(&know, Ability::Shift) {
                        if let Some(k) = know.as_mut() {
                            k.abilities.consume(Ability::Shift);
                        }
                        info!("Jason: Shift");
                        j.state = JasonState::Shifting { shift: Shift::start() };
                        j.velocity = Vec2::ZERO;
                        continue;
                    }
                    if p.morph && ready(&know, Ability::Morph) {
                        let ahead = xf.translation + Vec3::new(i.forward.x, 0.0, i.forward.y) * crate::jason_player::MORPH_AHEAD_M;
                        if let Some(to) = morph_points.as_ref().and_then(|m| m.pick(ahead)) {
                            if let Some(k) = know.as_mut() {
                                k.abilities.consume(Ability::Morph);
                            }
                            info!("Jason: Morph");
                            j.state = JasonState::Morphing { t: 0.0, to };
                            j.velocity = Vec2::ZERO;
                            continue;
                        }
                    }
                    if p.stalk && j.stalk.is_none() && ready(&know, Ability::Stalk) {
                        if let Some(k) = know.as_mut() {
                            k.abilities.consume(Ability::Stalk);
                        }
                        info!("Jason: Stalk");
                        j.stalk = Some(Stalk::start(t.stalk_active_time));
                    }
                    if p.knife && j.knives > 0 && j.knife_cooldown <= 0.0 {
                        info!("Jason: throwing a knife ({} left)", j.knives);
                        j.state = JasonState::Throwing { t: 0.0, thrown: false };
                        j.velocity = Vec2::ZERO;
                        commands.entity(je).insert(PlayAction(KNIFE_SELECT.into()));
                        continue;
                    }
                    if p.trap && j.traps > 0 && killer.info.animations.contains_key(TRAP_SET) {
                        info!("Jason: setting a trap ({} left)", j.traps);
                        j.state = JasonState::PlacingTrap { t: 0.0 };
                        commands.entity(je).insert(PlayAction(TRAP_SET.into()));
                        continue;
                    }
                    // A fuse box at hand: a blow cuts its power (the game's Jason_Generator_Destruction move).
                    if p.interact && j.cooldown <= 0.0 {
                        let here = xf.translation;
                        if let Some(mut fuse) = fuse_boxes.iter_mut().filter(|f| !f.destroyed).find(|f| Vec2::new(f.interact.x - here.x, f.interact.z - here.z).length() * 100.0 <= f.reach + 100.0) {
                            if let Some(g) = grids.as_mut() {
                                crate::power::set_destroyed(&mut fuse, true, g);
                            }
                            info!("Jason: destroyed a fuse box");
                            j.state = JasonState::Swinging { t: 0.0, hit: true, heavy: false };
                            commands.entity(je).insert(PlayAction(ATTACK.into()));
                            continue;
                        }
                    }
                    if j.cooldown <= 0.0 && p.grab {
                        info!("Jason: grab");
                        j.state = JasonState::Reaching { t: 0.0 };
                        commands.entity(je).insert(PlayAction(GRAB_STARTUP.into()));
                        continue;
                    }
                    if j.cooldown <= 0.0 && (p.attack || p.heavy) {
                        let heavy = p.heavy && killer.info.animations.contains_key(HEAVY_ATTACK);
                        info!("Jason: {}swing", if heavy { "heavy " } else { "" });
                        j.state = JasonState::Swinging { t: 0.0, hit: false, heavy };
                        commands.entity(je).insert(PlayAction(if heavy { HEAVY_ATTACK } else { ATTACK }.into()));
                        continue;
                    }
                    let raged = know.as_ref().is_some_and(|k| k.abilities.rage_active);
                    if i.movement != Vec2::ZERO {
                        if let Some((state, anim)) = door_in_way(&doors, &barricades, xf.translation, i.movement, false, raged) {
                            j.velocity = Vec2::ZERO;
                            j.state = state;
                            commands.entity(je).insert(PlayAction(anim.into()));
                            continue;
                        }
                    }
                    let speed = if i.run && t.can_run { t.run_speed } else { t.walk_speed };
                    walk(&mut j, &mut xf, i.movement, speed, None, dt, &t, &world);
                    continue;
                }
                let sees = know.as_ref().is_none_or(|k| k.sees);
                // Near the goal but not getting any closer: as good as there.
                let stalled = j.stall > STALL_SECONDS;
                // Where to go: her, if he sees her; else the hiding spot he's after, the last clue, or somewhere.
                let mut goal = to;
                // Going for a hiding spot or a clue (not her, not wandering).
                let mut following = false;
                if !sees {
                    if let Some(s) = know.as_ref().and_then(|k| k.hiding_spot).filter(|s| !j.searched.contains(s)) {
                        if j.target_spot != Some(s) {
                            j.target_side = None;
                        }
                        j.target_spot = Some(s);
                    }
                    // Clues near one he couldn't get to don't count for a while (Sense keeps showing her there).
                    let unreachable = j.unreachable.filter(|u| now - u.1 < UNREACHABLE_SECONDS);
                    let clue = know.as_ref().and_then(|k| k.last_known).filter(|c| c.1 > j.clue_done && unreachable.is_none_or(|u| c.0.distance(u.0) > UNREACHABLE_RADIUS_M));
                    if let Some(s) = j.target_spot {
                        match spots.get_mut(s) {
                            Ok((_, mut spot, g)) if !spot.destroyed || spot.occupant.is_some() => {
                                let side = *j.target_side.get_or_insert_with(|| spot.nearest_side(g, xf.translation));
                                let at = crate::hiding::HidingSpot::world(g, &spot.sides[side].search);
                                goal = at.translation;
                                following = true;
                                // Close to the special move's spot: it puts him on it (the game moves him there too).
                                if stalled || Vec2::new(at.translation.x - xf.translation.x, at.translation.z - xf.translation.z).length() < SPECIAL_MOVE_REACH_M {
                                    j.stall = 0.0;
                                    j.velocity = Vec2::ZERO;
                                    let s_ = spot.sides[side].clone();
                                    let victim = spot.occupant == Some(pe) && hiding.as_ref().is_some_and(|h| h.phase == crate::hiding::HidePhase::Hidden);
                                    match (victim, s_.killer_kill, s_.victim_kill, s_.search_anim) {
                                        (true, Some(kill), Some(vkill), _) => {
                                            info!("Jason: found Vanessa in a {:?}: hiding spot kill", spot.kind);
                                            let (origin, end) = killer.place(kill, crate::hiding::HidingSpot::world(g, &s_.kill));
                                            *xf = origin;
                                            j.state = JasonState::HideKill { spot: s, t: 0.0, end, anim: kill, victim: vkill };
                                            commands.entity(je).insert(PlayAction(kill.into()));
                                            if let (Some(h), Some(c)) = (hiding.as_mut(), counselor_model.as_ref()) {
                                                h.phase = crate::hiding::HidePhase::Killed;
                                                *pxf = c.place(vkill, *pxf).0;
                                            }
                                            commands.entity(pe).insert((PlayAction(vkill.into()), Visibility::Inherited));
                                            spot.open_until = now + killer.length(kill).unwrap_or(4.0);
                                        }
                                        (false, _, _, Some(search)) => {
                                            info!("Jason: searching a {:?}", spot.kind);
                                            let (origin, end) = killer.place(search, at);
                                            *xf = origin;
                                            j.state = JasonState::Searching { spot: s, t: 0.0, end, anim: search };
                                            commands.entity(je).insert(PlayAction(search.into()));
                                            spot.open_until = now + killer.length(search).unwrap_or(4.0);
                                        }
                                        _ => {
                                            j.searched.push(s);
                                            j.target_spot = None;
                                        }
                                    }
                                    continue;
                                }
                            }
                            _ => {
                                j.searched.push(s);
                                j.target_spot = None;
                            }
                        }
                    } else if let Some((at, when, how)) = clue {
                        goal = at;
                        following = true;
                        if stalled || Vec2::new(at.x - xf.translation.x, at.z - xf.translation.z).length() < 1.5 {
                            if stalled && !j.path_reaches {
                                info!("Jason: can't get to the clue ({:.0} m away)", Vec2::new(at.x - xf.translation.x, at.z - xf.translation.z).length());
                                j.unreachable = Some((at, now));
                            }
                            j.stall = 0.0;
                            // Nothing here: the hiding spots around it, nearest first.
                            j.clue_done = when;
                            j.target_side = None;
                            j.target_spot = spots
                                .iter()
                                .filter(|(e, s, g)| !s.destroyed && !j.searched.contains(e) && g.translation().distance(at) < SEARCH_RADIUS_M)
                                .min_by(|a, b| a.2.translation().distance(at).total_cmp(&b.2.translation().distance(at)))
                                .map(|(e, _, _)| e);
                            info!("Jason: lost her ({how:?}); {}", if j.target_spot.is_some() { "searching a hiding spot nearby" } else { "nothing to search here" });
                            j.wander = None;
                        }
                    } else {
                        // Wander between cabins.
                        let here = xf.translation;
                        let stale = stalled || j.wander.is_none_or(|(w, at)| now - at > WANDER_SECONDS || Vec2::new(w.x - here.x, w.z - here.z).length() < 2.0);
                        if stale {
                            j.stall = 0.0;
                        }
                        if stale {
                            let places: Vec<Vec3> = spots
                                .iter()
                                .map(|(_, _, g)| g.translation())
                                .chain(cabinets.iter().map(|g| g.translation()))
                                .filter(|p| (10.0..WANDER_RANGE_M).contains(&p.distance(here)))
                                .collect();
                            if !places.is_empty() {
                                let w = places[rng.0.index(places.len())];
                                debug!("Jason: wandering to ({:.0}, {:.0})", w.x, w.z);
                                j.wander = Some((w, now));
                            }
                        }
                        if let Some((w, _)) = j.wander {
                            goal = w;
                        }
                    }
                }
                let gflat = Vec2::new(goal.x - xf.translation.x, goal.z - xf.translation.z);
                let gdist = gflat.length();
                if now >= j.next_log {
                    j.next_log = now + 5.0;
                    let what = if sees { "her" } else if j.target_spot.is_some() { "a hiding spot" } else if j.wander.is_some() && goal == j.wander.unwrap().0 { "wandering" } else { "a clue" };
                    let door = doors.iter().map(|(_, d, g)| (g.translation().distance(xf.translation), d.open, d.locked, g.translation())).min_by(|a, b| a.0.total_cmp(&b.0));
                    debug!("Jason: going for {what}, {gdist:.1} m to go (at {:.1},{:.1},{:.1} -> {:.1},{:.1},{:.1}; path {:?}); nearest door {door:?}", xf.translation.x, xf.translation.y, xf.translation.z, goal.x, goal.y, goal.z, j.path);
                }
                // His abilities (when is the AI's choice; see the module docs).
                let ready = |know: &Option<Mut<crate::senses::Knowledge>>, a: Ability| know.as_ref().is_some_and(|k| k.abilities.is_available(a, &Blockers::default()));
                let after_clue = !sees && following;
                if sees && !driving && (SHIFT_RANGE_M.0..SHIFT_RANGE_M.1).contains(&dist) && ready(&know, Ability::Shift) {
                    if let Some(k) = know.as_mut() {
                        k.abilities.consume(Ability::Shift);
                    }
                    info!("Jason: Shift ({dist:.0} m from her)");
                    j.state = JasonState::Shifting { shift: Shift::start() };
                    j.velocity = Vec2::ZERO;
                    continue;
                }
                if after_clue && gdist > MORPH_MIN_M && ready(&know, Ability::Morph) {
                    if let Some(to) = morph_points.as_ref().and_then(|m| m.pick(goal)) {
                        if let Some(k) = know.as_mut() {
                            k.abilities.consume(Ability::Morph);
                        }
                        info!("Jason: Morph ({gdist:.0} m to his goal; to a point {:.0} m from it)", Vec2::new(to.translation.x - goal.x, to.translation.z - goal.z).length());
                        j.state = JasonState::Morphing { t: 0.0, to };
                        j.velocity = Vec2::ZERO;
                        continue;
                    }
                }
                if after_clue && gdist < STALK_RANGE_M && j.stalk.is_none() && ready(&know, Ability::Stalk) {
                    if let Some(k) = know.as_mut() {
                        k.abilities.consume(Ability::Stalk);
                    }
                    info!("Jason: Stalk ({gdist:.0} m to his goal)");
                    j.stalk = Some(Stalk::start(t.stalk_active_time));
                }
                // A knife at her when she's in throwing range.
                if sees && !driving && alive && j.knives > 0 && j.knife_cooldown <= 0.0 && (KNIFE_RANGE_M.0..=KNIFE_RANGE_M.1).contains(&dist) {
                    info!("Jason: throwing a knife ({dist:.0} m, {} left)", j.knives);
                    j.state = JasonState::Throwing { t: 0.0, thrown: false };
                    j.velocity = Vec2::ZERO;
                    commands.entity(je).insert(PlayAction(KNIFE_SELECT.into()));
                    continue;
                }
                if sees && !driving && j.cooldown <= 0.0 {
                    // Going for a grab, he swings instead when she's inside the grab's minimum distance (else he'd
                    // stand there); going for a swing, he closes to swing range.
                    let can_grab = dist >= t.grab_min_m && dist <= t.grab_max_m && angle <= t.grab_angle_deg;
                    let can_swing = dist <= t.attack_range_m && angle <= t.attack_angle_deg;
                    if j.wants_grab && can_grab {
                        info!("Jason: grab ({dist:.2} m, {angle:.0} deg)");
                        j.state = JasonState::Reaching { t: 0.0 };
                        commands.entity(je).insert(PlayAction(GRAB_STARTUP.into()));
                        continue;
                    }
                    if can_swing && (!j.wants_grab || dist < t.grab_min_m) {
                        // At a raised guard, the heavy swing (AI choice).
                        let heavy = blocking && killer.info.animations.contains_key(HEAVY_ATTACK);
                        info!("Jason: {}swing ({dist:.2} m, {angle:.0} deg)", if heavy { "heavy " } else { "" });
                        j.state = JasonState::Swinging { t: 0.0, hit: false, heavy };
                        commands.entity(je).insert(PlayAction(if heavy { HEAVY_ATTACK } else { ATTACK }.into()));
                        continue;
                    }
                }
                // Close in: straight at the goal when near, else along a path.
                let mut wish = if gdist < 1.5 { gflat.normalize_or_zero() } else { steer(&mut j, &mut nav.0, navmesh, &world, xf.translation, goal, dt) };
                if sees && dist < t.attack_range_m * 0.7 {
                    wish = Vec2::ZERO; // close enough for anything; don't walk into her
                }
                // A closed door ahead: open it, or break it down when it's locked or barred.
                let raged = know.as_ref().is_some_and(|k| k.abilities.rage_active);
                if let Some((state, anim)) = door_in_way(&doors, &barricades, xf.translation, wish, wish == Vec2::ZERO, raged) {
                    j.velocity = Vec2::ZERO;
                    j.state = state;
                    commands.entity(je).insert(PlayAction(anim.into()));
                    continue;
                }
                let speed = if t.can_run && gdist > t.attack_range_m * 1.5 { t.run_speed } else { t.walk_speed };
                // Up close, square up to her (he only turns toward where he walks otherwise).
                let face = (sees && dist < t.grab_max_m * 1.5).then_some(flat);
                walk(&mut j, &mut xf, wish, speed, face, dt, &t, &world);
                trace!("Jason: at {:.2} wish {wish:.2} vel {:.0} grounded {} vy {:.2} path0 {:?}", xf.translation, j.velocity, j.grounded, j.vel_y, j.path.first());
                // Not getting anywhere: try another way for a moment.
                j.progress.1 += dt;
                if j.progress.1 >= 1.0 {
                    let moved = xf.translation.distance(j.progress.0);
                    // Not getting closer near the goal, or at the end of a path that doesn't get there.
                    if !sees && (gdist < STALL_RANGE_M || !j.path_reaches) && gdist > j.last_gdist - 0.3 {
                        j.stall += 1.0;
                    } else {
                        j.stall = 0.0;
                    }
                    j.last_gdist = gdist;
                    if moved < 0.2 && wish != Vec2::ZERO && gdist > 1.5 {
                        // What's in the way (diagnostic).
                        let probe = xf.translation + Vec3::new(wish.x, 0.0, wish.y) * 0.5;
                        let mut hits = Vec::new();
                        f13_core::collision::TriSource::for_each_in(&world, probe - Vec3::new(0.3, 0.0, 0.3), probe + Vec3::new(0.3, 2.0, 0.3), &mut |t| {
                            hits.push((t.min(), t.max(), t.n));
                        });
                        hits.sort_by(|a, b| b.1.y.total_cmp(&a.1.y));
                        debug!("Jason: stuck at {:.2} heading {wish:.2}; {} triangles ahead, highest {:?}; navmesh here {:?}", xf.translation, hits.len(), hits.first(), navmesh.and_then(|m| m.nearest(xf.translation, 0.5)).map(|n| n.1));
                        let a = rng.0.frand() * std::f32::consts::TAU;
                        j.unstick = Some((Vec2::new(a.cos(), a.sin()), 0.6));
                        j.path.clear();
                    }
                    j.progress = (xf.translation, 0.0);
                }
            }
            JasonState::Shifting { mut shift } => {
                let phase = shift.tick(dt, t.shift_time);
                let in_reach = alive && !driving && dist >= t.grab_min_m && dist <= t.grab_max_m && angle <= t.grab_angle_deg;
                if phase == ShiftPhase::Over || !alive {
                    info!("Jason: Shift over");
                    j.state = JasonState::Hunting;
                } else if in_reach && phase == ShiftPhase::Moving {
                    // Grabbing ends the shift (CancelShift).
                    info!("Jason: grab out of a Shift ({dist:.2} m, {angle:.0} deg)");
                    j.state = JasonState::Reaching { t: 0.0 };
                    commands.entity(je).insert(PlayAction(GRAB_STARTUP.into()));
                } else {
                    j.state = JasonState::Shifting { shift };
                    let sees = know.as_ref().is_none_or(|k| k.sees);
                    let goal = if sees { to } else { know.as_ref().and_then(|k| k.last_known).map_or(to, |c| c.0) };
                    if phase == ShiftPhase::Moving {
                        // The game pushes him along his facing at ShiftSpeed; he turns as he steers.
                        let dir = if let Some(i) = player_input { i.forward } else if sees { flat.normalize_or_zero() } else { steer(&mut j, &mut nav.0, navmesh, &world, xf.translation, goal, dt) };
                        turn_toward(&mut xf, dir, t.turn_rate_deg, dt);
                        let yaw = yaw_of(xf.rotation);
                        let fwd = Vec2::new(yaw.sin(), yaw.cos());
                        walk(&mut j, &mut xf, fwd, t.shift_speed, Some(fwd), dt, &t, &world);
                    } else {
                        walk(&mut j, &mut xf, Vec2::ZERO, t.walk_speed, Some(face_dir), dt, &t, &world);
                    }
                }
            }
            JasonState::Throwing { t: at, thrown } => {
                let at = at + dt;
                walk(&mut j, &mut xf, Vec2::ZERO, t.walk_speed, Some(face_dir), dt, &t, &world);
                let len = |n: &str, d: f32| killer.length(n).unwrap_or(d);
                let aim_at = len(KNIFE_SELECT, 0.37);
                let throw_at = aim_at + KNIFE_AIM_SECONDS;
                let release = throw_at + 0.18f32.min(len(KNIFE_THROW, 0.72));
                let end = throw_at + len(KNIFE_THROW, 0.72);
                let anim = if at < aim_at { KNIFE_SELECT } else if at < throw_at { KNIFE_AIM } else { KNIFE_THROW };
                commands.entity(je).insert(PlayAction(anim.into()));
                let mut thrown = thrown;
                if !thrown && at >= release {
                    thrown = true;
                    j.knives -= 1;
                    j.knife_cooldown = t.knife_cooldown;
                    let yaw = yaw_of(xf.rotation);
                    let from = xf.translation + Vec3::Y * 1.6 + Vec3::new(yaw.sin(), 0.0, yaw.cos()) * 0.5;
                    // At her; the player's knife goes where they look.
                    let target = player_input.map_or(pxf.translation + Vec3::Y * 1.2, |i| from + i.look * KNIFE_RANGE_M.1 * 0.5);
                    commands.trigger(ThrowKnife { from, vel: knife_velocity(from, target), damage: t.knife_damage });
                }
                if at >= end {
                    j.state = JasonState::Hunting;
                    j.cooldown = t.attack_delay;
                    commands.entity(je).remove::<PlayAction>();
                } else {
                    j.state = JasonState::Throwing { t: at, thrown };
                }
            }
            JasonState::PlacingTrap { t: at } => {
                let at = at + dt;
                j.velocity = Vec2::ZERO;
                if at >= killer.length(TRAP_SET).unwrap_or(2.47) {
                    let yaw = yaw_of(xf.rotation);
                    let at_pos = xf.translation + Vec3::new(yaw.sin(), 0.0, yaw.cos()) * t.trap_placement_m;
                    let floor = f13_core::collision::surface_below(&world, at_pos.x, at_pos.z, at_pos.y + 0.5, at_pos.y - 2.0).map_or(at_pos.y, |s| s.0);
                    let spot = Vec3::new(at_pos.x, floor, at_pos.z);
                    j.traps -= 1;
                    j.trap_spots.push(spot);
                    commands.trigger(crate::items::SetKillerTrap(Transform::from_translation(spot).with_rotation(xf.rotation)));
                    info!("Jason: trap set at ({:.1}, {:.1})", spot.x, spot.z);
                    j.state = JasonState::Hunting;
                    commands.entity(je).remove::<PlayAction>();
                } else {
                    j.state = JasonState::PlacingTrap { t: at };
                }
            }
            JasonState::Morphing { t: at, to: dest } => {
                let at = at + dt;
                j.velocity = Vec2::ZERO;
                if at >= t.teleport_screen_time {
                    info!("Jason: morphed to ({:.0}, {:.0})", dest.translation.x, dest.translation.z);
                    *xf = dest;
                    j.path.clear();
                    j.progress = (dest.translation, 0.0);
                    j.stall = 0.0;
                    j.last_gdist = f32::MAX;
                    j.state = JasonState::Hunting;
                } else {
                    j.state = JasonState::Morphing { t: at, to: dest };
                }
            }
            JasonState::Swinging { t: at, hit, heavy } => {
                let at = at + dt;
                walk(&mut j, &mut xf, Vec2::ZERO, t.walk_speed, Some(face_dir), dt, &t, &world);
                let mut hit = hit;
                let (window, length) = if heavy { (t.heavy_window, t.heavy_length) } else { (t.attack_window, t.attack_length) };
                if !hit && at >= window.0 && at <= window.1 && alive && dist <= t.attack_range_m && angle_to(xf.rotation, flat) <= t.attack_angle_deg {
                    hit = true;
                    let r = p.counselor.take_damage(Hit { amount: t.damage, blocked_with_weapon: blocking }, &ptuning.counselor);
                    info!("Jason: hit Vanessa for {:.0}{} (health {:.0})", r.dealt, if blocking { ", blocked" } else { "" }, p.counselor.health);
                    if blocking {
                        commands.trigger(crate::items::BlockedHit);
                        // A heavy attack ends the block (after its blocked damage).
                        if heavy {
                            info!("Jason: his heavy swing broke her block");
                            commands.entity(pe).remove::<crate::items::Blocking>();
                        }
                    }
                    if r.died {
                        info!("Vanessa: killed by Jason's swing");
                        commands.entity(pe).insert(PlayAction(DEATH.into())).remove::<(Staggered, crate::items::ItemAction)>();
                    } else if r.dealt > 0.0 {
                        let reaction = if blocking { BLOCK_REACT } else { hit_reaction(&pxf, xf.translation) };
                        let len = counselor_model.as_ref().and_then(|c| c.length(reaction)).unwrap_or(0.67);
                        commands.entity(pe).insert((PlayAction(reaction.into()), Staggered(len))).remove::<crate::items::ItemAction>();
                        p.velocity = Vec2::ZERO;
                    }
                }
                if at >= length {
                    j.state = JasonState::Hunting;
                    j.cooldown = t.attack_delay;
                    j.wants_grab = rng.0.frand() < GRAB_ODDS;
                    commands.entity(je).remove::<PlayAction>();
                } else {
                    j.state = JasonState::Swinging { t: at, hit, heavy };
                }
            }
            JasonState::OpeningDoor { door: de, t: at, anim } => {
                let at = at + dt;
                let Ok((_, mut d, g)) = doors.get_mut(de) else {
                    j.state = JasonState::Hunting;
                    continue;
                };
                let to_door = Vec2::new(g.translation().x - xf.translation.x, g.translation().z - xf.translation.z);
                walk(&mut j, &mut xf, Vec2::ZERO, t.walk_speed, Some(to_door), dt, &t, &world);
                // The leaf swings during the montage's OpenCloseDoor driver window.
                let (swing_at, swing_len) = killer.info.animations.get(anim).and_then(|a| a.window("Driver: OpenCloseDoor")).map_or((0.5, 0.9), |(s, e)| (s, e - s));
                if !d.open && at >= swing_at {
                    let barred = barricades.iter().any(|b| b.door == Some(de) && b.enabled);
                    if d.locked || barred {
                        // Locked while he reached for it.
                        info!("Jason: the door's locked now; breaking it down");
                        j.state = JasonState::BreakingDoor { door: de, t: 0.0, section: 0, hit: false };
                        commands.entity(je).insert(PlayAction(BREAK_DOOR[0].into()));
                        continue;
                    }
                    d.press(g, xf.translation);
                    d.swing_seconds = swing_len;
                }
                if at >= killer.length(anim).unwrap_or(1.6) {
                    j.state = JasonState::Hunting;
                    commands.entity(je).remove::<PlayAction>();
                    // A trap in the doorway when he isn't after her and has none of his near.
                    let sees = know.as_ref().is_some_and(|k| k.sees);
                    let here = xf.translation;
                    if !sees && j.traps > 0 && j.trap_spots.iter().all(|s| s.distance(here) > TRAP_SPACING_M) && killer.info.animations.contains_key(TRAP_SET) {
                        info!("Jason: setting a trap in the doorway ({} left)", j.traps);
                        j.state = JasonState::PlacingTrap { t: 0.0 };
                        commands.entity(je).insert(PlayAction(TRAP_SET.into()));
                    }
                } else {
                    j.state = JasonState::OpeningDoor { door: de, t: at, anim };
                }
            }
            JasonState::BurstingDoor { door: de, t: at } => {
                let at = at + dt;
                let Ok((_, mut d, g)) = doors.get_mut(de) else {
                    j.state = JasonState::Hunting;
                    continue;
                };
                let burst_at = killer.info.animations.get(DOOR_BURST).and_then(|a| a.window("Driver: BurstDoor")).map_or(0.29, |w| w.0);
                if !d.broken && at >= burst_at {
                    // SCDoor::BurstDestroyDoor: health 0 at once.
                    let health = d.health;
                    d.damage(health, false, &mut commands);
                    info!("Jason: burst through the door");
                    commands.entity(d.pivot).insert(Visibility::Hidden);
                    for mut b in barricades.iter_mut().filter(|b| b.door == Some(de)) {
                        b.enabled = false;
                    }
                }
                // The montage's root motion: on through the doorway, facing it.
                let yaw = yaw_of(xf.rotation);
                let forward = Vec2::new(yaw.sin(), yaw.cos());
                let speed = anims.0.get(DOOR_BURST_MONTAGE).map_or(100.0, |v| v.y.abs());
                let to_door = Vec2::new(g.translation().x - xf.translation.x, g.translation().z - xf.translation.z);
                let face = (at < burst_at).then_some(to_door);
                walk(&mut j, &mut xf, if at < burst_at { Vec2::ZERO } else { forward }, speed, face, dt, &t, &world);
                if at >= killer.length(DOOR_BURST).unwrap_or(1.6) {
                    j.state = JasonState::Hunting;
                    commands.entity(je).remove::<PlayAction>();
                } else {
                    j.state = JasonState::BurstingDoor { door: de, t: at };
                }
            }
            JasonState::BreakingDoor { door: de, t: at, section, hit } => {
                let Ok((_, mut d, g)) = doors.get_mut(de) else {
                    j.state = JasonState::Hunting;
                    continue;
                };
                let to_door = Vec2::new(g.translation().x - xf.translation.x, g.translation().z - xf.translation.z);
                walk(&mut j, &mut xf, Vec2::ZERO, t.walk_speed, Some(to_door), dt, &t, &world);
                let barred = barricades.iter().any(|b| b.door == Some(de) && b.enabled);
                // Unlocked (or unbarred) from inside before it gave way: he stops (the montage's Abort).
                if section < 2 && !d.broken && !d.locked && !barred {
                    info!("Jason: the door's no longer locked; stopped breaking it");
                    j.state = JasonState::Hunting;
                    commands.entity(je).remove::<PlayAction>();
                    continue;
                }
                let timing = break_timing(&killer);
                let (mut at, mut section, mut hit) = (at + dt, section, hit);
                if !hit && at >= timing.hits[section] {
                    if section < 2 && d.health <= crate::doors::Door::hit_damage(t.world_damage, barred) {
                        // BreakDownSync: this hit finishes it.
                        (at, section) = (0.0, 2);
                    } else {
                        hit = true;
                        if d.damage(t.world_damage, barred, &mut commands) {
                            info!("Jason: broke the door down");
                            commands.entity(d.pivot).insert(Visibility::Hidden);
                            for mut b in barricades.iter_mut().filter(|b| b.door == Some(de)) {
                                b.enabled = false;
                            }
                        } else {
                            info!("Jason: hit the door ({:.0} left)", d.health);
                        }
                    }
                }
                if at >= timing.lengths[section] {
                    if section == 2 {
                        j.state = JasonState::Hunting;
                        commands.entity(je).remove::<PlayAction>();
                        continue;
                    }
                    // On to (another) Loop.
                    (at, section, hit) = (at - timing.lengths[section], 1, false);
                }
                commands.entity(je).insert(PlayAction(BREAK_DOOR[section].into()));
                j.state = JasonState::BreakingDoor { door: de, t: at, section, hit };
            }
            JasonState::Reaching { t: at } => {
                let at = at + dt;
                walk(&mut j, &mut xf, face_dir.normalize_or_zero(), t.walk_speed, Some(face_dir), dt, &t, &world);
                let startup = killer.length(GRAB_STARTUP).unwrap_or(0.25);
                if at < startup {
                    j.state = JasonState::Reaching { t: at };
                } else if alive && !driving && dist <= t.grab_max_m && angle_to(xf.rotation, flat) <= t.grab_angle_deg {
                    // A pocket knife in her pockets breaks the grab at once (and is used up).
                    let knife = inventories.get(pe).ok().and_then(|inv| {
                        inv.pockets.iter().copied().find(|&e| items.get(e).is_ok_and(|i| i.kind == crate::props::ItemKind::PocketKnife))
                    });
                    if let Some(k) = knife {
                        info!("Jason: grabbed Vanessa; she stabs him with her pocket knife");
                        commands.entity(pe).queue(move |mut e: EntityWorldMut| {
                            if let Some(mut inv) = e.get_mut::<crate::interact::Inventory>() {
                                inv.pockets.retain(|&x| x != k);
                            }
                        });
                        commands.entity(k).despawn();
                        let len = counselor_model.as_ref().and_then(|c| c.length(KNIFE_BREAK)).unwrap_or(2.0);
                        commands.entity(pe).insert((PlayAction(KNIFE_BREAK.into()), Staggered(len)));
                        j.state = JasonState::Stunned { t: 0.0, stun: Stun::PocketKnife };
                        j.velocity = Vec2::ZERO;
                        commands.entity(je).insert(PlayAction(Stun::PocketKnife.anims().0.into()));
                        continue;
                    }
                    info!("Jason: grabbed Vanessa");
                    struggle.wiggle = default();
                    // Out of a trap, into his hands.
                    commands.entity(pe).remove::<crate::items::Trapped>();
                    j.state = JasonState::Holding { t: 0.0 };
                    j.velocity = Vec2::ZERO;
                    commands.entity(je).insert(PlayAction(GRAB_ENTER.into()));
                    commands.entity(pe).insert((Held, PlayAction(GRABBED_ENTER.into()))).remove::<(Staggered, crate::items::ItemAction)>();
                    p.velocity = Vec2::ZERO;
                } else {
                    info!("Jason: grab missed");
                    j.state = JasonState::Missed { t: 0.0 };
                    commands.entity(je).insert(PlayAction(GRAB_MISS.into()));
                }
            }
            JasonState::Missed { t: at } => {
                let at = at + dt;
                walk(&mut j, &mut xf, Vec2::ZERO, t.walk_speed, None, dt, &t, &world);
                if at >= killer.length(GRAB_MISS).unwrap_or(1.0) {
                    j.state = JasonState::Hunting;
                    j.cooldown = t.attack_delay;
                    j.wants_grab = rng.0.frand() < GRAB_ODDS;
                    commands.entity(je).remove::<PlayAction>();
                } else {
                    j.state = JasonState::Missed { t: at };
                }
            }
            JasonState::Holding { t: at } => {
                let at = at + dt;
                let jrig = rigs.iter().find(|(_, c, _)| c.parent() == je);
                let crig = rigs.iter().find(|(_, c, _)| c.parent() == pe);
                let placed = match (jrig, crig, &counselor_model) {
                    (Some((jr, _, _)), Some((cr, _, cxf)), Some(c)) => place_held(pe, &mut pxf, false, jr, (cr, cxf), &killer.info, &c.info, &globals),
                    _ => false,
                };
                if !placed {
                    let yaw = yaw_of(xf.rotation);
                    pxf.translation = xf.translation + Vec3::new(yaw.sin(), 0.0, yaw.cos()) * GRAB_HOLD_M;
                    pxf.rotation = Quat::from_rotation_y(yaw + std::f32::consts::PI);
                }
                if at >= killer.length(GRAB_ENTER).unwrap_or(0.3) {
                    commands.entity(je).insert(PlayAction(GRAB_IDLE.into()));
                }
                if at >= counselor_model.as_ref().and_then(|c| c.length(GRABBED_ENTER)).unwrap_or(2.0) {
                    commands.entity(pe).insert(PlayAction(GRABBED_IDLE.into()));
                }
                let input = struggle.input;
                if struggle.wiggle.tick(dt, input, &grab_struggle(&p.counselor.stats)) {
                    info!("Vanessa: struggled free after {at:.1}s");
                    commands.trigger(Score(Stun::GrabBreak.score_event()));
                    release(&mut commands, pe, GRAB_BREAK, counselor_model.as_deref());
                    j.state = JasonState::Stunned { t: 0.0, stun: Stun::GrabBreak };
                    commands.entity(je).insert(PlayAction(Stun::GrabBreak.anims().0.into()));
                } else if player_input.map_or(at >= HOLD_BEFORE_KILL, |i| i.presses.attack) {
                    // The AI kills after a while; the player when they click.
                    info!("Jason: Pickaxe Throat Hack");
                    j.state = JasonState::Killing { t: 0.0 };
                    commands.entity(je).insert((PlayAction(KILL.into()), crate::kill_camera::KillCam::new(crate::kill_camera::KILL_CLASS, *xf)));
                    commands.entity(pe).insert(PlayAction(COUNSELOR_KILL.into()));
                } else {
                    j.state = JasonState::Holding { t: at };
                }
            }
            JasonState::Killing { t: at } => {
                let at = at + dt;
                let jrig = rigs.iter().find(|(_, c, _)| c.parent() == je);
                let crig = rigs.iter().find(|(_, c, _)| c.parent() == pe);
                if alive {
                    if let (Some((jr, _, _)), Some((cr, _, cxf)), Some(c)) = (jrig, crig, &counselor_model) {
                        place_held(pe, &mut pxf, true, jr, (cr, cxf), &killer.info, &c.info, &globals);
                    }
                    // She dies, and goes limp, where her side of the kill says (he plays his out).
                    if at >= ragdoll_time(counselor_model.as_deref(), COUNSELOR_KILL) {
                        let health = p.counselor.health;
                        p.counselor.take_damage(Hit { amount: health, blocked_with_weapon: false }, &ptuning.counselor);
                        commands.entity(pe).remove::<(Held, PlayAction)>();
                        info!("Vanessa: killed by Jason (Pickaxe Throat Hack)");
                    }
                }
                if at >= killer.length(KILL).unwrap_or(12.0) {
                    // He stays where the kill took him (its root motion), not back at its start.
                    if let Some((_, last)) = killer.root_motion(KILL) {
                        *xf = crate::character::flat(Transform::from_matrix(xf.to_matrix() * last.to_matrix()));
                        snap_rig(&mut rigs, je);
                    }
                    j.state = JasonState::Done;
                    commands.entity(je).remove::<PlayAction>();
                } else {
                    j.state = JasonState::Killing { t: at };
                }
            }
            JasonState::Stunned { t: at, stun } => {
                if at == 0.0 {
                    j.stun_wiggle = default();
                    j.stun_done = None;
                    j.next_wiggle = (-1.0, 0.0);
                }
                let at = at + dt;
                walk(&mut j, &mut xf, Vec2::ZERO, t.walk_speed, None, dt, &t, &world);
                // Shaking it off: the AI wiggles at a steady rate (the axis flips every 1 / rate s).
                if let (Some(game), None) = (stun.minigame(), j.stun_done) {
                    // The axis flips every AI wiggle interval.
                    if at >= j.next_wiggle.1 {
                        j.next_wiggle = (-j.next_wiggle.0, at + AI_WIGGLE_INTERVAL.0 + (AI_WIGGLE_INTERVAL.1 - AI_WIGGLE_INTERVAL.0) * rng.0.frand());
                    }
                    let input = j.next_wiggle.0;
                    if j.stun_wiggle.tick(dt, input, &game) {
                        info!("Jason: shook off the stun after {at:.1}s");
                        j.stun_done = Some(at);
                    }
                }
                let (enter, looped, exit) = stun.anims();
                let len = |n: &str| killer.length(n).unwrap_or(1.0);
                // The exit plays out the last part of the stun (from when he shook it off).
                let exit_at = match j.stun_done {
                    Some(d) => d,
                    None => exit.map_or(stun.seconds(), |e| (stun.seconds() - len(e)).max(len(enter))),
                };
                let end = exit.map_or(exit_at, |e| exit_at + len(e));
                let anim = if at < len(enter) {
                    enter
                } else if at < exit_at {
                    looped.unwrap_or(enter)
                } else {
                    exit.unwrap_or(enter)
                };
                commands.entity(je).insert(PlayAction(anim.into()));
                if at >= end {
                    info!("Jason: recovered");
                    j.state = JasonState::Hunting;
                    j.cooldown = t.attack_delay;
                    j.stun_immune = STUN_IMMUNE_SECONDS;
                    commands.entity(je).remove::<PlayAction>();
                } else {
                    j.state = JasonState::Stunned { t: at, stun };
                }
            }
            JasonState::Searching { spot, t: at, end, anim } => {
                let at = at + dt;
                if at >= killer.length(anim).unwrap_or(6.0) {
                    *xf = end;
                    j.state = JasonState::Hunting;
                    j.searched.push(spot);
                    j.target_spot = None;
                    if let Ok((_, mut s, _)) = spots.get_mut(spot) {
                        s.searched(&mut commands);
                    }
                    commands.entity(je).remove::<PlayAction>();
                    snap_rig(&mut rigs, je);
                    info!("Jason: searched it: empty");
                } else {
                    j.state = JasonState::Searching { spot, t: at, end, anim };
                }
            }
            JasonState::HideKill { spot, t: at, end, anim, victim } => {
                let at = at + dt;
                if alive && at >= ragdoll_time(counselor_model.as_deref(), victim) {
                    let health = p.counselor.health;
                    p.counselor.take_damage(Hit { amount: health, blocked_with_weapon: false }, &ptuning.counselor);
                    info!("Vanessa: killed by Jason in her hiding spot");
                }
                if at >= killer.length(anim).unwrap_or(4.0) {
                    *xf = end;
                    j.state = JasonState::Done;
                    commands.entity(je).remove::<PlayAction>();
                    snap_rig(&mut rigs, je);
                    commands.entity(pe).remove::<crate::hiding::Hiding>();
                    if let Ok((_, mut s, _)) = spots.get_mut(spot) {
                        s.occupant = None;
                        s.searched(&mut commands);
                    }
                } else {
                    j.state = JasonState::HideKill { spot, t: at, end, anim, victim };
                }
            }
            JasonState::Flinching { t: at } => {
                let at = at + dt;
                walk(&mut j, &mut xf, Vec2::ZERO, t.walk_speed, None, dt, &t, &world);
                if at >= killer.length(FLINCH_FRONT).unwrap_or(0.5) {
                    j.state = JasonState::Hunting;
                    commands.entity(je).remove::<PlayAction>();
                } else {
                    j.state = JasonState::Flinching { t: at };
                }
            }
            JasonState::Done => {
                walk(&mut j, &mut xf, Vec2::ZERO, t.walk_speed, None, dt, &t, &world);
            }
        }
    }
}

/// When a counselor's kill animation hands her to the ragdoll: its "shouldRagdoll" notify (the counselor anim
/// Blueprint's cue), else its end.
fn ragdoll_time(model: Option<&crate::character::CounselorCharacter>, anim: &str) -> f32 {
    let info = model.and_then(|m| m.info.animations.get(anim));
    info.and_then(|a| a.window("shouldRagdoll")).map(|w| w.0).or(info.map(|a| a.length)).unwrap_or(4.0)
}

/// Snaps Jason's rig to its next pose (he was moved to where a root motion ended).
fn snap_rig(rigs: &mut Query<(&mut Rig, &ChildOf, &Transform), (Without<Jason>, Without<Player>)>, owner: Entity) {
    if let Some((mut r, _, _)) = rigs.iter_mut().find(|(_, c, _)| c.parent() == owner) {
        r.snap();
    }
}

/// A/D presses while held are wiggles, at most MaxWigglesPerSecond.
/// The wiggle axis: A -1, D +1 (both or neither: 0).
pub(crate) fn read_struggle(keys: Res<ButtonInput<KeyCode>>, mut s: ResMut<Struggle>) {
    s.input = keys.pressed(KeyCode::KeyD) as i32 as f32 - keys.pressed(KeyCode::KeyA) as i32 as f32;
}

/// CounselorGrabStun_C: the grab is a stun she wiggles out of: TotalStunTime 15 s (lower with Strength 15 and
/// Composure 60), PercentPerWiggle 4.5 (more with Composure 60), DepletionRate 0.
pub fn grab_struggle(stats: &f13_core::stats::Stats) -> f13_core::stun::WiggleTuning {
    use f13_core::stats::{Stat, StatModifiedValue};
    let mut total = StatModifiedValue::constant(15.0);
    total.modifiers[Stat::Strength as usize] = 15.0;
    total.modifiers[Stat::Composure as usize] = 60.0;
    let mut per = StatModifiedValue::constant(4.5);
    per.modifiers[Stat::Composure as usize] = 60.0;
    f13_core::stun::WiggleTuning { total_time: total.value(stats, true).max(1.0), percent_per_wiggle: per.value(stats, false), depletion_rate: 0.0 }
}

/// Lets the counselor go: she plays `anim` (and can't move until it ends).
fn release(commands: &mut Commands, counselor: Entity, anim: &str, model: Option<&crate::character::CounselorCharacter>) {
    let len = model.and_then(|c| c.length(anim)).unwrap_or(1.5);
    commands.entity(counselor).remove::<Held>().insert((PlayAction(anim.into()), Staggered(len)));
}

/// Ends a counselor's hit reaction.
fn recover_counselor(mut commands: Commands, time: Res<Time>, mut players: Query<(Entity, &mut Staggered, &Player), Without<Held>>) {
    for (e, mut s, p) in &mut players {
        s.0 -= time.delta_secs();
        if s.0 <= 0.0 {
            commands.entity(e).remove::<Staggered>();
            if p.counselor.alive() {
                commands.entity(e).remove::<PlayAction>();
            }
        }
    }
}

fn animate_jason(
    time: Res<Time>,
    killer: Option<Res<KillerCharacter>>,
    tuning: Option<Res<KillerTuning>>,
    jasons: Query<(&Jason, Option<&PlayAction>)>,
    players: Query<&Transform, With<Player>>,
    owners: Query<&Transform, With<Jason>>,
    mut rigs: Query<(&ChildOf, &mut Rig, &mut AnimationPlayer)>,
) {
    let (Some(killer), Some(t)) = (killer, tuning) else { return };
    let dt = time.delta_secs();
    for (parent, mut rig, mut anim) in &mut rigs {
        if rig.model != Model::Killer {
            continue;
        }
        let Ok((j, action)) = jasons.get(parent.parent()) else { continue };
        rig.set_action(action.map(|a| a.0.as_str()), dt);
        // BS_Jason_Normal_Move's SpeedState: 0 idle, 1 walk, 2 fast walk, 3 run.
        let speed = j.velocity.length();
        let marks = [0.0, t.walk_speed, 0.5 * (t.walk_speed + t.run_speed), t.run_speed];
        let state = speed_state(speed, &marks);
        // Squared up to a counselor nearby: the pickaxe's combat idle.
        let near = owners.get(parent.parent()).ok().zip(players.iter().next()).is_some_and(|(a, b)| a.translation.distance(b.translation) < 4.0);
        let idle = if near && j.state != JasonState::Done { COMBAT_IDLE } else { IDLE };
        let mut targets: HashMap<&str, f32> = HashMap::new();
        for (k, name) in [idle, MOVE[0], MOVE[1], MOVE[2]].iter().enumerate() {
            let w = (1.0 - (state - k as f32).abs()).max(0.0);
            if w > 0.0 {
                *targets.entry(name).or_default() += w;
            }
        }
        let synced = |n: &str| MOVE.contains(&n);
        rig.drive(&killer.0, &targets, &synced, dt, &mut anim);
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
    fn angles_are_measured_from_facing() {
        let facing_z = Quat::IDENTITY;
        assert!(angle_to(facing_z, Vec2::new(0.0, 1.0)) < 1e-3);
        assert!((angle_to(facing_z, Vec2::new(1.0, 0.0)) - 90.0).abs() < 1e-3);
        assert!((angle_to(facing_z, Vec2::new(0.0, -1.0)) - 180.0).abs() < 1e-3);
    }

    #[test]
    fn a_blow_from_the_front_knocks_her_back() {
        let xf = Transform::IDENTITY; // facing +Z
        assert_eq!(hit_reaction(&xf, Vec3::new(0.0, 0.0, 2.0)), HIT_BACK);
        assert_ne!(hit_reaction(&xf, Vec3::new(2.0, 0.0, 0.0)), HIT_BACK);
    }
}
