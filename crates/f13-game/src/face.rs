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
//! Not ported yet: the eyes' look (their L_Eye / R_Eye bones), the face skeleton's base pose, brow and jaw additives,
//! and the body montages' face curves (MontageFaceNames, the jaw and brow alphas, the blink override).

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
        }
    }
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
            commands.entity(face_entity).insert(FaceAnim { blink_timer: timer, blink: false, eyelid: 0.0, eyelid_target: 1.0, fear: 0.0, shock: 0.0 });
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
