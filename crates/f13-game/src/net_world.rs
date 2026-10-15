//! The world over the network: the level's actors' state, and what the match does to them that a client sees or
//! hears.
//!
//! Every machine of a match builds the same world from the same layouts and seed (props.rs: MatchWorld), so an
//! actor's index in it ([`WorldId`]) names the same door, window or item everywhere. The server gives each actor whose
//! state changes a proxy: a replicated entity with [`ProxyOf`] and the actor's state (a door's [`NetDoor`], an item's
//! [`NetItem`]...), and each client applies the proxies' state to its own actors. Their looks follow from the state
//! as they do on the server (the door swings, the window's pane, the plank, the lights and the actor sounds that
//! watch it).
//!
//! What the server's simulation does at once (a sound it plays, an item it drops, a door it breaks apart) goes out
//! as a [`Heard`] message ([`shout`]); a client plays it on its own actors.
//!
//! What the simulation spawns as it goes (a thrown knife, a firecracker, a flare, Jason's trap, the police car) carries a
//! [`NetVisual`] (its kind) and goes out with its place ([`NetXf`]) and its prop animation ([`NetPropAnim`]); a client
//! gives its copy the same looks.
//!
//! PLACEHOLDER: the client builds the server's world from its seed, layout and weather (net.rs `query_world`).

use crate::net::NetBody;
use bevy::prelude::*;
use lightyear::prelude::*;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// A level actor's index in the match's world (the same on every machine of the match).
#[derive(Component, Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct WorldId(pub u32);

impl WorldId {
    /// The id of an actor spawned apart from the match's world (a fuse box, a phone): from its level actor's name
    /// (FNV-1a), with the top bit set.
    pub fn named(name: &str) -> Self {
        let h = name.bytes().fold(0x811c_9dc5u32, |h, b| (h ^ b as u32).wrapping_mul(0x0100_0193));
        WorldId(h | 0x8000_0000)
    }
}

/// The actor a proxy carries the state of.
#[derive(Component, Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct ProxyOf(pub u32);

#[derive(Component, Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
pub struct NetDoor {
    pub open: bool,
    pub locked: bool,
    pub target: f32,
    pub health: f32,
    pub broken: bool,
}

/// Used up: a CB radio called on (Tommy's coming), a placed kill used (its bOnlyUseableOnce).
#[derive(Component, Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
pub struct NetUsed(pub bool);

/// A destructible wall's state: the damage state its section shows (stage, from inside) and whether it's broken through.
#[derive(Component, Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
pub struct NetWall {
    pub shown: Option<(u8, bool)>,
    pub broken: bool,
}

#[derive(Component, Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
pub struct NetWindow {
    pub open: bool,
    pub destroyed: bool,
}

#[derive(Component, Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
pub struct NetBarricade {
    pub enabled: bool,
}

/// Where an item a counselor carries is.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub enum Slot {
    Hand,
    Pocket(u8),
    Special(u8),
}

/// An item: who carries it and where (a character's [`NetBody::id`]), else where it is and whether it shows; in a
/// cabinet's slot (the cabinet's id); a gun that has fired.
#[derive(Component, Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
pub struct NetItem {
    pub holder: Option<(u64, Slot)>,
    pub at: [f32; 7],
    pub visible: bool,
    pub cabinet: Option<(u32, u8)>,
    pub spent: bool,
    /// Set as a bear trap, and its prop animation then.
    pub trap: Option<String>,
}

#[derive(Component, Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
pub struct NetVehicle {
    pub at: [f32; 7],
    pub started: bool,
    pub throttle: f32,
    pub handbrake: bool,
    pub speed: f32,
    pub steer: f32,
    pub repairs: Vec<bool>,
}

/// A cabinet: its drawers' places (local transforms), whether each is open, and whether its Open section is done (her
/// item can be taken: a client's bot holds E then).
#[derive(Component, Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
pub struct NetCabinet {
    pub drawers: Vec<([f32; 7], bool, bool)>,
}

/// What the simulation spawned, for a client to show.
#[derive(Component, Serialize, Deserialize, Clone, Debug, PartialEq)]
pub enum NetVisual {
    /// A thrown knife; stuck in what it hit (its dart shows).
    Knife { stuck: bool },
    Firecracker,
    Flare,
    /// Jason's bear trap.
    KillerTrap,
    PoliceCar { class: String },
}

/// Where a spawned thing is (translation, rotation).
#[derive(Component, Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Default)]
pub struct NetXf(pub [f32; 7]);

/// The prop animation it plays (a bear trap's: armed, sprung, escaped).
#[derive(Component, Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
pub struct NetPropAnim(pub Option<String>);

#[derive(Component, Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
pub struct NetFuseBox {
    pub destroyed: bool,
}

/// A radio's state (SCRadio's replicated bIsPlaying, bIsBroken, and its track): radio.rs plays what follows from it.
#[derive(Component, Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
pub struct NetRadio {
    pub playing: bool,
    pub broken: bool,
    pub track: usize,
}

#[derive(Component, Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
pub struct NetPhoneBox {
    pub repaired: bool,
    pub broken: bool,
}

