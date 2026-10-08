// PP_Jason_Sense_Cabin_01 (Jason_Base's SenseCabinMaterial): its compiled FPostProcessMaterialPS, constants exact,
// its uniforms the material's defaults (its uniform expression set, F13-decomp uniexpr.py: nothing sets the
// instance's). After tonemapping: it reads and writes the display-encoded colour. sense_view.rs.
//
// Where the custom stencil holds Bit_Id (1: a sensed cabin's meshes, drawn through walls), its red channel's detail
// and three drifting smoke tilings, tinted F13_RED, less a far texture by the scene's depth. The view's: its size,
// forward axis (Unreal's, x forward z up), time, and the scene's depth (cm).
#import bevy_core_pipeline::fullscreen_vertex_shader::FullscreenVertexOutput

struct Params {
    // xyz: the view's forward axis (Unreal axes), w: the view's time (s).
    forward_time: vec4<f32>,
    // x: the projection's near plane (m).
    near: vec4<f32>,
}

@group(0) @binding(0) var scene: texture_2d<f32>;
@group(0) @binding(1) var scene_sampler: sampler;
@group(0) @binding(2) var<uniform> p: Params;
// The custom stencil (sense_view.rs's mask: green where the cabin's meshes are).
@group(0) @binding(3) var stencil: texture_2d<f32>;
// t0 (WispySmoke1B, sampled for 1B and 1D), t1 (1A), t2 (1C): all T_Wispy_Smoke_Tiled_01. Wrapped.
@group(0) @binding(4) var smoke: texture_2d<f32>;
@group(0) @binding(5) var wrap_sampler: sampler;
#ifdef MULTISAMPLED
@group(0) @binding(6) var depth: texture_depth_multisampled_2d;
#else
@group(0) @binding(6) var depth: texture_depth_2d;
#endif

const F13_RED: vec3<f32> = vec3<f32>(0.8549929857254028, 0.030712999403476715, 0.017642000690102577);
const FAR_TEXTURE_CONTRAST_LEVEL: f32 = 0.30000001192092896;
const DETAIL_CONTRAST: f32 = 0.019999999552965164;
const GLOW_CONTRAST_LEVEL: f32 = 1.2999999523162842;
const DEPTH_IMAGE_SIZE_NEAR: f32 = 0.0;
const DEPTH_CLAMP_MAX1: f32 = 1.0;
const DEPTH_CLAMP_MIN1: f32 = 0.10000000149011612;
const T1D_UV: f32 = 30.0;
const TIME_MASTER_MULTIPLIER1: f32 = 0.03999999910593033;
const TIME_MASTER_MULTIPLIER2: f32 = 4.999999873689376e-05;
const T1D_SINE_SCALE: f32 = 1.0;
const UV_MASTER_MULTIPLIER: f32 = 1.2000000476837158;
const BIT_ID: f32 = 1.0;
const PIXEL_DEPTH_DISTANCE2: f32 = 2.9999999242136255e-05;
const DETAIL_MULTIPLIER: f32 = 1.5;
const STENCIL_BRIGHTNESS: f32 = 0.20000000298023224;
const GLOW_CONTRAST_DIVIDE: f32 = 3.299999952316284;
const T1A_UV: f32 = 5.0;
const T1A_SINE_TIME: f32 = 1.0;
const T1A_SINE_SCALE: f32 = 0.30000001192092896;
const T1B_UV: f32 = 0.8500000238418579;
const T1B_SINE_TIME: f32 = 0.949999988079071;
const T1B_SINE_SCALE: f32 = 0.30000001192092896;
const T1C_UV: f32 = 0.75;
const T1C_SINE_TIME: f32 = -0.8500000238418579;
const T1C_SINE_SCALE: f32 = 0.30000001192092896;
const SMOKE_ADD_LEVEL: f32 = 0.5;
const SMOKE_MULTIPLY_LEVEL: f32 = 1.0;
const INNER_EDGE_GLOW_STRENGTH: f32 = 1.5;
const PIXEL_DEPTH_DISTANCE1: f32 = 0.0003000000142492354;

fn srgb_to_linear(c: vec3<f32>) -> vec3<f32> {
    let lo = c / 12.92;
    let hi = pow((c + 0.055) / 1.055, vec3<f32>(2.4));
    return select(hi, lo, c <= vec3<f32>(0.04045));
}

fn linear_to_srgb(c: vec3<f32>) -> vec3<f32> {
    let lo = c * 12.92;
    let hi = 1.055 * pow(c, vec3<f32>(1.0 / 2.4)) - 0.055;
    return select(hi, lo, c <= vec3<f32>(0.0031308));
}

// The shader's atan2 (its polynomial).
fn atan2_ue(y: f32, x: f32) -> f32 {
    let t = min(abs(x), abs(y)) * (1.0 / max(abs(x), abs(y)));
    let t2 = t * t;
    var poly = t2 * 0.02083509974181652 - 0.08513300120830536;
    poly = t2 * poly + 0.18014100193977356;
    poly = t2 * poly - 0.3302994966506958;
    poly = t2 * poly + 0.9998660087585449;
    var a = t * poly;
    if abs(x) < abs(y) {
        a = a + (t * poly * -2.0 + 1.5707963705062866);
    }
    if x < -x {
        a = a - 3.1415927410125732;
    }
    if min(x, y) < -min(x, y) && max(x, y) >= -max(x, y) {
        a = -a;
    }
    return a;
}

// The view-panned coordinate a tiling starts from: its side and up multipliers (x sign), the view's yaw (0..1) and
// forward z.
fn pan(uv: vec2<f32>, side: f32, up: f32, yaw: f32, forward_z: f32) -> vec2<f32> {
    return vec2<f32>(yaw * (side * 4.0 + 8.0) + uv.x * 2.0, uv.y - 2.0 * forward_z * up + 1.0);
}

