//! Enlighten 3's precomputed data, as the game's runtime reads it (decompiled: F13-decomp/MODLOG "Enlighten GI"; the
//! export: F13-decomp/enlighten/extract.py writes each system's blocks to `export/enlighten/`).
//!
//! - [`Transport`]: a system's radiosity precompute (GESG, RadSystemCore's m_RadiosityPrecomp; the solve
//!   SolveIrradianceTask's plain kernel 0x141db3980): its output pixels in blocks, each block gathering clusters of
//!   light from its input list (the system's own input workspace and its neighbours', and the environment) through u8
//!   coefficients per pixel and cluster.
//! - [`InputWorkspace`]: a system's input samples (GEIW and its cluster table GEDG; their readers 0x141d88d20,
//!   0x141d890b0): points on its surfaces in clusters, the light falling on them averaged per cluster being the
//!   system's input lighting (DoDirectInputLighting 0x141db7010).

use glam::{Vec3, Vec4};

fn u16_at(b: &[u8], o: usize) -> Option<u16> {
    Some(u16::from_le_bytes(b.get(o..o + 2)?.try_into().ok()?))
}
fn u32_at(b: &[u8], o: usize) -> Option<u32> {
    Some(u32::from_le_bytes(b.get(o..o + 4)?.try_into().ok()?))
}
fn i32_at(b: &[u8], o: usize) -> Option<i32> {
    Some(i32::from_le_bytes(b.get(o..o + 4)?.try_into().ok()?))
}
fn f32_at(b: &[u8], o: usize) -> Option<f32> {
    Some(f32::from_le_bytes(b.get(o..o + 4)?.try_into().ok()?))
}

/// The gather's scale for byte coefficients (the plain kernel 0x141db3980): a cluster's u16 weight over 65535 and a
/// pixel's u8 coefficient over 255 together.
pub const GATHER_SCALE: f32 = 1.0 / (65535.0 * 255.0);
/// For packed coefficients (the kernel 0x141dabbc0, a block's shift 1): the u16 weight over 65535 and the squared
/// nibble (0..225) over 225 (DAT_1429c08e0).
pub const PACKED_GATHER_SCALE: f32 = 1.0 / (65535.0 * 225.0);

/// One cluster a block gathers: its entry in the input list, its index in that input's lighting, its weight (with
/// [`GATHER_SCALE`] in).
#[derive(Clone, Copy, Debug)]
pub struct Gathered {
    pub input: u16,
    pub index: u32,
    pub weight: f32,
}

/// A block of output pixels.
#[derive(Clone, Debug, Default)]
pub struct Block {
    pub clusters: Vec<Gathered>,
    /// Its pixels (x, y in the system's output).
    pub pixels: Vec<(u16, u16)>,
    /// Per pixel, per cluster: its coefficient (pixels x clusters; already squared for packed blocks, whose weights
    /// carry [`PACKED_GATHER_SCALE`]).
    pub coefficients: Vec<u8>,
    /// Pixels copied from another: (destination, source).
    pub duplicates: Vec<((u16, u16), (u16, u16))>,
}

/// A system's transport (GESG).
#[derive(Clone, Debug, Default)]
pub struct Transport {
    pub width: u16,
    pub height: u16,
    /// The input list: each entry's Geo GUID as four u32s (a system's: its UE SystemGuid's first u32 at [1]; the
    /// environment's: [0, 0, 0xfffffffe, 0]; all ones: none, gathering nothing).
    pub inputs: Vec<[u32; 4]>,
    pub blocks: Vec<Block>,
}

