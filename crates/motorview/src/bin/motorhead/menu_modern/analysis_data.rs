//! REMASTER Lap Analysis (#429 phase 3/4): what the screen of `analysis.rs` shows, apart from the drawing. Two sides (A left,
//! B right), each one lap of a ghost (`.ghl`), race replay (`.mhr`) or demo (`.MDE`) of the screen's track, worked out by
//! `motor_sim::lap_compare` and compared on its distance grid. The picker's rows are the player's own laps of the track:
//! the Time Attack ghosts, the Ghost screen's demos (each car's best lap) and the race replays (newest first; one row per
//! car once re-simulated). A race replay is re-simulated on a worker thread (`Job`); files read and refusals are kept for
//! the session, so a replay found outdated stays marked in the list. Outside files (phase 4/4) come in through the picker's
//! BROWSE FILES row (the system file dialog, run by `App`) or dropped onto the window; one from outside the game's folders
//! may be copied into them (KEEP THIS LAP?). Only that copy writes a file.

use super::replays::ymd;
use super::*;
use motor_sim::lap_compare::{self, Comparison, Kind, LapFile, LapTrace, Refusal, Source};
use motor_sim::race::TrackProfile;
use std::collections::HashSet;
use std::io::{Read, Write};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Arc;

/// The largest file the screen reads, far above a two-hour race replay (design choice: a hostile file is not read whole).
const MAX_BYTES: u64 = 64 << 20;
/// The start of a race replay the picker reads for its header (`Header::peek`).
const PEEK_BYTES: u64 = 4096;

/// Which lap of a file a side asks for: a slot (and its lap), or a car / driver name and lap (`LapFile::pick`); all
/// `None` = the recorder's best flying lap.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Want {
    pub slot: Option<usize>,
    pub car: Option<String>,
    pub lap: Option<u32>,
}

/// A side's lap and what its card says.
#[derive(Clone, Debug)]
pub(crate) struct SideLap {
    pub path: PathBuf,
    pub trace: LapTrace,
    pub driver: String,
    pub car: String,
    /// the file's date `yyyy-mm-dd`; a demo's `NAME · date`
    pub note: String,
}

/// A picker row: one car's best lap of a file, or a race replay not re-simulated yet (its recorder's car, from the header).
#[derive(Clone, Debug)]
pub(crate) struct Cand {
    pub path: PathBuf,
    pub kind: Kind,
    /// the car's slot; `None` = a race replay not read yet
    pub slot: Option<usize>,
    /// the race lap of a demo / replay row
    pub lap: Option<u32>,
    pub driver: String,
    pub car: String,
    pub lap_ms: Option<f32>,
    pub date: String,
    /// the file name without its extension (a demo's name)
    pub stem: String,
    /// refused this session (an outdated race replay, or one without a complete lap)
    pub outdated: bool,
    /// the file's time, seconds (replays list newest first)
    order: u64,
}

