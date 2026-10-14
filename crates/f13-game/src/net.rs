//! The netcode (Milestone 3; lightyear 0.30 over UDP with netcode.io): `--server <port>` runs the match and takes
//! players; `--connect <host:port>` joins one.
//!
//! The server is the authority: each joining player gets a counselor at the next counselor start, her [`Controls`]
//! from the player's input messages (the buttons held and the control rotation, every tick), and every character's
//! body ([`NetBody`]: where it is and which way it faces) goes out to all players. A client sends its input and shows
//! the characters it receives; its own counselor follows the server's.
//!
//! A client doesn't simulate the match ([`simulating`] gates the gameplay systems off): every character it receives
//! has a local twin (its own counselor, the local Jason, a counselor spawned for each other player), and the
//! twin takes the server's state each frame. That state is the body, what the animation reads ([`NetCounselor`],
//! [`NetJason`]) and the full-body action ([`NetAction`]). Then the twin's own animation, sounds and camera run as
//! they do on the server.
//!
//! PLACEHOLDERS:
//! - the development key and protocol id (no matchmaking backend issues tokens yet);
//! - the world's doors, windows, items and cars don't replicate yet.

use crate::character::{PlayAction, RestartAction, SeekAction};
use crate::controls::{Controls, LocalPlayer};
use crate::match_flow::{GameMatch, MatchState};
use crate::player::Player;
use f13_core::counselor::Stance;
use bevy::prelude::*;
use core::time::Duration;
use lightyear::prelude::*;
use serde::{Deserialize, Serialize};
use std::net::{Ipv4Addr, SocketAddr};

/// The protocol's id (netcode.io: clients and servers of other ids don't connect).
const PROTOCOL_ID: u64 = 0x0F13_0003;
/// PLACEHOLDER: the development key (a backend would sign connect tokens with its own).
const DEV_KEY: [u8; 32] = [0x13; 32];
/// How long either side waits without hearing from the other before dropping the connection (netcode's default is
/// 3 s, which a level still streaming in on either side outlasts; several games on one machine stall longer).
const TIMEOUT_SECS: i32 = 90;
/// The netcode tick (the game's fixed step).
const TICK: Duration = Duration::from_micros(15_625);

/// What this process is on the network.
#[derive(Clone, Copy, Debug)]
pub enum Role {
    Server(u16),
    Client(SocketAddr),
}

/// The match's world as the server built it: its seed, cabin layout and weather. A client asks for it before it
/// builds its own (the same, so the actors' ids match: net_world.rs); PLACEHOLDER for the lobby's travel.
#[derive(Clone, Debug, PartialEq)]
pub struct WorldInfo {
    pub seed: u64,
    pub layout: String,
    pub rain: bool,
}

const WORLD_ASK: &[u8] = b"F13 world?";
const WORLD_TELL: &str = "F13 world ";

/// Answers clients' world questions on UDP `port + 1`.
pub fn serve_world(port: u16, info: WorldInfo) {
    let answer = format!("{WORLD_TELL}{} {} {}", info.seed, info.layout, info.rain);
    std::thread::spawn(move || {
        // (A rematch's relaunch starts while the last match's server is still letting go of the port: a few tries.)
        let mut socket = None;
        for _ in 0..50 {
            match std::net::UdpSocket::bind((Ipv4Addr::UNSPECIFIED, port + 1)) {
                Ok(s) => {
                    socket = Some(s);
                    break;
                }
                Err(_) => std::thread::sleep(Duration::from_millis(200)),
            }
        }
        let Some(socket) = socket else {
            eprintln!("net: can't answer world questions on UDP {}", port + 1);
            return;
        };
        let mut buf = [0u8; 64];
        while let Ok((n, from)) = socket.recv_from(&mut buf) {
            if &buf[..n] == WORLD_ASK {
                let _ = socket.send_to(answer.as_bytes(), from);
            }
        }
    });
}

/// Asks the server at `server` (its world question port, one up) for its world, for up to a minute.
pub fn query_world(server: SocketAddr) -> Option<WorldInfo> {
    let socket = std::net::UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0)).ok()?;
    socket.set_read_timeout(Some(Duration::from_secs(1))).ok()?;
    let to = SocketAddr::new(server.ip(), server.port() + 1);
    let mut buf = [0u8; 256];
    for _ in 0..60 {
        let _ = socket.send_to(WORLD_ASK, to);
        if let Ok((n, _)) = socket.recv_from(&mut buf) {
            let text = std::str::from_utf8(&buf[..n]).ok()?;
            let mut parts = text.strip_prefix(WORLD_TELL)?.split(' ');
            let (seed, layout, rain) = (parts.next()?.parse().ok()?, parts.next()?.to_string(), parts.next()? == "true");
            return Some(WorldInfo { seed, layout, rain });
        }
    }
    eprintln!("net: no answer from {to} about the world: building our own");
    None
}

/// The client's role marker: present on a client (its own simulation of others is replaced by what it receives).
#[derive(Resource)]
pub struct NetClient;

/// A player's input for one frame: the buttons held ([`Controls::held`]), the control rotation, and how many times
/// each button was pressed and released so far (wrapping counts, by [`Controls::held`]'s bit): the server keeps only
/// the newest input, so a tap between two of its frames (or in a lost message) shows as a count gone up.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub struct InputMsg {
    pub held: u32,
    pub yaw: f32,
    pub pitch: f32,
    pub presses: [u8; 32],
    pub releases: [u8; 32],
}

/// The buttons whose counts changed from `old` to `new` (bits as [`Controls::held`]).
fn counted(old: &[u8; 32], new: &[u8; 32]) -> u32 {
    (0..32).filter(|&i| old[i] != new[i]).fold(0, |bits, i| bits | 1 << i)
}

/// On a client: its buttons' press and release counts so far (sent in every [`InputMsg`]).
#[derive(Default)]
struct Counts {
    presses: [u8; 32],
    releases: [u8; 32],
}

impl Counts {
    fn add(&mut self, pressed: u32, released: u32) {
        for i in 0..32 {
            if pressed & 1 << i != 0 {
                self.presses[i] = self.presses[i].wrapping_add(1);
            }
            if released & 1 << i != 0 {
                self.releases[i] = self.releases[i].wrapping_add(1);
            }
        }
    }
}

/// On the server, a player link's counts as last received (none before the first input).
#[derive(Component)]
struct LinkCounts {
    presses: [u8; 32],
    releases: [u8; 32],
}

/// The input channel: only the newest input counts.
pub struct InputChannel;

/// A character's body as the server sends it: position (Bevy metres), facing (yaw), and what it is.
#[derive(Component, Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub struct NetBody {
    /// The server's name for the character (its entity's bits).
    pub id: u64,
    pub pos: [f32; 3],
    pub yaw: f32,
    pub kind: BodyKind,
    /// When the server took it (its fixed-step clock, s): a client spaces the bodies by this, not by when they arrive.
    pub t: f32,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub enum BodyKind {
    Counselor,
    Jason,
}

/// What a counselor's animation, HUD and sounds read of her state (the server's [`Player`]).
#[derive(Component, Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
pub struct NetCounselor {
    pub stance: u8,
    pub wounded: bool,
    pub health: f32,
    pub stamina: f32,
    pub sprinting: bool,
    pub running: bool,
    /// cm/s, Bevy's XZ.
    pub velocity: [f32; 2],
    pub fear: f32,
    pub montage: Option<String>,
    pub root_motion: bool,
    pub hidden: bool,
    pub grounded: bool,
    /// Her player's name, and where she looks (her control rotation: yaw, pitch).
    pub name: String,
    pub look: [f32; 2],
    /// In Jason's hands, and the grab's struggle (its completion).
    pub held: Option<f32>,
    /// In a bear trap, and its struggle's completion while she's still in it.
    pub trapped: Option<Option<f32>>,
    /// Dead or escaped (voice.rs: heard only by the others spectating).
    pub spectating: bool,
    /// In a hiding spot: its world id, the side and the phase (hiding.rs HidePhase).
    pub hiding: Option<(u32, u8, u8)>,
    /// Her breath (hiding.rs Breathing): how much is left and whether she holds it.
    pub breath: (f32, bool),
    /// Her hold interaction (a repair's dial and its pips: her client's dial shows it, her presses go to the server).
    pub hold: Option<NetHold>,
    /// In a car: its world id, her seat (none: the driver's) and the ignition's hold.
    pub driving: Option<(u32, Option<u8>, Option<NetHold>)>,
    /// Escaped, and how.
    pub escaped: Option<String>,
    /// In a move at a door or window, or on her way onto a special move's spot ([`RemoteMove`] on her twin).
    pub moving: bool,
    /// Her gore records and detached parts (blood.rs Gore).
    pub gore: crate::blood::Gore,
    /// Her class ("Athlete", "Hero", ...; character.rs CounselorClass).
    pub class: String,
    /// She's coming as Tommy: his intro plays for her player (tommy.rs HunterIntro).
    #[serde(default)]
    pub hunter_intro: bool,
    /// Her outfit (her player's wardrobe: character.rs CounselorOutfit; empty, her default).
    #[serde(default)]
    pub outfit: String,
}

/// A client's twin of a counselor in a move the server plays (at a door or window, or onto a special move's spot).
#[derive(Component)]
pub struct RemoteMove;

/// A hold interaction (f13-core minigame HoldInteraction): its times, and its pips (width, position, state, key).
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct NetHold {
    pub hold_time: f32,
    pub timer: f32,
    pub started: bool,
    pub bonus: f32,
    pub has_pips: bool,
    pub pips: Vec<(f32, f32, u8, u8)>,
    pub current: Option<u8>,
}

impl NetHold {
    fn of(h: &f13_core::minigame::HoldInteraction) -> Self {
        Self {
            hold_time: h.hold_time,
            timer: h.timer,
            started: h.started,
            bonus: h.bonus,
            has_pips: h.has_pips,
            pips: h.pips.iter().map(|p| (p.width, p.position, p.state.index(), p.key)).collect(),
            current: h.current.map(|c| c as u8),
        }
    }

    fn hold(&self) -> f13_core::minigame::HoldInteraction {
        use f13_core::minigame::{HoldInteraction, Pip, PipState};
        let pips = self.pips.iter().map(|&(width, position, state, key)| Pip { width, position, state: PipState::from_index(state), key }).collect();
        HoldInteraction::remote(self.hold_time, self.timer, self.started, self.bonus, self.has_pips, pips, self.current.map(|c| c as usize))
    }
}

