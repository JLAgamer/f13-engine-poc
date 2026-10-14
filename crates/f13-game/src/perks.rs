//! The counselors' perks (SCPerkData: Blueprints/Perks/*; decompiled).
//!
//! A player's perks are rolled instances (FSCPerkBackendData: the perk class and Positive / Negative / Legendary
//! modifiers, each a SCPerkData field name and a percentage) from the game's backend; each counselor profile picks
//! three (Counselor_Menu's PerkButton_Slot_1..3 open Counselor_Perk_Menu, its Perk Library in rows of ten:
//! PerkButtonLarge, SavePerk / ClearPerk), sent with her pick (ServerReceivePerksAndOutfit) and made her ActivePerks
//! (FUN_1403d0110 -> FUN_14033adc0: NewObject of the class, its CDO's values, then FUN_140331530 sets the positive and
//! negative modifiers' fields to their percentage x 0.01 and adds the legendary one's). The game reads each modifier as
//! the sum over her ActivePerks where it applies (f13-core counselor.rs: stamina and speed).
//!
//! f13-rs has no backend: every perk is in the library once, at its class's own values (PLACEHOLDER: no rolls, rarity
//! or legendary modifiers). Not ported: the perks' StartingItems (the match world sets items aside before the picks are
//! known), and the modifiers other than f13-core's stamina and speed ones.

use bevy::prelude::*;
use serde::Deserialize;
use std::collections::BTreeMap;

/// A perk (perks.json): its texts, LoadoutIcon (export-relative, no extension), its class's modifier values by
/// SCPerkData field name and StartingItems.
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct PerkDef {
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub positive: Option<String>,
    #[serde(default)]
    pub negative: Option<String>,
    #[serde(default)]
    pub legendary: Option<String>,
    #[serde(default)]
    pub icon: Option<String>,
    #[serde(default)]
    pub modifiers: BTreeMap<String, f32>,
    #[serde(default)]
    pub starting_items: Vec<String>,
}

/// Every perk, by class name ("Marathon"), in the library's order.
#[derive(Resource, Default)]
pub struct Perks(pub BTreeMap<String, PerkDef>);

impl Perks {
    /// The perk library (OwnedCounselorPerks: here every perk once).
    pub fn library(&self) -> Vec<String> {
        self.0.keys().cloned().collect()
    }

    /// Her ActivePerks' modifiers, summed.
    pub fn mods(&self, picked: &[String]) -> f13_core::counselor::PerkMods {
        let mut m = f13_core::counselor::PerkMods::default();
        for p in picked.iter().filter_map(|p| self.0.get(p)) {
            m.add(p.modifiers.iter());
        }
        m
    }
}

/// A counselor's perks (her ActivePerks: classes; set by her player's lobby pick).
#[derive(Component, Clone, Debug, Default, PartialEq)]
pub struct CounselorPerks(pub Vec<String>);

pub struct PerksPlugin;

impl Plugin for PerksPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Perks>().add_systems(Startup, load).add_systems(Update, (apply, dress_local));
    }
}

fn load(mut commands: Commands, export: Res<crate::ExportRoot>) {
    match std::fs::read_to_string(export.0.join("perks.json")).map_err(|e| e.to_string()).and_then(|t| serde_json::from_str::<BTreeMap<String, PerkDef>>(&t).map_err(|e| e.to_string())) {
        Ok(p) => commands.insert_resource(Perks(p)),
        Err(e) => warn!("no perks.json ({e}): re-export with tools/f13-export counselor"),
    }
}

/// Her perks' modifiers onto her rules (f13-core) as they change.
fn apply(perks: Res<Perks>, mut counselors: Query<(&CounselorPerks, &mut crate::player::Player), Changed<CounselorPerks>>) {
    for (picked, mut p) in &mut counselors {
        p.counselor.perks = perks.mods(&picked.0);
        info!("perks: {:?} -> {:?}", picked.0, p.counselor.perks.0);
    }
}

/// Without a lobby (and not a client: the server sets its counselors'), the local counselor's perks as her profile has
/// them for her class.
fn dress_local(
    mut commands: Commands,
    profiles: Option<Res<crate::wardrobe::CounselorProfiles>>,
    lobby: Query<(), With<crate::lobby::NetLobby>>,
    client: Option<Res<crate::net::NetClient>>,
    local: Query<(Entity, &crate::character::CounselorClass, Option<&CounselorPerks>), (With<crate::controls::LocalPlayer>, With<crate::player::Player>)>,
) {
    let Some(profiles) = profiles else { return };
    if client.is_some() || !lobby.is_empty() {
        return;
    }
    for (e, class, perks) in &local {
        let want = profiles.picked_perks(&class.0);
        if perks.map_or(!want.is_empty(), |p| p.0 != want) {
            commands.entity(e).insert(CounselorPerks(want));
        }
    }
}

