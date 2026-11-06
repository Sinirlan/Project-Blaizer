//! The start camera: the fly-by before the race (#216, 08-missing-features §8.16).
//!
//! The original's pre-race lasts 12 s from the race clock base `0x704e38`. While the start-camera
//! flag `0x4b7b1c` is set (`0x45b6a4..0x45ba51`) the eye comes from the track's two grid-to-start
//! motions `StartSpline1/2` (`BasePath\StartSplineN`, loaded by `0x45ea6d` with scale 1.0) and the
//! camera looks at the player's car [O]:
//! 0..6 s the eye stands at spline 1's first key, 6..8 s it follows spline 1, 8..10 s spline 2,
//! with the frame `(ms - window start) * 0.0125` (`0x4d4f46`); from 10 s the saved camera returns.

use crate::props::evaluate;
use glam::Vec3;
use motor_data::cfg::Motion;

/// End of the fixed view and start of spline 1 (`0x45b754`: base + 0x1770).
pub const HOLD_S: f32 = 6.0;
// HOLD_S stays a plain const: it is also the moment the "3" appears and the camera keys unlock
// (the countdown trio of `session.rs`), so it moves with those, not on its own.
motor_tune::tunable! {
    /// Spline 2 starts (`0x45b791`: base + 0x1f40) [O], seconds after the race init. Keep it between
    /// `HOLD_S` (6 s) and `END_S`.
    /// range: 6..12
    /// unit: s
    SPLINE2_S: f32 = 8.0;
    /// The start camera ends (`0x45b9b5`: base + 0x2710) [O], seconds after the race init; the player's camera returns.
    /// range: 6..12
    /// unit: s
    pub END_S: f32 = 10.0;
    /// Motion frames per millisecond (`0x4d4f46`) [O]: how fast the fly-by splines play.
    /// range: 0.001..0.05
    /// unit: frames/ms
    FRAMES_PER_MS: f32 = 0.0125;
}

#[derive(Debug, Clone, Default)]
pub struct StartCamera {
    splines: [Option<Motion>; 2],
}

impl StartCamera {
    pub fn new(a: Option<Motion>, b: Option<Motion>) -> StartCamera {
        StartCamera { splines: [a, b] }
    }

    /// The eye (game space) `t` seconds after the race init; `None` once the start camera is
    /// over, or when the window's spline is missing (the game then keeps the player's camera).
    pub fn eye(&self, t: f32) -> Option<Vec3> {
        let (spline2, end) = (SPLINE2_S.get(), END_S.get());
        let (m, from) = match t {
            t if t < 0.0 => return None,
            t if t < HOLD_S => (self.splines[0].as_ref()?, None),
            t if t < spline2 => (self.splines[0].as_ref()?, Some(HOLD_S)),
            t if t < end => (self.splines[1].as_ref()?, Some(spline2)),
            _ => return None,
        };
        let frame = from.map_or(0.0, |s| (t - s) * 1000.0 * FRAMES_PER_MS.get());
        let v = evaluate(m, frame, false);
        (v.len() >= 3).then(|| Vec3::new(v[0], v[1], v[2]))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use motor_data::cfg::MotionKey;

    fn motion(x0: f32, x1: f32) -> Motion {
        let key = |x: f32, frame: f32| MotionKey { values: vec![x, 0.0, 0.0], frame, linear: 1.0, tension: 0.0, continuity: 0.0, bias: 0.0 };
        Motion { version: 1, channels: 3, keys: vec![key(x0, 0.0), key(x1, 30.0)] }
    }

    #[test]
    fn holds_then_spline_one_then_two_then_ends() {
        let c = StartCamera::new(Some(motion(0.0, 30.0)), Some(motion(100.0, 130.0)));
        assert_eq!(c.eye(0.0).unwrap().x, 0.0);
        assert_eq!(c.eye(5.9).unwrap().x, 0.0);
        // 1 s into the window = frame 12.5 of 30
        assert!((c.eye(7.0).unwrap().x - 12.5).abs() < 1e-3);
        assert!((c.eye(9.0).unwrap().x - 112.5).abs() < 1e-3);
        assert!(c.eye(10.0).is_none());
        assert!(StartCamera::default().eye(1.0).is_none());
    }
}
