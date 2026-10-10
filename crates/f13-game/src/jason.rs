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
//! 75 cm in front of him with AM_normal_stand_trap_set). The knife (SCThrowable::Use 0x140413440, decompiled) leaves
//! 120 cm ahead of the camera along its forward at his ThrowStrength (Jason_Base 12000 cm/s; FUN_1403f2f60), then
//! falls at ProjectileGravityScale 8. PLACEHOLDERS (the AI's): from his eyes, aimed to arc onto her chest; how long he aims
//! ([`KNIFE_AIM_SECONDS`]); he sets a trap in a doorway he has just opened when he isn't after her and has none of his
//! within [`TRAP_SPACING_M`]; his trap looks like the counselors' bear trap (A_Trap_Jason isn't exported).
//!
//! Shift, Morph and Stalk run as the game runs them (f13-core::killer_moves; J2's ShiftSpeed 1000 cm/s and ShiftTime
//! 4 s, StalkActiveTime, IdleStalkModifier, TeleportScreenTime; the map's SCKillerMorphPoints), once his abilities
//! unlock and recharge. When he uses them is the AI's (PLACEHOLDERS): Shift when he sees her [`SHIFT_RANGE_M`] away,
//! steering at her and grabbing her when she's in reach (the wind-up stands him still); Morph when the clue or hiding
//! spot he's after is more than [`MORPH_MIN_M`] away, not in his first [`MORPH_GRACE_S`] and landing short of her;
//! Stalk when he's closing on one within [`STALK_RANGE_M`] or as he arrives from a Morph. The
//! teleport effect's fade hides him below half (the game dissolves his materials); Stalk silences nothing yet (no
//! audio).
//!
//! In the water (SCKillerCharacter, decompiled): he's under it when his capsule's top is below its surface (his tick
//! 0x14040d900); there he walks the bottom at MaxSlowRunSpeed up to UnderwaterSpeed by his feet's depth over
//! UnderwaterFullSpeedDepth (GetMaxSpeed, GetUnderwaterDepthModifier), slides (GroundFriction 0), can't Shift, and
//! is unseen outside his special moves (his surface image is bubbles: no particles here). Deep enough (his feet
//! UnderwaterFullSpeedDepth down) with a swimming counselor over ground within 350 cm of her, his feet
//! UnderwaterKillDepth down and her within his UnderwaterInteractComponent's reach (its DistanceLimit: his distance
//! to the surface over him plus 104 cm), he drowns her (Underwater_GrabKill: he takes the kill's place at her, turned
//! as his mesh; she stays). The AI follows her into the water and only closes in under her there (its choice).
//! Going under drops a counselor he holds (OnRep_UnderWater, MULTICAST_DropCounselor). Not ported:
//! the underwater sound mix and loop, the player's underwater input bindings.

use bevy::prelude::*;
use bevy::world_serialization::{WorldAsset, WorldAssetRoot};
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
/// Jason_AnimBP's GrabMontage, AM_Jason_Enter_Grab_Montage (PlayGrabAnim 0x14031a110, rate 1): its Default section
/// (the reach) runs on into GrabMissed unless the grab lands, which takes it to GrabSuccess. Its notifies:
/// EnterGrabBegin at 0 (his move input ignored), PerformGrabTest at 0.3 s (GrabCounselor: the range and angle test),
/// EnterGrabFinish 1.279 s into the montage after a miss (the move input back) or 1.469 s into GrabSuccess.
const GRAB_STARTUP: &str = "normal_grab_enter_1";
const PERFORM_GRAB_TEST_S: f32 = 0.3;
/// The GrabSuccess section; the grab kills open once it's played (FinishGrabCounselor's UpdateGrabState timer).
pub const GRAB_ENTER: &str = "normal_grab_Success_2";
/// The GrabMissed section, and the montage's end counted from the test: its root motion holds him until then
/// (EnterGrabFinish at 1.279 s hands his input back, but the montage's root motion overrides it).
const GRAB_MISS: &str = "normal_grab_fail";
const GRAB_MISS_END_S: f32 = 1.8000001 - PERFORM_GRAB_TEST_S;
const GRAB_MONTAGE: &str = "AM_Jason_Enter_Grab_Montage";

/// The grab montage's ILLCameraShakeAnimNotifies (Notify 0x140232260: ClientPlayCameraShake(CameraShake, Scale) when
/// he's locally controlled; ShakeRange 0, so no one else): CineCamShakeMedium x1 at 0.347 s (a miss: counted from
/// the test), CineCamShakeHard x0.6 at 1.834 s (0.034 s into GrabSuccess).
const GRAB_MISS_SHAKE: (&str, f32, f32) = ("CineCamShakeMedium", 1.0, 0.34681368 - PERFORM_GRAB_TEST_S);
const GRAB_SUCCESS_SHAKE: (&str, f32, f32) = ("CineCamShakeHard", 0.6, 1.8340796 - 1.8000001);

/// An anim notify's camera shake on Jason's camera (jason_player.rs plays it when he's played).
#[derive(Event, Clone, Copy, Debug)]
pub struct JasonCameraShake {
    pub shake: &'static str,
    pub scale: f32,
}

/// A section of his grab montage's root motion (anims.json; cm/s in his frame).
fn grab_root(anims: &crate::Anims, take: &str) -> Vec2 {
    anims.0.get(&format!("{GRAB_MONTAGE}/{take}")).copied().unwrap_or(Vec2::ZERO)
}
/// Jason_Base's ShiftAudioCue (the Shift's rush, on him) and TeleportCueCounselorHears (the static a counselor near his
/// Morph or Stalk hears: [`distortion`]).
const SHIFT_CUE: &str = "ShiftTunnelCue";
const TELEPORT_HEARD_CUE: &str = "VHS_SFXCue";

/// Stopping a car dead: his take (AM_normal_stand_car_stop_slam).
const CAR_SLAM: &str = "normal_stand_car_stop_slam_01";

/// A car ran into Jason and stopped dead (vehicle.rs): he slams it, facing it, if he's free to.
#[derive(Event, Clone, Copy, Debug)]
pub struct CarSlam {
    pub jason: Entity,
    pub car: Vec3,
}

fn car_slam(ev: On<CarSlam>, mut commands: Commands, mut jasons: Query<(&mut Jason, &mut Transform)>) {
    let Ok((mut j, mut xf)) = jasons.get_mut(ev.jason) else { return };
    if !matches!(j.state, JasonState::Hunting) {
        return;
    }
    let to = ev.car - xf.translation;
    xf.rotation = Quat::from_rotation_y(to.x.atan2(to.z));
    info!("Jason: slams the car");
    j.state = JasonState::Slamming { t: 0.0 };
    j.velocity = Vec2::ZERO;
    commands.entity(ev.jason).insert(PlayAction(CAR_SLAM.into()));
}

/// Pulling the driver out of the car: both sides' take (AM_normal_stand_car_grab_left), played from DriverGrab.
const CAR_GRAB: &str = "cinematic_car_grab_left";
const GRAB_IDLE: &str = "normal_grab_idle";
/// Jason_AnimBP's grab layer (LayeredBoneBlend over his locomotion, BlendWeights[0] bound to GrabWeight =
/// SCKillerCharacter's GrabValue: 1 from FinishGrabCounselor until he drops her): its BranchFilters, in component
/// space (bMeshSpaceRotationBlend); its layer is the GrabState machine's Idle (GRAB_IDLE).
const GRAB_LAYER_FILTERS: &[(&str, i32)] = &[("spine_01", 3), ("target_jnt", 0)];
/// Jason_AnimBP's CombatUpperBody layer (LayeredBoneBlend_36EE: spine_01 BlendDepth 1, target_jnt 0, BlendWeights
/// 0.8, mesh-space rotation), chosen over CombatFullBody while he moves (BlendListByBool on bIsMoving, 0.1 s).
const BLOCK_LAYER_FILTERS: &[(&str, i32)] = &[("spine_01", 1), ("target_jnt", 0)];
const BLOCK_LAYER_WEIGHT: f32 = 0.8;
const BLOCK_LAYER_BLEND_S: f32 = 0.1;
/// His grab kills on the four kill keys (SCKillerCharacter's GrabKills: UseRightGrabKill 0, UseBottomGrabKill 1,
/// UseLeftGrabKill 2, UseTopGrabKill 3, used by FUN_1403ffa80; a click while holding her uses slot 1), as indexes into
/// [`KILLS`]. InitFromPlayerState (0x1403fb9f0) spawns each slot's kill from the player's picks (PlayerState
/// PickedGrabKills, from SCCharacterSelectionsSaveGame's killer profile; this port has no save) or else his
/// GrabKillClasses in order, the first empty one taking the weapon's DefaultGrabKill into slot 1 and ending the
/// list: Jason_SackHead keeps Jason_Base's GrabKillClasses[0] (Choke_GrabKill) and empties 1-3, so the choke on the
/// right, the pickaxe's throat hack on the bottom (and the click), nothing on the left or top.
pub const GRAB_KILL_SLOTS: [Option<usize>; 4] = [Some(5), Some(0), None, None];

/// The grab kills' KillBounds by [`KILLS`] index (DoesKillVolumeFit): the box's place in the kill's root
/// space (its component transform, Bevy) and half size (BoxExtent, m); and their ContextKillHUDIcon (its PNG):
/// KillerGrabWidget's icons (GetGrabKillIcon(slot, true)).
#[derive(Resource, Default)]
pub struct GrabKillInfo {
    pub bounds: [Option<(Transform, Vec3)>; KILLS.len()],
    pub icons: [Option<String>; KILLS.len()],
}

/// Which of his four grab kill slots he can use as he holds her (IsGrabKillAvailable 0x1403f5500): bGrabKillsActive
/// and the slot's kill's KillBounds fitting where he stands (SCDynamicContextKill::DoesKillVolumeFit 0x140379c40: an
/// overlap on WorldDynamic of the box, the kill attached at his feet facing his way (ReattachKill 0x140396600),
/// blocked by anything but him, her and the kill). A kill key on a slot that isn't (FUN_1403ffa80) does nothing.
/// bGrabKillsActive: FinishGrabCounselor (0x1403eb560) sets it once his grab montage's enter has played out
/// (UpdateGrabState 0x140324b40: the rest of its section and the GrabSuccess section; a timer for that long, its
/// lambda the guarded write at 0x1403eaf64), so none is available during the grab's enter take. Not ported:
/// IsGrabKillAvailable's other conditions (her vt+0xe90; slot 1's controller flag 0x3a8).
#[derive(Resource, Default, Debug, PartialEq)]
pub struct GrabKillsAvailable(pub [bool; 4]);

/// The grab kills: his and her paired montage's take (same name), the kill class (its camera), and its KillName: the
/// pickaxe's, and his own choke (Choke_GrabKill: Jason_Choke_CK / Counselor_Choke_CK).
pub const KILLS: [(&str, &str, &str); 6] = [
    ("cinematic_pickaxe_throathack", "Pickaxe_ThroatHack_GrabKill", "Pickaxe Throat Hack"),
    ("cinematic_pickaxe_head_canopener", "Pickaxe_Canopener_GrabKill", "Pickaxe Can Opener"),
    ("cinematic_pickaxe_curbstomp", "PickAxe_CurbStomp_GrabKill", "Pickaxe Curb Stomp"),
    ("cinematic_pickaxe_sweeptheleg", "PickAxe_SweepTheLeg_GrabKill", "Pickaxe Sweep The Leg"),
    ("cinematic_pickaxe_toppopper", "PickAxe_TopPopper_GrabKill", "Pickaxe Top Popper"),
    ("cinematic_hand_choke", "Choke_GrabKill", "Choke"),
];

const HIT_BACK: &str = "f_normal_stand_hitreaction_back_01";
const HIT_LEFT: &str = "f_normal_stand_hitreaction_left_01";
const HIT_RIGHT: &str = "f_normal_stand_hitreaction_right_01";
/// AM_F_Combat_Stand_Block_React.
const BLOCK_REACT: &str = "f_combat_stand_2h_attack_block";
const DEATH: &str = "f_normal_stand_death_01";
const GRABBED_ENTER: &str = "f_normal_grab_enter";
const GRABBED_IDLE: &str = "f_normal_grab_idle";
const DOOR_OPEN_INSIDE: &str = "normal_stand_door_open_inside_forward";
const DOOR_OPEN_OUTSIDE: &str = "normal_stand_door_open_outside_forward";
const DOOR_BURST: &str = "uber_stand_wall_smash_02";
/// Its montage (anims.json: its root motion).
const DOOR_BURST_MONTAGE: &str = "AM_Jason_uber_stand_wall_smash_02";
/// AM_normal_stand_door_break_axe's Enter, Loop and Success sections.
const BREAK_DOOR: [&str; 3] = ["normal_stand_door_break_axe_enter", "normal_stand_door_break_axe_cycle", "normal_stand_door_break_axe_success"];
/// Its Abort section (5.733-6.733 s): BreakDownLoop (FUN_140363a00) sets it next at a BreakDownLoop driver once he's
/// let go of the attack (bWantsToBreakDown cleared) and the blow didn't finish it. Played as `section` [`ABORT`].
const BREAK_ABORT: &str = "normal_stand_door_break_axe_abort";
const ABORT: usize = 3;

/// Sockets of SCCounselorCharacter::UpdateGrabTransform: Jason_Base's CharacterAttachmentSocket, SCKillerCharacter's
/// hand socket (constructor), the counselor's neck socket (native FName).
const GRAB_SOCKET: &str = "GrabSocket";
const HAND_GRAB_SOCKET: &str = "handGrabSocket";
const NECK_GRAB_SOCKET: &str = "NeckGrabSocket";
/// Fallback when the sockets aren't posed yet: how far in front of Jason a held counselor stands.
const GRAB_HOLD_M: f32 = 0.65;
/// Seconds Jason holds a grabbed counselor before the kill (a player's choice in the game).
const HOLD_BEFORE_KILL: f32 = 4.0;
/// How far the computer Jason carries a held counselor to a placed kill (an AI choice).
const CARRY_TO_KILL_M: f32 = 4.0;
/// Where a hold pulled from a car starts (past the grab's enter takes: the car take ends in the hold).
const HOLD_AFTER_CAR_GRAB: f32 = 2.0;
/// SCDriveableVehicle constructor: MinUseCarDoorSpeed (cm/s).
const MIN_USE_CAR_DOOR_SPEED: f32 = 50.0;
/// How near the driver door he pulls her out (m; the special move moves him onto its spot).
const CAR_DOOR_REACH_M: f32 = 1.2;
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
/// His drowning kill (UnderwaterContextKillClass Underwater_GrabKill: Jason_ and Counselor_UnderwaterGrab_CK's
/// AM_*_cinematic_swim_grabkill_01; KillInfo WaterDrown_KillInfo) and its kill camera's key.
const DROWN_KILL: (&str, &str, &str) = ("cinematic_swim_grabkill_01", "cinematic_swim_grabkill_01", "Underwater Grab");
const DROWN_KILL_KEY: &str = "Underwater_GrabKill";
/// The UnderwaterInteractComponent's height over UnderwaterVisualRoot (Jason_Base), and the 50 cm its DistanceLimit
/// adds (DAT_142138908).
const UNDERWATER_INTERACT_HEIGHT_M: f32 = 0.54;
const UNDERWATER_REACH_PAD_M: f32 = 0.5;
/// DrowningDepthTrace's length under her (DAT_14215e7d8, 350 cm).
const DROWNING_DEPTH_TRACE_M: f32 = 3.5;