/// What Jason's animation reads of his state.
#[derive(Component, Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
pub struct NetJason {
    pub velocity: [f32; 2],
    pub combat: bool,
    pub blocking: bool,
    pub done: bool,
    pub shifting: bool,
    pub stalking: bool,
    pub holding: bool,
    pub knife_aim: bool,
    pub hidden: bool,
    pub underwater: bool,
    /// The door he's opening or breaking down (its world id) and how.
    pub door: Option<(u32, crate::jason::DoorMove)>,
    pub knives: u32,
    pub traps: u32,
    /// His state's name (a Jason player's HUD), and his attack's recovery left (s; a client's bot steps in then).
    pub state: String,
    pub cooldown: f32,
    /// His abilities (f13-core KillerAbilities): unlocked, recharge timers, the unlock timer, rage.
    pub unlocked: [bool; 4],
    pub recharge: [f32; 4],
    pub unlock_timer: f32,
    pub rage: (bool, f32),
    /// The kill camera running (kill_camera.rs): its kill, origin (translation, rotation) and whether at the root.
    pub kill_cam: Option<(String, [f32; 7], bool)>,
    /// Its victim's body (NetBody id) and whether everyone watches it (kill_camera.rs: whose it is).
    #[serde(default)]
    pub kill_cam_of: (Option<u64>, bool),
    /// The counselors (NetBody ids) this Sense leaves out (senses.rs roll_sense_avoiders).
    #[serde(default)]
    pub sense_avoid: Vec<u64>,
    /// His mask on (bMaskOn) and his health.
    #[serde(default)]
    pub mask_on: bool,
    #[serde(default)]
    pub health: f32,
    /// His map out for a Morph (the player's MorphMap: bIsMorphActive), for the client that plays him.
    #[serde(default)]
    pub morph_map: bool,
    /// Sense: how long it's still on (s; 0 off) and its length (every machine works out its highlights: senses.rs).
    #[serde(default)]
    pub sense: (f32, f32),
    /// Pamela's sweater stunned him (his kill's condition: the bots read it).
    #[serde(default)]
    pub sweater_stunned: bool,
    /// His stun's minigame (its completion, in hundredths, and StunText), for his player's wiggle widget.
    #[serde(default)]
    pub minigame: Option<(u8, String)>,
}

/// A character's full-body action ([`PlayAction`]), how often it restarted ([`RestartAction`]), its last seek
/// ([`SeekAction`]: a count and where to), and whether she blocks.
#[derive(Component, Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
pub struct NetAction {
    pub take: Option<String>,
    pub restarts: u32,
    pub seek: (u32, f32),
    pub blocking: bool,
}

/// On a client: the local entity that shows this replicated character, and the restarts and seeks it has taken.
#[derive(Component)]
struct Twin {
    local: Entity,
    restarts: u32,
    seeks: u32,
    /// The kill camera started for it (one ending here before the server's doesn't start again).
    kill: Option<String>,
}

/// On a client: each character's twin, by the server's name for it ([`NetBody::id`]).
#[derive(Resource, Default)]
pub struct Twins(std::collections::HashMap<u64, Entity>);

impl Twins {
    pub fn of(&self, id: u64) -> Option<Entity> {
        self.0.get(&id).copied()
    }
}

/// On a client: the twins have taken the server's state.
#[derive(SystemSet, Clone, PartialEq, Eq, Hash, Debug)]
pub struct Mirrored;

/// Whether this process runs the match (not a client). The gameplay systems run only if so.
pub fn simulating(client: Option<Res<NetClient>>, lobby: Option<Res<crate::lobby::LobbyOpen>>) -> bool {
    client.is_none() && lobby.is_none()
}

/// On the server: the link of the player who plays this counselor.
#[derive(Component)]
pub(crate) struct PlayedBy(pub Entity);

/// The counselor starts not yet taken.
#[derive(Resource, Default)]
pub struct FreeStarts(pub Vec<Vec3>);

/// A character this close to a start occupies it (m; the counselor's capsule, side by side).
const START_OCCUPIED_M: f32 = 2.0 * crate::player::CAPSULE_RADIUS_M;

impl FreeStarts {
    /// SCGameMode's ChoosePlayerStart (0x140378140, AGameModeBase's): a random counselor start no one stands on
    /// (FindTeleportSpot fits her there), else a random one of the rest; it's taken.
    pub fn take(&mut self, occupied: &[Vec3], frand: &mut dyn FnMut() -> f32) -> Option<Vec3> {
        let pick = |n: usize, frand: &mut dyn FnMut() -> f32| ((n as f32 * frand()) as usize).min(n.saturating_sub(1));
        let free: Vec<usize> = (0..self.0.len()).filter(|&i| occupied.iter().all(|o| o.with_y(0.0).distance(self.0[i].with_y(0.0)) > START_OCCUPIED_M)).collect();
        let i = if free.is_empty() {
            if self.0.is_empty() {
                return None;
            }
            pick(self.0.len(), frand)
        } else {
            free[pick(free.len(), frand)]
        };
        Some(self.0.remove(i))
    }
}

pub struct NetPlugin {
    pub role: Role,
    /// A client: the network test bot ([`net_bot`]), and what it asks the lobby for. A lobby's server that plays: only
    /// the lobby's part (it asks and readies up), so a lobby test takes two games, not three.
    pub bot: bool,
    /// The client bot as Jason also uses his abilities (`--bot-net-abilities`; the server needs `--bot-hooks`).
    pub bot_abilities: bool,
    pub prefer: crate::lobby::Preference,
    /// A server: the lobby first (lobby.rs), and whether the server plays no one.
    pub lobby: bool,
    pub dedicated: bool,
    /// This player's name in the lobby.
    pub name: String,
    /// This player's counselor pick (--counselor).
    pub class: String,
    /// A client's netcode id (else one from the clock), and a lobby's last match's Jason (`--net-id`, `--last-jason`:
    /// a rematch's relaunch keeps them).
    pub net_id: Option<u64>,
    pub last_jason: Option<u64>,
    /// A server: its players' scenario bots may set their scenes up (`--bot-hooks`: teleports, locks; testing).
    pub bot_hooks: bool,
    /// A lobby's server: the last match's results (`--last-match`: a rematch's relaunch passes them on).
    pub last_match: Option<std::path::PathBuf>,
}

/// A client's scenario bot's scene change (bot.rs [`crate::bot::BotSetup`]), for the server to make.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct BotSetupMsg(pub crate::bot::BotSetup);

/// On a client: its scenario bot's scene changes to the server.
fn send_bot_setup(mut senders: Query<&mut MessageSender<BotSetupMsg>, With<Connected>>) {
    // (Kept until connected: a bot can set its scene up first.)
    if senders.is_empty() {
        return;
    }
    let changes = crate::bot::take_setup();
    for mut s in &mut senders {
        for c in &changes {
            s.send::<InputChannelReliable>(BotSetupMsg(c.clone()));
        }
    }
}

/// The reliable channel for the bots' scene changes.
pub struct InputChannelReliable;

/// On a `--bot-hooks` server: each player's bot's scene changes, made to her counselor and the world.
#[allow(clippy::type_complexity)]
fn bot_hooks(
    mut commands: Commands,
    mut links: Query<(Entity, &mut MessageReceiver<BotSetupMsg>)>,
    mut counselors: Query<(Entity, &PlayedBy, &mut Transform), Without<crate::jason::Jason>>,
    mut doors: Query<(Entity, &mut crate::doors::Door, &GlobalTransform)>,
    mut barricades: Query<&mut crate::windows::Barricade>,
    mut jasons: Query<(&mut crate::jason::Jason, Option<&mut crate::senses::Knowledge>)>,
    time: Res<Time>,
    (own, mut starts, mut dropped, remote_jason): (Query<(Entity, &Transform), (With<LocalPlayer>, With<Player>, Without<PlayedBy>)>, ResMut<FreeStarts>, Local<bool>, Option<Res<crate::lobby::JasonPlayer>>),
    mut players: Query<(&PlayedBy, &mut Player)>,
    jason_entities: Query<Entity, With<crate::jason::Jason>>,
    (mut fuse_boxes, mut grids): (Query<&mut crate::power::FuseBox>, Option<ResMut<crate::power::PowerGrids>>),
    (drivers, mut cars): (Query<(&PlayedBy, &crate::vehicle::Driving)>, Query<&mut Transform, (With<crate::props::Vehicle>, Without<PlayedBy>, Without<Player>)>),
    (mut phone_boxes, mut phones, mut game): (Query<&mut crate::police::PhoneBox>, Query<&mut crate::police::Phone>, Option<ResMut<crate::match_flow::GameMatch>>),
    mut jason_xfs: Query<&mut Transform, (With<crate::jason::Jason>, Without<PlayedBy>, Without<Player>, Without<crate::props::Vehicle>)>,
    (others, vehicles, mut unplayed): (Query<(Entity, Has<crate::vehicle::Driving>, Has<crate::vehicle::Escaped>), (With<Player>, Without<PlayedBy>)>, Query<&crate::props::Vehicle>, Query<&mut Player, (Without<PlayedBy>, Without<LocalPlayer>)>),
) {
    for (link, mut r) in &mut links {
        for BotSetupMsg(change) in r.receive() {
            // A test's server plays no one (its own counselor would be Jason's nearest): gone, as a dedicated
            // lobby's, at the first scene change. (Not when a player's Jason is the test: with no counselor at all
            // he'd have no target, and update_jason skips him.)
            if !*dropped && remote_jason.is_none() {
                *dropped = true;
                for (e, xf) in &own {
                    starts.0.insert(0, xf.translation);
                    commands.entity(e).despawn();
                }
            }
            info!("net: bot hook from {link}: {change:?}");
            match change {
                crate::bot::BotSetup::Place(at) => {
                    if let Some((e, _, mut xf)) = counselors.iter_mut().find(|(_, by, _)| by.0 == link) {
                        xf.translation = Vec3::from_array(at);
                        // Off any special move's way onto its spot (it would carry her back there). (A door's or window's
                        // move plays on: its fixture moves with it.)
                        commands.entity(e).insert(crate::player::Unsettled).remove::<crate::interact::MoveToSpot>();
                    }
                }
                crate::bot::BotSetup::Door { at, barred } => {
                    let at = Vec3::from_array(at);
                    let Some((de, mut d, _)) = doors.iter_mut().min_by(|a, b| a.2.transform_point(a.1.interact).distance(at).total_cmp(&b.2.transform_point(b.1.interact).distance(at))) else { continue };
                    if barred {
                        for mut b in barricades.iter_mut().filter(|b| b.door == Some(de)) {
                            b.enabled = true;
                        }
                    } else {
                        d.locked = true;
                    }
                }
                crate::bot::BotSetup::Abilities => {
                    for (mut j, k) in &mut jasons {
                        if let Some(mut k) = k {
                            let a = &mut k.abilities;
                            a.unlocked = [true; 4];
                            a.recharge_timer = a.tuning.recharge_time;
                        }
                        j.skip_morph_grace();
                    }
                }
                crate::bot::BotSetup::Give(classes) => {
                    if let Some((e, _, _)) = counselors.iter().find(|(_, by, _)| by.0 == link) {
                        commands.entity(e).insert(crate::items::GiveItems(classes));
                    }
                }
                crate::bot::BotSetup::Health(h) => {
                    if let Some((_, mut p)) = players.iter_mut().find(|(by, _)| by.0 == link) {
                        p.counselor.health = h.clamp(1.0, p.counselor.health.max(100.0));
                    }
                }
                crate::bot::BotSetup::CutPower => {
                    for mut f in &mut fuse_boxes {
                        match grids.as_deref_mut() {
                            Some(g) => crate::power::set_destroyed(&mut f, true, g),
                            None => f.destroyed = true,
                        }
                    }
                }
                crate::bot::BotSetup::PlaceCar { at, yaw } => {
                    if let Some(mut xf) = drivers.iter().find(|(by, _)| by.0 == link).and_then(|(_, d)| cars.get_mut(d.vehicle).ok()) {
                        xf.translation = Vec3::from_array(at);
                        xf.rotation = Quat::from_rotation_y(yaw);
                    }
                }
                crate::bot::BotSetup::PlaceJason(at) => {
                    for mut xf in &mut jason_xfs {
                        xf.translation = Vec3::from_array(at);
                    }
                }
                crate::bot::BotSetup::BreakPhoneBox => {
                    if let Some(mut b) = phone_boxes.iter_mut().find(|b| b.phone.is_some()) {
                        crate::police::break_phone_box(&mut b, &mut phones);
                    }
                }
                crate::bot::BotSetup::PoliceSoon => {
                    if let Some(g) = game.as_mut() {
                        g.state.police_time_remaining = g.state.police_time_remaining.min(10);
                    }
                }
                crate::bot::BotSetup::NoJason => {
                    for e in &jason_entities {
                        commands.entity(e).despawn();
                    }
                }
                crate::bot::BotSetup::SeatPassenger => {
                    let car = drivers.iter().find(|(by, _)| by.0 == link).map(|(_, d)| d.vehicle);
                    let seat = car.and_then(|c| vehicles.get(c).ok()).and_then(|v| v.passengers.first().map(|s| s.right));
                    if let (Some(car), Some(right), Some((e, _, _))) = (car, seat, others.iter().find(|(_, seated, escaped)| !seated && !escaped)) {
                        commands.entity(e).insert((crate::vehicle::Driving::ride(car, 0, right), PlayAction("f_normal_stand_car_enter_right".into())));
                    }
                }
                crate::bot::BotSetup::Die => {
                    if let Some((_, mut p)) = players.iter_mut().find(|(by, _)| by.0 == link) {
                        p.counselor.health = 0.0;
                    }
                }
                crate::bot::BotSetup::KillAnother => {
                    if let Some(mut p) = unplayed.iter_mut().find(|p| p.counselor.alive()) {
                        p.counselor.health = 0.0;
                    }
                }
                crate::bot::BotSetup::Rage => {
                    for (_, k) in &mut jasons {
                        if let Some(mut k) = k {
                            k.abilities.rage_active = true;
                        }
                    }
                }
                crate::bot::BotSetup::Heard(at) => {
                    for (_, k) in &mut jasons {
                        if let Some(mut k) = k.filter(|k| !k.sees) {
                            k.last_known = Some((Vec3::from_array(at), time.elapsed_secs(), crate::senses::Clue::Heard));
                        }
                    }
                }
            }
        }
    }
}

