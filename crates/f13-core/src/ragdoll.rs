//! A ragdoll: the bodies of a character's physics asset as spheres (one per simulated bone, plus a tip for the
//! bodies at the ends of the chains), held together by distance constraints and pushed out of the world's
//! triangles. Position-based dynamics (Verlet integration, then constraint and collision passes).
//!
//! The game ragdolls dead counselors with PhysX rigid bodies and the physics asset's joint limits. Here each body's
//! orientation follows its particles (the direction to the particle it points at, rolled by a left/right pair of
//! particles where it has one), and a joint's swing limits are kept the way PhysX measures them: between the joint's
//! frame on the child body and its frame on the parent, an elliptical cone (Swing1 toward the frame's Y, Swing2
//! toward its Z). Out of the cone, the child's aim particle is turned back about the joint. Twist is not simulated.
//!
//! Space: Y up, metres, seconds.

use crate::collision::{Tri, TriSource};
use glam::{Mat3, Quat, Vec3};

/// How far past its limit a joint may rest when what hangs off it lies on the ground.
const GROUND_HOLDS_RAD: f32 = 0.2;

#[derive(Clone, Debug)]
pub struct Particle {
    pub pos: Vec3,
    prev: Vec3,
    pub radius: f32,
    /// Touching the world after the last step: the surface's normal.
    pub contact: Option<Vec3>,
}

/// Keeps the distance between two particles within `min..=max`.
#[derive(Clone, Debug)]
pub struct Link {
    pub a: usize,
    pub b: usize,
    pub min: f32,
    pub max: f32,
}

#[derive(Clone, Debug)]
pub struct RagdollSettings {
    pub gravity: f32,
    /// Velocity kept per second in the air (Verlet damping).
    pub air_damping: f32,
    /// Fraction of the sliding velocity removed per contact step.
    pub friction: f32,
    pub iterations: usize,
    /// At rest when every particle moves slower than this (m/s) for `sleep_seconds`.
    pub sleep_speed: f32,
    pub sleep_seconds: f32,
}

impl Default for RagdollSettings {
    fn default() -> Self {
        RagdollSettings { gravity: 9.8, air_damping: 0.6, friction: 0.5, iterations: 10, sleep_speed: 0.08, sleep_seconds: 1.0 }
    }
}

/// A rigid body's orientation from its particles: `rotation0` when the particle at `particle` pointed at `aim` along
/// `aim0` (and its `sides` pair, left to right, along `side0`).
#[derive(Clone, Debug)]
pub struct Body {
    pub particle: usize,
    pub aim: usize,
    pub sides: Option<(usize, usize)>,
    pub rotation0: Quat,
    pub aim0: Vec3,
    pub side0: Option<Vec3>,
    /// The particles that turn with the body about its particle: its aim and everything hanging off it.
    pub subtree: Vec<usize>,
}

/// A joint's swing limit: the child body's frame X axis (`x1`, in the child's local space) stays within an elliptical
/// cone around the parent's frame X axis (`frame2`'s, in the parent's local space): `swing1` radians toward the frame's
/// Y, `swing2` toward its Z. Pi or more: free.
#[derive(Clone, Debug)]
pub struct Cone {
    pub child: usize,
    pub parent: usize,
    pub x1: Vec3,
    pub frame2: Mat3,
    pub swing1: f32,
    pub swing2: f32,
}

#[derive(Clone, Debug, Default)]
pub struct Ragdoll {
    pub particles: Vec<Particle>,
    pub links: Vec<Link>,
    pub bodies: Vec<Body>,
    pub cones: Vec<Cone>,
    still: f32,
    /// Came to rest: no longer simulated.
    pub asleep: bool,
}

impl Ragdoll {
    /// Adds a particle at `pos` moving at `velocity`.
    pub fn add(&mut self, pos: Vec3, radius: f32, velocity: Vec3, dt: f32) -> usize {
        self.particles.push(Particle { pos, prev: pos - velocity * dt, radius, contact: None });
        self.particles.len() - 1
    }

