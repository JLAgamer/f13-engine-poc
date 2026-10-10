//! The counselor's hair: her Hair component's anim Blueprint (an SCHairAnimInstance, e.g. AthleteHair_AnimBP) is
//! MeshRefPose -> AnimDynamics (a chain of boxes from BoundBone to ChainEnd) at Alpha ScaledAlpha. Ported from the
//! binary (MODLOG "Hair dynamics"): FAnimNode_AnimDynamics (vtable 0x1424bccd0: InitPhysics 0x140f512c0, UpdateLimits
//! 0x140f5cc00, EvaluateSkeletalControl 0x140f3a460, PreUpdate 0x140f56a70) and FAnimPhys's solver
//! (PhysicsUpdate 0x141297070 and its body and limit routines). Everything runs in Unreal's units and axes (cm, Z up),
//! in World simulation space.

use bevy::prelude::*;
use serde_json::Value;

use crate::character::{CounselorCharacter, Rig};
use crate::counselor_camera::{to_bevy, to_unreal};

pub struct HairPlugin;

impl Plugin for HairPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, load_wind).add_systems(PostUpdate, hair_dynamics.after(bevy::transform::TransformSystems::Propagate));
    }
}

/// FAnimPhys's defaults (AnimPhysicsSolver): linear and angular damping (DAT_1420f5940), the wind's force scale
/// (DAT_14213890c), the angular joints' bias (DAT_142108c54).
const DEFAULT_DAMPING: f32 = 0.7;
const WIND_FORCE_SCALE: f32 = 250.0;
const JOINT_BIAS: f32 = 0.3;
/// |DefaultGravityZ| (DefaultEngine.ini PhysicsSettings).
const GRAVITY: f32 = 980.0;
/// MaxPhysicsDeltaTime (DefaultEngine.ini): one step a frame of at most this (p.AnimDynamicsAdaptiveSubstep is 0).
const MAX_PHYSICS_DT: f32 = 0.033333;
/// SCHairAnimInstance's constructor (0x1402f3e00): TargetFramerate and MaxAlpha.
const TARGET_FRAMERATE: f32 = 60.0;
const MAX_ALPHA: f32 = 0.8;

/// A rotation in Unreal's axes from Bevy's, and back (the export's swap of Y and Z: the same map both ways).
fn ue_quat(q: Quat) -> Quat {
    Quat::from_xyzw(-q.x, -q.z, -q.y, q.w)
}

/// The level's WindDirectionalSources (their component's Speed and forward, Unreal space).
#[derive(Resource, Default)]
struct Wind(Vec<(f32, Vec3)>);

fn load_wind(mut commands: Commands, level: Res<crate::Level>) {
    let mut sources = Vec::new();
    for c in level.layouts.iter().flat_map(|l| &l.actors).filter(|a| a.class == "WindDirectionalSource").flat_map(|a| &a.components) {
        if c.kind != "WindDirectionalSourceComponent" {
            continue;
        }
        // PLACEHOLDER: FScene::GetWindParameters_GameThread isn't traced: UE's directional source (bPointWind off)
        // gives its Speed (default 0.1) and forward, weight 1; several are averaged by weight.
        let speed = c.props.get("Speed").and_then(|v| v.as_f64()).unwrap_or(0.1) as f32;
        let r = c.world.as_ref().map_or(Quat::IDENTITY, |w| Quat::from_array(w.r).normalize());
        sources.push((speed, r * Vec3::X));
    }
    commands.insert_resource(Wind(sources));
}

/// The node's settings (FAnimNode_AnimDynamics, from the export).
struct Settings {
    bound: String,
    end: String,
    extents: Vec3,
    joint_offset: Vec3,
    gravity_scale: f32,
    linear_damping: Option<f32>,
    angular_damping: Option<f32>,
    angular_bias: Option<f32>,
    pre: u32,
    post: u32,
    /// ConstraintSetup: each linear axis's limits (None when Free), the angular limits (degrees).
    linear: [Option<(f32, f32)>; 3],
    angular_min: Vec3,
    angular_max: Vec3,
    wind: bool,
    wind_scale: f32,
    external_force: Vec3,
    alpha: f32,
    target_framerate: f32,
}