/// On a lobby's server: its matches end back in the lobby ([`relaunch`]).
#[derive(Resource)]
pub(crate) struct Rematch;

/// This process again with `args` (its own, less `drop`'s flags and their values, plus `add`), and this one leaves: a
/// lobby's match over, the server starts a new lobby with a new world and the clients join it again (the game travels
/// everyone back to its lobby map; PLACEHOLDER: a restart stands in for the travel, there being one world per process).
pub(crate) fn relaunch(args: &[String], drop: &[&str], add: &[(&str, String)]) -> std::io::Result<std::process::Child> {
    let exe = std::env::current_exe()?;
    std::process::Command::new(exe).args(relaunch_args(args, drop, add)).spawn()
}

fn relaunch_args(args: &[String], drop: &[&str], add: &[(&str, String)]) -> Vec<String> {
    let mut out = Vec::new();
    let mut it = args.iter();
    while let Some(a) = it.next() {
        if drop.contains(&a.as_str()) {
            it.next();
            continue;
        }
        out.push(a.clone());
    }
    for (flag, v) in add {
        out.push(flag.to_string());
        out.push(v.clone());
    }
    out
}

/// On a client: its netcode id from `--net-id`.
#[derive(Resource)]
struct NetId(Option<u64>);

impl Plugin for NetPlugin {
    fn build(&self, app: &mut App) {
        match self.role {
            Role::Server(port) => {
                app.add_plugins(server::ServerPlugins { tick_duration: TICK });
                protocol(app);
                if self.bot_hooks {
                    app.add_systems(Update, bot_hooks);
                    // A test's server draws nothing (it plays no one, and the client's bot shares its GPU: two worlds
                    // drawn at once lost the device).
                    app.add_systems(Update, |mut cameras: Query<&mut Camera, With<Camera3d>>| {
                        for mut c in &mut cameras {
                            if c.is_active {
                                c.is_active = false;
                            }
                        }
                    });
                }
                if self.lobby {
                    app.insert_resource(Rematch).insert_resource(crate::lobby::LastJason(self.last_jason));
                }
                app.init_resource::<FreeStarts>()
                    .insert_resource(ServerPort(port))
                    .add_systems(Startup, start_server)
                    .add_observer(new_link)
                    .add_observer(player_joined)
                    .add_observer(player_left)
                    .add_observer(|add: On<Add, RestartAction>, mut actions: Query<&mut NetAction>| {
                        if let Ok(mut a) = actions.get_mut(add.entity) {
                            a.restarts += 1;
                        }
                    })
                    .add_observer(|insert: On<Insert, SeekAction>, seeks: Query<&SeekAction>, mut actions: Query<&mut NetAction>| {
                        if let (Ok(s), Ok(mut a)) = (seeks.get(insert.entity), actions.get_mut(insert.entity)) {
                            a.seek = (a.seek.0 + 1, s.0);
                        }
                    })
                    // Before the frame's gameplay (each press shows for exactly one frame).
                    .add_systems(PreUpdate, apply_inputs.after(MessageSystems::Receive));
                crate::net_world::server(app);
                crate::voice::server(app, self.dedicated);
                if self.lobby {
                    crate::lobby::server(app, self.dedicated, self.name.clone(), self.last_match.clone(), self.class.clone());
                    if self.bot && !self.dedicated {
                        app.insert_resource(NetBot { prefer: self.prefer, ..default() })
                            .add_systems(Update, (net_bot_lobby.before(crate::lobby::Keys), host_bot_jason.run_if(resource_exists::<crate::jason_player::PlayJason>)));
                    }
                }
                crate::lobby::ui(app);
                app
                    .add_systems(Startup, spawn_match)
                    .add_systems(PostUpdate, (replicate_bodies, update_bodies, send_match).chain().before(ReplicationSystems::Send));
            }
            Role::Client(addr) => {
                app.add_plugins(client::ClientPlugins { tick_duration: TICK });
                protocol(app);
                app.insert_resource(NetClient)
                    .insert_resource(ServerAddr(addr))
                    .insert_resource(NetId(self.net_id))
                    // (Its scenario bot, if any, plays here: bot.rs.)
                    .add_systems(Startup, || crate::bot::ON_CLIENT.store(true, std::sync::atomic::Ordering::Relaxed))
                    .add_systems(Update, send_bot_setup)
                    // A client's --give is the server's to make (bot hooks).
                    .add_systems(Update, |mut commands: Commands, give: Option<Res<crate::items::Give>>| {
                        if let Some(g) = give {
                            crate::bot::setup(crate::bot::BotSetup::Give(g.0.clone()));
                            commands.remove_resource::<crate::items::Give>();
                        }
                    })
                    // And its --health (her health is the server's).
                    .add_systems(Update, |config: Option<Res<crate::player::PlayerConfig>>, mut sent: Local<bool>| {
                        if let (Some(h), false) = (config.and_then(|c| c.health), *sent) {
                            crate::bot::setup(crate::bot::BotSetup::Health(h));
                            *sent = true;
                        }
                    })
                    .add_systems(Update, connect)
                    .add_observer(body_gone)
                    .init_resource::<Twins>()
                    .init_resource::<ServerClock>()
                    .add_systems(Update, (send_input, (make_twins, record_bodies, mirror, hide_jason, sync_classes, sync_morph_map, sync_hunter_intro).chain().in_set(Mirrored).before(crate::character::animate)));
                app.add_systems(Update, receive_match.after(Mirrored));
                crate::net_world::client(app);
                crate::lobby::client(app);
                crate::voice::client(app);
                crate::lobby::ui(app);
                app.insert_resource(ClientName(self.name.clone()));
                if self.bot {
                    app.insert_resource(NetBot { prefer: self.prefer, abilities: self.bot_abilities, ..default() })
                        .add_systems(Update, (net_bot_lobby, net_bot.run_if(not(resource_exists::<crate::jason_player::PlayJason>)), net_bot_jason.run_if(resource_exists::<crate::jason_player::PlayJason>)).chain().before(send_input).before(crate::lobby::Keys));
                }
            }
        }
    }
}

/// What both sides agree on: the input message and its channel, and the replicated body.
fn protocol(app: &mut App) {
    app.add_channel::<InputChannel>(ChannelSettings { mode: ChannelMode::SequencedUnreliable, ..default() }).add_direction(NetworkDirection::ClientToServer);
    app.register_message::<InputMsg>().add_direction(NetworkDirection::ClientToServer);
    app.add_channel::<InputChannelReliable>(ChannelSettings { mode: ChannelMode::OrderedReliable(ReliableSettings::default()), ..default() }).add_direction(NetworkDirection::ClientToServer);
    app.register_message::<BotSetupMsg>().add_direction(NetworkDirection::ClientToServer);
    app.component::<NetBody>().replicate();
    app.component::<NetCounselor>().replicate();
    app.component::<NetJason>().replicate();
    app.component::<NetAction>().replicate();
    app.component::<NetMatch>().replicate();
    app.component::<NetTally>().replicate();
    crate::net_world::protocol(app);
    crate::lobby::protocol(app);
    crate::voice::protocol(app);
    crate::emotes::protocol(app);
}

#[derive(Resource)]
struct ServerPort(u16);

#[derive(Resource)]
struct ServerAddr(SocketAddr);

fn start_server(mut commands: Commands, port: Res<ServerPort>) {
    // No server address check: the server binds 0.0.0.0, which netcode matches only to a loopback token address, so
    // a client that typed the host's LAN or public address was ignored ("not in connect token whitelist"). The
    // check guards a token against use on another server; with every build's shared DEV_KEY it guards nothing.
    let config = lightyear::prelude::server::NetcodeConfig {
        protocol_id: PROTOCOL_ID,
        private_key: DEV_KEY,
        client_timeout_secs: TIMEOUT_SECS,
        server_addr_check: false,
        ..default()
    };
    let server = commands
        .spawn((
            Name::new("server"),
            lightyear::prelude::server::NetcodeServer::new(config),
            LocalAddr(SocketAddr::new(Ipv4Addr::UNSPECIFIED.into(), port.0)),
            lightyear::prelude::server::ServerUdpIo::default(),
        ))
        .id();
    commands.trigger(lightyear::prelude::server::Start { entity: server });
    info!("net: serving on UDP port {}", port.0);
}

/// A client's link: it gets the world's replication.
fn new_link(link: On<Add, LinkOf>, mut commands: Commands) {
    commands.entity(link.entity).insert((ReplicationSender, Name::new("player link")));
}

