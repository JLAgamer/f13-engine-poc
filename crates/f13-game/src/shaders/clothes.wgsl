// Counselors' clothes (ClothesMultiGore: Clothes_Layer under Gore_Layer, Blood_Layer, Gore_Bright_Layer and
// Gore_Burn_Layer), from its compiled base pass (F13-decomp shaders/ClothesMultiGore s192; MODLOG "Clothes gore").
// The StandardMaterial part gives the clothing's diffuse (base colour) and normal; uv is TEXCOORD_0, uv_b TEXCOORD_1
// (the gore's UVs).
// - The damage: the body parts' masks (Torso / Left Arm / Right Arm / Left Leg / Right Leg Damage, Environment Damage,
//   Environment Damage 2) summed by channel and saturated, here combined on the CPU into one texture (character.rs
//   blood): r = their blue (the wet blood), g = their alpha (the dark gore), b = their red (the soaked cloth),
//   a = Environment Damage's green (the burn).
// - The normal: the cloth's, towards UV2_Gore_normal by r, then by g, flat by a.
// - The colour: the diffuse, desaturated by Cloth_Diffuse_Desaturation, overlaid with Cloth_diffuse_HueTint, plus
//   Cloth_diffuse_LIghtTint, times Cloth_diffuse_DarkTint (with TilePatternToggle, the tile pattern's tints over it),
//   darkened by the rain (x (1 - Rain / 2)); then towards (0.04, 0.003936, 0) by b, the blood (UV2_Gore_Colormask at
//   uv_b x 12: (0.015, 0.001476, 0) + 1.4 cm - 0.2 x (0.045, 0.017141, 0.0141), towards (0.07, 0.029775, 0.0287) by
//   its green) by r, (0.02, 0.002192, 0.003988) by g, and the burn's char ((1 - Gore_RGB.r) x (0.01, 0.005813,
//   0.003365)) by a.
// - Metallic: RMA.g, towards 0.2 by b, 0.3 by r, 0.2 by g, 0 by a. Roughness: RMA.r towards RainGloss by the rain,
//   then 0.2 by b, 0.4 - 0.32 colormask.g by r, 0.2 by g, 0.6 by a. AO: RMA.b towards 1 by r and g, + RMA.b x a.
// - Emissive: the burn's glow (Gore_RGB.r scrolling at 0.1 a second along u, x (1, 0.245427, 0.12) x Gore_RGB.g) by a,
//   plus SelectionColor.
#import bevy_pbr::{
    pbr_fragment::pbr_input_from_standard_material,
    pbr_functions::{apply_pbr_lighting, main_pass_post_lighting_processing},
    forward_io::{VertexOutput, FragmentOutput},
    mesh_view_bindings::globals,
}

#ifdef SCREEN_SPACE_AMBIENT_OCCLUSION
#import bevy_pbr::mesh_view_bindings::screen_space_ambient_occlusion_texture
#endif

struct ClothesParams {
    // Cloth_diffuse_HueTint
    hue: vec4<f32>,
    // TileTint, TileTint_logo, Tint_AdditionalElements
    tile_tint: vec4<f32>,
    logo_tint: vec4<f32>,
    extra_tint: vec4<f32>,
    // Cloth_Diffuse_Desaturation, Cloth_diffuse_LIghtTint, Cloth_diffuse_DarkTint, TilePatternTile
    tints: vec4<f32>,
    // PAttern_IsolationMAsk, InvertTilePatternMask, addGreenChannel, addblueChannel
    pattern: vec4<f32>,
    // addGreenChannel_TP2, TilePatternToggle, RainGloss, Rain_MPC Rain
    more: vec4<f32>,
    // SelectionColor
    selection: vec4<f32>,
}

