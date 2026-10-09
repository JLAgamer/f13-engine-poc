//! Character collision against the world's blocking geometry (triangles: convex hulls of the game's simple collision,
//! terrain), and a walking capsule mover in the spirit of Unreal's CharacterMovementComponent: the capsule's body
//! (above step height) is pushed out of walls, the floor is found below the feet (steps up to `max_step`, unwalkable
//! slopes slide), and without a floor the character falls.
//!
//! Space: Y up; any unit (f13-game uses metres).

use glam::{Vec2, Vec3};
use std::collections::HashMap;

/// A triangle with its unit normal (counter-clockwise front face) and its physical surface type (Unreal's
/// EPhysicalSurface: 0 the default; [`Tri::TERRAIN`]: the terrain, whose surface is its paint layer's).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Tri {
    pub a: Vec3,
    pub b: Vec3,
    pub c: Vec3,
    pub n: Vec3,
    pub surface: u8,
}

impl Tri {
    pub const TERRAIN: u8 = u8::MAX;

    pub fn new(a: Vec3, b: Vec3, c: Vec3) -> Option<Self> {
        let n = (b - a).cross(c - a);
        let len = n.length();
        (len > 1e-12).then(|| Tri { a, b, c, n: n / len, surface: 0 })
    }

    pub fn with_surface(self, surface: u8) -> Self {
        Tri { surface, ..self }
    }

    pub fn min(&self) -> Vec3 {
        self.a.min(self.b).min(self.c)
    }

    pub fn max(&self) -> Vec3 {
        self.a.max(self.b).max(self.c)
    }

    /// The closest point of the triangle to `p` (Ericson, Real-Time Collision Detection 5.1.5).
    pub fn closest_point(&self, p: Vec3) -> Vec3 {
        let (a, b, c) = (self.a, self.b, self.c);
        let ab = b - a;
        let ac = c - a;
        let ap = p - a;
        let d1 = ab.dot(ap);
        let d2 = ac.dot(ap);
        if d1 <= 0.0 && d2 <= 0.0 {
            return a;
        }
        let bp = p - b;
        let d3 = ab.dot(bp);
        let d4 = ac.dot(bp);
        if d3 >= 0.0 && d4 <= d3 {
            return b;
        }
        let vc = d1 * d4 - d3 * d2;
        if vc <= 0.0 && d1 >= 0.0 && d3 <= 0.0 {
            return a + ab * (d1 / (d1 - d3));
        }
        let cp = p - c;
        let d5 = ab.dot(cp);
        let d6 = ac.dot(cp);
        if d6 >= 0.0 && d5 <= d6 {
            return c;
        }
        let vb = d5 * d2 - d1 * d6;
        if vb <= 0.0 && d2 >= 0.0 && d6 <= 0.0 {
            return a + ac * (d2 / (d2 - d6));
        }
        let va = d3 * d6 - d5 * d4;
        if va <= 0.0 && (d4 - d3) >= 0.0 && (d5 - d6) >= 0.0 {
            return b + (c - b) * ((d4 - d3) / ((d4 - d3) + (d5 - d6)));
        }
        let denom = 1.0 / (va + vb + vc);
        a + ab * (vb * denom) + ac * (vc * denom)
    }

    /// Height of the triangle at (x, z), if the vertical line through it crosses the triangle (not for walls).
    pub fn height_at(&self, x: f32, z: f32) -> Option<f32> {
        if self.n.y.abs() < 1e-4 {
            return None;
        }
        let p = Vec2::new(x, z);
        let (a, b, c) = (Vec2::new(self.a.x, self.a.z), Vec2::new(self.b.x, self.b.z), Vec2::new(self.c.x, self.c.z));
        let cross = |u: Vec2, v: Vec2| u.x * v.y - u.y * v.x;
        let area = cross(b - a, c - a);
        if area.abs() < 1e-12 {
            return None;
        }
        let w1 = cross(p - a, c - a) / area;
        let w2 = cross(b - a, p - a) / area;
        let w0 = 1.0 - w1 - w2;
        let e = -1e-5;
        (w0 >= e && w1 >= e && w2 >= e).then(|| w0 * self.a.y + w1 * self.b.y + w2 * self.c.y)
    }
}

