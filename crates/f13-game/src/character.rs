//! Character models and animations, from the export's `counselors/<Blueprint>.json`, `killers/<Blueprint>.json`
//! and their `.anim` files. Jason (jason.rs) is assembled the same way: his mask and hair follow the body as
//! master-pose parts and his weapon hangs off its hand socket.
//!
//! Assembled as the game does it: the bare base body (CharacterMesh0) carries the skeleton, the outfit follows it as
//! a master-pose component (its skin is re-bound to the body's bones of the same names), the face hangs off the head
//! bone and the hair off the face. The face and hair stay in their bind pose (their own anim Blueprints, facial
//! animation and hair dynamics, are not ported).
//!
//! Animations are retargeted as Unreal does on the shared counselor skeleton: rotations from the animation,
//! translations from the animation only for the bones whose retargeting mode is "Animation" (root, pelvis, weapon
//! attach points); every other bone keeps the body's own bind-pose translation (the exporter only writes the former).
//!
//! Blending follows Athlete_AnimBP's female blendspaces: BS_F_Normal_Move (SpeedState 0 idle, 1 walk, 2 run,
//! 3 sprint), BS_F_Combat (direction x speed: combat idle, eight walk directions), crouch idle/walk, the wounded idle,
//! and full-body montages (dodges, stumbles, first aid). Weights move toward their targets at the blendspaces'
//! TargetWeightInterpolationSpeedPerSec; cycles are synced by normalised time (the game syncs by foot markers).

use bevy::animation::{animated_field, AnimatedBy, AnimationTargetId};
use bevy::math::curve::{ConstantCurve, Interval, SampleAutoCurve};
use bevy::prelude::*;
use bevy::world_serialization::{WorldAssetRoot, WorldInstanceReady};
use f13_core::counselor::Stance;
use f13_data::character::{AnimData, CharacterInfo};
use std::collections::HashMap;

use crate::player::{Player, PlayerTuning};

/// Weight change per second (BS_F_Normal_Move's and BS_F_Combat's TargetWeightInterpolationSpeedPerSec).
const BLEND_RATE: f32 = 4.0;
/// Montage blend in/out (Unreal's default 0.25 s): weight change per second.
const MONTAGE_BLEND_RATE: f32 = 4.0;

const IDLE: &str = "F_normal_stand_idle_01";
const HURT_IDLE: &str = "F_hurt_stand_idle_01";
const MOVE: [&str; 3] = ["F_normal_stand_walk_forward", "F_normal_stand_run_forward", "F_normal_stand_sprint_forward"];
const CROUCH_IDLE: &str = "f_normal_crouch_idle_01";
const CROUCH_WALK: &str = "f_normal_crouch_walk_forward";
const COMBAT_IDLE: &str = "f_combat_stand_idle_1h";
/// BS_F_Combat's walk samples by direction (degrees, + is right).
const COMBAT_WALK: [(f32, &str); 9] = [
    (-180.0, "F_normal_stand_walk_back"),
    (-135.0, "F_normal_stand_walk_back_45_left"),
    (-90.0, "F_normal_stand_walk_left"),
    (-45.0, "F_normal_stand_walk_forward_45_left"),
    (0.0, "F_normal_stand_walk_forward"),
    (45.0, "F_normal_stand_walk_forward_45_right"),
    (90.0, "F_normal_stand_walk_right"),
    (135.0, "F_normal_stand_walk_back_45_right"),
    (180.0, "F_normal_stand_walk_back"),
];
const FIRST_AID: &str = "f_normal_stand_use_firstaid_self";

/// The female sequence Vanessa plays for a montage the rules start (the shared montages hold the male takes; the
/// female ones have the same length and root motion).
fn female_sequence(montage: &str) -> Option<&'static str> {
    Some(match montage {
        "AM_combat_stand_Dodge_Left" => "f_combat_stand_dodge_left_01",
        "AM_combat_stand_Dodge_Right" => "f_combat_stand_dodge_right_01",
        "AM_combat_stand_dodge_back" => "f_combat_stand_dodge_back_01",
        "AM_normal_stand_run_stumble_short" => "f_normal_stand_run_stumble_short_01",
        "AM_normal_stand_sprint_stumble_short" => "f_normal_stand_sprint_stumble_short_01",
        _ => return None,
    })
}

