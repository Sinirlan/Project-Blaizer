//! A complete race: track, grid, cars, bots, countdown, progress and standings.

use crate::ai::{GameAi, Temper, Traffic};
use crate::body::{self, Hull};
use crate::car::{self, Car, CarParams, Input, Telemetry};
use crate::props::Props;
use crate::race::{Centreline, Grid, Progress};
use crate::world::World;
use glam::Vec3;
use motor_data::{aidat, cfg, fs};
use std::path::Path;

/// Pre-race before GO, seconds: the 6 s start fly-by, then "3", "2", "1" of `COUNT_DIGIT` each
/// ("!32.wav" ... "GO!.wav"). GO = race clock base + 12000 ms (0x45a42b), 08 §8.16 [O].
pub const COUNTDOWN: f32 = 12.0;
/// Seconds each countdown digit shows (windows 0x45e3c7..0x45e64f).
pub const COUNT_DIGIT: f32 = 2.0;
/// Seconds of the countdown that are the digits (the rest is the fly-by).
pub const COUNT_DIGITS_S: f32 = 3.0 * COUNT_DIGIT;
motor_tune::tunable! {
    /// Reset a car that has had too few wheels down this long (0x40d854, 0x4cf4a3) [O]: fewer than
    /// 2, or 2 that are not the wheels of one side; the time holds with a side pair and clears with 3 or 4.
    /// range: 1..60
    /// unit: s
    RESET_AFTER: f32 = 8.0;
    /// Reset a car that has touched the world upside down (c[0xa4], up.y < -0.5) for this long in
    /// total since it last had 3 wheels down (0x40d854, 0x4cf49b) [O], #368.
    /// range: 0.5..60
    /// unit: s
    FLIP_RESET_AFTER: f32 = 2.0;
}

/// The timer part of the reset check `0x40d854`, once per frame: `wheel_s` (c[0xc4]) and `flip_s`
/// (c[0xc0]) after this frame; `reset_due` says whether they reset the car. `wheels` = on ground FL, FR, BR, BL
/// (P+0x134..0x140), `flip` = c[0xa4] (`Car::flip_contact`). The frame time is capped at 0.5 s.
// CLASSIC 0x40d854
pub fn reset_timers(wheel_s: f32, flip_s: f32, wheels: [bool; 4], flip: bool, dt: f32) -> (f32, f32) {
    let dt = if dt > 0.5 { 0.5 } else { dt };
    let mut flip_s = flip_s;
    if flip {
        flip_s += dt;
    }
    if flip_s > FLIP_RESET_AFTER.get() {
        return (wheel_s, flip_s);
    }
    let n = wheels.iter().filter(|&&w| w).count();
    let side_pair = (wheels[0] && wheels[3]) || (wheels[1] && wheels[2]); // FL+BL, FR+BR (0x40d8d9..0x40d8f7)
    let mut wheel_s = wheel_s;
    if n < 2 || (n == 2 && !side_pair) {
        wheel_s += dt;
    }
    if n > 2 {
        (wheel_s, flip_s) = (0.0, 0.0);
    }
    (wheel_s, flip_s)
}

/// 0x40d854 returns 0 (reset) when either timer is past its limit.
pub fn reset_due(wheel_s: f32, flip_s: f32) -> bool {
    flip_s > FLIP_RESET_AFTER.get() || wheel_s > RESET_AFTER.get()
}

/// A driver from League/Profiles/*.adp (or the human player).
#[derive(Debug, Clone, Default)]
pub struct Driver {
    /// PlayerName
    pub name: String,
    /// PlayerNick
    pub nick: String,
    /// PlayerPicture, a 90x56 grayscale TGA in League/Profiles/Gfx (Game/player.tga for you)
    pub picture: String,
    /// PlayerColour (RGB 0..255): minimap marker, nameplate and split-list colour
    pub colour: [u8; 3],
    /// CarColour1..3 of the driver record (+0x712c99..): what the painter 0x47b9c4 turns the car
    /// file's Colour1..3 into at race load (0x45f36e). None: the car keeps its own paint.
    pub car_colours: Option<[[u8; 3]; 3]>,
    pub temper: Temper,
    /// CHEAT avenger / zx (0x412aa4, #322): the minimap draws this racer's dot as a ninja. Set by the caller
    /// once per race (`cheats::ninja_marker`), never saved, not in recordings.
    pub ninja_marker: bool,
}

pub struct Racer {
    pub car_id: u32,
    pub driver: Driver,
    /// c[0xc4]: seconds with too few wheels down (`reset_timers`, 0x40d854: reset after 8 s)
    pub airborne_s: f32,
    /// c[0xc0]: seconds upside down against the world (`reset_timers`, 0x40d854: reset after 2 s)
    pub flip_s: f32,
    /// AI stuck timer (0x40d854 +0xbc/+0xf8): armed by the reverse manoeuvre, cleared above
    /// 15 m/s, reset after 8 s
    stuck: Option<f32>,
    pub car: Car,
    pub progress: Progress,
    /// the game AI; for the human player it only tracks position (traffic data for others)
    pub ai: GameAi,
    pub human: bool,
    /// unscaled gear tops / acc force, for the rubber band
    base_gear_top: Vec<f32>,
    base_acc: f32,
    pub telemetry: Telemetry,
    /// this frame's brake input (0..1), for the brake lights
    pub brake: f32,
    /// a car posed from outside (Ghost mode's recorded cars, `demo::drive_remote`): no physics step,
    /// no collisions, no respawn, in the original the "remote" flag (`car+0xc`, 03-physics `c[0x8c]`) [D].
    /// Its lap progress still runs.
    pub remote: bool,
}

/// What a racer changes while the race runs (rewind; the rest of `Racer` is fixed at load).
#[derive(Clone)]
struct RacerState {
    airborne_s: f32,
    flip_s: f32,
    stuck: Option<f32>,
    car: Car,
    progress: Progress,
    ai: crate::ai::AiState,
    telemetry: Telemetry,
    brake: f32,
}

/// The race's moving state at one instant (`Race::snapshot`): motorview's rewind.
#[derive(Clone)]
pub struct Snapshot {
    racers: Vec<RacerState>,
    countdown: f32,
    clock_ms: f64,
    ai_vector: [f32; 11],
    props: crate::props::PropState,
    acc: f32,
    prev: Vec<ViewPose>,
}

/// Which AI parameter row of the .trk a race uses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AiRow {
    /// AI1..AI3 by difficulty
    Difficulty(Difficulty),
    /// AID0..AID3: league races use the human's division (0x458ecf)
    Division(u32),
}

/// Difficulty: index into AI1..AI3 of the .trk. `Beginner` is ours (not in the original): the
/// Easy row slowed down by `beginner_row`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Difficulty {
    Easy = 0,
    Medium = 1,
    Hard = 2,
    Beginner = 3,
}

/// Race rules. `Elimination` is our design (Knockout mode, not in the original): laps = cars - 1
/// and the last car over the line at the end of each lap is out.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Rules {
    #[default]
    Normal,
    Elimination,
}

pub struct Race {
    pub world: World,
    pub line: Centreline,
    pub ai_line: aidat::RacingLine,
    pub racers: Vec<Racer>,
    pub laps: u32,
    pub rules: Rules,
    /// seconds left before GO
    pub countdown: f32,
    /// race clock in ms since GO
    pub clock_ms: f64,
    /// dev/test hook (headless `--autopilot`): the human's car is driven by the AI
    pub autopilot: bool,
    grid: Grid,
    /// active AI parameter vector (0x74e890)
    pub ai_vector: [f32; 11],
    /// scene objects (SceneName): animated scenery and props
    pub props: Props,
    /// the track's goal camera (eye, look-at point), game space
    pub goal_camera: Option<(Vec3, Vec3)>,
    /// the TV camera's stations (`CameraSplineName`); empty if the track has none
    pub tv_camera: Vec<crate::tvcam::TvKey>,
    /// the start fly-by's eye splines (`StartSpline1/2`)
    pub start_camera: crate::startcam::StartCamera,
    /// CHEAT gulliver: the size of every car and the feel of the human's (1.0 = the original;
    /// `set_car_scale`)
    pub car_scale: (f32, f32),
    /// the original's driver/team name cheat of this race (`cheats::NameCheat::of`, 08 §8.14); set by
    /// the caller when the race is handed over, rebuilt for every race, never saved, not in snapshots
    pub name_cheat: Option<crate::cheats::NameCheat>,
    /// diagnosis (aicompare, #381): the other cars collide with the remote cars too. A remote car is posed from outside
    /// every frame, so it pushes the others and keeps its recorded motion. Off in the game (Ghost mode's cars are ghosts)
    pub solid_remote: bool,
    /// the spring length and rate factors the cars are at (demon / grem, #313; (1, 1) = as loaded)
    susp: (f32, f32),
    /// design choice (#374): the fixed physics tick in seconds (`set_fixed_tick`); `None` steps once per rendered
    /// frame like the original (0x4814ac), whose damper then follows the frame rate
    tick_s: Option<f32>,
    /// the same tick as `set_fixed_tick` was given it, in Hz (`fixed_hz`: the replay header keeps it exactly)
    tick_hz: Option<f32>,
    /// real time stepped on the fixed tick minus the frames' time, seconds, in (-tick, 0] after a frame: the ticks run
    /// up to the frame's end and the view is drawn `-acc` before the latest tick
    acc: f32,
    /// the cars' view poses at the start of the latest tick (`view_pose`); empty = the current poses
    prev: Vec<ViewPose>,
    /// how many ticks the latest `step_as` ran (the variable path: always 1)
    ticks_last_frame: u32,
}

/// What the renderer draws of a car, between two physics ticks (`Race::view_pose`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ViewPose {
    pub pos: Vec3,
    pub rot: glam::Mat3,
    pub wheel_ext: [f32; 4],
    pub steer_smooth: f32,
}

impl ViewPose {
    pub fn of(car: &Car) -> ViewPose {
        ViewPose { pos: car.pos, rot: car.rot, wheel_ext: car.wheel_ext, steer_smooth: car.steer_smooth }
    }

