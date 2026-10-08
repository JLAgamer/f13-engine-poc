// The game's Gaussian depth of field (dof.rs), far blur only: FPostProcessDOFSetupPS<1, 0> (PS10) and
// FPostProcessDOFRecombinePS<1, 0, 0> (PS100), from its GlobalShaderCache; the blur between is bloom.wgsl's weighted
// sum. Constants exact.
#import bevy_core_pipeline::fullscreen_vertex_shader::FullscreenVertexOutput

struct Params {
    world_from_clip: mat4x4<f32>,
    // xyz: the camera's position; w: the projection's near plane (m).
    camera: vec4<f32>,
    // x: DepthOfFieldFocalDistance + DepthOfFieldFocalRegion (cm), y: max(DepthOfFieldFarTransitionRegion, 0.01),
    // z: the sky's distance (DepthOfFieldSkyFocusDistance, else 1e8).
    focus: vec4<f32>,
}

@group(0) @binding(0) var scene: texture_2d<f32>;
@group(0) @binding(1) var far_blur: texture_2d<f32>;
@group(0) @binding(2) var bilinear: sampler;
@group(0) @binding(3) var<uniform> p: Params;
#ifdef MULTISAMPLED
@group(0) @binding(4) var depth: texture_depth_multisampled_2d;
#else
@group(0) @binding(4) var depth: texture_depth_2d;
#endif

// SceneDepth (cm) at a full-size pixel.
fn scene_depth(px: vec2<i32>) -> f32 {
    let size = vec2<i32>(textureDimensions(depth));
    let d = textureLoad(depth, clamp(px, vec2<i32>(0), size - 1), 0);
    return 100.0 * p.camera.w / d;
}

// How far beyond the focus a depth is (0..1).
fn far_weight(d: f32) -> f32 {
    return clamp((d - p.focus.x) / p.focus.y, 0.0, 1.0);
}

// Half size: the four full-size pixels' colours weighted by how far out of focus each is, the weights in alpha.
@fragment
fn setup(in: FullscreenVertexOutput) -> @location(0) vec4<f32> {
    let half_size = vec2<f32>(textureDimensions(scene) + 1u) / 2.0;
    let base = vec2<i32>(floor(in.uv * floor(half_size))) * 2;
    let size = vec2<i32>(textureDimensions(scene));
    // The view ray's up (Unreal's z) for the sky: less blur looking up.
    let ndc = vec2<f32>(in.uv.x * 2.0 - 1.0, 1.0 - in.uv.y * 2.0);
    let near = p.world_from_clip * vec4<f32>(ndc, 1.0, 1.0);
    let ray = normalize(near.xyz / near.w - p.camera.xyz);
    let up = clamp(ray.y * 3.0, 0.0, 1.0);
    var sum = vec4<f32>(0.0);
    for (var i = 0; i < 4; i++) {
        let px = base + vec2<i32>(i & 1, i >> 1);
        let d = scene_depth(px);
        var w = far_weight(d);
        if p.focus.z < d {
            w = up * -w + w;
        }
        let c = textureLoad(scene, clamp(px, vec2<i32>(0), size - 1), 0).rgb;
        sum += vec4<f32>(c, 1.0) * w;
    }
    return sum * 0.25;
}

// Full size: the far blur (its weights divided out) over the scene by how out of focus each pixel is.
@fragment
fn recombine(in: FullscreenVertexOutput) -> @location(0) vec4<f32> {
    let size = vec2<f32>(textureDimensions(scene));
    let px = vec2<i32>(floor(in.uv * size));
    let sharp = textureLoad(scene, px, 0).rgb;
    let focus = pow(max((0.25 - far_weight(scene_depth(px))) * 4.0, 0.0), 2.0);
    let blur_size = vec2<f32>(textureDimensions(far_blur));
    let at = clamp(in.uv, 0.5 / blur_size, 1.0 - 0.5 / blur_size);
    let f = textureSampleLevel(far_blur, bilinear, at, 0.0);
    let far = (f.rgb + sharp * 0.0001) / (f.a + 0.0001);
    let s = sqrt(f.a);
    let amount = s * (3.0 - 2.0 * sqrt(s));
    let mixed = amount * (far - sharp) + sharp;
    // (The game writes alpha 0; nothing reads it.)
    return vec4<f32>(focus * (sharp - mixed) + mixed, 1.0);
}
