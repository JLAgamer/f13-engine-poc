//! Her side of a fight with Jason, as SCCounselorCharacter has it (decompiled; F13-decomp MODLOG "Combat audit"):
//! - **Damage** on him (FUN_1403a3df0): StrengthAttackMod (stat-modified) x her weapon's Damage, x HeavyAttackModifier
//!   for a heavy swing (her perks' damage mods aside).
//! - **Stun** (FUN_1403a3870, FUN_1403cbef0): a roll of RandHelper(101 - StunChanceMod), at most 100 - StunChanceMod
//!   (0 when the mod reaches 101); the hit stuns him when the weapon's StunChance beats it.
//! - **Stamina**: a hit or a stun returns HitKillerStaminaBoost (ReturnStamina); breaking free of his grab,
//!   BreakFreeStaminaBoost (FUN_1403a6c10), both clamped to her stamina maximum.
//! - **His grab** (FUN_1403bc910): her progress per wiggle x WoundedStunMod while she's wounded, x RageGrabStunMod
//!   against a raged Jason (and x his GrabModifier, 1).

use crate::stats::{StatModifiedValue, Stats};

#[derive(Clone, Debug)]
pub struct CombatTuning {
    pub strength_attack_mod: StatModifiedValue,
    pub stun_chance_mod: StatModifiedValue,
    pub hit_killer_stamina_boost: StatModifiedValue,
    pub break_free_stamina_boost: StatModifiedValue,
    pub heavy_attack_modifier: f32,
    pub wounded_stun_mod: f32,
    pub rage_grab_stun_mod: f32,
}

impl Default for CombatTuning {
    /// Neutral (a tuning.json from before these were extracted): flat damage, a 0..100 stun roll, no stamina back.
    fn default() -> Self {
        Self {
            strength_attack_mod: StatModifiedValue::constant(1.0),
            stun_chance_mod: StatModifiedValue::constant(0.0),
            hit_killer_stamina_boost: StatModifiedValue::constant(0.0),
            break_free_stamina_boost: StatModifiedValue::constant(0.0),
            heavy_attack_modifier: 1.0,
            wounded_stun_mod: 1.0,
            rage_grab_stun_mod: 1.0,
        }
    }
}

impl CombatTuning {
    /// Her weapon's damage on him.
    pub fn damage(&self, weapon_damage: f32, stats: &Stats, heavy: bool) -> f32 {
        self.strength_attack_mod.value(stats, false) * weapon_damage * if heavy { self.heavy_attack_modifier } else { 1.0 }
    }

    /// The stun roll, from a uniform `unit` in 0..1 (rand() / 32768).
    pub fn stun_roll(&self, stats: &Stats, unit: f32) -> i32 {
        let m = self.stun_chance_mod.value(stats, false) as i32;
        let n = 101 - m;
        if n <= 0 {
            return 0;
        }
        ((unit * n as f32) as i32).min(100 - m)
    }

    /// Whether her hit stuns him: the weapon's StunChance (an int) beats the roll.
    pub fn stuns(stun_chance: f32, roll: i32) -> bool {
        stun_chance as i32 > roll
    }

    /// Stamina a hit (or a stun) on him gives her back.
    pub fn hit_stamina(&self, stats: &Stats) -> f32 {
        self.hit_killer_stamina_boost.value(stats, false)
    }

    /// Stamina breaking free of his grab gives her back.
    pub fn break_free_stamina(&self, stats: &Stats) -> f32 {
        self.break_free_stamina_boost.value(stats, false)
    }

    /// The grab's wiggle progress modifier.
    pub fn wiggle_mod(&self, wounded: bool, raged: bool) -> f32 {
        (if wounded { self.wounded_stun_mod } else { 1.0 }) * (if raged { self.rage_grab_stun_mod } else { 1.0 })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stats::Stat;

    fn vanessa() -> (CombatTuning, Stats) {
        let mut strength = StatModifiedValue::constant(1.0);
        strength.modifiers[Stat::Strength as usize] = 50.0;
        let mut boost = StatModifiedValue::constant(50.0);
        boost.modifiers[Stat::Luck as usize] = 20.0;
        let t = CombatTuning {
            strength_attack_mod: strength,
            stun_chance_mod: StatModifiedValue::constant(25.0),
            hit_killer_stamina_boost: boost,
            heavy_attack_modifier: 2.0,
            wounded_stun_mod: 0.75,
            rage_grab_stun_mod: 0.5,
            ..CombatTuning::default()
        };
        (t, [4, 10, 2, 9, 6, 3, 1])
    }

    #[test]
    fn strength_scales_damage_and_a_heavy_swing_doubles_it() {
        let (t, stats) = vanessa();
        // Strength 4 of 10 with a +-50% modifier: 1 + (3/9 * 1.0 - 0.5) = 0.8333.
        let light = t.damage(10.0, &stats, false);
        assert!((light - 8.333).abs() < 0.01, "{light}");
        assert!((t.damage(10.0, &stats, true) - 2.0 * light).abs() < 1e-4);
    }

    #[test]
    fn the_stun_roll_runs_to_100_less_the_mod() {
        let (t, stats) = vanessa();
        assert_eq!(t.stun_roll(&stats, 0.0), 0);
        assert_eq!(t.stun_roll(&stats, 0.9999), 75);
        // A bat (StunChance 70) stuns unless the roll is 70..75.
        assert!(CombatTuning::stuns(70.0, t.stun_roll(&stats, 0.9)));
        assert!(!CombatTuning::stuns(70.0, t.stun_roll(&stats, 0.95)));
        // No mod: 0..100, as a flat chance.
        let flat = CombatTuning::default();
        assert_eq!(flat.stun_roll(&stats, 0.9999), 100);
    }

    #[test]
    fn wounded_and_raged_slow_her_struggle() {
        let (t, _) = vanessa();
        assert_eq!(t.wiggle_mod(false, false), 1.0);
        assert!((t.wiggle_mod(true, true) - 0.375).abs() < 1e-6);
    }

    #[test]
    fn luck_raises_the_stamina_a_hit_gives_back() {
        let (t, stats) = vanessa();
        // Luck 6: 50 x (1 + 5/9 * 0.4 - 0.2) = 51.11.
        assert!((t.hit_stamina(&stats) - 51.11).abs() < 0.01, "{}", t.hit_stamina(&stats));
    }
}
