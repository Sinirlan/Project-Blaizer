//! Cine (#32, re-notes/20-cine.md): planned camera sequences. A `.cine` file describes an all-AI race
//! and a set of cameras with the shots that put them on air; `Director` turns the race state into the
//! camera of a moment, `cine_render` records the result as a video. Plain `Key value` words like
//! `.fill` (procgen/fill.rs): `#` starts a comment, a bad line is reported and skipped, game
//! coordinates, angles in degrees.
//!
//! ```text
//! # Motorhead cine v1
//! Race Track 1 Cars 1 2 3 4 5 6 7 8 Difficulty medium Laps 3 Catchup on Start grid|go     # time 0 = the "3" of the countdown (grid) or the green light (go)
//! Output Size 1920 1080 Fps 60
//! End 40                              # seconds; omitted = every car finished + 2 s
//! Camera gantry static Eye x y z Target car 3 Zoom auto 25 Lag 0.3 Lead 0.2 Clear on
//! Camera hill   static Eye x y z At x y z Zoom 1.5               # fixed aim
//! Camera roof   mounted Target leader Offset 0 1.6 -0.5 Angles 5 0 0 Zoom 1 Hide off
//! Camera follow chase|far|incar|bumper|tv Target nearest
//! Camera fly    path Drive time|car N Aim keys|tangent|Target T Lag 0.2 Zoom 1  # a keyed path (#151)
//! Key fly 0 Eye x y z Angles 5 90 0 Fov 60 Tcb 0 0 0 Linear off    # keys of a path / zoom keys of any camera
//! Shake gantry proximity 2 Radius 15  # proximity|handheld|impact, amplitude in degrees
//! Shot 0 gantry Speed 0.5 FadeIn 1 FadeOut 1   # from this second on the camera is on air
//! ```
//! `Target`: `car N` (grid slot, 0-based, order of `Cars`), `leader`, `nearest` (static cameras: the
//! car nearest the eye), `player` (the last slot, as in the viewer's line-up).

use crate::drive::{cam_origin, CamTune, PoseCam};
use crate::render::Camera;
use glam::{Mat3, Vec3};
use motor_data::cfg::{Motion, MotionKey};
use motor_sim::props::evaluate;
use motor_sim::tvcam::{euler_to_rot, TvShot};
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

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Target {
    /// grid slot (index into `RaceDef::cars`)
    Car(usize),
    Leader,
    Nearest,
    Player,
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
}

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
    /// tension, continuity, bias
    pub tcb: [f32; 3],
    pub linear: bool,
}

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
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Shot {
    pub t: f32,
    pub cam: usize,
    /// race seconds per video second (slow motion below 1)
    pub speed: f32,
    /// seconds from black at the start / to black at the end of the shot
    pub fade_in: f32,
    pub fade_out: f32,
}

impl Shot {
    pub fn new(t: f32, cam: usize) -> Shot {
        Shot { t, cam, speed: 1.0, fade_in: 0.0, fade_out: 0.0 }
    }
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
    pub end: Option<f32>,
    pub cameras: Vec<CineCam>,
    pub shots: Vec<Shot>,
}

impl Default for RaceDef {
    fn default() -> Self {
        RaceDef { track: 1, cars: (1..=8).collect(), difficulty: Difficulty::Medium, laps: None, catchup: true, start_go: false }
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
        ["car", n] => n.parse().map(Target::Car).map_err(|_| format!("Target car N: bad slot `{n}`")),
        _ => Err("Target car N | leader | nearest | player".into()),
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
        k => {
            let mode = POSE_MODES.iter().find(|m| m.0 == k).map(|m| m.1).ok_or(format!("camera kind `{k}`: static, mounted, path, chase, far, incar, bumper or tv"))?;
            let f = fields(rest, &["target"])?;
            let t = target(f.get("target").ok_or("needs Target")?)?;
            if t == Target::Nearest {
                return Err("Target nearest is for static cameras".into());
            }
            CamKind::Pose { mode, target: t }
        }
    };
    if let CamKind::Mounted { target: Target::Nearest, .. } | CamKind::Path { aim: PathAim::Track { target: Target::Nearest, .. }, .. } = kind {
        return Err("Target nearest is for static cameras".into());
    }
    Ok((CineCam { name: name.to_string(), kind, keys: Vec::new(), shakes: Vec::new() }, aim_given))
}

