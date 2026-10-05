//! Counselor movement state: stance, walk/run/sprint, stamina and max speed.
//!
//! Behaviour reimplemented from the original counselor tick:
//! - **Stamina** changes every tick on every machine (not only the server), unless locked or attacking.
//!   - Standing still (horizontal speed² ≤ 1e-4), in a special move or stunned: it refills by
//!     `rate * refill * fear_curve(fear) * dt`. While grabbed it instead refills by `grabbed_refill * rate * dt`.
//!   - Moving (and not stumbling): it drains by `rate * deplete_rate * dt`. A negative rate (walking) refills.
//!   - Swimming refills when not moving and only drains while also accelerating.
//!   - `rate` depends on what the counselor is doing (see [`Counselor::stamina_rate`]) and is scaled by the
//!     *inverted* stamina stat modifier, so a higher Stamina stat both drains and refills more slowly.
//!   - At 0 the counselor stops sprinting (falling back to running), may stumble, and is *depleted* until stamina
//!     climbs back to `min_threshold`. Stamina is clamped to 0..=100.
//! - **Sprint input**: while wounded or depleted, pressing sprint only makes the counselor run (unless already
//!   sprinting). Sprinting stands up from a crouch and leaves combat stance. Running is impossible in combat stance.
//! - **Max speed** is the stance's base speed times a modifier (stat, dodging, stumbling, water). Running while
//!   depleted or wounded uses the flat slow-run speed, without the modifier.
//! - **Wounded** is `health <= wounded_threshold` while alive (evaluated on the server).
//!
//! Distances and speeds are in Unreal units (cm, cm/s). Perks are not modelled (their bonuses are 0).

use crate::fear::Curve;
use crate::stats::{StatModifiedValue, Stats};
use crate::stumble::{StumbleState, StumbleTuning};

/// Character stance, in the original enum order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
#[repr(u8)]
pub enum Stance {
    #[default]
    Standing = 0,
    Prone = 1,
    Crouching = 2,
    Swimming = 3,
    Driving = 4,
    Combat = 5,
    Finished = 6,
    Grabbed = 7,
    Trapped = 8,
}

/// Stamina is clamped to this (a constant in the original, independent of `stamina_max`).
pub const STAMINA_CLAMP: f32 = 100.0;
const STILL_SPEED_SQ: f32 = 1e-4;

#[derive(Clone)]
pub struct CounselorTuning {
    // Movement component speeds (cm/s)
    pub max_walk_speed: f32,
    pub max_walk_speed_crouched: f32,
    pub max_slow_run_speed: f32,
    pub max_run_speed: f32,
    pub max_sprint_speed: f32,
    pub max_combat_speed: f32,
    pub max_swim_speed: f32,
    // Speed modifiers
    pub dodging_speed_mod: f32,
    pub stumbling_speed_mod: f32,
    pub water_speed_mod: f32,
    pub speed_stat_mod: StatModifiedValue,
    // Stamina
    pub stamina_max: f32,
    pub stamina_min_threshold: f32,
    pub stamina_deplete_rate: f32,
    pub stamina_refill_rate: StatModifiedValue,
    pub stamina_idle_modifier: f32,
    pub stamina_hiding_modifier: f32,
    pub stamina_walking_modifier: f32,
    pub stamina_running_modifier: f32,
    pub stamina_sprinting_modifier: f32,
    pub stamina_swimming_modifier: f32,
    pub stamina_crouching_modifier: f32,
    pub stamina_stat_mod: StatModifiedValue,
    pub grabbed_stamina_refill: StatModifiedValue,
    /// Fear (0..=100) → refill multiplier; also scales the walking modifier. `None` = 1.
    pub stamina_fear_curve: Option<Curve>,
    pub force_stumble_on_stamina_depletion: bool,
    // Health
    pub wounded_threshold: f32,
    pub stumble: StumbleTuning,
}

impl std::fmt::Debug for CounselorTuning {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CounselorTuning")
            .field("max_walk_speed", &self.max_walk_speed)
            .field("max_run_speed", &self.max_run_speed)
            .field("max_sprint_speed", &self.max_sprint_speed)
            .field("stamina_deplete_rate", &self.stamina_deplete_rate)
            .field("stamina_refill_rate", &self.stamina_refill_rate)
            .field("stamina_fear_curve", &self.stamina_fear_curve.is_some())
            .finish_non_exhaustive()
    }
}

