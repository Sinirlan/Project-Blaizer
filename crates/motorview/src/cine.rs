//! Cine (#32, re-notes/20-cine.md): planned camera sequences. A `.cine` file describes an all-AI race
//! and a set of cameras with the shots that put them on air; `Director` turns the race state into the
//! camera of a moment, `cine_render` records the result as a video. Plain `Key value` words like
//! `.fill` (procgen/fill.rs): `#` starts a comment, a bad line is reported and skipped, game
//! coordinates, angles in degrees.
//!
//! ```text
//! # Motorhead cine v1
//! Race Track 1 Cars 1 2 3 4 5 6 7 8 Difficulty medium Laps 3 Catchup on Start grid|go     # time 0 = the "3" of the countdown (grid) or the green light (go)
//! Output Size 1920 1080 Fps 60 Letterbox 2.39     # Letterbox = black bars down to this aspect (omitted = none)
//! End 40                              # seconds; omitted = every car finished + 2 s
//! Camera gantry static Eye x y z Target car 3 Zoom auto 25 Lag 0.3 Lead 0.2 Clear on
//! Camera hill   static Eye x y z At x y z Zoom 1.5               # fixed aim
//! Camera roof   mounted Target leader Offset 0 1.6 -0.5 Angles 5 0 0 Zoom 1 Hide off
//! Camera follow chase|far|incar|bumper|tv Target nearest
//! Camera fly    path Drive time|car N Aim keys|tangent|Target T Lag 0.2 Zoom 1  # a keyed path (#151)
//! Camera drone  orbit Target leader Radius 12 Height 4 Speed 20 Start 0 Lag 0.3 Zoom 1 Clear on  # circles a car (#425)
//! Key fly 0 Eye x y z Angles 5 90 0 Fov 60 Tcb 0 0 0 Linear off    # keys of a path / zoom keys of any camera
//! Shake gantry proximity 2 Radius 15  # proximity|handheld|impact, amplitude in degrees
//! Shot 0 gantry Speed 0.5 FadeIn 1 FadeOut 1   # from this second on the camera is on air
//! Shot 11 nose Speed 0.5 Pitch tape            # the shot's sound slows with its Speed, pitch included (#426); keep = normal pitch
//! Shot 12 nose Mode remaster                # this shot on draws in Remaster (or classic); no Mode = the render's own (#427)
//! Shot 20 auto Cams gantry hill Follow battle Min 3 Dissolve 0.4  # the director picks the camera, TV style; Dissolve = every cut crossfades (#154)
//! ```
//! `Target`: `car N` (grid slot, 0-based, order of `Cars`), `leader`, `nearest` (static cameras: the
//! car nearest the eye), `player` (the last slot, as in the viewer's line-up), `auto` (the auto director's car, #154).

use crate::drive::{cam_origin, cam_origin_at, CamTune, PoseCam};
use crate::render::{Camera, Dof, RenderMode};
use glam::{Mat3, Vec3};
use motor_data::cfg::{Motion, MotionKey};
use motor_sim::props::evaluate;
use motor_sim::tvcam::{euler_to_rot, rot_to_euler, TvShot};
use motor_sim::{Difficulty, Race};
use std::collections::HashMap;

pub use crate::drive::CamMode;

pub(crate) const HEADER: &str = "# Motorhead cine v1";
const DEG: f32 = std::f32::consts::PI / 180.0;
/// Zoom limits of the TV camera (`motor_sim::tvcam`), and the far-plane growth cap.
const ZOOM_MAX: f32 = 10.0;
const FAR_K_MAX: f32 = 4.0;
/// Where a tracking camera aims on the car: above its model origin, metres.
const AIM_UP: f32 = 0.5;
/// Limits of the per-shot options and the Fov keys.
pub const SPEED_RANGE: (f32, f32) = (0.1, 4.0);
pub const FOV_RANGE: (f32, f32) = (1.0, 170.0);
/// `Output Letterbox A` limits (width / height of the visible window).
pub const LETTERBOX_RANGE: (f32, f32) = (1.0, 4.0);

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Target {
    /// grid slot (index into `RaceDef::cars`)
    Car(usize),
    Leader,
    Nearest,
    Player,
    /// the car the auto director follows (#154; outside auto shots the leader)
    Auto,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Zoom {
    Fixed(f32),
    /// distance to the car / this many metres = zoom 1.0 at that distance (tvcam style)
    Auto(f32),
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Aim {
    At(Vec3),
    /// follow a car; `lag` = cameraman time constant (s), `lead` = look-ahead (s of car velocity)
    Track { target: Target, lag: f32, lead: f32 },
}

/// What moves a path camera along its keys (#151).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Drive {
    /// key times are seconds since the camera's shot started
    Time,
    /// key times are metres of lap arc (`Progress::s`) of this car: a dolly keeping pace with it
    Car(Target),
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PathAim {
    /// the `Angles` keys
    Keys,
    /// along the path
    Tangent,
    /// at a car, as a static tracking camera
    Track { target: Target, lag: f32, lead: f32 },
}

#[derive(Clone, Debug, PartialEq)]
pub enum CamKind {
    Static { eye: Vec3, aim: Aim, zoom: Zoom, clear: bool },
    Mounted { target: Target, offset: Vec3, angles: Vec3, zoom: f32, hide: bool },
    /// the game's own views (`PoseCam`): chase, far, in-car, bumper, tv
    Pose { mode: CamMode, target: Target },
    /// a camera on a keyed TCB path (#151)
    Path { drive: Drive, aim: PathAim, zoom: Zoom, clear: bool },
    /// a drone circling a car (#425): `radius` metres out and `height` up, turning `speed` degrees a second from `start`
    /// degrees (0 = straight behind the car, positive towards its right), the car's position and heading eased by `lag`
    Orbit { target: Target, radius: f32, height: f32, speed: f32, start: f32, lag: f32, zoom: Zoom, clear: bool },
}

/// `Camera ... orbit` defaults (#425).
pub const ORBIT_RADIUS: f32 = 12.0;
pub const ORBIT_HEIGHT: f32 = 4.0;
pub const ORBIT_SPEED: f32 = 20.0;
pub const ORBIT_LAG: f32 = 0.3;


/// One `Key` line: a key of each channel it names. Eye and Angles are for path cameras, Fov for any camera.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CamKey {
    /// seconds since the shot started, or metres of lap arc (`Drive car`)
    pub at: f32,
    pub eye: Option<Vec3>,
    /// pitch heading bank, degrees (as `mounted` `Angles`)
    pub angles: Option<Vec3>,
    /// horizontal field of view, degrees (90 = zoom 1)
    pub fov: Option<f32>,
    /// what the lens focuses on from this key (#421; a rack between keys, `focus_between`)
    pub focus: Option<FocusOn>,
    /// tension, continuity, bias
    pub tcb: [f32; 3],
    pub linear: bool,
}

/// What a camera's lens focuses on (#421, `Focus`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum FocusOn {
    /// the car the camera shows (a static camera with `At`: that point); the default
    Own,
    /// metres in front of the lens
    Dist(f32),
    Car(Target),
}

/// A camera's lens (#421, `Aperture` / `Focus` on its line): with an aperture, the Remaster capture blurs what lies off
/// the focus distance (render.rs `Dof`). No aperture = everything sharp.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Lens {
    /// f-number
    pub aperture: Option<f32>,
    /// the focus without `Focus` keys
    pub focus: FocusOn,
}

impl Default for Lens {
    fn default() -> Self {
        Lens { aperture: None, focus: FocusOn::Own }
    }
}

/// `Aperture` f-numbers: below about 1 no real lens, an artistic dial for a shallow focus at the short lenses and
/// distances of most cine cameras (a real one needs a lens of under 1 degree from hundreds of metres) [design choice]
pub const APERTURE_RANGE: (f32, f32) = (0.1, 32.0);
/// the nearest focus, metres
pub const FOCUS_MIN: f32 = 0.3;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShakeKind {
    /// cars passing close to the camera
    Proximity,
    /// a slow sway all the time
    Handheld,
    /// hits (walls, cars, props) near the camera
    Impact,
}

pub(crate) const SHAKE_KINDS: [(&str, ShakeKind); 3] = [("proximity", ShakeKind::Proximity), ("handheld", ShakeKind::Handheld), ("impact", ShakeKind::Impact)];

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Shake {
    pub kind: ShakeKind,
    /// degrees of yaw / pitch at full strength (roll half of it; handheld also 1 cm of sway per degree)
    pub amp: f32,
    /// metres: cars further from the eye do not count (proximity, impact)
    pub radius: f32,
}

impl ShakeKind {
    fn default_radius(self) -> f32 {
        match self {
            ShakeKind::Impact => 30.0,
            _ => 15.0,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct CineCam {
    pub name: String,
    pub kind: CamKind,
    /// sorted by `at`
    pub keys: Vec<CamKey>,
    pub shakes: Vec<Shake>,
    pub lens: Lens,
}

/// The car an auto shot follows (#154).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Follow {
    /// the chasing car of the closest pair of neighbours in the standings
    Battle,
    Leader,
    /// each camera is scored with the car nearest its eye; the followed car is the one on air
    Nearest,
    /// grid slot
    Car(usize),
    Player,
}

pub(crate) const FOLLOWS: [(&str, Follow); 4] = [("battle", Follow::Battle), ("leader", Follow::Leader), ("nearest", Follow::Nearest), ("player", Follow::Player)];

/// `Shot T auto ...` (#154): the director picks the camera, TV style, among `pool`.
#[derive(Clone, Debug, PartialEq)]
pub struct Auto {
    /// camera indices; empty = every static and path camera
    pub pool: Vec<usize>,
    pub follow: Follow,
    /// seconds a shot holds at least (unless the car is lost)
    pub min: f32,
}

/// An auto shot's default `Min`, seconds.
pub const AUTO_MIN: f32 = 3.0;

impl Default for Auto {
    fn default() -> Self {
        Auto { pool: Vec::new(), follow: Follow::Battle, min: AUTO_MIN }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Shot {
    pub t: f32,
    /// the camera on air (an auto shot: the first of its pool, the camera before the director has picked)
    pub cam: usize,
    /// race seconds per video second (slow motion below 1)
    pub speed: f32,
    /// the shot's sound in slow motion (#426)
    pub pitch: Pitch,
    /// the graphics mode this shot is drawn in (#427), None = the render's own
    pub mode: Option<RenderMode>,
    /// seconds from black at the start / to black at the end of the shot
    pub fade_in: f32,
    pub fade_out: f32,
    /// seconds the cut into this shot crossfades from the camera before (an auto shot: every cut inside it too), 0 = a hard cut
    pub dissolve: f32,
    /// the director picks the camera (#154); fades and speed belong to the whole auto shot
    pub auto: Option<Auto>,
}

/// What a shot's `Speed` does to its sound (#426): `Keep` plays every sound at its own pitch (the events just come further
/// apart), `Tape` plays the mix `Speed` times as fast, pitch included, like a tape slowing down.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Pitch {
    #[default]
    Keep,
    Tape,
}

pub const PITCHES: [(&str, Pitch); 2] = [("keep", Pitch::Keep), ("tape", Pitch::Tape)];

impl Shot {
    pub fn new(t: f32, cam: usize) -> Shot {
        Shot { t, cam, speed: 1.0, pitch: Pitch::Keep, mode: None, fade_in: 0.0, fade_out: 0.0, dissolve: 0.0, auto: None }
    }
}

/// A camera standing in the world (static or path), the ones an auto shot picks by distance.
pub fn is_world(k: &CamKind) -> bool {
    matches!(k, CamKind::Static { .. } | CamKind::Path { .. })
}

/// The cameras an auto shot picks from: its pool, else every world camera, else every camera.
pub fn auto_pool(seq: &Sequence, a: &Auto) -> Vec<usize> {
    let mut p: Vec<usize> = a.pool.iter().copied().filter(|&i| i < seq.cameras.len()).collect();
    if p.is_empty() {
        p = (0..seq.cameras.len()).filter(|&i| is_world(&seq.cameras[i].kind)).collect();
    }
    if p.is_empty() {
        p = (0..seq.cameras.len()).collect();
    }
    p
}

#[derive(Clone, Debug, PartialEq)]
pub struct RaceDef {
    pub track: u32,
    pub cars: Vec<u32>,
    pub difficulty: Difficulty,
    pub laps: Option<u32>,
    pub catchup: bool,
    /// time 0 is the green light, else the "3" of the countdown (the fly-by is skipped)
    pub start_go: bool,
    /// `Recording FILE.mhr` (#423, 20-cine "Race source"): film that recorded race instead of an all-AI one. Its
    /// track, cars, laps and drivers replace the ones above when it is loaded (`from_header`); the path as written
    /// (`recording_path` resolves it)
    pub recording: Option<String>,
}

impl RaceDef {
    /// The track, cars and laps of a recording's header (#423), so the scene and the editor show the recorded race.
    pub fn from_header(&mut self, h: &motor_sim::replay::Header) {
        (self.track, self.cars, self.laps, self.catchup) = (h.track, h.cars.clone(), Some(h.laps), h.catchup);
        if let motor_sim::AiRow::Difficulty(d) = h.row {
            self.difficulty = d;
        }
    }
}

/// Where a `Recording` path points (#423): the path as written (absolute, `~/`, or from the working folder) when it
/// exists, else that name in the game's Replays folder (`ghost::replay_dir`), where the game saves its recordings.
pub fn recording_path(p: &str) -> std::path::PathBuf {
    let given = match p.strip_prefix("~/") {
        Some(rest) => std::env::var_os("HOME").map_or_else(|| p.into(), |h| std::path::Path::new(&h).join(rest)),
        None => std::path::PathBuf::from(p),
    };
    if given.exists() || given.is_absolute() {
        return given;
    }
    crate::ghost::replay_dir().map(|d| d.join(p)).filter(|f| f.exists()).unwrap_or(given)
}

/// `Audio on|off Doppler on|off` (#153, 20-cine "Audio"): the render's sound, heard by the camera on air.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AudioDef {
    pub on: bool,
    pub doppler: bool,
}

impl Default for AudioDef {
    fn default() -> Self {
        AudioDef { on: true, doppler: true }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Sequence {
    pub race: RaceDef,
    pub audio: AudioDef,
    pub size: (u32, u32),
    pub fps: u32,
    /// `Output Letterbox A`: the picture shows an A:1 window, black bars above and below (None = the whole frame)
    pub letterbox: Option<f32>,
    pub end: Option<f32>,
    pub cameras: Vec<CineCam>,
    pub shots: Vec<Shot>,
}

impl Default for RaceDef {
    fn default() -> Self {
        RaceDef { track: 1, cars: (1..=8).collect(), difficulty: Difficulty::Medium, laps: None, catchup: true, start_go: false, recording: None }
    }
}

pub(crate) const DIFFICULTIES: [(&str, Difficulty); 4] = [("easy", Difficulty::Easy), ("medium", Difficulty::Medium), ("hard", Difficulty::Hard), ("beginner", Difficulty::Beginner)];
pub(crate) const POSE_MODES: [(&str, CamMode); 5] = [("chase", CamMode::Chase), ("far", CamMode::FarChase), ("incar", CamMode::InCar), ("bumper", CamMode::Bumper), ("tv", CamMode::Tv)];

// ---------------------------------------------------------------- parsing

type Fields<'a> = HashMap<String, Vec<&'a str>>;

/// Split `words` at the words that are `keys` (case-insensitive); the last occurrence of a key wins.
fn fields<'a>(words: &[&'a str], keys: &[&str]) -> Result<Fields<'a>, String> {
    let mut out: Fields = HashMap::new();
    let mut cur: Option<String> = None;
    for &w in words {
        let lw = w.to_ascii_lowercase();
        if keys.contains(&lw.as_str()) {
            out.insert(lw.clone(), Vec::new());
            cur = Some(lw);
        } else if let Some(k) = &cur {
            out.get_mut(k).unwrap().push(w);
        } else {
            return Err(format!("expected a key ({}), got `{w}`", keys.join(", ")));
        }
    }
    Ok(out)
}

fn nums(v: &[&str], n: usize, key: &str) -> Result<Vec<f32>, String> {
    if v.len() != n {
        return Err(format!("{key} needs {n} number{}", if n == 1 { "" } else { "s" }));
    }
    v.iter().map(|x| x.parse::<f32>().ok().filter(|f| f.is_finite()).ok_or(format!("{key}: bad number `{x}`"))).collect()
}

fn vec3(v: &[&str], key: &str) -> Result<Vec3, String> {
    let n = nums(v, 3, key)?;
    Ok(Vec3::new(n[0], n[1], n[2]))
}

fn flag(v: &[&str], key: &str) -> Result<bool, String> {
    match v {
        ["on"] | ["yes"] | ["true"] => Ok(true),
        ["off"] | ["no"] | ["false"] => Ok(false),
        _ => Err(format!("{key} on|off")),
    }
}

fn target(v: &[&str]) -> Result<Target, String> {
    match v {
        ["leader"] => Ok(Target::Leader),
        ["nearest"] => Ok(Target::Nearest),
        ["player"] => Ok(Target::Player),
        ["auto"] => Ok(Target::Auto),
        ["car", n] => n.parse().map(Target::Car).map_err(|_| format!("Target car N: bad slot `{n}`")),
        _ => Err("Target car N | leader | nearest | player | auto".into()),
    }
}

fn focus_on(v: &[&str]) -> Result<FocusOn, String> {
    const USE: &str = "Focus D (metres) | own | car N | leader | nearest | player | auto";
    match v {
        ["own"] => Ok(FocusOn::Own),
        [d] if d.parse::<f32>().is_ok() => match d.parse::<f32>() {
            Ok(d) if d.is_finite() && d >= FOCUS_MIN => Ok(FocusOn::Dist(d)),
            _ => Err(format!("Focus distance is at least {FOCUS_MIN} m")),
        },
        _ => target(v).map(FocusOn::Car).map_err(|_| USE.into()),
    }
}

/// Take `Aperture N` and `Focus D | TARGET` (any camera kind, #421) out of a camera line's words.
fn take_lens<'a>(words: &[&'a str]) -> Result<(Vec<&'a str>, Lens), String> {
    let (mut rest, mut lens, mut i) = (Vec::new(), Lens::default(), 0);
    while i < words.len() {
        match words[i].to_ascii_lowercase().as_str() {
            "aperture" => {
                let n = nums(words.get(i + 1..i + 2).ok_or("Aperture N")?, 1, "Aperture")?[0];
                if !(APERTURE_RANGE.0..=APERTURE_RANGE.1).contains(&n) {
                    return Err(format!("Aperture is f/{}..f/{}", APERTURE_RANGE.0, APERTURE_RANGE.1));
                }
                lens.aperture = Some(n);
                i += 2;
            }
            "focus" => {
                // `car N` is two words, every other value one
                let n = if words.get(i + 1).is_some_and(|w| w.eq_ignore_ascii_case("car")) { 2 } else { 1 };
                lens.focus = focus_on(words.get(i + 1..i + 1 + n).ok_or("Focus D | TARGET")?)?;
                i += 1 + n;
            }
            _ => {
                rest.push(words[i]);
                i += 1;
            }
        }
    }
    Ok((rest, lens))
}

