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
//! or for 0 while his music is silenced (Stalk); rising, it fades the counselor's music track out over 0.2 s.
//!
//! The counselor's music (SCMusicComponent::PlayMusic with Athlete's MusicObjectClass, MusicCounselor): a track starts
//! at once, or fades the playing one out over its FadeOutTime and fades in over FadeInTime; a finished track is
//! followed by CounselorIdle after MinIdleMusicTime..MaxIdleMusicTime. MatchStart plays when the match starts; the
//! TwoMinuteWarningMusic at two minutes left; DeathMusic when she dies, SurviveMusic when she escapes. Stingers
//! ([`MusicStinger`]: a STINGER entry's cue over the music): JasonTeleportedNearStinger when he Morphs in within his
//! near music's reach (NearCounselorMusicDistance; PLACEHOLDER rule). JasonAbilityUnlockWarning when he unlocks an
//! ability. Not yet: the other stingers' triggers (OpenDoorStinger, SeeCounselorDeadBody), scary sounds.
//!
//! Delay nodes start their voices late (started paused). PLACEHOLDERS: the listener is the camera; actor sounds
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
            .init_resource::<MusicState>()
            .add_systems(Update, (add_listener, attenuate, actor_sounds, counselor_music, near_music, fades, ambience, voices, start_delayed).chain())
            .add_observer(play_sound)
            .add_observer(footstep)
            .add_observer(actor_sound)
            .add_observer(stop_voice_over)
            .add_observer(actor_loop)
            .add_observer(stop_loop)
            .add_observer(music_stinger)
            .add_observer(dropped);
    }
}

/// Play a cue: at a place (attenuated and panned), or everywhere.
#[derive(Event, Clone, Debug)]
pub struct PlaySound {
    pub cue: String,
    pub at: Option<Vec3>,
}

/// The sounds animations play (their AnimNotify_PlaySound notifies, anim_sounds.json): take -> [(time s, cue)].
#[derive(Resource, Default, Debug)]
pub struct AnimSounds(HashMap<String, Vec<(f32, String)>>);

impl AnimSounds {
    /// The cues `take` plays after `from` (s; negative from its start), up to `to`.
    pub fn passed<'a>(&'a self, take: &str, from: f32, to: f32) -> impl Iterator<Item = &'a str> + 'a {
        self.0.get(take).into_iter().flatten().filter(move |(t, _)| from < *t && *t <= to).map(|(_, c)| c.as_str())
    }
}