    /// `self` at t = 0, `b` at t = 1: positions, wheels and steering lerped, the rotation slerped.
    pub fn lerp(&self, b: &ViewPose, t: f32) -> ViewPose {
        let (qa, qb) = (glam::Quat::from_mat3(&self.rot), glam::Quat::from_mat3(&b.rot));
        ViewPose {
            pos: self.pos.lerp(b.pos, t),
            rot: glam::Mat3::from_quat(qa.slerp(qb, t)),
            wheel_ext: std::array::from_fn(|i| self.wheel_ext[i] + (b.wheel_ext[i] - self.wheel_ext[i]) * t),
            steer_smooth: self.steer_smooth + (b.steer_smooth - self.steer_smooth) * t,
        }
    }
}

motor_tune::tunable! {
    /// Design choice (#374): the fixed physics tick of the game, ticks per second. 60 Hz is the game's per-frame
    /// stepping at 60 fps exactly; the damper keeps its 1/90 s scaling (`SUSP_DAMP_REF_H3`), the original's ~30 fps feel.
    /// range: 30..240
    /// unit: Hz
    /// applies: restart
    pub PHYSICS_HZ: f32 = 60.0;
}

motor_tune::tunable! {
    /// CHEAT gulliver (ours, 08 §8.14): the cars' size while the cheat is on. A race takes it when the cheat is set.
    /// range: 0.05..1
    /// applies: restart
    pub GULLIVER_SCALE: f32 = 0.2;
    /// CHEAT gulliver: the human's car turns, rolls and pitches like a car of this size
    /// (`CarParams::scaled`; 1.0 = the full-size response, `GULLIVER_SCALE` = a car scaled for real)
    /// [design choice]. AI cars keep 1.0: the game's AI steers for the full-size response, and at
    /// 0.6 its races took 10-35 % longer with resets (motorrace, #140).
    /// range: 0.05..1
    /// applies: restart
    pub GULLIVER_FEEL: f32 = 0.6;
}

/// The AI parameter vector of a race (04 §4.3): the .trk's AI1..AI3 by difficulty, or AID0..AID3
/// by division; the defaults from 0x4210xx fill what the file leaves out.
pub fn ai_vector(trk: &cfg::Config, row: AiRow) -> [f32; 11] {
    let key = match row {
        AiRow::Difficulty(Difficulty::Easy | Difficulty::Beginner) => "AI1".to_string(),
        AiRow::Difficulty(Difficulty::Medium) => "AI2".to_string(),
        AiRow::Difficulty(Difficulty::Hard) => "AI3".to_string(),
        AiRow::Division(n) => format!("AID{}", n.min(3)),
    };
    let mut v = [7.0, 0.0, 7.0, 0.0, 10.0, 1.0, 0.95, 1.0, 3.0, 18.0, 0.0];
    if let Some(e) = trk.get(&key) {
        for (k, x) in v.iter_mut().enumerate() {
            if let Some(y) = e.f32(k) {
                *x = y;
            }
        }
    }
    if row == AiRow::Difficulty(Difficulty::Beginner) {
        beginner_row(&mut v);
    }
    v
}

// Beginner level [design choice, ours]: the track's Easy row with the corner speed law and the
// corner target speed scaled down (`a[8]`, `a[9]`) and the car's speed / acceleration levels
// blended towards weak ones (`a[0..4]`, the blend of 0x40ccc0); the rubber band and the grip and
// brake terms stay as on Easy. The AI cars are about `BEGINNER_*` slower per lap than Easy
// (motorrace, 04 §4.3). A race takes these when it is built.
motor_tune::tunable! {
    /// Beginner level [design choice]: the factor on the corner speed law (`a[8]`).
    /// range: 0.2..1
    /// applies: restart
    BEGINNER_CORNER_GRIP: f32 = 0.55;
    /// Beginner level [design choice]: the factor on the corner target speed (`a[9]`).
    /// range: 0.2..1
    /// applies: restart
    BEGINNER_CORNER_BASE: f32 = 0.75;
    /// Beginner level [design choice]: the speed level the cars are blended towards (`a[0]`).
    /// range: 0..10
    /// applies: restart
    BEGINNER_SPEED_LEVEL: f32 = 3.0;
    /// Beginner level [design choice]: how far the speed level is blended towards `BEGINNER_SPEED_LEVEL` (`a[1]`).
    /// range: 0..1
    /// applies: restart
    BEGINNER_SPEED_BLEND: f32 = 0.7;
    /// Beginner level [design choice]: the acceleration level the cars are blended towards (`a[2]`).
    /// range: 0..10
    /// applies: restart
    BEGINNER_ACC_LEVEL: f32 = 3.0;
    /// Beginner level [design choice]: how far the acceleration level is blended towards `BEGINNER_ACC_LEVEL` (`a[3]`).
    /// range: 0..1
    /// applies: restart
    BEGINNER_ACC_BLEND: f32 = 0.7;
}

fn beginner_row(v: &mut [f32; 11]) {
    (v[0], v[1]) = (BEGINNER_SPEED_LEVEL.get(), BEGINNER_SPEED_BLEND.get());
    (v[2], v[3]) = (BEGINNER_ACC_LEVEL.get(), BEGINNER_ACC_BLEND.get());
    v[8] *= BEGINNER_CORNER_GRIP.get();
    v[9] *= BEGINNER_CORNER_BASE.get();
}

fn first_value(c: &cfg::Config, k: &str) -> Result<String, String> {
    c.get(k).and_then(|e| e.values.first().cloned()).ok_or(format!("track file: no {k}"))
}

impl Race {
    /// The moving state (cars, AI, progress, scene objects, clocks); the world is not part of it.
    pub fn snapshot(&self) -> Snapshot {
        let racers = self
            .racers
            .iter()
            .map(|r| RacerState { airborne_s: r.airborne_s, flip_s: r.flip_s, stuck: r.stuck, car: r.car.clone(), progress: r.progress.clone(), ai: r.ai.state(), telemetry: r.telemetry, brake: r.brake })
            .collect();
        Snapshot { racers, countdown: self.countdown, clock_ms: self.clock_ms, ai_vector: self.ai_vector, props: self.props.state(), acc: self.acc, prev: self.prev.clone() }
    }

    /// Go back to a snapshot of this race. The cars keep the race's current size (`car_scale`).
    pub fn restore(&mut self, s: &Snapshot) {
        let g = self.world.gravity;
        let (scale, feel) = self.car_scale;
        for (r, d) in self.racers.iter_mut().zip(&s.racers) {
            (r.airborne_s, r.flip_s, r.stuck, r.car, r.progress, r.telemetry, r.brake) = (d.airborne_s, d.flip_s, d.stuck, d.car.clone(), d.progress.clone(), d.telemetry, d.brake);
            r.ai.set_state(&d.ai);
            let feel = if r.human { feel } else { 1.0 };
            if !r.remote && (r.car.p.scale, r.car.p.feel) != (scale, feel) {
                r.car.rescale(scale, feel, g);
            }
        }
        (self.countdown, self.clock_ms, self.ai_vector) = (s.countdown, s.clock_ms, s.ai_vector);
        self.props.set_state(&s.props);
        (self.acc, self.prev) = (s.acc, s.prev.clone());
    }

    /// `cars[i]` goes to grid slot `i`; `player` is the slot driven by the caller (or none).
    pub fn load(root: &Path, track: u32, cars: &[u32], player: Option<usize>, laps: Option<u32>) -> Result<Race, String> {
        Self::load_with(root, track, cars, player, laps, Difficulty::Medium)
    }

    pub fn load_with(root: &Path, track: u32, cars: &[u32], player: Option<usize>, laps: Option<u32>, difficulty: Difficulty) -> Result<Race, String> {
        Self::load_field(root, track, cars, player, laps, AiRow::Difficulty(difficulty), None)
    }

