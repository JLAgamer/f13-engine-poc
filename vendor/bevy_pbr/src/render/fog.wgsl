#define_import_path bevy_pbr::fog

#import bevy_pbr::{
    mesh_view_bindings::fog,
    mesh_view_types::Fog,
}

// Fog formulas adapted from:
// https://learn.microsoft.com/en-us/windows/win32/direct3d9/fog-formulas
// https://catlikecoding.com/unity/tutorials/rendering/part-14/
// https://iquilezles.org/articles/fog/ (Atmospheric Fog and Scattering)

fn scattering_adjusted_fog_color(
    fog_params: Fog,
    scattering: vec3<f32>,
) -> vec4<f32> {
    if (fog_params.directional_light_color.a > 0.0) {
        return vec4<f32>(
            fog_params.base_color.rgb
                + scattering * fog_params.directional_light_color.rgb * fog_params.directional_light_color.a,
            fog_params.base_color.a,
        );
    } else {
        return fog_params.base_color;
    }
}

fn linear_fog(
    fog_params: Fog,
    input_color: vec4<f32>,
    distance: f32,
    scattering: vec3<f32>,
) -> vec4<f32> {
    var fog_color = scattering_adjusted_fog_color(fog_params, scattering);
    let start = fog_params.be.x;
    let end = fog_params.be.y;
    fog_color.a *= 1.0 - clamp((end - distance) / (end - start), 0.0, 1.0);
    return vec4<f32>(mix(input_color.rgb, fog_color.rgb, fog_color.a), input_color.a);
}

fn exponential_fog(
    fog_params: Fog,
    input_color: vec4<f32>,
    distance: f32,
    scattering: vec3<f32>,
) -> vec4<f32> {
    var fog_color = scattering_adjusted_fog_color(fog_params, scattering);
    let density = fog_params.be.x;
    // f13-rs: the colour's alpha is Unreal's FogMaxOpacity, a cap on the fog (FExponentialHeightFogPS: transmittance
    // max(exp2(-integral), 1 - FogMaxOpacity)), not a factor (stock Bevy: a x (1 - e^-dD), a fifth of the rain's fog).
    fog_color.a = min(1.0 - 1.0 / exp(distance * density), fog_color.a);
    return vec4<f32>(mix(input_color.rgb, fog_color.rgb, fog_color.a), input_color.a);
}

// f13-rs: Unreal's exponential height fog, FExponentialHeightFogPS's (UE 4.18; the global shader cache's 3fe31 and,
// with directional inscattering, 128c4 permutations), its start distance 0. be: FogDensity at the fog's height and
// FogHeightFalloff (base 2, per m), the fog's height; bi.x DirectionalInscatteringStartDistance; base_color.rgb the
// inscattering, .a FogMaxOpacity. `ray_y` is the ray's rise, `view_y` the camera's height. `scattering`: the
// directional light's pow(saturate(dot(ray, light)), exponent) x its luminance (apply_fog).
fn height_fog(
    fog_params: Fog,
    input_color: vec4<f32>,
    distance: f32,
    ray_y: f32,
    view_y: f32,
    scattering: vec3<f32>,
) -> vec4<f32> {
    let density = fog_params.be.x;
    let falloff = fog_params.be.y;
    // The fog's density where the ray starts (RayOriginTerms).
    let origin = density * exp2(-max(falloff * (view_y - fog_params.be.z), -127.0));
    let f = max(falloff * ray_y, -127.0);
    // (1 - 2^-f) / f, its Taylor expansion near 0.
    var line = 0.693147 - 0.240227 * f;
    if abs(f) > 0.01 {
        line = (1.0 - exp2(-f)) / f;
    }
    let integral = origin * line;
    let transmittance = max(min(exp2(-integral * distance), 1.0), 1.0 - fog_params.base_color.a);
    // Directional inscattering past its start distance, not capped by FogMaxOpacity.
    let directional = 1.0 - min(exp2(-integral * max(distance - fog_params.bi.x, 0.0)), 1.0);
    let color = fog_params.base_color.rgb * (1.0 - transmittance)
        + fog_params.directional_light_color.rgb * scattering * directional;
    return vec4<f32>(input_color.rgb * transmittance + color, input_color.a);
}

fn exponential_squared_fog(
    fog_params: Fog,
    input_color: vec4<f32>,
    distance: f32,
    scattering: vec3<f32>,
) -> vec4<f32> {
    var fog_color = scattering_adjusted_fog_color(fog_params, scattering);
    let distance_times_density = distance * fog_params.be.x;
    fog_color.a *= 1.0 - 1.0 / exp(distance_times_density * distance_times_density);
    return vec4<f32>(mix(input_color.rgb, fog_color.rgb, fog_color.a), input_color.a);
}

fn atmospheric_fog(
    fog_params: Fog,
    input_color: vec4<f32>,
    distance: f32,
    scattering: vec3<f32>,
) -> vec4<f32> {
    var fog_color = scattering_adjusted_fog_color(fog_params, scattering);
    let extinction_factor = 1.0 - 1.0 / exp(distance * fog_params.be);
    let inscattering_factor = 1.0 - 1.0 / exp(distance * fog_params.bi);
    return vec4<f32>(
        input_color.rgb * (1.0 - extinction_factor * fog_color.a)
            + fog_color.rgb * inscattering_factor * fog_color.a,
        input_color.a
    );
}
