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
//! teleport effect's fade hides him below half (the game dissolves his materials). Stalk silences his footsteps and
//! the counselors' near-Jason music, pushes JasonStalkSoundMix for his player ([`jason_mixes`]) and is a toggle; a
//! stun ends it ([`cancel_on_stun`]).
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

mod kit;
mod knives;
mod update;
pub use kit::*;
pub use knives::*;
pub(crate) use update::*;

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
/// A stuck knife's life from its impact (OnStop's SetLifeSpan: SCThrowable's TimeToDestroy, its constructor's 5).
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
    /// The grab kill he always does (its take's name's end: [`kill_by_name`]; `--kill <name>`), else one at random.
    pub kill: Option<String>,
    /// Testing (--jason-health): his health at the start.
    pub health: Option<f32>,
}

#[derive(Resource)]
pub(crate) struct JasonConfig {
    delay: f32,
    kill: Option<String>,
    health: Option<f32>,
}

impl Plugin for JasonPlugin {
    fn build(&self, app: &mut App) {
        crate::net_world::sounds(app);
        app.insert_resource(JasonConfig { delay: self.delay, kill: self.kill.clone(), health: self.health })
            .init_resource::<GrabKillsAvailable>()
            .add_systems(Update, (show_mask, apply_weapon))
            .init_resource::<JasonTarget>()
            // (As his model loads: at startup, or in a lobby's game as its match picks him.)
            .add_systems(Update, (spawn_jason, load_knife_look).run_if(resource_added::<KillerCharacter>))
            .add_systems(Update, apply_pending_loadout)
            .add_systems(FixedUpdate, choose_target.before(crate::senses::perceive).before(grab_kills_available).before(update_jason).run_if(crate::net::simulating))
            .add_systems(FixedUpdate, (grab_kills_available.before(update_jason), update_jason, recover_counselor, fly_knives, pick_up_knives).run_if(crate::net::simulating).run_if(crate::match_flow::in_progress))
            .add_systems(FixedUpdate, cancel_on_stun.after(update_jason).run_if(crate::net::simulating).run_if(crate::match_flow::in_progress))
            .add_observer(spawn_knife)
            .add_observer(pickup_taken)
            .add_observer(distortion)
            .add_observer(car_slam)
            .add_systems(Update, (tag_jason_parts, animate_jason, grab_layer, jason_mixes, teleport_mix, jason_door_camera, held_knife.after(animate_jason)))
            .add_systems(Update, finish_kills.run_if(crate::net::simulating));
    }
}

/// The counselor Jason's after this tick (his senses and his update act on her). PLACEHOLDER: the game's Jason AI was
/// cut (Jason is a player), so this is f13-rs's choice: the counselor in his hands, else the nearest one alive, not
/// escaped and not hidden (a hidden one only when she's the only one left).
#[derive(Resource, Default)]
pub struct JasonTarget(pub Option<Entity>);

