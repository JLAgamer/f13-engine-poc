// The counselor skin's depth, normal and shadow passes: the same clip under the clothes as its base pass (skin.wgsl),
// so the prepass doesn't write the clipped skin's depth over what shows through it (the hair), then Bevy's own prepass
// output (pbr_prepass.wgsl) without the meshlet paths.
#import bevy_pbr::{
    pbr_bindings,
    pbr_types,
    pbr_functions,
    prepass_io,
    mesh_view_bindings::view,
}

#ifdef BINDLESS
#import bevy_render::bindless::{bindless_samplers_filtering, bindless_textures_2d}
#import bevy_pbr::{mesh_bindings::mesh, pbr_bindings::material_indices}
#endif

@group(#{MATERIAL_BIND_GROUP}) @binding(100) var mask_a: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(101) var mask_sampler: sampler;
@group(#{MATERIAL_BIND_GROUP}) @binding(102) var mask_b: texture_2d<f32>;

fn clip(in: prepass_io::VertexOutput) {
#ifdef VERTEX_UVS_A
#ifdef BINDLESS
    let slot = mesh[in.instance_index].material_and_lightmap_bind_group_slot & 0xffffu;
    let cutoff = pbr_bindings::material_array[material_indices[slot].material].alpha_cutoff;
#else
    let cutoff = pbr_bindings::material.alpha_cutoff;
#endif
    let a = textureSample(mask_a, mask_sampler, in.uv).r;
    let b = textureSample(mask_b, mask_sampler, in.uv).r;
    if saturate(1.0 - a * (1.0 - b)) < cutoff {
        discard;
    }
#endif
}

#ifdef PREPASS_FRAGMENT
@fragment
fn fragment(in: prepass_io::VertexOutput, @builtin(front_facing) is_front: bool) -> prepass_io::FragmentOutput {
#ifdef BINDLESS
    let slot = mesh[in.instance_index].material_and_lightmap_bind_group_slot & 0xffffu;
    let flags = pbr_bindings::material_array[material_indices[slot].material].flags;
    let uv_transform = pbr_bindings::material_array[material_indices[slot].material].uv_transform;
#else
    let flags = pbr_bindings::material.flags;
    let uv_transform = pbr_bindings::material.uv_transform;
#endif
    clip(in);
    var out: prepass_io::FragmentOutput;
#ifdef UNCLIPPED_DEPTH_ORTHO_EMULATION
    out.frag_depth = in.unclipped_depth;
#endif
#ifdef NORMAL_PREPASS
    let double_sided = (flags & pbr_types::STANDARD_MATERIAL_FLAGS_DOUBLE_SIDED_BIT) != 0u;
    var normal = pbr_functions::prepare_world_normal(in.world_normal, double_sided, is_front);
#ifdef VERTEX_UVS_A
#ifdef VERTEX_TANGENTS
#ifdef STANDARD_MATERIAL_NORMAL_MAP
    let uv = (uv_transform * vec3(in.uv, 1.0)).xy;
    let Nt = textureSampleBias(
#ifdef BINDLESS
        bindless_textures_2d[material_indices[slot].normal_map_texture],
        bindless_samplers_filtering[material_indices[slot].normal_map_sampler],
#else
        pbr_bindings::normal_map_texture,
        pbr_bindings::normal_map_sampler,
#endif
        uv,
        view.mip_bias,
    ).rgb;
    let TBN = pbr_functions::calculate_tbn_mikktspace(normal, in.world_tangent);
    normal = pbr_functions::apply_normal_mapping(flags, TBN, double_sided, is_front, Nt);
#endif
#endif
#endif
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
    clip(in);
}
#endif
