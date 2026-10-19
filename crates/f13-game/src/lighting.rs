//! The map's night lighting, from the export: its point and spot lights, the moon and height fog of its weather
//! level (CLsm_RainOff, or CLsm_RainOn when the match rolls rain: weather.rs), the post-process volume's exposure and ambient light, and the counselor's
//! flashlight.
//!
//! Units carry over from Unreal 4.18 as follows:
//! - **Exposure**: the volume fixes it (auto exposure min = max brightness, 1.2), so a pixel is the scene value /
//!   1.2. That's Bevy's exposure at `ev100 = 0`. Tonemapping is the game's own (tonemap.rs).
//! - **Lights**:
//!   - The moon's Intensity is its illuminance (lux).
//!   - An inverse-square light's Intensity is 16 / 100^2 candela ([`inverse_squared_lumens`]): its proxy's Color is
//!     Intensity x LightColor (FLightSceneProxy's constructor 0x14139fd00), the deferred light uniform's fill
//!     (SetDeferredLightParameters 0x140b44a40) multiplies it by 16 when the proxy IsInverseSquared (its virtual
//!     +0x30; DAT_14209aa84 = 16.0), and the game's deferred light shader (TDeferredLightPS in its global shader
//!     cache) lights that LightColor x rcp(distance^2 + 1), distance in cm: the same LightColor the falloff lights
//!     below take as lux.
//!   - A light without inverse-square falloff lights `Intensity * (1 - (d/R)^2)^FalloffExponent` lux at distance
//!     `d` within its radius `R` (DeferredLightingCommon's RadialAttenuation). Bevy has only the inverse-square law,
//!     so its point light shading is patched ([`patch_falloff`]): a light whose sphere radius is negative takes that
//!     falloff, the radius being minus its exponent, and its Bevy intensity is Intensity x 4 pi (lumens over 4 pi:
//!     Intensity lux). (Matched at a third of the radius instead, as before, the lamps lit what's near them up to 30
//!     times too bright.)
//!   - Colours are sRGB; `bUseTemperature` multiplies in the colour temperature (f13-core::light).
//!   - Lights fade out over their last MaxDistanceFadeRange before MaxDrawDistance from the camera, as Unreal fades
//!     them.
//! - **Ambient**: the ambient cubemap's mean radiance (computed by the exporter) × AmbientCubemapIntensity × tint,
//!   as uniform ambient light.
//! - **Fog** (ExponentialHeightFog):
//!   - density FogDensity / 1000 per cm in base 2, thinning by `2^(-FogHeightFalloff/1000 per cm)` with height
//!     above the fog actor, integrated along each view ray (FExponentialHeightFogPS: vendored bevy_pbr's
//!     FogFalloff::HeightFog);
//!   - the transmittance capped below by 1 - FogMaxOpacity;
//!   - inscattering colours as radiance; with a directional light (the clear night's moon, its
//!     bUsedAsAtmosphereSunLight) DirectionalInscatteringColor past DirectionalInscatteringStartDistance, not capped.
//! - **Flashlight**:
//!   - The counselor's Flashlight spot light, toggled with F (CSLR_Flashlight). Hiding turns it off.
//!   - It sits where the counselor constructor attaches it: (-3.5, 50, 88) cm in the mesh's space, i.e. 50 cm in
//!     front of her, 3.5 cm to the right, at hip height.
//!   - It turns toward her facing at FlashlightLerpSpeed (RInterpTo). The game aims it with the flashlight mesh
//!     on her pelvis, which sways with her animations.
//!   - It goes out while a line from her capsule to it is blocked (the game's FlashlightTrace).
//!
//! Not ported:
//! - Enlighten global illumination (the moon's indirect light);
//! - local lights' shadows (only the moon and the flashlight cast them);
//! - the colour grading LUT, ColorContrast and bloom;
//! - rain (CLsm_RainOn);
//! - the power's flicker curves (power.rs turns lights off and on).

use bevy::camera::Exposure;
use bevy::light::{CascadeShadowConfigBuilder, GlobalAmbientLight};
use bevy::pbr::{DistanceFog, FogFalloff};
use bevy::prelude::*;
use f13_core::light::{temperature_color, Light};
use f13_data::classes::ClassDb;
use f13_data::level::{ComponentInfo, Xform};
use serde_json::{Map, Value};

use crate::player::Player;

/// The moon's shadows reach this far (m) when the export has no moon (DynamicShadowDistanceMovableLight's default).
const DEFAULT_SHADOW_DISTANCE_M: f32 = 100.0;
/// SCCounselorCharacter constructor: the flashlight's place in the mesh's space (cm).
const FLASHLIGHT_IN_MESH: Vec3 = Vec3::new(-3.5, 50.0, 88.0);

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum LightKind {
    Point,
    Spot,
    Directional,
}

/// A light the map places, with the placed actor's properties over its class's.
#[derive(Clone, Debug)]
pub struct PlacedLight {
    pub kind: LightKind,
    pub world: Xform,
    pub props: Map<String, Value>,
    /// A Powered{Spot,Point}LightComponent: on while its power grid is (power.rs).
    pub powered: bool,
}

impl PlacedLight {
    pub(crate) fn num(&self, name: &str, default: f32) -> f32 {
        self.props.get(name).and_then(Value::as_f64).map_or(default, |v| v as f32)
    }

    fn flag(&self, name: &str, default: bool) -> bool {
        self.props.get(name).and_then(Value::as_bool).unwrap_or(default)
    }

    pub fn visible(&self) -> bool {
        self.flag("bVisible", true)
    }

    /// The component's name in its actor.
    pub fn name(&self) -> &str {
        self.props.get("ComponentName").and_then(Value::as_str).unwrap_or("")
    }

    /// The power grid that feeds it (a powered light in a grid's box), or that a fuse box's power-out light watches.
    pub fn grid(&self, grids: &crate::power::PowerGrids) -> Option<usize> {
        (self.powered || self.is_power_out()).then(|| grids.grid_at(self.position())).flatten()
    }

    /// A fuse box's light for a power cut (hidden until then).
    pub fn is_power_out(&self) -> bool {
        self.name() == "PowerOutPointLight"
    }

    /// Unreal space (cm, Z up): the light's position and the direction it shines (its +X).
    fn position(&self) -> Vec3 {
        Vec3::from_array(self.world.t)
    }

    fn direction(&self) -> Vec3 {
        Vec3::from_array(self.world.rotate([1.0, 0.0, 0.0])).normalize_or_zero()
    }

    /// Unreal's defaults: AttenuationRadius 1000, LightFalloffExponent 8, cone 0 / 44 degrees.
    pub fn radius(&self) -> f32 {
        self.num("AttenuationRadius", 1000.0)
    }

    pub(crate) fn cone(&self) -> (f32, f32) {
        (self.num("InnerConeAngle", 0.0), self.num("OuterConeAngle", 44.0))
    }

