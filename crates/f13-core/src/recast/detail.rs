//! Detail meshes (Recast's rcBuildPolyMeshDetail, with the settings UE's tile generator sets: a sample distance of
//! 600 cm and a maximum error of 1 cm): each polygon's own triangles carrying the ground's heights. Its height patch
//! takes the heightfield's spans of the polygon's region over its bounds and floods out from that region's border
//! (from the cells nearest its corners when it has none there); its edges are sampled every sample distance and the
//! samples kept where they stray from the straight edge by more than the error; its inside is sampled on a grid of the
//! sample distance (half a sample from its edges), the sample furthest from the triangles added each round and the
//! polygon re-triangulated (Delaunay) until none strays by more than the error.

use super::Compact;
use glam::Vec3;

const UNSET: u16 = 0xffff;
const MAX_VERTS: usize = 127;
const MAX_TRIS: usize = 255;
const MAX_VERTS_PER_EDGE: usize = 32;

/// One polygon's heights over its bounds (cells; UNSET where it has none).
struct HeightPatch {
    xmin: i32,
    zmin: i32,
    w: i32,
    h: i32,
    data: Vec<u16>,
}

impl HeightPatch {
    fn at(&self, x: i32, z: i32) -> u16 {
        self.data[((x - self.xmin) + (z - self.zmin) * self.w) as usize]
    }

    fn set(&mut self, x: i32, z: i32, v: u16) {
        self.data[((x - self.xmin) + (z - self.zmin) * self.w) as usize] = v;
    }

    fn inside(&self, x: i32, z: i32) -> bool {
        x >= self.xmin && x < self.xmin + self.w && z >= self.zmin && z < self.zmin + self.h
    }
}

/// The polygon's height patch (getHeightData): its region's spans, then a flood fill from that region's border cells
/// (or, with none of its region inside, from the spans nearest its corners walked to its centre).
fn height_patch(c: &Compact, reg: &[u32], poly: &[(i32, i32, i32)], region: u32) -> HeightPatch {
    let (mut xmin, mut xmax, mut zmin, mut zmax) = (c.w, 0, c.h, 0);
    for &(x, _, z) in poly {
        xmin = xmin.min(x);
        xmax = xmax.max(x);
        zmin = zmin.min(z);
        zmax = zmax.max(z);
    }
    let (xmin, xmax, zmin, zmax) = ((xmin - 1).max(0), (xmax + 1).min(c.w), (zmin - 1).max(0), (zmax + 1).min(c.h));
    let (w, h) = ((xmax - xmin).max(0), (zmax - zmin).max(0));
    let mut hp = HeightPatch { xmin, zmin, w, h, data: vec![UNSET; (w * h) as usize] };
    let mut queue: Vec<(i32, i32, usize)> = Vec::new();
    let mut empty = true;
    for z in zmin..zmax {
        for x in xmin..xmax {
            let (f, n) = c.cells[(x + z * c.w) as usize];
            for i in f as usize..(f + n) as usize {
                if reg[i] != region {
                    continue;
                }
                hp.set(x, z, c.y[i] as u16);
                empty = false;
                // A border of the region: the flood fill starts there.
                if (0..4).any(|d| c.nei(i, d).is_some_and(|a| reg[a] != region)) {
                    queue.push((x, z, i));
                }
                break;
            }
        }
    }
    if empty {
        queue = seeds_from_vertices(c, poly, &mut hp);
    }
    let mut head = 0;
    while head < queue.len() {
        let (cx, cz, ci) = queue[head];
        head += 1;
        for d in 0..4 {
            let Some(ai) = c.nei(ci, d) else { continue };
            let (ax, az) = (cx + super::DX[d], cz + super::DZ[d]);
            if !hp.inside(ax, az) || hp.at(ax, az) != UNSET {
                continue;
            }
            hp.set(ax, az, c.y[ai] as u16);
            queue.push((ax, az, ai));
        }
    }
    hp
}

