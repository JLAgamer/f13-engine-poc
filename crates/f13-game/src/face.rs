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
//! - the base pose: the graph's cached SequencePlayer (Face_AnimBP_<counselor>'s Counselor_Face_<counselor>_Anim,
//!   looping at rate 1; the export's faces/<counselor>.json, named by Face.BasePose) on the face's bones, under the
//!   eyes' ModifyBones; dead or alive (the dead branch is the cached pose).
//!
//! - in a body montage (its full-body action here): as it starts, FacesOfFearNames (Fear, Shock) 0; each
//!   MontageFaceNames morph (Concept, Face_Fear -> Fear, Face_Shock -> Shock, Face_Anger -> Anger, Face_Disgust ->
//!   Disgust, Face_Happy -> Happy) set to its body curve (the face BP's CurveNames only; others 0) when it moves by
//!   more than 1e-4; the EmotionCurve fear faces only outside montages.
//!
//! - alive: the graph's additives over the base pose, weighted by the montage's Face_Jaw / Brow_* / L_/R_Brow_Alpha
//!   curves ([`additives`]). The counselor faces are skinned to Head_Root and the eyes only (their expressions are the
//!   morph targets), so the base pose and these additives move nothing on them that shows.
//!
//! Not ported yet: bAnimationOverride's blink override and look at target_jnt.

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
    /// The base pose's play time (its sequence player's accumulator).
    pose_time: f32,
    /// bIsBodyPlayingMontage, and MontageFaceNames' last values.
    montage: bool,
    montage_last: [f32; 6],
    /// JawAlpha, BrowAnger, BrowFear, BrowShock, BrowLAlpha, BrowRAlpha (set only during body montages; they keep
    /// their last values after).
    alphas: [f32; 6],
    /// The face bones' rest local transforms (the mesh's reference pose), by bone name.
    rest: std::collections::HashMap<String, Transform>,
}

/// A single-frame pose's local transforms by bone name (glTF space; no translation: the bone's retargeted one, the
/// mesh's reference).
type Pose = std::collections::HashMap<String, (Quat, Option<Vec3>)>;

/// The first frame of a face sequence the export wrote (faces/<counselor>/<name>.anim) as a pose.
fn load_pose(root: &std::path::Path, info: &f13_data::character::CharacterInfo, name: &str) -> Option<Pose> {
    let index: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(root.join("faces").join(format!("{}.json", info.name))).ok()?).ok()?;
    let data = f13_data::character::AnimData::load(&root.join(index[name]["File"].as_str()?)).ok()?;
    Some(
        data.tracks
            .iter()
            .filter_map(|t| Some((t.bone.clone(), (Quat::from_array(*t.rotation.first()?).normalize(), t.translation.as_ref().and_then(|v| v.first()).map(|v| Vec3::from_array(*v))))))
            .collect(),
    )
}

/// SCFaceAnimInstance's constructor (0x1402f3840): MontageFaceNames (body curve -> morph target) and
/// FacesOfFearNames (zeroed as a body montage starts).
const MONTAGE_FACE: [(&str, &str); 6] = [("Concept", "Concept"), ("Face_Fear", "Fear"), ("Face_Shock", "Shock"), ("Face_Anger", "Anger"), ("Face_Disgust", "Disgust"), ("Face_Happy", "Happy")];
const FACES_OF_FEAR: [&str; 2] = ["Fear", "Shock"];
/// Face_AnimBP's CurveNames: the body curves it copies each update (GetBodyCurveValue reads only these; others are 0).
const CURVE_NAMES: [&str; 11] = ["Face_Fear", "Face_Shock", "Face_Anger", "Face_Jaw", "Brow_Fear", "Brow_Shock", "Brow_Anger", "L_Brow_Alpha", "R_Brow_Alpha", "Blink", "Face_Disgust"];

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
    /// The base pose and its length (s).
    base: Option<(f13_data::character::AnimData, f32)>,
    /// JawOpen's local-space additive (AAT_LocalSpaceBase against its RefPoseSeq: rotation target x base^-1,
    /// translation target - base), and the brows' mesh-space rotation targets (AAT_RotationOffsetMeshSpace against
    /// the reference pose: their deltas need the rest pose).
    jaw: Option<Pose>,
    brows: [Option<Pose>; 3],
}

