//! Driving a repaired car and escaping through a car escape zone.
//!
//! Handling (f13-core::vehicle_px): PhysX's 4-wheeled drive as the game sets it up from the car's movement component,
//! its wheels and its physics asset's chassis box; its suspension rays and its body's contacts against the world's
//! collision (the car's own parts left out). Without that setup in the export, the reduced model (f13-core::vehicle:
//! the car turns like a bicycle on WHEELBASE_CM, follows the ground under its axles, stops at obstacles).
//!
//! Starting is the game's: when a counselor drives, the starter's hold time becomes
//! `random[MinRestartTime, MaxRestartTime] * RestartTimeMod(stats, inverted)`, held like any interaction (after
//! `MinInteractHoldTime`, no pips).
//!
//! Only the driver can use the ignition (`SCDriveableVehicle::CanInteractWithStartVehicle` checks `Driver`); it
//! needs the ignition part (keys), which then stays in the car.
//!
//! Controls in the driver's seat: hold E to start, W/S throttle and reverse, A/D steer, F get out (when nearly
//! stopped), H horn (its HornAudioComponent), Space the handbrake (VHCL_Brake). Not ported: the handbrake's skid
//! (Chevette_BP's OnHandbrake plays its BrakeAudio at 350 cm/s or more, fading out over 0.5 s when let go or under
//! 250 cm/s).

use crate::interact::{take_part, Inventory};
use crate::player::PlayerTuning;
use f13_core::minigame::{HoldInteraction, MinigameEvent};
use crate::player::{Player, PlayerPart};
use crate::props::Item;
use crate::props::{EscapeZone, Vehicle, VehiclePart};
use bevy::picking::mesh_picking::ray_cast::{MeshRayCast, MeshRayCastSettings};
use bevy::prelude::*;
use crate::controls::Button;

/// The game's PhysicsSettings (DefaultEngine.ini): MaxSubstepDeltaTime and MaxSubsteps (bSubstepping on).
const PHYSICS_MAX_SUBSTEP_DT: f32 = 0.05;
const PHYSICS_MAX_SUBSTEPS: u32 = 4;
/// The Chevette's axles, front to back (its wheel bones are about this far apart).
const WHEELBASE_CM: f32 = 250.0;
/// Half the car's length (front bumper distance from the root), metres.
const HALF_LENGTH_M: f32 = 2.4;
/// How far beyond the bumper, and how far to either side of the middle, Jason blocks the car (m; the game's
/// KillerRearCollisionVolume: 20 × 90 cm half extents).
const BLOCK_DEPTH_M: f32 = 0.6;
const BLOCK_HALF_WIDTH_M: f32 = 0.9;

/// The player is in a vehicle's driver seat.
#[derive(Component)]
pub struct Driving {
    pub vehicle: Entity,
    /// Holding the ignition (the starter interaction).
    pub starting: Option<HoldInteraction>,
    /// Getting in or out: her take carrying her between the door and the seat.
    pub seat_move: Option<SeatMove>,
}

impl Driving {
    /// Getting into `vehicle`'s driver's seat (her enter take).
    pub fn enter(vehicle: Entity) -> Self {
        Driving { vehicle, starting: None, seat_move: Some(SeatMove { exiting: false, t: 0.0 }) }
    }
}

/// Her take getting in (f_normal_stand_car_enter_left: its root motion ends in the seat) or out
/// (f_normal_sit_car_exit_left: it starts there), played in the car's space.
#[derive(Clone, Copy, Debug)]
pub struct SeatMove {
    pub exiting: bool,
    t: f32,
}

impl SeatMove {
    pub fn take(self) -> &'static str {
        if self.exiting { EXIT_TAKE } else { ENTER_TAKE }
    }
}