/// A player connected: her counselor at the next free start.
#[allow(clippy::too_many_arguments)]
fn player_joined(
    joined: On<Add, Connected>,
    links: Query<&RemoteId, With<LinkOf>>,
    mut commands: Commands,
    assets: Res<AssetServer>,
    character: Option<Res<crate::character::CounselorCharacter>>,
    tuning: Res<crate::player::PlayerTuning>,
    mut starts: ResMut<FreeStarts>,
    lobby: Option<Res<crate::lobby::LobbyOpen>>,
    (others, mut rng): (Query<&Transform, With<Player>>, ResMut<crate::GameRng>),
) {
    let Ok(id) = links.get(joined.entity) else { return };
    // In the lobby, she plays once the match starts (lobby.rs).
    if lobby.is_some() {
        return;
    }
    let occupied: Vec<Vec3> = others.iter().map(|t| t.translation).collect();
    let Some(at) = starts.take(&occupied, &mut || rng.0.frand()) else {
        warn!("net: player {:?} joined with no counselor start left", id.0);
        return;
    };
    let counselor = crate::player::spawn_counselor(&mut commands, &assets, character.as_deref(), &tuning, at, crate::character::DEFAULT_CLASS);
    commands.entity(counselor).insert((PlayedBy(joined.entity), ControlledBy { owner: joined.entity, lifetime: default() }));
    info!("net: player {:?} joined: counselor {counselor} at {at}", id.0);
}

/// A player gone: her counselor with her (her start free again).
fn player_left(left: On<Remove, Connected>, counselors: Query<(Entity, &PlayedBy, &Transform)>, mut commands: Commands, mut starts: ResMut<FreeStarts>) {
    for (e, by, xf) in &counselors {
        if by.0 == left.entity {
            info!("net: player left: counselor {e} gone");
            starts.0.push(xf.translation);
            commands.entity(e).despawn();
        }
    }
}

/// Each player's newest input into her counselor's controls, with the buttons pressed or released since the last
/// (their counts changed: [`InputMsg`]). A frame with no input keeps the buttons held, none pressed again.
fn apply_inputs(mut commands: Commands, mut links: Query<(Entity, &mut MessageReceiver<InputMsg>, Option<&mut LinkCounts>)>, mut counselors: Query<(&PlayedBy, &mut Controls)>) {
    for (link, mut receiver, counts) in &mut links {
        let input = receiver.receive().last();
        let edges = match (&input, counts) {
            (Some(i), Some(mut c)) => {
                let edges = (counted(&c.presses, &i.presses), counted(&c.releases, &i.releases));
                (c.presses, c.releases) = (i.presses, i.releases);
                edges
            }
            (Some(i), None) => {
                commands.entity(link).insert(LinkCounts { presses: i.presses, releases: i.releases });
                (0, 0)
            }
            (None, _) => (0, 0),
        };
        for (by, mut c) in &mut counselors {
            if by.0 == link {
                *c = match &input {
                    Some(i) => c.next_with(i.held, edges.0, edges.1, i.yaw, i.pitch),
                    None => c.held_still(),
                };
            }
        }
    }
}

/// Every character goes out to all players.
fn replicate_bodies(
    mut commands: Commands,
    counselors: Query<Entity, (With<Player>, Without<NetBody>)>,
    jasons: Query<Entity, (With<crate::jason::Jason>, Without<NetBody>)>,
) {
    for e in &counselors {
        commands.entity(e).insert((NetBody { id: e.to_bits(), pos: [0.0; 3], yaw: 0.0, kind: BodyKind::Counselor, t: 0.0 }, NetCounselor::default(), NetTally::default(), NetAction::default(), Replicate::to_clients(NetworkTarget::All)));
    }
    for e in &jasons {
        commands.entity(e).insert((NetBody { id: e.to_bits(), pos: [0.0; 3], yaw: 0.0, kind: BodyKind::Jason, t: 0.0 }, NetJason::default(), NetAction::default(), Replicate::to_clients(NetworkTarget::All)));
    }
}

/// Writes `v` only if it changed (an unchanged component doesn't go out again).
fn set<T: PartialEq>(mut c: Mut<T>, v: T) {
    if *c != v {
        *c = v;
    }
}

#[allow(clippy::type_complexity)]
fn update_bodies(
    mut bodies: Query<(&mut NetBody, &Transform, &mut NetAction, Option<&PlayAction>, Has<crate::items::Blocking>)>,
    mut counselors: Query<(&mut NetCounselor, &Player, Option<&Name>, Option<&Controls>, (Has<crate::jason::Held>, Option<&crate::items::Trapped>, Has<crate::voice::Spectating>, Option<&crate::character::CounselorClass>, Option<&crate::character::CounselorOutfit>), (Option<&crate::hiding::Hiding>, Option<&crate::hiding::Breathing>, Option<&crate::interact::Interaction>), (Option<&crate::vehicle::Driving>, Option<&crate::vehicle::Escaped>, Has<crate::interact::MoveToSpot>, Has<crate::interact::DoorMove>, Has<crate::interact::WindowMove>, Option<&crate::blood::Gore>, Has<crate::tommy::HunterIntro>))>,
    struggle: Option<Res<crate::jason::Struggle>>,
    mut jasons: Query<(&mut NetJason, &crate::jason::Jason, Option<&crate::senses::Knowledge>, Option<&crate::kill_camera::KillCam>)>,
    world_ids: Query<&crate::net_world::WorldId>,
    fixed: Res<Time<Fixed>>,
    morph_map: Option<Res<crate::jason_player::MorphMap>>,
) {
    for (mut b, xf, mut a, action, blocking) in &mut bodies {
        let pos = xf.translation.to_array();
        let yaw = xf.rotation.to_euler(EulerRot::YXZ).0;
        if b.pos != pos || b.yaw != yaw {
            b.pos = pos;
            b.yaw = yaw;
            // The fixed step the body moved in (the characters move only there).
            b.t = fixed.elapsed_secs();
        }
        let take = action.map(|p| p.0.clone());
        if a.take != take || a.blocking != blocking {
            a.take = take;
            a.blocking = blocking;
        }
    }
    for (n, p, name, controls, (held, trapped, spectating, class, outfit), (hiding, breathing, interaction), (driving, escaped, to_spot, door_move, window_move, blood, hunter_intro)) in &mut counselors {
        let c = &p.counselor;
        set(n, NetCounselor {
            stance: c.stance as u8,
            wounded: c.wounded,
            health: c.health,
            stamina: c.stamina,
            sprinting: c.sprinting,
            running: c.running,
            velocity: p.velocity.to_array(),
            fear: p.fear,
            montage: p.montage.clone(),
            root_motion: p.root_motion.is_some(),
            hidden: p.hidden,
            grounded: p.grounded,
            name: name.map_or(String::new(), |n| n.to_string()),
            look: controls.map_or([0.0; 2], |c| [c.yaw, c.pitch]),
            held: held.then(|| struggle.as_ref().map_or(0.0, |s| s.wiggle.completion)),
            trapped: trapped.map(|t| t.struggle()),
            spectating,
            hiding: hiding.and_then(|h| Some((world_ids.get(h.spot).ok()?.0, h.side as u8, h.phase.index()))),
            breath: breathing.map_or((0.0, false), |b| (b.breath.current, b.breath.holding)),
            hold: interaction.and_then(|i| i.hold.as_ref()).map(NetHold::of),
            driving: driving.and_then(|d| Some((world_ids.get(d.vehicle).ok()?.0, d.seat.map(|s| s as u8), d.starting.as_ref().map(NetHold::of)))),
            escaped: escaped.map(|e| e.how.clone()),
            moving: to_spot || door_move || window_move,
            gore: blood.cloned().unwrap_or_default(),
            class: class.map_or(String::new(), |c| c.0.clone()),
            hunter_intro,
            outfit: outfit.map_or(String::new(), |o| o.0.clone()),
        });
    }
    for (n, j, know, cam) in &mut jasons {
        let a = know.map(|k| &k.abilities);
        set(n, NetJason {
            velocity: j.velocity.to_array(),
            combat: j.combat,
            blocking: j.blocking,
            done: j.state == crate::jason::JasonState::Done,
            shifting: j.shifting(),
            stalking: j.stalking(),
            holding: j.holding(),
            knife_aim: j.aiming_knife(),
            hidden: j.hidden(),
            underwater: j.underwater,
            door: j.door_move().and_then(|(d, m)| Some((world_ids.get(d).ok()?.0, m))),
            knives: j.knives,
            traps: j.traps,
            mask_on: j.mask_on,
            health: j.health,
            morph_map: morph_map.as_ref().is_some_and(|m| m.morph),
            sweater_stunned: j.sweater_stunned,
            minigame: j.stun_minigame().map(|(c, t)| ((c.clamp(0.0, 1.0) * 100.0).round() as u8, t.to_string())),
            // (Tenths of a second: an unchanged component doesn't go out again.)
            sense: know.map_or((0.0, 0.0), |k| (((k.sense_until - fixed.elapsed_secs()).max(0.0) * 10.0).ceil() / 10.0, k.sense_length)),
            sense_avoid: know.map_or_else(Vec::new, |k| k.sense_avoiders.iter().map(|e| e.to_bits()).collect()),
            state: j.state_name(),
            cooldown: j.cooldown,
            unlocked: a.map_or([false; 4], |a| a.unlocked),
            recharge: a.map_or([0.0; 4], |a| a.recharge_timer),
            unlock_timer: a.map_or(0.0, |a| a.unlock_timer),
            rage: a.map_or((false, 0.0), |a| (a.rage_active, a.rage_timer)),
            kill_cam_of: cam.map_or((None, false), |c| (c.victim.map(|v| v.to_bits()), c.everyone)),
            kill_cam: cam.map(|c| {
                let (k, o, root) = c.parts();
                let (p, r) = (o.translation, o.rotation);
                (k.to_string(), [p.x, p.y, p.z, r.x, r.y, r.z, r.w], root)
            }),
        });
    }
}

/// Once the level is in (its first frames hitch while it streams), the connection.
/// On a client: the name it gives the lobby.
#[derive(Resource)]
struct ClientName(String);

#[allow(clippy::too_many_arguments)]
fn connect(mut commands: Commands, server: Res<ServerAddr>, level: Res<crate::Level>, assets: Res<AssetServer>, name: Res<ClientName>, net_id: Res<NetId>, mut done: Local<bool>) {
    if *done || level.scenes.is_empty() || !level.scenes.iter().all(|h| assets.is_loaded_with_dependencies(h.id())) {
        return;
    }
    *done = true;
    // A client id from the clock (PLACEHOLDER: a backend would hand it out with the token).
    let id = net_id.0.unwrap_or_else(|| std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(1, |d| d.as_nanos() as u64));
    let auth = Authentication::Manual { server_addr: server.0, client_id: id, private_key: DEV_KEY, protocol_id: PROTOCOL_ID };
    let name = if name.0.is_empty() { format!("player {}", id % 10000) } else { name.0.clone() };
    commands.insert_resource(crate::lobby::Me { id, name });
    let config = lightyear::prelude::client::NetcodeConfig { client_timeout_secs: TIMEOUT_SECS, token_expire_secs: TIMEOUT_SECS, ..default() };
    let netcode = match lightyear::prelude::client::NetcodeClient::new(auth, config) {
        Ok(n) => n,
        Err(e) => {
            error!("net: can't make the netcode client: {e:?}");
            return;
        }
    };
    let client = commands
        .spawn((
            Name::new("client"),
            ReplicationReceiver,
            netcode,
            LocalAddr(SocketAddr::new(Ipv4Addr::UNSPECIFIED.into(), 0)),
            UdpIo::default(),
        ))
        .id();
    commands.trigger(Connect { entity: client });
    info!("net: connecting to {}", server.0);
}

