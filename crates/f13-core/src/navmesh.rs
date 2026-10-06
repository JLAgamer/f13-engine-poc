//! Path finding on a navmesh (recast.rs builds it), the way Detour does it: tiles are stitched where polygon edges
//! overlap on a shared tile border, a path is an A* search over polygons (moving between portal midpoints), then pulled
//! taut through the portals (the "simple stupid funnel").

use crate::recast::TileMesh;
use glam::{Vec2, Vec3};
use std::cmp::Ordering;
use std::collections::{BinaryHeap, HashMap};

/// A way from one polygon into a neighbour: the shared stretch of edge (`left`, `right` as Detour orders them).
#[derive(Clone, Copy, Debug)]
pub struct Portal {
    pub to: u32,
    pub left: Vec3,
    pub right: Vec3,
}

#[derive(Clone, Debug)]
pub struct Poly {
    pub verts: Vec<u32>,
    pub portals: Vec<Portal>,
    pub min: Vec3,
    pub max: Vec3,
}

pub struct NavMesh {
    pub verts: Vec<Vec3>,
    pub polys: Vec<Poly>,
    cell: f32,
    grid: HashMap<(i32, i32), Vec<u32>>,
}

/// Detour's dtTriArea2D (XZ).
fn tri_area(a: Vec3, b: Vec3, c: Vec3) -> f32 {
    let (abx, abz) = (b.x - a.x, b.z - a.z);
    let (acx, acz) = (c.x - a.x, c.z - a.z);
    acx * abz - abx * acz
}

fn xz(v: Vec3) -> Vec2 {
    Vec2::new(v.x, v.z)
}