impl Settings {
    fn new(v: &Value) -> Option<Settings> {
        let n = &v["Node"];
        let vec = |o: &Value| Vec3::new(o["X"].as_f64().unwrap_or(0.0) as f32, o["Y"].as_f64().unwrap_or(0.0) as f32, o["Z"].as_f64().unwrap_or(0.0) as f32);
        let f = |k: &str, d: f32| n[k].as_f64().map_or(d, |x| x as f32);
        let b = |k: &str| n[k].as_bool() == Some(true);
        let c = &n["ConstraintSetup"];
        if n["SimulationSpace"].as_str() != Some("AnimPhysSimSpaceType::World") {
            warn!("hair: AnimDynamics in {:?} isn't ported (only World)", n["SimulationSpace"]);
            return None;
        }
        if c["AngularConstraintType"].as_str() != Some("AnimPhysAngularConstraintType::Angular") || c["TwistAxis"].as_str() != Some("AnimPhysTwistAxis::AxisX") {
            warn!("hair: only the Angular constraint about AxisX is ported");
            return None;
        }
        if n["PlanarLimits"].as_array().is_some_and(|a| !a.is_empty()) || b("bUseSphericalLimits") || b("bLinearSpring") || b("bAngularSpring") {
            warn!("hair: planar / spherical limits and springs aren't ported");
        }
        let (min, max) = (vec(&c["LinearAxesMin"]), vec(&c["LinearAxesMax"]));
        let axis = |k: &str, i: usize| (c[k].as_str() != Some("AnimPhysLinearConstraintType::Free")).then_some((min[i], max[i]));
        let mut extents = vec(&n["BoxExtents"]);
        if extents.length_squared() <= 1e-8 {
            extents = Vec3::ONE;
        }
        Some(Settings {
            bound: n["BoundBone"]["BoneName"].as_str()?.to_string(),
            end: n["ChainEnd"]["BoneName"].as_str()?.to_string(),
            extents,
            joint_offset: vec(&n["LocalJointOffset"]),
            gravity_scale: f("GravityScale", 1.0),
            linear_damping: b("bOverrideLinearDamping").then(|| f("LinearDampingOverride", 0.0)),
            angular_damping: b("bOverrideAngularDamping").then(|| f("AngularDampingOverride", 0.0)),
            angular_bias: b("bOverrideAngularBias").then(|| f("AngularBiasOverride", 0.0)),
            pre: n["NumSolverIterationsPreUpdate"].as_u64().unwrap_or(4) as u32,
            post: n["NumSolverIterationsPostUpdate"].as_u64().unwrap_or(1) as u32,
            linear: [axis("LinearXLimitType", 0), axis("LinearYLimitType", 1), axis("LinearZLimitType", 2)],
            angular_min: vec(&c["AngularLimitsMin"]),
            angular_max: vec(&c["AngularLimitsMax"]),
            wind: b("bEnableWind"),
            wind_scale: f("WindScale", 1.0),
            external_force: vec(&n["ExternalForce"]),
            alpha: v["MaxAlpha"].as_f64().map_or(MAX_ALPHA, |x| x as f32),
            target_framerate: v["TargetFramerate"].as_f64().map_or(TARGET_FRAMERATE, |x| x as f32),
        })
    }

    /// The node's bLinearFullyLocked (InitPhysics): every linear axis limited to a single value.
    fn fully_locked(&self) -> bool {
        self.linear.iter().all(|l| l.is_some_and(|(a, b)| a - b == 0.0))
    }
}

/// FAnimPhysRigidBody (the parts the node uses).
#[derive(Clone, Copy)]
struct Body {
    pos: Vec3,
    rot: Quat,
    linear_momentum: Vec3,
    angular_momentum: Vec3,
    inverse_mass: f32,
    /// InverseTensorWithoutMass (body space) and InverseWorldSpaceTensor.
    inverse_tensor: Mat3,
    world_inverse_tensor: Mat3,
    next_pos: Vec3,
    next_rot: Quat,
    gravity_scale: f32,
    linear_damping: Option<f32>,
    angular_damping: Option<f32>,
    /// WindData: enabled, direction, speed, BodyWindScale, WindAdaption.
    wind: Option<(Vec3, f32, f32, f32)>,
}

