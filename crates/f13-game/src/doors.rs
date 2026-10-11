//! Cabin doors: open/close, lock/unlock, and the leaf swinging on its pivot (a closed leaf blocks the way).
//!
//! Rules from `SCDoor` (CanInteractWith / OnInteract / IsInteractorInside):
//! - The interactor is **inside** when behind the door's forward axis: `dot(root forward, interactor - root) < 0`.
//! - **Press** (interact method 1): locked or barricaded → the locked sound; otherwise open it if closed (from either
//!   side), or close it.
//! - **Hold** (method 2, the interactable's `HoldTimeLimit`): from inside a closed, lockable door (lock or unlock), or
//!   from outside a locked one (unlock). Toggles the lock.
//! - A press is a release before the hold starts (`MinInteractHoldTime`), as with the game's hold interactables.
//!
//! - A door with a barricade (windows.rs) uses it instead of the lock: hold from inside toggles it, and a barred door
//!   doesn't open.
//!
//! The swing (SCDoor's OpenCloseDoorDriver, decompiled): as the opener's montage enters its "Driver: OpenCloseDoor"
//! window, DriverBegin (0x140353c60) sets the Pivot's start and end yaw, 0 to -90 to open, -90 to 0 to close: the
//! leaf always opens the same way, whoever opens it. DriverTick (0x140353cb0) sets the Pivot's relative yaw to
//! start + (end - start) x clamp(the opener's DoorRotation curve, 0, 1); OpenCloseDoorCompleted (0x140353b20) snaps
//! it to the end if the montage ends first.
//! Here the opener's move passes its take's DoorRotation curve and its montage time ([`Door::drive`]); without one
//! (a press with no take) the leaf turns at a steady rate over [`SWING_SECONDS`].

use bevy::prelude::*;
use std::collections::HashMap;

pub const SWING_SECONDS: f32 = 0.5;
const OPEN_DEGREES: f32 = 90.0;
/// What an enabled barricade lets through of a hit (SCDoor's TakeDamage constant).
const BARRICADE_DAMAGE_SCALE: f32 = 0.5;

/// A destructible wall (SCDestructableWall): its health (BaseHealth), and the section that goes when it's broken
/// through (its mesh and its Collision box; the frame stays).
#[derive(Component)]
pub struct Wall {
    /// Health left (of BaseHealth; the game keeps it as a fraction of BaseHealth: the same thresholds).
    pub health: f32,
    pub base_health: f32,
    /// Light, Medium and HeavyDestructionThreshold (fractions of BaseHealth).
    pub thresholds: [f32; 3],
    pub broken: bool,
    pub section: Option<Entity>,
    pub collision: Option<Entity>,
    /// BurstExplosionForce, and HitExplosionForce (the force a blow that breaks it gives its chunks: TakeDamage's
    /// DestroyWall(killer, HitExplosionForce)).
    pub burst_force: f32,
    pub hit_force: f32,
    /// WallLifespanAfterDestruction (s).
    pub lifespan: f32,
    /// The section's place and look, and the Light / Medium / Heavy DestructionInside and Outside meshes it's swapped
    /// for as it takes damage (`[inside, outside][stage]`).
    pub section_xf: Transform,
    pub overrides: Option<crate::materials::MaterialOverrides>,
    pub damaged: [[Option<Handle<WorldAsset>>; 3]; 2],
    /// Its Interactable (bCanAttackToInteract: Jason's attack breaks it down): where it is (wall local space) and
    /// its DistanceLimit (cm).
    pub interact: Vec3,
    pub reach: f32,
    /// HitParticle and DestroyParticle (particle system names).
    pub hit_particle: Option<String>,
    pub destroy_particle: Option<String>,
}

impl Wall {
    /// SCDestructableWall::IsInteractorInside (0x14034bdf0): the direction from `at` to the wall (flat) along its
    /// Facing arrow (the wall's +X).
    pub fn is_inside(wall: &GlobalTransform, at: Vec3) -> bool {
        let to = (wall.translation() - at).with_y(0.0).normalize_or_zero();
        (wall.rotation() * Vec3::X).dot(to) > 0.0
    }

