//! DEBUG motorview's Cine editor (#152, re-notes/20-cine.md): a `.cine` sequence edited in the viewer. Opening one
//! makes its all-AI race the viewer's race (`cine_render::race`, the render's own start); while it is open the
//! sequence's clock drives the race (`frame`: a seek on the race timeline, else steps of 1/fps past its end), the
//! `Director` runs every camera, and the view can look through the selected camera or the one on air. Cameras and
//! path keys are added where the free camera stands; every field is edited on the Properties page (`page`).
//! The ribbon's Cine tab and the shortcuts push `Req`s the viewer runs after the frame (they need the free camera
//! or a race reload). Saving writes `cine::to_text` (comments of a hand-written file are not kept).
//! In the world (`gizmos`) every camera is a frustum with its name, a path camera its curve with a cross per key, a
//! tracking camera a line to its car; a click picks a camera, key or aim point (`press`) and the move arrows
//! (`fill_gizmo`, global axes) drag it. Render runs `cine_render::render` on a worker thread (`Job`).

use crate::cine::{self, path_points, on_air, race_secs, shot_at, since_shot, video_secs, Aim, CamKey, CamKind, CamMode, CineCam, Director, Drive, PathAim, RaceDef, Sequence, Shake, ShakeKind, Shot, Target, Zoom, DIFFICULTIES, FOV_RANGE, POSE_MODES, SHAKE_KINDS, SPEED_RANGE};
use crate::cine_render::{self, Codec, Out, RenderOpts, Watch, CANCELLED};
use crate::debug::{self, Gizmos};
use crate::drive::{cam_origin, Session};
use crate::fill_gizmo::{self, Mode as GizmoMode, AXES};
use crate::gui::wheel_drag;
use crate::icons as ic;
use crate::inspect::Passes;
use crate::render::{Camera, View, FOV_X};
use crate::tune::section;
use crate::world_edit::PICK_PX;
use egui::{ComboBox, Ui};
use glam::{Mat3, Vec3};
use motor_sim::Race;
use motor_sim::tvcam::rot_to_euler;
use motor_sim::Input;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::JoinHandle;
use std::time::Instant;

const FLIP: Vec3 = Vec3::new(1.0, 1.0, -1.0);
const DEG: f32 = std::f32::consts::PI / 180.0;
/// Race seconds the editor steps past the recorded run in one viewer frame (a scrub ahead takes a few frames).
const RUN_AHEAD: f32 = 1.0;
/// A clock move further than this (video seconds, or backwards) is a jump: the director starts afresh.
const JUMP: f32 = 0.25;
/// The time slider reaches this far past the recorded run when the sequence has no `End`.
const SLIDER_AHEAD: f32 = 60.0;
/// Defaults of a new camera.
const NEW_LAG: f32 = 0.3;
const NEW_LEAD: f32 = 0.2;
const NEW_AUTO: f32 = 25.0;
const MOUNT_OFFSET: Vec3 = Vec3::new(0.0, 1.6, -0.5);
/// The status bar's hint while the editor has the mouse.
pub const HINT: &str = "LMB: pick a camera, key or aim point · drag an arrow to move it · RMB drag: look · Ctrl+K camera here · Ctrl+Shift+K key here · Ctrl+P preview";
/// Gizmos: points sampled along a path, the frustum depth and a key cross as shares of the eye distance (constant
/// size on screen) with their floors, metres.
const PATH_POINTS: usize = 96;
const FRUSTUM_K: f32 = 0.08;
const FRUSTUM_MIN: f32 = 0.8;
const CROSS_K: f32 = 0.012;
const CROSS_MIN: f32 = 0.15;
/// No move gizmo nearer the eye than this (previewing the camera it sits on), metres.
const GIZMO_NEAR: f32 = 1.0;

/// What the view looks through.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Preview {
    /// the viewer's own camera
    #[default]
    Off,
    /// the camera selected in the editor
    Selected,
    /// the camera on air at the clock (the video)
    OnAir,
}

impl Preview {
    pub const ALL: [Preview; 3] = [Preview::Off, Preview::Selected, Preview::OnAir];

    pub fn name(self) -> &'static str {
        match self {
            Preview::Off => "off",
            Preview::Selected => "selected",
            Preview::OnAir => "on air",
        }
    }

    pub fn parse(s: &str) -> Option<Preview> {
        match s {
            "off" => Some(Preview::Off),
            "selected" | "sel" => Some(Preview::Selected),
            "air" | "onair" | "on-air" => Some(Preview::OnAir),
            _ => None,
        }
    }

    /// Off -> On air -> Selected -> Off (Ctrl+P).
    pub fn next(self) -> Preview {
        match self {
            Preview::Off => Preview::OnAir,
            Preview::OnAir => Preview::Selected,
            Preview::Selected => Preview::Off,
        }
    }
}

/// Work the viewer does after the frame: it needs the free camera, the session or a race reload.
#[derive(Clone, Debug, PartialEq)]
pub enum Req {
    /// a new sequence on the viewer's track, one camera where the free camera is
    New,
    Open(PathBuf),
    /// reload the race from the sequence's `Race` (track, cars, difficulty, laps)
    ApplyRace,
    /// a static camera at the free camera, tracking the editor's target
    AddStatic,
    /// a camera mounted on the followed car where the free camera is
    AddMounted,
    /// a path camera with its first key at the free camera
    AddPath,
    /// a key of the selected path camera at the free camera (at the clock)
    AddKey,
    /// the selected camera's eye (static: and aim, when it has a fixed one) to the free camera
    EyeFromCamera,
    /// key `k` of the selected camera to the free camera
    KeyFromCamera(usize),
    /// a shot of the selected camera at the clock
    Cut,
    /// render the sequence to a video next to its file (the viewer's graphics options)
    Render,
}

/// The point of the selected camera the move gizmo sits on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Handle {
    /// a static camera's eye, a mounted camera's place on its car (its offset)
    Eye,
    /// a static camera's fixed aim point (`At`)
    Aim,
    /// key `k` (its Eye)
    Key(usize),
}

/// A move-gizmo drag: the handle, the axis, where the handle was and the axis parameter of the press.
#[derive(Clone, Copy, Debug)]
struct Drag {
    handle: Handle,
    axis: usize,
    base: Vec3,
    from: f32,
}

/// A render on a worker thread (`cine_render::render`, its own GPU device).
pub struct Job {
    pub out: PathBuf,
    pub watch: Watch,
    thread: Option<JoinHandle<Result<(), String>>>,
    started: Instant,
}

pub struct CineEdit {
    pub seq: Option<Sequence>,
    /// the file it came from / goes to
    pub path: Option<PathBuf>,
    pub dirty: bool,
    /// the selected camera
    pub sel: usize,
    pub preview: Preview,
    /// the clock, video seconds
    pub t: f32,
    /// the race the viewer has loaded for this sequence (a changed `Race` waits for Apply)
    pub loaded: Option<RaceDef>,
    /// the target of new cameras
    pub target: Target,
    /// last result (open, save, warnings)
    pub msg: String,
    /// the page's path field and the `.cine` files of the install (None = list again)
    pub path_text: String,
    pub files: Option<Vec<PathBuf>>,
    /// the page's `Cars` field
    pub cars_text: String,
    pub reqs: Vec<Req>,
    /// the session's camera mode, followed slot and the viewer's camera before a preview took them
    pub saved: Option<(CamMode, usize, Camera)>,
    dir: Option<Director>,
    /// the clock of the director's last update
    dir_t: f32,
    /// the sequence changed or the clock jumped: a fresh director next frame
    reset: bool,
    /// the race is not at the clock yet (running ahead)
    pub behind: bool,
    /// the gizmo's point, with its camera (another camera, or a point the camera lacks: the selected camera's first, `handle`)
    pub handle: Option<(usize, Handle)>,
    /// the gizmo axis under the mouse or held
    hot: Option<usize>,
    drag: Option<Drag>,
    pub job: Option<Job>,
}

impl Default for CineEdit {
    fn default() -> Self {
        CineEdit {
            seq: None,
            path: None,
            dirty: false,
            sel: 0,
            preview: Preview::Off,
            t: 0.0,
            loaded: None,
            target: Target::Leader,
            msg: String::new(),
            path_text: String::new(),
            files: None,
            cars_text: String::new(),
            reqs: Vec::new(),
            saved: None,
            dir: None,
            dir_t: 0.0,
            reset: true,
            behind: false,
            handle: None,
            hot: None,
            drag: None,
            job: None,
        }
    }
}

/// The install's folder of sequences.
pub fn dir(root: &Path) -> PathBuf {
    root.join("remaster/cine")
}

/// The `.cine` files of the install, by name.
pub fn list(root: &Path) -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = std::fs::read_dir(dir(root)).map(|d| d.filter_map(|e| e.ok().map(|e| e.path())).filter(|p| p.extension().is_some_and(|x| x.eq_ignore_ascii_case("cine"))).collect()).unwrap_or_default();
    v.sort();
    v
}

/// A bare name is looked for in the install's folder (as `--cine`).
pub fn resolve(root: &Path, p: &Path) -> PathBuf {
    if p.exists() || p.components().count() > 1 {
        p.to_path_buf()
    } else {
        dir(root).join(p)
    }
}

/// Read and parse a `.cine`.
pub fn open(path: &Path) -> Result<(Sequence, Vec<String>), String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    cine::parse(&text).map_err(|e| format!("{}: {e}", path.display()))
}

