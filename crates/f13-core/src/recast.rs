//! Navmesh generation from the world's blocking triangles, as Unreal's RecastNavMesh builds it at run time (the game
//! sets `RuntimeGeneration=Dynamic`, so its navmesh covers the cabin layout and furniture of the match). A port of
//! Recast's tiled pipeline with the game's settings (DefaultEngine.ini `[/Script/Engine.RecastNavMesh]` and the agent's
//! NavDataConfig): rasterize the triangles into a voxel heightfield, filter it (low hanging obstacles, ledges, low
//! ceilings), compact it, erode it by the agent radius, partition it into monotone regions swept in chunks
//! (`RegionPartitioning=ChunkyMonotone`) and merge the small ones (`MergeRegionSize`), trace and simplify the region
//! contours, and triangulate them into convex polygons of up to six vertices.
//!
//! Each polygon also gets its detail mesh (detail.rs: rcBuildPolyMeshDetail), its triangles following the ground.
//!
//! Not ported: border vertex removal (tile edges are matched by overlap, see navmesh.rs), area types.
//!
//! Space: Y up, metres (any unit works if the settings use it).

mod detail;

use crate::collision::{Tri, TriSource};
use glam::Vec3;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

/// The build settings, in world units.
#[derive(Clone, Debug)]
pub struct NavConfig {
    /// Cell size (XZ) and height.
    pub cell_size: f32,
    pub cell_height: f32,
    pub agent_height: f32,
    pub agent_radius: f32,
    /// Highest step the agent climbs.
    pub agent_max_climb: f32,
    pub agent_max_slope_deg: f32,
    /// Tile side, in cells.
    pub tile_cells: i32,
    /// Contour simplification error (cells) and longest wall edge (cells).
    pub max_simplification_error: f32,
    pub max_edge_len: i32,
    pub max_verts_per_poly: usize,
    /// Regions smaller than this (cells) merge into a neighbour (from UE's `MergeRegionSize`; see navigation.rs).
    pub merge_region_area: usize,
    /// ChunkyMonotone partitioning: the tile is swept in this many chunks per side (UE's `RegionChunkSplits`; 1 is
    /// plain monotone).
    pub region_chunk_splits: i32,
    /// Detail meshes: the sample spacing and the largest height error kept (world units; 0 spacing: none).
    pub detail_sample_dist: f32,
    pub detail_sample_max_error: f32,
}

impl NavConfig {
    fn walkable_height(&self) -> i32 {
        (self.agent_height / self.cell_height).ceil() as i32
    }
    fn walkable_climb(&self) -> i32 {
        (self.agent_max_climb / self.cell_height).floor() as i32
    }
    fn walkable_radius(&self) -> i32 {
        (self.agent_radius / self.cell_size).ceil() as i32
    }
    fn border(&self) -> i32 {
        self.walkable_radius() + 3
    }
    /// A tile's side in world units.
    pub fn tile_size(&self) -> f32 {
        self.tile_cells as f32 * self.cell_size
    }
}

/// One tile's polygons: world-space vertices and convex polygons (vertex indices, at most `max_verts_per_poly`), and
/// each polygon's detail mesh: its added vertices (world) and triangles (indices: the polygon's corners in order,
/// then its added vertices; none when it has no detail).
#[derive(Clone, Debug, Default)]
pub struct TileMesh {
    pub tx: i32,
    pub tz: i32,
    pub verts: Vec<Vec3>,
    pub polys: Vec<Vec<u32>>,
    pub detail: Vec<(Vec<Vec3>, Vec<[u8; 3]>)>,
}

const NULL_AREA: u8 = 0;
const WALKABLE: u8 = 63;
const MAX_HEIGHT: i32 = 0xffff;
const NOT_CONNECTED: u32 = u32::MAX;
const BORDER_REG: u32 = 0x8000;
const BORDER_VERTEX: u32 = 0x10000;
const AREA_BORDER: u32 = 0x20000;
const REG_MASK: u32 = 0xffff;
const DX: [i32; 4] = [-1, 0, 1, 0];
const DZ: [i32; 4] = [0, 1, 0, -1];

#[derive(Clone, Copy)]
struct Span {
    smin: i32,
    smax: i32,
    area: u8,
}

/// A voxel heightfield: per column, its solid spans sorted bottom up.
struct Heightfield {
    w: i32,
    h: i32,
    bmin: Vec3,
    cs: f32,
    ch: f32,
    cols: Vec<Vec<Span>>,
}

impl Heightfield {
    fn add_span(&mut self, x: i32, z: i32, smin: i32, smax: i32, area: u8, merge_thr: i32) {
        let col = &mut self.cols[(x + z * self.w) as usize];
        let mut s = Span { smin, smax, area };
        let mut i = 0;
        while i < col.len() {
            let cur = col[i];
            if cur.smin > s.smax {
                break;
            }
            if cur.smax < s.smin {
                i += 1;
                continue;
            }
            // Merge.
            s.smin = s.smin.min(cur.smin);
            s.smax = s.smax.max(cur.smax);
            if (s.smax - cur.smax).abs() <= merge_thr {
                s.area = s.area.max(cur.area);
            }
            col.remove(i);
        }
        col.insert(i, s);
    }
}

/// A clipped polygon on the stack (a triangle clipped by four lines has at most 7 vertices).
#[derive(Clone, Copy)]
struct Clip {
    v: [Vec3; 12],
    n: usize,
}

impl Clip {
    const EMPTY: Clip = Clip { v: [Vec3::ZERO; 12], n: 0 };

    fn push(&mut self, p: Vec3) {
        if self.n < self.v.len() {
            self.v[self.n] = p;
            self.n += 1;
        }
    }
}

/// Splits a convex polygon by the line `axis = x` into the parts below (`a`) and above (`b`) it (Recast's dividePoly).
fn divide_poly(input: &Clip, x: f32, axis: usize, a: &mut Clip, b: &mut Clip) {
    a.n = 0;
    b.n = 0;
    let n = input.n;
    if n == 0 {
        return;
    }
    let mut d = [0f32; 12];
    for i in 0..n {
        d[i] = x - input.v[i][axis];
    }
    let mut j = n - 1;
    for i in 0..n {
        let ina = d[j] >= 0.0;
        let inb = d[i] >= 0.0;
        if ina != inb {
            let s = d[j] / (d[j] - d[i]);
            let p = input.v[j] + (input.v[i] - input.v[j]) * s;
            a.push(p);
            b.push(p);
            // Points on the dividing line were added above.
            if d[i] > 0.0 {
                a.push(input.v[i]);
            } else if d[i] < 0.0 {
                b.push(input.v[i]);
            }
        } else {
            // Same side; a point on the line goes to both.
            if d[i] >= 0.0 {
                a.push(input.v[i]);
                if d[i] != 0.0 {
                    j = i;
                    continue;
                }
            }
            b.push(input.v[i]);
        }
        j = i;
    }
}