impl Transport {
    /// Reads a GESG block (from its magic). None if it isn't one or doesn't read.
    pub fn read(g: &[u8]) -> Option<Self> {
        if g.get(0..4)? != b"GESG" || u32_at(g, 4)? != 37 {
            return None;
        }
        let blocks = i32_at(g, 8)? as usize;
        let list = i32_at(g, 0x0c)? as usize;
        let (width, height) = (u16_at(g, 0x10)?, u16_at(g, 0x12)?);
        let at = 32 * (blocks + 1);
        let mut inputs = Vec::with_capacity(list);
        for i in 0..list {
            let o = at + 16 * i;
            inputs.push([u32_at(g, o)?, u32_at(g, o + 4)?, u32_at(g, o + 8)?, u32_at(g, o + 12)?]);
        }
        let mut out = Vec::with_capacity(blocks);
        for b in 0..blocks {
            let h = 32 + 32 * b;
            let pixels = u16_at(g, h)? as usize;
            let q = u16_at(g, h + 2)? as usize;
            let dups = u16_at(g, h + 4)? as usize;
            let clusters = u16_at(g, h + 8)? as usize;
            let shift = *g.get(h + 0x0a)? as u32;
            let extra = u16_at(g, h + 0x0c)? as usize;
            let off = i32_at(g, h + 0x10)?;
            if shift > 1 {
                return None;
            }
            // The kernels' layout: D = the header's +8 + off; the u16 weights from D - 8, the u32 indices, the
            // per-input counts (`extra` = the input list's length), the directional data (q x C bytes), the
            // coefficients, the pixels.
            let d = (h as i64 + 8 + off as i64) as usize;
            let weights = d - 8;
            let indices = (d + 2 * clusters - 5) & !3;
            let counts = indices + 4 * clusters;
            let r8 = (indices + 4 * (clusters + extra) + 15) & !15;
            let scale = if shift == 1 { PACKED_GATHER_SCALE } else { GATHER_SCALE };
            let coef = r8 + q * clusters;
            let coord = (coef + ((pixels * clusters) >> shift) + 3) & !3;
            let mut gathered = Vec::with_capacity(clusters);
            let mut c = 0;
            for e in 0..inputs.len() {
                let n = u16_at(g, counts + 2 * e)? as usize;
                for _ in 0..n {
                    if c >= clusters {
                        return None;
                    }
                    gathered.push(Gathered { input: e as u16, index: u32_at(g, indices + 4 * c)?, weight: u16_at(g, weights + 2 * c)? as f32 * scale });
                    c += 1;
                }
            }
            if c != clusters {
                return None;
            }
            let coefficients = if shift == 0 {
                g.get(coef..coef + pixels * clusters)?.to_vec()
            } else {
                // 4 bits each, squared: per pixel ceil(C/16) chunks of 8 bytes, chunk i's byte k holding cluster 16i + k
                // (low nibble) and 16i + 8 + k (high); a pixel's own bytes stop short of its last chunk's spare half.
                let per = clusters.div_ceil(16) * 8 - ((clusters.wrapping_neg() & 15) >> 1);
                let mut out = Vec::with_capacity(pixels * clusters);
                for p in 0..pixels {
                    for c in 0..clusters {
                        let byte = *g.get(coef + p * per + 8 * (c / 16) + (c % 16) % 8)?;
                        let n = if c % 16 < 8 { byte & 15 } else { byte >> 4 };
                        out.push(n * n);
                    }
                }
                out
            };
            let pixels: Option<Vec<_>> = (0..pixels).map(|p| Some((u16_at(g, coord + 2 * p)?, u16_at(g, coord + 2 * pixels + 2 * p)?))).collect();
            let pixels = pixels?;
            let d = coord + 4 * pixels.len();
            let duplicates: Option<Vec<_>> = (0..dups)
                .map(|i| Some(((u16_at(g, d + 2 * i)?, u16_at(g, d + 2 * dups + 2 * i)?), (u16_at(g, d + 4 * dups + 2 * i)?, u16_at(g, d + 6 * dups + 2 * i)?))))
                .collect();
            out.push(Block { clusters: gathered, pixels, coefficients, duplicates: duplicates? });
        }
        Some(Transport { width, height, inputs, blocks: out })
    }

    /// The output pixels' irradiance (width x height, row-major; pixels the transport doesn't cover stay zero) from
    /// each input entry's cluster lighting (None: an input not present, gathering zero), times `scale` (the task's
    /// scale, SolveIrradianceTask's params +0x2c).
    pub fn solve(&self, inputs: &[Option<&[Vec4]>], scale: f32) -> Vec<Vec4> {
        let w = self.width as usize;
        let mut out = vec![Vec4::ZERO; w * self.height as usize];
        let mut g = Vec::new();
        for b in &self.blocks {
            g.clear();
            g.extend(b.clusters.iter().map(|c| inputs.get(c.input as usize).copied().flatten().and_then(|l| l.get(c.index as usize)).map_or(Vec4::ZERO, |v| *v * c.weight)));
            let n = b.clusters.len();
            for (p, &(x, y)) in b.pixels.iter().enumerate() {
                let row = &b.coefficients[p * n..(p + 1) * n];
                let e = row.iter().zip(&g).fold(Vec4::ZERO, |a, (&k, v)| a + *v * k as f32);
                out[y as usize * w + x as usize] = e.max(Vec4::ZERO) * scale;
            }
            for &((dx, dy), (sx, sy)) in &b.duplicates {
                out[dy as usize * w + dx as usize] = out[sy as usize * w + sx as usize];
            }
        }
        out
    }
}

/// A packet of four points sharing a normal.
#[derive(Clone, Debug)]
pub struct Packet {
    /// Unreal space (cm).
    pub points: [Vec3; 4],
    pub normal: Vec3,
}

/// A system's input samples (GEIW + GEDG).
#[derive(Clone, Debug, Default)]
pub struct InputWorkspace {
    /// Each cluster's packets.
    pub clusters: Vec<Vec<Packet>>,
}

