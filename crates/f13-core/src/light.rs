//! Whether a counselor stands in the light (her "Darkness" fear event is pushed when she doesn't).
//!
//! Reimplemented from the original game code (the counselor's light check, run with her fear events each tick):
//! - Her flashlight on: lit.
//! - Otherwise every point or spot light that is visible and affects her capsule's bounds (Unreal's
//!   `AffectsBounds`: within its attenuation radius plus the bounds' radius; a spot light also needs the bounds
//!   to touch its outer cone) is scored from the distance `d` between the light and her capsule's centre:
//!   `max(1 - d / radius, 0) ^ (falloff exponent + 1) * radius * 1.25`. She's lit by the first light scoring more
//!   than her `MinLightStrength`.
//!
//! Brightness, colour and shadowing don't count. Positions are in Unreal units (cm).

use glam::Vec3;

/// The light score's constant factor.
const STRENGTH_SCALE: f32 = 1.25;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Light {
    pub position: Vec3,
    /// Unit vector the light points along (its X axis); spot lights only use it.
    pub direction: Vec3,
    /// `AttenuationRadius`.
    pub radius: f32,
    /// `LightFalloffExponent`.
    pub falloff_exponent: f32,
    /// Spot lights: `(InnerConeAngle, OuterConeAngle)` in degrees.
    pub cone: Option<(f32, f32)>,
}

impl Light {
    /// Unreal's `AffectsBounds` for point and spot lights, against bounds given as a sphere.
    pub fn affects(&self, centre: Vec3, bounds_radius: f32) -> bool {
        if (centre - self.position).length_squared() > (self.radius + bounds_radius).powi(2) {
            return false;
        }
        let Some((inner, outer)) = self.cone else { return true };
        // USpotLightComponent::AffectsBounds: the bounds' sphere against the cone, its apex moved back so the
        // cone's sides clear the sphere.
        let inner = inner.clamp(0.0, 89.0).to_radians();
        let outer = outer.to_radians().clamp(inner + 0.001, 89f32.to_radians() + 0.001);
        let (sin, cos) = outer.sin_cos();
        let apex = self.position - self.direction * (bounds_radius / sin);
        let d = centre - apex;
        let e = self.direction.dot(d);
        if !(e > 0.0 && e * e >= d.length_squared() * cos * cos) {
            return false;
        }
        let d = centre - self.position;
        let e = -self.direction.dot(d);
        if e > 0.0 && e * e >= d.length_squared() * sin * sin {
            d.length_squared() <= bounds_radius * bounds_radius
        } else {
            true
        }
    }

    /// The light's score at `point`.
    pub fn strength_at(&self, point: Vec3) -> f32 {
        let d = (point - self.position).length();
        (1.0 - d * (1.0 / self.radius)).max(0.0).powf(self.falloff_exponent + 1.0) * self.radius * STRENGTH_SCALE
    }
}

/// Whether a capsule centred at `centre` with bounds radius `bounds_radius` is lit (flashlight off).
pub fn is_lit(lights: &[Light], centre: Vec3, bounds_radius: f32, min_strength: f32) -> bool {
    lights.iter().any(|l| l.affects(centre, bounds_radius) && l.strength_at(centre) > min_strength)
}

/// Bounds radius of an upright capsule (Unreal's capsule bounds: the box round it, as a sphere).
/// A light's colour temperature as linear RGB (Unreal's FLinearColor::MakeFromColorTemperature): the Planckian
/// locus in CIE 1960 UCS, to XYZ with Y = 1, to linear sRGB. Unreal clamps the temperature to 1000..15000 K.
pub fn temperature_color(kelvin: f32) -> [f32; 3] {
    let t = kelvin.clamp(1000.0, 15000.0) as f64;
    let u = (0.860117757 + 1.54118254e-4 * t + 1.28641212e-7 * t * t) / (1.0 + 8.42420235e-4 * t + 7.08145163e-7 * t * t);
    let v = (0.317398726 + 4.22806245e-5 * t + 4.20481691e-8 * t * t) / (1.0 - 2.89741816e-5 * t + 1.61456053e-7 * t * t);
    let x = 3.0 * u / (2.0 * u - 8.0 * v + 4.0);
    let y = 2.0 * v / (2.0 * u - 8.0 * v + 4.0);
    let (cx, cy, cz) = (x / y, 1.0, (1.0 - x - y) / y);
    let r = 3.2404542 * cx - 1.5371385 * cy - 0.4985314 * cz;
    let g = -0.9692660 * cx + 1.8760108 * cy + 0.0415560 * cz;
    let b = 0.0556434 * cx - 0.2040259 * cy + 1.0572252 * cz;
    [r as f32, g as f32, b as f32]
}

pub fn capsule_bounds_radius(radius: f32, half_height: f32) -> f32 {
    (2.0 * radius * radius + half_height * half_height).sqrt()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn point(radius: f32) -> Light {
        Light { position: Vec3::ZERO, direction: Vec3::X, radius, falloff_exponent: 2.0, cone: None }
    }

    #[test]
    fn a_point_light_lights_her_within_its_radius() {
        let l = [point(500.0)];
        let r = capsule_bounds_radius(34.0, 82.0);
        assert!(is_lit(&l, Vec3::new(300.0, 0.0, 0.0), r, 0.3));
        // Near the edge the score drops under MinLightStrength: (1 - 495/500)^3 * 500 * 1.25 = 0.00078.
        assert!(!is_lit(&l, Vec3::new(495.0, 0.0, 0.0), r, 0.3));
        assert!(!is_lit(&l, Vec3::new(700.0, 0.0, 0.0), r, 0.3));
    }

    #[test]
    fn colour_temperature_is_warm_below_daylight_and_blue_above() {
        // About white at 6500 K (D65 is 6504 K); 2700 K is orange, 12000 K blue.
        let d = temperature_color(6504.0);
        assert!(d.iter().all(|c| (c - 1.0).abs() < 0.05), "{d:?}");
        let warm = temperature_color(2700.0);
        assert!(warm[0] > warm[1] && warm[1] > warm[2]);
        let cold = temperature_color(12000.0);
        assert!(cold[2] > cold[1] && cold[1] > cold[0]);
    }

    #[test]
    fn strength_falls_off_with_the_exponent() {
        let l = point(1000.0);
        // (1 - 0.5)^3 * 1000 * 1.25
        assert!((l.strength_at(Vec3::new(500.0, 0.0, 0.0)) - 156.25).abs() < 1e-3);
    }

    #[test]
    fn a_spot_light_lights_only_its_cone() {
        // A lamp 4 m up pointing straight down with a 45 degree cone.
        let l = Light { position: Vec3::new(0.0, 0.0, 400.0), direction: Vec3::NEG_Z, radius: 1200.0, falloff_exponent: 2.0, cone: Some((20.0, 45.0)) };
        let r = capsule_bounds_radius(34.0, 82.0);
        assert!(is_lit(&[l], Vec3::new(100.0, 0.0, 82.0), r, 0.3));
        // 8 m to the side: inside the radius, outside the cone.
        assert!(l.position.distance(Vec3::new(800.0, 0.0, 82.0)) < l.radius);
        assert!(!is_lit(&[l], Vec3::new(800.0, 0.0, 82.0), r, 0.3));
        // Above the lamp.
        assert!(!is_lit(&[l], Vec3::new(0.0, 0.0, 700.0), r, 0.3));
    }

    #[test]
    fn capsule_bounds_match_unreal() {
        assert!((capsule_bounds_radius(34.0, 82.0) - 95.06).abs() < 0.01);
    }
}
