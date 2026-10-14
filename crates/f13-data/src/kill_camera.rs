//! `kill_cameras.json` (f13-export): each kill's camera spline (SCCameraSplineCurves as Unreal InterpCurve points), by
//! kill class, into f13-core's [`CameraSpline`].

use f13_core::camera_spline::{CameraSpline, Curve, Interp, Point};
use glam::{Quat, Vec3};
use serde_json::Value;
use std::collections::HashMap;
use std::path::Path;

fn f(v: &Value) -> f32 {
    v.as_f64().unwrap_or(0.0) as f32
}

fn vec3(v: &Value) -> Vec3 {
    Vec3::new(f(&v["X"]), f(&v["Y"]), f(&v["Z"]))
}

fn quat(v: &Value) -> Quat {
    if v.is_object() { Quat::from_xyzw(f(&v["X"]), f(&v["Y"]), f(&v["Z"]), v["W"].as_f64().unwrap_or(1.0) as f32) } else { Quat::IDENTITY }
}

fn curve<T>(v: &Value, value: impl Fn(&Value) -> T) -> Curve<T> {
    let points = v["Points"].as_array().map_or(vec![], |p| p.clone());
    Curve(
        points
            .iter()
            .map(|p| Point {
                key: f(&p["InVal"]),
                out: value(&p["OutVal"]),
                arrive: value(&p["ArriveTangent"]),
                leave: value(&p["LeaveTangent"]),
                mode: Interp::from_name(p["InterpMode"].as_str().unwrap_or("CIM_Linear")),
            })
            .collect(),
    )
}

/// One kill's spline from its SplineCurves JSON.
pub fn parse(v: &Value) -> CameraSpline {
    CameraSpline {
        position: curve(&v["Position"], vec3),
        rotation: curve(&v["Rotation"], quat),
        scale: curve(&v["Scale"], vec3),
        fov: curve(&v["FOVCurve"], f),
        hold: curve(&v["HoldTimeCurve"], f),
        time_to_next: curve(&v["TimeToNextCurve"], f),
    }
}

/// Kill class -> its spline component's place under the kill's root (Unreal cm and rotation; identity when the export
/// has none).
pub fn load_frames(path: &Path) -> Result<HashMap<String, (Vec3, Quat)>, crate::LoadError> {
    let text = std::fs::read_to_string(path).map_err(crate::LoadError::Io)?;
    let v: Value = serde_json::from_str(&text).map_err(|e| crate::LoadError::Invalid(format!("{}: {e}", path.display())))?;
    Ok(v.as_object().map_or(HashMap::new(), |o| o.iter().map(|(k, v)| (k.clone(), frame(v))).collect()))
}

/// A spline's RelativeLocation and RelativeRotation (a rotator, degrees).
pub fn frame(v: &Value) -> (Vec3, Quat) {
    let r = &v["RelativeRotation"];
    let rotation = f13_core::camera_spline::quaternion(Vec3::new(f(&r["Pitch"]), f(&r["Yaw"]), f(&r["Roll"])));
    (vec3(&v["RelativeLocation"]), rotation)
}

/// Kill class -> its SCSplineCamera's HandicamShakeFrequency, where the export has it.
pub fn load_frequencies(path: &Path) -> Result<HashMap<String, Vec3>, crate::LoadError> {
    let text = std::fs::read_to_string(path).map_err(crate::LoadError::Io)?;
    let v: Value = serde_json::from_str(&text).map_err(|e| crate::LoadError::Invalid(format!("{}: {e}", path.display())))?;
    Ok(v.as_object().map_or(HashMap::new(), |o| o.iter().filter(|(_, v)| v["HandicamShakeFrequency"].is_object()).map(|(k, v)| (k.clone(), vec3(&v["HandicamShakeFrequency"]))).collect()))
}

/// Kill class -> its camera spline.
pub fn load(path: &Path) -> Result<HashMap<String, CameraSpline>, crate::LoadError> {
    let text = std::fs::read_to_string(path).map_err(crate::LoadError::Io)?;
    let v: Value = serde_json::from_str(&text).map_err(|e| crate::LoadError::Invalid(format!("{}: {e}", path.display())))?;
    Ok(v.as_object().map_or(HashMap::new(), |o| o.iter().map(|(k, v)| (k.clone(), parse(v))).collect()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_curves() {
        let v = serde_json::json!({
            "TimeToNextCurve": {"Points": [{"InVal": 0.0, "OutVal": 12.4, "ArriveTangent": 0.0, "LeaveTangent": 0.0, "InterpMode": "CIM_Constant"},
                                           {"InVal": 1.0, "OutVal": 1.0, "ArriveTangent": 0.0, "LeaveTangent": 0.0, "InterpMode": "CIM_Constant"}]},
            "Position": {"Points": [{"InVal": 0.0, "OutVal": {"X": 1.0, "Y": 2.0, "Z": 3.0}, "ArriveTangent": {"X": 0.0, "Y": 0.0, "Z": 0.0},
                                     "LeaveTangent": {"X": 0.0, "Y": 0.0, "Z": 0.0}, "InterpMode": "CIM_CurveUser"},
                                    {"InVal": 1.0, "OutVal": {"X": 3.0, "Y": 2.0, "Z": 1.0}, "ArriveTangent": {"X": 0.0, "Y": 0.0, "Z": 0.0},
                                     "LeaveTangent": {"X": 0.0, "Y": 0.0, "Z": 0.0}, "InterpMode": "CIM_CurveUser"}]},
            "Rotation": {"Points": [{"InVal": 0.0, "OutVal": {"X": 0.0, "Y": 0.0, "Z": -0.793354, "W": 0.608761, "IsNormalized": true},
                                     "ArriveTangent": {"X": 0.0, "Y": 0.0, "Z": 0.0, "W": 1.0}, "LeaveTangent": {"X": 0.0, "Y": 0.0, "Z": 0.0, "W": 1.0},
                                     "InterpMode": "CIM_Linear"}]}
        });
        let s = parse(&v);
        assert_eq!(s.position.0.len(), 2);
        assert_eq!(s.time_to_next.0[0].mode, Interp::Constant);
        assert!((s.duration() - 12.4).abs() < 1e-5);
        assert_eq!(s.at(6.2).location, Vec3::new(2.0, 2.0, 2.0));
        assert_eq!(s.rotation.0[0].out.z, -0.793354);
        assert!(s.fov.0.is_empty());
        assert_eq!(frame(&v), (Vec3::ZERO, Quat::IDENTITY));
    }

    #[test]
    fn frame_turns_and_moves_the_spline() {
        let v = serde_json::json!({"RelativeLocation": {"X": 0.0, "Y": 400.0, "Z": 100.0}, "RelativeRotation": {"Pitch": 0.0, "Yaw": -90.0, "Roll": 0.0}});
        let (t, r) = frame(&v);
        assert_eq!(t, Vec3::new(0.0, 400.0, 100.0));
        // Yaw -90: the spline's +X is the kill's -Y.
        assert!((r * Vec3::X - Vec3::NEG_Y).length() < 1e-5);
    }
}
