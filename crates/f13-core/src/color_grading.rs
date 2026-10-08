//! The engine's colour pipeline baked into a 32³ LUT, as this build's CombineLUTs pass computes it for an SDR (sRGB)
//! display: FLUTBlenderPS<2> from the game's GlobalShaderCache-PCD3D_SM5.bin (MODLOG "Post-processing"), its constants
//! read exactly from the bytecode. In order: the LUT cell's log-encoded colour back to linear; white balance (WhiteTemp,
//! WhiteTint: the Planckian / daylight locus and a Bradford adaptation to D65); colour correction in ACEScg (AP1) with
//! the global, shadows, midtones and highlights saturation / contrast / gamma / gain / offset; the film curve
//! (FilmSlope, Toe, Shoulder, BlackClip, WhiteClip: this build's r.TonemapperFilm default is 1); sRGB encoding; the
//! colour grading LUTs blended by weight; the colour remap polynomial; the camera's colour scale and overlay; the display
//! gamma; sRGB again. The tonemap pass (FPostProcessTonemapPS) multiplies what it reads by 1.05, hence the 1 / 1.05.

use glam::Vec3;

/// The LUT's size along each axis.
pub const LUT_SIZE: usize = 32;

/// One set of colour correction controls (xyz per channel, w for all three).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Correction {
    pub saturation: [f32; 4],
    pub contrast: [f32; 4],
    pub gamma: [f32; 4],
    pub gain: [f32; 4],
    pub offset: [f32; 4],
}

impl Default for Correction {
    fn default() -> Self {
        Correction { saturation: [1.0; 4], contrast: [1.0; 4], gamma: [1.0; 4], gain: [1.0; 4], offset: [0.0; 4] }
    }
}

/// CombineLUTs' parameters. Defaults: FPostProcessSettings' and the view's (no fade, display gamma 2.2).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CombineLuts {
    pub white_temp: f32,
    pub white_tint: f32,
    pub global: Correction,
    pub shadows: Correction,
    pub midtones: Correction,
    pub highlights: Correction,
    pub shadows_max: f32,
    pub highlights_min: f32,
    pub film_slope: f32,
    pub film_toe: f32,
    pub film_shoulder: f32,
    pub film_black_clip: f32,
    pub film_white_clip: f32,
    /// The colour remap polynomial (x², x, 1): r.Color.Min/Mid/Max's defaults give the identity.
    pub mapping_polynomial: [f32; 3],
    /// The view's colour scale and overlay (camera fades).
    pub color_scale: [f32; 3],
    pub overlay: [f32; 4],
    /// InverseGamma.y: 2.2 / the display gamma.
    pub inverse_gamma: f32,
    /// LUTWeights: the neutral table's, then the colour grading LUT's (ColorGradingIntensity).
    pub weights: [f32; 2],
}

impl Default for CombineLuts {
    fn default() -> Self {
        CombineLuts {
            white_temp: 6500.0,
            white_tint: 0.0,
            global: Correction::default(),
            shadows: Correction::default(),
            midtones: Correction::default(),
            highlights: Correction::default(),
            shadows_max: 0.09,
            highlights_min: 0.5,
            film_slope: 0.88,
            film_toe: 0.55,
            film_shoulder: 0.26,
            film_black_clip: 0.0,
            film_white_clip: 0.04,
            mapping_polynomial: [0.0, 1.0, 0.0],
            color_scale: [1.0; 3],
            overlay: [0.0; 4],
            inverse_gamma: 1.0,
            weights: [1.0, 0.0],
        }
    }
}

/// Rows of a 3x3 matrix applied as dot products (the shader's dp3s).
fn rows(m: [[f32; 3]; 3], v: Vec3) -> Vec3 {
    Vec3::new(Vec3::from(m[0]).dot(v), Vec3::from(m[1]).dot(v), Vec3::from(m[2]).dot(v))
}

const SRGB_2_AP1: [[f32; 3]; 3] = [
    [0.6131914854049683, 0.33951207995414734, 0.04736633226275444],
    [0.0702069029211998, 0.9163358211517334, 0.013450011610984802],
    [0.020618870854377747, 0.10956729203462601, 0.8696067333221436],
];
const AP1_2_SRGB: [[f32; 3]; 3] = [
    [1.7050515413284302, -0.6217907071113586, -0.08325839787721634],
    [-0.13025714457035065, 1.1408028602600098, -0.010548528283834457],
    [-0.024003274738788605, -0.12896877527236938, 1.1529717445373535],
];
const AP1_RGB2Y: Vec3 = Vec3::new(0.2722287178039551, 0.6740817427635193, 0.053689517080783844);

