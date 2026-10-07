//! Jason's Shift, Stalk and Morph, as SCKillerCharacter runs them (Sense is f13-core::perception).
//!
//! From the game:
//! - Shift (SERVER_SetShift, OnRep_ShiftActive, the killer's tick): starting it sets a 0.7 s wind-up; once that has
//!   run out, the shift time counts up and the killer is pushed forward every tick (AddMovementInput along his
//!   facing) with his movement's max speed at ShiftSpeed (times his speed modifier). It ends after ShiftTime, when
//!   he grabs a counselor, or when the player presses Shift again. While it's on he passes through counselors
//!   (IgnoreActorWhenMoving both ways) and the teleport effect fades him out.
//! - Stalk: StalkActiveTimer starts at StalkActiveTime and runs down by the tick's time, scaled by
//!   IdleStalkModifier while he stands still; at 0 it ends. While it's on his music and footsteps are silent.
//! - Morph (the morph map's SetMorphLocation, then SERVER_OnMorph): the killer picks a spot; the nearest
//!   SCKillerMorphPoint inside a 3000 cm sphere around it and within 3000 cm of it horizontally is the destination
//!   (none: no morph). The teleport effect turns on, and TeleportScreenTime (1 s) later he's put on the point,
//!   facing its way. Points near two of the game state's areas (1.5 times their collision radius) are skipped;
//!   those aren't ported.
//! - The teleport effect (the killer's tick): a 0..1 value that runs down at 1/s while the effect is on and back up
//!   at 1/s otherwise; his body and mask materials fade with it.

/// SCKillerCharacter::OnRep_ShiftActive's wind-up (s) before the shift moves him.
pub const SHIFT_WIND_UP: f32 = 0.7;
/// Morph: the sphere searched for morph points around the chosen spot, and their horizontal reach from it (cm).
pub const MORPH_SEARCH_RADIUS: f32 = 3000.0;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Shift {
    /// Wind-up left (s); negative once it has run out.
    pub wind_up: f32,
    /// Time shifting (s), counted once the wind-up is over.
    pub elapsed: f32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShiftPhase {
    WindingUp,
    /// Pushed forward at ShiftSpeed.
    Moving,
    /// ShiftTime is up: the shift ends.
    Over,
}

impl Shift {
    pub fn start() -> Self {
        Shift { wind_up: SHIFT_WIND_UP, elapsed: 0.0 }
    }

    /// One tick, in the game's order. As in the game (it checks `<= 0 && != 0`), a wind-up that lands on exactly 0
    /// never moves him and never times out: such a shift lasts until it's cancelled.
    pub fn tick(&mut self, dt: f32, shift_time: f32) -> ShiftPhase {
        if 0.0 < self.wind_up {
            self.wind_up -= dt;
        }
        if self.wind_up < 0.0 {
            self.elapsed += dt;
            if shift_time < self.elapsed {
                return ShiftPhase::Over;
            }
            return ShiftPhase::Moving;
        }
        ShiftPhase::WindingUp
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Stalk {
    /// StalkActiveTimer (s).
    pub timer: f32,
}

impl Stalk {
    pub fn start(stalk_active_time: f32) -> Self {
        Stalk { timer: stalk_active_time }
    }

    /// One tick; false once it has run out. `moving`: his velocity isn't zero.
    pub fn tick(&mut self, dt: f32, moving: bool, idle_stalk_modifier: f32) -> bool {
        let rate = if moving { 1.0 } else { idle_stalk_modifier };
        self.timer -= rate * dt;
        0.0 < self.timer
    }
}

/// The teleport effect's fade: 1 is fully there, 0 fully faded.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TeleportFade(pub f32);

impl Default for TeleportFade {
    fn default() -> Self {
        TeleportFade(1.0)
    }
}

impl TeleportFade {
    pub fn tick(&mut self, dt: f32, effect_on: bool) {
        self.0 = if effect_on { (self.0 - dt).max(0.0) } else { (self.0 + dt).min(1.0) };
    }
}

/// Morph's destination for a chosen spot (cm, Unreal axes: Z up): the index of the nearest morph point, or None.
pub fn morph_point(points: &[[f32; 3]], spot: [f32; 3]) -> Option<usize> {
    let mut best = None;
    let mut best_d2 = MORPH_SEARCH_RADIUS * MORPH_SEARCH_RADIUS;
    for (i, p) in points.iter().enumerate() {
        let (dx, dy, dz) = (p[0] - spot[0], p[1] - spot[1], p[2] - spot[2]);
        // The overlap sphere, then the horizontal distance against the best so far.
        if MORPH_SEARCH_RADIUS * MORPH_SEARCH_RADIUS < dx * dx + dy * dy + dz * dz {
            continue;
        }
        let d2 = dx * dx + dy * dy;
        if d2 < best_d2 {
            best_d2 = d2;
            best = Some(i);
        }
    }
    best
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shift_winds_up_then_moves_until_shift_time() {
        let mut s = Shift::start();
        let dt = 0.1;
        let mut phases = Vec::new();
        for _ in 0..60 {
            phases.push(s.tick(dt, 5.0));
        }
        let moving_from = phases.iter().position(|p| *p == ShiftPhase::Moving).unwrap();
        // 0.7 s of wind-up: the tick that runs it out already moves (the 7th, or the 8th if float steps leave it
        // a hair above 0).
        assert!((6..=7).contains(&moving_from), "{moving_from}");
        let over_at = phases.iter().position(|p| *p == ShiftPhase::Over).unwrap();
        // 5 s of movement after the wind-up.
        assert!((over_at - moving_from) as f32 * dt > 4.9 && (over_at - moving_from) as f32 * dt < 5.2);
    }

    #[test]
    fn stalk_lasts_longer_standing_still() {
        let mut moving = Stalk::start(20.0);
        let mut t = 0.0;
        while moving.tick(0.5, true, 0.1) {
            t += 0.5;
        }
        assert_eq!(t, 19.5);
        let mut idle = Stalk::start(20.0);
        let mut t = 0.0;
        while idle.tick(1.0, false, 0.1) {
            t += 1.0;
        }
        assert!((198.0..=200.0).contains(&t), "{t}");
    }

    #[test]
    fn fade_runs_down_and_back() {
        let mut f = TeleportFade::default();
        f.tick(0.4, true);
        assert!((f.0 - 0.6).abs() < 1e-6);
        f.tick(2.0, true);
        assert_eq!(f.0, 0.0);
        f.tick(0.5, false);
        assert_eq!(f.0, 0.5);
        f.tick(5.0, false);
        assert_eq!(f.0, 1.0);
    }

    #[test]
    fn morph_picks_the_nearest_point_in_reach() {
        let points = [[2500.0, 0.0, 0.0], [1000.0, 1000.0, 0.0], [0.0, 0.0, 2900.0], [3500.0, 0.0, 0.0]];
        // The point straight above is nearest horizontally (0) and inside the sphere.
        assert_eq!(morph_point(&points, [0.0, 0.0, 0.0]), Some(2));
        // Without it: the nearest of the rest; the one 3500 cm away is out of reach.
        assert_eq!(morph_point(&points[..2], [0.0, 0.0, 0.0]), Some(1));
        assert_eq!(morph_point(&points[3..], [0.0, 0.0, 0.0]), None);
        // Above the sphere: out.
        assert_eq!(morph_point(&[[0.0, 0.0, 3100.0]], [0.0, 0.0, 0.0]), None);
    }
}
