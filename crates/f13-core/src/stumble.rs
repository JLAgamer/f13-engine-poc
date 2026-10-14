//! Random stumbles while running or sprinting, and the forced stumble when stamina runs out.
//!
//! Reimplemented from the original counselor (server side):
//! - **When a roll happens.** Only while the counselor can stumble: moving (horizontal speed² ≥ 1e-4), running or
//!   sprinting, standing, not in a special move, not attacking, not grabbed, not leaving a seat and not dropping an
//!   item. Otherwise the attempt clock is pushed back by `dt`, so time spent unable to stumble doesn't count.
//!   While able, one attempt is due every [`StumbleTuning::frequency`] seconds (the clock advances by exactly that,
//!   at most once per tick). An attempt only rolls when not already stumbling and more than `cooldown` seconds have
//!   passed since the last stumble.
//! - **The roll.** `roll = rand * 100 / 32767 + 1e-8`; the counselor stumbles when `roll > 100 - chance`, so
//!   `chance` is a percentage.
//! - **Chance** = `stat_mod(inverted) * (fear_chance(fear / 100) + stamina_chance(1 - stamina / max)
//!   + wounded_chance if wounded)`. Missing curves add nothing. (A perk sum is compared against a random number
//!   before the fear term counts; with no perks the fear term practically always counts.)
//! - **Frequency** = the average of the fear and stamina frequency curves that are set (1 s if neither is).
//! - **The stumble** plays a montage picked by gait (sprint, run, otherwise walk; the out-of-stamina one when
//!   forced) and lasts its length, at least 0.1 s. A zero-length montage cancels it. The last-stumble time is set
//!   when it starts. Running out of stamina while sprinting and standing forces one (if the counselor has
//!   `force_stumble_on_stamina_depletion`), regardless of the cooldown.
//!
//! Times are match seconds (the world clock), so the cooldown also blocks stumbles at the very start of a match.

use crate::counselor::{Counselor, Situation, Stance};
use crate::fear::Curve;
use crate::stats::StatModifiedValue;
use crate::Montage;

const MIN_STUMBLE_SECONDS: f32 = 0.1;
const MOVING_SPEED_SQ: f32 = 1e-4;

#[derive(Clone)]
pub struct StumbleTuning {
    /// Fear 0..1 → seconds between attempts.
    pub fear_frequency: Option<Curve>,
    /// Missing stamina 0..1 → seconds between attempts.
    pub stamina_frequency: Option<Curve>,
    /// Fear 0..1 → chance (percent).
    pub fear_chance: Option<Curve>,
    /// Missing stamina 0..1 → chance (percent).
    pub stamina_chance: Option<Curve>,
    /// Scales the chance; evaluated inverted (higher stats stumble less).
    pub stat_mod: StatModifiedValue,
    /// Added to the chance while wounded (percent).
    pub wounded_chance: f32,
    pub cooldown: f32,
    /// Montages by gait.
    pub sprint: Montage,
    pub run: Montage,
    pub walk: Montage,
    pub out_of_stamina: Montage,
}

impl Default for StumbleTuning {
    /// Never stumbles: no curves and zero-length montages.
    fn default() -> Self {
        Self {
            fear_frequency: None,
            stamina_frequency: None,
            fear_chance: None,
            stamina_chance: None,
            stat_mod: StatModifiedValue::constant(1.0),
            wounded_chance: 0.0,
            cooldown: 0.0,
            sprint: Montage::default(),
            run: Montage::default(),
            walk: Montage::default(),
            out_of_stamina: Montage::default(),
        }
    }
}

impl std::fmt::Debug for StumbleTuning {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StumbleTuning")
            .field("wounded_chance", &self.wounded_chance)
            .field("cooldown", &self.cooldown)
            .field("run", &self.run)
            .finish_non_exhaustive()
    }
}

/// Stumble state kept on the counselor.
#[derive(Clone, Debug, Default)]
pub struct StumbleState {
    /// Match time of the last attempt slot.
    pub last_attempt: f32,
    /// Match time the last stumble started.
    pub last_stumble: f32,
    /// Seconds until the current stumble ends.
    pub time_left: f32,
    /// The montage playing (its root motion moves the counselor).
    pub montage: Montage,
}

impl StumbleTuning {
    fn missing_stamina(c: &Counselor, stamina_max: f32) -> f32 {
        1.0 - c.stamina / stamina_max
    }

