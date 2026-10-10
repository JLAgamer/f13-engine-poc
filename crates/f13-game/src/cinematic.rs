//! The match intro (and outro) level sequences (F13-decomp MODLOG "Level intro"; `f13-export sequences`): the game
//! mode's PreMatchIntro has SCWorldSettings play the killer's IntroSequences entry (J2: CLSM_Intro_Master_JTS) on its
//! intro level (CLsm_IntroCinematic, staged off the map), and the match starts on the sequence's
//! IntroSequenceCompleted event. This is a small sequence player for what those sequences use (UE 4.18 MovieScene,
//! times in seconds):
//! - the master's cinematic shots, each a sub-sequence from its start offset;
//! - camera cuts to the staged CineCameraActors, with their transform and CurrentFocalLength tracks (the field of view
//!   from the camera's filmback: its exported FieldOfView at its CurrentFocalLength gives the sensor width);
//! - 3D transform tracks (translation, rotation as roll/pitch/yaw, scale; a channel without keys keeps its default,
//!   else the actor's placed value; outside its sections, the nearest one);
//! - skeletal animation sections (looping the take from the section's start), posed on the actor's joints;
//! - visibility (bHidden), lights' Intensity, the fade (black), the audio sections, the events.
//! The level Blueprint's own setup: the killer stand-in (Jason_TS, Jts_Mask) wears the picked killer's default body and
//! mask (GetKillerDefaultMeshAndMask: J2's). Actors attached to another's socket or bone follow it.
//! While it plays the match's own clock is held (virtual time paused): it runs on real time at the game's speed.
//! - particle tracks: their keys (EParticleKey: Activate 0, Deactivate 1, Trigger 2) as playback crosses them, through
//!   the shots' time mapping, activate (from the start, when it isn't active) or deactivate the bound Emitter actor's
//!   systems (particles.rs ParticleActivation). PLACEHOLDER: from UE 4.18's particle track template (engine source,
//!   not the game's code); Trigger isn't used by these sequences.
//! - the TextRenderActor's text (its TextRenderComponent, [`title_text`]) and its component material track's
//!   Transparency (Text_Cinematics: masked at (distance field x 180 - 90) x Transparency >= 0.3333).
//! - an actor attached to another's root takes its transform track as its offset from it.
//! - lights' Temperature (float) and LightColor (colour: its linear curves, as UE sets an FColor property from them,
//!   ToFColor(sRGB)).
//! - the cine cameras' Circle DOF (circle_dof.rs): CurrentAperture and the manual focus distance, their tracks over
//!   their placed values, the filmback's width as above.
//! Not played: the Fear track.

use crate::lighting::{LightKind, PlacedLight};
use bevy::prelude::*;
use bevy::world_serialization::WorldAssetRoot;
use f13_core::camera_spline::{quaternion, rotator};
use f13_data::character::{ue_rich_curve, AnimData, AnimInfo};
use f13_data::level::Xform;
use serde_json::Value;
use std::collections::HashMap;

pub struct CinematicPlugin;

impl Plugin for CinematicPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<CinematicDone>()
            .add_systems(Update, (start, ready, attach, title_text).chain())
            .add_systems(PostUpdate, play.before(bevy::transform::TransformSystems::Propagate));
    }
}

/// The intro: its master sequence and the level it stages, and the event that ends it.
pub const INTRO: Cinematic = Cinematic { master: "CLSM_Intro_Master_JTS", level: "CLsm_IntroCinematic", end_event: Some("IntroSequenceCompleted") };
/// The outro (PostMatchOutro, SCWorldSettings FUN_140445ef0: only with the killer alive): it plays to its end.
pub const OUTRO: Cinematic = Cinematic { master: "CLSM_Outro_JTS", level: "CLsm_OutroCinematic", end_event: None };

#[derive(Clone, Copy)]
pub struct Cinematic {
    pub master: &'static str,
    pub level: &'static str,
    /// The event that ends it, else its playback range's end.
    pub end_event: Option<&'static str>,
}

/// The intro and outro play (not under a bot, nor with --no-intro, nor without their export).
#[derive(Resource)]
pub struct IntroEnabled;

/// Play this one (match_flow, as the match enters PreMatchIntro).
#[derive(Resource)]
pub struct PlayCinematic(pub Cinematic);

/// Set as the playing one's end event fires.
#[derive(Resource, Default)]
pub struct CinematicDone(pub bool);

/// Whether the intro's data is in the export (else the match starts at once).
pub fn available(export: &std::path::Path, c: &Cinematic) -> bool {
    export.join("sequences").join(format!("{}.json", c.master)).exists() && export.join("levels").join(format!("{}.json", c.level)).exists()
}

// ---- The sequences.

struct Section {
    start: f32,
    end: f32,
    props: Value,
}

#[derive(PartialEq)]
enum Kind {
    Transform,
    Visibility,
    Float(String),
    Skeletal,
    CameraCut,
    Shot,
    Fade,
    Audio,
    Event,
    Particle,
    ComponentMaterial,
    Color(String),
    Other,
}

struct Track {
    binding: Option<String>,
    kind: Kind,
    sections: Vec<Section>,
}

struct Binding {
    name: String,
    object: Option<String>,
    parent: Option<String>,
}

struct Sequence {
    end: f32,
    bindings: HashMap<String, Binding>,
    tracks: Vec<Track>,
}