impl FaceTuning {
    fn new(info: &f13_data::character::CharacterInfo, root: &std::path::Path) -> Self {
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
            base: info.names.get("Face.BasePose").and_then(|seq| {
                let index: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(root.join("faces").join(format!("{}.json", info.name))).ok()?).ok()?;
                let file = index[seq]["File"].as_str()?;
                Some((f13_data::character::AnimData::load(&root.join(file)).ok()?, index[seq]["Length"].as_f64()? as f32))
            }),
            jaw: (|| {
                let target = load_pose(root, info, info.names.get("Face.JawOpen")?)?;
                let base = load_pose(root, info, info.names.get("Face.JawOpen.RefPoseSeq")?)?;
                Some(target.iter().filter_map(|(b, (q, t))| base.get(b).map(|(bq, bt)| (b.clone(), (*q * bq.inverse(), Some(t.zip(*bt).map_or(Vec3::ZERO, |(t, bt)| t - bt)))))).collect())
            })(),
            brows: ["BrowAnger", "BrowFear", "BrowShock"].map(|k| info.names.get(&format!("Face.{k}")).and_then(|n| load_pose(root, info, n))),
        }
    }
}

/// Face_AnimBP's graph after its cached base pose: the right brows (R_Brow_1..3 and below) blended toward the base
/// plus the mesh-space brow additives (Brows_Anger_01 x BrowAnger, then Brows_Fear_01 x BrowFear, Brows_Suprise_01 x
/// BrowShock) by BrowRAlpha, the left brows likewise by BrowLAlpha (LayeredBoneBlends, local blend), then JawOpen's
/// local-space additive x JawAlpha on everything. Additives as FTransform::BlendFromIdentityAndAccumulate: rotation
/// slerp(identity, delta, w) x base, translation + delta x w; the mesh-space ones (FAnimationRuntime::
/// AccumulateMeshSpaceRotationAdditiveToLocalPose) on the component-space rotations, then back to local.
fn additives(
    t: &FaceTuning,
    f: &mut FaceAnim,
    face_entity: Entity,
    children: &Query<&Children>,
    names: &Query<&Name>,
    bones: &mut Query<(&mut Transform, &GlobalTransform)>,
    parents: &Query<&ChildOf>,
) {
    let [jaw_a, anger, fear, shock, l_a, r_a] = f.alphas;
    if [jaw_a, anger * l_a.max(r_a), fear * l_a.max(r_a), shock * l_a.max(r_a)].iter().all(|w| w.abs() < 1e-6) {
        return;
    }
    // The skeleton: each bone's entity, name, parent (by name) and current local pose, root first.
    let mut list: Vec<(Entity, String, Option<String>)> = vec![];
    for e in children.iter_descendants(face_entity) {
        let (Ok(n), Ok(_)) = (names.get(e), bones.get(e)) else { continue };
        let parent = parents.get(e).ok().and_then(|c| names.get(c.parent()).ok()).map(|p| p.as_str().to_string());
        list.push((e, n.as_str().to_string(), parent));
    }
    let local: std::collections::HashMap<String, (Quat, Vec3)> = list.iter().filter_map(|(e, n, _)| bones.get(*e).ok().map(|(x, _)| (n.clone(), (x.rotation, x.translation)))).collect();
    let parent_of: std::collections::HashMap<&str, &str> = list.iter().filter_map(|(_, n, p)| p.as_deref().filter(|p| local.contains_key(*p)).map(|p| (n.as_str(), p))).collect();
    // Component-space rotation of a bone from local rotations.
    fn cs(rot: &dyn Fn(&str) -> Quat, parent_of: &std::collections::HashMap<&str, &str>, bone: &str) -> Quat {
        let mut q = rot(bone);
        let mut b = bone;
        while let Some(p) = parent_of.get(b) {
            q = rot(p) * q;
            b = p;
        }
        q
    }
    let rest_q = |b: &str| f.rest.get(b).map_or(Quat::IDENTITY, |x| x.rotation);
    // The brow branch: the base plus the three mesh-space additives.
    let mut branch = local.clone();
    for (target, w) in t.brows.iter().zip([anger, fear, shock]) {
        let Some(target) = target else { continue };
        if w.abs() < 1e-6 {
            continue;
        }
        let branch_q = branch.clone();
        let now_q = |b: &str| branch_q.get(b).map_or(Quat::IDENTITY, |x| x.0);
        let target_q = |b: &str| target.get(b).map_or_else(|| rest_q(b), |x| x.0);
        let mut cs_new: std::collections::HashMap<String, Quat> = branch.keys().map(|b| (b.clone(), cs(&now_q, &parent_of, b))).collect();
        for (b, (_, tt)) in target {
            let Some(rest) = f.rest.get(b) else { continue };
            // The delta against the reference pose, in component space (translations local).
            let delta_rot = cs(&target_q, &parent_of, b) * cs(&rest_q, &parent_of, b).inverse();
            let delta_t = tt.map_or(Vec3::ZERO, |t| t - rest.translation);
            if let Some(q) = cs_new.get_mut(b) {
                *q = Quat::IDENTITY.slerp(delta_rot, w) * *q;
            }
            if let Some(l) = branch.get_mut(b) {
                l.1 += delta_t * w;
            }
        }
        for (b, l) in branch.iter_mut() {
            let parent_cs = parent_of.get(b.as_str()).and_then(|p| cs_new.get(*p)).copied().unwrap_or(Quat::IDENTITY);
            l.0 = parent_cs.inverse() * cs_new[b];
        }
    }
    // The layered blends on the brows (BlendDepth 0: each filter bone and everything below it).
    let under = |b: &str, roots: [&str; 3]| -> bool {
        let mut c = b;
        loop {
            if roots.contains(&c) {
                return true;
            }
            match parent_of.get(c) {
                Some(p) => c = p,
                None => return false,
            }
        }
    };
    let mut out = local.clone();
    for (b, l) in out.iter_mut() {
        let w = if under(b, ["R_Brow_1", "R_Brow_2", "R_Brow_3"]) {
            r_a
        } else if under(b, ["L_Brow_1", "L_Brow_2", "L_Brow_3"]) {
            l_a
        } else {
            continue;
        };
        let w = w.clamp(0.0, 1.0);
        if let Some(target) = branch.get(b) {
            *l = (l.0.slerp(target.0, w), l.1.lerp(target.1, w));
        }
    }
    // JawOpen.
    if let Some(jaw) = &t.jaw {
        if jaw_a.abs() > 1e-6 {
            for (b, (dq, dt)) in jaw {
                if let Some(l) = out.get_mut(b) {
                    *l = (Quat::IDENTITY.slerp(*dq, jaw_a) * l.0, l.1 + dt.unwrap_or(Vec3::ZERO) * jaw_a);
                }
            }
        }
    }
    for (e, n, _) in &list {
        if let (Some(&(q, v)), Ok((mut x, _))) = (out.get(n), bones.get_mut(*e)) {
            x.rotation = q.normalize();
            x.translation = v;
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
    root: Res<crate::ExportRoot>,
    character: Option<Res<CounselorCharacter>>,
    rigs: Query<(&ChildOf, &Rig)>,
    players: Query<(&Player, Option<&f13_bevy::CounselorFear>)>,
    mut faces: Query<&mut FaceAnim>,
    children: Query<&Children>,
    mut weights: Query<(&mut MorphWeights, &Children)>,
    mesh_of: Query<&Mesh3d>,
    meshes: Res<Assets<Mesh>>,
    mut state: Local<(u64, std::collections::HashMap<String, FaceTuning>)>,
    (names, mut bones, parents): (Query<&Name>, Query<(&mut Transform, &GlobalTransform)>, Query<&ChildOf>),
) {
    let Some(character) = character else { return };
    let info = &character.0.info;
    let Some(face_index) = info.parts.iter().position(|p| p.name == "Face") else { return };
    let (rng, tuning) = &mut *state;
    if *rng == 0 {
        *rng = 0x9e37_79b9_7f4a_7c15;
    }
    let dt = time.delta_secs();
    for (parent, rig) in &rigs {
        let Ok((p, fear)) = players.get(parent.parent()) else { continue };
        // Her own face's (her class's base pose and poses).
        let looks = rig.visual.as_deref().unwrap_or(info);
        let t = &*tuning.entry(looks.name.clone()).or_insert_with(|| FaceTuning::new(looks, &root.0));
        let Some(face_entity) = rig.part(face_index) else { continue };
        let Ok(mut f) = faces.get_mut(face_entity) else {
            let timer = frand(rng) * (t.blink_max - t.blink_min) + t.blink_min;
            commands.entity(face_entity).try_insert(FaceAnim { blink_timer: timer, blink: false, eyelid: 0.0, eyelid_target: 1.0, fear: 0.0, shock: 0.0, look: Look::default(), pose_time: 0.0, montage: false, montage_last: [0.0; 6], alphas: [0.0; 6], rest: Default::default() });
            continue;
        };
        let alive = p.counselor.alive();
        let mut set: Vec<(&str, f32)> = vec![];
        if !alive {
            set.push((&t.blink_left, 1.0));
            set.push((&t.blink_right, 1.0));
        }
        // The body's montage (Montage_IsPlaying: its full-body action) and its copied curves at its time.
        let action = rig.action.as_ref().and_then(|(name, at)| info.animations.get(name).map(|a| (a, *at)));
        let body_curve = |name: &str| -> f32 {
            let Some((a, at)) = action else { return 0.0 };
            if !CURVE_NAMES.contains(&name) {
                return 0.0;
            }
            a.curve(name).map_or(0.0, |c| c.eval(at))
        };
        let montage = action.is_some();
        if montage && !f.montage {
            for m in FACES_OF_FEAR {
                set.push((m, 0.0));
            }
        }
        f.montage = montage;
        if montage {
            // JawAlpha and the brow alphas from their body curves; each MontageFaceNames morph follows its body curve
            // when it moves by more than 1e-4.
            for (i, k) in ["Face_Jaw", "Brow_Anger", "Brow_Fear", "Brow_Shock", "L_Brow_Alpha", "R_Brow_Alpha"].iter().enumerate() {
                f.alphas[i] = body_curve(k);
            }
            for (i, (curve, morph)) in MONTAGE_FACE.iter().enumerate() {
                let v = body_curve(curve);
                if (v - f.montage_last[i]).abs() > 9.999999747378752e-05 {
                    set.push((morph, v));
                    f.montage_last[i] = v;
                }
            }
        } else if let Some([cx, cy]) = &t.emotion {
            // The fear faces, outside montages.
            let fear_alpha = fear.map_or(0.0, |f| f.0.fear) * 0.009999999776482582;
            let length = cx.keys.iter().chain(cy.keys.iter()).map(|k| k.time).fold(0.0, f32::max);
            let now = time.elapsed_secs();
            let at = if length.abs() > 9.99999993922529e-09 { now - (now / length).trunc() * length } else { 0.0 };
            f.shock = if fear_alpha <= t.shock_min { finterp_to(f.shock, 0.0, dt, t.to_neutral) } else { fear_alpha.min(cx.eval(at)) };
            f.fear = if fear_alpha <= t.fear_min { finterp_to(f.fear, 0.0, dt, t.to_neutral) } else { fear_alpha.min(cy.eval(at)) };
            set.push((&t.fear_morph, f.fear));
            set.push((&t.shock_morph, f.shock));
        }
        // The rest pose (the bones as the mesh spawned them), captured before anything poses them.
        for bone in children.iter_descendants(face_entity) {
            if let (Ok(n), Ok((x, _))) = (names.get(bone), bones.get(bone)) {
                f.rest.entry(n.as_str().to_string()).or_insert(*x);
            }
        }
        // The base pose, looping.
        if let Some((data, length)) = &t.base {
            f.pose_time = if *length > 0.0 { (f.pose_time + dt) % length } else { 0.0 };
            let frames = data.frames.max(1);
            let x = if *length > 0.0 { f.pose_time / length * (frames - 1) as f32 } else { 0.0 };
            let (i, a) = (x.floor() as usize, x.fract());
            let j = (i + 1).min(frames - 1);
            for bone in children.iter_descendants(face_entity) {
                let Ok(name) = names.get(bone) else { continue };
                let Some(track) = data.tracks.iter().find(|t| t.bone == name.as_str()) else { continue };
                let Ok((mut xf, _)) = bones.get_mut(bone) else { continue };
                if let (Some(q0), Some(q1)) = (track.rotation.get(i), track.rotation.get(j)) {
                    xf.rotation = Quat::from_array(*q0).normalize().slerp(Quat::from_array(*q1).normalize(), a);
                }
                if let Some((t0, t1)) = track.translation.as_ref().and_then(|t| Some((t.get(i)?, t.get(j)?))) {
                    xf.translation = Vec3::from_array(*t0).lerp(Vec3::from_array(*t1), a);
                }
            }
        }
        // The graph's additives over the base pose (alive: the dead branch is the cached base pose alone).
        if alive {
            additives(&t, &mut f, face_entity, &children, &names, &mut bones, &parents);
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