/// What the rest of the game tells the counselor each tick.
#[derive(Clone, Copy, Debug, Default)]
pub struct Situation {
    /// Horizontal velocity squared (cm²/s²).
    pub speed_sq: f32,
    /// Swimming only: horizontal acceleration squared; stamina drains only while accelerating.
    pub accel_sq: f32,
    pub fear: f32,
    pub attacking: bool,
    pub in_special_move: bool,
    pub stunned: bool,
    pub in_hiding: bool,
    pub grabbed: bool,
    pub dodging: bool,
    /// Holding the drop button for a small or large item.
    pub dropping_item: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CounselorEvent {
    /// Ran out of stamina while sprinting. `forced_stumble`: the caller should call
    /// [`Counselor::start_stumble`] with `out_of_stamina` (the original does it in the same tick).
    StaminaDepleted { forced_stumble: bool },
    StaminaRecovered,
}

#[derive(Clone, Debug)]
pub struct Counselor {
    pub stats: Stats,
    pub stance: Stance,
    pub sprinting: bool,
    pub running: bool,
    pub stumbling: bool,
    pub stumble: StumbleState,
    pub stamina: f32,
    pub stamina_depleted: bool,
    pub stamina_locked: bool,
    pub wounded: bool,
}

impl Counselor {
    pub fn new(stats: Stats) -> Self {
        Self {
            stats,
            stance: Stance::Standing,
            sprinting: false,
            running: false,
            stumbling: false,
            stumble: StumbleState::default(),
            stamina: STAMINA_CLAMP,
            stamina_depleted: false,
            stamina_locked: false,
            wounded: false,
        }
    }

    /// Sprint button pressed or released.
    pub fn set_sprint_input(&mut self, pressed: bool) {
        if (self.wounded || self.stamina_depleted) && !self.sprinting {
            self.set_running(pressed);
            return;
        }
        if pressed && self.stance == Stance::Combat {
            self.stance = Stance::Standing;
        }
        self.set_sprinting(pressed);
    }

    pub fn set_sprinting(&mut self, sprinting: bool) {
        if sprinting && self.stance == Stance::Crouching {
            self.stance = Stance::Standing;
        }
        self.sprinting = sprinting;
    }

    pub fn set_running(&mut self, running: bool) {
        self.running = running && self.stance != Stance::Combat;
    }

    /// Server: update the wounded flag from health.
    pub fn update_wounded(&mut self, health: f32, alive: bool, t: &CounselorTuning) {
        self.wounded = alive && health <= t.wounded_threshold;
    }

    /// The stamina drain (positive) or refill (negative while moving / positive while still) multiplier.
    pub fn stamina_rate(&self, s: &Situation, t: &CounselorTuning) -> f32 {
        let fear_curve = |f: f32| t.stamina_fear_curve.as_ref().map_or(1.0, |c| c(f));
        let modifier = if s.speed_sq <= 0.0 || s.in_special_move {
            match self.stance {
                Stance::Crouching => t.stamina_crouching_modifier,
                _ if s.in_hiding => t.stamina_hiding_modifier,
                _ => t.stamina_idle_modifier,
            }
        } else {
            match self.stance {
                Stance::Standing if self.sprinting => t.stamina_sprinting_modifier,
                Stance::Standing if self.running => t.stamina_running_modifier,
                Stance::Standing => fear_curve(s.fear) * t.stamina_walking_modifier,
                Stance::Crouching => -t.stamina_crouching_modifier,
                Stance::Swimming if self.sprinting => t.stamina_swimming_modifier + t.stamina_sprinting_modifier,
                Stance::Swimming => t.stamina_swimming_modifier,
                Stance::Combat => t.stamina_walking_modifier,
                _ => 0.0,
            }
        };
        t.stamina_stat_mod.value(&self.stats, true) * modifier
    }

    /// One tick of stamina. Returns events for the caller (animation, sound, UI).
    pub fn tick_stamina(&mut self, dt: f32, s: &Situation, t: &CounselorTuning, events: &mut Vec<CounselorEvent>) {
        if self.stamina_locked || s.attacking {
            return;
        }
        let refill = t.stamina_refill_rate.value(&self.stats, false);
        let fear_mul = t.stamina_fear_curve.as_ref().map_or(1.0, |c| c(s.fear));
        let regen = |c: &Self| c.stamina_rate(s, t) * refill * fear_mul * dt;
        let drain = |c: &Self| c.stamina_rate(s, t) * t.stamina_deplete_rate * dt;

        if self.stance == Stance::Swimming {
            if s.speed_sq <= STILL_SPEED_SQ {
                self.stamina += regen(self);
            } else if s.accel_sq > STILL_SPEED_SQ {
                self.stamina -= drain(self);
            }
        } else if s.speed_sq <= STILL_SPEED_SQ || s.in_special_move || s.stunned {
            if s.grabbed {
                self.stamina += t.grabbed_stamina_refill.value(&self.stats, false) * self.stamina_rate(s, t) * dt;
            } else {
                self.stamina += regen(self);
            }
        } else if !self.stumbling {
            self.stamina -= drain(self);
        }

        if self.stamina <= 0.0 {
            if self.sprinting {
                let forced_stumble = t.force_stumble_on_stamina_depletion && self.stance == Stance::Standing;
                self.set_sprinting(false);
                if !self.running {
                    self.set_running(true);
                }
                events.push(CounselorEvent::StaminaDepleted { forced_stumble });
            }
            self.stamina_depleted = true;
        }
        if self.stamina_depleted && t.stamina_min_threshold <= self.stamina {
            self.stamina_depleted = false;
            events.push(CounselorEvent::StaminaRecovered);
        }
        self.stamina = self.stamina.clamp(0.0, STAMINA_CLAMP);
    }

