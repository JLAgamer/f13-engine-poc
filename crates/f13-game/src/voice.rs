//! Voice chat over the match's connection: the microphone (cpal), Opus at 16 kHz mono (the game's VoIP sound:
//! SCCharacter's VoIPAudioComponent plays a procedural wave of 16000 Hz, one channel: FUN_1402e6c70), to the server,
//! which passes each frame to the other players; each machine plays a player's voice from their character
//! (VoIPAudioComponent on its root, VolumeMultiplier 1.5, spatialized).
//!
//! The game's voice is open mic by default (DefaultGame.ini bRequiresPushToTalk=false).
//!
//! PLACEHOLDERS:
//! - the attenuation: no attenuation asset or override is set on VoIPAudioComponent by code or Blueprint that was found;
//!   UE's FSoundAttenuationSettings defaults stand in (linear, full within 400 cm, silent 3600 cm further);
//! - the walkie-talkies (SCWalkieTalkie MinVoIPDistance 400: carriers further apart hear each other by radio) and who
//!   hears the dead aren't ported: everyone hears everyone by distance;
//! - the gate (frames whose RMS is under [`GATE_RMS`] aren't sent), not the engine's.

use bevy::prelude::*;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use lightyear::prelude::*;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};

/// The VoIP wave's rate and an Opus frame of it (20 ms).
const RATE: u32 = 16000;
const FRAME: usize = 320;
/// VoIPAudioComponent's VolumeMultiplier (DAT_1420f594c).
const VOLUME: f32 = 1.5;
/// FSoundAttenuationSettings' defaults (cm): the inner radius and the falloff distance.
const INNER_CM: f32 = 400.0;
const FALLOFF_CM: f32 = 3600.0;
/// Frames quieter than this aren't sent (PLACEHOLDER).
const GATE_RMS: f32 = 0.01;
/// Frames buffered before a voice starts playing (jitter).
const START_FRAMES: usize = 3;

/// A frame of the local player's voice, to the server.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct VoiceUp(pub Vec<u8>);

/// A frame of a player's voice, from the server: whose (their character's server id: NetBody.id) and the Opus packet.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct VoiceDown {
    pub talker: u64,
    pub frame: Vec<u8>,
}

pub struct VoiceChannel;

pub(crate) fn protocol(app: &mut App) {
    // Unordered (the decoder takes frames as they come; a late one plays late), above the replication's priority.
    app.add_channel::<VoiceChannel>(ChannelSettings { mode: ChannelMode::UnorderedUnreliable, priority: 10.0, ..default() }).add_direction(NetworkDirection::Bidirectional);
    app.register_message::<VoiceUp>().add_direction(NetworkDirection::ClientToServer);
    app.register_message::<VoiceDown>().add_direction(NetworkDirection::ServerToClient);
}

/// A dedicated server relays without audio devices of its own.
#[derive(Resource)]
struct NoDevices;

pub(crate) fn server(app: &mut App, dedicated: bool) {
    if dedicated {
        app.insert_resource(NoDevices);
    }
    app.add_systems(Startup, open_devices).add_systems(Update, (capture, relay, play_gains).chain());
}

pub(crate) fn client(app: &mut App) {
    app.add_systems(Startup, open_devices).add_systems(Update, (capture, receive, play_gains).chain());
}

/// One player's voice being played: its 16 kHz samples, the read position, started (past the jitter buffer), and its
/// gains (left, right).
#[derive(Default)]
struct Voice {
    buf: VecDeque<f32>,
    pos: f32,
    started: bool,
    gains: (f32, f32),
}

type Voices = Arc<Mutex<HashMap<u64, Voice>>>;

/// The audio devices (cpal's streams aren't Send: a non-send resource) and the codec state.
struct Devices {
    _input: Option<cpal::Stream>,
    _output: Option<cpal::Stream>,
    /// Captured samples (mono, the input's rate) and that rate.
    mic: Arc<Mutex<Vec<f32>>>,
    mic_rate: u32,
    /// Captured audio resampled to 16 kHz, waiting for a whole frame; the resampler's position.
    pending: Vec<f32>,
    phase: f64,
    encoder: Option<opus::Encoder>,
    decoders: HashMap<u64, opus::Decoder>,
    voices: Voices,
    /// Testing (F13_VOICE_TONE): a 440 Hz tone instead of the microphone; its phase.
    tone: Option<f64>,
    tone_clock: Option<std::time::Instant>,
    heard: HashMap<u64, u32>,
}

