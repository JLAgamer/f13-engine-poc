// The game's vertex-painted blend masters, ported from their compiled SM5 base pass shaders. Mode (params.blend.z):
// 0 Master_Blend_01(_POM): two sets A and B, t = saturate((2 * vertex_alpha - (1 - B.a + blend_map)) / sharpness)
//   (sharpness 0: a hard step); RMAS = roughness (clamped to [min, max] per set), metallic, AO, specular (A only).
// 1 Master_Tint_Blend_01: t = saturate((2 * vertex_alpha - blend_map) / sharpness); each set's colour c becomes
//   c * (1 - a) + a^2 * Tint (a = its alpha), then lerp(c, luminance, Desaturation); Specular A/B parameters.
// 2 Master_Tint_Blend_Curtains_01: set B only, tinted and desaturated as above, then lerp(DirtColor, c, vertex_alpha).
// Normals add a detail normal (own tiling, scaled by Detail Strength). Everything else is lerp(A, B, t).
// Master_Blend_01_POM (params.pom.x > 0): the colour and normal maps at a parallax-occluded UV (UE's
// ParallaxOcclusionMapping as its compiled base pass runs it: [`pom_uv`]); both RMAS maps at TilingRate A.
#import bevy_pbr::{
    pbr_fragment::pbr_input_from_standard_material,
    pbr_functions::{alpha_discard, calculate_tbn_mikktspace, apply_pbr_lighting, main_pass_post_lighting_processing},
    pbr_types::STANDARD_MATERIAL_FLAGS_DOUBLE_SIDED_BIT,
    forward_io::{VertexOutput, FragmentOutput},
}

#ifdef SCREEN_SPACE_AMBIENT_OCCLUSION
#import bevy_pbr::mesh_view_bindings::screen_space_ambient_occlusion_texture
#endif

struct BlendParams {
    // TilingRate A, Detail Tiling A, TilingRate B, Detail Tiling B
    tiling: vec4<f32>,
    // Blend TilingRate, Blend Sharpness, mode
    blend: vec4<f32>,
    // Roughness Min A, Max A, Min B, Max B
    roughness: vec4<f32>,
    detail_strength_a: vec4<f32>,
    detail_strength_b: vec4<f32>,
    tint_a: vec4<f32>,
    tint_b: vec4<f32>,
    // Desaturation A, B, Specular A, B
    tint: vec4<f32>,
    dirt: vec4<f32>,
    // Height of Texture, -(1 - ref plane), MinSteps, MaxSteps
    pom: vec4<f32>,
    // Tiling
    pom_tiling: vec4<f32>,
    channel: vec4<f32>,
}

