//! The menus' and screens' contents: the match end, the scoreboard, the lobby, the counselor, Jason, kill, weapon,
//! perk, emote, wardrobe and audio menus, and the inventory.

use super::*;

/// The match's XP by category, in the order first scored (SCMatchEndLobbyWidget DisplayPlayerScore 0x1404234e0: a line
/// per categorized score event, its category's name (0x140448fc0) and its total. PLACEHOLDER: the order, the player
/// state's categorized events' own not read). Events without a category (None) aren't shown.
pub(super) fn match_end_lines(awards: &[f13_core::game_mode::Award], tuning: Option<&crate::match_flow::MatchTuning>) -> Vec<(String, i32)> {
    let mut lines: Vec<(String, i32)> = vec![];
    for a in awards {
        let category = tuning.and_then(|t| t.events.get(&a.event)).map_or("None", |e| e.category.as_str());
        let Some(name) = category_name(category) else { continue };
        match lines.iter_mut().find(|(n, _)| n == name) {
            Some(l) => l.1 += a.xp,
            None => lines.push((name.to_string(), a.xp)),
        }
    }
    lines
}

/// EScoreEventCategory's display names (0x140448fc0).
pub(super) fn category_name(category: &str) -> Option<&'static str> {
    Some(match category {
        "Offense" => "Offense",
        "Defense" => "Defense",
        "Betrayal" => "Betrayal",
        "Fixer" => "Fixer",
        "Escaped" => "Escaped",
        "TeamEscaped" => "Team Escaped",
        "Reinforcements" => "Reinforcements",
        "CompletedMatch" => "Completed Match",
        "TimeBonus" => "Time Bonus",
        "Demasking" => "Demasking",
        "Kill" => "Kill",
        "NoSurvivors" => "No Survivors",
        "Resourceful" => "Resourceful",
        "Versatile" => "Versatile",
        "Seeker" => "Seeker",
        "DemolitionMan" => "Demolition Man",
        "BoatKill" => "Boat Kill",
        "Butcher" => "Butcher",
        "Trapper" => "Trapper",
        "PartyCrasher" => "Party Crasher",
        "DefeatedJason" => "Defeated Jason!",
        "Escort" => "Escort",
        "Sacrifice" => "Sacrifice",
        _ => return None,
    })
}

/// A lobby row's speaker is its mute button (PlayerSpeakIconMuteButton's MuteButton: a click mutes or unmutes that
/// player here, not for this player's own row; CanBeMuted). Its rows' speakers take clicks (Interaction).
pub(super) fn lobby_mute_clicks(
    mut commands: Commands,
    speakers: Query<(Entity, &Name), (Without<Interaction>, With<Node>)>,
    clicked: Query<(Entity, &Interaction, &Name), Changed<Interaction>>,
    parents: Query<&ChildOf>,
    names: Query<&Name>,
    lobby: Query<&crate::lobby::NetLobby>,
    (me, client): (Option<Res<crate::lobby::Me>>, Option<Res<crate::net::NetClient>>),
    mutes: Option<ResMut<crate::voice::VoiceMutes>>,
) {
    for (e, n) in &speakers {
        if n.as_str() == "PlayerSpeakIconMuteButton" {
            commands.entity(e).try_insert(Interaction::None);
        }
    }
    let (Some(l), Some(mut mutes)) = (lobby.iter().next().filter(|l| l.open), mutes) else { return };
    let my_id = if client.is_some() { me.map(|m| m.id) } else { Some(0) };
    for (e, i, n) in &clicked {
        if *i != Interaction::Pressed || n.as_str() != "PlayerSpeakIconMuteButton" {
            continue;
        }
        // Its row: LobbyRow<i> among its ancestors.
        let row = parents.iter_ancestors(e).find_map(|a| names.get(a).ok()?.as_str().strip_prefix("LobbyRow")?.parse::<usize>().ok());
        let Some(m) = row.and_then(|r| l.members.get(r)).filter(|m| Some(m.id) != my_id) else { continue };
        if !mutes.0.remove(&m.name) {
            mutes.0.insert(m.name.clone());
        }
        info!("lobby: {} {}", m.name, if mutes.0.contains(&m.name) { "muted" } else { "unmuted" });
    }
}

/// The match end's page: 0 its XP, then a badge each (Continue: MenuSelect, Enter here), back to 0 for the next match.
#[derive(Resource, Default)]
pub struct MatchEndPage(pub usize);

pub(super) fn match_end_page(mut page: ResMut<MatchEndPage>, keys: Res<ButtonInput<KeyCode>>, game: Option<Res<crate::match_flow::GameMatch>>) {
    let ended = game.as_ref().is_some_and(|g| matches!(g.state.state, f13_core::game_mode::MatchState::WaitingPostMatch | f13_core::game_mode::MatchState::LeavingMap));
    if !ended {
        if page.0 != 0 {
            page.0 = 0;
        }
    } else if keys.just_pressed(KeyCode::Enter) {
        page.0 += 1;
    }
}

/// A scoreboard row: their name, their status text, whether they're Jason and whether they're dead.
pub(super) type ScoreRow = (String, String, bool, bool);

/// The scoreboard's players: each counselor (her name and SCPlayerListItemWidget GetStatusDisplayText 0x14042d630:
/// "Escaped", her death's DeathScoreboardDisplayText, "Alive"), then Jason ("Killed {0}/{1}": his kills of the
/// counselors). PLACEHOLDERS: the order (the list's own not read), a death's text ("Dead"), Jason's name.
pub(super) fn scoreboard_rows(g: &crate::match_flow::GameMatch, names: &Query<&Name>) -> Vec<ScoreRow> {
    let mut rows: Vec<ScoreRow> = g
        .counselors
        .iter()
        .map(|(e, t)| {
            let name = names.get(*e).map_or("Counselor".to_string(), |n| n.to_string());
            let status = if t.record.escaped { "Escaped" } else if t.record.dead { "Dead" } else { "Alive" };
            (name, status.to_string(), false, t.record.dead && !t.record.escaped)
        })
        .collect();
    let kills = g.counselors.values().filter(|t| t.record.dead && !t.record.escaped).count();
    rows.push(("Jason".to_string(), format!("Killed {kills}/{}", g.counselors.len()), true, false));
    rows
}

