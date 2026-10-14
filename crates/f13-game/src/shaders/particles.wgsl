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
    mesh_view_bindings::{globals, view, view_transmission_texture, view_transmission_sampler},
    mesh_view_bindings as view_bindings,
    mesh_view_types,
    clustered_forward as clustering,
    shadows,
    lighting,
}
#ifdef DEPTH_PREPASS
#import bevy_pbr::prepass_utils::prepass_depth
#endif
#ifdef DISTANCE_FOG
#import bevy_pbr::{mesh_view_bindings::fog, pbr_functions::apply_fog}
#endif

struct ParticleParams {
    // x: mode (0 stand-in, 1 M_Procedural_Fire_01, 2 M_Procedural_Fire_02, 3 M_MasterParticle_Emissive,
    // 4 M_Fly_Animated_01, 5 M_MasterParticle_Diffuse_nearFade, 6 M_Procedural_Smoke_05, 7 M_Rain_Master,
    // 8 M_Procedural_Tire_Dust_01_Inst, 9 M_Procedural_Smoke_01B_no-self-shadow, 10 its
    // _separate-transparency_off sibling, 11 M_Enviroment_Fog_02, 12 M_Savini_Jason_Ashes_01,
    // 13 M_Blood_Burst_01); y: additive
    mode: vec4<f32>,
    // The master's material uniform buffer as its compiled shader reads it (uniexpr.py's cb1[n]).
    p: array<vec4<f32>, 32>,
}

