//! Cascade particle systems (`particles/<name>.json` from `f13-export particles`): each emitter's first LOD level as
//! its required and spawn modules and the per-particle modules f13-game runs, in their order. Unreal units (cm, Z up,
//! rotations in turns).
//!
//! Cooked modules keep their distributions as FRawDistribution lookup tables (UE 4.18 FDistributionLookupTable):
//! `Op` 1 a value per entry, 2 a min and a max per entry (a random lerp between them), 3 the extremes; entries are
//! looked up at `(t - TimeBias) * TimeScale`, lerped. Where a table is missing the distribution object's own
//! properties are used (Constant, Uniform, ConstantCurve).

use glam::Vec3;
use serde_json::Value;
use std::path::Path;

/// A float or vector distribution (one to three components).
#[derive(Clone, Debug, Default)]
pub struct Dist {
    /// 1 none, 2 random, 3 extreme.
    pub op: u8,
    pub count: usize,
    pub stride: usize,
    pub time_scale: f32,
    pub time_bias: f32,
    pub values: Vec<f32>,
    /// FDistributionLookupTable LockFlag (vector random): 0 none, 1 XY, 2 XZ, 3 YZ, 4 XYZ.
    pub lock: u8,
    /// A curve (an uncooked ConstantCurve): (in, out per component, arrive and leave tangents per component).
    pub curve: Vec<CurvePoint>,
}

#[derive(Clone, Debug, Default)]
pub struct CurvePoint {
    pub t: f32,
    pub v: [f32; 3],
    pub arrive: [f32; 3],
    pub leave: [f32; 3],
    /// CIM_Linear, CIM_Constant or one of the curve modes.
    pub linear: bool,
    pub constant: bool,
}

impl Dist {
    pub fn constant(v: &[f32]) -> Dist {
        Dist { op: 1, count: 1, stride: v.len(), values: v.to_vec(), ..Default::default() }
    }

    /// Component `i` (`n` components) at `t`, `r` the random values (one per component, 0..1).
    pub fn eval_n(&self, t: f32, r: [f32; 3], n: usize) -> [f32; 3] {
        let mut out = [0.0; 3];
        if !self.curve.is_empty() {
            let p = &self.curve;
            let v = if t <= p[0].t {
                p[0].v
            } else if t >= p[p.len() - 1].t {
                p[p.len() - 1].v
            } else {
                let i = p.iter().position(|k| k.t > t).unwrap_or(p.len() - 1);
                let (a, b) = (&p[i - 1], &p[i]);
                let d = b.t - a.t;
                let x = if d > 0.0 { (t - a.t) / d } else { 0.0 };
                let mut v = [0.0; 3];
                for c in 0..3 {
                    v[c] = if a.constant {
                        a.v[c]
                    } else if a.linear {
                        a.v[c] + (b.v[c] - a.v[c]) * x
                    } else {
                        // FMath::CubicInterp(P0, T0 * d, P1, T1 * d, x)
                        let (p0, p1, m0, m1) = (a.v[c], b.v[c], a.leave[c] * d, b.arrive[c] * d);
                        let (x2, x3) = (x * x, x * x * x);
                        (2.0 * x3 - 3.0 * x2 + 1.0) * p0 + (x3 - 2.0 * x2 + x) * m0 + (x3 - x2) * m1 + (-2.0 * x3 + 3.0 * x2) * p1
                    };
                }
                v
            };
            return v;
        }
        if self.values.is_empty() || self.count == 0 {
            return out;
        }
        let x = ((t - self.time_bias) * self.time_scale).max(0.0);
        let alpha = x.fract();
        let i = x as usize;
        let (e1, e2) = (i.min(self.count - 1) * self.stride, (i + 1).min(self.count - 1) * self.stride);
        let at = |e: usize, k: usize| self.values.get(e + k).copied().unwrap_or(0.0);
        let mut r = r;
        match self.lock {
            1 => r[1] = r[0],
            2 => r[2] = r[0],
            3 => r[2] = r[1],
            4 => {
                r[1] = r[0];
                r[2] = r[0];
            }
            _ => {}
        }
        for c in 0..n {
            let lo = at(e1, c) + (at(e2, c) - at(e1, c)) * alpha;
            out[c] = match self.op {
                2 | 3 => {
                    let hi = at(e1, n + c) + (at(e2, n + c) - at(e1, n + c)) * alpha;
                    if self.op == 3 {
                        if r[c] > 0.5 { hi } else { lo }
                    } else {
                        lo + (hi - lo) * r[c]
                    }
                }
                _ => lo,
            };
        }
        out
    }

