//! The host's commands (`--host`, `--host-port n`; the `f13-host` CLI sends them): a line-based TCP channel on 127.0.0.1 only.
//! One command per line, one reply line each:
//!
//! - `status`: the match state and time, where the counselor and Jason are, her health
//! - `give <Class>`: puts one of the match's items of that class in her hands or pockets (as `--give`)
//! - `teleport counselor|jason <x> <y> <z>`: moves them (Bevy metres; she's put on the floor under the point)
//! - `start`: starts the match now (from waiting); `end`: ends it now (to the outro, with the end-of-match awards)
//! - `kick`: the counselor leaves (the game closes; it's single-player)
//! - `jason`: makes the player Jason (his controls and camera; the counselor is left without a player)

use crate::jason::Jason;
use crate::player::{Player, Unsettled};
use bevy::prelude::*;
use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::Mutex;

pub struct HostPlugin {
    pub port: u16,
}

/// A command and where its reply goes.
type Request = (String, Sender<String>);

#[derive(Resource)]
struct Inbox(Mutex<Receiver<Request>>);

/// Match changes the host asked for (match_flow.rs applies them in its tick).
#[derive(Resource, Default)]
pub struct HostRequests {
    pub start: bool,
    pub end: bool,
}

impl Plugin for HostPlugin {
    fn build(&self, app: &mut App) {
        let (tx, rx) = channel::<Request>();
        let port = self.port;
        match TcpListener::bind(("127.0.0.1", port)) {
            Ok(listener) => {
                info!("host: listening on 127.0.0.1:{port}");
                std::thread::spawn(move || {
                    for stream in listener.incoming().flatten() {
                        let tx = tx.clone();
                        std::thread::spawn(move || {
                            let Ok(mut out) = stream.try_clone() else { return };
                            for line in BufReader::new(stream).lines().map_while(Result::ok) {
                                let (reply_tx, reply_rx) = channel();
                                if tx.send((line, reply_tx)).is_err() {
                                    return;
                                }
                                let reply = reply_rx.recv().unwrap_or_else(|_| "error: the game closed".into());
                                if writeln!(out, "{reply}").is_err() {
                                    return;
                                }
                            }
                        });
                    }
                });
            }
            Err(e) => error!("host: can't listen on 127.0.0.1:{port}: {e}"),
        }
        app.insert_resource(Inbox(Mutex::new(rx))).init_resource::<HostRequests>().add_systems(Update, serve);
    }
}

#[allow(clippy::type_complexity)]
fn serve(
    mut commands: Commands,
    inbox: Res<Inbox>,
    mut requests: ResMut<HostRequests>,
    game: Option<Res<crate::match_flow::GameMatch>>,
    mut players: Query<(Entity, &Player, &mut Transform), Without<Jason>>,
    mut jasons: Query<(&mut Jason, &mut Transform), Without<Player>>,
    mut exit: MessageWriter<AppExit>,
    play: Option<Res<crate::jason_player::PlayJason>>,
) {
    let Ok(inbox) = inbox.0.lock() else { return };
    while let Ok((line, reply)) = inbox.try_recv() {
        let words: Vec<&str> = line.split_whitespace().collect();
        let point = |w: &[&str]| -> Option<Vec3> {
            let v: Vec<f32> = w.iter().filter_map(|x| x.trim_matches(',').parse().ok()).collect();
            (v.len() == 3).then(|| Vec3::new(v[0], v[1], v[2]))
        };
        let answer = match words.as_slice() {
            ["status"] => {
                let m = game.as_ref().map_or("no match".to_string(), |g| {
                    format!("{:?}, {} s left, {} s played", g.state.state, g.state.remaining_time, g.state.elapsed_in_progress_time)
                });
                let her = players.iter().next().map_or("no counselor".into(), |(_, p, xf)| {
                    format!("counselor at {:.1} health {:.0}{}", xf.translation, p.counselor.health, if p.counselor.alive() { "" } else { " (dead)" })
                });
                let him = jasons.iter().next().map_or("no Jason".into(), |(j, xf)| format!("Jason at {:.1} ({:?}, {} knives)", xf.translation, j.state, j.knives));
                format!("{m}; {her}; {him}")
            }
            ["give", class] => {
                commands.insert_resource(crate::items::Give(vec![class.to_string()]));
                format!("giving {class} (if the match has one free)")
            }
            ["teleport", who, rest @ ..] => match (point(rest), *who) {
                (Some(p), "counselor") => match players.iter_mut().next() {
                    Some((e, _, mut xf)) => {
                        xf.translation = p;
                        commands.entity(e).insert(Unsettled);
                        format!("counselor to {p}")
                    }
                    None => "no counselor".into(),
                },
                (Some(p), "jason") => match jasons.iter_mut().next() {
                    Some((mut j, mut xf)) => {
                        xf.translation = p;
                        j.teleported();
                        format!("Jason to {p}")
                    }
                    None => "no Jason".into(),
                },
                _ => "usage: teleport counselor|jason <x> <y> <z>".into(),
            },
            ["start"] => {
                requests.start = true;
                "starting the match".into()
            }
            ["end"] => {
                requests.end = true;
                "ending the match".into()
            }
            ["kick"] => {
                exit.write(AppExit::Success);
                "kicked: the game closes".into()
            }
            ["jason"] => match jasons.iter().next() {
                Some(_) if play.is_none() => {
                    crate::jason_player::become_jason(&mut commands);
                    "you're Jason now".into()
                }
                Some(_) => "already Jason".into(),
                None => "no Jason in this match".into(),
            },
            _ => "commands: status, give <Class>, teleport counselor|jason <x> <y> <z>, start, end, kick, jason".into(),
        };
        info!("host: {line} -> {answer}");
        let _ = reply.send(answer);
    }
}