/// getHeightDataSeedsFromVertices: the span nearest in height to each corner (in its 3×3 cells), walked to the
/// polygon's centre; the fill starts there.
fn seeds_from_vertices(c: &Compact, poly: &[(i32, i32, i32)], hp: &mut HeightPatch) -> Vec<(i32, i32, usize)> {
    const OFFSET: [(i32, i32); 9] = [(0, 0), (-1, -1), (0, -1), (1, -1), (1, 0), (1, 1), (0, 1), (-1, 1), (-1, 0)];
    hp.data.iter_mut().for_each(|d| *d = 0);
    let mut stack: Vec<(i32, i32, usize)> = Vec::new();
    for &(vx, vy, vz) in poly {
        let mut best: Option<(i32, i32, usize)> = None;
        let mut dmin = UNSET as i32;
        for (ox, oz) in OFFSET {
            let (ax, az) = (vx + ox, vz + oz);
            if !hp.inside(ax, az) {
                continue;
            }
            let (f, n) = c.cells[(ax + az * c.w) as usize];
            for i in f as usize..(f + n) as usize {
                let d = (vy - c.y[i]).abs();
                if d < dmin {
                    best = Some((ax, az, i));
                    dmin = d;
                }
            }
        }
        stack.extend(best);
    }
    let n = poly.len() as i32;
    let (pcx, pcz) = (poly.iter().map(|v| v.0).sum::<i32>() / n, poly.iter().map(|v| v.2).sum::<i32>() / n);
    for &(x, z, _) in &stack {
        hp.set(x, z, 1);
    }
    while let Some((cx, cz, ci)) = stack.pop() {
        if (cx - pcx).abs() <= 1 && (cz - pcz).abs() <= 1 {
            stack.clear();
            stack.push((cx, cz, ci));
            break;
        }
        for d in 0..4 {
            let Some(ai) = c.nei(ci, d) else { continue };
            let (ax, az) = (cx + super::DX[d], cz + super::DZ[d]);
            if !hp.inside(ax, az) || hp.at(ax, az) != 0 {
                continue;
            }
            hp.set(ax, az, 1);
            stack.push((ax, az, ai));
        }
    }
    hp.data.iter_mut().for_each(|d| *d = UNSET);
    for &(x, z, i) in &stack {
        hp.set(x, z, c.y[i] as u16);
    }
    stack
}

/// getHeight: the patch's height under (fx, fz) (polygon space), or its 8 neighbours' nearest to `fy` where unset.
fn get_height(fx: f32, fy: f32, fz: f32, cs: f32, ch: f32, hp: &HeightPatch) -> u16 {
    let ix = ((fx / cs + 0.01).floor() as i32 - hp.xmin).clamp(0, hp.w - 1);
    let iz = ((fz / cs + 0.01).floor() as i32 - hp.zmin).clamp(0, hp.h - 1);
    let mut h = hp.data[(ix + iz * hp.w) as usize];
    if h == UNSET {
        const OFF: [(i32, i32); 8] = [(-1, 0), (-1, -1), (0, -1), (1, -1), (1, 0), (1, 1), (0, 1), (-1, 1)];
        let mut dmin = f32::MAX;
        for (ox, oz) in OFF {
            let (nx, nz) = (ix + ox, iz + oz);
            if nx < 0 || nz < 0 || nx >= hp.w || nz >= hp.h {
                continue;
            }
            let nh = hp.data[(nx + nz * hp.w) as usize];
            if nh == UNSET {
                continue;
            }
            let d = (nh as f32 * ch - fy).abs();
            if d < dmin {
                h = nh;
                dmin = d;
            }
        }
    }
    h
}

fn vdist2(p: Vec3, q: Vec3) -> f32 {
    ((q.x - p.x).powi(2) + (q.z - p.z).powi(2)).sqrt()
}

