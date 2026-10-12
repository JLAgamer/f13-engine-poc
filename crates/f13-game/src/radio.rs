//! Radios (SCRadio: BP_Radio_C). A counselor turns one on or off (a press: OnInteract's Counselor_UseRadio move) or,
//! while it plays, changes its song (a hold past the Interactable's HoldTimeLimit, 0.5 s: CanInteractWith gives a
//! counselor press and hold, 3, only while it plays). Her take's RadioUse driver window (OnUseRadio): changing tracks,
//! another song at random (NextTrack: rand() until it differs); else bIsPlaying flips (Role authority). Playing,
//! PlaySong sets the MusicComponent (VolumeMultiplier 0.5) to Songs[track] (StreamerModeSong in streamer mode, not
//! offered here) and a timer for the song's duration that calls NextTrack; StartMusic plays TurnOnSound on its
//! SoundEffectsComponent; StopMusic stops the music and plays TurnOffSound unless broken. Turning one on scores its
//! TurnRadioOnScoreEvent (DJDistracted_C) for her.
//!
//! Jason breaks one (CanInteractWith: not while grabbing; OnInteract's Destroy_SpecialMoveHandler, his weapon's move:
//! Jason_Radio_Destruction_Axe for the pickaxe): at his take's DestroyRadio driver window OnBreakRadio sets bIsBroken
//! (OnRep_Broken: its Interactable off, BreakSound) and stops the music; the Blueprint's OnRadioDestroyed activates
//! P_Radio_Destroyed_01. A playing radio puts sound blips on his screen (senses.rs).
//!
//! Not ported: the perk volume (PerkModifierVolume: perks aren't in), the AudioComponents' attenuation overrides
//! (their NaturalSound falloff and occlusion: the cue's own attenuation plays). PLACEHOLDER: the first track (0: the
//! constructor's zeroed index, unread).

use bevy::prelude::*;
use f13_data::classes::ClassDb;
use f13_data::level::Xform;

pub struct RadioPlugin;

impl Plugin for RadioPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, radio_bot.run_if(resource_exists::<RadioBot>)).add_systems(Update, (spawn_radios, next_track.run_if(crate::net::simulating), uses.run_if(crate::net::simulating), effects).chain());
    }
}

/// The counselor's use take (Counselor_UseRadio's ContextMontage, her `f_` take).
pub const USE_TAKE: &str = "f_normal_stand_use_radio";
/// Jason's (Jason_Radio_Destruction_Axe's AM_Break_Radio_Axe).
pub const BREAK_TAKE: &str = "normal_stand_radio_break_axe";
/// The MusicComponent's VolumeMultiplier.
const MUSIC_VOLUME: f32 = 0.5;
/// Its music's voices (audio.rs ActorCue / StopLoop).
const MUSIC: &str = "MusicComponent";

#[derive(Component, Debug)]
pub struct Radio {
    pub class: String,
    /// Its destroyed particles' name (`<root entity>.P_Radio_Destroyed_01`).
    pub actor: String,
    /// The Interactable (world) and its DistanceLimit (cm), HoldTimeLimit (s).
    pub interact: Vec3,
    pub reach: f32,
    pub hold_limit: f32,
    /// Counselor_UseRadio's spot (CounselorUse_SpecialMoveHandler, world).
    pub use_spot: Transform,
    pub broken: bool,
    pub playing: bool,
    pub track: usize,
    /// Songs: their cues and lengths (s).
    pub songs: Vec<(String, f32)>,
    /// When the song ends (game time): NextTrack.
    pub song_end: Option<f32>,
    /// Its sound blips' countdown (+0x378; senses.rs).
    pub blip: f32,
    turn_on: Option<String>,
    turn_off: Option<String>,
    break_sound: Option<String>,
    /// What [`effects`] last played for.
    shown: (bool, bool, usize),
}

impl Radio {
    /// CanInteractWith for a counselor: the press only, or press and hold while it plays (none once broken).
    pub fn usable(&self) -> bool {
        !self.broken
    }
}

/// A counselor's Counselor_UseRadio move on a radio: s into her take, and whether it changes tracks
/// (bIsChangingTracks).
#[derive(Component)]
pub struct RadioUse {
    pub radio: Entity,
    pub change: bool,
    pub t: f32,
    applied: bool,
}

impl RadioUse {
    pub fn new(radio: Entity, change: bool) -> Self {
        RadioUse { radio, change, t: 0.0, applied: false }
    }
}