/// A free name `trackNN[_k].cine` in the install's folder.
pub fn new_path(root: &Path, track: u32) -> PathBuf {
    let d = dir(root);
    (1..)
        .map(|k| d.join(if k == 1 { format!("track{track:02}.cine") } else { format!("track{track:02}_{k}.cine") }))
        .find(|p| !p.exists())
        .unwrap_or_else(|| d.join("new.cine"))
}

/// A new sequence on `track` (the default all-AI race), its one camera where `cam` is, following the leader.
pub fn new_seq(track: u32, cam: &Camera) -> Sequence {
    let mut seq = Sequence { race: RaceDef { track, ..RaceDef::default() }, audio: Default::default(), size: (1920, 1080), fps: 60, end: None, cameras: Vec::new(), shots: vec![Shot::new(0.0, 0)] };
    let c = static_here(&seq, cam, Target::Leader);
    seq.cameras.push(c);
    seq
}

/// Game-space orientation (columns right, up, forward) of a view camera: the inverse of `Camera::from_rot`.
pub fn game_rot(cam: &Camera) -> Mat3 {
    let (f, u) = (cam.forward() * FLIP, cam.up() * FLIP);
    let r = u.cross(f).normalize_or(Vec3::X);
    Mat3::from_cols(r, f.cross(r), f)
}

/// Horizontal field of view (degrees) of a zoom: the inverse of `cine::fov_zoom`.
pub fn fov_of(zoom: f32) -> f32 {
    (2.0 * ((FOV_X * 0.5 * DEG).tan() / zoom.max(1e-3)).atan() / DEG).clamp(FOV_RANGE.0, FOV_RANGE.1)
}

/// Pitch heading bank (degrees, `Angles`) of a game-space orientation.
fn angles_of(rot: Mat3) -> Vec3 {
    let a = rot_to_euler(rot) / DEG;
    // tidy -0.0 and float dust for the file
    a.map(|x| (x * 1000.0).round() / 1000.0)
}

fn round3(v: Vec3) -> Vec3 {
    v.map(|x| (x * 1000.0).round() / 1000.0)
}

/// The kinds offered in the camera combo: (name, Pose mode for the game's views).
const KINDS: [(&str, Option<CamMode>); 8] =
    [("static", None), ("mounted", None), ("path", None), ("chase", Some(CamMode::Chase)), ("far", Some(CamMode::FarChase)), ("incar", Some(CamMode::InCar)), ("bumper", Some(CamMode::Bumper)), ("tv", Some(CamMode::Tv))];

fn kind_name(k: &CamKind) -> &'static str {
    match k {
        CamKind::Static { .. } => "static",
        CamKind::Mounted { .. } => "mounted",
        CamKind::Path { .. } => "path",
        CamKind::Pose { mode, .. } => POSE_MODES.iter().find(|m| m.1 == *mode).map_or("chase", |m| m.0),
    }
}

/// The car a camera follows or rides on, if any.
fn target_of(k: &CamKind) -> Option<Target> {
    match k {
        CamKind::Static { aim: Aim::Track { target, .. }, .. } | CamKind::Path { aim: PathAim::Track { target, .. }, .. } => Some(*target),
        CamKind::Mounted { target, .. } | CamKind::Pose { target, .. } => Some(*target),
        CamKind::Path { drive: Drive::Car(t), .. } => Some(*t),
        _ => None,
    }
}

/// `nearest` is for static cameras only (cine.rs parse): elsewhere it becomes the leader.
fn no_nearest(t: Target) -> Target {
    if t == Target::Nearest {
        Target::Leader
    } else {
        t
    }
}

fn target_name(t: Target, names: &[String]) -> String {
    match t {
        Target::Car(n) => names.get(n).map_or(format!("car {n}"), |name| format!("car {n}: {name}")),
        Target::Leader => "leader".into(),
        Target::Nearest => "nearest".into(),
        Target::Player => "player (last slot)".into(),
    }
}

/// The drivers' names by grid slot (the target combos).
pub fn names(sess: Option<&Session>) -> Vec<String> {
    sess.map_or_else(Vec::new, |s| s.race.racers.iter().map(|r| r.driver.name.clone()).collect())
}

/// A target combo; `nearest` offered for static cameras only; `names` = the drivers by slot. True when changed.
pub fn target_combo(ui: &mut Ui, id: &str, t: &mut Target, nearest: bool, cars: usize, names: &[String]) -> bool {
    let mut changed = false;
    ComboBox::from_id_salt(id).selected_text(target_name(*t, names)).show_ui(ui, |ui| {
        let mut opts = vec![Target::Leader];
        if nearest {
            opts.push(Target::Nearest);
        }
        opts.push(Target::Player);
        opts.extend((0..cars).map(Target::Car));
        for o in opts {
            if ui.selectable_label(*t == o, target_name(o, names)).clicked() && *t != o {
                *t = o;
                changed = true;
            }
        }
    });
    changed
}

/// A camera name without spaces (the file splits on them) that no other camera has.
fn unique_name(seq: &Sequence, want: &str, skip: Option<usize>) -> String {
    let base: String = want.split_whitespace().collect::<Vec<_>>().join("_");
    let base = if base.is_empty() { "cam".to_string() } else { base };
    let taken = |n: &str| seq.cameras.iter().enumerate().any(|(i, c)| Some(i) != skip && c.name == n);
    if !taken(&base) {
        return base;
    }
    (2..).map(|k| format!("{base}{k}")).find(|n| !taken(n)).unwrap_or(base)
}

fn next_name(seq: &Sequence, stem: &str) -> String {
    let n = seq.cameras.len() + 1;
    unique_name(seq, &format!("{stem}{n}"), None)
}

/// A static camera at the free camera tracking `target`.
fn static_here(seq: &Sequence, cam: &Camera, target: Target) -> CineCam {
    let kind = CamKind::Static { eye: round3(cam.pos * FLIP), aim: Aim::Track { target, lag: NEW_LAG, lead: NEW_LEAD }, zoom: Zoom::Auto(NEW_AUTO), clear: true };
    CineCam { name: next_name(seq, "cam"), kind, keys: Vec::new(), shakes: Vec::new() }
}

/// Keep the sequence usable by the director: a camera, shots sorted with valid cameras (at least one), keys sorted,
/// every path camera with an Eye key (`eye`: where a missing one goes).
fn tidy(seq: &mut Sequence, eye: Vec3) {
    let n = seq.cameras.len();
    seq.shots.retain(|s| s.cam < n);
    seq.shots.sort_by(|a, b| a.t.total_cmp(&b.t));
    if seq.shots.is_empty() {
        seq.shots.push(Shot::new(0.0, 0));
    }
    for c in &mut seq.cameras {
        c.keys.sort_by(|a, b| a.at.total_cmp(&b.at));
        if matches!(c.kind, CamKind::Path { .. }) && !c.keys.iter().any(|k| k.eye.is_some()) {
            c.keys.insert(0, CamKey { at: 0.0, eye: Some(eye), angles: None, fov: None, tcb: [0.0; 3], linear: false });
        }
    }
}

/// Bring the race to race second `to` of its timeline: a seek inside the recorded run, else steps of `step` past its
/// end, at most `budget` seconds of them. True when it got there.
pub fn sync(sess: &mut Session, to: f32, step: f32, budget: f32) -> bool {
    let Some((start, now, end)) = sess.timeline.as_ref().map(|t| t.span()) else { return true };
    let to = (to as f64).max(start);
    let half = step as f64 * 0.5;
    if (to - now).abs() < half {
        return true;
    }
    if to <= end + half {
        sess.seek(to);
        return true;
    }
    if now < end {
        sess.seek(end);
    }
    let mut left = budget;
    loop {
        let now = sess.timeline.as_ref().map_or(to, |t| t.span().1);
        if now + half >= to {
            return true;
        }
        if left <= 0.0 {
            return false;
        }
        sess.step(step, Input::default());
        left -= step;
    }
}

impl CineEdit {
    /// A sequence is open.
    pub fn active(&self) -> bool {
        self.seq.is_some()
    }

    /// Put a sequence in the editor (its race still has to be loaded: `loaded`).
    pub fn set(&mut self, seq: Sequence, path: Option<PathBuf>) {
        self.cars_text = seq.race.cars.iter().map(u32::to_string).collect::<Vec<_>>().join(" ");
        self.path_text = path.as_ref().map_or_else(String::new, |p| p.display().to_string());
        (self.seq, self.path, self.dirty, self.sel, self.t, self.reset, self.loaded) = (Some(seq), path, false, 0, 0.0, true, None);
        (self.handle, self.hot, self.drag) = (None, None, None);
    }

    /// The race was (re)loaded: the clock back to 0, the cameras from rest.
    pub fn restart(&mut self, race: RaceDef) {
        (self.loaded, self.t, self.dir_t, self.reset, self.behind, self.saved) = (Some(race), 0.0, 0.0, true, false, None);
    }

    /// Leave the editor (a render keeps running).
    pub fn close(&mut self) {
        (self.seq, self.path, self.dirty, self.dir, self.loaded, self.preview) = (None, None, false, None, None, Preview::Off);
        (self.handle, self.hot, self.drag) = (None, None, None);
    }

    /// The sequence was edited: unsaved, and the director starts afresh.
    pub fn edited(&mut self) {
        self.dirty = true;
        self.reset = true;
    }

