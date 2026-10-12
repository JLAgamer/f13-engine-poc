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
//! - twins smooth toward the newest body rather than interpolating between snapshots;
//! - the world's doors, windows, items and cars don't replicate yet.

use crate::character::{PlayAction, RestartAction, SeekAction};
use crate::controls::{Controls, LocalPlayer};
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
/// 3 s, which a level still streaming in on either side outlasts).
const TIMEOUT_SECS: i32 = 30;
/// The netcode tick (the game's fixed step).
const TICK: Duration = Duration::from_micros(15_625);

/// What this process is on the network.
#[derive(Clone, Copy, Debug)]
pub enum Role {
    Server(u16),
    Client(SocketAddr),
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
}

/// What Jason's animation reads of his state.
#[derive(Component, Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
pub struct NetJason {
    pub velocity: [f32; 2],
    pub combat: bool,
    pub blocking: bool,
    pub done: bool,
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
pub fn simulating(client: Option<Res<NetClient>>) -> bool {
    client.is_none()
}

/// On the server: the link of the player who plays this counselor.
#[derive(Component)]
struct PlayedBy(Entity);

/// The counselor starts not yet taken (the server hands them to joining players in order).
#[derive(Resource, Default)]
pub struct FreeStarts(pub Vec<Vec3>);

pub struct NetPlugin {
    pub role: Role,
    /// A client: the network test bot ([`net_bot`]).
    pub bot: bool,
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
                app
                    .add_systems(PostUpdate, (replicate_bodies, update_bodies).chain().before(ReplicationSystems::Send));
            }
            Role::Client(addr) => {
                app.add_plugins(client::ClientPlugins { tick_duration: TICK });
                protocol(app);
                app.insert_resource(NetClient)
                    .insert_resource(ServerAddr(addr))
                    .add_systems(Update, connect)
                    .add_observer(body_gone)
                    .init_resource::<Twins>()
                    .add_systems(Update, (send_input, (make_twins, mirror).chain().in_set(Mirrored).before(crate::character::animate)));
                crate::net_world::client(app);
                if self.bot {
                    app.init_resource::<NetBot>().add_systems(Update, net_bot.before(send_input));
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
    crate::net_world::protocol(app);
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
) {
    let Ok(id) = links.get(joined.entity) else { return };
    if starts.0.is_empty() {
        warn!("net: player {:?} joined with no counselor start left", id.0);
        return;
    }
    let at = starts.0.remove(0);
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
        commands.entity(e).insert((NetBody { id: e.to_bits(), pos: [0.0; 3], yaw: 0.0, kind: BodyKind::Counselor }, NetCounselor::default(), NetAction::default(), Replicate::to_clients(NetworkTarget::All)));
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
    mut counselors: Query<(&mut NetCounselor, &Player)>,
    mut jasons: Query<(&mut NetJason, &crate::jason::Jason)>,
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
    for (n, p) in &mut counselors {
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
        });
    }
    for (n, j) in &mut jasons {
        set(n, NetJason { velocity: j.velocity.to_array(), combat: j.combat, blocking: j.blocking, done: j.state == crate::jason::JasonState::Done });
    }
}

