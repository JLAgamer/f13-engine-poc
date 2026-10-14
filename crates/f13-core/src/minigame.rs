//! The hold-to-interact timer and the repair skill-check minigame ("pips").
//!
//! Reimplemented from the character's interact hold tick, `SCInteractComponent`'s hold time and the native
//! `SCInteractMinigameWidget`:
//! - **Hold timer.** While the interact key is held the timer grows by `dt` until it passes
//!   `MinInteractHoldTime`; then the interaction starts (camera, animation, pips). From then on it grows by
//!   `dt * speed`, where `speed = MinigameSpeedMod(stats) * (1 + perks) * bonus` when the interactable has pips (just
//!   `bonus` without pips). The interaction completes when the timer reaches `hold_time + MinInteractHoldTime`.
//!   Progress (0..1) is `(timer - MinInteractHoldTime) / hold_time`. Letting go cancels it.
//! - **Pips** are arcs on a 360-degree dial laid out when the interaction starts: a random count in
//!   `[min_count, max_count]`, widths in `[min_width, max_width]` degrees, positions spread over the first 350
//!   degrees with at least 10 degrees after each pip, and a random key (0 or 1) each. The counts and widths are
//!   adjusted by stat-modified values (count - mod, width + mod).
//! - **The needle** is the progress. A pip becomes active once the needle passes its start; if the needle passes its
//!   end (plus 1.5 degrees) it's a miss.
//! - **Hit:** pressing the pip's key while it's active (and past the last hit). The bonus grows by 0.5 and the next
//!   pip becomes current.
//! - **Miss** (wrong key, too early, too late): the bonus resets to 1, the needle goes back to the end of the
//!   previous pip (or 0), the pip grows 25% wider (up to 10 degrees before the next one) and stays current. After
//!   `AllowableFailsBeforeKillerNotify` misses the killer is notified and the count restarts.

use crate::spawn::Rng;
use crate::stats::{StatModifiedValue, Stats};

/// The dial the pips are laid out on (degrees); the last 10 stay free.
const DIAL_DEGREES: f32 = 350.0;
const PIP_GAP_DEGREES: f32 = 10.0;
/// Grace after a pip's end before it counts as missed (degrees).
const MISS_GRACE_DEGREES: f32 = 1.5;
const HIT_BONUS: f32 = 0.5;
const MISS_WIDEN: f32 = 1.25;

#[derive(Clone, Debug)]
pub struct MinigameTuning {
    pub min_hold_time: f32,
    pub max_pip_count: i32,
    pub min_pip_count: i32,
    pub max_pip_width: f32,
    pub min_pip_width: f32,
    pub max_pip_count_mod: StatModifiedValue,
    pub min_pip_count_mod: StatModifiedValue,
    pub max_pip_width_mod: StatModifiedValue,
    pub min_pip_width_mod: StatModifiedValue,
    /// The counselor's `MinigameSpeedMod` (higher stats fill faster).
    pub speed_mod: StatModifiedValue,
    pub allowable_fails: f32,
}

