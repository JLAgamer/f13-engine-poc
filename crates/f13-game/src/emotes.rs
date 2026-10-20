//! The counselors' emotes (SCEmoteData: Blueprints/Emotes/*; SCCounselorCharacter's, decompiled).
//!
//! CSLR_ShowEmote (V; DefaultInput.ini) shows the emote wheel (OnRequestShowEmoteSelect 0x1403c68b0: CanPlayEmote,
//! EmoteHoldDelayKeyboard 0 / EmoteHoldDelayController 0.15 s, then the HUD's EmoteWidgetClass, Counselor_Emote_Select,
//! an ILLRadialSelectWidget of her eight emotes: GetEmotesForCounselor, her profile's or her class's DefaultEmotes);
//! releasing it hides the wheel (OnHideEmoteSelect), whose transition out plays the highlighted one (PlayEmote:
//! SERVER_PlayEmote). The server (FUN_1403ce5b0) plays it if she can (CanPlayEmote, FUN_1403a7600: not in a special
//! move, not attacking, walking (MovementMode 1), nothing in her hands' way, no interaction): MULTICAST_PlayEmote
//! (FUN_1403bf390) PlayAnimMontage(her gender's montage) as CurrentEmote; a looping one (bLooping) locks her movement
//! until her stick is released, the rest until they end (SERVER_CancelEmote at the montage's length). Moving (the move
//! axes, past the lock), attacking (unless locked) and crouching cancel it (MULTICAST_CancelEmote: Montage_Stop 0.25 s).
//!
//! f13recomp: the emote plays as her action (character.rs PlayAction: replicated); a client asks the server with
//! [`EmoteMsg`]. PLACEHOLDER: the wheel's highlight from the mouse's travel since it opened (the radial widget's cursor
//! mapping not read), and CanPlayEmote's checks as f13recomp's states have them. Not ported: the looping emotes' 0.35 s
//! SetIgnoreMoveInput.
//!
//! The counselor menu's emotes (Counselor_Emote_Menu, SCCounselorEmoteMenuWidget: decompiled): its CounselorEmoteButton
//! pushes it on the counselor shown; its wheel (Counselor_Emote_Select, not in game) shows her eight, picking a section
//! (OnSelect: PendingIndex, ShowEmoteBox) shows the panel (Counselor_Emote_Panel: GetAvailableEmotesForCounselor's
//! unlocked ones as EmoteButtonLarges, rows of nine), whose highlighted one her preview plays (CounselorEmotePreview:
//! PlayPreviewEmote) and whose pick takes the section (SetEmote), back to the wheel (HideEmoteBox); Back from the panel
//! returns to the wheel, from the wheel to the counselor menu with the profiles (SetUnsavedCounselorProfile; f13recomp
//! saves them then: counselor_profiles.json). All unlocked; the available ones in emotes.json's order (PLACEHOLDER: the
//! native list's order not read).

use bevy::input::mouse::AccumulatedMouseMotion;
use bevy::prelude::*;
use lightyear::prelude::*;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use crate::controls::{Button, LocalPlayer};
use crate::player::Player;

/// An emote: its EmoteName, EmoteIcon (export-relative, no extension), bLooping and its take (the counselors'
/// animation: its FemaleMontage's sequence; a male counselor's is its MaleMontage's under that name).
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct EmoteDef {
    pub name: String,
    #[serde(default)]
    pub icon: Option<String>,
    #[serde(default)]
    pub looping: bool,
    pub take: String,
}

/// Every emote (emotes.json), by class name ("Emote_Wave"), and Base_Counselor's DefaultEmotes.
#[derive(Resource, Default, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct Emotes {
    pub emotes: HashMap<String, EmoteDef>,
    pub default: Vec<String>,
}

impl Emotes {
    /// A counselor's wheel (GetEmotesForCounselor: her profile's emotes, else her class's DefaultEmotes).
    pub fn wheel<'a>(&'a self, class: &str, profiles: Option<&'a crate::wardrobe::CounselorProfiles>) -> &'a [String] {
        profiles.and_then(|p| p.emotes.get(class)).filter(|e| !e.is_empty()).map_or(&self.default, |e| e.as_slice())
    }

