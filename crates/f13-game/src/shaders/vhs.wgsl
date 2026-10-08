// PP_VHS_GlitchEffect (Jason_Base's TeleportScreenEffect): its compiled FPostProcessMaterialPS (from the material's
// .uexp; its Custom node), constants exact. A post-process material after tonemapping: it reads and writes the
// display-encoded colour. The material's uniforms: OffsetAmount (its one parameter, cb1[2].x) and SelectionColor
// (cb1[1], black in a game). The view's: its inverse size (cb0[119].zw) and its time (cb0[127].z). screen_effects.rs.
#import bevy_core_pipeline::fullscreen_vertex_shader::FullscreenVertexOutput

struct Params {
    // x: OffsetAmount, y: the view's time (s).
    v: vec4<f32>,
}

@group(0) @binding(0) var scene: texture_2d<f32>;
@group(0) @binding(1) var scene_sampler: sampler;
@group(0) @binding(2) var<uniform> p: Params;

fn hash(v: vec2<f32>) -> f32 {
    return fract(sin(dot(v, vec2<f32>(12.989800453186035, 78.23300170898438))) * 43758.546875);
}

// The shader's fmod(x, 1): fract of |x|, x's sign.
fn fmod1(x: f32) -> f32 {
    let f = fract(abs(x));
    return select(-f, f, x >= -x);
}

fn srgb_to_linear(c: vec3<f32>) -> vec3<f32> {
    let lo = c / 12.92;
    let hi = pow((c + 0.055) / 1.055, vec3<f32>(2.4));
    return select(hi, lo, c <= vec3<f32>(0.04045));
}

fn linear_to_srgb(c: vec3<f32>) -> vec3<f32> {
    let lo = c * 12.92;
    let hi = 1.055 * pow(c, vec3<f32>(1.0 / 2.4)) - 0.055;
    return select(hi, lo, c <= vec3<f32>(0.0031308));
}

fn scene_at(uv: vec2<f32>) -> vec3<f32> {
    return linear_to_srgb(max(textureSample(scene, scene_sampler, uv).rgb, vec3<f32>(0.0)));
}

@fragment
fn fragment(in: FullscreenVertexOutput) -> @location(0) vec4<f32> {
    let u = in.uv.x;
    let v = in.uv.y;
    let time = p.v.y;
    let f = fmod1(time * 0.0027777778450399637);
    let t360 = f * 360.0;
    let fl = floor(f * 216.00001525878906);
    let c = cos(t360);
    let h0 = hash(vec2<f32>(t360, t360));
    let k = h0 * abs(c);
    let s = sin(h0 * 360.0);
    let amp = select(s * 0.5, 0.125, s < 0.0);
    let d = v * 100.0 - k * 100.0;
    let w = exp2(d * d * -0.06011229380965233);
    let ux = u - amp * w;
    let vy = fmod1(v + p.v.x);
    let noise = vec3<f32>(hash(vec2<f32>(v, time)), hash(vec2<f32>(v, time + 1.0)), hash(vec2<f32>(v, time + 2.0))) - 0.5;
    let cosang = cos(k * 3.5999999046325684);
    let lo = cosang * 0.800000011920929;
    let hi = hash(vec2<f32>(fl, t360)) * 0.25 + lo;
    let band = lo < v && v < hi;
    let c5 = scene_at(vec2<f32>(ux, vy));
    let jitter = hash(vec2<f32>(t360, vy)) * 0.008700000122189522 * sin(v + v);
    let c6 = scene_at(vec2<f32>(jitter + ux, vy));
    var col = noise * 0.1;
    if band {
        let a = cos(cosang * 0.02879999950528145) * 36.0;
        let b = fmod1(ux * 10.0) * 10.0;
        if sin(a) < b * sin(t360) || 0.4 < hash(vec2<f32>(b, b)) {
            col = noise * 0.1 + (hash(vec2<f32>(fl, fl)) * 0.75 + c5);
        }
    } else {
        let h1 = hash(vec2<f32>(v * t360, t360 * t360));
        let h2 = hash(vec2<f32>(ux, t360));
        let c1 = h1 < 2.0 * fmod1(ux * 20.0) && v < c;
        let c2 = 2.0 * fmod1(v * 6.0) < h2;
        if c1 || c2 {
            if 0.8 < hash(vec2<f32>(t360 + ux, v + t360)) {
                col = noise * 0.1 + hash(vec2<f32>(t360 * ux, v * t360));
            } else {
                col = noise * 0.1 + c5;
            }
        } else {
            col = noise * 0.1 + c6;
        }
    }
    return vec4<f32>(srgb_to_linear(max(col, vec3<f32>(0.0))), 1.0);
}