#[derive(Component, Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
pub struct NetPhone {
    pub enabled: bool,
    pub called: bool,
}

/// An entity as both sides name it: a character (its [`NetBody::id`]), a level actor, or a part of one (the child
/// indices down from the actor).
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub enum NetRef {
    Character(u64),
    Actor(u32),
    Part(u32, Vec<u16>),
}

/// What the server's simulation did at once, for the clients to see and hear.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub enum Heard {
    Sound { cue: String, at: Option<[f32; 3]> },
    ActorSound { actor: NetRef, prop: String },
    ActorLoop { actor: NetRef, prop: String },
    StopLoop { actor: NetRef, prop: String },
    Dropped { item: NetRef, at: [f32; 3] },
    BreakApart { piece: NetRef, origin: [f32; 3], radius: f32, force: f32, lifespan: f32 },
    Distortion { at: [f32; 3], dest: Option<[f32; 3]> },
    SoundBlip { at: [f32; 3] },
    JasonSound { cue: String },
    PickupTaken { pickup: NetRef },
}

/// The channel [`Heard`] goes on (in order, every one).
pub struct HeardChannel;

/// What the simulation does at once that clients must see too: [`shout`] triggers it here and, on a server, sends it.
#[derive(Event, Clone, Debug)]
pub enum Shout {
    Sound(crate::audio::PlaySound),
    ActorSound(crate::audio::ActorSound),
    ActorLoop(crate::audio::ActorLoop),
    StopLoop(crate::audio::StopLoop),
    Dropped(crate::audio::Dropped),
    BreakApart(crate::doors::BreakApart),
    Distortion(crate::jason::Distortion),
    /// Only to whoever plays Jason.
    SoundBlip(crate::senses::SoundBlip),
    /// A 2D sound for whoever plays Jason only (his locally controlled character's PlaySound2D).
    JasonSound(JasonSound),
    /// A knife pickup taken (MULTICAST_DisablePickup).
    PickupTaken(crate::jason::PickupTaken),
}

/// A 2D cue only Jason's player hears (played where PlayJason is).
#[derive(Event, Clone, Debug)]
pub struct JasonSound(pub String);

fn jason_sound(ev: On<JasonSound>, mut commands: Commands, play: Option<Res<crate::jason_player::PlayJason>>) {
    if play.is_some() {
        commands.trigger(crate::audio::PlaySound { cue: ev.0.clone(), at: None });
    }
}

pub(crate) fn sounds(app: &mut App) {
    app.add_observer(jason_sound);
}

/// Triggers what the simulation did, and has a server tell its clients.
pub fn shout(commands: &mut Commands, what: Shout) {
    match what.clone() {
        Shout::Sound(e) => commands.trigger(e),
        Shout::ActorSound(e) => commands.trigger(e),
        Shout::ActorLoop(e) => commands.trigger(e),
        Shout::StopLoop(e) => commands.trigger(e),
        Shout::Dropped(e) => commands.trigger(e),
        Shout::BreakApart(e) => commands.trigger(e),
        Shout::Distortion(e) => commands.trigger(e),
        Shout::SoundBlip(e) => commands.trigger(e),
        Shout::JasonSound(e) => commands.trigger(e),
        Shout::PickupTaken(e) => commands.trigger(e),
    }
    commands.trigger(what);
}

/// What both sides agree on.
pub(crate) fn protocol(app: &mut App) {
    app.add_channel::<HeardChannel>(ChannelSettings { mode: ChannelMode::OrderedReliable(ReliableSettings::default()), ..default() }).add_direction(NetworkDirection::ServerToClient);
    app.register_message::<Heard>().add_direction(NetworkDirection::ServerToClient);
    app.component::<ProxyOf>().replicate();
    app.component::<NetDoor>().replicate();
    app.component::<NetWindow>().replicate();
    app.component::<NetWall>().replicate();
    app.component::<NetUsed>().replicate();
    app.component::<NetBarricade>().replicate();
    app.component::<NetItem>().replicate();
    app.component::<NetVehicle>().replicate();
    app.component::<NetFuseBox>().replicate();
    app.component::<NetPhoneBox>().replicate();
    app.component::<NetPhone>().replicate();
    app.component::<NetCabinet>().replicate();
    app.component::<NetRadio>().replicate();
    app.component::<NetVisual>().replicate();
    app.component::<NetXf>().replicate();
    app.component::<NetPropAnim>().replicate();
}

pub(crate) fn server(app: &mut App) {
    app.add_observer(send_heard).add_systems(PostUpdate, (make_proxies, tag_visuals, (send_fixtures, send_walls, send_used, send_items, send_vehicles, send_cabinets, send_radios, send_visuals)).chain().before(ReplicationSystems::Send));
}

pub(crate) fn client(app: &mut App) {
    app.init_resource::<Actors>().add_systems(Update, (index_actors, (apply_fixtures, apply_walls, apply_used, apply_vehicles, apply_items, apply_traps, apply_cabinets, apply_radios, show_visuals, hear)).chain().after(crate::net::Mirrored));
}