/// ScoreboardWidget filled: a ScoreboardPlayerListItem per player in its ScoreBoardHUDWidget's PlayerVerticalBox,
/// each its AliveDeadPanel, its picked character (the character's ThumbnailImage; its border red for the dead and
/// Jason, else (0.552, 0.527, 0.434, 0.5); the image red for the dead), its name and its status.
pub(super) fn fill_scoreboard(hud: &Hud, tree: &mut Widget, rows: &[ScoreRow]) {
    // Its header (MenuSmallHeaderWidget: TitleText its Text, TitleBar gone with RemoveLine).
    if let Some(h) = tree.find_mut("MenuSmallHeaderWidget") {
        let (text, remove_line) = (h.props["Text"].clone(), h.props["RemoveLine"].as_bool().unwrap_or(false));
        if let Some(t) = h.find_mut("TitleText") {
            t.props["Text"] = text;
        }
        if let Some(bar) = h.find_mut("TitleBar").filter(|_| remove_line) {
            bar.set_visibility(Vis::Collapsed);
        }
    }
    let Some(row) = &hud.scoreboard_row else { return };
    let Some(list) = tree.find_mut("PlayerVerticalBox") else { return };
    list.children.clear();
    let red = json!({ "R": 0.854993, "G": 0.030713, "B": 0.017642, "A": 1.0 });
    for (i, (name, status, killer, dead)) in rows.iter().enumerate() {
        let mut w = row.clone();
        w.name = format!("ScoreboardRow{i}");
        w.slot_kind = "VerticalBoxSlot".into();
        w.slot = json!({});
        for (n, shown) in [("AliveDeadPanel", true), ("PickedCharacterPanel", true), ("PlayerStatusPanel", true), ("SpectatingPanel", false)] {
            if let Some(x) = w.find_mut(n) {
                x.set_visibility(if shown { Vis::Visible } else { Vis::Collapsed });
            }
        }
        if let (Some(i), Some(file)) = (w.find_mut("CharacterImage"), hud.characters[if *killer { "Killer" } else { "Counselor" }]["Thumbnail"].as_str()) {
            i.props["Brush"] = json!({ "ImageSize": { "X": 32.0, "Y": 32.0 }, "ResourceObject": { "File": file } });
            i.props["ColorAndOpacity"] = if *dead { red.clone() } else { json!({ "R": 1.0, "G": 1.0, "B": 1.0, "A": 1.0 }) };
        }
        if let Some(b) = w.find_mut("CharacterBorderImg") {
            b.props["ColorAndOpacity"] = if *dead || *killer { red.clone() } else { json!({ "R": 0.552012, "G": 0.527115, "B": 0.434154, "A": 0.5 }) };
        }
        if let Some(t) = w.find_mut("PlayerNameText") {
            t.props["Text"] = json!({ "LocalizedString": name });
        }
        if let Some(t) = w.find_mut("StatusText") {
            t.props["Text"] = json!({ "LocalizedString": status });
        }
        if let Some(t) = w.find_mut("PingText") {
            t.props["Text"] = json!({ "LocalizedString": "" });
        }
        list.children.push(w);
    }
}

/// MatchEnd_Badges filled with a badge (GetBadgeInfo at star level 0: its DisplayName, one-star icon and unlock
/// count; BadgeProgress "{c}/{t}", both its unlock count as the blueprint passes them), and the time until the lobby.
pub(super) fn fill_badge(hud: &Hud, tree: &mut Widget, badge: &str, closing: i32) {
    let b = &hud.badges[badge];
    if let Some(t) = tree.find_mut("BadgeName") {
        t.props["Text"] = json!({ "LocalizedString": b["Name"].as_str().unwrap_or(badge) });
    }
    if let (Some(t), Some(n)) = (tree.find_mut("BadgeProgress"), b["Unlock"].as_i64()) {
        t.props["Text"] = json!({ "LocalizedString": format!("{n}/{n}") });
    }
    if let (Some(i), Some(file)) = (tree.find_mut("BadgeImage"), b["Icon"].as_str()) {
        i.props["Brush"]["ResourceObject"] = json!({ "File": file });
    }
    if let Some(t) = tree.find_mut("TextBlock_358") {
        let c = closing.max(0);
        t.props["Text"] = json!({ "LocalizedString": format!("{:02}:{:02}", c / 60, c % 60) });
    }
}

/// MatchEnd filled: a MatchEndLine per category in ExperienceBox (OnAddMatchEndLine: CatagoryText, "{Num} xp"), the
/// total ("{0} xp": ShowTotal 0x140444550), and the time until the lobby (GetText_0: the travel countdown, mm:ss).
pub(super) fn fill_match_end(hud: &Hud, tree: &mut Widget, lines: &[(String, i32)], closing: i32) {
    let total: i32 = lines.iter().map(|l| l.1).sum();
    if let Some(t) = tree.find_mut("TotalXPText") {
        t.props["Text"] = json!({ "LocalizedString": format!("{total} xp") });
    }
    if let Some(t) = tree.find_mut("TextBlock_358") {
        let c = closing.max(0);
        t.props["Text"] = json!({ "LocalizedString": format!("{:02}:{:02}", c / 60, c % 60) });
    }
    let Some(row) = &hud.match_end_line else { return };
    let Some(list) = tree.find_mut("ExperienceBox") else { return };
    list.children.clear();
    for (i, (category, xp)) in lines.iter().enumerate() {
        let mut w = row.clone();
        w.name = format!("MatchEndLine{i}");
        w.slot_kind = "VerticalBoxSlot".into();
        w.slot = json!({});
        if let Some(t) = w.find_mut("Catagory") {
            t.props["Text"] = json!({ "LocalizedString": category });
        }
        if let Some(t) = w.find_mut("XPAmount") {
            t.props["Text"] = json!({ "LocalizedString": format!("{xp} xp") });
        }
        list.children.push(w);
    }
}

