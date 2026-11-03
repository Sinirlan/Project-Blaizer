//! Race loading progress (#335): the stage the worker thread is in and how far, shared with the
//! screen that waits for it; the same value carries the cancel flag back to the worker.
//!
//! The stages follow the Remaster loading screen mockups (`reference_materials/loading-overhaul`).
//! They are in the order the work runs, so AUDIO (the race sounds decoded on the worker) comes
//! after REMASTER, where the mockups put it before. GRID is the GPU upload: its CPU half
//! (`render::SceneUpload`) on the worker, then the main thread's slices (`Gpu::load_step`, a few
//! ms a frame behind the loading screen).

use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU8, Ordering};
use std::sync::Mutex;
use std::time::Instant;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Stage {
    /// physics world, racing line, drivers (`Session::with_field`)
    TrackData,
    /// track mesh and textures
    World,
    /// the cars' meshes, paint, wheels
    Cars,
    /// props, lights, sky
    Scene,
    /// light probe source, world fill, water, grass
    Remaster,
    /// race sounds (`audio::race_sounds` of the game)
    Audio,
    /// the scene to the GPU: mip chains on the worker, the upload on the main thread
    Grid,
}

impl Stage {
    pub const ALL: [Stage; 7] = [Stage::TrackData, Stage::World, Stage::Cars, Stage::Scene, Stage::Remaster, Stage::Audio, Stage::Grid];

    pub fn name(self) -> &'static str {
        match self {
            Stage::TrackData => "TRACK DATA",
            Stage::World => "WORLD",
            Stage::Cars => "CARS",
            Stage::Scene => "SCENE",
            Stage::Remaster => "REMASTER",
            Stage::Audio => "AUDIO",
            Stage::Grid => "GRID",
        }
    }

    /// Where this stage's part of the bar starts and how wide it is (0..1 of the bar), from the weights.
    pub fn span(self) -> (f32, f32) {
        let total: f32 = Stage::ALL.iter().map(|s| s.weight()).sum();
        let before: f32 = Stage::ALL.iter().filter(|&&o| o < self).map(|o| o.weight()).sum();
        (before / total, self.weight() / total)
    }

    /// The stage that holds `share` (0..1 of the bar): a picture of the bar without a load (`--load-progress`).
    pub fn at(share: f32) -> Stage {
        Stage::ALL.into_iter().find(|s| { let (a, w) = s.span(); share < a + w }).unwrap_or(Stage::Grid)
    }

    /// This stage's share of the whole bar. [O] measured with the `load:` log line on all eight
    /// tracks in Remaster (0.7-1.35 s in all: CARS ~330 ms, REMASTER 25-470 ms with the track,
    /// GRID 170-290 ms of which the upload is 9-14 frames, WORLD 50-150 ms, SCENE 70-110 ms,
    /// AUDIO and TRACK DATA < 10 ms). Only the sum matters, it is normalised.
    fn weight(self) -> f32 {
        match self {
            Stage::TrackData => 0.01,
            Stage::World => 0.10,
            Stage::Cars => 0.32,
            Stage::Scene => 0.09,
            Stage::Remaster => 0.26,
            Stage::Audio => 0.01,
            Stage::Grid => 0.21,
        }
    }
}

/// What a cancelled load returns as its error.
pub const CANCELLED: &str = "cancelled";

pub struct Progress {
    stage: AtomicU8,
    frac: AtomicU32,
    cancel: AtomicBool,
    /// when each stage was first reported, for the log line
    marks: Mutex<Vec<(Stage, Instant)>>,
}

impl Default for Progress {
    fn default() -> Self {
        Progress { stage: AtomicU8::new(0), frac: AtomicU32::new(0), cancel: AtomicBool::new(false), marks: Mutex::new(Vec::new()) }
    }
}

impl Progress {
    /// The worker reports `stage` at `frac` (0..1 of that stage). Err(`CANCELLED`) once the
    /// load was cancelled: the caller returns it with `?`.
    pub fn set(&self, stage: Stage, frac: f32) -> Result<(), String> {
        if self.cancel.load(Ordering::Relaxed) {
            return Err(CANCELLED.into());
        }
        self.stage.store(stage as u8, Ordering::Relaxed);
        if let Ok(mut m) = self.marks.lock() {
            if m.last().is_none_or(|l| l.0 != stage) {
                m.push((stage, Instant::now()));
            }
        }
        self.frac.store(frac.clamp(0.0, 1.0).to_bits(), Ordering::Relaxed);
        Ok(())
    }

    /// `set` for an optional progress (loaders that are also called without one).
    pub fn report(p: Option<&Progress>, stage: Stage, frac: f32) -> Result<(), String> {
        p.map_or(Ok(()), |p| p.set(stage, frac))
    }

    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }

    pub fn cancelled(&self) -> bool {
        self.cancel.load(Ordering::Relaxed)
    }

    pub fn stage(&self) -> Stage {
        Stage::ALL[(self.stage.load(Ordering::Relaxed) as usize).min(Stage::ALL.len() - 1)]
    }

    /// The whole bar, 0..1.
    pub fn share(&self) -> f32 {
        let total: f32 = Stage::ALL.iter().map(|s| s.weight()).sum();
        let s = self.stage();
        let before: f32 = Stage::ALL.iter().filter(|&&o| o < s).map(|o| o.weight()).sum();
        let f = f32::from_bits(self.frac.load(Ordering::Relaxed));
        ((before + s.weight() * f) / total).clamp(0.0, 1.0)
    }

    /// "TRACK DATA 120 ms, WORLD 840 ms, ..." for the log: each stage from its first report to the next's.
    pub fn timings(&self) -> String {
        let end = Instant::now();
        let marks = self.marks.lock().map(|m| m.clone()).unwrap_or_default();
        let mut out = Vec::new();
        for (i, (st, t)) in marks.iter().enumerate() {
            let next = marks.get(i + 1).map_or(end, |n| n.1);
            out.push(format!("{} {:.0} ms", st.name(), next.duration_since(*t).as_secs_f32() * 1000.0));
        }
        out.join(", ")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spans_tile_the_bar_and_at_finds_them() {
        let mut end = 0.0;
        for s in Stage::ALL {
            let (a, w) = s.span();
            assert!((a - end).abs() < 1e-6, "{s:?} starts at {a}, the last ended at {end}");
            assert_eq!(Stage::at(a + w * 0.5), s);
            end = a + w;
        }
        assert!((end - 1.0).abs() < 1e-6);
        assert_eq!(Stage::at(0.0), Stage::TrackData);
        assert_eq!(Stage::at(1.0), Stage::Grid);
    }
}
