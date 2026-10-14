// M_UI_JasonHud_01 (BLEND_Additive in the UI domain: its colour times its opacity added to what's under it), from its
// compiled Slate pixel shader (F13-decomp shaders/M_UI_JasonHud_01 s000; uniexpr.py for its parameters):
// - smoke: three samples of UI_Smoke's red at the screen UV / Strech + Offset turned by time x 0.05 rad and moved by
//   time x (-0.1, 0.05), x 0.05 and x -0.05, multiplied; opacity x lerp(1, smoke x Brightness, Smoke);
// - grime: Grime's red at the screen UV; opacity x lerp(grime, 1, 1 - Grime);
// - opacity = saturate(that x the texture's alpha x the tint's alpha);
// - colour = opacity x max(tint x texture + SelectionColor (0), 0) x tint (Slate multiplies by the vertex colour again).
// Brightness only scales the smoke in the opacity. Slate draws the UI in gamma space and writes a UI material's output
// as it is (the engine's "UI materials look darker" behaviour): the value is sRGB-encoded and added to the encoded scene.
// Here the target blends in linear space, so the add is made against the scene under it (tonemap.rs SceneCopy, the
// tonemapped scene before the UI): decode(encode(scene) + value) - scene, added. (The UI drawn under it is left out.)
#import bevy_ui::ui_vertex_output::UiVertexOutput
#import bevy_render::{view::View, globals::Globals}

@group(0) @binding(0) var<uniform> view: View;
@group(0) @binding(1) var<uniform> globals: Globals;

@group(1) @binding(0) var<uniform> color: vec4<f32>;
// Brightness, Smoke, Grime.
@group(1) @binding(1) var<uniform> params: vec4<f32>;
// Strech X / Y, Offset X / Y.
@group(1) @binding(2) var<uniform> coords: vec4<f32>;
@group(1) @binding(3) var tex: texture_2d<f32>;
@group(1) @binding(4) var tex_sampler: sampler;
@group(1) @binding(5) var smoke_tex: texture_2d<f32>;
@group(1) @binding(6) var smoke_sampler: sampler;
@group(1) @binding(7) var grime_tex: texture_2d<f32>;
@group(1) @binding(8) var grime_sampler: sampler;
@group(1) @binding(9) var scene_tex: texture_2d<f32>;
@group(1) @binding(10) var scene_sampler: sampler;

fn linear_to_srgb(c: vec3<f32>) -> vec3<f32> {
    let lo = c * 12.92;
    let hi = 1.055 * pow(c, vec3<f32>(1.0 / 2.4)) - 0.055;
    return select(hi, lo, c <= vec3<f32>(0.0031308));
}

fn srgb_to_linear(c: vec3<f32>) -> vec3<f32> {
    let lo = c / 12.92;
    let hi = pow((c + 0.055) / 1.055, vec3<f32>(2.4));
    return select(hi, lo, c <= vec3<f32>(0.04045));
}

@fragment
fn fragment(in: UiVertexOutput) -> @location(0) vec4<f32> {
    let t = textureSample(tex, tex_sampler, in.uv);
    let screen = (in.position.xy - view.viewport.xy) / view.viewport.zw;
    let time = globals.time;
    let grime = textureSample(grime_tex, grime_sampler, screen).r;
    let p = screen / coords.xy + coords.zw;
    let a = time * 0.05;
    let s = sin(a);
    let c = cos(a);
    let q = vec2<f32>(c * p.x - s * p.y, s * p.x + c * p.y);
    let n = textureSample(smoke_tex, smoke_sampler, q + time * vec2<f32>(-0.1, 0.05)).r
        * textureSample(smoke_tex, smoke_sampler, q + time * 0.05).r
        * textureSample(smoke_tex, smoke_sampler, q - time * 0.05).r;
    let opacity = clamp(mix(1.0, n * params.x, params.y) * mix(grime, 1.0, 1.0 - params.z) * t.a * color.a, 0.0, 1.0);
    let rgb = opacity * max(color.rgb * t.rgb, vec3<f32>(0.0)) * color.rgb;
    let value = clamp(rgb, vec3<f32>(0.0), vec3<f32>(1.0));
    // Without the scene's copy (its 1 x 1 stand-in before the window's size is known): added in linear space.
    if textureDimensions(scene_tex).x <= 1u {
        return vec4<f32>(srgb_to_linear(value), opacity);
    }
    let under = clamp(textureSample(scene_tex, scene_sampler, screen).rgb, vec3<f32>(0.0), vec3<f32>(1.0));
    let sum = srgb_to_linear(clamp(linear_to_srgb(under) + value, vec3<f32>(0.0), vec3<f32>(1.0)));
    return vec4<f32>(max(sum - under, vec3<f32>(0.0)), opacity);
}
