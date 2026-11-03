//! DEBUG race timeline (#60, viewer only, design choice): the frames stepped so far
//! (`motor_sim::replay::Frame`, the format of a repro `.mhr`) plus a snapshot of the race every
//! `KEY_SECS`. Seeking restores the key before the target and re-steps the recorded frames, so any
//! moment of the run is reachable in a few milliseconds and the future survives a rewind.
//! Stepping in the past replays the recorded frames; driving input or a respawn there takes over
//! (the old future is dropped). The sim is deterministic (replay.rs), so a replay is exact.

use motor_sim::car::Input;
use motor_sim::replay::{Frame, Recording};
use motor_sim::session::{Race, Snapshot};

/// A snapshot of the race this often (sim seconds).
const KEY_SECS: f64 = 0.5;
/// The oldest part of the run is dropped beyond this length (sim seconds).
const MAX_SECS: f64 = 600.0;

struct Key {
    /// index into `frames` of the next frame to step from this state
    frame: usize,
    secs: f64,
    snap: Snapshot,
}

pub struct Timeline {
    /// the race as the timeline began: `restart` goes back to it
    origin: Snapshot,
    /// `frames` begin at `origin` (false once the `MAX_SECS` cap dropped the head)
    whole: bool,
    frames: Vec<Frame>,
    keys: Vec<Key>,
    /// frames stepped = the state the race is in
    cursor: usize,
    /// sim seconds at the cursor / at the end of the frames, since the run began
    secs: f64,
    end: f64,
    /// a respawn / skip applied by hand since the last frame; it joins the next recorded one
    respawn: Option<usize>,
    skip: bool,
}

impl Timeline {
    /// Start recording from the race as it is now.
    pub fn new(race: &Race) -> Timeline {
        let origin = race.snapshot();
        Timeline { whole: true, frames: Vec::new(), keys: vec![Key { frame: 0, secs: 0.0, snap: origin.clone() }], origin, cursor: 0, secs: 0.0, end: 0.0, respawn: None, skip: false }
    }

    /// A timeline of a recording (a freshly loaded `race` in its setup): steps every frame, no cap,
    /// and ends at the start, the whole run on the slider.
    pub fn from_frames(race: &mut Race, frames: &[Frame]) -> Timeline {
        let mut t = Timeline::new(race);
        for f in frames {
            Recording::step(race, f);
            t.record(race, *f);
        }
        t.seek(race, 0.0);
        t
    }

    /// The frames begin at the race's start state (they make a complete repro file).
    pub fn whole(&self) -> bool {
        self.whole
    }

    /// Back to the state the timeline began in, a new run from there. The cars keep their
    /// parameters (F11 edits are not part of the state).
    pub fn restart(&mut self, race: &mut Race) {
        let params: Vec<_> = race.racers.iter().map(|r| r.car.p.clone()).collect();
        race.restore(&self.origin);
        for (r, p) in race.racers.iter_mut().zip(params) {
            r.car.p = p;
        }
        self.frames.clear();
        self.keys.truncate(1);
        (self.cursor, self.secs, self.end, self.whole) = (0, 0.0, 0.0, true);
        (self.respawn, self.skip) = (None, false);
    }

    /// Run `f` on the race at the end of the run, then put it back where it was.
    pub fn at_end<R>(&mut self, race: &mut Race, f: impl FnOnce(&Race) -> R) -> R {
        let was = self.secs;
        self.seek(race, self.end);
        let r = f(race);
        self.seek(race, was);
        r
    }

    /// (oldest reachable, now, newest) in sim seconds.
    pub fn span(&self) -> (f64, f64, f64) {
        (self.keys[0].secs, self.secs, self.end)
    }

    pub fn in_past(&self) -> bool {
        self.cursor < self.frames.len()
    }

    /// The recorded frames up to the end (the repro file's body).
    pub fn frames(&self) -> &[Frame] {
        &self.frames
    }

    /// Drop the future after the cursor (the player took over).
    fn truncate(&mut self) {
        self.frames.truncate(self.cursor);
        while self.keys.len() > 1 && self.keys.last().is_some_and(|k| k.frame > self.cursor) {
            self.keys.pop();
        }
        self.end = self.secs;
    }

    /// The followed car was respawned by hand (the caller does it right after).
    pub fn note_respawn(&mut self, slot: usize) {
        self.truncate();
        self.respawn = Some(slot);
    }

    /// The start fly-by was skipped by hand (the caller does it right after).
    pub fn note_skip(&mut self) {
        self.truncate();
        self.skip = true;
    }

    /// Step the race one frame: the recorded one while in the past and `input` is idle, else a
    /// new live frame of `dt` (`hand` = the slot `input` drives, `Race::step_as`). Returns the step's length.
    pub fn step(&mut self, race: &mut Race, dt: f32, input: Input, hand: Option<usize>) -> f32 {
        let replay = self.in_past() && input == Input::default() && self.respawn.is_none() && !self.skip;
        if replay {
            let f = self.frames[self.cursor];
            Recording::step(race, &f);
            self.cursor += 1;
            self.secs += f.dt as f64;
            return f.dt;
        }
        self.truncate();
        let f = Frame { dt, input, respawn: self.respawn.take(), skip: std::mem::take(&mut self.skip), hand };
        // a respawn / skip of this frame was already applied by hand
        race.step_as(f.dt, f.input, f.hand);
        self.record(race, f);
        self.trim();
        dt
    }

    /// Append the frame just stepped (live) and a key every `KEY_SECS`.
    fn record(&mut self, race: &Race, f: Frame) {
        self.frames.push(f);
        self.cursor += 1;
        self.secs += f.dt as f64;
        self.end = self.secs;
        if self.secs - self.keys.last().map_or(0.0, |k| k.secs) >= KEY_SECS {
            self.keys.push(Key { frame: self.cursor, secs: self.secs, snap: race.snapshot() });
        }
    }

    /// Drop the oldest part of a live run beyond `MAX_SECS`.
    fn trim(&mut self) {
        if self.secs - self.keys[0].secs > MAX_SECS && self.keys.len() > 2 {
            let gone = self.keys[1].frame;
            self.keys.remove(0);
            self.frames.drain(..gone);
            self.cursor -= gone;
            for k in &mut self.keys {
                k.frame -= gone;
            }
            self.whole = false;
        }
    }

    /// Put the race at the recorded moment `target` (clamped to the run; lands on the last frame
    /// boundary at or before it). Returns the sim seconds moved (negative = back).
    pub fn seek(&mut self, race: &mut Race, target: f64) -> f64 {
        const EPS: f64 = 1e-4;
        let before = self.secs;
        let target = target.clamp(self.keys[0].secs, self.end);
        self.respawn = None;
        self.skip = false;
        let k = self.keys.iter().rposition(|k| k.secs <= target + EPS).unwrap_or(0);
        // going forward from where we are is cheaper than from the key, when it is ahead of the key
        if self.cursor < self.keys[k].frame || self.secs > target + EPS {
            let key = &self.keys[k];
            race.restore(&key.snap);
            (self.cursor, self.secs) = (key.frame, key.secs);
        }
        while self.cursor < self.frames.len() && self.secs + self.frames[self.cursor].dt as f64 <= target + EPS {
            let f = self.frames[self.cursor];
            Recording::step(race, &f);
            self.cursor += 1;
            self.secs += f.dt as f64;
        }
        self.secs - before
    }
}