    /// `drivers[i]` drives slot `i` (None: the profiles in file order, as before).
    pub fn load_field(root: &Path, track: u32, cars: &[u32], player: Option<usize>, laps: Option<u32>, row: AiRow, drivers: Option<&[Driver]>) -> Result<Race, String> {
        let trk = fs::config(root, &format!("Game/Track{track}.trk"))?;
        let world = World::load(root, track)?;
        let line = crate::race::load_centreline(root, &trk)?;
        let ai_line = aidat::parse(&fs::read(root, &format!("{}/AI/ai.dat", first_value(&trk, "AITrackName")?))?).map_err(|e| e.to_string())?;
        let grid = match trk.get("StartGrid").map(|e| (e.f32(0), e.f32(1), e.f32(2), e.f32(3))) {
            Some((Some(row), Some(goal), Some(space), Some(tint))) => Grid { row, goal, space, tint },
            _ => Grid::default(),
        };
        let laps = laps.unwrap_or_else(|| trk.get("Laps").and_then(|e| e.i64(0)).unwrap_or(3) as u32);
        let ai_vector = ai_vector(&trk, row);
        let profiles = if drivers.is_none() { load_drivers(root) } else { Vec::new() };
        let mut racers = Vec::new();
        for (slot, &id) in cars.iter().enumerate() {
            let cfg = fs::config(root, &format!("Game/car{id:02}.car"))?;
            let human = Some(slot) == player;
            let mut params = if human { CarParams::from_config(&cfg)? } else { CarParams::for_ai(&cfg, &ai_vector)? };
            // the car's hull for contacts with props; the box over its corners stays when the file is missing
            if let Some(h) = cfg.get("CollisionName").and_then(|e| e.values.first()).and_then(|n| fs::read(root, n).ok()).and_then(|b| motor_data::col::parse(&b).ok()) {
                params.hull = Hull::from_col(&h);
            }
            // CLASSIC 0x409033 / 0x409039: the traffic size A+0x44 / A+0x48 comes from the table at 0x4a7db0, 2.1 x 4.4
            // for every slot, not from the car file's Width / Length [O, #379]
            let (width, length) = (crate::ai::TRAFFIC_WIDTH.get(), crate::ai::TRAFFIC_LENGTH.get());
            let driver = if human {
                Driver { name: "Player".into(), nick: "You".into(), picture: "Game/player.tga".into(), colour: [60, 100, 255], car_colours: None, temper: Temper::default(), ninja_marker: false }
            } else if let Some(d) = drivers {
                d.get(slot).cloned().unwrap_or_default()
            } else {
                profiles.get(slot % profiles.len().max(1)).cloned().unwrap_or_default()
            };
            let ai = GameAi::new(&ai_line, &ai_vector, params.mass, driver.temper, slot, width, length);
            let (base_gear_top, base_acc) = (params.gear_top.clone(), params.acc_force);
            let mut car = Car::new(params, Vec3::ZERO, Vec3::Z);
            car.ai_driver = !human;
            let progress = Progress::new(&line, laps, Vec3::ZERO);
            racers.push(Racer { car_id: id, driver, airborne_s: 0.0, flip_s: 0.0, stuck: None, car, progress, ai, human, base_gear_top, base_acc, telemetry: Telemetry::default(), brake: 0.0, remote: false });
        }
        let props = Props::load(root, &trk)?;
        // `GoalCamera x y z lx ly lz`: eye and look-at point (TrackDef +0x1440) [O]
        let goal_camera = trk.get("GoalCamera").and_then(|e| {
            let v: Option<Vec<f32>> = (0..6).map(|i| e.f32(i)).collect();
            v.map(|v| (Vec3::new(v[0], v[1], v[2]), Vec3::new(v[3], v[4], v[5])))
        });
        // `CameraSplineName`: the TV camera's motion (TrackDef +0x908 → +0x1d90, 0x45ea6d) [O]
        let tv_camera = first_value(&trk, "CameraSplineName")
            .ok()
            .and_then(|n| fs::read(root, &n).ok())
            .and_then(|b| motor_data::tbl::load(&b, None).ok())
            .and_then(|t| cfg::parse_lwmo(&t.data).ok())
            .map(|m| crate::tvcam::keys(&m))
            .unwrap_or_default();
        // `BasePath\StartSpline1/2`: the grid-to-start fly-by motions (TrackDef +0x1108 / +0x1208, 0x45ea6d) [O]
        let start_spline = |k: &str| {
            let path = format!("{}\\{}", first_value(&trk, "BasePath").ok()?, first_value(&trk, k).ok()?);
            let t = motor_data::tbl::load(&fs::read(root, &path).ok()?, None).ok()?;
            cfg::parse_lwmo(&t.data).ok()
        };
        let start_camera = crate::startcam::StartCamera::new(start_spline("StartSpline1"), start_spline("StartSpline2"));
        let mut race = Race { world, line, ai_line, racers, laps, rules: Rules::Normal, countdown: COUNTDOWN, clock_ms: 0.0, autopilot: false, grid, ai_vector, props, goal_camera, tv_camera, start_camera, car_scale: (1.0, 1.0), name_cheat: None, solid_remote: false, susp: (1.0, 1.0), tick_s: None, tick_hz: None, acc: 0.0, prev: Vec::new(), ticks_last_frame: 0 };
        race.restart();
        Ok(race)
    }

    /// Switch the rules (Knockout: one lap fewer than cars) and put everyone back on the grid.
    pub fn set_rules(&mut self, rules: Rules) {
        self.rules = rules;
        if rules == Rules::Elimination {
            self.laps = (self.racers.len() as u32).saturating_sub(1).max(1);
        }
        self.restart();
    }

    /// Everyone back to the grid, clock reset.
    pub fn restart(&mut self) {
        for slot in 0..self.racers.len() {
            let (p, dir) = self.grid.slot(&self.line, slot);
            let ride = self.racers[slot].car.p.ride_height();
            let y = self.world.ground(p.x, p.z, p.y + 20.0).unwrap_or(p.y);
            let r = &mut self.racers[slot];
            r.car.place(Vec3::new(p.x, y - ride, p.z), dir);
            r.progress = Progress::new(&self.line, self.laps, r.car.pos);
            r.ai.reset(&r.car);
        }
        self.countdown = COUNTDOWN;
        self.clock_ms = 0.0;
        self.props.reset();
        (self.acc, self.ticks_last_frame) = (0.0, 0);
        self.prev.clear();
    }

    pub fn started(&self) -> bool {
        self.countdown <= 0.0
    }

    /// Seconds since the race init while the pre-race runs (0 at the start, `COUNTDOWN` at GO).
    pub fn pre_race_s(&self) -> f32 {
        (COUNTDOWN - self.countdown).max(0.0)
    }

    /// Seconds since the race init, the pre-race included: `pre_race_s` before GO, `COUNTDOWN` + the race clock after it
    /// (the Tron scroller's timer `0x758ca8`, reset at `0x48bd18`; #320).
    pub fn since_init_s(&self) -> f32 {
        if self.started() {
            COUNTDOWN + (self.clock_ms.max(0.0) / 1000.0) as f32
        } else {
            self.pre_race_s()
        }
    }

    /// The lit countdown digit (3, 2, 1) and the seconds since it lit; `None` during the fly-by
    /// and after GO (windows 0x45e3c7..0x45e64f).
    pub fn count_digit(&self) -> Option<(u32, f32)> {
        if self.countdown <= 0.0 || self.countdown > COUNT_DIGITS_S {
            return None;
        }
        let n = (self.countdown / COUNT_DIGIT).ceil().clamp(1.0, 3.0);
        Some((n as u32, n * COUNT_DIGIT - self.countdown))
    }

    /// The start fly-by's eye (game space) while it runs: `None` from the "1" on.
    pub fn start_eye(&self) -> Option<Vec3> {
        if self.started() {
            return None;
        }
        self.start_camera.eye(self.pre_race_s())
    }

    /// Jump the fly-by to the "3" (our design, #216: the original has no skip).
    pub fn skip_intro(&mut self) {
        self.countdown = self.countdown.min(COUNT_DIGITS_S);
    }

    pub fn step(&mut self, dt: f32, player: Input) {
        self.step_as(dt, player, None);
    }

    /// `step` with the caller's input driving slot `hand` instead of the human's car (DEBUG, #60:
    /// motorview drives any followed car by hand; design choice). The AI keeps tracking that car
    /// but its input is dropped, and the human's car coasts. `None` = the human car, as `step`.
    pub fn step_as(&mut self, dt: f32, player: Input, hand: Option<usize>) {
        let Some(h) = self.tick_s else {
            self.step_once(dt, player, hand, true);
            self.ticks_last_frame = 1;
            return;
        };
        // design choice (#374): the fixed tick from a time accumulator. A tick is `h` of sim time; under the
        // ramlosa / h2o cheat the sim runs slower than the clock, so a tick then takes more real time. The ticks run up
        // to the frame's end (`acc` ends in (-tick, 0]): the view is drawn between the last two ticks at most one tick
        // behind, and a frame that ends on a tick (60 fps at 60 Hz) draws that tick exactly, as the per-frame path does.
        self.acc += dt;
        let mut n = 0;
        // the tolerance absorbs rounding (two frames of 1/120 s make one tick of 1/60 s)
        while self.acc > h * 1e-4 {
            let real = self.tick_real(h);
            if n == car::MAX_STEPS.get() as u32 {
                // a frame too long to catch up (a hitch, a debugger): drop the backlog, keep the phase
                self.acc = self.acc.rem_euclid(real) - real;
                break;
            }
            self.capture_prev();
            self.step_once(real, player, hand, n == 0);
            self.acc -= real;
            n += 1;
        }
        self.ticks_last_frame = n;
    }

    /// The fixed tick (#374): `Some(hz)` steps the physics at `hz` ticks per second whatever the frame rate and
    /// interpolates the view poses (`view_pose`); `None` (the default) steps once per frame, the original's way.
    pub fn set_fixed_tick(&mut self, hz: Option<f32>) {
        self.tick_hz = hz.filter(|&hz| hz > 0.0);
        self.tick_s = self.tick_hz.map(|hz| 1.0 / hz);
        self.acc = 0.0;
        self.prev.clear();
    }

    /// The fixed tick's rate as it was set, `None` on the per-frame path (the replay header's `tick_hz`).
    pub fn fixed_hz(&self) -> Option<f32> {
        self.tick_hz
    }

    /// The fixed tick's length in sim seconds, `None` on the per-frame path.
    pub fn fixed_tick(&self) -> Option<f32> {
        self.tick_s
    }

    /// How many physics ticks the latest `step_as` ran: 0 on a frame between two ticks (the cars' contact flags
    /// are then still the last tick's: fire one-shot effects only when this is > 0). Always 1 per frame without
    /// the fixed tick.
    pub fn ticks_last_frame(&self) -> u32 {
        self.ticks_last_frame
    }

    /// How far the view is between the previous tick (0) and the latest (1): 1 without the fixed tick and on a frame
    /// that ends on a tick.
    pub fn alpha(&self) -> f32 {
        self.tick_s.map_or(1.0, |h| (1.0 + self.acc / self.tick_real(h)).clamp(0.0, 1.0))
    }

    /// How far the view is behind the latest tick, real seconds (0 on the per-frame path and on a frame that ends on a
    /// tick). Recorded poses (ghosts, a demo's car) are looked up this much before the race clock, so they move with
    /// the interpolated cars.
    pub fn view_lag(&self) -> f32 {
        if self.tick_s.is_some() { (-self.acc).max(0.0) } else { 0.0 }
    }

    /// The pose to draw car `slot` at: between its previous and its latest tick by `alpha`, so the motion is
    /// smooth at any frame rate (one tick behind the physics). Remote cars (posed from outside every frame) and
    /// the per-frame path give the latest pose.
    pub fn view_pose(&self, slot: usize) -> ViewPose {
        let r = &self.racers[slot];
        let now = ViewPose::of(&r.car);
        let alpha = self.alpha();
        match self.prev.get(slot) {
            // alpha 1: the latest tick exactly (no rounding of the lerp: 60 fps draws as the per-frame path)
            Some(prev) if self.tick_s.is_some() && !r.remote && alpha < 1.0 => prev.lerp(&now, alpha),
            _ => now,
        }
    }

