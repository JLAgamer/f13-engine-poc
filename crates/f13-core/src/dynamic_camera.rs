//! A character's cameras, read from the decomp (MODLOG "Counselor cameras"), in Unreal space: cm, Z up, rotators as
//! (pitch, yaw, roll) degrees.
//!
//! - [`DynamicCamera`]: ILLDynamicCameraComponent. Its MovementBasedSettings (FCameraBehaviorSettings) are picked by
//!   the owner's speed and blended toward over the picked one's BlendInDelay and BlendInTime (TickComponent
//!   0x140244cb0, the blend FUN_140230fd0), then written onto its spring arm and camera (FUN_140216d70).
//! - [`SpringArm`]: USpringArmComponent::UpdateDesiredArmLocation as this build has it (0x141457570): rotation lag by
//!   RInterpTo, location lag by VInterpTo, both substepped by CameraLagMaxTimeStep; the probe sweep from the arm's
//!   origin.
//! - [`CameraStack`]: ILLCharacter's camera stack: SetActiveCamera (0x140280ae0), SetActiveCameraWithBlend
//!   (0x140280d70) and its CalcCamera (FUN_14025a820), which weights the stacked cameras' views.

use crate::camera_spline::{quaternion, rotator};
use glam::Vec3;

/// FMath's KINDA_SMALL_NUMBER.
const KINDA_SMALL: f32 = 1e-4;

/// EViewTargetBlendFunction.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum BlendFunc {
    Linear,
    Cubic,
    EaseIn,
    EaseOut,
    #[default]
    EaseInOut,
}

impl BlendFunc {
    /// From its Unreal name ("VTBlend_EaseInOut").
    pub fn from_name(name: &str) -> Self {
        match name.trim_start_matches("VTBlend_") {
            "Linear" => Self::Linear,
            "Cubic" => Self::Cubic,
            "EaseIn" => Self::EaseIn,
            "EaseOut" => Self::EaseOut,
            _ => Self::EaseInOut,
        }
    }

    /// FUN_140230fd0's eased alpha (for 0 < `a` < 1).
    pub fn apply(self, a: f32, exponent: f32) -> f32 {
        match self {
            Self::Linear => a,
            Self::Cubic => a * a * 3.0 - 2.0 * a * a * a,
            Self::EaseIn => a.powf(exponent),
            Self::EaseOut => a.powf(1.0 / exponent),
            Self::EaseInOut if a < 0.5 => (a + a).powf(exponent) * 0.5,
            Self::EaseInOut => (1.0 - (1.0 - (a + a - 1.0)).powf(exponent) + 1.0) * 0.5,
        }
    }
}

/// FCameraBehaviorSettings: what a dynamic camera sets on its spring arm and camera. Defaults: its constructor
/// (inlined in ILLDynamicCameraComponent's, 0x14020f7d0).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Settings {
    pub minimum_character_speed: f32,
    pub blend_exponent: f32,
    pub blend_func: BlendFunc,
    pub blend_in_delay: f32,
    pub blend_in_time: f32,
    pub target_arm_length: f32,
    pub socket_offset: Vec3,
    pub target_offset: Vec3,
    pub use_pawn_control_rotation: bool,
    pub inherit_pitch: bool,
    pub inherit_yaw: bool,
    pub inherit_roll: bool,
    pub enable_camera_lag: bool,
    pub enable_camera_rotation_lag: bool,
    pub use_camera_lag_substepping: bool,
    pub camera_lag_speed: f32,
    pub camera_rotation_lag_speed: f32,
    pub camera_lag_max_time_step: f32,
    pub camera_lag_max_distance: f32,
    /// Horizontal degrees.
    pub field_of_view: f32,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            minimum_character_speed: 0.0,
            blend_exponent: 2.0,
            blend_func: BlendFunc::EaseInOut,
            blend_in_delay: 1.0,
            blend_in_time: 1.0,
            target_arm_length: 300.0,
            socket_offset: Vec3::ZERO,
            target_offset: Vec3::ZERO,
            use_pawn_control_rotation: true,
            inherit_pitch: true,
            inherit_yaw: true,
            inherit_roll: false,
            enable_camera_lag: true,
            enable_camera_rotation_lag: true,
            use_camera_lag_substepping: true,
            camera_lag_speed: 10.0,
            camera_rotation_lag_speed: 10.0,
            camera_lag_max_time_step: 1.0 / 60.0,
            camera_lag_max_distance: 0.0,
            field_of_view: 90.0,
        }
    }
}