    /// The light as the counselor's light check sees it (f13-core::light).
    pub fn rule_light(&self) -> Light {
        Light {
            position: self.position(),
            direction: self.direction(),
            radius: self.radius(),
            falloff_exponent: self.num("LightFalloffExponent", 8.0),
            cone: (self.kind == LightKind::Spot).then(|| self.cone()),
        }
    }

    /// Linear colour: LightColor (sRGB), times the colour temperature when the light uses it.
    pub(crate) fn color(&self) -> LinearRgba {
        let c = self.props.get("LightColor");
        let byte = |k: &str| c.and_then(|c| c.get(k)).and_then(Value::as_u64).unwrap_or(255) as u8;
        let mut color = LinearRgba::from(Color::srgb_u8(byte("R"), byte("G"), byte("B")));
        if self.flag("bUseTemperature", false) {
            let t = temperature_color(self.num("Temperature", 6500.0));
            color = LinearRgba::rgb(color.red * t[0], color.green * t[1], color.blue * t[2]);
        }
        color
    }

    /// Bevy lumens for a point or spot light (see the module notes).
    pub(crate) fn lumens(&self) -> f32 {
        let intensity = self.num("Intensity", 5000.0);
        if self.inverse_squared() {
            return inverse_squared_lumens(intensity);
        }
        falloff_lumens(intensity)
    }

    fn inverse_squared(&self) -> bool {
        self.flag("bUseInverseSquaredFalloff", true)
    }

    /// Its Bevy sphere radius: 0, or minus its falloff exponent for a light without inverse-square falloff (the
    /// patched shading, [`patch_falloff`]).
    pub(crate) fn sphere_radius(&self) -> f32 {
        if self.inverse_squared() { 0.0 } else { falloff_radius(self.num("LightFalloffExponent", 8.0)) }
    }
}

/// Bevy lumens (a spot's as a point light's: Bevy's cone only masks) for an inverse-square light's Intensity: 16 / 100^2
/// candela per unit, x 4 pi (see the module notes).
pub(crate) fn inverse_squared_lumens(intensity: f32) -> f32 {
    intensity * 16.0 / 10_000.0 * 4.0 * std::f32::consts::PI
}

/// Bevy intensity for a light without inverse-square falloff: its Intensity (lux at its centre) x 4 pi, so the
/// patched shading ([`patch_falloff`]) lights Intensity x (1 - (d/R)^2)^exponent.
pub(crate) fn falloff_lumens(intensity: f32) -> f32 {
    intensity * 4.0 * std::f32::consts::PI
}

/// The sphere radius that marks a light for Unreal's radial falloff: minus its exponent.
pub(crate) fn falloff_radius(exponent: f32) -> f32 {
    -exponent.max(1e-3)
}

/// Unreal 4.18's RadialAttenuation for lights without inverse-square falloff, in Bevy's point light shading
/// (bevy_pbr::lighting, its source patched once it has loaded): a light with a negative sphere radius gets
/// pow(saturate(1 - d^2 / R^2), -radius) in place of the inverse square and its range window; the sphere radius of
/// every light is otherwise taken as at least 0 (only the specular's area light uses it).
fn patch_falloff(mut done: Local<bool>, assets: Res<AssetServer>, mut shaders: ResMut<Assets<bevy::shader::Shader>>, mut handle: Local<Option<Handle<bevy::shader::Shader>>>) {
    if *done {
        return;
    }
    let h = handle.get_or_insert_with(|| assets.load("embedded://bevy_pbr/render/pbr_lighting.wgsl")).clone();
    let Some(shader) = shaders.get(&h) else { return };
    let bevy::shader::Source::Wgsl(source) = &shader.source else {
        *done = true;
        return;
    };
    let mut s = source.to_string();
    let replace = |s: &mut String, a: &str, b: &str, n: usize| {
        let found = s.matches(a).count();
        if found != n {
            error!("lighting: bevy_pbr::lighting isn't as expected ({found} of {a:?}): Unreal's falloff not patched in");
            return false;
        }
        *s = s.replace(a, b);
        true
    };
    let ok = replace(
        &mut s,
        "fn getDistanceAttenuation(distanceSquare: f32, inverseRangeSquared: f32) -> f32 {",
        "fn unrealAttenuation(distanceSquare: f32, inverseRangeSquared: f32, radius: f32) -> f32 {\n    \
             if radius < 0.0 {\n        return pow(saturate(1.0 - distanceSquare * inverseRangeSquared), -radius);\n    }\n    \
             return getDistanceAttenuation(distanceSquare, inverseRangeSquared);\n}\n\n\
         fn getDistanceAttenuation(distanceSquare: f32, inverseRangeSquared: f32) -> f32 {",
        1,
    ) && replace(
        &mut s,
        "let rangeAttenuation = getDistanceAttenuation(distance_square, (*light).color_inverse_square_range.w);",
        "let rangeAttenuation = unrealAttenuation(distance_square, (*light).color_inverse_square_range.w, (*light).position_radius.w);",
        1,
    ) && replace(&mut s, "        (*light).position_radius.w,\n", "        max((*light).position_radius.w, 0.0),\n", 2)
        && replace(&mut s, "let light_radius = (*light).position_radius.w;", "let light_radius = max((*light).position_radius.w, 0.0);", 1);
    *done = true;
    if let (true, Some(mut shader)) = (ok, shaders.get_mut(&h)) {
        // Its imports and their handles kept (the function added imports nothing).
        shader.source = bevy::shader::Source::Wgsl(s.into());
        info!("lighting: Unreal's radial falloff patched into bevy_pbr::lighting");
    }
}

/// Every component the level places whose type `keep` accepts: (type, world transform, properties), the placed
/// actor's own values over its class's; the properties also hold the component's name (`ComponentName`).
pub fn placed_components(level: &crate::Level, classes: &ClassDb, keep: impl Fn(&str) -> bool) -> Vec<(String, Xform, Map<String, Value>)> {
    let mut out = Vec::new();
    // The actor's index in the match's actors (f13-data::world::MatchWorld lists the level's placed actors first, in
    // this order).
    let mut index = 0;
    for a in level.layouts.iter().flat_map(|l| &l.actors) {
        let Some(t) = a.transform else { continue };
        index += 1;
        let class: &[ComponentInfo] = classes.get(&a.class).map_or(&[], |c| &c.components);
        let mut comps: Vec<(&ComponentInfo, Option<&ComponentInfo>)> = class.iter().map(|c| (c, a.components.iter().find(|i| i.name == c.name))).collect();
        comps.extend(a.components.iter().filter(|i| !class.iter().any(|c| c.name == i.name)).map(|i| (i, None)));
        for (c, inst) in comps {
            if !keep(&c.kind) {
                continue;
            }
            let Some(world) = inst.and_then(|i| i.world).or(c.world).or_else(|| c.local.map(|l| l.then(&t))) else { continue };
            let mut props = c.props.as_object().cloned().unwrap_or_default();
            for (k, v) in inst.and_then(|i| i.props.as_object()).into_iter().flatten() {
                if !v.is_null() {
                    props.insert(k.clone(), v.clone());
                }
            }
            props.insert("ComponentName".into(), Value::String(c.name.clone()));
            props.insert("ActorIndex".into(), Value::from(index - 1));
            props.insert("ActorClass".into(), Value::String(a.class.clone()));
            props.insert("ActorName".into(), Value::String(a.name.clone()));
            out.push((c.kind.clone(), world, props));
        }
    }
    out
}