impl InputWorkspace {
    /// Reads a GEIW block from its 32-byte prefix (its magic at +0x20), its GEDG following.
    pub fn read(b: &[u8]) -> Option<Self> {
        if b.get(0x20..0x24)? != b"GEIW" {
            return None;
        }
        let points = i32_at(b, 0x24)? as usize;
        let clusters = i32_at(b, 0x28)? as usize;
        let (o_pts, o_box, o_cnt) = (i32_at(b, 0x30)? as usize, i32_at(b, 0x34)? as usize, i32_at(b, 0x40)? as usize);
        let mut out = Vec::with_capacity(clusters);
        let mut packet = 0;
        for c in 0..clusters {
            let o = o_box + 32 * c;
            let mn = Vec3::new(f32_at(b, o)?, f32_at(b, o + 4)?, f32_at(b, o + 8)?);
            let mx = Vec3::new(f32_at(b, o + 16)?, f32_at(b, o + 20)?, f32_at(b, o + 24)?);
            let n = *b.get(o_cnt + 0x10 + c)? as usize;
            let mut packets = Vec::with_capacity(n);
            for _ in 0..n {
                let p = o_pts + 16 * packet;
                let q = b.get(p..p + 16)?;
                let normal = Vec3::new(q[3] as f32, q[7] as f32, q[11] as f32) * (2.0 / 255.0) - Vec3::ONE;
                let pt = |j: usize| mn + (mx - mn) * Vec3::new(q[4 * j] as f32, q[4 * j + 1] as f32, q[4 * j + 2] as f32) / 255.0;
                packets.push(Packet { points: [pt(0), pt(1), pt(2), pt(3)], normal });
                packet += 1;
            }
            out.push(packets);
        }
        (packet * 4 == points).then_some(InputWorkspace { clusters: out })
    }
}

/// A system's resampling of its output onto its clusters (GERD, ResamplingData; DoResampleBounce 0x141db1950): per
/// cluster the output pixels (x, y) it averages and their weights (summing to about 1). The bounce: each cluster's
/// light from the last solve.
#[derive(Clone, Debug, Default)]
pub struct Resampling {
    pub clusters: Vec<Vec<(u16, u16, f32)>>,
}

impl Resampling {
    /// Reads a GERD block (from its magic): "GERD", version 2, u32 cluster count, u32, u32 sample count, then from
    /// +0x20 a u16 per cluster (its samples), then (8-aligned) the samples: u16 x, u16 y, f32 weight.
    pub fn read(b: &[u8]) -> Option<Self> {
        if b.get(0..4)? != b"GERD" || u32_at(b, 4)? != 2 {
            return None;
        }
        let clusters = u32_at(b, 8)? as usize;
        let samples = u32_at(b, 16)? as usize;
        let mut p = (0x20 + 2 * clusters + 7) & !7;
        let mut out = Vec::with_capacity(clusters);
        let mut total = 0;
        for c in 0..clusters {
            let n = u16_at(b, 0x20 + 2 * c)? as usize;
            total += n;
            let mut v = Vec::with_capacity(n);
            for _ in 0..n {
                v.push((u16_at(b, p)?, u16_at(b, p + 2)?, f32_at(b, p + 4)?));
                p += 8;
            }
            out.push(v);
        }
        (total == samples).then_some(Resampling { clusters: out })
    }

    /// Each cluster's bounce from the output (width `w`, row-major).
    pub fn bounce(&self, output: &[Vec4], w: usize) -> Vec<Vec4> {
        self.clusters.iter().map(|s| s.iter().map(|&(x, y, k)| output.get(y as usize * w + x as usize).copied().unwrap_or(Vec4::ZERO) * k).sum()).collect()
    }
}

/// A system's precomputed visibility of directional lights (GEVS v5, PrecomputedVisibilityData; SetDirectionalVisibility
/// 0x141d8ab50 -> 0x141db2090, the direction's lookup 0x141db1700 via 0x141db15b0, the slices' decoding 0x141db0a50):
/// per direction on a sphere of rings (ring 0 the pole), a slice holding each cluster's visibility in 1, 2, 4 or 8 bits.
/// The header: +0x1c the entries per byte, +0x24 the blocks' offset table, +0x28 the data's size, +0x2c the data's
/// offset, +0x30 the block count, +0x34 slices per block, +0x38 the ring count, +0x40 u16 each ring's first slice,
/// +0x140 u16 each ring's slice count. A block's slices are run-length coded: a byte c < 0x80 then c literal bytes, or
/// c >= 0x80 then one byte repeated c - 0x80 times.
#[derive(Clone, Debug, Default)]
pub struct Visibility {
    per_byte: u32,
    rings: Vec<(u16, u16)>,
    slices: Vec<Vec<u8>>,
}

