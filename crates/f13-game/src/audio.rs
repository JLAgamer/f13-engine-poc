//! Sound: the game's SoundCues (f13-data::sound, exported by tools/f13-export sounds) played at a place, attenuated
//! as Unreal attenuates them (FAttenuationSettings: full volume inside the inner radius, then the falloff curve), and
//! panned left/right around the camera; the counselor's footsteps.
//!
//! Footsteps (SCAnimInstance's handler, slot 0x2f0): at each Footstep_L / Footstep_R notify of her locomotion animation,
//! the surface under her (a mesh's physical material, or the terrain's paint layer that weighs most there) picks the cue
//! from Base_Counselor_AnimBP's tables (female: Athlete; run when running or sprinting, else walk); a surface without
//! one takes the default surface's. Not yet: landing sounds, wading (in-water footsteps), Jason's footsteps.
//!
//! Actors play the cues their class names (`ClassSounds`): doors (OpenDoorSound as one starts to open,
//! CloseDoorSound as one starts to close, LockedDoorSound pressing a locked one, DoorHitSoundCue per blow,
//! DoorDestroySoundCue when it breaks), windows (WindowOpenSound / WindowCloseSound), barricades
//! (EnableBarricadeAudio / DisableBarricadeAudio).
//!
//! Jason's music (SCMusicComponent's tick on the counselor): his NearCounselorMusic (J2: JasonNearPart2) loops
//! unpanned; its volume heads for `1 - clamp((distance - NearCounselorMusicMaxVolumeDistance) /
//! NearCounselorMusicDistance, 0.001, 1)` (full within 3 m, gone 43 m out) at 0.5 per second (FInterpTo_Constant),
//! or for 0 while his music is silenced (Stalk). Not yet: the other music (tracks, stingers, warnings).
//!
//! PLACEHOLDERS: the listener is the camera; Delay and Concatenator nodes play their first child at once; actor sounds
//! play 1 m above the actor's origin (the game plays them from the actor or its audio components).

use bevy::asset::RenderAssetUsages;
use bevy::image::{CompressedImageFormats, ImageSampler, ImageType};
use bevy::audio::{AudioPlayer, PlaybackMode, PlaybackSettings, SpatialAudioSink, SpatialListener, SpatialScale, Volume};
use bevy::prelude::*;
use f13_data::sound::{Attenuation, Cue, Footsteps};
use std::collections::HashMap;
use std::path::PathBuf;

use crate::player::Player;

/// Positions are scaled down this much for rodio's spatial sink, so its own inverse-square falloff stays at 1 and only
/// panning remains (Unreal's attenuation is applied here).
const SPATIAL_SCALE: f32 = 0.001;
/// The listener's ear gap (m).
const EAR_GAP_M: f32 = 0.2;

pub struct AudioPlugin;

impl Plugin for AudioPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, load_sounds)
            .add_systems(Update, (add_listener, attenuate, actor_sounds, near_music))
            .add_observer(play_sound)
            .add_observer(footstep)
            .add_observer(actor_sound);
    }
}

/// Play a cue: at a place (attenuated and panned), or everywhere.
#[derive(Event, Clone, Debug)]
pub struct PlaySound {
    pub cue: String,
    pub at: Option<Vec3>,
}

/// The cues an actor's class names: property name -> cue.
#[derive(Component, Clone, Debug, Default)]
pub struct ClassSounds(pub HashMap<String, String>);

/// A class property's sound reference (`SoundCue'Name'` / `SoundWave'Name'` in classes.json): the cue's name.
pub fn cue_name(v: &serde_json::Value) -> Option<String> {
    let s = v.as_str()?;
    let rest = s.strip_prefix("SoundCue'").or_else(|| s.strip_prefix("SoundWave'"))?;
    Some(rest.trim_end_matches('\'').to_string())
}

/// Play the cue `actor`'s class names under `prop`, at the actor.
#[derive(Event, Clone, Copy, Debug)]
pub struct ActorSound {
    pub actor: Entity,
    pub prop: &'static str,
}

fn actor_sound(ev: On<ActorSound>, mut commands: Commands, actors: Query<(&ClassSounds, &GlobalTransform)>) {
    let Ok((s, g)) = actors.get(ev.actor) else { return };
    if let Some(cue) = s.0.get(ev.prop) {
        commands.trigger(PlaySound { cue: cue.clone(), at: Some(g.translation() + Vec3::Y) });
    }
}