/// The level's lights.
pub fn placed_lights(level: &crate::Level, classes: &ClassDb) -> Vec<PlacedLight> {
    let kind = |k: &str| {
        if k.ends_with("SpotLightComponent") {
            Some(LightKind::Spot)
        } else if k.ends_with("PointLightComponent") {
            Some(LightKind::Point)
        } else if k == "DirectionalLightComponent" {
            Some(LightKind::Directional)
        } else {
            None
        }
    };
    placed_components(level, classes, |k| kind(k).is_some())
        .into_iter()
        .map(|(k, world, props)| PlacedLight { kind: kind(&k).unwrap(), world, props, powered: k.starts_with("Powered") })
        .collect()
}

/// Unreal (cm, Z up) to Bevy (m, Y up).
fn to_bevy(v: Vec3) -> Vec3 {
    Vec3::new(v.x, v.z, v.y) / 100.0
}

/// The height fog, for the camera.
#[derive(Resource, Clone)]
struct HeightFog {
    /// Base-2 extinction per metre at the fog's height.
    density: f32,
    /// Base-2 thinning per metre of height.
    falloff: f32,
    /// The fog actor's height (m).
    height: f32,
    max_opacity: f32,
    inscattering: LinearRgba,
    directional: LinearRgba,
    directional_exponent: f32,
    /// DirectionalInscatteringStartDistance (m; the component's constructor 0x141350280: 10000 cm).
    directional_start: f32,
    /// A directional light with bUsedAsAtmosphereSunLight: the fill (0x140b58325) gives the fog directional
    /// inscattering only from such a light (the clear night's moon; the rain's has none).
    sun: bool,
}

/// A placed light that casts dynamic shadows (its CastShadows and CastDynamicShadows: LightComponentBase's constructor
/// 0x141350740 sets both, with bAffectsWorld and CastStaticShadows). The map has no built static lighting (its
/// materials compile FNoLightMapPolicy; Enlighten gives the indirect light), and its lights are Movable (the Blueprint
/// lamps' components: SceneComponent's constructor 0x141376820 sets Mobility 2) or Stationary without precomputed
/// shadow maps (APointLight / ASpotLight's constructors 0x1414e0ab0 / 0x14173a030 set 1), so every one renders a
/// whole-scene shadow map when it's seen ([`shadow_casters`]).
#[derive(Component)]
struct ShadowCaster;

/// r.Shadow.MinResolution (32; the cvar 0x14014f490, no ini override): a shadow whose unclamped resolution isn't over
/// it fades to nothing (CalculateShadowFadeAlpha; r.Shadow.FadeResolution 64 the top of the fade).
const SHADOW_MIN_RESOLUTION: f32 = 32.0;
/// r.Shadow.FadeResolution (64; the cvar 0x14014fe40): the shadow at full strength from this resolution.
const SHADOW_FADE_RESOLUTION: f32 = 64.0;

/// CalculateShadowFadeAlpha (FUN_140dd3e30): a shadow of unclamped resolution `r`'s strength: 1 from FadeResolution,
/// ((r - MinResolution) / (FadeResolution - MinResolution))^1.25 between, 0 at or below MinResolution.
fn shadow_fade_alpha(r: f32) -> f32 {
    ((r - SHADOW_MIN_RESOLUTION) / (SHADOW_FADE_RESOLUTION - SHADOW_MIN_RESOLUTION)).clamp(0.0, 1.0).powf(1.25)
}
/// r.Shadow.TexelsPerPixelPointlight (1.27324) and ...Spotlight (2.54648): texels per screen pixel of the light's
/// radius (the cvars 0x14014ff40 / 0x14014ffc0).
const TEXELS_PER_PIXEL_POINT: f32 = 1.27324;
const TEXELS_PER_PIXEL_SPOT: f32 = 2.54648;
/// How many of those render shadow maps at once: the nearest whose range reaches the camera. PLACEHOLDER: the game
/// caches its whole-scene shadows (r.Shadow.CacheWholeSceneShadows 1: a light's static casters drawn once, only movers
/// each frame); Bevy re-renders every shadow map each frame, and the ten or so a cabin's lamps qualify for (six cube
/// faces each) cost seconds a frame, so a budget stands in for that caching. The four highest resolutions cost
/// 3.6 ms a frame more than the nearest four in reach (the played Jason demo: 17.3 against 13.7 ms, 13.0 with none), so a
/// lamp seen from beyond its reach goes without.
const SHADOW_BUDGET: usize = 4;

/// The shadow budget, or another for measuring (`F13_SHADOW_BUDGET=<n>`).
fn shadow_budget() -> usize {
    static BUDGET: std::sync::OnceLock<usize> = std::sync::OnceLock::new();
    *BUDGET.get_or_init(|| std::env::var("F13_SHADOW_BUDGET").ok().and_then(|v| v.parse().ok()).unwrap_or(SHADOW_BUDGET))
}

/// The moon's shadow cascades, or another count for measuring (`F13_SUN_CASCADES=<n>`, 0 none).
fn moon_cascades() -> usize {
    std::env::var("F13_SUN_CASCADES").ok().and_then(|v| v.parse().ok()).unwrap_or(3)
}

/// A placed light's distance fade: MaxDrawDistance and MaxDistanceFadeRange (m), and its full intensity.
#[derive(Component)]
struct DrawDistance {
    max: f32,
    fade: f32,
    lumens: f32,
}

#[derive(Component)]
pub(crate) struct Lit;

/// The counselor's flashlight values (her character export's Values).
#[derive(Resource, Clone, Copy)]
struct FlashlightTuning {
    lumens: f32,
    color: LinearRgba,
    range: f32,
    inner: f32,
    outer: f32,
    lerp_speed: f32,
}

/// The counselor's flashlight: on or off, and the yaw it's turned to (radians, Bevy).
#[derive(Component)]
pub struct Flashlight {
    pub on: bool,
    yaw: f32,
    light: Entity,
}

pub struct LightingPlugin {
    /// --flashlight: she starts with it on.
    pub flashlight: bool,
    /// --unlit: no lights, shadows or fog, flat white ambient instead (for looking at things in test screenshots).
    pub unlit: bool,
}

/// --unlit's flat ambient (cd/m^2 per unit albedo, before the 1/1.2 exposure).
const UNLIT_AMBIENT: f32 = 1.2;
/// --unlit's directional light (lux).
const UNLIT_SUN_LUX: f32 = 2.0;