fn follow(v: &[&str]) -> Result<Follow, String> {
    match v {
        ["car", n] => n.parse().map(Follow::Car).map_err(|_| format!("Follow car N: bad slot `{n}`")),
        [w] => FOLLOWS.iter().find(|f| f.0.eq_ignore_ascii_case(w)).map(|f| f.1).ok_or_else(|| "Follow battle | leader | nearest | car N | player".into()),
        _ => Err("Follow battle | leader | nearest | car N | player".into()),
    }
}

fn zoom(v: &[&str]) -> Result<Zoom, String> {
    match v {
        ["auto", r] => match r.parse::<f32>() {
            Ok(r) if r > 0.0 && r.is_finite() => Ok(Zoom::Auto(r)),
            _ => Err("Zoom auto R: R is metres > 0".into()),
        },
        [z] => match z.parse::<f32>() {
            Ok(z) if z > 0.0 && z.is_finite() => Ok(Zoom::Fixed(z)),
            _ => Err("Zoom must be > 0".into()),
        },
        _ => Err("Zoom auto R | Z".into()),
    }
}

fn parse_race(w: &[&str], r: &mut RaceDef) -> Result<(), String> {
    let f = fields(w, &["track", "cars", "difficulty", "laps", "catchup", "start"])?;
    if let Some(v) = f.get("track") {
        r.track = nums(v, 1, "Track")?[0] as u32;
        if !(1..=8).contains(&r.track) {
            return Err("Track is 1..8".into());
        }
    }
    if let Some(v) = f.get("cars") {
        let cars: Result<Vec<u32>, _> = v.iter().map(|x| x.parse::<u32>().ok().filter(|c| (1..=10).contains(c)).ok_or(format!("Cars: `{x}` is not a car 1..10"))).collect();
        r.cars = cars?;
        if r.cars.is_empty() || r.cars.len() > 8 {
            return Err("Cars needs 1..8 cars".into());
        }
    }
    if let Some(v) = f.get("difficulty") {
        r.difficulty = DIFFICULTIES.iter().find(|d| v.len() == 1 && d.0.eq_ignore_ascii_case(v[0])).map(|d| d.1).ok_or("Difficulty easy|medium|hard|beginner")?;
    }
    if let Some(v) = f.get("laps") {
        r.laps = Some(nums(v, 1, "Laps")?[0] as u32).filter(|&l| l > 0);
    }
    if let Some(v) = f.get("catchup") {
        r.catchup = flag(v, "Catchup")?;
    }
    if let Some(v) = f.get("start") {
        r.start_go = match v.as_slice() {
            ["go"] => true,
            ["grid"] => false,
            _ => return Err("Start grid|go".into()),
        };
    }
    Ok(())
}

/// `Audio on|off Doppler on|off`: the first word switches the sound, `Doppler` the pitch shift.
fn parse_audio(w: &[&str], a: &mut AudioDef) -> Result<(), String> {
    let (on, rest) = match w.split_first() {
        Some((&x, rest)) if !x.eq_ignore_ascii_case("doppler") => (Some(flag(&[x], "Audio")?), rest),
        _ => (None, w),
    };
    let f = fields(rest, &["doppler"])?;
    if let Some(on) = on {
        a.on = on;
    }
    if let Some(v) = f.get("doppler") {
        a.doppler = flag(v, "Doppler")?;
    }
    Ok(())
}

fn track_fields(f: &Fields, t: &[&str]) -> Result<(Target, f32, f32), String> {
    Ok((
        target(t)?,
        f.get("lag").map_or(Ok(0.0), |v| nums(v, 1, "Lag").map(|n| n[0].max(0.0)))?,
        f.get("lead").map_or(Ok(0.0), |v| nums(v, 1, "Lead").map(|n| n[0]))?,
    ))
}

/// A camera line; the bool is false for a path camera without `Aim` / `Target` (its aim is settled once its keys are known).
fn parse_camera(w: &[&str]) -> Result<(CineCam, bool), String> {
    let [name, kind, rest @ ..] = w else { return Err("Camera NAME KIND ...".into()) };
    let (rest, lens) = take_lens(rest)?;
    let rest = rest.as_slice();
    let kind = kind.to_ascii_lowercase();
    let mut aim_given = true;
    let kind = match kind.as_str() {
        "static" => {
            let f = fields(rest, &["eye", "at", "target", "zoom", "lag", "lead", "clear"])?;
            let eye = vec3(f.get("eye").ok_or("static needs Eye x y z")?, "Eye")?;
            let aim = match (f.get("at"), f.get("target")) {
                (Some(a), None) => Aim::At(vec3(a, "At")?),
                (None, Some(t)) => {
                    let (target, lag, lead) = track_fields(&f, t)?;
                    Aim::Track { target, lag, lead }
                }
                _ => return Err("static needs At x y z or Target ...".into()),
            };
            CamKind::Static {
                eye,
                aim,
                zoom: f.get("zoom").map_or(Ok(Zoom::Fixed(1.0)), |v| zoom(v))?,
                clear: f.get("clear").map_or(Ok(false), |v| flag(v, "Clear"))?,
            }
        }
        "mounted" => {
            let f = fields(rest, &["target", "offset", "angles", "zoom", "hide"])?;
            let z = f.get("zoom").map_or(Ok(Zoom::Fixed(1.0)), |v| zoom(v))?;
            let Zoom::Fixed(zoom) = z else { return Err("mounted: Zoom is a number".into()) };
            CamKind::Mounted {
                target: target(f.get("target").ok_or("mounted needs Target")?)?,
                offset: f.get("offset").map_or(Ok(Vec3::ZERO), |v| vec3(v, "Offset"))?,
                angles: f.get("angles").map_or(Ok(Vec3::ZERO), |v| vec3(v, "Angles"))?,
                zoom,
                hide: f.get("hide").map_or(Ok(false), |v| flag(v, "Hide"))?,
            }
        }
        "path" => {
            let f = fields(rest, &["drive", "aim", "target", "zoom", "lag", "lead", "clear"])?;
            let drive = match f.get("drive").map(|v| v.as_slice()) {
                None | Some(["time"]) => Drive::Time,
                Some(v) => match target(v)? {
                    Target::Nearest => return Err("Drive time | car N | leader | player".into()),
                    t => Drive::Car(t),
                },
            };
            let aim = match (f.get("aim").map(|v| v.as_slice()), f.get("target")) {
                (None, Some(t)) | (Some(["target"]), Some(t)) => {
                    let (target, lag, lead) = track_fields(&f, t)?;
                    PathAim::Track { target, lag, lead }
                }
                (Some(["keys"]), None) => PathAim::Keys,
                (Some(["tangent"]), None) => PathAim::Tangent,
                (None, None) => {
                    aim_given = false;
                    PathAim::Tangent
                }
                _ => return Err("path: Aim keys | Aim tangent | Target T".into()),
            };
            CamKind::Path {
                drive,
                aim,
                zoom: f.get("zoom").map_or(Ok(Zoom::Fixed(1.0)), |v| zoom(v))?,
                clear: f.get("clear").map_or(Ok(false), |v| flag(v, "Clear"))?,
            }
        }
        "orbit" => {
            let f = fields(rest, &["target", "radius", "height", "speed", "start", "lag", "zoom", "clear"])?;
            let num = |k: &str, name: &str, d: f32| f.get(k).map_or(Ok(d), |v| nums(v, 1, name).map(|n| n[0]));
            let radius = num("radius", "Radius", ORBIT_RADIUS)?;
            if radius <= 0.0 {
                return Err("Radius must be > 0".into());
            }
            let lag = num("lag", "Lag", ORBIT_LAG)?;
            if lag < 0.0 {
                return Err("Lag must be >= 0".into());
            }
            CamKind::Orbit {
                target: target(f.get("target").ok_or("orbit needs Target")?)?,
                radius,
                height: num("height", "Height", ORBIT_HEIGHT)?,
                speed: num("speed", "Speed", ORBIT_SPEED)?,
                start: num("start", "Start", 0.0)?,
                lag,
                zoom: f.get("zoom").map_or(Ok(Zoom::Fixed(1.0)), |v| zoom(v))?,
                clear: f.get("clear").map_or(Ok(false), |v| flag(v, "Clear"))?,
            }
        }
        k => {
            let mode = POSE_MODES.iter().find(|m| m.0 == k).map(|m| m.1).ok_or(format!("camera kind `{k}`: static, mounted, path, orbit, chase, far, incar, bumper or tv"))?;
            let f = fields(rest, &["target"])?;
            let t = target(f.get("target").ok_or("needs Target")?)?;
            if t == Target::Nearest {
                return Err("Target nearest is for static cameras".into());
            }
            CamKind::Pose { mode, target: t }
        }
    };
    if let CamKind::Mounted { target: Target::Nearest, .. } | CamKind::Orbit { target: Target::Nearest, .. } | CamKind::Path { aim: PathAim::Track { target: Target::Nearest, .. }, .. } = kind {
        return Err("Target nearest is for static cameras".into());
    }
    Ok((CineCam { name: name.to_string(), kind, keys: Vec::new(), shakes: Vec::new(), lens }, aim_given))
}

/// `Key NAME T ...`: the camera name and its key.
fn parse_key(w: &[&str]) -> Result<(String, CamKey), String> {
    let [name, at, rest @ ..] = w else { return Err("Key CAMERA T ...".into()) };
    let at = nums(&[at], 1, "Key time")?[0];
    let f = fields(rest, &["eye", "angles", "fov", "focus", "tcb", "linear"])?;
    let mut key = CamKey { at, eye: None, angles: None, fov: None, focus: None, tcb: [0.0; 3], linear: false };
    if let Some(v) = f.get("eye") {
        key.eye = Some(vec3(v, "Eye")?);
    }
    if let Some(v) = f.get("angles") {
        key.angles = Some(vec3(v, "Angles")?);
    }
    if let Some(v) = f.get("fov") {
        let fov = nums(v, 1, "Fov")?[0];
        if !(FOV_RANGE.0..=FOV_RANGE.1).contains(&fov) {
            return Err(format!("Fov is {}..{} degrees", FOV_RANGE.0, FOV_RANGE.1));
        }
        key.fov = Some(fov);
    }
    if let Some(v) = f.get("focus") {
        key.focus = Some(focus_on(v)?);
    }
    if let Some(v) = f.get("tcb") {
        let n = nums(v, 3, "Tcb")?;
        if n.iter().any(|x| x.abs() > 1.0) {
            return Err("Tcb values are -1..1".into());
        }
        key.tcb = [n[0], n[1], n[2]];
    }
    if let Some(v) = f.get("linear") {
        key.linear = flag(v, "Linear")?;
    }
    if key.eye.is_none() && key.angles.is_none() && key.fov.is_none() && key.focus.is_none() {
        return Err("Key needs Eye, Angles, Fov or Focus".into());
    }
    Ok((name.to_string(), key))
}

/// `Shake NAME KIND A [Radius R]`.
fn parse_shake(w: &[&str]) -> Result<(String, Shake), String> {
    let [name, kind, amp, rest @ ..] = w else { return Err("Shake CAMERA proximity|handheld|impact A [Radius R]".into()) };
    let kind = SHAKE_KINDS.iter().find(|k| k.0.eq_ignore_ascii_case(kind)).map(|k| k.1).ok_or("Shake kind proximity | handheld | impact")?;
    let amp = nums(&[amp], 1, "Shake amplitude")?[0];
    if amp < 0.0 {
        return Err("Shake amplitude is >= 0 degrees".into());
    }
    let f = fields(rest, &["radius"])?;
    let radius = f.get("radius").map_or(Ok(kind.default_radius()), |v| nums(v, 1, "Radius").map(|n| n[0]))?;
    if radius <= 0.0 {
        return Err("Radius must be > 0".into());
    }
    Ok((name.to_string(), Shake { kind, amp, radius }))
}

/// A `Shot` line: its time, camera name (`auto`: the director picks), the auto pool's camera names, the shot.
fn parse_shot(w: &[&str]) -> Result<(f32, String, Vec<String>, Shot), String> {
    let [t, cam, rest @ ..] = w else { return Err("Shot T CAMERA|auto [Speed S] [Pitch keep|tape] [Mode classic|remaster] [FadeIn s] [FadeOut s] [Dissolve s]".into()) };
    let t = match t.parse::<f32>() {
        Ok(t) if t >= 0.0 && t.is_finite() => t,
        _ => return Err(format!("Shot: bad time `{t}`")),
    };
    let is_auto = cam.eq_ignore_ascii_case("auto");
    let keys: &[&str] = if is_auto { &["speed", "pitch", "mode", "fadein", "fadeout", "dissolve", "cams", "follow", "min"] } else { &["speed", "pitch", "mode", "fadein", "fadeout", "dissolve"] };
    let f = fields(rest, keys)?;
    let mut shot = Shot::new(t, 0);
    let mut pool = Vec::new();
    if is_auto {
        let mut a = Auto::default();
        if let Some(v) = f.get("follow") {
            a.follow = follow(v)?;
        }
        if let Some(v) = f.get("min") {
            a.min = nums(v, 1, "Min")?[0];
            if a.min < 0.0 {
                return Err("Min is >= 0 seconds".into());
            }
        }
        if let Some(v) = f.get("cams") {
            pool = v.iter().map(|s| s.to_string()).collect();
        }
        shot.auto = Some(a);
    }
    if let Some(v) = f.get("speed") {
        shot.speed = nums(v, 1, "Speed")?[0];
        if !(SPEED_RANGE.0..=SPEED_RANGE.1).contains(&shot.speed) {
            return Err(format!("Speed is {}..{}", SPEED_RANGE.0, SPEED_RANGE.1));
        }
    }
    if let Some(v) = f.get("pitch") {
        shot.pitch = match v.as_slice() {
            [w] => PITCHES.iter().find(|p| p.0.eq_ignore_ascii_case(w)).map(|p| p.1),
            _ => None,
        }
        .ok_or("Pitch keep | tape")?;
    }
    if let Some(v) = f.get("mode") {
        shot.mode = Some(match v.as_slice() {
            [w] => RenderMode::parse(w),
            _ => None,
        }
        .ok_or("Mode classic | remaster")?);
    }
    for (k, out) in [("fadein", &mut shot.fade_in), ("fadeout", &mut shot.fade_out), ("dissolve", &mut shot.dissolve)] {
        if let Some(v) = f.get(k) {
            *out = nums(v, 1, "Fade")?[0];
            if *out < 0.0 {
                return Err("a fade or dissolve is >= 0 seconds".into());
            }
        }
    }
    Ok((t, cam.to_string(), pool, shot))
}

