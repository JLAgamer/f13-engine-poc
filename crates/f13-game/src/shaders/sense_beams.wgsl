// Master_Sense_Lightbeam_01 (the cabins' Sense light: SM_Sense_*'s window and crack rays and holes, shown by their
// SetSenseEffects), from its compiled base pass (F13-decomp shaders/Master_Sense_Lightbeam_01 s016; uniexpr.py for its
// uniforms). Translucent, two-sided, drawn without the depth test (materials.rs BeamExt):
// - the air: Air Mask's red at the UV x (Rotator X, Y) - 0.5 turned by time x Air Speed x 0.05 rad (+ 0.5), times its
//   red at the UV x (Panning X, Y) + that angle; times saturate(vertex alpha x (1 + 2 Ray Length) - Ray Length);
// - x the depth fade saturate((scene depth - its depth) / max(Depth Fade Distance, 1e-5)) (cm): behind a wall, none;
// - k = lerp(Lerp_A, that, (its depth / Cuttoff)^Cutoff Fade);
// - opacity = saturate(1 - (Fresnel + (1 - Fresnel) x (1 - max(N.V, 0))^k));
// - colour: Color x Emissive_Level (its emissive). PLACEHOLDERS: its base colour (the same, saturated) lit by the
//   translucency volume, and its world position offset (a simplex noise x Ray movement Frequency x vertex red: at most
//   1 cm on these instances) are left out.
#import bevy_pbr::{
    pbr_fragment::pbr_input_from_standard_material,
    pbr_functions::main_pass_post_lighting_processing,
    forward_io::{VertexOutput, FragmentOutput},
    mesh_view_bindings::{globals, view},
    prepass_utils::prepass_depth,
    view_transformations::{depth_ndc_to_view_z, position_world_to_view},
}

struct BeamParams {
    // rgb: Color x Emissive_Level.
    color: vec4<f32>,
    // Air Speed, Rotator X, Rotator Y, Ray Length.
    air: vec4<f32>,
    // Panning X, Panning Y, Depth Fade Distance, Cuttoff.
    pan: vec4<f32>,
    // Cutoff Fade, Lerp_A, Fresnel.
    fade: vec4<f32>,
}

@group(#{MATERIAL_BIND_GROUP}) @binding(100) var<uniform> p: BeamParams;
@group(#{MATERIAL_BIND_GROUP}) @binding(101) var air_mask: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(102) var air_sampler: sampler;

@fragment
fn fragment(in: VertexOutput, @builtin(front_facing) is_front: bool) -> FragmentOutput {
    var n = normalize(in.world_normal);
    if !is_front {
        n = -n;
    }
    let v = normalize(view.world_position - in.world_position.xyz);
    let rim = 1.0 - max(dot(n, v), 0.0);

    let angle = globals.time * p.air.x * 0.05;
    let s = sin(angle);
    let c = cos(angle);
    let q = in.uv * p.air.yz - 0.5;
    let turned = vec2<f32>(c * q.x - s * q.y, s * q.x + c * q.y) + 0.5;
    var air = textureSample(air_mask, air_sampler, turned).r * textureSample(air_mask, air_sampler, in.uv * p.pan.xy + angle).r;
#ifdef VERTEX_COLORS
    let alpha = in.color.a;
#else
    let alpha = 1.0;
#endif
    air = air * saturate(alpha * (1.0 + 2.0 * p.air.w) - p.air.w);

    // Its depth and the scene's (cm).
    let depth = -position_world_to_view(in.world_position.xyz).z * 100.0;
    var scene = 1.0e8;
#ifdef DEPTH_PREPASS
    scene = -depth_ndc_to_view_z(prepass_depth(in.position, 0u)) * 100.0;
#endif
    let near = saturate((scene - depth) / max(p.pan.z, 1e-5));
    let cut = pow(max(depth / p.pan.w, 0.0), p.fade.x);
    let k = mix(p.fade.y, air * near, cut);
    // (pow(0, 0) is 1 here; the shader's exp(0 x log 0) is undefined.)
    let f = select(pow(abs(rim), k), 1.0, k <= 0.0) * (1.0 - p.fade.z) + p.fade.z;
    let opacity = saturate(1.0 - f);

    var pbr_input = pbr_input_from_standard_material(in, is_front);
    var out: FragmentOutput;
    out.color = vec4<f32>(max(p.color.rgb, vec3<f32>(0.0)) * view.exposure, opacity);
    out.color = main_pass_post_lighting_processing(pbr_input, out.color);
    return out;
}
