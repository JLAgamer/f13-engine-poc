// Mesh particles' depth, normal and shadow passes: the same dithered clip as their base pass (mesh_particles.wgsl), then
// Bevy's own prepass output (pbr_prepass.wgsl) without the meshlet and normal-map paths (the material has no normal
// map: its normal is the mesh's).
#import bevy_pbr::{
    pbr_bindings,
    pbr_types,
    pbr_functions,
    prepass_io,
}

#ifdef BINDLESS
#import bevy_pbr::{mesh_bindings::mesh, pbr_bindings::material_indices}
#endif

@group(#{MATERIAL_BIND_GROUP}) @binding(100) var<uniform> clip: vec4<f32>;

fn dither(p: vec2<f32>) -> f32 {
    let q = vec2<u32>(p);
    return f32((q.x + 2u * q.y) % 5u) / 6.0;
}

fn mask(in: prepass_io::VertexOutput) {
#ifdef BINDLESS
    let slot = mesh[in.instance_index].material_and_lightmap_bind_group_slot & 0xffffu;
    let alpha = pbr_bindings::material_array[material_indices[slot].material].base_color.a;
#else
    let alpha = pbr_bindings::material.base_color.a;
#endif
    if alpha + dither(in.position.xy) - clip.x < 0.0 {
        discard;
    }
}

#ifdef PREPASS_FRAGMENT
@fragment
fn fragment(in: prepass_io::VertexOutput, @builtin(front_facing) is_front: bool) -> prepass_io::FragmentOutput {
#ifdef BINDLESS
    let slot = mesh[in.instance_index].material_and_lightmap_bind_group_slot & 0xffffu;
    let flags = pbr_bindings::material_array[material_indices[slot].material].flags;
#else
    let flags = pbr_bindings::material.flags;
#endif
    mask(in);
    var out: prepass_io::FragmentOutput;
#ifdef UNCLIPPED_DEPTH_ORTHO_EMULATION
    out.frag_depth = in.unclipped_depth;
#endif
#ifdef NORMAL_PREPASS
    let double_sided = (flags & pbr_types::STANDARD_MATERIAL_FLAGS_DOUBLE_SIDED_BIT) != 0u;
    let normal = pbr_functions::prepare_world_normal(in.world_normal, double_sided, is_front);
    out.normal = vec4(normal * 0.5 + vec3(0.5), 1.0);
#endif
#ifdef MOTION_VECTOR_PREPASS
    out.motion_vector = bevy_pbr::pbr_prepass_functions::calculate_motion_vector(in.world_position, in.previous_world_position);
#endif
    return out;
}
#else
@fragment
fn fragment(in: prepass_io::VertexOutput) {
    mask(in);
}
#endif
