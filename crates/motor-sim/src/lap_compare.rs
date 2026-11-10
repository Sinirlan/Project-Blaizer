//! REMASTER Lap Analysis (#429): two laps of one track side by side, the way a TV qualifying analysis shows them. Every
//! source becomes a `LapTrace` on one distance grid: a Time Attack ghost (`.ghl`), a race replay (`.mhr`, re-simulated,
//! every car of the race) or a demo (`.MDE`, every recorded car). `compare` works out the gap along the lap, the
//! mini-sectors between the turns with their winner and speed class, and each lap's throttle / braking / cornering shares.
//!
//! [D] design choices (the original has no such screen), after the mockup's `gen_data.py` / `analysis.js`
//! (`reference_materials/ui-overhaul/analysis-overhaul`):
//! - distance is the arc length along the track's lap polyline (`TrackProfile::outline`), 0 at the start line; positions
//!   are projected onto it as `demo_stats` does;
//! - the grid has equal steps of about `GRID_M` and ends on the line. A lap's times are its lap clock, stretched so the
//!   grid's end is the file's lap time (a frame's worth at most). A flying lap is 0 at the line; a standing lap (from the
//!   grid) keeps the time it took to reach the line, so its first mini-sector holds the start;
//! - speed is the distance driven over a short time window: only a replay knows the car's speed, so every source uses
//!   its positions;
//! - the shares come from the speed and the radius of the car's own path, the same way for every source (ghosts and
//!   demos keep no inputs).

use crate::demo::GO_MS;
use crate::demo_stats::{car_laps, unwrap_arcs};
use crate::race::TrackProfile;
use crate::replay::{GhostFile, GhostTrack, Header, Recording};
use motor_data::fs;
use motor_data::mde::{self, Mde};
use std::f32::consts::{PI, TAU};
use std::path::Path;

motor_tune::tunable! {
    /// Lap Analysis: the distance grid's step; a lap is split into equal steps of about this (design choice).
    /// range: 1..20
    /// unit: m
    /// applies: reload
    pub GRID_M: f32 = 5.0;
    /// Lap Analysis: a sample's speed is the distance driven over this either side of it, at least two samples (so a
    /// demo's 62.5 ms records use about 0.13 s), then a median of 5 (design choice).
    /// range: 10..500
    /// unit: ms
    /// applies: reload
    SPEED_WINDOW_MS: f32 = 100.0;
    /// Lap Analysis: a lap whose car is slower than this when its lap clock starts began on the grid (a standing lap)
    /// (design choice).
    /// range: 1..100
    /// unit: km/h
    /// applies: reload
    STANDING_KMH: f32 = 30.0;
    /// Lap Analysis: turns whose apexes are closer than this share a mini-sector (design choice).
    /// range: 0..300
    /// unit: m
    /// applies: reload
    pub GROUP_M: f32 = 90.0;
    /// Lap Analysis: the most turns one mini-sector holds (design choice).
    /// range: 1..6
    /// applies: reload
    pub GROUP_MAX: usize = 3;
    /// Lap Analysis: a mini-sector won by less than this is nobody's (design choice).
    /// range: 0..100
    /// unit: ms
    /// applies: reload
    pub TIE_MS: f32 = 5.0;
    /// Lap Analysis speed class: the grid points this far either side of a group's apexes (design choice).
    /// range: 5..200
    /// unit: m
    /// applies: reload
    APEX_M: f32 = 60.0;
    /// Lap Analysis speed class: a group whose corner speed is under this is LOW SPEED. Motorhead runs far faster than a
    /// real car, so F1's thresholds do not fit (design choice).
    /// range: 50..400
    /// unit: km/h
    /// applies: reload
    pub CLASS_LOW_KMH: f32 = 180.0;
    /// Lap Analysis speed class: under this (and not LOW) is MEDIUM SPEED, else HIGH SPEED (design choice).
    /// range: 50..500
    /// unit: km/h
    /// applies: reload
    pub CLASS_MEDIUM_KMH: f32 = 260.0;
    /// Lap Analysis shares: slowing faster than this is HEAVY BRAKING (design choice).
    /// range: 0.5..30
    /// unit: m/s²
    /// applies: reload
    BRAKE_MS2: f32 = 4.5;
    /// Lap Analysis shares: the car's own path bending tighter than this radius is CORNERING (not braking) (design choice).
    /// range: 20..1000
    /// unit: m
    /// applies: reload
    CORNER_R_M: f32 = 220.0;
    /// Lap Analysis shares: the path's radius is the heading change between the chords this long before and after a point
    /// (design choice).
    /// range: 2..50
    /// unit: m
    /// applies: reload
    CURVE_M: f32 = 10.0;
}

/// Samples kept beyond a lap's start and end (ms): the speed window and the interpolation at the line.
const MARGIN_MS: f64 = 500.0;
/// A lap's samples must reach this close to the line at both ends (m).
const END_SHORT_M: f32 = 30.0;
/// The positions must fit the file's lap time within this share.
const STRETCH_MAX: f64 = 0.03;
/// A position farther out than this is no track's (m).
const FAR_M: f32 = 1.0e6;
/// A standing lap's run-up ends at the peak before the speed first drops by this (km/h): the low speed leaves it out.
const RUN_UP_DROP_KMH: f32 = 2.0;
/// A race replay reports its progress every this many frames.
const PROGRESS_EVERY: usize = 256;

/// What kind of file a lap comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Kind {
    /// a Time Attack ghost (`.ghl`)
    Ghost,
    /// a race replay (`.mhr`), re-simulated
    Replay,
    /// a demo (`.MDE`, the original's recording format)
    Demo,
}

/// A recording, read but not worked out yet (`open`): enough to check its track before the slow part (`read`).
#[derive(Debug, Clone)]
pub enum Source {
    Ghost(GhostFile),
    Replay(Recording),
    Demo(Box<Mde>),
}

impl Source {
    pub fn kind(&self) -> Kind {
        match self {
            Source::Ghost(_) => Kind::Ghost,
            Source::Replay(_) => Kind::Replay,
            Source::Demo(_) => Kind::Demo,
        }
    }

    /// The track's `TrackName` as the file gives it.
    pub fn track_name(&self) -> &str {
        match self {
            Source::Ghost(g) => &g.track_name,
            Source::Replay(r) => &r.header.track_name,
            Source::Demo(m) => &m.header.track,
        }
    }

    /// The track's number (`Game/TrackN.trk`): a replay names it, a ghost or a demo by its name.
    pub fn track_number(&self, root: &Path) -> Option<u32> {
        match self {
            Source::Replay(r) => Some(r.header.track),
            _ => track_number(root, self.track_name()),
        }
    }
}

/// Why a file gives no lap to compare (the screen's requesters; a different track is the screen's own check).
#[derive(Debug, Clone, PartialEq)]
pub enum Refusal {
    /// not a ghost, race replay or demo
    NotARecording,
    /// one of those, but damaged or not consistent
    Damaged(String),
    /// no complete lap in it
    NoLap,
    /// a race replay that no longer drives the same race (recorded with older car physics): the time its header kept and
    /// the one the re-simulation gives, the best lap (`best_lap`) or else the race time
    Outdated { best_lap: bool, recorded_ms: Option<u32>, replayed_ms: Option<u32> },
    /// the caller stopped the re-simulation
    Cancelled,
    /// the race could not be rebuilt (a track or car that is not installed)
    Failed(String),
}

