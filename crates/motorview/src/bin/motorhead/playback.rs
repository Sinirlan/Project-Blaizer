//! REMASTER demo-play controls (#346; mockup B · Telemetry, `reference_materials/playback-overhaul/`): the state of
//! watching an `.MDE` (Ghost screen > replay race) as a player with play / pause, hold to rewind or fast-forward,
//! lap jumps, a speed from x0.25 to x4 and a telemetry fold-out. The picture is `hud_glass::transport`; the keys are
//! read here. Classic keeps the original's demo play (no controls, the list back after 4 s).
//!
//! DESIGN CHOICE: the original plays a demo forward only. Here the posed car is a `remote` racer whose lap progress
//! runs forward only, so a backward seek restores the race snapshot kept every `KEY_MS` of file time and steps the
//! posed car forward again from it in fixed `STEP` seconds (motorview's timeline does the same for its slider). The
//! file clock is `demo::race_ms`: 0 at the "3", `GO_MS` at the green light; before 0 the start fly-by runs.

use motor_sim::car::Input;
use motor_sim::demo::{self, GO_MS};
use motor_sim::demo_stats;
use motor_sim::race::TrackProfile;
use motor_sim::session::Snapshot;
use motorview::drive::{CamMode, Session};
use std::collections::HashSet;
use winit::keyboard::KeyCode;

use crate::controls::{task, Device, PadIn};
use crate::demo::DemoCars;
use crate::front::menu_modern::{code_glyph, key_glyph, pad_glyph};
use crate::front::Game;
use crate::hud_transport::Hint;

/// The speeds of the speed chip (the file's time over the clock's).
pub const RATES: [f32; 5] = [0.25, 0.5, 1.0, 2.0, 4.0];
/// The index of x1 in `RATES`.
pub const NORMAL: usize = 2;
/// The speed while a rewind or forward control is held.
pub const HOLD_RATE: f32 = 4.0;
/// The controls stay this long after the last input (s).
pub const SHOW_FOR: f32 = 3.2;
/// A tap on rewind / forward (shorter than this, s) skips `TAP_SKIP` instead of scrubbing.
const TAP_MAX: f32 = 0.22;
const TAP_SKIP: f64 = 5000.0;
/// Q within this of a lap's start goes to the lap before (ms).
const LAP_BACK_MS: f64 = 1500.0;
/// A snapshot of the race this often (ms of file time).
const KEY_MS: f64 = 500.0;
/// The longest sim step while playing fast or re-stepping after a seek (s): a checkpoint is never jumped over.
const STEP: f32 = 1.0 / 30.0;
/// Points of the speed trace: one per 2 design px of the 708 px bar.
const TRACE_POINTS: usize = 354;
/// How fast the card fades and the diagram unfolds (1 / s): the mockup's .28 s.
const EASE: f32 = 12.0;

/// The next view of the Change camera key in demo play: the race's four views, then the TV camera when the track has
/// one (the other races keep the four; F3 and the pad's D-pad right pick the TV camera at once in every race).
pub fn next_camera(cur: CamMode, has_tv: bool) -> CamMode {
    match cur {
        CamMode::Bumper if has_tv => CamMode::Tv,
        _ => cur.next(),
    }
}

/// What the keys and pad can do. Edges are taken from one frame to the next (`Playback::input`).
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Act {
    Play,
    Rew,
    Fwd,
    Prev,
    Next,
    Faster,
    Slower,
    Tele,
    Home,
    Confirm,
    /// the speed chip clicked: the next speed, wrapping (the mockup's `cycleSpeed`; the keys step up and down)
    Speed,
    /// the camera chip clicked: the next view (taken by the game, `Mouse::camera`)
    Camera,
}

/// What the pointer can be over on the card (`hud_transport::targets` gives their rectangles).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Target {
    Back,
    Rew,
    Play,
    Fwd,
    Next,
    Speed,
    Camera,
    Tele,
    /// the progress bar and the speed diagram: a drag seeks
    Bar,
    Graph,
    /// the end choice's pills (0 WATCH AGAIN, 1 BACK TO THE LIST)
    Pick(usize),
}

/// The pointer's part of the state (REMASTER, #352). The pointer is in HUD canvas px.
#[derive(Default, Debug)]
pub struct Mouse {
    pub pointer: Option<[f32; 2]>,
    pub hover: Option<Target>,
    /// the target a press began on, until the button comes up
    pub press: Option<Target>,
    down: bool,
    /// the bar or diagram is being dragged
    pub scrub: bool,
    /// file ms the bubble tells about: under the pointer, or the dragged-to moment
    pub tip: Option<f64>,
    /// the camera chip was clicked
    pub camera: bool,
}

/// The camera's name on the camera chip.
pub fn cam_label(c: CamMode) -> &'static str {
    match c {
        CamMode::Chase => "CHASE",
        CamMode::FarChase => "FAR",
        CamMode::InCar => "IN-CAR",
        CamMode::Bumper => "BUMPER",
        CamMode::Goal => "GOAL",
        CamMode::Tv => "TV",
    }
}