    /// Tries to spend stamina for an action (dodge, block...). Fails when depleted or when it would go below -0.5.
    pub fn try_use_stamina(&mut self, amount: f32) -> bool {
        if self.stamina_depleted {
            return false;
        }
        let after = self.stamina - amount;
        if after <= -0.5 {
            return false;
        }
        if !self.stamina_locked {
            self.stamina = after.max(0.0);
        }
        true
    }

    pub fn speed_modifier(&self, s: &Situation, t: &CounselorTuning) -> f32 {
        let mut m = 1.0;
        if s.dodging {
            m = t.dodging_speed_mod;
        }
        if self.stance == Stance::Standing && self.stumbling {
            m = t.stumbling_speed_mod;
        }
        if self.stance == Stance::Swimming {
            m *= t.water_speed_mod;
        }
        t.speed_stat_mod.value(&self.stats, false) * m
    }

    /// Maximum movement speed in cm/s.
    pub fn max_speed(&self, s: &Situation, t: &CounselorTuning) -> f32 {
        let m = self.speed_modifier(s, t);
        if self.sprinting && self.stance != Stance::Crouching {
            return m * t.max_sprint_speed;
        }
        if self.running && self.stance != Stance::Crouching {
            if self.stamina_depleted || self.wounded {
                return t.max_slow_run_speed;
            }
            return m * t.max_run_speed;
        }
        match self.stance {
            Stance::Combat => m * t.max_combat_speed,
            Stance::Crouching => m * t.max_walk_speed_crouched,
            Stance::Swimming => m * t.max_swim_speed,
            _ => m * t.max_walk_speed,
        }
    }
}

/// The "Jason near" fear event's distance input: 2D distance normalised between `min_range` and `max_range`
/// (0 = at `min_range` or closer, 1 = at `max_range`). `None` when Jason is at or beyond `max_range`,
/// in which case the event is inactive.
pub fn jason_near_amount(distance_2d: f32, min_range: f32, max_range: f32) -> Option<f32> {
    if max_range <= distance_2d {
        return None;
    }
    Some(((distance_2d - min_range) / (max_range - min_range)).clamp(0.0, 1.0))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stats::{Stat, STAT_COUNT};

    // Synthetic tuning (not the game's values): round numbers that make the arithmetic easy to check.
    fn tuning() -> CounselorTuning {
        CounselorTuning {
            max_walk_speed: 100.0,
            max_walk_speed_crouched: 80.0,
            max_slow_run_speed: 200.0,
            max_run_speed: 300.0,
            max_sprint_speed: 400.0,
            max_combat_speed: 90.0,
            max_swim_speed: 150.0,
            dodging_speed_mod: 2.0,
            stumbling_speed_mod: 0.5,
            water_speed_mod: 0.5,
            speed_stat_mod: StatModifiedValue::constant(1.0),
            stamina_max: 100.0,
            stamina_min_threshold: 20.0,
            stamina_deplete_rate: 10.0,
            stamina_refill_rate: StatModifiedValue::constant(5.0),
            stamina_idle_modifier: 1.0,
            stamina_hiding_modifier: 2.0,
            stamina_walking_modifier: -0.5,
            stamina_running_modifier: 0.5,
            stamina_sprinting_modifier: 2.0,
            stamina_swimming_modifier: 1.0,
            stamina_crouching_modifier: 1.0,
            stamina_stat_mod: StatModifiedValue::constant(1.0),
            grabbed_stamina_refill: StatModifiedValue::constant(3.0),
            stamina_fear_curve: None,
            force_stumble_on_stamina_depletion: true,
            wounded_threshold: 50.0,
            stumble: Default::default(),
        }
    }

    fn moving() -> Situation {
        Situation { speed_sq: 100.0 * 100.0, ..Default::default() }
    }

    #[test]
    fn sprinting_drains_at_modifier_times_deplete_rate() {
        let t = tuning();
        let mut c = Counselor::new([5; STAT_COUNT]);
        c.set_sprint_input(true);
        let mut ev = vec![];
        c.tick_stamina(1.0, &moving(), &t, &mut ev);
        assert!((c.stamina - 80.0).abs() < 1e-4); // 2.0 * 10 * 1s
    }

    #[test]
    fn standing_still_refills_and_walking_refills_slowly() {
        let t = tuning();
        let mut c = Counselor::new([5; STAT_COUNT]);
        c.stamina = 50.0;
        let mut ev = vec![];
        c.tick_stamina(1.0, &Situation::default(), &t, &mut ev);
        assert!((c.stamina - 55.0).abs() < 1e-4); // idle 1.0 * refill 5
        c.tick_stamina(1.0, &moving(), &t, &mut ev);
        assert!((c.stamina - 60.0).abs() < 1e-4); // -(-0.5 * 10)
    }

    #[test]
    fn running_out_while_sprinting_drops_to_running_and_depletes_until_threshold() {
        let t = tuning();
        let mut c = Counselor::new([5; STAT_COUNT]);
        c.stamina = 5.0;
        c.set_sprint_input(true);
        let mut ev = vec![];
        c.tick_stamina(1.0, &moving(), &t, &mut ev);
        assert_eq!(c.stamina, 0.0);
        assert!(c.stamina_depleted && !c.sprinting && c.running);
        assert_eq!(ev, vec![CounselorEvent::StaminaDepleted { forced_stumble: true }]);
        // While depleted, sprint input only runs, and running uses the flat slow-run speed.
        c.set_sprint_input(true);
        assert!(!c.sprinting && c.running);
        assert_eq!(c.max_speed(&moving(), &t), 200.0);
        // Recovers once stamina reaches the threshold (still: +5/s).
        ev.clear();
        for _ in 0..3 {
            c.tick_stamina(1.0, &Situation::default(), &t, &mut ev);
        }
        assert!(c.stamina_depleted); // 15 < 20
        c.tick_stamina(1.0, &Situation::default(), &t, &mut ev);
        assert!(!c.stamina_depleted);
        assert_eq!(ev, vec![CounselorEvent::StaminaRecovered]);
    }

    #[test]
    fn higher_stamina_stat_slows_drain() {
        let mut t = tuning();
        t.stamina_stat_mod.modifiers[Stat::Stamina as usize] = 20.0;
        let mut lo = Counselor::new([5; STAT_COUNT]);
        lo.stats[Stat::Stamina as usize] = 1;
        let mut hi = lo.clone();
        hi.stats[Stat::Stamina as usize] = 10;
        lo.set_sprint_input(true);
        hi.set_sprint_input(true);
        let mut ev = vec![];
        lo.tick_stamina(1.0, &moving(), &t, &mut ev);
        hi.tick_stamina(1.0, &moving(), &t, &mut ev);
        assert!((100.0 - lo.stamina - 24.0).abs() < 1e-3); // 1.2 * 20
        assert!((100.0 - hi.stamina - 16.0).abs() < 1e-3); // 0.8 * 20
    }

    #[test]
    fn speeds_by_state() {
        let t = tuning();
        let s = Situation::default();
        let mut c = Counselor::new([5; STAT_COUNT]);
        assert_eq!(c.max_speed(&s, &t), 100.0);
        c.stance = Stance::Crouching;
        assert_eq!(c.max_speed(&s, &t), 80.0);
        c.set_sprint_input(true); // stands up
        assert_eq!(c.stance, Stance::Standing);
        assert_eq!(c.max_speed(&s, &t), 400.0);
        c.set_sprint_input(false);
        c.set_running(true);
        assert_eq!(c.max_speed(&s, &t), 300.0);
        c.wounded = true;
        assert_eq!(c.max_speed(&s, &t), 200.0);
        c.stance = Stance::Combat;
        c.set_running(true);
        assert!(!c.running);
        assert_eq!(c.max_speed(&s, &t), 90.0);
    }

    #[test]
    fn wounded_cannot_start_sprinting() {
        let t = tuning();
        let mut c = Counselor::new([5; STAT_COUNT]);
        c.update_wounded(40.0, true, &t);
        c.set_sprint_input(true);
        assert!(!c.sprinting && c.running);
    }

    #[test]
    fn jason_near_normalises_between_ranges() {
        assert_eq!(jason_near_amount(100.0, 300.0, 2000.0), Some(0.0));
        assert_eq!(jason_near_amount(1150.0, 300.0, 2000.0), Some(0.5));
        assert_eq!(jason_near_amount(2000.0, 300.0, 2000.0), None);
    }
}
