//! A rigid body's step as PhysX 3.4 takes it (PhysX3_x64.dll from the game install, read with its PDB), for the APEX
//! debris chunks (f13-game doors.rs). Metres and seconds (Unreal's centimetres / 100).
//!
//! - Dy::preIntegrationParallel (0x1801890f0): v += g dt; v *= max(0, 1 - dt linearDamping); w *= max(0, 1 - dt
//!   angularDamping); both clamped to their maximum speeds.
//! - Dy::setupFinalizeSolverConstraints (0x18007bf10), per contact (normal n from the world to the body, separation s
//!   less the rest offset): velMultiplier = 1 / (1/m + (r x n) . I^-1 (r x n)); vrel = n . (v + w x r); a bounce when
//!   the restitution is above 0, vrel < -bounceThreshold and s / dt < -vrel: the target velocity -restitution x vrel;
//!   bias = max(maxPenBias, 0.8 s / dt) x velMultiplier (none on a penetrating bounce); the velocity iterations' bias
//!   max(bias, 0) (none on a bounce).
//! - Dy::solveContact_BStatic (0x18009b2a0), against the static world, per patch: each normal impulse
//!   delta = max(velMultiplier (target - n . v_r) - bias, -applied); then the patch's friction rows (patch friction,
//!   PhysX's default): delta = -velMultiplier_t (t . v_r), the applied force clamped to staticFriction x the patch's
//!   normal impulse, and once beyond it the patch "broken" and held to dynamicFriction x that.
//! - Four position iterations (biased), the pose integrated with the velocity they leave, one velocity iteration
//!   (unbiased) for the velocity kept (PxRigidDynamic's default solver counts).
//! - Sleep: a mass-normalized kinetic energy below sleepThreshold for the wake counter's 0.4 s stops the body.
//!
//! PLACEHOLDER: the contact generation (UE sets bEnablePCM false: PhysX's legacy convex-mesh contacts aren't ported;
//! here the hull's vertices within the contact offset of a world triangle, at most four per patch of equal normals),
//! and the friction anchors (two per patch: its first contact and the one farthest from it).

use crate::collision::{Tri, TriSource};
use glam::{Mat3, Quat, Vec3};

/// The step's constants (UE's PhysicsSettings and PhysX's defaults, in metres).
#[derive(Clone, Copy, Debug)]
pub struct Params {
    pub gravity: Vec3,
    pub linear_damping: f32,
    pub angular_damping: f32,
    pub max_angular_velocity: f32,
    pub restitution: f32,
    pub static_friction: f32,
    pub dynamic_friction: f32,
    /// BounceThresholdVelocity (DefaultEngine.ini 200 cm/s).
    pub bounce_threshold: f32,
    /// The shapes' contact offset (PhysX's default 0.02 x the tolerance length, UE's p.ToleranceScale_Length 100 cm)
    /// and rest offset (the APEX template's 0).
    pub contact_offset: f32,
    pub rest_offset: f32,
    /// PhysX's sleepThreshold (5e-5 x the tolerance speed², p.ToleranceScale_Speed 1000 cm/s: 50 cm²/s²) and the wake
    /// counter (0.4 s).
    pub sleep_threshold: f32,
    pub wake_time: f32,
    pub position_iterations: u32,
    pub velocity_iterations: u32,
}

impl Default for Params {
    fn default() -> Self {
        Params {
            gravity: Vec3::new(0.0, -9.8, 0.0),
            linear_damping: 0.01,
            angular_damping: 0.0,
            // Sc::BodyCore's constructor (0x1800e0690) for a PxRigidDynamic: maxAngularVelocitySq 49.
            max_angular_velocity: 7.0,
            restitution: 0.3,
            static_friction: 0.7,
            dynamic_friction: 0.7,
            bounce_threshold: 2.0,
            contact_offset: 0.02,
            rest_offset: 0.0,
            sleep_threshold: 50.0e-4,
            wake_time: 0.4,
            position_iterations: 4,
            velocity_iterations: 1,
        }
    }
}

/// A convex rigid body: its centre of mass and orientation (world), velocities, inverse mass and inverse inertia (body
/// frame), and its hull's points about the centre of mass (body frame).
#[derive(Clone, Debug)]
pub struct Body {
    pub pos: Vec3,
    pub rot: Quat,
    pub v: Vec3,
    pub w: Vec3,
    pub inv_mass: f32,
    pub inv_inertia: Mat3,
    pub points: Vec<Vec3>,
    pub radius: f32,
    pub asleep: bool,
    sleep_time: f32,
}