impl std::fmt::Display for Refusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let time = |ms: &Option<u32>| ms.map_or("none".to_string(), |ms| format!("{}.{:03}", ms / 1000, ms % 1000));
        match self {
            Refusal::NotARecording => write!(f, "not a Motorhead ghost (.ghl), race replay (.mhr) or demo (.MDE)"),
            Refusal::Damaged(e) => write!(f, "the file is damaged: {e}"),
            Refusal::NoLap => write!(f, "no complete lap"),
            Refusal::Outdated { best_lap, recorded_ms, replayed_ms } => {
                let what = if *best_lap { "best lap" } else { "race time" };
                write!(f, "recorded with older car physics: the recorded {what} was {}, the replay gives {}", time(recorded_ms), time(replayed_ms))
            }
            Refusal::Cancelled => write!(f, "cancelled"),
            Refusal::Failed(e) => write!(f, "the race could not be rebuilt: {e}"),
        }
    }
}

impl std::error::Error for Refusal {}

/// One lap of one car on the distance grid.
#[derive(Debug, Clone, PartialEq)]
pub struct LapTrace {
    pub kind: Kind,
    /// the car's slot in the race (0 for a ghost)
    pub slot: usize,
    /// the recorder's own car (a ghost is always the player's)
    pub human: bool,
    /// the driver's name or nick; "" = the player (a ghost)
    pub driver: String,
    /// the team tag (demos), else ""
    pub team: String,
    /// `Game/carNN.car` (ghosts, replays)
    pub car_id: Option<u32>,
    /// the car's name when known (demos; replays), else ""
    pub car_name: String,
    /// the lap of the race (1 = from the grid); `None` for a ghost
    pub lap: Option<u32>,
    /// the lap time the file gives (ms)
    pub lap_ms: f32,
    /// the lap clock started with the car standing on the grid
    pub standing: bool,
    /// the grid's step (m): point k is k * step along the lap, the last is the line again
    pub step: f32,
    /// the lap clock at each grid point (ms): 0 at the line on a flying lap, the time from the grid on a standing one;
    /// the last is `lap_ms`
    pub t_ms: Vec<f32>,
    /// km/h at each grid point
    pub kmh: Vec<f32>,
    /// where the car was at each grid point (game x, z)
    pub xz: Vec<[f32; 2]>,
}

impl LapTrace {
    /// The lap clock (ms) at arc length `s` (m).
    pub fn ms_at(&self, s: f32) -> f32 {
        let n = self.t_ms.len();
        if n == 0 {
            return 0.0;
        }
        let x = (s / self.step.max(1e-6)).clamp(0.0, (n - 1) as f32);
        let k = (x.floor() as usize).min(n.saturating_sub(2));
        match self.t_ms.get(k + 1) {
            Some(&b) => self.t_ms[k] + (b - self.t_ms[k]) * (x - k as f32),
            None => self.t_ms[k],
        }
    }

    /// Where the car was at lap clock `ms` (game x, z): the cursor's dot of the other side at the same moment.
    pub fn xz_at_ms(&self, ms: f32) -> [f32; 2] {
        let n = self.t_ms.len().min(self.xz.len());
        match n {
            0 => [0.0; 2],
            1 => self.xz[0],
            _ => {
                let j = self.t_ms[..n].partition_point(|&t| t <= ms).clamp(1, n - 1);
                let (a, b) = (j - 1, j);
                let f = ((ms - self.t_ms[a]) / (self.t_ms[b] - self.t_ms[a]).max(1e-6)).clamp(0.0, 1.0);
                [self.xz[a][0] + (self.xz[b][0] - self.xz[a][0]) * f, self.xz[a][1] + (self.xz[b][1] - self.xz[a][1]) * f]
            }
        }
    }
}

/// Every complete lap of a recording.
#[derive(Debug, Clone)]
pub struct LapFile {
    pub kind: Kind,
    pub track_name: String,
    /// the recorder's slot (a ghost: 0; `None` = nobody's, an AI-only race)
    pub me: Option<usize>,
    /// every complete lap of every car, by slot then lap
    pub laps: Vec<LapTrace>,
    /// a race replay whose re-simulation matched the times its header kept (false: nothing to check, or not a replay)
    pub checked: bool,
}

impl LapFile {
    /// The fastest flying lap of `slot` (any car with `None`); a standing lap only when there is no flying one.
    pub fn best(&self, slot: Option<usize>) -> Option<&LapTrace> {
        let mine = |l: &&LapTrace| slot.is_none_or(|s| l.slot == s);
        let fastest = |a: &&LapTrace, b: &&LapTrace| a.lap_ms.total_cmp(&b.lap_ms);
        self.laps.iter().filter(mine).filter(|l| !l.standing).min_by(fastest).or_else(|| self.laps.iter().filter(mine).min_by(fastest))
    }

    /// The lap a caller asks for: `car` is a car number, a car name or a driver name (`car_name` names a car number from
    /// `Game/carNN.car`), `lap` the race lap. With neither: the recorder's best flying lap, else anyone's. Among several
    /// matches the fastest flying lap wins, a standing one only when there is no flying one.
    pub fn pick(&self, car: Option<&str>, lap: Option<u32>, car_name: &dyn Fn(u32) -> String) -> Option<&LapTrace> {
        if car.is_none() && lap.is_none() {
            return self.best(self.me).or_else(|| self.best(None));
        }
        let named = |l: &LapTrace| match car {
            None => true,
            Some(c) => {
                let name = if l.car_name.is_empty() { l.car_id.map(car_name).unwrap_or_default() } else { l.car_name.clone() };
                let by_number = c.parse::<u32>().ok().is_some_and(|n| l.car_id == Some(n) || car_name(n).eq_ignore_ascii_case(&name));
                by_number || c.eq_ignore_ascii_case(&name) || c.eq_ignore_ascii_case(&l.driver)
            }
        };
        let cand: Vec<&LapTrace> = self.laps.iter().filter(|l| named(l) && lap.is_none_or(|n| l.lap == Some(n))).collect();
        let fastest = |a: &&&LapTrace, b: &&&LapTrace| a.lap_ms.total_cmp(&b.lap_ms);
        cand.iter().filter(|l| !l.standing).min_by(fastest).or_else(|| cand.iter().min_by(fastest)).copied()
    }
}

/// Side A (left) or B (right) of a comparison.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    A,
    B,
}

/// How fast a group of turns is taken (the map's and the chart's labels).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpeedClass {
    Low,
    Medium,
    High,
}

impl SpeedClass {
    pub fn of(kmh: f32) -> SpeedClass {
        if kmh < CLASS_LOW_KMH.get() {
            SpeedClass::Low
        } else if kmh < CLASS_MEDIUM_KMH.get() {
            SpeedClass::Medium
        } else {
            SpeedClass::High
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            SpeedClass::Low => "LOW",
            SpeedClass::Medium => "MEDIUM",
            SpeedClass::High => "HIGH",
        }
    }
}

/// A mini-sector of the lap: its arc lengths and the turns in it (1-based numbers of `TrackProfile::turns`).
#[derive(Debug, Clone, PartialEq)]
pub struct Stretch {
    pub a: f32,
    pub b: f32,
    pub turns: Vec<usize>,
}

