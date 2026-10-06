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
    /// locks, bars or blocks (the leaf is hidden by the caller).
    pub fn damage(&mut self, damage: f32, barred: bool) -> bool {
        if self.broken {
            return false;
        }
        // Health is an int: the new value truncates.
        self.health = (self.health - Door::hit_damage(damage, barred)).trunc().max(0.0);
        if self.health > 0.0 {
            return false;
        }
        self.broken = true;
        self.open = true;
        self.locked = false;
        true
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
        }
    }

    #[test]
    fn a_locked_door_breaks_on_the_third_pickaxe_hit() {
        let mut d = door(true);
        assert!(!d.damage(20.0, false));
        assert!(!d.damage(20.0, false));
        assert!(d.locked && !d.open);
        assert!(d.damage(20.0, false));
        assert!(d.broken && d.open && !d.locked);
        assert!(!d.damage(20.0, false), "already broken");
    }

    #[test]
    fn a_barricade_halves_each_hit() {
        let mut d = door(false);
        for _ in 0..5 {
            assert!(!d.damage(20.0, true));
        }
        assert_eq!(d.health, 10.0);
        assert!(d.damage(20.0, true), "the sixth hit");
    }
}
