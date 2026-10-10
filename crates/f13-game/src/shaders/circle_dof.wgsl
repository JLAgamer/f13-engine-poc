// The cine cameras' Circle DOF (circle_dof.rs) at the default r.DepthOfFieldQuality 2: FPostProcessCircleDOFSetupPS<1>
// (r.DepthOfField.FarBlur 1), FPostProcessCircleDOFDilatePS<0>, FPostProcessCircleDOFPS0 (one iteration) and
// FPostProcessCircleDOFRecombinePS0, from the game's GlobalShaderCache (F13-decomp globalshaders/asm/*Circle*), constants
// exact. Their inputs are sampled point, border 0 (the passes' sampler state: Filter 0, AddressU/V 3): texel loads.
#import bevy_core_pipeline::fullscreen_vertex_shader::FullscreenVertexOutput

struct Params {
    // CircleDofHalfCoc: x the CoC scale (half-size pixels), y 1e-5 / DepthBlurAmount, z the depth blur's radius, w the
    // view's width / 1920.
    coc: vec4<f32>,
    // x: DepthOfFieldFocalDistance (cm); y: the frame's random offset (Halton base 2 of FrameIndex & 1023); z: the
    // projection's near plane (m).
    focus: vec4<f32>,
}

@group(0) @binding(0) var tex0: texture_2d<f32>;
@group(0) @binding(1) var tex1: texture_2d<f32>;
@group(0) @binding(2) var<uniform> p: Params;
#ifdef MULTISAMPLED
@group(0) @binding(3) var depth: texture_depth_multisampled_2d;
#else
@group(0) @binding(3) var depth: texture_depth_2d;
#endif

// SceneDepth (cm) at a full-size pixel.
fn scene_depth(px: vec2<i32>) -> f32 {
    let size = vec2<i32>(textureDimensions(depth));
    let d = textureLoad(depth, clamp(px, vec2<i32>(0), size - 1), 0);
    return 100.0 * p.focus.z / d;
}

// The signed CoC (half-size pixels): sign(depth - focus) x max((1 - exp(-depth y)) z, |(depth - focus) / depth| x).
fn coc(d: f32) -> f32 {
    let r = (d - p.focus.x) / d * p.coc.x;
    let m = max((1.0 - exp(-d * p.coc.y)) * p.coc.z, abs(r));
    return select(m, -m, r < 0.0);
}

// A point sample with a border of 0.
fn load(t: texture_2d<f32>, uv: vec2<f32>) -> vec4<f32> {
    let size = vec2<i32>(textureDimensions(t));
    let px = vec2<i32>(floor(uv * vec2<f32>(size)));
    if any(px < vec2<i32>(0)) || any(px >= size) {
        return vec4<f32>(0.0);
    }
    return textureLoad(t, px, 0);
}

fn hash(x: vec2<f32>) -> f32 {
    return fract(sin(dot(x, vec2<f32>(12.9898, 78.233))) * 43758.546875);
}

fn dir(a: f32) -> vec2<f32> {
    return vec2<f32>(cos(a), sin(a));
}

// Setup<1>, half size: each of the 2x2 full-size pixels' CoC; the one kept (a signed compare against the kept one's
// size, in gather order) in alpha, the colours weighted by 1.4 - min(|CoC - kept| x 64, 1).
@fragment
fn setup(in: FullscreenVertexOutput) -> @location(0) vec4<f32> {
    let size = vec2<i32>(textureDimensions(tex0));
    let half_size = vec2<f32>((size + 1) / 2);
    let base = vec2<i32>(floor(in.uv * half_size)) * 2;
    // gather4 order: (0, 1), (1, 1), (1, 0), (0, 0).
    let offsets = array<vec2<i32>, 4>(vec2<i32>(0, 1), vec2<i32>(1, 1), vec2<i32>(1, 0), vec2<i32>(0, 0));
    var c: array<f32, 4>;
    for (var i = 0; i < 4; i++) {
        c[i] = coc(scene_depth(base + offsets[i]));
    }
    var kept = select(c[0], c[1], c[1] < abs(c[0]));
    kept = select(kept, c[2], c[2] < abs(kept));
    kept = select(kept, c[3], c[3] < abs(kept));
    var sum = vec3<f32>(0.0);
    var weights = 0.0;
    for (var i = 0; i < 4; i++) {
        let w = 1.4 - min(abs(c[i] - kept) * 64.0, 1.0);
        let px = clamp(base + offsets[i], vec2<i32>(0), size - 1);
        sum += min(textureLoad(tex0, px, 0).rgb, vec3<f32>(65536.0)) * w;
        weights += w;
    }
    return vec4<f32>(sum / weights, kept);
}

