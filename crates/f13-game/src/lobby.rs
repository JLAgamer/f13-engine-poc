//! The lobby (`--lobby` on a server): the players gather, say what they'd rather play and ready up; then the match
//! starts with one of them as Jason and the rest as counselors.
//!
//! Who plays Jason is SCGameMode_Hunt's pick as the match starts (its PreMatchIntro handler FUN_140382110,
//! decompiled), over the players (not spectators) in a random order:
//! - last match's Jason is out when anyone else is in (the game instance keeps his ids);
//! - shuffled again, those who'd rather be a counselor (ESCCharacterPreference Counselor, SCPlayerState_Hunt +0x868)
//!   leave, picked at random one at a time, while more than one is left;
//! - shuffled again, each left goes in 5 times if they'd rather be Jason (Killer), 3 times with no preference, once
//!   otherwise;
//! - shuffled four times more, and one entry is picked at random.
//!
//! Every shuffle swaps each place with a random one of all of them (FRand: `min(n * rand() / 32768, n - 1)`).
//!
//! Everyone else plays a counselor at the next counselor start.
//!
//! PLACEHOLDERS:
//! - The game's lobby is its own map, with a Steam session, its party beacons and its menus; here it's a list over the
//!   loaded level, which waits (nothing simulates while it's open).
//! - All ready starts a 5 s countdown (the game's travel countdown: the host starts it, or a full Quick Play lobby).
//! - The keys: 1 counselor, 2 no preference, 3 Jason, Enter ready.
//! - The counselors' character picks: every counselor is the one model.
//! - The server's own player is a member unless `--dedicated`.

use crate::controls::LocalPlayer;
use crate::net::{NetClient, PlayedBy};
use crate::player::Player;
use bevy::prelude::*;
use lightyear::prelude::*;
use serde::{Deserialize, Serialize};

/// ESCCharacterPreference.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Preference {
    #[default]
    NoPreference,
    Counselor,
    Killer,
}

impl Preference {
    fn label(self) -> &'static str {
        match self {
            Preference::NoPreference => "no preference",
            Preference::Counselor => "counselor",
            Preference::Killer => "Jason",
        }
    }

    /// How many times SCGameMode_Hunt puts a player with this preference in Jason's draw.
    fn entries(self) -> usize {
        match self {
            Preference::Killer => 5,
            Preference::NoPreference => 3,
            Preference::Counselor => 1,
        }
    }
}

/// What a player tells the lobby.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub enum LobbyMsg {
    Name(String),
    Preference(Preference),
    Ready(bool),
    /// Their counselor pick (a class: "Athlete", "Hero", ...).
    Class(String),
    /// Their outfit for it (their wardrobe's: SCPlayerState ServerReceivePerksAndOutfit; character.rs CounselorOutfit).
    Outfit(String),
    /// Their perks for it (perks.rs: perk classes).
    Perks(Vec<String>),
}

pub struct LobbyChannel;

/// A member as the players see them: their id (the netcode client id; 0 the server's own player), name, preference,
/// and whether they're ready.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
pub struct Member {
    pub id: u64,
    pub name: String,
    pub preference: Preference,
    pub ready: bool,
    /// Their round trip to the server (ms; the server's own player: 0), and whether they've talked and talk now
    /// (the row's ping and speaker: PlayerPingDisplayWidget, PlayerSpeakIconMuteButton).
    pub ping: u32,
    pub talked: bool,
    pub talking: bool,
    /// Their counselor pick (empty: the default) and its outfit (empty: its default).
    #[serde(default)]
    pub class: String,
    #[serde(default)]
    pub outfit: String,
    #[serde(default)]
    pub perks: Vec<String>,
}

/// The lobby as the players see it: open, its members, the countdown to the match (s), and who plays Jason once it's
/// started.
#[derive(Component, Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
pub struct NetLobby {
    pub open: bool,
    pub members: Vec<Member>,
    pub countdown: Option<f32>,
    pub jason: Option<u64>,
    /// The last match's results, back in the lobby after it (SCMatchEndLobbyWidget's: PLACEHOLDER, as text).
    pub last_match: Vec<MatchResult>,
}

/// A player's last match: their name, what they played and how it ended, their XP and badges.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
pub struct MatchResult {
    pub name: String,
    pub outcome: String,
    pub xp: i32,
    pub badges: Vec<String>,
}

/// Where a lobby's server leaves the match's results for the lobby it starts again (`--last-match`).
pub(crate) fn last_match_file() -> std::path::PathBuf {
    std::env::temp_dir().join("lakeside-last-match.json")
}

/// The results of the match that just ended, from its tallies (counselors by name) and Jason's score.
pub(crate) fn match_results(game: &crate::match_flow::GameMatch, names: &dyn Fn(Entity) -> String, jason: Option<String>) -> Vec<MatchResult> {
    let mut out: Vec<MatchResult> = game
        .counselors
        .iter()
        .map(|(e, t)| MatchResult {
            name: names(*e),
            outcome: if t.record.escaped { "escaped".into() } else if t.record.dead { "died".into() } else { "survived".into() },
            xp: t.score.total(),
            badges: t.badges.clone(),
        })
        .collect();
    if let Some(name) = jason {
        out.push(MatchResult { name, outcome: "Jason".into(), xp: game.killer_score.total(), badges: game.killer_badges.clone() });
    }
    out
}

/// On the server: the lobby is open (nothing simulates: [`crate::net::simulating`]).
#[derive(Resource)]
pub struct LobbyOpen;