    /// Keeps `a` and `b` at their current distance.
    pub fn rigid(&mut self, a: usize, b: usize) {
        let d = self.particles[a].pos.distance(self.particles[b].pos);
        self.links.push(Link { a, b, min: d, max: d });
    }

    /// Lets the angle at `joint` between `parent` and `child` bend by up to `swing_deg` either way from its current
    /// value (straightening stops at 180 degrees).
    pub fn bend(&mut self, parent: usize, joint: usize, child: usize, swing_deg: f32) {
        let (p, j, c) = (self.particles[parent].pos, self.particles[joint].pos, self.particles[child].pos);
        let (a, b) = (p.distance(j), j.distance(c));
        if a < 1e-4 || b < 1e-4 {
            return;
        }
        let angle = (p - j).angle_between(c - j);
        let swing = swing_deg.to_radians();
        let side = |t: f32| (a * a + b * b - 2.0 * a * b * t.clamp(0.0, std::f32::consts::PI).cos()).max(0.0).sqrt();
        // A locked joint still gets a little play (PhysX joints are soft too).
        let swing = swing.max(5f32.to_radians());
        self.links.push(Link { a: parent, b: child, min: side(angle - swing), max: side(angle + swing) });
    }

    /// A body's current orientation.
    pub fn rotation(&self, b: &Body) -> Quat {
        let pos = |i: usize| self.particles[i].pos;
        let side = b.sides.zip(b.side0).map(|((l, r), s0)| (s0, pos(r) - pos(l)));
        turn(b.aim0, pos(b.aim) - pos(b.particle), side) * b.rotation0
    }

    /// Each joint's swing now (degrees toward its frame's Y and Z), for logs.
    pub fn swings(&self) -> Vec<(f32, f32)> {
        self.cones
            .iter()
            .map(|c| {
                let x1 = (self.rotation(&self.bodies[c.child]) * c.x1).normalize_or_zero();
                let f2 = Mat3::from_quat(self.rotation(&self.bodies[c.parent])) * c.frame2;
                let (cx, cy, cz) = (x1.dot(f2.x_axis), x1.dot(f2.y_axis), x1.dot(f2.z_axis));
                (cy.atan2(cx).to_degrees(), cz.atan2(cx).to_degrees())
            })
            .collect()
    }

    /// Turns the child of each joint (with everything hanging off it) back into its swing cone. The correction moves
    /// the particles without giving them speed: the bodies' roll (which hinges like the knee are measured in) isn't
    /// simulated, so a hinge can disagree with its parent from one step to the next, and that must not pump energy in.
    fn keep_cones(&mut self) {
        for k in 0..self.cones.len() {
            let c = &self.cones[k];
            let (child, parent) = (&self.bodies[c.child], &self.bodies[c.parent]);
            if child.aim == child.particle || (c.swing1 >= std::f32::consts::PI && c.swing2 >= std::f32::consts::PI) {
                continue;
            }
            let x1 = self.rotation(child) * c.x1;
            let f2 = Mat3::from_quat(self.rotation(parent)) * c.frame2;
            let Some(clamped) = clamp_to_cone(x1, f2, c.swing1, c.swing2) else { continue };
            // Resting on the ground a little past the limit: the ground holds it (turning it back would only slide
            // the body along the ground, step after step).
            let grounded = child.subtree.iter().any(|&i| self.particles[i].contact.is_some());
            if grounded && x1.angle_between(clamped) < GROUND_HOLDS_RAD {
                continue;
            }
            let fix = Quat::from_rotation_arc(x1.normalize(), clamped);
            let j = self.particles[child.particle].pos;
            for k in 0..self.bodies[c.child].subtree.len() {
                let i = self.bodies[c.child].subtree[k];
                let a = self.particles[i].pos;
                let moved = j + fix * (a - j) - a;
                self.particles[i].pos += moved;
                self.particles[i].prev += moved;
            }
        }
    }