    /// The real time one fixed tick of `h` sim seconds takes now (`step_once` scales it back to `h`).
    fn tick_real(&self, h: f32) -> f32 {
        if self.started() { h / crate::cheats::time_scale(self.name_cheat) } else { h }
    }

    fn capture_prev(&mut self) {
        self.prev.clear();
        self.prev.extend(self.racers.iter().map(|r| ViewPose::of(&r.car)));
    }

    /// One step of `dt` real seconds: a rendered frame on the per-frame path, one tick on the fixed one. Every step
    /// clears the cars' per-frame contact flags for its physics; on a frame's later ticks (`first` false) the
    /// earlier ticks' flags are merged back afterwards, so the frame's readers see all of them.
    fn step_once(&mut self, dt: f32, player: Input, hand: Option<usize>, first: bool) {
        let racing = self.started();
        if !racing {
            self.countdown -= dt;
        } else {
            self.clock_ms += dt as f64 * 1000.0;
        }
        let now = self.clock_ms as u32;
        // CHEAT ramlosa / h2o (0x45c2d6, #317): once the clock runs, the sim steps half the frame while the clock keeps
        // real time. `dt` stays the clock's (the countdown, the clock, the wrong-way time in `progress.update`); `sim_dt`
        // drives the world: props, physics substeps, the AI, the car's frame end and the respawn timers [I: the
        // original's timers sit in sim_step's callers; ours count how long a car is in a state of the sim].
        let sim_dt = if racing { dt * crate::cheats::time_scale(self.name_cheat) } else { dt };
        self.rubber_band();
        // inputs first: the frame's base input per car. The AI cars' inputs are replaced by the
        // AI step before every physics substep below.
        let mut inputs = Vec::with_capacity(self.racers.len());
        let mut held = Vec::with_capacity(self.racers.len());
        for (slot, r) in self.racers.iter_mut().enumerate() {
            let done = r.progress.done();
            // the car the caller's input drives (a car with the AI under it too, when `hand` names it)
            let driven = match hand {
                Some(h) => h == slot,
                None => r.human && !self.autopilot,
            };
            // Before GO the car step runs in its grid-hold mode (0x47cdf4 mode 1: no tyre forces,
            // horizontal speed removed). After the flag we brake, only while rolling forward
            // because the automatic gearbox engages reverse when the brake is held at a
            // standstill [design choice]. A manual gearbox can still be shifted in both.
            let shift = hand.map_or(r.human, |h| h == slot) && r.car.manual;
            let hold = Input {
                hold: !racing,
                brake: if racing && r.car.kmh() > 3.0 { 1.0 } else { 0.0 },
                shift_up: shift && player.shift_up,
                shift_down: shift && player.shift_down,
                ..Default::default()
            };
            // CLASSIC 0x45a6af / 0x45a72b: the original reads the throttle and steering bindings
            // before the start too, only the brake waits for the flag (0x45a699). The car step
            // runs in its grid hold (clutch in, no tyre forces), so the engine free-revs on the
            // pedal and the car stays put; the revs it built give the wheelspin launch at GO.
            let input = if racing && !done && driven {
                player
            } else if !racing && driven {
                Input { throttle: player.throttle, steer: player.steer, ..hold }
            } else if r.human && !self.autopilot && hand.is_some() {
                // the human's car while another is driven by hand: coasts
                Input { hold: !racing, ..Default::default() }
            } else {
                hold
            };
            // a remote car's per-frame contacts come only from `demo::drive_remote` (the spark slots, apply
            // 0x40c5f4), which runs after the step: clearing them here would wipe them before the effects read them
            if !r.remote {
                held.push((!first).then(|| r.car.contact_flags()));
                r.car.begin_frame();
            } else {
                held.push(None);
            }
            inputs.push(input);
        }
        // the world step 0x480fbc: car integrate, prop integrate, cars vs props, props vs track
        self.props.animate(sim_dt);
        let mut traffic: Vec<Option<Traffic>> = Vec::with_capacity(self.racers.len());
        let mut others: Vec<Traffic> = Vec::with_capacity(self.racers.len());
        let mut order: Vec<usize> = Vec::with_capacity(self.racers.len());
        for h in car::substeps(sim_dt) {
            // CLASSIC 0x48172e: the AI step 0x407800 runs once per physics substep with the slice
            // length, before the cars' forces. It only runs while racing: on the grid its "stuck,
            // back up" timer would arm and the car would reverse at GO. Eliminated cars (Knockout)
            // are out of the traffic the AI looks at. 0x407800 is one loop over the cars and
            // writes each car's traffic fields (speed at the start, offset at the end of its
            // turn), so a car sees the cars before it as updated in this same step; the loop
            // walks them sorted by race progress, the largest (node, projection on the node's
            // direction) first (prepass 0x40781e..0x40788b, lap-independent) [O]
            if racing {
                traffic.clear();
                traffic.extend(self.racers.iter().map(|r| r.progress.eliminated_ms.is_none().then(|| r.ai.traffic())));
                order.clear();
                order.extend(0..self.racers.len());
                let keys: Vec<_> = self.racers.iter().map(|r| r.ai.order_key()).collect();
                order.sort_by(|&a, &b| keys[b].0.cmp(&keys[a].0).then(keys[b].1.total_cmp(&keys[a].1)));
                for &slot in &order {
                    let r = &mut self.racers[slot];
                    if r.progress.done() {
                        continue;
                    }
                    others.clear();
                    others.extend(traffic.iter().flatten().filter(|t| t.slot != slot).copied());
                    if r.human && !self.autopilot {
                        // CLASSIC 0x407ab2: a human car only gets the prepass tracking (node, speed, alignment,
                        // offset); its lane and lateral velocity stay as initialised [O, #404]
                        r.ai.track(&r.car);
                    } else {
                        let ai_input = r.ai.drive(&r.car, h, &others);
                        if hand != Some(slot) {
                            inputs[slot] = ai_input;
                        }
                    }
                    if r.progress.eliminated_ms.is_none() {
                        traffic[slot] = Some(r.ai.traffic());
                    }
                }
            }
            self.props.begin_substep(h, self.world.gravity);
            for (r, &input) in self.racers.iter_mut().zip(&inputs).filter(|(r, _)| !r.remote) {
                r.telemetry = r.car.substep(h, input, &self.world);
            }
            // the world step per car (0x480fbc): up to three passes of world collide and car vs
            // cars (eliminated cars are out of the traffic), then the resting pass
            self.world_step(h);
            for r in self.racers.iter_mut().filter(|r| r.progress.eliminated_ms.is_none() && !r.remote) {
                self.props.contact(&mut r.car);
            }
            self.props.finish_substep(h, &self.world);
        }
        for (r, held) in self.racers.iter_mut().zip(&held) {
            if let Some(f) = held {
                r.car.merge_contact_flags(f);
            }
        }
        for (slot, (r, input)) in self.racers.iter_mut().zip(&inputs).enumerate() {
            r.brake = input.brake;
            let done = r.progress.done();
            (r.airborne_s, r.flip_s) = if r.remote { (0.0, 0.0) } else { reset_timers(r.airborne_s, r.flip_s, r.car.wheel_contact, r.car.flip_contact, sim_dt) };
            if !r.human && racing && !done && hand != Some(slot) {
                if r.car.vel.z.abs() > 15.0 {
                    r.stuck = None;
                } else if r.ai.reversing() && r.stuck.is_none() {
                    r.stuck = Some(0.0);
                }
                if let Some(t) = &mut r.stuck {
                    *t += sim_dt;
                    if *t > RESET_AFTER.get() {
                        r.airborne_s = RESET_AFTER.get() + 1.0; // reuse the reset path below
                    }
                }
            }
            if racing {
                r.progress.update(&self.line, r.car.pos, now, dt * 1000.0);
            }
        }
        if self.rules == Rules::Elimination {
            self.eliminate();
        }
        for slot in 0..self.racers.len() {
            let r = &self.racers[slot];
            if reset_due(r.airborne_s, r.flip_s) && r.progress.eliminated_ms.is_none() {
                self.respawn(slot);
            }
        }
    }

    /// Knockout rules (our design). Round k = eliminated so far + 1: once every running car but one
    /// has completed k laps, the one still on lap k is out. When a car wins (finishes the last lap)
    /// the cars still running are out too.
    fn eliminate(&mut self) {
        let now = self.clock_ms as u32;
        let done_laps = |r: &Racer| if r.progress.finished_ms.is_some() { self.laps } else { r.progress.lap - 1 };
        let running: Vec<usize> = (0..self.racers.len()).filter(|&i| !self.racers[i].progress.done()).collect();
        let out = self.racers.iter().filter(|r| r.progress.eliminated_ms.is_some()).count() as u32;
        let winner = self.racers.iter().any(|r| r.progress.finished_ms.is_some());
        let knocked: Vec<usize> = if winner {
            running
        } else if running.len() >= 2 && out + 1 < self.laps {
            let k = out + 1;
            let behind: Vec<usize> = running.iter().copied().filter(|&i| done_laps(&self.racers[i]) < k).collect();
            if behind.len() == 1 { behind } else { Vec::new() }
        } else {
            Vec::new()
        };
        for i in knocked {
            self.racers[i].progress.eliminated_ms = Some(now);
        }
    }

    /// `0x480fbc` for the cars, after their steps: each car runs up to three passes of
    /// {world collide `0x47f5e8`; car vs every other car `0x480400`} while the last pass hit
    /// something (0x481149), then the resting pass `0x47eda0` and the record copy (04 §4.9.1).
    fn world_step(&mut self, h: f32) {
        let n = self.racers.len();
        let local: Vec<usize> = (0..n).filter(|&i| !self.racers[i].remote).collect();
        for i in local {
            let mut again = true;
            for _ in 0..3 {
                if !again {
                    break;
                }
                again = self.racers[i].car.world_collide(&self.world, h);
                for j in 0..n {
                    if j == i || (self.racers[j].remote && !self.solid_remote) || self.racers[i].progress.eliminated_ms.is_some() || self.racers[j].progress.eliminated_ms.is_some() {
                        continue;
                    }
                    let (a, b) = two_mut(&mut self.racers, i, j);
                    again |= body::car_pair(&mut a.car, &mut b.car);
                }
            }
            self.racers[i].car.world_post(&self.world, h);
            self.racers[i].car.end_substep();
        }
    }