/// On the server: each member's link (none: the server's own player).
#[derive(Resource, Default)]
struct Links(Vec<(u64, Option<Entity>)>);

/// On the server: the player who plays Jason from another machine (their link).
#[derive(Resource)]
pub struct JasonPlayer(pub Entity);

/// A remote Jason's input for a frame: his keys and buttons held (jason_player.rs's bits), his control rotation and
/// his camera's place and forward.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub struct JasonInputMsg {
    pub keys: u32,
    pub buttons: u8,
    pub yaw: f32,
    pub pitch: f32,
    pub view: [f32; 6],
    /// The last place picked on his Morph map (KLR_Morph_Activate) and how many picks so far (a new count is a new
    /// pick: the message is sent every frame, and some are lost).
    #[serde(default)]
    pub morph_to: [f32; 3],
    #[serde(default)]
    pub morph_picks: u32,
}

/// Last match's Jason (his id: the server's own player 0), out of the draw when anyone else is in.
#[derive(Resource, Default)]
pub struct LastJason(pub Option<u64>);

/// The countdown once everyone is ready (s; PLACEHOLDER).
const COUNTDOWN_S: f32 = 5.0;

/// On a client: its netcode id and its name.
#[derive(Resource, Clone)]
pub struct Me {
    pub id: u64,
    pub name: String,
}

pub(crate) fn protocol(app: &mut App) {
    app.add_channel::<LobbyChannel>(ChannelSettings { mode: ChannelMode::OrderedReliable(ReliableSettings::default()), ..default() }).add_direction(NetworkDirection::ClientToServer);
    app.register_message::<LobbyMsg>().add_direction(NetworkDirection::ClientToServer);
    app.register_message::<JasonInputMsg>().add_direction(NetworkDirection::ClientToServer);
    app.component::<NetLobby>().replicate();
}

/// The server's lobby: `dedicated`, the server's own player isn't in it.
pub(crate) fn server(app: &mut App, dedicated: bool, name: String, last_match: Option<std::path::PathBuf>, class: String) {
    let last_match = last_match.and_then(|p| std::fs::read_to_string(p).ok()).and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default();
    let mut lobby = NetLobby { open: true, last_match, ..default() };
    let mut links = Links::default();
    if !dedicated {
        lobby.members.push(Member { id: 0, name, class, ..default() });
        links.0.push((0, None));
    }
    app.insert_resource(LobbyOpen)
        .insert_resource(links)
        .add_systems(Startup, move |mut commands: Commands| {
            commands.spawn((lobby.clone(), Replicate::to_clients(NetworkTarget::All), Name::new("lobby")));
        })
        .add_observer(joined)
        .add_observer(left)
        .add_systems(Update, (receive, local_keys.in_set(Keys).run_if(move || !dedicated), count_down, member_stats).chain().run_if(resource_exists::<LobbyOpen>))
        .add_systems(Update, apply_jason_input.run_if(resource_exists::<JasonPlayer>));
}

/// Each member's ping (their link's round trip) and voice (talked, and within the last 0.3 s: PLACEHOLDER, the
/// game's IsPlayerTalking is its VoIP's own), written when they change (by 10 ms for the ping).
fn member_stats(
    pings: Query<(&RemoteId, &lightyear::prelude::PingManager), With<LinkOf>>,
    talkers: Option<Res<crate::voice::Talkers>>,
    real: Res<Time<Real>>,
    mut lobby: Query<&mut NetLobby>,
) {
    let Ok(mut lobby) = lobby.single_mut() else { return };
    let now = real.elapsed_secs();
    let stats: Vec<(u32, bool, bool)> = lobby
        .members
        .iter()
        .map(|m| {
            let ping = pings.iter().find(|(id, _)| peer_id(id) == m.id).map_or(0, |(_, p)| p.rtt().as_millis() as u32);
            let last = talkers.as_ref().and_then(|t| t.0.get(&m.id).copied());
            (ping, last.is_some(), last.is_some_and(|t| now - t < 0.3))
        })
        .collect();
    let changed = lobby.members.iter().zip(&stats).any(|(m, (ping, talked, talking))| m.ping.abs_diff(*ping) >= 10 || m.talked != *talked || m.talking != *talking);
    if changed {
        for (m, (ping, talked, talking)) in lobby.members.iter_mut().zip(stats) {
            (m.ping, m.talked, m.talking) = (ping, talked, talking);
        }
    }
}

/// The lobby's keys are read (a bot presses them before).
#[derive(SystemSet, Clone, PartialEq, Eq, Hash, Debug)]
pub struct Keys;

pub(crate) fn client(app: &mut App) {
    app.add_systems(Update, (send_name, client_keys.in_set(Keys), become_jason, send_jason_input.run_if(resource_exists::<crate::jason_player::PlayJason>)));
}

pub(crate) fn ui(app: &mut App) {
    app.add_plugins(crate::counselor_preview::CounselorPreviewPlugin);
    app.add_systems(PreUpdate, lobby_clicks.after(bevy::input::InputSystems));
    app.init_resource::<crate::voice::VoiceMutes>().add_systems(Startup, spawn_overlay).add_systems(Update, (voice_keys, show_overlay, load_counselor_list, (counselor_menu, crate::wardrobe::wardrobe_menu, crate::emotes::emote_menu, crate::perks::perk_menu).chain(), crate::audio_settings::audio_menu));
}