/// Parse a `.cine` text. Bad lines are skipped and returned as warnings; an unusable sequence is an Err.
pub fn parse(text: &str) -> Result<(Sequence, Vec<String>), String> {
    let mut seq = Sequence { race: RaceDef::default(), audio: AudioDef::default(), size: (1920, 1080), fps: 60, letterbox: None, end: None, cameras: Vec::new(), shots: Vec::new() };
    let mut warn = Vec::new();
    let mut shot_lines = Vec::new();
    let mut key_lines = Vec::new();
    let mut shake_lines = Vec::new();
    let mut aim_open = Vec::new();
    for (i, line) in text.lines().enumerate() {
        let words: Vec<&str> = line.split('#').next().unwrap_or("").split_whitespace().collect();
        let Some((&head, rest)) = words.split_first() else { continue };
        let res: Result<(), String> = match head.to_ascii_lowercase().as_str() {
            "race" => parse_race(rest, &mut seq.race),
            // the rest of the line as written, so a path may hold spaces
            "recording" => match line.split('#').next().unwrap_or("").trim()[head.len()..].trim() {
                "" => Err("Recording FILE.mhr".into()),
                p => {
                    seq.race.recording = Some(p.to_string());
                    Ok(())
                }
            },
            "output" => fields(rest, &["size", "fps", "letterbox"]).and_then(|f| {
                if let Some(v) = f.get("size") {
                    let n = nums(v, 2, "Size")?;
                    seq.size = (n[0] as u32, n[1] as u32);
                    if seq.size.0 == 0 || seq.size.1 == 0 {
                        return Err("Size must be > 0".into());
                    }
                }
                if let Some(v) = f.get("fps") {
                    seq.fps = nums(v, 1, "Fps")?[0] as u32;
                    if !(1..=240).contains(&seq.fps) {
                        return Err("Fps is 1..240".into());
                    }
                }
                if let Some(v) = f.get("letterbox") {
                    seq.letterbox = if v.as_slice() == ["off"] {
                        None
                    } else {
                        let a = nums(v, 1, "Letterbox")?[0];
                        if !(LETTERBOX_RANGE.0..=LETTERBOX_RANGE.1).contains(&a) {
                            return Err(format!("Letterbox is an aspect {}..{} or off", LETTERBOX_RANGE.0, LETTERBOX_RANGE.1));
                        }
                        Some(a)
                    };
                }
                Ok(())
            }),
            "audio" => parse_audio(rest, &mut seq.audio),
            "end" => nums(rest, 1, "End").and_then(|n| {
                if n[0] <= 0.0 {
                    return Err("End must be > 0".into());
                }
                seq.end = Some(n[0]);
                Ok(())
            }),
            "camera" => parse_camera(rest).and_then(|(c, aim_given)| {
                if c.name.eq_ignore_ascii_case("auto") {
                    return Err("`auto` is not a camera name (Shot T auto lets the director pick)".into());
                }
                if seq.cameras.iter().any(|o| o.name == c.name) {
                    return Err(format!("camera {} is defined twice", c.name));
                }
                if !aim_given {
                    aim_open.push(c.name.clone());
                }
                seq.cameras.push(c);
                Ok(())
            }),
            "key" => parse_key(rest).map(|k| key_lines.push((i + 1, k))),
            "shake" => parse_shake(rest).map(|s| shake_lines.push((i + 1, s))),
            "shot" => parse_shot(rest).map(|(t, name, pool, shot)| shot_lines.push((i + 1, t, name, pool, shot))),
            h => Err(format!("unknown line `{h}`")),
        };
        if let Err(e) = res {
            warn.push(format!("line {}: {e} (skipped)", i + 1));
        }
    }
    // keys and shakes name their camera: attached once all are known
    for (line, (name, mut key)) in key_lines {
        let Some(c) = seq.cameras.iter_mut().find(|c| c.name == name) else {
            warn.push(format!("line {line}: Key names the unknown camera {name} (skipped)"));
            continue;
        };
        if !matches!(c.kind, CamKind::Path { .. }) && (key.eye.is_some() || key.angles.is_some()) {
            warn.push(format!("line {line}: Eye and Angles keys are for path cameras (ignored)"));
            (key.eye, key.angles) = (None, None);
            if key.fov.is_none() && key.focus.is_none() {
                continue;
            }
        }
        // a channel keyed twice at the same time keeps its first key
        if let Some(o) = c.keys.iter().find(|o| o.at == key.at) {
            let dup = (o.eye.is_some() && key.eye.is_some()) || (o.angles.is_some() && key.angles.is_some()) || (o.fov.is_some() && key.fov.is_some()) || (o.focus.is_some() && key.focus.is_some());
            if dup {
                warn.push(format!("line {line}: camera {name} already has a key at {} (skipped)", key.at));
                continue;
            }
        }
        c.keys.push(key);
    }
    for (line, (name, shake)) in shake_lines {
        match seq.cameras.iter_mut().find(|c| c.name == name) {
            Some(c) => c.shakes.push(shake),
            None => warn.push(format!("line {line}: Shake names the unknown camera {name} (skipped)")),
        }
    }
    let mut dropped = Vec::new();
    for c in &mut seq.cameras {
        c.keys.sort_by(|a, b| a.at.total_cmp(&b.at));
        let has_angles = c.keys.iter().any(|k| k.angles.is_some());
        if let CamKind::Path { aim, .. } = &mut c.kind {
            if aim_open.contains(&c.name) && has_angles {
                *aim = PathAim::Keys;
            }
            if *aim == PathAim::Keys && !has_angles {
                warn.push(format!("camera {}: Aim keys without Angles keys: looks along the path", c.name));
                *aim = PathAim::Tangent;
            }
            if !c.keys.iter().any(|k| k.eye.is_some()) {
                warn.push(format!("camera {}: a path needs Eye keys (camera skipped)", c.name));
                dropped.push(c.name.clone());
            }
        }
    }
    seq.cameras.retain(|c| !dropped.contains(&c.name));
    // shots refer to cameras by name: resolved once all are known
    for (line, _, name, pool, mut shot) in shot_lines {
        if let Some(a) = shot.auto.as_mut() {
            for p in pool {
                match seq.cameras.iter().position(|c| c.name == p) {
                    Some(i) if !a.pool.contains(&i) => a.pool.push(i),
                    Some(_) => {}
                    None => warn.push(format!("line {line}: Cams names the unknown camera {p} (ignored)")),
                }
            }
            shot.cam = auto_pool(&seq, a).first().copied().unwrap_or(0);
            seq.shots.push(shot);
            continue;
        }
        match seq.cameras.iter().position(|c| c.name == name) {
            Some(cam) => {
                shot.cam = cam;
                seq.shots.push(shot);
            }
            None => warn.push(format!("line {line}: Shot names the unknown camera {name} (skipped)")),
        }
    }
    if seq.cameras.is_empty() {
        return Err("the sequence has no camera".into());
    }
    seq.shots.sort_by(|a, b| a.t.total_cmp(&b.t));
    if seq.shots.is_empty() {
        warn.push(format!("no Shot lines: camera {} is on air throughout", seq.cameras[0].name));
        seq.shots.push(Shot::new(0.0, 0));
    }
    // slots named by a Target must exist
    let n = seq.race.cars.len();
    for c in &seq.cameras {
        let t = match &c.kind {
            CamKind::Static { aim: Aim::Track { target, .. }, .. } => Some(*target),
            CamKind::Mounted { target, .. } | CamKind::Pose { target, .. } | CamKind::Orbit { target, .. } => Some(*target),
            CamKind::Path { aim: PathAim::Track { target, .. }, .. } | CamKind::Path { drive: Drive::Car(target), .. } => Some(*target),
            _ => None,
        };
        if let Some(Target::Car(s)) = t {
            if s >= n {
                warn.push(format!("camera {}: car {s} is not on the grid ({n} cars): the last slot is used", c.name));
            }
        }
    }
    Ok((seq, warn))
}

fn target_text(t: Target) -> String {
    match t {
        Target::Car(n) => format!("car {n}"),
        Target::Leader => "leader".into(),
        Target::Nearest => "nearest".into(),
        Target::Player => "player".into(),
        Target::Auto => "auto".into(),
    }
}

fn focus_text(f: FocusOn) -> String {
    match f {
        FocusOn::Own => "own".into(),
        FocusOn::Dist(d) => d.to_string(),
        FocusOn::Car(t) => target_text(t),
    }
}

/// A camera line's `Aperture` / `Focus` words (#421; none at their defaults).
fn lens_text(l: &Lens) -> String {
    let mut o = String::new();
    if let Some(n) = l.aperture {
        o += &format!(" Aperture {n}");
    }
    if l.focus != FocusOn::Own {
        o += &format!(" Focus {}", focus_text(l.focus));
    }
    o
}

fn follow_text(f: Follow) -> String {
    match f {
        Follow::Car(n) => format!("car {n}"),
        f => FOLLOWS.iter().find(|x| x.1 == f).map_or("battle", |x| x.0).into(),
    }
}

fn v3(v: Vec3) -> String {
    format!("{} {} {}", v.x, v.y, v.z)
}

/// The file text of a sequence (`parse` reads it back).
pub fn to_text(s: &Sequence) -> String {
    let r = &s.race;
    let mut o = format!("{HEADER}\nRace Track {} Cars {}", r.track, r.cars.iter().map(u32::to_string).collect::<Vec<_>>().join(" "));
    o += &format!(" Difficulty {}", DIFFICULTIES.iter().find(|d| d.1 == r.difficulty).map_or("medium", |d| d.0));
    if let Some(l) = r.laps {
        o += &format!(" Laps {l}");
    }
    o += &format!(" Catchup {} Start {}\n", if r.catchup { "on" } else { "off" }, if r.start_go { "go" } else { "grid" });
    if let Some(p) = &r.recording {
        o += &format!("Recording {p}\n");
    }
    o += &format!("Output Size {} {} Fps {}", s.size.0, s.size.1, s.fps);
    if let Some(a) = s.letterbox {
        o += &format!(" Letterbox {a}");
    }
    o += "\n";
    let onoff = |b: bool| if b { "on" } else { "off" };
    o += &format!("Audio {} Doppler {}\n", onoff(s.audio.on), onoff(s.audio.doppler));
    if let Some(e) = s.end {
        o += &format!("End {e}\n");
    }
    for c in &s.cameras {
        o += &format!("Camera {} ", c.name);
        match &c.kind {
            CamKind::Static { eye, aim, zoom, clear } => {
                o += &format!("static Eye {}", v3(*eye));
                match aim {
                    Aim::At(a) => o += &format!(" At {}", v3(*a)),
                    Aim::Track { target, lag, lead } => o += &format!(" Target {} Lag {lag} Lead {lead}", target_text(*target)),
                }
                match zoom {
                    Zoom::Fixed(z) => o += &format!(" Zoom {z}"),
                    Zoom::Auto(r) => o += &format!(" Zoom auto {r}"),
                }
                o += &format!(" Clear {}", if *clear { "on" } else { "off" });
            }
            CamKind::Mounted { target, offset, angles, zoom, hide } => {
                o += &format!("mounted Target {} Offset {} Angles {} Zoom {zoom} Hide {}", target_text(*target), v3(*offset), v3(*angles), if *hide { "on" } else { "off" });
            }
            CamKind::Orbit { target, radius, height, speed, start, lag, zoom, clear } => {
                o += &format!("orbit Target {} Radius {radius} Height {height} Speed {speed} Start {start} Lag {lag}", target_text(*target));
                match zoom {
                    Zoom::Fixed(z) => o += &format!(" Zoom {z}"),
                    Zoom::Auto(r) => o += &format!(" Zoom auto {r}"),
                }
                o += &format!(" Clear {}", if *clear { "on" } else { "off" });
            }
            CamKind::Pose { mode, target } => {
                o += &format!("{} Target {}", POSE_MODES.iter().find(|m| m.1 == *mode).map_or("chase", |m| m.0), target_text(*target));
            }
            CamKind::Path { drive, aim, zoom, clear } => {
                o += "path Drive ";
                o += &match drive {
                    Drive::Time => "time".into(),
                    Drive::Car(t) => target_text(*t),
                };
                match aim {
                    PathAim::Keys => o += " Aim keys",
                    PathAim::Tangent => o += " Aim tangent",
                    PathAim::Track { target, lag, lead } => o += &format!(" Target {} Lag {lag} Lead {lead}", target_text(*target)),
                }
                match zoom {
                    Zoom::Fixed(z) => o += &format!(" Zoom {z}"),
                    Zoom::Auto(r) => o += &format!(" Zoom auto {r}"),
                }
                o += &format!(" Clear {}", if *clear { "on" } else { "off" });
            }
        }
        o += &lens_text(&c.lens);
        o += "\n";
        for k in &c.keys {
            o += &format!("Key {} {}", c.name, k.at);
            if let Some(e) = k.eye {
                o += &format!(" Eye {}", v3(e));
            }
            if let Some(a) = k.angles {
                o += &format!(" Angles {}", v3(a));
            }
            if let Some(f) = k.fov {
                o += &format!(" Fov {f}");
            }
            if let Some(f) = k.focus {
                o += &format!(" Focus {}", focus_text(f));
            }
            if k.tcb != [0.0; 3] {
                o += &format!(" Tcb {} {} {}", k.tcb[0], k.tcb[1], k.tcb[2]);
            }
            if k.linear {
                o += " Linear on";
            }
            o += "\n";
        }
        for sh in &c.shakes {
            let kind = SHAKE_KINDS.iter().find(|k| k.1 == sh.kind).map_or("handheld", |k| k.0);
            o += &format!("Shake {} {kind} {}", c.name, sh.amp);
            if sh.radius != sh.kind.default_radius() {
                o += &format!(" Radius {}", sh.radius);
            }
            o += "\n";
        }
    }
    for sh in &s.shots {
        match &sh.auto {
            Some(a) => {
                o += &format!("Shot {} auto", sh.t);
                if !a.pool.is_empty() {
                    o += " Cams";
                    for &i in &a.pool {
                        o += &format!(" {}", s.cameras[i].name);
                    }
                }
                o += &format!(" Follow {} Min {}", follow_text(a.follow), a.min);
            }
            None => o += &format!("Shot {} {}", sh.t, s.cameras[sh.cam].name),
        }
        if sh.speed != 1.0 {
            o += &format!(" Speed {}", sh.speed);
        }
        if sh.pitch == Pitch::Tape {
            o += " Pitch tape";
        }
        if let Some(m) = sh.mode {
            o += &format!(" Mode {}", m.name().to_ascii_lowercase());
        }
        if sh.fade_in > 0.0 {
            o += &format!(" FadeIn {}", sh.fade_in);
        }
        if sh.fade_out > 0.0 {
            o += &format!(" FadeOut {}", sh.fade_out);
        }
        if sh.dissolve > 0.0 {
            o += &format!(" Dissolve {}", sh.dissolve);
        }
        o += "\n";
    }
    o
}

// ---------------------------------------------------------------- director

/// Index into `seq.shots` of the shot at second `t`: the last one at or before it (before the first shot, the first).
pub fn shot_at(seq: &Sequence, t: f32) -> usize {
    seq.shots.iter().rposition(|s| s.t <= t).unwrap_or(0)
}

/// Index of the camera on air at second `t`: the last shot at or before it (before the first shot, the first).
pub fn on_air(seq: &Sequence, t: f32) -> usize {
    seq.shots[shot_at(seq, t)].cam
}

/// Seconds since camera `cam`'s latest shot started (`t` itself when it has not had one yet): the clock of
/// its `Drive time` path and of its Fov keys.
pub fn since_shot(seq: &Sequence, cam: usize, t: f32) -> f32 {
    seq.shots.iter().rev().find(|s| s.auto.is_none() && s.cam == cam && s.t <= t).map_or(t, |s| t - s.t)
}

/// The mixer's tape speed at `t` (#426): the shot's `Speed` when its `Pitch` is `tape`, else 1.
pub fn audio_rate_at(seq: &Sequence, t: f32) -> f32 {
    let s = &seq.shots[shot_at(seq, t)];
    if s.pitch == Pitch::Tape { s.speed } else { 1.0 }
}

/// The graphics mode at `t` (#427): the shot's `Mode`, else `base` (the render's own).
pub fn mode_at(seq: &Sequence, t: f32, base: RenderMode) -> RenderMode {
    seq.shots[shot_at(seq, t)].mode.unwrap_or(base)
}

