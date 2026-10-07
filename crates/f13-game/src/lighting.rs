//! The map's night lighting, from the export: its point and spot lights, the moon and height fog of its weather
//! level (CLsm_RainOff, clear), the post-process volume's exposure and ambient light, and the counselor's
//! flashlight.
//!
//! Units carry over from Unreal 4.18 as follows:
//! - **Exposure**: the volume fixes it (auto exposure min = max brightness, 1.2), so a pixel is the scene value /
//!   1.2. That's Bevy's exposure at `ev100 = 0`. Tonemapping is ACES, the curve the game's filmic tonemapper fits.
//! - **Lights**:
//!   - The moon's Intensity is its illuminance (lux).
//!   - An inverse-square light's Intensity is lumens, as in Bevy.
//!   - A light without inverse-square falloff lights `Intensity * (1 - (d/R)^2)^FalloffExponent` lux at distance
//!     `d` within its radius `R`. Bevy has only the inverse-square law, so that light gets the lumens that match it
//!     at a third of its radius.
//!   - Colours are sRGB; `bUseTemperature` multiplies in the colour temperature (f13-core::light).
//!   - Lights fade out over their last MaxDistanceFadeRange before MaxDrawDistance from the camera, as Unreal fades
//!     them.
//! - **Ambient**: the ambient cubemap's mean radiance (computed by the exporter) × AmbientCubemapIntensity × tint,
//!   as uniform ambient light.
//! - **Fog** (ExponentialHeightFog):
//!   - density FogDensity / 1000 per cm in base 2, thinning by `2^(-FogHeightFalloff/1000 per cm)` with the
//!     camera's height above the fog actor;
//!   - capped at FogMaxOpacity;
//!   - inscattering colours as radiance.
//!   - Bevy fog is uniform along the ray, so the height falloff is taken at the camera only.
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
//! - light functions (the flashlight's cookie);
//! - the colour grading LUT, ColorContrast and bloom;
//! - the sky's texture (clouds, stars): the sky is its material's ColorTint × Overall;
//! - rain (CLsm_RainOn);
//! - the power's flicker curves (power.rs turns lights off and on).

use bevy::camera::Exposure;
use bevy::core_pipeline::tonemapping::Tonemapping;
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
    fn num(&self, name: &str, default: f32) -> f32 {
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

    fn cone(&self) -> (f32, f32) {
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
    fn color(&self) -> LinearRgba {
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
    fn lumens(&self) -> f32 {
        let intensity = self.num("Intensity", 5000.0);
        if self.flag("bUseInverseSquaredFalloff", true) {
            return intensity;
        }
        unreal_falloff_lumens(intensity, self.radius(), self.num("LightFalloffExponent", 8.0))
    }
}

/// Lumens for a light without inverse-square falloff, matching its illuminance at a third of its radius `r` (cm).
fn unreal_falloff_lumens(intensity: f32, r: f32, exponent: f32) -> f32 {
    let lux = intensity * (8.0f32 / 9.0).powf(exponent);
    let d = r / 300.0;
    lux * d * d * 4.0 * std::f32::consts::PI
}

/// Every component the level places whose type `keep` accepts: (type, world transform, properties), the placed
/// actor's own values over its class's; the properties also hold the component's name (`ComponentName`).
pub fn placed_components(level: &crate::Level, classes: &ClassDb, keep: impl Fn(&str) -> bool) -> Vec<(String, Xform, Map<String, Value>)> {
    let mut out = Vec::new();
    for a in level.layouts.iter().flat_map(|l| &l.actors) {
        let Some(t) = a.transform else { continue };
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
}

/// A placed light's distance fade: MaxDrawDistance and MaxDistanceFadeRange (m), and its full intensity.
#[derive(Component)]
struct DrawDistance {
    max: f32,
    fade: f32,
    lumens: f32,
}

#[derive(Component)]
struct Lit;

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
}

#[derive(Resource)]
struct FlashlightAtStart(bool);

impl Plugin for LightingPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(FlashlightAtStart(self.flashlight))
            .add_systems(Startup, spawn_lights)
            .add_systems(Update, (setup_cameras, update_fog, fade_lights, add_flashlight, toggle_flashlight, aim_flashlight).chain());
    }
}