impl NavMesh {
    /// Joins built tiles. `tile_size` and `origin` are the tile grid's; `climb` the largest height step between
    /// polygons that meet at a tile border.
    pub fn new(tiles: &[TileMesh], origin: Vec3, tile_size: f32, climb: f32) -> NavMesh {
        let mut verts = Vec::new();
        let mut polys: Vec<Poly> = Vec::new();
        // Open edges on a tile border: (axis, line key) -> (poly, a, b, tile).
        let mut border: HashMap<(u8, i64), Vec<(u32, Vec3, Vec3, (i32, i32))>> = HashMap::new();
        let eps = 1e-3;
        for t in tiles {
            let base = verts.len() as u32;
            let first = polys.len() as u32;
            verts.extend_from_slice(&t.verts);
            // Edges by vertex pair inside the tile.
            let mut edges: HashMap<(u32, u32), Vec<(u32, usize)>> = HashMap::new();
            for (k, p) in t.polys.iter().enumerate() {
                for i in 0..p.len() {
                    let (a, b) = (p[i], p[(i + 1) % p.len()]);
                    edges.entry((a.min(b), a.max(b))).or_default().push((first + k as u32, i));
                }
            }
            let (x0, z0) = (origin.x + t.tx as f32 * tile_size, origin.z + t.tz as f32 * tile_size);
            let (x1, z1) = (x0 + tile_size, z0 + tile_size);
            for (k, p) in t.polys.iter().enumerate() {
                let pv: Vec<u32> = p.iter().map(|&v| base + v).collect();
                let (mut min, mut max) = (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN));
                for &v in &pv {
                    min = min.min(verts[v as usize]);
                    max = max.max(verts[v as usize]);
                }
                let mut portals = Vec::new();
                for i in 0..p.len() {
                    let (a, b) = (p[i], p[(i + 1) % p.len()]);
                    let (va, vb) = (t.verts[a as usize], t.verts[b as usize]);
                    let shared = &edges[&(a.min(b), a.max(b))];
                    if let Some(&(other, _)) = shared.iter().find(|(q, _)| *q != first + k as u32) {
                        portals.push(Portal { to: other, left: va, right: vb });
                    } else {
                        // On the tile's border: matched against the neighbouring tile below.
                        let on = |v: f32, line: f32| (v - line).abs() < eps;
                        let key = if on(va.x, x0) && on(vb.x, x0) || on(va.x, x1) && on(vb.x, x1) {
                            Some((0u8, (va.x * 1000.0).round() as i64))
                        } else if on(va.z, z0) && on(vb.z, z0) || on(va.z, z1) && on(vb.z, z1) {
                            Some((1u8, (va.z * 1000.0).round() as i64))
                        } else {
                            None
                        };
                        if let Some(key) = key {
                            border.entry(key).or_default().push((first + k as u32, va, vb, (t.tx, t.tz)));
                        }
                    }
                }
                polys.push(Poly { verts: pv, portals, min, max });
            }
        }
        // Stitch tiles: overlapping edges on the same border line, from different tiles, at about the same height.
        for ((axis, _), list) in &border {
            let along = |v: Vec3| if *axis == 0 { v.z } else { v.x };
            for (i, &(pa, a0, a1, ta)) in list.iter().enumerate() {
                for &(pb, b0, b1, tb) in &list[i + 1..] {
                    if ta == tb {
                        continue;
                    }
                    let (alo, ahi) = (along(a0).min(along(a1)), along(a0).max(along(a1)));
                    let (blo, bhi) = (along(b0).min(along(b1)), along(b0).max(along(b1)));
                    let (lo, hi) = (alo.max(blo), ahi.min(bhi));
                    if hi - lo < 0.01 {
                        continue;
                    }
                    let at = |p: Vec3, q: Vec3, s: f32| {
                        let (sp, sq) = (along(p), along(q));
                        let t = if (sq - sp).abs() < 1e-6 { 0.0 } else { (s - sp) / (sq - sp) };
                        p.lerp(q, t.clamp(0.0, 1.0))
                    };
                    let (ax_lo, ax_hi) = (at(a0, a1, lo), at(a0, a1, hi));
                    let (bx_lo, bx_hi) = (at(b0, b1, lo), at(b0, b1, hi));
                    if (ax_lo.y - bx_lo.y).abs() > climb || (ax_hi.y - bx_hi.y).abs() > climb {
                        continue;
                    }
                    // Keep each polygon's own edge direction for left/right.
                    let order = |p0: Vec3, p1: Vec3, lo_pt: Vec3, hi_pt: Vec3| if along(p0) <= along(p1) { (lo_pt, hi_pt) } else { (hi_pt, lo_pt) };
                    let (l, r) = order(a0, a1, ax_lo, ax_hi);
                    polys[pa as usize].portals.push(Portal { to: pb, left: l, right: r });
                    let (l, r) = order(b0, b1, bx_lo, bx_hi);
                    polys[pb as usize].portals.push(Portal { to: pa, left: l, right: r });
                }
            }
        }
        let cell = 4.0;
        let mut grid: HashMap<(i32, i32), Vec<u32>> = HashMap::new();
        for (i, p) in polys.iter().enumerate() {
            let k = |v: f32| (v / cell).floor() as i32;
            for x in k(p.min.x)..=k(p.max.x) {
                for z in k(p.min.z)..=k(p.max.z) {
                    grid.entry((x, z)).or_default().push(i as u32);
                }
            }
        }
        NavMesh { verts, polys, cell, grid }
    }

    fn poly_verts(&self, p: u32) -> impl Iterator<Item = Vec3> + '_ {
        self.polys[p as usize].verts.iter().map(|&v| self.verts[v as usize])
    }

    /// Height of polygon `p` at (x, z) if the point is over it.
    pub fn height_on(&self, p: u32, x: f32, z: f32) -> Option<f32> {
        let v: Vec<Vec3> = self.poly_verts(p).collect();
        let q = Vec2::new(x, z);
        for i in 1..v.len() - 1 {
            let (a, b, c) = (xz(v[0]), xz(v[i]), xz(v[i + 1]));
            let d = (b - a).perp_dot(c - a);
            if d.abs() < 1e-9 {
                continue;
            }
            let u = (q - a).perp_dot(c - a) / -d;
            let w = (b - a).perp_dot(q - a) / d;
            let (u, w) = (-u, w);
            let s = 1.0 - u - w;
            let tol = -1e-4;
            if u >= tol && w >= tol && s >= tol {
                return Some(v[0].y * s + v[i].y * u + v[i + 1].y * w);
            }
        }
        None
    }

    fn candidates(&self, p: Vec3, radius: f32) -> Vec<u32> {
        let k = |v: f32| (v / self.cell).floor() as i32;
        let mut out = Vec::new();
        for x in k(p.x - radius)..=k(p.x + radius) {
            for z in k(p.z - radius)..=k(p.z + radius) {
                for &i in self.grid.get(&(x, z)).into_iter().flatten() {
                    if !out.contains(&i) {
                        out.push(i);
                    }
                }
            }
        }
        out
    }

    /// The nearest point on the navmesh to `p` within `radius` (XZ; heights within `radius` too, favouring the
    /// surface under the point), and its polygon.
    pub fn nearest(&self, p: Vec3, radius: f32) -> Option<(u32, Vec3)> {
        let mut best: Option<(f32, u32, Vec3)> = None;
        for i in self.candidates(p, radius) {
            let poly = &self.polys[i as usize];
            if poly.min.x > p.x + radius || poly.max.x < p.x - radius || poly.min.z > p.z + radius || poly.max.z < p.z - radius {
                continue;
            }
            let point = match self.height_on(i, p.x, p.z) {
                Some(h) => Vec3::new(p.x, h, p.z),
                None => {
                    // Closest point on the boundary.
                    let v: Vec<Vec3> = self.poly_verts(i).collect();
                    let mut bp = v[0];
                    let mut bd = f32::MAX;
                    for k in 0..v.len() {
                        let (a, b) = (v[k], v[(k + 1) % v.len()]);
                        let ab = xz(b) - xz(a);
                        let t = if ab.length_squared() > 0.0 { ((xz(p) - xz(a)).dot(ab) / ab.length_squared()).clamp(0.0, 1.0) } else { 0.0 };
                        let c = a.lerp(b, t);
                        let d = xz(c).distance_squared(xz(p));
                        if d < bd {
                            bd = d;
                            bp = c;
                        }
                    }
                    bp
                }
            };
            let dy = point.y - p.y;
            if dy.abs() > radius.max(2.0) {
                continue;
            }
            // Horizontal distance, plus height (a surface above the feet counts more than one just below).
            let score = xz(point).distance(xz(p)) + if dy > 0.0 { dy * 2.0 } else { -dy };
            if best.is_none_or(|b| score < b.0) {
                best = Some((score, i, point));
            }
        }
        best.map(|(_, i, pt)| (i, pt))
    }

    /// The nearest point within growing radii up to `max` (so a point just off the mesh snaps to the floor beside
    /// it, not to one past a wall that happens to be closer in height).
    pub fn snap(&self, p: Vec3, max: f32) -> Option<(u32, Vec3)> {
        let mut r: f32 = 0.5;
        loop {
            if let Some(hit) = self.nearest(p, r.min(max)) {
                return Some(hit);
            }
            if r >= max {
                return None;
            }
            r *= 2.0;
        }
    }

    /// A path from `from` to `to` (both snapped onto the mesh), as corner points ending at the goal. When the goal
    /// can't be reached, the path leads to the reachable point nearest to it. `max_nodes` bounds the search.
    pub fn find_path(&self, from: Vec3, to: Vec3, max_nodes: usize) -> Option<Vec<Vec3>> {
        let (start, sp) = self.snap(from, 2.0)?;
        let (goal, gp) = self.snap(to, 4.0).unwrap_or((u32::MAX, to));
        #[derive(PartialEq)]
        struct Node(f32, u32);
        impl Eq for Node {}
        impl Ord for Node {
            fn cmp(&self, o: &Self) -> Ordering {
                o.0.partial_cmp(&self.0).unwrap_or(Ordering::Equal)
            }
        }
        impl PartialOrd for Node {
            fn partial_cmp(&self, o: &Self) -> Option<Ordering> {
                Some(self.cmp(o))
            }
        }
        // Per polygon: cost so far, entry point, parent.
        let mut info: HashMap<u32, (f32, Vec3, u32)> = HashMap::new();
        let mut open = BinaryHeap::new();
        info.insert(start, (0.0, sp, u32::MAX));
        open.push(Node(sp.distance(gp), start));
        let mut best = (sp.distance(gp), start);
        let mut expanded = 0;
        while let Some(Node(_, cur)) = open.pop() {
            if cur == goal {
                best = (0.0, cur);
                break;
            }
            expanded += 1;
            if expanded > max_nodes {
                break;
            }
            let (g, pos, _) = info[&cur];
            for portal in &self.polys[cur as usize].portals {
                let mid = (portal.left + portal.right) * 0.5;
                let ng = g + pos.distance(mid) + if portal.to == goal { mid.distance(gp) } else { 0.0 };
                if info.get(&portal.to).is_some_and(|i| i.0 <= ng) {
                    continue;
                }
                info.insert(portal.to, (ng, mid, cur));
                let h = mid.distance(gp);
                if h < best.0 {
                    best = (h, portal.to);
                }
                open.push(Node(ng + h, portal.to));
            }
        }
        // The polygon corridor, start to end.
        let end = best.1;
        let mut corridor = vec![end];
        let mut at = end;
        while let Some(&(_, _, parent)) = info.get(&at) {
            if parent == u32::MAX {
                break;
            }
            corridor.push(parent);
            at = parent;
        }
        corridor.reverse();
        let target = if end == goal { gp } else { info[&end].1 };
        Some(self.string_pull(sp, target, &corridor))
    }

    /// Pulls a path taut through the corridor's portals (the simple stupid funnel).
    fn string_pull(&self, start: Vec3, end: Vec3, corridor: &[u32]) -> Vec<Vec3> {
        let mut portals: Vec<(Vec3, Vec3)> = vec![(start, start)];
        for w in corridor.windows(2) {
            if let Some(p) = self.polys[w[0] as usize].portals.iter().find(|p| p.to == w[1]) {
                portals.push((p.left, p.right));
            }
        }
        portals.push((end, end));
        let mut path = Vec::new();
        let (mut apex, mut left, mut right) = (start, portals[0].0, portals[0].1);
        let (mut left_i, mut right_i) = (0usize, 0usize);
        let mut apex_i;
        let same = |a: Vec3, b: Vec3| xz(a).distance_squared(xz(b)) < 1e-8;
        let mut i = 1;
        while i < portals.len() {
            let (l, r) = portals[i];
            // Tighten the right side.
            if tri_area(apex, right, r) <= 0.0 {
                if same(apex, right) || tri_area(apex, left, r) > 0.0 {
                    right = r;
                    right_i = i;
                } else {
                    // Right crossed over left: the left point is a corner.
                    path.push(left);
                    apex = left;
                    apex_i = left_i;
                    left = apex;
                    right = apex;
                    left_i = apex_i;
                    right_i = apex_i;
                    i = apex_i + 1;
                    continue;
                }
            }
            // Tighten the left side.
            if tri_area(apex, left, l) >= 0.0 {
                if same(apex, left) || tri_area(apex, right, l) < 0.0 {
                    left = l;
                    left_i = i;
                } else {
                    path.push(right);
                    apex = right;
                    apex_i = right_i;
                    left = apex;
                    right = apex;
                    left_i = apex_i;
                    right_i = apex_i;
                    i = apex_i + 1;
                    continue;
                }
            }
            i += 1;
        }
        path.push(end);
        path.dedup_by(|a, b| a.distance_squared(*b) < 1e-6);
        path
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::collision::{convex_hull, Tri, TriGrid};
    use crate::recast::{build, NavConfig};

    fn cfg() -> NavConfig {
        NavConfig {
            cell_size: 0.09,
            cell_height: 0.1,
            agent_height: 2.0,
            agent_radius: 0.34,
            agent_max_climb: 0.4,
            agent_max_slope_deg: 45.0,
            tile_cells: 100,
            max_simplification_error: 1.2,
            max_edge_len: 133,
            max_verts_per_poly: 6,
        }
    }

    fn cube(c: Vec3, h: Vec3) -> Vec<Tri> {
        let pts: Vec<Vec3> = (0..8).map(|i| c + Vec3::new(if i & 1 != 0 { h.x } else { -h.x }, if i & 2 != 0 { h.y } else { -h.y }, if i & 4 != 0 { h.z } else { -h.z })).collect();
        convex_hull(&pts)
    }

    /// A 20 x 20 m floor with a wall across the middle (x = 10) and a 1.2 m gap in it at z 2..3.2.
    fn world() -> TriGrid {
        let mut g = TriGrid::new(2.0);
        for t in cube(Vec3::new(10.0, -0.5, 10.0), Vec3::new(10.0, 0.5, 10.0)) {
            g.insert(t);
        }
        for t in cube(Vec3::new(10.0, 1.5, 11.6), Vec3::new(0.2, 1.5, 8.4)) {
            g.insert(t);
        }
        for t in cube(Vec3::new(10.0, 1.5, 0.5), Vec3::new(0.2, 1.5, 1.5)) {
            g.insert(t);
        }
        g
    }

    #[test]
    fn paths_go_through_the_gap_in_the_wall() {
        let w = world();
        let c = cfg();
        let tiles = build(&w, &c, Vec3::new(0.0, -1.0, 0.0), Vec3::new(20.0, 3.0, 20.0), &Default::default());
        let mesh = NavMesh::new(&tiles, Vec3::new(0.0, 0.0, 0.0), c.tile_size(), c.agent_max_climb);
        assert!(mesh.polys.len() > 4, "{} polys", mesh.polys.len());
        // The floor is walkable at y 0; nothing within the agent radius of the wall.
        let (_, p) = mesh.nearest(Vec3::new(5.0, 0.0, 5.0), 1.0).expect("on the floor");
        assert!(p.y.abs() < 0.15, "{p}");
        assert!(mesh.nearest(Vec3::new(10.0, 0.0, 10.0), 0.1).is_none(), "the wall is not walkable");
        let path = mesh.find_path(Vec3::new(5.0, 0.0, 15.0), Vec3::new(15.0, 0.0, 15.0), 10000).expect("a path");
        let last = *path.last().unwrap();
        assert!(last.distance(Vec3::new(15.0, 0.0, 15.0)) < 0.2, "{path:?}");
        // It bends through the gap (z 2..3.2), keeping clear of its posts.
        let through = path.windows(2).any(|w| (w[0].x - 10.0) * (w[1].x - 10.0) <= 0.0 && w[0].z.max(w[1].z) < 3.3);
        assert!(through, "{path:?}");
        // Taut: a few corners only, and not much longer than the way through the gap.
        assert!(path.len() <= 5, "{path:?}");
        let len: f32 = std::iter::once(Vec3::new(5.0, 0.0, 15.0)).chain(path.iter().copied()).collect::<Vec<_>>().windows(2).map(|w| w[0].distance(w[1])).sum();
        assert!(len < 30.0, "{len} {path:?}");
    }
}
