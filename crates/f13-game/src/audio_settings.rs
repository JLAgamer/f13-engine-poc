//! The audio settings (SCAudioSettingsSaveGame, its ILLAudioSettingsSaveGame fields) and their menu
//! (AudioSettingsMenuWidget: SettingsMenuWidget's audio page).
//!
//! Applying them (ILLAudioSettingsSaveGame vt+0x270, 0x140240870, from FUN_14028e720) sets the sound classes' Volume
//! (USoundClass +0x28) to the volumes: Master, Menu, Music, SoundEffects and Voice (DefaultEngine.ini VoiPSoundClass:
//! the voice chat's); VolumeVoiceOver has no class. bUsePushToTalk is the voice chat's push-to-talk (voice.rs OpenMic
//! when off). Not traced: what bPlayPushToTalkSound plays and what bStreamingMode ("Mute Copyrighted Music",
//! ILLGameUserSettings +0x1ca: IsUsingStreamingMode) mutes; f13-engine-poc keeps both without an effect.
//!
//! f13-engine-poc keeps them in `<home>/.f13-engine-poc/audio_settings.json` (the game: its save game slot).

use bevy::prelude::*;
use serde::{Deserialize, Serialize};

/// ILLAudioSettingsSaveGame's fields, its constructor's defaults (SCAudioSettingsSaveGame's VolumeMusic 0.75).
#[derive(Resource, Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AudioSettings {
    pub volume_master: f32,
    pub volume_menu: f32,
    pub volume_music: f32,
    pub volume_sound_effects: f32,
    pub volume_voice_over: f32,
    pub volume_voice: f32,
    pub use_push_to_talk: bool,
    pub play_push_to_talk_sound: bool,
    pub use_headset_for_game_voice: bool,
    pub streaming_mode: bool,
}

impl Default for AudioSettings {
    fn default() -> Self {
        AudioSettings {
            volume_master: 1.0,
            volume_menu: 1.0,
            volume_music: 0.75,
            volume_sound_effects: 1.0,
            volume_voice_over: 1.0,
            volume_voice: 1.0,
            use_push_to_talk: true,
            play_push_to_talk_sound: false,
            use_headset_for_game_voice: true,
            streaming_mode: false,
        }
    }
}

fn path() -> std::path::PathBuf {
    f13_data::default_path().with_file_name("audio_settings.json")
}

impl AudioSettings {
    /// The saved settings, or the defaults.
    pub fn load() -> Self {
        std::fs::read_to_string(path()).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default()
    }

    fn save(&self) {
        let p = path();
        let _ = std::fs::create_dir_all(p.parent().unwrap_or(std::path::Path::new(".")));
        match serde_json::to_string_pretty(self) {
            Ok(t) => {
                if let Err(e) = std::fs::write(&p, t) {
                    warn!("audio settings: writing {}: {e}", p.display());
                }
            }
            Err(e) => warn!("audio settings: {e}"),
        }
    }