fn rasterize_tri(hf: &mut Heightfield, t: &Tri, area: u8, merge_thr: i32) {
    let (cs, ch) = (hf.cs, hf.ch);
    let ics = 1.0 / cs;
    let bmin = hf.bmin;
    let bmax_x = bmin.x + hf.w as f32 * cs;
    let bmax_z = bmin.z + hf.h as f32 * cs;
    let tmin = t.min();
    let tmax = t.max();
    if tmax.x < bmin.x || tmin.x > bmax_x || tmax.z < bmin.z || tmin.z > bmax_z {
        return;
    }
    let by = MAX_HEIGHT as f32 * ch;
    let z0 = (((tmin.z - bmin.z) * ics) as i32).clamp(-1, hf.h - 1);
    let z1 = (((tmax.z - bmin.z) * ics) as i32).clamp(0, hf.h - 1);
    let mut rest = Clip::EMPTY;
    rest.push(t.a);
    rest.push(t.b);
    rest.push(t.c);
    let (mut row, mut next, mut cell, mut rnext) = (Clip::EMPTY, Clip::EMPTY, Clip::EMPTY, Clip::EMPTY);
    for z in z0..=z1 {
        let cz = bmin.z + (z + 1) as f32 * cs;
        divide_poly(&rest, cz, 2, &mut row, &mut next);
        std::mem::swap(&mut rest, &mut next);
        if row.n < 3 || z < 0 {
            continue;
        }
        let (mut minx, mut maxx) = (row.v[0].x, row.v[0].x);
        for v in &row.v[1..row.n] {
            minx = minx.min(v.x);
            maxx = maxx.max(v.x);
        }
        let x0i = ((minx - bmin.x) * ics) as i32;
        let x1i = ((maxx - bmin.x) * ics) as i32;
        if x1i < 0 || x0i >= hf.w {
            continue;
        }
        let x0 = x0i.clamp(-1, hf.w - 1);
        let x1 = x1i.clamp(0, hf.w - 1);
        let mut rrest = row;
        for x in x0..=x1 {
            let cx = bmin.x + (x + 1) as f32 * cs;
            divide_poly(&rrest, cx, 0, &mut cell, &mut rnext);
            std::mem::swap(&mut rrest, &mut rnext);
            if cell.n < 3 || x < 0 {
                continue;
            }
            let (mut smin, mut smax) = (cell.v[0].y, cell.v[0].y);
            for v in &cell.v[1..cell.n] {
                smin = smin.min(v.y);
                smax = smax.max(v.y);
            }
            smin -= bmin.y;
            smax -= bmin.y;
            if smax < 0.0 || smin > by {
                continue;
            }
            let smin = smin.max(0.0);
            let smax = smax.min(by);
            let ismin = ((smin / ch).floor() as i32).clamp(0, MAX_HEIGHT);
            let ismax = ((smax / ch).ceil() as i32).clamp(ismin + 1, MAX_HEIGHT);
            hf.add_span(x, z, ismin, ismax, area, merge_thr);
        }
    }
}

fn filter_low_hanging_obstacles(hf: &mut Heightfield, climb: i32) {
    for col in &mut hf.cols {
        let mut prev_walkable = false;
        let mut prev_area = NULL_AREA;
        let mut prev_max = 0;
        for s in col.iter_mut() {
            let walkable = s.area != NULL_AREA;
            // A non-walkable span just above a walkable one (within climb) becomes walkable.
            if !walkable && prev_walkable && (s.smax - prev_max).abs() <= climb {
                s.area = prev_area;
            }
            prev_walkable = walkable;
            prev_area = s.area;
            prev_max = s.smax;
        }
    }
}

fn filter_ledges(hf: &mut Heightfield, height: i32, climb: i32) {
    let (w, h) = (hf.w, hf.h);
    let mut cleared = Vec::new();
    for z in 0..h {
        for x in 0..w {
            let col = &hf.cols[(x + z * w) as usize];
            for (k, s) in col.iter().enumerate() {
                if s.area == NULL_AREA {
                    continue;
                }
                let bot = s.smax;
                let top = col.get(k + 1).map_or(MAX_HEIGHT, |n| n.smin);
                let mut minh = MAX_HEIGHT;
                let (mut asmin, mut asmax) = (s.smax, s.smax);
                for d in 0..4 {
                    let (nx, nz) = (x + DX[d], z + DZ[d]);
                    if nx < 0 || nz < 0 || nx >= w || nz >= h {
                        minh = minh.min(-climb - bot);
                        continue;
                    }
                    let ncol = &hf.cols[(nx + nz * w) as usize];
                    // From the floor down to the first span.
                    let nbot = -climb;
                    let ntop = ncol.first().map_or(MAX_HEIGHT, |n| n.smin);
                    if top.min(ntop) - bot.max(nbot) > height {
                        minh = minh.min(nbot - bot);
                    }
                    for (nk, ns) in ncol.iter().enumerate() {
                        let nbot = ns.smax;
                        let ntop = ncol.get(nk + 1).map_or(MAX_HEIGHT, |n| n.smin);
                        if top.min(ntop) - bot.max(nbot) > height {
                            minh = minh.min(nbot - bot);
                            if (nbot - bot).abs() <= climb {
                                asmin = asmin.min(nbot);
                                asmax = asmax.max(nbot);
                            }
                        }
                    }
                }
                // A drop to a ledge, or too steep a step between the accessible neighbours.
                if minh < -climb || asmax - asmin > climb {
                    cleared.push(((x + z * w) as usize, k));
                }
            }
        }
    }
    for (c, k) in cleared {
        hf.cols[c][k].area = NULL_AREA;
    }
}

fn filter_low_ceilings(hf: &mut Heightfield, height: i32) {
    for col in &mut hf.cols {
        for k in 0..col.len() {
            let top = col.get(k + 1).map_or(MAX_HEIGHT, |n| n.smin);
            if top - col[k].smax < height {
                col[k].area = NULL_AREA;
            }
        }
    }
}

/// A run of spans along a row in the monotone partitioning: its region, and the region of the row before it touches
/// (`NULL_NEI`: more than one) with how many spans.
#[derive(Clone, Copy, Default)]
struct Sweep {
    id: u32,
    ns: u32,
    nei: u32,
}
const NULL_NEI: u32 = 0xffff_ffff;

/// The walkable spans only, with their neighbour links.
struct Compact {
    w: i32,
    h: i32,
    /// Per column: first span index and count.
    cells: Vec<(u32, u32)>,
    y: Vec<i32>,
    area: Vec<u8>,
    con: Vec<[u32; 4]>,
}

impl Compact {
    fn build(hf: &Heightfield, height: i32, climb: i32) -> Compact {
        let (w, h) = (hf.w, hf.h);
        let mut c = Compact { w, h, cells: Vec::with_capacity((w * h) as usize), y: Vec::new(), area: Vec::new(), con: Vec::new() };
        let mut tops = Vec::new();
        for col in &hf.cols {
            let first = c.y.len() as u32;
            for (k, s) in col.iter().enumerate() {
                if s.area == NULL_AREA {
                    continue;
                }
                let top = col.get(k + 1).map_or(MAX_HEIGHT, |n| n.smin);
                c.y.push(s.smax);
                tops.push(top);
                c.area.push(s.area);
                c.con.push([NOT_CONNECTED; 4]);
            }
            c.cells.push((first, c.y.len() as u32 - first));
        }
        for z in 0..h {
            for x in 0..w {
                let (first, count) = c.cells[(x + z * w) as usize];
                for i in first..first + count {
                    let (y, top) = (c.y[i as usize], tops[i as usize]);
                    for d in 0..4 {
                        let (nx, nz) = (x + DX[d], z + DZ[d]);
                        if nx < 0 || nz < 0 || nx >= w || nz >= h {
                            continue;
                        }
                        let (nf, nc) = c.cells[(nx + nz * w) as usize];
                        for k in nf..nf + nc {
                            let (ny, ntop) = (c.y[k as usize], tops[k as usize]);
                            let bot = y.max(ny);
                            let t = top.min(ntop);
                            if t - bot >= height && (ny - y).abs() <= climb {
                                c.con[i as usize][d] = k;
                                break;
                            }
                        }
                    }
                }
            }
        }
        c
    }

    fn span_count(&self) -> usize {
        self.y.len()
    }

    /// The neighbour of span `i` in direction `d`.
    fn nei(&self, i: usize, d: usize) -> Option<usize> {
        let k = self.con[i][d];
        (k != NOT_CONNECTED).then_some(k as usize)
    }