impl Body {
    /// FAnimPhysRigidBody's constructor (0x1412835e0) from one box shape (FAnimPhysShape::MakeBox 0x141295180: its
    /// corners at +-extents / 2): Mass 1, at `pos`, identity orientation; its inertia FAnimPhys::CalculateInertia
    /// (0x141288200: the shapes' polyhedral inertia, unit mass, volume-weighted; a centred box's is
    /// diag((b^2 + c^2) / 12, ..) over its full extents), inverted.
    fn new(pos: Vec3, extents: Vec3) -> Body {
        let (a, b, c) = (extents.x * extents.x, extents.y * extents.y, extents.z * extents.z);
        let inverse_tensor = Mat3::from_diagonal(Vec3::new(12.0 / (b + c), 12.0 / (a + c), 12.0 / (a + b)));
        Body {
            pos,
            rot: Quat::IDENTITY,
            linear_momentum: Vec3::ZERO,
            angular_momentum: Vec3::ZERO,
            inverse_mass: 1.0,
            inverse_tensor,
            world_inverse_tensor: inverse_tensor,
            next_pos: pos,
            next_rot: Quat::IDENTITY,
            gravity_scale: 1.0,
            linear_damping: None,
            angular_damping: None,
            wind: None,
        }
    }

    /// Body space to world.
    fn transform(&self, local: Vec3) -> Vec3 {
        self.pos + self.rot * local
    }

    /// The world inverse inertia tensor at `rot`, mass included. PLACEHOLDER: the binary's matrix order (M T M^T
    /// with M from 0x141283e90) isn't checked against Bevy's; the shipped hair bodies are cubes, where it doesn't
    /// matter.
    fn world_tensor(&self, rot: Quat) -> Mat3 {
        let r = Mat3::from_quat(rot);
        r * (self.inverse_tensor * self.inverse_mass) * r.transpose()
    }

    fn angular_velocity(&self) -> Vec3 {
        self.world_inverse_tensor * self.angular_momentum
    }

    /// FAnimPhys::InitializeBodyVelocity (0x1412938d0): damping, gravity and wind into the momentum, the world tensor.
    fn initialize_velocity(&mut self, dt: f32, gravity: Vec3) {
        let ld = self.linear_damping.unwrap_or(DEFAULT_DAMPING);
        let ad = self.angular_damping.unwrap_or(DEFAULT_DAMPING);
        self.linear_momentum *= (1.0 - ld).powf(dt);
        self.angular_momentum *= (1.0 - ad).powf(dt);
        let mut force = gravity * GRAVITY * (1.0 / self.inverse_mass) * self.gravity_scale;
        if let Some((dir, speed, scale, adaption)) = self.wind {
            let wind = dir * speed * WIND_FORCE_SCALE * scale;
            if wind.length_squared() > 1e-8 {
                force += wind * adaption;
            }
        }
        self.linear_momentum += force * dt;
        self.world_inverse_tensor = self.world_tensor(self.rot);
    }

    /// The orientation's derivative (0x14128b8c0): 0.5 (w, 0) q, w the angular velocity at q.
    fn spin(&self, q: Quat) -> Quat {
        let q = q.normalize();
        let w = self.world_tensor(q) * self.angular_momentum;
        Quat::from_xyzw(w.x, w.y, w.z, 0.0) * q * 0.5
    }

    /// FAnimPhys::CalculateNextPose (0x141288ad0): the position along the momentum, the orientation by RK4
    /// (0x14129b5a0), normalised.
    fn calculate_next_pose(&mut self, dt: f32) {
        self.next_pos = self.pos + self.linear_momentum * self.inverse_mass * dt;
        let q0 = self.rot;
        let add = |a: Quat, b: Quat, s: f32| Quat::from_vec4(Vec4::from(a) + Vec4::from(b) * s);
        let k1 = self.spin(q0);
        let k2 = self.spin(add(q0, k1, dt * 0.5));
        let k3 = self.spin(add(q0, k2, dt * 0.5));
        let k4 = self.spin(add(q0, k3, dt));
        let q = Vec4::from(q0) + (Vec4::from(k1) + Vec4::from(k4)) * (dt / 6.0) + (Vec4::from(k2) + Vec4::from(k3)) * (dt / 3.0);
        self.next_rot = if q.length_squared() >= 1e-8 { Quat::from_vec4(q).normalize() } else { Quat::IDENTITY };
    }