/// Doors, windows and barricades sound when their state changes.
#[allow(clippy::type_complexity)]
fn actor_sounds(
    mut commands: Commands,
    mut last: Local<HashMap<Entity, (bool, f32, bool)>>,
    doors: Query<(Entity, &crate::doors::Door), With<ClassSounds>>,
    windows: Query<(Entity, &crate::windows::Window), With<ClassSounds>>,
    barricades: Query<(Entity, &crate::windows::Barricade), With<ClassSounds>>,
) {
    let mut play = |actor: Entity, prop: &'static str| commands.trigger(ActorSound { actor, prop });
    for (e, d) in &doors {
        let now = (d.open, d.health, d.broken);
        if let Some(&(open, health, broken)) = last.get(&e) {
            if d.broken && !broken {
                play(e, "DoorDestroySoundCue");
            } else if d.health < health {
                play(e, "DoorHitSoundCue");
            } else if d.open != open && !d.broken {
                play(e, if d.open { "OpenDoorSound" } else { "CloseDoorSound" });
            }
        }
        last.insert(e, now);
    }
    for (e, w) in &windows {
        if let Some(&(open, _, _)) = last.get(&e) {
            if w.open != open {
                play(e, if w.open { "WindowOpenSound" } else { "WindowCloseSound" });
            }
        }
        last.insert(e, (w.open, 0.0, w.destroyed));
    }
    for (e, b) in &barricades {
        if let Some(&(on, _, _)) = last.get(&e) {
            if b.enabled != on {
                play(e, if b.enabled { "EnableBarricadeAudio" } else { "DisableBarricadeAudio" });
            }
        }
        last.insert(e, (b.enabled, 0.0, false));
    }
}

/// Jason's music near the counselor: the looping player, its cue volume and its current volume.
#[derive(Default)]
struct NearMusic {
    player: Option<(Entity, f32)>,
    volume: f32,
}

#[allow(clippy::too_many_arguments)]
fn near_music(
    mut commands: Commands,
    time: Res<Time>,
    assets: Res<AssetServer>,
    mut music: Local<NearMusic>,
    sounds: Option<ResMut<Sounds>>,
    killer: Option<Res<crate::character::KillerCharacter>>,
    mut rng: ResMut<crate::GameRng>,
    jasons: Query<(&crate::jason::Jason, &GlobalTransform)>,
    players: Query<(&Player, &GlobalTransform)>,
    mut sinks: Query<&mut bevy::audio::AudioSink>,
) {
    let (Some(mut sounds), Some(killer)) = (sounds, killer) else { return };
    let Some(cue) = killer.info.sounds.get("NearCounselorMusic").and_then(|s| cue_name(&serde_json::Value::String(s.clone()))) else { return };
    let Some((jason, jxf)) = jasons.iter().next() else { return };
    let Ok((p, pxf)) = players.single() else { return };
    if music.player.is_none() {
        let Some(c) = sounds.cue(&cue).cloned() else { return };
        let (voices, _) = c.play(&mut || rng.0.frand());
        let Some(v) = voices.into_iter().next() else { return };
        let e = commands
            .spawn((AudioPlayer::new(assets.load(format!("sounds/{}", v.file))), PlaybackSettings { volume: Volume::Linear(0.0), speed: v.pitch, ..PlaybackSettings::LOOP }))
            .id();
        music.player = Some((e, v.volume));
    }
    let v = |k: &str, d: f32| killer.info.values.get(k).copied().unwrap_or(d);
    let distance = jxf.translation().distance(pxf.translation()) * 100.0;
    let silenced = jason.stalking() || !p.counselor.alive() || matches!(jason.state, crate::jason::JasonState::Done);
    let target = if silenced {
        0.0
    } else {
        1.0 - ((distance - v("NearCounselorMusicMaxVolumeDistance", 300.0)) / v("NearCounselorMusicDistance", 4000.0)).clamp(0.001, 1.0)
    };
    let step = 0.5 * time.delta_secs();
    music.volume += (target - music.volume).clamp(-step, step);
    trace!("near music: {:.0} m, target {target:.2}, volume {:.2}", distance / 100.0, music.volume);
    if let Some((e, base)) = music.player {
        if let Ok(mut sink) = sinks.get_mut(e) {
            bevy::audio::AudioSinkPlayback::set_volume(&mut *sink, Volume::Linear(base * music.volume));
        }
    }
}

/// A footstep notify of `owner`'s animation.
#[derive(Event, Clone, Copy, Debug)]
pub struct Footstep {
    pub owner: Entity,
    pub left: bool,
}

#[derive(Resource)]
pub struct Sounds {
    dir: PathBuf,
    cues: HashMap<String, Option<Cue>>,
    footsteps: Option<Footsteps>,
    terrain: Vec<TerrainSurfaces>,
}

impl Sounds {
    fn cue(&mut self, name: &str) -> Option<&Cue> {
        let dir = &self.dir;
        self.cues
            .entry(name.to_string())
            .or_insert_with(|| Cue::load(dir, name).map_err(|_| warn!("sound {name}: no {}/{name}.json (re-export with tools/f13-export sounds)", dir.display())).ok())
            .as_ref()
    }

