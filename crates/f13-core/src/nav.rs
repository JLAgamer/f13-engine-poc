//! Paths for computer-controlled characters over the world's collision (the game's AI uses Recast navmeshes; this
//! is a stand-in built from the same blocking geometry the characters collide with).
//!
//! The ground is a grid of cells (`cell` metres). A cell is standable when there is a walkable floor under its centre
//! within a step of the neighbour it is reached from and the capsule fits there (nothing above its step height
//! overlaps it, as for [`crate::collision::move_capsule`]). Cells are evaluated lazily and cached: the static world
//! doesn't change. Paths are found with A* under a node budget; when the goal isn't reached within it, the path leads
//! to the explored cell nearest the goal, so a caller re-planning as it goes still makes progress.

use crate::collision::{surface_below, Capsule, TriSource, WalkSettings};
use glam::{Vec2, Vec3};
use std::cmp::Ordering;
use std::collections::{BinaryHeap, HashMap};

/// Whether a capsule standing at `feet` overlaps nothing above its step height (what
/// [`crate::collision::move_capsule`] would push it out of).
pub fn capsule_fits(world: &(impl TriSource + ?Sized), feet: Vec3, cap: &Capsule, s: &WalkSettings) -> bool {
    let r = cap.radius;
    let bottom = s.max_step + r;
    let top = (2.0 * cap.half_height - r).max(bottom);
    let count = (((top - bottom) / r).ceil() as usize).max(1) + 1;
    for i in 0..count {
        let center = feet + Vec3::Y * (bottom + (top - bottom) * i as f32 / (count - 1).max(1) as f32);
        let mut hit = false;
        world.for_each_in(center - Vec3::splat(r), center + Vec3::splat(r), &mut |t| {
            if hit {
                return;
            }
            let d = center - t.closest_point(center);
            let dist = d.length();
            if dist >= r * 0.98 {
                return;
            }
            let dir = if dist > 1e-6 { d / dist } else { t.n };
            // Ceilings and floors under the body aren't walls (as for the mover).
            hit = Vec2::new(dir.x, dir.z).length() >= 0.2;
        });
        if hit {
            return false;
        }
    }
    true
}

/// The walkable floor at (x, z) reachable from height `from_y`: at most a step up, `max_drop` down.
fn floor_near(world: &(impl TriSource + ?Sized), x: f32, z: f32, from_y: f32, max_drop: f32, s: &WalkSettings) -> Option<f32> {
    match surface_below(world, x, z, from_y + s.max_step, from_y - max_drop) {
        Some((y, n)) if n.y >= s.walkable_normal_y => Some(y),
        _ => None,
    }
}

/// Farthest goal a straight walk is checked for before searching.
const STRAIGHT_CHECK_M: f32 = 30.0;

/// How far down a path may step from one cell to the next (steeper than this is a ledge to avoid).
const MAX_DROP: f32 = 0.6;

/// Whether a capsule can walk straight from `a` to `b` (feet positions): floor all along, nothing in the way.
pub fn clear_line(world: &(impl TriSource + ?Sized), a: Vec3, b: Vec3, cap: &Capsule, s: &WalkSettings) -> bool {
    let flat = Vec2::new(b.x - a.x, b.z - a.z);
    let samples = ((flat.length() / (cap.radius * 0.5)).ceil() as usize).max(1);
    let mut y = a.y;
    for i in 1..=samples {
        let f = i as f32 / samples as f32;
        let (x, z) = (a.x + flat.x * f, a.z + flat.y * f);
        let Some(fy) = floor_near(world, x, z, y, MAX_DROP, s) else { return false };
        y = fy;
        if !capsule_fits(world, Vec3::new(x, y, z), cap, s) {
            return false;
        }
    }
    (y - b.y).abs() < 1.0
}

/// Standable cells, evaluated on demand and cached.
pub struct NavGrid {
    pub cell: f32,
    pub cap: Capsule,
    pub walk: WalkSettings,
    /// (cell x, cell z, approach height in quarter metres) -> the floor height there, if standable.
    cache: HashMap<(i32, i32, i32), Option<f32>>,
}

#[derive(Clone, Copy, PartialEq)]
struct Open {
    f: f32,
    node: usize,
}

impl Eq for Open {}

impl Ord for Open {
    fn cmp(&self, other: &Self) -> Ordering {
        // Min-heap on f.
        other.f.total_cmp(&self.f)
    }
}

impl PartialOrd for Open {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

struct Node {
    cell: (i32, i32),
    y: f32,
    g: f32,
    parent: Option<usize>,
}

impl NavGrid {
    pub fn new(cell: f32, cap: Capsule, walk: WalkSettings) -> Self {
        NavGrid { cell, cap, walk, cache: HashMap::new() }
    }

    pub fn cached(&self) -> usize {
        self.cache.len()
    }

    fn cell_of(&self, p: Vec3) -> (i32, i32) {
        ((p.x / self.cell).floor() as i32, (p.z / self.cell).floor() as i32)
    }