// Dilate: the nearest (lowest) CoC of the 4x4 half-size pixels about this one (-1..+2), at most 0.
@fragment
fn dilate(in: FullscreenVertexOutput) -> @location(0) vec4<f32> {
    let size = vec2<i32>(textureDimensions(tex0));
    let px = vec2<i32>(floor(in.uv * vec2<f32>(size)));
    var m = 1.0e30;
    for (var y = -1; y <= 2; y++) {
        for (var x = -1; x <= 2; x++) {
            let at = px + vec2<i32>(x, y);
            var v = 0.0;
            if all(at >= vec2<i32>(0)) && all(at < size) {
                v = textureLoad(tex0, at, 0).a;
            }
            m = min(m, v);
        }
    }
    return vec4<f32>(min(m, 0.0));
}

// One tap of a ring: its weight saturate((|CoC| - fraction x radius) x 0.25).
fn tap_weight(s: vec4<f32>, fraction: f32, radius: f32) -> f32 {
    return saturate((abs(s.w) - fraction * radius) * 0.25);
}

// CircleDOFPS0 (one iteration): a near-field probe of four dilated taps, then three rings of four taps of the setup's
// colour, each ring's radius the nearest CoC found so far.
@fragment
fn gather(in: FullscreenVertexOutput) -> @location(0) vec4<f32> {
    let uv = in.uv;
    let texel = 1.0 / vec2<f32>(textureDimensions(tex0));
    let reach = p.coc.w * 8.0;
    let centre = load(tex0, uv);
    var acc = vec4<f32>(0.0);
    for (var i = 0u; i < 1u; i++) {
        let z = f32(i) + p.focus.y;
        let r3 = vec4<f32>(z * 0.01, z * 0.01, z * 0.013, z * 0.013) + uv.xyxy;
        let a = hash(r3.xy) * 2.0 - 1.0;
        let h = hash(r3.zw);
        let f0 = h * 0.08695652335882187;
        // The probe: radii sqrt(h / 4 + (0.75, 0.5, 0.25, 0.15)) x 8 x width / 1920.
        let pr = sqrt(h * 0.25 + vec4<f32>(0.75, 0.5, 0.25, 0.15000000596046448)) * reach;
        let a0 = a * 6.283180236816406;
        let probe = array<vec2<f32>, 4>(
            dir(a0) * pr.x,
            dir(a * 6.283180236816406 + 3.141590118408203) * pr.y,
            dir(a * 6.283180236816406 + 1.5707950592041016) * pr.z,
            dir(a * 6.283180236816406 + 4.712385177612305) * pr.w,
        );
        var nearest = 65536.0;
        for (var k = 0; k < 4; k++) {
            let d = load(tex1, probe[k] * texel + uv).x;
            if pr[k] < abs(d) {
                nearest = min(d, nearest);
            }
        }
        let near = min(centre.w, nearest);
        // Ring 1 (fractions 18, 6, 12 / 23 and h x 2 / 23) at R = max(|near|, |centre's CoC|).
        let r = max(abs(near), abs(centre.w));
        let fr1 = f0 + vec4<f32>(0.782608687877655, 0.260869562625885, 0.52173912525177, 0.8695651888847351);
        let rad1 = vec4<f32>(r * fr1.x, r * fr1.y, r * fr1.z, r * f0);
        let s1 = load(tex0, dir(a0) * max(rad1.x, 1.0) * texel + uv);
        let s2 = load(tex0, dir(a * 6.283180236816406 + 1.5707950592041016) * max(rad1.y, 1.0) * texel + uv);
        let s3 = load(tex0, dir(a * 6.283180236816406 + 3.141590118408203) * max(rad1.z, 1.0) * texel + uv);
        let s4 = load(tex0, dir(a * 6.283180236816406 + 4.712385177612305) * max(rad1.w, 1.0) * texel + uv);
        var w1 = tap_weight(s1, fr1.x, r);
        var w2 = tap_weight(s2, fr1.y, r);
        var w3 = tap_weight(s3, fr1.z, r);
        var w4 = tap_weight(s4, f0, r);
        var ring = centre.w;
        if rad1.x * w1 > 0.0 { ring = min(centre.w, s1.w); }
        if rad1.y * w2 > 0.0 { ring = min(s2.w, ring); }
        if rad1.z * w3 > 0.0 { ring = min(s3.w, ring); }
        if rad1.w * w4 > 0.0 { ring = min(s4.w, ring); }
        // How much the near field spreads over this pixel.
        let sum = centre.w + s1.w + s2.w + s3.w + s4.w;
        let most = max(centre.w, max(max(s1.w, s2.w), max(s4.w, s3.w)));
        let spread = saturate((sum * 0.2 - most) / (near - most) * 2.0) * saturate(-near);
        w1 = spread * (1.0 - w1) + w1;
        w2 = spread * (1.0 - w2) + w2;
        w3 = spread * (1.0 - w3) + w3;
        w4 = spread * (1.0 - w4) + w4;
        let middle = 1.0000152587890625 - min(r, 1.0);
        var colour = s1.rgb * (w1 * rad1.x) + centre.rgb * middle + s2.rgb * (w2 * rad1.y) + s3.rgb * (w3 * rad1.z) + s4.rgb * (w4 * rad1.w);
        var total = middle + rad1.x * w1 + rad1.y * w2 + rad1.z * w3 + rad1.w * w4;
        // Ring 2 (20, 8, 14, 2 / 23) at R2 = max(|near|, |ring's nearest|).
        let r2 = max(abs(near), abs(ring));
        let keep2 = 1.0000152587890625 - min(abs(r - r2), 1.0);
        let fr2 = f0 + vec4<f32>(0.3478260934352875, 0.6086956262588501, 0.08695652335882187, 0.95652174949646);
        let ra = r2 * fr1.w;
        let rad2 = r2 * fr2.xyz;
        let sa = load(tex0, dir(a * 6.283180236816406 + 4.18878698348999) * max(ra, 1.0) * texel + uv);
        let sb = load(tex0, dir(a * 6.283180236816406 + 5.759582042694092) * max(rad2.x, 1.0) * texel + uv);
        let sc = load(tex0, dir(a * 6.283180236816406 + 1.0471967458724976) * max(rad2.y, 1.0) * texel + uv);
        let sd = load(tex0, dir(a * 6.283180236816406 + 2.6179916858673096) * max(rad2.z, 1.0) * texel + uv);
        var wa = tap_weight(sa, fr1.w, r2);
        var wb = tap_weight(sb, fr2.x, r2);
        var wc = tap_weight(sc, fr2.y, r2);
        var wd = tap_weight(sd, fr2.z, r2);
        var ring2 = ring;
        if ra * wa > 0.0 { ring2 = min(ring, sa.w); }
        if rad2.x * wb > 0.0 { ring2 = min(sb.w, ring2); }
        if rad2.y * wc > 0.0 { ring2 = min(sc.w, ring2); }
        if rad2.z * wd > 0.0 { ring2 = min(sd.w, ring2); }
        wa = spread * (1.0 - wa) + wa;
        wb = spread * (1.0 - wb) + wb;
        wc = spread * (1.0 - wc) + wc;
        wd = spread * (1.0 - wd) + wd;
        colour = sb.rgb * (wb * rad2.x) + sa.rgb * (wa * ra) + sc.rgb * (wc * rad2.y) + sd.rgb * (wd * rad2.z) + colour * keep2;
        total = ra * wa + wb * rad2.x + rad2.y * wc + rad2.z * wd + total * keep2;
        // Ring 3 (22, 10, 16, 4 / 23) at R3 = max(|near|, |ring 2's nearest|).
        let r3r = max(abs(near), abs(ring2));
        let keep3 = 1.0000152587890625 - min(abs(r2 - r3r), 1.0);
        let nearest_coc = min(near, ring2);
        let fr3 = f0 + vec3<f32>(0.43478259444236755, 0.695652186870575, 0.17391304671764374);
        let re = r3r * fr2.w;
        let rad3 = r3r * fr3;
        let se = load(tex0, dir(a * 6.283180236816406 + 2.094393491744995) * max(re, 1.0) * texel + uv);
        let sf = load(tex0, dir(a * 6.283180236816406 + 3.6651885509490967) * max(rad3.x, 1.0) * texel + uv);
        let sg = load(tex0, dir(a * 6.283180236816406 + 5.235983371734619) * max(rad3.y, 1.0) * texel + uv);
        let sh = load(tex0, dir(a * 6.283180236816406 + 0.5235983729362488) * max(rad3.z, 1.0) * texel + uv);
        var we = tap_weight(se, fr2.w, r3r);
        var wf = tap_weight(sf, fr3.x, r3r);
        var wg = tap_weight(sg, fr3.y, r3r);
        var wh = tap_weight(sh, fr3.z, r3r);
        we = spread * (1.0 - we) + we;
        wf = spread * (1.0 - wf) + wf;
        wg = spread * (1.0 - wg) + wg;
        wh = spread * (1.0 - wh) + wh;
        colour = sf.rgb * (wf * rad3.x) + se.rgb * (we * re) + sg.rgb * (wg * rad3.y) + sh.rgb * (wh * rad3.z) + colour * keep3;
        total = re * we + wf * rad3.x + rad3.y * wg + rad3.z * wh + total * keep3;
        acc += vec4<f32>(colour / total, nearest_coc);
    }
    return acc;
}

