//! `--bot-escape`: a scripted counselor that plays the car escape end to end through the normal input and
//! interaction systems (it presses the same keys a player would). Without navigation it teleports between goals:
//! find the keys and each part the car needs (searching cabinets on the way), install the parts, get in, start it
//! with the keys, then drive into a car escape zone. Logs every step; exits when it escapes (or gives up).
//!
//! `--bot-heal`: fetch a first aid spray and use it (start hurt with `--health`); exits when healed.
//! `--bot-door`: doors, barricades and windows ([`FX_STEPS`]): open/close/lock/unlock the nearest lockable door,
//! barricade the nearest barred door and check it holds from both sides, then open a window, climb in, close it,
//! dive out through the glass (20 damage) and climb back in; screenshots to temp f13-bot-fixture-*.png.
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
    Door,
    /// Stand still and let Jason find, grab and kill the counselor.
    Jason,
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
    /// Door test step `i` (see [`DOOR_STEPS`]).
    Door(usize),
    /// Waiting for Jason.
    AwaitJason,
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
    /// The first tap of a double tap was pressed last frame.
    second_tap: bool,
    /// Keep E down across frames (a hold) instead of re-pressing it each frame.
    hold_e: bool,
    /// Fixture test targets: lockable door, barricaded door, window.
    fixtures: [Option<Entity>; 3],
    door_ok: bool,
    /// Jason states already screenshotted.
    jason_shots: Vec<&'static str>,
}

impl Default for Bot {
    fn default() -> Self {
        Self { goal: Goal::Escape, step: Step::Start, vehicle: None, timer: 0.0, total: 0.0, placed_car: false, shot: false, dodge_from: None, dodges_ok: 0, second_tap: false, hold_e: false, fixtures: [None; 3], door_ok: false, jason_shots: Vec::new() }
    }
}

/// What a fixture step acts on.
#[derive(Clone, Copy, PartialEq, Debug)]
enum Fx {
    Door,
    BarredDoor,
    Window,
}

/// State the fixture checks look at, taken after each step.
struct Snap {
    door_open: bool,
    door_locked: bool,
    barred: bool,
    window_open: bool,
    window_broken: bool,
    /// The counselor is on the window's inside.
    inside: bool,
    /// The counselor's distance in front of the door's plane (negative: inside).
    past_door: f32,
    health: f32,
}

struct FxStep {
    what: &'static str,
    fx: Fx,
    /// Where to stand (None: stay where the last step left us, e.g. after going through a window).
    outside: Option<bool>,
    hold: bool,
    sprint: bool,
    /// Walk through the doorway instead of pressing E: Some(true) inwards, Some(false) back out.
    walk: Option<bool>,
    expect: fn(&Snap) -> bool,
}

