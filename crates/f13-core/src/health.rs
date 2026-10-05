//! Counselor health: damage, healing, wounded and dead.
//!
//! Reimplemented from the counselor's TakeDamage (server):
//! - Ignored when already dead.
//! - **Blocking** with a weapon in hand multiplies the damage by `blocking_damage_mod` (stat-modified, inverted:
//!   higher stats take less). Blocking without a weapon doesn't reduce it.
//! - Perk damage modifiers (1 + Σ) are not modelled.
//! - Health becomes `clamp(health - damage, 0, max_health)`; a counselor is alive while health > 0.
//! - **Wounded** = alive and `health <= wounded_threshold` (updated by the server every tick; see
//!   [`Counselor::update_wounded`]).
//! - **First aid** restores `HealthRestoredPerUse` (clamped to the maximum). Assumption: the heal is a plain add;
//!   the item's use function was not decompiled.
//!
//! Damage from stun damage types (a separate branch scaled by `StunnedDamageMod`) and death side effects
//! (kill credit, dismemberment) belong to combat and are not here yet.

use crate::counselor::{Counselor, CounselorTuning};
use crate::stats::StatModifiedValue;

#[derive(Clone, Debug)]
pub struct HealthTuning {
    pub max_health: f32,
    /// Damage multiplier while blocking with a weapon; evaluated inverted.
    pub blocking_damage_mod: StatModifiedValue,
}

impl Default for HealthTuning {
    fn default() -> Self {
        Self { max_health: 100.0, blocking_damage_mod: StatModifiedValue::constant(1.0) }
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Hit {
    pub amount: f32,
    /// The counselor is blocking with a weapon in hand.
    pub blocked_with_weapon: bool,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DamageResult {
    pub dealt: f32,
    pub became_wounded: bool,
    pub died: bool,
}

impl Counselor {
    pub fn alive(&self) -> bool {
        self.health > 0.0
    }

    /// Applies a hit. Returns what happened; `dealt` is 0 when the hit was ignored.
    pub fn take_damage(&mut self, hit: Hit, t: &CounselorTuning) -> DamageResult {
        let none = DamageResult { dealt: 0.0, became_wounded: false, died: false };
        if !self.alive() {
            return none;
        }
        let block = if hit.blocked_with_weapon { t.health.blocking_damage_mod.value(&self.stats, true) } else { 1.0 };
        let damage = hit.amount * block;
        if damage <= 0.0 {
            return none;
        }
        let was_wounded = self.wounded;
        let before = self.health;
        let after = self.health - damage;
        self.health = if after < 0.0 { 0.0 } else { after.min(t.health.max_health) };
        self.update_wounded(self.health, self.alive(), t);
        DamageResult { dealt: before - self.health, became_wounded: self.wounded && !was_wounded, died: !self.alive() }
    }

    /// Restores health (first aid). Does nothing for the dead. Returns the amount restored.
    pub fn heal(&mut self, amount: f32, t: &CounselorTuning) -> f32 {
        if !self.alive() {
            return 0.0;
        }
        let before = self.health;
        self.health = (self.health + amount).min(t.health.max_health);
        self.update_wounded(self.health, true, t);
        self.health - before
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::counselor::tests::tuning;
    use crate::stats::{Stat, STAT_COUNT};

    #[test]
    fn damage_wounds_then_kills() {
        let t = tuning(); // wounded at 50
        let mut c = Counselor::new([5; STAT_COUNT]);
        let r = c.take_damage(Hit { amount: 40.0, ..Default::default() }, &t);
        assert_eq!(r, DamageResult { dealt: 40.0, became_wounded: false, died: false });
        let r = c.take_damage(Hit { amount: 10.0, ..Default::default() }, &t);
        assert!(r.became_wounded && c.wounded && c.health == 50.0);
        let r = c.take_damage(Hit { amount: 80.0, ..Default::default() }, &t);
        assert_eq!(r.dealt, 50.0);
        assert!(r.died && !c.alive() && !c.wounded); // the dead aren't "wounded"
        assert_eq!(c.take_damage(Hit { amount: 5.0, ..Default::default() }, &t).dealt, 0.0);
        assert_eq!(c.heal(100.0, &t), 0.0);
    }

    #[test]
    fn blocking_with_a_weapon_scales_damage_by_inverted_stats() {
        let mut t = tuning();
        t.health.blocking_damage_mod = StatModifiedValue::constant(0.2);
        t.health.blocking_damage_mod.modifiers[Stat::Strength as usize] = 10.0;
        let mut c = Counselor::new([5; STAT_COUNT]);
        c.stats[Stat::Strength as usize] = 10; // inverted: 0.2 - 0.02
        let r = c.take_damage(Hit { amount: 50.0, blocked_with_weapon: true }, &t);
        assert!((r.dealt - 9.0).abs() < 1e-4);
    }

    #[test]
    fn first_aid_heals_to_max_and_clears_wounded() {
        let t = tuning();
        let mut c = Counselor::new([5; STAT_COUNT]);
        c.take_damage(Hit { amount: 70.0, ..Default::default() }, &t);
        assert!(c.wounded);
        assert_eq!(c.heal(100.0, &t), 70.0);
        assert_eq!(c.health, 100.0);
        assert!(!c.wounded);
    }
}
