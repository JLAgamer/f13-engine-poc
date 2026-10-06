//! What the killer can know about a counselor: her noise, the sound blips it puts on his screen, and Sense.
//!
//! Reimplemented from the original game code:
//! - `SCCounselorCharacter::GetNoiseLevel`: 0 when she isn't moving; otherwise her stance's noise (standing or
//!   combat: walking, running or sprinting; crouching; swimming; nothing in other stances), times her
//!   `NoiseStatMod` evaluated inverted (stealthier counselors are quieter). Perk bonuses and holding her breath are
//!   not ported.
//! - The killer's blip update (every tick, per counselor): once her next-blip time has come, a counselor at least
//!   `MinBlipRadius` away whose `min(noise x (rage ? 0.5 : 1), 17) x (1 - min(distance / NoiseDistanceThreshold, 1))`
//!   is above `NoiseLevelThreshold` puts a blip at her position (the SoundBlip actor, alive `InitialLifeSpan`
//!   seconds); her next blip comes `rand[MinBlipFrequencyTime, MaxBlipFrequencyTime]` seconds later.
//! - Sense: a counselor is revealed within `SenseFearRadiusCurve(fear) x RageSenseDistanceCurve(rage timer /
//!   RageUnlockTime) x (rage ? RagedSenseDistanceModifier : 1)` (2D distance). A hiding counselor only while
//!   raging, within `SenseFearHidingRadiusCurve(fear) x RageSenseDistanceCurve(..) x RagedSenseDistanceModifier`.
//!   Indoors the game lights up her cabin instead of her.

use crate::counselor::Stance;
use crate::curve::RichCurve;
use crate::stats::{StatModifiedValue, Stats};

/// The killer's blip level cap and the factor rage applies to noise (constants in the blip update).
pub const MAX_NOISE_LEVEL: f32 = 17.0;
pub const RAGED_NOISE_SCALE: f32 = 0.5;

/// A counselor's noise values (`SCCounselorCharacter`).
#[derive(Clone, Debug, PartialEq)]
pub struct NoiseTuning {
    pub crouching: f32,
    pub walking: f32,
    pub running: f32,
    pub sprinting: f32,
    pub swimming: f32,
    /// Evaluated inverted.
    pub stat_mod: StatModifiedValue,
}

/// `GetNoiseLevel` for a counselor moving (`moving`) in `stance`.
pub fn noise_level(t: &NoiseTuning, stats: &Stats, stance: Stance, sprinting: bool, running: bool, moving: bool) -> f32 {
    if !moving {
        return 0.0;
    }
    let base = match stance {
        Stance::Standing | Stance::Combat => {
            if sprinting {
                t.sprinting
            } else if running {
                t.running
            } else {
                t.walking
            }
        }
        Stance::Crouching => t.crouching,
        Stance::Swimming => t.swimming,
        _ => 0.0,
    };
    t.stat_mod.value(stats, true) * base
}

/// The killer's hearing (`SCKillerCharacter`).
#[derive(Clone, Debug, PartialEq)]
pub struct HearingTuning {
    /// cm: no blips for counselors closer than this.
    pub min_blip_radius: f32,
    pub noise_level_threshold: f32,
    /// cm: the distance at which noise fades to nothing.
    pub noise_distance_threshold: f32,
    pub min_blip_frequency: f32,
    pub max_blip_frequency: f32,
    /// Seconds a blip stays on his screen.
    pub blip_life: f32,
}

/// The blip level of a counselor making `noise` at `distance` cm, or None when she's too close for blips.
pub fn blip_level(t: &HearingTuning, noise: f32, distance: f32, raged: bool) -> Option<f32> {
    if distance < t.min_blip_radius {
        return None;
    }
    let fade = (distance / t.noise_distance_threshold).min(1.0);
    let scale = if raged { RAGED_NOISE_SCALE } else { 1.0 };
    Some((scale * noise).min(MAX_NOISE_LEVEL) * (1.0 - fade))
}

/// Whether a counselor whose next blip is due puts one on the killer's screen.
pub fn makes_blip(t: &HearingTuning, noise: f32, distance: f32, raged: bool) -> bool {
    blip_level(t, noise, distance, raged).is_some_and(|l| l > t.noise_level_threshold)
}

/// Seconds until a counselor's next blip, for a uniform random `r` in [0, 1).
pub fn next_blip_delay(t: &HearingTuning, r: f32) -> f32 {
    r * (t.max_blip_frequency - t.min_blip_frequency) + t.min_blip_frequency
}

