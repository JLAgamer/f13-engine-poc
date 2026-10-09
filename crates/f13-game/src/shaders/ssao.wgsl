// The engine's screen-space ambient occlusion at this game's settings (ssao.rs has the passes and where each value
// comes from): the HZB, the half-size setup, the half-size occlusion (6 sample directions), the full-size occlusion
// (3 directions) upsampling the half-size one, then the copy into Bevy's SSAO texture. Depths are in cm, the view
// space is the engine's (x right, y up, z forward), positions are "screen position x depth, depth".
#import bevy_core_pipeline::fullscreen_vertex_shader::FullscreenVertexOutput

struct Params {
    // World normal to view space: rows for the normal's x, y, z (TranslatedWorldToView).
    to_view: array<vec4<f32>, 3>,
    // InvDeviceZToWorldZTransform.
    devz: vec4<f32>,
    // ScreenSpaceAOParams [0] power, bias, 1 / distance, intensity; [1] random UV scale xy, radius, ratio;
    // [2] scale to full size, mip threshold, radius in world space, mip blend; [3] temporal offset xy, static
    // fraction, projection [0][0]; [4] fade scale and offset, mip level factor.
    ao: array<vec4<f32>, 5>,
    // HZBRemapping: screen position to HZB UV.
    hzb_remap: vec4<f32>,
    // The upsample's half-size texel (zw), the setup's input texel (xy).
    texel: vec4<f32>,
    // AmbientOcclusionSetupParams: scale, threshold.
    setup: vec4<f32>,
    // The output's size (xy), the HZB build's UV factor and offset (into the depth).
    size: vec4<f32>,
    hzb_uv: vec4<f32>,
    // The HZB build: the input's texel (xy), its UV clamp (zw).
    hzb_in: vec4<f32>,
}

@group(0) @binding(0) var<uniform> p: Params;

fn lin_depth(d: f32) -> f32 {
    return d * p.devz.x + p.devz.y + 1.0 / (d * p.devz.z - p.devz.w);
}

// ---- HZB: each texel the farthest (smallest reversed device Z) of four.

#ifdef HZB_FIRST
@group(0) @binding(1) var depth: texture_depth_2d;

fn load_depth(uv: vec2<f32>) -> f32 {
    let size = vec2<i32>(textureDimensions(depth));
    let c = clamp(vec2<i32>(floor(min(uv, p.hzb_in.zw) * vec2<f32>(size))), vec2<i32>(0), size - 1);
    return textureLoad(depth, c, 0);
}

@fragment
fn hzb(in: FullscreenVertexOutput) -> @location(0) vec4<f32> {
    let uv = in.uv * p.hzb_uv.xy + p.hzb_uv.zw;
    let o = p.hzb_in.xy * 0.25;
    let a = min(load_depth(uv + vec2(-o.x, -o.y)), load_depth(uv + vec2(o.x, -o.y)));
    let b = min(load_depth(uv + vec2(-o.x, o.y)), load_depth(uv + vec2(o.x, o.y)));
    return vec4(min(b, a));
}
#endif

#ifdef HZB_MIP
@group(0) @binding(1) var src: texture_2d<f32>;

fn load_src(uv: vec2<f32>) -> f32 {
    let size = vec2<i32>(textureDimensions(src));
    let c = clamp(vec2<i32>(floor(uv * vec2<f32>(size))), vec2<i32>(0), size - 1);
    return textureLoad(src, c, 0).x;
}

@fragment
fn hzb(in: FullscreenVertexOutput) -> @location(0) vec4<f32> {
    let o = p.hzb_in.xy * 0.25;
    let a = min(load_src(in.uv + vec2(-o.x, -o.y)), load_src(in.uv + vec2(o.x, -o.y)));
    let b = min(load_src(in.uv + vec2(-o.x, o.y)), load_src(in.uv + vec2(o.x, o.y)));
    return vec4(min(b, a));
}
#endif

// ---- Setup: half size; the farthest of four depths, and their normals weighted by how near each is to it.

#ifdef SETUP
@group(0) @binding(1) var normals: texture_2d<f32>;
@group(0) @binding(2) var depth: texture_depth_2d;

fn tap(uv: vec2<f32>) -> vec2<i32> {
    let size = vec2<i32>(textureDimensions(depth));
    return clamp(vec2<i32>(floor(uv * vec2<f32>(size))), vec2<i32>(0), size - 1);
}

@fragment
fn setup(in: FullscreenVertexOutput) -> @location(0) vec4<f32> {
    let o = p.texel.xy * 0.5;
    let c = array<vec2<i32>, 4>(tap(in.uv + vec2(-o.x, -o.y)), tap(in.uv + vec2(o.x, -o.y)), tap(in.uv + vec2(-o.x, o.y)), tap(in.uv + vec2(o.x, o.y)));
    var d: array<f32, 4>;
    var far = 0.0;
    for (var i = 0; i < 4; i++) {
        d[i] = lin_depth(textureLoad(depth, c[i], 0));
        far = max(far, d[i]);
    }
    var sum = vec4(0.0001);
    for (var i = 0; i < 4; i++) {
        let w = saturate(1.0 - abs(d[i] - far) * p.setup.y);
        let n = normalize(textureLoad(normals, c[i], 0).xyz * 2.0 - 1.0);
        sum += vec4(n * 0.5 + 0.5, 1.0) * w;
    }
    return vec4(sum.xyz / sum.w, far * (1.0 / 131072.0));
}
#endif