fn vcross2(p1: Vec3, p2: Vec3, p3: Vec3) -> f32 {
    let (u1, v1) = (p2.x - p1.x, p2.z - p1.z);
    let (u2, v2) = (p3.x - p1.x, p3.z - p1.z);
    u1 * v2 - v1 * u2
}

/// Squared distance from `pt` to the segment p..q (3D).
fn distance_pt_seg(pt: Vec3, p: Vec3, q: Vec3) -> f32 {
    let pq = q - p;
    let d = pt - p;
    let len = pq.length_squared();
    let mut t = pq.dot(d);
    if len > 0.0 {
        t /= len;
    }
    let t = t.clamp(0.0, 1.0);
    (p + pq * t - pt).length_squared()
}

fn distance_pt_seg_2d(pt: Vec3, p: Vec3, q: Vec3) -> f32 {
    let (pqx, pqz) = (q.x - p.x, q.z - p.z);
    let (dx, dz) = (pt.x - p.x, pt.z - p.z);
    let d = pqx * pqx + pqz * pqz;
    let mut t = pqx * dx + pqz * dz;
    if d > 0.0 {
        t /= d;
    }
    let t = t.clamp(0.0, 1.0);
    let (dx, dz) = (p.x + t * pqx - pt.x, p.z + t * pqz - pt.z);
    dx * dx + dz * dz
}

/// Signed distance (XZ) from `p` to the polygon's outline: negative inside.
fn dist_to_poly(verts: &[Vec3], p: Vec3) -> f32 {
    let mut dmin = f32::MAX;
    let mut c = false;
    let n = verts.len();
    let mut j = n - 1;
    for i in 0..n {
        let (vi, vj) = (verts[i], verts[j]);
        if ((vi.z > p.z) != (vj.z > p.z)) && (p.x < (vj.x - vi.x) * (p.z - vi.z) / (vj.z - vi.z) + vi.x) {
            c = !c;
        }
        dmin = dmin.min(distance_pt_seg_2d(p, vj, vi));
        j = i;
    }
    if c { -dmin } else { dmin }
}

/// Height difference from `p` to the triangle under it (None if it isn't over it).
fn dist_pt_tri(p: Vec3, a: Vec3, b: Vec3, c: Vec3) -> Option<f32> {
    let (v0, v1, v2) = (c - a, b - a, p - a);
    let dot00 = v0.x * v0.x + v0.z * v0.z;
    let dot01 = v0.x * v1.x + v0.z * v1.z;
    let dot02 = v0.x * v2.x + v0.z * v2.z;
    let dot11 = v1.x * v1.x + v1.z * v1.z;
    let dot12 = v1.x * v2.x + v1.z * v2.z;
    let inv = 1.0 / (dot00 * dot11 - dot01 * dot01);
    let u = (dot11 * dot02 - dot01 * dot12) * inv;
    let v = (dot00 * dot12 - dot01 * dot02) * inv;
    const EPS: f32 = 1e-4;
    if u >= -EPS && v >= -EPS && (u + v) <= 1.0 + EPS {
        let y = a.y + v0.y * u + v1.y * v;
        return Some((y - p.y).abs());
    }
    None
}

fn dist_to_tri_mesh(p: Vec3, verts: &[Vec3], tris: &[[usize; 3]]) -> Option<f32> {
    let mut dmin: Option<f32> = None;
    for t in tris {
        if let Some(d) = dist_pt_tri(p, verts[t[0]], verts[t[1]], verts[t[2]]) {
            dmin = Some(dmin.map_or(d, |m| m.min(d)));
        }
    }
    dmin
}