    /// Unwalkable within `radius` cells of an edge (Recast's distance transform, 2 per straight and 3 per diagonal step).
    fn erode(&mut self, radius: i32) {
        let n = self.span_count();
        let mut dist = vec![255u8; n];
        for i in 0..n {
            if self.area[i] == NULL_AREA {
                dist[i] = 0;
                continue;
            }
            let links = (0..4).filter(|&d| self.nei(i, d).is_some_and(|k| self.area[k] != NULL_AREA)).count();
            if links != 4 {
                dist[i] = 0;
            }
        }
        let relax = |dist: &mut Vec<u8>, i: usize, k: usize, cost: u8| {
            let nd = dist[k].saturating_add(cost);
            if nd < dist[i] {
                dist[i] = nd;
            }
        };
        let (w, h) = (self.w, self.h);
        for z in 0..h {
            for x in 0..w {
                let (f, c) = self.cells[(x + z * w) as usize];
                for i in f as usize..(f + c) as usize {
                    if let Some(a) = self.nei(i, 0) {
                        relax(&mut dist, i, a, 2);
                        if let Some(aa) = self.nei(a, 3) {
                            relax(&mut dist, i, aa, 3);
                        }
                    }
                    if let Some(a) = self.nei(i, 3) {
                        relax(&mut dist, i, a, 2);
                        if let Some(aa) = self.nei(a, 2) {
                            relax(&mut dist, i, aa, 3);
                        }
                    }
                }
            }
        }
        for z in (0..h).rev() {
            for x in (0..w).rev() {
                let (f, c) = self.cells[(x + z * w) as usize];
                for i in f as usize..(f + c) as usize {
                    if let Some(a) = self.nei(i, 2) {
                        relax(&mut dist, i, a, 2);
                        if let Some(aa) = self.nei(a, 1) {
                            relax(&mut dist, i, aa, 3);
                        }
                    }
                    if let Some(a) = self.nei(i, 1) {
                        relax(&mut dist, i, a, 2);
                        if let Some(aa) = self.nei(a, 0) {
                            relax(&mut dist, i, aa, 3);
                        }
                    }
                }
            }
        }
        let thr = (radius * 2).min(255) as u8;
        for i in 0..n {
            if dist[i] < thr {
                self.area[i] = NULL_AREA;
            }
        }
    }

    /// Monotone regions (rcBuildRegionsMonotone), the border cells marked as border regions. With `chunks` > 1, UE's
    /// ChunkyMonotone: the tile's inside is cut into `chunks` x `chunks` rectangles swept separately.
    fn regions_monotone(&self, border: i32, chunks: i32) -> Vec<u32> {
        let (w, h) = (self.w, self.h);
        let mut reg = vec![0u32; self.span_count()];
        let mut id: u32 = 1;
        let paint = |reg: &mut Vec<u32>, x0: i32, x1: i32, z0: i32, z1: i32, r: u32| {
            for z in z0..z1 {
                for x in x0..x1 {
                    let (f, c) = self.cells[(x + z * w) as usize];
                    for i in f as usize..(f + c) as usize {
                        if self.area[i] != NULL_AREA {
                            reg[i] = r;
                        }
                    }
                }
            }
        };
        if border > 0 {
            let bw = w.min(border);
            let bh = h.min(border);
            paint(&mut reg, 0, bw, 0, h, id | BORDER_REG);
            id += 1;
            paint(&mut reg, w - bw, w, 0, h, id | BORDER_REG);
            id += 1;
            paint(&mut reg, 0, w, 0, bh, id | BORDER_REG);
            id += 1;
            paint(&mut reg, 0, w, h - bh, h, id | BORDER_REG);
            id += 1;
        }
        let mut sweeps: Vec<Sweep> = Vec::new();
        let mut prev: Vec<u32> = Vec::new();
        let chunks = chunks.max(1);
        let (cw, ch) = ((w - 2 * border + chunks - 1) / chunks, (h - 2 * border + chunks - 1) / chunks);
        for (cx, cz) in (0..chunks).flat_map(|cz| (0..chunks).map(move |cx| (cx, cz))) {
            let (x0, x1) = (border + cx * cw, (border + (cx + 1) * cw).min(w - border));
            let (z0, z1) = (border + cz * ch, (border + (cz + 1) * ch).min(h - border));
            self.sweep(&mut reg, &mut id, &mut sweeps, &mut prev, x0, x1, z0, z1);
        }
        reg
    }

    /// One rectangle of the monotone partitioning: rows of spans become sweeps, a sweep joined one-to-one with a
    /// region of the row before keeps it, the others start new ones. Neighbours outside the rectangle don't count.
    #[allow(clippy::too_many_arguments)]
    fn sweep(&self, reg: &mut [u32], id: &mut u32, sweeps: &mut Vec<Sweep>, prev: &mut Vec<u32>, x0: i32, x1: i32, z0: i32, z1: i32) {
        let w = self.w;
        for z in z0..z1 {
            // Collect the row's spans into sweeps.
            prev.iter_mut().for_each(|p| *p = 0);
            let mut rid: u32 = 1;
            for x in x0..x1 {
                let (f, c) = self.cells[(x + z * w) as usize];
                for i in f as usize..(f + c) as usize {
                    if self.area[i] == NULL_AREA {
                        continue;
                    }
                    // -x (within the rectangle)
                    let mut previd = 0;
                    if let Some(a) = self.nei(i, 0).filter(|_| x > x0) {
                        if reg[a] & BORDER_REG == 0 && self.area[i] == self.area[a] {
                            previd = reg[a];
                        }
                    }
                    if previd == 0 {
                        previd = rid;
                        rid += 1;
                        if sweeps.len() <= previd as usize {
                            sweeps.resize(previd as usize + 1, Sweep::default());
                        }
                        sweeps[previd as usize] = Sweep { id: 0, ns: 0, nei: 0 };
                    }
                    // -z (within the rectangle)
                    if let Some(a) = self.nei(i, 3).filter(|_| z > z0) {
                        let nr = reg[a];
                        if nr != 0 && nr & BORDER_REG == 0 && self.area[i] == self.area[a] {
                            let s = &mut sweeps[previd as usize];
                            if s.nei == 0 || s.nei == nr {
                                s.nei = nr;
                                s.ns += 1;
                                if prev.len() <= nr as usize {
                                    prev.resize(nr as usize + 1, 0);
                                }
                                prev[nr as usize] += 1;
                            } else {
                                s.nei = NULL_NEI;
                            }
                        }
                    }
                    reg[i] = previd;
                }
            }
            // Sweeps joined one-to-one with the row above keep its region; the others get a new one.
            for i in 1..rid as usize {
                let s = sweeps[i];
                if s.nei != NULL_NEI && s.nei != 0 && prev.get(s.nei as usize).copied() == Some(s.ns) {
                    sweeps[i].id = s.nei;
                } else {
                    sweeps[i].id = *id;
                    *id += 1;
                }
            }
            for x in x0..x1 {
                let (f, c) = self.cells[(x + z * w) as usize];
                for i in f as usize..(f + c) as usize {
                    if reg[i] > 0 && reg[i] < rid {
                        reg[i] = sweeps[reg[i] as usize].id;
                    }
                }
            }
        }
    }

    /// The regions around region `reg[i]`, walking its outline from span `i`'s edge `dir` (Recast's region
    /// walkContour): neighbour ids in order, 0 for a wall, without repeats.
    fn region_outline(&self, reg: &[u32], mut i: usize, mut dir: usize) -> Vec<u32> {
        let (start_i, start_dir) = (i, dir);
        let across = |i: usize, d: usize| self.nei(i, d).map_or(0, |a| reg[a]);
        let mut cur = across(i, dir);
        let mut out = vec![cur];
        for _ in 0..40000 {
            if across(i, dir) != reg[i] {
                let r = across(i, dir);
                if r != cur {
                    cur = r;
                    out.push(cur);
                }
                dir = (dir + 1) & 3;
            } else {
                let Some(n) = self.nei(i, dir) else { return out };
                i = n;
                dir = (dir + 3) & 3;
            }
            if i == start_i && dir == start_dir {
                break;
            }
        }
        dedup_ring(&mut out);
        out
    }