/// The acts held this frame: the mockup's keys and the pad (`down` = a pad button is held).
pub fn held_acts(keys: &HashSet<KeyCode>, down: &dyn Fn(gilrs::Button) -> bool) -> HashSet<Act> {
    use gilrs::Button as B;
    let k = |c: &[KeyCode]| c.iter().any(|c| keys.contains(c));
    let mut out = HashSet::new();
    let mut add = |on: bool, a: Act| {
        if on {
            out.insert(a);
        }
    };
    add(k(&[KeyCode::Space]) || down(B::South), Act::Play);
    add(k(&[KeyCode::ArrowLeft]) || down(B::LeftTrigger2) || down(B::DPadLeft), Act::Rew);
    add(k(&[KeyCode::ArrowRight]) || down(B::RightTrigger2) || down(B::DPadRight), Act::Fwd);
    add(k(&[KeyCode::KeyQ, KeyCode::PageUp]) || down(B::LeftTrigger), Act::Prev);
    add(k(&[KeyCode::KeyE, KeyCode::PageDown]) || down(B::RightTrigger), Act::Next);
    add(k(&[KeyCode::ArrowUp]) || down(B::DPadUp), Act::Faster);
    add(k(&[KeyCode::ArrowDown]) || down(B::DPadDown), Act::Slower);
    add(k(&[KeyCode::KeyT]) || down(B::West), Act::Tele);
    add(k(&[KeyCode::Home]) || down(B::Select), Act::Home);
    add(k(&[KeyCode::Enter, KeyCode::NumpadEnter]), Act::Confirm);
    out
}

/// What a recording says, worked out once (`Playback::new`): the laps of the recorder on the file clock.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Laps {
    /// file ms of each lap's start (the green light, then every line crossing)
    pub starts: Vec<f64>,
    /// the flag: file ms the recorder finished
    pub flag: Option<f64>,
    /// the best lap (index into `starts`)
    pub best: Option<usize>,
    /// top speed (km/h) and when (file ms)
    pub top: Option<(u32, f64)>,
}

impl Laps {
    /// From `demo_stats` against the track's lap; without a profile (or a recording that is not a race) only the green light.
    pub fn of(mde: &motor_data::mde::Mde, profile: Option<&TrackProfile>) -> Laps {
        let Some(st) = profile.and_then(|p| demo_stats::analyse(mde, p)) else { return Laps { starts: vec![GO_MS], ..Laps::default() } };
        let run = &st.cars[st.me];
        let mut starts = vec![GO_MS];
        for &l in &run.laps_ms {
            starts.push(starts[starts.len() - 1] + l as f64);
        }
        let flag = run.finish_ms.map(|f| GO_MS + f as f64);
        // the line crossing that ends the last lap is the flag, not another lap's start
        if flag.is_some() {
            starts.pop();
        }
        Laps { starts, flag, best: st.best.map(|b| b.0 as usize - 1).filter(|&b| b < run.laps_ms.len()), top: None }
    }

    /// The start and end of lap `k` (file ms); the last lap ends at the flag, an unfinished one at `len`.
    pub fn span(&self, k: usize, len: f64) -> (f64, f64) {
        (self.starts[k], self.starts.get(k + 1).copied().or(self.flag).unwrap_or(len))
    }

    /// The jump marks of Q / E: the "3", then every lap's start.
    pub fn marks(&self) -> Vec<f64> {
        std::iter::once(0.0).chain(self.starts.iter().copied()).collect()
    }
}

/// One frame's view of the controls for `hud_glass::transport`.
pub struct View<'a> {
    pub t_ms: f64,
    pub len_ms: f64,
    pub laps: &'a Laps,
    pub playing: bool,
    pub hold: i8,
    pub rate: f32,
    pub tele: bool,
    pub trace: &'a [f32],
    /// card opacity 0..1 (the 3 s timeout fades it)
    pub show: f32,
    /// the diagram's unfold 0..1
    pub unfold: f32,
    pub ended: bool,
    pub picked: usize,
    pub hover: Option<Target>,
    pub tip: Option<f64>,
    pub pointer: Option<[f32; 2]>,
    /// the bar or diagram is dragged
    pub scrub: bool,
}

pub struct Playback {
    pub playing: bool,
    /// index into `RATES`
    pub rate: usize,
    /// -1 / 0 / 1: a rewind or forward control is held
    pub hold: i8,
    /// seconds the controls still show
    active: f32,
    pub tele: bool,
    /// the end choice: 0 WATCH AGAIN, 1 BACK TO THE LIST
    pub picked: usize,
    /// the file's last record has passed
    pub ended: bool,
    /// "BACK TO THE LIST" was confirmed: the game leaves
    pub leave: bool,
    pub len_ms: f64,
    pub laps: Laps,
    /// the speed trace over the file, built when the diagram is first unfolded
    pub trace: Vec<f32>,
    /// eased card opacity and unfold, 0..1
    show: f32,
    unfold: f32,
    keys: Vec<(f64, Snapshot)>,
    prev: HashSet<Act>,
    pub mouse: Mouse,
    /// seconds the rewind / forward control has been held, and where the file clock was at the press
    held_s: f32,
    held_from: f64,
}