/// A song's length (s): its Ogg Vorbis file's last granule position over its sample rate.
fn ogg_length(path: &std::path::Path) -> Option<f32> {
    let b = std::fs::read(path).ok()?;
    // The identification header: "\x01vorbis", version (4), channels (1), sample rate (4).
    let id = b.windows(7).position(|w| w == b"\x01vorbis")?;
    let rate = u32::from_le_bytes(b.get(id + 12..id + 16)?.try_into().ok()?);
    let last = b.windows(4).rposition(|w| w == b"OggS")?;
    let granule = u64::from_le_bytes(b.get(last + 6..last + 14)?.try_into().ok()?);
    (rate > 0).then(|| granule as f32 / rate as f32)
}

/// A cue's length: its longest wave's.
fn cue_length(root: &std::path::Path, cue: &str) -> Option<f32> {
    let text = std::fs::read_to_string(root.join(format!("sounds/{cue}.json"))).ok()?;
    let v: serde_json::Value = serde_json::from_str(&text).ok()?;
    let mut waves = Vec::new();
    fn walk(v: &serde_json::Value, out: &mut Vec<String>) {
        if let Some(w) = v["Wave"].as_str() {
            out.push(w.to_string());
        }
        for c in v["Children"].as_array().into_iter().flatten() {
            walk(c, out);
        }
    }
    walk(&v["Root"], &mut waves);
    waves.iter().filter_map(|w| ogg_length(&root.join("sounds").join(w))).reduce(f32::max)
}

/// The match's radios (props.rs spawns their meshes from the match world: cabin spawners, which only spawn when the
/// map's world settings override spawning; Crystal Lake's lists none, `--override-spawning` forces it): their state,
/// and their P_Radio_Destroyed_01 (inactive until broken), named `<root entity>.P_Radio_Destroyed_01`.
fn spawn_radios(
    mut commands: Commands,
    export: Res<crate::ExportRoot>,
    added: Query<(Entity, &Name, &Transform), Added<crate::net_world::WorldId>>,
    mut classes: Local<Option<ClassDb>>,
    mut particles: MessageWriter<crate::particles::SpawnParticles>,
) {
    if added.is_empty() {
        return;
    }
    let classes = classes.get_or_insert_with(|| ClassDb::load(&export.0.join("classes.json")).unwrap_or_default());
    for (e, name, xf) in &added {
        let class = name.as_str();
        if !classes.is_child_of(class, "SCRadio") {
            continue;
        }
        let Some(info) = classes.get(class) else { continue };
        let component = |name: &str| info.components.iter().find(|c| c.name == name);
        let local = |name: &str| crate::to_transform(&component(name).and_then(|c| c.local).unwrap_or(Xform::IDENTITY));
        let world = |name: &str| *xf * local(name);
        let props = component("Interactable").map(|c| &c.props);
        let prop = |k: &str| info.props.get(k);
        let songs = prop("Songs")
            .and_then(|s| s.as_array())
            .into_iter()
            .flatten()
            .filter_map(|s| s["AssetPathName"].as_str()?.rsplit('.').next().map(str::to_string))
            .map(|cue| {
                let len = cue_length(&export.0, &cue).unwrap_or_else(|| {
                    warn!("radio: no length for {cue} (export its sounds): 180 s");
                    180.0
                });
                (cue, len)
            })
            .collect::<Vec<_>>();
        let cue = |k: &str| prop(k).and_then(crate::audio::cue_name);
        let actor = format!("{e}.P_Radio_Destroyed_01");
        if let Some(c) = component("P_Radio_Destroyed_01") {
            if let Some(system) = c.props["Template"].as_str().and_then(|t| t.split('\'').nth(1)) {
                particles.write(crate::particles::SpawnParticles { system: system.to_string(), transform: local("P_Radio_Destroyed_01"), follow: Some(e), actor: actor.clone(), active: false, time_dilation: 1.0 });
            }
        }
        commands.entity(e).insert((
            Radio {
                class: class.to_string(),
                actor,
                interact: world("Interactable").translation,
                reach: props.and_then(|p| p["DistanceLimit"].as_f64()).map_or(100.0, |v| v as f32),
                hold_limit: props.and_then(|p| p["HoldTimeLimit"].as_f64()).map_or(0.5, |v| v as f32),
                use_spot: world("CounselorUse_SpecialMoveHandler"),
                broken: false,
                playing: false,
                track: 0,
                songs,
                song_end: None,
                blip: 0.0,
                turn_on: cue("TurnOnSound"),
                turn_off: cue("TurnOffSound"),
                break_sound: cue("BreakSound"),
                shown: (false, false, 0),
            },
            crate::interact::InteractLimits::of(classes, class),
        ));
        info!("radio at ({:.0}, {:.0}, {:.0})", xf.translation.x, xf.translation.y, xf.translation.z);
    }
}

/// NextTrack: another song, at random, not the one playing.
fn pick_other(r: &mut Radio, rng: &mut crate::GameRng) {
    let n = r.songs.len();
    if n > 1 {
        loop {
            let i = ((rng.0.frand() * n as f32) as usize).min(n - 1);
            if i != r.track {
                r.track = i;
                break;
            }
        }
    }
}