const ENTER_TAKE: &str = "f_normal_stand_car_enter_left";
const EXIT_TAKE: &str = "f_normal_sit_car_exit_left";
/// Her take while she drives, and while she steers left or right.
pub const DRIVE_TAKE: &str = "f_normal_sit_car_drive_forward";
const STEER_TAKES: [&str; 2] = ["f_normal_sit_car_drive_forward_turnleft", "f_normal_sit_car_drive_forward_turnright"];

/// The counselor has escaped (and how); the match is over for her.
#[derive(Component)]
pub struct Escaped {
    pub how: String,
}

pub struct VehiclePlugin;

impl Plugin for VehiclePlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, (drive.run_if(crate::net::simulating), follow_vehicle, exit_vehicle.run_if(crate::net::simulating), check_escape.run_if(crate::net::simulating), show_escape).chain());
    }
}

#[allow(clippy::too_many_arguments)]
fn drive(
    mut commands: Commands,
    pads: crate::controls::Pads,
    time: Res<Time>,
    tuning: Res<PlayerTuning>,
    mut rng: ResMut<crate::GameRng>,
    mut drivers: Query<(Entity, &mut Driving, &mut Inventory, &Player, Option<&Escaped>)>,
    items: Query<&Item>,
    mut vehicles: Query<(&mut Vehicle, &mut Transform)>,
    mut ray_cast: MeshRayCast,
    vehicle_parts: Query<&VehiclePart>,
    player_parts: Query<(), With<PlayerPart>>,
    jasons: Query<(Entity, &Transform), (With<crate::jason::Jason>, Without<Vehicle>)>,
    (collision, dynamic, parents): (Res<crate::collision::WorldCollision>, Res<crate::collision::DynamicColliders>, Query<&ChildOf>),
    (no_vehicle, not_vehicle): (Query<(), With<crate::collision::NoVehicle>>, Option<Res<crate::collision::NotVehicle>>),
) {
    let dt = time.delta_secs();
    for (driver, mut player, mut inv, p, escaped) in &mut drivers {
    let (player, inv) = (&mut *player, &mut *inv);
    if player.seat_move.is_some() {
        continue;
    }
    let Ok((mut v, mut xf)) = vehicles.get_mut(player.vehicle) else { continue };
    if !v.started {
        let has_keys = inv.hand.iter().chain(inv.pockets.iter()).any(|&e| items.get(e).is_ok_and(|i| v.ignition.contains(&i.class)));
        if escaped.is_some() || !has_keys || !pads.pressed(driver, Button::Interact) {
            player.starting = None;
            continue;
        }
        let s = &tuning.vehicle_start;
        let hold = player.starting.get_or_insert_with(|| {
            let base = rng.0.frand() * (s.max_restart_time - s.min_restart_time) + s.min_restart_time;
            HoldInteraction::new(base * s.restart_time_mod.value(&p.counselor.stats, true), false)
        });
        let mut events = Vec::new();
        hold.tick(dt, &p.counselor.stats, &tuning.minigame, &mut rng.0, &mut events);
        if events.contains(&MinigameEvent::Completed) {
            if let Some(k) = take_part(inv, &items, &v.ignition) {
                commands.entity(k).despawn();
            }
            v.started = true;
            info!("{} started after holding {:.1}s", v.class, hold.timer);
            // The key turning, then the engine running (EngineStartCue, EngineCue).
            crate::net_world::shout(&mut commands, crate::net_world::Shout::ActorSound(crate::audio::ActorSound { actor: player.vehicle, prop: "EngineStartCue" }));
            // SCDriveableVehicle::StartVehicle (0x140446ad0): the starter's AppliedLastPart line.
            crate::net_world::shout(&mut commands, crate::net_world::Shout::ActorSound(crate::audio::ActorSound { actor: driver, prop: "VO:AppliedLastPart" }));
            crate::net_world::shout(&mut commands, crate::net_world::Shout::ActorLoop(crate::audio::ActorLoop { actor: player.vehicle, prop: "EngineCue" }));
        }
        continue;
    }
    let (mut throttle, mut steer) = (0.0f32, 0.0f32);
    if pads.just_pressed(driver, Button::Horn) {
        crate::net_world::shout(&mut commands, crate::net_world::Shout::ActorSound(crate::audio::ActorSound { actor: player.vehicle, prop: "HornAudioComponent" }));
    }
    if escaped.is_none() {
        if pads.pressed(driver, Button::Forward) {
            throttle += 1.0;
        }
        if pads.pressed(driver, Button::Back) {
            throttle -= 1.0;
        }
        if pads.pressed(driver, Button::Right) {
            steer += 1.0;
        }
        if pads.pressed(driver, Button::Left) {
            steer -= 1.0;
        }
    }
    v.throttle = throttle;
    v.handbrake = escaped.is_none() && pads.pressed(driver, Button::Space);
    if v.px.is_some() {
        let v = &mut *v;
        let me = player.vehicle;
        if v.sim.is_none() {
            let setup = v.px.clone().unwrap();
            v.sim = Some(f13_core::vehicle_px::Car::new(setup, to_unreal(xf.translation), ue_quat(xf.rotation)));
        }
        // What blocks the Vehicle channel, less the car's own colliders (its body, its parts: anything under it). The
        // suspension rays too: UE's WheelRaycastPreFilter blocks on the hit shape's response to the query's channel,
        // the car's own object type (Vehicle); so the police blockers (Vehicle ignored) let cars through.
        let own = |e: Entity| e == me || no_vehicle.contains(e) || vehicle_parts.get(e).is_ok_and(|p| p.0 == me) || parents.iter_ancestors(e).any(|a| a == me);
        let (grid, lists) = collision.without(&dynamic, own);
        let level = crate::collision::Excluding(grid, not_vehicle.as_ref().map(|g| &g.0));
        let world = (&level, lists.as_slice());
        let car = v.sim.as_mut().unwrap();
        // Moved from outside (the host, a bot): the simulated car starts again there, at rest.
        if xf.translation.distance(to_bevy(car.position)) > 0.5 {
            *car = f13_core::vehicle_px::Car::new(v.px.clone().unwrap(), to_unreal(xf.translation), ue_quat(xf.rotation));
        }
        // The suspension rays (Unreal space in and out).
        let mut ray = |start: Vec3, dir: Vec3, len: f32| {
            let (a, b) = (to_bevy(start), to_bevy(start + dir * len));
            f13_core::collision::segment_hit_normal(&world, a, b, 0.5).map(|(t, n)| {
                let n = to_unreal(n).normalize_or_zero();
                (t * len, if n.dot(dir) > 0.0 { -n } else { n })
            })
        };
        // The body's contacts: a corner moving into the world's surface this step.
        let mut contact = |from: Vec3, to: Vec3| {
            let d = to - from;
            if d.length_squared() < 1e-6 {
                return None;
            }
            f13_core::collision::segment_hit_normal(&world, to_bevy(from), to_bevy(to), 0.5).map(|(t, n)| {
                let n = to_unreal(n).normalize_or_zero();
                ((1.0 - t) * d.length(), if n.dot(d) > 0.0 { -n } else { n })
            })
        };
        let handbrake = escaped.is_none() && pads.pressed(driver, Button::Space);
        // FPhysSubstepTask::UpdateTime (0x141662930) under the game's PhysicsSettings (DefaultEngine.ini:
        // bSubstepping, MaxSubstepDeltaTime 0.05, MaxSubsteps 4): the frame's delta (SetUpForFrame 0x14162f470 keeps
        // it unclamped) capped at MaxSubsteps x MaxSubstepDeltaTime, in ceil(delta / MaxSubstepDeltaTime) equal
        // substeps (1..MaxSubsteps); the vehicles update each substep.
        let used = dt.min(PHYSICS_MAX_SUBSTEPS as f32 * PHYSICS_MAX_SUBSTEP_DT);
        let n = ((used / PHYSICS_MAX_SUBSTEP_DT).ceil() as u32).clamp(1, PHYSICS_MAX_SUBSTEPS);
        for _ in 0..n {
            car.step(used / n as f32, throttle, steer, handbrake, &mut ray, &mut contact);
        }
        // Jason in front of (or behind) the car stops it dead (KillersInFrontOfCar / KillersBehindCar).
        let speed = car.forward_speed();
        let (pos, rot) = (to_bevy(car.position), bevy_quat(car.rotation));
        let blocked = jasons.iter().find(|(_, j)| {
            let local = rot.inverse() * (j.translation - pos);
            let ahead = local.x * speed.signum();
            speed != 0.0 && ahead > 0.0 && ahead < HALF_LENGTH_M + BLOCK_DEPTH_M && local.z.abs() < BLOCK_HALF_WIDTH_M && local.y.abs() < 2.0
        });
        if let Some((je, _)) = blocked {
            if speed.abs() > 50.0 {
                info!("Jason stopped the car ({:.0} cm/s)", speed);
                commands.trigger(crate::jason::CarSlam { jason: je, car: pos });
            }
            car.velocity = Vec3::ZERO;
            car.angular_velocity = Vec3::ZERO;
        }
        xf.translation = to_bevy(car.position);
        xf.rotation = bevy_quat(car.rotation);
        v.speed = car.forward_speed();
        v.steer = car.steering_input;
        v.drive.rpm = car.rpm();
        if v.started {
            commands.entity(player.vehicle).insert(crate::audio::CueParams([("RPM".to_string(), v.drive.rpm)].into()));
        }
        continue;
    }
    // The drivetrain: throttle forward or back (against the motion, the brakes).
    let setup = v.setup.clone();
    v.drive.speed = v.speed;
    v.drive.step(&setup, throttle, dt);
    v.speed = v.drive.speed;
    // Its engine sound's RPM (SCDriveableVehicle's tick: GetEngineRotationSpeed, while started).
    if v.started {
        commands.entity(player.vehicle).insert(crate::audio::CueParams([("RPM".to_string(), v.drive.rpm)].into()));
    }
    v.steer += (steer - v.steer) * (dt * 6.0).min(1.0);

    let me = player.vehicle;
    let filter = |e: Entity| vehicle_parts.get(e).map_or(true, |p| p.0 != me) && !player_parts.contains(e);
    let settings = MeshRayCastSettings::default().with_filter(&filter);

    // Turn like a bicycle model, then move along the car's forward axis (its Unreal +X).
    let yaw_rate = v.speed / WHEELBASE_CM * (v.steer * setup.steer_at(v.speed).to_radians()).tan();
    let (yaw, _, _) = xf.rotation.to_euler(EulerRot::YXZ);
    let yaw = yaw - yaw_rate * dt;
    let forward = Quat::from_rotation_y(yaw) * Vec3::X;
    let step = forward * v.speed * dt / 100.0;

    // Stop at obstacles (anything that isn't ground-like) in front of or behind the car.
    if step.length_squared() > 0.0 {
        let dir = Dir3::new(step).unwrap();
        let origin = xf.translation + Vec3::Y * 0.7 + forward * HALF_LENGTH_M * v.speed.signum();
        if let Some((_, hit)) = ray_cast.cast_ray(Ray3d::new(origin, dir), &settings).first() {
            if hit.distance < step.length() + 0.1 && hit.normal.y < 0.7 {
                v.speed = 0.0;
                continue;
            }
        }
    }
    // Jason in front of (or behind) the car stops it dead (KillersInFrontOfCar / KillersBehindCar: the car is blocked
    // while he's in its KillerFront/RearCollisionVolume).
    if v.speed != 0.0 {
        let blocked = jasons.iter().find(|(_, j)| {
            let local = xf.rotation.inverse() * (j.translation - xf.translation);
            let ahead = local.x * v.speed.signum();
            ahead > 0.0 && ahead < HALF_LENGTH_M + BLOCK_DEPTH_M && local.z.abs() < BLOCK_HALF_WIDTH_M && local.y.abs() < 2.0
        });
        if let Some((je, _)) = blocked {
            // Hit at speed (not just nudged against him): he slams it.
            if v.speed.abs() > 50.0 {
                info!("Jason stopped the car ({:.0} cm/s)", v.speed);
                commands.trigger(crate::jason::CarSlam { jason: je, car: xf.translation });
            }
            v.speed = 0.0;
            continue;
        }
    }
    let mut pos = xf.translation + step;

    // Follow the ground under the front and rear axles.
    let ground = |ray_cast: &mut MeshRayCast, p: Vec3| {
        ray_cast.cast_ray(Ray3d::new(p + Vec3::Y * 2.0, Dir3::NEG_Y), &settings).first().map(|(_, h)| h.point.y)
    };
    let half_wb = WHEELBASE_CM / 200.0;
    let front = ground(&mut ray_cast, pos + forward * half_wb);
    let rear = ground(&mut ray_cast, pos - forward * half_wb);
    let mut pitch = 0.0;
    match (front, rear) {
        (Some(f), Some(r)) => {
            pos.y = (f + r) * 0.5;
            pitch = ((f - r) / (2.0 * half_wb)).atan();
        }
        (Some(y), None) | (None, Some(y)) => pos.y = y,
        _ => {}
    }
    xf.translation = pos;
    // Pitch about the car's right axis (Bevy +Z before yaw).
    xf.rotation = Quat::from_rotation_y(yaw) * Quat::from_rotation_z(pitch);
    }
}