impl Visibility {
    pub fn read(g: &[u8]) -> Option<Self> {
        if g.get(0..4)? != b"GEVS" || u32_at(g, 4)? != 5 {
            return None;
        }
        let per_byte = u32_at(g, 0x1c)?;
        if ![1, 2, 4, 8].contains(&per_byte) {
            return None;
        }
        let (table, size, data, blocks, per_block) = (u32_at(g, 0x24)? as usize, u32_at(g, 0x28)? as usize, u32_at(g, 0x2c)? as usize, u32_at(g, 0x30)? as usize, u32_at(g, 0x34)? as usize);
        let n = u32_at(g, 0x38)? as usize;
        if n == 0 || n > 128 || per_block == 0 {
            return None;
        }
        let rings: Option<Vec<_>> = (0..n).map(|r| Some((u16_at(g, 0x40 + 2 * r)?, u16_at(g, 0x140 + 2 * r)?))).collect();
        let mut slices = Vec::with_capacity(blocks * per_block);
        let mut slice_bytes = None;
        for b in 0..blocks {
            let at = u32_at(g, table + 4 * b)? as usize;
            let end = if b + 1 == blocks { size } else { u32_at(g, table + 4 * (b + 1))? as usize };
            let src = g.get(data + at..data + end)?;
            let mut out = Vec::new();
            let mut i = 0;
            while i < src.len() {
                let c = src[i] as usize;
                if c < 0x80 {
                    out.extend_from_slice(src.get(i + 1..i + 1 + c)?);
                    i += 1 + c;
                } else {
                    out.extend(std::iter::repeat_n(*src.get(i + 1)?, c - 0x80));
                    i += 2;
                }
            }
            // (A slice's bytes: the first block's over its slices; the last block may hold fewer slices.)
            // (A system with no clusters: empty slices.)
            let each = *slice_bytes.get_or_insert(out.len() / per_block);
            if each == 0 {
                slices.extend(std::iter::repeat_n(Vec::new(), per_block));
                continue;
            }
            if out.len() % each != 0 {
                return None;
            }
            slices.extend(out.chunks_exact(each).map(<[u8]>::to_vec));
        }
        Some(Visibility { per_byte, rings: rings?, slices })
    }

    /// Each cluster's visibility (0..1) of a directional light towards `to_light` (Unreal space): the four slices
    /// round the direction (two in each of the two nearest rings) blended bilinearly, the weights in bytes as the
    /// game's (the first 255 less the others).
    pub fn directional(&self, to_light: Vec3, clusters: usize) -> Vec<f32> {
        let d = to_light.normalize_or_zero();
        let n = self.rings.len() as i32;
        let e = d.y.clamp(-1.0, 1.0).acos() * std::f32::consts::FRAC_1_PI * (n as f32 - 1.0);
        let r0 = (e as i32).min(n - 1).max(0);
        let r1 = (e as i32 + 1).min(n - 1).max(0);
        let fe = e - r0 as f32;
        let a = d.z.atan2(-d.x) * (0.5 * std::f32::consts::FRAC_1_PI) + 0.5;
        let around = |r: i32| {
            let (start, count) = self.rings[r as usize];
            let s = count as f32 * a;
            let j = (s as i32).min(count as i32 - 1).max(0);
            let k = if j + 1 < count as i32 { j + 1 } else { j + 1 - count as i32 };
            (start as usize + j as usize, start as usize + k as usize, s - j as f32)
        };
        let (i0, i1, f0) = around(r0);
        let (i2, i3, f1) = around(r1);
        let w = [(1.0 - f0) * (1.0 - fe), (1.0 - fe) * f0, (1.0 - f1) * fe, f1 * fe].map(|w| (w * 255.0) as i32 as u8);
        let w = [255u8.wrapping_sub(w[3]).wrapping_sub(w[2]).wrapping_sub(w[1]), w[1], w[2], w[3]];
        let bits = 8 / self.per_byte;
        let mask = (1u32 << bits) - 1;
        let shift = self.per_byte.trailing_zeros();
        let slices = [i0, i1, i2, i3].map(|i| self.slices.get(i));
        (0..clusters)
            .map(|c| {
                let byte = c >> shift;
                let bit = ((c as u32) & (self.per_byte - 1)) * bits;
                let v: u32 = slices.iter().zip(w).map(|(s, w)| s.and_then(|s| s.get(byte)).map_or(0, |&b| (b as u32 >> bit) & mask) * w as u32).sum();
                v as f32 / (mask * 255) as f32
            })
            .collect()
    }
}

impl InputWorkspace {
    /// Each cluster's light from a directional light shining along `dir` (Unreal space, normalised) with `color`, as
    /// DoDirectInputLighting's type 2 kernel (0x141dd3f10): per packet the mean of max(0, N . -dir) over its four
    /// points, the cluster the mean over its packets (rcp of their count). `seen`: each cluster's visibility of the
    /// light ([`Visibility::directional`]; None: every point seen), as SetDirectionalVisibility's bits: of a cluster's
    /// 4 x P points the first floor(seen x 4P), in packet order, see it.
    pub fn directional(&self, dir: Vec3, color: Vec4, seen: Option<&[f32]>) -> Vec<Vec4> {
        let to_light = -dir;
        self.clusters
            .iter()
            .enumerate()
            .map(|(c, packets)| {
                if packets.is_empty() {
                    return Vec4::ZERO;
                }
                let mut points = seen.map_or(usize::MAX, |s| (s.get(c).copied().unwrap_or(1.0) * 4.0 * packets.len() as f32) as usize);
                let sum: f32 = packets
                    .iter()
                    .map(|p| {
                        let k = points.min(4);
                        points -= k;
                        p.normal.dot(to_light).max(0.0) * k as f32 / 4.0
                    })
                    .sum();
                color * (sum / packets.len() as f32)
            })
            .collect()
    }
}

