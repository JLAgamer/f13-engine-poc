// Counselor skin (SkinGoreMaster), from its compiled base pass (F13-decomp shaders/SkinGoreMaster s192; MODLOG "Skin
// gore"), clipped under the clothes like the game's alpha mask compiler (MM_AlphaMaskCompiler_Skin), which bakes the
// skin diffuse with alpha = saturate(1 - FaceMask.r * AlphaMask.r) for the masked skin material.
// mask_a is the counselor's own mask (white: body below the neck), mask_b the outfit's SkinAlphaMask, which is white
// where skin stays visible (head, hands); the game must feed it to the compiler inverted (where is not found), as
// 1 - a * b with it as-is would hide the hands and keep the skin under the clothes. Inferred, checked on screen. (The
// face: both black, nothing clipped.)
// The gore as the clothes' (shaders/clothes.wgsl), the damage texture holding its body parts' masks (Head Damage too)
// and the burn (Burn Damage 1's red) in its alpha: the normal towards UV2_Gore_normal by its r then g (no flattening);
// the colour Skin Diffuse x Skin_diffuse_DarkTint, then the soak, blood, gore and char; metallic 0 towards 0.2 by b,
// 0.3 by r, 0.2 by g, 0 by a; specular 0.35 + Sweat (towards 1 by PoreSpecWeight), towards 0.5 by r and g; roughness
// RoughMin..RoughMax by RMA.r, x RoughnessMultiplier, or x RainGloss with the rain, then the gore's as the clothes';
// AO RMA.b towards 1 by r and g. (Its subsurface colour, RMA.g x SSScolor, isn't ported.)
#import bevy_pbr::{
    pbr_fragment::pbr_input_from_standard_material,
    pbr_functions::{alpha_discard, apply_pbr_lighting, main_pass_post_lighting_processing},
    forward_io::{VertexOutput, FragmentOutput},
    mesh_view_bindings::globals,
}

#ifdef SCREEN_SPACE_AMBIENT_OCCLUSION
#import bevy_pbr::mesh_view_bindings::screen_space_ambient_occlusion_texture
#endif

struct SkinParams {
    // Skin_diffuse_DarkTint
    dark_tint: vec4<f32>,
    // RoughMin, RoughMax, RoughnessMultiplier, RainGloss
    rough: vec4<f32>,
    // 0.35 + Sweat, PoreSpecWeight, Rain_MPC's Rain, -
    spec: vec4<f32>,
    // SelectionColor
    selection: vec4<f32>,
}