/// A song's timer ran out: the next one (NextTrack).
fn next_track(time: Res<Time>, mut rng: ResMut<crate::GameRng>, mut radios: Query<&mut Radio>) {
    let now = time.elapsed_secs();
    for mut r in &mut radios {
        if r.playing && r.song_end.is_some_and(|e| now >= e) {
            pick_other(&mut r, &mut rng);
            let len = r.songs.get(r.track).map_or(180.0, |s| s.1);
            r.song_end = Some(now + len);
            debug!("radio {}: next song {}", r.actor, r.track);
        }
    }
}

/// Counselors' use moves: the radio changes at the take's RadioUse window; the take ends the move.
fn uses(
    mut commands: Commands,
    time: Res<Time>,
    mut rng: ResMut<crate::GameRng>,
    model: Option<Res<crate::character::CounselorCharacter>>,
    mut users: Query<(Entity, &mut RadioUse)>,
    mut radios: Query<&mut Radio>,
) {
    let take = model.as_ref().and_then(|m| m.info.animations.get(USE_TAKE));
    let window = take.and_then(|a| a.windows.iter().find(|w| w.name == "Driver: RadioUse")).map_or(0.5, |w| w.start);
    let length = take.map_or(1.17, |a| a.length);
    let now = time.elapsed_secs();
    for (e, mut u) in &mut users {
        u.t += time.delta_secs();
        if !u.applied && u.t >= window {
            u.applied = true;
            if let Ok(mut r) = radios.get_mut(u.radio) {
                if r.broken {
                } else if u.change {
                    pick_other(&mut r, &mut rng);
                    info!("radio {}: song {}", r.actor, r.track);
                } else {
                    r.playing = !r.playing;
                    info!("radio {}: {}", r.actor, if r.playing { "on" } else { "off" });
                }
                if r.playing {
                    let len = r.songs.get(r.track).map_or(180.0, |s| s.1);
                    r.song_end = Some(now + len);
                } else {
                    r.song_end = None;
                }
            }
        }
        if u.t >= length {
            commands.entity(e).remove::<(RadioUse, crate::character::PlayAction)>();
        }
    }
}

/// Jason's OnBreakRadio: broken, its music stopped.
pub fn break_radio(r: &mut Radio) {
    r.broken = true;
    r.playing = false;
    r.song_end = None;
}

/// What a radio's state sounds and looks like, on every machine: its music and switch sounds (StartMusic /
/// StopMusic / PlaySong), its break (OnRep_Broken's BreakSound, OnRadioDestroyed's particles).
fn effects(mut commands: Commands, mut radios: Query<(Entity, &mut Radio, &GlobalTransform), Changed<Radio>>, mut particles: MessageWriter<crate::particles::ParticleActivation>) {
    for (e, mut r, g) in &mut radios {
        let now = (r.playing, r.broken, r.track);
        if now == r.shown {
            continue;
        }
        let (was_playing, was_broken, was_track) = r.shown;
        r.shown = now;
        let at = Some(g.translation());
        let sound = |commands: &mut Commands, cue: &Option<String>| {
            if let Some(c) = cue {
                commands.trigger(crate::audio::PlaySound { cue: c.clone(), at });
            }
        };
        if r.broken && !was_broken {
            sound(&mut commands, &r.break_sound);
            particles.write(crate::particles::ParticleActivation { actor: r.actor.clone(), active: true, reset: true });
        }
        if was_playing && (!r.playing || r.track != was_track) {
            commands.trigger(crate::audio::StopLoop { actor: e, prop: MUSIC });
            if !r.playing && !r.broken {
                sound(&mut commands, &r.turn_off);
            }
        }
        if r.playing && (!was_playing || r.track != was_track) {
            if let Some((cue, _)) = r.songs.get(r.track) {
                commands.trigger(crate::audio::ActorCue { actor: e, prop: MUSIC, cue: cue.clone(), volume: MUSIC_VOLUME });
            }
            sound(&mut commands, &r.turn_on);
        }
    }
}

