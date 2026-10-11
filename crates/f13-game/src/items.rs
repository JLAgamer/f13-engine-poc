//! Using items against Jason: swinging a weapon, firing the flare gun or the shotgun, throwing a firecracker,
//! placing a bear trap; the pocket knife (breaks a grab) and the grab struggle are in jason.rs. The held item hangs
//! off its hand socket. The map shows the camp from above, with a dot where she is.
//!
//! Controls: left mouse uses the item in the hands (swing; held 0.1 s in combat stance, the heavy ForwardAttack), 1-3 the
//! pocket items (first aid in interact.rs, firecrackers here), M the map (when carried). In combat stance, right
//! mouse blocks with a weapon in the hands.
//!
//! Blocking (SCCharacter::OnStartBlock / CanBlock / SCCounselorCharacter::TakeDamage): it starts only with a weapon
//! in hand, alive, not attacking, stunned or interacting, and lasts while the button is held. A hit taken while
//! blocking is scaled by her BlockingDamageMod (f13-core::health) and wears the weapon by its BlockingWear (a weapon
//! worn out breaks); she plays the block reaction (AM_F_Combat_Stand_Block_React) instead of a hit reaction. A
//! killer's heavy attack (his weapon's ForwardAttack) ends the block. BlockStaminaCost isn't read by the game's
//! code. The block idle plays full-body standing and as an upper-body layer on the move (character.rs).
//!
//! From the game's data: WeaponStats (Damage, StunChance, Durability, AttackingWear), the swings' AttackTraceNotify
//! windows, the stun classes' times (jason.rs), the firecracker's ThrowDelayTime, BP_Firecracker_Projectile's
//! ActivationTime 1.5 s, NoiseTimer 20 s and BlastRadius 200 cm, the flare gun's ProjectileVelocity and the
//! shotgun's NumPellets and MaxSpreadAngle.
//!
//! A swing (SCCharacter::AttackTraceBegin / AttackTrace, the weapon's sweep FUN_1404501f0): in the swing's
//! AttackTraceNotify window, each frame sweeps the weapon's KillVolume capsule (its "DamageVolume") from where it was
//! to where it is, in min(max(its yaw change / 15 degrees (the sc.AttackRotationIteration CVar), its move / its
//! diameter, 1), 9) steps: step i sweeps from i/n of the way there to its current place, and a line runs from her
//! to that point. The first actor hit is hit once per swing. The sweeps hit Jason's physics bodies (his mesh's
//! PhysicsAsset, posed by his animation: the largest shape of each; the Pawn channel, his capsule, is ignored); walls
//! don't stop the sweeps (only the line from her).
//!
//! Guns (SCGun): using one fires it at once (its Use: bHasAmmunition and bIsAiming -> Fire; the flare's OnUseItem
//! notify is at 0 s too), and one shot spends it (Fire 0x14044ea80: bHasAmmunition false unless infinite ammo, and
//! bIsAiming false). The spent gun stays in her hands: SwapToOnUse (the shotgun's MeleeShotgun_C) is applied only by
//! SCCounselorCharacter::OnSwapItemOnUse (vt +0x1190, 0x1403cac60), which only the AnimBP's AnimNotify_ItemSwap
//! calls, and no package carries that notify (f13-export grep). The shotgun keeps its own attacks (NeutralAttack,
//! ForwardAttack: the two-handed axe swings) in combat stance; the flare gun has none. Each shotgun pellet
//! traces FireDistance / cos(its spread angle): it reaches FireDistance (SCGun's constructor, 2000 cm) ahead.
//! Only while aiming (SCGun::Use needs bIsAiming).
//!
//! Aiming (CSLR_Aim, right mouse: the AimInputComponent's press 0x1403c0280 / release 0x1403c0480): the press, out of
//! the water, not seated, picking up, emoting, interacting or in a montage, with a gun in her hands, calls
//! SCGun::AimGun(true) (bIsAiming only with bHasAmmunition) and then the controller's SetIgnoreMoveInput(true)
//! (vtable +0x6d0, the slot Controller::SetIgnoreMoveInput's thunk calls): she stands. The release: AimGun(false),
//! SetIgnoreMoveInput(false). The tick (0x1403d8de0): her bIsAiming is the gun's (false without a gun); aiming, she
//! takes the control yaw (bUseControllerRotationYaw, not oriented to movement) and AimPitch is the control pitch
//! (-180..180), else AimPitch eases to 0 (FInterpTo, speed 5). In combat stance right mouse is CMBT_CSLR_Block too:
//! there the port blocks (which input the game's stack takes first isn't read).
//!
//! The firecracker's throw (SCNoiseMaker::ThrowProjectile 0x14039aa60): the control pitch is clamped between two
//! unreflected fields (+0x5a8 / +0x5ac) that the constructor zeroes and nothing sets, so it's always 0: the pack
//! leaves level along the camera's yaw at VelocityPitchCurve(0) (FirecrackerVelocityPitch_Curve: 500 cm/s), under
//! DefaultGravityZ, from the item's root: its place in her hand (its hand socket, WeaponSocket, as last posed).
//!
//! Bear traps (SCTrap::OnBeginOverlap and its trigger): an armed trap catches whoever steps into it, its armer too
//! (bCanArmerTrigger, constructor), but not someone already standing in it when it was armed (it's an overlap's
//! begin). A caught counselor takes TrapDamage (50, constructor) and is stunned by the foot's stun class
//! (BearTrapLeft/RightFootStun(Sprinting)_C): she plays the caught montage for that foot, walking or running, and
//! wiggles free with A / D (f13-core::stun: TotalStunTime 10, PercentPerWiggle 5 and DepletionRate 0.02 from the
//! constructor), then steps out. The sprung trap stays shut and can't be picked up (bCanPickUp false). Trapper is
//! scored only when the armer or the one caught is the killer.

use bevy::prelude::*;
use f13_core::collision::surface_below;

use crate::character::{CounselorCharacter, PlayAction, Rig};
use crate::interact::{Carried, Inventory};
use crate::jason::{HitJason, Jason, Stun};
use crate::player::{Player, PlayerCamera};
use crate::props::{Item, ItemKind, WeaponStats};
use crate::match_flow::{ClassScore, Score};

