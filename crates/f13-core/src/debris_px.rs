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
//! - The first pass's reduction (ContactReductionAllIn), the friction patches and their anchors kept from step to step
//!   (getFrictionPatches, createContactPatches, correlatePatches, growPatches) and the triangle mesh's active edges
//!   (EdgeListBuilder::computeActiveEdges, as cooking flags them), see their functions.
//!
//! PLACEHOLDER: the active edges are worked out from the triangles near the body (an edge whose neighbour isn't among
//! them counts as a boundary); Dy::ContactReduction<6> (only for several shapes on one actor pair) isn't needed here.

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
    /// PxSceneDesc's frictionOffsetThreshold and frictionCorrelationDistance (PhysX's defaults 0.04 and 0.025 x the
    /// tolerance length): a contact this near gives a friction anchor; anchors at least this far apart.
    pub friction_offset_threshold: f32,
    pub friction_correlation_distance: f32,
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
            friction_offset_threshold: 0.04,
            friction_correlation_distance: 0.025,
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

/// QuickHull's post merge (PhysX3Cooking QuickHull::postMergeHull 0x18002a5e0): adjacent faces whose normals are
/// within MAXDOT_MINANG (0x18003a008: 0.99863, cos 3 degrees) merge, the larger taking the smaller, when
/// canMergeFaces (0x18002a760) allows it.
const MAXDOT_MINANG: f32 = 0.998_629_5;
/// UE's PxCookingParams (FPhysXCooking's setup 0x1411d28a0): planeTolerance 0.0007, eQUICKHULL.
const PLANE_TOLERANCE: f32 = 0.0007;

/// A polygon's Newell normal and its vertices' mean (QuickHull's computeNormalAndCentroid, as canMergeFaces does it).
fn polygon_plane(loop_: &[Vec3]) -> (Vec3, Vec3) {
    let mut n = Vec3::ZERO;
    for k in 0..loop_.len() {
        let (a, b) = (loop_[k], loop_[(k + 1) % loop_.len()]);
        n += Vec3::new((a.y - b.y) * (a.z + b.z), (a.z - b.z) * (a.x + b.x), (a.x - b.x) * (a.y + b.y));
    }
    (n.normalize_or_zero(), loop_.iter().sum::<Vec3>() / loop_.len() as f32)
}

/// Two faces' polygon loops joined across their shared edges (both counter-clockwise about their normals).
fn join_loops(a: &[usize], b: &[usize]) -> Option<Vec<usize>> {
    let mut edges: Vec<(usize, usize)> = vec![];
    for l in [a, b] {
        for k in 0..l.len() {
            edges.push((l[k], l[(k + 1) % l.len()]));
        }
    }
    let kept: Vec<(usize, usize)> = edges.iter().copied().filter(|(p, q)| !edges.contains(&(*q, *p))).collect();
    if kept.len() == edges.len() {
        return None;
    }
    let mut out = vec![kept[0].0];
    let mut at = kept[0].1;
    while at != out[0] {
        if out.len() > kept.len() {
            return None;
        }
        out.push(at);
        at = kept.iter().find(|e| e.0 == at)?.1;
    }
    (out.len() == kept.len()).then_some(out)
}

impl Hull {
    /// The hull of `points` as PhysX cooks it, its tolerances from the points themselves.
    pub fn new(points: &[Vec3]) -> Option<Hull> {
        Hull::cooked(points, points)
    }

