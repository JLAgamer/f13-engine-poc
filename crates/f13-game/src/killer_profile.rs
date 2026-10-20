//! Jason's loadout: the player's picks for each Jason (the game's SCCharacterSelectionsSaveGame killer profiles:
//! GrabKills, SelectedWeapon, the ability order; f13recomp: `<home>/.f13recomp/killer_profiles.json`), sent to the lobby's
//! server with the Jason pick (lobby.rs LobbyMsg::Loadout) and applied to the match's Jason by whoever plays him
//! (SCKillerCharacter::InitFromPlayerState 0x1403fb9f0: each grab kill slot the PlayerState's PickedGrabKills names, else
//! his own: jason.rs grab_kill_slots).
//!
//! The grab kill picker (Kill_Select_Widget, from Jason_Select_Widget's Kill_Select_Button1..4: OnGrabKillButton_Clicked):
//! its NonWeaponGrabKills, then the WeaponGrabKills whose weapon is his (IsMatchingWeaponType), in rows of ten
//! (GrabKillList); a kill picked goes in the slot (OnClickedWeaponGrabKill / SetGrabKill, UpdateProfile). Not ported:
//! purchases (all owned), the level and cost lines.

use std::collections::BTreeMap;

use bevy::prelude::*;
use serde::{Deserialize, Serialize};

/// Each Jason's picks, by killer Blueprint name ("Jason_SackHead").
#[derive(Resource, Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct KillerProfiles {
    /// His four grab kill slots (kill classes without "_C"; "" his own).
    pub grab_kills: BTreeMap<String, Vec<String>>,
    /// His weapon (SelectedWeapon: a weapon class; none: his WeaponClass).
    pub weapons: BTreeMap<String, String>,
    /// The Jason picked (the Jason menu: RequestKillerClass; "" the default).
    pub jason: String,
}

/// (F13_KILLER_PROFILES: another file, for tests.)
fn path() -> std::path::PathBuf {
    std::env::var_os("F13_KILLER_PROFILES").map(std::path::PathBuf::from).unwrap_or_else(|| f13_data::default_path().with_file_name("killer_profiles.json"))
}

impl KillerProfiles {
    pub fn load() -> Self {
        std::fs::read_to_string(path()).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default()
    }

    pub(crate) fn save(&self) {
        let p = path();
        let _ = std::fs::create_dir_all(p.parent().unwrap_or(std::path::Path::new(".")));
        if let Err(e) = serde_json::to_string_pretty(self).map_err(|e| e.to_string()).and_then(|t| std::fs::write(&p, t).map_err(|e| e.to_string())) {
            warn!("killer profiles: writing {}: {e}", p.display());
        }
    }

    /// A Jason's loadout as the lobby carries it.
    pub fn loadout(&self, killer: &str) -> JasonLoadout {
        JasonLoadout { jason: killer.to_string(), grab_kills: self.grab_kills.get(killer).cloned().unwrap_or_default(), weapon: self.weapons.get(killer).cloned().unwrap_or_default() }
    }

    /// The Jason picked, else the default (--jason or Part 2).
    pub fn picked_jason(&self) -> String {
        if self.jason.is_empty() {
            crate::character::killer_class().to_string()
        } else {
            self.jason.clone()
        }
    }

    /// A Jason's weapon as picked (his own when there's no pick or it's not one he may carry).
    pub fn weapon_of(&self, killer: &str, json: &serde_json::Value) -> String {
        let (own, allowed) = own_weapons(json);
        crate::jason::weapon_for(&own, &allowed, self.weapons.get(killer).map_or("", String::as_str))
    }
}

/// What the player playing Jason picked for him (the PlayerState's PickedGrabKills).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct JasonLoadout {
    /// The Jason (his killer Blueprint; "" the server's default).
    pub jason: String,
    pub grab_kills: Vec<String>,
    /// The PlayerState's picked weapon ("" his own).
    pub weapon: String,
}

