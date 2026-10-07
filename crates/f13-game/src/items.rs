//! Using items against Jason: swinging a weapon, firing the flare gun or the shotgun, throwing a firecracker,
//! placing a bear trap; the pocket knife (breaks a grab) and the grab struggle are in jason.rs. The held item hangs
//! off its hand socket. The map shows the camp from above, with a dot where she is.
//!
//! Controls: left mouse uses the item in the hands (swing; moving forward swings the heavy ForwardAttack), 1-3 the
//! pocket items (first aid in interact.rs, firecrackers here), M the map (when carried). In combat stance, right
//! mouse blocks with a weapon in the hands.
//!
//! Blocking (SCCharacter::OnStartBlock / CanBlock / SCCounselorCharacter::TakeDamage): it starts only with a weapon
//! in hand, alive, not attacking, stunned or interacting, and lasts while the button is held. A hit taken while
//! blocking is scaled by her BlockingDamageMod (f13-core::health) and wears the weapon by its BlockingWear (a weapon
//! worn out breaks); she plays the block reaction (AM_F_Combat_Stand_Block_React) instead of a hit reaction. A
//! killer's heavy attack ends the block (Jason doesn't heavy attack yet). BlockStaminaCost isn't read by the game's
//! code. PLACEHOLDER: the block idle replaces the combat idle (the game plays it in an upper-body slot).
//!
//! From the game's data: WeaponStats (Damage, StunChance, Durability, AttackingWear), the swings' AttackTraceNotify
//! windows, the stun classes' times (jason.rs), the firecracker's ThrowDelayTime, BP_Firecracker_Projectile's
//! ActivationTime 1.5 s, NoiseTimer 20 s and BlastRadius 200 cm, the flare gun's ProjectileVelocity and the
//! shotgun's NumPellets and MaxSpreadAngle.
//!
//! PLACEHOLDERS (not from the game): a swing connects with Jason within [`SWING_REACH_M`] and [`SWING_ANGLE_DEG`] of
//! her facing (the game traces the weapon mesh); the firecracker's throw speed (its VelocityPitchCurve isn't
//! exported); when the gun fires in its animation; the shotgun's range; guns hold one shot. Counselors don't step in
//! their own traps.

use bevy::prelude::*;
use f13_core::collision::surface_below;

use crate::character::{CounselorCharacter, PlayAction, Rig};
use crate::interact::{Carried, Inventory};
use crate::jason::{HitJason, Jason, Stun};
use crate::player::{Player, PlayerCamera};
use crate::props::{Item, ItemKind, WeaponStats};
use crate::match_flow::{ClassScore, Score};

const SWING_REACH_M: f32 = 1.8;
const SWING_ANGLE_DEG: f32 = 60.0;
const SHOTGUN_RANGE_M: f32 = 25.0;
/// Firecracker throw: forward and up speed (m/s).
const THROW_SPEED: (f32, f32) = (7.0, 3.0);
/// BP_Firecracker_Projectile: ActivationTime, NoiseTimer, BlastRadius (m).
const FIRECRACKER_ACTIVATION: f32 = 1.5;
const FIRECRACKER_NOISE: f32 = 20.0;
const FIRECRACKER_BLAST_M: f32 = 2.0;
/// FlareGun: ProjectileLifespan.
const FLARE_LIFESPAN: f32 = 7.0;
/// A bear trap snaps on a foot this close (m).
const TRAP_RADIUS_M: f32 = 0.45;
/// Jason's capsule (for projectiles).
const JASON_RADIUS_M: f32 = 0.34;
const JASON_HEIGHT_M: f32 = 1.76;

const SWING_1H: [&str; 2] = ["f_combat_stand_1h_melee_light_01", "f_combat_stand_1h_melee_heavy_01"];
const SWING_2H: [&str; 2] = ["f_combat_stand_2h_melee_light_01", "f_combat_stand_2h_melee_heavy_01"];
const FLARE_FIRE: &str = "f_normal_stand_flare_shoot";
const SHOTGUN_FIRE: &str = "combat_stand_rifle_fire_01";
const THROW: &str = "m_normal_stand_firecracker_throw";
const PLACE_TRAP: &str = "f_normal_stand_trap_place";

