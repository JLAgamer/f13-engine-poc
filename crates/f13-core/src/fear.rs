//! Counselor fear: a 0..=100 value driven by named fear events (Jason near, darkness, wounded, ...).
//!
//! Behaviour reimplemented from observation of the original fear component tick:
//! - Each event has an *active* flag. An event **adds** fear while `active != decrease_when_active`, and
//!   **removes** fear otherwise (so most events add while active, and "hidden"-style events subtract while active).
//! - Adding waits `increase_delay`, then adds `rate * step * increase * curve(distance)` until this event's
//!   `added` total reaches its cap. Removing waits `decrease_delay`, then removes `rate * step * decrease`, never
//!   more than the event has added and never past its cap of removals.
//!   The add check runs before each step, so `added` can overshoot the cap by up to one step while removal stops
//!   exactly at the cap; the overshoot remains as residual fear (kept for fidelity).
//! - When an event is stuck at its cap while active and has `auto_deactivate_time > 0`, it deactivates after that long.
//! - Updates can be throttled: with `update_delay > 0`, fear changes once every `update_delay` seconds using a step
//!   of exactly `update_delay`; the cosmetic (displayed) fear eases toward the real value every frame regardless.
//! - The server clamps fear to 0..=100 and reports when it reaches 100.
//!
//! `rate` is the counselor's fear modifier (stat-modified, plus perks) and the cap is `(1 + perk bonus) * max_fear_cap`;
//! both are supplied by the caller so perks stay outside this module.

use std::sync::Arc;

pub const MAX_FEAR: f32 = 100.0;

/// Distance → multiplier curve (the original uses an engine float curve asset).
pub type Curve = Arc<dyn Fn(f32) -> f32 + Send + Sync>;

#[derive(Clone)]
pub struct FearEventData {
    pub increase: f32,
    pub decrease: f32,
    pub increase_delay: f32,
    pub decrease_delay: f32,
    pub max_fear_cap: f32,
    pub decrease_when_active: bool,
    pub auto_deactivate_time: f32,
    pub distance_curve: Option<Curve>,
}

impl std::fmt::Debug for FearEventData {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FearEventData")
            .field("increase", &self.increase)
            .field("decrease", &self.decrease)
            .field("increase_delay", &self.increase_delay)
            .field("decrease_delay", &self.decrease_delay)
            .field("max_fear_cap", &self.max_fear_cap)
            .field("decrease_when_active", &self.decrease_when_active)
            .field("auto_deactivate_time", &self.auto_deactivate_time)
            .field("distance_curve", &self.distance_curve.is_some())
            .finish()
    }
}

/// Running state of one event on one counselor.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct FearEventState {
    pub added: f32,
    pub removed: f32,
    pub increase_timer: f32,
    pub decrease_timer: f32,
    pub active: bool,
    pub deactivate_timer: f32,
    /// Input for the distance curve (e.g. distance to Jason), set on push.
    pub distance: f32,
}

#[derive(Clone, Debug)]
pub struct FearEntry<K> {
    pub key: K,
    pub data: FearEventData,
    pub state: FearEventState,
}

#[derive(Clone, Debug)]
pub struct Fear<K> {
    pub fear: f32,
    /// Smoothed value for HUD/visuals.
    pub cosmetic: f32,
    pub update_delay: f32,
    pub time_until_update: f32,
    pub locked: bool,
    /// Insertion-ordered (matches the original map's iteration order, which affects f32 rounding).
    pub events: Vec<FearEntry<K>>,
}

/// Per-tick inputs computed by the caller.
pub struct FearContext<'a, K> {
    /// Stat-modified fear modifier plus perk modifiers (FUN_140331060: the base plus her perks'
    /// HidespotFearModifier hiding and CounselorsAroundFearModifier).
    pub rate: f32,
    /// Added for an active event only (FUN_140331060's third argument: her perks' FearModifier).
    pub rate_active: f32,
    /// `(1 + perk bonus for this event) * max_fear_cap`.
    pub cap: &'a dyn Fn(&K, &FearEventData) -> f32,
    pub has_authority: bool,
}

impl<K: PartialEq + Clone> Fear<K> {
    pub fn new(update_delay: f32) -> Self {
        Self { fear: 0.0, cosmetic: 0.0, update_delay, time_until_update: 0.0, locked: false, events: vec![] }
    }

    fn find(&mut self, key: &K) -> Option<&mut FearEntry<K>> {
        self.events.iter_mut().find(|e| &e.key == key)
    }

