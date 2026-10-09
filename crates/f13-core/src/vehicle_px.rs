//! The car as PhysX's 4-wheeled drive simulates it (PhysX 3.4's PxVehicleUpdate.cpp, PxVehicleUpdate::updateDrive4W:
//! the published source, F13-decomp/ref), set up as UE 4.18's PhysXVehicles plugin sets it up from the movement
//! component and its wheels (UWheeledVehicleMovementComponent(4W) / UVehicleWheel: their constructors' defaults read
//! from the game, 0x141f86e90 / 0x141f869b0 / 0x141f86830; the plugin's conversion recalled from the engine's published
//! source, MODLOG "Car handling (PhysX)").
//!
//! Units are Unreal's: cm, kg, s, Z up, X forward, Y right; torques in kg·cm²/s² (the plugin's M2ToCm2 of N·m).
//! Each step:
//! 1. the movement component's inputs (UpdateState: reverse as brake, the input rates; then PhysX's pad smoothing and
//!    its steer-vs-speed table);
//! 2. the suspension raycasts and PxVehicleUpdate in velocity-change mode: autobox, gears, the engine + driven wheels'
//!    implicit solve, springs, the default tyre model, sub-steps;
//! 3. what the PhysX scene then does to the chassis: gravity and the plugin's drag, the vehicle's suspension-limit and
//!    sticky-tyre constraints, and its contacts with the world (PLACEHOLDER: a few sequential-impulse iterations
//!    stand in for PhysX's solver; the car body's contacts are its root box's corners against the world, friction
//!    `BODY_FRICTION`), then the pose is integrated.

use glam::{Mat3, Quat, Vec3};

/// UE's default gravity (cm/s²).
pub const GRAVITY: f32 = 980.0;
/// The PhysX scene's tolerance length in Unreal (PxTolerancesScale::length: 100 cm).
const TOLERANCE_LENGTH: f32 = 100.0;
/// setVehicleToleranceScale.
const THRESHOLD_FORWARD_SPEED_WHEEL_ANGLE: f32 = 5.0 * TOLERANCE_LENGTH;
const MIN_LAT_SPEED_FOR_TIRE_MODEL: f32 = 1.0 * TOLERANCE_LENGTH;
const STICKY_TIRE_THRESHOLD_SPEED: f32 = 0.2 * TOLERANCE_LENGTH;
const MIN_SLIP_THRESHOLD: f32 = 1e-5;
const STICKY_FORWARD_DAMPING: f32 = 0.01;
const STICKY_SIDE_DAMPING: f32 = 0.1;
const LOW_FORWARD_SPEED_TIME: f32 = 1.0;
const LOW_SIDE_SPEED_TIME: f32 = 1.0;
/// gMinLongSlipDenominator x the tolerance length.
const MIN_LONG_SLIP_DENOMINATOR: f32 = 4.0 * TOLERANCE_LENGTH;
/// PxVehicleTireData's camber stiffness and its friction-vs-slip graph (both PhysX defaults; UE sets neither).
const CAMBER_STIFFNESS_PER_UNIT_GRAVITY: f32 = 0.1 * (180.0 / std::f32::consts::PI);
const FRICTION_VS_SLIP: [(f32, f32); 3] = [(0.0, 1.0), (0.1, 1.0), (1.0, 1.0)];
/// The plugin's air density (kg/cm³) in its drag.
const AIR_DENSITY: f32 = 1.25 / (100.0 * 100.0 * 100.0);
/// PLACEHOLDER: the car body's friction against the world, and the solver's iterations.
const BODY_FRICTION: f32 = 0.7;
const SOLVER_ITERATIONS: usize = 4;

/// One wheel's setup (UVehicleWheel, plus where its bone puts it).
#[derive(Clone, Debug, PartialEq)]
pub struct WheelSetup {
    /// The wheel's centre in the actor's space (its bone's position plus the setup's AdditionalOffset).
    pub position: Vec3,
    pub radius: f32,
    pub width: f32,
    pub mass: f32,
    /// N·m·s (DampingRate).
    pub damping_rate: f32,
    pub steer_deg: f32,
    /// N·m.
    pub max_brake: f32,
    pub max_handbrake: f32,
    pub affected_by_handbrake: bool,
    pub lat_stiff_max_load: f32,
    pub lat_stiff_value: f32,
    pub long_stiff_value: f32,
    pub suspension_force_offset: f32,
    pub max_raise: f32,
    pub max_drop: f32,
    pub natural_frequency: f32,
    pub damping_ratio: f32,
    /// The tyre's friction on the ground (its TireConfig's FrictionScale and the surface's).
    pub friction: f32,
}

/// The movement component's setup.
#[derive(Clone, Debug, PartialEq)]
pub struct CarSetup {
    pub mass: f32,
    /// The chassis body's centre of mass (actor space) and its box's full extents along its own X, Y, Z.
    pub com: Vec3,
    pub box_size: Vec3,
    /// The box's own frame in the actor's (PhysX's mass frame: its inertia's axes).
    pub box_rotation: Quat,
    pub inertia_scale: Vec3,
    /// (RPM, N·m).
    pub torque_curve: Vec<(f32, f32)>,
    pub max_rpm: f32,
    /// kg·m², N·m·s.
    pub engine_moi: f32,
    pub damping_full_throttle: f32,
    pub damping_zero_throttle_engaged: f32,
    pub damping_zero_throttle_disengaged: f32,
    /// Forward gears: (ratio, down, up).
    pub gears: Vec<(f32, f32, f32)>,
    pub reverse_ratio: f32,
    pub final_ratio: f32,
    pub gear_switch_time: f32,
    pub autobox_latency: f32,
    pub neutral_gear_up_ratio: f32,
    pub clutch_strength: f32,
    /// Open_FrontDrive: FrontLeftRightSplit.
    pub front_left_right_split: f32,
    pub ackermann_accuracy: f32,
    /// (km/h, scale).
    pub steering_curve: Vec<(f32, f32)>,
    pub drag_coefficient: f32,
    pub chassis_width: f32,
    pub chassis_height: f32,
    pub idle_brake: f32,
    pub stop_threshold: f32,
    pub wrong_direction_threshold: f32,
    /// (rise, fall) per second.
    pub throttle_rate: (f32, f32),
    pub brake_rate: (f32, f32),
    pub handbrake_rate: (f32, f32),
    pub steering_rate: (f32, f32),
    pub threshold_longitudinal_speed: f32,
    pub low_speed_substeps: u32,
    pub high_speed_substeps: u32,
    /// Min / min filtered / max / max filtered normalised tyre load.
    pub load_filter: [f32; 4],
    /// Front left, front right, rear left, rear right.
    pub wheels: [WheelSetup; 4],
}

