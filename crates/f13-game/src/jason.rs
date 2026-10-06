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
//! - Doors: a locked or barricaded door takes his weapon damage x DamageWorldModifier per swing from its
//!   BaseDoorHealth (60): three swings.
//!
//! PLACEHOLDERS (not from the game): the AI itself (the game's Jason is a player). He always knows where the
//! counselor is, picks grab or swing with even odds, and holds a grabbed counselor [`HOLD_BEFORE_KILL`] seconds
//! before killing her; she can't struggle free yet. Paths come from f13-core::nav over the collision (the game
//! uses Recast). The grabbed counselor is placed as SCCounselorCharacter::UpdateGrabTransform does (see [`place_held`]). No
//! Shift, Morph, Sense or Stalk; no character-to-character collision.

use bevy::prelude::*;
use f13_core::collision::{Capsule, WalkSettings};
use f13_core::health::Hit;
use f13_core::movement::{calc_velocity, input_acceleration, MovementSettings};
use f13_core::nav::NavGrid;
use std::collections::HashMap;

use crate::character::{KillerCharacter, Model, PlayAction, Rig};
use crate::player::{Player, PlayerTuning};

/// Unreal units (cm) per metre.
const CM: f32 = 100.0;
const WALK: WalkSettings = WalkSettings { max_step: 0.45, walkable_normal_y: 0.71, gravity: 9.8 };

const IDLE: &str = "normal_stand_idle_01";
/// BS_Jason_Normal_Move's SpeedState 1..3.
const MOVE: [&str; 3] = ["normal_stand_walk_forward", "normal_stand_walk_forward_fast", "normal_stand_run_forward_02"];
const COMBAT_IDLE: &str = "combat_stand_idle_2h";
const ATTACK: &str = "combat_stand_2h_attack_light";
const GRAB_STARTUP: &str = "normal_grab_startup";
const GRAB_ENTER: &str = "normal_grab_enter_1";
const GRAB_IDLE: &str = "normal_grab_idle";
const GRAB_MISS: &str = "normal_grab_miss";
const KILL: &str = "cinematic_pickaxe_throathack";

const HIT_BACK: &str = "f_normal_stand_hitreaction_back_01";
const HIT_LEFT: &str = "f_normal_stand_hitreaction_left_01";
const HIT_RIGHT: &str = "f_normal_stand_hitreaction_right_01";
const DEATH: &str = "f_normal_stand_death_01";
const GRABBED_ENTER: &str = "f_normal_grab_enter";
const GRABBED_IDLE: &str = "f_normal_grab_idle";
const COUNSELOR_KILL: &str = "cinematic_pickaxe_throathack";

