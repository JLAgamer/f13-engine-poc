//! The audio volumes' interior settings (MODLOG "Audio volumes"): inside a cabin's AudioVolume, the sounds whose class
//! has bApplyAmbientVolumes are quieter and low-passed by where they are. From the binary:
//! - FAudioDevice::GetAudioVolumeSettings (0x1412d60d0): the highest-priority volume that encompasses the point, else
//!   the world's (AudioVolumeID 0, bIsWorldSettings);
//! - FListener::ApplyInteriorSettings (0x1412cc970) when the listener's volume or its settings change: a transition
//!   from now to now + the new settings' Interior / Exterior (LPF) times (the old settings' when the new are the
//!   world's), and FAudioDevice::Update's interpolation (0x1412e4d20): each factor (now - start) / (end - start) in 0..1;
//! - FActiveSound::HandleInteriorVolumes (0x1411efeb0): the sound's volume and LPF eased from where they were when the
//!   listener's transition began toward the target: 1 and no filter when both are in the same volume (or the sound
//!   isn't spatialized); the listener's ExteriorVolume / ExteriorLPF when the sound is in the world's; else the
//!   sound's volume's InteriorVolume times the listener's ExteriorVolume, and the lower of the two LPFs;
//! - FXAudio2SoundSource's filter (0x141957900): the lowest of the sound's LPFs (FUN_1412e0a70; only this one is
//!   ported) as XAUDIO2_FILTER_PARAMETERS Frequency 2 f / 44100 (the device's rate), clamped to 0..1.

use bevy::audio::SpatialListener;
use bevy::prelude::*;
use f13_core::collision::Tri;

use crate::audio::{Pan, SoundMixes, VoiceClass};

pub struct AudioVolumesPlugin;

impl Plugin for AudioVolumesPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<ListenerInterior>()
            .add_systems(Startup, load_volumes)
            .add_systems(Update, (listener_interior, voice_interiors).chain().before(crate::audio::attenuate));
    }
}

/// MAX_FILTER_FREQUENCY (DAT_1425adbac).
const MAX_FILTER_FREQUENCY: f32 = 20000.0;
/// The XAudio2 device's sample rate (the mastering voice's format, 44100).
const DEVICE_SAMPLE_RATE: f32 = 44100.0;

/// The source voice's filter Frequency for an LPF (FXAudio2SoundSource 0x141957900: clamp(2 x LPF / the device's
/// rate, 0, 1)). A voice starts at MAX_FILTER_FREQUENCY with its last LPF FLT_MAX (0x141955d90), so even an
/// unfiltered voice is set to 2 x 20000 / 44100, which XAudio2 doesn't bypass.
pub(crate) fn voice_filter(lpf: f32) -> f32 {
    (2.0 * lpf / DEVICE_SAMPLE_RATE).clamp(0.0, 1.0)
}

/// [`voice_filter`] for a voice with no LPF of its own.
pub(crate) fn default_voice_filter() -> f32 {
    voice_filter(MAX_FILTER_FREQUENCY)
}

/// FInteriorSettings (its constructor 0x1412c86f0 gives the defaults).
#[derive(Clone, Copy, PartialEq, Debug)]
struct InteriorSettings {
    world: bool,
    exterior_volume: f32,
    exterior_time: f32,
    exterior_lpf: f32,
    exterior_lpf_time: f32,
    interior_volume: f32,
    interior_time: f32,
    interior_lpf: f32,
    interior_lpf_time: f32,
}

impl Default for InteriorSettings {
    fn default() -> Self {
        InteriorSettings {
            world: false,
            exterior_volume: 1.0,
            exterior_time: 0.5,
            exterior_lpf: MAX_FILTER_FREQUENCY,
            exterior_lpf_time: 0.5,
            interior_volume: 1.0,
            interior_time: 0.5,
            interior_lpf: MAX_FILTER_FREQUENCY,
            interior_lpf_time: 0.5,
        }
    }
}

