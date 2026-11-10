//! Race rules: centreline progress, checkpoints, laps, wrong-way, grid, standings.
//!
//! Recovered from the per-car progress update 0x425b44 and its init 0x426194
//! (re-notes/04-track-car-data.md §4.6). The track's centreline is the closed LightWave
//! spline named by `SplineName`; key channel 4 holds checkpoint numbers (1, 2).

use glam::Vec3;
use motor_data::cfg::{self, Motion};
use motor_data::fs;
use std::f32::consts::{PI, TAU};
use std::path::Path;

/// Spline samples per key interval (the game's group index is sample >> 4).
const SAMPLES_PER_KEY: usize = 16;
/// Largest forward key step that still extends the visited chain (= the projection's forward
/// search window of 256 samples).
const MAX_KEY_SKIP: usize = 16;
motor_tune::tunable! {
    /// Wrong-way timer: the wrong-way warning shows once the timer is above this (0x425b44) [O].
    /// range: 0..10000
    /// unit: ms
    WRONG_WAY_ON_MS: f32 = 2000.0;
    /// Wrong-way timer: it is capped at this, so the warning goes out soon after turning round (0x425b44) [O].
    /// range: 0..10000
    /// unit: ms
    WRONG_WAY_CAP_MS: f32 = 2500.0;
}

/// Closed Catmull-Rom centreline with arc length (LightWave TCB with t = c = b = 0).
pub struct Centreline {
    pts: Vec<Vec3>,
    /// arc length at each sample
    s: Vec<f32>,
    pub length: f32,
    /// key intervals ("groups" in the visit rule)
    pub groups: usize,
    /// (arc length, checkpoint number)
    pub checkpoints: Vec<(f32, u32)>,
    /// The key channel 3 ("heading" slot) along the line, one value per sample: the zone's echo strength
    /// 0..1. The original copies it from the spline tracker to the car's progress struct (`0x425bab`,
    /// S+0x38) and from there to P+0xc28 (`0x45b570`), the `b` of the engine sound (07-audio 7.2) [O].
    echo: Vec<f32>,
    /// The key channels 6-8 (`sx sy sz`) along the line, one value per sample: the tint the original
    /// multiplies into a car's or a loose prop's ambient and sun light (`0x425b96` -> S+0x3c ->
    /// object +0xb8, used by `0x439134`) [O]. 1.0 where a key has no such channel.
    tint: Vec<Vec3>,
    pub finish_point: Vec3,
    pub finish_dir: Vec3,
}

/// Channels 6-8 of a key; 1.0 when the file has fewer channels.
fn tint_of(v: &[f32]) -> Vec3 {
    Vec3::new(v.get(6).copied().unwrap_or(1.0), v.get(7).copied().unwrap_or(1.0), v.get(8).copied().unwrap_or(1.0))
}

fn catmull(p0: Vec3, p1: Vec3, p2: Vec3, p3: Vec3, t: f32) -> Vec3 {
    let t2 = t * t;
    let t3 = t2 * t;
    0.5 * ((2.0 * p1) + (p2 - p0) * t + (2.0 * p0 - 5.0 * p1 + 4.0 * p2 - p3) * t2 + (3.0 * p1 - p0 - 3.0 * p2 + p3) * t3)
}

impl Centreline {
    pub fn from_motion(m: &Motion) -> Result<Centreline, String> {
        let mut keys: Vec<(Vec3, f32, f32, Vec3)> = m
            .keys
            .iter()
            .map(|k| (Vec3::new(k.values[0], k.values[1], k.values[2]), k.values.get(4).copied().unwrap_or(0.0), k.values.get(3).copied().unwrap_or(0.0), tint_of(&k.values)))
            .collect();
        if keys.len() > 2 && keys[0].0.distance(keys[keys.len() - 1].0) < 0.01 {
            keys.pop(); // closed: last key repeats the first
        }
        let n = keys.len();
        if n < 4 {
            return Err("centreline spline has fewer than 4 keys".into());
        }
        let p = |i: usize| keys[i % n].0;
        let mut pts = Vec::with_capacity(n * SAMPLES_PER_KEY);
        for i in 0..n {
            for k in 0..SAMPLES_PER_KEY {
                pts.push(catmull(p(i + n - 1), p(i), p(i + 1), p(i + 2), k as f32 / SAMPLES_PER_KEY as f32));
            }
        }
        let echo = (0..n)
            .flat_map(|i| {
                let e = |j: usize| Vec3::new(keys[j % n].2, 0.0, 0.0);
                (0..SAMPLES_PER_KEY).map(move |k| catmull(e(i + n - 1), e(i), e(i + 1), e(i + 2), k as f32 / SAMPLES_PER_KEY as f32).x)
            })
            .collect();
        let tint = (0..n)
            .flat_map(|i| (0..SAMPLES_PER_KEY).map(move |k| (i, k)))
            .map(|(i, k)| catmull(keys[(i + n - 1) % n].3, keys[i].3, keys[(i + 1) % n].3, keys[(i + 2) % n].3, k as f32 / SAMPLES_PER_KEY as f32))
            .collect();
        let mut s = Vec::with_capacity(pts.len());
        let mut acc = 0.0;
        for i in 0..pts.len() {
            s.push(acc);
            acc += pts[i].distance(pts[(i + 1) % pts.len()]);
        }
        let checkpoints = keys
            .iter()
            .enumerate()
            .filter(|(_, k)| k.1.round() >= 1.0)
            .map(|(i, k)| (s[i * SAMPLES_PER_KEY], k.1.round() as u32))
            .collect();
        let finish_dir = (pts[1] - pts[0]).normalize_or(Vec3::Z);
        Ok(Centreline { finish_point: pts[0], finish_dir, pts, s, length: acc, groups: n, checkpoints, echo, tint })
    }

    /// Arc length at key `k` (the lap's length for the key after the last).
    fn key_arc(&self, k: usize) -> f32 {
        if k >= self.groups { self.length } else { self.s[k * SAMPLES_PER_KEY] }
    }

