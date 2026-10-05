//! `--bot-escape`: a scripted counselor that plays the car escape end to end through the normal input and
//! interaction systems (it presses the same keys a player would). Without navigation it teleports between goals:
//! find the keys and each part the car needs (searching cabinets on the way), install the parts, get in, start it
//! with the keys, then drive into a car escape zone. Logs every step; exits when it escapes (or gives up).

use crate::interact::{Carried, Inventory};
use crate::player::{Player, PlayerCamera};
use crate::props::{Cabinet, EscapeZone, InCabinet, Item, Vehicle};
use crate::vehicle::{Driving, Escaped};
use bevy::input::InputSystems;
use bevy::prelude::*;
use bevy::render::view::screenshot::{save_to_disk, Screenshot};

pub struct BotPlugin;

impl Plugin for BotPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Bot>().add_systems(PreUpdate, run_bot.after(InputSystems));
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
    Done,
}

#[derive(Resource)]
struct Bot {
    step: Step,
    vehicle: Option<Entity>,
    timer: f32,
    total: f32,
    placed_car: bool,
    shot: bool,
}

impl Default for Bot {
    fn default() -> Self {
        Self { step: Step::Start, vehicle: None, timer: 0.0, total: 0.0, placed_car: false, shot: false }
    }
}

fn place(player: &mut Transform, at: Vec3) {
    player.translation = at;
}

#[allow(clippy::too_many_arguments)]
fn run_bot(
    mut commands: Commands,
    mut bot: ResMut<Bot>,
    time: Res<Time>,
    mut keys: ResMut<ButtonInput<KeyCode>>,
    mut player: Single<(&mut Transform, Option<&Inventory>, Has<Driving>), With<Player>>,
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
    for k in [KeyCode::KeyE, KeyCode::KeyW] {
        keys.release(k);
    }
    if bot.total > 240.0 && bot.step != Step::Done {
        error!("bot: gave up at {:?}", bot.step);
        exit.write(AppExit::error());
        return;
    }
    let (ref mut pxf, inv, driving) = *player;
    match bot.step {
        Step::Start => {
            if bot.timer < 3.0 {
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
            // Nearest matching part to the car.
            let Some((e, item, ixf, in_cab)) = items
                .iter()
                .filter(|(_, it, _, _)| parts.contains(&it.class))
                .min_by(|a, b| a.2.translation().distance(vxf.translation()).total_cmp(&b.2.translation().distance(vxf.translation())))
            else {
                error!("bot: no {:?} anywhere", parts);
                bot.step = Step::Done;
                return;
            };
            let closed_cabinet = in_cab.and_then(|c| cabinets.get(c.0).ok()).filter(|(c, _)| !c.open);
            if let Some((cab, cxf)) = closed_cabinet {
                // Stand at the cabinet's interact point (feet a metre below it) and search it.
                place(pxf, cxf.transform_point(cab.interact) - Vec3::Y * 1.0);
                if bot.timer > 0.3 {
                    info!("bot: searching cabinet for {}", item.name);
                    keys.press(KeyCode::KeyE);
                    bot.timer = 0.0;
                }
            } else {
                place(pxf, ixf.translation());
                if bot.timer > 0.3 {
                    info!("bot: picking up {} ({e:?})", item.name);
                    keys.press(KeyCode::KeyE);
                    bot.timer = 0.0;
                }
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
        Step::Done => {
            if bot.timer > 2.0 {
                exit.write(if escaped.is_some() { AppExit::Success } else { AppExit::error() });
            }
        }
    }
}