const M2_TO_CM2: f32 = 10000.0;

fn rpm_to_omega(rpm: f32) -> f32 {
    rpm * std::f32::consts::PI / 30.0
}

/// A piecewise-linear table, clamped at its ends (PxFixedSizeLookupTable::getYVal).
fn lookup(table: &[(f32, f32)], x: f32) -> f32 {
    match table {
        [] => 0.0,
        [only] => only.1,
        _ => {
            if x <= table[0].0 {
                return table[0].1;
            }
            for w in table.windows(2) {
                if x < w[1].0 {
                    let t = (x - w[0].0) / (w[1].0 - w[0].0).max(1e-9);
                    return w[0].1 + (w[1].1 - w[0].1) * t;
                }
            }
            table[table.len() - 1].1
        }
    }
}

/// A wheel as PhysX has it (the plugin's SetupWheels).
#[derive(Clone, Debug)]
struct Wheel {
    /// From the centre of mass (body space).
    centre: Vec3,
    susp_force_point: Vec3,
    tire_force_point: Vec3,
    radius: f32,
    moi: f32,
    damping_rate: f32,
    max_steer: f32,
    max_brake: f32,
    max_handbrake: f32,
    max_compression: f32,
    max_droop: f32,
    spring: f32,
    damper: f32,
    sprung_mass: f32,
    rest_load: f32,
    lat_stiff_x: f32,
    lat_stiff_y: f32,
    long_stiff: f32,
    friction: f32,
}

/// The car's state.
#[derive(Clone, Debug)]
pub struct Car {
    /// The actor's root (world) and rotation.
    pub position: Vec3,
    pub rotation: Quat,
    /// The centre of mass's velocity and the angular velocity (world).
    pub velocity: Vec3,
    pub angular_velocity: Vec3,
    pub wheel_speeds: [f32; 4],
    pub wheel_angles: [f32; 4],
    pub jounces: [f32; 4],
    pub steer_angles: [f32; 4],
    pub in_air: [bool; 4],
    pub engine_omega: f32,
    /// PhysX's gear: 0 reverse, 1 neutral, 2.. forward.
    pub current_gear: usize,
    pub target_gear: usize,
    gear_switch_time: f32,
    autobox_switch_time: f32,
    gear_up: bool,
    gear_down: bool,
    low_forward: [f32; 4],
    low_side: [f32; 4],
    /// UE's smoothed inputs (UpdateState) and PhysX's (its analog inputs).
    pub steering_input: f32,
    pub throttle_input: f32,
    pub brake_input: f32,
    pub handbrake_input: f32,
    px_accel: f32,
    px_brake: f32,
    px_handbrake: f32,
    px_steer: f32,
    wheels: [Wheel; 4],
    inv_mass: f32,
    /// Mass-space inverse inertia (PhysX's vehicle update takes it in the chassis's frame).
    inv_inertia: Vec3,
    /// The mass frame's rotation in the actor (the scene's own integration uses it).
    mass_rotation: Quat,
    com: Vec3,
    setup: CarSetup,
    max_omega: f32,
    peak_torque: f32,
    torque_curve: Vec<(f32, f32)>,
    gear_ratios: Vec<f32>,
    up_ratios: Vec<f32>,
    down_ratios: Vec<f32>,
    axle_separation: f32,
    front_width: f32,
    rear_width: f32,
    /// The vehicle constraint rows of the update's first sub-step (the scene solves them).
    pending_sticky: Vec<Row>,
    pending_limits: Vec<Row>,
}

/// A 1D constraint row on the chassis: along `dir` (world) at `offset` from the centre of mass (world), to a velocity.
#[derive(Clone, Copy, Debug)]
struct Row {
    dir: Vec3,
    offset: Vec3,
    target: f32,
    error: f32,
}

/// What processSuspTireWheels gives back.
struct WheelsOut {
    force: Vec3,
    torque: Vec3,
    tire_torques: [f32; 4],
    jounces: [f32; 4],
    forward_speeds: [f32; 4],
    low_forward: [f32; 4],
    low_side: [f32; 4],
    in_air: [bool; 4],
    sticky: Vec<Row>,
    limits: Vec<Row>,
}

/// The ground a suspension raycast found: its distance along the ray and the surface's normal.
pub type RayHit = Option<(f32, Vec3)>;

impl Car {
    pub fn new(setup: CarSetup, position: Vec3, rotation: Quat) -> Self {
        let com = setup.com;
        // computeSprungMasses (4 wheels, gravity along Z): the masses on the wheels that put their centre at the
        // chassis's, as even as possible.
        let sprung = sprung_masses(&setup.wheels.each_ref().map(|w| w.position), com, setup.mass);
        let wheels = std::array::from_fn(|i| {
            let w = &setup.wheels[i];
            let m = sprung[i];
            let spring = w.natural_frequency * w.natural_frequency * m;
            let centre = w.position - com;
            // bDeprecatedSpringOffsetMode: the suspension and tyre forces at the wheel's X and Y, at
            // SuspensionForceOffset's height.
            let force_point = Vec3::new(w.position.x, w.position.y, w.suspension_force_offset) - com;
            Wheel {
                centre,
                susp_force_point: force_point,
                tire_force_point: force_point,
                radius: w.radius,
                moi: 0.5 * w.mass * w.radius * w.radius,
                damping_rate: w.damping_rate * M2_TO_CM2,
                max_steer: w.steer_deg.to_radians(),
                max_brake: w.max_brake * M2_TO_CM2,
                max_handbrake: if w.affected_by_handbrake { w.max_handbrake * M2_TO_CM2 } else { 0.0 },
                max_compression: w.max_raise,
                max_droop: w.max_drop,
                spring,
                damper: w.damping_ratio * 2.0 * (spring * m).sqrt(),
                sprung_mass: m,
                rest_load: w.mass + m,
                lat_stiff_x: w.lat_stiff_max_load,
                lat_stiff_y: w.lat_stiff_value,
                long_stiff: w.long_stiff_value,
                friction: w.friction,
            }
        });
        // The chassis box's inertia (a solid box), scaled to Mass and by InertiaTensorScale in the mass frame.
        let ext = setup.box_size;
        let local = Vec3::new(ext.y * ext.y + ext.z * ext.z, ext.x * ext.x + ext.z * ext.z, ext.x * ext.x + ext.y * ext.y) * (setup.mass / 12.0);
        // The box's axes in the actor's: the mass frame is the box's.
        let inertia = local * setup.inertia_scale;
        let peak_torque = setup.torque_curve.iter().map(|k| k.1).fold(0.0, f32::max);
        let torque_curve = setup.torque_curve.iter().map(|&(rpm, t)| ((rpm / setup.max_rpm).clamp(0.0, 1.0), t / peak_torque.max(1e-6))).collect();
        let mut gear_ratios = vec![setup.reverse_ratio, 0.0];
        let mut up_ratios = vec![0.0, setup.neutral_gear_up_ratio];
        // The autobox's latency is stored in its reverse down ratio.
        let mut down_ratios = vec![setup.autobox_latency, 0.0];
        for &(ratio, down, up) in &setup.gears {
            gear_ratios.push(ratio);
            down_ratios.push(down);
            up_ratios.push(up);
        }
        let p = setup.wheels.each_ref().map(|w| w.position);
        Car {
            position,
            rotation,
            velocity: Vec3::ZERO,
            angular_velocity: Vec3::ZERO,
            wheel_speeds: [0.0; 4],
            wheel_angles: [0.0; 4],
            jounces: [f32::MAX; 4],
            steer_angles: [0.0; 4],
            in_air: [true; 4],
            engine_omega: 0.0,
            current_gear: 1,
            target_gear: 1,
            gear_switch_time: 0.0,
            autobox_switch_time: 0.0,
            gear_up: false,
            gear_down: false,
            low_forward: [0.0; 4],
            low_side: [0.0; 4],
            steering_input: 0.0,
            throttle_input: 0.0,
            brake_input: 0.0,
            handbrake_input: 0.0,
            px_accel: 0.0,
            px_brake: 0.0,
            px_handbrake: 0.0,
            px_steer: 0.0,
            wheels,
            inv_mass: 1.0 / setup.mass,
            inv_inertia: inertia.recip(),
            mass_rotation: setup.box_rotation,
            com,
            max_omega: rpm_to_omega(setup.max_rpm),
            peak_torque: peak_torque * M2_TO_CM2,
            torque_curve,
            gear_ratios,
            up_ratios,
            down_ratios,
            axle_separation: (p[0].x - p[2].x).abs(),
            front_width: (p[1].y - p[0].y).abs(),
            rear_width: (p[3].y - p[2].y).abs(),
            pending_sticky: Vec::new(),
            pending_limits: Vec::new(),
            setup,
        }
    }