/// The requesters of the screen (texts of the mockup's `ASKS`, with the real file and times). One OK button.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum AnaAsk {
    /// a race replay that no longer drives the race it recorded
    Outdated { file: String, best_lap: bool, recorded: Option<u32>, replayed: Option<u32> },
    /// not a ghost, race replay or demo (`damaged`: one of them, but broken)
    NotARecording { file: String, damaged: bool },
    /// a lap of another track (`its`) than the screen's (`ours`); `other` = the side it is compared with
    Track { file: String, its: String, ours: String, other: usize },
    /// nothing to compare in it (no complete lap, no such car or lap, unreadable): `why` from `Refusal`'s text
    NoLap { file: String, why: String },
    /// a lap loaded from outside the game's folders: offer a copy in `folder` (`Ghosts` / `Replays` / `Demos`)
    Keep { file: String, from: String, folder: &'static str, path: PathBuf },
    /// the copy failed
    NotKept { file: String, folder: &'static str, why: String },
}

/// A requester's text as runs: (text, bold). Bold is the mockup's `<b>` (Rajdhani Bold, white).
pub(crate) type Runs = Vec<(String, bool)>;

/// `m.sss` (or `m:ss.sss`) of a lap time in ms.
pub(crate) fn lap_secs(ms: u32) -> String {
    let (m, s, f) = (ms / 60_000, ms / 1000 % 60, ms % 1000);
    if m > 0 { format!("{m}:{s:02}.{f:03}") } else { format!("{s}.{f:03}") }
}

impl AnaAsk {
    pub(crate) fn title(&self) -> &'static str {
        match self {
            AnaAsk::Outdated { .. } => "OUTDATED REPLAY",
            AnaAsk::NotARecording { .. } => "NOT A RECORDING",
            AnaAsk::Track { .. } => "DIFFERENT TRACK",
            AnaAsk::NoLap { .. } => "CANNOT COMPARE",
            AnaAsk::Keep { .. } => "KEEP THIS LAP?",
            AnaAsk::NotKept { .. } => "NOT KEPT",
        }
    }

    /// The icon of the title: the floppy for the question, else the warning triangle.
    pub(crate) fn icon(&self) -> char {
        if matches!(self, AnaAsk::Keep { .. }) { ui_font::icon::SAVE } else { ui_font::icon::ALERT }
    }

    /// The buttons: KEEP A COPY / NOT NOW for the question, else OK.
    pub(crate) fn buttons(&self) -> Vec<(char, String)> {
        if matches!(self, AnaAsk::Keep { .. }) {
            vec![(ui_font::icon::SAVE, "KEEP A COPY".into()), (ui_font::icon::CIRCLE_X, "NOT NOW".into())]
        } else {
            vec![(ui_font::icon::CHECK, "OK".into())]
        }
    }

    pub(crate) fn runs(&self) -> Runs {
        let r = |s: &str| (s.to_string(), false);
        let b = |s: &str| (s.to_string(), true);
        let t = |ms: &Option<u32>| ms.map_or("no lap".to_string(), lap_secs);
        match self {
            AnaAsk::Outdated { file, best_lap, recorded, replayed } => {
                let what = if *best_lap { "best lap" } else { "race time" };
                vec![
                    b(file),
                    r(&format!(" was recorded with older car physics: replayed today it no longer drives the same race (the recorded {what} was ")),
                    b(&t(recorded)),
                    r(", the replay gives "),
                    b(&t(replayed)),
                    r("). It cannot be compared."),
                ]
            }
            AnaAsk::NotARecording { file, damaged: false } => {
                vec![b(file), r(" is not a Motorhead ghost (.ghl), race replay (.mhr) or demo (.MDE). Nothing was loaded.")]
            }
            AnaAsk::NotARecording { file, damaged: true } => {
                vec![b(file), r(" looks like a Motorhead recording, but it is damaged. Nothing was loaded.")]
            }
            // the mockup's "or change side A first" is left out: the track is chosen on the Ghost screen (design choice)
            AnaAsk::Track { file, its, ours, other } => vec![
                b(file),
                r(" is a lap of "),
                b(its),
                r(". Only laps of "),
                b(ours),
                r(&format!(" can be compared with side {}. Pick a file of the same track.", ["A", "B"][(*other).min(1)])),
            ],
            AnaAsk::NoLap { file, why } => vec![b(file), r(&format!(": {why}. Nothing was loaded."))],
            AnaAsk::Keep { file, from, folder, .. } => vec![
                b(file),
                r(" came from "),
                b(from),
                r(&format!(". Keep a copy in your {folder} folder, so it is in the list next time? Your own {} are never overwritten.", folder.to_lowercase())),
            ],
            AnaAsk::NotKept { file, folder, why } => vec![b(file), r(&format!(" could not be copied into your {folder} folder ({why}). The lap is still loaded."))],
        }
    }
}

/// A race replay being re-simulated (`lap_compare::read` on a worker thread). Dropping it cancels the work.
pub(crate) struct Job {
    pub side: usize,
    pub path: PathBuf,
    want: Want,
    rx: mpsc::Receiver<Result<LapFile, Refusal>>,
    progress: Arc<AtomicU32>,
    cancel: Arc<AtomicBool>,
}

impl Job {
    /// How far the re-simulation is, 0..1.
    pub(crate) fn frac(&self) -> f32 {
        f32::from_bits(self.progress.load(Ordering::Relaxed)).clamp(0.0, 1.0)
    }
}

impl Drop for Job {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}

/// The screen's state; it lives on `Game` for the session, so its caches outlast a visit.
#[derive(Default)]
pub(crate) struct Analysis {
    /// the screen's track (`Game::tracks` index)
    pub track: Option<usize>,
    pub sides: [Option<SideLap>; 2],
    /// the selected side (its card has the rim; Enter opens its picker)
    pub side: usize,
    /// the cursor's grid point
    pub cursor: Option<usize>,
    /// the open picker's row (the list of `list`)
    pub picker: Option<usize>,
    pub ask: Option<AnaAsk>,
    pub cmp: Option<Comparison>,
    pub list: Vec<Cand>,
    pub job: Option<Job>,
    /// headless shots: race replays re-simulate on the spot
    pub sync: bool,
    /// headless `--analysis-overlay working`: the working overlay at this share on the selected side
    pub fake_work: Option<f32>,
    /// Wayland: winit 0.30 delivers no dropped files there, so the picker offers BROWSE FILES only
    pub wayland: bool,
    /// a file is being dragged over the window (the drop overlay)
    pub hover: bool,
    /// the picker's BROWSE FILES was chosen for this side: `App` takes it and opens the file dialog
    browse: Option<usize>,
    /// the file dialog is open (the screen waits for it)
    pub dialog: bool,
    /// the selected button of the KEEP THIS LAP? requester
    pub ask_btn: usize,
    /// when the last file was dropped (the other files of one drop come as events right after it)
    dropped: Option<std::time::Instant>,
    /// laps that came through BROWSE FILES / a drop from outside the game's folders (their card says FROM <folder>)
    ext: HashSet<PathBuf>,
    /// the outside file whose lap, once loaded, asks KEEP THIS LAP?
    offer: Option<PathBuf>,
    /// outside files already answered (kept or NOT NOW): not asked again this session
    answered: HashSet<PathBuf>,
    files: HashMap<PathBuf, Arc<LapFile>>,
    refused: HashMap<PathBuf, Refusal>,
}

