//! Rigid bodies stepped as PhysX 3.4 steps them (PhysX3_x64.dll and PhysX3Common_x64.dll from the game install, read
//! with their PDBs), for the APEX debris chunks (f13-game doors.rs). Metres and seconds (Unreal's centimetres / 100).
//!
//! The step:
//! - Dy::preIntegrationParallel (0x1801890f0): v += g dt; v *= max(0, 1 - dt linearDamping); w *= max(0, 1 - dt
//!   angularDamping); both clamped to their maximum speeds.
//! - Contacts (UE's PhysicsSettings bEnablePCM false: the legacy generators), see below.
//! - Dy::setupFinalizeSolverConstraints (0x18007bf10), per contact (normal n from B, the world or the other body, to
//!   A; separation s less the rest offset): velMultiplier = 1 / (unit response of both bodies along n); vrel the
//!   relative normal velocity; a bounce when the restitution is above 0, vrel < -bounceThreshold and s / dt < -vrel:
//!   the target velocity -restitution x vrel; bias = max(maxPenBias, 0.8 s / dt) x velMultiplier (none on a
//!   penetrating bounce); the velocity iterations' bias max(bias, 0) (none on a bounce).
//! - Dy::solveContact / solveContact_BStatic (0x18009acc0 / 0x18009b2a0), per patch: each normal impulse delta =
//!   max(velMultiplier (target - vrel) - bias, -applied); then the patch's friction rows (patch friction, PhysX's
//!   default): delta = -velMultiplier_t (t . v_rel), the applied force clamped to staticFriction x the patch's normal
//!   impulse, and once beyond it the patch "broken" and held to dynamicFriction x that.
//! - Four position iterations (biased), the poses integrated with the velocity they leave, one velocity iteration
//!   (unbiased) for the velocity kept (PxRigidDynamic's default solver counts).
//! - Sleep: a mass-normalized kinetic energy below sleepThreshold for the wake counter's 0.4 s stops a body; a contact
//!   with a moving one wakes it.
//!
//! The contacts (PhysX3Common):
//! - Convex against the triangle mesh: Gu::contactConvexMesh (0x1800864f0) -> contactHullMesh2 (0x180087b80) ->
//!   ConvexMeshContactGeneration::processTriangle (0x18008abc0) per triangle overlapping the hull's box: triangles
//!   whose plane has the hull's centre behind them are skipped; triangleConvexTest (0x18008bd40), a separating-axis
//!   test over the hull's face normals, the triangle's normal (preferred unless 1e-4 worse) and the edges' cross
//!   products, each axis's depth min(hull max - triangle min, triangle max - hull min), apart beyond the contact
//!   distance ending it; with the triangle's normal the best, generateContacts (0x180088310) at once, otherwise the
//!   triangle waits for generateLastContacts (0x180089140), which skips it when one of its vertices or edges already
//!   gave contacts. generateContacts: the normal turned toward the hull, the hull face most against it
//!   (mSelectClosestEdgeCB), Gu::contactPolygonPolygonExt (0x18008e050) of that face and the triangle.
//! - Convex against convex: Gu::contactConvexConvex (0x180081b50) -> GuContactHullHull (0x180081ca0): the same test
//!   over both hulls' faces and their edges' cross products; the faces of the two hulls most against each other along
//!   the best axis, contactPolygonPolygonExt.
//! - contactPolygonPolygonExt: each of the first polygon's vertices carried along the normal onto the second's plane,
//!   inside it (PointInConvexPolygon2D_OutCodes) and within the contact distance: a contact there with that distance;
//!   the same for the second's vertices into the first; then the edges crossing in the plane across the normal.
//!
//! PLACEHOLDER: the triangle mesh's edge flags (its cooked active edges: every edge here), the contact reduction
//! (ContactReductionAllIn and Dy::ContactReduction<6>: here at most four per patch: the deepest, the farthest from it,
//! and the two farthest either side of that line), and the friction anchors (a patch's first contact and its farthest).

use crate::collision::{Tri, TriSource};
use glam::{Mat3, Quat, Vec2, Vec3};

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
    /// The shapes' contact offset (PhysX's default 0.02 x the tolerance length, UE's p.ToleranceScale_Length 100 cm),
    /// so a pair's contact distance twice it, and the rest offset (the APEX template's 0).
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

/// A convex hull as PhysX keeps one: its vertices, its polygonal faces (coplanar triangles merged; each loop counter-
/// clockwise about its outward normal) and its edges.
#[derive(Clone, Debug, Default)]
pub struct Hull {
    pub verts: Vec<Vec3>,
    pub faces: Vec<Face>,
    pub edges: Vec<Edge>,
    /// Each vertex's neighbours along the edges (for hill-climbing projections).
    pub neighbours: Vec<Vec<usize>>,
}