    /// The centre of mass (world).
    pub fn com_world(&self) -> Vec3 {
        self.position + self.rotation * self.com
    }

    /// GetForwardSpeed: the chassis's velocity along its forward (cm/s).
    pub fn forward_speed(&self) -> f32 {
        self.velocity.dot(self.rotation * Vec3::X)
    }

    /// The engine's speed (RPM).
    pub fn rpm(&self) -> f32 {
        self.engine_omega * 30.0 / std::f32::consts::PI
    }

    /// UE's gear numbering: -1 reverse, 0 neutral, 1.. forward.
    pub fn gear(&self) -> i32 {
        self.current_gear as i32 - 1
    }

    /// A wheel's place for drawing: its centre (world) at its jounce, its steer and its roll angle.
    pub fn wheel_pose(&self, i: usize) -> (Vec3, f32, f32) {
        let w = &self.wheels[i];
        let jounce = if self.jounces[i] == f32::MAX { 0.0 } else { self.jounces[i] };
        let centre = self.com_world() + self.rotation * (w.centre + Vec3::Z * jounce);
        (centre, self.steer_angles[i], self.wheel_angles[i])
    }

    /// One frame: `throttle` -1..1 (the raw throttle: back is reverse, through the brakes first), `steer` -1..1
    /// (right positive), the handbrake; `ray(start, dir, length)` finds the ground; `contact(point)` the world's
    /// surface at a point of the body (penetration depth along the normal, and the normal) for the body's contacts.
    pub fn step(
        &mut self,
        dt: f32,
        throttle: f32,
        steer: f32,
        handbrake: bool,
        ray: &mut dyn FnMut(Vec3, Vec3, f32) -> RayHit,
        contact: &mut dyn FnMut(Vec3) -> Option<(f32, Vec3)>,
    ) {
        if dt <= 0.0 {
            return;
        }
        self.update_inputs(dt, throttle, steer, handbrake);
        let hits = self.raycasts(ray);
        self.update_drive(dt, &hits);
        self.scene_step(dt, contact);
    }

    /// UWheeledVehicleMovementComponent::UpdateState with bReverseAsBrake, then PxVehicleDrive4WSmoothAnalog...
    fn update_inputs(&mut self, dt: f32, raw_throttle: f32, raw_steer: f32, handbrake: bool) {
        let s = self.setup.clone();
        let speed = self.forward_speed();
        let target = self.target_gear as i32 - 1;
        // Reverse and first are swapped through only when slow.
        if speed.abs() < s.wrong_direction_threshold {
            let current = self.current_gear as i32 - 1;
            if raw_throttle < 0.0 && current >= 0 && target >= 0 {
                self.set_target_gear(-1);
            } else if raw_throttle > 0.0 && current <= 0 && target <= 0 {
                self.set_target_gear(1);
            }
        }
        let target = self.target_gear as i32 - 1;
        let throttle_target = if (raw_throttle > 0.0 && target < 0) || (raw_throttle < 0.0 && target > 0) { 0.0 } else { raw_throttle.abs() };
        let brake_target = if raw_throttle > 0.0 {
            if speed < -s.wrong_direction_threshold { 1.0 } else { 0.0 }
        } else if raw_throttle < 0.0 {
            if speed > s.wrong_direction_threshold { 1.0 } else { 0.0 }
        } else if speed.abs() < s.stop_threshold {
            1.0
        } else {
            s.idle_brake
        };
        let interp = |rate: (f32, f32), current: f32, target: f32| {
            let delta = target - current;
            let rising = ((delta > 0.0) == (current > 0.0)) || (delta != 0.0 && current == 0.0);
            let max = if rising { rate.0 } else { rate.1 } * dt;
            current + delta.clamp(-max, max)
        };
        self.steering_input = interp(s.steering_rate, self.steering_input, raw_steer.clamp(-1.0, 1.0));
        self.throttle_input = interp(s.throttle_rate, self.throttle_input, throttle_target);
        self.brake_input = interp(s.brake_rate, self.brake_input, brake_target.clamp(0.0, 1.0));
        self.handbrake_input = interp(s.handbrake_rate, self.handbrake_input, if handbrake { 1.0 } else { 0.0 });
        // PhysX's pad smoothing (the same rates) and its steer-vs-speed table (km/h -> cm/s).
        let positive = |rate: (f32, f32), current: f32, target: f32| {
            if current < target { (current + rate.0 * dt).min(target) } else { (current - rate.1 * dt).max(target) }
        };
        self.px_accel = positive(s.throttle_rate, self.px_accel, self.throttle_input);
        self.px_brake = positive(s.brake_rate, self.px_brake, self.brake_input);
        self.px_handbrake = positive(s.handbrake_rate, self.px_handbrake, self.handbrake_input);
        let table: Vec<(f32, f32)> = s.steering_curve.iter().map(|&(kmh, v)| (kmh * 100000.0 / 3600.0, v.clamp(0.0, 1.0))).collect();
        let in_air = self.in_air.iter().all(|&a| a);
        let steer_target = self.steering_input * if in_air { 1.0 } else { lookup(&table, speed.abs()) };
        self.px_steer = analog(s.steering_rate, self.px_steer, steer_target, dt);
    }

