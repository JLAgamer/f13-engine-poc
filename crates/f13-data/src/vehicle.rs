//! A car's [`VehicleSetup`] from its class (classes.json): the SCWheeledVehicleMovementComponent's EngineSetup,
//! TransmissionSetup, IdleBrakeInput (and Mass, DragCoefficient, ChassisWidth/Height when set), and its front wheel
//! class's ShapeRadius and SteerAngle; Unreal's defaults for the rest.

use crate::classes::ClassDb;
use f13_core::vehicle::{Gear, VehicleSetup};
use f13_core::vehicle_px::{CarSetup, WheelSetup};
use glam::{Quat, Vec3};
use serde_json::Value;

/// A car's PhysX setup (f13-core vehicle_px) from its class: the movement component's Blueprint values over
/// UWheeledVehicleMovementComponent's and ...4W's constructors (0x141f86e90, 0x141f869b0), each WheelSetup's wheel
/// class over UVehicleWheel's (0x141f86830) at its bone (plus AdditionalOffset), and its mesh's physics asset root box
/// (the chassis's mass properties). None without a movement component, wheels or a root box.
pub fn px_setup(classes: &ClassDb, class: &str) -> Option<CarSetup> {
    let info = classes.get(class)?;
    let m = &info.components.iter().find(|c| c.kind.contains("WheeledVehicleMovement"))?.props;
    let mesh = info.components.iter().find(|c| c.root_body.is_some())?;
    let body = mesh.root_body.as_ref()?;
    let f = |v: &Value| v.as_f64().map(|x| x as f32);
    let v3 = |v: &Value| Some(Vec3::new(f(&v[0])?, f(&v[1])?, f(&v[2])?));
    let com = v3(&body["Center"])?;
    let size = v3(&body["Size"])?;
    let rot = v3(&body["Rotation"])?;
    // FRotator (pitch, yaw, roll) -> quaternion (yaw about Z, pitch about Y, roll about X).
    let box_rotation = Quat::from_euler(glam::EulerRot::ZYX, rot.y.to_radians(), -rot.x.to_radians(), -rot.z.to_radians());
    let e = &m["EngineSetup"];
    let torque_curve: Vec<(f32, f32)> =
        e["TorqueCurve"]["EditorCurveData"]["Keys"].as_array().map_or(vec![], |k| k.iter().filter_map(|k| Some((f(&k["Time"])?, f(&k["Value"])?))).collect());
    // The steering lock against speed (km/h): the Blueprint's, else the 4W constructor's four keys.
    let steering_curve: Vec<(f32, f32)> = m["SteeringCurve"]["EditorCurveData"]["Keys"]
        .as_array()
        .map(|k| k.iter().filter_map(|k| Some((f(&k["Time"])?, f(&k["Value"])?))).collect())
        .filter(|k: &Vec<(f32, f32)>| !k.is_empty())
        .unwrap_or_else(|| vec![(0.0, 1.0), (20.0, 0.9), (60.0, 0.8), (120.0, 0.7)]);
    let t = &m["TransmissionSetup"];
    let gears: Vec<(f32, f32, f32)> = t["ForwardGears"]
        .as_array()
        .map_or(vec![], |g| g.iter().filter_map(|g| Some((f(&g["Ratio"])?, f(&g["DownRatio"]).unwrap_or(0.5), f(&g["UpRatio"]).unwrap_or(0.65)))).collect());
    let d = &m["DifferentialSetup"];
    let scale = &m["InertiaTensorScale"];
    let rate = |k: &str, rise: f32, fall: f32| (f(&m[k]["RiseRate"]).unwrap_or(rise), f(&m[k]["FallRate"]).unwrap_or(fall));
    let setups = m["WheelSetups"].as_array()?;
    let wheel = |i: usize| -> Option<WheelSetup> {
        let ws = setups.get(i)?;
        let wc = ws["WheelClass"].as_str()?;
        let p = |k: &str, def: f32| f(classes.prop(wc, k)).unwrap_or(def);
        let bone = mesh.bone_positions.get(ws["BoneName"].as_str()?)?;
        let add = Vec3::new(f(&ws["AdditionalOffset"]["X"]).unwrap_or(0.0), f(&ws["AdditionalOffset"]["Y"]).unwrap_or(0.0), f(&ws["AdditionalOffset"]["Z"]).unwrap_or(0.0));
        Some(WheelSetup {
            position: Vec3::from_array(*bone) + add,
            radius: p("ShapeRadius", 30.0),
            width: p("ShapeWidth", 10.0),
            mass: p("Mass", 20.0),
            damping_rate: p("DampingRate", 0.25),
            steer_deg: p("SteerAngle", 70.0),
            max_brake: p("MaxBrakeTorque", 1500.0),
            max_handbrake: p("MaxHandBrakeTorque", 3000.0),
            affected_by_handbrake: classes.prop(wc, "bAffectedByHandbrake").as_bool().unwrap_or(true),
            lat_stiff_max_load: p("LatStiffMaxLoad", 2.0),
            lat_stiff_value: p("LatStiffValue", 17.0),
            long_stiff_value: p("LongStiffValue", 1000.0),
            suspension_force_offset: p("SuspensionForceOffset", 0.0),
            max_raise: p("SuspensionMaxRaise", 10.0),
            max_drop: p("SuspensionMaxDrop", 10.0),
            natural_frequency: p("SuspensionNaturalFrequency", 7.0),
            damping_ratio: p("SuspensionDampingRatio", 1.0),
            friction: 1.0,
        })
    };
    Some(CarSetup {
        mass: f(&m["Mass"]).unwrap_or(1500.0),
        com,
        box_size: size,
        box_rotation,
        inertia_scale: Vec3::new(f(&scale["X"]).unwrap_or(1.0), f(&scale["Y"]).unwrap_or(1.0), f(&scale["Z"]).unwrap_or(1.0)),
        torque_curve,
        max_rpm: f(&e["MaxRPM"]).unwrap_or(5729.578),
        engine_moi: f(&e["MOI"]).unwrap_or(1.0),
        damping_full_throttle: f(&e["DampingRateFullThrottle"]).unwrap_or(0.15),
        damping_zero_throttle_engaged: f(&e["DampingRateZeroThrottleClutchEngaged"]).unwrap_or(2.0),
        damping_zero_throttle_disengaged: f(&e["DampingRateZeroThrottleClutchDisengaged"]).unwrap_or(0.35),
        gears,
        reverse_ratio: f(&t["ReverseGearRatio"]).unwrap_or(-4.0),
        final_ratio: f(&t["FinalRatio"]).unwrap_or(4.0),
        gear_switch_time: f(&t["GearSwitchTime"]).unwrap_or(0.5),
        autobox_latency: f(&t["GearAutoBoxLatency"]).unwrap_or(2.0),
        neutral_gear_up_ratio: f(&t["NeutralGearUpRatio"]).unwrap_or(0.15),
        clutch_strength: f(&t["ClutchStrength"]).unwrap_or(10.0),
        front_left_right_split: f(&d["FrontLeftRightSplit"]).unwrap_or(0.5),
        ackermann_accuracy: f(&m["AckermannAccuracy"]).unwrap_or(1.0),
        steering_curve,
        drag_coefficient: f(&m["DragCoefficient"]).unwrap_or(0.3),
        chassis_width: f(&m["ChassisWidth"]).unwrap_or(180.0),
        chassis_height: f(&m["ChassisHeight"]).unwrap_or(140.0),
        idle_brake: f(&m["IdleBrakeInput"]).unwrap_or(0.0),
        stop_threshold: f(&m["StopThreshold"]).unwrap_or(10.0),
        wrong_direction_threshold: f(&m["WrongDirectionThreshold"]).unwrap_or(100.0),
        throttle_rate: rate("ThrottleInputRate", 6.0, 10.0),
        brake_rate: rate("BrakeInputRate", 6.0, 10.0),
        handbrake_rate: rate("HandbrakeInputRate", 12.0, 12.0),
        steering_rate: rate("SteeringInputRate", 2.5, 5.0),
        threshold_longitudinal_speed: f(&m["ThresholdLongitudinalSpeed"]).unwrap_or(5.0),
        low_speed_substeps: m["LowForwardSpeedSubStepCount"].as_u64().unwrap_or(3) as u32,
        high_speed_substeps: m["HighForwardSpeedSubStepCount"].as_u64().unwrap_or(1) as u32,
        load_filter: [
            f(&m["MinNormalizedTireLoad"]).unwrap_or(0.0),
            f(&m["MinNormalizedTireLoadFiltered"]).unwrap_or(0.2308),
            f(&m["MaxNormalizedTireLoad"]).unwrap_or(3.0),
            f(&m["MaxNormalizedTireLoadFiltered"]).unwrap_or(3.0),
        ],
        wheels: [wheel(0)?, wheel(1)?, wheel(2)?, wheel(3)?],
    })
}

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