impl Settings {
    /// FUN_140230fd0: `alpha` of the way from `self` to `to` through `to`'s blend function. The flags and blend
    /// parameters are `to`'s; the arm, offsets, lag speeds and field of view lerp. Outside (0, 1) it's either end.
    pub fn blend(&self, to: &Settings, alpha: f32) -> Settings {
        if alpha < 0.0 || alpha == 0.0 {
            return *self;
        }
        if alpha >= 1.0 {
            return *to;
        }
        let a = to.blend_func.apply(alpha, to.blend_exponent);
        let f = |x: f32, y: f32| (y - x) * a + x;
        Settings {
            target_arm_length: f(self.target_arm_length, to.target_arm_length),
            socket_offset: (to.socket_offset - self.socket_offset) * a + self.socket_offset,
            target_offset: (to.target_offset - self.target_offset) * a + self.target_offset,
            camera_lag_speed: f(self.camera_lag_speed, to.camera_lag_speed),
            camera_rotation_lag_speed: f(self.camera_rotation_lag_speed, to.camera_rotation_lag_speed),
            camera_lag_max_time_step: f(self.camera_lag_max_time_step, to.camera_lag_max_time_step),
            camera_lag_max_distance: f(self.camera_lag_max_distance, to.camera_lag_max_distance),
            field_of_view: f(self.field_of_view, to.field_of_view),
            ..*to
        }
    }
}

/// How far a blend that started at `start` is at `now`: (now - start - delay) / time, clamped to [0, 1]; a NaN (a zero
/// time at its first instant) counts as 0, as the game's clamp does.
fn progress(now: f32, start: f32, delay: f32, time: f32) -> f32 {
    let a = (now - start - delay) / time;
    if a >= 0.0 { a.min(1.0) } else { 0.0 }
}

/// ILLDynamicCameraComponent.
#[derive(Clone, Debug)]
pub struct DynamicCamera {
    pub settings: Vec<Settings>,
    /// The owner's speed without its vertical part (constructor: true).
    pub use_2d_velocity: bool,
    /// The blend's base: where the last blend stood when the settings last changed.
    current: Settings,
    /// The picked setting and when it was picked.
    index: usize,
    start: f32,
    pub arm: SpringArm,
}

impl DynamicCamera {
    /// As registered: the base is the first setting (FUN_14022e280).
    pub fn new(settings: Vec<Settings>, use_2d_velocity: bool, arm: SpringArm) -> Self {
        let current = settings.first().copied().unwrap_or_default();
        DynamicCamera { settings, use_2d_velocity, current, index: 0, start: 0.0, arm }
    }

    /// TickComponent (0x140244cb0): the last setting whose MinimumCharacterSpeed the owner's speed reaches; when that
    /// changes, the blend toward the old one freezes as the new base. Returns what goes onto the arm and camera. (Its
    /// SpringArmAdjustPitchCurve / SpeedCurve would scale the arm length; no counselor camera sets them.)
    pub fn tick(&mut self, now: f32, velocity: Vec3) -> Settings {
        if self.settings.is_empty() {
            return self.current;
        }
        let v = if self.use_2d_velocity { velocity.truncate().length() } else { velocity.length() };
        let reached = self.settings.iter().take_while(|s| v >= s.minimum_character_speed).count();
        let index = reached.saturating_sub(1);
        if index != self.index {
            let old = self.settings[self.index];
            self.current = self.current.blend(&old, progress(now, self.start, old.blend_in_delay, old.blend_in_time));
            self.start = now;
            self.index = index;
        }
        let to = self.settings[self.index];
        let s = self.current.blend(&to, progress(now, self.start, to.blend_in_delay, to.blend_in_time));
        self.arm.settings = s;
        s
    }
}