    fn center(&self, c: (i32, i32)) -> Vec2 {
        Vec2::new((c.0 as f32 + 0.5) * self.cell, (c.1 as f32 + 0.5) * self.cell)
    }

    /// The floor of cell `c` reached from height `from_y`, if a capsule can stand there.
    fn standable(&mut self, world: &(impl TriSource + ?Sized), c: (i32, i32), from_y: f32) -> Option<f32> {
        let key = (c.0, c.1, (from_y * 4.0).round() as i32);
        if let Some(v) = self.cache.get(&key) {
            return *v;
        }
        let p = self.center(c);
        let v = floor_near(world, p.x, p.y, from_y, MAX_DROP, &self.walk).filter(|&y| capsule_fits(world, Vec3::new(p.x, y, p.y), &self.cap, &self.walk));
        self.cache.insert(key, v);
        v
    }

    /// A path of feet positions from `from` toward `to` (excluding `from`, ending at `to` when it was reached),
    /// exploring at most `budget` cells. `reached` says whether it gets to `to`.
    pub fn find_path(&mut self, world: &(impl TriSource + ?Sized), from: Vec3, to: Vec3, budget: usize) -> (Vec<Vec3>, bool) {
        // A straight walk when it's clear (only tried up close: the check samples the whole line).
        if from.distance(to) < STRAIGHT_CHECK_M && clear_line(world, from, to, &self.cap, &self.walk) {
            return (vec![to], true);
        }
        let goal = self.cell_of(to);
        let goal_xz = Vec2::new(to.x, to.z);
        let h = |p: Vec2| p.distance(goal_xz);
        let mut nodes = vec![Node { cell: self.cell_of(from), y: from.y, g: 0.0, parent: None }];
        let mut best: HashMap<(i32, i32), (usize, f32)> = HashMap::new();
        best.insert(nodes[0].cell, (0, 0.0));
        let mut open = BinaryHeap::new();
        open.push(Open { f: h(Vec2::new(from.x, from.z)), node: 0 });
        let mut closest = (0usize, h(Vec2::new(from.x, from.z)));
        let mut found = None;
        let mut expanded = 0;
        while let Some(Open { node, .. }) = open.pop() {
            let (cell, y, g) = (nodes[node].cell, nodes[node].y, nodes[node].g);
            if best.get(&cell).is_some_and(|&(n, _)| n != node) {
                continue; // a cheaper way here was found after this entry was queued
            }
            let here = self.center(cell);
            if cell == goal || (here.distance(goal_xz) < self.cell * 1.5 && (y - to.y).abs() < 1.0) {
                found = Some(node);
                break;
            }
            expanded += 1;
            if expanded > budget {
                break;
            }
            for (dx, dz) in [(1, 0), (-1, 0), (0, 1), (0, -1), (1, 1), (1, -1), (-1, 1), (-1, -1)] {
                let next = (cell.0 + dx, cell.1 + dz);
                let Some(ny) = self.standable(world, next, y) else { continue };
                if dx != 0 && dz != 0 {
                    // Diagonal: both side cells must be open too, or the capsule would cut a corner.
                    if self.standable(world, (cell.0 + dx, cell.1), y).is_none() || self.standable(world, (cell.0, cell.1 + dz), y).is_none() {
                        continue;
                    }
                }
                let p = self.center(next);
                let ng = g + p.distance(here) + (ny - y).abs();
                if best.get(&next).is_some_and(|&(_, bg)| bg <= ng) {
                    continue;
                }
                let id = nodes.len();
                nodes.push(Node { cell: next, y: ny, g: ng, parent: Some(node) });
                best.insert(next, (id, ng));
                let hn = h(p);
                if hn < closest.1 {
                    closest = (id, hn);
                }
                open.push(Open { f: ng + hn, node: id });
            }
        }
        let reached = found.is_some();
        let mut at = found.unwrap_or(closest.0);
        let mut cells = Vec::new();
        while let Some(parent) = nodes[at].parent {
            let c = self.center(nodes[at].cell);
            cells.push(Vec3::new(c.x, nodes[at].y, c.y));
            at = parent;
        }
        cells.reverse();
        if reached {
            if let Some(last) = cells.last_mut() {
                *last = to;
            } else {
                cells.push(to);
            }
        }
        (self.smooth(world, from, cells), reached)
    }