/// A mini-sector, compared.
#[derive(Debug, Clone, PartialEq)]
pub struct MiniSector {
    pub a: f32,
    pub b: f32,
    pub turns: Vec<usize>,
    /// each side's time through it (ms); the first starts at lap clock 0, so it holds a standing start
    pub ms: [f32; 2],
    /// B's time minus A's (ms): > 0 = A faster
    pub delta_ms: f32,
    /// `None` = within `TIE_MS`
    pub winner: Option<Side>,
    /// the corner speed (km/h) its class comes from
    pub min_kmh: f32,
    pub class: SpeedClass,
}

/// A lap's share of its time (percent, summing to 100).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Shares {
    pub throttle: u8,
    pub braking: u8,
    pub cornering: u8,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SideStats {
    pub lap_ms: f32,
    pub top_kmh: f32,
    /// the lowest speed (a standing lap's run-up from the grid left out)
    pub low_kmh: f32,
    pub shares: Shares,
}

/// Two laps compared (`compare`).
#[derive(Debug, Clone, PartialEq)]
pub struct Comparison {
    pub step: f32,
    /// B's lap clock minus A's at each grid point (ms): > 0 = A ahead there
    pub gap: Vec<f32>,
    /// B's lap time minus A's (ms) = the sum of the mini-sectors' deltas
    pub lap_gap_ms: f32,
    /// the largest gap of the lap either way (ms): the delta strip's scale
    pub max_gap_ms: f32,
    pub minis: Vec<MiniSector>,
    pub sides: [SideStats; 2],
}

/// Read a recording's bytes: its kind by the magic, then the file. Fast (no lap worked out), so the track can be checked
/// before `read`.
pub fn open(d: &[u8]) -> Result<Source, Refusal> {
    match d.get(..4) {
        Some(b"GHL1") => GhostTrack::parse(d).map(Source::Ghost).map_err(Refusal::Damaged),
        Some(b"MHR1" | b"MHR2" | b"MHR3") => Recording::from_bytes(d).map(Source::Replay).map_err(Refusal::Damaged),
        Some(m) if m == mde::VERSION.to_le_bytes() => Mde::parse(d).map(|m| Source::Demo(Box::new(m))).map_err(|e| Refusal::Damaged(e.to_string())),
        _ => Err(Refusal::NotARecording),
    }
}

/// Every complete lap of a recording, on `prof`'s grid (the recording's track). A race replay is re-simulated from the
/// install at `root` (a second or two): `progress` hears how far it is (0..1) and stops it by returning false. Ghosts and
/// demos use neither.
pub fn read(src: &Source, prof: &TrackProfile, root: &Path, progress: &mut dyn FnMut(f32) -> bool) -> Result<LapFile, Refusal> {
    if prof.outline.len() < 8 || prof.length.is_nan() || prof.length <= 1.0 {
        return Err(Refusal::Failed("the track has no lap outline".into()));
    }
    match src {
        Source::Ghost(g) => read_ghost(g, prof),
        Source::Replay(r) => read_replay(r, prof, root, progress),
        Source::Demo(m) => read_demo(m, prof),
    }
}

/// The track's number by its `TrackName` (case ignored), among the installed tracks 1..8.
pub fn track_number(root: &Path, name: &str) -> Option<u32> {
    (1..=8).find(|n| {
        fs::config(root, &format!("Game/Track{n}.trk")).ok().is_some_and(|c| c.get("TrackName").is_some_and(|e| e.values.join(" ").eq_ignore_ascii_case(name.trim())))
    })
}

/// The mini-sectors: the turns in lap order, those closer than `GROUP_M` sharing one (up to `GROUP_MAX`), the borders
/// halfway between the groups; the stretch before the first group and after the last belong to them. A lap without turns
/// is one stretch.
pub fn stretches(prof: &TrackProfile) -> Vec<Stretch> {
    let mut groups: Vec<Vec<usize>> = Vec::new();
    for (i, t) in prof.turns.iter().enumerate() {
        match groups.last_mut() {
            Some(g) if g.len() < GROUP_MAX.get().max(1) && g.last().is_some_and(|&j| t.s - prof.turns[j].s < GROUP_M.get()) => g.push(i),
            _ => groups.push(vec![i]),
        }
    }
    if groups.is_empty() {
        return vec![Stretch { a: 0.0, b: prof.length, turns: Vec::new() }];
    }
    let first = |g: &[usize]| prof.turns[g[0]].s;
    let last = |g: &[usize]| prof.turns[g[g.len() - 1]].s;
    let k = groups.len();
    (0..k)
        .map(|i| Stretch {
            a: if i == 0 { 0.0 } else { (last(&groups[i - 1]) + first(&groups[i])) * 0.5 },
            b: if i + 1 == k { prof.length } else { (last(&groups[i]) + first(&groups[i + 1])) * 0.5 },
            turns: groups[i].iter().map(|&j| j + 1).collect(),
        })
        .collect()
}

/// Compare lap A with lap B (both on `prof`'s grid).
pub fn compare(a: &LapTrace, b: &LapTrace, prof: &TrackProfile) -> Comparison {
    let n = a.t_ms.len().min(b.t_ms.len()).min(a.kmh.len()).min(b.kmh.len());
    let step = a.step;
    let gap: Vec<f32> = (0..n).map(|k| b.t_ms[k] - a.t_ms[k]).collect();
    let max_gap_ms = gap.iter().fold(0.0f32, |m, g| m.max(g.abs()));
    let sides = [side_stats(a), side_stats(b)];
    let lap_gap_ms = b.lap_ms - a.lap_ms;
    if n < 2 {
        return Comparison { step, gap, lap_gap_ms, max_gap_ms, minis: Vec::new(), sides };
    }
    // the grid's last point is the line again
    let ring = n - 1;
    let fast = |k: usize| a.kmh[k % ring].max(b.kmh[k % ring]);
    let reach = (APEX_M.get() / step.max(1e-3)).round() as isize;
    let minis = stretches(prof)
        .into_iter()
        .enumerate()
        .map(|(i, st)| {
            let ms = [a, b].map(|l| l.ms_at(st.b) - if i == 0 { 0.0 } else { l.ms_at(st.a) });
            let delta_ms = ms[1] - ms[0];
            let winner = if delta_ms.abs() < TIE_MS.get() {
                None
            } else if delta_ms > 0.0 {
                Some(Side::A)
            } else {
                Some(Side::B)
            };
            // the class: the faster car's lowest speed round the group's apexes (one car's spin does not make a fast corner
            // slow; the mockup's rule)
            let apexes: Vec<f32> = st.turns.iter().filter_map(|&no| prof.turns.get(no.wrapping_sub(1))).map(|t| t.s).collect();
            let min_kmh = if apexes.is_empty() {
                (0..ring).map(fast).fold(f32::INFINITY, f32::min)
            } else {
                apexes
                    .iter()
                    .flat_map(|&s| {
                        let c = (s / step.max(1e-3)).round() as isize;
                        (c - reach..=c + reach).map(move |k| k.rem_euclid(ring as isize) as usize)
                    })
                    .map(fast)
                    .fold(f32::INFINITY, f32::min)
            };
            MiniSector { a: st.a, b: st.b, turns: st.turns, ms, delta_ms, winner, min_kmh, class: SpeedClass::of(min_kmh) }
        })
        .collect();
    Comparison { step, gap, lap_gap_ms, max_gap_ms, minis, sides }
}

