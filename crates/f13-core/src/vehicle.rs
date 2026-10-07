//! Car handling from the game's vehicle setup (SCWheeledVehicleMovementComponent, a UWheeledVehicleMovementComponent4W
//! over PhysX): the engine's torque curve and MaxRPM, the automatic gearbox's ratios, its up and down shift points
//! and latency, the final drive, the wheels' radius and steer angle, Unreal's defaults for what the Blueprint leaves
//! (1500 kg, final ratio 4, reverse -4, 1500 N·m brake torque per wheel, drag 3 on a 180 × 140 cm chassis, the
//! default steering curve).
//!
//! This is a reduced model, not PhysX: the car moves along its heading (no sliding), the engine's speed follows the
//! wheels through the gear (no clutch or engine inertia), and the drive is capped by the driven (front) tyres' grip
//! (friction 1 on the front axle's share of the weight). It keeps what the numbers decide: top speed per gear (MaxRPM
//! through the ratios), the torque curve's pull, the shifts and their pause, coasting on IdleBrakeInput, braking, drag
//! and less steering at speed. Without engine inertia the revs drop with the ratio on an upshift, below the next
//! gear's down point (the Chevette's 0.85 up, 0.5 down): so it shifts down only off the throttle, or near stalling.

/// Unreal's defaults where the Blueprint doesn't set them.
pub const DEFAULT_MASS_KG: f32 = 1500.0;
pub const DEFAULT_FINAL_RATIO: f32 = 4.0;
pub const DEFAULT_REVERSE_RATIO: f32 = 4.0;
pub const DEFAULT_BRAKE_TORQUE: f32 = 1500.0;
pub const DEFAULT_DRAG: f32 = 3.0;
pub const DEFAULT_CHASSIS_CM: (f32, f32) = (180.0, 140.0);
/// UWheeledVehicleMovementComponent4W's SteeringCurve: (km/h, scale).
pub const STEERING_CURVE: [(f32, f32); 3] = [(0.0, 1.0), (40.0, 0.7), (120.0, 0.6)];
/// The share of the weight on the driven front axle (a front-engined car).
pub const FRONT_LOAD: f32 = 0.6;
pub const GRAVITY: f32 = 9.81;
const AIR_DENSITY: f32 = 1.225;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Gear {
    pub ratio: f32,
    /// Shift down below this share of MaxRPM, up above this one.
    pub down: f32,
    pub up: f32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct VehicleSetup {
    /// (RPM, N·m), linear between keys.
    pub torque_curve: Vec<(f32, f32)>,
    pub max_rpm: f32,
    pub gears: Vec<Gear>,
    pub final_ratio: f32,
    pub reverse_ratio: f32,
    /// Seconds without drive while shifting (GearAutoBoxLatency).
    pub shift_latency: f32,
    pub mass_kg: f32,
    pub wheel_radius_cm: f32,
    /// The front wheels' SteerAngle (degrees).
    pub steer_deg: f32,
    pub brake_torque: f32,
    /// IdleBrakeInput: the brake applied with no throttle.
    pub idle_brake: f32,
    pub drag: f32,
    pub chassis_cm: (f32, f32),
}

impl Default for VehicleSetup {
    fn default() -> Self {
        // The Chevette's.
        VehicleSetup {
            torque_curve: vec![(0.0, 400.0), (1890.0, 500.0), (5729.0, 400.0)],
            max_rpm: 2500.0,
            gears: vec![Gear { ratio: 4.0, down: 0.5, up: 0.85 }, Gear { ratio: 2.0, down: 0.5, up: 0.65 }],
            final_ratio: DEFAULT_FINAL_RATIO,
            reverse_ratio: DEFAULT_REVERSE_RATIO,
            shift_latency: 1.0,
            mass_kg: DEFAULT_MASS_KG,
            wheel_radius_cm: 35.0,
            steer_deg: 35.0,
            brake_torque: DEFAULT_BRAKE_TORQUE,
            idle_brake: 0.1,
            drag: DEFAULT_DRAG,
            chassis_cm: DEFAULT_CHASSIS_CM,
        }
    }
}

impl VehicleSetup {
    pub fn torque_at(&self, rpm: f32) -> f32 {
        let c = &self.torque_curve;
        let Some(first) = c.first() else { return 0.0 };
        if rpm <= first.0 {
            return first.1;
        }
        for w in c.windows(2) {
            if rpm <= w[1].0 {
                let t = (rpm - w[0].0) / (w[1].0 - w[0].0).max(1e-3);
                return w[0].1 + (w[1].1 - w[0].1) * t;
            }
        }
        c.last().map_or(0.0, |k| k.1)
    }

    /// The top speed in a gear (cm/s): the wheels at MaxRPM through the ratios.
    pub fn top_speed(&self, ratio: f32) -> f32 {
        let wheel_rad_s = self.max_rpm / (ratio * self.final_ratio) * std::f32::consts::TAU / 60.0;
        wheel_rad_s * self.wheel_radius_cm
    }

    /// The steering angle (degrees) for full lock at a speed (cm/s): SteerAngle times the steering curve.
    pub fn steer_at(&self, speed_cm_s: f32) -> f32 {
        let kmh = speed_cm_s.abs() * 0.036;
        let c = &STEERING_CURVE;
        let scale = if kmh >= c[2].0 {
            c[2].1
        } else if kmh >= c[1].0 {
            c[1].1 + (c[2].1 - c[1].1) * (kmh - c[1].0) / (c[2].0 - c[1].0)
        } else {
            c[0].1 + (c[1].1 - c[0].1) * kmh / c[1].0
        };
        self.steer_deg * scale
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct VehicleState {
    /// Along the car's heading, cm/s.
    pub speed: f32,
    /// The forward gear (0 first); reverse is speed below zero with throttle back.
    pub gear: usize,
    /// Seconds left of a shift.
    pub shifting: f32,
    pub rpm: f32,
}

impl VehicleState {
    /// One step: `throttle` -1..1 (forward, back; against the motion it brakes).
    pub fn step(&mut self, s: &VehicleSetup, throttle: f32, dt: f32) {
        let r = s.wheel_radius_cm / 100.0;
        let v = self.speed / 100.0;
        let reversing = v < -0.01 || (v.abs() <= 0.01 && throttle < 0.0);
        let ratio = if reversing { s.reverse_ratio } else { s.gears.get(self.gear).map_or(1.0, |g| g.ratio) };
        let wheel_rpm = v.abs() / r * 60.0 / std::f32::consts::TAU;
        self.rpm = (wheel_rpm * ratio * s.final_ratio).min(s.max_rpm);
        // The automatic gearbox.
        self.shifting = (self.shifting - dt).max(0.0);
        if !reversing && self.shifting == 0.0 {
            if let Some(g) = s.gears.get(self.gear) {
                if self.rpm > g.up * s.max_rpm && self.gear + 1 < s.gears.len() {
                    self.gear += 1;
                    self.shifting = s.shift_latency;
                } else if self.gear > 0 && (self.rpm < 0.2 * s.max_rpm || (throttle <= 0.0 && self.rpm < g.down * s.max_rpm)) {
                    self.gear -= 1;
                    self.shifting = s.shift_latency;
                }
            }
        }
        let grip_front = FRONT_LOAD * s.mass_kg * GRAVITY;
        let grip_all = s.mass_kg * GRAVITY;
        let mut force = 0.0;
        let along = if v.abs() > 0.01 { v.signum() } else { throttle.signum() };
        if throttle != 0.0 && (v.abs() <= 0.01 || throttle.signum() == v.signum()) {
            // Drive: the curve's torque through the gear, none at the limiter or mid-shift.
            if self.shifting == 0.0 && self.rpm < s.max_rpm {
                let wheel_torque = s.torque_at(self.rpm) * throttle.abs() * ratio * s.final_ratio;
                force += along * (wheel_torque / r).min(grip_front);
            }
        } else {
            // Brakes: the throttle against the motion, else the idle brake.
            let input = if throttle != 0.0 { throttle.abs() } else { s.idle_brake };
            let brake = (input * s.brake_torque * 4.0 / r).min(grip_all);
            let stop = v.abs() * s.mass_kg / dt.max(1e-4);
            force -= v.signum() * brake.min(stop);
        }
        // Air drag: ½ ρ Cd A v².
        let area = s.chassis_cm.0 * s.chassis_cm.1 / 10_000.0;
        force -= v.signum() * 0.5 * AIR_DENSITY * s.drag * area * v * v;
        let new_v = v + force / s.mass_kg * dt;
        // Braking doesn't reverse the car.
        self.speed = if v != 0.0 && new_v.signum() != v.signum() && throttle.signum() != new_v.signum() { 0.0 } else { new_v * 100.0 };
        // The limiter: no faster than the gear allows.
        let top = s.top_speed(ratio);
        self.speed = self.speed.clamp(-s.top_speed(s.reverse_ratio), top.max(s.top_speed(s.gears.last().map_or(1.0, |g| g.ratio))));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(s: &VehicleSetup, state: &mut VehicleState, throttle: f32, secs: f32) {
        for _ in 0..(secs * 60.0) as usize {
            state.step(s, throttle, 1.0 / 60.0);
        }
    }

    #[test]
    fn chevette_accelerates_shifts_and_tops_out() {
        let s = VehicleSetup::default();
        // First gear tops out at 2500 / 16 wheel rpm on a 35 cm wheel: 5.7 m/s; second at 11.5 m/s.
        assert!((s.top_speed(4.0) - 572.6).abs() < 1.0);
        assert!((s.top_speed(2.0) - 1145.3).abs() < 2.0);
        let mut st = VehicleState::default();
        run(&s, &mut st, 1.0, 1.0);
        // Grip-limited launch (0.6 g on the front axle).
        assert!(st.speed > 400.0 && st.speed < 600.0, "{}", st.speed);
        run(&s, &mut st, 1.0, 10.0);
        assert_eq!(st.gear, 1);
        assert!(st.speed > 1050.0 && st.speed <= 1146.0, "{}", st.speed);
    }

    #[test]
    fn coasts_brakes_and_reverses() {
        let s = VehicleSetup::default();
        let mut st = VehicleState { speed: 1000.0, gear: 1, ..Default::default() };
        // The idle brake: 0.1 × 4 × 1500 N·m on 35 cm wheels, about 1.1 m/s² (and drag).
        run(&s, &mut st, 0.0, 1.0);
        assert!(st.speed > 850.0 && st.speed < 900.0, "{}", st.speed);
        // Full brakes (about 1 g, grip-limited) stop it within a second; held, it then backs up.
        let mut t = 0.0;
        while st.speed > 0.0 && t < 3.0 {
            st.step(&s, -1.0, 1.0 / 60.0);
            t += 1.0 / 60.0;
        }
        assert!(t > 0.7 && t < 1.2, "{t}");
        // Back: reverse tops out like first gear.
        run(&s, &mut st, -1.0, 5.0);
        assert!(st.speed < -500.0 && st.speed >= -573.0, "{}", st.speed);
    }

    #[test]
    fn torque_curve_and_steering() {
        let s = VehicleSetup::default();
        assert_eq!(s.torque_at(0.0), 400.0);
        assert_eq!(s.torque_at(1890.0), 500.0);
        assert!((s.torque_at(945.0) - 450.0).abs() < 1e-3);
        assert_eq!(s.steer_at(0.0), 35.0);
        // 40 km/h: 70% of the lock.
        assert!((s.steer_at(1111.1) - 24.5).abs() < 0.01);
    }
}
