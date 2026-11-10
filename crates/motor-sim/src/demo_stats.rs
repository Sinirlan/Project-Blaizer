//! REMASTER Ghost screen (#92 phase 3): what a `.MDE` recording says about the race, worked out from the cars' positions.
//! Classic shows only the file's length; the Glass screen shows the race time, the laps, the best lap and its speed
//! trace, the top speed and the finish.
//!
//! [D] A lap is a crossing of the track's start line, taken as arc length 0 of the centreline (`TrackProfile`): each
//! record's position is projected onto the lap polyline, the unwrapped arc length crosses `k * length` at the end of lap
//! k, and the crossing time is interpolated between the two records. The race clock starts at the green light
//! (`GO_MS`; the standing start is part of lap 1). The grid lies behind the line. A car that stops being recorded keeps its last
//! place. Speeds are the distance over a window of about one second (the file keeps no speed).

use motor_data::mde::{Mde, Record};

use crate::demo::GO_MS;
use crate::race::TrackProfile;

/// Points of the best lap's trace (equal time steps).
pub const TRACE_POINTS: usize = 150;
/// The speed window: records either side of the one asked about (about 0.9 s).
const WINDOW: usize = 7;
/// Segments searched either side of the last one (10 m each); farther away than `LOST` the whole lap is searched again.
const REACH: usize = 15;
const LOST: f32 = 60.0;
/// How far short of the last line the file may stop and still count as a finish (m).
const EDGE: f32 = 12.0;

/// One recorded car's run.
#[derive(Clone, Debug, PartialEq)]
pub struct CarRun {
    /// the header slot
    pub slot: usize,
    /// the laps it completed, ms (lap 1 from the green light)
    pub laps_ms: Vec<u32>,
    /// the race time from the green light when it crossed the line for the last lap
    pub finish_ms: Option<u32>,
    /// the lap it was on at the end of the file (1-based)
    pub lap_at_end: u32,
    /// the distance along the race at the end of the file, in laps
    pub progress: f32,
}

/// A recording, worked out.
#[derive(Clone, Debug, PartialEq)]
pub struct Stats {
    /// one entry per recorded car, in block order
    pub cars: Vec<CarRun>,
    /// the block of the car that recorded it
    pub me: usize,
    /// `cars` by finish: those that finished by time, then the rest by distance covered
    pub order: Vec<usize>,
    /// the recorder's best lap: its number (1-based) and time
    pub best: Option<(u32, u32)>,
    /// the recorder's top speed from the green light to the flag, km/h
    pub top_kmh: u32,
    /// the recorder had not finished when the file ends: the lap it was on
    pub quit_lap: Option<u32>,
    /// the best lap's line: game x, y, z and km/h at equal time steps, a lap round; empty without a completed lap
    pub trace: Vec<[f32; 4]>,
}

impl Stats {
    /// The recorder's race time from the green light.
    pub fn race_ms(&self) -> Option<u32> {
        self.cars.get(self.me).and_then(|c| c.finish_ms)
    }
}

/// The nearest point of the closed lap polyline to `p` (x, z): its arc length and distance. `from` = the segment the
/// last point was on, to search a window round it; `None` searches everything.
/// The outline must have points (`analyse` and `lap_compare::read` check for 8).
pub(crate) fn project(prof: &TrackProfile, p: [f32; 3], from: Option<usize>) -> (f32, f32, usize) {
    let n = prof.outline.len();
    let step = prof.length / n as f32;
    let one = |i: usize| {
        let (a, b) = (prof.outline[i], prof.outline[(i + 1) % n]);
        let (dx, dz) = (b[0] - a[0], b[2] - a[2]);
        let l2 = (dx * dx + dz * dz).max(1e-6);
        let u = (((p[0] - a[0]) * dx + (p[2] - a[2]) * dz) / l2).clamp(0.0, 1.0);
        let (ex, ez) = (a[0] + dx * u - p[0], a[2] + dz * u - p[2]);
        (i as f32 * step + u * step, (ex * ex + ez * ez).sqrt(), i)
    };
    let best = |it: &mut dyn Iterator<Item = usize>| it.map(one).min_by(|a, b| a.1.total_cmp(&b.1)).unwrap();
    match from {
        Some(f) => {
            let r = best(&mut (0..=2 * REACH).map(|k| (f + n + k - REACH) % n));
            if r.1 > LOST { best(&mut (0..n)) } else { r }
        }
        None => best(&mut (0..n)),
    }
}