/// Loads the MVP counselor (Vanessa) and Jason from the export before anything spawns, then assembles their models
/// and animates the counselor (Jason's animation is driven by jason.rs).
pub struct CharacterPlugin {
    pub export: std::path::PathBuf,
    /// Testing: hold the counselor in one animation at a time (seconds) instead of driving it from her state.
    pub pose: Option<(String, f32)>,
}

#[derive(Resource)]
pub(crate) struct ForcedPose(String, f32);

/// Vanessa's and Jason's Blueprints, as the exporter names their files.
const COUNSELOR: &str = "Athlete_Counselor";
const KILLER: &str = "Jason_SackHead";

impl Plugin for CharacterPlugin {
    fn build(&self, app: &mut App) {
        let export = self.export.clone();
        if let Some((name, at)) = self.pose.clone() {
            app.insert_resource(ForcedPose(name, at));
        }
        app.add_systems(PreStartup, move |mut commands: Commands, mut clips: ResMut<Assets<AnimationClip>>, mut graphs: ResMut<Assets<AnimationGraph>>| {
            if let Some(c) = CharacterModel::load(&export.join("counselors").join(format!("{COUNSELOR}.json")), &export, &mut clips, &mut graphs) {
                commands.insert_resource(CounselorCharacter(c));
            }
            if let Some(c) = CharacterModel::load(&export.join("killers").join(format!("{KILLER}.json")), &export, &mut clips, &mut graphs) {
                commands.insert_resource(KillerCharacter(c));
            }
        })
        .add_systems(Update, (assemble, animate).chain());
    }
}

/// A loaded character: its description and one graph node per animation (with its length).
pub struct CharacterModel {
    pub info: CharacterInfo,
    graph: Handle<AnimationGraph>,
    clips: HashMap<String, (AnimationNodeIndex, f32)>,
}

#[derive(Resource, Deref)]
pub struct CounselorCharacter(pub CharacterModel);

#[derive(Resource, Deref)]
pub struct KillerCharacter(pub CharacterModel);

/// Which loaded model a rig shows.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Model {
    Counselor,
    Killer,
}

impl CharacterModel {
    /// Loads a character file and its animations. None if the export predates character export.
    pub fn load(path: &std::path::Path, export: &std::path::Path, clips: &mut Assets<AnimationClip>, graphs: &mut Assets<AnimationGraph>) -> Option<Self> {
        let info = match CharacterInfo::load(path) {
            Ok(i) => i,
            Err(e) => {
                warn!("no character model {} ({e}); re-export with tools/f13-export counselor / killer", path.display());
                return None;
            }
        };
        let animated: Vec<&str> = info.bones.iter().filter(|b| b.retarget == "Animation").map(|b| b.name.as_str()).collect();
        let mut graph = AnimationGraph::new();
        let mut nodes = HashMap::new();
        for (name, a) in &info.animations {
            let data = match AnimData::load(&export.join(&a.file)) {
                Ok(d) => d,
                Err(e) => {
                    warn!("{name}: {e}");
                    continue;
                }
            };
            let clip = build_clip(&data, a.length, &animated);
            nodes.insert(name.clone(), (graph.add_clip(clips.add(clip), 1.0, graph.root), a.length));
        }
        info!("character model {}: {} animations", info.name, nodes.len());
        Some(Self { info, graph: graphs.add(graph), clips: nodes })
    }

    /// An animation's length, if it was exported.
    pub fn length(&self, name: &str) -> Option<f32> {
        self.clips.get(name).map(|c| c.1)
    }
}

fn build_clip(data: &AnimData, length: f32, animated: &[&str]) -> AnimationClip {
    let mut clip = AnimationClip::default();
    let interval = Interval::new(0.0, length.max(1e-3)).unwrap();
    for t in &data.tracks {
        let target = AnimationTargetId::from_name(&Name::new(t.bone.clone()));
        if !t.rotation.is_empty() {
            let samples: Vec<Quat> = t.rotation.iter().map(|q| Quat::from_array(*q).normalize()).collect();
            if samples.len() >= 2 {
                let curve = SampleAutoCurve::new(interval, samples).unwrap();
                clip.add_curve_to_target(target, AnimatableCurve::new(animated_field!(Transform::rotation), curve));
            } else {
                clip.add_curve_to_target(target, AnimatableCurve::new(animated_field!(Transform::rotation), ConstantCurve::new(interval, samples[0])));
            }
        }
        if let Some(tr) = t.translation.as_ref().filter(|_| animated.contains(&t.bone.as_str())) {
            let samples: Vec<Vec3> = tr.iter().map(|v| Vec3::from_array(*v)).collect();
            if samples.len() >= 2 {
                let curve = SampleAutoCurve::new(interval, samples).unwrap();
                clip.add_curve_to_target(target, AnimatableCurve::new(animated_field!(Transform::translation), curve));
            } else {
                clip.add_curve_to_target(target, AnimatableCurve::new(animated_field!(Transform::translation), ConstantCurve::new(interval, samples[0])));
            }
        }
    }
    clip.set_duration(length);
    clip
}

