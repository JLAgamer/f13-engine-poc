//! Kill cameras: SCSplineCamera riding a kill's SCCameraSplineComponent (decompiled). The spline's points carry, besides
//! position and rotation (in the kill actor's space, Unreal units), a hold time (the camera waits at the point), a
//! time to the next point, a field of view and a "scale" that is the handicam shake's strength (pitch, yaw, roll
//! degrees). Time walks the points: each holds for its HoldTime, then the segment to the next takes its TimeToNext
//! (at least [`MIN_SEGMENT`]), the input key moving evenly along it; every channel is evaluated at that key as an
//! Unreal InterpCurve. The spline's Duration is the sum of the holds and the segments; the camera's ActiveTime runs
//! from [`MIN_TIME`] to Duration - [`MIN_TIME`].
//!
//! The handicam adds sines to the pitch, yaw and roll: per axis a phase that advances at (1 + f) per second and a
//! sine of f × phase, where f is the axis' HandicamShakeFrequency scaled by 1 + rand × HandicamShakeFrequenceRandomizer
//! (drawn each tick).
//!
//! Not ported: the camera's trace pulling it in front of walls (and widening its field of view), ping-pong and loop
//! modes, blending back.

use glam::{Quat, Vec3};

/// The shortest segment, seconds (DAT_142108c44; a small epsilon).
pub const MIN_SEGMENT: f32 = 1e-4;
/// The camera's time is kept this far inside the spline's duration (DAT_14209aa54; a small epsilon).
pub const MIN_TIME: f32 = 1e-4;
/// A field of view curve's default (no points), degrees.
pub const DEFAULT_FOV: f32 = 90.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Interp {
    Linear,
    Constant,
    /// The Hermite modes (CurveAuto, CurveUser, CurveBreak, CurveAutoClamped): the points' tangents as stored.
    Curve,
}