fn choose_target(
    mut target: ResMut<JasonTarget>,
    jasons: Query<(&Transform, &Jason)>,
    counselors: Query<(Entity, &Player, &Transform, Has<Held>, Has<crate::hiding::Hiding>), (Without<Jason>, Without<crate::vehicle::Escaped>)>,
) {
    let Some((jxf, j)) = jasons.iter().next() else {
        target.0 = None;
        return;
    };
    // What he's doing to her he finishes with her (a kill goes on after she's dead and let go).
    let busy = matches!(
        j.state,
        JasonState::Swinging { .. } | JasonState::Reaching { .. } | JasonState::Holding { .. } | JasonState::Killing { .. } | JasonState::HideKill { .. } | JasonState::Slamming { .. } | JasonState::PullingOut { .. } | JasonState::Throwing { .. } | JasonState::Done | JasonState::KillStance { .. } | JasonState::Dying { .. } | JasonState::Dead
    );
    if busy && target.0.is_some_and(|t| counselors.contains(t)) {
        return;
    }
    if let Some((e, ..)) = counselors.iter().find(|c| c.3) {
        target.0 = Some(e);
        return;
    }
    let pick = |hidden_ok: bool| {
        counselors
            .iter()
            .filter(|(_, p, _, _, hidden)| p.counselor.alive() && (hidden_ok || !hidden))
            .min_by(|a, b| a.2.translation.distance_squared(jxf.translation).total_cmp(&b.2.translation.distance_squared(jxf.translation)))
            .map(|c| c.0)
    };
    // (The last counselor standing is still his, alive or not, so the match's end plays out as it did.)
    target.0 = pick(false).or_else(|| pick(true)).or_else(|| counselors.iter().next().map(|c| c.0));
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
    /// His health (TakeDamage 0x14040b600): MaxHealth, MinimumHealth (damage never takes him lower), the mask's
    /// MaskPopOffThreshold, and a blocked hit's BlockedHitDamageModifier.
    pub max_health: f32,
    pub min_health: f32,
    pub mask_threshold: f32,
    pub blocked_damage: f32,
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
        let attack = info.animations.get(attack_take());
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
            heavy_length: info.animations.get(heavy_take()).map_or(1.2, |a| a.length),
            heavy_window: info.animations.get(heavy_take()).and_then(|a| a.window("AttackTraceNotify")).unwrap_or((0.3, 0.6)),
            num_knives: v("NumKnives", 0.0) as u32,
            knife_cooldown: v("ThrowingKnifeCooldown", 3.5),
            throw_strength: v("ThrowStrength", 1000.0) / CM,
            knife_damage: v("ThrowingKnifeDamage", 30.0),
            trap_count: v("InitialTrapCount", 5.0) as u32,
            trap_placement_m: v("TrapPlacementDistance", 75.0) / CM,
            max_health: v("MaxHealth", 100.0),
            min_health: v("MinimumHealth", 1.0),
            mask_threshold: v("MaskPopOffThreshold", 30.0),
            blocked_damage: v("BlockedHitDamageModifier", 1.0),
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
    /// Kneeling in his kill stance (KillStanceMontage AM_normal_death_sequence: entrance, idle, exit), his
    /// JasonKillObject at him (jason_death.rs): s into it; it lasts PamelasSweaterStun's 10 s (EnterKillStance
    /// reapplies it).
    KillStance { t: f32 },
    /// Being killed (JasonDeath's Jason_Death_* take): s into it, the take, and who kills him.
    Dying { t: f32, take: &'static str, by: Entity },
    /// Dead.
    Dead,
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
    pub(crate) cooldown: f32,
    wants_grab: bool,
    /// Where he was at the last progress check, and the time since.
    progress: (Vec3, f32),
    /// Walking a fixed direction for a moment to get unstuck.
    unstick: Option<(Vec2, f32)>,
    /// Seconds left of immunity to stuns.
    stun_immune: f32,
    /// His health and whether his mask is on (bMaskOn).
    pub health: f32,
    pub mask_on: bool,
    /// Pamela's sweater stunned him (the bool at 0x1291, set with PamelasSweaterStun; cleared as he recovers).
    pub sweater_stunned: bool,
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
    /// On a client: what the server's Jason is doing, for his looks (net.rs mirrors it; his own state isn't run).
    pub remote: Option<RemoteView>,
    /// The server's state's name (a client's HUD).
    pub remote_state: String,
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
    /// The stun's StunTimeModifier (its time and minigame's x it).
    stun_scale: f32,
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
    /// The played Jason's fast walk as SetRunning leaves it (0x1402e99f0): on as Left Shift is pressed (OnStartFastWalk,
    /// which first leaves combat stance; SetRunning refuses in it), off as it's let go (OnStopFastWalk) and as a grab
    /// takes her (FinishGrabCounselor 0x1403eb560: SetCombatStance(false), SetRunning(false)), so Shift held through
    /// the grab doesn't run him, and a new press doesn't either: his SetRunning override (0x140407400) refuses while
    /// GrabbedCounselor is set (his GrabInput binds KLR_FastWalk, to no effect).
    pub fast_walk: bool,
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
        self.remote.map_or(self.stalk.is_some(), |r| r.stalking)
    }

    /// Aiming a knife he hasn't thrown yet.
    pub fn aiming_knife(&self) -> bool {
        self.remote.map_or(matches!(self.state, JasonState::Throwing { thrown: false, .. }), |r| r.knife_aim)
    }

    /// The closed door the played Jason has as his best interactable.
    pub(crate) fn focus_door(&self) -> Option<Entity> {
        // (A client's twin: the server's.)
        if let Some(r) = self.remote {
            return r.focus_door;
        }
        match self.focus {
            Some(Focus::Door(d)) => Some(d),
            _ => None,
        }
    }

    pub fn shifting(&self) -> bool {
        self.remote.map_or(matches!(self.state, JasonState::Shifting { .. }), |r| r.shifting)
    }

    /// Testing (a bot hook): stunned as a firecracker's hit stuns him; the take his stun plays.
    pub(crate) fn stun_for_test(&mut self) -> &'static str {
        let stun = Stun::Firecracker;
        self.state = JasonState::Stunned { t: 0.0, stun };
        self.stun_scale = 1.0;
        self.velocity = Vec2::ZERO;
        stun.anims().0
    }

    pub fn stunned(&self) -> bool {
        // (A client's twin: the server's state.)
        if self.remote.is_some() {
            return self.remote_state == "Stunned";
        }
        matches!(self.state, JasonState::Stunned { .. })
    }

    /// In a context kill (bInContextKill): a grab, placed or hiding spot kill.
    /// In his kill stance (a counselor can kill him: jason_death.rs).
    pub fn kneeling(&self) -> bool {
        // (A client has the server's state by name.)
        if self.remote.is_some() {
            return self.remote_state == "KillStance";
        }
        matches!(self.state, JasonState::KillStance { .. })
    }

    /// Dead (killed: jason_death.rs).
    /// A client's view of the server's state, by name (None on the server).
    pub fn state_name_remote(&self) -> Option<&str> {
        self.remote.as_ref().map(|_| self.remote_state.as_str())
    }

    pub fn dead(&self) -> bool {
        if self.remote.is_some() {
            return self.remote_state == "Dying" || self.remote_state == "Dead";
        }
        matches!(self.state, JasonState::Dying { .. } | JasonState::Dead)
    }

    /// Morphing (bIsMorphActive).
    pub fn morphing(&self) -> bool {
        if self.remote.is_some() {
            return self.remote_state == "Morphing";
        }
        matches!(self.state, JasonState::Morphing { .. })
    }

    pub fn in_context_kill(&self) -> bool {
        if self.remote.is_some() {
            return self.remote_state == "Killing" || self.remote_state == "HideKill";
        }
        matches!(self.state, JasonState::Killing { .. } | JasonState::HideKill { .. })
    }

    /// Holding her in his grab (she can struggle), not killing her.
    pub fn holding(&self) -> bool {
        self.remote.map_or(matches!(self.state, JasonState::Holding { .. }), |r| r.holding)
    }

    /// IsInSpecialMove for his anim instance (Jason_AnimBP's look-at switch): the moves the game plays through an
    /// SCSpecialMoveComponent's handler. PLACEHOLDER: the set is read off the handlers f13-rs plays, not the binary.
    pub(crate) fn in_special_move(&self) -> bool {
        matches!(
            self.state,
            JasonState::OpeningDoor { .. }
                | JasonState::BreakingDoor { .. }
                | JasonState::BreakingWall { .. }
                | JasonState::BurstingDoor { .. }
                | JasonState::Killing { .. }
                | JasonState::Searching { .. }
                | JasonState::HideKill { .. }
                | JasonState::PullingOut { .. }
                | JasonState::BreakingFuse { .. }
        )
    }

    /// bIsGrabEnterPlaying (SCKillerAnimInstance's update 0x140314310: Montage_IsPlaying(GrabMontage)): one of
    /// AM_Jason_Enter_Grab_Montage's sections is his action.
    pub(crate) fn grab_enter_playing(action: &str) -> bool {
        [GRAB_STARTUP, GRAB_ENTER, GRAB_MISS].contains(&action)
    }

    /// His stun's minigame while it runs: its completion and the stun class's StunText.
    pub fn stun_minigame(&self) -> Option<(f32, &'static str)> {
        if let Some(r) = &self.remote {
            return r.minigame;
        }
        match self.state {
            JasonState::Stunned { stun, .. } if self.stun_done.is_none() && stun.minigame().is_some() => Some((self.stun_wiggle.completion, stun.text()?)),
            _ => None,
        }
    }

    /// How long he's held her (from the grab's enter), while he does.
    pub fn holding_for(&self) -> Option<f32> {
        if let Some(r) = self.remote {
            return r.holding_for;
        }
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
    /// MaskKnockedOffStun_C: his mask knocked off (3 s, its directional montages, no minigame).
    MaskOff { left: bool },
    /// PamelasSweaterStun_C: Pamela's sweater (10 s, its "Resist" minigame, DepletionRate 0.2; JasonStunAnimMontage2H
    /// with his two-handed pickaxe; sweater.rs).
    Sweater,
}

