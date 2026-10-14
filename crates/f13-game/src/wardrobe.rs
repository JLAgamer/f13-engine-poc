//! The counselors' clothing: each class's outfit and its slots' options as the player chose them in the wardrobe
//! (Counselor_Wardrobe_Menu, SCCounselorWardrobeWidget: decompiled), kept with their character profiles
//! (SaveCharacterProfiles; f13-rs: `<home>/.f13-rs/counselor_profiles.json`), sent with their counselor pick
//! (SCPlayerState ServerReceivePerksAndOutfit: lobby.rs LobbyMsg::Outfit) and worn in the match (character.rs
//! CounselorOutfit, replicated in net.rs NetCounselor).
//!
//! The wardrobe: Counselor_Menu's ClothingButton pushes it, on the counselor shown. Its OutfitList (the class's outfits
//! by SortIndex: Counselor_Outfit_Buttons with their OutfitIcon) picks the outfit (SetPreviewOutfit:
//! SetCurrentOutfit(.., bUseMenuPose)), its slot buttons (Counselor_Slot_Button, the outfit's ClothingSlots) open their
//! swatch lists (EnableSwatch: GetSelectedSwatchFromSave), whose buttons pick the slot's material (SetPreviewSwatch);
//! SaveCharacterProfiles keeps it. All unlocked (f13-rs: no entitlements or levels).

use bevy::prelude::*;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// The counselors' profiles (FSCCounselorCharacterProfile, by class): each one's outfit (as CharacterInfo::choose_outfit
/// takes it: "<outfit class>:<option>,...") and emotes (the wheel's eight, by emote class: emotes.rs).
#[derive(Resource, Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct CounselorProfiles {
    pub outfits: BTreeMap<String, String>,
    pub emotes: BTreeMap<String, Vec<String>>,
    /// Each class's three perk slots (perks.rs: perk classes; empty, none).
    pub perks: BTreeMap<String, Vec<String>>,
}

/// (F13_COUNSELOR_PROFILES: another file, for tests.)
fn path() -> std::path::PathBuf {
    std::env::var_os("F13_COUNSELOR_PROFILES").map(std::path::PathBuf::from).unwrap_or_else(|| f13_data::default_path().with_file_name("counselor_profiles.json"))
}

impl CounselorProfiles {
    pub fn load() -> Self {
        let Some(t) = std::fs::read_to_string(path()).ok() else { return Self::default() };
        // (The first files: just the outfits, by class.)
        serde_json::from_str::<BTreeMap<String, String>>(&t).map(|outfits| Self { outfits, ..default() }).or_else(|_| serde_json::from_str(&t)).unwrap_or_default()
    }

    pub(crate) fn save(&self) {
        let p = path();
        let _ = std::fs::create_dir_all(p.parent().unwrap_or(std::path::Path::new(".")));
        if let Err(e) = serde_json::to_string_pretty(self).map_err(|e| e.to_string()).and_then(|t| std::fs::write(&p, t).map_err(|e| e.to_string())) {
            warn!("wardrobe: writing {}: {e}", p.display());
        }
    }

    /// A class's outfit (empty: its default).
    pub fn outfit(&self, class: &str) -> &str {
        self.outfits.get(class).map_or("", String::as_str)
    }
}

pub(crate) fn plugin(app: &mut App) {
    if !app.world().contains_resource::<CounselorProfiles>() {
        app.insert_resource(CounselorProfiles::load());
    }
    app.add_systems(Update, (send_outfit, dress_local));
}

/// The wardrobe, open on a class: the outfit shown (an index into her Outfits) and each of its slots' option, the row
/// focused (0 the outfit list, then the slots) and the menu's arrow repeat.
#[derive(Resource, Clone, Debug)]
pub struct WardrobeMenu {
    pub class: String,
    pub outfit: usize,
    pub picks: Vec<usize>,
    pub row: usize,
}

