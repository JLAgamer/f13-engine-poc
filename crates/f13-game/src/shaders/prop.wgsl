// Props and foliage, from the compiled shaders. Base colour, normal map and alpha come from the StandardMaterial part.
// Mode 0, prop masters (RMA map): roughness = RMA.r * RoughnessMult, metallic = RMA.g * MetalnessMult, AO = RMA.b,
//   specular = Spec. M_Prop_Master_Lights also glows: emissive = Diffuse.rgb * Diffuse.a * Glow. The Chevette's
//   exterior lights up by its lights mask: emissive = Diffuse.rgb * dot(mask.rgb, (Headlights, Tailights, Backup)).
// Mode 1, foliage masters (RSS map): roughness = max(RSS.r * RoughnessMultiplier, RoughnessMin). (RSS.b scales the
//   subsurface colour, not ported.)
#import bevy_pbr::{
    pbr_fragment::pbr_input_from_standard_material,
    pbr_functions::{alpha_discard, apply_pbr_lighting, main_pass_post_lighting_processing},
    forward_io::{VertexOutput, FragmentOutput},
}

#ifdef SCREEN_SPACE_AMBIENT_OCCLUSION
#import bevy_pbr::mesh_view_bindings::screen_space_ambient_occlusion_texture
#endif

struct PropParams {
    // RoughnessMult, MetalnessMult, Spec, mode; foliage: RoughnessMultiplier, -, Spec, mode
    mults: vec4<f32>,
    // foliage: RoughnessMin; y: Glow (M_Prop_Master_Lights)
    extra: vec4<f32>,
    // The Chevette: Headlights, Tailights, Backup Lights; w: 1 with its lights mask
    lights: vec4<f32>,
}

@group(#{MATERIAL_BIND_GROUP}) @binding(100) var<uniform> params: PropParams;
@group(#{MATERIAL_BIND_GROUP}) @binding(101) var rma: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(102) var rma_sampler: sampler;
@group(#{MATERIAL_BIND_GROUP}) @binding(103) var lights_mask: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(104) var lights_sampler: sampler;

@fragment
fn fragment(in: VertexOutput, @builtin(front_facing) is_front: bool) -> FragmentOutput {
#ifdef VISIBILITY_RANGE_DITHER
    // Its cull distance's crossfade (main.rs Placed: the placement's VisibilityRange), as Bevy's own shader does.
    bevy_pbr::pbr_functions::visibility_range_dither(in.position, in.visibility_range_dither);
#endif
    var pbr_input = pbr_input_from_standard_material(in, is_front);
    var material_ao = 1.0;
#ifdef VERTEX_UVS_A
    let m = textureSample(rma, rma_sampler, in.uv);
    if params.mults.w < 0.5 {
        pbr_input.material.perceptual_roughness = saturate(m.r * params.mults.x);
        pbr_input.material.metallic = saturate(m.g * params.mults.y);
        material_ao = saturate(m.b);
        pbr_input.diffuse_occlusion = vec3(material_ao);
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
#ifdef VERTEX_UVS_A
    if params.lights.w > 0.0 {
        let mask = textureSample(lights_mask, lights_sampler, in.uv).rgb;
        pbr_input.material.emissive = vec4(pbr_input.material.base_color.rgb * dot(mask, params.lights.xyz), 1.0);
    }
#endif
#ifdef SCREEN_SPACE_AMBIENT_OCCLUSION
    // The ambient light's occlusion as the engine's ambient pass has it (FPostProcessAmbientPS): the screen-space AO
    // (ssao.rs) times the material's own, on its diffuse and specular alike.
    let ao = material_ao * textureLoad(screen_space_ambient_occlusion_texture, vec2<i32>(in.position.xy), 0i).r;
    pbr_input.diffuse_occlusion = vec3(ao);
    pbr_input.specular_occlusion = ao;
#endif
    pbr_input.material.base_color = alpha_discard(pbr_input.material, pbr_input.material.base_color);
    var out: FragmentOutput;
    out.color = apply_pbr_lighting(pbr_input);
    out.color = main_pass_post_lighting_processing(pbr_input, out.color);
    return out;
}