/// Unreal actors face +X, characters here face +Z: the rig turns a quarter about Y on top of the mesh's own yaw, so
/// the owner's transform times this is the Unreal actor's.
pub const FACING: Quat = Quat::from_xyzw(0.0, -std::f32::consts::FRAC_1_SQRT_2, 0.0, std::f32::consts::FRAC_1_SQRT_2);

/// The animated model under a character: holds the AnimationPlayer every bone is animated by.
#[derive(Component)]
pub struct Rig {
    pub model: Model,
    body: Entity,
    outfit: Vec<Entity>,
    /// Parts hanging off a bone or another part, spawned once the body is in: (part index, entity holding its scene).
    attached: Vec<(usize, Entity)>,
    bones_tagged: bool,
    /// The body's bones by name (filled once its scene is in).
    pub bones: HashMap<String, Entity>,
    outfit_bound: Vec<Entity>,
    /// Current weight per animation.
    weights: HashMap<String, f32>,
    /// Shared phase (0..1) of the synced locomotion cycles, and free-running time of the idles.
    phase: f32,
    idle_time: f32,
    /// The montage or action playing: its animation and time since it started.
    pub action: Option<(String, f32)>,
}

/// A full-body animation a character plays now (hit reactions, grabs, kills, death), whatever its movement state.
/// Restarts when the name changes; holds its last frame once over.
#[derive(Component, Clone, Debug, PartialEq)]
pub struct PlayAction(pub String);

/// A part's scene finished spawning.
#[derive(Component)]
struct SceneReady;

/// Spawns the model under `owner` (feet at its origin, facing +Z), with `marker` on the rig. Returns the rig entity.
pub fn spawn_model(commands: &mut Commands, assets: &AssetServer, character: &CharacterModel, model: Model, owner: Entity, marker: impl Bundle) -> Entity {
    let info = &character.info;
    // CharacterMesh0 sits at MeshTransform from the capsule's centre; the player's origin is the capsule's bottom.
    // Unreal actors face +X, the player here faces +Z: a quarter turn about Y on top of the mesh's own yaw.
    let mut mesh = crate::to_transform(&info.mesh_transform);
    mesh.translation.y += info.capsule_half_height / 100.0;
    let rig_xf = Transform::from_rotation(FACING) * mesh;

    let ready = |ev: On<WorldInstanceReady>, mut commands: Commands| {
        commands.entity(ev.entity).insert(SceneReady);
    };
    let rig = commands
        .spawn((rig_xf, Visibility::default(), AnimationPlayer::default(), AnimationGraphHandle(character.graph.clone()), marker, ChildOf(owner)))
        .id();
    let mut body = commands.spawn((WorldAssetRoot(assets.load(GltfAssetLabel::Scene(0).from_asset(info.body.clone()))), ChildOf(rig)));
    if info.skin_masks.len() == 2 {
        body.insert(crate::materials::SkinMasks(info.skin_masks.clone()));
    }
    let body = body.observe(ready).id();
    let mut outfit = Vec::new();
    for p in info.parts.iter().filter(|p| p.master_pose()) {
        let mut e = commands.spawn((WorldAssetRoot(assets.load(GltfAssetLabel::Scene(0).from_asset(p.file.clone()))), ChildOf(rig)));
        if let Some(o) = crate::materials::MaterialOverrides::new(&p.material_map) {
            e.insert(o);
        }
        outfit.push(e.observe(ready).id());
    }
    commands.entity(rig).insert(Rig {
        model,
        body,
        outfit,
        attached: Vec::new(),
        bones_tagged: false,
        bones: HashMap::new(),
        outfit_bound: Vec::new(),
        weights: HashMap::new(),
        phase: 0.0,
        idle_time: 0.0,
        action: None,
    });
    rig
}