    /// Point and direction at arc length `s`.
    pub fn at(&self, s: f32) -> (Vec3, Vec3) {
        let s = s.rem_euclid(self.length);
        let i = self.s.partition_point(|&x| x <= s).saturating_sub(1);
        let j = (i + 1) % self.pts.len();
        let seg = self.pts[j] - self.pts[i];
        let t = ((s - self.s[i]) / seg.length().max(1e-6)).clamp(0.0, 1.0);
        (self.pts[i] + seg * t, seg.normalize_or(self.finish_dir))
    }

    /// Echo strength 0..1 at arc length `s` (`b` of the car sound update, clamped as at `0x483d8a`) [O].
    // CLASSIC 0x425bab -> S+0x38 -> P+0xc28 (0x45b570)
    pub fn echo_at(&self, s: f32) -> f32 {
        let s = s.rem_euclid(self.length);
        let i = self.s.partition_point(|&x| x <= s).saturating_sub(1);
        let j = (i + 1) % self.pts.len();
        let next = if j == 0 { self.length } else { self.s[j] };
        let t = ((s - self.s[i]) / (next - self.s[i]).max(1e-6)).clamp(0.0, 1.0);
        (self.echo[i] + (self.echo[j] - self.echo[i]) * t).clamp(0.0, 1.0)
    }

    /// The `sx sy sz` tint at arc length `s`, not clamped (the light is, later, `0x439134`) [O].
    // CLASSIC 0x425b96 -> S+0x3c -> object +0xb8
    pub fn tint_at(&self, s: f32) -> Vec3 {
        let s = s.rem_euclid(self.length);
        let i = self.s.partition_point(|&x| x <= s).saturating_sub(1);
        let j = (i + 1) % self.pts.len();
        let next = if j == 0 { self.length } else { self.s[j] };
        let t = ((s - self.s[i]) / (next - self.s[i]).max(1e-6)).clamp(0.0, 1.0);
        self.tint[i].lerp(self.tint[j], t)
    }

    /// Nearest sample to `p` (searching around `hint`, or everywhere when `None`) and its arc length.
    pub fn project(&self, p: Vec3, hint: Option<usize>) -> (usize, f32) {
        let n = self.pts.len();
        let d2 = |i: usize| self.pts[i % n].distance_squared(p);
        let best = match hint {
            Some(h) => (h + n - 64..h + n + 256).min_by(|&a, &b| d2(a).total_cmp(&d2(b))).unwrap() % n,
            None => (0..n).min_by(|&a, &b| d2(a).total_cmp(&d2(b))).unwrap(),
        };
        // refine along the following segment
        let (a, b) = (self.pts[best], self.pts[(best + 1) % n]);
        let seg = b - a;
        let t = ((p - a).dot(seg) / seg.length_squared().max(1e-6)).clamp(0.0, 1.0);
        (best, self.s[best] + t * seg.length())
    }
}

/// The centreline of `trk` (a parsed `Game/TrackN.trk`): its `SplineName` motion file.
pub fn load_centreline(root: &Path, trk: &cfg::Config) -> Result<Centreline, String> {
    let name = trk.get("SplineName").and_then(|e| e.values.first().cloned()).ok_or("track file: no SplineName")?;
    let motion = cfg::parse_lwmo(&motor_data::tbl::load(&fs::read(root, &name)?, None).map_err(|e| e.to_string())?.data).map_err(|e| e.to_string())?;
    Centreline::from_motion(&motion)
}

/// REMASTER track screen (#294): the track's lap as the Glass HUD shows it, derived from the
/// centreline [I] design choice (the original's track screen shows only the line object).
#[derive(Debug, Clone, PartialEq)]
pub struct TrackProfile {
    /// centreline length (m)
    pub length: f32,
    /// height above the lap's lowest point at `ELEVATION_SAMPLES` even arc lengths (m)
    pub elevation: Vec<f32>,
    /// the lap (game space) every `OUTLINE_STEP` m, closed (the first point is not repeated),
    /// with each point's arc length in `outline_s`
    pub outline: Vec<[f32; 3]>,
    pub outline_s: Vec<f32>,
    /// arc lengths where sectors 2 and 3 begin (checkpoints 1 and 2; thirds of the lap without them)
    pub sectors: [f32; 2],
    /// the turns in lap order: the apex's arc length, its point and the bend's outward direction (x, z, unit)
    pub turns: Vec<Turn>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Turn {
    pub s: f32,
    /// the bend's total heading change (degrees, unsigned)
    pub deg: f32,
    pub apex: [f32; 3],
    pub outward: [f32; 2],
}

// The track profile (`Centreline::profile`: elevation graph, outline, corners) is built when a race is built.
motor_tune::tunable! {
    /// Points of the track's elevation graph (design choice).
    /// range: 8..256
    /// applies: restart
    pub ELEVATION_SAMPLES: usize = 64;
    /// Distance between the points of the track's outline (map) (design choice).
    /// range: 2..50
    /// unit: m
    /// applies: restart
    OUTLINE_STEP: f32 = 10.0;
    /// Turn detection: a heading change over this window above `TURN_DEG` is a bend (80 m also sees the
    /// long gentle bends a 40 m window slides past) (design choice).
    /// range: 10..200
    /// unit: m
    /// applies: restart
    TURN_WINDOW: f32 = 80.0;
    /// Turn detection: the heading change over `TURN_WINDOW` that makes a bend (design choice).
    /// range: 5..90
    /// unit: °
    /// applies: restart
    TURN_DEG: f32 = 22.0;
    /// Turn detection: bends of one sense closer than this are one turn (a double apex or a bend that eases off in
    /// the middle) (design choice).
    /// range: 0..100
    /// unit: m
    /// applies: restart
    TURN_GAP: f32 = 30.0;
    /// Turn detection: the heading is sampled every this far along the track (design choice).
    /// range: 1..20
    /// unit: m
    /// applies: restart
    TURN_STEP: f32 = 5.0;
    /// Turn detection: a merged bend whose total heading change is under this is a kink, not a corner (design choice).
    /// range: 5..120
    /// unit: °
    /// applies: restart
    TURN_MIN_DEG: f32 = 25.0;
    /// Turn detection: the chord whose midpoint lies inside a bend; outward is away from it (design choice).
    /// range: 5..200
    /// unit: m
    /// applies: restart
    TURN_CHORD: f32 = 40.0;
}

impl Centreline {
    /// The samples (game space) and their arc lengths.
    pub fn samples(&self) -> &[Vec3] {
        &self.pts
    }

