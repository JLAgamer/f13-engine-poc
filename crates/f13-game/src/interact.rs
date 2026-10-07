//! Counselor interactions: picking up items (one large item in the hands, up to `MaxSmallItems` small ones in the
//! pockets), opening cabinets, installing car parts and getting into the driver's seat (usable once the parts
//! installed from outside are in; the keys are used at the ignition from inside, see vehicle.rs).
//!
//! Controls: E interact (hold for repairs), G drop the held item, 1-3 use the small item in that pocket
//! (first aid: heals when its use animation ends; the counselor can't move meanwhile).
//! Repairs are the game's hold timer and pip minigame (f13-core::minigame): hold E; when the needle is in a pip,
//! press its key (right mouse = key 0, left mouse = key 1, the game's MG_InteractSkill1/2). Letting go cancels.
//! PLACEHOLDER: opening a cabinet is instant.

use crate::player::{Player, PlayerCamera, PlayerTuning};
use crate::doors::Door;
use crate::windows::{Barricade, Window, WindowAction};
use crate::match_flow::ClassScore;
use crate::props::{Cabinet, Drawer, InCabinet, Item, Vehicle};
use crate::vehicle::Driving;
use bevy::prelude::*;
use f13_core::minigame::{HoldInteraction, MinigameEvent};

/// Counselor `MaxSmallItems` default (constructor).
pub const MAX_SMALL_ITEMS: usize = 3;
/// Interaction ranges are measured from the counselor's capsule surface, roughly.
const CAPSULE_RADIUS_CM: f32 = 34.0;
/// Interaction distance is measured from about chest height.
pub(crate) const CHEST_M: f32 = 1.0;

#[derive(Component, Default, Debug)]
pub struct Inventory {
    pub hand: Option<Entity>,
    pub pockets: Vec<Entity>,
}

/// Class of the first aid spray.
pub const FIRST_AID: &str = "BP_FirstAid_C";
/// The door barricade's class (its score event).
const BARRICADE: &str = "Barricade_C";

/// The counselor is using a pocket item (playing its use animation).
#[derive(Component)]
pub struct UsingItem {
    pub item: Entity,
    pub left: f32,
}

/// An item someone is holding or carrying in a pocket.
#[derive(Component)]
pub struct Carried;

/// Takes the first carried item of one of `classes` out of the inventory (hands first, then pockets).
pub fn take_part(inv: &mut Inventory, items: &Query<&Item>, classes: &[String]) -> Option<Entity> {
    let fits = |e: &Entity| items.get(*e).is_ok_and(|it| classes.contains(&it.class));
    let used = inv.hand.filter(fits).or_else(|| inv.pockets.iter().copied().find(fits))?;
    if inv.hand == Some(used) {
        inv.hand = None;
    }
    inv.pockets.retain(|&p| p != used);
    Some(used)
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Target {
    Item(Entity),
    Cabinet(Entity),
    Repair(Entity, usize),
    Seat(Entity),
    Door(Entity),
    Window(Entity),
    Hide(Entity),
}

#[derive(Resource, Default)]
pub struct Interaction {
    target: Option<Target>,
    /// The hold interaction (repair) in progress.
    pub hold: Option<HoldInteraction>,
    /// A hold finished while E is still down: its release isn't a press.
    hold_done: bool,
}

impl Interaction {
    /// The counselor is busy with a started hold interaction (can't walk).
    pub fn busy(&self) -> bool {
        self.hold.as_ref().is_some_and(|h| h.started)
    }
}

#[derive(Component)]
struct PromptText;

pub struct InteractPlugin;

impl Plugin for InteractPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Interaction>()
            .add_systems(Startup, spawn_prompt)
            .add_systems(
            Update,
            (add_inventory, crate::windows::link_barricades, choose_target, act, drop_item, use_item, update_prompt)
                .chain()
                .before(crate::doors::swing_doors),
        )
        .add_systems(Update, (crate::doors::swing_doors, crate::windows::update_window_visuals));
    }
}