    /// Slot indices ordered by race position: finishers by time, then by progress.
    pub fn standings(&self) -> Vec<usize> {
        let mut order: Vec<usize> = (0..self.racers.len()).collect();
        order.sort_by(|&a, &b| {
            let (pa, pb) = (&self.racers[a].progress, &self.racers[b].progress);
            match (pa.finished_ms, pb.finished_ms) {
                (Some(x), Some(y)) => x.cmp(&y),
                (Some(_), None) => std::cmp::Ordering::Less,
                (None, Some(_)) => std::cmp::Ordering::Greater,
                // Knockout: running cars by progress, then the knocked-out, the latest first
                (None, None) => match (pa.eliminated_ms, pb.eliminated_ms) {
                    (Some(x), Some(y)) => y.cmp(&x),
                    (Some(_), None) => std::cmp::Ordering::Greater,
                    (None, Some(_)) => std::cmp::Ordering::Less,
                    (None, None) => pb.score().total_cmp(&pa.score()),
                },
            }
        });
        order
    }

    /// CLASSIC 0x429290: the race times the results screen shows, one per place of `standings()` (`None` =
    /// no time: a Knockout car). Cars that have not finished get a made-up time, see `made_up_times`.
    /// The seed is APPROX [I]: the original draws from the C library's rand state, ours is the race clock
    /// so a headless shot is reproducible.
    pub fn results_times(&self) -> Vec<Option<u32>> {
        let rows: Vec<ResultRow> = self
            .standings()
            .into_iter()
            .map(|i| {
                let r = &self.racers[i];
                let p = &r.progress;
                ResultRow { finished_ms: p.finished_ms, out: p.eliminated_ms.is_some(), lap: p.lap, best_lap_ms: p.best_lap_ms, human: r.human }
            })
            .collect();
        made_up_times(&rows, self.clock_ms as u32, (self.clock_ms as u32).wrapping_mul(2_654_435_761).wrapping_add(1))
    }

    /// REMASTER live gap (#299): slot `behind`'s gap to slot `ahead` now (`race::live_gap`).
    pub fn live_gap(&self, ahead: usize, behind: usize) -> Option<crate::race::Gap> {
        crate::race::live_gap(&self.line, &self.racers[ahead].progress, &self.racers[behind].progress, self.clock_ms as u32)
    }

    /// REMASTER (#299): metres from slot `behind` to slot `ahead` along the track.
    pub fn distance_gap(&self, ahead: usize, behind: usize) -> Option<f32> {
        crate::race::distance_gap(&self.line, &self.racers[ahead].progress, &self.racers[behind].progress)
    }

    pub fn all_finished(&self) -> bool {
        self.racers.iter().all(|r| r.progress.done())
    }

    /// The driver/team name cheat of this race (`cheats::NameCheat::of`, 08 §8.14), and its effects
    /// that live in the sim: buzz aldrin / nasa halves gravity for the whole race (`0x481548`, #315),
    /// demon / grem gives every car long soft springs and a weak damper (`0x40ced0`, #313),
    /// g-ride / west a negative damper on 1.2x longer, half-as-stiff springs (`0x40ced0`, #314).
    /// Set it before the first step; a plain `None` puts everything back.
    pub fn set_name_cheat(&mut self, cheat: Option<crate::cheats::NameCheat>) {
        self.name_cheat = cheat;
        // demon / grem (#313): bouncy wheels for every car, as ratios of where they are now (they
        // compose with gulliver); the body moves to the new resting height at the old gravity
        let (len, rate, damper) = crate::cheats::suspension(cheat);
        if (len, rate) != self.susp || self.racers.iter().any(|r| !r.remote && r.car.p.damper != damper) {
            let g = self.world.gravity;
            for r in self.racers.iter_mut().filter(|r| !r.remote) {
                r.car.resuspend(len / self.susp.0, rate / self.susp.1, damper, g);
            }
            self.susp = (len, rate);
        }
        self.world.gravity = crate::cheats::gravity(cheat);
    }

    /// CHEAT gulliver (08 §8.14): every car at size `scale`, the human's turning like a car of
    /// size `feel` and the AI's with the full-size response (`GULLIVER_FEEL`; `CarParams::scaled`;
    /// (1.0, 1.0) = the original), from where it is, mid-race too. The AI's traffic rules take
    /// the new width and length. Remote racers keep their size.
    pub fn set_car_scale(&mut self, scale: f32, feel: f32) {
        if self.car_scale == (scale, feel) {
            return;
        }
        let k = scale / self.car_scale.0;
        let g = self.world.gravity;
        // remote racers are posed from full-size recordings and drawn shrunk (motorview `Session`)
        for r in self.racers.iter_mut().filter(|r| !r.remote) {
            r.car.rescale(scale, if r.human { feel } else { 1.0 }, g);
            r.ai.rescale(k);
        }
        self.car_scale = (scale, feel);
    }

    /// Put a racer back on the racing line a little ahead of where it is.
    pub fn respawn(&mut self, slot: usize) {
        let r = &mut self.racers[slot];
        let flat = Vec3::new(r.car.pos.x, 0.0, r.car.pos.z);
        let nodes = &self.ai_line.nodes;
        let Some(i) = (0..nodes.len()).min_by(|&a, &b| {
            Vec3::new(nodes[a].x, 0.0, nodes[a].z).distance_squared(flat).total_cmp(&Vec3::new(nodes[b].x, 0.0, nodes[b].z).distance_squared(flat))
        }) else {
            return;
        };
        let nd = &nodes[(i + 5) % nodes.len()];
        let ride = r.car.p.ride_height();
        // the height comes from the centreline at the node (projected from the car's progress),
        // not from the car: a car that fell under the track would otherwise find no ground and
        // respawn in the void
        let y_ref = self.line.at(r.progress.s).0.y;
        let (_, s_node) = self.line.project(Vec3::new(nd.x, y_ref, nd.z), r.progress.sample());
        let y_line = self.line.at(s_node).0.y;
        // design choice: of the road-like faces in the column, take the one nearest where the
        // road should be. The centreline runs 1-2.5 m above the road on every track
        // (lineheight example); the first face from above can be the top of a collision box
        // over the road (NeoCity near (-439, -93): a face at +4 m, between walls).
        let y = road_near(&self.world, nd.x, nd.z, y_line + 5.0, y_line - LINE_ABOVE_ROAD.get())
            .or_else(|| self.world.ground(nd.x, nd.z, y_line + 50.0))
            .unwrap_or(y_line);
        r.car.place(Vec3::new(nd.x, y - ride, nd.z), Vec3::from(nd.dir));
        (r.airborne_s, r.flip_s) = (0.0, 0.0); // 0x40da34
        r.stuck = None;
        r.ai.reset(&r.car);
        r.ai.ease_in();
        r.progress.teleported(&self.line, Vec3::new(nd.x, y, nd.z), self.clock_ms as u32);
        // a teleport, not motion: no interpolation from where the car was
        if let Some(p) = self.prev.get_mut(slot) {
            *p = ViewPose::of(&self.racers[slot].car);
        }
    }

}

motor_tune::tunable! {
    /// How far the centreline typically runs above the road surface (median over the tracks) [O].
    /// range: 0..6
    /// unit: m
    /// applies: restart
    LINE_ABOVE_ROAD: f32 = 2.0;
}

/// Of the faces flat enough to drive on in the column at (x, z) below `y_top` (30 m deep), the
/// height of the one nearest `y_want`.
fn road_near(world: &World, x: f32, z: f32, y_top: f32, y_want: f32) -> Option<f32> {
    let mut top = y_top;
    let mut best: Option<f32> = None;
    for _ in 0..32 {
        let Some(h) = world.raycast(Vec3::new(x, top, z), Vec3::new(x, y_top - 30.0, z)) else { break };
        if h.normal.y >= crate::car::WHEEL_MIN_NY.get() && best.is_none_or(|b| (h.point.y - y_want).abs() < (b - y_want).abs()) {
            best = Some(h.point.y);
        }
        top = h.point.y - 0.01;
    }
    best
}

impl Race {
    /// Rubber band (0x40d450 normalises, 0x40d574 applies): AI gear tops and acc force scale by
    /// 1 + clamp(diff, −2, 2) · AI[10] · 0.01 [O]. Cars are rated by progress in ai.dat nodes
    /// ((laps counted − 1 + finished) · N + node, 1-based; a car whose lap counter has just
    /// turned but which has not reached the lap's first checkpoint and is past 70 % of the
    /// nodes is still on the old lap: minus N, 0x40d5f5..0x40d64a): the leader (race position one)
    /// gets −1 and the last car +1 (linear between); when the field is within 50 nodes the
    /// pair is ∓0.02 · spread, so a tight pack is hardly touched. `diff` is the car's rating
    /// minus the human's (0 when there is no human). The original rates from the previous
    /// frame's progress table, indexed by race position; ours uses this frame's [D].
    fn rubber_band(&mut self) {
        let k = self.ai_vector[10] * 0.01;
        if k == 0.0 {
            return;
        }
        let n = self.ai_line.nodes.len() as f32;
        let progress: Vec<f32> = self
            .racers
            .iter()
            .map(|r| {
                let laps = (r.progress.lap - 1 + u32::from(r.progress.finished_ms.is_some())) as f32;
                let p = laps * n + (r.ai.node() + 1) as f32;
                let first_cp = r.progress.splits.get(r.progress.lap as usize * 4 + 2).is_some_and(|s| s.is_some());
                if !first_cp && f64::from(laps * n) + f64::from(n) * 0.7 < f64::from(p) {
                    p - n
                } else {
                    p
                }
            })
            .collect();
        let order = self.standings();
        let (Some(&first), Some(&last)) = (order.first(), order.last()) else { return };
        let (best, worst) = (progress[first], progress[last]);
        let spread = best - worst; // may be <= 0 when the position order disagrees with progress
        // value of a progress: −1 for the leader … +1 for the last car (0 if the field is one car)
        let one_car = self.racers.len() < 2;
        let rank = |p: f32| {
            if one_car || spread <= 0.0 {
                0.0
            } else {
                let amp = spread.min(50.0) / 50.0; // 0x4cf43b / 0x4cf443
                -amp + 2.0 * amp * (best - p) / spread
            }
        };
        let reference = self.racers.iter().position(|r| r.human).map_or(0.0, |h| rank(progress[h]));
        for (r, &p) in self.racers.iter_mut().zip(&progress) {
            if r.human {
                continue;
            }
            let scale = 1.0 + (rank(p) - reference).clamp(-2.0, 2.0) * k;
            for (g, b) in r.car.p.gear_top.iter_mut().zip(&r.base_gear_top) {
                *g = b * scale;
            }
            r.car.p.acc_force = r.base_acc * scale;
        }
    }
}

