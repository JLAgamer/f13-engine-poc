// Small one-off masters, from their compiled base passes (F13-decomp MODLOG "Emissive masters"). Time is the view's
// time (globals.time); roughness 0.5, metallic 0 and specular 0.5 unless a mode says otherwise.
// Mode 0, the pulsing lights (the phone box's blue and yellow, the CB tower's red): c = sin(1.256637 t) * a + b,
//   emissive = c * max(Power, 0)^2; the phone box's base colour is c too (a.w = 1), the tower's is fixed (StandardMaterial).
// Mode 1, the phone box's red flicker: emissive = saturate(Flicker2.g * 5 - 2) * (1, 0.028928, 0), Flicker2 sampled at
//   uv * (0.001, 0.1) + (FlickerOffset, 1) + t * (0.1, 0.01); base colour black.
// Mode 2, the cabin name signs: roughness = RMA.g, specular 0.1 (the StandardMaterial's reflectance).
#import bevy_pbr::{
    pbr_fragment::pbr_input_from_standard_material,
    pbr_functions::{alpha_discard, apply_pbr_lighting, main_pass_post_lighting_processing},
    forward_io::{VertexOutput, FragmentOutput},
    mesh_view_bindings::globals,
}

#ifdef SCREEN_SPACE_AMBIENT_OCCLUSION
#import bevy_pbr::mesh_view_bindings::screen_space_ambient_occlusion_texture
#endif

struct MasterParams {
    // x: mode; y: Power; z: FlickerOffset
    mode: vec4<f32>,
    a: vec4<f32>,
    b: vec4<f32>,
}

@group(#{MATERIAL_BIND_GROUP}) @binding(100) var<uniform> params: MasterParams;
@group(#{MATERIAL_BIND_GROUP}) @binding(101) var tex: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(102) var tex_sampler: sampler;

@fragment
fn fragment(in: VertexOutput, @builtin(front_facing) is_front: bool) -> FragmentOutput {
#ifdef VISIBILITY_RANGE_DITHER
    // Its cull distance's crossfade (main.rs Placed: the placement's VisibilityRange), as Bevy's own shader does.
    bevy_pbr::pbr_functions::visibility_range_dither(in.position, in.visibility_range_dither);
#endif
    var pbr_input = pbr_input_from_standard_material(in, is_front);
    let mode = u32(params.mode.x);
    let t = globals.time;
    if mode == 0u {
        let c = sin(t * 1.256637) * params.a.rgb + params.b.rgb;
        let power = max(params.mode.y, 0.0);
        pbr_input.material.emissive = vec4(c * power * power, 1.0);
        if params.a.w > 0.0 {
            pbr_input.material.base_color = vec4(c, 1.0);
        }
    }
#ifdef VERTEX_UVS_A
    if mode == 1u {
        let uv = in.uv * vec2(0.001, 0.1) + vec2(params.mode.z, 1.0) + t * vec2(0.1, 0.01);
        let flicker = saturate(textureSample(tex, tex_sampler, uv).g * 5.0 - 2.0);
        pbr_input.material.emissive = vec4(flicker * vec3(1.0, 0.028928, 0.0), 1.0);
        pbr_input.material.base_color = vec4(0.0, 0.0, 0.0, 1.0);
    }
    if mode == 2u {
        pbr_input.material.perceptual_roughness = saturate(textureSample(tex, tex_sampler, in.uv).g);
    }
#endif
    pbr_input.material.base_color = alpha_discard(pbr_input.material, pbr_input.material.base_color);
#ifdef SCREEN_SPACE_AMBIENT_OCCLUSION
    // The ambient light's occlusion as the engine's ambient pass has it (FPostProcessAmbientPS): the screen-space AO
    // (ssao.rs), on its diffuse and specular alike.
    let ao = textureLoad(screen_space_ambient_occlusion_texture, vec2<i32>(in.position.xy), 0i).r;
    pbr_input.diffuse_occlusion = vec3(ao);
    pbr_input.specular_occlusion = ao;
#endif
    var out: FragmentOutput;
    out.color = apply_pbr_lighting(pbr_input);
    out.color = main_pass_post_lighting_processing(pbr_input, out.color);
    return out;
}