    /// FAnimPhys::UpdatePose (0x14129b930).
    fn update_pose(&mut self) {
        self.pos = self.next_pos;
        self.rot = self.next_rot;
        self.world_inverse_tensor = self.world_tensor(self.rot);
    }
}

/// FAnimPhysLinearLimit.
struct LinearLimit {
    b0: Option<usize>,
    b1: Option<usize>,
    normal: Vec3,
    target: f32,
    target_without_bias: f32,
    min: f32,
    max: f32,
    sum: f32,
    inverse_inertia_impulse: f32,
    r0: Vec3,
    r1: Vec3,
}

impl LinearLimit {
    /// Its constructor (0x14129b010): the lever arms in world space, the impulse's inverse inertia.
    #[allow(clippy::too_many_arguments)]
    fn new(bodies: &[Body], b0: Option<usize>, p0: Vec3, b1: Option<usize>, p1: Vec3, normal: Vec3, target: f32, target_without_bias: f32, min: f32, max: f32) -> LinearLimit {
        let arm = |b: Option<usize>, p: Vec3| b.map_or(p, |b| bodies[b].rot * p);
        let (r0, r1) = (arm(b0, p0), arm(b1, p1));
        let term = |b: Option<usize>, r: Vec3| {
            b.map_or(0.0, |b| bodies[b].inverse_mass + normal.dot((bodies[b].world_inverse_tensor * r.cross(normal)).cross(r)))
        };
        LinearLimit { b0, b1, normal, target, target_without_bias, min, max, sum: 0.0, inverse_inertia_impulse: 1.0 / (term(b0, r0) + term(b1, r1)), r0, r1 }
    }

    /// FAnimPhysLinearLimit::Iter (0x141294d00).
    fn iter(&mut self, bodies: &mut [Body], dt: f32) {
        let v = |b: Option<usize>, r: Vec3| b.map_or(Vec3::ZERO, |b| bodies[b].linear_momentum * bodies[b].inverse_mass + bodies[b].angular_velocity().cross(r));
        let (v0, v1) = (v(self.b0, self.r0), v(self.b1, self.r1));
        let lambda = ((-self.target - (v1 - v0).dot(self.normal)) * self.inverse_inertia_impulse).min(dt * self.max - self.sum).max(dt * self.min - self.sum);
        if let Some(b) = self.b0 {
            let impulse = -lambda * self.normal;
            bodies[b].linear_momentum += impulse;
            bodies[b].angular_momentum += self.r0.cross(impulse);
        }
        if let Some(b) = self.b1 {
            let impulse = lambda * self.normal;
            bodies[b].linear_momentum += impulse;
            bodies[b].angular_momentum += self.r1.cross(impulse);
        }
        self.sum += lambda;
    }
}

/// FAnimPhysAngularLimit.
struct AngularLimit {
    b0: Option<usize>,
    b1: Option<usize>,
    axis: Vec3,
    sum: f32,
    target: f32,
    min: f32,
    max: f32,
    inverse_inertia_impulse: f32,
}

impl AngularLimit {
    /// Its constructor (0x14129ae80).
    fn new(bodies: &[Body], b0: Option<usize>, b1: Option<usize>, axis: Vec3, target: f32, min: f32, max: f32) -> AngularLimit {
        let term = |b: Option<usize>| b.map_or(0.0, |b| axis.dot(bodies[b].world_inverse_tensor * axis));
        AngularLimit { b0, b1, axis, sum: 0.0, target, min, max, inverse_inertia_impulse: 1.0 / (term(b0) + term(b1)) }
    }

    /// FAnimPhysAngularLimit::Iter (0x141294aa0): skipped while its target is -FLT_MAX.
    fn iter(&mut self, bodies: &mut [Body], dt: f32) {
        if self.target == -f32::MAX {
            return;
        }
        let w = |b: Option<usize>| b.map_or(0.0, |b| self.axis.dot(bodies[b].angular_velocity()));
        let lambda = ((self.target - (w(self.b1) - w(self.b0))) * self.inverse_inertia_impulse).min(dt * self.max - self.sum).max(dt * self.min - self.sum);
        if let Some(b) = self.b0 {
            bodies[b].angular_momentum -= lambda * self.axis;
        }
        if let Some(b) = self.b1 {
            bodies[b].angular_momentum += lambda * self.axis;
        }
        self.sum += lambda;
    }
}