    /// The sound classes the volumes set (vt+0x270).
    pub fn class_volumes(&self) -> [(&'static str, f32); 5] {
        [("Master", self.volume_master), ("Menu", self.volume_menu), ("Music", self.volume_music), ("SoundEffects", self.volume_sound_effects), ("Voice", self.volume_voice)]
    }
}

#[derive(Resource)]
struct Registered;

/// The settings (loaded unless already given: `--open-mic`) and their application; any app with sounds or voice.
pub(crate) fn init(app: &mut App) {
    if app.world().contains_resource::<Registered>() {
        return;
    }
    app.insert_resource(Registered);
    if !app.world().contains_resource::<AudioSettings>() {
        app.insert_resource(AudioSettings::load());
    }
    app.add_systems(Update, (apply_volumes, push_to_talk));
}

/// The volumes to their sound classes (when changed, or once the classes are loaded).
fn apply_volumes(settings: Res<AudioSettings>, mixes: Option<ResMut<crate::audio::SoundMixes>>) {
    let Some(mut mixes) = mixes else { return };
    if !settings.is_changed() && !mixes.is_added() {
        return;
    }
    for (class, v) in settings.class_volumes() {
        mixes.set_class_volume(class, v);
    }
}

/// bUsePushToTalk off: the microphone always sends (voice.rs OpenMic).
fn push_to_talk(mut commands: Commands, settings: Res<AudioSettings>) {
    if !settings.is_changed() {
        return;
    }
    if settings.use_push_to_talk {
        commands.remove_resource::<crate::voice::OpenMic>();
    } else {
        commands.insert_resource(crate::voice::OpenMic);
    }
}

/// AudioSettingsMenuWidget's rows shown on PC (MenuVolumeSlider and UseHeadsetForGameVoiceBoolean are collapsed), in
/// its VerticalBox's order: the option widget and the field.
pub const ROWS: [(&str, Field); 7] = [
    ("MasterVolumeSlider", Field::Volume(0)),
    ("MusicVolumeSlider", Field::Volume(2)),
    ("GameVolumeSlider", Field::Volume(3)),
    ("VoiceVolumeSlider", Field::Volume(5)),
    ("UsePushToTalkBoolean", Field::Flag(0)),
    ("PushToTalkSoundBoolean", Field::Flag(1)),
    ("MuteCopyrightedMusicBoolean", Field::Flag(3)),
];

/// A row's field: a volume (master, menu, music, sound effects, voice over, voice) or a flag (push-to-talk, its sound,
/// headset, streaming mode).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Field {
    Volume(usize),
    Flag(usize),
}

impl AudioSettings {
    fn volume_mut(&mut self, i: usize) -> &mut f32 {
        match i {
            0 => &mut self.volume_master,
            1 => &mut self.volume_menu,
            2 => &mut self.volume_music,
            3 => &mut self.volume_sound_effects,
            4 => &mut self.volume_voice_over,
            _ => &mut self.volume_voice,
        }
    }

    fn flag_mut(&mut self, i: usize) -> &mut bool {
        match i {
            0 => &mut self.use_push_to_talk,
            1 => &mut self.play_push_to_talk_sound,
            2 => &mut self.use_headset_for_game_voice,
            _ => &mut self.streaming_mode,
        }
    }

    pub fn volume(&self, i: usize) -> f32 {
        [self.volume_master, self.volume_menu, self.volume_music, self.volume_sound_effects, self.volume_voice_over, self.volume_voice][i.min(5)]
    }

    pub fn flag(&self, i: usize) -> bool {
        [self.use_push_to_talk, self.play_push_to_talk_sound, self.use_headset_for_game_voice, self.streaming_mode][i.min(3)]
    }
}

/// The audio settings menu, open: the row focused (its DefaultFocusWidgetName: MasterVolumeSlider), the settings being
/// edited (SCSettingsMenuWidget EditableSaveGame) and the arrow's hold (s) for its repeat.
#[derive(Resource)]
pub struct AudioMenu {
    pub focus: usize,
    pub edit: AudioSettings,
    held: f32,
}

/// The slider's step: SetStepSize(ValueDisplayStep / (ValueDisplayMax - ValueDisplayMin)) = 1 / 100.
const STEP: f32 = 0.01;
/// PLACEHOLDER: a held arrow's repeat (Slate's key repeat comes from the OS): after this long, a step each [`REPEAT`].
const REPEAT_DELAY: f32 = 0.4;
const REPEAT: f32 = 0.03;