    /// Seconds between stumble attempts.
    pub fn frequency(&self, c: &Counselor, fear: f32, stamina_max: f32) -> f32 {
        let mut n = 0u8;
        let mut sum = 0.0f32;
        if let Some(f) = &self.fear_frequency {
            sum = f(fear * 0.01);
            n += 1;
        }
        if let Some(f) = &self.stamina_frequency {
            sum += f(Self::missing_stamina(c, stamina_max));
            n += 1;
        }
        if n == 0 {
            1.0
        } else {
            sum / n as f32
        }
    }

    /// Chance of a stumble per attempt, in percent. `avoid`: EvaluateStumbleChance's rand() (0..1): the fear term
    /// counts unless it's under her perks' AvoidFearStumble.
    pub fn chance(&self, c: &Counselor, fear: f32, stamina_max: f32, avoid: f32) -> f32 {
        let mut chance = 0.0f32;
        if let Some(f) = self.fear_chance.as_ref().filter(|_| c.perks.get("AvoidFearStumble") < avoid) {
            chance = f(fear * 0.01);
        }
        if let Some(f) = &self.stamina_chance {
            chance += f(Self::missing_stamina(c, stamina_max));
        }
        if c.wounded {
            chance += self.wounded_chance;
        }
        self.stat_mod.value(&c.stats, true) * chance
    }

    fn montage(&self, c: &Counselor, out_of_stamina: bool) -> &Montage {
        if out_of_stamina {
            &self.out_of_stamina
        } else if c.sprinting {
            &self.sprint
        } else if c.running {
            &self.run
        } else {
            &self.walk
        }
    }
}

/// A 0..=100 stumble roll from a uniform random number in 0..=1 (the original uses `rand() * 100 / 32767`).
pub fn roll_from_unit(u: f32) -> f32 {
    u * 100.0 + 1e-8
}

impl Counselor {
    /// Whether a random stumble may be attempted this tick.
    pub fn can_stumble(&self, s: &Situation) -> bool {
        s.speed_sq >= MOVING_SPEED_SQ
            && (self.running || self.sprinting)
            && !s.in_special_move
            && self.stance == Stance::Standing
            && !s.attacking
            && !s.grabbed
            && !s.dropping_item
    }

    /// Starts a stumble at match time `now`. Does nothing if already stumbling. Returns whether one started.
    pub fn start_stumble(&mut self, now: f32, out_of_stamina: bool, t: &StumbleTuning) -> bool {
        if self.stumbling {
            return false;
        }
        self.stumble.last_stumble = now;
        let montage = t.montage(self, out_of_stamina);
        if montage.length <= 0.0 {
            return false;
        }
        self.stumbling = true;
        self.stumble.time_left = montage.length.max(MIN_STUMBLE_SECONDS);
        self.stumble.montage = montage.clone();
        true
    }

