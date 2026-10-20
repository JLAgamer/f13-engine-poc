// The engine's light shaft occlusion passes (UE 4.18, the global shader cache's; light_shafts.rs):
// TDownsampleLightShaftsPixelShader<Directional, true>, FBlurLightShaftsPixelShader, FFinishOcclusionPixelShader.

#import bevy_core_pipeline::fullscreen_vertex_shader::FullscreenVertexOutput

struct Params {
    // TextureSpaceBlurOrigin: the light's screen UV times aspect.zw (cb0[0].xy).
    origin: vec2<f32>,
    // The near plane (m), for the scene depth.
    near: f32,
    // 1 / OcclusionDepthRange (per cm) (LightShaftParameters.x, cb0[7].x).
    inv_range: f32,
    // UVMinMax (cb0[5]).
    uv_min_max: vec4<f32>,
    // AspectRatioAndInvAspectRatio (cb0[6]).
    aspect: vec4<f32>,
    // OcclusionMaskDarkness (LightShaftParameters.w, cb0[7].w).
    darkness: f32,
    // The blur pass's distance: min(exp2(pass x 2.263035) x FirstPassDistance, 1) (cb0[10]).
    step: f32,
    pad: vec2<f32>,
}

@group(0) @binding(0) var<uniform> p: Params;

#ifdef DOWNSAMPLE
@group(0) @binding(1) var depth: texture_depth_2d;

// The occlusion term: 1 where far (past OcclusionDepthRange) or at the view's edge, the scene depth's fraction of the
// range elsewhere.
@fragment
fn downsample(in: FullscreenVertexOutput) -> @location(0) vec4<f32> {
    let n = (in.uv - p.uv_min_max.xy) / p.uv_min_max.zw;
    var edge = 1.0 - n.x * (1.0 - n.x) * n.y * (1.0 - n.y) * 8.0;
    edge = edge * edge;
    edge = edge * edge;
    let size = vec2<i32>(textureDimensions(depth));
    let d = textureLoad(depth, clamp(vec2<i32>(floor(in.uv * vec2<f32>(size))), vec2<i32>(0), size - 1), 0);
    // SceneDepth (cm): reversed infinite Z.
    let scene_depth = p.near * 100.0 / max(d, 1e-30);
    return vec4(max(edge, saturate(scene_depth * p.inv_range)), 1.0, 1.0, 1.0);
}
#endif

#ifdef BLUR
@group(0) @binding(1) var src: texture_2d<f32>;
@group(0) @binding(2) var src_sampler: sampler;

// 12 samples from the pixel toward the light, `step` of the way, averaged.
@fragment
fn blur(in: FullscreenVertexOutput) -> @location(0) vec4<f32> {
    let at = in.uv * p.aspect.zw;
    let to_light = (p.origin - at) * p.step;
    var sum = vec3(0.0);
    for (var i = 0; i < 12; i += 1) {
        let uv = clamp((at + to_light * (f32(i) / 12.0)) * p.aspect.xy, p.uv_min_max.xy, p.uv_min_max.zw);
        sum += textureSample(src, src_sampler, uv).xyz;
    }
    return vec4(sum / 12.0, 1.0);
}
#endif

#ifdef FINISH
@group(0) @binding(1) var src: texture_2d<f32>;
@group(0) @binding(2) var src_sampler: sampler;

// OcclusionMaskDarkness where occluded, faded out with the distance from the light (DistanceFade: 0 for a
// directional light).
@fragment
fn finish(in: FullscreenVertexOutput) -> @location(0) vec4<f32> {
    let from_light = min(length(p.origin - in.uv * p.aspect.zw) * 0.2, 1.0);
    let o = textureSample(src, src_sampler, in.uv).x;
    let occlusion = mix(p.darkness, 1.0, o * o);
    return vec4(mix(occlusion, 1.0, from_light), 1.0, 1.0, 1.0);
}
#endif