pub struct ItemsPlugin;

impl Plugin for ItemsPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(PostUpdate, give_items)
            .add_systems(Update, (hold_in_hand, start_action, block, toggle_map, follow_map).chain())
            .add_observer(wear_on_block)
            .add_systems(FixedUpdate, (tick_action, fly, spring_traps).chain().after(crate::player::move_player));
    }
}

/// Testing (--give): item classes the counselor starts with, taken from the match's spawned items.
#[derive(Resource)]
pub struct Give(pub Vec<String>);

fn give_items(
    mut commands: Commands,
    give: Option<Res<Give>>,
    mut player: Query<&mut Inventory, With<Player>>,
    items: Query<(Entity, &Item), Without<Carried>>,
    mut visibility: Query<&mut Visibility>,
) {
    let Some(give) = give else { return };
    let Ok(mut inv) = player.single_mut() else { return };
    if items.is_empty() {
        return; // the match hasn't spawned yet
    }
    for class in &give.0 {
        let Some((e, item)) = items.iter().find(|(e, i)| &i.class == class && inv.hand != Some(*e) && !inv.pockets.contains(e)) else {
            warn!("--give: no {class} in this match");
            continue;
        };
        if item.large {
            inv.hand = Some(e);
        } else {
            inv.pockets.push(e);
        }
        info!("--give: {}", item.name);
        commands.entity(e).insert(Carried).remove::<crate::props::InCabinet>();
        if let Ok(mut v) = visibility.get_mut(e) {
            *v = Visibility::Hidden;
        }
    }
    commands.remove_resource::<Give>();
}

/// The held item hangs off the counselor's hand socket.
#[derive(Component)]
pub struct InHand;

#[derive(Clone, Debug)]
enum Use {
    Swing { stats: WeaponStats, window: (f32, f32) },
    Fire { kind: ItemKind },
    Throw,
    PlaceTrap,
}

/// The counselor is using an item: its animation runs `len` seconds; the item acts at `at` (or in the swing's window).
#[derive(Component)]
pub struct ItemAction {
    what: Use,
    item: Entity,
    t: f32,
    len: f32,
    at: f32,
    done: bool,
}

/// Wear left on a weapon (starts at its Durability).
#[derive(Component)]
struct Wear(f32);

/// The counselor is blocking.
#[derive(Component)]
pub struct Blocking;

/// A hit landed on her block: it wears her weapon.
#[derive(Event)]
pub struct BlockedHit;

/// Wears the weapon in her hands by `amount`; true when it broke (it's gone from her hands).
fn wear_weapon(commands: &mut Commands, wear: &mut Query<Option<&mut Wear>>, inv: &mut Inventory, weapon: Entity, durability: f32, amount: f32, name: &str) -> bool {
    let left = match wear.get_mut(weapon) {
        Ok(Some(mut w)) => {
            w.0 -= amount;
            w.0
        }
        _ => {
            let w = durability - amount;
            commands.entity(weapon).insert(Wear(w));
            w
        }
    };
    if left > 0.0 {
        return false;
    }
    info!("the {name} broke");
    inv.hand = None;
    commands.entity(weapon).despawn();
    true
}

/// Right mouse in combat stance: block, while held, with a weapon in her hands.
#[allow(clippy::type_complexity)]
fn block(
    mut commands: Commands,
    mouse: Res<ButtonInput<MouseButton>>,
    interaction: Res<crate::interact::Interaction>,
    player: Query<(Entity, &Player, &Inventory, Has<Blocking>, Has<ItemAction>, Has<crate::jason::Staggered>, Has<crate::jason::Held>), Without<crate::vehicle::Driving>>,
    items: Query<&Item>,
) {
    let Ok((pe, p, inv, blocking, acting, staggered, held)) = player.single() else { return };
    let c = &p.counselor;
    let weapon = inv.hand.and_then(|h| items.get(h).ok()).is_some_and(|i| matches!(i.kind, ItemKind::Weapon(_) | ItemKind::Gun { .. }));
    let combat = c.stance == f13_core::counselor::Stance::Combat;
    if blocking {
        if !mouse.pressed(MouseButton::Right) || !combat || !weapon || !c.alive() || held {
            info!("block ends");
            commands.entity(pe).remove::<Blocking>();
        }
    } else if mouse.just_pressed(MouseButton::Right) && combat && weapon && c.alive() && !acting && !staggered && !held && !interaction.busy() && !c.dodging() && p.root_motion.is_none() {
        info!("blocking");
        commands.entity(pe).insert(Blocking);
    }
}