/// Counselor_Perk_Menu, open on a class and a slot (PerkIndex): the library's highlighted perk.
#[derive(Resource, Clone, Debug)]
pub struct PerkMenu {
    pub class: String,
    pub slot: usize,
    pub index: usize,
}

/// Perks in a row of the library (SetupPerkList: a PerkHorizontalBox every ten).
pub const ROW: usize = 10;

/// The perk menu (PLACEHOLDER keys: its buttons are mouse and gamepad widgets): 1 / 2 / 3 in the counselor menu open it
/// on that slot (PerkButton_Slot_N); the arrows (WASD) move through the library by one or a row; Enter puts the perk in
/// the slot (SavePerk: not if it's in another; counselor_profiles.json), Delete or Backspace clears the slot
/// (OnClick_ClearSlot: ClearPerk), Escape backs out. Not ported: rolling, selling back, customization points.
#[allow(clippy::too_many_arguments)]
pub(crate) fn perk_menu(
    mut commands: Commands,
    keys: Res<ButtonInput<KeyCode>>,
    menu: Option<ResMut<PerkMenu>>,
    counselor_menu: Option<Res<crate::lobby::CounselorMenu>>,
    list: Option<Res<crate::lobby::CounselorList>>,
    perks: Res<Perks>,
    mut profiles: ResMut<crate::wardrobe::CounselorProfiles>,
    (wardrobe, emotes): (Option<Res<crate::wardrobe::WardrobeMenu>>, Option<Res<crate::emotes::EmoteMenu>>),
    mut tested: Local<bool>,
) {
    if wardrobe.is_some() || emotes.is_some() {
        return;
    }
    let pressed = |a: KeyCode, b: KeyCode| keys.just_pressed(a) || keys.just_pressed(b);
    let library = perks.library();
    let Some(mut menu) = menu else {
        let class = counselor_menu.as_ref().and_then(|m| list.as_ref()?.0.get(m.index).cloned());
        // (F13_PERK_MENU=<slot>,<library index>: opened over the counselor menu, for screenshots.)
        let test = std::env::var("F13_PERK_MENU").ok().filter(|_| class.is_some() && !std::mem::replace(&mut *tested, true));
        let slot = [KeyCode::Digit1, KeyCode::Digit2, KeyCode::Digit3].iter().position(|k| keys.just_pressed(*k));
        let n: Vec<usize> = test.as_deref().unwrap_or("").split(',').filter_map(|v| v.parse().ok()).collect();
        let slot = slot.or(test.as_ref().map(|_| n.first().copied().unwrap_or(0).min(2)));
        if let (Some(class), Some(slot)) = (class, slot.filter(|_| !library.is_empty())) {
            let current = profiles.perks.get(&class).and_then(|p| p.get(slot)).cloned().unwrap_or_default();
            let index = n.get(1).copied().or_else(|| library.iter().position(|p| *p == current)).unwrap_or(0).min(library.len() - 1);
            commands.insert_resource(PerkMenu { class, slot, index });
        }
        return;
    };
    if counselor_menu.is_none() || keys.just_pressed(KeyCode::Escape) {
        commands.remove_resource::<PerkMenu>();
        return;
    }
    let slots = profiles.perks.entry(menu.class.clone()).or_insert_with(|| vec![String::new(); 3]);
    slots.resize(3, String::new());
    if keys.just_pressed(KeyCode::Enter) {
        let Some(perk) = library.get(menu.index).cloned() else { return };
        // (SavePerk: one that's in another slot stays there.)
        if slots.iter().enumerate().any(|(i, p)| *p == perk && i != menu.slot) {
            info!("perks: {perk} is in another slot");
            return;
        }
        slots[menu.slot] = perk;
        info!("perks: {} has {:?}", menu.class, slots);
        profiles.save();
        commands.trigger(crate::audio::PlaySound { cue: "UITransitionAll".into(), at: None });
        commands.remove_resource::<PerkMenu>();
        return;
    }
    if pressed(KeyCode::Delete, KeyCode::Backspace) {
        slots[menu.slot].clear();
        info!("perks: {} has {:?}", menu.class, slots);
        profiles.save();
        commands.remove_resource::<PerkMenu>();
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
    menu.index = (menu.index as isize + step).clamp(0, library.len() as isize - 1) as usize;
    commands.trigger(crate::audio::PlaySound { cue: "MenuButtonFocusCue".into(), at: None });
}

impl crate::wardrobe::CounselorProfiles {
    /// A class's picked perks (the empty slots left out).
    pub fn picked_perks(&self, class: &str) -> Vec<String> {
        self.perks.get(class).map_or_else(Vec::new, |p| p.iter().filter(|p| !p.is_empty()).cloned().collect())
    }
}
