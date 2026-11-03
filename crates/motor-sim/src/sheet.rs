//! REMASTER (#94, design choice [I], not in the original): the numbers the modern Results screen
//! shows, collected from a finished race: lap and sector times, position at the end of every lap,
//! start place, laps led. The Classic results screen does not use it.
//!
//! Split indices (`Progress::splits`, 04-track-car-data 4.6): lap L (1-based) starts at
//! `splits[L*4]` (lap 1: race time 0, the slot is never written), checkpoint 1 / 2 is stored at
//! `L*4 + 2` / `L*4 + 3`, and the lap ends where the next one starts, `splits[(L+1)*4]`. The
//! sectors are start -> checkpoint 1 -> checkpoint 2 -> line, the bounds of `TrackProfile::sectors`.

use crate::race::{Centreline, Progress};
use crate::Race;

/// One car's laps. The vectors hold the laps it completed: a car out of the race (Knockout, DNF)
/// has fewer than the race has.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CarSheet {
    /// lap time (ms) of each completed lap
    pub lap_ms: Vec<u32>,
    /// the three sector times of each completed lap (the third is the rest of the lap, so they sum
    /// to the lap exactly); `None` when a bound has no time (a respawn jumped over a checkpoint
    /// and the key times cannot tell either)
    pub sector_ms: Vec<Option<[u32; 3]>>,
    /// start place, 1-based (the grid slot)
    pub grid: u8,
    /// place at the end of each lap, `[0]` = the grid: the order the cars crossed the line in
    /// (equal times: grid order), so the last entry of a finisher is its finishing place
    pub pos_at: Vec<u8>,
    /// laps whose end this car crossed first
    pub led: u8,
    pub best_lap: Option<u32>,
    /// owns the race's fastest lap (the first to set it on a tie)
    pub fastest: bool,
    /// race time if it finished
    pub race_ms: Option<u32>,
    /// Knockout: the lap it was knocked out on
    pub out_lap: Option<u32>,
}

#[derive(Debug, Clone, Default)]
pub struct RaceSheet {
    pub laps: u32,
    /// slots in finishing order (`Race::standings`)
    pub order: Vec<usize>,
    /// by slot
    pub cars: Vec<CarSheet>,
}

/// Race time (ms) at which the car completed `lap` (1-based).
fn lap_end(p: &Progress, lap: u32) -> Option<u32> {
    p.splits.get((lap as usize + 1) * 4).copied().flatten()
}

/// The three sector times of `lap`, which ran from `start` to `end`.
fn sectors(line: &Centreline, p: &Progress, lap: u32, start: u32, end: u32) -> Option<[u32; 3]> {
    let arcs = line.sector_arcs();
    // the checkpoint splits are the bounds only when the track has both checkpoints; otherwise
    // (OkkunSpeedway) or when a respawn skipped one, the time the car passed that arc
    let from_cp = line.sector_checkpoints().is_some();
    let bound = |i: usize| {
        let split = if from_cp { p.splits.get(lap as usize * 4 + i + 2).copied().flatten() } else { None };
        split.or_else(|| p.time_at_arc(line, lap, arcs[i]).map(|t| t.round().max(0.0) as u32))
    };
    let (t1, t2) = (bound(0)?.clamp(start, end), bound(1)?);
    let t2 = t2.clamp(t1, end);
    Some([t1 - start, t2 - t1, end - t2])
}

impl RaceSheet {
    pub fn build(race: &Race) -> RaceSheet {
        let progress: Vec<&Progress> = race.racers.iter().map(|r| &r.progress).collect();
        RaceSheet::from_progress(&race.line, race.laps, &progress, race.standings())
    }

