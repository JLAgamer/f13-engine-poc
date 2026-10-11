// The impact decals' masters (MD_DeferredDecal), from their compiled deferred decal passes (FDeferredDecalPS; F13-decomp
// MODLOG "Impact decals"). Drawn as the decal's box (decals.rs: a 2 x 2 x 2 cube scaled by DecalSize, its back faces,
// no depth test): each pixel's scene position (the depth prepass) is taken into the decal's space and dropped outside
// the box, as the engine's decal pass does. The engine blends the decal's attributes into the G-buffer (or, for the
// blood, the DBuffer) before lighting; here the decal is lit with its own attributes and blended over the lit scene
// (the same where the lighting is linear in them; the underlying surface's specular and metalness are not known here).
//
// p is the decal's local position in Unreal's axes (X into the surface, -1..1), p01 = p x 0.5 + 0.5.
// Mode 0, M_Impact_Decal_MASTER_01 (s001): uv0 = Texture1_Rotation's 2 x 2 rotation of (p.z, p.y) x 0.5, + 0.5;
//   Diffuse (tex_a) and Normal (tex_b) at uv0; alpha = clamp(saturate(lerp(-ALPHA_Contrast, 1 + ALPHA_Contrast,
//   Diffuse.a)) x OpacityLevel, AlphaClamp_Min, AlphaClamp_Max) (saturated); the overlay mask (tex_c .r) at
//   (p01.z x 0_U_Multiplier + 1_U_Offset, p01.y x 0_V_Multiplier + 1_V_Offset) turned by Overlay_Rotation about its
//   middle, contrasted by 0_OverlayTextureContrast and x 0_OverlayTextureMultiplier (saturated); base colour =
//   clamp((Diffuse.rgb + mask) x 0_BaseMasterMultiplier x Decal_Color, 0_BaseClamp_Min, 0_BaseClamp_Max) (saturated);
//   normal (tangent) = (Normal.xy x z_Normal_Multiplier, sqrt(1 - |Normal.xy|^2)) lerped from Normal_Base by
//   saturate(2 saturate(z_Normal_Multiplier, 0, 0.5)) and toward (0, 0, 1) by saturate(that x 2 - 1); its z along
//   -X, y along Y, x along Z; roughness saturate(Roughness), specular saturate(SpecularLevel).
// Mode 1, M_Wall_Impact_Default_01 (s001): uv the overlay's (Rotation, 0_U/V_Multiplier, 1_U/V_Offset); Diffuse
//   (tex_a), Normal (tex_b); a = saturate(lerp(-ALPHA_Contrast, 1 + ALPHA_Contrast, Diffuse.a)); opacity =
//   saturate(a x OpacityLevel); base colour saturate(Diffuse.rgb x Decal_Color); normal lerp((0, 0, 1), the normal
//   map's, Normal_Levels) in world space (bTangentSpaceNormal off); roughness saturate(lerp(roughLow, RoughHigh, a)),
//   specular saturate(SpecularLevel).
// Mode 2, M_Blood_Decal_01 (DBM_DBuffer_ColorNormalRoughness, s001): uv as mode 1; t = Diffuse (tex_a) .rgb; opacity
//   = saturate(lerp(-ALPHA_Contrast, 1 + ALPHA_Contrast, t.r x OpacityLevel)); colour saturate(t x Decal_Color);
//   normal the decal's -X (the surface's facing); roughness saturate(Roughness). PLACEHOLDER: its specular (the
//   DBuffer keeps the surface's; 0.5 here).
// Every mode's opacity x saturate(4 - 4 |p.x|) (the fade along the projection) x the decal's fade (params.mode.y:
// FDeferredDecalPS's DecalParams.x x .y).
#import bevy_pbr::{
    pbr_fragment::pbr_input_from_standard_material,
    pbr_functions::{apply_pbr_lighting, main_pass_post_lighting_processing},
    forward_io::{VertexOutput, FragmentOutput},
    mesh_functions::get_world_from_local,
    mesh_view_bindings::view,
}