/// A blocked hit wears the weapon in her hands by its BlockingWear.
fn wear_on_block(_: On<BlockedHit>, mut commands: Commands, mut player: Query<&mut Inventory, With<Player>>, items: Query<&Item>, mut wear: Query<Option<&mut Wear>>) {
    let Ok(mut inv) = player.single_mut() else { return };
    let Some(hand) = inv.hand else { return };
    let Ok(Item { kind: ItemKind::Weapon(stats) | ItemKind::Gun { stats, .. }, name, .. }) = items.get(hand) else { return };
    let (durability, amount, name) = (stats.durability, stats.blocking_wear, name.clone());
    wear_weapon(&mut commands, &mut wear, &mut inv, hand, durability, amount, &name);
}

#[derive(Component)]
struct Firecracker {
    vel: Vec3,
    age: f32,
    stunned: bool,
}

#[derive(Component)]
struct Flare {
    vel: Vec3,
    age: f32,
    hit: bool,
}

#[derive(Component)]
pub struct ArmedTrap;

fn hold_in_hand(
    mut commands: Commands,
    character: Option<Res<CounselorCharacter>>,
    players: Query<(Entity, &Inventory), With<Player>>,
    rigs: Query<(&ChildOf, &Rig)>,
    items: Query<(&Item, Has<InHand>)>,
    mut visibility: Query<&mut Visibility>,
) {
    let Some(character) = character else { return };
    for (pe, inv) in &players {
        let Some(hand) = inv.hand else { continue };
        let Ok((item, attached)) = items.get(hand) else { continue };
        if attached {
            continue;
        }
        let Some((_, rig)) = rigs.iter().find(|(c, _)| c.parent() == pe) else { continue };
        let socket = character.info.sockets.get(&item.hand_socket).or_else(|| character.info.sockets.get("WeaponSocket"));
        let Some(bone) = socket.and_then(|s| rig.bones.get(&s.attach)) else { continue };
        let offset = socket.and_then(|s| s.transform.as_ref()).map(crate::to_transform).unwrap_or_default();
        commands.entity(hand).insert((InHand, ChildOf(*bone), offset));
        if let Ok(mut v) = visibility.get_mut(hand) {
            *v = Visibility::Inherited;
        }
    }
}