/// A probe set's transport (GESH v40, RadProbeSetCore's m_EntireProbeSetPrecomp; SolveEntireProbeSetTask's plain
/// solver 0x141d8bde0): blocks of probes gathering clusters of the input list (laid out as [`Transport`]'s), per probe
/// and cluster a squared nibble (its coefficient, with [`PACKED_GATHER_SCALE`] in the cluster's weight) and a
/// direction byte (into [`PROBE_DIRECTIONS`]); per probe its output index. A block's header (16 bytes from +0x10): u16
/// probes P, u16 clusters C, u16 the list's length L, two u8 LODs, i32 its data's offset from H + 2. Its data: u16
/// weights from H + off, u32 indices from (H + 2 + off + 1 + 2C) & !3, u16 per-input counts, then from that + 4(L + C)
/// + 0xe the nibbles ((C + 1) / 2 bytes a probe, cluster c in byte c / 2's low (even c) or high nibble), the direction
/// bytes (C a probe) and, 4-aligned, the i32 output indices.
#[derive(Clone, Debug, Default)]
pub struct ProbeTransport {
    pub inputs: Vec<[u32; 4]>,
    pub blocks: Vec<ProbeBlock>,
}

#[derive(Clone, Debug, Default)]
pub struct ProbeBlock {
    pub clusters: Vec<Gathered>,
    /// Per probe: its output index, then per cluster (coefficient n^2, direction index).
    pub probes: Vec<(u32, Vec<(u8, u8)>)>,
}

/// A probe's L1 light (rgb): its mean radiance and its directional moment (x, y, z in Unreal space), each over the
/// probe's whole sphere (the palette entry (1, x, y, z)): lit by a sky of radiance 1 an open probe gathers (1, 0, 0, 0).
pub type ProbeSh = [Vec3; 4];

impl ProbeTransport {
    pub fn read(g: &[u8]) -> Option<Self> {
        if g.get(0..4)? != b"GESH" || u32_at(g, 4)? != 0x28 {
            return None;
        }
        let blocks = i32_at(g, 8)? as usize;
        let list = i32_at(g, 0x0c)? as usize;
        let at = 16 * (blocks + 1);
        let inputs: Option<Vec<_>> = (0..list).map(|i| Some([u32_at(g, at + 16 * i)?, u32_at(g, at + 16 * i + 4)?, u32_at(g, at + 16 * i + 8)?, u32_at(g, at + 16 * i + 12)?])).collect();
        let mut out = Vec::with_capacity(blocks);
        for b in 0..blocks {
            let h = 0x10 + 16 * b;
            let (p, c, l) = (u16_at(g, h)? as usize, u16_at(g, h + 2)? as usize, u16_at(g, h + 4)? as usize);
            let off = i32_at(g, h + 8)? as usize;
            let weights = h + off;
            let indices = (h + 2 + off + 1 + 2 * c) & !3;
            let counts = indices + 4 * c;
            let nib = indices + 4 * (l + c) + 0xe;
            let dirs = nib + c.div_ceil(2) * p;
            let outs = (dirs + 3 + p * c) & !3;
            let mut clusters = Vec::with_capacity(c);
            let mut k = 0;
            for e in 0..l {
                for _ in 0..u16_at(g, counts + 2 * e)? {
                    clusters.push(Gathered { input: e as u16, index: u32_at(g, indices + 4 * k)?, weight: u16_at(g, weights + 2 * k)? as f32 * PACKED_GATHER_SCALE });
                    k += 1;
                }
            }
            if k != c {
                return None;
            }
            let mut probes = Vec::with_capacity(p);
            for q in 0..p {
                let row: Option<Vec<_>> = (0..c)
                    .map(|j| {
                        let byte = *g.get(nib + q * c.div_ceil(2) + j / 2)?;
                        let n = if j % 2 == 0 { byte & 15 } else { byte >> 4 };
                        Some((n * n, *g.get(dirs + q * c + j)?))
                    })
                    .collect();
                probes.push((u32_at(g, outs + 4 * q)?, row?));
            }
            out.push(ProbeBlock { clusters, probes });
        }
        Some(ProbeTransport { inputs: inputs?, blocks: out })
    }