/// Outward-facing triangles of the convex hull of `points` (incremental hull). Flat or degenerate point sets are
/// given a sliver of thickness so they still block.
pub fn convex_hull(points: &[Vec3]) -> Vec<Tri> {
    let mut pts: Vec<Vec3> = Vec::with_capacity(points.len());
    let extent = points.iter().fold((Vec3::splat(f32::MAX), Vec3::splat(f32::MIN)), |(lo, hi), p| (lo.min(*p), hi.max(*p)));
    let scale = (extent.1 - extent.0).max_element().max(1e-3);
    let eps = scale * 1e-5;
    for p in points {
        if p.is_finite() && !pts.iter().any(|q| q.distance_squared(*p) < eps * eps) {
            pts.push(*p);
        }
    }
    if pts.len() < 3 {
        return Vec::new();
    }
    // Initial tetrahedron: two far-apart points, the farthest from their line, the farthest from their plane.
    let i0 = 0;
    let i1 = (0..pts.len()).max_by(|&a, &b| pts[a].distance_squared(pts[i0]).total_cmp(&pts[b].distance_squared(pts[i0]))).unwrap();
    let line = (pts[i1] - pts[i0]).normalize_or_zero();
    let off_line = |p: Vec3| (p - pts[i0]).reject_from_normalized(line).length_squared();
    let i2 = (0..pts.len()).max_by(|&a, &b| off_line(pts[a]).total_cmp(&off_line(pts[b]))).unwrap();
    if off_line(pts[i2]) < eps * eps {
        return Vec::new(); // collinear
    }
    let plane_n = (pts[i1] - pts[i0]).cross(pts[i2] - pts[i0]).normalize();
    let off_plane = |p: Vec3| (p - pts[i0]).dot(plane_n);
    let i3 = (0..pts.len()).max_by(|&a, &b| off_plane(pts[a]).abs().total_cmp(&off_plane(pts[b]).abs())).unwrap();
    if off_plane(pts[i3]).abs() < scale * 1e-3 {
        // Flat: thicken along the plane normal and hull again.
        let t = (scale * 1e-3).max(1e-3);
        let thick: Vec<Vec3> = pts.iter().flat_map(|p| [*p + plane_n * t, *p - plane_n * t]).collect();
        return convex_hull(&thick);
    }

    struct Face {
        v: [usize; 3],
        n: Vec3,
        d: f32,
        alive: bool,
    }
    let make = |pts: &[Vec3], v: [usize; 3]| {
        let n = (pts[v[1]] - pts[v[0]]).cross(pts[v[2]] - pts[v[0]]).normalize_or_zero();
        Face { v, n, d: n.dot(pts[v[0]]), alive: true }
    };
    let centroid = (pts[i0] + pts[i1] + pts[i2] + pts[i3]) / 4.0;
    let mut faces: Vec<Face> = Vec::new();
    for v in [[i0, i1, i2], [i0, i1, i3], [i0, i2, i3], [i1, i2, i3]] {
        let mut f = make(&pts, v);
        if f.n.dot(centroid) - f.d > 0.0 {
            f = make(&pts, [v[0], v[2], v[1]]);
        }
        faces.push(f);
    }
    for (pi, &p) in pts.iter().enumerate() {
        if [i0, i1, i2, i3].contains(&pi) {
            continue;
        }
        let visible: Vec<usize> = (0..faces.len()).filter(|&f| faces[f].alive && faces[f].n.dot(p) - faces[f].d > eps).collect();
        if visible.is_empty() {
            continue;
        }
        let mut edges: HashMap<(usize, usize), ()> = HashMap::new();
        for &f in &visible {
            let v = faces[f].v;
            for k in 0..3 {
                edges.insert((v[k], v[(k + 1) % 3]), ());
            }
        }
        let horizon: Vec<(usize, usize)> = edges.keys().filter(|(a, b)| !edges.contains_key(&(*b, *a))).copied().collect();
        for &f in &visible {
            faces[f].alive = false;
        }
        for (a, b) in horizon {
            faces.push(make(&pts, [a, b, pi]));
        }
    }
    faces
        .iter()
        .filter(|f| f.alive)
        .filter_map(|f| Tri::new(pts[f.v[0]], pts[f.v[1]], pts[f.v[2]]))
        .collect()
}

/// Something triangles can be fetched from by region.
pub trait TriSource {
    /// Calls `f` with every triangle whose bounds may overlap the box `min..max`.
    fn for_each_in(&self, min: Vec3, max: Vec3, f: &mut dyn FnMut(&Tri));
}

/// Static triangles bucketed in a uniform XZ grid. Very large triangles are kept in a separate list.
pub struct TriGrid {
    cell: f32,
    tris: Vec<Tri>,
    cells: HashMap<(i32, i32), Vec<u32>>,
    large: Vec<u32>,
}

impl TriGrid {
    pub fn new(cell: f32) -> Self {
        TriGrid { cell, tris: Vec::new(), cells: HashMap::new(), large: Vec::new() }
    }

    pub fn len(&self) -> usize {
        self.tris.len()
    }

    /// The bounds of every triangle.
    pub fn bounds(&self) -> (Vec3, Vec3) {
        self.tris.iter().fold((Vec3::splat(f32::MAX), Vec3::splat(f32::MIN)), |(lo, hi), t| (lo.min(t.min()), hi.max(t.max())))
    }

    pub fn is_empty(&self) -> bool {
        self.tris.is_empty()
    }

    fn key(&self, v: f32) -> i32 {
        (v / self.cell).floor() as i32
    }

    pub fn insert(&mut self, t: Tri) {
        let id = self.tris.len() as u32;
        let (lo, hi) = (t.min(), t.max());
        let (x0, x1, z0, z1) = (self.key(lo.x), self.key(hi.x), self.key(lo.z), self.key(hi.z));
        self.tris.push(t);
        if (x1 - x0 + 1) as i64 * (z1 - z0 + 1) as i64 > 64 {
            self.large.push(id);
            return;
        }
        for x in x0..=x1 {
            for z in z0..=z1 {
                self.cells.entry((x, z)).or_default().push(id);
            }
        }
    }
}