/// Whether any shot asks for mode `m` (#427).
pub fn uses_mode(seq: &Sequence, m: RenderMode) -> bool {
    seq.shots.iter().any(|s| s.mode == Some(m))
}

/// Race seconds per video second at `t` (the shot's `Speed`).
pub fn speed_at(seq: &Sequence, t: f32) -> f32 {
    seq.shots[shot_at(seq, t)].speed
}

/// Race seconds at video second `t`: the shots' `Speed` summed over the video (the editor's clock, #152). The
/// render steps the race once per frame, so its race time agrees to within a frame at a shot boundary [APPROX].
pub fn race_secs(seq: &Sequence, t: f32) -> f32 {
    let (mut r, mut at, mut speed) = (0.0, 0.0, seq.shots.first().map_or(1.0, |s| s.speed));
    for s in seq.shots.iter().filter(|s| s.t > 0.0) {
        if s.t >= t {
            break;
        }
        r += (s.t - at) * speed;
        (at, speed) = (s.t, s.speed);
    }
    r + (t - at).max(0.0) * speed
}

/// Video second at race second `r`: the inverse of `race_secs`.
pub fn video_secs(seq: &Sequence, r: f32) -> f32 {
    let (mut done, mut at, mut speed) = (0.0, 0.0, seq.shots.first().map_or(1.0, |s| s.speed));
    for s in seq.shots.iter().filter(|s| s.t > 0.0) {
        let span = (s.t - at) * speed;
        if done + span >= r {
            break;
        }
        done += span;
        (at, speed) = (s.t, s.speed);
    }
    at + (r - done).max(0.0) / speed
}

/// Brightness at `t` from the shot's fades: 1 = as drawn, 0 = black. `end` is the end of the video when
/// known (the last shot fades out only then).
pub fn fade_at(seq: &Sequence, t: f32, end: Option<f32>) -> f32 {
    let i = shot_at(seq, t);
    let sh = &seq.shots[i];
    let start = if sh.t <= t { sh.t } else { 0.0 };
    let mut f: f32 = 1.0;
    if sh.fade_in > 0.0 {
        f = f.min((t - start) / sh.fade_in);
    }
    if let Some(e) = seq.shots.get(i + 1).map(|n| n.t).or(end).filter(|_| sh.fade_out > 0.0) {
        f = f.min((e - t) / sh.fade_out);
    }
    f.clamp(0.0, 1.0)
}

/// Rows of each black bar of a letterboxed `h` pixel tall frame `w` wide showing an `aspect`:1 window (0 = the window is not
/// shorter than the frame). Whole rows; an odd remainder goes to the picture.
pub fn letterbox_bar(w: u32, h: u32, aspect: f32) -> u32 {
    let win = (w as f32 / aspect.max(1e-3)).round() as u32;
    h.saturating_sub(win) / 2
}

/// The editor's preview frame (#438 design choice, the passepartout of a 3D package's camera view): the largest `video`-aspect
/// rectangle `[x, y, w, h]` centred on the free area of a `win` window between the panels' `margins` (top, right, bottom,
/// left), `gap` more clear of a panel that is there, all in the same units. `preview_cam` zooms the view out by `w / win.0`
/// (the field spans the window's width) and shifts its lens to the frame's centre (#441). Never smaller than one unit.
pub fn preview_frame(win: (f32, f32), margins: (f32, f32, f32, f32), video: (u32, u32), gap: f32) -> [f32; 4] {
    let m = |v: f32| if v > 0.0 { v + gap } else { 0.0 };
    let (top, right, bottom, left) = (m(margins.0), m(margins.1), m(margins.2), m(margins.3));
    let hw = ((win.0 - left - right) * 0.5).max(0.5);
    let hh = ((win.1 - top - bottom) * 0.5).max(0.5);
    let (cx, cy) = (left + hw, top + hh);
    let aspect = video.0.max(1) as f32 / video.1.max(1) as f32;
    let w = (2.0 * hw).min(2.0 * hh * aspect);
    let h = w / aspect;
    [cx - w * 0.5, cy - h * 0.5, w, h]
}

/// The camera that shows `cam`'s view in the preview frame `rect` (`preview_frame`) of a `win` window: zoomed out so its
/// horizontal field spans the frame, its lens shifted so the view axis lands on the frame's centre (#441, design choice).
pub fn preview_cam(cam: Camera, rect: [f32; 4], win: (f32, f32)) -> Camera {
    let (w, h) = (win.0.max(1.0), win.1.max(1.0));
    let (cx, cy) = (rect[0] + rect[2] * 0.5, rect[1] + rect[3] * 0.5);
    Camera { zoom: cam.zoom * rect[2] / w, shift: glam::Vec2::new(2.0 * cx / w - 1.0, 1.0 - 2.0 * cy / h), ..cam }
}

/// APPROX [design choice]: auto zoom keeps a car at about the same size on screen, zoom 1.0 at `r` metres.
pub fn auto_zoom(dist: f32, r: f32) -> f32 {
    (dist / r).clamp(1.0, ZOOM_MAX)
}

/// Far-plane multiplier that goes with a zoom (the TV camera grows it with the zoom, 0x40b564) [I].
fn far_k(zoom: f32) -> f32 {
    zoom.clamp(1.0, FAR_K_MAX)
}

// ---------------------------------------------------------------- TV camera export (#424)

/// APPROX [design choice]: the static cameras of a sequence as keys of the game's TV camera motion (`tvcam::shot`).
/// A `Target` camera becomes type 0 (eye at the station, look at the car), an `At` camera type 2 (fixed angles); `Zoom Z` a
/// fixed zoom, `Zoom auto R` the game's distance-scaled zoom (the same d / R clamp as `auto_zoom`). Lag, lead, the target car,
/// `Clear`, Fov / Focus keys, lens and shakes have no TV equivalent and are listed in the notes. `first_frame` numbers the keys.
pub fn tv_keys(seq: &Sequence, first_frame: f32) -> (Vec<MotionKey>, Vec<String>) {
    let (mut keys, mut notes) = (Vec::new(), Vec::new());
    for c in &seq.cameras {
        let CamKind::Static { eye, aim, zoom, .. } = &c.kind else { continue };
        let (kind, angles) = match aim {
            Aim::Track { .. } => (0.0, Vec3::ZERO),
            Aim::At(a) => (2.0, rot_to_euler(look_rot(*eye, *a)) / DEG),
        };
        // the far plane: the game grows it by `1 + r * f / 100` with the zoom r, the cine by `far_k`
        let (tension, cap, far) = match zoom {
            Zoom::Fixed(z) => (-*z, 0.0, 100.0 * (far_k(*z) - 1.0)),
            Zoom::Auto(r) => (1.0 / r.max(0.01), ZOOM_MAX * r, 100.0 * (FAR_K_MAX - 1.0) / FAR_K_MAX),
        };
        let mut dropped = Vec::new();
        if let Aim::Track { lag, lead, .. } = aim {
            if *lag > 0.0 || *lead > 0.0 {
                dropped.push("Lag / Lead");
            }
        }
        if !c.keys.is_empty() {
            dropped.push("keys");
        }
        if !c.shakes.is_empty() {
            dropped.push("shakes");
        }
        if c.lens.aperture.is_some() {
            dropped.push("lens");
        }
        if !dropped.is_empty() {
            notes.push(format!("{}: {} not exported", c.name, dropped.join(", ")));
        }
        keys.push(MotionKey {
            values: vec![eye.x, eye.y, eye.z, angles.x, angles.y, angles.z, 0.0, 0.0, 0.0],
            frame: first_frame + 30.0 * keys.len() as f32,
            linear: kind,
            tension,
            continuity: far,
            bias: cap,
        });
    }
    (keys, notes)
}

/// Write `remaster/tv/trackNN.mot`: the track's shipped TV stations plus the sequence's static cameras. Always rebuilt from the
/// shipped file, so exporting again replaces the earlier export. Returns the message to show.
pub fn export_tv(root: &std::path::Path, seq: &Sequence) -> Result<String, String> {
    let track = seq.race.track;
    let trk = motor_data::fs::config(root, &format!("Game/Track{track}.trk"))?;
    let mut motion = motor_sim::tvcam::shipped_motion(root, &trk).unwrap_or(Motion { version: 1, channels: 9, keys: Vec::new() });
    let shipped = motion.keys.len();
    let last = motion.keys.iter().map(|k| k.frame).fold(-30.0, f32::max);
    let (added, notes) = tv_keys(seq, last + 30.0);
    if added.is_empty() {
        return Err("no static camera to export (TV stations are fixed eyes)".into());
    }
    let n = added.len();
    motion.keys.extend(added);
    let p = motor_sim::tvcam::override_path(root, track);
    if let Some(d) = p.parent() {
        std::fs::create_dir_all(d).map_err(|e| format!("{}: {e}", d.display()))?;
    }
    std::fs::write(&p, motor_data::cfg::write_lwmo(&motion)).map_err(|e| format!("{}: {e}", p.display()))?;
    let mut msg = format!("wrote {}: {shipped} shipped + {n} cine stations", p.display());
    if !notes.is_empty() {
        msg += &format!(" ({})", notes.join("; "));
    }
    Ok(msg)
}

/// Delete the exported TV stations of `track`; Ok(false) when there were none.
pub fn remove_tv(root: &std::path::Path, track: u32) -> Result<bool, String> {
    let p = motor_sim::tvcam::override_path(root, track);
    if !p.exists() {
        return Ok(false);
    }
    std::fs::remove_file(&p).map_err(|e| format!("{}: {e}", p.display()))?;
    Ok(true)
}

/// The zoom of a horizontal field of view: 1.0 at the views' `FOV_X` (90 degrees).
pub fn fov_zoom(fov_deg: f32) -> f32 {
    (crate::render::FOV_X.get() * 0.5 * DEG).tan() / (fov_deg * 0.5 * DEG).tan()
}

/// The focus distance at key parameter `u` from `(at, metres, linear)` keys sorted by `at` (#421): interpolated in
/// 1 / distance (the travel of a focus ring), a smoothstep between neighbours (`Linear on` on the later key: straight),
/// held outside them. Not a TCB curve: the keys' distances move every frame [design choice].
pub fn focus_between(keys: &[(f32, f32, bool)], u: f32) -> f32 {
    let n = keys.iter().take_while(|k| k.0 <= u).count();
    match (n.checked_sub(1).map(|i| keys[i]), keys.get(n).copied()) {
        (Some(a), Some(b)) => {
            let s = ((u - a.0) / (b.0 - a.0).max(1e-6)).clamp(0.0, 1.0);
            let s = if b.2 { s } else { s * s * (3.0 - 2.0 * s) };
            1.0 / (1.0 / a.1 + (1.0 / b.1 - 1.0 / a.1) * s)
        }
        (Some(k), None) | (None, Some(k)) => k.1,
        (None, None) => f32::INFINITY,
    }
}

/// Where an orbit camera (#425) hangs: `radius` metres from `centre` on the horizontal circle and `height` above it, at
/// `angle_deg` from straight behind a car heading `fwd` (horizontal, unit), positive towards the car's right.
pub fn orbit_eye(centre: Vec3, fwd: Vec3, radius: f32, height: f32, angle_deg: f32) -> Vec3 {
    let right = Vec3::new(fwd.z, 0.0, -fwd.x);
    let a = angle_deg * DEG;
    centre + (right * a.sin() - fwd * a.cos()) * radius + Vec3::Y * height
}

/// One step of the cameraman's easing towards `goal` (exponential, time constant `lag` seconds).
pub fn ease(from: Vec3, goal: Vec3, lag: f32, dt: f32) -> Vec3 {
    if lag <= 0.0 {
        return goal;
    }
    from.lerp(goal, 1.0 - (-dt / lag).exp())
}

/// Rotation (columns right, up, forward) looking from `eye` at `to`, no bank; identity if they coincide.
fn look_rot(eye: Vec3, to: Vec3) -> Mat3 {
    look_dir(to - eye).unwrap_or(Mat3::IDENTITY)
}

fn look_dir(d: Vec3) -> Option<Mat3> {
    let f = d.try_normalize()?;
    let r = Vec3::new(f.z, 0.0, -f.x).try_normalize().unwrap_or(Vec3::X);
    Some(Mat3::from_cols(r, f.cross(r), f))
}

/// The keys of one channel (Eye, Angles or Fov) as a LightWave motion for `props::evaluate`; None without keys.
fn curve(keys: &[CamKey], get: impl Fn(&CamKey) -> Option<Vec<f32>>) -> Option<Motion> {
    let keys: Vec<MotionKey> = keys
        .iter()
        .filter_map(|k| {
            get(k).map(|values| MotionKey { values, frame: k.at, linear: if k.linear { 1.0 } else { 0.0 }, tension: k.tcb[0], continuity: k.tcb[1], bias: k.tcb[2] })
        })
        .collect();
    let channels = keys.first()?.values.len() as u32;
    Some(Motion { version: 1, channels, keys })
}

fn eval3(m: &Motion, u: f32) -> Vec3 {
    let v = evaluate(m, u, false);
    Vec3::new(v[0], v[1], v[2])
}

/// `n` points along a camera's Eye curve from its first key to its last (the editor's path gizmo); empty without
/// Eye keys.
pub fn path_points(c: &CineCam, n: usize) -> Vec<Vec3> {
    let Some(m) = curve(&c.keys, |k| k.eye.map(|v| v.to_array().to_vec())) else { return Vec::new() };
    let (a, b) = (m.keys[0].frame, m.keys[m.keys.len() - 1].frame);
    if n < 2 || b <= a {
        return vec![eval3(&m, a)];
    }
    (0..n).map(|i| eval3(&m, a + (b - a) * i as f32 / (n - 1) as f32)).collect()
}

/// The key parameter of a car-paced path: the car's lap arc `s`, moved by a lap (`len`) when that brings it nearer
/// the keyed span `first..last` (keys past the lap length cross the line).
pub fn lap_param(s: f32, len: f32, first: f32, last: f32) -> f32 {
    let off = |u: f32| if u < first { first - u } else if u > last { u - last } else { 0.0 };
    [s, s - len, s + len].into_iter().min_by(|a, b| off(*a).total_cmp(&off(*b))).unwrap_or(s)
}

// ---------------------------------------------------------------- shake

/// Shake noise: an integer hash of (seed, lattice point) in -1..1.
fn hash(seed: u32, i: i32) -> f32 {
    let mut x = (i as u32).wrapping_mul(0x9e37_79b9) ^ seed.wrapping_mul(0x85eb_ca6b).wrapping_add(0x27d4_eb2f);
    x ^= x >> 16;
    x = x.wrapping_mul(0x7feb_352d);
    x ^= x >> 15;
    x = x.wrapping_mul(0x846c_a68b);
    x ^= x >> 16;
    (x >> 8) as f32 / (1u32 << 23) as f32 - 1.0
}

fn value_noise(seed: u32, x: f32) -> f32 {
    let i = x.floor();
    let f = x - i;
    let f = f * f * (3.0 - 2.0 * f);
    let a = hash(seed, i as i32);
    a + (hash(seed, i as i32 + 1) - a) * f
}

/// APPROX [design choice]: three octaves of value noise at `f0`, 2 `f0`, 4 `f0` Hz, in -1..1. Time-based, no state:
/// a re-render (or a single `--at-time` frame) shakes the same.
pub fn shake_noise(seed: u32, t: f32, f0: f32) -> f32 {
    let mut sum = 0.0;
    for (o, w) in [1.0, 0.5, 0.25].into_iter().enumerate() {
        sum += w * value_noise(seed.wrapping_add(o as u32 * 0x1000), t * f0 * (1 << o) as f32);
    }
    sum / 1.75
}

/// APPROX [design choice] shake tuning: base frequencies (Hz), proximity gain (50 m/s at 3 m = full) and its
/// easing in / out (s), the impact speed (m/s) of a full kick and the kick's decay (s).
const HANDHELD_HZ: f32 = 0.5;
const PROXIMITY_HZ: f32 = 6.0;
const IMPACT_HZ: f32 = 10.0;
const PROXIMITY_K: f32 = 0.18;
const PROXIMITY_IN: f32 = 0.1;
const PROXIMITY_OUT: f32 = 0.4;
const IMPACT_FULL: f32 = 30.0;
const IMPACT_DECAY: f32 = 0.35;
/// handheld sway, metres per degree of amplitude
const HANDHELD_SWAY: f32 = 0.01;