impl Stun {
    pub fn seconds(self) -> f32 {
        match self {
            Stun::Weapon { .. } | Stun::Flare => 5.0,
            Stun::Shotgun => 7.0,
            Stun::Firecracker => 4.0,
            Stun::PocketKnife | Stun::GrabBreak | Stun::Police => 6.0,
            Stun::BearTrap => 10.0,
            Stun::MaskOff { .. } => 3.0,
            Stun::Sweater => 10.0,
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
            Stun::Police | Stun::MaskOff { .. } | Stun::Sweater => return None,
        })
    }

    /// The stun class's StunText (the minigame's prompt): BaseWeaponStun and HeavyShotgunStun "Rise",
    /// FlareGunLightStun "Brush Off", BearTrapLeftFootStun "Open Trap", PoliceShootJasonStun "Resist".
    /// Every StunText (a client maps the server's back onto these).
    pub const TEXTS: [&'static str; 4] = ["Rise", "Brush Off", "Open Trap", "Resist"];

    pub fn text(self) -> Option<&'static str> {
        Some(match self {
            Stun::Weapon { .. } | Stun::Shotgun => "Rise",
            Stun::Flare => "Brush Off",
            Stun::BearTrap => "Open Trap",
            Stun::Police | Stun::Sweater => "Resist",
            Stun::Firecracker | Stun::PocketKnife | Stun::GrabBreak | Stun::MaskOff { .. } => return None,
        })
    }

    /// The stun's minigame (bPlayMinigame), which he can wiggle out of early: TotalStunTime is [`Stun::seconds`],
    /// PercentPerWiggle SCStunDamageType's constructor 5, DepletionRate the class's (BaseWeaponStun 0.3, the flare and
    /// shotgun stuns 0.2, the bear trap's the constructor 0.02). The firecracker and grab-break stuns have none.
    pub fn minigame(self) -> Option<f13_core::stun::WiggleTuning> {
        let depletion_rate = match self {
            Stun::Weapon { .. } => 0.3,
            Stun::Flare | Stun::Shotgun | Stun::Police | Stun::Sweater => 0.2,
            Stun::BearTrap => 0.02,
            Stun::Firecracker | Stun::PocketKnife | Stun::GrabBreak | Stun::MaskOff { .. } => return None,
        };
        Some(f13_core::stun::WiggleTuning { total_time: self.seconds(), percent_per_wiggle: 5.0, depletion_rate })
    }

    /// Enter, loop (until the exit is due) and exit animations; a single animation plays once and holds.
    fn anims(self) -> (&'static str, Option<&'static str>, Option<&'static str>) {
        match self {
            Stun::Weapon { left: true } | Stun::MaskOff { left: true } => ("combat_stand_stunned_left_enter_01", Some("combat_stand_stunned_left_loop_01"), Some("combat_stand_stunned_left_exit_01")),
            Stun::Weapon { left: false } | Stun::MaskOff { left: false } => ("combat_stand_stunned_right_enter_01", Some("combat_stand_stunned_right_loop_01"), Some("combat_stand_stunned_right_exit_01")),
            Stun::Flare => ("combat_stand_stunned_back_enter_01", Some("combat_stand_stunned_back_loop_01"), Some("combat_stand_stunned_back_exit_01")),
            Stun::Shotgun => ("combat_stand_stunned_down_enter_01", Some("combat_stand_stunned_down_loop_01"), Some("combat_stand_stunned_down_exit_01")),
            Stun::Firecracker => ("combat_stand_stunned_firecracker_enter_01", Some("combat_stand_stunned_firecracker_loop_01"), Some("combat_stand_stunned_firecracker_exit_01")),
            Stun::Police => ("combat_stand_stunned_police_02", Some("combat_stand_stunned_police_loop_01"), Some("combat_stand_stunned_police_exit_01")),
            Stun::PocketKnife => ("normal_stand_knife_pullout", None, None),
            Stun::GrabBreak => ("normal_grab_break", None, None),
            Stun::Sweater => ("combat_stand_2h_axe_sweater", None, None),
            Stun::BearTrap => ("normal_stand_trap01_enter_lf", Some("normal_stand_trap01_cycle_lf"), Some("normal_stand_trap01_exit_lf")),
        }
    }
}