/// The lobby's player list: a LobbyPlayerListItem per member in LobbyPlayerListWidget's PlayerVerticalBox (its
/// AddPlayerWidget), each its name (PlayerName), its tint (GetPlayerTint: LocalPlayerColor for this player, else
/// RemoteUnreadyPlayerColor to RemoteReadyPlayerColor by bIsReady), its ready box (Get_ReadyBoxImage_Brush:
/// Img_Lobby_ReadyBox_Checked_01 or _Empty_01) and the host's arrow; and the travel countdown (CountdownHorizontalBox,
/// CountdownText); each row's ping and speaker. Not ported: the mute button (Shift + its number mutes here), the
/// connecting spinner, the host's Jason pick.
/// AudioSettingsMenuWidget's options (SettingsOptionSliderWidget, SettingsOptionBooleanWidget: decompiled) as the menu
/// edits them: each TitleText its OptionTitle; a slider's ValueSlider at its value and ValueText "{value x 100}%"
/// (ValueDisplayPercentage, ValueDisplayMax 100, 0 digits), a boolean's ValueText "Yes" / "No" (OnOff unset: its arrows
/// hidden). The focused row (OnSliderFocus / OnButtonFocus) shows its FocusBar, its title in HoverFont (Bold, 24) and
/// HoverColor, and a slider's ValueText (ValueTextFade's end, alpha 0.5: hidden otherwise). Not ported: the focus
/// animations (FocusPulse, FocusBurst, FocusFadeOut) and the fades (their end states shown).
pub(super) fn fill_audio_settings(tree: &mut Widget, menu: &crate::audio_settings::AudioMenu) {
    use crate::audio_settings::{Field, ROWS};
    let text = |t: &str| json!({ "LocalizedString": t });
    for (i, (name, field)) in ROWS.iter().enumerate() {
        let Some(x) = tree.find_mut(name) else { continue };
        let focused = i == menu.focus;
        let title = x.props["OptionTitle"].clone();
        let hover = x.props["HoverColor"]["SpecifiedColor"].clone();
        if let Some(t) = x.find_mut("TitleText") {
            if title.is_object() {
                t.props["Text"] = title;
            }
            if focused {
                t.props["Font"]["TypefaceFontName"] = json!("Bold");
                t.props["Font"]["Size"] = json!(24);
                if hover.is_object() {
                    t.props["ColorAndOpacity"]["SpecifiedColor"] = hover;
                }
            }
        }
        if let Some(b) = x.find_mut("FocusBar") {
            b.set_visibility(if focused { Vis::Visible } else { Vis::Hidden });
        }
        match *field {
            Field::Volume(v) => {
                let value = menu.edit.volume(v);
                if let Some(s) = x.find_mut("ValueSlider") {
                    s.props["Value"] = json!(value);
                }
                if let Some(t) = x.find_mut("ValueText") {
                    t.props["Text"] = text(&format!("{}%", (value * 100.0).round() as i32));
                    t.props["ColorAndOpacity"]["SpecifiedColor"]["A"] = json!(if focused { 0.5 } else { 0.0 });
                }
            }
            Field::Flag(f) => {
                if let Some(t) = x.find_mut("ValueText") {
                    t.props["Text"] = text(if menu.edit.flag(f) { "Yes" } else { "No" });
                }
                for arrow in ["LeftButton", "RightButton"] {
                    if let Some(a) = x.find_mut(arrow) {
                        a.set_visibility(Vis::Collapsed);
                    }
                }
            }
        }
    }
}

/// Counselor_Menu's PerkButton_Slot_1..3 (PerkButton: SetPerk / ClearPerk): each slot's perk's LoadoutIcon (PerkImage
/// shown), else the empty slot (PerkImage hidden); HighlightImg (its focus) hidden.
pub(super) fn fill_perk_slots(tree: &mut Widget, class: &str, perks: Option<&crate::perks::Perks>, profiles: Option<&crate::wardrobe::CounselorProfiles>) {
    let slots = profiles.and_then(|p| p.perks.get(class)).cloned().unwrap_or_default();
    for i in 0..3 {
        let Some(b) = tree.find_mut(&format!("PerkButton_Slot_{}", i + 1)) else { continue };
        let icon = slots.get(i).and_then(|p| perks?.0.get(p)).and_then(|d| d.icon.clone());
        if let Some(img) = b.find_mut("PerkImage") {
            match icon {
                Some(icon) => {
                    img.props["Brush"]["ResourceObject"] = json!({ "File": format!("{icon}.png") });
                    img.set_visibility(Vis::Visible);
                }
                None => img.set_visibility(Vis::Hidden),
            }
        }
        if let Some(h) = b.find_mut("HighlightImg") {
            h.set_visibility(Vis::Hidden);
        }
    }
}

/// Counselor_Perk_Menu as perks.rs has it: her names; PerkBox's PerkButtonLarges in rows of ten (SetupPerkList), each
/// its perk's LoadoutIcon, AlreadySelectedCheck on her picked ones, HighlightImg on the highlighted one; PerkName and
/// PerkDescription the highlighted perk's, Increases / Decreases its Positive / NegativeDescription. PLACEHOLDER: no
/// percentages (the rolled modifiers'), no rarity; three rows shown at a time, around the highlighted one (the rows fit
/// above PerkName: the game's library was the player's own rolls), none new (Image_0).
pub(super) fn fill_perk_menu(hud: &Hud, tree: &mut Widget, m: &crate::perks::PerkMenu, perks: &crate::perks::Perks, profiles: &crate::wardrobe::CounselorProfiles, counselors: Option<&crate::character::CounselorCharacter>) {
    let text = |t: &str| json!({ "LocalizedString": t });
    let name = counselors.map(|c| c.looks(&m.class)).and_then(|l| l.names.get("CharacterName").cloned()).unwrap_or_else(|| m.class.clone());
    let (first, rest) = name.split_once(' ').unwrap_or((name.as_str(), ""));
    let library = perks.library();
    let picked = profiles.picked_perks(&m.class);
    let def = library.get(m.index).and_then(|p| perks.0.get(p));
    let (positive, negative) = (def.and_then(|d| d.positive.clone()).unwrap_or_default(), def.and_then(|d| d.negative.clone()).unwrap_or_default());
    for (n, t) in [
        ("FirstNameText", first.to_string()),
        ("LastNameText", rest.to_string()),
        ("PerkName", def.map_or(String::new(), |d| d.name.clone())),
        ("PerkDescription", def.and_then(|d| d.description.clone()).unwrap_or_default()),
        ("IncreaseDescriptionText", positive.clone()),
        ("DecreaseDescriptionText", negative.clone()),
        ("IncreasePercentText", String::new()),
        ("DecreasePrecentageText", String::new()),
    ] {
        if let Some(x) = tree.find_mut(n) {
            x.props["Text"] = text(&t);
        }
    }
    for (row, shown) in [("Increases", !positive.is_empty()), ("Decreases", !negative.is_empty())] {
        if let Some(x) = tree.find_mut(row) {
            x.set_visibility(if shown { Vis::Visible } else { Vis::Hidden });
        }
    }
    let (Some(button), Some(b)) = (&hud.perk_button, tree.find_mut("PerkBox")) else { return };
    let rows = library.len().div_ceil(crate::perks::ROW);
    let first = (m.index / crate::perks::ROW).saturating_sub(1).min(rows.saturating_sub(3));
    b.children = library
        .chunks(crate::perks::ROW)
        .enumerate()
        .skip(first)
        .take(3)
        .map(|(r, row)| {
            let buttons = row
                .iter()
                .enumerate()
                .map(|(k, p)| {
                    let i = r * crate::perks::ROW + k;
                    let mut x = button.clone();
                    x.name = format!("Perk_{i}");
                    x.slot_kind = "HorizontalBoxSlot".into();
                    x.slot = json!({});
                    if let (Some(img), Some(icon)) = (x.find_mut("PerkImage"), perks.0.get(p).and_then(|d| d.icon.clone())) {
                        img.props["Brush"]["ResourceObject"] = json!({ "File": format!("{icon}.png") });
                    }
                    for (n, shown) in [("HighlightImg", i == m.index), ("AlreadySelectedCheck", picked.contains(p)), ("Rarity", false), ("Image_0", false)] {
                        if let Some(h) = x.find_mut(n) {
                            h.set_visibility(if shown { Vis::Visible } else { Vis::Hidden });
                        }
                    }
                    x
                })
                .collect();
            Widget::new("HorizontalBox", &format!("PerkRow_{r}"), json!({}), "VerticalBoxSlot", json!({}), buttons)
        })
        .collect();
}

