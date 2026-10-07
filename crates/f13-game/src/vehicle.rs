//! Driving a repaired car and escaping through a car escape zone.
//!
//! Handling (f13-core::vehicle): the car's movement component's engine (torque curve, MaxRPM), automatic gearbox
//! (ratios, shift points, latency), idle brake and wheels (radius, steering lock), in a reduced model of the game's
//! PhysX vehicle (no sliding; grip-capped drive). The car turns like a bicycle on WHEELBASE_CM, follows the ground
//! under its axles and stops at obstacles.
//!
//! Starting is the game's: when a counselor drives, the starter's hold time becomes
//! `random[MinRestartTime, MaxRestartTime] * RestartTimeMod(stats, inverted)`, held like any interaction (after
//! `MinInteractHoldTime`, no pips).
//!
//! Only the driver can use the ignition (`SCDriveableVehicle::CanInteractWithStartVehicle` checks `Driver`); it
//! needs the ignition part (keys), which then stays in the car.
//!
//! Controls in the driver's seat: hold E to start, W/S throttle and reverse, A/D steer, F get out (when nearly
//! stopped), H horn (its HornAudioComponent); braking hard at speed plays its BrakeAudio.

use crate::interact::{take_part, Inventory};
use crate::player::PlayerTuning;
use f13_core::minigame::{HoldInteraction, MinigameEvent};
use crate::player::{Player, PlayerPart};
use crate::props::Item;
use crate::props::{EscapeZone, Vehicle, VehiclePart};
use bevy::picking::mesh_picking::ray_cast::{MeshRayCast, MeshRayCastSettings};
use bevy::prelude::*;

/// Braking faster than this (cm/s) skids audibly (PLACEHOLDER threshold).
const BRAKE_SOUND_CM_S: f32 = 500.0;
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
}

/// The counselor has escaped; the match is over for them.
#[derive(Resource)]
pub struct Escaped {
    pub how: String,
}

pub struct VehiclePlugin;

impl Plugin for VehiclePlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, (drive, follow_vehicle, exit_vehicle, check_escape, show_escape).chain());
    }
}