impl JasonLoadout {
    /// Bounded (from the network).
    pub fn sanitized(mut self) -> Self {
        self.grab_kills.truncate(4);
        for k in &mut self.grab_kills {
            k.truncate(64);
        }
        self.weapon.truncate(64);
        self.jason.truncate(64);
        self
    }

    /// His weapon and grab kill slots from it (jason.rs: the weapon first, its DefaultGrabKill his default slot).
    pub fn apply(&self) {
        crate::jason::set_weapon(&self.weapon);
        crate::jason::set_grab_kill_picks(&self.grab_kills);
    }
}

/// The grab kill picker, open: the Jason, the slot (0..4) and the highlighted entry (into [`kill_list`]).
#[derive(Resource, Clone, Debug)]
pub struct KillMenu {
    pub killer: String,
    pub slot: usize,
    pub index: usize,
}

/// Kills in a row (GrabKillList: a NonWeaponGrabKillsBox every ten).
pub const ROW: usize = 10;

/// A Jason's own weapon (his killer JSON's Menu.WeaponClass, else his DefaultGrabKill's weapon) and the ones he may
/// carry (Menu.AllowedWeapons).
pub fn own_weapons(killer: &serde_json::Value) -> (String, Vec<String>) {
    let own = killer["Menu"]["WeaponClass"].as_str().map(str::to_string).unwrap_or_else(|| {
        let default = killer["Names"]["DefaultGrabKill"].as_str().unwrap_or("").trim_end_matches("_C");
        crate::jason::kill_index(default).map(|k| crate::jason::kills()[k].weapon.clone()).unwrap_or_default()
    });
    let allowed = killer["Menu"]["AllowedWeapons"].as_array().into_iter().flatten().filter_map(|w| w.as_str().map(str::to_string)).collect();
    (own, allowed)
}

/// The weapons a Jason's menu lists (SCKillerWeaponSelectionWidget::InitializeList: his AllowedWeaponClasses, those
/// exported; his own first when it isn't among them).
pub fn weapon_list(killer: &serde_json::Value) -> Vec<&'static crate::jason::WeaponInfo> {
    let (own, allowed) = own_weapons(killer);
    let mut list: Vec<String> = allowed;
    if !list.contains(&own) {
        list.insert(0, own);
    }
    list.iter().filter_map(|c| crate::jason::weapon_info(c)).collect()
}

/// The picker's entries for his weapon: the hand kills, then his weapon's (indexes into jason.rs kills()); and how
/// many of them are hand kills.
pub fn kill_list(weapon: &str) -> (Vec<usize>, usize) {
    let kills = crate::jason::kills();
    let hand: Vec<usize> = (0..kills.len()).filter(|&k| kills[k].weapon.is_empty()).collect();
    let n = hand.len();
    let mut all = hand;
    all.extend((0..kills.len()).filter(|&k| !kills[k].weapon.is_empty() && kills[k].weapon == weapon));
    (all, n)
}

pub struct KillerProfilePlugin;

impl Plugin for KillerProfilePlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(KillerProfiles::load()).add_systems(Update, apply_local);
    }
}

/// Playing Jason without a lobby (--play-jason): his own picks (as he's set up: jason.rs PendingLoadout).
fn apply_local(mut commands: Commands, play: Option<Res<crate::jason_player::PlayJason>>, lobby: Query<(), With<crate::lobby::NetLobby>>, profiles: Res<KillerProfiles>) {
    if play.is_some_and(|p| p.is_added()) && lobby.is_empty() {
        commands.insert_resource(crate::jason::PendingLoadout(profiles.loadout(crate::character::killer_class())));
    }
}

/// The weapon picker, open (WeaponSelectionMenu, from Jason_Select_Widget's WeaponSelectionButton): the Jason and the
/// highlighted weapon (into [`weapon_list`]).
#[derive(Resource, Clone, Debug)]
pub struct WeaponMenu {
    pub killer: String,
    pub index: usize,
}