/// Testing (`--bot-radio`, with `--override-spawning`): her, the nearest radio turned on, its song changed by a hold,
/// turned off; playing Jason (`--play-jason`), a playing radio broken with E. Exits 0 when each change happened.
#[derive(Resource, Default)]
pub struct RadioBot {
    step: u32,
    t: f32,
    radio: Option<Entity>,
    track: usize,
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn radio_bot(
    mut bot: ResMut<RadioBot>,
    (time, real): (Res<Time>, Res<Time<Real>>),
    mut keys: ResMut<ButtonInput<KeyCode>>,
    mut jason_controls: Option<ResMut<crate::jason_player::JasonControls>>,
    mut camera: Option<Single<&mut crate::player::PlayerCamera>>,
    mut players: Query<&mut Transform, (With<crate::controls::LocalPlayer>, With<crate::player::Player>)>,
    mut jasons: Query<&mut Transform, (With<crate::jason::Jason>, Without<crate::player::Player>, Without<Radio>)>,
    mut radios: Query<(Entity, &mut Radio), Without<crate::player::Player>>,
    mut exit: MessageWriter<AppExit>,
) {
    bot.t += time.delta_secs();
    if real.elapsed_secs() < 8.0 {
        return;
    }
    let mut finish = |ok: bool, what: &str| {
        if ok {
            info!("bot: radio: {what}: passed");
            exit.write(AppExit::Success);
        } else {
            error!("bot: radio: {what}: FAILED");
            exit.write(AppExit::from_code(1));
        }
    };
    let jason = jason_controls.is_some();
    if bot.step == 0 {
        let me = if jason { jasons.iter().next().map(|x| x.translation) } else { players.iter().next().map(|x| x.translation) };
        let Some(me) = me else { return };
        let Some((e, r)) = radios.iter().min_by(|a, b| a.1.interact.distance(me).total_cmp(&b.1.interact.distance(me))) else {
            finish(false, "no radio (run with --override-spawning)");
            bot.step = 99;
            return;
        };
        info!("bot: radio: the one at {:.1} ({} songs)", r.interact, r.songs.len());
        bot.radio = Some(e);
        bot.step = 1;
        bot.t = 0.0;
        return;
    }
    let Some(re) = bot.radio else { return };
    let Ok((_, mut r)) = radios.get_mut(re) else { return };
    if jason {
        let (Some(c), Ok(mut jx)) = (jason_controls.as_mut(), jasons.single_mut()) else { return };
        match bot.step {
            1 => {
                // A playing radio (its blips too), Jason a step out from its front, facing it.
                r.playing = true;
                let use_at = r.use_spot.translation;
                let out = Vec3::new(use_at.x - r.interact.x, 0.0, use_at.z - r.interact.z).normalize_or_zero();
                jx.translation = Vec3::new(r.interact.x, use_at.y, r.interact.z) + out * 0.9;
                let to = r.interact - jx.translation;
                jx.rotation = Quat::from_rotation_y(to.x.atan2(to.z));
                bot.step = 2;
                bot.t = 0.0;
            }
            2 if bot.t > 2.0 => {
                c.press_interact();
                bot.step = 3;
                bot.t = 0.0;
            }
            3 => {
                if r.broken {
                    finish(!r.playing, "Jason broke it");
                    bot.step = 99;
                } else if bot.t > 6.0 {
                    finish(false, "Jason broke it");
                    bot.step = 99;
                }
            }
            _ => {}
        }
        return;
    }
    let (Some(cam), Ok(mut px)) = (camera.as_mut(), players.single_mut()) else { return };
    let aim = |cam: &mut crate::player::PlayerCamera, px: &Transform, at: Vec3| {
        if let Some(d) = (at - (px.translation + Vec3::Y)).try_normalize() {
            cam.yaw = (-d.x).atan2(-d.z);
            cam.pitch = d.y.asin();
        }
    };
    match bot.step {
        1 => {
            // On her use spot's floor, a little out from the radio.
            let use_at = r.use_spot.translation;
            px.translation = use_at + Vec3::Y * 0.3;
            bot.step = 2;
            bot.t = 0.0;
        }
        2 if bot.t > 2.0 => {
            aim(cam, &px, r.interact);
            keys.press(KeyCode::KeyE);
            bot.step = 3;
            bot.t = 0.0;
        }
        3 => {
            keys.release(KeyCode::KeyE);
            if bot.t > 3.0 {
                if !r.playing {
                    finish(false, "turned on");
                    bot.step = 99;
                    return;
                }
                info!("bot: radio: on, song {}", r.track);
                bot.track = r.track;
                bot.step = 4;
                bot.t = 0.0;
            }
        }
        4 => {
            // Held past its HoldTimeLimit: the next song.
            aim(cam, &px, r.interact);
            if bot.t < 1.0 {
                keys.press(KeyCode::KeyE);
            } else {
                keys.release(KeyCode::KeyE);
            }
            if bot.t > 4.0 {
                if r.track == bot.track || !r.playing {
                    finish(false, "song changed");
                    bot.step = 99;
                    return;
                }
                info!("bot: radio: song {} now", r.track);
                bot.step = 5;
                bot.t = 0.0;
            }
        }
        5 => {
            aim(cam, &px, r.interact);
            keys.press(KeyCode::KeyE);
            bot.step = 6;
            bot.t = 0.0;
        }
        6 => {
            keys.release(KeyCode::KeyE);
            if bot.t > 3.0 {
                finish(!r.playing, "turned off");
                bot.step = 99;
            }
        }
        _ => {}
    }
}
