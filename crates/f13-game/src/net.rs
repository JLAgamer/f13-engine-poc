//! The netcode (Milestone 3; lightyear 0.30 over UDP with netcode.io): `--server <port>` runs the match and takes
//! players; `--connect <host:port>` joins one.
//!
//! The server is the authority: each joining player gets a counselor at the next counselor start, her [`Controls`]
//! from the player's input messages (the buttons held and the control rotation, every tick), and every character's
//! body ([`NetBody`]: where it is and which way it faces) goes out to all players. A client sends its input and shows
//! the characters it receives; its own counselor follows the server's.
//!
//! PLACEHOLDER: the development key and protocol id (no matchmaking backend issues tokens yet); the client still
//! simulates the match itself apart from its counselor's movement (the split of simulation from presentation is the
//! next step), and only bodies replicate (no animation state, health, items or the world's doors and windows yet).

use crate::controls::{Controls, LocalPlayer};
use crate::player::Player;
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
    pub pos: [f32; 3],
    pub yaw: f32,
    pub kind: BodyKind,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub enum BodyKind {
    Counselor,
    Jason,
}

/// On the server: the link of the player who plays this counselor.
#[derive(Component)]
struct PlayedBy(Entity);

/// The counselor starts not yet taken (the server hands them to joining players in order).
#[derive(Resource, Default)]
pub struct FreeStarts(pub Vec<Vec3>);

pub struct NetPlugin(pub Role);

impl Plugin for NetPlugin {
    fn build(&self, app: &mut App) {
        match self.0 {
            Role::Server(port) => {
                app.add_plugins(server::ServerPlugins { tick_duration: TICK });
                protocol(app);
                app.init_resource::<FreeStarts>()
                    .insert_resource(ServerPort(port))
                    .add_systems(Startup, start_server)
                    .add_observer(new_link)
                    .add_observer(player_joined)
                    .add_systems(Update, apply_inputs)
                    .add_systems(PostUpdate, (replicate_bodies, update_bodies).chain());
            }
            Role::Client(addr) => {
                app.add_plugins(client::ClientPlugins { tick_duration: TICK });
                protocol(app);
                app.insert_resource(NetClient)
                    .insert_resource(ServerAddr(addr))
                    .add_systems(Update, connect)
                    .add_systems(Update, (send_input, show_bodies, follow_own_body));
            }
        }
    }
}

/// What both sides agree on: the input message and its channel, and the replicated body.
fn protocol(app: &mut App) {
    app.add_channel::<InputChannel>(ChannelSettings { mode: ChannelMode::SequencedUnreliable, ..default() }).add_direction(NetworkDirection::ClientToServer);
    app.register_message::<InputMsg>().add_direction(NetworkDirection::ClientToServer);
    app.component::<NetBody>().replicate();
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
    for (e, kind) in counselors.iter().map(|e| (e, BodyKind::Counselor)).chain(jasons.iter().map(|e| (e, BodyKind::Jason))) {
        commands.entity(e).insert((NetBody { pos: [0.0; 3], yaw: 0.0, kind }, Replicate::to_clients(NetworkTarget::All)));
    }
}

fn update_bodies(mut bodies: Query<(&mut NetBody, &GlobalTransform)>) {
    for (mut b, g) in &mut bodies {
        let pos = g.translation().to_array();
        let yaw = g.rotation().to_euler(EulerRot::YXZ).0;
        if b.pos != pos || b.yaw != yaw {
            b.pos = pos;
            b.yaw = yaw;
        }
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

/// The characters received: their models, placed where the server has them.
#[allow(clippy::type_complexity)]
fn show_bodies(
    mut commands: Commands,
    assets: Res<AssetServer>,
    counselor: Option<Res<crate::character::CounselorCharacter>>,
    killer: Option<Res<crate::character::KillerCharacter>>,
    added: Query<(Entity, &NetBody, Has<Controlled>), Added<NetBody>>,
    mut bodies: Query<(&NetBody, &mut Transform), Without<Controlled>>,
) {
    for (e, b, own) in &added {
        info!("net: received {:?} {e} at {:?}{}", b.kind, b.pos, if own { " (hers)" } else { "" });
        if own {
            continue; // her own counselor is the local one (follow_own_body)
        }
        commands.entity(e).insert((Transform::from_translation(Vec3::from_array(b.pos)), Visibility::default()));
        match (b.kind, counselor.as_deref(), killer.as_deref()) {
            (BodyKind::Counselor, Some(c), _) => {
                crate::character::spawn_model(&mut commands, &assets, c, crate::character::Model::Counselor, e, ());
            }
            (BodyKind::Jason, _, Some(k)) => {
                crate::character::spawn_model(&mut commands, &assets, k, crate::character::Model::Killer, e, ());
            }
            _ => {}
        }
    }
    for (b, mut xf) in &mut bodies {
        xf.translation = Vec3::from_array(b.pos);
        xf.rotation = Quat::from_rotation_y(b.yaw);
    }
}

/// Her own counselor where the server has her.
fn follow_own_body(own: Query<&NetBody, With<Controlled>>, mut local: Query<&mut Transform, (With<LocalPlayer>, With<Player>)>) {
    let (Ok(b), Ok(mut xf)) = (own.single(), local.single_mut()) else { return };
    xf.translation = Vec3::from_array(b.pos);
    xf.rotation = Quat::from_rotation_y(b.yaw);
}