    /// Recast's mergeAndFilterRegions (minimum region area 0): a region smaller than `merge_area` cells, or one with
    /// no wall around it, joins its smallest neighbour of the same area that it borders along one stretch only and
    /// that isn't above or below it, until nothing merges.
    fn merge_regions(&self, reg: &mut [u32], merge_area: usize) {
        #[derive(Default)]
        struct Region {
            spans: usize,
            id: u32,
            area: u8,
            overlap: bool,
            connections: Vec<u32>,
            floors: Vec<u32>,
        }
        let nreg = reg.iter().filter(|r| **r & BORDER_REG == 0).max().map_or(0, |m| *m as usize + 1);
        let mut regions: Vec<Region> = (0..nreg).map(|i| Region { id: i as u32, ..Default::default() }).collect();
        for z in 0..self.h {
            for x in 0..self.w {
                let (f, c) = self.cells[(x + z * self.w) as usize];
                for i in f as usize..(f + c) as usize {
                    let r = reg[i] as usize;
                    if r == 0 || r >= nreg {
                        continue;
                    }
                    regions[r].spans += 1;
                    for j in f as usize..(f + c) as usize {
                        let floor = reg[j];
                        if i == j || floor == 0 || floor as usize >= nreg {
                            continue;
                        }
                        if floor as usize == r {
                            regions[r].overlap = true;
                        }
                        if !regions[r].floors.contains(&floor) {
                            regions[r].floors.push(floor);
                        }
                    }
                    if !regions[r].connections.is_empty() {
                        continue;
                    }
                    regions[r].area = self.area[i];
                    if let Some(d) = (0..4).find(|&d| self.nei(i, d).map_or(0, |a| reg[a]) != reg[i]) {
                        regions[r].connections = self.region_outline(reg, i, d);
                    }
                }
            }
        }
        let can_merge = |a: &Region, b: &Region| a.area == b.area && a.connections.iter().filter(|&&c| c == b.id).count() <= 1 && !a.floors.contains(&b.id);
        loop {
            let mut merged = 0;
            for i in 0..nreg {
                let r = &regions[i];
                if r.id == 0 || r.id & BORDER_REG != 0 || r.overlap || r.spans == 0 {
                    continue;
                }
                if r.spans > merge_area && r.connections.contains(&0) {
                    continue;
                }
                // The smallest neighbour it can join.
                let mut target = None;
                let mut smallest = usize::MAX;
                for &c in &r.connections {
                    if c & BORDER_REG != 0 || c as usize >= nreg {
                        continue;
                    }
                    let m = &regions[c as usize];
                    if m.id == 0 || m.id & BORDER_REG != 0 || m.overlap {
                        continue;
                    }
                    if m.spans < smallest && can_merge(r, m) && can_merge(m, r) {
                        smallest = m.spans;
                        target = Some(m.id);
                    }
                }
                let Some(to) = target.filter(|&t| t != r.id) else { continue };
                let old = r.id;
                // Splice the outlines together where they meet.
                let (acon, bcon) = (regions[to as usize].connections.clone(), regions[i].connections.clone());
                let (Some(ia), Some(ib)) = (acon.iter().position(|&c| c == old), bcon.iter().position(|&c| c == to)) else { continue };
                let mut joined: Vec<u32> = (0..acon.len() - 1).map(|k| acon[(ia + 1 + k) % acon.len()]).collect();
                joined.extend((0..bcon.len() - 1).map(|k| bcon[(ib + 1 + k) % bcon.len()]));
                dedup_ring(&mut joined);
                let floors = std::mem::take(&mut regions[i].floors);
                let spans = std::mem::take(&mut regions[i].spans);
                regions[i].connections.clear();
                let t = &mut regions[to as usize];
                t.connections = joined;
                for f in floors {
                    if !t.floors.contains(&f) {
                        t.floors.push(f);
                    }
                }
                t.spans += spans;
                // Everything that pointed at the merged region now points at the one it joined.
                for g in regions.iter_mut() {
                    if g.id == 0 || g.id & BORDER_REG != 0 {
                        continue;
                    }
                    if g.id == old {
                        g.id = to;
                    }
                    let mut changed = false;
                    for c in g.connections.iter_mut() {
                        if *c == old {
                            *c = to;
                            changed = true;
                        }
                    }
                    for f in g.floors.iter_mut() {
                        if *f == old {
                            *f = to;
                        }
                    }
                    if changed {
                        dedup_ring(&mut g.connections);
                    }
                }
                merged += 1;
            }
            if merged == 0 {
                break;
            }
        }
        for r in reg.iter_mut() {
            if *r & BORDER_REG == 0 && (*r as usize) < nreg {
                *r = regions[*r as usize].id;
            }
        }
    }
}

/// Drops repeats next to each other in a ring (the last and the first are next to each other too).
fn dedup_ring(v: &mut Vec<u32>) {
    let mut j = 0;
    while v.len() > 1 && j < v.len() {
        if v[j] == v[(j + 1) % v.len()] {
            v.remove(j);
        } else {
            j += 1;
        }
    }
}

/// A traced contour vertex: cell coordinates (x, height, z) and flags (neighbour region, border vertex, area border).
#[derive(Clone, Copy, Debug)]
struct CVert {
    x: i32,
    y: i32,
    z: i32,
    r: u32,
}

fn corner_height(c: &Compact, reg: &[u32], i: usize, dir: usize) -> (i32, bool) {
    let mut ch = c.y[i];
    let dirp = (dir + 1) & 3;
    let mut regs = [0u32; 4];
    let key = |k: usize| reg[k] | ((c.area[k] as u32) << 16);
    regs[0] = key(i);
    if let Some(a) = c.nei(i, dir) {
        ch = ch.max(c.y[a]);
        regs[1] = key(a);
        if let Some(b) = c.nei(a, dirp) {
            ch = ch.max(c.y[b]);
            regs[2] = key(b);
        }
    }
    if let Some(a) = c.nei(i, dirp) {
        ch = ch.max(c.y[a]);
        regs[3] = key(a);
        if let Some(b) = c.nei(a, dir) {
            ch = ch.max(c.y[b]);
            regs[2] = key(b);
        }
    }
    let mut border = false;
    for j in 0..4 {
        let (a, b, cc, d) = (j, (j + 1) & 3, (j + 2) & 3, (j + 3) & 3);
        let two_same_exts = (regs[a] & regs[b] & BORDER_REG) != 0 && regs[a] == regs[b];
        let two_ints = ((regs[cc] | regs[d]) & BORDER_REG) == 0;
        let ints_same_area = (regs[cc] >> 16) == (regs[d] >> 16);
        let no_zeros = regs[a] != 0 && regs[b] != 0 && regs[cc] != 0 && regs[d] != 0;
        if two_same_exts && two_ints && ints_same_area && no_zeros {
            border = true;
            break;
        }
    }
    (ch, border)
}

fn walk_contour(c: &Compact, reg: &[u32], mut x: i32, mut z: i32, mut i: usize, flags: &mut [u8]) -> Vec<CVert> {
    let mut dir = 0;
    while flags[i] & (1 << dir) == 0 {
        dir += 1;
    }
    let (start_dir, start_i) = (dir, i);
    let area = c.area[i];
    let mut out = Vec::new();
    for _ in 0..40000 {
        if flags[i] & (1 << dir) != 0 {
            let (py, is_border) = corner_height(c, reg, i, dir);
            let (mut px, mut pz) = (x, z);
            match dir {
                0 => pz += 1,
                1 => {
                    px += 1;
                    pz += 1
                }
                2 => px += 1,
                _ => {}
            }
            let mut r = 0;
            let mut area_border = false;
            if let Some(a) = c.nei(i, dir) {
                r = reg[a];
                if area != c.area[a] {
                    area_border = true;
                }
            }
            if is_border {
                r |= BORDER_VERTEX;
            }
            if area_border {
                r |= AREA_BORDER;
            }
            out.push(CVert { x: px, y: py, z: pz, r });
            flags[i] &= !(1 << dir);
            dir = (dir + 1) & 3; // clockwise
        } else {
            let Some(ni) = c.nei(i, dir) else { break };
            x += DX[dir];
            z += DZ[dir];
            i = ni;
            dir = (dir + 3) & 3; // counter-clockwise
        }
        if start_i == i && start_dir == dir {
            break;
        }
    }
    out
}

