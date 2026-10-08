// PP_Player_Jason_Shift_01 (Jason_Base's ShiftScreenEffect): its compiled FPostProcessMaterialPS, constants exact,
// its uniforms at the material's defaults (its uniform expression set, F13-decomp uniexpr.py: nothing sets the
// instance's parameters): BlurAmount 1, BlurRadius 0.5, RadiusExponent 2, AlphaDelta 0.6. After tonemapping: it reads
// and writes the display-encoded colour. screen_effects.rs.
//
// An 11-tap blur toward the screen's centre, kept off the middle by RadialGradientExponential (1 - exp(-((1 - d /
// BlurRadius) x RadiusExponent)²)), then darkened by BaseTexture1A's (T_Vignette_Simple_01) alpha x AlphaDelta. The
// BaseTexture1B term multiplies by -AlphaDelta and is clamped to 0 (cb1[8].z): it adds nothing.
#import bevy_core_pipeline::fullscreen_vertex_shader::FullscreenVertexOutput

@group(0) @binding(0) var scene: texture_2d<f32>;
@group(0) @binding(1) var scene_sampler: sampler;
@group(0) @binding(2) var vignette: texture_2d<f32>;
@group(0) @binding(3) var vignette_sampler: sampler;

const BLUR_AMOUNT: f32 = 1.0;
const BLUR_RADIUS: f32 = 0.5;
const RADIUS_EXPONENT: f32 = 2.0;
const ALPHA_DELTA: f32 = 0.6000000238418579;

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
    return linear_to_srgb(max(textureSample(scene, scene_sampler, uv).rgb, vec3<f32>(0.0)));
}

@fragment
fn fragment(in: FullscreenVertexOutput) -> @location(0) vec4<f32> {
    let uv = in.uv;
    let here = scene_at(uv);
    let dir = (vec2<f32>(0.5) - uv) * BLUR_AMOUNT;
    var sum = scene_at(uv - 0.07999999821186066 * dir) + here + scene_at(uv - 0.05000000074505806 * dir);
    sum += scene_at(uv - 0.029999999329447746 * dir) + scene_at(uv - 0.019999999552965164 * dir);
    sum += scene_at(uv - 0.009999999776482582 * dir) + scene_at(uv + 0.009999999776482582 * dir);
    sum += scene_at(uv + 0.019999999552965164 * dir) + scene_at(uv + 0.029999999329447746 * dir);
    sum += scene_at(uv + 0.05000000074505806 * dir) + scene_at(uv + 0.07999999821186066 * dir);
    let blurred = sum * 0.09090909361839294;
    let d = 1.0 - length(uv - vec2<f32>(0.5)) / BLUR_RADIUS;
    let e = d * RADIUS_EXPONENT;
    var mask = 1.0 - 1.0 / exp2(e * e * 1.442695140838623);
    if !(d >= 0.0 && abs(d) > 9.999999747378752e-06) {
        mask = 0.0;
    }
    var col = mask * (here - blurred) + blurred;
    let r = textureSample(vignette, vignette_sampler, uv).a * ALPHA_DELTA * 0.5;
    col = col + clamp(r + r, 0.0, 1.0) * (vec3<f32>(0.0) - col);
    col = col + clamp(r * 2.0 - 1.0, 0.0, 1.0) * (vec3<f32>(0.0, 0.0, 1.0) - col);
    return vec4<f32>(srgb_to_linear(max(col, vec3<f32>(0.0))), 1.0);
}