/// Left mouse: use the item in the hands. 1-3: throw a firecracker from that pocket.
#[allow(clippy::too_many_arguments)]
fn start_action(
    mut commands: Commands,
    mouse: Res<ButtonInput<MouseButton>>,
    keys: Res<ButtonInput<KeyCode>>,
    character: Option<Res<CounselorCharacter>>,
    interaction: Res<crate::interact::Interaction>,
    camera: Single<&PlayerCamera>,
    mut player: Query<
        (Entity, &Player, &Inventory, &mut Transform),
        (Without<ItemAction>, Without<crate::interact::UsingItem>, Without<crate::jason::Held>, Without<crate::jason::Staggered>, Without<crate::vehicle::Driving>, Without<Blocking>),
    >,
    items: Query<&Item>,
) {
    let Some(character) = character else { return };
    let Ok((pe, p, inv, mut xf)) = player.single_mut() else { return };
    if !p.counselor.alive() || interaction.busy() || p.counselor.dodging() || p.root_motion.is_some() {
        return;
    }
    let len = |n: &str| character.length(n).unwrap_or(1.0);
    let mut start = |what: Use, item: Entity, anim: &str, at: f32| {
        commands.entity(pe).insert((ItemAction { what, item, t: 0.0, len: len(anim), at, done: false }, PlayAction(anim.into())));
    };
    // Face where the camera looks.
    let (sin, cos) = camera.yaw.sin_cos();
    let face = Quat::from_rotation_y((-sin).atan2(-cos));
    if mouse.just_pressed(MouseButton::Left) {
        let Some(hand) = inv.hand else { return };
        let Ok(item) = items.get(hand) else { return };
        match &item.kind {
            ItemKind::Weapon(stats) => {
                let heavy = keys.pressed(KeyCode::KeyW) as usize;
                let anim = if stats.two_handed { SWING_2H[heavy] } else { SWING_1H[heavy] };
                let window = character.info.animations.get(anim).and_then(|a| a.window("AttackTraceNotify")).unwrap_or((0.6, 0.8));
                info!("swing {} ({})", item.name, if heavy == 1 { "heavy" } else { "light" });
                xf.rotation = face;
                start(Use::Swing { stats: stats.clone(), window }, hand, anim, window.0);
            }
            ItemKind::Gun { projectile, .. } => {
                info!("fire {}", item.name);
                xf.rotation = face;
                let (anim, at) = if *projectile { (FLARE_FIRE, 0.25) } else { (SHOTGUN_FIRE, 0.1) };
                start(Use::Fire { kind: item.kind.clone() }, hand, anim, at);
            }
            ItemKind::Trap => {
                info!("placing {}", item.name);
                xf.rotation = face;
                start(Use::PlaceTrap, hand, PLACE_TRAP, len(PLACE_TRAP));
            }
            _ => {}
        }
        return;
    }
    let slot = [KeyCode::Digit1, KeyCode::Digit2, KeyCode::Digit3].iter().position(|&k| keys.just_pressed(k));
    let Some(&item) = slot.and_then(|i| inv.pockets.get(i)) else { return };
    if let Ok(Item { kind: ItemKind::Firecracker { throw_delay }, .. }) = items.get(item) {
        info!("throwing firecrackers");
        xf.rotation = face;
        start(Use::Throw, item, THROW, *throw_delay);
    }
}

