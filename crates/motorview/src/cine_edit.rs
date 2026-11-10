//! DEBUG motorview's Cine editor (#152, re-notes/20-cine.md): a `.cine` sequence edited in the viewer. Opening one
//! makes its all-AI race the viewer's race (`cine_render::race`, the render's own start); while it is open the
//! sequence's clock drives the race (`frame`: a seek on the race timeline, else steps of 1/fps past its end), the
//! `Director` runs every camera, and the view can look through the selected camera or the one on air. Cameras and
//! path keys are added where the free camera stands; every field is edited on the Properties page (`page`).
//! The ribbon's Cine tab and the shortcuts push `Req`s the viewer runs after the frame (they need the free camera
//! or a race reload). Saving writes `cine::to_text` (comments of a hand-written file are not kept).
//! In the world (`gizmos`) every camera is a frustum with its name, a path camera its curve with a cross per key, a
//! tracking camera a line to its car; a click picks a camera, key or aim point (`press`) and the move arrows
//! (`fill_gizmo`, global axes) drag it; in Rotate the rings sit on the camera's own axes (`Rig`, `Turn`) and turn its aim point, mounted
//! angles or a key's angles. Render runs `cine_render::render` on a worker thread (`Job`).

use crate::cine::{self, audio_rate_at, PITCHES, path_points, Auto, Follow, FocusOn, FOLLOWS, APERTURE_RANGE, FOCUS_MIN, on_air, race_secs, shot_at, since_shot, video_secs, Aim, CamKey, CamKind, CamMode, CineCam, Director, Lens, Drive, PathAim, RaceDef, Sequence, Shake, ShakeKind, Shot, Target, Zoom, DIFFICULTIES, FOV_RANGE, LETTERBOX_RANGE, POSE_MODES, SHAKE_KINDS, SPEED_RANGE};
use crate::cine_render::{self, Codec, Out, RenderOpts, Watch, CANCELLED};
use crate::debug::{self, Gizmos};
use crate::drive::{cam_origin, Session};
use crate::fill_gizmo::{self, Mode as GizmoMode, AXES};
use crate::glass_ui;
use crate::gui::wheel_drag;
use crate::icons as ic;
use crate::inspect::Passes;
use crate::race_audio::{CineEar, Ear, RaceAudio, MASTER};
use crate::render::{Camera, RenderMode, View, FOV_X};
use crate::tune::section;
use crate::world_edit::PICK_PX;
use egui::{ComboBox, Ui};
use glam::{Mat3, Quat, Vec3};
use motor_sim::Race;
use motor_sim::tvcam::{euler_to_rot, rot_to_euler};
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
/// Defaults of a freshly ticked Aperture (f-number) and a Focus distance (metres) (#421).
const NEW_APERTURE: f32 = 2.8;
const NEW_FOCUS: f32 = 20.0;
const MOUNT_OFFSET: Vec3 = Vec3::new(0.0, 1.6, -0.5);
/// The status bar's hint while the editor has the mouse.
pub const HINT: &str = "*CINE* [LMB] pick camera, key or aim point [Drag] move arrow / rotate ring [Ctrl+W / Ctrl+T] move / rotate [Ctrl+Z / Ctrl+Y] undo / redo [RMB] look [Ctrl+K] camera here [Ctrl+Shift+K] key here [Ctrl+P] preview [Ctrl+S] save";
/// Undo steps kept (the oldest is dropped past this), as the world editor's.
const UNDO_LIMIT: usize = 100;
/// Gizmos: points sampled along a path, the frustum depth and a key cross as shares of the eye distance (constant
/// size on screen) with their floors, metres.
const PATH_POINTS: usize = 96;
const FRUSTUM_K: f32 = 0.08;
const FRUSTUM_MIN: f32 = 0.8;
const CROSS_K: f32 = 0.012;
const CROSS_MIN: f32 = 0.15;
/// No move gizmo nearer the eye than this (previewing the camera it sits on), metres.
const GIZMO_NEAR: f32 = 1.0;

/// A dissolve the on-air preview draws (#438): the outgoing camera `from` posed this frame (its slot, whether it hides it, its
/// lens), the incoming camera's weight `k` and the outgoing picture's graphics mode (None = the viewer's own).
pub struct PreviewDissolve {
    pub from: usize,
    pub k: f32,
    pub cam: Camera,
    pub slot: usize,
    pub hide: bool,
    pub dof: Option<crate::render::Dof>,
    pub mode: Option<RenderMode>,
}

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
    pub const ALL: [Preview; 3] = [Preview::Off, Preview::OnAir, Preview::Selected];

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
    /// leave the editor (asks first with unsaved edits, #438)
    Close,
    /// Open…: the system file dialog; the picked file comes back as `Open`
    Pick,
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

/// What a rotate drag turns, from its state at the press (the turn is always applied to this, not to the last frame's).
#[derive(Clone, Copy, Debug)]
enum Turn {
    /// a static camera's fixed aim point about its eye; `rot` = the camera's frame looking at it (game space)
    Aim { eye: Vec3, aim: Vec3, rot: Mat3 },
    /// a mounted camera's or a key's angles; `local` = their orientation (relative to the car for a mounted camera),
    /// `base` = the angles in degrees
    Angles { local: Mat3, base: Vec3 },
}

/// A gizmo drag: the handle, the axis, where the gizmo sits and the press's parameter (Move: along the axis, Rotate:
/// the angle in the ring's plane), the world axes of the gizmo and, rotating, what turns.
#[derive(Clone, Copy, Debug)]
struct Drag {
    handle: Handle,
    axis: usize,
    base: Vec3,
    from: f32,
    axes: [Vec3; 3],
    turn: Option<Turn>,
}

/// The gizmo on the selected handle: where, along which world axes, which axes work and (Rotate) what they turn.
#[derive(Clone, Copy, Debug)]
struct Rig {
    handle: Handle,
    pivot: Vec3,
    axes: [Vec3; 3],
    en: [bool; 3],
    turn: Option<Turn>,
}

/// A render on a worker thread (`cine_render::render`, its own GPU device).
pub struct Job {
    pub out: PathBuf,
    pub watch: Watch,
    thread: Option<JoinHandle<Result<(), String>>>,
    started: Instant,
}

/// #349 REMASTER: the live preview's sound, a `RaceAudio` on a real output heard by the previewed camera (as `cine_render`'s `Sound`, but live).
struct Listen {
    /// keeps the output stream open
    audio: motor_audio::Audio,
    race: RaceAudio,
    /// the camera heard last (a change restarts the Doppler history)
    heard: Option<usize>,
    /// the clock (video seconds) of the last frame heard
    at: Option<f32>,
}

/// The clock `t` follows the last heard frame `at` smoothly (a forward step of at most `JUMP`): the voices play on.
/// Paused (`t == at`), backwards or a jump: they start again from the race's state at `t`.
fn continuous(at: Option<f32>, t: f32) -> bool {
    at.is_some_and(|a| t > a && t - a <= JUMP)
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
    /// Move arrows or Rotate rings (Ctrl+W / Ctrl+T)
    pub gizmo: GizmoMode,
    /// the gizmo axis under the mouse or held
    hot: Option<usize>,
    drag: Option<Drag>,
    pub job: Option<Job>,
    /// #349 the live preview's sound is muted (the ribbon's Sound toggle; not saved in the .cine)
    pub muted: bool,
    sound: Option<Listen>,
    /// undo / redo (#348): snapshots of the sequence; `base` = the sequence as of the last step (`settle` compares to it)
    undo: Vec<Sequence>,
    redo: Vec<Sequence>,
    base: Option<Sequence>,
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
            gizmo: GizmoMode::Move,
            hot: None,
            drag: None,
            job: None,
            muted: false,
            sound: None,
            undo: Vec::new(),
            redo: Vec::new(),
            base: None,
        }
    }
}

/// The folder of the user's sequences, `<config>/Cine` beside settings.cfg, `Ghosts` and `Replays` (`ghost::config_dir`);
/// the install's `remaster/cine` when there is no config folder.
pub fn dir(root: &Path) -> PathBuf {
    dir_in(crate::ghost::config_dir(), root)
}

fn dir_in(config: Option<PathBuf>, root: &Path) -> PathBuf {
    config.map_or_else(|| legacy_dir(root), |c| c.join("Cine"))
}

/// The old home of the sequences, `<install>/remaster/cine`: still read (a bare name, the list), never written.
pub fn legacy_dir(root: &Path) -> PathBuf {
    root.join("remaster/cine")
}

/// The `.cine` files in `d`.
fn cines_in(d: &Path) -> Vec<PathBuf> {
    std::fs::read_dir(d).map(|d| d.filter_map(|e| e.ok().map(|e| e.path())).filter(|p| p.extension().is_some_and(|x| x.eq_ignore_ascii_case("cine"))).collect()).unwrap_or_default()
}

/// The `.cine` files of the sequences folder, then those only the install's old folder has, by name.
pub fn list(root: &Path) -> Vec<PathBuf> {
    list_in(&dir(root), &legacy_dir(root))
}

fn list_in(d: &Path, legacy: &Path) -> Vec<PathBuf> {
    let mut v = cines_in(d);
    if legacy != d {
        let old: Vec<PathBuf> = cines_in(legacy).into_iter().filter(|p| !v.iter().any(|q| q.file_name() == p.file_name())).collect();
        v.extend(old);
    }
    v.sort_by(|a, b| a.file_name().cmp(&b.file_name()));
    v
}