#[allow(clippy::too_many_arguments)]
fn drive(
    mut commands: Commands,
    keys: Res<ButtonInput<KeyCode>>,
    time: Res<Time>,
    escaped: Option<Res<Escaped>>,
    tuning: Res<PlayerTuning>,
    mut rng: ResMut<crate::GameRng>,
    mut player: Single<(&mut Driving, &mut Inventory, &Player)>,
    items: Query<&Item>,
    mut vehicles: Query<(&mut Vehicle, &mut Transform)>,
    mut ray_cast: MeshRayCast,
    vehicle_parts: Query<&VehiclePart>,
    player_parts: Query<(), With<PlayerPart>>,
    jasons: Query<&Transform, (With<crate::jason::Jason>, Without<Vehicle>)>,
) {
    let dt = time.delta_secs();
    let (ref mut player, ref mut inv, p) = *player;
    let Ok((mut v, mut xf)) = vehicles.get_mut(player.vehicle) else { return };
    if !v.started {
        let has_keys = inv.hand.iter().chain(inv.pockets.iter()).any(|&e| items.get(e).is_ok_and(|i| v.ignition.contains(&i.class)));
        if escaped.is_some() || !has_keys || !keys.pressed(KeyCode::KeyE) {
            player.starting = None;
            return;
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
            commands.trigger(crate::audio::ActorSound { actor: player.vehicle, prop: "EngineStartCue" });
            commands.trigger(crate::audio::ActorLoop { actor: player.vehicle, prop: "EngineCue" });
        }
        return;
    }
    let (mut throttle, mut steer) = (0.0f32, 0.0f32);
    if keys.just_pressed(KeyCode::KeyH) {
        commands.trigger(crate::audio::ActorSound { actor: player.vehicle, prop: "HornAudioComponent" });
    }
    if escaped.is_none() {
        if keys.pressed(KeyCode::KeyW) {
            throttle += 1.0;
        }
        if keys.pressed(KeyCode::KeyS) {
            throttle -= 1.0;
        }
        if keys.pressed(KeyCode::KeyD) {
            steer += 1.0;
        }
        if keys.pressed(KeyCode::KeyA) {
            steer -= 1.0;
        }
    }
    // The drivetrain: throttle forward or back (against the motion, the brakes): braking hard at speed skids.
    if throttle != 0.0 && throttle.signum() != v.speed.signum() && v.speed.abs() > BRAKE_SOUND_CM_S && !v.braking {
        commands.trigger(crate::audio::ActorSound { actor: player.vehicle, prop: "BrakeAudio" });
    }
    v.braking = throttle != 0.0 && throttle.signum() != v.speed.signum() && v.speed.abs() > BRAKE_SOUND_CM_S;
    let setup = v.setup.clone();
    v.drive.speed = v.speed;
    v.drive.step(&setup, throttle, dt);
    v.speed = v.drive.speed;
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
                return;
            }
        }
    }
    // Jason in front of (or behind) the car stops it dead (KillersInFrontOfCar / KillersBehindCar: the car is blocked
    // while he's in its KillerFront/RearCollisionVolume).
    if v.speed != 0.0 {
        let blocked = jasons.iter().any(|j| {
            let local = xf.rotation.inverse() * (j.translation - xf.translation);
            let ahead = local.x * v.speed.signum();
            ahead > 0.0 && ahead < HALF_LENGTH_M + BLOCK_DEPTH_M && local.z.abs() < BLOCK_HALF_WIDTH_M && local.y.abs() < 2.0
        });
        if blocked {
            info!("Jason stopped the car ({:.0} cm/s)", v.speed);
            v.speed = 0.0;
            return;
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

/// She sits in the driver's seat: her capsule at its socket (her feet a capsule half-height below), facing the car's
/// forward (its +X; her model faces +Z).
fn follow_vehicle(mut player: Single<(&Driving, &mut Transform), With<Player>>, vehicles: Query<(&Vehicle, &Transform), Without<Player>>) {
    let (d, ref mut xf) = *player;
    if let Ok((v, vxf)) = vehicles.get(d.vehicle) {
        **xf = *vxf * v.sitting * Transform::from_xyz(0.0, -crate::player::CAPSULE_HALF_HEIGHT_M, 0.0).with_rotation(Quat::from_rotation_y(std::f32::consts::FRAC_PI_2));
    }
}

fn exit_vehicle(
    mut commands: Commands,
    keys: Res<ButtonInput<KeyCode>>,
    escaped: Option<Res<Escaped>>,
    mut player: Single<(Entity, &Driving, &mut Transform, &mut Visibility), With<Player>>,
    vehicles: Query<(&Vehicle, &GlobalTransform)>,
) {
    if escaped.is_some() || !keys.just_pressed(KeyCode::KeyF) {
        return;
    }
    let (entity, d, ref mut xf, ref mut vis) = *player;
    let Ok((v, gxf)) = vehicles.get(d.vehicle) else { return };
    if v.speed.abs() > 100.0 {
        return;
    }
    // Out through the driver's door: a metre further out from the seat.
    let seat = v.driver_seat;
    let outward = Vec3::new(seat.x, 0.0, seat.z).normalize_or_zero();
    let mut at = gxf.transform_point(seat + outward * 1.0);
    at.y = gxf.translation().y;
    xf.translation = at;
    **vis = Visibility::Inherited;
    // Facing out of the door, upright.
    let (yaw, _, _) = gxf.rotation().to_euler(EulerRot::YXZ);
    xf.rotation = Quat::from_rotation_y(yaw + outward.x.atan2(outward.z));
    commands.entity(entity).remove::<(Driving, crate::character::PlayAction)>();
}

fn check_escape(
    mut commands: Commands,
    escaped: Option<Res<Escaped>>,
    player: Single<&Driving, With<Player>>,
    vehicles: Query<(&Vehicle, &GlobalTransform)>,
    zones: Query<(&EscapeZone, &GlobalTransform, &Name)>,
) {
    if escaped.is_some() {
        return;
    }
    let Ok((v, vxf)) = vehicles.get(player.vehicle) else { return };
    if !v.started {
        return;
    }
    for (zone, zxf, name) in &zones {
        if zone.car && vxf.translation().distance(zxf.translation()) <= zone.radius_m {
            info!("escaped by car through {name}");
            commands.insert_resource(Escaped { how: format!("by car ({})", v.class.trim_end_matches("_C")) });
            return;
        }
    }
}

#[derive(Component)]
struct EscapeBanner;

fn show_escape(mut commands: Commands, escaped: Option<Res<Escaped>>, banner: Query<(), With<EscapeBanner>>) {
    let Some(e) = escaped else { return };
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
