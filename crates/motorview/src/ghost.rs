//! DEBUG motorview's ghosts: load a Time Attack lap (`.ghl`) or a race recording (`.mhr`) and
//! draw its car in the viewer and in the ghost view window (design choice; the game's own
//! Time Attack ghost is in motorhead's front.rs). Re-notes/00-debuger.md.

use crate::drive::Session;
use glam::Mat4;
use motor_sim::replay::{GhostPose, GhostTrack, Recording};
use std::path::{Path, PathBuf};

/// motorhead's settings directory (`settings.cfg` and the `Ghosts`, `Replays` folders beside it).
pub fn config_dir() -> Option<PathBuf> {
    Some(if cfg!(windows) {
        PathBuf::from(std::env::var_os("APPDATA")?).join("Motorhead")
    } else if let Some(x) = std::env::var_os("XDG_CONFIG_HOME").filter(|x| !x.is_empty()) {
        PathBuf::from(x).join("motorhead")
    } else {
        PathBuf::from(std::env::var_os("HOME")?).join(".config").join("motorhead")
    })
}

/// A file the Ghost page lists.
#[derive(Debug, Clone)]
pub struct Entry {
    pub path: PathBuf,
    /// file name for the list
    pub name: String,
}

/// The `.ghl` files of `Ghosts` and the `.mhr` files of `Replays`, by name.
pub fn list() -> Vec<Entry> {
    let Some(dir) = config_dir() else { return Vec::new() };
    let mut out = Vec::new();
    for (sub, ext) in [("Ghosts", "ghl"), ("Replays", "mhr")] {
        let Ok(rd) = std::fs::read_dir(dir.join(sub)) else { continue };
        for e in rd.flatten() {
            let path = e.path();
            if path.extension().is_some_and(|x| x.eq_ignore_ascii_case(ext)) {
                let name = format!("{sub}/{}", e.file_name().to_string_lossy());
                out.push(Entry { path, name });
            }
        }
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

/// The game's `Replays` folder: motorview saves its runs (`.mhr`) here, as the game does.
pub fn replay_dir() -> Option<PathBuf> {
    Some(config_dir()?.join("Replays"))
}

/// The `.mhr` files of `Replays`, by name (the runs motorview can open).
pub fn runs() -> Vec<Entry> {
    list().into_iter().filter(|e| e.path.extension().is_some_and(|x| x.eq_ignore_ascii_case("mhr"))).collect()
}

/// `TrackName` of track `n` (`Game/TrackN.trk`).
pub fn track_name(root: &Path, n: u32) -> Option<String> {
    motor_data::fs::config(root, &format!("Game/Track{n}.trk")).ok().and_then(|c| c.get("TrackName").map(|e| e.values.join(" ")))
}

/// The track whose `TrackName` is `name` (letters and digits compared, as in a `.ghl` file name).
pub fn track_number(root: &Path, name: &str) -> Option<u32> {
    let key = |s: &str| s.chars().filter(char::is_ascii_alphanumeric).collect::<String>().to_ascii_lowercase();
    let want = key(name);
    (1..=8).find(|n| {
        motor_data::fs::config(root, &format!("Game/Track{n}.trk")).ok().and_then(|c| c.get("TrackName").map(|e| key(&e.values.join(" ")))).is_some_and(|k| k == want)
    })
}

/// What the ghost window follows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Follow {
    /// the loaded ghost
    Ghost,
    /// the car in this grid slot (`Session::race.racers`)
    Car(usize),
}

/// How the ghost's clock runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Sync {
    /// the lap clock of the followed car (as in Time Attack)
    Lap,
    /// its own clock, advanced by the sim's time, repeating
    Loop,
}

impl Sync {
    pub const ALL: [Sync; 2] = [Sync::Lap, Sync::Loop];

    pub fn name(self) -> &'static str {
        match self {
            Sync::Lap => "followed car's lap",
            Sync::Loop => "loop",
        }
    }
}

/// The ghost loaded in the viewer.
pub struct ViewerGhost {
    pub track: GhostTrack,
    /// the car it was driven in (file number) and the track it belongs to, when known
    pub car: u32,
    pub track_no: Option<u32>,
    /// file name, for the page
    pub name: String,
    pub sync: Sync,
    /// draw it in the main view too
    pub show_main: bool,
    /// the loop clock, ms
    pub clock_ms: f64,
}

impl ViewerGhost {
    /// Read a `.ghl` or `.mhr`. A recording is replayed to its first lap's end (seconds).
    pub fn load(root: &Path, path: &Path) -> Result<ViewerGhost, String> {
        let name = path.file_name().map_or_else(String::new, |n| n.to_string_lossy().into_owned());
        let ext = path.extension().map(|e| e.to_string_lossy().to_ascii_lowercase());
        let (track, car, track_no) = match ext.as_deref() {
            Some("mhr") => {
                let rec = Recording::load(path)?;
                let car = rec.header.player.and_then(|p| rec.header.cars.get(p)).copied().ok_or("the recording has no player")?;
                (rec.ghost(root)?, car, Some(rec.header.track))
            }
            _ => {
                let f = GhostTrack::parse(&std::fs::read(path).map_err(|e| e.to_string())?)?;
                (f.track, f.car, track_number(root, &f.track_name))
            }
        };
        if track.poses.is_empty() {
            return Err("the file holds no lap".into());
        }
        Ok(ViewerGhost { track, car, track_no, name, sync: Sync::Lap, show_main: true, clock_ms: 0.0 })
    }

    /// The racer whose meshes the ghost is drawn with (the same car).
    pub fn slot(&self, sess: &Session) -> Option<usize> {
        let r = &sess.race.racers;
        r.iter().position(|x| x.human && x.car_id == self.car).or_else(|| r.iter().position(|x| x.car_id == self.car))
    }

    /// Length of the loop, ms.
    fn span(&self) -> f64 {
        self.track.poses.last().map_or(0.0, |p| p.clock_ms) + 1000.0
    }

    /// Advance the loop clock by `dt` seconds of sim time (negative: rewind).
    pub fn advance(&mut self, dt: f32) {
        self.clock_ms = (self.clock_ms + dt as f64 * 1000.0).rem_euclid(self.span().max(1.0));
    }

    pub fn pose(&self, sess: &Session) -> Option<GhostPose> {
        match self.sync {
            Sync::Loop => self.track.pose_at(self.clock_ms),
            Sync::Lap => {
                let p = &sess.race.racers.get(sess.focus)?.progress;
                self.track.pose_at(sess.race.clock_ms - p.lap_start_ms as f64)
            }
        }
    }

    /// The ghost's meshes (empty without a pose or a car to draw with).
    pub fn models(&self, sess: &Session) -> Vec<(usize, Mat4)> {
        match (self.slot(sess), self.pose(sess)) {
            (Some(slot), Some(p)) => sess.ghost_models(slot, &p),
            _ => Vec::new(),
        }
    }
}
