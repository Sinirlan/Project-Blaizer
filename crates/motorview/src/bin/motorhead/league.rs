//! League mode (re-notes 08-missing-features §8.8): profiles, the 26 places in four divisions,
//! points, promotion and relegation, track unlocks and the `.LGF` save files.

use motor_data::cfg::Config;
use motor_sim::ai::Temper;
use motor_sim::Driver;
use std::path::{Path, PathBuf};

/// Points per finishing position (0x4b1a48).
pub const POINTS: [u32; 8] = [12, 10, 8, 6, 4, 3, 2, 1];
/// Places per division: Division 0 is a two-car final, Divisions 1-3 hold eight each.
pub const DIV_SIZE: [usize; 4] = [2, 8, 8, 8];
/// First place of each division (league +0xd4, +0x814, +0x2514, +0x4214).
pub const DIV_START: [usize; 4] = [0, 2, 10, 18];
pub const PLACES: usize = 26;

/// A driver profile (`League/Profiles/*.adp`, parser 0x44e748).
#[derive(Debug, Clone, PartialEq)]
pub struct Profile {
    /// PlayerName (26 chars at most)
    pub name: String,
    /// PlayerNick (16 chars at most), the name the league files use for AI places
    pub nick: String,
    /// League/Profiles/Gfx/<PlayerPicture>; Game/player.tga for the human
    pub picture: String,
    pub colour: [u8; 3],
    pub car_colours: [[u8; 3]; 3],
    /// CarDivision0..3: the car driven in each division
    pub cars: [String; 4],
    /// CarHorn, as written (kept for the file)
    pub horn: String,
    pub temper: Temper,
}

impl Default for Profile {
    /// The parser's defaults (0x44e8a8): "Player %d", colour 0xffcc88, car colours 0xffff00.
    fn default() -> Self {
        Profile {
            name: "Player".into(),
            nick: "Player".into(),
            picture: String::new(),
            colour: [0xff, 0xcc, 0x88],
            car_colours: [[0xff, 0xff, 0]; 3],
            cars: Default::default(),
            horn: String::new(),
            temper: Temper::default(),
        }
    }
}

fn joined(c: &Config, k: &str) -> Option<String> {
    c.get(k).map(|e| e.values.join(" "))
}

/// "R G B", 0..255 each.
fn rgb_of(e: &motor_data::cfg::Entry) -> [u8; 3] {
    std::array::from_fn(|i| e.f32(i).unwrap_or(0.0).clamp(0.0, 255.0) as u8)
}

fn rgb(c: &Config, k: &str) -> Option<[u8; 3]> {
    c.get(k).map(rgb_of)
}

fn clip(s: &str, n: usize) -> String {
    s.chars().take(n).collect()
}

impl Profile {
    fn parse(c: &Config, car_names: &[String]) -> Profile {
        let mut p = Profile::default();
        if let Some(v) = joined(c, "PlayerName") {
            p.name = clip(&v, 26);
        }
        if let Some(v) = joined(c, "PlayerNick") {
            p.nick = clip(&v, 16);
        }
        if let Some(v) = joined(c, "PlayerPicture") {
            p.picture = format!("League/Profiles/Gfx/{v}");
        }
        p.colour = rgb(c, "PlayerColour").unwrap_or(p.colour);
        for (k, cc) in p.car_colours.iter_mut().enumerate() {
            *cc = rgb(c, &format!("CarColour{}", k + 1)).unwrap_or(*cc);
        }
        // a car name is kept only when that car exists (0x44f000)
        for (d, car) in p.cars.iter_mut().enumerate() {
            if let Some(v) = joined(c, &format!("CarDivision{d}")).filter(|v| car_names.iter().any(|n| n.eq_ignore_ascii_case(v))) {
                *car = v;
            }
        }
        p.horn = joined(c, "CarHorn").unwrap_or_default();
        // CLASSIC: the key is "Aggressiveness" (0x4d410d); the shipped files write
        // "Aggressivness", so it stays 0.5
        let level = |k: &str| c.get(k).and_then(|e| e.f32(0)).map(|v| (v * 0.01).clamp(0.0, 1.0));
        p.temper = Temper { aggression: level("Aggressiveness").unwrap_or(0.5), eagerness: level("Eagerness").unwrap_or(0.5) };
        p
    }

    pub fn driver(&self) -> Driver {
        Driver { name: self.name.clone(), nick: self.nick.clone(), picture: self.picture.clone(), colour: self.colour, car_colours: Some(self.car_colours), temper: self.temper, ninja_marker: false }
    }
}

/// Every profile in League/Profiles, sorted by file name.
pub fn load_profiles(root: &Path, car_names: &[String]) -> Vec<Profile> {
    let Some(dir) = motor_data::fs::find_ci(root, "League/Profiles") else { return Vec::new() };
    let mut files: Vec<_> = std::fs::read_dir(dir)
        .map(|d| d.flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|x| x.eq_ignore_ascii_case("adp"))).collect())
        .unwrap_or_default();
    files.sort();
    files
        .iter()
        .filter_map(|p| {
            let raw = std::fs::read(p).ok()?;
            let data = motor_data::tbl::load(&raw, None).ok()?.data;
            Some(Profile::parse(&Config::parse(&data), car_names))
        })
        .collect()
}

/// A place in the league: a profile plus its season record (+0x28c..+0x29c).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Place {
    pub human: bool,
    pub p: Profile,
    pub score: u32,
    pub wins: u32,
    pub seconds: u32,
    pub thirds: u32,
}