fn spawn_prompt(mut commands: Commands) {
    commands.spawn((
        Text::new(""),
        TextFont { font_size: FontSize::Px(18.0), ..default() },
        // Under the HUD's timer (the inventory takes the bottom left).
        Node { position_type: PositionType::Absolute, top: Val::Percent(20.0), left: Val::Px(12.0), ..default() },
        PromptText,
    ));
}

fn add_inventory(mut commands: Commands, players: Query<Entity, (With<Player>, Without<Inventory>)>) {
    for p in &players {
        commands.entity(p).insert(Inventory::default());
    }
}

fn has_part(inv: &Inventory, carried: &Query<&Item, With<Carried>>, parts: &[String]) -> bool {
    inv.hand.iter().chain(inv.pockets.iter()).filter_map(|&e| carried.get(e).ok()).any(|i| parts.contains(&i.class))
}

/// Picks what the counselor interacts with: in reach, preferring what the camera looks at, then what's close
/// (score = distance x (1 + 2 x view angle / pi)). PLACEHOLDER weighting: the game's interactable manager
/// also ranks by view, but its exact formula isn't ported.
fn choose_target(
    player: Single<(&Transform, &Inventory), (With<Player>, Without<Driving>, Without<crate::hiding::Hiding>)>,
    camera: Single<&PlayerCamera>,
    items: Query<(Entity, &Item, &GlobalTransform, Option<&InCabinet>), Without<Carried>>,
    carried: Query<&Item, With<Carried>>,
    cabinets: Query<(Entity, &Cabinet, &GlobalTransform)>,
    vehicles: Query<(Entity, &Vehicle, &GlobalTransform)>,
    doors: Query<(Entity, &Door, &GlobalTransform)>,
    windows: Query<(Entity, &Window, &GlobalTransform)>,
    barricades: Query<&Barricade>,
    spots: Query<(Entity, &crate::hiding::HidingSpot, &GlobalTransform)>,
    mut interaction: ResMut<Interaction>,
) {
    if interaction.busy() {
        return; // keep the target while repairing
    }
    let (xf, inv) = *player;
    let chest = xf.translation + Vec3::Y * CHEST_M;
    let look = camera.look();
    let mut best: Option<(f32, Target)> = None;
    let mut consider = |point: Vec3, reach_cm: f32, t: Target| {
        let d = point.distance(chest) * 100.0;
        if d > reach_cm + CAPSULE_RADIUS_CM {
            return;
        }
        let angle = (point - chest).try_normalize().map_or(0.0, |dir| dir.angle_between(look));
        let score = d * (1.0 + 2.0 * angle / std::f32::consts::PI);
        if best.is_none_or(|(bs, _)| score < bs) {
            best = Some((score, t));
        }
    };
    for (e, item, gxf, in_cab) in &items {
        if in_cab.is_some_and(|c| cabinets.get(c.0).is_ok_and(|(_, cab, _)| !cab.open)) {
            continue;
        }
        if !item.large && inv.pockets.len() >= MAX_SMALL_ITEMS {
            continue; // can't be taken, so don't let it hide what's next to it
        }
        consider(gxf.translation(), item.reach, Target::Item(e));
    }
    for (e, cab, gxf) in &cabinets {
        if !cab.open {
            consider(gxf.transform_point(cab.interact), cab.reach, Target::Cabinet(e));
        }
    }
    for (e, v, gxf) in &vehicles {
        for (i, r) in v.repairs.iter().enumerate() {
            if !r.done && has_part(inv, &carried, &r.parts) {
                consider(gxf.transform_point(r.at), r.reach, Target::Repair(e, i));
            }
        }
        if v.repaired() {
            consider(gxf.transform_point(v.driver_seat), v.seat_reach + 60.0, Target::Seat(e));
        }
    }
    for (e, d, gxf) in &doors {
        let (press, hold) = d.can_interact(gxf, xf.translation, barricade_of(&barricades, e));
        if press || hold {
            consider(gxf.transform_point(d.interact), d.reach, Target::Door(e));
        }
    }
    for (e, w, gxf) in &windows {
        consider(gxf.transform_point(w.interact), w.reach, Target::Window(e));
    }
    for (e, s, gxf) in &spots {
        if s.usable() {
            consider(gxf.transform_point(s.interact), s.reach, Target::Hide(e));
        }
    }
    let target = best.map(|(_, t)| t);
    if target != interaction.target {
        interaction.hold = None;
    }
    interaction.target = target;
}

