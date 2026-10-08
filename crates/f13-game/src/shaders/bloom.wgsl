// The game's bloom passes (bloom.rs): FPostProcessDownsamplePS1, FPostProcessBloomSetupPS and TFilterPS<N, 0 / 1>,
// from its GlobalShaderCache.
#import bevy_core_pipeline::fullscreen_vertex_shader::FullscreenVertexOutput

struct Pass {
    // xy: the source's texel size.
    texel: vec4<f32>,
    // x: samples, y: 1 to add the second texture, z: BloomThreshold.
    params: vec4<f32>,
    offsets: array<vec4<f32>, 32>,
    weights: array<vec4<f32>, 32>,
}

@group(0) @binding(0) var source: texture_2d<f32>;
@group(0) @binding(1) var second: texture_2d<f32>;
@group(0) @binding(2) var bilinear: sampler;
@group(0) @binding(3) var<uniform> p: Pass;

// Four bilinear taps a texel out (a 4 x 4 box), averaged; negatives clamped.
@fragment
fn downsample(in: FullscreenVertexOutput) -> @location(0) vec4<f32> {
    let d = p.texel.xy;
    var c = textureSampleLevel(source, bilinear, in.uv - d, 0.0);
    c += textureSampleLevel(source, bilinear, in.uv + d * vec2<f32>(1.0, -1.0), 0.0);
    c += textureSampleLevel(source, bilinear, in.uv + d * vec2<f32>(-1.0, 1.0), 0.0);
    c += textureSampleLevel(source, bilinear, in.uv + d, 0.0);
    c *= 0.25;
    return vec4<f32>(max(c.rgb, vec3<f32>(0.0)), c.a);
}

// The colour weighted by how far its (already exposed) luminance is over the threshold.
@fragment
fn setup(in: FullscreenVertexOutput) -> @location(0) vec4<f32> {
    let c = min(textureSampleLevel(source, bilinear, in.uv, 0.0).rgb, vec3<f32>(65536.0));
    let luminance = dot(c, vec3<f32>(0.30000001192092896, 0.5899999737739563, 0.10999999940395355));
    let w = clamp((luminance - p.params.z) * 0.5, 0.0, 1.0);
    return vec4<f32>(c * w, 0.0);
}

// The weighted sum of the kernel's samples (weights carry the tint), plus the second texture when adding.
@fragment
fn weighted_sum(in: FullscreenVertexOutput) -> @location(0) vec4<f32> {
    var sum = vec4<f32>(0.0);
    let n = u32(p.params.x);
    for (var i = 0u; i < n; i++) {
        sum += textureSampleLevel(source, bilinear, in.uv + p.offsets[i].xy, 0.0) * p.weights[i];
    }
    if p.params.y > 0.5 {
        sum += textureSampleLevel(second, bilinear, in.uv, 0.0);
    }
    return sum;
}
