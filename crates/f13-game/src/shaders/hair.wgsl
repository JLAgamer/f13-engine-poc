// Counselors' hair, from the masters' compiled base passes (F13-decomp shaders/HairMaster_03_m s188,
// shaders/M_Athlete_Girl_Hair_Inst s027; MODLOG "Counselor hair"). Both are Unreal's hair or subsurface shading
// models; here their colour, roughness and specular go to Bevy's standard lighting.
// Mode 0, HairMaster_03_m (uv TEXCOORD_0, uv_b TEXCOORD_1, the vertex colour's red c):
// - colour: lerp(Diffuse Hair Color 1, lerp(Diffuse Hair Color 2, HairColor3, Diffuse 2 Hair Mask), Mask x c) x Hair
//   Diffuse Texture (at uv_b) x Diffuse Brightness, saturated; with Use Second Hair Color off (M_Prep_Hair_Inst's
//   permutation) Diffuse Hair Color 2 itself in the inner lerp's place;
// - specular: lerp(Haircolor 1 Specular, Haircolor 2 Specular, c); roughness: Roughness;
// - opacity: RootMask^HairAlphaContrast plus the dither ((x + 2y) mod 5) / 6 over 0.9 (its scatter, tangent and pixel
//   depth offset aren't drawn).
// Mode 1, M_Athlete_Girl_Hair_Inst (its own T_Athlete_Girl_Hair_D, _N, _RMA): colour D.rgb, roughness RMA.r,
// metallic RMA.g, AO RMA.b, specular 0.06; opacity D.a plus the dither ((x + 2y) mod 5 + noise) / 6 over 0.8333
// (the noise texture's 0.5 here).
#import bevy_pbr::{
    pbr_fragment::pbr_input_from_standard_material,
    pbr_functions::{apply_pbr_lighting, main_pass_post_lighting_processing},
    forward_io::{VertexOutput, FragmentOutput},
}

struct HairParams {
    // Diffuse Hair Color 1, 2, HairColor3.
    color1: vec4<f32>,
    color2: vec4<f32>,
    color3: vec4<f32>,
    // Diffuse Brightness, Haircolor 1 Specular, Haircolor 2 Specular, Roughness.
    a: vec4<f32>,
    // HairAlphaContrast, mode, Use Second Hair Color (1 on).
    b: vec4<f32>,
}

@group(#{MATERIAL_BIND_GROUP}) @binding(100) var<uniform> params: HairParams;
@group(#{MATERIAL_BIND_GROUP}) @binding(101) var tex_sampler: sampler;
@group(#{MATERIAL_BIND_GROUP}) @binding(102) var tex_a: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(103) var tex_b: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(104) var tex_c: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(105) var tex_d: texture_2d<f32>;

fn dither(p: vec2<f32>) -> f32 {
    let q = vec2<u32>(p);
    return f32((q.x + 2u * q.y) % 5u);
}

@fragment
fn fragment(in: VertexOutput, @builtin(front_facing) is_front: bool) -> FragmentOutput {
    var pbr_input = pbr_input_from_standard_material(in, is_front);
#ifdef VERTEX_UVS_A
    let uv = in.uv;
#ifdef VERTEX_UVS_B
    let uv2 = in.uv_b;
#else
    let uv2 = in.uv;
#endif
    var c = 1.0;
#ifdef VERTEX_COLORS
    c = in.color.r;
#endif
    if params.b.y < 0.5 {
        // tex_a: Diffuse 2 Hair Mask, tex_b: Mask, tex_c: Hair Diffuse Texture, tex_d: RootMask.
        let alpha = pow(max(textureSample(tex_d, tex_sampler, uv).r, 0.0), params.b.x);
        if alpha + dither(in.position.xy) / 6.0 - 0.9 < 0.0 {
            discard;
        }
        var col = params.color2.rgb;
        if params.b.z > 0.5 {
            col = mix(col, params.color3.rgb, textureSample(tex_a, tex_sampler, uv).rgb);
        }
        col = mix(params.color1.rgb, col, textureSample(tex_b, tex_sampler, uv).rgb * c);
        col = saturate(col * textureSample(tex_c, tex_sampler, uv2).rgb * params.a.x);
        pbr_input.material.base_color = vec4(col, 1.0);
        pbr_input.material.metallic = 0.0;
        pbr_input.material.perceptual_roughness = saturate(params.a.w);
        let spec = saturate(mix(params.a.y, params.a.z, c));
        pbr_input.material.reflectance = vec3(sqrt(0.5 * spec));
    } else {
        // tex_a: D, tex_c: RMA (the normal map is the StandardMaterial part's).
        let d = textureSample(tex_a, tex_sampler, uv);
        if d.a + (dither(in.position.xy) + 0.5) / 6.0 - 0.8333 < 0.0 {
            discard;
        }
        let rma = saturate(textureSample(tex_c, tex_sampler, uv).rgb);
        pbr_input.material.base_color = vec4(saturate(d.rgb), 1.0);
        pbr_input.material.perceptual_roughness = rma.r;
        pbr_input.material.metallic = rma.g;
        pbr_input.diffuse_occlusion = vec3(rma.b);
        pbr_input.material.reflectance = vec3(sqrt(0.5 * 0.06));
    }
#endif
    var out: FragmentOutput;
    out.color = apply_pbr_lighting(pbr_input);
    out.color = main_pass_post_lighting_processing(pbr_input, out.color);
    return out;
}