/// sc.AttackRotationIteration: degrees of the weapon's turn per extra sweep step.
const ATTACK_ROTATION_ITERATION_DEG: f32 = 15.0;
/// The most sweep steps a frame.
const ATTACK_TRACE_STEPS: f32 = 9.0;
/// The counselor's gun aim: bIsAiming and AimPitch (degrees, + up).
#[derive(Component, Default, Debug)]
pub struct GunAim {
    pub aiming: bool,
    pub pitch: f32,
}

/// AimPitch's FInterpTo speed back to 0 when not aiming (the tick's constant).
const AIM_PITCH_RETURN_SPEED: f32 = 5.0;

/// SCGun's FireDistance (constructor; the shotgun doesn't override it).
const SHOTGUN_RANGE_M: f32 = 20.0;
/// Firecracker throw speed (m/s): FirecrackerVelocityPitch_Curve at its pitch, always 0.
const THROW_SPEED: f32 = 5.0;
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
/// The weapons' NeutralAttack / ForwardAttack montages (her female ones; every melee weapon class names one of these
/// two pairs) whose segments are the takes above: their root motion carries her through the swing (anims.json).
const SWING_MONTAGES: [(&str, &str); 4] = [
    ("f_combat_stand_2h_melee_light_01", "AM_F_combat_stand_2h_axe_attack_light"),
    ("f_combat_stand_2h_melee_heavy_01", "AM_F_combat_stand_2h_axe_attack_heavy"),
    ("f_combat_stand_1h_melee_light_01", "AM_F_combat_stand_1h_attack_light"),
    ("f_combat_stand_1h_melee_heavy_01", "AM_F_combat_stand_1h_attack_heavy"),
];

/// A swing take's root motion (local cm/s: x right, y forward), from its montage's segment.
fn swing_root(anims: &crate::Anims, take: &str) -> Option<Vec2> {
    let (_, montage) = SWING_MONTAGES.iter().find(|(t, _)| *t == take)?;
    anims.0.get(&format!("{montage}/{take}")).copied()
}
const SWING_2H: [&str; 2] = ["f_combat_stand_2h_melee_light_01", "f_combat_stand_2h_melee_heavy_01"];
const FLARE_FIRE: &str = "f_normal_stand_flare_shoot";
const SHOTGUN_FIRE: &str = "combat_stand_rifle_fire_01";
const THROW: &str = "m_normal_stand_firecracker_throw";
const PLACE_TRAP: &str = "f_normal_stand_trap_place";

pub struct ItemsPlugin;

impl Plugin for ItemsPlugin {
    fn build(&self, app: &mut App) {
        // The wiggle axis, for her grab struggle (jason.rs) and trap struggle.
        app.init_resource::<crate::jason::Struggle>()
            .add_systems(PreUpdate, (crate::jason::read_struggle, hold_aim, circle_run).after(bevy::input::InputSystems))
            .add_systems(PostUpdate, give_items)
            .init_resource::<MapScreen>()
            .add_systems(Update, (hold_in_hand, shotgun_hold, aim, start_action, block, toggle_map).chain())
            .add_observer(wear_on_block)
            .add_observer(set_killer_trap)
            .add_systems(FixedUpdate, (tick_action, fly, spring_traps, struggle_in_trap).chain().after(crate::player::move_player));
    }
}

/// Testing (--aim): right mouse held, the camera at this pitch (degrees), and the left clicked once at this time (s).
#[derive(Resource)]
pub struct AimHold(pub f32, pub Option<f32>);

/// Testing (--circle): W held, the camera turning at this rate (degrees per second): she runs a circle.
#[derive(Resource)]
pub struct CircleRun(pub f32);

fn circle_run(run: Option<Res<CircleRun>>, time: Res<Time>, mut keys: ResMut<ButtonInput<KeyCode>>, mut camera: Single<&mut PlayerCamera>) {
    let Some(run) = run else { return };
    keys.press(KeyCode::KeyW);
    camera.yaw += run.0.to_radians() * time.delta_secs();
}