/// The unwrapped arc length (m, 0 at the start line) of each point of a run, in order: a jump of more than half a lap
/// between two points is a crossing of the line. The first point counts as behind the line (negative) when it lies on the
/// second half of the lap: the grid, or the run-up to a lap. Also `lap_compare`'s ghosts and replays.
pub(crate) fn unwrap_arcs(prof: &TrackProfile, points: impl Iterator<Item = [f32; 3]>) -> Vec<f32> {
    let len = prof.length;
    let (mut out, mut at, mut lap, mut prev) = (Vec::new(), None, 0.0f32, 0.0f32);
    for (i, p) in points.enumerate() {
        let (s, _, seg) = project(prof, p, at);
        at = Some(seg);
        if i == 0 {
            // the grid is behind the line
            if s > len * 0.5 {
                lap = -1.0;
            }
        } else if s - prev < -len * 0.5 {
            lap += 1.0;
        } else if s - prev > len * 0.5 {
            lap -= 1.0;
        }
        prev = s;
        out.push(s + lap * len);
    }
    out
}

/// The unwrapped arc length (m, 0 at the start line, negative on the grid) of block `k` at every record.
fn progress_of(mde: &Mde, k: usize, prof: &TrackProfile) -> Vec<f32> {
    unwrap_arcs(prof, mde.records.iter().map(|r| r.cars[k].pos))
}

/// Block `k`'s unwrapped arc length at every record and the ends of the laps it completed (ms on the file's clock; lap 1
/// from the green light). Every record must hold block `k` (`analyse` checks).
pub(crate) fn car_laps(mde: &Mde, k: usize, prof: &TrackProfile) -> (Vec<f32>, Vec<f64>) {
    let (h, len) = (&mde.header, prof.length);
    let p = progress_of(mde, k, prof);
    // the ends of the laps, ms on the file's clock
    let mut ends: Vec<f64> = (1..=h.laps).map_while(|l| crossing(&mde.records, &p, l as f32 * len)).collect();
    // the recorder stops at the flag, up to a record short of the line: carry the last metres on at the last speed
    let n = p.len();
    let (end, last) = (h.laps as f32 * len, p[n - 1]);
    if ends.len() + 1 == h.laps as usize && last < end && end - last <= EDGE {
        let (a, b) = (&mde.records[n - 2], &mde.records[n - 1]);
        let v = ((p[n - 1] - p[n - 2]) / (b.ms.saturating_sub(a.ms)).max(1) as f32).max(1e-3);
        ends.push(b.ms as f64 + ((end - last) / v) as f64);
    }
    (p, ends)
}

/// When `target` (m) is first reached, ms on the `.MDE` clock.
fn crossing(recs: &[Record], p: &[f32], target: f32) -> Option<f64> {
    let i = p.windows(2).position(|w| w[0] < target && w[1] >= target)?;
    let (a, b) = (p[i], p[i + 1]);
    let f = ((target - a) / (b - a).max(1e-6)) as f64;
    Some(recs[i].ms as f64 + f * (recs[i + 1].ms as f64 - recs[i].ms as f64))
}

/// Speed at every record of block `k`, km/h (metres over a window of records either side).
fn speeds_of(mde: &Mde, k: usize) -> Vec<f32> {
    let n = mde.records.len();
    (0..n)
        .map(|i| {
            let (a, b) = (&mde.records[i.saturating_sub(WINDOW)], &mde.records[(i + WINDOW).min(n - 1)]);
            let (p, q) = (a.cars[k].pos, b.cars[k].pos);
            let d = ((q[0] - p[0]).powi(2) + (q[1] - p[1]).powi(2) + (q[2] - p[2]).powi(2)).sqrt();
            d / ((b.ms - a.ms).max(1) as f32 / 1000.0) * 3.6
        })
        .collect()
}

/// REMASTER playback bar (#346): the speed of block `k` over the whole file, km/h, at `points` equal time steps from the
/// first record to the last (the diagram that unfolds over the bar). Empty without records.
pub fn speed_trace(mde: &Mde, k: usize, points: usize) -> Vec<f32> {
    let n = mde.records.len();
    if n < 2 || points < 2 {
        return Vec::new();
    }
    let speed = speeds_of(mde, k);
    let (t0, t1) = (mde.records[0].ms as f64, mde.records[n - 1].ms as f64);
    (0..points)
        .map(|q| {
            let t = t0 + (t1 - t0) * q as f64 / (points - 1) as f64;
            let i = mde.records.partition_point(|r| r.ms as f64 <= t).clamp(1, n - 1);
            let (a, b) = (&mde.records[i - 1], &mde.records[i]);
            let f = (((t - a.ms as f64) / (b.ms - a.ms).max(1) as f64).clamp(0.0, 1.0)) as f32;
            speed[i - 1] + (speed[i] - speed[i - 1]) * f
        })
        .collect()
}