fn open_devices(world: &mut World) {
    let host = cpal::default_host();
    let mic = Arc::new(Mutex::new(Vec::new()));
    let voices: Voices = Arc::new(Mutex::new(HashMap::new()));
    let tone = std::env::var("F13_VOICE_TONE").ok().map(|_| 0.0);
    // F13_NO_VOICE: no microphone or voice output (to compare the game's other sound without them).
    let none = world.contains_resource::<NoDevices>() || std::env::var("F13_NO_VOICE").is_ok();
    let (input, mic_rate) = match host.default_input_device().filter(|_| tone.is_none() && !none) {
        Some(dev) => match open_input(&dev, mic.clone()) {
            Ok(v) => v,
            Err(e) => {
                warn!("voice: no microphone ({e})");
                (None, RATE)
            }
        },
        None => (None, RATE),
    };
    let output = match host.default_output_device().filter(|_| !none) {
        Some(dev) => open_output(&dev, voices.clone()).unwrap_or_else(|e| {
            warn!("voice: no output ({e})");
            None
        }),
        None => None,
    };
    info!("voice: microphone {}, output {}", if tone.is_some() { "a test tone".to_string() } else if input.is_some() { format!("{mic_rate} Hz") } else { "none".into() }, if output.is_some() { "on" } else { "none" });
    world.insert_non_send_resource(Devices {
        _input: input,
        _output: output,
        mic,
        mic_rate,
        pending: Vec::new(),
        phase: 0.0,
        encoder: opus::Encoder::new(RATE, opus::Channels::Mono, opus::Application::Voip).ok(),
        decoders: HashMap::new(),
        voices,
        tone,
        tone_clock: None,
        heard: HashMap::new(),
    });
}

fn open_input(dev: &cpal::Device, mic: Arc<Mutex<Vec<f32>>>) -> Result<(Option<cpal::Stream>, u32), String> {
    let cfg = dev.default_input_config().map_err(|e| e.to_string())?;
    let (rate, channels) = (cfg.sample_rate(), cfg.channels() as usize);
    let push = move |mono: &mut dyn Iterator<Item = f32>| {
        if let Ok(mut m) = mic.lock() {
            m.extend(mono);
        }
    };
    let err = |e: cpal::Error| warn!("voice: microphone: {e}");
    let stream = match cfg.sample_format() {
        cpal::SampleFormat::F32 => {
            let mut push = push;
            dev.build_input_stream::<f32, _, _>(cfg.config(), move |d: &[f32], _| push(&mut d.chunks(channels).map(|c| c.iter().sum::<f32>() / channels as f32)), err, None)
        }
        cpal::SampleFormat::I16 => {
            let mut push = push;
            dev.build_input_stream::<i16, _, _>(cfg.config(), move |d: &[i16], _| push(&mut d.chunks(channels).map(|c| c.iter().map(|&s| s as f32 / 32768.0).sum::<f32>() / channels as f32)), err, None)
        }
        f => return Err(format!("unsupported sample format {f:?}")),
    }
    .map_err(|e| e.to_string())?;
    stream.play().map_err(|e| e.to_string())?;
    Ok((Some(stream), rate))
}

fn open_output(dev: &cpal::Device, voices: Voices) -> Result<Option<cpal::Stream>, String> {
    let cfg = dev.default_output_config().map_err(|e| e.to_string())?;
    let (rate, channels) = (cfg.sample_rate() as f32, cfg.channels() as usize);
    let step = RATE as f32 / rate;
    // Mixes every voice into (left, right) for one output frame.
    let mut mix = move || -> (f32, f32) {
        let Ok(mut vs) = voices.lock() else { return (0.0, 0.0) };
        let (mut l, mut r) = (0.0, 0.0);
        for v in vs.values_mut() {
            if !v.started {
                v.started = v.buf.len() >= START_FRAMES * FRAME;
                if !v.started {
                    continue;
                }
            }
            let i = v.pos as usize;
            let (Some(&a), Some(&b)) = (v.buf.get(i), v.buf.get(i + 1)) else {
                v.started = false;
                continue;
            };
            let s = a + (b - a) * v.pos.fract();
            l += s * v.gains.0;
            r += s * v.gains.1;
            v.pos += step;
            let used = v.pos as usize;
            v.buf.drain(..used.min(v.buf.len()));
            v.pos -= used as f32;
        }
        (l, r)
    };
    let err = |e: cpal::Error| warn!("voice: output: {e}");
    let stream = match cfg.sample_format() {
        cpal::SampleFormat::F32 => dev.build_output_stream::<f32, _, _>(
            cfg.config(),
            move |d: &mut [f32], _| {
                for f in d.chunks_mut(channels) {
                    let (l, r) = mix();
                    for (k, s) in f.iter_mut().enumerate() {
                        *s = if k % 2 == 0 { l } else { r };
                    }
                }
            },
            err,
            None,
        ),
        cpal::SampleFormat::I16 => dev.build_output_stream::<i16, _, _>(
            cfg.config(),
            move |d: &mut [i16], _| {
                for f in d.chunks_mut(channels) {
                    let (l, r) = mix();
                    for (k, s) in f.iter_mut().enumerate() {
                        *s = ((if k % 2 == 0 { l } else { r }).clamp(-1.0, 1.0) * 32767.0) as i16;
                    }
                }
            },
            err,
            None,
        ),
        f => return Err(format!("unsupported sample format {f:?}")),
    }
    .map_err(|e| e.to_string())?;
    stream.play().map_err(|e| e.to_string())?;
    Ok(Some(stream))
}