fn xf_array(t: &Transform) -> [f32; 7] {
    let (p, r) = (t.translation, t.rotation);
    [p.x, p.y, p.z, r.x, r.y, r.z, r.w]
}

fn array_xf(a: &[f32; 7]) -> Transform {
    Transform::from_translation(Vec3::new(a[0], a[1], a[2])).with_rotation(Quat::from_xyzw(a[3], a[4], a[5], a[6]).normalize())
}

/// Writes `v` only if it changed (an unchanged component doesn't go out again).
fn set<T: PartialEq>(mut c: Mut<T>, v: T) {
    if *c != v {
        *c = v;
    }
}

/// Each actor whose state the clients need gets its proxy.
#[allow(clippy::type_complexity)]
fn make_proxies(
    mut commands: Commands,
    actors: Query<
        (Entity, &WorldId, (Has<crate::doors::Door>, Has<crate::windows::Window>, Has<crate::windows::Barricade>, Has<crate::props::Item>), (Has<crate::props::Vehicle>, Has<crate::power::FuseBox>, Has<crate::police::PhoneBox>, Has<crate::police::Phone>), (Has<crate::cabinets::Cabinet>, Has<crate::radio::Radio>, Has<crate::doors::Wall>, Has<crate::tommy::CbRadio>, Has<crate::placed_kills::PlacedKill>)),
        Without<ProxyLink>,
    >,
) {
    for (e, id, (door, window, barricade, item), (vehicle, fuse, phone_box, phone), (cabinet, radio, wall, cb_radio, placed_kill)) in &actors {
        if !(door || window || barricade || item || vehicle || fuse || phone_box || phone || cabinet || radio || wall || cb_radio || placed_kill) {
            continue;
        }
        let mut p = commands.spawn((ProxyOf(id.0), Replicate::to_clients(NetworkTarget::All)));
        if cb_radio || placed_kill {
            p.insert(NetUsed::default());
        }
        if wall {
            p.insert(NetWall::default());
        }
        if door {
            p.insert(NetDoor::default());
        }
        if window {
            p.insert(NetWindow::default());
        }
        if barricade {
            p.insert(NetBarricade::default());
        }
        if item {
            p.insert(NetItem::default());
        }
        if vehicle {
            p.insert(NetVehicle::default());
        }
        if fuse {
            p.insert(NetFuseBox::default());
        }
        if phone_box {
            p.insert(NetPhoneBox::default());
        }
        if phone {
            p.insert(NetPhone::default());
        }
        if cabinet {
            p.insert(NetCabinet::default());
        }
        if radio {
            p.insert(NetRadio::default());
        }
        let p = p.id();
        commands.entity(e).insert(ProxyLink(p));
    }
}

fn send_radios(radios: Query<(&ProxyLink, &crate::radio::Radio)>, mut proxies: Query<&mut NetRadio>) {
    for (link, r) in &radios {
        if let Ok(n) = proxies.get_mut(link.0) {
            set(n, NetRadio { playing: r.playing, broken: r.broken, track: r.track });
        }
    }
}

fn apply_radios(actors: Res<Actors>, proxies: Query<(&ProxyOf, &NetRadio), Changed<NetRadio>>, mut radios: Query<&mut crate::radio::Radio>) {
    for (of, n) in &proxies {
        let Some(mut r) = actors.0.get(&of.0).and_then(|&e| radios.get_mut(e).ok()) else { continue };
        r.playing = n.playing;
        r.broken = n.broken;
        r.track = n.track;
    }
}

/// On the server: an actor's proxy entity.
#[derive(Component)]
struct ProxyLink(Entity);

/// What's used up: CB radios called on, placed kills used.
fn send_used(actors: Query<(&ProxyLink, Option<&crate::tommy::CbRadio>, Option<&crate::placed_kills::PlacedKill>)>, mut proxies: Query<&mut NetUsed>) {
    for (link, cb, pk) in &actors {
        if let Ok(n) = proxies.get_mut(link.0) {
            set(n, NetUsed(cb.is_some_and(|c| c.called) || pk.is_some_and(|k| k.used)));
        }
    }
}

/// A client: what the server has used up (the prompts here follow it).
fn apply_used(actors: Res<Actors>, proxies: Query<(&ProxyOf, &NetUsed), Changed<NetUsed>>, mut cbs: Query<&mut crate::tommy::CbRadio>, mut kills: Query<&mut crate::placed_kills::PlacedKill>) {
    for (of, n) in &proxies {
        let Some(&e) = actors.0.get(&of.0) else { continue };
        if let Ok(mut c) = cbs.get_mut(e) {
            c.called = n.0;
        }
        if let Ok(mut k) = kills.get_mut(e) {
            k.used = n.0;
        }
    }
}

/// Each destructible wall's damage state and break (its OnRep_Health: the section's mesh, broken through).
fn send_walls(walls: Query<(&ProxyLink, &crate::doors::Wall)>, mut proxies: Query<&mut NetWall>) {
    for (link, w) in &walls {
        if let Ok(n) = proxies.get_mut(link.0) {
            set(n, NetWall { shown: w.shown.map(|(s, inside)| (s as u8, inside)), broken: w.broken });
        }
    }
}

