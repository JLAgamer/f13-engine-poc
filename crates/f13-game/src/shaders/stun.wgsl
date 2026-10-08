// PP_Player_StunEffect_01 (Jason_Base's StunScreenEffectMaterial): its compiled FPostProcessMaterialPS, constants
// exact. After tonemapping: it reads and writes the display-encoded colour. screen_effects.rs.
//
// Its uniforms (the material's uniform expression set, F13-decomp uniexpr.py), from DistortionDelta and AlphaDelta
// (his tick sets them) and the parameters' defaults (ImageBlurRadius 1, SphereMask_Radius 0.4, SphereMask_Hardness
// 0, SelectionColor black):
// - a box blur ImageBlurRadius x DistortionDelta pixels each way, kept off the middle by a sphere mask;
// - three tilings of Stun4 multiplied and tinted red, over the screen by BaseTexture1A's and BaseTexture1B's (both
//   Stun2) alpha, scaled by AlphaDelta.
// The view's: its time (cb1[127].z).
#import bevy_core_pipeline::fullscreen_vertex_shader::FullscreenVertexOutput

struct Params {
    // x: DistortionDelta, y: AlphaDelta, z: the view's time (s).
    v: vec4<f32>,
}

@group(0) @binding(0) var scene: texture_2d<f32>;
@group(0) @binding(1) var scene_sampler: sampler;
@group(0) @binding(2) var<uniform> p: Params;
// t0: Stun4; t1, t2: BaseTexture1A, BaseTexture1B (Stun2). Wrapped.
@group(0) @binding(3) var stun4: texture_2d<f32>;
@group(0) @binding(4) var stun2: texture_2d<f32>;
@group(0) @binding(5) var wrap_sampler: sampler;

const IMAGE_BLUR_RADIUS: f32 = 1.0;
const SPHERE_MASK_RADIUS: f32 = 0.4000000059604645;
const SPHERE_MASK_HARDNESS: f32 = 0.0;

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

fn scene_at(uv: vec2<f32>) -> vec3<f32> {
    return linear_to_srgb(max(textureSampleLevel(scene, scene_sampler, uv, 0.0).rgb, vec3<f32>(0.0)));
}

@fragment
fn fragment(in: FullscreenVertexOutput) -> @location(0) vec4<f32> {
    let distortion = p.v.x;
    let alpha = p.v.y;
    let time = p.v.z;
    let uv = in.uv;
    let inv = 1.0 / vec2<f32>(textureDimensions(scene));

    // The blur: the pixel itself, then 2R x 2R samples from -R pixels a pixel apart (R = cb2[4].w).
    let r = IMAGE_BLUR_RADIUS * (distortion * 1.0);
    let n = r + r;
    var sum = scene_at(uv);
    var count = 1.0;
    var ox = inv.x * -r;
    for (var i = 0.0; !(i >= n); i += 1.0) {
        var oy = inv.y * -r;
        for (var j = 0.0; !(j >= n); j += 1.0) {
            sum += scene_at(uv + vec2<f32>(ox, oy));
            oy += inv.y;
            count += 1.0;
        }
        ox += inv.x;
    }
    var col = sum / count;
    let sharp = scene_at(uv);
    var mask = (1.0 - length(uv - 0.5) * (1.0 / max(9.999999747378752e-06, SPHERE_MASK_RADIUS)))
        * (1.0 / max(1.0 - SPHERE_MASK_HARDNESS, 9.999999747378752e-06));
    mask = min(max(mask, 0.0), 0.5);
    col = (mask + mask) * (sharp - col) + col;

    // Stun4 three times, drifting with the time.
    let drift = time * vec2<f32>(-0.05000000074505806, 0.05000000074505806);
    var veins = textureSample(stun4, wrap_sampler, uv * 0.20000000298023224 + drift.y).rgb;
    veins *= textureSample(stun4, wrap_sampler, vec2<f32>(uv.x * 0.4000000059604645 + drift.y, uv.y * 0.4000000059604645 + drift.x)).rgb;
    veins *= textureSample(stun4, wrap_sampler, uv * 0.6000000238418579 + drift).rgb;

    // Its weight: BaseTexture1A's alpha x AlphaDelta x 0.5 plus BaseTexture1B's through two cheap contrasts.
    let a = alpha * 1.0;
    let m = 1.0 - clamp(a, 0.0, 0.5);
    let lo = 0.0 - m;
    let hi = m + 1.0;
    let base1a = textureSample(stun2, wrap_sampler, uv).a * a;
    var w = textureSample(stun2, wrap_sampler, uv).a * 0.5;
    w = clamp(w * (hi - lo) + lo, 0.0, 1.0);
    w = clamp(w * (hi - lo) + lo, 0.0, 1.0);
    w = w * clamp(a * 4.0, 0.0, 1.0);
    w = w * (1.149999976158142 - a);
    w = min(max(w, 0.0), 0.5);
    w = w * (1.0 - a / 2.0);
    w = base1a * 0.5 + w;
    w = min(max(w, 0.0), 0.5);
    w = w + w;
    let tinted = veins * vec3<f32>(0.8549929857254028, 0.030712999403476715, 0.017642000690102577);
    col = w * (tinted - col) + col;
    return vec4<f32>(srgb_to_linear(max(col, vec3<f32>(0.0))), 1.0);
}
