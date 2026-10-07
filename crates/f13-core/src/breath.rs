//! Holding her breath while hiding, as SCCounselorCharacter runs it (UpdateBreath, OnHoldBreath, OnReleaseBreath,
//! OnRep_IsHoldingBreath).
//!
//! - Not holding: breath refills by BreathRechargeModifier per second (stat-modified) up to MaxBreath, and the
//!   LastBreathTimer runs down; once she can't hold it (after letting go), she can again when her breath is above
//!   MinHoldBreathThreshold and the timer has run out.
//! - Holding (only when she can): breath drains by BreathHoldModifier per second (stat-modified, inverted: more
//!   Stealth holds longer); at 0 she lets go.
//! - Letting go (while hiding) starts the LastBreathTimer at MinHoldBreathTimer and she can't hold it until it's over.

use crate::stats::{StatModifiedValue, Stats};

#[derive(Clone, Debug, PartialEq)]
pub struct BreathTuning {
    pub max: f32,
    pub min_hold_threshold: f32,
    pub min_hold_timer: f32,
    pub hold: StatModifiedValue,
    pub recharge: StatModifiedValue,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Breath {
    pub current: f32,
    pub holding: bool,
    pub can_hold: bool,
    last_timer: f32,
}

impl Breath {
    pub fn new(t: &BreathTuning) -> Self {
        Breath { current: t.max, holding: false, can_hold: true, last_timer: 0.0 }
    }

    /// OnHoldBreath: she starts holding it, if she can. True if she did.
    pub fn hold(&mut self) -> bool {
        if self.can_hold && !self.holding {
            self.holding = true;
            return true;
        }
        false
    }

    /// OnReleaseBreath / running out: she lets go (and must wait MinHoldBreathTimer). True if she was holding.
    pub fn release(&mut self, t: &BreathTuning) -> bool {
        if !self.holding {
            return false;
        }
        self.holding = false;
        self.last_timer = t.min_hold_timer;
        self.can_hold = false;
        true
    }

    /// UpdateBreath. Returns true when she ran out and let go this tick.
    pub fn tick(&mut self, dt: f32, stats: &Stats, t: &BreathTuning) -> bool {
        if self.holding {
            if self.current <= 0.0 {
                self.release(t);
                return true;
            }
            self.current = (self.current - t.hold.value(stats, true) * dt).max(0.0);
            return false;
        }
        if self.current < t.max || !self.can_hold {
            self.current = (self.current + t.recharge.value(stats, false) * dt).min(t.max);
            if 0.0 < self.last_timer {
                self.last_timer -= dt;
            }
            if !self.can_hold {
                self.can_hold = t.min_hold_threshold < self.current && self.last_timer <= 0.0;
            }
        }
        false
    }

    pub fn fraction(&self, t: &BreathTuning) -> f32 {
        (self.current / t.max.max(1e-3)).clamp(0.0, 1.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tuning() -> BreathTuning {
        BreathTuning {
            max: 100.0,
            min_hold_threshold: 50.0,
            min_hold_timer: 1.5,
            hold: StatModifiedValue::constant(10.0),
            recharge: StatModifiedValue::constant(10.0),
        }
    }

    #[test]
    fn holding_drains_then_lets_go_and_waits() {
        let t = tuning();
        let stats = [5; 7];
        let mut b = Breath::new(&t);
        assert!(b.hold());
        let mut ran_out = false;
        for _ in 0..110 {
            ran_out |= b.tick(0.1, &stats, &t);
        }
        // 100 at 10/s: out after 10 s.
        assert!(ran_out && !b.holding && !b.can_hold);
        assert!(!b.hold());
        // It refills (the loop's last second already gave 10); she can hold it again once above 50 and the 1.5 s are
        // up: about 4 s more.
        let mut t_ok = 0.0;
        while !b.can_hold {
            b.tick(0.1, &stats, &t);
            t_ok += 0.1;
        }
        assert!((4.0..4.4).contains(&t_ok), "{t_ok}");
    }

    #[test]
    fn letting_go_early_waits_the_timer() {
        let t = tuning();
        let mut b = Breath::new(&t);
        b.hold();
        b.tick(1.0, &[5; 7], &t);
        assert!(b.release(&t));
        assert!(!b.can_hold);
        b.tick(1.0, &[5; 7], &t);
        assert!(!b.can_hold);
        b.tick(1.0, &[5; 7], &t);
        assert!(b.can_hold);
    }
}