    /// Move the clock (video seconds; clamped to 0 and the sequence's End).
    pub fn seek(&mut self, t: f32) {
        let end = self.seq.as_ref().and_then(|s| s.end).unwrap_or(f32::MAX);
        self.t = t.clamp(0.0, end);
    }

    /// The race second of the clock / the video second of a race second.
    pub fn race_secs(&self) -> f32 {
        self.seq.as_ref().map_or(self.t, |s| race_secs(s, self.t))
    }

    pub fn video_secs(&self, r: f32) -> f32 {
        self.seq.as_ref().map_or(r, |s| video_secs(s, r))
    }

    /// The time slider's end: the sequence's End, else a minute past the recorded run.
    pub fn slider_end(&self, sess: Option<&Session>) -> f32 {
        match self.seq.as_ref().and_then(|s| s.end) {
            Some(e) => e,
            None => {
                let recorded = sess.and_then(|s| s.timeline.as_ref()).map_or(0.0, |t| t.span().2 as f32);
                self.video_secs(recorded).max(self.t) + SLIDER_AHEAD
            }
        }
    }

    /// The camera on air at the clock.
    pub fn on_air(&self) -> Option<usize> {
        self.seq.as_ref().map(|s| on_air(s, self.t))
    }

    /// The camera to look through and the slot it shows / hides, when previewing.
    pub fn preview_cam(&self) -> Option<(Camera, usize, bool)> {
        let i = match self.preview {
            Preview::Off => return None,
            Preview::Selected => self.sel,
            Preview::OnAir => self.on_air()?,
        };
        self.dir.as_ref()?.view(i)
    }

    /// Write the sequence to `path` (else its own file).
    pub fn save(&mut self, path: Option<PathBuf>) -> Result<PathBuf, String> {
        let seq = self.seq.as_ref().ok_or("no sequence")?;
        let p = path.or_else(|| self.path.clone()).ok_or("no file name")?;
        if let Some(d) = p.parent().filter(|d| !d.as_os_str().is_empty()) {
            std::fs::create_dir_all(d).map_err(|e| format!("{}: {e}", d.display()))?;
        }
        std::fs::write(&p, cine::to_text(seq)).map_err(|e| format!("{}: {e}", p.display()))?;
        self.path_text = p.display().to_string();
        (self.path, self.dirty, self.files) = (Some(p.clone()), false, None);
        Ok(p)
    }

    /// One viewer frame with a sequence open: the clock moves `dt` video seconds (0 when paused), the race is brought
    /// to the clock's race second and the director runs.
    pub fn frame(&mut self, sess: &mut Session, dt: f32) {
        let Some(seq) = self.seq.as_ref() else { return };
        if dt > 0.0 && !self.behind {
            self.t += dt;
            if let Some(e) = seq.end {
                self.t = self.t.min(e);
            }
        }
        let step = 1.0 / seq.fps.max(1) as f32;
        self.behind = !sync(sess, race_secs(seq, self.t), step, RUN_AHEAD);
        if self.behind {
            self.reset = true;
            return;
        }
        let gap = self.t - self.dir_t;
        let fresh = self.reset || self.dir.is_none() || !(0.0..=JUMP).contains(&gap);
        if fresh {
            // APPROX: after a jump or an edit the cameras start from rest (eased aims, chase easing, shake levels);
            // the render runs them from time 0
            self.dir = Some(Director::new(seq));
        }
        if fresh || gap > 0.0 {
            if let Some(d) = self.dir.as_mut() {
                d.camera(seq, &sess.race, self.t, if fresh { 0.0 } else { gap });
            }
            self.dir_t = self.t;
        }
        self.reset = false;
    }

    /// Run a `Req` that edits the sequence (not New / Open / ApplyRace, which the viewer runs). `cam` = the viewer's
    /// camera, `sess` = the race.
    pub fn edit(&mut self, req: Req, cam: &Camera, sess: Option<&Session>) {
        let Some(seq) = self.seq.as_mut() else { return };
        let here = round3(cam.pos * FLIP);
        let rot = game_rot(cam);
        match req {
            Req::AddStatic => {
                let c = static_here(seq, cam, self.target);
                seq.cameras.push(c);
                self.sel = seq.cameras.len() - 1;
            }
            Req::AddMounted => {
                let Some(ss) = sess else { return self.msg = "no race loaded".into() };
                let slot = ss.focus.min(ss.race.racers.len() - 1);
                let car = &ss.race.racers[slot].car;
                let target = if self.target == Target::Nearest { Target::Car(slot) } else { self.target };
                // where the free camera is relative to the followed car (else the roof)
                let local = car.rot.transpose() * (here - cam_origin(car));
                let (offset, angles) = if local.length() < 15.0 { (round3(local), angles_of(car.rot.transpose() * rot)) } else { (MOUNT_OFFSET, Vec3::new(5.0, 0.0, 0.0)) };
                let name = next_name(seq, "mount");
                seq.cameras.push(CineCam { name, kind: CamKind::Mounted { target, offset, angles, zoom: 1.0, hide: false }, keys: Vec::new(), shakes: Vec::new() });
                self.sel = seq.cameras.len() - 1;
            }
            Req::AddPath => {
                let key = CamKey { at: 0.0, eye: Some(here), angles: Some(angles_of(rot)), fov: None, tcb: [0.0; 3], linear: false };
                let name = next_name(seq, "path");
                seq.cameras.push(CineCam { name, kind: CamKind::Path { drive: Drive::Time, aim: PathAim::Keys, zoom: Zoom::Fixed(1.0), clear: false }, keys: vec![key], shakes: Vec::new() });
                self.sel = seq.cameras.len() - 1;
            }
            Req::AddKey => {
                let sel = self.sel.min(seq.cameras.len() - 1);
                let at = match &seq.cameras[sel].kind {
                    CamKind::Path { drive: Drive::Car(t), .. } => {
                        let Some(ss) = sess else { return self.msg = "no race loaded".into() };
                        let slot = match t {
                            Target::Car(n) => (*n).min(ss.race.racers.len() - 1),
                            Target::Player => ss.race.racers.len() - 1,
                            _ => ss.race.standings().first().copied().unwrap_or(0),
                        };
                        ss.race.racers[slot].progress.s
                    }
                    _ => since_shot(seq, sel, self.t),
                };
                let at = (at * 100.0).round() / 100.0;
                let c = &mut seq.cameras[sel];
                let path = matches!(c.kind, CamKind::Path { .. });
                let fov = (cam.zoom != 1.0 || c.keys.iter().any(|k| k.fov.is_some())).then(|| (fov_of(cam.zoom) * 10.0).round() / 10.0);
                let key = CamKey { at, eye: path.then_some(here), angles: path.then(|| angles_of(rot)), fov: fov.or((!path).then_some(FOV_X)), tcb: [0.0; 3], linear: false };
                // a key at the same time is replaced
                match c.keys.iter().position(|k| (k.at - at).abs() < 1e-3) {
                    Some(i) => c.keys[i] = key,
                    None => c.keys.push(key),
                }
                if !path {
                    self.msg = format!("{}: a Fov key at {at} s (only path cameras have Eye / Angles keys)", c.name);
                }
            }
            Req::EyeFromCamera => {
                let sel = self.sel.min(seq.cameras.len() - 1);
                match &mut seq.cameras[sel].kind {
                    CamKind::Static { eye, aim, .. } => {
                        *eye = here;
                        if let Aim::At(a) = aim {
                            *a = round3(here + rot.z_axis * 20.0);
                        }
                    }
                    CamKind::Mounted { target, offset, angles, .. } => {
                        let Some(ss) = sess else { return };
                        let car = &ss.race.racers[match target {
                            Target::Car(n) => (*n).min(ss.race.racers.len() - 1),
                            Target::Player => ss.race.racers.len() - 1,
                            _ => ss.race.standings().first().copied().unwrap_or(0),
                        }]
                        .car;
                        *offset = round3(car.rot.transpose() * (here - cam_origin(car)));
                        *angles = angles_of(car.rot.transpose() * rot);
                    }
                    _ => return self.msg = "only static and mounted cameras have an eye of their own".into(),
                }
            }
            Req::KeyFromCamera(k) => {
                let sel = self.sel.min(seq.cameras.len() - 1);
                let Some(key) = seq.cameras[sel].keys.get_mut(k) else { return };
                if key.eye.is_some() {
                    key.eye = Some(here);
                }
                if key.angles.is_some() {
                    key.angles = Some(angles_of(rot));
                }
                if key.fov.is_some() {
                    key.fov = Some((fov_of(cam.zoom) * 10.0).round() / 10.0);
                }
            }
            Req::Cut => {
                let sel = self.sel.min(seq.cameras.len() - 1);
                let t = (self.t * 100.0).round() / 100.0;
                match seq.shots.iter().position(|s| (s.t - t).abs() < 1e-3) {
                    Some(i) => seq.shots[i].cam = sel,
                    None => seq.shots.push(Shot::new(t, sel)),
                }
            }
            Req::New | Req::Open(_) | Req::ApplyRace | Req::Render => return,
        }
        tidy(seq, here);
        self.edited();
    }

