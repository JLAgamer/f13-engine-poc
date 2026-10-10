// Cascade sprite particles (particles.rs): camera-facing quads built on the CPU, in world space relative to the
// emitter's entity. Per vertex: the particle's colour, its sub-image UV and its dynamic parameter (the material's
// DynamicParameter). The materials' graphs are ported from their compiled translucent base passes (F13-decomp MODLOG
// "Particle materials"); their lighting terms multiply View.DiffuseOverrideParameter (0 in the game) and drop out.
// Mode 0 is the stand-in for masters not ported (F13_PARTICLE_STANDINS): the first texture times the particle
// colour (PLACEHOLDER).
#import bevy_pbr::{
    pbr_types,
    pbr_functions::{apply_pbr_lighting, calculate_view},
    mesh_functions::get_world_from_local,
    view_transformations::{position_world_to_clip, depth_ndc_to_view_z, position_world_to_view},
    mesh_view_bindings::{globals, view},
}
#ifdef DEPTH_PREPASS
#import bevy_pbr::prepass_utils::prepass_depth
#endif
#ifdef DISTANCE_FOG
#import bevy_pbr::{mesh_view_bindings::fog, pbr_functions::apply_fog}
#endif

struct ParticleParams {
    // x: mode (0 stand-in, 1 M_Procedural_Fire_01, 2 M_Procedural_Fire_02, 3 M_MasterParticle_Emissive,
    // 4 M_Fly_Animated_01, 5 M_MasterParticle_Diffuse_nearFade, 6 M_Procedural_Smoke_05); y: additive
    mode: vec4<f32>,
    // The master's material uniform buffer as its compiled shader reads it (uniexpr.py's cb1[n]).
    p: array<vec4<f32>, 16>,
}