/// A convex hull's volume, centroid and inertia about the centroid for density 1 (the canonical tetrahedron covariance
/// summed over the hull's faces).
pub fn mass_properties(tris: &[Tri]) -> (f32, Vec3, Mat3) {
    let canonical = Mat3::from_cols(Vec3::new(2.0, 1.0, 1.0), Vec3::new(1.0, 2.0, 1.0), Vec3::new(1.0, 1.0, 2.0)) * (1.0 / 120.0);
    let (mut volume, mut first, mut cov) = (0.0f32, Vec3::ZERO, Mat3::ZERO);
    for t in tris {
        let a = Mat3::from_cols(t.a, t.b, t.c);
        let det = a.determinant();
        volume += det / 6.0;
        first += det * (t.a + t.b + t.c) / 24.0;
        cov += (a * canonical * a.transpose()) * det;
    }
    if volume.abs() < 1e-12 {
        return (0.0, Vec3::ZERO, Mat3::IDENTITY);
    }
    let com = first / volume;
    // About the centroid, then the inertia tensor trace(C) I - C.
    let c = cov - volume * outer(com, com);
    let trace = c.x_axis.x + c.y_axis.y + c.z_axis.z;
    (volume, com, Mat3::from_diagonal(Vec3::splat(trace)) - c)
}

fn outer(a: Vec3, b: Vec3) -> Mat3 {
    Mat3::from_cols(a * b.x, a * b.y, a * b.z)
}

impl Body {
    /// A body from its hull (world-scaled points in the body frame's orientation, about any origin) and its mass: the
    /// inertia scaled to the mass as PxRigidBodyExt::setMassAndUpdateInertia does. Returns it with its centre of mass
    /// in those points' frame.
    pub fn new(points: &[Vec3], mass: f32, at: Vec3, rot: Quat) -> Option<(Body, Vec3)> {
        let hull = crate::collision::convex_hull(points);
        let (volume, com, inertia) = mass_properties(&hull);
        if volume <= 0.0 || mass <= 0.0 {
            return None;
        }
        let inertia = inertia * (mass / volume);
        let local: Vec<Vec3> = points.iter().map(|p| *p - com).collect();
        let radius = local.iter().map(|p| p.length()).fold(0.0, f32::max);
        let body = Body {
            pos: at + rot * com,
            rot,
            v: Vec3::ZERO,
            w: Vec3::ZERO,
            inv_mass: 1.0 / mass,
            inv_inertia: inertia.inverse(),
            points: local,
            radius,
            asleep: false,
            sleep_time: 0.0,
        };
        Some((body, com))
    }

    fn inv_inertia_world(&self) -> Mat3 {
        let r = Mat3::from_quat(self.rot);
        r * self.inv_inertia * r.transpose()
    }
}

/// One contact against the world: where, the world's normal, the separation.
#[derive(Clone, Copy, Debug)]
pub struct Contact {
    pub point: Vec3,
    pub normal: Vec3,
    pub separation: f32,
}

/// The body's contacts with the world (see the module's PLACEHOLDER), in patches of one normal, at most four each.
pub fn contacts(b: &Body, world: &(impl TriSource + ?Sized), p: &Params) -> Vec<Vec<Contact>> {
    let pts: Vec<Vec3> = b.points.iter().map(|q| b.pos + b.rot * *q).collect();
    let reach = p.contact_offset;
    let (min, max) = pts.iter().fold((Vec3::splat(f32::MAX), Vec3::splat(f32::MIN)), |(lo, hi), q| (lo.min(*q), hi.max(*q)));
    let mut found: Vec<Contact> = vec![];
    world.for_each_in(min - Vec3::splat(reach), max + Vec3::splat(reach), &mut |t: &Tri| {
        for x in &pts {
            let d = t.n.dot(*x - t.a);
            if d >= reach || d < -b.radius {
                continue;
            }
            // Over the triangle itself (its closest point straight below along the normal).
            let c = t.closest_point(*x);
            if c.distance(*x - t.n * d) > 1e-3 {
                continue;
            }
            found.push(Contact { point: *x, normal: t.n, separation: d });
        }
    });
    let mut patches: Vec<Vec<Contact>> = vec![];
    for c in found {
        match patches.iter_mut().find(|p| p[0].normal.dot(c.normal) > 0.999) {
            Some(p) => {
                if !p.iter().any(|o| o.point.distance_squared(c.point) < 1e-8) {
                    p.push(c);
                }
            }
            None => patches.push(vec![c]),
        }
    }
    for patch in &mut patches {
        if patch.len() > 4 {
            // The deepest, the farthest from it, the farthest from that line, then the one farthest the other way.
            let mut keep: Vec<Contact> = vec![];
            let deepest = *patch.iter().min_by(|a, b| a.separation.total_cmp(&b.separation)).unwrap();
            keep.push(deepest);
            let far = *patch.iter().max_by(|a, b| a.point.distance_squared(deepest.point).total_cmp(&b.point.distance_squared(deepest.point))).unwrap();
            keep.push(far);
            let axis = (far.point - deepest.point).normalize_or_zero();
            let off = |c: &Contact| {
                let d = c.point - deepest.point;
                (d - axis * d.dot(axis)).dot(deepest.normal.cross(axis))
            };
            keep.push(*patch.iter().max_by(|a, b| off(a).total_cmp(&off(b))).unwrap());
            keep.push(*patch.iter().min_by(|a, b| off(a).total_cmp(&off(b))).unwrap());
            *patch = keep;
        }
    }
    patches
}

