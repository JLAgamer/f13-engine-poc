//! Sound: the game's SoundCues (f13-data::sound, exported by tools/f13-export sounds) played at a place, attenuated
//! as Unreal attenuates them (FAttenuationSettings: full volume inside the inner radius, then the falloff curve), and
//! panned left/right around the camera; the counselor's footsteps.
//!
//! Footsteps (SCAnimInstance's handler, slot 0x2f0): at each Footstep_L / Footstep_R notify of her locomotion animation,
//! the surface under her (a mesh's physical material, or the terrain's paint layer that weighs most there) picks the cue
//! from Base_Counselor_AnimBP's tables (female: Athlete; run when running or sprinting, else walk); a surface without
//! one takes the default surface's. Jason's: his locomotion's notifies (jason.rs), Jason_AnimBP's tables.
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
//! ([`MusicStinger`]: a STINGER entry's cue over the music): JasonAbilityUnlockWarning when he unlocks an ability.
//! MusicCounselor's OpenDoorStinger and JasonTeleportedNearStinger are never played (no code or other package names
//! them); not yet: SeeCounselorDeadBody's trigger, scary sounds.
//!
//! Delay nodes start their voices late (started paused). Continuous modulators and param / distance cross-fades follow
//! the sound's parameters (`CueParams` on its actor, as AudioComponent::SetFloatParameter sets them) while it plays.
//! Actors' cues play at their root (PlaySoundAtLocation). The listener is the camera: APlayerController's own
//! GetAudioListenerPosition (its view point), as nothing sets an override (SetAudioListenerOverride has no caller in
//! the code or the packages). Her voice lines play at her root (SCVoiceOverComponent_Counselor's constructor
//! 0x1402f4760: its "Audio" component attached to the counselor's RootComponent, her capsule's centre).

use bevy::asset::RenderAssetUsages;
use bevy::image::{CompressedImageFormats, ImageSampler, ImageType};
use bevy::audio::{AddAudioSource, AudioPlayer, PlaybackMode, PlaybackSettings, SpatialAudioSink, SpatialListener, SpatialScale, Volume};
use bevy::prelude::*;
use f13_data::sound::{Attenuation, Cue, Footsteps};
use std::collections::HashMap;
use std::path::PathBuf;

use crate::player::Player;

/// Positions are scaled down this much for rodio's spatial sink, so its own inverse-square falloff stays at 1 (Unreal's
/// attenuation is applied here). Spatialized voices are panned by [`Panned`] instead once their clip is in; rodio's
/// own panning (spatial.rs `set_positions`) puts a sound louder in the far ear, by at most 6 dB.
const SPATIAL_SCALE: f32 = 0.001;
/// The listener's ear gap (m).
const EAR_GAP_M: f32 = 0.2;

/// The sounds' own random numbers (cue variations, modulators): sound never draws from the match's [`crate::GameRng`],
/// so how many sounds play doesn't change what the game rolls.
#[derive(Resource)]
pub(crate) struct AudioRng(pub f13_core::spawn::Rng);

/// A spatialized voice's stereo gains, set from where it is around the listener each frame ([`pan`]), and its voice
/// filter's coefficient (audio_volumes.rs; 0: none).
#[derive(Default)]
pub(crate) struct PanGains(std::sync::atomic::AtomicU64, std::sync::atomic::AtomicU32);

impl PanGains {
    fn set(&self, l: f32, r: f32) {
        self.0.store((u64::from(l.to_bits()) << 32) | u64::from(r.to_bits()), std::sync::atomic::Ordering::Relaxed);
    }
    fn get(&self) -> (f32, f32) {
        let v = self.0.load(std::sync::atomic::Ordering::Relaxed);
        (f32::from_bits((v >> 32) as u32), f32::from_bits(v as u32))
    }
    /// The low-pass's Frequency (XAUDIO2_FILTER_PARAMETERS: 0..1), 0 for none.
    pub(crate) fn set_filter(&self, frequency: f32) {
        self.1.store(frequency.to_bits(), std::sync::atomic::Ordering::Relaxed);
    }
    fn filter(&self) -> f32 {
        f32::from_bits(self.1.load(std::sync::atomic::Ordering::Relaxed))
    }
}

/// A clip played panned: its own gains (one asset per voice, sharing the clip's bytes).
#[derive(Asset, TypePath)]
pub(crate) struct Panned {
    clip: bevy::audio::AudioSource,
    gains: std::sync::Arc<PanGains>,
}

/// [`Panned`]'s samples: the clip's (a stereo clip's channels averaged) out as two channels at its gains.
pub(crate) struct PanDecoder {
    inner: <bevy::audio::AudioSource as bevy::audio::Decodable>::Decoder,
    gains: std::sync::Arc<PanGains>,
    right: Option<bevy::audio::Sample>,
    /// The voice filter's state (low, band).
    filter: (f32, f32),
}

impl Iterator for PanDecoder {
    type Item = bevy::audio::Sample;
    fn next(&mut self) -> Option<Self::Item> {
        use bevy::audio::Source;
        if let Some(r) = self.right.take() {
            return Some(r);
        }
        let s = match self.inner.channels().get() {
            1 => self.inner.next()?,
            n => {
                let first = self.inner.next()?;
                let mut sum = first;
                for _ in 1..n {
                    sum += self.inner.next().unwrap_or(first);
                }
                sum / n as bevy::audio::Sample
            }
        };
        // The source voice's filter (created with XAUDIO2_VOICE_USEFILTER: FXAudio2SoundSource 0x1419529b9) on the
        // mono signal before the output matrix: XAudio2_7.dll's state-variable filter (FUN_180044940, one channel;
        // its LowPassFilter output) at Frequency f, OneOverQ LowPassFilterResonance (BaseEngine.ini 0.9): high = in -
        // low - q x band, band += f x high, low += f x band. XAudio2 bypasses only a low-pass at 0 and a high-pass at 1
        // (FUN_180038918), so the game's 2 x 20000 / 44100 filters too. PLACEHOLDER: it runs at the clip's own rate
        // here (XAudio2's after the voice's rate conversion).
        let f = self.gains.filter();
        let s = if f > 0.0 {
            let (low, band) = &mut self.filter;
            let high = s as f32 - *low - LOW_PASS_ONE_OVER_Q * *band;
            *band += f * high;
            *low += f * *band;
            *low as bevy::audio::Sample
        } else {
            s
        };
        let (l, r) = self.gains.get();
        self.right = Some(s * r as bevy::audio::Sample);
        Some(s * l as bevy::audio::Sample)
    }
}

impl bevy::audio::Source for PanDecoder {
    fn current_span_len(&self) -> Option<usize> {
        None
    }
    fn channels(&self) -> bevy::audio::ChannelCount {
        bevy::audio::ChannelCount::new(2).unwrap()
    }
    fn sample_rate(&self) -> bevy::audio::SampleRate {
        self.inner.sample_rate()
    }
    fn total_duration(&self) -> Option<std::time::Duration> {
        self.inner.total_duration()
    }
}

impl bevy::audio::Decodable for Panned {
    type Decoder = PanDecoder;
    fn decoder(&self) -> PanDecoder {
        PanDecoder { inner: self.clip.decoder(), gains: self.gains.clone(), right: None, filter: (0.0, 0.0) }
    }
}

/// UAudioSettings LowPassFilterResonance (BaseEngine.ini 0.9): the voice filter's OneOverQ (FUN_1412d7370).
const LOW_PASS_ONE_OVER_Q: f32 = 0.9;

/// A panned voice's gains.
#[derive(Component)]
pub(crate) struct Pan(pub(crate) std::sync::Arc<PanGains>);

