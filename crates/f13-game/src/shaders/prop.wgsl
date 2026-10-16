// Props and foliage, from the compiled shaders. Base colour, normal map and alpha come from the StandardMaterial part.
// Mode 0, prop masters (RMA map): roughness = RMA.r * RoughnessMult, metallic = RMA.g * MetalnessMult, AO = RMA.b,
//   specular = Spec. M_Prop_Master_Lights also glows: emissive = Diffuse.rgb * Diffuse.a * Glow. The Chevette's
//   exterior lights up by its lights mask: emissive = Diffuse.rgb * dot(mask.rgb, (Headlights, Tailights, Backup)).
// Mode 1, foliage masters (RSS map): roughness = max(RSS.r * RoughnessMultiplier, RoughnessMin). (RSS.b scales the
//   subsurface colour, not ported.)
// Jason's pickaxe (extra.w 1; M_JasonV2_PickAxe's compiled base pass): its blood layer by b = WeaponBloodMask.r x
//   ShowWeaponBlood (extra.z): base colour saturate(lerp(Diffuse, (0.04, 0.003936, 0), b)), metallic and roughness
//   saturate(lerp(RMA, 0.2, b)); the mask in the lights' slot.
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

#ifdef BINDLESS
// (materials.rs PropExt: its index table's slots are bindings 100..105 in order.)
#import bevy_render::bindless::{bindless_samplers_filtering, bindless_textures_2d}
#import bevy_pbr::mesh_bindings::mesh
struct PropIndices {
    params: u32,
    rma: u32,
    rma_sampler: u32,
    lights_mask: u32,
    lights_sampler: u32,
}
@group(#{MATERIAL_BIND_GROUP}) @binding(105) var<storage> prop_params: array<PropParams>;
@group(#{MATERIAL_BIND_GROUP}) @binding(106) var<storage> prop_indices: array<PropIndices>;
#else
@group(#{MATERIAL_BIND_GROUP}) @binding(100) var<uniform> prop_uniform: PropParams;
@group(#{MATERIAL_BIND_GROUP}) @binding(101) var rma: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(102) var rma_sampler: sampler;
@group(#{MATERIAL_BIND_GROUP}) @binding(103) var lights_mask: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(104) var lights_sampler: sampler;
#endif

fn sample_rma(slot: u32, uv: vec2<f32>) -> vec4<f32> {
#ifdef BINDLESS
    return textureSample(bindless_textures_2d[prop_indices[slot].rma], bindless_samplers_filtering[prop_indices[slot].rma_sampler], uv);
#else
    return textureSample(rma, rma_sampler, uv);
#endif
}

fn sample_lights_mask(slot: u32, uv: vec2<f32>) -> vec4<f32> {
#ifdef BINDLESS
    return textureSample(bindless_textures_2d[prop_indices[slot].lights_mask], bindless_samplers_filtering[prop_indices[slot].lights_sampler], uv);
#else
    return textureSample(lights_mask, lights_sampler, uv);
#endif
}

@fragment
fn fragment(in: VertexOutput, @builtin(front_facing) is_front: bool) -> FragmentOutput {
#ifdef VISIBILITY_RANGE_DITHER
    // Its cull distance's crossfade (main.rs Placed: the placement's VisibilityRange), as Bevy's own shader does.
    bevy_pbr::pbr_functions::visibility_range_dither(in.position, in.visibility_range_dither);
#endif
    var pbr_input = pbr_input_from_standard_material(in, is_front);
#ifdef BINDLESS
    let slot = mesh[in.instance_index].material_and_lightmap_bind_group_slot & 0xffffu;
    let params = prop_params[prop_indices[slot].params];
#else
    let slot = 0u;
    let params = prop_uniform;
#endif
    var material_ao = 1.0;
#ifdef VERTEX_UVS_A
    let m = sample_rma(slot, in.uv);
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
    if params.extra.w > 0.5 {
        let blood = sample_lights_mask(slot, in.uv).r * params.extra.z;
        let c = pbr_input.material.base_color;
        pbr_input.material.base_color = vec4(saturate(mix(c.rgb, vec3(0.04, 0.003936, 0.0), blood)), c.a);
        pbr_input.material.metallic = saturate(mix(pbr_input.material.metallic, 0.2, blood));
        pbr_input.material.perceptual_roughness = saturate(mix(pbr_input.material.perceptual_roughness, 0.2, blood));
    }
    if params.lights.w > 0.0 {
        let mask = sample_lights_mask(slot, in.uv).rgb;
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