    pub fn eval(&self, t: f32, r: f32) -> f32 {
        self.eval_n(t, [r; 3], 1)[0]
    }

    pub fn eval3(&self, t: f32, r: [f32; 3]) -> Vec3 {
        Vec3::from(self.eval_n(t, r, 3))
    }

    /// Whether the value can differ between particles (so it draws random numbers).
    pub fn random(&self) -> bool {
        self.op >= 2
    }
}

#[derive(Clone, Debug)]
pub enum Module {
    Lifetime(Dist),
    /// StartLocation (cm, emitter space).
    Location(Dist),
    /// StartRadius, VelocityScale, StartLocation (ParticleModuleLocationPrimitiveSphere: on or in the sphere, surface
    /// only not handled).
    LocationSphere { radius: Dist, velocity: bool, velocity_scale: Dist, origin: Dist },
    /// StartVelocity, StartVelocityRadial, bInWorldSpace.
    Velocity { start: Dist, radial: Dist, world: bool },
    Size(Dist),
    /// LifeMultiplier, MultiplyX/Y/Z.
    SizeMultiplyLife { life: Dist, axes: [bool; 3] },
    /// SpeedScale, MaxScale.
    SizeScaleBySpeed { speed: [f32; 2], max: [f32; 2] },
    /// ColorOverLife, AlphaOverLife.
    ColorOverLife { color: Dist, alpha: Dist },
    /// StartRotation (turns).
    Rotation(Dist),
    /// StartRotationRate (turns per second).
    RotationRate(Dist),
    RotationRateMultiplyLife(Dist),
    AccelerationConstant { accel: Vec3, world: bool },
    /// DragCoefficientRaw (over the particle's life).
    Drag(Dist),
    /// OffsetAmount, RotationAmount, RotationRateAmount (each with its options: at spawn only by default).
    Orbit { offset: Dist, rotation: Dist, rate: Dist, offset_life: bool },
    /// SubImageIndex over the particle's life.
    SubUV(Dist),
    /// The dynamic parameters (up to four), each: value distribution, spawn time only, by emitter time.
    Dynamic([Option<(Dist, bool, bool)>; 4]),
    /// ColorScaleOverLife, BrightnessOverLife, RadiusScale, LightExponent (InitializeDefaults 0x1415aa300: 1, 32, 15,
    /// 16), bUseInverseSquaredFalloff (default on), SpawnFraction (default 1), bHighQualityLights.
    Light { color: Dist, brightness: Dist, radius: Dist, exponent: Dist, inverse_squared: bool, spawn_fraction: f32, high_quality: bool },
    PivotOffset([f32; 2]),
    /// ParticleModuleOrientationAxisLock's LockAxisFlags (EPAL_...).
    AxisLock(String),
    /// ParticleModuleMeshRotation's StartRotation (turns, x 360 into the mesh rotation payload's InitRotation).
    MeshRotation(Dist),
    /// ParticleModuleMeshRotationRate's StartRotationRate (turns per second, x 360: the payload's RotationRate).
    MeshRotationRate(Dist),
    /// ParticleModuleAttractorPoint (Update 0x1415b2a70): Position, Range, Strength, and its flags (bit 0
    /// bStrengthByDistance, 1 bAffectBaseVelocity, 2 bOverrideVelocity, 3 bUseWorldSpacePosition, 4-6 Positive_X/Y/Z,
    /// 7-9 Negative_X/Y/Z; the constructor 0x141592e20 sets 0x3f1).
    AttractorPoint { position: Dist, range: Dist, strength: Dist, flags: u32 },
    /// ParticleModuleAttractorParticle (Spawn 0x1415adf20, Update 0x1415b2480): the attracting emitter, Range,
    /// Strength, bStrengthByDistance (constructor 0x141592d00: on), bAffectBaseVelocity, SelectionMethod random,
    /// bRenewSource, bInheritSourceVel (off).
    AttractorParticle { emitter: String, range: Dist, strength: Dist, by_distance: bool, affect_base: bool, random: bool, renew: bool, inherit: bool },
    /// ParticleModuleLocationPrimitiveCylinder (SpawnEx 0x1415e1ca0): StartRadius, StartHeight, StartLocation,
    /// HeightAxis (0 X, 1 Y, 2 Z), SurfaceOnly, Velocity, VelocityScale.
    LocationCylinder { radius: Dist, height: Dist, origin: Dist, axis: u8, surface_only: bool, velocity: bool, velocity_scale: Dist },
    /// ParticleModuleSourceMovement (FinalUpdate 0x14159bc20): SourceMovementScale.
    SourceMovement(Dist),
    /// ParticleModuleCollision (Spawn 0x1415df650, Update 0x1415e4920): DampingFactor, DampingFactorRotation,
    /// MaxCollisions, CollisionCompletionOption (0 kill, 1 freeze, 2 halt collisions, 3 freeze translation, 4 freeze
    /// rotation, 5 freeze movement), DelayAmount, bOnlyVerticalNormalsDecrementCount, VerticalFudgeFactor and DirScalar
    /// (constructor 0x1415b95f0: 0.1 and 3.5).
    Collision { damping: Dist, damping_rotation: Dist, max_collisions: Dist, completion: u8, delay: Dist, only_vertical: bool, vertical_fudge: f32, dir_scalar: f32 },
    /// A module with no CPU effect (ParticleModuleAttractorPointGravity overrides only its GPU compile, vt +0x230).
    GpuOnly,
    /// A module f13-game doesn't run (its type).
    Unported(String),
}