    /// Remove the selected camera (not the last one) and its shots.
    pub fn remove_selected(&mut self) {
        let Some(seq) = self.seq.as_mut() else { return };
        if seq.cameras.len() < 2 {
            return self.msg = "a sequence keeps one camera".into();
        }
        let i = self.sel.min(seq.cameras.len() - 1);
        seq.cameras.remove(i);
        seq.shots.retain(|s| s.cam != i);
        for s in &mut seq.shots {
            if s.cam > i {
                s.cam -= 1;
            }
        }
        self.sel = i.saturating_sub(1);
        tidy(seq, Vec3::ZERO);
        self.edited();
    }

    /// Clock to the start of the previous (-1) / next (1) shot.
    pub fn shot_step(&mut self, dir: i32) {
        let Some(seq) = self.seq.as_ref() else { return };
        let i = shot_at(seq, self.t);
        let t = if dir < 0 {
            // inside a shot: its start; at its start: the one before
            let cur = seq.shots[i].t;
            if self.t > cur + 0.05 {
                cur
            } else {
                seq.shots[i.saturating_sub(1)].t
            }
        } else {
            match seq.shots.get(i + 1) {
                Some(s) => s.t,
                None => return,
            }
        };
        self.seek(t);
    }
}

// ---------------------------------------------------------------- gizmos, picking, render

/// Where handle `h` of camera `i` is (game space; a mounted camera's on its car now).
fn handle_pos(seq: &Sequence, i: usize, h: Handle, race: Option<&Race>) -> Option<Vec3> {
    let c = seq.cameras.get(i)?;
    match (h, &c.kind) {
        (Handle::Eye, CamKind::Static { eye, .. }) => Some(*eye),
        (Handle::Eye, CamKind::Mounted { target, offset, .. }) => {
            let race = race?;
            let car = &race.racers[cine::resolve(race, *target, None)].car;
            Some(cam_origin(car) + car.rot * *offset)
        }
        (Handle::Aim, CamKind::Static { aim: Aim::At(a), .. }) => Some(*a),
        (Handle::Key(k), _) => c.keys.get(k)?.eye,
        _ => None,
    }
}

/// Put handle `h` of camera `i` at `p` (a mounted camera: its offset on the car).
fn set_handle(seq: &mut Sequence, i: usize, h: Handle, p: Vec3, race: Option<&Race>) {
    let Some(c) = seq.cameras.get_mut(i) else { return };
    match (h, &mut c.kind) {
        (Handle::Eye, CamKind::Static { eye, .. }) => *eye = p,
        (Handle::Eye, CamKind::Mounted { target, offset, .. }) => {
            if let Some(race) = race {
                let car = &race.racers[cine::resolve(race, *target, None)].car;
                *offset = car.rot.transpose() * (p - cam_origin(car));
            }
        }
        (Handle::Aim, CamKind::Static { aim: Aim::At(a), .. }) => *a = p,
        (Handle::Key(k), _) => {
            if let Some(e) = c.keys.get_mut(k).and_then(|k| k.eye.as_mut()) {
                *e = p;
            }
        }
        _ => {}
    }
}

/// The 8 corners of a frustum (bit 0 x, bit 1 y, bit 2 far) at `p` along `rot`, `tan` = half the fields' tangents.
fn frustum(p: Vec3, rot: Mat3, tan: (f32, f32), depth: f32) -> [Vec3; 8] {
    std::array::from_fn(|i| {
        let d = if i & 4 != 0 { depth } else { depth * 0.1 };
        let (sx, sy) = (if i & 1 != 0 { 1.0 } else { -1.0 }, if i & 2 != 0 { 1.0 } else { -1.0 });
        p + rot.z_axis * d + rot.x_axis * (sx * tan.0 * d) + rot.y_axis * (sy * tan.1 * d)
    })
}

impl CineEdit {
    /// The camera the view looks through (its gizmos are not drawn).
    pub fn previewed(&self) -> Option<usize> {
        match self.preview {
            Preview::Off => None,
            Preview::Selected => Some(self.sel),
            Preview::OnAir => self.on_air(),
        }
    }

    /// The gizmo's handle on the selected camera: the chosen one when the camera has it, else its first (a static or
    /// mounted camera's eye, a path's first Eye key; the game's views have none).
    pub fn handle(&self) -> Option<Handle> {
        let c = self.seq.as_ref()?.cameras.get(self.sel)?;
        let has = |h: Handle| match (h, &c.kind) {
            (Handle::Eye, CamKind::Static { .. } | CamKind::Mounted { .. }) | (Handle::Aim, CamKind::Static { aim: Aim::At(_), .. }) => true,
            (Handle::Key(k), _) => c.keys.get(k).is_some_and(|k| k.eye.is_some()),
            _ => false,
        };
        self.handle.filter(|&(i, h)| i == self.sel && has(h)).map(|(_, h)| h).or_else(|| match c.kind {
            CamKind::Static { .. } | CamKind::Mounted { .. } => Some(Handle::Eye),
            CamKind::Path { .. } => c.keys.iter().position(|k| k.eye.is_some()).map(Handle::Key),
            CamKind::Pose { .. } => None,
        })
    }

    /// The move gizmo: its handle and where it is.
    fn gizmo_at(&self, race: Option<&Race>) -> Option<(Handle, Vec3)> {
        let h = self.handle()?;
        Some((h, handle_pos(self.seq.as_ref()?, self.sel, h, race)?))
    }

    /// What a click can pick: (camera, handle (None: the camera's own), game position). A static camera is picked
    /// by its eye, the others where the director has them now.
    fn points(&self, race: Option<&Race>) -> Vec<(usize, Option<Handle>, Vec3)> {
        let (Some(seq), skip) = (self.seq.as_ref(), self.previewed()) else { return Vec::new() };
        let mut v = Vec::new();
        for (i, c) in seq.cameras.iter().enumerate() {
            if Some(i) != skip {
                match &c.kind {
                    CamKind::Static { eye, .. } => v.push((i, Some(Handle::Eye), *eye)),
                    CamKind::Mounted { .. } => v.extend(handle_pos(seq, i, Handle::Eye, race).map(|p| (i, Some(Handle::Eye), p))),
                    _ => v.extend(self.dir.as_ref().and_then(|d| d.view(i)).map(|(cam, ..)| (i, None, cam.pos * FLIP))),
                }
            }
            if let CamKind::Static { aim: Aim::At(a), .. } = c.kind {
                v.push((i, Some(Handle::Aim), a));
            }
            v.extend(c.keys.iter().enumerate().filter_map(|(k, key)| Some((i, Some(Handle::Key(k)), key.eye?))));
        }
        v
    }

    /// The editor's lines in the world: each camera's frustum (its zoom, the video's aspect) and name, a path's curve
    /// and key crosses, a tracking camera's line to its car, a fixed aim point; the selected camera yellow and drawn
    /// over everything, the one on air orange; the move gizmo on the selected handle. `eye` = the view's eye (game
    /// space), `fov_x` its field at zoom 1. Nothing while previewing on air (the video's picture).
    pub fn gizmos(&self, g: &mut Gizmos, eye: Vec3, fov_x: f32, race: Option<&Race>) {
        let (Some(seq), Some(dir)) = (self.seq.as_ref(), self.dir.as_ref()) else { return };
        if self.preview == Preview::OnAir {
            return;
        }
        let aspect = seq.size.0 as f32 / seq.size.1.max(1) as f32;
        let (air, skip) = (self.on_air(), self.previewed());
        for (i, c) in seq.cameras.iter().enumerate() {
            let sel = i == self.sel;
            let col = if sel { debug::YELLOW } else if Some(i) == air { debug::ORANGE } else { debug::CYAN };
            if matches!(c.kind, CamKind::Path { .. }) {
                g.polyline(&path_points(c, PATH_POINTS), false, col, sel);
            }
            let held = if sel { self.handle() } else { None };
            for (k, key) in c.keys.iter().enumerate() {
                let Some(e) = key.eye else { continue };
                let r = (e.distance(eye) * CROSS_K).max(CROSS_MIN);
                g.cross(e, r, if held == Some(Handle::Key(k)) { debug::WHITE } else { col }, sel);
                if sel {
                    g.label(e + Vec3::Y * r * 2.5, format!("{:.2}", key.at), col);
                }
            }
            if let CamKind::Static { eye: e, aim: Aim::At(a), .. } = c.kind {
                g.line(e, a, col, false);
                g.cross(a, (a.distance(eye) * CROSS_K).max(CROSS_MIN), if held == Some(Handle::Aim) { debug::WHITE } else { col }, sel);
            }
            if Some(i) == skip {
                continue;
            }
            let Some((cam, slot, _)) = dir.view(i) else { continue };
            let (p, rot) = (cam.pos * FLIP, game_rot(&cam));
            let tx = (fov_x.to_radians() * 0.5).tan() / cam.zoom.max(1e-3);
            let depth = (p.distance(eye) * FRUSTUM_K).max(FRUSTUM_MIN);
            let k = frustum(p, rot, (tx, tx / aspect), depth);
            g.cuboid(k, col, sel);
            for c in &k[..4] {
                g.line(p, *c, col, sel);
            }
            // the frame's top: a tick above the far edge (roll)
            let top = (k[6] + k[7]) * 0.5;
            g.line(k[6], top + rot.y_axis * (tx / aspect) * depth * 0.6, col, sel);
            g.line(k[7], top + rot.y_axis * (tx / aspect) * depth * 0.6, col, sel);
            g.label(p - rot.y_axis * (depth * 0.3 + 0.2), c.name.as_str(), col);
            let tracks = matches!(c.kind, CamKind::Static { aim: Aim::Track { .. }, .. } | CamKind::Path { aim: PathAim::Track { .. }, .. });
            if let (true, Some(r)) = (tracks, race) {
                g.line(p, cam_origin(&r.racers[slot.min(r.racers.len() - 1)].car), col, false);
            }
        }
        if let Some((_, pivot)) = self.gizmo_at(race).filter(|(_, p)| p.distance(eye) > GIZMO_NEAR) {
            fill_gizmo::draw(g, GizmoMode::Move, pivot, [true; 3], AXES, self.hot, eye);
        }
    }

