//! REMASTER engine sound: a grid of loops baked from engine-sim (re-notes/09-remaster.md 9.19,
//! `tools/engine-bake`), crossfaded by rpm and throttle. Classic keeps the original six loops
//! (audio.rs). The pack is a user folder, `<install>/remaster/engines/carNN/`:
//!   engine.cfg   `Engine name`, `Redline rpm`, repeated `Loop file.wav rpm throttle`
//!   *.wav        one seamless loop per grid point (throttle 0 = lifted, 1 = full)

use motor_audio::{wav, Clip};
use std::path::Path;

pub struct GridLoop {
    pub clip: Clip,
    pub rpm: f32,
    pub throttle: f32,
}

pub struct SimEngine {
    pub name: String,
    pub loops: Vec<GridLoop>,
    /// the grid's rpm range: lowest loop to the engine's redline
    pub idle: f32,
    pub redline: f32,
}

/// One loop's share of the mix: its index in `SimEngine::loops`, the amplitude, and the pitch
/// that brings the loop to the wanted rpm.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Tap {
    pub index: usize,
    pub gain: f32,
    pub pitch: f32,
}

impl SimEngine {
    /// Reads `remaster/engines/car{id:02}/engine.cfg`; None when the car has no pack.
    pub fn load(root: &Path, car_id: u32) -> Option<SimEngine> {
        let dir = format!("remaster/engines/car{car_id:02}");
        let text = String::from_utf8(motor_data::fs::read(root, &format!("{dir}/engine.cfg")).ok()?).ok()?;
        let mut e = SimEngine { name: String::new(), loops: Vec::new(), idle: f32::MAX, redline: 0.0 };
        for line in text.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with("//")) {
            let w: Vec<&str> = line.split_whitespace().collect();
            match w.as_slice() {
                ["Engine", rest @ ..] => e.name = rest.join(" "),
                ["Redline", r] => e.redline = r.parse().ok()?,
                ["Loop", file, rpm, thr] => {
                    let bytes = motor_data::fs::read(root, &format!("{dir}/{file}")).ok()?;
                    match wav::decode(&bytes) {
                        Ok(clip) => e.loops.push(GridLoop { clip, rpm: rpm.parse().ok()?, throttle: thr.parse().ok()? }),
                        Err(err) => eprintln!("engine pack {dir}/{file}: {err}"),
                    }
                }
                _ => {}
            }
        }
        e.idle = e.loops.iter().map(|l| l.rpm).fold(f32::MAX, f32::min);
        let top = e.loops.iter().map(|l| l.rpm).fold(0.0, f32::max);
        e.redline = e.redline.max(top);
        (e.loops.len() >= 2 && e.idle < e.redline).then_some(e)
    }

    /// The car's rpm range onto the engine's idle..redline [design choice].
    pub fn map_rpm(&self, rpm: f32, min_rpm: f32, max_rpm: f32) -> f32 {
        let f = ((rpm - min_rpm) / (max_rpm - min_rpm).max(1.0)).clamp(0.0, 1.0);
        self.idle + f * (self.redline - self.idle)
    }

    /// Equal-power crossfade over the two rpm and two throttle rows around the wanted point;
    /// the four taps' gains square to 1. Loops are pitched to `rpm` exactly.
    pub fn taps(&self, rpm: f32, pedal: f32) -> Vec<Tap> {
        let rpm = rpm.clamp(self.idle, self.redline);
        let mut rpms: Vec<f32> = self.loops.iter().map(|l| l.rpm).collect();
        rpms.sort_by(f32::total_cmp);
        rpms.dedup();
        let hi = rpms.iter().position(|&r| r >= rpm).unwrap_or(rpms.len() - 1);
        let lo = hi.saturating_sub((rpms[hi] > rpm) as usize);
        let t = if rpms[hi] > rpms[lo] { (rpm - rpms[lo]) / (rpms[hi] - rpms[lo]) } else { 0.0 };
        let rows = |p: f32| -> [(f32, f32); 2] {
            let a = p.clamp(0.0, 1.0) * std::f32::consts::FRAC_PI_2;
            [(0.0, a.cos()), (1.0, a.sin())]
        };
        let a = t * std::f32::consts::FRAC_PI_2;
        let mut taps = Vec::new();
        for (r, wr) in [(rpms[lo], a.cos()), (rpms[hi], a.sin())] {
            for (thr, wt) in rows(pedal) {
                // the loop at this grid point, or the nearest throttle row when it's missing
                let best = self.loops.iter().enumerate().filter(|(_, l)| l.rpm == r).min_by(|x, y| (x.1.throttle - thr).abs().total_cmp(&(y.1.throttle - thr).abs()));
                if let Some((index, l)) = best {
                    taps.push(Tap { index, gain: wr * wt, pitch: rpm / l.rpm });
                }
            }
        }
        taps
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    fn grid() -> SimEngine {
        let clip = Clip { rate: 44100, data: Arc::new(vec![0.0; 10]), loop_points: None };
        let mut loops = Vec::new();
        for rpm in [1000.0, 2000.0, 4000.0] {
            for throttle in [0.0, 1.0] {
                loops.push(GridLoop { clip: clip.clone(), rpm, throttle });
            }
        }
        SimEngine { name: "t".into(), loops, idle: 1000.0, redline: 4000.0 }
    }

    #[test]
    fn gains_are_equal_power() {
        let e = grid();
        for (rpm, pedal) in [(1000.0, 0.0), (1500.0, 0.3), (2000.0, 1.0), (3999.0, 0.5), (9999.0, 0.5), (0.0, 0.0)] {
            let taps = e.taps(rpm, pedal);
            let sum: f32 = taps.iter().map(|t| t.gain * t.gain).sum();
            assert!((sum - 1.0).abs() < 1e-5, "{rpm} {pedal}: {sum}");
        }
    }

    #[test]
    fn taps_bracket_and_pitch() {
        let e = grid();
        let taps = e.taps(3000.0, 1.0);
        // rpm rows 2000 and 4000, full throttle only has weight
        let live: Vec<&Tap> = taps.iter().filter(|t| t.gain > 1e-4).collect();
        assert_eq!(live.len(), 2);
        for t in live {
            assert!(matches!(e.loops[t.index].rpm, 2000.0 | 4000.0) && e.loops[t.index].throttle == 1.0);
            assert!((t.pitch * e.loops[t.index].rpm - 3000.0).abs() < 1e-3);
            assert!((t.gain - std::f32::consts::FRAC_1_SQRT_2).abs() < 1e-5);
        }
    }

    #[test]
    fn rpm_mapping() {
        let e = grid();
        assert_eq!(e.map_rpm(800.0, 800.0, 8000.0), 1000.0);
        assert_eq!(e.map_rpm(8000.0, 800.0, 8000.0), 4000.0);
        assert_eq!(e.map_rpm(4400.0, 800.0, 8000.0), 2500.0);
    }
}