/// A bare name is looked for in the sequences folder, then in the install's old one (as `--cine`);
/// a name that exists in neither is a new file of the sequences folder.
pub fn resolve(root: &Path, p: &Path) -> PathBuf {
    resolve_in(&dir(root), &legacy_dir(root), p)
}

fn resolve_in(d: &Path, legacy: &Path, p: &Path) -> PathBuf {
    if p.exists() || p.components().count() > 1 {
        p.to_path_buf()
    } else if !d.join(p).exists() && legacy.join(p).exists() {
        legacy.join(p)
    } else {
        d.join(p)
    }
}

/// Where a sequence read from `p` is saved: a file of the install's old folder goes to the same name in the
/// sequences folder (the install is never written), any other path stays.
pub fn home(root: &Path, p: &Path) -> PathBuf {
    let (d, legacy) = (dir(root), legacy_dir(root));
    match (p.parent(), p.file_name()) {
        (Some(par), Some(n)) if d != legacy && par == legacy => d.join(n),
        _ => p.to_path_buf(),
    }
}

/// Read and parse a `.cine`.
pub fn open(path: &Path) -> Result<(Sequence, Vec<String>), String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    cine::parse(&text).map_err(|e| format!("{}: {e}", path.display()))
}

/// A free name `trackNN[_k].cine` in the sequences folder (and not taken in the install's old one).
pub fn new_path(root: &Path, track: u32) -> PathBuf {
    let (d, legacy) = (dir(root), legacy_dir(root));
    let name = |k: u32| if k == 1 { format!("track{track:02}.cine") } else { format!("track{track:02}_{k}.cine") };
    (1..).map(name).find(|n| !d.join(n).exists() && !legacy.join(n).exists()).map_or_else(|| d.join("new.cine"), |n| d.join(n))
}

/// A new sequence on `track` (the default all-AI race), its one camera where `cam` is, following the leader.
pub fn new_seq(track: u32, cam: &Camera) -> Sequence {
    let mut seq = Sequence { race: RaceDef { track, ..RaceDef::default() }, audio: Default::default(), size: (1920, 1080), fps: 60, letterbox: None, end: None, cameras: Vec::new(), shots: vec![Shot::new(0.0, 0)] };
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
    (2.0 * ((FOV_X.get() * 0.5 * DEG).tan() / zoom.max(1e-3)).atan() / DEG).clamp(FOV_RANGE.0, FOV_RANGE.1)
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
const KINDS: [(&str, Option<CamMode>); 9] =
    [("static", None), ("mounted", None), ("path", None), ("orbit", None), ("chase", Some(CamMode::Chase)), ("far", Some(CamMode::FarChase)), ("incar", Some(CamMode::InCar)), ("bumper", Some(CamMode::Bumper)), ("tv", Some(CamMode::Tv))];

fn kind_name(k: &CamKind) -> &'static str {
    match k {
        CamKind::Static { .. } => "static",
        CamKind::Mounted { .. } => "mounted",
        CamKind::Path { .. } => "path",
        CamKind::Orbit { .. } => "orbit",
        CamKind::Pose { mode, .. } => POSE_MODES.iter().find(|m| m.1 == *mode).map_or("chase", |m| m.0),
    }
}

