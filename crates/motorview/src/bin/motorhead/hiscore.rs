//! Hiscores and split records (re-notes/08-missing-features.md §8.7).
//!
//! Two systems, both raw little-endian files in HiscorePath (ours: `HiScores/` next to
//! settings.cfg, design choice):
//! - `.MHI` top-ten tables (`0x446b00..0x447738`): 10 entries of 0x38 bytes. The Rankings
//!   screen shows them; the race end (`0x46e66a`) inserts the player.
//! - `.CL` / `.CR` split records (`0x4267ec` load, `0x426900` save): the track's best lap and
//!   best race with their checkpoint splits, for the Time Attack gaps (`0x45d23b`).

use std::path::{Path, PathBuf};

use motor_sim::Difficulty;

pub const ENTRIES: usize = 10;
const ENTRY: usize = 0x38;
const TABLE: usize = ENTRIES * ENTRY;
/// the default table in motor.exe (VA 0x4b0390)
const DEFAULTS_AT: usize = 0x4b0390 - 0x4a4000 + 0xa2a00;
/// .CL: lap time, 4 splits, 4 unused dwords; .CR: unused, race time, 5 dwords per lap
const CL_SIZE: usize = 0x24;
const CR_SIZE: usize = 0x234;
pub const CR_LAPS: usize = (CR_SIZE - 8) / 20;

/// One row (`0x447618` writes it).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Entry {
    /// +0: written from config +0x28fdc [U]; the defaults alternate 0/1
    pub flag: u32,
    /// +4, 24 bytes (23 chars)
    pub name: String,
    /// +0x1c, 16 bytes (15 chars)
    pub car: String,
    /// +0x2c: 0 from a race; the defaults hold 13..4 [I: league score]
    pub score: u32,
    /// +0x30 best lap, +0x34 race time, ms
    pub lap_ms: u32,
    pub race_ms: u32,
}

/// Which table: the file name templates at file offset 0xd25aa.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    /// `%sBL%c%c%02ld%s.MHI`: 'M', mirror, 00, track — best laps from any mode
    BestLap,
    /// `%sBR%c%c%02ld%s.MHI`: 'M', mirror, laps, track — finished races, any mode
    BestRace,
    /// `%sSR%c%c%02ld%s.MHI`: difficulty, mirror, laps, track — single races only
    SingleRace,
    /// `%sL%c%c.MHI`: difficulty, '3' / '2' / '1' — league seasons played in Division 3, in
    /// Divisions 2 and 3, or in any (the original only reads these, 0x447028: the season-end
    /// writer is our design choice)
    LeagueDiv3,
    LeagueDiv23,
    LeagueTotal,
}

impl Kind {
    /// The league tables hold season scores (`+0x2c`), not laps; their race time is stored ×100
    /// (`0x4470d8`).
    pub fn is_league(self) -> bool {
        matches!(self, Kind::LeagueDiv3 | Kind::LeagueDiv23 | Kind::LeagueTotal)
    }
}

impl Kind {
    pub fn label(self) -> &'static str {
        // strings at file offset 0xbc6a8
        match self {
            Kind::BestLap => "Best laptime",
            Kind::BestRace => "Best racetime",
            Kind::SingleRace => "Single race",
            Kind::LeagueDiv3 => "Division 3",
            Kind::LeagueDiv23 => "Division 2&3",
            Kind::LeagueTotal => "Total",
        }
    }
}

/// Difficulty and mirror letters (tables at 0x4b0380 "EMH" and 0x4b0388 "NM").
fn diff_char(d: Difficulty) -> char {
    match d {
        Difficulty::Easy => 'E',
        Difficulty::Medium => 'M',
        Difficulty::Hard => 'H',
        // ours: Beginner times are kept apart from Easy's
        Difficulty::Beginner => 'B',
    }
}