/// A lap's top and low speed and its shares.
fn side_stats(l: &LapTrace) -> SideStats {
    let m = l.t_ms.len().min(l.kmh.len()).min(l.xz.len());
    if m < 2 {
        return SideStats { lap_ms: l.lap_ms, top_kmh: 0.0, low_kmh: 0.0, shares: Shares { throttle: 100, ..Shares::default() } };
    }
    let top_kmh = l.kmh[..m].iter().copied().fold(0.0, f32::max);
    // a standing lap's run-up: up to the peak before the car first slows down (or its fastest point, if it never does)
    let mut from = 0;
    if l.standing {
        for k in 0..m {
            if l.kmh[k] > l.kmh[from] {
                from = k;
            } else if l.kmh[k] < l.kmh[from] - RUN_UP_DROP_KMH {
                break;
            }
        }
    }
    let low_kmh = l.kmh[from..m].iter().copied().fold(f32::INFINITY, f32::min);
    // the radius of the car's own path: the heading change between the chords before and after a point
    let ring = m - 1;
    let w = ((CURVE_M.get() / l.step.max(1e-3)).round() as usize).clamp(1, (ring / 4).max(1));
    let p = |k: usize| l.xz[k % ring];
    let head = |i: usize, j: usize| {
        let (a, b) = (p(i), p(j));
        (b[1] - a[1]).atan2(b[0] - a[0])
    };
    let radius = |k: usize| {
        let d = ((head(k, k + w) - head(k + ring - w, k) + PI).rem_euclid(TAU) - PI).abs();
        if d > 1e-6 { w as f32 * l.step / d } else { f32::INFINITY }
    };
    // a standing lap's time from the grid to the line is throttle
    let mut ms = [if l.standing { l.t_ms[0].max(0.0) } else { 0.0 }, 0.0, 0.0];
    for k in 0..ring {
        let dt = l.t_ms[k + 1] - l.t_ms[k];
        if dt.is_nan() || dt <= 0.0 {
            continue;
        }
        let acc = (l.kmh[k + 1] - l.kmh[k]) / 3.6 / (dt / 1000.0);
        let i = if acc < -BRAKE_MS2.get() {
            1
        } else if radius(k) < CORNER_R_M.get() {
            2
        } else {
            0
        };
        ms[i] += dt;
    }
    let [throttle, braking, cornering] = percents(ms);
    SideStats { lap_ms: l.lap_ms, top_kmh, low_kmh, shares: Shares { throttle, braking, cornering } }
}

/// Percentages of three times that sum to 100 (largest remainder).
fn percents(v: [f32; 3]) -> [u8; 3] {
    let v = v.map(|x| if x.is_finite() { x.max(0.0) } else { 0.0 });
    let tot: f32 = v.iter().sum();
    if tot <= 0.0 {
        return [100, 0, 0];
    }
    let raw = v.map(|x| x / tot * 100.0);
    let mut out = raw.map(|x| (x.floor() as u8).min(100));
    let mut left = 100u8.saturating_sub(out.iter().sum::<u8>());
    let mut order = [0usize, 1, 2];
    order.sort_by(|&a, &b| (raw[b] - raw[b].floor()).total_cmp(&(raw[a] - raw[a].floor())));
    for &i in order.iter().cycle().take(3) {
        if left == 0 {
            break;
        }
        out[i] += 1;
        left -= 1;
    }
    out
}

/// One sample of a lap: ms on its lap clock, the unwrapped arc length (m, 0 at the line), the position.
#[derive(Debug, Clone, Copy)]
struct Sample {
    ms: f64,
    s: f32,
    pos: [f32; 3],
}

/// Who drove a lap and where it comes from: the `LapTrace` fields beside the grid.
struct Who {
    kind: Kind,
    slot: usize,
    human: bool,
    driver: String,
    team: String,
    car_id: Option<u32>,
    car_name: String,
    lap: Option<u32>,
}

fn median(v: &[f32]) -> f32 {
    let mut s = [0.0f32; 5];
    let n = v.len().min(5);
    s[..n].copy_from_slice(&v[..n]);
    s[..n].sort_by(f32::total_cmp);
    s[n / 2]
}

/// A lap on the grid from its samples (the same for every source) and the file's lap time.
fn build(samples: &[Sample], lap_ms: f64, prof: &TrackProfile, who: Who) -> Result<LapTrace, Refusal> {
    if !(lap_ms.is_finite() && lap_ms > 0.0) {
        return Err(Refusal::Damaged("a lap has no time".into()));
    }
    let mut v: Vec<Sample> = Vec::with_capacity(samples.len());
    for x in samples {
        if !(x.ms.is_finite() && x.s.is_finite() && x.pos.iter().all(|c| c.is_finite() && c.abs() < FAR_M)) {
            return Err(Refusal::Damaged("a position is not a number".into()));
        }
        // a frame between two ticks, or the countdown, repeats the clock
        if v.last().is_none_or(|l| x.ms > l.ms) {
            v.push(*x);
        }
    }
    let n = v.len();
    if n < 4 {
        return Err(Refusal::NoLap);
    }
    let len = prof.length;
    // speed: the distance driven over the window, then a median of 5
    let mut run = vec![0.0f64; n];
    for i in 1..n {
        let (a, b) = (v[i - 1].pos, v[i].pos);
        run[i] = run[i - 1] + ((b[0] - a[0]).powi(2) + (b[1] - a[1]).powi(2) + (b[2] - a[2]).powi(2)).sqrt() as f64;
    }
    let w = SPEED_WINDOW_MS.get() as f64;
    let raw: Vec<f32> = (0..n)
        .map(|i| {
            let lo = v.partition_point(|x| x.ms < v[i].ms - w).min(i.saturating_sub(2));
            let hi = v.partition_point(|x| x.ms <= v[i].ms + w).saturating_sub(1).max((i + 2).min(n - 1));
            let dt = (v[hi].ms - v[lo].ms) / 1000.0;
            if dt > 0.0 { ((run[hi] - run[lo]) / dt * 3.6) as f32 } else { 0.0 }
        })
        .collect();
    let kmh: Vec<f32> = (0..n).map(|i| median(&raw[i.saturating_sub(2)..(i + 3).min(n)])).collect();
    // the arc never runs backwards (a spin, a respawn, a projection hop)
    let mut m = Vec::with_capacity(n);
    let mut hi = f32::NEG_INFINITY;
    for x in &v {
        hi = hi.max(x.s);
        m.push(hi);
    }
    if m[0] > END_SHORT_M || m[n - 1] < len - END_SHORT_M {
        return Err(Refusal::NoLap);
    }
    // the first sample past the first one's arc (a car standing still repeats it), the first at the farthest arc
    let first_up = m.partition_point(|&x| x <= m[0]);
    let last_from = m.partition_point(|&x| x < m[n - 1]);
    if first_up >= n || last_from == 0 {
        return Err(Refusal::NoLap);
    }
    let ng = ((len / GRID_M.get().max(0.5)).round() as usize).max(1);
    let step = len / ng as f32;
    let (mut t, mut sp, mut xz) = (Vec::with_capacity(ng + 1), Vec::with_capacity(ng + 1), Vec::with_capacity(ng + 1));
    for k in 0..=ng {
        let s = if k == ng { len } else { k as f32 * step };
        let j = m.partition_point(|&x| x < s);
        // before the first sample or past the last one: carried on from the samples at that end
        let (a, b) = if j == 0 {
            (0, first_up)
        } else if j >= n {
            (last_from - 1, last_from)
        } else {
            (j - 1, j)
        };
        let f = (s - m[a]) / (m[b] - m[a]).max(1e-6);
        t.push(v[a].ms + (v[b].ms - v[a].ms) * f as f64);
        let g = f.clamp(0.0, 1.0);
        sp.push(kmh[a] + (kmh[b] - kmh[a]) * g);
        xz.push([v[a].pos[0] + (v[b].pos[0] - v[a].pos[0]) * g, v[a].pos[2] + (v[b].pos[2] - v[a].pos[2]) * g]);
    }
    // standing: slow when the lap clock starts
    let j0 = v.partition_point(|x| x.ms < 0.0).clamp(1, n - 1);
    let g0 = ((0.0 - v[j0 - 1].ms) / (v[j0].ms - v[j0 - 1].ms)).clamp(0.0, 1.0) as f32;
    let standing = kmh[j0 - 1] + (kmh[j0] - kmh[j0 - 1]) * g0 < STANDING_KMH.get();
    // the file's lap time ends the grid: a flying lap from 0 at the line, a standing one keeps its time to the line
    let (t0, tl) = (t[0], t[ng]);
    let z = if standing { t0.max(0.0) } else { 0.0 };
    let (span, want) = (tl - t0, lap_ms - z);
    if !(span > 0.0 && want > 0.0 && (want / span - 1.0).abs() <= STRETCH_MAX) {
        return Err(Refusal::Damaged("the positions do not fit the lap time".into()));
    }
    let ratio = want / span;
    let t_ms = t.iter().map(|&x| (z + (x - t0) * ratio) as f32).collect();
    let Who { kind, slot, human, driver, team, car_id, car_name, lap } = who;
    Ok(LapTrace { kind, slot, human, driver, team, car_id, car_name, lap, lap_ms: lap_ms as f32, standing, step, t_ms, kmh: sp, xz })
}