/// Runs the item's action: the hit during the swing's window, the shot, the throw, the trap.
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
fn tick_action(
    mut commands: Commands,
    time: Res<Time>,
    camera: Single<(&PlayerCamera, &GlobalTransform)>,
    mut rng: ResMut<crate::GameRng>,
    mut player: Query<(Entity, &mut ItemAction, &mut Inventory, &Transform), With<Player>>,
    jasons: Query<(Entity, &Transform), (With<Jason>, Without<Player>)>,
    mut wear: Query<Option<&mut Wear>>,
    items: Query<&Item>,
    (collision, dynamic): (Res<crate::collision::WorldCollision>, Res<crate::collision::DynamicColliders>),
    mut assets: (ResMut<Assets<Mesh>>, ResMut<Assets<StandardMaterial>>),
) {
    let Ok((pe, mut a, mut inv, xf)) = player.single_mut() else { return };
    a.t += time.delta_secs();
    let chest = xf.translation + Vec3::Y * crate::interact::CHEST_M;
    let facing = xf.rotation * Vec3::Z;
    let jason = jasons.iter().next();
    if !a.done && a.t >= a.at {
        match a.what.clone() {
            Use::Swing { stats, window } => {
                if a.t <= window.1 {
                    if let Some((je, jxf)) = jason {
                        let to = jxf.translation - xf.translation;
                        let flat = Vec2::new(to.x, to.z);
                        let angle = flat.normalize_or_zero().dot(Vec2::new(facing.x, facing.z)).clamp(-1.0, 1.0).acos().to_degrees();
                        if flat.length() <= SWING_REACH_M && angle <= SWING_ANGLE_DEG {
                            a.done = true;
                            let stun = rng.0.frand() * 100.0 < stats.stun_chance;
                            // Which of his sides she hit: his right is -X of his facing (+Z).
                            let jfwd = jxf.rotation * Vec3::Z;
                            let left = Vec2::new(-to.x, -to.z).perp_dot(Vec2::new(jfwd.x, jfwd.z)) < 0.0;
                            info!("hit Jason ({:.0} damage, {:.0}% stun chance: {})", stats.damage, stats.stun_chance, if stun { "stunned" } else { "no stun" });
                            commands.entity(je).insert(HitJason { stun: stun.then_some(Stun::Weapon { left }), from: xf.translation });
                            let name = items.get(a.item).map_or("weapon".to_string(), |i| i.name.clone());
                            wear_weapon(&mut commands, &mut wear, &mut inv, a.item, stats.durability, stats.attacking_wear, &name);
                        }
                    }
                } else {
                    a.done = true; // missed
                }
            }
            Use::Fire { kind: ItemKind::Gun { projectile, pellets, spread_deg, velocity, .. } } => {
                a.done = true;
                // At what the middle of the screen shows: Jason if the camera's ray passes through him, else far out.
                let (eye, look) = (camera.1.translation(), camera.1.forward().as_vec3());
                let target = jason
                    .and_then(|(_, jxf)| ray_hits_capsule(eye, look, jxf.translation))
                    .unwrap_or(eye + look * 30.0);
                let aim = (target - chest).normalize_or(look);
                debug!("aim: eye {eye} look {look} target {target} Jason {:?}", jason.map(|j| j.1.translation));
                if projectile {
                    let (mesh, mat) = (assets.0.add(Sphere::new(0.05)), assets.1.add(StandardMaterial { emissive: LinearRgba::rgb(40.0, 4.0, 2.0), ..default() }));
                    commands.spawn((
                        Mesh3d(mesh),
                        MeshMaterial3d(mat),
                        Transform::from_translation(chest + aim * 0.5),
                        PointLight { color: Color::srgb(1.0, 0.2, 0.1), intensity: 400_000.0, range: 25.0, ..default() },
                        Flare { vel: aim * velocity / 100.0, age: 0.0, hit: false },
                    ));
                    info!("flare fired");
                } else if let Some((je, jxf)) = jason {
                    // Pellets: Jason is hit if he's in range and inside the spread.
                    let center = jxf.translation + Vec3::Y * JASON_HEIGHT_M * 0.5;
                    let d = center - chest;
                    let along = d.dot(aim);
                    let off = (d - aim * along).length();
                    if along > 0.0 && along <= SHOTGUN_RANGE_M && off <= JASON_RADIUS_M + along * spread_deg.to_radians().tan() {
                        info!("shotgun hit Jason ({pellets} pellets)");
                        commands.entity(je).insert(HitJason { stun: Some(Stun::Shotgun), from: xf.translation });
                    } else {
                        info!("shotgun missed");
                    }
                }
                // One shot.
                inv.hand = None;
                commands.entity(a.item).despawn();
            }
            Use::Fire { .. } => a.done = true,
            Use::Throw => {
                a.done = true;
                if let Ok(item) = items.get(a.item) {
                    commands.trigger(ClassScore::new(&item.class, "UseScoreEvent"));
                }
                inv.pockets.retain(|&e| e != a.item);
                commands.entity(a.item).despawn();
                let (mesh, mat) = (assets.0.add(Cuboid::new(0.06, 0.06, 0.14)), assets.1.add(StandardMaterial { base_color: Color::srgb(0.7, 0.05, 0.05), ..default() }));
                let vel = facing * THROW_SPEED.0 + Vec3::Y * THROW_SPEED.1;
                commands.spawn((Mesh3d(mesh), MeshMaterial3d(mat), Transform::from_translation(chest + facing * 0.4), Firecracker { vel, age: 0.0, stunned: false }));
            }
            Use::PlaceTrap => {
                a.done = true;
                let world = collision.with(&dynamic);
                let at = xf.translation + facing * 0.7;
                let y = surface_below(&world, at.x, at.z, at.y + 0.5, at.y - 2.0).map_or(at.y, |s| s.0);
                inv.hand = None;
                commands
                    .entity(a.item)
                    .remove::<(Carried, ChildOf, InHand, Item)>()
                    .insert((ArmedTrap, Transform::from_translation(Vec3::new(at.x, y, at.z)).with_rotation(xf.rotation), Visibility::Inherited));
                info!("bear trap armed at ({:.1}, {:.1}, {:.1})", at.x, y, at.z);
                if let Ok(item) = items.get(a.item) {
                    commands.trigger(ClassScore::new(&item.class, "PlaceTrapScoreEvent"));
                }
            }
        }
    }
    if a.t >= a.len {
        commands.entity(pe).remove::<(ItemAction, PlayAction)>();
    }
}