/// The drowning kill's camera spline (Underwater_GrabKill_C's SCCameraSpline) on the kill.
#[derive(Resource)]
pub(crate) struct DrownKillCamera(Transform);

/// SCThrowable::Use: the knife starts this far ahead of the camera (DAT_142108c74, 120 cm).
const KNIFE_START_M: f32 = 1.2;
/// PLACEHOLDERS (see the module docs): how long he aims, his traps' spacing (m).
const KNIFE_AIM_SECONDS: f32 = 0.6;
/// SCCharacter's tick: how long the attack is held in combat stance before it's the heavy attack (DAT_14207a200).
const HEAVY_HOLD_S: f32 = 0.1;
const TRAP_SPACING_M: f32 = 10.0;
/// ThrowingKnife_C's ProjectileGravityScale; SCThrowable's TimeToDestroy (s).
pub(crate) const KNIFE_GRAVITY: f32 = 9.8 * 8.0;
const KNIFE_LIFE: f32 = 5.0;
/// AM_normal_stand_knife_throw's sections; when the knife leaves (s into the throw section: its ThrowKnife notify).
const KNIFE_SELECT: &str = "normal_stand_knife_select_01";
const KNIFE_AIM: &str = "normal_stand_knife_aim_01";
const KNIFE_THROW: &str = "normal_stand_knife_throw_01";
/// AM_normal_stand_trap_set.
const TRAP_SET: &str = "normal_stand_trap_set";
/// Destroying a fuse box: AM_Break_Electronic_Box_Machete's take.
const BREAK_FUSE: &str = "normal_stand_door_break_machete_enter";
/// Breaking the phone box: the same take in AM_Break_Phone_Box_Machete (its own sound).
const BREAK_PHONE_BOX: &str = "AM_Break_Phone_Box_Machete";

/// The montage his action plays in, when its take's sounds differ by montage (anim_sounds.json's
/// "montage/take" entries).
#[derive(Component)]
pub struct ActionSounds(pub &'static str);
const MORPH_MIN_M: f32 = 50.0;
/// The AI's Morph (PLACEHOLDERS): not before he has hunted this long (s; Morph unlocks first, a second in), and aimed
/// this far short of his goal on his side of it (m), landing no nearer her than [`MORPH_KEEP_OFF_M`]: he closes the
/// distance, he doesn't drop onto her.
const MORPH_GRACE_S: f32 = 60.0;
const MORPH_SHORT_M: f32 = 35.0;
const MORPH_KEEP_OFF_M: f32 = 25.0;
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
    /// The grab kill he always does (an index into [`KILLS`]; `--kill <name>`), else one at random.
    pub kill: Option<usize>,
}

#[derive(Resource)]
pub(crate) struct JasonConfig {
    delay: f32,
    kill: Option<usize>,
}

impl Plugin for JasonPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(JasonConfig { delay: self.delay, kill: self.kill })
            .init_resource::<GrabKillsAvailable>()
            .add_systems(Startup, (spawn_jason, load_knife_look))
            .add_systems(FixedUpdate, (grab_kills_available.before(update_jason), update_jason, recover_counselor, fly_knives, pick_up_knives))
            .add_observer(spawn_knife)
            .add_observer(distortion)
            .add_observer(car_slam)
            .add_systems(Update, (tag_jason_parts, animate_jason, grab_layer, jason_mixes, teleport_mix, jason_door_camera, held_knife.after(animate_jason)));
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
    /// ThrowStrength: the knife's speed (m/s).
    pub throw_strength: f32,
    pub knife_damage: f32,
    pub trap_count: u32,
    pub trap_placement_m: f32,
    /// MaxAttackInteractAngle (degrees) and MaxAttackInteractCounselorDistance (m): the attack's break-down checks
    /// ([`attack_interact`]).
    pub attack_interact_angle: f32,
    pub attack_interact_counselor_m: f32,
    /// Shift's max speed (cm/s) and length (s); Stalk's length and idle rate; Morph's delay (s).
    pub shift_speed: f32,
    pub shift_time: f32,
    pub stalk_active_time: f32,
    pub idle_stalk_modifier: f32,
    /// Under water (SCKillerCharacter): UnderwaterSpeed, MaxSlowRunSpeed (his movement's; cm/s), and
    /// UnderwaterFullSpeedDepth and UnderwaterKillDepth (cm).
    pub underwater_speed: f32,
    pub slow_run_speed: f32,
    /// MaxCombatSpeed (cm/s): his speed in combat stance.
    pub combat_speed: f32,
    pub underwater_full_speed_depth: f32,
    pub underwater_kill_depth: f32,
    pub teleport_screen_time: f32,
    /// MaxTeleportViewDistance (m): how near (flat) the counselor hears his distortion.
    pub max_teleport_view_m: f32,
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
            throw_strength: v("ThrowStrength", 1000.0) / CM,
            knife_damage: v("ThrowingKnifeDamage", 30.0),
            trap_count: v("InitialTrapCount", 5.0) as u32,
            trap_placement_m: v("TrapPlacementDistance", 75.0) / CM,
            attack_interact_angle: v("MaxAttackInteractAngle", 45.0),
            attack_interact_counselor_m: v("MaxAttackInteractCounselorDistance", 200.0) / CM,
            shift_speed: v("ShiftSpeed", 1000.0),
            shift_time: v("ShiftTime", 4.0),
            stalk_active_time: v("StalkActiveTime", 20.0),
            idle_stalk_modifier: v("IdleStalkModifier", 0.1),
            underwater_speed: v("UnderwaterSpeed", 400.0),
            slow_run_speed: v("MaxSlowRunSpeed", 200.0),
            combat_speed: v("MaxCombatSpeed", 200.0),
            underwater_full_speed_depth: v("UnderwaterFullSpeedDepth", 275.0),
            underwater_kill_depth: v("UnderwaterKillDepth", 290.0),
            teleport_screen_time: v("TeleportScreenTime", 1.0),
            max_teleport_view_m: v("MaxTeleportViewDistance", 2000.0) / CM,
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
    /// Breaking through a destructible wall (the door break's montage: Start, Loops, Success; a blow per section).
    BreakingWall { wall: Entity, t: f32, section: usize, hit: bool },
    /// Raged: bursting through a closed door (s into it).
    BurstingDoor { door: Entity, t: f32 },
    /// Reaching for a grab (the grab montage's Default section); it connects or misses at its PerformGrabTest.
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
    /// Hit without being stunned: a short flinch (its hit reaction, or a blocked hit's).
    Flinching { t: f32, anim: &'static str },
    /// Shifting (f13-core::killer_moves).
    Shifting { shift: Shift },
    /// Morphing: s into it, and where he'll be.
    Morphing { t: f32, to: Transform },
    /// Throwing a knife: s into it, and whether it has left his hand.
    Throwing { t: f32, thrown: bool, release: Option<f32> },
    /// Setting a trap: s into it.
    PlacingTrap { t: f32 },
    /// Stopping a car dead (AM_normal_stand_car_stop_slam: his hands on its hood).
    Slamming { t: f32 },
    /// Pulling the driver out of her car (DriverGrab's paired take), into his grab.
    PullingOut { t: f32 },
    /// Destroying a fuse box (Jason_Generator_Destruction_Machete's montage), or the phone box
    /// (Jason_PhoneBox_Destruction_Machete's: the same take): it goes in the montage's DestroyGenerator driver
    /// window.
    BreakingFuse { t: f32, fuse: Entity, done: bool },
    /// The counselor is dead.
    Done,
}

impl Jason {
    /// Between his attacks (the attack delay after a swing or a missed grab).
    pub fn recovering(&self) -> bool {
        self.cooldown > 0.0
    }

    /// Whether a stun takes now: not right after one (stun immunity), and in a state a hit can interrupt.
    pub fn stunnable(&self) -> bool {
        self.stun_immune <= 0.0
            && matches!(
                self.state,
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
                    | JasonState::BreakingFuse { .. }
                    | JasonState::Slamming { .. }
            )
    }
}