/// The requester for a refusal (`Cancelled` gets none).
fn refusal_ask(path: &Path, r: &Refusal) -> AnaAsk {
    let file = file_name(path);
    match r {
        Refusal::NotARecording => AnaAsk::NotARecording { file, damaged: false },
        Refusal::Damaged(_) => AnaAsk::NotARecording { file, damaged: true },
        Refusal::Outdated { best_lap, recorded_ms, replayed_ms } => AnaAsk::Outdated { file, best_lap: *best_lap, recorded: *recorded_ms, replayed: *replayed_ms },
        Refusal::NoLap | Refusal::Cancelled | Refusal::Failed(_) => AnaAsk::NoLap { file, why: r.to_string() },
    }
}

/// A folder as the card and the requester name it: under the home folder as `~/…`.
fn folder_label(dir: &Path) -> String {
    let home = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")).map(PathBuf::from);
    match home.as_ref().and_then(|h| dir.strip_prefix(h).ok()) {
        Some(r) => std::iter::once("~".to_string()).chain(r.components().map(|c| c.as_os_str().to_string_lossy().into_owned())).collect::<Vec<_>>().join("/"),
        None => dir.display().to_string(),
    }
}

/// `label` shortened from the left to at most `max` characters for the card's one line: whole folders from the end behind
/// `…/`, the last one cut when it alone is too long.
fn tail(label: &str, max: usize) -> String {
    if label.chars().count() <= max {
        return label.to_string();
    }
    let mut keep = String::new();
    for part in label.rsplit('/') {
        let next = if keep.is_empty() { part.to_string() } else { format!("{part}/{keep}") };
        if next.chars().count() + 2 > max {
            break;
        }
        keep = next;
    }
    if keep.is_empty() {
        let n = label.chars().count();
        return format!("\u{2026}{}", label.chars().skip(n - (max - 1)).collect::<String>());
    }
    format!("\u{2026}/{keep}")
}

/// The start of a race replay's file name for a track (`Replays/<Track>-*.mhr`, as `save_recording` names them).
fn replay_prefix(track: &str) -> String {
    format!("{}-", track.chars().filter(char::is_ascii_alphanumeric).collect::<String>().to_ascii_lowercase())
}

/// Copy `src` into `dir` as `name`, or `stem (n).ext` when that is taken. `create_new` makes sure an existing file is
/// never overwritten, whatever happens between the check and the write. Returns the new path.
fn copy_unique(src: &Path, dir: &Path, name: &str) -> std::io::Result<PathBuf> {
    let bytes = read_capped(src, MAX_BYTES)?;
    std::fs::create_dir_all(dir)?;
    let (stem, ext) = match name.rsplit_once('.') {
        Some((s, e)) => (s, format!(".{e}")),
        None => (name, String::new()),
    };
    for n in 0..1000 {
        let to = dir.join(if n == 0 { name.to_string() } else { format!("{stem} ({n}){ext}") });
        match std::fs::OpenOptions::new().write(true).create_new(true).open(&to) {
            Ok(mut f) => {
                if let Err(e) = f.write_all(&bytes).and_then(|()| f.flush()) {
                    drop(f);
                    let _ = std::fs::remove_file(&to);
                    return Err(e);
                }
                return Ok(to);
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e),
        }
    }
    Err(std::io::Error::other("too many copies of it"))
}

pub(crate) fn file_name(path: &Path) -> String {
    path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
}

/// At most `cap` bytes of a file (`Err` when it is longer: the screen does not read it).
fn read_capped(path: &Path, cap: u64) -> std::io::Result<Vec<u8>> {
    let mut d = Vec::new();
    std::fs::File::open(path)?.take(cap + 1).read_to_end(&mut d)?;
    if d.len() as u64 > cap {
        return Err(std::io::Error::other("the file is too large"));
    }
    Ok(d)
}

/// The start of a file, up to `n` bytes.
fn read_head(path: &Path, n: u64) -> std::io::Result<Vec<u8>> {
    let mut d = Vec::new();
    std::fs::File::open(path)?.take(n).read_to_end(&mut d)?;
    Ok(d)
}

/// The files of a folder with this extension (any case), by name.
fn dir_files(dir: Option<PathBuf>, ext: &str) -> Vec<PathBuf> {
    let Some(rd) = dir.and_then(|d| std::fs::read_dir(d).ok()) else { return Vec::new() };
    let mut v: Vec<PathBuf> = rd.flatten().map(|e| e.path()).filter(|p| p.extension().and_then(|x| x.to_str()).is_some_and(|x| x.eq_ignore_ascii_case(ext))).collect();
    v.sort();
    v
}

/// A file's date (`yyyy-mm-dd`) and its time in seconds.
fn file_date(path: &Path) -> (String, u64) {
    let secs = std::fs::metadata(path).and_then(|m| m.modified()).ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map_or(0, |d| d.as_secs());
    (if secs > 0 { ymd(secs / 86_400) } else { String::new() }, secs)
}

/// The fastest flying lap of `laps`, a standing one only when there is none.
fn fastest<'a>(laps: impl Iterator<Item = &'a LapTrace> + Clone) -> Option<&'a LapTrace> {
    let by = |a: &&LapTrace, b: &&LapTrace| a.lap_ms.total_cmp(&b.lap_ms);
    laps.clone().filter(|l| !l.standing).min_by(by).or_else(|| laps.min_by(by))
}