/// The lobby's voice mutes (PLACEHOLDER keys: the game's are in its scoreboard menu): Shift + a member's number mutes or
/// unmutes them. (Push-to-talk is in the audio settings: audio_settings.rs.)
fn voice_keys(
    keys: Res<ButtonInput<KeyCode>>,
    lobby: Query<&NetLobby>,
    mut mutes: ResMut<crate::voice::VoiceMutes>,
    me: Option<Res<Me>>,
) {
    let Some(l) = lobby.iter().next().filter(|l| l.open) else { return };
    if !keys.pressed(KeyCode::ShiftLeft) && !keys.pressed(KeyCode::ShiftRight) {
        return;
    }
    const DIGITS: [KeyCode; 8] = [KeyCode::Digit1, KeyCode::Digit2, KeyCode::Digit3, KeyCode::Digit4, KeyCode::Digit5, KeyCode::Digit6, KeyCode::Digit7, KeyCode::Digit8];
    for (i, k) in DIGITS.iter().enumerate() {
        let Some(m) = l.members.get(i).filter(|_| keys.just_pressed(*k)) else { continue };
        if me.as_ref().is_some_and(|me| me.id == m.id) {
            continue;
        }
        if !mutes.0.remove(&m.name) {
            mutes.0.insert(m.name.clone());
        }
        info!("voice: {} {}", m.name, if mutes.0.contains(&m.name) { "muted" } else { "unmuted" });
    }
}

pub(crate) fn peer_id(id: &RemoteId) -> u64 {
    match id.0 {
        PeerId::Netcode(n) => n,
        other => other.to_bits(),
    }
}

fn joined(joined: On<Add, Connected>, ids: Query<&RemoteId, With<LinkOf>>, mut links: ResMut<Links>, mut lobby: Query<&mut NetLobby>, open: Option<Res<LobbyOpen>>) {
    let (Some(_), Ok(id), Ok(mut lobby)) = (open, ids.get(joined.entity), lobby.single_mut()) else { return };
    let id = peer_id(id);
    links.0.push((id, Some(joined.entity)));
    lobby.members.push(Member { id, name: format!("player {}", id % 10000), ..default() });
    info!("lobby: {id} joined ({} in the lobby)", lobby.members.len());
}

fn left(left: On<Remove, Connected>, mut links: ResMut<Links>, mut lobby: Query<&mut NetLobby>) {
    let Some(i) = links.0.iter().position(|(_, l)| *l == Some(left.entity)) else { return };
    let (id, _) = links.0.remove(i);
    if let Ok(mut lobby) = lobby.single_mut() {
        lobby.members.retain(|m| m.id != id);
        if lobby.open {
            lobby.countdown = None;
        }
    }
    info!("lobby: {id} left");
}

/// The members' messages.
fn receive(mut receivers: Query<(Entity, &mut MessageReceiver<LobbyMsg>)>, links: Res<Links>, mut lobby: Query<&mut NetLobby>) {
    let Ok(mut lobby) = lobby.single_mut() else { return };
    for (link, mut r) in &mut receivers {
        let Some(&(id, _)) = links.0.iter().find(|(_, l)| *l == Some(link)) else { continue };
        for msg in r.receive() {
            let Some(m) = lobby.members.iter_mut().find(|m| m.id == id) else { continue };
            match msg {
                LobbyMsg::Name(n) => m.name = n.chars().take(24).collect(),
                LobbyMsg::Preference(p) => m.preference = p,
                LobbyMsg::Ready(r) => m.ready = r,
                LobbyMsg::Class(c) => m.class = c.chars().take(32).collect(),
                LobbyMsg::Outfit(o) => m.outfit = o.chars().take(96).collect(),
                LobbyMsg::Perks(p) => m.perks = p.into_iter().take(3).map(|p| p.chars().take(48).collect()).collect(),
            }
            info!("lobby: {} ({}) {} {} {}", m.name, m.id, m.preference.label(), m.class, if m.ready { "ready" } else { "not ready" });
        }
    }
}

/// The lobby's keys: 1 counselor, 2 no preference, 3 Jason, Enter ready or not.
fn lobby_key(keys: &ButtonInput<KeyCode>) -> Option<LobbyMsg> {
    // (Shift + a number mutes a player: voice_keys.)
    if keys.pressed(KeyCode::ShiftLeft) || keys.pressed(KeyCode::ShiftRight) {
        return None;
    }
    if keys.just_pressed(KeyCode::Digit1) {
        Some(LobbyMsg::Preference(Preference::Counselor))
    } else if keys.just_pressed(KeyCode::Digit2) {
        Some(LobbyMsg::Preference(Preference::NoPreference))
    } else if keys.just_pressed(KeyCode::Digit3) {
        Some(LobbyMsg::Preference(Preference::Killer))
    } else if keys.just_pressed(KeyCode::Enter) {
        Some(LobbyMsg::Ready(true))
    } else {
        None
    }
}

/// Counselor_Menu (her row's CounselorSelect: LobbyPlayerListItem pushes it), open: the counselor shown (an index into
/// its CharacterClassList) and NextCounselorTimer (no change while it's above 0).
#[derive(Resource)]
pub struct CounselorMenu {
    pub index: usize,
    timer: f32,
}

/// Counselor_Menu's CharacterClassList (its defaults: the classes in its order), as counselor classes ("Athlete").
#[derive(Resource, Default)]
pub struct CounselorList(pub Vec<String>);