/// The car a camera follows or rides on, if any.
fn target_of(k: &CamKind) -> Option<Target> {
    match k {
        CamKind::Static { aim: Aim::Track { target, .. }, .. } | CamKind::Path { aim: PathAim::Track { target, .. }, .. } => Some(*target),
        CamKind::Mounted { target, .. } | CamKind::Pose { target, .. } | CamKind::Orbit { target, .. } => Some(*target),
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
        Target::Auto => "auto (the auto shot's car)".into(),
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
        opts.push(Target::Auto);
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
    CineCam { name: next_name(seq, "cam"), kind, keys: Vec::new(), shakes: Vec::new(), lens: Lens::default() }
}

/// Keep the sequence usable by the director: a camera, shots sorted with valid cameras (at least one), keys sorted,
/// every path camera with an Eye key (`eye`: where a missing one goes).
fn tidy(seq: &mut Sequence, eye: Vec3) {
    let n = seq.cameras.len();
    seq.shots.retain(|s| s.cam < n);
    // an auto shot's camera is its pool's first (the one before the director has picked)
    let firsts: Vec<Option<usize>> = seq.shots.iter().map(|s| s.auto.as_ref().and_then(|a| cine::auto_pool(seq, a).first().copied())).collect();
    for (s, f) in seq.shots.iter_mut().zip(firsts) {
        if let Some(f) = f {
            s.cam = f;
        }
    }
    seq.shots.sort_by(|a, b| a.t.total_cmp(&b.t));
    if seq.shots.is_empty() {
        seq.shots.push(Shot::new(0.0, 0));
    }
    for c in &mut seq.cameras {
        c.keys.sort_by(|a, b| a.at.total_cmp(&b.at));
        if matches!(c.kind, CamKind::Path { .. }) && !c.keys.iter().any(|k| k.eye.is_some()) {
            c.keys.insert(0, CamKey { at: 0.0, eye: Some(eye), angles: None, fov: None, focus: None, tcb: [0.0; 3], linear: false });
        }
    }
}

/// Bring the race to race second `to` of its timeline: a seek inside the recorded run, else steps of `step` past its
/// end, at most `budget` seconds of them. True when it got there.
pub fn sync(sess: &mut Session, to: f32, step: f32, budget: f32) -> bool {
    if sess.replay {
        // #423: a recording plays its own frames to the clock, held at its end
        sess.play_to(to as f64);
        return true;
    }
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
        self.base = Some(seq.clone());
        (self.seq, self.path, self.dirty, self.sel, self.t, self.reset, self.loaded) = (Some(seq), path, false, 0, 0.0, true, None);
        (self.handle, self.hot, self.drag) = (None, None, None);
        (self.undo, self.redo) = (Vec::new(), Vec::new());
    }

    /// The race was (re)loaded: the clock back to 0, the cameras from rest.
    pub fn restart(&mut self, race: RaceDef) {
        (self.loaded, self.t, self.dir_t, self.reset, self.behind, self.saved) = (Some(race), 0.0, 0.0, true, false, None);
        // the new race has other cars: fresh voices
        if let Some(l) = self.sound.as_mut() {
            l.race.stop();
            (l.heard, l.at) = (None, None);
        }
    }

    /// #349 REMASTER: the race heard from the previewed camera, once per viewer frame after `frame` (`dt` = video seconds
    /// the clock moved). Quiet (voices kept) while paused, running ahead, muted, the sequence's `audio` off or no camera
    /// is previewed; after a seek the voices start again [design choice]. `device` = the saved output id (None: default).
    pub fn listen(&mut self, root: &Path, device: Option<&str>, sess: &Session, dt: f32) {
        let (Some(seq), Some(air)) = (self.seq.as_ref(), self.on_air()) else { return self.hush() };
        let view = self.preview_cam();
        let i = if self.preview == Preview::Selected { self.sel } else { air };
        let Some((cam, focus, in_car)) = view.filter(|_| seq.audio.on && !self.muted && dt > 0.0 && !self.behind) else {
            if let Some(l) = &self.sound {
                l.race.silence();
            }
            return;
        };
        let l = self.sound.get_or_insert_with(|| {
            let audio = motor_audio::Audio::open_on(device);
            audio.with(|m| m.master = MASTER.get());
            let race = RaceAudio::new(root, audio.share());
            Listen { audio, race, heard: None, at: None }
        });
        if !l.race.active() || !continuous(l.at, self.t) {
            l.race.stop();
            l.race.start(seq.race.track, &sess.race);
            l.heard = None;
        }
        let cut = l.heard.is_some_and(|h| h != i);
        let ear = Ear { cam, focus, in_car, cine: Some(CineEar { doppler: seq.audio.doppler, cut }) };
        // #426: a `Pitch tape` shot plays the sound at the shot's Speed, as the render does
        let rate = audio_rate_at(seq, self.t);
        l.audio.with(|m| m.rate = rate);
        l.race.update(sess, &ear, dt * rate);
        (l.heard, l.at) = (Some(i), Some(self.t));
    }

    /// The editor is not previewing: no output stream (it opens again when the sound is wanted).
    pub fn hush(&mut self) {
        self.sound = None;
    }

    /// The open sequence's file name while it has edits that are not saved (the requester's question, #438); a
    /// sequence not on disk yet has the name it will be saved under.
    pub fn unsaved_name(&self) -> Option<String> {
        if !(self.active() && self.dirty) {
            return None;
        }
        Some(self.path.as_ref().and_then(|p| p.file_name()).map_or_else(|| "(unsaved)".to_string(), |n| n.to_string_lossy().into_owned()))
    }

    /// Leave the editor (a render keeps running).
    pub fn close(&mut self) {
        (self.seq, self.path, self.dirty, self.dir, self.loaded, self.preview) = (None, None, false, None, None, Preview::Off);
        self.sound = None;
        (self.handle, self.hot, self.drag) = (None, None, None);
        (self.undo, self.redo, self.base) = (Vec::new(), Vec::new(), None);
    }

    /// The sequence was edited: unsaved, and the director starts afresh.
    /// The race as it was loaded (#423: a recording's header sets the track, cars and laps): the sequence and its
    /// undo base take it without counting as an edit.
    pub fn set_race(&mut self, r: RaceDef) {
        if let Some(seq) = self.seq.as_mut() {
            self.cars_text = r.cars.iter().map(u32::to_string).collect::<Vec<_>>().join(" ");
            if let Some(b) = self.base.as_mut() {
                b.race = r.clone();
            }
            seq.race = r;
        }
    }

    pub fn edited(&mut self) {
        self.dirty = true;
        self.reset = true;
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    /// Close the open undo step (#348): when the sequence differs from the last step's, push that one. Every edit
    /// source (requests, the page, gizmo drags) just changes the sequence; called once a frame with `held` = the
    /// pointer is down, so a whole drag (gizmo or a field) is one step. A new edit drops the redo stack.
    pub fn settle(&mut self, held: bool) {
        if held || self.drag.is_some() {
            return;
        }
        let (Some(seq), Some(base)) = (self.seq.as_ref(), self.base.as_mut()) else { return };
        if seq == base {
            return;
        }
        self.undo.push(std::mem::replace(base, seq.clone()));
        if self.undo.len() > UNDO_LIMIT {
            self.undo.remove(0);
        }
        self.redo.clear();
    }

    /// Ctrl+Z: the sequence as it was before the last step.
    pub fn undo(&mut self) {
        self.step(true);
    }

    /// Ctrl+Y: the step undone last, again.
    pub fn redo(&mut self) {
        self.step(false);
    }

    fn step(&mut self, back: bool) {
        if self.drag.is_some() {
            return;
        }
        self.settle(false);
        let Some(seq) = self.seq.as_mut() else { return };
        let Some(to) = (if back { self.undo.pop() } else { self.redo.pop() }) else { return };
        let from = std::mem::replace(seq, to);
        if back { &mut self.redo } else { &mut self.undo }.push(from);
        self.base = Some(seq.clone());
        self.sel = self.sel.min(seq.cameras.len().saturating_sub(1));
        self.cars_text = seq.race.cars.iter().map(u32::to_string).collect::<Vec<_>>().join(" ");
        (self.hot, self.msg) = (None, if back { "undo" } else { "redo" }.into());
        self.edited();
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

    /// The camera on air at the clock (an auto shot: the director's pick).
    pub fn on_air(&self) -> Option<usize> {
        let seq = self.seq.as_ref()?;
        Some(match &self.dir {
            Some(d) if d.air() < seq.cameras.len() => d.air(),
            _ => on_air(seq, self.t),
        })
    }

    /// The letterbox aspect to draw over the viewport: the on-air preview of a sequence with `Output Letterbox`.
    pub fn preview_letterbox(&self) -> Option<f32> {
        self.seq.as_ref()?.letterbox.filter(|_| self.preview == Preview::OnAir)
    }

    /// The video's frame size while a camera is previewed (the preview fits the view to its aspect, #438).
    pub fn preview_size(&self) -> Option<(u32, u32)> {
        self.preview_index()?;
        self.seq.as_ref().map(|q| q.size)
    }

    /// The on-air preview's brightness from the shots' fades (#438; 1 = as drawn, 0 = black), as the render with the
    /// sequence's End as the video's end (without one the last shot does not fade out, as a render of an unknown length).
    pub fn preview_fade(&self) -> Option<f32> {
        let seq = self.seq.as_ref().filter(|_| self.preview == Preview::OnAir)?;
        Some(cine::fade_at(seq, self.t, seq.end))
    }

    /// The on-air preview's dissolve in progress (#438): the camera it comes from, posed this frame, as the render draws it.
    pub fn preview_dissolve(&self) -> Option<PreviewDissolve> {
        let seq = self.seq.as_ref().filter(|_| self.preview == Preview::OnAir)?;
        let dir = self.dir.as_ref()?;
        let (from, k) = dir.dissolve()?;
        let (cam, slot, hide) = dir.view(from)?;
        // the picture cut away from is drawn in the mode of the frame before the cut (#427, as `cine_render`)
        let start = dir.dissolve_start()?;
        let mode = seq.shots[shot_at(seq, (start - 1e-3).max(0.0))].mode;
        Some(PreviewDissolve { from, k, cam, slot, hide, dof: dir.dof(from), mode })
    }

    /// The previewed camera's index.
    fn preview_index(&self) -> Option<usize> {
        match self.preview {
            Preview::Off => None,
            Preview::Selected => Some(self.sel),
            Preview::OnAir => self.on_air(),
        }
    }

    /// The camera to look through and the slot it shows / hides, when previewing.
    pub fn preview_cam(&self) -> Option<(Camera, usize, bool)> {
        self.dir.as_ref()?.view(self.preview_index()?)
    }

    /// The graphics mode of the shot under the clock while a camera is previewed (#427), None = the viewer's own (F9).
    pub fn preview_mode(&self) -> Option<RenderMode> {
        self.preview_index()?;
        self.seq.as_ref().and_then(|q| q.shots[shot_at(q, self.t)].mode)
    }

    /// REMASTER depth of field of the previewed camera (#421): the lens it has this frame, None without an aperture.
    pub fn preview_dof(&self) -> Option<crate::render::Dof> {
        self.dir.as_ref()?.dof(self.preview_index()?)
    }

    /// Save to the sequence's own file and say so in `msg` (the shelf's Save, Ctrl+S).
    pub fn save_msg(&mut self) {
        self.msg = match self.save(None) {
            Ok(p) => format!("saved {}", p.display()),
            Err(e) => e,
        };
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
                // the car the new camera rides on (New cameras follow; nearest = the car nearest the free camera), the same
                // one its offset is measured from
                let slot = cine::resolve(&ss.race, self.target, Some(here));
                let car = &ss.race.racers[slot].car;
                let target = if self.target == Target::Nearest { Target::Car(slot) } else { self.target };
                // where the free camera is relative to that car (else the roof)
                let local = car.rot.transpose() * (here - cam_origin(car));
                let (offset, angles) = if local.length() < 15.0 { (round3(local), angles_of(car.rot.transpose() * rot)) } else { (MOUNT_OFFSET, Vec3::new(5.0, 0.0, 0.0)) };
                let name = next_name(seq, "mount");
                seq.cameras.push(CineCam { name, kind: CamKind::Mounted { target, offset, angles, zoom: 1.0, hide: false }, keys: Vec::new(), shakes: Vec::new(), lens: Lens::default() });
                self.sel = seq.cameras.len() - 1;
            }
            Req::AddPath => {
                let key = CamKey { at: 0.0, eye: Some(here), angles: Some(angles_of(rot)), fov: None, focus: None, tcb: [0.0; 3], linear: false };
                let name = next_name(seq, "path");
                seq.cameras.push(CineCam { name, kind: CamKind::Path { drive: Drive::Time, aim: PathAim::Keys, zoom: Zoom::Fixed(1.0), clear: false }, keys: vec![key], shakes: Vec::new(), lens: Lens::default() });
                self.sel = seq.cameras.len() - 1;
            }
            Req::AddKey => {
                let sel = self.sel.min(seq.cameras.len() - 1);
                let at = match &seq.cameras[sel].kind {
                    CamKind::Path { drive: Drive::Car(t), .. } => {
                        let Some(ss) = sess else { return self.msg = "no race loaded".into() };
                        ss.race.racers[cine::resolve(&ss.race, *t, None)].progress.s
                    }
                    _ => since_shot(seq, sel, self.t),
                };
                let at = (at * 100.0).round() / 100.0;
                let c = &mut seq.cameras[sel];
                let path = matches!(c.kind, CamKind::Path { .. });
                let fov = (cam.zoom != 1.0 || c.keys.iter().any(|k| k.fov.is_some())).then(|| (fov_of(cam.zoom) * 10.0).round() / 10.0);
                // #438: a Fov key overrides Zoom auto, and an aperture camera is keyed to rack its focus: such a camera gets
                // a Focus key (the focus it has at `at`) unless the free camera is zoomed or it has Fov keys already
                let auto = matches!(c.kind, CamKind::Static { zoom: Zoom::Auto(_), .. } | CamKind::Orbit { zoom: Zoom::Auto(_), .. });
                let focus_key = !path && fov.is_none() && (auto || c.lens.aperture.is_some());
                let focus = focus_key.then(|| c.keys.iter().filter(|k| k.focus.is_some() && k.at <= at).max_by(|a, b| a.at.total_cmp(&b.at)).and_then(|k| k.focus).unwrap_or(c.lens.focus));
                let key = CamKey { at, eye: path.then_some(here), angles: path.then(|| angles_of(rot)), fov: fov.or((!path && !focus_key).then_some(FOV_X.get())), focus, tcb: [0.0; 3], linear: false };
                // a key at the same time is replaced
                match c.keys.iter().position(|k| (k.at - at).abs() < 1e-3) {
                    Some(i) => c.keys[i] = key,
                    None => c.keys.push(key),
                }
                if focus_key {
                    self.msg = format!("{}: a Focus key at {at} s (a Fov key would override the zoom)", c.name);
                } else if !path {
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
                        let car = &ss.race.racers[cine::resolve(&ss.race, *target, None)].car;
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
            Req::New | Req::Open(_) | Req::Close | Req::Pick | Req::ApplyRace | Req::Render => return,
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
        seq.shots.retain(|s| s.auto.is_some() || s.cam != i);
        for s in &mut seq.shots {
            if let Some(a) = s.auto.as_mut() {
                a.pool.retain(|&c| c != i);
                for c in &mut a.pool {
                    if *c > i {
                        *c -= 1;
                    }
                }
            }
            if s.cam > i || (s.cam == i && s.auto.is_some()) {
                s.cam = s.cam.saturating_sub(1);
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

/// A camera frame (columns right, up, forward) looking from `eye` at `aim`, no roll (`tvcam::look_at`); None when they coincide.
fn look_at(eye: Vec3, aim: Vec3) -> Option<Mat3> {
    let f = (aim - eye).try_normalize()?;
    let r = Vec3::new(f.z, 0.0, -f.x).try_normalize().unwrap_or(Vec3::X);
    Some(Mat3::from_cols(r, f.cross(r), f))
}

/// `a` degrees wrapped to within 180 of `base` (a drag across the seam keeps its turn going the short way).
fn near_deg(a: f32, base: f32) -> f32 {
    base + (a - base + 180.0).rem_euclid(360.0) - 180.0
}

/// Turn the selected camera's aim point or angles by `delta` radians about axis `axis` of the frame `turn` was taken
/// in (the press's state, so the whole drag is one turn).
fn apply_turn(seq: &mut Sequence, sel: usize, handle: Handle, turn: Turn, axis: usize, delta: f32) {
    let Some(c) = seq.cameras.get_mut(sel) else { return };
    match turn {
        Turn::Aim { eye, aim, rot } => {
            if let CamKind::Static { aim: Aim::At(a), .. } = &mut c.kind {
                *a = eye + Quat::from_axis_angle(rot.col(axis), delta) * (aim - eye);
            }
        }
        Turn::Angles { local, base } => {
            let now = angles_of(Mat3::from_axis_angle(local.col(axis), delta) * local);
            let angles = Vec3::new(now.x, near_deg(now.y, base.y), near_deg(now.z, base.z));
            match (&mut c.kind, handle) {
                (CamKind::Mounted { angles: a, .. }, Handle::Eye) => *a = angles,
                (CamKind::Path { .. }, Handle::Key(k)) => {
                    if let Some(a) = c.keys.get_mut(k).map(|k| &mut k.angles) {
                        *a = Some(angles);
                    }
                }
                _ => {}
            }
        }
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
            CamKind::Pose { .. } | CamKind::Orbit { .. } => None,
        })
    }

    /// The move gizmo: its handle and where it is.
    fn gizmo_at(&self, race: Option<&Race>) -> Option<(Handle, Vec3)> {
        let h = self.handle()?;
        Some((h, handle_pos(self.seq.as_ref()?, self.sel, h, race)?))
    }

    /// The gizmo's mode: Move or Rotate (a world-editor mode set by mistake is Move).
    fn mode(&self) -> GizmoMode {
        if self.gizmo == GizmoMode::Rotate {
            GizmoMode::Rotate
        } else {
            GizmoMode::Move
        }
    }

    /// The gizmo of the selected handle. Move: arrows on the global axes. Rotate: rings on the camera's own axes at its
    /// eye; a static camera with an aim point turns that point (pitch, heading; it has no bank), a mounted camera its
    /// angles, a path key with Angles those. Any other camera, or a key without Angles, shows grey rings and turns nothing.
    fn rig(&self, race: Option<&Race>) -> Option<Rig> {
        let (handle, at) = self.gizmo_at(race)?;
        if self.mode() == GizmoMode::Move {
            return Some(Rig { handle, pivot: at, axes: AXES, en: [true; 3], turn: None });
        }
        let seq = self.seq.as_ref()?;
        let c = seq.cameras.get(self.sel)?;
        let cols = |m: Mat3| [m.x_axis, m.y_axis, m.z_axis];
        let grey = || {
            let axes = self.dir.as_ref().and_then(|d| d.view(self.sel)).map_or(AXES, |(cam, ..)| cols(game_rot(&cam)));
            Rig { handle, pivot: at, axes, en: [false; 3], turn: None }
        };
        Some(match (&c.kind, handle) {
            (CamKind::Static { eye, aim: Aim::At(a), .. }, _) => match look_at(*eye, *a) {
                Some(rot) => Rig { handle, pivot: *eye, axes: cols(rot), en: [true, true, false], turn: Some(Turn::Aim { eye: *eye, aim: *a, rot }) },
                None => grey(),
            },
            (CamKind::Mounted { target, angles, .. }, Handle::Eye) => {
                let car = &race?.racers[cine::resolve(race?, *target, None)].car;
                let local = euler_to_rot(*angles * DEG);
                Rig { handle, pivot: at, axes: cols(car.rot * local), en: [true; 3], turn: Some(Turn::Angles { local, base: *angles }) }
            }
            (CamKind::Path { .. }, Handle::Key(k)) => match c.keys.get(k).and_then(|key| key.angles) {
                Some(angles) => {
                    let local = euler_to_rot(angles * DEG);
                    Rig { handle, pivot: at, axes: cols(local), en: [true; 3], turn: Some(Turn::Angles { local, base: angles }) }
                }
                None => grey(),
            },
            _ => grey(),
        })
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
        if let Some(r) = self.rig(race).filter(|r| r.pivot.distance(eye) > GIZMO_NEAR) {
            fill_gizmo::draw(g, self.mode(), r.pivot, r.en, r.axes, self.hot, eye);
        }
    }

    /// LMB in the world: grab an arrow of the move gizmo, else select the camera, key or aim point within `PICK_PX`
    /// of the cursor. `project` = game space to window pixels, `ray` = the mouse ray (game space). True when taken.
    pub fn press(&mut self, project: &dyn Fn(Vec3) -> Option<(f32, f32)>, cursor: (f32, f32), ray: (Vec3, Vec3), race: Option<&Race>) -> bool {
        if self.preview == Preview::OnAir {
            return false;
        }
        if let Some(r) = self.rig(race).filter(|r| r.pivot.distance(ray.0) > GIZMO_NEAR) {
            let axis = fill_gizmo::pick(self.mode(), r.pivot, r.en, r.axes, ray.0, project, cursor);
            let start = |a: usize| if self.mode() == GizmoMode::Rotate { fill_gizmo::plane_angle(r.pivot, r.axes, a, ray) } else { fill_gizmo::axis_param(r.pivot, r.axes[a], ray) };
            if let Some((axis, from)) = axis.and_then(|a| Some((a, start(a)?))) {
                (self.drag, self.hot) = (Some(Drag { handle: r.handle, axis, base: r.pivot, from, axes: r.axes, turn: r.turn }), Some(axis));
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
            if let Some(turn) = d.turn {
                let Some(a) = fill_gizmo::plane_angle(d.base, d.axes, d.axis, ray) else { return };
                if let Some(seq) = self.seq.as_mut() {
                    apply_turn(seq, self.sel, d.handle, turn, d.axis, fill_gizmo::angle_diff(d.from, a));
                    self.edited();
                }
                return;
            }
            let Some(t) = fill_gizmo::axis_param(d.base, AXES[d.axis], ray) else { return };
            if let Some(seq) = self.seq.as_mut() {
                set_handle(seq, self.sel, d.handle, d.base + AXES[d.axis] * (t - d.from), race);
                self.edited();
            }
            return;
        }
        self.hot = if self.preview == Preview::OnAir { None } else { self.rig(race).and_then(|r| fill_gizmo::pick(self.mode(), r.pivot, r.en, r.axes, ray.0, project, cursor)) };
    }

    /// LMB up: a drag ends (its point rounded to millimetres, as the file keeps it).
    pub fn release(&mut self, race: Option<&Race>) {
        let Some(d) = self.drag.take() else { return };
        self.hot = None;
        let Some(seq) = self.seq.as_mut() else { return };
        if d.turn.is_some() {
            // a turn: the aim point to millimetres (angles are kept rounded while dragging)
            if let Some(CamKind::Static { aim: Aim::At(a), .. }) = seq.cameras.get_mut(self.sel).map(|c| &mut c.kind) {
                *a = round3(*a);
            }
            return self.edited();
        }
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
        Some(("orbit", _)) => CamKind::Orbit { target: no_nearest(target), radius: cine::ORBIT_RADIUS, height: cine::ORBIT_HEIGHT, speed: cine::ORBIT_SPEED, start: 0.0, lag: cine::ORBIT_LAG, zoom: Zoom::Auto(NEW_AUTO), clear: true },
        Some(("path", _)) => {
            if !c.keys.iter().any(|k| k.eye.is_some()) {
                c.keys.insert(0, CamKey { at: 0.0, eye: Some(eye), angles: None, fov: None, focus: None, tcb: [0.0; 3], linear: false });
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
    keys_section(ui, ed, names);
    shots_section(ui, ed);
}

fn file_section(ui: &mut Ui, ed: &mut CineEdit, root: &Path) {
    section(ui, ic::FILM, "File", false, |ui| {
        if glass_ui::row_counted(ui.ctx(), "path") {
            ui.horizontal(|ui| {
                ui.add(egui::TextEdit::singleline(&mut ed.path_text).desired_width(200.0).hint_text("name.cine or a path"));
                let p = PathBuf::from(ed.path_text.trim());
                if ui.add_enabled(!ed.path_text.trim().is_empty(), egui::Button::new("open")).clicked() {
                    ed.reqs.push(Req::Open(resolve(root, &p)));
                }
                if ui.add_enabled(ed.seq.is_some() && !ed.path_text.trim().is_empty(), egui::Button::new("save as")).clicked() {
                    let p = home(root, &resolve(root, &p));
                    ed.msg = match ed.save(Some(p)) {
                        Ok(p) => format!("saved {}", p.display()),
                        Err(e) => e,
                    };
                }
            });
        }
        // #424: the static cameras as stations of the game's TV camera (F3); the game reads remaster/tv/trackNN.mot
        if glass_ui::row_counted(ui.ctx(), "TV keys") {
            ui.horizontal(|ui| {
                let track = ed.seq.as_ref().map(|s| s.race.track);
                let has = track.is_some_and(|t| motor_sim::tvcam::override_path(root, t).exists());
                if ui.add_enabled(ed.seq.is_some(), egui::Button::new("export TV keys")).on_hover_text("add this sequence's static cameras to the track's TV camera stations (remaster/tv/trackNN.mot; the shipped stations stay, exporting again replaces the earlier export)").clicked() {
                    if let Some(seq) = ed.seq.as_ref() {
                        ed.msg = cine::export_tv(root, seq).unwrap_or_else(|e| e);
                    }
                }
                if ui.add_enabled(has, egui::Button::new("remove TV keys")).on_hover_text("delete the exported TV stations of this track: the game goes back to the shipped ones").clicked() {
                    if let Some(t) = track {
                        ed.msg = match cine::remove_tv(root, t) {
                            Ok(true) => format!("removed {}", motor_sim::tvcam::override_path(root, t).display()),
                            Ok(false) => "no exported TV keys".into(),
                            Err(e) => e,
                        };
                    }
                }
            });
        }
        if !glass_ui::unfiltered(ui.ctx()) {
            return;
        }
        if !ed.msg.is_empty() {
            ui.weak(&ed.msg);
        }
        ui.horizontal_wrapped(|ui| {
            ui.strong(format!("{}", dir(root).display()));
            if ui.small_button("refresh").clicked() {
                ed.files = None;
            }
        });
        let files = ed.files.get_or_insert_with(|| list(root)).clone();
        let legacy = legacy_dir(root);
        let files_legacy = dir(root) != legacy && files.iter().any(|f| f.parent() == Some(legacy.as_path()));
        if files.is_empty() {
            ui.weak("no .cine files yet");
        }
        ui.horizontal_wrapped(|ui| {
            for f in files {
                let mut name = f.file_name().map_or_else(String::new, |n| n.to_string_lossy().into_owned());
                let old = f.parent() == Some(legacy.as_path());
                if old {
                    name += " (old)";
                }
                let shown = ed.path.as_ref() == Some(&f) || (old && ed.path.as_ref() == Some(&home(root, &f)));
                if ui.selectable_label(shown, name).clicked() {
                    ed.reqs.push(Req::Open(f));
                }
            }
        });
        ui.weak("saving rewrites the file: comments of a hand-written one are not kept");
        if files_legacy {
            ui.weak("(old) = still in the install's remaster/cine; saving writes a copy to the folder above");
        }
    });
}

fn race_section(ui: &mut Ui, ed: &mut CineEdit) {
    let mut ch = false;
    let mut apply = false;
    let Some(seq) = ed.seq.as_mut() else { return };
    section(ui, ic::FLAG, "Race and output", false, |ui| {
        egui::Grid::new("cine-race").num_columns(2).spacing([8.0, 4.0]).show(ui, |ui| {
            let r = &mut seq.race;
            // #423: a recording brings its own track, cars, difficulty, laps and catch-up (the file's `Recording` line)
            let free = r.recording.is_none();
            if let Some(p) = &r.recording {
                if glass_ui::row_counted(ui.ctx(), "recording") {
                    ui.label("recording");
                    let name = std::path::Path::new(p).file_name().map_or(p.clone(), |n| n.to_string_lossy().into_owned());
                    ui.label(name).on_hover_text(format!("{p}\nthe recorded race is filmed: its track, cars and laps (edit the file's Recording line to change it)"));
                    ui.end_row();
                }
            }
            if glass_ui::row_counted(ui.ctx(), "track") {
                ui.label("track");
                ch |= ui.add_enabled_ui(free, |ui| wheel_drag(ui, &mut r.track, Some(1..=8), |d| d.speed(0.05)).changed()).inner;
                ui.end_row();
            }
            if glass_ui::row_counted(ui.ctx(), "cars") {
                ui.label("cars");
                if ui.add_enabled(free, egui::TextEdit::singleline(&mut ed.cars_text).desired_width(160.0)).on_hover_text("car numbers in grid order, slot 0 first (the last slot is `player`)").changed() {
                    let cars: Vec<u32> = ed.cars_text.split_whitespace().filter_map(|w| w.parse().ok()).filter(|c| (1..=10).contains(c)).collect();
                    if !cars.is_empty() && cars.len() <= 8 && cars != r.cars {
                        r.cars = cars;
                        ch = true;
                    }
                }
                ui.end_row();
            }
            if glass_ui::row_counted(ui.ctx(), "difficulty") {
                ui.label("difficulty");
                ui.add_enabled_ui(free, |ui| {
                    ComboBox::from_id_salt("cine-diff").selected_text(DIFFICULTIES.iter().find(|d| d.1 == r.difficulty).map_or("?", |d| d.0)).show_ui(ui, |ui| {
                        for (n, d) in DIFFICULTIES {
                            ch |= ui.selectable_value(&mut r.difficulty, d, n).changed();
                        }
                    });
                });
                ui.end_row();
            }
            if glass_ui::row_counted(ui.ctx(), "laps") {
                ui.label("laps");
                ui.add_enabled_ui(free, |ui| {
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
                });
                ui.end_row();
            }
            if glass_ui::row_counted(ui.ctx(), "start") {
                ui.label("start");
                ui.horizontal(|ui| {
                    ch |= ui.add_enabled(free, egui::Checkbox::new(&mut r.catchup, "catch-up")).changed();
                    ch |= ui.selectable_value(&mut r.start_go, false, "grid").on_hover_text("time 0 = the \"3\" of the countdown").changed();
                    ch |= ui.selectable_value(&mut r.start_go, true, "go").on_hover_text("time 0 = the green light").changed();
                });
                ui.end_row();
            }
            if glass_ui::row_counted(ui.ctx(), "size") {
                ui.label("size");
                ui.horizontal(|ui| {
                    ch |= wheel_drag(ui, &mut seq.size.0, Some(16..=7680), |d| d.speed(2.0)).changed();
                    ui.label("x");
                    ch |= wheel_drag(ui, &mut seq.size.1, Some(16..=4320), |d| d.speed(2.0)).changed();
                });
                ui.end_row();
            }
            if glass_ui::row_counted(ui.ctx(), "fps") {
                ui.label("fps");
                ch |= wheel_drag(ui, &mut seq.fps, Some(1..=240), |d| d.speed(0.2)).changed();
                ui.end_row();
            }
            if glass_ui::row_counted(ui.ctx(), "letterbox") {
                ui.label("letterbox");
                ui.horizontal(|ui| {
                    let mut own = seq.letterbox.is_some();
                    if ui.checkbox(&mut own, "").on_hover_text("black bars down to this aspect ratio (2.39 = cinema scope), after the fades").changed() {
                        seq.letterbox = own.then_some(2.39);
                        ch = true;
                    }
                    if let Some(a) = seq.letterbox.as_mut() {
                        ch |= wheel_drag(ui, a, Some(LETTERBOX_RANGE.0..=LETTERBOX_RANGE.1), |d| d.speed(0.01).max_decimals(2).suffix(" : 1")).changed();
                    }
                });
                ui.end_row();
            }
            if glass_ui::row_counted(ui.ctx(), "end") {
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
            }
            // #153 the render's sound, #349 also heard in the live preview
            if glass_ui::row_counted(ui.ctx(), "audio") {
                ui.label("audio");
                ui.horizontal(|ui| {
                    ch |= ui.checkbox(&mut seq.audio.on, "").on_hover_text("the race's sound, heard by the camera on air, in the rendered video and in the editor's live preview (the ribbon's Sound toggle mutes only the preview)").changed();
                    ui.add_enabled_ui(seq.audio.on, |ui| {
                        ch |= ui.checkbox(&mut seq.audio.doppler, "Doppler").on_hover_text("pitch rises as a car comes at the camera and drops as it goes away").changed();
                    });
                });
                ui.end_row();
            }
        });
        let differs = ed.loaded.as_ref() != Some(&seq.race);
        // a searched row can be edited and applied: the button stays while the race differs
        if glass_ui::unfiltered(ui.ctx()) || (differs && !glass_ui::harvesting()) {
            ui.add_enabled_ui(differs, |ui| {
                apply = ui.button(format!("{}  apply race", ic::RELOAD)).on_hover_text("reload the race from these values (the clock goes back to 0)").clicked();
            });
            if differs {
                ui.weak("the race differs from the one loaded");
            }
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
            if glass_ui::row_counted(ui.ctx(), "name") {
                ui.label("name");
                if ui.add(egui::TextEdit::singleline(&mut name).desired_width(140.0)).changed() {
                    let n = unique_name(seq, &name, Some(sel));
                    seq.cameras[sel].name = n;
                    ch = true;
                }
                ui.end_row();
            }
            if glass_ui::row_counted(ui.ctx(), "kind") {
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
            }
            let c = &mut seq.cameras[sel];
            match &mut c.kind {
                CamKind::Static { eye, aim, zoom, clear } => {
                    if glass_ui::row_counted(ui.ctx(), "eye") {
                        ui.label("eye");
                        ui.horizontal(|ui| {
                            ch |= vec3_row(ui, eye, 0.05);
                            if ui.small_button(ic::CAMERA).on_hover_text("to the free camera (Ctrl+K adds a new one)").clicked() {
                                reqs.push(Req::EyeFromCamera);
                            }
                        });
                        ui.end_row();
                    }
                    if glass_ui::row_counted(ui.ctx(), "aim") {
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
                    }
                    ch |= aim_rows(ui, aim, ncars, names);
                    if glass_ui::row_counted(ui.ctx(), "zoom") {
                        ui.label("zoom");
                        ui.horizontal(|ui| ch |= zoom_row(ui, sel, zoom, true));
                        ui.end_row();
                    }
                    if glass_ui::row_counted(ui.ctx(), "clear view") {
                        ui.label("clear view");
                        ch |= ui.checkbox(clear, "").on_hover_text("move in front of what hides the car (the TV camera's test)").changed();
                        ui.end_row();
                    }
                }
                CamKind::Mounted { target, offset, angles, zoom, hide } => {
                    if glass_ui::row_counted(ui.ctx(), "on") {
                        ui.label("on");
                        ch |= target_combo(ui, "cine-mt", target, false, ncars, names);
                        ui.end_row();
                    }
                    if glass_ui::row_counted(ui.ctx(), "offset") {
                        ui.label("offset");
                        ui.horizontal(|ui| {
                            ch |= vec3_row(ui, offset, 0.02);
                            if ui.small_button(ic::CAMERA).on_hover_text("offset and angles to the free camera, relative to the car").clicked() {
                                reqs.push(Req::EyeFromCamera);
                            }
                        });
                        ui.end_row();
                    }
                    if glass_ui::row_counted(ui.ctx(), "angles") {
                        ui.label("angles");
                        ch |= ui.horizontal(|ui| vec3_row(ui, angles, 0.5)).inner;
                        ui.end_row();
                    }
                    if glass_ui::row_counted(ui.ctx(), "zoom") {
                        ui.label("zoom");
                        ch |= wheel_drag(ui, zoom, Some(0.1..=10.0), |d| d.speed(0.01).max_decimals(2).prefix("x ")).changed();
                        ui.end_row();
                    }
                    if glass_ui::row_counted(ui.ctx(), "hide car") {
                        ui.label("hide car");
                        ch |= ui.checkbox(hide, "").on_hover_text("the camera sits inside the car: leave its body out").changed();
                        ui.end_row();
                    }
                }
                CamKind::Orbit { target, radius, height, speed, start, lag, zoom, clear } => {
                    if glass_ui::row_counted(ui.ctx(), "around") {
                        ui.label("around");
                        ch |= target_combo(ui, "cine-ot", target, false, ncars, names);
                        ui.end_row();
                    }
                    if glass_ui::row_counted(ui.ctx(), "radius") {
                        ui.label("radius");
                        ch |= wheel_drag(ui, radius, Some(1.0..=200.0), |d| d.speed(0.1).max_decimals(1).suffix(" m")).on_hover_text("metres from the car, on the circle").changed();
                        ui.end_row();
                    }
                    if glass_ui::row_counted(ui.ctx(), "height") {
                        ui.label("height");
                        ch |= wheel_drag(ui, height, Some(-50.0..=100.0), |d| d.speed(0.1).max_decimals(1).suffix(" m")).on_hover_text("metres above the car").changed();
                        ui.end_row();
                    }
                    if glass_ui::row_counted(ui.ctx(), "speed") {
                        ui.label("speed");
                        ch |= wheel_drag(ui, speed, Some(-360.0..=360.0), |d| d.speed(0.5).max_decimals(1).suffix(" deg/s")).on_hover_text("turn rate in video time (a slow-motion shot does not slow it); negative turns the other way").changed();
                        ui.end_row();
                    }
                    if glass_ui::row_counted(ui.ctx(), "start") {
                        ui.label("start");
                        ch |= wheel_drag(ui, start, Some(-360.0..=360.0), |d| d.speed(0.5).max_decimals(1).suffix(" deg")).on_hover_text("angle when it goes on air: 0 = straight behind the car, 90 = on its right").changed();
                        ui.end_row();
                    }
                    if glass_ui::row_counted(ui.ctx(), "lag") {
                        ui.label("lag");
                        ch |= wheel_drag(ui, lag, Some(0.0..=5.0), |d| d.speed(0.01).max_decimals(2).suffix(" s")).on_hover_text("how late the circle follows the car's position and heading").changed();
                        ui.end_row();
                    }
                    if glass_ui::row_counted(ui.ctx(), "zoom") {
                        ui.label("zoom");
                        ui.horizontal(|ui| ch |= zoom_row(ui, sel, zoom, true));
                        ui.end_row();
                    }
                    if glass_ui::row_counted(ui.ctx(), "clear view") {
                        ui.label("clear view");
                        ch |= ui.checkbox(clear, "").changed();
                        ui.end_row();
                    }
                }
                CamKind::Pose { mode, target } => {
                    if glass_ui::row_counted(ui.ctx(), "follows") {
                        ui.label("follows");
                        ch |= target_combo(ui, "cine-pt", target, false, ncars, names);
                        ui.end_row();
                    }
                    if glass_ui::row_counted(ui.ctx(), "view") {
                        ui.label("view");
                        ui.weak(format!("the game's {} camera", POSE_MODES.iter().find(|m| m.1 == *mode).map_or("?", |m| m.0)));
                        ui.end_row();
                    }
                }
                CamKind::Path { drive, aim, zoom, clear } => {
                    if glass_ui::row_counted(ui.ctx(), "paced by") {
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
                    }
                    if glass_ui::row_counted(ui.ctx(), "aim") {
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
                    }
                    if let PathAim::Track { target, lag, lead } = aim {
                        let mut a = Aim::Track { target: *target, lag: *lag, lead: *lead };
                        if aim_rows(ui, &mut a, ncars, names) {
                            if let Aim::Track { target: t, lag: l, lead: d } = a {
                                (*target, *lag, *lead) = (no_nearest(t), l, d);
                            }
                            ch = true;
                        }
                    }
                    if glass_ui::row_counted(ui.ctx(), "zoom") {
                        ui.label("zoom");
                        ui.horizontal(|ui| ch |= zoom_row(ui, sel, zoom, true));
                        ui.end_row();
                    }
                    if glass_ui::row_counted(ui.ctx(), "clear view") {
                        ui.label("clear view");
                        ch |= ui.checkbox(clear, "").changed();
                        ui.end_row();
                    }
                }
            }
            // #421 depth of field (every kind): Remaster previews and renders only
            if glass_ui::row_counted(ui.ctx(), "aperture") {
                ui.label("aperture");
                ui.horizontal(|ui| {
                    let mut on = c.lens.aperture.is_some();
                    if ui.checkbox(&mut on, "").on_hover_text("depth of field: blur what lies off the focus distance (Remaster); a long zoom blurs more").changed() {
                        c.lens.aperture = on.then_some(NEW_APERTURE);
                        ch = true;
                    }
                    if let Some(n) = c.lens.aperture.as_mut() {
                        ch |= wheel_drag(ui, n, Some(APERTURE_RANGE.0..=APERTURE_RANGE.1), |d| d.speed(0.05).max_decimals(2).prefix("f/")).on_hover_text("f-number: lower = shallower focus (below 1 is an artistic dial)").changed();
                    }
                });
                ui.end_row();
            }
            if glass_ui::row_counted(ui.ctx(), "focus") {
                ui.label("focus");
                ui.add_enabled_ui(c.lens.aperture.is_some(), |ui| {
                    ui.horizontal(|ui| ch |= focus_row(ui, "cine-focus", &mut c.lens.focus, ncars, names));
                });
                ui.end_row();
            }
        });
        if let Some(k) = new_kind {
            let eye = eye_now.unwrap_or(Vec3::ZERO);
            convert(&mut seq.cameras[sel], k, eye, fallback);
            ch = true;
        }
        // shakes (searchable as `shake`)
        if glass_ui::row_counted(ui.ctx(), "shake") {
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
            if glass_ui::row_counted(ui.ctx(), "target") {
                ui.label("target");
                ch |= target_combo(ui, "cine-at", target, true, ncars, names);
                ui.end_row();
            }
            if glass_ui::row_counted(ui.ctx(), "lag / lead") {
                ui.label("lag / lead");
                ui.horizontal(|ui| {
                    ch |= wheel_drag(ui, lag, Some(0.0..=5.0), |d| d.speed(0.01).max_decimals(2).suffix(" s")).on_hover_text("the cameraman's delay").changed();
                    ch |= wheel_drag(ui, lead, Some(0.0..=5.0), |d| d.speed(0.01).max_decimals(2).suffix(" s")).on_hover_text("aim ahead of the car").changed();
                });
                ui.end_row();
            }
        }
        Aim::At(a) => {
            if glass_ui::row_counted(ui.ctx(), "at") {
                ui.label("at");
                ch |= ui.horizontal(|ui| vec3_row(ui, a, 0.05)).inner;
                ui.end_row();
            }
        }
    }
    ch
}

/// A Focus field (#421): the camera's own car, a distance in metres, or a car. True when changed.
fn focus_row(ui: &mut Ui, id: &str, f: &mut FocusOn, ncars: usize, names: &[String]) -> bool {
    let mut ch = false;
    let cur = match f {
        FocusOn::Own => "own",
        FocusOn::Dist(_) => "distance",
        FocusOn::Car(_) => "car",
    };
    ComboBox::from_id_salt((id, "kind")).width(76.0).selected_text(cur).show_ui(ui, |ui| {
        for (n, new) in [("own", FocusOn::Own), ("distance", FocusOn::Dist(NEW_FOCUS)), ("car", FocusOn::Car(Target::Leader))] {
            if ui.selectable_label(n == cur, n).clicked() && n != cur {
                *f = new;
                ch = true;
            }
        }
    }).response.on_hover_text("own = the car this camera shows; distance = metres in front of the lens; car = that car");
    match f {
        FocusOn::Own => {}
        FocusOn::Dist(d) => ch |= wheel_drag(ui, d, Some(FOCUS_MIN..=10_000.0), |x| x.speed(0.1).max_decimals(1).suffix(" m")).changed(),
        FocusOn::Car(t) => ch |= target_combo(ui, id, t, false, ncars, names),
    }
    ch
}

fn keys_section(ui: &mut Ui, ed: &mut CineEdit, names: &[String]) {
    let (mut ch, mut reqs) = (false, Vec::new());
    let Some(seq) = ed.seq.as_mut() else { return };
    let ncars = seq.race.cars.len();
    let sel = ed.sel.min(seq.cameras.len() - 1);
    let c = &mut seq.cameras[sel];
    let lens_focus = c.lens.focus;
    let path = matches!(c.kind, CamKind::Path { .. });
    let unit = if matches!(c.kind, CamKind::Path { drive: Drive::Car(_), .. }) { " m" } else { " s" };
    let title = format!("Keys of {}", c.name);
    section(ui, ic::SPLINE, &title, false, |ui| {
        if !glass_ui::unfiltered(ui.ctx()) {
            return;
        }
        ui.weak(if path { "Eye and Angles keys move the camera along a path; Fov keys zoom; Focus keys rack the focus. Ctrl+Shift+K adds one at the free camera." } else { "Fov keys zoom this camera over its shot (seconds since the shot began); Focus keys rack the focus. Ctrl+Shift+K adds one (a Focus key on a camera with an aperture or zoom auto)." });
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
                        key.fov = on.then_some(FOV_X.get());
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
                // #421 a rack focus: the focus from this key on, pulled between keys
                ui.horizontal(|ui| {
                    let mut on = key.focus.is_some();
                    if ui.checkbox(&mut on, "focus").on_hover_text("a Focus key: the focus is pulled to this subject by this time (needs the camera's aperture)").changed() {
                        key.focus = on.then_some(lens_focus);
                        ch = true;
                    }
                    if let Some(f) = key.focus.as_mut() {
                        ch |= focus_row(ui, &format!("cine-kf{k}"), f, ncars, names);
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
        c.keys.retain(|k| k.eye.is_some() || k.angles.is_some() || k.fov.is_some() || k.focus.is_some());
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

fn follow_name(f: Follow) -> String {
    match f {
        Follow::Car(n) => format!("car {n}"),
        f => FOLLOWS.iter().find(|x| x.1 == f).map_or("battle", |x| x.0).into(),
    }
}

/// An auto shot's second row in the shots grid: what it follows, its Min and its pool of cameras. True when changed.
fn auto_row(ui: &mut Ui, i: usize, a: &mut Auto, names: &[String], cars: usize) -> bool {
    let mut ch = false;
    ui.label("");
    ui.weak("follow").on_hover_text("battle = the chasing car of the closest pair; nearest = the car nearest each camera");
    ComboBox::from_id_salt(("cine-shot-follow", i)).width(90.0).selected_text(follow_name(a.follow)).show_ui(ui, |ui| {
        for f in FOLLOWS.iter().map(|f| f.1).chain((0..cars).map(Follow::Car)) {
            if ui.selectable_label(a.follow == f, follow_name(f)).clicked() && a.follow != f {
                a.follow = f;
                ch = true;
            }
        }
    });
    ch |= wheel_drag(ui, &mut a.min, Some(0.0..=30.0), |d| d.speed(0.05).max_decimals(1).prefix("min ").suffix(" s")).on_hover_text("a shot holds at least this long, unless the car is lost").changed();
    let label = if a.pool.is_empty() { "cams: world".to_string() } else { format!("cams: {}", a.pool.len()) };
    ui.menu_button(label, |ui| {
        ui.weak("none ticked = every static and path camera");
        for (k, nm) in names.iter().enumerate() {
            let mut on = a.pool.contains(&k);
            if ui.checkbox(&mut on, nm).changed() {
                if on {
                    a.pool.push(k);
                    a.pool.sort_unstable();
                } else {
                    a.pool.retain(|&c| c != k);
                }
                ch = true;
            }
        }
    });
    ui.label("");
    ui.label("");
    ui.label("");
    ui.end_row();
    ch
}

fn shots_section(ui: &mut Ui, ed: &mut CineEdit) {
    let (mut ch, mut cut, mut jump) = (false, false, None);
    let t = ed.t;
    let Some(seq) = ed.seq.as_mut() else { return };
    let air = shot_at(seq, t);
    section(ui, ic::CLOCK, "Shots", false, |ui| {
        if !glass_ui::unfiltered(ui.ctx()) {
            return;
        }
        ui.horizontal(|ui| {
            cut = ui.button(format!("{}  cut to {} at {:.2} s", ic::VIDEO, seq.cameras[ed.sel.min(seq.cameras.len() - 1)].name, t)).on_hover_text("a shot of the selected camera at the clock").clicked();
        });
        let names: Vec<String> = seq.cameras.iter().map(|c| c.name.clone()).collect();
        let cars = seq.race.cars.len();
        let mut drop = None;
        let n = seq.shots.len();
        egui::Grid::new("cine-shots").num_columns(10).spacing([6.0, 3.0]).show(ui, |ui| {
            for h in ["", "time", "camera", "speed", "pitch", "mode", "fade in", "fade out", "dissolve", ""] {
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
                let shown = if s.auto.is_some() { "auto" } else { names.get(s.cam).map_or("?", |n| n.as_str()) };
                ComboBox::from_id_salt(("cine-shot-cam", i)).width(90.0).selected_text(shown).show_ui(ui, |ui| {
                    if ui.selectable_label(s.auto.is_some(), "auto").on_hover_text("the director picks the camera, TV style").clicked() && s.auto.is_none() {
                        s.auto = Some(Auto::default());
                        ch = true;
                    }
                    for (k, nm) in names.iter().enumerate() {
                        if ui.selectable_label(s.auto.is_none() && s.cam == k, nm).clicked() && (s.auto.is_some() || s.cam != k) {
                            (s.cam, s.auto) = (k, None);
                            ch = true;
                        }
                    }
                });
                ch |= wheel_drag(ui, &mut s.speed, Some(SPEED_RANGE.0..=SPEED_RANGE.1), |d| d.speed(0.01).max_decimals(2).prefix("x ")).changed();
                let shown = PITCHES.iter().find(|p| p.1 == s.pitch).map_or("keep", |p| p.0);
                ComboBox::from_id_salt(("cine-shot-pitch", i)).width(56.0).selected_text(shown).show_ui(ui, |ui| {
                    for (name, p) in PITCHES {
                        ch |= ui.selectable_value(&mut s.pitch, p, name).changed();
                    }
                }).response.on_hover_text("the shot's sound in slow motion: keep = normal pitch; tape = slowed with the shot, pitch dropping like a tape (rendered videos and the live preview)");
                let shown = s.mode.map_or("-", |m| if m == RenderMode::Remaster { "remaster" } else { "classic" });
                ComboBox::from_id_salt(("cine-shot-mode", i)).width(72.0).selected_text(shown).show_ui(ui, |ui| {
                    ch |= ui.selectable_value(&mut s.mode, None, "- (the render's)").changed();
                    for m in [RenderMode::Classic, RenderMode::Remaster] {
                        ch |= ui.selectable_value(&mut s.mode, Some(m), m.name().to_ascii_lowercase()).changed();
                    }
                }).response.on_hover_text("the graphics mode this shot is drawn in (#427): classic or remaster from this shot on; - = the render's own (F9 in the viewer, --classic / --remaster)");
                ch |= wheel_drag(ui, &mut s.fade_in, Some(0.0..=10.0), |d| d.speed(0.02).max_decimals(2)).changed();
                ch |= wheel_drag(ui, &mut s.fade_out, Some(0.0..=10.0), |d| d.speed(0.02).max_decimals(2)).changed();
                ch |= wheel_drag(ui, &mut s.dissolve, Some(0.0..=10.0), |d| d.speed(0.02).max_decimals(2)).on_hover_text("seconds the cut into this shot crossfades from the camera before (an auto shot: every cut in it); the on-air preview shows it too").changed();
                if ui.add_enabled(n > 1, egui::Button::new(ic::TRASH).small()).clicked() {
                    drop = Some(i);
                }
                ui.end_row();
                if let Some(a) = s.auto.as_mut() {
                    ch |= auto_row(ui, i, a, &names, cars);
                }
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

    /// #349 the live sound plays on only while the clock steps forward a little
    #[test]
    fn sound_continuity() {
        assert!(!continuous(None, 1.0));
        assert!(continuous(Some(1.0), 1.0 + 1.0 / 60.0));
        assert!(!continuous(Some(1.0), 1.0), "paused / the same frame");
        assert!(!continuous(Some(1.0), 0.5), "seek back");
        assert!(!continuous(Some(1.0), 3.0), "jump ahead");
    }

    fn temp(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("cine_dirs_{tag}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn sequences_live_in_the_config_folder_with_the_install_folder_as_read_only_fallback() {
        let base = temp("dirs");
        let (cfg, root) = (base.join("cfg"), base.join("install"));
        let (d, old) = (dir_in(Some(cfg.clone()), &root), legacy_dir(&root));
        assert_eq!(d, cfg.join("Cine"));
        assert_eq!(dir_in(None, &root), old, "no config folder: the old place");
        std::fs::create_dir_all(&d).unwrap();
        std::fs::create_dir_all(&old).unwrap();
        std::fs::write(old.join("a.cine"), "").unwrap();
        std::fs::write(old.join("both.cine"), "").unwrap();
        std::fs::write(d.join("both.cine"), "").unwrap();
        std::fs::write(d.join("b.cine"), "").unwrap();
        std::fs::write(d.join("notes.txt"), "").unwrap();
        let names: Vec<_> = list_in(&d, &old).iter().map(|p| (p.parent() == Some(old.as_path()), p.file_name().unwrap().to_string_lossy().into_owned())).collect();
        assert_eq!(names, vec![(true, "a.cine".into()), (false, "b.cine".into()), (false, "both.cine".into())]);
        // a bare name: config first, the old folder when only it has the file, else a new config file
        assert_eq!(resolve_in(&d, &old, Path::new("both.cine")), d.join("both.cine"));
        assert_eq!(resolve_in(&d, &old, Path::new("a.cine")), old.join("a.cine"));
        assert_eq!(resolve_in(&d, &old, Path::new("fresh.cine")), d.join("fresh.cine"));
        assert_eq!(resolve_in(&d, &old, Path::new("sub/x.cine")), PathBuf::from("sub/x.cine"));
        let _ = std::fs::remove_dir_all(&base);
    }

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

    fn mounted(angles: &str) -> Sequence {
        cine::parse(&format!("Camera m mounted Target leader Offset 0 1 0 Angles {angles}\nShot 0 m\n")).unwrap().0
    }

    fn angles(seq: &Sequence) -> Vec3 {
        let CamKind::Mounted { angles, .. } = seq.cameras[0].kind else { panic!() };
        angles
    }

    #[test]
    fn a_turn_about_the_right_ring_is_a_pitch() {
        let mut seq = mounted("0 0 0");
        let turn = Turn::Angles { local: Mat3::IDENTITY, base: Vec3::ZERO };
        apply_turn(&mut seq, 0, Handle::Eye, turn, 0, 30.0 * DEG);
        assert!(angles(&seq).distance(Vec3::new(30.0, 0.0, 0.0)) < 0.01, "{:?}", angles(&seq));
    }

    #[test]
    fn a_heading_drag_across_180_keeps_going_the_short_way() {
        let mut seq = mounted("0 175 0");
        let local = euler_to_rot(Vec3::new(0.0, 175.0, 0.0) * DEG);
        apply_turn(&mut seq, 0, Handle::Eye, Turn::Angles { local, base: Vec3::new(0.0, 175.0, 0.0) }, 1, 10.0 * DEG);
        assert!(angles(&seq).distance(Vec3::new(0.0, 185.0, 0.0)) < 0.01, "{:?}", angles(&seq));
    }

    #[test]
    fn turning_an_aim_point_keeps_its_distance_from_the_eye() {
        let (mut seq, _) = cine::parse("Camera a static Eye 1 2 3 At 1 2 13\nShot 0 a\n").unwrap();
        let (eye, aim) = (Vec3::new(1.0, 2.0, 3.0), Vec3::new(1.0, 2.0, 13.0));
        let rot = look_at(eye, aim).unwrap();
        // heading (the camera's up ring) a quarter turn: the point swings to the side, level with the eye
        apply_turn(&mut seq, 0, Handle::Eye, Turn::Aim { eye, aim, rot }, 1, 90.0 * DEG);
        let CamKind::Static { aim: Aim::At(a), .. } = seq.cameras[0].kind else { panic!() };
        assert!(((a - eye).length() - 10.0).abs() < 1e-3 && (a - eye).y.abs() < 1e-3 && (a - eye).x.abs() > 9.99, "{a:?}");
    }

    #[test]
    fn rotate_ring_drag_turns_a_mounted_camera_and_the_rig_has_no_bank_for_an_aim_point() {
        let (mut ed, _) = picker();
        ed.gizmo = GizmoMode::Rotate;
        let rig = ed.rig(None);
        // a static camera with a fixed aim: two rings work, the bank ring does not
        assert_eq!(rig.map(|r| r.en), Some([true, true, false]));
        // a mounted camera needs the race for its place: none, no gizmo
        let mut ed = CineEdit::default();
        ed.set(mounted("0 0 0"), None);
        ed.gizmo = GizmoMode::Rotate;
        assert!(ed.rig(None).is_none());
    }

    #[test]
    fn a_held_edit_is_one_undo_step_and_undo_redo_restore_it() {
        let (mut ed, _) = picker();
        let eye = |ed: &CineEdit| match ed.seq.as_ref().unwrap().cameras[0].kind {
            CamKind::Static { eye, .. } => eye,
            _ => panic!(),
        };
        let nudge = |ed: &mut CineEdit, x: f32| {
            if let CamKind::Static { eye, .. } = &mut ed.seq.as_mut().unwrap().cameras[0].kind {
                eye.x = x;
            }
            ed.edited();
        };
        nudge(&mut ed, 1.0);
        ed.settle(true);
        nudge(&mut ed, 2.0);
        ed.settle(true);
        assert!(!ed.can_undo(), "pointer held: no step yet");
        ed.settle(false);
        assert_eq!(ed.undo.len(), 1);
        ed.undo();
        assert_eq!(eye(&ed).x, 0.0);
        assert!(!ed.can_undo() && ed.can_redo() && ed.dirty);
        ed.redo();
        assert_eq!(eye(&ed).x, 2.0);
        // a new edit drops the redo stack
        ed.undo();
        nudge(&mut ed, 5.0);
        ed.settle(false);
        assert!(!ed.can_redo() && ed.undo.len() == 1);
        // opening a sequence starts a new history
        let (seq, _) = cine::parse("Camera a static Eye 0 0 0 At 1 0 0\nShot 0 a\n").unwrap();
        ed.set(seq, None);
        assert!(!ed.can_undo() && !ed.can_redo());
    }

    #[test]
    fn a_gizmo_drag_is_one_undo_step() {
        let (mut ed, project) = picker();
        assert!(ed.press(&project, (550.0, 500.0), ray_at(5.0, 0.0), None));
        ed.mouse_move(&project, (570.0, 500.0), ray_at(7.0, 0.0), None);
        ed.settle(false);
        assert!(!ed.can_undo(), "mid-drag");
        ed.mouse_move(&project, (580.0, 500.0), ray_at(8.0004, 3.0), None);
        ed.release(None);
        ed.settle(false);
        assert_eq!(ed.undo.len(), 1);
        ed.undo();
        let CamKind::Static { eye, .. } = ed.seq.as_ref().unwrap().cameras[0].kind else { panic!() };
        assert_eq!(eye, Vec3::ZERO);
    }

    #[test]
    fn undoing_a_removal_brings_the_camera_back_and_keeps_the_selection_valid() {
        let (seq, _) = cine::parse("Camera a static Eye 0 0 0 At 1 0 0\nCamera b static Eye 0 0 0 At 1 0 0\nShot 0 a\nShot 2 b\n").unwrap();
        let mut ed = CineEdit::default();
        ed.set(seq, None);
        ed.sel = 1;
        ed.remove_selected();
        ed.settle(false);
        assert_eq!(ed.seq.as_ref().unwrap().cameras.len(), 1);
        ed.undo();
        assert_eq!(ed.seq.as_ref().unwrap().cameras.len(), 2);
        ed.redo();
        assert!(ed.sel < ed.seq.as_ref().unwrap().cameras.len());
    }

    #[test]
    fn names_stay_unique_and_spaceless() {
        let (seq, _) = cine::parse("Camera a static Eye 0 0 0 At 1 0 0\nCamera b static Eye 0 0 0 At 1 0 0\n").unwrap();
        assert_eq!(unique_name(&seq, "a", Some(1)), "a2");
        assert_eq!(unique_name(&seq, "my cam", None), "my_cam");
        assert_eq!(unique_name(&seq, "a", Some(0)), "a");
    }

    /// #438: Add key on a static camera adds a Focus key when it has an aperture or Zoom auto (a Fov key would override
    /// the zoom), and a Fov key otherwise
    #[test]
    fn add_key_on_a_static_camera_is_a_focus_key_with_an_aperture_or_zoom_auto() {
        let cam = Camera::from_rot(Vec3::ZERO, Mat3::IDENTITY, 1.0, 1.0);
        let add = |tweak: &dyn Fn(&mut CineCam)| {
            let mut ed = CineEdit::default();
            ed.set(new_seq(1, &cam), None);
            tweak(&mut ed.seq.as_mut().unwrap().cameras[0]);
            ed.edit(Req::AddKey, &cam, None);
            ed.seq.as_ref().unwrap().cameras[0].keys.clone()
        };
        let plain = |c: &mut CineCam| c.kind = CamKind::Static { eye: Vec3::ZERO, aim: Aim::At(Vec3::Z), zoom: Zoom::Fixed(1.0), clear: false };
        let keys = add(&plain);
        assert!(keys[0].fov.is_some() && keys[0].focus.is_none(), "a plain static camera: a Fov key");
        let keys = add(&|c| {
            plain(c);
            c.lens.aperture = Some(2.8);
            c.lens.focus = FocusOn::Dist(30.0);
        });
        assert!(keys[0].fov.is_none(), "an aperture: no Fov key");
        assert_eq!(keys[0].focus, Some(FocusOn::Dist(30.0)), "the camera's own focus");
        let keys = add(&|c| {
            if let CamKind::Static { zoom, .. } = &mut c.kind {
                *zoom = Zoom::Auto(25.0);
            }
        });
        assert!(keys[0].fov.is_none() && keys[0].focus.is_some(), "Zoom auto: a Focus key");
    }
}