    /// The hull of `points`, with QuickHull's tolerances (computeMinMaxVerts 0x180027ee0) from `cooked`, the points
    /// as they went to the cooker (their frame, cm): the sum s of their largest |x|, |y|, |z|, mTolerance = max(s x
    /// 3 FLT_EPSILON, 3 FLT_EPSILON) and mPlaneTolerance = max(s x planeTolerance, planeTolerance).
    pub fn cooked(points: &[Vec3], cooked: &[Vec3]) -> Option<Hull> {
        let s = cooked.iter().fold(Vec3::ZERO, |m, p| m.max(p.abs() * 100.0)).element_sum();
        const EPS3: f32 = 3.576_279_4e-7;
        let tolerance = (s * EPS3).max(EPS3) / 100.0;
        let plane_tolerance = (s * PLANE_TOLERANCE).max(PLANE_TOLERANCE) / 100.0;
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
        let mut faces: Vec<Face> = vec![];
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
        // postMergeHull: each live face in turn takes its neighbours while one qualifies.
        let area = |f: &Face| {
            let l: Vec<Vec3> = f.verts.iter().map(|i| verts[*i]).collect();
            (1..l.len().saturating_sub(1)).map(|k| (l[k] - l[0]).cross(l[k + 1] - l[0]).length()).sum::<f32>() * 0.5
        };
        let mut alive = vec![true; faces.len()];
        for fi in 0..faces.len() {
            if !alive[fi] {
                continue;
            }
            'merge: loop {
                for gi in 0..faces.len() {
                    if gi == fi || !alive[gi] || faces[fi].n.dot(faces[gi].n) <= MAXDOT_MINANG {
                        continue;
                    }
                    let (af, ag) = (area(&faces[fi]), area(&faces[gi]));
                    if af <= ag && ag != af {
                        continue;
                    }
                    let Some(joined) = join_loops(&faces[fi].verts, &faces[gi].verts) else { continue };
                    // canMergeFaces: every vertex within mPlaneTolerance of the merged plane, and the polygon convex
                    // (no vertex past an edge's line by more than mTolerance).
                    let l: Vec<Vec3> = joined.iter().map(|i| verts[*i]).collect();
                    let (n, c) = polygon_plane(&l);
                    let d = n.dot(c);
                    if l.iter().any(|p| n.dot(*p) - d > plane_tolerance) {
                        continue;
                    }
                    let convex = (0..l.len()).all(|k| {
                        let (p, q) = (l[k], l[(k + 1) % l.len()]);
                        let inward = n.cross((q - p).normalize_or_zero());
                        l.iter().all(|r| -(*r - p).dot(inward) <= tolerance)
                    });
                    if !convex {
                        continue;
                    }
                    faces[fi] = Face { verts: joined, n, d };
                    alive[gi] = false;
                    continue 'merge;
                }
                break;
            }
        }
        let faces: Vec<Face> = faces.into_iter().zip(alive).filter(|(_, a)| *a).map(|(f, _)| f).collect();
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
    /// Its id, and the friction patches it keeps with the world ([`WORLD`]) and with other bodies (by their id).
    pub id: u64,
    friction: Vec<(u64, Vec<StoredPatch>)>,
}

/// The world's id in a body's friction patches.
const WORLD: u64 = u64::MAX;