/// `Key NAME T ...`: the camera name and its key.
fn parse_key(w: &[&str]) -> Result<(String, CamKey), String> {
    let [name, at, rest @ ..] = w else { return Err("Key CAMERA T ...".into()) };
    let at = nums(&[at], 1, "Key time")?[0];
    let f = fields(rest, &["eye", "angles", "fov", "tcb", "linear"])?;
    let mut key = CamKey { at, eye: None, angles: None, fov: None, tcb: [0.0; 3], linear: false };
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
    if key.eye.is_none() && key.angles.is_none() && key.fov.is_none() {
        return Err("Key needs Eye, Angles or Fov".into());
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

/// `Shot T NAME [Speed S] [FadeIn s] [FadeOut s]`: time, camera name and the shot (its camera unresolved).
fn parse_shot(w: &[&str]) -> Result<(f32, String, Shot), String> {
    let [t, cam, rest @ ..] = w else { return Err("Shot T CAMERA [Speed S] [FadeIn s] [FadeOut s]".into()) };
    let t = match t.parse::<f32>() {
        Ok(t) if t >= 0.0 && t.is_finite() => t,
        _ => return Err(format!("Shot: bad time `{t}`")),
    };
    let f = fields(rest, &["speed", "fadein", "fadeout"])?;
    let mut shot = Shot::new(t, 0);
    if let Some(v) = f.get("speed") {
        shot.speed = nums(v, 1, "Speed")?[0];
        if !(SPEED_RANGE.0..=SPEED_RANGE.1).contains(&shot.speed) {
            return Err(format!("Speed is {}..{}", SPEED_RANGE.0, SPEED_RANGE.1));
        }
    }
    for (k, out) in [("fadein", &mut shot.fade_in), ("fadeout", &mut shot.fade_out)] {
        if let Some(v) = f.get(k) {
            *out = nums(v, 1, "Fade")?[0];
            if *out < 0.0 {
                return Err("a fade is >= 0 seconds".into());
            }
        }
    }
    Ok((t, cam.to_string(), shot))
}

/// Parse a `.cine` text. Bad lines are skipped and returned as warnings; an unusable sequence is an Err.
pub fn parse(text: &str) -> Result<(Sequence, Vec<String>), String> {
    let mut seq = Sequence { race: RaceDef::default(), audio: AudioDef::default(), size: (1920, 1080), fps: 60, end: None, cameras: Vec::new(), shots: Vec::new() };
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
            "output" => fields(rest, &["size", "fps"]).and_then(|f| {
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
            "shot" => parse_shot(rest).map(|(t, name, shot)| shot_lines.push((i + 1, t, name, shot))),
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
            if key.fov.is_none() {
                continue;
            }
        }
        // a channel keyed twice at the same time keeps its first key
        if let Some(o) = c.keys.iter().find(|o| o.at == key.at) {
            let dup = (o.eye.is_some() && key.eye.is_some()) || (o.angles.is_some() && key.angles.is_some()) || (o.fov.is_some() && key.fov.is_some());
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
    for (line, _, name, mut shot) in shot_lines {
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
            CamKind::Mounted { target, .. } | CamKind::Pose { target, .. } => Some(*target),
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
    o += &format!("Output Size {} {} Fps {}\n", s.size.0, s.size.1, s.fps);
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
                o += &format!(" Clear {}\n", if *clear { "on" } else { "off" });
            }
            CamKind::Mounted { target, offset, angles, zoom, hide } => {
                o += &format!("mounted Target {} Offset {} Angles {} Zoom {zoom} Hide {}\n", target_text(*target), v3(*offset), v3(*angles), if *hide { "on" } else { "off" });
            }
            CamKind::Pose { mode, target } => {
                o += &format!("{} Target {}\n", POSE_MODES.iter().find(|m| m.1 == *mode).map_or("chase", |m| m.0), target_text(*target));
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
                o += &format!(" Clear {}\n", if *clear { "on" } else { "off" });
            }
        }
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
        o += &format!("Shot {} {}", sh.t, s.cameras[sh.cam].name);
        if sh.speed != 1.0 {
            o += &format!(" Speed {}", sh.speed);
        }
        if sh.fade_in > 0.0 {
            o += &format!(" FadeIn {}", sh.fade_in);
        }
        if sh.fade_out > 0.0 {
            o += &format!(" FadeOut {}", sh.fade_out);
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
    seq.shots.iter().rev().find(|s| s.cam == cam && s.t <= t).map_or(t, |s| t - s.t)
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

/// APPROX [design choice]: auto zoom keeps a car at about the same size on screen, zoom 1.0 at `r` metres.
pub fn auto_zoom(dist: f32, r: f32) -> f32 {
    (dist / r).clamp(1.0, ZOOM_MAX)
}

/// Far-plane multiplier that goes with a zoom (the TV camera grows it with the zoom, 0x40b564) [I].
fn far_k(zoom: f32) -> f32 {
    zoom.clamp(1.0, FAR_K_MAX)
}

/// The zoom of a horizontal field of view: 1.0 at the views' `FOV_X` (90 degrees).
pub fn fov_zoom(fov_deg: f32) -> f32 {
    (crate::render::FOV_X * 0.5 * DEG).tan() / (fov_deg * 0.5 * DEG).tan()
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
    tune: CamTune,
    focus: usize,
    hidden: Option<usize>,
    /// per camera, as of the last `camera` call: the camera, the slot it shows, whether it hides that car (the editor's preview)
    views: Vec<Option<(Camera, usize, bool)>>,
}

/// The car slot a `Target` means now (`eye`: the camera position, for `nearest`).
pub fn resolve(race: &Race, t: Target, eye: Option<Vec3>) -> usize {
    let last = race.racers.len() - 1;
    match t {
        Target::Car(n) => n.min(last),
        Target::Player => last,
        Target::Leader => race.standings().first().copied().unwrap_or(0),
        Target::Nearest => match eye {
            Some(e) => (0..=last).min_by(|&a, &b| cam_origin(&race.racers[a].car).distance_squared(e).total_cmp(&cam_origin(&race.racers[b].car).distance_squared(e))).unwrap_or(0),
            None => race.standings().first().copied().unwrap_or(0),
        },
    }
}

/// A camera at `eye` following a car (static `Target`, path `Target`): eased aim, lead, auto zoom, clear view.
#[allow(clippy::too_many_arguments)]
fn track_cam(race: &Race, eye: Vec3, target: Target, lag: f32, lead: f32, zoom: Zoom, clear: bool, state: &mut Option<Vec3>, dt: f32) -> (Camera, usize) {
    let slot = resolve(race, target, Some(eye));
    let car = &race.racers[slot].car;
    let pos = cam_origin(car);
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
        Director { states, curves, shake, tune: CamTune::default(), focus: 0, hidden: None, views: vec![None; seq.cameras.len()] }
    }

    /// Camera `i` as of the last `camera` call (on air or not): the camera, the slot it shows and whether it hides that car.
    pub fn view(&self, i: usize) -> Option<(Camera, usize, bool)> {
        self.views.get(i).copied().flatten()
    }

    /// The slot of the car the camera on air looks at (the session's followed car: env probe, emblem).
    pub fn focus(&self) -> usize {
        self.focus
    }

    /// The slot whose body the camera on air sits inside and so must not be drawn.
    pub fn hidden(&self) -> Option<usize> {
        self.hidden
    }

    /// Advance every camera by `dt` (the race has been stepped to `t`) and return the one on air.
    pub fn camera(&mut self, seq: &Sequence, race: &Race, t: f32, dt: f32) -> Camera {
        let air = on_air(seq, t);
        let mut on = None;
        for (i, c) in seq.cameras.iter().enumerate() {
            let since = since_shot(seq, i, t);
            let (mut cam, slot, hide, param, rides) = self.update(i, c, race, since, dt);
            // zoom keys override the camera's own zoom
            if let Some(m) = &self.curves[i].fov {
                let z = fov_zoom(evaluate(m, param, false)[0]);
                (cam.zoom, cam.far_k) = (z, far_k(z));
            }
            self.apply_shake(i, c, &mut cam, race, rides.then_some(slot), t, dt);
            self.views[i] = Some((cam, slot, hide));
            if i == air {
                on = Some(cam);
                (self.focus, self.hidden) = (slot, hide.then_some(slot));
            }
        }
        on.unwrap_or_else(|| Camera::new(Vec3::ZERO, 0.0, 0.0))
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
                    let (cam, slot) = track_cam(race, *eye, *target, *lag, *lead, *zoom, *clear, state, dt);
                    (cam, slot, false, since, false)
                }
            },
            (CamKind::Mounted { target, offset, angles, zoom, hide }, State::Mounted) => {
                let slot = resolve(race, *target, None);
                let car = &race.racers[slot].car;
                let eye = cam_origin(car) + car.rot * *offset;
                (Camera::from_rot(eye, car.rot * euler_to_rot(*angles * DEG), *zoom, far_k(*zoom)), slot, *hide, since, true)
            }
            (CamKind::Pose { mode, target }, State::Pose(pc)) => {
                let slot = resolve(race, *target, None);
                let car = &race.racers[slot].car;
                (pc.camera(race, &self.tune, cam_origin(car), car.rot, dt), slot, mode.hides_car(), since, true)
            }
            (CamKind::Path { drive, aim, zoom, clear }, State::Path { aim: state, dir }) => {
                let cv = &self.curves[i];
                let Some(path) = &cv.eye else { unreachable!("parse drops a path without Eye keys") };
                // the key parameter, and a step for the tangent
                let (u, du, mut slot) = match drive {
                    Drive::Time => (since, 0.02, 0),
                    Drive::Car(t) => {
                        let slot = resolve(race, *t, None);
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
                        let (cam, s) = track_cam(race, eye, *target, *lag, *lead, *zoom, *clear, state, dt);
                        slot = s;
                        cam
                    }
                };
                (cam, slot, false, u, false)
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

    fn key(at: f32, x: f32, tcb: [f32; 3]) -> CamKey {
        CamKey { at, eye: Some(Vec3::new(x, 0.0, 0.0)), angles: None, fov: None, tcb, linear: false }
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
}
