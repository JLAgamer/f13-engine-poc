// A detached limb's gore cap (GoreCap: GoreCap_Meat_Layer under Blood_Layer, MatLayerBlend_Simple), from its compiled
// base pass (F13-decomp shaders/GoreCap s025; MODLOG "Dismemberment"). The StandardMaterial part gives the normal
// (the instance's Normal, gorecap_meat_normal) and two-sidedness; the vertex colour's red is the blood (Blood_Layer's
// alpha, COLOR0.x in the compiled shader).
// - The colour: gorecap_meat_D x gorecap_meat_AO, towards (0.04, 0.003936, 0) by the blood.
// - Roughness: 0.8 x (1 - AO.r), towards 0.2 by the blood. Specular: 0.3 - 0.1 x the blood. Metallic 0.
// - Emissive: SelectionColor.
#import bevy_pbr::{
    pbr_fragment::pbr_input_from_standard_material,
    pbr_functions::{apply_pbr_lighting, main_pass_post_lighting_processing},
    forward_io::{VertexOutput, FragmentOutput},
}

@group(#{MATERIAL_BIND_GROUP}) @binding(100) var diffuse: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(101) var tex_sampler: sampler;
@group(#{MATERIAL_BIND_GROUP}) @binding(102) var ao: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(103) var<uniform> selection: vec4<f32>;

@fragment
fn fragment(in: VertexOutput, @builtin(front_facing) is_front: bool) -> FragmentOutput {
    var pbr_input = pbr_input_from_standard_material(in, is_front);
#ifdef VERTEX_UVS_A
    var blood = 0.0;
#ifdef VERTEX_COLORS
    blood = saturate(in.color.r);
#endif
    let d = textureSample(diffuse, tex_sampler, in.uv).rgb;
    let a = textureSample(ao, tex_sampler, in.uv).rgb;
    pbr_input.material.base_color = vec4(saturate(mix(d * a, vec3(0.04, 0.003936, 0.0), blood)), 1.0);
    pbr_input.material.metallic = 0.0;
    pbr_input.material.perceptual_roughness = saturate(mix(0.8 * (1.0 - a.r), 0.2, blood));
    // Unreal's F0 is 0.08 x Specular, Bevy's 0.16 x reflectance^2.
    pbr_input.material.reflectance = vec3(sqrt(0.5 * saturate(0.3 - 0.1 * blood)));
    pbr_input.material.emissive = vec4(max(selection.rgb, vec3(0.0)), 1.0);
#endif
    var out: FragmentOutput;
    out.color = apply_pbr_lighting(pbr_input);
    out.color = main_pass_post_lighting_processing(pbr_input, out.color);
    return out;
}