/// Plays the cues a character's action passed this frame, where it stands.
pub fn play_anim_sounds(commands: &mut Commands, sounds: Option<&AnimSounds>, span: Option<(&str, f32, f32)>, at: Vec3) {
    let (Some(sounds), Some((take, from, to))) = (sounds, span) else { return };
    for cue in sounds.passed(take, from, to) {
        commands.trigger(PlaySound { cue: cue.to_string(), at: Some(at) });
    }
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

/// Keep playing the cue `actor`'s class names under `prop` on it (a car's engine), or the cue `prop` itself when its
/// class names none: its voices follow the actor.
#[derive(Event, Clone, Copy, Debug)]
pub struct ActorLoop {
    pub actor: Entity,
    pub prop: &'static str,
}

/// Stop the loop `actor` plays under `prop` (see [`ActorLoop`]).
#[derive(Event, Clone, Copy, Debug)]
pub struct StopLoop {
    pub actor: Entity,
    pub prop: &'static str,
}

/// A voice of the loop an actor plays under this property.
#[derive(Component)]
struct LoopOf(&'static str);

fn stop_loop(ev: On<StopLoop>, mut commands: Commands, voices: Query<(Entity, &LoopOf, &ChildOf)>) {
    for (e, l, parent) in &voices {
        if parent.parent() == ev.actor && l.0 == ev.prop {
            commands.entity(e).despawn();
        }
    }
}

/// An item put down (its class's DroppedSoundCue, where it lands).
#[derive(Event, Clone, Copy, Debug)]
pub struct Dropped {
    pub item: Entity,
    pub at: Vec3,
}

fn dropped(ev: On<Dropped>, mut commands: Commands, items: Query<&crate::props::Item>) {
    if let Some(cue) = items.get(ev.item).ok().and_then(|i| i.drop_cue.clone()) {
        commands.trigger(PlaySound { cue, at: Some(ev.at) });
    }
}

fn actor_loop(
    ev: On<ActorLoop>,
    mut commands: Commands,
    assets: Res<AssetServer>,
    sounds: Option<ResMut<Sounds>>,
    mut rng: ResMut<crate::GameRng>,
    actors: Query<&ClassSounds>,
) {
    let Some(mut sounds) = sounds else { return };
    let name = actors.get(ev.actor).ok().and_then(|s| s.0.get(ev.prop).cloned()).unwrap_or_else(|| ev.prop.to_string());
    let Some(cue) = sounds.cue(&name).cloned() else { return };
    trace!("loop {} on {}", ev.prop, ev.actor);
    let (voices, att) = cue.play(&mut || rng.0.frand());
    for v in voices {
        let settings = PlaybackSettings { mode: PlaybackMode::Loop, speed: v.pitch, volume: Volume::Linear(v.volume), ..PlaybackSettings::LOOP };
        let source = AudioPlayer::new(assets.load(format!("sounds/{}", v.file)));
        let mut e = commands.spawn((source, settings, Transform::from_translation(Vec3::Y), ChildOf(ev.actor), LoopOf(ev.prop)));
        if let Some(att) = att.clone().filter(|a| a.attenuate || a.spatialize) {
            e.insert((PlaybackSettings { spatial: att.spatialize, spatial_scale: Some(SpatialScale::new(SPATIAL_SCALE)), ..settings }, Emitter { volume: v.volume, attenuation: att }));
        }
    }
}

/// Play the cue `actor`'s class names under `prop`, at the actor. A voice line ("VO:<name>") is said by the actor's
/// voice: SCVoiceOverComponent plays its lines on one audio component (0x14031bff0), so a new line cuts the last
/// one off; the voice follows the actor.
#[derive(Event, Clone, Copy, Debug)]
pub struct ActorSound {
    pub actor: Entity,
    pub prop: &'static str,
}

/// A voice of the line an actor is saying.
#[derive(Component)]
pub struct VoiceOf(pub &'static str);

/// SCVoiceOverComponent's StopVoiceOver (0x140320a10): the line the actor is saying stops.
#[derive(Event, Clone, Copy, Debug)]
pub struct StopVoiceOver {
    pub actor: Entity,
}

fn stop_voice_over(ev: On<StopVoiceOver>, mut commands: Commands, voices: Query<(Entity, &ChildOf), With<VoiceOf>>) {
    debug!("voice: {} stops", ev.actor);
    for (e, parent) in &voices {
        if parent.parent() == ev.actor {
            commands.entity(e).despawn();
        }
    }
}

/// Whether `actor` is saying `line` (IsVoiceOverPlaying).
pub fn saying(voices: &Query<(&VoiceOf, &ChildOf)>, actor: Entity, line: &str) -> bool {
    voices.iter().any(|(v, p)| p.parent() == actor && v.0 == line)
}

#[allow(clippy::too_many_arguments)]
fn actor_sound(
    ev: On<ActorSound>,
    mut commands: Commands,
    actors: Query<(&ClassSounds, &GlobalTransform)>,
    voices: Query<(Entity, &ChildOf), With<VoiceOf>>,
    assets: Res<AssetServer>,
    sounds: Option<ResMut<Sounds>>,
    mut rng: ResMut<crate::GameRng>,
) {
    let Ok((s, g)) = actors.get(ev.actor) else { return };
    let Some(cue) = s.0.get(ev.prop) else { return };
    if !ev.prop.starts_with("VO:") {
        commands.trigger(PlaySound { cue: cue.clone(), at: Some(g.translation() + Vec3::Y) });
        return;
    }
    let Some(mut sounds) = sounds else { return };
    let Some(c) = sounds.cue(cue).cloned() else { return };
    debug!("voice: {} says {}", ev.actor, ev.prop);
    for (e, parent) in &voices {
        if parent.parent() == ev.actor {
            commands.entity(e).despawn();
        }
    }
    let (played, att) = c.play(&mut || rng.0.frand());
    for v in played {
        let settings = PlaybackSettings {
            mode: if v.looping { PlaybackMode::Loop } else { PlaybackMode::Despawn },
            speed: v.pitch,
            volume: Volume::Linear(v.volume),
            paused: v.delay > 0.0,
            ..PlaybackSettings::DESPAWN
        };
        let source = AudioPlayer::new(assets.load(format!("sounds/{}", v.file)));
        let mut e = commands.spawn((source, settings, Transform::from_translation(Vec3::Y), ChildOf(ev.actor), VoiceOf(ev.prop)));
        if v.delay > 0.0 {
            e.insert(StartAfter(v.delay));
        }
        if let Some(att) = att.clone().filter(|a| a.attenuate || a.spatialize) {
            e.insert((PlaybackSettings { spatial: att.spatialize, spatial_scale: Some(SpatialScale::new(SPATIAL_SCALE)), ..settings }, Emitter { volume: v.volume, attenuation: att }));
        }
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
    rigs: Query<&crate::character::Rig>,
) {
    let mut play = |actor: Entity, prop: &'static str| commands.trigger(ActorSound { actor, prop });
    // A character's door take plays the door's sounds itself (its notifies: anim_sounds.json). PLACEHOLDER rule.
    let door_take = rigs.iter().any(|r| r.action.as_ref().is_some_and(|(a, _)| a.contains("_door_open_") || a.contains("_door_close_")));
    for (e, d) in &doors {
        let now = (d.open, d.health, d.broken);
        if let Some(&(open, health, broken)) = last.get(&e) {
            if d.broken && !broken {
                play(e, "DoorDestroySoundCue");
            } else if d.health < health {
                play(e, "DoorHitSoundCue");
            } else if d.open != open && !d.broken && !door_take {
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

/// A 2D (unpanned) player of a cue's first voice: the entity and the voice's volume.
fn spawn_2d(commands: &mut Commands, assets: &AssetServer, sounds: &mut Sounds, rng: &mut crate::GameRng, cue: &str, looping: bool, volume: f32) -> Option<(Entity, f32)> {
    let c = sounds.cue(cue)?.clone();
    let (voices, _) = c.play(&mut || rng.0.frand());
    let v = voices.into_iter().next()?;
    let mode = if looping || v.looping { PlaybackSettings::LOOP } else { PlaybackSettings::DESPAWN };
    let settings = PlaybackSettings { volume: Volume::Linear(v.volume * volume), speed: v.pitch, ..mode };
    Some((commands.spawn((AudioPlayer::new(assets.load(format!("sounds/{}", v.file))), settings)).id(), v.volume))
}

/// The line her voice object says as an action starts (its take): swings, windows, the kills.
fn action_line(take: &str) -> Option<&'static str> {
    Some(match take {
        t if t.contains("_melee_heavy") => "VO:WeaponSwingHeavy",
        t if t.contains("_melee_light") => "VO:WeaponSwing",
        t if t.contains("window_climb") => "VO:ClimbThroughWindow",
        "f_normal_stand_window_dive" => "VO:WindowDiveStart",
        "f_normal_dive_land_01" => "VO:WindowDiveLand",
        "cinematic_pickaxe_throathack" => "VO:PickAxeThroatHackKill",
        "cinematic_pickaxe_head_canopener" => "VO:CanOpenerKill",
        "cinematic_pickaxe_curbstomp" => "VO:CurbStompKill",
        "cinematic_pickaxe_sweeptheleg" => "VO:SweepTheLegKill",
        "cinematic_pickaxe_toppopper" => "VO:TopPopperKill",
        "cinematic_hidespot_closet_kill_Axe" => "VO:HIdeSpotClosetAxeKill",
        "cinematic_hidespot_bed_kill_Axe" => "VO:AxeBedKill",
        _ => return None,
    })
}

/// Her voice lines (her VoiceOverObjectClass's, as her class sounds) on what happens to her: Impact when she's hurt,
/// Wounded as she becomes wounded, Stumble, Died (a death outside a kill), and each action's line as it starts (swings,
/// windows, kills: [`action_line`]). Choke as Base_Counselor's graph says it: a second after she's grabbed
/// (BP_OnGrabbed's Delay, which a second grab while it waits doesn't restart), stopped when Jason drops her
/// (BP_OnDropped) or while she's in a special move (its tick: a kill, its take or its camera running). Her lines only while she's alive (PlayVoiceOver).
/// PLACEHOLDER: the game's voice component picks the others on its own events, not read; the hiding lines are
/// hiding.rs's.
#[allow(clippy::type_complexity)]
fn voices(
    mut commands: Commands,
    time: Res<Time>,
    mut last: Local<Option<(f32, bool, bool, Option<String>, Option<String>, bool)>>,
    mut choke_at: Local<Option<f32>>,
    players: Query<(Entity, &Player, Option<&crate::character::PlayAction>, Has<crate::jason::Held>), With<ClassSounds>>,
    saying_now: Query<(&VoiceOf, &ChildOf)>,
    kills: Query<(), With<crate::kill_camera::KillCam>>,
) {
    let Ok((pe, p, action, held)) = players.single() else { return };
    let c = &p.counselor;
    let now = (c.health, c.wounded, held, action.map(|a| a.0.clone()), p.montage.clone(), c.alive());
    let Some((health, wounded, was_held, was_action, was_montage, was_alive)) = last.replace(now.clone()) else { return };
    let alive = now.5;
    let mut say = |line: &'static str| {
        if alive || line == "VO:Died" {
            commands.trigger(ActorSound { actor: pe, prop: line });
        }
    };
    let kill = |a: &Option<String>| a.as_deref().and_then(action_line).is_some_and(|l| l.ends_with("Kill"));
    let in_kill = kill(&now.3) || kill(&was_action);
    let t = time.elapsed_secs();
    if held && !was_held && choke_at.is_none() {
        *choke_at = Some(t + 1.0);
    }
    if choke_at.is_some_and(|at| t >= at) {
        *choke_at = None;
        say("VO:Choke");
    }
    if !now.5 && was_alive && !in_kill {
        say("VO:Died");
    } else if now.0 < health && now.5 {
        say("VO:Impact");
    }
    if now.1 && !wounded && now.5 {
        say("VO:Wounded");
    }
    if now.4.as_deref().is_some_and(|m| m.contains("stumble")) && now.4 != was_montage {
        say("VO:Stumble");
    }
    // A new line (the kill's) cuts Choke off itself.
    let mut said = false;
    if now.3 != was_action {
        if let Some(line) = now.3.as_deref().and_then(action_line) {
            say(line);
            said = true;
        }
    }
    if was_held && !held {
        commands.trigger(StopVoiceOver { actor: pe });
    } else if !said && (kill(&now.3) || !kills.is_empty()) && saying(&saying_now, pe, "VO:Choke") {
        commands.trigger(StopVoiceOver { actor: pe });
    }
}

/// One of the map's ambient loops (its AmbientSounds' AudioComponents): its volume (cue volume times
/// VolumeMultiplier) and, for a box attenuation override, the box (world, Unreal cm: centre, rotation, extents) and
/// its falloff: full inside, fading linearly to nothing FalloffDistance outside (flat; the boxes are 2D).
#[derive(Component)]
struct Ambient {
    volume: f32,
    area: Option<(Vec3, Quat, Vec2, f32)>,
}

/// Starts the ambient loops once the sounds are in, then keeps the attenuated ones' volume to the listener.
fn ambience(
    mut commands: Commands,
    mut started: Local<bool>,
    assets: Res<AssetServer>,
    level: Res<crate::Level>,
    sounds: Option<ResMut<Sounds>>,
    mut rng: ResMut<crate::GameRng>,
    listener: Query<&GlobalTransform, With<SpatialListener>>,
    mut playing: Query<(&Ambient, &mut bevy::audio::AudioSink)>,
) {
    let ear = listener_position(&listener);
    if *started {
        let Some(ear) = ear else { return };
        let ue = Vec3::new(ear.x, ear.z, ear.y) * 100.0;
        for (a, mut sink) in &mut playing {
            let Some((centre, rotation, extents, falloff)) = a.area else { continue };
            let local = rotation.inverse() * (ue - centre);
            let outside = Vec2::new((local.x.abs() - extents.x).max(0.0), (local.y.abs() - extents.y).max(0.0)).length();
            sink.set_volume(Volume::Linear(a.volume * (1.0 - outside / falloff.max(1.0)).clamp(0.0, 1.0)));
        }
        return;
    }
    let Some(mut sounds) = sounds else { return };
    *started = true;
    let mut n = 0;
    for c in level.layouts.iter().flat_map(|l| &l.actors).filter(|a| a.class == "AmbientSound").flat_map(|a| &a.components) {
        let Some(cue) = cue_name(&c.props["Sound"]) else { continue };
        let multiplier = c.props["VolumeMultiplier"].as_f64().map_or(1.0, |v| v as f32);
        let o = &c.props["AttenuationOverrides"];
        let area = (c.props["bOverrideAttenuation"].as_bool() == Some(true) && o["AttenuationShape"] == "EAttenuationShape::Box")
            .then(|| c.world)
            .flatten()
            .map(|w| {
                let e = |k: &str| o["AttenuationShapeExtents"][k].as_f64().unwrap_or(0.0) as f32;
                (Vec3::from_array(w.t), Quat::from_array(w.r), Vec2::new(e("X"), e("Y")), o["FalloffDistance"].as_f64().unwrap_or(0.0) as f32)
            });
        if let Some((e, volume)) = spawn_2d(&mut commands, &assets, &mut sounds, &mut rng, &cue, true, multiplier) {
            commands.entity(e).insert(Ambient { volume: volume * multiplier, area });
            n += 1;
        }
    }
    info!("ambience: {n} loops");
}

/// A player's volume heading for `target` (times its cue volume `base`) at `rate` per second; despawned at 0 if asked.
#[derive(Component)]
struct Fade {
    base: f32,
    level: f32,
    target: f32,
    rate: f32,
    despawn: bool,
}

fn fades(mut commands: Commands, time: Res<Time>, mut players: Query<(Entity, &mut Fade, Option<&mut bevy::audio::AudioSink>)>) {
    for (e, mut f, sink) in &mut players {
        let step = f.rate * time.delta_secs();
        f.level += (f.target - f.level).clamp(-step, step);
        if let Some(mut sink) = sink {
            bevy::audio::AudioSinkPlayback::set_volume(&mut *sink, Volume::Linear(f.base * f.level));
        }
        if f.despawn && f.level <= 0.0 {
            commands.entity(e).despawn();
        }
    }
}

/// A music event for the counselor: a STINGER entry of her MusicData (its cue played over the music), or one of her
/// music object's own cues (`JasonAbilityUnlockWarning`).
#[derive(Event, Clone, Copy, Debug)]
pub struct MusicStinger(pub &'static str);

fn music_stinger(
    ev: On<MusicStinger>,
    mut commands: Commands,
    assets: Res<AssetServer>,
    sounds: Option<ResMut<Sounds>>,
    counselor: Option<Res<crate::character::CounselorCharacter>>,
    mut rng: ResMut<crate::GameRng>,
) {
    let (Some(mut sounds), Some(counselor)) = (sounds, counselor) else { return };
    let music = &counselor.info.music;
    let entry = music["MusicData"].as_array().into_iter().flatten().find(|d| d["EventName"] == ev.0);
    let cue = entry.map_or_else(|| cue_name(&music[ev.0]), |e| cue_name(&e["MusicTrack"]));
    if let Some(cue) = cue {
        spawn_2d(&mut commands, &assets, &mut sounds, &mut rng, &cue, false, 1.0);
        info!("music: {}", ev.0);
    }
}

/// The counselor's music track (its player and FadeOutTime), when the idle music is due, and what has played.
#[derive(Resource, Default)]
pub struct MusicState {
    track: Option<(Entity, f32, f32)>,
    idle_at: Option<f32>,
    match_started: bool,
    two_minutes: bool,
    ended: bool,
}

impl MusicState {
    /// Fades the playing track out over `seconds`.
    fn fade_out(&mut self, commands: &mut Commands, seconds: f32) {
        if let Some((e, base, _)) = self.track.take() {
            if let Ok(mut ec) = commands.get_entity(e) {
                ec.insert(Fade { base, level: 1.0, target: 0.0, rate: 1.0 / seconds.max(0.01), despawn: true });
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn counselor_music(
    mut commands: Commands,
    time: Res<Time>,
    assets: Res<AssetServer>,
    mut state: ResMut<MusicState>,
    sounds: Option<ResMut<Sounds>>,
    counselor: Option<Res<crate::character::CounselorCharacter>>,
    (game, escaped): (Option<Res<crate::match_flow::GameMatch>>, Option<Res<crate::vehicle::Escaped>>),
    mut rng: ResMut<crate::GameRng>,
    players: Query<&Player>,
    playing: Query<Entity>,
) {
    let (Some(mut sounds), Some(counselor)) = (sounds, counselor) else { return };
    let music = counselor.info.music.clone();
    if music.is_null() {
        return;
    }
    let now = time.elapsed_secs();
    // A finished track (checked before anything starts: a new player spawns at the end of the frame): the idle music
    // a while later.
    if let Some((e, _, _)) = state.track {
        if playing.get(e).is_err() {
            state.track = None;
            let lo = music["MinIdleMusicTime"].as_f64().unwrap_or(5.0) as f32;
            let hi = music["MaxIdleMusicTime"].as_f64().unwrap_or(10.0) as f32;
            state.idle_at = Some(now + lo + (hi - lo) * rng.0.frand());
        }
    }
    // PlayMusic(event), for a MUSICTRACK entry.
    let play = |state: &mut MusicState, commands: &mut Commands, sounds: &mut Sounds, rng: &mut crate::GameRng, event: &str| {
        let Some(entry) = music["MusicData"].as_array().into_iter().flatten().find(|d| d["EventName"] == event) else { return };
        let Some(cue) = cue_name(&entry["MusicTrack"]) else { return };
        let fade_in = entry["FadeInTime"].as_f64().unwrap_or(0.0) as f32;
        let fade_out = entry["FadeOutTime"].as_f64().unwrap_or(0.0) as f32;
        if let Some((_, _, out)) = state.track {
            state.fade_out(commands, out);
        }
        let start = if fade_in > 0.0 { 0.0 } else { 1.0 };
        if let Some((e, base)) = spawn_2d(commands, &assets, sounds, rng, &cue, false, start) {
            if fade_in > 0.0 {
                commands.entity(e).insert(Fade { base, level: 0.0, target: 1.0, rate: 1.0 / fade_in, despawn: false });
            }
            state.track = Some((e, base, fade_out));
            state.idle_at = None;
            info!("music: {event}");
        }
    };
    let in_progress = game.as_ref().is_some_and(|g| g.state.state == crate::match_flow::MatchState::InProgress);
    if in_progress && !state.match_started {
        state.match_started = true;
        play(&mut state, &mut commands, &mut sounds, &mut rng, "MatchStart");
    }
    if state.idle_at.is_some_and(|t| now >= t) && !state.ended {
        play(&mut state, &mut commands, &mut sounds, &mut rng, "CounselorIdle");
    }
    // Two minutes left.
    if in_progress && !state.two_minutes && game.as_ref().is_some_and(|g| g.state.remaining_time <= 120) {
        state.two_minutes = true;
        if let Some(cue) = cue_name(&music["TwoMinuteWarningMusic"]) {
            spawn_2d(&mut commands, &assets, &mut sounds, &mut rng, &cue, false, 1.0);
            info!("music: two-minute warning");
        }
    }
    // Her end: death or escape.
    let Ok(p) = players.single() else { return };
    let end = if !p.counselor.alive() {
        Some("DeathMusic")
    } else if escaped.is_some() {
        Some("SurviveMusic")
    } else {
        None
    };
    if let (Some(which), false) = (end, state.ended) {
        state.ended = true;
        state.fade_out(&mut commands, 0.5);
        state.idle_at = None;
        if let Some(cue) = cue_name(&music[which]) {
            spawn_2d(&mut commands, &assets, &mut sounds, &mut rng, &cue, false, 1.0);
            info!("music: {which}");
        }
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
    (mut sinks, mut state): (Query<&mut bevy::audio::AudioSink>, ResMut<MusicState>),
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
    // Rising, it fades the counselor's track out (0.2 s).
    if target > 0.001 {
        state.fade_out(&mut commands, 0.2);
    }
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
    /// Landing from a fall (GetLandingSound) rather than a step.
    pub land: bool,
}

#[derive(Resource)]
pub struct Sounds {
    dir: PathBuf,
    cues: HashMap<String, Option<Cue>>,
    footsteps: Option<Footsteps>,
    /// Jason's (Jason_AnimBP's GetFootstepSound).
    jason_footsteps: Option<Footsteps>,
    terrain: Vec<TerrainSurfaces>,
}

impl Sounds {
    /// How far (cm) a cue can be heard: its attenuation's inner radius plus falloff (None: not attenuated, or no
    /// such cue). Random nodes don't change it, so one evaluation tells.
    pub fn audible_range(&mut self, cue: &str) -> Option<f32> {
        let c = self.cue(cue)?.clone();
        let (_, att) = c.play(&mut || 0.0);
        att.filter(|a| a.attenuate).map(|a| a.inner_radius + a.falloff)
    }

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
    let jason_footsteps = Footsteps::load_file(&dir, "footsteps_jason.json").ok();
    let anim = std::fs::read_to_string(export.0.join("anim_sounds.json"))
        .map_err(|e| e.to_string())
        .and_then(|s| serde_json::from_str(&s).map_err(|e| e.to_string()))
        .map_err(|e| warn!("no animation sounds ({e}): re-export with tools/f13-export sounds"))
        .unwrap_or_default();
    commands.insert_resource(AnimSounds(anim));
    commands.insert_resource(Sounds { dir, cues: HashMap::new(), footsteps, jason_footsteps, terrain: load_terrain(&export.0, &level) });
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
    trace!("play {} at {:?}", ev.cue, ev.at);
    let (voices, att) = cue.play(&mut || rng.0.frand());
    let ear = listener_position(&listener);
    for v in voices {
        let mut settings = PlaybackSettings {
            mode: if v.looping { PlaybackMode::Loop } else { PlaybackMode::Despawn },
            speed: v.pitch,
            paused: v.delay > 0.0,
            ..PlaybackSettings::DESPAWN
        };
        let source = AudioPlayer::new(assets.load(format!("sounds/{}", v.file)));
        let e = match (ev.at, att.clone().filter(|a| a.attenuate || a.spatialize)) {
            (Some(at), Some(att)) => {
                let factor = ear.map_or(1.0, |e| att.factor(e.distance(at) * 100.0));
                settings.volume = Volume::Linear(v.volume * factor);
                settings.spatial = att.spatialize;
                settings.spatial_scale = Some(SpatialScale::new(SPATIAL_SCALE));
                commands.spawn((source, settings, Transform::from_translation(at), Emitter { volume: v.volume, attenuation: att })).id()
            }
            _ => {
                settings.volume = Volume::Linear(v.volume);
                commands.spawn((source, settings)).id()
            }
        };
        if v.delay > 0.0 {
            commands.entity(e).insert(StartAfter(v.delay));
        }
    }
}

/// A voice waiting out its cue's Delay nodes (spawned paused).
#[derive(Component)]
struct StartAfter(f32);

fn start_delayed(mut commands: Commands, time: Res<Time>, mut voices: Query<(Entity, &mut StartAfter, Option<&AudioSink>, Option<&SpatialAudioSink>)>) {
    for (e, mut s, sink, spatial) in &mut voices {
        s.0 -= time.delta_secs();
        if s.0 > 0.0 {
            continue;
        }
        // Its sink comes once its source has loaded.
        if let Some(sink) = sink {
            sink.play();
        } else if let Some(sink) = spatial {
            sink.play();
        } else {
            continue;
        }
        commands.entity(e).remove::<StartAfter>();
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
    jasons: Query<(&crate::jason::Jason, &GlobalTransform)>,
    killer: Option<Res<crate::jason::KillerTuning>>,
    water: Option<Res<crate::water::Water>>,
) {
    let (Some(sounds), Some(collision), Some(dynamic)) = (sounds, collision, dynamic) else { return };
    // Hers, or Jason's (his table; running past his walking speed).
    let (tables, g, gait) = if let Ok((p, g)) = players.get(ev.owner) {
        let c = &p.counselor;
        (sounds.footsteps.as_ref(), g, if ev.land { "land" } else if c.running || c.sprinting { "run" } else { "walk" })
    } else if let Ok((j, g)) = jasons.get(ev.owner) {
        let walk = killer.as_ref().map_or(160.0, |t| t.walk_speed);
        (sounds.jason_footsteps.as_ref(), g, if j.velocity.length() > walk * 1.2 { "run" } else { "walk" })
    } else {
        return;
    };
    let Some(tables) = tables else { return };
    let feet = g.translation();
    // In water (GetFootstepInWater): deep water's cue past half a metre, else shallow water's.
    if let Some(top) = water.as_ref().and_then(|w| w.surface_at(feet + Vec3::Y * 0.05)) {
        let cue = if top - feet.y > 0.5 { tables.in_water.first() } else { tables.in_water.get(1) };
        if let Some(cue) = cue {
            trace!("footstep: in water, {cue}");
            commands.trigger(PlaySound { cue: cue.clone(), at: Some(feet) });
        }
        return;
    }
    // Her foot's side of her (the game traces from the foot socket).
    let side = g.rotation() * Vec3::X * if ev.left { 0.1 } else { -0.1 };
    let at = feet + side;
    let world = collision.with(&dynamic);
    let surface = match f13_core::collision::ground_surface(&world, at.x, at.z, at.y + 0.5, at.y - 0.5) {
        Some((_, f13_core::collision::Tri::TERRAIN)) => sounds.terrain_surface(at.x, at.z),
        Some((_, s)) => s,
        None => 0,
    };
    // (The decoder tags tables by the cue names' gender; Jason's carry none and come out "female".)
    if let Some(cue) = tables.cue("female", gait, surface) {
        trace!("footstep: surface {surface}, {cue}");
        commands.trigger(PlaySound { cue: cue.to_string(), at: Some(at) });
    }
}