/// This player's input, every frame (the server keeps the newest).
fn send_input(local: Query<&Controls, With<LocalPlayer>>, mut senders: Query<&mut MessageSender<InputMsg>, With<Connected>>, mut counts: Local<Counts>) {
    let Ok(c) = local.single() else { return };
    let (pressed, released) = c.edges();
    counts.add(pressed, released);
    for mut s in &mut senders {
        s.send::<InputChannel>(InputMsg { held: c.held(), yaw: c.yaw, pitch: c.pitch, presses: counts.presses, releases: counts.releases });
    }
}

/// A client: the counselor coming as Tommy (her intro plays where she's the local player: tommy.rs).
fn sync_hunter_intro(mut commands: Commands, bodies: Query<(&Twin, &NetCounselor), Changed<NetCounselor>>, intros: Query<(), With<crate::tommy::HunterIntro>>) {
    for (t, n) in &bodies {
        match (n.hunter_intro, intros.contains(t.local)) {
            (true, false) => {
                commands.entity(t.local).insert(crate::tommy::HunterIntro);
            }
            (false, true) => {
                commands.entity(t.local).remove::<crate::tommy::HunterIntro>();
            }
            _ => {}
        }
    }
}

/// The client that plays Jason: his map out for a Morph as the server has it (pressing 1 opens it there), so the
/// map shows here and its click picks the place (sent back: lobby.rs send_jason_input).
fn sync_morph_map(own: Query<&NetJason, (With<Controlled>, Changed<NetJason>)>, map: Option<ResMut<crate::jason_player::MorphMap>>) {
    let (Ok(n), Some(mut map)) = (own.single(), map) else { return };
    if map.morph != n.morph_map {
        map.morph = n.morph_map;
        map.open = n.morph_map;
        if !n.morph_map {
            map.click = None;
        }
    }
}

/// A counselor's class and outfit as the server has them (her lobby pick and wardrobe; Tommy's, revived as him:
/// tommy.rs) on her twin (player.rs restyles her).
fn sync_classes(mut commands: Commands, bodies: Query<(&Twin, &NetCounselor), Changed<NetCounselor>>, classes: Query<(Option<&crate::character::CounselorClass>, Option<&crate::character::CounselorOutfit>)>) {
    for (t, n) in &bodies {
        let Ok((class, outfit)) = classes.get(t.local) else { continue };
        if !n.class.is_empty() && class.map_or(true, |c| c.0 != n.class) {
            commands.entity(t.local).insert(crate::character::CounselorClass(n.class.clone()));
        }
        if outfit.map_or(!n.outfit.is_empty(), |o| o.0 != n.outfit) {
            commands.entity(t.local).insert(crate::character::CounselorOutfit(n.outfit.clone()));
        }
    }
}

/// Each character received gets its twin: her own counselor is the local one, Jason the local Jason, another
/// player's counselor one spawned for her.
#[allow(clippy::too_many_arguments)]
fn make_twins(
    mut commands: Commands,
    assets: Res<AssetServer>,
    counselor: Option<Res<crate::character::CounselorCharacter>>,
    tuning: Res<crate::player::PlayerTuning>,
    bodies: Query<(Entity, &NetBody, Has<Controlled>, Option<&NetCounselor>), Without<Twin>>,
    local: Query<Entity, (With<LocalPlayer>, With<Player>)>,
    jason: Query<Entity, With<crate::jason::Jason>>,
    mut index: ResMut<Twins>,
) {
    for (e, b, own, nc) in &bodies {
        let at = Vec3::from_array(b.pos);
        let class = nc.map(|n| n.class.as_str()).filter(|c| !c.is_empty()).unwrap_or(crate::character::DEFAULT_CLASS);
        let twin = match (b.kind, own) {
            (BodyKind::Counselor, true) => {
                // Hers as the server has her (her lobby pick).
                let l = local.single().ok();
                if let Some(l) = l {
                    commands.entity(l).insert(crate::character::CounselorClass(class.to_string()));
                }
                l
            }
            (BodyKind::Counselor, false) => Some(crate::player::spawn_counselor(&mut commands, &assets, counselor.as_deref(), &tuning, at, class)),
            (BodyKind::Jason, _) => jason.single().ok(),
        };
        let Some(twin) = twin else { continue };
        info!("net: received {:?} {e} at {at}{}: twin {twin}", b.kind, if own { " (hers)" } else { "" });
        commands.entity(e).insert(Twin { local: twin, restarts: 0, seeks: 0, kill: None });
        index.0.insert(b.id, twin);
    }
}

/// The local Jason shows only as the server's Jason's twin (the server may have none: `--no-jason`), and not while the
/// server has him faded out or under the water.
fn hide_jason(twins: Res<Twins>, mut jasons: Query<(Entity, &mut Visibility, &crate::jason::Jason)>) {
    for (e, mut v, j) in &mut jasons {
        let shown = twins.0.values().any(|t| *t == e) && !j.hidden();
        v.set_if_neq(if shown { Visibility::Inherited } else { Visibility::Hidden });
    }
}

/// A character gone from the server: a spawned twin goes with it.
fn body_gone(gone: On<Remove, NetBody>, twins: Query<(&NetBody, &Twin, Has<Controlled>)>, mut commands: Commands) {
    if let Ok((b, t, own)) = twins.get(gone.entity) {
        if b.kind == BodyKind::Counselor && !own {
            commands.entity(t.local).despawn();
        }
    }
}

fn stance(n: u8) -> Stance {
    match n {
        1 => Stance::Prone,
        2 => Stance::Crouching,
        3 => Stance::Swimming,
        4 => Stance::Driving,
        5 => Stance::Combat,
        6 => Stance::Finished,
        7 => Stance::Grabbed,
        8 => Stance::Trapped,
        _ => Stance::Standing,
    }
}

/// How far behind the newest body a twin is shown (s): between the two bodies received around then, so the server's
/// ticks and the network's jitter don't show. A body further than [`SNAP_M`] from the one before is a jump (a
/// teleport, a respawn): no blend across it.
const INTERP_DELAY_S: f32 = 0.1;
const SNAP_M: f32 = 3.0;
/// How long the bodies received are kept (s).
const SNAPSHOTS_KEPT_S: f32 = 1.0;
/// A body that changes after standing still longer than this (s) held its last place until about this long before
/// (the server sends a body only when it moves, every frame while it does).
const STILL_GAP_S: f32 = 0.05;
/// How fast the server clock estimate rises towards a later arrival, per body received (it drops to an earlier one
/// at once): the fastest arrivals set it, so a slow packet doesn't move it.
const CLOCK_RISE: f32 = 0.002;

/// On a client: this machine's real clock minus the server's, as the fastest bodies to arrive show it.
#[derive(Resource, Default)]
struct ServerClock(Option<f32>);

impl ServerClock {
    /// The server's clock now (s), by this machine's.
    fn now(&self, real: &Time<Real>) -> f32 {
        real.elapsed_secs() - self.0.unwrap_or(0.0)
    }
}

/// On a client: a character's bodies as the server took them (server time, place, yaw).
#[derive(Component, Default)]
struct Snapshots(std::collections::VecDeque<(f32, Vec3, f32)>);

impl Snapshots {
    /// Adds a body taken at server time `t` (an older one than the newest, arrived late, is dropped). After standing
    /// still, the body held its place until just before it moved.
    fn push(&mut self, t: f32, at: Vec3, yaw: f32) {
        if let Some(&(t0, p0, y0)) = self.0.back() {
            if t <= t0 {
                return;
            }
            if t - t0 > 2.0 * STILL_GAP_S {
                self.0.push_back((t - STILL_GAP_S, p0, y0));
            }
        }
        self.0.push_back((t, at, yaw));
        while self.0.front().is_some_and(|f| t - f.0 > SNAPSHOTS_KEPT_S) && self.0.len() > 2 {
            self.0.pop_front();
        }
    }
}

/// Each body received is kept, with when the server took it.
fn record_bodies(
    mut commands: Commands,
    real: Res<Time<Real>>,
    mut clock: ResMut<ServerClock>,
    mut bodies: Query<(Entity, &NetBody, Option<&mut Snapshots>), Changed<NetBody>>,
) {
    let now = real.elapsed_secs();
    for (e, b, snaps) in &mut bodies {
        let behind = now - b.t;
        clock.0 = Some(match clock.0 {
            Some(c) if behind >= c => c + (behind - c) * CLOCK_RISE,
            _ => behind,
        });
        let s = (b.t, Vec3::from_array(b.pos), b.yaw);
        match snaps {
            Some(mut q) => q.push(s.0, s.1, s.2),
            None => {
                commands.entity(e).insert(Snapshots([s].into()));
            }
        }
    }
}

/// The place and yaw at time `t` among the bodies: between the two around it (no blend across a jump), else the
/// nearest end.
fn sample(snaps: &Snapshots, t: f32) -> Option<(Vec3, f32)> {
    let q = &snaps.0;
    let last = q.back()?;
    if t >= last.0 {
        return Some((last.1, last.2));
    }
    let i = q.iter().position(|s| s.0 > t)?;
    if i == 0 {
        return Some((q[0].1, q[0].2));
    }
    let (a, b) = (q[i - 1], q[i]);
    if a.1.distance(b.1) > SNAP_M {
        return Some((b.1, b.2));
    }
    let f = ((t - a.0) / (b.0 - a.0).max(1e-4)).clamp(0.0, 1.0);
    let dy = (b.2 - a.2 + std::f32::consts::PI).rem_euclid(std::f32::consts::TAU) - std::f32::consts::PI;
    Some((a.1.lerp(b.1, f), a.2 + dy * f))
}

