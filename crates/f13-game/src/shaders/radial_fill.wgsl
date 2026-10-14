// A ring filled part way, for UI materials whose graphs are cooked away: the ring texture where the gradient is in
// [fill.y, fill.x), in the widget's color. The stamina ring (StaminaBarMaterial) uses its gradient texture (rising
// around the ring from its left end); with fill.z set the gradient is the angle clockwise from 12 o'clock (the
// interaction dial's ProgressCircle and its pips). fill.z 2: HoldInteractMaterial (its compiled shader): shown where
// frac(atan2(0.5 - v, 0.5 - u) / 2 pi) + Alpha (fill.x) >= 1, the colour the texture's x its alpha x the tint.
#import bevy_ui::ui_vertex_output::UiVertexOutput

@group(1) @binding(0) var<uniform> color: vec4<f32>;
@group(1) @binding(1) var<uniform> fill: vec4<f32>;
@group(1) @binding(2) var ring: texture_2d<f32>;
@group(1) @binding(3) var ring_sampler: sampler;
@group(1) @binding(4) var gradient: texture_2d<f32>;
@group(1) @binding(5) var gradient_sampler: sampler;

@fragment
fn fragment(in: UiVertexOutput) -> @location(0) vec4<f32> {
    let r = textureSample(ring, ring_sampler, in.uv);
    if fill.z > 1.5 {
        let a = fract(atan2(0.5 - in.uv.y, 0.5 - in.uv.x) / 6.2831853);
        let mask = floor(a + fill.x);
        return vec4<f32>(r.rgb * r.a * color.rgb, clamp(mask * r.a * color.a, 0.0, 1.0));
    }
    let d = in.uv - vec2<f32>(0.5, 0.5);
    let angle = fract(atan2(d.x, -d.y) / 6.2831853 + 1.0);
    let g = select(textureSample(gradient, gradient_sampler, in.uv).r, angle, fill.z > 0.5);
    let shown = select(0.0, 1.0, g >= fill.y && g < fill.x);
    return vec4<f32>(r.rgb * color.rgb, r.a * color.a * shown);
}