/// FQuat::FindBetween (0x14056f960).
fn find_between(a: Vec3, b: Vec3) -> Quat {
    let norm = (a.length_squared() * b.length_squared()).sqrt();
    let w = a.dot(b) + norm;
    let q = if w < norm * 1e-6 {
        if a.x.abs() > a.y.abs() { Quat::from_xyzw(-a.z, 0.0, a.x, 0.0) } else { Quat::from_xyzw(0.0, -a.z, a.y, 0.0) }
    } else {
        let c = a.cross(b);
        Quat::from_xyzw(c.x, c.y, c.z, w)
    };
    q.normalize()
}

/// FAnimPhys::ConstrainAngularRange (0x1412890f0 -> 0x1412891f0), twist about X: the swing (FindBetween X and the
/// relative rotation's X) and the twist left; per swing axis (Y, then Z) one-sided limits at 2 sin(limit / 2) when
/// the range is open (under 2 pi), or a two-sided one when min == max (on Y at 2 sin(min / 2) without bias, on Z at
/// (min - swing) x 2 bias, as the binary has them); the twist held at 0 with the bias (its limits unused).
#[allow(clippy::too_many_arguments)]
fn constrain_angular_range(limits: &mut Vec<AngularLimit>, bodies: &[Body], dt: f32, b0: Option<usize>, b1: usize, frame: Quat, min_deg: Vec3, max_deg: Vec3, bias: f32) {
    let (min, max) = (min_deg * std::f32::consts::PI / 180.0, max_deg * std::f32::consts::PI / 180.0);
    let frame0 = b0.map_or(frame, |b| bodies[b].rot * frame);
    let frame1 = bodies[b1].rot;
    let q = frame0.conjugate() * frame1;
    let mut swing = find_between(Vec3::X, q * Vec3::X);
    let mut twist = swing.conjugate() * q;
    if swing.w < 0.0 {
        swing = -swing;
    }
    if twist.w < 0.0 {
        twist = -twist;
    }
    let mut add = |axis: Vec3, target: f32, lo: f32, hi: f32| limits.push(AngularLimit::new(bodies, b0, Some(b1), axis, target, lo, hi));
    for (i, axis, s) in [(1, frame1 * Vec3::Y, swing.y), (2, frame1 * Vec3::Z, swing.z)] {
        let (lo, hi) = (min[i], max[i]);
        if lo == hi {
            let target = if i == 1 { (2.0 * (lo * 0.5).sin() - 2.0 * s) / dt } else { (lo - s) * (bias + bias) / dt };
            add(axis, target, -f32::MAX, f32::MAX);
        } else if hi - lo < std::f32::consts::TAU {
            add(axis, (2.0 * (lo * 0.5).sin() - 2.0 * s) / dt, 0.0, f32::MAX);
            add(-axis, 2.0 * (s - (hi * 0.5).sin()) / dt, 0.0, f32::MAX);
        }
    }
    add(frame1 * Vec3::X, bias * -2.0 * twist.x / dt, -f32::MAX, f32::MAX);
}

/// The simulated chain on the hair part.
#[derive(Component)]
struct HairSim {
    /// The chain's bones (BoundBone first), the bone above it, their reference locals (the graph's MeshRefPose), and
    /// the bones under the chain that aren't in it (parents first).
    chain: Vec<Entity>,
    parent: Entity,
    reference: Vec<Transform>,
    below: Vec<Entity>,
    bodies: Vec<Body>,
    /// Each body's JointOffsets entry (body space).
    joint_offsets: Vec<Vec3>,
    rng: u64,
}

/// The chain's reference pose in Unreal world space this frame: (position, rotation) per bone.
fn input_pose(sim: &HairSim, parent: &GlobalTransform) -> (Vec<(Vec3, Quat)>, Vec<GlobalTransform>) {
    let mut world = Vec::new();
    let mut at = *parent;
    for local in &sim.reference {
        at = at.mul_transform(*local);
        world.push(at);
    }
    let ue = world.iter().map(|g| (to_unreal(g.translation()), ue_quat(g.rotation()))).collect();
    (ue, world)
}

