//! Player statistics (Personal options > Statistics). Our own feature, not in the original
//! (design choice): `stats.cfg` next to settings.cfg keeps the totals, per circuit and per car.
//!
//! `Run` collects one race frame by frame (`Run::frame` has no sim dependency, so the counting
//! rules are unit-tested); `Stats::add` folds it in when the race ends, is restarted or quit.
//! Races, wins, podiums and clean races only count for a race that reached the results screen
//! against rivals (`place`); everything driven counts either way.

use std::collections::BTreeMap;
use std::path::PathBuf;

/// A car-to-car hit counts from this closing speed (m/s) [APPROX, design choice].
pub const CAR_HIT_MIN: f32 = 6.0;
/// A hit against the world (wall, barrier) counts from this closing speed (m/s) [APPROX].
pub const WALL_HIT_MIN: f32 = 5.0;
/// A hit of one kind does not count again until this long (s) after the last frame it was
/// still hitting, so a scrape along a wall is one collision.
pub const HIT_COOLDOWN: f32 = 1.0;
/// Wheels off the ground for at least this long (s) is a jump; shorter is a bump [APPROX].
pub const JUMP_MIN_S: f32 = 0.25;
/// A car passed again within this time (s) is the same overtake (side by side swaps).
pub const PASS_COOLDOWN: f32 = 3.0;
/// A run shorter than this (m) does not count as a play of the circuit.
pub const PLAY_MIN_M: f64 = 100.0;

/// The summed driving figures (of a run, and over all runs).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Driving {
    pub distance_m: f64,
    pub race_s: f64,
    pub overtakes: u32,
    pub car_hits: u32,
    pub wall_hits: u32,
    pub drift_m: f64,
    pub air_s: f64,
    pub jumps: u32,
}

/// What the race loop reads for one frame of the player's car.
pub struct FrameSample<'a> {
    /// the clock runs: past the green light and not finished
    pub running: bool,
    /// m/s and km/h
    pub speed: f32,
    pub kmh: f32,
    /// a wheel on the ground slides sideways (the skid-mark test)
    pub drifting: bool,
    /// no wheel on the ground
    pub airborne: bool,
    /// closing speed of this frame's hit against another car / the world (0 = none)
    pub car_impact: f32,
    pub world_impact: f32,
    /// the slots still racing, ahead of the player and behind him
    pub ahead: &'a [usize],
    pub behind: &'a [usize],
}

/// One race as driven so far.
#[derive(Clone, Debug, Default)]
pub struct Run {
    pub d: Driving,
    pub top_kmh: f32,
    pub longest_drift_m: f32,
    pub longest_jump_s: f32,
    /// the best lap of the race, set when it is committed
    pub best_lap_ms: Option<u32>,
    drift_m: f32,
    air_s: f32,
    in_jump: bool,
    car_cd: f32,
    wall_cd: f32,
    prev_ahead: Vec<usize>,
    pass_cd: Vec<f32>,
}