/// triangulateHull: from the hull's first corner, advancing left or right by the shorter new diagonal.
fn triangulate_hull(verts: &[Vec3], hull: &[usize]) -> Vec<[usize; 3]> {
    let n = hull.len();
    let next = |i: usize| (i + 1) % n;
    let prev = |i: usize| (i + n - 1) % n;
    let (mut left, mut right) = (1, n - 1);
    let mut tris = vec![[hull[0], hull[left], hull[right]]];
    while next(left) != right {
        let (nleft, nright) = (next(left), prev(right));
        let (cvleft, nvleft, cvright, nvright) = (verts[hull[left]], verts[hull[nleft]], verts[hull[right]], verts[hull[nright]]);
        let dleft = vdist2(cvleft, nvleft) + vdist2(nvleft, cvright);
        let dright = vdist2(cvright, nvright) + vdist2(cvleft, nvright);
        if dleft < dright {
            tris.push([hull[left], hull[nleft], hull[right]]);
            left = nleft;
        } else {
            tris.push([hull[left], hull[nright], hull[right]]);
            right = nright;
        }
    }
    tris
}

const EV_UNDEF: i32 = -1;
const EV_HULL: i32 = -2;

fn find_edge(edges: &[[i32; 4]], s: i32, t: i32) -> Option<usize> {
    edges.iter().position(|e| (e[0] == s && e[1] == t) || (e[0] == t && e[1] == s))
}

fn add_edge(edges: &mut Vec<[i32; 4]>, max: usize, s: i32, t: i32, l: i32, r: i32) {
    if edges.len() >= max {
        return;
    }
    if find_edge(edges, s, t).is_none() {
        edges.push([s, t, l, r]);
    }
}

fn update_left_face(e: &mut [i32; 4], s: i32, t: i32, f: i32) {
    if e[0] == s && e[1] == t && e[2] == EV_UNDEF {
        e[2] = f;
    } else if e[1] == s && e[0] == t && e[3] == EV_UNDEF {
        e[3] = f;
    }
}

fn overlap_seg_seg_2d(a: Vec3, b: Vec3, c: Vec3, d: Vec3) -> bool {
    let a1 = vcross2(a, b, d);
    let a2 = vcross2(a, b, c);
    if a1 * a2 < 0.0 {
        let a3 = vcross2(c, d, a);
        let a4 = a3 + a2 - a1;
        if a3 * a4 < 0.0 {
            return true;
        }
    }
    false
}

fn overlap_edges(pts: &[Vec3], edges: &[[i32; 4]], s1: usize, t1: usize) -> bool {
    for e in edges {
        let (s0, t0) = (e[0] as usize, e[1] as usize);
        // Same or connected edges do not overlap.
        if s0 == s1 || s0 == t1 || t0 == s1 || t0 == t1 {
            continue;
        }
        if overlap_seg_seg_2d(pts[s0], pts[t0], pts[s1], pts[t1]) {
            return true;
        }
    }
    false
}

/// The circle through three points (XZ): its centre and radius.
fn circum_circle(p1: Vec3, p2: Vec3, p3: Vec3) -> (Vec3, f32) {
    const EPS: f32 = 1e-6;
    let v1 = Vec3::ZERO;
    let (v2, v3) = (p2 - p1, p3 - p1);
    let cp = vcross2(v1, v2, v3);
    if cp.abs() > EPS {
        let (v1sq, v2sq, v3sq) = (0.0, v2.x * v2.x + v2.z * v2.z, v3.x * v3.x + v3.z * v3.z);
        let c = Vec3::new(
            (v1sq * (v2.z - v3.z) + v2sq * (v3.z - v1.z) + v3sq * (v1.z - v2.z)) / (2.0 * cp),
            0.0,
            (v1sq * (v3.x - v2.x) + v2sq * (v1.x - v3.x) + v3sq * (v2.x - v1.x)) / (2.0 * cp),
        );
        return (c + p1, vdist2(c, v1));
    }
    (p1, 0.0)
}