impl TriSource for TriGrid {
    fn for_each_in(&self, min: Vec3, max: Vec3, f: &mut dyn FnMut(&Tri)) {
        let overlaps = |t: &Tri| {
            let (lo, hi) = (t.min(), t.max());
            lo.x <= max.x && hi.x >= min.x && lo.y <= max.y && hi.y >= min.y && lo.z <= max.z && hi.z >= min.z
        };
        let (x0, x1, z0, z1) = (self.key(min.x), self.key(max.x), self.key(min.z), self.key(max.z));
        let single = x0 == x1 && z0 == z1;
        let mut seen: std::collections::HashSet<u32> = std::collections::HashSet::new();
        for x in x0..=x1 {
            for z in z0..=z1 {
                for &id in self.cells.get(&(x, z)).into_iter().flatten() {
                    if !single && !seen.insert(id) {
                        continue;
                    }
                    let t = &self.tris[id as usize];
                    if overlaps(t) {
                        f(t);
                    }
                }
            }
        }
        for &id in &self.large {
            let t = &self.tris[id as usize];
            if overlaps(t) {
                f(t);
            }
        }
    }
}

/// A list of triangles checked by bounds only (few, moving: doors, cars).
#[derive(Default, Clone)]
pub struct TriList {
    pub tris: Vec<Tri>,
    pub min: Vec3,
    pub max: Vec3,
}

impl TriList {
    pub fn new(tris: Vec<Tri>) -> Self {
        let (min, max) = tris.iter().fold((Vec3::splat(f32::MAX), Vec3::splat(f32::MIN)), |(lo, hi), t| (lo.min(t.min()), hi.max(t.max())));
        TriList { tris, min, max }
    }
}

impl TriSource for TriList {
    fn for_each_in(&self, min: Vec3, max: Vec3, f: &mut dyn FnMut(&Tri)) {
        if self.tris.is_empty() || self.min.x > max.x || self.max.x < min.x || self.min.y > max.y || self.max.y < min.y || self.min.z > max.z || self.max.z < min.z {
            return;
        }
        for t in &self.tris {
            f(t);
        }
    }
}

impl<T: TriSource + ?Sized> TriSource for &T {
    fn for_each_in(&self, min: Vec3, max: Vec3, f: &mut dyn FnMut(&Tri)) {
        (**self).for_each_in(min, max, f);
    }
}

impl<T: TriSource> TriSource for [T] {
    fn for_each_in(&self, min: Vec3, max: Vec3, f: &mut dyn FnMut(&Tri)) {
        for s in self {
            s.for_each_in(min, max, f);
        }
    }
}

impl<A: TriSource + ?Sized, B: TriSource + ?Sized> TriSource for (&A, &B) {
    fn for_each_in(&self, min: Vec3, max: Vec3, f: &mut dyn FnMut(&Tri)) {
        self.0.for_each_in(min, max, f);
        self.1.for_each_in(min, max, f);
    }
}

/// Where the segment `from..to` first hits a triangle (either side), as a fraction of its length.
/// The segment is tested in pieces of at most `step` so each region query stays small.
pub fn segment_hit(world: &(impl TriSource + ?Sized), from: Vec3, to: Vec3, step: f32) -> Option<f32> {
    segment_hit_normal(world, from, to, step).map(|(t, _)| t)
}

/// [`segment_hit`], with the hit triangle's normal.
pub fn segment_hit_normal(world: &(impl TriSource + ?Sized), from: Vec3, to: Vec3, step: f32) -> Option<(f32, Vec3)> {
    let d = to - from;
    let len = d.length();
    if len < 1e-6 {
        return None;
    }
    let pieces = (len / step.max(1e-3)).ceil().max(1.0) as usize;
    for i in 0..pieces {
        let (t0, t1) = (i as f32 / pieces as f32, (i + 1) as f32 / pieces as f32);
        let (a, b) = (from + d * t0, from + d * t1);
        let mut best: Option<(f32, Vec3)> = None;
        world.for_each_in(a.min(b), a.max(b), &mut |t| {
            // Möller-Trumbore, both faces.
            let (e1, e2) = (t.b - t.a, t.c - t.a);
            let p = d.cross(e2);
            let det = e1.dot(p);
            if det.abs() < 1e-12 {
                return;
            }
            let inv = 1.0 / det;
            let s = from - t.a;
            let u = s.dot(p) * inv;
            if !(0.0..=1.0).contains(&u) {
                return;
            }
            let q = s.cross(e1);
            let v = d.dot(q) * inv;
            if v < 0.0 || u + v > 1.0 {
                return;
            }
            let at = e2.dot(q) * inv;
            if (t0..=t1).contains(&at) && best.is_none_or(|b| at < b.0) {
                best = Some((at, t.n));
            }
        });
        if best.is_some() {
            return best;
        }
    }
    None
}