/// What the race (or the season) meant for the human (0x453940's return code).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    /// more races this season (0)
    Season,
    /// season over: promoted from Division 3 (1), 2 (2) or 1 to the final (3)
    Promoted(usize),
    /// won the final: the league is over (4)
    Won,
    /// season over, same division (5)
    Stays,
    /// relegated from Division 1 or 2 (6)
    Relegated,
    /// lost the final (7)
    LostFinal,
}

impl Outcome {
    /// The original's season code 1..7 (0 = mid-season has none).
    pub fn from_code(code: u32) -> Option<Outcome> {
        Some(match code {
            1 => Outcome::Promoted(3),
            2 => Outcome::Promoted(2),
            3 => Outcome::Promoted(1),
            4 => Outcome::Won,
            5 => Outcome::Stays,
            6 => Outcome::Relegated,
            7 => Outcome::LostFinal,
            _ => return None,
        })
    }

    /// The `movies.pak` entry played for this outcome (0x42aa18: 1->3, 2->4, 3->5, 4->8, 5->6, 6->7, 7->7) [O].
    pub fn movie(self) -> Option<usize> {
        Some(match self {
            Outcome::Season => return None,
            Outcome::Promoted(3) => 3,
            Outcome::Promoted(2) => 4,
            Outcome::Promoted(_) => 5,
            Outcome::Won => 8,
            Outcome::Stays => 6,
            Outcome::Relegated | Outcome::LostFinal => 7,
        })
    }
}

impl Outcome {
    /// REMASTER Results (#94): the big word of the League tab at a season's end.
    pub fn word(self) -> &'static str {
        match self {
            Outcome::Season => "",
            Outcome::Promoted(_) => "PROMOTED",
            Outcome::Won => "CHAMPION",
            Outcome::Stays => "SEASON OVER",
            Outcome::Relegated => "RELEGATED",
            Outcome::LostFinal => "LOST THE FINAL",
        }
    }

    /// The move on the division ladder, from and to (3, 2, 1, 0 = the final; -1 = the league won), for a human who
    /// raced the season in `div` (0x453ccc's swaps: the second of the final drops to Division 1).
    pub fn ladder(self, div: usize) -> (i32, i32) {
        let d = div as i32;
        match self {
            Outcome::Season | Outcome::Stays => (d, d),
            Outcome::Promoted(n) => (n as i32, n as i32 - 1),
            Outcome::Won => (0, -1),
            Outcome::Relegated => (d, (d + 1).min(3)),
            Outcome::LostFinal => (0, 1),
        }
    }

    /// What follows the screen: the season movie's name (shown when the install has the movies).
    pub fn movie_name(self) -> &'static str {
        match self {
            Outcome::Season => "",
            Outcome::Promoted(1) => "TO THE FINAL",
            Outcome::Promoted(_) => "PROMOTED",
            Outcome::Won => "LEAGUE WON",
            Outcome::Stays => "SEASON OVER",
            Outcome::Relegated | Outcome::LostFinal => "RELEGATED",
        }
    }
}

/// A place of the human's division as it stood before a race: who, where it was, where it finished
/// (REMASTER Results, the movement arrows of the League tab).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Moved {
    pub name: String,
    pub human: bool,
    /// the 1-based place in the division before the race
    pub prev: usize,
    /// the 1-based finishing position of the race
    pub finish: u32,
}

impl League {
    /// The human division's order before `finish_race(finish, ..)` re-sorts it (`finish` as there).
    pub fn moved_before(&self, finish: &[(usize, u32)]) -> Vec<Moved> {
        let d = self.human_division();
        self.division(d)
            .iter()
            .enumerate()
            .map(|(k, pl)| Moved { name: pl.p.name.clone(), human: pl.human, prev: k + 1, finish: finish.iter().find(|f| f.0 == DIV_START[d] + k).map_or(0, |f| f.1) })
            .collect()
    }
}

/// The promotion / relegation zone of place `i` (0-based) in division `div` (`season_result`, 0x453b9c):
/// Some(true) goes up, Some(false) goes down. Division 3: places 1-2 up; Division 2: 1-2 up, 7-8 down; Division 1:
/// place 1 up (to the final), 7-8 down; the final has none (its place 1 wins, place 2 loses).
pub fn zone(div: usize, i: usize) -> Option<bool> {
    match div {
        3 => (i < 2).then_some(true),
        2 if i < 2 => Some(true),
        1 if i == 0 => Some(true),
        1 | 2 if i >= 6 => Some(false),
        _ => None,
    }
}

/// What a promotion makes pickable in Single race: the track and car list entries between the picks before and
/// after the tracks `unlock` are flagged (`single_locks`).
pub fn new_unlocks(codes: &[u32], flagged: &[bool], car_codes: &[u32], unlock: &[usize]) -> (std::ops::Range<usize>, std::ops::Range<usize>) {
    let before = single_locks(codes, flagged, car_codes);
    let mut f = flagged.to_vec();
    for &t in unlock {
        if let Some(x) = f.get_mut(t) {
            *x = true;
        }
    }
    let after = single_locks(codes, &f, car_codes);
    (before.tracks_open..after.tracks_open.max(before.tracks_open), before.cars_open..after.cars_open.max(before.cars_open))
}

/// Small LCG with the Watcom rand() shape (15-bit results) [design choice: not the same seed].
pub struct Rng(pub u32);

impl Rng {
    pub fn next(&mut self) -> u32 {
        self.0 = self.0.wrapping_mul(1_103_515_245).wrapping_add(12345);
        (self.0 >> 16) & 0x7fff
    }

    fn below(&mut self, n: usize) -> usize {
        self.next() as usize % n.max(1)
    }
}