// ---- Occlusion.

#ifdef OCCLUSION
@group(0) @binding(2) var hzb_tex: texture_2d<f32>;
@group(0) @binding(3) var hzb_sampler: sampler;
@group(0) @binding(4) var random_tex: texture_2d<f32>;

fn load_random(uv: vec2<f32>) -> vec2<f32> {
    // Point, wrapping.
    let size = vec2<i32>(textureDimensions(random_tex));
    let c = vec2<i32>(floor(uv * vec2<f32>(size))) % size;
    return textureLoad(random_tex, (c + size) % size, 0).xy;
}

// The mean of (1 - occlusion)^2 over the sample directions: each direction a mix of the two random vectors, sampled
// at half and full step on both sides of the pixel; a side's occlusion is the larger of its two steps'. `screen` is the
// pixel's screen position, `depth` its depth (cm), `n` its view normal.
fn occlusion(groups: ptr<function, array<vec4<f32>, 6>>, count: i32, inv: f32, screen: vec2<f32>, depth: f32, n: vec3<f32>, rnd: vec2<f32>) -> f32 {
    let a = mix(depth, 1.0, p.ao[2].z);
    let b = mix(depth, 1000.0, p.ao[2].z);
    let r = a * p.ao[1].z;
    let p00 = p.ao[3].w;
    let p11 = p.ao[1].w * p.ao[3].w;
    let bias = depth * p.ao[0].y * p.ao[2].x;
    let pos = vec3(screen * depth, depth) + bias * vec3(p00, p11, 1.0) * n;
    let iz = 1.0 / pos.z;
    let v1 = vec2(r * rnd.x * p00, r * rnd.y * p11) * iz;
    let v2 = vec2(-r * rnd.y * p00, r * rnd.x * p11) * iz;
    let s0 = pos.xy / pos.z;
    let thick = n * (b * 0.08);
    var sum = 0.0001;
    for (var g = 0; g < count; g++) {
        let k = (*groups)[g];
        let dir = k.x * v1 + k.y * v2;
        let lod = log2(p.ao[4].z * k.zw);
        for (var side = 0; side < 2; side++) {
            let s = select(-1.0, 1.0, side == 0);
            let m = max(sample_occlusion(s0 + s * 0.5 * dir, lod.x, pos, thick, p00, p11), sample_occlusion(s0 + s * dir, lod.y, pos, thick, p00, p11));
            sum += (1.0 - m) * (1.0 - m);
        }
    }
    return sum * inv;
}

// WedgeWithNormal: the sample's point in the HZB against the pixel's, along the normal.
fn sample_occlusion(q: vec2<f32>, lod: f32, pos: vec3<f32>, thick: vec3<f32>, p00: f32, p11: f32) -> f32 {
    let uv = q * p.hzb_remap.xy + p.hzb_remap.zw;
    let d = lin_depth(textureSampleLevel(hzb_tex, hzb_sampler, uv, lod).x);
    let delta = (vec3(q * d, d) - pos) * vec3(1.0 / p00, 1.0 / p11, 1.0);
    return saturate(dot(delta, thick) / dot(delta, delta));
}

// The 2x2 quad's smoothing: each pixel takes its horizontal and vertical neighbours' occlusion (through the fine
// derivatives) weighted by how alike their view normals are (dot^4).
fn quad_smooth(ao: f32, nxy: vec2<f32>, pixel: vec2<f32>) -> f32 {
    let own = vec3(ao, nxy);
    let sign = vec2<f32>((vec2<i32>(vec2<u32>(pixel) & vec2(1u)) << vec2(1u)) - vec2(1));
    let nx = own - dpdxFine(own) * sign.x;
    let ny = own - dpdyFine(own) * sign.y;
    let n = vec3(nxy, sqrt(1.0 - dot(nxy, nxy)));
    let cx = saturate(dot(n, vec3(nx.yz, sqrt(1.0 - dot(nx.yz, nx.yz)))));
    let cy = saturate(dot(n, vec3(ny.yz, sqrt(1.0 - dot(ny.yz, ny.yz)))));
    let wx = cx * cx * cx * cx;
    let wy = cy * cy * cy * cy;
    let norm = 1.0 / (1.0 + wx + wy);
    return norm * ao + wx * norm * nx.x + wy * norm * ny.x;
}

fn to_view(world: vec3<f32>) -> vec3<f32> {
    return normalize(world.x * p.to_view[0].xyz + world.y * p.to_view[1].xyz + world.z * p.to_view[2].xyz);
}
#endif