/// From how many vertices a hull's projections climb its vertex graph instead of scanning every vertex (PhysX keeps
/// BigConvexData's valencies for its larger convexes and walks them the same way).
const CLIMB_FROM: usize = 32;

impl Hull {
    /// The hull's extent along a direction (body frame): min and max of its vertices' projections.
    fn extent(&self, dir: Vec3) -> (f32, f32) {
        if self.verts.len() < CLIMB_FROM {
            return project(&self.verts, dir);
        }
        let climb = |d: Vec3| {
            let mut at = 0;
            let mut best = self.verts[0].dot(d);
            loop {
                let next = self.neighbours[at].iter().map(|n| (*n, self.verts[*n].dot(d))).max_by(|a, b| a.1.total_cmp(&b.1));
                match next {
                    Some((n, v)) if v > best + 1e-7 => {
                        at = n;
                        best = v;
                    }
                    _ => return best,
                }
            }
        };
        (-climb(-dir), climb(dir))
    }
}

/// An edge: its two vertices and the two faces either side.
#[derive(Clone, Copy, Debug)]
pub struct Edge {
    pub a: usize,
    pub b: usize,
    pub f0: usize,
    pub f1: usize,
}

/// PxConvexMeshDesc's vertexLimit (255, PhysX's default cooking).
const HULL_VERTEX_LIMIT: usize = 255;

/// A point set ready for hulling: duplicates (render vertices split by normal or UV) welded at 0.5 mm, and, past the
/// cooking vertex limit, the hull's vertices thinned to it. PLACEHOLDER: PhysX's own limiting (its quickhull stopping
/// at the limit and expanding the planes) isn't ported; the farthest-point sample of the hull's vertices stands in.
pub fn hull_points(points: &[Vec3]) -> Vec<Vec3> {
    let mut seen = std::collections::HashSet::new();
    let welded: Vec<Vec3> = points.iter().copied().filter(|p| p.is_finite() && seen.insert((*p * 2000.0).round().as_ivec3().to_array())).collect();
    let tris = crate::collision::convex_hull(&welded);
    let mut verts: Vec<Vec3> = vec![];
    for t in &tris {
        for p in [t.a, t.b, t.c] {
            if !verts.iter().any(|v| v.distance_squared(p) < 1e-12) {
                verts.push(p);
            }
        }
    }
    if verts.len() <= HULL_VERTEX_LIMIT {
        return verts;
    }
    let mut keep = vec![verts[0]];
    let mut dist: Vec<f32> = verts.iter().map(|v| v.distance_squared(verts[0])).collect();
    while keep.len() < HULL_VERTEX_LIMIT {
        let (i, _) = dist.iter().enumerate().max_by(|a, b| a.1.total_cmp(b.1)).unwrap();
        let p = verts[i];
        keep.push(p);
        for (d, v) in dist.iter_mut().zip(&verts) {
            *d = d.min(v.distance_squared(p));
        }
    }
    keep
}

#[derive(Clone, Debug)]
pub struct Face {
    pub verts: Vec<usize>,
    pub n: Vec3,
    pub d: f32,
}

impl Hull {
    pub fn new(points: &[Vec3]) -> Option<Hull> {
        let tris = crate::collision::convex_hull(points);
        if tris.len() < 4 {
            return None;
        }
        // The hull's vertices (welded), its faces by plane.
        let mut verts: Vec<Vec3> = vec![];
        let index = |p: Vec3, verts: &mut Vec<Vec3>| match verts.iter().position(|v| v.distance_squared(p) < 1e-12) {
            Some(i) => i,
            None => {
                verts.push(p);
                verts.len() - 1
            }
        };
        let mut groups: Vec<(Vec3, f32, Vec<usize>)> = vec![];
        let mut tri_ids: Vec<[usize; 3]> = vec![];
        for t in &tris {
            let ids = [index(t.a, &mut verts), index(t.b, &mut verts), index(t.c, &mut verts)];
            tri_ids.push(ids);
            let d = t.n.dot(t.a);
            match groups.iter_mut().find(|g| g.0.dot(t.n) > 0.9999 && (g.1 - d).abs() < 1e-4) {
                Some(g) => g.2.extend(ids),
                None => groups.push((t.n, d, ids.to_vec())),
            }
        }
        let mut faces = vec![];
        for (n, d, mut ids) in groups {
            ids.sort_unstable();
            ids.dedup();
            // Ordered counter-clockwise about n around the centroid.
            let c = ids.iter().map(|i| verts[*i]).sum::<Vec3>() / ids.len() as f32;
            let u = n.any_orthonormal_vector();
            let v = n.cross(u);
            ids.sort_by(|a, b| {
                let (pa, pb) = (verts[*a] - c, verts[*b] - c);
                pa.dot(v).atan2(pa.dot(u)).total_cmp(&pb.dot(v).atan2(pb.dot(u)))
            });
            faces.push(Face { verts: ids, n, d });
        }
        let mut edges: Vec<Edge> = vec![];
        let mut open: std::collections::HashMap<(usize, usize), usize> = std::collections::HashMap::new();
        for (fi, f) in faces.iter().enumerate() {
            for k in 0..f.verts.len() {
                let (a, b) = (f.verts[k], f.verts[(k + 1) % f.verts.len()]);
                match open.remove(&(b, a)) {
                    Some(other) => edges.push(Edge { a, b, f0: other, f1: fi }),
                    None => {
                        open.insert((a, b), fi);
                    }
                }
            }
        }
        // The vertex graph from the hull's triangles (every vertex reachable, whatever the merged polygons).
        let mut neighbours: Vec<Vec<usize>> = vec![vec![]; verts.len()];
        for ids in &tri_ids {
            for k in 0..3 {
                let (a, b) = (ids[k], ids[(k + 1) % 3]);
                if !neighbours[a].contains(&b) {
                    neighbours[a].push(b);
                    neighbours[b].push(a);
                }
            }
        }
        Some(Hull { verts, faces, edges, neighbours })
    }
}