    /// Brings the starting pose within the constraints (a pose the animation took past a joint's limits) without
    /// turning the correction into speed.
    pub fn settle(&mut self, world: &(impl TriSource + ?Sized), iterations: usize) {
        let velocity: Vec<Vec3> = self.particles.iter().map(|p| p.pos - p.prev).collect();
        for _ in 0..iterations {
            self.solve_links();
            self.keep_cones();
            for p in &mut self.particles {
                push_out(world, &mut p.pos, p.radius);
            }
        }
        for (p, v) in self.particles.iter_mut().zip(velocity) {
            p.prev = p.pos - v;
        }
    }

    fn solve_links(&mut self) {
        for l in &self.links {
            let (pa, pb) = (self.particles[l.a].pos, self.particles[l.b].pos);
            let d = pb - pa;
            let len = d.length();
            if len < 1e-6 {
                continue;
            }
            let target = len.clamp(l.min, l.max);
            if (target - len).abs() < 1e-6 {
                continue;
            }
            let fix = d / len * ((len - target) * 0.5);
            self.particles[l.a].pos += fix;
            self.particles[l.b].pos -= fix;
        }
    }

    /// One step: gravity, constraints, collisions with the world.
    pub fn step(&mut self, world: &(impl TriSource + ?Sized), dt: f32, s: &RagdollSettings) {
        if self.asleep || dt <= 0.0 {
            return;
        }
        let keep = s.air_damping.powf(dt);
        let start: Vec<Vec3> = self.particles.iter().map(|p| p.pos).collect();
        for p in &mut self.particles {
            let v = (p.pos - p.prev) * keep;
            p.prev = p.pos;
            p.pos += v + Vec3::NEG_Y * s.gravity * dt * dt;
        }
        // Don't let a fast particle pass through a surface between steps.
        for (i, p) in self.particles.iter_mut().enumerate() {
            if let Some((hit, n)) = sweep(world, start[i], p.pos) {
                p.pos = hit + n * p.radius;
            }
        }
        // The joints see last step's contacts until this step's are known.
        let mut contacts: Vec<Option<Vec3>> = vec![None; self.particles.len()];
        for _ in 0..s.iterations {
            self.solve_links();
            self.keep_cones();
            for (p, c) in self.particles.iter_mut().zip(&mut contacts) {
                if let Some(n) = push_out(world, &mut p.pos, p.radius) {
                    *c = Some(n);
                }
            }
        }
        // Contacts: no speed away from the surface (being pushed out of it is not a bounce), and friction takes part
        // of the sliding speed.
        for (p, c) in self.particles.iter_mut().zip(contacts) {
            p.contact = c;
            if let Some(n) = c {
                let mut v = p.pos - p.prev;
                let out = v.dot(n);
                if out > 0.0 {
                    v -= n * out;
                }
                v *= 1.0 - s.friction;
                p.prev = p.pos - v;
            }
        }
        let fastest = self.particles.iter().map(|p| (p.pos - p.prev).length() / dt).fold(0.0, f32::max);
        if fastest < s.sleep_speed {
            self.still += dt;
            if self.still >= s.sleep_seconds {
                self.asleep = true;
            }
        } else {
            self.still = 0.0;
        }
    }
}

/// The rotation turning `from` into `to`, using a second pair of directions (before, after) for the roll when there
/// is one.
pub fn turn(from: Vec3, to: Vec3, side: Option<(Vec3, Vec3)>) -> Quat {
    let frame = |a: Vec3, s: Vec3| -> Option<Mat3> {
        let x = a.try_normalize()?;
        let z = x.cross(s).try_normalize()?;
        Some(Mat3::from_cols(x, z.cross(x), z))
    };
    if let Some((s0, s1)) = side {
        if let (Some(f0), Some(f1)) = (frame(from, s0), frame(to, s1)) {
            return Quat::from_mat3(&(f1 * f0.transpose())).normalize();
        }
    }
    Quat::from_rotation_arc(from.normalize_or(Vec3::Y), to.normalize_or(Vec3::Y))
}