    /// Each probe's light (`count` outputs) from the input entries' cluster lighting (as [`Transport::solve`]'s).
    pub fn solve(&self, inputs: &[Option<&[Vec4]>], count: usize) -> Vec<ProbeSh> {
        let mut out = vec![[Vec3::ZERO; 4]; count];
        let mut g = Vec::new();
        for b in &self.blocks {
            g.clear();
            g.extend(b.clusters.iter().map(|c| inputs.get(c.input as usize).copied().flatten().and_then(|l| l.get(c.index as usize)).map_or(Vec3::ZERO, |v| v.truncate() * c.weight)));
            for (o, row) in &b.probes {
                let mut sh = [Vec3::ZERO; 4];
                for ((k, d), l) in row.iter().zip(&g) {
                    let dir = PROBE_DIRECTIONS.get(*d as usize).copied().unwrap_or([0.0; 3]);
                    let v = *l * *k as f32;
                    sh[0] += v;
                    sh[1] += v * dir[0];
                    sh[2] += v * dir[1];
                    sh[3] += v * dir[2];
                }
                if let Some(slot) = out.get_mut(*o as usize) {
                    *slot = sh;
                }
            }
        }
        out
    }
}

/// A probe set's octree (GEPO v40, RadProbeSetCore's m_InterpolationData; ExtractOctreeProbePositions 0x141d879a0,
/// its walk 0x141d876c0): nodes of 0x80 bytes from the root (+0x4c + 4 x ((u16 +10) x (i32 +0x2c + i32 +0xc) + 3 & !3)),
/// each 27 u32 probe indices on its 3 x 3 x 3 grid (x + (z x 3 + y) x 3) and at +0x70 8 u16 children (an offset in
/// nodes from this one; 0xffff none); u16 +8 the node count (0: the root alone, its corners only). Past the nodes a
/// pair of i32 per virtual probe (i32 +0x30: LOD 0's count; 0x141d822c0): its value the pair's mean, made in order
/// after the real probes'. The octree's origin and size are the core's (+0x50, +0x60).
#[derive(Clone, Debug, Default)]
pub struct ProbeOctree {
    pub origin: Vec3,
    pub size: f32,
    pub probes: usize,
    nodes: Vec<([u32; 27], [u16; 8])>,
    flat: bool,
    virtuals: Vec<(u32, u32)>,
}

impl ProbeOctree {
    pub fn read(g: &[u8], origin: Vec3, size: f32, probes: usize) -> Option<Self> {
        if g.get(0..4)? != b"GEPO" || u32_at(g, 4)? != 0x28 {
            return None;
        }
        let count = u16_at(g, 8)? as usize;
        let k = (u16_at(g, 10)? as usize * (i32_at(g, 0x2c)? as usize + i32_at(g, 0x0c)? as usize) + 3) & !3;
        let root = 0x4c + 4 * k;
        let nodes: Option<Vec<_>> = (0..count.max(1))
            .map(|i| {
                let n = root + 0x80 * i;
                let mut grid = [0u32; 27];
                for (j, v) in grid.iter_mut().enumerate() {
                    *v = u32_at(g, n + 4 * j)?;
                }
                // (A lone root has no children stored: its grid only.)
                let mut ch = [0xffffu16; 8];
                for (j, v) in ch.iter_mut().enumerate().filter(|_| count > 0) {
                    *v = u16_at(g, n + 0x70 + 2 * j)?;
                }
                Some((grid, ch))
            })
            .collect();
        let pairs = root + 0x80 * count;
        let virtuals: Option<Vec<_>> = (0..i32_at(g, 0x30)?.max(0) as usize).map(|i| Some((u32_at(g, pairs + 8 * i)?, u32_at(g, pairs + 8 * i + 4)?))).collect();
        Some(ProbeOctree { origin, size, probes, nodes: nodes?, flat: count == 0, virtuals: virtuals? })
    }

    /// The real probes' values with the virtual ones after them.
    pub fn with_virtuals(&self, real: &[ProbeSh]) -> Vec<ProbeSh> {
        let mut v = real.to_vec();
        v.resize(self.probes, [Vec3::ZERO; 4]);
        for &(a, b) in &self.virtuals {
            let (pa, pb) = (v.get(a as usize).copied().unwrap_or_default(), v.get(b as usize).copied().unwrap_or_default());
            v.push(std::array::from_fn(|k| (pa[k] + pb[k]) * 0.5));
        }
        v
    }

