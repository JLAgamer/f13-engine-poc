//! Jason's ability unlock, recharge and rage timers.
//!
//! Behaviour reimplemented from observation of the original `ASCKillerCharacter` ability update:
//! - Abilities unlock one at a time in `unlock_order`. The unlock timer restarts at 0 after each unlock,
//!   so `unlock_time[i]` is the *interval* before the i-th unlock, not an absolute time.
//! - `unlock_time` is indexed by unlock position; `recharge_time` is indexed by [`Ability`].
//! - Unlocking an ability fills its charge.
//! - Every unlocked ability that is not available recharges, scaled by `raged_recharge_modifier` while
//!   rage is active, clamped to its recharge time. Blockers (stun, special move, ...) only stop *use*;
//!   they do not pause recharging.
//! - Rage activates once `rage_unlock_time` seconds have accumulated.
//!
//! All arithmetic is `f32`, in the same order as the original, so timings match frame for frame.

/// Killer abilities, in the original enum order (the discriminant indexes `recharge_time`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum Ability {
    Morph = 0,
    Sense = 1,
    Shift = 2,
    Stalk = 3,
}

impl Ability {
    pub const ALL: [Ability; 4] = [Ability::Morph, Ability::Sense, Ability::Shift, Ability::Stalk];

    pub fn index(self) -> usize {
        self as usize
    }
}

/// Per-killer tuning. Load from the player's install; see `KillerTuning::default_order`.
#[derive(Clone, Debug, PartialEq)]
pub struct KillerTuning {
    /// Seconds before each unlock, by unlock position (interval, restarts after each unlock).
    pub unlock_time: [f32; 4],
    /// Full-charge time per ability, indexed by `Ability`.
    pub recharge_time: [f32; 4],
    /// Recharge speed multiplier while raging.
    pub raged_recharge_modifier: f32,
    /// Seconds of match time before rage activates.
    pub rage_unlock_time: f32,
}

impl KillerTuning {
    /// Unlock order used when the player has not chosen one.
    pub const DEFAULT_ORDER: [Ability; 4] = Ability::ALL;
}

/// Things that stop an ability from being *used* this frame (they never pause recharging).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Blockers {
    pub in_special_move: bool,
    pub breaking_down_door_or_wall: bool,
    pub stunned: bool,
    pub underwater: bool,
    pub holding_throwable: bool,
}

/// What happened during an update, for the engine layer to react to (HUD, sounds, replication).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AbilityEvent {
    Unlocked(Ability),
    /// Server: rage is now active.
    RageActivated,
    /// Client reached the rage time; ask the server to activate rage.
    RequestRageFromServer,
}

#[derive(Clone, Debug, PartialEq)]
pub struct KillerAbilities {
    pub tuning: KillerTuning,
    pub unlock_order: [Ability; 4],
    pub unlocked: [bool; 4],
    pub unlock_timer: f32,
    pub recharge_timer: [f32; 4],
    pub rage_active: bool,
    pub rage_timer: f32,
}

impl KillerAbilities {
    pub fn new(tuning: KillerTuning, unlock_order: Option<[Ability; 4]>) -> Self {
        Self {
            tuning,
            unlock_order: unlock_order.unwrap_or(KillerTuning::DEFAULT_ORDER),
            unlocked: [false; 4],
            unlock_timer: 0.0,
            recharge_timer: [0.0; 4],
            rage_active: false,
            rage_timer: 0.0,
        }
    }

    pub fn is_unlocked(&self, a: Ability) -> bool {
        self.unlocked[a.index()]
    }

    /// Unlocked, fully charged and not blocked. Shift alone is blocked underwater.
    pub fn is_available(&self, a: Ability, b: &Blockers) -> bool {
        let i = a.index();
        self.unlocked[i]
            && self.tuning.recharge_time[i] <= self.recharge_timer[i]
            && !b.in_special_move
            && !b.breaking_down_door_or_wall
            && !b.stunned
            && !(a == Ability::Shift && b.underwater)
            && !b.holding_throwable
    }