/// The white balance matrix (applied to linear sRGB) for a colour temperature and tint.
fn white_balance(temp: f32, tint: f32) -> [[f32; 3]; 3] {
    // The daylight locus (D_IlluminantChromaticity), with the 1.4388 / 1.438 correction of Planck's law.
    let t = temp * 1.0005563497543335;
    let (a, b) = if temp <= 6996.10791015625 { (4607000064.0f32, 2967800.0f32) } else { (2006400000.0, 1901800.0) };
    let (c, d) = if temp <= 6996.10791015625 { (99.11000061035156f32, 0.24406300485134125f32) } else { (247.47999572753906, 0.23703999817371368) };
    let x = ((((-a / t) + b) / t + c) / t) + d;
    let daylight = glam::Vec2::new(x, x * 2.869999885559082 + x * x * -3.0 - 0.2750000059604645);
    // The Planckian locus (PlanckianLocusChromaticity) in uv, then xy.
    let t2 = temp * temp;
    let num = Vec3::new(0.8601177334785461, 1.0, 0.31739872694015503) + temp * Vec3::new(0.00015411825734190643, 0.0008424202096648514, 4.228062607580796e-05) + t2 * Vec3::new(1.2864121856637212e-07, 7.081451371959702e-07, 4.204816761443908e-08);
    let u = num.x / num.y;
    let v = num.z / (t2 * 1.6145605741257896e-07 + (-temp * 2.8974181986995973e-05 + 1.0));
    let uv_to_xy = |u: f32, v: f32| {
        let den = (u + u) - v * 8.0 + 4.0;
        glam::Vec2::new(u * 3.0 / den, (v + v) / den)
    };
    let planck = uv_to_xy(u, v);
    let mut src = if temp < 4000.0 { planck } else { daylight };
    // Along the isotherm by the tint (PlanckianIsothermal).
    let n = glam::Vec2::new(u, v) * (u * u + v * v).sqrt().recip();
    let iso = uv_to_xy(-n.y * tint * 0.05000000074505806 + u, n.x * tint * 0.05000000074505806 + v);
    src += iso - planck;
    // ChromaticAdaptation (Bradford) from the source white to D65.
    let y = src.y.max(1.000000013351432e-10);
    let xyz = Vec3::new(src.x / y, 1.0, (1.0 - src.x - src.y) / y);
    let r0 = Vec3::new(0.8950999975204468, 0.266400009393692, -0.16140000522136688);
    let r1 = Vec3::new(-0.7501999735832214, 1.7135000228881836, 0.03669999912381172);
    let r2 = Vec3::new(0.03889999911189079, -0.06849999725818634, 1.0296000242233276);
    let s0 = 0.9413792490959167 / r0.dot(xyz);
    let s1 = 1.0404363870620728 / r1.dot(xyz);
    let s2 = 1.089766502380371 / r2.dot(xyz);
    // Columns of diag(scale) x Bradford, then Bradford⁻¹ x that, XYZ <- sRGB, sRGB <- XYZ.
    let scaled = [r0 * s0, r1 * s1, r2 * s2];
    let col = |i: usize| Vec3::new(scaled[0][i], scaled[1][i], scaled[2][i]);
    let inv = [
        Vec3::new(0.9869928956031799, -0.1470542997121811, 0.1599626988172531),
        Vec3::new(0.43230530619621277, 0.5183603167533875, 0.04929120093584061),
        Vec3::new(-0.008528700098395348, 0.04004279896616936, 0.9684867262840271),
    ];
    // m[row][col] = inv[row] . scaled column col.
    let m: [[f32; 3]; 3] = std::array::from_fn(|r| std::array::from_fn(|c| inv[r].dot(col(c))));
    // MS = M x sRGB_2_XYZ (the shader's r3, r4, r5 are its columns), then XYZ_2_sRGB x MS.
    let srgb_to_xyz_cols = [
        Vec3::new(0.4124563932418823, 0.2126729041337967, 0.01933390088379383),
        Vec3::new(0.3575761020183563, 0.7151522040367126, 0.11919199675321579),
        Vec3::new(0.18043750524520874, 0.07217500358819962, 0.9503040909767151),
    ];
    let mrow = |r: usize| Vec3::from(m[r]);
    let ms_col = |c: usize| Vec3::new(mrow(0).dot(srgb_to_xyz_cols[c]), mrow(1).dot(srgb_to_xyz_cols[c]), mrow(2).dot(srgb_to_xyz_cols[c]));
    let xyz_to_srgb = [
        Vec3::new(3.2409698963165283, -1.5373831987380981, -0.4986107647418976),
        Vec3::new(-0.9692436456680298, 1.8759675025939941, 0.04155505821108818),
        Vec3::new(0.05563008040189743, -0.20397695899009705, 1.056971549987793),
    ];
    std::array::from_fn(|r| [xyz_to_srgb[r].dot(ms_col(0)), xyz_to_srgb[r].dot(ms_col(1)), xyz_to_srgb[r].dot(ms_col(2))])
}