/// Once the body's scene is in: tags its bones as animation targets and hangs the attached parts off them; once an
/// outfit's scene is in, re-binds its skin to the body's bones.
fn assemble(
    mut commands: Commands,
    assets: Res<AssetServer>,
    counselor: Option<Res<CounselorCharacter>>,
    killer: Option<Res<KillerCharacter>>,
    mut rigs: Query<(Entity, &mut Rig)>,
    ready: Query<(), With<SceneReady>>,
    children: Query<&Children>,
    names: Query<&Name>,
    mut skins: Query<&mut bevy::mesh::skinning::SkinnedMesh>,
) {
    for (rig_entity, mut rig) in &mut rigs {
        if !ready.contains(rig.body) {
            continue;
        }
        let character = match rig.model {
            Model::Counselor => counselor.as_deref().map(|c| &c.0),
            Model::Killer => killer.as_deref().map(|c| &c.0),
        };
        let Some(info) = character.map(|c| &c.info) else { continue };
        let bones: HashMap<&str, Entity> = children
            .iter_descendants(rig.body)
            .filter_map(|e| names.get(e).ok().map(|n| (n.as_str(), e)))
            .filter(|(n, _)| info.bones.iter().any(|b| b.name == *n))
            .collect();
        if !rig.bones_tagged {
            for (name, &e) in &bones {
                commands.entity(e).insert((AnimationTargetId::from_name(&Name::new(name.to_string())), AnimatedBy(rig_entity)));
            }
            rig.bones_tagged = true;
            rig.bones = bones.iter().map(|(n, e)| (n.to_string(), *e)).collect();
            // Parts attached to a bone first, then parts attached to those parts.
            let mut pending: Vec<usize> = (0..info.parts.len()).filter(|&i| !info.parts[i].master_pose()).collect();
            while !pending.is_empty() {
                let before = pending.len();
                pending.retain(|&i| {
                    let p = &info.parts[i];
                    let parent = bones.get(p.attach.as_str()).copied().or_else(|| {
                        rig.attached.iter().find(|(j, _)| info.parts[*j].name == p.attach).map(|(_, e)| *e)
                    });
                    let Some(parent) = parent else { return true };
                    let xf = p.transform.as_ref().map(crate::to_transform).unwrap_or_default();
                    let mut e = commands.spawn((xf, Visibility::default(), ChildOf(parent)));
                    if !p.file.is_empty() {
                        e.insert(WorldAssetRoot(assets.load(GltfAssetLabel::Scene(0).from_asset(p.file.clone()))));
                    }
                    let e = e.id();
                    rig.attached.push((i, e));
                    false
                });
                if pending.len() == before {
                    warn!("character parts with nothing to attach to: {:?}", pending.iter().map(|&i| &info.parts[i].name).collect::<Vec<_>>());
                    break;
                }
            }
        }
        for outfit in rig.outfit.clone() {
            if rig.outfit_bound.contains(&outfit) || !ready.contains(outfit) {
                continue;
            }
            for e in children.iter_descendants(outfit) {
                let Ok(mut skin) = skins.get_mut(e) else { continue };
                for joint in skin.joints.iter_mut() {
                    if let Some(&b) = names.get(*joint).ok().and_then(|n| bones.get(n.as_str())) {
                        *joint = b;
                    }
                }
            }
            rig.outfit_bound.push(outfit);
        }
    }
}