#[allow(clippy::too_many_arguments, clippy::type_complexity)]
fn hair_dynamics(
    mut commands: Commands,
    time: Res<Time>,
    counselor: Option<Res<CounselorCharacter>>,
    wind: Option<Res<Wind>>,
    rigs: Query<&Rig>,
    mut sims: Query<&mut HairSim>,
    children: Query<&Children>,
    names: Query<&Name>,
    parents: Query<&ChildOf>,
    mut transforms: Query<(&mut Transform, &mut GlobalTransform)>,
    mut settings: Local<Option<Option<Settings>>>,
) {
    let Some(model) = counselor.as_deref().map(|c| &c.0) else { return };
    let Some(settings) = settings.get_or_insert_with(|| model.info.hair_dynamics.as_ref().and_then(Settings::new)).as_ref() else { return };
    let Some(hair_index) = model.info.parts.iter().position(|p| p.name == "Hair") else { return };
    let frame_dt = time.delta_secs();
    for rig in &rigs {
        let Some(part) = rig.part(hair_index) else { continue };
        let Ok(mut sim) = sims.get_mut(part) else {
            // Once its scene is in: the chain from ChainEnd up to BoundBone.
            let named = |n: &str| children.iter_descendants(part).find(|e| names.get(*e).is_ok_and(|x| x.as_str() == n));
            let (Some(bound), Some(end)) = (named(&settings.bound), named(&settings.end)) else { continue };
            let mut chain = vec![end];
            while *chain.last().unwrap() != bound {
                let Ok(p) = parents.get(*chain.last().unwrap()) else { break };
                chain.push(p.parent());
            }
            if *chain.last().unwrap() != bound {
                warn!("hair: {} isn't under {}", settings.end, settings.bound);
                continue;
            }
            chain.reverse();
            let Ok(parent) = parents.get(bound).map(|p| p.parent()) else { continue };
            let reference = chain.iter().filter_map(|e| transforms.get(*e).ok().map(|(t, _)| *t)).collect();
            let below = children.iter_descendants(bound).filter(|e| !chain.contains(e)).collect();
            commands.entity(part).insert(HairSim { chain, parent, reference, below, bodies: Vec::new(), joint_offsets: Vec::new(), rng: 0x2545_f491_4f6c_dd1d });
            continue;
        };
        let Ok((_, parent_global)) = transforms.get(sim.parent).map(|(t, g)| (*t, *g)) else { continue };
        let (input, input_world) = input_pose(&sim, &parent_global);
        let n = input.len();

        // InitPhysics (0x140f512c0): a body per bone, at the bone less LocalJointOffset (its orientation identity);
        // JointOffsets: the first LocalJointOffset, then LocalJointOffset's direction times half the distance from the
        // bone before (or, with a zero offset, the bone before's body centre less this bone).
        if sim.bodies.len() != n {
            let lj = settings.joint_offset;
            let mut bodies = Vec::new();
            let mut offsets = Vec::new();
            for (i, &(pos, rot)) in input.iter().enumerate() {
                let offset = if i == 0 {
                    lj
                } else {
                    let (prev_pos, prev_rot) = input[i - 1];
                    if lj.length_squared() >= 1e-8 {
                        lj.normalize() * (prev_pos - pos).length() * 0.5
                    } else {
                        prev_pos + prev_rot * -lj - pos
                    }
                };
                let mut body = Body::new(pos + rot * -lj, settings.extents);
                body.gravity_scale = settings.gravity_scale;
                body.linear_damping = settings.linear_damping;
                body.angular_damping = settings.angular_damping;
                bodies.push(body);
                offsets.push(offset);
            }
            sim.bodies = bodies;
            sim.joint_offsets = offsets;
        }

        // PreUpdate (0x140f56a70; p.AnimDynamicsWind on): each body's wind from the scene, its direction taken into
        // the hair component's space (as the binary does, whatever the simulation space), WindAdaption
        // FRandRange(0, 2) every update, BodyWindScale WindScale.
        let sources = wind.as_deref().map_or(&[][..], |w| &w.0[..]);
        if settings.wind && !sources.is_empty() {
            let count = sources.len() as f32;
            let speed = sources.iter().map(|s| s.0).sum::<f32>() / count;
            let dir = sources.iter().map(|s| s.1).sum::<Vec3>() / count;
            let Ok((_, part_global)) = transforms.get(part).map(|(t, g)| (*t, *g)) else { continue };
            let (scale, rot, _) = part_global.to_scale_rotation_translation();
            let scale = Vec3::new(scale.x, scale.z, scale.y).abs().max(Vec3::splat(1e-4));
            let component_dir = ue_quat(rot).conjugate() * (dir / scale);
            let wind_scale = settings.wind_scale;
            let HairSim { bodies, rng, .. } = &mut *sim;
            for body in bodies.iter_mut() {
                *rng ^= *rng << 13;
                *rng ^= *rng >> 7;
                *rng ^= *rng << 17;
                let adaption = (*rng % 32768) as f32 / 32767.0 * 2.0;
                body.wind = Some((component_dir, speed, wind_scale, adaption));
            }
        }

        // EvaluateSkeletalControl (0x140f3a460): one step of min(dt, MaxPhysicsDeltaTime).
        let dt = frame_dt.min(MAX_PHYSICS_DT);
        if dt > 0.0 {
            let bias = settings.angular_bias.unwrap_or(JOINT_BIAS);
            let HairSim { bodies, joint_offsets, .. } = &mut *sim;
            // UpdateLimits (0x140f5cc00): per body, against the body before (its own space: the anchors -JointOffset
            // and JointOffset) or, for the first, the bone (its sim transform; the anchors the bone and
            // LocalJointOffset).
            let mut linear = Vec::new();
            let mut angular = Vec::new();
            for i in 0..n {
                let prev = i.checked_sub(1);
                let (shape_pos, shape_rot, anchor) = match prev {
                    Some(_) => (-joint_offsets[i], Quat::IDENTITY, joint_offsets[i]),
                    None => (input[0].0, input[0].1, settings.joint_offset),
                };
                let world = |b: Option<usize>, p: Vec3| b.map_or(p, |b| bodies[b].transform(p));
                if settings.fully_locked() {
                    // 0x14128ae40: three two-way limits along X, Y and Z, biased toward closing the gap.
                    let d = world(Some(i), anchor) - world(prev, shape_pos);
                    for axis in [Vec3::X, Vec3::Y, Vec3::Z] {
                        linear.push(LinearLimit::new(bodies, prev, shape_pos, Some(i), anchor, axis, d.dot(axis) / dt, 0.0, -f32::MAX, f32::MAX));
                    }
                } else {
                    // ConstrainAlongDirection (0x141288d90) along the shape's axes.
                    for (k, limits) in settings.linear.iter().enumerate() {
                        let Some((lo, hi)) = *limits else { continue };
                        let axis = shape_rot * [Vec3::X, Vec3::Y, Vec3::Z][k];
                        let d = (world(Some(i), anchor) - world(prev, shape_pos)).dot(axis);
                        if (lo - hi).abs() >= 1e-8 {
                            linear.push(LinearLimit::new(bodies, prev, shape_pos, Some(i), anchor, axis, (d - lo) / dt, (d - lo) / dt, 0.0, f32::MAX));
                            linear.push(LinearLimit::new(bodies, prev, shape_pos, Some(i), anchor, axis, (d - hi) / dt, (d - hi) / dt, -f32::MAX, 0.0));
                        } else {
                            linear.push(LinearLimit::new(bodies, prev, shape_pos, Some(i), anchor, axis, d / dt, d / dt, -f32::MAX, f32::MAX));
                        }
                    }
                }
                constrain_angular_range(&mut angular, bodies, dt, prev, i, shape_rot, settings.angular_min, settings.angular_max, bias);
            }
            // FAnimPhys::PhysicsUpdate (0x141297070), gravity (0, 0, -1) in sim space.
            for body in bodies.iter_mut() {
                body.initialize_velocity(dt, Vec3::NEG_Z);
                if settings.external_force.length_squared() > 1e-8 {
                    body.linear_momentum += settings.external_force * dt;
                }
            }
            for _ in 0..settings.pre {
                linear.iter_mut().for_each(|l| l.iter(bodies, dt));
                angular.iter_mut().for_each(|l| l.iter(bodies, dt));
            }
            bodies.iter_mut().for_each(|b| b.calculate_next_pose(dt));
            linear.iter_mut().for_each(|l| l.target = l.target_without_bias);
            angular.iter_mut().for_each(|l| l.target = if l.min >= 0.0 { l.target.min(0.0) } else { 0.0 });
            for _ in 0..settings.post {
                linear.iter_mut().for_each(|l| l.iter(bodies, dt));
                angular.iter_mut().for_each(|l| l.iter(bodies, dt));
            }
            bodies.iter_mut().for_each(|b| b.update_pose());
        }

        // The bones: each body's pose with its JointOffsets entry, blended with the reference pose by ScaledAlpha
        // (SCHairAnimInstance 0x1403142c0: min((1 / TargetFramerate) / max(dt, 1e-4), 1) x MaxAlpha).
        let alpha = ((1.0 / settings.target_framerate.max(1.0)) / frame_dt.max(1e-4)).min(1.0) * settings.alpha;
        let mut parent_world = parent_global;
        for i in 0..n {
            let body = sim.bodies[i];
            let out_pos = to_bevy(body.transform(sim.joint_offsets[i]));
            let out_rot = ue_quat(body.rot);
            let input = input_world[i];
            let pos = input.translation().lerp(out_pos, alpha);
            let rot = {
                let a = input.rotation();
                let b = if a.dot(out_rot) < 0.0 { -out_rot } else { out_rot };
                Quat::from_vec4(Vec4::from(a) * (1.0 - alpha) + Vec4::from(b) * alpha).normalize()
            };
            let world = GlobalTransform::from(Transform { translation: pos, rotation: rot, scale: input.scale() });
            let local = world.reparented_to(&parent_world);
            if let Ok((mut t, mut g)) = transforms.get_mut(sim.chain[i]) {
                *t = local;
                *g = world;
            }
            parent_world = world;
        }
        // The bones below the chain follow.
        for &e in &sim.below {
            let Some(p) = parents.get(e).ok().map(|p| p.parent()) else { continue };
            let Ok(pg) = transforms.get(p).map(|(_, g)| *g) else { continue };
            if let Ok((t, mut g)) = transforms.get_mut(e) {
                *g = pg.mul_transform(*t);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn find_between_turns_a_onto_b() {
        let q = find_between(Vec3::X, Vec3::Y);
        assert!((q * Vec3::X - Vec3::Y).length() < 1e-5);
        let q = find_between(Vec3::X, -Vec3::X);
        assert!((q * Vec3::X + Vec3::X).length() < 1e-5);
    }

    #[test]
    fn a_locked_body_stays_at_its_anchor() {
        // One 10 cm box hanging off a fixed point by its top (LocalJointOffset (0, 0, 1)), under gravity: the
        // lock-all limits hold the joint where it is.
        let mut bodies = vec![Body::new(Vec3::new(0.0, 0.0, -1.0), Vec3::splat(10.0))];
        let anchor = Vec3::ZERO;
        let dt = 1.0 / 60.0;
        for _ in 0..120 {
            let d = bodies[0].transform(Vec3::Z) - anchor;
            let mut linear: Vec<LinearLimit> = [Vec3::X, Vec3::Y, Vec3::Z].into_iter().map(|a| LinearLimit::new(&bodies, None, anchor, Some(0), Vec3::Z, a, d.dot(a) / dt, 0.0, -f32::MAX, f32::MAX)).collect();
            bodies[0].initialize_velocity(dt, Vec3::NEG_Z);
            for _ in 0..4 {
                linear.iter_mut().for_each(|l| l.iter(&mut bodies, dt));
            }
            bodies[0].calculate_next_pose(dt);
            linear.iter_mut().for_each(|l| l.target = l.target_without_bias);
            linear.iter_mut().for_each(|l| l.iter(&mut bodies, dt));
            bodies[0].update_pose();
        }
        assert!((bodies[0].transform(Vec3::Z) - anchor).length() < 0.5, "{:?}", bodies[0].transform(Vec3::Z));
    }
}