    /// Drops waypoints that can be walked past in a straight line.
    fn smooth(&self, world: &(impl TriSource + ?Sized), from: Vec3, points: Vec<Vec3>) -> Vec<Vec3> {
        let mut out = Vec::new();
        let mut at = from;
        let mut i = 0;
        while i < points.len() {
            // The farthest of the next few points reachable straight from `at`.
            let mut j = i;
            for k in (i + 1..points.len().min(i + 12)).rev() {
                if clear_line(world, at, points[k], &self.cap, &self.walk) {
                    j = k;
                    break;
                }
            }
            out.push(points[j]);
            at = points[j];
            i = j + 1;
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::collision::{convex_hull, Tri, TriGrid};

    const CAP: Capsule = Capsule { radius: 0.34, half_height: 0.88 };
    const WALK: WalkSettings = WalkSettings { max_step: 0.45, walkable_normal_y: 0.71, gravity: 9.8 };

    fn cube(center: Vec3, half: Vec3) -> Vec<Tri> {
        let pts: Vec<Vec3> = (0..8)
            .map(|i| center + Vec3::new(if i & 1 != 0 { half.x } else { -half.x }, if i & 2 != 0 { half.y } else { -half.y }, if i & 4 != 0 { half.z } else { -half.z }))
            .collect();
        convex_hull(&pts)
    }

    fn ground() -> TriGrid {
        let mut g = TriGrid::new(2.0);
        let s = 30.0;
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

    /// A wall along x = 0 from z = -10 to 10, with a 1.2 m gap at z = 5..6.2.
    fn wall_with_gap() -> TriGrid {
        let mut g = ground();
        for t in cube(Vec3::new(0.0, 1.5, -2.5), Vec3::new(0.1, 1.5, 7.5)).into_iter().chain(cube(Vec3::new(0.0, 1.5, 8.1), Vec3::new(0.1, 1.5, 1.9))) {
            g.insert(t);
        }
        g
    }

    #[test]
    fn capsule_fits_away_from_walls_only() {
        let g = wall_with_gap();
        assert!(capsule_fits(&g, Vec3::new(-2.0, 0.0, 0.0), &CAP, &WALK));
        assert!(!capsule_fits(&g, Vec3::new(-0.3, 0.0, 0.0), &CAP, &WALK));
        assert!(capsule_fits(&g, Vec3::new(0.0, 0.0, 5.6), &CAP, &WALK), "the gap fits a capsule");
    }

    #[test]
    fn straight_line_when_nothing_is_in_the_way() {
        let g = ground();
        let mut nav = NavGrid::new(0.5, CAP, WALK);
        let (path, reached) = nav.find_path(&g, Vec3::new(-5.0, 0.0, 0.0), Vec3::new(5.0, 0.0, 3.0), 2000);
        assert!(reached);
        assert_eq!(path, vec![Vec3::new(5.0, 0.0, 3.0)]);
    }

    #[test]
    fn goes_through_the_gap() {
        let g = wall_with_gap();
        let mut nav = NavGrid::new(0.5, CAP, WALK);
        let (from, to) = (Vec3::new(-3.0, 0.0, 0.0), Vec3::new(3.0, 0.0, 0.0));
        let (path, reached) = nav.find_path(&g, from, to, 5000);
        assert!(reached, "{path:?}");
        assert_eq!(*path.last().unwrap(), to);
        // Every leg is walkable, and one of them crosses the wall through the gap.
        let mut at = from;
        for p in &path {
            assert!(clear_line(&g, at, *p, &CAP, &WALK), "{at} -> {p}");
            at = *p;
        }
        assert!(path.iter().any(|p| p.z > 4.5 && p.z < 6.7 && p.x.abs() < 1.5), "{path:?}");
    }

    #[test]
    fn unreachable_goal_gives_a_partial_path_toward_it() {
        let mut g = ground();
        // A closed box around the goal.
        for (c, h) in [
            (Vec3::new(10.0, 1.5, -3.0), Vec3::new(3.0, 1.5, 0.1)),
            (Vec3::new(10.0, 1.5, 3.0), Vec3::new(3.0, 1.5, 0.1)),
            (Vec3::new(7.0, 1.5, 0.0), Vec3::new(0.1, 1.5, 3.0)),
            (Vec3::new(13.0, 1.5, 0.0), Vec3::new(0.1, 1.5, 3.0)),
        ] {
            for t in cube(c, h) {
                g.insert(t);
            }
        }
        let mut nav = NavGrid::new(0.5, CAP, WALK);
        let (path, reached) = nav.find_path(&g, Vec3::new(0.0, 0.0, 0.0), Vec3::new(10.0, 0.0, 0.0), 3000);
        assert!(!reached);
        let end = *path.last().unwrap();
        assert!(end.x > 5.5 && end.x < 7.0, "ends against the box: {end}");
    }

    #[test]
    fn steps_up_but_not_up_walls() {
        let mut g = ground();
        // A 0.3 m platform (a step) and a 1.5 m block.
        for t in cube(Vec3::new(4.0, 0.15, 0.0), Vec3::new(2.0, 0.15, 2.0)).into_iter().chain(cube(Vec3::new(-4.0, 0.75, 0.0), Vec3::new(1.0, 0.75, 1.0))) {
            g.insert(t);
        }
        let mut nav = NavGrid::new(0.5, CAP, WALK);
        let (path, reached) = nav.find_path(&g, Vec3::new(0.0, 0.0, 0.0), Vec3::new(4.0, 0.3, 0.0), 2000);
        assert!(reached && (path.last().unwrap().y - 0.3).abs() < 1e-3, "{path:?}");
        let (_, reached) = nav.find_path(&g, Vec3::new(0.0, 0.0, 0.0), Vec3::new(-4.0, 1.5, 0.0), 2000);
        assert!(!reached, "on top of a 1.5 m block");
    }
}
