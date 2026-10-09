// Lake water (M_Lake_Ripples_Translucent_02 and its instances), from the instance's compiled translucent base pass
// (F13-decomp MODLOG "Water"); the engine's lighting is Bevy's. Unreal space (cm, Z up): p = the world position's XY.
// - Normal: near = lerp(N(WaveTexture01, p / (WaterScale x Small Wave Scale) + s(0.1, 0.1)) + N(WaveTexture02, p /
//   (..) + s(0.1, -0.1) + (0.651340, 0.751638)), (0, 0, 1), NormalPowerNear), s = time x SmallWaveTime; far = lerp((0, 0,
//   1), N(WaveTextureFar01, p / (LArgeWaterScale x LArge Wave Scale) + time x LargeWaveTime x (0.1, 0.2)),
//   NormalPowerFAR); lerp(near, far, min((distance / FarCamerablend)^FarCamerablendContrast, 1)).
// - Colour: lerp(Ocean Water Color, Ocean Water Fresnel Color, saturate(dot((T + B + N) / sqrt 3, V))); foam =
//   foam.r(p / 400 + f(0.1, 0.1)) + foam.g(p / 600 + f(0.1, -0.1)), f = time x EdgeFoamTime; with D the scene's depth
//   behind the water less the water's (cm), lerp(foam x EdgeColor, colour, clamp((D x edgeFade)^edgeContrast_Power, 0,
//   edgeContrast)); plus Reflection Amount x Reflection Cubemap(reflect(-V, N)) x Reflection CubemapTint, saturated.
// - Opacity: (a + saturate(D / epacityDepth) (1 - a))^depthContrast, a = saturate(D / 100), times the cylinder
//   dissolve: MPC_Cylindrical_dissolve, which nothing sets, keeps its zero defaults, and with them the compiled term
//   is exactly 1 (|dot(p, orientation)| = 0 >= CylinderScale.z x 25 = 0 selects 1).
// - metalic, specular, rough as given.
#import bevy_pbr::{
    pbr_fragment::pbr_input_from_standard_material,
    pbr_functions::{apply_pbr_lighting, main_pass_post_lighting_processing, calculate_tbn_mikktspace},
    mesh_view_bindings::{globals, view},
    view_transformations::{depth_ndc_to_view_z, position_world_to_view},
    forward_io::{VertexOutput, FragmentOutput},
}
#ifdef DEPTH_PREPASS
#import bevy_pbr::prepass_utils::prepass_depth
#endif

struct WaterParams {
    // SmallWaveTime, WaterScale x Small Wave Scale, NormalPowerNear, LargeWaveTime
    near: vec4<f32>,
    // LArgeWaterScale x LArge Wave Scale, NormalPowerFAR, FarCamerablend, FarCamerablendContrast
    far: vec4<f32>,
    // EdgeFoamTime, edgeFade, edgeContrast_Power, edgeContrast
    foam: vec4<f32>,
    color: vec4<f32>,
    fresnel: vec4<f32>,
    edge: vec4<f32>,
    cube_tint: vec4<f32>,
    // Reflection Amount, max(epacityDepth, 1e-5), depthContrast
    misc: vec4<f32>,
    // rough, specular, metalic
    surface: vec4<f32>,
}

