//! Counselor stats and stat-modified values.
//!
//! A stat-modified value is `base + Σ base * lerp(-m_i, +m_i, (stat_i - 1) / 9)` with `m_i = scale * modifier_i`
//! and `scale = +0.01`, or `-0.01` for "inverted" values where a higher stat should lower the result (fear).
//! Stats run 1..=10, so stat 1 contributes `-m_i` and stat 10 contributes `+m_i`.

/// Counselor stats, in the original enum order (the discriminant indexes stat arrays).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum Stat {
    Strength = 0,
    Speed = 1,
    Intelligence = 2,
    Stamina = 3,
    Luck = 4,
    Composure = 5,
    Stealth = 6,
}

pub const STAT_COUNT: usize = 7;

/// Per-counselor stat levels, 1..=10, indexed by [`Stat`].
pub type Stats = [u8; STAT_COUNT];

#[derive(Clone, Debug, PartialEq)]
pub struct StatModifiedValue {
    pub base: f32,
    /// Percent swing per stat, indexed by [`Stat`].
    pub modifiers: [f32; STAT_COUNT],
}

impl StatModifiedValue {
    pub fn constant(base: f32) -> Self {
        Self { base, modifiers: [0.0; STAT_COUNT] }
    }

    /// Evaluate for a counselor. `inverted` = higher stat lowers the value.
    pub fn value(&self, stats: &Stats, inverted: bool) -> f32 {
        let scale = if inverted { -0.01f32 } else { 0.01f32 };
        let mut v = self.base;
        for i in 0..STAT_COUNT {
            let m = scale * self.modifiers[i];
            let lo = -m;
            v += ((stats[i] as f32 - 1.0) * (1.0f32 / 9.0) * (m - lo) + lo) * self.base;
        }
        v
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn composure_scales_fear_modifier_from_1_8_to_0_2() {
        let mut fear = StatModifiedValue::constant(1.0);
        fear.modifiers[Stat::Composure as usize] = 80.0;
        let mut s: Stats = [5; STAT_COUNT];
        s[Stat::Composure as usize] = 1;
        assert!((fear.value(&s, true) - 1.8).abs() < 1e-5);
        s[Stat::Composure as usize] = 10;
        assert!((fear.value(&s, true) - 0.2).abs() < 1e-5);
    }

    #[test]
    fn zero_modifiers_leave_base() {
        let v = StatModifiedValue::constant(3.0);
        assert_eq!(v.value(&[10; STAT_COUNT], false), 3.0);
    }
}