#[derive(Clone, Debug)]
pub struct Burst {
    pub time: f32,
    pub count: u32,
    pub count_low: i32,
}

#[derive(Clone, Debug)]
pub struct Emitter {
    pub name: String,
    pub material: Option<String>,
    pub origin: Vec3,
    /// EmitterRotation (degrees: pitch, yaw, roll).
    pub rotation: Vec3,
    /// PSA_Square, PSA_Rectangle, PSA_Velocity, ...
    pub screen_alignment: String,
    pub local_space: bool,
    pub duration: f32,
    pub loops: u32,
    pub delay: f32,
    pub delay_first_loop_only: bool,
    pub sub_images: [u32; 2],
    /// PSUVIM_None, PSUVIM_Linear, PSUVIM_Linear_Blend, PSUVIM_Random, ...
    pub interpolation: String,
    pub max_draw_count: Option<u32>,
    pub rate: Dist,
    pub rate_scale: Dist,
    pub burst_scale: Dist,
    pub bursts: Vec<Burst>,
    /// SpawnPerUnit (particles per UnitScalar cm moved).
    pub spawn_per_unit: Option<(f32, Dist)>,
    pub gpu: bool,
    /// ParticleModuleTypeDataMesh: its particles are the mesh.
    pub mesh: Option<MeshData>,
    pub modules: Vec<Module>,
}

/// A mesh emitter's ParticleModuleTypeDataMesh.
#[derive(Clone, Debug)]
pub struct MeshData {
    /// The static mesh's glTF (export-relative).
    pub file: String,
    /// RollPitchYawRange (degrees, the payload's InitialOrientation: the pre-rotation).
    pub roll_pitch_yaw: Dist,
    /// bOverrideMaterial: the required module's material instead of the mesh's.
    pub override_material: bool,
    /// bCameraFacing, AxisLockOption, CameraFacingOption, bApplyParticleRotationAsSpin (not drawn yet when set).
    pub camera_facing: bool,
    pub axis_lock: String,
    pub camera_facing_option: String,
    pub spin: bool,
}