/// The weapon picker (PLACEHOLDER keys: its buttons are mouse and gamepad widgets): Q in the Jason menu opens it
/// (WeaponSelectionButton); Left / Right (A / D) move; Enter takes the weapon (OnWeaponSelected -> SaveWeaponProfile:
/// SelectedWeapon; killer_profiles.json), Escape backs out.
pub(crate) fn weapon_menu(
    mut commands: Commands,
    keys: Res<ButtonInput<KeyCode>>,
    menu: Option<ResMut<WeaponMenu>>,
    jason_menu: Option<Res<crate::lobby::JasonMenu>>,
    list: Option<Res<crate::lobby::JasonList>>,
    kill_menu: Option<Res<KillMenu>>,
    profiles: Option<ResMut<KillerProfiles>>,
    mut tested: Local<bool>,
) {
    let Some(mut profiles) = profiles else { return };
    if kill_menu.is_some() {
        return;
    }
    let jason = jason_menu.as_ref().and_then(|m| list.as_ref()?.0.get(m.index).cloned());
    let Some(mut menu) = menu else {
        // (F13_WEAPON_MENU=<index>: opened over the Jason menu, for screenshots.)
        let test = std::env::var("F13_WEAPON_MENU").ok().filter(|_| jason.is_some() && !std::mem::replace(&mut *tested, true));
        if let Some((killer, json)) = jason.filter(|_| keys.just_pressed(KeyCode::KeyQ) || test.is_some()) {
            let weapons = weapon_list(&json);
            let current = profiles.weapon_of(&killer, &json);
            let index = test.and_then(|t| t.parse().ok()).or_else(|| weapons.iter().position(|w| w.class == current)).unwrap_or(0).min(weapons.len().saturating_sub(1));
            commands.insert_resource(WeaponMenu { killer, index });
        }
        return;
    };
    let Some((_, json)) = jason.filter(|(k, _)| *k == menu.killer) else {
        commands.remove_resource::<WeaponMenu>();
        return;
    };
    if keys.just_pressed(KeyCode::Escape) {
        commands.remove_resource::<WeaponMenu>();
        return;
    }
    let weapons = weapon_list(&json);
    if keys.just_pressed(KeyCode::Enter) {
        let Some(w) = weapons.get(menu.index) else { return };
        info!("weapon: {} carries the {}", menu.killer, w.name);
        profiles.weapons.insert(menu.killer.clone(), w.class.clone());
        profiles.save();
        commands.trigger(crate::audio::PlaySound { cue: "UITransitionAll".into(), at: None });
        commands.remove_resource::<WeaponMenu>();
        return;
    }
    let step: isize = if keys.just_pressed(KeyCode::ArrowLeft) || keys.just_pressed(KeyCode::KeyA) {
        -1
    } else if keys.just_pressed(KeyCode::ArrowRight) || keys.just_pressed(KeyCode::KeyD) {
        1
    } else {
        return;
    };
    menu.index = (menu.index as isize + step).clamp(0, weapons.len() as isize - 1) as usize;
    commands.trigger(crate::audio::PlaySound { cue: "MenuButtonFocusCue".into(), at: None });
}