#[allow(clippy::too_many_arguments)]
fn act(
    mut commands: Commands,
    keys: Res<ButtonInput<KeyCode>>,
    time: Res<Time>,
    mut interaction: ResMut<Interaction>,
    mouse: Res<ButtonInput<MouseButton>>,
    tuning: Res<PlayerTuning>,
    mut rng: ResMut<crate::GameRng>,
    mut player: Single<(Entity, &mut Transform, &mut Inventory, &mut Player), (Without<Driving>, Without<crate::hiding::Hiding>)>,
    items: Query<&Item>,
    (mut spots, model): (Query<(&mut crate::hiding::HidingSpot, &GlobalTransform)>, Option<Res<crate::character::CounselorCharacter>>),
    mut cabinets: Query<&mut Cabinet>,
    mut drawers: Query<(&Drawer, &mut Transform), Without<Player>>,
    mut vehicles: Query<&mut Vehicle>,
    mut doors: Query<(&mut Door, &GlobalTransform)>,
    (mut windows, mut barricades): (Query<(&mut Window, &GlobalTransform)>, Query<&mut Barricade>),
    mut visibility: Query<&mut Visibility>,
) {
    let Some(target) = interaction.target else { return };
    let (player_entity, ref mut player_xf, ref mut inv, ref mut p) = *player;
    match target {
        Target::Item(e) if keys.just_pressed(KeyCode::KeyE) => {
            let Ok(item) = items.get(e) else { return };
            if item.large {
                if let Some(old) = inv.hand.take() {
                    drop_at(&mut commands, &mut visibility, old, player_xf);
                }
                inv.hand = Some(e);
            } else if inv.pockets.len() < MAX_SMALL_ITEMS {
                inv.pockets.push(e);
            } else {
                return;
            }
            info!("picked up {}", item.name);
            commands.trigger(ClassScore::new(&item.class, "PickupScoreEvent"));
            commands.entity(e).insert(Carried).remove::<InCabinet>();
            if let Ok(mut v) = visibility.get_mut(e) {
                *v = Visibility::Hidden;
            }
            interaction.target = None;
        }
        Target::Cabinet(e) if keys.just_pressed(KeyCode::KeyE) => {
            let Ok(mut cab) = cabinets.get_mut(e) else { return };
            cab.open = true;
            // Slide the drawer out toward the front (-Y in the cabinet's Unreal space, -Z here).
            if let Some((d, mut t)) = cab.drawer.and_then(|d| drawers.get_mut(d).ok()) {
                t.translation = d.closed.translation + Vec3::new(0.0, 0.0, -0.3);
            }
            interaction.target = None;
        }
        Target::Repair(v, i) => {
            if !keys.pressed(KeyCode::KeyE) {
                if interaction.hold.take().is_some_and(|h| h.started) {
                    info!("repair cancelled");
                }
                return;
            }
            let Ok(mut vehicle) = vehicles.get_mut(v) else { return };
            let r = &vehicle.repairs[i];
            let hold = interaction.hold.get_or_insert_with(|| HoldInteraction::new(r.hold_time.unwrap_or(tuning.repair_hold_time), r.has_pips));
            let mut events = Vec::new();
            for (button, key) in [(MouseButton::Right, 0), (MouseButton::Left, 1)] {
                if mouse.just_pressed(button) {
                    hold.press(key, &tuning.minigame, &mut events);
                }
            }
            hold.tick(time.delta_secs(), &p.counselor.stats, &tuning.minigame, &mut rng.0, &mut events);
            let mut completed = false;
            for e in events {
                match e {
                    MinigameEvent::Started => info!("repair started: {} pips, {:.1}s at x{:.2}", hold.pips.len(), hold.hold_time, tuning.minigame.speed_mod.value(&p.counselor.stats, false)),
                    MinigameEvent::Hit => info!("pip hit (bonus x{:.1})", hold.bonus),
                    MinigameEvent::Missed => info!("pip missed"),
                    MinigameEvent::NotifyKiller => warn!("repair failed noisily: Jason would be notified"),
                    MinigameEvent::Completed => completed = true,
                }
            }
            if !completed {
                return;
            }
            let parts = vehicle.repairs[i].parts.clone();
            let Some(used) = take_part(inv, &items, &parts) else { return };
            commands.entity(used).despawn();
            vehicle.repairs[i].done = true;
            info!("{}: {} installed", vehicle.class, vehicle.repairs[i].name);
            commands.trigger(ClassScore::new(&vehicle.class, "PartRepairedScoreEvent"));
            if vehicle.repairs.iter().all(|r| r.done) {
                commands.trigger(ClassScore::new(&vehicle.class, "VehicleRepairedScoreEvent"));
            }
            interaction.hold = None;
            interaction.target = None;
        }
        Target::Door(d) => {
            let Ok((mut door, gxf)) = doors.get_mut(d) else { return };
            if interaction.hold_done {
                interaction.hold_done = keys.pressed(KeyCode::KeyE);
                return;
            }
            let barricade = barricades.iter().find(|b| b.door == Some(d)).map(|b| b.enabled);
            let (press, hold) = door.can_interact(gxf, player_xf.translation, barricade);
            // Press = let go before the hold starts; hold = keep holding for the door's hold time.
            let started = interaction.hold.as_ref().is_some_and(|h| h.started);
            if keys.just_released(KeyCode::KeyE) && !started && press {
                let what = door.press(gxf, player_xf.translation);
                info!("door {what}");
                if what == "locked" {
                    commands.trigger(crate::audio::ActorSound { actor: d, prop: "LockedDoorSound" });
                }
                interaction.hold = None;
                return;
            }
            if !keys.pressed(KeyCode::KeyE) || !hold {
                interaction.hold = None;
                return;
            }
            let h = interaction.hold.get_or_insert_with(|| HoldInteraction::new(door.hold_time, false));
            let mut events = Vec::new();
            h.tick(time.delta_secs(), &p.counselor.stats, &tuning.minigame, &mut rng.0, &mut events);
            if events.contains(&MinigameEvent::Completed) {
                let what = match barricades.iter_mut().find(|b| b.door == Some(d)) {
                    // A barricade only moves while the door is shut (OnUsedBarricade).
                    Some(mut b) if !door.open => {
                        b.enabled = !b.enabled;
                        if b.enabled {
                            commands.trigger(ClassScore::new(BARRICADE, "BarricadeDoorScoreEvent"));
                        }
                        if b.enabled { "barricaded" } else { "unbarricaded" }
                    }
                    Some(_) => "open (can't barricade)",
                    None => door.toggle_lock(),
                };
                info!("door {what}");
                interaction.hold = None;
                interaction.hold_done = true;
                interaction.target = None;
            }
        }
        Target::Window(w) => {
            let Ok((mut window, gxf)) = windows.get_mut(w) else { return };
            if interaction.hold_done {
                interaction.hold_done = keys.pressed(KeyCode::KeyE);
                return;
            }
            let started = interaction.hold.as_ref().is_some_and(|h| h.started);
            if keys.just_released(KeyCode::KeyE) && !started {
                interaction.hold = None;
                let action = window.press_action(p.counselor.sprinting, p.counselor.wounded);
                match action {
                    WindowAction::Open => window.open = true,
                    _ => {
                        // Through the window (instantly, for now), taking any damage.
                        let damage = window.damage(action);
                        if action == WindowAction::Dive && !window.open {
                            window.destroyed = true;
                        }
                        player_xf.translation = Window::landing(gxf, window.interact, player_xf.translation);
                        if damage > 0.0 {
                            p.counselor.take_damage(f13_core::health::Hit { amount: damage, ..Default::default() }, &tuning.counselor);
                        }
                        p.velocity = Vec2::ZERO;
                        info!("window {action:?}: {damage:.0} damage, health {:.0}", p.counselor.health);
                        interaction.target = None;
                        return;
                    }
                }
                info!("window {action:?}");
                return;
            }
            if !keys.pressed(KeyCode::KeyE) || !window.can_hold() {
                interaction.hold = None;
                return;
            }
            let h = interaction.hold.get_or_insert_with(|| HoldInteraction::new(0.5, false));
            let mut events = Vec::new();
            h.tick(time.delta_secs(), &p.counselor.stats, &tuning.minigame, &mut rng.0, &mut events);
            if events.contains(&MinigameEvent::Completed) {
                window.open = false;
                info!("window Close");
                interaction.hold = None;
                interaction.hold_done = true;
            }
        }
        Target::Hide(s) if keys.just_pressed(KeyCode::KeyE) && p.counselor.alive() => {
            let (Ok((mut spot, gxf)), Some(model)) = (spots.get_mut(s), model.as_ref()) else { return };
            if spot.usable() {
                crate::hiding::enter(&mut commands, player_entity, player_xf, p, s, &mut spot, gxf, model, time.elapsed_secs());
            }
            interaction.target = None;
        }
        Target::Seat(v) if keys.just_pressed(KeyCode::KeyE) => {
            commands.entity(player_entity).insert(Driving { vehicle: v, starting: None });
            info!("got in the driver's seat");
            if let Ok(mut vis) = visibility.get_mut(player_entity) {
                *vis = Visibility::Hidden;
            }
            interaction.target = None;
        }
        _ => {}
    }
}