/// FRotator::GetNormalized: each axis in (-180, 180].
pub fn normalize(r: Vec3) -> Vec3 {
    let n = |a: f32| {
        let a = a.rem_euclid(360.0);
        if a > 180.0 { a - 360.0 } else { a }
    };
    Vec3::new(n(r.x), n(r.y), n(r.z))
}

/// FMath::RInterpTo.
pub fn rinterp_to(current: Vec3, target: Vec3, dt: f32, speed: f32) -> Vec3 {
    if dt == 0.0 || current == target {
        return current;
    }
    if speed <= 0.0 {
        return target;
    }
    let delta = normalize(target - current);
    if delta.abs().max_element() <= KINDA_SMALL {
        return target;
    }
    normalize(current + delta * (speed * dt).clamp(0.0, 1.0))
}

/// FMath::VInterpTo.
pub fn vinterp_to(current: Vec3, target: Vec3, dt: f32, speed: f32) -> Vec3 {
    if speed <= 0.0 {
        return target;
    }
    let dist = target - current;
    if dist.length_squared() < KINDA_SMALL {
        return target;
    }
    current + dist * (dt * speed).clamp(0.0, 1.0)
}

/// FRotator::Vector: the direction a rotator faces.
pub fn direction(r: Vec3) -> Vec3 {
    let (sp, cp) = r.x.to_radians().sin_cos();
    let (sy, cy) = r.y.to_radians().sin_cos();
    Vec3::new(cp * cy, cp * sy, sp)
}

/// USpringArmComponent.
#[derive(Clone, Debug)]
pub struct SpringArm {
    /// bDoCollisionTest (constructor: true).
    pub do_collision_test: bool,
    /// The probe sphere's radius (constructor: 12).
    pub probe_size: f32,
    /// What its dynamic camera last set.
    pub settings: Settings,
    previous_desired_rot: Vec3,
    previous_desired_loc: Vec3,
    /// The camera's world location and rotation (the arm's socket).
    pub socket: (Vec3, Vec3),
}

impl Default for SpringArm {
    fn default() -> Self {
        SpringArm {
            do_collision_test: true,
            probe_size: 12.0,
            settings: Settings::default(),
            previous_desired_rot: Vec3::ZERO,
            previous_desired_loc: Vec3::ZERO,
            socket: (Vec3::ZERO, Vec3::ZERO),
        }
    }
}

impl SpringArm {
    pub fn new(probe_size: f32, do_collision_test: bool) -> Self {
        SpringArm { probe_size, do_collision_test, ..Default::default() }
    }

    /// TickComponent: UpdateDesiredArmLocation(bDoCollisionTest, bEnableCameraLag, bEnableCameraRotationLag, dt).
    pub fn tick(&mut self, origin: Vec3, component_rot: Vec3, control_rot: Vec3, dt: f32, sweep: impl FnOnce(Vec3, Vec3, f32) -> Option<Vec3>) {
        let s = self.settings;
        self.update(origin, component_rot, control_rot, self.do_collision_test, s.enable_camera_lag, s.enable_camera_rotation_lag, dt, sweep);
    }

    /// The dynamic camera's Activate (0x140216330): the arm snaps (no trace, no lag, no time).
    pub fn snap(&mut self, origin: Vec3, component_rot: Vec3, control_rot: Vec3) {
        self.update(origin, component_rot, control_rot, false, false, false, 0.0, |_, _, _| None);
    }

