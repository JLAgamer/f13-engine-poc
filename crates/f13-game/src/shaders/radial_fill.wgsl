// The stamina ring (StaminaBarMaterial; its graph is cooked away): the ring texture where the ring gradient (rising
// around the ring from its left end) is below the fill amount, in the widget's color.
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
    let g = textureSample(gradient, gradient_sampler, in.uv).r;
    let shown = select(0.0, 1.0, g < fill.x);
    return vec4<f32>(r.rgb * color.rgb, r.a * color.a * shown);
}
