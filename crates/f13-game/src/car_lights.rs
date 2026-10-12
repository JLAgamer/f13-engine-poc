//! The Chevettes' lights (Chevette_BP, Chevette_2Seat_BP; F13-decomp MODLOG "Car lights"): their Headlights and
//! TailLights spot lights and the exterior material's lights mask (materials.rs PropExt), by the Blueprints' events:
//! - BeginPlay: the material's own instance (SetupDynamicMaterial), Headlights 0, Tailights 0; the spot lights keep
//!   their templates' intensity and visibility;
//! - OnStarted (OnRep_IsStarted, now started): both spots visible, intensities 20 and 6; Headlights HeadlightsDefault,
//!   Tailights TailightsDefault;
//! - OnStalled (OnRep_IsStarted, now stopped): the Fade Lights timeline (1 s; its curve v, 1 to 0) sets both spots'
//!   intensity to v, Headlights v x HeadlightsDefault, Tailights v x TailightsDefault; when it ends, both spots are
//!   hidden and every parameter is 0;
//! - OnHandbrake: on, Tailights TaillightsHandbrake and the tail spot 10; off, TailightsDefault and 6;
//! - OnThrottle: below 0, Backup Lights ReverseLights, else 0;
//! - OnStarted turns the hood smoke (P_Car_Hood_Destroyed_Smoking_01) off, OnStalled on;
//! - Tick: LeftTireParticles and RightTireParticles SetActive(forward speed > 200, false).
//!
//! The headlights' light function (M_LightFunction_CarHeadlights_01) is their cookie (lighting.rs LightCookie). Not
//! ported: the spots' shadows (with the other local lights').

use crate::lighting::{LightKind, PlacedLight};
use crate::materials::PropMaterial;
use crate::props::Vehicle;
use bevy::prelude::*;
use f13_data::classes::ClassDb;
use f13_data::level::ComponentInfo;

pub struct CarLightsPlugin;

impl Plugin for CarLightsPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, (own_material, car_lights).chain());
    }
}

/// A car's Blueprint light values and state.
#[derive(Component)]
pub struct CarLights {
    head_default: f32,
    tail_default: f32,
    reverse: f32,
    tail_handbrake: f32,
    /// Its own instance of the exterior material (SetupDynamicMaterial), once its meshes have theirs.
    material: Option<Handle<PropMaterial>>,
    started: bool,
    handbrake: bool,
    backing: bool,
    /// The Fade Lights timeline's time, while it plays.
    fade: Option<f32>,
    /// Its root entity's name, for its particle systems (`<root>.<component>`: props.rs).
    actor: String,
    /// The tire dust's SetActive state.
    dust: bool,
}

impl CarLights {
    pub fn new(classes: &ClassDb, class: &str, root: &str) -> Self {
        let f = |name: &str, default: f32| classes.prop(class, name).as_f64().map_or(default, |v| v as f32);
        CarLights {
            head_default: f("HeadlightsDefault", 5.0),
            tail_default: f("TailightsDefault", 1.0),
            reverse: f("ReverseLights", 3.0),
            tail_handbrake: f("TaillightsHandbrake", 2.0),
            material: None,
            actor: root.to_string(),
            dust: false,
            started: false,
            handbrake: false,
            backing: false,
            fade: None,
        }
    }
}

/// One of a car's spot lights.
#[derive(Component)]
pub struct CarSpot {
    tail: bool,
    /// Bevy lumens per unit of the component's Intensity (it has no inverse-square falloff, so they scale together).
    lumens_per_intensity: f32,
}

/// The car's Headlights and TailLights spot light components, as children of its root (Bevy local transforms).
pub fn spot_lights(components: &[ComponentInfo]) -> Vec<((CarSpot, SpotLight, Transform, Visibility), Option<crate::lighting::LightCookie>)> {
    components
        .iter()
        .filter(|c| c.kind == "SpotLightComponent" && (c.name == "Headlights" || c.name == "TailLights"))
        .map(|c| {
            let props = c.props.as_object().cloned().unwrap_or_default();
            let l = PlacedLight { kind: LightKind::Spot, world: c.local.unwrap_or(f13_data::level::Xform::IDENTITY), props, powered: false };
            let local = crate::to_transform(&l.world);
            // Its +X, the way it shines.
            let dir = (local.rotation * Vec3::X).normalize_or(Vec3::NEG_Z);
            let (inner, outer) = l.cone();
            let outer = outer.clamp(0.5, 89.0).to_radians();
            let intensity = l.num("Intensity", 5000.0);
            let lumens = l.lumens();
            let spot = SpotLight {
                color: l.color().into(),
                intensity: lumens,
                radius: l.sphere_radius(),
                range: l.radius() / 100.0,
                inner_angle: inner.to_radians().min(outer),
                outer_angle: outer,
                shadow_maps_enabled: false,
                ..default()
            };
            let xf = Transform::from_translation(local.translation).looking_to(dir, Vec3::Y);
            let visibility = if l.visible() { Visibility::Inherited } else { Visibility::Hidden };
            // Its LightFunctionMaterial (the headlights'), as a cookie.
            let cookie = l.props.get("LightFunctionMaterial").and_then(|m| m.as_str()).and_then(|m| m.split('\'').nth(1)).and_then(crate::lighting::light_function_texture).map(crate::lighting::LightCookie);
            ((CarSpot { tail: c.name == "TailLights", lumens_per_intensity: if intensity > 0.0 { lumens / intensity } else { 0.0 } }, spot, xf, visibility), cookie)
        })
        .collect()
}