/// The CarDivisionN a single race takes its AI cars from for the player's car tier (0x46e110):
/// tier 1 CarDivision3, tier 2 CarDivision2, tiers 3 and 4 CarDivision1.
pub fn single_car_division(player_tier: u32) -> usize {
    match player_tier {
        0 | 1 => 3,
        2 => 2,
        _ => 1,
    }
}

/// The single-race field (0x46e110): `n` distinct random profiles are dealt to random grid slots
/// and you replace the one in the last slot, so the AI slots 0..n-1 hold `n - 1` distinct profiles in
/// random order. Returns their indices into `profiles` by slot. The draw order is ours [I]; the
/// original's rand() calls are not traced.
pub fn single_field(profiles: usize, n: usize, rng: &mut Rng) -> Vec<usize> {
    let n = n.min(profiles);
    let mut drawn: Vec<usize> = Vec::with_capacity(n);
    while drawn.len() < n {
        let p = rng.below(profiles);
        if !drawn.contains(&p) {
            drawn.push(p);
        }
    }
    let mut slots: Vec<usize> = Vec::with_capacity(n);
    while slots.len() < n {
        let s = rng.below(n);
        if !slots.contains(&s) {
            slots.push(s);
        }
    }
    let mut field = vec![0; n];
    for (p, s) in drawn.into_iter().zip(slots) {
        field[s] = p;
    }
    field.pop();
    field
}

/// Track or car `Division` code: tier·100 + n; n >= 10 is a bonus track.
pub fn tier(code: u32) -> u32 {
    code / 100
}

/// The tracks a division races, in track-list order (0x4534b8).
pub fn season_tracks(codes: &[u32], div: usize) -> Vec<usize> {
    codes
        .iter()
        .enumerate()
        .filter(|&(_, &c)| {
            c % 100 < 10
                && match div {
                    3 => tier(c) == 1,
                    2 => tier(c) < 3,
                    1 => tier(c) < 4,
                    _ => tier(c) == 4,
                }
        })
        .map(|(i, _)| i)
        .collect()
}

/// The bonus tracks a division unlocks (0x4535cc).
pub fn bonus_tracks(codes: &[u32], div: usize) -> Vec<usize> {
    codes.iter().enumerate().filter(|&(_, &c)| c % 100 >= 10 && tier(c) as usize == 4 - div.min(4)).map(|(i, _)| i).collect()
}

/// A car may be driven in a division when its tier is at most 4 - division (0x46d595).
pub fn car_allowed(car_code: u32, div: usize) -> bool {
    tier(car_code) as usize <= 4 - div.min(4)
}

#[derive(Debug, Clone, PartialEq)]
pub struct League {
    /// LeagueName (15 chars at most)
    pub name: String,
    /// file stem of the .LGF
    pub file: String,
    /// CreateDate, as YYYYMMDD [I: the original's format is 0x45f924's]
    pub create_date: u32,
    pub race_number: u32,
    pub races_done: u32,
    pub laps_done: u32,
    /// the human's summed race time and best lap this season, ms (0 = none): fill the L*.MHI
    /// tables at the season end [design choice, `SeasonMs` / `SeasonBestLap` keys are ours]
    pub season_ms: u32,
    pub season_lap_ms: u32,
    pub places: Vec<Place>,
}

impl League {
    /// A new league (0x44f8d0): every place a distinct random profile, then a random place of
    /// Division 3 becomes you, with your car as all four CarDivisionN.
    pub fn create(name: Option<&str>, profiles: &[Profile], me: &Profile, rng: &mut Rng, date: u32) -> League {
        let name = match name.map(str::trim).filter(|n| !n.is_empty()) {
            Some(n) => clip(n, 15),
            None => format!("League{}{}", (b'A' + (rng.next() % 25) as u8) as char, (b'A' + (rng.next() % 25) as u8) as char),
        };
        let mut pool: Vec<usize> = (0..profiles.len()).collect();
        let places = (0..PLACES)
            .map(|_| {
                let p = if pool.is_empty() { Profile::default() } else { profiles[pool.swap_remove(rng.below(pool.len()))].clone() };
                Place { p, ..Place::default() }
            })
            .collect();
        let mut l = League { file: name.clone(), name, create_date: date, race_number: 0, races_done: 0, laps_done: 0, season_ms: 0, season_lap_ms: 0, places };
        let at = DIV_START[3] + rng.below(DIV_SIZE[3]);
        l.places[at] = Place { human: true, p: me.clone(), ..Place::default() };
        l
    }

    pub fn division_of(i: usize) -> usize {
        (0..4).rev().find(|&d| i >= DIV_START[d]).unwrap_or(0)
    }

    pub fn human(&self) -> Option<usize> {
        self.places.iter().position(|p| p.human)
    }

    /// The first division holding a human, else 3 (0x4536f8).
    pub fn human_division(&self) -> usize {
        self.human().map_or(3, Self::division_of)
    }

    pub fn division(&self, d: usize) -> &[Place] {
        &self.places[DIV_START[d]..DIV_START[d] + DIV_SIZE[d]]
    }

    /// The track index of the next race, or None when the division has no tracks.
    pub fn next_track(&self, codes: &[u32]) -> Option<usize> {
        let t = season_tracks(codes, self.human_division());
        t.get(self.race_number as usize).or(t.first()).copied()
    }

    /// The human's division as a starting field: (place, grid slot). In Divisions 1-3 place k
    /// starts in slot 7 - k [I: driver field +0xdaf9 read as the grid slot], so the leader
    /// starts last; in the final the AI starts first (0 / 1).
    pub fn field(&self) -> Vec<(usize, usize)> {
        let d = self.human_division();
        let n = DIV_SIZE[d];
        (0..n)
            .map(|k| {
                let i = DIV_START[d] + k;
                let slot = if d == 0 { usize::from(self.places[i].human) } else { n - 1 - k };
                (i, slot)
            })
            .collect()
    }