impl Racer {
    /// Debug tuning (motorview F11): `car.p` was edited from `before`; take its gear tops and
    /// acc force as the new unscaled values, dividing out the rubber band's current scale, so
    /// the next rubber-band pass keeps the edit.
    pub fn rebase(&mut self, before: &CarParams) {
        let scale = if self.base_acc != 0.0 && before.acc_force != 0.0 { before.acc_force / self.base_acc } else { 1.0 };
        self.base_acc = self.car.p.acc_force / scale;
        self.base_gear_top = self.car.p.gear_top.iter().map(|g| g / scale).collect();
    }
}

/// Drivers from League/Profiles/*.adp, sorted by file name.
/// The profiles in League/Profiles, sorted by file name.
pub fn load_drivers(root: &Path) -> Vec<Driver> {
    let Some(dir) = fs::find_ci(root, "League/Profiles") else { return Vec::new() };
    let mut files: Vec<_> = std::fs::read_dir(dir)
        .map(|d| d.flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|x| x.eq_ignore_ascii_case("adp"))).collect())
        .unwrap_or_default();
    files.sort();
    files
        .iter()
        .filter_map(|p| {
            let raw = std::fs::read(p).ok()?;
            let c = cfg::Config::parse(&motor_data::tbl::load(&raw, None).ok()?.data);
            let get = |k: &str| (c.get(k).and_then(|e| e.f32(0)).unwrap_or(50.0) * 0.01).clamp(0.0, 1.0);
            let text = |k: &str| c.get(k).map(|e| e.values.join(" ")).unwrap_or_default();
            Some(Driver {
                name: text("PlayerName"),
                nick: text("PlayerNick"),
                picture: format!("League/Profiles/Gfx/{}", text("PlayerPicture")),
                colour: {
                    let e = c.get("PlayerColour");
                    let ch = |i: usize| e.and_then(|e| e.f32(i)).unwrap_or(200.0).clamp(0.0, 255.0) as u8;
                    [ch(0), ch(1), ch(2)]
                },
                // CLASSIC: the parser (0x44f314) looks for "Aggressiveness"; the files spell it
                // "Aggressivness", so every AI keeps the default 0.5
                temper: Temper { aggression: get("Aggressiveness"), eagerness: get("Eagerness") },
                // the parser's default is 0xffff00 (0x44e8a8)
                car_colours: Some(std::array::from_fn(|k| {
                    let e = c.get(&format!("CarColour{}", k + 1));
                    match e {
                        Some(e) => std::array::from_fn(|i| e.f32(i).unwrap_or(0.0).clamp(0.0, 255.0) as u8),
                        None => [0xff, 0xff, 0],
                    }
                })),
                ninja_marker: false,
            })
        })
        .collect()
}

/// Two distinct elements of a slice, mutably.
fn two_mut<T>(v: &mut [T], i: usize, j: usize) -> (&mut T, &mut T) {
    if i < j {
        let (x, y) = v.split_at_mut(j);
        (&mut x[i], &mut y[0])
    } else {
        let (x, y) = v.split_at_mut(i);
        (&mut y[0], &mut x[j])
    }
}

/// One place of the finishing order, as `made_up_times` reads it.
#[derive(Debug, Clone, Copy)]
pub struct ResultRow {
    pub finished_ms: Option<u32>,
    /// knocked out (Knockout, ours): no time
    pub out: bool,
    /// `S+0`: the current lap, capped at the race's laps once the car finished
    pub lap: u32,
    pub best_lap_ms: Option<u32>,
    pub human: bool,
}

/// `Progress` starts every car's best lap at this (04 §4.6 "Init: best = 1 000 000").
const BEST_LAP_INIT: u32 = 1_000_000;

/// CLASSIC 0x429290 (04 §4.6 "Race end"): `rows` in place order, `now` = the race clock for the case that nobody
/// finished, `seed` for the generator.
/// - `last` = the finish time of the last finished place;
/// - the n-th place (1-based) without a time gets `last + 5000·n + rand % 4900 − 2450`;
/// - if the local player finished, every place then gets `+ (leader.lap − lap) · best lap`;
/// - the order is not touched.
pub fn made_up_times(rows: &[ResultRow], now: u32, seed: u32) -> Vec<Option<u32>> {
    let last = rows.iter().rev().find_map(|r| r.finished_ms).unwrap_or(now);
    let human_done = rows.iter().any(|r| r.human && r.finished_ms.is_some());
    let leader_lap = rows.first().map_or(0, |r| r.lap);
    let mut rng = seed;
    let mut n = 0i64;
    rows.iter()
        .map(|r| {
            let base = match r.finished_ms {
                Some(t) => i64::from(t),
                None if r.out => return None,
                None => {
                    n += 1;
                    i64::from(last) + 5000 * n + i64::from(crate::ai::watcom_rand(&mut rng) % 4900) - 2450
                }
            };
            let behind = if human_done { i64::from(leader_lap.saturating_sub(r.lap)) } else { 0 };
            let best = i64::from(r.best_lap_ms.unwrap_or(BEST_LAP_INIT));
            Some((base + behind * best).clamp(0, i64::from(u32::MAX)) as u32)
        })
        .collect()
}

#[cfg(test)]
mod rewind_tests {
    use super::*;