    pub fn arc(&self) -> &[f32] {
        &self.s
    }

    /// Arc lengths where sectors 2 and 3 begin: checkpoints 1 and 2, thirds of the lap without them
    /// (OkkunSpeedway has no checkpoint 2).
    pub fn sector_arcs(&self) -> [f32; 2] {
        let len = self.length.max(1.0);
        self.sector_checkpoints().unwrap_or([len / 3.0, len * 2.0 / 3.0])
    }

    /// The arc lengths of checkpoints 1 and 2 when both exist in lap order (the sector bounds then
    /// have a split time in `Progress::splits`), else `None`.
    pub fn sector_checkpoints(&self) -> Option<[f32; 2]> {
        let cp = |k: u32| self.checkpoints.iter().find(|c| c.1 == k).map(|c| c.0);
        match (cp(1), cp(2)) {
            (Some(a), Some(b)) if a < b => Some([a, b]),
            _ => None,
        }
    }

    pub fn profile(&self) -> TrackProfile {
        let len = self.length.max(1.0);
        let heights: Vec<f32> = (0..ELEVATION_SAMPLES.get()).map(|i| self.at(len * i as f32 / ELEVATION_SAMPLES.get() as f32).0.y).collect();
        let low = heights.iter().copied().fold(f32::INFINITY, f32::min);
        let elevation = heights.iter().map(|h| h - low).collect();
        let n_out = (len / OUTLINE_STEP.get()).round().max(8.0) as usize;
        let outline_s: Vec<f32> = (0..n_out).map(|i| len * i as f32 / n_out as f32).collect();
        let outline = outline_s
            .iter()
            .map(|&s| self.at(s).0.to_array())
            .collect();
        let sectors = self.sector_arcs();
        // the heading change over the window at every step (signed, -pi..pi)
        let n = (len / TURN_STEP.get()).ceil().max(4.0) as usize;
        // from the chord of the points, not the spline's tangent: a key where the tangent flips (Nolby Hills
        // at 965 m) would read as a half turn
        let heading = |s: f32| {
            let d = self.at(s + TURN_STEP.get() * 0.5).0 - self.at(s - TURN_STEP.get() * 0.5).0;
            d.x.atan2(d.z)
        };
        let at = |i: usize| len * (i % n) as f32 / n as f32;
        let bend: Vec<f32> = (0..n).map(|i| (heading(at(i) + TURN_WINDOW.get() * 0.5) - heading(at(i) - TURN_WINDOW.get() * 0.5) + PI).rem_euclid(TAU) - PI).collect();
        let thr = TURN_DEG.get().to_radians();
        // runs of one sense above the threshold, walked from a straight sample so no run wraps
        let start = (0..n).find(|&i| bend[i].abs() <= thr);
        let mut runs: Vec<(usize, usize, f32)> = Vec::new(); // first sample, last sample (inclusive, unwrapped), sense
        if let Some(k) = start {
            let mut open: Option<(usize, usize, f32)> = None;
            for j in k..k + n {
                let b = bend[j % n];
                let sense = if b.abs() > thr { b.signum() } else { 0.0 };
                match open {
                    Some((a, _, s)) if sense == s => open = Some((a, j, s)),
                    _ => {
                        if let Some(r) = open.take() {
                            runs.push(r);
                        }
                        if sense != 0.0 {
                            open = Some((j, j, sense));
                        }
                    }
                }
            }
            runs.extend(open);
        } else {
            runs.push((0, n - 1, bend[0].signum()));
        }
        // bends of one sense separated by less than the gap are one turn
        let gap = (TURN_GAP.get() / TURN_STEP.get()).round() as usize;
        let mut merged: Vec<(usize, usize, f32)> = Vec::new();
        for r in runs {
            match merged.last_mut() {
                Some(m) if m.2 == r.2 && r.0 <= m.1 + gap + 1 => m.1 = r.1,
                _ => merged.push(r),
            }
        }
        // the heading changes of a run, step by step from the start of its window to the end of it (signed, a hairpin
        // wraps past 180 degrees in a single difference)
        let steps = |a: usize, b: usize| -> Vec<(f32, f32)> {
            let (s0, s1) = (at(a) - TURN_WINDOW.get() * 0.5, at(b) + TURN_WINDOW.get() * 0.5);
            let k = (((s1 - s0) / TURN_STEP.get()).ceil() as usize).max(1);
            let p = |i: usize| s0 + (s1 - s0) * i as f32 / k as f32;
            (0..k).map(|i| (p(i + 1), (heading(p(i + 1)) - heading(p(i)) + PI).rem_euclid(TAU) - PI)).collect()
        };
        let total = |a: usize, b: usize| steps(a, b).iter().map(|x| x.1).sum::<f32>().to_degrees().abs();
        // the apex is where half of the bend's heading change is done (the middle of a long bend, the point of a corner)
        let apex = |a: usize, b: usize| -> f32 {
            let st = steps(a, b);
            let half = st.iter().map(|x| x.1).sum::<f32>() * 0.5;
            let mut acc = 0.0;
            st.iter().find(|x| {
                acc += x.1;
                acc.abs() >= half.abs()
            }).map_or(at(a), |x| x.0)
        };
        let mut turns: Vec<Turn> = merged
            .iter()
            .map(|&(a, b, _)| (a, b, total(a, b)))
            .filter(|&(_, _, deg)| deg >= TURN_MIN_DEG.get())
            .map(|(a, b, deg)| {
                let s = apex(a, b);
                let p = self.at(s).0;
                let (q0, q1) = (self.at(s - TURN_CHORD.get() * 0.5).0, self.at(s + TURN_CHORD.get() * 0.5).0);
                // the chord's midpoint lies inside the bend: outward is away from it
                let inward = Vec3::new((q0.x + q1.x) * 0.5 - p.x, 0.0, (q0.z + q1.z) * 0.5 - p.z).normalize_or(Vec3::X);
                Turn { s, deg, apex: p.to_array(), outward: [-inward.x, -inward.z] }
            })
            .collect();
        turns.sort_by(|a, b| a.s.total_cmp(&b.s));
        TrackProfile { length: len, elevation, outline, outline_s, sectors, turns }
    }
}

/// `StartGrid Row Goal Space Tint` from the .trk (grid placement in 0x426xxx).
#[derive(Debug, Clone, Copy)]
pub struct Grid {
    pub row: f32,
    pub goal: f32,
    pub space: f32,
    pub tint: f32,
}

impl Default for Grid {
    fn default() -> Self {
        // code defaults before parsing (0x42194c)
        Grid { row: 10.0, goal: 10.0, space: 2.5, tint: 2.0 }
    }
}

impl Grid {
    /// Slot 0 is pole. Rows of two behind the finish point; odd slots staggered back by `tint`.
    pub fn slot(&self, line: &Centreline, slot: usize) -> (Vec3, Vec3) {
        let dir = line.finish_dir;
        let mut along = -self.row * (slot / 2) as f32 - self.goal;
        if slot & 1 == 1 {
            along -= self.tint;
        }
        let lateral = Vec3::new(dir.z, dir.y, -dir.x) * if slot & 1 == 0 { -self.space } else { self.space };
        (line.finish_point + dir * along + lateral, dir)
    }
}

/// Per-car race progress (the struct at car+0x1a75 in the original).
#[derive(Debug, Clone)]
pub struct Progress {
    /// current lap, starting at 1
    pub lap: u32,
    pub laps: u32,
    visited: Vec<bool>,
    visited_count: usize,
    last_group: usize,
    sample: Option<usize>,
    pub s: f32,
    /// centreline length (for the grid ranking)
    length: f32,
    wrong_ms: f32,
    pub wrong_way: bool,
    finish_side_prev: f32,
    /// race time (ms) at each split: index lap*4 + 0 (lap start), + cp + 1
    pub splits: Vec<Option<u32>>,
    pub lap_start_ms: u32,
    pub last_lap_ms: Option<u32>,
    pub best_lap_ms: Option<u32>,
    pub finished_ms: Option<u32>,
    /// Knockout (our design): race time (ms) at which this car was eliminated
    pub eliminated_ms: Option<u32>,
    /// (checkpoint number, race time) of the most recent checkpoint
    pub last_checkpoint: Option<(u32, u32)>,
    /// REMASTER live gap (#299, design choice [I], not in the original): race time (ms) at which
    /// the car first passed each key of each lap, index lap * groups + key. Written beside the
    /// ranking's key chain, read by `live_gap`; the checkpoint `splits` stay as they are.
    pass_ms: Vec<Option<f32>>,
}

/// When the cars were where, for `Progress::mark_keys` to time a key's pass inside the frame.
struct Stamp {
    now: f32,
    dt: f32,
    /// arc length before and after the move
    s0: f32,
    s1: f32,
}

/// REMASTER live gap (#299): a car's gap to the car in front of it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Gap {
    /// ms behind
    Time(u32),
    /// a whole lap or more behind
    Laps(u32),
}