/// Projectiles: firecrackers bounce, wait, then crackle and stun Jason if he comes within their blast; flares fly
/// and stun Jason if they hit him.
#[allow(clippy::type_complexity)]
fn fly(
    mut commands: Commands,
    time: Res<Time>,
    (collision, dynamic): (Res<crate::collision::WorldCollision>, Res<crate::collision::DynamicColliders>),
    mut crackers: Query<(Entity, &mut Firecracker, &mut Transform), Without<Flare>>,
    mut flares: Query<(Entity, &mut Flare, &mut Transform), Without<Firecracker>>,
    jasons: Query<(Entity, &Transform), (With<Jason>, Without<Firecracker>, Without<Flare>)>,
    mut lights: Query<&mut PointLight>,
) {
    let dt = time.delta_secs();
    let world = collision.with(&dynamic);
    let ground = |p: Vec3| surface_below(&world, p.x, p.z, p.y + 0.3, p.y - 50.0);
    for (e, mut f, mut xf) in &mut crackers {
        f.age += dt;
        f.vel.y -= 9.8 * dt;
        let next = xf.translation + f.vel * dt;
        match ground(xf.translation) {
            Some((y, n)) if next.y <= y + 0.03 => {
                // Bounciness 0.3, Friction 0.8 (its ProjectileMovement).
                xf.translation = Vec3::new(next.x, y + 0.03, next.z);
                let vn = f.vel.dot(n);
                f.vel = (f.vel - n * vn) * 0.2 - n * vn * 0.3;
                if f.vel.length() < 0.2 {
                    f.vel = Vec3::ZERO;
                }
            }
            _ => xf.translation = next,
        }
        let active = f.age >= FIRECRACKER_ACTIVATION;
        if active && f.age - dt < FIRECRACKER_ACTIVATION {
            info!("firecrackers going off");
            commands.entity(e).insert(PointLight { color: Color::srgb(1.0, 0.8, 0.4), intensity: 0.0, range: 10.0, ..default() });
        }
        if active {
            if let Ok(mut l) = lights.get_mut(e) {
                l.intensity = if (f.age * 23.0).sin() > 0.3 { 300_000.0 } else { 20_000.0 };
            }
            if !f.stunned {
                if let Some((je, _)) = jasons.iter().find(|(_, j)| j.translation.distance(xf.translation) <= FIRECRACKER_BLAST_M) {
                    f.stunned = true;
                    info!("firecrackers: Jason in the blast");
                    commands.entity(je).insert(HitJason { stun: Some(Stun::Firecracker), from: xf.translation });
                }
            }
        }
        if f.age >= FIRECRACKER_ACTIVATION + FIRECRACKER_NOISE {
            commands.entity(e).despawn();
        }
    }
    for (e, mut f, mut xf) in &mut flares {
        f.age += dt;
        let from = xf.translation;
        if f.vel != Vec3::ZERO {
            f.vel.y -= 9.8 * dt;
            let next = from + f.vel * dt;
            if !f.hit {
                for (je, jxf) in &jasons {
                    if segment_hits_capsule(from, next, jxf.translation) {
                        f.hit = true;
                        info!("flare hit Jason");
                        // BP_Flare_Projectile's FlareScoreEventClass.
                        commands.trigger(Score("SpotJasonFlare"));
                        commands.entity(je).insert(HitJason { stun: Some(Stun::Flare), from });
                        f.vel *= 0.1;
                    }
                }
            }
            match ground(from) {
                Some((y, _)) if next.y <= y + 0.05 => {
                    xf.translation = Vec3::new(next.x, y + 0.05, next.z);
                    f.vel = Vec3::ZERO;
                }
                _ => xf.translation = next,
            }
        }
        if f.age >= FLARE_LIFESPAN {
            commands.entity(e).despawn();
        }
    }
}