/// Once the level is in (its first frames hitch while it streams), the connection.
fn connect(mut commands: Commands, server: Res<ServerAddr>, level: Res<crate::Level>, assets: Res<AssetServer>, mut done: Local<bool>) {
    if *done || level.scenes.is_empty() || !level.scenes.iter().all(|h| assets.is_loaded_with_dependencies(h.id())) {
        return;
    }
    *done = true;
    // A client id from the clock (PLACEHOLDER: a backend would hand it out with the token).
    let id = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(1, |d| d.as_nanos() as u64);
    let auth = Authentication::Manual { server_addr: server.0, client_id: id, private_key: DEV_KEY, protocol_id: PROTOCOL_ID };
    let config = lightyear::prelude::client::NetcodeConfig { client_timeout_secs: TIMEOUT_SECS, ..default() };
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
        commands.entity(e).insert(Twin { local: twin, restarts: 0, seeks: 0 });
        index.0.insert(b.id, twin);
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

/// How fast a twin closes on the newest body (1/s; PLACEHOLDER for interpolating between snapshots), and how far
/// off it snaps instead (m).
const FOLLOW_RATE: f32 = 25.0;
const SNAP_M: f32 = 3.0;

/// Each twin takes its character's state.
#[allow(clippy::type_complexity)]
fn mirror(
    mut commands: Commands,
    time: Res<Time>,
    mut bodies: Query<(&NetBody, &NetAction, Option<&NetCounselor>, Option<&NetJason>, &mut Twin)>,
    mut twins: Query<(&mut Transform, Option<&mut Player>, Option<&mut crate::jason::Jason>, Option<&PlayAction>, Has<crate::items::Blocking>)>,
) {
    let k = 1.0 - (-FOLLOW_RATE * time.delta_secs()).exp();
    for (b, a, nc, nj, mut twin) in &mut bodies {
        let Ok((mut xf, player, jason, action, blocking)) = twins.get_mut(twin.local) else { continue };
        let to = Vec3::from_array(b.pos);
        let rot = Quat::from_rotation_y(b.yaw);
        if xf.translation.distance(to) > SNAP_M {
            xf.translation = to;
            xf.rotation = rot;
        } else {
            xf.translation = xf.translation.lerp(to, k);
            xf.rotation = xf.rotation.slerp(rot, k);
        }
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
        }
        if let (Some(mut j), Some(n)) = (jason, nj) {
            j.velocity = Vec2::from_array(n.velocity);
            j.combat = n.combat;
            j.blocking = n.blocking;
            if n.done {
                j.state = crate::jason::JasonState::Done;
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

/// The network test bot's state (`--bot-net` on a client).
#[derive(Resource, Default)]
struct NetBot {
    step: u8,
    t: f32,
    from: Vec3,
    jason_from: Option<Vec3>,
    failed: Vec<&'static str>,
}

/// How long the bot waits for the server (real seconds) before giving up.
const NET_BOT_CONNECT_S: f32 = 120.0;

/// `--bot-net`: plays as a client only through its input, and checks what comes back. Its counselor walks (W, 3 s)
/// and must move on the server; she goes into combat stance (C) and must be in it there; and Jason, whom the server
/// runs, must move.
fn net_bot(
    mut bot: ResMut<NetBot>,
    time: Res<Time>,
    real: Res<Time<Real>>,
    mut keys: ResMut<ButtonInput<KeyCode>>,
    own: Query<(&NetBody, &NetCounselor), With<Controlled>>,
    jason: Query<&NetBody, With<NetJason>>,
    mut exit: MessageWriter<AppExit>,
) {
    bot.t += time.delta_secs();
    for k in [KeyCode::KeyW, KeyCode::KeyC] {
        keys.release(k);
    }
    let Ok((body, counselor)) = own.single() else {
        if real.elapsed_secs() > NET_BOT_CONNECT_S {
            error!("bot: no counselor from the server");
            exit.write(AppExit::error());
        }
        return;
    };
    let at = Vec3::from_array(body.pos);
    if bot.jason_from.is_none() {
        bot.jason_from = jason.single().ok().map(|b| Vec3::from_array(b.pos));
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
            if counselor.stance != Stance::Combat as u8 {
                bot.failed.push("combat stance");
            }
            next(&mut bot);
        }
        4 if bot.t < 0.3 => keys.press(KeyCode::KeyC),
        4 if bot.t > 1.5 => next(&mut bot),
        // Jason moves on the server (it hunts from the start with --jason-delay 0).
        5 => {
            let moved = jason.single().ok().zip(bot.jason_from).map(|(b, from)| Vec3::from_array(b.pos).distance(from));
            if moved.is_some_and(|m| m > 2.0) || bot.t > 30.0 {
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