    /// The light at `p` (Unreal space, cm) and the size of the cell it came from: in the smallest node holding it,
    /// trilinear between the corners of its octant. None outside the octree. PLACEHOLDER: the game's interpolation
    /// (DoProbeInterpolationTask) isn't read.
    pub fn sample(&self, values: &[ProbeSh], p: Vec3) -> Option<(ProbeSh, f32)> {
        let rel = p - self.origin;
        if rel.min_element() < 0.0 || rel.max_element() >= self.size {
            return None;
        }
        let get = |i: u32| values.get(i as usize).copied().unwrap_or_default();
        let mix = |corners: [ProbeSh; 8], t: Vec3| -> ProbeSh {
            std::array::from_fn(|k| {
                let c = |i: usize| corners[i][k];
                let y0 = c(0).lerp(c(1), t.x).lerp(c(2).lerp(c(3), t.x), t.y);
                let y1 = c(4).lerp(c(5), t.x).lerp(c(6).lerp(c(7), t.x), t.y);
                y0.lerp(y1, t.z)
            })
        };
        let cell = |x: usize, y: usize, z: usize| x + (z * 3 + y) * 3;
        if self.flat {
            let (grid, _) = &self.nodes[0];
            let corners = std::array::from_fn(|i| get(grid[cell((i & 1) * 2, (i >> 1 & 1) * 2, (i >> 2 & 1) * 2)]));
            return Some((mix(corners, rel / self.size), self.size));
        }
        let (mut node, mut o, mut s) = (0usize, self.origin, self.size);
        loop {
            let h = s * 0.5;
            let c = ((p - o) / h).floor().clamp(Vec3::ZERO, Vec3::ONE);
            let oct = c.x as usize + 2 * c.y as usize + 4 * c.z as usize;
            let (grid, children) = self.nodes.get(node)?;
            let co = o + c * h;
            if children[oct] != 0xffff {
                node += children[oct] as usize;
                o = co;
                s = h;
                continue;
            }
            let (cx, cy, cz) = (c.x as usize, c.y as usize, c.z as usize);
            let corners = std::array::from_fn(|i| get(grid[cell(cx + (i & 1), cy + (i >> 1 & 1), cz + (i >> 2 & 1))]));
            return Some((mix(corners, ((p - co) / h).clamp(Vec3::ZERO, Vec3::ONE)), h));
        }
    }
}

/// The probe solver's 256 directions (the default palette's, 0x141d884d0, from the table at 0x1429bdbc0; the basis
/// axes the identity, so a palette entry is (1, x, y, z)); entry 0 is zero.
pub static PROBE_DIRECTIONS: std::sync::LazyLock<Vec<[f32; 3]>> = std::sync::LazyLock::new(|| serde_json::from_str(include_str!("probe_directions.json")).unwrap_or_default());

#[cfg(test)]
mod tests {
    use super::*;

    /// Every exported system's blocks read (when `F13_EXPORT` has `enlighten/`): each transport's pixels are its
    /// output's, each workspace's points its header's; and lit by a uniform 1 from every cluster of every input, a
    /// pixel gathers the fraction of its view the transport covers: between 0 and about 1.
    #[test]
    fn exported_systems_read() {
        let Some(root) = std::env::var_os("F13_EXPORT").map(std::path::PathBuf::from) else { return };
        let Ok(dir) = std::fs::read_dir(root.join("enlighten")) else { return };
        let (mut transports, mut workspaces, mut visibilities, mut max) = (0, 0, 0, 0.0f32);
        let mut shares = Vec::new();
        for f in dir.flatten() {
            let path = f.path();
            let name = path.file_name().unwrap().to_string_lossy().to_string();
            // (Only the blocks: the folder also holds meshes/ and the primitives' UVs.)
            if ![".gesg.bin", ".geiw.bin", ".gerd.bin", ".gevs.bin"].iter().any(|e| name.ends_with(e)) {
                continue;
            }
            let bytes = std::fs::read(&path).unwrap();
            if name.ends_with(".gesg.bin") {
                let t = Transport::read(&bytes).unwrap_or_else(|| panic!("{name}: transport doesn't read"));
                let pixels: usize = t.blocks.iter().map(|b| b.pixels.len() + b.duplicates.len()).sum();
                assert!(pixels <= t.width as usize * t.height as usize, "{name}: {pixels} pixels");
                // The environment lit by 1 (the systems dark): each pixel gathers about the sky's share of its view.
                let ones = vec![Vec4::ONE; 1 << 16];
                let inputs: Vec<Option<&[Vec4]>> = t.inputs.iter().map(|id| (id[2] == 0xffff_fffe).then_some(&ones[..])).collect();
                let e = t.solve(&inputs, 1.0);
                let m = e.iter().map(|v| v.x).fold(0.0f32, f32::max);
                max = max.max(m);
                assert!(e.iter().all(|v| v.x >= 0.0 && v.x < 2.0), "{name}: the sky's share {m}");
                shares.extend(e.iter().map(|v| v.x).filter(|&x| x > 0.0));
                transports += 1;
            } else if name.ends_with(".gevs.bin") {
                Visibility::read(&bytes).unwrap_or_else(|| panic!("{name}: visibility doesn't read"));
                visibilities += 1;
            } else if name.ends_with(".gerd.bin") {
                Resampling::read(&bytes).unwrap_or_else(|| panic!("{name}: resampling doesn't read"));
            } else if name.ends_with(".geiw.bin") {
                // (Some systems have no clusters.)
                InputWorkspace::read(&bytes).unwrap_or_else(|| panic!("{name}: workspace doesn't read"));
                workspaces += 1;
            }
        }
        assert!(transports > 200 && workspaces > 500 && visibilities > 500, "{transports} transports, {workspaces} workspaces, {visibilities} visibilities");

        // A light straight down on the terrain's (mostly upward) clusters lights them nearly fully; straight up, not at all.
        let t = std::fs::read(root.join("enlighten/CrystalLake_Small_Terrain.EnlightenSystem_Medium_Dynamic_12.geiw.bin")).unwrap();
        let w = InputWorkspace::read(&t).unwrap();
        let down = w.directional(Vec3::NEG_Z, Vec4::ONE, None);
        let up = w.directional(Vec3::Z, Vec4::ONE, None);
        let mean = |v: &[Vec4]| v.iter().map(|c| c.x).sum::<f32>() / v.len() as f32;
        assert!(mean(&down) > 0.8 && mean(&up) < 0.05, "down {} up {}", mean(&down), mean(&up));
        // Its precomputed visibility: every cluster sees a light above it, none one below.
        let v = Visibility::read(&std::fs::read(root.join("enlighten/CrystalLake_Small_Terrain.EnlightenSystem_Medium_Dynamic_12.gevs.bin")).unwrap()).unwrap();
        let seen = |d: Vec3| v.directional(d, w.clusters.len()).iter().sum::<f32>() / w.clusters.len() as f32;
        assert!(seen(Vec3::Z) > 0.99 && seen(Vec3::NEG_Z) < 0.01, "above {} below {}", seen(Vec3::Z), seen(Vec3::NEG_Z));
        shares.sort_by(f32::total_cmp);
        let at = |f: f64| shares[((shares.len() - 1) as f64 * f) as usize];
        eprintln!("{transports} transports, {workspaces} workspaces; the sky's share: median {}, p99 {}, p99.9 {}, max {max}", at(0.5), at(0.99), at(0.999));
    }

