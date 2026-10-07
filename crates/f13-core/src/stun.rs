//! The stun minigame: a counselor wiggles out of a stun (Jason's grab, a bear trap) as SCCharacter runs it.
//!
//! From the game (the stun's setup and tick on SCCharacter):
//! - Setup: completion, wiggles and elapsed time start at 0; a wiggle is worth PercentPerWiggle / 100; the meter drains
//!   by DepletionRate per second; the stun lasts TotalStunTime (times the character's modifier).
//! - Each tick: the wiggles-per-second counter resets every second (at most MaxWigglesPerSecond, 10, count). The wiggle
//!   axis (A / D) changing sign while not 0 is a wiggle, adding the larger of its worth and MinWiggleDelta (0.01);
//!   otherwise the meter drains by DepletionRate x dt. Completion is the larger of elapsed / stun time and the meter
//!   (capped at 1); the meter carries on from the completion. At 1 she's free.

/// SCCharacter's constructor: MinWiggleDelta, MaxWigglesPerSecond.
pub const MIN_WIGGLE_DELTA: f32 = 0.01;
pub const MAX_WIGGLES_PER_SECOND: u32 = 10;

/// A stun's minigame values (its stat-modified ones already evaluated for the counselor).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WiggleTuning {
    /// TotalStunTime (s): free at the latest after this long.
    pub total_time: f32,
    /// PercentPerWiggle (%).
    pub percent_per_wiggle: f32,
    /// DepletionRate (completion per second).
    pub depletion_rate: f32,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Wiggle {
    /// 0..1; free at 1.
    pub completion: f32,
    pub elapsed: f32,
    /// Wiggles counted this second, and the time into it.
    pub wiggles: u32,
    second: f32,
    /// The axis last tick.
    last_input: f32,
}

impl Wiggle {
    /// One tick with the wiggle axis (-1 A, +1 D, 0 neither). True once she's free.
    pub fn tick(&mut self, dt: f32, input: f32, t: &WiggleTuning) -> bool {
        self.second += dt;
        if 1.0 <= self.second {
            self.wiggles = 0;
            self.second -= 1.0;
        }
        self.elapsed += dt;
        let mut delta = -(dt * t.depletion_rate);
        if input.is_sign_negative() != self.last_input.is_sign_negative() && input != 0.0 && self.wiggles < MAX_WIGGLES_PER_SECOND {
            self.wiggles += 1;
            delta = (t.percent_per_wiggle * 0.01).max(MIN_WIGGLE_DELTA);
        }
        let by_time = if t.total_time <= 0.0 { 0.0 } else { self.elapsed / t.total_time };
        let by_input = delta + self.completion;
        self.completion = if by_time <= by_input { by_input.min(1.0) } else { by_time };
        self.last_input = input;
        1.0 <= self.completion
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TRAP: WiggleTuning = WiggleTuning { total_time: 10.0, percent_per_wiggle: 5.0, depletion_rate: 0.02 };

    #[test]
    fn time_alone_frees_her() {
        let mut w = Wiggle::default();
        let mut t = 0.0;
        while !w.tick(0.1, 0.0, &TRAP) {
            t += 0.1;
        }
        assert!((t - 9.9f32).abs() < 0.11, "{t}");
    }

    #[test]
    fn alternating_wiggles_free_her_sooner_and_cap_at_ten_a_second() {
        let mut w = Wiggle::default();
        let mut t = 0.0;
        let mut i = 0;
        // 60 ticks a second, alternating every tick: only ten wiggles count each second, 0.05 each.
        while !w.tick(1.0 / 60.0, if i % 2 == 0 { 1.0 } else { -1.0 }, &TRAP) {
            t += 1.0 / 60.0;
            i += 1;
        }
        // 0.5 a second from wiggles, less the drain between them: about 2 s.
        assert!((1.6..2.3).contains(&t), "{t}");
    }

    #[test]
    fn holding_a_key_is_one_wiggle() {
        let mut w = Wiggle::default();
        // The axis starts at 0 (a positive sign): D first isn't a change, A is.
        w.tick(0.1, 1.0, &TRAP);
        assert_eq!(w.wiggles, 0);
        w.tick(0.1, -1.0, &TRAP);
        w.tick(0.1, -1.0, &TRAP);
        assert_eq!(w.wiggles, 1);
    }

    #[test]
    fn the_meter_drains_but_never_below_time() {
        let mut w = Wiggle::default();
        w.tick(0.1, -1.0, &TRAP);
        let after = w.completion;
        assert!((after - 0.05).abs() < 1e-6);
        for _ in 0..10 {
            w.tick(0.1, 0.0, &TRAP);
        }
        // 1 s of draining (0.02) vs 1.1 s of time (0.11).
        assert!((w.completion - 0.11).abs() < 1e-4, "{}", w.completion);
    }
}