    /// SetTargetGear(gear, immediate): straight there (PhysX's startGearChange... with immediate: forceGearChange).
    fn set_target_gear(&mut self, gear: i32) {
        let g = (gear + 1).clamp(0, self.gear_ratios.len() as i32 - 1) as usize;
        self.target_gear = g;
        self.current_gear = g;
    }

    /// PxVehicleWheels4SuspensionRaycasts: from the top of each wheel at full compression down its travel.
    fn raycasts(&self, ray: &mut dyn FnMut(Vec3, Vec3, f32) -> RayHit) -> [Option<(f32, Vec3, Vec3)>; 4] {
        let com = self.com_world();
        std::array::from_fn(|i| {
            let w = &self.wheels[i];
            let dir = self.rotation * Vec3::NEG_Z;
            let start = com + self.rotation * w.centre - dir * (w.radius + w.max_compression);
            let length = w.radius + w.max_compression + w.max_droop + w.radius;
            ray(start, dir, length).map(|(d, n)| (d, n, start + dir * d))
        })
    }

    /// PxVehicleUpdate::updateDrive4W (one block of four wheels: the drive wheels).
    fn update_drive(&mut self, timestep: f32, hits: &[Option<(f32, Vec3, Vec3)>; 4]) {
        let s = self.setup.clone();
        // Hit planes (the raycast's normal and point).
        let planes: [Option<(Vec3, f32)>; 4] = std::array::from_fn(|i| hits[i].filter(|h| h.0 != 0.0).map(|(_, n, p)| (n, -n.dot(p))));
        // processAutoBox and processGears.
        let mut accel = self.px_accel;
        if self.target_gear != self.current_gear && self.current_gear == 1 {
            accel = 0.0;
        }
        let latency = self.down_ratios[0];
        if self.target_gear == self.current_gear && self.autobox_switch_time > latency {
            let norm = self.engine_omega / self.max_omega;
            let g = self.current_gear;
            let up = norm > self.up_ratios[g] && g != 0;
            let down = norm < self.down_ratios[g] && g > 2;
            if up || down {
                self.gear_up = up;
                self.gear_down = down;
                self.autobox_switch_time = 0.0;
            }
        } else {
            self.autobox_switch_time += timestep;
        }
        self.process_gears(timestep);
        let g = self.current_gear;
        let k = if g != 1 { s.clutch_strength * M2_TO_CM2 } else { 0.0 };
        let ratio = self.gear_ratios[g] * s.final_ratio;
        let (brake, handbrake) = (self.px_brake, self.px_handbrake);
        let intent = accel > 0.0 && brake == 0.0 && handbrake == 0.0 && g != 1;
        // Open front drive.
        let split = s.front_left_right_split;
        let ave = [split, 1.0 - split, 0.0, 0.0];
        let accel_applied = ave.map(|a| intent && a != 0.0);
        self.steer_angles = self.ackermann(self.px_steer);
        // Sub-steps.
        let fwd = self.rotation * Vec3::X;
        let n = if self.velocity.dot(fwd).abs() < s.threshold_longitudinal_speed { s.low_speed_substeps } else { s.high_speed_substeps }.max(1);
        let sub = timestep / n as f32;
        let mut pose_p = self.com_world();
        let mut pose_q = self.rotation;
        let (mut v, mut w) = (self.velocity, self.angular_velocity);
        let mut sticky = Vec::new();
        let mut limits = Vec::new();
        for step in 0..n {
            let mut force = Vec3::ZERO;
            let mut torque = Vec3::ZERO;
            // computeBrakeAndHandBrakeTorques.
            let brake_torques: [f32; 4] = std::array::from_fn(|i| {
                let sign = sign(self.wheel_speeds[i]);
                -brake * sign * self.wheels[i].max_brake - handbrake * sign * self.wheels[i].max_handbrake
            });
            let brake_applied: [bool; 4] = std::array::from_fn(|i| brake * self.wheels[i].max_brake + handbrake * self.wheels[i].max_handbrake != 0.0);
            let out = self.process_wheels(pose_p, pose_q, v, w, &planes, &accel_applied, &brake_applied, intent, sub);
            force += out.force;
            torque += out.torque;
            if step == 0 {
                sticky = out.sticky.clone();
                limits = out.limits.clone();
            }
            // Torque split (open front drive) and the engine.
            let diff = ave;
            let drive = accel * self.peak_torque * lookup(&self.torque_curve, self.engine_omega / self.max_omega);
            let zero = if g != 1 { s.damping_zero_throttle_engaged } else { s.damping_zero_throttle_disengaged };
            let damping = (zero + (s.damping_full_throttle - zero) * accel) * M2_TO_CM2;
            self.solve_drive(sub, k, ratio, drive, damping, &diff, &ave, &brake_torques, &brake_applied, &out.tire_torques);
            // integrateWheelRotationAngles.
            for i in 0..4 {
                let mut omega = self.wheel_speeds[i];
                if out.jounces[i] > -self.wheels[i].max_droop
                    && !brake_applied[i]
                    && diff[i] * k * ratio * drive == 0.0
                    && out.forward_speeds[i].abs() < THRESHOLD_FORWARD_SPEED_WHEEL_ANGLE
                {
                    let alpha = out.forward_speeds[i].abs() / THRESHOLD_FORWARD_SPEED_WHEEL_ANGLE;
                    omega = out.forward_speeds[i] / self.wheels[i].radius * (1.0 - alpha) + omega * alpha;
                }
                let mut a = self.wheel_angles[i] + omega * sub;
                let lim = 10.0 * std::f32::consts::PI;
                if a > lim {
                    a -= lim;
                } else if a < -lim {
                    a += lim;
                }
                self.wheel_angles[i] = a;
            }
            self.low_forward = out.low_forward;
            self.low_side = out.low_side;
            self.jounces = out.jounces;
            self.in_air = out.in_air;
            // integrateBody (the inverse inertia taken in the chassis's frame).
            v += force * (self.inv_mass * sub);
            w += world_inv_inertia(self.inv_inertia, pose_q) * (torque * sub);
            pose_p += v * sub;
            let q = Quat::from_xyzw(w.x, w.y, w.z, 0.0) * pose_q * (sub * 0.5);
            pose_q = (pose_q + q).normalize();
        }
        self.velocity = v;
        self.angular_velocity = w;
        self.pending_sticky = sticky;
        self.pending_limits = limits;
    }