    /// Charge fraction in 0..=1 (the active-duration display for Sense/Shift/Stalk is engine-side).
    pub fn recharge_fraction(&self, a: Ability) -> f32 {
        let i = a.index();
        (self.recharge_timer[i] / self.tuning.recharge_time[i]).clamp(0.0, 1.0)
    }

    /// Progress toward unlocking `a`: 1 if unlocked, 0 if an earlier ability is still locked,
    /// otherwise the running unlock timer over this slot's interval.
    pub fn unlock_fraction(&self, a: Ability) -> f32 {
        if self.unlocked[a.index()] {
            return 1.0;
        }
        let pos = self.unlock_order.iter().position(|&x| x == a).unwrap_or(4);
        if pos > 0 && !self.unlocked[self.unlock_order[pos - 1].index()] {
            return 0.0;
        }
        self.unlock_timer / self.tuning.unlock_time[pos.min(3)]
    }

    /// Advance all timers by `dt` seconds. `has_authority` = running on the server.
    pub fn update(&mut self, dt: f32, has_authority: bool, b: &Blockers, events: &mut Vec<AbilityEvent>) {
        // Unlock: only the first still-locked ability in the order advances.
        if !self.unlocked[self.unlock_order[3].index()] {
            for (pos, &a) in self.unlock_order.iter().enumerate() {
                if self.unlocked[a.index()] {
                    continue;
                }
                let limit = self.tuning.unlock_time[pos];
                let t = (dt + self.unlock_timer).min(limit);
                self.unlock_timer = t;
                if limit <= t {
                    self.unlocked[a.index()] = true;
                    self.recharge_timer[a.index()] = self.tuning.recharge_time[a.index()];
                    self.unlock_timer = 0.0;
                    events.push(AbilityEvent::Unlocked(a));
                }
                break;
            }
        }

        // Recharge.
        let modifier = if self.rage_active { self.tuning.raged_recharge_modifier } else { 1.0 };
        for &a in &self.unlock_order {
            let i = a.index();
            if self.unlocked[i] && !self.is_available(a, b) {
                self.recharge_timer[i] = (modifier * dt + self.recharge_timer[i]).min(self.tuning.recharge_time[i]);
            }
        }

        // Rage.
        let rage_time = self.tuning.rage_unlock_time;
        if self.rage_timer < rage_time {
            self.rage_timer += dt;
            if rage_time <= self.rage_timer {
                if !has_authority {
                    events.push(AbilityEvent::RequestRageFromServer);
                    return;
                }
                self.rage_timer = rage_time;
                self.rage_active = true;
                events.push(AbilityEvent::RageActivated);
            }
        }
    }