    /// Add the human's race to the season record (`finished`: race ms, `best_lap`: ms).
    pub fn add_times(&mut self, finished: Option<u32>, best_lap: Option<u32>) {
        self.season_ms = self.season_ms.saturating_add(finished.unwrap_or(0));
        if let Some(t) = best_lap.filter(|&t| t > 0) {
            self.season_lap_ms = if self.season_lap_ms == 0 { t } else { self.season_lap_ms.min(t) };
        }
    }

    /// Apply a race (0x453940). `finish[k]` = the 1-based finishing position of place
    /// `field()[k].0`. Every other division gets a random order. Returns what it meant.
    pub fn finish_race(&mut self, finish: &[(usize, u32)], laps: u32, codes: &[u32], rng: &mut Rng) -> Outcome {
        let hd = self.human_division();
        let mut pos = [0u32; PLACES];
        for &(i, p) in finish {
            pos[i] = p;
        }
        for d in (0..4).filter(|&d| d != hd) {
            // a random permutation of 1..n (0x45193c, per division)
            let mut order: Vec<u32> = (1..=DIV_SIZE[d] as u32).collect();
            for k in (1..order.len()).rev() {
                order.swap(k, rng.below(k + 1));
            }
            for (k, p) in order.into_iter().enumerate() {
                pos[DIV_START[d] + k] = p;
            }
        }
        for (pl, &p) in self.places.iter_mut().zip(pos.iter()) {
            match p {
                1 => pl.wins += 1,
                2 => pl.seconds += 1,
                3 => pl.thirds += 1,
                _ => {}
            }
            // CLASSIC: the no-CD penalty (-11 above 20 points, 0x453a3c) is not ported
            pl.score += p.checked_sub(1).and_then(|k| POINTS.get(k as usize)).copied().unwrap_or(0);
        }
        // bubble sort by score, swapping only when the next is greater (0x453404): stable
        for d in 0..4 {
            self.places[DIV_START[d]..DIV_START[d] + DIV_SIZE[d]].sort_by_key(|p| std::cmp::Reverse(p.score));
        }
        self.race_number += 1;
        // RacesDone / LapsDone: not updated in 0x453940 [I: counted here]
        self.races_done += 1;
        self.laps_done += laps;
        if (self.race_number as usize) < season_tracks(codes, hd).len() {
            return Outcome::Season;
        }
        self.season_result()
    }

    /// The end-of-season code, in the original's order of overrides (0x453b9c..).
    fn season_result(&self) -> Outcome {
        let h = |i: usize| self.places[i].human;
        let mut o = Outcome::Stays;
        if h(DIV_START[3]) || h(DIV_START[3] + 1) {
            o = Outcome::Promoted(3);
        }
        if h(DIV_START[2]) || h(DIV_START[2] + 1) {
            o = Outcome::Promoted(2);
        }
        if h(DIV_START[1]) {
            o = Outcome::Promoted(1);
        }
        if [DIV_START[2] + 6, DIV_START[2] + 7, DIV_START[1] + 6, DIV_START[1] + 7].into_iter().any(h) {
            o = Outcome::Relegated;
        }
        if h(1) {
            o = Outcome::LostFinal;
        }
        if h(0) {
            o = Outcome::Won;
        }
        o
    }

    /// The season is over (0x453ccc): returns the tracks it unlocks (on a promotion or a
    /// win), then zeroes the records and swaps the places between divisions.
    pub fn end_season(&mut self, outcome: Outcome, codes: &[u32]) -> Vec<usize> {
        let d = self.human_division();
        let mut unlock = Vec::new();
        if matches!(outcome, Outcome::Promoted(_) | Outcome::Won) {
            unlock = season_tracks(codes, d);
            unlock.extend(bonus_tracks(codes, d));
        }
        for p in &mut self.places {
            (p.score, p.wins, p.seconds, p.thirds) = (0, 0, 0, 0);
        }
        for (a, b) in [(1, DIV_START[1]), (DIV_START[1] + 6, DIV_START[2]), (DIV_START[1] + 7, DIV_START[2] + 1), (DIV_START[2] + 6, DIV_START[3]), (DIV_START[2] + 7, DIV_START[3] + 1)] {
            self.places.swap(a, b);
        }
        self.race_number = 0;
        (self.season_ms, self.season_lap_ms) = (0, 0);
        unlock
    }

    // ------------------------------------------------------------------ .LGF files

    /// The text the writer 0x450fac produces.
    pub fn to_text(&self) -> String {
        let mut s = String::from("//\tLeagueFile\n//\t(C) Copyright Digital Illusions CE AB 1998\n\n");
        s += "//---------- League ---------";
        s += &format!("\nLeagueName\t\t{}\nCreateDate\t\t{}\nRaceNumber\t\t{}\nRacesDone\t\t{}\nLapsDone\t\t{}\nSeasonMs\t\t{}\nSeasonBestLap\t\t{}\n", self.name, self.create_date, self.race_number, self.races_done, self.laps_done, self.season_ms, self.season_lap_ms);
        s += "\n//---------- Players --------";
        for d in 0..4 {
            s += &format!("\n\n//---------- Division {d} --------");
            for pl in self.division(d) {
                let p = &pl.p;
                if pl.human {
                    s += &format!("\nHumanPlayer\t\t{}", p.name);
                } else {
                    s += &format!("\nAIPlayer\t\t{}", p.nick);
                }
                for (k, car) in p.cars.iter().enumerate() {
                    s += &format!("\nCarDivision{k}\t\t{car}");
                }
                let [r, g, b] = p.colour;
                s += &format!("\nColour\t\t\t{r} {g} {b}\nCarHorn\t\t\t{}\nScore\t\t\t{}\nWins\t\t\t{}\nSeconds\t\t\t{}\nThirds\t\t\t{}\n", p.horn, pl.score, pl.wins, pl.seconds, pl.thirds);
            }
        }
        s
    }