impl Game {
    /// The screen's track profile (with a usable outline).
    pub(crate) fn ana_profile(&self) -> Option<&TrackProfile> {
        self.analysis.track.and_then(|t| self.tracks.get(t)).and_then(|t| t.profile.as_ref()).filter(|p| p.outline.len() >= 8)
    }

    /// The `tracks` index of a recording's track (by number for a race replay, else by name).
    fn src_track(&self, src: &Source) -> Option<usize> {
        match src {
            Source::Replay(r) => self.tracks.iter().position(|t| t.n == r.header.track),
            _ => self.tracks.iter().position(|t| t.name.eq_ignore_ascii_case(src.track_name())),
        }
    }

    fn car_label(&self, l: &LapTrace) -> String {
        if !l.car_name.is_empty() {
            return l.car_name.clone();
        }
        l.car_id.and_then(|id| self.cars.iter().find(|c| c.id == id)).map(|c| c.name.clone()).unwrap_or_default()
    }

    fn driver_label(&self, driver: &str) -> String {
        if driver.is_empty() { self.settings.player_name.clone() } else { driver.to_string() }
    }

    /// The card's note: the date (a demo's `NAME · date`), or `FROM <folder>` for a lap that came from outside the game's folders.
    fn side_note(&self, path: &Path, kind: Kind) -> String {
        if self.analysis.ext.contains(path) {
            let from = path.parent().map(folder_label).unwrap_or_default();
            return format!("FROM {}", tail(&from, 20).to_uppercase());
        }
        let (date, _) = file_date(path);
        if kind == Kind::Demo { format!("{} · {date}", path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default()) } else { date }
    }

    fn side_lap(&self, path: &Path, l: &LapTrace) -> SideLap {
        let note = self.side_note(path, l.kind);
        SideLap { path: path.to_path_buf(), trace: l.clone(), driver: self.driver_label(&l.driver), car: self.car_label(l), note }
    }

    /// Start the screen on track `ti`: both sides, the cursor, picker and requester cleared; the laps of the track listed.
    fn ana_begin(&mut self, ti: usize) {
        let a = &mut self.analysis;
        a.track = Some(ti);
        a.sides = [None, None];
        a.side = 0;
        a.cursor = None;
        a.picker = None;
        a.ask = None;
        a.cmp = None;
        a.job = None;
        a.fake_work = None;
        a.hover = false;
        a.offer = None;
        self.ana_list();
    }

    /// Recompare after a side changed (the cursor goes, as the mockup's pick does).
    fn ana_compare(&mut self) {
        self.analysis.cursor = None;
        let cmp = match (&self.analysis.sides, self.ana_profile()) {
            ([Some(a), Some(b)], Some(p)) => Some(lap_compare::compare(&a.trace, &b.trace, p)),
            _ => None,
        };
        self.analysis.cmp = cmp;
    }

    /// Put the lap `want` asks for of a read file on `side`.
    fn ana_set(&mut self, side: usize, path: &Path, f: &LapFile, want: &Want) {
        let cars = |id: u32| self.cars.iter().find(|c| c.id == id).map(|c| c.name.clone()).unwrap_or_default();
        let l = match want.slot {
            Some(s) => fastest(f.laps.iter().filter(|l| l.slot == s && want.lap.is_none_or(|n| l.lap == Some(n)))),
            None => f.pick(want.car.as_deref(), want.lap, &cars),
        };
        match l {
            Some(l) => {
                let lap = self.side_lap(path, l);
                let kind = l.kind;
                self.analysis.sides[side.min(1)] = Some(lap);
                self.ana_compare();
                self.ana_offer_keep(path, kind);
            }
            None => self.analysis.ask = Some(AnaAsk::NoLap { file: file_name(path), why: "no such car or lap in it".into() }),
        }
    }

    /// An outside file's lap is on a side: ask whether to keep a copy in the game's own folder (never without a settings folder).
    fn ana_offer_keep(&mut self, path: &Path, kind: Kind) {
        if self.analysis.offer.as_deref() != Some(path) {
            return;
        }
        self.analysis.offer = None;
        let Some((_, folder)) = self.keep_dir(kind) else { return };
        self.analysis.ask = Some(AnaAsk::Keep { file: file_name(path), from: path.parent().map(folder_label).unwrap_or_default(), folder, path: path.to_path_buf() });
        self.analysis.ask_btn = 0;
    }

    /// Where a copy of a recording of this kind goes, and its folder's name: Ghosts / Replays / Demos beside settings.cfg.
    fn keep_dir(&self, kind: Kind) -> Option<(PathBuf, &'static str)> {
        match kind {
            Kind::Ghost => hiscore::ghost_dir(self.persist).map(|d| (d, "Ghosts")),
            Kind::Replay => hiscore::replay_dir(self.persist).map(|d| (d, "Replays")),
            Kind::Demo => hiscore::demo_dir(self.persist).map(|d| (d, "Demos")),
        }
    }