fn to_unreal(v: Vec3) -> Vec3 {
    crate::counselor_camera::to_unreal(v)
}

fn to_bevy(v: Vec3) -> Vec3 {
    crate::counselor_camera::to_bevy(v)
}

/// A rotation in Unreal's space from Bevy's (the export's swap of Y and Z), and back: the same map both ways.
fn ue_quat(q: Quat) -> Quat {
    Quat::from_xyzw(-q.x, -q.z, -q.y, q.w)
}

fn bevy_quat(q: Quat) -> Quat {
    Quat::from_xyzw(-q.x, -q.z, -q.y, q.w)
}

/// She sits in the driver's seat: her capsule at its socket (her feet a capsule half-height below), facing the car's
/// forward (its +X; her model faces +Z). Getting in, her enter take plays from where its root motion ends in the seat;
/// getting out, her exit take from the seat, leaving her where it ends.
fn follow_vehicle(
    mut commands: Commands,
    time: Res<Time>,
    model: Option<Res<crate::character::CounselorCharacter>>,
    mut drivers: Query<(Entity, &mut Driving, &mut Transform, Option<&crate::character::PlayAction>), With<Player>>,
    vehicles: Query<(&Vehicle, &Transform), Without<Player>>,
) {
    for (pe, mut d, mut xf, action) in &mut drivers {
    let (d, xf) = (&mut *d, &mut *xf);
    let Ok((v, vxf)) = vehicles.get(d.vehicle) else { continue };
    let seat = *vxf * v.sitting * Transform::from_xyz(0.0, -crate::player::CAPSULE_HALF_HEIGHT_M, 0.0).with_rotation(Quat::from_rotation_y(std::f32::consts::FRAC_PI_2));
    let Some(m) = d.seat_move.as_mut() else {
        *xf = seat;
        // Steering: her turn take while the wheel is (mostly) over.
        let take = match v.steer {
            s if s < -0.5 => STEER_TAKES[0],
            s if s > 0.5 => STEER_TAKES[1],
            _ => DRIVE_TAKE,
        };
        if action.is_none_or(|a| a.0 != take) {
            commands.entity(pe).insert(crate::character::PlayAction(take.into()));
        }
        continue;
    };
    m.t += time.delta_secs();
    let take = m.take();
    let roots = model.as_ref().and_then(|c| c.root_motion(take));
    let len = model.as_ref().and_then(|c| c.length(take)).unwrap_or(0.0);
    // Where the take plays from: its last frame in the seat (in), its first (out).
    let origin = match (roots, m.exiting) {
        (Some((_, last)), false) => Transform::from_matrix(seat.to_matrix() * last.to_matrix().inverse()),
        (Some((first, _)), true) => Transform::from_matrix(seat.to_matrix() * first.to_matrix().inverse()),
        (None, _) => seat,
    };
    if m.t < len {
        *xf = origin;
        continue;
    }
    if m.exiting {
        // Standing where the take leaves her, upright.
        let end = roots.map_or(seat, |(_, last)| Transform::from_matrix(origin.to_matrix() * last.to_matrix()));
        let (yaw, _, _) = end.rotation.to_euler(EulerRot::YXZ);
        *xf = Transform::from_translation(end.translation).with_rotation(Quat::from_rotation_y(yaw));
        commands.entity(pe).remove::<(Driving, crate::character::PlayAction)>();
        info!("got out of the car");
    } else {
        d.seat_move = None;
        *xf = seat;
        commands.entity(pe).insert(crate::character::PlayAction(DRIVE_TAKE.into()));
    }
    }
}