impl Playback {
    pub fn new(mde: &motor_data::mde::Mde, profile: Option<&TrackProfile>) -> Playback {
        let len_ms = mde.records.last().map_or(0.0, |r| r.ms as f64);
        let laps = Laps::of(mde, profile);
        Playback { playing: true, rate: NORMAL, hold: 0, active: SHOW_FOR, tele: false, picked: 0, ended: false, leave: false, len_ms, laps, trace: Vec::new(), show: 1.0, unfold: 0.0, keys: Vec::new(), prev: HashSet::new(), mouse: Mouse::default(), held_s: 0.0, held_from: 0.0 }
    }

    /// The speed trace and top speed of the recorder's `block`, once the diagram needs them.
    pub fn ensure_trace(&mut self, demo: &DemoCars, block: usize) {
        if !self.trace.is_empty() {
            return;
        }
        self.trace = demo_stats::speed_trace(&demo.mde, block, TRACE_POINTS);
        let (t0, span) = (demo.mde.records.first().map_or(0.0, |r| r.ms as f64), self.len_ms - demo.mde.records.first().map_or(0.0, |r| r.ms as f64));
        let end = self.laps.flag.unwrap_or(self.len_ms);
        let at = |q: usize| t0 + span * q as f64 / (self.trace.len().max(2) - 1) as f64;
        let top = (0..self.trace.len()).filter(|&q| at(q) >= GO_MS && at(q) <= end).max_by(|&a, &b| self.trace[a].total_cmp(&self.trace[b]));
        self.laps.top = top.map(|q| (self.trace[q].round() as u32, at(q)));
    }

    /// The speed of the clock: the control held, the chip's rate, or 0 (paused, ended).
    pub fn speed(&self) -> f32 {
        if self.mouse.scrub {
            // DESIGN CHOICE: the clock holds while the bar is dragged (the file is where the pointer is)
            0.0
        } else if self.hold != 0 {
            self.hold as f32 * HOLD_RATE
        } else if self.playing && !self.ended {
            RATES[self.rate]
        } else {
            0.0
        }
    }

    /// Some of the card is still on screen (it fades out over a few frames after `visible` goes).
    pub fn shown(&self) -> bool {
        self.show > 0.01
    }

    /// The controls are drawn: just touched, paused, held or ended.
    pub fn visible(&self) -> bool {
        self.active > 0.0 || !self.playing || self.ended || self.hold != 0 || self.mouse.scrub
    }

    /// The race sound is off while it is not running at its own speed.
    pub fn silent(&self) -> bool {
        self.hold != 0 || !self.playing || self.ended || self.mouse.scrub
    }