/// Picks the target weight of each animation from the counselor's state, eases the weights toward them, and poses
/// the AnimationPlayer (every clip is positioned by hand: speed 0, seek time set here).
pub(crate) fn animate(
    time: Res<Time>,
    character: Option<Res<CounselorCharacter>>,
    killer: Option<Res<KillerCharacter>>,
    tuning: Res<PlayerTuning>,
    forced: Option<Res<ForcedPose>>,
    players: Query<(&Player, &Transform, Option<&crate::interact::UsingItem>, Option<&PlayAction>)>,
    mut rigs: Query<(&ChildOf, &mut Rig, &mut AnimationPlayer), Without<crate::ragdoll::Ragdolled>>,
) {
    let Some(character) = character else { return };
    let dt = time.delta_secs();
    if let Some(f) = forced {
        for (_, rig, mut anim) in &mut rigs {
            let model = match rig.model {
                Model::Counselor => &character.0,
                Model::Killer => match killer.as_deref() {
                    Some(k) => &k.0,
                    None => continue,
                },
            };
            let Some(&(node, len)) = model.clips.get(&f.0) else { continue };
            anim.play(node).set_speed(0.0).set_weight(1.0).set_seek_time(f.1.min(len));
        }
        return;
    }
    for (parent, mut rig, mut anim) in &mut rigs {
        let Ok((p, xf, using, scripted)) = players.get(parent.parent()) else { continue };
        let c = &p.counselor;
        let t = &tuning.counselor;
        let speed = p.velocity.length();

        // A full-body action: a scripted one (hit, grab, kill, death), the montage the rules started (dodge,
        // stumble) or the first aid spray.
        let action = match (scripted, &p.montage, using) {
            (Some(a), _, _) => Some(a.0.as_str()),
            (_, Some(m), _) if p.root_motion.is_some() => female_sequence(m),
            (_, _, Some(_)) => Some(FIRST_AID),
            _ => None,
        };
        rig.set_action(action, dt);

        let mut targets: HashMap<&str, f32> = HashMap::new();
        if rig.action.as_ref().is_some_and(|(a, _)| !character.clips.contains_key(a.as_str())) {
            targets.insert(IDLE, 1.0);
        } else if rig.action.is_none() {
            match c.stance {
                Stance::Crouching => {
                    let s = (speed / t.max_walk_speed_crouched.max(1.0)).clamp(0.0, 1.0);
                    targets.insert(CROUCH_IDLE, 1.0 - s);
                    targets.insert(CROUCH_WALK, s);
                }
                Stance::Combat => {
                    let s = (speed / t.max_combat_speed.max(1.0)).clamp(0.0, 1.0);
                    targets.insert(COMBAT_IDLE, 1.0 - s);
                    // Direction of travel relative to facing, + to the right (Unreal's CalculateDirection).
                    let (yaw, _, _) = xf.rotation.to_euler(EulerRot::YXZ);
                    let fwd = Vec2::new(yaw.sin(), yaw.cos());
                    let right = Vec2::new(-fwd.y, fwd.x);
                    let dir = p.velocity.dot(right).atan2(p.velocity.dot(fwd)).to_degrees();
                    let i = COMBAT_WALK.iter().position(|(a, _)| *a >= dir).unwrap_or(COMBAT_WALK.len() - 1).max(1);
                    let (a0, n0) = COMBAT_WALK[i - 1];
                    let (a1, n1) = COMBAT_WALK[i];
                    let f = ((dir - a0) / (a1 - a0)).clamp(0.0, 1.0);
                    *targets.entry(n0).or_default() += s * (1.0 - f);
                    *targets.entry(n1).or_default() += s * f;
                }
                _ => {
                    // BS_F_Normal_Move's SpeedState: 0 idle, 1 walk, 2 run, 3 sprint, linear in between.
                    let marks = [0.0, t.max_walk_speed, t.max_run_speed, t.max_sprint_speed];
                    let state = speed_state(speed, &marks);
                    let idle = if c.wounded { HURT_IDLE } else { IDLE };
                    let names = [idle, MOVE[0], MOVE[1], MOVE[2]];
                    for (k, name) in names.iter().enumerate() {
                        let w = (1.0 - (state - k as f32).abs()).max(0.0);
                        if w > 0.0 {
                            *targets.entry(name).or_default() += w;
                        }
                    }
                }
            }
        }

        let synced = |n: &str| MOVE.contains(&n) || n == CROUCH_WALK || COMBAT_WALK.iter().any(|(_, m)| *m == n);
        rig.drive(&character.0, &targets, &synced, dt, &mut anim);
    }
}

impl Rig {
    /// A socket's world transform (its bone as last posed, times the socket's offset).
    pub fn socket(&self, info: &CharacterInfo, name: &str, globals: &Query<&GlobalTransform>) -> Option<Transform> {
        let s = info.sockets.get(name)?;
        let bone = globals.get(*self.bones.get(&s.attach)?).ok()?;
        let offset = s.transform.as_ref().map(crate::to_transform).unwrap_or_default();
        Some(bone.mul_transform(offset).compute_transform())
    }

