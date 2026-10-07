//! A car's [`VehicleSetup`] from its class (classes.json): the SCWheeledVehicleMovementComponent's EngineSetup,
//! TransmissionSetup, IdleBrakeInput (and Mass, DragCoefficient, ChassisWidth/Height when set), and its front wheel
//! class's ShapeRadius and SteerAngle; Unreal's defaults for the rest.

use crate::classes::ClassDb;
use f13_core::vehicle::{Gear, VehicleSetup};
use serde_json::Value;

pub fn setup(classes: &ClassDb, class: &str) -> VehicleSetup {
    let mut s = VehicleSetup::default();
    let Some(info) = classes.get(class) else { return s };
    let Some(m) = info.components.iter().find(|c| c.kind.contains("WheeledVehicleMovement")).map(|c| &c.props) else { return s };
    let f = |v: &Value| v.as_f64().map(|x| x as f32);
    let e = &m["EngineSetup"];
    let keys: Vec<(f32, f32)> = e["TorqueCurve"]["EditorCurveData"]["Keys"]
        .as_array()
        .map_or(vec![], |k| k.iter().filter_map(|k| Some((f(&k["Time"])?, f(&k["Value"])?))).collect());
    if !keys.is_empty() {
        s.torque_curve = keys;
    }
    if let Some(r) = f(&e["MaxRPM"]) {
        s.max_rpm = r;
    }
    let t = &m["TransmissionSetup"];
    let gears: Vec<Gear> = t["ForwardGears"]
        .as_array()
        .map_or(vec![], |g| g.iter().filter_map(|g| Some(Gear { ratio: f(&g["Ratio"])?, down: f(&g["DownRatio"]).unwrap_or(0.5), up: f(&g["UpRatio"]).unwrap_or(0.65) })).collect());
    if !gears.is_empty() {
        s.gears = gears;
    }
    s.shift_latency = f(&t["GearAutoBoxLatency"]).unwrap_or(2.0);
    s.final_ratio = f(&t["FinalRatio"]).unwrap_or(s.final_ratio);
    s.reverse_ratio = f(&t["ReverseGearRatio"]).map_or(s.reverse_ratio, f32::abs);
    s.idle_brake = f(&m["IdleBrakeInput"]).unwrap_or(0.0);
    s.mass_kg = f(&m["Mass"]).unwrap_or(s.mass_kg);
    s.drag = f(&m["DragCoefficient"]).unwrap_or(s.drag);
    s.chassis_cm = (f(&m["ChassisWidth"]).unwrap_or(s.chassis_cm.0), f(&m["ChassisHeight"]).unwrap_or(s.chassis_cm.1));
    // The front wheels (the first WheelSetup): radius and steering lock (UVehicleWheel: 30 cm, 70 degrees).
    if let Some(wheel) = m["WheelSetups"][0]["WheelClass"].as_str() {
        s.wheel_radius_cm = f(classes.prop(wheel, "ShapeRadius")).unwrap_or(30.0);
        s.steer_deg = f(classes.prop(wheel, "SteerAngle")).unwrap_or(70.0);
        s.brake_torque = f(classes.prop(wheel, "MaxBrakeTorque")).unwrap_or(s.brake_torque);
    }
    s
}