    /// The emotes she can pick (GetAvailableEmotesForCounselor: all of them unlocked).
    pub fn available(&self) -> Vec<String> {
        let mut v: Vec<String> = self.emotes.keys().cloned().collect();
        v.sort();
        v
    }
}

/// The counselor menu's emotes, open on a class: her wheel's emotes as edited, the section highlighted and (the panel
/// shown) the emote highlighted in it (an index into [`Emotes::available`]).
#[derive(Resource, Clone, Debug)]
pub struct EmoteMenu {
    pub class: String,
    pub slots: Vec<String>,
    pub section: usize,
    pub list: Option<usize>,
}

/// The emote menu (PLACEHOLDER keys: its wheel and panel are mouse and gamepad widgets): E in the counselor menu opens
/// it; on the wheel Left / Right (A / D) step the section (MenuButtonFocusCue), Enter shows the panel on its emote,
/// Escape keeps them (counselor_profiles.json) and backs out; in the panel the arrows (WASD) move by one or a row of
/// nine, Enter puts the emote in the section and Escape returns to the wheel.
pub(crate) fn emote_menu(
    mut commands: Commands,
    keys: Res<ButtonInput<KeyCode>>,
    menu: Option<ResMut<EmoteMenu>>,
    counselor_menu: Option<Res<crate::lobby::CounselorMenu>>,
    list: Option<Res<crate::lobby::CounselorList>>,
    emotes: Res<Emotes>,
    mut profiles: ResMut<crate::wardrobe::CounselorProfiles>,
    mut tested: Local<bool>,
    perk_menu: Option<Res<crate::perks::PerkMenu>>,
) {
    if perk_menu.is_some() {
        return;
    }
    let pressed = |a: KeyCode, b: KeyCode| keys.just_pressed(a) || keys.just_pressed(b);
    let Some(mut menu) = menu else {
        let class = counselor_menu.as_ref().and_then(|m| list.as_ref()?.0.get(m.index).cloned());
        // (F13_EMOTE_MENU=<section>[,<panel index>]: opened over the counselor menu, for screenshots.)
        let test = std::env::var("F13_EMOTE_MENU").ok().filter(|_| class.is_some() && !std::mem::replace(&mut *tested, true));
        if let Some(class) = class.filter(|_| (keys.just_pressed(KeyCode::KeyE) || test.is_some()) && !emotes.default.is_empty()) {
            let mut slots = emotes.wheel(&class, Some(&profiles)).to_vec();
            slots.resize(8, String::new());
            let n: Vec<usize> = test.as_deref().unwrap_or("").split(',').filter_map(|v| v.parse().ok()).collect();
            commands.insert_resource(EmoteMenu { class, slots, section: n.first().copied().unwrap_or(0) % 8, list: n.get(1).copied() });
        }
        return;
    };
    if counselor_menu.is_none() {
        commands.remove_resource::<EmoteMenu>();
        return;
    }
    let available = emotes.available();
    let focus = |commands: &mut Commands| commands.trigger(crate::audio::PlaySound { cue: "MenuButtonFocusCue".into(), at: None });
    match menu.list {
        None => {
            if keys.just_pressed(KeyCode::Escape) {
                if profiles.emotes.get(&menu.class) != Some(&menu.slots) {
                    profiles.emotes.insert(menu.class.clone(), menu.slots.clone());
                    profiles.save();
                    info!("emotes: {} has {:?}", menu.class, menu.slots);
                }
                // (HandleBackButton's UITransitionAll.)
                commands.trigger(crate::audio::PlaySound { cue: "UITransitionAll".into(), at: None });
                commands.remove_resource::<EmoteMenu>();
            } else if pressed(KeyCode::ArrowLeft, KeyCode::KeyA) {
                menu.section = (menu.section + 7) % 8;
                focus(&mut commands);
            } else if pressed(KeyCode::ArrowRight, KeyCode::KeyD) {
                menu.section = (menu.section + 1) % 8;
                focus(&mut commands);
            } else if keys.just_pressed(KeyCode::Enter) {
                let at = available.iter().position(|e| *e == menu.slots[menu.section]).unwrap_or(0);
                menu.list = Some(at);
            }
        }
        Some(i) => {
            let n = available.len().max(1);
            if keys.just_pressed(KeyCode::Escape) {
                menu.list = None;
            } else if keys.just_pressed(KeyCode::Enter) {
                if let Some(e) = available.get(i) {
                    let s = menu.section;
                    menu.slots[s] = e.clone();
                }
                menu.list = None;
            } else {
                let step: isize = if pressed(KeyCode::ArrowLeft, KeyCode::KeyA) {
                    -1
                } else if pressed(KeyCode::ArrowRight, KeyCode::KeyD) {
                    1
                } else if pressed(KeyCode::ArrowUp, KeyCode::KeyW) {
                    -9
                } else if pressed(KeyCode::ArrowDown, KeyCode::KeyS) {
                    9
                } else {
                    0
                };
                if step != 0 {
                    menu.list = Some((i as isize + step).clamp(0, n as isize - 1) as usize);
                    focus(&mut commands);
                }
            }
        }
    }
}