/// BeginPlay's SetupDynamicMaterial: the car's exterior gets an instance of its own, with Headlights and Tailights 0.
fn own_material(
    mut commands: Commands,
    mut cars: Query<(Entity, &mut CarLights)>,
    children: Query<&Children>,
    meshes: Query<&MeshMaterial3d<PropMaterial>>,
    mut materials: ResMut<Assets<PropMaterial>>,
) {
    for (car, mut lights) in &mut cars {
        if lights.material.is_some() {
            continue;
        }
        let Some((shared, _)) = children
            .iter_descendants(car)
            .filter_map(|e| meshes.get(e).ok().map(|m| (m.0.clone(), e)))
            .find(|(h, _)| materials.get(h).is_some_and(|m| m.extension.has_lights()))
        else {
            continue;
        };
        let Some(mut own) = materials.get(&shared).cloned() else { continue };
        own.extension.set_lights(0.0, 0.0, 0.0);
        let own = materials.add(own);
        for e in children.iter_descendants(car).filter(|&e| meshes.get(e).is_ok_and(|m| m.0 == shared)) {
            commands.entity(e).insert(MeshMaterial3d(own.clone()));
        }
        lights.material = Some(own);
    }
}

/// The Fade Lights timeline's curve (CurveFloat_0): from 1 at 0 s (cubic, leave tangent -3.0217378) to 0 at 1 s
/// (arrive tangent 0): UE's cubic key segment, a Hermite curve over the 1 s between the keys.
fn fade_curve(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    let (p0, p1, m0, m1) = (1.0, 0.0, -3.0217378, 0.0);
    let (t2, t3) = (t * t, t * t * t);
    (2.0 * t3 - 3.0 * t2 + 1.0) * p0 + (t3 - 2.0 * t2 + t) * m0 + (-2.0 * t3 + 3.0 * t2) * p1 + (t3 - t2) * m1
}

fn car_lights(
    mut commands: Commands,
    time: Res<Time>,
    mut cars: Query<(Entity, &Vehicle, &mut CarLights)>,
    mut spots: Query<(&CarSpot, &ChildOf, &mut SpotLight, &mut Visibility)>,
    mut materials: ResMut<Assets<PropMaterial>>,
) {
    let dt = time.delta_secs();
    for (car, v, mut l) in &mut cars {
        let name = l.actor.clone();
        let set_active = |commands: &mut Commands, component: &str, active: bool| {
            commands.write_message(crate::particles::ParticleActivation { actor: format!("{name}.{component}"), active, reset: false });
        };
        // Tick: the tire dust above 200 cm/s.
        let dust = v.speed > 200.0;
        if dust != l.dust {
            set_active(&mut commands, "LeftTireParticles", dust);
            set_active(&mut commands, "RightTireParticles", dust);
            l.dust = dust;
        }
        if v.started != l.started {
            set_active(&mut commands, "P_Car_Hood_Destroyed_Smoking_01", !v.started);
        }
        let Some(handle) = l.material.clone() else { continue };
        let mut spot = |tail: bool, intensity: Option<f32>, visible: Option<bool>| {
            for (s, parent, mut light, mut vis) in &mut spots {
                if parent.parent() != car || s.tail != tail {
                    continue;
                }
                if let Some(i) = intensity {
                    light.intensity = i * s.lumens_per_intensity;
                }
                if let Some(on) = visible {
                    vis.set_if_neq(if on { Visibility::Inherited } else { Visibility::Hidden });
                }
            }
        };
        let mut params: Option<(Option<f32>, Option<f32>, Option<f32>)> = None;
        if v.started != l.started {
            l.started = v.started;
            if v.started {
                // OnStarted.
                l.fade = None;
                spot(false, Some(20.0), Some(true));
                spot(true, Some(6.0), Some(true));
                params = Some((Some(l.head_default), Some(l.tail_default), None));
            } else {
                // OnStalled: Fade Lights from the start.
                l.fade = Some(0.0);
            }
        }
        if let Some(t) = l.fade {
            let t = t + dt;
            if t >= 1.0 {
                // Its finished event.
                l.fade = None;
                spot(false, Some(1.0), Some(false));
                spot(true, Some(1.0), Some(false));
                params = Some((Some(0.0), Some(0.0), Some(0.0)));
            } else {
                l.fade = Some(t);
                let x = fade_curve(t);
                spot(false, Some(x), None);
                spot(true, Some(x), None);
                params = Some((Some(x * l.head_default), Some(x * l.tail_default), None));
            }
        }
        if v.handbrake != l.handbrake {
            l.handbrake = v.handbrake;
            spot(true, Some(if v.handbrake { 10.0 } else { 6.0 }), None);
            let tail = if v.handbrake { l.tail_handbrake } else { l.tail_default };
            let p = params.get_or_insert((None, None, None));
            p.1 = Some(tail);
        }
        let backing = v.throttle < 0.0;
        if backing != l.backing {
            l.backing = backing;
            let p = params.get_or_insert((None, None, None));
            p.2 = Some(if backing { l.reverse } else { 0.0 });
        }
        if let Some((head, tail, backup)) = params {
            if let Some(mut m) = materials.get_mut(&handle) {
                let cur = m.extension.lights();
                m.extension.set_lights(head.unwrap_or(cur.x), tail.unwrap_or(cur.y), backup.unwrap_or(cur.z));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::fade_curve;

    #[test]
    fn fade_goes_from_one_to_zero() {
        assert!((fade_curve(0.0) - 1.0).abs() < 1e-6);
        assert!(fade_curve(1.0).abs() < 1e-6);
        // The steep start (tangent -3): already well under the linear fade at a quarter.
        assert!(fade_curve(0.25) < 0.75);
    }
}