/// Each twin takes its character's state.
#[allow(clippy::type_complexity)]
fn mirror(
    mut commands: Commands,
    (real, clock): (Res<Time<Real>>, Res<ServerClock>),
    mut bodies: Query<(&NetBody, &NetAction, Option<&NetCounselor>, Option<&NetJason>, &mut Twin, Option<&Snapshots>)>,
    mut twins: Query<(&mut Transform, Option<&mut Player>, Option<&mut crate::jason::Jason>, Option<&PlayAction>, Has<crate::items::Blocking>, (Option<&Name>, Option<&mut Controls>, Has<LocalPlayer>, Option<&mut crate::senses::Knowledge>), (Has<crate::jason::Held>, Option<&mut crate::items::Trapped>, Has<crate::voice::Spectating>), (Option<&mut crate::hiding::Hiding>, Option<&mut crate::hiding::Breathing>, Option<&mut crate::interact::Interaction>), (Option<&mut crate::vehicle::Driving>, Has<crate::vehicle::Escaped>, Has<RemoteMove>, Option<&crate::blood::Gore>))>,
    mut struggle: Option<ResMut<crate::jason::Struggle>>,
    actors: Res<crate::net_world::Actors>,
    cams: Query<&crate::kill_camera::KillCam>,
    time: Res<Time>,
    bodies_of: Res<Twins>,
) {
    let shown_at = clock.now(&real) - INTERP_DELAY_S;
    for (b, a, nc, nj, mut twin, snaps) in &mut bodies {
        let Ok((mut xf, player, jason, action, blocking, (name, controls, local, know), (held, trapped, spectating), (hiding, breathing, interaction), (driving, escaped, remote_move, blood))) = twins.get_mut(twin.local) else { continue };
        let (to, yaw) = snaps.and_then(|s| sample(s, shown_at)).unwrap_or((Vec3::from_array(b.pos), b.yaw));
        xf.translation = to;
        xf.rotation = Quat::from_rotation_y(yaw);
        if let (Some(mut p), Some(n)) = (player, nc) {
            let c = &mut p.counselor;
            c.stance = stance(n.stance);
            c.wounded = n.wounded;
            c.health = n.health;
            c.stamina = n.stamina;
            c.sprinting = n.sprinting;
            c.running = n.running;
            p.velocity = Vec2::from_array(n.velocity);
            p.fear = n.fear;
            p.montage = n.montage.clone();
            p.root_motion = n.root_motion.then_some((Vec2::ZERO, 0.0));
            p.hidden = n.hidden;
            p.grounded = n.grounded;
            // In his hands or a trap: the grab and trap cameras and the struggle's bar (the server runs the struggle).
            let mut e = commands.entity(twin.local);
            match (n.held, held) {
                (Some(c), _) => {
                    if !held {
                        e.insert(crate::jason::Held);
                    }
                    if local {
                        if let Some(s) = struggle.as_deref_mut() {
                            s.wiggle.completion = c;
                        }
                    }
                }
                (None, true) => {
                    e.remove::<crate::jason::Held>();
                }
                (None, false) => {}
            }
            // In a hiding spot: her hiding camera, the breath meter, the way out.
            match (n.hiding.and_then(|(id, side, phase)| Some((actors.get(id)?, side, phase))), hiding) {
                (Some((spot, side, phase)), Some(mut h)) => {
                    h.spot = spot;
                    h.side = side as usize;
                    h.phase = crate::hiding::HidePhase::from_index(phase);
                }
                (Some((spot, side, phase)), None) => {
                    e.insert(crate::hiding::Hiding::remote(spot, side as usize, crate::hiding::HidePhase::from_index(phase)));
                }
                (None, Some(_)) => {
                    e.remove::<crate::hiding::Hiding>();
                }
                (None, None) => {}
            }
            if let Some(mut b) = breathing {
                (b.breath.current, b.breath.holding) = n.breath;
            }
            if let Some(mut i) = interaction {
                i.hold = n.hold.as_ref().map(NetHold::hold);
            }
            // In a car (its seat, the ignition's dial) and out of the match.
            match (n.driving.as_ref().and_then(|(id, seat, start)| Some((actors.get(*id)?, *seat, start))), driving) {
                (Some((vehicle, seat, start)), Some(mut d)) => {
                    d.vehicle = vehicle;
                    d.seat = seat.map(|s| s as usize);
                    d.starting = start.as_ref().map(NetHold::hold);
                }
                (Some((vehicle, seat, start)), None) => {
                    e.insert(crate::vehicle::Driving { vehicle, seat: seat.map(|s| s as usize), starting: start.as_ref().map(NetHold::hold), seat_move: None });
                }
                (None, Some(_)) => {
                    e.remove::<crate::vehicle::Driving>();
                }
                (None, None) => {}
            }
            if blood.is_none_or(|b| *b != n.gore) && (n.gore != crate::blood::Gore::default() || blood.is_some()) {
                e.insert(n.gore.clone());
            }
            match (n.moving, remote_move) {
                (true, false) => {
                    e.insert(RemoteMove);
                }
                (false, true) => {
                    e.remove::<RemoteMove>();
                }
                _ => {}
            }
            if let (Some(how), false) = (&n.escaped, escaped) {
                e.insert(crate::vehicle::Escaped { how: how.clone() });
            }
            match (n.spectating, spectating) {
                (true, false) => {
                    e.insert(crate::voice::Spectating);
                }
                (false, true) => {
                    e.remove::<crate::voice::Spectating>();
                }
                _ => {}
            }
            match (n.trapped, trapped) {
                (Some(c), Some(mut t)) => t.set_struggle(c),
                (Some(c), None) => {
                    e.insert(crate::items::Trapped::remote(c));
                }
                (None, Some(_)) => {
                    e.remove::<crate::items::Trapped>();
                }
                (None, None) => {}
            }
            if !n.name.is_empty() && name.is_none_or(|x| x.as_str() != n.name) {
                commands.entity(twin.local).insert(Name::new(n.name.clone()));
            }
            // Another player's look (the local one's is this machine's mouse).
            if let (Some(mut c), false) = (controls, local) {
                c.yaw = n.look[0];
                c.pitch = n.look[1];
            }
        }
        if let (Some(mut j), Some(n)) = (jason, nj) {
            j.velocity = Vec2::from_array(n.velocity);
            j.combat = n.combat;
            j.blocking = n.blocking;
            if n.done {
                j.state = crate::jason::JasonState::Done;
            }
            j.underwater = n.underwater;
            j.knives = n.knives;
            j.mask_on = n.mask_on;
            j.sweater_stunned = n.sweater_stunned;
            j.health = n.health;
            j.traps = n.traps;
            j.remote = Some(crate::jason::RemoteView {
                shifting: n.shifting,
                stalking: n.stalking,
                holding: n.holding,
                knife_aim: n.knife_aim,
                hidden: n.hidden,
                door: n.door.and_then(|(id, m)| Some((actors.get(id)?, m))),
                minigame: n.minigame.as_ref().map(|(c, t)| (*c as f32 / 100.0, crate::jason::Stun::TEXTS.iter().find(|x| **x == t.as_str()).copied().unwrap_or("Resist"))),
            });
            j.remote_state = n.state.clone();
            j.cooldown = n.cooldown;
            let key = n.kill_cam.as_ref().map(|(k, ..)| k.clone());
            if key != twin.kill {
                match &n.kill_cam {
                    Some((k, o, root)) => {
                        let origin = Transform::from_xyz(o[0], o[1], o[2]).with_rotation(Quat::from_xyzw(o[3], o[4], o[5], o[6]));
                        let victim = n.kill_cam_of.0.and_then(|id| bodies_of.of(id));
                        commands.entity(twin.local).insert(crate::kill_camera::KillCam::remote(k, origin, *root, victim, n.kill_cam_of.1));
                    }
                    None if cams.contains(twin.local) => {
                        commands.entity(twin.local).remove::<crate::kill_camera::KillCam>();
                    }
                    None => {}
                }
                twin.kill = key;
            }
            if let Some(mut k) = know {
                // Sense's timer on this clock.
                let until = time.elapsed_secs() + n.sense.0;
                if (until - k.sense_until).abs() > 0.25 || n.sense.0 == 0.0 {
                    k.sense_until = if n.sense.0 > 0.0 { until } else { 0.0 };
                }
                k.sense_length = n.sense.1;
                let avoiders: Vec<Entity> = n.sense_avoid.iter().filter_map(|&id| bodies_of.of(id)).collect();
                if k.sense_avoiders != avoiders {
                    k.sense_avoiders = avoiders;
                }
                let a = &mut k.abilities;
                a.unlocked = n.unlocked;
                a.recharge_timer = n.recharge;
                a.unlock_timer = n.unlock_timer;
                (a.rage_active, a.rage_timer) = n.rage;
            }
        }
        let mut e = commands.entity(twin.local);
        match (&a.take, action) {
            (Some(t), Some(have)) if *t == have.0 => {}
            (Some(t), _) => {
                e.insert(PlayAction(t.clone()));
            }
            (None, Some(_)) => {
                e.remove::<PlayAction>();
            }
            (None, None) => {}
        }
        if a.restarts != twin.restarts {
            twin.restarts = a.restarts;
            e.insert(RestartAction);
        }
        if a.seek.0 != twin.seeks {
            twin.seeks = a.seek.0;
            e.insert(SeekAction(a.seek.1));
        }
        match (a.blocking, blocking) {
            (true, false) => {
                e.insert(crate::items::Blocking);
            }
            (false, true) => {
                e.remove::<crate::items::Blocking>();
            }
            _ => {}
        }
    }
}

/// The network test bot's state (`--bot-net` on a client, or a lobby's playing server).
#[derive(Resource, Default)]
struct NetBot {
    step: u8,
    t: f32,
    from: Vec3,
    jason_from: Option<Vec3>,
    clock: Option<i32>,
    tries: u8,
    failed: Vec<&'static str>,
    /// The first thrown knife seen: its entity, where Jason stood then, and the furthest it went from there.
    knife: Option<(Entity, Vec3, f32)>,
    /// What it asks the lobby for, and how far it got there (0 waiting, 1 asked, 2 ready).
    prefer: crate::lobby::Preference,
    lobby: u8,
    /// As Jason, also use his abilities (`--bot-net-abilities`), and their recharge timers before each.
    abilities: bool,
    recharge: [f32; 4],
    /// The knife pickup it takes as Jason, and his knives before.
    pickup: Option<(Entity, u32)>,
}

/// How long the bot waits for the server (real seconds) before giving up.
const NET_BOT_CONNECT_S: f32 = 120.0;