/// The wheel, open on this machine: the mouse's travel since it opened (design px, y down) and the section it
/// highlights (0 the top, clockwise).
#[derive(Resource, Default, Clone, Debug)]
pub struct EmoteWheel {
    pub aim: Vec2,
    pub section: Option<usize>,
    /// Her eight (GetEmotesForCounselor as it opened).
    pub slots: Vec<String>,
}

/// Playing an emote (CurrentEmote): its time in, length, bLooping, and whether her movement is locked out.
#[derive(Component, Clone, Debug)]
pub struct Emoting {
    pub emote: String,
    t: f32,
    len: f32,
    looping: bool,
    locked: bool,
}

/// A client's SERVER_PlayEmote: the emote's class name.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct EmoteMsg(pub String);

/// A counselor asks to play an emote (her own machine's, or a client's message on the server).
#[derive(Event, Clone, Debug)]
pub struct PlayEmote {
    pub counselor: Entity,
    pub emote: String,
}

pub(crate) fn protocol(app: &mut App) {
    app.register_message::<EmoteMsg>().add_direction(NetworkDirection::ClientToServer);
}

pub struct EmotesPlugin;

impl Plugin for EmotesPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Emotes>()
            .add_systems(Startup, load)
            .add_systems(Update, (wheel, receive))
            .add_systems(FixedUpdate, tick.before(crate::player::move_player).run_if(crate::net::simulating))
            .add_observer(play);
    }
}

fn load(mut commands: Commands, export: Res<crate::ExportRoot>) {
    match std::fs::read_to_string(export.0.join("emotes.json")).map_err(|e| e.to_string()).and_then(|t| serde_json::from_str::<Emotes>(&t).map_err(|e| e.to_string())) {
        Ok(e) => commands.insert_resource(e),
        Err(e) => warn!("no emotes.json ({e}): re-export with tools/f13-export counselor"),
    }
}

/// The section a travel highlights: the eight around the wheel, 0 at the top, clockwise (bOffsetRotation: centred on
/// their icons). PLACEHOLDER: none within 30 px.
pub fn section(aim: Vec2) -> Option<usize> {
    if aim.length() < 30.0 {
        return None;
    }
    let degrees = aim.x.atan2(-aim.y).to_degrees();
    Some(((degrees / 45.0).round() as i32).rem_euclid(8) as usize)
}

/// CanPlayEmote as f13recomp has her states: alive, on her feet on the ground, not in a car, hiding, Jason's hands or a
/// trap, no action or interaction under way, no gun raised.
#[allow(clippy::type_complexity)]
type CanEmote<'a> = (
    &'a Player,
    (Has<crate::vehicle::Driving>, Has<crate::hiding::Hiding>, Has<crate::jason::Held>, Has<crate::items::Trapped>),
    (Option<&'a crate::character::PlayAction>, Option<&'a crate::interact::Interaction>, Has<crate::items::ItemAction>, Option<&'a crate::items::GunAim>),
);

