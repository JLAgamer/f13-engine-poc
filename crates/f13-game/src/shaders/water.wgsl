// Lake water (M_Lake_Ripples_Translucent_*), first pass: the instance's water colour, roughness and specular, with
// two layers of its ripple normal map panning in world space at the instance's Small/Large Wave Scale and Time.
// Not ported yet: depth fade and edge foam, the reflection cubemap, far/near normal strength blending.
#import bevy_pbr::{
    pbr_fragment::pbr_input_from_standard_material,
    pbr_functions::{apply_pbr_lighting, main_pass_post_lighting_processing},
    mesh_view_bindings::globals,
    forward_io::{VertexOutput, FragmentOutput},
}

struct WaterParams {
    color: vec4<f32>,
    // Small Wave Scale, Large Wave Scale (cm per tile), Small Wave Time, Large Wave Time
    waves: vec4<f32>,
    // roughness, Unreal specular, normal strength
    surface: vec4<f32>,
}

@group(#{MATERIAL_BIND_GROUP}) @binding(100) var<uniform> params: WaterParams;
@group(#{MATERIAL_BIND_GROUP}) @binding(101) var ripples: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(102) var tex_sampler: sampler;

fn unpack_normal(t: vec4<f32>) -> vec2<f32> {
    return t.rg * 2.0 - 1.0;
}

@fragment
fn fragment(in: VertexOutput, @builtin(front_facing) is_front: bool) -> FragmentOutput {
    var pbr_input = pbr_input_from_standard_material(in, is_front);
    // World units are metres; the wave scales are in Unreal centimetres.
    let p = in.world_position.xz * 100.0;
    let t = globals.time;
    let small = unpack_normal(textureSample(ripples, tex_sampler, p / params.waves.x + vec2(t, 0.6 * t) * params.waves.z * 0.1));
    let large = unpack_normal(textureSample(ripples, tex_sampler, p / params.waves.y - vec2(0.7 * t, t) * params.waves.w));
    let xz = (small + large) * params.surface.z;
    // The plane faces up: tangent-space x/y map onto world x/z.
    pbr_input.N = normalize(vec3(xz.x, 1.0, xz.y));

    pbr_input.material.base_color = vec4(params.color.rgb, 1.0);
    pbr_input.material.perceptual_roughness = params.surface.x;
    pbr_input.material.metallic = 0.0;
    pbr_input.material.reflectance = vec3(sqrt(0.5 * params.surface.y));

    var out: FragmentOutput;
    out.color = apply_pbr_lighting(pbr_input);
    out.color = main_pass_post_lighting_processing(pbr_input, out.color);
    return out;
}
