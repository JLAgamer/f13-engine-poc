//! Counselor dodges (combat stance).
//!
//! Reimplemented from the counselor's dodge input handlers and `CanDodge`:
//! - Bound on the combat-stance input component as **double taps** of the strafe/back keys (PC: A, D, S within the
//!   engine's 0.2 s `DoubleClickTime`); directions are Left 0, Right 1, Back 2.
//! - Allowed when not already dodging, not attacking, not blocking and not in a special move.
//! - Costs `DodgeStaminaCost` (stat-modified, inverted: higher stats pay less) through `TryUseStamina`, so it fails
//!   while stamina is depleted or would go below -0.5.
//! - The dodge is the anim instance's Dodge{Left,Right,Back}Montage; the counselor is "dodging" while it plays and
//!   its root motion moves them (the anim instance takes root motion from montages).

use crate::counselor::{Counselor, Situation};
use crate::stats::StatModifiedValue;
use crate::Montage;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum DodgeDirection {
    Left = 0,
    Right = 1,
    Back = 2,
}

#[derive(Clone, Debug)]
pub struct DodgeTuning {
    pub stamina_cost: StatModifiedValue,
    pub left: Montage,
    pub right: Montage,
    pub back: Montage,
}

impl Default for DodgeTuning {
    fn default() -> Self {
        Self {
            stamina_cost: StatModifiedValue::constant(0.0),
            left: Montage::default(),
            right: Montage::default(),
            back: Montage::default(),
        }
    }
}

impl DodgeTuning {
    pub fn montage(&self, dir: DodgeDirection) -> &Montage {
        match dir {
            DodgeDirection::Left => &self.left,
            DodgeDirection::Right => &self.right,
            DodgeDirection::Back => &self.back,
        }
    }
}

impl Counselor {
    pub fn dodging(&self) -> bool {
        self.dodge_left > 0.0
    }

    /// `CanDodge`: `blocking` is the counselor's block state (combat isn't modelled here yet).
    pub fn can_dodge(&self, s: &Situation, blocking: bool) -> bool {
        !self.dodging() && !s.attacking && !blocking && !s.in_special_move
    }

    /// Her dodge montage's play rate (FUN_140319cc0: Montage_Play at 1 + her perks' DodgeSpeedModifier).
    pub fn dodge_rate(&self) -> f32 {
        (1.0 + self.perks.get("DodgeSpeedModifier")).max(0.01)
    }

    /// Tries to dodge. On success pays the stamina and returns the montage to play (it lasts its length over
    /// [`Self::dodge_rate`]).
    pub fn try_dodge<'t>(&mut self, dir: DodgeDirection, s: &Situation, blocking: bool, t: &'t DodgeTuning) -> Option<&'t Montage> {
        if !self.can_dodge(s, blocking) {
            return None;
        }
        let montage = t.montage(dir);
        if montage.length <= 0.0 || !self.try_use_stamina(t.stamina_cost.value(&self.stats, true)) {
            return None;
        }
        self.dodge_left = montage.length / self.dodge_rate();
        Some(montage)
    }

    pub fn tick_dodge(&mut self, dt: f32) {
        self.dodge_left = (self.dodge_left - dt).max(0.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stats::{Stat, STAT_COUNT};

    fn tuning() -> DodgeTuning {
        let m = |n: &str| Montage { name: n.into(), length: 0.6 };
        let mut cost = StatModifiedValue::constant(20.0);
        cost.modifiers[Stat::Speed as usize] = 50.0;
        DodgeTuning { stamina_cost: cost, left: m("L"), right: m("R"), back: m("B") }
    }

    #[test]
    fn dodge_speed_perk_shortens_it() {
        let t = tuning();
        let mut c = Counselor::new([5; STAT_COUNT]);
        c.perks.0.insert("DodgeSpeedModifier".into(), 0.2);
        assert!(c.try_dodge(DodgeDirection::Left, &Situation::default(), false, &t).is_some());
        assert!((c.dodge_left - 0.5).abs() < 1e-5);
    }

    #[test]
    fn dodge_costs_stamina_by_inverted_stats_and_lasts_its_montage() {
        let t = tuning();
        let mut c = Counselor::new([5; STAT_COUNT]);
        c.stats[Stat::Speed as usize] = 10; // inverted: 20 - 10
        let s = Situation::default();
        assert_eq!(c.try_dodge(DodgeDirection::Back, &s, false, &t).map(|m| m.name.as_str()), Some("B"));
        assert!((c.stamina - 90.0).abs() < 1e-3);
        assert!(c.dodging());
        assert!(c.try_dodge(DodgeDirection::Left, &s, false, &t).is_none()); // already dodging
        c.tick_dodge(0.6);
        assert!(!c.dodging());
    }

    #[test]
    fn no_dodge_while_blocking_attacking_or_depleted() {
        let t = tuning();
        let mut c = Counselor::new([5; STAT_COUNT]);
        assert!(c.try_dodge(DodgeDirection::Left, &Situation::default(), true, &t).is_none());
        let attacking = Situation { attacking: true, ..Default::default() };
        assert!(c.try_dodge(DodgeDirection::Left, &attacking, false, &t).is_none());
        c.stamina_depleted = true;
        assert!(c.try_dodge(DodgeDirection::Left, &Situation::default(), false, &t).is_none());
        assert_eq!(c.stamina, 100.0);
    }
}