/// Where a ray first comes within Jason's capsule (standing at `feet`), if it does (sampled along the ray).
fn ray_hits_capsule(from: Vec3, dir: Vec3, feet: Vec3) -> Option<Vec3> {
    let (p, q) = (feet + Vec3::Y * JASON_RADIUS_M, feet + Vec3::Y * (JASON_HEIGHT_M - JASON_RADIUS_M));
    (0..600).map(|i| from + dir * (i as f32 * 0.1)).find(|x| {
        let t = ((*x - p).dot(q - p) / (q - p).length_squared()).clamp(0.0, 1.0);
        x.distance(p.lerp(q, t)) <= JASON_RADIUS_M
    })
}

/// The segment passes through Jason's capsule (standing at `feet`).
fn segment_hits_capsule(a: Vec3, b: Vec3, feet: Vec3) -> bool {
    let (p, q) = (feet + Vec3::Y * JASON_RADIUS_M, feet + Vec3::Y * (JASON_HEIGHT_M - JASON_RADIUS_M));
    // Closest points between the two segments, sampled (short steps).
    let steps = ((b - a).length() / 0.1).ceil().max(1.0) as usize;
    (0..=steps).any(|i| {
        let x = a.lerp(b, i as f32 / steps as f32);
        let t = ((x - p).dot(q - p) / (q - p).length_squared()).clamp(0.0, 1.0);
        x.distance(p.lerp(q, t)) <= JASON_RADIUS_M
    })
}

/// Jason stepping on an armed trap is caught (BearTrapLeftFootStun_C); the trap stays shut.
fn spring_traps(mut commands: Commands, traps: Query<(Entity, &Transform), With<ArmedTrap>>, jasons: Query<(Entity, &Transform, &Jason)>, player: Query<&Transform, With<Player>>) {
    for (te, txf) in &traps {
        for (je, jxf, j) in &jasons {
            let d = Vec2::new(jxf.translation.x - txf.translation.x, jxf.translation.z - txf.translation.z).length();
            if d < 2.0 {
                trace!("trap: Jason {d:.2} m away ({:?}, dy {:.2}); she is {:.2} m from it", j.state, jxf.translation.y - txf.translation.y, player.iter().next().map_or(0.0, |p| p.translation.distance(txf.translation)));
            }
            if d <= TRAP_RADIUS_M && (jxf.translation.y - txf.translation.y).abs() < 0.6 && matches!(j.state, crate::jason::JasonState::Hunting) {
                info!("Jason stepped in a bear trap");
                commands.entity(je).insert(HitJason { stun: Some(Stun::BearTrap), from: txf.translation });
                commands.entity(te).remove::<ArmedTrap>();
            }
        }
    }
}

/// The map: a picture of the camp from straight above, rendered once (the camp doesn't change), shown in the corner
/// with a dot where she is.
#[derive(Resource)]
struct MapPicture {
    image: Handle<Image>,
    /// World XZ at the picture's centre.
    center: Vec2,
    /// Frames the snapshot camera has left (it needs a few before its target image is ready on the GPU).
    frames: u32,
    camera: Option<Entity>,
}

#[derive(Component)]
struct MapPanel;

#[derive(Component)]
struct MapDot;

/// Metres across the map picture, and its size on screen and in pixels.
const MAP_SPAN_M: f32 = 300.0;
const MAP_PANEL_PX: f32 = 360.0;
const MAP_IMAGE_PX: u32 = 1024;