    /// Spend an ability's charge (called when it is activated).
    pub fn consume(&mut self, a: Ability) {
        self.recharge_timer[a.index()] = 0.0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Values as extracted from a stock install (Jason_Base Blueprint defaults + C++ defaults).
    fn tuning() -> KillerTuning {
        KillerTuning {
            unlock_time: [1.0, 45.0, 90.0, 120.0],
            recharge_time: [30.0, 15.0, 30.0, 30.0],
            raged_recharge_modifier: 1.5,
            rage_unlock_time: 900.0,
        }
    }

    /// Step at 60 fps, recording the match time of each event.
    fn run(k: &mut KillerAbilities, seconds: f32, b: Blockers) -> Vec<(f32, AbilityEvent)> {
        let dt = 1.0 / 60.0;
        let mut out = vec![];
        let mut ev = vec![];
        let steps = (seconds / dt).round() as usize;
        for n in 1..=steps {
            k.update(dt, true, &b, &mut ev);
            out.extend(ev.drain(..).map(|e| (n as f32 * dt, e)));
        }
        out
    }

    #[test]
    fn default_unlock_schedule_is_cumulative_intervals() {
        let mut k = KillerAbilities::new(tuning(), None);
        let ev = run(&mut k, 300.0, Blockers::default());
        let unlocks: Vec<_> = ev.iter().filter_map(|(t, e)| match e {
            AbilityEvent::Unlocked(a) => Some((t.round(), *a)),
            _ => None,
        }).collect();
        assert_eq!(unlocks, vec![
            (1.0, Ability::Morph),
            (46.0, Ability::Sense),
            (136.0, Ability::Shift),
            (256.0, Ability::Stalk),
        ]);
    }

    #[test]
    fn custom_order_uses_position_not_ability_for_unlock_time() {
        let order = [Ability::Stalk, Ability::Shift, Ability::Sense, Ability::Morph];
        let mut k = KillerAbilities::new(tuning(), Some(order));
        run(&mut k, 2.0, Blockers::default());
        assert!(k.is_unlocked(Ability::Stalk));
        assert!(!k.is_unlocked(Ability::Morph));
    }

    #[test]
    fn unlock_fills_charge_and_consume_empties_it() {
        let mut k = KillerAbilities::new(tuning(), None);
        run(&mut k, 1.5, Blockers::default());
        assert!(k.is_available(Ability::Morph, &Blockers::default()));
        k.consume(Ability::Morph);
        assert!(!k.is_available(Ability::Morph, &Blockers::default()));
        run(&mut k, 29.0, Blockers::default());
        assert!(!k.is_available(Ability::Morph, &Blockers::default()));
        run(&mut k, 1.1, Blockers::default());
        assert!(k.is_available(Ability::Morph, &Blockers::default()));
    }

    #[test]
    fn blockers_stop_use_but_not_recharge() {
        let mut k = KillerAbilities::new(tuning(), None);
        run(&mut k, 1.5, Blockers::default());
        k.consume(Ability::Morph);
        let stunned = Blockers { stunned: true, ..Default::default() };
        run(&mut k, 31.0, stunned);
        assert_eq!(k.recharge_fraction(Ability::Morph), 1.0);
        assert!(!k.is_available(Ability::Morph, &stunned));
        assert!(k.is_available(Ability::Morph, &Blockers::default()));
    }

    #[test]
    fn only_shift_is_blocked_underwater() {
        let mut k = KillerAbilities::new(tuning(), None);
        run(&mut k, 140.0, Blockers::default());
        let wet = Blockers { underwater: true, ..Default::default() };
        assert!(!k.is_available(Ability::Shift, &wet));
        assert!(k.is_available(Ability::Sense, &wet));
    }

    #[test]
    fn rage_at_fifteen_minutes_speeds_recharge() {
        let mut k = KillerAbilities::new(tuning(), None);
        let ev = run(&mut k, 901.0, Blockers::default());
        let (t, _) = ev.iter().find(|(_, e)| *e == AbilityEvent::RageActivated).unwrap();
        assert_eq!(t.round(), 900.0);
        k.consume(Ability::Morph);
        run(&mut k, 19.5, Blockers::default()); // 19.5 s at x1.5 = 29.25 s of charge: not yet
        assert!(!k.is_available(Ability::Morph, &Blockers::default()));
        // 20 s x1.5 is exactly 30 s; f32 frame accumulation lands just short, so allow one frame more
        run(&mut k, 0.5 + 1.0 / 60.0, Blockers::default());
        assert!(k.is_available(Ability::Morph, &Blockers::default()));
    }

    #[test]
    fn client_requests_rage_instead_of_activating() {
        let mut k = KillerAbilities::new(tuning(), None);
        k.rage_timer = 899.99;
        let mut ev = vec![];
        k.update(0.1, false, &Blockers::default(), &mut ev);
        assert!(ev.contains(&AbilityEvent::RequestRageFromServer));
        assert!(!k.rage_active);
    }
}