// RecombinePS0, full size: the CoC (the depth's, at most the DOF result's), two scene samples mirrored on a hashed disc
// of twice its size, clamped into the half-size result's 2x2 range (widened by min / 8 while the CoC is small), blended
// in by min(CoC^2 x 4, 1).
@fragment
fn recombine(in: FullscreenVertexOutput) -> @location(0) vec4<f32> {
    let uv = in.uv;
    let size = vec2<i32>(textureDimensions(tex0));
    let texel = 1.0 / vec2<f32>(size);
    let px = vec2<i32>(floor(uv * vec2<f32>(size)));
    let c = min(load(tex1, uv).w, coc(scene_depth(px)));
    let radius = abs(c) + abs(c);
    var blur = vec3<f32>(0.0);
    for (var i = 0u; i < 1u; i++) {
        let z = f32(i) + p.focus.y;
        let angle = hash(uv + z * 0.01) * 3.1415927410125732;
        let len = sqrt(hash(uv + z * 0.013)) * radius;
        let o = dir(angle) * len;
        let a = min(load(tex0, o * texel + uv).rgb, vec3<f32>(65536.0));
        let b = min(load(tex0, uv - o * texel).rgb, vec3<f32>(65536.0));
        blur += b * 1.52587890625e-05 + a * 1.52587890625e-05;
    }
    blur *= 32768.0;
    let half_size = vec2<f32>(textureDimensions(tex1));
    let at = vec2<i32>(trunc(uv * half_size - 0.5));
    let hs = vec2<i32>(half_size);
    var q: array<vec3<f32>, 4>;
    let offsets = array<vec2<i32>, 4>(vec2<i32>(0, 0), vec2<i32>(1, 0), vec2<i32>(0, 1), vec2<i32>(1, 1));
    for (var k = 0; k < 4; k++) {
        let t = at + offsets[k];
        q[k] = select(vec3<f32>(0.0), textureLoad(tex1, clamp(t, vec2<i32>(0), hs - 1), 0).rgb, all(t >= vec2<i32>(0)) && all(t < hs));
    }
    let hi = max(max(q[0], q[1]), max(q[3], q[2]));
    let lo = min(min(q[3], q[2]), min(q[0], q[1]));
    let k = radius * radius;
    let widen = lo * 0.125 * (1.0 - min(k * 0.015625, 1.0));
    let clamped = min(widen + hi, max(blur, lo - widen));
    return vec4<f32>(min(k * 4.0, 1.0) * (clamped - blur) + blur, 1.0);
}