    /// UpdateDesiredArmLocation (0x141457570). `origin` is the arm component's location (the owner's capsule centre),
    /// `sweep(from, to, radius)` the first blocking hit's sphere centre on the camera channel.
    #[allow(clippy::too_many_arguments)]
    pub fn update(
        &mut self,
        origin: Vec3,
        component_rot: Vec3,
        control_rot: Vec3,
        do_trace: bool,
        location_lag: bool,
        rotation_lag: bool,
        dt: f32,
        sweep: impl FnOnce(Vec3, Vec3, f32) -> Option<Vec3>,
    ) {
        let s = self.settings;
        // GetTargetRotation: the pawn's view rotation; what it doesn't inherit is the arm's relative rotation (0).
        let mut desired_rot = if s.use_pawn_control_rotation { control_rot } else { component_rot };
        if !s.inherit_pitch {
            desired_rot.x = 0.0;
        }
        if !s.inherit_yaw {
            desired_rot.y = 0.0;
        }
        if !s.inherit_roll {
            desired_rot.z = 0.0;
        }
        let step = s.camera_lag_max_time_step;
        if rotation_lag {
            if s.use_camera_lag_substepping && dt > step && s.camera_rotation_lag_speed > 0.0 {
                let arm_rot_step = normalize(desired_rot - self.previous_desired_rot) * (1.0 / dt);
                let mut lerp_target = self.previous_desired_rot;
                let mut remaining = dt;
                while remaining > KINDA_SMALL {
                    let amount = step.min(remaining);
                    lerp_target += arm_rot_step * amount;
                    remaining -= amount;
                    desired_rot = rinterp_to(self.previous_desired_rot, lerp_target, amount, s.camera_rotation_lag_speed);
                    self.previous_desired_rot = desired_rot;
                }
            } else {
                desired_rot = rinterp_to(self.previous_desired_rot, desired_rot, dt, s.camera_rotation_lag_speed);
            }
        }
        self.previous_desired_rot = desired_rot;

        let arm_origin = origin + s.target_offset;
        let mut desired_loc = arm_origin;
        if location_lag {
            if s.use_camera_lag_substepping && dt > step && s.camera_lag_speed > 0.0 {
                let arm_step = (desired_loc - self.previous_desired_loc) * (1.0 / dt);
                let mut lerp_target = self.previous_desired_loc;
                let mut remaining = dt;
                while remaining > KINDA_SMALL {
                    let amount = step.min(remaining);
                    lerp_target += arm_step * amount;
                    remaining -= amount;
                    desired_loc = vinterp_to(self.previous_desired_loc, lerp_target, amount, s.camera_lag_speed);
                    self.previous_desired_loc = desired_loc;
                }
            } else {
                desired_loc = vinterp_to(self.previous_desired_loc, desired_loc, dt, s.camera_lag_speed);
            }
            if s.camera_lag_max_distance > 0.0 {
                let from_origin = desired_loc - arm_origin;
                if from_origin.length_squared() > s.camera_lag_max_distance * s.camera_lag_max_distance {
                    desired_loc = arm_origin + from_origin.clamp_length_max(s.camera_lag_max_distance);
                }
            }
        }
        self.previous_desired_loc = desired_loc;

        desired_loc -= direction(desired_rot) * s.target_arm_length;
        desired_loc += quaternion(desired_rot) * s.socket_offset;
        let result = if do_trace && s.target_arm_length != 0.0 { sweep(arm_origin, desired_loc, self.probe_size).unwrap_or(desired_loc) } else { desired_loc };
        self.socket = (result, normalize(desired_rot));
    }
}

/// A camera's view (FMinimalViewInfo's parts the stack blends): location, rotation, horizontal field of view.
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct View {
    pub location: Vec3,
    pub rotation: Vec3,
    pub fov: f32,
}