impl Progress {
    pub fn new(line: &Centreline, laps: u32, pos: Vec3) -> Progress {
        let (sample, s) = line.project(pos, None);
        Progress {
            lap: 1,
            laps,
            visited: vec![false; line.groups],
            visited_count: 0,
            last_group: sample / SAMPLES_PER_KEY,
            sample: Some(sample),
            s,
            length: line.length.max(1e-6),
            wrong_ms: 0.0,
            wrong_way: false,
            finish_side_prev: (pos - line.finish_point).dot(line.finish_dir),
            splits: vec![None; (laps as usize + 2) * 4],
            lap_start_ms: 0,
            last_lap_ms: None,
            best_lap_ms: None,
            finished_ms: None,
            eliminated_ms: None,
            last_checkpoint: None,
            pass_ms: vec![None; (laps as usize + 2) * line.groups],
        }
    }

    /// Finished the race or knocked out: the car no longer races.
    pub fn done(&self) -> bool {
        self.finished_ms.is_some() || self.eliminated_ms.is_some()
    }

    /// Compact internal state for traces.
    pub fn debug(&self) -> String {
        format!("lap {} s {:.0} group {} visited {}/{}", self.lap, self.s, self.last_group, self.visited_count, self.visited.len())
    }

    /// Ranking score: laps dominate, then keys passed in order, then distance inside the key.
    pub fn score(&self) -> f64 {
        let g = self.visited.len() as f64;
        if self.visited_count == 0 {
            // Design fix [I]: behind the line (the grid) no key is visited yet and the grid spans
            // several keys, so rank by the arc length on the whole lap, in (-1, 0), below any car
            // that has crossed (the original ranks via its spline tracker, 0x456ab0).
            return self.lap as f64 * g * 2.0 + (self.s / self.length) as f64 - 1.0;
        }
        let frac = self.sample.map_or(0.0, |s| (s % SAMPLES_PER_KEY) as f64 / SAMPLES_PER_KEY as f64);
        self.lap as f64 * g * 2.0 + self.visited_count.min(self.visited.len()) as f64 + frac
    }

    /// Advance the in-order key chain to group `g`. `last_group` is the head of the chain.
    /// Design choice [I]: the original walks its spline tracker key by key (0x456ab0); our
    /// nearest-sample projection can hop several keys when the car is off the line (shortcuts,
    /// respawns), so a forward step of up to `MAX_KEY_SKIP` keys marks the keys in between, and a
    /// larger backward hop leaves the head where it is. Each key is counted once.
    fn mark_keys(&mut self, line: &Centreline, g: usize, st: &Stamp) {
        let groups = self.visited.len();
        if g == 0 && !self.visited[0] {
            self.visited[0] = true;
            self.visited_count += 1;
            self.stamp(line, 0, st);
        }
        let head = self.last_group;
        if !self.visited[head] {
            self.last_group = g; // no chain yet (before key 0): just follow the car
            return;
        }
        let step = (g + groups - head) % groups;
        if (1..=MAX_KEY_SKIP).contains(&step) {
            for i in 1..=step {
                let k = (head + i) % groups;
                if !self.visited[k] {
                    self.visited[k] = true;
                    self.visited_count += 1;
                    self.stamp(line, k, st);
                }
            }
            self.last_group = g;
        } else if step == groups - 1 {
            // moving back one key unmarks the key left [O]
            self.visited[head] = false;
            self.visited_count = self.visited_count.saturating_sub(1);
            self.last_group = g;
        }
    }