#[derive(Resource)]
pub(crate) struct Unlit;

#[derive(Resource)]
struct FlashlightAtStart(bool);

impl Plugin for LightingPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(FlashlightAtStart(self.flashlight && !self.unlit));
        if self.unlit {
            app.insert_resource(Unlit);
        }
        app.add_systems(Startup, spawn_lights)
            .add_systems(Update, patch_falloff)
            .init_resource::<Cookies>()
            .add_systems(Update, light_cookies)
            .add_systems(Update, (setup_cameras, ambient_cube_cameras, update_fog, fade_lights, light_functions, shadow_casters, add_flashlight, toggle_flashlight, aim_flashlight).chain());
    }
}

/// A light's light function (its LightFunctionMaterial's FLightFunctionPS, F13-decomp MODLOG "Light functions"): its
/// value scales the light (the function is evaluated per pixel in the light's space; these two barely vary across it,
/// so here once per frame). Fade by LightFunctionFadeDistance (100000 cm by default) is ignored: it starts 800 m out.
/// - M_FixedLightFunction: 0.44 sin(time x 1.256637) + 0.44.
/// - M_BrokenLightFunction: saturate(Flicker2(u x 0.001 + FlickerOffset + time x 0.1, v x 0.1 + 1 + time x 0.01).r x 5
///   - 2), (u, v) the direction's spherical (point lights) or projected (spot) coordinates: PLACEHOLDER (0.5, 0.5),
///   the middle of the light, for the per-pixel spread of v x 0.1.
/// The cars' M_LightFunction_CarHeadlights_01 (a pattern across the beam) isn't run.
#[derive(Component, Clone, Copy)]
enum LightFunction {
    Pulse,
    Flicker,
}

/// A spot light's projected light function (M_FlashlightFunction_01, M_LightFunction_CarHeadlights_01: the same
/// compiled FLightFunctionPS, s012): its texture (export-relative, no extension) sampled at 0.5 + 0.5 x (up, right) /
/// (forward x tan(outer cone)) in the light's space, the light scaled by the mean of its rgb. Baked once into Bevy's
/// light texture ([`SpotLightTexture`], red, its own projection: uv = 0.5 + 0.5 x (right, -up) / (forward x tan)), so
/// Bevy's (u, v) reads the function at (1 - v, u).
#[derive(Component, Clone, Copy)]
pub(crate) struct LightCookie(pub(crate) &'static str);

/// A light function material's texture, for [`LightCookie`] (their TextureSample's Texture, f13-export debug-json).
pub(crate) fn light_function_texture(material: &str) -> Option<&'static str> {
    match material {
        "M_FlashlightFunction_01" => Some("SummerCamp/Content/Art/Textures/LightFunctionTest"),
        "M_LightFunction_CarHeadlights_01" => Some("SummerCamp/Content/Lighting/LightFunctions/T_LightFunction_CarHeadlights_01"),
        _ => None,
    }
}