/// The strength (0..1) a shake aims at this frame. `eye` in game space; `own` = the car the camera rides on.
fn shake_target(sh: &Shake, race: &Race, eye: Vec3, own: Option<usize>) -> f32 {
    let near = race.racers.iter().enumerate().filter_map(|(i, r)| {
        let d = cam_origin(&r.car).distance(eye);
        (d <= sh.radius).then_some((i, &r.car, d))
    });
    match sh.kind {
        ShakeKind::Handheld => 1.0,
        ShakeKind::Proximity => near.filter(|(i, ..)| Some(*i) != own).map(|(_, c, d)| c.world_vel().length() * PROXIMITY_K / (d.max(1.0) * d.max(1.0))).sum::<f32>().min(1.0),
        ShakeKind::Impact => near
            .map(|(_, c, d)| {
                let hit = c.impact.max(c.bump.map_or(0.0, |b| b.impact * 0.5));
                (hit / IMPACT_FULL).min(1.0) * (1.0 - d / sh.radius)
            })
            .fold(0.0, f32::max),
    }
}

/// Ease a shake's strength towards `goal` (proximity eases in and out, an impact kicks and decays).
fn shake_level(kind: ShakeKind, level: f32, goal: f32, dt: f32) -> f32 {
    let toward = |tc: f32| level + (goal - level) * (1.0 - (-dt / tc).exp());
    match kind {
        ShakeKind::Handheld => 1.0,
        ShakeKind::Proximity => toward(if goal > level { PROXIMITY_IN } else { PROXIMITY_OUT }),
        ShakeKind::Impact => (level * (-dt / IMPACT_DECAY).exp()).max(goal),
    }
}

/// The angle offsets (yaw, pitch, roll; degrees) and the sway (right, up; metres) of a shake at strength `level`.
pub fn shake_offsets(sh: &Shake, seed: u32, t: f32, level: f32) -> ([f32; 3], [f32; 2]) {
    let hz = match sh.kind {
        ShakeKind::Handheld => HANDHELD_HZ,
        ShakeKind::Proximity => PROXIMITY_HZ,
        ShakeKind::Impact => IMPACT_HZ,
    };
    let a = sh.amp * level;
    let n = |c: u32| shake_noise(seed.wrapping_mul(8).wrapping_add(c), t, hz);
    let ang = [a * n(0), a * n(1), 0.5 * a * n(2)];
    let sway = if sh.kind == ShakeKind::Handheld { [a * HANDHELD_SWAY * n(3), a * HANDHELD_SWAY * n(4)] } else { [0.0; 2] };
    (ang, sway)
}

// ---------------------------------------------------------------- director

enum State {
    Static { aim: Option<Vec3> },
    Mounted,
    Pose(PoseCam),
    /// the eased aim point (Target) and the last direction along the path (Tangent, held at the path's ends)
    Path { aim: Option<Vec3>, dir: Vec3 },
    /// the eased car position and horizontal heading, and the eased aim point (#425)
    Orbit { centre: Option<Vec3>, fwd: Vec3, aim: Option<Vec3> },
}

/// A camera's key curves (`Key` lines), built once.
struct Curves {
    eye: Option<Motion>,
    angles: Option<Motion>,
    fov: Option<Motion>,
    /// first and last key parameter of the eye curve
    span: (f32, f32),
}

/// Turns the race into the camera of each moment. Every camera is updated every frame, on air or not, so a cut
/// shows a camera that has already settled and a single frame (`--at-time`) has the camera pose of the same frame of a full render.
pub struct Director {
    states: Vec<State>,
    curves: Vec<Curves>,
    /// per camera, per shake: its current strength
    shake: Vec<Vec<f32>>,
    focus: usize,
    hidden: Option<usize>,
    /// per camera, as of the last `camera` call: the camera, the slot it shows, whether it hides that car (the editor's preview)
    views: Vec<Option<(Camera, usize, bool)>>,
    /// per camera, as of the last `camera` call: its lens (#421; None without an aperture)
    dofs: Vec<Option<Dof>>,
    /// the camera on air this frame (an auto shot: the director's pick)
    air: usize,
    /// the car the auto director follows (`Target auto`; outside auto shots the leader)
    follow: usize,
    /// per camera: the video second of its latest auto cut (its clock restarts there, as at a shot)
    cut_at: Vec<Option<f32>>,
    auto: AutoRun,
    /// `Follow battle`: the pair held (ahead, behind)
    battle: Option<(usize, usize)>,
    /// the camera on air at the last `camera` call, and the dissolve in progress: the camera it comes from, its start second, its length
    last_air: Option<usize>,
    dis: Option<(usize, f32, f32)>,
    /// the video second of the last `camera` call
    now: f32,
}

/// The car slot a `Target` means now (`eye`: the camera position, for `nearest`; `Target auto` = the leader here).
pub fn resolve(race: &Race, t: Target, eye: Option<Vec3>) -> usize {
    resolve_in(race, t, eye, None)
}

/// `resolve` with the auto director's followed car (`Target auto`).
fn resolve_in(race: &Race, t: Target, eye: Option<Vec3>, follow: Option<usize>) -> usize {
    let last = race.racers.len() - 1;
    let leader = || race.standings().first().copied().unwrap_or(0);
    match t {
        Target::Car(n) => n.min(last),
        Target::Player => last,
        Target::Leader => leader(),
        Target::Auto => follow.map_or_else(leader, |f| f.min(last)),
        Target::Nearest => match eye {
            Some(e) => (0..=last).min_by(|&a, &b| cam_origin(&race.racers[a].car).distance_squared(e).total_cmp(&cam_origin(&race.racers[b].car).distance_squared(e))).unwrap_or(0),
            None => race.standings().first().copied().unwrap_or(0),
        },
    }
}

/// A camera at `eye` following a car (static `Target`, path `Target`): eased aim, lead, auto zoom, clear view.
#[allow(clippy::too_many_arguments)]
fn track_cam(race: &Race, eye: Vec3, target: Target, follow: usize, lag: f32, lead: f32, zoom: Zoom, clear: bool, state: &mut Option<Vec3>, dt: f32) -> (Camera, usize) {
    let slot = resolve_in(race, target, Some(eye), Some(follow));
    let car = &race.racers[slot].car;
    // #374: the car where it is drawn
    let pos = cam_origin_at(&car.p, &race.view_pose(slot));
    let goal = pos + car.world_vel() * lead + Vec3::Y * AIM_UP;
    let at = state.map_or(goal, |p| ease(p, goal, lag, dt));
    *state = Some(at);
    let z = match zoom {
        Zoom::Fixed(z) => z,
        Zoom::Auto(r) => auto_zoom(eye.distance(at), r),
    };
    let mut shot = TvShot { eye, rot: look_rot(eye, at), zoom: z, far_k: far_k(z), kind: 0 };
    if clear {
        shot.clear_view(&race.world, pos);
    }
    (Camera::from_rot(shot.eye, shot.rot, shot.zoom, shot.far_k), slot)
}

// ---------------------------------------------------------------- auto director (#154)

motor_tune::tunable! {
    /// Auto director [design choice]: a cut needs a camera nearer the car than this share of the distance of the camera on air.
    /// range: 0.1..1
    /// area: Cine
    AUTO_SWITCH: f32 = 0.7;
    /// Auto director [design choice]: the camera on air may cut before the shot's `Min` once it has not seen the car this long.
    /// range: 0..5
    /// unit: s
    /// area: Cine
    AUTO_LOST: f32 = 0.5;
    /// Auto director [design choice]: a camera farther from the car than this does not see it.
    /// range: 10..1000
    /// unit: m
    /// area: Cine
    AUTO_SEE_MAX: f32 = 250.0;
    /// Auto director [design choice]: the share of the frame kept clear at each edge; the car must lie inside the rest.
    /// range: 0..0.4
    /// area: Cine
    AUTO_MARGIN: f32 = 0.1;
    /// `Follow battle` [design choice]: another pair takes over when its gap is under this share of the held pair's.
    /// range: 0.1..1
    /// area: Cine
    BATTLE_SWITCH: f32 = 0.6;
}

/// The auto shot in progress.
#[derive(Clone, Debug, Default)]
pub(crate) struct AutoRun {
    /// index into `seq.shots` of the auto shot
    pub shot: Option<usize>,
    /// the camera on air, the video second of the last cut, the camera cut away from
    pub cur: usize,
    pub since: f32,
    pub left: Option<usize>,
    /// seconds the camera on air has not seen its car
    pub lost: f32,
}

/// One pool camera's chances: the distance from its eye to its car when it sees it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Candidate {
    pub cam: usize,
    /// the car it is judged with (`Follow nearest`: the car nearest its eye)
    pub car: usize,
    pub sees: Option<f32>,
    /// mounted or game-view camera on that car: a fallback when no world camera sees it
    pub rides: bool,
}

/// The auto director's cut rule at video second `t` (20-cine "Auto director"): Some(camera) to cut to, None to stay.
pub(crate) fn auto_pick(run: &AutoRun, cands: &[Candidate], t: f32, min: f32, switch: f32, lost_after: f32) -> Option<usize> {
    let held = t - run.since;
    let cur = cands.iter().find(|c| c.cam == run.cur);
    let cur_rides = cur.is_some_and(|c| c.rides);
    let cur_d = cur.and_then(|c| c.sees).filter(|_| !cur_rides);
    let lost = !cur_rides && cur_d.is_none() && run.lost >= lost_after;
    if held < min && !lost {
        return None;
    }
    // no cut-back: the camera just left waits 2 x Min, unless it alone sees the car
    let barred = |c: usize| run.left == Some(c) && held < 2.0 * min;
    let mut seeing: Vec<(usize, f32)> = cands.iter().filter(|c| !c.rides && c.cam != run.cur).filter_map(|c| c.sees.map(|d| (c.cam, d))).collect();
    seeing.sort_by(|a, b| a.1.total_cmp(&b.1));
    let best = seeing.iter().find(|c| !barred(c.0)).or(seeing.first().filter(|_| lost)).copied();
    match best {
        Some((c, d)) => match cur_d {
            Some(cd) => (d < switch * cd).then_some(c),
            None => (lost || cur_rides).then_some(c),
        },
        None if lost => cands.iter().find(|c| c.rides && c.cam != run.cur && c.sees.is_some()).map(|c| c.cam),
        None => None,
    }
}

/// The first camera of an auto shot: the nearest world camera that sees its car, else a riding one, else the pool's first.
pub(crate) fn auto_first(cands: &[Candidate]) -> usize {
    let world = cands.iter().filter(|c| !c.rides).filter_map(|c| c.sees.map(|d| (c.cam, d))).min_by(|a, b| a.1.total_cmp(&b.1));
    world.map(|w| w.0).or_else(|| cands.iter().find(|c| c.rides && c.sees.is_some()).map(|c| c.cam)).or(cands.first().map(|c| c.cam)).unwrap_or(0)
}

/// Where a camera looks at car `slot` (game space): its drawn model origin, `AIM_UP` above.
fn aim_point(race: &Race, slot: usize) -> Vec3 {
    cam_origin_at(&race.racers[slot].car.p, &race.view_pose(slot)) + Vec3::Y * AIM_UP
}

/// The distance from `station` to car `slot` when camera `cam` sees it: in front, inside the frame less its margins
/// (`size` = the video's), within `AUTO_SEE_MAX` and with no wall between the station and the car. `station` is where
/// the camera stands (a static camera's `Eye`): `Clear on` pulls the drawn eye up to a car behind a wall, a close-up
/// that must neither count as near nor as seen.
fn sees(race: &Race, cam: &Camera, station: Vec3, size: (u32, u32), bars: f32, slot: usize) -> Option<f32> {
    let flip = Vec3::new(1.0, 1.0, -1.0);
    let aim = aim_point(race, slot);
    let d = station.distance(aim);
    if d > AUTO_SEE_MAX.get() {
        return None;
    }
    let (x, y, _) = crate::debug::project_any(cam, size, crate::render::FOV_X.get(), aim * flip)?;
    let m = AUTO_MARGIN.get();
    let (w, h) = (size.0 as f32, size.1 as f32);
    // a letterbox's black bars (`bars` of the height each) are not frame either
    let my = bars + m * (1.0 - 2.0 * bars);
    if !(w * m..=w * (1.0 - m)).contains(&x) || !(h * my..=h * (1.0 - my)).contains(&y) {
        return None;
    }
    race.world.raycast(station, aim).is_none().then_some(d)
}

/// Running cars in the standings, the closest neighbouring pair (ahead, behind) by track distance, and its gap.
fn closest_pair(race: &Race) -> Option<((usize, usize), f32)> {
    let run: Vec<usize> = race.standings().into_iter().filter(|&s| race.racers[s].progress.finished_ms.is_none()).collect();
    run.windows(2).filter_map(|w| pair_gap(race, (w[0], w[1])).map(|g| ((w[0], w[1]), g))).min_by(|a, b| a.1.total_cmp(&b.1))
}

/// Metres between the two cars of a pair (either order), None when one has finished or has no position.
fn pair_gap(race: &Race, (a, b): (usize, usize)) -> Option<f32> {
    let (pa, pb) = (&race.racers[a].progress, &race.racers[b].progress);
    if pa.finished_ms.is_some() || pb.finished_ms.is_some() {
        return None;
    }
    motor_sim::race::distance_gap(&race.line, pa, pb).map(f32::abs)
}

impl Director {
    pub fn new(seq: &Sequence) -> Director {
        let states = seq
            .cameras
            .iter()
            .map(|c| match &c.kind {
                CamKind::Static { .. } => State::Static { aim: None },
                CamKind::Mounted { .. } => State::Mounted,
                CamKind::Pose { mode, .. } => State::Pose(PoseCam::new(*mode)),
                CamKind::Path { .. } => State::Path { aim: None, dir: Vec3::Z },
                CamKind::Orbit { .. } => State::Orbit { centre: None, fwd: Vec3::Z, aim: None },
            })
            .collect();
        let curves = seq
            .cameras
            .iter()
            .map(|c| {
                let eye = curve(&c.keys, |k| k.eye.map(|v| v.to_array().to_vec()));
                let span = eye.as_ref().map_or((0.0, 0.0), |m| (m.keys[0].frame, m.keys[m.keys.len() - 1].frame));
                Curves { eye, angles: curve(&c.keys, |k| k.angles.map(|v| v.to_array().to_vec())), fov: curve(&c.keys, |k| k.fov.map(|f| vec![f])), span }
            })
            .collect();
        let shake = seq.cameras.iter().map(|c| vec![0.0; c.shakes.len()]).collect();
        let n = seq.cameras.len();
        Director { states, curves, shake, focus: 0, hidden: None, views: vec![None; n], dofs: vec![None; n], air: 0, follow: 0, cut_at: vec![None; n], auto: AutoRun::default(), battle: None, last_air: None, dis: None, now: 0.0 }
    }

    /// Camera `i` as of the last `camera` call (on air or not): the camera, the slot it shows and whether it hides that car.
    pub fn view(&self, i: usize) -> Option<(Camera, usize, bool)> {
        self.views.get(i).copied().flatten()
    }

    /// Camera `i`'s depth of field as of the last `camera` call (#421): None without an `Aperture`.
    pub fn dof(&self, i: usize) -> Option<Dof> {
        self.dofs.get(i).copied().flatten()
    }

    /// The slot of the car the camera on air looks at (the session's followed car: env probe, emblem).
    pub fn focus(&self) -> usize {
        self.focus
    }

    /// The slot whose body the camera on air sits inside and so must not be drawn.
    pub fn hidden(&self) -> Option<usize> {
        self.hidden
    }

    /// The camera on air as of the last `camera` call (an auto shot: the director's pick).
    pub fn air(&self) -> usize {
        self.air
    }

    /// The car the auto director follows (`Target auto`), as of the last `camera` call.
    pub fn follow(&self) -> usize {
        self.follow
    }

    /// A dissolve in progress as of the last `camera` call (#154): the camera it comes from and the weight of the
    /// camera on air, 0..1 (smoothstep over the shot's `Dissolve` seconds; 0 on the frame of the cut).
    pub fn dissolve(&self) -> Option<(usize, f32)> {
        let (from, start, len) = self.dis?;
        let k = (self.now - start) / len;
        (0.0..1.0).contains(&k).then_some((from, k * k * (3.0 - 2.0 * k)))
    }

    /// The video second the dissolve in progress started at (its cut), None outside one.
    pub fn dissolve_start(&self) -> Option<f32> {
        self.dissolve().and(self.dis.map(|d| d.1))
    }

    /// Camera `i`'s clock (its `Drive time` path, its Fov keys): seconds since its latest shot or auto cut.
    fn clock(&self, seq: &Sequence, i: usize, t: f32) -> f32 {
        let cut = self.cut_at[i].filter(|&c| c <= t);
        let shot = seq.shots.iter().rev().find(|s| s.auto.is_none() && s.cam == i && s.t <= t).map(|s| s.t);
        match (shot, cut) {
            (Some(a), Some(b)) => t - a.max(b),
            (Some(a), None) | (None, Some(a)) => t - a,
            (None, None) => t,
        }
    }