    /// Every exported probe set reads (its transport and octree); lit by the sky alone (1 on every environment
    /// cluster) its probes gather their share of the sky, about 1 in the open; the octree's samples stay within its
    /// probes' range.
    #[test]
    fn exported_probe_sets_read() {
        let Some(root) = std::env::var_os("F13_EXPORT").map(std::path::PathBuf::from) else { return };
        let Ok(text) = std::fs::read_to_string(root.join("enlighten/index.json")) else { return };
        let index: serde_json::Value = serde_json::from_str(&text).unwrap();
        let ones = vec![Vec4::ONE; 4096];
        let (mut sets, mut shares, mut terrain_probe) = (0, Vec::new(), None);
        for p in index["probe_sets"].as_array().into_iter().flatten() {
            let name = format!("{}.{}", p["package"].as_str().unwrap(), p["name"].as_str().unwrap());
            let t = ProbeTransport::read(&std::fs::read(root.join(format!("enlighten/{name}.gesh.bin"))).unwrap()).unwrap_or_else(|| panic!("{name}: transport"));
            let n = p["probes"].as_u64().unwrap() as usize;
            let f = |v: &serde_json::Value| v.as_f64().unwrap() as f32;
            let origin = Vec3::new(f(&p["origin"][0]), f(&p["origin"][1]), f(&p["origin"][2]));
            let o = ProbeOctree::read(&std::fs::read(root.join(format!("enlighten/{name}.gepo.bin"))).unwrap(), origin, f(&p["size"]), n).unwrap_or_else(|| panic!("{name}: octree"));
            let inputs: Vec<Option<&[Vec4]>> = t.inputs.iter().map(|id| (id[2] == 0xffff_fffe).then_some(&ones[..])).collect();
            let sh = t.solve(&inputs, n);
            shares.extend(sh.iter().map(|s| s[0].x));
            if name == "CrystalLake_Small_Terrain.EnlightenProbeSet_High_Dynamic_1" {
                terrain_probe = Some((sh[0][0].x, sh[0][3].x));
            }
            let all = o.with_virtuals(&sh);
            let max = all.iter().map(|s| s[0].x).fold(0.0f32, f32::max);
            for k in 0..64 {
                let q = origin + Vec3::new((k % 4) as f32 + 0.5, (k / 4 % 4) as f32 + 0.5, (k / 16) as f32 + 0.5) * (o.size / 4.0);
                let (v, _) = o.sample(&all, q).unwrap_or_else(|| panic!("{name}: no sample at {q}"));
                assert!(v[0].x >= -1e-4 && v[0].x <= max + 1e-4, "{name}: {} outside 0..{max}", v[0].x);
            }
            sets += 1;
        }
        assert!(sets > 100, "{sets} probe sets");
        shares.sort_by(f32::total_cmp);
        let at = |q: f64| shares[((shares.len() - 1) as f64 * q) as usize];
        eprintln!("{sets} probe sets, {} probes; the sky's share: median {}, p90 {}, p99 {}, max {}", shares.len(), at(0.5), at(0.9), at(0.99), at(1.0));
        assert!(at(0.99) > 0.8 && at(1.0) < 1.6, "the sky's share p99 {} max {}", at(0.99), at(1.0));
        // A probe over open terrain (CrystalLake_Small_Terrain's set 1, its first): about half the sky, from above
        // (its moment (0, 0, 1/4): z is Unreal's up).
        let (m0, mz) = terrain_probe.expect("the terrain's probe set");
        assert!((0.4..0.6).contains(&m0) && (0.2..0.3).contains(&mz), "the terrain probe: mean {m0}, up moment {mz}");
    }
}
