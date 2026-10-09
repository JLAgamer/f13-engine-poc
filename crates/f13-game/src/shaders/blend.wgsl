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

@group(#{MATERIAL_BIND_GROUP}) @binding(100) var<uniform> params: BlendParams;
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
        let h = dot(params.channel, textureSampleLevel(height_map, tex_sampler, base + cur, 0.0));
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
    let ca = textureSample(color_a, tex_sampler, uv_a);
    let cb = textureSample(color_b, tex_sampler, uv_b);
    let blend = textureSample(blend_map, tex_sampler, uv * params.blend.x).r;

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

    let ra = textureSample(rmas_a, tex_sampler, select(uv_a, uv_rmas_b, params.pom.x > 0.0));
    let rb = textureSample(rmas_b, tex_sampler, uv_rmas_b);
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
    let na = unpack_normal(textureSample(normal_a, tex_sampler, uv_a))
        + unpack_normal(textureSample(detail_a, tex_sampler, uv * params.tiling.y)) * params.detail_strength_a.xyz;
    let nb = unpack_normal(textureSample(normal_b, tex_sampler, uv_b))
        + unpack_normal(textureSample(detail_b, tex_sampler, uv * params.tiling.w)) * params.detail_strength_b.xyz;
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