/// The minigame dial as text: `=` done, `R`/`L` pips by key (lower case once hit), `|` the needle.
fn dial(h: &HoldInteraction, t: &f13_core::minigame::MinigameTuning) -> String {
    const CELLS: usize = 72; // 5 degrees each
    let needle = ((h.progress(t) * CELLS as f32) as usize).min(CELLS - 1);
    (0..CELLS)
        .map(|c| {
            let deg = c as f32 * 5.0 + 2.5;
            let pip = h.pips.iter().find(|p| deg >= p.position && deg <= p.position + p.width);
            if c == needle {
                '|'
            } else if let Some(p) = pip {
                let ch = if p.key == 0 { 'R' } else { 'L' };
                if p.state == f13_core::minigame::PipState::Hit { ch.to_ascii_lowercase() } else { ch }
            } else if c < needle {
                '='
            } else {
                '.'
            }
        })
        .collect()
}

fn barricade_of(barricades: &Query<&Barricade>, door: Entity) -> Option<bool> {
    barricades.iter().find(|b| b.door == Some(door)).map(|b| b.enabled)
}

/// Puts an item back in the world at the counselor's feet.
fn drop_at(commands: &mut Commands, visibility: &mut Query<&mut Visibility>, item: Entity, player: &Transform) {
    let forward = player.rotation * Vec3::Z;
    let at = player.translation + forward * 0.5 + Vec3::Y * 0.05;
    commands.entity(item).remove::<(Carried, ChildOf, crate::items::InHand)>().insert(Transform::from_translation(at));
    if let Ok(mut v) = visibility.get_mut(item) {
        *v = Visibility::Inherited;
    }
}