fn can_emote((p, (driving, hiding, held, trapped), (action, interaction, item_action, aim)): &(&Player, (bool, bool, bool, bool), (Option<&crate::character::PlayAction>, Option<&crate::interact::Interaction>, bool, Option<&crate::items::GunAim>))) -> bool {
    p.counselor.alive() && p.grounded && p.counselor.stance != f13_core::counselor::Stance::Swimming && !driving && !hiding && !held && !trapped && action.is_none() && !interaction.is_some_and(|i| i.busy()) && !item_action && !aim.is_some_and(|a| a.aiming)
}

/// The wheel on this machine (V held), its highlight from the mouse, and the pick as V is released.
#[allow(clippy::too_many_arguments)]
fn wheel(
    mut commands: Commands,
    keys: Res<ButtonInput<KeyCode>>,
    motion: Res<AccumulatedMouseMotion>,
    open: Option<ResMut<EmoteWheel>>,
    (emotes, profiles): (Res<Emotes>, Option<Res<crate::wardrobe::CounselorProfiles>>),
    local: Query<(Entity, CanEmote, Option<&Emoting>, Option<&crate::character::CounselorClass>), With<LocalPlayer>>,
    (client, play_jason, map): (Option<Res<crate::net::NetClient>>, Option<Res<crate::jason_player::PlayJason>>, Option<Res<crate::items::MapScreen>>),
    mut senders: Query<&mut MessageSender<EmoteMsg>, With<Connected>>,
    mut tested: Local<bool>,
) {
    // (F13_EMOTE=<emote>: played once she can; F13_EMOTE_WHEEL=<section>: the wheel held open on it. Screenshots.)
    let test_wheel = std::env::var("F13_EMOTE_WHEEL").ok().and_then(|v| v.parse::<usize>().ok());
    let mut request = None;
    if let (Ok((e, can, None, _)), Ok(emote), false) = (local.single(), std::env::var("F13_EMOTE"), *tested) {
        if can_emote(&can) {
            *tested = true;
            request = Some((e, emote));
        }
    }
    if let (Some(s), None, Ok(_), false) = (test_wheel, &open, local.single(), *tested) {
        *tested = true;
        let a = (s as f32 * 45.0).to_radians();
        let class = local.single().ok().and_then(|l| l.3).map_or(crate::character::DEFAULT_CLASS, |c| c.0.as_str());
        commands.insert_resource(EmoteWheel { aim: Vec2::new(a.sin(), -a.cos()) * 100.0, section: Some(s % 8), slots: emotes.wheel(class, profiles.as_deref()).to_vec() });
        return;
    }
    if test_wheel.is_some() {
        return;
    }
    let Ok((e, can, emoting, class)) = local.single() else {
        if open.is_some() {
            commands.remove_resource::<EmoteWheel>();
        }
        return;
    };
    let class = class.map_or(crate::character::DEFAULT_CLASS, |c| c.0.as_str());
    let slots = emotes.wheel(class, profiles.as_deref());
    match open {
        None => {
            if keys.just_pressed(KeyCode::KeyV) && play_jason.is_none() && !map.is_some_and(|m| m.0) && emoting.is_none() && can_emote(&can) && !slots.is_empty() {
                commands.insert_resource(EmoteWheel { slots: slots.to_vec(), ..default() });
            }
        }
        Some(mut wheel) => {
            wheel.aim = (wheel.aim + motion.delta).clamp_length_max(200.0);
            let s = section(wheel.aim);
            if s != wheel.section {
                wheel.section = s;
                if s.is_some() {
                    // OnUpdateHighlightRotation's MenuButtonFocusCue.
                    commands.trigger(crate::audio::PlaySound { cue: "MenuButtonFocusCue".into(), at: None });
                }
            }
            if !keys.pressed(KeyCode::KeyV) {
                commands.remove_resource::<EmoteWheel>();
                request = wheel.section.and_then(|i| wheel.slots.get(i)).filter(|e| !e.is_empty()).cloned().map(|emote| (e, emote));
            }
        }
    }
    let Some((e, emote)) = request else { return };
    info!("emote: {emote}");
    if client.is_some() {
        for mut s in &mut senders {
            s.send::<crate::lobby::LobbyChannel>(EmoteMsg(emote.clone()));
        }
    } else {
        commands.trigger(PlayEmote { counselor: e, emote });
    }
}