/// A client: the server's walls' damage states and breaks (their pieces fly from its BreakApart shout).
fn apply_walls(mut commands: Commands, actors: Res<Actors>, proxies: Query<(&ProxyOf, &NetWall), Changed<NetWall>>, mut walls: Query<&mut crate::doors::Wall>) {
    for (of, n) in &proxies {
        let Some(&e) = actors.0.get(&of.0) else { continue };
        let Ok(mut w) = walls.get_mut(e) else { continue };
        w.show(e, n.shown.map(|(s, inside)| (s as usize, inside)), &mut commands);
        if n.broken && !w.broken {
            w.break_down(&mut commands);
        }
    }
}

#[allow(clippy::type_complexity)]
fn send_fixtures(
    actors: Query<(&ProxyLink, Option<&crate::doors::Door>, Option<&crate::windows::Window>, Option<&crate::windows::Barricade>, (Option<&crate::power::FuseBox>, Option<&crate::police::PhoneBox>, Option<&crate::police::Phone>))>,
    mut proxies: Query<(Option<&mut NetDoor>, Option<&mut NetWindow>, Option<&mut NetBarricade>, (Option<&mut NetFuseBox>, Option<&mut NetPhoneBox>, Option<&mut NetPhone>))>,
) {
    for (link, door, window, barricade, (fuse, phone_box, phone)) in &actors {
        let Ok((nd, nw, nb, (nf, npb, np))) = proxies.get_mut(link.0) else { continue };
        if let (Some(d), Some(n)) = (door, nd) {
            set(n, NetDoor { open: d.open, locked: d.locked, target: d.target, health: d.health, broken: d.broken });
        }
        if let (Some(w), Some(n)) = (window, nw) {
            set(n, NetWindow { open: w.open, destroyed: w.destroyed });
        }
        if let (Some(b), Some(n)) = (barricade, nb) {
            set(n, NetBarricade { enabled: b.enabled });
        }
        if let (Some(f), Some(n)) = (fuse, nf) {
            set(n, NetFuseBox { destroyed: f.destroyed });
        }
        if let (Some(b), Some(n)) = (phone_box, npb) {
            set(n, NetPhoneBox { repaired: b.repaired, broken: b.broken });
        }
        if let (Some(p), Some(n)) = (phone, np) {
            set(n, NetPhone { enabled: p.enabled, called: p.called });
        }
    }
}

#[allow(clippy::type_complexity)]
fn send_items(
    carriers: Query<(&NetBody, &crate::interact::Inventory)>,
    items: Query<(Entity, &ProxyLink, &GlobalTransform, Option<&Visibility>, Option<&crate::props::InCabinet>, Has<crate::items::Spent>, (Has<crate::items::ArmedTrap>, Option<&crate::prop_anim::PropAnim>))>,
    cabinets: Query<&WorldId>,
    mut proxies: Query<&mut NetItem>,
) {
    let mut held: HashMap<Entity, (u64, Slot)> = HashMap::new();
    for (b, inv) in &carriers {
        if let Some(h) = inv.hand {
            held.insert(h, (b.id, Slot::Hand));
        }
        for (i, &p) in inv.pockets.iter().enumerate() {
            held.insert(p, (b.id, Slot::Pocket(i as u8)));
        }
        for (i, &s) in inv.special.iter().enumerate() {
            held.insert(s, (b.id, Slot::Special(i as u8)));
        }
    }
    for (e, link, g, vis, cabinet, spent, (armed, anim)) in &items {
        let Ok(n) = proxies.get_mut(link.0) else { continue };
        // Set as a trap (no longer an item): armed, or sprung (its animation since).
        let trap = (armed || n.trap.is_some()).then(|| anim.map_or(String::new(), |a| a.name.to_string()));
        let holder = held.get(&e).copied();
        // Where it lies (not while carried: then it hangs off her hand or is nowhere).
        let at = if holder.is_some() { n.at } else { xf_array(&g.compute_transform()) };
        let cabinet = cabinet.and_then(|c| Some((cabinets.get(c.0).ok()?.0, c.1 as u8)));
        set(n, NetItem { holder, at, visible: vis.is_none_or(|v| *v != Visibility::Hidden), cabinet, spent, trap });
    }
}

fn send_vehicles(vehicles: Query<(&ProxyLink, &crate::props::Vehicle, &Transform)>, mut proxies: Query<&mut NetVehicle>) {
    for (link, v, xf) in &vehicles {
        let Ok(n) = proxies.get_mut(link.0) else { continue };
        set(n, NetVehicle {
            at: xf_array(xf),
            started: v.started,
            throttle: v.throttle,
            handbrake: v.handbrake,
            speed: v.speed,
            steer: v.steer,
            repairs: v.repairs.iter().map(|r| r.done).collect(),
        });
    }
}

