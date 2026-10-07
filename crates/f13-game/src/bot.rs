//! `--bot-escape`: a scripted counselor that plays the car escape end to end through the normal input and
//! interaction systems (it presses the same keys a player would). Without navigation it teleports between goals:
//! find the keys and each part the car needs (searching cabinets on the way), install the parts, get in, start it
//! with the keys, then drive into a car escape zone. Logs every step; exits when it escapes (or gives up).
//!
//! `--bot-police`: fetch the phone box's fuse, put it in, repair the box once it's broken (as Jason breaks it),
//! call the police on its phone, then (the countdown cut
//! to 10 s) walk into the escape volume they open; exits when she escapes.
//! `--bot-placed-kill`: wait at the placed kill nearest Jason, where he hears her: passes when he grabs her and kills
//! her with it (a placed kill: his place and her death on its paired takes, its own camera).
//! `--bot-heal`: fetch a first aid spray and use it (start hurt with `--health`); exits when healed.
//! `--bot-door`: doors, barricades and windows ([`FX_STEPS`]): open/close/lock/unlock the nearest lockable door,
//! barricade the nearest barred door and check it holds from both sides, then open a window, climb in, close it,
//! dive out through the glass (20 damage) and climb back in; screenshots to temp f13-bot-fixture-*.png.
//! `--bot-dodge`: enter combat stance and double-tap S, A, D; logs how far each dodge moved (needs open ground).
//! `--bot-hide`: hide in the nearest hiding spot while Jason is far off, hold her breath for 4 s, stay 60 s, come
//! out; passes alive, having held it (her breath fell, then came back).
//! `--bot-hide-seen`: let Jason see her, then hide in the hiding spot nearest him; passes when he finds and kills
//! her there. Screenshots f13-bot-hide-*.png.
//! `--bot-locked-in` / `--bot-barred-in`: shut her in behind the lockable (barricaded) door nearest Jason, locked
//! (barred), and tell him where she is; passes when he breaks it down in a hit per 20 of its health (10 barred) and
//! kills her; `--bot-raged-in`: a locked door with Jason raged, who bursts through it. Screenshots
//! f13-bot-breakdown-*.png.
//! `--bot-trap`: with a bear trap (`--give BearTrap_C`), place it, step back and walk into it, then wiggle free:
//! passes when it caught her (50 damage) and she got out. Screenshot f13-bot-trap.png.
//! `--bot-block`: with a weapon (`--give BaseballBat_C`), in combat stance, block Jason's swings: passes when a
//! blocked hit has done less than a full one and he has killed her.
//! `--bot-abilities`: Jason starts with every ability ready and hears where she stands, across the map: passes when
//! he has Morphed near her, Stalked and Shifted at her, and killed her. Screenshot f13-bot-abilities-shift.png
//! (mid-shift: he's faded out).
//! With Jason (`--bot-jason`, `--bot-fight`, the hide bots), she steps out 10 m in front of him when he hasn't seen
//! her for a while (he only knows what he sees, hears and senses).

use crate::interact::{Carried, Inventory, UsingItem, FIRST_AID};
use crate::player::{Player, PlayerCamera};

use crate::props::{Cabinet, EscapeZone, InCabinet, Item, Vehicle};
use crate::match_flow::MatchState;
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
    /// Stand still and use the items she was given (--give) on Jason, and struggle when grabbed: passes when he is
    /// stunned (or she breaks free) and recovers.
    Fight,
    /// Hide out of Jason's sight and come out alive.
    Hide,
    /// Hide where Jason can see her: he should find and kill her.
    HideSeen,
    /// Shut in behind a locked (or barricaded) door: he should break it down and kill her.
    LockedIn { barred: bool, raged: bool },
    /// Jason with every ability ready, told where she is: he should Morph, Stalk and Shift, and kill her.
    Abilities,
    /// Block his swings with the weapon she was given.
    Block,
    /// Walk into her own bear trap and wiggle free.
    Trap,
    /// Repair a destroyed fuse box (every one starts destroyed): its grid's power comes back.
    Power,
    /// Repair a car and sit in it, unstarted, near Jason: he should pull her out of it and kill her.
    CarGrab,
    /// Fetch the phone box's fuse, put it in, call the police, and escape with them.
    Police,
    /// Wait at a placed kill for Jason: he should grab her and use it.
    PlacedKill,
}

pub struct BotPlugin(pub Goal);

impl Plugin for BotPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(Bot { goal: self.0, ..default() })
            .add_systems(PreUpdate, run_bot.after(InputSystems))
            .add_systems(FixedPostUpdate, watch_fear);
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
    /// Hiding test stage.
    Hide(u8),
    /// Behind the door, waiting for Jason.
    LockedIn,
    /// Bear trap test stage.
    Trap(u8),
    /// Repairing the fuse box (bot.fixtures[0]).
    Power,
    /// Calling the police: fetch the fuse (0), put it in (1), call (2), wait for them and walk in (3).
    Police(u8),
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
    /// The door's health after each of Jason's hits (bot time, health).
    door_hits: Vec<(f32, f32)>,
    /// Fear events that have been active (fear_events.rs).
    fear_seen: Vec<String>,
    /// Her health last frame (--bot-block).
    last_health: f32,
    /// The least breath she had (--bot-hide).
    min_breath: f32,
}