fn hold_aim(hold: Option<Res<AimHold>>, time: Res<Time>, mut mouse: ResMut<ButtonInput<MouseButton>>, mut camera: Single<&mut PlayerCamera>, mut fired: Local<bool>) {
    let Some(hold) = hold else { return };
    mouse.release(MouseButton::Right);
    mouse.press(MouseButton::Right);
    camera.pitch = hold.0.to_radians();
    // And a click at this time, once (--aim pitch:seconds).
    if !*fired && hold.1.is_some_and(|t| time.elapsed_secs() >= t) {
        *fired = true;
        mouse.press(MouseButton::Left);
    } else if *fired {
        mouse.release(MouseButton::Left);
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
        let Some((e, item)) = items.iter().find(|(e, i)| &i.class == class && inv.hand != Some(*e) && !inv.pockets.contains(e) && !inv.special.contains(e)) else {
            warn!("--give: no {class} in this match");
            continue;
        };
        // As a pickup sorts it (0x1403a4e30): a special item (the map) to SpecialItemInventory, apart from the pockets.
        if item.special {
            inv.special.push(e);
        } else if item.large {
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

/// A gun that has fired its shot (bHasAmmunition false): it can't aim again.
#[derive(Component)]
pub struct Spent;

/// The socket the item in her hands hangs from (its HandAttachSocket, or where ShotgunHold moved it).
#[derive(Component)]
pub struct HeldAt(pub String);

#[derive(Clone, Debug)]
enum Use {
    /// A swing: light (NeutralAttack) or heavy (ForwardAttack).
    Swing { stats: WeaponStats, window: (f32, f32), heavy: bool },
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
    /// The weapon's KillVolume as last traced (PreviousTraceLocation / Rotation).
    trace: Option<Transform>,
    /// The attack button's hold in combat stance (SCCharacter +0xf4c / +0xf50, from the press that swung).
    hold: Option<f32>,
    /// Its montage's root motion moving her while it plays (a swing's: local cm/s, x right, y forward).
    pub(crate) root: Option<Vec2>,
}

/// SCCharacter's tick: the attack held this long in combat stance is the heavy attack (DAT_14207a200).
const HEAVY_HOLD_S: f32 = 0.1;

/// The closest points' distance between segments `p0..p1` and `q0..q1` (Ericson 5.1.9).
fn segment_distance(p0: Vec3, p1: Vec3, q0: Vec3, q1: Vec3) -> f32 {
    let (d1, d2, r) = (p1 - p0, q1 - q0, p0 - q0);
    let (a, e, f) = (d1.dot(d1), d2.dot(d2), d2.dot(r));
    let (s, t) = if a <= 1e-9 && e <= 1e-9 {
        (0.0, 0.0)
    } else if a <= 1e-9 {
        (0.0, (f / e).clamp(0.0, 1.0))
    } else {
        let c = d1.dot(r);
        if e <= 1e-9 {
            ((-c / a).clamp(0.0, 1.0), 0.0)
        } else {
            let b = d1.dot(d2);
            let denom = a * e - b * b;
            let mut s = if denom > 1e-9 { ((b * f - c * e) / denom).clamp(0.0, 1.0) } else { 0.0 };
            let mut t = (b * s + f) / e;
            if t < 0.0 {
                t = 0.0;
                s = (-c / a).clamp(0.0, 1.0);
            } else if t > 1.0 {
                t = 1.0;
                s = ((b - c) / a).clamp(0.0, 1.0);
            }
            (s, t)
        }
    };
    (p0 + d1 * s).distance(q0 + d2 * t)
}

/// One frame of a swing's trace (FUN_1404501f0): whether the weapon's KillVolume (`radius`, `half` its half height),
/// moving from `prev` to `now`, or the line from its owner at `owner` to each step's point, touches the capsule
/// `body` (its sphere centres) of `body_radius`. A wall in the way stops the line.
/// A physics body in the world: a capsule (segment, radius) or a box (its transform, half extents), metres.
#[derive(Clone, Copy, Debug)]
enum BodyShape {
    Capsule(Vec3, Vec3, f32),
    Box(Transform, Vec3),
}

impl BodyShape {
    /// Whether the segment `a..b` comes within `r` of the shape.
    fn near_segment(&self, a: Vec3, b: Vec3, r: f32) -> bool {
        match *self {
            BodyShape::Capsule(p, q, cr) => segment_distance(a, b, p, q) <= r + cr,
            BodyShape::Box(xf, half) => {
                // The segment's closest approach to the box, by its points (a fine sampling of it).
                let inv = xf.compute_affine().inverse();
                let n = ((a.distance(b) / (r.max(0.01))).ceil() as usize).clamp(1, 64);
                (0..=n).any(|k| {
                    let p = inv.transform_point3(a.lerp(b, k as f32 / n as f32));
                    let d = (p.abs() - half).max(Vec3::ZERO);
                    d.length() <= r
                })
            }
        }
    }
}

/// Jason's physics bodies where his animation has them (his rig's bones), from his PhysicsAsset's shapes: a Sphyl's
/// axis is its Z (Bevy's Y here).
fn jason_bodies(info: &f13_data::character::CharacterInfo, rig: &Rig, globals: &Query<&GlobalTransform>) -> Vec<BodyShape> {
    info.ragdoll
        .iter()
        .filter_map(|b| {
            let bone = globals.get(*rig.bones.get(&b.bone)?).ok()?;
            let local = b.transform.as_ref().map(crate::to_transform).unwrap_or_default();
            let xf = bone.mul_transform(local).compute_transform();
            Some(match (b.shape.as_str(), b.extents) {
                ("Box", Some([x, y, z])) => BodyShape::Box(Transform { scale: Vec3::ONE, ..xf }, Vec3::new(x, z, y) / 200.0),
                _ => {
                    let half = xf.rotation * Vec3::Y * (b.length / 200.0);
                    BodyShape::Capsule(xf.translation - half, xf.translation + half, b.radius / 100.0)
                }
            })
        })
        .collect()
}

fn swing_hits(prev: Transform, now: Transform, radius: f32, half: f32, owner: Vec3, bodies: &[BodyShape], world: &(impl f13_core::collision::TriSource + ?Sized)) -> bool {
    use f13_core::camera_spline::rotator;
    // Unreal's yaw (rotator Y) wraps into -180..180; only a positive change from the last trace counts (as in the game).
    let yaw = |t: &Transform| rotator(t.rotation).y;
    let mut dyaw = yaw(&prev) - yaw(&now);
    if dyaw > 180.0 {
        dyaw -= 360.0;
    } else if dyaw < -180.0 {
        dyaw += 360.0;
    }
    let moves = (prev.translation.distance(now.translation) / (radius * 2.0).max(1e-3)).max(1.0).trunc();
    let steps = (dyaw / ATTACK_ROTATION_ITERATION_DEG).max(moves).min(ATTACK_TRACE_STEPS) as i32;
    let axis = |at: Vec3, rot: Quat| {
        let a = rot * Vec3::Y * (half - radius).max(0.0);
        (at - a, at + a)
    };
    let touches = |a: (Vec3, Vec3)| bodies.iter().any(|b| b.near_segment(a.0, a.1, radius));
    for i in 1..=steps.max(1) {
        let s = i as f32 / steps.max(1) as f32;
        let (from, rot) = (prev.translation.lerp(now.translation, s), prev.rotation.slerp(now.rotation, s));
        // The sweep from this step's point to the volume's current place (with this step's rotation).
        let n = ((from.distance(now.translation) / radius.max(1e-3)).ceil() as usize).max(1);
        if (0..=n).any(|k| touches(axis(from.lerp(now.translation, k as f32 / n as f32), rot))) {
            return true;
        }
        // The line from her to this step's point, unless a wall comes first.
        let line = bodies.iter().any(|b| b.near_segment(owner, from, 0.0));
        if line && f13_core::collision::segment_hit(world, owner, from, 1.0).is_none() {
            return true;
        }
    }
    false
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

/// Right mouse out of combat stance with a gun in her hands: aim while held (see the module notes).
#[allow(clippy::type_complexity)]
fn aim(
    mut commands: Commands,
    time: Res<Time>,
    mouse: Res<ButtonInput<MouseButton>>,
    interaction: Res<crate::interact::Interaction>,
    camera: Single<&PlayerCamera>,
    mut player: Query<(Entity, &Player, &Inventory, &mut Transform, Option<&mut GunAim>, Has<ItemAction>, (Has<crate::vehicle::Driving>, Has<crate::hiding::Hiding>, Has<crate::jason::Held>, Has<crate::interact::UsingItem>))>,
    items: Query<(&Item, Has<Spent>)>,
) {
    let Ok((pe, p, inv, mut xf, aim, acting, (driving, hiding, held, using))) = player.single_mut() else { return };
    let Some(mut aim) = aim else {
        commands.entity(pe).try_insert(GunAim::default());
        return;
    };
    let c = &p.counselor;
    // A loaded gun (AimGun sets bIsAiming only with bHasAmmunition; Fire clears it).
    let gun = inv.hand.and_then(|h| items.get(h).ok()).is_some_and(|(i, spent)| matches!(i.kind, ItemKind::Gun { .. }) && !spent);
    use f13_core::counselor::Stance;
    if !gun || !c.alive() || held || driving || hiding {
        aim.aiming = false;
    } else if !mouse.pressed(MouseButton::Right) {
        aim.aiming = false;
    } else if mouse.just_pressed(MouseButton::Right)
        && !matches!(c.stance, Stance::Swimming | Stance::Combat)
        && !interaction.busy()
        && !using
        && !acting
        && p.root_motion.is_none()
    {
        // The gun has its one shot until it's fired (the shot spends it; the port drops the gun then).
        aim.aiming = true;
    }
    if aim.aiming {
        let (sin, cos) = camera.yaw.sin_cos();
        xf.rotation = Quat::from_rotation_y((-sin).atan2(-cos));
        aim.pitch = (camera.pitch.to_degrees() + 180.0).rem_euclid(360.0) - 180.0;
    } else if aim.pitch.abs() > 1e-8 {
        aim.pitch = finterp_to(aim.pitch, 0.0, time.delta_secs(), AIM_PITCH_RETURN_SPEED);
    }
}

/// FMath::FInterpTo.
pub(crate) fn finterp_to(current: f32, target: f32, dt: f32, speed: f32) -> f32 {
    if speed <= 0.0 {
        return target;
    }
    let dist = target - current;
    if dist * dist < 1e-8 {
        return target;
    }
    current + dist * (dt * speed).clamp(0.0, 1.0)
}

/// Right mouse in combat stance: block, while held, with a weapon in her hands.
#[allow(clippy::type_complexity)]
fn block(
    mut commands: Commands,
    mouse: Res<ButtonInput<MouseButton>>,
    interaction: Res<crate::interact::Interaction>,
    player: Query<(Entity, &Player, &Inventory, Has<Blocking>, Has<ItemAction>, Has<crate::jason::Staggered>, Has<crate::jason::Held>), (Without<crate::vehicle::Driving>, Without<Trapped>)>,
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

/// An armed bear trap; `clear` once the counselor isn't standing in it (she was when it was armed); `killer`: Jason's
/// (KillerTrap_C: bCanArmerTrigger false, so it doesn't catch him).
#[derive(Component)]
pub struct ArmedTrap {
    clear: bool,
    pub killer: bool,
}

/// Jason's trap class (his TrapClass).
const KILLER_TRAP_CLASS: &str = "KillerTrap_C";

/// Jason sets a trap here (jason.rs).
#[derive(Event, Clone, Copy, Debug)]
pub struct SetKillerTrap(pub Transform);

/// The counselors' bear trap mesh, standing in for Jason's (A_Trap_Jason isn't exported).
const TRAP_MESH: &str = "SummerCamp/Content/Props/SkeletalMeshes/Traps/A_Trap_Bear_01.glb";

fn set_killer_trap(ev: On<SetKillerTrap>, mut commands: Commands, assets: Res<AssetServer>) {
    commands.spawn((
        ArmedTrap { clear: true, killer: true },
        crate::prop_anim::PropAnim::new(crate::prop_anim::TRAP_ARMED),
        ev.0,
        Visibility::Inherited,
        bevy::world_serialization::WorldAssetRoot(assets.load(GltfAssetLabel::Scene(0).from_asset(TRAP_MESH))),
    ));
}

/// The counselor is caught in a bear trap.
#[derive(Component)]
pub struct Trapped {
    /// The trap she's in (it opens as she steps out).
    trap: Entity,
    left: bool,
    enter: &'static str,
    t: f32,
    wiggle: f13_core::stun::Wiggle,
    /// Seconds into stepping out, once free.
    out: Option<f32>,
}

impl Trapped {
    /// Her stun minigame's completion while she's still in it.
    pub fn struggle(&self) -> Option<f32> {
        self.out.is_none().then_some(self.wiggle.completion)
    }
}

/// SCTrap: TrapDamage (constructor); the bear trap stun's minigame (TotalStunTime 10; PercentPerWiggle and
/// DepletionRate from SCStunDamageType's constructor).
const TRAP_DAMAGE: f32 = 50.0;
const TRAP_STRUGGLE: f13_core::stun::WiggleTuning = f13_core::stun::WiggleTuning { total_time: 10.0, percent_per_wiggle: 5.0, depletion_rate: 0.02 };
/// Her animations caught by the left / right foot: stepping in walking, running; the struggle; stepping out.
const TRAP_WALK: [&str; 2] = ["f_normal_stand_trap_walk_enter_lf", "f_normal_stand_trap_walk_enter_rf"];
const TRAP_RUN: [&str; 2] = ["f_normal_stand_trap_run_enter_lf", "f_normal_stand_trap_run_enter_rf"];
const TRAP_CYCLE: [&str; 2] = ["f_normal_stand_trap_cycle_lf", "f_normal_stand_trap_cycle_rf"];
const TRAP_EXIT: [&str; 2] = ["f_normal_stand_trap_exit_lf", "f_normal_stand_trap_exit_rf"];
const DEATH: &str = "f_normal_stand_death_01";

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
        commands.entity(hand).insert((InHand, HeldAt(item.hand_socket.clone()), ChildOf(*bone), offset));
        if let Ok(mut v) = visibility.get_mut(hand) {
            *v = Visibility::Inherited;
        }
    }
}

/// ShotgunHold (the notify state on the hunter aim takes and the shotgun's fire): its begin moves the shotgun to
/// her hand (Shotgun_C's AttachToCharacter(false, 'WeaponSocket')), its end back to shotgun_Socket unless she's
/// aiming (the gun's bIsAiming).
#[allow(clippy::type_complexity)]
fn shotgun_hold(
    mut commands: Commands,
    character: Option<Res<CounselorCharacter>>,
    players: Query<(Entity, &Inventory, Option<&GunAim>), With<Player>>,
    rigs: Query<(&ChildOf, &Rig)>,
    mut items: Query<(&Item, &mut HeldAt)>,
    mut was: Local<bool>,
) {
    let Some(character) = character else { return };
    let Ok((pe, inv, aim)) = players.single() else { return };
    let Some((_, rig)) = rigs.iter().find(|(c, _)| c.parent() == pe) else { return };
    let (now, before) = (rig.shotgun_hold, std::mem::replace(&mut *was, rig.shotgun_hold));
    let Some(hand) = inv.hand else { return };
    let Ok((item, mut at)) = items.get_mut(hand) else { return };
    if item.class != "Shotgun_C" {
        return;
    }
    let socket = match (before, now) {
        (false, true) => "WeaponSocket",
        (true, false) if !aim.is_some_and(|a| a.aiming) => "shotgun_Socket",
        _ => return,
    };
    if at.0 == socket {
        return;
    }
    let Some(s) = character.info.sockets.get(socket) else { return };
    let Some(&bone) = rig.bones.get(&s.attach) else { return };
    let offset = s.transform.as_ref().map(crate::to_transform).unwrap_or_default();
    debug!("shotgun to {socket}");
    at.0 = socket.to_string();
    commands.entity(hand).insert((ChildOf(bone), offset));
}

/// Left mouse: use the item in the hands. 1-3: throw a firecracker from that pocket.
#[allow(clippy::too_many_arguments)]
fn start_action(
    mut commands: Commands,
    mouse: Res<ButtonInput<MouseButton>>,
    character: Option<Res<CounselorCharacter>>,
    interaction: Res<crate::interact::Interaction>,
    camera: Single<&PlayerCamera>,
    mut player: Query<
        (Entity, &Player, &Inventory, &mut Transform, Option<&GunAim>),
        (Without<ItemAction>, Without<crate::interact::UsingItem>, Without<crate::jason::Held>, Without<crate::jason::Staggered>, Without<crate::vehicle::Driving>, Without<Blocking>, Without<Trapped>),
    >,
    items: Query<&Item>,
    spent: Query<(), With<Spent>>,
    anims: Res<crate::Anims>,
) {
    let Some(character) = character else { return };
    let Ok((pe, p, inv, mut xf, aim)) = player.single_mut() else { return };
    let aiming = aim.is_some_and(|a| a.aiming);
    if !p.counselor.alive() || interaction.busy() || p.counselor.dodging() || p.root_motion.is_some() {
        return;
    }
    let len = |n: &str| character.length(n).unwrap_or(1.0);
    let combat = p.counselor.stance == f13_core::counselor::Stance::Combat;
    let mut start = |what: Use, item: Entity, anim: &str, at: f32| {
        // A swing in combat stance: the button's hold counts toward the heavy attack.
        let hold = (combat && matches!(what, Use::Swing { .. })).then_some(0.0);
        let root = matches!(what, Use::Swing { .. }).then(|| swing_root(&anims, anim)).flatten();
        commands.entity(pe).insert((ItemAction { what, item, t: 0.0, len: len(anim), at, done: false, trace: None, hold, root }, PlayAction(anim.into())));
    };
    // Face where the camera looks.
    let (sin, cos) = camera.yaw.sin_cos();
    let face = Quat::from_rotation_y((-sin).atan2(-cos));
    if mouse.just_pressed(MouseButton::Left) {
        let Some(hand) = inv.hand else { return };
        let Ok(item) = items.get(hand) else { return };
        let melee_gun = match &item.kind {
            ItemKind::Gun { stats, melee: true, .. } if combat && !(aiming && !spent.contains(hand)) => Some(stats.clone()),
            _ => None,
        };
        match (&item.kind, melee_gun.as_ref()) {
            (ItemKind::Weapon(stats), _) | (_, Some(stats)) => {
                // The NeutralAttack (0x1402e5aa0: MULTICAST_Attack(false)); in combat stance the button's hold starts
                // counting (the heavy attack after 0.1 s, in tick_action).
                let anim = if stats.two_handed { SWING_2H[0] } else { SWING_1H[0] };
                let window = character.info.animations.get(anim).and_then(|a| a.window("AttackTraceNotify")).unwrap_or((0.6, 0.8));
                info!("swing {} (light)", item.name);
                xf.rotation = face;
                start(Use::Swing { stats: stats.clone(), window, heavy: false }, hand, anim, window.0);
            }
            (ItemKind::Gun { projectile, .. }, None) => {
                if !aiming || spent.contains(hand) {
                    return;
                }
                info!("fire {}", item.name);
                xf.rotation = face;
                let anim = if *projectile { FLARE_FIRE } else { SHOTGUN_FIRE };
                start(Use::Fire { kind: item.kind.clone() }, hand, anim, 0.0);
            }
            (ItemKind::Trap, _) => {
                info!("placing {}", item.name);
                xf.rotation = face;
                start(Use::PlaceTrap, hand, PLACE_TRAP, len(PLACE_TRAP));
            }
            _ => {}
        }
        return;
    }
    let Some(item) = inv.use_small else { return };
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
    mut player: Query<(Entity, &mut ItemAction, &mut Inventory, &Transform, &mut Player)>,
    jasons: Query<(Entity, &Transform), (With<Jason>, Without<Player>)>,
    (mouse, anims, tuning): (Res<ButtonInput<MouseButton>>, Res<crate::Anims>, Res<crate::player::PlayerTuning>),
    mut wear: Query<Option<&mut Wear>>,
    items: Query<&Item>,
    (collision, dynamic): (Res<crate::collision::WorldCollision>, Res<crate::collision::DynamicColliders>),
    mut assets: (ResMut<Assets<Mesh>>, ResMut<Assets<StandardMaterial>>),
    (globals, killer, character, rigs, killer_model): (Query<&GlobalTransform>, Option<Res<crate::jason::KillerTuning>>, Option<Res<CounselorCharacter>>, Query<(&ChildOf, &Rig)>, Option<Res<crate::character::KillerCharacter>>),
    sounds: Option<Res<crate::audio::Sounds>>,
) {
    let Ok((pe, mut a, mut inv, xf, mut p)) = player.single_mut() else { return };
    a.t += time.delta_secs();
    // SCCharacter's tick: held past 0.1 s in combat stance, the light swing becomes the weapon's ForwardAttack
    // (MULTICAST_Attack(true): the anim instance switches to it at the same fraction of the swing).
    if let (Some(h), Use::Swing { stats, heavy: false, .. }) = (a.hold, a.what.clone()) {
        let held = mouse.pressed(MouseButton::Left) && p.counselor.stance == f13_core::counselor::Stance::Combat;
        a.hold = held.then_some(h + time.delta_secs());
        if held && h + time.delta_secs() > HEAVY_HOLD_S && !a.done {
            a.hold = None;
            let anim = if stats.two_handed { SWING_2H[1] } else { SWING_1H[1] };
            if let Some(c) = character.as_ref().filter(|c| c.info.animations.contains_key(anim)) {
                let window = c.info.animations.get(anim).and_then(|x| x.window("AttackTraceNotify")).unwrap_or((0.6, 0.8));
                let len = c.length(anim).unwrap_or(1.0);
                a.t = len * (a.t / a.len.max(1e-3)).min(1.0);
                a.len = len;
                a.at = window.0;
                a.what = Use::Swing { stats, window, heavy: true };
                a.root = swing_root(&anims, anim);
                info!("swing (heavy)");
                commands.entity(pe).insert((PlayAction(anim.into()), crate::character::SeekAction(a.t)));
            }
        }
    }
    let chest = xf.translation + Vec3::Y * crate::interact::CHEST_M;
    let facing = xf.rotation * Vec3::Z;
    let jason = jasons.iter().next();
    if !a.done && a.t >= a.at {
        match a.what.clone() {
            Use::Swing { stats, window, .. } => {
                if a.t <= window.1 {
                    // The KillVolume where the weapon is now (its hand socket as last posed).
                    let now = stats.volume.zip(globals.get(a.item).ok()).map(|((local, ..), g)| g.mul_transform(local).compute_transform());
                    let prev = a.trace.or(now);
                    a.trace = now;
                    let world = collision.with(&dynamic);
                    let hit = jason.filter(|(je, jxf)| {
                        let (Some(now), Some(prev), Some((_, radius, half)), Some(cap), Some(c)) = (now, prev, stats.volume, killer.as_ref().map(|k| k.capsule), character.as_ref()) else {
                            return false;
                        };
                        // Her actor's location: her capsule's centre.
                        let owner = xf.translation + Vec3::Y * c.info.capsule_half_height / 100.0;
                        // His physics bodies (his capsule only for an export without them).
                        let mut bodies = match (killer_model.as_ref(), rigs.iter().find(|(c, _)| c.parent() == *je)) {
                            (Some(k), Some((_, rig))) => jason_bodies(&k.info, rig, &globals),
                            _ => vec![],
                        };
                        if bodies.is_empty() {
                            bodies.push(BodyShape::Capsule(jxf.translation + Vec3::Y * cap.radius, jxf.translation + Vec3::Y * (2.0 * cap.half_height - cap.radius).max(cap.radius), cap.radius));
                        }
                        swing_hits(prev, now, radius, half, owner, &bodies, &world)
                    });
                    if let Some((je, jxf)) = hit {
                        let to = jxf.translation - xf.translation;
                        a.done = true;
                        // FUN_1403cbef0: her damage (StrengthAttackMod x the weapon's, x2 heavy), the stun roll against
                        // the weapon's StunChance, and HitKillerStaminaBoost back (f13-core::combat). His health isn't
                        // modelled: the damage is logged.
                        let combat = &tuning.counselor.combat;
                        let heavy = matches!(a.what, Use::Swing { heavy: true, .. });
                        let damage = combat.damage(stats.damage, &p.counselor.stats, heavy);
                        let roll = combat.stun_roll(&p.counselor.stats, rng.0.frand());
                        let stun = f13_core::combat::CombatTuning::stuns(stats.stun_chance, roll);
                        let back = combat.hit_stamina(&p.counselor.stats);
                        p.counselor.stamina = (p.counselor.stamina + back).min(tuning.counselor.stamina_max);
                        // Which of his sides she hit: his right is -X of his facing (+Z).
                        let jfwd = jxf.rotation * Vec3::Z;
                        let left = Vec2::new(-to.x, -to.z).perp_dot(Vec2::new(jfwd.x, jfwd.z)) < 0.0;
                        info!(
                            "hit Jason ({damage:.1} damage{}, roll {roll} vs stun chance {:.0}: {}; stamina +{back:.0})",
                            if heavy { ", heavy" } else { "" },
                            stats.stun_chance,
                            if stun { "stun" } else { "no stun" }
                        );
                        commands.entity(je).insert(HitJason { stun: stun.then_some(Stun::Weapon { left }), from: xf.translation });
                        // No impact effect: a melee hit's path to it, SCWeapon::SpawnHitEffect (0x1404558e0), is never
                        // called in this build (MODLOG "Melee hits spawn no impact effects").
                        let name = items.get(a.item).map_or("weapon".to_string(), |i| i.name.clone());
                        wear_weapon(&mut commands, &mut wear, &mut inv, a.item, stats.durability, stats.attacking_wear, &name);
                    }
                } else {
                    a.done = true; // missed
                }
            }
            Use::Fire { kind: ItemKind::Gun { projectile, pellets, spread_deg, velocity, impacts, impact_sounds, impact_decals, .. } } => {
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
                } else {
                    // SCGun::Fire (0x14044ea80): each pellet tilted off the aim by rand / 32767 x MaxSpreadAngle (about
                    // aim x up), spun about the aim by rand / 32767 x 360 degrees, traced FireDistance /
                    // sin(180 - (tilt + 90)) out; a hit on Jason counts toward his stun, a hit on the world spawns
                    // the gun's PelletImpactTemplate FX for its surface (MULTICAST_SpawnPelletImpact 0x14049bf10 ->
                    // vt +0x6f0 0x140452b30 -> the impact spawn 0x14035e130). The pellets draw from their own
                    // stream (the game's rand(); GameRng stays the AI's).
                    let mut seed = (time.elapsed_secs_f64() * 1e6) as u64 | 1;
                    let mut r = || {
                        seed ^= seed << 13;
                        seed ^= seed >> 7;
                        seed ^= seed << 17;
                        (seed % 32768) as f32 / 32767.0
                    };
                    let world = collision.with(&dynamic);
                    let axis = aim.cross(Vec3::Y).normalize_or(Vec3::X);
                    let mut on_jason = 0;
                    for _ in 0..pellets {
                        let tilt = r() * spread_deg;
                        let spin = r() * 360.0;
                        let dir = Quat::from_axis_angle(aim, spin.to_radians()) * (Quat::from_axis_angle(axis, tilt.to_radians()) * aim);
                        let reach = SHOTGUN_RANGE_M / tilt.to_radians().cos().max(1e-3);
                        let world_hit = f13_core::collision::segment_hit_tri(&world, chest, chest + dir * reach, 2.0).map(|(t, n, s)| (t * reach, n, s));
                        let jason_hit = jason.and_then(|(_, jxf)| ray_hits_capsule(chest, dir, jxf.translation)).map(|p| p.distance(chest)).filter(|d| *d <= reach);
                        match (jason_hit, world_hit) {
                            (Some(dj), w) if w.is_none_or(|w| dj < w.0) => on_jason += 1,
                            (_, Some((d, n, surface))) => {
                                let at = chest + dir * d;
                                let surface = if surface == f13_core::collision::Tri::TERRAIN { sounds.as_ref().map_or(0, |s| s.terrain_surface(at.x, at.z)) } else { surface };
                                // GetImpactFX: surface types 1..22 their own (none if unset), else DefaultFX.
                                let fx = if (1..=22).contains(&surface) { impacts.get(2 + surface as usize).cloned().flatten() } else { impacts.first().cloned().flatten() };
                                if let Some(fx) = fx {
                                    let n = if n.dot(dir) > 0.0 { -n } else { n };
                                    commands.write_message(crate::particles::SpawnParticles::impact(&fx, at, n));
                                }
                                // GetImpactSound: the surface's (none if unset: "No Sound Cue to Play!"), else DefaultSound (a
                                // pellet isn't heavy).
                                let cue = if (1..=22).contains(&surface) { impact_sounds.get(surface as usize).cloned().flatten() } else { impact_sounds.first().cloned().flatten() };
                                if let Some(cue) = cue {
                                    commands.trigger(crate::audio::PlaySound { cue, at: Some(at) });
                                }
                                // GetImpactDecal: the surface's, DefaultDecal if it has none (or no surface type).
                                let decal = (1..=22).contains(&surface).then(|| impact_decals.get(2 + surface as usize).cloned().flatten()).flatten().or_else(|| impact_decals.first().cloned().flatten());
                                if let Some(decal) = decal {
                                    let n = if n.dot(dir) > 0.0 { -n } else { n };
                                    crate::decals::spawn_impact(&mut commands, &decal, at, n, facing, &mut r, |a, b| {
                                        f13_core::collision::segment_hit_tri(&world, a, b, 2.0).map(|(t, n, _)| (a + (b - a) * t, n))
                                    });
                                }
                            }
                            _ => {}
                        }
                    }
                    if let (Some((je, _)), true) = (jason, on_jason > 0) {
                        info!("shotgun hit Jason ({on_jason} of {pellets} pellets)");
                        commands.entity(je).insert(HitJason { stun: Some(Stun::Shotgun), from: xf.translation });
                    } else {
                        info!("shotgun missed");
                    }
                }
                // One shot: the gun stays in her hands, spent.
                commands.entity(a.item).insert(Spent);
            }
            Use::Fire { .. } => a.done = true,
            Use::Throw => {
                a.done = true;
                // From her hand (the item's socket on her rig, as last posed), along the camera's yaw.
                let socket = items.get(a.item).map_or("WeaponSocket", |i| i.hand_socket.as_str());
                let hand = character.as_ref().and_then(|c| rigs.iter().find(|(p, _)| p.parent() == pe).and_then(|(_, rig)| rig.socket(&c.info, socket, &globals)));
                if let Ok(item) = items.get(a.item) {
                    commands.trigger(ClassScore::new(&item.class, "UseScoreEvent"));
                }
                inv.pockets.retain(|&e| e != a.item);
                commands.entity(a.item).despawn();
                let (mesh, mat) = (assets.0.add(Cuboid::new(0.06, 0.06, 0.14)), assets.1.add(StandardMaterial { base_color: Color::srgb(0.7, 0.05, 0.05), ..default() }));
                let vel = camera.0.look().with_y(0.0).normalize_or(facing.with_y(0.0).normalize_or_zero()) * THROW_SPEED;
                let from = hand.map_or(chest + facing * 0.4, |h| h.translation);
                commands.spawn((Mesh3d(mesh), MeshMaterial3d(mat), Transform::from_translation(from), Firecracker { vel, age: 0.0, stunned: false }));
            }
            Use::PlaceTrap => {
                a.done = true;
                let world = collision.with(&dynamic);
                let at = xf.translation + facing * 0.7;
                let y = surface_below(&world, at.x, at.z, at.y + 0.5, at.y - 2.0).map_or(at.y, |s| s.0);
                inv.hand = None;
                commands
                    .entity(a.item)
                    .remove::<(Carried, ChildOf, InHand, HeldAt, Item)>()
                    .insert((ArmedTrap { clear: false, killer: false }, crate::prop_anim::PropAnim::new(crate::prop_anim::TRAP_ARMED), Transform::from_translation(Vec3::new(at.x, y, at.z)).with_rotation(xf.rotation), Visibility::Inherited));
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
#[allow(clippy::type_complexity)]
fn spring_traps(
    mut commands: Commands,
    tuning: Res<crate::player::PlayerTuning>,
    mut traps: Query<(Entity, &Transform, &mut ArmedTrap)>,
    jasons: Query<(Entity, &Transform, &Jason)>,
    mut player: Query<(Entity, &Transform, &mut Player, Has<Trapped>, Has<crate::jason::Held>, Has<crate::vehicle::Driving>, Option<&crate::hiding::Hiding>)>,
) {
    for (te, txf, mut trap) in &mut traps {
        // Her stepping in.
        if let Ok((pe, pxf, mut p, trapped, held, driving, hiding)) = player.single_mut() {
            let d = Vec2::new(pxf.translation.x - txf.translation.x, pxf.translation.z - txf.translation.z);
            let on = d.length() <= TRAP_RADIUS_M && (pxf.translation.y - txf.translation.y).abs() < 0.6;
            if !on {
                trap.clear = true;
            } else if trap.clear && !trapped && !held && !driving && hiding.is_none() && p.counselor.alive() {
                // Which foot: the side of her the trap is on.
                let fwd = pxf.rotation * Vec3::Z;
                let left = Vec2::new(fwd.x, fwd.z).perp_dot(-d) > 0.0;
                let foot = if left { 0 } else { 1 };
                let enter = if p.counselor.sprinting { TRAP_RUN[foot] } else { TRAP_WALK[foot] };
                let r = p.counselor.take_damage(f13_core::health::Hit { amount: TRAP_DAMAGE, blocked_with_weapon: false }, &tuning.counselor);
                // The trap's TrapScoreEvent to its armer when the armer or she is a killer (FUN_14039c670): his.
                if trap.killer {
                    commands.trigger(crate::match_flow::ClassScore::killer(KILLER_TRAP_CLASS, "TrapScoreEvent"));
                }
                info!("Vanessa stepped in a bear trap ({} foot): {:.0} damage (health {:.0})", if left { "left" } else { "right" }, r.dealt, p.counselor.health);
                commands.entity(te).remove::<ArmedTrap>().insert(crate::prop_anim::PropAnim::new(crate::prop_anim::TRAP_SPRUNG));
                p.velocity = Vec2::ZERO;
                if r.died {
                    commands.entity(pe).insert(PlayAction(DEATH.into())).remove::<(Blocking, ItemAction)>();
                } else {
                    commands.entity(pe).insert((Trapped { trap: te, left, enter, t: 0.0, wiggle: default(), out: None }, PlayAction(enter.into()))).remove::<(Blocking, ItemAction)>();
                }
                continue;
            }
        }
        for (je, jxf, j) in jasons.iter().filter(|_| !trap.killer) {
            let d = Vec2::new(jxf.translation.x - txf.translation.x, jxf.translation.z - txf.translation.z).length();
            if d < 2.0 {
                trace!("trap: Jason {d:.2} m away ({:?}, dy {:.2}); she is {:.2} m from it", j.state, jxf.translation.y - txf.translation.y, player.iter().next().map_or(0.0, |p| p.1.translation.distance(txf.translation)));
            }
            if d <= TRAP_RADIUS_M && (jxf.translation.y - txf.translation.y).abs() < 0.6 && matches!(j.state, crate::jason::JasonState::Hunting) {
                info!("Jason stepped in a bear trap");
                commands.entity(je).insert(HitJason { stun: Some(Stun::BearTrap), from: txf.translation });
                commands.entity(te).remove::<ArmedTrap>().insert(crate::prop_anim::PropAnim::new(crate::prop_anim::TRAP_SPRUNG));
            }
        }
    }
}

/// Caught in a trap: the struggle's cycle while she wiggles (f13-core::stun), then stepping out.
fn struggle_in_trap(
    mut commands: Commands,
    time: Res<Time>,
    character: Option<Res<CounselorCharacter>>,
    struggle: Res<crate::jason::Struggle>,
    mut player: Query<(Entity, &mut Trapped, &Player)>,
) {
    let Ok((pe, mut tr, p)) = player.single_mut() else { return };
    let dt = time.delta_secs();
    let foot = if tr.left { 0 } else { 1 };
    let len = |n: &str| character.as_ref().and_then(|c| c.length(n)).unwrap_or(2.0);
    if !p.counselor.alive() {
        commands.entity(pe).remove::<Trapped>();
        return;
    }
    if let Some(out) = tr.out.as_mut() {
        *out += dt;
        if *out >= len(TRAP_EXIT[foot]) {
            commands.entity(pe).remove::<(Trapped, PlayAction)>();
        }
        return;
    }
    let before = tr.t;
    tr.t += dt;
    let enter_len = len(tr.enter);
    if before < enter_len && tr.t >= enter_len {
        commands.entity(pe).insert(PlayAction(TRAP_CYCLE[foot].into()));
    }
    if tr.wiggle.tick(dt, struggle.input, &TRAP_STRUGGLE) {
        info!("Vanessa: free of the trap after {:.1}s", tr.t);
        tr.out = Some(0.0);
        commands.entity(pe).insert(PlayAction(TRAP_EXIT[foot].into()));
        if let Ok(mut t) = commands.get_entity(tr.trap) {
            t.insert(crate::prop_anim::PropAnim::new(crate::prop_anim::TRAP_ESCAPED));
        }
    }
}

/// GLB_Map for her: the HUD's ShowCounselorMap (0x14043a430: when no menu is up, or the one up is the objectives
/// menu) toggles CounselorHUDWidget's OnShowCounselorMap, her objectives and map screen (ui.rs `map_screen`), with the
/// map or without it.
#[derive(Resource, Default)]
pub struct MapScreen(pub bool);

fn toggle_map(keys: Res<ButtonInput<KeyCode>>, mut screen: ResMut<MapScreen>, play: Option<Res<crate::jason_player::PlayJason>>) {
    if play.is_none() && keys.just_pressed(KeyCode::KeyM) {
        screen.0 = !screen.0;
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