impl InteriorSettings {
    fn from_props(v: &serde_json::Value) -> Self {
        let d = InteriorSettings::default();
        let f = |k: &str, x: f32| v[k].as_f64().map_or(x, |y| y as f32);
        InteriorSettings {
            world: v["bIsWorldSettings"].as_bool().unwrap_or(false),
            exterior_volume: f("ExteriorVolume", d.exterior_volume),
            exterior_time: f("ExteriorTime", d.exterior_time),
            exterior_lpf: f("ExteriorLPF", d.exterior_lpf),
            exterior_lpf_time: f("ExteriorLPFTime", d.exterior_lpf_time),
            interior_volume: f("InteriorVolume", d.interior_volume),
            interior_time: f("InteriorTime", d.interior_time),
            interior_lpf: f("InteriorLPF", d.interior_lpf),
            interior_lpf_time: f("InteriorLPFTime", d.interior_lpf_time),
        }
    }

    /// The world's (AWorldSettings DefaultAmbientZoneSettings: bIsWorldSettings, the rest default; no Crystal Lake
    /// world settings override it).
    fn world() -> Self {
        InteriorSettings { world: true, ..default() }
    }
}

/// An AudioVolume: its id (0 is the world's), priority, brush hulls (Bevy space) and interior settings.
struct Volume {
    id: u32,
    priority: f32,
    hulls: Vec<Vec<Tri>>,
    settings: InteriorSettings,
}

/// The level's audio volumes, highest priority first.
#[derive(Resource, Default)]
struct AudioVolumes(Vec<Volume>);

impl AudioVolumes {
    /// GetAudioVolumeSettings: the id and settings of the volume `p` is in.
    fn at(&self, p: Vec3) -> (u32, InteriorSettings) {
        self.0.iter().find(|v| v.hulls.iter().any(|h| inside(h, p))).map_or((0, InteriorSettings::world()), |v| (v.id, v.settings))
    }
}

/// Whether `p` is inside a convex hull's triangles (behind every face, by the hull's centre's side of it).
fn inside(hull: &[Tri], p: Vec3) -> bool {
    if hull.is_empty() {
        return false;
    }
    let centre = hull.iter().map(|t| t.a + t.b + t.c).sum::<Vec3>() / (hull.len() as f32 * 3.0);
    hull.iter().all(|t| {
        let n = (t.b - t.a).cross(t.c - t.a);
        let outward = if n.dot(centre - t.a) > 0.0 { -n } else { n };
        outward.dot(p - t.a) <= 0.0
    })
}

fn load_volumes(mut commands: Commands, level: Res<crate::Level>) {
    let mut volumes = Vec::new();
    for a in level.layouts.iter().flat_map(|l| &l.actors).filter(|a| a.class == "AudioVolume") {
        let hulls: Vec<Vec<Tri>> = a.hulls.iter().map(|h| crate::collision::hull_from_unreal(h)).filter(|h| !h.is_empty()).collect();
        if hulls.is_empty() {
            continue;
        }
        let priority = a.props.get("Priority").and_then(|v| v.as_f64()).unwrap_or(0.0) as f32;
        let settings = a.props.get("AmbientZoneSettings").map_or_else(InteriorSettings::default, InteriorSettings::from_props);
        volumes.push(Volume { id: volumes.len() as u32 + 1, priority, hulls, settings });
    }
    // FAudioDevice::AddAudioVolumeProxy keeps them by priority, highest first.
    volumes.sort_by(|a, b| b.priority.total_cmp(&a.priority));
    if volumes.is_empty() {
        info!("audio volumes: none with brushes (re-export with tools/f13-export collision)");
    }
    commands.insert_resource(AudioVolumes(volumes));
}

/// FListener's interior state: its volume, settings, transition times and interpolation factors.
#[derive(Resource)]
struct ListenerInterior {
    id: u32,
    settings: InteriorSettings,
    start: f64,
    interior_end: f64,
    exterior_end: f64,
    interior_lpf_end: f64,
    exterior_lpf_end: f64,
    interior: f32,
    exterior: f32,
    interior_lpf: f32,
    exterior_lpf: f32,
}

impl Default for ListenerInterior {
    fn default() -> Self {
        ListenerInterior {
            id: 0,
            settings: InteriorSettings::world(),
            start: 0.0,
            interior_end: 0.0,
            exterior_end: 0.0,
            interior_lpf_end: 0.0,
            exterior_lpf_end: 0.0,
            interior: 1.0,
            exterior: 1.0,
            interior_lpf: 1.0,
            exterior_lpf: 1.0,
        }
    }
}