fn spawn_lights(mut commands: Commands, level: Res<crate::Level>, export: Res<crate::ExportRoot>) {
    let classes = match ClassDb::load(&export.0.join("classes.json")) {
        Ok(c) => c,
        Err(e) => {
            warn!("lighting: no classes.json ({e}): only the moon");
            ClassDb::default()
        }
    };
    let lights = placed_lights(&level, &classes);
    let grids = crate::power::PowerGrids::from_level(&level);
    let mut moon = false;
    let (mut points, mut spots) = (0, 0);
    for l in lights.iter().filter(|l| (l.visible() || l.is_power_out()) && l.flag("bAffectsWorld", true)) {
        let grid = l.grid(&grids);
        let at = to_bevy(l.position());
        let dir = to_bevy(l.direction()).normalize_or(Vec3::NEG_Y);
        match l.kind {
            LightKind::Directional => {
                moon = true;
                let shadow_m = l.num("DynamicShadowDistanceMovableLight", 20000.0) / 100.0;
                info!("lighting: moon {:.2} lux, shadows to {shadow_m:.0} m", l.num("Intensity", 10.0));
                commands.spawn((
                    DirectionalLight { color: l.color().into(), illuminance: l.num("Intensity", 10.0), shadow_maps_enabled: true, ..default() },
                    CascadeShadowConfigBuilder { num_cascades: 3, maximum_distance: shadow_m, first_cascade_far_bound: 8.0, ..default() }.build(),
                    Transform::default().looking_to(dir, Vec3::Y),
                ));
            }
            LightKind::Point | LightKind::Spot => {
                let lumens = l.lumens();
                let range = l.radius() / 100.0;
                let fade = DrawDistance { max: l.num("MaxDrawDistance", 0.0) / 100.0, fade: l.num("MaxDistanceFadeRange", 0.0) / 100.0, lumens };
                let xf = Transform::from_translation(at).looking_to(dir, if dir.y.abs() > 0.99 { Vec3::Z } else { Vec3::Y });
                let e = if l.kind == LightKind::Point {
                    points += 1;
                    commands.spawn((PointLight { color: l.color().into(), intensity: lumens, range, shadow_maps_enabled: false, ..default() }, xf, fade)).id()
                } else {
                    spots += 1;
                    let (inner, outer) = l.cone();
                    let outer = outer.clamp(0.5, 89.0).to_radians();
                    commands
                        .spawn((
                            SpotLight { color: l.color().into(), intensity: lumens, range, inner_angle: inner.to_radians().min(outer), outer_angle: outer, shadow_maps_enabled: false, ..default() },
                            xf,
                            fade,
                        ))
                        .id()
                };
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
    if let (Some(s), Some(mean)) = (settings, mean.filter(|m| m.len() == 3)) {
        let scale = s["AmbientCubemapIntensity"].as_f64().unwrap_or(1.0) as f32;
        let tint = |k: &str| s["AmbientCubemapTint"][k].as_f64().unwrap_or(1.0) as f32;
        let c = LinearRgba::rgb(mean[0] * scale * tint("R"), mean[1] * scale * tint("G"), mean[2] * scale * tint("B"));
        info!("lighting: ambient {:.4} {:.4} {:.4}", c.red, c.green, c.blue);
        commands.insert_resource(GlobalAmbientLight { color: c.into(), brightness: 1.0, ..default() });
    } else {
        warn!("lighting: no ambient cubemap in the export (re-export the map)");
    }

    // The sky: the sky sphere's material, ColorTint × Overall (its texture's clouds and stars aren't drawn), seen
    // through the exposure.
    if let Some(sky) = level.layouts.iter().map(|l| &l.sky).find(|s| s.is_object()) {
        let overall = sky["Overall"].as_f64().unwrap_or(1.0) as f32;
        let tint = |i: usize| sky["ColorTint"][i].as_f64().unwrap_or(1.0) as f32 * overall * Exposure { ev100: 0.0 }.exposure();
        commands.insert_resource(ClearColor(Color::linear_rgb(tint(0), tint(1), tint(2))));
    } else {
        commands.insert_resource(ClearColor(Color::BLACK));
    }

    // The weather's height fog.
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
        });
    }
}

/// Fixed exposure, the game's tonemapping curve and its fog, on every 3D camera.
fn setup_cameras(mut commands: Commands, cameras: Query<Entity, (With<Camera3d>, Without<Lit>)>) {
    for e in &cameras {
        commands.entity(e).insert((Lit, Exposure { ev100: 0.0 }, Tonemapping::AcesFitted));
    }
}