/// ColorCorrect with the global and one range's controls multiplied (offsets added).
fn correct(c: Vec3, luma: f32, g: &Correction, r: &Correction) -> Vec3 {
    let m = |a: [f32; 4], b: [f32; 4]| [a[0] * b[0], a[1] * b[1], a[2] * b[2], a[3] * b[3]];
    let (sat, con, gam, gain) = (m(g.saturation, r.saturation), m(g.contrast, r.contrast), m(g.gamma, r.gamma), m(g.gain, r.gain));
    let off = [g.offset[0] + r.offset[0], g.offset[1] + r.offset[1], g.offset[2] + r.offset[2], g.offset[3] + r.offset[3]];
    let v = |a: [f32; 4]| Vec3::new(a[0], a[1], a[2]) * a[3];
    let mut x = (v(sat) * (c - luma) + luma).max(Vec3::ZERO);
    x = (x * 5.55555534362793).powf_v(v(con)) * 0.18000000715255737;
    x = x.powf_v(Vec3::ONE / v(gam));
    x * v(gain) + (Vec3::new(off[0], off[1], off[2]) + off[3])
}

trait PowV {
    fn powf_v(self, e: Vec3) -> Vec3;
}

impl PowV for Vec3 {
    fn powf_v(self, e: Vec3) -> Vec3 {
        Vec3::new(self.x.powf(e.x), self.y.powf(e.y), self.z.powf(e.z))
    }
}

/// The film curve on positive AP1 (the shader's filmic branch).
fn film(c: Vec3, p: &CombineLuts) -> Vec3 {
    let mut w = c.max(Vec3::ZERO);
    let luma = w.dot(AP1_RGB2Y);
    w = (w - luma) * 0.9599999785423279 + luma;
    let (slope, toe, shoulder, black, white) = (p.film_slope, p.film_toe, p.film_shoulder, p.film_black_clip, p.film_white_clip);
    let toe_scale = black + 1.0 - toe;
    let shoulder_scale = white + 1.0 - shoulder;
    let toe_match = if 0.800000011920929 < toe {
        (0.8199999928474426 - toe) / slope - 0.7447274923324585
    } else {
        let bt = (black + 0.18000000715255737) / toe_scale;
        let bt = bt / (1.0 - (bt - 1.0));
        -(bt.log2() * 0.3465735912322998) * (toe_scale / slope) - 0.7447274923324585
    };
    let straight_match = (1.0 - toe) / slope - toe_match;
    let shoulder_match = shoulder / slope - straight_match;
    let log = Vec3::new(w.x.log2(), w.y.log2(), w.z.log2()) * 0.3010300099849701;
    let straight = (log + straight_match) * slope;
    let e = |v: Vec3| Vec3::new((v.x * 1.4426950216293335).exp2(), (v.y * 1.4426950216293335).exp2(), (v.z * 1.4426950216293335).exp2());
    let toe_c = Vec3::splat(toe_scale + toe_scale) / (e((log - toe_match) * (slope * -2.0 / toe_scale)) + 1.0) - black;
    let shoulder_c = Vec3::splat(white + 1.0) - Vec3::splat(shoulder_scale + shoulder_scale) / (e((log - shoulder_match) * ((slope + slope) / shoulder_scale)) + 1.0);
    let sel = |m: [bool; 3], a: Vec3, b: Vec3| Vec3::new(if m[0] { a.x } else { b.x }, if m[1] { a.y } else { b.y }, if m[2] { a.z } else { b.z });
    let toe_c = sel([log.x < toe_match, log.y < toe_match, log.z < toe_match], toe_c, straight);
    let shoulder_c = sel([shoulder_match < log.x, shoulder_match < log.y, shoulder_match < log.z], shoulder_c, straight);
    let span = shoulder_match - toe_match;
    let mut t = ((log - toe_match) / span).clamp(Vec3::ZERO, Vec3::ONE);
    if shoulder_match < toe_match {
        t = Vec3::ONE - t;
    }
    t = (Vec3::splat(3.0) - t * 2.0) * t * t;
    let mut tone = (shoulder_c - toe_c) * t + toe_c;
    let luma = tone.dot(AP1_RGB2Y);
    tone = ((tone - luma) * 0.9300000071525574 + luma).max(Vec3::ZERO);
    rows(AP1_2_SRGB, tone).max(Vec3::ZERO)
}