fn drop_item(
    mut commands: Commands,
    keys: Res<ButtonInput<KeyCode>>,
    mut player: Single<(&Transform, &mut Inventory), (With<Player>, Without<Driving>)>,
    mut visibility: Query<&mut Visibility>,
) {
    if !keys.just_pressed(KeyCode::KeyG) {
        return;
    }
    let (xf, ref mut inv) = *player;
    if let Some(item) = inv.hand.take() {
        drop_at(&mut commands, &mut visibility, item, xf);
    }
}

/// 1-3: use the small item in that pocket. Only first aid has a use so far.
fn use_item(
    mut commands: Commands,
    keys: Res<ButtonInput<KeyCode>>,
    time: Res<Time>,
    tuning: Res<PlayerTuning>,
    mut player: Single<(Entity, &mut Player, &mut Inventory, Option<&mut UsingItem>), Without<Driving>>,
    items: Query<&Item>,
) {
    let (entity, ref mut p, ref mut inv, ref mut using) = *player;
    if let Some(u) = using {
        u.left -= time.delta_secs();
        if u.left > 0.0 {
            return;
        }
        let healed = p.counselor.heal(tuning.first_aid.health_restored, &tuning.counselor);
        info!("used first aid: +{healed:.0} health");
        commands.trigger(ClassScore::new(FIRST_AID, "UseFirstAidScoreEvent"));
        inv.pockets.retain(|&e| e != u.item);
        commands.entity(u.item).despawn();
        commands.entity(entity).remove::<UsingItem>();
        return;
    }
    let slot = [KeyCode::Digit1, KeyCode::Digit2, KeyCode::Digit3].iter().position(|&k| keys.just_pressed(k));
    let Some(&item) = slot.and_then(|i| inv.pockets.get(i)) else { return };
    if !p.counselor.alive() || items.get(item).map_or(true, |i| i.class != FIRST_AID) {
        return;
    }
    if p.counselor.health >= tuning.counselor.health.max_health {
        info!("already at full health");
        return;
    }
    commands.entity(entity).insert(UsingItem { item, left: tuning.first_aid.use_length });
}