/// A convex rigid body: its centre of mass and orientation (world), velocities, inverse mass and inverse inertia (body
/// frame), and its hull about the centre of mass (body frame).
#[derive(Clone, Debug)]
pub struct Body {
    pub pos: Vec3,
    pub rot: Quat,
    pub v: Vec3,
    pub w: Vec3,
    pub inv_mass: f32,
    pub inv_inertia: Mat3,
    pub hull: Hull,
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
    /// A body from its hull points (world-scaled, in the body frame's orientation, about any origin) and its mass: the
    /// inertia scaled to the mass as PxRigidBodyExt::setMassAndUpdateInertia does. Returns it with its centre of mass
    /// in those points' frame.
    pub fn new(points: &[Vec3], mass: f32, at: Vec3, rot: Quat) -> Option<(Body, Vec3)> {
        let points = hull_points(points);
        let tris = crate::collision::convex_hull(&points);
        let (volume, com, inertia) = mass_properties(&tris);
        if volume <= 0.0 || mass <= 0.0 {
            return None;
        }
        let inertia = inertia * (mass / volume);
        let local: Vec<Vec3> = points.iter().map(|p| *p - com).collect();
        let hull = Hull::new(&local)?;
        let radius = hull.verts.iter().map(|p| p.length()).fold(0.0, f32::max);
        let body = Body {
            pos: at + rot * com,
            rot,
            v: Vec3::ZERO,
            w: Vec3::ZERO,
            inv_mass: 1.0 / mass,
            inv_inertia: inertia.inverse(),
            hull,
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

    fn world_verts(&self) -> Vec<Vec3> {
        self.hull.verts.iter().map(|v| self.pos + self.rot * *v).collect()
    }
}

/// One contact: where, the normal (from B to A), the separation (negative: overlapping).
#[derive(Clone, Copy, Debug)]
pub struct Contact {
    pub point: Vec3,
    pub normal: Vec3,
    pub separation: f32,
}

fn project(points: &[Vec3], axis: Vec3) -> (f32, f32) {
    points.iter().fold((f32::MAX, f32::MIN), |(lo, hi), p| {
        let d = p.dot(axis);
        (lo.min(d), hi.max(d))
    })
}

/// The overlap of two shapes along an axis (min(a max - b min, b max - a min)); None when apart beyond `reach`.
fn overlap(a: &impl Fn(Vec3) -> (f32, f32), b: &impl Fn(Vec3) -> (f32, f32), axis: Vec3, reach: f32) -> Option<f32> {
    let (amin, amax) = a(axis);
    let (bmin, bmax) = b(axis);
    if bmin > amax + reach || amin > bmax + reach {
        return None;
    }
    Some((amax - bmin).min(bmax - amin))
}

/// In a convex polygon (any winding), seen along `n`.
fn inside(poly: &[Vec3], p: Vec3, n: Vec3) -> bool {
    let mut sign = 0.0f32;
    for k in 0..poly.len() {
        let (a, b) = (poly[k], poly[(k + 1) % poly.len()]);
        let s = (b - a).cross(p - a).dot(n);
        if s.abs() < 1e-9 {
            continue;
        }
        if sign == 0.0 {
            sign = s.signum();
        } else if s.signum() != sign {
            return false;
        }
    }
    true
}

/// Gu::contactPolygonPolygonExt for polygon `p0` (A's) and `p1` (B's) with the contact normal `n` (B to A): A's
/// vertices onto B's plane along n, B's onto A's, then the edges crossing; separations along n.
fn polygon_polygon(p0: &[Vec3], n0: Vec3, p1: &[Vec3], n1: Vec3, n: Vec3, reach: f32, out: &mut Vec<Contact>) {
    let start = out.len();
    let along1 = n1.dot(n);
    if along1.abs() > 1e-6 {
        for v in p0 {
            // v - n t lies on B's plane.
            let t = n1.dot(*v - p1[0]) / along1;
            if t < reach && inside(p1, *v - n * t, n) {
                out.push(Contact { point: *v, normal: n, separation: t });
            }
        }
    }
    let along0 = n0.dot(n);
    if along0.abs() > 1e-6 {
        for u in p1 {
            // u + n t lies on A's plane.
            let t = n0.dot(p0[0] - *u) / along0;
            if t < reach && inside(p0, *u + n * t, n) {
                out.push(Contact { point: *u + n * t, normal: n, separation: t });
            }
        }
    }
    // The edges crossing in the plane across n.
    let u = n.any_orthonormal_vector();
    let w = n.cross(u);
    let flat = |p: Vec3| Vec2::new(p.dot(u), p.dot(w));
    for i in 0..p0.len() {
        let (a0, b0) = (p0[i], p0[(i + 1) % p0.len()]);
        for j in 0..p1.len() {
            let (a1, b1) = (p1[j], p1[(j + 1) % p1.len()]);
            let (fa0, fb0, fa1, fb1) = (flat(a0), flat(b0), flat(a1), flat(b1));
            let r = fb0 - fa0;
            let s = fb1 - fa1;
            let den = r.perp_dot(s);
            if den.abs() < 1e-12 {
                continue;
            }
            let q = fa1 - fa0;
            let alpha = q.perp_dot(s) / den;
            let beta = q.perp_dot(r) / den;
            if !(0.0..=1.0).contains(&alpha) || !(0.0..=1.0).contains(&beta) {
                continue;
            }
            let (pa, pb) = (a0 + (b0 - a0) * alpha, a1 + (b1 - a1) * beta);
            let sep = n.dot(pa - pb);
            if sep < reach {
                out.push(Contact { point: pa, normal: n, separation: sep });
            }
        }
    }
    // Duplicates (a vertex on an edge) once.
    let mut k = start;
    while k < out.len() {
        if out[start..k].iter().any(|c| c.point.distance_squared(out[k].point) < 1e-10) {
            out.remove(k);
        } else {
            k += 1;
        }
    }
}

/// The hull face most against a direction (the one whose normal is nearest `-dir`): mSelectClosestEdgeCB.
fn closest_face(hull: &Hull, rot: Quat, dir: Vec3) -> usize {
    (0..hull.faces.len()).min_by(|a, b| (rot * hull.faces[*a].n).dot(dir).total_cmp(&(rot * hull.faces[*b].n).dot(dir))).unwrap_or(0)
}

fn face_world(b: &Body, f: usize) -> (Vec<Vec3>, Vec3) {
    let face = &b.hull.faces[f];
    (face.verts.iter().map(|i| b.pos + b.rot * b.hull.verts[*i]).collect(), b.rot * face.n)
}

/// The best separating-axis candidate: (depth, axis, the triangle's or B's face normal won).
fn sat(a: &impl Fn(Vec3) -> (f32, f32), b: &impl Fn(Vec3) -> (f32, f32), axes: impl Iterator<Item = (Vec3, bool)>, reach: f32, prefer_b: bool) -> Option<(f32, Vec3, bool)> {
    let mut best: Option<(f32, Vec3, bool)> = None;
    for (axis, is_b) in axes {
        let len = axis.length();
        if len < 1e-6 {
            continue;
        }
        let axis = axis / len;
        let d = overlap(a, b, axis, reach)?;
        let better = match best {
            None => true,
            // triangleConvexTest: the triangle's normal wins unless 1e-4 deeper than the best.
            Some((bd, _, _)) if prefer_b && is_b => d < bd + 1e-4,
            Some((bd, _, _)) => d < bd,
        };
        if better {
            best = Some((d, axis, is_b));
        }
    }
    best
}

/// A body's contacts with the static world's triangles (ConvexMeshContactGeneration).
pub fn world_contacts(b: &Body, world: &(impl TriSource + ?Sized), p: &Params) -> Vec<Contact> {
    let reach = p.contact_offset;
    let verts = b.world_verts();
    let (min, max) = verts.iter().fold((Vec3::splat(f32::MAX), Vec3::splat(f32::MIN)), |(lo, hi), q| (lo.min(*q), hi.max(*q)));
    let face_normals: Vec<Vec3> = b.hull.faces.iter().map(|f| b.rot * f.n).collect();
    // The hull's world extent along a direction.
    let inv = b.rot.inverse();
    let hull_extent = |d: Vec3| {
        let (lo, hi) = b.hull.extent(inv * d);
        let c = b.pos.dot(d);
        (lo + c, hi + c)
    };
    // Each hull edge: its direction and its faces' normals (world).
    let edges: Vec<(Vec3, Vec3, Vec3)> = b.hull.edges.iter().map(|e| (b.rot * (b.hull.verts[e.b] - b.hull.verts[e.a]), face_normals[e.f0], face_normals[e.f1])).collect();
    let mut tris: Vec<Tri> = vec![];
    world.for_each_in(min - Vec3::splat(reach), max + Vec3::splat(reach), &mut |t: &Tri| tris.push(*t));
    let key = |p: Vec3| (p * 1.0e4).round().as_ivec3();
    let mut used_verts: Vec<glam::IVec3> = vec![];
    let mut used_edges: Vec<(glam::IVec3, glam::IVec3)> = vec![];
    let mut out: Vec<Contact> = vec![];
    let mut delayed: Vec<(Tri, Vec3, f32)> = vec![];
    let emit = |t: &Tri, n: Vec3, out: &mut Vec<Contact>| -> bool {
        // generateContacts: the normal toward the hull, the hull's face most against it, against the triangle.
        let centroid = (t.a + t.b + t.c) / 3.0;
        let n = if (b.pos - centroid).dot(n) < 0.0 { -n } else { n };
        let (face, face_n) = face_world(b, closest_face(&b.hull, b.rot, n));
        let before = out.len();
        polygon_polygon(&face, face_n, &[t.a, t.b, t.c], t.n, n, reach, out);
        out.len() > before
    };
    for t in &tris {
        // processTriangle: the hull's centre in front of the triangle.
        if t.n.dot(b.pos - t.a) < 0.0 {
            continue;
        }
        let tri = [t.a, t.b, t.c];
        let centroid = (t.a + t.b + t.c) / 3.0;
        // The triangle's edges with their outward directions in its plane.
        let tri_edges: Vec<(Vec3, Vec3)> = [(t.a, t.b), (t.b, t.c), (t.c, t.a)]
            .iter()
            .map(|(p, q)| {
                let d = *q - *p;
                let out = d.cross(t.n);
                (d, if out.dot(*p - centroid) < 0.0 { -out } else { out })
            })
            .collect();
        // Edge pairs whose Gauss-map arcs cross (a face of the Minkowski difference; the others can't be the
        // separating axis): the hull edge's arc between its faces' normals against the triangle edge's half circle
        // from n through its outward direction to -n, negated.
        let axes = face_normals
            .iter()
            .map(|n| (*n, false))
            .chain(std::iter::once((t.n, true)))
            .chain(edges.iter().flat_map(|(dir, na, nb)| {
                tri_edges.iter().filter_map(move |(te, out)| {
                    let (sa, sb) = (na.dot(*te), nb.dot(*te));
                    if sa * sb >= 0.0 {
                        return None;
                    }
                    let c = *na + (*nb - *na) * (sa / (sa - sb));
                    (c.dot(*out) < 0.0).then(|| (dir.cross(*te), false))
                })
            }));
        let Some((depth, axis, tri_axis)) = sat(&hull_extent, &|d: Vec3| project(&tri, d), axes, reach, true) else { continue };
        if tri_axis {
            if emit(t, axis, &mut out) {
                for p in tri {
                    used_verts.push(key(p));
                }
                for (a, c) in [(t.a, t.b), (t.b, t.c), (t.c, t.a)] {
                    let (ka, kc) = (key(a), key(c));
                    used_edges.push(if ka.to_array() < kc.to_array() { (ka, kc) } else { (kc, ka) });
                }
            }
        } else {
            delayed.push((*t, axis, depth));
        }
    }
    // generateLastContacts: a waiting triangle only when none of its vertices or edges gave contacts.
    for (t, axis, _) in delayed {
        let shared = [t.a, t.b, t.c].iter().any(|p| used_verts.contains(&key(*p)))
            || [(t.a, t.b), (t.b, t.c), (t.c, t.a)].iter().any(|(a, c)| {
                let (ka, kc) = (key(*a), key(*c));
                used_edges.contains(&if ka.to_array() < kc.to_array() { (ka, kc) } else { (kc, ka) })
            });
        if !shared {
            emit(&t, axis, &mut out);
        }
    }
    out
}

/// Two bodies' contacts (GuContactHullHull), normals from `b` to `a`.
pub fn pair_contacts(a: &Body, b: &Body, p: &Params) -> Vec<Contact> {
    let reach = 2.0 * p.contact_offset;
    if a.pos.distance(b.pos) > a.radius + b.radius + reach {
        return vec![];
    }
    let (inv_a, inv_b) = (a.rot.inverse(), b.rot.inverse());
    let ea_fn = |d: Vec3| {
        let (lo, hi) = a.hull.extent(inv_a * d);
        let c = a.pos.dot(d);
        (lo + c, hi + c)
    };
    let eb_fn = |d: Vec3| {
        let (lo, hi) = b.hull.extent(inv_b * d);
        let c = b.pos.dot(d);
        (lo + c, hi + c)
    };
    let fa: Vec<Vec3> = a.hull.faces.iter().map(|f| a.rot * f.n).collect();
    let fb: Vec<Vec3> = b.hull.faces.iter().map(|f| b.rot * f.n).collect();
    // Edge pairs that make a Minkowski face (their Gauss-map arcs, B's negated, cross), with each edge's arc
    // precomputed: A's (a, b, b x a), B's (c, d, d x c) for its negated normals.
    let ea: Vec<(Vec3, Vec3, Vec3, Vec3)> = a
        .hull
        .edges
        .iter()
        .map(|e| {
            let (na, nb) = (fa[e.f0], fa[e.f1]);
            (a.rot * (a.hull.verts[e.b] - a.hull.verts[e.a]), na, nb, nb.cross(na))
        })
        .collect();
    let eb: Vec<(Vec3, Vec3, Vec3, Vec3)> = b
        .hull
        .edges
        .iter()
        .map(|e| {
            let (c, d) = (-fb[e.f0], -fb[e.f1]);
            (b.rot * (b.hull.verts[e.b] - b.hull.verts[e.a]), c, d, d.cross(c))
        })
        .collect();
    let eb = &eb;
    let axes = fa.iter().chain(fb.iter()).map(|n| (*n, false)).chain(ea.iter().flat_map(move |(x, na, nb, bxa)| {
        eb.iter().filter_map(move |(y, c, d, dxc)| {
            let (cba, dba) = (c.dot(*bxa), d.dot(*bxa));
            if cba * dba >= 0.0 {
                return None;
            }
            let (adc, bdc) = (na.dot(*dxc), nb.dot(*dxc));
            (adc * bdc < 0.0 && cba * bdc > 0.0).then(|| (x.cross(*y), false))
        })
    }));
    let Some((_, axis, _)) = sat(&ea_fn, &eb_fn, axes, reach, false) else { return vec![] };
    let n = if (a.pos - b.pos).dot(axis) < 0.0 { -axis } else { axis };
    let (pa, na) = face_world(a, closest_face(&a.hull, a.rot, n));
    let (pb, nb) = face_world(b, closest_face(&b.hull, b.rot, -n));
    let mut out = vec![];
    polygon_polygon(&pa, na, &pb, nb, n, reach, &mut out);
    out
}

/// Patches of one normal, at most four contacts each (see the module's PLACEHOLDER).
fn patches(contacts: Vec<Contact>) -> Vec<Vec<Contact>> {
    let mut patches: Vec<Vec<Contact>> = vec![];
    for c in contacts {
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
            let deepest = *patch.iter().min_by(|a, b| a.separation.total_cmp(&b.separation)).unwrap();
            let far = *patch.iter().max_by(|a, b| a.point.distance_squared(deepest.point).total_cmp(&b.point.distance_squared(deepest.point))).unwrap();
            let axis = (far.point - deepest.point).normalize_or_zero();
            let off = |c: &Contact| {
                let d = c.point - deepest.point;
                (d - axis * d.dot(axis)).dot(deepest.normal.cross(axis))
            };
            let side_a = *patch.iter().max_by(|a, b| off(a).total_cmp(&off(b))).unwrap();
            let side_b = *patch.iter().min_by(|a, b| off(a).total_cmp(&off(b))).unwrap();
            *patch = vec![deepest, far, side_a, side_b];
        }
    }
    patches
}

struct NormalRow {
    ra: Vec3,
    rb: Vec3,
    n: Vec3,
    vel_multiplier: f32,
    target: f32,
    bias: f32,
    unbiased_bias: f32,
    applied: f32,
}

struct FrictionRow {
    ra: Vec3,
    rb: Vec3,
    t: Vec3,
    vel_multiplier: f32,
    applied: f32,
}

struct Patch {
    a: usize,
    b: Option<usize>,
    normals: Vec<NormalRow>,
    friction: Vec<FrictionRow>,
    broken: bool,
}

struct State {
    v: Vec3,
    w: Vec3,
    inv_mass: f32,
    iw: Mat3,
}

fn rel_velocity(s: &[State], a: usize, b: Option<usize>, ra: Vec3, rb: Vec3) -> Vec3 {
    let va = s[a].v + s[a].w.cross(ra);
    let vb = b.map_or(Vec3::ZERO, |b| s[b].v + s[b].w.cross(rb));
    va - vb
}

fn apply(s: &mut [State], a: usize, b: Option<usize>, ra: Vec3, rb: Vec3, impulse: Vec3) {
    let (ma, ia) = (s[a].inv_mass, s[a].iw);
    s[a].v += impulse * ma;
    s[a].w += ia * ra.cross(impulse);
    if let Some(b) = b {
        let (mb, ib) = (s[b].inv_mass, s[b].iw);
        s[b].v -= impulse * mb;
        s[b].w -= ib * rb.cross(impulse);
    }
}

/// One step of `dt` for all the bodies together (a substep: UE's MaxSubstepDeltaTime 0.05, at most four a frame).
pub fn step_all(bodies: &mut [&mut Body], world: &(impl TriSource + ?Sized), p: &Params, dt: f32) {
    if dt <= 0.0 || bodies.is_empty() {
        return;
    }
    let inv_dt = 1.0 / dt;
    // Pre-integration of the awake.
    for b in bodies.iter_mut().filter(|b| !b.asleep) {
        b.v += p.gravity * dt;
        b.v *= (1.0 - dt * p.linear_damping).max(0.0);
        b.w *= (1.0 - dt * p.angular_damping).max(0.0);
        if b.w.length() > p.max_angular_velocity {
            b.w = b.w.normalize() * p.max_angular_velocity;
        }
    }
    // Contacts: each awake body with the world; each pair with one awake (waking the other).
    let mut found: Vec<(usize, Option<usize>, Vec<Vec<Contact>>)> = vec![];
    for i in 0..bodies.len() {
        if !bodies[i].asleep {
            let c = world_contacts(bodies[i], world, p);
            if !c.is_empty() {
                found.push((i, None, patches(c)));
            }
        }
    }
    for i in 0..bodies.len() {
        for j in i + 1..bodies.len() {
            if bodies[i].asleep && bodies[j].asleep {
                continue;
            }
            let c = pair_contacts(bodies[i], bodies[j], p);
            if !c.is_empty() {
                bodies[i].asleep = false;
                bodies[j].asleep = false;
                found.push((i, Some(j), patches(c)));
            }
        }
    }
    let mut state: Vec<State> = bodies
        .iter()
        .map(|b| {
            let still = b.asleep;
            State { v: b.v, w: b.w, inv_mass: if still { 0.0 } else { b.inv_mass }, iw: if still { Mat3::ZERO } else { b.inv_inertia_world() } }
        })
        .collect();
    // Constraint setup.
    let unit = |s: &[State], a: usize, b: Option<usize>, ra: Vec3, rb: Vec3, d: Vec3| {
        let x = ra.cross(d);
        let mut k = s[a].inv_mass + x.dot(s[a].iw * x);
        if let Some(b) = b {
            let y = rb.cross(d);
            k += s[b].inv_mass + y.dot(s[b].iw * y);
        }
        if k > 0.0 { 1.0 / k } else { 0.0 }
    };
    let mut rows: Vec<Patch> = vec![];
    for (a, b, list) in found {
        for patch in list {
            let pa = bodies[a].pos;
            let pb = b.map(|b| bodies[b].pos);
            let mut normals = vec![];
            for c in &patch {
                let (ra, rb) = (c.point - pa, pb.map_or(Vec3::ZERO, |pb| c.point - pb));
                let n = c.normal;
                let vel_multiplier = unit(&state, a, b, ra, rb, n);
                let vrel = n.dot(rel_velocity(&state, a, b, ra, rb));
                let s = c.separation - p.rest_offset;
                let bounce = p.restitution > 0.0 && vrel < -p.bounce_threshold && s * inv_dt < -vrel;
                let target = if bounce { -p.restitution * vrel } else { 0.0 };
                let scaled = 0.8 * s * inv_dt * vel_multiplier;
                let bias = if bounce && s <= 0.0 { 0.0 } else { scaled };
                let unbiased_bias = if bounce { 0.0 } else { scaled.max(0.0) };
                normals.push(NormalRow { ra, rb, n, vel_multiplier, target, bias, unbiased_bias, applied: 0.0 });
            }
            let n = patch[0].normal;
            let first = patch[0].point;
            let far = patch.iter().max_by(|x, y| x.point.distance_squared(first).total_cmp(&y.point.distance_squared(first))).map_or(first, |c| c.point);
            let anchors: Vec<Vec3> = if far.distance_squared(first) > 1e-8 { vec![first, far] } else { vec![first] };
            let mut friction = vec![];
            for at in anchors {
                let (ra, rb) = (at - pa, pb.map_or(Vec3::ZERO, |pb| at - pb));
                let slide = rel_velocity(&state, a, b, ra, rb);
                let tangential = slide - n * n.dot(slide);
                let t0 = if tangential.length_squared() > 1e-10 { tangential.normalize() } else { n.any_orthonormal_vector() };
                for t in [t0, n.cross(t0)] {
                    friction.push(FrictionRow { ra, rb, t, vel_multiplier: unit(&state, a, b, ra, rb, t), applied: 0.0 });
                }
            }
            rows.push(Patch { a, b, normals, friction, broken: false });
        }
    }
    let solve = |s: &mut Vec<State>, rows: &mut Vec<Patch>, biased: bool| {
        for patch in rows.iter_mut() {
            let (a, b) = (patch.a, patch.b);
            let mut sum = 0.0;
            for row in patch.normals.iter_mut() {
                let bias = if biased { row.bias } else { row.unbiased_bias };
                let vn = row.n.dot(rel_velocity(s, a, b, row.ra, row.rb));
                let delta = (row.vel_multiplier * (row.target - vn) - bias).max(-row.applied);
                row.applied += delta;
                sum += row.applied;
                apply(s, a, b, row.ra, row.rb, row.n * delta);
            }
            let max_static = p.static_friction * sum;
            let max_dynamic = p.dynamic_friction * sum;
            for row in patch.friction.iter_mut() {
                let vt = row.t.dot(rel_velocity(s, a, b, row.ra, row.rb));
                let mut applied = row.applied - row.vel_multiplier * vt;
                if applied.abs() > max_static {
                    patch.broken = true;
                }
                let limit = if patch.broken { max_dynamic } else { max_static };
                applied = applied.clamp(-limit, limit);
                let delta = applied - row.applied;
                row.applied = applied;
                apply(s, a, b, row.ra, row.rb, row.t * delta);
            }
        }
    };
    for _ in 0..p.position_iterations {
        solve(&mut state, &mut rows, true);
    }
    // The poses move with the position iterations' velocity; the velocity iterations' is the one kept.
    let motion: Vec<(Vec3, Vec3)> = state.iter().map(|s| (s.v, s.w)).collect();
    for _ in 0..p.velocity_iterations {
        solve(&mut state, &mut rows, false);
    }
    for (k, b) in bodies.iter_mut().enumerate() {
        if b.asleep {
            continue;
        }
        let (mv, mw) = motion[k];
        b.v = state[k].v;
        b.w = state[k].w;
        b.pos += mv * dt;
        let spin = Quat::from_xyzw(mw.x, mw.y, mw.z, 0.0) * b.rot;
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
}

/// One body alone.
pub fn step(b: &mut Body, world: &(impl TriSource + ?Sized), p: &Params, dt: f32) {
    step_all(&mut [b], world, p, dt);
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
    fn cube_hull_and_inertia() {
        let hull = Hull::new(&cube(0.5)).unwrap();
        assert_eq!((hull.verts.len(), hull.faces.len(), hull.edges.len()), (8, 6, 12));
        assert!(hull.edges.iter().all(|e| e.f0 != e.f1));
        let (vol, com, i) = mass_properties(&crate::collision::convex_hull(&cube(0.5)));
        assert!((vol - 1.0).abs() < 1e-4, "{vol}");
        assert!(com.length() < 1e-4);
        // A unit cube of density 1: I = m (1 + 1) / 12 = 1/6 on each axis.
        assert!((i.x_axis.x - 1.0 / 6.0).abs() < 1e-4 && i.y_axis.x.abs() < 1e-4, "{i:?}");
    }

    #[test]
    fn box_on_floor_face_contacts() {
        // A box resting on the floor: its four bottom corners (the corners on the floor's diagonal from both of its
        // triangles, one patch of four), separation 0.
        let (b, _) = Body::new(&cube(0.1), 1.0, Vec3::new(0.0, 0.1, 0.0), Quat::IDENTITY).unwrap();
        let c = world_contacts(&b, &floor(), &Params::default());
        let p = patches(c.clone());
        assert_eq!((p.len(), p[0].len()), (1, 4), "{c:?}");
        assert!(c.iter().all(|c| c.separation.abs() < 1e-4 && c.normal.y > 0.999));
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

    #[test]
    fn stacked_boxes_rest() {
        // Two boxes dropped one above the other: the top one comes to rest on the lower (chunk-chunk contacts).
        let world = floor();
        let p = Params::default();
        let (mut a, _) = Body::new(&cube(0.1), 2.0, Vec3::new(0.0, 0.15, 0.0), Quat::IDENTITY).unwrap();
        let (mut b, _) = Body::new(&cube(0.1), 2.0, Vec3::new(0.02, 0.6, 0.0), Quat::IDENTITY).unwrap();
        for _ in 0..600 {
            step_all(&mut [&mut a, &mut b], &world, &p, 1.0 / 60.0);
        }
        assert!((a.pos.y - 0.1).abs() < 0.02, "lower {:?}", a.pos);
        assert!((b.pos.y - 0.3).abs() < 0.03, "upper {:?}", b.pos);
    }
}