    fn process_gears(&mut self, timestep: f32) {
        let last = self.gear_ratios.len() - 1;
        if self.gear_up && self.current_gear != last && self.current_gear == self.target_gear {
            self.gear_switch_time = 0.0;
            self.target_gear = if self.current_gear <= 1 { 2 } else { self.current_gear + 1 };
            self.current_gear = 1;
        }
        if self.gear_down && self.current_gear != 0 && self.current_gear == self.target_gear {
            self.gear_switch_time = 0.0;
            self.target_gear = if self.current_gear <= 2 { 0 } else { self.current_gear - 1 };
            self.current_gear = 1;
        }
        if self.current_gear != self.target_gear {
            if self.gear_switch_time > self.setup.gear_switch_time {
                self.current_gear = self.target_gear;
                self.gear_switch_time = 0.0;
                self.gear_up = false;
                self.gear_down = false;
            } else {
                self.gear_switch_time += timestep;
            }
        }
    }

    /// computeAckermannCorrectedSteerAngles.
    fn ackermann(&self, steer: f32) -> [f32; 4] {
        let acc = self.setup.ackermann_accuracy;
        let pair = |gain: f32, width: f32| -> (f32, f32) {
            let angle = steer * gain;
            if angle == 0.0 {
                return (0.0, 0.0);
            }
            let right = angle.abs();
            let dz = self.axle_separation;
            let dx = width + dz / right.tan();
            let left_perfect = (dz / dx).atan();
            let left = right + acc * (left_perfect - right);
            if angle >= 0.0 { (left, right) } else { (-right, -left) }
        };
        let w = &self.wheels;
        let (fl, fr) = pair(w[0].max_steer.max(w[1].max_steer), self.front_width);
        let (rl, rr) = pair(w[2].max_steer.max(w[3].max_steer), self.rear_width);
        [fl, fr, -rl, -rr]
    }
}

fn sign(f: f32) -> f32 {
    if f > 0.0 {
        1.0
    } else if f < 0.0 {
        -1.0
    } else {
        0.0
    }
}

/// processAnalogValue (-1..1).
fn analog(rate: (f32, f32), current: f32, target: f32, dt: f32) -> f32 {
    let (rise, fall) = rate;
    if target == 0.0 {
        if current > 0.0 {
            (current - fall * dt).max(0.0)
        } else if current < 0.0 {
            (current + fall * dt).min(0.0)
        } else {
            0.0
        }
    } else if current < target {
        if current < 0.0 { (current + fall * dt).min(target) } else { (current + rise * dt).min(target) }
    } else if current > 0.0 {
        (current - fall * dt).max(target)
    } else {
        (current - rise * dt).max(target)
    }
}

/// The chassis's inverse inertia in the world (transformInertiaTensor: the mass-space one in the chassis's frame).
fn world_inv_inertia(inv: Vec3, q: Quat) -> Mat3 {
    let m = Mat3::from_quat(q);
    m * Mat3::from_diagonal(inv) * m.transpose()
}

/// PxVehicleComputeSprungMasses for 4 or more masses, gravity along Z: minimise the spread around the average subject
/// to the masses' centre being the chassis's (in X and Y) and their sum the total (Lagrange multipliers, LU solve).
pub fn sprung_masses(points: &[Vec3; 4], com: Vec3, total: f32) -> [f32; 4] {
    let n = 4;
    let size = n + 3;
    let mut a = vec![vec![0.0f64; size]; size];
    let mut b = vec![0.0f64; size];
    let mbar = total as f64 / n as f64;
    for i in 0..n {
        a[0][i] = points[i].x as f64;
        a[1][i] = points[i].y as f64;
        a[2][i] = 1.0;
    }
    b[0] = (total * com.x) as f64;
    b[1] = (total * com.y) as f64;
    b[2] = total as f64;
    for i in 0..n {
        a[i + 3][i] = 2.0;
        a[i + 3][n] = points[i].x as f64;
        a[i + 3][n + 1] = points[i].y as f64;
        a[i + 3][n + 2] = 1.0;
        b[i + 3] = 2.0 * mbar;
    }
    let x = solve(a, b);
    [x[0] as f32, x[1] as f32, x[2] as f32, x[3] as f32]
}

/// Gaussian elimination with partial pivoting (MatrixNNLUSolver's result).
fn solve(mut a: Vec<Vec<f64>>, mut b: Vec<f64>) -> Vec<f64> {
    let n = b.len();
    for c in 0..n {
        let p = (c..n).max_by(|&i, &j| a[i][c].abs().total_cmp(&a[j][c].abs())).unwrap_or(c);
        a.swap(c, p);
        b.swap(c, p);
        let d = a[c][c];
        if d.abs() < 1e-12 {
            continue;
        }
        for r in c + 1..n {
            let f = a[r][c] / d;
            if f == 0.0 {
                continue;
            }
            for k in c..n {
                a[r][k] -= f * a[c][k];
            }
            b[r] -= f * b[c];
        }
    }
    let mut x = vec![0.0; n];
    for r in (0..n).rev() {
        let s: f64 = (r + 1..n).map(|k| a[r][k] * x[k]).sum();
        x[r] = if a[r][r].abs() < 1e-12 { 0.0 } else { (b[r] - s) / a[r][r] };
    }
    x
}

