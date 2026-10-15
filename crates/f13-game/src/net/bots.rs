//! The network test bots (`--bot-net`, `--bot-net-abilities`: a client's, or a lobby's playing server's) and the
//! `--bot-hooks` server's scene changes for them; net.rs registers them.

use super::*;

/// On a client: its scenario bot's scene changes to the server.
pub(super) fn send_bot_setup(mut senders: Query<&mut MessageSender<BotSetupMsg>, With<Connected>>) {
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

/// On a `--bot-hooks` server: each player's bot's scene changes, made to her counselor and the world.
#[allow(clippy::type_complexity)]
pub(super) fn bot_hooks(
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
                crate::bot::BotSetup::PlaceHostCounselor(at) => {
                    for (e, _) in &own {
                        commands.entity(e).insert((Transform::from_translation(Vec3::from_array(at)), crate::player::Unsettled)).remove::<crate::interact::MoveToSpot>();
                    }
                }
                crate::bot::BotSetup::StunJason => {
                    for (je, (mut j, _)) in jason_entities.iter().zip(jasons.iter_mut()) {
                        let take = j.stun_for_test();
                        commands.entity(je).insert(PlayAction(take.into()));
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

/// The network test bot's state (`--bot-net` on a client, or a lobby's playing server).
#[derive(Resource, Default)]
pub(super) struct NetBot {
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
    /// Its camera took his view here.
    pickup_camera: bool,
    /// His Shift's and Stalk's ability bars came here (his HUD's, as they run).
    bars_seen: (bool, bool),
    /// His grab seen here (the server's hold time), and the grab kill's slot pressed.
    grabbed: bool,
    kill_slot: Option<usize>,
    /// The door he was set at, and the stun asked for.
    door: Option<Entity>,
    stun_asked: bool,
    /// The placed kill she was set at, and its prompt seen here while he held her.
    placed_at: bool,
    placed_prompt: bool,
}

impl NetBot {
    pub(super) fn new(prefer: crate::lobby::Preference, abilities: bool) -> Self {
        NetBot { prefer, abilities, ..default() }
    }
}

/// How long the bot waits for the server (real seconds) before giving up.
const NET_BOT_CONNECT_S: f32 = 120.0;

/// `--bot-net`: plays as a client only through its input, and checks what comes back. Its counselor walks (W, 3 s)
/// and must move on the server; she goes into combat stance (C) and must be in it there; the match's clock must run;
/// and Jason, whom the server runs, must move.
pub(super) fn net_bot(
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

/// The net bot in the lobby: asks for its preference, then readies up once there's another player.
pub(super) fn net_bot_lobby(mut bot: ResMut<NetBot>, mut keys: ResMut<ButtonInput<KeyCode>>, lobby: Query<&crate::lobby::NetLobby>) {
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
pub(super) fn host_bot_jason(
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
pub(super) fn net_bot_jason(
    mut bot: ResMut<NetBot>,
    time: Res<Time>,
    mut controls: ResMut<crate::jason_player::JasonControls>,
    own: Query<(&NetBody, &NetJason), With<Controlled>>,
    know: Query<&crate::senses::Knowledge, With<crate::jason::Jason>>,
    (mut map, window): (ResMut<crate::jason_player::MorphMap>, Query<&bevy::window::Window, With<bevy::window::PrimaryWindow>>),
    pickups: Query<(Entity, &crate::props::KnifePickup, &GlobalTransform, Has<crate::props::PickedUp>)>,
    mut exit: MessageWriter<AppExit>,
    mut cameras: Query<&mut crate::player::PlayerCamera>,
    (jasons, rig, interaction_view, bodies, grab_kills): (Query<&crate::jason::Jason>, Option<Res<crate::jason_player::JasonRig>>, Option<Res<crate::interaction_camera::View>>, Query<&NetBody, Without<Controlled>>, Option<Res<crate::jason::GrabKillsAvailable>>),
    (doors, stun_views): (Query<(Entity, &crate::doors::Door, &GlobalTransform)>, Query<(), (With<crate::player::PlayerCamera>, With<crate::screen_effects::StunEffect>)>),
    (placed_kills, prompts): (Query<&crate::placed_kills::PlacedKill>, Res<crate::ui::Prompts>),
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
            for j in &jasons {
                if let Some(r) = j.remote {
                    bot.bars_seen.0 |= r.shift_bar.is_some();
                    bot.bars_seen.1 |= r.stalk_bar.is_some();
                }
            }
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
                // (And his camera's: his look copies it into the controls each frame, before or after this.)
                for mut cam in &mut cameras {
                    cam.yaw = ky;
                    cam.pitch = kp;
                }
                // (The pickup's camera takes his view here as it plays: AlignmentSpecialMove's.)
                if interaction_view.is_some() && !bot.pickup_camera {
                    bot.pickup_camera = true;
                    info!("bot: net: the knife pickup's camera here");
                }
                if taken && nj.knives > before {
                    if !bot.pickup_camera {
                        warn!("bot: net: the knife pickup's camera never took the view here");
                        bot.failed.push("knife pickup camera");
                    }
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
            // Then a knife aimed (middle mouse held): the server's aim seen here, his reticle (ui.rs) and knife camera
            // follow it; let go, it's thrown.
            if i == STEPS.len() + 1 {
                // (Once he's done with the pickup: a press while busy is lost.)
                if !matches!(nj.state.as_str(), "Hunting" | "Throwing") && bot.t < 6.0 {
                    bot.t = 0.0;
                    return;
                }
                let aimed = jasons.iter().any(|j| j.remote.as_ref().is_some_and(|r| r.knife_aim));
                let camera = rig.as_ref().is_some_and(|r| r.knife_camera());
                if aimed && camera {
                    info!("bot: net: aiming a knife here (the server's aim, the reticle, the knife camera)");
                    controls.apply_remote(0, 0, yaw, pitch, view);
                    bot.step += 1;
                    bot.t = 0.0;
                } else if bot.t > 6.0 {
                    warn!("bot: net: the knife aim: seen {aimed}, knife camera {camera}");
                    bot.failed.push("knife aim");
                    controls.apply_remote(0, 0, yaw, pitch, view);
                    bot.step += 1;
                    bot.t = 0.0;
                } else if bot.t > 1.0 {
                    controls.apply_remote(0, 1 << 2, yaw, pitch, view);
                }
                return;
            }
            // (The knife let go: a moment for it to land here.)
            if i == STEPS.len() + 2 && bot.t < 3.0 {
                return;
            }
            // Last a grab (right mouse) on the host's counselor, set beside her: his hold time comes here (the placed
            // kill prompts wait on it); then a grab kill (its slot's key let go): his kill's camera here.
            // (First she's set at the nearest placed kill's spot: its prompt shows him once he holds her.)
            if i == STEPS.len() + 4 && !bot.placed_at {
                bot.placed_at = true;
                if let Some(k) = placed_kills.iter().min_by(|a, b| a.spot.translation.distance(at).total_cmp(&b.spot.translation.distance(at))) {
                    info!("bot: net: her to the placed kill at {}", k.spot.translation);
                    crate::bot::setup(crate::bot::BotSetup::PlaceHostCounselor(k.spot.translation.to_array()));
                    bot.t = 0.0;
                }
                return;
            }
            if i == STEPS.len() + 5 && !bot.placed_prompt {
                if prompts.has(crate::ui::PromptSource::PlacedKill) {
                    info!("bot: net: the placed kill's prompt here, as he holds her");
                    bot.placed_prompt = true;
                } else if bot.t > 5.0 {
                    warn!("bot: net: no placed kill prompt here");
                    bot.failed.push("placed kill prompt");
                    bot.placed_prompt = true;
                } else {
                    return;
                }
            }
            if i == STEPS.len() + 4 || i == STEPS.len() + 5 {
                let her = bodies.iter().filter(|b| b.kind == BodyKind::Counselor).map(|b| Vec3::from_array(b.pos)).min_by(|a, b| a.distance(at).total_cmp(&b.distance(at)));
                let Some(her) = her else {
                    bot.failed.push("no counselor to grab");
                    bot.step += 2;
                    return;
                };
                let to = her - at;
                let (ky, kp) = ((-to.x).atan2(-to.z), 0.0);
                controls.yaw = ky;
                controls.pitch = kp;
                for mut cam in &mut cameras {
                    cam.yaw = ky;
                    cam.pitch = kp;
                }
                let holding = jasons.iter().any(|j| j.holding_for().is_some());
                if i == STEPS.len() + 4 {
                    if holding {
                        info!("bot: net: his grab here (held {:.1} s)", jasons.iter().find_map(|j| j.holding_for()).unwrap_or(0.0));
                        bot.grabbed = true;
                        bot.step += 1;
                        bot.t = 0.0;
                        controls.apply_remote(0, 0, ky, kp, view);
                    } else if bot.t > 20.0 {
                        warn!("bot: net: no grab here (state {})", nj.state);
                        bot.failed.push("grab");
                        bot.step += 2;
                        bot.t = 0.0;
                    } else if Vec2::new(to.x, to.z).length() > 1.8 {
                        // Beside her, 1.5 m off, from her side towards him.
                        let off = Vec2::new(-to.x, -to.z).normalize_or(Vec2::X) * 1.5;
                        crate::bot::setup(crate::bot::BotSetup::PlaceJason([her.x + off.x, her.y, her.z + off.y]));
                        controls.apply_remote(0, 0, ky, kp, view);
                    } else {
                        // (Right mouse, a 0.1 s tap every 0.5 s.)
                        controls.apply_remote(0, if (bot.t * 2.0).fract() < 0.2 { 1 << 1 } else { 0 }, ky, kp, view);
                    }
                    return;
                }
                // A grab kill: the first slot available here (the server's), its key held then let go.
                if rig.as_ref().is_some_and(|r| r.kill_camera()) && jasons.iter().any(|j| j.remote_state == "Killing") {
                    info!("bot: net: his kill's camera here ({})", bot.kill_slot.map_or("the placed kill".to_string(), |s| format!("grab kill slot {s}")));
                    bot.step += 1;
                    bot.t = 0.0;
                    controls.apply_remote(0, 0, ky, kp, view);
                    return;
                }
                if bot.t > 20.0 {
                    warn!("bot: net: no grab kill camera here (state {}, slot {:?})", nj.state, bot.kill_slot);
                    bot.failed.push("grab kill camera");
                    bot.step += 1;
                    bot.t = 0.0;
                    return;
                }
                let slot = bot.kill_slot.or_else(|| grab_kills.as_ref().and_then(|g| g.0.iter().position(|a| *a)));
                bot.kill_slot = slot;
                if slot.is_none() && bot.t > 2.0 {
                    // No grab kill fits here: the placed kill (E on its spot, facing it), his kill as well.
                    let Some(k) = placed_kills.iter().min_by(|a, b| a.spot.translation.distance(at).total_cmp(&b.spot.translation.distance(at))) else { return };
                    if at.distance(k.spot.translation) > 0.5 {
                        crate::bot::setup(crate::bot::BotSetup::PlaceJason(k.spot.translation.to_array()));
                    }
                    let to = k.interact - (at + Vec3::Y * 0.88);
                    let (ky, kp) = ((-to.x).atan2(-to.z), to.y.atan2(Vec2::new(to.x, to.z).length()));
                    controls.yaw = ky;
                    controls.pitch = kp;
                    for mut cam in &mut cameras {
                        cam.yaw = ky;
                        cam.pitch = kp;
                    }
                    controls.apply_remote(if (bot.t * 2.0).fract() < 0.2 { 1 << 7 } else { 0 }, 0, ky, kp, view);
                    return;
                }
                let key = slot.map_or(0, |s| 1u32 << (8 + s));
                controls.apply_remote(if slot.is_some() && (bot.t * 2.0).fract() < 0.3 { key } else { 0 }, 0, ky, kp, view);
                return;
            }
            // Then at the nearest closed door, facing it: the door his E and attack act on (the server's focus) here, his
            // prompt's best.
            if i == STEPS.len() + 2 {
                if bot.t < 2.0 {
                    if nj.state != "Hunting" {
                        bot.t = 0.0;
                    }
                    return;
                }
                let Some((de, at_door, out)) = bot.door.and_then(|d| doors.get(d).ok()).or_else(|| doors.iter().filter(|(_, d, _)| !d.open && !d.broken).min_by(|a, b| a.2.transform_point(a.1.interact).distance(at).total_cmp(&b.2.transform_point(b.1.interact).distance(at)))).map(|(e, d, g)| (e, g.transform_point(d.interact), (g.rotation() * Vec3::X).with_y(0.0).normalize_or(Vec3::X))) else {
                    bot.failed.push("no door");
                    bot.step += 1;
                    return;
                };
                if bot.door.is_none() {
                    bot.door = Some(de);
                    let stand = at_door + out * 1.0;
                    crate::bot::setup(crate::bot::BotSetup::PlaceJason([stand.x, at.y, stand.z]));
                    info!("bot: net: Jason to the door at {at_door}");
                }
                let to = at_door - (at + Vec3::Y * 0.88);
                let (ky, kp) = ((-to.x).atan2(-to.z), to.y.atan2(Vec2::new(to.x, to.z).length()));
                controls.yaw = ky;
                controls.pitch = kp;
                for mut cam in &mut cameras {
                    cam.yaw = ky;
                    cam.pitch = kp;
                }
                controls.apply_remote(0, 0, ky, kp, view);
                if jasons.iter().any(|j| j.focus_door() == Some(de)) {
                    info!("bot: net: his focused door here");
                    bot.step += 1;
                    bot.t = 0.0;
                } else if bot.t > 12.0 {
                    warn!("bot: net: no focused door here");
                    bot.failed.push("focused door");
                    bot.step += 1;
                    bot.t = 0.0;
                }
                return;
            }
            // Then stunned (the server's hook): his stun's screen effect here.
            if i == STEPS.len() + 3 {
                if !bot.stun_asked {
                    bot.stun_asked = true;
                    crate::bot::setup(crate::bot::BotSetup::StunJason);
                }
                if jasons.iter().any(|j| j.stunned()) && !stun_views.is_empty() {
                    info!("bot: net: his stun here (its screen effect)");
                    bot.step += 1;
                    bot.t = 0.0;
                } else if bot.t > 10.0 {
                    warn!("bot: net: no stun here (state {}, effect {})", nj.state, !stun_views.is_empty());
                    bot.failed.push("stun effect");
                    bot.step += 1;
                    bot.t = 0.0;
                }
                return;
            }
            let Some(&(name, key)) = STEPS.get(i) else {
                // (His HUD's Shift and Stalk bars, from the server's.)
                if !bot.bars_seen.0 {
                    bot.failed.push("Shift bar");
                }
                if !bot.bars_seen.1 {
                    bot.failed.push("Stalk bar");
                }
                if bot.bars_seen.0 && bot.bars_seen.1 && bot.failed.is_empty() {
                    info!("bot: net: his Shift and Stalk bars here");
                }
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