/// The baked light textures by source texture (and the sources while they load).
#[derive(Resource, Default)]
struct Cookies(std::collections::HashMap<&'static str, (Handle<Image>, Option<Handle<Image>>)>);

/// Bakes each [`LightCookie`]'s texture once it's loaded and gives the light its [`SpotLightTexture`].
fn light_cookies(
    mut commands: Commands,
    mut cookies: ResMut<Cookies>,
    assets: Res<AssetServer>,
    mut images: ResMut<Assets<Image>>,
    lights: Query<(Entity, &LightCookie), Without<bevy::light::SpotLightTexture>>,
) {
    for (e, c) in &lights {
        let entry = cookies.0.entry(c.0).or_insert_with(|| {
            let path = format!("{}.png", c.0);
            (assets.load_with_settings(path, |s: &mut bevy::image::ImageLoaderSettings| s.is_srgb = false), None)
        });
        if entry.1.is_none() {
            let Some(src) = images.get(&entry.0) else { continue };
            let (w, h) = (src.width(), src.height());
            let mut data = Vec::with_capacity((w * h * 4) as usize);
            for y in 0..h {
                for x in 0..w {
                    let (u, v) = ((x as f32 + 0.5) / w as f32, (y as f32 + 0.5) / h as f32);
                    let (su, sv) = (((1.0 - v) * w as f32) as u32, (u * h as f32) as u32);
                    let c = src.get_color_at(su.min(w - 1), sv.min(h - 1)).map(|c| c.to_linear()).unwrap_or(LinearRgba::BLACK);
                    // The function in red (the light reads .r); alpha 0: Bevy also hands a spot light's texture to
                    // the clustered decals, whose base colour pass composites it over every StandardMaterial in the
                    // light's box (an opaque R8 read as (r, 0, 0, 1) turned her clothes red); at alpha 0 it changes
                    // nothing there.
                    data.extend([(((c.red + c.green + c.blue) * 0.3333).clamp(0.0, 1.0) * 255.0).round() as u8, 0, 0, 0]);
                }
            }
            let baked = Image::new(
                bevy::render::render_resource::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
                bevy::render::render_resource::TextureDimension::D2,
                data,
                bevy::render::render_resource::TextureFormat::Rgba8Unorm,
                bevy::asset::RenderAssetUsages::RENDER_WORLD,
            );
            entry.1 = Some(images.add(baked));
        }
        if let Some(image) = entry.1.clone() {
            commands.entity(e).insert(bevy::light::SpotLightTexture { image });
        }
    }
}

/// The Flicker2 texture, read on the CPU for [`LightFunction::Flicker`].
#[derive(Resource)]
struct FlickerTexture(Handle<Image>);

fn spawn_lights(mut commands: Commands, level: Res<crate::Level>, export: Res<crate::ExportRoot>, unlit: Option<Res<Unlit>>, assets: Res<AssetServer>, mut images: ResMut<Assets<Image>>) {
    if unlit.is_some() {
        info!("lighting: off (--unlit)");
        commands.insert_resource(GlobalAmbientLight { color: Color::WHITE, brightness: UNLIT_AMBIENT, ..default() });
        commands.insert_resource(ClearColor(Color::srgb(0.4, 0.45, 0.5)));
        // And a soft light from above, so surfaces whose ambient occlusion reads 0 (BC5 RMA maps) still show.
        commands.spawn((DirectionalLight { illuminance: UNLIT_SUN_LUX, shadow_maps_enabled: false, ..default() }, Transform::from_rotation(Quat::from_euler(EulerRot::YXZ, 0.6, -0.9, 0.0))));
        return;
    }
    let classes = match ClassDb::load(&export.0.join("classes.json")) {
        Ok(c) => c,
        Err(e) => {
            warn!("lighting: no classes.json ({e}): only the moon");
            ClassDb::default()
        }
    };
    let lights = placed_lights(&level, &classes);
    // Data (not sRGB), as the material samples it.
    commands.insert_resource(FlickerTexture(
        assets.load_with_settings("SummerCamp/Content/Art/Textures/Flicker2.png", |s: &mut bevy::image::ImageLoaderSettings| s.is_srgb = false),
    ));
    let grids = crate::power::PowerGrids::from_level(&level);
    let mut moon = false;
    let (mut points, mut spots) = (0, 0);
    // A phone box's broken and fixed lights: both sets, shown by its state (police.rs).
    let box_light = |l: &PlacedLight| {
        let class = l.props.get("ActorClass").and_then(Value::as_str).unwrap_or("");
        let broken = l.name().ends_with("_Broken");
        (classes.is_child_of(class, "SCPhoneJunctionBox") && (broken || l.name().ends_with("_Fixed")))
            .then(|| crate::police::BoxLight { actor: l.props.get("ActorName").and_then(Value::as_str).unwrap_or("").to_string(), broken })
    };
    for l in lights.iter().filter(|l| (l.visible() || l.is_power_out() || box_light(l).is_some()) && l.flag("bAffectsWorld", true)) {
        let grid = l.grid(&grids);
        let at = to_bevy(l.position());
        let dir = to_bevy(l.direction()).normalize_or(Vec3::NEG_Y);
        match l.kind {
            LightKind::Directional => {
                moon = true;
                let shadow_m = l.num("DynamicShadowDistanceMovableLight", 20000.0) / 100.0;
                info!("lighting: moon {:.2} lux, shadows to {shadow_m:.0} m", l.num("Intensity", 10.0));
                commands.spawn((
                    DirectionalLight { color: l.color().into(), illuminance: l.num("Intensity", 10.0), shadow_maps_enabled: moon_cascades() > 0, ..default() },
                    CascadeShadowConfigBuilder { num_cascades: moon_cascades().max(1), maximum_distance: shadow_m, first_cascade_far_bound: 8.0, ..default() }.build(),
                    Transform::default().looking_to(dir, Vec3::Y),
                    crate::weather::Moon,
                ));
            }
            LightKind::Point | LightKind::Spot => {
                let lumens = l.lumens();
                let range = l.radius() / 100.0;
                let fade = DrawDistance { max: l.num("MaxDrawDistance", 0.0) / 100.0, fade: l.num("MaxDistanceFadeRange", 0.0) / 100.0, lumens };
                let xf = Transform::from_translation(at).looking_to(dir, if dir.y.abs() > 0.99 { Vec3::Z } else { Vec3::Y });
                let e = if l.kind == LightKind::Point {
                    points += 1;
                    commands.spawn((PointLight { color: l.color().into(), intensity: lumens, range, radius: l.sphere_radius(), shadow_maps_enabled: false, ..default() }, xf, fade)).id()
                } else {
                    spots += 1;
                    let (inner, outer) = l.cone();
                    let outer = outer.clamp(0.5, 89.0).to_radians();
                    commands
                        .spawn((
                            SpotLight { color: l.color().into(), intensity: lumens, range, radius: l.sphere_radius(), inner_angle: inner.to_radians().min(outer), outer_angle: outer, shadow_maps_enabled: false, ..default() },
                            xf,
                            fade,
                        ))
                        .id()
                };
                if l.flag("CastShadows", true) && l.flag("CastDynamicShadows", true) {
                    commands.entity(e).insert(ShadowCaster);
                }
                // (Its share of the indirect light: enlighten.rs.)
                commands.entity(e).insert(crate::enlighten::IndirectIntensity(l.num("IndirectLightingIntensity", 1.0)));
                match l.props.get("LightFunctionMaterial").and_then(Value::as_str).and_then(|m| m.split('\'').nth(1)) {
                    Some("M_FixedLightFunction") => {
                        commands.entity(e).insert(LightFunction::Pulse);
                    }
                    Some("M_BrokenLightFunction") => {
                        commands.entity(e).insert(LightFunction::Flicker);
                    }
                    _ => {}
                }
                if let Some(b) = box_light(l) {
                    commands.entity(e).insert((b, if l.visible() { Visibility::Inherited } else { Visibility::Hidden }));
                }
                // Power (power.rs): powered lights follow their grid, a fuse box's power-out light waits for a cut.
                match (grid, l.is_power_out()) {
                    (Some(g), true) => {
                        commands.entity(e).insert((crate::power::PowerOutLight(g), Visibility::Hidden));
                    }
                    (Some(g), false) => {
                        commands.entity(e).insert(crate::power::Powered(g));
                    }
                    (None, true) => {
                        commands.entity(e).despawn();
                    }
                    _ => {}
                }
            }
        }
    }
    if !moon {
        warn!("lighting: no moon in the export (re-export the map): a placeholder one");
        commands.spawn((
            DirectionalLight { illuminance: 1.0, shadow_maps_enabled: true, ..default() },
            CascadeShadowConfigBuilder { num_cascades: 3, maximum_distance: DEFAULT_SHADOW_DISTANCE_M, first_cascade_far_bound: 8.0, ..default() }.build(),
            Transform::from_rotation(Quat::from_euler(EulerRot::YXZ, 0.6, -0.9, 0.0)),
        ));
    }
    info!("lighting: {points} point and {spots} spot lights");

    // Ambient: the unbound post-process volume's ambient cubemap.
    let actors = || level.layouts.iter().flat_map(|l| &l.actors);
    let volume = actors().find(|a| a.class == "PostProcessVolume" && a.props["bUnbound"].as_bool() == Some(true));
    let settings = volume.map(|v| &v.props["Settings"]);
    let mean = volume.and_then(|v| v.props["AmbientCubemapAverage"].as_array()).map(|a| a.iter().filter_map(Value::as_f64).map(|v| v as f32).collect::<Vec<_>>());
    let cube = volume.and_then(|v| v.props["AmbientCubemapFile"].as_str()).and_then(|f| std::fs::read(export.0.join(f)).ok());
    if let (Some(s), Some(cube)) = (settings, cube.as_deref().zip(settings).and_then(|(b, s)| ambient_cube(b, s))) {
        // FPostProcessAmbientPS: per pixel the cube along its normal (diffuse) and its reflection (specular), x
        // AmbientCubemapIntensity x tint (the tint in the texels), on every pixel (the vendored environment map shader).
        let intensity = s["AmbientCubemapIntensity"].as_f64().unwrap_or(1.0) as f32;
        info!("lighting: ambient cubemap ({} mips), intensity {intensity}", cube.texture_descriptor.mip_level_count);
        commands.insert_resource(AmbientCube { image: images.add(cube), intensity });
        commands.insert_resource(GlobalAmbientLight { color: Color::BLACK, brightness: 0.0, ..default() });
    } else if let (Some(s), Some(mean)) = (settings, mean.filter(|m| m.len() == 3)) {
        let scale = s["AmbientCubemapIntensity"].as_f64().unwrap_or(1.0) as f32;
        let tint = |k: &str| s["AmbientCubemapTint"][k].as_f64().unwrap_or(1.0) as f32;
        let c = LinearRgba::rgb(mean[0] * scale * tint("R"), mean[1] * scale * tint("G"), mean[2] * scale * tint("B"));
        warn!("lighting: no ambient cubemap file (re-export the map): its mean {:.4} {:.4} {:.4} as uniform ambient", c.red, c.green, c.blue);
        commands.insert_resource(GlobalAmbientLight { color: c.into(), brightness: 1.0, ..default() });
    } else {
        warn!("lighting: no ambient cubemap in the export (re-export the map)");
    }

    // Behind the sky sphere (the engine's SM_SkySphere with M_Sky_CrystalLake: materials.rs), and where its export is
    // missing: its material's ColorTint × Overall, seen through the exposure.
    if let Some(sky) = level.layouts.iter().map(|l| &l.sky).find(|s| s.is_object()) {
        let overall = sky["Overall"].as_f64().unwrap_or(1.0) as f32;
        let tint = |i: usize| sky["ColorTint"][i].as_f64().unwrap_or(1.0) as f32 * overall * Exposure { ev100: 0.0 }.exposure();
        commands.insert_resource(ClearColor(Color::linear_rgb(tint(0), tint(1), tint(2))));
    } else {
        commands.insert_resource(ClearColor(Color::BLACK));
    }

    // The weather's height fog.
    let sun = placed_components(&level, &classes, |k| k == "DirectionalLightComponent").iter().any(|(_, _, p)| p.get("bUsedAsAtmosphereSunLight").and_then(Value::as_bool) == Some(true));
    if let Some(fog) = placed_components(&level, &classes, |k| k == "ExponentialHeightFogComponent").first() {
        let (_, world, p) = fog;
        let num = |k: &str, d: f64| p.get(k).and_then(Value::as_f64).unwrap_or(d) as f32;
        let color = |k: &str, d: [f32; 3]| {
            let c = p.get(k);
            let ch = |n: &str, i: usize| c.and_then(|c| c.get(n)).and_then(Value::as_f64).map_or(d[i], |v| v as f32);
            LinearRgba::rgb(ch("R", 0), ch("G", 1), ch("B", 2))
        };
        // Unreal's defaults: density 0.02, falloff 0.2, max opacity 1, inscattering (0.447, 0.638, 1), directional
        // inscattering exponent 4, colour (0.25, 0.25, 0.125).
        commands.insert_resource(HeightFog {
            density: num("FogDensity", 0.02) / 1000.0 * 100.0,
            falloff: num("FogHeightFalloff", 0.2) / 1000.0 * 100.0,
            height: world.t[2] / 100.0,
            max_opacity: num("FogMaxOpacity", 1.0),
            inscattering: color("FogInscatteringColor", [0.447, 0.638, 1.0]),
            directional: color("DirectionalInscatteringColor", [0.25, 0.25, 0.125]),
            directional_exponent: num("DirectionalInscatteringExponent", 4.0),
            directional_start: num("DirectionalInscatteringStartDistance", 10000.0) / 100.0,
            sun,
        });
    }
}

/// The post-process volume's ambient cubemap (its cooked mips, Unreal's face layout and axes: the vendored
/// environment map shader looks it up so) and AmbientCubemapIntensity, for the cameras' EnvironmentMapLight.
#[derive(Resource, Clone)]
struct AmbientCube {
    image: Handle<Image>,
    intensity: f32,
}

/// An f13-export `.cube` ("F13C", u32 size, u32 mips, per mip six faces of half-float RGBA) as a cube image with its
/// mips, its texels times the volume's AmbientCubemapTint.
fn ambient_cube(b: &[u8], settings: &Value) -> Option<Image> {
    use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat, TextureViewDescriptor, TextureViewDimension};
    if b.get(0..4)? != b"F13C" {
        return None;
    }
    let size = u32::from_le_bytes(b.get(4..8)?.try_into().ok()?);
    let mips = u32::from_le_bytes(b.get(8..12)?.try_into().ok()?);
    let tint = |k: &str| settings["AmbientCubemapTint"][k].as_f64().unwrap_or(1.0) as f32;
    let tint = [tint("R"), tint("G"), tint("B"), 1.0];
    let mut data = b.get(12..)?.to_vec();
    let expected: usize = (0..mips).map(|m| ((size >> m).max(1) as usize).pow(2) * 6 * 8).sum();
    if data.len() != expected {
        return None;
    }
    for (i, px) in data.chunks_exact_mut(2).enumerate() {
        let v = crate::enlighten::f16_value(u16::from_le_bytes([px[0], px[1]])) * tint[i % 4];
        px.copy_from_slice(&crate::enlighten::f16_bits(v).to_le_bytes());
    }
    let mut image = Image::default();
    image.texture_descriptor.size = Extent3d { width: size, height: size, depth_or_array_layers: 6 };
    image.texture_descriptor.dimension = TextureDimension::D2;
    image.texture_descriptor.format = TextureFormat::Rgba16Float;
    image.texture_descriptor.mip_level_count = mips;
    image.data = Some(data);
    image.data_order = bevy::render::render_resource::TextureDataOrder::MipMajor;
    image.texture_view_descriptor = Some(TextureViewDescriptor { dimension: Some(TextureViewDimension::Cube), ..default() });
    image.sampler = bevy::image::ImageSampler::linear();
    Some(image)
}