    /// The terrain's surface at (x, z): the paint layer that weighs most there.
    fn terrain_surface(&self, x: f32, z: f32) -> u8 {
        self.terrain.iter().find_map(|t| t.at(x, z)).unwrap_or(0)
    }
}

/// A landscape's paint layers (surface type and weight map) and the world XZ -> weight map UV mapping.
struct TerrainSurfaces {
    layers: Vec<(u8, Vec<u8>)>,
    size: UVec2,
    /// u = a.x * x + a.y * z + a.z, v likewise with b.
    u: Vec3,
    v: Vec3,
}

impl TerrainSurfaces {
    fn at(&self, x: f32, z: f32) -> Option<u8> {
        let p = Vec3::new(x, z, 1.0);
        let (u, v) = (self.u.dot(p), self.v.dot(p));
        if !(0.0..=1.0).contains(&u) || !(0.0..=1.0).contains(&v) {
            return None;
        }
        let px = ((u * (self.size.x - 1) as f32).round() as u32).min(self.size.x - 1);
        let py = ((v * (self.size.y - 1) as f32).round() as u32).min(self.size.y - 1);
        let i = (py * self.size.x + px) as usize;
        let (best, w) = self.layers.iter().map(|(s, w)| (*s, w.get(i).copied().unwrap_or(0))).max_by_key(|(_, w)| *w)?;
        Some(if w == 0 { 0 } else { best })
    }
}

/// Least-squares fit of `value = a.x * x + a.y * z + a.z` over the samples.
fn fit_plane(samples: &[(f32, f32, f32)]) -> Option<Vec3> {
    let mut m = Mat3::ZERO;
    let mut r = Vec3::ZERO;
    for &(x, z, val) in samples {
        let p = Vec3::new(x, z, 1.0);
        m += Mat3::from_cols(p * p.x, p * p.y, p * p.z);
        r += p * val;
    }
    (m.determinant().abs() > 1e-9).then(|| m.inverse() * r)
}

fn load_terrain(export: &std::path::Path, level: &crate::Level) -> Vec<TerrainSurfaces> {
    let mut out = Vec::new();
    for layout in &level.layouts {
        for l in &layout.landscapes {
            let dir = export.join(l.file.trim_end_matches(".glb"));
            let Ok(text) = std::fs::read_to_string(dir.join("surfaces.json")) else { continue };
            let Ok(map) = serde_json::from_str::<HashMap<String, u8>>(&text) else { continue };
            let xf = layout
                .actors
                .iter()
                .find(|a| a.level == l.level && a.name == l.actor)
                .and_then(|a| a.transform.as_ref())
                .map(crate::to_transform)
                .unwrap_or_default()
                .compute_affine();
            let Ok(verts) = f13_data::collision::glb_vertices(&export.join(&l.file)) else { continue };
            let pts: Vec<(Vec3, [f32; 2])> = verts.iter().step_by(7).map(|(p, uv)| (xf.transform_point3(Vec3::from_array(*p)), *uv)).collect();
            let (Some(u), Some(v)) = (
                fit_plane(&pts.iter().map(|(p, uv)| (p.x, p.z, uv[0])).collect::<Vec<_>>()),
                fit_plane(&pts.iter().map(|(p, uv)| (p.x, p.z, uv[1])).collect::<Vec<_>>()),
            ) else {
                continue;
            };
            // The landscape's UVs count its grid's quads (0..quads), not 0..1: normalise by their range.
            let (umax, vmax) = verts.iter().fold((0f32, 0f32), |(a, b), (_, uv)| (a.max(uv[0]), b.max(uv[1])));
            let (u, v) = (u / if umax > 1.5 { umax } else { 1.0 }, v / if vmax > 1.5 { vmax } else { 1.0 });
            let mut size = UVec2::ZERO;
            let mut layers = Vec::new();
            for (name, surface) in &map {
                let Ok(bytes) = std::fs::read(dir.join(format!("{name}.png"))) else { continue };
                let Ok(img) = Image::from_buffer(&bytes, ImageType::Extension("png"), CompressedImageFormats::NONE, false, ImageSampler::Default, RenderAssetUsages::MAIN_WORLD) else {
                    continue;
                };
                let Some(data) = img.data.as_ref() else { continue };
                if size != UVec2::ZERO && size != img.size() {
                    continue;
                }
                size = img.size();
                // Greyscale decodes to one byte per pixel; else take the first channel.
                let stride = (data.len() / (size.x * size.y).max(1) as usize).max(1);
                layers.push((*surface, data.iter().step_by(stride).copied().collect()));
            }
            if !layers.is_empty() {
                info!("terrain surfaces: {} layers ({}x{})", layers.len(), size.x, size.y);
                out.push(TerrainSurfaces { layers, size, u, v });
            }
        }
    }
    out
}