fn load_counselor_list(mut commands: Commands, export: Res<crate::ExportRoot>, character: Option<Res<crate::character::CounselorCharacter>>, mut done: Local<bool>) {
    let Some(character) = character.filter(|_| !*done) else { return };
    *done = true;
    let v: serde_json::Value = std::fs::read_to_string(export.0.join("ui").join("Counselor_Menu.json")).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default();
    // "/Game/Characters/Counselors/Athlete/Athlete_Counselor.Athlete_Counselor_C" -> "Athlete" (the exported ones).
    let list: Vec<String> = v["Defaults"]["CharacterClassList"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|e| e.as_object()?.iter().find(|(k, _)| k.starts_with("CharacterClass"))?.1["AssetPathName"].as_str().map(str::to_string))
        .filter_map(|p| p.rsplit('.').next()?.strip_suffix("_Counselor_C").map(str::to_string))
        .filter(|c| character.1.contains_key(c))
        .collect();
    if list.is_empty() {
        warn!("lobby: no Counselor_Menu CharacterClassList (re-export: f13-export ui): no counselor menu");
    }
    commands.insert_resource(CounselorList(list));
}

/// The counselor menu (Counselor_Menu, decompiled): C opens it (her row's CounselorSelect), on her pick (SetCounselorIndex:
/// her PickedCounselorClass); Left / Right (or A / D: its arrow buttons, CSLR_ObjectivePrev / Next) the counselor before or
/// after, round (with CharacterSelectChange and NextCounselorTimer 0.5 s between); Enter (Accept, OnClick_Accept)
/// requests it (RequestCounselorClass) with UITransitionAll and closes it; Escape (Back) closes it.
#[allow(clippy::too_many_arguments)]
fn counselor_menu(
    mut commands: Commands,
    keys: Res<ButtonInput<KeyCode>>,
    time: Res<Time<bevy::time::Real>>,
    menu: Option<ResMut<CounselorMenu>>,
    list: Option<Res<CounselorList>>,
    mut lobby: Query<&mut NetLobby>,
    me: Option<Res<Me>>,
    mut senders: Query<&mut MessageSender<LobbyMsg>, With<Connected>>,
    (audio_menu, wardrobe, emote_menu, perk_menu): (Option<Res<crate::audio_settings::AudioMenu>>, Option<Res<crate::wardrobe::WardrobeMenu>>, Option<Res<crate::emotes::EmoteMenu>>, Option<Res<crate::perks::PerkMenu>>),
    mut tested: Local<bool>,
) {
    let (Some(list), Ok(mut lobby)) = (list.filter(|l| !l.0.is_empty()), lobby.single_mut()) else { return };
    // (The audio menu, or the wardrobe, emotes or perks over it, has the keys.)
    if audio_menu.is_some() || wardrobe.is_some() || emote_menu.is_some() || perk_menu.is_some() {
        return;
    }
    let my_id = me.as_ref().map_or(0, |m| m.id);
    let Some(menu) = menu else {
        // (F13_COUNSELOR_MENU[=<index>]: opened as the lobby opens, for screenshots.)
        let test = std::env::var("F13_COUNSELOR_MENU").is_ok() && !std::mem::replace(&mut *tested, true);
        if lobby.open && (keys.just_pressed(KeyCode::KeyC) || test) {
            let current = lobby.members.iter().find(|m| m.id == my_id).map_or("", |m| m.class.as_str());
            let current = if current.is_empty() { crate::character::DEFAULT_CLASS } else { current };
            let index = list.0.iter().position(|c| c == current).unwrap_or(0);
            let index = if test { std::env::var("F13_COUNSELOR_MENU").ok().and_then(|v| v.parse::<usize>().ok()).map_or(index, |i| i % list.0.len()) } else { index };
            commands.insert_resource(CounselorMenu { index, timer: 0.0 });
        }
        return;
    };
    let mut menu = menu;
    if !lobby.open || keys.just_pressed(KeyCode::Escape) {
        commands.remove_resource::<CounselorMenu>();
        return;
    }
    menu.timer -= time.delta_secs();
    let n = list.0.len();
    let step = if keys.just_pressed(KeyCode::ArrowLeft) || keys.just_pressed(KeyCode::KeyA) {
        Some(n - 1)
    } else if keys.just_pressed(KeyCode::ArrowRight) || keys.just_pressed(KeyCode::KeyD) {
        Some(1)
    } else {
        None
    };
    if let Some(step) = step.filter(|_| menu.timer <= 0.0) {
        menu.index = (menu.index + step) % n;
        menu.timer = 0.5;
        commands.trigger(crate::audio::PlaySound { cue: "CharacterSelectChange".into(), at: None });
    }
    if keys.just_pressed(KeyCode::Enter) {
        let class = list.0[menu.index].clone();
        info!("lobby: picked {class}");
        if me.is_some() {
            for mut s in &mut senders {
                s.send::<LobbyChannel>(LobbyMsg::Class(class.clone()));
            }
        } else if let Some(m) = lobby.members.iter_mut().find(|m| m.id == 0) {
            m.class = class;
        }
        commands.trigger(crate::audio::PlaySound { cue: "UITransitionAll".into(), at: None });
        commands.remove_resource::<CounselorMenu>();
    }
}

/// The counselor after `current` among the playable ones (by class name; the first after the last).
fn next_class(character: Option<&crate::character::CounselorCharacter>, current: &str) -> String {
    // (Not Tommy: the game mode's HunterCharacterClass, who comes only when called: SCGameMode::SpawnHunter.)
    let mut classes: Vec<&String> = character.map(|c| c.1.keys().filter(|k| k.as_str() != "Hunter").collect()).unwrap_or_default();
    classes.sort();
    let current = if current.is_empty() { crate::character::DEFAULT_CLASS } else { current };
    let i = classes.iter().position(|c| c.as_str() == current).map_or(0, |i| (i + 1) % classes.len().max(1));
    classes.get(i).map_or_else(|| crate::character::DEFAULT_CLASS.to_string(), |c| c.to_string())
}