    /// Whether a file lies outside the folders the game lists laps from (Ghosts, Replays, Demos, the install's Demos).
    fn ana_outside(&self, path: &Path) -> bool {
        let canon = |p: &Path| p.canonicalize().unwrap_or_else(|_| p.to_path_buf());
        let Some(parent) = path.parent().map(canon) else { return true };
        let known = [hiscore::ghost_dir(self.persist), hiscore::replay_dir(self.persist), hiscore::demo_dir(self.persist), self.demo_preview.clone(), Some(self.root.join("Demos"))];
        !known.iter().flatten().any(|d| canon(d) == parent)
    }

    /// KEEP A COPY: copy the outside file into its folder (never over a file), then the copy stands in for the original in the
    /// session's files and on the cards. A failure opens its requester.
    pub(crate) fn ana_keep(&mut self, path: &Path) {
        self.analysis.answered.insert(path.to_path_buf());
        let Some(f) = self.analysis.files.get(path).cloned() else { return };
        let Some((dir, folder)) = self.keep_dir(f.kind) else { return };
        let mut name = file_name(path);
        if f.kind == Kind::Replay && !name.to_ascii_lowercase().starts_with(&replay_prefix(&f.track_name)) {
            name = format!("{}{name}", replay_prefix(&f.track_name));
        }
        match copy_unique(path, &dir, &name) {
            Ok(to) => {
                let a = &mut self.analysis;
                if let Some(f) = a.files.remove(path) {
                    a.files.insert(to.clone(), f);
                }
                a.ext.remove(path);
                for s in a.sides.iter_mut().flatten().filter(|s| s.path == path) {
                    s.path = to.clone();
                }
                for i in 0..2 {
                    if let Some(kind) = self.analysis.sides[i].as_ref().filter(|s| s.path == to).map(|s| s.trace.kind) {
                        let note = self.side_note(&to, kind);
                        if let Some(s) = self.analysis.sides[i].as_mut() {
                            s.note = note;
                        }
                    }
                }
                self.ana_list();
            }
            Err(e) => self.analysis.ask = Some(AnaAsk::NotKept { file: file_name(path), folder, why: e.to_string() }),
        }
    }

    /// NOT NOW: not asked again for this file this session.
    pub(crate) fn ana_decline(&mut self, path: &Path) {
        self.analysis.answered.insert(path.to_path_buf());
    }

    /// BROWSE FILES was chosen on the picker: `App` opens the file dialog for the selected side.
    pub(crate) fn ana_browse(&mut self) {
        if !self.analysis.dialog {
            self.analysis.browse = Some(self.analysis.side.min(1));
        }
    }

    /// `App` takes the request for the file dialog (and the screen waits for it).
    pub fn ana_take_browse(&mut self) -> bool {
        let take = self.screen == Screen::Analysis && self.analysis.browse.take().is_some();
        self.analysis.dialog |= take;
        take
    }

    /// The file dialog is closed: the chosen file goes onto the selected side (`None`: cancelled, the picker stays).
    pub fn ana_dialog_done(&mut self, path: Option<PathBuf>) {
        self.analysis.dialog = false;
        self.analysis.browse = None;
        if let (Some(p), Screen::Analysis) = (path, self.screen) {
            self.ana_load_outside(&p);
        }
    }

    /// A file is dragged over the window: the drop overlay (only on the screen, and not over a requester).
    pub fn ana_hover(&mut self, on: bool) {
        self.analysis.hover = on && self.screen == Screen::Analysis && self.analysis.ask.is_none() && !self.analysis.dialog && !self.analysis.wayland;
    }

    /// A file is dropped on the window: the first one of a drop loads on the selected side, an open picker closes. Further files
    /// of the same drop (winit sends one event each) are ignored.
    pub fn ana_dropped(&mut self, path: PathBuf) {
        let again = self.analysis.dropped.is_some_and(|t| t.elapsed().as_secs_f32() < 0.5);
        self.analysis.hover = false;
        if self.screen != Screen::Analysis || self.analysis.ask.is_some() || self.analysis.dialog || again {
            return;
        }
        self.analysis.dropped = Some(std::time::Instant::now());
        self.ana_load_outside(&path);
    }

    /// Wayland session: the picker hides its drop hint, the overlay never shows (`App`, once the event loop is known).
    pub fn set_wayland(&mut self, on: bool) {
        self.analysis.wayland = on;
    }

    /// Load a file the player chose (dialog or drop) onto the selected side. One from outside the game's folders says
    /// `FROM <folder>` on its card and, once loaded, asks to be kept.
    fn ana_load_outside(&mut self, path: &Path) {
        self.analysis.picker = None;
        self.analysis.hover = false;
        let outside = self.ana_outside(path);
        if outside {
            self.analysis.ext.insert(path.to_path_buf());
        }
        self.analysis.offer = (outside && !self.analysis.answered.contains(path)).then(|| path.to_path_buf());
        let side = self.analysis.side;
        self.ana_load(side, path, Want::default());
        if self.analysis.ask.is_some() {
            self.sfx.push(Sfx::Menu(ev::REQUESTER));
        }
    }