#ifdef HALF
@group(0) @binding(1) var setup_tex: texture_2d<f32>;

@fragment
fn occlusion_half(in: FullscreenVertexOutput) -> @location(0) vec4<f32> {
    var groups = array<vec4<f32>, 6>(
        vec4(0.325, 0.101, 0.125, 0.25),
        vec4(0.0, 0.2, 0.0416666679084301, 0.0833333358168602),
        vec4(0.272, -0.396, 0.2083333283662796, 0.4166666567325592),
        vec4(-0.385, -0.488, 0.2916666567325592, 0.5833333134651184),
        vec4(-0.711, 0.274, 0.375, 0.75),
        vec4(0.06, 0.9, 0.4583333432674408, 0.9166666865348816),
    );
    let size = vec2<i32>(textureDimensions(setup_tex));
    let c = clamp(vec2<i32>(in.position.xy), vec2<i32>(0), size - 1);
    let s = textureLoad(setup_tex, c, 0);
    let rnd = load_random(in.uv * p.ao[1].xy + p.ao[3].xy) * 2.0 - 1.0;
    let n = to_view(s.xyz * 2.0 - 1.0);
    let screen = vec2(in.uv.x * 2.0 - 1.0, 1.0 - in.uv.y * 2.0);
    let ao = occlusion(&groups, 6, 0.08333263546228409, screen, s.w * 131072.0, n, rnd);
    return vec4(quad_smooth(ao, n.xy, in.position.xy));
}
#endif

#ifdef FULL
@group(0) @binding(1) var normals: texture_2d<f32>;
@group(0) @binding(5) var depth: texture_depth_2d;
@group(0) @binding(6) var low_setup: texture_2d<f32>;
@group(0) @binding(7) var low_ao: texture_2d<f32>;

fn load_low(t: texture_2d<f32>, uv: vec2<f32>) -> vec4<f32> {
    let size = vec2<i32>(textureDimensions(t));
    return textureLoad(t, clamp(vec2<i32>(floor(uv * vec2<f32>(size))), vec2<i32>(0), size - 1), 0);
}

@fragment
fn occlusion_full(in: FullscreenVertexOutput) -> @location(0) vec4<f32> {
    var groups = array<vec4<f32>, 6>(
        vec4(0.406, 0.5698, 0.25, 0.5),
        vec4(0.0, -0.43, 0.0833333358168602, 0.1666666716337204),
        vec4(-0.58, 0.814, 0.4166666567325592, 0.8333333134651184),
        vec4(0.0), vec4(0.0), vec4(0.0),
    );
    let c = vec2<i32>(in.position.xy);
    let d = lin_depth(textureLoad(depth, c, 0));
    let world = textureLoad(normals, c, 0).xyz * 2.0 - 1.0;
    let n = to_view(world);
    let rnd = load_random(in.uv * p.ao[1].xy + p.ao[3].xy) * 2.0 - 1.0;
    let screen = vec2(in.uv.x * 2.0 - 1.0, 1.0 - in.uv.y * 2.0);
    let mean = occlusion(&groups, 3, 0.16666388511657715, screen, d, n, rnd);
    // The half-size level, upsampled by depth and (world) normal.
    var sw = 0.0001;
    var sa = 0.0001;
    let offsets = array<vec2<f32>, 4>(vec2(-0.5, 0.5), vec2(0.5, 0.5), vec2(-0.5, -0.5), vec2(0.5, -0.5));
    for (var i = 0; i < 4; i++) {
        let uv = in.uv + offsets[i] * p.texel.zw;
        let s = load_low(low_setup, uv);
        let wd = max(1.0 - abs(s.w * 131072.0 - d) * 0.003, 0.0);
        let w = saturate(dot(s.xyz * 2.0 - 1.0, world)) * wd;
        sw += w;
        sa += load_low(low_ao, uv).x * w;
    }
    let blended = mix(mean, sa / sw, p.ao[2].w);
    let fade = saturate(d * p.ao[4].x + p.ao[4].y);
    let x = mix(blended, 1.0, fade);
    let ao = 1.0 - (1.0 - exp2(log2(abs(x)) * p.ao[0].x)) * p.ao[0].w;
    return vec4(quad_smooth(ao, n.xy, in.position.xy));
}
#endif

// ---- Into Bevy's SSAO texture (its format: R16Float or R32Float).

#ifdef COPY
@group(0) @binding(0) var result: texture_2d<f32>;
#ifdef R32
@group(0) @binding(1) var target_tex: texture_storage_2d<r32float, write>;
#else
@group(0) @binding(1) var target_tex: texture_storage_2d<r16float, write>;
#endif

@compute @workgroup_size(8, 8, 1)
fn copy(@builtin(global_invocation_id) id: vec3<u32>) {
    let size = textureDimensions(target_tex);
    if any(id.xy >= size) {
        return;
    }
    textureStore(target_tex, id.xy, vec4(textureLoad(result, vec2<i32>(id.xy), 0).x));
}
#endif