fn complete_facet(pts: &[Vec3], edges: &mut Vec<[i32; 4]>, max_edges: usize, nfaces: &mut i32, e: usize) {
    const EPS: f32 = 1e-5;
    let edge = edges[e];
    let (s, t) = if edge[2] == EV_UNDEF {
        (edge[0] as usize, edge[1] as usize)
    } else if edge[3] == EV_UNDEF {
        (edge[1] as usize, edge[0] as usize)
    } else {
        return;
    };
    let npts = pts.len();
    let mut pt = npts;
    let mut c = Vec3::ZERO;
    let mut r = -1.0f32;
    for u in 0..npts {
        if u == s || u == t {
            continue;
        }
        if vcross2(pts[s], pts[t], pts[u]) > EPS {
            if r < 0.0 {
                pt = u;
                (c, r) = circum_circle(pts[s], pts[t], pts[u]);
                continue;
            }
            let d = vdist2(c, pts[u]);
            const TOL: f32 = 0.001;
            if d > r * (1.0 + TOL) {
                continue;
            } else if d < r * (1.0 - TOL) {
                pt = u;
                (c, r) = circum_circle(pts[s], pts[t], pts[u]);
            } else {
                if overlap_edges(pts, edges, s, u) || overlap_edges(pts, edges, t, u) {
                    continue;
                }
                pt = u;
                (c, r) = circum_circle(pts[s], pts[t], pts[u]);
            }
        }
    }
    let (s, t) = (s as i32, t as i32);
    if pt < npts {
        let p = pt as i32;
        update_left_face(&mut edges[e], s, t, *nfaces);
        match find_edge(edges, p, s) {
            None => add_edge(edges, max_edges, p, s, *nfaces, EV_UNDEF),
            Some(k) => update_left_face(&mut edges[k], p, s, *nfaces),
        }
        match find_edge(edges, t, p) {
            None => add_edge(edges, max_edges, t, p, *nfaces, EV_UNDEF),
            Some(k) => update_left_face(&mut edges[k], t, p, *nfaces),
        }
        *nfaces += 1;
    } else {
        update_left_face(&mut edges[e], s, t, EV_HULL);
    }
}

/// delaunayHull: the points' Delaunay triangulation inside the hull.
fn delaunay_hull(pts: &[Vec3], hull: &[usize]) -> Vec<[usize; 3]> {
    let max_edges = pts.len() * 10;
    let mut edges: Vec<[i32; 4]> = Vec::new();
    let n = hull.len();
    let mut j = n - 1;
    for i in 0..n {
        add_edge(&mut edges, max_edges, hull[j] as i32, hull[i] as i32, EV_HULL, EV_UNDEF);
        j = i;
    }
    let mut nfaces = 0;
    let mut cur = 0;
    while cur < edges.len() {
        if edges[cur][2] == EV_UNDEF {
            complete_facet(pts, &mut edges, max_edges, &mut nfaces, cur);
        }
        if edges[cur][3] == EV_UNDEF {
            complete_facet(pts, &mut edges, max_edges, &mut nfaces, cur);
        }
        cur += 1;
    }
    let mut tris = vec![[-1i32; 3]; nfaces as usize];
    for e in &edges {
        if e[3] >= 0 {
            let t = &mut tris[e[3] as usize];
            if t[0] == -1 {
                t[0] = e[0];
                t[1] = e[1];
            } else if t[0] == e[1] {
                t[2] = e[0];
            } else if t[1] == e[0] {
                t[2] = e[1];
            }
        }
        if e[2] >= 0 {
            let t = &mut tris[e[2] as usize];
            if t[0] == -1 {
                t[0] = e[1];
                t[1] = e[0];
            } else if t[0] == e[0] {
                t[2] = e[1];
            } else if t[1] == e[1] {
                t[2] = e[0];
            }
        }
    }
    // Dangling faces dropped (Recast swaps the last one in).
    let mut i = 0;
    while i < tris.len() {
        if tris[i].contains(&-1) {
            tris.swap_remove(i);
        } else {
            i += 1;
        }
    }
    tris.into_iter().map(|t| t.map(|v| v as usize)).collect()
}

fn jitter_x(i: usize) -> f32 {
    ((((i as u32).wrapping_mul(0x8da6b343)) & 0xffff) as f32 / 65535.0 * 2.0) - 1.0
}