/// How the server names an entity to its clients: a character's body id, an actor's id, or a part's path below
/// its actor.
fn net_ref(e: Entity, bodies: &Query<&NetBody>, ids: &Query<&WorldId>, parents: &Query<&ChildOf>, children: &Query<&Children>) -> Option<NetRef> {
    if let Ok(b) = bodies.get(e) {
        return Some(NetRef::Character(b.id));
    }
    if let Ok(id) = ids.get(e) {
        return Some(NetRef::Actor(id.0));
    }
    let mut path = Vec::new();
    let mut at = e;
    while let Ok(p) = parents.get(at) {
        let parent = p.parent();
        let i = children.get(parent).ok()?.iter().position(|c| c == at)?;
        path.push(i as u16);
        if let Ok(id) = ids.get(parent) {
            path.reverse();
            return Some(NetRef::Part(id.0, path));
        }
        at = parent;
    }
    None
}

/// The server tells its clients what its simulation shouted.
#[allow(clippy::too_many_arguments)]
fn send_heard(
    what: On<Shout>,
    bodies: Query<&NetBody>,
    ids: Query<&WorldId>,
    parents: Query<&ChildOf>,
    children: Query<&Children>,
    mut sender: ServerMultiMessageSender,
    server: Query<&lightyear::prelude::server::Server>,
    jason_player: Option<Res<crate::lobby::JasonPlayer>>,
    remotes: Query<&RemoteId>,
) {
    let r = |e: Entity| net_ref(e, &bodies, &ids, &parents, &children);
    let heard = match &*what {
        Shout::Sound(s) => Some(Heard::Sound { cue: s.cue.clone(), at: s.at.map(|a| a.to_array()) }),
        Shout::ActorSound(s) => r(s.actor).map(|actor| Heard::ActorSound { actor, prop: s.prop.to_string() }),
        Shout::ActorLoop(s) => r(s.actor).map(|actor| Heard::ActorLoop { actor, prop: s.prop.to_string() }),
        Shout::StopLoop(s) => r(s.actor).map(|actor| Heard::StopLoop { actor, prop: s.prop.to_string() }),
        Shout::Dropped(d) => r(d.item).map(|item| Heard::Dropped { item, at: d.at.to_array() }),
        Shout::BreakApart(b) => r(b.piece).map(|piece| Heard::BreakApart { piece, origin: b.origin.to_array(), radius: b.radius, force: b.force, lifespan: b.lifespan }),
        Shout::Distortion(d) => Some(Heard::Distortion { at: d.at.to_array(), dest: d.dest.map(|d| d.to_array()) }),
        Shout::SoundBlip(b) => Some(Heard::SoundBlip { at: b.at.to_array() }),
        Shout::JasonSound(s) => Some(Heard::JasonSound { cue: s.0.clone() }),
        Shout::PickupTaken(p) => r(p.0).map(|pickup| Heard::PickupTaken { pickup }),
    };
    let Some(heard) = heard else {
        debug!("net: can't name {:?} to the clients", *what);
        return;
    };
    let Ok(server) = server.single() else { return };
    // Jason's sound blips are his alone (SoundBlip_C spawns on his machine only).
    let to = match heard {
        Heard::SoundBlip { .. } | Heard::JasonSound { .. } => match jason_player.as_ref().and_then(|j| remotes.get(j.0).ok()) {
            Some(id) => NetworkTarget::Single(id.0),
            None => return,
        },
        _ => NetworkTarget::All,
    };
    let _ = sender.send::<_, HeardChannel>(&heard, server, &to);
}

/// On a client: its actors by id.
#[derive(Resource, Default)]
pub(crate) struct Actors(HashMap<u32, Entity>);

impl Actors {
    pub(crate) fn get(&self, id: u32) -> Option<Entity> {
        self.0.get(&id).copied()
    }
}

fn index_actors(mut actors: ResMut<Actors>, added: Query<(Entity, &WorldId), Added<WorldId>>) {
    for (e, id) in &added {
        actors.0.insert(id.0, e);
    }
}

/// The entity a server's name stands for here.
fn resolve(r: &NetRef, actors: &Actors, twins: &crate::net::Twins, children: &Query<&Children>) -> Option<Entity> {
    match r {
        NetRef::Character(id) => twins.of(*id),
        NetRef::Actor(id) => actors.0.get(id).copied(),
        NetRef::Part(id, path) => {
            let mut at = *actors.0.get(id)?;
            for &i in path {
                at = *children.get(at).ok()?.get(i as usize)?;
            }
            Some(at)
        }
    }
}

