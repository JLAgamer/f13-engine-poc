// PP_Player_BloodEffect_01..06 (the kills' screen blood: their montages' SCPostProcessBlendableNotify windows): their
// compiled FPostProcessMaterialPS (one shader for all six; F13-decomp shaders/PP_Player_BloodEffect_06 s000), constants
// exact. After tonemapping (the materials' default BlendableLocation): it reads and writes the display-encoded colour.
// screen_effects.rs.
//
// b = 2 x min(max(1 - BloodTexture01.a ^ bloodContrast, 0), 0.4), BloodTexture01 at the screen's UV; the colour
// lerp(saturate(scene), BloodTexture01.rgb x (0.215, 0.14405, 0.14405), b) (+ SelectionColor: black).
#import bevy_core_pipeline::fullscreen_vertex_shader::FullscreenVertexOutput

struct Params {
    // x: bloodContrast (the notify's intensity hook).
    v: vec4<f32>,
}

@group(0) @binding(0) var scene: texture_2d<f32>;
@group(0) @binding(1) var scene_sampler: sampler;
@group(0) @binding(2) var<uniform> p: Params;
@group(0) @binding(3) var blood: texture_2d<f32>;
@group(0) @binding(4) var blood_sampler: sampler;

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

@fragment
fn fragment(in: FullscreenVertexOutput) -> @location(0) vec4<f32> {
    let t = textureSample(blood, blood_sampler, in.uv);
    let b = 2.0 * min(max(1.0 - pow(max(t.a, 0.0), p.v.x), 0.0), 0.4);
    let s = saturate(linear_to_srgb(max(textureSampleLevel(scene, scene_sampler, in.uv, 0.0).rgb, vec3<f32>(0.0))));
    let c = max(mix(s, t.rgb * vec3<f32>(0.215, 0.14405, 0.14405), b), vec3<f32>(0.0));
    return vec4<f32>(srgb_to_linear(c), 1.0);
}
