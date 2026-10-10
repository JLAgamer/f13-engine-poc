//! The counselor's face (F13-decomp MODLOG "The counselor's face"): her face part's anim instance, SCFaceAnimInstance
//! (Face_AnimBP_<counselor>; the export's "Face.*" values, names and EmotionCurve), as its NativeUpdateAnimation
//! (0x140313880) and eye/blink update (FUN_1403241a0) set its morph targets:
//! - dead: Blink_L and Blink_R 1;
//! - outside a body montage, the fear faces: FearAlpha = her fear x 0.01; the EmotionCurve at the world time modulo its
//!   length; Shock = min(FearAlpha, curve.x) above ShockFaceMin, else FInterpTo(Shock, 0, dt,
//!   FearToNeutralInterpSpeed); Fear the same with FearFaceMin and curve.y;
//! - alive: blinks. The blink timer counts down and, at 0, flags a blink and restarts at rand(MinBlinkUpdateFrequency,
//!   MaxBlinkUpdateFrequency) (the first from NativeInitializeAnimation, 0x140311890). Once the eyelid weight is within
//!   0.1 of its target, the target becomes MaxEyelidWeight for a flagged blink still open, else MinEyelidWeight; the
//!   weight moves dt x EyelidMovementSpeed x (target - weight), clamped to Min..MaxEyelidWeight, onto EyelidMorphTarget
//!   and EyelashMorphTarget. (The constructor starts it at 0 with its target 1: a blink as she appears.)
//!
//! - alive: the eyes' look. Each time the left eye has reached its target (every component within 0.01 degrees,
//!   wrapped), the look (unless held): a look-away flag with its timer (rand(MinLook, MaxLookUpdateFrequency)) and hold
//!   (rand(0, MaxLookTime)) alternates a random direction (yaw +/- MaxEyeLookYawOffset, pitch +/-
//!   MaxEyeLookPitchOffset) with straight ahead, x LookDistance; the target is the eyes' midpoint + that offset turned
//!   by the left eye's parent bone; each eye's target rotator = the parent's world rotator minus the look's, with its
//!   pitch moved to yaw and its yaw to roll (pitch 0); both move by RInterpTo at EyeMovementSpeed; the anim graph's
//!   ModifyBone replaces the eye bones' component-space rotation with them (alpha EyeMovementOverride:
//!   !bInContextKill; 1 here). Computed in Unreal space (to_unreal; the export's quaternion map (-x, -z, -y, w)).
//!
//! Not ported yet: the face skeleton's base pose, brow and jaw additives, the body montages' face curves
//! (MontageFaceNames, the jaw and brow alphas, the blink override), bAnimationOverride's look at target_jnt.

use bevy::mesh::morph::MorphWeights;
use bevy::prelude::*;
use f13_core::curve::RichCurve;
use f13_core::fear::finterp_to;

use crate::character::{CounselorCharacter, Rig};
use crate::player::Player;

pub struct FacePlugin;

impl Plugin for FacePlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, face);
    }
}

/// A face anim instance's state.
#[derive(Component)]
struct FaceAnim {
    blink_timer: f32,
    blink: bool,
    eyelid: f32,
    eyelid_target: f32,
    fear: f32,
    shock: f32,
    look: Look,
}

/// The eyes' look state (SCFaceAnimInstance +0x4c8..0x554).
#[derive(Default)]
struct Look {
    offset: Vec3,
    ahead: bool,
    trigger: bool,
    timer: f32,
    hold: f32,
    left: Vec3,
    right: Vec3,
    left_target: Vec3,
    right_target: Vec3,
}

/// The values the face reads, from the counselor's export.
struct FaceTuning {
    fear_min: f32,
    shock_min: f32,
    to_neutral: f32,
    eyelid_min: f32,
    eyelid_max: f32,
    eyelid_speed: f32,
    blink_min: f32,
    blink_max: f32,
    eyelid_morph: String,
    eyelash_morph: String,
    fear_morph: String,
    shock_morph: String,
    blink_left: String,
    blink_right: String,
    emotion: Option<[RichCurve; 2]>,
    look_distance: f32,
    max_yaw: f32,
    max_pitch: f32,
    eye_speed: f32,
    look_min: f32,
    look_max: f32,
    max_look_time: f32,
    left_eye: String,
    right_eye: String,
}