/// Spatialized voices (PlaybackSettings::spatial) are played panned: once its clip is in (before the engine starts
/// it), the voice's player becomes a [`Panned`] one.
fn to_panned(
    mut commands: Commands,
    clips: Res<Assets<bevy::audio::AudioSource>>,
    mut panned: ResMut<Assets<Panned>>,
    voices: Query<(Entity, &AudioPlayer, &PlaybackSettings), Without<bevy::audio::AudioSink>>,
) {
    for (e, player, settings) in &voices {
        if !settings.spatial {
            continue;
        }
        let Some(clip) = clips.get(&player.0) else { continue };
        let gains = std::sync::Arc::new(PanGains::default());
        gains.set(0.5, 0.5);
        gains.set_filter(crate::audio_volumes::default_voice_filter());
        let handle = panned.add(Panned { clip: clip.clone(), gains: gains.clone() });
        commands
            .entity(e)
            .remove::<AudioPlayer>()
            .insert((AudioPlayer::<Panned>(handle), PlaybackSettings { spatial: false, ..*settings }, Pan(gains)));
    }
}

/// X3DAudio's speaker pan (X3DAudio1_7.dll FUN_00402360) for a stereo device (speaker mask 3: left at -pi/2, right at
/// +pi/2 azimuth): of the differences azimuth - speaker (wrapped to [-pi, pi]) the smallest d+ >= 0 and the d- < 0
/// nearest zero pick the two speakers either side; t = tan(|d-| x pi/2 / (d+ + |d-|)), the d- speaker gets 1 / (1 + t)
/// and the d+ one t / (1 + t) (an amplitude pan: the gains sum to one), all to one of them when its difference is
/// under 1.19e-7. At the listener (a horizontal distance at most 1.19e-7) the gain splits equally over the speakers.
pub(crate) fn x3d_stereo(azimuth: f32, at_listener: bool) -> (f32, f32) {
    use std::f32::consts::{FRAC_PI_2, PI, TAU};
    if at_listener {
        return (0.5, 0.5);
    }
    const SPEAKERS: [f32; 2] = [-FRAC_PI_2, FRAC_PI_2];
    let (mut plus, mut plus_d, mut minus, mut minus_d) = (None, 4.0f32, None, -4.0f32);
    for (i, s) in SPEAKERS.iter().enumerate() {
        let mut d = azimuth - s;
        if d > PI {
            d -= TAU;
        } else if d < -PI {
            d += TAU;
        }
        if d <= 0.0 && d > minus_d {
            minus = Some(i);
            minus_d = d;
        }
        if d >= 0.0 && d < plus_d {
            plus = Some(i);
            plus_d = d;
        }
    }
    let (Some(neg), Some(pos)) = (minus, plus) else { return (1.0, 0.0) };
    let (dn, dp) = (-minus_d, plus_d);
    let mut g = [0.0f32; 2];
    if dn >= f32::EPSILON {
        if dp >= f32::EPSILON {
            let t = (dn * FRAC_PI_2 / (dp + dn)).tan();
            g[neg] += 1.0 / (t + 1.0);
            g[pos] += t / (t + 1.0);
        } else {
            g[pos] += 1.0;
        }
    } else {
        g[neg] += 1.0;
    }
    (g[0], g[1])
}

/// The game's XAudio2 device (WindowsEngine.ini: AudioDeviceModuleName=XAudio2) pans a 3D sound with X3DAudio
/// (XAudio2Source GetMonoChannelVolumes 0x1419539c0: X3DAudioCalculate, MATRIX | REVERB, its volume curve flat at 1,
/// InnerRadius 0) over the device's speakers (FUN_1419546b0: X3DAudioInitialize with the device's channel mask), the
/// azimuth atan2(v . right, v . front) on the listener's plane ([`x3d_stereo`]). The centre channel is then
/// max(volume x VoiceCenterChannelVolume, X3DAudio's centre) (the sound classes leave it 0) and, on a six-channel
/// device only, LFE and the front pair's pull toward the centre; the 6-channel submixes go to the device through
/// FUN_141953d50's matrix (stereo: L = FL + 0.7 FC + 1.25 BL, R = FR + 0.7 FC + 1.25 BR), so a stereo output hears
/// X3DAudio's pair as it is. PLACEHOLDER: f13-rs plays to a stereo device only (the game's 5.1 / 7.1 speaker layouts
/// aren't used).
fn pan(listener: Query<&GlobalTransform, With<SpatialListener>>, voices: Query<(&Pan, &GlobalTransform)>) {
    let Some(ear) = listener.iter().next() else { return };
    let to_listener = ear.affine().inverse();
    for (p, g) in &voices {
        let local = to_listener.transform_point3(g.translation());
        // Bevy's listener space: right +x, front -z.
        let (right, front) = (local.x, -local.z);
        let at = right.hypot(front) <= f32::EPSILON;
        let (l, r) = x3d_stereo(right.atan2(front), at);
        p.0.set(l, r);
    }
}

pub struct AudioPlugin;

impl Plugin for AudioPlugin {
    fn build(&self, app: &mut App) {
        crate::audio_settings::init(app);
        app.insert_resource(AudioRng(f13_core::spawn::Rng::new(0xa0d1_0f13)))
            .add_systems(Startup, load_sounds)
            .init_resource::<MusicState>()
            .add_systems(
                Update,
                (add_listener, modulate, attenuate, actor_sounds, counselor_music, near_music, scary_sounds, fades, ambience, voices, start_delayed, reloop, update_mixes, apply_mixes).chain(),
            )
            .add_systems(PostUpdate, (to_panned.before(TransformSystems::Propagate), pan.after(TransformSystems::Propagate)))
            .add_observer(push_mix)
            .add_observer(loop_2d)
            .add_observer(stop_2d)
            .add_observer(pop_mix)
            .add_observer(clear_mixes)
            .add_observer(play_sound)
            .add_observer(footstep)
            .add_observer(actor_sound)
            .add_observer(stop_voice_over)
            .add_observer(actor_loop)
            .add_observer(actor_cue)
            .add_observer(stop_loop)
            .add_observer(music_stinger)
            .add_observer(kill_music)
            .add_observer(dropped);
    }

    // After the engine's plugins (its asset server).
    fn finish(&self, app: &mut App) {
        app.add_audio_source::<Panned>();
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
    /// Whether `take` has sounds (an entry of its own).
    pub fn has(&self, take: &str) -> bool {
        self.0.contains_key(take)
    }

    /// The cues `take` plays after `from` (s; negative from its start), up to `to`.
    pub fn passed<'a>(&'a self, take: &str, from: f32, to: f32) -> impl Iterator<Item = &'a str> + 'a {
        self.0.get(take).into_iter().flatten().filter(move |(t, _)| from < *t && *t <= to).map(|(_, c)| c.as_str())
    }
}

/// Plays the cues a character's action passed this frame, where it stands.
/// Its SCAnimNotify_PlayCounselorVO notifies ("VO:<line>") are said by `actor`'s voice (PlayVoiceOver); its
/// SCAnimNotify_PlayCinematicMusic notifies ("MUSIC:<track>") go to `owner`'s music ([`KillMusic`]); the other cues
/// play unless `voice_only` (a paired take, whose sounds come from the other side).
#[allow(clippy::too_many_arguments)]
pub fn play_anim_sounds(commands: &mut Commands, sounds: Option<&AnimSounds>, span: Option<(&str, f32, f32)>, at: Vec3, actor: Option<Entity>, owner: Option<Entity>, voice_only: bool) {
    let (Some(sounds), Some((take, from, to))) = (sounds, span) else { return };
    for cue in sounds.passed(take, from, to) {
        if let Some(track) = cue.strip_prefix("MUSIC:") {
            if let Some(owner) = owner {
                commands.trigger(KillMusic { owner, track: track.to_string() });
            }
            continue;
        }
        if cue.starts_with("VO:") {
            if let Some(actor) = actor {
                commands.trigger(ActorSound { actor, prop: intern(cue) });
            }
            continue;
        }
        if voice_only {
            continue;
        }
        commands.trigger(PlaySound { cue: cue.to_string(), at: Some(at) });
    }
}

/// A line's name as the `&'static str` [`ActorSound`] takes (the notifies' few names, kept once each).
pub(crate) fn intern(name: &str) -> &'static str {
    static NAMES: std::sync::OnceLock<std::sync::Mutex<std::collections::HashSet<&'static str>>> = std::sync::OnceLock::new();
    let mut names = NAMES.get_or_init(Default::default).lock().unwrap();
    if let Some(n) = names.get(name) {
        return n;
    }
    let n: &'static str = Box::leak(name.to_string().into_boxed_str());
    names.insert(n);
    n
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