/// M: the map (when carried).
#[allow(clippy::too_many_arguments)]
fn toggle_map(
    mut commands: Commands,
    keys: Res<ButtonInput<KeyCode>>,
    player: Query<(&Inventory, &Transform), With<Player>>,
    items: Query<&Item>,
    panels: Query<Entity, With<MapPanel>>,
    mut picture: Option<ResMut<MapPicture>>,
    mut images: ResMut<Assets<Image>>,
    mut cameras: Query<&mut Camera>,
) {
    // Stop the snapshot camera once it has rendered.
    if let Some(p) = picture.as_mut() {
        if let Some(cam) = p.camera {
            if p.frames == 0 {
                commands.entity(cam).despawn();
                p.camera = None;
            } else {
                p.frames -= 1;
                if let Ok(mut c) = cameras.get_mut(cam) {
                    c.is_active = true;
                }
            }
        }
    }
    let Ok((inv, xf)) = player.single() else { return };
    let has_map = inv.pockets.iter().chain(inv.hand.iter()).any(|&e| items.get(e).is_ok_and(|i| i.kind == ItemKind::Map));
    let open = !panels.is_empty();
    if open && (!has_map || keys.just_pressed(KeyCode::KeyM)) {
        for e in &panels {
            commands.entity(e).despawn();
        }
        return;
    }
    if !keys.just_pressed(KeyCode::KeyM) {
        return;
    }
    if !has_map {
        info!("no map (find one in a cabin)");
        return;
    }
    let image = match picture.as_ref() {
        Some(p) => p.image.clone(),
        None => {
            let image = images.add(Image::new_target_texture(MAP_IMAGE_PX, MAP_IMAGE_PX, bevy::render::render_resource::TextureFormat::Bgra8UnormSrgb, None));
            let center = Vec2::new(xf.translation.x, xf.translation.z);
            let camera = commands
                .spawn((
                    Camera3d::default(),
                    Camera { order: -1, ..default() },
                    bevy::camera::RenderTarget::Image(image.clone().into()),
                    Projection::Orthographic(OrthographicProjection {
                        scaling_mode: bevy::camera::ScalingMode::Fixed { width: MAP_SPAN_M, height: MAP_SPAN_M },
                        far: 2000.0,
                        ..OrthographicProjection::default_3d()
                    }),
                    Transform::from_xyz(center.x, xf.translation.y + 500.0, center.y).looking_at(Vec3::new(center.x, xf.translation.y, center.y), Vec3::NEG_Z),
                ))
                .id();
            commands.insert_resource(MapPicture { image: image.clone(), center, frames: 30, camera: Some(camera) });
            image
        }
    };
    commands
        .spawn((
            Node { position_type: PositionType::Absolute, top: Val::Px(12.0), right: Val::Px(12.0), width: Val::Px(MAP_PANEL_PX), height: Val::Px(MAP_PANEL_PX), ..default() },
            ImageNode::new(image),
            MapPanel,
        ))
        .with_child((
            Node { position_type: PositionType::Absolute, width: Val::Px(10.0), height: Val::Px(10.0), border_radius: BorderRadius::all(Val::Px(5.0)), ..default() },
            BackgroundColor(Color::srgb(1.0, 0.1, 0.1)),
            MapDot,
        ));
}

/// The dot follows her.
fn follow_map(picture: Option<Res<MapPicture>>, player: Query<&Transform, With<Player>>, mut dots: Query<&mut Node, With<MapDot>>) {
    let (Some(picture), Ok(p)) = (picture, player.single()) else { return };
    for mut n in &mut dots {
        // The picture's up is -Z, its right +X.
        let u = (p.translation.x - picture.center.x) / MAP_SPAN_M + 0.5;
        let v = (p.translation.z - picture.center.y) / MAP_SPAN_M + 0.5;
        n.left = Val::Px(u.clamp(0.0, 1.0) * MAP_PANEL_PX - 5.0);
        n.top = Val::Px(v.clamp(0.0, 1.0) * MAP_PANEL_PX - 5.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_flare_through_jason_hits_and_one_past_him_misses() {
        let feet = Vec3::ZERO;
        assert!(segment_hits_capsule(Vec3::new(-1.0, 1.0, 0.0), Vec3::new(1.0, 1.0, 0.0), feet));
        assert!(!segment_hits_capsule(Vec3::new(-1.0, 1.0, 1.0), Vec3::new(1.0, 1.0, 1.0), feet));
        assert!(!segment_hits_capsule(Vec3::new(-1.0, 2.5, 0.0), Vec3::new(1.0, 2.5, 0.0), feet));
    }
}