/// Sense (`SCKillerCharacter`).
#[derive(Clone, Debug, PartialEq)]
pub struct SenseTuning {
    /// Seconds Sense stays on.
    pub duration: f32,
    pub fear_radius: RichCurve,
    pub fear_hiding_radius: RichCurve,
    pub rage_distance: RichCurve,
    pub raged_distance_modifier: f32,
    pub rage_unlock_time: f32,
}

/// How far (cm, 2D) Sense reaches a counselor with `fear` (0..100): 0 for a hiding counselor unless he's raging.
/// `rage_timer` is his rage unlock timer (s).
pub fn sense_radius(t: &SenseTuning, fear: f32, hiding: bool, raged: bool, rage_timer: f32) -> f32 {
    let rage = t.rage_distance.eval(rage_timer / t.rage_unlock_time);
    if hiding {
        if raged {
            t.fear_hiding_radius.eval(fear) * rage * t.raged_distance_modifier
        } else {
            0.0
        }
    } else {
        t.fear_radius.eval(fear) * rage * if raged { t.raged_distance_modifier } else { 1.0 }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::curve::{Interp, Key};
    use crate::stats::Stat;

    fn noise() -> NoiseTuning {
        let mut stat_mod = StatModifiedValue::constant(1.0);
        stat_mod.modifiers[Stat::Luck as usize] = 10.0;
        stat_mod.modifiers[Stat::Composure as usize] = 15.0;
        stat_mod.modifiers[Stat::Stealth as usize] = 50.0;
        NoiseTuning { crouching: 1.0, walking: 2.0, running: 3.0, sprinting: 4.0, swimming: 10.0, stat_mod }
    }

    fn hearing() -> HearingTuning {
        HearingTuning {
            min_blip_radius: 1000.0,
            noise_level_threshold: 2.0,
            noise_distance_threshold: 50000.0,
            min_blip_frequency: 1.0,
            max_blip_frequency: 3.0,
            blip_life: 6.0,
        }
    }

    fn line(a: (f32, f32), b: (f32, f32)) -> RichCurve {
        let k = |(time, value): (f32, f32)| Key { time, value, interp: Interp::Linear, arrive_tangent: 0.0, leave_tangent: 0.0 };
        RichCurve { keys: vec![k(a), k(b)] }
    }

    #[test]
    fn standing_still_is_silent() {
        assert_eq!(noise_level(&noise(), &[5; 7], Stance::Standing, true, true, false), 0.0);
    }

    #[test]
    fn stealth_lowers_noise() {
        let mut quiet = [5; 7];
        quiet[Stat::Stealth as usize] = 10;
        let mut loud = quiet;
        loud[Stat::Stealth as usize] = 1;
        let t = noise();
        assert!(noise_level(&t, &quiet, Stance::Standing, true, false, true) < noise_level(&t, &loud, Stance::Standing, true, false, true));
        // All stats at the middle-ish 5: near the base noise.
        let mid = noise_level(&t, &[5; 7], Stance::Standing, false, false, true);
        assert!((mid - 2.0).abs() < 0.3, "{mid}");
        assert_eq!(noise_level(&t, &[5; 7], Stance::Driving, false, false, true), 0.0);
    }

    #[test]
    fn sprinting_is_heard_far_walking_is_not() {
        let t = hearing();
        // Sprinting (4): above the threshold (2) out to half of NoiseDistanceThreshold.
        assert!(makes_blip(&t, 4.0, 20000.0, false));
        assert!(!makes_blip(&t, 4.0, 26000.0, false));
        // Walking at base noise (2) never beats the threshold.
        assert!(!makes_blip(&t, 2.0, 1000.0, false));
        // Too close: no blip at all (he can see her).
        assert!(!makes_blip(&t, 10.0, 999.0, false));
        // Rage halves the noise he hears.
        assert!(!makes_blip(&t, 4.0, 1000.0, true));
    }

    #[test]
    fn sense_reaches_hiders_only_in_rage() {
        let t = SenseTuning {
            duration: 15.0,
            fear_radius: line((0.0, 5000.0), (100.0, 11000.0)),
            fear_hiding_radius: line((0.0, 5000.0), (100.0, 10000.0)),
            rage_distance: line((0.0, 0.5), (1.0, 1.0)),
            raged_distance_modifier: 2.0,
            rage_unlock_time: 900.0,
        };
        // Match start, no fear: half of 50 m.
        assert!((sense_radius(&t, 0.0, false, false, 0.0) - 2500.0).abs() < 1e-3);
        assert_eq!(sense_radius(&t, 100.0, true, false, 450.0), 0.0);
        assert!((sense_radius(&t, 100.0, true, true, 900.0) - 20000.0).abs() < 1e-2);
    }
}