fn dist_pt_seg_sq(x: i32, z: i32, px: i32, pz: i32, qx: i32, qz: i32) -> f32 {
    let (pqx, pqz) = ((qx - px) as f32, (qz - pz) as f32);
    let (dx, dz) = ((x - px) as f32, (z - pz) as f32);
    let d = pqx * pqx + pqz * pqz;
    let mut t = pqx * dx + pqz * dz;
    if d > 0.0 {
        t /= d;
    }
    let t = t.clamp(0.0, 1.0);
    let (ex, ez) = (px as f32 + t * pqx - x as f32, pz as f32 + t * pqz - z as f32);
    ex * ex + ez * ez
}

/// Simplified contour: (vertex, index of the raw vertex it came from).
fn simplify_contour(pts: &[CVert], max_error: f32, max_edge_len: i32) -> Vec<CVert> {
    let pn = pts.len();
    let mut s: Vec<(CVert, usize)> = Vec::new();
    let has_connections = pts.iter().any(|p| p.r & REG_MASK != 0);
    if has_connections {
        for i in 0..pn {
            let ii = (i + 1) % pn;
            let different_regs = (pts[i].r & REG_MASK) != (pts[ii].r & REG_MASK);
            let area_borders = (pts[i].r & AREA_BORDER) != (pts[ii].r & AREA_BORDER);
            if different_regs || area_borders {
                s.push((pts[i], i));
            }
        }
    }
    if s.is_empty() {
        // Lower left and upper right vertices.
        let (mut ll, mut ur) = (0, 0);
        for (i, p) in pts.iter().enumerate() {
            let (l, u) = (&pts[ll], &pts[ur]);
            if p.x < l.x || (p.x == l.x && p.z < l.z) {
                ll = i;
            }
            if p.x > u.x || (p.x == u.x && p.z > u.z) {
                ur = i;
            }
        }
        s.push((pts[ll], ll));
        s.push((pts[ur], ur));
    }
    // Add points until every raw point is within the error.
    let mut i = 0;
    while i < s.len() {
        let ii = (i + 1) % s.len();
        let (mut a, ai) = (s[i].0, s[i].1);
        let (mut b, bi) = (s[ii].0, s[ii].1);
        let (mut maxd, mut maxi) = (0.0f32, None);
        // Walk the segment in lexicographic order so both directions give the same result.
        let (cinc, mut ci, endi) = if b.x > a.x || (b.x == a.x && b.z > a.z) {
            (1, (ai + 1) % pn, bi)
        } else {
            std::mem::swap(&mut a, &mut b);
            (pn - 1, (bi + pn - 1) % pn, ai)
        };
        // Every edge is tessellated, borders between regions too (Recast only does walls and area borders): after
        // region merging a border between two regions can wind round a wall end (outside a cabin, through its doorway,
        // inside), and drawn straight it put both regions' polygons across the wall. The lexicographic walk makes both
        // regions pick the same points, so their edges still match.
        {
            while ci != endi {
                let d = dist_pt_seg_sq(pts[ci].x, pts[ci].z, a.x, a.z, b.x, b.z);
                if d > maxd {
                    maxd = d;
                    maxi = Some(ci);
                }
                ci = (ci + cinc) % pn;
            }
        }
        match maxi {
            Some(m) if maxd > max_error * max_error => s.insert(i + 1, (pts[m], m)),
            _ => i += 1,
        }
    }
    // Split wall edges longer than the limit.
    if max_edge_len > 0 {
        let mut i = 0;
        while i < s.len() {
            let ii = (i + 1) % s.len();
            let (a, ai) = s[i];
            let (b, bi) = s[ii];
            let ci = (ai + 1) % pn;
            let mut maxi = None;
            if pts[ci].r & REG_MASK == 0 {
                let (dx, dz) = (b.x - a.x, b.z - a.z);
                if dx * dx + dz * dz > max_edge_len * max_edge_len {
                    let n = if bi < ai { bi + pn - ai } else { bi - ai };
                    if n > 1 {
                        maxi = Some(if b.x > a.x || (b.x == a.x && b.z > a.z) { (ai + n / 2) % pn } else { (ai + n.div_ceil(2)) % pn });
                    }
                }
            }
            match maxi {
                Some(m) => s.insert(i + 1, (pts[m], m)),
                None => i += 1,
            }
        }
    }
    // Each vertex takes its region from the next raw vertex and its border flag from its own.
    let mut out: Vec<CVert> = s
        .iter()
        .map(|&(v, idx)| {
            let ai = (idx + 1) % pn;
            CVert { r: (pts[ai].r & (REG_MASK | AREA_BORDER)) | (pts[idx].r & BORDER_VERTEX), ..v }
        })
        .collect();
    // Remove degenerate segments (consecutive vertices at the same x, z).
    let mut i = 0;
    while out.len() > 1 && i < out.len() {
        let ni = (i + 1) % out.len();
        if out[i].x == out[ni].x && out[i].z == out[ni].z {
            out.remove(ni);
        } else {
            i += 1;
        }
    }
    out
}

// --- Triangulation (Recast's ear clipping on contour vertices, in cell XZ). ---

fn area2(a: (i32, i32), b: (i32, i32), c: (i32, i32)) -> i64 {
    (b.0 - a.0) as i64 * (c.1 - a.1) as i64 - (c.0 - a.0) as i64 * (b.1 - a.1) as i64
}
fn left(a: (i32, i32), b: (i32, i32), c: (i32, i32)) -> bool {
    area2(a, b, c) < 0
}
fn left_on(a: (i32, i32), b: (i32, i32), c: (i32, i32)) -> bool {
    area2(a, b, c) <= 0
}
fn collinear(a: (i32, i32), b: (i32, i32), c: (i32, i32)) -> bool {
    area2(a, b, c) == 0
}
fn intersect_prop(a: (i32, i32), b: (i32, i32), c: (i32, i32), d: (i32, i32)) -> bool {
    if collinear(a, b, c) || collinear(a, b, d) || collinear(c, d, a) || collinear(c, d, b) {
        return false;
    }
    (left(a, b, c) ^ left(a, b, d)) && (left(c, d, a) ^ left(c, d, b))
}
fn between(a: (i32, i32), b: (i32, i32), c: (i32, i32)) -> bool {
    if !collinear(a, b, c) {
        return false;
    }
    if a.0 != b.0 {
        (a.0 <= c.0 && c.0 <= b.0) || (a.0 >= c.0 && c.0 >= b.0)
    } else {
        (a.1 <= c.1 && c.1 <= b.1) || (a.1 >= c.1 && c.1 >= b.1)
    }
}
fn intersect(a: (i32, i32), b: (i32, i32), c: (i32, i32), d: (i32, i32)) -> bool {
    intersect_prop(a, b, c, d) || between(a, b, c) || between(a, b, d) || between(c, d, a) || between(c, d, b)
}

struct Tri2<'a> {
    v: &'a [(i32, i32)],
}

impl Tri2<'_> {
    fn p(&self, idx: &[usize], i: usize) -> (i32, i32) {
        self.v[idx[i] & 0x0fff_ffff]
    }
    fn diagonalie(&self, i: usize, j: usize, idx: &[usize], loose: bool) -> bool {
        let n = idx.len();
        let (d0, d1) = (self.p(idx, i), self.p(idx, j));
        for k in 0..n {
            let k1 = (k + 1) % n;
            if k == i || k1 == i || k == j || k1 == j {
                continue;
            }
            let (p0, p1) = (self.p(idx, k), self.p(idx, k1));
            if d0 == p0 || d1 == p0 || d0 == p1 || d1 == p1 {
                continue;
            }
            if if loose { intersect_prop(d0, d1, p0, p1) } else { intersect(d0, d1, p0, p1) } {
                return false;
            }
        }
        true
    }
    fn in_cone(&self, i: usize, j: usize, idx: &[usize], loose: bool) -> bool {
        let n = idx.len();
        let (pi, pj) = (self.p(idx, i), self.p(idx, j));
        let pi1 = self.p(idx, (i + 1) % n);
        let pin1 = self.p(idx, (i + n - 1) % n);
        if left_on(pin1, pi, pi1) {
            return if loose { left_on(pi, pj, pin1) && left_on(pj, pi, pi1) } else { left(pi, pj, pin1) && left(pj, pi, pi1) };
        }
        !(left_on(pi, pj, pi1) && left_on(pj, pi, pin1))
    }
    fn diagonal(&self, i: usize, j: usize, idx: &[usize], loose: bool) -> bool {
        self.in_cone(i, j, idx, loose) && self.diagonalie(i, j, idx, loose)
    }
}

