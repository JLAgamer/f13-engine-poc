//! `--bot-escape`: a scripted counselor that plays the car escape end to end through the normal input and
//! interaction systems (it presses the same keys a player would). Without navigation it teleports between goals:
//! find the keys and each part the car needs (searching cabinets on the way), install the parts, get in, start it
//! with the keys, then drive into a car escape zone. Logs every step; exits when it escapes (or gives up).
//!
//! `--bot-heal`: fetch a first aid spray and use it (start hurt with `--health`); exits when healed.
//! `--bot-dodge`: enter combat stance and double-tap S, A, D; logs how far each dodge moved (needs open ground).

use crate::interact::{Carried, Inventory, UsingItem, FIRST_AID};
use crate::player::{Player, PlayerCamera};

use crate::props::{Cabinet, EscapeZone, InCabinet, Item, Vehicle};
use crate::vehicle::{Driving, Escaped};
use bevy::input::InputSystems;
use bevy::prelude::*;
use bevy::render::view::screenshot::{save_to_disk, Screenshot};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Goal {
    Escape,
    Heal,
    Dodge,
}

pub struct BotPlugin(pub Goal);

impl Plugin for BotPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(Bot { goal: self.0, ..default() }).add_systems(PreUpdate, run_bot.after(InputSystems));
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Step {
    /// Waiting for the match to spawn.
    Start,
    /// Fetch the part for repair point `i`.
    Fetch(usize),
    /// Install it (hold E).
    Install(usize),
    GetIn,
    Drive,
    /// Fetch a first aid spray, then use it.
    FetchAid,
    Heal,
    /// Dodge number `i` (0 back, 1 left, 2 right).
    Dodge(usize),
    Done,
}

#[derive(Resource)]
struct Bot {
    goal: Goal,
    step: Step,
    vehicle: Option<Entity>,
    timer: f32,
    total: f32,
    placed_car: bool,
    shot: bool,
    /// Where the current dodge started, and the stamina then.
    dodge_from: Option<(Vec3, f32)>,
    dodges_ok: usize,
}

impl Default for Bot {
    fn default() -> Self {
        Self { goal: Goal::Escape, step: Step::Start, vehicle: None, timer: 0.0, total: 0.0, placed_car: false, shot: false, dodge_from: None, dodges_ok: 0 }
    }
}

fn place(player: &mut Transform, at: Vec3) {
    player.translation = at;
}

/// Points the camera from the counselor's chest at `target` (interaction prefers what the camera looks at).
fn look_at(camera: &mut PlayerCamera, player: &Transform, target: Vec3) {
    if let Some(d) = (target - (player.translation + Vec3::Y)).try_normalize() {
        camera.yaw = (-d.x).atan2(-d.z);
        camera.pitch = d.y.asin();
    }
}