    /// Read a league (0x450204). AIPlayer copies the profile with that nick [I: the compared
    /// field]; HumanPlayer takes `me` with the name from the file.
    pub fn parse(text: &[u8], file: &str, profiles: &[Profile], me: &Profile) -> League {
        let c = Config::parse(text);
        let mut l = League { name: file.into(), file: file.into(), create_date: 0, race_number: 0, races_done: 0, laps_done: 0, season_ms: 0, season_lap_ms: 0, places: Vec::new() };
        let num = |e: &motor_data::cfg::Entry| e.i64(0).unwrap_or(0).max(0) as u32;
        for e in &c.entries {
            let v = e.values.join(" ");
            match e.key.to_ascii_lowercase().as_str() {
                "leaguename" => l.name = clip(&v, 15),
                "createdate" => l.create_date = num(e),
                "racenumber" => l.race_number = num(e),
                "racesdone" => l.races_done = num(e),
                "lapsdone" => l.laps_done = num(e),
                "seasonms" => l.season_ms = num(e),
                "seasonbestlap" => l.season_lap_ms = num(e),
                "humanplayer" | "aiplayer" if l.places.len() < PLACES => {
                    let human = e.key.eq_ignore_ascii_case("humanplayer");
                    let p = if human {
                        Profile { name: clip(&v, 15), nick: clip(&v, 15), ..me.clone() }
                    } else {
                        profiles.iter().find(|p| p.nick.eq_ignore_ascii_case(&v)).cloned().unwrap_or(Profile { name: v.clone(), nick: v, ..Profile::default() })
                    };
                    l.places.push(Place { human, p, ..Place::default() });
                }
                key => {
                    let Some(pl) = l.places.last_mut() else { continue };
                    match key {
                        "colour" => pl.p.colour = rgb_of(e),
                        "carhorn" => pl.p.horn = v,
                        "score" => pl.score = num(e),
                        "wins" => pl.wins = num(e),
                        "seconds" => pl.seconds = num(e),
                        "thirds" => pl.thirds = num(e),
                        k if k.starts_with("cardivision") => {
                            if let Some(d) = k[11..].parse::<usize>().ok().filter(|&d| d < 4) {
                                pl.p.cars[d] = v;
                            }
                        }
                        _ => {}
                    }
                }
            }
        }
        while l.places.len() < PLACES {
            l.places.push(Place::default());
        }
        l
    }

    pub fn path(dir: &Path, file: &str) -> PathBuf {
        dir.join(format!("{file}.LGF"))
    }

    pub fn save(&self, dir: &Path) -> std::io::Result<()> {
        std::fs::create_dir_all(dir)?;
        std::fs::write(Self::path(dir, &self.file), self.to_text().replace('\n', "\r\n"))
    }
}

/// The leagues saved in `dir` (`*.lgf`), sorted by file name.
pub fn load_all(dir: &Path, profiles: &[Profile], me: &Profile) -> Vec<League> {
    let mut files: Vec<_> = std::fs::read_dir(dir)
        .map(|d| d.flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|x| x.eq_ignore_ascii_case("lgf"))).collect())
        .unwrap_or_default();
    files.sort();
    files
        .iter()
        .filter_map(|p| {
            let stem = p.file_stem()?.to_string_lossy().into_owned();
            Some(League::parse(&std::fs::read(p).ok()?, &stem, profiles, me))
        })
        .collect()
}

/// A free file stem for a league name (letters, digits, spaces; a number when taken).
pub fn file_stem(name: &str, taken: &[String]) -> String {
    let base: String = name.chars().filter(|c| c.is_ascii_alphanumeric() || *c == ' ').collect();
    let base = if base.trim().is_empty() { "League".to_string() } else { base.trim().to_string() };
    let mut stem = base.clone();
    let mut n = 1;
    while taken.iter().any(|t| t.eq_ignore_ascii_case(&stem)) {
        stem = format!("{base}{n}");
        n += 1;
    }
    stem
}

/// Where leagues and unlock flags live: `League/` next to settings.cfg [design choice].
pub fn dir(persist: bool) -> Option<PathBuf> {
    persist.then(|| crate::front::settings_path().and_then(|p| p.parent().map(|d| d.join("League")))).flatten()
}

/// The league you played last: `league.cfg` next to settings.cfg, `LastLeague <file stem>`
/// [design choice; the original keeps the current league in memory only].
fn last_path(persist: bool) -> Option<PathBuf> {
    persist.then(|| crate::front::settings_path().and_then(|p| p.parent().map(|d| d.join("league.cfg")))).flatten()
}

pub fn load_last(persist: bool) -> Option<String> {
    read_last(&last_path(persist)?)
}

/// The `LastLeague` of a `league.cfg` file.
pub fn read_last(path: &Path) -> Option<String> {
    let text = std::fs::read_to_string(path).ok()?;
    text.lines().find_map(|l| l.trim().strip_prefix("LastLeague").map(|v| v.trim().to_string())).filter(|v| !v.is_empty())
}