const FX_STEPS: &[FxStep] = &[
    FxStep { what: "open from outside", fx: Fx::Door, outside: Some(true), hold: false, sprint: false, walk: None, expect: |s| s.door_open },
    FxStep { what: "close", fx: Fx::Door, outside: Some(true), hold: false, sprint: false, walk: None, expect: |s| !s.door_open },
    FxStep { what: "lock from inside", fx: Fx::Door, outside: Some(false), hold: true, sprint: false, walk: None, expect: |s| s.door_locked },
    FxStep { what: "a press does not open it", fx: Fx::Door, outside: Some(false), hold: false, sprint: false, walk: None, expect: |s| !s.door_open },
    FxStep { what: "unlock from outside", fx: Fx::Door, outside: Some(true), hold: true, sprint: false, walk: None, expect: |s| !s.door_locked },
    FxStep { what: "open it", fx: Fx::Door, outside: Some(true), hold: false, sprint: false, walk: None, expect: |s| s.door_open },
    FxStep { what: "walk in through the doorway", fx: Fx::Door, outside: Some(true), hold: false, sprint: false, walk: Some(true), expect: |s| s.past_door < -0.6 },
    FxStep { what: "walk back out", fx: Fx::Door, outside: None, hold: false, sprint: false, walk: Some(false), expect: |s| s.past_door > 0.6 },
    FxStep { what: "barricade from inside", fx: Fx::BarredDoor, outside: Some(false), hold: true, sprint: false, walk: None, expect: |s| s.barred },
    FxStep { what: "barred: press from inside", fx: Fx::BarredDoor, outside: Some(false), hold: false, sprint: false, walk: None, expect: |s| !s.door_open },
    FxStep { what: "barred: press from outside", fx: Fx::BarredDoor, outside: Some(true), hold: false, sprint: false, walk: None, expect: |s| !s.door_open },
    FxStep { what: "remove barricade", fx: Fx::BarredDoor, outside: Some(false), hold: true, sprint: false, walk: None, expect: |s| !s.barred },
    FxStep { what: "open it", fx: Fx::BarredDoor, outside: Some(false), hold: false, sprint: false, walk: None, expect: |s| s.door_open },
    FxStep { what: "open window from outside", fx: Fx::Window, outside: Some(true), hold: false, sprint: false, walk: None, expect: |s| s.window_open },
    FxStep { what: "climb in", fx: Fx::Window, outside: None, hold: false, sprint: false, walk: None, expect: |s| s.inside && s.health == 100.0 },
    FxStep { what: "close it from inside", fx: Fx::Window, outside: None, hold: true, sprint: false, walk: None, expect: |s| !s.window_open },
    FxStep { what: "dive out through the glass", fx: Fx::Window, outside: None, hold: false, sprint: true, walk: None, expect: |s| !s.inside && s.window_broken && s.health == 80.0 },
    FxStep { what: "climb back in over the broken glass", fx: Fx::Window, outside: None, hold: false, sprint: false, walk: None, expect: |s| s.inside },
];

const CAPSULE: f13_core::collision::Capsule =
    f13_core::collision::Capsule { radius: crate::player::CAPSULE_RADIUS_M, half_height: crate::player::CAPSULE_HALF_HEIGHT_M };