fn load_sounds(mut commands: Commands, export: Res<crate::ExportRoot>, level: Res<crate::Level>) {
    let dir = export.0.join("sounds");
    let footsteps = Footsteps::load(&dir).map_err(|e| warn!("no footstep sounds ({e}): re-export with tools/f13-export sounds")).ok();
    commands.insert_resource(Sounds { dir, cues: HashMap::new(), footsteps, terrain: load_terrain(&export.0, &level) });
}

/// The camera hears.
fn add_listener(mut commands: Commands, cameras: Query<Entity, (With<Camera3d>, Without<SpatialListener>)>) {
    for e in &cameras {
        commands.entity(e).insert(SpatialListener::new(EAR_GAP_M));
    }
}

/// A placed sound's base volume and how it fades.
#[derive(Component)]
struct Emitter {
    volume: f32,
    attenuation: Attenuation,
}

fn listener_position(cameras: &Query<&GlobalTransform, With<SpatialListener>>) -> Option<Vec3> {
    cameras.iter().next().map(|g| g.translation())
}

fn play_sound(
    ev: On<PlaySound>,
    mut commands: Commands,
    assets: Res<AssetServer>,
    sounds: Option<ResMut<Sounds>>,
    mut rng: ResMut<crate::GameRng>,
    listener: Query<&GlobalTransform, With<SpatialListener>>,
) {
    let Some(mut sounds) = sounds else { return };
    let Some(cue) = sounds.cue(&ev.cue).cloned() else { return };
    let (voices, att) = cue.play(&mut || rng.0.frand());
    let ear = listener_position(&listener);
    for v in voices {
        let mut settings = PlaybackSettings {
            mode: if v.looping { PlaybackMode::Loop } else { PlaybackMode::Despawn },
            speed: v.pitch,
            ..PlaybackSettings::DESPAWN
        };
        let source = AudioPlayer::new(assets.load(format!("sounds/{}", v.file)));
        match (ev.at, att.clone().filter(|a| a.attenuate || a.spatialize)) {
            (Some(at), Some(att)) => {
                let factor = ear.map_or(1.0, |e| att.factor(e.distance(at) * 100.0));
                settings.volume = Volume::Linear(v.volume * factor);
                settings.spatial = att.spatialize;
                settings.spatial_scale = Some(SpatialScale::new(SPATIAL_SCALE));
                commands.spawn((source, settings, Transform::from_translation(at), Emitter { volume: v.volume, attenuation: att }));
            }
            _ => {
                settings.volume = Volume::Linear(v.volume);
                commands.spawn((source, settings));
            }
        }
    }
}

/// Fades placed sounds with the listener's distance.
fn attenuate(listener: Query<&GlobalTransform, With<SpatialListener>>, mut emitters: Query<(&Emitter, &GlobalTransform, &mut SpatialAudioSink)>) {
    let Some(ear) = listener_position(&listener) else { return };
    for (e, g, mut sink) in &mut emitters {
        let factor = e.attenuation.factor(ear.distance(g.translation()) * 100.0);
        bevy::audio::AudioSinkPlayback::set_volume(&mut *sink, Volume::Linear(e.volume * factor));
    }
}

/// A footstep: the cue for the surface under the foot, played there.
fn footstep(
    ev: On<Footstep>,
    mut commands: Commands,
    sounds: Option<Res<Sounds>>,
    players: Query<(&Player, &GlobalTransform)>,
    (collision, dynamic): (Option<Res<crate::collision::WorldCollision>>, Option<Res<crate::collision::DynamicColliders>>),
) {
    let (Some(sounds), Some(collision), Some(dynamic)) = (sounds, collision, dynamic) else { return };
    let Some(tables) = &sounds.footsteps else { return };
    let Ok((p, g)) = players.get(ev.owner) else { return };
    let feet = g.translation();
    // Her foot's side of her (the game traces from the foot socket).
    let side = g.rotation() * Vec3::X * if ev.left { 0.1 } else { -0.1 };
    let at = feet + side;
    let world = collision.with(&dynamic);
    let surface = match f13_core::collision::ground_surface(&world, at.x, at.z, at.y + 0.5, at.y - 0.5) {
        Some((_, f13_core::collision::Tri::TERRAIN)) => sounds.terrain_surface(at.x, at.z),
        Some((_, s)) => s,
        None => 0,
    };
    let c = &p.counselor;
    let gait = if c.running || c.sprinting { "run" } else { "walk" };
    if let Some(cue) = tables.cue("female", gait, surface) {
        trace!("footstep: surface {surface}, {cue}");
        commands.trigger(PlaySound { cue: cue.to_string(), at: Some(at) });
    }
}