/// The table's file name. BL and BR always use 'M' (`0x4b0381`); BL has laps 00.
pub fn file_name(kind: Kind, track: &str, laps: u32, diff: Difficulty, mirror: bool) -> String {
    let m = if mirror { 'M' } else { 'N' };
    match kind {
        Kind::BestLap => format!("BLM{m}00{track}.MHI"),
        Kind::BestRace => format!("BRM{m}{laps:02}{track}.MHI"),
        Kind::SingleRace => format!("SR{}{m}{laps:02}{track}.MHI", diff_char(diff)),
        Kind::LeagueDiv3 => format!("L{}3.MHI", diff_char(diff)),
        Kind::LeagueDiv23 => format!("L{}2.MHI", diff_char(diff)),
        Kind::LeagueTotal => format!("L{}1.MHI", diff_char(diff)),
    }
}

/// HiscorePath (default `HiScores\`, 0x4174e8); None when nothing may be read or written.
pub fn dir(persist: bool) -> Option<PathBuf> {
    let p = crate::front::settings_path().filter(|_| persist)?;
    Some(p.parent()?.join("HiScores"))
}

/// Record race files (`.MDE`, the original's format) are written to `Demos/` next to settings.cfg;
/// the install's own `Demos/` is read as well (Ghost mode).
pub fn demo_dir(persist: bool) -> Option<PathBuf> {
    let p = crate::front::settings_path().filter(|_| persist)?;
    Some(p.parent()?.join("Demos"))
}

/// Our older race recordings (`.mhr`) live beside the hiscores.
pub fn replay_dir(persist: bool) -> Option<PathBuf> {
    let p = crate::front::settings_path().filter(|_| persist)?;
    Some(p.parent()?.join("Replays"))
}

/// Time Attack ghost (`.ghl`): the best lap of a track and car (cars differ in performance, so a
/// ghost is only ever the car you drive).
pub fn ghost_path(persist: bool, track_name: &str, car: u32) -> Option<PathBuf> {
    let p = crate::front::settings_path().filter(|_| persist)?;
    let name: String = track_name.chars().filter(char::is_ascii_alphanumeric).collect();
    Some(p.parent()?.join("Ghosts").join(format!("{name}-car{car:02}.ghl")))
}

fn u32_at(d: &[u8], o: usize) -> u32 {
    d.get(o..o + 4).map_or(0, |b| u32::from_le_bytes(b.try_into().unwrap()))
}

fn str_at(d: &[u8], o: usize, n: usize) -> String {
    let s = &d[o..o + n];
    let end = s.iter().position(|&c| c == 0).unwrap_or(n);
    s[..end].iter().map(|&c| c as char).collect() // Latin-1
}

fn put_str(d: &mut [u8], s: &str, n: usize) {
    for (i, c) in s.chars().take(n - 1).enumerate() {
        d[i] = if (c as u32) < 256 { c as u8 } else { b'?' };
    }
}

#[derive(Clone, Debug)]
pub struct Table {
    pub entries: Vec<Entry>,
    dirty: bool,
}

impl Table {
    fn parse(d: &[u8]) -> Vec<Entry> {
        (0..ENTRIES)
            .map(|i| {
                let e = &d[i * ENTRY..(i + 1) * ENTRY];
                Entry {
                    flag: u32_at(e, 0),
                    name: str_at(e, 4, 24),
                    car: str_at(e, 0x1c, 16),
                    score: u32_at(e, 0x2c),
                    lap_ms: u32_at(e, 0x30),
                    race_ms: u32_at(e, 0x34),
                }
            })
            .collect()
    }

    fn bytes(&self) -> Vec<u8> {
        let mut d = vec![0u8; TABLE];
        for (i, e) in self.entries.iter().enumerate() {
            let r = &mut d[i * ENTRY..(i + 1) * ENTRY];
            r[0..4].copy_from_slice(&e.flag.to_le_bytes());
            put_str(&mut r[4..0x1c], &e.name, 24);
            put_str(&mut r[0x1c..0x2c], &e.car, 16);
            r[0x2c..0x30].copy_from_slice(&e.score.to_le_bytes());
            r[0x30..0x34].copy_from_slice(&e.lap_ms.to_le_bytes());
            r[0x34..0x38].copy_from_slice(&e.race_ms.to_le_bytes());
        }
        d
    }