/// Whether a capsule whose sphere centres run from `a` to `b` with `radius` touches any triangle (checked as spheres
/// along the segment, a quarter radius apart).
pub fn capsule_blocked(world: &(impl TriSource + ?Sized), a: Vec3, b: Vec3, radius: f32) -> bool {
    let n = ((b - a).length() / (radius * 0.25).max(1e-3)).ceil().max(1.0) as usize;
    let r = Vec3::splat(radius);
    let mut hit = false;
    world.for_each_in(a.min(b) - r, a.max(b) + r, &mut |t| {
        if hit {
            return;
        }
        hit = (0..=n).any(|i| {
            let p = a.lerp(b, i as f32 / n as f32);
            t.closest_point(p).distance_squared(p) < radius * radius
        });
    });
    hit
}

/// An oriented box: its centre, its three unit axes and its half size along each.
#[derive(Clone, Copy, Debug)]
pub struct Obb {
    pub centre: Vec3,
    pub axes: [Vec3; 3],
    pub half: Vec3,
}

impl Obb {
    /// Its world-space bounds.
    pub fn aabb(&self) -> (Vec3, Vec3) {
        let e = self.axes[0].abs() * self.half.x + self.axes[1].abs() * self.half.y + self.axes[2].abs() * self.half.z;
        (self.centre - e, self.centre + e)
    }

    /// Whether it overlaps a triangle (the separating axis test: the box's axes, the triangle's normal and the nine
    /// edge cross products).
    pub fn overlaps_tri(&self, t: &Tri) -> bool {
        let v = [t.a - self.centre, t.b - self.centre, t.c - self.centre];
        let edges = [v[1] - v[0], v[2] - v[1], v[0] - v[2]];
        let separated = |axis: Vec3| {
            if axis.length_squared() < 1e-12 {
                return false;
            }
            let p = v.map(|p| p.dot(axis));
            let r = self.half.x * self.axes[0].dot(axis).abs() + self.half.y * self.axes[1].dot(axis).abs() + self.half.z * self.axes[2].dot(axis).abs();
            p[0].min(p[1]).min(p[2]) > r || p[0].max(p[1]).max(p[2]) < -r
        };
        if self.axes.iter().any(|&a| separated(a)) || separated(edges[0].cross(edges[1])) {
            return false;
        }
        !self.axes.iter().any(|&a| edges.iter().any(|&e| separated(a.cross(e))))
    }
}

/// Whether an oriented box overlaps any triangle (an overlap query's blocking hit).
pub fn box_blocked(world: &(impl TriSource + ?Sized), b: &Obb) -> bool {
    let (min, max) = b.aabb();
    let mut hit = false;
    world.for_each_in(min, max, &mut |t| hit = hit || b.overlaps_tri(t));
    hit
}

/// Whether a sphere overlaps a convex hull (its triangles, normals outward): the centre inside it, or a triangle
/// within `radius`.
pub fn sphere_overlaps_hull(hull: &[Tri], centre: Vec3, radius: f32) -> bool {
    !hull.is_empty()
        && (hull.iter().all(|t| (centre - t.a).dot(t.n) <= 0.0) || hull.iter().any(|t| t.closest_point(centre).distance_squared(centre) < radius * radius))
}

/// Where a sphere of `radius` moving from `from` to `to` first touches a triangle, as a fraction of the way (0 when it
/// starts touching one): the spring arm's probe (SweepSingleByChannel with a sphere). Found in steps of a quarter
/// radius, then refined by halving.
pub fn sphere_sweep(world: &(impl TriSource + ?Sized), from: Vec3, to: Vec3, radius: f32) -> Option<f32> {
    let r = Vec3::splat(radius);
    let mut tris = Vec::new();
    world.for_each_in(from.min(to) - r, from.max(to) + r, &mut |t| tris.push(*t));
    let touches = |p: Vec3| tris.iter().any(|t| t.closest_point(p).distance_squared(p) < radius * radius);
    if touches(from) {
        return Some(0.0);
    }
    let d = to - from;
    let n = (d.length() / (radius * 0.25).max(1e-3)).ceil().max(1.0) as usize;
    let mut clear = 0.0;
    for i in 1..=n {
        let t = i as f32 / n as f32;
        if touches(from + d * t) {
            let mut hit = t;
            for _ in 0..12 {
                let mid = (clear + hit) * 0.5;
                if touches(from + d * mid) { hit = mid } else { clear = mid }
            }
            return Some(clear);
        }
        clear = t;
    }
    None
}

/// Unreal's walking capsule rests this far above its floor (UCharacterMovementComponent's MAX_FLOOR_DIST, 2.4 cm), so
/// sweeps from rest don't hit it.
pub const MAX_FLOOR_DIST_M: f32 = 0.024;