    pub fn view(&self, t_ms: f64) -> View<'_> {
        View { t_ms, len_ms: self.len_ms, laps: &self.laps, playing: self.playing, hold: self.hold, rate: RATES[self.rate], tele: self.tele, trace: &self.trace, show: self.show, unfold: self.unfold, ended: self.ended, picked: self.picked, hover: self.mouse.hover, tip: self.mouse.tip, pointer: self.mouse.pointer, scrub: self.mouse.scrub }
    }

    /// Headless shots: the card and the unfold at their targets at once.
    pub fn snap(&mut self) {
        (self.show, self.unfold) = (self.visible() as u8 as f32, self.tele as u8 as f32);
    }

    /// Headless shots: the controls timed out.
    pub fn hide(&mut self) {
        self.active = 0.0;
        self.snap();
    }

    /// Shown for `SHOW_FOR` more seconds.
    pub fn touch(&mut self) {
        self.active = SHOW_FOR;
    }

    /// The input of one frame: the edges of `held_acts`. Returns the file ms to seek to, if asked.
    pub fn input(&mut self, now: &HashSet<Act>, dt: f32, t_ms: f64) -> Option<f64> {
        let prev = self.prev.clone();
        let edge = |a: Act| now.contains(&a) && !prev.contains(&a);
        let mut seek = None;
        if !now.is_empty() || !self.prev.is_empty() {
            self.touch();
        }
        if self.ended {
            // the end choice: left / right pick, Enter or Play confirm
            if edge(Act::Rew) {
                self.picked = 0;
            }
            if edge(Act::Fwd) {
                self.picked = 1;
            }
            // Play (Space, A) always starts again, as the mockup's play button turned into replay
            if edge(Act::Play) || edge(Act::Confirm) && self.picked == 0 {
                seek = Some(0.0);
                self.restart();
            } else if edge(Act::Confirm) {
                self.leave = true;
            }
        } else {
            if edge(Act::Play) {
                self.playing = !self.playing;
            }
            // hold to scrub: acts only while held; a tap (released within `TAP_MAX`) skips 5 s instead
            let d = now.contains(&Act::Fwd) as i8 - now.contains(&Act::Rew) as i8;
            // nothing to rewind to at the "3"
            let d = if d < 0 && t_ms <= 0.0 { 0 } else { d };
            if d != 0 && self.hold == 0 {
                (self.held_s, self.held_from) = (0.0, t_ms);
            }
            if d != 0 {
                self.held_s += dt;
            } else if self.hold != 0 {
                if self.held_s < TAP_MAX {
                    seek = Some(self.held_from + self.hold as f64 * TAP_SKIP);
                }
                self.held_s = 0.0;
            }
            self.hold = d;
            if edge(Act::Prev) {
                let m = self.laps.marks();
                seek = Some(m.iter().rev().copied().find(|&x| x < t_ms - LAP_BACK_MS).unwrap_or(0.0));
            }
            if edge(Act::Next) {
                seek = self.laps.marks().into_iter().find(|&x| x > t_ms + 50.0).or(seek);
            }
            if edge(Act::Faster) {
                self.rate = (self.rate + 1).min(RATES.len() - 1);
            }
            if edge(Act::Slower) {
                self.rate = self.rate.saturating_sub(1);
            }
            if edge(Act::Speed) {
                self.rate = (self.rate + 1) % RATES.len();
            }
            if edge(Act::Home) {
                seek = Some(0.0);
                self.restart();
            }
        }
        if edge(Act::Tele) {
            self.tele = !self.tele;
        }
        if edge(Act::Camera) {
            self.mouse.camera = true;
        }
        self.prev = now.clone();
        seek
    }

    /// The pointer of one frame (REMASTER, #352): `hits` are the rectangles of what can be pressed (canvas px, empty while the
    /// card is not shown), `down` the left button. A press on a control puts its act in `held`, so a click acts as its key
    /// does (a held rewind scrubs, a short one skips 5 s); a press on the bar or diagram latches it and the pointer's moment
    /// is returned as the file ms to seek to while it is dragged. Over the end choice the pointer picks.
    pub fn mouse(&mut self, hits: &[(Target, [f32; 4])], pointer: Option<[f32; 2]>, down: bool, held: &mut HashSet<Act>) -> Option<f64> {
        let inside = |r: &[f32; 4], p: [f32; 2]| p[0] >= r[0] && p[0] < r[0] + r[2] && p[1] >= r[1] && p[1] < r[1] + r[3];
        let hit = pointer.and_then(|p| hits.iter().find(|(_, r)| inside(r, p)).map(|(t, _)| *t));
        let moved = pointer != self.mouse.pointer;
        self.mouse.pointer = pointer;
        self.mouse.hover = hit;
        if let (true, true, Some(Target::Pick(i))) = (moved, self.ended, hit) {
            self.picked = i;
        }
        let edge = down && !self.mouse.down;
        self.mouse.down = down;
        if edge {
            self.touch();
            if hit.is_some() {
                self.mouse.press = hit;
                if let Some(Target::Pick(i)) = hit {
                    self.picked = i;
                }
            }
        }
        if !down {
            self.mouse.press = None;
        }
        let act = match self.mouse.press {
            Some(Target::Back) => Some(Act::Prev),
            Some(Target::Rew) => Some(Act::Rew),
            Some(Target::Play) => Some(Act::Play),
            Some(Target::Fwd) => Some(Act::Fwd),
            Some(Target::Next) => Some(Act::Next),
            Some(Target::Speed) => Some(Act::Speed),
            Some(Target::Camera) => Some(Act::Camera),
            Some(Target::Tele) => Some(Act::Tele),
            Some(Target::Pick(_)) => Some(Act::Confirm),
            Some(Target::Bar | Target::Graph) | None => None,
        };
        held.extend(act);
        // the moment under the pointer on the bar or diagram (a latched drag keeps its own, even off the rectangle)
        let at = |t: Target, p: [f32; 2]| hits.iter().find(|(h, _)| *h == t).map(|(_, r)| ((p[0] - r[0]) / r[2]).clamp(0.0, 1.0) as f64 * self.len_ms);
        let drag = self.mouse.press.filter(|t| matches!(t, Target::Bar | Target::Graph));
        self.mouse.scrub = drag.is_some();
        let mut seek = None;
        self.mouse.tip = match (drag, hit, pointer) {
            (Some(t), _, Some(p)) => at(t, p),
            (None, Some(t @ (Target::Bar | Target::Graph)), Some(p)) => at(t, p),
            _ => None,
        };
        if let (true, Some(ms)) = (drag.is_some() && (edge || moved), self.mouse.tip) {
            seek = Some(ms);
        }
        seek
    }

    /// WATCH AGAIN / Home: playing from the "3", the choice forgotten.
    pub fn restart(&mut self) {
        (self.playing, self.ended, self.picked, self.hold) = (true, false, 0, 0);
        self.touch();
    }

    /// Ease the card's opacity and the diagram's unfold (`dt` real seconds).
    pub fn ease(&mut self, dt: f32, running: bool) {
        if running && self.hold == 0 && self.playing && !self.ended && !self.mouse.scrub {
            self.active = (self.active - dt).max(0.0);
        }
        let k = 1.0 - (-EASE * dt).exp();
        self.show += (self.visible() as u8 as f32 - self.show) * k;
        self.unfold += (self.tele as u8 as f32 - self.unfold) * k;
    }

    /// Keep a snapshot of the race when `ms` is `KEY_MS` past the last one.
    fn key(&mut self, race: &motor_sim::session::Race, ms: f64) {
        if self.keys.last().is_none_or(|k| ms >= k.0 + KEY_MS) {
            self.keys.push((ms, race.snapshot()));
        }
    }

    /// One sim step of `h` s of file time: the race, then the recorded car posed at the new moment. `full` runs the
    /// session's effects too (smoke, skid marks, the camera), a seek's re-stepping does not.
    fn step(&mut self, sess: &mut Session, demo: &DemoCars, block: usize, player: usize, h: f32, full: bool) {
        if full {
            sess.step_as(h, Input::default(), None);
        } else {
            sess.race.step_as(h, Input::default(), None);
        }
        let ms = demo::race_ms(&sess.race);
        // #374: the recorded car is posed at the view time (the other cars are drawn up to a tick behind the clock)
        if let Some(s) = demo.state(block, demo::view_ms(&sess.race)) {
            demo::drive_remote(&mut sess.race.racers[player], &s, h, &sess.race.world.materials);
        }
        self.key(&sess.race, ms);
    }

    /// Run the file forward by `adv` seconds of file time (in steps of at most `STEP`); stops at the end of the file.
    fn run(&mut self, sess: &mut Session, demo: &DemoCars, block: usize, player: usize, adv: f32, full: bool) {
        let mut left = adv;
        while left > 1e-6 {
            let now = demo::race_ms(&sess.race);
            if now >= self.len_ms {
                self.ended = true;
                self.playing = false;
                self.hold = 0;
                return;
            }
            let h = left.min(STEP).min(((self.len_ms - now) / 1000.0).max(1e-4) as f32);
            self.step(sess, demo, block, player, h, full);
            left -= h;
        }
        if demo::race_ms(&sess.race) >= self.len_ms {
            self.ended = true;
            self.playing = false;
            self.hold = 0;
        }
    }

    /// Put the race at file time `target` ms (the "3" at the earliest, the end of the file at the latest): the latest
    /// snapshot at or before it, then stepped forward.
    pub fn seek(&mut self, sess: &mut Session, demo: &DemoCars, block: usize, player: usize, target: f64) {
        let target = target.clamp(0.0, self.len_ms);
        self.key(&sess.race, demo::race_ms(&sess.race));
        let now = demo::race_ms(&sess.race);
        let k = self.keys.partition_point(|k| k.0 <= target).saturating_sub(1);
        // going on from where the race is, when that is after the key and not past the target
        if now > target + 0.5 || now < self.keys[k].0 {
            sess.race.restore(&self.keys[k].1);
        }
        self.ended = false;
        let from = demo::race_ms(&sess.race);
        self.run(sess, demo, block, player, ((target - from) / 1000.0).max(0.0) as f32, false);
        sess.jumped();
    }

    /// One frame: the input's seek or the clock's step. `dt` is the real frame time. Returns whether the race jumped to
    /// another moment (a seek, so the HUD's lap and checkpoint memory must follow).
    pub fn frame(&mut self, sess: &mut Session, demo: &DemoCars, block: usize, player: usize, dt: f32, seek: Option<f64>) -> bool {
        self.key(&sess.race, demo::race_ms(&sess.race));
        if self.tele || self.mouse.tip.is_some() {
            self.ensure_trace(demo, block);
        }
        if let Some(t) = seek {
            self.seek(sess, demo, block, player, t);
            return true;
        }
        let v = self.speed();
        let now = demo::race_ms(&sess.race);
        if v > 0.0 {
            self.run(sess, demo, block, player, dt * v, true);
        } else if v < 0.0 {
            if now <= 0.0 {
                self.hold = 0;
            } else {
                self.seek(sess, demo, block, player, now + dt as f64 * v as f64 * 1000.0);
                return true;
            }
        } else {
            sess.step_camera(dt);
        }
        false
    }
}