/// The laps that built; with none, the first reason (damage before a missing lap).
fn gather(results: Vec<Result<LapTrace, Refusal>>) -> Result<Vec<LapTrace>, Refusal> {
    let mut laps = Vec::new();
    let mut why = Refusal::NoLap;
    for r in results {
        match r {
            Ok(l) => laps.push(l),
            Err(e) => {
                if why == Refusal::NoLap {
                    why = e;
                }
            }
        }
    }
    if laps.is_empty() { Err(why) } else { Ok(laps) }
}

fn read_ghost(g: &GhostFile, prof: &TrackProfile) -> Result<LapFile, Refusal> {
    let poses = &g.track.poses;
    let arcs = unwrap_arcs(prof, poses.iter().map(|p| p.pos.to_array()));
    let samples: Vec<Sample> = poses.iter().zip(&arcs).map(|(p, &s)| Sample { ms: p.clock_ms, s, pos: p.pos.to_array() }).collect();
    let who = Who { kind: Kind::Ghost, slot: 0, human: true, driver: String::new(), team: String::new(), car_id: Some(g.car), car_name: String::new(), lap: None };
    let lap = build(&samples, g.track.lap_ms as f64, prof, who)?;
    Ok(LapFile { kind: Kind::Ghost, track_name: g.track_name.clone(), me: Some(0), laps: vec![lap], checked: false })
}

/// [D] a demo's laps are its line crossings (`demo_stats`): lap 1 from the green light (standing), lap k from crossing k-1.
fn read_demo(m: &Mde, prof: &TrackProfile) -> Result<LapFile, Refusal> {
    let h = &m.header;
    let slots = h.active_slots();
    if m.records.iter().any(|r| r.cars.len() < slots.len()) {
        return Err(Refusal::Damaged("the records hold fewer cars than the header names".into()));
    }
    let me = slots.contains(&(h.player as usize)).then_some(h.player as usize);
    let mut results = Vec::new();
    if m.records.len() >= 2 && h.laps > 0 {
        for (k, &slot) in slots.iter().enumerate() {
            let (p, ends) = car_laps(m, k, prof);
            let c = &h.cars[slot];
            for (l, &end) in ends.iter().enumerate() {
                let start = if l == 0 { GO_MS } else { ends[l - 1] };
                let base = l as f32 * prof.length;
                let span = start - MARGIN_MS..=end + MARGIN_MS;
                let samples: Vec<Sample> = m.records.iter().zip(&p).filter(|(r, _)| span.contains(&(r.ms as f64))).map(|(r, &s)| Sample { ms: r.ms as f64 - start, s: s - base, pos: r.cars[k].pos }).collect();
                let who = Who { kind: Kind::Demo, slot, human: me == Some(slot), driver: c.name.clone(), team: c.team.clone(), car_id: None, car_name: c.car.clone(), lap: Some(l as u32 + 1) };
                results.push(build(&samples, end - start, prof, who));
            }
        }
    }
    Ok(LapFile { kind: Kind::Demo, track_name: h.track.clone(), me, laps: gather(results)?, checked: false })
}

/// The determinism check of a race replay: the human's best lap and race time from the re-simulation against the ones
/// its header kept (the sim is deterministic, so a difference means the physics changed since). `Ok(true)` = there was a
/// time and it matched, `Ok(false)` = nothing to check.
fn check(h: &Header, finished: Option<u32>, best: Option<u32>) -> Result<bool, Refusal> {
    if h.best_lap_ms != best {
        return Err(Refusal::Outdated { best_lap: true, recorded_ms: h.best_lap_ms, replayed_ms: best });
    }
    if h.race_ms != finished {
        return Err(Refusal::Outdated { best_lap: false, recorded_ms: h.race_ms, replayed_ms: finished });
    }
    Ok(h.best_lap_ms.is_some() || h.race_ms.is_some())
}