    /// The built-in table of motor.exe (10 rows of "DI dude n") with the race time scaled by `scale`.
    pub fn defaults(root: &Path, scale: u32) -> Table {
        let mut entries = match motor_data::fs::read(root, "motor.exe") {
            Ok(exe) if exe.len() >= DEFAULTS_AT + TABLE => Self::parse(&exe[DEFAULTS_AT..DEFAULTS_AT + TABLE]),
            _ => vec![Entry { lap_ms: 240_000, race_ms: 240_000, ..Entry::default() }; ENTRIES],
        };
        for e in &mut entries {
            e.race_ms = e.race_ms.saturating_mul(scale);
        }
        Table { entries, dirty: false }
    }

    /// The defaults from motor.exe with the race time scaled by `scale` (the laps for BR and
    /// SR, `0x447213`; BL keeps them), then the file over them when there is one
    /// (`0x447028`: at most 0x230 bytes are read).
    pub fn load(root: &Path, dir: Option<&Path>, name: &str, scale: u32) -> Table {
        let mut entries = Self::defaults(root, scale).entries;
        if let Some(d) = dir.and_then(|d| std::fs::read(d.join(name)).ok()) {
            let mut buf = Table { entries: entries.clone(), dirty: false }.bytes();
            let n = d.len().min(TABLE);
            buf[..n].copy_from_slice(&d[..n]);
            entries = Self::parse(&buf);
        }
        Table { entries, dirty: false }
    }

    /// 1-based place for a race time (`0x4475c0`) or lap time (`0x4475ec`): before the first
    /// entry it beats outright; 0 = not in the table.
    pub fn place(&self, ms: u32, lap: bool) -> usize {
        self.entries.iter().position(|e| ms < if lap { e.lap_ms } else { e.race_ms }).map_or(0, |i| i + 1)
    }

    /// 1-based place for a league score: before the first entry with a lower score, so ties go
    /// below (the defaults hold 13..4); 0 = not in the table.
    pub fn place_score(&self, score: u32) -> usize {
        self.entries.iter().position(|e| score > e.score).map_or(0, |i| i + 1)
    }

    /// Shift the rows below down and write the new one (`0x447738`).
    pub fn insert(&mut self, place: usize, e: Entry) {
        if (1..=ENTRIES).contains(&place) {
            self.entries.insert(place - 1, e);
            self.entries.truncate(ENTRIES);
            self.dirty = true;
        }
    }

    /// Written only when changed (`0x44753c`).
    pub fn save(&self, dir: &Path, name: &str) -> std::io::Result<()> {
        if !self.dirty {
            return Ok(());
        }
        std::fs::create_dir_all(dir)?;
        std::fs::write(dir.join(name), self.bytes())
    }
}

/// REMASTER Results (#94): one ranking table as the race left it, for the Records tab.
#[derive(Clone, Debug)]
pub struct RankView {
    pub kind: Kind,
    /// the ten rows after the player's new one went in
    pub rows: Vec<Entry>,
    /// 1-based place of the player's new row; 0 = it did not make the top ten
    pub at: usize,
    /// a best lap that beat the first row of the table when that row was an earlier run (not a default)
    pub pb: bool,
    /// per row: a built-in "DI dude" row of motor.exe
    pub default: Vec<bool>,
    /// the table ranks lap times (else race times)
    pub lap: bool,
}

