// Landscape layers (M_Terrain_CrystalLake), ported from the game's compiled per-component shaders. Every layer is
// a height blend: w = clamp(2 * weight + height - 1, 0.0001, 1) with height = the layer's diffuse alpha, then the
// weights are normalised. Layer textures tile over landscape coordinates (UV0, in quads) / MappingScale; weights
// come from the painted weight maps (UV1 over the whole landscape). Layers without an RMA map have roughness 1.
#import bevy_pbr::{
    pbr_fragment::pbr_input_from_standard_material,
    pbr_functions::{calculate_tbn_mikktspace, apply_pbr_lighting, main_pass_post_lighting_processing},
    forward_io::{VertexOutput, FragmentOutput},
}

#ifdef SCREEN_SPACE_AMBIENT_OCCLUSION
#import bevy_pbr::mesh_view_bindings::screen_space_ambient_occlusion_texture
#endif

struct TerrainParams {
    mapping_scale: f32,
    layer_count: u32,
    // Bit i: layer i has an RMA map.
    rma_mask: u32,
    _pad: u32,
    // The weight maps' UV from UV0 (scale, offset) when UV1 holds the Enlighten lightmap's; zero: UV1.
    weight_uv: vec4<f32>,
}

@group(#{MATERIAL_BIND_GROUP}) @binding(100) var<uniform> params: TerrainParams;
// Layer weights, 4 per texture (layers 0-3, 4-7).
@group(#{MATERIAL_BIND_GROUP}) @binding(101) var weights0: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(102) var weights1: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(103) var tex_sampler: sampler;
@group(#{MATERIAL_BIND_GROUP}) @binding(104) var d0: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(105) var d1: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(106) var d2: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(107) var d3: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(108) var d4: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(109) var d5: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(110) var n0: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(111) var n1: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(112) var n2: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(113) var n3: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(114) var n4: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(115) var n5: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(116) var r0: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(117) var r1: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(118) var r2: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(119) var r3: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(120) var r4: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(121) var r5: texture_2d<f32>;

fn unpack_normal(t: vec4<f32>) -> vec3<f32> {
    let xy = t.rg * 2.0 - 1.0;
    return vec3(xy, sqrt(max(1.0 - dot(xy, xy), 0.0)));
}

fn layer_weight(i: u32, painted: f32, height: f32) -> f32 {
    if i >= params.layer_count {
        return 0.0;
    }
    return clamp(2.0 * painted + height - 1.0, 0.0001, 1.0);
}

fn roughness(i: u32, rma: vec4<f32>) -> f32 {
    return select(1.0, rma.r, (params.rma_mask & (1u << i)) != 0u);
}

@fragment
fn fragment(in: VertexOutput, @builtin(front_facing) is_front: bool) -> FragmentOutput {
#ifdef VISIBILITY_RANGE_DITHER
    // Its cull distance's crossfade (main.rs Placed: the placement's VisibilityRange), as Bevy's own shader does.
    bevy_pbr::pbr_functions::visibility_range_dither(in.position, in.visibility_range_dither);
#endif
    var pbr_input = pbr_input_from_standard_material(in, is_front);

#ifdef VERTEX_UVS_A
    let uv = in.uv / params.mapping_scale;
#else
    let uv = vec2(0.0);
#endif
#ifdef VERTEX_UVS_B
    // UV1 runs over vertex positions; weight map texels sit on the vertices, so shift by half a texel.
#ifdef VERTEX_UVS_A
    let uv1 = select(in.uv_b, in.uv * params.weight_uv.xy + params.weight_uv.zw, any(params.weight_uv.xy != vec2(0.0)));
#else
    let uv1 = in.uv_b;
#endif
    let wuv = uv1 + 0.5 / vec2<f32>(textureDimensions(weights0));
#else
    let wuv = vec2(0.0);
#endif
    let p0 = textureSample(weights0, tex_sampler, wuv);
    let p1 = textureSample(weights1, tex_sampler, wuv);

    let c0 = textureSample(d0, tex_sampler, uv);
    let c1 = textureSample(d1, tex_sampler, uv);
    let c2 = textureSample(d2, tex_sampler, uv);
    let c3 = textureSample(d3, tex_sampler, uv);
    let c4 = textureSample(d4, tex_sampler, uv);
    let c5 = textureSample(d5, tex_sampler, uv);
    var w = array<f32, 6>(
        layer_weight(0u, p0.r, c0.a),
        layer_weight(1u, p0.g, c1.a),
        layer_weight(2u, p0.b, c2.a),
        layer_weight(3u, p0.a, c3.a),
        layer_weight(4u, p1.r, c4.a),
        layer_weight(5u, p1.g, c5.a),
    );
    let total = max(w[0] + w[1] + w[2] + w[3] + w[4] + w[5], 0.0001);
    for (var i = 0u; i < 6u; i++) {
        w[i] /= total;
    }

    let color = c0.rgb * w[0] + c1.rgb * w[1] + c2.rgb * w[2] + c3.rgb * w[3] + c4.rgb * w[4] + c5.rgb * w[5];
    pbr_input.material.base_color = vec4(saturate(color), 1.0);
    pbr_input.material.perceptual_roughness = saturate(
        roughness(0u, textureSample(r0, tex_sampler, uv)) * w[0]
        + roughness(1u, textureSample(r1, tex_sampler, uv)) * w[1]
        + roughness(2u, textureSample(r2, tex_sampler, uv)) * w[2]
        + roughness(3u, textureSample(r3, tex_sampler, uv)) * w[3]
        + roughness(4u, textureSample(r4, tex_sampler, uv)) * w[4]
        + roughness(5u, textureSample(r5, tex_sampler, uv)) * w[5]);
    pbr_input.material.metallic = 0.0;

#ifdef VERTEX_TANGENTS
    var nt = unpack_normal(textureSample(n0, tex_sampler, uv)) * w[0]
        + unpack_normal(textureSample(n1, tex_sampler, uv)) * w[1]
        + unpack_normal(textureSample(n2, tex_sampler, uv)) * w[2]
        + unpack_normal(textureSample(n3, tex_sampler, uv)) * w[3]
        + unpack_normal(textureSample(n4, tex_sampler, uv)) * w[4]
        + unpack_normal(textureSample(n5, tex_sampler, uv)) * w[5];
    nt = normalize(nt);
    nt.y = -nt.y; // DirectX-style (green down)
    let tbn = calculate_tbn_mikktspace(pbr_input.world_normal, in.world_tangent);
    pbr_input.N = normalize(nt.x * tbn[0] + nt.y * tbn[1] + nt.z * tbn[2]);
#endif

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