/// His mask (the Mask part, JasonSkeletalMask) hidden once it's knocked off.
fn show_mask(jasons: Query<(Entity, &Jason), Changed<Jason>>, children: Query<&Children>, parts: Query<&crate::character::PartOf>, mut vis: Query<&mut Visibility>) {
    for (je, j) in &jasons {
        for c in children.iter_descendants(je) {
            if parts.get(c).is_ok_and(|p| p.0 == "Mask") {
                if let Ok(mut v) = vis.get_mut(c) {
                    let want = if j.mask_on { Visibility::Inherited } else { Visibility::Hidden };
                    if *v != want {
                        *v = want;
                        info!("Jason: his mask {}", if j.mask_on { "shown" } else { "hidden" });
                    }
                }
            }
        }
    }
}

/// A counselor's item hit Jason (items.rs): `stun` if it stunned him, else he flinches.
#[derive(Component, Clone, Debug)]
pub struct HitJason {
    /// The counselor who hit him (none: the police).
    pub by: Option<Entity>,
    pub stun: Option<Stun>,
    pub from: Vec3,
    /// Its damage (a weapon's; the stuns' own, 0, PLACEHOLDER).
    pub damage: f32,
    /// The stun's StunTimeModifier: 1 + the attacker's perks' WeaponStunDurationModifier her weapon qualifies for
    /// (FUN_14040b600; the melee hit's weapon here).
    pub stun_scale: f32,
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
    her: Query<(&Transform, Option<&crate::fear_events::Pushed>), (With<Player>, With<crate::controls::LocalPlayer>)>,
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

/// What a client shows of the server's Jason (net.rs): the states his looks, sounds and cameras follow.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct RemoteView {
    pub shifting: bool,
    pub stalking: bool,
    pub holding: bool,
    pub knife_aim: bool,
    /// Faded out (a Shift or Morph) or hidden under the water.
    pub hidden: bool,
    /// The door he's opening or breaking down (its entity here).
    pub door: Option<(Entity, DoorMove)>,
    /// The throwing knives he's picking up (their pickup's entity here).
    pub knife_pickup: Option<Entity>,
    /// His Shift's and Stalk's ability bars while they run (net.rs NetJason).
    pub shift_bar: Option<f32>,
    pub stalk_bar: Option<f32>,
    /// His focused door (its entity here) and how long he's held her.
    pub focus_door: Option<Entity>,
    pub holding_for: Option<f32>,
    /// His stun's minigame: its completion and StunText.
    pub minigame: Option<(f32, &'static str)>,
}

/// What he's doing at a door.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum DoorMove {
    OpenInside,
    OpenOutside,
    Break,
}