/// Play `cue` once on `actor` under `prop` (an AudioComponent's SetSound and Play: a radio's song), at `volume` (the
/// component's VolumeMultiplier); its voices follow the actor and [`StopLoop`] stops them.
#[derive(Event, Clone, Debug)]
pub struct ActorCue {
    pub actor: Entity,
    pub prop: &'static str,
    pub cue: String,
    pub volume: f32,
}

fn actor_cue(ev: On<ActorCue>, mut commands: Commands, assets: Res<AssetServer>, sounds: Option<ResMut<Sounds>>, mut rng: ResMut<AudioRng>) {
    let Some(mut sounds) = sounds else { return };
    let Some((voices, att)) = sounds.play(&ev.cue, &mut || rng.0.frand()) else { return };
    for v in voices {
        let volume = v.volume * ev.volume;
        let settings = PlaybackSettings { mode: mode(&v), speed: v.pitch, volume: Volume::Linear(volume), paused: v.delay > 0.0, ..PlaybackSettings::DESPAWN };
        let source = AudioPlayer::new(assets.load(format!("sounds/{}", v.file)));
        let mut e = commands.spawn((source, settings, Transform::IDENTITY, ChildOf(ev.actor), LoopOf(ev.prop), VoiceClass(v.class.clone())));
        if let Some(att) = att.clone().filter(|a| a.attenuate || a.spatialize) {
            e.insert((PlaybackSettings { spatial: att.spatialize, spatial_scale: Some(SpatialScale::new(SPATIAL_SCALE)), ..settings }, Emitter { volume, attenuation: att }));
        }
        if v.delay > 0.0 {
            e.insert(StartAfter(v.delay));
        }
    }
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
    mut rng: ResMut<AudioRng>,
    actors: Query<&ClassSounds>,
) {
    let Some(mut sounds) = sounds else { return };
    let name = actors.get(ev.actor).ok().and_then(|s| s.0.get(ev.prop).cloned()).unwrap_or_else(|| ev.prop.to_string());
    trace!("loop {} on {}", ev.prop, ev.actor);
    let Some((voices, att)) = sounds.play(&name, &mut || rng.0.frand()) else { return };
    for v in voices {
        let settings = PlaybackSettings { mode: PlaybackMode::Loop, speed: v.pitch, volume: Volume::Linear(v.volume), ..PlaybackSettings::LOOP };
        let source = AudioPlayer::new(assets.load(format!("sounds/{}", v.file)));
        let mut e = commands.spawn((source, settings, Transform::from_translation(Vec3::Y), ChildOf(ev.actor), LoopOf(ev.prop), VoiceClass(v.class.clone())));
        if let Some(att) = att.clone().filter(|a| a.attenuate || a.spatialize) {
            e.insert((PlaybackSettings { spatial: att.spatialize, spatial_scale: Some(SpatialScale::new(SPATIAL_SCALE)), ..settings }, Emitter { volume: v.volume, attenuation: att }));
        }
        if v.delay > 0.0 {
            e.insert(StartAfter(v.delay));
        }
        if !v.mods.is_empty() {
            e.insert(Modulated { volume: v.volume, pitch: v.pitch, mods: v.mods.clone() });
        }
    }
}

/// A sound's parameters (AudioComponent::SetFloatParameter): on the actor its voices play from.
#[derive(Component, Clone, Debug, Default)]
pub struct CueParams(pub HashMap<String, f32>);

/// A voice whose volume and pitch follow its sound's parameters (continuous modulators, cross-fades): its base volume
/// and pitch, and what changes them.
#[derive(Component)]
struct Modulated {
    volume: f32,
    pitch: f32,
    mods: Vec<f13_data::sound::VoiceMod>,
}