fn linear_to_srgb(c: Vec3) -> Vec3 {
    let f = |x: f32| if x >= 0.0031306699384003878 { (x.log2() * 0.4166666567325592).exp2() * 1.0549999475479126 - 0.054999999701976776 } else { x * 12.920000076293945 };
    Vec3::new(f(c.x), f(c.y), f(c.z))
}

fn srgb_to_linear(c: Vec3) -> Vec3 {
    let f = |x: f32| if 0.040449999272823334 < x { ((x * 0.9478672742843628 + 0.05213269963860512).log2() * 2.4000000953674316).exp2() } else { x * 0.07739938050508499 };
    Vec3::new(f(c.x), f(c.y), f(c.z))
}

/// One cell of the combined LUT: what the tonemap pass reads for a scene colour whose log encoding is `log`.
/// `lut(u, v)` samples the colour grading LUT texture (its 256 x 16 strip, bilinear, clamped).
pub fn combine(log: Vec3, p: &CombineLuts, lut: &dyn Fn(f32, f32) -> Vec3) -> Vec3 {
    let linear = Vec3::new(((log.x - 0.4340175986289978) * 14.0).exp2(), ((log.y - 0.4340175986289978) * 14.0).exp2(), ((log.z - 0.4340175986289978) * 14.0).exp2()) * 0.18000000715255737 - 0.002667719265446067;
    let balanced = rows(white_balance(p.white_temp, p.white_tint), linear);
    let ap1 = rows(SRGB_2_AP1, balanced);
    let luma = ap1.dot(AP1_RGB2Y);
    let shadows = correct(ap1, luma, &p.global, &p.shadows);
    let highlights = correct(ap1, luma, &p.global, &p.highlights);
    let midtones = correct(ap1, luma, &p.global, &p.midtones);
    let s = (luma * (1.0 / p.shadows_max)).clamp(0.0, 1.0);
    let shadow_w = 1.0 - (3.0 - s * 2.0) * s * s;
    let h = ((1.0 / (1.0 - p.highlights_min)) * (luma - p.highlights_min)).clamp(0.0, 1.0);
    let highlight_w = h * h * (3.0 - h * 2.0);
    let mid_w = 1.0 - shadow_w - (3.0 - h * 2.0) * h * h;
    let corrected = midtones * mid_w + shadows * shadow_w + highlights * highlight_w;
    let srgb_lin = rows(AP1_2_SRGB, corrected);
    let toned = film(rows(SRGB_2_AP1, srgb_lin), p).clamp(Vec3::ZERO, Vec3::ONE);
    let encoded = linear_to_srgb(toned);
    // The colour grading LUT: 16 slices of 16 x 16 side by side, blue picking the slice.
    let g = encoded * 0.9375 + 0.03125;
    let slice = g.z * 16.0 - 0.5;
    let base = slice.floor();
    let frac = slice - base;
    let u = (base + g.x) * 0.0625;
    let a = lut(u, g.y);
    let b = lut(u + 0.0625, g.y);
    let graded = (b - a) * frac + a;
    let mut c = (encoded * p.weights[0] + graded * p.weights[1]).max(Vec3::splat(6.103519990574569e-05));
    c = srgb_to_linear(c);
    let [pa, pb, pc] = p.mapping_polynomial;
    c = c * c * pa + c * pb + pc;
    let scale = Vec3::from(p.color_scale);
    let overlay = Vec3::new(p.overlay[0], p.overlay[1], p.overlay[2]);
    c = ((overlay - c * scale) * p.overlay[3] + c * scale).max(Vec3::ZERO);
    let g = Vec3::new(c.x.log2(), c.y.log2(), c.z.log2()) * p.inverse_gamma;
    let lin = Vec3::new(g.x.exp2(), g.y.exp2(), g.z.exp2());
    linear_to_srgb(lin) * 0.9523810148239136
}