/// Counselor_Emote_Menu as emotes.rs has it: her names; its wheel (CounselorEmoteSelect: her eight, the section
/// highlighted) or, the panel shown (ShowEmoteBox: the wheel hidden), CounselorEmotePanel's EmoteBox of
/// EmoteButtonLarges, rows of nine (PerkHorizontalBox: AddEmote), each its emote's icon, the highlighted one's
/// HighlightImg; EmoteName the highlighted emote's name (OnUpdateHighlightRotation, the panel's highlight).
pub(super) fn fill_emote_menu(hud: &Hud, tree: &mut Widget, m: &crate::emotes::EmoteMenu, emotes: &crate::emotes::Emotes, counselors: Option<&crate::character::CounselorCharacter>) {
    let text = |t: &str| json!({ "LocalizedString": t });
    let name = counselors.map(|c| c.looks(&m.class)).and_then(|l| l.names.get("CharacterName").cloned()).unwrap_or_else(|| m.class.clone());
    let (first, rest) = name.split_once(' ').unwrap_or((name.as_str(), ""));
    for (n, t) in [("FirstNameText", first), ("LastNameText", rest)] {
        if let Some(x) = tree.find_mut(n) {
            x.props["Text"] = text(t);
        }
    }
    let available = emotes.available();
    let highlighted = match m.list {
        Some(i) => available.get(i).cloned(),
        None => m.slots.get(m.section).cloned(),
    };
    let label = highlighted.and_then(|e| emotes.emotes.get(&e)).map_or(String::new(), |d| d.name.clone());
    if let Some(x) = tree.find_mut("EmoteName") {
        x.props["Text"] = text(&label);
    }
    if let Some(select) = tree.find_mut("CounselorEmoteSelect") {
        select.set_visibility(if m.list.is_some() { Vis::Hidden } else { Vis::Visible });
        fill_emote_wheel(select, &m.slots, Some(m.section), emotes);
    }
    let Some(panel) = tree.find_mut("CounselorEmotePanel") else { return };
    panel.set_visibility(if m.list.is_some() { Vis::Visible } else { Vis::Hidden });
    // (ShowEmoteList's end: the panel faded in from its alpha 0 as built.)
    panel.props["ColorAndOpacity"]["A"] = json!(1.0);
    let (Some(i), Some(button), Some(b)) = (m.list, &hud.emote_button, panel.find_mut("EmoteBox")) else { return };
    b.children = available
        .chunks(9)
        .enumerate()
        .map(|(r, row)| {
            let buttons = row
                .iter()
                .enumerate()
                .map(|(k, e)| {
                    let mut x = button.clone();
                    x.name = format!("Emote_{}", r * 9 + k);
                    x.slot_kind = "HorizontalBoxSlot".into();
                    x.slot = json!({});
                    if let (Some(img), Some(icon)) = (x.find_mut("EmoteImage"), emotes.emotes.get(e).and_then(|d| d.icon.clone())) {
                        img.props["Brush"]["ResourceObject"] = json!({ "File": format!("{icon}.png") });
                    }
                    if let Some(h) = x.find_mut("HighlightImg") {
                        h.set_visibility(if r * 9 + k == i { Vis::Visible } else { Vis::Hidden });
                    }
                    x
                })
                .collect();
            Widget::new("HorizontalBox", &format!("EmoteRow_{r}"), json!({}), "VerticalBoxSlot", json!({}), buttons)
        })
        .collect();
}

/// Counselor_Emote_Select as emotes.rs has it: each EmoteIcon's EmoteImage its emote's EmoteIcon (SetEmoteIcon), the
/// highlight (HLGradient, HLLines, Arrow: GetHighlightVisibility, OnUpdateHighlightRotation) turned to the highlighted
/// section (45 degrees each, about the wheel's centre: their RenderTransformPivot), hidden with none.
pub(super) fn fill_emote_wheel(tree: &mut Widget, slots: &[String], section: Option<usize>, emotes: &crate::emotes::Emotes) {
    for (i, name) in slots.iter().enumerate().take(8) {
        let icon = emotes.emotes.get(name).and_then(|e| e.icon.clone());
        if let (Some(x), Some(icon)) = (tree.find_mut(&format!("EmoteIcon{i}")).and_then(|x| x.find_mut("EmoteImage")), icon) {
            x.props["Brush"]["ResourceObject"] = json!({ "File": format!("{icon}.png") });
        }
    }
    for name in ["HLGradient", "HLLines", "Arrow"] {
        let Some(x) = tree.find_mut(name) else { continue };
        match section {
            Some(s) => {
                x.set_visibility(Vis::Visible);
                x.props["RenderTransform"] = json!({ "Angle": s as f64 * 45.0 });
            }
            None => x.set_visibility(Vis::Hidden),
        }
    }
}