/// The current league's index after league `deleted` was removed from a list that now holds `left` leagues: the same
/// save when an earlier one went, else the one that moved into its place (the last, when it was the last).
pub fn index_after_delete(current: usize, deleted: usize, left: usize) -> usize {
    let at = if deleted < current { current - 1 } else { current };
    at.min(left.saturating_sub(1))
}

pub fn save_last(persist: bool, file: &str) {
    let Some(p) = last_path(persist) else { return };
    let r = p.parent().map_or(Ok(()), std::fs::create_dir_all).and_then(|_| std::fs::write(&p, format!("LastLeague {file}\n")));
    if let Err(e) = r {
        eprintln!("{}: {e}", p.display());
    }
}

/// Track unlocks: `<dir>/<TrackName>.FLG` (the original writes `<BasePath>Data\<name>.FLG`).
pub fn unlock(dir: &Path, track: &str) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    std::fs::write(dir.join(format!("{track}.FLG")), track)
}

/// What the Track and Car screens list: each list is browsed up to a total, and only the first
/// `*_open` entries can be picked; the entries between are drawn dimmed (tracks 0x75104c /
/// 0x751050, cars 0x750c34 / `[G]+0x1c`) [O].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Locks {
    pub tracks: usize,
    pub tracks_open: usize,
    pub cars: usize,
    pub cars_open: usize,
}

impl Locks {
    /// Nothing locked: every entry of both lists can be picked.
    pub fn all(tracks: usize, cars: usize) -> Locks {
        Locks { tracks, tracks_open: tracks, cars, cars_open: cars }
    }
}

/// The single-race locks from the unlock flags (the list builder 0x4613ae, the tier scan
/// 0x4617aa..0x461b74, the car counts 0x4615a7..0x461639): `flagged[i]` = track i has its .FLG,
/// `car_codes` = the Division codes of the car list (sorted ascending) [O].
/// - A tier is complete when all its normal tracks (n < 10) are flagged. The pick count is every
///   track of the complete tiers before the first incomplete one, plus the normal tracks of that
///   tier (its bonus tracks wait), **capped at the browse total** `count[t1] + count[t2] +
///   normals[t3]` (0x461a64, 0x461af0; raised only when the complete tiers hold more). So after
///   Division 1 the cap hides OkkunSpeedway until its own .FLG (the league win) exists. With every
///   tier complete the total is the shown count, and an unflagged last track is dropped from the
///   pick count (0x461b74).
/// - The cars pick up to the tier of the first unflagged normal track (4 when there is none;
///   0x4615a7). The car browse count is one less than the list (the Serpent, 410, 0x461600) and
///   the whole list again when the track total equals the track count (0x46161e). At tier 4 the
///   pick count also drops the Serpent and gets it back only when the total equals the track count
///   and every track is shown: the league win.
pub fn single_locks(codes: &[u32], flagged: &[bool], car_codes: &[u32]) -> Locks {
    let open = |i: usize| flagged.get(i).copied().unwrap_or(true);
    let normal = |c: u32| c % 100 < 10;
    // per tier 1..=4: all tracks, normal tracks, and whether every normal track is flagged
    let count = |t: u32| codes.iter().filter(|&&c| tier(c) == t).count();
    let normals = |t: u32| codes.iter().filter(|&&c| tier(c) == t && normal(c)).count();
    let complete = |t: u32| codes.iter().enumerate().all(|(i, &c)| tier(c) != t || !normal(c) || open(i));
    let mut total = count(1) + count(2) + normals(3);
    let mut shown = 0;
    let first_incomplete = (1..=4).find(|&t| !complete(t));
    for t in 1..=4 {
        if first_incomplete == Some(t) {
            break;
        }
        shown += count(t);
    }
    total = total.max(shown);
    match first_incomplete {
        Some(t) => shown = (shown + normals(t)).min(total),
        None => {
            total = shown;
            if shown > 0 && !open(shown - 1) {
                shown -= 1;
            }
        }
    }
    let tier_cap = (1..=4).find(|&t| codes.iter().enumerate().any(|(i, &c)| tier(c) == t && normal(c) && !open(i))).unwrap_or(4);
    let n = car_codes.len();
    let mut cars_open = car_codes.iter().filter(|&&c| tier(c) <= tier_cap).count();
    if tier_cap == 4 && !(total == codes.len() && total == shown) {
        cars_open = cars_open.saturating_sub(1);
    }
    let cars = if total == codes.len() { n } else { n.saturating_sub(1) };
    Locks { tracks: total, tracks_open: shown, cars: cars.max(cars_open), cars_open }
}

/// The league car screen lists only the cars with n = Division % 100 < 10, which leaves out the
/// Serpent (410): the browse count of the league car init 0x46cb7c [O].
pub fn league_cars(car_codes: &[u32]) -> usize {
    car_codes.iter().filter(|&&c| c % 100 < 10).count()
}

/// The tracks that have a `<TrackName>.FLG` in `dir`.
pub fn flagged(dir: &Path, names: &[&str]) -> Vec<bool> {
    names.iter().map(|n| dir.join(format!("{n}.FLG")).exists()).collect()
}