impl Interp {
    /// From Unreal's EInterpCurveMode name ("CIM_Linear"...).
    pub fn from_name(s: &str) -> Interp {
        match s {
            "CIM_Linear" => Interp::Linear,
            "CIM_Constant" => Interp::Constant,
            _ => Interp::Curve,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Point<T> {
    pub key: f32,
    pub out: T,
    pub arrive: T,
    pub leave: T,
    pub mode: Interp,
}

/// Values a curve can interpolate.
pub trait Value: Copy {
    fn lerp(a: Self, b: Self, t: f32) -> Self;
    /// Cubic Hermite with tangents already scaled to the segment.
    fn hermite(p0: Self, m0: Self, p1: Self, m1: Self, t: f32) -> Self;
    fn scale(self, by: f32) -> Self;
}

fn hermite_weights(t: f32) -> [f32; 4] {
    let (t2, t3) = (t * t, t * t * t);
    // p0, m0, m1, p1
    [2.0 * t3 - 3.0 * t2 + 1.0, t3 - 2.0 * t2 + t, t3 - t2, 3.0 * t2 - 2.0 * t3]
}

impl Value for f32 {
    fn lerp(a: f32, b: f32, t: f32) -> f32 {
        a + (b - a) * t
    }
    fn hermite(p0: f32, m0: f32, p1: f32, m1: f32, t: f32) -> f32 {
        let [h00, h10, h11, h01] = hermite_weights(t);
        h00 * p0 + h10 * m0 + h11 * m1 + h01 * p1
    }
    fn scale(self, by: f32) -> f32 {
        self * by
    }
}

impl Value for Vec3 {
    fn lerp(a: Vec3, b: Vec3, t: f32) -> Vec3 {
        a.lerp(b, t)
    }
    fn hermite(p0: Vec3, m0: Vec3, p1: Vec3, m1: Vec3, t: f32) -> Vec3 {
        let [h00, h10, h11, h01] = hermite_weights(t);
        p0 * h00 + m0 * h10 + m1 * h11 + p1 * h01
    }
    fn scale(self, by: f32) -> Vec3 {
        self * by
    }
}

impl Value for Quat {
    /// FMath::Lerp on quaternions is Slerp.
    fn lerp(a: Quat, b: Quat, t: f32) -> Quat {
        a.slerp(b, t)
    }
    /// Unreal uses squad for curved quaternion keys; slerp here.
    fn hermite(p0: Quat, _m0: Quat, p1: Quat, _m1: Quat, t: f32) -> Quat {
        p0.slerp(p1, t)
    }
    fn scale(self, _by: f32) -> Quat {
        self
    }
}

/// An Unreal InterpCurve (not looped).
#[derive(Clone, Debug, PartialEq)]
pub struct Curve<T>(pub Vec<Point<T>>);

impl<T> Default for Curve<T> {
    fn default() -> Self {
        Curve(vec![])
    }
}

impl<T: Value> Curve<T> {
    /// FInterpCurve::Eval: `default` with no points; the first point's value before it, the last's after it.
    pub fn eval(&self, key: f32, default: T) -> T {
        let p = &self.0;
        let Some(first) = p.first() else { return default };
        if key < first.key {
            return first.out;
        }
        let i = p.iter().rposition(|q| q.key <= key).unwrap_or(0);
        if i == p.len() - 1 {
            return p[i].out;
        }
        let (a, b) = (&p[i], &p[i + 1]);
        let diff = b.key - a.key;
        if diff <= 0.0 || a.mode == Interp::Constant {
            return a.out;
        }
        let t = (key - a.key) / diff;
        match a.mode {
            Interp::Linear => T::lerp(a.out, b.out, t),
            _ => T::hermite(a.out, a.leave.scale(diff), b.out, b.arrive.scale(diff), t),
        }
    }

    /// Point `i`'s value (the last point past the end), `default` with no points.
    fn point(&self, i: usize, default: T) -> T {
        self.0.get(i.min(self.0.len().saturating_sub(1))).map_or(default, |p| p.out)
    }
}

/// A camera spline's channels (SCCameraSplineCurves).
#[derive(Clone, Debug, Default)]
pub struct CameraSpline {
    /// Unreal cm, the kill actor's space.
    pub position: Curve<Vec3>,
    /// Unreal (x, y, z, w), the kill actor's space.
    pub rotation: Curve<Quat>,
    /// The handicam's strength: pitch, yaw, roll degrees.
    pub scale: Curve<Vec3>,
    pub fov: Curve<f32>,
    pub hold: Curve<f32>,
    pub time_to_next: Curve<f32>,
}

/// Where the camera is at a time.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CameraPoint {
    /// Unreal cm.
    pub location: Vec3,
    /// Unreal (x, y, z, w).
    pub rotation: Quat,
    pub shake: Vec3,
    /// Horizontal, degrees.
    pub fov: f32,
}

impl CameraSpline {
    /// SCCameraSplineComponent's Duration: the points' holds plus the segments between them.
    pub fn duration(&self) -> f32 {
        let n = self.position.0.len();
        (0..n).map(|i| self.hold.point(i, 0.0)).sum::<f32>() + (0..n.saturating_sub(1)).map(|i| self.time_to_next.point(i, 1.0)).sum::<f32>()
    }

    /// The input key at `t` seconds (None at or before 0). Past the last point the walk carries on through its
    /// TimeToNext as the game's does (the curves hold at their last point).
    pub fn key_at(&self, mut t: f32) -> Option<f32> {
        let mut i = 0.0f32;
        while t > 0.0 {
            let segment = self.time_to_next.eval(i, 1.0).max(MIN_SEGMENT);
            t -= self.hold.eval(i, 0.0);
            if t <= 0.0 {
                return Some(i);
            }
            if t < segment {
                return Some(i + t / segment);
            }
            t -= segment;
            i += 1.0;
        }
        None
    }

    /// The camera at `t` seconds, kept inside the duration as the camera's ActiveTime is.
    pub fn at(&self, t: f32) -> CameraPoint {
        let d = self.duration();
        let t = if t < MIN_TIME || t > d - MIN_TIME { if t < MIN_TIME { MIN_TIME } else { (d - MIN_TIME).max(MIN_TIME) } } else { t };
        match self.key_at(t) {
            Some(k) => CameraPoint {
                location: self.position.eval(k, Vec3::ZERO),
                rotation: self.rotation.eval(k, Quat::IDENTITY).normalize(),
                shake: self.scale.eval(k, Vec3::ONE),
                fov: self.fov.eval(k, DEFAULT_FOV),
            },
            None => CameraPoint { location: Vec3::ZERO, rotation: Quat::IDENTITY, shake: Vec3::ONE, fov: DEFAULT_FOV },
        }
    }
}

/// The handicam's phases (SCSplineCamera +0x88c..0x894).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Handicam {
    phase: Vec3,
}

impl Handicam {
    /// Advances by `dt` and returns the (pitch, yaw, roll) degrees to add: `frequency` HandicamShakeFrequency,
    /// `randomizer` HandicamShakeFrequenceRandomizer, `rand` in [0, 1], `strength` the point's scale.
    pub fn tick(&mut self, dt: f32, frequency: Vec3, randomizer: f32, rand: f32, strength: Vec3) -> Vec3 {
        let m = rand * randomizer + 1.0;
        let f = frequency * m;
        self.phase += (f + Vec3::ONE) * dt;
        let arg = f * self.phase;
        let wrap = |a: f32| (a % std::f32::consts::TAU).sin();
        Vec3::new(wrap(arg.x), wrap(arg.y), wrap(arg.z)) * strength
    }
}

/// Unreal's FRotator (pitch, yaw, roll degrees) from an Unreal quaternion (FQuat::Rotator).
pub fn rotator(q: Quat) -> Vec3 {
    let (x, y, z, w) = (q.x, q.y, q.z, q.w);
    let singularity = z * x - w * y;
    let yaw = (2.0 * (w * z + x * y)).atan2(1.0 - 2.0 * (y * y + z * z)).to_degrees();
    let pitch = (2.0 * singularity).clamp(-1.0, 1.0).asin().to_degrees();
    let roll = (-2.0 * (w * x + y * z)).atan2(1.0 - 2.0 * (x * x + y * y)).to_degrees();
    Vec3::new(pitch, yaw, roll)
}

/// An Unreal quaternion from FRotator (pitch, yaw, roll degrees) (FRotator::Quaternion).
pub fn quaternion(r: Vec3) -> Quat {
    let (sp, cp) = (r.x.to_radians() / 2.0).sin_cos();
    let (sy, cy) = (r.y.to_radians() / 2.0).sin_cos();
    let (sr, cr) = (r.z.to_radians() / 2.0).sin_cos();
    Quat::from_xyzw(cr * sp * sy - sr * cp * cy, -cr * sp * cy - sr * cp * sy, cr * cp * sy - sr * sp * cy, cr * cp * cy + sr * sp * sy)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p<T: Copy>(key: f32, out: T, tangent: T, mode: Interp) -> Point<T> {
        Point { key, out, arrive: tangent, leave: tangent, mode }
    }

    /// Pickaxe_ThroatHack_GrabKill's spline: two points, 12.4 s apart.
    fn throat_hack() -> CameraSpline {
        CameraSpline {
            position: Curve(vec![
                p(0.0, Vec3::new(64.41, 185.94, 85.47), Vec3::new(7.74, -1.46, 0.0), Interp::Curve),
                p(1.0, Vec3::new(154.12, 117.99, 79.53), Vec3::new(-15.11, -51.8, -12.12), Interp::Curve),
            ]),
            rotation: Curve(vec![
                p(0.0, Quat::from_xyzw(0.0, 0.0, -0.793354, 0.608761), Quat::IDENTITY, Interp::Linear),
                p(1.0, Quat::from_xyzw(0.0, 0.0, -0.976297, 0.216439), Quat::IDENTITY, Interp::Linear),
            ]),
            scale: Curve(vec![p(0.0, Vec3::new(0.8, 0.6, 0.2), Vec3::ZERO, Interp::Curve), p(1.0, Vec3::new(0.8, 0.6, 0.2), Vec3::ZERO, Interp::Curve)]),
            time_to_next: Curve(vec![p(0.0, 12.4, 0.0, Interp::Constant), p(1.0, 1.0, 0.0, Interp::Constant)]),
            ..Default::default()
        }
    }

    #[test]
    fn walks_holds_and_segments() {
        let s = throat_hack();
        assert!((s.duration() - 12.4).abs() < 1e-5);
        assert_eq!(s.key_at(0.0), None);
        assert!((s.key_at(6.2).unwrap() - 0.5).abs() < 1e-5);
        // Past the last point the walk goes on (the last point's segment); the curves hold there.
        assert!((s.key_at(13.0).unwrap() - 1.6).abs() < 1e-4);
        // Ends where the points are; FOV defaults to 90.
        let a = s.at(0.0);
        assert!((a.location - Vec3::new(64.41, 185.94, 85.47)).length() < 0.01);
        assert_eq!(a.fov, 90.0);
        let b = s.at(100.0);
        assert!((b.location - Vec3::new(154.12, 117.99, 79.53)).length() < 0.01);
        assert!(b.rotation.dot(Quat::from_xyzw(0.0, 0.0, -0.976297, 0.216439)).abs() > 0.9999);

        // A hold keeps the camera at its point.
        let mut held = throat_hack();
        held.hold = Curve(vec![p(0.0, 2.0, 0.0, Interp::Constant), p(1.0, 0.0, 0.0, Interp::Constant)]);
        assert!((held.duration() - 14.4).abs() < 1e-5);
        assert_eq!(held.key_at(1.5), Some(0.0));
        assert!((held.key_at(8.2).unwrap() - 0.5).abs() < 1e-5);
    }

    #[test]
    fn curves_evaluate_as_unreal() {
        let c = Curve(vec![p(0.0, 0.0f32, 0.0, Interp::Curve), p(2.0, 10.0, 0.0, Interp::Linear)]);
        // Flat tangents: smoothstep.
        assert!((c.eval(1.0, -1.0) - 5.0).abs() < 1e-5);
        assert!((c.eval(0.5, -1.0) - 10.0 * (3.0 * 0.0625 - 2.0 * 0.015625)).abs() < 1e-4);
        assert_eq!(c.eval(-1.0, -1.0), 0.0);
        assert_eq!(c.eval(5.0, -1.0), 10.0);
        assert_eq!(Curve::<f32>::default().eval(1.0, 7.0), 7.0);
        let k = Curve(vec![p(0.0, 1.0f32, 0.0, Interp::Constant), p(1.0, 3.0, 0.0, Interp::Constant)]);
        assert_eq!(k.eval(0.9, 0.0), 1.0);
        // Tangents are per key unit: a line with slope 10 over keys 0..2.
        let line = Curve(vec![p(0.0, 0.0f32, 10.0, Interp::Curve), p(2.0, 20.0, 10.0, Interp::Curve)]);
        assert!((line.eval(0.5, 0.0) - 5.0).abs() < 1e-4);
    }

    #[test]
    fn rotators_round_trip() {
        for r in [Vec3::new(10.0, 20.0, 30.0), Vec3::new(-45.0, 170.0, 5.0), Vec3::new(0.0, -104.8, 0.0)] {
            let back = rotator(quaternion(r));
            assert!((back - r).length() < 1e-3, "{r} -> {back}");
        }
        // Yaw about Unreal Z.
        let q = quaternion(Vec3::new(0.0, 90.0, 0.0));
        assert!((q * Vec3::X - Vec3::Y).length() < 1e-5);
    }

    #[test]
    fn handicam_shakes_within_strength() {
        let mut h = Handicam::default();
        let strength = Vec3::new(0.8, 0.6, 0.2);
        let mut seen = Vec3::ZERO;
        for _ in 0..600 {
            let s = h.tick(1.0 / 60.0, Vec3::new(1.0, 1.0, 0.5), 0.0, 0.5, strength);
            assert!(s.abs().cmple(strength + 1e-6).all());
            seen = seen.max(s.abs());
        }
        assert!(seen.x > 0.5 && seen.z > 0.1);
    }
}