/// The wardrobe (Counselor_Wardrobe_Menu, SCCounselorWardrobeWidget: decompiled) as wardrobe.rs has it: her names
/// (PreviewClassFirstName / LastName); the OutfitList's Counselor_Outfit_Buttons (each outfit's OutfitIcon, the worn
/// one's SelectedImage); the outfit's slot buttons (their SlotIcon; the slots it lacks collapsed), the focused slot's
/// swatch list (EnableSwatch: its SlotIcon and a Counselor_Swatch_Button per option, its SwatchIcon, the chosen one's
/// SelectedImage); DescriptionText the focused row's (OnOutfitHighlighted / OnSwatchHighlighted's text: the outfit's
/// or slot's name). All unlocked (LockImage collapsed). PLACEHOLDER: the focused slot shown by its button's
/// HighlightBorder (the buttons' focus styles not read).
pub(super) fn fill_wardrobe(hud: &Hud, tree: &mut Widget, w: &crate::wardrobe::WardrobeMenu, counselors: &crate::character::CounselorCharacter) {
    let looks = counselors.looks(&w.class);
    let text = |t: &str| json!({ "LocalizedString": t });
    let image = |path: &Option<String>| path.as_ref().map(|p| json!({ "File": format!("{p}.png") }));
    let name = looks.names.get("CharacterName").cloned().unwrap_or_else(|| w.class.clone());
    let (first, rest) = name.split_once(' ').unwrap_or((name.as_str(), ""));
    for (n, t) in [("FirstNameText", first), ("LastNameText", rest)] {
        if let Some(x) = tree.find_mut(n) {
            x.props["Text"] = text(t);
        }
    }
    let Some(outfit) = looks.outfits.get(w.outfit) else { return };
    let description = if w.row == 0 { outfit.name.clone() } else { outfit.slots.get(w.row - 1).and_then(|s| s.name.clone()).unwrap_or_default() };
    if let Some(x) = tree.find_mut("DescriptionText") {
        x.props["Text"] = text(&description);
    }
    if let (Some(list), Some(button)) = (tree.find_mut("OutfitList").and_then(|l| l.find_mut("ScrollBox_0")), &hud.outfit_button) {
        list.children = looks
            .outfits
            .iter()
            .enumerate()
            .map(|(i, o)| {
                let mut b = button.clone();
                b.name = format!("Outfit_{i}");
                b.slot_kind = "ScrollBoxSlot".into();
                b.slot = json!({});
                if let (Some(p), Some(r)) = (b.find_mut("PreviewImage"), image(&o.icon)) {
                    p.props["Brush"]["ResourceObject"] = r;
                }
                if let Some(s) = b.find_mut("SelectedImage") {
                    s.set_visibility(if i == w.outfit { Vis::Visible } else { Vis::Hidden });
                }
                if let Some(l) = b.find_mut("LockImage") {
                    l.set_visibility(Vis::Collapsed);
                }
                b
            })
            .collect();
    }
    for k in 0..3 {
        let slot = outfit.slots.get(k);
        if let Some(b) = tree.find_mut(&format!("Slot_Button_0{}", k + 1)) {
            match slot {
                Some(s) => {
                    if let (Some(i), Some(r)) = (b.find_mut("Image_0"), image(&s.icon)) {
                        i.props["Brush"]["ResourceObject"] = r;
                    }
                    if let Some(h) = b.find_mut("HighlightBorder") {
                        h.set_visibility(if w.row == k + 1 { Vis::Visible } else { Vis::Hidden });
                    }
                }
                None => b.set_visibility(Vis::Collapsed),
            }
        }
        let list_name = if k == 0 { "Counselor_Slot_01_Swatch_List_195".to_string() } else { format!("Counselor_Slot_0{}_Swatch_List", k + 1) };
        let Some(list) = tree.find_mut(&list_name) else { continue };
        let Some(s) = slot.filter(|_| w.row == k + 1) else {
            list.set_visibility(Vis::Collapsed);
            continue;
        };
        list.set_visibility(Vis::Visible);
        if let (Some(i), Some(r)) = (list.find_mut("SlotIcon"), image(&s.icon)) {
            i.props["Brush"]["ResourceObject"] = r;
        }
        let pick = w.picks.get(k).copied().unwrap_or(0);
        if let (Some(b), Some(button)) = (list.find_mut("SwatchListBox"), &hud.swatch_button) {
            b.children = (0..s.options.len())
                .map(|i| {
                    let mut sw = button.clone();
                    sw.name = format!("Swatch_{i}");
                    sw.slot_kind = "WrapBoxSlot".into();
                    sw.slot = json!({});
                    if let (Some(img), Some(r)) = (sw.find_mut("Image_0"), image(&s.option_icons.get(i).cloned().flatten())) {
                        img.props["Brush"]["ResourceObject"] = r;
                    }
                    if let Some(x) = sw.find_mut("SelectedImage") {
                        x.set_visibility(if i == pick { Vis::Visible } else { Vis::Hidden });
                    }
                    if let Some(l) = sw.find_mut("LockImage") {
                        l.set_visibility(Vis::Collapsed);
                    }
                    sw
                })
                .collect();
        }
    }
}

/// Counselor_Menu's SetCounselor (decompiled): FirstNameText the counselor's CharacterName's first word, LastNameText the
/// rest; NameText and DescriptionText cleared (they show the focused stat, perk or ability's, not ported); each
/// Stat_Widget's UpdateStat: StatAmount her stat (GetCounselorStatAmount), StatBar at it / 10, TitleName its StatName.
pub(super) fn fill_counselor_menu(tree: &mut Widget, class: &str, counselors: Option<&crate::character::CounselorCharacter>, tuning: &crate::player::PlayerTuning) {
    use f13_core::stats::Stat;
    let name = counselors.map(|c| c.looks(class)).and_then(|l| l.names.get("CharacterName").cloned()).unwrap_or_else(|| class.to_string());
    let mut words = name.split(' ');
    let first = words.next().unwrap_or("").to_string();
    let rest = words.collect::<Vec<_>>().join(" ");
    let text = |t: &str| json!({ "LocalizedString": t });
    for (w, t) in [("FirstNameText", first), ("LastNameText", rest), ("NameText", String::new()), ("DescriptionText", String::new())] {
        if let Some(x) = tree.find_mut(w) {
            x.props["Text"] = text(&t);
        }
    }
    let stats = tuning.stats_of(class);
    // (Its RepairStat is Intelligence.)
    for (w, stat) in [("ComposureStat", Stat::Composure), ("LuckStat", Stat::Luck), ("RepairStat", Stat::Intelligence), ("SpeedStat", Stat::Speed), ("StaminaStat", Stat::Stamina), ("StealthSTat", Stat::Stealth), ("StrengthStat", Stat::Strength)] {
        let Some(x) = tree.find_mut(w) else { continue };
        let title = x.props["StatName"].clone();
        let v = stats[stat as usize];
        if let Some(t) = x.find_mut("TitleName") {
            if title.is_object() {
                t.props["Text"] = title;
            }
        }
        if let Some(t) = x.find_mut("StatAmount") {
            t.props["Text"] = text(&v.to_string());
        }
        if let Some(b) = x.find_mut("StatBar") {
            b.props["Percent"] = json!(v as f32 / 10.0);
        }
    }
}