impl Default for MinigameTuning {
    fn default() -> Self {
        let zero = StatModifiedValue::constant(0.0);
        Self {
            min_hold_time: 0.2,
            max_pip_count: 10,
            min_pip_count: 5,
            max_pip_width: 30.0,
            min_pip_width: 15.0,
            max_pip_count_mod: zero.clone(),
            min_pip_count_mod: zero.clone(),
            max_pip_width_mod: zero.clone(),
            min_pip_width_mod: zero,
            speed_mod: StatModifiedValue::constant(1.0),
            allowable_fails: 3.0,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PipState {
    Waiting,
    Active,
    Hit,
    Missed,
}

impl PipState {
    /// As a number (for the network) and back.
    pub fn index(self) -> u8 {
        self as u8
    }

    pub fn from_index(i: u8) -> Self {
        [Self::Waiting, Self::Active, Self::Hit, Self::Missed][(i as usize).min(3)]
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Pip {
    /// Degrees.
    pub width: f32,
    /// Start, degrees.
    pub position: f32,
    pub state: PipState,
    /// 0 or 1 (PC: right / left mouse button).
    pub key: u8,
}

impl Pip {
    fn start(&self) -> f32 {
        self.position / 360.0
    }

    /// Where the pip ends, as progress, including the grace.
    fn end(&self) -> f32 {
        (self.position + self.width + MISS_GRACE_DEGREES) / 360.0
    }
}

/// Lays out the pips for a counselor with `stats`.
pub fn layout_pips(stats: &Stats, t: &MinigameTuning, rng: &mut Rng) -> Vec<Pip> {
    let min_width = t.min_pip_width + t.min_pip_width_mod.value(stats, false);
    let max_width = t.max_pip_width + t.max_pip_width_mod.value(stats, false);
    let max_count = t.max_pip_count - t.max_pip_count_mod.value(stats, false) as i32;
    let min_count = t.min_pip_count - t.min_pip_count_mod.value(stats, false) as i32;
    let spread = max_count - min_count;
    let mut count = min_count;
    if spread + 1 > 0 {
        count += (((spread + 1) as f32 * rng.frand()) as i32).min(spread);
    }
    let spacing = DIAL_DEGREES / count as f32;
    let mut pips = Vec::new();
    let mut prev_end = 0.0f32;
    let mut gap = min_width;
    let mut i = 0;
    while i < count {
        let lo = (i as f32 * spacing).max(prev_end) + gap;
        let hi = (lo + max_width).min(DIAL_DEGREES - max_width);
        let position = rng.frand() * (hi - lo) + lo;
        let width = rng.frand() * (max_width - min_width) + min_width;
        gap = width + PIP_GAP_DEGREES;
        let key = (rng.next_u32() & 1) as u8;
        pips.push(Pip { width, position, state: PipState::Waiting, key });
        prev_end = position;
        i += 1;
        if position + gap + max_width >= DIAL_DEGREES {
            break;
        }
    }
    pips
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MinigameEvent {
    Started,
    Hit,
    Missed,
    /// Enough misses that the killer hears it.
    NotifyKiller,
    Completed,
}

/// One hold interaction in progress.
#[derive(Clone, Debug)]
pub struct HoldInteraction {
    pub hold_time: f32,
    pub timer: f32,
    pub started: bool,
    pub bonus: f32,
    /// Whether this interactable uses pips at all.
    pub has_pips: bool,
    pub pips: Vec<Pip>,
    pub current: Option<usize>,
    last_hit_end: f32,
    fails: f32,
}

impl HoldInteraction {
    pub fn new(hold_time: f32, has_pips: bool) -> Self {
        Self { hold_time, timer: 0.0, started: false, bonus: 1.0, has_pips, pips: vec![], current: None, last_hit_end: 0.0, fails: 0.0 }
    }

    /// Another machine's (a client's copy of the server's: what its dial shows; the server ticks it).
    pub fn remote(hold_time: f32, timer: f32, started: bool, bonus: f32, has_pips: bool, pips: Vec<Pip>, current: Option<usize>) -> Self {
        Self { hold_time, timer, started, bonus, has_pips, pips, current, last_hit_end: 0.0, fails: 0.0 }
    }

    /// 0..1 once started.
    pub fn progress(&self, t: &MinigameTuning) -> f32 {
        if self.hold_time <= 0.0 {
            return 0.0;
        }
        ((self.timer - t.min_hold_time) / self.hold_time).max(0.0)
    }

    fn set_progress(&mut self, p: f32, t: &MinigameTuning) {
        self.timer = self.hold_time * p + t.min_hold_time;
    }

    /// One tick with the key held. `repair_perk`: her perks' RepairSpeedModifier sum (the interact timer FUN_1402ec8f0:
    /// a minigame with pips goes at (1 + it) x the speed stat mod x the bonus).
    pub fn tick(&mut self, dt: f32, stats: &Stats, repair_perk: f32, t: &MinigameTuning, rng: &mut Rng, events: &mut Vec<MinigameEvent>) {
        let rate = if !self.started {
            1.0
        } else if self.has_pips {
            (1.0 + repair_perk) * t.speed_mod.value(stats, false) * self.bonus
        } else {
            self.bonus
        };
        self.timer += rate * dt;
        if !self.started && self.timer > t.min_hold_time {
            self.started = true;
            if self.has_pips {
                self.pips = layout_pips(stats, t, rng);
                self.current = (!self.pips.is_empty()).then_some(0);
            }
            events.push(MinigameEvent::Started);
        }
        if self.started {
            let progress = self.progress(t);
            if let Some(i) = self.current {
                let pip = self.pips[i];
                if progress > pip.start() && pip.state != PipState::Active {
                    self.pips[i].state = PipState::Active;
                }
                if progress > pip.end() {
                    self.miss(t, events);
                }
            }
        }
        if self.timer >= self.hold_time + t.min_hold_time {
            events.push(MinigameEvent::Completed);
        }
    }

    /// A pip key (0 or 1) was pressed.
    pub fn press(&mut self, key: u8, t: &MinigameTuning, events: &mut Vec<MinigameEvent>) {
        let Some(i) = self.current else { return };
        let progress = self.progress(t);
        if progress < self.last_hit_end {
            return;
        }
        let pip = self.pips[i];
        if progress <= pip.start() || progress > pip.end() || key != pip.key {
            self.miss(t, events);
            return;
        }
        self.last_hit_end = pip.end();
        self.pips[i].state = PipState::Hit;
        self.bonus += HIT_BONUS;
        self.current = (i + 1 < self.pips.len()).then_some(i + 1);
        events.push(MinigameEvent::Hit);
    }

    fn miss(&mut self, t: &MinigameTuning, events: &mut Vec<MinigameEvent>) {
        let Some(i) = self.current else { return };
        self.pips[i].state = PipState::Missed;
        self.bonus = 1.0;
        let back_to = if i == 0 { 0.0 } else { self.pips[i - 1].end() };
        if i == 0 || self.progress(t) > back_to {
            self.set_progress(back_to, t);
        }
        let limit = self.pips.get(i + 1).map_or(DIAL_DEGREES, |n| n.position - PIP_GAP_DEGREES);
        let pip = &mut self.pips[i];
        pip.width *= MISS_WIDEN;
        if pip.position + pip.width > limit {
            pip.width = limit - pip.position;
        }
        pip.state = PipState::Waiting;
        events.push(MinigameEvent::Missed);
        self.fails += 1.0;
        if t.allowable_fails <= self.fails {
            self.fails = 0.0;
            events.push(MinigameEvent::NotifyKiller);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stats::STAT_COUNT;

    fn tuning() -> MinigameTuning {
        MinigameTuning { speed_mod: StatModifiedValue::constant(1.0), ..Default::default() }
    }

    #[test]
    fn layout_fits_the_dial_with_gaps() {
        let t = tuning();
        for seed in 0..50 {
            let pips = layout_pips(&[5; STAT_COUNT], &t, &mut Rng::new(seed));
            assert!(!pips.is_empty() && pips.len() <= 10, "{} pips", pips.len());
            for p in &pips {
                assert!((15.0..=30.0).contains(&p.width) && p.key <= 1);
                assert!(p.position + p.width <= DIAL_DEGREES + 30.0);
            }
            for w in pips.windows(2) {
                assert!(w[1].position >= w[0].position + w[0].width + PIP_GAP_DEGREES - 1e-3, "{w:?}");
            }
        }
    }

    /// Plays up to `limit` seconds, pressing each pip's key in its middle when `perfect`.
    /// Returns (seconds to complete, if it did; hits; killer notifications).
    fn play(perfect: bool, limit: f32) -> (Option<f32>, usize, usize) {
        let t = tuning();
        let mut rng = Rng::new(3);
        let mut h = HoldInteraction::new(15.0, true);
        let (mut time, mut hits, mut notified) = (0.0, 0, 0);
        let mut ev = vec![];
        while time < limit {
            ev.clear();
            h.tick(1.0 / 60.0, &[5; STAT_COUNT], 0.0, &t, &mut rng, &mut ev);
            time += 1.0 / 60.0;
            if let Some(i) = h.current {
                let p = h.pips[i];
                if perfect && h.progress(&t) >= (p.position + p.width / 2.0) / 360.0 {
                    h.press(p.key, &t, &mut ev);
                }
            }
            hits += ev.iter().filter(|e| **e == MinigameEvent::Hit).count();
            notified += ev.iter().filter(|e| **e == MinigameEvent::NotifyKiller).count();
            if ev.contains(&MinigameEvent::Completed) {
                return (Some(time), hits, notified);
            }
        }
        (None, hits, notified)
    }

    #[test]
    fn hitting_every_pip_finishes_faster_than_the_hold_time() {
        let (done, hits, notified) = play(true, 60.0);
        let done = done.expect("completes");
        assert!(hits >= 1 && notified == 0);
        assert!(done < 15.2, "took {done}");
    }

    #[test]
    fn ignoring_the_pips_never_finishes_and_alerts_the_killer() {
        let (done, hits, notified) = play(false, 60.0);
        assert!(done.is_none() && hits == 0 && notified > 0);
    }

    #[test]
    fn a_miss_rolls_back_widens_and_resets_the_bonus() {
        let t = tuning();
        let mut h = HoldInteraction::new(10.0, true);
        h.started = true;
        h.pips = vec![
            Pip { width: 20.0, position: 50.0, state: PipState::Waiting, key: 0 },
            Pip { width: 20.0, position: 150.0, state: PipState::Waiting, key: 1 },
        ];
        h.current = Some(0);
        let mut ev = vec![];
        h.set_progress(60.0 / 360.0, &t);
        h.press(0, &t, &mut ev);
        assert_eq!(ev, vec![MinigameEvent::Hit]);
        assert_eq!(h.bonus, 1.5);
        // wrong key on the second pip
        ev.clear();
        h.set_progress(160.0 / 360.0, &t);
        h.press(0, &t, &mut ev);
        assert_eq!(ev, vec![MinigameEvent::Missed]);
        assert_eq!(h.bonus, 1.0);
        assert!((h.progress(&t) - 71.5 / 360.0).abs() < 1e-5); // end of the first pip
        assert_eq!(h.pips[1].width, 25.0);
        assert_eq!(h.current, Some(1));
    }

    #[test]
    fn misses_notify_the_killer_every_allowable_fails() {
        let t = tuning();
        let mut h = HoldInteraction::new(10.0, true);
        h.started = true;
        h.pips = vec![Pip { width: 20.0, position: 50.0, state: PipState::Waiting, key: 0 }];
        h.current = Some(0);
        let mut ev = vec![];
        for _ in 0..3 {
            h.set_progress(55.0 / 360.0, &t);
            h.press(1, &t, &mut ev);
        }
        assert_eq!(ev.iter().filter(|e| **e == MinigameEvent::NotifyKiller).count(), 1);
    }
}