#ifdef DEPTH_PREPASS
#import bevy_pbr::prepass_utils::prepass_depth
#endif
#ifdef SCREEN_SPACE_AMBIENT_OCCLUSION
#import bevy_pbr::mesh_view_bindings::screen_space_ambient_occlusion_texture
#endif

struct DecalParams {
    // x: mode; y: the decal's fade
    mode: vec4<f32>,
    // The uv's rotation: rows (x, y), (z, w)
    rot: vec4<f32>,
    // The overlay's rotation (mode 0)
    rot2: vec4<f32>,
    // U multiplier, U offset, V multiplier, V offset
    uv: vec4<f32>,
    color: vec4<f32>,
    // -ALPHA_Contrast, 1 + ALPHA_Contrast, OpacityLevel, AlphaClamp_Min
    alpha: vec4<f32>,
    // AlphaClamp_Max, Roughness (roughLow), SpecularLevel, Normal_Levels (z_Normal_Multiplier)
    surface: vec4<f32>,
    // Mode 0: -OverlayTextureContrast, 1 + OverlayTextureContrast, OverlayTextureMultiplier, BaseMasterMultiplier;
    // mode 1: RoughHigh
    overlay: vec4<f32>,
    // Mode 0: BaseClamp_Min, BaseClamp_Max, the normal's lerp from Normal_Base, its lerp toward (0, 0, 1)
    base: vec4<f32>,
    // Mode 0: Normal_Base
    normal_base: vec4<f32>,
}