struct NormalRow {
    r: Vec3,
    n: Vec3,
    vel_multiplier: f32,
    target: f32,
    bias: f32,
    unbiased_bias: f32,
    applied: f32,
}

struct FrictionRow {
    r: Vec3,
    t: Vec3,
    vel_multiplier: f32,
    applied: f32,
}

struct Patch {
    normals: Vec<NormalRow>,
    friction: Vec<FrictionRow>,
    broken: bool,
}

fn velocity_at(b: &Body, r: Vec3) -> Vec3 {
    b.v + b.w.cross(r)
}

fn apply(b: &mut Body, iw: &Mat3, r: Vec3, impulse: Vec3) {
    b.v += impulse * b.inv_mass;
    b.w += *iw * r.cross(impulse);
}

/// One step of `dt` (a substep: UE's MaxSubstepDeltaTime 0.05, at most four a frame).
pub fn step(b: &mut Body, world: &(impl TriSource + ?Sized), p: &Params, dt: f32) {
    if b.asleep || dt <= 0.0 {
        return;
    }
    let inv_dt = 1.0 / dt;
    // Pre-integration.
    b.v += p.gravity * dt;
    b.v *= (1.0 - dt * p.linear_damping).max(0.0);
    b.w *= (1.0 - dt * p.angular_damping).max(0.0);
    if b.w.length() > p.max_angular_velocity {
        b.w = b.w.normalize() * p.max_angular_velocity;
    }
    // Constraint setup.
    let iw = b.inv_inertia_world();
    let unit = |r: Vec3, d: Vec3| {
        let rd = r.cross(d);
        b.inv_mass + rd.dot(iw * rd)
    };
    let mut patches: Vec<Patch> = vec![];
    for patch in contacts(b, world, p) {
        let mut normals = vec![];
        for c in &patch {
            let r = c.point - b.pos;
            let n = c.normal;
            let k = unit(r, n);
            let vel_multiplier = if k > 0.0 { 1.0 / k } else { 0.0 };
            let vrel = n.dot(velocity_at(b, r));
            let s = c.separation - p.rest_offset;
            let bounce = p.restitution > 0.0 && vrel < -p.bounce_threshold && s * inv_dt < -vrel;
            let target = if bounce { -p.restitution * vrel } else { 0.0 };
            let scaled = 0.8 * s * inv_dt * vel_multiplier;
            let bias = if bounce && s <= 0.0 { 0.0 } else { scaled };
            let unbiased_bias = if bounce { 0.0 } else { scaled.max(0.0) };
            normals.push(NormalRow { r, n, vel_multiplier, target, bias, unbiased_bias, applied: 0.0 });
        }
        // Two anchors, two directions each: along the sliding velocity and across it.
        let n = patch[0].normal;
        let first = patch[0].point;
        let far = patch.iter().max_by(|a, c| a.point.distance_squared(first).total_cmp(&c.point.distance_squared(first))).map_or(first, |c| c.point);
        let anchors: Vec<Vec3> = if far.distance_squared(first) > 1e-8 { vec![first, far] } else { vec![first] };
        let mut friction = vec![];
        for a in anchors {
            let r = a - b.pos;
            let slide = velocity_at(b, r);
            let tangential = slide - n * n.dot(slide);
            let t0 = if tangential.length_squared() > 1e-10 { tangential.normalize() } else { n.any_orthonormal_vector() };
            for t in [t0, n.cross(t0)] {
                let k = unit(r, t);
                friction.push(FrictionRow { r, t, vel_multiplier: if k > 0.0 { 1.0 / k } else { 0.0 }, applied: 0.0 });
            }
        }
        patches.push(Patch { normals, friction, broken: false });
    }
    let solve = |b: &mut Body, patches: &mut Vec<Patch>, biased: bool| {
        for patch in patches.iter_mut() {
            let mut sum = 0.0;
            for row in patch.normals.iter_mut() {
                let bias = if biased { row.bias } else { row.unbiased_bias };
                let vn = row.n.dot(velocity_at(b, row.r));
                let delta = (row.vel_multiplier * (row.target - vn) - bias).max(-row.applied);
                row.applied += delta;
                sum += row.applied;
                apply(b, &iw, row.r, row.n * delta);
            }
            let max_static = p.static_friction * sum;
            let max_dynamic = p.dynamic_friction * sum;
            for row in patch.friction.iter_mut() {
                let vt = row.t.dot(velocity_at(b, row.r));
                let mut applied = row.applied - row.vel_multiplier * vt;
                if applied.abs() > max_static {
                    patch.broken = true;
                }
                let limit = if patch.broken { max_dynamic } else { max_static };
                applied = applied.clamp(-limit, limit);
                let delta = applied - row.applied;
                row.applied = applied;
                apply(b, &iw, row.r, row.t * delta);
            }
        }
    };
    for _ in 0..p.position_iterations {
        solve(b, &mut patches, true);
    }
    // The pose moves with the position iterations' velocity; the velocity iterations' is the one kept.
    let (motion_v, motion_w) = (b.v, b.w);
    for _ in 0..p.velocity_iterations {
        solve(b, &mut patches, false);
    }
    b.pos += motion_v * dt;
    let spin = Quat::from_xyzw(motion_w.x, motion_w.y, motion_w.z, 0.0) * b.rot;
    b.rot = Quat::from_xyzw(b.rot.x + 0.5 * dt * spin.x, b.rot.y + 0.5 * dt * spin.y, b.rot.z + 0.5 * dt * spin.z, b.rot.w + 0.5 * dt * spin.w).normalize();
    // Sleep.
    let inertia = b.inv_inertia_world().inverse();
    let energy = 0.5 * (b.v.length_squared() + b.w.dot(inertia * b.w) * b.inv_mass);
    if energy < p.sleep_threshold {
        b.sleep_time += dt;
        if b.sleep_time >= p.wake_time {
            b.asleep = true;
            b.v = Vec3::ZERO;
            b.w = Vec3::ZERO;
        }
    } else {
        b.sleep_time = 0.0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::collision::TriList;

    fn floor() -> TriList {
        let (a, b, c, d) = (Vec3::new(-10.0, 0.0, -10.0), Vec3::new(10.0, 0.0, -10.0), Vec3::new(10.0, 0.0, 10.0), Vec3::new(-10.0, 0.0, 10.0));
        TriList::new(vec![Tri::new(a, d, c).unwrap(), Tri::new(a, c, b).unwrap()])
    }

    fn cube(h: f32) -> Vec<Vec3> {
        let mut v = vec![];
        for x in [-h, h] {
            for y in [-h, h] {
                for z in [-h, h] {
                    v.push(Vec3::new(x, y, z));
                }
            }
        }
        v
    }

    #[test]
    fn cube_inertia() {
        let hull = crate::collision::convex_hull(&cube(0.5));
        let (vol, com, i) = mass_properties(&hull);
        assert!((vol - 1.0).abs() < 1e-4, "{vol}");
        assert!(com.length() < 1e-4);
        // A unit cube of density 1: I = m (1 + 1) / 12 = 1/6 on each axis.
        assert!((i.x_axis.x - 1.0 / 6.0).abs() < 1e-4 && i.y_axis.x.abs() < 1e-4, "{i:?}");
    }

    #[test]
    fn dropped_box_comes_to_rest_on_the_floor() {
        let world = floor();
        let p = Params::default();
        let (mut b, _) = Body::new(&cube(0.1), 2.0, Vec3::new(0.0, 1.0, 0.0), Quat::from_rotation_z(0.3)).unwrap();
        b.v = Vec3::new(1.0, 0.0, 0.0);
        for _ in 0..400 {
            step(&mut b, &world, &p, 1.0 / 60.0);
        }
        // Resting on a face: its centre a half-size above the floor, still.
        assert!((b.pos.y - 0.1).abs() < 0.02, "{:?}", b.pos);
        assert!(b.v.length() < 0.05, "{:?}", b.v);
    }
}