impl Default for Bot {
    fn default() -> Self {
        Self { goal: Goal::Escape, step: Step::Start, vehicle: None, timer: 0.0, total: 0.0, placed_car: false, shot: false, dodge_from: None, dodges_ok: 0, second_tap: false, hold_e: false, fixtures: [None; 3], door_ok: false, jason_shots: Vec::new(), door_hits: Vec::new(), fear_seen: Vec::new(), last_health: 100.0, min_breath: f32::MAX }
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
    mut player: Single<(&mut Transform, &Player, Option<&Inventory>, Has<Driving>, Has<UsingItem>, (Has<crate::jason::Held>, Has<crate::items::ItemAction>, Has<crate::jason::Staggered>, Has<crate::items::Blocking>, Has<crate::items::Trapped>, Option<&crate::hiding::Breathing>), Option<&mut crate::hiding::Hiding>)>,
    mut camera: Single<&mut PlayerCamera>,
    items: Query<(Entity, &Item, &GlobalTransform, Option<&InCabinet>), Without<Carried>>,
    carried: Query<&Item, With<Carried>>,
    cabinets: Query<(&Cabinet, &GlobalTransform)>,
    mut vehicles: Query<(Entity, &Vehicle, &GlobalTransform, &mut Transform), Without<Player>>,
    (zones, mut doors, windows, mut barricades, mut jasons): (
        Query<(&EscapeZone, &GlobalTransform)>,
        Query<(Entity, &mut crate::doors::Door, &GlobalTransform)>,
        Query<(Entity, &crate::windows::Window, &GlobalTransform)>,
        Query<&mut crate::windows::Barricade>,
        Query<(&mut crate::jason::Jason, &Transform), (Without<Player>, Without<Vehicle>)>,
    ),
    (escaped, spots, mut knowledge, mut game, traps): (
        Option<Res<Escaped>>,
        Query<(Entity, &crate::hiding::HidingSpot, &GlobalTransform)>,
        Query<&mut crate::senses::Knowledge>,
        Option<ResMut<crate::match_flow::GameMatch>>,
        Query<&Transform, (With<crate::items::ArmedTrap>, Without<Player>, Without<Vehicle>)>,
    ),
    (mut exit, play, fuse_boxes, grids): (
        MessageWriter<AppExit>,
        Option<Res<crate::jason_player::PlayJason>>,
        Query<(Entity, &crate::power::FuseBox, &Transform), (Without<Player>, Without<Vehicle>)>,
        Option<Res<crate::power::PowerGrids>>,
    ),
    (collision, dynamic, (mut phone_boxes, mut phones, arrived, police_cars, jason_entities, placed_kills)): (Res<crate::collision::WorldCollision>, Res<crate::collision::DynamicColliders>, (Query<&mut crate::police::PhoneBox>, Query<&mut crate::police::Phone>, Option<Res<crate::police::PoliceArrived>>, Query<&Transform, (With<crate::police::PoliceCar>, Without<Player>, Without<Vehicle>)>, Query<Entity, With<crate::jason::Jason>>, Query<(Entity, &crate::placed_kills::PlacedKill)>)),
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
    for k in [KeyCode::Space, KeyCode::KeyW, KeyCode::Digit1, KeyCode::Digit2, KeyCode::Digit3, KeyCode::KeyC, KeyCode::KeyA, KeyCode::KeyS, KeyCode::KeyD] {
        keys.release(k);
    }
    let with_jason = matches!(bot.goal, Goal::PlacedKill | Goal::CarGrab | Goal::Jason | Goal::Fight | Goal::Hide | Goal::HideSeen | Goal::LockedIn { .. } | Goal::Abilities | Goal::Block);
    let limit = if with_jason { 600.0 } else { 240.0 };
    if bot.total > limit && bot.step != Step::Done {
        error!("bot: gave up at {:?}", bot.step);
        // Playing Jason against it: it stops, the game goes on.
        if play.is_some() {
            bot.step = Step::Done;
        } else {
            exit.write(AppExit::error());
        }
        return;
    }
    let (ref mut pxf, p, inv, driving, using, (held, acting, staggered, blocking, trapped, breathing), ref mut hiding) = *player;
    let busy = acting || staggered || held;
    let sees = knowledge.iter().next().is_some_and(|k| k.sees);
    // Jason only knows what he perceives: when he hasn't seen her for a while, step out 10 m in front of him.
    // (Blocking, only once the block is up: else his first swing or knife lands before it.)
    if matches!(bot.goal, Goal::Jason | Goal::Fight | Goal::HideSeen | Goal::Block) && (bot.goal != Goal::Block || blocking) && matches!(bot.step, Step::AwaitJason | Step::Hide(0)) && hiding.is_none() && p.counselor.alive() && !held {
        if sees {
            bot.dodge_from = Some((Vec3::ZERO, bot.total));
        }
        let unseen_for = bot.total - bot.dodge_from.map_or(0.0, |d| d.1);
        if let Some((j, jxf)) = jasons.iter().next().filter(|(j, _)| matches!(j.state, crate::jason::JasonState::Hunting)) {
            let _ = j;
            if !sees && unseen_for > 8.0 {
                let yaw = jxf.rotation.to_euler(EulerRot::YXZ).0;
                // Hiding where he sees her: farther out, so she has time to get in before he's on her.
                let out = if bot.goal == Goal::HideSeen { 15.0 } else { 10.0 };
                let ahead = jxf.translation + Vec3::new(yaw.sin(), 0.0, yaw.cos()) * out + Vec3::Y * 1.0;
                if let Some(at) = stand_spot(&world, ahead, 1.5) {
                    info!("bot: stepping out in front of Jason");
                    place(pxf, at);
                    look_at(&mut camera, pxf, jxf.translation + Vec3::Y);
                }
                bot.dodge_from = Some((Vec3::ZERO, bot.total));
            }
        }
    }
    match bot.step {
        Step::Start => {
            // Let the map finish loading (in real time: frames hitch while assets stream in, whatever --speed).
            if bot.timer < 3.0 || real.elapsed_secs() < 6.0 {
                return;
            }
            if bot.goal == Goal::Police {
                info!("bot: {} phone boxes, {} phones", phone_boxes.iter().count(), phones.iter().count());
                bot.step = Step::Police(0);
                bot.timer = 0.0;
                return;
            }
            if bot.goal == Goal::Power {
                // The nearest destroyed fuse box that feeds a grid.
                let me = pxf.translation;
                let fuse = fuse_boxes.iter().filter(|(_, f, _)| f.destroyed && f.grid.is_some()).min_by(|a, b| a.2.translation.distance(me).total_cmp(&b.2.translation.distance(me)));
                let Some((e, f, _)) = fuse else {
                    error!("bot: no destroyed fuse box");
                    bot.step = Step::Done;
                    return;
                };
                info!("bot: repairing the fuse box at {:.1}", f.interact);
                bot.fixtures = [Some(e), None, None];
                bot.step = Step::Power;
                bot.timer = 0.0;
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
            if let Goal::LockedIn { barred, .. } = bot.goal {
                // The door nearest Jason that locks (that has a barricade), with room to stand inside it.
                let Some((_, jxf)) = jasons.iter().next() else {
                    error!("bot: no Jason");
                    bot.step = Step::Done;
                    return;
                };
                let jat = jxf.translation;
                let has_bar = |e: Entity| barricades.iter().any(|b| b.door == Some(e));
                let mut candidates: Vec<(Entity, Vec3, Vec3)> = doors
                    .iter()
                    .filter(|(e, d, _)| if barred { has_bar(*e) } else { d.lockable && !has_bar(*e) })
                    .filter_map(|(e, _, g)| {
                        let inside = g.translation() - g.rotation() * Vec3::X * 1.5 + Vec3::Y;
                        stand_spot(&world, inside, 0.8).map(|at| (e, at, g.translation()))
                    })
                    .collect();
                candidates.sort_by(|a, b| a.2.distance(jat).total_cmp(&b.2.distance(jat)));
                let Some(&(de, at, door_at)) = candidates.first() else {
                    error!("bot: no door to shut her in behind");
                    bot.step = Step::Done;
                    return;
                };
                place(pxf, at);
                look_at(&mut camera, pxf, door_at + Vec3::Y);
                if let Ok((_, mut d, _)) = doors.get_mut(de) {
                    if barred {
                        for mut b in barricades.iter_mut().filter(|b| b.door == Some(de)) {
                            b.enabled = true;
                        }
                    } else {
                        d.locked = true;
                    }
                    info!("bot: shut in behind a {} door (health {:.0}), Jason {:.0} m away", if barred { "barricaded" } else { "locked" }, d.health, door_at.distance(jat));
                    bot.door_hits.push((0.0, d.health));
                }
                bot.fixtures[0] = Some(de);
                bot.step = Step::LockedIn;
                bot.timer = 0.0;
                return;
            }
            if matches!(bot.goal, Goal::Hide | Goal::HideSeen) {
                info!("bot: {} hiding spots", spots.iter().count());
                bot.step = Step::Hide(0);
                bot.timer = 0.0;
                return;
            }
            if bot.goal == Goal::Trap {
                bot.step = Step::Trap(0);
                bot.timer = 0.0;
                return;
            }
            if bot.goal == Goal::Abilities {
                if let Some(mut k) = knowledge.iter_mut().next() {
                    let a = &mut k.abilities;
                    a.unlocked = [true; 4];
                    a.recharge_timer = a.tuning.recharge_time;
                }
                // And past the AI's wait before its first Morph.
                for (mut j, _) in &mut jasons {
                    j.skip_morph_grace();
                }
            }
            if bot.goal == Goal::PlacedKill {
                // The placed kill nearest Jason: she stands just in front of it.
                let Some((_, jxf)) = jasons.iter().next() else {
                    error!("bot: no Jason");
                    bot.step = Step::Done;
                    return;
                };
                let near = |k: &crate::placed_kills::PlacedKill| k.interact.distance(jxf.translation);
                let Some((e, k)) = placed_kills.iter().filter(|(_, k)| !k.used).min_by(|a, b| near(a.1).total_cmp(&near(b.1))) else {
                    error!("bot: no placed kill");
                    bot.step = Step::Done;
                    return;
                };
                let front = k.interact + Vec3::new(k.forward.x, 0.0, k.forward.y) * 0.9;
                let at = stand_spot(&world, front.with_y(k.spot.translation.y + 1.0), 1.5).unwrap_or(front);
                place(pxf, at);
                info!("bot: waiting at {} ({}), Jason {:.0} m away", k.takes().2, k.camera_key, jxf.translation.distance(at));
                bot.fixtures[0] = Some(e);
                bot.step = Step::AwaitJason;
                bot.timer = 0.0;
                return;
            }
            if matches!(bot.goal, Goal::Jason | Goal::Fight | Goal::Abilities | Goal::Block) {
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
                // The dial, once, past halfway.
                if h.progress(&tuning.minigame) > 0.55 && !bot.jason_shots.contains(&"repair") {
                    bot.jason_shots.push("repair");
                    let path = std::env::temp_dir().join("f13-bot-repair.png");
                    info!("bot: screenshot {}", path.display());
                    commands.spawn(Screenshot::primary_window()).observe(save_to_disk(path));
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
        Step::Drive if bot.goal == Goal::CarGrab => {
            // Waiting in the car, unstarted, near Jason (who's told where she is) for him to pull her out.
            if !driving {
                if !bot.jason_shots.contains(&"pulled") {
                    bot.jason_shots.push("pulled");
                    info!("bot: pulled out of the car after {:.1}s", bot.timer);
                }
                if !p.counselor.alive() {
                    bot.step = Step::Done;
                    bot.timer = 0.0;
                }
                return;
            }
            let Some((_, jxf)) = jasons.iter().next() else { return };
            let Ok((_, v, _, mut vxf)) = vehicles.get_mut(bot.vehicle.unwrap()) else { return };
            if !bot.placed_car {
                bot.placed_car = true;
                // 8 m from him, the driver's side toward him.
                let jat = jxf.translation;
                let seat_dir = (Quat::from_rotation_y(0.0) * v.driver_seat).with_y(0.0).normalize_or_zero();
                let at = jat + Vec3::new(8.0, 0.0, 0.0);
                vxf.translation = at + Vec3::Y * 0.5;
                // Turn the car so its driver side (seat_dir, local) faces him (-X from the car).
                let want = Vec3::NEG_X;
                let yaw = want.z.atan2(want.x) - seat_dir.z.atan2(seat_dir.x);
                vxf.rotation = Quat::from_rotation_y(-yaw);
                info!("bot: waiting in the car 8 m from Jason");
            }
            if let Some(mut k) = knowledge.iter_mut().next() {
                if !k.sees {
                    k.last_known = Some((pxf.translation, time.elapsed_secs(), crate::senses::Clue::Heard));
                }
            }
            if bot.timer > 120.0 {
                error!("bot: Jason never pulled her out");
                bot.step = Step::Done;
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
            // Placed on the step's first frame however long it was (a hitch at speed can pass 0.5 s at once).
            let first_frame = bot.timer <= dt + 1e-6;
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
                if step.outside.is_some() && (bot.timer < 0.5 || first_frame) {
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
                if bot.timer < 0.5 || first_frame {
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
        Step::AwaitJason if bot.goal == Goal::Fight => {
            use crate::jason::JasonState;
            use crate::props::ItemKind;
            let Some((j, jxf)) = jasons.iter().next() else { return };
            let dist = jxf.translation.distance(pxf.translation);
            if bot.timer >= 5.0 {
                bot.timer = 0.0;
                info!("bot: Jason {:?}, {:.1} m away; health {:.0}; hand {:?}", j.state, dist, p.counselor.health, inv.and_then(|i| i.hand).map(|e| carried.get(e).map(|i| i.kind.clone())));
            }
            keys.release(KeyCode::KeyA);
            keys.release(KeyCode::KeyD);
            keys.release(KeyCode::KeyM);
            // Carrying the map: open it once (and photograph it).
            let has_map = inv.is_some_and(|i| i.pockets.iter().any(|&e| carried.get(e).is_ok_and(|it| it.kind == ItemKind::Map)));
            if has_map && !bot.jason_shots.contains(&"map") {
                bot.jason_shots.push("map");
                keys.press(KeyCode::KeyM);
            } else if bot.jason_shots.contains(&"map") && !bot.jason_shots.contains(&"mapshot") && bot.timer > 1.0 {
                bot.jason_shots.push("mapshot");
                let path = std::env::temp_dir().join("f13-bot-map.png");
                info!("bot: screenshot {}", path.display());
                commands.spawn(Screenshot::primary_window()).observe(save_to_disk(path));
            }
            for k in [KeyCode::Digit1, KeyCode::Digit2, KeyCode::Digit3] {
                keys.release(k);
            }
            if held {
                // Struggle: alternate A and D.
                bot.second_tap = !bot.second_tap;
                keys.press(if bot.second_tap { KeyCode::KeyA } else { KeyCode::KeyD });
            }
            // Face him.
            let d = jxf.translation - pxf.translation;
            camera.yaw = (-d.x).atan2(-d.z);
            // At his chest (the camera's eye is about 1.9 m over her feet).
            camera.pitch = (d.y + 1.2 - 1.9).atan2(Vec2::new(d.x, d.z).length()).clamp(-1.0, 0.5);
            if matches!(j.state, JasonState::Hunting) && !busy {
                let kind = |e: Entity| carried.get(e).ok().map(|i| i.kind.clone());
                let hand = inv.and_then(|i| i.hand).and_then(kind);
                match hand {
                    Some(ItemKind::Weapon(_)) if dist < 2.6 => {
                        debug!("bot: swing at {dist:.2} m");
                        mouse.press(MouseButton::Left);
                    }
                    Some(ItemKind::Gun { .. }) if dist < 10.0 => mouse.press(MouseButton::Left),
                    Some(ItemKind::Trap) if dist < 14.0 && !bot.jason_shots.contains(&"trap") => {
                        bot.jason_shots.push("trap");
                        mouse.press(MouseButton::Left);
                    }
                    _ => {}
                }
                if let Some(slot) = inv.and_then(|i| i.pockets.iter().position(|&e| matches!(kind(e), Some(ItemKind::Firecracker { .. })))) {
                    if dist < 11.0 && dist > 7.0 {
                        keys.press([KeyCode::Digit1, KeyCode::Digit2, KeyCode::Digit3][slot]);
                    }
                }
            }
            // After placing a trap, back away 2.5 m along his line so he walks over it to reach her.
            keys.release(KeyCode::KeyS);
            if bot.jason_shots.contains(&"trap") && !busy && !bot.jason_shots.contains(&"backed") {
                let from = bot.dodge_from.get_or_insert((pxf.translation, 0.0)).0;
                if from.distance(pxf.translation) < 2.5 {
                    keys.press(KeyCode::KeyS);
                } else {
                    bot.jason_shots.push("backed");
                }
            }
            if let JasonState::Stunned { t, stun } = j.state {
                if !bot.jason_shots.contains(&"stun") && t > 1.0 {
                    bot.jason_shots.push("stun");
                    info!("bot: Jason stunned ({stun:?})");
                    let path = std::env::temp_dir().join("f13-bot-fight-stun.png");
                    info!("bot: screenshot {}", path.display());
                    commands.spawn(Screenshot::primary_window()).observe(save_to_disk(path));
                }
            }
            if bot.jason_shots.contains(&"stun") && matches!(j.state, JasonState::Hunting) {
                info!("bot: Jason recovered after the stun ({:.1}s)", bot.total);
                bot.step = Step::Done;
                bot.timer = 0.0;
            }
            if !p.counselor.alive() {
                info!("bot: Vanessa died ({:?})", j.state);
                bot.step = Step::Done;
                bot.timer = 0.0;
            }
        }
        Step::Power => {
            let Some((_, f, fxf)) = bot.fixtures[0].and_then(|e| fuse_boxes.get(e).ok()) else {
                bot.step = Step::Done;
                return;
            };
            if !f.destroyed {
                info!("bot: fuse box repaired after {:.1}s; its grid is {}", bot.timer, if f.grid.is_some_and(|g| grids.as_ref().is_some_and(|p| p.on(g))) { "on" } else { "OFF" });
                bot.hold_e = false;
                bot.step = Step::Done;
                bot.timer = 0.0;
                return;
            }
            // Facing it, a step out from its front, on its floor.
            let out = Vec3::new(f.interact.x - fxf.translation.x, 0.0, f.interact.z - fxf.translation.z).normalize_or_zero();
            place(pxf, Vec3::new(f.interact.x, fxf.translation.y, f.interact.z) + out * 0.6);
            look_at(&mut camera, pxf, f.interact);
            bot.hold_e = bot.timer > 0.3;
            if bot.hold_e {
                keys.press(KeyCode::KeyE);
            }
            // The pips: each one's key when the needle is in its middle.
            if let Some(h) = interaction.hold.as_ref().filter(|h| h.started) {
                if let Some(p) = h.current.map(|c| h.pips[c]) {
                    if h.progress(&tuning.minigame) >= (p.position + p.width / 2.0) / 360.0 {
                        mouse.press(if p.key == 0 { MouseButton::Right } else { MouseButton::Left });
                    }
                }
            }
            if bot.timer > 60.0 {
                error!("bot: fuse box repair not progressing");
                bot.hold_e = false;
                bot.step = Step::Done;
            }
        }
        Step::Police(stage) => {
            // Breaking the box after its fuse is in (as Jason's DestroyBox does), once, so she repairs it too.
            if stage == 2 && !bot.jason_shots.contains(&"box broken") {
                if let Some(mut b) = phone_boxes.iter_mut().find(|b| b.phone.is_some()) {
                    bot.jason_shots.push("box broken");
                    crate::police::break_phone_box(&mut b, &mut phones);
                    info!("bot: the phone box is broken; repairing it");
                    bot.step = Step::Police(1);
                    bot.timer = 0.0;
                    return;
                }
            }
            let Some(pb) = phone_boxes.iter().find(|b| b.phone.is_some()) else {
                error!("bot: no phone box");
                bot.step = Step::Done;
                return;
            };
            // Holding E on a spot (a special move's, or a step out from the interact point), playing the pips.
            let mut hold_at = |bot: &mut Bot, pxf: &mut Transform, interact: Vec3, stand: Option<Transform>| {
                let at = stand.map_or(interact - Vec3::Y * 1.0, |s| s.translation + Vec3::Y * 0.9);
                if !interaction.hold.as_ref().is_some_and(|h| h.started) {
                    place(pxf, stand_spot(&world, at, 0.5).unwrap_or(at));
                }
                look_at(&mut camera, pxf, interact);
                bot.hold_e = bot.timer > 0.3;
                if bot.hold_e {
                    keys.press(KeyCode::KeyE);
                }
                if let Some(h) = interaction.hold.as_ref().filter(|h| h.started) {
                    if let Some(p) = h.current.map(|c| h.pips[c]) {
                        if h.progress(&tuning.minigame) >= (p.position + p.width / 2.0) / 360.0 {
                            mouse.press(if p.key == 0 { MouseButton::Right } else { MouseButton::Left });
                        }
                    }
                }
            };
            match stage {
                0 => {
                    let Some(inv) = inv else { return };
                    if inv.hand.iter().chain(inv.pockets.iter()).any(|&e| carried.get(e).is_ok_and(|it| pb.parts.contains(&it.class))) {
                        info!("bot: have the fuse after {:.1}s", bot.total);
                        bot.step = Step::Police(1);
                        bot.timer = 0.0;
                        return;
                    }
                    let parts = pb.parts.clone();
                    if !go_fetch(&world, &mut bot, &mut camera, &parts, pb.interact, pxf, &items, &cabinets, &mut keys) {
                        bot.step = Step::Done;
                    }
                }
                1 => {
                    if pb.repaired && !pb.broken {
                        info!("bot: phone box repaired after {:.1}s", bot.timer);
                        bot.hold_e = false;
                        bot.step = Step::Police(2);
                        bot.timer = 0.0;
                        return;
                    }
                    hold_at(&mut bot, pxf, pb.interact, pb.stand);
                    if bot.timer > 60.0 {
                        error!("bot: phone box repair not progressing");
                        bot.hold_e = false;
                        bot.step = Step::Done;
                    }
                }
                2 => {
                    let Some(phone) = pb.phone.and_then(|e| phones.get(e).ok()) else { return };
                    if phone.called {
                        bot.hold_e = false;
                        // Test shortcut: the police come in 10 s instead of 300.
                        if let Some(g) = game.as_mut() {
                            info!("bot: police called after {:.1}s; cutting the countdown from {} s to 10", bot.timer, g.state.police_time_remaining);
                            g.state.police_time_remaining = g.state.police_time_remaining.min(10);
                        }
                        bot.step = Step::Police(3);
                        bot.timer = 0.0;
                        return;
                    }
                    hold_at(&mut bot, pxf, phone.interact, phone.stand);
                    if bot.timer > 60.0 {
                        error!("bot: phone call not progressing");
                        bot.hold_e = false;
                        bot.step = Step::Done;
                    }
                }
                _ => {
                    if escaped.is_some() {
                        info!("bot: escaped with the police after {:.1}s", bot.total);
                        bot.step = Step::Done;
                        bot.timer = 0.0;
                        return;
                    }
                    let Some(a) = &arrived else {
                        // The HUD's police timer, counting.
                        if bot.timer > 2.0 && !bot.jason_shots.contains(&"police hud") {
                            bot.jason_shots.push("police hud");
                            let path = std::env::temp_dir().join("f13-bot-police-hud.png");
                            info!("bot: screenshot {}", path.display());
                            commands.spawn(Screenshot::primary_window()).observe(save_to_disk(path));
                        }
                        if bot.timer > 20.0 {
                            error!("bot: the police never came");
                            bot.step = Step::Done;
                        }
                        return;
                    };
                    let Ok((zone, zxf)) = zones.get(a.zone) else { return };
                    // Jason, if he's about, dropped in the middle of their volume: they should shoot him back out.
                    if let (Some(je), Some((j, jxf))) = (jason_entities.iter().next(), jasons.iter().next()) {
                        // Again if he got away unstunned (a Morph as he landed).
                        if !bot.jason_shots.contains(&"dropped") || (bot.timer > 3.0 && !bot.jason_shots.contains(&"police stun") && jxf.translation.distance(zxf.translation()) > zone.radius_m + 3.0) {
                            bot.jason_shots.retain(|s| *s != "dropped");
                            bot.jason_shots.push("dropped");
                            bot.timer = 0.0;
                            info!("bot: dropping Jason into the police's escape");
                            commands.entity(je).insert(Transform { translation: zxf.translation() + Vec3::Y * 1.0, ..*jxf });
                        }
                        if matches!(j.state, crate::jason::JasonState::Stunned { stun: crate::jason::Stun::Police, .. }) && !bot.jason_shots.contains(&"police stun") {
                            bot.jason_shots.push("police stun");
                            info!("bot: Jason stunned by the police {:.1} m from their volume's centre (radius {:.1})", jxf.translation.distance(zxf.translation()), zone.radius_m);
                        }
                        if !bot.jason_shots.contains(&"police stun") {
                            return;
                        }
                    }
                    // A look at the police car first (from 7 m behind it, on its road).
                    if !bot.jason_shots.contains(&"arrived") {
                        bot.jason_shots.push("arrived");
                        bot.timer = 0.0;
                    }
                    if !bot.jason_shots.contains(&"police") {
                        if let Some(car) = police_cars.iter().next() {
                            let side = -(car.rotation * Vec3::X);
                            let from = car.translation + side * 7.0 + Vec3::Y * 2.0;
                            place(pxf, stand_spot(&world, from, 3.0).unwrap_or(from));
                            look_at(&mut camera, pxf, car.translation + Vec3::Y * 0.5);
                            if bot.timer > 2.0 {
                                bot.jason_shots.push("police");
                                let path = std::env::temp_dir().join("f13-bot-police.png");
                                info!("bot: screenshot {}", path.display());
                                commands.spawn(Screenshot::primary_window()).observe(save_to_disk(path));
                            }
                            return;
                        }
                    }
                    // Start 3 m outside the volume and walk in.
                    if !bot.placed_car {
                        bot.placed_car = true;
                        let c = zxf.translation();
                        let out = (zone.trigger - c).with_y(0.0).normalize_or(Vec3::X);
                        let from = c + out * (zone.radius_m + 3.0) + Vec3::Y * 2.0;
                        let at = stand_spot(&world, from, 2.0).unwrap_or(from);
                        info!("bot: walking into the police's escape from ({:.0}, {:.0}, {:.0})", at.x, at.y, at.z);
                        place(pxf, at);
                        look_at(&mut camera, pxf, c.with_y(at.y + 1.0));
                    }
                    keys.press(KeyCode::KeyW);
                    if bot.timer > 30.0 {
                        error!("bot: did not reach the police: at ({:.1}, {:.1}, {:.1}), {:.1} m from the volume's centre (radius {:.1})", pxf.translation.x, pxf.translation.y, pxf.translation.z, pxf.translation.distance(zxf.translation()), zone.radius_m);
                        bot.step = Step::Done;
                    }
                }
            }
        }
        Step::Trap(stage) => {
            let trap = traps.iter().next().map(|t| t.translation);
            match stage {
                // Place it.
                0 => {
                    if bot.timer > 1.0 && !acting {
                        if trap.is_some() {
                            info!("bot: trap placed");
                            bot.step = Step::Trap(1);
                            bot.timer = 0.0;
                        } else if !bot.second_tap || bot.timer > 4.0 {
                            // One click (again after a while if it didn't take): a fixed window can pass in one
                            // long frame at speed.
                            mouse.press(MouseButton::Left);
                            bot.second_tap = true;
                            if bot.timer > 4.0 {
                                bot.timer = 1.0;
                            }
                        }
                    }
                }
                // Stand 2 m back from it, facing it, then walk in.
                1 => {
                    if trapped {
                        info!("bot: caught, health {:.0}", p.counselor.health);
                        bot.jason_shots.push("caught");
                        bot.last_health = p.counselor.health;
                        bot.step = Step::Trap(2);
                        bot.timer = 0.0;
                        return;
                    }
                    let Some(at) = trap else { return };
                    if bot.timer < 0.1 || bot.timer <= dt + 1e-6 {
                        let back = pxf.translation - at;
                        let back = Vec3::new(back.x, 0.0, back.z).normalize_or(Vec3::Z);
                        if let Some(spot) = stand_spot(&world, at + back * 2.0 + Vec3::Y, 1.5) {
                            place(pxf, spot);
                        }
                    }
                    look_at(&mut camera, pxf, at);
                    camera.pitch = -0.2;
                    if bot.timer > 0.5 {
                        keys.press(KeyCode::KeyW);
                    }
                    if bot.timer > 8.0 {
                        error!("bot: never stepped in the trap");
                        bot.step = Step::Done;
                    }
                }
                // Wiggle free (A / D alternating).
                _ => {
                    if trapped {
                        bot.second_tap = !bot.second_tap;
                        keys.press(if bot.second_tap { KeyCode::KeyA } else { KeyCode::KeyD });
                        if bot.timer > 0.8 && !bot.jason_shots.contains(&"trapshot") {
                            bot.jason_shots.push("trapshot");
                            let path = std::env::temp_dir().join("f13-bot-trap.png");
                            info!("bot: screenshot {}", path.display());
                            commands.spawn(Screenshot::primary_window()).observe(save_to_disk(path));
                        }
                    } else {
                        info!("bot: out of the trap after {:.1}s", bot.timer);
                        bot.jason_shots.push("free");
                        bot.step = Step::Done;
                        bot.timer = 0.0;
                    }
                }
            }
        }
        Step::AwaitJason if bot.goal == Goal::Block => {
            let Some((j, jxf)) = jasons.iter().next() else { return };
            if bot.timer >= 5.0 {
                bot.timer = 0.0;
                info!("bot: Jason {:?}, {:.1} m away; health {:.0}; blocking {blocking}", j.state, jxf.translation.distance(pxf.translation), p.counselor.health);
            }
            // Face him in combat stance, the block held.
            let d = jxf.translation - pxf.translation;
            camera.yaw = (-d.x).atan2(-d.z);
            if p.counselor.stance != f13_core::counselor::Stance::Combat {
                if !bot.second_tap {
                    keys.press(KeyCode::KeyC);
                }
                bot.second_tap = !bot.second_tap;
            } else if p.counselor.alive() && !held {
                mouse.press(MouseButton::Right);
            }
            // A hit taken while blocking.
            let health = p.counselor.health;
            let last = bot.last_health;
            if health < last {
                info!("bot: hit for {:.1} ({}blocking)", last - health, if blocking { "" } else { "not " });
                // Less than his full 20: blocked (a heavy swing ends the block in the same tick).
                if last - health < 19.0 && !bot.jason_shots.contains(&"blocked") {
                    bot.jason_shots.push("blocked");
                    let path = std::env::temp_dir().join("f13-bot-block.png");
                    info!("bot: screenshot {}", path.display());
                    commands.spawn(Screenshot::primary_window()).observe(save_to_disk(path));
                }
            }
            bot.last_health = health;
            if !p.counselor.alive() {
                info!("bot: Vanessa died ({:?})", j.state);
                bot.step = Step::Done;
                bot.timer = 0.0;
            }
        }
        Step::AwaitJason => {
            let Some((j, jxf)) = jasons.iter().next() else { return };
            if bot.goal == Goal::PlacedKill {
                // He hears her where she waits.
                if let Some(mut k) = knowledge.iter_mut().next() {
                    if !k.sees {
                        k.last_known = Some((pxf.translation, time.elapsed_secs(), crate::senses::Clue::Heard));
                    }
                }
                let placed = bot.fixtures[0].and_then(|e| placed_kills.get(e).ok()).map(|(_, k)| k.takes().2);
                if matches!(j.state, crate::jason::JasonState::Killing { .. }) && placed == Some(j.kill_move.2) && !bot.jason_shots.contains(&"placed") {
                    bot.jason_shots.push("placed");
                    info!("bot: Jason is killing her with the placed kill ({})", j.kill_move.2);
                }
            }
            if bot.goal == Goal::Abilities {
                // A Morph can come and go between two frames at speed: also seen as Morph recharging once unlocked.
                let (mut morph_spent, mut stalk_spent, mut shift_spent) = (false, false, false);
                if let Some(mut k) = knowledge.iter_mut().next() {
                    if !k.sees {
                        k.last_known = Some((pxf.translation, time.elapsed_secs(), crate::senses::Clue::Heard));
                    }
                    let a = &k.abilities;
                    let spent = |ability| a.is_unlocked(ability) && a.recharge_fraction(ability) < 1.0;
                    morph_spent = spent(f13_core::killer_abilities::Ability::Morph);
                    // Stalk and Shift too: one can start and end within a long frame at speed (a grab out of a Shift).
                    stalk_spent = spent(f13_core::killer_abilities::Ability::Stalk);
                    shift_spent = spent(f13_core::killer_abilities::Ability::Shift);
                }
                let used = [
                    (matches!(j.state, crate::jason::JasonState::Morphing { .. }) || morph_spent, "morph"),
                    (j.stalking() || stalk_spent, "stalk"),
                    (j.shifting() || shift_spent, "shift"),
                ];
                for (on, name) in used {
                    if on && !bot.jason_shots.contains(&name) {
                        bot.jason_shots.push(name);
                        info!("bot: Jason used {name}, {:.1} m away", jxf.translation.distance(pxf.translation));
                    }
                }
                if j.shifting() && j.fade.0 == 0.0 && !bot.jason_shots.contains(&"shift-shot") {
                    bot.jason_shots.push("shift-shot");
                    look_at(&mut camera, pxf, jxf.translation + Vec3::Y);
                    let path = std::env::temp_dir().join("f13-bot-abilities-shift.png");
                    info!("bot: screenshot {}", path.display());
                    commands.spawn(Screenshot::primary_window()).observe(save_to_disk(path));
                }
            }
            if bot.timer >= 5.0 {
                bot.timer = 0.0;
                info!("bot: Jason {:?}, {:.1} m away; health {:.0}", j.state, jxf.translation.distance(pxf.translation), p.counselor.health);
            }
            // Up close, watch from Jason's side (to check the grab and kill poses).
            if jxf.translation.distance(pxf.translation) < 3.0 {
                let (yaw, _, _) = jxf.rotation.to_euler(EulerRot::YXZ);
                let right = Vec3::new(-yaw.cos(), 0.0, yaw.sin());
                camera.yaw = (-right.x).atan2(-right.z);
                // Looking down at the end of the kill, where she lies.
                camera.pitch = if matches!(j.state, crate::jason::JasonState::Killing { t } if t > 9.0) { -0.6 } else { -0.15 };
            }
            // Screenshots of the hit, the grab and the kill, to check the paired poses.
            let shot = match j.state {
                crate::jason::JasonState::Swinging { t, .. } if t > 0.35 => Some("swing"),
                crate::jason::JasonState::Holding { t } if t > 1.0 => Some("grab"),
                crate::jason::JasonState::Killing { t } if t > 9.3 => Some("kill3"),
                crate::jason::JasonState::Killing { t } if t > 7.0 => Some("kill2"),
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
        Step::LockedIn => {
            let (Some((j, jxf)), Some(de)) = (jasons.iter().next(), bot.fixtures[0]) else { return };
            let Ok((_, d, g)) = doors.get(de) else { return };
            // He hears her in there (and is raged, for --bot-raged-in).
            if let Some(mut k) = knowledge.iter_mut().next() {
                if matches!(bot.goal, Goal::LockedIn { raged: true, .. }) {
                    k.abilities.rage_active = true;
                }
                if !k.sees {
                    k.last_known = Some((pxf.translation, time.elapsed_secs(), crate::senses::Clue::Heard));
                }
            }
            if bot.timer >= 5.0 {
                bot.timer = 0.0;
                info!("bot: Jason {:?}, {:.1} m away; door health {:.0}", j.state, jxf.translation.distance(pxf.translation), d.health);
            }
            if let Some(&(_, last)) = bot.door_hits.last() {
                if d.health < last {
                    info!("bot: hit {} on the door at {:.1}s: health {:.0}", bot.door_hits.len(), bot.total, d.health);
                    let total = bot.total;
                    bot.door_hits.push((total, d.health));
                }
            }
            if !held && !busy {
                look_at(&mut camera, pxf, g.translation() + Vec3::Y);
            }
            let shot = match j.state {
                crate::jason::JasonState::BreakingDoor { section: 1, t, .. } if t > 1.5 && bot.door_hits.len() == 3 => Some("medium"),
                crate::jason::JasonState::BreakingDoor { section: 1, t, .. } if t > 0.9 => Some("loop"),
                crate::jason::JasonState::BurstingDoor { t, .. } if t > 0.45 => Some("burst"),
                _ if d.broken && !bot.jason_shots.contains(&"broken") => Some("broken"),
                _ => None,
            };
            if let Some(s) = shot.filter(|s| !bot.jason_shots.contains(s)) {
                bot.jason_shots.push(s);
                let path = std::env::temp_dir().join(format!("f13-bot-breakdown-{s}.png"));
                info!("bot: screenshot {}", path.display());
                commands.spawn(Screenshot::primary_window()).observe(save_to_disk(path));
            }
            if !p.counselor.alive() {
                info!("bot: Vanessa died after {:.1}s ({:?}); door hits {:?}", bot.total, j.state, bot.door_hits);
                bot.step = Step::Done;
                bot.timer = 0.0;
            }
        }
        Step::Hide(stage) => {
            let Some((j, jxf)) = jasons.iter().next() else {
                error!("bot: no Jason");
                bot.step = Step::Done;
                return;
            };
            let jd = jxf.translation.distance(pxf.translation);
            let shoot = |commands: &mut Commands, name: &str| {
                let path = std::env::temp_dir().join(format!("f13-bot-hide-{name}.png"));
                info!("bot: screenshot {}", path.display());
                commands.spawn(Screenshot::primary_window()).observe(save_to_disk(path));
            };
            match stage {
                // Pick a spot: unseen, the nearest one to her once Jason is far; seen, the one nearest Jason once he
                // watches her (and she's 8..25 m from him).
                0 => {
                    let ready = match bot.goal {
                        Goal::HideSeen => sees && (6.0..30.0).contains(&jd),
                        _ => bot.timer > 1.0,
                    };
                    if !ready {
                        return;
                    }
                    let from = if bot.goal == Goal::HideSeen { jxf.translation } else { pxf.translation };
                    let Some((e, s, g)) = spots
                        .iter()
                        .filter(|(_, s, g)| s.usable() && (bot.goal != Goal::HideSeen || g.translation().distance(jxf.translation) > 4.0))
                        .min_by(|a, b| a.2.translation().distance(from).total_cmp(&b.2.translation().distance(from)))
                    else {
                        error!("bot: no hiding spot");
                        bot.step = Step::Done;
                        return;
                    };
                    let point = g.transform_point(s.interact);
                    let Some(stand) = stand_spot(&world, point, s.reach / 100.0 + 0.3) else {
                        warn!("bot: nowhere to stand at {:?}", s.kind);
                        return;
                    };
                    place(pxf, stand);
                    look_at(&mut camera, pxf, point);
                    info!("bot: going to hide in a {:?} ({e:?}), Jason {jd:.0} m away ({:?})", s.kind, j.state);
                    bot.vehicle = Some(e);
                    bot.step = Step::Hide(1);
                    bot.timer = 0.0;
                }
                // Press E at it.
                1 => {
                    if hiding.is_some() {
                        bot.step = Step::Hide(2);
                        bot.timer = 0.0;
                        return;
                    }
                    if bot.timer > 0.3 && !bot.second_tap {
                        keys.press(KeyCode::KeyE);
                        bot.second_tap = true;
                    } else {
                        bot.second_tap = false;
                    }
                    // (8 s: at speed, a few long frames can pass 3 s with only a press or two.)
                    if bot.timer > 8.0 {
                        error!("bot: couldn't get into the hiding spot");
                        bot.step = Step::Done;
                        bot.timer = 0.0;
                    }
                }
                // Going in, then hidden.
                2 => {
                    let Some(h) = hiding.as_deref_mut() else {
                        if !p.counselor.alive() {
                            bot.step = Step::Done;
                        }
                        return;
                    };
                    if h.phase == crate::hiding::HidePhase::Entering && bot.timer > 1.2 && !bot.jason_shots.contains(&"entering") {
                        bot.jason_shots.push("entering");
                        shoot(&mut commands, "entering");
                    }
                    if h.phase == crate::hiding::HidePhase::Hidden {
                        if !bot.jason_shots.contains(&"inside") && bot.timer > 4.0 {
                            bot.jason_shots.push("inside");
                            shoot(&mut commands, "inside");
                        }
                        if bot.timer >= 5.0 && (bot.timer - dt) as u32 / 10 != bot.timer as u32 / 10 {
                            info!("bot: hidden {:.0}s; Jason {:?}, {jd:.0} m away", bot.timer, j.state);
                        }
                        // Hold her breath for 4 s.
                        if bot.goal == Goal::Hide {
                            if (10.0..14.0).contains(&bot.timer) {
                                keys.press(KeyCode::Space);
                            }
                            if let Some(b) = breathing {
                                bot.min_breath = bot.min_breath.min(b.breath.current);
                            }
                        }
                        if bot.goal == Goal::Hide && bot.timer > 60.0 {
                            info!("bot: leaving the hiding spot");
                            crate::hiding::request_exit(h);
                            bot.step = Step::Hide(3);
                            bot.timer = 0.0;
                        }
                    }
                    if h.phase == crate::hiding::HidePhase::Killed && bot.timer > 0.0 && !bot.jason_shots.contains(&"kill") {
                        bot.timer = -1.5;
                        bot.jason_shots.push("kill");
                    }
                    if bot.jason_shots.contains(&"kill") && !bot.jason_shots.contains(&"killshot") && bot.timer > 0.0 {
                        bot.jason_shots.push("killshot");
                        let d = jxf.translation - pxf.translation;
                        camera.yaw = (d.z).atan2(-d.x);
                        camera.pitch = -0.2;
                        shoot(&mut commands, "kill");
                    }
                    if bot.goal == Goal::HideSeen && bot.timer > 240.0 {
                        error!("bot: Jason never came for her");
                        bot.step = Step::Done;
                        bot.timer = 0.0;
                    }
                }
                // Coming out.
                _ => {
                    if hiding.is_some() {
                        if bot.timer > 0.6 && !bot.jason_shots.contains(&"exit") {
                            bot.jason_shots.push("exit");
                            shoot(&mut commands, "exit");
                        }
                        return;
                    }
                    info!("bot: out of hiding after {:.1}s, alive {}", bot.total, p.counselor.alive());
                    bot.step = Step::Done;
                    bot.timer = 0.0;
                }
            }
            if !p.counselor.alive() && bot.step != Step::Done {
                info!("bot: Vanessa died after {:.1}s ({:?})", bot.total, j.state);
                bot.step = Step::Done;
                bot.timer = 0.0;
            }
        }
        Step::Done => {
            // Playing Jason against it: no photographs or verdict, the game goes on.
            if play.is_some() {
                return;
            }
            // After a death, let the ragdoll settle and photograph it from above.
            // The ragdoll taking over from the kill's last frame, then settled, from above.
            if bot.goal == Goal::Jason && !p.counselor.alive() {
                for (name, at) in [("dead0", 0.1), ("dead1", 0.4), ("dead2", 1.0)] {
                    if bot.timer > at && !bot.jason_shots.contains(&name) {
                        bot.jason_shots.push(name);
                        let path = std::env::temp_dir().join(format!("f13-bot-jason-{name}.png"));
                        info!("bot: screenshot {}", path.display());
                        commands.spawn(Screenshot::primary_window()).observe(save_to_disk(path));
                    }
                }
                camera.pitch = if bot.timer > 1.5 { -0.75 } else { -0.6 };
            }
            if bot.goal == Goal::Jason && !p.counselor.alive() && !bot.jason_shots.contains(&"dead") && bot.timer > 3.0 {
                bot.jason_shots.push("dead");
                let path = std::env::temp_dir().join("f13-bot-jason-dead.png");
                info!("bot: screenshot {}", path.display());
                commands.spawn(Screenshot::primary_window()).observe(save_to_disk(path));
            }
            // The escape run goes on to the match's results.
            let results = game.as_ref().is_none_or(|g| matches!(g.state.state, MatchState::WaitingPostMatch | MatchState::LeavingMap));
            if bot.goal == Goal::Escape {
                if !results {
                    return;
                }
                if !bot.jason_shots.contains(&"results") {
                    bot.jason_shots.push("results");
                    bot.timer = 0.0;
                    let path = std::env::temp_dir().join("f13-bot-results.png");
                    info!("bot: screenshot {}", path.display());
                    commands.spawn(Screenshot::primary_window()).observe(save_to_disk(path));
                }
            }
            if bot.timer > if bot.goal == Goal::Jason { 4.0 } else { 2.0 } {
                let ok = match bot.goal {
                    Goal::Escape => escaped.is_some(),
                    // With Jason about, the police must have shot him.
                    Goal::Police => escaped.is_some() && (jasons.is_empty() || bot.jason_shots.contains(&"police stun")),
                    Goal::Heal => p.counselor.health >= 100.0,
                    Goal::Dodge => bot.dodges_ok == 3,
                    Goal::Door => bot.door_ok,
                    Goal::Jason => !p.counselor.alive(),
                    Goal::Abilities => !p.counselor.alive() && ["morph", "stalk", "shift"].iter().all(|a| bot.jason_shots.contains(a)),
                    Goal::Block => !p.counselor.alive() && bot.jason_shots.contains(&"blocked"),
                    Goal::Trap => p.counselor.alive() && bot.last_health == 50.0 && bot.jason_shots.contains(&"free"),
                    Goal::Fight => p.counselor.alive() && bot.jason_shots.contains(&"stun"),
                    Goal::CarGrab => !p.counselor.alive() && bot.jason_shots.contains(&"pulled"),
                    Goal::PlacedKill => !p.counselor.alive() && bot.jason_shots.contains(&"placed"),
                    Goal::Power => bot.fixtures[0]
                        .and_then(|e| fuse_boxes.get(e).ok())
                        .is_some_and(|(_, f, _)| !f.destroyed && f.grid.is_some_and(|g| grids.as_ref().is_some_and(|p| p.on(g)))),
                    Goal::Hide => {
                        let breath = breathing.map_or(0.0, |b| b.breath.current);
                        info!("bot: breath fell to {:.0}, now {breath:.0}", bot.min_breath);
                        p.counselor.alive() && hiding.is_none() && bot.jason_shots.contains(&"inside") && bot.min_breath < 80.0 && breath > bot.min_breath
                    }
                    Goal::HideSeen => !p.counselor.alive() && bot.jason_shots.contains(&"kill"),
                    // A hit per 20 health (the pickaxe), per 10 through a barricade: cabin doors (60) take three or six,
                    // Higgins Haven's outer doors (20) one or two. The first entry is the starting health.
                    Goal::LockedIn { barred, raged } => {
                        // Raged: one burst.
                        let hits = if raged { 1 } else { bot.door_hits.first().map_or(0, |h| (h.1 / if barred { 10.0 } else { 20.0 }).ceil() as usize) };
                        !p.counselor.alive() && bot.jason_shots.contains(&"broken") && bot.door_hits.len() == hits + 1
                    }
                };
                // The fear events the run must have pushed.
                let fear: &[&str] = match bot.goal {
                    Goal::Jason => &["JasonNearFearEvent", "JasonSpottedFearEvent", "GrabFearEvent"],
                    Goal::Fight => &["JasonNearFearEvent", "PickedUpWeaponFearEvent"],
                    Goal::Hide | Goal::HideSeen => &["HiddenFearEvent"],
                    Goal::LockedIn { .. } => &["IndoorFearEvent", "JasonNearFearEvent"],
                    _ => &[],
                };
                let missing: Vec<&&str> = fear.iter().filter(|e| !bot.fear_seen.iter().any(|s| s == **e)).collect();
                info!("bot: fear events pushed: {}", bot.fear_seen.join(", "));
                if !missing.is_empty() {
                    error!("bot: fear events never pushed: {missing:?}");
                }
                // The score events the run must have scored (any one of a group).
                let scored: &[&[&str]] = match bot.goal {
                    Goal::Escape => &[
                        &["PickupVehiclePart"],
                        &["CarPartRepaired"],
                        &["CarRepaired"],
                        &["Escape"],
                        &["Unscathed"],
                        &["CounselorTimeBonus"],
                        &["CompletedMatch"],
                        &["TeamEscaped"],
                    ],
                    Goal::Heal => &[&["UseFirstAid"]],
                    Goal::Door => &[&["BarricadeDoor"]],
                    Goal::Power => &[&["GridPartRepaired"]],
                    Goal::Police => &[&["GridPartRepaired"], &["CallPolice"], &["Escape"]],
                    Goal::Fight => &[&["StunJason", "StunJasonShotgun", "StunJasonFlare", "FirecrackerJasonStun", "EscapeJasonGrab", "Trapper"]],
                    // Her death ends the match.
                    Goal::Jason | Goal::HideSeen | Goal::LockedIn { .. } | Goal::Abilities | Goal::Block | Goal::CarGrab | Goal::PlacedKill => &[&["CompletedMatch"]],
                    _ => &[],
                };
                let mut unscored = Vec::new();
                if let Some(g) = &game {
                    let awards: Vec<String> = g.score.awards.iter().map(|a| format!("{} {}", a.event, a.xp)).collect();
                    info!("bot: scored {} XP: {}", g.score.total(), awards.join(", "));
                    unscored = scored.iter().filter(|any| !any.iter().any(|e| g.score.has(e))).collect();
                    if !unscored.is_empty() {
                        error!("bot: never scored {unscored:?}");
                    }
                }
                exit.write(if ok && missing.is_empty() && unscored.is_empty() { AppExit::Success } else { AppExit::error() });
            }
        }
    }
}

/// Records the fear events that have been active.
fn watch_fear(mut bot: ResMut<Bot>, fear: Query<&f13_bevy::CounselorFear>) {
    for f in &fear {
        for e in f.0.events.iter().filter(|e| e.state.active) {
            if !bot.fear_seen.contains(&e.key) {
                info!("bot: fear event {} ({:.1} fear)", e.key, f.0.fear);
                bot.fear_seen.push(e.key.clone());
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