/// The server's own player's keys.
fn local_keys(keys: Res<ButtonInput<KeyCode>>, mut lobby: Query<&mut NetLobby>, character: Option<Res<crate::character::CounselorCharacter>>, menu: Option<Res<CounselorMenu>>, audio_menu: Option<Res<crate::audio_settings::AudioMenu>>) {
    // (The counselor and audio menus have the keys while they're open.)
    if menu.is_some() || audio_menu.is_some() {
        return;
    }
    let (Some(msg), Ok(mut lobby)) = (lobby_key(&keys), lobby.single_mut()) else { return };
    let Some(m) = lobby.members.iter_mut().find(|m| m.id == 0) else { return };
    match msg {
        LobbyMsg::Preference(p) => m.preference = p,
        LobbyMsg::Ready(_) => m.ready = !m.ready,
        LobbyMsg::Name(_) => {}
        LobbyMsg::Class(_) => m.class = next_class(character.as_deref(), &m.class),
        LobbyMsg::Outfit(_) | LobbyMsg::Perks(_) => {}
    }
    info!("lobby: {} {} {} {}", m.name, m.preference.label(), m.class, if m.ready { "ready" } else { "not ready" });
}

/// Everyone ready: the countdown, then the match.
#[allow(clippy::too_many_arguments)]
fn count_down(
    mut commands: Commands,
    time: Res<Time>,
    mut lobby: Query<&mut NetLobby>,
    links: Res<Links>,
    mut rng: ResMut<crate::GameRng>,
    mut starts: ResMut<crate::net::FreeStarts>,
    (assets, character, tuning): (Res<AssetServer>, Option<Res<crate::character::CounselorCharacter>>, Res<crate::player::PlayerTuning>),
    local: Query<(Entity, &Transform), (With<LocalPlayer>, With<Player>)>,
    jason: Query<Entity, With<crate::jason::Jason>>,
    last: Res<LastJason>,
) {
    let Ok(mut lobby) = lobby.single_mut() else { return };
    let ready = !lobby.members.is_empty() && lobby.members.iter().all(|m| m.ready);
    lobby.countdown = match (ready, lobby.countdown) {
        (false, _) => None,
        (true, None) => {
            info!("lobby: everyone is ready: the match in {COUNTDOWN_S} s");
            Some(COUNTDOWN_S)
        }
        (true, Some(t)) => Some(t - time.delta_secs()),
    };
    if !lobby.countdown.is_some_and(|t| t <= 0.0) {
        return;
    }
    // The match: Jason, then the counselors.
    let members: Vec<(u64, Preference)> = lobby.members.iter().map(|m| (m.id, m.preference)).collect();
    let killer = pick_killer(&members, last.0, &mut || rng.0.frand());
    let name = |id: u64| lobby.members.iter().find(|m| m.id == id).map_or(String::new(), |m| m.name.clone());
    info!("lobby: the match starts: Jason is {}", killer.map_or("nobody".into(), name));
    let link = |id: u64| links.0.iter().find(|(i, _)| *i == id).and_then(|(_, l)| *l);
    let class = |id: u64| lobby.members.iter().find(|m| m.id == id).map(|m| m.class.clone()).filter(|c| !c.is_empty());
    let outfit = |id: u64| lobby.members.iter().find(|m| m.id == id).map(|m| m.outfit.clone()).filter(|o| !o.is_empty());
    let perks = |id: u64| lobby.members.iter().find(|m| m.id == id).map(|m| m.perks.clone()).filter(|p| !p.is_empty());
    // The server's own counselor: hers if she's a counselor, else gone (her start free again).
    let local_plays = links.0.iter().find(|(_, l)| l.is_none()).map(|(i, _)| *i);
    if local_plays.is_none() || local_plays == killer {
        if let Ok((e, xf)) = local.single() {
            starts.0.insert(0, xf.translation);
            commands.entity(e).despawn();
        }
    } else if let (Ok((e, _)), Some(id)) = (local.single(), local_plays) {
        commands.entity(e).insert(Name::new(name(id)));
        if let Some(c) = class(id) {
            commands.entity(e).insert(crate::character::CounselorClass(c));
        }
        if let Some(o) = outfit(id) {
            commands.entity(e).insert(crate::character::CounselorOutfit(o));
        }
        if let Some(p) = perks(id) {
            commands.entity(e).insert(crate::perks::CounselorPerks(p));
        }
    }
    for &(id, _) in &members {
        let Some(l) = link(id) else { continue };
        if Some(id) == killer {
            continue;
        }
        let occupied: Vec<Vec3> = local.iter().map(|(_, xf)| xf.translation).collect();
        let Some(at) = starts.take(&occupied, &mut || rng.0.frand()) else {
            warn!("lobby: no counselor start left for {id}");
            continue;
        };
        let c = crate::player::spawn_counselor(&mut commands, &assets, character.as_deref(), &tuning, at, &class(id).unwrap_or_else(|| crate::character::DEFAULT_CLASS.to_string()));
        commands.entity(c).insert((PlayedBy(l), ControlledBy { owner: l, lifetime: default() }, Name::new(name(id))));
        if let Some(o) = outfit(id) {
            commands.entity(c).insert(crate::character::CounselorOutfit(o));
        }
        if let Some(p) = perks(id) {
            commands.entity(c).insert(crate::perks::CounselorPerks(p));
        }
        info!("lobby: {} plays counselor {c} at {at}", name(id));
    }
    if let Some(k) = killer {
        commands.init_resource::<crate::jason_player::JasonControls>();
        match link(k) {
            None => commands.insert_resource(crate::jason_player::PlayJason),
            Some(l) => {
                commands.insert_resource(JasonPlayer(l));
                if let Ok(j) = jason.single() {
                    commands.entity(j).insert(ControlledBy { owner: l, lifetime: default() });
                }
            }
        }
    }
    lobby.open = false;
    lobby.countdown = None;
    lobby.jason = killer;
    commands.remove_resource::<LobbyOpen>();
}