#[allow(clippy::type_complexity)]
fn apply_fixtures(
    mut commands: Commands,
    actors: Res<Actors>,
    proxies: Query<
        (&ProxyOf, Option<Ref<NetDoor>>, Option<Ref<NetWindow>>, Option<Ref<NetBarricade>>, (Option<Ref<NetFuseBox>>, Option<Ref<NetPhoneBox>>, Option<Ref<NetPhone>>)),
        Or<(Changed<NetDoor>, Changed<NetWindow>, Changed<NetBarricade>, Changed<NetFuseBox>, Changed<NetPhoneBox>, Changed<NetPhone>)>,
    >,
    mut doors: Query<&mut crate::doors::Door>,
    mut windows: Query<&mut crate::windows::Window>,
    mut barricades: Query<&mut crate::windows::Barricade>,
    (mut fuses, mut grids): (Query<&mut crate::power::FuseBox>, Option<ResMut<crate::power::PowerGrids>>),
    mut phone_boxes: Query<&mut crate::police::PhoneBox>,
    mut phones: Query<&mut crate::police::Phone>,
) {
    for (of, nd, nw, nb, (nf, npb, np)) in &proxies {
        let Some(&e) = actors.0.get(&of.0) else { continue };
        if let (Some(n), Ok(mut d)) = (nd.filter(|n| n.is_changed()), doors.get_mut(e)) {
            debug!("net: door {} open {} locked {} health {} broken {}", of.0, n.open, n.locked, n.health, n.broken);
            d.open = n.open;
            d.locked = n.locked;
            if d.target != n.target {
                d.target = n.target;
            }
            // The leaf's damage state the hits crossed into (SCDoor's OnRep: its mesh), and broken, the leaf gone
            // (its pieces fly from the BreakApart shout).
            let before = d.health;
            d.health = n.health;
            if n.health < before && !n.broken {
                if let Some(stage) = d.damage_stage(before) {
                    d.show_damage(stage, &mut commands);
                }
            }
            if n.broken && !d.broken {
                commands.entity(d.pivot).insert(Visibility::Hidden);
            }
            d.broken = n.broken;
        }
        if let (Some(n), Ok(mut w)) = (nw.filter(|n| n.is_changed()), windows.get_mut(e)) {
            debug!("net: window {} open {} destroyed {}", of.0, n.open, n.destroyed);
            w.open = n.open;
            w.destroyed = n.destroyed;
        }
        if let (Some(n), Ok(mut b)) = (nb.filter(|n| n.is_changed()), barricades.get_mut(e)) {
            b.enabled = n.enabled;
        }
        // Its grid with it (the lights it feeds).
        if let (Some(n), Ok(mut f)) = (nf.filter(|n| n.is_changed()), fuses.get_mut(e)) {
            match grids.as_deref_mut() {
                Some(g) if f.destroyed != n.destroyed => crate::power::set_destroyed(&mut f, n.destroyed, g),
                _ => f.destroyed = n.destroyed,
            }
        }
        if let (Some(n), Ok(mut b)) = (npb.filter(|n| n.is_changed()), phone_boxes.get_mut(e)) {
            b.repaired = n.repaired;
            b.broken = n.broken;
        }
        if let (Some(n), Ok(mut p)) = (np.filter(|n| n.is_changed()), phones.get_mut(e)) {
            p.enabled = n.enabled;
            p.called = n.called;
        }
    }
}

fn apply_vehicles(actors: Res<Actors>, proxies: Query<(&ProxyOf, &NetVehicle), Changed<NetVehicle>>, mut vehicles: Query<(&mut crate::props::Vehicle, &mut Transform)>) {
    for (of, n) in &proxies {
        let Some(&e) = actors.0.get(&of.0) else { continue };
        let Ok((mut v, mut xf)) = vehicles.get_mut(e) else { continue };
        *xf = array_xf(&n.at);
        v.started = n.started;
        v.throttle = n.throttle;
        v.handbrake = n.handbrake;
        v.speed = n.speed;
        v.steer = n.steer;
        for (r, &done) in v.repairs.iter_mut().zip(&n.repairs) {
            r.done = done;
        }
    }
}