fn listener_interior(time: Res<Time>, volumes: Option<Res<AudioVolumes>>, listener: Query<&GlobalTransform, With<SpatialListener>>, mut l: ResMut<ListenerInterior>) {
    let (Some(volumes), Some(ear)) = (volumes, listener.iter().next()) else { return };
    let now = time.elapsed_secs_f64();
    let (id, settings) = volumes.at(ear.translation());
    if id != l.id || settings != l.settings {
        // ApplyInteriorSettings: into the world's zone the times are the old settings'.
        let times = if settings.world { l.settings } else { settings };
        l.start = now;
        l.interior_end = now + f64::from(times.interior_time);
        l.exterior_end = now + f64::from(times.exterior_time);
        l.interior_lpf_end = now + f64::from(times.interior_lpf_time);
        l.exterior_lpf_end = now + f64::from(times.exterior_lpf_time);
        l.id = id;
        l.settings = settings;
        info!("audio volume: listener in {id} ({}exterior volume {}, LPF {})", if settings.world { "the world's: " } else { "" }, settings.exterior_volume, settings.exterior_lpf);
    }
    let factor = |end: f64| if l.start > now { 0.0 } else if end <= now { 1.0 } else { (((now - l.start) / (end - l.start)) as f32).clamp(0.0, 1.0) };
    let (i, e, il, el) = (factor(l.interior_end), factor(l.exterior_end), factor(l.interior_lpf_end), factor(l.exterior_lpf_end));
    l.interior = i;
    l.exterior = e;
    l.interior_lpf = il;
    l.exterior_lpf = el;
}

/// A spatialized voice's interior state (FActiveSound's): the volume it's in, its volume and LPF now and where they
/// started from at the listener's last transition.
#[derive(Component)]
pub(crate) struct VoiceInterior {
    position: Vec3,
    id: u32,
    settings: InteriorSettings,
    source: (f32, f32),
    current: (f32, f32),
    last_update: f64,
}

/// A voice's InteriorVolumeMultiplier (attenuate multiplies it in).
#[derive(Component)]
pub(crate) struct InteriorGain(pub f32);

#[allow(clippy::type_complexity)]
fn voice_interiors(
    mut commands: Commands,
    time: Res<Time>,
    volumes: Option<Res<AudioVolumes>>,
    mixes: Option<Res<SoundMixes>>,
    l: Res<ListenerInterior>,
    mut voices: Query<(Entity, &GlobalTransform, &VoiceClass, &Pan, Option<&mut VoiceInterior>)>,
) {
    let (Some(volumes), Some(mixes)) = (volumes, mixes) else { return };
    let now = time.elapsed_secs_f64();
    for (e, g, class, pan, state) in &mut voices {
        if !class.0.as_deref().is_some_and(|c| mixes.applies_ambient_volumes(c)) {
            continue;
        }
        let p = g.translation();
        let Some(mut s) = state else {
            let (id, settings) = volumes.at(p);
            commands.entity(e).insert(VoiceInterior { position: p, id, settings, source: (1.0, MAX_FILTER_FREQUENCY), current: (1.0, MAX_FILTER_FREQUENCY), last_update: 0.0 });
            continue;
        };
        // Its volume again when it has moved.
        if s.position.distance_squared(p) * 1e4 > 1e-4 {
            (s.id, s.settings) = volumes.at(p);
            s.position = p;
        }
        if s.last_update < l.start {
            s.source = s.current;
            s.last_update = now;
        }
        let (sv, sl) = s.source;
        s.current = if l.id == s.id {
            ((1.0 - sv) * l.interior + sv, (MAX_FILTER_FREQUENCY - sl) * l.interior_lpf + sl)
        } else if !s.settings.world {
            let volume = ((s.settings.interior_volume - sv) * l.interior + sv) * ((l.settings.exterior_volume - sv) * l.exterior + sv);
            let lpf = ((s.settings.interior_lpf - sl) * l.interior_lpf + sl).min((l.settings.exterior_lpf - sl) * l.exterior_lpf + sl);
            (volume, lpf)
        } else {
            ((l.settings.exterior_volume - sv) * l.exterior + sv, (l.settings.exterior_lpf - sl) * l.exterior_lpf + sl)
        };
        let (volume, lpf) = s.current;
        if volume < 0.999 || lpf < MAX_FILTER_FREQUENCY {
            trace!("audio volume: {:?} in {} at volume {volume:.2}, LPF {lpf:.0}", class.0, s.id);
        }
        commands.entity(e).insert(InteriorGain(volume));
        pan.0.set_filter(voice_filter(lpf));
    }
}