/// SCGameMode_Hunt's draw for Jason (see the module's notes): the players' ids and preferences, last match's
/// Jason, and FRand.
pub fn pick_killer(players: &[(u64, Preference)], last: Option<u64>, frand: &mut dyn FnMut() -> f32) -> Option<u64> {
    let mut pick = |n: usize, frand: &mut dyn FnMut() -> f32| ((n as f32 * frand()) as usize).min(n.saturating_sub(1));
    let mut shuffle = |v: &mut Vec<(u64, Preference)>, frand: &mut dyn FnMut() -> f32| {
        let n = v.len();
        for i in 0..n {
            let j = pick(n, frand);
            v.swap(i, j);
        }
    };
    let mut c: Vec<(u64, Preference)> = players.to_vec();
    shuffle(&mut c, frand);
    if c.len() > 1 {
        if let Some(i) = c.iter().position(|p| Some(p.0) == last) {
            c.remove(i);
        }
    }
    shuffle(&mut c, frand);
    let mut counselors: Vec<u64> = c.iter().filter(|p| p.1 == Preference::Counselor).map(|p| p.0).collect();
    while c.len() > 1 && !counselors.is_empty() {
        let i = pick(counselors.len(), frand);
        let id = counselors.remove(i);
        c.retain(|p| p.0 != id);
    }
    shuffle(&mut c, frand);
    let mut draw = Vec::new();
    for p in c.iter().rev() {
        for _ in 0..p.1.entries() {
            draw.push(*p);
        }
    }
    for _ in 0..4 {
        shuffle(&mut draw, frand);
    }
    if draw.is_empty() {
        return None;
    }
    Some(draw[pick(draw.len(), frand)].0)
}

/// A remote Jason's input into his controls (every message: a tap between two frames still counts).
fn apply_jason_input(
    player: Res<JasonPlayer>,
    mut receivers: Query<&mut MessageReceiver<JasonInputMsg>>,
    controls: Option<ResMut<crate::jason_player::JasonControls>>,
    mut map: ResMut<crate::jason_player::MorphMap>,
    mut picks: Local<u32>,
) {
    let (Ok(mut r), Some(mut c)) = (receivers.get_mut(player.0), controls) else { return };
    for m in r.receive() {
        // A new pick on his Morph map: the place (jason.rs AttemptMorph reads it).
        if m.morph_picks != *picks {
            *picks = m.morph_picks;
            map.target = Some(Vec3::from_array(m.morph_to));
        }
        let v = m.view;
        c.apply_remote(m.keys, m.buttons, m.yaw, m.pitch, (Vec3::new(v[0], v[1], v[2]), Vec3::new(v[3], v[4], v[5])));
    }
}

/// The client's name and counselor pick (--counselor), once connected.
fn send_name(me: Option<Res<Me>>, local: Option<Res<crate::character::LocalClass>>, mut senders: Query<&mut MessageSender<LobbyMsg>, Added<Connected>>) {
    let Some(me) = me else { return };
    for mut s in &mut senders {
        s.send::<LobbyChannel>(LobbyMsg::Name(me.name.clone()));
        if let Some(c) = &local {
            s.send::<LobbyChannel>(LobbyMsg::Class(c.0.clone()));
        }
    }
}

/// The lobby's keys on a client.
fn client_keys(keys: Res<ButtonInput<KeyCode>>, lobby: Query<&NetLobby>, me: Option<Res<Me>>, character: Option<Res<crate::character::CounselorCharacter>>, mut senders: Query<&mut MessageSender<LobbyMsg>, With<Connected>>, menu: Option<Res<CounselorMenu>>, audio_menu: Option<Res<crate::audio_settings::AudioMenu>>) {
    if menu.is_some() || audio_menu.is_some() {
        return;
    }
    let (Ok(lobby), Some(me)) = (lobby.single(), me) else { return };
    if !lobby.open {
        return;
    }
    let Some(mut msg) = lobby_key(&keys) else { return };
    if let LobbyMsg::Ready(_) = msg {
        let ready = lobby.members.iter().find(|m| m.id == me.id).is_some_and(|m| m.ready);
        msg = LobbyMsg::Ready(!ready);
    }
    if let LobbyMsg::Class(_) = msg {
        let current = lobby.members.iter().find(|m| m.id == me.id).map_or("", |m| m.class.as_str());
        msg = LobbyMsg::Class(next_class(character.as_deref(), current));
    }
    for mut s in &mut senders {
        s.send::<LobbyChannel>(msg.clone());
    }
}