impl Sequence {
    fn load(path: &std::path::Path) -> Option<Self> {
        let v: Value = serde_json::from_str(&std::fs::read_to_string(path).ok()?).ok()?;
        let bindings = v["Bindings"]
            .as_array()?
            .iter()
            .map(|b| {
                let s = |k: &str| b[k].as_str().map(str::to_string);
                // The level object's actor: "...PersistentLevel.<actor>".
                let object = s("Object").map(|o| o.rsplit('.').next().unwrap_or("").to_string());
                (s("Guid").unwrap_or_default(), Binding { name: s("Name").unwrap_or_default(), object, parent: s("Parent") })
            })
            .collect();
        let tracks = v["Tracks"]
            .as_array()?
            .iter()
            .map(|t| {
                let kind = match t["Type"].as_str().unwrap_or("") {
                    "MovieScene3DTransformTrack" => Kind::Transform,
                    "MovieSceneVisibilityTrack" => Kind::Visibility,
                    "MovieSceneFloatTrack" => Kind::Float(t["Props"]["PropertyName"].as_str().unwrap_or("").to_string()),
                    "MovieSceneSkeletalAnimationTrack" => Kind::Skeletal,
                    "MovieSceneCameraCutTrack" => Kind::CameraCut,
                    "MovieSceneCinematicShotTrack" => Kind::Shot,
                    "MovieSceneFadeTrack" => Kind::Fade,
                    "MovieSceneAudioTrack" => Kind::Audio,
                    "MovieSceneEventTrack" => Kind::Event,
                    "MovieSceneParticleTrack" => Kind::Particle,
                    "MovieSceneComponentMaterialTrack" => Kind::ComponentMaterial,
                    "MovieSceneColorTrack" => Kind::Color(t["Props"]["PropertyName"].as_str().unwrap_or("").to_string()),
                    _ => Kind::Other,
                };
                let sections = t["Sections"]
                    .as_array()
                    .map(|s| {
                        s.iter()
                            .map(|s| {
                                let p = &s["Props"];
                                let infinite = p["bIsInfinite"].as_bool().unwrap_or(false);
                                let f = |k: &str, or: f32| p[k].as_f64().map_or(or, |v| v as f32);
                                Section {
                                    start: if infinite { f32::NEG_INFINITY } else { f("StartTime", 0.0) },
                                    end: if infinite { f32::INFINITY } else { f("EndTime", f32::INFINITY) },
                                    props: p.clone(),
                                }
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                Track { binding: t["Binding"].as_str().map(str::to_string), kind, sections }
            })
            .collect();
        Some(Sequence { end: v["End"].as_f64().unwrap_or(0.0) as f32, bindings, tracks })
    }

    /// The actor (and component, for a component binding) a binding drives.
    fn target(&self, guid: &str) -> Option<(String, Option<String>)> {
        let b = self.bindings.get(guid)?;
        match &b.parent {
            Some(p) => Some((self.bindings.get(p)?.object.clone()?, Some(b.name.clone()))),
            None => Some((b.object.clone()?, None)),
        }
    }
}

/// The section active at `t`, else (when `nearest`) the closest one.
fn section_at(sections: &[Section], t: f32, nearest: bool) -> Option<&Section> {
    if let Some(s) = sections.iter().rev().find(|s| s.start <= t && t <= s.end) {
        return Some(s);
    }
    if !nearest {
        return None;
    }
    sections.iter().min_by(|a, b| {
        let d = |s: &Section| if t < s.start { s.start - t } else { t - s.end };
        d(a).total_cmp(&d(b))
    })
}

/// A curve's value: its keys, else its default value, else `or`.
fn channel(v: &Value, t: f32, or: f32) -> f32 {
    if v["Keys"].as_array().is_some_and(|k| !k.is_empty()) {
        return ue_rich_curve(v).eval(t);
    }
    v["DefaultValue"].as_f64().map_or(or, |d| d as f32)
}

/// What a sequence sets at a moment.
#[derive(Default)]
struct Frame {
    transforms: HashMap<String, Xform>,
    hidden: HashMap<String, bool>,
    anims: HashMap<String, (String, f32)>,
    floats: HashMap<(String, Option<String>, String), f32>,
    colors: HashMap<(String, Option<String>, String), [f32; 4]>,
    camera: Option<String>,
    fade: Option<f32>,
}

fn evaluate(seqs: &HashMap<String, Sequence>, name: &str, t: f32, base: &HashMap<String, Xform>, lengths: &HashMap<(String, String), f32>, out: &mut Frame) {
    let Some(seq) = seqs.get(name) else { return };
    for track in &seq.tracks {
        let target = track.binding.as_deref().and_then(|g| seq.target(g));
        match &track.kind {
            Kind::Transform => {
                let (Some((actor, _)), Some(s)) = (target, section_at(&track.sections, t, true)) else { continue };
                let b = base.get(&actor).cloned().unwrap_or(Xform::IDENTITY);
                let r = rotator(Quat::from_array(b.r));
                let p = &s.props;
                let tr = [channel(&p["Translation"], t, b.t[0]), channel(&p["Translation[1]"], t, b.t[1]), channel(&p["Translation[2]"], t, b.t[2])];
                // Rotation X, Y, Z: roll, pitch, yaw.
                let (roll, pitch, yaw) = (channel(&p["Rotation"], t, r.z), channel(&p["Rotation[1]"], t, r.x), channel(&p["Rotation[2]"], t, r.y));
                let sc = [channel(&p["Scale"], t, b.s[0]), channel(&p["Scale[1]"], t, b.s[1]), channel(&p["Scale[2]"], t, b.s[2])];
                out.transforms.insert(actor, Xform { t: tr, r: quaternion(Vec3::new(pitch, yaw, roll)).to_array(), s: sc });
            }
            Kind::Visibility => {
                let (Some((actor, _)), Some(s)) = (target, section_at(&track.sections, t, true)) else { continue };
                // A step curve of bHidden.
                let keys = s.props["BoolCurve"]["Keys"].as_array().cloned().unwrap_or_default();
                let v = keys.iter().take_while(|k| k["Time"].as_f64().unwrap_or(0.0) as f32 <= t).last().or(keys.first()).and_then(|k| k["Value"].as_i64());
                let v = v.or(s.props["BoolCurve"]["DefaultValue"].as_i64()).unwrap_or(0);
                out.hidden.insert(actor, v != 0);
            }
            Kind::ComponentMaterial => {
                // Its scalar parameters, by name (a curve without keys and the float max default leaves the material's).
                let (Some((actor, component)), Some(s)) = (target, section_at(&track.sections, t, true)) else { continue };
                for c in s.props["ScalarParameterNamesAndCurves"].as_array().into_iter().flatten() {
                    let Some(name) = c["ParameterName"].as_str() else { continue };
                    let curve = &c["ParameterCurve"];
                    if curve["Keys"].as_array().is_some_and(|k| !k.is_empty()) {
                        out.floats.insert((actor.clone(), component.clone(), name.to_string()), channel(curve, t, 0.0));
                    }
                }
            }
            Kind::Color(prop) => {
                let (Some((actor, component)), Some(s)) = (target, section_at(&track.sections, t, true)) else { continue };
                let c = |k: &str| channel(&s.props[k], t, 1.0);
                out.colors.insert((actor, component, prop.clone()), [c("RedCurve"), c("GreenCurve"), c("BlueCurve"), c("AlphaCurve")]);
            }
            Kind::Float(prop) => {
                let (Some((actor, component)), Some(s)) = (target, section_at(&track.sections, t, true)) else { continue };
                out.floats.insert((actor, component, prop.clone()), channel(&s.props["FloatCurve"], t, 0.0));
            }
            Kind::Skeletal => {
                let Some((actor, _)) = target else { continue };
                // The legacy index blend: the last active section wins.
                let Some(s) = track.sections.iter().rev().find(|s| s.start <= t && t <= s.end) else { continue };
                let Some(take) = s.props["Params"]["Animation"]["ObjectPath"].as_str().and_then(|p| p.rsplit('/').next()).map(|p| p.split('.').next().unwrap_or(p).to_string()) else {
                    continue;
                };
                let rate = s.props["Params"]["PlayRate"].as_f64().unwrap_or(1.0) as f32;
                let offset = s.props["Params"]["StartOffset"].as_f64().unwrap_or(0.0) as f32;
                let len = lengths.get(&(actor.clone(), take.clone())).copied().unwrap_or(0.0);
                let local = (t - s.start.max(0.0)) * rate;
                let at = if len > 0.0 { local.rem_euclid(len) + offset } else { offset };
                out.anims.insert(actor, (take, at));
            }
            Kind::CameraCut => {
                if let Some(s) = section_at(&track.sections, t, false) {
                    if let Some(camera) = s.props["CameraGuid"].as_str().and_then(|g| seq.target(g)) {
                        out.camera = Some(camera.0);
                    }
                }
            }
            Kind::Shot => {
                let Some(s) = section_at(&track.sections, t, false) else { continue };
                let Some(sub) = s.props["SubSequence"]["ObjectName"].as_str().and_then(|n| n.split('\'').nth(1)) else { continue };
                let offset = s.props["Parameters"]["StartOffset"].as_f64().unwrap_or(0.0) as f32;
                let scale = s.props["Parameters"]["TimeScale"].as_f64().unwrap_or(1.0) as f32;
                evaluate(seqs, sub, (t - s.start.max(0.0)) * scale + offset, base, lengths, out);
            }
            Kind::Fade => {
                if let Some(s) = section_at(&track.sections, t, true) {
                    out.fade = Some(channel(&s.props["FloatCurve"], t, 0.0));
                }
            }
            _ => {}
        }
    }
}

/// The master's audio sections that start, and its events that fire, in (from, to].
fn crossings(seq: &Sequence, from: f32, to: f32) -> (Vec<String>, Vec<String>) {
    let (mut sounds, mut events) = (vec![], vec![]);
    for track in &seq.tracks {
        for s in &track.sections {
            match track.kind {
                Kind::Audio if from < s.start.max(0.0) && s.start.max(0.0) <= to || from < 0.0 && s.start <= 0.0 && to >= 0.0 => {
                    if let Some(name) = s.props["Sound"]["ObjectName"].as_str().and_then(|n| n.split('\'').nth(1)) {
                        sounds.push(name.to_string());
                    }
                }
                Kind::Event => {
                    let times = s.props["EventData"]["KeyTimes"].as_array().cloned().unwrap_or_default();
                    for (i, k) in times.iter().enumerate() {
                        let k = k.as_f64().unwrap_or(0.0) as f32;
                        if from < k && k <= to {
                            if let Some(e) = s.props["EventData"]["KeyValues"][i]["EventName"].as_str() {
                                events.push(e.to_string());
                            }
                        }
                    }
                }
                _ => {}
            }
        }
    }
    (sounds, events)
}

/// The particle keys crossed in (from, to] of a sequence, through its shots (each mapped into its sub-sequence's
/// time): the bound actor and the key (0 Activate, 1 Deactivate, 2 Trigger).
fn particle_crossings(seqs: &HashMap<String, Sequence>, name: &str, from: f32, to: f32, out: &mut Vec<(String, i64)>) {
    let Some(seq) = seqs.get(name) else { return };
    for track in &seq.tracks {
        match track.kind {
            Kind::Particle => {
                let Some((actor, _)) = track.binding.as_deref().and_then(|g| seq.target(g)) else { continue };
                for s in &track.sections {
                    for k in s.props["ParticleKeys"]["Keys"].as_array().into_iter().flatten() {
                        let at = k["Time"].as_f64().unwrap_or(0.0) as f32;
                        if from < at && at <= to && s.start <= at && at <= s.end {
                            out.push((actor.clone(), k["Value"].as_i64().unwrap_or(0)));
                        }
                    }
                }
            }
            Kind::Shot => {
                for s in &track.sections {
                    if to < s.start || from >= s.end {
                        continue;
                    }
                    let Some(sub) = s.props["SubSequence"]["ObjectName"].as_str().and_then(|n| n.split('\'').nth(1)) else { continue };
                    let offset = s.props["Parameters"]["StartOffset"].as_f64().unwrap_or(0.0) as f32;
                    let scale = s.props["Parameters"]["TimeScale"].as_f64().unwrap_or(1.0) as f32;
                    let local = |t: f32| (t - s.start.max(0.0)) * scale + offset;
                    // Entering the shot: from just before its first frame.
                    let sub_from = if from < s.start { local(s.start) - 1e-4 } else { local(from) };
                    particle_crossings(seqs, sub, sub_from, local(to.min(s.end)), out);
                }
            }
            _ => {}
        }
    }
}

// ---- Playing.

/// A staged actor.
#[derive(Component)]
struct Staged {
    name: String,
}

/// An actor attached to another's bone (or, with no socket, its root), and its offset from it (Bevy space, at the
/// bind pose).
#[derive(Component)]
struct Attached {
    parent: String,
    bone: Option<String>,
    rel: Option<Transform>,
}

/// The entity an attachment follows: the parent's bone, or the parent itself.
fn anchor(parent: Entity, bone: &Option<String>, children: &Query<&Children>, names: &Query<&Name>) -> Option<Entity> {
    match bone {
        // The deepest node of that name: the joint (a glTF scene can also have a wrapper node called "root").
        Some(b) => children.iter_descendants(parent).filter(|&e| names.get(e).is_ok_and(|n| n.as_str() == b)).last(),
        None => Some(parent),
    }
}

/// A staged light and its placed properties.
#[derive(Component)]
struct StagedLight(PlacedLight);

/// The fade's black overlay.
#[derive(Component)]
struct FadeOverlay;

#[derive(Resource)]
struct Playing {
    cinematic: Cinematic,
    sequences: HashMap<String, Sequence>,
    t: f32,
    started: bool,
    base: HashMap<String, Xform>,
    actors: HashMap<String, Entity>,
    /// (actor, take) -> its samples and length.
    takes: HashMap<(String, String), (AnimData, f32)>,
    lengths: HashMap<(String, String), f32>,
    /// Camera actor -> (field of view at its focal length, that focal length).
    /// A cine camera's placed FieldOfView, CurrentFocalLength, CurrentAperture and ManualFocusDistance.
    cameras: HashMap<String, (f32, f32, f32, f32)>,
    frames_waited: u32,
    /// The frame every attachment's bone was first there.
    found_at: Option<u32>,
    /// The HUD's top-level nodes and their visibility, hidden while it plays.
    hud: Vec<(Entity, Visibility)>,
}

/// The level and sequences loaded, the actors spawned; virtual time paused.
fn start(
    mut commands: Commands,
    request: Option<Res<PlayCinematic>>,
    playing: Option<Res<Playing>>,
    export: Res<crate::ExportRoot>,
    assets: Res<AssetServer>,
    mut virtual_time: ResMut<Time<Virtual>>,
    mut done: ResMut<CinematicDone>,
    mut ui_roots: Query<(Entity, &mut Visibility), (With<Node>, Without<ChildOf>)>,
) {
    let (Some(request), None) = (request, playing) else { return };
    let c = request.0;
    commands.remove_resource::<PlayCinematic>();
    done.0 = false;
    let root = &export.0;
    let level: Value = match std::fs::read_to_string(root.join("levels").join(format!("{}.json", c.level))).ok().and_then(|t| serde_json::from_str(&t).ok()) {
        Some(v) => v,
        None => {
            warn!("cinematic: no levels/{}.json (f13-export sequences)", c.level);
            done.0 = true;
            return;
        }
    };
    let mut sequences = HashMap::new();
    for f in std::fs::read_dir(root.join("sequences")).into_iter().flatten().flatten() {
        let path = f.path();
        if let (Some(name), Some(seq)) = (path.file_stem().and_then(|s| s.to_str()).map(str::to_string), Sequence::load(&path)) {
            sequences.insert(name, seq);
        }
    }
    if !sequences.contains_key(c.master) {
        warn!("cinematic: no sequences/{}.json", c.master);
        done.0 = true;
        return;
    }
    // The killer stand-in's body and mask: J2's (GetKillerDefaultMeshAndMask for the picked killer).
    let killer: Option<Value> = std::fs::read_to_string(root.join("killers").join("Jason_SackHead.json")).ok().and_then(|t| serde_json::from_str(&t).ok());
    let killer_body = killer.as_ref().and_then(|k| k["Body"].as_str().map(str::to_string));
    let killer_mask = killer.as_ref().and_then(|k| k["Parts"].as_array()?.iter().find(|p| p["Name"] == "Mask")?["File"].as_str().map(str::to_string));

    let mut base = HashMap::new();
    let mut actors = HashMap::new();
    let mut cameras = HashMap::new();
    let mut takes = HashMap::new();
    let mut lengths = HashMap::new();
    let scene = |file: &str| WorldAssetRoot(assets.load(GltfAssetLabel::Scene(0).from_asset(file.to_string())));
    // Static meshes: the level's placements, by their component's actor.
    let mut placements: HashMap<String, Vec<(String, Xform)>> = HashMap::new();
    for p in level["Placements"].as_array().into_iter().flatten() {
        let actor = p["Component"].as_str().and_then(|c| c.split("PersistentLevel.").nth(1)).and_then(|c| c.split('.').next()).unwrap_or("").to_string();
        if let (Some(file), Ok(xf)) = (p["File"].as_str(), serde_json::from_value::<Xform>(p["Transform"].clone())) {
            placements.entry(actor).or_default().push((file.to_string(), xf));
        }
    }
    for a in level["Actors"].as_array().into_iter().flatten() {
        let name = a["Name"].as_str().unwrap_or("").to_string();
        let class = a["Class"].as_str().unwrap_or("");
        let Ok(xf) = serde_json::from_value::<Xform>(a["Transform"].clone()) else { continue };
        base.insert(name.clone(), xf);
        let comps = a["Components"].as_array().cloned().unwrap_or_default();
        let mut e = commands.spawn((Staged { name: name.clone() }, Name::new(format!("cinematic {name}")), crate::to_transform(&xf), Visibility::default()));
        let mesh = comps.iter().find_map(|c| c["Mesh"].as_str()).map(str::to_string);
        // The killer stand-in wears the picked killer's body and mask.
        let mesh = if name.starts_with("Jason_TS") { killer_body.clone().or(mesh) } else if name.starts_with("Jts_Mask") { killer_mask.clone().or(mesh) } else { mesh };
        if let Some(m) = mesh {
            e.insert(scene(&m));
        } else if let Some(list) = placements.get(&name) {
            if let Some((file, _)) = list.first() {
                e.insert(scene(file));
            }
        }
        if matches!(class, "SpotLight" | "PointLight") {
            if let Some(c) = comps.iter().find(|c| c["Type"].as_str().is_some_and(|t| t.ends_with("LightComponent"))) {
                let kind = if class == "SpotLight" { LightKind::Spot } else { LightKind::Point };
                let props = c["Props"].as_object().cloned().unwrap_or_default();
                let l = PlacedLight { kind, world: xf.clone(), props, powered: false };
                let color: Color = l.color().into();
                let (lumens, range) = (l.lumens(), l.radius() / 100.0);
                if kind == LightKind::Spot {
                    let (inner, outer) = l.cone();
                    let outer = outer.clamp(0.5, 89.0).to_radians();
                    e.insert(SpotLight { color, intensity: lumens, range, inner_angle: inner.to_radians().min(outer), outer_angle: outer, shadow_maps_enabled: false, ..default() });
                } else {
                    e.insert(PointLight { color, intensity: lumens, range, shadow_maps_enabled: false, ..default() });
                }
                e.insert(StagedLight(l));
            }
        }
        if class == "TextRenderActor" {
            if let Some(c) = comps.iter().find(|c| c["Type"] == "TextRenderComponent") {
                let p = &c["Props"];
                let font: Option<Value> = p["Font"]
                    .as_str()
                    .and_then(|f| f.split('\'').nth(1))
                    .and_then(|f| std::fs::read_to_string(root.join("fonts").join(format!("{f}.json"))).ok())
                    .and_then(|t| serde_json::from_str(&t).ok());
                if let Some(font) = font {
                    let page = font["Pages"][0].as_str().unwrap_or("").to_string();
                    let image = assets.load_with_settings(format!("{page}.png"), |s: &mut bevy::image::ImageLoaderSettings| s.is_srgb = false);
                    let col = &p["TextRenderColor"];
                    let byte = |k: &str, d: f64| col[k].as_f64().unwrap_or(d) as f32 / 255.0;
                    e.insert(PendingTitle {
                        text: p["Text"]["SourceString"].as_str().unwrap_or("").to_string(),
                        font,
                        image,
                        // FColor vertex colour, read as unorm (no sRGB decode).
                        color: LinearRgba::new(byte("R", 255.0), byte("G", 255.0), byte("B", 255.0), 1.0),
                        center: match p["HorizontalAlignment"].as_str() {
                            Some("EHTA_Center") => 1,
                            Some("EHTA_Right") => 2,
                            _ => 0,
                        },
                    });
                }
            }
        }
        if class == "CineCameraActor" {
            if let Some(c) = comps.iter().find(|c| c["Type"] == "CineCameraComponent") {
                let p = &c["Props"];
                let f = |v: &Value, d: f64| v.as_f64().unwrap_or(d) as f32;
                // The constructor's ManualFocusDistance 100000 when it isn't set.
                cameras.insert(name.clone(), (f(&p["FieldOfView"], 90.0), f(&p["CurrentFocalLength"], 35.0), f(&p["CurrentAperture"], 2.0), f(&p["FocusSettings"]["ManualFocusDistance"], 100000.0)));
            }
        }
        if let Some(parent) = a["AttachParent"].as_str() {
            // A socket on the parent's mesh (its bone), a bone itself, or (none) the parent's root.
            let parent_comps = level["Actors"].as_array().and_then(|l| l.iter().find(|p| p["Name"] == parent)).map(|p| p["Components"].clone()).unwrap_or(Value::Null);
            let bone = a["AttachSocket"].as_str().map(|socket| {
                parent_comps
                    .as_array()
                    .and_then(|cs| cs.iter().find_map(|c| c["SocketBones"][socket]["Bone"].as_str()))
                    .unwrap_or(socket)
                    .to_string()
            });
            e.insert(Attached { parent: parent.to_string(), bone, rel: None });
        }
        actors.insert(name.clone(), e.id());
        // Its takes (cinematic/<actor>.json).
        if let Some(index) = std::fs::read_to_string(root.join("cinematic").join(format!("{name}.json"))).ok().and_then(|t| serde_json::from_str::<HashMap<String, AnimInfo>>(&t).ok()) {
            for (take, info) in index {
                if let Ok(data) = AnimData::load(&root.join(&info.file)) {
                    lengths.insert((name.clone(), take.clone()), info.length);
                    takes.insert((name.clone(), take), (data, info.length));
                }
            }
        }
    }
    commands.spawn((
        FadeOverlay,
        Node { position_type: PositionType::Absolute, width: Val::Percent(100.0), height: Val::Percent(100.0), ..default() },
        BackgroundColor(Color::BLACK),
        GlobalZIndex(i32::MAX - 1),
    ));
    info!("cinematic: {} ({} actors, {} sequences)", c.master, actors.len(), sequences.len());
    virtual_time.pause();
    // No HUD over it.
    let mut hud = vec![];
    for (e, mut v) in &mut ui_roots {
        hud.push((e, *v));
        *v = Visibility::Hidden;
    }
    commands.insert_resource(Playing { cinematic: c, sequences, t: 0.0, started: false, base, actors, takes, lengths, cameras, frames_waited: 0, found_at: None, hud });
}

/// Starts once the staged meshes are in (their attached actors' offsets taken at the bind pose).
fn ready(
    mut playing: Option<ResMut<Playing>>,
    staged: Query<(Entity, &Staged)>,
    mut attached: Query<(&Staged, &mut Attached, &GlobalTransform)>,
    children: Query<&Children>,
    names: Query<&Name>,
    globals: Query<&GlobalTransform>,
) {
    let Some(p) = playing.as_mut() else { return };
    if p.started {
        return;
    }
    p.frames_waited += 1;
    // Every attachment's bone there (the scenes spawned)...
    let mut missing = vec![];
    for (s, a, _) in &attached {
        let parent = staged.iter().find(|(_, s)| s.name == a.parent).map(|(e, _)| e);
        if parent.and_then(|e| anchor(e, &a.bone, &children, &names)).is_none() {
            missing.push(format!("{} ({}.{:?})", s.name, a.parent, a.bone));
        }
    }
    if missing.is_empty() && p.found_at.is_none() {
        p.found_at = Some(p.frames_waited);
    }
    // ...and their world transforms propagated (a frame or two after they spawn): the offsets from the bind pose.
    let settled = p.found_at.is_some_and(|f| p.frames_waited >= f + 3);
    if !settled && p.frames_waited <= 600 {
        return;
    }
    if !missing.is_empty() {
        warn!("cinematic: attachments without their bone: {}", missing.join(", "));
    }
    for (_, mut a, gt) in &mut attached {
        let Some((parent, _)) = staged.iter().find(|(_, s)| s.name == a.parent) else { continue };
        if let Some(bg) = anchor(parent, &a.bone, &children, &names).and_then(|b| globals.get(b).ok()) {
            a.rel = Some(Transform::from_matrix((bg.affine().inverse() * gt.affine()).into()));
        }
    }
    p.started = true;
}

/// A TextRenderActor's text waiting for its font's page.
#[derive(Component)]
struct PendingTitle {
    text: String,
    font: Value,
    image: Handle<Image>,
    color: LinearRgba,
    /// HorizontalAlignment: 0 left, 1 center, 2 right.
    center: u8,
}

/// A staged text's material (its Transparency sets the mask's cutoff).
#[derive(Component)]
struct TitleText(Handle<StandardMaterial>);

/// The quads of UTextRenderComponent's string mesh (FTextRenderSceneProxy, FUN_1413a6bd0; its line size
/// FUN_1413a9990, the vertical offset FUN_1413a9440), in the actor's space (Bevy, m). The proxy's scale is XScale x
/// WorldSize x InvDefaultSize: the component's own (1, and the CDO's WorldSize and 1 / it: the constructor
/// 0x1413a1900 takes both from the default font's max char height), so a glyph is its atlas size in cm. Lines split at
/// "<br>" and newlines; each starts at -width / 2 (center) or -width (right); a glyph spans (x, y + VerticalOffset) to
/// + (USize, VSize), then x moves on by USize and, before another non-space character, (Kerning +
/// HorizSpacingAdjust); a line is its tallest (VSize + VertSpacingAdjust + VerticalOffset), else the font's max height.
/// UE places a vertex at (0, -x, -y), raised by the text's height for EVRTA_TextBottom (the default: FUN_1413a9440's
/// 2). PLACEHOLDER: the font's CharRemap (IsRemapped) isn't exported; the characters are taken by their code.
fn title_mesh(text: &str, font: &Value, size: Vec2, center: u8) -> Mesh {
    let chars: Vec<[f32; 6]> = font["Characters"]
        .as_array()
        .map(|a| a.iter().map(|c| std::array::from_fn(|i| c[i].as_f64().unwrap_or(0.0) as f32)).collect())
        .unwrap_or_default();
    let kerning = font["Kerning"].as_f64().unwrap_or(0.0) as f32;
    let max_height = chars.iter().map(|c| c[3]).fold(0.0, f32::max);
    let glyph = |ch: char| chars.get(ch as usize).filter(|c| c[4] == 0.0);
    let lines: Vec<Vec<char>> = text.split("<br>").flat_map(|l| l.split('\n')).map(|l| l.chars().collect()).collect();
    let advance = |line: &[char], i: usize, c: &[f32; 6]| c[2] + if line.get(i + 1).is_some_and(|n| !n.is_whitespace()) { kerning } else { 0.0 };
    let total: f32 = lines.iter().map(|l| l.iter().filter_map(|&c| glyph(c)).map(|c| c[3] + c[5]).fold(0.0, f32::max)).map(|h| if h > 0.0 { h } else { max_height }).sum();
    let (mut positions, mut uvs, mut normals, mut indices) = (vec![], vec![], vec![], vec![]);
    let mut y = 0.0;
    for line in &lines {
        let mut x = 0.0f32;
        let mut width = 0.0f32;
        for (i, &ch) in line.iter().enumerate() {
            if let Some(c) = glyph(ch) {
                width = width.max(x + c[2]);
                x += advance(line, i, c);
            }
        }
        let start = match center {
            1 => -width * 0.5,
            2 => -width,
            _ => 0.0,
        };
        let mut x = 0.0f32;
        let mut height = 0.0f32;
        for (i, &ch) in line.iter().enumerate() {
            let Some(c) = glyph(ch) else { continue };
            let (left, top) = (x + start, y + c[5]);
            let (right, bottom) = (left + c[2], top + c[3]);
            let (u0, v0, u1, v1) = (c[0] / size.x, c[1] / size.y, (c[0] + c[2]) / size.x, (c[1] + c[3]) / size.y);
            let base = positions.len() as u32;
            // UE (0, -x, total - y) cm to Bevy (x, z, y) m.
            for (px, py, u, v) in [(left, top, u0, v0), (right, top, u1, v0), (left, bottom, u0, v1), (right, bottom, u1, v1)] {
                positions.push([0.0, (total - py) * 0.01, -px * 0.01]);
                uvs.push([u, v]);
                normals.push([1.0, 0.0, 0.0]);
            }
            indices.extend_from_slice(&[base, base + 2, base + 1, base + 1, base + 2, base + 3]);
            height = height.max(c[3] + c[5]);
            x += advance(line, i, c);
        }
        y += if height > 0.0 { height } else { max_height };
    }
    Mesh::new(bevy::mesh::PrimitiveTopology::TriangleList, bevy::asset::RenderAssetUsages::default())
        .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, positions)
        .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, normals)
        .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, uvs)
        .with_inserted_indices(bevy::mesh::Indices::U32(indices))
}

/// Text_Cinematics' mask: (distance field x 180 - 90) x Transparency - 0.3333 >= 0, as a cutoff on the field.
fn title_cutoff(transparency: f32) -> f32 {
    if transparency > 0.0 {
        0.5 + 0.3333 / (180.0 * transparency)
    } else {
        2.0
    }
}

/// Builds a staged text once its font page is in: its glyphs' quads, and Text_Cinematics (masked, unlit, two-sided:
/// its emissive the component's colour, its mask the page's distance field) at the material's Transparency 1.
fn title_text(
    mut commands: Commands,
    pending: Query<(Entity, &PendingTitle)>,
    mut images: ResMut<Assets<Image>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    for (e, t) in &pending {
        let Some(src) = images.get(&t.image) else { continue };
        let (w, h) = (src.width(), src.height());
        // The field in alpha (the page is grey).
        let mut data = Vec::with_capacity((w * h * 4) as usize);
        for y in 0..h {
            for x in 0..w {
                let v = src.get_color_at(x, y).map(|c| c.to_linear().red).unwrap_or(0.0);
                data.extend_from_slice(&[255, 255, 255, (v.clamp(0.0, 1.0) * 255.0).round() as u8]);
            }
        }
        let field = images.add(Image::new(
            bevy::render::render_resource::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
            bevy::render::render_resource::TextureDimension::D2,
            data,
            bevy::render::render_resource::TextureFormat::Rgba8Unorm,
            bevy::asset::RenderAssetUsages::RENDER_WORLD,
        ));
        let mesh = meshes.add(title_mesh(&t.text, &t.font, Vec2::new(w as f32, h as f32), t.center));
        let material = materials.add(StandardMaterial {
            base_color: Color::BLACK,
            base_color_texture: Some(field),
            emissive: t.color,
            emissive_exposure_weight: 1.0,
            reflectance: 0.0,
            perceptual_roughness: 1.0,
            alpha_mode: AlphaMode::Mask(title_cutoff(1.0)),
            double_sided: true,
            cull_mode: None,
            ..default()
        });
        commands.entity(e).remove::<PendingTitle>().insert((Mesh3d(mesh), MeshMaterial3d(material.clone()), TitleText(material)));
    }
}

/// Attached actors follow their parent's bone.
fn attach(
    playing: Option<Res<Playing>>,
    mut attached: Query<(&Attached, &mut Transform)>,
    children: Query<&Children>,
    names: Query<&Name>,
    globals: Query<&GlobalTransform>,
) {
    let Some(p) = playing else { return };
    if !p.started {
        return;
    }
    for (a, mut xf) in &mut attached {
        let Some(rel) = a.rel else { continue };
        let Some(&parent) = p.actors.get(&a.parent) else { continue };
        let Some(bone) = anchor(parent, &a.bone, &children, &names) else { continue };
        if let Ok(bg) = globals.get(bone) {
            *xf = Transform::from_matrix((bg.affine() * rel.compute_affine()).into());
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn play(
    mut commands: Commands,
    real: Res<Time<Real>>,
    mut virtual_time: ResMut<Time<Virtual>>,
    playing: Option<ResMut<Playing>>,
    mut done: ResMut<CinematicDone>,
    mut transforms: Query<&mut Transform, Without<crate::player::PlayerCamera>>,
    mut visibility: Query<&mut Visibility>,
    mut lights: Query<(&StagedLight, Option<&mut SpotLight>, Option<&mut PointLight>)>,
    children: Query<&Children>,
    names: Query<&Name>,
    mut camera: Single<(Entity, &mut Transform, &mut Projection), With<crate::player::PlayerCamera>>,
    mut overlay: Query<(Entity, &mut BackgroundColor), With<FadeOverlay>>,
    staged: Query<Entity, With<Staged>>,
    windows: Query<&Window>,
    roots: Query<Entity, (With<Node>, Without<ChildOf>, Without<FadeOverlay>)>,
    (mut attached, titles, mut title_materials, frames): (Query<&mut Attached>, Query<&TitleText>, ResMut<Assets<StandardMaterial>>, Res<bevy::diagnostic::FrameCount>),
) {
    let Some(mut p) = playing else { return };
    // No HUD over it (its own systems may show parts of it again, or spawn new roots such as the minimap's: hidden
    // after them, every frame, and restored at the end).
    for e in &roots {
        if !p.hud.iter().any(|(h, _)| *h == e) {
            let v = visibility.get(e).map_or(Visibility::Inherited, |v| *v);
            p.hud.push((e, v));
        }
    }
    for (e, _) in &p.hud {
        if let Ok(mut v) = visibility.get_mut(*e) {
            v.set_if_neq(Visibility::Hidden);
        }
    }
    if !p.started {
        return;
    }
    let dt = real.delta_secs() * virtual_time.relative_speed();
    let from = p.t;
    p.t += dt;
    let t = p.t;
    let mut frame = Frame::default();
    evaluate(&p.sequences, p.cinematic.master, t, &p.base, &p.lengths, &mut frame);
    // Transforms (an attached actor's are relative to its parent: it keeps its offset from its bone) and visibility.
    for (actor, xf) in &frame.transforms {
        let Some(&e) = p.actors.get(actor) else { continue };
        match attached.get_mut(e) {
            // One on its parent's root: the track is its offset from it.
            Ok(mut a) => {
                if a.bone.is_none() {
                    a.rel = Some(crate::to_transform(xf));
                }
            }
            Err(_) => {
                if let Ok(mut t) = transforms.get_mut(e) {
                    *t = crate::to_transform(xf);
                }
            }
        }
    }
    // The texts' Transparency.
    for ((actor, _component, prop), v) in &frame.floats {
        if prop != "Transparency" {
            continue;
        }
        let Some(m) = p.actors.get(actor).and_then(|&e| titles.get(e).ok()) else { continue };
        if let Some(mut mat) = title_materials.get_mut(&m.0) {
            mat.alpha_mode = AlphaMode::Mask(title_cutoff(*v));
        }
    }
    for (actor, hidden) in &frame.hidden {
        if let Some(&e) = p.actors.get(actor) {
            if let Ok(mut v) = visibility.get_mut(e) {
                v.set_if_neq(if *hidden { Visibility::Hidden } else { Visibility::Inherited });
            }
        }
    }
    // Poses.
    for (actor, (take, at)) in &frame.anims {
        let (Some(&e), Some((data, length))) = (p.actors.get(actor), p.takes.get(&(actor.clone(), take.clone()))) else { continue };
        let frames = data.frames.max(1);
        let f = if *length > 0.0 { (at / length).clamp(0.0, 1.0) * (frames - 1) as f32 } else { 0.0 };
        let (i, a) = (f.floor() as usize, f.fract());
        let j = (i + 1).min(frames - 1);
        for bone in children.iter_descendants(e) {
            let Ok(name) = names.get(bone) else { continue };
            let Some(track) = data.tracks.iter().find(|t| t.bone == name.as_str()) else { continue };
            let Ok(mut xf) = transforms.get_mut(bone) else { continue };
            if let (Some(q0), Some(q1)) = (track.rotation.get(i), track.rotation.get(j)) {
                xf.rotation = Quat::from_array(*q0).normalize().slerp(Quat::from_array(*q1).normalize(), a);
            }
            if let Some((t0, t1)) = track.translation.as_ref().and_then(|t| Some((t.get(i)?, t.get(j)?))) {
                xf.translation = Vec3::from_array(*t0).lerp(Vec3::from_array(*t1), a);
            }
        }
    }
    // Lights: Intensity (their placed units), Temperature and LightColor, over their placed values.
    let mut light_props: HashMap<&String, Vec<(&str, Value)>> = HashMap::new();
    for ((actor, _component, prop), v) in &frame.floats {
        if matches!(prop.as_str(), "Intensity" | "Temperature") {
            light_props.entry(actor).or_default().push((prop.as_str(), Value::from(*v as f64)));
        }
    }
    for ((actor, _component, prop), c) in &frame.colors {
        if prop == "LightColor" {
            let s = Color::LinearRgba(LinearRgba::new(c[0], c[1], c[2], c[3])).to_srgba();
            let byte = |v: f32| Value::from((v.clamp(0.0, 1.0) * 255.0).round() as u64);
            light_props.entry(actor).or_default().push(("LightColor", serde_json::json!({ "R": byte(s.red), "G": byte(s.green), "B": byte(s.blue), "A": byte(s.alpha) })));
        }
    }
    for (actor, props) in light_props {
        let Some(&e) = p.actors.get(actor) else { continue };
        let Ok((l, spot, point)) = lights.get_mut(e) else { continue };
        let mut placed = l.0.clone();
        for (k, v) in props {
            placed.props.insert(k.into(), v);
        }
        let (lumens, color): (f32, Color) = (placed.lumens(), placed.color().into());
        if let Some(mut s) = spot {
            s.intensity = lumens;
            s.color = color;
        }
        if let Some(mut pl) = point {
            pl.intensity = lumens;
            pl.color = color;
        }
    }
    // The camera: the cut's CineCameraActor, its field of view by its focal length.
    if let Some(cam) = frame.camera.as_ref() {
        let xf = frame.transforms.get(cam).or_else(|| p.base.get(cam)).cloned();
        if let Some(xf) = xf {
            let world = crate::to_transform(&xf);
            let (camera_entity, ref mut cxf, ref mut proj) = *camera;
            cxf.translation = world.translation;
            cxf.rotation = world.rotation * Quat::from_rotation_y(-std::f32::consts::FRAC_PI_2);
            if let (Projection::Perspective(persp), Some(&(fov0, focal0, aperture0, focus0))) = (&mut **proj, p.cameras.get(cam)) {
                // The filmback's width from the placed field of view at the placed focal length.
                let sensor = 2.0 * focal0 * (fov0.to_radians() / 2.0).tan();
                let focal = frame.floats.get(&(cam.clone(), Some("CameraComponent".into()), "CurrentFocalLength".into())).copied().unwrap_or(focal0);
                let hfov = 2.0 * (sensor / (2.0 * focal.max(1.0))).atan();
                let aspect = windows.iter().next().map_or(16.0 / 9.0, |w| w.width() / w.height().max(1.0));
                persp.fov = 2.0 * ((hfov / 2.0).tan() / aspect).atan();
                let float = |prop: &str, or: f32| frame.floats.get(&(cam.clone(), Some("CameraComponent".into()), prop.into())).copied().unwrap_or(or);
                commands.entity(camera_entity).insert(crate::circle_dof::CircleDof {
                    focal_distance: float("ManualFocusDistance", focus0),
                    fstop: float("CurrentAperture", aperture0),
                    sensor_width: sensor,
                    hfov,
                    near: persp.near,
                    frame: frames.0,
                });
            }
        }
    }
    // The fade (to black).
    let fade = frame.fade.unwrap_or(0.0).clamp(0.0, 1.0);
    for (_, mut c) in &mut overlay {
        c.0 = Color::BLACK.with_alpha(fade);
    }
    // F13_CINE_SHOTS=t1,t2,...: screenshots at those intro times (temp f13-cine-<t>.png; checks).
    if let Ok(list) = std::env::var("F13_CINE_SHOTS") {
        for at in list.split(',').filter_map(|s| s.trim().parse::<f32>().ok()) {
            if from < at && at <= t {
                let path = std::env::temp_dir().join(format!("f13-cine-{at}.png"));
                info!("cinematic: screenshot {}", path.display());
                commands.spawn(bevy::render::view::screenshot::Screenshot::primary_window()).observe(bevy::render::view::screenshot::save_to_disk(path));
            }
        }
    }
    // Its sounds and events.
    let (sounds, events) = p.sequences.get(p.cinematic.master).map_or((vec![], vec![]), |s| crossings(s, if from == 0.0 { -1.0 } else { from }, t));
    for cue in sounds {
        commands.trigger(crate::audio::PlaySound { cue, at: None });
    }
    let mut keys = vec![];
    particle_crossings(&p.sequences, p.cinematic.master, if from == 0.0 { -1.0 } else { from }, t, &mut keys);
    for (actor, key) in keys {
        debug!("cinematic: {actor} particle key {key} at {t:.2} s");
        commands.write_message(crate::particles::ParticleActivation { actor, active: key != 1, reset: true });
    }
    let end = p.sequences.get(p.cinematic.master).map_or(0.0, |s| s.end);
    let finished = match p.cinematic.end_event {
        Some(event) => events.iter().any(|e| e == event),
        None => t >= end,
    };
    if finished {
        info!("cinematic: {} over at {:.2} s", p.cinematic.master, t);
        for e in &staged {
            commands.entity(e).despawn();
        }
        for (e, _) in &overlay {
            commands.entity(e).despawn();
        }
        for (e, v) in &p.hud {
            if let Ok(mut vis) = visibility.get_mut(*e) {
                *vis = *v;
            }
        }
        commands.entity(camera.0).remove::<crate::circle_dof::CircleDof>();
        commands.remove_resource::<Playing>();
        virtual_time.unpause();
        done.0 = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(start: f32, end: f32) -> Section {
        Section { start, end, props: Value::Null }
    }

    #[test]
    fn sections_active_then_nearest() {
        let list = [s(0.0, 2.0), s(3.0, 5.0)];
        assert_eq!(section_at(&list, 1.0, false).unwrap().start, 0.0);
        assert!(section_at(&list, 2.5, false).is_none());
        assert_eq!(section_at(&list, 2.9, true).unwrap().start, 3.0);
        assert_eq!(section_at(&list, 9.0, true).unwrap().start, 3.0);
    }

    #[test]
    fn a_channel_without_keys_keeps_its_default_or_the_placed_value() {
        assert_eq!(channel(&serde_json::json!({"DefaultValue": 5.0, "Keys": []}), 1.0, 2.0), 5.0);
        assert_eq!(channel(&Value::Null, 1.0, 2.0), 2.0);
    }
}
