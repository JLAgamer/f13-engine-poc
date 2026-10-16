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

impl InputWorkspace {
    /// Each cluster's light from a directional light shining along `dir` (Unreal space, normalised) with `color`, as
    /// DoDirectInputLighting's type 2 kernel (0x141dd3f10): per packet the mean of max(0, N . -dir) over its four
    /// points, the cluster the mean over its packets (rcp of their count). PLACEHOLDER: every point seen (the light's
    /// visibility bits, PrecomputedVisibilityData GEVS, aren't read yet).
    pub fn directional(&self, dir: Vec3, color: Vec4) -> Vec<Vec4> {
        let to_light = -dir;
        self.clusters
            .iter()
            .map(|packets| {
                if packets.is_empty() {
                    return Vec4::ZERO;
                }
                let sum: f32 = packets.iter().map(|p| p.normal.dot(to_light).max(0.0)).sum();
                color * (sum / packets.len() as f32)
            })
            .collect()
    }
}

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
        let (mut transports, mut workspaces, mut max) = (0, 0, 0.0f32);
        let mut shares = Vec::new();
        for f in dir.flatten() {
            let path = f.path();
            let name = path.file_name().unwrap().to_string_lossy().to_string();
            // (Only the blocks: the folder also holds meshes/ and the primitives' UVs.)
            if !name.ends_with(".gesg.bin") && !name.ends_with(".geiw.bin") {
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
            } else if name.ends_with(".geiw.bin") {
                // (Some systems have no clusters.)
                InputWorkspace::read(&bytes).unwrap_or_else(|| panic!("{name}: workspace doesn't read"));
                workspaces += 1;
            }
        }
        assert!(transports > 200 && workspaces > 500, "{transports} transports, {workspaces} workspaces");

        // A light straight down on the terrain's (mostly upward) clusters lights them nearly fully; straight up, not at all.
        let t = std::fs::read(root.join("enlighten/CrystalLake_Small_Terrain.EnlightenSystem_Medium_Dynamic_12.geiw.bin")).unwrap();
        let w = InputWorkspace::read(&t).unwrap();
        let down = w.directional(Vec3::NEG_Z, Vec4::ONE);
        let up = w.directional(Vec3::Z, Vec4::ONE);
        let mean = |v: &[Vec4]| v.iter().map(|c| c.x).sum::<f32>() / v.len() as f32;
        assert!(mean(&down) > 0.8 && mean(&up) < 0.05, "down {} up {}", mean(&down), mean(&up));
        shares.sort_by(f32::total_cmp);
        let at = |f: f64| shares[((shares.len() - 1) as f64 * f) as usize];
        eprintln!("{transports} transports, {workspaces} workspaces; the sky's share: median {}, p99 {}, p99.9 {}, max {max}", at(0.5), at(0.99), at(0.999));
    }
}