/// `x` (a direction) pulled back into the elliptical cone around `frame`'s X axis (`swing1` toward its Y, `swing2`
/// toward its Z), or None when it's inside.
fn clamp_to_cone(x: Vec3, frame: Mat3, swing1: f32, swing2: f32) -> Option<Vec3> {
    let x = x.try_normalize()?;
    let (ax, ay, az) = (frame.x_axis.normalize(), frame.y_axis.normalize(), frame.z_axis.normalize());
    let (cx, cy, cz) = (x.dot(ax), x.dot(ay), x.dot(az));
    let angle = cx.clamp(-1.0, 1.0).acos();
    let off = (cy * cy + cz * cz).sqrt();
    // A locked swing still gets some play (PhysX joints are soft too, and twist isn't simulated here).
    let (l1, l2) = (swing1.max(10f32.to_radians()), swing2.max(10f32.to_radians()));
    let (dy, dz) = if off > 1e-6 { (cy / off, cz / off) } else { (1.0, 0.0) };
    let limit = 1.0 / ((dy / l1).powi(2) + (dz / l2).powi(2)).sqrt();
    if angle <= limit || limit >= std::f32::consts::PI {
        return None;
    }
    Some((ax * limit.cos() + (ay * dy + az * dz) * limit.sin()).normalize())
}

/// Pushes a sphere out of the triangles it overlaps (to their front side). The direction it was pushed, if it touched
/// any.
fn push_out(world: &(impl TriSource + ?Sized), pos: &mut Vec3, r: f32) -> Option<Vec3> {
    let mut touched = false;
    let mut push = Vec3::ZERO;
    let mut deepest = 0.0f32;
    world.for_each_in(*pos - Vec3::splat(r), *pos + Vec3::splat(r), &mut |t: &Tri| {
        let cp = t.closest_point(*pos);
        let d = *pos - cp;
        let dist = d.length();
        if dist >= r {
            return;
        }
        touched = true;
        // Behind the face (sunk in), or centred on it: out along the face's normal.
        let (dir, depth) = if d.dot(t.n) <= 0.0 || dist < 1e-6 { (t.n, r + dist) } else { (d / dist, r - dist) };
        if depth > deepest {
            deepest = depth;
            push = dir * depth;
        }
    });
    *pos += push;
    touched.then(|| push.normalize_or(Vec3::Y))
}