/// The menu (PLACEHOLDER key: the game opens it from its main or pause menu's Settings, which f13-engine-poc doesn't have): O in
/// the lobby opens it; Up / Down (W / S) focus a row (MenuButtonFocusCue); on a slider Left / Right (A / D) step it
/// (MenuSliderStepCue), on a boolean they flip it (SettingsOptionBooleanWidget's arrows); Enter applies (the action
/// bar's Apply: OnSettingsApply, saving and applying them) and Escape backs out, the edits dropped (OnSettingsBack).
/// Not ported: OnSettingsApply's and OnSettingsBack's confirmation dialogs (ConfirmApplyText, ConfirmBackText) and the
/// Defaults button.
#[allow(clippy::too_many_arguments)]
pub(crate) fn audio_menu(
    mut commands: Commands,
    keys: Res<ButtonInput<KeyCode>>,
    time: Res<Time<bevy::time::Real>>,
    menu: Option<ResMut<AudioMenu>>,
    mut settings: ResMut<AudioSettings>,
    lobby: Query<&crate::lobby::NetLobby>,
    counselor_menu: Option<Res<crate::lobby::CounselorMenu>>,
    mut tested: Local<bool>,
) {
    let open = lobby.iter().any(|l| l.open);
    let Some(mut menu) = menu else {
        // (F13_AUDIO_MENU=<row>: opened on that row as the lobby opens, for screenshots.)
        let test = std::env::var("F13_AUDIO_MENU").ok().and_then(|v| v.parse::<usize>().ok()).filter(|_| !*tested);
        if open && counselor_menu.is_none() && (keys.just_pressed(KeyCode::KeyO) || test.is_some()) {
            *tested = true;
            commands.insert_resource(AudioMenu { focus: test.unwrap_or(0).min(ROWS.len() - 1), edit: settings.clone(), held: 0.0 });
        }
        return;
    };
    if !open || keys.just_pressed(KeyCode::Escape) {
        commands.remove_resource::<AudioMenu>();
        return;
    }
    if keys.just_pressed(KeyCode::Enter) {
        if *settings != menu.edit {
            *settings = menu.edit.clone();
            settings.save();
            info!("audio settings: applied {:?}", *settings);
        }
        commands.remove_resource::<AudioMenu>();
        return;
    }
    let pressed = |a: KeyCode, b: KeyCode| keys.just_pressed(a) || keys.just_pressed(b);
    let n = ROWS.len();
    let focus = if pressed(KeyCode::ArrowUp, KeyCode::KeyW) {
        Some((menu.focus + n - 1) % n)
    } else if pressed(KeyCode::ArrowDown, KeyCode::KeyS) {
        Some((menu.focus + 1) % n)
    } else {
        None
    };
    if let Some(f) = focus {
        menu.focus = f;
        commands.trigger(crate::audio::PlaySound { cue: "MenuButtonFocusCue".into(), at: None });
    }
    let left = keys.pressed(KeyCode::ArrowLeft) || keys.pressed(KeyCode::KeyA);
    let right = keys.pressed(KeyCode::ArrowRight) || keys.pressed(KeyCode::KeyD);
    let dir = right as i32 - left as i32;
    let just = pressed(KeyCode::ArrowLeft, KeyCode::KeyA) || pressed(KeyCode::ArrowRight, KeyCode::KeyD);
    // (`held`: the time to the arrow's next repeat.)
    let mut steps = 0;
    if dir == 0 {
        return;
    } else if just {
        steps = 1;
        menu.held = REPEAT_DELAY;
    } else {
        menu.held -= time.delta_secs();
        while menu.held <= 0.0 {
            steps += 1;
            menu.held += REPEAT;
        }
    }
    if steps == 0 {
        return;
    }
    match ROWS[menu.focus].1 {
        Field::Volume(i) => {
            let v = menu.edit.volume_mut(i);
            let next = ((*v / STEP).round() + (dir * steps) as f32) * STEP;
            let next = next.clamp(0.0, 1.0);
            if next != *v {
                *v = next;
                commands.trigger(crate::audio::PlaySound { cue: "MenuSliderStepCue".into(), at: None });
            }
        }
        Field::Flag(i) if just => {
            let f = menu.edit.flag_mut(i);
            *f = !*f;
        }
        Field::Flag(_) => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_fields_take_the_defaults() {
        let s: AudioSettings = serde_json::from_str(r#"{ "volume_master": 0.5 }"#).unwrap();
        assert_eq!(s.volume_master, 0.5);
        assert_eq!(s.volume_music, 0.75);
        assert!(s.use_push_to_talk);
        assert_eq!(s.class_volumes()[0], ("Master", 0.5));
        assert_eq!(s.volume(2), 0.75);
        assert!(!s.flag(3));
    }
}