/// Each lit camera's environment light: the ambient cubemap.
fn ambient_cube_cameras(mut commands: Commands, cube: Option<Res<AmbientCube>>, cameras: Query<Entity, (With<Lit>, Without<EnvironmentMapLight>)>) {
    let Some(cube) = cube else { return };
    for e in &cameras {
        commands.entity(e).insert(EnvironmentMapLight {
            diffuse_map: cube.image.clone(),
            specular_map: cube.image.clone(),
            intensity: cube.intensity,
            affects_lightmapped_mesh_diffuse: true,
            ..default()
        });
    }
}

/// Fixed exposure and its fog on every 3D camera (tonemap.rs puts the game's tonemapper on them).
fn setup_cameras(mut commands: Commands, cameras: Query<Entity, (With<Camera3d>, Without<Lit>, Without<crate::sense_view::SenseMaskCamera>, Without<crate::counselor_preview::PreviewCamera>)>) {
    for e in &cameras {
        commands.entity(e).insert((Lit, Exposure { ev100: 0.0 }));
    }
}

fn update_fog(mut commands: Commands, fog: Option<Res<HeightFog>>, cameras: Query<Entity, With<Lit>>) {
    let Some(f) = fog else { return };
    // Pixels are radiance / 1.2 (the exposure); Bevy adds fog after exposure.
    let exposure = Exposure { ev100: 0.0 }.exposure();
    for e in &cameras {
        let c = f.inscattering * exposure;
        commands.entity(e).insert(DistanceFog {
            color: Color::linear_rgba(c.red, c.green, c.blue, f.max_opacity),
            directional_light_color: f.directional.with_alpha(if f.sun { 1.0 } else { 0.0 }).into(),
            directional_light_exponent: f.directional_exponent,
            falloff: FogFalloff::HeightFog { density: f.density, falloff: f.falloff, height: f.height, directional_start: f.directional_start },
        });
    }
}