    /// A blow of `damage` on `wall` (this) from `inside` or out (its TakeDamage 0x140360430 and OnRep_Health
    /// 0x140352db0): whether it broke through. Short of that the section shows the damage state the blow crossed
    /// into, the side's mesh.
    pub fn damage(&mut self, wall: Entity, damage: f32, inside: bool, commands: &mut Commands) -> bool {
        if self.broken {
            return false;
        }
        let before = self.health;
        self.health = (self.health - damage).max(0.0);
        if self.health > 0.0 {
            let base = self.base_health.max(1.0);
            let (was, now) = (before / base, self.health / base);
            // Heavy if it crossed that threshold, else medium, else light.
            if let Some(stage) = (0..3).rev().find(|&i| now <= self.thresholds[i] && self.thresholds[i] < was) {
                self.show_damage(wall, inside, stage, commands);
            }
            return false;
        }
        self.broken = true;
        // The section flies apart (BreakApart copies it), its collision goes.
        if let Some(s) = self.section {
            commands.entity(s).insert(Visibility::Hidden);
        }
        if let Some(c) = self.collision {
            commands.entity(c).despawn();
        }
        true
    }

    /// Swaps the section for damage state `stage`'s mesh on that side (SetDestructibleMesh: a new section entity, same
    /// place).
    fn show_damage(&mut self, wall: Entity, inside: bool, stage: usize, commands: &mut Commands) {
        let Some(scene) = self.damaged[usize::from(!inside)][stage].clone() else { return };
        if let Some(old) = self.section {
            commands.entity(old).despawn();
        }
        let mut section = commands.spawn((bevy::world_serialization::WorldAssetRoot(scene), self.section_xf, ChildOf(wall)));
        if let Some(o) = &self.overrides {
            section.insert(o.clone());
        }
        self.section = Some(section.id());
    }
}

#[derive(Component, Debug)]
pub struct Door {
    pub open: bool,
    pub locked: bool,
    pub lockable: bool,
    /// The entity the leaf hangs on (rotates about its local Y).
    pub pivot: Entity,
    /// Closed pivot transform, local to the door.
    pub pivot_closed: Transform,
    /// Interact point (door local space) and range in cm.
    pub interact: Vec3,
    pub reach: f32,
    /// Seconds to hold for the lock (the interact component's `HoldTimeLimit`).
    pub hold_time: f32,
    /// Current and target swing angle (radians, about the pivot's Y).
    pub angle: f32,
    pub target: f32,
    /// Health left against Jason's weapon (`BaseDoorHealth`; whole numbers, an int in the game); at 0 the door is
    /// broken: gone, open for good.
    pub health: f32,
    pub broken: bool,
    /// How long the current swing takes when no take drives it ([`SWING_SECONDS`]).
    pub swing_seconds: f32,
    /// The opener's take driving the swing: its DoorRotation curve, its montage time, the driver window's end, and
    /// the angle the swing started from.
    pub driver: Option<(f13_core::curve::RichCurve, f32, f32, f32)>,
    /// `BaseDoorHealth`, and the Light / Medium / Heavy DestructionThreshold fractions of it.
    pub base_health: f32,
    pub damage_thresholds: [f32; 3],
    /// BreakExplosionForce and BurstExplosionForce: how hard its chunks fly when it's broken down or burst through.
    pub break_force: f32,
    pub burst_force: f32,
    /// Its OutArrow (door local: the radius damage's origin) and LifespanAfterDestruction (s).
    pub out_arrow: Vec3,
    pub lifespan: f32,
    /// Its special move spots (component name -> door local): where her open, close, lock and unlock moves play from.
    pub spots: std::collections::HashMap<String, Transform>,
    /// The leaf's look (None in tests).
    pub visuals: Option<DoorVisuals>,
}

