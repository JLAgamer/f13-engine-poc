// The game's tonemap pass (FPostProcessTonemapPS4 and TPostProcessTonemapVS<true>, from its GlobalShaderCache;
// constants exact). tonemap.rs.
#import bevy_core_pipeline::fullscreen_vertex_shader::FullscreenVertexOutput

struct Params {
    color_scale0: vec4<f32>,
    // x: VignetteIntensity, y: the view's height / width.
    vignette: vec4<f32>,
    // xy: GrainRandomFull.
    grain: vec4<f32>,
    // ColorScale1: BloomIntensity.
    color_scale1: vec4<f32>,
    // x: Jason's colour grade's weight (tonemap.rs: Stalk's or Sense's); y, z: the colour fringe's scales for green
    // and red about the view's centre (1: none).
    grade: vec4<f32>,
}

@group(0) @binding(0) var scene: texture_2d<f32>;
@group(0) @binding(1) var scene_sampler: sampler;
@group(0) @binding(2) var lut: texture_3d<f32>;
@group(0) @binding(3) var lut_sampler: sampler;
@group(0) @binding(4) var<uniform> p: Params;
@group(0) @binding(5) var bloom: texture_2d<f32>;
@group(0) @binding(6) var grade_lut: texture_3d<f32>;

fn srgb_to_linear(c: vec3<f32>) -> vec3<f32> {
    let lo = c / 12.92;
    let hi = pow((c + 0.055) / 1.055, vec3<f32>(2.4));
    return select(hi, lo, c <= vec3<f32>(0.04045));
}

@fragment
fn fragment(in: FullscreenVertexOutput) -> @location(0) vec4<f32> {
    let uv = in.uv;
    // The vertex shader's vignette space: the screen position (y up) as a circle, sqrt(2) / sqrt(1 + (h / w)²).
    let hw = p.vignette.y;
    let ndc = vec2<f32>(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0);
    let vignette_space = ndc * vec2<f32>(1.0, hw) * (1.4142135381698608 / sqrt(hw * hw + 1.0));
    // The grain quantization's random: UV + GrainRandomFull.
    let seed = uv + p.grain.xy;
    let grain = fract(sin(seed.y * 543.3099975585938 + seed.x) * 493013.0);
    // Scene colour x ColorScale0 + bloom x ColorScale1 (the dirt mask's intensity is 0); Bevy has applied the exposure.
    let b = textureSample(bloom, scene_sampler, uv).rgb;
    // The colour fringe (the ColorFringe permutation): red and green from scaled positions, blue at the pixel.
    var s = textureSample(scene, scene_sampler, uv).rgb;
    s.r = textureSample(scene, scene_sampler, (uv - 0.5) * p.grade.z + 0.5).r;
    s.g = textureSample(scene, scene_sampler, (uv - 0.5) * p.grade.y + 0.5).g;
    var c = s * p.color_scale0.rgb + b * p.color_scale1.rgb;
    let v = vignette_space * p.vignette.x;
    let f = 1.0 / (dot(v, v) + 1.0);
    c = c * (f * f);
    let encoded = clamp(log2(c + 0.002667719265446067) * 0.0714285746216774 + 0.6107269525527954, vec3<f32>(0.0), vec3<f32>(1.0));
    let at = encoded * 0.96875 + 0.015625;
    let graded = mix(textureSample(lut, lut_sampler, at).rgb, textureSample(grade_lut, lut_sampler, at).rgb, p.grade.x);
    let out = graded * 1.0499999523162842 + (grain * 0.00390625 - 0.001953125);
    // Display-encoded sRGB; Bevy's output encodes again.
    return vec4<f32>(srgb_to_linear(max(out, vec3<f32>(0.0))), 1.0);
}