/// Fades placed lights out toward their max draw distance from the camera.
fn fade_lights(
    cameras: Query<&GlobalTransform, (With<Lit>, With<Camera3d>)>,
    mut points: Query<(&mut PointLight, &Transform, &DrawDistance), Without<SpotLight>>,
    mut spots: Query<(&mut SpotLight, &Transform, &DrawDistance), Without<PointLight>>,
) {
    let Some(cam) = cameras.iter().next().map(GlobalTransform::translation) else { return };
    let scale = |xf: &Transform, d: &DrawDistance| {
        if d.max <= 0.0 {
            return 1.0;
        }
        let dist = xf.translation.distance(cam);
        if d.fade > 0.0 { ((d.max - dist) / d.fade).clamp(0.0, 1.0) } else if dist <= d.max { 1.0 } else { 0.0 }
    };
    for (mut l, xf, d) in &mut points {
        let i = d.lumens * scale(xf, d);
        if l.intensity != i {
            l.intensity = i;
        }
    }
    for (mut l, xf, d) in &mut spots {
        let i = d.lumens * scale(xf, d);
        if l.intensity != i {
            l.intensity = i;
        }
    }
}

/// Scales the lights with a [`LightFunction`] (after [`fade_lights`] set their intensity).
fn light_functions(
    time: Res<Time>,
    flicker: Option<Res<FlickerTexture>>,
    images: Res<Assets<Image>>,
    mut points: Query<(&mut PointLight, &LightFunction), Without<SpotLight>>,
    mut spots: Query<(&mut SpotLight, &LightFunction), Without<PointLight>>,
) {
    let t = time.elapsed_secs();
    let pulse = (0.44 * (t * 1.256637).sin() + 0.44).max(0.0);
    let flicker = flicker.and_then(|f| images.get(&f.0)).map_or(1.0, |img| {
        let size = img.size().as_vec2();
        // Bilinear, wrapping (the texture's sampler).
        let uv = Vec2::new(0.5 * 0.001 + t * 0.1, 0.5 * 0.1 + 1.0 + t * 0.01) * size - 0.5;
        let (x0, f) = (uv.floor(), uv.fract());
        let at = |x: f32, y: f32| {
            let (w, h) = (size.x as i64, size.y as i64);
            let (x, y) = ((x as i64).rem_euclid(w) as u32, (y as i64).rem_euclid(h) as u32);
            img.get_color_at(x, y).map_or(0.0, |c| c.to_linear().red)
        };
        let top = at(x0.x, x0.y) * (1.0 - f.x) + at(x0.x + 1.0, x0.y) * f.x;
        let bottom = at(x0.x, x0.y + 1.0) * (1.0 - f.x) + at(x0.x + 1.0, x0.y + 1.0) * f.x;
        ((top * (1.0 - f.y) + bottom * f.y) * 5.0 - 2.0).clamp(0.0, 1.0)
    });
    let value = |f: &LightFunction| match f {
        LightFunction::Pulse => pulse,
        LightFunction::Flicker => flicker,
    };
    for (mut l, f) in &mut points {
        l.intensity *= value(f);
    }
    for (mut l, f) in &mut spots {
        l.intensity *= value(f);
    }
}

/// Which lights render shadow maps this frame, as the engine picks its whole-scene shadows: a shadow-casting light
/// that's on, whose sphere meets the view's frustum, and whose shadow would be over r.Shadow.MinResolution: texels per
/// pixel x max(width / 2 x P[0][0], height / 2 x P[1][1]) x its radius / max(its distance, 1 cm) (the shadow fade's
/// resolution, FUN_140b571d0); of those, the [`SHADOW_BUDGET`] nearest whose range reaches the camera, each at its fade's
/// strength ([`shadow_fade_alpha`], vendored bevy_pbr's ShadowFade). PLACEHOLDER: the shadow map's own resolution
/// (Bevy's one size for all; the game's clamps that resolution to r.Shadow.MaxResolution).
#[allow(clippy::type_complexity)]
fn shadow_casters(
    cameras: Query<(&GlobalTransform, &Projection, &bevy::camera::primitives::Frustum, &Camera), (With<Lit>, With<Camera3d>)>,
    mut points: Query<(Entity, &mut PointLight, &GlobalTransform, &InheritedVisibility), (With<ShadowCaster>, Without<SpotLight>)>,
    mut spots: Query<(Entity, &mut SpotLight, &GlobalTransform, &InheritedVisibility), (With<ShadowCaster>, Without<PointLight>)>,
    fades: Query<&bevy::pbr::ShadowFade>,
    mut commands: Commands,
) {
    let Some((cam, projection, frustum, camera)) = cameras.iter().find(|c| c.3.is_active) else { return };
    let clip = projection.get_clip_from_view();
    let size = camera.logical_viewport_size().unwrap_or(Vec2::new(1920.0, 1080.0));
    let screen = (0.5 * size.x * clip.x_axis.x).max(0.5 * size.y * clip.y_axis.y);
    let eye = cam.translation();
    // The light's distance when its sphere meets the frustum, its shadow's unclamped resolution is over MinResolution
    // and its range reaches the camera.
    let qualifies = |at: Vec3, range: f32, texels: f32| {
        let seen = frustum.intersects_sphere(&bevy::camera::primitives::Sphere { center: at.into(), radius: range }, true);
        let d = at.distance(eye);
        let r = texels * screen * range / d.max(0.01);
        (seen && r > SHADOW_MIN_RESOLUTION && d < range).then_some((d, shadow_fade_alpha(r)))
    };
    let mut wanted: Vec<(f32, Entity, f32)> = vec![];
    for (e, l, g, v) in &points {
        if let Some((d, a)) = qualifies(g.translation(), l.range, TEXELS_PER_PIXEL_POINT).filter(|_| v.get() && l.intensity > 0.0) {
            wanted.push((d, e, a));
        }
    }
    for (e, l, g, v) in &spots {
        if let Some((d, a)) = qualifies(g.translation(), l.range, TEXELS_PER_PIXEL_SPOT).filter(|_| v.get() && l.intensity > 0.0) {
            wanted.push((d, e, a));
        }
    }
    wanted.sort_by(|a, b| a.0.total_cmp(&b.0));
    wanted.truncate(shadow_budget());
    let on: Vec<Entity> = wanted.iter().map(|w| w.1).collect();
    // Their strength, set when it moves (each change re-extracts the light).
    for &(_, e, a) in &wanted {
        if fades.get(e).is_ok_and(|f| (f.0 - a).abs() < 0.01) {
            continue;
        }
        commands.entity(e).insert(bevy::pbr::ShadowFade(a));
    }
    for (e, mut l, _, _) in &mut points {
        let want = on.contains(&e);
        if l.shadow_maps_enabled != want {
            l.shadow_maps_enabled = want;
        }
    }
    for (e, mut l, _, _) in &mut spots {
        let want = on.contains(&e);
        if l.shadow_maps_enabled != want {
            l.shadow_maps_enabled = want;
        }
    }
}