#[allow(clippy::too_many_arguments)]
fn run_bot(
    mut commands: Commands,
    mut bot: ResMut<Bot>,
    time: Res<Time>,
    mut keys: ResMut<ButtonInput<KeyCode>>,
    mut player: Single<(&mut Transform, &Player, Option<&Inventory>, Has<Driving>, Has<UsingItem>)>,
    mut camera: Single<&mut PlayerCamera>,
    items: Query<(Entity, &Item, &GlobalTransform, Option<&InCabinet>), Without<Carried>>,
    carried: Query<&Item, With<Carried>>,
    cabinets: Query<(&Cabinet, &GlobalTransform)>,
    mut vehicles: Query<(Entity, &Vehicle, &GlobalTransform, &mut Transform), Without<Player>>,
    zones: Query<(&EscapeZone, &GlobalTransform)>,
    escaped: Option<Res<Escaped>>,
    mut exit: MessageWriter<AppExit>,
) {
    let dt = time.delta_secs();
    bot.timer += dt;
    bot.total += dt;
    for k in [KeyCode::KeyE, KeyCode::KeyW, KeyCode::Digit1, KeyCode::Digit2, KeyCode::Digit3, KeyCode::KeyC, KeyCode::KeyA, KeyCode::KeyS, KeyCode::KeyD] {
        keys.release(k);
    }
    if bot.total > 240.0 && bot.step != Step::Done {
        error!("bot: gave up at {:?}", bot.step);
        exit.write(AppExit::error());
        return;
    }
    let (ref mut pxf, p, inv, driving, using) = *player;
    match bot.step {
        Step::Start => {
            if bot.timer < 3.0 {
                return;
            }
            if bot.goal == Goal::Dodge {
                info!("bot: combat stance");
                keys.press(KeyCode::KeyC);
                bot.step = Step::Dodge(0);
                bot.timer = 0.0;
                return;
            }
            if bot.goal == Goal::Heal {
                info!("bot: health {:.0}, going for first aid", p.counselor.health);
                bot.step = Step::FetchAid;
                bot.timer = 0.0;
                return;
            }
            let Some((e, v, _, _)) = vehicles.iter().find(|(_, v, _, _)| v.class.contains("2Seat")).or_else(|| vehicles.iter().next()) else { return };
            info!("bot: going for {} (needs {:?})", v.class, v.repairs.iter().map(|r| &r.parts).collect::<Vec<_>>());
            bot.vehicle = Some(e);
            bot.step = Step::Fetch(0);
            bot.timer = 0.0;
        }
        Step::Fetch(i) => {
            let Some(inv) = inv else { return };
            let Ok((_, v, vxf, _)) = vehicles.get(bot.vehicle.unwrap()) else { return };
            if i > v.repairs.len() {
                bot.step = Step::GetIn;
                return;
            }
            // First the ignition keys (they stay in a pocket), then each part installed from outside.
            let parts = if i == 0 { &v.ignition } else { &v.repairs[i - 1].parts };
            let have = inv.hand.iter().chain(inv.pockets.iter()).any(|&e| carried.get(e).is_ok_and(|it| parts.contains(&it.class)));
            if have {
                info!("bot: have {:?}", parts);
                bot.step = if i == 0 { Step::Fetch(1) } else { Step::Install(i - 1) };
                bot.timer = 0.0;
                return;
            }
            let near = vxf.translation();
            let parts = parts.clone();
            if !go_fetch(&mut bot, &mut camera, &parts, near, pxf, &items, &cabinets, &mut keys) {
                bot.step = Step::Done;
            }
        }
        Step::FetchAid => {
            let Some(inv) = inv else { return };
            if inv.pockets.iter().any(|&e| carried.get(e).is_ok_and(|it| it.class == FIRST_AID)) {
                info!("bot: have first aid");
                bot.step = Step::Heal;
                bot.timer = 0.0;
                return;
            }
            let near = pxf.translation;
            if !go_fetch(&mut bot, &mut camera, &[FIRST_AID.to_string()], near, pxf, &items, &cabinets, &mut keys) {
                bot.step = Step::Done;
            }
        }
        Step::Heal => {
            let Some(inv) = inv else { return };
            if !using && bot.timer > 0.3 {
                if p.counselor.health >= 100.0 {
                    info!("bot: healed to {:.0} after {:.1}s", p.counselor.health, bot.total);
                    bot.step = Step::Done;
                    bot.timer = 0.0;
                    return;
                }
                let Some(slot) = inv.pockets.iter().position(|&e| carried.get(e).is_ok_and(|it| it.class == FIRST_AID)) else { return };
                info!("bot: using first aid (pocket {})", slot + 1);
                keys.press([KeyCode::Digit1, KeyCode::Digit2, KeyCode::Digit3][slot]);
                bot.timer = 0.0;
            }
        }
        Step::Install(i) => {
            let Ok((_, v, vxf, _)) = vehicles.get(bot.vehicle.unwrap()) else { return };
            if v.repairs[i].done {
                info!("bot: {} done after {:.1}s", v.repairs[i].name, bot.timer);
                bot.step = Step::Fetch(i + 2);
                bot.timer = 0.0;
                return;
            }
            let at = vxf.transform_point(v.repairs[i].at);
            let out = (at - vxf.translation()).with_y(0.0).normalize_or_zero();
            place(pxf, at + out * 0.5 - Vec3::Y * 0.8);
            if bot.timer > 0.3 {
                keys.press(KeyCode::KeyE);
            }
            if bot.timer > 10.0 {
                error!("bot: install {} not progressing", v.repairs[i].name);
                bot.step = Step::Done;
            }
        }
        Step::GetIn => {
            if driving {
                info!("bot: in the driver's seat");
                bot.step = Step::Drive;
                bot.timer = 0.0;
                return;
            }
            let Ok((_, v, vxf, _)) = vehicles.get(bot.vehicle.unwrap()) else { return };
            let seat = vxf.transform_point(v.driver_seat);
            let out = (seat - vxf.translation()).with_y(0.0).normalize_or_zero();
            place(pxf, seat + out * 0.6 - Vec3::Y * 0.9);
            if bot.timer > 0.3 {
                keys.press(KeyCode::KeyE);
                bot.timer = 0.0;
            }
        }
        Step::Drive => {
            if let Some(e) = escaped {
                info!("bot: escaped {} after {:.1}s", e.how, bot.total);
                bot.step = Step::Done;
                bot.timer = 0.0;
                return;
            }
            let Ok((_, v, _, mut vxf)) = vehicles.get_mut(bot.vehicle.unwrap()) else { return };
            if !v.started {
                keys.press(KeyCode::KeyE); // hold E at the ignition
                bot.timer = 0.0;
                return;
            }
            // Put the car 15 m short of the nearest car escape zone, facing it, then drive in.
            if !bot.placed_car {
                bot.placed_car = true;
                let car = vxf.translation;
                let Some((z, zxf)) = zones.iter().filter(|(z, _)| z.car).min_by(|a, b| a.1.translation().distance(car).total_cmp(&b.1.translation().distance(car))) else { return };
                let zone = zxf.translation();
                // Come along the road: from the trigger's side of the zone.
                let inward = (z.trigger - zone).with_y(0.0).normalize_or_zero();
                let start = zone + inward * 15.0;
                vxf.translation = start + Vec3::Y * 0.5;
                // Car forward is its local +X; turn it to face the zone.
                let to_zone = -inward;
                let yaw = (-to_zone.z).atan2(to_zone.x);
                vxf.rotation = Quat::from_rotation_y(yaw);
                camera.yaw = (-to_zone.x).atan2(-to_zone.z);
                info!("bot: driving into the escape zone at ({:.0}, {:.0}, {:.0})", zone.x, zone.y, zone.z);
            }
            keys.press(KeyCode::KeyW);
            if (bot.timer * 2.0) as u32 != ((bot.timer - dt) * 2.0) as u32 {
                let nearest = zones.iter().filter(|(z, _)| z.car).map(|(z, x)| (x.translation().distance(vxf.translation), z.radius_m)).min_by(|a, b| a.0.total_cmp(&b.0));
                info!("bot: car at ({:.1}, {:.1}, {:.1}) speed {:.0} cm/s, zone {:?}", vxf.translation.x, vxf.translation.y, vxf.translation.z, v.speed, nearest);
            }
            if bot.timer > 6.0 && v.speed == 0.0 && !bot.shot {
                bot.shot = true;
                let path = std::env::temp_dir().join("f13-bot-blocked.png");
                info!("bot: blocked; screenshot {}", path.display());
                commands.spawn(Screenshot::primary_window()).observe(save_to_disk(path));
            }
            if bot.timer > 20.0 {
                error!("bot: did not reach the escape zone");
                bot.step = Step::Done;
            }
        }
        Step::Dodge(i) => {
            if i == 3 {
                bot.step = Step::Done;
                return;
            }
            let key = [KeyCode::KeyS, KeyCode::KeyA, KeyCode::KeyD][i];
            match bot.dodge_from {
                None if bot.timer > 0.5 => {
                    bot.dodge_from = Some((pxf.translation, p.counselor.stamina));
                    bot.timer = 0.0;
                    keys.press(key);
                }
                // second tap two frames later (released in between)
                Some(_) if bot.timer > 0.0 && bot.timer < 0.05 && !p.counselor.dodging() => keys.press(key),
                Some((from, stamina)) if bot.timer > 1.2 => {
                    let moved = pxf.translation - from;
                    info!("bot: dodge {:?}: moved {:.2} m, stamina {:.1} -> {:.1}", ["back", "left", "right"][i], moved.with_y(0.0).length(), stamina, p.counselor.stamina);
                    if moved.with_y(0.0).length() > 1.5 && p.counselor.stamina < stamina {
                        bot.dodges_ok += 1;
                    }
                    bot.dodge_from = None;
                    bot.step = Step::Dodge(i + 1);
                    bot.timer = 0.0;
                }
                _ => {}
            }
        }
        Step::Done => {
            if bot.timer > 2.0 {
                let ok = match bot.goal {
                    Goal::Escape => escaped.is_some(),
                    Goal::Heal => p.counselor.health >= 100.0,
                    Goal::Dodge => bot.dodges_ok == 3,
                };
                exit.write(if ok { AppExit::Success } else { AppExit::error() });
            }
        }
    }
}