impl Car {
    /// processSuspTireWheels (raycast hits): springs from the jounce, then the tyres at the filtered load.
    #[allow(clippy::too_many_arguments)]
    fn process_wheels(
        &self,
        com: Vec3,
        q: Quat,
        v: Vec3,
        w: Vec3,
        planes: &[Option<(Vec3, f32)>; 4],
        accel_applied: &[bool; 4],
        brake_applied: &[bool; 4],
        intent: bool,
        dt: f32,
    ) -> WheelsOut {
        let s = &self.setup;
        let gravity = Vec3::new(0.0, 0.0, -GRAVITY);
        let lat_dir = q * Vec3::Y;
        let mut out = WheelsOut {
            force: Vec3::ZERO,
            torque: Vec3::ZERO,
            tire_torques: [0.0; 4],
            jounces: [0.0; 4],
            forward_speeds: [0.0; 4],
            low_forward: self.low_forward,
            low_side: self.low_side,
            in_air: [true; 4],
            sticky: Vec::new(),
            limits: Vec::new(),
        };
        for i in 0..4 {
            let wh = &self.wheels[i];
            out.jounces[i] = -wh.max_droop;
            let travel = q * Vec3::NEG_Z;
            let Some((n, d)) = planes[i] else { continue };
            if n.dot(travel) >= 0.0 {
                continue;
            }
            // intersectRayPlane.
            let start = com + q * wh.centre - travel * (wh.radius + wh.max_compression);
            if n.dot(start) + d < 0.0 {
                continue;
            }
            let t = -(n.dot(start) + d) / n.dot(travel);
            let rest = 2.0 * wh.radius + wh.max_compression;
            let dx = rest - t;
            let bottom = start + travel * (rest - dx);
            if dx <= -wh.max_droop {
                continue;
            }
            out.in_air[i] = false;
            // The suspension limit: compressed past its max.
            if dx > wh.max_compression {
                out.limits.push(Row { dir: travel, offset: q * wh.centre, target: 0.0, error: travel.dot(n) * (-dx + wh.max_compression) });
            }
            let jounce = dx.min(wh.max_compression);
            out.jounces[i] = jounce;
            let r = bottom - com;
            let bottom_vel = v + w.cross(r);
            let jounce_speed = if self.jounces[i] != f32::MAX { (jounce - self.jounces[i]) / dt } else { 0.0 };
            let alpha = gravity.dot(travel).max(0.0);
            let t_beta = if alpha != 0.0 { gravity - travel * alpha } else { Vec3::ZERO };
            let mut f_w = (wh.sprung_mass * alpha + wh.spring * jounce).max(0.0);
            f_w += wh.damper * jounce_speed;
            let f_mag = n.dot(-travel * f_w - t_beta * wh.sprung_mass);
            let susp_force = n * f_mag;
            out.force += susp_force;
            out.torque += (q * wh.susp_force_point).cross(susp_force);
            // The tyre's load, normalised and filtered.
            let normalised = f_mag / GRAVITY / wh.rest_load;
            let [min_n, min_f, max_n, max_f] = s.load_filter;
            let filtered = if normalised <= min_n {
                min_f
            } else if normalised >= max_n {
                max_f
            } else {
                min_f + (normalised - min_n) * (max_f - min_f) / (max_n - min_n)
            };
            let filtered_load = filtered * GRAVITY * wh.rest_load;
            // computeTireDirs.
            let steer = self.steer_angles[i];
            let tz_raw = lat_dir.cross(n).normalize_or_zero();
            let tx_raw = n.cross(tz_raw).normalize_or_zero();
            let (sn, cs) = steer.sin_cos();
            let long_dir = tz_raw * cs + tx_raw * sn;
            let lat_dir_t = tx_raw * cs - tz_raw * sn;
            let long_speed = bottom_vel.dot(long_dir);
            let lat_speed = bottom_vel.dot(lat_dir_t);
            out.forward_speeds[i] = long_speed;
            // computeTireSlips.
            let omega = self.wheel_speeds[i];
            let wheel_speed = omega * wh.radius;
            let mut lat_slip = (lat_speed / (long_speed.abs() + MIN_LAT_SPEED_FOR_TIRE_MODEL)).atan();
            let mut long_slip = if long_speed == 0.0 && omega == 0.0 {
                0.0
            } else if brake_applied[i] || accel_applied[i] {
                (wheel_speed - long_speed) / (long_speed.abs().max(wheel_speed.abs()) + 0.1 * TOLERANCE_LENGTH)
            } else {
                (wheel_speed - long_speed) / MIN_LONG_SLIP_DENOMINATOR.max(long_speed.abs().max(wheel_speed.abs()))
            };
            // No camber at rest, compression or droop (none set).
            let camber = 0.0;
            // computeTireFriction.
            let ls = long_slip.abs();
            let [(x0, y0), (x1, y1), (x2, y2)] = FRICTION_VS_SLIP;
            let mu = if ls < x1 {
                y0 + (y1 - y0) * (ls - x0) / (x1 - x0)
            } else if ls < x2 {
                y1 + (y2 - y1) * (ls - x1) / (x2 - x1)
            } else {
                y2
            };
            let friction = mu * wh.friction;
            if filtered_load * wh.friction <= 0.0 {
                continue;
            }
            let tire_offset = q * wh.tire_force_point;
            // The low-speed timers and the sticky tyre constraints.
            let mut low_f = out.low_forward[i];
            if long_speed.abs() < STICKY_TIRE_THRESHOLD_SPEED && omega.abs() < STICKY_TIRE_THRESHOLD_SPEED / wh.radius && !intent {
                low_f += dt;
            } else {
                low_f = 0.0;
            }
            out.low_forward[i] = low_f;
            if (long_speed.abs() < STICKY_TIRE_THRESHOLD_SPEED && omega == 0.0 && !intent) || low_f > LOW_FORWARD_SPEED_TIME {
                out.sticky.push(Row { dir: long_dir, offset: tire_offset, target: long_speed * STICKY_FORWARD_DAMPING, error: 0.0 });
                long_slip = 0.0;
            }
            let mut low_s = out.low_side[i];
            if lat_speed.abs() < STICKY_TIRE_THRESHOLD_SPEED && !intent {
                low_s += dt;
            } else {
                low_s = 0.0;
            }
            out.low_side[i] = low_s;
            if low_f > 0.0 && low_s > LOW_SIDE_SPEED_TIME {
                out.sticky.push(Row { dir: lat_dir_t, offset: tire_offset, target: lat_speed * STICKY_SIDE_DAMPING, error: 0.0 });
                lat_slip = 0.0;
            }
            // PxVehicleComputeTireForceDefault.
            let (wheel_torque, fz, fx) = tire_force(wh, friction, long_slip, lat_slip, camber, GRAVITY * wh.rest_load, filtered, filtered_load);
            out.tire_torques[i] = wheel_torque;
            let tire_force = long_dir * fz + lat_dir_t * fx;
            out.force += tire_force;
            out.torque += tire_offset.cross(tire_force);
        }
        out
    }