fn update_fog(mut commands: Commands, fog: Option<Res<HeightFog>>, cameras: Query<(Entity, &GlobalTransform), With<Lit>>) {
    let Some(f) = fog else { return };
    // Pixels are radiance / 1.2 (the exposure); Bevy adds fog after exposure.
    let exposure = Exposure { ev100: 0.0 }.exposure();
    for (e, xf) in &cameras {
        let above = xf.translation().y - f.height;
        let density = f.density * 2f32.powf(-f.falloff * above) * std::f32::consts::LN_2;
        let c = f.inscattering * exposure;
        commands.entity(e).insert(DistanceFog {
            color: Color::linear_rgba(c.red, c.green, c.blue, f.max_opacity),
            directional_light_color: f.directional.with_alpha(1.0).into(),
            directional_light_exponent: f.directional_exponent,
            falloff: FogFalloff::Exponential { density },
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

/// Gives the counselor her flashlight (off) once her character data is in.
fn add_flashlight(
    mut commands: Commands,
    at_start: Res<FlashlightAtStart>,
    model: Option<Res<crate::character::CounselorCharacter>>,
    players: Query<(Entity, &Transform), (With<Player>, Without<Flashlight>)>,
) {
    let Some(model) = model else { return };
    let Ok((pe, xf)) = players.single() else { return };
    let v = |k: &str, d: f32| model.0.info.values.get(k).copied().unwrap_or(d);
    let byte = |k: &str| v(k, 255.0).clamp(0.0, 255.0) as u8;
    let mut color = LinearRgba::from(Color::srgb_u8(byte("FlashlightColorR"), byte("FlashlightColorG"), byte("FlashlightColorB")));
    let temperature = v("FlashlightTemperature", 0.0);
    if temperature > 0.0 {
        let t = temperature_color(temperature);
        color = LinearRgba::rgb(color.red * t[0], color.green * t[1], color.blue * t[2]);
    }
    let (intensity, radius) = (v("FlashlightIntensity", 5000.0), v("FlashlightAttenuationRadius", 1000.0));
    let lumens = if v("FlashlightInverseSquared", 1.0) > 0.5 { intensity } else { unreal_falloff_lumens(intensity, radius, v("FlashlightFalloffExponent", 8.0)) };
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
            SpotLight { color: t.color.into(), intensity: 0.0, range: t.range, inner_angle: t.inner, outer_angle: t.outer, shadow_maps_enabled: true, ..default() },
            Transform::from_translation(xf.translation),
        ))
        .id();
    info!("flashlight: {lumens:.0} lm, {:.0} m, cone {:.0}/{:.0} degrees", t.range, t.inner.to_degrees(), t.outer.to_degrees());
    commands.insert_resource(t);
    commands.entity(pe).insert(Flashlight { on: at_start.0, yaw: xf.rotation.to_euler(EulerRot::YXZ).0, light });
}

/// F turns it on and off (CSLR_Flashlight); hiding and dying turn it off.
fn toggle_flashlight(
    keys: Res<ButtonInput<KeyCode>>,
    mut players: Query<(&mut Flashlight, &Player, Has<crate::hiding::Hiding>, Has<crate::vehicle::Driving>)>,
) {
    let Ok((mut f, p, hiding, driving)) = players.single_mut() else { return };
    if hiding || !p.counselor.alive() {
        if f.on {
            info!("flashlight off");
        }
        f.on = false;
        return;
    }
    if keys.just_pressed(KeyCode::KeyF) && !driving {
        f.on = !f.on;
        info!("flashlight {}", if f.on { "on" } else { "off" });
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
    let Ok((mut f, xf)) = players.single_mut() else { return };
    let Ok((mut light, mut lxf)) = lights.get_mut(f.light) else { return };
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn falloff_light_matches_at_a_third_of_its_radius() {
        // 30 lux-units, 15 m, exponent 4 (the flashlight): (8/9)^4 * 30 lux at 5 m.
        let lm = unreal_falloff_lumens(30.0, 1500.0, 4.0);
        let lux_at_5m = lm / (4.0 * std::f32::consts::PI) / 25.0;
        assert!((lux_at_5m - 30.0 * (8.0f32 / 9.0).powi(4)).abs() < 1e-3);
    }
}