/// Jason_Select_Widget's SetJason (decompiled): FirstNameText his CharacterName ("Part"), MovieNumber his
/// CharacterTropeName ("3"), WeaponAxeText "Weapon: {WeaponName}" (his default weapon: no picks), StrengthBox and
/// WeaknessesBox an Attribute_Display per strength / weakness (AtrributeText its Title, AttrImg its Icon); SetupGrabKills:
/// Kill_Select_Button1..4 his grab kills (Kill_Select_Button SetGrabKill: GrabKillImg GetGrabKillImage(class, false),
/// the kill's ContextKillHUDIcon; none: KillSelectionEmpty_01, its default). The profile's GrabKills taken as his own
/// slots (InitFromPlayerState with no picks: jason.rs grab_kill_slots). NameText and DescriptionText cleared (the
/// focused button's).
pub(super) fn fill_jason_menu(hud: &Hud, tree: &mut Widget, killer: &Value, picks: &[String], weapon_class: &str) {
    let menu = &killer["Menu"];
    let text = |t: &str| json!({ "LocalizedString": t });
    // His weapon (the profile's SelectedWeapon, else his own): its name, and WeaponSelectionButton's image
    // (SCKillerWeaponSelectionButton::GetButtonImage: its ThumbnailImage).
    let w = crate::jason::weapon_info(weapon_class);
    let weapon = format!("Weapon: {}", w.map_or_else(|| menu["WeaponName"].as_str().unwrap_or("").to_string(), |w| w.name.clone()));
    if let (Some(img), Some(w)) = (tree.find_mut("WeaponSelectionButton").and_then(|b| b.find_mut("Image_0")), w.filter(|w| !w.image.is_empty())) {
        img.props["Brush"]["ResourceObject"] = json!({ "File": w.image });
    }
    for (w, t) in [("FirstNameText", menu["CharacterName"].as_str().unwrap_or("")), ("MovieNumber", menu["CharacterTropeName"].as_str().unwrap_or("")), ("WeaponAxeText", weapon.as_str()), ("NameText", ""), ("DescriptionText", "")] {
        if let Some(x) = tree.find_mut(w) {
            x.props["Text"] = text(t);
        }
    }
    if let Some(row) = &hud.attribute {
        for (b, key) in [("StrengthBox", "Strengths"), ("WeaknessesBox", "Weaknesses")] {
            let Some(bx) = tree.find_mut(b) else { continue };
            bx.children = menu[key]
                .as_array()
                .into_iter()
                .flatten()
                .enumerate()
                .map(|(i, a)| {
                    let mut x = row.clone();
                    x.name = format!("{key}_{i}");
                    x.slot_kind = "VerticalBoxSlot".into();
                    x.slot = json!({});
                    if let Some(t) = x.find_mut("AtrributeText") {
                        t.props["Text"] = text(a["Title"].as_str().unwrap_or(""));
                    }
                    if let (Some(img), Some(icon)) = (x.find_mut("AttrImg"), a["Icon"].as_str()) {
                        img.props["Brush"]["ResourceObject"] = json!({ "File": icon });
                    }
                    x
                })
                .collect();
        }
    }
    // His slots: his picks, else his own (GrabKillClasses in order, the first empty one his weapon's DefaultGrabKill).
    let mut names = killer["Names"].clone();
    if let Some(w) = w {
        names["DefaultGrabKill"] = json!(format!("{}_C", w.default_grab_kill));
    }
    let own = crate::jason::slots_from(&names);
    let slots: Vec<Option<usize>> = (0..4).map(|i| picks.get(i).filter(|p| !p.is_empty()).and_then(|p| crate::jason::kill_index(p)).or(own[i])).collect();
    for (i, kill) in slots.iter().enumerate() {
        let icon = kill.and_then(|k| hud.textures.get(&crate::jason::kills()[k].icon).cloned()).or_else(|| {
            let class = kill.map(|k| format!("{}_C", crate::jason::kills()[k].class));
            class.as_deref().and_then(|c| hud.classes.prop(c, "ContextKillHUDIcon").as_str()).and_then(|s| s.split('\'').nth(1)).and_then(|n| hud.textures.get(n).cloned())
        });
        let Some(b) = tree.find_mut(&format!("Kill_Select_Button{}", i + 1)) else { continue };
        if let (Some(img), Some(icon)) = (b.find_mut("GrabKillImg"), icon) {
            img.props["Brush"]["ResourceObject"] = json!({ "File": icon });
        }
        if let Some(h) = b.find_mut("HighlightImg") {
            h.set_visibility(Vis::Hidden);
        }
    }
}

/// Kill_Select_Widget (decompiled): FirstNameText / MovieNumber as the Jason menu's; NonWeaponGrabKillsVeritcalBox and
/// WeaponGrabKillBox a GrabKillList (a row) every ten of its hand kills, then of his weapon's (killer_profile.rs
/// kill_list), each a GrabKillButtonLarge (GrabKillImage the kill's GetGrabKillImage(class, false): its
/// ContextKillHUDIcon; AlreadySelectedCheck when it's in another slot; HighlightImg on the focused one); NameText and
/// DescriptionText the focused kill's FriendlyName and Description (OnFocused_WeaponGrabKill).
pub(super) fn fill_kill_menu(hud: &Hud, tree: &mut Widget, m: &crate::killer_profile::KillMenu, killer: &Value, picks: &[String], weapon: &str) {
    let menu = &killer["Menu"];
    let text = |t: &str| json!({ "LocalizedString": t });
    let kills = crate::jason::kills();
    let (entries, hand) = crate::killer_profile::kill_list(weapon);
    let focused = entries.get(m.index).map(|&k| &kills[k]);
    for (w, t) in [
        ("FirstNameText", menu["CharacterName"].as_str().unwrap_or("")),
        ("MovieNumber", menu["CharacterTropeName"].as_str().unwrap_or("")),
        ("NameText", focused.map_or("", |k| k.title.as_str())),
        ("DescriptionText", focused.map_or("", |k| k.description.as_str())),
    ] {
        if let Some(x) = tree.find_mut(w) {
            x.props["Text"] = text(t);
        }
    }
    let Some(button) = &hud.kill_button else { return };
    let rows = |range: std::ops::Range<usize>, tag: &str| -> Vec<Widget> {
        let list: Vec<usize> = range.collect();
        list.chunks(crate::killer_profile::ROW)
            .enumerate()
            .map(|(r, row)| {
                let buttons = row
                    .iter()
                    .map(|&i| {
                        let k = &kills[entries[i]];
                        let mut x = button.clone();
                        x.name = format!("GrabKill_{i}");
                        x.slot_kind = "HorizontalBoxSlot".into();
                        x.slot = json!({});
                        if let (Some(img), Some(icon)) = (x.find_mut("GrabKillImage"), hud.textures.get(&k.icon)) {
                            img.props["Brush"]["ResourceObject"] = json!({ "File": icon });
                        }
                        let elsewhere = picks.iter().enumerate().any(|(s, p)| *p == k.class && s != m.slot);
                        for (n, shown) in [("HighlightImg", i == m.index), ("AlreadySelectedCheck", elsewhere)] {
                            if let Some(h) = x.find_mut(n) {
                                h.set_visibility(if shown { Vis::Visible } else { Vis::Hidden });
                            }
                        }
                        x
                    })
                    .collect();
                Widget::new("HorizontalBox", &format!("{tag}Row_{r}"), json!({}), "VerticalBoxSlot", json!({}), buttons)
            })
            .collect()
    };
    if let Some(b) = tree.find_mut("NonWeaponGrabKillsVeritcalBox") {
        b.children = rows(0..hand, "Hand");
    }
    if let Some(b) = tree.find_mut("WeaponGrabKillBox") {
        b.children = rows(hand..entries.len(), "Weapon");
    }
}