/// The items where the server has them, and each counselor's inventory from them.
#[allow(clippy::type_complexity)]
fn apply_items(
    mut commands: Commands,
    actors: Res<Actors>,
    twins: Res<crate::net::Twins>,
    proxies: Query<(&ProxyOf, Ref<NetItem>)>,
    mut items: Query<(&mut Transform, &mut Visibility, Has<crate::interact::Carried>, Has<crate::items::InHand>, Has<crate::items::Spent>, Option<&crate::props::InCabinet>), With<crate::props::Item>>,
    mut inventories: Query<(Entity, &mut crate::interact::Inventory), With<crate::player::Player>>,
) {
    // Each carrier's hand, pockets and special items, in the server's order.
    let mut carried: HashMap<Entity, (Option<Entity>, Vec<(u8, Entity)>, Vec<(u8, Entity)>)> = HashMap::new();
    for (of, n) in &proxies {
        let Some(&e) = actors.0.get(&of.0) else { continue };
        let Ok((mut xf, mut vis, is_carried, in_hand, spent, cabinet)) = items.get_mut(e) else { continue };
        if n.is_changed() {
            debug!("net: item {} holder {:?} visible {} cabinet {:?}", of.0, n.holder, n.visible, n.cabinet);
        }
        let mut ec = commands.entity(e);
        match (n.spent, spent) {
            (true, false) => {
                ec.insert(crate::items::Spent);
            }
            (false, true) => {
                ec.remove::<crate::items::Spent>();
            }
            _ => {}
        }
        match n.holder {
            Some((id, slot)) => {
                let Some(twin) = twins.of(id) else { continue };
                let c = carried.entry(twin).or_default();
                match slot {
                    Slot::Hand => c.0 = Some(e),
                    Slot::Pocket(i) => c.1.push((i, e)),
                    Slot::Special(i) => c.2.push((i, e)),
                }
                if !is_carried {
                    ec.insert(crate::interact::Carried).remove::<crate::props::InCabinet>();
                }
                // Out of her hand (hold_in_hand hangs the hand's item on her).
                if slot != Slot::Hand {
                    if in_hand {
                        ec.remove::<(ChildOf, crate::items::InHand, crate::items::HeldAt)>();
                    }
                    vis.set_if_neq(Visibility::Hidden);
                }
            }
            None => {
                if is_carried || in_hand {
                    ec.remove::<(crate::interact::Carried, ChildOf, crate::items::InHand, crate::items::HeldAt)>();
                }
                if n.is_changed() || is_carried || in_hand {
                    *xf = array_xf(&n.at);
                    vis.set_if_neq(if n.visible { Visibility::Inherited } else { Visibility::Hidden });
                }
                let want = n.cabinet.and_then(|(c, slot)| Some(crate::props::InCabinet(*actors.0.get(&c)?, slot as usize)));
                match (want, cabinet) {
                    (Some(w), Some(have)) if w.0 == have.0 && w.1 == have.1 => {}
                    (Some(w), _) => {
                        ec.insert(w);
                    }
                    (None, Some(_)) => {
                        ec.remove::<crate::props::InCabinet>();
                    }
                    (None, None) => {}
                }
            }
        }
    }
    for (pe, mut inv) in &mut inventories {
        let (hand, mut pockets, mut special) = carried.remove(&pe).unwrap_or_default();
        pockets.sort_by_key(|p| p.0);
        special.sort_by_key(|p| p.0);
        let pockets: Vec<Entity> = pockets.into_iter().map(|p| p.1).collect();
        let special: Vec<Entity> = special.into_iter().map(|p| p.1).collect();
        if inv.hand != hand || inv.pockets != pockets || inv.special != special {
            inv.hand = hand;
            inv.pockets = pockets;
            inv.special = special;
        }
    }
}

/// What the server's simulation did, here.
fn hear(mut commands: Commands, actors: Res<Actors>, twins: Res<crate::net::Twins>, children: Query<&Children>, mut receivers: Query<&mut MessageReceiver<Heard>>) {
    let v = |a: [f32; 3]| Vec3::from_array(a);
    for mut r in &mut receivers {
        for heard in r.receive() {
            let find = |n: &NetRef| resolve(n, &actors, &twins, &children);
            match heard {
                Heard::Sound { cue, at } => commands.trigger(crate::audio::PlaySound { cue, at: at.map(v) }),
                Heard::ActorSound { actor, prop } => {
                    if let Some(actor) = find(&actor) {
                        commands.trigger(crate::audio::ActorSound { actor, prop: crate::audio::intern(&prop) });
                    }
                }
                Heard::ActorLoop { actor, prop } => {
                    if let Some(actor) = find(&actor) {
                        commands.trigger(crate::audio::ActorLoop { actor, prop: crate::audio::intern(&prop) });
                    }
                }
                Heard::StopLoop { actor, prop } => {
                    if let Some(actor) = find(&actor) {
                        commands.trigger(crate::audio::StopLoop { actor, prop: crate::audio::intern(&prop) });
                    }
                }
                Heard::Dropped { item, at } => {
                    if let Some(item) = find(&item) {
                        commands.trigger(crate::audio::Dropped { item, at: v(at) });
                    }
                }
                Heard::BreakApart { piece, origin, radius, force, lifespan } => {
                    if let Some(piece) = find(&piece) {
                        commands.trigger(crate::doors::BreakApart { piece, origin: v(origin), radius, force, lifespan });
                    }
                }
                Heard::Distortion { at, dest } => commands.trigger(crate::jason::Distortion { at: v(at), dest: dest.map(v) }),
                Heard::SoundBlip { at } => commands.trigger(crate::senses::SoundBlip { at: v(at) }),
                Heard::JasonSound { cue } => commands.trigger(JasonSound(cue)),
                Heard::PickupTaken { pickup } => {
                    if let Some(p) = find(&pickup) {
                        commands.trigger(crate::jason::PickupTaken(p));
                    }
                }
            }
        }
    }
}

fn send_cabinets(cabinets: Query<(&ProxyLink, &crate::cabinets::Cabinet)>, drawers: Query<&Transform>, mut proxies: Query<&mut NetCabinet>) {
    for (link, c) in &cabinets {
        let Ok(n) = proxies.get_mut(link.0) else { continue };
        let d = c.slots.iter().map(|s| (s.drawer.and_then(|d| drawers.get(d).ok()).map_or([0.0; 7], xf_array), s.open, s.ready)).collect();
        set(n, NetCabinet { drawers: d });
    }
}

/// What the simulation spawned goes out to the clients.
fn tag_visuals(mut commands: Commands, added: Query<(Entity, &Transform), (Added<NetVisual>, Without<NetXf>)>) {
    for (e, xf) in &added {
        commands.entity(e).insert((NetXf(xf_array(xf)), NetPropAnim::default(), Replicate::to_clients(NetworkTarget::All)));
    }
}

