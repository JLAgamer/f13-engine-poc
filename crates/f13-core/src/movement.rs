//! Horizontal character velocity, matching Unreal 4.18 `UCharacterMovementComponent::CalcVelocity` and
//! `ApplyVelocityBraking` for walking (no root motion, no avoidance). Units are Unreal's (cm, cm/s).

/// Engine defaults the counselor Blueprint does not override, plus its `MaxAcceleration`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MovementSettings {
    pub max_acceleration: f32,
    pub ground_friction: f32,
    pub braking_deceleration_walking: f32,
    pub braking_friction_factor: f32,
    pub braking_sub_step_time: f32,
}

impl Default for MovementSettings {
    /// Unreal 4.18 `UCharacterMovementComponent` defaults.
    fn default() -> Self {
        Self {
            max_acceleration: 2048.0,
            ground_friction: 8.0,
            braking_deceleration_walking: 2048.0,
            braking_friction_factor: 2.0,
            braking_sub_step_time: 1.0 / 33.0,
        }
    }
}

const MIN_TICK_TIME: f32 = 1e-6;
const KINDA_SMALL_NUMBER: f32 = 1e-4;
const BRAKE_TO_STOP_VELOCITY: f32 = 10.0;

type V2 = [f32; 2];

fn len_sq(v: V2) -> f32 {
    v[0] * v[0] + v[1] * v[1]
}
fn dot(a: V2, b: V2) -> f32 {
    a[0] * b[0] + a[1] * b[1]
}
fn scale(v: V2, s: f32) -> V2 {
    [v[0] * s, v[1] * s]
}
fn safe_normal(v: V2) -> V2 {
    let l2 = len_sq(v);
    if l2 < 1e-8 {
        [0.0, 0.0]
    } else {
        scale(v, 1.0 / l2.sqrt())
    }
}

/// Converts a movement input (any length; clamped to 1) into an acceleration (cm/s²).
pub fn input_acceleration(input: V2, s: &MovementSettings) -> V2 {
    let l = len_sq(input).sqrt();
    let clamped = if l > 1.0 { scale(input, 1.0 / l) } else { input };
    scale(clamped, s.max_acceleration)
}

/// `ApplyVelocityBraking`: friction and deceleration while there is no input (or speed is over the max).
pub fn apply_braking(velocity: &mut V2, dt: f32, friction: f32, s: &MovementSettings) {
    if len_sq(*velocity) == 0.0 || dt < MIN_TICK_TIME {
        return;
    }
    let friction = (friction * s.braking_friction_factor.max(0.0)).max(0.0);
    let braking = s.braking_deceleration_walking.max(0.0);
    let zero_friction = friction == 0.0;
    let zero_braking = braking == 0.0;
    if zero_friction && zero_braking {
        return;
    }
    let old = *velocity;
    let mut remaining = dt;
    let max_step = s.braking_sub_step_time.clamp(1.0 / 75.0, 1.0 / 20.0);
    let rev_accel = if zero_braking { [0.0, 0.0] } else { scale(safe_normal(*velocity), -braking) };
    while remaining >= MIN_TICK_TIME {
        let step = if remaining > max_step && !zero_friction { max_step.min(remaining * 0.5) } else { remaining };
        remaining -= step;
        velocity[0] += (-friction * velocity[0] + rev_accel[0]) * step;
        velocity[1] += (-friction * velocity[1] + rev_accel[1]) * step;
        if dot(*velocity, old) <= 0.0 {
            *velocity = [0.0, 0.0];
            return;
        }
    }
    let v2 = len_sq(*velocity);
    if v2 <= KINDA_SMALL_NUMBER || (!zero_braking && v2 <= BRAKE_TO_STOP_VELOCITY * BRAKE_TO_STOP_VELOCITY) {
        *velocity = [0.0, 0.0];
    }
}

/// `CalcVelocity` for walking: updates `velocity` toward `acceleration` with friction, limited to `max_speed`.
pub fn calc_velocity(velocity: &mut V2, acceleration: V2, max_speed: f32, dt: f32, s: &MovementSettings) {
    if dt < MIN_TICK_TIME {
        return;
    }
    let friction = s.ground_friction.max(0.0);
    let zero_accel = len_sq(acceleration) == 0.0;
    let over_max = |v: V2| len_sq(v) > max_speed * max_speed * 1.01f32.powi(2);
    let velocity_over_max = over_max(*velocity);

    if zero_accel || velocity_over_max {
        let old = *velocity;
        apply_braking(velocity, dt, friction, s);
        // Don't let braking drop us below max speed while still accelerating along the old direction.
        if velocity_over_max && len_sq(*velocity) < max_speed * max_speed && dot(acceleration, old) > 0.0 {
            *velocity = scale(safe_normal(old), max_speed);
        }
    } else {
        let dir = safe_normal(acceleration);
        let speed = len_sq(*velocity).sqrt();
        let k = (dt * friction).min(1.0);
        velocity[0] -= (velocity[0] - dir[0] * speed) * k;
        velocity[1] -= (velocity[1] - dir[1] * speed) * k;
    }

    if !zero_accel {
        let new_max = if over_max(*velocity) { len_sq(*velocity).sqrt() } else { max_speed };
        velocity[0] += acceleration[0] * dt;
        velocity[1] += acceleration[1] * dt;
        let l = len_sq(*velocity).sqrt();
        if l > new_max && l > 0.0 {
            *velocity = scale(*velocity, new_max / l);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn settings() -> MovementSettings {
        MovementSettings { max_acceleration: 1000.0, ..Default::default() }
    }

    #[test]
    fn accelerates_to_and_holds_max_speed() {
        let s = settings();
        let mut v = [0.0, 0.0];
        let a = input_acceleration([1.0, 0.0], &s);
        for _ in 0..120 {
            calc_velocity(&mut v, a, 300.0, 1.0 / 60.0, &s);
        }
        assert!((v[0] - 300.0).abs() < 1e-3 && v[1] == 0.0);
    }

    #[test]
    fn friction_turns_velocity_toward_input() {
        let s = settings();
        let mut v = [300.0, 0.0];
        let a = input_acceleration([0.0, 1.0], &s);
        calc_velocity(&mut v, a, 300.0, 1.0 / 60.0, &s);
        assert!(v[0] < 300.0 * 0.9 && v[1] > 0.0);
    }

    #[test]
    fn brakes_to_a_stop_without_input() {
        let s = settings();
        let mut v = [300.0, 0.0];
        let mut t = 0.0;
        while v[0] > 0.0 {
            calc_velocity(&mut v, [0.0, 0.0], 300.0, 1.0 / 60.0, &s);
            t += 1.0 / 60.0;
            assert!(t < 1.0);
        }
        assert!(t > 0.05);
    }

    #[test]
    fn slowing_down_when_max_speed_drops() {
        let s = settings();
        let mut v = [450.0, 0.0];
        let a = input_acceleration([1.0, 0.0], &s);
        calc_velocity(&mut v, a, 280.0, 1.0 / 60.0, &s);
        assert!(v[0] < 450.0 && v[0] >= 280.0);
    }
}