    /// Called after a racer is moved by `respawn`: follow the car along the line so the key chain
    /// is not broken. The finish-plane side is left alone, so a jump over the line still counts.
    /// The centreline sample last matched, as a `Centreline::project` hint.
    pub fn sample(&self) -> Option<usize> {
        self.sample
    }

    pub fn teleported(&mut self, line: &Centreline, pos: Vec3, now_ms: u32) {
        let (sample, s) = line.project(pos, self.sample);
        self.sample = Some(sample);
        // the keys it jumps over are timed now
        let st = Stamp { now: now_ms as f32, dt: 0.0, s0: self.s, s1: s };
        self.s = s;
        self.mark_keys(line, sample / SAMPLES_PER_KEY, &st);
    }

    /// REMASTER (#299): note the time at which key `k` of this lap was reached, within the
    /// frame by the arc length the car covered (the first pass only).
    fn stamp(&mut self, line: &Centreline, k: usize, st: &Stamp) {
        let i = self.lap as usize * line.groups + k;
        let Some(slot) = self.pass_ms.get_mut(i).filter(|s| s.is_none()) else { return };
        let wrap = |d: f32| if d > line.length * 0.5 { d - line.length } else if d < -line.length * 0.5 { d + line.length } else { d };
        let span = wrap(st.s1 - st.s0);
        let f = if span > 1e-3 { (wrap(line.key_arc(k) - st.s0) / span).clamp(0.0, 1.0) } else { 1.0 };
        *slot = Some(st.now - st.dt * (1.0 - f));
    }

    /// REMASTER (#299): (lap, key, fraction of the key) of the car, `None` before the line.
    fn place(&self, line: &Centreline) -> Option<(usize, usize, f32)> {
        if self.visited_count == 0 {
            return None;
        }
        let g = line.groups;
        let k = self.sample? / SAMPLES_PER_KEY;
        // the nearest sample is already past the line but the lap is not counted yet
        if self.visited_count >= g && k == 0 {
            return Some((self.lap as usize, g - 1, 1.0));
        }
        let (a0, a1) = (line.key_arc(k), line.key_arc(k + 1));
        Some((self.lap as usize, k, ((self.s - a0) / (a1 - a0).max(1e-3)).clamp(0.0, 1.0)))
    }

    /// REMASTER (#299): metres driven since the line (negative on the grid).
    fn distance(&self, line: &Centreline) -> Option<f32> {
        if self.finished_ms.is_some() {
            return Some(self.laps as f32 * line.length);
        }
        let Some((lap, k, f)) = self.place(line) else { return Some(self.s - line.length) };
        let (a0, a1) = (line.key_arc(k), line.key_arc(k + 1));
        Some((lap - 1) as f32 * line.length + a0 + f * (a1 - a0))
    }

    /// REMASTER (#299): race time (ms) at which this car was at (lap, key, fraction) `at`.
    fn time_at(&self, line: &Centreline, at: (usize, usize, f32), now_ms: f32) -> Option<f32> {
        let (lap, key, frac) = at;
        let g = line.groups;
        let t0 = (*self.pass_ms.get(lap * g + key)?)?;
        let next = if key + 1 < g { lap * g + key + 1 } else { (lap + 1) * g };
        if let Some(t1) = self.pass_ms.get(next).copied().flatten() {
            return Some(t0 + (t1 - t0) * frac);
        }
        // still inside this key: at the speed it has had since entering it
        let (l, k, f) = self.place(line)?;
        if (l, k) != (lap, key) {
            return None;
        }
        Some(if f > 1e-3 { t0 + (now_ms - t0) * (frac / f).min(1.0) } else { t0 })
    }

    /// REMASTER (#94): race time (ms) at which this car passed arc length `s` of `lap` (1-based),
    /// linear between the pass times of the key holding `s` and the next one; `None` if either is
    /// missing. Used for a sector bound whose checkpoint split was never written (no checkpoint 2,
    /// or a respawn jumped over it).
    pub fn time_at_arc(&self, line: &Centreline, lap: u32, s: f32) -> Option<f32> {
        let g = line.groups;
        let k = (0..g).rev().find(|&k| line.key_arc(k) <= s)?;
        let (a0, a1) = (line.key_arc(k), line.key_arc(k + 1));
        let at = |lap: usize, k: usize| self.pass_ms.get(if k < g { lap * g + k } else { (lap + 1) * g }).copied().flatten();
        let (t0, t1) = (at(lap as usize, k)?, at(lap as usize, k + 1)?);
        Some(t0 + (t1 - t0) * ((s - a0) / (a1 - a0).max(1e-3)).clamp(0.0, 1.0))
    }