/// Sockets of SCCounselorCharacter::UpdateGrabTransform: Jason_Base's CharacterAttachmentSocket, SCKillerCharacter's
/// hand socket (constructor), the counselor's neck socket (native FName).
const GRAB_SOCKET: &str = "GrabSocket";
const HAND_GRAB_SOCKET: &str = "handGrabSocket";
const NECK_GRAB_SOCKET: &str = "NeckGrabSocket";
/// Fallback when the sockets aren't posed yet: how far in front of Jason a held counselor stands.
const GRAB_HOLD_M: f32 = 0.65;
/// Seconds Jason holds a grabbed counselor before the kill (a player's choice in the game).
const HOLD_BEFORE_KILL: f32 = 2.5;
/// Odds of going for a grab rather than a swing, rolled after each attempt (AI choice).
const GRAB_ODDS: f32 = 0.5;
/// Paths: re-planned this often (s), with this many cells explored at most.
const REPLAN_SECONDS: f32 = 0.5;
const PATH_BUDGET: usize = 4000;
/// A closed door this close (m) in the direction Jason walks is opened (or broken).
const DOOR_REACH_M: f32 = 1.4;

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
            .add_systems(FixedUpdate, (update_jason, recover_counselor))
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
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum JasonState {
    /// Before the hunt starts (s left).
    Waiting(f32),
    Hunting,
    /// A pickaxe swing at the counselor, or at a door in his way.
    Swinging { t: f32, hit: bool, door: Option<Entity> },
    /// Reaching for a grab; it connects (or misses) when the startup ends.
    Reaching { t: f32 },
    Missed { t: f32 },
    Holding { t: f32 },
    Killing { t: f32 },
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
    replan: f32,
    cooldown: f32,
    wants_grab: bool,
    /// Where he was at the last progress check, and the time since.
    progress: (Vec3, f32),
    /// Walking a fixed direction for a moment to get unstuck.
    unstick: Option<(Vec2, f32)>,
}

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
struct Nav(NavGrid);

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
    let feet = f13_core::collision::spawn_floor(&world, start.translation, 1.5, &WALK).unwrap_or(start.translation);
    // Unreal actors face +X; characters here face +Z.
    let (yaw, _, _) = start.rotation.to_euler(EulerRot::YXZ);
    let rotation = Quat::from_rotation_y(yaw - std::f32::consts::FRAC_PI_2);
    info!("Jason at {feet} ({:?})", tuning);
    let jason = commands
        .spawn((
            Jason {
                state: JasonState::Waiting(config.delay),
                velocity: Vec2::ZERO,
                vel_y: 0.0,
                grounded: true,
                path: Vec::new(),
                replan: 0.0,
                cooldown: 0.0,
                wants_grab: rng.0.frand() < GRAB_ODDS,
                progress: (feet, 0.0),
                unstick: None,
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

/// Where Jason wants to go next (unit vector, XZ) on his way to `to`.
fn steer(j: &mut Jason, nav: &mut NavGrid, world: &(impl f13_core::collision::TriSource + ?Sized), from: Vec3, to: Vec3, dt: f32) -> Vec2 {
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
        j.path = nav.find_path(world, from, to, PATH_BUDGET).0;
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

#[allow(clippy::too_many_arguments, clippy::type_complexity)]
fn update_jason(
    mut commands: Commands,
    time: Res<Time>,
    tuning: Option<Res<KillerTuning>>,
    ptuning: Res<PlayerTuning>,
    nav: Option<ResMut<Nav>>,
    killer: Option<Res<KillerCharacter>>,
    counselor_model: Option<Res<crate::character::CounselorCharacter>>,
    collision: Res<crate::collision::WorldCollision>,
    dynamic: Res<crate::collision::DynamicColliders>,
    mut rng: ResMut<crate::GameRng>,
    mut jasons: Query<(Entity, &mut Jason, &mut Transform), Without<Player>>,
    mut players: Query<(Entity, &mut Player, &mut Transform, Has<crate::vehicle::Driving>), Without<Jason>>,
    mut doors: Query<(Entity, &mut crate::doors::Door, &GlobalTransform)>,
    mut barricades: Query<&mut crate::windows::Barricade>,
    rigs: Query<(&Rig, &ChildOf, &Transform), (Without<Jason>, Without<Player>)>,
    globals: Query<&GlobalTransform>,
) {
    let (Some(t), Some(mut nav), Some(killer)) = (tuning, nav, killer) else { return };
    let dt = time.delta_secs();
    let world = collision.with(&dynamic);
    let Ok((pe, mut p, mut pxf, driving)) = players.single_mut() else { return };
    for (je, mut j, mut xf) in &mut jasons {
        let to = pxf.translation;
        let flat = Vec2::new(to.x - xf.translation.x, to.z - xf.translation.z);
        let dist = flat.length();
        let angle = angle_to(xf.rotation, flat);
        let alive = p.counselor.alive();
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
                if !driving && j.cooldown <= 0.0 {
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
                        info!("Jason: swing ({dist:.2} m, {angle:.0} deg)");
                        j.state = JasonState::Swinging { t: 0.0, hit: false, door: None };
                        commands.entity(je).insert(PlayAction(ATTACK.into()));
                        continue;
                    }
                }
                // Close in: straight at her when near, else along a path.
                let mut wish = if dist < 1.5 { flat.normalize_or_zero() } else { steer(&mut j, &mut nav.0, &world, xf.translation, to, dt) };
                if dist < t.attack_range_m * 0.7 {
                    wish = Vec2::ZERO; // close enough for anything; don't walk into her
                }
                // A closed door ahead: open it, or break it down when it's locked or barred.
                let here = xf.translation;
                let door_ahead = doors
                    .iter()
                    .filter(|(_, d, _)| !d.open && !d.broken)
                    .find(|(_, _, g)| {
                        let to_door = Vec2::new(g.translation().x - here.x, g.translation().z - here.z);
                        to_door.length() < DOOR_REACH_M && (wish == Vec2::ZERO || to_door.normalize_or_zero().dot(wish) > 0.3)
                    })
                    .map(|(e, _, _)| e);
                if let Some(de) = door_ahead {
                    let barred = barricades.iter().any(|b| b.door == Some(de) && b.enabled);
                    if let Ok((_, mut d, g)) = doors.get_mut(de) {
                        if d.locked || barred {
                            info!("Jason: breaking a {} door (health {:.0})", if barred { "barricaded" } else { "locked" }, d.health);
                            j.state = JasonState::Swinging { t: 0.0, hit: false, door: Some(de) };
                            commands.entity(je).insert(PlayAction(ATTACK.into()));
                            continue;
                        }
                        info!("Jason: {} a door", d.press(g, here));
                    }
                }
                let speed = if t.can_run && dist > t.attack_range_m * 1.5 { t.run_speed } else { t.walk_speed };
                // Up close, square up to her (he only turns toward where he walks otherwise).
                let face = (dist < t.grab_max_m * 1.5).then_some(flat);
                walk(&mut j, &mut xf, wish, speed, face, dt, &t, &world);
                // Not getting anywhere: try another way for a moment.
                j.progress.1 += dt;
                if j.progress.1 >= 1.0 {
                    let moved = xf.translation.distance(j.progress.0);
                    if moved < 0.2 && wish != Vec2::ZERO && dist > 1.5 {
                        let a = rng.0.frand() * std::f32::consts::TAU;
                        j.unstick = Some((Vec2::new(a.cos(), a.sin()), 0.6));
                        j.path.clear();
                    }
                    j.progress = (xf.translation, 0.0);
                }
            }
            JasonState::Swinging { t: at, hit, door } => {
                let at = at + dt;
                let target = match door.and_then(|d| doors.get(d).ok()) {
                    Some((_, _, g)) => Vec2::new(g.translation().x - xf.translation.x, g.translation().z - xf.translation.z),
                    None => flat,
                };
                walk(&mut j, &mut xf, Vec2::ZERO, t.walk_speed, Some(target), dt, &t, &world);
                let mut hit = hit;
                if !hit && at >= t.attack_window.0 && at <= t.attack_window.1 {
                    match door {
                        Some(de) => {
                            hit = true;
                            if let Ok((_, mut d, _)) = doors.get_mut(de) {
                                let broke = d.damage(t.world_damage);
                                info!("Jason: hit the door ({:.0} left)", d.health);
                                if broke {
                                    info!("Jason: broke the door down");
                                    commands.entity(d.pivot).insert(Visibility::Hidden);
                                    for mut b in barricades.iter_mut().filter(|b| b.door == Some(de)) {
                                        b.enabled = false;
                                    }
                                }
                            }
                        }
                        None if alive && dist <= t.attack_range_m && angle_to(xf.rotation, flat) <= t.attack_angle_deg => {
                            hit = true;
                            let r = p.counselor.take_damage(Hit { amount: t.damage, blocked_with_weapon: false }, &ptuning.counselor);
                            info!("Jason: hit Vanessa for {:.0} (health {:.0})", r.dealt, p.counselor.health);
                            if r.died {
                                info!("Vanessa: killed by Jason's swing");
                                commands.entity(pe).insert(PlayAction(DEATH.into())).remove::<Staggered>();
                            } else if r.dealt > 0.0 {
                                let reaction = hit_reaction(&pxf, xf.translation);
                                let len = counselor_model.as_ref().and_then(|c| c.length(reaction)).unwrap_or(0.67);
                                commands.entity(pe).insert((PlayAction(reaction.into()), Staggered(len)));
                                p.velocity = Vec2::ZERO;
                            }
                        }
                        _ => {}
                    }
                }
                if at >= t.attack_length {
                    j.state = JasonState::Hunting;
                    j.cooldown = t.attack_delay;
                    j.wants_grab = rng.0.frand() < GRAB_ODDS;
                    commands.entity(je).remove::<PlayAction>();
                } else {
                    j.state = JasonState::Swinging { t: at, hit, door };
                }
            }
            JasonState::Reaching { t: at } => {
                let at = at + dt;
                walk(&mut j, &mut xf, flat.normalize_or_zero(), t.walk_speed, Some(flat), dt, &t, &world);
                let startup = killer.length(GRAB_STARTUP).unwrap_or(0.25);
                if at < startup {
                    j.state = JasonState::Reaching { t: at };
                } else if alive && !driving && dist <= t.grab_max_m && angle_to(xf.rotation, flat) <= t.grab_angle_deg {
                    info!("Jason: grabbed Vanessa");
                    j.state = JasonState::Holding { t: 0.0 };
                    j.velocity = Vec2::ZERO;
                    commands.entity(je).insert(PlayAction(GRAB_ENTER.into()));
                    commands.entity(pe).insert((Held, PlayAction(GRABBED_ENTER.into()))).remove::<Staggered>();
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
                if at >= HOLD_BEFORE_KILL {
                    info!("Jason: Pickaxe Throat Hack");
                    j.state = JasonState::Killing { t: 0.0 };
                    commands.entity(je).insert(PlayAction(KILL.into()));
                    commands.entity(pe).insert(PlayAction(COUNSELOR_KILL.into()));
                } else {
                    j.state = JasonState::Holding { t: at };
                }
            }
            JasonState::Killing { t: at } => {
                let at = at + dt;
                let jrig = rigs.iter().find(|(_, c, _)| c.parent() == je);
                let crig = rigs.iter().find(|(_, c, _)| c.parent() == pe);
                if let (Some((jr, _, _)), Some((cr, _, cxf)), Some(c)) = (jrig, crig, &counselor_model) {
                    place_held(pe, &mut pxf, true, jr, (cr, cxf), &killer.info, &c.info, &globals);
                }
                if at >= killer.length(KILL).unwrap_or(12.0) {
                    let health = p.counselor.health;
                    p.counselor.take_damage(Hit { amount: health, blocked_with_weapon: false }, &ptuning.counselor);
                    info!("Vanessa: killed by Jason (Pickaxe Throat Hack)");
                    j.state = JasonState::Done;
                    commands.entity(je).remove::<PlayAction>();
                } else {
                    j.state = JasonState::Killing { t: at };
                }
            }
            JasonState::Done => {
                walk(&mut j, &mut xf, Vec2::ZERO, t.walk_speed, None, dt, &t, &world);
            }
        }
    }
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