impl Jason {
    /// The door move under way, for a client's camera.
    pub fn door_move(&self) -> Option<(Entity, DoorMove)> {
        match self.state {
            JasonState::OpeningDoor { door, anim, .. } => Some((door, if anim == DOOR_OPEN_INSIDE { DoorMove::OpenInside } else { DoorMove::OpenOutside })),
            JasonState::BreakingDoor { door, .. } => Some((door, DoorMove::Break)),
            _ => None,
        }
    }

    /// Faded out (a Shift or Morph) or hidden under the water (his visibility, as his tick sets it).
    pub fn hidden(&self) -> bool {
        self.remote.map_or(self.faded || self.under_hidden, |r| r.hidden)
    }

    /// His state's name (for a client's HUD).
    pub fn state_name(&self) -> String {
        let s = format!("{:?}", self.state);
        s.split([' ', '(', '{']).next().unwrap_or("").to_string()
    }
}

/// The local counselor's wiggle input (GLB_Wiggle: -1 while E is down), every counselor's (by her entity), and the
/// grab struggle's minigame (f13-core::stun).
#[derive(Resource, Default)]
pub struct Struggle {
    pub input: f32,
    pub inputs: std::collections::HashMap<Entity, f32>,
    pub wiggle: f13_core::stun::Wiggle,
}
/// SCCharacterAIProperties (constructor): an AI character wiggles every Min..MaxWiggleInterval seconds.
const AI_WIGGLE_INTERVAL: (f32, f32) = (0.15, 0.2);
/// SCCharacter InvulnerableStunDuration (constructor): no new stun this long after one ends.
const STUN_IMMUNE_SECONDS: f32 = 3.0;
const FLINCH_FRONT: &str = "combat_stand_hit_reaction_front";
/// His KillStanceMontage (AM_normal_death_sequence)'s takes: entrance, idle, exit; and how long he kneels
/// (PamelasSweaterStun's TotalStunTime, which EnterKillStance reapplies).
const KILL_STANCE: (&str, &str, &str) = ("normal_death_sequence_entrance", "normal_death_sequence_idle", "normal_death_sequence_exit");
const KILL_STANCE_S: f32 = 10.0;