    /// solveDrive4WInternaDynamicsEnginePlusDrivenWheels: the engine and the four wheels' speeds at the end of the
    /// sub-step, coupled through the clutch (an implicit 5x5 solve); then braked wheels that changed direction lock and
    /// the engine's revs are clamped.
    #[allow(clippy::too_many_arguments)]
    fn solve_drive(
        &mut self,
        dt: f32,
        k: f32,
        g: f32,
        drive: f32,
        damping: f32,
        diff: &[f32; 4],
        ave: &[f32; 4],
        brake: &[f32; 4],
        brake_applied: &[bool; 4],
        tire: &[f32; 4],
    ) {
        let (kg, kgg) = (k * g, k * g * g);
        let mut a = vec![vec![0.0f64; 5]; 5];
        let mut b = vec![0.0f64; 5];
        for i in 0..4 {
            let wh = &self.wheels[i];
            let dti = dt / wh.moi;
            let r = diff[i];
            let x = dti * kgg * r;
            for (j, &c) in ave.iter().enumerate() {
                a[i][j] = (x * c) as f64;
            }
            a[i][i] = (1.0 + x * ave[i] + dti * wh.damping_rate) as f64;
            a[i][4] = (-dti * kg * r) as f64;
            b[i] = (self.wheel_speeds[i] + dti * (brake[i] + tire[i])) as f64;
        }
        let dte = dt / (self.setup.engine_moi * M2_TO_CM2);
        for (j, &c) in ave.iter().enumerate() {
            a[4][j] = (-dte * k * g * c) as f64;
        }
        a[4][4] = (1.0 + dte * (k + damping)) as f64;
        b[4] = (self.engine_omega + dte * drive) as f64;
        let x = solve(a, b);
        for i in 0..4 {
            let nw = x[i] as f32;
            self.wheel_speeds[i] = if brake_applied[i] && self.wheel_speeds[i] * nw <= 0.0 { 0.0 } else { nw };
        }
        self.engine_omega = (x[4] as f32).clamp(0.0, self.max_omega);
    }

    /// What the scene does after the vehicle update: gravity and the plugin's drag, the vehicle's constraints and the
    /// body's contacts (sequential impulses), then the pose.
    fn scene_step(&mut self, dt: f32, contact: &mut dyn FnMut(Vec3) -> Option<(f32, Vec3)>) {
        let s = &self.setup;
        self.velocity += Vec3::new(0.0, 0.0, -GRAVITY) * dt;
        // UpdateDrag: 0.5 rho v^2 Cd A against the velocity.
        let speed_sq = self.velocity.length_squared();
        let drag = 0.5 * AIR_DENSITY * speed_sq * s.drag_coefficient * s.chassis_width * s.chassis_height;
        self.velocity -= self.velocity.normalize_or_zero() * drag * self.inv_mass * dt;
        let inv_i = world_inv_inertia(self.inv_inertia, self.rotation * self.mass_rotation);
        let com = self.com_world();
        // The body's contacts: the chassis box's corners where they'd be after this step.
        let half = s.box_size * 0.5;
        let mut contacts = Vec::new();
        for c in 0..8 {
            let local = Vec3::new(if c & 1 == 0 { -half.x } else { half.x }, if c & 2 == 0 { -half.y } else { half.y }, if c & 4 == 0 { -half.z } else { half.z });
            let offset = self.rotation * (self.mass_rotation * local);
            let point = com + offset + (self.velocity + self.angular_velocity.cross(offset)) * dt;
            if let Some((depth, n)) = contact(point) {
                contacts.push((offset, n, depth, 0.0f32, [0.0f32; 2]));
            }
        }
        let limits = std::mem::take(&mut self.pending_limits);
        let sticky = std::mem::take(&mut self.pending_sticky);
        let mut acc_limits = vec![0.0f32; limits.len()];
        let (inv_m, mut v, mut w) = (self.inv_mass, self.velocity, self.angular_velocity);
        let eff = |dir: Vec3, offset: Vec3| {
            let ang = offset.cross(dir);
            1.0 / (inv_m + ang.dot(inv_i * ang))
        };
        let spring = dt * 1000.0 / (1.0 + dt * 1000.0);
        for _ in 0..SOLVER_ITERATIONS {
            for (row, acc) in limits.iter().zip(acc_limits.iter_mut()) {
                let jv = row.dir.dot(v) + row.offset.cross(row.dir).dot(w);
                let target = row.error / dt;
                let new = (*acc + eff(row.dir, row.offset) * (target - jv)).min(0.0);
                let d = new - *acc;
                *acc = new;
                v += row.dir * (d * inv_m);
                w += inv_i * (row.offset.cross(row.dir) * d);
            }
            for row in &sticky {
                let jv = row.dir.dot(v) + row.offset.cross(row.dir).dot(w);
                let d = eff(row.dir, row.offset) * (row.target - jv) * spring / SOLVER_ITERATIONS as f32;
                v += row.dir * (d * inv_m);
                w += inv_i * (row.offset.cross(row.dir) * d);
            }
            for (offset, n, depth, acc, tacc) in contacts.iter_mut() {
                let pv = v + w.cross(*offset);
                let vn = pv.dot(*n);
                let target = (*depth / dt) * 0.2;
                let new = (*acc + eff(*n, *offset) * (target - vn)).max(0.0);
                let d = new - *acc;
                *acc = new;
                v += *n * (d * inv_m);
                w += inv_i * (offset.cross(*n) * d);
                // Friction along the two tangents, capped by the normal impulse.
                let t1 = n.any_orthonormal_vector();
                let t2 = n.cross(t1);
                for (k, t) in [t1, t2].into_iter().enumerate() {
                    let vt = (v + w.cross(*offset)).dot(t);
                    let cap = BODY_FRICTION * *acc;
                    let new_t = (tacc[k] - eff(t, *offset) * vt).clamp(-cap, cap);
                    let di = new_t - tacc[k];
                    tacc[k] = new_t;
                    v += t * (di * inv_m);
                    w += inv_i * (offset.cross(t) * di);
                }
            }
        }
        self.velocity = v;
        self.angular_velocity = w;
        // The pose (about the centre of mass).
        let new_com = com + v * dt;
        let q = Quat::from_xyzw(w.x, w.y, w.z, 0.0) * self.rotation * (dt * 0.5);
        self.rotation = (self.rotation + q).normalize();
        self.position = new_com - self.rotation * self.com;
    }
}

