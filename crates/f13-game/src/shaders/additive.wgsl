// A UI material drawn additively (BLEND_Additive in the UI domain: its colour times its opacity added to what's under
// it), for UI materials whose graphs are cooked away: the brush's texture times its tint (vertex colour) and the
// material's Brightness. Slate draws the UI in gamma space and writes a UI material's output as it is (the engine's
// "UI materials look darker" behaviour), so the value is taken as an sRGB-encoded one and decoded before it's added
// here in linear space.
#import bevy_ui::ui_vertex_output::UiVertexOutput

@group(1) @binding(0) var<uniform> color: vec4<f32>;
@group(1) @binding(1) var tex: texture_2d<f32>;
@group(1) @binding(2) var tex_sampler: sampler;

fn srgb_to_linear(c: vec3<f32>) -> vec3<f32> {
    let lo = c / 12.92;
    let hi = pow((c + 0.055) / 1.055, vec3<f32>(2.4));
    return select(hi, lo, c <= vec3<f32>(0.04045));
}

@fragment
fn fragment(in: UiVertexOutput) -> @location(0) vec4<f32> {
    let t = textureSample(tex, tex_sampler, in.uv);
    let a = t.a * color.a;
    let c = srgb_to_linear(clamp(t.rgb * color.rgb * a, vec3<f32>(0.0), vec3<f32>(1.0)));
    return vec4<f32>(c, a);
}