/// The first surface the segment `a`..`b` enters from its front: the point and the surface normal.
fn sweep(world: &(impl TriSource + ?Sized), a: Vec3, b: Vec3) -> Option<(Vec3, Vec3)> {
    let d = b - a;
    if d.length_squared() < 1e-10 {
        return None;
    }
    let mut best: Option<(f32, Vec3)> = None;
    world.for_each_in(a.min(b), a.max(b), &mut |t: &Tri| {
        if d.dot(t.n) >= 0.0 {
            return; // leaving or along the surface
        }
        // Möller-Trumbore.
        let e1 = t.b - t.a;
        let e2 = t.c - t.a;
        let h = d.cross(e2);
        let det = e1.dot(h);
        if det.abs() < 1e-12 {
            return;
        }
        let f = 1.0 / det;
        let s = a - t.a;
        let u = f * s.dot(h);
        if !(0.0..=1.0).contains(&u) {
            return;
        }
        let q = s.cross(e1);
        let v = f * d.dot(q);
        if v < 0.0 || u + v > 1.0 {
            return;
        }
        let k = f * e2.dot(q);
        if (0.0..=1.0).contains(&k) && best.is_none_or(|(bk, _)| k < bk) {
            best = Some((k, t.n));
        }
    });
    best.map(|(k, n)| (a + d * k, n))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::collision::TriList;

    fn floor() -> TriList {
        let s = 10.0;
        TriList::new(vec![
            Tri::new(Vec3::new(-s, 0.0, -s), Vec3::new(-s, 0.0, s), Vec3::new(s, 0.0, s)).unwrap(),
            Tri::new(Vec3::new(-s, 0.0, -s), Vec3::new(s, 0.0, s), Vec3::new(s, 0.0, -s)).unwrap(),
        ])
    }

    #[test]
    fn a_dropped_body_lands_on_the_floor_and_sleeps() {
        let world = floor();
        let mut r = Ragdoll::default();
        let a = r.add(Vec3::new(0.0, 1.5, 0.0), 0.1, Vec3::ZERO, 1.0 / 64.0);
        let b = r.add(Vec3::new(0.0, 1.0, 0.0), 0.1, Vec3::ZERO, 1.0 / 64.0);
        let c = r.add(Vec3::new(0.0, 0.5, 0.1), 0.1, Vec3::ZERO, 1.0 / 64.0);
        r.rigid(a, b);
        r.rigid(b, c);
        r.bend(a, b, c, 45.0);
        let s = RagdollSettings::default();
        for _ in 0..64 * 8 {
            r.step(&world, 1.0 / 64.0, &s);
        }
        for p in &r.particles {
            assert!(p.pos.y > 0.09 && p.pos.y < 0.2, "{:?}", p.pos);
        }
        assert!((r.particles[a].pos.distance(r.particles[b].pos) - 0.5).abs() < 0.02);
        assert!(r.asleep);
    }

    #[test]
    fn a_fast_particle_does_not_tunnel_through_the_floor() {
        let world = floor();
        let mut r = Ragdoll::default();
        r.add(Vec3::new(0.0, 0.5, 0.0), 0.05, Vec3::new(0.0, -200.0, 0.0), 1.0 / 64.0);
        r.step(&world, 1.0 / 64.0, &RagdollSettings::default());
        assert!(r.particles[0].pos.y >= 0.04, "{:?}", r.particles[0].pos);
    }

    #[test]
    fn a_cone_clamps_to_its_ellipse() {
        let f = Mat3::IDENTITY;
        // Inside: no change.
        assert!(clamp_to_cone(Vec3::new(1.0, 0.1, 0.0), f, 0.5, 0.0).is_none());
        // 90 degrees toward Y with a 45 degree Swing1: back to 45.
        let c = clamp_to_cone(Vec3::Y, f, 45f32.to_radians(), 0.0).unwrap();
        assert!((c.angle_between(Vec3::X).to_degrees() - 45.0).abs() < 0.1, "{c:?}");
        // Toward Z with Swing2 locked: nearly straight.
        let c = clamp_to_cone(Vec3::new(1.0, 0.0, 1.0), f, 45f32.to_radians(), 0.0).unwrap();
        assert!(c.angle_between(Vec3::X).to_degrees() < 10.5, "{c:?}");
    }

    #[test]
    fn turning_with_a_side_keeps_the_roll() {
        let q = turn(Vec3::Y, Vec3::X, Some((Vec3::Z, Vec3::Z)));
        assert!((q * Vec3::Y).distance(Vec3::X) < 1e-4);
        assert!((q * Vec3::Z).distance(Vec3::Z) < 1e-4);
    }

    #[test]
    fn bend_limits_hold_a_straight_leg_within_its_swing() {
        let mut r = Ragdoll::default();
        let a = r.add(Vec3::new(0.0, 1.0, 0.0), 0.1, Vec3::ZERO, 0.01);
        let b = r.add(Vec3::ZERO, 0.1, Vec3::ZERO, 0.01);
        let c = r.add(Vec3::new(0.0, -1.0, 0.0), 0.1, Vec3::ZERO, 0.01);
        r.bend(a, b, c, 45.0);
        let l = r.links.last().unwrap();
        // Straight (180 degrees) can only fold toward 135.
        assert!((l.max - 2.0).abs() < 1e-4);
        assert!((l.min - (2.0 - 2.0 * (135f32).to_radians().cos()).sqrt()).abs() < 1e-4);
    }
}