/// WeaponSelectionMenu (SCKillerWeaponSelectionMenu::InitalizeMenu): FirstNameText / Trope his names; its
/// WeaponSelectionWidget's HorizontalBox a KillerWeaponListItem per weapon he may carry (InitializeList: his
/// AllowedWeaponClasses; TextBlock_0 its FriendlyName, ButtonImage its ThumbnailImage: GetFriendlyName,
/// GetWeaponThumbnail). (PLACEHOLDER: the focused one at full opacity, the others at half; the game's focus look not
/// read.)
pub(super) fn fill_weapon_menu(hud: &Hud, tree: &mut Widget, m: &crate::killer_profile::WeaponMenu, killer: &Value) {
    let menu = &killer["Menu"];
    let text = |t: &str| json!({ "LocalizedString": t });
    for (w, t) in [("FirstNameText", menu["CharacterName"].as_str().unwrap_or("")), ("Trope", menu["CharacterTropeName"].as_str().unwrap_or(""))] {
        if let Some(x) = tree.find_mut(w) {
            x.props["Text"] = text(t);
        }
    }
    let (Some(item), Some(list)) = (&hud.weapon_item, tree.find_mut("HorizontalBox_0")) else { return };
    list.children = crate::killer_profile::weapon_list(killer)
        .iter()
        .enumerate()
        .map(|(i, w)| {
            let mut x = item.clone();
            x.name = format!("Weapon_{i}");
            x.slot_kind = "HorizontalBoxSlot".into();
            x.slot = json!({});
            if let Some(t) = x.find_mut("TextBlock_0") {
                t.props["Text"] = text(&w.name);
            }
            if let Some(img) = x.find_mut("ButtonImage") {
                if !w.image.is_empty() {
                    img.props["Brush"]["ResourceObject"] = json!({ "File": w.image });
                }
                img.props["ColorAndOpacity"] = json!({ "R": 1.0, "G": 1.0, "B": 1.0, "A": if i == m.index { 1.0 } else { 0.5 } });
            }
            x
        })
        .collect();
}

pub(super) fn fill_lobby(hud: &Hud, tree: &mut Widget, lobby: &crate::lobby::NetLobby, me: Option<u64>, mutes: Option<&crate::voice::VoiceMutes>, ready_focus: bool) {
    if let Some(cd) = tree.find_mut("CountdownHorizontalBox") {
        cd.set_visibility(if lobby.countdown.is_some() { Vis::Visible } else { Vis::Hidden });
    }
    if let (Some(t), Some(text)) = (lobby.countdown, tree.find_mut("CountdownText")) {
        text.props["Text"] = json!({ "LocalizedString": format!("{}", t.ceil().max(0.0) as i32) });
    }
    // The map (USCLobbyWidget's map display: its definition's DisplayName and LobbyThumbnailImages).
    if let (Some(name), Some(text)) = (hud.lobby_map["Name"].as_str(), tree.find_mut("MapNameText")) {
        text.props["Text"] = json!({ "LocalizedString": name });
    }
    if let (Some(file), Some(image)) = (hud.lobby_map["Image"].as_str(), tree.find_mut("CurrentMapImage")) {
        image.props["Brush"]["ResourceObject"] = json!({ "File": file });
    }
    // The ready bar: its glow while the ready button has the focus (its Tick: ReadyFocused, which its hover sets and
    // clears: lobby.rs ReadyFocus; on at first, the default focus, as Enter readies here), its text the other way from
    // her (ReadyText, UnreadyText).
    let ready = lobby.members.iter().any(|m| Some(m.id) == me && m.ready);
    if let Some(bar) = tree.find_mut("StartBarMaterialImage") {
        bar.props["Brush"]["ResourceObject"]["Scalars"]["Glow"] = json!(if ready_focus { 1.0 } else { 0.0 });
    }
    if let Some(t) = tree.find_mut("ReadyUnreadyText") {
        t.props["Text"] = json!({ "LocalizedString": if ready { "U N R E A D Y" } else { "R E A D Y" } });
    }
    let Some((row, textures, defaults)) = &hud.lobby_row else { return };
    let Some(list) = tree.find_mut("PlayerVerticalBox") else { return };
    list.children.clear();
    let tint = |k: &str| color(&defaults[k], [1.0; 4]);
    for (i, m) in lobby.members.iter().enumerate() {
        let mut w = row.clone();
        w.name = format!("LobbyRow{i}");
        w.slot_kind = "VerticalBoxSlot".into();
        w.slot = json!({});
        let c = if Some(m.id) == me {
            tint("LocalPlayerColor")
        } else if m.ready {
            tint("RemoteReadyPlayerColor")
        } else {
            tint("RemoteUnreadyPlayerColor")
        };
        // Their picks (Get_HasPlayerFullyLoaded_Visibility: shown once they're in): the counselor button their
        // PickedCounselorClass's thumbnail, Jason's the killer's (CharacterSelectButtonWidget Get_CharacterImage_Brush:
        // GetCharacterThumbnailImage, 58 x 58).
        for (button, file) in [
            ("CounselorButton", hud.characters["Classes"][if m.class.is_empty() { crate::character::DEFAULT_CLASS } else { m.class.as_str() }].as_str()),
            ("JasonButton", hud.characters["Killer"]["Thumbnail"].as_str()),
        ] {
            let Some(b) = w.find_mut(button) else { continue };
            b.set_visibility(Vis::Visible);
            if let (Some(file), Some(image)) = (file, b.find_mut("CharacterImage")) {
                image.props["Brush"] = json!({ "ImageSize": { "X": 58.0, "Y": 58.0 }, "ResourceObject": { "File": file } });
                image.set_visibility(Vis::Visible);
            }
        }
        if let Some(name) = w.find_mut("PlayerName") {
            name.props["Text"] = json!({ "LocalizedString": m.name });
            name.props["ColorAndOpacity"] = json!({ "SpecifiedColor": { "R": c[0], "G": c[1], "B": c[2], "A": c[3] } });
        }
        let ready = if m.ready { "Img_Lobby_ReadyBox_Checked_01" } else { "Img_Lobby_ReadyBox_Empty_01" };
        if let (Some(file), Some(image)) = (textures.get(ready), w.find_mut("ReadyBoxImage")) {
            image.props["Brush"]["ResourceObject"] = json!({ "File": file });
            image.set_visibility(Vis::Visible);
        }
        // Its ping: "{0}ms" (ILLPlayerState GetPingDisplayText 0x140269480: none at 0), good to 150 ms, bad to 300,
        // very bad past (Get_PingText_ColorAndOpacity). PLACEHOLDER: the round trip for its Ping (the player state's
        // replicated ping, x4).
        if let Some(t) = w.find_mut("PingText") {
            let ping = m.ping.min(999);
            let c = color(&defaults[if ping <= 150 { "PingGoodColor" } else if ping <= 300 { "PingBadColor" } else { "PingVeryBadColor" }], [1.0; 4]);
            t.props["Text"] = json!({ "LocalizedString": if ping == 0 { String::new() } else { format!("{ping}ms") } });
            t.props["ColorAndOpacity"] = json!({ "SpecifiedColor": { "R": c[0], "G": c[1], "B": c[2], "A": c[3] } });
        }
        // Its speaker: on while they talk, its alpha 0.15, 0.35 once they've talked, 0.9 while they talk
        // (Get_SpeakingIcon_Brush, Get_SpeakingIcon_ColorAndOpacity); muted here, the mute overlay.
        if let Some(icon) = w.find_mut("SpeakingIcon") {
            let file = textures.get(if m.talking { "Img_Speaker_On_01" } else { "Img_Speaker_Off_01" });
            if let Some(file) = file {
                icon.props["Brush"]["ResourceObject"] = json!({ "File": file });
            }
            let a = if m.talking { 0.9 } else if m.talked { 0.35 } else { 0.15 };
            icon.props["ColorAndOpacity"] = json!({ "R": 1.0, "G": 1.0, "B": 1.0, "A": a });
        }
        if let Some(muted) = w.find_mut("MutedIcon") {
            let shown = Some(m.id) != me && mutes.is_some_and(|x| x.0.contains(&m.name));
            muted.set_visibility(if shown { Vis::Visible } else { Vis::Hidden });
        }
        if let Some(h) = w.find_mut("HostArrowImage") {
            h.set_visibility(if m.id == 0 { Vis::Visible } else { Vis::Hidden });
        }
        for (name, shown) in [
            ("ConnectingSpinner", false),
            ("KillerPickerButtonWidget_0", false),
            ("SelectionBar", Some(m.id) == me),
        ] {
            if let Some(x) = w.find_mut(name) {
                x.set_visibility(if shown { Vis::Visible } else { Vis::Hidden });
            }
        }
        list.children.push(w);
    }
}