/// A counselor kills him (jason_death.rs): his take.
#[derive(Component, Clone, Debug)]
pub struct KillJason {
    pub take: &'static str,
    pub by: Entity,
}

/// A stun ends his screen-effect abilities (the stun's application FUN_1403f6070, locally controlled: CancelStalk, and
/// CancelSense if Sense is on): Stalk's recharge timer back to 0, Sense's unused share refunded (as its own cancel).
/// (Neither can start again while he's stunned, so cancelling whenever he is equals cancelling as it starts.)
fn cancel_on_stun(time: Res<Time>, mut jasons: Query<(&mut Jason, &mut crate::senses::Knowledge)>) {
    let now = time.elapsed_secs();
    for (mut j, mut k) in &mut jasons {
        if !j.stunned() {
            continue;
        }
        if j.stalk.is_some() {
            j.stalk = None;
            k.abilities.consume(Ability::Stalk);
            info!("Jason: Stalk cancelled (stunned)");
        }
        if k.sensing(now) {
            let unused = (k.sense_until - now) / k.sense_length.max(1e-3);
            k.abilities.refund(Ability::Sense, unused);
            k.sense_until = now;
            info!("Jason: Sense cancelled (stunned)");
        }
    }
}

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

/// How far the computer Jason goes for throwing knives when he has none (m; PLACEHOLDER: the AI's choice).
const KNIFE_SEEK_M: f32 = 60.0;