/// The microphone's new audio as encoded 20 ms frames worth sending.
fn frames(d: &mut Devices) -> Vec<Vec<u8>> {
    let captured: Vec<f32> = match d.tone.as_mut() {
        Some(phase) => {
            // As much tone as real time has passed (16 kHz).
            let now = std::time::Instant::now();
            let due = d.tone_clock.map_or(0.0, |t| (now - t).as_secs_f64() * RATE as f64) as usize;
            if d.tone_clock.is_none() || due > 0 {
                d.tone_clock = Some(d.tone_clock.map_or(now, |t| t + std::time::Duration::from_secs_f64(due as f64 / RATE as f64)));
            }
            let mut v = Vec::with_capacity(due);
            for _ in 0..due {
                v.push((*phase * std::f64::consts::TAU).sin() as f32 * 0.3);
                *phase = (*phase + 440.0 / RATE as f64).fract();
            }
            d.pending.extend(v);
            Vec::new()
        }
        None => std::mem::take(&mut *d.mic.lock().map_or_else(|e| e.into_inner(), |m| m)),
    };
    // Resampled to 16 kHz (linear).
    if !captured.is_empty() {
        let step = d.mic_rate as f64 / RATE as f64;
        while (d.phase as usize) + 1 < captured.len() {
            let i = d.phase as usize;
            let t = (d.phase - i as f64) as f32;
            d.pending.push(captured[i] + (captured[i + 1] - captured[i]) * t);
            d.phase += step;
        }
        d.phase -= (captured.len() - 1) as f64;
        d.phase = d.phase.max(0.0);
    }
    let mut out = Vec::new();
    static SENT: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    while d.pending.len() >= FRAME {
        let frame: Vec<f32> = d.pending.drain(..FRAME).collect();
        let rms = (frame.iter().map(|s| s * s).sum::<f32>() / FRAME as f32).sqrt();
        if rms < GATE_RMS {
            continue;
        }
        let mut packet = vec![0u8; 1000];
        if let Some(Ok(n)) = d.encoder.as_mut().map(|e| e.encode_float(&frame, &mut packet)) {
            packet.truncate(n);
            out.push(packet);
            let c = SENT.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
            if c == 1 {
                info!("voice: sending");
            } else if c % 250 == 0 {
                debug!("voice: sent {c} frames ({n} bytes)");
            }
        }
    }
    out
}

/// A received frame into its talker's voice.
fn hear(d: &mut Devices, talker: u64, frame: &[u8]) {
    let Some(dec) = (match d.decoders.entry(talker) {
        std::collections::hash_map::Entry::Occupied(e) => Some(e.into_mut()),
        std::collections::hash_map::Entry::Vacant(e) => opus::Decoder::new(RATE, opus::Channels::Mono).ok().map(|dec| e.insert(dec)),
    }) else {
        return;
    };
    let mut pcm = vec![0f32; FRAME * 3];
    let Ok(n) = dec.decode_float(frame, &mut pcm, false) else { return };
    let count = d.heard.entry(talker).or_default();
    *count += 1;
    if *count == 1 {
        info!("voice: hearing {talker:x}");
    } else if *count % 250 == 0 {
        debug!("voice: hearing {talker:x} ({count} frames)");
    }
    if let Ok(mut vs) = d.voices.lock() {
        let v = vs.entry(talker).or_default();
        v.buf.extend(&pcm[..n]);
        // Over a second behind: drop the oldest.
        let excess = v.buf.len().saturating_sub(RATE as usize);
        v.buf.drain(..excess);
    }
}