    /// Read and check a file for the screen: its magic and that it is a lap of the screen's track.
    fn ana_source(&self, path: &Path, side: usize) -> Result<Source, AnaAsk> {
        let file = file_name(path);
        let bytes = read_capped(path, MAX_BYTES).map_err(|e| AnaAsk::NoLap { file: file.clone(), why: format!("it cannot be read ({e})") })?;
        let src = lap_compare::open(&bytes).map_err(|r| refusal_ask(path, &r))?;
        let ti = self.analysis.track.unwrap_or(0);
        if self.src_track(&src) != Some(ti) {
            let its = self.src_track(&src).map_or_else(|| src.track_name().to_string(), |t| self.tracks[t].name.clone());
            let ours = self.tracks.get(ti).map(|t| t.name.clone()).unwrap_or_default();
            return Err(AnaAsk::Track { file, its, ours, other: 1 - side.min(1) });
        }
        Ok(src)
    }

    /// Work a checked file out on this thread and keep it (a refusal is kept too).
    fn ana_read(&mut self, path: &Path, src: &Source) -> Result<Arc<LapFile>, AnaAsk> {
        let r = match self.ana_profile() {
            Some(p) => lap_compare::read(src, p, &self.root, &mut |_| true),
            None => Err(Refusal::Failed("the track is not installed".into())),
        };
        match r {
            Ok(f) => {
                let f = Arc::new(f);
                self.analysis.files.insert(path.to_path_buf(), f.clone());
                Ok(f)
            }
            Err(e) => {
                let a = refusal_ask(path, &e);
                self.analysis.refused.insert(path.to_path_buf(), e);
                Err(a)
            }
        }
    }

    /// Load a lap onto `side`: from the cache, at once (ghosts, demos), or by re-simulating a race replay on a worker
    /// thread (`Job`, polled by `ana_poll`). A refusal opens its requester.
    pub(crate) fn ana_load(&mut self, side: usize, path: &Path, want: Want) {
        if let Some(f) = self.analysis.files.get(path).cloned() {
            return self.ana_set(side, path, &f, &want);
        }
        if let Some(r) = self.analysis.refused.get(path) {
            self.analysis.ask = Some(refusal_ask(path, r));
            return;
        }
        let src = match self.ana_source(path, side) {
            Ok(s) => s,
            Err(a) => {
                self.analysis.ask = Some(a);
                return;
            }
        };
        if src.kind() == Kind::Replay && !self.analysis.sync {
            return self.ana_spawn(side, path, src, want);
        }
        match self.ana_read(path, &src) {
            Ok(f) => {
                self.ana_set(side, path, &f, &want);
                self.ana_list();
            }
            Err(a) => {
                self.analysis.ask = Some(a);
                self.ana_list();
            }
        }
    }

    /// Re-simulate a race replay on a worker thread (the pattern of the garage prefetch); a running one is cancelled.
    fn ana_spawn(&mut self, side: usize, path: &Path, src: Source, want: Want) {
        let Some(prof) = self.ana_profile().cloned() else { return };
        let root = self.root.clone();
        let (tx, rx) = mpsc::channel();
        let progress = Arc::new(AtomicU32::new(0));
        let cancel = Arc::new(AtomicBool::new(false));
        let (p, c) = (progress.clone(), cancel.clone());
        std::thread::spawn(move || {
            let r = lap_compare::read(&src, &prof, &root, &mut |f| {
                p.store(f.to_bits(), Ordering::Relaxed);
                !c.load(Ordering::Relaxed)
            });
            let _ = tx.send(r);
        });
        self.analysis.job = Some(Job { side: side.min(1), path: path.to_path_buf(), want, rx, progress, cancel });
    }

    /// Each frame on the screen: a finished re-simulation goes onto its side (or opens its requester).
    pub(crate) fn ana_poll(&mut self) {
        let Some(job) = &self.analysis.job else { return };
        let r = match job.rx.try_recv() {
            Ok(r) => r,
            Err(mpsc::TryRecvError::Empty) => return,
            Err(mpsc::TryRecvError::Disconnected) => Err(Refusal::Failed("the re-simulation stopped".into())),
        };
        let Some(job) = self.analysis.job.take() else { return };
        match r {
            Ok(f) => {
                let f = Arc::new(f);
                self.analysis.files.insert(job.path.clone(), f.clone());
                self.ana_set(job.side, &job.path, &f, &job.want);
                self.sfx.push(Sfx::Menu(if self.analysis.ask.is_some() { ev::REQUESTER } else { ev::SELECT }));
            }
            Err(Refusal::Cancelled) => {}
            Err(e) => {
                self.analysis.ask = Some(refusal_ask(&job.path, &e));
                self.analysis.refused.insert(job.path.clone(), e);
                self.sfx.push(Sfx::Menu(ev::REQUESTER));
            }
        }
        self.ana_list();
    }