/// Work a recording out against its track's lap. `None` when it has no records or no cars.
pub fn analyse(mde: &Mde, prof: &TrackProfile) -> Option<Stats> {
    let h = &mde.header;
    let slots = h.active_slots();
    // a damaged or crafted file can name more cars than its records hold
    if mde.records.len() < 2 || slots.is_empty() || prof.outline.len() < 8 || h.laps == 0 || mde.records.iter().any(|r| r.cars.len() < slots.len()) {
        return None;
    }
    let me = slots.iter().position(|&s| s == h.player as usize).unwrap_or(0);
    let len = prof.length;
    let go = GO_MS;
    let mut cars = Vec::new();
    let mut mine_ends = Vec::new();
    for (k, &slot) in slots.iter().enumerate() {
        let (p, ends) = car_laps(mde, k, prof);
        let laps_ms: Vec<u32> = ends.iter().enumerate().map(|(l, &t)| (t - if l == 0 { go } else { ends[l - 1] }).round().max(0.0) as u32).collect();
        let finish_ms = (ends.len() == h.laps as usize).then(|| (ends[ends.len() - 1] - go).round().max(0.0) as u32);
        let last = *p.last().unwrap_or(&0.0);
        cars.push(CarRun { slot, laps_ms, finish_ms, lap_at_end: ((last / len).floor().max(0.0) as u32 + 1).min(h.laps), progress: last / len });
        if k == me {
            mine_ends = ends;
        }
    }
    let mut order: Vec<usize> = (0..cars.len()).collect();
    order.sort_by(|&a, &b| match (cars[a].finish_ms, cars[b].finish_ms) {
        (Some(x), Some(y)) => x.cmp(&y),
        (Some(_), None) => std::cmp::Ordering::Less,
        (None, Some(_)) => std::cmp::Ordering::Greater,
        (None, None) => cars[b].progress.total_cmp(&cars[a].progress),
    });
    let run = &cars[me];
    let best = run.laps_ms.iter().enumerate().min_by_key(|(_, &t)| t).map(|(l, &t)| (l as u32 + 1, t));
    let speed = speeds_of(mde, me);
    // top speed between the green light and the flag
    let end_ms = run.finish_ms.map_or(f64::MAX, |f| go + f as f64);
    let top = mde.records.iter().zip(&speed).filter(|(r, _)| r.ms as f64 >= go && r.ms as f64 <= end_ms).map(|(_, &s)| s).fold(0.0, f32::max);
    // the best lap's trace: equal time steps, the position and speed blended between the two records round each
    let mut trace = Vec::new();
    if let Some((no, ms)) = best {
        let end = mine_ends.get(no as usize - 1).copied().unwrap_or(go);
        let start = end - ms as f64;
        for q in 0..TRACE_POINTS {
            let t = start + (end - start) * q as f64 / TRACE_POINTS as f64;
            let i = mde.records.partition_point(|r| (r.ms as f64) <= t).clamp(1, mde.records.len() - 1);
            let (a, b) = (&mde.records[i - 1], &mde.records[i]);
            let f = (((t - a.ms as f64) / ((b.ms - a.ms).max(1) as f64)).clamp(0.0, 1.0)) as f32;
            let (pa, pb) = (a.cars[me].pos, b.cars[me].pos);
            let mix = |x: f32, y: f32| x + (y - x) * f;
            trace.push([mix(pa[0], pb[0]), mix(pa[1], pb[1]), mix(pa[2], pb[2]), mix(speed[i - 1], speed[i])]);
        }
    }
    let quit_lap = cars[me].finish_ms.is_none().then_some(cars[me].lap_at_end);
    Some(Stats { cars, me, order, best, top_kmh: top.round() as u32, quit_lap, trace })
}

#[cfg(test)]
mod tests {
    use super::*;
    use motor_data::mde::{CarInfo, CarState, Header, CAR_ENTRIES};