/// On a client: the microphone's frames to the server.
fn capture(mut d: NonSendMut<Devices>, client: Option<Res<crate::net::NetClient>>, mut senders: Query<&mut MessageSender<VoiceUp>, With<Connected>>) {
    // (The server's own player's are taken by `relay`.)
    if client.is_none() {
        return;
    }
    let frames = frames(&mut d);
    for mut s in &mut senders {
        for f in &frames {
            s.send::<VoiceChannel>(VoiceUp(f.clone()));
        }
    }
}

/// The server: each player's frames (and its own player's) to everyone else, and played here.
#[allow(clippy::too_many_arguments)]
fn relay(
    mut d: NonSendMut<Devices>,
    mut links: Query<(Entity, &mut MessageReceiver<VoiceUp>)>,
    counselors: Query<(Entity, &crate::net::PlayedBy)>,
    jason_player: Option<Res<crate::lobby::JasonPlayer>>,
    jasons: Query<Entity, With<crate::jason::Jason>>,
    local_counselor: Query<Entity, (With<crate::controls::LocalPlayer>, With<crate::player::Player>)>,
    play_jason: Option<Res<crate::jason_player::PlayJason>>,
    mut sender: ServerMultiMessageSender,
    server: Query<&lightyear::prelude::server::Server>,
    remotes: Query<&RemoteId>,
) {
    let Ok(server) = server.single() else { return };
    // A link's character: her counselor, or Jason when they play him.
    let pawn = |link: Entity| counselors.iter().find(|(_, by)| by.0 == link).map(|(e, _)| e).or_else(|| jason_player.as_ref().filter(|j| j.0 == link).and_then(|_| jasons.iter().next()));
    let mut out: Vec<(u64, Vec<u8>, Option<Entity>)> = Vec::new();
    for (link, mut r) in &mut links {
        let frames: Vec<VoiceUp> = r.receive().collect();
        if let Some(p) = pawn(link) {
            out.extend(frames.into_iter().map(|f| (p.to_bits(), f.0, Some(link))));
        }
    }
    // The server's own player's voice.
    let own = local_counselor.iter().next().or_else(|| play_jason.as_ref().and_then(|_| jasons.iter().next()));
    let local = frames(&mut d);
    if let Some(p) = own {
        out.extend(local.into_iter().map(|f| (p.to_bits(), f, None)));
    }
    for (talker, frame, from) in out {
        let msg = VoiceDown { talker, frame: frame.clone() };
        let to = match from.and_then(|l| remotes.get(l).ok()) {
            Some(id) => NetworkTarget::AllExceptSingle(id.0),
            None => NetworkTarget::All,
        };
        if let Err(e) = sender.send::<_, VoiceChannel>(&msg, server, &to) {
            warn_once!("voice: relay failed: {e:?}");
        }
        if from.is_some() {
            hear(&mut d, talker, &frame);
        }
    }
}

/// A client: the other players' frames.
fn receive(mut d: NonSendMut<Devices>, mut receivers: Query<&mut MessageReceiver<VoiceDown>>) {
    for mut r in &mut receivers {
        for m in r.receive() {
            hear(&mut d, m.talker, &m.frame);
        }
    }
}

/// Each voice's gains from where its talker's character is (the server's id: the twin on a client), heard from the
/// listener: VolumeMultiplier x the attenuation, panned as the game's other sounds (audio.rs x3d_stereo).
fn play_gains(d: NonSend<Devices>, listener: Query<&GlobalTransform, With<SpatialListener>>, twins: Option<Res<crate::net::Twins>>, globals: Query<&GlobalTransform>) {
    let Some(ear) = listener.iter().next() else { return };
    let to_listener = ear.affine().inverse();
    let Ok(mut vs) = d.voices.lock() else { return };
    for (&talker, v) in vs.iter_mut() {
        let entity = match &twins {
            Some(t) => t.of(talker),
            None => Some(Entity::from_bits(talker)),
        };
        let Some(g) = entity.and_then(|e| globals.get(e).ok()) else {
            v.gains = (0.0, 0.0);
            continue;
        };
        let at = g.translation() + Vec3::Y * crate::player::CAPSULE_HALF_HEIGHT_M;
        let cm = ear.translation().distance(at) * 100.0;
        let a = (1.0 - (cm - INNER_CM).max(0.0) / FALLOFF_CM).clamp(0.0, 1.0);
        let local = to_listener.transform_point3(at);
        let (right, front) = (local.x, -local.z);
        let (l, r) = crate::audio::x3d_stereo(right.atan2(front), right.hypot(front) <= f32::EPSILON);
        v.gains = (l * a * VOLUME, r * a * VOLUME);
    }
}