/// PxVehicleComputeTireForceDefault: (wheel torque, longitudinal force, lateral force).
#[allow(clippy::too_many_arguments)]
fn tire_force(w: &Wheel, friction: f32, long_slip: f32, lat_slip: f32, camber: f32, rest_load: f32, normalised: f32, load: f32) -> (f32, f32, f32) {
    let lat = if lat_slip.abs() >= MIN_SLIP_THRESHOLD { lat_slip } else { 0.0 };
    let long = if long_slip.abs() >= MIN_SLIP_THRESHOLD { long_slip } else { 0.0 };
    let camber = if camber.abs() >= MIN_SLIP_THRESHOLD { camber } else { 0.0 };
    if lat == 0.0 && long == 0.0 && camber == 0.0 {
        return (0.0, 0.0, 0.0);
    }
    let smooth1 = |k: f32| (k - k * k / 3.0 + k * k * k / 27.0).min(1.0);
    let lat_stiff = rest_load * w.lat_stiff_y * smooth1(normalised * 3.0 / w.lat_stiff_x);
    let long_stiff = w.long_stiff * GRAVITY;
    let camber_stiff = CAMBER_STIFFNESS_PER_UNIT_GRAVITY * GRAVITY;
    let t_eff = (lat - camber * camber_stiff / lat_stiff).tan();
    let k = ((lat_stiff * t_eff).powi(2) + (long_stiff * long).powi(2)).sqrt() / (friction * load);
    let f_bar = smooth1(k);
    let mut nu = 1.0;
    if k <= 2.0 * std::f32::consts::PI {
        let ratio = lat_stiff / long_stiff;
        nu = 0.5 * (1.0 + ratio - (1.0 - ratio) * (k * 0.5).cos());
    }
    let f_zero = friction * load / (long * long + nu * t_eff * nu * t_eff).sqrt();
    let fz = long * f_bar * f_zero;
    let fx = -nu * t_eff * f_bar * f_zero;
    (-fz * w.radius, fz, fx)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// The Chevette: its Blueprint over the constructors' defaults.
    pub fn chevette() -> CarSetup {
        let wheel = |x: f32, y: f32, front: bool| WheelSetup {
            position: Vec3::new(x, y, 32.52),
            radius: 35.0,
            width: 10.0,
            mass: 20.0,
            damping_rate: 0.25,
            steer_deg: if front { 35.0 } else { 0.0 },
            max_brake: 1500.0,
            max_handbrake: 3000.0,
            affected_by_handbrake: !front,
            lat_stiff_max_load: 2.0,
            lat_stiff_value: if front { 15.0 } else { 6.5 },
            long_stiff_value: 1000.0,
            suspension_force_offset: 0.0,
            max_raise: 10.0,
            max_drop: 10.0,
            natural_frequency: 7.0,
            damping_ratio: 1.0,
            friction: 1.0,
        };
        CarSetup {
            mass: 1500.0,
            com: Vec3::new(-10.0, 0.31, 87.06),
            box_size: Vec3::new(178.73, 106.39, 449.97),
            box_rotation: Quat::from_euler(glam::EulerRot::ZYX, (-90.54f32).to_radians(), 0.0, (-88.26f32).to_radians()),
            inertia_scale: Vec3::new(2.0, 1.0, 1.0),
            torque_curve: vec![(0.0, 400.0), (1890.0, 500.0), (5729.0, 400.0)],
            max_rpm: 2500.0,
            engine_moi: 1.0,
            damping_full_throttle: 0.15,
            damping_zero_throttle_engaged: 2.0,
            damping_zero_throttle_disengaged: 0.35,
            gears: vec![(4.0, 0.5, 0.85), (2.0, 0.5, 0.65)],
            reverse_ratio: -4.0,
            final_ratio: 4.0,
            gear_switch_time: 0.5,
            autobox_latency: 1.0,
            neutral_gear_up_ratio: 0.15,
            clutch_strength: 10.0,
            front_left_right_split: 0.5,
            ackermann_accuracy: 1.0,
            steering_curve: vec![(0.0, 1.0), (40.0, 0.7), (120.0, 0.6)],
            drag_coefficient: 0.3,
            chassis_width: 180.0,
            chassis_height: 140.0,
            idle_brake: 0.1,
            stop_threshold: 10.0,
            wrong_direction_threshold: 100.0,
            throttle_rate: (6.0, 10.0),
            brake_rate: (6.0, 10.0),
            handbrake_rate: (12.0, 12.0),
            steering_rate: (2.5, 5.0),
            threshold_longitudinal_speed: 5.0,
            low_speed_substeps: 3,
            high_speed_substeps: 1,
            load_filter: [0.0, 0.2308, 3.0, 3.0],
            wheels: [wheel(136.3, -70.35, true), wheel(136.3, 70.35, true), wheel(-137.03, -70.35, false), wheel(-137.03, 70.35, false)],
        }
    }

    /// Flat ground at z = 0.
    fn flat(start: Vec3, dir: Vec3, len: f32) -> RayHit {
        if dir.z >= 0.0 {
            return None;
        }
        let t = -start.z / dir.z;
        (t >= 0.0 && t <= len).then_some((t, Vec3::Z))
    }

    fn no_contact(_: Vec3) -> Option<(f32, Vec3)> {
        None
    }

    #[test]
    fn sprung_masses_hold_the_centre_of_mass() {
        let s = chevette();
        let m = sprung_masses(&s.wheels.each_ref().map(|w| w.position), s.com, s.mass);
        let total: f32 = m.iter().sum();
        assert!((total - s.mass).abs() < 0.5, "{m:?}");
        let x: f32 = m.iter().zip(&s.wheels).map(|(m, w)| m * w.position.x).sum::<f32>() / total;
        assert!((x - s.com.x).abs() < 0.1, "{x}");
    }

    #[test]
    fn it_settles_on_its_springs_and_stays_put() {
        let mut car = Car::new(chevette(), Vec3::new(0.0, 0.0, 10.0), Quat::IDENTITY);
        for _ in 0..600 {
            car.step(1.0 / 60.0, 0.0, 0.0, false, &mut flat, &mut no_contact);
        }
        assert!(car.velocity.length() < 1.0, "still moving: {:?}", car.velocity);
        let (fl, _, _) = car.wheel_pose(0);
        assert!((fl.z - 35.0).abs() < 3.0, "front left wheel at {fl:?}");
        assert!(car.in_air.iter().all(|a| !a));
    }

    #[test]
    fn throttle_drives_it_forward_through_its_gears_and_back_stops_it() {
        let mut car = Car::new(chevette(), Vec3::ZERO, Quat::IDENTITY);
        for _ in 0..120 {
            car.step(1.0 / 60.0, 0.0, 0.0, false, &mut flat, &mut no_contact);
        }
        for _ in 0..600 {
            car.step(1.0 / 60.0, 1.0, 0.0, false, &mut flat, &mut no_contact);
        }
        let fast = car.forward_speed();
        assert!(fast > 500.0, "only {fast} cm/s after 10 s");
        assert_eq!(car.gear(), 2, "top gear");
        for _ in 0..600 {
            car.step(1.0 / 60.0, -1.0, 0.0, false, &mut flat, &mut no_contact);
        }
        assert!(car.forward_speed() < 100.0, "still {} cm/s", car.forward_speed());
    }

    #[test]
    fn steering_turns_it() {
        let mut car = Car::new(chevette(), Vec3::ZERO, Quat::IDENTITY);
        for _ in 0..90 {
            car.step(1.0 / 60.0, 1.0, 1.0, false, &mut flat, &mut no_contact);
        }
        assert!(car.angular_velocity.z > 0.0, "yawing right: {:?}", car.angular_velocity);
        let heading = car.rotation * Vec3::X;
        assert!(heading.y > 0.1, "turned right: heading {heading:?}");
    }
}