/// A door broken down or burst through, a wall broken through: the destructible piece (its leaf, its section: the
/// mesh's chunks, the `Part` joints) breaks as APEX breaks it. The game calls UDestructibleComponent::
/// ApplyRadiusDamage(100, `origin`, `radius`, `force`, bFullDamage) on it (SCDoor's destroy 0x14034d410: origin the
/// OutArrow + 5 cm toward the killer from the InteractComponent, radius 3000; SCDestructableWall::DestroyWall
/// 0x14033cfc0: the Interactable + 10 cm, radius 300), then clears its CastShadow and destroys it LifespanAfterDestruction
/// (10 s) later (0x14034f810). APEX_Destructible_x64.dll (its PDB's names): applyRadiusDamage_immediate damages each
/// visible chunk within the radius (DestructibleStructure::damageChunk: the default behaviour group's damage spread,
/// minimumRadius 0, radiusMultiplier 1, falloffExponent 1, so a fraction (R - d) / R of the damage with d the chunk's
/// distance from the origin; the door's DamageThreshold 1 against 100: every chunk breaks free), and gives each freed
/// chunk the impulse normalize(its centroid - origin) x fraction x momentum (fractureImpulseScale 0 adds nothing),
/// applied at the point on its bounding sphere toward the origin (fractureChunk: so no spin). Its mass:
/// (hull volume x density)^0.5 (DestructibleScene::tasked_beforeTick: massScale 1, scaledMassExponent 0.5, the module's
/// defaults; density PhysicalMaterial Density 1 x 0.001 kg/cm³ from DestructibleComponent's p3BodyDescTemplate.density
/// 0x141e43f08). Then PhysX: gravity, linear damping 0.01 (FBodyInstance's), angular 0. PLACEHOLDER: PhysX's contact
/// solver (a chunk's centroid traced against the static world, bouncing with the default material's restitution 0.3
/// and friction 0.7 taken off its sliding speed), the chunk's convex hull from its render vertices (APEX's cooked hulls
/// aren't exported) and its centroid the hull bounds' centre.
#[derive(Event, Clone, Copy, Debug)]
pub struct BreakApart {
    pub piece: Entity,
    /// The radius damage's origin and radius (Bevy space, m).
    pub origin: Vec3,
    pub radius: f32,
    /// ImpulseStrength (Break / Burst / HitExplosionForce: kg cm/s).
    pub force: f32,
    /// LifespanAfterDestruction (s): the pieces are gone after it.
    pub lifespan: f32,
}

impl BreakApart {
    /// The radius damage's origin: `at` (the OutArrow, the Interactable) moved `offset_cm` toward the killer, the
    /// direction flat (SCDoor's FUN_140331b10: killer - the InteractComponent, Z dropped, normalized).
    pub fn origin(at: Vec3, interact: Vec3, killer: Vec3, offset_cm: f32) -> Vec3 {
        at + (killer - interact).with_y(0.0).normalize_or_zero() * offset_cm / 100.0
    }
}

/// The broken piece in flight: how long it has been, and how long it lasts.
#[derive(Component)]
struct Debris {
    t: f32,
    lifespan: f32,
}

/// One freed chunk: its velocity (world, m/s) and where its centroid is (world).
#[derive(Component)]
struct Chunk {
    velocity: Vec3,
    centroid: Vec3,
}

/// A chunk's shape in its mesh's space (m): its hull bounds and volume (cm³).
#[derive(Clone, Copy, Debug)]
pub struct ChunkShape {
    pub min: Vec3,
    pub max: Vec3,
    pub volume_cm3: f32,
}

impl ChunkShape {
    pub fn centre(&self) -> Vec3 {
        (self.min + self.max) * 0.5
    }
    pub fn radius(&self) -> f32 {
        (self.max - self.min).length() * 0.5
    }
}