impl Run {
    pub fn frame(&mut self, dt: f32, s: &FrameSample) {
        if !s.running {
            return;
        }
        let step = s.speed * dt;
        self.d.distance_m += step as f64;
        self.d.race_s += dt as f64;
        self.top_kmh = self.top_kmh.max(s.kmh.abs());

        self.car_cd = (self.car_cd - dt).max(0.0);
        self.wall_cd = (self.wall_cd - dt).max(0.0);
        if s.car_impact >= CAR_HIT_MIN {
            self.d.car_hits += (self.car_cd <= 0.0) as u32;
            self.car_cd = HIT_COOLDOWN;
        }
        if s.world_impact >= WALL_HIT_MIN {
            self.d.wall_hits += (self.wall_cd <= 0.0) as u32;
            self.wall_cd = HIT_COOLDOWN;
        }

        if s.drifting {
            self.drift_m += step;
            self.d.drift_m += step as f64;
            self.longest_drift_m = self.longest_drift_m.max(self.drift_m);
        } else {
            self.drift_m = 0.0;
        }

        // a jump only counts once it passed JUMP_MIN_S; then its first part is added at once
        if s.airborne {
            self.air_s += dt;
            if self.air_s >= JUMP_MIN_S {
                if !self.in_jump {
                    self.in_jump = true;
                    self.d.jumps += 1;
                    self.d.air_s += self.air_s as f64;
                } else {
                    self.d.air_s += dt as f64;
                }
                self.longest_jump_s = self.longest_jump_s.max(self.air_s);
            }
        } else {
            (self.air_s, self.in_jump) = (0.0, false);
        }

        // overtake: a car that was ahead last frame is racing behind now
        for c in &mut self.pass_cd {
            *c = (*c - dt).max(0.0);
        }
        for &slot in s.behind {
            if self.prev_ahead.contains(&slot) {
                if self.pass_cd.len() <= slot {
                    self.pass_cd.resize(slot + 1, 0.0);
                }
                self.d.overtakes += (self.pass_cd[slot] <= 0.0) as u32;
                self.pass_cd[slot] = PASS_COOLDOWN;
            }
        }
        self.prev_ahead.clear();
        self.prev_ahead.extend_from_slice(s.ahead);
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct TrackStats {
    /// runs of more than `PLAY_MIN_M`, any mode
    pub plays: u32,
    /// counted races
    pub races: u32,
    pub distance_m: f64,
    pub race_s: f64,
    /// best lap (ms) and the car (file id) that drove it
    pub best_lap: Option<(u32, u32)>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct CarStats {
    pub races: u32,
    pub distance_m: f64,
    pub race_s: f64,
    /// best lap (ms) and the circuit it was driven on
    pub best_lap: Option<(u32, String)>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct TopSpeed {
    pub kmh: f32,
    pub car: u32,
    pub track: String,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Stats {
    pub races: u32,
    pub wins: u32,
    pub podiums: u32,
    pub clean: u32,
    /// the game running, menus included
    pub play_s: f64,
    pub total: Driving,
    pub longest_drift_m: f32,
    pub longest_jump_s: f32,
    pub top_speed: Option<TopSpeed>,
    pub tracks: BTreeMap<String, TrackStats>,
    pub cars: BTreeMap<u32, CarStats>,
}

/// `file` of the stats next to settings.cfg; `None` unless the game persists.
pub fn path(persist: bool) -> Option<PathBuf> {
    let p = crate::front::settings_path().filter(|_| persist)?;
    Some(p.with_file_name("stats.cfg"))
}

impl Stats {
    /// Fold a finished (or abandoned) run in. `place` is the finishing place (0 = win) of a
    /// race that counts as played, `None` for Time Attack, quit and restarted races.
    pub fn add(&mut self, run: &Run, track: &str, car: u32, place: Option<usize>) {
        if run.d.race_s <= 0.0 {
            return;
        }
        let d = &run.d;
        self.total.distance_m += d.distance_m;
        self.total.race_s += d.race_s;
        self.total.overtakes += d.overtakes;
        self.total.car_hits += d.car_hits;
        self.total.wall_hits += d.wall_hits;
        self.total.drift_m += d.drift_m;
        self.total.air_s += d.air_s;
        self.total.jumps += d.jumps;
        self.longest_drift_m = self.longest_drift_m.max(run.longest_drift_m);
        self.longest_jump_s = self.longest_jump_s.max(run.longest_jump_s);
        if run.top_kmh > 0.0 && self.top_speed.as_ref().is_none_or(|t| run.top_kmh > t.kmh) {
            self.top_speed = Some(TopSpeed { kmh: run.top_kmh, car, track: track.to_string() });
        }
        let t = self.tracks.entry(track.to_string()).or_default();
        let c = self.cars.entry(car).or_default();
        t.plays += (d.distance_m > PLAY_MIN_M) as u32;
        (t.distance_m, t.race_s) = (t.distance_m + d.distance_m, t.race_s + d.race_s);
        (c.distance_m, c.race_s) = (c.distance_m + d.distance_m, c.race_s + d.race_s);
        if let Some(ms) = run.best_lap_ms.filter(|&ms| ms > 0) {
            if t.best_lap.is_none_or(|(b, _)| ms < b) {
                t.best_lap = Some((ms, car));
            }
            if c.best_lap.as_ref().is_none_or(|(b, _)| ms < *b) {
                c.best_lap = Some((ms, track.to_string()));
            }
        }
        if let Some(place) = place {
            self.races += 1;
            self.wins += (place == 0) as u32;
            self.podiums += (place < 3) as u32;
            self.clean += (d.car_hits + d.wall_hits == 0) as u32;
            t.races += 1;
            c.races += 1;
        }
    }

    pub fn win_rate(&self) -> Option<f32> {
        (self.races > 0).then(|| self.wins as f32 / self.races as f32)
    }

    pub fn podium_rate(&self) -> Option<f32> {
        (self.races > 0).then(|| self.podiums as f32 / self.races as f32)
    }

    pub fn clean_rate(&self) -> Option<f32> {
        (self.races > 0).then(|| self.clean as f32 / self.races as f32)
    }

    /// The most-played circuit (ties: the most distance driven there).
    pub fn favourite(&self) -> Option<(&str, &TrackStats)> {
        self.tracks
            .iter()
            .filter(|(_, t)| t.plays > 0)
            .max_by(|a, b| a.1.plays.cmp(&b.1.plays).then(a.1.distance_m.total_cmp(&b.1.distance_m)))
            .map(|(n, t)| (n.as_str(), t))
    }

    /// The fastest lap of the whole game: (ms, circuit, car).
    pub fn fastest(&self) -> Option<(u32, &str, u32)> {
        self.tracks.iter().filter_map(|(n, t)| t.best_lap.map(|(ms, car)| (ms, n.as_str(), car))).min_by_key(|x| x.0)
    }

    pub fn to_text(&self) -> String {
        let d = &self.total;
        let mut s = format!(
            "Races {}\nWins {}\nPodiums {}\nClean {}\nPlayTime {}\nDriving distance {} time {} overtakes {} carhits {} wallhits {} drift {} air {} jumps {}\nLongestDrift {}\nLongestJump {}\n",
            self.races, self.wins, self.podiums, self.clean, self.play_s, d.distance_m, d.race_s, d.overtakes, d.car_hits, d.wall_hits, d.drift_m, d.air_s, d.jumps, self.longest_drift_m, self.longest_jump_s
        );
        if let Some(t) = &self.top_speed {
            s += &format!("TopSpeed {} car {} track {}\n", t.kmh, t.car, t.track);
        }
        for (name, t) in &self.tracks {
            let lap = t.best_lap.map_or(String::new(), |(ms, car)| format!(" lap {ms} car {car}"));
            s += &format!("Track plays {} races {} distance {} time {}{lap} name {name}\n", t.plays, t.races, t.distance_m, t.race_s);
        }
        for (id, c) in &self.cars {
            let lap = c.best_lap.as_ref().map_or(String::new(), |(ms, track)| format!(" lap {ms} track {track}"));
            s += &format!("Car {id} races {} distance {} time {}{lap}\n", c.races, c.distance_m, c.race_s);
        }
        s
    }

    /// Parse a stats file; unknown or damaged lines are skipped.
    pub fn parse(text: &str) -> Stats {
        let mut st = Stats::default();
        for line in text.lines() {
            let (key, rest) = line.split_once(' ').unwrap_or((line, ""));
            let rest = rest.trim();
            match key {
                "Races" => st.races = rest.parse().unwrap_or(0),
                "Wins" => st.wins = rest.parse().unwrap_or(0),
                "Podiums" => st.podiums = rest.parse().unwrap_or(0),
                "Clean" => st.clean = rest.parse().unwrap_or(0),
                "PlayTime" => st.play_s = rest.parse().unwrap_or(0.0),
                "LongestDrift" => st.longest_drift_m = rest.parse().unwrap_or(0.0),
                "LongestJump" => st.longest_jump_s = rest.parse().unwrap_or(0.0),
                "Driving" => {
                    let p = Pairs::new(rest);
                    st.total = Driving {
                        distance_m: p.get("distance"),
                        race_s: p.get("time"),
                        overtakes: p.get("overtakes"),
                        car_hits: p.get("carhits"),
                        wall_hits: p.get("wallhits"),
                        drift_m: p.get("drift"),
                        air_s: p.get("air"),
                        jumps: p.get("jumps"),
                    };
                }
                "TopSpeed" => {
                    let Some((head, track)) = rest.split_once(" track ") else { continue };
                    let (kmh, pairs) = head.split_once(' ').unwrap_or((head, ""));
                    let car = Pairs::new(pairs).get::<u32>("car");
                    if let Ok(kmh) = kmh.parse() {
                        st.top_speed = Some(TopSpeed { kmh, car, track: track.trim().to_string() });
                    }
                }
                "Track" => {
                    let Some((head, name)) = rest.split_once(" name ") else { continue };
                    let p = Pairs::new(head);
                    let lap = p.has("lap").then(|| (p.get("lap"), p.get("car")));
                    st.tracks.insert(name.trim().to_string(), TrackStats { plays: p.get("plays"), races: p.get("races"), distance_m: p.get("distance"), race_s: p.get("time"), best_lap: lap });
                }
                "Car" => {
                    let (head, track) = match rest.split_once(" track ") {
                        Some((h, t)) => (h, Some(t.trim().to_string())),
                        None => (rest, None),
                    };
                    let Some((id, head)) = head.split_once(' ') else { continue };
                    let Ok(id) = id.parse::<u32>() else { continue };
                    let p = Pairs::new(head);
                    let lap = track.filter(|_| p.has("lap")).map(|t| (p.get("lap"), t));
                    st.cars.insert(id, CarStats { races: p.get("races"), distance_m: p.get("distance"), race_s: p.get("time"), best_lap: lap });
                }
                _ => {}
            }
        }
        st
    }

    /// Load `stats.cfg`; missing or unreadable = zero.
    pub fn load(path: &std::path::Path) -> Stats {
        std::fs::read_to_string(path).map_or_else(|_| Stats::default(), |t| Stats::parse(&t))
    }

    /// Write via a temporary file, so an interrupted save keeps the old totals.
    pub fn save(&self, path: &std::path::Path) -> std::io::Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let tmp = path.with_extension("cfg.tmp");
        std::fs::write(&tmp, self.to_text())?;
        std::fs::rename(&tmp, path)
    }
}

/// `key value key value ...` of a stats line.
struct Pairs<'a>(Vec<&'a str>);

impl<'a> Pairs<'a> {
    fn new(s: &'a str) -> Self {
        Pairs(s.split_whitespace().collect())
    }

    fn has(&self, key: &str) -> bool {
        self.0.chunks(2).any(|c| c[0] == key && c.len() == 2)
    }

    fn get<T: std::str::FromStr + Default>(&self, key: &str) -> T {
        self.0.chunks(2).find(|c| c[0] == key && c.len() == 2).and_then(|c| c[1].parse().ok()).unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DT: f32 = 0.1;

    fn sample<'a>(ahead: &'a [usize], behind: &'a [usize]) -> FrameSample<'a> {
        FrameSample { running: true, speed: 10.0, kmh: 36.0, drifting: false, airborne: false, car_impact: 0.0, world_impact: 0.0, ahead, behind }
    }

    #[test]
    fn counts_distance_time_and_top_speed_only_while_running() {
        let mut r = Run::default();
        r.frame(DT, &FrameSample { running: false, ..sample(&[], &[]) });
        assert_eq!(r.d.race_s, 0.0);
        r.frame(DT, &sample(&[], &[]));
        r.frame(DT, &FrameSample { kmh: -120.0, ..sample(&[], &[]) });
        assert!((r.d.distance_m - 2.0).abs() < 1e-6 && (r.d.race_s - 0.2).abs() < 1e-6);
        assert_eq!(r.top_kmh, 120.0);
    }

    #[test]
    fn a_scrape_is_one_collision_and_kinds_are_separate() {
        let mut r = Run::default();
        for _ in 0..8 {
            r.frame(DT, &FrameSample { world_impact: 7.0, ..sample(&[], &[]) });
        }
        assert_eq!((r.d.wall_hits, r.d.car_hits), (1, 0));
        for _ in 0..12 {
            r.frame(DT, &sample(&[], &[]));
        }
        r.frame(DT, &FrameSample { world_impact: 7.0, car_impact: 9.0, ..sample(&[], &[]) });
        assert_eq!((r.d.wall_hits, r.d.car_hits), (2, 1));
        // below the thresholds nothing counts
        r.frame(DT, &FrameSample { world_impact: 2.0, car_impact: 3.0, ..sample(&[], &[]) });
        assert_eq!((r.d.wall_hits, r.d.car_hits), (2, 1));
    }

    #[test]
    fn short_airtime_is_a_bump_and_long_airtime_a_jump() {
        let mut r = Run::default();
        for _ in 0..2 {
            r.frame(DT, &FrameSample { airborne: true, ..sample(&[], &[]) });
        }
        r.frame(DT, &sample(&[], &[]));
        assert_eq!((r.d.jumps, r.d.air_s), (0, 0.0));
        for _ in 0..10 {
            r.frame(DT, &FrameSample { airborne: true, ..sample(&[], &[]) });
        }
        r.frame(DT, &sample(&[], &[]));
        assert_eq!(r.d.jumps, 1);
        assert!((r.d.air_s - 1.0).abs() < 1e-4 && (r.longest_jump_s - 1.0).abs() < 1e-4);
    }

    #[test]
    fn drifts_add_distance_and_keep_the_longest() {
        let mut r = Run::default();
        for n in [3, 5] {
            for _ in 0..n {
                r.frame(DT, &FrameSample { drifting: true, ..sample(&[], &[]) });
            }
            r.frame(DT, &sample(&[], &[]));
        }
        assert!((r.d.drift_m - 8.0).abs() < 1e-4 && (r.longest_drift_m - 5.0).abs() < 1e-4);
    }

    #[test]
    fn an_overtake_needs_a_racing_car_that_was_ahead() {
        let mut r = Run::default();
        r.frame(DT, &sample(&[1, 2], &[3]));
        // 1 is now behind; 2 left the race (finished), so it is in neither list
        r.frame(DT, &sample(&[], &[1, 3]));
        assert_eq!(r.d.overtakes, 1);
        // 1 passes back and is passed again at once: the same overtake
        r.frame(DT, &sample(&[1], &[3]));
        r.frame(DT, &sample(&[], &[1, 3]));
        assert_eq!(r.d.overtakes, 1);
    }

    fn run(m: f64, best: Option<u32>) -> Run {
        let mut r = Run::default();
        r.d.distance_m = m;
        r.d.race_s = m / 50.0;
        r.top_kmh = 100.0 + m as f32 / 100.0;
        r.best_lap_ms = best;
        r
    }

    #[test]
    fn only_a_counted_race_is_played_and_everything_driven_counts() {
        let mut s = Stats::default();
        s.add(&run(5000.0, Some(61_000)), "Neo City", 5, Some(0));
        s.add(&run(3000.0, None), "Neo City", 5, None); // time attack or quit
        s.add(&run(4000.0, Some(59_000)), "Docks", 2, Some(2));
        s.add(&run(2000.0, None), "Docks", 2, Some(4));
        assert_eq!((s.races, s.wins, s.podiums), (3, 1, 2));
        assert_eq!(s.win_rate(), Some(1.0 / 3.0));
        assert_eq!(s.total.distance_m, 14_000.0);
        assert_eq!(s.tracks["Neo City"].races, 1);
        assert_eq!(s.tracks["Neo City"].plays, 2);
        assert_eq!(s.cars[&5].distance_m, 8000.0);
        assert_eq!(s.fastest(), Some((59_000, "Docks", 2)));
        assert_eq!(s.cars[&5].best_lap, Some((61_000, "Neo City".to_string())));
        // a run that never left the grid adds nothing
        s.add(&Run::default(), "Docks", 2, Some(0));
        assert_eq!(s.races, 3);
    }

    #[test]
    fn clean_race_has_no_collisions_and_top_speed_keeps_the_best() {
        let mut s = Stats::default();
        let mut a = run(4000.0, None);
        a.d.wall_hits = 1;
        s.add(&a, "Docks", 2, Some(1));
        s.add(&run(4000.0, None), "Docks", 2, Some(1));
        assert_eq!((s.clean, s.races), (1, 2));
        s.add(&run(9000.0, None), "Neo City", 7, None);
        let t = s.top_speed.as_ref().unwrap();
        assert_eq!((t.car, t.track.as_str()), (7, "Neo City"));
        s.add(&run(100.0, None), "Docks", 2, None);
        assert_eq!(s.top_speed.as_ref().unwrap().car, 7);
    }

    #[test]
    fn favourite_is_the_most_played_then_the_most_driven() {
        let mut s = Stats::default();
        s.add(&run(5000.0, None), "A", 1, None);
        s.add(&run(9000.0, None), "B", 1, None);
        assert_eq!(s.favourite().map(|f| f.0), Some("B"));
        s.add(&run(1000.0, None), "A", 1, None);
        assert_eq!(s.favourite().map(|f| f.0), Some("A"));
        assert_eq!(Stats::default().favourite(), None);
    }

    #[test]
    fn text_round_trip_with_spaces_in_names() {
        let mut s = Stats::default();
        let mut r = run(5000.0, Some(61_234));
        r.d.overtakes = 4;
        r.d.drift_m = 12.5;
        r.longest_jump_s = 1.5;
        s.add(&r, "Neo City 2", 5, Some(0));
        s.add(&run(3000.0, None), "Docks", 9, None);
        s.play_s = 1234.5;
        let back = Stats::parse(&s.to_text());
        assert_eq!(back, s);
        assert_eq!(Stats::parse("garbage\nRaces x\nTrack broken"), Stats::default());
    }
}