    /// The picker rows of the screen's track: the Time Attack ghosts and the Ghost screen's demos (each car's best lap,
    /// by lap time), then the race replays (newest first; one row per car once read, else your car from the header).
    fn ana_list(&mut self) {
        let Some(ti) = self.analysis.track else { return };
        let (tname, tn) = (self.tracks[ti].name.clone(), self.tracks[ti].n);
        // the ghosts: read when first seen (small files); another track's are only opened
        for path in dir_files(hiscore::ghost_dir(self.persist), "ghl") {
            if self.analysis.files.contains_key(&path) || self.analysis.refused.contains_key(&path) {
                continue;
            }
            let Ok(src) = read_capped(&path, MAX_BYTES).map_err(|_| ()).and_then(|d| lap_compare::open(&d).map_err(|_| ())) else { continue };
            if self.src_track(&src) == Some(ti) {
                let _ = self.ana_read(&path, &src);
            }
        }
        // the demos of the Ghost screen's list
        let demos: Vec<(PathBuf, Mde)> = self.replays.iter().filter(|(_, m)| m.header.track.eq_ignore_ascii_case(&tname)).cloned().collect();
        for (path, m) in demos {
            if !self.analysis.files.contains_key(&path) && !self.analysis.refused.contains_key(&path) {
                let _ = self.ana_read(&path, &Source::Demo(Box::new(m)));
            }
        }
        let mut rows: Vec<Cand> = Vec::new();
        let mut files: Vec<(PathBuf, Arc<LapFile>)> = self.analysis.files.iter().filter(|(_, f)| f.track_name.eq_ignore_ascii_case(&tname)).map(|(p, f)| (p.clone(), f.clone())).collect();
        files.sort_by(|a, b| a.0.cmp(&b.0));
        for (path, f) in &files {
            let (date, order) = file_date(path);
            let stem = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
            let mut slots: Vec<usize> = f.laps.iter().map(|l| l.slot).collect();
            slots.dedup();
            for s in slots {
                let Some(l) = f.best(Some(s)) else { continue };
                rows.push(Cand {
                    path: path.clone(),
                    kind: f.kind,
                    slot: Some(s),
                    lap: l.lap,
                    driver: self.driver_label(&l.driver),
                    car: self.car_label(l),
                    lap_ms: Some(l.lap_ms),
                    date: date.clone(),
                    stem: stem.clone(),
                    outdated: false,
                    order,
                });
            }
        }
        // race replays not read yet: the header only (`Replays/<Track>-*.mhr`, as `save_recording` names them)
        let prefix = replay_prefix(&tname);
        for path in dir_files(hiscore::replay_dir(self.persist), "mhr") {
            if self.analysis.files.contains_key(&path) || !file_name(&path).to_ascii_lowercase().starts_with(&prefix) {
                continue;
            }
            let Some(h) = read_head(&path, PEEK_BYTES).ok().and_then(|d| motor_sim::replay::Header::peek(&d).ok()) else { continue };
            if h.track != tn {
                continue;
            }
            let (date, order) = file_date(&path);
            rows.push(Cand {
                path: path.clone(),
                kind: Kind::Replay,
                slot: None,
                lap: None,
                driver: self.driver_label(&h.nick),
                car: h.car_name.clone(),
                lap_ms: h.best_lap_ms.map(|v| v as f32),
                date,
                stem: path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default(),
                outdated: self.analysis.refused.contains_key(&path),
                order,
            });
        }
        let group = |c: &Cand| match c.kind {
            Kind::Ghost => 0,
            Kind::Demo => 1,
            Kind::Replay => 2,
        };
        rows.sort_by(|a, b| {
            group(a).cmp(&group(b)).then_with(|| {
                if a.kind == Kind::Replay {
                    b.order.cmp(&a.order).then_with(|| a.path.cmp(&b.path)).then_with(|| a.slot.cmp(&b.slot))
                } else {
                    a.lap_ms.unwrap_or(f32::MAX).total_cmp(&b.lap_ms.unwrap_or(f32::MAX))
                }
            })
        });
        self.analysis.list = rows;
    }

    /// Whether ANALYSE can open the Ghost screen's recording `idx`: its track is installed and it holds a complete lap.
    pub(crate) fn ana_possible(&self, idx: usize) -> bool {
        let Some((_, m)) = self.replays.get(idx) else { return false };
        let installed = self.tracks.iter().any(|t| t.name.eq_ignore_ascii_case(&m.header.track) && t.profile.as_ref().is_some_and(|p| p.outline.len() >= 8));
        installed && self.replay_info.get(idx).and_then(|i| i.stats.as_ref()).is_some_and(|s| s.best.is_some())
    }