    /// Restoring a snapshot and stepping again reproduces the original run (needs the install).
    #[test]
    fn snapshot_restore_replays_identically() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../installed_game");
        if !root.exists() {
            return;
        }
        let mut race = Race::load(&root, 1, &[1, 2, 3], None, Some(3)).unwrap();
        let dt = 1.0 / 60.0;
        for _ in 0..1320 {
            race.step(dt, Input::default());
        }
        let snap = race.snapshot();
        let run = |race: &mut Race| {
            for _ in 0..300 {
                race.step(dt, Input::default());
            }
            race.racers.iter().map(|r| (r.car.pos, r.car.vel, r.progress.lap)).collect::<Vec<_>>()
        };
        let first = run(&mut race);
        race.restore(&snap);
        assert_eq!(first, run(&mut race));
    }

    /// #404: the human's traffic record keeps its lane as initialised while it races, the AIs' lanes move (needs the install).
    #[test]
    fn human_lane_stays_as_initialised() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../installed_game");
        if !root.exists() {
            return;
        }
        let mut race = Race::load(&root, 1, &[1, 2, 3], Some(0), Some(3)).unwrap();
        let dt = 1.0 / 60.0;
        let drive = Input { throttle: 1.0, ..Default::default() };
        let start_node = race.racers[0].ai.node();
        let mut ai_lane_moved = false;
        for _ in 0..3000 {
            race.step(dt, drive);
            ai_lane_moved |= race.racers.iter().skip(1).any(|r| r.ai.traffic().lane != 0.0);
        }
        let human = &race.racers[0];
        assert!(human.human);
        assert_eq!(human.ai.traffic().lane, 0.0);
        assert!(human.ai.node() != start_node, "the human was tracked along the line");
        assert!(ai_lane_moved, "an AI car changed lane in the same run");
    }

    /// buzz aldrin / nasa (#315): a car dropped 20 m falls about half as far in the same time
    /// with the cheat (needs the install). Free fall is `g t^2 / 2`, so the cheat's drop is half.
    #[test]
    fn moon_gravity_halves_the_fall() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../installed_game");
        if !root.exists() {
            return;
        }
        let drop = |cheat: Option<crate::cheats::NameCheat>| {
            let mut race = Race::load(&root, 1, &[1], None, Some(3)).unwrap();
            race.set_name_cheat(cheat);
            race.racers[0].car.pos.y += 20.0;
            let y0 = race.racers[0].car.pos.y;
            for _ in 0..30 {
                race.step(1.0 / 60.0, Input::default());
            }
            y0 - race.racers[0].car.pos.y
        };
        let (earth, moon) = (drop(None), drop(crate::cheats::NameCheat::of("buzz aldrin", "nasa")));
        eprintln!("fall in 0.5 s: earth {earth:.2} m, moon {moon:.2} m");
        assert!(earth > 1.0 && (moon / earth - 0.5).abs() < 0.1, "{earth} {moon}");
    }

    /// ramlosa / h2o (#317): after GO the sim runs at half speed and the clock at full speed, so 4 s of
    /// clock under water bring the cars as far as 2 s without it (needs the install).
    #[test]
    fn underwater_halves_the_race_speed() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../installed_game");
        if !root.exists() {
            return;
        }
        let run = |cheat: Option<crate::cheats::NameCheat>, secs: f32| {
            let mut race = Race::load(&root, 1, &[1, 2], None, Some(3)).unwrap();
            race.set_name_cheat(cheat);
            let dt = 1.0 / 60.0;
            while !race.started() {
                race.step(dt, Input::default());
            }
            let (p0, c0) = (race.racers[0].car.pos, race.clock_ms);
            for _ in 0..(secs / dt).round() as usize {
                race.step(dt, Input::default());
            }
            ((race.racers[0].car.pos - p0).length(), race.clock_ms - c0)
        };
        let (dry, dry_ms) = run(None, 2.0);
        let (wet, wet_ms) = run(crate::cheats::NameCheat::of("ramlosa", "h2o"), 4.0);
        eprintln!("normal 2 s: {dry:.2} m ({dry_ms:.0} ms); underwater 4 s: {wet:.2} m ({wet_ms:.0} ms)");
        assert!(dry > 5.0 && (wet / dry - 1.0).abs() < 0.1, "{dry} {wet}");
        assert!((dry_ms - 2000.0).abs() < 1.0 && (wet_ms - 4000.0).abs() < 1.0);
    }

    /// demon / grem (#313): the springs are 3x longer, 0.16x as stiff, the damper 21666; the car rests
    /// higher; `None` gives the loaded values back (needs the install).
    #[test]
    fn bouncy_wheels_soften_and_restore_the_suspension() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../installed_game");
        if !root.exists() {
            return;
        }
        let mut race = Race::load(&root, 1, &[1], None, Some(3)).unwrap();
        let (len, k, y0) = (race.racers[0].car.p.spring_len, race.racers[0].car.p.spring_k, race.racers[0].car.pos.y);
        race.set_name_cheat(crate::cheats::NameCheat::of("demon", "grem"));
        let p = &race.racers[0].car.p;
        assert!((p.spring_len / len - 3.0).abs() < 1e-5 && (p.spring_k / k - 0.16).abs() < 1e-5 && p.damper == 21666.0);
        let rest = p.rest_ground(race.world.gravity);
        eprintln!("rest height: {:.3} m -> {rest:.3} m, car y {y0:.3} -> {:.3}", race.racers[0].car.p.rest_ground(race.world.gravity), race.racers[0].car.pos.y);
        // repeating the same cheat changes nothing
        race.set_name_cheat(crate::cheats::NameCheat::of("demon", "grem"));
        assert!((race.racers[0].car.p.spring_len / len - 3.0).abs() < 1e-5);
        race.set_name_cheat(None);
        let p = &race.racers[0].car.p;
        assert!((p.spring_len - len).abs() < 1e-6 && (p.spring_k - k).abs() < 1e-3 && p.damper == 65000.0);
        assert!((race.racers[0].car.pos.y - y0).abs() < 1e-4);
    }

    /// g-ride / west (#314): length 1.2x, rate 0.5x, damper -50000; the parked car pumps up and down
    /// much more than normal and stays finite; `None` restores the loaded values (needs the install).
    #[test]
    fn bouncy_cars_pump_but_stay_finite() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../installed_game");
        if !root.exists() {
            return;
        }
        let travel = |cheat: Option<crate::cheats::NameCheat>| {
            let mut race = Race::load(&root, 2, &[1], None, Some(3)).unwrap();
            race.set_name_cheat(cheat);
            let (mut lo, mut hi) = (f32::MAX, f32::MIN);
            for i in 0..600 {
                race.step(1.0 / 60.0, Input::default());
                let y = race.racers[0].car.pos.y;
                assert!(y.is_finite() && y.abs() < 1000.0, "diverged at step {i}: {y}");
                if i >= 120 {
                    lo = lo.min(y);
                    hi = hi.max(y);
                }
            }
            hi - lo
        };
        let mut race = Race::load(&root, 2, &[1], None, Some(3)).unwrap();
        let (len, k) = (race.racers[0].car.p.spring_len, race.racers[0].car.p.spring_k);
        race.set_name_cheat(crate::cheats::NameCheat::of("g-ride", "west"));
        let p = &race.racers[0].car.p;
        assert!((p.spring_len / len - 1.2).abs() < 1e-5 && (p.spring_k / k - 0.5).abs() < 1e-5 && p.damper == -50000.0);
        race.set_name_cheat(None);
        let p = &race.racers[0].car.p;
        assert!((p.spring_len - len).abs() < 1e-6 && (p.spring_k - k).abs() < 1e-3 && p.damper == 65000.0);
        let (normal, bouncy) = (travel(None), travel(crate::cheats::NameCheat::of("g-ride", "west")));
        eprintln!("vertical travel after 2 s: normal {normal:.4} m, g-ride {bouncy:.4} m");
        assert!(bouncy > normal * 2.0 && bouncy > 0.02, "{normal} {bouncy}");
    }

    /// `Car::world_vel` is the velocity the car moves by; the body-frame `vel` is not once the
    /// car has turned (#202; needs the install).
    #[test]
    fn world_velocity_moves_the_car() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../installed_game");
        if !root.exists() {
            return;
        }
        let mut race = Race::load(&root, 2, &[1, 2, 3], None, Some(3)).unwrap();
        let dt = 1.0 / 60.0;
        let mut turned = false;
        for _ in 0..1920 {
            let before: Vec<_> = race.racers.iter().map(|r| (r.car.pos, r.car.world_vel())).collect();
            race.step(dt, Input::default());
            for (i, ((p0, v0), r)) in before.into_iter().zip(&race.racers).enumerate() {
                let (moved, v) = ((r.car.pos - p0) / dt, (v0 + r.car.world_vel()) * 0.5);
                // a hull hit or a touch with another car moves the car apart from its velocity
                let alone = race.racers.iter().enumerate().all(|(j, o)| j == i || (o.car.pos - r.car.pos).length() > 6.0);
                if v.length() > 10.0 && r.car.hull_hits.is_empty() && alone {
                    assert!(moved.normalize().dot(v.normalize()) > 0.99, "moved {moved} at world velocity {v}");
                    turned |= moved.normalize().dot(r.car.vel.normalize()) < 0.5;
                }
            }
        }
        assert!(turned, "no car turned away from +z");
    }
}

#[cfg(test)]
mod bump_tests {
    use super::*;
    use crate::props::Kind;

    /// A car pushed into a cone of NeoCity records the hit with the cone's `ObjectColSound` id
    /// (needs the install).
    #[test]
    fn car_hits_a_cone_and_records_its_sound_id() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../installed_game");
        if !root.exists() {
            return;
        }
        let mut race = Race::load(&root, 3, &[1], None, Some(1)).unwrap();
        let mut car = race.racers.remove(0).car;
        let spots: Vec<_> = race.props.props.iter().filter(|p| matches!(p.kind, Kind::Body { .. })).map(|p| p.pose.pos).collect();
        let mut ids = std::collections::BTreeSet::new();
        for pos in spots {
            car.begin_frame();
            car.pos = pos + Vec3::new(0.5, 0.3, 0.0);
            car.vel = Vec3::new(0.0, 0.0, 20.0);
            race.props.contact(&mut car);
            if let Some(b) = car.bump {
                assert!(b.impact >= 0.0);
                ids.insert(b.id);
            }
        }
        assert!(ids.contains(&1), "no cone hit recorded, ids {ids:?}");
    }
}

#[cfg(test)]
mod countdown_tests {
    use super::*;

    /// 6 s of fly-by, then "3", "2", "1" for 2 s each, GO at 12 s (0x45e3c7..0x45e64f); the intro
    /// skip lands on the "3"; the start camera follows the track's splines (needs the install).
    #[test]
    fn twelve_second_pre_race_with_fly_by() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../installed_game");
        if !root.exists() {
            return;
        }
        let mut race = Race::load(&root, 3, &[1, 2, 3], Some(0), Some(3)).unwrap();
        let dt = 1.0 / 60.0;
        // the .MDE clock: 0 at the "3", 6000 at GO (demo::GO_MS)
        assert_eq!(crate::demo::race_ms(&race), -6000.0);
        let mut seen = Vec::new();
        let mut eyes = Vec::new();
        let mut t = 0.0;
        while !race.started() {
            race.step(dt, Input::default());
            t += dt;
            let d = race.count_digit().map(|d| d.0);
            if seen.last().copied() != Some(d) {
                seen.push(d);
                eyes.push((t, race.start_eye()));
            }
        }
        assert_eq!(seen, [None, Some(3), Some(2), Some(1), None]);
        assert!((t - 12.0).abs() < 0.05, "GO at {t}");
        // the digits start at 6, 8 and 10 s
        for (got, want) in eyes.iter().skip(1).map(|e| e.0).zip([6.0, 8.0, 10.0]) {
            assert!((got - want).abs() < 0.05, "digit at {got}, want {want}");
        }
        assert!(eyes[0].1.is_some(), "no start camera at the race start");
        assert!(eyes[3].1.is_none(), "the fly-by is over at the \"1\"");
        assert!((crate::demo::race_ms(&race) - crate::demo::GO_MS).abs() < 20.0);
        let mut skipped = Race::load(&root, 3, &[1, 2, 3], Some(0), Some(3)).unwrap();
        skipped.step(dt, Input::default());
        skipped.skip_intro();
        assert_eq!(skipped.count_digit().map(|d| d.0), Some(3));
        skipped.skip_intro();
        assert!((skipped.countdown - COUNT_DIGITS_S).abs() < 1e-6);
    }

    /// The original reads throttle and steering before the start (0x45a6af): the engine free-revs
    /// in the grid hold, the car stays on its slot, and the revs give a wheelspin launch at GO
    /// (needs the install). Without throttle the engine idles.
    #[test]
    fn throttle_revs_the_engine_on_the_grid() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../installed_game");
        if !root.exists() {
            return;
        }
        let dt = 1.0 / 60.0;
        let human = |race: &Race| race.racers.iter().position(|r| r.human).unwrap();
        let mut revved = Race::load(&root, 2, &[1, 2, 3], Some(0), Some(3)).unwrap();
        let mut idle = Race::load(&root, 2, &[1, 2, 3], Some(0), Some(3)).unwrap();
        let (i, j) = (human(&revved), human(&idle));
        let start = revved.racers[i].car.pos;
        let full = Input { throttle: 1.0, ..Default::default() };
        let mut spun = false;
        while !revved.started() {
            revved.step(dt, full);
            idle.step(dt, Input::default());
        }
        let (min, moved) = (revved.racers[i].car.p.min_rpm, (revved.racers[i].car.pos - start).length());
        assert!(revved.racers[i].car.rpm > min + 3000.0, "rpm {}", revved.racers[i].car.rpm);
        assert!(moved < 0.2, "the car rolled {moved} m on the grid");
        assert!(idle.racers[j].car.rpm <= min + 1.0, "idle rpm {}", idle.racers[j].car.rpm);
        for _ in 0..90 {
            revved.step(dt, full);
            spun |= revved.racers[i].car.wheelspin;
        }
        assert!(spun, "no wheelspin launch");
        assert!((revved.racers[i].car.pos - start).length() > 1.0, "the car did not launch");
    }
}