@group(#{MATERIAL_BIND_GROUP}) @binding(100) var<uniform> params: ClothesParams;
@group(#{MATERIAL_BIND_GROUP}) @binding(101) var gore_normal: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(102) var tex_sampler: sampler;
@group(#{MATERIAL_BIND_GROUP}) @binding(103) var damage: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(104) var gore_rgb: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(105) var pattern: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(106) var pattern2: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(107) var colormask: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(108) var rma: texture_2d<f32>;

fn overlay(c: vec3<f32>, t: vec3<f32>) -> vec3<f32> {
    let lo = c * t * 2.0;
    let hi = vec3(1.0) - (vec3(1.0) - c) * (vec3(1.0) - t) * 2.0;
    return select(lo, hi, c >= vec3(0.5));
}

@fragment
fn fragment(in: VertexOutput, @builtin(front_facing) is_front: bool) -> FragmentOutput {
    var pbr_input = pbr_input_from_standard_material(in, is_front);
    var material_ao = 1.0;
#ifdef VERTEX_UVS_A
#ifdef VERTEX_UVS_B
    let uv = in.uv;
    let uv2 = in.uv_b;
    let m = saturate(textureSample(damage, tex_sampler, uv2));
    let wet = m.r;
    let gore = m.g;
    let soak = m.b;
    let burn = m.a;
    let rain = params.more.w;

    // The normal.
#ifdef VERTEX_TANGENTS
    let g = textureSample(gore_normal, tex_sampler, uv2).xy * 2.0 - 1.0;
    let gn = vec3(g, sqrt(max(1.0 - dot(g, g), 0.0)));
    var geom = normalize(in.world_normal);
    if !is_front {
        geom = -geom;
    }
    let t = normalize(in.world_tangent.xyz);
    let b = cross(geom, t) * in.world_tangent.w;
    let gore_n = gn.x * t + gn.y * b + gn.z * geom;
    var n = mix(pbr_input.N, gore_n, wet);
    n = mix(n, gore_n, gore);
    n = mix(n, geom, burn);
    pbr_input.N = normalize(n);
#endif

    // The cloth's colour.
    let d = pbr_input.material.base_color.rgb;
    let lum = dot(d, vec3(0.3, 0.59, 0.11));
    var c = mix(d, vec3(lum), params.tints.x);
    c = overlay(c, params.hue.rgb) + params.tints.y;
    let plain = c * params.tints.z;
    let p = textureSample(pattern, tex_sampler, uv * params.tints.w).rgb;
    let q = textureSample(pattern2, tex_sampler, uv).rg;
    var pm = mix(p.r, q.r * p.r, params.pattern.x);
    if params.pattern.y >= 0.1 {
        pm = 1.0 - pm;
    }
    var tiled = mix(params.tile_tint.rgb, plain, pm);
    var green = select(0.0, p.g, params.pattern.z >= 0.1);
    if params.pattern.w >= 0.1 {
        green = p.b + green;
    }
    tiled = mix(tiled, params.logo_tint.rgb, green);
    let green2 = select(0.0, q.g, params.more.x >= 0.1);
    tiled = mix(tiled, params.extra_tint.rgb, green2);
    c = select(plain, tiled, params.more.y >= 0.1);
    c = c - c * rain * 0.5;

    // The gore and blood over it.
    c = mix(c, vec3(0.04, 0.003936, 0.0), soak);
    let cm = textureSample(colormask, tex_sampler, uv2 * 12.0).rg;
    let k = saturate(cm.r * 1.4 - 0.2);
    var blood = vec3(0.015, 0.001476, 0.0) + k * vec3(0.045, 0.017141, 0.0141);
    blood = mix(blood, vec3(0.07, 0.029775, 0.0287), cm.g);
    c = mix(c, blood, wet);
    c = mix(c, vec3(0.02, 0.002192, 0.003988), gore);
    let gr = textureSample(gore_rgb, tex_sampler, uv2).rg;
    c = saturate(mix(c, (1.0 - gr.r) * vec3(0.01, 0.005813, 0.003365), burn));
    pbr_input.material.base_color = vec4(c, 1.0);

    // Metallic, roughness, AO.
    let r = textureSample(rma, tex_sampler, uv);
    var metal = mix(r.g, 0.2, soak);
    metal = mix(metal, 0.3, wet);
    metal = mix(metal, 0.2, gore);
    pbr_input.material.metallic = saturate(mix(metal, 0.0, burn));
    var rough = mix(r.r, params.more.z, rain);
    rough = mix(rough, 0.2, soak);
    rough = mix(rough, 0.4 - 0.32 * cm.g, wet);
    rough = mix(rough, 0.2, gore);
    pbr_input.material.perceptual_roughness = saturate(mix(rough, 0.6, burn));
    var ao = mix(r.b, 1.0, wet);
    ao = mix(ao, 1.0, gore);
    material_ao = saturate(ao + r.b * burn);
    pbr_input.diffuse_occlusion = vec3(material_ao);
    pbr_input.material.reflectance = vec3(0.5);

    // The burn's glow.
    let scroll = textureSample(gore_rgb, tex_sampler, vec2(uv2.x + globals.time * 0.1, uv2.y)).r;
    let glow = scroll * vec3(1.0, 0.245427, 0.12) * gr.g;
    pbr_input.material.emissive = vec4(max(burn * glow + params.selection.rgb, vec3(0.0)), 1.0);
#endif
#endif
#ifdef SCREEN_SPACE_AMBIENT_OCCLUSION
    let ssao = material_ao * textureLoad(screen_space_ambient_occlusion_texture, vec2<i32>(in.position.xy), 0i).r;
    pbr_input.diffuse_occlusion = vec3(ssao);
    pbr_input.specular_occlusion = ssao;
#endif
    var out: FragmentOutput;
    out.color = apply_pbr_lighting(pbr_input);
    out.color = main_pass_post_lighting_processing(pbr_input, out.color);
    return out;
}
