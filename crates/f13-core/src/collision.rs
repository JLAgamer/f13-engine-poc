//! Character collision against the world's blocking geometry (triangles: convex hulls of the game's simple collision,
//! terrain), and a walking capsule mover in the spirit of Unreal's CharacterMovementComponent: the capsule's body
//! (above step height) is pushed out of walls, the floor is found below the feet (steps up to `max_step`, unwalkable
//! slopes slide), and without a floor the character falls.
//!
//! Space: Y up; any unit (f13-game uses metres).

use glam::{Vec2, Vec3};
use std::collections::HashMap;

/// A triangle with its unit normal (counter-clockwise front face).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Tri {
    pub a: Vec3,
    pub b: Vec3,
    pub c: Vec3,
    pub n: Vec3,
}

impl Tri {
    pub fn new(a: Vec3, b: Vec3, c: Vec3) -> Option<Self> {
        let n = (b - a).cross(c - a);
        let len = n.length();
        (len > 1e-12).then(|| Tri { a, b, c, n: n / len })
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
        let mut seen: Vec<u32> = Vec::new();
        for x in x0..=x1 {
            for z in z0..=z1 {
                for &id in self.cells.get(&(x, z)).into_iter().flatten() {
                    if !single {
                        if seen.contains(&id) {
                            continue;
                        }
                        seen.push(id);
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
#[derive(Default)]
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

/// Where a character placed near `at` stands: the walkable floor nearest below a point `above` over it, or, if
/// there's none (spawned outside the map's collision), the highest surface over it from far above.
pub fn spawn_floor(world: &(impl TriSource + ?Sized), at: Vec3, above: f32, s: &WalkSettings) -> Option<Vec3> {
    let mut y = at.y + above;
    // Walk down through the surfaces under the point until a walkable one.
    for _ in 0..16 {
        match surface_below(world, at.x, at.z, y, at.y - 1000.0) {
            Some((h, n)) if n.y >= s.walkable_normal_y => return Some(Vec3::new(at.x, h, at.z)),
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
    const WALK: WalkSettings = WalkSettings { max_step: 0.45, walkable_normal_y: 0.71, gravity: 9.8 };

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
        // Under the slab: the ground below the spawn point, not the slab's top.
        assert_eq!(spawn_floor(&g, Vec3::new(0.0, 1.0, 0.0), 1.0, &WALK).unwrap().y, 0.0);
        // Spawned under the map: up onto the highest surface.
        assert!((spawn_floor(&g, Vec3::new(10.0, -5.0, 0.0), 1.0, &WALK).unwrap().y).abs() < 1e-5);
    }
}
