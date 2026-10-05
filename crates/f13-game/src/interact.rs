//! Counselor interactions: picking up items (one large item in the hands, up to `MaxSmallItems` small ones in the
//! pockets), opening cabinets, installing car parts and getting into the driver's seat (usable once the parts
//! installed from outside are in; the keys are used at the ignition from inside, see vehicle.rs).
//!
//! Controls: E interact (hold for repairs), G drop the held item, 1-3 use the small item in that pocket
//! (first aid: heals when its use animation ends; the counselor can't move meanwhile).
//! Placeholders until the matching game code is ported: opening a cabinet is instant, a repair is a plain hold of
//! [`REPAIR_SECONDS`] instead of the pip minigame.

use crate::player::{Player, PlayerCamera, PlayerTuning};
use crate::props::{Cabinet, Drawer, InCabinet, Item, Vehicle};
use crate::vehicle::Driving;
use bevy::prelude::*;

/// Counselor `MaxSmallItems` default (constructor).
pub const MAX_SMALL_ITEMS: usize = 3;
/// Placeholder for the repair minigame.
pub const REPAIR_SECONDS: f32 = 3.0;
/// Interaction ranges are measured from the counselor's capsule surface, roughly.
const CAPSULE_RADIUS_CM: f32 = 34.0;
/// Interaction distance is measured from about chest height.
const CHEST_M: f32 = 1.0;

#[derive(Component, Default, Debug)]
pub struct Inventory {
    pub hand: Option<Entity>,
    pub pockets: Vec<Entity>,
}

/// Class of the first aid spray.
pub const FIRST_AID: &str = "BP_FirstAid_C";

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
}

#[derive(Resource, Default)]
struct Interaction {
    target: Option<Target>,
    held: f32,
}

#[derive(Component)]
struct PromptText;

pub struct InteractPlugin;

impl Plugin for InteractPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Interaction>()
            .add_systems(Startup, spawn_prompt)
            .add_systems(Update, (add_inventory, choose_target, act, drop_item, use_item, update_prompt).chain());
    }
}

fn spawn_prompt(mut commands: Commands) {
    commands.spawn((
        Text::new(""),
        TextFont { font_size: FontSize::Px(18.0), ..default() },
        Node { position_type: PositionType::Absolute, bottom: Val::Px(16.0), left: Val::Px(12.0), ..default() },
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
    player: Single<(&Transform, &Inventory), (With<Player>, Without<Driving>)>,
    camera: Single<&PlayerCamera>,
    items: Query<(Entity, &Item, &GlobalTransform, Option<&InCabinet>), Without<Carried>>,
    carried: Query<&Item, With<Carried>>,
    cabinets: Query<(Entity, &Cabinet, &GlobalTransform)>,
    vehicles: Query<(Entity, &Vehicle, &GlobalTransform)>,
    mut interaction: ResMut<Interaction>,
) {
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
    let target = best.map(|(_, t)| t);
    if target != interaction.target {
        interaction.held = 0.0;
    }
    interaction.target = target;
}

#[allow(clippy::too_many_arguments)]
fn act(
    mut commands: Commands,
    keys: Res<ButtonInput<KeyCode>>,
    time: Res<Time>,
    mut interaction: ResMut<Interaction>,
    mut player: Single<(Entity, &Transform, &mut Inventory), (With<Player>, Without<Driving>)>,
    items: Query<&Item>,
    mut cabinets: Query<&mut Cabinet>,
    mut drawers: Query<(&Drawer, &mut Transform), Without<Player>>,
    mut vehicles: Query<&mut Vehicle>,
    mut visibility: Query<&mut Visibility>,
) {
    let Some(target) = interaction.target else { return };
    let (player_entity, player_xf, ref mut inv) = *player;
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
                interaction.held = 0.0;
                return;
            }
            interaction.held += time.delta_secs();
            if interaction.held < REPAIR_SECONDS {
                return;
            }
            let Ok(mut vehicle) = vehicles.get_mut(v) else { return };
            let parts = vehicle.repairs[i].parts.clone();
            let Some(used) = take_part(inv, &items, &parts) else { return };
            commands.entity(used).despawn();
            vehicle.repairs[i].done = true;
            info!("{}: {} installed", vehicle.class, vehicle.repairs[i].name);
            interaction.held = 0.0;
            interaction.target = None;
        }
        Target::Seat(v) if keys.just_pressed(KeyCode::KeyE) => {
            commands.entity(player_entity).insert(Driving { vehicle: v, starting: 0.0 });
            info!("got in the driver's seat");
            if let Ok(mut vis) = visibility.get_mut(player_entity) {
                *vis = Visibility::Hidden;
            }
            interaction.target = None;
        }
        _ => {}
    }
}

/// Puts an item back in the world at the counselor's feet.
fn drop_at(commands: &mut Commands, visibility: &mut Query<&mut Visibility>, item: Entity, player: &Transform) {
    let forward = player.rotation * Vec3::Z;
    let at = player.translation + forward * 0.5 + Vec3::Y * 0.05;
    commands.entity(item).remove::<Carried>().insert(Transform::from_translation(at));
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
    player: Single<(&Inventory, Has<Driving>, Option<&UsingItem>), With<Player>>,
    items: Query<&Item>,
    vehicles: Query<&Vehicle>,
    mut text: Single<&mut Text, With<PromptText>>,
) {
    let (inv, driving, using) = *player;
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
    if let Some(u) = using {
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
                    let pct = (interaction.held / REPAIR_SECONDS * 100.0).min(100.0);
                    lines.push(format!("[Hold E] Repair: {} ({pct:.0}%)", v.repairs[i].name));
                }
            }
            Some(Target::Seat(_)) => lines.push("[E] Get in (driver)".into()),
            None => {}
        }
        if inv.hand.is_some() {
            lines.push("[G] Drop".into());
        }
        if let Some(i) = inv.pockets.iter().position(|&e| items.get(e).is_ok_and(|it| it.class == FIRST_AID)) {
            lines.push(format!("[{}] Use first aid", i + 1));
        }
    }
    text.0 = lines.join("\n");
}