fn update_prompt(
    interaction: Res<Interaction>,
    tuning: Res<PlayerTuning>,
    doors: Query<(&Door, &GlobalTransform)>,
    (windows, barricades): (Query<&Window>, Query<&Barricade>),
    player: Single<(&Inventory, Has<Driving>, &Transform, Option<&UsingItem>, &Player, Has<crate::jason::Held>, Option<&crate::hiding::Hiding>)>,
    items: Query<&Item>,
    vehicles: Query<&Vehicle>,
    mut text: Single<&mut Text, With<PromptText>>,
) {
    let (inv, driving, _, using, p, held, hiding) = *player;
    let name = |e: Entity| items.get(e).map_or("?".to_string(), |i| i.name.clone());
    let mut lines = vec![format!(
        "Hands: {}   Pockets: [{}] ({}/{})",
        inv.hand.map_or("-".to_string(), name),
        inv.pockets.iter().map(|&e| name(e)).collect::<Vec<_>>().join(", "),
        inv.pockets.len(),
        MAX_SMALL_ITEMS
    )];
    for v in &vehicles {
        let mut parts: Vec<String> = v.repairs.iter().map(|r| format!("{} {}", r.name.trim_end_matches("Repair"), if r.done { "ok" } else { "--" })).collect();
        parts.push(format!("Ignition {}", if v.started { "running" } else { "--" }));
        lines.push(format!("{}: {}", v.class.trim_end_matches("_C"), parts.join(", ")));
    }
    if let Some(h) = hiding {
        lines.push(match h.phase {
            crate::hiding::HidePhase::Hidden => "Hiding   [E] Leave   (mouse: look around)".into(),
            crate::hiding::HidePhase::Killed => String::new(),
            _ => "...".into(),
        });
    } else if let Some(u) = using {
        lines.push(format!("Using {}... {:.1}s", name(u.item), u.left.max(0.0)));
    } else if driving {
        lines.push("[Hold E] Start (needs keys)   [W/S/A/D] Drive   [F] Get out".into());
    } else {
        match interaction.target {
            Some(Target::Item(e)) => {
                let full = items.get(e).is_ok_and(|i| !i.large) && inv.pockets.len() >= MAX_SMALL_ITEMS;
                lines.push(if full { format!("Pockets full ({})", name(e)) } else { format!("[E] Pick up {}", name(e)) });
            }
            Some(Target::Cabinet(_)) => lines.push("[E] Search".into()),
            Some(Target::Repair(v, i)) => {
                if let Ok(v) = vehicles.get(v) {
                    match &interaction.hold {
                        Some(h) if h.started => {
                            lines.push(format!("Repairing {}: {:.0}%  (RMB/LMB when the needle | is in a pip)", v.repairs[i].name, h.progress(&tuning.minigame) * 100.0));
                            lines.push(dial(h, &tuning.minigame));
                        }
                        _ => lines.push(format!("[Hold E] Repair: {}", v.repairs[i].name)),
                    }
                }
            }
            Some(Target::Seat(_)) => lines.push("[E] Get in (driver)".into()),
            Some(Target::Hide(_)) => lines.push("[E] Hide".into()),
            Some(Target::Window(w)) => {
                if let Ok(window) = windows.get(w) {
                    let press = match window.press_action(p.counselor.sprinting, p.counselor.wounded) {
                        WindowAction::Dive => "[E] Dive through",
                        WindowAction::Open => "[E] Open window",
                        _ => "[E] Climb through",
                    };
                    lines.push(if window.can_hold() { format!("{press}   [Hold E] Close") } else { press.to_string() });
                }
            }
            Some(Target::Door(d)) => {
                if let Ok((door, gxf)) = doors.get(d) {
                    let barricade = barricades.iter().find(|b| b.door == Some(d)).map(|b| b.enabled);
                    let (press, hold) = door.can_interact(gxf, player.2.translation, barricade);
                    let mut s = Vec::new();
                    if press {
                        s.push(if door.open { "[E] Close door" } else { "[E] Open door" });
                    }
                    if hold {
                        s.push(match barricade {
                            Some(true) => "[Hold E] Remove barricade",
                            Some(false) => "[Hold E] Barricade",
                            None if door.locked => "[Hold E] Unlock",
                            None => "[Hold E] Lock",
                        });
                    }
                    if barricade == Some(true) && !hold {
                        s.push("Barricaded");
                    }
                    lines.push(s.join("   "));
                }
            }
            None => {}
        }
        if inv.hand.is_some() {
            lines.push("[G] Drop".into());
        }
        if let Some(i) = inv.pockets.iter().position(|&e| items.get(e).is_ok_and(|it| it.class == FIRST_AID)) {
            lines.push(format!("[{}] Use first aid", i + 1));
        }
        use crate::props::ItemKind;
        match inv.hand.and_then(|e| items.get(e).ok()).map(|i| &i.kind) {
            Some(ItemKind::Weapon(_)) => lines.push("[LMB] Swing   [W + LMB] Heavy swing".into()),
            Some(ItemKind::Gun { .. }) => lines.push("[LMB] Fire (aim with the camera)".into()),
            Some(ItemKind::Trap) => lines.push("[LMB] Place the bear trap".into()),
            _ => {}
        }
        for (i, it) in inv.pockets.iter().filter_map(|&e| items.get(e).ok()).enumerate() {
            match it.kind {
                ItemKind::Firecracker { .. } => lines.push(format!("[{}] Throw firecrackers", i + 1)),
                ItemKind::PocketKnife => lines.push("Pocket knife: frees you when Jason grabs you".into()),
                ItemKind::Map => lines.push("[M] Map".into()),
                ItemKind::WalkieTalkie => lines.push("Walkie-talkie: talks to other players (no use offline)".into()),
                _ => {}
            }
        }
        if held {
            lines.push("GRABBED: mash A / D to break free".into());
        }
    }
    text.0 = lines.join("\n");
}