/// `--bot-net`: plays as a client only through its input, and checks what comes back. Its counselor walks (W, 3 s)
/// and must move on the server; she goes into combat stance (C) and must be in it there; the match's clock must run;
/// and Jason, whom the server runs, must move.
fn net_bot(
    mut bot: ResMut<NetBot>,
    time: Res<Time>,
    real: Res<Time<Real>>,
    mut keys: ResMut<ButtonInput<KeyCode>>,
    own: Query<(&NetBody, &NetCounselor), With<Controlled>>,
    jason: Query<&NetBody, With<NetJason>>,
    game: Res<GameMatch>,
    mut exit: MessageWriter<AppExit>,
    knives: Query<(Entity, &crate::net_world::NetVisual, &Transform)>,
) {
    bot.t += time.delta_secs();
    // A thrown knife the server sent: how far it goes here.
    for (e, v, xf) in &knives {
        if !matches!(v, crate::net_world::NetVisual::Knife { .. }) {
            continue;
        }
        // (From where Jason stood as it showed up: a knife can arrive here already stuck.)
        let jason_at = jason.single().ok().map_or(xf.translation, |b| Vec3::from_array(b.pos));
        match &mut bot.knife {
            Some((k, from, far)) if *k == e => *far = far.max(xf.translation.distance(*from)),
            Some(_) => {}
            None => {
                info!("bot: net: a knife at {} (Jason at {jason_at})", xf.translation);
                bot.knife = Some((e, jason_at, xf.translation.distance(jason_at)));
            }
        }
    }
    for k in [KeyCode::KeyW, KeyCode::KeyC] {
        keys.release(k);
    }
    // Where Jason is first seen (a player playing him may be done walking before her match starts).
    if bot.jason_from.is_none() {
        bot.jason_from = jason.single().ok().map(|b| Vec3::from_array(b.pos));
    }
    let Ok((body, counselor)) = own.single() else {
        if real.elapsed_secs() > NET_BOT_CONNECT_S {
            error!("bot: no counselor from the server");
            exit.write(AppExit::error());
        }
        return;
    };
    let at = Vec3::from_array(body.pos);
    if bot.step == 1 && (bot.t * 2.0).fract() < time.delta_secs() * 2.0 {
        info!("bot: net: walking at {at}, stance {}", counselor.stance);
    }
    let next = |bot: &mut NetBot| {
        bot.step += 1;
        bot.t = 0.0;
    };
    match bot.step {
        // Settled on the floor.
        0 if bot.t > 3.0 => {
            bot.from = at;
            info!("bot: net: her counselor at {at}");
            next(&mut bot);
        }
        1 if bot.t < 3.0 => keys.press(KeyCode::KeyW),
        1 => next(&mut bot),
        2 if bot.t > 1.0 => {
            let moved = (at - bot.from).with_y(0.0).length();
            info!("bot: net: walked {moved:.1} m on the server");
            if moved < 1.5 {
                bot.failed.push("walk");
            }
            next(&mut bot);
        }
        3 if bot.t < 0.3 => keys.press(KeyCode::KeyC),
        3 if bot.t > 1.5 => {
            info!("bot: net: stance {} on the server", counselor.stance);
            // (Refused where she stands, a slope or a fall: again, a few times.)
            if counselor.stance != Stance::Combat as u8 && bot.tries < 3 {
                bot.tries += 1;
                bot.t = 0.0;
                return;
            }
            if counselor.stance != Stance::Combat as u8 {
                bot.failed.push("combat stance");
            }
            next(&mut bot);
        }
        4 if bot.t < 0.3 => keys.press(KeyCode::KeyC),
        4 if bot.t > 1.5 => next(&mut bot),
        // The match runs: in progress, its clock counting down.
        5 if bot.t < 0.1 => bot.clock = Some(game.state.remaining_time),
        5 if bot.t > 3.0 => {
            let (state, from, now) = (game.state.state, bot.clock.unwrap_or(0), game.state.remaining_time);
            info!("bot: net: match {state:?}, clock {from} -> {now}");
            if state != MatchState::InProgress || now >= from {
                bot.failed.push("match clock");
            }
            next(&mut bot);
        }
        // Jason moves on the server (it hunts from the start with --jason-delay 0).
        6 => {
            let moved = jason.single().ok().zip(bot.jason_from).map(|(b, from)| Vec3::from_array(b.pos).distance(from));
            // (No Jason in the match: nothing to see.)
            if bot.jason_from.is_none() {
                info!("bot: net: no Jason");
                info!("bot: net: passed");
                exit.write(AppExit::Success);
                next(&mut bot);
            } else if moved.is_some_and(|m| m > 2.0) || bot.t > 30.0 {
                info!("bot: net: Jason moved {:.1} m", moved.unwrap_or(0.0));
                if !moved.is_some_and(|m| m > 2.0) {
                    bot.failed.push("Jason");
                }
                if let Some((_, _, far)) = bot.knife {
                    info!("bot: net: his knife went {far:.1} m from him here");
                    if far < 2.0 {
                        bot.failed.push("knife");
                    }
                }
                if bot.failed.is_empty() {
                    info!("bot: net: passed");
                    exit.write(AppExit::Success);
                } else {
                    error!("bot: net: failed {:?}", bot.failed);
                    exit.write(AppExit::error());
                }
                next(&mut bot);
            }
        }
        _ => {}
    }
}

/// The match as the clients see it: its state (MatchState's order), its timers, and Jason's score.
#[derive(Component, Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
pub struct NetMatch {
    pub state: u8,
    pub remaining: i32,
    pub elapsed: i32,
    pub police: i32,
    pub killer_awards: Vec<(String, i32)>,
    pub killer_popups: Vec<(String, String)>,
    pub killer_badges: Vec<String>,
    /// The match ends back in the lobby ([`relaunch`]).
    pub rematch: bool,
}

/// A counselor's match: her awards, her score texts (and their category), and what became of her.
#[derive(Component, Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
pub struct NetTally {
    pub awards: Vec<(String, i32)>,
    pub popups: Vec<(String, String)>,
    pub dead: bool,
    pub escaped: bool,
    pub taken_damage: bool,
    pub badges: Vec<String>,
}

const MATCH_STATES: [MatchState; 8] = [
    MatchState::WaitingToStart,
    MatchState::WaitingPreMatch,
    MatchState::PreMatchIntro,
    MatchState::InProgress,
    MatchState::WaitingPostMatchOutro,
    MatchState::PostMatchOutro,
    MatchState::WaitingPostMatch,
    MatchState::LeavingMap,
];

fn spawn_match(mut commands: Commands) {
    commands.spawn((NetMatch::default(), Replicate::to_clients(NetworkTarget::All), Name::new("match")));
}

fn awards(s: &f13_core::game_mode::PlayerScore) -> Vec<(String, i32)> {
    s.awards.iter().map(|a| (a.event.clone(), a.xp)).collect()
}

/// The match and each counselor's tally, for the clients.
fn send_match(game: Res<GameMatch>, mut matches: Query<&mut NetMatch>, mut tallies: Query<(Entity, &mut NetTally)>, rematch: Option<Res<Rematch>>) {
    if let Ok(n) = matches.single_mut() {
        let m = &game.state;
        set(n, NetMatch {
            state: MATCH_STATES.iter().position(|s| *s == m.state).unwrap_or(0) as u8,
            remaining: m.remaining_time,
            elapsed: m.elapsed_in_progress_time,
            police: m.police_time_remaining,
            killer_awards: awards(&game.killer_score),
            killer_popups: game.killer_popups.iter().map(|(t, c, _)| (t.clone(), c.clone())).collect(),
            killer_badges: game.killer_badges.clone(),
            rematch: rematch.is_some(),
        });
    }
    for (e, n) in &mut tallies {
        let Some(t) = game.counselors.get(&e) else { continue };
        set(n, NetTally {
            awards: awards(&t.score),
            popups: t.popups.iter().map(|(t, c, _)| (t.clone(), c.clone())).collect(),
            dead: t.record.dead,
            escaped: t.record.escaped,
            taken_damage: t.record.taken_damage,
            badges: t.badges.clone(),
        });
    }
}

/// The server's match here: its state and timers, the intro and outro as it enters them, each counselor's tally (her
/// new score texts shown from now), and leaving with it.
#[allow(clippy::too_many_arguments)]
fn receive_match(
    mut commands: Commands,
    time: Res<Time>,
    mut game: ResMut<GameMatch>,
    matches: Query<&NetMatch, Changed<NetMatch>>,
    tallies: Query<(&NetTally, &Twin)>,
    local: Query<Entity, (With<LocalPlayer>, With<Player>)>,
    intro: Option<Res<crate::cinematic::IntroEnabled>>,
    leave: Res<crate::match_flow::Leave>,
    mut exit: MessageWriter<AppExit>,
    me: Option<Res<crate::lobby::Me>>,
) {
    let now = time.elapsed_secs();
    let g = &mut *game;
    g.local = local.single().ok();
    if let Ok(n) = matches.single() {
        let state = MATCH_STATES.get(n.state as usize).copied().unwrap_or(MatchState::WaitingToStart);
        if state != g.state.state {
            info!("net: match {:?}", state);
            match state {
                MatchState::PreMatchIntro if intro.is_some() => commands.insert_resource(crate::cinematic::PlayCinematic(crate::cinematic::INTRO)),
                MatchState::PostMatchOutro if intro.is_some() => commands.insert_resource(crate::cinematic::PlayCinematic(crate::cinematic::OUTRO)),
                MatchState::LeavingMap if leave.0 => {
                    // Back to the lobby with the server (the same netcode id, for its last-Jason rule).
                    if n.rematch {
                        let args: Vec<String> = std::env::args().skip(1).collect();
                        let id = me.as_ref().map(|m| m.id);
                        match relaunch(&args, &["--net-id"], &id.map(|id| ("--net-id", id.to_string())).into_iter().collect::<Vec<_>>()) {
                            Ok(_) => info!("match: back to the lobby"),
                            Err(e) => warn!("match: can't start again for the lobby: {e}"),
                        }
                    }
                    info!("match: leaving");
                    exit.write(AppExit::Success);
                }
                _ => {}
            }
        }
        g.state.state = state;
        g.state.remaining_time = n.remaining;
        g.state.elapsed_in_progress_time = n.elapsed;
        g.state.police_time_remaining = n.police;
        g.killer_score.awards = n.killer_awards.iter().map(|(e, xp)| f13_core::game_mode::Award { event: e.clone(), xp: *xp }).collect();
        g.killer_badges = n.killer_badges.clone();
        let seen = g.killer_popups.len();
        g.killer_popups.extend(n.killer_popups.iter().skip(seen).map(|(t, c)| (t.clone(), c.clone(), now)));
    }
    for (n, twin) in &tallies {
        let t = g.counselors.entry(twin.local).or_default();
        t.score.awards = n.awards.iter().map(|(e, xp)| f13_core::game_mode::Award { event: e.clone(), xp: *xp }).collect();
        let seen = t.popups.len();
        t.popups.extend(n.popups.iter().skip(seen).map(|(t, c)| (t.clone(), c.clone(), now)));
        t.record = f13_core::game_mode::CounselorRecord { dead: n.dead, escaped: n.escaped, taken_damage: n.taken_damage };
        t.badges = n.badges.clone();
    }
}

/// The net bot in the lobby: asks for its preference, then readies up once there's another player.
fn net_bot_lobby(mut bot: ResMut<NetBot>, mut keys: ResMut<ButtonInput<KeyCode>>, lobby: Query<&crate::lobby::NetLobby>) {
    for k in [KeyCode::Digit1, KeyCode::Digit2, KeyCode::Digit3, KeyCode::Enter] {
        keys.release(k);
    }
    let Ok(l) = lobby.single() else { return };
    if !l.open {
        return;
    }
    match bot.lobby {
        0 => {
            keys.press(match bot.prefer {
                crate::lobby::Preference::Counselor => KeyCode::Digit1,
                crate::lobby::Preference::NoPreference => KeyCode::Digit2,
                crate::lobby::Preference::Killer => KeyCode::Digit3,
            });
            info!("bot: net: asking the lobby for {:?}", bot.prefer);
            bot.lobby = 1;
        }
        // Ready once another player is in (a match needs a counselor and Jason).
        1 if l.members.len() >= 2 => {
            keys.press(KeyCode::Enter);
            info!("bot: net: ready");
            bot.lobby = 2;
        }
        _ => {}
    }
}

/// A lobby's playing server, as Jason: he walks a little (W, 3 s), so a client's counselor bot sees him move, then
/// opens his map.
fn host_bot_jason(
    mut bot: ResMut<NetBot>,
    time: Res<Time>,
    mut controls: ResMut<crate::jason_player::JasonControls>,
    mut map: ResMut<crate::jason_player::MorphMap>,
    mut camera: Single<&mut crate::player::PlayerCamera>,
) {
    bot.t += time.delta_secs();
    match bot.step {
        0 if bot.t > 3.0 => {
            info!("bot: net: the host's Jason walks");
            controls.keys.press(KeyCode::KeyW);
            bot.step = 1;
            bot.t = 0.0;
        }
        // A knife (middle mouse held aims, let go throws), for the client to see fly.
        1 if bot.t > 3.0 => {
            controls.keys.release(KeyCode::KeyW);
            info!("bot: net: the host's Jason aims a knife");
            // Back the way he came (he walked up to his shack's wall), level.
            camera.yaw += std::f32::consts::PI;
            camera.pitch = 0.0;
            controls.press_knife();
            bot.step = 2;
            bot.t = 0.0;
        }
        2 if bot.t < 1.5 => {}
        2 => {
            info!("bot: net: the host's Jason throws it");
            controls.release_knife();
            bot.step = 3;
            bot.t = 0.0;
        }
        3 if bot.t > 2.0 => {
            // His map left open (M), to be seen on the host's screen.
            info!("bot: net: the host's Jason opens his map");
            map.open = true;
            bot.step = 4;
        }
        _ => {}
    }
}