/// The picker (PLACEHOLDER keys: its buttons are mouse and gamepad widgets): 1-4 in the Jason menu open it on that slot
/// (Kill_Select_Button1..4); the arrows (WASD) move by one or a row; Enter puts the kill in the slot (not if it's in
/// another: AlreadySelectedGrabKills), Delete or Backspace clears it (his own again), Escape backs out.
#[allow(clippy::too_many_arguments)]
pub(crate) fn kill_menu(
    mut commands: Commands,
    keys: Res<ButtonInput<KeyCode>>,
    menu: Option<ResMut<KillMenu>>,
    jason_menu: Option<Res<crate::lobby::JasonMenu>>,
    list: Option<Res<crate::lobby::JasonList>>,
    profiles: Option<ResMut<KillerProfiles>>,
    mut tested: Local<bool>,
    weapon_menu: Option<Res<WeaponMenu>>,
) {
    let Some(mut profiles) = profiles.filter(|_| weapon_menu.is_none()) else { return };
    let pressed = |a: KeyCode, b: KeyCode| keys.just_pressed(a) || keys.just_pressed(b);
    let jason = jason_menu.as_ref().and_then(|m| list.as_ref()?.0.get(m.index).cloned());
    let Some(mut menu) = menu else {
        // (F13_KILL_MENU=<slot>,<entry>: opened over the Jason menu, for screenshots.)
        let test = std::env::var("F13_KILL_MENU").ok().filter(|_| jason.is_some() && !std::mem::replace(&mut *tested, true));
        let n: Vec<usize> = test.as_deref().unwrap_or("").split(',').filter_map(|v| v.parse().ok()).collect();
        let slot = [KeyCode::Digit1, KeyCode::Digit2, KeyCode::Digit3, KeyCode::Digit4].iter().position(|k| keys.just_pressed(*k));
        let slot = slot.or(test.as_ref().map(|_| n.first().copied().unwrap_or(0).min(3)));
        if let (Some((killer, json)), Some(slot)) = (jason, slot) {
            let (entries, _) = kill_list(&profiles.weapon_of(&killer, &json));
            let current = profiles.grab_kills.get(&killer).and_then(|p| p.get(slot)).cloned().unwrap_or_default();
            let index = n.get(1).copied().or_else(|| entries.iter().position(|&k| crate::jason::kills()[k].class == current)).unwrap_or(0).min(entries.len().saturating_sub(1));
            commands.insert_resource(KillMenu { killer, slot, index });
        }
        return;
    };
    let Some((_, json)) = jason.filter(|(k, _)| *k == menu.killer) else {
        commands.remove_resource::<KillMenu>();
        return;
    };
    if keys.just_pressed(KeyCode::Escape) {
        commands.remove_resource::<KillMenu>();
        return;
    }
    let (entries, _) = kill_list(&profiles.weapon_of(&menu.killer, &json));
    let slots = profiles.grab_kills.entry(menu.killer.clone()).or_insert_with(|| vec![String::new(); 4]);
    slots.resize(4, String::new());
    if keys.just_pressed(KeyCode::Enter) {
        let Some(kill) = entries.get(menu.index).map(|&k| crate::jason::kills()[k].class.clone()) else { return };
        if slots.iter().enumerate().any(|(i, p)| *p == kill && i != menu.slot) {
            info!("grab kills: {kill} is in another slot");
            return;
        }
        slots[menu.slot] = kill;
        info!("grab kills: {} has {:?}", menu.killer, slots);
        profiles.save();
        commands.trigger(crate::audio::PlaySound { cue: "UITransitionAll".into(), at: None });
        commands.remove_resource::<KillMenu>();
        return;
    }
    if pressed(KeyCode::Delete, KeyCode::Backspace) {
        slots[menu.slot].clear();
        info!("grab kills: {} has {:?}", menu.killer, slots);
        profiles.save();
        commands.remove_resource::<KillMenu>();
        return;
    }
    let step: isize = if pressed(KeyCode::ArrowLeft, KeyCode::KeyA) {
        -1
    } else if pressed(KeyCode::ArrowRight, KeyCode::KeyD) {
        1
    } else if pressed(KeyCode::ArrowUp, KeyCode::KeyW) {
        -(ROW as isize)
    } else if pressed(KeyCode::ArrowDown, KeyCode::KeyS) {
        ROW as isize
    } else {
        return;
    };
    menu.index = (menu.index as isize + step).clamp(0, entries.len() as isize - 1) as usize;
    commands.trigger(crate::audio::PlaySound { cue: "MenuButtonFocusCue".into(), at: None });
}
