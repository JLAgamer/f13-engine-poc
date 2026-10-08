// PP_Jason_Sense_Counselor_01_MASKED (Jason_Base's SenseCounselorMaterial): its compiled FPostProcessMaterialPS,
// constants exact, its uniforms the material's defaults (its uniform expression set, F13-decomp uniexpr.py: nothing
// sets the instance's). After tonemapping: it reads and writes the display-encoded colour. sense_view.rs.
//
// Where the custom stencil holds Bit_Id (2: the counselor's meshes, drawn through walls), her red channel's detail and
// three drifting tilings of T_Wispy_Smoke_Tiled_01, tinted F13_RED, fading out with the scene's depth there. Both of
// its stencil blurs have radius 0 (Stencil_BlurRadius 0; Depth_StencilBlur 25 clamps it to 0), so only her pixels
// change. The view's: its size, forward axis (Unreal's, x forward z up), time, and the scene's depth (cm).
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
// The custom stencil (sense_view.rs's mask: 1 where her meshes are).
@group(0) @binding(3) var stencil: texture_2d<f32>;
// WispySmoke1A, 1B, 1C (all T_Wispy_Smoke_Tiled_01). Wrapped.
@group(0) @binding(4) var smoke: texture_2d<f32>;
@group(0) @binding(5) var wrap_sampler: sampler;
#ifdef MULTISAMPLED
@group(0) @binding(6) var depth: texture_depth_multisampled_2d;
#else
@group(0) @binding(6) var depth: texture_depth_2d;
#endif

const F13_RED: vec3<f32> = vec3<f32>(0.6583750247955322, 0.1274379938840866, 0.0886560007929802);
const DETAIL_CONTRAST: f32 = 0.019999999552965164;
const BIT_ID: f32 = 2.0;
const DETAIL_MULTIPLIER: f32 = 1.5;
const GLOW_CONTRAST_LEVEL: f32 = 0.0;
const DEPTH_IMAGE_SIZE: f32 = 9.999999747378752e-06;
const DEPTH_CLAMP_MAX: f32 = 1.0;
const DEPTH_CLAMP_MIN: f32 = 0.10000000149011612;
const GLOW_CONTRAST_DIVIDE: f32 = 1.0;
const SIDE_SIDE_MULT: f32 = 0.25;
const UP_DOWN_MULT: f32 = 0.25;
const UV_MASTER_MULTIPLIER: f32 = 1.2000000476837158;
const TIME_MASTER_MULTIPLIER: f32 = 0.03999999910593033;
const T1A_UV: f32 = 1.5;
const T1A_SINE_TIME: f32 = 1.0;
const T1A_SINE_SCALE: f32 = 0.30000001192092896;
const T1B_UV: f32 = 0.8500000238418579;
const T1B_SINE_TIME: f32 = 0.949999988079071;
const T1B_SINE_SCALE: f32 = 0.30000001192092896;
const T1C_UV: f32 = 0.75;
const T1C_SINE_TIME: f32 = -0.8500000238418579;
const T1C_SINE_SCALE: f32 = 0.30000001192092896;
const SMOKE_ADD_LEVEL: f32 = 1.0;
const SMOKE_MULTIPLY_LEVEL: f32 = 1.0;
const INNER_EDGE_GLOW_STRENGTH: f32 = 1.5;
const GLOW_CONTRAST_LEVEL2: f32 = 1.0;
const GLOW_CONTRAST_DIVIDE2: f32 = 1.7000000476837158;
const INNER_EDGE_GLOW_STRENGTH2: f32 = 1.0;
const INNER_GLOW_MULTIPLIER: f32 = 0.5;
const PIXEL_DEPTH_DISTANCE1: f32 = 2.0;
const FAR_STENCIL_CONTRAST: f32 = 0.5;
const PIXEL_DEPTH_DISTANCE2: f32 = 5.0;

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

// One of the three smoke tilings: its UV multiplier offset, sine time and sine scale, the view's yaw (0..1) and
// forward z, and the depth's sign (1).
fn smoke_at(uv: vec2<f32>, mult_offset: f32, sine_time: f32, sine_scale: f32, yaw: f32, forward_z: f32, sign: f32, time: f32) -> vec3<f32> {
    let side = sign * (mult_offset + 1.0) * SIDE_SIDE_MULT;
    let up = sign * (mult_offset + 1.0) * UP_DOWN_MULT;
    let pan = vec2<f32>(yaw * (side * 4.0 + 8.0) + uv.x * 2.0, uv.y - 2.0 * forward_z * up + 1.0);
    let angle = time * TIME_MASTER_MULTIPLIER * sine_time * 6.2831854820251465;
    let wobble = vec2<f32>(cos(angle), sin(angle)) * sine_scale + uv * UV_MASTER_MULTIPLIER * mult_offset;
    return textureSample(smoke, wrap_sampler, pan * UV_MASTER_MULTIPLIER + wobble).rgb;
}