    /// Activate an event (or refresh its distance). Reactivating restarts its increase and deactivate timers.
    pub fn push(&mut self, key: K, data: &FearEventData, distance: f32) {
        match self.find(&key) {
            None => self.events.push(FearEntry {
                key,
                data: data.clone(),
                state: FearEventState { active: true, distance, ..Default::default() },
            }),
            Some(e) => {
                if !e.state.active {
                    e.state.active = true;
                    e.state.increase_timer = 0.0;
                    e.state.deactivate_timer = 0.0;
                }
                e.state.distance = distance;
            }
        }
    }

    /// Deactivate an event. Unknown events are registered as inactive.
    pub fn pop(&mut self, key: K, data: &FearEventData) {
        match self.find(&key) {
            None => self.events.push(FearEntry { key, data: data.clone(), state: FearEventState::default() }),
            Some(e) => {
                if e.state.active {
                    e.state.active = false;
                    e.state.decrease_timer = 0.0;
                }
            }
        }
    }

    /// Server-side lock (cutscenes, scripted moments): sets and freezes fear.
    pub fn lock_to(&mut self, value: f32, locked: bool) {
        let v = value.clamp(0.0, MAX_FEAR);
        self.fear = v;
        self.cosmetic = v;
        self.locked = locked;
    }

    /// Advance by `dt`. Returns true when the server just clamped fear at 100.
    pub fn tick(&mut self, dt: f32, ctx: &FearContext<K>) -> bool {
        if self.locked {
            return false;
        }
        let mut step = dt;
        if 0.0 < self.update_delay {
            self.time_until_update -= dt;
            if 0.0 < self.time_until_update {
                self.cosmetic = finterp_to(self.cosmetic, self.fear, dt, 1.0);
                return false;
            }
            self.time_until_update = self.update_delay;
            step = self.update_delay;
        }

        for e in &mut self.events {
            let cap = (ctx.cap)(&e.key, &e.data);
            let curve = e.data.distance_curve.as_ref().map_or(1.0, |c| c(e.state.distance));
            let s = &mut e.state;
            let d = &e.data;
            let mut stuck = false;
            if s.active == d.decrease_when_active {
                if 0.0 < self.fear {
                    s.decrease_timer += step;
                    if d.decrease_delay <= s.decrease_timer {
                        s.decrease_timer = d.decrease_delay;
                        if cap <= s.removed || s.added <= 0.0 {
                            stuck = true;
                        } else {
                            let r = (ctx.rate + if s.active { ctx.rate_active } else { 0.0 }) * step * d.decrease;
                            s.added = (s.added - r).max(0.0);
                            s.removed += r;
                            self.fear -= r;
                            continue;
                        }
                    }
                } else {
                    continue;
                }
            } else if self.fear < MAX_FEAR {
                let t = step + s.increase_timer;
                s.increase_timer = t;
                if d.increase_delay <= t {
                    s.increase_timer = d.increase_delay;
                    if s.added < cap {
                        let r = (ctx.rate + if s.active { ctx.rate_active } else { 0.0 }) * step * d.increase * curve;
                        s.removed = (s.removed - r).max(0.0);
                        s.added += r;
                        self.fear += r;
                        continue;
                    }
                    stuck = true;
                }
            } else {
                continue;
            }
            if s.active && 0.0 < d.auto_deactivate_time && stuck {
                s.deactivate_timer += step;
                s.active = s.deactivate_timer < d.auto_deactivate_time;
            }
        }

        let mut maxed = false;
        if ctx.has_authority {
            self.fear = self.fear.clamp(0.0, MAX_FEAR);
            maxed = MAX_FEAR <= self.fear;
        }
        self.cosmetic = finterp_to(self.cosmetic, self.fear, dt, 1.0);
        maxed
    }
}