/// Each skin joint's chunk of a destructible mesh's .glb (the vertices weighted to it): its convex hull's bounds and
/// volume, by joint name.
pub fn chunk_shapes(path: &std::path::Path) -> HashMap<String, ChunkShape> {
    let (Some(prims), Some(names)) = (f13_data::glb::read_primitives(path), f13_data::glb::joint_names(path)) else { return HashMap::new() };
    let mut points: Vec<Vec<Vec3>> = vec![vec![]; names.len()];
    for p in &prims {
        for ((pos, j), w) in p.positions.iter().zip(&p.joints).zip(&p.weights) {
            let k = (0..4).max_by(|a, b| w[*a].total_cmp(&w[*b])).unwrap_or(0);
            if let Some(v) = points.get_mut(j[k] as usize) {
                v.push(Vec3::from_array(*pos));
            }
        }
    }
    names
        .into_iter()
        .zip(points)
        .filter(|(_, p)| p.len() >= 4)
        .map(|(name, p)| {
            let hull = f13_core::collision::convex_hull(&p);
            let volume = hull.iter().map(|t| t.a.dot(t.b.cross(t.c))).sum::<f32>().abs() / 6.0;
            let (min, max) = p.iter().fold((Vec3::splat(f32::MAX), Vec3::splat(f32::MIN)), |(lo, hi), v| (lo.min(*v), hi.max(*v)));
            (name, ChunkShape { min, max, volume_cm3: volume * 1.0e6 })
        })
        .collect()
}

/// The destructible meshes' chunk shapes, by scene path.
#[derive(Resource, Default)]
struct ChunkShapes(HashMap<String, std::sync::Arc<HashMap<String, ChunkShape>>>);

/// UE's default gravity (cm/s² / 100) and the chunks' linear damping (FBodyInstance's LinearDamping 0.01, constructor
/// 0x1415ee0ad).
const GRAVITY: f32 = 9.8;
const LINEAR_DAMPING: f32 = 0.01;
/// The default physical material's (UPhysicalMaterial's constructor 0x1416148b0): Restitution 0.3, Friction 0.7,
/// Density 1 (g/cm³; x 0.001 kg/cm³ for APEX).
const RESTITUTION: f32 = 0.3;
const FRICTION: f32 = 0.7;
const DENSITY_KG_PER_CM3: f32 = 0.001;

pub struct DebrisPlugin;

impl Plugin for DebrisPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<ChunkShapes>().add_observer(break_apart).add_systems(Update, fly);
    }
}

/// A freed chunk's velocity (m/s) from the radius damage (see [`BreakApart`]): `centroid` its world centroid, `reach`
/// its bounding radius (m), `volume_cm3` its hull's volume.
pub fn chunk_velocity(centroid: Vec3, reach: f32, volume_cm3: f32, origin: Vec3, radius: f32, force: f32) -> Vec3 {
    // damageChunk: the chunk's distance from the origin (to its bounding sphere), the fraction (R - d) / R.
    let d = (centroid.distance(origin) - reach).max(0.0);
    let fraction = if radius > 0.0 { ((radius - d) / radius).clamp(0.0, 1.0) } else { 1.0 };
    let mass = (volume_cm3 * DENSITY_KG_PER_CM3).max(1e-6).sqrt();
    // Impulse in kg cm/s over kg: cm/s; to m/s.
    (centroid - origin).normalize_or_zero() * fraction * force / mass / 100.0
}