impl RankView {
    /// `before` = the table as loaded, `after` = with the new row inserted at `at` (`Table::place`), `defaults` = the
    /// built-in rows (`Table::defaults`) to tell them from earlier runs.
    pub fn new(kind: Kind, lap: bool, at: usize, before: &Table, after: &Table, defaults: &Table) -> RankView {
        let default = after.entries.iter().map(|e| defaults.entries.contains(e)).collect();
        let first_was_run = before.entries.first().is_some_and(|e| !defaults.entries.contains(e));
        RankView { kind, rows: after.entries.clone(), at, pb: lap && at == 1 && first_was_run, default, lap }
    }
}

/// The track's best lap: its time and the lap-relative times at the lap start and the three
/// checkpoints (`0x4259e4`; split 0 is always 0).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LapRecord {
    pub lap_ms: u32,
    pub splits: [u32; 4],
}

/// The best race over this many laps: per lap, the race time at its end and at its four
/// splits (`0x425a3c`).
#[derive(Clone, Debug, PartialEq)]
pub struct RaceRecord {
    pub race_ms: u32,
    pub laps: Vec<[u32; 5]>,
}

#[derive(Clone, Debug, Default)]
pub struct Records {
    pub lap: Option<LapRecord>,
    pub race: Option<RaceRecord>,
}

impl Records {
    fn names(track: &str, laps: u32) -> (String, String) {
        // "%s%s.CL" / "%s%s%02ld.CR" (file offset 0xd1118)
        (format!("{track}.CL"), format!("{track}{laps:02}.CR"))
    }

    /// Both files must have exactly their size, as in `0x4267ec`.
    pub fn load(dir: Option<&Path>, track: &str, laps: u32) -> Records {
        let Some(dir) = dir else { return Records::default() };
        let (cl, cr) = Self::names(track, laps);
        let lap = std::fs::read(dir.join(cl)).ok().filter(|d| d.len() == CL_SIZE).map(|d| LapRecord {
            lap_ms: u32_at(&d, 0),
            splits: std::array::from_fn(|i| u32_at(&d, 4 + i * 4)),
        });
        let race = std::fs::read(dir.join(cr)).ok().filter(|d| d.len() == CR_SIZE).map(|d| RaceRecord {
            race_ms: u32_at(&d, 4),
            laps: (0..(laps as usize).min(CR_LAPS)).map(|l| std::array::from_fn(|k| u32_at(&d, 8 + l * 20 + k * 4))).collect(),
        });
        Records { lap, race }
    }

    pub fn save(&self, dir: &Path, track: &str, laps: u32) -> std::io::Result<()> {
        let (cl, cr) = Self::names(track, laps);
        std::fs::create_dir_all(dir)?;
        if let Some(l) = &self.lap {
            let mut d = vec![0u8; CL_SIZE];
            d[0..4].copy_from_slice(&l.lap_ms.to_le_bytes());
            for (i, s) in l.splits.iter().enumerate() {
                d[4 + i * 4..8 + i * 4].copy_from_slice(&s.to_le_bytes());
            }
            std::fs::write(dir.join(cl), d)?;
        }
        if let Some(r) = &self.race {
            let mut d = vec![0u8; CR_SIZE];
            d[4..8].copy_from_slice(&r.race_ms.to_le_bytes());
            for (l, lap) in r.laps.iter().take(CR_LAPS).enumerate() {
                for (k, v) in lap.iter().enumerate() {
                    let o = 8 + l * 20 + k * 4;
                    d[o..o + 4].copy_from_slice(&v.to_le_bytes());
                }
            }
            std::fs::write(dir.join(cr), d)?;
        }
        Ok(())
    }

    /// A lap was completed (`0x425b44` → `0x4259e4`): keep it when it beats the record.
    /// `splits`: race ms at index lap*4 + k (lap from 0, k 0 = lap start; front.rs `lap_splits`).
    pub fn lap_done(&mut self, splits: &[Option<u32>], lap: usize) -> bool {
        let at = |k: usize| splits.get(lap * 4 + k).copied().flatten();
        let (Some(start), Some(end)) = (at(0), at(4)) else { return false };
        let lap_ms = end.saturating_sub(start);
        if self.lap.is_some_and(|r| r.lap_ms <= lap_ms) {
            return false;
        }
        let splits = std::array::from_fn(|k| at(k).map_or(0, |t| t.saturating_sub(start)));
        self.lap = Some(LapRecord { lap_ms, splits });
        true
    }