const EAR: usize = 0x8000_0000;

/// Twice a contour's signed area in XZ (Recast's calcAreaOfPolygon2D): negative for a hole, an obstacle inside a
/// region that its outline goes round (walked the other way).
fn contour_area(c: &[CVert]) -> i64 {
    let n = c.len();
    (0..n).map(|i| {
        let (vi, vj) = (&c[i], &c[(i + n - 1) % n]);
        vi.x as i64 * vj.z as i64 - vj.x as i64 * vi.z as i64
    })
    .sum()
}

/// Whether segment `d0`-`d1` crosses an edge of `verts` (Recast's intersectSegCountour), skipping the edges at vertex
/// `skip` and edges sharing an end with it.
fn crosses_contour(d0: (i32, i32), d1: (i32, i32), skip: Option<usize>, verts: &[(i32, i32)]) -> bool {
    let n = verts.len();
    (0..n).any(|k| {
        let k1 = (k + 1) % n;
        if skip.is_some_and(|i| i == k || i == k1) {
            return false;
        }
        let (p0, p1) = (verts[k], verts[k1]);
        if d0 == p0 || d1 == p0 || d0 == p1 || d1 == p1 {
            return false;
        }
        intersect(d0, d1, p0, p1)
    })
}

/// Whether `pj` lies in the cone of outline vertex `i` (Recast's inCone, for hole bridges).
fn in_cone_of(i: usize, verts: &[(i32, i32)], pj: (i32, i32)) -> bool {
    let n = verts.len();
    let (pi, pi1, pin1) = (verts[i], verts[(i + 1) % n], verts[(i + n - 1) % n]);
    if left_on(pin1, pi, pi1) {
        return left(pi, pj, pin1) && left(pj, pi, pi1);
    }
    !(left_on(pi, pj, pi1) && left_on(pj, pi, pin1))
}

/// Recast's mergeRegionHoles: each hole joins its region's outline through a bridge (a diagonal from the outline to the
/// hole's leftmost vertex that crosses nothing), so the polygons go round the obstacle instead of over it. Without it a
/// region wrapped round a wall (inside and outside a cabin, joined through the doorway) got polygons across the wall,
/// and the hole itself was garbage. Returns the outlines.
fn merge_holes(walked: Vec<(u32, Vec<CVert>)>) -> Vec<(u32, Vec<CVert>)> {
    let xz = |c: &[CVert]| c.iter().map(|v| (v.x, v.z)).collect::<Vec<_>>();
    let (mut outlines, mut holes): (Vec<(u32, Vec<CVert>)>, Vec<(u32, Vec<CVert>)>) = walked.into_iter().partition(|(_, c)| contour_area(c) >= 0);
    // Leftmost first (min x, then min z), as Recast does.
    let leftmost = |c: &[CVert]| (0..c.len()).min_by_key(|&i| (c[i].x, c[i].z)).unwrap_or(0);
    holes.sort_by_key(|(_, h)| {
        let l = leftmost(h);
        (h[l].x, h[l].z)
    });
    for hi in 0..holes.len() {
        let (region, hole) = (holes[hi].0, holes[hi].1.clone());
        let hole_xz = xz(&hole);
        // The region's outline that holds the hole (point in polygon on its leftmost vertex), else the first one.
        let lm = hole_xz[leftmost(&hole)];
        let contains = |c: &[(i32, i32)]| {
            let n = c.len();
            let mut inside = false;
            for (i, j) in (0..n).zip((0..n).cycle().skip(n - 1)) {
                let (a, b) = (c[i], c[j]);
                if (a.1 > lm.1) != (b.1 > lm.1) {
                    let cross_x = a.0 as f64 + (lm.1 - a.1) as f64 * (b.0 - a.0) as f64 / (b.1 - a.1) as f64;
                    if (lm.0 as f64) < cross_x {
                        inside = !inside;
                    }
                }
            }
            inside
        };
        let mine: Vec<usize> = (0..outlines.len()).filter(|&o| outlines[o].0 == region).collect();
        let Some(&o) = mine.iter().find(|&&o| contains(&xz(&outlines[o].1))).or(mine.first()) else { continue };
        let outline_xz = xz(&outlines[o].1);
        let later: Vec<Vec<(i32, i32)>> = holes[hi..].iter().filter(|(r, _)| *r == region).map(|(_, h)| xz(h)).collect();
        let mut bridge = None;
        let mut best_vertex = leftmost(&hole);
        for _ in 0..hole.len() {
            let corner = hole_xz[best_vertex];
            let mut diags: Vec<(i64, usize)> = (0..outline_xz.len())
                .filter(|&j| in_cone_of(j, &outline_xz, corner))
                .map(|j| {
                    let (dx, dz) = ((outline_xz[j].0 - corner.0) as i64, (outline_xz[j].1 - corner.1) as i64);
                    (dx * dx + dz * dz, j)
                })
                .collect();
            diags.sort();
            bridge = diags.into_iter().map(|(_, j)| j).find(|&j| {
                let pt = outline_xz[j];
                !crosses_contour(pt, corner, Some(j), &outline_xz) && !later.iter().any(|h| crosses_contour(pt, corner, None, h))
            });
            if bridge.is_some() {
                break;
            }
            best_vertex = (best_vertex + 1) % hole.len();
        }
        let Some(ia) = bridge else { continue };
        // Recast's mergeContours: the outline from the bridge round to it again, then the hole likewise.
        let ca = &outlines[o].1;
        let (na, nb) = (ca.len(), hole.len());
        let merged: Vec<CVert> = (0..=na).map(|k| ca[(ia + k) % na]).chain((0..=nb).map(|k| hole[(best_vertex + k) % nb])).collect();
        outlines[o].1 = merged;
    }
    outlines
}

/// Triangles (indices into `v`) of a simple polygon, and whether it had to cut a corner that isn't a proper ear (an
/// outline touching itself); None if it can't be triangulated at all.
fn triangulate(v: &[(i32, i32)]) -> Option<(Vec<[usize; 3]>, bool)> {
    let t = Tri2 { v };
    let mut idx: Vec<usize> = (0..v.len()).collect();
    let mut tris = Vec::new();
    let next = |i: usize, n: usize| (i + 1) % n;
    let prev = |i: usize, n: usize| (i + n - 1) % n;
    let n = idx.len();
    for i in 0..n {
        let (i1, i2) = (next(i, n), next(next(i, n), n));
        if t.diagonal(i, i2, &idx, false) {
            idx[i1] |= EAR;
        }
    }
    let mut stuck = false;
    while idx.len() > 3 {
        let n = idx.len();
        let mut best: Option<(i64, usize)> = None;
        for i in 0..n {
            let i1 = next(i, n);
            if idx[i1] & EAR != 0 {
                let (p0, p2) = (t.p(&idx, i), t.p(&idx, next(i1, n)));
                let len = ((p2.0 - p0.0) as i64).pow(2) + ((p2.1 - p0.1) as i64).pow(2);
                if best.is_none_or(|(l, _)| len < l) {
                    best = Some((len, i));
                }
            }
        }
        if best.is_none() {
            // Degenerate polygon: try loose diagonals.
            for i in 0..n {
                let (i1, i2) = (next(i, n), next(next(i, n), n));
                if t.diagonal(i, i2, &idx, true) {
                    let (p0, p2) = (t.p(&idx, i), t.p(&idx, next(i1, n)));
                    let len = ((p2.0 - p0.0) as i64).pow(2) + ((p2.1 - p0.1) as i64).pow(2);
                    if best.is_none_or(|(l, _)| len < l) {
                        best = Some((len, i));
                    }
                }
            }
        }
        if best.is_none() {
            // Still none (an outline that touches itself): cut the convex corner with the shortest diagonal, so the
            // region keeps its floor rather than losing all of it (Recast gives up here).
            for i in 0..n {
                let (i1, i2) = (next(i, n), next(next(i, n), n));
                let (p0, p1, p2) = (t.p(&idx, i), t.p(&idx, i1), t.p(&idx, i2));
                if left(p0, p1, p2) {
                    let len = ((p2.0 - p0.0) as i64).pow(2) + ((p2.1 - p0.1) as i64).pow(2);
                    if best.is_none_or(|(l, _)| len < l) {
                        best = Some((len, i));
                    }
                }
            }
            stuck = true;
        }
        let (_, i) = best?;
        let i1 = next(i, n);
        let i2 = next(i1, n);
        tris.push([idx[i] & !EAR, idx[i1] & !EAR, idx[i2] & !EAR]);
        idx.remove(i1);
        let n = idx.len();
        let i1 = if i1 >= n { 0 } else { i1 };
        let i = prev(i1, n);
        let ok = t.diagonal(prev(i, n), i1, &idx, false);
        idx[i] = if ok { idx[i] | EAR } else { idx[i] & !EAR };
        let ok = t.diagonal(i, next(i1, n), &idx, false);
        idx[i1] = if ok { idx[i1] | EAR } else { idx[i1] & !EAR };
    }
    tris.push([idx[0] & !EAR, idx[1] & !EAR, idx[2] & !EAR]);
    Some((tris, stuck))
}