impl FaceTuning {
    fn new(info: &f13_data::character::CharacterInfo) -> Self {
        let v = |k: &str, d: f32| info.values.get(&format!("Face.{k}")).copied().unwrap_or(d);
        let n = |k: &str, d: &str| info.names.get(&format!("Face.{k}")).cloned().unwrap_or_else(|| d.to_string());
        let curve = |k: &str| info.curve(&format!("Face.EmotionCurve.{k}"));
        FaceTuning {
            fear_min: v("FearFaceMin", 0.75),
            shock_min: v("ShockFaceMin", 0.35),
            to_neutral: v("FearToNeutralInterpSpeed", 5.0),
            eyelid_min: v("MinEyelidWeight", 0.0),
            eyelid_max: v("MaxEyelidWeight", 1.0),
            eyelid_speed: v("EyelidMovementSpeed", 1.0),
            blink_min: v("MinBlinkUpdateFrequency", 5.0),
            blink_max: v("MaxBlinkUpdateFrequency", 10.0),
            eyelid_morph: n("EyelidMorphTarget", "None"),
            eyelash_morph: n("EyelashMorphTarget", "None"),
            fear_morph: n("FearMorphTargetName", "Fear"),
            shock_morph: n("ShockMorphTargetName", "Shock"),
            blink_left: n("BlinkLeftName", "Blink_L"),
            blink_right: n("BlinkRightName", "Blink_R"),
            emotion: curve("X").zip(curve("Y")).map(|(x, y)| [x, y]),
            look_distance: v("LookDistance", 2500.0),
            max_yaw: v("MaxEyeLookYawOffset", 30.0),
            max_pitch: v("MaxEyeLookPitchOffset", 25.0),
            eye_speed: v("EyeMovementSpeed", 1.0),
            look_min: v("MinLookUpdateFrequency", 5.0),
            look_max: v("MaxLookUpdateFrequency", 10.0),
            max_look_time: v("MaxLookTime", 1.5),
            left_eye: n("LeftEyeBoneName", "L_Eye"),
            right_eye: n("RightEyeBoneName", "R_Eye"),
        }
    }
}

/// FRotator::NormalizeAxis: (-180, 180].
fn normalize_axis(a: f32) -> f32 {
    let a = a.rem_euclid(360.0);
    if a > 180.0 { a - 360.0 } else { a }
}

fn normalize(r: Vec3) -> Vec3 {
    Vec3::new(normalize_axis(r.x), normalize_axis(r.y), normalize_axis(r.z))
}

/// UE's FMath::RInterpTo on (pitch, yaw, roll).
fn rinterp_to(current: Vec3, target: Vec3, dt: f32, speed: f32) -> Vec3 {
    if dt == 0.0 || current == target {
        return current;
    }
    if speed <= 0.0 {
        return target;
    }
    let delta = normalize(target - current);
    if delta.abs().max_element() <= 1.0e-4 {
        return target;
    }
    normalize(current + delta * (dt * speed).clamp(0.0, 1.0))
}

/// Unreal quaternion <-> Bevy (the export's basis change: (x, z, y)).
fn swap_quat(q: Quat) -> Quat {
    Quat::from_xyzw(-q.x, -q.z, -q.y, q.w)
}

/// Unreal position (cm) of a Bevy one.
fn ue_pos(v: Vec3) -> Vec3 {
    Vec3::new(v.x, v.z, v.y) * 100.0
}

/// UKismetMathLibrary::Conv_VectorToRotator: (pitch, yaw, 0) of a direction.
fn vector_rotator(v: Vec3) -> Vec3 {
    Vec3::new(v.z.atan2((v.x * v.x + v.y * v.y).sqrt()).to_degrees(), v.y.atan2(v.x).to_degrees(), 0.0)
}