    pub fn update(&mut self, line: &Centreline, pos: Vec3, now_ms: u32, dt_ms: f32) {
        if self.done() {
            return;
        }
        let (sample, s) = line.project(pos, self.sample);
        self.sample = Some(sample);
        // wrong way: time accumulates while arc length decreases
        let mut ds = s - self.s;
        if ds > line.length * 0.5 {
            ds -= line.length;
        } else if ds < -line.length * 0.5 {
            ds += line.length;
        }
        self.wrong_ms = (self.wrong_ms + if ds < 0.0 { dt_ms } else { -dt_ms }).clamp(0.0, WRONG_WAY_CAP_MS.get());
        self.wrong_way = self.wrong_ms > WRONG_WAY_ON_MS.get();
        // keys must be passed in order before a lap counts
        let groups = self.visited.len();
        let g = sample / SAMPLES_PER_KEY;
        let st = Stamp { now: now_ms as f32, dt: dt_ms, s0: self.s, s1: s };
        self.mark_keys(line, g, &st);
        // checkpoints: arc length crosses a checkpoint key
        for &(cs, num) in &line.checkpoints {
            if self.s < cs && cs <= s {
                let slot = &mut self.splits[self.lap as usize * 4 + num as usize + 1];
                if slot.is_none() {
                    *slot = Some(now_ms);
                    self.last_checkpoint = Some((num, now_ms));
                }
            }
        }
        self.s = s;
        // finish plane: negative side -> non-negative side, after visiting every key
        let side = (pos - line.finish_point).dot(line.finish_dir);
        if self.finish_side_prev < 0.0 && side >= 0.0 && self.visited_count >= groups {
            let lap_ms = now_ms - self.lap_start_ms;
            self.last_lap_ms = Some(lap_ms);
            if self.best_lap_ms.is_none_or(|b| lap_ms < b) {
                self.best_lap_ms = Some(lap_ms);
            }
            self.lap += 1;
            if let Some(slot) = self.splits.get_mut(self.lap as usize * 4) {
                slot.get_or_insert(now_ms);
            }
            // REMASTER (#299): key 0 of the new lap, at the moment the car crossed the line
            let f = if side - self.finish_side_prev > 1e-6 { -self.finish_side_prev / (side - self.finish_side_prev) } else { 1.0 };
            let i = self.lap as usize * groups;
            if let Some(slot) = self.pass_ms.get_mut(i) {
                slot.get_or_insert(now_ms as f32 - dt_ms * (1.0 - f));
            }
            // the new lap's chain starts at key 0 even if the car is already past it this frame
            self.visited.iter_mut().for_each(|v| *v = false);
            self.visited[0] = true;
            self.visited_count = 1;
            self.last_group = 0;
            self.lap_start_ms = now_ms;
            if self.lap > self.laps {
                self.lap = self.laps;
                self.finished_ms = Some(now_ms);
            }
        }
        self.finish_side_prev = side;
    }
}

/// REMASTER live gap (#299, design choice [I]): how far `behind` is behind `ahead` right now,
/// as the time since `ahead` was where `behind` is, or whole laps when `ahead` has lapped it.
/// `None` when it cannot be told: a car out of the race, `behind` still on the grid, or `ahead`
/// with no time there (it has not been that far).
pub fn live_gap(line: &Centreline, ahead: &Progress, behind: &Progress, now_ms: u32) -> Option<Gap> {
    if ahead.eliminated_ms.is_some() || behind.eliminated_ms.is_some() {
        return None;
    }
    match (ahead.finished_ms, behind.finished_ms) {
        (Some(a), Some(b)) => return Some(Gap::Time(b.saturating_sub(a))),
        (None, Some(_)) => return None,
        _ => {}
    }
    let at = behind.place(line)?;
    let lead = ahead.distance(line)? - behind.distance(line)?;
    if lead >= line.length {
        return Some(Gap::Laps((lead / line.length) as u32));
    }
    let t = ahead.time_at(line, at, now_ms as f32)?;
    Some(Gap::Time((now_ms as f32 - t).max(0.0).round() as u32))
}

/// REMASTER (#299): metres `behind` is behind `ahead` along the track (negative when `behind` leads).
pub fn distance_gap(line: &Centreline, ahead: &Progress, behind: &Progress) -> Option<f32> {
    Some(ahead.distance(line)? - behind.distance(line)?)
}