/// Where an upright capsule (feet at `from`, `radius`, `half_height`) moving to `to` first touches a triangle, as a
/// fraction of the way: SetActorTransform's sweep. It's swept from [`MAX_FLOOR_DIST_M`] over its floor, and
/// triangles it already touches at the start don't stop it (a sweep moving out of contact goes on).
pub fn capsule_sweep(world: &(impl TriSource + ?Sized), from: Vec3, to: Vec3, radius: f32, half_height: f32) -> Option<f32> {
    let lift = Vec3::Y * MAX_FLOOR_DIST_M;
    let (lo, hi) = (lift + Vec3::Y * radius, lift + Vec3::Y * (2.0 * half_height - radius).max(radius));
    let r = Vec3::splat(radius);
    let mut tris = Vec::new();
    world.for_each_in(from.min(to) + lo - r, from.max(to) + hi + r, &mut |t| tris.push(*t));
    let touches = |t: &Tri, p: Vec3| {
        let (a, b) = (p + lo, p + hi);
        let n = ((b - a).length() / (radius * 0.25).max(1e-3)).ceil().max(1.0) as usize;
        (0..=n).any(|i| {
            let q = a.lerp(b, i as f32 / n as f32);
            t.closest_point(q).distance_squared(q) < radius * radius
        })
    };
    tris.retain(|t| !touches(t, from));
    let blocked = |p: Vec3| tris.iter().any(|t| touches(t, p));
    let d = to - from;
    let n = (d.length() / (radius * 0.25).max(1e-3)).ceil().max(1.0) as usize;
    let mut clear = 0.0;
    for i in 1..=n {
        let t = i as f32 / n as f32;
        if blocked(from + d * t) {
            let mut hit = t;
            for _ in 0..12 {
                let mid = (clear + hit) * 0.5;
                if blocked(from + d * mid) { hit = mid } else { clear = mid }
            }
            return Some(clear);
        }
        clear = t;
    }
    None
}

/// The highest surface crossing the vertical line at (x, z) between `bottom` and `top`: (height, normal).
pub fn surface_below(world: &(impl TriSource + ?Sized), x: f32, z: f32, top: f32, bottom: f32) -> Option<(f32, Vec3)> {
    let mut best: Option<(f32, Vec3)> = None;
    world.for_each_in(Vec3::new(x, bottom, z), Vec3::new(x, top, z), &mut |t| {
        if let Some(y) = t.height_at(x, z) {
            if y <= top && y >= bottom && best.is_none_or(|(b, _)| y > b) {
                // Facing down (the underside of a hull): use the hull's top face instead, found separately.
                best = Some((y, if t.n.y < 0.0 { -t.n } else { t.n }));
            }
        }
    });
    best
}

/// The highest triangle crossing the vertical line at (x, z) between `bottom` and `top`: (height, its surface).
pub fn ground_surface(world: &(impl TriSource + ?Sized), x: f32, z: f32, top: f32, bottom: f32) -> Option<(f32, u8)> {
    let mut best: Option<(f32, u8)> = None;
    world.for_each_in(Vec3::new(x, bottom, z), Vec3::new(x, top, z), &mut |t| {
        if let Some(y) = t.height_at(x, z) {
            if y <= top && y >= bottom && best.is_none_or(|(b, _)| y > b) {
                best = Some((y, t.surface));
            }
        }
    });
    best
}

/// The walking capsule: origin at the feet.
#[derive(Debug, Clone, Copy)]
pub struct Capsule {
    pub radius: f32,
    pub half_height: f32,
}