@group(#{MATERIAL_BIND_GROUP}) @binding(0) var<uniform> params: ParticleParams;
@group(#{MATERIAL_BIND_GROUP}) @binding(1) var t0: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(2) var s: sampler;
@group(#{MATERIAL_BIND_GROUP}) @binding(3) var t1: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(4) var t2: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(5) var t3: texture_2d<f32>;

struct Vertex {
    @builtin(instance_index) instance_index: u32,
    @location(0) position: vec3<f32>,
    @location(1) uv: vec2<f32>,
    @location(2) color: vec4<f32>,
    @location(3) dynamic: vec4<f32>,
}

struct VertexOutput {
    @builtin(position) clip: vec4<f32>,
    @location(0) world_position: vec3<f32>,
    @location(1) uv: vec2<f32>,
    @location(2) color: vec4<f32>,
    @location(3) dynamic: vec4<f32>,
}

@vertex
fn vertex(v: Vertex) -> VertexOutput {
    var out: VertexOutput;
    let world = (get_world_from_local(v.instance_index) * vec4(v.position, 1.0)).xyz;
    out.clip = position_world_to_clip(world);
    out.world_position = world;
    out.uv = v.uv;
    out.color = v.color;
    out.dynamic = v.dynamic;
    return out;
}

// UE 4.18 BlackBody(Temp).
fn black_body(temp: f32) -> vec3<f32> {
    let u = (0.860117757 + 1.54118254e-4 * temp + 1.28641212e-7 * temp * temp) / (1.0 + 8.42420235e-4 * temp + 7.08145163e-7 * temp * temp);
    let v = (0.317398726 + 4.22806245e-5 * temp + 4.20481691e-8 * temp * temp) / (1.0 - 2.89741816e-5 * temp + 1.61456053e-7 * temp * temp);
    let x = 3.0 * u / (2.0 * u - 8.0 * v + 4.0);
    let y = 2.0 * v / (2.0 * u - 8.0 * v + 4.0);
    let z = 1.0 - x - y;
    let xyz = vec3(x / y, 1.0, z / y);
    let rgb = vec3(
        dot(vec3(3.240454, -1.537138, -0.498531), xyz),
        dot(vec3(-0.969266, 1.876011, 0.041556), xyz),
        dot(vec3(0.055643, -0.204026, 1.057225), xyz),
    );
    let k = 0.0004 * temp;
    let k2 = k * k;
    return rgb * k2 * k2;
}

// The depth fade and pixel depth fade terms: saturate((scene depth - pixel depth) / FadeDistance) and
// saturate(pixel depth / PixelDepthDistance) (cm; a zero distance gives 1, as the division's infinity saturates).
fn depth_fades(in: VertexOutput, fade_distance: f32, pixel_distance: f32) -> vec2<f32> {
    let own = -position_world_to_view(in.world_position).z * 100.0;
    var scene = 1.0e8;
#ifdef DEPTH_PREPASS
    scene = -depth_ndc_to_view_z(prepass_depth(in.clip, 0u)) * 100.0;
#endif
    let fade = saturate((scene - own) / fade_distance);
    let pixel = select(saturate(own / pixel_distance), 1.0, pixel_distance <= 0.0);
    return vec2(fade, pixel);
}

// M_Procedural_Fire_01 (the campfire's and fireplace's flames): two flow-mapped, scrolling flame tiles; the emissive
// is the black body at (DynamicParameter.y + blackBodyTemp) x their luminance; opacity a sphere mask (distorted by
// the same flow) x the tiles' alpha contrasted by DynamicParameter.x, x the depth fade and the particle's alpha.
fn fire_01(in: VertexOutput) -> vec4<f32> {
    let p = params.p;
    let dp = in.dynamic;
    let uv = in.uv;
    let time = globals.time;
    let k = dp.z * p[2].y;
    let k5 = k * 5.0;
    let uv65 = uv * 0.65;
    let flow1 = textureSample(t0, s, vec2(time * p[2].z + uv65.x + dp.w, time * 0.3 + uv65.y + dp.w) * p[2].w).xy;
    let base = uv + k * 0.15;
    let shifted = base - k * 2.5;
    let d1 = shifted + vec2(k5 * (1.0 - flow1.x) + base.x, k5 * flow1.y + base.y);
    let b1uv = (vec2((time * p[3].y + uv.x) * p[3].w + dp.w, (time * p[4].x + uv.y) * p[4].y + dp.w) + d1 * p[3].x) / p[4].z * 0.9;
    let b1 = textureSample(t1, s, b1uv);
    let flow2 = textureSample(t0, s, vec2(time * p[4].w + uv65.x + dp.w, time * 0.2 + uv65.y + dp.w) * p[5].x).xy;
    let d2 = shifted + vec2(k5 * (1.0 - flow2.x) + base.x, k5 * flow2.y + base.y);
    let b2uv = (d2 * p[3].x + vec2((time * p[5].y + uv.x) * p[5].w + dp.w, (time * p[6].x + uv.y) * p[6].y + dp.w)) / p[6].z;
    let b2 = textureSample(t2, s, b2uv);
    let sum = (b1 + b2) * vec4(0.571429, 0.571429, 0.571429, 0.233333);
    let temp = (dp.y + p[2].x) * dot(saturate(sum.rgb), vec3(0.3, 0.59, 0.11));
    let emissive = max(black_body(temp) * p[6].w + p[1].xyz, vec3(0.0));
    let fades = depth_fades(in, p[7].y, p[7].z);
    let faded = saturate(fades.x * in.color.a) * fades.y;
    let m = uv + p[8].x * ((d1 + d2) * p[3].x - uv) - 0.5;
    let radius = max(p[8].y - k * 0.2, 0.00001);
    let mask = saturate((1.0 - length(m) / radius) * p[9].y);
    let contrast = clamp(saturate(sum.w * (2.0 * dp.x + 1.0) - dp.x) * p[9].z, 0.0, 0.9);
    let alpha = saturate(mask * contrast * p[7].w * faded * p[7].w);
    return vec4(emissive, alpha);
}

// M_Procedural_Fire_02 (the candles): the flame texture (Base_ALPHA_Texture1) clamped to Diffuse_Min/Max x
// Emissive_Multiplier x the particle's colour; opacity its alpha x Texture3 (a smoke tile turning on a circle of
// 0.75, clamped to Texture3_Min/Max) x the particle's alpha x the depth fade.
fn fire_02(in: VertexOutput) -> vec4<f32> {
    let p = params.p;
    let uv = in.uv;
    let d = min(max(p[6].x, 0.0), p[5].w);
    let flame = textureSample(t0, s, uv * 1.25 + d - 0.125);
    let emissive = max(clamp(flame.rgb * 0.98, vec3(p[6].z), vec3(p[6].y)) * p[6].w * in.color.rgb + p[1].xyz, vec3(0.0));
    let angle = globals.time * p[7].z * p[7].w * 6.283185;
    let t3 = textureSample(t1, s, vec2(cos(angle) * 0.75 + uv.x, sin(angle) * 0.75 + uv.y) * p[8].x).x;
    let a = saturate(clamp(t3, p[8].z, p[8].y) * flame.a) * in.color.a;
    let fades = depth_fades(in, p[9].x, p[9].y);
    let o = saturate(fades.x * a) * fades.y * p[9].z;
    let alpha = saturate(o * (p[9].w - p[3].x) + p[3].x);
    return vec4(emissive, alpha);
}

// M_MasterParticle_Emissive (the sparks): ParticleTexture x the particle's colour x EmissiveLevel x the vector
// parameter, x (Texture1 + Texture1_1), each turning on a circle of 0.75 and scrolling with time x TimeMultiplier(_2);
// both x the depth fade (fadeAmount) x the particle's alpha; opacity that x ParticleTexture.a ^ Opacity_Power.
fn emissive_master(in: VertexOutput) -> vec4<f32> {
    let p = params.p;
    let uv = in.uv;
    let base = textureSample(t0, s, uv);
    var c = base.rgb * in.color.rgb * p[4].x * p[2].xyz;
    let time1 = globals.time * p[4].y;
    let a1 = time1 * p[4].z * 6.283185;
    let s1 = textureSample(t1, s, vec2((cos(a1) * 0.75 + uv.x) * p[4].w, p[4].w * (sin(a1) * 0.75 + uv.y) + time1)).rgb;
    let time2 = globals.time * p[5].x;
    let a2 = time2 * p[5].y * 6.283185;
    let s2 = textureSample(t2, s, vec2((cos(a2) * 0.75 + uv.x) * p[5].z, p[5].z * (sin(a2) * 0.75 + uv.y) + time2)).rgb;
    c = c * (s1 + s2);
    let d = depth_fades(in, p[6].x, 0.0).x * in.color.a;
    let emissive = max(c * d + p[3].xyz, vec3(0.0));
    let alpha = saturate(d * pow(max(base.a, 0.0), p[6].y));
    return vec4(emissive, alpha);
}

// M_Fly_Animated_01 (the flies round the lamps; masked, lit: its G-buffer pass with no DBuffer decals): a sphere's
// normal (the particle's position and radius, from the dynamic attribute here) under the flipbook's normal map
// (Normal1); the UV turned 0.5 rad, zoomed by the view depth (/ ScaleByDistance x 0.1, clamped 0.1 to
// MaximumDistanceSize) and stepped through UV_COLUMNS x UV_ROWS at (cos(time x FLIPBOOK_Speed x 2pi) + 1) / 2; masked
// by Texture1's alpha x the particle's alpha x a circle, dithered (DitherTemporalAA) against 0.9; base colour 0.2,
// specular 0. Lit by Bevy as the G-buffer is by the engine.
fn fly(in: VertexOutput) -> vec4<f32> {
    let p = params.p;
    let centre = in.dynamic.xyz;
    let radius = in.dynamic.w;
    let pixel = in.world_position;
    let forward = -normalize(view.world_from_view[2].xyz);
    var n = vec3(0.0, 1.0, 0.0);
    let d = pixel - centre;
    let k = radius * radius - dot(d, d);
    if k > 0.0 {
        n = normalize(pixel - forward * sqrt(k) - centre);
    }
    let up = normalize(view.world_from_view[1].xyz);
    let t = normalize(cross(up, n));
    let b = cross(n, t);
    let q = in.uv - 0.5;
    let r = vec2(dot(vec2(0.877583, 0.479426), q), dot(vec2(-0.479426, 0.877583), q)) + 0.5;
    let depth = -position_world_to_view(pixel).z * 100.0;
    let scale = min(max(depth / p[5].y * 0.1, 0.1), p[5].z);
    var uv = (r / scale + 0.5 - 0.5 / scale) / p[1].xy;
    let f = fract((cos(globals.time * p[6].z * 6.283185) + 1.0) * 0.5);
    uv = uv + floor(f * p[2].xy) / p[1].xy;
    let nm = textureSample(t0, s, uv).xy * 2.0 - 1.0;
    let nt = vec3(nm, sqrt(max(1.0 - dot(nm, nm), 0.0)));
    let normal = normalize(t * nt.x + b * nt.y + n * nt.z);
    let mask = textureSample(t1, s, uv).a * in.color.a;
    let circle = saturate((1.0 - length(q) / clamp(scale * 0.8 - 0.2, 0.05, 0.5)) * 100000.0);
    let xy = in.clip.xy;
    let noise = (fract((2.0 * xy.y + xy.x - 1.5) * 0.2) * 5.0 + fract(dot(vec2(2.408451, 3.253521), xy))) * 0.166667;
    if mask * circle + noise - 0.9 < 0.0 {
        discard;
    }
    var pbr = pbr_types::pbr_input_new();
    pbr.material.base_color = vec4(0.2, 0.2, 0.2, 1.0);
    pbr.material.metallic = 0.0;
    pbr.material.reflectance = vec3(0.0);
    pbr.material.perceptual_roughness = 0.5;
    pbr.frag_coord = in.clip;
    pbr.world_position = vec4(pixel, 1.0);
    pbr.world_normal = n;
    pbr.N = normal;
    pbr.V = calculate_view(pbr.world_position, false);
    return vec4(apply_pbr_lighting(pbr).rgb, 1.0);
}

// The translucency lighting volume (TLM_VolumetricNonDirectional: x 0.886228, the SH band 0 of the injected light)
// on a diffuse colour. PLACEHOLDER: f13-rs has no such volume; Bevy's lighting at the pixel, averaged over facing the
// camera and away from it, stands in for its non-directional light.
fn volume_lit(in: VertexOutput, diffuse: vec3<f32>) -> vec3<f32> {
    var pbr = pbr_types::pbr_input_new();
    pbr.material.base_color = vec4(diffuse, 1.0);
    pbr.material.metallic = 0.0;
    pbr.material.reflectance = vec3(0.0);
    pbr.material.perceptual_roughness = 1.0;
    pbr.frag_coord = in.clip;
    pbr.world_position = vec4(in.world_position, 1.0);
    pbr.V = calculate_view(pbr.world_position, false);
    pbr.N = pbr.V;
    pbr.world_normal = pbr.V;
    let front = apply_pbr_lighting(pbr).rgb;
    pbr.N = -pbr.V;
    pbr.world_normal = -pbr.V;
    let back = apply_pbr_lighting(pbr).rgb;
    return (front + back) * 0.5 / view.exposure;
}

// M_MasterParticle_Diffuse_nearFade (the dust specks, the wispy fog; lit translucency, s090): diffuse =
// saturate(ParticleTexture x the particle's colour).rgb under the lighting volume; opacity its alpha x
// (1 - saturate((1 - view depth / fadeDistance) x 1.111111)) x saturate(depth fade / fadeAmount).
fn near_fade(in: VertexOutput) -> vec4<f32> {
    let p = params.p;
    let t = textureSample(t0, s, in.uv) * in.color;
    let depth = -position_world_to_view(in.world_position).z * 100.0;
    let near = 1.0 - saturate((1.0 - depth * p[3].z) * 1.111111);
    let fade = depth_fades(in, p[4].x, 0.0).x;
    let alpha = saturate(fade * near * t.a);
    return vec4(volume_lit(in, saturate(t.rgb)) + max(p[2].xyz, vec3(0.0)), alpha);
}

fn turn(uv: vec2<f32>, s: f32, c: f32) -> vec2<f32> {
    return vec2(c * uv.x - s * uv.y, s * uv.x + c * uv.y);
}

// M_Procedural_Smoke_05_no-self-shadow (the campfire's smoke; lit translucency, s119, the instance's shader map):
// t0 DistortionTexture, t1 Texture3, t2 Texture1, t3 Texture2 (their red). Texture3 at a circling, flow-distorted UV,
// Texture1 and Texture2 pulsing in scale ((cos(..) / 2 + 6) / 7 x their scales) and turning, contrasted in turn by
// DynamicParameter.x + 0_Texture3Contrast / 0_AlphaContrast / 0_BaseContrast and masked by a distorted sphere; the
// result clamped to BaseMin..BaseMax x the particle's colour is the diffuse (under the lighting volume) and, x the
// colour again x Emissive_Multiplier, the emissive; opacity x DynamicParameter.y x colour.a x the depth fades x 3.
fn smoke_05(in: VertexOutput) -> vec4<f32> {
    let p = params.p;
    let dp = in.dynamic;
    let uv = in.uv;
    let time = globals.time;
    let contrast = dp.x + vec3(p[8].z, p[8].y, p[8].x);
    let a1 = (time * p[8].w + dp.w) * 6.283185;
    let sc1 = vec2(sin(a1), cos(a1));
    let t3uv = vec2(p[9].y * (sc1.y * p[9].x + uv.x) + dp.w, p[9].y * (sc1.x * p[9].x + uv.y) + 2.0 * dp.w);
    let d = dp.z + p[9].z;
    let flow = textureSample(t0, s, vec2(p[9].w * (uv.x - sc1.y * p[9].x + 1.0) + dp.w, p[9].w * (uv.y - sc1.x * p[9].x + 1.0) + 2.0 * dp.w)).xy;
    let tex3 = textureSample(t1, s, d * flow + t3uv).x * 0.375;
    let k = contrast * 2.0 + 1.0;
    let c3 = saturate(tex3 * k.x - contrast.x) * 2.0;
    let a = (time * p[10].z * 1.052632 + dp.w * 10.0) * 0.75;
    let pulse1 = (cos((time * p[10].z + dp.w) * 3.141593) * 0.5 + 6.0) * 0.142857;
    let scale1 = pulse1 * p[3].xy;
    let uv1 = uv / scale1 + 0.5 - 0.5 / scale1 - 0.5;
    let tex1 = textureSample(t2, s, turn(uv1, sin(a), cos(a)) + 0.5).x;
    let b = (time * p[11].y + dp.w * 10.0) * -0.6;
    let pulse2 = (cos((time * p[11].y + dp.w) * 3.141593) * 0.5 + 6.0) * 0.142857;
    let scale2 = pulse2 * p[6].xy;
    let uv2 = (1.0 - uv) / scale2 + 0.5 - 0.5 / scale2 - 0.5;
    let tex2 = textureSample(t3, s, turn(uv2, sin(b), cos(b)) + 0.5).x;
    let shape = tex1 * tex2 * c3;
    let m = d * flow + uv + d * vec2(-0.3, -0.15) - 0.5;
    let radius = max(p[11].z - 0.1 * d, 0.00001);
    let mask = saturate((1.0 - length(m) / radius) * p[12].z);
    let alpha_c = saturate(mask * shape * k.y - contrast.y);
    let opacity = alpha_c * p[14].y;
    let base = clamp(saturate(alpha_c * 0.9 * k.z - contrast.z) * 2.0, p[13].x, p[12].w);
    let diffuse = base * in.color.rgb;
    let emissive = max(diffuse * p[13].y * in.color.rgb + p[7].xyz, vec3(0.0));
    let fades = depth_fades(in, p[13].w, p[14].x);
    let alpha = saturate(saturate(fades.x * dp.y * in.color.a) * fades.y * opacity * 3.0);
    return vec4(emissive + volume_lit(in, saturate(diffuse)), alpha);
}

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    let mode = u32(params.mode.x);
    var c: vec4<f32>;
    if mode == 1u {
        c = fire_01(in);
    } else if mode == 2u {
        c = fire_02(in);
    } else if mode == 3u {
        c = emissive_master(in);
    } else if mode == 5u {
        c = near_fade(in);
    } else if mode == 6u {
        c = smoke_05(in);
    } else if mode == 4u {
        // Already exposed (apply_pbr_lighting).
        c = fly(in);
        c = vec4(c.rgb / view.exposure, 1.0);
    } else {
        let t = textureSample(t0, s, in.uv);
        c = vec4(t.rgb * in.color.rgb, t.a * in.color.a);
    }
    var rgb = c.rgb * view.exposure;
    var out = vec4(rgb, c.a);
#ifdef DISTANCE_FOG
    out = apply_fog(fog, out, in.world_position, view.world_position, in.clip.xy);
#endif
    if params.mode.y > 0.0 {
        // Additive (UE: One, One of colour x opacity), drawn with Bevy's premultiplied AlphaMode::Add.
        out = vec4(out.rgb * out.a, 0.0);
    }
    return out;
}