/// [D] a replay's laps are the race's own lap timing (`Progress`), every car's, from re-simulating the frames.
fn read_replay(rec: &Recording, prof: &TrackProfile, root: &Path, progress: &mut dyn FnMut(f32) -> bool) -> Result<LapFile, Refusal> {
    rec.validate().map_err(Refusal::Damaged)?;
    let h = &rec.header;
    let mut race = rec.load_race(root).map_err(Refusal::Failed)?;
    let n = race.racers.len();
    // per car: its positions by race clock, (lap, lap start, finished) as last seen, the laps it completed (lap, start, ms)
    let mut runs: Vec<Vec<(f64, [f32; 3])>> = vec![Vec::new(); n];
    let mut seen: Vec<(u32, u32, bool)> = race.racers.iter().map(|r| (r.progress.lap, r.progress.lap_start_ms, r.progress.finished_ms.is_some())).collect();
    let mut done: Vec<Vec<(u32, u32, u32)>> = vec![Vec::new(); n];
    let total = rec.frames.len().max(1);
    for (i, f) in rec.frames.iter().enumerate() {
        if i % PROGRESS_EVERY == 0 && !progress(i as f32 / total as f32) {
            return Err(Refusal::Cancelled);
        }
        Recording::step(&mut race, f);
        // on the fixed tick a frame between two ticks changes nothing
        if race.ticks_last_frame() == 0 {
            continue;
        }
        for (slot, r) in race.racers.iter().enumerate() {
            runs[slot].push((race.clock_ms, r.car.pos.to_array()));
            let p = &r.progress;
            let (lap, start, fin) = seen[slot];
            if p.lap != lap || p.finished_ms.is_some() != fin {
                if let Some(ms) = p.last_lap_ms.filter(|_| p.lap > lap || (p.finished_ms.is_some() && !fin)) {
                    done[slot].push((lap, start, ms));
                }
                seen[slot] = (p.lap, p.lap_start_ms, p.finished_ms.is_some());
            }
        }
    }
    progress(1.0);
    let checked = match h.player.and_then(|p| race.racers.get(p)) {
        Some(r) => check(h, r.progress.finished_ms, r.progress.best_lap_ms)?,
        None => false,
    };
    let mut results = Vec::new();
    for (slot, r) in race.racers.iter().enumerate() {
        let human = h.player == Some(slot);
        let driver = if human { h.nick.clone() } else { r.driver.nick.clone() };
        for &(lap, start, ms) in &done[slot] {
            let span = start as f64 - MARGIN_MS..=start as f64 + ms as f64 + MARGIN_MS;
            let run: Vec<&(f64, [f32; 3])> = runs[slot].iter().filter(|x| span.contains(&x.0)).collect();
            let arcs = unwrap_arcs(prof, run.iter().map(|x| x.1));
            let samples: Vec<Sample> = run.iter().zip(&arcs).map(|(x, &s)| Sample { ms: x.0 - start as f64, s, pos: x.1 }).collect();
            let who = Who { kind: Kind::Replay, slot, human, driver: driver.clone(), team: String::new(), car_id: Some(r.car_id), car_name: r.car.p.name.clone(), lap: Some(lap) };
            results.push(build(&samples, ms as f64, prof, who));
        }
    }
    Ok(LapFile { kind: Kind::Replay, track_name: h.track_name.clone(), me: h.player, laps: gather(results)?, checked })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::car::Input;
    use crate::demo_stats::analyse;
    use crate::race::Turn;
    use crate::replay::GhostPose;
    use crate::session::{AiRow, Difficulty, Rules};
    use glam::{Mat3, Vec3};
    use motor_data::mde::{CarInfo, CarState, Header as MdeHeader, Record, CAR_ENTRIES};

    const SIDE: f32 = 400.0;
    const LAP: f32 = 4.0 * SIDE;

    /// The square lap of `demo_stats`' tests (400 m sides, 10 m outline steps) at arc length `s`.
    fn at(s: f32) -> [f32; 3] {
        let s = s.rem_euclid(LAP);
        let (e, d) = ((s / SIDE).floor() as usize, s % SIDE);
        match e {
            0 => [d, 0.0, 0.0],
            1 => [SIDE, 0.0, d],
            2 => [SIDE - d, 0.0, SIDE],
            _ => [0.0, 0.0, SIDE - d],
        }
    }

    /// The square with turns placed at `turns` (arc lengths).
    fn square(turns: &[f32]) -> TrackProfile {
        let outline_s: Vec<f32> = (0..160).map(|i| i as f32 * 10.0).collect();
        let outline = outline_s.iter().map(|&s| at(s)).collect();
        let turns = turns.iter().map(|&s| Turn { s, deg: 90.0, apex: at(s), outward: [0.0, 0.0] }).collect();
        TrackProfile { length: LAP, elevation: vec![], outline, outline_s, sectors: [500.0, 1000.0], turns }
    }

    /// A ghost lap driven at `v(s)` m/s from arc `s0` at lap clock 0 to the line a lap on, posed every 1/60 s and at
    /// the frame that ends the lap.
    fn ghost(v: impl Fn(f32) -> f32, s0: f32) -> Vec<u8> {
        let pose = |ms: f64, s: f32| GhostPose { clock_ms: ms, pos: Vec3::from(at(s)), rot: Mat3::IDENTITY, wheel_ext: [0.0; 4], steer_smooth: 0.0, roll: 0.0 };
        let (mut s, mut t, mut next, mut poses) = (s0, 0.0f64, 0.0f64, Vec::new());
        while s < LAP {
            if t >= next {
                poses.push(pose(t, s));
                next += 1000.0 / 60.0;
            }
            s += v(s).max(0.0) * 0.001;
            t += 1.0;
        }
        poses.push(pose(t, s));
        GhostTrack { poses, lap_ms: t.round() as u32 }.to_bytes(10, "Square")
    }

    fn lap_of(bytes: &[u8], prof: &TrackProfile) -> LapTrace {
        let src = open(bytes).unwrap();
        assert_eq!(src.kind(), Kind::Ghost);
        let mut f = read(&src, prof, Path::new("."), &mut |_| true).unwrap();
        assert_eq!(f.laps.len(), 1);
        f.laps.remove(0)
    }

    fn sum_of_deltas(c: &Comparison) -> f32 {
        c.minis.iter().map(|m| m.delta_ms).sum()
    }

    /// Two flying laps, B slower through 700..900 m: the stretch with turn 3 is A's, the others nobody's, and the deltas add
    /// up to the lap gap.
    #[test]
    fn flying_laps_compare_by_stretch() {
        let prof = square(&[400.0, 460.0, 800.0, 1200.0]);
        let a = lap_of(&ghost(|_| 40.0, 0.0), &prof);
        let b = lap_of(&ghost(|s| if (700.0..900.0).contains(&s) { 30.0 } else { 40.0 }, 0.0), &prof);
        assert!(!a.standing && !b.standing);
        assert_eq!(a.t_ms.len(), 321);
        assert!((a.step - 5.0).abs() < 1e-4);
        assert!((a.kmh[100] - 144.0).abs() < 1.0, "{}", a.kmh[100]);
        assert!(a.t_ms[0].abs() < 1e-3 && (a.t_ms[320] - a.lap_ms).abs() < 1e-2);
        let c = compare(&a, &b, &prof);
        let turns: Vec<_> = c.minis.iter().map(|m| m.turns.clone()).collect();
        assert_eq!(turns, vec![vec![1, 2], vec![3], vec![4]]);
        assert_eq!((c.minis[0].a, c.minis[0].b, c.minis[1].b, c.minis[2].b), (0.0, 630.0, 1000.0, LAP));
        assert_eq!(c.minis.iter().map(|m| m.winner).collect::<Vec<_>>(), vec![None, Some(Side::A), None]);
        assert!((c.minis[1].delta_ms - 1666.7).abs() < 20.0, "{}", c.minis[1].delta_ms);
        assert!((sum_of_deltas(&c) - c.lap_gap_ms).abs() < 0.5, "{} vs {}", sum_of_deltas(&c), c.lap_gap_ms);
        assert!((c.gap[320] - c.lap_gap_ms).abs() < 0.5);
        assert!((c.max_gap_ms - c.lap_gap_ms.abs()).abs() < 0.5);
        // the faster car's corner speed: 144 km/h, under the LOW threshold
        assert!((c.minis[1].min_kmh - 144.0).abs() < 2.0 && c.minis[1].class == SpeedClass::Low);
        // B's dot at A's moment halfway: behind A
        let ms = a.ms_at(800.0);
        assert!(b.xz_at_ms(ms)[1] < at(800.0)[2] - 10.0, "B is still on the second side: {:?}", b.xz_at_ms(ms));
    }

    #[test]
    fn turns_group_into_stretches() {
        let st = stretches(&square(&[100.0, 160.0, 220.0, 280.0, 900.0]));
        assert_eq!(st.iter().map(|s| s.turns.clone()).collect::<Vec<_>>(), vec![vec![1, 2, 3], vec![4], vec![5]]);
        assert_eq!(st.iter().map(|s| (s.a, s.b)).collect::<Vec<_>>(), vec![(0.0, 250.0), (250.0, 590.0), (590.0, LAP)]);
        assert_eq!(stretches(&square(&[])), vec![Stretch { a: 0.0, b: LAP, turns: vec![] }]);
    }

    /// A 3 s stop from 50 to 20 m/s at 10 m/s² is heavy braking; the square's corners are cornering; the rest throttle.
    #[test]
    fn shares_follow_speed_and_path() {
        let prof = square(&[400.0, 800.0, 1200.0]);
        let v = |s: f32| match s {
            s if (300.0..405.0).contains(&s) => (2500.0 - 20.0 * (s - 300.0)).max(400.0).sqrt(),
            s if (405.0..500.0).contains(&s) => 20.0,
            _ => 50.0,
        };
        let a = lap_of(&ghost(v, 0.0), &prof);
        let st = side_stats(&a);
        let sh = st.shares;
        assert_eq!(sh.throttle as u32 + sh.braking as u32 + sh.cornering as u32, 100);
        let want = 100.0 * 3000.0 / a.lap_ms;
        assert!((sh.braking as f32 - want).abs() <= 2.0, "braking {} want {want}", sh.braking);
        assert!(sh.cornering > 0 && sh.cornering < 15 && sh.throttle > 50, "{sh:?}");
        assert!((st.top_kmh - 180.0).abs() < 2.0 && (st.low_kmh - 72.0).abs() < 2.0, "{st:?}");
    }

    #[test]
    fn percentages_sum_to_100() {
        assert_eq!(percents([1.0, 1.0, 1.0]), [34, 33, 33]);
        assert_eq!(percents([50.5, 49.5, 0.0]), [51, 49, 0]);
        assert_eq!(percents([0.0, 0.0, 0.0]), [100, 0, 0]);
        assert_eq!(percents([f32::NAN, 1.0, 3.0]), [0, 25, 75]);
    }

    /// A lap from the grid 10 m behind the line keeps its run to the line in the lap clock and the first stretch.
    #[test]
    fn a_standing_lap_keeps_its_start() {
        let prof = square(&[400.0, 800.0, 1200.0]);
        let a = lap_of(&ghost(|_| 40.0, 0.0), &prof);
        let b = lap_of(&ghost(|s| (20.0 * (s + 10.0)).sqrt().clamp(0.5, 40.0), -10.0), &prof);
        assert!(b.standing && !a.standing);
        // 10 m from rest at 10 m/s²: 1.41 s
        assert!((b.t_ms[0] - 1414.0).abs() < 60.0, "{}", b.t_ms[0]);
        let c = compare(&a, &b, &prof);
        assert!((c.gap[0] - b.t_ms[0]).abs() < 1e-3);
        assert!((sum_of_deltas(&c) - c.lap_gap_ms).abs() < 0.5);
        assert_eq!(c.minis[0].winner, Some(Side::A));
        // the run-up is left out of the low speed: the same as the flying lap's (the square's sharp corners cut a few km/h
        // off the speed from positions)
        assert!((c.sides[1].low_kmh - c.sides[0].low_kmh).abs() < 4.0 && c.sides[1].low_kmh > 135.0, "{:?}", c.sides);
    }

    /// A demo of one car that pulls away at the green light (10 m/s² up to 40 m/s) from 20 m behind the line.
    fn demo(laps: u32, run_ms: u32) -> Mde {
        let mut cars: [CarInfo; CAR_ENTRIES] = Default::default();
        cars[0] = CarInfo { active: true, name: "ME".into(), team: "APEX".into(), car: "Titan".into(), ..CarInfo::default() };
        let mut m = Mde::new(MdeHeader::new("Square", laps, false, 0, cars));
        let mut ms = 62u32;
        while ms < run_ms {
            let t = ((ms as f32 - GO_MS as f32) / 1000.0).max(0.0);
            let s = -20.0 + if t < 4.0 { 5.0 * t * t } else { 80.0 + 40.0 * (t - 4.0) };
            m.records.push(Record { mask: 1, ms, cars: vec![CarState { pos: at(s), ..CarState::default() }] });
            ms += 62;
        }
        m
    }

    #[test]
    fn a_demo_gives_every_lap_by_its_line_crossings() {
        let prof = square(&[400.0, 800.0, 1200.0]);
        let m = demo(3, 6000 + 130_000);
        let stats = analyse(&m, &prof).unwrap();
        let src = open(&m.to_bytes()).unwrap();
        assert_eq!((src.kind(), src.track_name()), (Kind::Demo, "Square"));
        let f = read(&src, &prof, Path::new("."), &mut |_| true).unwrap();
        assert_eq!(f.me, Some(0));
        assert_eq!(f.laps.iter().map(|l| l.lap).collect::<Vec<_>>(), vec![Some(1), Some(2), Some(3)]);
        assert_eq!(f.laps.iter().map(|l| l.standing).collect::<Vec<_>>(), vec![true, false, false]);
        for (l, &want) in f.laps.iter().zip(&stats.cars[0].laps_ms) {
            assert!((l.lap_ms - want as f32).abs() <= 1.0, "{} vs {want}", l.lap_ms);
            assert!((l.t_ms[l.t_ms.len() - 1] - l.lap_ms).abs() < 1e-2);
        }
        assert_eq!((f.laps[1].driver.as_str(), f.laps[1].team.as_str(), f.laps[1].car_name.as_str()), ("ME", "APEX", "Titan"));
        assert!((f.laps[1].kmh[100] - 144.0).abs() < 2.0);
        // the best flying lap, not the standing start
        assert_ne!(f.best(Some(0)).unwrap().lap, Some(1));
    }

    /// A demo whose records hold fewer cars than its header names is refused, and the Ghost screen's analysis skips it.
    #[test]
    fn a_demo_with_missing_cars_is_refused() {
        let prof = square(&[]);
        let mut m = demo(3, 6000 + 50_000);
        m.header.cars[1] = CarInfo { active: true, ..CarInfo::default() };
        assert!(analyse(&m, &prof).is_none());
        assert!(matches!(read(&Source::Demo(Box::new(m)), &prof, Path::new("."), &mut |_| true), Err(Refusal::Damaged(_))));
    }

    #[test]
    fn the_determinism_check() {
        let h = |race_ms, best_lap_ms| Header {
            track: 5,
            laps: 2,
            catchup: false,
            rules: Rules::Normal,
            manual: false,
            row: AiRow::Difficulty(Difficulty::Hard),
            player: Some(0),
            cars: vec![10],
            track_name: String::new(),
            nick: String::new(),
            car_name: String::new(),
            drivers: vec![],
            tick_hz: Some(60.0),
            race_ms,
            best_lap_ms,
        };
        assert_eq!(check(&h(Some(60_000), Some(29_000)), Some(60_000), Some(29_000)), Ok(true));
        assert_eq!(check(&h(None, None), None, None), Ok(false));
        assert_eq!(check(&h(None, Some(29_000)), None, Some(29_500)), Err(Refusal::Outdated { best_lap: true, recorded_ms: Some(29_000), replayed_ms: Some(29_500) }));
        assert_eq!(check(&h(Some(60_000), Some(29_000)), Some(60_100), Some(29_000)), Err(Refusal::Outdated { best_lap: false, recorded_ms: Some(60_000), replayed_ms: Some(60_100) }));
    }

    fn install() -> Option<std::path::PathBuf> {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../installed_game");
        root.exists().then_some(root)
    }

    fn replay_header(player: Option<usize>) -> Header {
        Header {
            track: 5,
            laps: 2,
            catchup: false,
            rules: Rules::Normal,
            manual: false,
            row: AiRow::Difficulty(Difficulty::Hard),
            player,
            cars: vec![10, 4],
            track_name: "OkkunSpeedway".into(),
            nick: "Ann".into(),
            car_name: String::new(),
            drivers: vec![],
            tick_hz: Some(60.0),
            race_ms: None,
            best_lap_ms: None,
        }
    }

    /// Needs the game install (skipped without it): an AI-only race of 2 laps, recorded and re-simulated, gives each car's
    /// laps with the live race's lap times; cancelling stops it.
    #[test]
    fn a_replay_resimulates_to_every_cars_laps() {
        let Some(root) = install() else { return };
        let mut rec = Recording::new(replay_header(None));
        let mut live = rec.load_race(&root).unwrap();
        let mut laps: Vec<Vec<u32>> = vec![Vec::new(); 2];
        let mut lap_no = [1u32; 2];
        for i in 0..12_000 {
            rec.push(1.0 / 60.0, Input::default(), None, i == 0);
            let f = *rec.frames.last().unwrap();
            Recording::step(&mut live, &f);
            for (slot, r) in live.racers.iter().enumerate() {
                let p = &r.progress;
                let now = if p.finished_ms.is_some() { p.laps + 1 } else { p.lap };
                if now > lap_no[slot] {
                    laps[slot].push(p.last_lap_ms.unwrap());
                    lap_no[slot] = now;
                }
            }
            if live.racers.iter().all(|r| r.progress.finished_ms.is_some()) {
                break;
            }
        }
        assert!(laps.iter().all(|l| l.len() == 2), "both cars finish: {laps:?}");
        let prof = live.line.profile();
        let src = open(&rec.to_bytes()).unwrap();
        assert_eq!(src.track_number(&root), Some(5));
        let f = read(&src, &prof, &root, &mut |_| true).unwrap();
        assert!(!f.checked && f.me.is_none());
        for (slot, want) in laps.iter().enumerate() {
            let mine: Vec<&LapTrace> = f.laps.iter().filter(|l| l.slot == slot).collect();
            assert_eq!(&mine.iter().map(|l| l.lap_ms as u32).collect::<Vec<_>>(), want, "slot {slot}");
            assert_eq!(mine.iter().map(|l| (l.lap, l.standing)).collect::<Vec<_>>(), vec![(Some(1), true), (Some(2), false)]);
            assert!(mine[1].kmh.iter().copied().fold(0.0, f32::max) > 150.0);
            assert_eq!(mine[1].car_id, Some(live.racers[slot].car_id));
        }
        assert_eq!(read(&src, &prof, &root, &mut |p| p < 0.5).err(), Some(Refusal::Cancelled));
    }

    /// Needs the game install (skipped without it): a replay whose header claims a best lap the re-simulation does not
    /// drive is outdated.
    #[test]
    fn a_replay_that_drives_differently_is_outdated() {
        let Some(root) = install() else { return };
        let mut rec = Recording::new(replay_header(Some(0)));
        rec.header.best_lap_ms = Some(30_000);
        for i in 0..600 {
            rec.push(1.0 / 60.0, Input::default(), None, i == 0);
        }
        let prof = rec.load_race(&root).unwrap().line.profile();
        let got = read(&Source::Replay(rec), &prof, &root, &mut |_| true);
        assert_eq!(got.err(), Some(Refusal::Outdated { best_lap: true, recorded_ms: Some(30_000), replayed_ms: None }));
    }

    struct Rng(u64);

    impl Rng {
        fn next(&mut self) -> u64 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            self.0
        }
        fn below(&mut self, n: usize) -> usize {
            (self.next() % n.max(1) as u64) as usize
        }
    }

    /// Truncated, mutated and random bytes behind every magic: `open` and `read` refuse them and never panic. Replays are
    /// re-simulated only with the install, and only a few short ones.
    #[test]
    fn hostile_bytes_never_panic() {
        let prof = square(&[400.0, 800.0, 1200.0]);
        let root = install();
        let mut rec = Recording::new(replay_header(Some(0)));
        for i in 0..30 {
            rec.push(1.0 / 60.0, Input { throttle: 1.0, ..Input::default() }, (i == 20).then_some(1), i == 0);
        }
        let valid = [ghost(|_| 40.0, 0.0), demo(2, 6000 + 90_000).to_bytes(), rec.to_bytes()];
        let mut rng = Rng(0x9e37_79b9_7f4a_7c15);
        let mut bufs: Vec<Vec<u8>> = Vec::new();
        for v in &valid {
            for n in 0..v.len().min(400) {
                bufs.push(v[..n].to_vec());
            }
            for _ in 0..300 {
                let mut b = v.clone();
                for _ in 0..1 + rng.below(8) {
                    let i = rng.below(b.len());
                    b[i] = rng.next() as u8;
                }
                if rng.below(4) == 0 {
                    b.truncate(rng.below(b.len()));
                }
                bufs.push(b);
            }
            for _ in 0..200 {
                let mut b = v[..4].to_vec();
                b.extend((0..rng.below(5000)).map(|_| rng.next() as u8));
                bufs.push(b);
            }
        }
        bufs.push(b"lap notes\n".to_vec());
        let mut resims = 0;
        for b in &bufs {
            let Ok(src) = open(b) else { continue };
            match &src {
                Source::Replay(r) => {
                    if r.validate().is_ok() && r.seconds() < 2.0 && resims < 12 {
                        if let Some(root) = &root {
                            resims += 1;
                            let _ = read(&src, &prof, root, &mut |_| true);
                        }
                    }
                }
                Source::Demo(m) => {
                    let _ = analyse(m, &prof);
                    let _ = read(&src, &prof, Path::new("."), &mut |_| true);
                }
                Source::Ghost(_) => {
                    let _ = read(&src, &prof, Path::new("."), &mut |_| true);
                }
            }
        }
        assert_eq!(open(b"lap notes\n").err(), Some(Refusal::NotARecording));
        assert_eq!(open(b"").err(), Some(Refusal::NotARecording));
    }
}