/// Format milliseconds as m:ss.cc
pub fn fmt_ms(ms: u32) -> String {
    format!("{}:{:02}.{:02}", ms / 60000, (ms / 1000) % 60, (ms % 1000) / 10)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub(crate) fn ring(n: usize) -> Centreline {
        let keys: Vec<Vec3> = (0..n).map(|i| {
            let a = i as f32 / n as f32 * std::f32::consts::TAU;
            Vec3::new(a.cos() * 100.0, 0.0, a.sin() * 100.0)
        }).collect();
        let p = |i: usize| keys[i % n];
        let mut pts = Vec::new();
        for i in 0..n {
            for k in 0..SAMPLES_PER_KEY {
                pts.push(catmull(p(i + n - 1), p(i), p(i + 1), p(i + 2), k as f32 / SAMPLES_PER_KEY as f32));
            }
        }
        let mut s = Vec::new();
        let mut acc = 0.0;
        for i in 0..pts.len() {
            s.push(acc);
            acc += pts[i].distance(pts[(i + 1) % pts.len()]);
        }
        let finish_dir = (pts[1] - pts[0]).normalize();
        Centreline { finish_point: pts[0], finish_dir, echo: vec![0.0; pts.len()], tint: vec![Vec3::ONE; pts.len()], pts, s, length: acc, groups: n, checkpoints: vec![] }
    }

    /// A 400 x 300 m rectangle with chamfered corners and checkpoints on its long sides: the
    /// profile must count its four corners as four turns.
    #[test]
    fn profile_of_a_rectangle() {
        // chamfered corners (two keys 40 m along each side), a key mid-side for the checkpoints
        let corners = [(40.0, 0.0), (200.0, 0.0), (360.0, 0.0), (400.0, 40.0), (400.0, 260.0), (360.0, 300.0), (200.0, 300.0), (40.0, 300.0), (0.0, 260.0), (0.0, 40.0)];
        rectangle_turns(&corners);
    }

    /// The echo channel (key channel 3) is interpolated along the line and clamped to 0..1.
    #[test]
    fn echo_follows_channel_three() {
        let pts = [(0.0, 0.0), (100.0, 0.0), (200.0, 0.0), (300.0, 0.0), (300.0, 100.0), (200.0, 100.0), (100.0, 100.0), (0.0, 100.0)];
        let keys = pts
            .iter()
            .enumerate()
            .map(|(i, &(x, z))| cfg::MotionKey { values: vec![x, 0.0, z, if i == 2 { 0.4 } else { 0.0 }, 0.0], frame: i as f32, linear: 0.0, tension: 0.0, continuity: 0.0, bias: 0.0 })
            .collect();
        let line = Centreline::from_motion(&Motion { version: 1, channels: 9, keys }).unwrap();
        let at_key = line.echo_at(line.key_arc(2));
        assert!((at_key - 0.4).abs() < 1e-3, "echo at the key: {at_key}");
        assert!(line.echo_at(line.key_arc(5)).abs() < 1e-3);
        let mid = line.echo_at((line.key_arc(1) + line.key_arc(2)) * 0.5);
        assert!(mid > 0.0 && mid < 0.4, "between the keys: {mid}");
        for k in 0..400 {
            let e = line.echo_at(k as f32 * line.length / 400.0);
            assert!((0.0..=1.0).contains(&e));
        }
    }

    /// The tint (key channels 6-8) is interpolated along the line, not clamped, and 1.0 without the channels.
    #[test]
    fn tint_follows_channels_six_to_eight() {
        let pts = [(0.0, 0.0), (100.0, 0.0), (200.0, 0.0), (300.0, 0.0), (300.0, 100.0), (200.0, 100.0), (100.0, 100.0), (0.0, 100.0)];
        let mk = |n: usize| -> Vec<cfg::MotionKey> {
            pts.iter()
                .enumerate()
                .map(|(i, &(x, z))| {
                    let mut values = vec![x, 0.0, z, 0.0, 0.0, 0.0];
                    if n > 6 {
                        values.extend(if i == 2 { [0.5, 1.0, 1.3] } else { [1.0; 3] });
                    }
                    cfg::MotionKey { values, frame: i as f32, linear: 0.0, tension: 0.0, continuity: 0.0, bias: 0.0 }
                })
                .collect()
        };
        let line = Centreline::from_motion(&Motion { version: 1, channels: 9, keys: mk(9) }).unwrap();
        let t = line.tint_at(line.key_arc(2));
        assert!((t - Vec3::new(0.5, 1.0, 1.3)).abs().max_element() < 1e-3, "tint at the key: {t}");
        assert!((line.tint_at(line.key_arc(5)) - Vec3::ONE).abs().max_element() < 1e-3);
        let plain = Centreline::from_motion(&Motion { version: 1, channels: 6, keys: mk(6) }).unwrap();
        assert_eq!(plain.tint_at(123.0), Vec3::ONE);
    }

    /// A 28 degree kink in the top side (up and back down within 80 m) is not a corner: still four turns.
    #[test]
    fn kinks_are_not_corners() {
        let corners = [(40.0, 0.0), (200.0, 0.0), (360.0, 0.0), (400.0, 40.0), (400.0, 260.0), (360.0, 300.0), (200.0, 300.0), (120.0, 320.0), (40.0, 300.0), (0.0, 260.0), (0.0, 40.0)];
        rectangle_turns(&corners);
    }

    fn rectangle_turns(corners: &[(f32, f32)]) {
        let keys = corners
            .iter()
            .enumerate()
            .map(|(i, &(x, z))| cfg::MotionKey { values: vec![x, 0.0, z, 0.0, if i == 1 { 1.0 } else if i == 6 { 2.0 } else { 0.0 }], frame: i as f32, linear: 0.0, tension: 0.0, continuity: 0.0, bias: 0.0 })
            .collect();
        let line = Centreline::from_motion(&Motion { version: 1, channels: 9, keys }).unwrap();
        let p = line.profile();
        assert_eq!(p.turns.len(), 4, "turns at the corners: {:?}", p.turns);
        assert_eq!(p.elevation.len(), ELEVATION_SAMPLES.get());
        assert!(p.elevation.iter().all(|&h| h.abs() < 1e-3));
        assert_eq!(p.outline.len(), p.outline_s.len());
        assert!((p.outline.len() as f32 - p.length / OUTLINE_STEP.get()).abs() <= 1.0);
        let (s1, s2) = (line.checkpoints[0].0, line.checkpoints[1].0);
        assert_eq!(p.sectors, [s1, s2]);
        // every turn's outward direction points away from the rectangle's centre
        for t in &p.turns {
            assert!((t.apex[0] - 200.0) * t.outward[0] + (t.apex[2] - 150.0) * t.outward[1] > 0.0, "turn at {} points inward", t.s);
        }
    }

    /// Drive `laps` laps in steps of `step` samples; `skip_at` jumps over a few keys once.
    pub(crate) fn drive(line: &Centreline, laps: u32, skip: bool, teleport: bool) -> Progress {
        let start = line.pts[line.pts.len() - 8];
        let mut pr = Progress::new(line, laps, start);
        let total = line.pts.len();
        let mut t = 0;
        let mut i = 0;
        while pr.finished_ms.is_none() && i < total * (laps as usize + 2) {
            let idx = (total - 8 + i) % total;
            let pos = line.pts[idx];
            if skip && i == total / 2 {
                i += 2 * SAMPLES_PER_KEY; // two keys vanish
                continue;
            }
            if teleport && i == total / 3 {
                let to = line.pts[(idx + 5 * SAMPLES_PER_KEY) % total];
                pr.teleported(line, to, t);
                i += 5 * SAMPLES_PER_KEY;
                continue;
            }
            pr.update(line, pos, t, 16.0);
            t += 16;
            i += 2;
        }
        pr
    }

    #[test]
    fn lap_counts_after_skipped_keys_and_respawn() {
        let line = ring(24);
        for (skip, tp) in [(false, false), (true, false), (false, true)] {
            let pr = drive(&line, 3, skip, tp);
            assert!(pr.finished_ms.is_some(), "skip {skip} teleport {tp}: not finished, {}", pr.debug());
            assert_eq!(pr.lap, 3);
        }
    }

    /// Off the line (a shortcut), the nearest sample hops forward and back by several keys.
    #[test]
    fn lap_counts_through_a_shortcut_with_projection_hops() {
        let line = ring(24);
        let total = line.pts.len();
        let k = |key: usize| key * SAMPLES_PER_KEY + 3;
        let mut path: Vec<usize> = (total - 8..total).chain(0..k(10)).collect();
        path.extend([k(14), k(11), k(15), k(12), k(18)]); // hop ahead, back, ahead
        path.extend(k(18)..total);
        path.extend(0..40); // fast: never lands in key 0's first samples twice
        let mut pr = Progress::new(&line, 2, line.pts[total - 8]);
        for (t, &i) in path.iter().enumerate() {
            pr.update(&line, line.pts[i % total], t as u32 * 16, 16.0);
        }
        assert_eq!(pr.lap, 2, "{}", pr.debug());
    }

    /// On the grid the ranking follows the slots (slot 0 in front), across key boundaries; ties
    /// keep slot order through the stable sort in `standings`.
    #[test]
    fn grid_order_ranks_by_slot() {
        let line = ring(64);
        let scores: Vec<f64> = (0..8).map(|i| Progress::new(&line, 3, Grid::default().slot(&line, i).0).score()).collect();
        assert!(scores.windows(2).all(|w| w[0] >= w[1]), "{scores:?}");
    }

    // ---- live gap (#299)

    /// Cars on the ring at constant speeds (m/s, start delay s) from 20 m before the line; `check`
    /// sees the cars and the clock (ms) after every 16 ms frame. Car `i` also gets `jump_at` ms
    /// ahead by `jump` m once, through `Progress::teleported`.
    fn race_ring(line: &Centreline, laps: u32, cars: &[(f32, f32)], jump: Option<(usize, u32, f32)>, mut check: impl FnMut(&[Progress], u32)) {
        let start = line.length - 20.0;
        let mut arc = vec![start; cars.len()];
        let mut prog: Vec<Progress> = cars.iter().map(|_| Progress::new(line, laps, line.at(start).0)).collect();
        for frame in 1..4000u32 {
            let now = frame * 16;
            for (i, &(v, delay)) in cars.iter().enumerate() {
                if prog[i].done() {
                    continue;
                }
                if now as f32 / 1000.0 > delay {
                    arc[i] += v * 0.016;
                }
                match jump {
                    Some((j, at, m)) if j == i && now == at => {
                        arc[i] += m;
                        prog[i].teleported(line, line.at(arc[i]).0, now);
                    }
                    _ => prog[i].update(line, line.at(arc[i]).0, now, 16.0),
                }
            }
            check(&prog, now);
            if prog.iter().all(|p| p.done()) {
                return;
            }
        }
    }

    /// B leaves 1.5 s after A at the same speed: B is 1.5 s behind from the moment it crosses the
    /// line, every frame, across keys and the finish line.
    #[test]
    fn gap_of_a_late_starter_is_its_head_start() {
        let line = ring(24);
        let (mut some, mut none) = (0, 0);
        race_ring(&line, 3, &[(50.0, 0.0), (50.0, 1.5)], None, |p, now| {
            if p[0].done() || p[1].done() {
                return;
            }
            match live_gap(&line, &p[0], &p[1], now) {
                Some(Gap::Time(ms)) => {
                    some += 1;
                    assert!((ms as i32 - 1500).abs() <= 40, "t {now}: {ms} ms");
                }
                g => {
                    none += 1;
                    assert!(p[1].place(&line).is_none(), "t {now}: {g:?} with {}", p[1].debug());
                }
            }
        });
        assert!(some > 1000 && none > 90, "{some} {none}");
    }

    /// A at 60 m/s, B at 50 m/s from the same start: at B's t seconds A was there at 5/6 t.
    #[test]
    fn gap_grows_with_the_speed_difference() {
        let line = ring(24);
        let mut seen = 0;
        race_ring(&line, 6, &[(60.0, 0.0), (50.0, 0.0)], None, |p, now| {
            let t = now as f32 / 1000.0;
            // B has crossed the line after 0.4 s; A leads by under a lap until t = 12
            if t < 1.0 || t > 11.0 {
                return;
            }
            let Some(Gap::Time(ms)) = live_gap(&line, &p[0], &p[1], now) else { panic!("t {t}: no gap") };
            assert!((ms as f32 - t / 6.0 * 1000.0).abs() <= 40.0, "t {t}: {ms} ms");
            seen += 1;
        });
        assert!(seen > 500);
    }

    /// A laps B: a whole lap behind reads in laps, not as a time; metres tell the same.
    #[test]
    fn a_lapped_car_reads_laps() {
        let line = ring(24);
        let mut at_20s = None;
        race_ring(&line, 10, &[(100.0, 0.0), (50.0, 0.0)], None, |p, now| {
            if now == 20_000 {
                at_20s = Some((live_gap(&line, &p[0], &p[1], now), distance_gap(&line, &p[0], &p[1])));
            }
        });
        let (gap, metres) = at_20s.unwrap();
        assert_eq!(gap, Some(Gap::Laps(1)));
        assert!((metres.unwrap() - 1000.0).abs() < 3.0, "{metres:?}");
    }

    /// A respawn ahead leaves no hole in the table: the gap is still known after it.
    #[test]
    fn gap_survives_a_teleport() {
        let line = ring(24);
        let mut after = 0;
        race_ring(&line, 3, &[(50.0, 0.0), (50.0, 1.5)], Some((1, 8000, 40.0)), |p, now| {
            if p[0].done() || p[1].done() || now < 8000 {
                return;
            }
            let g = live_gap(&line, &p[0], &p[1], now);
            assert!(matches!(g, Some(Gap::Time(_))), "t {now}: {g:?}");
            after += 1;
        });
        assert!(after > 100);
    }

    /// Both home: the difference of their finish times.
    #[test]
    fn finished_cars_gap_is_the_finish_difference() {
        let line = ring(24);
        let mut last = None;
        race_ring(&line, 2, &[(50.0, 0.0), (50.0, 1.5)], None, |p, now| {
            if p[0].finished_ms.is_some() && p[1].finished_ms.is_some() {
                last = Some((p[0].finished_ms.unwrap(), p[1].finished_ms.unwrap(), live_gap(&line, &p[0], &p[1], now)));
            }
        });
        let (a, b, gap) = last.unwrap();
        assert_eq!(gap, Some(Gap::Time(b - a)));
        assert!((b - a).abs_diff(1500) <= 40);
    }
}