    /// The race was finished (`0x425a3c`).
    pub fn race_done(&mut self, splits: &[Option<u32>], laps: u32, race_ms: u32) -> bool {
        if self.race.as_ref().is_some_and(|r| r.race_ms <= race_ms) {
            return false;
        }
        let at = |i: usize| splits.get(i).copied().flatten().unwrap_or(0);
        let laps = (0..laps as usize).map(|l| [if l + 1 == laps as usize { race_ms } else { at((l + 1) * 4) }, at(l * 4), at(l * 4 + 1), at(l * 4 + 2), at(l * 4 + 3)]).collect();
        self.race = Some(RaceRecord { race_ms, laps });
        true
    }

    /// Time Attack gaps at split `idx` (lap*4 + k, k 1..=3 checkpoints, 4 = the line), in ms
    /// against: this race's best earlier lap ("Lap", `0x42645c`, from lap 2), the record lap
    /// ("Lap Ever", `0x426500`) and the record race ("Race Ever", `0x426558`).
    pub fn gaps(&self, splits: &[Option<u32>], idx: usize) -> [Option<i64>; 3] {
        let at = |i: usize| splits.get(i).copied().flatten().map(|t| t as i64);
        let lap = (idx.max(1) - 1) / 4;
        let k = idx - lap * 4;
        let (Some(now), Some(start)) = (at(idx), at(lap * 4)) else { return [None; 3] };
        let in_lap = now - start;
        let best = (0..lap).filter_map(|l| Some((at(l * 4 + 4)? - at(l * 4)?, l))).min().and_then(|(_, l)| Some(at(l * 4 + k)? - at(l * 4)?));
        let ever_lap = self.lap.map(|r| if k == 4 { r.lap_ms } else { r.splits[k] } as i64);
        let ever_race = self.race.as_ref().and_then(|r| r.laps.get(lap)).map(|l| if k == 4 { l[0] } else { l[k + 1] } as i64);
        [best.map(|b| in_lap - b), ever_lap.map(|b| in_lap - b), ever_race.map(|b| now - b)]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn table_round_trip_and_place() {
        let mut t = Table { entries: (0..10).map(|i| Entry { name: format!("n{i}"), race_ms: 1000 * (i + 1), lap_ms: 500, ..Entry::default() }).collect(), dirty: false };
        assert_eq!(Table::parse(&t.bytes()), t.entries);
        assert_eq!(t.place(999, false), 1);
        assert_eq!(t.place(1000, false), 2); // ties go below
        assert_eq!(t.place(20_000, false), 0);
        t.insert(2, Entry { name: "me".into(), race_ms: 1500, ..Entry::default() });
        assert_eq!(t.entries[1].name, "me");
        assert_eq!(t.entries.len(), 10);
        assert_eq!(t.entries[9].name, "n8");
    }

    #[test]
    fn records_and_gaps() {
        // two laps of 60 s with checkpoints every 15 s, then a 50 s lap
        let s: Vec<Option<u32>> = [0, 15, 30, 45, 60, 75, 90, 105, 110].iter().map(|&x| Some(x * 1000)).collect();
        let mut r = Records::default();
        assert!(r.lap_done(&s, 0));
        assert!(!r.lap_done(&s, 0));
        assert_eq!(r.lap.unwrap().splits, [0, 15_000, 30_000, 45_000]);
        assert!(r.race_done(&s, 2, 110_000));
        assert_eq!(r.race.as_ref().unwrap().laps[1], [110_000, 60_000, 75_000, 90_000, 105_000]);
        // lap 2, checkpoint 1: 15 s into the lap, like lap 1 and the record lap
        assert_eq!(r.gaps(&s, 5), [Some(0), Some(0), Some(0)]);
        // the line after lap 2: 50 s against 60 s
        assert_eq!(r.gaps(&s, 8), [Some(-10_000), Some(-10_000), Some(0)]);
    }

    /// The sector delta (#409) is `gaps()[0]`: the time into the lap against the split of the best earlier lap, with the
    /// slots as the game writes them (+0 lap start, +1 never, +2 / +3 checkpoints, the next lap's +0 the line).
    #[test]
    fn sector_delta_against_the_best_earlier_lap() {
        let s: Vec<Option<u32>> = [Some(0), None, Some(25), Some(50), Some(80), None, Some(102), Some(134), Some(156), None, Some(180)].iter().map(|x| x.map(|t| t * 1000)).collect();
        let r = Records::default();
        // lap 1 has no earlier lap
        assert_eq!(r.gaps(&s, 2)[0], None);
        assert_eq!(r.gaps(&s, 4)[0], None);
        // lap 2 against lap 1: sector 1 22 s vs 25 s, sectors 1 + 2 54 s vs 50 s, the line 76 s vs 80 s
        assert_eq!(r.gaps(&s, 6)[0], Some(-3000));
        assert_eq!(r.gaps(&s, 7)[0], Some(4000));
        assert_eq!(r.gaps(&s, 8)[0], Some(-4000));
        // lap 3 against the best of laps 1 and 2 (lap 2): 24 s vs 22 s
        assert_eq!(r.gaps(&s, 10)[0], Some(2000));
    }

    fn entry(name: &str, ms: u32) -> Entry {
        Entry { name: name.into(), car: "TITAN".into(), lap_ms: ms, race_ms: ms, ..Entry::default() }
    }

    fn table(rows: &[(&str, u32)]) -> Table {
        Table { entries: rows.iter().map(|&(n, t)| entry(n, t)).collect(), dirty: false }
    }

    #[test]
    fn rank_view_marks_new_row_default_rows_and_pb() {
        let defaults = table(&[("DI dude 1", 240_000), ("DI dude 2", 241_000), ("DI dude 3", 242_000)]);
        // one earlier run, then the defaults
        let before = table(&[("SIN", 60_000), ("DI dude 1", 240_000), ("DI dude 2", 241_000)]);
        // 59 s: a new first row and a personal best
        let mut after = before.clone();
        let at = after.place(59_000, true);
        after.insert(at, entry("SIN", 59_000));
        let v = RankView::new(Kind::BestLap, true, at, &before, &after, &defaults);
        assert_eq!((v.at, v.pb), (1, true));
        assert_eq!(v.default, [false, false, true, true]);
        // 61 s: second place, no PB
        let mut after = before.clone();
        let at = after.place(61_000, true);
        after.insert(at, entry("SIN", 61_000));
        let v = RankView::new(Kind::BestLap, true, at, &before, &after, &defaults);
        assert_eq!((v.at, v.pb), (2, false));
        // against a table of defaults only, a first place is not a PB
        let before = defaults.clone();
        let mut after = before.clone();
        let at = after.place(100_000, true);
        after.insert(at, entry("SIN", 100_000));
        assert!(!RankView::new(Kind::BestLap, true, at, &before, &after, &defaults).pb);
    }

    #[test]
    fn rank_view_outside_the_table() {
        let defaults = table(&[("DI dude 1", 240_000), ("DI dude 2", 241_000)]);
        let before = defaults.clone();
        let mut after = before.clone();
        let at = after.place(300_000, true);
        assert_eq!(at, 0);
        after.insert(at, entry("SIN", 300_000));
        let v = RankView::new(Kind::BestLap, true, at, &before, &after, &defaults);
        assert_eq!((v.at, v.pb, v.rows.len()), (0, false, 2));
        assert!(v.default.iter().all(|&d| d));
    }
}