fn jitter_y(i: usize) -> f32 {
    ((((i as u32).wrapping_mul(0xd8163841)) & 0xffff) as f32 / 65535.0 * 2.0) - 1.0
}

/// The smallest of the polygon's widths (polyMinExtent): its narrowest distance across, edge to far corner.
fn poly_min_extent(verts: &[Vec3]) -> f32 {
    let n = verts.len();
    let mut min_dist = f32::MAX;
    for i in 0..n {
        let ni = (i + 1) % n;
        let (p1, p2) = (verts[i], verts[ni]);
        let mut max_edge_dist = 0.0f32;
        for (j, &v) in verts.iter().enumerate() {
            if j == i || j == ni {
                continue;
            }
            max_edge_dist = max_edge_dist.max(distance_pt_seg_2d(v, p1, p2));
        }
        min_dist = min_dist.min(max_edge_dist);
    }
    min_dist.sqrt()
}

/// buildPolyDetail: the polygon `poly` (polygon space: cells × cell size, heights × cell height) with its edge and
/// inside samples, and its triangles (indices into the polygon's corners, then the added vertices).
fn build_poly_detail(poly: &[Vec3], sample_dist: f32, sample_max_error: f32, cs: f32, ch: f32, hp: &HeightPatch) -> (Vec<Vec3>, Vec<[usize; 3]>) {
    let nin = poly.len();
    let mut verts: Vec<Vec3> = poly.to_vec();
    let mut hull: Vec<usize> = Vec::new();
    let min_extent = poly_min_extent(&verts);
    if sample_dist > 0.0 {
        let mut j = nin - 1;
        for i in 0..nin {
            let (mut vj, mut vi) = (poly[j], poly[i]);
            let mut swapped = false;
            // The same order for an edge both polygons share (lexicographic), or there would be seams.
            if (vj.x - vi.x).abs() < 1e-6 {
                if vj.z > vi.z {
                    std::mem::swap(&mut vj, &mut vi);
                    swapped = true;
                }
            } else if vj.x > vi.x {
                std::mem::swap(&mut vj, &mut vi);
                swapped = true;
            }
            let (dx, dy, dz) = (vi.x - vj.x, vi.y - vj.y, vi.z - vj.z);
            let d = (dx * dx + dz * dz).sqrt();
            let mut nn = 1 + (d / sample_dist).floor() as usize;
            if nn >= MAX_VERTS_PER_EDGE {
                nn = MAX_VERTS_PER_EDGE - 1;
            }
            if verts.len() + nn >= MAX_VERTS {
                nn = MAX_VERTS - 1 - verts.len();
            }
            let edge: Vec<Vec3> = (0..=nn)
                .map(|k| {
                    let u = k as f32 / nn as f32;
                    let p = Vec3::new(vj.x + dx * u, vj.y + dy * u, vj.z + dz * u);
                    Vec3::new(p.x, get_height(p.x, p.y, p.z, cs, ch, hp) as f32 * ch, p.z)
                })
                .collect();
            // Simplify the samples: keep the one straying furthest until none strays more than the error.
            let mut idx: Vec<usize> = vec![0, nn];
            let mut k = 0;
            while k + 1 < idx.len() {
                let (a, b) = (idx[k], idx[k + 1]);
                let (va, vb) = (edge[a], edge[b]);
                let mut maxd = 0.0;
                let mut maxi = None;
                for (m, &e) in edge.iter().enumerate().take(b).skip(a + 1) {
                    let dev = distance_pt_seg(e, va, vb);
                    if dev > maxd {
                        maxd = dev;
                        maxi = Some(m);
                    }
                }
                match maxi {
                    Some(m) if maxd > sample_max_error * sample_max_error => idx.insert(k + 1, m),
                    _ => k += 1,
                }
            }
            hull.push(j);
            let inner: Vec<usize> = if swapped { idx[1..idx.len() - 1].iter().rev().copied().collect() } else { idx[1..idx.len() - 1].to_vec() };
            for k in inner {
                hull.push(verts.len());
                verts.push(edge[k]);
            }
            j = i;
        }
    }
    // Slivers and small triangles: no inside samples.
    if min_extent < sample_dist * 2.0 {
        let tris = triangulate_hull(&verts, &hull);
        return (verts, tris);
    }
    let mut tris = triangulate_hull(&verts, &hull);
    if tris.is_empty() {
        return (verts, tris);
    }
    if sample_dist > 0.0 {
        let (mut bmin, mut bmax) = (poly[0], poly[0]);
        for &v in poly {
            bmin = bmin.min(v);
            bmax = bmax.max(v);
        }
        let (x0, x1) = ((bmin.x / sample_dist).floor() as i32, (bmax.x / sample_dist).ceil() as i32);
        let (z0, z1) = ((bmin.z / sample_dist).floor() as i32, (bmax.z / sample_dist).ceil() as i32);
        let mut samples: Vec<(i32, u16, i32, bool)> = Vec::new();
        for z in z0..z1 {
            for x in x0..x1 {
                let pt = Vec3::new(x as f32 * sample_dist, (bmax.y + bmin.y) * 0.5, z as f32 * sample_dist);
                // Not too near the edges.
                if dist_to_poly(poly, pt) > -sample_dist / 2.0 {
                    continue;
                }
                samples.push((x, get_height(pt.x, pt.y, pt.z, cs, ch, hp), z, false));
            }
        }
        // Add the sample furthest from the triangles until none strays more than the error.
        for _ in 0..samples.len() {
            if verts.len() >= MAX_VERTS {
                break;
            }
            let mut best: Option<(usize, Vec3, f32)> = None;
            for (i, s) in samples.iter().enumerate() {
                if s.3 {
                    continue;
                }
                // Jittered against the grid's symmetric triangulations.
                let pt = Vec3::new(s.0 as f32 * sample_dist + jitter_x(i) * cs * 0.1, s.1 as f32 * ch, s.2 as f32 * sample_dist + jitter_y(i) * cs * 0.1);
                let Some(d) = dist_to_tri_mesh(pt, &verts, &tris) else { continue };
                if best.is_none_or(|b| d > b.2) {
                    best = Some((i, pt, d));
                }
            }
            let Some((bi, bpt, bd)) = best.filter(|b| b.2 > 0.0) else { break };
            if bd <= sample_max_error {
                break;
            }
            samples[bi].3 = true;
            verts.push(bpt);
            tris = delaunay_hull(&verts, &hull);
        }
    }
    tris.truncate(MAX_TRIS);
    (verts, tris)
}

/// A tile polygon's detail mesh: its corners (cells: x, y, z) in region `region` of the compact heightfield. Returns
/// its added vertices (tile cell-space floats: x and z in cells, y in cell heights, with Recast's one cell height
/// lift) and its triangles (indices: the corners first, then the added vertices).
pub(super) fn poly_detail(c: &Compact, reg: &[u32], poly: &[(i32, i32, i32)], region: u32, cs: f32, ch: f32, sample_dist: f32, sample_max_error: f32) -> (Vec<Vec3>, Vec<[u8; 3]>) {
    let hp = height_patch(c, reg, poly, region);
    if hp.w == 0 || hp.h == 0 {
        return (Vec::new(), Vec::new());
    }
    let pv: Vec<Vec3> = poly.iter().map(|&(x, y, z)| Vec3::new(x as f32 * cs, y as f32 * ch, z as f32 * cs)).collect();
    let (verts, tris) = build_poly_detail(&pv, sample_dist, sample_max_error, cs, ch, &hp);
    let extra = verts[poly.len()..].iter().map(|v| Vec3::new(v.x / cs, v.y / ch + 1.0, v.z / cs)).collect();
    (extra, tris.into_iter().map(|t| t.map(|i| i as u8)).collect())
}