/// Today as YYYYMMDD (UTC).
pub fn today() -> u32 {
    let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
    // civil-from-days (H. Hinnant)
    let z = (secs / 86400) as i64 + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    (y * 10000 + m * 100 + d) as u32
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The shipped Division codes in track-list order (Track1..Track8).
    const TRACKS: [u32; 8] = [100, 101, 200, 201, 300, 301, 310, 400];

    fn profiles() -> Vec<Profile> {
        (0..26)
            .map(|i| Profile { name: format!("Driver {i}"), nick: format!("Nick{i}"), cars: ["Serpent".into(), "Titan".into(), "R-Build 2".into(), "Resonic".into()], ..Profile::default() })
            .collect()
    }

    fn me() -> Profile {
        Profile { name: "Me".into(), nick: "Me".into(), cars: std::array::from_fn(|_| "Adder Mk2".into()), ..Profile::default() }
    }

    #[test]
    fn moves_zones_and_unlocks() {
        let l = League::create(Some("T"), &profiles(), &me(), &mut Rng(3), 20260101);
        let hd = l.human_division();
        assert_eq!(hd, 3);
        // the human wins, the leader (place 1) comes last: the places before the race are recorded
        let field = l.field();
        let finish: Vec<(usize, u32)> = field.iter().enumerate().map(|(k, &(i, _))| (i, if l.places[i].human { 1 } else { 2 + (k as u32 % 7) })).collect();
        let moved = l.moved_before(&finish);
        assert_eq!(moved.len(), 8);
        assert_eq!(moved.iter().map(|m| m.prev).collect::<Vec<_>>(), (1..=8).collect::<Vec<_>>());
        assert_eq!(moved.iter().filter(|m| m.human).count(), 1);
        assert_eq!(moved.iter().find(|m| m.human).unwrap().finish, 1);
        // zones
        let z = |d: usize| (0..8).map(|i| zone(d, i)).collect::<Vec<_>>();
        assert_eq!(z(3).iter().filter(|x| **x == Some(true)).count(), 2);
        assert!(z(3).iter().all(|x| *x != Some(false)));
        assert_eq!((z(2)[1], z(2)[2], z(2)[6], z(2)[7]), (Some(true), None, Some(false), Some(false)));
        assert_eq!((z(1)[0], z(1)[1], z(1)[7]), (Some(true), None, Some(false)));
        assert_eq!((zone(0, 0), zone(0, 1)), (None, None));
        // ladders
        assert_eq!(Outcome::Promoted(3).ladder(3), (3, 2));
        assert_eq!(Outcome::Promoted(1).ladder(1), (1, 0));
        assert_eq!(Outcome::Relegated.ladder(2), (2, 3));
        assert_eq!(Outcome::Won.ladder(0), (0, -1));
        assert_eq!(Outcome::LostFinal.ladder(0), (0, 1));
        // a Division 3 promotion on a fresh install opens Division 2's tracks and its cars
        let cars = [100, 101, 102, 200, 201, 202, 300, 301, 302, 410];
        let unlock: Vec<usize> = season_tracks(&TRACKS, 3).into_iter().chain(bonus_tracks(&TRACKS, 3)).collect();
        let (t, c) = new_unlocks(&TRACKS, &[false; 8], &cars, &unlock);
        assert_eq!((t, c), (2..4, 3..6));
        // nothing new when everything is open
        let (t, c) = new_unlocks(&TRACKS, &[true; 8], &cars, &unlock);
        assert!(t.is_empty() && c.is_empty());
    }

    #[test]
    fn single_field_is_distinct_and_random() {
        let mut rng = Rng(5);
        let a = single_field(27, 8, &mut rng);
        let b = single_field(27, 8, &mut rng);
        assert_eq!((a.len(), b.len()), (7, 7));
        for f in [&a, &b] {
            let mut d = f.clone();
            d.sort();
            d.dedup();
            assert_eq!(d.len(), 7);
            assert!(f.iter().all(|&p| p < 27));
        }
        assert_ne!(a, b);
        // fewer profiles than the grid: as many cars as there are profiles, one slot left for you
        assert_eq!(single_field(3, 8, &mut rng).len(), 2);
        assert_eq!([1, 2, 3, 4].map(single_car_division), [3, 2, 1, 1]);
    }

    #[test]
    fn single_locks_follow_the_flags() {
        // shipped car tiers in list order: 3 x tier 1, 3 x tier 2, 3 x tier 3, the Serpent
        let cars = [100, 101, 102, 200, 201, 202, 300, 301, 302, 410];
        let f = |n: usize| -> Vec<bool> { (0..8).map(|i| i < n).collect() };
        // (track browse, track pick, car browse, car pick)
        let l = |x: Locks| (x.tracks, x.tracks_open, x.cars, x.cars_open);
        // fresh install: 6 tracks / 9 cars browsed, tier 1 can be picked
        assert_eq!(l(single_locks(&TRACKS, &[false; 8], &cars)), (6, 2, 9, 3));
        // out of Division 3, out of Division 2
        assert_eq!(l(single_locks(&TRACKS, &f(2), &cars)), (6, 4, 9, 6));
        assert_eq!(l(single_locks(&TRACKS, &f(4), &cars)), (6, 6, 9, 9));
        // a bonus track stays locked until its tier is complete
        assert_eq!(l(single_locks(&TRACKS, &f(5), &cars)), (6, 6, 9, 9));
        // out of Division 1 (NolbyHills open, OkkunSpeedway capped away): 7 tracks, no Serpent
        assert_eq!(l(single_locks(&TRACKS, &f(6), &cars)), (7, 7, 9, 9));
        assert_eq!(l(single_locks(&TRACKS, &f(7), &cars)), (7, 7, 9, 9));
        // league won: OkkunSpeedway flagged, everything and the Serpent
        assert_eq!(l(single_locks(&TRACKS, &[true; 8], &cars)), (8, 8, 10, 10));
        // every tier complete but the last (bonus) track unflagged: it is dropped (0x461b74)
        assert_eq!(l(single_locks(&[100, 110], &[true, false], &[100, 410])), (2, 1, 2, 1));
    }

    #[test]
    fn unlock_all_cheat_opens_everything() {
        // R Pettersson / SWE (#310) flags every track in memory: all tracks and cars, Serpent included
        let cars = [100, 101, 102, 200, 201, 202, 300, 301, 302, 410];
        let l = single_locks(&TRACKS, &vec![true; TRACKS.len()], &cars);
        assert_eq!((l.tracks, l.tracks_open, l.cars, l.cars_open), (TRACKS.len(), TRACKS.len(), cars.len(), cars.len()));
    }

    #[test]
    fn league_lists_no_serpent() {
        assert_eq!(league_cars(&[100, 101, 102, 200, 201, 202, 300, 301, 302, 410]), 9);
    }

    #[test]
    fn seasons() {
        assert_eq!(season_tracks(&TRACKS, 3), vec![0, 1]);
        assert_eq!(season_tracks(&TRACKS, 2), vec![0, 1, 2, 3]);
        assert_eq!(season_tracks(&TRACKS, 1), vec![0, 1, 2, 3, 4, 5]);
        assert_eq!(season_tracks(&TRACKS, 0), vec![7]);
        assert_eq!(bonus_tracks(&TRACKS, 1), vec![6]);
        assert!(bonus_tracks(&TRACKS, 3).is_empty());
        assert!(car_allowed(100, 3) && !car_allowed(200, 3) && car_allowed(302, 1) && !car_allowed(410, 1) && car_allowed(410, 0));
    }

    #[test]
    fn create_and_file_round_trip() {
        let mut rng = Rng(7);
        let l = League::create(None, &profiles(), &me(), &mut rng, 19980101);
        assert_eq!(l.places.len(), PLACES);
        assert_eq!(l.human_division(), 3);
        assert_eq!(l.places.iter().filter(|p| p.human).count(), 1);
        let mut nicks: Vec<_> = l.places.iter().filter(|p| !p.human).map(|p| p.p.nick.clone()).collect();
        nicks.sort();
        nicks.dedup();
        assert_eq!(nicks.len(), 25);
        assert!(l.name.starts_with("League") && l.name.len() == 8);
        let back = League::parse(l.to_text().as_bytes(), &l.file, &profiles(), &me());
        assert_eq!(back, l);
    }

    #[test]
    fn a_season_in_division_3() {
        let mut rng = Rng(1);
        let mut l = League::create(Some("Test"), &profiles(), &me(), &mut rng, 0);
        // win both races: the field is division 3, place order = score order
        for race in 0..2 {
            let field = l.field();
            let finish: Vec<_> = field.iter().enumerate().map(|(k, &(i, _))| (i, if l.places[i].human { 1 } else { k as u32 + 2 }.min(8))).collect();
            let o = l.finish_race(&finish, 3, &TRACKS, &mut rng);
            assert_eq!(o, if race == 0 { Outcome::Season } else { Outcome::Promoted(3) });
        }
        assert!(l.places[DIV_START[3]].human);
        assert_eq!(l.places[DIV_START[3]].score, 24);
        assert_eq!(l.places[DIV_START[3]].wins, 2);
        let scores: Vec<_> = l.division(2).iter().map(|p| p.score).collect();
        assert!(scores.windows(2).all(|w| w[0] >= w[1]));
        let unlock = l.end_season(Outcome::Promoted(3), &TRACKS);
        assert_eq!(unlock, vec![0, 1]);
        assert_eq!(l.human_division(), 2);
        assert_eq!(l.race_number, 0);
        assert!(l.places.iter().all(|p| p.score == 0));
        assert_eq!(l.next_track(&TRACKS), Some(0));
    }

    #[test]
    fn result_movie_per_season_code() {
        let movies: Vec<_> = (0..=8).map(|c| Outcome::from_code(c).and_then(Outcome::movie)).collect();
        assert_eq!(movies, [None, Some(3), Some(4), Some(5), Some(8), Some(6), Some(7), Some(7), None]);
    }

    #[test]
    fn grid_puts_the_leader_last() {
        let l = League::create(Some("Grid"), &profiles(), &me(), &mut Rng(3), 0);
        let f = l.field();
        assert_eq!(f.len(), 8);
        assert_eq!(f[0], (DIV_START[3], 7));
        assert_eq!(f[7], (DIV_START[3] + 7, 0));
    }

    #[test]
    fn files_on_disk() {
        let dir = std::env::temp_dir().join(format!("motorhead-league-{}", std::process::id()));
        let l = League::create(Some("Disk"), &profiles(), &me(), &mut Rng(9), 20260101);
        l.save(&dir).unwrap();
        let back = load_all(&dir, &profiles(), &me());
        assert_eq!(back, vec![l]);
        assert_eq!(file_stem("Disk", &["disk".into()]), "Disk1");
        unlock(&dir, "RedRock").unwrap();
        assert!(dir.join("RedRock.FLG").exists());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn the_current_league_stays_the_same_save_when_another_is_deleted() {
        // five leagues, the third (index 2) current; one is deleted, four are left
        assert_eq!(index_after_delete(2, 0, 4), 1, "an earlier one went: the same save, one place up");
        assert_eq!(index_after_delete(2, 1, 4), 1);
        assert_eq!(index_after_delete(2, 3, 4), 2, "a later one went: nothing moves");
        assert_eq!(index_after_delete(2, 2, 4), 2, "the current one went: the next moves into its place");
        assert_eq!(index_after_delete(4, 4, 4), 3, "the last one went: the new last");
        assert_eq!(index_after_delete(0, 0, 0), 0);
    }

    #[test]
    fn dates() {
        assert!(today() > 2020_0101);
    }
}