fn send_visuals(mut visuals: Query<(&Transform, &mut NetXf, &mut NetPropAnim, Option<&crate::prop_anim::PropAnim>), With<NetVisual>>) {
    for (xf, x, a, anim) in &mut visuals {
        set(x, NetXf(xf_array(xf)));
        set(a, NetPropAnim(anim.map(|a| a.name.to_string())));
    }
}

/// A prop animation's name as [`crate::prop_anim::PropAnim`] takes it (the bear trap's few).
fn prop_anim(name: &str) -> Option<&'static str> {
    use crate::prop_anim::*;
    [TRAP_ARMED, TRAP_SPRUNG, TRAP_ESCAPED, TRAP_CLOSED].into_iter().find(|n| *n == name)
}

/// Bear traps the counselors set: no longer items here; their animations as the server plays them.
fn apply_traps(mut commands: Commands, actors: Res<Actors>, proxies: Query<(&ProxyOf, &NetItem), Changed<NetItem>>, anims: Query<&crate::prop_anim::PropAnim>) {
    for (of, n) in &proxies {
        let (Some(&e), Some(trap)) = (actors.0.get(&of.0), &n.trap) else { continue };
        let mut ec = commands.entity(e);
        ec.remove::<(crate::props::Item, crate::interact::Carried, ChildOf, crate::items::InHand, crate::items::HeldAt)>().insert((array_xf(&n.at), Visibility::Inherited));
        if let Some(name) = prop_anim(trap) {
            if anims.get(e).map_or(true, |a| a.name != name) {
                ec.insert(crate::prop_anim::PropAnim::new(name));
            }
        }
        // Armed (set, not sprung): an armed trap here too.
        if prop_anim(trap) == Some(crate::prop_anim::TRAP_ARMED) {
            ec.insert(crate::items::ArmedTrap::remote());
        } else {
            ec.remove::<crate::items::ArmedTrap>();
        }
    }
}

fn apply_cabinets(actors: Res<Actors>, proxies: Query<(&ProxyOf, &NetCabinet), Changed<NetCabinet>>, mut cabinets: Query<&mut crate::cabinets::Cabinet>, mut transforms: Query<&mut Transform>) {
    for (of, n) in &proxies {
        let Some(&e) = actors.0.get(&of.0) else { continue };
        let Ok(mut c) = cabinets.get_mut(e) else { continue };
        for (slot, (xf, open, ready)) in c.slots.iter_mut().zip(&n.drawers) {
            slot.open = *open;
            slot.ready = *ready;
            if let Some(mut t) = slot.drawer.and_then(|d| transforms.get_mut(d).ok()) {
                *t = array_xf(xf);
            }
        }
    }
}

/// The server's spawned things here: their looks once, then their place and animation.
#[allow(clippy::too_many_arguments)]
fn show_visuals(
    mut commands: Commands,
    (assets, export, look): (Res<AssetServer>, Res<crate::ExportRoot>, Option<Res<crate::jason::KnifeLook>>),
    (mut meshes, mut materials): (ResMut<Assets<Mesh>>, ResMut<Assets<StandardMaterial>>),
    added: Query<(Entity, &NetVisual, &NetXf), Added<NetVisual>>,
    mut moved: Query<(Entity, &NetVisual, Ref<NetXf>, &NetPropAnim, Option<&Shown>, Option<&mut Transform>)>,
    anims: Query<&crate::prop_anim::PropAnim>,
    mut visibility: Query<&mut Visibility>,
) {
    for (e, v, x) in &added {
        let at = array_xf(&x.0);
        info!("net: the server's {v:?} at {}", at.translation);
        match v {
            NetVisual::PoliceCar { class } => {
                crate::police::spawn_car(&mut commands, &export, &assets, class, at);
                commands.entity(e).insert(Shown(None));
            }
            NetVisual::Knife { .. } => {
                commands.entity(e).insert((at, Visibility::default()));
                let dart = crate::jason::knife_look(&mut commands, look.as_deref(), e);
                commands.entity(e).insert(Shown(dart));
            }
            _ => {
                commands.entity(e).insert((at, Visibility::default(), Shown(None)));
                crate::items::projectile_look(&mut commands, &assets, &mut meshes, &mut materials, e, v);
            }
        }
    }
    for (e, v, x, a, shown, xf) in &mut moved {
        if let (true, Some(mut xf)) = (x.is_changed(), xf) {
            *xf = array_xf(&x.0);
        }
        if let (NetVisual::Knife { stuck: true }, Some(Shown(Some(dart)))) = (v, shown) {
            if let Ok(mut vis) = visibility.get_mut(*dart) {
                if *vis != Visibility::Inherited {
                    *vis = Visibility::Inherited;
                    crate::jason::stop_knife_flight(&mut commands, e);
                }
            }
        }
        if let Some(name) = a.0.as_deref().and_then(prop_anim) {
            if anims.get(e).map_or(true, |p| p.name != name) {
                commands.entity(e).insert(crate::prop_anim::PropAnim::new(name));
            }
        }
    }
}

/// On a client: a spawned thing's looks are made (its knife's dart, if it has one).
#[derive(Component)]
struct Shown(Option<Entity>);