/// Where to stand to reach `point` (an interact point) within `reach_m` of the chest: the nearest floor spot, on rings
/// around it, where the counselor's capsule fits (the collision would push her out of furniture and walls).
fn stand_spot(world: &(impl f13_core::collision::TriSource + ?Sized), point: Vec3, reach_m: f32) -> Option<Vec3> {
    for ring in 0..=10 {
        let r = ring as f32 * 0.15;
        let n = if ring == 0 { 1 } else { 16 };
        for k in 0..n {
            let a = k as f32 * std::f32::consts::TAU / n as f32;
            let (x, z) = (point.x + a.cos() * r, point.z + a.sin() * r);
            let Some((y, normal)) = f13_core::collision::surface_below(world, x, z, point.y + 0.3, point.y - 2.5) else { continue };
            let feet = Vec3::new(x, y, z);
            let chest = feet + Vec3::Y * crate::interact::CHEST_M;
            if normal.y >= crate::player::WALK.walkable_normal_y
                && chest.distance(point) <= reach_m
                && f13_core::nav::capsule_fits(world, feet, &CAPSULE, &crate::player::WALK)
            {
                return Some(feet);
            }
        }
    }
    None
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
    (time, real): (Res<Time>, Res<Time<Real>>),
    mut keys: ResMut<ButtonInput<KeyCode>>,
    mut mouse: ResMut<ButtonInput<MouseButton>>,
    (interaction, tuning): (Res<crate::interact::Interaction>, Res<crate::player::PlayerTuning>),
    mut player: Single<(&mut Transform, &Player, Option<&Inventory>, Has<Driving>, Has<UsingItem>)>,
    mut camera: Single<&mut PlayerCamera>,
    items: Query<(Entity, &Item, &GlobalTransform, Option<&InCabinet>), Without<Carried>>,
    carried: Query<&Item, With<Carried>>,
    cabinets: Query<(&Cabinet, &GlobalTransform)>,
    mut vehicles: Query<(Entity, &Vehicle, &GlobalTransform, &mut Transform), Without<Player>>,
    (zones, doors, windows, barricades, jasons): (
        Query<(&EscapeZone, &GlobalTransform)>,
        Query<(Entity, &crate::doors::Door, &GlobalTransform)>,
        Query<(Entity, &crate::windows::Window, &GlobalTransform)>,
        Query<&crate::windows::Barricade>,
        Query<(&crate::jason::Jason, &Transform), (Without<Player>, Without<Vehicle>)>,
    ),
    escaped: Option<Res<Escaped>>,
    mut exit: MessageWriter<AppExit>,
    (collision, dynamic): (Res<crate::collision::WorldCollision>, Res<crate::collision::DynamicColliders>),
) {
    let world = collision.with(&dynamic);
    let dt = time.delta_secs();
    bot.timer += dt;
    bot.total += dt;
    mouse.release(MouseButton::Left);
    mouse.release(MouseButton::Right);
    if !bot.hold_e {
        keys.release(KeyCode::KeyE);
    }
    for k in [KeyCode::KeyW, KeyCode::Digit1, KeyCode::Digit2, KeyCode::Digit3, KeyCode::KeyC, KeyCode::KeyA, KeyCode::KeyS, KeyCode::KeyD] {
        keys.release(k);
    }
    let limit = if bot.goal == Goal::Jason { 600.0 } else { 240.0 };
    if bot.total > limit && bot.step != Step::Done {
        error!("bot: gave up at {:?}", bot.step);
        exit.write(AppExit::error());
        return;
    }
    let (ref mut pxf, p, inv, driving, using) = *player;
    match bot.step {
        Step::Start => {
            // Let the map finish loading (in real time: frames hitch while assets stream in, whatever --speed).
            if bot.timer < 3.0 || real.elapsed_secs() < 6.0 {
                return;
            }
            if bot.goal == Goal::Door {
                // Nearest lockable door, nearest door with a barricade, nearest window.
                let me = pxf.translation;
                let near = |p: Vec3| p.distance(me);
                let barred = |e: Entity| barricades.iter().any(|b| b.door == Some(e));
                let pick = |want_bar: bool| {
                    doors.iter().filter(|d| barred(d.0) == want_bar).min_by(|a, b| near(a.2.translation()).total_cmp(&near(b.2.translation()))).map(|d| d.0)
                };
                let window = windows.iter().min_by(|a, b| near(a.2.translation()).total_cmp(&near(b.2.translation()))).map(|w| w.0);
                bot.fixtures = [pick(false), pick(true), window];
                info!("bot: fixtures {:?}", bot.fixtures);
                bot.step = Step::Door(0);
                bot.timer = 0.0;
                return;
            }
            if bot.goal == Goal::Jason {
                match jasons.iter().next() {
                    Some((_, jxf)) => info!("bot: waiting for Jason, {:.0} m away", jxf.translation.distance(pxf.translation)),
                    None => {
                        error!("bot: no Jason");
                        bot.step = Step::Done;
                        return;
                    }
                }
                bot.step = Step::AwaitJason;
                bot.timer = 0.0;
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
            if !go_fetch(&world, &mut bot, &mut camera, &parts, near, pxf, &items, &cabinets, &mut keys) {
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
            if !go_fetch(&world, &mut bot, &mut camera, &[FIRST_AID.to_string()], near, pxf, &items, &cabinets, &mut keys) {
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
            look_at(&mut camera, pxf, at);
            if bot.timer > 0.3 {
                keys.press(KeyCode::KeyE);
            }
            // Play the pips: press each one's key when the needle is in its middle.
            if let Some(h) = interaction.hold.as_ref().filter(|h| h.started) {
                if let Some(p) = h.current.map(|c| h.pips[c]) {
                    if h.progress(&tuning.minigame) >= (p.position + p.width / 2.0) / 360.0 {
                        mouse.press(if p.key == 0 { MouseButton::Right } else { MouseButton::Left });
                    }
                }
            }
            if bot.timer > 60.0 {
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
        Step::Door(i) => {
            if i == FX_STEPS.len() {
                bot.door_ok = true;
                bot.step = Step::Done;
                return;
            }
            let step = &FX_STEPS[i];
            let target = bot.fixtures[step.fx as usize];
            let Some((interact, gxf)) = target.and_then(|e| match step.fx {
                Fx::Window => windows.get(e).ok().map(|(_, w, x)| (w.interact, *x)),
                _ => doors.get(e).ok().map(|(_, d, x)| (d.interact, *x)),
            }) else {
                error!("bot: no fixture for {:?}", step.fx);
                bot.step = Step::Done;
                return;
            };
            // Stand 0.8 m from the interact point on the chosen side (outside = the fixture's forward side).
            let at = gxf.transform_point(interact);
            let fwd = (gxf.rotation() * Vec3::X).with_y(0.0).normalize_or_zero();
            let past_door = (pxf.translation - gxf.translation()).dot(fwd);
            if let Some(inwards) = step.walk {
                // From 1 m out, walk at the doorway's centre (the actor origin) to 3 m beyond it.
                let side = if inwards { 1.0 } else { -1.0 };
                if step.outside.is_some() && bot.timer < 0.5 {
                    place(pxf, gxf.translation() + fwd * side);
                }
                look_at(&mut camera, pxf, gxf.translation() - fwd * side * 3.0 + Vec3::Y);
                if bot.timer < 0.5 {
                    return;
                }
                if bot.timer < 3.0 {
                    keys.press(KeyCode::KeyW);
                    return;
                }
                keys.release(KeyCode::KeyW);
            }
            if let (Some(outside), None) = (step.outside, step.walk) {
                let stand = Vec3::new(at.x, gxf.translation().y, at.z) + fwd * if outside { 0.8 } else { -0.8 };
                if bot.timer < 0.5 {
                    place(pxf, stand);
                }
            }
            if step.walk.is_none() {
                look_at(&mut camera, pxf, at);
            }
            if step.sprint && bot.timer < 2.0 && step.walk.is_none() {
                keys.press(KeyCode::ShiftLeft);
            }
            if bot.timer < 0.5 {
                return;
            }
            if step.hold && bot.timer < 2.0 && step.walk.is_none() {
                keys.press(KeyCode::KeyE);
                bot.hold_e = true;
                return;
            }
            bot.hold_e = false;
            if !step.hold && !bot.second_tap && step.walk.is_none() {
                keys.press(KeyCode::KeyE); // one frame: released next frame, so it reads as a press
                bot.second_tap = true;
                return;
            }
            if bot.timer < 2.5 {
                return;
            }
            keys.release(KeyCode::ShiftLeft);
            let door = target.filter(|_| step.fx != Fx::Window).and_then(|e| doors.get(e).ok());
            let window = bot.fixtures[2].and_then(|e| windows.get(e).ok());
            let snap = Snap {
                door_open: door.is_some_and(|d| d.1.open),
                door_locked: door.is_some_and(|d| d.1.locked),
                barred: door.is_some_and(|d| barricades.iter().any(|b| b.door == Some(d.0) && b.enabled)),
                window_open: window.is_some_and(|w| w.1.open),
                window_broken: window.is_some_and(|w| w.1.destroyed),
                inside: window.is_some_and(|w| crate::windows::Window::is_inside(w.2, pxf.translation)),
                past_door,
                health: p.counselor.health,
            };
            let ok = (step.expect)(&snap);
            info!(
                "bot: {:?} step {i} ({}): door open {} locked {} barred {} past {:.1} | window open {} broken {} inside {} | health {:.0} -> {}",
                step.fx,
                step.what,
                snap.door_open,
                snap.door_locked,
                snap.barred,
                snap.past_door,
                snap.window_open,
                snap.window_broken,
                snap.inside,
                snap.health,
                if ok { "ok" } else { "WRONG" }
            );
            if !ok {
                bot.step = Step::Done;
                return;
            }
            if matches!(i, 0 | 7 | 16) {
                let path = std::env::temp_dir().join(format!("f13-bot-fixture-{i}.png"));
                commands.spawn(Screenshot::primary_window()).observe(save_to_disk(path));
            }
            bot.step = Step::Door(i + 1);
            bot.second_tap = false;
            bot.timer = 0.0;
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
                    bot.second_tap = true;
                }
                // second tap on the next frame (all keys are released in between), whatever the game speed
                Some(_) if bot.second_tap => {
                    keys.press(key);
                    bot.second_tap = false;
                }
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
        Step::AwaitJason => {
            let Some((j, jxf)) = jasons.iter().next() else { return };
            if bot.timer >= 5.0 {
                bot.timer = 0.0;
                info!("bot: Jason {:?}, {:.1} m away; health {:.0}", j.state, jxf.translation.distance(pxf.translation), p.counselor.health);
            }
            // Up close, watch from Jason's side (to check the grab and kill poses).
            if jxf.translation.distance(pxf.translation) < 3.0 {
                let (yaw, _, _) = jxf.rotation.to_euler(EulerRot::YXZ);
                let right = Vec3::new(-yaw.cos(), 0.0, yaw.sin());
                camera.yaw = (-right.x).atan2(-right.z);
                camera.pitch = -0.15;
            }
            // Screenshots of the hit, the grab and the kill, to check the paired poses.
            let shot = match j.state {
                crate::jason::JasonState::Swinging { t, door: None, .. } if t > 0.35 => Some("swing"),
                crate::jason::JasonState::Holding { t } if t > 1.0 => Some("grab"),
                crate::jason::JasonState::Killing { t } if t > 9.0 => Some("kill2"),
                crate::jason::JasonState::Killing { t } if t > 4.0 => Some("kill"),
                _ => None,
            };
            if let Some(s) = shot.filter(|s| !bot.jason_shots.contains(s)) {
                bot.jason_shots.push(s);
                info!("bot: Vanessa's feet {:.2} m above Jason's", pxf.translation.y - jxf.translation.y);
                let path = std::env::temp_dir().join(format!("f13-bot-jason-{s}.png"));
                info!("bot: screenshot {}", path.display());
                commands.spawn(Screenshot::primary_window()).observe(save_to_disk(path));
            }
            if !p.counselor.alive() {
                info!("bot: Vanessa died after {:.1}s ({:?})", bot.total, j.state);
                bot.step = Step::Done;
                bot.timer = 0.0;
            }
        }
        Step::Done => {
            if bot.timer > 2.0 {
                let ok = match bot.goal {
                    Goal::Escape => escaped.is_some(),
                    Goal::Heal => p.counselor.health >= 100.0,
                    Goal::Dodge => bot.dodges_ok == 3,
                    Goal::Door => bot.door_ok,
                    Goal::Jason => !p.counselor.alive(),
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
    world: &(impl f13_core::collision::TriSource + ?Sized),
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
        let stand = stand_spot(world, at, cab.reach / 100.0).unwrap_or(at - Vec3::Y * 1.0);
        if pxf.translation.distance(stand) > 0.05 {
            place(pxf, stand);
        }
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
            Some((cab, cxf)) => {
                let point = cxf.transform_point(cab.interact);
                stand_spot(world, point, cab.reach / 100.0).unwrap_or(point - Vec3::Y * 1.0)
            }
            None => {
                let back = (pxf.translation - at).with_y(0.0).try_normalize().unwrap_or(Vec3::X);
                let short = at + back * 0.7;
                if f13_core::nav::capsule_fits(world, short, &CAPSULE, &crate::player::WALK) {
                    short
                } else {
                    stand_spot(world, at, item.reach / 100.0).unwrap_or(short)
                }
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
