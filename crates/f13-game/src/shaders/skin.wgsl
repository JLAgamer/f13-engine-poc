// Counselor skin, clipped under the clothes like the game's alpha mask compiler (MM_AlphaMaskCompiler_Skin), which
// bakes the skin diffuse with alpha = saturate(1 - FaceMask.r * AlphaMask.r) for the masked skin material.
// mask_a is the counselor's own mask (white: body below the neck), mask_b the outfit's SkinAlphaMask, which is white
// where skin stays visible (head, hands); the game must feed it to the compiler inverted (where is not found), as
// 1 - a * b with it as-is would hide the hands and keep the skin under the clothes. Inferred, checked on screen.
#import bevy_pbr::{
    pbr_fragment::pbr_input_from_standard_material,
    pbr_functions::{alpha_discard, apply_pbr_lighting, main_pass_post_lighting_processing},
    forward_io::{VertexOutput, FragmentOutput},
}

#ifdef SCREEN_SPACE_AMBIENT_OCCLUSION
#import bevy_pbr::mesh_view_bindings::screen_space_ambient_occlusion_texture
#endif

@group(#{MATERIAL_BIND_GROUP}) @binding(100) var mask_a: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(101) var mask_sampler: sampler;
@group(#{MATERIAL_BIND_GROUP}) @binding(102) var mask_b: texture_2d<f32>;

@fragment
fn fragment(in: VertexOutput, @builtin(front_facing) is_front: bool) -> FragmentOutput {
    var pbr_input = pbr_input_from_standard_material(in, is_front);
#ifdef VERTEX_UVS_A
    let a = textureSample(mask_a, mask_sampler, in.uv).r;
    let b = textureSample(mask_b, mask_sampler, in.uv).r;
    pbr_input.material.base_color.a = saturate(1.0 - a * (1.0 - b));
#endif
    pbr_input.material.base_color = alpha_discard(pbr_input.material, pbr_input.material.base_color);
#ifdef SCREEN_SPACE_AMBIENT_OCCLUSION
    // The ambient light's occlusion as the engine's ambient pass has it (FPostProcessAmbientPS): the screen-space AO
    // (ssao.rs) times the material's own, on its diffuse and specular alike.
    let ao = 1.0 * textureLoad(screen_space_ambient_occlusion_texture, vec2<i32>(in.position.xy), 0i).r;
    pbr_input.diffuse_occlusion = vec3(ao);
    pbr_input.specular_occlusion = ao;
#endif
    var out: FragmentOutput;
    out.color = apply_pbr_lighting(pbr_input);
    out.color = main_pass_post_lighting_processing(pbr_input, out.color);
    return out;
}