    /// LMB in the world: grab an arrow of the move gizmo, else select the camera, key or aim point within `PICK_PX`
    /// of the cursor. `project` = game space to window pixels, `ray` = the mouse ray (game space). True when taken.
    pub fn press(&mut self, project: &dyn Fn(Vec3) -> Option<(f32, f32)>, cursor: (f32, f32), ray: (Vec3, Vec3), race: Option<&Race>) -> bool {
        if self.preview == Preview::OnAir {
            return false;
        }
        if let Some((handle, pivot)) = self.gizmo_at(race).filter(|(_, p)| p.distance(ray.0) > GIZMO_NEAR) {
            let axis = fill_gizmo::pick(GizmoMode::Move, pivot, [true; 3], AXES, ray.0, project, cursor);
            if let Some((axis, from)) = axis.and_then(|a| Some((a, fill_gizmo::axis_param(pivot, AXES[a], ray)?))) {
                (self.drag, self.hot) = (Some(Drag { handle, axis, base: pivot, from }), Some(axis));
                return true;
            }
        }
        let near = self
            .points(race)
            .into_iter()
            .filter_map(|(i, h, p)| {
                let (x, y) = project(p)?;
                let d = (x - cursor.0).hypot(y - cursor.1);
                (d <= PICK_PX).then_some((i, h, d))
            })
            .min_by(|a, b| a.2.total_cmp(&b.2));
        let Some((i, h, _)) = near else { return false };
        (self.sel, self.handle) = (i, h.map(|h| (i, h)));
        true
    }

    /// The mouse moved: the held handle follows it along its axis, else the arrow under it lights up.
    pub fn mouse_move(&mut self, project: &dyn Fn(Vec3) -> Option<(f32, f32)>, cursor: (f32, f32), ray: (Vec3, Vec3), race: Option<&Race>) {
        if let Some(d) = self.drag {
            let Some(t) = fill_gizmo::axis_param(d.base, AXES[d.axis], ray) else { return };
            if let Some(seq) = self.seq.as_mut() {
                set_handle(seq, self.sel, d.handle, d.base + AXES[d.axis] * (t - d.from), race);
                self.edited();
            }
            return;
        }
        self.hot = if self.preview == Preview::OnAir { None } else { self.gizmo_at(race).and_then(|(_, p)| fill_gizmo::pick(GizmoMode::Move, p, [true; 3], AXES, ray.0, project, cursor)) };
    }

    /// LMB up: a drag ends (its point rounded to millimetres, as the file keeps it).
    pub fn release(&mut self, race: Option<&Race>) {
        let Some(d) = self.drag.take() else { return };
        self.hot = None;
        let Some(seq) = self.seq.as_mut() else { return };
        if let Some(p) = handle_pos(seq, self.sel, d.handle, race) {
            set_handle(seq, self.sel, d.handle, round3(p), race);
        }
        let i = self.sel.min(seq.cameras.len() - 1);
        if let CamKind::Mounted { offset, .. } = &mut seq.cameras[i].kind {
            *offset = round3(*offset);
        }
        self.edited();
    }

    pub fn dragging(&self) -> bool {
        self.drag.is_some()
    }

    /// Where Render writes: the sequence's file as `.mp4` (New and Open always give it one).
    pub fn render_path(&self) -> Option<PathBuf> {
        self.path.as_ref().map(|p| p.with_extension("mp4"))
    }

    /// Render the sequence as it is in the editor (saved or not) to `render_path` on a worker thread, with the
    /// viewer's graphics options (`view`, `passes`; no racing line) at the sequence's size and fps.
    pub fn render(&mut self, root: &Path, view: View, passes: Passes) {
        if self.job.is_some() {
            return self.msg = "a render is already running".into();
        }
        let (Some(seq), Some(out)) = (self.seq.clone(), self.render_path()) else { return self.msg = "no sequence file to render next to".into() };
        let watch = Watch::default();
        let opts = RenderOpts { view: View { show_line: false, ..view }, passes, size: seq.size, fps: seq.fps, from: 0.0, to: None, codec: Codec::H264, watch: Some(watch.clone()), audio: true };
        let (root, o) = (root.to_path_buf(), out.clone());
        match std::thread::Builder::new().name("cine-render".into()).spawn(move || cine_render::render(&root, &opts, &seq, Out::Video(&o))) {
            Ok(t) => {
                self.msg = format!("rendering {}", out.display());
                self.job = Some(Job { out, watch, thread: Some(t), started: Instant::now() });
            }
            Err(e) => self.msg = format!("render: {e}"),
        }
    }

    /// The running render: its line for the status bar, its progress (0..1, None while the length is not known)
    /// and its cancel flag.
    pub fn job_status(&self) -> Option<(String, Option<f32>, &AtomicBool)> {
        let j = self.job.as_ref()?;
        let (n, total) = (j.watch.frames.load(Ordering::Relaxed), j.watch.total.load(Ordering::Relaxed));
        let name = j.out.file_name().map_or_else(String::new, |n| n.to_string_lossy().into_owned());
        let cancel: &AtomicBool = &j.watch.cancel;
        if cancel.load(Ordering::Relaxed) {
            return Some((format!("{} stopping…", ic::VIDEO), None, cancel));
        }
        Some(match total {
            0 => (format!("{} rendering {name}: frame {n}", ic::VIDEO), None, cancel),
            t => (format!("{} rendering {name}: {n} / {t}", ic::VIDEO), Some(n as f32 / t as f32), cancel),
        })
    }

    /// A render that has ended: its note (the job is gone).
    pub fn poll_job(&mut self) -> Option<String> {
        if !self.job.as_ref()?.thread.as_ref()?.is_finished() {
            return None;
        }
        let mut job = self.job.take()?;
        let result = job.thread.take()?.join().unwrap_or_else(|_| Err("the render thread panicked".into()));
        let n = job.watch.frames.load(Ordering::Relaxed);
        let note = match result {
            Ok(()) => format!("rendered {} ({n} frames in {:.0} s)", job.out.display(), job.started.elapsed().as_secs_f32()),
            Err(e) if e == CANCELLED => format!("render cancelled after {n} frames: {}", job.out.display()),
            Err(e) => format!("render failed: {e}"),
        };
        self.msg = note.clone();
        Some(note)
    }
}

// ---------------------------------------------------------------- the Properties page

fn vec3_row(ui: &mut Ui, v: &mut Vec3, speed: f64) -> bool {
    let mut ch = false;
    for (c, p) in [&mut v.x, &mut v.y, &mut v.z].into_iter().zip(["x ", "y ", "z "]) {
        ch |= wheel_drag(ui, c, None, |d| d.speed(speed).max_decimals(3).prefix(p)).changed();
    }
    ch
}

fn num(ui: &mut Ui, v: &mut f32, lo: f32, hi: f32, speed: f64) -> bool {
    wheel_drag(ui, v, Some(lo..=hi), |d| d.speed(speed).max_decimals(3)).changed()
}

/// A Zoom field: fixed or auto (metres). True when changed.
fn zoom_row(ui: &mut Ui, id: usize, z: &mut Zoom, auto_ok: bool) -> bool {
    let mut ch = false;
    let auto = matches!(z, Zoom::Auto(_));
    if auto_ok {
        ComboBox::from_id_salt(("cine-zoom", id)).width(64.0).selected_text(if auto { "auto" } else { "fixed" }).show_ui(ui, |ui| {
            if ui.selectable_label(!auto, "fixed").clicked() && auto {
                *z = Zoom::Fixed(1.0);
                ch = true;
            }
            if ui.selectable_label(auto, "auto").clicked() && !auto {
                *z = Zoom::Auto(NEW_AUTO);
                ch = true;
            }
        });
    }
    match z {
        Zoom::Fixed(v) => ch |= wheel_drag(ui, v, Some(0.1..=10.0), |d| d.speed(0.01).max_decimals(2).prefix("x ")).changed(),
        Zoom::Auto(r) => ch |= wheel_drag(ui, r, Some(1.0..=500.0), |d| d.speed(0.2).max_decimals(1).suffix(" m")).on_hover_text("zoom 1 at this distance, closer cars fill the same part of the frame").changed(),
    }
    ch
}

/// Turn camera `c` into kind `name`, keeping what carries over (the eye, the target).
fn convert(c: &mut CineCam, name: &str, eye: Vec3, fallback: Target) {
    let target = target_of(&c.kind).unwrap_or(fallback);
    c.kind = match KINDS.iter().find(|k| k.0 == name) {
        Some((_, Some(mode))) => CamKind::Pose { mode: *mode, target: no_nearest(target) },
        Some(("mounted", _)) => CamKind::Mounted { target: no_nearest(target), offset: MOUNT_OFFSET, angles: Vec3::new(5.0, 0.0, 0.0), zoom: 1.0, hide: false },
        Some(("path", _)) => {
            if !c.keys.iter().any(|k| k.eye.is_some()) {
                c.keys.insert(0, CamKey { at: 0.0, eye: Some(eye), angles: None, fov: None, tcb: [0.0; 3], linear: false });
            }
            CamKind::Path { drive: Drive::Time, aim: PathAim::Track { target: no_nearest(target), lag: NEW_LAG, lead: NEW_LEAD }, zoom: Zoom::Auto(NEW_AUTO), clear: true }
        }
        _ => CamKind::Static { eye, aim: Aim::Track { target, lag: NEW_LAG, lead: NEW_LEAD }, zoom: Zoom::Auto(NEW_AUTO), clear: true },
    };
}