    /// `progress[slot]`; `order` = the finishing order of the slots.
    pub fn from_progress(line: &Centreline, laps: u32, progress: &[&Progress], order: Vec<usize>) -> RaceSheet {
        let mut cars: Vec<CarSheet> = progress
            .iter()
            .enumerate()
            .map(|(slot, p)| {
                let mut c = CarSheet { grid: slot as u8 + 1, race_ms: p.finished_ms, out_lap: p.eliminated_ms.map(|_| p.lap), ..CarSheet::default() };
                let mut start = 0;
                for lap in 1..=laps {
                    let Some(end) = lap_end(p, lap).filter(|&e| e >= start) else { break };
                    c.lap_ms.push(end - start);
                    c.sector_ms.push(sectors(line, p, lap, start, end));
                    start = end;
                }
                c.best_lap = c.lap_ms.iter().copied().min();
                c
            })
            .collect();
        // positions: the order the line was crossed in at the end of each lap
        for slot in 0..cars.len() {
            cars[slot].pos_at.push(slot as u8 + 1);
            for lap in 1..=cars[slot].lap_ms.len() as u32 {
                let mine = lap_end(progress[slot], lap).unwrap_or(u32::MAX);
                let ahead = (0..cars.len()).filter(|&j| j != slot && lap_end(progress[j], lap).is_some_and(|t| t < mine || t == mine && j < slot)).count();
                cars[slot].pos_at.push(ahead as u8 + 1);
            }
            cars[slot].led = cars[slot].pos_at[1..].iter().filter(|&&p| p == 1).count() as u8;
        }
        // the fastest lap: the lowest time, the earlier crossing on a tie
        let fastest = (0..cars.len())
            .filter_map(|slot| {
                let best = cars[slot].best_lap?;
                let lap = cars[slot].lap_ms.iter().position(|&l| l == best)? as u32 + 1;
                Some((best, lap_end(progress[slot], lap)?, slot))
            })
            .min();
        if let Some((_, _, slot)) = fastest {
            cars[slot].fastest = true;
        }
        RaceSheet { laps, order, cars }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::race::tests::{drive, ring};

    /// `p` with every time scaled by `num / den` (a slower or faster car on the same laps).
    fn scaled(p: &Progress, num: u64, den: u64) -> Progress {
        let mut q = p.clone();
        let f = |t: u32| (t as u64 * num / den) as u32;
        q.splits.iter_mut().for_each(|s| *s = s.map(f));
        q.finished_ms = q.finished_ms.map(f);
        q.best_lap_ms = q.best_lap_ms.map(f);
        q
    }

    fn with_checkpoints() -> Centreline {
        let mut line = ring(24);
        line.checkpoints = vec![(line.length * 0.3, 1), (line.length * 0.6, 2)];
        line
    }

    #[test]
    fn laps_and_sectors_add_up() {
        let line = with_checkpoints();
        let p = drive(&line, 3, false, false);
        let s = RaceSheet::from_progress(&line, 3, &[&p], vec![0]);
        let c = &s.cars[0];
        assert_eq!(c.lap_ms.len(), 3);
        assert_eq!(c.lap_ms.iter().sum::<u32>(), p.finished_ms.unwrap());
        for (lap, sec) in c.lap_ms.iter().zip(&c.sector_ms) {
            let sec = sec.expect("sectors");
            assert_eq!(sec.iter().sum::<u32>(), *lap);
        }
        // the checkpoints sit at 30 % and 60 % of the lap, at a constant speed
        let sec = c.sector_ms[1].unwrap();
        let lap = c.lap_ms[1] as f32;
        assert!((sec[0] as f32 / lap - 0.3).abs() < 0.02 && (sec[1] as f32 / lap - 0.3).abs() < 0.02, "{sec:?} of {lap}");
        assert_eq!((c.grid, c.pos_at.clone(), c.led, c.fastest), (1, vec![1, 1, 1, 1], 3, true));
        assert_eq!(c.best_lap, p.best_lap_ms);
        assert_eq!(c.race_ms, p.finished_ms);
        assert_eq!(c.out_lap, None);
    }

    /// A track without checkpoints (or with one) takes the sector bounds from the key pass times: thirds of the lap.
    #[test]
    fn sectors_without_checkpoints_use_the_key_times() {
        let line = ring(24);
        let p = drive(&line, 3, false, false);
        let s = RaceSheet::from_progress(&line, 3, &[&p], vec![0]);
        let c = &s.cars[0];
        for (lap, sec) in c.lap_ms.iter().zip(&c.sector_ms) {
            let sec = sec.expect("sectors from the key times");
            assert_eq!(sec.iter().sum::<u32>(), *lap);
            for x in sec {
                assert!((x as f32 - *lap as f32 / 3.0).abs() < *lap as f32 * 0.05, "{sec:?} of {lap}");
            }
        }
    }

    /// Order, grid, positions per lap and laps led for a field; equal times keep the grid order.
    #[test]
    fn positions_follow_the_crossings() {
        let line = with_checkpoints();
        let base = drive(&line, 3, false, false);
        // slot 0 and 2 are equally slow, slot 1 is fastest, slot 3 is the slowest
        let ps = [scaled(&base, 11, 10), base.clone(), scaled(&base, 11, 10), scaled(&base, 12, 10)];
        let refs: Vec<&Progress> = ps.iter().collect();
        let order = vec![1, 0, 2, 3];
        let s = RaceSheet::from_progress(&line, 3, &refs, order.clone());
        for lap in 0..=3 {
            let places: Vec<u8> = s.cars.iter().map(|c| c.pos_at[lap]).collect();
            assert_eq!(places, if lap == 0 { vec![1, 2, 3, 4] } else { vec![2, 1, 3, 4] }, "end of lap {lap}");
        }
        for (place, &slot) in order.iter().enumerate() {
            assert_eq!(*s.cars[slot].pos_at.last().unwrap() as usize, place + 1);
            assert_eq!(s.cars[slot].grid as usize, slot + 1);
        }
        assert_eq!(s.cars.iter().map(|c| c.led).collect::<Vec<_>>(), vec![0, 3, 0, 0]);
        assert_eq!(s.cars.iter().map(|c| c.fastest).collect::<Vec<_>>(), vec![false, true, false, false]);
    }

    /// Knockout: a car out on lap 3 has two laps, the positions up to there, and the lap it was knocked out on.
    #[test]
    fn a_knocked_out_car_stops_at_its_lap() {
        let line = with_checkpoints();
        let base = drive(&line, 3, false, false);
        let mut out = scaled(&base, 12, 10);
        out.splits[16..].iter_mut().for_each(|s| *s = None);
        out.finished_ms = None;
        out.eliminated_ms = Some(out.splits[12].unwrap() + 500);
        out.lap = 3;
        let ps = [base, out];
        let refs: Vec<&Progress> = ps.iter().collect();
        let s = RaceSheet::from_progress(&line, 3, &refs, vec![0, 1]);
        let c = &s.cars[1];
        assert_eq!((c.lap_ms.len(), c.sector_ms.len(), c.pos_at.clone()), (2, 2, vec![2, 2, 2]));
        assert_eq!((c.out_lap, c.race_ms, c.led), (Some(3), None, 0));
        assert_eq!(s.cars[0].pos_at, vec![1, 1, 1, 1]);
    }
}