#[allow(clippy::too_many_arguments)]
fn break_apart(
    ev: On<BreakApart>,
    mut commands: Commands,
    pieces: Query<(&bevy::world_serialization::WorldAssetRoot, &GlobalTransform, Option<&crate::materials::MaterialOverrides>)>,
    assets: Res<AssetServer>,
    export: Res<crate::ExportRoot>,
    mut shapes: ResMut<ChunkShapes>,
) {
    let Ok((scene, g, overrides)) = pieces.get(ev.piece) else { return };
    let at = g.compute_transform();
    let path = assets.get_path(&scene.0).map(|p| p.path().to_string_lossy().into_owned()).unwrap_or_default();
    let chunks = shapes.0.entry(path.clone()).or_insert_with(|| std::sync::Arc::new(chunk_shapes(&export.0.join(&path)))).clone();
    debug!("break apart {path}: {} chunks, force {:.0} radius {:.1} m", chunks.len(), ev.force, ev.radius);
    // Unshadowed (the destroy clears the component's CastShadow).
    let mut e = commands.spawn((
        bevy::world_serialization::WorldAssetRoot(scene.0.clone()),
        at,
        Debris { t: 0.0, lifespan: ev.lifespan },
        crate::Placed { cast_shadow: false, cull_m: 0.0 },
        PendingChunks { chunks, origin: ev.origin, radius: ev.radius, force: ev.force },
    ));
    if let Some(o) = overrides {
        e.insert(o.clone());
    }
}

/// A broken piece whose scene hasn't spawned its chunk joints yet: their shapes and the blow.
#[derive(Component)]
struct PendingChunks {
    chunks: std::sync::Arc<HashMap<String, ChunkShape>>,
    origin: Vec3,
    radius: f32,
    force: f32,
}

#[allow(clippy::type_complexity)]
fn fly(
    mut commands: Commands,
    time: Res<Time>,
    mut debris: Query<(Entity, &mut Debris, &GlobalTransform, Option<&PendingChunks>)>,
    children: Query<&Children>,
    names: Query<&Name>,
    parents: Query<&ChildOf>,
    globals: Query<&GlobalTransform>,
    mut chunks: Query<(&mut Transform, Option<&mut Chunk>)>,
    collision: Res<crate::collision::WorldCollision>,
    dynamic: Res<crate::collision::DynamicColliders>,
) {
    let dt = time.delta_secs();
    let (world, _) = collision.with(&dynamic);
    for (e, mut d, g, pending) in &mut debris {
        d.t += dt;
        if d.t > d.lifespan {
            commands.entity(e).despawn();
            continue;
        }
        // Free the chunks once the scene has them: each its velocity from the blow.
        if let Some(p) = pending {
            let mut freed = 0;
            for c in children.iter_descendants(e) {
                let Some(shape) = names.get(c).ok().and_then(|n| p.chunks.get(n.as_str())) else { continue };
                let centroid = g.transform_point(shape.centre());
                let reach = shape.radius() * g.scale().max_element();
                let velocity = chunk_velocity(centroid, reach, shape.volume_cm3, p.origin, p.radius, p.force);
                commands.entity(c).insert(Chunk { velocity, centroid });
                freed += 1;
            }
            if freed > 0 {
                commands.entity(e).remove::<PendingChunks>();
            }
            continue;
        }
        for c in children.iter_descendants(e) {
            let Ok((mut xf, Some(mut ch))) = chunks.get_mut(c) else { continue };
            // Gravity and PhysX's linear damping, then the move, stopped and bounced at the static world.
            ch.velocity += Vec3::NEG_Y * GRAVITY * dt;
            ch.velocity *= (1.0 - LINEAR_DAMPING * dt).max(0.0);
            let from = ch.centroid;
            let mut to = from + ch.velocity * dt;
            if let Some((t, n)) = f13_core::collision::segment_hit_normal(&world, from, to, 0.25).filter(|(_, n)| n.dot(ch.velocity) < 0.0) {
                to = from + (to - from) * t + n * 0.005;
                let vn = n * ch.velocity.dot(n);
                let vt = ch.velocity - vn;
                ch.velocity = vt * (1.0 - FRICTION).max(0.0) - vn * RESTITUTION;
            }
            let moved = to - from;
            ch.centroid = to;
            // Into the joint's parent's space.
            let local = parents.get(c).ok().and_then(|p| globals.get(p.parent()).ok()).map_or(moved, |pg| pg.affine().inverse().transform_vector3(moved));
            xf.translation += local;
        }
    }
}

