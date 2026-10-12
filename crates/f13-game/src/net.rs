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
    let Ok(socket) = std::net::UdpSocket::bind((Ipv4Addr::UNSPECIFIED, port + 1)) else {
        eprintln!("net: can't answer world questions on UDP {}", port + 1);
        return;
    };
    let answer = format!("{WORLD_TELL}{} {} {}", info.seed, info.layout, info.rain);
    std::thread::spawn(move || {
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

/// A player's input for one tick: the buttons held ([`Controls::held`]) and the control rotation.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub struct InputMsg {
    pub held: u32,
    pub yaw: f32,
    pub pitch: f32,
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
    /// His state's name (a Jason player's HUD).
    pub state: String,
    /// His abilities (f13-core KillerAbilities): unlocked, recharge timers, the unlock timer, rage.
    pub unlocked: [bool; 4],
    pub recharge: [f32; 4],
    pub unlock_timer: f32,
    pub rage: (bool, f32),
    /// The kill camera running (kill_camera.rs): its kill, origin (translation, rotation) and whether at the root.
    pub kill_cam: Option<(String, [f32; 7], bool)>,
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
    pub prefer: crate::lobby::Preference,
    /// A server: the lobby first (lobby.rs), and whether the server plays no one.
    pub lobby: bool,
    pub dedicated: bool,
    /// This player's name in the lobby.
    pub name: String,
}

impl Plugin for NetPlugin {
    fn build(&self, app: &mut App) {
        match self.role {
            Role::Server(port) => {
                app.add_plugins(server::ServerPlugins { tick_duration: TICK });
                protocol(app);
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
                    .add_systems(Update, apply_inputs);
                crate::net_world::server(app);
                crate::voice::server(app, self.dedicated);
                if self.lobby {
                    crate::lobby::server(app, self.dedicated, self.name.clone());
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
                    .add_systems(Update, connect)
                    .add_observer(body_gone)
                    .init_resource::<Twins>()
                    .add_systems(Update, (send_input, (make_twins, record_bodies, mirror, hide_jason).chain().in_set(Mirrored).before(crate::character::animate)));
                app.add_systems(Update, receive_match.after(Mirrored));
                crate::net_world::client(app);
                crate::lobby::client(app);
                crate::voice::client(app);
                crate::lobby::ui(app);
                app.insert_resource(ClientName(self.name.clone()));
                if self.bot {
                    app.insert_resource(NetBot { prefer: self.prefer, ..default() })
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
    app.component::<NetBody>().replicate();
    app.component::<NetCounselor>().replicate();
    app.component::<NetJason>().replicate();
    app.component::<NetAction>().replicate();
    app.component::<NetMatch>().replicate();
    app.component::<NetTally>().replicate();
    crate::net_world::protocol(app);
    crate::lobby::protocol(app);
    crate::voice::protocol(app);
}

#[derive(Resource)]
struct ServerPort(u16);

#[derive(Resource)]
struct ServerAddr(SocketAddr);

fn start_server(mut commands: Commands, port: Res<ServerPort>) {
    let config = lightyear::prelude::server::NetcodeConfig { protocol_id: PROTOCOL_ID, private_key: DEV_KEY, client_timeout_secs: TIMEOUT_SECS, ..default() };
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
    let counselor = crate::player::spawn_counselor(&mut commands, &assets, character.as_deref(), &tuning, at);
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

/// Each player's newest input into her counselor's controls.
fn apply_inputs(mut links: Query<(Entity, &mut MessageReceiver<InputMsg>)>, mut counselors: Query<(&PlayedBy, &mut Controls)>) {
    for (link, mut receiver) in &mut links {
        let Some(input) = receiver.receive().last() else { continue };
        for (by, mut c) in &mut counselors {
            if by.0 == link {
                *c = c.next(input.held, input.yaw, input.pitch);
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
        commands.entity(e).insert((NetBody { id: e.to_bits(), pos: [0.0; 3], yaw: 0.0, kind: BodyKind::Counselor }, NetCounselor::default(), NetTally::default(), NetAction::default(), Replicate::to_clients(NetworkTarget::All)));
    }
    for e in &jasons {
        commands.entity(e).insert((NetBody { id: e.to_bits(), pos: [0.0; 3], yaw: 0.0, kind: BodyKind::Jason }, NetJason::default(), NetAction::default(), Replicate::to_clients(NetworkTarget::All)));
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
    mut counselors: Query<(&mut NetCounselor, &Player, Option<&Name>, Option<&Controls>)>,
    mut jasons: Query<(&mut NetJason, &crate::jason::Jason, Option<&crate::senses::Knowledge>, Option<&crate::kill_camera::KillCam>)>,
    world_ids: Query<&crate::net_world::WorldId>,
) {
    for (mut b, xf, mut a, action, blocking) in &mut bodies {
        let pos = xf.translation.to_array();
        let yaw = xf.rotation.to_euler(EulerRot::YXZ).0;
        if b.pos != pos || b.yaw != yaw {
            b.pos = pos;
            b.yaw = yaw;
        }
        let take = action.map(|p| p.0.clone());
        if a.take != take || a.blocking != blocking {
            a.take = take;
            a.blocking = blocking;
        }
    }
    for (n, p, name, controls) in &mut counselors {
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
            state: j.state_name(),
            unlocked: a.map_or([false; 4], |a| a.unlocked),
            recharge: a.map_or([0.0; 4], |a| a.recharge_timer),
            unlock_timer: a.map_or(0.0, |a| a.unlock_timer),
            rage: a.map_or((false, 0.0), |a| (a.rage_active, a.rage_timer)),
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

fn connect(mut commands: Commands, server: Res<ServerAddr>, level: Res<crate::Level>, assets: Res<AssetServer>, name: Res<ClientName>, mut done: Local<bool>) {
    if *done || level.scenes.is_empty() || !level.scenes.iter().all(|h| assets.is_loaded_with_dependencies(h.id())) {
        return;
    }
    *done = true;
    // A client id from the clock (PLACEHOLDER: a backend would hand it out with the token).
    let id = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(1, |d| d.as_nanos() as u64);
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
fn send_input(local: Query<&Controls, With<LocalPlayer>>, mut senders: Query<&mut MessageSender<InputMsg>, With<Connected>>) {
    let Ok(c) = local.single() else { return };
    for mut s in &mut senders {
        s.send::<InputChannel>(InputMsg { held: c.held(), yaw: c.yaw, pitch: c.pitch });
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
    bodies: Query<(Entity, &NetBody, Has<Controlled>), Without<Twin>>,
    local: Query<Entity, (With<LocalPlayer>, With<Player>)>,
    jason: Query<Entity, With<crate::jason::Jason>>,
    mut index: ResMut<Twins>,
) {
    for (e, b, own) in &bodies {
        let at = Vec3::from_array(b.pos);
        let twin = match (b.kind, own) {
            (BodyKind::Counselor, true) => local.single().ok(),
            (BodyKind::Counselor, false) => Some(crate::player::spawn_counselor(&mut commands, &assets, counselor.as_deref(), &tuning, at)),
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

/// On a client: a character's bodies as they came in (real time received, place, yaw).
#[derive(Component, Default)]
struct Snapshots(std::collections::VecDeque<(f32, Vec3, f32)>);

/// Each body received is kept, with when it came.
fn record_bodies(mut commands: Commands, real: Res<Time<Real>>, mut bodies: Query<(Entity, &NetBody, Option<&mut Snapshots>), Changed<NetBody>>) {
    let now = real.elapsed_secs();
    for (e, b, snaps) in &mut bodies {
        let s = (now, Vec3::from_array(b.pos), b.yaw);
        match snaps {
            Some(mut q) => {
                q.0.push_back(s);
                while q.0.front().is_some_and(|f| now - f.0 > SNAPSHOTS_KEPT_S) && q.0.len() > 2 {
                    q.0.pop_front();
                }
            }
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
    real: Res<Time<Real>>,
    mut bodies: Query<(&NetBody, &NetAction, Option<&NetCounselor>, Option<&NetJason>, &mut Twin, Option<&Snapshots>)>,
    mut twins: Query<(&mut Transform, Option<&mut Player>, Option<&mut crate::jason::Jason>, Option<&PlayAction>, Has<crate::items::Blocking>, (Option<&Name>, Option<&mut Controls>, Has<LocalPlayer>, Option<&mut crate::senses::Knowledge>))>,
    actors: Res<crate::net_world::Actors>,
    cams: Query<&crate::kill_camera::KillCam>,
) {
    let shown_at = real.elapsed_secs() - INTERP_DELAY_S;
    for (b, a, nc, nj, mut twin, snaps) in &mut bodies {
        let Ok((mut xf, player, jason, action, blocking, (name, controls, local, know))) = twins.get_mut(twin.local) else { continue };
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
            j.traps = n.traps;
            j.remote = Some(crate::jason::RemoteView {
                shifting: n.shifting,
                stalking: n.stalking,
                holding: n.holding,
                knife_aim: n.knife_aim,
                hidden: n.hidden,
                door: n.door.and_then(|(id, m)| Some((actors.get(id)?, m))),
            });
            j.remote_state = n.state.clone();
            let key = n.kill_cam.as_ref().map(|(k, ..)| k.clone());
            if key != twin.kill {
                match &n.kill_cam {
                    Some((k, o, root)) => {
                        let origin = Transform::from_xyz(o[0], o[1], o[2]).with_rotation(Quat::from_xyzw(o[3], o[4], o[5], o[6]));
                        commands.entity(twin.local).insert(crate::kill_camera::KillCam::remote(k, origin, *root));
                    }
                    None if cams.contains(twin.local) => {
                        commands.entity(twin.local).remove::<crate::kill_camera::KillCam>();
                    }
                    None => {}
                }
                twin.kill = key;
            }
            if let Some(mut k) = know {
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
    /// What it asks the lobby for, and how far it got there (0 waiting, 1 asked, 2 ready).
    prefer: crate::lobby::Preference,
    lobby: u8,
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
) {
    bot.t += time.delta_secs();
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
fn send_match(game: Res<GameMatch>, mut matches: Query<&mut NetMatch>, mut tallies: Query<(Entity, &mut NetTally)>) {
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
fn host_bot_jason(mut bot: ResMut<NetBot>, time: Res<Time>, mut controls: ResMut<crate::jason_player::JasonControls>, mut map: ResMut<crate::jason_player::MorphMap>) {
    bot.t += time.delta_secs();
    match bot.step {
        0 if bot.t > 3.0 => {
            info!("bot: net: the host's Jason walks");
            controls.keys.press(KeyCode::KeyW);
            bot.step = 1;
            bot.t = 0.0;
        }
        1 if bot.t > 3.0 => {
            controls.keys.release(KeyCode::KeyW);
            // His map left open (M), to be seen on the host's screen.
            info!("bot: net: the host's Jason opens his map");
            map.open = true;
            bot.step = 2;
        }
        _ => {}
    }
}

/// The net bot as Jason: he walks (W, 3 s) through his remote controls and must move on the server; his abilities
/// (the server's) must show unlocked here.
fn net_bot_jason(
    mut bot: ResMut<NetBot>,
    time: Res<Time>,
    mut controls: ResMut<crate::jason_player::JasonControls>,
    own: Query<&NetBody, (With<NetJason>, With<Controlled>)>,
    know: Query<&crate::senses::Knowledge, With<crate::jason::Jason>>,
    mut exit: MessageWriter<AppExit>,
) {
    bot.t += time.delta_secs();
    let Ok(body) = own.single() else { return };
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
            } else {
                info!("bot: net: passed");
                exit.write(AppExit::Success);
            }
            bot.step = 3;
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
}