impl WardrobeMenu {
    /// Outfit `i` shown, its slots as saved for it, else their first options (the outfit list's step).
    pub fn set_outfit(&mut self, i: usize, character: &crate::character::CounselorCharacter, profiles: &CounselorProfiles) {
        let looks = character.looks(&self.class);
        let Some(outfit) = looks.outfits.get(i) else { return };
        self.outfit = i;
        let saved = character.dressed(&self.class, profiles.outfit(&self.class)).current_outfit();
        self.picks = match saved {
            Some((o, p)) if o == i => p,
            _ => vec![0; outfit.slots.len()],
        };
        self.row = self.row.min(outfit.slots.len());
    }

    /// Its outfit as CounselorOutfit's spec.
    pub fn spec(&self, character: &crate::character::CounselorCharacter) -> String {
        character.looks(&self.class).outfit_spec(self.outfit, &self.picks).unwrap_or_default()
    }
}

/// This player's outfit and perks for their pick to the lobby (the server's own player's straight in) as they change.
fn send_outfit(
    profiles: Res<CounselorProfiles>,
    mut lobby: Query<&mut crate::lobby::NetLobby>,
    me: Option<Res<crate::lobby::Me>>,
    mut senders: Query<&mut lightyear::prelude::MessageSender<crate::lobby::LobbyMsg>, With<lightyear::prelude::Connected>>,
    mut sent: Local<Option<(String, String, Vec<String>)>>,
) {
    let Ok(mut lobby) = lobby.single_mut() else { return };
    if !lobby.open {
        return;
    }
    let id = me.as_ref().map_or(0, |m| m.id);
    let Some(m) = lobby.members.iter_mut().find(|m| m.id == id) else { return };
    let class = if m.class.is_empty() { crate::character::DEFAULT_CLASS.to_string() } else { m.class.clone() };
    let outfit = profiles.outfit(&class).to_string();
    let perks = profiles.picked_perks(&class);
    if m.outfit == outfit && m.perks == perks {
        return;
    }
    if me.is_none() {
        m.outfit = outfit;
        m.perks = perks;
        return;
    }
    // (Once per change: the server's copy comes back with the lobby.)
    if sent.as_ref() == Some(&(class.clone(), outfit.clone(), perks.clone())) {
        return;
    }
    for mut s in &mut senders {
        s.send::<crate::lobby::LobbyChannel>(crate::lobby::LobbyMsg::Outfit(outfit.clone()));
        s.send::<crate::lobby::LobbyChannel>(crate::lobby::LobbyMsg::Perks(perks.clone()));
    }
    *sent = Some((class, outfit, perks));
}

/// Without a lobby (and not a client: the server dresses its counselors), the local counselor in her class's outfit.
fn dress_local(
    mut commands: Commands,
    profiles: Res<CounselorProfiles>,
    lobby: Query<(), With<crate::lobby::NetLobby>>,
    client: Option<Res<crate::net::NetClient>>,
    local: Query<(Entity, &crate::character::CounselorClass, Option<&crate::character::CounselorOutfit>), (With<crate::controls::LocalPlayer>, With<crate::player::Player>)>,
) {
    if client.is_some() || !lobby.is_empty() {
        return;
    }
    for (e, class, outfit) in &local {
        let want = profiles.outfit(&class.0);
        if outfit.map_or(!want.is_empty(), |o| o.0 != want) {
            commands.entity(e).insert(crate::character::CounselorOutfit(want.to_string()));
        }
    }
}