static NEXT_BODY: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// A friction patch kept to the next step (Dy::FrictionPatch): its normal in either body's frame (B's in the world for
/// the world), its anchors in either frame, and whether it broke (slid) this step.
#[derive(Clone, Debug)]
struct StoredPatch {
    normal_a: Vec3,
    normal_b: Vec3,
    anchors: Vec<(Vec3, Vec3)>,
    broken: bool,
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
        // Cooked from the chunk's own vertices (its mesh frame), then kept about its centre of mass.
        let hull = Hull::cooked(&local, &points)?;
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
            id: NEXT_BODY.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
            friction: vec![],
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
    let mut inside_count = 0;
    if along1.abs() > 1e-6 {
        for v in p0 {
            // v - n t lies on B's plane.
            let t = n1.dot(*v - p1[0]) / along1;
            if inside(p1, *v - n * t, n) {
                inside_count += 1;
                if t < reach {
                    out.push(Contact { point: *v, normal: n, separation: t });
                }
            }
        }
    }
    reduce_all_in(out, start, inside_count == p0.len(), n);
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

/// ContactReductionAllIn (0x18008f820) on the first pass's contacts (`out[start..]`) when over four: with all of the
/// first polygon's vertices inside, those at 0, n/4, n/2 and 3n/4 and the deepest when it's none of them; otherwise,
/// of eight directions in the plane (k pi/8), the one along which they spread widest, and the extremes along it and
/// across it.
fn reduce_all_in(out: &mut Vec<Contact>, start: usize, all_in: bool, n: Vec3) {
    let count = out.len() - start;
    if count <= 4 {
        return;
    }
    let list: Vec<Contact> = out[start..].to_vec();
    let keep: Vec<usize> = if all_in {
        let deepest = (0..count).min_by(|a, b| list[*a].separation.total_cmp(&list[*b].separation)).unwrap();
        let mut k = vec![0, count / 4, count / 2, count * 3 / 4];
        if !k.contains(&deepest) {
            k.push(deepest);
        }
        k
    } else {
        let u = n.any_orthonormal_vector();
        let w = n.cross(u);
        let flat: Vec<Vec2> = list.iter().map(|c| Vec2::new(c.point.dot(u), c.point.dot(w))).collect();
        let dirs: Vec<Vec2> = (0..8).map(|k| Vec2::from_angle(k as f32 * std::f32::consts::FRAC_PI_8)).collect();
        let spread = |d: Vec2| {
            let (lo, hi) = flat.iter().fold((f32::MAX, f32::MIN), |(lo, hi), p| (lo.min(p.dot(d)), hi.max(p.dot(d))));
            hi - lo
        };
        let d = *dirs.iter().max_by(|a, b| spread(**a).total_cmp(&spread(**b))).unwrap();
        let perp = d.perp();
        let ext = |axis: Vec2, max: bool| {
            (0..count).max_by(|a, b| {
                let (x, y) = (flat[*a].dot(axis), flat[*b].dot(axis));
                if max { x.total_cmp(&y) } else { y.total_cmp(&x) }
            }).unwrap()
        };
        let mut k: Vec<usize> = (0..count).filter(|i| [ext(d, false), ext(d, true), ext(perp, false), ext(perp, true)].contains(i)).collect();
        k.dedup();
        k
    };
    out.truncate(start);
    out.extend(keep.iter().map(|i| list[*i]));
}

/// The hull face most against a direction (SelectClosestEdgeCB_Convex 0x1800a0120 with -dir): the face whose normal
/// is nearest -dir; but when an edge's two faces' normals, summed, point nearer it than that face's, the nearer of
/// those two faces.
fn closest_face(hull: &Hull, rot: Quat, dir: Vec3) -> usize {
    let d = rot.inverse() * -dir;
    let mut best = 0;
    let mut best_dot = f32::MIN;
    for (k, f) in hull.faces.iter().enumerate() {
        let v = f.n.dot(d);
        if v > best_dot {
            best_dot = v;
            best = k;
        }
    }
    let mut score = best_dot * best_dot;
    let mut edge = None;
    for (k, e) in hull.edges.iter().enumerate() {
        let sum = hull.faces[e.f0].n + hull.faces[e.f1].n;
        let (len2, along) = (sum.length_squared(), sum.dot(d));
        if along >= 0.0 && len2 * score < along * along {
            score = along * along / len2;
            edge = Some(k);
        }
    }
    match edge {
        Some(k) => {
            let e = hull.edges[k];
            if hull.faces[e.f1].n.dot(d) < hull.faces[e.f0].n.dot(d) { e.f0 } else { e.f1 }
        }
        None => best,
    }
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
    // The mesh's active edges as cooking flags them (Gu::EdgeListBuilder::computeActiveEdges 0x180004cd0, its
    // angle 0.1 rad from TriangleMeshBuilder::createEdgeList): an edge on one triangle; between two, when the first's
    // opposite vertex is behind the second's plane (a convex ridge) and their normals part by more than 0.1 rad, or when
    // they fold back on each other (dot < -0.999).
    let close = |p: Vec3, q: Vec3| p.distance_squared(q) < 1e-10;
    let active = |ti: usize, p: Vec3, q: Vec3, opp: Vec3| -> bool {
        let t = &tris[ti];
        let Some(o) = tris.iter().enumerate().find(|(oi, o)| {
            *oi != ti && [o.a, o.b, o.c].iter().any(|v| close(*v, p)) && [o.a, o.b, o.c].iter().any(|v| close(*v, q))
        }) else {
            return true;
        };
        let o = o.1;
        if o.n.dot(opp - o.a) < 0.0 {
            o.n.cross(t.n).length().atan2(o.n.dot(t.n)) > 0.1
        } else {
            o.n.dot(t.n) < -0.999
        }
    };
    for (ti, t) in tris.iter().enumerate() {
        // processTriangle: the hull's centre in front of the triangle.
        if t.n.dot(b.pos - t.a) < 0.0 {
            continue;
        }
        let tri = [t.a, t.b, t.c];
        let centroid = (t.a + t.b + t.c) / 3.0;
        // The triangle's edges with their outward directions in its plane.
        // Only the active ones give edge axes (performEETests).
        let tri_edges: Vec<(Vec3, Vec3)> = [(t.a, t.b, t.c), (t.b, t.c, t.a), (t.c, t.a, t.b)]
            .iter()
            .filter(|(p, q, opp)| active(ti, *p, *q, *opp))
            .map(|(p, q, _)| {
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
        let r = sat(&hull_extent, &|d: Vec3| project(&tri, d), axes, reach, true);
        let Some((depth, axis, tri_axis)) = r else { continue };
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

/// Dy::createContactPatches (0x180094d20): consecutive contacts while their normal stays within 0.999 of the patch's
/// first's (the contact buffer holding 64).
fn contact_patches(contacts: &[Contact]) -> Vec<Vec<Contact>> {
    let mut patches: Vec<Vec<Contact>> = vec![];
    for c in contacts.iter().take(64) {
        match patches.last_mut() {
            Some(p) if p[0].normal.dot(c.normal) >= 0.999 => p.push(*c),
            _ => patches.push(vec![*c]),
        }
    }
    patches
}

/// A friction patch this step: its world normal, its contacts, its anchors (world points on A and on B), broken.
struct FrictionPatch {
    normal: Vec3,
    contacts: Vec<Contact>,
    anchors: Vec<(Vec3, Vec3)>,
    broken: bool,
}

fn to_world(b: Option<&Body>, local: Vec3) -> Vec3 {
    b.map_or(local, |b| b.pos + b.rot * local)
}

fn to_local(b: Option<&Body>, world: Vec3) -> Vec3 {
    b.map_or(world, |b| b.rot.inverse() * (world - b.pos))
}

/// A pair's friction patches as createFinalizeSolverContacts builds them: getFrictionPatches (0x18007b760) keeps last
/// step's that didn't break, have anchors, whose normals still agree (0.999) and whose anchors haven't parted along the
/// normal by the correlation distance; correlatePatches (0x180094ec0) adds each contact patch to the one of its normal
/// (0.999), else a new one (at most 32); growPatches (0x1800953e0) gives one with under two anchors: the first contact
/// nearer than the friction offset threshold, then the first farther than the correlation distance from it, then any
/// farther from one than the two are apart taking the other's place.
fn friction_patches(a: &Body, b: Option<&Body>, kept: &[StoredPatch], contacts: &[Contact], p: &Params) -> Vec<FrictionPatch> {
    let mut out: Vec<FrictionPatch> = vec![];
    for k in kept {
        if k.broken || k.anchors.is_empty() {
            continue;
        }
        let na = a.rot * k.normal_a;
        let nb = b.map_or(k.normal_b, |b| b.rot * k.normal_b);
        if na.dot(nb) <= 0.999 {
            continue;
        }
        let anchors: Vec<(Vec3, Vec3)> = k.anchors.iter().map(|(la, lb)| (to_world(Some(a), *la), to_world(b, *lb))).collect();
        if anchors.iter().any(|(wa, wb)| (*wa - *wb).dot(na).abs() >= p.friction_correlation_distance) {
            continue;
        }
        out.push(FrictionPatch { normal: na, contacts: vec![], anchors, broken: false });
    }
    for cp in contact_patches(contacts) {
        let n = cp[0].normal;
        match out.iter().position(|f| f.normal.dot(n) >= 0.999) {
            Some(k) => out[k].contacts.extend(cp),
            None if out.len() < 32 => out.push(FrictionPatch { normal: n, contacts: cp, anchors: vec![], broken: false }),
            None => {}
        }
    }
    // A patch no contact joined gives no constraint and isn't kept (setupFinalizeSolverConstraints skips a patch whose
    // contact count is 0).
    out.retain(|f| !f.contacts.is_empty());
    let threshold = p.friction_offset_threshold + p.rest_offset;
    let corr2 = p.friction_correlation_distance * p.friction_correlation_distance;
    for f in &mut out {
        if f.anchors.len() >= 2 {
            continue;
        }
        let mut points: Vec<Vec3> = f.anchors.iter().map(|x| x.0).collect();
        let mut apart = 0.0f32;
        for c in f.contacts.iter().filter(|c| c.separation < threshold) {
            match points.len() {
                0 => points.push(c.point),
                1 => {
                    let d = c.point.distance_squared(points[0]);
                    if d > corr2 {
                        points.push(c.point);
                        apart = d;
                    }
                }
                _ => {
                    let (d0, d1) = (c.point.distance_squared(points[0]), c.point.distance_squared(points[1]));
                    if d0 <= d1 {
                        if apart < d1 {
                            points[0] = c.point;
                            apart = d1;
                        }
                    } else if apart < d0 {
                        points[1] = c.point;
                        apart = d0;
                    }
                }
            }
        }
        // A new anchor is where its contact is, on both bodies.
        f.anchors = points.into_iter().map(|q| (q, q)).collect();
    }
    out
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
    /// The anchor's drift along t (on A less on B) x invDt.
    bias: f32,
    applied: f32,
}

struct Patch {
    a: usize,
    b: Option<usize>,
    normals: Vec<NormalRow>,
    friction: Vec<FrictionRow>,
    broken: bool,
    /// To keep: the patch's normal and anchors (world).
    normal: Vec3,
    anchors: Vec<(Vec3, Vec3)>,
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
    let mut found: Vec<(usize, Option<usize>, Vec<FrictionPatch>)> = vec![];
    for i in 0..bodies.len() {
        if !bodies[i].asleep {
            let c = world_contacts(bodies[i], world, p);
            if !c.is_empty() {
                let kept = bodies[i].friction.iter().find(|f| f.0 == WORLD).map_or(&[][..], |f| &f.1[..]);
                found.push((i, None, friction_patches(bodies[i], None, kept, &c, p)));
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
                let other = bodies[j].id;
                let kept = bodies[i].friction.iter().find(|f| f.0 == other).map_or(&[][..], |f| &f.1[..]);
                found.push((i, Some(j), friction_patches(bodies[i], Some(&*bodies[j]), kept, &c, p)));
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
            for c in &patch.contacts {
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
            // The tangents: the relative linear velocity's sliding part when over 0.1 cm²/s² (1e-5 m²/s²), else a
            // fixed perpendicular of n; then n x t0.
            let n = patch.normal;
            let dv = state[a].v - b.map_or(Vec3::ZERO, |b| state[b].v);
            let slide = dv - n * n.dot(dv);
            let t0 = if slide.length_squared() > 1e-5 {
                slide.normalize()
            } else if n.x.abs() < std::f32::consts::FRAC_1_SQRT_2 {
                Vec3::new(0.0, -n.z, n.y).normalize()
            } else {
                Vec3::new(-n.y, n.x, 0.0).normalize()
            };
            let t1 = n.cross(t0);
            let mut friction = vec![];
            for (wa, wb) in &patch.anchors {
                let (ra, rb) = (*wa - pa, pb.map_or(Vec3::ZERO, |pb| *wb - pb));
                for t in [t0, t1] {
                    friction.push(FrictionRow { ra, rb, t, vel_multiplier: unit(&state, a, b, ra, rb, t), bias: (*wa - *wb).dot(t) * inv_dt, applied: 0.0 });
                }
            }
            rows.push(Patch { a, b, normals, friction, broken: patch.broken, normal: n, anchors: patch.anchors });
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
                let bias = if biased { row.bias } else { 0.0 };
                let mut applied = row.applied - row.vel_multiplier * (vt + bias);
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
    // The friction patches kept, in either body's frame (before the bodies move: PhysX stores them at the setup).
    let mut kept: Vec<(usize, u64, StoredPatch)> = vec![];
    for r in &rows {
        if r.anchors.is_empty() {
            continue;
        }
        let a = &*bodies[r.a];
        let b = r.b.map(|b| &*bodies[b]);
        let key = b.map_or(WORLD, |b| b.id);
        let stored = StoredPatch {
            normal_a: a.rot.inverse() * r.normal,
            normal_b: b.map_or(r.normal, |b| b.rot.inverse() * r.normal),
            anchors: r.anchors.iter().map(|(wa, wb)| (to_local(Some(a), *wa), to_local(b, *wb))).collect(),
            broken: r.broken,
        };
        kept.push((r.a, key, stored));
    }
    for b in bodies.iter_mut().filter(|b| !b.asleep) {
        b.friction.clear();
    }
    for (a, key, stored) in kept {
        let list = &mut bodies[a].friction;
        match list.iter_mut().find(|f| f.0 == key) {
            Some(f) => f.1.push(stored),
            None => list.push((key, vec![stored])),
        }
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
        let p = contact_patches(&c);
        assert_eq!(p.len(), 1, "{c:?}");
        let mut unique: Vec<Vec3> = vec![];
        for q in &p[0] {
            if !unique.iter().any(|u| u.distance_squared(q.point) < 1e-10) {
                unique.push(q.point);
            }
        }
        assert_eq!(unique.len(), 4, "{c:?}");
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
    fn box_holds_on_a_slope() {
        // 20 degrees: tan 0.36 under the friction 0.7, so the kept anchors hold it (it neither slides nor creeps).
        let q = Quat::from_rotation_z(20f32.to_radians());
        let (a, b, c, d) = (q * Vec3::new(-10.0, 0.0, -10.0), q * Vec3::new(10.0, 0.0, -10.0), q * Vec3::new(10.0, 0.0, 10.0), q * Vec3::new(-10.0, 0.0, 10.0));
        let slope = TriList::new(vec![Tri::new(a, d, c).unwrap(), Tri::new(a, c, b).unwrap()]);
        let p = Params::default();
        let (mut body, _) = Body::new(&cube(0.1), 2.0, q * Vec3::new(0.0, 0.1, 0.0), q).unwrap();
        for _ in 0..30 {
            step(&mut body, &slope, &p, 1.0 / 60.0);
        }
        let start = body.pos;
        for _ in 0..300 {
            step(&mut body, &slope, &p, 1.0 / 60.0);
        }
        assert!(body.pos.distance(start) < 0.01, "moved {:?}", body.pos - start);
    }

    #[test]
    fn active_edges_on_a_flat_floor() {
        // A box over the floor's middle touches both triangles: their shared diagonal is flat (inactive), so no edge
        // axis there; the contacts are the face's.
        let (b, _) = Body::new(&cube(0.1), 1.0, Vec3::new(0.0, 0.095, 0.0), Quat::from_rotation_y(0.4)).unwrap();
        let c = world_contacts(&b, &floor(), &Params::default());
        assert!(!c.is_empty() && c.iter().all(|c| c.normal.y > 0.999), "{c:?}");
    }

    #[test]
    fn first_pass_reduction() {
        // Eight contacts with all of the first polygon in: indices 0, 2, 4, 6 and the deepest (5).
        let mut out: Vec<Contact> = (0..8).map(|k| Contact { point: Vec3::new(k as f32, 0.0, 0.0), normal: Vec3::Y, separation: if k == 5 { -0.1 } else { 0.0 } }).collect();
        reduce_all_in(&mut out, 0, true, Vec3::Y);
        let xs: Vec<f32> = out.iter().map(|c| c.point.x).collect();
        assert_eq!(xs, vec![0.0, 2.0, 4.0, 6.0, 5.0]);
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