/// A stacked camera (FILLCameraBlendInfo): when its blend started, over how long, its progress (D), weight (W) and
/// normalised weight.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Entry {
    pub camera: usize,
    pub start: f32,
    pub length: f32,
    pub progress: f32,
    pub weight: f32,
    pub normalized: f32,
}

/// What the stack asks of its cameras: (re)activate one (its arm snaps), or deactivate one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CameraEvent {
    Activate(usize),
    Deactivate(usize),
}

/// ILLCharacter's camera stack, the newest camera last.
#[derive(Clone, Debug, Default)]
pub struct CameraStack {
    pub entries: Vec<Entry>,
}

impl CameraStack {
    /// The camera the stack is blending to.
    pub fn top(&self) -> Option<usize> {
        self.entries.last().map(|e| e.camera)
    }

    /// SetActiveCamera: unless it's already the first, the stack empties (its cameras deactivated) and holds only
    /// `camera`, fully.
    pub fn set_active(&mut self, camera: usize, now: f32, events: &mut Vec<CameraEvent>) {
        if self.entries.first().is_some_and(|e| e.camera == camera) {
            return;
        }
        events.extend(self.entries.drain(..).map(|e| CameraEvent::Deactivate(e.camera)));
        events.push(CameraEvent::Activate(camera));
        self.entries.push(Entry { camera, start: now, length: 0.0, progress: 1.0, weight: 1.0, normalized: 1.0 });
    }

    /// SetActiveCameraWithBlend: `camera` (re)activates and blends in over `time`. Already stacked, the blends back to
    /// it run in reverse: the newest camera stays full, then each one above it again, back down to it, each from
    /// where its blend had got to.
    pub fn set_active_with_blend(&mut self, camera: usize, time: f32, now: f32, events: &mut Vec<CameraEvent>) {
        if self.entries.len() == 1 && self.entries[0].camera == camera {
            return;
        }
        if time <= 0.0 || self.entries.is_empty() {
            self.set_active(camera, now, events);
            return;
        }
        events.push(CameraEvent::Activate(camera));
        let Some(k) = self.entries.iter().position(|e| e.camera == camera) else {
            self.entries.push(Entry { camera, start: now, length: time, progress: 0.0, weight: 0.0, normalized: 0.0 });
            return;
        };
        let old = std::mem::take(&mut self.entries);
        let newest = old[old.len() - 1].camera;
        self.entries.extend_from_slice(&old[..k]);
        self.entries.push(Entry { camera: newest, start: 0.0, length: 0.0, progress: 1.0, weight: 1.0, normalized: 0.0 });
        for j in (k..old.len() - 1).rev() {
            let next = old[j + 1];
            self.entries.push(Entry {
                camera: old[j].camera,
                start: now - (1.0 - next.progress) * next.length,
                length: next.length,
                progress: 0.0,
                weight: 0.0,
                normalized: 0.0,
            });
        }
    }

    /// CalcCamera's weighting: each camera's progress; the newest weighs its progress, each older one its progress
    /// less the next one's, and one that has blended out (progress over 0.1, weight 0.01 or less) leaves the stack.
    /// Weights are normalised (at least 0.0001). Cameras `active` says are off leave first.
    pub fn update(&mut self, now: f32, active: impl Fn(usize) -> bool, events: &mut Vec<CameraEvent>) {
        self.entries.retain(|e| active(e.camera));
        if self.entries.len() == 1 {
            let e = &mut self.entries[0];
            (e.progress, e.weight, e.normalized) = (1.0, 1.0, 1.0);
            return;
        }
        let mut sum = 0.0;
        let mut i = 0;
        while i < self.entries.len() {
            let d = progress(now, self.entries[i].start, 0.0, self.entries[i].length);
            self.entries[i].progress = d;
            if i == self.entries.len() - 1 {
                self.entries[i].weight = d;
                sum += d;
            }
            if i > 0 {
                let prev = self.entries[i - 1].progress;
                let w = prev - d;
                self.entries[i - 1].weight = w;
                if prev <= 0.1 || w > 0.01 {
                    sum += w;
                } else {
                    events.push(CameraEvent::Deactivate(self.entries[i - 1].camera));
                    self.entries.remove(i - 1);
                    i -= 1;
                }
            }
            i += 1;
        }
        for e in &mut self.entries {
            e.normalized = ((1.0 / sum) * e.weight).max(0.0001);
        }
        if self.entries.len() == 1 {
            let e = &mut self.entries[0];
            (e.progress, e.weight, e.normalized) = (1.0, 1.0, 1.0);
        }
    }