/// The merge value of two polygons sharing an edge (its squared length), if the result is convex and small enough.
fn merge_value(pa: &[usize], pb: &[usize], v: &[(i32, i32)], nvp: usize) -> Option<(i64, usize, usize)> {
    let (na, nb) = (pa.len(), pb.len());
    if na + nb - 2 > nvp {
        return None;
    }
    let mut shared = None;
    'outer: for i in 0..na {
        let (mut a0, mut a1) = (pa[i], pa[(i + 1) % na]);
        if a0 > a1 {
            std::mem::swap(&mut a0, &mut a1);
        }
        for j in 0..nb {
            let (mut b0, mut b1) = (pb[j], pb[(j + 1) % nb]);
            if b0 > b1 {
                std::mem::swap(&mut b0, &mut b1);
            }
            if a0 == b0 && a1 == b1 {
                shared = Some((i, j));
                break 'outer;
            }
        }
    }
    let (ea, eb) = shared?;
    let uleft = |a: usize, b: usize, c: usize| area2(v[a], v[b], v[c]) < 0;
    if !uleft(pa[(ea + na - 1) % na], pa[ea], pb[(eb + 2) % nb]) {
        return None;
    }
    if !uleft(pb[(eb + nb - 1) % nb], pb[eb], pa[(ea + 2) % na]) {
        return None;
    }
    let (a, b) = (v[pa[ea]], v[pa[(ea + 1) % na]]);
    let (dx, dz) = ((a.0 - b.0) as i64, (a.1 - b.1) as i64);
    Some((dx * dx + dz * dz, ea, eb))
}

fn merge_polys(pa: &[usize], pb: &[usize], ea: usize, eb: usize) -> Vec<usize> {
    let (na, nb) = (pa.len(), pb.len());
    let mut out = Vec::with_capacity(na + nb - 2);
    for i in 0..na - 1 {
        out.push(pa[(ea + 1 + i) % na]);
    }
    for i in 0..nb - 1 {
        out.push(pb[(eb + 1 + i) % nb]);
    }
    out
}

/// A build's progress, readable while it runs: tiles done of the total, and the time spent per stage (microseconds,
/// summed over tiles): rasterize, filter, compact and erode, regions, contours, polygons.
#[derive(Default, Debug)]
pub struct BuildStats {
    times: [AtomicU64; 6],
    pub tiles_done: AtomicUsize,
    pub tiles_total: AtomicUsize,
    /// Region outlines that didn't triangulate cleanly (cut at corners that aren't proper ears; Recast drops them).
    pub bad_contours: AtomicUsize,
}

impl BuildStats {
    fn add(&self, stage: usize, since: &mut std::time::Instant) {
        let now = std::time::Instant::now();
        self.times[stage].fetch_add((now - *since).as_micros() as u64, Ordering::Relaxed);
        *since = now;
    }

    /// Seconds per stage.
    pub fn seconds(&self) -> [f32; 6] {
        std::array::from_fn(|i| self.times[i].load(Ordering::Relaxed) as f32 / 1e6)
    }

    /// Tiles done and in all.
    pub fn progress(&self) -> (usize, usize) {
        (self.tiles_done.load(Ordering::Relaxed), self.tiles_total.load(Ordering::Relaxed))
    }
}

/// The voxel column at `p` (diagnostics): its solid spans bottom up, (bottom, top, walkable by slope) in world
/// units.
pub fn probe_column(world: &(impl TriSource + ?Sized), cfg: &NavConfig, p: Vec3, ymin: f32, ymax: f32) -> Vec<(f32, f32, bool)> {
    let cs = cfg.cell_size;
    let bmin = Vec3::new((p.x / cs).floor() * cs, ymin, (p.z / cs).floor() * cs);
    let mut hf = Heightfield { w: 1, h: 1, bmin, cs, ch: cfg.cell_height, cols: vec![Vec::new()] };
    let walkable_y = cfg.agent_max_slope_deg.to_radians().cos();
    world.for_each_in(bmin - Vec3::Y, Vec3::new(bmin.x + cs, ymax, bmin.z + cs), &mut |t| {
        rasterize_tri(&mut hf, t, if t.n.y > walkable_y { WALKABLE } else { NULL_AREA }, cfg.walkable_climb());
    });
    hf.cols[0].iter().map(|s| (ymin + s.smin as f32 * cfg.cell_height, ymin + s.smax as f32 * cfg.cell_height, s.area != NULL_AREA)).collect()
}