#ifdef BINDLESS
// (materials.rs BlendExt: its index table's slots are bindings 100..112 in order.)
#import bevy_render::bindless::{bindless_samplers_filtering, bindless_textures_2d}
#import bevy_pbr::mesh_bindings::mesh
struct BlendIndices {
    params: u32,
    color_a: u32,
    tex_sampler: u32,
    color_b: u32,
    normal_a: u32,
    normal_b: u32,
    detail_a: u32,
    detail_b: u32,
    rmas_a: u32,
    rmas_b: u32,
    blend_map: u32,
    height_map: u32,
}
@group(#{MATERIAL_BIND_GROUP}) @binding(112) var<storage> blend_params: array<BlendParams>;
@group(#{MATERIAL_BIND_GROUP}) @binding(113) var<storage> blend_indices: array<BlendIndices>;
#else
@group(#{MATERIAL_BIND_GROUP}) @binding(100) var<uniform> blend_uniform: BlendParams;
@group(#{MATERIAL_BIND_GROUP}) @binding(101) var color_a: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(102) var tex_sampler: sampler;
@group(#{MATERIAL_BIND_GROUP}) @binding(103) var color_b: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(104) var normal_a: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(105) var normal_b: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(106) var detail_a: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(107) var detail_b: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(108) var rmas_a: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(109) var rmas_b: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(110) var blend_map: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(111) var height_map: texture_2d<f32>;
#endif

// This fragment's material (set as the fragment starts) and, bindless, its slot.
var<private> params: BlendParams;
var<private> slot: u32;

// The textures by number: color_a, color_b, normal_a, normal_b, detail_a, detail_b, rmas_a, rmas_b, blend_map, height_map.
const T_COLOR_A: u32 = 0u;
const T_COLOR_B: u32 = 1u;
const T_NORMAL_A: u32 = 2u;
const T_NORMAL_B: u32 = 3u;
const T_DETAIL_A: u32 = 4u;
const T_DETAIL_B: u32 = 5u;
const T_RMAS_A: u32 = 6u;
const T_RMAS_B: u32 = 7u;
const T_BLEND_MAP: u32 = 8u;
const T_HEIGHT_MAP: u32 = 9u;
fn sample_t(which: u32, uv: vec2<f32>) -> vec4<f32> {
#ifdef BINDLESS
    var t: u32;
    switch which {
        case 0u: { t = blend_indices[slot].color_a; }
        case 1u: { t = blend_indices[slot].color_b; }
        case 2u: { t = blend_indices[slot].normal_a; }
        case 3u: { t = blend_indices[slot].normal_b; }
        case 4u: { t = blend_indices[slot].detail_a; }
        case 5u: { t = blend_indices[slot].detail_b; }
        case 6u: { t = blend_indices[slot].rmas_a; }
        case 7u: { t = blend_indices[slot].rmas_b; }
        case 8u: { t = blend_indices[slot].blend_map; }
        default: { t = blend_indices[slot].height_map; }
    }
    return textureSample(bindless_textures_2d[t], bindless_samplers_filtering[blend_indices[slot].tex_sampler], uv);
#else
    switch which {
        case 0u: { return sample_t(T_COLOR_A, uv); }
        case 1u: { return sample_t(T_COLOR_B, uv); }
        case 2u: { return sample_t(T_NORMAL_A, uv); }
        case 3u: { return sample_t(T_NORMAL_B, uv); }
        case 4u: { return sample_t(T_DETAIL_A, uv); }
        case 5u: { return sample_t(T_DETAIL_B, uv); }
        case 6u: { return sample_t(T_RMAS_A, uv); }
        case 7u: { return sample_t(T_RMAS_B, uv); }
        case 8u: { return sample_t(T_BLEND_MAP, uv); }
        default: { return sample_t(T_HEIGHT_MAP, uv); }
    }
#endif
}

fn sample_level(which: u32, uv: vec2<f32>, lod: f32) -> vec4<f32> {
#ifdef BINDLESS
    return textureSampleLevel(bindless_textures_2d[blend_indices[slot].height_map], bindless_samplers_filtering[blend_indices[slot].tex_sampler], uv, lod);
#else
    return textureSampleLevel(height_map, tex_sampler, uv, lod);
#endif
}

// The parallax UV: the view ray in tangent space (Unreal's binormal, here -B: its normal maps are green-down) scaled by
// Height of Texture over N.V, stepped through the height (Channel of Height, mip 0) in lerp(MaxSteps, MinSteps, |N.V|)
// steps (floor + 2 at most) from the top; at the first sample above the ray, the secant between it and the last.
fn pom_uv(uv: vec2<f32>, t: vec3<f32>, b: vec3<f32>, n: vec3<f32>, v: vec3<f32>) -> vec2<f32> {
    let ndv = dot(n, v);
    let off = -vec2(dot(t, v), dot(-b, v)) / ndv * params.pom.x;
    let base = uv * params.pom_tiling.x + off * params.pom.y;
    let steps = mix(params.pom.w, params.pom.z, min(abs(ndv), 1.0));
    let count = floor(steps) + 2.0;
    let step = 1.0 / steps;
    let dstep = off * step;
    var cur = vec2(0.0);
    var ray = 1.0;
    var prev_ray = 1.0;
    var prev_h = 1.0;
    for (var i = 0; f32(i) < count; i++) {
        let h = dot(params.channel, sample_level(T_HEIGHT_MAP, base + cur, 0.0));
        if ray < h {
            let d = h - ray;
            cur -= d / ((prev_ray - prev_h) + d) * dstep;
            break;
        }
        prev_ray = ray;
        prev_h = h;
        ray -= step;
        cur += dstep;
    }
    return base + cur;
}

// Unreal normal maps are two-channel (BC5): z is rebuilt from x and y.
fn unpack_normal(t: vec4<f32>) -> vec3<f32> {
    let xy = t.rg * 2.0 - 1.0;
    return vec3(xy, sqrt(max(1.0 - dot(xy, xy), 0.0)));
}

fn tinted(c: vec4<f32>, tint: vec3<f32>, desaturation: f32) -> vec3<f32> {
    let t = c.rgb + c.a * (c.a * tint - c.rgb);
    let lum = dot(t, vec3(0.3, 0.59, 0.11));
    return mix(t, vec3(lum), desaturation);
}

@fragment
fn fragment(in: VertexOutput, @builtin(front_facing) is_front: bool) -> FragmentOutput {
#ifdef VISIBILITY_RANGE_DITHER
    // Its cull distance's crossfade (main.rs Placed: the placement's VisibilityRange), as Bevy's own shader does.
    bevy_pbr::pbr_functions::visibility_range_dither(in.position, in.visibility_range_dither);
#endif
#ifdef BINDLESS
    slot = mesh[in.instance_index].material_and_lightmap_bind_group_slot & 0xffffu;
    params = blend_params[blend_indices[slot].params];
#else
    params = blend_uniform;
#endif
    var pbr_input = pbr_input_from_standard_material(in, is_front);
    var material_ao = 1.0;
    let mode = u32(params.blend.z + 0.5);

#ifdef VERTEX_UVS_A
    let uv = in.uv;
#else
    let uv = vec2(0.0);
#endif
#ifdef VERTEX_COLORS
    let vertex_alpha = in.color.a;
#else
    let vertex_alpha = 1.0;
#endif

    var uv_a = uv * params.tiling.x;
    var uv_b = uv * params.tiling.z;
    var uv_rmas_b = uv_b;
#ifdef VERTEX_TANGENTS
    if params.pom.x > 0.0 {
        let tbn = calculate_tbn_mikktspace(normalize(in.world_normal), in.world_tangent);
        let p = pom_uv(uv, tbn[0], tbn[1], tbn[2], pbr_input.V);
        uv_rmas_b = uv_a;
        uv_a = p;
        uv_b = p;
    }
#endif
    let ca = sample_t(T_COLOR_A, uv_a);
    let cb = sample_t(T_COLOR_B, uv_b);
    let blend = sample_t(T_BLEND_MAP, uv * params.blend.x).r;

    var x = 2.0 * vertex_alpha - (1.0 - cb.a + blend);
    if mode == 1u {
        x = 2.0 * vertex_alpha - blend;
    }
    var t = select(0.0, 1.0, x > 0.0);
    if params.blend.y > 0.0 {
        t = saturate(x / params.blend.y);
    }
    if mode == 2u {
        t = 1.0;
    }

    let ra = sample_t(T_RMAS_A, select(uv_a, uv_rmas_b, params.pom.x > 0.0));
    let rb = sample_t(T_RMAS_B, uv_rmas_b);
    let rough_a = clamp(ra.r, params.roughness.x, params.roughness.y);
    let rough_b = clamp(rb.r, params.roughness.z, params.roughness.w);

    var color = mix(ca.rgb, cb.rgb, t);
    var specular = ra.a;
    if mode == 1u {
        color = mix(tinted(ca, params.tint_a.rgb, params.tint.x), tinted(cb, params.tint_b.rgb, params.tint.y), t);
        specular = mix(params.tint.z, params.tint.w, t);
    } else if mode == 2u {
        color = mix(params.dirt.rgb, tinted(cb, params.tint_b.rgb, params.tint.y), vertex_alpha);
        specular = params.tint.w;
    }

    pbr_input.material.base_color = vec4(saturate(color), 1.0);
    pbr_input.material.perceptual_roughness = saturate(mix(rough_a, rough_b, t));
    if mode == 0u {
        pbr_input.material.metallic = saturate(mix(ra.g, rb.g, t));
        material_ao = saturate(mix(ra.b, rb.b, t));
        pbr_input.diffuse_occlusion = vec3(material_ao);
    } else {
        pbr_input.material.metallic = 0.0;
    }
    // Unreal F0 = 0.08 * specular; Bevy F0 = 0.16 * reflectance^2.
    pbr_input.material.reflectance = vec3(sqrt(0.5 * saturate(specular)));

#ifdef VERTEX_TANGENTS
    let na = unpack_normal(sample_t(T_NORMAL_A, uv_a))
        + unpack_normal(sample_t(T_DETAIL_A, uv * params.tiling.y)) * params.detail_strength_a.xyz;
    let nb = unpack_normal(sample_t(T_NORMAL_B, uv_b))
        + unpack_normal(sample_t(T_DETAIL_B, uv * params.tiling.w)) * params.detail_strength_b.xyz;
    var nt = normalize(mix(na, nb, t));
    nt.y = -nt.y; // DirectX-style (green down)
    let double_sided = (pbr_input.material.flags & STANDARD_MATERIAL_FLAGS_DOUBLE_SIDED_BIT) != 0u;
    if double_sided && !is_front {
        nt = -nt;
    }
    let tbn = calculate_tbn_mikktspace(pbr_input.world_normal, in.world_tangent);
    pbr_input.N = normalize(nt.x * tbn[0] + nt.y * tbn[1] + nt.z * tbn[2]);
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