#[cfg(test)]
mod beginner_tests {
    use super::*;

    /// Beginner is the track's Easy row slowed down; the other levels read their own rows.
    #[test]
    fn beginner_row_is_easy_slowed() {
        let trk = cfg::Config::parse(b"AI1 7 0 10 0.1 10 0.1 0.2 0.1 2.3 14 15\nAI2 7 0 10 0.3 10 0.2 0.5 0.2 2.7 16 10\n");
        let easy = ai_vector(&trk, AiRow::Difficulty(Difficulty::Easy));
        let medium = ai_vector(&trk, AiRow::Difficulty(Difficulty::Medium));
        let b = ai_vector(&trk, AiRow::Difficulty(Difficulty::Beginner));
        assert_eq!(easy[8], 2.3);
        assert_eq!(medium[8], 2.7);
        assert!(b[8] < easy[8] && b[9] < easy[9]);
        assert_eq!((b[8], b[9]), (2.3 * BEGINNER_CORNER_GRIP.get(), 14.0 * BEGINNER_CORNER_BASE.get()));
        // grip, brake and the rubber band stay as on Easy
        assert_eq!((b[4], b[5], b[6], b[7], b[10]), (easy[4], easy[5], easy[6], easy[7], easy[10]));
        assert_eq!((b[0], b[1], b[2], b[3]), (3.0, 0.7, 3.0, 0.7));
    }
}

#[cfg(test)]
mod made_up_time_tests {
    use super::*;

    fn row(finished: Option<u32>, lap: u32, best: u32, human: bool) -> ResultRow {
        ResultRow { finished_ms: finished, out: false, lap, best_lap_ms: Some(best), human }
    }

    /// The n-th unfinished place lands in last + 5000·n ± 2450, whatever the seed.
    #[test]
    fn made_up_times_stay_in_range() {
        let mut rows = vec![row(Some(200_000), 3, 60_000, true), row(Some(205_000), 3, 61_000, false)];
        rows.extend((0..6).map(|_| row(None, 3, 62_000, false)));
        for seed in 0..500u32 {
            let t = made_up_times(&rows, 0, seed.wrapping_mul(7919));
            assert_eq!(&t[..2], &[Some(200_000), Some(205_000)], "finished cars keep their time");
            for n in 1..=6u32 {
                let v = t[1 + n as usize].unwrap();
                let centre = 205_000 + 5000 * n;
                assert!((centre - 2450..=centre + 2449).contains(&v), "n {n} seed {seed}: {v}");
            }
        }
    }

    /// A lap-down car pays its best lap per lap, but only if the local player finished.
    #[test]
    fn lap_down_penalty_needs_a_finished_human() {
        let rows = |human_done: bool| {
            vec![row(Some(200_000), 3, 60_000, false), row(Some(210_000), 3, 61_000, human_done), row(None, 2, 70_000, false)]
        };
        let with = made_up_times(&rows(true), 0, 5)[2].unwrap();
        let without = made_up_times(&rows(false), 0, 5)[2].unwrap();
        // same seed, so the same noise; the human (place 2) finished in the first case only
        assert_eq!(with, without + 70_000);
        assert!((210_000 + 5000 - 2450 + 70_000..=210_000 + 5000 + 2449 + 70_000).contains(&with));
    }

    /// Nobody finished: the base is the clock; Knockout cars get no time; the length stays.
    #[test]
    fn no_finisher_and_knocked_out() {
        let mut rows = vec![row(None, 1, 0, false), row(None, 1, 0, false)];
        rows[1].out = true;
        let t = made_up_times(&rows, 100_000, 1);
        assert_eq!(t.len(), 2);
        assert!((102_550..=107_449).contains(&t[0].unwrap()));
        assert_eq!(t[1], None);
    }
}

#[cfg(test)]
mod tick_tests {
    use super::*;

    fn race(player: Option<usize>) -> Option<Race> {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../installed_game");
        root.exists().then(|| Race::load(&root, 1, &[1, 4, 7], player, Some(3)).unwrap())
    }

    type CarState = (Vec3, Vec3, glam::Mat3, [f32; 4], u32, Option<usize>);

    fn state(race: &Race) -> Vec<CarState> {
        race.racers.iter().map(|r| (r.car.pos, r.car.vel, r.car.rot, r.car.wheel_ext, r.progress.lap, r.car.chassis_mat)).collect()
    }

    /// #374: the 60 Hz tick stepped by 1/60 s frames is the per-frame path at 60 fps, bit for bit (needs the install).
    #[test]
    fn tick_60_at_60_fps_is_the_per_frame_path() {
        let (Some(mut frame), Some(mut tick)) = (race(Some(0)), race(Some(0))) else { return };
        tick.set_fixed_tick(Some(60.0));
        let drive = Input { throttle: 1.0, steer: 0.1, ..Default::default() };
        for _ in 0..1500 {
            frame.step(1.0 / 60.0, drive);
            tick.step(1.0 / 60.0, drive);
            assert_eq!(tick.ticks_last_frame(), 1);
            assert_eq!(state(&frame), state(&tick));
            let flags = |r: &Race| r.racers.iter().map(|r| r.car.contact_flags()).collect::<Vec<_>>();
            assert_eq!(flags(&frame), flags(&tick));
        }
        assert_eq!(frame.clock_ms, tick.clock_ms);
    }

    /// #374: on the fixed tick the race is the same after the same number of ticks at any frame rate (needs the install).
    #[test]
    fn tick_is_independent_of_the_frame_rate() {
        let drive = Input { throttle: 1.0, steer: -0.05, ..Default::default() };
        let run = |dt: f32| {
            let mut race = race(Some(0))?;
            race.set_fixed_tick(Some(60.0));
            let mut ticks = 0;
            while ticks < 1500 {
                race.step(dt, drive);
                ticks += race.ticks_last_frame();
            }
            assert_eq!(ticks, 1500, "frames of {dt} s overshot");
            Some((state(&race), race.clock_ms))
        };
        let Some(at60) = run(1.0 / 60.0) else { return };
        for dt in [1.0 / 30.0, 1.0 / 120.0, 1.0 / 144.0] {
            assert_eq!(run(dt).unwrap(), at60, "frames of {dt} s");
        }
    }

    /// #374: between two ticks the view pose is between the cars' tick poses; a snapshot taken between ticks
    /// replays identically (needs the install).
    #[test]
    fn view_pose_interpolates_and_snapshots_keep_the_phase() {
        let Some(mut race) = race(None) else { return };
        race.set_fixed_tick(Some(60.0));
        for _ in 0..800 {
            race.step(1.0 / 60.0, Input::default());
        }
        // half a tick: the tick runs to cover it, the view is drawn halfway from the previous one
        race.step(0.5 / 60.0, Input::default());
        assert_eq!(race.ticks_last_frame(), 1);
        assert!((race.view_lag() - 0.5 / 60.0).abs() < 1e-5);
        assert!((race.alpha() - 0.5).abs() < 1e-3);
        let (prev, now) = (race.prev[1], ViewPose::of(&race.racers[1].car));
        assert!(prev.pos.distance(now.pos) > 0.1, "the car moves");
        let mid = race.view_pose(1);
        assert!((mid.pos - (prev.pos + now.pos) * 0.5).length() < 1e-3);
        let snap = race.snapshot();
        let run = |race: &mut Race| {
            for _ in 0..300 {
                race.step(1.0 / 144.0, Input::default());
            }
            (state(race), race.view_pose(2))
        };
        let first = run(&mut race);
        race.restore(&snap);
        assert_eq!(first, run(&mut race));
    }
}

#[cfg(test)]
mod reset_tests {
    use super::{reset_due, reset_timers};

    const ALL: [bool; 4] = [true; 4];
    const NONE: [bool; 4] = [false; 4];

    /// 0x40d854: upside down against the world, reset after 2 s; the wheel timer runs alongside.
    #[test]
    fn flip_timer_resets_after_two_seconds() {
        let (mut w, mut f) = (0.0, 0.0);
        for _ in 0..8 {
            (w, f) = reset_timers(w, f, NONE, true, 0.25);
        }
        assert!(!reset_due(w, f), "2.0 s is not yet above the limit: {f}");
        (w, f) = reset_timers(w, f, NONE, true, 0.25);
        assert!(reset_due(w, f));
        // the flip timer holds when the flag drops; only 3 wheels down clear it
        let (w2, f2) = reset_timers(0.0, 1.5, NONE, false, 0.1);
        assert_eq!((w2, f2), (0.1, 1.5));
        assert_eq!(reset_timers(5.0, 1.5, [true, true, true, false], false, 0.1), (0.0, 0.0));
    }

    /// Two wheels: a side pair (FL+BL, FR+BR) holds the wheel timer, every other pair counts.
    #[test]
    fn wheel_pairs() {
        let pair = |a: usize, b: usize| {
            let mut w = NONE;
            (w[a], w[b]) = (true, true);
            reset_timers(1.0, 0.0, w, false, 0.25).0
        };
        assert_eq!(pair(0, 3), 1.0); // FL + BL
        assert_eq!(pair(1, 2), 1.0); // FR + BR
        assert_eq!(pair(0, 1), 1.25); // front
        assert_eq!(pair(2, 3), 1.25); // rear
        assert_eq!(pair(0, 2), 1.25); // diagonal
        assert_eq!(pair(1, 3), 1.25);
        assert_eq!(reset_timers(1.0, 0.0, [true, false, false, false], false, 0.25).0, 1.25);
        assert_eq!(reset_timers(1.0, 0.0, ALL, false, 0.25).0, 0.0);
        assert!(reset_due(8.01, 0.0) && !reset_due(8.0, 0.0));
    }

    /// The frame time counts at most 0.5 s.
    #[test]
    fn frame_time_capped() {
        assert_eq!(reset_timers(0.0, 0.0, NONE, true, 2.0), (0.5, 0.5));
    }
}