    /// A square lap of 400 m a side (10 m steps) and a recording of one car running it at `v` m/s from the grid 20 m behind
    /// the line, `laps` laps and then `extra` seconds more.
    fn square() -> TrackProfile {
        let side = 400.0;
        let mut outline = Vec::new();
        let mut outline_s = Vec::new();
        for i in 0..160 {
            let s = i as f32 * 10.0;
            let (e, d) = ((s / side).floor() as usize, s % side);
            let p = match e {
                0 => [d, 0.0, 0.0],
                1 => [side, 0.0, d],
                2 => [side - d, 0.0, side],
                _ => [0.0, 0.0, side - d],
            };
            outline.push(p);
            outline_s.push(s);
        }
        TrackProfile { length: 1600.0, elevation: vec![], outline, outline_s, sectors: [500.0, 1000.0], turns: vec![] }
    }

    fn at(prof: &TrackProfile, s: f32) -> [f32; 3] {
        let s = s.rem_euclid(prof.length);
        let i = ((s / 10.0).floor() as usize).min(159);
        let (a, b) = (prof.outline[i], prof.outline[(i + 1) % 160]);
        let u = (s - i as f32 * 10.0) / 10.0;
        [a[0] + (b[0] - a[0]) * u, 0.0, a[2] + (b[2] - a[2]) * u]
    }

    fn recording(prof: &TrackProfile, laps: u32, run_ms: u32, v: f32) -> Mde {
        let mut cars: [CarInfo; CAR_ENTRIES] = Default::default();
        cars[0] = CarInfo { active: true, name: "ME".into(), car: "Titan".into(), ..CarInfo::default() };
        let mut m = Mde::new(Header::new("Square", laps, false, 0, cars));
        let mut ms = 62u32;
        while ms < run_ms {
            // still on the grid until the green light, then v m/s
            let s = -20.0 + v * ((ms as f32 - GO_MS as f32) / 1000.0).max(0.0);
            let st = CarState { pos: at(prof, s), ..CarState::default() };
            m.records.push(Record { mask: 1, ms, cars: vec![st] });
            ms += 62;
        }
        m
    }

    #[test]
    fn the_speed_trace_follows_the_run() {
        let prof = square();
        let m = recording(&prof, 3, 6000 + 125_000, 40.0);
        let t = speed_trace(&m, 0, 100);
        assert_eq!(t.len(), 100);
        // standing on the grid at the start, 144 km/h once running
        assert!(t[0] < 1.0, "{}", t[0]);
        assert!((t[60] - 144.0).abs() < 3.0, "{}", t[60]);
        assert!(speed_trace(&Mde::new(m.header.clone()), 0, 100).is_empty());
    }

    #[test]
    fn laps_and_race_time_come_from_the_line_crossings() {
        let prof = square();
        // 40 m/s: 1600 m = 40 s a lap, the first starts 20 m behind the line (0.5 s more)
        let m = recording(&prof, 3, 6000 + 125_000, 40.0);
        let s = analyse(&m, &prof).unwrap();
        let laps = &s.cars[0].laps_ms;
        assert_eq!(laps.len(), 3);
        assert!(laps[0].abs_diff(40_500) < 70, "{laps:?}");
        assert!(laps[1].abs_diff(40_000) < 70 && laps[2].abs_diff(40_000) < 70, "{laps:?}");
        assert!(s.race_ms().unwrap().abs_diff(120_500) < 70);
        assert_ne!(s.best.unwrap().0, 1, "the standing start is the slowest lap");
        assert!(s.quit_lap.is_none());
        assert!(s.top_kmh.abs_diff(144) <= 2, "{}", s.top_kmh);
        assert_eq!(s.trace.len(), TRACE_POINTS);
    }

    #[test]
    fn a_recording_cut_short_says_which_lap_it_stopped_on() {
        let prof = square();
        // 2 laps and a half of 5
        let m = recording(&prof, 5, 6000 + 100_000, 40.0);
        let s = analyse(&m, &prof).unwrap();
        assert_eq!(s.cars[0].laps_ms.len(), 2);
        assert_eq!(s.quit_lap, Some(3));
        assert!(s.race_ms().is_none());
    }

    #[test]
    fn no_lap_driven_has_no_best_lap_and_no_trace() {
        let prof = square();
        let m = recording(&prof, 3, 6000 + 20_000, 40.0);
        let s = analyse(&m, &prof).unwrap();
        assert!(s.best.is_none() && s.trace.is_empty());
        assert_eq!(s.quit_lap, Some(1));
    }
}