/// Moves to the nearest item of `classes` to `near` (searching its cabinet first) and presses E.
/// Returns false when there is none.
#[allow(clippy::too_many_arguments)]
fn go_fetch(
    bot: &mut Bot,
    camera: &mut PlayerCamera,
    classes: &[String],
    near: Vec3,
    pxf: &mut Transform,
    items: &Query<(Entity, &Item, &GlobalTransform, Option<&InCabinet>), Without<Carried>>,
    cabinets: &Query<(&Cabinet, &GlobalTransform)>,
    keys: &mut ButtonInput<KeyCode>,
) -> bool {
    let Some((e, item, ixf, in_cab)) = items
        .iter()
        .filter(|(_, it, _, _)| classes.contains(&it.class))
        .min_by(|a, b| a.2.translation().distance(near).total_cmp(&b.2.translation().distance(near)))
    else {
        error!("bot: no {:?} anywhere", classes);
        return false;
    };
    let cabinet = in_cab.and_then(|c| cabinets.get(c.0).ok());
    let closed_cabinet = cabinet.filter(|(c, _)| !c.open);
    if let Some((cab, cxf)) = closed_cabinet {
        // Stand at the cabinet's interact point (feet a metre below it) and search it.
        let at = cxf.transform_point(cab.interact);
        place(pxf, at - Vec3::Y * 1.0);
        look_at(camera, pxf, at);
        if bot.timer > 0.3 {
            info!("bot: searching cabinet for {}", item.name);
            keys.press(KeyCode::KeyE);
            bot.timer = 0.0;
        }
    } else {
        let at = ixf.translation();
        // In an opened cabinet: stand where it was searched from. Loose: 0.7 m short of it, coming from where we are.
        let stand = match cabinet {
            Some((cab, cxf)) => cxf.transform_point(cab.interact) - Vec3::Y * 1.0,
            None => {
                let back = (pxf.translation - at).with_y(0.0).try_normalize().unwrap_or(Vec3::X);
                at + back * 0.7
            }
        };
        if pxf.translation.distance(stand) > 0.05 {
            place(pxf, stand);
        }
        look_at(camera, pxf, at);
        if bot.timer > 0.3 {
            info!("bot: picking up {} ({e:?}) at ({:.1}, {:.1}, {:.1}), standing {:.2} m away", item.name, at.x, at.y, at.z, pxf.translation.distance(at));
            keys.press(KeyCode::KeyE);
            bot.timer = 0.0;
        }
    }
    true
}