/// F: out through the driver's door (her exit take), once the car has (nearly) stopped.
fn exit_vehicle(
    mut commands: Commands,
    pads: crate::controls::Pads,
    mut drivers: Query<(Entity, &mut Driving, Option<&Escaped>), With<Player>>,
    vehicles: Query<&Vehicle>,
) {
    for (entity, mut d, escaped) in &mut drivers {
        if escaped.is_some() || !pads.just_pressed(entity, Button::Flashlight) {
            continue;
        }
        let Ok(v) = vehicles.get(d.vehicle) else { continue };
        if v.speed.abs() > 100.0 || d.seat_move.is_some() {
            continue;
        }
        let m = SeatMove { exiting: true, t: 0.0 };
        d.seat_move = Some(m);
        d.starting = None;
        commands.entity(entity).insert(crate::character::PlayAction(m.take().into()));
    }
}

fn check_escape(
    mut commands: Commands,
    drivers: Query<(Entity, &Driving, Has<crate::controls::LocalPlayer>), (With<Player>, Without<Escaped>)>,
    vehicles: Query<(&Vehicle, &GlobalTransform)>,
    zones: Query<(Entity, &EscapeZone, &GlobalTransform, &Name)>,
) {
    for (pe, d, local) in &drivers {
        let Ok((v, vxf)) = vehicles.get(d.vehicle) else { continue };
        if !v.started {
            continue;
        }
        for (ze, zone, zxf, name) in &zones {
            if zone.car && vxf.translation().distance(zxf.translation()) <= zone.radius_m {
                info!("escaped by car through {name}");
                // The escape view is the local counselor's.
                if local {
                    commands.trigger(crate::kill_camera::EscapeCamera { zone: ze });
                }
                commands.entity(pe).insert(Escaped { how: format!("by car ({})", v.class.trim_end_matches("_C")) });
                break;
            }
        }
    }
}

#[derive(Component)]
struct EscapeBanner;

fn show_escape(mut commands: Commands, escaped: Query<&Escaped, With<crate::controls::LocalPlayer>>, banner: Query<(), With<EscapeBanner>>) {
    let Ok(e) = escaped.single() else { return };
    if !banner.is_empty() {
        return;
    }
    commands.spawn((
        Text::new(format!("YOU ESCAPED {}", e.how.to_uppercase())),
        TextFont { font_size: FontSize::Px(40.0), ..default() },
        Node { position_type: PositionType::Absolute, top: Val::Percent(40.0), left: Val::Percent(30.0), ..default() },
        EscapeBanner,
    ));
}