@group(#{MATERIAL_BIND_GROUP}) @binding(100) var mask_a: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(101) var mask_sampler: sampler;
@group(#{MATERIAL_BIND_GROUP}) @binding(102) var mask_b: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(103) var<uniform> params: SkinParams;
@group(#{MATERIAL_BIND_GROUP}) @binding(104) var damage: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(105) var gore_normal: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(106) var gore_rgb: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(107) var colormask: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(108) var rma: texture_2d<f32>;

@fragment
fn fragment(in: VertexOutput, @builtin(front_facing) is_front: bool) -> FragmentOutput {
#ifdef VISIBILITY_RANGE_DITHER
    // Its cull distance's crossfade (main.rs Placed: the placement's VisibilityRange), as Bevy's own shader does.
    bevy_pbr::pbr_functions::visibility_range_dither(in.position, in.visibility_range_dither);
#endif
    var pbr_input = pbr_input_from_standard_material(in, is_front);
    var material_ao = 1.0;
#ifdef VERTEX_UVS_A
    let a = textureSample(mask_a, mask_sampler, in.uv).r;
    let b = textureSample(mask_b, mask_sampler, in.uv).r;
    pbr_input.material.base_color.a = saturate(1.0 - a * (1.0 - b));
#ifdef VERTEX_UVS_B
    let uv = in.uv;
    let uv2 = in.uv_b;
    let m = saturate(textureSample(damage, mask_sampler, uv2));
    let wet = m.r;
    let gore = m.g;
    let soak = m.b;
    let burn = m.a;
    let rain = params.spec.z;

#ifdef VERTEX_TANGENTS
    let g = textureSample(gore_normal, mask_sampler, uv2).xy * 2.0 - 1.0;
    let gn = vec3(g, sqrt(max(1.0 - dot(g, g), 0.0)));
    var geom = normalize(in.world_normal);
    if !is_front {
        geom = -geom;
    }
    let t = normalize(in.world_tangent.xyz);
    let bt = cross(geom, t) * in.world_tangent.w;
    let gore_n = gn.x * t + gn.y * bt + gn.z * geom;
    var n = mix(pbr_input.N, gore_n, wet);
    pbr_input.N = normalize(mix(n, gore_n, gore));
#endif

    var c = pbr_input.material.base_color.rgb * params.dark_tint.rgb;
    c = mix(c, vec3(0.04, 0.003936, 0.0), soak);
    let cm = textureSample(colormask, mask_sampler, uv2 * 12.0).rg;
    let k = saturate(cm.r * 1.4 - 0.2);
    var blood = vec3(0.015, 0.001476, 0.0) + k * vec3(0.045, 0.017141, 0.0141);
    blood = mix(blood, vec3(0.07, 0.029775, 0.0287), cm.g);
    c = mix(c, blood, wet);
    c = mix(c, vec3(0.02, 0.002192, 0.003988), gore);
    let gr = textureSample(gore_rgb, mask_sampler, uv2).rg;
    c = saturate(mix(c, (1.0 - gr.r) * vec3(0.01, 0.005813, 0.003365), burn));
    pbr_input.material.base_color = vec4(c, pbr_input.material.base_color.a);

    var metal = mix(0.2 * soak, 0.3, wet);
    metal = mix(metal, 0.2, gore);
    pbr_input.material.metallic = saturate(mix(metal, 0.0, burn));
    var spec = mix(params.spec.x, 1.0, params.spec.y);
    spec = mix(spec, 0.5, wet);
    spec = saturate(mix(spec, 0.5, gore));
    pbr_input.material.reflectance = vec3(sqrt(0.5 * spec));
    let r = textureSample(rma, mask_sampler, uv);
    let rough0 = mix(params.rough.x, params.rough.y, r.r);
    var rough = mix(rough0 * params.rough.z, rough0 * params.rough.w, rain);
    rough = mix(rough, 0.2, soak);
    rough = mix(rough, 0.4 - 0.32 * cm.g, wet);
    rough = mix(rough, 0.2, gore);
    pbr_input.material.perceptual_roughness = saturate(mix(rough, 0.6, burn));
    material_ao = saturate(mix(mix(r.b, 1.0, wet), 1.0, gore));
    pbr_input.diffuse_occlusion = vec3(material_ao);

    let scroll = textureSample(gore_rgb, mask_sampler, vec2(uv2.x + globals.time * 0.1, uv2.y)).r;
    let glow = scroll * vec3(1.0, 0.245427, 0.12) * gr.g;
    pbr_input.material.emissive = vec4(max(burn * glow + params.selection.rgb, vec3(0.0)), 1.0);
#endif
#endif
    pbr_input.material.base_color = alpha_discard(pbr_input.material, pbr_input.material.base_color);
#ifdef SCREEN_SPACE_AMBIENT_OCCLUSION
    // The ambient light's occlusion as the engine's ambient pass has it (FPostProcessAmbientPS): the screen-space AO
    // (ssao.rs) times the material's own, on its diffuse and specular alike.
    let ao = material_ao * textureLoad(screen_space_ambient_occlusion_texture, vec2<i32>(in.position.xy), 0i).r;
    pbr_input.diffuse_occlusion = vec3(ao);
    pbr_input.specular_occlusion = ao;
#endif
    var out: FragmentOutput;
    out.color = apply_pbr_lighting(pbr_input);
    out.color = main_pass_post_lighting_processing(pbr_input, out.color);
    return out;
}