/// The whole 32³ table, red fastest, then green, then blue (the slice).
pub fn combine_luts(p: &CombineLuts, lut: &dyn Fn(f32, f32) -> Vec3) -> Vec<Vec3> {
    let n = LUT_SIZE as f32 - 1.0;
    let mut out = Vec::with_capacity(LUT_SIZE * LUT_SIZE * LUT_SIZE);
    for b in 0..LUT_SIZE {
        for g in 0..LUT_SIZE {
            for r in 0..LUT_SIZE {
                // The cell's texture coordinate maps to (i + 0.5) / 32 * 32 / 31 - 0.5 / 31 = i / 31.
                out.push(combine(Vec3::new(r as f32 / n, g as f32 / n, b as f32 / n), p, lut));
            }
        }
    }
    out
}

/// The tonemap pass's log encoding of a scene colour for the LUT lookup (LinToLog), mapped to the 32³ texels' centres.
pub fn lut_coordinate(c: Vec3) -> Vec3 {
    let l = Vec3::new((c.x + 0.002667719265446067).log2(), (c.y + 0.002667719265446067).log2(), (c.z + 0.002667719265446067).log2());
    let log = (l * 0.0714285746216774 + 0.6107269525527954).clamp(Vec3::ZERO, Vec3::ONE);
    log * 0.96875 + 0.015625
}

#[cfg(test)]
mod tests {
    use super::*;

    fn none(_: f32, _: f32) -> Vec3 {
        Vec3::ZERO
    }

    #[test]
    fn white_balance_at_d65_is_near_identity() {
        let m = white_balance(6500.0, 0.0);
        for (r, row) in m.iter().enumerate() {
            for (c, v) in row.iter().enumerate() {
                let want = if r == c { 1.0 } else { 0.0 };
                assert!((v - want).abs() < 0.02, "{m:?}");
            }
        }
    }

    #[test]
    fn mid_grey_stays_near_its_film_value() {
        // 0.18 linear: the film curve maps it to about 0.18 (OutMatch), sRGB ~0.46, over 1.05.
        let p = CombineLuts::default();
        let log = Vec3::splat(((0.18f32 + 0.002667719265446067).log2() * 0.0714285746216774 + 0.6107269525527954).clamp(0.0, 1.0));
        let out = combine(log, &p, &none);
        assert!((out.x - linear_to_srgb(Vec3::splat(0.18)).x / 1.05).abs() < 0.03, "{out}");
        assert!((out.x - out.y).abs() < 1e-3 && (out.y - out.z).abs() < 1e-3);
    }

    #[test]
    fn black_and_white_ends() {
        let p = CombineLuts::default();
        assert!(combine(Vec3::ZERO, &p, &none).max_element() < 0.02);
        assert!(combine(Vec3::ONE, &p, &none).min_element() > 0.9);
    }

    #[test]
    fn the_lut_blends_by_weight() {
        let p = CombineLuts { weights: [0.0, 1.0], ..CombineLuts::default() };
        // A LUT of constant 0.5 (sRGB-encoded) gives 0.5 throughout.
        let out = combine(Vec3::splat(0.3), &p, &|_, _| Vec3::splat(0.5));
        assert!((out.x * 1.05 - 0.5).abs() < 1e-3, "{out}");
    }
}

#[cfg(test)]
mod more_tests {
    use super::*;

    #[test]
    fn grey_through_the_film_curve_holds_its_match_point() {
        // FilmToneMap's OutMatch: 0.18 in (AP1 grey) comes out 0.18.
        let out = film(Vec3::splat(0.18), &CombineLuts::default());
        assert!((out.x - 0.18).abs() < 1e-3, "{out}");
    }
}