@group(#{MATERIAL_BIND_GROUP}) @binding(0) var<uniform> params: ParticleParams;
@group(#{MATERIAL_BIND_GROUP}) @binding(1) var t0: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(2) var s: sampler;
@group(#{MATERIAL_BIND_GROUP}) @binding(3) var t1: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(4) var t2: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(5) var t3: texture_2d<f32>;
// A batch's sprites (particles.rs GpuSprite: six vec4s each); a placeholder for the others.
@group(#{MATERIAL_BIND_GROUP}) @binding(6) var<storage, read> sprites: array<vec4<f32>>;

#ifdef PARTICLE_SPRITES
struct Vertex {
    @builtin(instance_index) instance_index: u32,
    // (sprite, corner, 0): the batch's quad list carries them (vertex_index leaves out the mesh's base vertex in
    // indirect draws on some backends).
    @location(0) position: vec3<f32>,
}
#else
struct Vertex {
    @builtin(instance_index) instance_index: u32,
    @location(0) position: vec3<f32>,
    @location(1) uv: vec2<f32>,
    @location(2) color: vec4<f32>,
    @location(3) dynamic: vec4<f32>,
}
#endif

struct VertexOutput {
    @builtin(position) clip: vec4<f32>,
    @location(0) world_position: vec3<f32>,
    @location(1) uv: vec2<f32>,
    @location(2) color: vec4<f32>,
    @location(3) dynamic: vec4<f32>,
}

#ifdef RAIN_ANALYTIC
// The rain grid's GPU emitter (particles.rs RainGpu): `sprites` holds the emitter (header: [0] clock, rate, slots,
// cells; [1] lifetime min / max, size min; [2] / [3] start location min / max and size max; [4] / [5] start velocity
// min / max and the pivot; [6] the constant acceleration; [7..71] colour and alpha over relative time), then two
// vec4s a cell (its place, Unreal cm, and start; whether it draws). Slot k of a cell holds the latest particle spawned
// at index n = k (mod slots): born at start + n / rate, alive for its lifetime; the rest draw nothing.
const RAIN_HEADER: u32 = 71u;
const RAIN_CURVE: u32 = 64u;

struct RainSprite {
    a: vec4<f32>,
    b: vec4<f32>,
    dir: vec4<f32>,
    color: vec4<f32>,
}

fn rain_hash(x: u32) -> f32 {
    var s = x * 747796405u + 2891336453u;
    let w = ((s >> ((s >> 28u) + 4u)) ^ s) * 277803737u;
    return f32((w >> 22u) ^ w) / 4294967296.0;
}

fn rain_rand(n: u32, cell: u32, k: u32) -> f32 {
    return rain_hash(n * 16u + k + cell * 0x9E3779B9u);
}

// Unreal space (cm, Z up) to Bevy's (m, Y up).
fn rain_to_bevy(p: vec3<f32>) -> vec3<f32> {
    return vec3(p.x, p.z, p.y) * 0.01;
}

fn rain_sprite(index: u32) -> RainSprite {
    var out: RainSprite;
    out.a = vec4(0.0);
    out.b = vec4(0.0);
    out.dir = vec4(0.0, 0.0, 0.0, 1.0);
    out.color = vec4(0.0);
    let h = sprites[0];
    let slots = max(u32(h.z), 1u);
    let c = index / slots;
    let k = index % slots;
    let place = sprites[RAIN_HEADER + 2u * c];
    let draws = sprites[RAIN_HEADER + 2u * c + 1u];
    let m = floor((h.x - place.w) * h.y);
    if draws.x < 0.5 || m < f32(k) {
        return out;
    }
    let mi = u32(m);
    let n = mi - (mi - k) % slots;
    let age = h.x - (place.w + f32(n) / h.y);
    let life_r = sprites[1];
    let life = mix(life_r.x, life_r.y, rain_rand(n, c, 0u));
    if age < 0.0 || age >= life {
        return out;
    }
    let l0 = sprites[2];
    let l1 = sprites[3];
    let v0 = sprites[4];
    let v1 = sprites[5];
    let acc = sprites[6].xyz;
    let loc = mix(l0.xyz, l1.xyz, vec3(rain_rand(n, c, 1u), rain_rand(n, c, 2u), rain_rand(n, c, 3u)));
    let vel = mix(v0.xyz, v1.xyz, vec3(rain_rand(n, c, 4u), rain_rand(n, c, 5u), rain_rand(n, c, 6u)));
    let size = mix(vec2(life_r.z, life_r.w), vec2(l0.w, l1.w), vec2(rain_rand(n, c, 7u), rain_rand(n, c, 8u)));
    let p = place.xyz + loc + vel * age + 0.5 * acc * age * age;
    let v = vel + acc * age;
    let t = clamp(age / life, 0.0, 1.0) * f32(RAIN_CURVE - 1u);
    let j = min(u32(t), RAIN_CURVE - 2u);
    out.color = mix(sprites[7u + j], sprites[8u + j], t - f32(j));
    out.a = vec4(rain_to_bevy(p), 0.0);
    out.b = vec4(size * 0.01, v0.w, v1.w);
    let d = rain_to_bevy(v);
    out.dir = vec4(select(vec3(0.0), normalize(d), dot(d, d) > 0.0), 1.0);
    return out;
}
#endif

@vertex
fn vertex(v: Vertex) -> VertexOutput {
    var out: VertexOutput;
#ifdef PARTICLE_SPRITES
    // A batched sprite's corner, as particles.rs sprite_quad places it on the CPU for the unbatched emitters: its
    // centre, size x (corner + pivot) along the alignment's axes turned by its rotation; its sub-image's cell.
    let base = u32(v.position.x) * 6u;
    let k = u32(v.position.y);
    let corner = vec2(select(0.0, 1.0, k == 1u || k == 2u), select(0.0, 1.0, k >= 2u));
#ifdef RAIN_ANALYTIC
    let rs = rain_sprite(u32(v.position.x));
    let a = rs.a;
    let b = rs.b;
    let dir = rs.dir;
    let cell = vec4(0.0, 0.0, 1.0, 1.0);
#else
    let a = sprites[base];
    let b = sprites[base + 1u];
    let dir = sprites[base + 4u];
    let cell = sprites[base + 5u];
#endif
    let center = a.xyz;
    let view_right = view.world_from_view[0].xyz;
    let view_up = view.world_from_view[1].xyz;
    let to_view = view.world_position - center;
    var right = view_right;
    var up = view_up;
    if dir.w == 1.0 {
        // PSA_Velocity: along its velocity, turned to the camera.
        up = select(view_up, dir.xyz, dot(dir.xyz, dir.xyz) > 0.0);
        let r = cross(up, to_view);
        right = select(view_right, normalize(r), dot(r, r) > 1e-12);
    } else if dir.w == 2.0 {
        // EPAL_ROTATE_Z: upright, turned about the world's up to face the camera.
        up = vec3(0.0, 1.0, 0.0);
        let r = cross(to_view, up);
        right = select(view_right, normalize(r), dot(r, r) > 1e-12);
    }
    let s = sin(a.w);
    let c = cos(a.w);
    let r = right * c + up * s;
    let u = up * c - right * s;
    let o = corner + b.zw;
    let world = center + r * (o.x * b.x) - u * (o.y * b.y);
    out.clip = position_world_to_clip(world);
    out.world_position = world;
    out.uv = (cell.xy + corner) / cell.zw;
#ifdef RAIN_ANALYTIC
    out.color = rs.color;
    out.dynamic = vec4(0.0);
#else
    out.color = sprites[base + 2u];
    out.dynamic = sprites[base + 3u];
#endif
#else
    let world = (get_world_from_local(v.instance_index) * vec4(v.position, 1.0)).xyz;
    out.clip = position_world_to_clip(world);
    out.world_position = world;
    out.uv = v.uv;
    out.color = v.color;
    out.dynamic = v.dynamic;
#endif
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

// The translucency lighting volume (TLM_VolumetricNonDirectional) on a diffuse colour: the material reads the volume's
// ambient (SH band 0) x 0.886228, and each light's injection (TTranslucentLightingInjectPS, the light function
// materials' shader maps: DefaultLightFunctionMaterial s000-s011) writes LightColor x its attenuation x spot x shadow
// x 0.089794 there, so a light adds diffuse x E / (4 pi), E its light at the point: for point and spot lights
// 1 / (d^2 + 1) x saturate(1 - (d^2 / R^2)^2)^2 (inverse squared) x saturate((cos - cos outer) / range)^2 (as Bevy's
// own attenuation), for the directional light its colour; each x its shadow where it casts one. Evaluated here at the
// pixel rather than stored: the volume's voxels, its blur, its two cascades and their fades and the injection's
// half-voxel step toward the light aren't modelled (PLACEHOLDER). The ambient cubemap's injection adds its mean x
// 0.886228.
fn volume_lit(in: VertexOutput, diffuse: vec3<f32>) -> vec3<f32> {
    let p = vec4(in.world_position, 1.0);
    let view_z = dot(vec4(view.view_from_world[0].z, view.view_from_world[1].z, view.view_from_world[2].z, view.view_from_world[3].z), p);
    let ranges = clustering::unpack_clusterable_object_index_ranges(clustering::view_fragment_cluster_index(in.clip.xy, view_z, false));
    var e = vec3(0.0);
    for (var i: u32 = ranges.first_point_light_index_offset; i < ranges.first_reflection_probe_index_offset; i = i + 1u) {
        let id = clustering::get_clusterable_object_id(i);
        let light = &view_bindings::clustered_lights.data[id];
        let to_light = (*light).position_radius.xyz - p.xyz;
        let d2 = dot(to_light, to_light);
        var a = lighting::getDistanceAttenuation(d2, (*light).color_inverse_square_range.w);
        let l = to_light * inverseSqrt(max(d2, 1e-8));
        let spot = i >= ranges.first_spot_light_index_offset;
        if spot {
            var dir = vec3((*light).light_custom_data.x, 0.0, (*light).light_custom_data.y);
            dir.y = sqrt(max(0.0, 1.0 - dir.x * dir.x - dir.z * dir.z));
            if ((*light).flags & mesh_view_types::POINT_LIGHT_FLAGS_SPOT_LIGHT_Y_NEGATIVE) != 0u {
                dir.y = -dir.y;
            }
            let c = saturate(dot(dir, l) * (*light).light_custom_data.z + (*light).light_custom_data.w);
            a = a * c * c;
        }
        if a <= 0.0 {
            continue;
        }
        var shadow = 1.0;
        if ((*light).flags & mesh_view_types::POINT_LIGHT_FLAGS_SHADOWS_ENABLED_BIT) != 0u {
            if spot {
                shadow = shadows::fetch_spot_shadow(id, p, l, (*light).shadow_map_near_z, in.clip.xy);
            } else {
                shadow = shadows::fetch_point_shadow(id, p, l, in.clip.xy);
            }
        }
        e = e + (*light).color_inverse_square_range.rgb * a * shadow;
    }
    for (var i: u32 = 0u; i < view_bindings::lights.n_directional_lights; i = i + 1u) {
        let light = &view_bindings::lights.directional_lights[i];
        var shadow = 1.0;
        if ((*light).flags & mesh_view_types::DIRECTIONAL_LIGHT_FLAGS_SHADOWS_ENABLED_BIT) != 0u {
            shadow = shadows::fetch_directional_shadow(i, p, (*light).direction_to_light, view_z, in.clip.xy);
        }
        e = e + (*light).color.rgb * shadow;
    }
    // FInjectAmbientCubemapPS: the ambient cubemap's last mip x AmbientCubemapIntensity x tint into the ambient volume
    // (the level's: lighting.rs's ambient light, its mean), read x 0.886228.
    return diffuse * (e * 0.0795775 + view_bindings::lights.ambient_color.rgb * 0.886228);
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

// M_Rain_Master (the roof drips; lit translucency, s094): t0 Normal1 (its lighting normal, which the volume
// evaluation here doesn't take: PLACEHOLDER), t1 EmissiveTexture1, t2 ScrollTexture1. Emissive EmissiveTexture1 at (uv x UV_EmissiveScale +
// (z_U, z_V)_Emissive_OFFSET) + time x ScrollSpeed_Emissive1, x colour x EmissiveLevel x EmissiveColor; diffuse
// saturate(colour x BaseColor) under the lighting volume; opacity ScrollTexture1 at uv x UV_MasterScale +/- time x
// ScrollSpeed_Texture1 (both), x a sphere mask, saturate(x 0.45) x 0_Alpha_Contrast_Opacity_MASTER x colour.a x
// Rain_MPC's Rain (p[15].x). The refraction (its distortion pass) isn't drawn.
fn rain_master(in: VertexOutput) -> vec4<f32> {
    let p = params.p;
    let uv = in.uv;
    let time = globals.time;
    let base_uv = uv * p[6].z;
    let s1 = time * p[6].y + base_uv;
    let s2 = time * p[6].y - base_uv;
    let euv = vec2(uv.x * p[7].z + p[7].w, uv.y * p[7].z + p[8].x) + time * p[7].y;
    let e = textureSample(t1, s, euv).rgb;
    let emissive = max(e * (in.color.rgb * p[8].y * p[2].xyz) + p[3].xyz, vec3(0.0));
    let diffuse = saturate(in.color.rgb * p[5].xyz);
    let mask = saturate((1.0 - length(uv - 0.5) * p[9].x) * p[10].x);
    let o = saturate(textureSample(t2, s, s1).x * textureSample(t2, s, s2).x * mask * 0.45) * p[10].y * in.color.a;
    return vec4(emissive + volume_lit(in, diffuse), saturate(o * p[15].x));
}

// M_Procedural_Tire_Dust_01_Inst (the cars' tire dust; master M_Procedural_Smoke_04_no-self-shadow; lit translucency,
// the instance's own shader map (0_NRM_switch off), its sprite pass s003): t0 FlowMap1, t1 BaseTexture1 (their red; the flow's red inverted). Two
// flow phases (FlowMap1 at (uv + time x FlowmapN_XY_Time + DynamicParameter.w) x FlowmapN_UV (x 0.8 the first),
// scaled by DynamicParameter.z x 0_FlowMap_Multiply x 5) distort the uv, each phase's BaseTexture1 at that x
// Flowmaps_BasemapTileMultiply + a circle (DynamicParameter.y about time x BasemapN_XY_Time x 2pi) x BasemapN_UV,
// / 0_SmokeTexSize (x 0.8 the first). Their sum / 0_BaseTextureDarken, contrasted by BaseTextureContrast, x 3, at
// least 0.1, x the colour is the diffuse (x (1 - saturate(SPEC)): its Metallic) under the lighting volume; emissive
// colour x 0_EmissiveMasterLevel. Opacity: the sum / 3 contrasted by DynamicParameter.x, x colour.a, x a sphere mask
// (the distorted uv x SphereMaskGlobalDistortionLevel; radius SphereMask_Radius - 0.2 x the distortion), x 0.985,
// contrasted by 0_MasterAlphaContrast, x 0_Alpha_Contrast_Opacity_MASTER, at most 0.9; x the depth fades x colour.a x
// 0_Opacity_MASTER. M_Procedural_Dust_01_self-shadow's sprite pass (s004; per-pixel lit: the volume evaluation here too, PLACEHOLDER) is
// the same graph but its shape isn't x colour.a and the alpha is x 0_BaseContrastLevel: p[15] = (the shape's colour.a
// weight, the alpha's multiplier).
fn tire_dust(in: VertexOutput) -> vec4<f32> {
    let p = params.p;
    let dp = in.dynamic;
    let uv = in.uv;
    let col = in.color;
    let time = globals.time;
    let d = dp.z * p[6].w;
    let d5 = d * 5.0;
    let emissive = max(col.rgb * p[6].x + p[1].xyz, vec3(0.0));
    let base_uv = d * 0.15 + uv;
    let uvm = d * -2.5 + base_uv;
    let f1 = textureSample(t0, s, (time * p[7].x + uv + dp.w) * p[7].yz * 0.8).xy;
    let a2 = uvm + vec2(d5 * (1.0 - f1.x), d5 * f1.y) + base_uv;
    let ang1 = time * p[8].x * 6.283185;
    let c1 = vec2((cos(ang1) * dp.y + uv.x) * p[8].y + dp.w, (sin(ang1) * dp.y + uv.y) * p[8].z + dp.w);
    let base1 = textureSample(t1, s, (a2 * p[7].w + c1) / p[8].w * 0.8).x;
    let f2 = textureSample(t0, s, (time * p[9].x + uv + dp.w) * p[9].yz).xy;
    let b2 = uvm + vec2(d5 * (1.0 - f2.x), d5 * f2.y) + base_uv;
    let ang2 = time * p[9].w * 6.283185;
    let c2 = vec2((cos(ang2) * dp.y + uv.x) * p[10].x + dp.w, (sin(ang2) * dp.y + uv.y) * p[10].y + dp.w);
    let base2 = textureSample(t1, s, (b2 * p[7].w + c2) / p[8].w).x;
    let sum = base1 + base2;
    let base = max(saturate(sum / p[10].z * (p[10].w - p[3].x) + p[3].x) * 3.0, 0.1);
    let diffuse = saturate(base * col.rgb) * (1.0 - saturate(p[11].x));
    let fades = depth_fades(in, p[11].z, p[11].w);
    let opacity = fades.y * saturate(fades.x * col.a) * p[12].x;
    var shape = saturate(sum * 0.333333 * (dp.x * 2.0 + 1.0) - dp.x) * mix(1.0, col.a, p[15].x);
    let m = (a2 * p[7].w + b2 * p[7].w - uv) * p[12].w + uv - 0.5;
    let radius = max(d * -0.2 + p[13].x, 0.00001);
    shape = shape * saturate((1.0 - length(m) / radius) * p[14].x) * 0.985;
    shape = clamp(saturate(shape * (p[14].y - p[5].x) + p[5].x) * p[14].z, 0.0, 0.9);
    return vec4(emissive + volume_lit(in, diffuse), saturate(shape * opacity * p[15].y));
}

// M_Procedural_Smoke_01B_no-self-shadow (the hood smoke, the fuse boxes' and the phone box's flashes, the shotgun's
// blast, the electric shocks; lit translucency, the master's shader map: no instance has its own; sprite pass s003):
// t0 DistortionTexture, t1..t3 Base_ALPHA_Texture1..3 (their red). Two circling DistortionTexture samples (the second
// mirrored) summed, ^ z_DistortionBaseMapPower x DynamicParameter.z x 0_DistortionMultiplier clamped to
// DistortionMin..Max, offset Texture1 and Texture2, each pulsing in scale ((cos / 2 + 6) / 7 x
// Base_ALPHA_TextureN_Scale x BaseMask_Master_Scale; the second on the mirrored uv) and turning; x Texture3 (circling,
// clamped to Texture3_Min..Max) is the shape. A sphere mask (offset by the distortion ^ z_DistortionSphereMaskPower).
// Diffuse: mask x shape x 3 contrasted by Diffuse_Contrast, clamped to Diffuse_Min..Max, contrasted by 0_BaseContrast,
// x the colour (under the lighting volume); emissive that x the depth fades x Emissive_Multiplier. Opacity: (shape x
// mask - DynamicParameter.x / 26) contrasted by DynamicParameter.x, x 2 x the fades, contrasted by 0_AlphaContrast,
// x 0_Opacity_Multiplier, ^ 0_AlphaPower, at most 1.
fn smoke_01b(in: VertexOutput) -> vec4<f32> {
    let p = params.p;
    let dp = in.dynamic;
    let uv = in.uv;
    let col = in.color;
    let time = globals.time;
    let scroll = time * p[15].x;
    let a = scroll * p[15].y * 6.283185;
    let b = scroll * p[15].w * 6.283185;
    let d1 = textureSample(t0, s, (vec2(cos(a), sin(a)) * 0.75 + uv) * p[15].z + dp.w).xy;
    let d2 = textureSample(t0, s, 1.0 - ((vec2(cos(b), sin(b)) * -0.75 + uv) * p[16].x + dp.w)).xy;
    let lg = log(max(d1 + d2, vec2(0.0)));
    let amount = dp.z * p[16].z;
    let dist = clamp(amount * exp(lg * p[16].y), vec2(p[17].x), vec2(p[16].w));
    let rot = time * p[17].y;
    let a1 = (rot + dp.y + rot * 1.052632 + dp.w * 10.0) * 0.75;
    let pulse1 = (cos((rot + dp.w) * 3.141593) * 0.5 + 6.0) * 0.142857;
    let pulse2 = (cos((rot + dp.w) * 3.926991) * 0.5 + 6.0) * 0.142857;
    let scale1 = pulse1 * p[5].xy;
    let uv1 = uv * 0.95 / scale1 + 0.5 - 0.5 / scale1 - 0.5;
    let tex1 = textureSample(t1, s, dist + turn(uv1, sin(a1), cos(a1)) + 0.5).x;
    let a2 = ((rot + dp.y) + rot + dp.w * 10.0) * -0.6;
    let scale2 = pulse2 * p[8].xy;
    let uv2 = (1.0 - uv) / scale2 + 0.5 - 0.5 / scale2 - 0.5;
    let tex2 = textureSample(t2, s, dist + turn(uv2, sin(a2), cos(a2)) + 0.5).x;
    let a3 = rot * p[17].w * 6.283185;
    let tex3 = clamp(textureSample(t3, s, (vec2(cos(a3), sin(a3)) * 0.75 + uv) * p[18].x + dp.w).x, p[18].z, p[18].y);
    let shape = tex1 * tex2 * tex3;
    let amount2 = amount * p[19].x;
    let centre = amount2 * vec2(0.613497, 0.333333) + p[19].yz;
    let len = length(exp(lg * p[18].w) * amount2 + uv - centre);
    let radius = max(clamp(-amount2 * 0.1 + p[19].w, 0.0, 0.7), 0.00001);
    let mask = saturate((1.0 - len / radius) * p[20].w);
    var base = saturate(mask * shape * 3.0 * (p[21].x - p[9].x) + p[9].x);
    base = saturate(clamp(base, p[21].z, p[21].y) * (p[21].w - p[10].x) + p[10].x);
    let diffuse = base * col.rgb;
    let f = depth_fades(in, p[22].y, p[22].z);
    let fade = saturate(f.x * col.a) * f.y;
    let emissive = max(diffuse * fade * p[22].w + p[11].xyz, vec3(0.0));
    let shaped = saturate((shape * mask - dp.x * 0.038462) * (dp.x * 0.666667 + 1.0) - dp.x * 0.333333);
    var alpha = saturate(2.0 * shaped * fade * (p[23].z - p[13].x) + p[13].x) * p[23].w;
    alpha = min(pow(max(alpha, 0.0), p[24].x), 1.0);
    return vec4(emissive + volume_lit(in, saturate(diffuse)), alpha);
}

// M_Procedural_Smoke_01B_no-self-shadow_separate-transparency_off (the broken phone box's smoke; its own master,
// s003): smoke_01b's graph but each DistortionTexture sample ^ DistortionTexturePower before the sum (x dp.z x
// 0_DistortionMultiplier, clamped DistortionMin..Max for the offsets); the mask at (that x
// SphereMask_Distotrion_Multiplier + uv - a / 16 - 0.5) over clamp(SphereMask_Radius - a / 30, 0, 0.7); opacity
// 2 x shaped through 0_AlphaContrast x 0_Opacity_Multiplier, saturate(x the fades) (no power).
fn smoke_01b_sto(in: VertexOutput) -> vec4<f32> {
    let p = params.p;
    let dp = in.dynamic;
    let uv = in.uv;
    let col = in.color;
    let time = globals.time;
    let scroll = time * p[15].x;
    let a = scroll * p[15].y * 6.283185;
    let b = scroll * p[16].x * 6.283185;
    let d1 = pow(max(textureSample(t0, s, (vec2(cos(a), sin(a)) * 0.75 + uv) * p[15].z + dp.w).xy, vec2(0.0)), vec2(p[15].w));
    let d2 = pow(max(textureSample(t0, s, 1.0 - ((vec2(cos(b), sin(b)) * -0.75 + uv) * p[16].y + dp.w)).xy, vec2(0.0)), vec2(p[15].w));
    let amount = dp.z * p[16].z;
    let d = amount * (d1 + d2);
    let dist = clamp(d, vec2(p[17].x), vec2(p[16].w));
    let rot = time * p[17].y;
    let a1 = (rot + dp.y + rot * 1.052632 + dp.w * 10.0) * 0.75;
    let pulse1 = (cos((rot + dp.w) * 3.141593) * 0.5 + 6.0) * 0.142857;
    let pulse2 = (cos((rot + dp.w) * 3.926991) * 0.5 + 6.0) * 0.142857;
    let scale1 = pulse1 * p[5].xy;
    let uv1 = uv * 0.95 / scale1 + 0.5 - 0.5 / scale1 - 0.5;
    let tex1 = textureSample(t1, s, dist + turn(uv1, sin(a1), cos(a1)) + 0.5).x;
    let a2 = ((rot + dp.y) + rot + dp.w * 10.0) * -0.6;
    let scale2 = pulse2 * p[8].xy;
    let uv2 = (1.0 - uv) / scale2 + 0.5 - 0.5 / scale2 - 0.5;
    let tex2 = textureSample(t2, s, dist + turn(uv2, sin(a2), cos(a2)) + 0.5).x;
    let a3 = rot * p[17].w * 6.283185;
    let tex3 = clamp(textureSample(t3, s, (vec2(cos(a3), sin(a3)) * 0.75 + uv) * p[18].x + dp.w).x, p[18].z, p[18].y);
    let shape = tex1 * tex2 * tex3;
    let len = length(d * p[18].w + uv - amount * 0.0625 - 0.5);
    let radius = max(clamp(p[19].x - amount * 0.033333, 0.0, 0.7), 0.00001);
    let mask = saturate((1.0 - len / radius) * p[20].x);
    var base = saturate(mask * shape * 3.0 * (p[20].y - p[9].x) + p[9].x);
    base = saturate(clamp(base, p[20].w, p[20].z) * (p[21].x - p[10].x) + p[10].x);
    let f = depth_fades(in, p[21].z, p[21].w);
    let fade = saturate(f.x * col.a) * f.y;
    let emissive = max(fade * base * p[22].x * col.rgb + p[11].xyz, vec3(0.0));
    let shaped = saturate((shape * mask - dp.x * 0.038462) * (dp.x * 0.666667 + 1.0) - dp.x * 0.333333);
    let opacity = saturate(shaped * 2.0 * (p[22].w - p[13].x) + p[13].x) * p[23].x;
    return vec4(emissive + volume_lit(in, saturate(base * col.rgb)), saturate(fade * opacity));
}

// M_Enviroment_Fog_02 (the phone box's and fuse box's smoke, the fuse box's explosion; lit translucency, the master's
// shader map, sprite pass s003): t0 DistortionTexture (xy x 2 - 1), t1 BaseTexture1, t2 BaseTexture2. uv x
// Base_UV_Scale x dp.z + dp.y, turned by dp.x x z_Master_Time x time (Texture1) and by that x
// z_Base2_Time_Multiplier (Texture2); the distortion on the mirrored uv x Distortion_UV_Scale, turned by time x
// z_Distortion_Time + dp.x, ^ z_DistortionSphereMaskPower, x dp.w x 0_Distotrion_Multiplier_Base clamped
// DistortionMin..Max, offsets both. Their product's luminance x colour x EmissiveMultiplier is the emissive;
// (luminance x colour.r + Base_Add_Subtract) x BaseMultiplier through 0_BaseContrast is the (grey) diffuse under the
// lighting volume. Opacity: the product's red clamped 0_AlphaBaseMin..Max through 0_AlphaContrast x
// 0_Opacity_MASTER, at most 0_Opacity_Max, x (two sphere masks: the distorted one about SM_U/V_Offset + the
// distortion / 30 and / 20, radius SphereMask_Radius - it / 8; the plain one, radius SphereMask_Radius) ^
// SphereMask_Fade_Power, x the depth fade x colour.a x min((pixel depth / 0_PixelDepthDistance) ^ Fade Exponent, 1).
fn fog_02(in: VertexOutput) -> vec4<f32> {
    let p = params.p;
    let dp = in.dynamic;
    let uv = in.uv;
    let col = in.color;
    let time = globals.time;
    let a = dp.x * p[7].x * time;
    let uva = uv * p[7].y * dp.z + dp.y - 0.5;
    let b = time * p[7].z + dp.x;
    let uvb = (1.0 - uv) * p[7].w * dp.z + dp.y - 0.5;
    let d = pow(max(textureSample(t0, s, turn(uvb, sin(b), cos(b)) + 0.5).xy * 2.0 - 1.0, vec2(0.0)), vec2(p[8].x));
    let off = clamp(dp.w * p[8].y * d, vec2(p[8].w), vec2(p[8].z));
    let c1 = textureSample(t1, s, turn(uva, sin(a), cos(a)) + 0.5 + off).rgb;
    let c = a * p[9].x;
    let c2 = textureSample(t2, s, off + turn(uva, sin(c), cos(c)) + 0.5).rgb;
    let prod = c1 * c2;
    let lum = dot(prod, vec3(0.3, 0.59, 0.11));
    let emissive = max(lum * col.rgb * p[9].y + p[1].xyz, vec3(0.0));
    let base = saturate((lum * col.r + p[10].x) * p[10].y * (p[10].z - p[3].x) + p[3].x);
    var alpha = saturate(clamp(prod.r, p[11].z, p[11].y) * (p[11].w - p[5].x) + p[5].x);
    alpha = clamp(alpha * p[12].x, 0.0, p[12].y);
    let m1 = saturate((1.0 - length(d * (dp.w * p[12].z) + uv - p[6].xy) * p[15].w) * p[16].w);
    let m2 = saturate((1.0 - length(uv - 0.5) * p[17].y) * p[16].w);
    alpha = alpha * pow(m1 * m2, p[17].z);
    let own = -position_world_to_view(in.world_position).z * 100.0;
    let f = depth_fades(in, p[18].x, 0.0);
    let pixel = select(min(pow(max(own / p[18].y, 0.0), p[18].z), 1.0), 1.0, p[18].y <= 0.0);
    let fade = saturate(f.x * col.a) * pixel;
    return vec4(emissive + volume_lit(in, vec3(base)), saturate(fade * alpha));
}

// M_Savini_Jason_Ashes_01 (Jason's embers, their smoke and distortion sprites in the intro; lit translucency, the
// master's shader map, sprite pass s003): t0 ScrollTexture1, t1 EmissiveTexture1, t2 BaseTextureMask1 (Normal1 only
// feeds the lighting normal). ScrollTexture1 twice: uv x (TexCoordRand1_U, _V) x UV_MasterScaleMultiplier + dp.w +
// time x dp.z x the TexCoordRand1 speeds x ScrollSpeed_Texture1, turned by (dp.w + ScrollTexture1a_RotationAngle)
// turns about that same value (and its b twin with TexCoordRand2); a x Texture1a_Multiplier and b x
// Texture1b_Multiplier, their sum lerped toward their product by Texture1_Add_Multiply_Lerp, x the mask (uv scaled by
// dp.y x BaseTextureMask_UV_Scale about the centre) x colour.a. Emissive: (that x EmissiveColor x
// EmissiveLevelScrollTexture + EmissiveTexture1 (uv x UV_EmissiveScaleMultiplier + dp.w + the offsets + time x
// ScrollSpeed_Emissive1, turned by dp.w + EmissiveTexture1_RotationAngle) x EmissiveColor x EmissiveLevel) x colour;
// diffuse colour x BaseColor. Opacity: saturate(red x 0.45 x (2 dp.x + 1) - dp.x) x 0_Alpha_Contrast_Opacity_MASTER
// x colour.a x the sphere mask x saturate(mask.r x BaseTextureMask1_Distortion) x mask.r x 0_ALPHA_Opacity_MASTER,
// then the depth fade and the pixel depth fade.
fn ashes(in: VertexOutput) -> vec4<f32> {
    let p = params.p;
    let dp = in.dynamic;
    let uv = in.uv;
    let col = in.color;
    let time = globals.time;
    let tau = 6.283185;
    let a1 = dp.w + p[11].x;
    let u1 = time * (dp.z * p[2].xy) + uv * vec2(p[12].x, p[12].z) * p[12].y + dp.w - a1;
    let uva = turn(u1, sin(a1 * tau), cos(a1 * tau)) + a1;
    let a2 = dp.w + p[13].x;
    let u2 = time * (dp.z * p[4].xy) + uv * vec2(p[13].w, p[14].x) * p[12].y + dp.w - a2;
    let uvb = turn(u2, sin(a2 * tau), cos(a2 * tau)) + a2;
    let ae = (dp.w + p[16].x) * tau;
    let ue = uv * p[15].y + dp.w + p[15].zw + time * p[15].x - 0.5;
    let em = textureSample(t1, s, turn(ue, sin(ae), cos(ae)) + 0.5).rgb;
    let ta = textureSample(t0, s, uva).rgb * p[16].z;
    let tb = textureSample(t0, s, uvb).rgb * p[16].w;
    let sc = dp.y * p[17].y;
    let m = textureSample(t2, s, uv * sc - (sc - 1.0) * 0.5).rgb * col.a;
    let t = mix(ta + tb, ta * tb, p[17].x) * m;
    let emissive = max((t * p[6].xyz * p[17].z + em * p[7].xyz) * col.rgb + p[8].xyz, vec3(0.0));
    let diffuse = saturate(col.rgb * p[10].xyz);
    var alpha = saturate(t.r * 0.45 * (dp.x * 2.0 + 1.0) - dp.x) * p[17].w * col.a;
    let sphere = saturate((1.0 - length(uv - 0.5) * p[18].z) * p[19].z);
    alpha = alpha * sphere * saturate(m.r * p[19].w) * m.r * p[20].x;
    let f = depth_fades(in, p[20].z, p[20].w);
    return vec4(emissive + volume_lit(in, diffuse), saturate(f.x * alpha) * f.y);
}

// M_Blood_Burst_01 (the weapon hits' blood: P_Hit_Blood_Counselor/Jason_Default_01; lit translucency, the master's
// shader map, sprite pass s003): t0 DIFF (its red; NRM only feeds the lighting normal). The flipbook: phase =
// frac((time x FLIPBOOK_Global_Time + the particle's time) x FLIPBOOK_Master_Speed), moved toward DynamicParameter.w by
// DynamicFrameAnimation_ON_OFF (1 in every instance drawn, so the particle's time, not passed here, drops out), cell =
// floor(phase x (columns x rows, rows)) / (columns, rows) added to uv / (columns, rows) (the texture's wrap picks the
// column), by 0_FLIPBOOK_PLAY_ON_OFF. Mask = DIFF.r; colour = colour x BloodColor x saturate(mask x (2 EHC + 1) -
// EHC); emissive that x EmmissiveMultiplier, diffuse it saturated x (1 - MET). Opacity: saturate((mask - dp.x / 19) x
// (1 + 4 dp.x / 3) - 2 dp.x / 3) x (dp.x + 1.5), clamped 0..4, x saturate(mask x colour.a x the depth fade) x the
// pixel depth fade, x 0_Opacity_MASTER, saturated.
// M_Gradient_Graphic_Ring_01 (Jason's sound blips; its sprite base pass s006): emissive max(Normal_Main.rgb x
// ParticleColorParameter.rgb x the particle's alpha + SelectionColor, 0), additive (opacity 1).
fn graphic_ring(in: VertexOutput) -> vec4<f32> {
    let p = params.p;
    let t = textureSample(t0, s, in.uv).rgb;
    return vec4(max(t * p[2].xyz * in.color.a + p[3].xyz, vec3(0.0)), 1.0);
}

fn blood(in: VertexOutput) -> vec4<f32> {
    let p = params.p;
    let uv = in.uv;
    var phase = fract((globals.time * p[8].w) * p[9].x);
    phase = fract(phase + p[9].y * (in.dynamic.w - phase));
    let cell = floor(phase * p[2].xy) / p[1].xy;
    return blood_at(in, mix(uv, uv / p[1].xy + cell, p[9].z));
}

// M_Blood_Squirt_01's permutation: the flipbook UV offset by o = DynamicParameter.y + 0_DistortionMultiplier, plus
// two circling DistortionTexture samples ^ DistortionTexturePower, summed x o, clamped to DistortionMin..Max.
fn blood_squirt(in: VertexOutput) -> vec4<f32> {
    let p = params.p;
    let uv = in.uv;
    let o = in.dynamic.y + p[13].x;
    var phase = fract((globals.time * p[8].w) * p[9].x);
    phase = fract(phase + p[9].y * (in.dynamic.w - phase));
    let cell = floor(phase * p[2].xy) / p[1].xy;
    var fuv = mix(uv, (uv - o) / p[1].xy + cell, p[9].z);
    let t = globals.time * p[13].y;
    let a1 = t * p[13].z * 6.283185;
    let uv1 = vec2(cos(a1) * 0.75 + uv.x, sin(a1) * 0.75 + uv.y) * p[13].w;
    let n1 = pow(max(textureSample(t1, s, uv1).xy, vec2(0.0)), vec2(p[14].x));
    let a2 = t * p[14].y * 6.283185;
    let uv2 = vec2(1.0 - (uv.x - cos(a2) * 0.75) * p[14].z, 1.0 - (uv.y - sin(a2) * 0.75) * p[14].z);
    let n2 = pow(max(textureSample(t1, s, uv2).xy, vec2(0.0)), vec2(p[14].x));
    fuv += clamp((n1 + n2) * o, vec2(p[15].x), vec2(p[14].w));
    return blood_at(in, fuv);
}

// The blood masters' shading at the (flipbook, distorted) UV.
fn blood_at(in: VertexOutput, fuv: vec2<f32>) -> vec4<f32> {
    let p = params.p;
    let dp = in.dynamic;
    let col = in.color;
    let mask = textureSample(t0, s, fuv).r;
    let tint = col.rgb * p[6].xyz * saturate(mask * (p[10].z - p[4].x) + p[4].x);
    let emissive = max(tint * p[10].w + p[7].xyz, vec3(0.0));
    let diffuse = saturate(tint) * (1.0 - saturate(p[11].x));
    let f = depth_fades(in, p[12].x, p[12].y);
    let base = saturate(mask * col.a * f.x) * f.y;
    var shape = saturate((mask - dp.x * 0.052632) * (dp.x * 1.333333 + 1.0) - dp.x * 0.666667) * (dp.x + 1.5);
    shape = clamp(shape, 0.0, 4.0) * base;
    return vec4(emissive + volume_lit(in, diffuse), saturate(shape * p[12].z));
}

// The distortion pass (refraction): TDistortionMeshPS<FDistortMeshAccumulatePolicy> (each refracting material's own,
// F13-decomp MODLOG "Particle refraction") writes its offset, TDistortionApplyScreenPS reads the scene colour there.
// From the material's tangent-space normal and its refraction (IoR - 1, x its mask): the normal into view space
// (the quad's tangent frame: +U, +V, facing), offset = view normal.xy x refraction x (P[0][0], P[0][0] x aspect) x
// the view's size x 0.00023 x (1, -1) (DistortionParams: PLACEHOLDER, UE 4.18's (P[0][0], aspect, width, height):
// the setter wasn't found in the binary), none where |view normal.xy x refraction|^2 < 1e-5; x saturate((the scene's
// depth at the offset - the pixel's) / clamp(|RefractionDepthBias / 2|, 1, 50) - RefractionDepthBias) (cm); the
// apply keeps the pixel where the offset leaves the view. Drawn in Bevy's transmissive pass (the opaque scene's
// colour), each refracting particle on its own rather than summed into one offset buffer.
// The quad's tangent frame (+U, +V and its facing normal) from the screen derivatives of its position and uv.
fn quad_frame(in: VertexOutput) -> mat3x3<f32> {
    let dpx = dpdx(in.world_position);
    let dpy = dpdy(in.world_position);
    let duvx = dpdx(in.uv);
    let duvy = dpdy(in.uv);
    let det = duvx.x * duvy.y - duvx.y * duvy.x;
    let t = normalize((dpx * duvy.y - dpy * duvx.y) / det);
    let b = normalize((dpy * duvx.x - dpx * duvy.x) / det);
    var nrm = normalize(cross(b, t));
    if dot(nrm, view.world_position - in.world_position) < 0.0 {
        nrm = -nrm;
    }
    return mat3x3(t, b, nrm);
}

// Unreal's world axes (Z up) to Bevy's (Y up).
fn from_ue(v: vec3<f32>) -> vec3<f32> {
    return vec3(v.x, v.z, v.y);
}

fn distortion(in: VertexOutput, n: vec3<f32>, refraction: f32, depth_bias: f32) -> vec4<f32> {
    return distortion_world(in, normalize(quad_frame(in) * n), refraction, depth_bias);
}

// The same from a world-space normal (Bevy axes): materials without tangent-space normals.
fn distortion_world(in: VertexOutput, wn: vec3<f32>, refraction: f32, depth_bias: f32) -> vec4<f32> {
    let vn = normalize((view.view_from_world * vec4(wn, 0.0)).xyz);
    let d = vn.xy * refraction;
    if dot(d, d) - 0.00001 < 0.0 {
        discard;
    }
    let size = view.viewport.zw;
    let p00 = view.clip_from_view[0][0];
    var off = d * vec2(p00, p00 * size.x / size.y) * size * vec2(0.00023, -0.00023);
    let uv = (in.clip.xy - view.viewport.xy) / size;
#ifdef DEPTH_PREPASS
    let at = (uv + off) * size + view.viewport.xy;
    let scene = -depth_ndc_to_view_z(prepass_depth(vec4(at, 0.0, 0.0), 0u)) * 100.0;
    let pixel = -position_world_to_view(in.world_position).z * 100.0;
    off = off * saturate((scene - pixel) / clamp(abs(depth_bias * -0.5), 1.0, 50.0) - depth_bias);
#endif
    var uv2 = uv + off;
    if any(uv2 < vec2(0.0)) || any(uv2 > vec2(1.0)) {
        uv2 = uv;
    }
    return vec4(textureSampleLevel(view_transmission_texture, view_transmission_sampler, uv2, 0.0).rgb, 1.0);
}

// M_Rain_Master's distortion pass (sprite s090): a = Normal1 (t0) .r at time x ScrollSpeed - uv x UV_MasterScale, b
// at time x ScrollSpeed + uv x UV_MasterScale; k = (a + b - 1) x colour.a x NRM_Intensity; tangent normal (k, k, k);
// refraction (IoR - 1) x k x Rain_MPC Rain.
fn rain_distortion(in: VertexOutput) -> vec4<f32> {
    let p = params.p;
    let time = globals.time;
    let base_uv = in.uv * p[6].z;
    let a = textureSample(t0, s, time * p[6].y - base_uv).x;
    let b = textureSample(t0, s, time * p[6].y + base_uv).x;
    let k = (a + b - 1.0) * in.color.a * p[6].w;
    return distortion(in, vec3(k), (p[10].z - 1.0) * k * p[15].x, p[10].w);
}

// M_Jason_ThrowingKnife_Trail_02 (the knife's ribbon; its trail-vertex-factory passes s124 base, s120 distortion): uv'
// = ((time x Basemap1_X_Speed_Multiply + u) x Basemap1_U_Coord_Multiply, v x Basemap1_V_Coord_Multiply); t0
// Normal_Main, t1 SpriteTexture (the Paper2D sampler's default, P_ThrowingKnife_Trail_Tile_02), both at uv'; the edge
// mask min(sin(pi u)^0_EdgeTransparencyMultiplier, 1) x min(sin(pi v)^that, 1). Base pass: emissive GlowColor x
// SpriteTexture.rgb, opacity saturate(mask x SpriteTexture.a x OPACITY_Multiplier) (its lighting term x
// DiffuseOverrideParameter: 0). Distortion: normal lerp((0, 0, 1), Normal_Main, NRM_MainTextureIntensity), refraction
// (Index of Refraction - 1) x mask x SpriteTexture.a x that normal's x.
fn trail_uv(in: VertexOutput) -> vec2<f32> {
    let p = params.p;
    return vec2((globals.time * p[4].x + in.uv.x) * p[4].y, in.uv.y * p[4].w);
}

fn trail_mask(in: VertexOutput) -> f32 {
    let k = params.p[5].y;
    let a = min(pow(max(sin(in.uv.x * 3.141593), 0.0), k), 1.0);
    let b = min(pow(max(sin(in.uv.y * 3.141593), 0.0), k), 1.0);
    return a * b;
}

fn knife_trail(in: VertexOutput) -> vec4<f32> {
    let p = params.p;
    let sprite = textureSample(t1, s, trail_uv(in));
    let emissive = max(p[2].xyz * sprite.rgb + p[3].xyz, vec3(0.0));
    return vec4(emissive, saturate(trail_mask(in) * sprite.a * p[5].z));
}

fn knife_trail_distortion(in: VertexOutput) -> vec4<f32> {
    let p = params.p;
    let uv = trail_uv(in);
    let sprite = textureSample(t1, s, uv);
    let nm = textureSample(t0, s, uv).xy * 2.0 - 1.0;
    let n = mix(vec3(0.0, 0.0, 1.0), vec3(nm, sqrt(max(1.0 - dot(nm, nm), 0.0))), p[5].x);
    return distortion(in, n, (p[5].w - 1.0) * trail_mask(in) * sprite.a * n.x, p[6].x);
}

// The water splash masters (forward-lit translucency, lit per pixel by every light: TLM_Surface). Their tangent
// normal nt = Normal (t0) unpacked; k = colour.a x NRM_Intensity; the lighting normal lerp((0, 0, 1), nt, k); the
// Fresnel 0.04 + 0.96 |1 - max(nt . V, 0)|^DiffuseFresnel (the unpacked map against the world view vector, as the
// graph has it); d = that x colour x DiffuseMultiplier, the base colour saturate(d); metallic saturate(MET),
// specular saturate(SPEC), roughness saturate(ROUGH); opacity saturate(mask (t2) x the depth fade (FadeDistance)) x
// the pixel depth fade x colour.a x OpacityMultiplier, saturated. Refraction (IoR - 1) x the lighting normal's x.
// M_Water_Splash_03_masked-blend (s147 base, s141 distortion; and the _02_..._separate-transparency_off master,
// whose uniforms are its): the normal through the quad's tangent frame; emissive saturate(CloudMask (t1) at (0, 0)
// x 2 - 0.5) x colour x EmissiveScrollLevel + d x EmissiveBaseLevel.
// M_Water_Splash_02_masked-blend (s082, s024; Jason's bubbles): the normal taken as a world-space one (no tangent
// frame); emissive saturate(lerp(-EmissiveScroll_Contrast, 1 + it, CloudMask at time x Time_Multiplier x
// UV_Multiplier x (0.5, 1))) x colour x EmissiveScrollBrightnessLevel + d x EmissiveMasterLevel.
fn splash_normal(in: VertexOutput, k: f32) -> array<vec3<f32>, 2> {
    let nm = textureSample(t0, s, in.uv).xy * 2.0 - 1.0;
    let nt = vec3(nm, sqrt(max(1.0 - dot(nm, nm), 0.0)));
    return array(nt, mix(vec3(0.0, 0.0, 1.0), nt, k));
}

fn splash_fresnel(in: VertexOutput, nt: vec3<f32>, power: f32) -> f32 {
    let v = normalize(view.world_position - in.world_position);
    let v_ue = vec3(v.x, v.z, v.y);
    return 0.04 + 0.96 * pow(abs(1.0 - max(dot(nt, v_ue), 0.0)), power);
}

fn splash_lit(in: VertexOutput, n_world: vec3<f32>, base: vec3<f32>, metallic: f32, specular: f32, roughness: f32) -> vec3<f32> {
    var pbr = pbr_types::pbr_input_new();
    pbr.material.base_color = vec4(base, 1.0);
    pbr.material.metallic = metallic;
    pbr.material.reflectance = vec3(sqrt(0.5 * specular));
    pbr.material.perceptual_roughness = roughness;
    pbr.frag_coord = in.clip;
    pbr.world_position = vec4(in.world_position, 1.0);
    pbr.world_normal = n_world;
    pbr.N = n_world;
    pbr.V = calculate_view(pbr.world_position, false);
    return apply_pbr_lighting(pbr).rgb / view.exposure;
}

fn splash_opacity(in: VertexOutput, mask: f32, fade: f32, pixel_fade: f32, multiplier: f32) -> f32 {
    let fades = depth_fades(in, fade, pixel_fade);
    return saturate(saturate(mask * fades.x) * fades.y * in.color.a * multiplier);
}

fn splash_03(in: VertexOutput) -> vec4<f32> {
    let p = params.p;
    let n = splash_normal(in, in.color.a * p[2].x);
    let d = splash_fresnel(in, vec3(n[0].xy, n[0].z), p[2].y) * in.color.rgb * p[2].z;
    let cloud = textureSample(t1, s, vec2(0.0)).x;
    let emissive = saturate(cloud * 2.0 - 0.5) * in.color.rgb * p[3].y + d * p[2].w + p[1].xyz;
    let wn = normalize(quad_frame(in) * normalize(n[1]));
    let lit = splash_lit(in, wn, saturate(d), saturate(p[3].z), saturate(p[3].w), saturate(p[4].x));
    let mask = textureSample(t2, s, in.uv).x;
    return vec4(lit + emissive, splash_opacity(in, mask, p[4].z, p[4].w, p[5].x));
}

fn splash_03_distortion(in: VertexOutput) -> vec4<f32> {
    let p = params.p;
    let n = splash_normal(in, in.color.a * p[2].x)[1];
    return distortion(in, n, (p[5].y - 1.0) * n.x, p[5].z);
}

fn splash_02(in: VertexOutput) -> vec4<f32> {
    let p = params.p;
    let n = splash_normal(in, in.color.a * p[4].x);
    let d = splash_fresnel(in, n[0], p[4].y) * in.color.rgb * p[4].z;
    let speed = globals.time * p[5].z * p[5].w;
    let cloud = textureSample(t1, s, vec2(speed * 0.5, speed)).x;
    let e = saturate(mix(-p[5].x, p[6].x, cloud));
    let emissive = e * in.color.rgb * p[6].y + d * p[4].w + p[3].xyz;
    let lit = splash_lit(in, normalize(from_ue(normalize(n[1]))), saturate(d), saturate(p[6].z), saturate(p[6].w), saturate(p[7].x));
    let mask = textureSample(t2, s, in.uv).x * p[7].z;
    return vec4(lit + emissive, splash_opacity(in, mask, p[8].x, p[8].y, p[8].z));
}

fn splash_02_distortion(in: VertexOutput) -> vec4<f32> {
    let p = params.p;
    let n = splash_normal(in, in.color.a * p[4].x)[1];
    return distortion_world(in, normalize(from_ue(normalize(n))), (p[8].w - 1.0) * n.x, p[9].x);
}

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    let mode = u32(params.mode.x);
    // The distortion passes: the scene's colour as it is (already exposed).
    if mode == 30u {
        return rain_distortion(in);
    }
    if mode == 32u {
        return knife_trail_distortion(in);
    }
    if mode == 34u {
        return splash_03_distortion(in);
    }
    if mode == 36u {
        return splash_02_distortion(in);
    }
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
    } else if mode == 7u {
        c = rain_master(in);
    } else if mode == 8u {
        c = tire_dust(in);
    } else if mode == 9u {
        c = smoke_01b(in);
    } else if mode == 10u {
        c = smoke_01b_sto(in);
    } else if mode == 11u {
        c = fog_02(in);
    } else if mode == 12u {
        c = ashes(in);
    } else if mode == 13u {
        c = blood(in);
    } else if mode == 14u {
        c = blood_squirt(in);
    } else if mode == 15u {
        c = graphic_ring(in);
    } else if mode == 31u {
        c = knife_trail(in);
    } else if mode == 33u {
        c = splash_03(in);
    } else if mode == 35u {
        c = splash_02(in);
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
    if params.mode.y > 0.0 {
        // Additive: UE's base pass scales it by the fog's transmittance only (Color x Fogging.a, no in-scatter, which
        // would light the sprite's black corners as a square). apply_fog is linear in the colour, so its fog-only
        // part (on black) is taken back out.
        let fogged = apply_fog(fog, out, in.world_position, view.world_position, in.clip.xy);
        let inscatter = apply_fog(fog, vec4(0.0, 0.0, 0.0, out.a), in.world_position, view.world_position, in.clip.xy);
        out = vec4(fogged.rgb - inscatter.rgb, out.a);
    } else {
        out = apply_fog(fog, out, in.world_position, view.world_position, in.clip.xy);
    }
#endif
    if params.mode.y > 0.0 {
        // Additive (UE: One, One of colour x opacity), drawn with Bevy's premultiplied AlphaMode::Add.
        out = vec4(out.rgb * out.a, 0.0);
    }
    return out;
}