/// Builds one tile from the triangles `world` has around it. `tx`, `tz` index tiles from `origin`.
pub fn build_tile(world: &(impl TriSource + ?Sized), cfg: &NavConfig, origin: Vec3, tx: i32, tz: i32, ymin: f32, ymax: f32, stats: &BuildStats) -> TileMesh {
    let mut clock = std::time::Instant::now();
    let border = cfg.border();
    let size = cfg.tile_cells + 2 * border;
    let cs = cfg.cell_size;
    let bmin = Vec3::new(origin.x + (tx * cfg.tile_cells - border) as f32 * cs, ymin, origin.z + (tz * cfg.tile_cells - border) as f32 * cs);
    let bmax = Vec3::new(bmin.x + size as f32 * cs, ymax, bmin.z + size as f32 * cs);
    let mut hf = Heightfield { w: size, h: size, bmin, cs, ch: cfg.cell_height, cols: vec![Vec::new(); (size * size) as usize] };
    let (height, climb, radius) = (cfg.walkable_height(), cfg.walkable_climb(), cfg.walkable_radius());
    let walkable_y = cfg.agent_max_slope_deg.to_radians().cos();
    world.for_each_in(bmin, bmax, &mut |t| {
        let area = if t.n.y > walkable_y { WALKABLE } else { NULL_AREA };
        rasterize_tri(&mut hf, t, area, climb);
    });
    stats.add(0, &mut clock);
    filter_low_hanging_obstacles(&mut hf, climb);
    filter_ledges(&mut hf, height, climb);
    filter_low_ceilings(&mut hf, height);
    stats.add(1, &mut clock);
    let mut c = Compact::build(&hf, height, climb);
    c.erode(radius);
    stats.add(2, &mut clock);
    let mut reg = c.regions_monotone(border, cfg.region_chunk_splits);
    c.merge_regions(&mut reg, cfg.merge_region_area);
    stats.add(3, &mut clock);

    // Contours: a bit per span edge that borders another region.
    let n = c.span_count();
    let mut flags = vec![0u8; n];
    for i in 0..n {
        if reg[i] == 0 || reg[i] & BORDER_REG != 0 {
            continue;
        }
        let mut same = 0u8;
        for d in 0..4 {
            if c.nei(i, d).is_some_and(|a| reg[a] == reg[i]) {
                same |= 1 << d;
            }
        }
        flags[i] = same ^ 0xf;
    }
    let mut walked: Vec<(u32, Vec<CVert>)> = Vec::new();
    for z in 0..size {
        for x in 0..size {
            let (f, cnt) = c.cells[(x + z * size) as usize];
            for i in f as usize..(f + cnt) as usize {
                if flags[i] == 0 || flags[i] == 0xf {
                    flags[i] = 0;
                    continue;
                }
                let raw = walk_contour(&c, &reg, x, z, i, &mut flags);
                let simple = simplify_contour(&raw, cfg.max_simplification_error, cfg.max_edge_len);
                if simple.len() >= 3 {
                    walked.push((reg[i], simple));
                }
            }
        }
    }
    let contours = merge_holes(walked);

    stats.add(4, &mut clock);
    // Polygons: triangulate each contour, merge into convex polygons, share vertices.
    let mut mesh = TileMesh { tx, tz, ..Default::default() };
    let mut vmap: HashMap<(i32, i32), Vec<(i32, u32)>> = HashMap::new();
    let mut add_vertex = |mesh: &mut TileMesh, v: &CVert| -> u32 {
        let list = vmap.entry((v.x, v.z)).or_default();
        if let Some(&(_, idx)) = list.iter().find(|(y, _)| (y - v.y).abs() <= 2) {
            return idx;
        }
        let idx = mesh.verts.len() as u32;
        mesh.verts.push(Vec3::new(bmin.x + v.x as f32 * cs, bmin.y + v.y as f32 * cfg.cell_height, bmin.z + v.z as f32 * cs));
        list.push((v.y, idx));
        idx
    };
    for (region, cont) in &contours {
        let xz: Vec<(i32, i32)> = cont.iter().map(|v| (v.x, v.z)).collect();
        let Some((tris, stuck)) = triangulate(&xz) else {
            stats.bad_contours.fetch_add(1, Ordering::Relaxed);
            continue;
        };
        if stuck {
            stats.bad_contours.fetch_add(1, Ordering::Relaxed);
        }
        let mut polys: Vec<Vec<usize>> = tris.iter().filter(|t| area2(xz[t[0]], xz[t[1]], xz[t[2]]) != 0).map(|t| t.to_vec()).collect();
        loop {
            let mut best: Option<(i64, usize, usize, usize, usize)> = None;
            for j in 0..polys.len() {
                for k in j + 1..polys.len() {
                    if let Some((v, ea, eb)) = merge_value(&polys[j], &polys[k], &xz, cfg.max_verts_per_poly) {
                        if best.is_none_or(|b| v > b.0) {
                            best = Some((v, j, k, ea, eb));
                        }
                    }
                }
            }
            let Some((_, j, k, ea, eb)) = best else { break };
            let merged = merge_polys(&polys[j], &polys[k], ea, eb);
            polys[j] = merged;
            polys.swap_remove(k);
        }
        for p in polys {
            let ids: Vec<u32> = p.iter().map(|&k| add_vertex(&mut mesh, &cont[k])).collect();
            let mut dedup = ids.clone();
            dedup.dedup();
            if dedup.len() >= 3 && dedup.first() != dedup.last() {
                // Its detail mesh, from its corners as the mesh keeps them (cells).
                let corners: Vec<(i32, i32, i32)> = dedup
                    .iter()
                    .map(|&k| {
                        let v = (mesh.verts[k as usize] - bmin) / Vec3::new(cs, cfg.cell_height, cs);
                        (v.x.round() as i32, v.y.round() as i32, v.z.round() as i32)
                    })
                    .collect();
                let (extra, tris) = if cfg.detail_sample_dist > 0.0 {
                    detail::poly_detail(&c, &reg, &corners, *region, cs, cfg.cell_height, cfg.detail_sample_dist, cfg.detail_sample_max_error)
                } else {
                    (Vec::new(), Vec::new())
                };
                let to_world = |v: Vec3| Vec3::new(bmin.x + v.x * cs, bmin.y + v.y * cfg.cell_height, bmin.z + v.z * cs);
                mesh.detail.push((extra.into_iter().map(to_world).collect(), tris));
                mesh.polys.push(dedup);
            }
        }
    }
    stats.add(5, &mut clock);
    mesh
}

/// Builds every tile over `min..max` (XZ) in parallel. `origin` anchors the tile grid.
pub fn build(world: &(impl TriSource + Sync + ?Sized), cfg: &NavConfig, min: Vec3, max: Vec3, stats: &BuildStats) -> Vec<TileMesh> {
    let ts = cfg.tile_size();
    let origin = Vec3::new(min.x, 0.0, min.z);
    let (nx, nz) = (((max.x - min.x) / ts).ceil() as i32, ((max.z - min.z) / ts).ceil() as i32);
    let jobs: Vec<(i32, i32)> = (0..nz).flat_map(|z| (0..nx).map(move |x| (x, z))).collect();
    let threads = std::thread::available_parallelism().map_or(4, |n| n.get());
    let next = AtomicUsize::new(0);
    stats.tiles_total.store(jobs.len(), Ordering::Relaxed);
    let mut out: Vec<TileMesh> = Vec::new();
    std::thread::scope(|s| {
        let handles: Vec<_> = (0..threads)
            .map(|_| {
                s.spawn(|| {
                    let mut mine = Vec::new();
                    loop {
                        let k = next.fetch_add(1, Ordering::Relaxed);
                        let Some(&(x, z)) = jobs.get(k) else { break };
                        let t = build_tile(world, cfg, origin, x, z, min.y - 1.0, max.y + 1.0, stats);
                        stats.tiles_done.fetch_add(1, Ordering::Relaxed);
                        if !t.polys.is_empty() {
                            mine.push(t);
                        }
                    }
                    mine
                })
            })
            .collect();
        for h in handles {
            out.extend(h.join().unwrap_or_default());
        }
    });
    out.sort_by_key(|t| (t.tz, t.tx));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A raw outline: walls (region 0) on three sides of a 20 x 20 cell square, and on the fourth a border with
    /// region 5 that winds down to (10, 10) and back (as one round a wall end through a doorway does).
    fn winding() -> Vec<CVert> {
        let mut pts = Vec::new();
        let mut push = |x: i32, z: i32, r: u32| pts.push(CVert { x, y: 0, z, r });
        for x in 0..20 {
            push(x, 0, 0);
        }
        for z in 0..20 {
            push(20, z, 0);
        }
        // The border with region 5: (20, 20) down to (10, 10) and up to (0, 20).
        for k in 0..10 {
            push(20 - k, 20 - k, 5);
        }
        for k in 0..10 {
            push(10 - k, 10 + k, 5);
        }
        for z in (1..=20).rev() {
            push(0, z, 0);
        }
        pts
    }

    #[test]
    fn a_winding_border_between_regions_keeps_its_shape() {
        let simple = simplify_contour(&winding(), 1.3, 0);
        // Every raw point stays within the error of the simplified outline: the dip to (10, 10) is kept, not cut
        // straight across.
        let near = |x: i32, z: i32| {
            (0..simple.len()).any(|i| {
                let (a, b) = (&simple[i], &simple[(i + 1) % simple.len()]);
                dist_pt_seg_sq(x, z, a.x, a.z, b.x, b.z) <= 1.3 * 1.3 + 1e-3
            })
        };
        for p in winding() {
            assert!(near(p.x, p.z), "({}, {}) is off the simplified outline {:?}", p.x, p.z, simple.iter().map(|v| (v.x, v.z)).collect::<Vec<_>>());
        }
        assert!(simple.iter().any(|v| (v.x, v.z) == (10, 10)), "the dip");
        // Walked the other way (as the region on the far side walks it), the border keeps the same dip.
        let mut pts = winding();
        pts.reverse();
        assert!(simplify_contour(&pts, 1.3, 0).iter().any(|v| (v.x, v.z) == (10, 10)), "the dip, walked the other way");
    }
}