@fragment
fn fragment(in: FullscreenVertexOutput) -> @location(0) vec4<f32> {
    let uv = in.uv;
    let size = vec2<f32>(textureDimensions(scene));
    let px = vec2<i32>(floor(uv * size));
    let base = linear_to_srgb(max(textureSampleLevel(scene, scene_sampler, uv, 0.0).rgb, vec3<f32>(0.0)));
    let forward = p.forward_time.xyz;
    let time = p.forward_time.w;

    // The custom stencil at the pixel: Bit_Id where her meshes are.
    let s = select(0.0, BIT_ID, textureLoad(stencil, px, 0).r > 0.5);
    let matches = !(9.999999747378752e-06 < abs(s * 0.003921568859368563 - BIT_ID / 255.0));
    let hit = select(0.0, 1.0, matches);
    let miss = select(-1.0, -0.0, matches);

    // Her red channel through CheapContrast (Detail_Contrast); 0 off her.
    let detail = clamp((base.r + miss) * (1.0 + 2.0 * DETAIL_CONTRAST) - DETAIL_CONTRAST, 0.0, 1.0);

    // SceneDepth (cm) x Depth_Image_Size.
    let device_z = textureLoad(depth, px, 0);
    let scene_depth = 100.0 * p.near.x / max(device_z, 1e-9);
    let d = scene_depth * DEPTH_IMAGE_SIZE;
    let sign = clamp(d, DEPTH_CLAMP_MIN, DEPTH_CLAMP_MAX) / abs(clamp(d, DEPTH_CLAMP_MIN, DEPTH_CLAMP_MAX));

    // The first stencil blur (radius 0: the stencil itself), its contrast and the glow outside her.
    let outer = 1.0 - clamp(s / GLOW_CONTRAST_DIVIDE * ((GLOW_CONTRAST_LEVEL + 1.0) + GLOW_CONTRAST_LEVEL) - GLOW_CONTRAST_LEVEL, 0.0, 1.0);

    let yaw = fract(atan2_ue(forward.y, forward.x) * 0.1591549515724182);
    let a = smoke_at(uv, T1A_UV, T1A_SINE_TIME, T1A_SINE_SCALE, yaw, forward.z, sign, time);
    let b = smoke_at(uv, T1B_UV, T1B_SINE_TIME, T1B_SINE_SCALE, yaw, forward.z, sign, time);
    let c = smoke_at(uv, T1C_UV, T1C_SINE_TIME, T1C_SINE_SCALE, yaw, forward.z, sign, time);
    let wisps = (a + b + c) * SMOKE_ADD_LEVEL + a * b * c * SMOKE_MULTIPLY_LEVEL;
    let edge = INNER_EDGE_GLOW_STRENGTH - min(max(wisps, vec3<f32>(0.0)), vec3<f32>(0.5));
    var glow = vec3<f32>(detail * DETAIL_MULTIPLIER) + outer * edge;

    // The second stencil blur (radius 0), its contrast, the inner glow, less the depth.
    var inner = clamp(s / GLOW_CONTRAST_DIVIDE2 * ((GLOW_CONTRAST_LEVEL2 + 1.0) + GLOW_CONTRAST_LEVEL2) - GLOW_CONTRAST_LEVEL2, 0.0, 1.0);
    inner = clamp(inner * INNER_EDGE_GLOW_STRENGTH2, 0.0, 1.0);
    inner = clamp(hit * inner * INNER_GLOW_MULTIPLIER, 0.0, 1.0);
    let smoky = clamp(inner * wisps, vec3<f32>(0.0), vec3<f32>(1.0));
    glow += clamp(smoky - d * PIXEL_DEPTH_DISTANCE1, vec3<f32>(0.0), vec3<f32>(1.0));

    // Far away she fades (0_PixelDepthDistance2, 0_FarStencilContrast).
    let far = clamp(d * PIXEL_DEPTH_DISTANCE2 * hit * ((FAR_STENCIL_CONTRAST + 1.0) + FAR_STENCIL_CONTRAST) - FAR_STENCIL_CONTRAST, 0.0, 1.0);
    let tinted = F13_RED * glow - far;
    let w = min(max(glow.x * hit - far, 0.0), 0.5) * 2.0;
    let col = base + w * (tinted - base);
    return vec4<f32>(srgb_to_linear(max(col, vec3<f32>(0.0))), 1.0);
}