/// Puts an item widget in the inventory's Grid for each pocket item (SCInventory_Widget does it as items are
/// picked up): its ImageOn icon, the count hidden (one item each); ObjectText names the first.
/// SCInventory_Widget's InventoryChanged: an SC_InventoryItem_Widget per small item, Active for the selected one
/// (SmallItemIndex: ImageOn shown, else ImageOff), and ObjectText the selected item's name. Not here: the uses left
/// (NumUses - TimesUsed, shown above 1).
pub(super) fn fill_inventory(hud: &Hud, tree: &mut Widget, pockets: &[String], selected: usize) {
    if let Some(text) = tree.find_mut("ObjectText") {
        match pockets.get(selected) {
            Some(class) => {
                let name = hud.classes.display_name(class);
                text.props["Text"] = json!({ "LocalizedString": name });
                text.set_visibility(Vis::Visible);
            }
            None => text.set_visibility(Vis::Collapsed),
        }
    }
    let (Some(item), Some(grid)) = (&hud.item, tree.find_mut("Grid")) else { return };
    grid.children.clear();
    for (i, class) in pockets.iter().enumerate() {
        let mut w = item.clone();
        w.name = format!("Item{i}");
        w.slot_kind = "HorizontalBoxSlot".into();
        w.slot = json!({});
        let active = i == selected;
        for (widget, prop, shown) in [("Image_On", "ImageOn", active), ("Image_Off", "ImageOff", !active)] {
            let icon = class_texture(hud, class, prop).or_else(|| class_texture(hud, class, "HUDPickupIcon"));
            if let Some(image) = w.find_mut(widget) {
                match icon.filter(|_| shown) {
                    Some(file) => {
                        let size = png_size(&hud.export.join(&file)).unwrap_or(Vec2::splat(64.0));
                        image.props["Brush"] = json!({ "ImageSize": { "X": size.x, "Y": size.y }, "ResourceObject": { "File": file } });
                        image.set_visibility(Vis::Visible);
                    }
                    None => image.set_visibility(Vis::Hidden),
                }
            }
        }
        if let Some(h) = w.find_mut("TextBlock_0") {
            h.set_visibility(Vis::Collapsed);
        }
        grid.children.push(w);
    }
}

/// XPDisplayWidget's box: its last MaxAmountOfSlots lines (empty ones first, as its Construct adds them), each with
/// its text and its category's icon.
pub(super) fn fill_xp_lines(hud: &Hud, tree: &mut Widget, scores: &[(String, String, f32)]) {
    let (Some(xp), Some(stack)) = (&hud.xp, tree.find_mut("MainXPDisplayBox")) else { return };
    stack.children.clear();
    let shown = &scores[scores.len().saturating_sub(xp.slots)..];
    for i in 0..xp.slots {
        let mut w = xp.line.clone();
        w.name = format!("XPLine{i}");
        // Its widgets named by line (XPLine<i>.Border_0) for the fades.
        w.class = Some(format!("XPLine{i}"));
        w.slot_kind = "VerticalBoxSlot".into();
        w.slot = json!({});
        let score = (i + shown.len()).checked_sub(xp.slots).and_then(|k| shown.get(k));
        if let Some(text) = w.find_mut("XPDescriptionAndAmouint") {
            text.props["Text"] = json!({ "LocalizedString": score.map_or("", |s| s.0.as_str()) });
        }
        let icon = score.and_then(|s| XP_ICONS.iter().find(|(c, _)| *c == s.1)).and_then(|(_, t)| xp.textures.get(*t));
        if let (Some(file), Some(image)) = (icon, w.find_mut("XPIcon")) {
            image.props["Brush"]["ResourceObject"] = json!({ "File": file });
        }
        stack.children.push(w);
    }
}