/// The Cine page of the Properties pane. `root` = the install (the file list), `names` = the drivers by slot (`names`).
pub fn page(ui: &mut Ui, ed: &mut CineEdit, root: &Path, names: &[String]) {
    file_section(ui, ed, root);
    if ed.seq.is_none() {
        return;
    }
    race_section(ui, ed);
    camera_section(ui, ed, names);
    keys_section(ui, ed);
    shots_section(ui, ed);
}

fn file_section(ui: &mut Ui, ed: &mut CineEdit, root: &Path) {
    section(ui, ic::FILM, "File", false, |ui| {
        ui.horizontal(|ui| {
            ui.add(egui::TextEdit::singleline(&mut ed.path_text).desired_width(200.0).hint_text("name.cine or a path"));
            let p = PathBuf::from(ed.path_text.trim());
            if ui.add_enabled(!ed.path_text.trim().is_empty(), egui::Button::new("open")).clicked() {
                ed.reqs.push(Req::Open(resolve(root, &p)));
            }
            if ui.add_enabled(ed.seq.is_some() && !ed.path_text.trim().is_empty(), egui::Button::new("save as")).clicked() {
                let p = resolve(root, &p);
                ed.msg = match ed.save(Some(p)) {
                    Ok(p) => format!("saved {}", p.display()),
                    Err(e) => e,
                };
            }
        });
        if !ed.msg.is_empty() {
            ui.weak(&ed.msg);
        }
        ui.horizontal(|ui| {
            ui.strong(format!("{}", dir(root).display()));
            if ui.small_button("refresh").clicked() {
                ed.files = None;
            }
        });
        let files = ed.files.get_or_insert_with(|| list(root)).clone();
        if files.is_empty() {
            ui.weak("no .cine files yet");
        }
        ui.horizontal_wrapped(|ui| {
            for f in files {
                let name = f.file_name().map_or_else(String::new, |n| n.to_string_lossy().into_owned());
                if ui.selectable_label(ed.path.as_ref() == Some(&f), name).clicked() {
                    ed.reqs.push(Req::Open(f));
                }
            }
        });
        ui.weak("saving rewrites the file: comments of a hand-written one are not kept");
    });
}

fn race_section(ui: &mut Ui, ed: &mut CineEdit) {
    let mut ch = false;
    let mut apply = false;
    let Some(seq) = ed.seq.as_mut() else { return };
    section(ui, ic::FLAG, "Race and output", false, |ui| {
        egui::Grid::new("cine-race").num_columns(2).spacing([8.0, 4.0]).show(ui, |ui| {
            let r = &mut seq.race;
            ui.label("track");
            ch |= wheel_drag(ui, &mut r.track, Some(1..=8), |d| d.speed(0.05)).changed();
            ui.end_row();
            ui.label("cars");
            if ui.add(egui::TextEdit::singleline(&mut ed.cars_text).desired_width(160.0)).on_hover_text("car numbers in grid order, slot 0 first (the last slot is `player`)").changed() {
                let cars: Vec<u32> = ed.cars_text.split_whitespace().filter_map(|w| w.parse().ok()).filter(|c| (1..=10).contains(c)).collect();
                if !cars.is_empty() && cars.len() <= 8 && cars != r.cars {
                    r.cars = cars;
                    ch = true;
                }
            }
            ui.end_row();
            ui.label("difficulty");
            ComboBox::from_id_salt("cine-diff").selected_text(DIFFICULTIES.iter().find(|d| d.1 == r.difficulty).map_or("?", |d| d.0)).show_ui(ui, |ui| {
                for (n, d) in DIFFICULTIES {
                    ch |= ui.selectable_value(&mut r.difficulty, d, n).changed();
                }
            });
            ui.end_row();
            ui.label("laps");
            ui.horizontal(|ui| {
                let mut own = r.laps.is_some();
                if ui.checkbox(&mut own, "").on_hover_text("off: the track's default").changed() {
                    r.laps = own.then_some(3);
                    ch = true;
                }
                if let Some(l) = r.laps.as_mut() {
                    ch |= wheel_drag(ui, l, Some(1..=99), |d| d.speed(0.05)).changed();
                }
            });
            ui.end_row();
            ui.label("start");
            ui.horizontal(|ui| {
                ch |= ui.checkbox(&mut r.catchup, "catch-up").changed();
                ch |= ui.selectable_value(&mut r.start_go, false, "grid").on_hover_text("time 0 = the \"3\" of the countdown").changed();
                ch |= ui.selectable_value(&mut r.start_go, true, "go").on_hover_text("time 0 = the green light").changed();
            });
            ui.end_row();
            ui.label("size");
            ui.horizontal(|ui| {
                ch |= wheel_drag(ui, &mut seq.size.0, Some(16..=7680), |d| d.speed(2.0)).changed();
                ui.label("x");
                ch |= wheel_drag(ui, &mut seq.size.1, Some(16..=4320), |d| d.speed(2.0)).changed();
            });
            ui.end_row();
            ui.label("fps");
            ch |= wheel_drag(ui, &mut seq.fps, Some(1..=240), |d| d.speed(0.2)).changed();
            ui.end_row();
            ui.label("end");
            ui.horizontal(|ui| {
                let mut own = seq.end.is_some();
                if ui.checkbox(&mut own, "").on_hover_text("off: every car finished + 2 s").changed() {
                    seq.end = own.then_some(ed.t.max(1.0));
                    ch = true;
                }
                if let Some(e) = seq.end.as_mut() {
                    ch |= wheel_drag(ui, e, Some(0.1..=1200.0), |d| d.speed(0.1).max_decimals(2).suffix(" s")).changed();
                }
            });
            ui.end_row();
            // #153 the render's sound (the editor's preview stays silent)
            ui.label("audio");
            ui.horizontal(|ui| {
                ch |= ui.checkbox(&mut seq.audio.on, "").on_hover_text("the race's sound, heard by the camera on air, in the rendered video").changed();
                ui.add_enabled_ui(seq.audio.on, |ui| {
                    ch |= ui.checkbox(&mut seq.audio.doppler, "Doppler").on_hover_text("pitch rises as a car comes at the camera and drops as it goes away").changed();
                });
            });
            ui.end_row();
        });
        let differs = ed.loaded.as_ref() != Some(&seq.race);
        ui.add_enabled_ui(differs, |ui| {
            apply = ui.button(format!("{}  apply race", ic::RELOAD)).on_hover_text("reload the race from these values (the clock goes back to 0)").clicked();
        });
        if differs {
            ui.weak("the race differs from the one loaded");
        }
    });
    if apply {
        ed.reqs.push(Req::ApplyRace);
    }
    if ch {
        ed.edited();
    }
}