/// On the server: the players' SERVER_PlayEmote.
fn receive(mut commands: Commands, mut links: Query<(Entity, &mut MessageReceiver<EmoteMsg>)>, counselors: Query<(Entity, &crate::net::PlayedBy)>) {
    for (link, mut r) in &mut links {
        for EmoteMsg(emote) in r.receive() {
            if let Some((c, _)) = counselors.iter().find(|(_, by)| by.0 == link) {
                commands.trigger(PlayEmote { counselor: c, emote });
            }
        }
    }
}

/// SERVER_PlayEmote: if she can (CanPlayEmote), MULTICAST_PlayEmote: her take as her action, locked as it loops or
/// not.
fn play(ev: On<PlayEmote>, mut commands: Commands, emotes: Res<Emotes>, character: Option<Res<crate::character::CounselorCharacter>>, counselors: Query<(CanEmote, Has<crate::character::Male>)>) {
    let Some(def) = emotes.emotes.get(&ev.emote) else { return };
    let Ok((can, male)) = counselors.get(ev.counselor) else { return };
    if !can_emote(&can) {
        info!("emote: {} can't now", ev.emote);
        return;
    }
    let len = character.as_ref().and_then(|c| c.model(male).length(&def.take)).unwrap_or(0.0);
    if len <= 1e-4 {
        warn!("emote: no take {} (re-export: f13-export counselor)", def.take);
        return;
    }
    info!("emote: {} plays {} ({:.2} s{})", ev.counselor, def.name, len, if def.looping { ", looping" } else { "" });
    commands.entity(ev.counselor).insert((Emoting { emote: ev.emote.clone(), t: 0.0, len, looping: def.looping, locked: true }, crate::character::PlayAction(def.take.clone())));
}

/// Her emote along: a one-shot ends at its length (SERVER_CancelEmote), a looping one goes round; moving past the
/// lock, attacking unless locked, and crouching cancel it (MULTICAST_CancelEmote).
fn tick(mut commands: Commands, time: Res<Time>, pads: crate::controls::Pads, mut counselors: Query<(Entity, &mut Emoting, &Player)>) {
    let dt = time.delta_secs();
    for (e, mut em, p) in &mut counselors {
        let moving = pads.movement(e).length() > 1e-4;
        em.t += dt;
        let mut cancel = !p.counselor.alive() || pads.just_pressed(e, Button::Crouch);
        if em.locked {
            // (A looping one unlocks once her stick is let go.)
            if em.looping && !moving {
                em.locked = false;
            }
        } else if moving || pads.just_pressed(e, Button::Primary) {
            cancel = true;
        }
        if em.t >= em.len {
            if em.looping {
                em.t -= em.len;
                commands.entity(e).insert(crate::character::RestartAction);
            } else {
                cancel = true;
            }
        }
        if cancel {
            info!("emote: {e} ends {}", em.emote);
            commands.entity(e).remove::<(Emoting, crate::character::PlayAction)>();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_wheel_s_sections_go_clockwise_from_the_top() {
        assert_eq!(section(Vec2::new(0.0, -10.0)), None);
        assert_eq!(section(Vec2::new(0.0, -100.0)), Some(0));
        assert_eq!(section(Vec2::new(100.0, 0.0)), Some(2));
        assert_eq!(section(Vec2::new(0.0, 100.0)), Some(4));
        assert_eq!(section(Vec2::new(-100.0, -100.0)), Some(7));
    }
}
