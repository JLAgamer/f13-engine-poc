// Mesh particles drawn with an override material (particles.rs MeshParticleMaterial): M_Firecracker_Debris_01's base
// pass (its mesh-particle group's s049: masked, lit, two-sided). The particle's colour and dynamic parameters are folded
// into the StandardMaterial on the CPU (base colour, emissive, metallic, roughness, specular; base_color.a the opacity
// mask colour.a x OpacityMultiplier x DynamicParameter.x); here the mask's dither and clip: discard when mask +
// ((x + 2y) mod 5) / 6 - clip < 0, x and y the pixel's (the material function's screen pattern, its TemporalAA offset
// 0 without temporal AA here), the clip 0.8333.
#import bevy_pbr::{
    pbr_fragment::pbr_input_from_standard_material,
    pbr_functions::{apply_pbr_lighting, main_pass_post_lighting_processing},
    forward_io::{VertexOutput, FragmentOutput},
}

#ifdef SCREEN_SPACE_AMBIENT_OCCLUSION
#import bevy_pbr::mesh_view_bindings::screen_space_ambient_occlusion_texture
#endif

@group(#{MATERIAL_BIND_GROUP}) @binding(100) var<uniform> clip: vec4<f32>;

fn dither(p: vec2<f32>) -> f32 {
    let q = vec2<u32>(p);
    return f32((q.x + 2u * q.y) % 5u) / 6.0;
}

@fragment
fn fragment(in: VertexOutput, @builtin(front_facing) is_front: bool) -> FragmentOutput {
    var pbr_input = pbr_input_from_standard_material(in, is_front);
    if pbr_input.material.base_color.a + dither(in.position.xy) - clip.x < 0.0 {
        discard;
    }
    pbr_input.material.base_color.a = 1.0;
#ifdef SCREEN_SPACE_AMBIENT_OCCLUSION
    let ao = textureLoad(screen_space_ambient_occlusion_texture, vec2<i32>(in.position.xy), 0i).r;
    pbr_input.diffuse_occlusion = vec3(ao);
    pbr_input.specular_occlusion = ao;
#endif
    var out: FragmentOutput;
    out.color = apply_pbr_lighting(pbr_input);
    out.color = main_pass_post_lighting_processing(pbr_input, out.color);
    return out;
}