/// This client plays Jason: his controls and cameras here, and no counselor of its own.
fn become_jason(mut commands: Commands, jason: Query<(), (With<crate::net::NetJason>, Added<Controlled>)>, local: Query<Entity, (With<LocalPlayer>, With<Player>)>) {
    if jason.is_empty() {
        return;
    }
    info!("net: playing Jason");
    commands.insert_resource(crate::jason_player::PlayJason);
    commands.init_resource::<crate::jason_player::JasonControls>();
    for e in &local {
        commands.entity(e).despawn();
    }
}

/// This client's Jason input, every frame.
fn send_jason_input(
    controls: Option<Res<crate::jason_player::JasonControls>>,
    mut senders: Query<&mut MessageSender<JasonInputMsg>, With<Connected>>,
    mut map: ResMut<crate::jason_player::MorphMap>,
    mut pick: Local<(u32, [f32; 3])>,
) {
    let Some(c) = controls else { return };
    // The place his Morph map's click picked here (ui.rs): the server's to morph to.
    if let Some(at) = map.target.take() {
        *pick = (pick.0 + 1, at.to_array());
    }
    let (keys, buttons) = c.held();
    let (p, f) = c.view;
    for mut s in &mut senders {
        s.send::<crate::net::InputChannel>(JasonInputMsg { keys, buttons, yaw: c.yaw, pitch: c.pitch, view: [p.x, p.y, p.z, f.x, f.y, f.z], morph_to: pick.1, morph_picks: pick.0 });
    }
}

#[derive(Component)]
struct Overlay;

fn spawn_overlay(mut commands: Commands) {
    commands.spawn((
        Text::new(""),
        TextFont { font_size: FontSize::Px(22.0), ..default() },
        TextLayout::justify(Justify::Center),
        Node { position_type: PositionType::Absolute, bottom: Val::Percent(4.0), left: Val::Percent(30.0), width: Val::Percent(40.0), ..default() },
        BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.7)),
        GlobalZIndex(20),
        Visibility::Hidden,
        Overlay,
    ));
}

/// The lobby's list while it's open.
#[allow(clippy::type_complexity)]
fn show_overlay(
    lobby: Query<&NetLobby>,
    me: Option<Res<Me>>,
    client: Option<Res<NetClient>>,
    mut overlay: Single<(&mut Text, &mut Visibility), With<Overlay>>,
    (open_mic, mutes): (Option<Res<crate::voice::OpenMic>>, Option<Res<crate::voice::VoiceMutes>>),
    (menu, audio_menu): (Option<Res<CounselorMenu>>, Option<Res<crate::audio_settings::AudioMenu>>),
) {
    let (ref mut text, ref mut vis) = *overlay;
    let Some(l) = lobby.iter().next().filter(|l| l.open && menu.is_none() && audio_menu.is_none()) else {
        vis.set_if_neq(Visibility::Hidden);
        return;
    };
    let mine = if client.is_some() { me.map(|m| m.id) } else { Some(0) };
    // (The players are on the lobby's own screen: ui.rs LobbyWidget; its keys and the last match here.)
    let mut s = String::new();
    let _ = mine;
    s.push_str("1 counselor   2 no preference   3 Jason   C pick a counselor   O audio settings   Enter ready\n");
    let open = open_mic.is_some();
    s.push_str(&format!("push-to-talk (T): {}   Shift+n mute player n\n", if open { "off" } else { "on" }));
    let muted: Vec<&str> = mutes.as_ref().map_or_else(Vec::new, |m| m.0.iter().map(String::as_str).collect());
    if !muted.is_empty() {
        s.push_str(&format!("muted: {}\n", muted.join(", ")));
    }
    if let Some(t) = l.countdown {
        s.push_str(&format!("\nthe match in {:.0} s", t.ceil().max(0.0)));
    }
    if !l.last_match.is_empty() {
        s.push_str("\nLAST MATCH\n");
        for r in &l.last_match {
            let badges = if r.badges.is_empty() { String::new() } else { format!("  ({})", r.badges.join(", ")) };
            s.push_str(&format!("{}  -  {}  -  {} XP{}\n", r.name, r.outcome, r.xp, badges));
        }
    }
    if text.0 != s {
        text.0 = s;
    }
    vis.set_if_neq(Visibility::Inherited);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn draw(players: &[(u64, Preference)], last: Option<u64>, seed: u64) -> Option<u64> {
        let mut rng = f13_core::spawn::Rng::new(seed);
        pick_killer(players, last, &mut || rng.frand())
    }

    #[test]
    fn a_lone_player_is_jason() {
        assert_eq!(draw(&[(7, Preference::Counselor)], None, 1), Some(7));
        assert_eq!(draw(&[], None, 1), None);
    }

    #[test]
    fn counselors_by_choice_are_jason_only_when_everyone_is() {
        let players = [(1, Preference::Counselor), (2, Preference::NoPreference), (3, Preference::Counselor)];
        for seed in 0..200 {
            assert_eq!(draw(&players, None, seed), Some(2));
        }
        // All of them counselors by choice: one is left in, and he's Jason.
        let all = [(1, Preference::Counselor), (2, Preference::Counselor)];
        for seed in 0..50 {
            assert!(draw(&all, None, seed).is_some());
        }
    }

    #[test]
    fn last_matchs_jason_sits_out() {
        let players = [(1, Preference::Killer), (2, Preference::NoPreference)];
        for seed in 0..200 {
            assert_eq!(draw(&players, Some(1), seed), Some(2));
        }
    }

    #[test]
    fn wanting_jason_weighs_five_to_three() {
        let players = [(1, Preference::Killer), (2, Preference::NoPreference)];
        let n = 4000;
        let ones = (0..n).filter(|&s| draw(&players, None, s) == Some(1)).count() as f32 / n as f32;
        assert!((ones - 5.0 / 8.0).abs() < 0.05, "{ones}");
    }
}