#[derive(Component)]
pub struct Jason {
    pub state: JasonState,
    /// In combat stance (the player's C, GLB_Combat_PC: CurrentStance 5).
    pub combat: bool,
    /// Blocking (BlockData.bIsBlocking): CMBT_KLR_Block held in combat stance.
    pub blocking: bool,
    /// The context kill under way: its actor's class and kill component (its KillInfo: his score as she dies).
    kill_score: Option<(String, &'static str)>,
    /// The attack button's hold in combat stance (SCCharacter +0xf4c / +0xf50: from a press that swung).
    attack_hold: Option<f32>,
    /// Horizontal velocity, cm/s.
    pub velocity: Vec2,
    vel_y: f32,
    grounded: bool,
    path: Vec<Vec3>,
    /// Whether the path gets to the goal (else it ends at the nearest point it can reach).
    path_reaches: bool,
    /// At the end of a path that stops short of a goal in the water (the shore): straight on from here.
    at_shore: bool,
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
    /// Seconds spent hunting (the AI's Morph waits [`MORPH_GRACE_S`]).
    hunted: f32,
    /// Whether the Shift's sound is playing.
    shift_sound: bool,
    /// Just arrived from a Morph (the AI Stalks from there).
    morphed: bool,
    pub stalk: Option<Stalk>,
    pub fade: TeleportFade,
    /// Knives and traps left; the knife cooldown; where his traps are.
    pub knives: u32,
    /// The grab kill he's doing ([`KILLS`]).
    pub kill: usize,
    /// The kill he's doing: his take, hers, its name (a grab kill's, or a placed kill's).
    pub kill_move: (&'static str, &'static str, &'static str),
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
    /// Under water (bUnderWater: in the water with his capsule's top under its surface), his speed there, and
    /// whether he's hidden for it.
    pub underwater: bool,
    underwater_speed: Option<f32>,
    under_hidden: bool,
    /// The kill he's doing holds her on his GrabSocket (grab and placed kills; not the drowning kill).
    kill_holds: bool,
    /// The played Jason's best interactable of the doors and walls ([`killer_focus`]).
    focus: Option<Focus>,
    /// bIsRunning: his fast walk (KLR_FastWalk: OnStart/Stop/ToggleFastWalk -> SetRunning, vt+0xda0) or the AI's
    /// run; his footsteps take the run cue for it (Jason_AnimBP's GetFootstepSound).
    pub running: bool,
}

impl Jason {
    /// Moved by the host (host.rs): plan again from here, on his feet.
    pub(crate) fn teleported(&mut self) {
        self.path.clear();
        self.replan = 0.0;
        self.vel_y = 0.0;
        self.progress = (Vec3::ZERO, 0.0);
    }

    /// As if he had hunted past the AI's wait before its first Morph (--bot-abilities).
    pub(crate) fn skip_morph_grace(&mut self) {
        self.hunted = MORPH_GRACE_S;
    }

    pub fn stalking(&self) -> bool {
        self.stalk.is_some()
    }

    /// The closed door the played Jason has as his best interactable.
    pub(crate) fn focus_door(&self) -> Option<Entity> {
        match self.focus {
            Some(Focus::Door(d)) => Some(d),
            _ => None,
        }
    }

    pub fn shifting(&self) -> bool {
        matches!(self.state, JasonState::Shifting { .. })
    }

    pub fn stunned(&self) -> bool {
        matches!(self.state, JasonState::Stunned { .. })
    }

    /// In a context kill (bInContextKill): a grab, placed or hiding spot kill.
    pub fn in_context_kill(&self) -> bool {
        matches!(self.state, JasonState::Killing { .. } | JasonState::HideKill { .. })
    }

    /// Holding her in his grab (she can struggle), not killing her.
    pub fn holding(&self) -> bool {
        matches!(self.state, JasonState::Holding { .. })
    }

    /// His stun's minigame while it runs: its completion and the stun class's StunText.
    pub fn stun_minigame(&self) -> Option<(f32, &'static str)> {
        match self.state {
            JasonState::Stunned { stun, .. } if self.stun_done.is_none() && stun.minigame().is_some() => Some((self.stun_wiggle.completion, stun.text()?)),
            _ => None,
        }
    }

    /// How long he's held her (from the grab's enter), while he does.
    pub fn holding_for(&self) -> Option<f32> {
        match self.state {
            JasonState::Holding { t } => Some(t),
            _ => None,
        }
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

/// The break-down's Abort section, `at` s into it: done (back to hunting) once it has played out, else on.
fn abort(j: &mut Jason, killer: &KillerCharacter, at: f32) -> bool {
    let done = at >= killer.length(BREAK_ABORT).unwrap_or(1.0);
    j.state = match (done, &j.state) {
        (true, _) => JasonState::Hunting,
        (false, JasonState::BreakingDoor { door, .. }) => JasonState::BreakingDoor { door: *door, t: at, section: ABORT, hit: false },
        (false, JasonState::BreakingWall { wall, .. }) => JasonState::BreakingWall { wall: *wall, t: at, section: ABORT, hit: false },
        (false, s) => s.clone(),
    };
    done
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
    /// PoliceShootJasonStun_C: the police turned him back from their escape.
    Police,
}

impl Stun {
    pub fn seconds(self) -> f32 {
        match self {
            Stun::Weapon { .. } | Stun::Flare => 5.0,
            Stun::Shotgun => 7.0,
            Stun::Firecracker => 4.0,
            Stun::PocketKnife | Stun::GrabBreak | Stun::Police => 6.0,
            Stun::BearTrap => 10.0,
        }
    }

    /// The score event for stunning him this way:
    /// - the item's own: Shotgun's StunScoreEventClass, BP_Flare_Projectile's FlareStunScoreEventClass,
    ///   BP_Firecracker_Projectile's StunJasonScoreEventClass, BearTrap's TrapScoreEvent;
    /// - else the counselor's StunJasonScoreEvent;
    /// - breaking his grab is her EscapeJasonGrabScoreEvent;
    /// - the police's isn't hers.
    pub fn score_event(self) -> Option<&'static str> {
        Some(match self {
            Stun::Weapon { .. } => "StunJason",
            Stun::Shotgun => "StunJasonShotgun",
            Stun::Flare => "StunJasonFlare",
            Stun::Firecracker => "FirecrackerJasonStun",
            Stun::BearTrap => "Trapper",
            Stun::PocketKnife | Stun::GrabBreak => "EscapeJasonGrab",
            Stun::Police => return None,
        })
    }

    /// The stun class's StunText (the minigame's prompt): BaseWeaponStun and HeavyShotgunStun "Rise",
    /// FlareGunLightStun "Brush Off", BearTrapLeftFootStun "Open Trap", PoliceShootJasonStun "Resist".
    pub fn text(self) -> Option<&'static str> {
        Some(match self {
            Stun::Weapon { .. } | Stun::Shotgun => "Rise",
            Stun::Flare => "Brush Off",
            Stun::BearTrap => "Open Trap",
            Stun::Police => "Resist",
            Stun::Firecracker | Stun::PocketKnife | Stun::GrabBreak => return None,
        })
    }

    /// The stun's minigame (bPlayMinigame), which he can wiggle out of early: TotalStunTime is [`Stun::seconds`],
    /// PercentPerWiggle SCStunDamageType's constructor 5, DepletionRate the class's (BaseWeaponStun 0.3, the flare and
    /// shotgun stuns 0.2, the bear trap's the constructor 0.02). The firecracker and grab-break stuns have none.
    pub fn minigame(self) -> Option<f13_core::stun::WiggleTuning> {
        let depletion_rate = match self {
            Stun::Weapon { .. } => 0.3,
            Stun::Flare | Stun::Shotgun | Stun::Police => 0.2,
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
            Stun::Police => ("combat_stand_stunned_police_02", Some("combat_stand_stunned_police_loop_01"), Some("combat_stand_stunned_police_exit_01")),
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
    /// Its dart (shown once it stops).
    dart: Option<Entity>,
}

/// The velocity that arcs a knife from `from` onto `to` at `speed` (his ThrowStrength, m/s) under its gravity (the low
/// arc); straight at it when it's out of reach.
fn knife_velocity(from: Vec3, to: Vec3, speed: f32) -> Vec3 {
    let d = to - from;
    let flat = Vec2::new(d.x, d.z);
    let x = flat.length().max(1e-3);
    let (v, g, y) = (speed, KNIFE_GRAVITY, d.y);
    let disc = v.powi(4) - g * (g * x * x + 2.0 * y * v * v);
    if disc < 0.0 {
        return d.normalize_or_zero() * v;
    }
    let angle = ((v * v - disc.sqrt()) / (g * x)).atan();
    let h = flat / x * v * angle.cos();
    Vec3::new(h.x, v * angle.sin(), h.y)
}

/// ThrowingKnife_C's dart (its SkeletalMesh component): its scene and its place on the knife, thrown and in his hand.
/// The knife's Blueprint turns it to pitch 270 and hides it as the ProjectileMovement activates (the throw: the
/// particle trail shows instead) and shows it when the projectile stops; in his hand it's at its own place.
#[derive(Resource)]
struct KnifeLook(Handle<WorldAsset>, Transform, Transform);

/// The knife in his hand while he aims (SERVER_SpawnKnife 0x1404051d0: a ThrowingKnifeClass actor, CurrentThrowable,
/// attached to his Mesh at KnifeAttachSocket, SnapToTargetNotIncludingScale; the constructor's KnifeAttachSocket is
/// handGrabSocket), until it leaves his hand at the throw or he puts it away.
#[derive(Component)]
struct HeldKnife;

fn load_knife_look(mut commands: Commands, export: Res<crate::ExportRoot>, assets: Res<AssetServer>) {
    use f13_core::camera_spline::quaternion;
    let Ok(classes) = f13_data::classes::ClassDb::load(&export.0.join("classes.json")) else { return };
    let Some((file, local)) = classes.get("ThrowingKnife_C").and_then(|c| c.components.iter().find(|c| c.name == "SkeletalMesh")).and_then(|c| Some((c.mesh.clone()?, c.local?))) else {
        warn!("ThrowingKnife_C's dart isn't in classes.json (re-export: f13-export levels): stuck knives aren't shown");
        return;
    };
    // K2_SetRelativeRotation(FRotator(270, 0, 0)) as it's thrown.
    let held = crate::to_transform(&local);
    let local = f13_data::level::Xform { r: quaternion(Vec3::new(270.0, 0.0, 0.0)).to_array(), ..local };
    commands.insert_resource(KnifeLook(assets.load(GltfAssetLabel::Scene(0).from_asset(file)), crate::to_transform(&local), held));
    if let Some(c) = classes.get("Underwater_GrabKill_C").and_then(|c| c.components.iter().find(|c| c.kind == "SCCameraSplineComponent")) {
        commands.insert_resource(DrownKillCamera(crate::to_transform(&c.local.unwrap_or(f13_data::level::Xform::IDENTITY))));
    }
    // The grab kills' KillBounds and HUD icons.
    let textures: std::collections::HashMap<String, String> =
        std::fs::read_to_string(export.0.join("ui").join("textures.json")).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default();
    let mut info = GrabKillInfo::default();
    for (i, (_, class, _)) in KILLS.iter().enumerate() {
        let Some(c) = classes.get(&format!("{class}_C")) else {
            warn!("{class}_C isn't in classes.json (re-export: f13-export levels): its grab kill always fits and has no icon");
            continue;
        };
        if let Some(b) = c.components.iter().find(|c| c.name == "KillBounds") {
            let e = &b.props["BoxExtent"];
            let v = |k: &str| e[k].as_f64().unwrap_or(32.0) as f32;
            info.bounds[i] = Some((crate::to_transform(&b.local.unwrap_or(f13_data::level::Xform::IDENTITY)), Vec3::new(v("X"), v("Z"), v("Y")) / 100.0));
        }
        let icon = c.props["ContextKillHUDIcon"].as_str().and_then(|s| s.split('\'').nth(1));
        info.icons[i] = icon.and_then(|n| textures.get(n).cloned());
    }
    commands.insert_resource(info);
}

/// IsGrabKillAvailable for each slot while he holds her (see [`GrabKillsAvailable`]).
#[allow(clippy::type_complexity)]
fn grab_kills_available(
    info: Option<Res<GrabKillInfo>>,
    killer: Option<Res<KillerCharacter>>,
    mut available: ResMut<GrabKillsAvailable>,
    jasons: Query<(&Jason, &Transform)>,
    (collision, dynamic, no_world_dynamic, not_world_dynamic): (
        Option<Res<crate::collision::WorldCollision>>,
        Res<crate::collision::DynamicColliders>,
        Query<(), With<crate::collision::NoWorldDynamic>>,
        Option<Res<crate::collision::NotWorldDynamic>>,
    ),
) {
    // bGrabKillsActive: his grab's enter played out.
    let active = |j: &Jason| matches!(j.state, JasonState::Holding { t } if t >= killer.as_ref().and_then(|k| k.length(GRAB_ENTER)).unwrap_or(0.0));
    let Some((_, xf)) = jasons.iter().find(|(j, _)| j.holding() && active(j)) else {
        available.set_if_neq(GrabKillsAvailable([false; 4]));
        return;
    };
    let origin = Transform::from_matrix(xf.to_matrix() * Transform::from_rotation(crate::character::FACING).to_matrix());
    let fits = GRAB_KILL_SLOTS.map(|k| {
        // No kill in the slot: never available.
        let Some(k) = k else { return false };
        let (Some(world), Some((local, half))) = (collision.as_ref(), info.as_ref().and_then(|i| i.bounds[k])) else { return true };
        let w = origin.mul_transform(local);
        let b = f13_core::collision::Obb { centre: w.translation, axes: [w.rotation * Vec3::X, w.rotation * Vec3::Y, w.rotation * Vec3::Z], half: half * w.scale.abs() };
        let (grid, lists) = world.without(&dynamic, |e| no_world_dynamic.contains(e));
        let level = crate::collision::Excluding(grid, not_world_dynamic.as_ref().map(|g| &g.0));
        !f13_core::collision::box_blocked(&(&level, lists.as_slice()), &b)
    });
    if available.set_if_neq(GrabKillsAvailable(fits)) {
        info!("Jason: grab kills that fit (right, bottom, left, top): {fits:?}");
    }
}

/// The knife's rotation: its forward (Unreal +X) along its velocity (its ProjectileMovement's bRotationFollowsVelocity).
fn knife_rotation(vel: Vec3) -> Quat {
    Transform::IDENTITY.looking_to(vel.normalize_or(Vec3::X), Vec3::Y).rotation * Quat::from_rotation_y(std::f32::consts::FRAC_PI_2)
}

/// His knife in hand: shown on KnifeAttachSocket while he aims, until it's thrown (or put away).
#[allow(clippy::type_complexity)]
fn held_knife(
    mut commands: Commands,
    look: Option<Res<KnifeLook>>,
    killer: Option<Res<KillerCharacter>>,
    jasons: Query<&Jason>,
    rigs: Query<&Rig, Without<HeldKnife>>,
    globals: Query<&GlobalTransform>,
    mut held: Query<(Entity, &mut Transform), With<HeldKnife>>,
) {
    let (Some(look), Some(killer)) = (look, killer) else { return };
    let aiming = jasons.iter().any(|j| matches!(j.state, JasonState::Throwing { thrown: false, .. }));
    let hand = rigs.iter().find(|r| matches!(r.model, crate::character::Model::Killer)).and_then(|r| r.socket(&killer.info, HAND_GRAB_SOCKET, &globals));
    match (aiming.then_some(hand).flatten(), held.single_mut()) {
        (Some(hand), Ok((_, mut xf))) => *xf = hand,
        (Some(hand), Err(_)) => {
            let knife = commands.spawn((HeldKnife, hand, Visibility::default())).id();
            commands.spawn((WorldAssetRoot(look.0.clone()), look.2, ChildOf(knife)));
        }
        (None, Ok((e, _))) => commands.entity(e).despawn(),
        (None, Err(_)) => {}
    }
}

fn spawn_knife(ev: On<ThrowKnife>, mut commands: Commands, look: Option<Res<KnifeLook>>) {
    let knife = commands.spawn((Transform::from_translation(ev.from).with_rotation(knife_rotation(ev.vel)), Visibility::default())).id();
    let dart = look.map(|l| commands.spawn((WorldAssetRoot(l.0.clone()), l.1, Visibility::Hidden, ChildOf(knife))).id());
    commands.entity(knife).insert(Knife { vel: ev.vel, age: 0.0, damage: ev.damage, stuck: false, dart });
    // MULTICAST_Use (0x1403f7170) activates its ParticleSystem (P_Jason_ThrowingKnife_01 on its root): the spinning
    // dart (a mesh emitter) while it flies; its ribbon trail (TracerTrail) isn't drawn.
    commands.write_message(crate::particles::SpawnParticles {
        system: "P_Jason_ThrowingKnife_01".into(),
        transform: Transform::IDENTITY,
        follow: Some(knife),
        actor: String::new(),
        active: true,
    });
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
                    commands.trigger(crate::match_flow::KillerScore("Butcher"));
                    commands.trigger(crate::match_flow::KillerScore("KillCounselor"));
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
            // OnProjectileStop: the dart shows.
            if let Some(d) = k.dart {
                commands.entity(d).insert(Visibility::Inherited);
            }
            continue;
        }
        xf.translation = to;
        xf.rotation = knife_rotation(k.vel);
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

/// SCKillerCharacter::PlayDistortionEffect (0x1403ffd50), from OnRep_Teleport (a Morph starting and ending) and
/// OnRep_Stalk (Stalk starting): where he is and, Morphing, where he's going (+0x1480, cleared as he arrives).
#[derive(Event, Clone, Copy, Debug)]
pub struct Distortion {
    pub at: Vec3,
    pub dest: Option<Vec3>,
}

/// PlayDistortionEffect on the local counselor: within MaxTeleportViewDistance (flat) of him or of his Morph's
/// destination, or with him in her view (KillerInViewPtr), she hears TeleportCueCounselorHears (PlaySound2D) and
/// TeleportCounselorHearsMix is pushed (cleared as the effect's TeleportScreenTime runs out: [`teleport_mix`]), with
/// TeleportScreenEffectInst (not ported). Not when the local player is Jason.
fn distortion(
    ev: On<Distortion>,
    mut commands: Commands,
    tuning: Option<Res<KillerTuning>>,
    play: Option<Res<crate::jason_player::PlayJason>>,
    her: Query<(&Transform, Option<&crate::fear_events::Pushed>), With<Player>>,
) {
    let (Some(t), None, Ok((xf, pushed))) = (tuning, play, her.single()) else { return };
    let flat = |a: Vec3| Vec2::new(a.x - xf.translation.x, a.z - xf.translation.z).length();
    let near = flat(ev.at) <= t.max_teleport_view_m || ev.dest.is_some_and(|d| flat(d) <= t.max_teleport_view_m);
    if near || pushed.is_some_and(|p| p.spotted) {
        info!("Vanessa: hears Jason's distortion (the teleport static)");
        commands.trigger(crate::audio::PlaySound { cue: TELEPORT_HEARD_CUE.into(), at: None });
        commands.trigger(crate::audio::PushMix("TeleportCounselorHearsSoundMix"));
        commands.insert_resource(TeleportEffect { left: t.teleport_screen_time, total: t.teleport_screen_time });
    }
}

/// The counselor's wiggle input (GLB_Wiggle: -1 while E is down) and her grab struggle's minigame (f13-core::stun).
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
/// The level's fuse boxes' class (power.rs).
const FUSE_BOX_CLASS: &str = "PowerGridFuseBox_C";
/// Jason_AnimBP's BlockMontages (AM_Jason_Combat_Block: its block idle, looped) and BlockingHitMontages (each ends
/// at its OnEndBlockReaction notify: OnEndBlockReation -> OnRep_Block, the block montage again).
pub(crate) const BLOCK: &str = "combat_stand_1h_block_idle_01";
const BLOCK_REACTIONS: [&str; 2] = ["combat_stand_1h_block_reaction_01", "combat_stand_1h_block_reaction_02"];
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
        Transform { translation: feet, rotation: Quat::from_rotation_y(yaw + std::f32::consts::FRAC_PI_2), ..default() }
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
                combat: false,
                blocking: false,
                kill_score: None,
                attack_hold: None,
                velocity: Vec2::ZERO,
                vel_y: 0.0,
                grounded: true,
                path: Vec::new(),
                path_reaches: true,
                at_shore: false,
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
                hunted: 0.0,
                shift_sound: false,
                morphed: false,
                stalk: None,
                fade: TeleportFade::default(),
                knives: tuning.num_knives,
                kill: 0,
                kill_move: (KILLS[0].0, KILLS[0].0, KILLS[0].2),
                traps: tuning.trap_count,
                knife_cooldown: 0.0,
                trap_spots: Vec::new(),
                faded: false,
                stun_wiggle: default(),
                stun_done: None,
                next_wiggle: (-1.0, 0.0),
                underwater: false,
                underwater_speed: None,
                under_hidden: false,
                kill_holds: true,
                focus: None,
                running: false,
            },
            Transform { translation: feet, rotation, ..default() },
            Visibility::default(),
            Name::new("Jason"),
            crate::character::LayeredBlend::new(GRAB_LAYER_FILTERS, true, GRAB_IDLE),
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
    // Under water his speed is the depth's (GetMaxSpeed 0x1403b7ab0) and he slides (OnRep_UnderWater: GroundFriction 0).
    let max_speed = j.underwater_speed.unwrap_or(max_speed);
    let movement = if j.underwater { MovementSettings { ground_friction: 0.0, ..t.movement } } else { t.movement };
    let accel = input_acceleration(wish.to_array(), &movement);
    let mut v = j.velocity.to_array();
    calc_velocity(&mut v, accel, max_speed, dt, &movement);
    j.velocity = Vec2::from_array(v);
    slide(j, xf, face, dt, t, world);
}

/// A montage's root motion moving him (the character movement's root motion velocity, his input ignored): `local`
/// in his own frame (x right, y forward), cm/s, against the world as he walks.
fn root_move(j: &mut Jason, xf: &mut Transform, local: Vec2, face: Option<Vec2>, dt: f32, t: &KillerTuning, world: &(impl f13_core::collision::TriSource + ?Sized)) {
    let yaw = yaw_of(xf.rotation);
    let fwd = Vec2::new(yaw.sin(), yaw.cos());
    let right = Vec2::new(-fwd.y, fwd.x);
    j.velocity = fwd * local.y + right * local.x;
    slide(j, xf, face, dt, t, world);
}

/// Moves him by his velocity against the world, turning to `face` (else his motion).
fn slide(j: &mut Jason, xf: &mut Transform, face: Option<Vec2>, dt: f32, t: &KillerTuning, world: &(impl f13_core::collision::TriSource + ?Sized)) {
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

/// A standing destructible wall within reach ahead of `wish` (its section's middle, 1.6 m up).
fn wall_in_way(walls: &Query<(Entity, &mut crate::doors::Wall, &GlobalTransform)>, here: Vec3, wish: Vec2) -> Option<Entity> {
    if wish == Vec2::ZERO {
        return None;
    }
    walls.iter().filter(|(_, w, _)| !w.broken).find_map(|(e, _, g)| {
        let to = Vec2::new(g.translation().x - here.x, g.translation().z - here.z);
        (to.length() < DOOR_REACH_M && to.normalize_or_zero().dot(wish) > 0.3 && (g.translation().y - here.y).abs() < 2.0).then_some(e)
    })
}

/// A door or destructible wall the played Jason's interactable manager has as its best interactable.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Focus {
    Door(Entity),
    Wall(Entity),
}

/// The played Jason's best interactable of the doors and walls (what E and his attack act on): the interactable
/// manager's choice (ILLInteractableManagerComponent, interact.rs `score`) of those in range (DistanceLimit from his
/// capsule's centre) and within their InteractComponents' facing limits, the doors only while closed (SCDoor's
/// CanInteractWith 0x140339f10 gives the killer the press on a closed door when he isn't holding her) and the walls
/// while standing. (interact.rs's occlusion traces aren't run for him.)
#[allow(clippy::too_many_arguments)]
fn killer_focus(
    doors: &Query<(Entity, &mut crate::doors::Door, &GlobalTransform)>,
    walls: &Query<(Entity, &mut crate::doors::Wall, &GlobalTransform)>,
    limits: &Query<&crate::interact::InteractLimits>,
    centre: Vec3,
    look: Vec3,
    facing: Vec3,
    moving: Option<Vec3>,
    current: Option<Focus>,
) -> Option<Focus> {
    let mut best: Option<(f32, Focus)> = None;
    let mut consider = |owner: Entity, g: &GlobalTransform, component: &str, point: Vec3, reach_cm: f32, f: Focus| {
        if point.distance(centre) * 100.0 > reach_cm {
            return;
        }
        if limits.get(owner).ok().and_then(|l| l.0.get(component)).is_some_and(|l| !l.facing(g, centre - point, look)) {
            return;
        }
        let score = crate::interact::score(centre, point, reach_cm, look, facing, moving, current == Some(f));
        if best.is_none_or(|(b, _)| score > b) {
            best = Some((score, f));
        }
    };
    for (e, d, g) in doors.iter().filter(|(_, d, _)| !d.open && !d.broken) {
        consider(e, g, "InteractComponent", g.transform_point(d.interact), d.reach, Focus::Door(e));
    }
    for (e, w, g) in walls.iter().filter(|(_, w, _)| !w.broken) {
        consider(e, g, "Interactable", g.transform_point(w.interact), w.reach, Focus::Wall(e));
    }
    best.map(|(_, f)| f)
}

/// SCKillerCharacter's attack-to-interact checks (FUN_1403df5d0 doors, FUN_1403df9b0 walls): his view (flat) within
/// acos(clamp(`angle_deg` in radians, -1, 1)) of the direction to the interactable (the game takes the arc cosine of
/// the angle in radians: the doors' 90, which FUN_14040fd00 sets for the call, gives a dot of 0; the walls'
/// MaxAttackInteractAngle 45 gives 0.667), and no living counselor within MaxAttackInteractCounselorDistance of him
/// (3D) on his side of it (`her`: where she is and whether she's on his side).
fn attack_interact(view: Vec2, here: Vec3, point: Vec3, angle_deg: f32, near_m: f32, her: Option<(Vec3, bool)>) -> bool {
    let to = Vec2::new(point.x - here.x, point.z - here.z).normalize_or_zero();
    if view.normalize_or_zero().dot(to) < angle_deg.to_radians().clamp(-1.0, 1.0).acos() {
        return false;
    }
    !her.is_some_and(|(at, same_side)| same_side && at.distance(here) <= near_m)
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
    (time, anims, water, drown): (Res<Time>, Res<crate::Anims>, Option<Res<crate::water::Water>>, Option<Res<DrownKillCamera>>),
    tuning: Option<Res<KillerTuning>>,
    ptuning: Res<PlayerTuning>,
    nav: Option<ResMut<Nav>>,
    (killer, counselor_model): (Option<Res<KillerCharacter>>, Option<Res<crate::character::CounselorCharacter>>),
    (collision, dynamic): (Res<crate::collision::WorldCollision>, Res<crate::collision::DynamicColliders>),
    (mut rng, mut struggle, mut controls, config): (ResMut<crate::GameRng>, ResMut<Struggle>, Option<ResMut<crate::jason_player::JasonControls>>, Res<JasonConfig>),
    (mut jasons, morph_points, mut morph_map, grab_kills): (
        Query<(Entity, &mut Jason, &mut Transform, Option<&HitJason>, Option<&mut crate::senses::Knowledge>), Without<Player>>,
        Option<Res<MorphPoints>>,
        Option<ResMut<crate::jason_player::MorphMap>>,
        Res<GrabKillsAvailable>,
    ),
    (inventories, items, mut fuse_boxes, (mut grids, mut phone_boxes, mut phones), vehicles): (
        Query<&crate::interact::Inventory>,
        Query<&crate::props::Item>,
        Query<(Entity, &mut crate::power::FuseBox)>,
        (Option<ResMut<crate::power::PowerGrids>>, Query<(Entity, &mut crate::police::PhoneBox)>, Query<&mut crate::police::Phone>),
        Query<(&crate::props::Vehicle, &GlobalTransform)>,
    ),
    mut players: Query<(Entity, &mut Player, &mut Transform, Option<&crate::vehicle::Driving>, Option<&mut crate::hiding::Hiding>, Has<crate::items::Blocking>), Without<Jason>>,
    mut doors: Query<(Entity, &mut crate::doors::Door, &GlobalTransform)>,
    (mut barricades, mut walls): (Query<&mut crate::windows::Barricade>, Query<(Entity, &mut crate::doors::Wall, &GlobalTransform)>),
    (mut spots, cabinets, mut placed_kills): (Query<(Entity, &mut crate::hiding::HidingSpot, &GlobalTransform)>, Query<&GlobalTransform, With<crate::props::Cabinet>>, Query<&mut crate::placed_kills::PlacedKill>),
    mut rigs: Query<(&mut Rig, &ChildOf, &Transform), (Without<Jason>, Without<Player>)>,
    (globals, navmesh, leaves, parents, interact_limits): (
        Query<&GlobalTransform>,
        Option<Res<crate::navigation::Navmesh>>,
        Query<(Entity, &ChildOf), With<crate::navigation::NotOnNavmesh>>,
        Query<&ChildOf>,
        Query<&crate::interact::InteractLimits>,
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
    let Ok((pe, mut p, mut pxf, in_car, mut hiding, blocking)) = players.single_mut() else { return };
    let driving = in_car.is_some();
    // Her car's driver door (the game's DriverGrab context kill, where he stands to pull the driver out) and its speed.
    let car_door = in_car.and_then(|d| vehicles.get(d.vehicle).ok()).map(|(v, g)| {
        let at = v.driver_grab.map_or_else(
            || {
                let seat = v.driver_seat;
                let out = Vec3::new(seat.x, 0.0, seat.z).normalize_or_zero();
                Transform::from_translation(g.transform_point(seat + out))
            },
            |grab| crate::hiding::HidingSpot::world(g, &grab),
        );
        (at, v.speed)
    });
    for (je, mut j, mut xf, hit, mut know) in &mut jasons {
        // Under water (his tick 0x14040d900): in the water with his capsule's top under its surface; there he walks
        // the bottom at UnderwaterSpeed scaled by his feet's depth (GetUnderwaterDepthModifier: up to
        // UnderwaterFullSpeedDepth) from MaxSlowRunSpeed, and he's unseen outside his special moves
        // (NativeOnUnderwaterUpdate: his mesh, mask and weapon hidden).
        let centre = xf.translation + Vec3::Y * t.capsule.half_height;
        let surface = water.as_ref().and_then(|w| w.surface_at(centre));
        let under = surface.is_some_and(|s| centre.y + t.capsule.half_height <= s);
        if under != j.underwater {
            j.underwater = under;
            info!("Jason: {} the water", if under { "under" } else { "out of" });
            // OnRep_UnderWater (0x1403fdb30): going under drops a counselor he holds, unless in a context kill
            // (MULTICAST_DropCounselor 0x1403f6450: his grab and its animation end). In the water she isn't moved
            // or staggered (SCCounselorCharacter::OnDropped 0x1403c1610 with bInWater): her grab and its animation
            // end and she swims (or stands, in shallow water) from where he held her.
            if under && matches!(j.state, JasonState::Holding { .. }) {
                info!("Jason: dropped her going under");
                commands.entity(pe).remove::<(Held, PlayAction)>();
                commands.entity(je).remove::<PlayAction>();
                j.state = JasonState::Hunting;
            }
            // And ends a Shift.
            if under && matches!(j.state, JasonState::Shifting { .. }) {
                info!("Jason: Shift over (under the water)");
                j.state = JasonState::Hunting;
            }
        }
        j.underwater_speed = surface.filter(|_| under).map(|s| {
            let depth = ((s - xf.translation.y) * CM).clamp(0.0, t.underwater_full_speed_depth);
            let m = (depth / t.underwater_full_speed_depth).clamp(0.0, 1.0);
            (t.underwater_speed - t.slow_run_speed) * m + t.slow_run_speed
        });
        let hide = under && !matches!(j.state, JasonState::Killing { .. });
        if hide != j.under_hidden {
            j.under_hidden = hide;
            commands.entity(je).insert(if hide || j.faded { Visibility::Hidden } else { Visibility::Inherited });
        }
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
        // The Shift's rush (a loop) lasts as long as the Shift, however it ends.
        if j.shifting() != j.shift_sound {
            j.shift_sound = j.shifting();
            if j.shift_sound {
                commands.trigger(crate::audio::ActorLoop { actor: je, prop: SHIFT_CUE });
            } else {
                commands.trigger(crate::audio::StopLoop { actor: je, prop: SHIFT_CUE });
            }
        }
        let effect = matches!(j.state, JasonState::Shifting { .. } | JasonState::Morphing { .. });
        j.fade.tick(dt, effect);
        let faded = j.fade.0 < 0.5;
        if faded != j.faded {
            j.faded = faded;
            commands.entity(je).insert(if faded || j.under_hidden { Visibility::Hidden } else { Visibility::Inherited });
        }
        // A counselor's item hit him.
        if let Some(h) = hit {
            commands.entity(je).remove::<HitJason>();
            commands.trigger(crate::match_flow::JasonDamaged);
            // Her weapon's damage (the item stuns below score their own).
            if matches!(h.stun, None | Some(Stun::Weapon { .. })) {
                commands.trigger(Score("DamageJason"));
            }
            let can_stun = j.stunnable();
            // Blocking (SCKillerCharacter's TakeDamage 0x14040b600): a hit whose damage type can be blocked
            // (bCanBeBlocked: only BaseWeaponStun among his stuns; a shotgun never) doesn't stun him, takes
            // BlockedHitDamageModifier of its damage (his health isn't modelled), and plays one of his
            // BlockingHitMontages (FUN_1402fdbb0 with BlockData.bIsBlocking).
            let blocked = j.blocking && matches!(h.stun, None | Some(Stun::Weapon { .. }));
            match h.stun {
                _ if blocked => {
                    let anim = BLOCK_REACTIONS[rng.0.index(BLOCK_REACTIONS.len())];
                    info!("Jason: blocked the hit");
                    j.state = JasonState::Flinching { t: 0.0, anim };
                    commands.entity(je).insert(PlayAction(anim.into()));
                }
                Some(stun) if can_stun => {
                    info!("Jason: stunned ({stun:?}, {:.0}s)", stun.seconds());
                    if let Some(event) = stun.score_event() {
                        commands.trigger(Score(event));
                        commands.trigger(crate::audio::ActorSound { actor: pe, prop: "VO:JasonStun" });
                    }
                    if matches!(j.state, JasonState::Holding { .. }) {
                        let in_water = water.as_ref().is_some_and(|w| w.surface_at(xf.translation + Vec3::Y * t.capsule.half_height).is_some());
                        drop_counselor(&mut pxf, &xf, t.capsule.half_height, in_water, &world);
                        release(&mut commands, pe, GRAB_BREAK, counselor_model.as_deref());
                    }
                    j.state = JasonState::Stunned { t: 0.0, stun };
                    j.velocity = Vec2::ZERO;
                    commands.entity(je).insert(PlayAction(stun.anims().0.into()));
                }
                _ if matches!(j.state, JasonState::Hunting) => {
                    let front = angle_to(xf.rotation, Vec2::new(h.from.x - xf.translation.x, h.from.z - xf.translation.z)) < 90.0;
                    info!("Jason: hit, not stunned");
                    let anim = if front { FLINCH_FRONT } else { FLINCH_BACK };
                    j.state = JasonState::Flinching { t: 0.0, anim };
                    commands.entity(je).insert(PlayAction(anim.into()));
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
                j.hunted += dt;
                // Her in a car that's (nearly) stopped, him at its driver door: he pulls her out into his grab
                // (DriverGrab; the car door only works under MinUseCarDoorSpeed). The AI does it when he gets there,
                // the player when they grab.
                if let Some((door, speed)) = car_door {
                    let at_door = Vec2::new(door.translation.x - xf.translation.x, door.translation.z - xf.translation.z).length() < CAR_DOOR_REACH_M;
                    let wants = player_input.map_or(true, |i| i.presses.grab);
                    if at_door && speed.abs() < MIN_USE_CAR_DOOR_SPEED && wants && alive {
                        info!("Jason: pulling Vanessa out of the car");
                        struggle.wiggle = default();
                        j.velocity = Vec2::ZERO;
                        p.velocity = Vec2::ZERO;
                        commands.entity(pe).remove::<crate::vehicle::Driving>().insert((Held, Visibility::Inherited));
                        if killer.info.animations.contains_key(CAR_GRAB) {
                            // The paired take from DriverGrab: she rides his GrabSocket out of the seat.
                            *xf = killer.place(CAR_GRAB, door).0;
                            snap_rig(&mut rigs, je);
                            j.state = JasonState::PullingOut { t: 0.0 };
                            commands.entity(je).insert(PlayAction(CAR_GRAB.into()));
                            commands.entity(pe).insert(PlayAction(CAR_GRAB.into()));
                        } else {
                            pxf.translation = Vec3::new(door.translation.x, xf.translation.y, door.translation.z);
                            j.state = JasonState::Holding { t: 0.0 };
                            commands.entity(je).insert(PlayAction(GRAB_ENTER.into()));
                            commands.entity(pe).insert(PlayAction(GRABBED_ENTER.into()));
                        }
                        continue;
                    }
                }
                // Under water, deep enough, a swimming counselor in reach (CanInteractUnderwater 0x1403e6f60, its
                // victims FUN_1403e87f0): his drowning kill (OnInteractUnderwater 0x1403f9c10). The AI does it at
                // once, the player with interact.
                if let Some(s) = surface.filter(|_| j.underwater && alive && !driving && hiding.is_none()) {
                    let feet_depth = (s - xf.translation.y) * CM;
                    let her_centre = pxf.translation + Vec3::Y * crate::player::CAPSULE_HALF_HEIGHT_M;
                    // The UnderwaterInteractComponent rides UnderwaterVisualRoot at the surface over him (Jason_Base's
                    // OnUnderWaterUpdate): its DistanceLimit is his distance to it plus 50 cm.
                    let reach = (s + UNDERWATER_INTERACT_HEIGHT_M - centre.y).abs() + UNDERWATER_REACH_PAD_M;
                    // DrowningDepthTrace: ground within 350 cm under her.
                    let ground = f13_core::collision::segment_hit(&world, her_centre, her_centre - Vec3::Y * DROWNING_DEPTH_TRACE_M, 0.5).is_some();
                    let victim = p.counselor.stance == f13_core::counselor::Stance::Swimming
                        && ground
                        && feet_depth >= t.underwater_kill_depth
                        && her_centre.distance(centre) <= reach;
                    let wants = player_input.map_or(true, |i| i.presses.interact);
                    debug!("drown check: feet {feet_depth:.0} cm, swimming {}, ground {ground}, dist {:.2} reach {reach:.2}", p.counselor.stance == f13_core::counselor::Stance::Swimming, her_centre.distance(centre));
                    if feet_depth >= t.underwater_full_speed_depth && victim && wants && killer.info.animations.contains_key(DROWN_KILL.0) {
                        // The kill rides his mesh (attached, snapped), moved to her: he takes its place, his feet
                        // there (the special move's desired transform adds his half height), turned as his mesh is;
                        // she stays.
                        let mesh_rot = xf.rotation * killer.rig_transform().rotation;
                        let spot = Transform { translation: her_centre + Vec3::Y * 0.01, rotation: crate::character::flat(Transform::from_rotation(mesh_rot * crate::character::FACING.inverse())).rotation, ..default() };
                        info!("Jason: drowning Vanessa ({} under)", feet_depth as i32);
                        struggle.wiggle = default();
                        j.velocity = Vec2::ZERO;
                        p.velocity = Vec2::ZERO;
                        j.kill_move = DROWN_KILL;
                        j.kill_score = Some((format!("{DROWN_KILL_KEY}_C"), "ContextKillComponent"));
                        j.kill_holds = false;
                        *xf = killer.place(DROWN_KILL.0, spot).0;
                        snap_rig(&mut rigs, je);
                        j.state = JasonState::Killing { t: 0.0 };
                        // Its camera spline, on the kill (at her, turned as his mesh).
                        let camera = Transform::from_translation(her_centre).with_rotation(mesh_rot) * drown.as_ref().map_or(Transform::IDENTITY, |d| d.0);
                        debug!("drowning kill: her {:.2}, him {:.2} facing {:.2}, camera {:.2} facing {:.2}", pxf.translation, xf.translation, xf.rotation * Vec3::Z, camera.translation, camera.rotation * Vec3::X);
                        commands.entity(je).insert((PlayAction(DROWN_KILL.0.into()), crate::kill_camera::KillCam::at(DROWN_KILL_KEY, camera)));
                        commands.entity(pe).insert((Held, PlayAction(DROWN_KILL.1.into())));
                        continue;
                    }
                }
                // The player's Jason (jason_player.rs): what they pressed, else the AI below.
                if let Some(i) = player_input {
                    let p = i.presses;
                    let moving = (j.velocity.length_squared() > 0.0).then(|| Vec3::new(j.velocity.x, 0.0, j.velocity.y).normalize_or_zero());
                    let centre = xf.translation + Vec3::Y * t.capsule.half_height;
                    j.focus = killer_focus(&doors, &walls, &interact_limits, centre, i.look, xf.rotation * Vec3::Z, moving, j.focus);
                    let blockers = Blockers { underwater: j.underwater, ..Blockers::default() };
                    let ready = |know: &Option<Mut<crate::senses::Knowledge>>, a: Ability| know.as_ref().is_some_and(|k| k.abilities.is_available(a, &blockers));
                    if p.shift && ready(&know, Ability::Shift) {
                        if let Some(k) = know.as_mut() {
                            k.abilities.consume(Ability::Shift);
                        }
                        info!("Jason: Shift");
                        j.state = JasonState::Shifting { shift: Shift::start() };
                        j.velocity = Vec2::ZERO;
                        continue;
                    }
                    // KLR_Morph_PC (FUN_1403e4a70): his map out for a Morph (not over his plain map), or put away.
                    if let Some(map) = morph_map.as_deref_mut() {
                        if p.morph {
                            if map.morph {
                                map.morph = false;
                                map.open = false;
                            } else if !map.open && ready(&know, Ability::Morph) {
                                map.morph = true;
                                map.open = true;
                                map.target = None;
                            }
                        }
                        // AttemptMorph: the place picked on the map; Morph (SetMorphLocation) takes the morph point
                        // nearest it, the map put away, or nothing happens.
                        if let Some(at) = map.target.take().filter(|_| map.morph && ready(&know, Ability::Morph)) {
                            if let Some(to) = morph_points.as_ref().and_then(|m| m.pick(at)) {
                                if let Some(k) = know.as_mut() {
                                    k.abilities.consume(Ability::Morph);
                                }
                                info!("Jason: Morph");
                                map.morph = false;
                                map.open = false;
                                j.state = JasonState::Morphing { t: 0.0, to };
                                commands.trigger(Distortion { at: xf.translation, dest: Some(to.translation) });
                                j.velocity = Vec2::ZERO;
                                continue;
                            }
                            info!("Jason: no morph point near that place");
                        }
                    }
                    if p.stalk && j.stalk.is_none() && ready(&know, Ability::Stalk) {
                        if let Some(k) = know.as_mut() {
                            k.abilities.consume(Ability::Stalk);
                        }
                        info!("Jason: Stalk");
                        j.stalk = Some(Stalk::start(t.stalk_active_time));
                        commands.trigger(Distortion { at: xf.translation, dest: None });
                    }
                    // GLB_Combat_PC: in or out of combat stance.
                    if p.combat {
                        j.combat = !j.combat;
                        info!("Jason: combat stance {}", j.combat);
                    }
                    // KLR_KnifeThrow held (PressKnifeThrow): he takes a knife out and aims until it's let go.
                    if p.knife && j.knives > 0 && j.knife_cooldown <= 0.0 {
                        info!("Jason: aiming a knife ({} left)", j.knives);
                        j.state = JasonState::Throwing { t: 0.0, thrown: false, release: None };
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
                    // E on a closed door (SCDoor::OnInteract 0x140351150, the killer's press): raged, he bursts
                    // through it whatever its lock; locked or barricaded, the LockedDoorSound; else he opens it.
                    if let Some(Focus::Door(de)) = j.focus.filter(|_| p.interact) {
                        if let Ok((_, d, g)) = doors.get(de) {
                            let raged = know.as_ref().is_some_and(|k| k.abilities.rage_active);
                            let barred = barricades.iter().any(|b| b.door == Some(de) && b.enabled);
                            let (state, anim) = if raged {
                                info!("Jason: bursting through a door (raged)");
                                (JasonState::BurstingDoor { door: de, t: 0.0 }, DOOR_BURST)
                            } else if d.locked || barred {
                                commands.trigger(crate::audio::ActorSound { actor: de, prop: "LockedDoorSound" });
                                continue;
                            } else {
                                info!("Jason: opening a door");
                                let anim = if crate::doors::Door::is_inside(g, xf.translation) { DOOR_OPEN_INSIDE } else { DOOR_OPEN_OUTSIDE };
                                (JasonState::OpeningDoor { door: de, t: 0.0, anim }, anim)
                            };
                            j.velocity = Vec2::ZERO;
                            j.state = state;
                            commands.entity(je).insert(PlayAction(anim.into()));
                            continue;
                        }
                    }
                    // A fuse box at hand: his Jason_Generator_Destruction move cuts its power.
                    if p.interact && j.cooldown <= 0.0 {
                        let here = xf.translation;
                        if let Some((fe, fuse)) = fuse_boxes.iter().filter(|(_, f)| !f.destroyed).find(|(_, f)| Vec2::new(f.interact.x - here.x, f.interact.z - here.z).length() * 100.0 <= f.reach + 100.0) {
                            info!("Jason: breaking a fuse box");
                            // OnInteract (0x14038c950): its DestroyScoreEvent as the destroy move begins.
                            commands.trigger(crate::match_flow::ClassScore::killer(FUSE_BOX_CLASS, "DestroyScoreEvent"));
                            let to = fuse.interact - xf.translation;
                            xf.rotation = Quat::from_rotation_y(to.x.atan2(to.z));
                            j.state = JasonState::BreakingFuse { t: 0.0, fuse: fe, done: false };
                            j.velocity = Vec2::ZERO;
                            commands.entity(je).insert(PlayAction(BREAK_FUSE.into()));
                            continue;
                        }
                        let near = |at: Vec3, reach: f32| Vec2::new(at.x - here.x, at.z - here.z).length() * 100.0 <= reach + 100.0;
                        if let Some((be, b)) = phone_boxes.iter().find(|(_, b)| b.breakable() && near(b.interact, b.reach)) {
                            info!("Jason: breaking the phone box");
                            let to = b.interact - xf.translation;
                            xf.rotation = Quat::from_rotation_y(to.x.atan2(to.z));
                            j.state = JasonState::BreakingFuse { t: 0.0, fuse: be, done: false };
                            j.velocity = Vec2::ZERO;
                            commands.entity(je).insert((PlayAction(BREAK_FUSE.into()), ActionSounds(BREAK_PHONE_BOX)));
                            continue;
                        }
                    }
                    // CMBT_KLR_Block (right mouse in combat stance): OnStartBlock while CanBlock (FUN_1402ded00: not
                    // attacking, stunned, interacting or in a special move: here, hunting), OnEndBlock as it's let go.
                    // Leaving combat stance ends it (SetCombatStance 0x1402e94e0 -> vt+0xea8 0x1402e6e20: bIsBlocking
                    // cleared, OnRep_Block).
                    let block = j.combat && i.block_held;
                    if block != j.blocking {
                        j.blocking = block;
                        info!("Jason: {}", if block { "blocking" } else { "block ends" });
                    }
                    // KLR_Grab: not in combat stance (FUN_1403e6e80), where the button blocks.
                    if j.cooldown <= 0.0 && p.grab && !j.combat {
                        info!("Jason: grab");
                        j.state = JasonState::Reaching { t: 0.0 };
                        commands.entity(je).insert(PlayAction(GRAB_STARTUP.into()));
                        continue;
                    }
                    // CMBT_KLR_Attack (0x1403f89a0): his best interactable an SCInteractComponent with
                    // bCanAttackToInteract (the doors' and walls'), he breaks it down when he may (FUN_14040fd00 a door:
                    // closed and intact, the check at 90 degrees; FUN_14040fdd0 a wall: standing, the check at
                    // MaxAttackInteractAngle): bWantsToBreakDown, BreakDownFrom{Inside,Outside}. Held, the loop goes
                    // on; let go (0x1403fec50 clears bWantsToBreakDown), it aborts at the next loop.
                    if p.attack && !j.blocking {
                        let her = alive.then_some(pxf.translation);
                        let breaks = match j.focus {
                            Some(Focus::Door(de)) => doors.get(de).ok().filter(|(_, d, _)| !d.open && !d.broken).is_some_and(|(_, d, g)| {
                                let side = crate::doors::Door::is_inside(g, xf.translation);
                                attack_interact(i.forward, xf.translation, g.transform_point(d.interact), 90.0, t.attack_interact_counselor_m, her.map(|h| (h, crate::doors::Door::is_inside(g, h) == side)))
                            }),
                            Some(Focus::Wall(we)) => walls.get(we).ok().filter(|(_, w, _)| !w.broken).is_some_and(|(_, w, g)| {
                                let side = crate::doors::Wall::is_inside(g, xf.translation);
                                attack_interact(i.forward, xf.translation, g.transform_point(w.interact), t.attack_interact_angle, t.attack_interact_counselor_m, her.map(|h| (h, crate::doors::Wall::is_inside(g, h) == side)))
                            }),
                            None => false,
                        };
                        let state = match j.focus.filter(|_| breaks) {
                            Some(Focus::Door(de)) => Some(JasonState::BreakingDoor { door: de, t: 0.0, section: 0, hit: false }),
                            Some(Focus::Wall(we)) => Some(JasonState::BreakingWall { wall: we, t: 0.0, section: 0, hit: false }),
                            None => None,
                        };
                        if let Some(state) = state {
                            info!("Jason: breaking it down ({state:?})");
                            j.velocity = Vec2::ZERO;
                            j.state = state;
                            commands.entity(je).insert(PlayAction(BREAK_DOOR[0].into()));
                            continue;
                        }
                    }
                    // CMBT_KLR_Attack (0x1403f89a0 -> 0x1402e5aa0): the NeutralAttack; in combat stance the button's
                    // hold starts counting (the heavy attack after 0.1 s, below).
                    // Not while blocking (0x1402e5aa0).
                    if j.cooldown <= 0.0 && p.attack && !j.blocking {
                        info!("Jason: swing");
                        j.state = JasonState::Swinging { t: 0.0, hit: false, heavy: false };
                        j.attack_hold = j.combat.then_some(0.0);
                        commands.entity(je).insert(PlayAction(ATTACK.into()));
                        continue;
                    }
                    // In combat stance he walks at MaxCombatSpeed, facing where the player looks.
                    let speed = if j.combat { t.combat_speed } else if i.run && t.can_run { t.run_speed } else { t.walk_speed };
                    j.running = speed == t.run_speed;
                    let face = j.combat.then_some(i.forward);
                    walk(&mut j, &mut xf, i.movement, speed, face, dt, &t, &world);
                    continue;
                }
                let sees = know.as_ref().is_none_or(|k| k.sees);
                // Near the goal but not getting any closer: as good as there.
                let stalled = j.stall > STALL_SECONDS;
                // Where to go: her (her car's driver door when she drives), if he sees her; else the hiding spot he's
                // after, the last clue, or somewhere.
                let mut goal = car_door.map_or(to, |(door, _)| door.translation);
                // Going for a hiding spot or a clue (not her, not wandering).
                let mut following = false;
                // PLACEHOLDER (the AI's choice; the game's Jason is a player): not seeing her, he goes to break a
                // working phone box (its fuse in) before anything else, as a player would to stop the call.
                let phone_box = if sees { None } else { phone_boxes.iter().find(|(_, b)| b.breakable()).map(|(e, b)| (e, b.interact, b.reach)) };
                if let Some((be, at, reach)) = phone_box {
                    if j.target_spot.is_none() && !j.wander.is_some_and(|w| w.0 == at) {
                        info!("Jason: going for the phone box ({:.0} m)", at.distance(xf.translation));
                        j.wander = Some((at, now));
                    }
                    goal = at;
                    following = true;
                    if stalled || Vec2::new(at.x - xf.translation.x, at.z - xf.translation.z).length() * 100.0 <= reach + 100.0 {
                        info!("Jason: breaking the phone box (AI)");
                        j.stall = 0.0;
                        let to = at - xf.translation;
                        xf.rotation = Quat::from_rotation_y(to.x.atan2(to.z));
                        j.state = JasonState::BreakingFuse { t: 0.0, fuse: be, done: false };
                        j.velocity = Vec2::ZERO;
                        commands.entity(je).insert((PlayAction(BREAK_FUSE.into()), ActionSounds(BREAK_PHONE_BOX)));
                        continue;
                    }
                } else if !sees {
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
                            Ok((_, spot, g)) if !spot.destroyed || spot.occupant.is_some() => {
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
                                            j.kill_score = Some((spot.class.clone(), s_.kill_component));
                                            commands.entity(je).insert(PlayAction(kill.into()));
                                            if let (Some(h), Some(c)) = (hiding.as_mut(), counselor_model.as_ref()) {
                                                h.phase = crate::hiding::HidePhase::Killed;
                                                *pxf = c.place(vkill, *pxf).0;
                                            }
                                            commands.entity(pe).insert((PlayAction(vkill.into()), Visibility::Inherited));
                                        }
                                        (false, _, _, Some(search)) => {
                                            info!("Jason: searching a {:?}", spot.kind);
                                            let (origin, end) = killer.place(search, at);
                                            *xf = origin;
                                            j.state = JasonState::Searching { spot: s, t: 0.0, end, anim: search };
                                            commands.entity(je).insert(PlayAction(search.into()));
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
                // Under water (the AI's choice): straight at his goal along the bottom, for his drowning kill (above);
                // no swings, grabs or abilities there.
                if j.underwater {
                    let wish = if gdist > 0.3 { gflat.normalize_or_zero() } else { Vec2::ZERO };
                    walk(&mut j, &mut xf, wish, t.run_speed, None, dt, &t, &world);
                    continue;
                }
                // His abilities (when is the AI's choice; see the module docs).
                let blockers = Blockers { underwater: j.underwater, ..Blockers::default() };
                let ready = |know: &Option<Mut<crate::senses::Knowledge>>, a: Ability| know.as_ref().is_some_and(|k| k.abilities.is_available(a, &blockers));
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
                if after_clue && gdist > MORPH_MIN_M && j.hunted > MORPH_GRACE_S && ready(&know, Ability::Morph) {
                    let short = goal + (xf.translation - goal).with_y(0.0).normalize_or_zero() * MORPH_SHORT_M;
                    let to = morph_points.as_ref().and_then(|m| m.pick(short)).filter(|to| to.translation.distance(pxf.translation) > MORPH_KEEP_OFF_M);
                    if let Some(to) = to {
                        if let Some(k) = know.as_mut() {
                            k.abilities.consume(Ability::Morph);
                        }
                        info!("Jason: Morph ({gdist:.0} m to his goal; to a point {:.0} m from it)", Vec2::new(to.translation.x - goal.x, to.translation.z - goal.z).length());
                        j.state = JasonState::Morphing { t: 0.0, to };
                        j.velocity = Vec2::ZERO;
                        commands.trigger(Distortion { at: xf.translation, dest: Some(to.translation) });
                        continue;
                    }
                }
                // Stalk closing on a clue unseen, or straight out of a Morph (closing the rest of the way quietly).
                let morphed = std::mem::take(&mut j.morphed);
                if (after_clue && gdist < STALK_RANGE_M || morphed) && j.stalk.is_none() && ready(&know, Ability::Stalk) {
                    if let Some(k) = know.as_mut() {
                        k.abilities.consume(Ability::Stalk);
                    }
                    info!("Jason: Stalk ({gdist:.0} m to his goal)");
                    j.stalk = Some(Stalk::start(t.stalk_active_time));
                    commands.trigger(Distortion { at: xf.translation, dest: None });
                }
                // A knife at her when she's in throwing range.
                if sees && !driving && alive && j.knives > 0 && j.knife_cooldown <= 0.0 && (KNIFE_RANGE_M.0..=KNIFE_RANGE_M.1).contains(&dist) {
                    info!("Jason: throwing a knife ({dist:.0} m, {} left)", j.knives);
                    let aim_at = killer.length(KNIFE_SELECT).unwrap_or(0.37);
                    j.state = JasonState::Throwing { t: 0.0, thrown: false, release: Some(aim_at + KNIFE_AIM_SECONDS) };
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
                // Close in: straight at the goal when near, else along a path. A goal in the water (the AI's choice;
                // the navmesh ends at the shore): straight in once his path runs out there, and along the bottom.
                let wet_goal = water.as_ref().is_some_and(|w| w.surface_at(goal + Vec3::Y * 0.5).is_some());
                // At the shore: the last point of a path that stops short of the water (its end reached, not cut).
                let shore_end = j.path.len() == 1 && j.path.first().is_some_and(|p| Vec2::new(p.x - xf.translation.x, p.z - xf.translation.z).length() < 1.5);
                j.at_shore = wet_goal && !j.path_reaches && (j.at_shore || shore_end);
                let direct = gdist < 1.5 || (wet_goal && (surface.is_some() || j.at_shore));
                let mut wish = if direct { gflat.normalize_or_zero() } else { steer(&mut j, &mut nav.0, navmesh, &world, xf.translation, goal, dt) };
                if sees && dist < t.attack_range_m * 0.7 {
                    wish = Vec2::ZERO; // close enough for anything; don't walk into her
                }
                // A closed door ahead: open it, or break it down when it's locked or barred.
                // A wall between him and her when he walks at her: through it.
                if sees {
                    if let Some(we) = wall_in_way(&walls, xf.translation, wish) {
                        info!("Jason: breaking through a wall");
                        j.velocity = Vec2::ZERO;
                        j.state = JasonState::BreakingWall { wall: we, t: 0.0, section: 0, hit: false };
                        commands.entity(je).insert(PlayAction(BREAK_DOOR[0].into()));
                        continue;
                    }
                }
                let raged = know.as_ref().is_some_and(|k| k.abilities.rage_active);
                if let Some((state, anim)) = door_in_way(&doors, &barricades, xf.translation, wish, wish == Vec2::ZERO, raged) {
                    j.velocity = Vec2::ZERO;
                    j.state = state;
                    commands.entity(je).insert(PlayAction(anim.into()));
                    continue;
                }
                let speed = if t.can_run && gdist > t.attack_range_m * 1.5 { t.run_speed } else { t.walk_speed };
                j.running = speed == t.run_speed;
                // Up close, square up to her (he only turns toward where he walks otherwise).
                let face = (sees && dist < t.grab_max_m * 1.5).then_some(flat);
                walk(&mut j, &mut xf, wish, speed, face, dt, &t, &world);
                trace!("Jason: at {:.2} wish {wish:.2} vel {:.0} grounded {} vy {:.2} path0 {:?}", xf.translation, j.velocity, j.grounded, j.vel_y, j.path.first());
                // Not getting anywhere: try another way for a moment.
                j.progress.1 += dt;
                if j.progress.1 >= 1.0 {
                    let moved = xf.translation.distance(j.progress.0);
                    // Not getting closer near the goal, or at the end of a path that doesn't get there.
                    if !sees && (gdist < STALL_RANGE_M || (!j.path_reaches && !wet_goal)) && gdist > j.last_gdist - 0.3 {
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
            JasonState::Throwing { t: at, thrown, release } => {
                let at = at + dt;
                walk(&mut j, &mut xf, Vec2::ZERO, t.walk_speed, Some(face_dir), dt, &t, &world);
                let len = |n: &str, d: f32| killer.length(n).unwrap_or(d);
                let aim_at = len(KNIFE_SELECT, 0.37);
                // The player's knife: thrown when the button's let go (after the select section), put away by
                // KLR_KnifeCancel (right mouse).
                let mut release = release;
                if let Some(i) = player_input.filter(|_| release.is_none()) {
                    if i.presses.grab {
                        info!("Jason: knife put away");
                        j.state = JasonState::Hunting;
                        commands.entity(je).remove::<PlayAction>();
                        continue;
                    }
                    if !i.knife_held {
                        release = Some(at.max(aim_at));
                    }
                }
                let throw_at = release.unwrap_or(f32::MAX);
                let let_go = throw_at + 0.18f32.min(len(KNIFE_THROW, 0.72));
                let end = throw_at + len(KNIFE_THROW, 0.72);
                let anim = if at < aim_at { KNIFE_SELECT } else if at < throw_at { KNIFE_AIM } else { KNIFE_THROW };
                commands.entity(je).insert(PlayAction(anim.into()));
                let mut thrown = thrown;
                if !thrown && at >= let_go {
                    thrown = true;
                    j.knives -= 1;
                    j.knife_cooldown = t.knife_cooldown;
                    // ThrowKnife (0x14040d100): from his camera, 120 cm out along its forward, at ThrowStrength along it;
                    // the computer Jason's from his eyes, aimed to arc onto her chest.
                    let eyes = xf.translation + Vec3::Y * crate::senses::EYE_M;
                    let (origin, vel) = match player_input {
                        // (No camera yet: his eyes and his look.)
                        Some(i) if i.view.1 == Vec3::ZERO => (eyes, i.look.normalize_or(Vec3::Z) * t.throw_strength),
                        Some(i) => (i.view.0, i.view.1.normalize_or(Vec3::Z) * t.throw_strength),
                        None => (eyes, knife_velocity(eyes, pxf.translation + Vec3::Y * 1.2, t.throw_strength)),
                    };
                    let from = origin + vel.normalize_or_zero() * KNIFE_START_M;
                    commands.trigger(ThrowKnife { from, vel, damage: t.knife_damage });
                }
                if at >= end {
                    j.state = JasonState::Hunting;
                    j.cooldown = t.attack_delay;
                    commands.entity(je).remove::<PlayAction>();
                } else {
                    j.state = JasonState::Throwing { t: at, thrown, release };
                }
            }
            JasonState::BreakingFuse { t: at, fuse, done } => {
                let at = at + dt;
                j.velocity = Vec2::ZERO;
                let window = killer.info.animations.get(BREAK_FUSE).and_then(|a| a.windows.iter().find(|w| w.name == "Driver: DestroyGenerator")).map_or(1.0, |w| w.start);
                let mut done = done;
                if !done && at >= window {
                    done = true;
                    if let (Ok((_, mut f)), Some(g)) = (fuse_boxes.get_mut(fuse), grids.as_mut()) {
                        crate::power::set_destroyed(&mut f, true, g);
                        info!("Jason: destroyed a fuse box");
                    }
                    if let Ok((_, mut b)) = phone_boxes.get_mut(fuse) {
                        crate::police::break_phone_box(&mut b, &mut phones);
                        info!("Jason: broke the phone box");
                    }
                }
                if at >= killer.length(BREAK_FUSE).unwrap_or(2.0) {
                    j.state = JasonState::Hunting;
                    commands.entity(je).remove::<(PlayAction, ActionSounds)>();
                } else {
                    j.state = JasonState::BreakingFuse { t: at, fuse, done };
                }
            }
            JasonState::Slamming { t: at } => {
                let at = at + dt;
                j.velocity = Vec2::ZERO;
                if at >= killer.length(CAR_SLAM).unwrap_or(1.87) {
                    j.state = JasonState::Hunting;
                    commands.entity(je).remove::<PlayAction>();
                } else {
                    j.state = JasonState::Slamming { t: at };
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
                    // OnRep_Teleport as the effect ends: the distortion where he arrived.
                    commands.trigger(Distortion { at: dest.translation, dest: None });
                    j.path.clear();
                    j.progress = (dest.translation, 0.0);
                    j.stall = 0.0;
                    j.last_gdist = f32::MAX;
                    j.morphed = true;
                    j.state = JasonState::Hunting;
                } else {
                    j.state = JasonState::Morphing { t: at, to: dest };
                }
            }
            JasonState::Swinging { t: at, hit, heavy } => {
                let mut at = at + dt;
                let mut heavy = heavy;
                // SCCharacter's tick: the attack held over 0.1 s in combat stance is the heavy attack (vt+0xd38 ->
                // MULTICAST_Attack(true)): the light swing playing turns into the weapon's ForwardAttack at the same
                // fraction of it.
                if let Some(h) = j.attack_hold {
                    let held = player_input.is_some_and(|i| i.attack_held) && j.combat;
                    j.attack_hold = held.then_some(h + dt);
                    if held && h + dt > HEAVY_HOLD_S && !heavy && killer.info.animations.contains_key(HEAVY_ATTACK) {
                        j.attack_hold = None;
                        heavy = true;
                        at = t.heavy_length * (at / t.attack_length.max(1e-3)).min(1.0);
                        info!("Jason: heavy swing");
                        commands.entity(je).insert((PlayAction(HEAVY_ATTACK.into()), crate::character::SeekAction(at)));
                    }
                }
                walk(&mut j, &mut xf, Vec2::ZERO, t.walk_speed, Some(face_dir), dt, &t, &world);
                let mut hit = hit;
                let (window, length) = if heavy { (t.heavy_window, t.heavy_length) } else { (t.attack_window, t.attack_length) };
                if !hit && at >= window.0 && at <= window.1 && alive && dist <= t.attack_range_m && angle_to(xf.rotation, flat) <= t.attack_angle_deg {
                    hit = true;
                    let r = p.counselor.take_damage(Hit { amount: t.damage, blocked_with_weapon: blocking }, &ptuning.counselor);
                    // His weapon's ImpactEffect (Killer_ImpactEffect_Base's CounselorFX for a counselor hit). PLACEHOLDER:
                    // the ImpactPoint and ImpactNormal of his weapon's trace: her chest, facing him (hits are by range).
                    let toward = (xf.translation - pxf.translation).with_y(0.0).normalize_or(Vec3::Z);
                    commands.write_message(crate::particles::SpawnParticles::impact("P_Hit_Blood_Counselor_Default_01", pxf.translation + Vec3::Y * 1.2 + toward * 0.15, toward));
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
                        // Not a context kill: JasonWeaponKillScoreEvent (Butcher), then KillCounselor.
                        commands.trigger(crate::match_flow::KillerScore("Butcher"));
                        commands.trigger(crate::match_flow::KillerScore("KillCounselor"));
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
                    d.drive(killer.info.animations.get(anim).and_then(|a| a.curve("DoorRotation")), at, swing_at + swing_len);
                }
                if at >= killer.length(anim).unwrap_or(1.6) {
                    j.state = JasonState::Hunting;
                    commands.entity(je).remove::<PlayAction>();
                    // The computer Jason's: a trap in the doorway when he isn't after her and has none of his near.
                    let sees = know.as_ref().is_some_and(|k| k.sees);
                    let here = xf.translation;
                    if player_input.is_none() && !sees && j.traps > 0 && j.trap_spots.iter().all(|s| s.distance(here) > TRAP_SPACING_M) && killer.info.animations.contains_key(TRAP_SET) {
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
                    if let Some(v) = &d.visuals {
                        commands.trigger(crate::doors::BreakApart { piece: v.leaf, from: xf.translation, force: d.burst_force });
                    }
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
                if section == ABORT {
                    if abort(&mut j, &killer, at + dt) {
                        commands.entity(je).remove::<PlayAction>();
                    }
                    continue;
                }
                let barred = barricades.iter().any(|b| b.door == Some(de) && b.enabled);
                // The computer Jason's: unlocked (or unbarred) from inside before it gave way, he stops.
                if player_input.is_none() && section < 2 && !d.broken && !d.locked && !barred {
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
                            if let Some(v) = &d.visuals {
                                commands.trigger(crate::doors::BreakApart { piece: v.leaf, from: xf.translation, force: d.break_force });
                            }
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
                    if player_input.is_some_and(|i| !i.attack_held) {
                        info!("Jason: let go; stopped breaking the door");
                        j.state = JasonState::BreakingDoor { door: de, t: 0.0, section: ABORT, hit: false };
                        commands.entity(je).insert(PlayAction(BREAK_ABORT.into()));
                        continue;
                    }
                    // On to (another) Loop.
                    (at, section, hit) = (at - timing.lengths[section], 1, false);
                }
                commands.entity(je).insert(PlayAction(BREAK_DOOR[section].into()));
                j.state = JasonState::BreakingDoor { door: de, t: at, section, hit };
            }
            JasonState::BreakingWall { wall: we, t: at, section, hit } => {
                let Ok((_, mut w, g)) = walls.get_mut(we) else {
                    j.state = JasonState::Hunting;
                    continue;
                };
                let to_wall = Vec2::new(g.translation().x - xf.translation.x, g.translation().z - xf.translation.z);
                walk(&mut j, &mut xf, Vec2::ZERO, t.walk_speed, Some(to_wall), dt, &t, &world);
                if section == ABORT {
                    if abort(&mut j, &killer, at + dt) {
                        commands.entity(je).remove::<PlayAction>();
                    }
                    continue;
                }
                let timing = break_timing(&killer);
                let (mut at, mut section, mut hit) = (at + dt, section, hit);
                if !hit && at >= timing.hits[section] {
                    if section < 2 && w.health <= t.world_damage {
                        // This blow goes through: on to the Success section.
                        (at, section) = (0.0, 2);
                    } else {
                        hit = true;
                        let broke = w.damage(we, t.world_damage, crate::doors::Wall::is_inside(g, xf.translation), &mut commands);
                        // OnRep_Health: every blow SpawnEmitterAttached(HitParticle, InteractComponent); DestroyWall:
                        // DestroyParticle there (0x14033cfc0; scale 1, no offset, auto-destroyed). Its place: the
                        // Interactable's (its rotation the wall's here).
                        if let Some(fx) = if broke { w.destroy_particle.clone() } else { w.hit_particle.clone() } {
                            let at = Transform::from_translation(g.transform_point(w.interact)).with_rotation(g.rotation());
                            commands.write_message(crate::particles::SpawnParticles::at(&fx, at));
                        }
                        if broke {
                            info!("Jason: broke through the wall");
                            if let Some(s) = w.section {
                                commands.trigger(crate::doors::BreakApart { piece: s, from: xf.translation, force: w.burst_force });
                            }
                            commands.trigger(crate::audio::ActorSound { actor: we, prop: "WallDestroySoundCue" });
                        } else {
                            info!("Jason: hit the wall ({:.0} left)", w.health);
                            commands.trigger(crate::audio::ActorSound { actor: we, prop: "WallHitSoundCue" });
                        }
                    }
                }
                if at >= timing.lengths[section] {
                    if section == 2 {
                        j.state = JasonState::Hunting;
                        commands.entity(je).remove::<PlayAction>();
                        continue;
                    }
                    if player_input.is_some_and(|i| !i.attack_held) {
                        info!("Jason: let go; stopped breaking the wall");
                        j.state = JasonState::BreakingWall { wall: we, t: 0.0, section: ABORT, hit: false };
                        commands.entity(je).insert(PlayAction(BREAK_ABORT.into()));
                        continue;
                    }
                    (at, section, hit) = (at - timing.lengths[section], 1, false);
                }
                commands.entity(je).insert(PlayAction(BREAK_DOOR[section].into()));
                j.state = JasonState::BreakingWall { wall: we, t: at, section, hit };
            }
            JasonState::Reaching { t: at } => {
                let at = at + dt;
                // The Default section's root motion (EnterGrabBegin: his move input ignored), turning to where he faces.
                root_move(&mut j, &mut xf, grab_root(&anims, GRAB_STARTUP), Some(face_dir), dt, &t, &world);
                if at < PERFORM_GRAB_TEST_S {
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
                root_move(&mut j, &mut xf, grab_root(&anims, GRAB_MISS), None, dt, &t, &world);
                if at - dt < GRAB_MISS_SHAKE.2 && at >= GRAB_MISS_SHAKE.2 {
                    commands.trigger(JasonCameraShake { shake: GRAB_MISS_SHAKE.0, scale: GRAB_MISS_SHAKE.1 });
                }
                if at >= GRAB_MISS_END_S {
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
                // Past the grab's GrabSuccess section: his locomotion with the grab layer over it (grab_layer); he moves
                // at his normal speeds (GetMaxSpeed has no grab case) from its EnterGrabFinish, the player walking him;
                // the AI stands.
                let success = killer.length(GRAB_ENTER).unwrap_or(2.2);
                if at >= success {
                    commands.entity(je).remove::<PlayAction>();
                }
                let speed = if j.combat { t.combat_speed } else if player_input.is_some_and(|i| i.run) && t.can_run { t.run_speed } else { t.walk_speed };
                j.running = speed == t.run_speed;
                if at - dt < GRAB_SUCCESS_SHAKE.2 && at >= GRAB_SUCCESS_SHAKE.2 {
                    commands.trigger(JasonCameraShake { shake: GRAB_SUCCESS_SHAKE.0, scale: GRAB_SUCCESS_SHAKE.1 });
                }
                // PLACEHOLDER (the AI's: the game's Jason is a player): an unused placed kill within CARRY_TO_KILL_M
                // that isn't at hand yet, he carries her to.
                let centre = xf.translation + Vec3::Y * t.capsule.half_height;
                let facing = Vec2::new((xf.rotation * Vec3::Z).x, (xf.rotation * Vec3::Z).z);
                let carry_to = (player_input.is_none() && config.kill.is_none())
                    .then(|| placed_kills.iter().filter(|k| !k.used && k.interact.distance(centre) < CARRY_TO_KILL_M).min_by(|a, b| a.interact.distance(centre).total_cmp(&b.interact.distance(centre))))
                    .flatten()
                    .filter(|k| !k.in_reach(centre, facing))
                    .map(|k| Vec2::new(k.interact.x - xf.translation.x, k.interact.z - xf.translation.z));
                if at < success {
                    // The GrabSuccess section's root motion carries him (and her with him) until it's played out.
                    root_move(&mut j, &mut xf, grab_root(&anims, GRAB_ENTER), None, dt, &t, &world);
                } else {
                    let wish = player_input.map_or(carry_to.map_or(Vec2::ZERO, |d| d.normalize_or_zero()), |i| i.movement);
                    walk(&mut j, &mut xf, wish, speed, carry_to, dt, &t, &world);
                }
                if at >= counselor_model.as_ref().and_then(|c| c.length(GRABBED_ENTER)).unwrap_or(2.0) {
                    commands.entity(pe).insert(PlayAction(GRABBED_IDLE.into()));
                }
                let input = struggle.input;
                // FUN_1403bc910: her progress per wiggle slowed while she's wounded and against a raged Jason.
                let combat = &ptuning.counselor.combat;
                let raged = know.as_ref().is_some_and(|k| k.abilities.rage_active);
                let mut wiggle = grab_struggle(&p.counselor.stats);
                wiggle.percent_per_wiggle *= combat.wiggle_mod(p.counselor.wounded, raged);
                if struggle.wiggle.tick(dt, input, &wiggle) {
                    // FUN_1403a6c10: BreakFreeStaminaBoost back.
                    let back = combat.break_free_stamina(&p.counselor.stats);
                    p.counselor.stamina = (p.counselor.stamina + back).min(ptuning.counselor.stamina_max);
                    info!("Vanessa: struggled free after {at:.1}s (stamina +{back:.0})");
                    commands.trigger(Score(Stun::GrabBreak.score_event().unwrap_or_default()));
                    let in_water = water.as_ref().is_some_and(|w| w.surface_at(xf.translation + Vec3::Y * t.capsule.half_height).is_some());
                    drop_counselor(&mut pxf, &xf, t.capsule.half_height, in_water, &world);
                    release(&mut commands, pe, GRAB_BREAK, counselor_model.as_deref());
                    j.state = JasonState::Stunned { t: 0.0, stun: Stun::GrabBreak };
                    commands.entity(je).insert(PlayAction(Stun::GrabBreak.anims().0.into()));
                } else if let Some(mut pk) = placed_kills
                    .iter_mut()
                    .find(|k| k.in_reach(xf.translation + Vec3::Y * t.capsule.half_height, player_input.map_or(Vec2::new((xf.rotation * Vec3::Z).x, (xf.rotation * Vec3::Z).z), |i| i.forward)))
                    // CanGrabKill (0x1403e6f10): bGrabKillsActive, his grab's enter played out.
                    .filter(|_| config.kill.is_none() && at >= success && player_input.map_or(at >= HOLD_BEFORE_KILL, |i| i.presses.interact))
                {
                    // A placed kill at hand (the player's E; the AI uses one when it can): he takes its place.
                    let (his, hers, name) = pk.takes();
                    info!("Jason: {name} (placed kill)");
                    pk.used = true;
                    // KillStarted: its prop plays its own animation.
                    if let Some((prop, anim, at_start)) = pk.prop {
                        commands.entity(prop).insert(crate::prop_anim::PropAnim::new(anim));
                        // Its relative transform under the actor as it now is (turned toward him, if it turns).
                        if let Some(xf) = at_start {
                            commands.entity(prop).insert(pk.prop_start(xf));
                        }
                    }
                    j.kill_move = (his, hers, name);
                    j.kill_score = Some((crate::placed_kills::KINDS[pk.kind].0.to_string(), "ContextKillComponent"));
                    *xf = killer.place(his, pk.spot).0;
                    j.state = JasonState::Killing { t: 0.0 };
                    commands.entity(je).insert((PlayAction(his.into()), crate::kill_camera::KillCam::at(&pk.camera_key, pk.camera)));
                    commands.entity(pe).insert(PlayAction(hers.into()));
                    snap_rig(&mut rigs, je);
                } else if player_input.map_or(at >= HOLD_BEFORE_KILL && (carry_to.is_none() || at >= HOLD_BEFORE_KILL * 2.0), |i| {
                    // A kill key (1-4, its slot) or a click (slot 1), its kill available (FUN_1403ffa80).
                    (i.presses.attack || i.presses.kill.is_some()) && grab_kills.0[i.presses.kill.unwrap_or(1).min(3)]
                }) {
                    // The AI kills after a while; the player with a kill key or a click. One of the weapon's grab kills.
                    let picked = player_input.and_then(|i| GRAB_KILL_SLOTS[i.presses.kill.unwrap_or(1).min(3)]);
                    j.kill = picked.or(config.kill).unwrap_or_else(|| rng.0.index(KILLS.len()));
                    let (anim, class, name) = KILLS[j.kill];
                    info!("Jason: {name}");
                    j.kill_move = (anim, anim, name);
                    j.kill_score = Some((format!("{class}_C"), "ContextKillComponent"));
                    j.state = JasonState::Killing { t: 0.0 };
                    commands.entity(je).insert((PlayAction(anim.into()), crate::kill_camera::KillCam::new(class, *xf)));
                    commands.entity(pe).insert(PlayAction(anim.into()));
                } else {
                    j.state = JasonState::Holding { t: at };
                }
            }
            JasonState::PullingOut { t: at } => {
                let at = at + dt;
                j.velocity = Vec2::ZERO;
                let jrig = rigs.iter().find(|(_, c, _)| c.parent() == je);
                let crig = rigs.iter().find(|(_, c, _)| c.parent() == pe);
                if let (Some((jr, _, _)), Some((cr, _, cxf)), Some(c)) = (jrig, crig, &counselor_model) {
                    place_held(pe, &mut pxf, true, jr, (cr, cxf), &killer.info, &c.info, &globals);
                }
                if at >= killer.length(CAR_GRAB).unwrap_or(6.0) {
                    // Out: he stays where the take left him, holding her (past the grab's enter).
                    if let Some((_, last)) = killer.root_motion(CAR_GRAB) {
                        *xf = crate::character::flat(Transform::from_matrix(xf.to_matrix() * last.to_matrix()));
                        snap_rig(&mut rigs, je);
                    }
                    j.state = JasonState::Holding { t: HOLD_AFTER_CAR_GRAB };
                    commands.entity(je).remove::<PlayAction>();
                    commands.entity(pe).insert(PlayAction(GRABBED_IDLE.into()));
                } else {
                    j.state = JasonState::PullingOut { t: at };
                }
            }
            JasonState::Killing { t: at } => {
                let at = at + dt;
                let (kill, her_kill, kill_name) = j.kill_move;
                let jrig = rigs.iter().find(|(_, c, _)| c.parent() == je);
                let crig = rigs.iter().find(|(_, c, _)| c.parent() == pe);
                if alive {
                    if let (true, Some((jr, _, _)), Some((cr, _, cxf)), Some(c)) = (j.kill_holds, jrig, crig, &counselor_model) {
                        place_held(pe, &mut pxf, true, jr, (cr, cxf), &killer.info, &c.info, &globals);
                    }
                    // She dies, and goes limp, where her side of the kill says (he plays his out).
                    if at >= ragdoll_time(counselor_model.as_deref(), her_kill) {
                        let health = p.counselor.health;
                        p.counselor.take_damage(Hit { amount: health, blocked_with_weapon: false }, &ptuning.counselor);
                        commands.entity(pe).remove::<(Held, PlayAction)>();
                        info!("Vanessa: killed by Jason ({kill_name})");
                        if let Some((class, component)) = j.kill_score.take() {
                            commands.trigger(crate::match_flow::ContextKillScore { class, component });
                        }
                    }
                }
                if at >= killer.length(kill).unwrap_or(12.0) {
                    // He stays where the kill took him (its root motion), not back at its start.
                    if let Some((_, last)) = killer.root_motion(kill) {
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
                    if let Some((class, component)) = j.kill_score.take() {
                        commands.trigger(crate::match_flow::ContextKillScore { class, component });
                    }
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
            JasonState::Flinching { t: at, anim } => {
                let at = at + dt;
                walk(&mut j, &mut xf, Vec2::ZERO, t.walk_speed, None, dt, &t, &world);
                let end = killer.info.animations.get(anim).and_then(|a| a.windows.iter().find(|w| w.name == "OnEndBlockReaction")).map(|w| w.start);
                if at >= end.or(killer.length(anim)).unwrap_or(0.5) {
                    j.state = JasonState::Hunting;
                    commands.entity(je).remove::<PlayAction>();
                } else {
                    j.state = JasonState::Flinching { t: at, anim };
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

/// The wiggle input (FUN_1402dcfa0): -1 while one of GLB_Wiggle's keys is down (DefaultInput.ini: E), else 0; the
/// minigame's tick counts its turning non-zero, so each press of E is a wiggle (at most MaxWigglesPerSecond). Playing
/// Jason, the keys are his (E is his interact): she doesn't wiggle and frees herself by the stun's time alone
/// (PLACEHOLDER: the game's offline counselor AI's struggle isn't read).
pub(crate) fn read_struggle(keys: Res<ButtonInput<KeyCode>>, mut s: ResMut<Struggle>, play_jason: Option<Res<crate::jason_player::PlayJason>>) {
    s.input = if play_jason.is_none() && keys.pressed(KeyCode::KeyE) { -1.0 } else { 0.0 };
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

/// Where she's set down as he lets go out of the water (SCCounselorCharacter::OnDropped 0x1403c1610): her capsule
/// (DropSweep, Pawn channel, ignoring the two of them) at his centre plus his forward turned by 0, +5, -5, +10 ... up
/// to 135 degrees about up, times her distance from him (flat); with ground within twice her half height below it.
/// Past 135 degrees the distance grows by that first distance, up to 2000 cm; then she stays where she is ("NO VALID
/// DROP SPOT FOUND"). In the water she stays where he held her (UpdateGrabTransform).
fn drop_counselor(pxf: &mut Transform, jxf: &Transform, his_half_height: f32, in_water: bool, world: &(impl f13_core::collision::TriSource + ?Sized)) {
    use crate::player::{CAPSULE_HALF_HEIGHT_M as HH, CAPSULE_RADIUS_M as R};
    if in_water {
        return;
    }
    let centre = jxf.translation + Vec3::Y * his_half_height;
    let fwd = (jxf.rotation * Vec3::Z).with_y(0.0).normalize_or_zero();
    let step = Vec2::new(pxf.translation.x - jxf.translation.x, pxf.translation.z - jxf.translation.z).length().max(0.01);
    let (mut dist, mut angle) = (step, 0.0f32);
    loop {
        // FVector::RotateAngleAxis about Unreal's up: a positive Unreal yaw is negative about Bevy's Y.
        let at = centre + Quat::from_rotation_y(-angle.to_radians()) * fwd * dist;
        let blocked = f13_core::collision::capsule_blocked(world, at - Vec3::Y * (HH - R), at + Vec3::Y * (HH - R), R);
        if !blocked && f13_core::collision::segment_hit(world, at, at - Vec3::Y * HH * 2.0, 0.5).is_some() {
            let to = at - Vec3::Y * HH;
            info!("Vanessa: set down {:.2} m from where he held her ({:.0} degrees off his forward)", to.distance(pxf.translation), angle);
            pxf.translation = to;
            return;
        }
        if angle != 0.0 {
            angle = -angle;
        }
        if angle >= 0.0 {
            angle += 5.0;
        }
        if angle > 135.0 {
            dist += step;
            angle = 0.0;
            if dist > 20.0 {
                warn!("no valid drop spot for her: she stays where he held her");
                return;
            }
        }
    }
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

/// His player's sound mixes (pushed only when he's locally controlled): OnRep_ShiftActive's ShiftAudioSoundMix,
/// OnRep_Stalk's StalkAudioSoundMix, OnRep_UnderWater's UnderwaterSoundMix with UnderWaterCue (CreateSound2D, faded
/// in over 1 s; out over 1 s as he comes up).
fn jason_mixes(mut commands: Commands, play: Option<Res<crate::jason_player::PlayJason>>, jasons: Query<&Jason>, mut last: Local<(bool, bool, bool)>) {
    let now = match (play, jasons.iter().next()) {
        (Some(_), Some(j)) => (j.shifting(), j.stalking(), j.underwater),
        _ => (false, false, false),
    };
    let toggle = |commands: &mut Commands, on: bool, mix: &'static str| {
        if on {
            commands.trigger(crate::audio::PushMix(mix));
        } else {
            commands.trigger(crate::audio::PopMix(mix));
        }
    };
    if now.0 != last.0 {
        toggle(&mut commands, now.0, "JasonShiftSoundMix");
    }
    if now.1 != last.1 {
        toggle(&mut commands, now.1, "JasonStalkSoundMix");
    }
    if now.2 != last.2 {
        toggle(&mut commands, now.2, "JasonUnderwaterSoundMix");
        if now.2 {
            commands.trigger(crate::audio::Loop2D { key: "JasonUnderwater", cue: "JasonUnderwater_Cue", fade_in: 1.0 });
        } else {
            commands.trigger(crate::audio::Stop2D { key: "JasonUnderwater", fade_out: 1.0 });
        }
    }
    *last = now;
}

/// What's left of the counselor's teleport effect (SCKillerCharacter +0x1444, TeleportScreenTime from each
/// PlayDistortionEffect): his tick, as it runs out with the effect on, clears the sound mixes
/// (ClearSoundMixModifiers: TeleportCounselorHearsMix with the rest).
#[derive(Resource)]
pub struct TeleportEffect {
    /// Its time left and its TeleportScreenTime (s).
    pub left: f32,
    pub total: f32,
}

fn teleport_mix(mut commands: Commands, time: Res<Time>, effect: Option<ResMut<TeleportEffect>>) {
    let Some(mut e) = effect else { return };
    e.left -= time.delta_secs();
    if e.left <= 0.0 {
        commands.remove_resource::<TeleportEffect>();
        commands.trigger(crate::audio::ClearMixes);
    }
}

/// GrabWeight (the anim instance's copy of GrabValue): the grab layer while he holds her.
/// The played Jason's door moves take the view (their handlers' SCSplineCameras: interaction_camera.rs): opening one
/// (KillerOpenDoorInside / Outside: 1 s in, 0.5 s back) or breaking one down (BreakDownFromInside / Outside, the
/// handler's defaults: at once, 0.25 s back), handed back as the move ends. A raged burst's and the walls' handlers
/// have no camera.
fn jason_door_camera(
    mut commands: Commands,
    play: Option<Res<crate::jason_player::PlayJason>>,
    jasons: Query<(&Jason, &Transform)>,
    doors: Query<&GlobalTransform, With<crate::doors::Door>>,
    mut playing: Local<Option<(Entity, &'static str)>>,
) {
    if play.is_none() {
        return;
    }
    let Some((j, xf)) = jasons.iter().next() else { return };
    let inside = |d: Entity| doors.get(d).is_ok_and(|g| crate::doors::Door::is_inside(g, xf.translation));
    let now = match j.state {
        JasonState::OpeningDoor { door, anim, .. } => {
            Some((door, if anim == DOOR_OPEN_INSIDE { "KillerOpenDoorInside_SpecialMoveHandler" } else { "KillerOpenDoorOutside_SpecialMoveHandler" }))
        }
        JasonState::BreakingDoor { door, .. } => Some((door, if inside(door) { "BreakDownFromInside_SpecialMoveHandler" } else { "BreakDownFromOutside_SpecialMoveHandler" })),
        _ => None,
    };
    // A move's handler is chosen as it starts (a door found locked mid-open goes on to the break down's).
    let now = match (*playing, now) {
        (Some((d, h)), Some((nd, nh))) if d == nd && (h == nh || !nh.starts_with("Break")) => Some((d, h)),
        _ => now,
    };
    if now != *playing {
        match now {
            Some((actor, handler)) => commands.trigger(crate::interaction_camera::PlayInteractionCamera { actor, handler, killer: true }),
            None => commands.trigger(crate::interaction_camera::StopInteractionCamera { killer: true }),
        }
        *playing = now;
    }
}

fn grab_layer(time: Res<Time>, mut jasons: Query<(&Jason, &mut crate::character::LayeredBlend)>) {
    for (j, mut l) in &mut jasons {
        // Holding her: the grab layer at once. Blocking on the move (bIsMoving): his block over his walk, its
        // BlendListByBool's 0.1 s blend in and out.
        let w = if j.holding() {
            l.switch(GRAB_LAYER_FILTERS, GRAB_IDLE);
            1.0
        } else {
            if j.blocking {
                l.switch(BLOCK_LAYER_FILTERS, BLOCK);
            }
            let target = if j.blocking && j.velocity.length_squared() > 1e-8 { BLOCK_LAYER_WEIGHT } else { 0.0 };
            let step = BLOCK_LAYER_WEIGHT * time.delta_secs() / BLOCK_LAYER_BLEND_S;
            l.weight + (target - l.weight).clamp(-step, step)
        };
        if l.weight != w {
            l.weight = w;
        }
    }
}

fn animate_jason(
    mut commands: Commands,
    time: Res<Time>,
    killer: Option<Res<KillerCharacter>>,
    tuning: Option<Res<KillerTuning>>,
    jasons: Query<(&Jason, Option<&PlayAction>, Option<&ActionSounds>, Option<&crate::character::SeekAction>)>,
    players: Query<&Transform, With<Player>>,
    owners: Query<&Transform, With<Jason>>,
    mut rigs: Query<(&ChildOf, &mut Rig, &mut AnimationPlayer)>,
    anim_sounds: Option<Res<crate::audio::AnimSounds>>,
) {
    let (Some(killer), Some(t)) = (killer, tuning) else { return };
    let dt = time.delta_secs();
    for (parent, mut rig, mut anim) in &mut rigs {
        if rig.model != Model::Killer {
            continue;
        }
        let Ok((j, action, montage, seek)) = jasons.get(parent.parent()) else { continue };
        rig.set_action(action.map(|a| a.0.as_str()), dt);
        if let Some(s) = seek {
            rig.seek_action(s.0);
            commands.entity(parent.parent()).remove::<crate::character::SeekAction>();
        }
        if let Ok(at) = owners.get(parent.parent()) {
            let span = rig.action_span(dt);
            let keyed = montage.zip(span).map(|(m, (take, from, to))| (format!("{}/{take}", m.0), from, to));
            let span = keyed.as_ref().map(|(k, from, to)| (k.as_str(), *from, *to)).or(span);
            crate::audio::play_anim_sounds(&mut commands, anim_sounds.as_deref(), span, at.translation);
        }
        // BS_Jason_Normal_Move's SpeedState: 0 idle, 1 walk, 2 fast walk, 3 run.
        let speed = j.velocity.length();
        let marks = [0.0, t.walk_speed, 0.5 * (t.walk_speed + t.run_speed), t.run_speed];
        let state = speed_state(speed, &marks);
        // Squared up to a counselor nearby: the pickaxe's combat idle.
        let near = owners.get(parent.parent()).ok().zip(players.iter().next()).is_some_and(|(a, b)| a.translation.distance(b.translation) < 4.0);
        // His block (the combat slots): standing, full body (CombatFullBody); on the move, the upper body over his
        // walk (CombatUpperBody: the block layer, below).
        let idle = if j.blocking && killer.info.animations.contains_key(BLOCK) {
            BLOCK
        } else if (near || j.combat) && j.state != JasonState::Done {
            COMBAT_IDLE
        } else {
            IDLE
        };
        let mut targets: HashMap<&str, f32> = HashMap::new();
        for (k, name) in [idle, MOVE[0], MOVE[1], MOVE[2]].iter().enumerate() {
            let w = (1.0 - (state - k as f32).abs()).max(0.0);
            if w > 0.0 {
                *targets.entry(name).or_default() += w;
            }
        }
        let synced = |n: &str| MOVE.contains(&n);
        rig.drive(&killer.0, &targets, &synced, dt, &mut anim);
        // His footsteps: the Footstep_L / Footstep_R notifies (AnimNotify_Footstep_L/R, to Jason_AnimBP's
        // GetFootstepSound) of the locomotion cycle that weighs most, as its phase passes them (as hers, character.rs);
        // none while Shift or Stalk is on (his anim instance's handler FUN_14030df00: bIsShiftActive, bIsStalkActive).
        let (before, after) = (rig.prev_phase, rig.phase);
        rig.prev_phase = after;
        if rig.action.is_none() && speed > 1.0 && !j.shifting() && !j.stalking() {
            let main = rig.weights.iter().filter(|(n, _)| synced(n)).max_by(|a, b| a.1.total_cmp(b.1)).map(|(n, _)| n.clone());
            if let Some(info) = main.and_then(|n| killer.info.animations.get(&n)) {
                for w in info.windows.iter().filter(|w| w.name == "Footstep_L" || w.name == "Footstep_R") {
                    let at = w.start / info.length.max(1e-3);
                    let passed = if after >= before { before < at && at <= after } else { at > before || at <= after };
                    if passed {
                        commands.trigger(crate::audio::Footstep { owner: parent.parent(), left: w.name == "Footstep_L", land: false });
                    }
                }
            }
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
    fn attacking_a_door_or_wall_breaks_it_when_he_faces_it_and_she_isnt_by_him() {
        let at = Vec3::new(0.0, 1.0, 1.0);
        // Doors: anything up to 90 degrees off (acos(clamp(pi/2)) = 0).
        assert!(attack_interact(Vec2::new(1.0, 0.01), Vec3::ZERO, at, 90.0, 2.0, None));
        assert!(!attack_interact(Vec2::new(1.0, -0.01), Vec3::ZERO, at, 90.0, 2.0, None));
        // Walls (45): a dot of at least acos(0.785) = 0.667, about 48 degrees.
        let off = |deg: f32| Vec2::new(deg.to_radians().sin(), deg.to_radians().cos());
        assert!(attack_interact(off(47.0), Vec3::ZERO, at, 45.0, 2.0, None));
        assert!(!attack_interact(off(49.0), Vec3::ZERO, at, 45.0, 2.0, None));
        // Her within 2 m on his side: he swings instead; on the other side, or further, he breaks it.
        let her = Vec3::new(0.5, 0.0, 0.5);
        assert!(!attack_interact(Vec2::Y, Vec3::ZERO, at, 90.0, 2.0, Some((her, true))));
        assert!(attack_interact(Vec2::Y, Vec3::ZERO, at, 90.0, 2.0, Some((her, false))));
        assert!(attack_interact(Vec2::Y, Vec3::ZERO, at, 90.0, 2.0, Some((her * 5.0, true))));
    }

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