fn camera_section(ui: &mut Ui, ed: &mut CineEdit, names: &[String]) {
    let eye_now = ed.dir.as_ref().and_then(|d| d.view(ed.sel)).map(|v| round3(v.0.pos * FLIP));
    let fallback = ed.target;
    let (mut ch, mut remove, mut reqs) = (false, false, Vec::new());
    let Some(seq) = ed.seq.as_mut() else { return };
    let ncars = seq.race.cars.len();
    ed.sel = ed.sel.min(seq.cameras.len() - 1);
    let sel = ed.sel;
    section(ui, ic::VIDEO, "Camera", false, |ui| {
        ui.horizontal(|ui| {
            ComboBox::from_id_salt("cine-sel").selected_text(&seq.cameras[sel].name).show_ui(ui, |ui| {
                for (i, c) in seq.cameras.iter().enumerate() {
                    ui.selectable_value(&mut ed.sel, i, format!("{}  ({})", c.name, kind_name(&c.kind)));
                }
            });
            remove = ui.add_enabled(seq.cameras.len() > 1, egui::Button::new(ic::TRASH)).on_hover_text("remove this camera and its shots").clicked();
        });
        if ed.sel != sel {
            return;
        }
        let mut name = seq.cameras[sel].name.clone();
        let mut new_kind = None;
        egui::Grid::new("cine-cam").num_columns(2).spacing([8.0, 4.0]).show(ui, |ui| {
            ui.label("name");
            if ui.add(egui::TextEdit::singleline(&mut name).desired_width(140.0)).changed() {
                let n = unique_name(seq, &name, Some(sel));
                seq.cameras[sel].name = n;
                ch = true;
            }
            ui.end_row();
            ui.label("kind");
            let cur = kind_name(&seq.cameras[sel].kind);
            ComboBox::from_id_salt("cine-kind").selected_text(cur).show_ui(ui, |ui| {
                for (k, _) in KINDS {
                    if ui.selectable_label(k == cur, k).clicked() && k != cur {
                        new_kind = Some(k);
                    }
                }
            });
            ui.end_row();
            let c = &mut seq.cameras[sel];
            match &mut c.kind {
                CamKind::Static { eye, aim, zoom, clear } => {
                    ui.label("eye");
                    ui.horizontal(|ui| {
                        ch |= vec3_row(ui, eye, 0.05);
                        if ui.small_button(ic::CAMERA).on_hover_text("to the free camera (Ctrl+K adds a new one)").clicked() {
                            reqs.push(Req::EyeFromCamera);
                        }
                    });
                    ui.end_row();
                    ui.label("aim");
                    let tracking = matches!(aim, Aim::Track { .. });
                    ui.horizontal(|ui| {
                        if ui.selectable_label(tracking, "car").clicked() && !tracking {
                            *aim = Aim::Track { target: fallback, lag: NEW_LAG, lead: NEW_LEAD };
                            ch = true;
                        }
                        if ui.selectable_label(!tracking, "point").clicked() && tracking {
                            *aim = Aim::At(*eye + Vec3::Z * 20.0);
                            ch = true;
                        }
                    });
                    ui.end_row();
                    ch |= aim_rows(ui, aim, ncars, names);
                    ui.label("zoom");
                    ui.horizontal(|ui| ch |= zoom_row(ui, sel, zoom, true));
                    ui.end_row();
                    ui.label("clear view");
                    ch |= ui.checkbox(clear, "").on_hover_text("move in front of what hides the car (the TV camera's test)").changed();
                    ui.end_row();
                }
                CamKind::Mounted { target, offset, angles, zoom, hide } => {
                    ui.label("on");
                    ch |= target_combo(ui, "cine-mt", target, false, ncars, names);
                    ui.end_row();
                    ui.label("offset");
                    ui.horizontal(|ui| {
                        ch |= vec3_row(ui, offset, 0.02);
                        if ui.small_button(ic::CAMERA).on_hover_text("offset and angles to the free camera, relative to the car").clicked() {
                            reqs.push(Req::EyeFromCamera);
                        }
                    });
                    ui.end_row();
                    ui.label("angles");
                    ch |= ui.horizontal(|ui| vec3_row(ui, angles, 0.5)).inner;
                    ui.end_row();
                    ui.label("zoom");
                    ch |= wheel_drag(ui, zoom, Some(0.1..=10.0), |d| d.speed(0.01).max_decimals(2).prefix("x ")).changed();
                    ui.end_row();
                    ui.label("hide car");
                    ch |= ui.checkbox(hide, "").on_hover_text("the camera sits inside the car: leave its body out").changed();
                    ui.end_row();
                }
                CamKind::Pose { mode, target } => {
                    ui.label("follows");
                    ch |= target_combo(ui, "cine-pt", target, false, ncars, names);
                    ui.end_row();
                    ui.label("view");
                    ui.weak(format!("the game's {} camera", POSE_MODES.iter().find(|m| m.1 == *mode).map_or("?", |m| m.0)));
                    ui.end_row();
                }
                CamKind::Path { drive, aim, zoom, clear } => {
                    ui.label("paced by");
                    let by_car = matches!(drive, Drive::Car(_));
                    ui.horizontal(|ui| {
                        if ui.selectable_label(!by_car, "time").on_hover_text("key times = seconds since the shot began").clicked() && by_car {
                            *drive = Drive::Time;
                            ch = true;
                        }
                        if ui.selectable_label(by_car, "car").on_hover_text("key times = metres of lap of a car (a dolly keeping pace)").clicked() && !by_car {
                            *drive = Drive::Car(no_nearest(fallback));
                            ch = true;
                        }
                        if let Drive::Car(t) = drive {
                            ch |= target_combo(ui, "cine-drive", t, false, ncars, names);
                        }
                    });
                    ui.end_row();
                    ui.label("aim");
                    ui.horizontal(|ui| {
                        for (n, a) in [("keys", PathAim::Keys), ("tangent", PathAim::Tangent), ("car", PathAim::Track { target: no_nearest(fallback), lag: NEW_LAG, lead: NEW_LEAD })] {
                            let on = std::mem::discriminant(aim) == std::mem::discriminant(&a);
                            if ui.selectable_label(on, n).clicked() && !on {
                                *aim = a;
                                ch = true;
                            }
                        }
                    });
                    ui.end_row();
                    if let PathAim::Track { target, lag, lead } = aim {
                        let mut a = Aim::Track { target: *target, lag: *lag, lead: *lead };
                        if aim_rows(ui, &mut a, ncars, names) {
                            if let Aim::Track { target: t, lag: l, lead: d } = a {
                                (*target, *lag, *lead) = (no_nearest(t), l, d);
                            }
                            ch = true;
                        }
                    }
                    ui.label("zoom");
                    ui.horizontal(|ui| ch |= zoom_row(ui, sel, zoom, true));
                    ui.end_row();
                    ui.label("clear view");
                    ch |= ui.checkbox(clear, "").changed();
                    ui.end_row();
                }
            }
        });
        if let Some(k) = new_kind {
            let eye = eye_now.unwrap_or(Vec3::ZERO);
            convert(&mut seq.cameras[sel], k, eye, fallback);
            ch = true;
        }
        // shakes
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.strong("shake");
            if ui.small_button("+").on_hover_text("add a shake").clicked() {
                seq.cameras[sel].shakes.push(Shake { kind: ShakeKind::Proximity, amp: 1.0, radius: 15.0 });
                ch = true;
            }
        });
        let mut drop = None;
        for (k, sh) in seq.cameras[sel].shakes.iter_mut().enumerate() {
            ui.horizontal(|ui| {
                ComboBox::from_id_salt(("cine-shake", k)).width(90.0).selected_text(SHAKE_KINDS.iter().find(|s| s.1 == sh.kind).map_or("?", |s| s.0)).show_ui(ui, |ui| {
                    for (n, kd) in SHAKE_KINDS {
                        ch |= ui.selectable_value(&mut sh.kind, kd, n).changed();
                    }
                });
                ch |= wheel_drag(ui, &mut sh.amp, Some(0.0..=20.0), |d| d.speed(0.05).max_decimals(2).suffix(" deg")).changed();
                if sh.kind != ShakeKind::Handheld {
                    ch |= wheel_drag(ui, &mut sh.radius, Some(0.5..=200.0), |d| d.speed(0.2).max_decimals(1).suffix(" m")).changed();
                }
                if ui.small_button(ic::TRASH).clicked() {
                    drop = Some(k);
                }
            });
        }
        if let Some(k) = drop {
            seq.cameras[sel].shakes.remove(k);
            ch = true;
        }
    });
    if ch {
        let eye = eye_now.unwrap_or(Vec3::ZERO);
        if let Some(seq) = ed.seq.as_mut() {
            tidy(seq, eye);
        }
        ed.edited();
    }
    ed.reqs.extend(reqs);
    if remove {
        ed.remove_selected();
    }
}

/// The target / lag / lead rows of a tracking aim (grid rows). True when changed.
fn aim_rows(ui: &mut Ui, aim: &mut Aim, ncars: usize, names: &[String]) -> bool {
    let mut ch = false;
    match aim {
        Aim::Track { target, lag, lead } => {
            ui.label("target");
            ch |= target_combo(ui, "cine-at", target, true, ncars, names);
            ui.end_row();
            ui.label("lag / lead");
            ui.horizontal(|ui| {
                ch |= wheel_drag(ui, lag, Some(0.0..=5.0), |d| d.speed(0.01).max_decimals(2).suffix(" s")).on_hover_text("the cameraman's delay").changed();
                ch |= wheel_drag(ui, lead, Some(0.0..=5.0), |d| d.speed(0.01).max_decimals(2).suffix(" s")).on_hover_text("aim ahead of the car").changed();
            });
            ui.end_row();
        }
        Aim::At(a) => {
            ui.label("at");
            ch |= ui.horizontal(|ui| vec3_row(ui, a, 0.05)).inner;
            ui.end_row();
        }
    }
    ch
}

