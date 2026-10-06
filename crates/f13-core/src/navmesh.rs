//! Path finding on a navmesh (recast.rs builds it), the way Detour does it: tiles are stitched where polygon edges
//! overlap on a shared tile border, a path is an A* search over polygons, then pulled taut through the portals (the
//! "simple stupid funnel"). Detour's A* moves between portal midpoints; this one crosses each portal where the line
//! toward the goal does (or at its nearer end), which costs large polygons more truly and picks better corridors.

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

/// Where the way from `from` toward `to` crosses the portal `l`..`r` (XZ): the point on the line, or the portal's
/// end nearest the line.
fn crossing(from: Vec3, to: Vec3, l: Vec3, r: Vec3) -> Vec3 {
    // from + d s = l + e t  =>  t = ((from - l) x d) / (e x d)
    let (d, e) = (xz(to) - xz(from), xz(r) - xz(l));
    let denom = e.perp_dot(d);
    let t = if denom.abs() < 1e-9 { 0.5 } else { (xz(from) - xz(l)).perp_dot(d) / denom };
    l.lerp(r, t.clamp(0.0, 1.0))
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

    /// Which island (polygons joined by portals) each polygon is on.
    pub fn island_of(&self) -> Vec<u32> {
        let mut island = vec![u32::MAX; self.polys.len()];
        let mut next = 0;
        for start in 0..self.polys.len() {
            if island[start] != u32::MAX {
                continue;
            }
            let mut stack = vec![start as u32];
            island[start] = next;
            while let Some(p) = stack.pop() {
                for q in &self.polys[p as usize].portals {
                    if island[q.to as usize] == u32::MAX {
                        island[q.to as usize] = next;
                        stack.push(q.to);
                    }
                }
            }
            next += 1;
        }
        island
    }

    /// The islands' sizes (polygons), largest first.
    pub fn islands(&self) -> Vec<usize> {
        let of = self.island_of();
        let mut sizes = vec![0usize; of.iter().map(|&i| i as usize + 1).max().unwrap_or(0)];
        for i in of {
            sizes[i as usize] += 1;
        }
        sizes.sort_unstable_by(|a, b| b.cmp(a));
        sizes
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

    /// A path from `from` to `to` (both snapped onto the mesh), as corner points ending at the goal, and whether it
    /// gets there: when the goal can't be reached, the path leads to the reachable point nearest to it. `max_nodes`
    /// bounds the search.
    pub fn find_path(&self, from: Vec3, to: Vec3, max_nodes: usize) -> Option<(Vec<Vec3>, bool)> {
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
                let mid = crossing(pos, gp, portal.left, portal.right);
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
        let pulled = self.string_pull(sp, target, &corridor);
        Some((self.shortcut(sp, pulled), end == goal))
    }

    /// Whether a straight walk from `from` to `to` stays on the mesh (Detour's raycast, in XZ): from the polygon
    /// under `from`, out through the edge the line leaves by, on through the portal there, until the polygon that
    /// holds `to`; a wall edge stops it.
    pub fn walkable_line(&self, from: Vec3, to: Vec3) -> bool {
        let dir = xz(to) - xz(from);
        let len = dir.length();
        if len < 1e-4 {
            return true;
        }
        // Start just inside the line, so a start on an edge or a corner picks the polygon the line goes into.
        let nudge = dir / len * 0.01;
        let start = from + Vec3::new(nudge.x, 0.0, nudge.y);
        let Some((mut poly, _)) = self.nearest(start, 0.3) else { return false };
        let mut at = xz(start);
        let target = xz(to);
        for _ in 0..4096 {
            let v: Vec<Vec2> = self.poly_verts(poly).map(xz).collect();
            let centre = v.iter().copied().sum::<Vec2>() / v.len() as f32;
            // The edge the line leaves by: the nearest crossing of an edge it heads out through.
            let d = target - at;
            let mut exit: Option<(f32, usize)> = None;
            for k in 0..v.len() {
                let (a, b) = (v[k], v[(k + 1) % v.len()]);
                let e = b - a;
                let mut n = Vec2::new(e.y, -e.x);
                if n.dot(a - centre) < 0.0 {
                    n = -n;
                }
                let denom = n.dot(d);
                if denom <= 1e-9 {
                    continue;
                }
                let s = n.dot(a - at) / denom;
                if exit.is_none_or(|(best, _)| s < best) {
                    exit = Some((s, k));
                }
            }
            let Some((s, k)) = exit else { return true };
            if s >= 1.0 {
                return true; // `to` is in this polygon
            }
            let out = at + d * s.max(0.0);
            // Through a portal on that edge, else into a wall.
            let (a, b) = (v[k], v[(k + 1) % v.len()]);
            let on_edge = |p: Vec2| {
                let e = b - a;
                let t = (p - a).dot(e) / e.length_squared().max(1e-12);
                (a + e * t).distance(p) < 0.02
            };
            let next = self.polys[poly as usize].portals.iter().find(|q| {
                let (l, r) = (xz(q.left), xz(q.right));
                if !on_edge(l) || !on_edge(r) {
                    return false;
                }
                let span = r - l;
                let t = (out - l).dot(span) / span.length_squared().max(1e-12);
                (-0.01..=1.01).contains(&t)
            });
            let Some(q) = next else { return false };
            poly = q.to;
            at = out + d.normalize_or_zero() * 0.001;
        }
        false
    }

    /// Drops the corners a straight line past them can skip (the funnel is only as good as the polygon corridor the
    /// search picked; a line along the mesh is shorter).
    fn shortcut(&self, start: Vec3, path: Vec<Vec3>) -> Vec<Vec3> {
        let mut pts = vec![start];
        pts.extend(path);
        let mut out = Vec::new();
        let mut i = 0;
        while i + 1 < pts.len() {
            let mut j = pts.len() - 1;
            while j > i + 1 && !self.walkable_line(pts[i], pts[j]) {
                j -= 1;
            }
            out.push(pts[j]);
            i = j;
        }
        out
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
            merge_region_area: 4444,
            region_chunk_splits: 2,
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
    fn crossing_follows_the_line_or_takes_the_nearer_end() {
        let (l, r) = (Vec3::new(1.0, 0.0, -1.0), Vec3::new(1.0, 0.0, 1.0));
        assert!(crossing(Vec3::ZERO, Vec3::new(2.0, 0.0, 1.0), l, r).distance(Vec3::new(1.0, 0.0, 0.5)) < 1e-5);
        assert!(crossing(Vec3::ZERO, Vec3::new(2.0, 0.0, 9.0), l, r).distance(r) < 1e-5);
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
        let (path, reached) = mesh.find_path(Vec3::new(5.0, 0.0, 15.0), Vec3::new(15.0, 0.0, 15.0), 10000).expect("a path");
        assert!(reached);
        let last = *path.last().unwrap();
        assert!(last.distance(Vec3::new(15.0, 0.0, 15.0)) < 0.2, "{path:?}");
        // It bends through the gap (z 2..3.2), keeping clear of its posts.
        let through = path.windows(2).any(|w| (w[0].x - 10.0) * (w[1].x - 10.0) <= 0.0 && w[0].z.max(w[1].z) < 3.3);
        assert!(through, "{path:?}");
        // Taut: round the gap's chamfered corners and on to the goal, within a few percent of the shortest way
        // (about 26.9 m; the polygon corridor the search picks can still cost a corner, 28.0 m here).
        assert!(path.len() <= 5, "{path:?}");
        let len: f32 = std::iter::once(Vec3::new(5.0, 0.0, 15.0)).chain(path.iter().copied()).collect::<Vec<_>>().windows(2).map(|w| w[0].distance(w[1])).sum();
        assert!(len < 28.5, "{len} {path:?}");
        // Straight lines: along the floor yes, through the wall no.
        assert!(mesh.walkable_line(Vec3::new(2.0, 0.0, 15.0), Vec3::new(8.0, 0.0, 3.0)));
        assert!(!mesh.walkable_line(Vec3::new(5.0, 0.0, 15.0), Vec3::new(15.0, 0.0, 15.0)));
    }

    /// A 5 x 5 m room (walls 0.3 m thick, a 1.2 m doorway in its east wall) at `at` on a 24 x 24 m floor.
    fn cabin(at: Vec3) -> (TriGrid, Vec<Vec3>) {
        let mut w = TriGrid::new(2.0);
        let walls = [
            (Vec3::new(0.0, 1.5, 2.5), Vec3::new(0.15, 1.5, 2.65)),
            (Vec3::new(2.5, 1.5, 0.0), Vec3::new(2.5, 1.5, 0.15)),
            (Vec3::new(2.5, 1.5, 5.0), Vec3::new(2.5, 1.5, 0.15)),
            // East wall, x = 5, with the doorway at z 3.4..4.6.
            (Vec3::new(5.0, 1.5, 1.7), Vec3::new(0.15, 1.5, 1.7)),
            (Vec3::new(5.0, 1.5, 4.8), Vec3::new(0.15, 1.5, 0.2)),
        ];
        for t in cube(Vec3::new(12.0, -0.5, 12.0), Vec3::new(12.0, 0.5, 12.0)).into_iter().chain(walls.iter().flat_map(|&(c, h)| cube(at + c, h))) {
            w.insert(t);
        }
        // Points inside the walls (not the doorway).
        let solid = vec![at + Vec3::new(5.0, 0.0, 0.8), at + Vec3::new(5.0, 0.0, 2.0), at + Vec3::new(5.0, 0.0, 3.0), at + Vec3::new(2.5, 0.0, 0.0), at + Vec3::new(2.5, 0.0, 5.0), at + Vec3::new(0.0, 0.0, 2.5)];
        (w, solid)
    }

    #[test]
    fn no_polygon_crosses_a_cabin_wall() {
        // Wherever the cabin sits on the tile and chunk grid, the floor's polygons stop at its walls (a border between
        // two regions that winds through the doorway was drawn straight, across the wall).
        let c = cfg();
        for i in 0..12 {
            let at = Vec3::new(4.0 + i as f32 * 0.77, 0.0, 6.0 + i as f32 * 0.53);
            let (w, solid) = cabin(at);
            let tiles = build(&w, &c, Vec3::new(0.0, -1.0, 0.0), Vec3::new(24.0, 3.0, 24.0), &Default::default());
            let mesh = NavMesh::new(&tiles, Vec3::ZERO, c.tile_size(), c.agent_max_climb);
            for p in &solid {
                let over: Vec<usize> = (0..mesh.polys.len()).filter(|&k| mesh.height_on(k as u32, p.x, p.z).is_some_and(|h| h.abs() < 0.5)).collect();
                assert!(over.is_empty(), "cabin at {at}: floor polygons {over:?} over the wall at {p}");
            }
            let (_, reached) = mesh.find_path(at + Vec3::new(8.0, 0.0, 2.0), at + Vec3::new(2.5, 0.0, 2.5), 20000).expect("a path");
            assert!(reached, "cabin at {at}: in through the doorway");
        }
    }

    #[test]
    fn polygons_go_round_obstacles_inside_a_region() {
        // A 20 x 20 m floor with a free-standing wall (x = 10, z 6..14) and a pillar: the regions merge round them, so
        // their outlines wind round the wall's ends and the pillar is a hole.
        let mut w = TriGrid::new(2.0);
        for t in cube(Vec3::new(10.0, -0.5, 10.0), Vec3::new(10.0, 0.5, 10.0))
            .into_iter()
            .chain(cube(Vec3::new(10.0, 1.5, 10.0), Vec3::new(0.2, 1.5, 4.0)))
            .chain(cube(Vec3::new(4.0, 1.5, 4.0), Vec3::new(0.6, 1.5, 0.6)))
        {
            w.insert(t);
        }
        let c = cfg();
        let tiles = build(&w, &c, Vec3::new(0.0, -1.0, 0.0), Vec3::new(20.0, 3.0, 20.0), &Default::default());
        let mesh = NavMesh::new(&tiles, Vec3::ZERO, c.tile_size(), c.agent_max_climb);
        // The floor's polygons (the pillar's top is walkable too, and so is its hollow inside: a closed mesh is
        // rasterized by its faces, as in Recast; both are islands of their own).
        let islands = mesh.island_of();
        let (start, _) = mesh.nearest(Vec3::new(1.0, 0.0, 1.0), 0.5).expect("on the floor");
        let floor = islands[start as usize];
        for p in [Vec3::new(10.0, 0.0, 7.0), Vec3::new(10.0, 0.0, 10.0), Vec3::new(10.0, 0.0, 13.0), Vec3::new(4.0, 0.0, 4.0)] {
            let over: Vec<usize> = (0..mesh.polys.len()).filter(|&i| islands[i] == floor && mesh.height_on(i as u32, p.x, p.z).is_some()).collect();
            assert!(over.is_empty(), "floor polygons {over:?} over the obstacle at {p}");
        }
        assert!(!mesh.walkable_line(Vec3::new(7.0, 0.0, 10.0), Vec3::new(13.0, 0.0, 10.0)), "straight through the wall");
        assert!(!mesh.walkable_line(Vec3::new(2.0, 0.0, 4.0), Vec3::new(6.0, 0.0, 4.0)), "straight through the pillar");
        let (_, reached) = mesh.find_path(Vec3::new(7.0, 0.0, 10.0), Vec3::new(13.0, 0.0, 10.0), 10000).expect("a path");
        assert!(reached, "round the wall's end");
    }
}