/// His way onto a special move's spot (SCSpecialMove_SoloMontage's interp: from where he stood, its params).
#[derive(Component, Clone, Copy)]
pub struct AlignTo {
    from: Transform,
    to: Transform,
    params: crate::interact::MoveParams,
}

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
                health: config.health.unwrap_or(tuning.max_health),
                mask_on: true,
                sweater_stunned: false,
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
                // (F13_JASON_KNIVES: another count to start with, testing.)
                knives: std::env::var("F13_JASON_KNIVES").ok().and_then(|v| v.parse().ok()).unwrap_or(tuning.num_knives),
                kill: 0,
                kill_move: (kills()[0].his.as_str(), kills()[0].hers.as_str(), kills()[0].name.as_str()),
                traps: tuning.trap_count,
                knife_cooldown: 0.0,
                trap_spots: Vec::new(),
                faded: false,
                stun_wiggle: default(),
                stun_done: None,
                stun_scale: 1.0,
                next_wiggle: (-1.0, 0.0),
                underwater: false,
                remote: None,
                remote_state: String::new(),
                underwater_speed: None,
                under_hidden: false,
                kill_holds: true,
                focus: None,
                running: false,
                fast_walk: false,
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

/// A kill's victim whose side outlasts his (her special move's bDieOnFinish): she dies as it ends, `left` s on.
#[derive(Component)]
pub struct FinishingKill {
    left: f32,
    kill_name: &'static str,
    score: Option<(String, &'static str)>,
}

fn finish_kills(mut commands: Commands, time: Res<Time>, ptuning: Res<PlayerTuning>, mut victims: Query<(Entity, &mut Player, &mut FinishingKill)>) {
    for (pe, mut p, mut f) in &mut victims {
        f.left -= time.delta_secs();
        if f.left > 0.0 {
            continue;
        }
        if p.counselor.alive() {
            let health = p.counselor.health;
            p.counselor.take_damage(Hit { amount: health, blocked_with_weapon: false }, &ptuning.counselor);
            info!("Vanessa: killed by Jason ({}, as her side ended)", f.kill_name);
            if let Some((class, component)) = f.score.take() {
                commands.trigger(crate::match_flow::ContextKillScore { class, component });
            }
        }
        commands.entity(pe).remove::<(Held, PlayAction, FinishingKill)>();
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
pub(crate) fn read_struggle(
    keys: Res<ButtonInput<KeyCode>>,
    mut s: ResMut<Struggle>,
    play_jason: Option<Res<crate::jason_player::PlayJason>>,
    pads: crate::controls::Pads,
    counselors: Query<(Entity, Has<crate::controls::LocalPlayer>), With<Player>>,
) {
    s.input = if play_jason.is_none() && keys.pressed(KeyCode::KeyE) { -1.0 } else { 0.0 };
    s.inputs.clear();
    for (e, local) in &counselors {
        let input = if local { s.input } else if pads.pressed(e, crate::controls::Button::Interact) { -1.0 } else { 0.0 };
        s.inputs.insert(e, input);
    }
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
/// have no camera. Picking up throwing knives the same: their AlignmentSpecialMove's camera.
fn jason_door_camera(
    mut commands: Commands,
    play: Option<Res<crate::jason_player::PlayJason>>,
    jasons: Query<(Entity, &Jason, &Transform)>,
    doors: Query<&GlobalTransform, With<crate::doors::Door>>,
    mut playing: Local<Option<(Entity, &'static str)>>,
    knives: Query<(), With<crate::props::KnifePickup>>,
) {
    if play.is_none() {
        return;
    }
    let Some((je, j, xf)) = jasons.iter().next() else { return };
    let inside = |d: Entity| doors.get(d).is_ok_and(|g| crate::doors::Door::is_inside(g, xf.translation));
    let now = match (j.remote.and_then(|r| r.door), &j.state) {
        (Some((door, kind)), _) => Some((
            door,
            match kind {
                DoorMove::OpenInside => "KillerOpenDoorInside_SpecialMoveHandler",
                DoorMove::OpenOutside => "KillerOpenDoorOutside_SpecialMoveHandler",
                DoorMove::Break if inside(door) => "BreakDownFromInside_SpecialMoveHandler",
                DoorMove::Break => "BreakDownFromOutside_SpecialMoveHandler",
            },
        )),
        (_, &JasonState::OpeningDoor { door, anim, .. }) => {
            Some((door, if anim == DOOR_OPEN_INSIDE { "KillerOpenDoorInside_SpecialMoveHandler" } else { "KillerOpenDoorOutside_SpecialMoveHandler" }))
        }
        (_, &JasonState::BreakingDoor { door, .. }) => Some((door, if inside(door) { "BreakDownFromInside_SpecialMoveHandler" } else { "BreakDownFromOutside_SpecialMoveHandler" })),
        (_, &JasonState::BreakingFuse { fuse, .. }) if knives.contains(fuse) => Some((fuse, "AlignmentSpecialMove")),
        // (A client's Jason: the pickup the server has him at.)
        _ if j.remote.and_then(|r| r.knife_pickup).is_some() => j.remote.and_then(|r| r.knife_pickup).map(|k| (k, "AlignmentSpecialMove")),
        _ => None,
    };
    // A move's handler is chosen as it starts (a door found locked mid-open goes on to the break down's).
    let now = match (*playing, now) {
        (Some((d, h)), Some((nd, nh))) if d == nd && (h == nh || !nh.starts_with("Break")) => Some((d, h)),
        _ => now,
    };
    if now != *playing {
        match now {
            Some((actor, handler)) => commands.trigger(crate::interaction_camera::PlayInteractionCamera { actor, handler, killer: true, player: je }),
            None => commands.trigger(crate::interaction_camera::StopInteractionCamera { killer: true, player: je }),
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
    forced: Option<Res<crate::character::ForcedPose>>,
) {
    let (Some(killer), Some(t)) = (killer, tuning) else { return };
    // (`--pose` holds every rig in its pose: character.rs.)
    if forced.is_some() {
        return;
    }
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
            // His take's sounds: its montage's when they differ by montage, else his own montage's (the paired kills
            // share their names with hers: anim_sounds.json "killer/<take>"), else the take's.
            let span = rig.action_span(dt);
            let keyed = match (montage, span) {
                (Some(m), Some((take, from, to))) => Some((format!("{}/{take}", m.0), from, to)),
                (None, Some((take, from, to))) if anim_sounds.as_ref().is_some_and(|s| s.has(&format!("killer/{take}"))) => Some((format!("killer/{take}"), from, to)),
                _ => None,
            };
            // His AnimNotify_BloodyWeapon (the kills'): his weapon bloodied.
            if let Some((take, from, to)) = rig.action_span(dt) {
                if killer.info.animations.get(take).is_some_and(|a| a.windows.iter().any(|w| w.name == "BloodyWeapon" && from < w.start && w.start <= to)) {
                    commands.trigger(crate::materials::BloodyWeapon(1.0));
                }
            }
            let span = keyed.as_ref().map(|(k, from, to)| (k.as_str(), *from, *to)).or(span);
            crate::audio::play_anim_sounds(&mut commands, anim_sounds.as_deref(), span, at.translation, None, Some(parent.parent()), false);
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