    /// Lap Analysis of the Ghost screen's recording `idx` (ANALYSE). Side A: its best lap of your car (anyone's in a
    /// recording without you). Side B: your Time Attack ghost of that car on the track, else your fastest ghost there,
    /// else the fastest other car of the recording, else your car's next best lap. False when nothing can be compared.
    pub(crate) fn ana_from_ghost(&mut self, idx: usize) -> bool {
        if !self.ana_possible(idx) {
            return false;
        }
        let Some((path, m)) = self.replays.get(idx).cloned() else { return false };
        let Some(ti) = self.tracks.iter().position(|t| t.name.eq_ignore_ascii_case(&m.header.track)) else { return false };
        self.ana_begin(ti);
        let Some(f) = self.analysis.files.get(&path).cloned() else { return false };
        let Some(a) = f.best(f.me).or_else(|| f.best(None)).cloned() else { return false };
        self.analysis.sides[0] = Some(self.side_lap(&path, &a));
        let car = a.car_id.or_else(|| self.cars.iter().find(|c| c.name.eq_ignore_ascii_case(&a.car_name)).map(|c| c.id));
        let own = car.and_then(|id| hiscore::ghost_path(self.persist, &self.tracks[ti].name, id)).filter(|p| self.analysis.files.contains_key(p));
        let ghost = own.or_else(|| self.analysis.list.iter().find(|c| c.kind == Kind::Ghost).map(|c| c.path.clone()));
        if let Some(g) = ghost {
            self.ana_load(1, &g, Want::default());
        } else {
            let other = fastest(f.laps.iter().filter(|l| l.slot != a.slot)).or_else(|| fastest(f.laps.iter().filter(|l| l.slot == a.slot && l.lap != a.lap))).unwrap_or(&a).clone();
            self.analysis.sides[1] = Some(self.side_lap(&path, &other));
            self.ana_compare();
        }
        self.ghost_last = Some(path.file_stem().map(|s| s.to_string_lossy().to_ascii_lowercase()).unwrap_or_default());
        true
    }

    /// Headless `--shot analysis`: the pair of `--compare` (any paths; the screen's track is A's), else the first recording
    /// of the Ghost screen's list. Race replays re-simulate on the spot. `overlay`: `working` shows the working overlay (42 %),
    /// `drop` the drop overlay, `keep` side B as a file from outside the game's folders with its KEEP THIS LAP? requester.
    pub fn preview_analysis(&mut self, compare: Option<[(PathBuf, Want); 2]>, overlay: Option<&str>) -> Result<(), String> {
        self.analysis.sync = true;
        if self.replays.is_empty() {
            self.load_replays();
        }
        match compare {
            None => {
                if !self.ana_from_ghost(0) {
                    return Err("--shot analysis: the first recording of the Ghost screen has no lap to compare (give --compare A,B)".into());
                }
            }
            Some([(pa, wa), (pb, wb)]) => {
                let d = read_capped(&pa, MAX_BYTES).map_err(|e| format!("{}: {e}", pa.display()))?;
                let src = lap_compare::open(&d).map_err(|e| format!("{}: {e}", pa.display()))?;
                let ti = self.src_track(&src).ok_or_else(|| format!("{}: the track {} is not installed", pa.display(), src.track_name()))?;
                self.ana_begin(ti);
                self.ana_load(0, &pa, wa);
                // side B's refusal is the one a shot shows (A must load)
                self.analysis.side = 1;
                self.ana_load(1, &pb, wb);
                self.analysis.side = 0;
            }
        }
        match overlay {
            Some("working") => self.analysis.fake_work = Some(0.42),
            Some("drop") => self.analysis.hover = true,
            Some("keep") => {
                let b = self.analysis.sides[1].as_ref().ok_or("--analysis-overlay keep needs a lap on side B")?;
                let (path, kind) = (b.path.clone(), b.trace.kind);
                self.analysis.ext.insert(path.clone());
                let note = self.side_note(&path, kind);
                if let Some(b) = self.analysis.sides[1].as_mut() {
                    b.note = note;
                }
                let folder = match kind {
                    Kind::Ghost => "Ghosts",
                    Kind::Replay => "Replays",
                    Kind::Demo => "Demos",
                };
                self.analysis.ask = Some(AnaAsk::Keep { file: file_name(&path), from: path.parent().map(folder_label).unwrap_or_default(), folder, path });
            }
            _ => {}
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("motorhead-ana-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    /// KEEP A COPY never overwrites: a taken name gets ` (n)` before the extension, and the file already there is untouched.
    #[test]
    fn copy_never_overwrites() {
        let (from, to) = (temp("from"), temp("to"));
        let src = from.join("Goldbridge-car10.ghl");
        std::fs::write(&src, b"new").unwrap();
        std::fs::write(to.join("Goldbridge-car10.ghl"), b"mine").unwrap();
        let a = copy_unique(&src, &to, "Goldbridge-car10.ghl").unwrap();
        let b = copy_unique(&src, &to, "Goldbridge-car10.ghl").unwrap();
        assert_eq!(file_name(&a), "Goldbridge-car10 (1).ghl");
        assert_eq!(file_name(&b), "Goldbridge-car10 (2).ghl");
        assert_eq!(std::fs::read(to.join("Goldbridge-car10.ghl")).unwrap(), b"mine");
        assert_eq!(std::fs::read(&a).unwrap(), b"new");
        // the folder is made when it is missing, and a missing source is an error, not a panic
        let c = copy_unique(&src, &to.join("sub").join("deeper"), "x.mhr").unwrap();
        assert!(c.exists());
        assert!(copy_unique(&from.join("none.ghl"), &to, "none.ghl").is_err());
        let _ = (std::fs::remove_dir_all(from), std::fs::remove_dir_all(to));
    }

    #[test]
    fn card_folder_is_cut_at_a_folder() {
        assert_eq!(tail("~/Downloads", 20), "~/Downloads");
        assert_eq!(tail("/run/media/sin/stick/laps/friend", 20), "\u{2026}/stick/laps/friend");
        assert_eq!(tail("/a/averyveryverylongfoldernameindeed", 20).chars().count(), 20);
    }
}