    /// Camera `i` posed for this frame without shake: the camera, its slot, whether it hides it, whether it rides on it,
    /// its key parameter.
    fn posed(&mut self, seq: &Sequence, i: usize, race: &Race, t: f32, dt: f32) -> (Camera, usize, bool, bool, f32) {
        let since = self.clock(seq, i, t);
        let (mut cam, slot, hide, param, rides) = self.update(i, &seq.cameras[i], race, since, dt);
        // zoom keys override the camera's own zoom
        if let Some(m) = &self.curves[i].fov {
            let z = fov_zoom(evaluate(m, param, false)[0]);
            (cam.zoom, cam.far_k) = (z, far_k(z));
        }
        (cam, slot, hide, rides, param)
    }

    /// Camera `c`'s lens this frame (#421): None without an aperture. `cam` is the drawn pose, `slot` the car it shows,
    /// `u` its key parameter. The focus distance is the subject's depth along the view axis (a focus plane).
    fn lens_at(&self, c: &CineCam, cam: &Camera, slot: usize, u: f32, race: &Race) -> Option<Dof> {
        let f_number = c.lens.aperture?;
        let flip = Vec3::new(1.0, 1.0, -1.0);
        let at = match &c.kind {
            CamKind::Static { aim: Aim::At(a), .. } => Some(*a),
            _ => None,
        };
        let dist = |f: FocusOn| {
            let p = match f {
                FocusOn::Dist(d) => return d,
                FocusOn::Own => at.unwrap_or_else(|| aim_point(race, slot)),
                FocusOn::Car(t) => aim_point(race, resolve_in(race, t, Some(cam.pos * flip), Some(self.follow))),
            };
            (p * flip - cam.pos).dot(cam.forward()).max(FOCUS_MIN)
        };
        let keys: Vec<(f32, f32, bool)> = c.keys.iter().filter_map(|k| k.focus.map(|f| (k.at, dist(f), k.linear))).collect();
        let focus = if keys.is_empty() { dist(c.lens.focus) } else { focus_between(&keys, u) };
        Some(Dof { focus, f_number })
    }

    /// Advance every camera by `dt` (the race has been stepped to `t`) and return the one on air.
    pub fn camera(&mut self, seq: &Sequence, race: &Race, t: f32, dt: f32) -> Camera {
        let si = shot_at(seq, t);
        match &seq.shots[si].auto {
            Some(a) => {
                // the first frame has no views to judge from: pose every camera once, without moving its easing on
                if self.views.iter().all(Option::is_none) {
                    for i in 0..seq.cameras.len() {
                        let (cam, slot, hide, _, _) = self.posed(seq, i, race, t, 0.0);
                        let cam = Camera { lens_across: true, ..cam };
                        self.views[i] = Some((cam, slot, hide));
                    }
                }
                self.auto_step(seq, si, a, race, t, dt);
                self.air = self.auto.cur;
            }
            None => {
                self.auto.shot = None;
                self.follow = resolve(race, Target::Leader, None);
                self.air = seq.shots[si].cam;
            }
        }
        // a cut starts the shot's dissolve from the camera that was on air (a restarted one replaces the running one [APPROX])
        if let Some(prev) = self.last_air.filter(|&p| p != self.air) {
            let d = seq.shots[si].dissolve;
            self.dis = (d > 0.0).then_some((prev, t, d));
        }
        (self.last_air, self.now) = (Some(self.air), t);
        let air = self.air;
        let mut on = None;
        for (i, c) in seq.cameras.iter().enumerate() {
            let (mut cam, slot, hide, rides, param) = self.posed(seq, i, race, t, dt);
            // #479: a .cine camera's FOV is the field across at every aspect, not the main view's Vert+
            cam.lens_across = true;
            self.apply_shake(i, c, &mut cam, race, rides.then_some(slot), t, dt);
            self.views[i] = Some((cam, slot, hide));
            self.dofs[i] = self.lens_at(c, &cam, slot, param, race);
            if i == air {
                on = Some(cam);
                (self.focus, self.hidden) = (slot, hide.then_some(slot));
            }
        }
        on.unwrap_or_else(|| Camera::new(Vec3::ZERO, 0.0, 0.0))
    }

    /// The car an auto shot follows now (`Follow nearest`: kept, it is the car of the camera on air).
    fn follow_car(&mut self, f: Follow, race: &Race) -> usize {
        let last = race.racers.len() - 1;
        match f {
            Follow::Leader => resolve(race, Target::Leader, None),
            Follow::Car(n) => n.min(last),
            Follow::Player => last,
            Follow::Nearest => self.follow.min(last),
            Follow::Battle => {
                let best = closest_pair(race);
                let held = self.battle.and_then(|p| pair_gap(race, p).map(|g| (p, g)));
                self.battle = match (held, best) {
                    (Some((p, g)), Some((q, h))) => Some(if h < BATTLE_SWITCH.get() * g { q } else { p }),
                    (None, b) => b.map(|b| b.0),
                    (h, None) => h.map(|h| h.0),
                };
                match self.battle {
                    // the chasing car: the one further back now
                    Some((a, b)) => {
                        let (pa, pb) = (&race.racers[a].progress, &race.racers[b].progress);
                        match motor_sim::race::distance_gap(&race.line, pa, pb) {
                            Some(g) if g < 0.0 => a,
                            _ => b,
                        }
                    }
                    None => resolve(race, Target::Leader, None),
                }
            }
        }
    }

    /// One frame of auto shot `si`: the followed car, then the cut rule over the pool (judged from the cameras' last views).
    fn auto_step(&mut self, seq: &Sequence, si: usize, a: &Auto, race: &Race, t: f32, dt: f32) {
        self.follow = self.follow_car(a.follow, race);
        let bars = seq.letterbox.map_or(0.0, |l| letterbox_bar(seq.size.0, seq.size.1, l) as f32 / seq.size.1.max(1) as f32);
        let cands: Vec<Candidate> = auto_pool(seq, a)
            .into_iter()
            .map(|i| {
                let rides = !is_world(&seq.cameras[i].kind);
                let Some((cam, slot, _)) = self.views[i] else { return Candidate { cam: i, car: self.follow, sees: None, rides } };
                if rides {
                    // a riding camera shows its own car
                    let ok = a.follow == Follow::Nearest || slot == self.follow;
                    let d = (cam.pos * Vec3::new(1.0, 1.0, -1.0)).distance(aim_point(race, slot));
                    return Candidate { cam: i, car: slot, sees: ok.then_some(d), rides };
                }
                let station = match &seq.cameras[i].kind {
                    CamKind::Static { eye, .. } => *eye,
                    _ => cam.pos * Vec3::new(1.0, 1.0, -1.0),
                };
                let car = if a.follow == Follow::Nearest { resolve(race, Target::Nearest, Some(station)) } else { self.follow };
                Candidate { cam: i, car, sees: sees(race, &cam, station, seq.size, bars, car), rides }
            })
            .collect();
        if self.auto.shot != Some(si) {
            let first = auto_first(&cands);
            // its clock starts with the shot (a director started inside the shot, the editor's after a seek, too)
            let at = seq.shots[si].t.min(t);
            self.auto = AutoRun { shot: Some(si), cur: first, since: at, left: None, lost: 0.0 };
            self.cut_at[first] = Some(at);
        } else {
            let seen = cands.iter().find(|c| c.cam == self.auto.cur).is_some_and(|c| c.sees.is_some());
            self.auto.lost = if seen { 0.0 } else { self.auto.lost + dt };
            if let Some(c) = auto_pick(&self.auto, &cands, t, a.min, AUTO_SWITCH.get(), AUTO_LOST.get()) {
                self.auto = AutoRun { shot: Some(si), cur: c, since: t, left: Some(self.auto.cur), lost: 0.0 };
                self.cut_at[c] = Some(t);
            }
        }
        if a.follow == Follow::Nearest {
            if let Some(c) = cands.iter().find(|c| c.cam == self.auto.cur) {
                self.follow = c.car;
            }
        }
    }

    /// Shake camera `i` (on video time `t`); `own` = the car it rides on (not counted by proximity).
    #[allow(clippy::too_many_arguments)]
    fn apply_shake(&mut self, i: usize, c: &CineCam, cam: &mut Camera, race: &Race, own: Option<usize>, t: f32, dt: f32) {
        let eye = cam.pos * Vec3::new(1.0, 1.0, -1.0);
        let (mut ang, mut sway) = ([0.0f32; 3], [0.0f32; 2]);
        for (k, sh) in c.shakes.iter().enumerate() {
            let level = &mut self.shake[i][k];
            *level = shake_level(sh.kind, *level, shake_target(sh, race, eye, own), dt);
            let (a, s) = shake_offsets(sh, (i * 16 + k) as u32, t, *level);
            for j in 0..3 {
                ang[j] += a[j];
            }
            sway[0] += s[0];
            sway[1] += s[1];
        }
        if c.shakes.is_empty() {
            return;
        }
        let (right, up) = (cam.right(), cam.up());
        cam.pos += right * sway[0] + up * sway[1];
        cam.yaw += ang[0] * DEG;
        cam.pitch += ang[1] * DEG;
        cam.roll += ang[2] * DEG;
    }

