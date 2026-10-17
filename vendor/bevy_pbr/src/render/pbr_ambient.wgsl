#define_import_path bevy_pbr::ambient

#import bevy_pbr::{
    lighting::{EnvBRDFApprox, F_AB},
    mesh_view_bindings::lights,
}

// A precomputed `NdotV` is provided because it is computed regardless,
// but `world_normal` and the view vector `V` are provided separately for more advanced uses.
fn ambient_light(
    world_position: vec4<f32>,
    world_normal: vec3<f32>,
    V: vec3<f32>,
    NdotV: f32,
    diffuse_color: vec3<f32>,
    specular_color: vec3<f32>,
    perceptual_roughness: f32,
    occlusion: vec3<f32>,
) -> vec3<f32> {
    // f13-rs: Unreal 4.18's ambient cubemap pass (FPostProcessAmbientPS) adds DiffuseColor x the cubemap x
    // AmbientCubemapColor x AO, with no split-sum factor on the diffuse (stock Bevy: EnvBRDFApprox(diffuse,
    // F_AB(1, NdotV)), about 0.45 x diffuse - 0.0024, which left the ambient-lit interiors 2-2.5x too dark).
    let diffuse_ambient = diffuse_color;
    let specular_ambient = EnvBRDFApprox(specular_color, F_AB(perceptual_roughness, NdotV));

    // No real world material has specular values under 0.02, so we use this range as a
    // "pre-baked specular occlusion" that extinguishes the fresnel term, for artistic control.
    // See: https://google.github.io/filament/Filament.md.html#specularocclusion
    let specular_occlusion = saturate(dot(specular_color, vec3(50.0 * 0.33)));

    return (diffuse_ambient + specular_ambient * specular_occlusion) * lights.ambient_color.rgb * occlusion;
}
