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
//! PLACEHOLDERS: the swing (90 degrees, away from whoever opens it, in [`SWING_SECONDS`]) stands in for the
//! special-move montages and the door's anim driver.

use bevy::prelude::*;

pub const SWING_SECONDS: f32 = 0.5;
const OPEN_DEGREES: f32 = 90.0;
/// What an enabled barricade lets through of a hit (SCDoor's TakeDamage constant).
const BARRICADE_DAMAGE_SCALE: f32 = 0.5;

/// A destructible wall (SCDestructableWall): its health (BaseHealth), and the section that goes when it's broken
/// through (its mesh and its Collision box; the frame stays).
#[derive(Component)]
pub struct Wall {
    pub health: f32,
    pub broken: bool,
    pub section: Option<Entity>,
    pub collision: Option<Entity>,
}

impl Wall {
    /// A blow of `damage`: whether it broke through.
    pub fn damage(&mut self, damage: f32, commands: &mut Commands) -> bool {
        if self.broken {
            return false;
        }
        self.health -= damage;
        if self.health > 0.0 {
            return false;
        }
        self.broken = true;
        for e in [self.section, self.collision].into_iter().flatten() {
            commands.entity(e).despawn();
        }
        true
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
    /// Leaf centre relative to the pivot (to pick the swing direction).
    pub leaf_center: Vec3,
    /// Current and target swing angle (radians, about the pivot's Y).
    pub angle: f32,
    pub target: f32,
    /// Health left against Jason's weapon (`BaseDoorHealth`; whole numbers, an int in the game); at 0 the door is
    /// broken: gone, open for good.
    pub health: f32,
    pub broken: bool,
    /// How long the current swing takes (the opener's OpenCloseDoor driver window; [`SWING_SECONDS`] otherwise).
    pub swing_seconds: f32,
    /// `BaseDoorHealth`, and the Light / Medium / Heavy DestructionThreshold fractions of it.
    pub base_health: f32,
    pub damage_thresholds: [f32; 3],
    /// Its special move spots (component name -> door local): where her open, close, lock and unlock moves play from.
    pub spots: std::collections::HashMap<String, Transform>,
    /// The leaf's look (None in tests).
    pub visuals: Option<DoorVisuals>,
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

    /// Press: open or close (or nothing when locked). Returns what happened, for the log.
    pub fn press(&mut self, door: &GlobalTransform, opener: Vec3) -> &'static str {
        if self.locked {
            return "locked";
        }
        self.open = !self.open;
        self.swing_seconds = SWING_SECONDS;
        if !self.open {
            self.target = 0.0;
            return "closed";
        }
        // Swing away from the opener: pick the side whose leaf centre ends up farther from them.
        let pivot = door.transform_point(self.pivot_closed.translation);
        let far = |a: f32| {
            let c = door.rotation() * (self.pivot_closed.rotation * (Quat::from_rotation_y(a) * self.leaf_center));
            (pivot + c).distance(opener)
        };
        let a = OPEN_DEGREES.to_radians();
        self.target = if far(a) >= far(-a) { a } else { -a };
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

/// Swings leaves toward their target angle.
pub fn swing_doors(time: Res<Time>, mut doors: Query<&mut Door>, mut pivots: Query<&mut Transform>) {
    for mut d in &mut doors {
        if d.angle == d.target {
            continue;
        }
        let step = OPEN_DEGREES.to_radians() / d.swing_seconds.max(1e-3) * time.delta_secs();
        let delta = (d.target - d.angle).clamp(-step, step);
        d.angle += delta;
        if let Ok(mut t) = pivots.get_mut(d.pivot) {
            t.rotation = d.pivot_closed.rotation * Quat::from_rotation_y(d.angle);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
            leaf_center: Vec3::X,
            angle: 0.0,
            target: 0.0,
            health: 60.0,
            broken: false,
            swing_seconds: SWING_SECONDS,
            base_health: 60.0,
            damage_thresholds: [0.75, 0.5, 0.25],
            spots: Default::default(),
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
    fn a_barricade_halves_each_hit() {
        let mut d = door(false);
        // 50, 40 (light), 30 (medium: exactly 50%), 20, 10 (heavy).
        let stages: Vec<_> = (0..5).map(|_| hit(&mut d, true)).collect();
        assert_eq!(stages, [(false, None), (false, Some(0)), (false, Some(1)), (false, None), (false, Some(2))]);
        assert_eq!(d.health, 10.0);
        assert!(hit(&mut d, true).0, "the sixth hit");
    }
}
