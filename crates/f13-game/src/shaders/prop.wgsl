// Props and foliage, from the compiled shaders. Base colour, normal map and alpha come from the StandardMaterial part.
// Mode 0, prop masters (RMA map): roughness = RMA.r * RoughnessMult, metallic = RMA.g * MetalnessMult, AO = RMA.b,
//   specular = Spec. M_Prop_Master_Lights also glows: emissive = Diffuse.rgb * Diffuse.a * Glow.
// Mode 1, foliage masters (RSS map): roughness = max(RSS.r * RoughnessMultiplier, RoughnessMin). (RSS.b scales the
//   subsurface colour, not ported.)
#import bevy_pbr::{
    pbr_fragment::pbr_input_from_standard_material,
    pbr_functions::{alpha_discard, apply_pbr_lighting, main_pass_post_lighting_processing},
    forward_io::{VertexOutput, FragmentOutput},
}

struct PropParams {
    // RoughnessMult, MetalnessMult, Spec, mode; foliage: RoughnessMultiplier, -, Spec, mode
    mults: vec4<f32>,
    // foliage: RoughnessMin; y: Glow (M_Prop_Master_Lights)
    extra: vec4<f32>,
}

@group(#{MATERIAL_BIND_GROUP}) @binding(100) var<uniform> params: PropParams;
@group(#{MATERIAL_BIND_GROUP}) @binding(101) var rma: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(102) var rma_sampler: sampler;

@fragment
fn fragment(in: VertexOutput, @builtin(front_facing) is_front: bool) -> FragmentOutput {
    var pbr_input = pbr_input_from_standard_material(in, is_front);
#ifdef VERTEX_UVS_A
    let m = textureSample(rma, rma_sampler, in.uv);
    if params.mults.w < 0.5 {
        pbr_input.material.perceptual_roughness = saturate(m.r * params.mults.x);
        pbr_input.material.metallic = saturate(m.g * params.mults.y);
        pbr_input.diffuse_occlusion = vec3(saturate(m.b));
    } else {
        pbr_input.material.perceptual_roughness = saturate(max(m.r * params.mults.x, params.extra.x));
        pbr_input.material.metallic = 0.0;
    }
#endif
    pbr_input.material.reflectance = vec3(sqrt(0.5 * saturate(params.mults.z)));
    if params.extra.y > 0.0 {
        let c = pbr_input.material.base_color;
        pbr_input.material.emissive = vec4(c.rgb * c.a * params.extra.y, 1.0);
    }
    pbr_input.material.base_color = alpha_discard(pbr_input.material, pbr_input.material.base_color);
    var out: FragmentOutput;
    out.color = apply_pbr_lighting(pbr_input);
    out.color = main_pass_post_lighting_processing(pbr_input, out.color);
    return out;
}