/// Unreal's `FMath::FInterpTo`.
pub fn finterp_to(current: f32, target: f32, dt: f32, speed: f32) -> f32 {
    if speed <= 0.0 {
        return target;
    }
    let dist = target - current;
    if dist * dist < 1.0e-8 {
        return target;
    }
    current + dist * (dt * speed).clamp(0.0, 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn jason_spotted() -> FearEventData {
        FearEventData {
            increase: 10.0,
            decrease: 1.0,
            increase_delay: 0.0,
            decrease_delay: 15.0,
            max_fear_cap: 30.0,
            decrease_when_active: false,
            auto_deactivate_time: 0.0,
            distance_curve: None,
        }
    }

    fn hidden() -> FearEventData {
        FearEventData {
            increase: 0.5,
            decrease: 10.0,
            increase_delay: 1.0,
            decrease_delay: 1.0,
            max_fear_cap: 10.0,
            decrease_when_active: true,
            auto_deactivate_time: 0.0,
            distance_curve: None,
        }
    }

    fn run(f: &mut Fear<&'static str>, seconds: f32, rate: f32) -> bool {
        let cap = |_: &&str, d: &FearEventData| d.max_fear_cap;
        let ctx = FearContext { rate, rate_active: 0.0, cap: &cap, has_authority: true };
        let dt = 1.0 / 60.0;
        let mut maxed = false;
        for _ in 0..(seconds / dt).round() as usize {
            maxed |= f.tick(dt, &ctx);
        }
        maxed
    }

    #[test]
    fn active_event_adds_until_its_cap() {
        let mut f = Fear::new(0.0);
        f.push("spotted", &jason_spotted(), 0.0);
        run(&mut f, 1.0, 1.0);
        assert!((f.fear - 10.0).abs() < 0.2, "10/s for 1 s, got {}", f.fear);
        run(&mut f, 10.0, 1.0);
        assert!((f.fear - 30.0).abs() < 0.2, "capped at 30, got {}", f.fear);
    }

    #[test]
    fn composure_rate_scales_gain() {
        let mut f = Fear::new(0.0);
        f.push("spotted", &jason_spotted(), 0.0);
        run(&mut f, 1.0, 0.2);
        assert!((f.fear - 2.0).abs() < 0.1, "x0.2 rate, got {}", f.fear);
    }

    #[test]
    fn inactive_event_bleeds_off_only_after_delay_and_only_what_it_added() {
        let mut f = Fear::new(0.0);
        let d = jason_spotted();
        f.push("spotted", &d, 0.0);
        run(&mut f, 5.0, 1.0);
        let peak = f.fear;
        f.pop("spotted", &d);
        run(&mut f, 14.0, 1.0);
        assert_eq!(f.fear, peak, "no decrease before the 15 s delay");
        run(&mut f, 60.0, 1.0);
        // Faithful quirk: adding checks `added < cap` before each step, so the last step overshoots the cap,
        // while removal stops exactly at the cap. The overshoot (less than one step) stays as residual fear.
        let e = &f.events[0].state;
        assert_eq!(e.removed.max(30.0), e.removed, "removal stops at the cap");
        assert!(f.fear > 0.0 && f.fear < 10.0 / 60.0 + 1e-4, "residual < one step, got {}", f.fear);
    }

    #[test]
    fn hiding_removes_fear_while_active() {
        let mut f = Fear::new(0.0);
        f.push("spotted", &jason_spotted(), 0.0);
        run(&mut f, 2.0, 1.0);
        f.pop("spotted", &jason_spotted());
        // "hidden" removes while active, but only fear it has itself added: on its own it removes nothing
        f.push("hidden", &hidden(), 0.0);
        let before = f.fear;
        run(&mut f, 5.0, 1.0);
        assert_eq!(f.fear, before);
    }

    #[test]
    fn throttled_update_steps_by_the_full_delay() {
        let mut f = Fear::new(0.5);
        f.push("spotted", &jason_spotted(), 0.0);
        let cap = |_: &&str, d: &FearEventData| d.max_fear_cap;
        let ctx = FearContext { rate: 1.0, rate_active: 0.0, cap: &cap, has_authority: true };
        f.tick(0.016, &ctx); // first tick: timer starts at 0 so it updates now with step 0.5
        assert!((f.fear - 5.0).abs() < 1e-5);
        f.tick(0.016, &ctx);
        assert!((f.fear - 5.0).abs() < 1e-5, "no change until the delay elapses");
        assert!(f.cosmetic > 0.0 && f.cosmetic < f.fear);
    }

    #[test]
    fn server_clamps_and_reports_max_fear() {
        let mut f = Fear::new(0.0);
        let mut big = jason_spotted();
        big.max_fear_cap = 1000.0;
        big.increase = 500.0;
        f.push("big", &big, 0.0);
        assert!(run(&mut f, 1.0, 1.0));
        assert_eq!(f.fear, MAX_FEAR);
    }

    #[test]
    fn lock_freezes_fear() {
        let mut f = Fear::new(0.0);
        f.push("spotted", &jason_spotted(), 0.0);
        f.lock_to(42.0, true);
        run(&mut f, 3.0, 1.0);
        assert_eq!(f.fear, 42.0);
    }

    #[test]
    fn finterp_matches_unreal() {
        assert_eq!(finterp_to(0.0, 10.0, 0.5, 1.0), 5.0);
        assert_eq!(finterp_to(0.0, 10.0, 2.0, 1.0), 10.0);
        assert_eq!(finterp_to(3.0, 10.0, 0.1, 0.0), 10.0);
    }
}