fn keys_section(ui: &mut Ui, ed: &mut CineEdit) {
    let (mut ch, mut reqs) = (false, Vec::new());
    let Some(seq) = ed.seq.as_mut() else { return };
    let sel = ed.sel.min(seq.cameras.len() - 1);
    let c = &mut seq.cameras[sel];
    let path = matches!(c.kind, CamKind::Path { .. });
    let unit = if matches!(c.kind, CamKind::Path { drive: Drive::Car(_), .. }) { " m" } else { " s" };
    let title = format!("Keys of {}", c.name);
    section(ui, ic::SPLINE, &title, false, |ui| {
        ui.weak(if path { "Eye and Angles keys move the camera along a path; Fov keys zoom. Ctrl+Shift+K adds one at the free camera." } else { "Fov keys zoom this camera over its shot (seconds since the shot began). Ctrl+Shift+K adds one." });
        let eye_keys = c.keys.iter().filter(|k| k.eye.is_some()).count();
        let mut drop = None;
        for (k, key) in c.keys.iter_mut().enumerate() {
            ui.push_id(("cine-key", k), |ui| {
                ui.horizontal(|ui| {
                    ch |= wheel_drag(ui, &mut key.at, Some(0.0..=100_000.0), |d| d.speed(0.02).max_decimals(2).suffix(unit)).changed();
                    if ui.small_button(ic::CAMERA).on_hover_text("this key to the free camera").clicked() {
                        reqs.push(Req::KeyFromCamera(k));
                    }
                    ch |= ui.checkbox(&mut key.linear, "linear").changed();
                    let last_eye = path && key.eye.is_some() && eye_keys < 2;
                    if ui.add_enabled(!last_eye, egui::Button::new(ic::TRASH).small()).on_hover_text("remove the key").clicked() {
                        drop = Some(k);
                    }
                });
                if path {
                    ui.horizontal(|ui| {
                        let mut on = key.eye.is_some();
                        if ui.add_enabled(!(on && eye_keys < 2), egui::Checkbox::new(&mut on, "eye")).changed() {
                            key.eye = on.then_some(Vec3::ZERO);
                            ch = true;
                        }
                        if let Some(e) = key.eye.as_mut() {
                            ch |= vec3_row(ui, e, 0.05);
                        }
                    });
                    ui.horizontal(|ui| {
                        let mut on = key.angles.is_some();
                        if ui.checkbox(&mut on, "angles").changed() {
                            key.angles = on.then_some(Vec3::ZERO);
                            ch = true;
                        }
                        if let Some(a) = key.angles.as_mut() {
                            ch |= vec3_row(ui, a, 0.5);
                        }
                    });
                }
                ui.horizontal(|ui| {
                    let mut on = key.fov.is_some();
                    if ui.checkbox(&mut on, "fov").changed() {
                        key.fov = on.then_some(FOV_X);
                        ch = true;
                    }
                    if let Some(f) = key.fov.as_mut() {
                        ch |= num(ui, f, FOV_RANGE.0, FOV_RANGE.1, 0.2);
                    }
                    ui.label("tcb");
                    for v in &mut key.tcb {
                        ch |= num(ui, v, -1.0, 1.0, 0.01);
                    }
                });
                ui.separator();
            });
        }
        if let Some(k) = drop {
            c.keys.remove(k);
            ch = true;
        }
        // a key with no channel left is dropped
        let before = c.keys.len();
        c.keys.retain(|k| k.eye.is_some() || k.angles.is_some() || k.fov.is_some());
        ch |= c.keys.len() != before;
    });
    if ch {
        if let Some(seq) = ed.seq.as_mut() {
            tidy(seq, Vec3::ZERO);
        }
        ed.edited();
    }
    ed.reqs.extend(reqs);
}

fn shots_section(ui: &mut Ui, ed: &mut CineEdit) {
    let (mut ch, mut cut, mut jump) = (false, false, None);
    let t = ed.t;
    let Some(seq) = ed.seq.as_mut() else { return };
    let air = shot_at(seq, t);
    section(ui, ic::CLOCK, "Shots", false, |ui| {
        ui.horizontal(|ui| {
            cut = ui.button(format!("{}  cut to {} at {:.2} s", ic::VIDEO, seq.cameras[ed.sel.min(seq.cameras.len() - 1)].name, t)).on_hover_text("a shot of the selected camera at the clock").clicked();
        });
        let names: Vec<String> = seq.cameras.iter().map(|c| c.name.clone()).collect();
        let mut drop = None;
        let n = seq.shots.len();
        egui::Grid::new("cine-shots").num_columns(7).spacing([6.0, 3.0]).show(ui, |ui| {
            for h in ["", "time", "camera", "speed", "fade in", "fade out", ""] {
                ui.weak(h);
            }
            ui.end_row();
            for (i, s) in seq.shots.iter_mut().enumerate() {
                ui.push_id(("cine-shot", i), |ui| {
                    if ui.selectable_label(i == air, if i == air { ic::PLAY } else { " " }).on_hover_text("clock to this shot").clicked() {
                        jump = Some(s.t);
                    }
                });
                ch |= wheel_drag(ui, &mut s.t, Some(0.0..=1200.0), |d| d.speed(0.02).max_decimals(2)).changed();
                ComboBox::from_id_salt(("cine-shot-cam", i)).width(90.0).selected_text(names.get(s.cam).map_or("?", |n| n.as_str())).show_ui(ui, |ui| {
                    for (k, nm) in names.iter().enumerate() {
                        ch |= ui.selectable_value(&mut s.cam, k, nm).changed();
                    }
                });
                ch |= wheel_drag(ui, &mut s.speed, Some(SPEED_RANGE.0..=SPEED_RANGE.1), |d| d.speed(0.01).max_decimals(2).prefix("x ")).changed();
                ch |= wheel_drag(ui, &mut s.fade_in, Some(0.0..=10.0), |d| d.speed(0.02).max_decimals(2)).changed();
                ch |= wheel_drag(ui, &mut s.fade_out, Some(0.0..=10.0), |d| d.speed(0.02).max_decimals(2)).changed();
                if ui.add_enabled(n > 1, egui::Button::new(ic::TRASH).small()).clicked() {
                    drop = Some(i);
                }
                ui.end_row();
            }
        });
        if let Some(i) = drop {
            seq.shots.remove(i);
            ch = true;
        }
    });
    if cut {
        ed.reqs.push(Req::Cut);
    }
    if let Some(t) = jump {
        ed.seek(t);
    }
    if ch {
        if let Some(seq) = ed.seq.as_mut() {
            tidy(seq, Vec3::ZERO);
        }
        ed.edited();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn game_rot_inverts_from_rot() {
        let rot = motor_sim::tvcam::euler_to_rot(Vec3::new(10.0, 70.0, 5.0) * DEG);
        let cam = Camera::from_rot(Vec3::new(3.0, 4.0, 5.0), rot, 1.0, 1.0);
        assert!(game_rot(&cam).abs_diff_eq(rot, 1e-4), "{:?} vs {rot:?}", game_rot(&cam));
    }

    #[test]
    fn fov_round_trip() {
        for z in [0.5, 1.0, 2.0, 5.0] {
            assert!((cine::fov_zoom(fov_of(z)) - z).abs() < 1e-3);
        }
    }

    #[test]
    fn remove_keeps_shots_valid() {
        let (seq, _) = cine::parse("Camera a static Eye 0 0 0 At 1 0 0\nCamera b static Eye 0 0 0 At 1 0 0\nCamera c static Eye 0 0 0 At 1 0 0\nShot 0 a\nShot 2 b\nShot 4 c\n").unwrap();
        let mut ed = CineEdit::default();
        ed.set(seq, None);
        ed.sel = 1;
        ed.remove_selected();
        let s = ed.seq.as_ref().unwrap();
        assert_eq!(s.cameras.iter().map(|c| c.name.as_str()).collect::<Vec<_>>(), ["a", "c"]);
        assert_eq!(s.shots.iter().map(|s| (s.t, s.cam)).collect::<Vec<_>>(), [(0.0, 0), (4.0, 1)]);
        // the text round-trips
        let (s2, w) = cine::parse(&cine::to_text(s)).unwrap();
        assert!(w.is_empty(), "{w:?}");
        assert_eq!(&s2, s);
    }

    /// An editor with two static cameras at x 0 and 20, seen along +z from z -50: one metre = 10 px, centre 500, 500.
    fn picker() -> (CineEdit, impl Fn(Vec3) -> Option<(f32, f32)>) {
        let (seq, _) = cine::parse("Camera a static Eye 0 0 0 At 0 0 10\nCamera b static Eye 20 0 0 At 20 0 10\nShot 0 a\n").unwrap();
        let mut ed = CineEdit::default();
        ed.set(seq, Some("x.cine".into()));
        (ed, |p: Vec3| Some((500.0 + p.x * 10.0, 500.0 - p.y * 10.0)))
    }

    fn ray_at(x: f32, y: f32) -> (Vec3, Vec3) {
        (Vec3::new(x, y, -50.0), Vec3::Z)
    }

    #[test]
    fn click_picks_the_nearest_point() {
        let (mut ed, project) = picker();
        assert!(ed.press(&project, (703.0, 502.0), ray_at(20.3, -0.2), None));
        assert_eq!((ed.sel, ed.handle()), (1, Some(Handle::Eye)));
        // the aim point of camera a: z does not show in this projection, so it lies on a's eye; a's own point is first
        assert!(ed.press(&project, (500.0, 500.0), ray_at(0.0, 0.0), None));
        assert_eq!(ed.sel, 0);
        // nothing near
        assert!(!ed.press(&project, (500.0, 300.0), ray_at(0.0, 20.0), None));
        assert_eq!(ed.render_path(), Some(PathBuf::from("x.mp4")));
    }

    #[test]
    fn arrow_drag_moves_the_eye_along_its_axis() {
        let (mut ed, project) = picker();
        // the X arrow of camera a's gizmo (length 0.16 x 50 m), grabbed 5 m along
        assert!(ed.press(&project, (550.0, 500.0), ray_at(5.0, 0.0), None));
        assert!(ed.dragging());
        ed.mouse_move(&project, (580.0, 500.0), ray_at(8.0004, 3.0), None);
        ed.release(None);
        assert!(!ed.dragging() && ed.dirty);
        let CamKind::Static { eye, .. } = ed.seq.as_ref().unwrap().cameras[0].kind else { panic!() };
        assert_eq!(eye, Vec3::new(3.0, 0.0, 0.0));
    }

    #[test]
    fn names_stay_unique_and_spaceless() {
        let (seq, _) = cine::parse("Camera a static Eye 0 0 0 At 1 0 0\nCamera b static Eye 0 0 0 At 1 0 0\n").unwrap();
        assert_eq!(unique_name(&seq, "a", Some(1)), "a2");
        assert_eq!(unique_name(&seq, "my cam", None), "my_cam");
        assert_eq!(unique_name(&seq, "a", Some(0)), "a");
    }
}