#[derive(Clone, Debug)]
pub struct System {
    pub name: String,
    pub warmup: f32,
    pub emitters: Vec<Emitter>,
}

fn num(v: &Value, d: f32) -> f32 {
    v.as_f64().map_or(d, |x| x as f32)
}

fn vec3(v: &Value) -> Vec3 {
    Vec3::new(num(&v["X"], 0.0), num(&v["Y"], 0.0), num(&v["Z"], 0.0))
}

impl System {
    pub fn load(path: &Path) -> Option<System> {
        let v: Value = serde_json::from_str(&std::fs::read_to_string(path).ok()?).ok()?;
        let exports = v["Exports"].as_array()?;
        let get = |r: &Value| -> Option<&Value> {
            let p = r["ObjectPath"].as_str()?;
            let i: usize = p.rsplit('.').next()?.parse().ok()?;
            exports.get(i)
        };
        // A distribution property: its table, else its distribution object.
        let dist = |p: &Value, n: usize| -> Dist {
            let t = &p["Table"];
            if t["Values"].as_array().is_some_and(|a| !a.is_empty()) {
                let values: Vec<f32> = t["Values"].as_array().unwrap().iter().map(|x| num(x, 0.0)).collect();
                return Dist {
                    op: t["Op"].as_u64().unwrap_or(1) as u8,
                    count: t["EntryCount"].as_u64().unwrap_or(1) as usize,
                    stride: t["EntryStride"].as_u64().unwrap_or(n as u64) as usize,
                    time_scale: num(&t["TimeScale"], 0.0),
                    time_bias: num(&t["TimeBias"], 0.0),
                    values,
                    lock: t["LockFlag"].as_u64().unwrap_or(0) as u8,
                    curve: vec![],
                };
            }
            let Some(d) = get(&p["Distribution"]) else { return Dist::constant(&vec![0.0; n]) };
            let dp = &d["Props"];
            let comps = |x: &Value| -> Vec<f32> { if n == 1 { vec![num(x, 0.0)] } else { vec![num(&x["X"], 0.0), num(&x["Y"], 0.0), num(&x["Z"], 0.0)] } };
            match d["Type"].as_str().unwrap_or("") {
                "DistributionFloatConstant" | "DistributionVectorConstant" => Dist::constant(&comps(&dp["Constant"])),
                "DistributionFloatUniform" | "DistributionVectorUniform" => {
                    let mut values = comps(&dp["Min"]);
                    values.extend(comps(&dp["Max"]));
                    let lock = if dp["bLockAxes"].as_bool().unwrap_or(false) {
                        match dp["LockedAxes"].as_str().unwrap_or("") {
                            "EDVLF_XY" => 1,
                            "EDVLF_XZ" => 2,
                            "EDVLF_YZ" => 3,
                            "EDVLF_XYZ" => 4,
                            _ => 0,
                        }
                    } else {
                        0
                    };
                    Dist { op: 2, count: 1, stride: 2 * n, values, lock, ..Default::default() }
                }
                "DistributionFloatConstantCurve" | "DistributionVectorConstantCurve" => {
                    let curve = dp["ConstantCurve"]["Points"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .map(|k| {
                            let c = |x: &Value| -> [f32; 3] {
                                let v = comps(x);
                                [v[0], v.get(1).copied().unwrap_or(v[0]), v.get(2).copied().unwrap_or(v[0])]
                            };
                            let mode = k["InterpMode"].as_str().unwrap_or("CIM_Linear");
                            CurvePoint { t: num(&k["InVal"], 0.0), v: c(&k["OutVal"]), arrive: c(&k["ArriveTangent"]), leave: c(&k["LeaveTangent"]), linear: mode == "CIM_Linear", constant: mode == "CIM_Constant" }
                        })
                        .collect();
                    Dist { op: 1, curve, ..Default::default() }
                }
                _ => Dist::constant(&vec![0.0; n]),
            }
        };
        let system = exports.iter().find(|e| e["Type"] == "ParticleSystem")?;
        let mut emitters = vec![];
        for er in system["Props"]["Emitters"].as_array()? {
            let Some(em) = get(er) else { continue };
            let Some(lod) = em["Props"]["LODLevels"].as_array().and_then(|l| l.first()).and_then(get) else { continue };
            let lp = &lod["Props"];
            if lp["bEnabled"].as_bool() == Some(false) {
                continue;
            }
            let Some(req) = get(&lp["RequiredModule"]) else { continue };
            let rq = &req["Props"];
            if rq["bEnabled"].as_bool() == Some(false) {
                continue;
            }
            let spawn = get(&lp["SpawnModule"]);
            let sp = spawn.map(|s| &s["Props"]);
            // The Spawn module's own bProcessSpawnRate isn't read: the flag gates the rate from the stacked spawn modules
            // (UParticleLODLevel's SpawningModules, from Modules: SpawnPerUnit). PLACEHOLDER, unverified in the binary;
            // the data agrees: the smoking effects (the hood's, the phone box's, the fuse box's) have it off, a rate and
            // no bursts.
            let spawn_enabled = sp.is_some_and(|s| s["bEnabled"].as_bool() != Some(false));
            let mut modules = vec![];
            let mut spawn_per_unit = None;
            for mr in lp["Modules"].as_array().into_iter().flatten() {
                let Some(m) = get(mr) else { continue };
                let p = &m["Props"];
                if p["bEnabled"].as_bool() == Some(false) {
                    continue;
                }
                let ty = m["Type"].as_str().unwrap_or("");
                modules.push(match ty {
                    "ParticleModuleLifetime" => Module::Lifetime(dist(&p["Lifetime"], 1)),
                    "ParticleModuleLocation" => Module::Location(dist(&p["StartLocation"], 3)),
                    "ParticleModuleLocationPrimitiveSphere" => Module::LocationSphere {
                        radius: dist(&p["StartRadius"], 1),
                        velocity: p["Velocity"].as_bool().unwrap_or(false),
                        velocity_scale: dist(&p["VelocityScale"], 1),
                        origin: dist(&p["StartLocation"], 3),
                    },
                    "ParticleModuleVelocity" => Module::Velocity { start: dist(&p["StartVelocity"], 3), radial: dist(&p["StartVelocityRadial"], 1), world: p["bInWorldSpace"].as_bool().unwrap_or(false) },
                    "ParticleModuleSize" => Module::Size(dist(&p["StartSize"], 3)),
                    "ParticleModuleSizeMultiplyLife" => Module::SizeMultiplyLife {
                        life: dist(&p["LifeMultiplier"], 3),
                        axes: [p["MultiplyX"].as_bool().unwrap_or(true), p["MultiplyY"].as_bool().unwrap_or(true), p["MultiplyZ"].as_bool().unwrap_or(true)],
                    },
                    "ParticleModuleSizeScaleBySpeed" => Module::SizeScaleBySpeed {
                        speed: [num(&p["SpeedScale"]["X"], 1.0), num(&p["SpeedScale"]["Y"], 1.0)],
                        max: [num(&p["MaxScale"]["X"], 1.0), num(&p["MaxScale"]["Y"], 1.0)],
                    },
                    "ParticleModuleColorOverLife" => Module::ColorOverLife { color: dist(&p["ColorOverLife"], 3), alpha: dist(&p["AlphaOverLife"], 1) },
                    "ParticleModuleRotation" => Module::Rotation(dist(&p["StartRotation"], 1)),
                    "ParticleModuleRotationRate" => Module::RotationRate(dist(&p["StartRotationRate"], 1)),
                    "ParticleModuleRotationRateMultiplyLife" => Module::RotationRateMultiplyLife(dist(&p["LifeMultiplier"], 1)),
                    "ParticleModuleAccelerationConstant" => Module::AccelerationConstant { accel: vec3(&p["Acceleration"]), world: p["bAlwaysInWorldSpace"].as_bool().unwrap_or(false) },
                    "ParticleModuleAccelerationDrag" => Module::Drag(dist(&p["DragCoefficientRaw"], 1)),
                    "ParticleModuleOrbit" => Module::Orbit {
                        offset: dist(&p["OffsetAmount"], 3),
                        rotation: dist(&p["RotationAmount"], 3),
                        rate: dist(&p["RotationRateAmount"], 3),
                        offset_life: p["OffsetOptions"]["bProcessDuringUpdate"].as_bool().unwrap_or(false),
                    },
                    "ParticleModuleSubUV" => Module::SubUV(dist(&p["SubImageIndex"], 1)),
                    "ParticleModuleParameterDynamic" => {
                        let mut d: [Option<(Dist, bool, bool)>; 4] = Default::default();
                        for (i, x) in p["DynamicParams"].as_array().into_iter().flatten().take(4).enumerate() {
                            d[i] = Some((dist(&x["ParamValue"], 1), x["bSpawnTimeOnly"].as_bool().unwrap_or(false), x["bUseEmitterTime"].as_bool().unwrap_or(false)));
                        }
                        Module::Dynamic(d)
                    }
                    "ParticleModuleLight" => {
                        let or = |k: &str, n: usize, d: f32| if p[k].is_null() { Dist::constant(&vec![d; n]) } else { dist(&p[k], n) };
                        Module::Light {
                            color: or("ColorScaleOverLife", 3, 1.0),
                            brightness: or("BrightnessOverLife", 1, 32.0),
                            radius: or("RadiusScale", 1, 15.0),
                            exponent: or("LightExponent", 1, 16.0),
                            inverse_squared: p["bUseInverseSquaredFalloff"].as_bool().unwrap_or(true),
                            spawn_fraction: num(&p["SpawnFraction"], 1.0),
                            high_quality: p["bHighQualityLights"].as_bool().unwrap_or(false),
                        }
                    }
                    "ParticleModulePivotOffset" => Module::PivotOffset([num(&p["PivotOffset"]["X"], 0.0), num(&p["PivotOffset"]["Y"], 0.0)]),
                    "ParticleModuleOrientationAxisLock" => Module::AxisLock(p["LockAxisFlags"].as_str().unwrap_or("EPAL_NONE").to_string()),
                    "ParticleModuleMeshRotation" => Module::MeshRotation(dist(&p["StartRotation"], 3)),
                    "ParticleModuleAttractorPoint" => {
                        let mut flags = 0x3f1u32;
                        for (bit, key) in [(0, "bStrengthByDistance"), (1, "bAffectBaseVelocity"), (2, "bOverrideVelocity"), (3, "bUseWorldSpacePosition"), (4, "Positive_X"), (5, "Positive_Y"), (6, "Positive_Z"), (7, "Negative_X"), (8, "Negative_Y"), (9, "Negative_Z")] {
                            if let Some(b) = p[key].as_bool() {
                                flags = if b { flags | 1 << bit } else { flags & !(1 << bit) };
                            }
                        }
                        Module::AttractorPoint { position: dist(&p["Position"], 3), range: dist(&p["Range"], 1), strength: dist(&p["Strength"], 1), flags }
                    }
                    "ParticleModuleAttractorParticle" => Module::AttractorParticle {
                        emitter: p["EmitterName"].as_str().unwrap_or("").to_string(),
                        range: dist(&p["Range"], 1),
                        strength: dist(&p["Strength"], 1),
                        by_distance: p["bStrengthByDistance"].as_bool().unwrap_or(true),
                        affect_base: p["bAffectBaseVelocity"].as_bool().unwrap_or(false),
                        random: p["SelectionMethod"].as_str().is_none_or(|m| m == "EAPSM_Random"),
                        renew: p["bRenewSource"].as_bool().unwrap_or(false),
                        inherit: p["bInheritSourceVel"].as_bool().unwrap_or(false),
                    },
                    "ParticleModuleLocationPrimitiveCylinder" => Module::LocationCylinder {
                        radius: dist(&p["StartRadius"], 1),
                        height: dist(&p["StartHeight"], 1),
                        origin: dist(&p["StartLocation"], 3),
                        axis: match p["HeightAxis"].as_str() {
                            Some("PMLPC_HEIGHTAXIS_X") => 0,
                            Some("PMLPC_HEIGHTAXIS_Y") => 1,
                            _ => 2,
                        },
                        surface_only: p["SurfaceOnly"].as_bool().unwrap_or(false),
                        velocity: p["Velocity"].as_bool().unwrap_or(false),
                        velocity_scale: dist(&p["VelocityScale"], 1),
                    },
                    "ParticleModuleSourceMovement" => Module::SourceMovement(dist(&p["SourceMovementScale"], 3)),
                    "ParticleModuleAttractorPointGravity" => Module::GpuOnly,
                    "ParticleModuleCollision" => Module::Collision {
                        damping: dist(&p["DampingFactor"], 3),
                        damping_rotation: dist(&p["DampingFactorRotation"], 3),
                        max_collisions: dist(&p["MaxCollisions"], 1),
                        completion: match p["CollisionCompletionOption"].as_str() {
                            Some("EPCC_Freeze") => 1,
                            Some("EPCC_HaltCollisions") => 2,
                            Some("EPCC_FreezeTranslation") => 3,
                            Some("EPCC_FreezeRotation") => 4,
                            Some("EPCC_FreezeMovement") => 5,
                            _ => 0,
                        },
                        delay: dist(&p["DelayAmount"], 1),
                        only_vertical: p["bOnlyVerticalNormalsDecrementCount"].as_bool().unwrap_or(false),
                        vertical_fudge: num(&p["VerticalFudgeFactor"], 0.1),
                        dir_scalar: num(&p["DirScalar"], 3.5),
                    },
                    "ParticleModuleMeshRotationRate" => Module::MeshRotationRate(dist(&p["StartRotationRate"], 3)),
                    "ParticleModuleSpawnPerUnit" => {
                        spawn_per_unit = Some((num(&p["UnitScalar"], 50.0), dist(&p["SpawnPerUnit"], 1)));
                        continue;
                    }
                    other => Module::Unported(other.to_string()),
                });
            }
            let bursts = sp
                .filter(|_| spawn_enabled || sp.is_some_and(|s| s["bEnabled"].as_bool() != Some(false)))
                .and_then(|s| s["BurstList"].as_array())
                .into_iter()
                .flatten()
                .map(|b| Burst { time: num(&b["Time"], 0.0), count: b["Count"].as_u64().unwrap_or(0) as u32, count_low: b["CountLow"].as_i64().unwrap_or(-1) as i32 })
                .collect();
            emitters.push(Emitter {
                name: em["Props"]["EmitterName"].as_str().unwrap_or("Particle Emitter").to_string(),
                material: rq["Material"]["ObjectName"].as_str().and_then(|s| s.split('\'').nth(1)).map(str::to_string),
                origin: vec3(&rq["EmitterOrigin"]),
                rotation: Vec3::new(num(&rq["EmitterRotation"]["Pitch"], 0.0), num(&rq["EmitterRotation"]["Yaw"], 0.0), num(&rq["EmitterRotation"]["Roll"], 0.0)),
                screen_alignment: rq["ScreenAlignment"].as_str().unwrap_or("PSA_Square").to_string(),
                local_space: rq["bUseLocalSpace"].as_bool().unwrap_or(false),
                duration: num(&rq["EmitterDuration"], 1.0),
                loops: rq["EmitterLoops"].as_u64().unwrap_or(0) as u32,
                delay: num(&rq["EmitterDelay"], 0.0),
                delay_first_loop_only: rq["bDelayFirstLoopOnly"].as_bool().unwrap_or(false),
                sub_images: [rq["SubImages_Horizontal"].as_u64().unwrap_or(1) as u32, rq["SubImages_Vertical"].as_u64().unwrap_or(1) as u32],
                interpolation: rq["InterpolationMethod"].as_str().unwrap_or("PSUVIM_None").to_string(),
                // bUseMaxDrawCount defaults on (MaxDrawCount 500).
                max_draw_count: if rq["bUseMaxDrawCount"].as_bool().unwrap_or(true) { Some(rq["MaxDrawCount"].as_u64().unwrap_or(500) as u32) } else { None },
                rate: if spawn_enabled { sp.map_or(Dist::constant(&[0.0]), |s| dist(&s["Rate"], 1)) } else { Dist::constant(&[0.0]) },
                rate_scale: sp.map_or(Dist::constant(&[1.0]), |s| dist(&s["RateScale"], 1)),
                burst_scale: sp.map_or(Dist::constant(&[1.0]), |s| dist(&s["BurstScale"], 1)),
                bursts,
                spawn_per_unit,
                gpu: get(&lp["TypeDataModule"]).is_some_and(|t| t["Type"] == "ParticleModuleTypeDataGpu"),
                mesh: get(&lp["TypeDataModule"]).filter(|t| t["Type"] == "ParticleModuleTypeDataMesh").and_then(|t| {
                    let tp = &t["Props"];
                    let path = tp["Mesh"]["ObjectPath"].as_str()?;
                    let path = path.trim_start_matches('/');
                    let path = path.rsplit_once('.').map_or(path, |(a, _)| a);
                    Some(MeshData {
                        file: format!("{path}.glb"),
                        roll_pitch_yaw: dist(&tp["RollPitchYawRange"], 3),
                        override_material: tp["bOverrideMaterial"].as_bool().unwrap_or(false),
                        camera_facing: tp["bCameraFacing"].as_bool().unwrap_or(false),
                        axis_lock: tp["AxisLockOption"].as_str().unwrap_or("EPAL_NONE").to_string(),
                        camera_facing_option: tp["CameraFacingOption"].as_str().unwrap_or("XAxisFacing_NoUp").to_string(),
                        spin: tp["bApplyParticleRotationAsSpin"].as_bool().unwrap_or(false),
                    })
                }),
                modules,
            });
        }
        Some(System { name: v["Name"].as_str().unwrap_or("").to_string(), warmup: num(&system["Props"]["WarmupTime"], 0.0), emitters })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn table_lookup() {
        // An AlphaOverLife-style table: 3 entries over 0..1.
        let d = Dist { op: 1, count: 3, stride: 1, time_scale: 2.0, values: vec![0.0, 1.0, 0.5], ..Default::default() };
        assert_eq!(d.eval(0.0, 0.0), 0.0);
        assert_eq!(d.eval(0.25, 0.0), 0.5);
        assert_eq!(d.eval(0.5, 0.0), 1.0);
        assert_eq!(d.eval(1.0, 0.0), 0.5);
        assert_eq!(d.eval(2.0, 0.0), 0.5);
        // A random vector (min, max): XYZ locked.
        let v = Dist { op: 2, count: 1, stride: 6, values: vec![40.0, 40.0, 40.0, 60.0, 60.0, 60.0], lock: 4, ..Default::default() };
        assert_eq!(v.eval3(0.0, [0.5, 0.0, 1.0]), Vec3::splat(50.0));
    }

    /// Every exported system loads, with emitters, when `F13_EXPORT` points at an export (skipped otherwise).
    #[test]
    fn exported_systems_load() {
        let Some(root) = std::env::var_os("F13_EXPORT").map(std::path::PathBuf::from) else { return };
        let Ok(dir) = std::fs::read_dir(root.join("particles")) else { return };
        for f in dir.flatten() {
            let s = System::load(&f.path()).unwrap_or_else(|| panic!("{:?} doesn't load", f.path()));
            assert!(!s.emitters.is_empty(), "{}: no emitters", s.name);
            // The campfire's base flames: 10..20 a second, living 0.75..1 s.
            if s.name == "P_CampFire_Fix" {
                let e = s.emitters.iter().find(|e| e.name == "FlamesBase1").unwrap();
                assert_eq!((e.rate.eval(0.0, 0.0), e.rate.eval(0.0, 1.0)), (10.0, 20.0));
                assert!(e.modules.iter().any(|m| matches!(m, Module::Lifetime(d) if d.eval(0.0, 0.0) == 0.75 && d.eval(0.0, 1.0) == 1.0)));
            }
        }
    }
}