/// Gives the counselor her flashlight (off) once her character data is in.
fn add_flashlight(
    mut commands: Commands,
    at_start: Res<FlashlightAtStart>,
    model: Option<Res<crate::character::CounselorCharacter>>,
    players: Query<(Entity, &Transform), (With<Player>, Without<Flashlight>)>,
) {
    let Some(model) = model else { return };
    for (pe, xf) in &players {
    let v = |k: &str, d: f32| model.0.info.values.get(k).copied().unwrap_or(d);
    let byte = |k: &str| v(k, 255.0).clamp(0.0, 255.0) as u8;
    let mut color = LinearRgba::from(Color::srgb_u8(byte("FlashlightColorR"), byte("FlashlightColorG"), byte("FlashlightColorB")));
    let temperature = v("FlashlightTemperature", 0.0);
    if temperature > 0.0 {
        let t = temperature_color(temperature);
        color = LinearRgba::rgb(color.red * t[0], color.green * t[1], color.blue * t[2]);
    }
    let (intensity, radius) = (v("FlashlightIntensity", 5000.0), v("FlashlightAttenuationRadius", 1000.0));
    let inverse = v("FlashlightInverseSquared", 1.0) > 0.5;
    let lumens = if inverse { inverse_squared_lumens(intensity) } else { falloff_lumens(intensity) };
    let sphere = if inverse { 0.0 } else { falloff_radius(v("FlashlightFalloffExponent", 8.0)) };
    let outer = v("FlashlightOuterConeAngle", 44.0).clamp(0.5, 89.0).to_radians();
    let t = FlashlightTuning {
        lumens,
        color,
        range: radius / 100.0,
        inner: v("FlashlightInnerConeAngle", 0.0).to_radians().min(outer),
        outer,
        lerp_speed: v("FlashlightLerpSpeed", 15.0),
    };
    let light = commands
        .spawn((
            SpotLight { color: t.color.into(), intensity: 0.0, range: t.range, radius: sphere, inner_angle: t.inner, outer_angle: t.outer, shadow_maps_enabled: true, ..default() },
            Transform::from_translation(xf.translation),
            // Base_Counselor's Flashlight: LightFunctionMaterial M_FlashlightFunction_01 (MODLOG "Night lighting").
            LightCookie(light_function_texture("M_FlashlightFunction_01").unwrap_or_default()),
        ))
        .id();
    info!("flashlight: {lumens:.0} lm, {:.0} m, cone {:.0}/{:.0} degrees", t.range, t.inner.to_degrees(), t.outer.to_degrees());
    commands.insert_resource(t);
    commands.entity(pe).insert(Flashlight { on: at_start.0, yaw: xf.rotation.to_euler(EulerRot::YXZ).0, light });
    }
}

/// F turns it on and off (CSLR_Flashlight); hiding and dying turn it off.
fn toggle_flashlight(
    pads: crate::controls::Pads,
    mut players: Query<(Entity, &mut Flashlight, &Player, Has<crate::hiding::Hiding>, Has<crate::vehicle::Driving>)>,
) {
    for (pe, mut f, p, hiding, driving) in &mut players {
        if hiding || !p.counselor.alive() {
            if f.on {
                info!("flashlight off");
            }
            f.on = false;
            continue;
        }
        if pads.just_pressed(pe, crate::controls::Button::Flashlight) && !driving {
            f.on = !f.on;
            info!("flashlight {}", if f.on { "on" } else { "off" });
        }
    }
}

/// Places and aims the flashlight, and puts it out while it's inside a wall.
fn aim_flashlight(
    time: Res<Time>,
    tuning: Option<Res<FlashlightTuning>>,
    collision: Option<Res<crate::collision::WorldCollision>>,
    dynamic: Option<Res<crate::collision::DynamicColliders>>,
    mut players: Query<(&mut Flashlight, &Transform), Without<SpotLight>>,
    mut lights: Query<(&mut SpotLight, &mut Transform)>,
) {
    let (Some(t), Some(collision), Some(dynamic)) = (tuning, collision, dynamic) else { return };
    for (mut f, xf) in &mut players {
    let Ok((mut light, mut lxf)) = lights.get_mut(f.light) else { continue };
    // RInterpTo on the yaw: the shortest way round, a step of dt * speed of the remaining angle.
    let target = xf.rotation.to_euler(EulerRot::YXZ).0;
    let delta = (target - f.yaw + std::f32::consts::PI).rem_euclid(std::f32::consts::TAU) - std::f32::consts::PI;
    f.yaw += delta * (time.delta_secs() * t.lerp_speed).clamp(0.0, 1.0);
    let facing = Quat::from_rotation_y(f.yaw);
    // Mesh space (Unreal): +Y forward, +X to her left, +Z up from her feet.
    let forward = facing * Vec3::Z;
    let right = facing * Vec3::NEG_X;
    let at = xf.translation + forward * FLASHLIGHT_IN_MESH.y / 100.0 - right * FLASHLIGHT_IN_MESH.x / 100.0 + Vec3::Y * FLASHLIGHT_IN_MESH.z / 100.0;
    *lxf = Transform::from_translation(at).looking_to(forward, Vec3::Y);
    let centre = xf.translation + Vec3::Y * crate::player::CAPSULE_HALF_HEIGHT_M;
    let blocked = f13_core::collision::segment_hit(&collision.with(&dynamic), centre, at, 1.0).is_some();
    let shine = f.on && !blocked;
    let intensity = if shine { t.lumens } else { 0.0 };
    if light.intensity != intensity {
        light.intensity = intensity;
    }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shadow_fades_between_min_and_fade_resolution() {
        assert_eq!(shadow_fade_alpha(20.0), 0.0);
        assert_eq!(shadow_fade_alpha(32.0), 0.0);
        assert!((shadow_fade_alpha(48.0) - 0.5f32.powf(1.25)).abs() < 1e-6);
        assert_eq!(shadow_fade_alpha(64.0), 1.0);
        assert_eq!(shadow_fade_alpha(500.0), 1.0);
    }

    #[test]
    fn falloff_light_is_its_intensity_at_the_centre() {
        // Bevy divides lumens by 4 pi into the shaded colour; the patched falloff is 1 at the light.
        assert!((falloff_lumens(30.0) / (4.0 * std::f32::consts::PI) - 30.0).abs() < 1e-4);
        assert_eq!(falloff_radius(4.0), -4.0);
        assert!(falloff_radius(0.0) < 0.0);
    }
}