impl Game {
    /// The hint row under the controls, with the glyphs of the device used last: keys (or pad buttons) for play / pause,
    /// the holds, laps, speed, camera (the layout's change camera control), telemetry and the pause menu; the end choice has its own.
    pub fn playback_hints(&self, ended: bool) -> Vec<Hint> {
        use gilrs::Button as B;
        let dev = if self.controls.last == Device::PlayStation { Device::PlayStation } else { Device::Xbox };
        let pad = self.controls.last != Device::Keyboard;
        let key = |k: KeyCode| key_glyph(k).map(|c| (c, [0.7, 0.92, 1.0, 1.0]));
        let btn = |b: B| Some(pad_glyph(PadIn::Button(b), dev));
        let caps = |keys: &[KeyCode], buttons: &[B]| -> Vec<(char, [f32; 4])> {
            if pad { buttons.iter().filter_map(|&b| btn(b)).collect() } else { keys.iter().filter_map(|&k| key(k)).collect() }
        };
        // the camera: whatever the layout (or the pad) binds to Change camera
        let cam: Vec<(char, [f32; 4])> = if pad {
            self.controls.pad_bind[task::CHANGE_CAMERA].map(|p| pad_glyph(p, dev)).into_iter().collect()
        } else {
            self.controls.layout().bind[task::CHANGE_CAMERA].and_then(|c| code_glyph(c, dev)).into_iter().collect()
        };
        let menu = caps(&[KeyCode::Escape], &[B::Start]);
        if ended {
            return vec![
                Hint { caps: caps(&[KeyCode::ArrowLeft, KeyCode::ArrowRight], &[B::DPadLeft, B::DPadRight]), label: "CHOOSE" },
                Hint { caps: caps(&[KeyCode::Enter], &[B::South]), label: "CONFIRM" },
                Hint { caps: menu, label: "MENU" },
            ];
        }
        vec![
            Hint { caps: caps(&[KeyCode::Space], &[B::South]), label: "PLAY / PAUSE" },
            Hint { caps: caps(&[KeyCode::ArrowLeft, KeyCode::ArrowRight], &[B::LeftTrigger2, B::RightTrigger2]), label: "HOLD TO SCRUB" },
            Hint { caps: caps(&[KeyCode::KeyQ, KeyCode::KeyE], &[B::LeftTrigger, B::RightTrigger]), label: "LAP" },
            Hint { caps: caps(&[KeyCode::ArrowUp, KeyCode::ArrowDown], &[B::DPadUp, B::DPadDown]), label: "SPEED" },
            Hint { caps: cam, label: "CAMERA" },
            Hint { caps: caps(&[KeyCode::KeyT], &[B::West]), label: "TELEMETRY" },
            Hint { caps: menu, label: "MENU" },
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn acts(a: &[Act]) -> HashSet<Act> {
        a.iter().copied().collect()
    }

    fn pb() -> Playback {
        Playback { playing: true, rate: NORMAL, hold: 0, active: 0.0, tele: false, picked: 0, ended: false, leave: false, len_ms: 200_000.0, laps: Laps { starts: vec![6000.0, 71_208.0, 136_079.0], flag: Some(201_098.0), best: Some(1), top: None }, trace: Vec::new(), show: 1.0, unfold: 0.0, keys: Vec::new(), prev: HashSet::new(), mouse: Mouse::default(), held_s: 0.0, held_from: 0.0 }
    }

    /// JeePen.MDE (ten laps of NeoCity) as a posed race: the session, the recorded cars and a playback over them.
    fn jeepen() -> Option<(Session, DemoCars, Playback)> {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../installed_game");
        let mde = motor_data::mde::Mde::load(&root.join("Demos/JeePen.MDE")).ok()?;
        let slots = mde.header.active_slots();
        let slot = if slots.contains(&(mde.header.player as usize)) { mde.header.player as usize } else { slots[0] };
        let mut sess = Session::with_field(&root, 3, &[7], Some(0), motor_sim::session::AiRow::Difficulty(motor_sim::session::Difficulty::Medium), Some(mde.header.laps), false, None).ok()?;
        sess.race.racers[0].remote = true;
        let (demo, _) = DemoCars::new(&root, mde, 1, Some(slot), |_| 7).ok()?;
        let pb = Playback::new(&demo.mde, None);
        Some((sess, demo, pb))
    }

    /// What the HUD shows of the recorder's progress.
    fn standing(sess: &Session) -> (u32, Option<u32>, Option<u32>, u32) {
        let p = &sess.race.racers[0].progress;
        (p.lap, p.best_lap_ms, p.last_lap_ms, p.lap_start_ms)
    }

    /// Playing to a moment, seeking back and playing on again, or jumping to it straight, give the laps and bests
    /// the race has when it is only ever played forward (the remote racer's progress runs forward only).
    #[test]
    fn a_seek_rebuilds_the_laps_as_playing_forward_does() {
        let Some((mut sess, demo, mut pb)) = jeepen() else { return };
        let go = |sess: &mut Session, pb: &mut Playback, ms: f64| {
            while demo::race_ms(&sess.race) < ms {
                pb.frame(sess, &demo, 0, 0, 1.0 / 60.0, None);
            }
        };
        // forward only: three laps and a bit (a lap of NeoCity is about 1 minute)
        go(&mut sess, &mut pb, 6000.0 + 200_000.0);
        let reference = standing(&sess);
        assert!(reference.0 >= 3 && reference.2.is_some(), "{reference:?}");
        let at = demo::race_ms(&sess.race);
        // back to the first lap, then on again to the same moment
        pb.seek(&mut sess, &demo, 0, 0, 20_000.0);
        assert_eq!(standing(&sess).0, 1);
        assert!((demo::race_ms(&sess.race) - 20_000.0).abs() < 1.0);
        go(&mut sess, &mut pb, at);
        assert_eq!(standing(&sess), reference);
        // straight there from a moment before: the snapshots and the re-stepping reach the same state
        pb.seek(&mut sess, &demo, 0, 0, 100_000.0);
        pb.seek(&mut sess, &demo, 0, 0, at);
        assert_eq!(standing(&sess), reference);
        // the tap-back to the "3": the clock is 0, nothing is driven yet
        pb.seek(&mut sess, &demo, 0, 0, 0.0);
        assert_eq!((standing(&sess).0, sess.race.clock_ms), (1, 0.0));
        assert!(sess.race.countdown > 5.9 && sess.race.countdown < 6.1, "{}", sess.race.countdown);
    }

    #[test]
    fn the_camera_key_reaches_the_tv_camera_where_there_is_one() {
        let cycle = |has_tv: bool| {
            let mut v = vec![CamMode::Chase];
            while v.len() < 8 && (v.len() < 2 || v[v.len() - 1] != CamMode::Chase) {
                v.push(next_camera(v[v.len() - 1], has_tv));
            }
            v
        };
        use CamMode::*;
        assert_eq!(cycle(true), [Chase, FarChase, InCar, Bumper, Tv, Chase]);
        assert_eq!(cycle(false), [Chase, FarChase, InCar, Bumper, Chase]);
        // from the goal camera (after the finish) the cycle starts again
        assert_eq!(next_camera(Goal, true), Chase);
    }

    #[test]
    fn a_hold_scrubs_and_a_tap_skips_five_seconds() {
        let mut p = pb();
        // held for half a second: x4 backwards, no jump on release
        for _ in 0..30 {
            p.input(&acts(&[Act::Rew]), 1.0 / 60.0, 100_000.0);
        }
        assert_eq!((p.hold, p.speed()), (-1, -4.0));
        let seek = p.input(&acts(&[]), 1.0 / 60.0, 100_000.0);
        assert_eq!((p.hold, seek), (0, None));
        // a tap: 3 frames, then skip 5 s from where it began
        let mut p = pb();
        for _ in 0..3 {
            p.input(&acts(&[Act::Fwd]), 1.0 / 60.0, 50_000.0);
        }
        let seek = p.input(&acts(&[]), 1.0 / 60.0, 50_200.0);
        assert_eq!(seek, Some(55_000.0));
    }

    #[test]
    fn q_within_a_second_and_a_half_of_a_lap_start_goes_to_the_lap_before() {
        let mut p = pb();
        let s = p.input(&acts(&[Act::Prev]), 0.016, 71_208.0 + 1000.0);
        assert_eq!(s, Some(6000.0));
        let mut p = pb();
        let s = p.input(&acts(&[Act::Prev]), 0.016, 71_208.0 + 3000.0);
        assert_eq!(s, Some(71_208.0));
        let mut p = pb();
        let s = p.input(&acts(&[Act::Next]), 0.016, 71_208.0 + 3000.0);
        assert_eq!(s, Some(136_079.0));
        // the "3" is the first mark
        let mut p = pb();
        let s = p.input(&acts(&[Act::Prev]), 0.016, 3000.0);
        assert_eq!(s, Some(0.0));
    }

    #[test]
    fn speeds_step_and_stop_at_the_ends() {
        let mut p = pb();
        for _ in 0..6 {
            p.input(&acts(&[Act::Faster]), 0.016, 0.0);
            p.input(&acts(&[]), 0.016, 0.0);
        }
        assert_eq!(RATES[p.rate], 4.0);
        for _ in 0..8 {
            p.input(&acts(&[Act::Slower]), 0.016, 0.0);
            p.input(&acts(&[]), 0.016, 0.0);
        }
        assert_eq!(RATES[p.rate], 0.25);
    }

    #[test]
    fn at_the_end_left_right_pick_and_enter_decides() {
        let mut p = pb();
        p.ended = true;
        p.playing = false;
        p.input(&acts(&[Act::Fwd]), 0.016, 200_000.0);
        assert_eq!(p.picked, 1);
        p.input(&acts(&[Act::Confirm]), 0.016, 200_000.0);
        assert!(p.leave);
        let mut p = pb();
        p.ended = true;
        p.playing = false;
        let s = p.input(&acts(&[Act::Confirm]), 0.016, 200_000.0);
        assert_eq!((s, p.leave, p.ended, p.playing), (Some(0.0), false, false, true));
    }

    #[test]
    fn the_controls_hide_after_the_timeout_unless_paused() {
        let mut p = pb();
        p.touch();
        assert!(p.visible());
        for _ in 0..200 {
            p.ease(0.016, true);
        }
        assert!(!p.visible());
        p.playing = false;
        assert!(p.visible());
    }

    /// One frame of the pointer then the keys, as `Game::drive` does; returns the seek.
    fn frame(p: &mut Playback, hits: &[(Target, [f32; 4])], at: Option<[f32; 2]>, down: bool, t_ms: f64) -> Option<f64> {
        let mut held = HashSet::new();
        let scrub = p.mouse(hits, at, down, &mut held);
        scrub.or(p.input(&held, 1.0 / 60.0, t_ms))
    }

    const REW: (Target, [f32; 4]) = (Target::Rew, [40.0, 10.0, 30.0, 30.0]);
    const BAR: (Target, [f32; 4]) = (Target::Bar, [100.0, 100.0, 200.0, 10.0]);

    #[test]
    fn a_held_press_scrubs_and_a_click_skips_five_seconds() {
        let (hits, on) = ([REW], Some([50.0, 20.0]));
        let mut p = pb();
        for _ in 0..30 {
            assert_eq!(frame(&mut p, &hits, on, true, 100_000.0), None);
        }
        assert_eq!((p.hold, p.speed()), (-1, -4.0));
        assert_eq!(frame(&mut p, &hits, on, false, 100_000.0), None);
        assert_eq!(p.hold, 0);
        // a click: 3 frames
        let mut p = pb();
        for _ in 0..3 {
            frame(&mut p, &hits, on, true, 50_000.0);
        }
        assert_eq!(frame(&mut p, &hits, on, false, 50_200.0), Some(45_000.0));
    }

    #[test]
    fn dragging_the_bar_seeks_to_the_pointer_and_holds_the_clock() {
        let mut p = pb();
        let hits = [BAR];
        // pointing at the bar tells the moment, with no press
        assert_eq!(frame(&mut p, &hits, Some([200.0, 105.0]), false, 0.0), None);
        assert_eq!((p.mouse.tip, p.mouse.scrub), (Some(100_000.0), false));
        // the press seeks there and holds the clock; moving off the rectangle keeps the drag (clamped)
        assert_eq!(frame(&mut p, &hits, Some([200.0, 105.0]), true, 0.0), Some(100_000.0));
        assert!(p.mouse.scrub && p.speed() == 0.0 && p.visible() && p.silent());
        assert_eq!(frame(&mut p, &hits, Some([250.0, 300.0]), true, 0.0), Some(150_000.0));
        assert_eq!(frame(&mut p, &hits, Some([900.0, 300.0]), true, 0.0), Some(200_000.0));
        // no move, no seek
        assert_eq!(frame(&mut p, &hits, Some([900.0, 300.0]), true, 0.0), None);
        assert_eq!(frame(&mut p, &hits, Some([900.0, 300.0]), false, 0.0), None);
        assert!(!p.mouse.scrub && p.playing && p.speed() > 0.0);
    }

    #[test]
    fn the_speed_chip_wraps_and_a_hidden_card_only_wakes() {
        let mut p = pb();
        p.rate = RATES.len() - 1;
        let hits = [(Target::Speed, [0.0, 0.0, 20.0, 20.0])];
        frame(&mut p, &hits, Some([5.0, 5.0]), true, 0.0);
        assert_eq!(p.rate, 0);
        frame(&mut p, &hits, Some([5.0, 5.0]), false, 0.0);
        // no rectangles (the card is gone): the press touches and nothing else
        let mut p = pb();
        p.active = 0.0;
        frame(&mut p, &[], Some([5.0, 5.0]), true, 0.0);
        assert!(p.visible() && p.mouse.press.is_none() && p.rate == NORMAL && p.playing);
    }

    #[test]
    fn pointing_at_the_end_choice_picks_and_a_click_decides() {
        let mut p = pb();
        p.ended = true;
        p.playing = false;
        let hits = [(Target::Pick(0), [0.0, 0.0, 50.0, 20.0]), (Target::Pick(1), [60.0, 0.0, 50.0, 20.0])];
        frame(&mut p, &hits, Some([70.0, 5.0]), false, 0.0);
        assert_eq!(p.picked, 1);
        frame(&mut p, &hits, Some([70.0, 5.0]), true, 0.0);
        assert!(p.leave);
        let mut p = pb();
        p.ended = true;
        p.playing = false;
        frame(&mut p, &hits, Some([10.0, 5.0]), false, 0.0);
        assert_eq!(frame(&mut p, &hits, Some([10.0, 5.0]), true, 0.0), Some(0.0));
        assert!(!p.ended && p.playing);
    }

    #[test]
    fn the_camera_chip_asks_for_the_next_view_once() {
        let mut p = pb();
        let hits = [(Target::Camera, [0.0, 0.0, 20.0, 20.0])];
        for _ in 0..5 {
            frame(&mut p, &hits, Some([5.0, 5.0]), true, 0.0);
        }
        assert!(std::mem::take(&mut p.mouse.camera));
        frame(&mut p, &hits, Some([5.0, 5.0]), true, 0.0);
        assert!(!p.mouse.camera);
    }
}