/// rand() / RAND_MAX: the face's own stream (the game's GameRng stays the AI's).
fn frand(state: &mut u64) -> f32 {
    let mut x = *state;
    x ^= x >> 12;
    x ^= x << 25;
    x ^= x >> 27;
    *state = x;
    ((x.wrapping_mul(0x2545_f491_4f6c_dd1d) >> 40) as f32) / (1u64 << 24) as f32
}

#[allow(clippy::too_many_arguments)]
fn face(
    mut commands: Commands,
    time: Res<Time>,
    character: Option<Res<CounselorCharacter>>,
    rigs: Query<(&ChildOf, &Rig)>,
    players: Query<(&Player, Option<&f13_bevy::CounselorFear>)>,
    mut faces: Query<&mut FaceAnim>,
    children: Query<&Children>,
    mut weights: Query<(&mut MorphWeights, &Children)>,
    mesh_of: Query<&Mesh3d>,
    meshes: Res<Assets<Mesh>>,
    mut state: Local<(u64, Option<FaceTuning>)>,
    (names, mut bones, parents): (Query<&Name>, Query<(&mut Transform, &GlobalTransform)>, Query<&ChildOf>),
) {
    let Some(character) = character else { return };
    let info = &character.0.info;
    let Some(face_index) = info.parts.iter().position(|p| p.name == "Face") else { return };
    let (rng, tuning) = &mut *state;
    if *rng == 0 {
        *rng = 0x9e37_79b9_7f4a_7c15;
    }
    let t = tuning.get_or_insert_with(|| FaceTuning::new(info));
    let dt = time.delta_secs();
    for (parent, rig) in &rigs {
        let Ok((p, fear)) = players.get(parent.parent()) else { continue };
        let Some(face_entity) = rig.part(face_index) else { continue };
        let Ok(mut f) = faces.get_mut(face_entity) else {
            let timer = frand(rng) * (t.blink_max - t.blink_min) + t.blink_min;
            commands.entity(face_entity).try_insert(FaceAnim { blink_timer: timer, blink: false, eyelid: 0.0, eyelid_target: 1.0, fear: 0.0, shock: 0.0, look: Look::default() });
            continue;
        };
        let alive = p.counselor.alive();
        let mut set: Vec<(&str, f32)> = vec![];
        if !alive {
            set.push((&t.blink_left, 1.0));
            set.push((&t.blink_right, 1.0));
        }
        // The fear faces (PLACEHOLDER: during the body's montages the game drives the face from their curves instead,
        // not ported; the fear faces carry on here).
        if let Some([cx, cy]) = &t.emotion {
            let fear_alpha = fear.map_or(0.0, |f| f.0.fear) * 0.009999999776482582;
            let length = cx.keys.iter().chain(cy.keys.iter()).map(|k| k.time).fold(0.0, f32::max);
            let now = time.elapsed_secs();
            let at = if length.abs() > 9.99999993922529e-09 { now - (now / length).trunc() * length } else { 0.0 };
            f.shock = if fear_alpha <= t.shock_min { finterp_to(f.shock, 0.0, dt, t.to_neutral) } else { fear_alpha.min(cx.eval(at)) };
            f.fear = if fear_alpha <= t.fear_min { finterp_to(f.fear, 0.0, dt, t.to_neutral) } else { fear_alpha.min(cy.eval(at)) };
            set.push((&t.fear_morph, f.fear));
            set.push((&t.shock_morph, f.shock));
        }
        // The eyes' look (alive only).
        let find = |name: &str| children.iter_descendants(face_entity).find(|&d| names.get(d).is_ok_and(|n| n.as_str() == name));
        if let (true, Some(le), Some(re)) = (alive, find(&t.left_eye), find(&t.right_eye)) {
            let parent = parents.get(le).map(|c| c.parent()).ok();
            let gt = |e: Entity| bones.get(e).map(|(_, g)| g.compute_transform()).ok();
            if let (Some(l), Some(r), Some(pg), Some(cg)) = (gt(le), gt(re), parent.and_then(gt), gt(face_entity)) {
                let k = &mut f.look;
                let settled = normalize(k.left_target - k.left).abs().max_element() <= 0.009999999776482582;
                let mut aim = false;
                if settled {
                    if k.ahead {
                        k.timer -= dt;
                        if k.timer <= 0.0 {
                            k.trigger = true;
                            k.timer = frand(rng) * (t.look_max - t.look_min) + t.look_min;
                            k.hold = frand(rng) * t.max_look_time;
                        }
                    }
                    if k.trigger && k.ahead {
                        let yaw = frand(rng) * 2.0 * t.max_yaw - t.max_yaw;
                        let pitch = frand(rng) * 2.0 * t.max_pitch - t.max_pitch;
                        k.offset = (f13_core::camera_spline::quaternion(Vec3::new(pitch, yaw, 0.0)) * Vec3::X).normalize_or_zero() * t.look_distance;
                        k.ahead = false;
                        k.trigger = false;
                        aim = true;
                    } else if !k.ahead {
                        if k.hold > 0.0 {
                            k.hold -= dt;
                        } else {
                            k.offset = Vec3::X * t.look_distance;
                            k.ahead = true;
                            aim = true;
                        }
                    } else {
                        aim = true;
                    }
                }
                if aim {
                    let (lp, rp) = (ue_pos(l.translation), ue_pos(r.translation));
                    let parent_rot = swap_quat(pg.rotation);
                    let target = (lp + rp) * 0.5 + parent_rot * k.offset;
                    let pr = f13_core::camera_spline::rotator(parent_rot);
                    let d = |eye: Vec3| {
                        let look = vector_rotator(target - eye);
                        Vec3::new(0.0, pr.x - look.x, pr.y - look.y)
                    };
                    k.left_target = d(lp);
                    k.right_target = d(rp);
                }
                k.left = rinterp_to(k.left, k.left_target, dt, t.eye_speed);
                k.right = rinterp_to(k.right, k.right_target, dt, t.eye_speed);
                // ModifyBone: the component-space rotation replaced (the face's own transform as component space).
                for (e, rot) in [(le, k.left), (re, k.right)] {
                    let world = cg.rotation * swap_quat(f13_core::camera_spline::quaternion(rot));
                    if let Ok((mut xf, _)) = bones.get_mut(e) {
                        xf.rotation = pg.rotation.inverse() * world;
                    }
                }
            }
        }
        // Blinks (alive only).
        if alive {
            f.blink_timer -= dt;
            if f.blink_timer <= 0.0 {
                f.blink = true;
                f.blink_timer = frand(rng) * (t.blink_max - t.blink_min) + t.blink_min;
            }
            if (f.eyelid_target - f.eyelid).abs() <= 0.1 {
                if (f.eyelid_target - t.eyelid_min).abs() > 0.1 || !f.blink {
                    f.eyelid_target = t.eyelid_min;
                } else {
                    f.eyelid_target = t.eyelid_max;
                    f.blink = false;
                }
            }
            let v = dt * t.eyelid_speed * (f.eyelid_target - f.eyelid) + f.eyelid;
            f.eyelid = v.clamp(t.eyelid_min, t.eyelid_max);
            set.push((&t.eyelid_morph, f.eyelid));
            set.push((&t.eyelash_morph, f.eyelid));
        }
        // SetMorphTarget: by name on the face's meshes.
        for d in children.iter_descendants(face_entity) {
            let Ok((mut w, kids)) = weights.get_mut(d) else { continue };
            let Some(names) = kids.iter().find_map(|k| mesh_of.get(k).ok()).and_then(|m| meshes.get(&m.0)).and_then(|m| m.morph_target_names().map(|n| n.to_vec())) else { continue };
            for &(name, value) in &set {
                if let Some(slot) = names.iter().position(|n| n == name).and_then(|i| w.weights_mut().get_mut(i)) {
                    *slot = value;
                }
            }
        }
    }
}