    /// Starts, continues or ends the full-body action.
    pub fn set_action(&mut self, action: Option<&str>, dt: f32) {
        match (&mut self.action, action) {
            (Some((name, at)), Some(a)) if name == a => *at += dt,
            (_, Some(a)) => self.action = Some((a.to_string(), 0.0)),
            (_, None) => self.action = None,
        }
    }

    /// Eases every animation's weight toward `targets` (the action, when one plays, gets all of it), advances the
    /// shared locomotion phase of the `synced` cycles, and poses the player.
    pub fn drive(&mut self, model: &CharacterModel, targets: &HashMap<&str, f32>, synced: &dyn Fn(&str) -> bool, dt: f32, anim: &mut AnimationPlayer) {
        let action_target: HashMap<&str, f32>;
        let targets = match &self.action {
            Some((a, _)) if model.clips.contains_key(a.as_str()) => {
                action_target = HashMap::from([(a.as_str(), 1.0)]);
                &action_target
            }
            _ => targets,
        };
        // Ease every weight toward its target (absent = 0).
        let rate = if self.action.is_some() { MONTAGE_BLEND_RATE } else { BLEND_RATE } * dt;
        let mut names: Vec<String> = self.weights.keys().cloned().collect();
        for k in targets.keys() {
            if !self.weights.contains_key(*k) {
                names.push(k.to_string());
            }
        }
        for name in names {
            let target = targets.get(name.as_str()).copied().unwrap_or(0.0);
            let w = self.weights.entry(name.clone()).or_insert(0.0);
            *w += (target - *w).clamp(-rate, rate);
            if *w <= 1e-4 && target == 0.0 {
                self.weights.remove(&name);
            }
        }

        // Locomotion cycles share one normalised phase, advanced by their weighted cycle length.
        let (mut wsum, mut lsum) = (0.0, 0.0);
        for (n, w) in &self.weights {
            if let (true, Some((_, len))) = (synced(n), model.clips.get(n)) {
                wsum += w;
                lsum += w * len;
            }
        }
        if wsum > 0.0 && lsum > 0.0 {
            self.phase = (self.phase + dt * wsum / lsum).fract();
        }
        self.idle_time += dt;

        for (name, &(node, len)) in &model.clips {
            let w = self.weights.get(name).copied().unwrap_or(0.0);
            if w <= 1e-4 {
                if anim.is_playing_animation(node) {
                    anim.stop(node);
                }
                continue;
            }
            let at = if self.action.as_ref().is_some_and(|(a, _)| a == name) {
                // Loop sections (a stun's middle) repeat; the rest hold their last frame.
                let t = self.action.as_ref().unwrap().1;
                if name.contains("_loop") || name.contains("_cycle") { t % len.max(1e-3) } else { t.min(len) }
            } else if synced(name) {
                self.phase * len
            } else {
                self.idle_time % len.max(1e-3)
            };
            anim.play(node).set_speed(0.0).set_weight(w).set_seek_time(at);
        }
    }
}

/// Position of `speed` along the blendspace's speed marks (0 at marks[0], 1 at marks[1], ...).
fn speed_state(speed: f32, marks: &[f32]) -> f32 {
    for i in 1..marks.len() {
        if speed <= marks[i] {
            let span = (marks[i] - marks[i - 1]).max(1e-3);
            return (i - 1) as f32 + ((speed - marks[i - 1]) / span).clamp(0.0, 1.0);
        }
    }
    (marks.len() - 1) as f32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn speed_state_is_piecewise_linear() {
        let marks = [0.0, 100.0, 300.0, 500.0];
        assert_eq!(speed_state(0.0, &marks), 0.0);
        assert_eq!(speed_state(50.0, &marks), 0.5);
        assert_eq!(speed_state(200.0, &marks), 1.5);
        assert_eq!(speed_state(500.0, &marks), 3.0);
        assert_eq!(speed_state(900.0, &marks), 3.0);
    }

    #[test]
    fn every_montage_the_rules_play_has_a_female_take() {
        for m in ["AM_combat_stand_Dodge_Left", "AM_combat_stand_Dodge_Right", "AM_combat_stand_dodge_back", "AM_normal_stand_run_stumble_short", "AM_normal_stand_sprint_stumble_short"] {
            assert!(female_sequence(m).is_some(), "{m}");
        }
    }
}