/// Applies each modulated voice's parameters (its actor's CueParams, the listener's distance) to its volume and pitch.
#[allow(clippy::type_complexity)]
fn modulate(
    listener: Query<&GlobalTransform, With<SpatialListener>>,
    params: Query<&CueParams>,
    mut voices: Query<(&Modulated, &GlobalTransform, Option<&ChildOf>, Option<&mut Emitter>, Option<&mut AudioSink>, Option<&mut SpatialAudioSink>)>,
) {
    let ear = listener_position(&listener);
    for (m, g, parent, emitter, mut sink, spatial) in &mut voices {
        let p = parent.and_then(|c| params.get(c.parent()).ok());
        let lookup = |name: &str| p.and_then(|p| p.0.get(name).copied());
        let distance_cm = ear.map_or(0.0, |e| e.distance(g.translation()) * 100.0);
        let (gain, pitch) = m.mods.iter().map(|x| x.factors(&lookup, distance_cm)).fold((1.0, 1.0), |a, b| (a.0 * b.0, a.1 * b.1));
        use bevy::audio::AudioSinkPlayback;
        // Attenuated voices: the gain goes through their emitter (attenuate sets the sink).
        if let Some(mut e) = emitter {
            e.volume = m.volume * gain;
        } else if let Some(s) = sink.as_deref_mut() {
            s.set_volume(Volume::Linear(m.volume * gain));
        }
        if let Some(s) = sink.as_deref_mut() {
            s.set_speed(m.pitch * pitch);
        }
        if let Some(s) = spatial {
            s.set_speed(m.pitch * pitch);
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
    mut rng: ResMut<AudioRng>,
) {
    let Ok((s, g)) = actors.get(ev.actor) else { return };
    let Some(cue) = s.0.get(ev.prop) else { return };
    // The doors', windows', barricades' and walls' cues play at the actor's root (PlaySoundAtLocation).
    if !ev.prop.starts_with("VO:") {
        commands.trigger(PlaySound { cue: cue.clone(), at: Some(g.translation()) });
        return;
    }
    let Some(mut sounds) = sounds else { return };
    let Some((played, att)) = sounds.play(cue, &mut || rng.0.frand()) else { return };
    debug!("voice: {} says {}", ev.actor, ev.prop);
    for (e, parent) in &voices {
        if parent.parent() == ev.actor {
            commands.entity(e).despawn();
        }
    }
    for v in played {
        let settings = PlaybackSettings {
            mode: mode(&v),
            speed: v.pitch,
            volume: Volume::Linear(v.volume),
            paused: v.delay > 0.0,
            ..PlaybackSettings::DESPAWN
        };
        let source = AudioPlayer::new(assets.load(format!("sounds/{}", v.file)));
        let mut e = commands.spawn((source, settings, Transform::from_translation(Vec3::Y * crate::player::CAPSULE_HALF_HEIGHT_M), ChildOf(ev.actor), VoiceOf(ev.prop), VoiceClass(v.class.clone())));
        if v.delay > 0.0 {
            e.insert(StartAfter(v.delay));
        }
        if v.reloop {
            e.insert(Reloop(cue.clone()));
        }
        if let Some(att) = att.clone().filter(|a| a.attenuate || a.spatialize) {
            e.insert((PlaybackSettings { spatial: att.spatialize, spatial_scale: Some(SpatialScale::new(SPATIAL_SCALE)), ..settings }, Emitter { volume: v.volume, attenuation: att }));
        }
    }
}

/// Doors, windows and barricades' own cues, where the game plays them: a door's DoorHitSoundCue or
/// DoorDestroySoundCue as it's damaged (its TakeDamage, 0x1403608c0); a window's WindowOpenSound / WindowCloseSound as
/// her move reaches it (SCWindow::NativeOpen/CloseDestinationReached); a barricade's EnableBarricadeAudio /
/// DisableBarricadeAudio as she starts using it (SCBarricade::OnUseStarted). A door's OpenDoorSound and CloseDoorSound
/// aren't played by any code or Blueprint: her and his door takes' own notifies sound them.
#[allow(clippy::type_complexity)]
fn actor_sounds(
    mut commands: Commands,
    mut last: Local<HashMap<Entity, (f32, bool)>>,
    doors: Query<(Entity, &crate::doors::Door), With<ClassSounds>>,
    window_moves: Query<&crate::interact::WindowMove, Added<crate::interact::WindowMove>>,
    barricade_moves: Query<&crate::interact::BarricadeMove, Added<crate::interact::BarricadeMove>>,
    barricades: Query<(Entity, &crate::windows::Barricade), With<ClassSounds>>,
    (windows, mut broken): (Query<(Entity, &crate::windows::Window, &GlobalTransform)>, Local<HashMap<Entity, bool>>),
    working: Query<(Entity, &crate::interact::WindowMove)>,
) {
    // Cabin_Window_BP's OnWindowDestroyed: WindowBreak where the window is, and its InteractingCounselor's Impact line
    // (PlayVoiceOver: her diving through it).
    for (e, w, g) in &windows {
        if w.destroyed && !broken.insert(e, true).unwrap_or(false) {
            commands.trigger(PlaySound { cue: "WindowBreak".into(), at: Some(g.translation()) });
            for (counselor, _) in working.iter().filter(|(_, m)| m.window == e) {
                commands.trigger(ActorSound { actor: counselor, prop: "VO:Impact" });
            }
        }
    }
    let mut play = |actor: Entity, prop: &'static str| commands.trigger(ActorSound { actor, prop });
    for (e, d) in &doors {
        let now = (d.health, d.broken);
        if let Some(&(health, broken)) = last.get(&e) {
            if d.broken && !broken {
                play(e, "DoorDestroySoundCue");
            } else if d.health < health {
                play(e, "DoorHitSoundCue");
            }
        }
        last.insert(e, now);
    }
    for m in &window_moves {
        match m.kind {
            crate::interact::WindowMoveKind::Open => play(m.window, "WindowOpenSound"),
            crate::interact::WindowMoveKind::Close => play(m.window, "WindowCloseSound"),
            _ => {}
        }
    }
    for m in &barricade_moves {
        if let Some((e, b)) = barricades.iter().find(|(_, b)| b.door == Some(m.door)) {
            play(e, if b.enabled { "DisableBarricadeAudio" } else { "EnableBarricadeAudio" });
        }
    }
}

/// A 2D (unpanned) player of a cue's first voice: the entity and the voice's volume.
fn spawn_2d(commands: &mut Commands, assets: &AssetServer, sounds: &mut Sounds, rng: &mut AudioRng, cue: &str, looping: bool, volume: f32) -> Option<(Entity, f32)> {
    let (voices, _) = sounds.play(cue, &mut || rng.0.frand())?;
    let v = voices.into_iter().next()?;
    let mode = if looping && !v.reloop { PlaybackSettings::LOOP } else { PlaybackSettings { mode: mode(&v), ..PlaybackSettings::DESPAWN } };
    let settings = PlaybackSettings { volume: Volume::Linear(v.volume * volume), speed: v.pitch, ..mode };
    let e = commands.spawn((AudioPlayer::new(assets.load(format!("sounds/{}", v.file))), settings, VoiceClass(v.class.clone()))).id();
    if v.reloop {
        commands.entity(e).insert(Reloop(cue.to_string()));
    }
    Some((e, v.volume))
}

/// The line a take of hers says (its SCAnimNotify_PlayCounselorVO; here only to tell her kill takes apart).
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

/// Her voice lines (her VoiceOverObjectClass's, as her class sounds) on what happens to her: Wounded as she becomes
/// wounded, Died (a death outside a kill); her takes' own (swings, dodges, windows, stumbles, kills, the phone, the
/// trap) are their SCAnimNotify_PlayCounselorVO notifies ([`play_anim_sounds`]), Impact the window's
/// ([`actor_sounds`]). Choke as Base_Counselor's graph says it: a second after she's grabbed
/// (BP_OnGrabbed's Delay, which a second grab while it waits doesn't restart), stopped when Jason drops her
/// (BP_OnDropped) or while she's in a special move (its tick: a kill, its take or its camera running). Her lines only while she's alive (PlayVoiceOver).
/// The hiding lines are hiding.rs's.
#[allow(clippy::type_complexity)]
fn voices(
    mut commands: Commands,
    time: Res<Time>,
    mut state: Local<HashMap<Entity, VoiceState>>,
    (chase, game, play_jason): (Res<crate::counselor_camera::Chase>, Option<Res<crate::match_flow::GameMatch>>, Option<Res<crate::jason_player::PlayJason>>),
    players: Query<(Entity, &Player, Option<&crate::character::PlayAction>, Has<crate::jason::Held>, Has<crate::controls::LocalPlayer>, Option<&crate::counselor_camera::HerChase>), With<ClassSounds>>,
    saying_now: Query<(&VoiceOf, &ChildOf)>,
    kills: Query<(), With<crate::kill_camera::KillCam>>,
) {
    state.retain(|e, _| players.contains(*e));
    for (pe, p, action, held, local, her_chase) in &players {
    let v = state.entry(pe).or_default();
    let c = &p.counselor;
    let now = (c.health, c.wounded, held, action.map(|a| a.0.clone()), p.montage.clone(), c.alive());
    let Some((_, wounded, was_held, was_action, _, was_alive)) = v.last.replace(now.clone()) else { continue };
    let alive = now.5;
    let mut say = |line: &'static str| {
        if alive || line == "VO:Died" {
            commands.trigger(ActorSound { actor: pe, prop: line });
        }
    };
    let kill = |a: &Option<String>| a.as_deref().and_then(action_line).is_some_and(|l| l.ends_with("Kill"));
    // PlayVoiceOver says Died only outside a context kill (!bInContextKill, FUN_14031bff0): any kill takes her while
    // he holds her (grab, placed, drown kills), or by her own kill take.
    let in_kill = kill(&now.3) || kill(&was_action) || held || was_held || !kills.is_empty();
    let t = time.elapsed_secs();
    if held && !was_held && v.choke_at.is_none() {
        v.choke_at = Some(t + 1.0);
    }
    if v.choke_at.is_some_and(|at| t >= at) {
        v.choke_at = None;
        say("VO:Choke");
    }
    if !now.5 && was_alive && !in_kill {
        say("VO:Died");
    }
    // MULTICAST_PlayWoundedVO (FUN_1403bf8a0): WoundedScreams, then Wounded queued after it.
    if now.1 && !wounded && now.5 {
        say("VO:WoundedScreams");
        v.queued = Some("VO:Wounded");
    } else if let Some(line) = v.queued.filter(|_| !saying_now.iter().any(|(_, c)| c.parent() == pe)) {
        v.queued = None;
        say(line);
    }
    // The chase update (FUN_1403db8f0) tells her voice (FUN_140316e40): a chase starting says ChaseStart, once a chase
    // and not before its timer (ChaseStart's cooldown, 15 s from the constructor 0x1402f4760, set as a chase ends) is out.
    let chased = her_chase.map_or(local && chase.chased, |c| c.0.chased);
    if chased && !v.chase_said && t >= v.chase_ready_at {
        v.chase_said = true;
        say("VO:ChaseStart");
    } else if !chased && v.chase_said {
        v.chase_said = false;
        v.chase_ready_at = t + CHASE_START_VO_COOLDOWN_S;
    }
    // SCPlayerController::OnSpawnIntroCompleted: her MatchStartVO (the intro isn't ported: as the match starts), when
    // she's the local player.
    if local && !v.match_said && play_jason.is_none() && game.as_ref().is_some_and(|g| g.state.state == crate::match_flow::MatchState::InProgress) {
        v.match_said = true;
        say("VO:MatchStartVO");
    }
    // (Her takes' own lines, the swings', dodges', windows', stumbles' and kills', are their notifies':
    // SCAnimNotify_PlayCounselorVO through play_anim_sounds.)
    if was_held && !held {
        commands.trigger(StopVoiceOver { actor: pe });
    } else if (kill(&now.3) || !kills.is_empty()) && saying(&saying_now, pe, "VO:Choke") {
        commands.trigger(StopVoiceOver { actor: pe });
    }
    }
}

/// One counselor's voice state: what she was last frame (health, wounded, held, action, montage, alive); her Choke's
/// time; a line waiting for hers to end (PlayVoiceOver's queue: +0x1d8); ChaseStart said this chase (+0x20d) and its
/// cooldown's end (+0x200); MatchStartVO said.
#[derive(Default)]
struct VoiceState {
    last: Option<(f32, bool, bool, Option<String>, Option<String>, bool)>,
    choke_at: Option<f32>,
    queued: Option<&'static str>,
    chase_said: bool,
    chase_ready_at: f32,
    match_said: bool,
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
    mut rng: ResMut<AudioRng>,
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
    mut rng: ResMut<AudioRng>,
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

/// The counselor's music track (its player and FadeOutTime), when the idle music is due, what has played, and a kill's
/// music while it plays.
#[derive(Resource, Default)]
pub struct MusicState {
    track: Option<(Entity, f32, f32)>,
    idle_at: Option<f32>,
    match_started: bool,
    two_minutes: bool,
    ended: bool,
    cinematic: Option<Entity>,
    /// The two-minute warning while it plays (TwoMinuteWarningTrack, IsPlaying2MinuteWarning): its player and volume.
    warning: Option<(Entity, f32)>,
}

/// StopMusic keeping the two-minute warning (its argument: IsPlaying2MinuteWarning, as PlayCinematicMusic passes it)
/// turns it down to this (SetVolumeMultiplier DAT_14209aa5c, FUN_1402fdcc0).
const WARNING_UNDER_MUSIC: f32 = 0.01;

/// SCAnimNotify_PlayCinematicMusic (its Notify 0x140314530): a kill montage's MusicTrackToPlay, for the character
/// playing it (`owner`).
#[derive(Event, Clone, Debug)]
pub struct KillMusic {
    pub owner: Entity,
    pub track: String,
}

/// The notify plays the track on the owner's MusicComponent only where the owner is locally controlled or is the one
/// the local player spectates (SCPlayerController SpectatingPlayer +0x838). PlayCinematicMusic (SCMusicComponent
/// vt+0x3c0, 0x140319300): StopMusic (vt+0x3b8: the current and next tracks stopped at once; the counselor's also fades
/// Jason's near music out), then the track in 2D (CreateSound2D, volume and pitch 1) as CurrentTrack, PlayingCinematicMusic
/// set; when it ends (CinematicKillMusicFinished 0x1402fdc20) the counselor's idle music is due again (vt+0x3d8
/// 0x14031d5b0: MinIdleMusicTime..MaxIdleMusicTime). StopMusic(IsPlaying2MinuteWarning) keeps a playing two-minute
/// warning, turned down to [`WARNING_UNDER_MUSIC`] for good. PLACEHOLDERS: Jason's music after it (JasonsMusic: only for
/// one spectating him, and Jason is spectated only in private matches) isn't ported, and her DeathMusic waits for the
/// kill's music to end (where they meet isn't read).
#[allow(clippy::too_many_arguments)]
fn kill_music(
    ev: On<KillMusic>,
    mut commands: Commands,
    assets: Res<AssetServer>,
    sounds: Option<ResMut<Sounds>>,
    mut state: ResMut<MusicState>,
    mut rng: ResMut<AudioRng>,
    local: Query<(), With<crate::controls::LocalPlayer>>,
    jasons: Query<(), With<crate::jason::Jason>>,
    (play_jason, spectate): (Option<Res<crate::jason_player::PlayJason>>, Option<Res<crate::spectate::Spectate>>),
    mut sinks: Query<&mut bevy::audio::AudioSink>,
) {
    let Some(mut sounds) = sounds else { return };
    let mine = local.contains(ev.owner) || (play_jason.is_some() && jasons.contains(ev.owner)) || spectate.as_ref().and_then(|s| s.viewed()) == Some(ev.owner);
    if !mine {
        return;
    }
    state.fade_out(&mut commands, 0.0);
    if let Some((e, base)) = state.warning {
        if let Ok(mut s) = sinks.get_mut(e) {
            s.set_volume(Volume::Linear(base * WARNING_UNDER_MUSIC));
        }
    }
    if let Some(e) = state.cinematic.take() {
        if let Ok(mut ec) = commands.get_entity(e) {
            ec.despawn();
        }
    }
    if let Some((e, _)) = spawn_2d(&mut commands, &assets, &mut sounds, &mut rng, &ev.track, false, 1.0) {
        state.cinematic = Some(e);
        state.idle_at = None;
        info!("music: kill {}", ev.track);
    }
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
    (game, escaped): (Option<Res<crate::match_flow::GameMatch>>, Option<Single<&crate::vehicle::Escaped, With<crate::controls::LocalPlayer>>>),
    mut rng: ResMut<AudioRng>,
    players: Query<&Player, With<crate::controls::LocalPlayer>>,
    playing: Query<Entity>,
) {
    let (Some(mut sounds), Some(counselor)) = (sounds, counselor) else { return };
    let music = counselor.info.music.clone();
    if music.is_null() {
        return;
    }
    let now = time.elapsed_secs();
    let idle = |rng: &mut AudioRng| {
        let lo = music["MinIdleMusicTime"].as_f64().unwrap_or(5.0) as f32;
        let hi = music["MaxIdleMusicTime"].as_f64().unwrap_or(10.0) as f32;
        now + lo + (hi - lo) * rng.0.frand()
    };
    // A kill's music over (CinematicKillMusicFinished): the idle music a while later.
    if let Some(e) = state.cinematic {
        if playing.get(e).is_err() {
            state.cinematic = None;
            if state.track.is_none() {
                state.idle_at = Some(idle(&mut rng));
            }
        } else {
            return;
        }
    }
    // A finished track (checked before anything starts: a new player spawns at the end of the frame): the idle music
    // a while later.
    if let Some((e, _, _)) = state.track {
        if playing.get(e).is_err() {
            state.track = None;
            state.idle_at = Some(idle(&mut rng));
        }
    }
    // PlayMusic(event), for a MUSICTRACK entry.
    let play = |state: &mut MusicState, commands: &mut Commands, sounds: &mut Sounds, rng: &mut AudioRng, event: &str| {
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
            state.warning = spawn_2d(&mut commands, &assets, &mut sounds, &mut rng, &cue, false, 1.0);
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
    mut rng: ResMut<AudioRng>,
    jasons: Query<(&crate::jason::Jason, &GlobalTransform)>,
    players: Query<(&Player, &GlobalTransform), With<crate::controls::LocalPlayer>>,
    (mut sinks, mut state): (Query<&mut bevy::audio::AudioSink>, ResMut<MusicState>),
) {
    let (Some(mut sounds), Some(killer)) = (sounds, killer) else { return };
    let Some(cue) = killer.info.sounds.get("NearCounselorMusic").and_then(|s| cue_name(&serde_json::Value::String(s.clone()))) else { return };
    let Some((jason, jxf)) = jasons.iter().next() else { return };
    let Ok((p, pxf)) = players.single() else { return };
    if music.player.is_none() {
        let Some((voices, _)) = sounds.play(&cue, &mut || rng.0.frand()) else { return };
        let Some(v) = voices.into_iter().next() else { return };
        let e = commands
            .spawn((AudioPlayer::new(assets.load(format!("sounds/{}", v.file))), PlaybackSettings { volume: Volume::Linear(0.0), speed: v.pitch, ..PlaybackSettings::LOOP }, VoiceClass(v.class.clone())))
            .id();
        music.player = Some((e, v.volume));
    }
    let v = |k: &str, d: f32| killer.info.values.get(k).copied().unwrap_or(d);
    let distance = jxf.translation().distance(pxf.translation()) * 100.0;
    // (And faded out while a kill's music plays: StopMusic.)
    let silenced = jason.stalking() || !p.counselor.alive() || matches!(jason.state, crate::jason::JasonState::Done) || state.cinematic.is_some();
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
    /// Each cue's random nodes' state (USoundNodeRandom keeps it on the node: across the cue's plays).
    random: HashMap<String, f13_data::sound::RandomState>,
}

impl Sounds {
    /// One play of the cue `name`: its voices and attenuation.
    fn play(&mut self, name: &str, rand: &mut dyn FnMut() -> f32) -> Option<(Vec<f13_data::sound::Voice>, Option<f13_data::sound::Attenuation>)> {
        let c = self.cue(name)?.clone();
        Some(c.play_with(rand, self.random.entry(name.to_string()).or_default()))
    }

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
    pub(crate) fn terrain_surface(&self, x: f32, z: f32) -> u8 {
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
    commands.insert_resource(SoundMixes::load(&dir));
    commands.insert_resource(Sounds { dir, cues: HashMap::new(), random: HashMap::new(), footsteps, jason_footsteps, terrain: load_terrain(&export.0, &level) });
}

/// A voice's sound class (its cue's SoundClassObject): the classes' volume and pitch, and the sound mixes pushed,
/// scale it ([`apply_mixes`]).
#[derive(Component, Clone, Debug)]
pub struct VoiceClass(pub Option<String>);

/// What [`apply_mixes`] last wrote to a voice (volume, speed) and by which factors.
#[derive(Component)]
struct MixApplied {
    volume: f32,
    speed: f32,
    factors: (f32, f32),
}

/// A sound mix's settings (USoundMix): its timing and its class adjusters (class, volume, pitch, bApplyToChildren).
#[derive(Clone, Debug)]
struct MixDef {
    initial_delay: f32,
    fade_in: f32,
    duration: f32,
    fade_out: f32,
    effects: Vec<(String, f32, f32, bool)>,
}

/// A pushed mix (FSoundMixState): its time since pushed, its pushes not yet popped, and the time into its fade out.
#[derive(Clone, Debug)]
struct ActiveMix {
    name: String,
    t: f32,
    count: u32,
    out: Option<f32>,
}

/// The sound classes (parent, Volume, Pitch: sounds/sound_classes.json), the mixes (sound_mixes.json) and those pushed.
#[derive(Resource, Default)]
pub struct SoundMixes {
    classes: HashMap<String, (Option<String>, f32, f32)>,
    /// The classes with bApplyAmbientVolumes (the audio volumes' interior settings apply to their sounds).
    ambient: std::collections::HashSet<String>,
    mixes: HashMap<String, MixDef>,
    active: Vec<ActiveMix>,
}

impl SoundMixes {
    fn load(dir: &std::path::Path) -> Self {
        let read = |f: &str| std::fs::read_to_string(dir.join(f)).ok().and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok());
        let mut s = SoundMixes::default();
        match read("sound_classes.json") {
            Some(v) => {
                for (k, c) in v.as_object().into_iter().flatten() {
                    let n = |f: &str| c[f].as_f64().unwrap_or(1.0) as f32;
                    s.classes.insert(k.clone(), (c["Parent"].as_str().map(str::to_string), n("Volume"), n("Pitch")));
                    if c["ApplyAmbientVolumes"].as_bool() == Some(true) {
                        s.ambient.insert(k.clone());
                    }
                }
            }
            None => warn!("no sounds/sound_classes.json (re-export with tools/f13-export sounds): no class volumes or mixes"),
        }
        for (k, m) in read("sound_mixes.json").as_ref().and_then(|v| v.as_object()).into_iter().flatten() {
            let n = |f: &str, d: f64| m[f].as_f64().unwrap_or(d) as f32;
            let effects = m["Effects"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|e| Some((e["Class"].as_str()?.to_string(), e["Volume"].as_f64().unwrap_or(1.0) as f32, e["Pitch"].as_f64().unwrap_or(1.0) as f32, e["ApplyToChildren"].as_bool().unwrap_or(false))))
                .collect();
            s.mixes.insert(k.clone(), MixDef { initial_delay: n("InitialDelay", 0.0), fade_in: n("FadeInTime", 0.2), duration: n("Duration", -1.0), fade_out: n("FadeOutTime", 0.2), effects });
        }
        s
    }

    /// FSoundMixState's interpolation: 0 during the initial delay, rising over the fade in, 1, falling over the fade
    /// out (a popped mix, or one past its Duration).
    fn interp(def: &MixDef, m: &ActiveMix) -> f32 {
        let t = m.t - def.initial_delay;
        let fade_in = if t <= 0.0 { 0.0 } else if def.fade_in > 0.0 { (t / def.fade_in).min(1.0) } else { 1.0 };
        match m.out {
            Some(o) => fade_in * if def.fade_out > 0.0 { (1.0 - o / def.fade_out).max(0.0) } else { 0.0 },
            None => fade_in,
        }
    }

    /// Whether a sound of `class` takes the audio volumes' interior settings (its class's bApplyAmbientVolumes: FActiveSound
    /// 0x1411fc240 sets bApplyInteriorVolumes from it).
    pub(crate) fn applies_ambient_volumes(&self, class: &str) -> bool {
        self.ambient.contains(class)
    }

    /// A class's Volume set (the audio settings: audio_settings.rs).
    pub(crate) fn set_class_volume(&mut self, class: &str, volume: f32) {
        if let Some(c) = self.classes.get_mut(class) {
            c.1 = volume;
        }
    }

    /// Whether `class` is `ancestor` or under it.
    fn is_under(&self, class: &str, ancestor: &str) -> bool {
        let mut c = Some(class.to_string());
        for _ in 0..32 {
            match c {
                Some(ref n) if n == ancestor => return true,
                Some(ref n) => c = self.classes.get(n).and_then(|x| x.0.clone()),
                None => return false,
            }
        }
        false
    }

    /// A class's volume and pitch (FAudioDevice::UpdateSoundClassProperties): its own and its parents' properties
    /// multiplied down the hierarchy, then each active mix's adjusters for it (or for an ancestor that applies to its
    /// children), each lerped from 1 by the mix's interpolation.
    pub(crate) fn factors(&self, class: &str) -> (f32, f32) {
        let (mut v, mut p) = (1.0, 1.0);
        let mut c = Some(class.to_string());
        for _ in 0..32 {
            let Some(n) = c else { break };
            let Some((parent, cv, cp)) = self.classes.get(&n) else { break };
            v *= cv;
            p *= cp;
            c = parent.clone();
        }
        for m in &self.active {
            let Some(def) = self.mixes.get(&m.name) else { continue };
            let a = Self::interp(def, m);
            for (target, av, ap, children) in &def.effects {
                if target == class || (*children && self.is_under(class, target)) {
                    v *= 1.0 + (av - 1.0) * a;
                    p *= 1.0 + (ap - 1.0) * a;
                }
            }
        }
        (v, p)
    }
}

/// A 2D sound kept by a key (GameplayStatics::CreateSound2D, then its AudioComponent's FadeIn(time, 1)).
#[derive(Event, Clone, Copy, Debug)]
pub struct Loop2D {
    pub key: &'static str,
    pub cue: &'static str,
    pub fade_in: f32,
}

/// The keyed 2D sound's AudioComponent::FadeOut(time, 0): it fades out and stops.
#[derive(Event, Clone, Copy, Debug)]
pub struct Stop2D {
    pub key: &'static str,
    pub fade_out: f32,
}

#[derive(Component)]
struct Keyed2D(&'static str);

fn loop_2d(ev: On<Loop2D>, mut commands: Commands, assets: Res<AssetServer>, sounds: Option<ResMut<Sounds>>, mut rng: ResMut<AudioRng>) {
    let Some(mut sounds) = sounds else { return };
    if let Some((e, volume)) = spawn_2d(&mut commands, &assets, &mut sounds, &mut rng, ev.cue, true, 0.0) {
        let rate = if ev.fade_in > 0.0 { 1.0 / ev.fade_in } else { f32::MAX };
        commands.entity(e).insert((Keyed2D(ev.key), Fade { base: volume, level: 0.0, target: 1.0, rate, despawn: false }));
    }
}

fn stop_2d(ev: On<Stop2D>, mut commands: Commands, mut keyed: Query<(Entity, &Keyed2D, Option<&mut Fade>)>) {
    for (e, k, fade) in &mut keyed {
        if k.0 != ev.key {
            continue;
        }
        match fade {
            Some(mut f) => {
                f.target = 0.0;
                f.rate = if ev.fade_out > 0.0 { 1.0 / ev.fade_out } else { f32::MAX };
                f.despawn = true;
            }
            None => commands.entity(e).despawn(),
        }
    }
}

/// UGameplayStatics::PushSoundMixModifier: the mix starts (or counts another push).
#[derive(Event, Clone, Copy, Debug)]
pub struct PushMix(pub &'static str);

/// UGameplayStatics::PopSoundMixModifier: one push undone; the last starts its fade out.
#[derive(Event, Clone, Copy, Debug)]
pub struct PopMix(pub &'static str);

/// UGameplayStatics::ClearSoundMixModifiers: every mix fades out.
#[derive(Event, Clone, Copy, Debug)]
pub struct ClearMixes;

fn push_mix(ev: On<PushMix>, mut mixes: ResMut<SoundMixes>) {
    match mixes.active.iter_mut().find(|m| m.name == ev.0) {
        Some(m) => {
            m.count += 1;
            m.out = None;
        }
        None => mixes.active.push(ActiveMix { name: ev.0.to_string(), t: 0.0, count: 1, out: None }),
    }
}

fn pop_mix(ev: On<PopMix>, mut mixes: ResMut<SoundMixes>) {
    if let Some(m) = mixes.active.iter_mut().find(|m| m.name == ev.0) {
        m.count = m.count.saturating_sub(1);
        if m.count == 0 && m.out.is_none() {
            m.out = Some(0.0);
        }
    }
}

fn clear_mixes(_: On<ClearMixes>, mut mixes: ResMut<SoundMixes>) {
    for m in &mut mixes.active {
        m.count = 0;
        m.out.get_or_insert(0.0);
    }
}

/// The pushed mixes' clocks: past its Duration a mix fades out; one faded out is gone.
fn update_mixes(time: Res<Time>, mut mixes: ResMut<SoundMixes>) {
    let dt = time.delta_secs();
    let s = &mut *mixes;
    for m in &mut s.active {
        m.t += dt;
        if let Some(o) = m.out.as_mut() {
            *o += dt;
        }
        if let Some(def) = s.mixes.get(&m.name) {
            if def.duration >= 0.0 && m.out.is_none() && m.t >= def.initial_delay + def.fade_in + def.duration {
                m.out = Some(0.0);
            }
        }
    }
    let defs = &s.mixes;
    s.active.retain(|m| match (m.out, defs.get(&m.name)) {
        (Some(o), Some(d)) => o < d.fade_out,
        (Some(_), None) => false,
        _ => true,
    });
}

/// Each voice's volume and speed scaled by its class's factors (the other systems set them, or set them once at
/// the start: a value this pass wrote is taken back to what it was before).
fn apply_mixes(
    mut commands: Commands,
    mixes: Res<SoundMixes>,
    mut voices: Query<(Entity, &VoiceClass, Option<&mut AudioSink>, Option<&mut SpatialAudioSink>, Option<&MixApplied>)>,
) {
    use bevy::audio::AudioSinkPlayback;
    for (e, class, sink, spatial, applied) in &mut voices {
        let (fv, fp) = class.0.as_deref().map_or((1.0, 1.0), |c| mixes.factors(c));
        let sink: &mut dyn AudioSinkPlayback = match (sink, spatial) {
            (Some(s), _) => s.into_inner(),
            (None, Some(s)) => s.into_inner(),
            _ => continue,
        };
        let (vol, speed) = (sink.volume().to_linear(), sink.speed());
        let (base_v, base_s) = match applied {
            Some(a) if a.volume == vol && a.speed == speed => (vol / a.factors.0.max(1e-6), speed / a.factors.1.max(1e-6)),
            _ => (vol, speed),
        };
        if applied.is_some_and(|a| a.factors == (fv, fp) && a.volume == vol && a.speed == speed) {
            continue;
        }
        let (nv, ns) = (base_v * fv, (base_s * fp).max(1e-3));
        sink.set_volume(Volume::Linear(nv));
        sink.set_speed(ns);
        commands.entity(e).insert(MixApplied { volume: sink.volume().to_linear(), speed: sink.speed(), factors: (fv, fp) });
    }
}

/// The camera hears.
fn add_listener(mut commands: Commands, cameras: Query<Entity, (With<Camera3d>, Without<SpatialListener>, Without<crate::sense_view::SenseMaskCamera>, Without<crate::counselor_preview::PreviewCamera>)>) {
    for e in &cameras {
        commands.entity(e).insert(SpatialListener::new(EAR_GAP_M));
    }
}

/// A placed sound's base volume and how it fades.
#[derive(Component)]
pub(crate) struct Emitter {
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
    mut rng: ResMut<AudioRng>,
    listener: Query<&GlobalTransform, With<SpatialListener>>,
) {
    let Some(mut sounds) = sounds else { return };
    spawn_cue(&mut commands, &assets, &mut sounds, &mut rng, listener_position(&listener), &ev.cue, ev.at);
}

/// Plays a cue (PlaySoundAtLocation at `at`, else 2D): its voices.
fn spawn_cue(commands: &mut Commands, assets: &AssetServer, sounds: &mut Sounds, rng: &mut AudioRng, ear: Option<Vec3>, name: &str, at: Option<Vec3>) -> Vec<Entity> {
    trace!("play {name} at {at:?}");
    let Some((voices, att)) = sounds.play(name, &mut || rng.0.frand()) else { return Vec::new() };
    let mut spawned = Vec::new();
    for v in voices {
        let mut settings = PlaybackSettings {
            mode: mode(&v),
            speed: v.pitch,
            paused: v.delay > 0.0,
            ..PlaybackSettings::DESPAWN
        };
        let source = AudioPlayer::new(assets.load(format!("sounds/{}", v.file)));
        let e = match (at, att.clone().filter(|a| a.attenuate || a.spatialize)) {
            (Some(at), Some(att)) => {
                let factor = ear.map_or(1.0, |e| att.factor(e.distance(at) * 100.0));
                settings.volume = Volume::Linear(v.volume * factor);
                settings.spatial = att.spatialize;
                settings.spatial_scale = Some(SpatialScale::new(SPATIAL_SCALE));
                commands.spawn((source, settings, Transform::from_translation(at), Emitter { volume: v.volume, attenuation: att }, VoiceClass(v.class.clone()))).id()
            }
            _ => {
                settings.volume = Volume::Linear(v.volume);
                commands.spawn((source, settings, VoiceClass(v.class.clone()))).id()
            }
        };
        if v.delay > 0.0 {
            commands.entity(e).insert(StartAfter(v.delay));
        }
        if v.reloop {
            commands.entity(e).insert(Reloop(name.to_string()));
        }
        spawned.push(e);
    }
    spawned
}

/// SCVoiceOverComponent_Counselor's ChaseStart cooldown (+0x208, constructor 0x1402f4760).
const CHASE_START_VO_COOLDOWN_S: f32 = 15.0;

/// A scary sound's voice (SCMusicComponent_Counselor's ScarySounds component).
#[derive(Component)]
struct ScaryVoice;

/// SCMusicComponent_Counselor's scary sounds: its BeginPlay (FUN_14031d130) loops a 0.1 s timer on FUN_1402fcb30,
/// which, for her when she's the local player (and alive), with her fear at least ScarySoundsFearLevel, no scary sound
/// playing and its cooldown timer out, plays Indoor_ or Outdoor_ScarySounds_Sounds (IsIndoors) with
/// SpawnSoundAtLocation at her root offset by a random X and Y each in ScarySoundsHearingRadiusMin..Max (cm), its Z the
/// root's added to itself (the game's sum: twice her height above the world's zero), then sets the cooldown:
/// rand(ScarySoundCooldownMin..Max) x ScarySoundCooldownCurve(fear / 100). SCMusicObject_Counselor's constructor
/// (0x1402f4140): fear level 70, radius -1000..1000, cooldown 10..20 s; MusicCounselor sets the curve and the cues.
#[allow(clippy::too_many_arguments)]
fn scary_sounds(
    mut commands: Commands,
    time: Res<Time>,
    assets: Res<AssetServer>,
    sounds: Option<ResMut<Sounds>>,
    mut rng: ResMut<AudioRng>,
    (counselor, play_jason, surroundings): (Option<Res<crate::character::CounselorCharacter>>, Option<Res<crate::jason_player::PlayJason>>, Option<Res<crate::fear_events::Surroundings>>),
    players: Query<(&Player, &Transform), With<crate::controls::LocalPlayer>>,
    listener: Query<&GlobalTransform, With<SpatialListener>>,
    playing: Query<(), With<ScaryVoice>>,
    mut state: Local<(f32, f32)>,
) {
    let (Some(mut sounds), Some(counselor)) = (sounds, counselor) else { return };
    let now = time.elapsed_secs();
    // The 0.1 s timer.
    if now < state.0 {
        return;
    }
    state.0 = now + 0.1;
    let Ok((p, xf)) = players.single() else { return };
    let music = &counselor.info.music;
    let v = |k: &str, d: f32| music[k].as_f64().map_or(d, |x| x as f32);
    if play_jason.is_some() || !p.counselor.alive() || p.fear < v("ScarySoundsFearLevel", 70.0) || !playing.is_empty() || now < state.1 {
        return;
    }
    let (lo, hi) = (v("ScarySoundsHearingRadiusMin", -1000.0), v("ScarySoundsHearingRadiusMax", 1000.0));
    let (dx, dy) = (rng.0.frand() * (hi - lo) + lo, rng.0.frand() * (hi - lo) + lo);
    let indoors = surroundings.as_ref().is_some_and(|s| s.indoors(xf.translation));
    let key = if indoors { "Indoor_ScarySounds_Sounds" } else { "Outdoor_ScarySounds_Sounds" };
    let Some(cue) = cue_name(&music[key]) else { return };
    // Her root (the capsule's centre), in Unreal's Z: doubled.
    let root_z = xf.translation.y + crate::player::CAPSULE_HALF_HEIGHT_M;
    let at = Vec3::new(xf.translation.x + dx / 100.0, 2.0 * root_z, xf.translation.z + dy / 100.0);
    debug!("scary sound {cue} at {at:.1} (fear {:.0})", p.fear);
    for e in spawn_cue(&mut commands, &assets, &mut sounds, &mut rng, listener_position(&listener), &cue, Some(at)) {
        commands.entity(e).insert(ScaryVoice);
    }
    if let Some(curve) = counselor.info.curve("ScarySoundCooldownCurve") {
        let (cmin, cmax) = (v("ScarySoundCooldownMin", 10.0), v("ScarySoundCooldownMax", 20.0));
        state.1 = now + (rng.0.frand() * (cmax - cmin) + cmin) * curve.eval(p.fear * 0.01);
    }
}

/// A voice waiting out its cue's Delay nodes (spawned paused).
#[derive(Component)]
struct StartAfter(f32);

/// How a voice plays: once and kept (its cue's looping part goes round again: [`Reloop`]), looped, or once.
fn mode(v: &f13_data::sound::Voice) -> PlaybackMode {
    if v.reloop {
        PlaybackMode::Once
    } else if v.looping {
        PlaybackMode::Loop
    } else {
        PlaybackMode::Despawn
    }
}

/// A voice under a Looping node with a random choice in it (the cue named): as it ends, USoundNodeLooping parses its
/// child again, so the next time round is a new pick (Vanessa's struggle takes, the counselor's idle music).
#[derive(Component)]
struct Reloop(String);

fn reloop(
    mut commands: Commands,
    assets: Res<AssetServer>,
    sounds: Option<ResMut<Sounds>>,
    mut rng: ResMut<AudioRng>,
    voices: Query<(Entity, &Reloop, &PlaybackSettings, Option<&AudioSink>, Option<&SpatialAudioSink>, Option<&Pan>), Without<StartAfter>>,
) {
    use bevy::audio::AudioSinkPlayback;
    let Some(mut sounds) = sounds else { return };
    for (e, r, settings, sink, spatial, pan) in &voices {
        let ended = sink.map(|s| s.empty()).or(spatial.map(|s| s.empty())).unwrap_or(false);
        if !ended {
            continue;
        }
        let Some((played, _)) = sounds.play(&r.0, &mut || rng.0.frand()) else { continue };
        let Some(v) = played.into_iter().find(|v| v.reloop) else { continue };
        trace!("reloop {} with {}", r.0, v.file);
        // A panned voice ([`to_panned`]) drops its panned player (left on, it played its first take again each time
        // round) and is panned again with the new take.
        let mut ec = commands.entity(e);
        ec.remove::<(AudioSink, SpatialAudioSink)>().insert(AudioPlayer::new(assets.load(format!("sounds/{}", v.file))));
        if pan.is_some() {
            ec.remove::<(AudioPlayer<Panned>, Pan)>().insert(PlaybackSettings { spatial: true, ..*settings });
        }
    }
}

fn start_delayed(
    mut commands: Commands,
    (time, real, virtual_time): (Res<Time>, Res<Time<Real>>, Res<Time<Virtual>>),
    mut voices: Query<(Entity, &mut StartAfter, Option<&AudioSink>, Option<&SpatialAudioSink>)>,
) {
    // While a cinematic holds the match's clock, its cues' delays run on its own (real time at the game's speed): the
    // outro's Outro_Random starts Pamela's lines 2.185 s and 10 s in, as the shots reach her.
    let dt = if virtual_time.is_paused() { real.delta_secs() * virtual_time.relative_speed() } else { time.delta_secs() };
    for (e, mut s, sink, spatial) in &mut voices {
        s.0 -= dt;
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
#[allow(clippy::type_complexity)]
#[allow(clippy::type_complexity)]
pub(crate) fn attenuate(
    listener: Query<&GlobalTransform, With<SpatialListener>>,
    mut emitters: Query<(&Emitter, &GlobalTransform, Option<&crate::audio_volumes::InteriorGain>, Option<&mut AudioSink>, Option<&mut SpatialAudioSink>)>,
) {
    use bevy::audio::AudioSinkPlayback;
    let Some(ear) = listener_position(&listener) else { return };
    for (e, g, interior, sink, spatial) in &mut emitters {
        // x the audio volumes' InteriorVolumeMultiplier (audio_volumes.rs).
        let interior = interior.map_or(1.0, |i| i.0);
        let volume = Volume::Linear(e.volume * e.attenuation.factor(ear.distance(g.translation()) * 100.0) * interior);
        if let Some(mut s) = sink {
            s.set_volume(volume);
        } else if let Some(mut s) = spatial {
            s.set_volume(volume);
        }
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
    water: Option<Res<crate::water::Water>>,
) {
    let (Some(sounds), Some(collision), Some(dynamic)) = (sounds, collision, dynamic) else { return };
    // Hers, or Jason's (his table; bIsRunning).
    let (tables, g, gait) = if let Ok((p, g)) = players.get(ev.owner) {
        let c = &p.counselor;
        (sounds.footsteps.as_ref(), g, if ev.land { "land" } else if c.running || c.sprinting { "run" } else { "walk" })
    } else if let Ok((j, g)) = jasons.get(ev.owner) {
        (sounds.jason_footsteps.as_ref(), g, if j.running { "run" } else { "walk" })
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

#[cfg(test)]
mod tests {
    use super::x3d_stereo;
    use std::f32::consts::{FRAC_PI_2, FRAC_PI_4, FRAC_PI_8, PI};

    #[test]
    fn x3d_stereo_pan() {
        let near = |a: (f32, f32), b: (f32, f32)| (a.0 - b.0).abs() < 1e-5 && (a.1 - b.1).abs() < 1e-5;
        // Ahead and behind: half each (an amplitude pan, -6 dB a side).
        assert!(near(x3d_stereo(0.0, false), (0.5, 0.5)));
        assert!(near(x3d_stereo(PI, false), (0.5, 0.5)));
        // On a speaker: all to it.
        assert!(near(x3d_stereo(FRAC_PI_2, false), (0.0, 1.0)));
        assert!(near(x3d_stereo(-FRAC_PI_2, false), (1.0, 0.0)));
        // Front right: t = tan(pi/8).
        let t = FRAC_PI_8.tan();
        assert!(near(x3d_stereo(FRAC_PI_4, false), (t / (1.0 + t), 1.0 / (1.0 + t))));
        // Back right mirrors front right.
        assert!(near(x3d_stereo(3.0 * FRAC_PI_4, false), x3d_stereo(FRAC_PI_4, false)));
        assert!(near(x3d_stereo(1.0, true), (0.5, 0.5)));
    }
}