/// The leaf entity and what it shows: its transform on the pivot, its material overrides, and the Light, Medium and
/// Heavy DestructionMesh scenes it's swapped for as the door takes damage.
#[derive(Debug)]
pub struct DoorVisuals {
    pub leaf: Entity,
    pub leaf_xf: Transform,
    pub overrides: Option<crate::materials::MaterialOverrides>,
    pub damaged: [Option<Handle<WorldAsset>>; 3],
}

impl Door {
    /// `SCDoor::IsInteractorInside`.
    pub fn is_inside(door: &GlobalTransform, at: Vec3) -> bool {
        // Unreal +X is the door's forward; it maps to Bevy +X in the door's frame.
        let forward = door.rotation() * Vec3::X;
        forward.dot(at - door.translation()) < 0.0
    }

    /// What a counselor at `at` may do: (press: open/close, hold: lock/unlock or barricade). From outside, a locked
    /// door offers only the hold, which unlocks it (`UnlockDoorOutside`); from inside a closed door offers the
    /// lock, or its barricade when it has one (`barricade` = Some(enabled)).
    pub fn can_interact(&self, door: &GlobalTransform, at: Vec3, barricade: Option<bool>) -> (bool, bool) {
        // A broken door is gone (SCDoor::CanInteractWith: bIsDestroyed).
        if self.broken {
            return (false, false);
        }
        let barred = barricade == Some(true);
        let press = !self.locked && !barred;
        let hold = if Door::is_inside(door, at) { !self.open && (self.lockable || barricade.is_some()) } else { self.locked };
        (press, hold)
    }

    /// The opener's take drives the swing just started (OpenCloseDoorDriverBegin): `curve` its DoorRotation, `t`
    /// its montage time now, `end` its OpenCloseDoor window's end.
    pub fn drive(&mut self, curve: Option<f13_core::curve::RichCurve>, t: f32, end: f32) {
        let from = if self.target > 0.0 { 0.0 } else { OPEN_DEGREES.to_radians() };
        self.driver = curve.map(|c| (c, t, end, from));
    }

    /// Press: open or close (or nothing when locked). Returns what happened, for the log. The leaf opens to the
    /// Pivot's -90 degree yaw (Unreal; +90 about Bevy's Y) from either side.
    pub fn press(&mut self, _door: &GlobalTransform, _opener: Vec3) -> &'static str {
        if self.locked {
            return "locked";
        }
        self.open = !self.open;
        self.swing_seconds = SWING_SECONDS;
        if !self.open {
            self.target = 0.0;
            return "closed";
        }
        self.target = OPEN_DEGREES.to_radians();
        "opened"
    }

    /// What a hit from Jason's weapon (`damage` already scaled by his `DamageWorldModifier`) takes off: half through
    /// an enabled barricade (SCDoor's TakeDamage, and BreakDownSync's check).
    pub fn hit_damage(damage: f32, barred: bool) -> f32 {
        if barred {
            damage * BARRICADE_DAMAGE_SCALE
        } else {
            damage
        }
    }

    /// A hit from Jason's weapon (see [`Door::hit_damage`]). Returns true when it breaks the door: it no longer
    /// locks, bars or blocks (the leaf is hidden by the caller). Short of that, the leaf shows the damage state the
    /// hit crossed into, if any.
    pub fn damage(&mut self, damage: f32, barred: bool, commands: &mut Commands) -> bool {
        if self.broken {
            return false;
        }
        let before = self.health;
        // Health is an int: the new value truncates.
        self.health = (self.health - Door::hit_damage(damage, barred)).trunc().max(0.0);
        if self.health > 0.0 {
            if let Some(stage) = self.damage_stage(before) {
                self.show_damage(stage, commands);
            }
            return false;
        }
        self.broken = true;
        self.open = true;
        self.locked = false;
        true
    }

    /// The damage state (0 light, 1 medium, 2 heavy) a hit from `before` health crossed into, as SCDoor's
    /// TakeDamage picks it: heavy if it crossed that threshold, else medium, else light.
    pub fn damage_stage(&self, before: f32) -> Option<usize> {
        let base = self.base_health.max(1.0);
        let (was, now) = (before / base, self.health / base);
        (0..3).rev().find(|&i| now <= self.damage_thresholds[i] && self.damage_thresholds[i] < was)
    }

    /// Swaps the leaf for damage state `stage`'s mesh (a new leaf entity on the pivot, same place).
    fn show_damage(&mut self, stage: usize, commands: &mut Commands) {
        let pivot = self.pivot;
        let Some(v) = self.visuals.as_mut() else { return };
        let Some(scene) = v.damaged[stage].clone() else { return };
        commands.entity(v.leaf).despawn();
        let mut leaf = commands.spawn((bevy::world_serialization::WorldAssetRoot(scene), v.leaf_xf, ChildOf(pivot)));
        if let Some(o) = &v.overrides {
            leaf.insert(o.clone());
        }
        v.leaf = leaf.id();
    }

    pub fn toggle_lock(&mut self) -> &'static str {
        self.locked = !self.locked;
        if self.locked {
            "locked"
        } else {
            "unlocked"
        }
    }
}