/// The net bot as Jason: he walks (W, 3 s) through his remote controls and must move on the server; his abilities
/// (the server's) must show unlocked here. With `--bot-net-abilities`: every ability made ready (a bot hook), he
/// Shifts (3), Stalks (4), Senses (2) and Morphs (1, then a click on his map's centre), each seen in the server's
/// replicated Jason.
#[allow(clippy::type_complexity)]
fn net_bot_jason(
    mut bot: ResMut<NetBot>,
    time: Res<Time>,
    mut controls: ResMut<crate::jason_player::JasonControls>,
    own: Query<(&NetBody, &NetJason), With<Controlled>>,
    know: Query<&crate::senses::Knowledge, With<crate::jason::Jason>>,
    (mut map, window): (ResMut<crate::jason_player::MorphMap>, Query<&bevy::window::Window, With<bevy::window::PrimaryWindow>>),
    pickups: Query<(Entity, &crate::props::KnifePickup, &GlobalTransform, Has<crate::props::PickedUp>)>,
    mut exit: MessageWriter<AppExit>,
) {
    bot.t += time.delta_secs();
    let Ok((body, nj)) = own.single() else { return };
    let at = Vec3::from_array(body.pos);
    let (yaw, pitch, view) = (controls.yaw, controls.pitch, controls.view);
    match bot.step {
        0 if bot.t > 3.0 => {
            info!("bot: net: playing Jason at {at}");
            bot.from = at;
            bot.step = 1;
            bot.t = 0.0;
        }
        1 if bot.t < 3.0 => controls.apply_remote(1, 0, yaw, pitch, view),
        1 => {
            controls.apply_remote(0, 0, yaw, pitch, view);
            bot.step = 2;
            bot.t = 0.0;
        }
        2 if bot.t > 1.0 => {
            let moved = (at - bot.from).with_y(0.0).length();
            info!("bot: net: Jason walked {moved:.1} m on the server");
            let unlocked = know.iter().next().map_or(0, |k| k.abilities.unlocked.iter().filter(|u| **u).count());
            info!("bot: net: {unlocked} of his abilities unlocked here");
            if unlocked == 0 {
                error!("bot: net: failed [\"abilities\"]");
                exit.write(AppExit::error());
            } else if moved < 1.5 {
                error!("bot: net: failed [\"Jason walk\"]");
                exit.write(AppExit::error());
            } else if bot.abilities {
                crate::bot::setup(crate::bot::BotSetup::Abilities);
                bot.step = 10;
                bot.t = 0.0;
                return;
            } else {
                info!("bot: net: passed");
                exit.write(AppExit::Success);
            }
            bot.step = 3;
        }
        // His abilities: each pressed (held a moment, then let go), then seen on the server.
        10.. => {
            // (Keys' bits: NET_KEYS' order, 1 at bit 8.)
            // (4 again while Stalk is on cancels it: KLR_Stalk_PC.)
            const STEPS: [(&str, u32); 5] = [("Shift", 1 << 10), ("Stalk", 1 << 11), ("Unstalk", 1 << 11), ("Sense", 1 << 9), ("Morph", 1 << 8)];
            let i = (bot.step - 10) as usize;
            // Then a knife pickup (E at it): taken on the server, hidden here.
            if i == STEPS.len() {
                if bot.t < 2.0 {
                    if nj.state != "Hunting" {
                        bot.t = 0.0;
                    }
                    return;
                }
                let Some((pe, before)) = bot.pickup.or_else(|| {
                    let (pe, k, g, _) = pickups.iter().filter(|p| !p.3).min_by(|a, b| a.2.translation().distance(at).total_cmp(&b.2.translation().distance(at)))?;
                    // Beside its AlignmentSpecialMove spot, facing the knives (as the knife demo stands him).
                    let spot = g.compute_transform() * k.spot;
                    let knives_at = g.transform_point(k.interact);
                    let off = (spot.translation - knives_at).with_y(0.0).normalize_or(Vec3::X) * 0.6;
                    let stand = Vec3::new(spot.translation.x + off.x, spot.translation.y, spot.translation.z + off.z);
                    crate::bot::setup(crate::bot::BotSetup::PlaceJason(stand.to_array()));
                    info!("bot: net: Jason to the knives at {knives_at}");
                    Some((pe, nj.knives))
                }) else {
                    bot.failed.push("knives");
                    bot.step += 1;
                    return;
                };
                bot.pickup = Some((pe, before));
                let Ok((_, k, g, taken)) = pickups.get(pe) else { return };
                let to = g.transform_point(k.interact) - (at + Vec3::Y * 0.88);
                let (ky, kp) = ((-to.x).atan2(-to.z), to.y.atan2(Vec2::new(to.x, to.z).length()));
                controls.yaw = ky;
                controls.pitch = kp;
                if taken && nj.knives > before {
                    info!("bot: net: the knife pickup taken (hidden here), {} knives", nj.knives);
                    bot.step += 1;
                    bot.t = 0.0;
                    controls.apply_remote(0, 0, ky, kp, view);
                } else if bot.t > 20.0 {
                    warn!("bot: net: the knife pickup: taken here {taken}, knives {} (were {before})", nj.knives);
                    bot.failed.push("knives");
                    bot.step += 1;
                    bot.t = 0.0;
                } else if bot.t > 3.0 {
                    controls.apply_remote(if (bot.t * 2.0) as u32 % 2 == 0 { 1 << 7 } else { 0 }, 0, ky, kp, view);
                }
                return;
            }
            let Some(&(name, key)) = STEPS.get(i) else {
                if bot.failed.is_empty() {
                    info!("bot: net: passed");
                    exit.write(AppExit::Success);
                } else {
                    error!("bot: net: failed {:?}", bot.failed);
                    exit.write(AppExit::error());
                }
                bot.step = 3;
                return;
            };
            // (Each once he's back to hunting: a press mid-Shift does nothing.)
            if bot.t < 2.0 {
                bot.recharge = nj.recharge;
                if nj.state != "Hunting" {
                    bot.t = 0.0;
                }
                return;
            }
            let held = bot.t < 2.3;
            controls.apply_remote(if held { key } else { 0 }, 0, yaw, pitch, view);
            // The Morph: once his map's out here (the server opened it), a click on its centre picks the place.
            if name == "Morph" && !held && map.morph && bot.tries == 0 {
                if let Ok(w) = window.single() {
                    info!("bot: net: his Morph map is out here: a click on it");
                    map.click = Some(Vec2::new(w.width(), w.height()) / 2.0);
                    bot.tries = 1;
                }
            }
            // (A recharge timer drops as the ability's used.)
            let spent = |a: f13_core::killer_abilities::Ability| nj.recharge[a as usize] < bot.recharge[a as usize];
            use f13_core::killer_abilities::Ability;
            let seen = match name {
                "Shift" => nj.shifting || spent(Ability::Shift),
                "Stalk" => nj.stalking || spent(Ability::Stalk),
                "Unstalk" => !nj.stalking,
                // (And on here: its highlights are worked out on this machine.)
                "Sense" => spent(Ability::Sense) && know.iter().next().is_some_and(|k| k.sensing(time.elapsed_secs())),
                _ => nj.state == "Morphing" || spent(Ability::Morph),
            };
            if seen {
                info!("bot: net: {name} on the server (state {}, recharge {:?})", nj.state, nj.recharge);
                bot.step += 1;
                bot.t = 0.0;
                controls.apply_remote(0, 0, yaw, pitch, view);
            } else if bot.t > 12.0 {
                warn!("bot: net: no {name} on the server (state {}, recharge {:?})", nj.state, nj.recharge);
                bot.failed.push(name);
                bot.step += 1;
                bot.t = 0.0;
                controls.apply_remote(0, 0, yaw, pitch, view);
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn twins_are_shown_between_the_bodies_around_then() {
        let s = Snapshots([(0.0, Vec3::ZERO, 0.0), (0.1, Vec3::X, 0.2), (0.2, Vec3::X * 10.0, 0.4)].into());
        let (p, y) = sample(&s, 0.05).unwrap();
        assert!((p.x - 0.5).abs() < 1e-4 && (y - 0.1).abs() < 1e-4);
        // Across a jump: no blend.
        assert_eq!(sample(&s, 0.15).unwrap().0, Vec3::X * 10.0);
        // Before the first and after the last: the ends.
        assert_eq!(sample(&s, -1.0).unwrap().0, Vec3::ZERO);
        assert_eq!(sample(&s, 5.0).unwrap().0, Vec3::X * 10.0);
        // Yaw the short way round.
        let w = Snapshots([(0.0, Vec3::ZERO, 3.0), (1.0, Vec3::ZERO, -3.0)].into());
        let (_, y) = sample(&w, 0.5).unwrap();
        assert!((y - (3.0 + (std::f32::consts::TAU - 6.0) / 2.0)).abs() < 1e-3, "{y}");
    }

    #[test]
    fn a_rematch_starts_again_with_its_flags() {
        let args: Vec<String> = ["--server", "5013", "--seed", "7", "--lobby", "--last-jason", "3"].iter().map(|s| s.to_string()).collect();
        let out = relaunch_args(&args, &["--seed", "--last-jason"], &[("--last-jason", "9".into())]);
        assert_eq!(out, ["--server", "5013", "--lobby", "--last-jason", "9"]);
    }

    #[test]
    fn a_tap_between_two_inputs_still_counts() {
        let e = 1 << crate::controls::Button::Interact as u32;
        let mut client = Counts::default();
        let before = client.presses;
        // Pressed and released between two of the server's frames: the newest input holds nothing.
        client.add(e, 0);
        client.add(0, e);
        let c = Controls::default().next_with(0, counted(&before, &client.presses), counted(&before, &client.releases), 0.0, 0.0);
        assert!(c.just_pressed(crate::controls::Button::Interact) && c.just_released(crate::controls::Button::Interact));
        assert!(!c.pressed(crate::controls::Button::Interact));
        // A frame with no input doesn't press it again.
        assert!(!c.held_still().just_pressed(crate::controls::Button::Interact));
    }

    #[test]
    fn bodies_keep_the_servers_spacing() {
        let mut s = Snapshots::default();
        s.push(1.0, Vec3::ZERO, 0.0);
        // A late arrival older than the newest is dropped.
        s.push(2.0, Vec3::X, 0.0);
        s.push(1.5, Vec3::Y, 0.0);
        assert_eq!(s.0.len(), 3);
        // After a second still, it held its place until just before it moved.
        assert_eq!(sample(&s, 1.5).unwrap().0, Vec3::ZERO);
        assert_eq!(sample(&s, 2.0 - STILL_GAP_S).unwrap().0, Vec3::ZERO);
        assert!((sample(&s, 2.0 - STILL_GAP_S / 2.0).unwrap().0.x - 0.5).abs() < 1e-3);
    }
}