/// The wardrobe (PLACEHOLDER keys: its buttons are the game's mouse and gamepad navigation): X in the counselor menu
/// opens it on the counselor shown, as her profile has her (GetSelectedSwatchFromSave); Up / Down (W / S) the outfit
/// list or a slot; Left / Right (A / D) the outfit before or after (its slots as saved for it, else their first
/// options) or the slot's option; Enter keeps it (SaveCharacterProfiles) and Escape drops it, back to the counselor
/// menu. Not ported: RandomizeOutfit, the locks and the store page.
pub(crate) fn wardrobe_menu(
    mut commands: Commands,
    keys: Res<ButtonInput<KeyCode>>,
    menu: Option<ResMut<WardrobeMenu>>,
    counselor_menu: Option<Res<crate::lobby::CounselorMenu>>,
    list: Option<Res<crate::lobby::CounselorList>>,
    character: Option<Res<crate::character::CounselorCharacter>>,
    mut profiles: ResMut<CounselorProfiles>,
    mut tested: Local<bool>,
    (emote_menu, perk_menu): (Option<Res<crate::emotes::EmoteMenu>>, Option<Res<crate::perks::PerkMenu>>),
) {
    let Some(character) = character else { return };
    // (The emote or perk menu over the counselor menu has the keys.)
    if emote_menu.is_some() || perk_menu.is_some() {
        return;
    }
    let Some(mut menu) = menu else {
        let class = counselor_menu.as_ref().and_then(|m| list.as_ref()?.0.get(m.index).cloned());
        // (F13_WARDROBE=<outfit>,<row>: opened over the counselor menu, for screenshots.)
        let test = std::env::var("F13_WARDROBE").ok().filter(|_| class.is_some() && !std::mem::replace(&mut *tested, true));
        if let Some(class) = class.filter(|_| keys.just_pressed(KeyCode::KeyX) || test.is_some()) {
            let looks = character.dressed(&class, profiles.outfit(&class));
            let Some((mut outfit, mut picks)) = looks.current_outfit() else {
                warn!("wardrobe: {class} has no outfits (re-export: f13-export counselor)");
                return;
            };
            let mut row = 0;
            if let Some(t) = test {
                let n: Vec<usize> = t.split(',').filter_map(|v| v.parse().ok()).collect();
                if let Some(&o) = n.first().filter(|&&o| o < looks.outfits.len()) {
                    outfit = o;
                    picks = vec![0; looks.outfits[o].slots.len()];
                }
                row = n.get(1).copied().unwrap_or(0).min(picks.len());
            }
            commands.insert_resource(WardrobeMenu { class, outfit, picks, row });
        }
        return;
    };
    if counselor_menu.is_none() || keys.just_pressed(KeyCode::Escape) {
        commands.remove_resource::<WardrobeMenu>();
        return;
    }
    let looks = character.looks(&menu.class);
    if keys.just_pressed(KeyCode::Enter) {
        let spec = menu.spec(&character);
        info!("wardrobe: {} wears {spec}", menu.class);
        profiles.outfits.insert(menu.class.clone(), spec);
        profiles.save();
        commands.remove_resource::<WardrobeMenu>();
        return;
    }
    let Some(outfit) = looks.outfits.get(menu.outfit) else { return };
    let rows = 1 + outfit.slots.len();
    let pressed = |a: KeyCode, b: KeyCode| keys.just_pressed(a) || keys.just_pressed(b);
    if pressed(KeyCode::ArrowUp, KeyCode::KeyW) {
        menu.row = (menu.row + rows - 1) % rows;
    } else if pressed(KeyCode::ArrowDown, KeyCode::KeyS) {
        menu.row = (menu.row + 1) % rows;
    }
    let step: isize = if pressed(KeyCode::ArrowLeft, KeyCode::KeyA) {
        -1
    } else if pressed(KeyCode::ArrowRight, KeyCode::KeyD) {
        1
    } else {
        return;
    };
    let wrap = |i: usize, n: usize| ((i as isize + step).rem_euclid(n.max(1) as isize)) as usize;
    if menu.row == 0 {
        let next = wrap(menu.outfit, looks.outfits.len());
        menu.set_outfit(next, &character, &profiles);
    } else {
        let slot = menu.row - 1;
        let n = outfit.slots[slot].options.len();
        if let Some(p) = menu.picks.get_mut(slot) {
            *p = wrap(*p, n);
        }
    }
}