    /// One server tick of random stumbling at match time `now`. `roll` gives a 0..=100 roll when an attempt is
    /// made (see [`roll_from_unit`]). Returns true when a stumble started this tick.
    pub fn tick_stumble(
        &mut self,
        dt: f32,
        now: f32,
        s: &Situation,
        stamina_max: f32,
        t: &StumbleTuning,
        mut roll: impl FnMut() -> f32,
    ) -> bool {
        if self.stumbling {
            self.stumble.time_left -= dt;
            if self.stumble.time_left <= 0.0 {
                self.stumbling = false;
                self.stumble.time_left = 0.0;
            }
        }
        if !self.can_stumble(s) {
            self.stumble.last_attempt += dt;
            return false;
        }
        let frequency = t.frequency(self, s.fear, stamina_max);
        if frequency >= now - self.stumble.last_attempt {
            return false;
        }
        self.stumble.last_attempt += frequency;
        if self.stumbling || now - self.stumble.last_stumble <= t.cooldown {
            return false;
        }
        // (EvaluateStumbleChance's own rand() first: its perks' fear stumble avoidance.)
        let avoid = (roll() - 1e-8) * 0.01;
        let threshold = 100.0 - t.chance(self, s.fear, stamina_max, avoid);
        if threshold < roll() {
            return self.start_stumble(now, false, t);
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stats::{Stat, STAT_COUNT};
    use std::sync::Arc;

    // Synthetic tuning (not the game's): 50% chance per attempt, one attempt every 2 s, 10 s cooldown.
    fn tuning() -> StumbleTuning {
        StumbleTuning {
            stamina_frequency: Some(Arc::new(|_| 2.0)),
            fear_chance: Some(Arc::new(|fear01| fear01 * 100.0)),
            wounded_chance: 5.0,
            cooldown: 10.0,
            sprint: Montage { name: "sprint".into(), length: 0.8 },
            run: Montage { name: "run".into(), length: 0.6 },
            walk: Montage { name: "walk".into(), length: 0.4 },
            out_of_stamina: Montage { name: "oos".into(), length: 1.0 },
            ..Default::default()
        }
    }

    fn running(fear: f32) -> Situation {
        Situation { speed_sq: 300.0 * 300.0, fear, ..Default::default() }
    }

    fn counselor() -> Counselor {
        let mut c = Counselor::new([5; STAT_COUNT]);
        c.set_running(true);
        c
    }

    #[test]
    fn chance_adds_fear_and_wounded_and_scales_by_inverted_stats() {
        let mut t = tuning();
        let mut c = counselor();
        assert_eq!(t.chance(&c, 50.0, 100.0, 1.0), 50.0);
        c.wounded = true;
        assert_eq!(t.chance(&c, 50.0, 100.0, 1.0), 55.0);
        // Composure 80% swing, inverted: composure 10 → x0.2, composure 1 → x1.8
        t.stat_mod.modifiers[Stat::Composure as usize] = 80.0;
        c.stats[Stat::Composure as usize] = 10;
        assert!((t.chance(&c, 50.0, 100.0, 1.0) - 11.0).abs() < 1e-4);
    }

    #[test]
    fn frequency_averages_the_curves_that_are_set() {
        let mut t = tuning();
        let c = counselor();
        assert_eq!(t.frequency(&c, 0.0, 100.0), 2.0);
        t.fear_frequency = Some(Arc::new(|_| 6.0));
        assert_eq!(t.frequency(&c, 0.0, 100.0), 4.0);
        t.fear_frequency = None;
        t.stamina_frequency = None;
        assert_eq!(t.frequency(&c, 0.0, 100.0), 1.0);
    }

    #[test]
    fn rolls_on_schedule_after_the_cooldown() {
        let t = tuning();
        let mut c = counselor();
        let s = running(50.0); // 50% chance
        let mut rolls = 0;
        let dt = 0.5;
        let mut now = 0.0;
        // Attempts are due every 2 s but the cooldown (10 s from match start) blocks the roll.
        while now < 10.0 {
            now += dt;
            assert!(!c.tick_stumble(dt, now, &s, 100.0, &t, || {
                rolls += 1;
                100.0
            }));
        }
        assert_eq!(rolls, 0);
        // Next due attempt after the cooldown rolls; a high roll stumbles.
        let mut started_at = None;
        while started_at.is_none() {
            now += dt;
            if c.tick_stumble(dt, now, &s, 100.0, &t, || 51.0) {
                started_at = Some(now);
            }
        }
        assert_eq!(started_at, Some(10.5)); // first attempt slot after 10 s
        assert!(c.stumbling);
        assert_eq!(c.stumble.time_left, 0.6); // run montage
        // It ends after its length.
        c.tick_stumble(dt, now + 0.5, &s, 100.0, &t, || 0.0);
        assert!(c.stumbling);
        c.tick_stumble(dt, now + 1.0, &s, 100.0, &t, || 0.0);
        assert!(!c.stumbling);
    }

    #[test]
    fn low_roll_does_not_stumble() {
        let t = tuning();
        let mut c = counselor();
        let s = running(50.0);
        assert!(!c.tick_stumble(0.1, 20.0, &s, 100.0, &t, || 50.0)); // needs > 50
        assert!(!c.stumbling);
    }

    #[test]
    fn time_unable_to_stumble_does_not_count() {
        let t = tuning();
        let mut c = counselor();
        let still = Situation { fear: 100.0, ..Default::default() };
        for i in 1..=100 {
            c.tick_stumble(0.5, i as f32 * 0.5, &still, 100.0, &t, || unreachable!());
        }
        // The attempt clock moved with time, so no attempt is due yet when running resumes.
        assert_eq!(c.stumble.last_attempt, 50.0);
        assert!(!c.tick_stumble(0.5, 50.5, &running(100.0), 100.0, &t, || unreachable!()));
    }

    #[test]
    fn forced_stumble_ignores_cooldown_and_uses_out_of_stamina_montage() {
        let t = tuning();
        let mut c = counselor();
        assert!(c.start_stumble(1.0, true, &t));
        assert_eq!(c.stumble.time_left, 1.0);
        assert!(!c.start_stumble(1.1, true, &t)); // already stumbling
    }

    #[test]
    fn walking_or_crouching_never_stumbles() {
        let mut c = Counselor::new([5; STAT_COUNT]);
        let s = running(100.0);
        assert!(!c.can_stumble(&s));
        c.set_running(true);
        c.stance = Stance::Crouching;
        assert!(!c.can_stumble(&s));
    }
}