#[derive(Debug, Clone, Copy)]
pub struct WalkSettings {
    /// Highest step walked up without being blocked (Unreal MaxStepHeight).
    pub max_step: f32,
    /// Minimum normal Y of a walkable floor (cos of Unreal's WalkableFloorAngle).
    pub walkable_normal_y: f32,
    /// Downward acceleration while falling.
    pub gravity: f32,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct MoveResult {
    /// Standing on a walkable floor after the move.
    pub grounded: bool,
    /// Horizontal normals of the walls hit (to cancel velocity into them), summed.
    pub wall_normal: Vec2,
}

/// Pushes the capsule's body (the part above the step height) out of everything it overlaps, horizontally.
/// Returns the summed push directions.
fn depenetrate(world: &(impl TriSource + ?Sized), feet: &mut Vec3, cap: &Capsule, s: &WalkSettings) -> Vec2 {
    let r = cap.radius;
    let bottom = s.max_step + r;
    let top = (2.0 * cap.half_height - r).max(bottom);
    let count = (((top - bottom) / r).ceil() as usize).max(1) + 1;
    let mut normals = Vec2::ZERO;
    for _ in 0..4 {
        let mut moved = false;
        for i in 0..count {
            let h = bottom + (top - bottom) * i as f32 / (count - 1).max(1) as f32;
            let center = *feet + Vec3::Y * h;
            let mut push = Vec2::ZERO;
            let mut deepest = 0.0f32;
            world.for_each_in(center - Vec3::splat(r), center + Vec3::splat(r), &mut |t| {
                let cp = t.closest_point(center);
                let d = center - cp;
                let dist = d.length();
                if dist >= r {
                    return;
                }
                let dir = if dist > 1e-6 { d / dist } else { t.n };
                let horizontal = Vec2::new(dir.x, dir.z);
                let hl = horizontal.length();
                if hl < 0.2 {
                    return; // a ceiling or a floor under the body: not a wall
                }
                let depth = (r - dist) / hl;
                if depth > deepest {
                    deepest = depth;
                    push = horizontal / hl * depth;
                }
            });
            if deepest > 1e-5 {
                feet.x += push.x;
                feet.z += push.y;
                normals += push.normalize_or_zero();
                moved = true;
            }
        }
        if !moved {
            break;
        }
    }
    normals
}

/// The floor under the capsule: the highest surface below `feet.y + max_step`, probed at the centre and around it
/// (the rounded bottom stands on edges). Walkable floors only, except at the centre, which never lets the feet sink
/// into a surface (unwalkable slopes are then slid off by the body).
fn find_floor(world: &(impl TriSource + ?Sized), feet: Vec3, cap: &Capsule, s: &WalkSettings, down: f32) -> Option<(f32, bool)> {
    let top = feet.y + s.max_step;
    let bottom = feet.y - down;
    let mut best: Option<(f32, bool)> = None;
    let ring = cap.radius * 0.6;
    for k in 0..9 {
        let offset = if k == 0 {
            Vec2::ZERO
        } else {
            let a = (k - 1) as f32 * std::f32::consts::TAU / 8.0;
            Vec2::new(a.cos(), a.sin()) * ring
        };
        if let Some((y, n)) = surface_below(world, feet.x + offset.x, feet.z + offset.y, top, bottom) {
            let walkable = n.y >= s.walkable_normal_y;
            if (walkable || k == 0) && best.is_none_or(|(b, _)| y > b) {
                best = Some((y, walkable));
            }
        }
    }
    best
}

/// Moves the capsule by `delta` (horizontal, XZ) over one tick of `dt`, with `vel_y` its vertical speed (updated).
/// `was_grounded` lets a walking character follow the floor down steps and slopes.
pub fn move_capsule(
    world: &(impl TriSource + ?Sized),
    feet: &mut Vec3,
    vel_y: &mut f32,
    was_grounded: bool,
    delta: Vec2,
    dt: f32,
    cap: &Capsule,
    s: &WalkSettings,
) -> MoveResult {
    let mut result = MoveResult::default();
    // Sub-steps no longer than half the radius, so thin walls can't be skipped.
    let steps = ((delta.length() / (cap.radius * 0.5)).ceil() as usize).max(1);
    let step = delta / steps as f32;
    for _ in 0..steps {
        feet.x += step.x;
        feet.z += step.y;
        result.wall_normal += depenetrate(world, feet, cap, s);
        // Walking: follow the floor (up a step, down a step or slope).
        if was_grounded && *vel_y <= 0.0 {
            if let Some((y, walkable)) = find_floor(world, *feet, cap, s, s.max_step) {
                feet.y = y;
                result.grounded = walkable;
            }
        }
    }
    if !(was_grounded && result.grounded) {
        // Falling (or stepped off a ledge).
        *vel_y -= s.gravity * dt;
        let next = feet.y + *vel_y * dt;
        match find_floor(world, *feet, cap, s, (feet.y - next).max(0.0)) {
            Some((y, walkable)) if y >= next => {
                feet.y = y;
                result.grounded = walkable;
                if walkable || *vel_y < 0.0 {
                    *vel_y = 0.0;
                }
            }
            _ => {
                feet.y = next;
                result.grounded = false;
            }
        }
    } else {
        *vel_y = 0.0;
    }
    result.wall_normal += depenetrate(world, feet, cap, s);
    result
}

/// Where a character placed near `at` stands: the walkable floor nearest below a point `above` over it with
/// `headroom` free above it (not the top of a ceiling slab, under a roof), or, if there's none (spawned outside the
/// map's collision), the highest surface over it from far above.
pub fn spawn_floor(world: &(impl TriSource + ?Sized), at: Vec3, above: f32, headroom: f32, s: &WalkSettings) -> Option<Vec3> {
    let mut y = at.y + above;
    // Walk down through the surfaces under the point until a walkable one with room above it.
    for _ in 0..16 {
        match surface_below(world, at.x, at.z, y, at.y - 1000.0) {
            Some((h, n)) if n.y >= s.walkable_normal_y && surface_below(world, at.x, at.z, h + headroom, h + 0.05).is_none() => {
                return Some(Vec3::new(at.x, h, at.z));
            }
            Some((h, _)) => y = h - 1e-3,
            None => break,
        }
    }
    surface_below(world, at.x, at.z, at.y + 10_000.0, at.y - 10_000.0).map(|(h, _)| Vec3::new(at.x, h, at.z))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cube(center: Vec3, half: Vec3) -> Vec<Tri> {
        let pts: Vec<Vec3> = (0..8)
            .map(|i| center + Vec3::new(if i & 1 != 0 { half.x } else { -half.x }, if i & 2 != 0 { half.y } else { -half.y }, if i & 4 != 0 { half.z } else { -half.z }))
            .collect();
        convex_hull(&pts)
    }

    #[test]
    fn sphere_overlaps_hull_inside_and_near() {
        let c = cube(Vec3::ZERO, Vec3::splat(1.0));
        assert!(sphere_overlaps_hull(&c, Vec3::ZERO, 0.1));
        assert!(sphere_overlaps_hull(&c, Vec3::new(2.4, 0.0, 0.0), 1.5));
        assert!(!sphere_overlaps_hull(&c, Vec3::new(2.6, 0.0, 0.0), 1.5));
        assert!(!sphere_overlaps_hull(&[], Vec3::ZERO, 1.0));
    }

    #[test]
    fn box_overlaps_tris() {
        let g = ground();
        let axis = [Vec3::X, Vec3::Y, Vec3::Z];
        assert!(box_blocked(&g, &Obb { centre: Vec3::new(0.0, 0.5, 0.0), axes: axis, half: Vec3::splat(1.0) }));
        assert!(!box_blocked(&g, &Obb { centre: Vec3::new(0.0, 1.3, 0.0), axes: axis, half: Vec3::splat(1.0) }));
        // Turned 45 degrees about Z: its corner reaches sqrt(2) down.
        let q = glam::Quat::from_rotation_z(std::f32::consts::FRAC_PI_4);
        let turned = [q * Vec3::X, q * Vec3::Y, q * Vec3::Z];
        assert!(box_blocked(&g, &Obb { centre: Vec3::new(0.0, 1.3, 0.0), axes: turned, half: Vec3::splat(1.0) }));
        assert!(!box_blocked(&g, &Obb { centre: Vec3::new(0.0, 1.5, 0.0), axes: turned, half: Vec3::splat(1.0) }));
        // A wall's triangle beside the box, an edge-on miss and a hit.
        let wall = Tri::new(Vec3::new(3.0, 0.0, -5.0), Vec3::new(3.0, 5.0, 0.0), Vec3::new(3.0, 0.0, 5.0)).unwrap();
        assert!(!Obb { centre: Vec3::new(1.9, 1.0, 0.0), axes: axis, half: Vec3::splat(1.0) }.overlaps_tri(&wall));
        assert!(Obb { centre: Vec3::new(2.1, 1.0, 0.0), axes: axis, half: Vec3::splat(1.0) }.overlaps_tri(&wall));
    }

    fn ground() -> TriGrid {
        let mut g = TriGrid::new(2.0);
        let s = 50.0;
        for t in [
            Tri::new(Vec3::new(-s, 0.0, -s), Vec3::new(-s, 0.0, s), Vec3::new(s, 0.0, s)),
            Tri::new(Vec3::new(-s, 0.0, -s), Vec3::new(s, 0.0, s), Vec3::new(s, 0.0, -s)),
        ]
        .into_iter()
        .flatten()
        {
            g.insert(t);
        }
        g
    }

    const CAP: Capsule = Capsule { radius: 0.34, half_height: 0.88 };

    #[test]
    fn segments_stop_at_walls() {
        let mut g = ground();
        for t in cube(Vec3::new(10.0, 1.0, 0.0), Vec3::new(0.5, 1.0, 5.0)) {
            g.insert(t);
        }
        let eye = Vec3::new(0.0, 1.5, 0.0);
        let hit = segment_hit(&g, eye, Vec3::new(20.0, 1.5, 0.0), 3.0).expect("the wall blocks");
        assert!((hit * 20.0 - 9.5).abs() < 1e-3, "{hit}");
        // Over the wall, and along the floor without touching it.
        assert!(segment_hit(&g, eye + Vec3::Y * 2.0, Vec3::new(20.0, 3.5, 0.0), 3.0).is_none());
        assert!(segment_hit(&g, eye, Vec3::new(0.0, 1.5, 20.0), 3.0).is_none());
    }
    #[test]
    fn capsule_sweeps_stop_at_walls_not_the_floor() {
        let mut g = ground();
        for t in cube(Vec3::new(5.0, 1.0, 0.0), Vec3::new(0.5, 1.0, 5.0)) {
            g.insert(t);
        }
        // Along the floor she stands on: clear.
        assert!(capsule_sweep(&g, Vec3::ZERO, Vec3::new(0.0, 0.0, 3.0), 0.34, 0.82).is_none());
        // Into the wall: stopped with her radius to spare (the wall's face at x 4.5).
        let t = capsule_sweep(&g, Vec3::ZERO, Vec3::new(8.0, 0.0, 0.0), 0.34, 0.82).expect("the wall blocks");
        assert!((t * 8.0 - (4.5 - 0.34)).abs() < 0.01, "{}", t * 8.0);
        // Touching it at the start, moving away: clear.
        assert!(capsule_sweep(&g, Vec3::new(4.2, 0.0, 0.0), Vec3::new(2.0, 0.0, 0.0), 0.34, 0.82).is_none());
    }

    const WALK: WalkSettings = WalkSettings { max_step: 0.45, walkable_normal_y: 0.71, gravity: 9.8 };

    #[test]
    fn capsules_are_blocked_by_walls_not_the_floor_under_them() {
        let mut g = ground();
        for t in cube(Vec3::new(2.0, 1.0, 0.0), Vec3::new(0.5, 1.0, 5.0)) {
            g.insert(t);
        }
        let clear = |x: f32| !capsule_blocked(&g, Vec3::new(x, 0.35, 0.0), Vec3::new(x, 1.5, 0.0), 0.34);
        assert!(clear(0.0), "standing just off the floor");
        assert!(!clear(1.3), "touching the wall's face (x = 1.5)");
        assert!(clear(5.0), "past the wall");
    }

    #[test]
    fn hull_of_a_cube_has_12_outward_faces() {
        let tris = cube(Vec3::ZERO, Vec3::ONE);
        assert_eq!(tris.len(), 12);
        for t in &tris {
            let c = (t.a + t.b + t.c) / 3.0;
            assert!(t.n.dot(c) > 0.0, "inward face {t:?}");
        }
    }

    #[test]
    fn hull_ignores_interior_points_and_handles_flat_sets() {
        let mut pts: Vec<Vec3> = (0..8).map(|i| Vec3::new((i & 1) as f32, ((i >> 1) & 1) as f32, ((i >> 2) & 1) as f32)).collect();
        pts.push(Vec3::splat(0.5));
        assert_eq!(convex_hull(&pts).len(), 12);
        let flat = [Vec3::ZERO, Vec3::X, Vec3::Z, Vec3::new(1.0, 0.0, 1.0)];
        assert!(!convex_hull(&flat).is_empty());
    }

    #[test]
    fn closest_point_regions() {
        let t = Tri::new(Vec3::ZERO, Vec3::X, Vec3::Z).unwrap();
        assert!(t.closest_point(Vec3::new(0.2, 1.0, 0.2)).distance(Vec3::new(0.2, 0.0, 0.2)) < 1e-6);
        assert_eq!(t.closest_point(Vec3::new(-1.0, 0.0, -1.0)), Vec3::ZERO);
        assert_eq!(t.height_at(0.2, 0.2), Some(0.0));
        assert_eq!(t.height_at(2.0, 2.0), None);
    }

    #[test]
    fn walks_on_flat_ground_and_stops_at_a_wall() {
        let mut g = ground();
        for t in cube(Vec3::new(2.0, 1.0, 0.0), Vec3::new(0.1, 1.0, 2.0)) {
            g.insert(t);
        }
        let mut feet = Vec3::new(0.0, 0.0, 0.0);
        let mut vy = 0.0;
        let mut grounded = true;
        let mut normal = Vec2::ZERO;
        for _ in 0..100 {
            let r = move_capsule(&g, &mut feet, &mut vy, grounded, Vec2::new(0.05, 0.0), 1.0 / 60.0, &CAP, &WALK);
            grounded = r.grounded;
            normal += r.wall_normal;
        }
        assert!(grounded);
        assert!(feet.y.abs() < 1e-4, "{feet}");
        // Wall face at x = 1.9: the capsule stops a radius before it.
        assert!((feet.x - (1.9 - CAP.radius)).abs() < 0.02, "{feet}");
        assert!(normal.x < 0.0);
    }

    #[test]
    fn steps_up_low_ledges_but_not_high_ones() {
        let mut g = ground();
        for t in cube(Vec3::new(2.5, 0.15, 0.0), Vec3::new(1.5, 0.15, 2.0)) {
            g.insert(t); // 30 cm step from x = 1 to the block
        }
        for t in cube(Vec3::new(5.0, 0.5, 0.0), Vec3::new(1.0, 0.5, 2.0)) {
            g.insert(t); // 1 m block from x = 4
        }
        let (mut feet, mut vy, mut grounded) = (Vec3::ZERO, 0.0, true);
        for _ in 0..200 {
            grounded = move_capsule(&g, &mut feet, &mut vy, grounded, Vec2::new(0.05, 0.0), 1.0 / 60.0, &CAP, &WALK).grounded;
        }
        assert!((feet.y - 0.3).abs() < 1e-3, "on the step: {feet}");
        assert!(feet.x < 4.0 - CAP.radius + 0.02, "stopped by the block: {feet}");
    }

    #[test]
    fn falls_off_a_ledge_and_lands() {
        let mut g = ground();
        for t in cube(Vec3::new(0.0, 1.0, 0.0), Vec3::new(1.0, 1.0, 1.0)) {
            g.insert(t); // a 2 m high platform
        }
        let (mut feet, mut vy, mut grounded) = (Vec3::new(0.0, 2.0, 0.0), 0.0, true);
        let mut airborne = false;
        for _ in 0..300 {
            let r = move_capsule(&g, &mut feet, &mut vy, grounded, Vec2::new(0.03, 0.0), 1.0 / 60.0, &CAP, &WALK);
            airborne |= !r.grounded;
            grounded = r.grounded;
        }
        assert!(airborne);
        assert!(grounded && feet.y.abs() < 1e-4, "{feet}");
    }

    #[test]
    fn spawn_floor_finds_the_walkable_floor_or_the_top_surface() {
        let mut g = ground();
        for t in cube(Vec3::new(0.0, 3.0, 0.0), Vec3::new(1.0, 0.1, 1.0)) {
            g.insert(t); // a slab overhead
        }
        for t in cube(Vec3::new(0.0, 3.5, 0.0), Vec3::new(1.0, 0.1, 1.0)) {
            g.insert(t); // and a roof 0.3 m over it
        }
        // Under the slab: the ground below the spawn point, not the slab's top.
        assert_eq!(spawn_floor(&g, Vec3::new(0.0, 1.0, 0.0), 1.0, 1.7, &WALK).unwrap().y, 0.0);
        // From above the slab too: no room on it under the roof.
        assert_eq!(spawn_floor(&g, Vec3::new(0.0, 1.0, 0.0), 2.3, 1.7, &WALK).unwrap().y, 0.0);
        // Spawned under the map: up onto the highest surface.
        assert!((spawn_floor(&g, Vec3::new(10.0, -5.0, 0.0), 1.0, 1.7, &WALK).unwrap().y).abs() < 1e-5);
    }
}