@group(#{MATERIAL_BIND_GROUP}) @binding(100) var<uniform> params: DecalParams;
@group(#{MATERIAL_BIND_GROUP}) @binding(101) var tex_a: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(102) var sampler_a: sampler;
@group(#{MATERIAL_BIND_GROUP}) @binding(103) var tex_b: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(104) var sampler_b: sampler;
@group(#{MATERIAL_BIND_GROUP}) @binding(105) var tex_c: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(106) var sampler_c: sampler;

fn rotate(uv: vec2<f32>, r: vec4<f32>) -> vec2<f32> {
    return vec2(dot(uv, r.xy), dot(uv, r.zw));
}

// Unreal's (cm, Z up) direction to Bevy's (Y up).
fn from_unreal(v: vec3<f32>) -> vec3<f32> {
    return vec3(v.x, v.z, v.y);
}

@fragment
fn fragment(in: VertexOutput, @builtin(front_facing) is_front: bool) -> FragmentOutput {
#ifndef DEPTH_PREPASS
    discard;
#else
    // The scene's position under this pixel.
    let depth = prepass_depth(in.position, 0u);
    let uv_screen = (in.position.xy - view.viewport.xy) / view.viewport.zw;
    let ndc = vec4(uv_screen.x * 2.0 - 1.0, 1.0 - uv_screen.y * 2.0, depth, 1.0);
    let w = view.world_from_clip * ndc;
    let scene = w.xyz / w.w;

    // Into the decal's space: its columns are its axes scaled (Bevy x, y, z = Unreal X, Z, Y).
    let m = get_world_from_local(in.instance_index);
    let c0 = m[0].xyz;
    let c1 = m[1].xyz;
    let c2 = m[2].xyz;
    let d = scene - m[3].xyz;
    let lb = vec3(dot(c0, d) / dot(c0, c0), dot(c1, d) / dot(c1, c1), dot(c2, d) / dot(c2, c2));
    let p = vec3(lb.x, lb.z, lb.y);
    if any(p < vec3(-1.0)) || any(p > vec3(1.0)) {
        discard;
    }
    let p01 = p * 0.5 + 0.5;
    // The decal's axes in Bevy space: Unreal X, Y, Z.
    let ax = normalize(c0);
    let ay = normalize(c2);
    let az = normalize(c1);

    let mode = u32(params.mode.x);
    var base_color: vec3<f32>;
    var opacity: f32;
    var n: vec3<f32>;
    var roughness: f32;
    var specular = 0.5;
    if mode == 0u {
        let uv0 = rotate(vec2(p.z, p.y) * 0.5, params.rot) + 0.5;
        let diffuse = textureSample(tex_a, sampler_a, uv0);
        let nm = textureSample(tex_b, sampler_b, uv0).xy * 2.0 - 1.0;
        var a = saturate(mix(params.alpha.x, params.alpha.y, diffuse.a)) * params.alpha.z;
        a = saturate(min(max(a, params.alpha.w), params.surface.x));
        opacity = a;
        let ovl_uv = rotate(vec2(p01.z * params.uv.x + params.uv.y, p01.y * params.uv.z + params.uv.w) - 0.5, params.rot2) + 0.5;
        var mask = textureSample(tex_c, sampler_c, ovl_uv).r;
        mask = saturate(saturate(mix(params.overlay.x, params.overlay.y, mask)) * params.overlay.z);
        base_color = saturate(min(max((diffuse.rgb + mask) * params.overlay.w * params.color.rgb, vec3(params.base.x)), vec3(params.base.y)));
        var t = vec3(nm * params.surface.w, sqrt(max(1.0 - dot(nm, nm), 0.0)));
        t = mix(params.normal_base.xyz, t, params.base.z);
        t = normalize(mix(t, vec3(0.0, 0.0, 1.0), params.base.w));
        n = normalize(-t.z * ax + t.y * ay + t.x * az);
        roughness = saturate(params.surface.y);
        specular = saturate(params.surface.z);
    } else {
        let uv = rotate(vec2(p01.z * params.uv.x + params.uv.y, p01.y * params.uv.z + params.uv.w) - 0.5, params.rot) + 0.5;
        let diffuse = textureSample(tex_a, sampler_a, uv);
        if mode == 1u {
            let a = saturate(mix(params.alpha.x, params.alpha.y, diffuse.a));
            opacity = saturate(a * params.alpha.z);
            base_color = saturate(diffuse.rgb * params.color.rgb);
            let nm = textureSample(tex_b, sampler_b, uv).xy * 2.0 - 1.0;
            let t = mix(vec3(0.0, 0.0, 1.0), vec3(nm, sqrt(max(1.0 - dot(nm, nm), 0.0))), params.surface.w);
            n = normalize(from_unreal(t));
            roughness = saturate(mix(params.surface.y, params.overlay.x, a));
            specular = saturate(params.surface.z);
        } else {
            opacity = saturate(mix(params.alpha.x, params.alpha.y, diffuse.r * params.alpha.z));
            base_color = saturate(diffuse.rgb * params.color.rgb);
            n = -ax;
            roughness = saturate(params.surface.y);
        }
    }
    opacity = opacity * saturate(4.0 - 4.0 * abs(p.x)) * params.mode.y;

    var pbr_input = pbr_input_from_standard_material(in, is_front);
    pbr_input.world_position = vec4(scene, 1.0);
    pbr_input.world_normal = n;
    pbr_input.N = n;
    pbr_input.V = normalize(view.world_position - scene);
    pbr_input.material.base_color = vec4(base_color, opacity);
    pbr_input.material.perceptual_roughness = roughness;
    pbr_input.material.metallic = 0.0;
    pbr_input.material.reflectance = vec3(sqrt(0.5 * specular));
#ifdef SCREEN_SPACE_AMBIENT_OCCLUSION
    let ao = textureLoad(screen_space_ambient_occlusion_texture, vec2<i32>(in.position.xy), 0i).r;
    pbr_input.diffuse_occlusion = vec3(ao);
    pbr_input.specular_occlusion = ao;
#endif
    var out: FragmentOutput;
    out.color = apply_pbr_lighting(pbr_input);
    out.color = main_pass_post_lighting_processing(pbr_input, out.color);
    out.color.a = opacity;
    return out;
#endif
}