    /// The blended view: the stacked cameras' views summed by normalised weight, component by component (rotators
    /// too, as the game sums them); one camera alone is its own view.
    pub fn view(&self, view_of: impl Fn(usize) -> View) -> Option<View> {
        match self.entries.as_slice() {
            [] => None,
            [only] => Some(view_of(only.camera)),
            entries => Some(entries.iter().fold(View::default(), |acc, e| {
                let v = view_of(e.camera);
                View { location: acc.location + v.location * e.normalized, rotation: acc.rotation + v.rotation * e.normalized, fov: acc.fov + v.fov * e.normalized }
            })),
        }
    }
}

/// The rotator of an Unreal quaternion (re-exported for callers converting views).
pub fn rotator_of(q: glam::Quat) -> Vec3 {
    rotator(q)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn outdoor() -> Vec<Settings> {
        let s = |min: f32, func: BlendFunc, exp: f32, time: f32, arm: f32, fov: f32| Settings {
            minimum_character_speed: min,
            blend_func: func,
            blend_exponent: exp,
            blend_in_delay: 0.0,
            blend_in_time: time,
            target_arm_length: arm,
            field_of_view: fov,
            ..Settings::default()
        };
        vec![
            s(0.0, BlendFunc::EaseOut, 1.0, 2.5, 180.0, 65.0),
            s(1.0, BlendFunc::EaseInOut, 1.0, 3.0, 150.0, 75.0),
            s(280.0, BlendFunc::EaseInOut, 2.0, 2.0, 205.0, 77.5),
            s(450.0, BlendFunc::EaseInOut, 2.0, 1.5, 215.0, 80.0),
        ]
    }

    #[test]
    fn eases_match_the_blend_functions() {
        assert_eq!(BlendFunc::Cubic.apply(0.5, 2.0), 0.5);
        assert!((BlendFunc::EaseInOut.apply(0.25, 2.0) - 0.125).abs() < 1e-6);
        assert!((BlendFunc::EaseInOut.apply(0.75, 2.0) - 0.875).abs() < 1e-6);
        assert!((BlendFunc::EaseOut.apply(0.25, 2.0) - 0.5).abs() < 1e-6);
    }

    #[test]
    fn speed_picks_the_setting_and_blends_toward_it() {
        let mut cam = DynamicCamera::new(outdoor(), true, SpringArm::default());
        assert_eq!(cam.tick(0.0, Vec3::ZERO).target_arm_length, 180.0);
        // Running (over 280 cm/s): toward the third over its 2 s, eased in and out.
        let s = cam.tick(10.0, Vec3::new(300.0, 0.0, 0.0));
        assert_eq!(s.target_arm_length, 180.0);
        let s = cam.tick(11.0, Vec3::new(300.0, 0.0, 0.0));
        assert!((s.target_arm_length - (180.0 + 25.0 * 0.5)).abs() < 1e-3, "{}", s.target_arm_length);
        assert_eq!(cam.tick(12.0, Vec3::new(300.0, 0.0, 0.0)).field_of_view, 77.5);
        // Falling straight down doesn't count (2D velocity).
        assert_eq!(cam.tick(13.0, Vec3::new(300.0, 0.0, -900.0)).target_arm_length, 205.0);
    }

    #[test]
    fn a_change_mid_blend_continues_from_where_it_was() {
        let mut cam = DynamicCamera::new(outdoor(), true, SpringArm::default());
        cam.tick(0.0, Vec3::ZERO);
        cam.tick(10.0, Vec3::new(300.0, 0.0, 0.0));
        // Halfway to 205, she stops: the new base is 192.5, and the walk-speed setting blends from there.
        let s = cam.tick(11.0, Vec3::ZERO);
        assert!((s.target_arm_length - 192.5).abs() < 1e-3, "{}", s.target_arm_length);
    }

    #[test]
    fn the_arm_sits_behind_the_view_with_its_offsets() {
        let mut arm = SpringArm { settings: Settings { target_arm_length: 180.0, socket_offset: Vec3::new(0.0, 43.0, -34.0), target_offset: Vec3::new(0.0, 0.0, 64.0), ..Settings::default() }, ..SpringArm::default() };
        arm.snap(Vec3::new(100.0, 0.0, 88.0), Vec3::ZERO, Vec3::new(0.0, 90.0, 0.0));
        // Looking along +Y: 180 back along -Y, 43 to the right (-X), 34 down from 64 above the origin.
        let (loc, rot) = arm.socket;
        assert!((loc - Vec3::new(100.0 - 43.0, -180.0, 88.0 + 64.0 - 34.0)).length() < 1e-3, "{loc}");
        assert_eq!(rot, Vec3::new(0.0, 90.0, 0.0));
    }

    #[test]
    fn rotation_lag_follows_with_rinterp() {
        let mut arm = SpringArm::default();
        arm.snap(Vec3::ZERO, Vec3::ZERO, Vec3::ZERO);
        // 10/s for 1/60 s (no substeps at the max step): a sixth of the way.
        arm.update(Vec3::ZERO, Vec3::ZERO, Vec3::new(0.0, 60.0, 0.0), false, false, true, 1.0 / 60.0, |_, _, _| None);
        assert!((arm.socket.1.y - 10.0).abs() < 1e-3, "{}", arm.socket.1.y);
    }

    #[test]
    fn stack_blends_and_drops_the_old_camera() {
        let mut stack = CameraStack::default();
        let mut ev = Vec::new();
        stack.set_active(0, 0.0, &mut ev);
        stack.set_active_with_blend(1, 2.0, 1.0, &mut ev);
        assert_eq!(ev, vec![CameraEvent::Activate(0), CameraEvent::Activate(1)]);
        stack.update(2.0, |_| true, &mut ev);
        let n: Vec<f32> = stack.entries.iter().map(|e| e.normalized).collect();
        assert_eq!(n, vec![0.5, 0.5]);
        let view = stack.view(|c| View { location: Vec3::splat(c as f32 * 10.0), rotation: Vec3::ZERO, fov: 60.0 + c as f32 * 20.0 }).unwrap();
        assert_eq!((view.location.x, view.fov), (5.0, 70.0));
        ev.clear();
        stack.update(3.0, |_| true, &mut ev);
        assert_eq!(ev, vec![CameraEvent::Deactivate(0)]);
        assert_eq!(stack.entries.len(), 1);
    }

    #[test]
    fn switching_back_mid_blend_reverses_it() {
        let mut stack = CameraStack::default();
        let mut ev = Vec::new();
        stack.set_active(0, 0.0, &mut ev);
        stack.set_active_with_blend(1, 2.0, 0.0, &mut ev);
        stack.update(0.5, |_| true, &mut ev);
        // A quarter of the way to 1, back to 0: 1 is full and 0 comes back from three quarters.
        stack.set_active_with_blend(0, 0.5, 0.5, &mut ev);
        stack.update(0.5, |_| true, &mut ev);
        let w: Vec<(usize, f32)> = stack.entries.iter().map(|e| (e.camera, e.normalized)).collect();
        assert_eq!(w, vec![(1, 0.25), (0, 0.75)]);
    }
}