/// The lobby's buttons by the mouse: a click on one presses the key its menu already takes (LobbyWidget's ReadyButton:
/// Enter; a row's CounselorButton: the counselor menu, C; its JasonButton: Jason preferred, 3 (PLACEHOLDER: the game's
/// opens its killer picker); Counselor_Menu's arrows, its three PerkButton slots (1-3), ClothingButton (X),
/// CounselorEmoteButton (E)); the right button backs out of an open menu (Escape, PLACEHOLDER for the menus' action
/// bar Back). The keys go down here, right after the input's own, and up the next frame. Hovering and pressing play
/// their style's sounds; their looks (each Blueprint's own OnHovered) aren't ported.
fn lobby_clicks(
    mut commands: Commands,
    lobby: Query<&NetLobby>,
    buttons: Query<(Entity, &Name), (Without<Interaction>, With<Node>)>,
    clicked: Query<(&Interaction, &Name), Changed<Interaction>>,
    mouse: Res<ButtonInput<MouseButton>>,
    mut keys: ResMut<ButtonInput<KeyCode>>,
    mut down: Local<Vec<KeyCode>>,
    (mut perk_menu, mut emote_menu, mut wardrobe, character, profiles): (
        Option<ResMut<crate::perks::PerkMenu>>,
        Option<ResMut<crate::emotes::EmoteMenu>>,
        Option<ResMut<crate::wardrobe::WardrobeMenu>>,
        Option<Res<crate::character::CounselorCharacter>>,
        Option<Res<crate::wardrobe::CounselorProfiles>>,
    ),
) {
    // The menus' list entries (ui.rs names them by index): a perk or emote in the panel picked and put in (Enter, as
    // its button's OnClicked SavePerk / AddEmote), an outfit shown, a slot focused, a swatch chosen.
    let entry = |n: &str| ["Perk_", "Emote_", "Outfit_", "Swatch_", "Slot_Button_0"].iter().find_map(|p| Some((*p, n.strip_prefix(p)?.parse::<usize>().ok()?)));
    const KEYS: [(&str, KeyCode); 10] = [
        ("ReadyButton", KeyCode::Enter),
        ("CounselorButton", KeyCode::KeyC),
        ("JasonButton", KeyCode::Digit3),
        ("LeftArrowButton", KeyCode::ArrowLeft),
        ("RightArrowButton", KeyCode::ArrowRight),
        ("PerkButton_Slot_1", KeyCode::Digit1),
        ("PerkButton_Slot_2", KeyCode::Digit2),
        ("PerkButton_Slot_3", KeyCode::Digit3),
        ("ClothingButton", KeyCode::KeyX),
        ("CounselorEmoteButton", KeyCode::KeyE),
    ];
    for k in down.drain(..) {
        keys.release(k);
    }
    if !lobby.iter().any(|l| l.open) {
        return;
    }
    for (e, n) in &buttons {
        if KEYS.iter().any(|(k, _)| *k == n.as_str()) || entry(n.as_str()).is_some() {
            commands.entity(e).try_insert(Interaction::None);
            debug!("lobby: {} takes clicks", n.as_str());
        }
    }
    for (i, n) in &clicked {
        let listed = entry(n.as_str());
        if !KEYS.iter().any(|(k, _)| *k == n.as_str()) && listed.is_none() {
            continue;
        }
        // Their WidgetStyle's HoveredSlateSound (MenuButtonFocusCue) and PressedSlateSound (ui_Click01_stereo).
        match i {
            Interaction::Hovered => commands.trigger(crate::audio::PlaySound { cue: "MenuButtonFocusCue".into(), at: None }),
            Interaction::Pressed => commands.trigger(crate::audio::PlaySound { cue: "ui_Click01_stereo".into(), at: None }),
            Interaction::None => {}
        }
        if let (Some((kind, k)), Interaction::Pressed) = (listed, i) {
            match kind {
                "Perk_" => {
                    if let Some(m) = perk_menu.as_mut() {
                        m.index = k;
                        keys.press(KeyCode::Enter);
                        down.push(KeyCode::Enter);
                    }
                }
                "Emote_" => {
                    if let Some(m) = emote_menu.as_mut() {
                        m.list = Some(k);
                        keys.press(KeyCode::Enter);
                        down.push(KeyCode::Enter);
                    }
                }
                "Outfit_" => {
                    if let (Some(w), Some(c), Some(p)) = (wardrobe.as_mut(), character.as_ref(), profiles.as_ref()) {
                        w.row = 0;
                        w.set_outfit(k, c, p);
                    }
                }
                "Slot_Button_0" => {
                    if let Some(w) = wardrobe.as_mut() {
                        w.row = k.min(w.picks.len());
                    }
                }
                _ => {
                    if let Some(w) = wardrobe.as_mut().filter(|w| w.row > 0) {
                        let slot = w.row - 1;
                        if let Some(p) = w.picks.get_mut(slot) {
                            *p = k;
                        }
                    }
                }
            }
            continue;
        }
        let Some(&(_, key)) = KEYS.iter().find(|(k, _)| *k == n.as_str()).filter(|_| *i == Interaction::Pressed) else { continue };
        info!("lobby: {} clicked", n.as_str());
        keys.press(key);
        down.push(key);
    }
    if mouse.just_pressed(MouseButton::Right) {
        keys.press(KeyCode::Escape);
        down.push(KeyCode::Escape);
    }
}
