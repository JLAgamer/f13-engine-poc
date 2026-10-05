//! Float curves matching Unreal 4.18 `FRichCurve::Eval` for the cases the game uses:
//! constant, linear and cubic (non-weighted tangents) keys, with constant extrapolation before the first and
//! after the last key.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Interp {
    Constant,
    Linear,
    Cubic,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Key {
    pub time: f32,
    pub value: f32,
    pub interp: Interp,
    pub arrive_tangent: f32,
    pub leave_tangent: f32,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct RichCurve {
    pub keys: Vec<Key>,
}

impl RichCurve {
    pub fn eval(&self, t: f32) -> f32 {
        let k = &self.keys;
        match k.len() {
            0 => 0.0,
            1 => k[0].value,
            n => {
                if t <= k[0].time {
                    return k[0].value;
                }
                if t >= k[n - 1].time {
                    return k[n - 1].value;
                }
                // last key with time <= t
                let i = k.partition_point(|key| key.time <= t) - 1;
                let (a, b) = (&k[i], &k[i + 1]);
                let diff = b.time - a.time;
                if diff <= 0.0 {
                    return a.value;
                }
                let alpha = (t - a.time) / diff;
                match a.interp {
                    Interp::Constant => a.value,
                    Interp::Linear => a.value + alpha * (b.value - a.value),
                    Interp::Cubic => {
                        let one_third = 1.0f32 / 3.0;
                        let p0 = a.value;
                        let p3 = b.value;
                        let p1 = p0 + a.leave_tangent * diff * one_third;
                        let p2 = p3 - b.arrive_tangent * diff * one_third;
                        bezier(p0, p1, p2, p3, alpha)
                    }
                }
            }
        }
    }
}

/// Unreal's `BezierInterp` (de Casteljau).
fn bezier(p0: f32, p1: f32, p2: f32, p3: f32, a: f32) -> f32 {
    let p01 = p0 + (p1 - p0) * a;
    let p12 = p1 + (p2 - p1) * a;
    let p23 = p2 + (p3 - p2) * a;
    let p012 = p01 + (p12 - p01) * a;
    let p123 = p12 + (p23 - p12) * a;
    p012 + (p123 - p012) * a
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(time: f32, value: f32, interp: Interp) -> Key {
        Key { time, value, interp, arrive_tangent: 0.0, leave_tangent: 0.0 }
    }

    #[test]
    fn clamps_outside_and_hits_keys() {
        let c = RichCurve { keys: vec![key(0.0, 2.0, Interp::Linear), key(10.0, 4.0, Interp::Linear)] };
        assert_eq!(c.eval(-5.0), 2.0);
        assert_eq!(c.eval(20.0), 4.0);
        assert_eq!(c.eval(5.0), 3.0);
    }

    #[test]
    fn flat_tangent_cubic_is_smoothstep_like() {
        let c = RichCurve { keys: vec![key(0.0, 0.0, Interp::Cubic), key(1.0, 1.0, Interp::Cubic)] };
        assert!((c.eval(0.5) - 0.5).abs() < 1e-6);
        assert!(c.eval(0.25) < 0.25, "eases in");
    }

    #[test]
    fn constant_holds() {
        let c = RichCurve { keys: vec![key(0.0, 1.0, Interp::Constant), key(1.0, 5.0, Interp::Constant)] };
        assert_eq!(c.eval(0.99), 1.0);
    }
}