    /// Camera `i` this frame: the camera, the slot it shows, whether it hides that car, its key parameter (Fov keys)
    /// and whether it rides on that car.
    fn update(&mut self, i: usize, c: &CineCam, race: &Race, since: f32, dt: f32) -> (Camera, usize, bool, f32, bool) {
        let follow = self.follow;
        match (&c.kind, &mut self.states[i]) {
            (CamKind::Static { eye, aim, zoom, clear }, State::Static { aim: state }) => match aim {
                Aim::At(a) => {
                    let z = match zoom {
                        Zoom::Fixed(z) => *z,
                        Zoom::Auto(_) => 1.0,
                    };
                    (Camera::from_rot(*eye, look_rot(*eye, *a), z, far_k(z)), 0, false, since, false)
                }
                Aim::Track { target, lag, lead } => {
                    let (cam, slot) = track_cam(race, *eye, *target, follow, *lag, *lead, *zoom, *clear, state, dt);
                    (cam, slot, false, since, false)
                }
            },
            (CamKind::Mounted { target, offset, angles, zoom, hide }, State::Mounted) => {
                let slot = resolve_in(race, *target, None, Some(follow));
                let v = race.view_pose(slot);
                let eye = cam_origin_at(&race.racers[slot].car.p, &v) + v.rot * *offset;
                (Camera::from_rot(eye, v.rot * euler_to_rot(*angles * DEG), *zoom, far_k(*zoom)), slot, *hide, since, true)
            }
            (CamKind::Pose { mode, target }, State::Pose(pc)) => {
                let slot = resolve_in(race, *target, None, Some(follow));
                let v = race.view_pose(slot);
                (pc.camera(race, &CamTune::live(), cam_origin_at(&race.racers[slot].car.p, &v), v.rot, dt), slot, mode.hides_car(), since, true)
            }
            (CamKind::Path { drive, aim, zoom, clear }, State::Path { aim: state, dir }) => {
                let cv = &self.curves[i];
                let Some(path) = &cv.eye else { unreachable!("parse drops a path without Eye keys") };
                // the key parameter, and a step for the tangent
                let (u, du, mut slot) = match drive {
                    Drive::Time => (since, 0.02, 0),
                    Drive::Car(t) => {
                        let slot = resolve_in(race, *t, None, Some(follow));
                        (lap_param(race.racers[slot].progress.s, race.line.length, cv.span.0, cv.span.1), 0.5, slot)
                    }
                };
                let eye = eval3(path, u);
                let fixed = match zoom {
                    Zoom::Fixed(z) => *z,
                    Zoom::Auto(_) => 1.0,
                };
                let cam = match aim {
                    PathAim::Keys => {
                        let rot = cv.angles.as_ref().map_or(Mat3::IDENTITY, |m| euler_to_rot(eval3(m, u) * DEG));
                        Camera::from_rot(eye, rot, fixed, far_k(fixed))
                    }
                    PathAim::Tangent => {
                        // held at the ends of the path, where the curve stops moving
                        if let Some(d) = (eval3(path, u + du) - eval3(path, u - du)).try_normalize() {
                            *dir = d;
                        }
                        Camera::from_rot(eye, look_dir(*dir).unwrap_or(Mat3::IDENTITY), fixed, far_k(fixed))
                    }
                    PathAim::Track { target, lag, lead } => {
                        let (cam, s) = track_cam(race, eye, *target, follow, *lag, *lead, *zoom, *clear, state, dt);
                        slot = s;
                        cam
                    }
                };
                (cam, slot, false, u, false)
            }
            (CamKind::Orbit { target, radius, height, speed, start, lag, zoom, clear }, State::Orbit { centre, fwd, aim }) => {
                let slot = resolve_in(race, *target, None, Some(follow));
                let v = race.view_pose(slot);
                let goal = cam_origin_at(&race.racers[slot].car.p, &v);
                let c = centre.map_or(goal, |p| ease(p, goal, *lag, dt));
                *centre = Some(c);
                // the heading eased as a vector, so a spin never jumps; a car pointing straight up or down keeps the last one
                let heading = Vec3::new(v.rot.z_axis.x, 0.0, v.rot.z_axis.z);
                if let Some(f) = ease(*fwd, heading.try_normalize().unwrap_or(*fwd), *lag, dt).with_y(0.0).try_normalize() {
                    *fwd = f;
                }
                let eye = orbit_eye(c, *fwd, *radius, *height, start + speed * since);
                let (cam, s) = track_cam(race, eye, *target, follow, *lag, 0.0, *zoom, *clear, aim, dt);
                (cam, s, false, since, true)
            }
            _ => unreachable!("camera state matches its kind"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "# Motorhead cine v1\n\
        Race Track 2 Cars 3 1 2 Difficulty hard Laps 2 Start go\n\
        Output Size 1280 720 Fps 30\nEnd 12.5\nAudio on Doppler off\n\
        Camera a static Eye 1 2 3 Target car 1 Zoom auto 25 Lag 0.3 Lead 0.2 Clear on\n\
        Camera b static Eye 4 5 6 At 0 0 0 Zoom 2\n\
        Camera c mounted Target leader Offset 0 1.5 -0.5 Angles 5 0 0 Hide on\n\
        Camera d bumper Target player\n\
        Shot 4 b\nShot 0 a\nShot 8 c\nShot 10 d\n";

    #[test]
    fn round_trip() {
        let (s, w) = parse(SAMPLE).unwrap();
        assert!(w.is_empty(), "{w:?}");
        assert_eq!(s.shots.iter().map(|x| x.t).collect::<Vec<_>>(), [0.0, 4.0, 8.0, 10.0]);
        assert_eq!(s.audio, AudioDef { on: true, doppler: false });
        let (s2, w2) = parse(&to_text(&s)).unwrap();
        assert!(w2.is_empty(), "{w2:?}");
        assert_eq!(s, s2);
    }

    /// #423: `Recording` keeps the rest of its line (spaces included, a comment cut off) and round-trips.
    #[test]
    fn recording_line_round_trips() {
        let (s, w) = parse(&format!("{SAMPLE}Recording ~/My Runs/Goldbridge 1.mhr  # the final\n")).unwrap();
        assert!(w.is_empty(), "{w:?}");
        assert_eq!(s.race.recording.as_deref(), Some("~/My Runs/Goldbridge 1.mhr"));
        let (s2, _) = parse(&to_text(&s)).unwrap();
        assert_eq!(s, s2);
        let (_, w) = parse(&format!("{SAMPLE}Recording\n")).unwrap();
        assert_eq!(w.len(), 1, "{w:?}");
    }

    #[test]
    fn race_and_video_clocks() {
        let (s, _) = parse(SAMPLE).unwrap();
        // every shot at speed 1: the clocks agree
        for t in [0.0, 3.0, 4.0, 9.5, 30.0] {
            assert!((race_secs(&s, t) - t).abs() < 1e-5);
            assert!((video_secs(&s, t) - t).abs() < 1e-5);
        }
        // 4..8 s of video at half speed: the race is 2 s behind from 8 s on
        let (s, _) = parse("Camera a static Eye 0 0 0 At 1 0 0\nShot 0 a\nShot 4 a Speed 0.5\nShot 8 a\n").unwrap();
        assert!((race_secs(&s, 6.0) - 5.0).abs() < 1e-5);
        assert!((race_secs(&s, 10.0) - 8.0).abs() < 1e-5);
        for t in [0.0, 2.0, 4.0, 5.5, 8.0, 12.0] {
            assert!((video_secs(&s, race_secs(&s, t)) - t).abs() < 1e-4, "{t}");
        }
        // a slow first shot
        let (s, _) = parse("Camera a static Eye 0 0 0 At 1 0 0\nShot 0 a Speed 0.25\nShot 4 a\n").unwrap();
        assert!((race_secs(&s, 4.0) - 1.0).abs() < 1e-5 && (race_secs(&s, 6.0) - 3.0).abs() < 1e-5);
        assert!((video_secs(&s, 3.0) - 6.0).abs() < 1e-5);
    }

    #[test]
    fn on_air_at_boundaries() {
        let (s, _) = parse(SAMPLE).unwrap();
        let name = |t| s.cameras[on_air(&s, t)].name.as_str();
        assert_eq!((name(0.0), name(3.99), name(4.0), name(9.9), name(99.0)), ("a", "a", "b", "c", "d"));
        // before the first shot: the first shot's camera
        let (s, _) = parse("Camera a static Eye 0 0 0 At 1 0 0\nCamera b static Eye 0 0 0 At 1 0 0\nShot 2 b\n").unwrap();
        assert_eq!(on_air(&s, 0.0), 1);
    }

    #[test]
    fn bad_lines_are_skipped() {
        let (s, w) = parse("Camera a static Eye 0 0 0 At 1 0 0\nCamera x static Eye 0 0\nCamera y wibble Target car 0\nFoo bar\nShot 1 nope\nShot x a\nRace Track 9\n").unwrap();
        assert_eq!(s.cameras.len(), 1);
        assert_eq!(w.len(), 7, "{w:?}");
        assert!(parse("Race Track 1\n").is_err());
    }

    #[test]
    fn auto_zoom_clamps() {
        assert_eq!(auto_zoom(10.0, 25.0), 1.0);
        assert!((auto_zoom(50.0, 25.0) - 2.0).abs() < 1e-6);
        assert_eq!(auto_zoom(5000.0, 25.0), ZOOM_MAX);
    }

    #[test]
    fn easing_is_time_constant_based() {
        assert_eq!(ease(Vec3::ZERO, Vec3::X, 0.0, 0.016), Vec3::X);
        // two half-steps equal one whole step
        let a = ease(ease(Vec3::ZERO, Vec3::X, 0.5, 0.05), Vec3::X, 0.5, 0.05);
        let b = ease(Vec3::ZERO, Vec3::X, 0.5, 0.1);
        assert!(a.distance(b) < 1e-5);
    }

    const PATHS: &str = "Race Track 1 Cars 1 2 3\n\
        Camera fly path Drive time Zoom 1.2\n\
        Camera dolly path Drive car 1 Target car 1 Lag 0.2 Lead 0 Zoom auto 20 Clear on\n\
        Camera keyed path Aim keys\n\
        Camera gantry static Eye 1 2 3 At 0 0 0\n\
        Key fly 0 Eye 0 5 0 Fov 60\n\
        Key fly 4 Eye 40 5 0 Tcb 0.5 -0.25 1 Linear on\n\
        Key fly 2 Eye 20 9 10 Angles 1 2 3\n\
        Key dolly 4900 Eye 0 0 0\n\
        Key dolly 5100 Eye 1 1 1\n\
        Key keyed 0 Eye 0 0 0 Angles 0 90 0\n\
        Key gantry 1 Fov 30\n\
        Shake gantry proximity 2\n\
        Shake fly handheld 0.5 Radius 3\n\
        Shot 0 fly FadeIn 0.5\nShot 4 dolly Speed 0.5 FadeOut 1\nShot 9 gantry\nShot 9.5 keyed\n";

    #[test]
    fn paths_round_trip() {
        let (s, w) = parse(PATHS).unwrap();
        assert!(w.is_empty(), "{w:?}");
        assert_eq!(s.cameras[0].keys.iter().map(|k| k.at).collect::<Vec<_>>(), [0.0, 2.0, 4.0], "keys sorted");
        assert!(matches!(s.cameras[2].kind, CamKind::Path { aim: PathAim::Keys, .. }), "Angles keys make the aim keyed");
        assert!(matches!(s.cameras[0].kind, CamKind::Path { aim: PathAim::Keys, .. }), "one Angles key is enough");
        assert!(matches!(s.cameras[1].kind, CamKind::Path { drive: Drive::Car(Target::Car(1)), aim: PathAim::Track { .. }, .. }));
        assert_eq!((s.shots[1].speed, s.shots[1].fade_out, s.shots[0].fade_in), (0.5, 1.0, 0.5));
        let (s2, w2) = parse(&to_text(&s)).unwrap();
        assert!(w2.is_empty(), "{w2:?}");
        assert_eq!(s, s2);
    }

    #[test]
    fn bad_path_lines_are_skipped() {
        let text = "Camera a static Eye 0 0 0 At 1 0 0\n\
            Camera p path Drive time\n\
            Camera q path Aim keys\nKey q 0 Eye 0 0 0\n\
            Key a 0 Eye 1 1 1\nKey a 1 Eye 1 1 1 Fov 50\nKey nope 0 Fov 50\nKey a 2\nKey a 3 Fov 200\nKey q 0 Eye 1 1 1\n\
            Shake a wobble 1\nShake nope handheld 1\nShot 0 a Speed 9\nShot 0 p\n";
        let (s, w) = parse(text).unwrap();
        // p has no Eye key: dropped, and the shot naming it with it
        assert_eq!(s.cameras.iter().map(|c| c.name.as_str()).collect::<Vec<_>>(), ["a", "q"]);
        assert_eq!(s.cameras[0].keys.len(), 1, "the Fov part of the second key stays");
        assert!(matches!(s.cameras[1].kind, CamKind::Path { aim: PathAim::Tangent, .. }));
        assert_eq!(w.len(), 13, "{w:#?}");
    }

    #[test]
    fn lens_parses_and_round_trips() {
        let text = "Camera a static Eye 1 2 3 Target car 1 Aperture 2.8 Zoom 4\n\
            Camera b mounted Target leader Focus car 2 Aperture 1.4 Hide on\n\
            Camera c static Eye 0 0 0 At 1 0 0 Focus 12.5\n\
            Camera d chase Target player Aperture 8 Focus own\n\
            Camera p path Aperture 2 Focus leader\nKey p 0 Eye 0 0 0\n\
            Key a 0 Focus car 1\nKey a 2 Focus car 2 Linear on\nKey a 4 Fov 30 Focus 40\nShot 0 a\n";
        let (s, w) = parse(text).unwrap();
        assert!(w.is_empty(), "{w:?}");
        assert_eq!(s.cameras[0].lens, Lens { aperture: Some(2.8), focus: FocusOn::Own });
        assert!(matches!(s.cameras[0].kind, CamKind::Static { zoom: Zoom::Fixed(4.0), .. }), "the other words still parse");
        assert_eq!(s.cameras[1].lens, Lens { aperture: Some(1.4), focus: FocusOn::Car(Target::Car(2)) });
        assert_eq!(s.cameras[2].lens, Lens { aperture: None, focus: FocusOn::Dist(12.5) });
        assert_eq!(s.cameras[3].lens.focus, FocusOn::Own);
        assert_eq!(s.cameras[4].lens.focus, FocusOn::Car(Target::Leader));
        let f: Vec<_> = s.cameras[0].keys.iter().map(|k| k.focus).collect();
        assert_eq!(f, [Some(FocusOn::Car(Target::Car(1))), Some(FocusOn::Car(Target::Car(2))), Some(FocusOn::Dist(40.0))]);
        assert!(s.cameras[0].keys[1].linear);
        let (s2, w2) = parse(&to_text(&s)).unwrap();
        assert!(w2.is_empty(), "{w2:?}");
        assert_eq!(s, s2);
        // bad lens words skip the line
        let (s, w) = parse("Camera a static Eye 0 0 0 At 1 0 0 Aperture 0.05\nCamera b static Eye 0 0 0 At 1 0 0 Focus 0.1\nCamera c static Eye 0 0 0 At 1 0 0 Focus car\nCamera d static Eye 0 0 0 At 1 0 0 Aperture\nCamera e static Eye 0 0 0 At 1 0 0\nKey e 0 Focus far\nShot 0 e\n").unwrap();
        assert_eq!(s.cameras.len(), 1);
        assert_eq!(w.len(), 5, "{w:#?}");
    }

    #[test]
    fn focus_racks_in_diopters() {
        let k = [(0.0, 10.0, false), (2.0, 40.0, true), (4.0, 20.0, false)];
        // held outside the keys
        assert_eq!(focus_between(&k, -1.0), 10.0);
        assert_eq!(focus_between(&k, 9.0), 20.0);
        assert_eq!(focus_between(&k, 0.0), 10.0);
        // halfway in 1 / distance, not in metres: 1 / ((1/10 + 1/40) / 2) = 16
        assert!((focus_between(&k, 1.0) - 16.0).abs() < 1e-4);
        // the segment into a `Linear on` key is straight at a quarter, the others ease (smoothstep 0.15625 at a quarter)
        assert!((focus_between(&k, 0.5) - 1.0 / (0.1 - 0.075 * 0.25)).abs() < 1e-3);
        assert!((focus_between(&k, 2.5) - 1.0 / (0.025 + 0.025 * 0.15625)).abs() < 1e-3);
        assert_eq!(focus_between(&k, 2.0), 40.0);
        assert_eq!(focus_between(&[], 1.0), f32::INFINITY);
    }

    fn key(at: f32, x: f32, tcb: [f32; 3]) -> CamKey {
        CamKey { at, eye: Some(Vec3::new(x, 0.0, 0.0)), angles: None, fov: None, focus: None, tcb, linear: false }
    }

    #[test]
    fn path_curve_passes_its_keys_and_tcb_shapes_it() {
        let keys = |t: [f32; 3]| vec![key(0.0, 0.0, [0.0; 3]), key(1.0, 10.0, t), key(3.0, 40.0, [0.0; 3])];
        let m = curve(&keys([0.0; 3]), |k| k.eye.map(|v| v.to_array().to_vec())).unwrap();
        for (t, x) in [(0.0, 0.0), (1.0, 10.0), (3.0, 40.0)] {
            assert!((eval3(&m, t).x - x).abs() < 1e-4);
        }
        // held past the ends
        assert_eq!(eval3(&m, -5.0), eval3(&m, 0.0));
        assert_eq!(eval3(&m, 9.0), eval3(&m, 3.0));
        // another TCB on the middle key: still through it, a different shape around it
        for tcb in [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, -1.0]] {
            let other = curve(&keys(tcb), |k| k.eye.map(|v| v.to_array().to_vec())).unwrap();
            assert!((eval3(&other, 1.0).x - 10.0).abs() < 1e-4);
            assert!((eval3(&other, 1.5).x - eval3(&m, 1.5).x).abs() > 0.1, "{tcb:?}");
        }
    }

    #[test]
    fn car_paced_paths_wrap_over_the_line() {
        let len = 5000.0;
        // keys 4900..5100 cross the line: a car at 50 m is at 5050 of the path
        assert_eq!(lap_param(50.0, len, 4900.0, 5100.0), 5050.0);
        assert_eq!(lap_param(4950.0, len, 4900.0, 5100.0), 4950.0);
        assert_eq!(lap_param(200.0, len, 100.0, 300.0), 200.0);
        // keys -100..100: a car at 4950 is at -50
        assert_eq!(lap_param(4950.0, len, -100.0, 100.0), -50.0);
        // outside the span: the nearer side
        assert_eq!(lap_param(2000.0, len, 100.0, 300.0), 2000.0);
    }

    #[test]
    fn shot_mode() {
        let text = "Camera a static Eye 0 0 0 At 1 0 0\nShot 0 a\nShot 4 a Mode remaster\nShot 8 auto Mode CLASSIC\n";
        let (s, w) = parse(text).unwrap();
        assert!(w.is_empty(), "{w:?}");
        assert_eq!(s.shots.iter().map(|x| x.mode).collect::<Vec<_>>(), [None, Some(RenderMode::Remaster), Some(RenderMode::Classic)]);
        assert_eq!(mode_at(&s, 1.0, RenderMode::Classic), RenderMode::Classic);
        assert_eq!(mode_at(&s, 1.0, RenderMode::Remaster), RenderMode::Remaster);
        assert_eq!(mode_at(&s, 5.0, RenderMode::Classic), RenderMode::Remaster);
        assert_eq!(mode_at(&s, 9.0, RenderMode::Remaster), RenderMode::Classic);
        assert!(uses_mode(&s, RenderMode::Remaster));
        let out = to_text(&s);
        assert!(out.contains("Mode remaster") && out.contains("Mode classic"));
        assert_eq!(parse(&out).unwrap().0.shots, s.shots);
        let (_, w) = parse("Camera a static Eye 0 0 0 At 1 0 0\nShot 0 a\nShot 5 a Mode fancy\n").unwrap();
        assert!(w.iter().any(|m| m.contains("Mode")), "{w:?}");
    }

    #[test]
    fn shot_pitch() {
        let text = "Camera a static Eye 0 0 0 At 1 0 0\nShot 0 a\nShot 4 a Speed 0.5 Pitch tape\nShot 8 auto Speed 0.25 Pitch TAPE\nShot 12 a Speed 0.5 Pitch keep\n";
        let (s, w) = parse(text).unwrap();
        assert!(w.is_empty(), "{w:?}");
        assert_eq!(s.shots.iter().map(|x| x.pitch).collect::<Vec<_>>(), [Pitch::Keep, Pitch::Tape, Pitch::Tape, Pitch::Keep]);
        assert_eq!((audio_rate_at(&s, 1.0), audio_rate_at(&s, 5.0), audio_rate_at(&s, 9.0), audio_rate_at(&s, 13.0)), (1.0, 0.5, 0.25, 1.0));
        let out = to_text(&s);
        assert_eq!(out.matches("Pitch tape").count(), 2);
        assert!(!out.contains("Pitch keep"));
        let (back, w) = parse(&out).unwrap();
        assert!(w.is_empty(), "{w:?}");
        assert_eq!(back, s);
        let (_, w) = parse("Camera a static Eye 0 0 0 At 1 0 0\nShot 0 a\nShot 5 a Pitch fast\n").unwrap();
        assert!(w.iter().any(|m| m.contains("Pitch")), "{w:?}");
    }

    #[test]
    fn shot_clocks_speed_and_fades() {
        let (s, _) = parse(PATHS).unwrap();
        assert_eq!(since_shot(&s, 0, 1.5), 1.5);
        assert_eq!(since_shot(&s, 1, 6.0), 2.0);
        assert_eq!(since_shot(&s, 1, 3.0), 3.0, "before its first shot: video time");
        assert_eq!((speed_at(&s, 3.9), speed_at(&s, 4.0), speed_at(&s, 9.0)), (1.0, 0.5, 1.0));
        assert_eq!(fade_at(&s, 0.0, None), 0.0);
        assert_eq!(fade_at(&s, 0.25, None), 0.5);
        assert_eq!(fade_at(&s, 1.0, None), 1.0);
        // the dolly shot fades out to the next shot at 9
        assert_eq!(fade_at(&s, 8.0, None), 1.0);
        assert!((fade_at(&s, 8.5, None) - 0.5).abs() < 1e-6);
        // a last shot fades out only to a known end
        let (s, _) = parse("Camera a static Eye 0 0 0 At 1 0 0\nShot 0 a FadeOut 2\n").unwrap();
        assert_eq!(fade_at(&s, 9.0, None), 1.0);
        assert_eq!(fade_at(&s, 9.0, Some(10.0)), 0.5);
    }

    #[test]
    fn fov_keys_zoom() {
        assert!((fov_zoom(90.0) - 1.0).abs() < 1e-6);
        assert!((fov_zoom(2.0 * (0.5f32).atan().to_degrees()) - 2.0).abs() < 1e-4);
    }

    #[test]
    fn shake_is_bounded_and_repeatable() {
        for kind in [ShakeKind::Proximity, ShakeKind::Handheld, ShakeKind::Impact] {
            let sh = Shake { kind, amp: 3.0, radius: 15.0 };
            let (mut peak, mut moved) = (0.0f32, false);
            for i in 0..20_000 {
                let t = i as f32 * 0.0037;
                let (a, w) = shake_offsets(&sh, 7, t, 1.0);
                assert_eq!((a, w), shake_offsets(&sh, 7, t, 1.0));
                peak = a.iter().fold(peak, |p, x| p.max(x.abs()));
                assert!(w.iter().all(|x| x.abs() <= 3.0 * HANDHELD_SWAY));
                moved |= a[0] != shake_offsets(&sh, 7, t + 0.05, 1.0).0[0];
            }
            assert!(peak <= 3.0 && peak > 1.0 && moved, "{kind:?} peak {peak}");
            // other seeds shake differently; no strength, no shake
            assert_ne!(shake_offsets(&sh, 7, 1.3, 1.0), shake_offsets(&sh, 8, 1.3, 1.0));
            assert_eq!(shake_offsets(&sh, 7, 1.3, 0.0).0, [0.0; 3]);
        }
    }

    #[test]
    fn shake_levels_ease() {
        // proximity eases in faster than out; an impact kicks at once and decays
        let up = shake_level(ShakeKind::Proximity, 0.0, 1.0, 0.1);
        let down = 1.0 - shake_level(ShakeKind::Proximity, 1.0, 0.0, 0.1);
        assert!(up > down && up < 1.0);
        assert_eq!(shake_level(ShakeKind::Impact, 0.2, 0.9, 0.016), 0.9);
        assert!(shake_level(ShakeKind::Impact, 0.9, 0.0, IMPACT_DECAY) < 0.9 * 0.37);
    }

    #[test]
    fn look_rot_faces_the_target() {
        let r = look_rot(Vec3::new(0.0, 2.0, 0.0), Vec3::new(10.0, 2.0, 10.0));
        assert!(r.z_axis.distance(Vec3::new(1.0, 0.0, 1.0).normalize()) < 1e-5);
        assert!(r.y_axis.y > 0.9);
        assert_eq!(look_rot(Vec3::ONE, Vec3::ONE), Mat3::IDENTITY);
    }

    #[test]
    fn auto_shots_parse_and_round_trip() {
        let text = "Camera a static Eye 0 0 0 Target auto\nCamera b static Eye 9 0 0 At 1 0 0\nCamera c chase Target auto\n\
            Shot 0 a\nShot 4 auto Cams b c Follow car 1 Min 2.5 FadeIn 1\nShot 9 auto\n";
        let (s, w) = parse(text).unwrap();
        assert!(w.is_empty(), "{w:?}");
        assert_eq!(s.shots.len(), 3);
        assert_eq!(s.shots[1].auto, Some(Auto { pool: vec![1, 2], follow: Follow::Car(1), min: 2.5 }));
        assert_eq!((s.shots[1].cam, s.shots[1].fade_in), (1, 1.0), "the pool's first camera");
        assert_eq!(s.shots[2].auto, Some(Auto::default()));
        assert_eq!(s.shots[2].cam, 0, "empty pool: the first world camera");
        assert!(matches!(s.cameras[0].kind, CamKind::Static { aim: Aim::Track { target: Target::Auto, .. }, .. }));
        let (back, w) = parse(&to_text(&s)).unwrap();
        assert!(w.is_empty(), "{w:?}");
        assert_eq!(back, s);
        // an auto shot is no shot of its first camera: that camera's clock runs on
        assert_eq!(since_shot(&s, 1, 6.0), 6.0);
        assert_eq!(auto_pool(&s, s.shots[2].auto.as_ref().unwrap()), [0, 1]);
        // `auto` is not a camera name; Follow is for auto shots only; unknown pool cameras are reported
        let (_, w) = parse("Camera auto static Eye 0 0 0 At 1 0 0\nCamera a static Eye 0 0 0 At 1 0 0\nShot 0 a Follow leader\nShot 1 auto Cams zz\n").unwrap();
        assert_eq!(w.len(), 3, "{w:?}");
    }

    #[test]
    fn dissolve_and_letterbox_round_trip_and_run() {
        let text = "Output Size 1920 1080 Fps 30 Letterbox 2.39\nCamera a static Eye 0 0 0 At 1 0 0\nCamera b static Eye 9 0 0 At 1 0 0\n\
            Shot 0 a\nShot 5 b Dissolve 1\nShot 9 auto Dissolve 0.4\n";
        let (s, w) = parse(text).unwrap();
        assert!(w.is_empty(), "{w:?}");
        assert_eq!((s.letterbox, s.shots[0].dissolve, s.shots[1].dissolve, s.shots[2].dissolve), (Some(2.39), 0.0, 1.0, 0.4));
        let (back, w) = parse(&to_text(&s)).unwrap();
        assert!(w.is_empty(), "{w:?}");
        assert_eq!(back, s);
        // no letterbox, no dissolve: the old text; off, a bad aspect and a negative dissolve
        let (plain, _) = parse("Camera a static Eye 0 0 0 At 1 0 0\nShot 0 a\n").unwrap();
        assert!(!to_text(&plain).contains("Letterbox") && !to_text(&plain).contains("Dissolve"));
        let (off, w) = parse("Output Letterbox off\nOutput Letterbox 9\nCamera a static Eye 0 0 0 At 1 0 0\nShot 0 a Dissolve -1\n").unwrap();
        assert_eq!(off.letterbox, None);
        assert!(w[0].contains("Letterbox") && w[1].contains("dissolve"), "{w:?}");
        // the director's dissolve: from the old camera, the new one's weight rising by smoothstep over the shot's seconds
        let mut d = Director::new(&s);
        assert_eq!(d.dissolve(), None);
        d.dis = Some((0, 5.0, 1.0));
        for (now, want) in [(5.0, Some(0.0)), (5.5, Some(0.5)), (5.25, Some(0.15625)), (6.0, None), (4.0, None)] {
            d.now = now;
            assert_eq!(d.dissolve().map(|x| x.1), want, "at {now}");
        }
        assert_eq!(d.dissolve(), None);
    }

    #[test]
    fn preview_frame_fits_and_centres() {
        let near = |a: [f32; 4], b: [f32; 4]| a.iter().zip(b).all(|(x, y)| (x - y).abs() < 1e-3);
        // no panels, the window's own aspect: the whole window
        assert!(near(preview_frame((1920.0, 1080.0), (0.0, 0.0, 0.0, 0.0), (1920, 1080), 0.0), [0.0, 0.0, 1920.0, 1080.0]));
        // a dock on the right: centred on the free area, as wide as it allows
        let r = preview_frame((1920.0, 1080.0), (100.0, 360.0, 120.0, 0.0), (1920, 1080), 8.0);
        assert!((r[0] + r[2] * 0.5 - (1920.0 - 368.0) * 0.5).abs() < 1e-3 && (r[1] + r[3] * 0.5 - (108.0 + 952.0) * 0.5).abs() < 1e-3);
        assert!((r[3] - (1080.0 - 108.0 - 128.0)).abs() < 1e-3 && (r[2] / r[3] - 16.0 / 9.0).abs() < 1e-3);
        assert!(r[0] >= -1e-3 && r[0] + r[2] <= 1920.0 - 368.0 + 1e-3 && r[1] >= 108.0 - 1e-3 && r[1] + r[3] <= 952.0 + 1e-3);
        // a tall window: the width binds
        let t = preview_frame((800.0, 1200.0), (0.0, 0.0, 0.0, 0.0), (1920, 1080), 0.0);
        assert!(near(t, [0.0, 600.0 - 225.0, 800.0, 450.0]));
        // panels over everything: still a positive rectangle
        assert!(preview_frame((100.0, 100.0), (90.0, 90.0, 90.0, 90.0), (16, 9), 0.0)[2] > 0.0);
    }

    #[test]
    fn preview_cam_puts_the_view_axis_on_the_frame() {
        let (win, size) = ((1920.0, 1080.0), (1920, 1080));
        let rect = preview_frame(win, (40.0, 700.0, 30.0, 0.0), (1920, 1080), 8.0);
        // a Cine camera: its FOV is the field across (#479)
        let cam = preview_cam(Camera { lens_across: true, ..Camera::new(Vec3::new(1.0, 2.0, 3.0), 0.4, -0.1) }, rect, win);
        assert!((cam.zoom - rect[2] / 1920.0).abs() < 1e-6);
        // a point straight ahead projects to the frame's centre
        let (x, y, _) = crate::debug::project_any(&cam, size, 90.0, cam.pos + cam.forward() * 50.0).unwrap();
        assert!((x - (rect[0] + rect[2] * 0.5)).abs() < 1e-2 && (y - (rect[1] + rect[3] * 0.5)).abs() < 1e-2, "{x} {y} {rect:?}");
        // the frame's corner is the field's corner: tan(45 deg) across half the frame's width
        let c = crate::debug::cursor_ray(&cam, size, 90.0, (rect[0], rect[1]));
        let (fwd, right, up) = (cam.forward(), cam.right(), cam.up());
        assert!((c.dot(right) / c.dot(fwd) + 1.0).abs() < 1e-3 && (c.dot(up) / c.dot(fwd) - 9.0 / 16.0).abs() < 1e-3);
        // the mouse ray inverts the projection
        let (px, py, _) = crate::debug::project_any(&cam, size, 90.0, cam.pos + c * 20.0).unwrap();
        assert!((px - rect[0]).abs() < 1e-2 && (py - rect[1]).abs() < 1e-2);
    }

    #[test]
    fn letterbox_bar_rows() {
        // 1920 x 1080 at 2.39:1 shows 803 rows: 138 above and below (the odd row stays in the picture)
        assert_eq!(letterbox_bar(1920, 1080, 2.39), 138);
        // a window not shorter than the frame has no bars
        assert_eq!(letterbox_bar(1920, 1080, 1.0), 0);
        assert_eq!(letterbox_bar(1920, 1080, 1.7777), 0);
    }

    fn cand(cam: usize, sees: Option<f32>, rides: bool) -> Candidate {
        Candidate { cam, car: 0, sees, rides }
    }

    #[test]
    fn auto_cut_rules() {
        let run = |cur, since, left, lost| AutoRun { shot: Some(0), cur, since, left, lost };
        let pick = |r: &AutoRun, c: &[Candidate], t| auto_pick(r, c, t, 3.0, 0.7, 0.5);
        // a nearer camera waits for Min, then needs to be under 0.7x the distance
        let c = [cand(0, Some(50.0), false), cand(1, Some(30.0), false)];
        assert_eq!(pick(&run(0, 0.0, None, 0.0), &c, 2.9), None, "Min holds");
        assert_eq!(pick(&run(0, 0.0, None, 0.0), &c, 3.1), Some(1));
        let near = [cand(0, Some(40.0), false), cand(1, Some(30.0), false)];
        assert_eq!(pick(&run(0, 0.0, None, 0.0), &near, 10.0), None, "0.75x is not enough");
        // a lost car cuts before Min, once lost long enough
        let lost = [cand(0, None, false), cand(1, Some(80.0), false)];
        assert_eq!(pick(&run(0, 0.0, None, 0.3), &lost, 1.0), None);
        assert_eq!(pick(&run(0, 0.0, None, 0.6), &lost, 1.0), Some(1));
        // no cut-back to the camera just left within 2 x Min, unless it alone sees the car
        let back = [cand(0, Some(50.0), false), cand(1, Some(10.0), false), cand(2, Some(20.0), false)];
        assert_eq!(pick(&run(0, 0.0, Some(1), 0.0), &back, 4.0), Some(2));
        assert_eq!(pick(&run(0, 0.0, Some(1), 0.0), &back, 7.0), Some(1), "after 2 x Min it may come back");
        let only = [cand(0, None, false), cand(1, Some(10.0), false)];
        assert_eq!(pick(&run(0, 0.0, Some(1), 0.0), &only, 4.0), None, "not lost long enough");
        assert_eq!(pick(&run(0, 0.0, Some(1), 1.0), &only, 4.0), Some(1));
        // riding cameras: a fallback when no world camera sees the car, left for a world camera after Min
        let ride = [cand(0, None, false), cand(1, None, false), cand(2, Some(4.0), true)];
        assert_eq!(pick(&run(0, 0.0, None, 1.0), &ride, 1.0), Some(2));
        let back_to_world = [cand(0, Some(90.0), false), cand(2, Some(4.0), true)];
        assert_eq!(pick(&run(2, 0.0, None, 0.0), &back_to_world, 2.0), None);
        assert_eq!(pick(&run(2, 0.0, None, 0.0), &back_to_world, 3.5), Some(0));
        // nothing sees the car: stay
        assert_eq!(pick(&run(0, 0.0, None, 5.0), &[cand(0, None, false), cand(1, None, false)], 9.0), None);
        // the first pick: nearest world camera, else a riding one, else the pool's first
        assert_eq!(auto_first(&[cand(3, Some(40.0), false), cand(4, Some(20.0), false), cand(5, Some(2.0), true)]), 4);
        assert_eq!(auto_first(&[cand(3, None, false), cand(5, Some(2.0), true)]), 5);
        assert_eq!(auto_first(&[cand(3, None, false), cand(4, None, false)]), 3);
    }

    #[test]
    fn exported_tv_station_reproduces_the_camera() {
        use motor_sim::tvcam;
        let text = "Race Track 1 Cars 1 2\nCamera a static Eye 10 3 20 At 40 1 80 Zoom 2.5\nCamera b static Eye -5 4 0 Target car 0 Zoom auto 20\nCamera c chase Target car 0\nShot 0 a\n";
        let (seq, _) = parse(text).unwrap();
        let (keys, _) = tv_keys(&seq, 0.0);
        assert_eq!(keys.len(), 2);
        let m = Motion { version: 1, channels: 9, keys };
        let ks = tvcam::keys(&m);
        // a car near `a`: fixed aim and zoom
        let s = tvcam::shot(&ks, Vec3::new(12.0, 0.0, 25.0), Mat3::IDENTITY).unwrap();
        assert!(s.eye.distance(Vec3::new(10.0, 3.0, 20.0)) < 1e-4);
        assert!(s.rot.z_axis.distance(look_rot(Vec3::new(10.0, 3.0, 20.0), Vec3::new(40.0, 1.0, 80.0)).z_axis) < 1e-4);
        assert!((s.zoom - 2.5).abs() < 1e-4);
        // near `b`: looks at the car, the zoom follows the distance as auto_zoom does
        for d in [10.0f32, 40.0, 100.0, 1000.0] {
            let car = Vec3::new(-5.0, 4.0, -d);
            let s = tvcam::shot(&ks, car, Mat3::IDENTITY).unwrap();
            assert!(s.rot.z_axis.distance((car - s.eye).normalize()) < 1e-4);
            assert!((s.zoom - auto_zoom(d, 20.0)).abs() < 1e-3, "d {d}: {} vs {}", s.zoom, auto_zoom(d, 20.0));
        }
    }

    /// #425: an orbit line parses with its defaults, round-trips, and refuses `Target nearest`.
    #[test]
    fn orbit_line_round_trips() {
        let (s, w) = parse("Camera a orbit Target leader\nCamera b orbit Target car 1 Radius 8 Height -2 Speed -45 Start 90 Lag 0 Zoom auto 20 Clear on\nShot 0 a\n").unwrap();
        assert!(w.is_empty(), "{w:?}");
        assert_eq!(s.cameras[0].kind, CamKind::Orbit { target: Target::Leader, radius: ORBIT_RADIUS, height: ORBIT_HEIGHT, speed: ORBIT_SPEED, start: 0.0, lag: ORBIT_LAG, zoom: Zoom::Fixed(1.0), clear: false });
        assert!(matches!(s.cameras[1].kind, CamKind::Orbit { radius, speed, zoom: Zoom::Auto(_), clear: true, .. } if radius == 8.0 && speed == -45.0));
        let (s2, w2) = parse(&to_text(&s)).unwrap();
        assert!(w2.is_empty(), "{w2:?}");
        assert_eq!(s, s2);
        let (s3, w3) = parse("Camera a orbit Target nearest\nCamera b orbit Radius 5\nCamera c orbit Target leader Radius 0\nCamera d static Eye 0 0 0 At 1 0 0\nShot 0 d\n").unwrap();
        assert_eq!(w3.len(), 3, "{w3:?}");
        assert_eq!(s3.cameras.len(), 1);
    }

    #[test]
    fn orbit_eye_circles_the_car() {
        let c = Vec3::new(10.0, 1.0, 20.0);
        let near = |a: Vec3, b: Vec3| a.distance(b) < 1e-4;
        // a car heading +z: angle 0 is straight behind, 90 on its right (+x), 180 in front
        assert!(near(orbit_eye(c, Vec3::Z, 12.0, 4.0, 0.0), c + Vec3::new(0.0, 4.0, -12.0)));
        assert!(near(orbit_eye(c, Vec3::Z, 12.0, 4.0, 90.0), c + Vec3::new(12.0, 4.0, 0.0)));
        assert!(near(orbit_eye(c, Vec3::Z, 12.0, 4.0, 180.0), c + Vec3::new(0.0, 4.0, 12.0)));
        // the circle turns with the car: heading +x, angle 90 is on its right (-z)
        assert!(near(orbit_eye(c, Vec3::X, 5.0, 0.0, 90.0), c + Vec3::new(0.0, 0.0, -5.0)));
    }
}