fn tiling(uv: vec2<f32>, mult_offset: f32, sine_time: f32, sine_scale: f32, sign: f32, yaw: f32, forward_z: f32, slow_time: f32) -> vec3<f32> {
    let at = pan(uv, sign * (mult_offset + 1.0) * 0.75, sign * (mult_offset + 1.0) * 0.550000011920929, yaw, forward_z);
    let angle = slow_time * sine_time * 6.2831854820251465;
    let wobble = vec2<f32>(cos(angle), sin(angle)) * sine_scale + uv * UV_MASTER_MULTIPLIER * mult_offset;
    return textureSample(smoke, wrap_sampler, at * UV_MASTER_MULTIPLIER + wobble).rgb;
}

@fragment
fn fragment(in: FullscreenVertexOutput) -> @location(0) vec4<f32> {
    let uv = in.uv;
    let size = vec2<f32>(textureDimensions(scene));
    let px = vec2<i32>(floor(uv * size));
    let base = linear_to_srgb(max(textureSampleLevel(scene, scene_sampler, uv, 0.0).rgb, vec3<f32>(0.0)));
    let forward = p.forward_time.xyz;
    let time = p.forward_time.w;
    let yaw = fract(atan2_ue(forward.y, forward.x) * 0.1591549515724182);

    let device_z = textureLoad(depth, px, 0);
    let scene_depth = 100.0 * p.near.x / max(device_z, 1e-9);
    let near = clamp(scene_depth * DEPTH_IMAGE_SIZE_NEAR, DEPTH_CLAMP_MIN1, DEPTH_CLAMP_MAX1);
    let sign = near / abs(near);
    let brightness = near / STENCIL_BRIGHTNESS;

    // Three smoke tilings (1A on t1, 1B on t0, 1C on t2).
    let slow_time = time * TIME_MASTER_MULTIPLIER1;
    let a = tiling(uv, T1A_UV, T1A_SINE_TIME, T1A_SINE_SCALE, sign, yaw, forward.z, slow_time);
    let b = tiling(uv, T1B_UV, T1B_SINE_TIME, T1B_SINE_SCALE, sign, yaw, forward.z, slow_time);
    let c = tiling(uv, T1C_UV, T1C_SINE_TIME, T1C_SINE_SCALE, sign, yaw, forward.z, slow_time);
    var wisps = (a + b + c) * SMOKE_ADD_LEVEL + a * b * c * SMOKE_MULTIPLY_LEVEL;
    wisps = max(wisps, vec3<f32>(0.0));
    let edge = INNER_EDGE_GLOW_STRENGTH - min(wisps, vec3<f32>(0.5));
    wisps = min(wisps, vec3<f32>(1.0));

    // The custom stencil at the pixel.
    let s = select(0.0, BIT_ID, textureLoad(stencil, px, 0).g > 0.5);
    let matches = !(9.999999747378752e-06 < abs(s * 0.003921568859368563 - BIT_ID / 255.0));
    let hit = select(0.0, 1.0, matches);
    let miss = select(-1.0, -0.0, matches);
    var outer = select(-0.0, -1.0, matches) + brightness;
    let inner_base = hit * (1.0 - outer) / GLOW_CONTRAST_DIVIDE;
    let inner = clamp(inner_base * ((GLOW_CONTRAST_LEVEL + 1.0) + GLOW_CONTRAST_LEVEL) - GLOW_CONTRAST_LEVEL, 0.0, 1.0);
    outer = clamp(outer, 0.0, 1.0) + inner;
    let glow_x = outer * edge.x + miss;
    var glow = edge * outer;

    // Its red channel through CheapContrast (Detail_Contrast); 0 off it.
    let detail = clamp((base.r + miss) * (1.0 + 2.0 * DETAIL_CONTRAST) - DETAIL_CONTRAST, 0.0, 1.0);
    var weight = detail * DETAIL_MULTIPLIER;
    glow = detail * DETAIL_MULTIPLIER + glow;
    let near_fade = clamp(scene_depth * PIXEL_DEPTH_DISTANCE1, 0.0, 1.0);
    let far_fade = clamp(scene_depth * PIXEL_DEPTH_DISTANCE2, 0.0, 1.0) * hit;
    weight = glow_x * near_fade + weight;
    weight = wisps.x * near_fade + weight;
    let tinted_glow = wisps * near_fade + glow;

    // Texture1D far away, very slowly turning.
    let side_d = sign * (T1D_UV + 1.0) * 0.75;
    let up_d = sign * (T1D_UV + 1.0) * 0.550000011920929;
    let at_d = pan(uv, side_d, up_d, yaw, forward.z);
    let angle_d = time * TIME_MASTER_MULTIPLIER2 * slow_time * 6.2831854820251465;
    let wobble_d = vec2<f32>(cos(angle_d), sin(angle_d)) * T1D_SINE_SCALE + uv * UV_MASTER_MULTIPLIER * T1D_UV;
    let far_tex = textureSample(smoke, wrap_sampler, at_d + wobble_d).rgb;
    let far = clamp(far_fade * ((FAR_TEXTURE_CONTRAST_LEVEL + 1.0) + FAR_TEXTURE_CONTRAST_LEVEL) - FAR_TEXTURE_CONTRAST_LEVEL, 0.0, 1.0);
    let far_rgb = far * far_tex;
    let under = base - far_tex * far;
    let w = min(max(weight * hit - far_rgb.x, 0.0), 0.5) * 2.0;
    let over = F13_RED * tinted_glow - far_rgb;
    let col = under + w * (over - under);
    return vec4<f32>(srgb_to_linear(max(col, vec3<f32>(0.0))), 1.0);
}