@group(#{MATERIAL_BIND_GROUP}) @binding(100) var<uniform> params: WaterParams;
@group(#{MATERIAL_BIND_GROUP}) @binding(101) var wave_1: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(102) var tex_sampler: sampler;
@group(#{MATERIAL_BIND_GROUP}) @binding(103) var wave_2: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(104) var wave_far: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(105) var foam_map: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(106) var reflection: texture_cube<f32>;

fn unpack(t: vec4<f32>) -> vec3<f32> {
    let xy = t.rg * 2.0 - 1.0;
    return vec3(xy, sqrt(max(1.0 - dot(xy, xy), 0.0)));
}

// Bevy (metres, Y up) to Unreal (cm, Z up) and back, for directions.
fn ue(v: vec3<f32>) -> vec3<f32> {
    return vec3(v.x, v.z, v.y);
}

@fragment
fn fragment(in: VertexOutput, @builtin(front_facing) is_front: bool) -> FragmentOutput {
    var pbr_input = pbr_input_from_standard_material(in, is_front);
    let world = in.world_position.xyz;
    let p = ue(world).xy * 100.0;
    let time = globals.time;

    // The tangent basis (Unreal's binormal is -B: its normal maps are green-down).
#ifdef VERTEX_TANGENTS
    let tbn = calculate_tbn_mikktspace(normalize(in.world_normal), in.world_tangent);
    let t_axis = tbn[0];
    let b_axis = -tbn[1];
    let n_axis = tbn[2];
#else
    let t_axis = vec3(1.0, 0.0, 0.0);
    let b_axis = vec3(0.0, 0.0, 1.0);
    let n_axis = normalize(in.world_normal);
#endif
    let v = pbr_input.V;

    let s = time * params.near.x;
    let uv = p / params.near.y;
    var near = unpack(textureSample(wave_1, tex_sampler, uv + s * vec2(0.1, 0.1)))
        + unpack(textureSample(wave_2, tex_sampler, uv + s * vec2(0.1, -0.1) + vec2(0.651340, 0.751638)));
    near = mix(near, vec3(0.0, 0.0, 1.0), params.near.z);
    let fs = textureSample(wave_far, tex_sampler, p / params.far.x + time * params.near.w * vec2(0.1, 0.2));
    let far = mix(vec3(0.0, 0.0, 1.0), unpack(fs), params.far.y);
    let dist = length(view.world_position - world) * 100.0;
    let f = min(pow(max(dist / params.far.z, 0.0), params.far.w), 1.0);
    let nt = mix(near, far, f);
    let n = normalize(nt.x * t_axis + nt.y * b_axis + nt.z * n_axis);
    pbr_input.N = n;

    // The depth behind the water (cm): none without the depth prepass (SSAO off), so deep and opaque.
    var depth = 1.0e6;
#ifdef DEPTH_PREPASS
    let scene = -depth_ndc_to_view_z(prepass_depth(in.position, 0u));
    let own = -position_world_to_view(world).z;
    depth = (scene - own) * 100.0;
#endif

    let facing = saturate(dot(normalize(t_axis + b_axis + n_axis), v));
    var color = mix(params.color.rgb, params.fresnel.rgb, facing);
    let ft = time * params.foam.x;
    let foam = textureSample(foam_map, tex_sampler, p * 0.0025 + ft * vec2(0.1, 0.1)).r
        + textureSample(foam_map, tex_sampler, p * 0.001667 + ft * vec2(0.1, -0.1)).g;
    let edge = clamp(pow(max(depth * params.foam.y, 0.0), params.foam.z), 0.0, params.foam.w);
    color = mix(foam * params.edge.rgb, color, edge);
    let r = reflect(-v, n);
    color = saturate(color + params.misc.x * textureSample(reflection, tex_sampler, ue(r)).rgb * params.cube_tint.rgb);

    let a = saturate(depth * 0.01);
    let opacity = min(pow(saturate(depth / params.misc.y) * (1.0 - a) + a, params.misc.z), 1.0);

    pbr_input.material.base_color = vec4(color, opacity);
    pbr_input.material.perceptual_roughness = saturate(params.surface.x);
    pbr_input.material.metallic = saturate(params.surface.z);
    pbr_input.material.reflectance = vec3(sqrt(0.5 * saturate(params.surface.y)));
    // Translucent: the engine's screen-space AO doesn't reach it.
    pbr_input.diffuse_occlusion = vec3(1.0);
    pbr_input.specular_occlusion = 1.0;

    var out: FragmentOutput;
    out.color = apply_pbr_lighting(pbr_input);
    out.color = main_pass_post_lighting_processing(pbr_input, out.color);
    return out;
}