/// Swings leaves toward their target angle: along the driving take's curve (SCDoor's driver tick: start + (end -
/// start) x clamp(DoorRotation, 0, 1)), at its window's end the end pose (OpenCloseDoorCompleted's snap), else
/// steadily.
pub fn swing_doors(time: Res<Time>, mut doors: Query<&mut Door>, mut pivots: Query<&mut Transform>) {
    let dt = time.delta_secs();
    for mut d in &mut doors {
        if let Some((curve, t, end, from)) = d.driver.take() {
            let t = t + dt;
            d.angle = if t >= end { d.target } else { from + (d.target - from) * curve.eval(t).clamp(0.0, 1.0) };
            if t < end {
                d.driver = Some((curve, t, end, from));
            }
        } else if d.angle != d.target {
            let step = OPEN_DEGREES.to_radians() / d.swing_seconds.max(1e-3) * dt;
            let delta = (d.target - d.angle).clamp(-step, step);
            d.angle += delta;
        } else {
            continue;
        }
        if let Ok(mut t) = pivots.get_mut(d.pivot) {
            t.rotation = d.pivot_closed.rotation * Quat::from_rotation_y(d.angle);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The Higgins Haven door's heavy destruction mesh, when `F13_EXPORT` points at an export (skipped otherwise):
    /// fourteen chunks (Part00..13, the Root joint has no vertices), and the break's speeds.
    #[test]
    fn door_chunks_fly() {
        let Some(root) = std::env::var_os("F13_EXPORT").map(std::path::PathBuf::from) else { return };
        let path = root.join("SummerCamp/Content/Props/Manmade/Door/S_Door_HigginsHaven_Door_01_b_DM.glb");
        if !path.exists() {
            return;
        }
        let shapes = chunk_shapes(&path);
        assert_eq!(shapes.len(), 14, "{:?}", shapes.keys().collect::<Vec<_>>());
        let total: f32 = shapes.values().map(|c| c.volume_cm3).sum();
        // The leaf and its casing (1.34 m x 2.17 m x 18 cm overall): the hulls fill most of that box.
        assert!((100_000.0..520_000.0).contains(&total), "volume {total}");
        // BreakExplosionForce 5000 from 5 cm in front of the leaf: tens of metres a second, away from the blow.
        for c in shapes.values() {
            let v = chunk_velocity(c.centre(), c.radius(), c.volume_cm3, Vec3::new(0.0, 1.0, -0.05), 30.0, 5000.0);
            assert!((2.0..200.0).contains(&v.length()), "{v}");
        }
    }

    fn door(locked: bool) -> Door {
        Door {
            open: false,
            locked,
            lockable: true,
            pivot: Entity::PLACEHOLDER,
            pivot_closed: Transform::IDENTITY,
            interact: Vec3::ZERO,
            reach: 200.0,
            hold_time: 0.5,
            angle: 0.0,
            target: 0.0,
            health: 60.0,
            broken: false,
            swing_seconds: SWING_SECONDS,
            driver: None,
            base_health: 60.0,
            damage_thresholds: [0.75, 0.5, 0.25],
            spots: Default::default(),
            break_force: 5000.0,
            burst_force: 20000.0,
            out_arrow: Vec3::ZERO,
            lifespan: 10.0,
            visuals: None,
        }
    }

    /// Hits `d` and returns (broke, the damage state the hit crossed into).
    fn hit(d: &mut Door, barred: bool) -> (bool, Option<usize>) {
        let mut world = World::new();
        let mut queue = bevy::ecs::world::CommandQueue::default();
        let mut commands = Commands::new(&mut queue, &mut world);
        let before = d.health;
        let broke = d.damage(20.0, barred, &mut commands);
        (broke, if broke { None } else { d.damage_stage(before) })
    }

    #[test]
    fn a_locked_door_breaks_on_the_third_pickaxe_hit() {
        let mut d = door(true);
        // 60 -> 40 crosses 75% (light damage), 40 -> 20 crosses 50% (medium).
        assert_eq!(hit(&mut d, false), (false, Some(0)));
        assert_eq!(hit(&mut d, false), (false, Some(1)));
        assert!(d.locked && !d.open);
        assert_eq!(hit(&mut d, false), (true, None));
        assert!(d.broken && d.open && !d.locked);
        assert_eq!(hit(&mut d, false), (false, None), "already broken");
    }

    #[test]
    fn a_wall_shows_its_sides_damage_and_breaks() {
        let mut world = World::new();
        let wall_e = world.spawn_empty().id();
        let mut queue = bevy::ecs::world::CommandQueue::default();
        let mut commands = Commands::new(&mut queue, &mut world);
        let mesh = || Some(Handle::<WorldAsset>::default());
        let mut w = Wall {
            interact: Vec3::ZERO,
            reach: 300.0,
            hit_particle: None,
            destroy_particle: None,
            health: 60.0,
            base_health: 60.0,
            thresholds: [0.8, 0.6, 0.4],
            broken: false,
            section: None,
            collision: None,
            burst_force: 2000.0,
            hit_force: 800.0,
            lifespan: 10.0,
            section_xf: Transform::IDENTITY,
            overrides: None,
            damaged: [[mesh(), mesh(), mesh()], [None, None, None]],
        };
        // 60 -> 40 (2/3: light) from inside: a new section.
        assert!(!w.damage(wall_e, 20.0, true, &mut commands));
        let light = w.section.expect("light mesh");
        // 40 -> 20 (1/3: past medium and heavy: heavy).
        assert!(!w.damage(wall_e, 20.0, true, &mut commands));
        assert_ne!(w.section, Some(light));
        // The outside has no meshes here: no swap.
        let heavy = w.section;
        assert!(w.damage(wall_e, 20.0, false, &mut commands));
        assert!(w.broken && w.section == heavy);
    }

    #[test]
    fn a_wall_is_hit_from_inside_when_its_facing_points_away_from_him() {
        let wall = GlobalTransform::from(Transform::from_xyz(0.0, 0.0, 0.0));
        assert!(Wall::is_inside(&wall, Vec3::new(-1.0, 0.0, 0.0)));
        assert!(!Wall::is_inside(&wall, Vec3::new(1.0, 0.0, 0.0)));
    }

    #[test]
    fn a_barricade_halves_each_hit() {
        let mut d = door(false);
        // 50, 40 (light), 30 (medium: exactly 50%), 20, 10 (heavy).
        let stages: Vec<_> = (0..5).map(|_| hit(&mut d, true)).collect();
        assert_eq!(stages, [(false, None), (false, Some(0)), (false, Some(1)), (false, None), (false, Some(2))]);
        assert_eq!(d.health, 10.0);
        assert!(hit(&mut d, true).0, "the sixth hit");
    }
}
