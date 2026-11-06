//! Race session for the viewer: wraps `motor_sim::Race`, the player's slot and render transforms.

use crate::render::{Camera, TrailStamp, TRAIL_RELAX};
use crate::scene::{Vertex, CAR_MESHES, WHEEL_X_SCALE};
use glam::{Mat3, Mat4, Quat, Vec2, Vec3};
use motor_data::cfg::Motion;
use motor_sim::car::skid_step2;
use motor_sim::props;
use motor_sim::race::fmt_ms;
use motor_sim::replay::{Header, Recording};
use motor_sim::session::{Rules, ViewPose};
use motor_sim::{AiRow, Car, Difficulty, Driver, Input, Race};
use std::f32::consts::{PI, TAU};
use std::path::Path;

motor_tune::tunable! {
    /// Env-probe height above a car's body origin [design choice].
    /// range: 0..3
    /// unit: m
    /// area: Camera & drive/Effects
    ENV_PROBE_HEIGHT: f32 = 0.6;
}

/// Game space (left-handed) -> view space (right-handed): negate z.
const FLIP: Vec3 = Vec3::new(1.0, 1.0, -1.0);

// The eye offsets and look-at points below are car space, game axes (x right, y up, z forward),
// one `tunable!` per axis. Each view has an accessor that puts the three together.
motor_tune::tunable! {
    // Chase camera (0x42c540, called from the race loop 0x45a210 at 0x45c4c2). Constants in
    // .data: look-at offset 0x4b7b60 (world space, added to the car position) and eye offset
    // 0x4b7b6c (car space).
    /// Chase camera look-at point, sideways (0x4b7b60) [O]: world space, added to the car's position.
    /// range: -5..5
    /// unit: m
    /// area: Camera & drive/Chase camera
    CHASE_LOOK_X: f32 = 0.0;
    /// Chase camera look-at point, height above the car (0x4b7b60) [O].
    /// range: -2..6
    /// unit: m
    /// area: Camera & drive/Chase camera
    CHASE_LOOK_Y: f32 = 1.5;
    /// Chase camera look-at point, ahead of the car (0x4b7b60) [O].
    /// range: -5..5
    /// unit: m
    /// area: Camera & drive/Chase camera
    CHASE_LOOK_Z: f32 = 0.0;
    /// Chase camera eye, sideways (0x4b7b6c) [O]: car space.
    /// range: -5..5
    /// unit: m
    /// area: Camera & drive/Chase camera
    CHASE_EYE_X: f32 = 0.0;
    /// Chase camera eye, height above the car (0x4b7b6c) [O].
    /// range: -1..8
    /// unit: m
    /// area: Camera & drive/Chase camera
    CHASE_EYE_Y: f32 = 2.0;
    /// Chase camera eye, ahead of the car (0x4b7b6c) [O]: negative is behind. The nose-pitch drop
    /// (`CHASE_PITCH`) is scaled by the eye's distance relative to the source's -4.5.
    /// range: -20..5
    /// unit: m
    /// area: Camera & drive/Chase camera
    CHASE_EYE_Z: f32 = -4.5;
    /// Chase camera looking back (rear-view key), eye sideways (0x45a210) [O].
    /// range: -5..5
    /// unit: m
    /// area: Camera & drive/Chase camera
    CHASE_BACK_EYE_X: f32 = 0.0;
    /// Chase camera looking back, eye height above the car (0x45a210) [O].
    /// range: -1..8
    /// unit: m
    /// area: Camera & drive/Chase camera
    CHASE_BACK_EYE_Y: f32 = 2.0;
    /// Chase camera looking back, eye ahead of the car (0x45a210) [O]: the view is turned round, so it sits in front.
    /// range: -5..20
    /// unit: m
    /// area: Camera & drive/Chase camera
    CHASE_BACK_EYE_Z: f32 = 5.0;
}

/// Shadow colour: polygons of colour (10, 10, 10) through the world path (0x438308), times
/// WorldBrightness [I: ~1]. The alpha is a guess; the reference shots show a dark blob that
/// lets the road through.
const SHADOW_RGB: [f32; 3] = [10.0 / 255.0, 10.0 / 255.0, 10.0 / 255.0];
motor_tune::tunable! {
    /// Opacity of the blob shadow under a car [I: a guess; the reference shots show a dark blob that lets the road through].
    /// range: 0..1
    /// area: Camera & drive/Effects
    SHADOW_ALPHA: f32 = 0.6;
    /// Ground plane rays for the blob shadow start this far above the shadow points (0x438308).
    /// range: 0..5
    /// unit: m
    /// area: Camera & drive/Effects
    SHADOW_RAY_UP: f32 = 1.0;
    /// Ground plane rays for the blob shadow end this far below the shadow points (0x438308).
    /// range: 0..30
    /// unit: m
    /// area: Camera & drive/Effects
    SHADOW_RAY_DOWN: f32 = 10.0;
    /// Skip the blob shadow on ground steeper than this normal.y (0x4d2f9d).
    /// range: 0..1
    /// area: Camera & drive/Effects
    SHADOW_MIN_NY: f32 = 0.8;
}

motor_tune::tunable! {
    // In-car camera: the car's own matrix moved 1.0 along up (0x45a210, 0x40af2c(1.0)); looking
    // back: 1.2 up, 1.0 back, turned 180 degrees. Chase looking back: 2 up, 5 forward, turned.
    /// In-car camera eye, sideways (0x45a210, 0x40af2c) [O]: the car's own matrix moved along its axes.
    /// range: -2..2
    /// unit: m
    /// area: Camera & drive/Views
    INCAR_EYE_X: f32 = 0.0;
    /// In-car camera eye, height above the car's model origin (0x45a210, 0x40af2c(1.0)) [O].
    /// range: -1..4
    /// unit: m
    /// area: Camera & drive/Views
    INCAR_EYE_Y: f32 = 1.0;
    /// In-car camera eye, ahead of the car's model origin (0x45a210) [O].
    /// range: -5..5
    /// unit: m
    /// area: Camera & drive/Views
    INCAR_EYE_Z: f32 = 0.0;
    /// In-car camera looking back, eye sideways (0x45a210) [O].
    /// range: -2..2
    /// unit: m
    /// area: Camera & drive/Views
    INCAR_BACK_EYE_X: f32 = 0.0;
    /// In-car camera looking back, eye height (0x45a210) [O].
    /// range: -1..4
    /// unit: m
    /// area: Camera & drive/Views
    INCAR_BACK_EYE_Y: f32 = 1.2;
    /// In-car camera looking back, eye ahead of the car (0x45a210) [O]: negative is behind; the view is turned 180 degrees.
    /// range: -5..5
    /// unit: m
    /// area: Camera & drive/Views
    INCAR_BACK_EYE_Z: f32 = -1.0;
    // Extra views not in this motor.exe build [design choice]: a far chase and a bumper camera.
    /// Far chase camera eye, sideways [design choice].
    /// range: -5..5
    /// unit: m
    /// area: Camera & drive/Views
    FAR_EYE_X: f32 = 0.0;
    /// Far chase camera eye, height above the car [design choice].
    /// range: -1..12
    /// unit: m
    /// area: Camera & drive/Views
    FAR_EYE_Y: f32 = 3.2;
    /// Far chase camera eye, ahead of the car [design choice]: negative is behind.
    /// range: -30..5
    /// unit: m
    /// area: Camera & drive/Views
    FAR_EYE_Z: f32 = -8.0;
    /// Far chase camera look-at point, sideways [design choice]: world space, added to the car's position.
    /// range: -5..5
    /// unit: m
    /// area: Camera & drive/Views
    FAR_LOOK_X: f32 = 0.0;
    /// Far chase camera look-at point, height above the car [design choice].
    /// range: -2..6
    /// unit: m
    /// area: Camera & drive/Views
    FAR_LOOK_Y: f32 = 1.2;
    /// Far chase camera look-at point, ahead of the car [design choice].
    /// range: -5..5
    /// unit: m
    /// area: Camera & drive/Views
    FAR_LOOK_Z: f32 = 0.0;
    /// Bumper camera eye, sideways [design choice].
    /// range: -2..2
    /// unit: m
    /// area: Camera & drive/Views
    BUMPER_EYE_X: f32 = 0.0;
    /// Bumper camera eye, height above the car's model origin [design choice].
    /// range: -1..3
    /// unit: m
    /// area: Camera & drive/Views
    BUMPER_EYE_Y: f32 = 0.55;
    /// Bumper camera eye, ahead of the car's model origin [design choice].
    /// range: -2..6
    /// unit: m
    /// area: Camera & drive/Views
    BUMPER_EYE_Z: f32 = 2.1;
}

/// Three axis tunables as a vector.
fn axes(x: &motor_tune::Tun<f32>, y: &motor_tune::Tun<f32>, z: &motor_tune::Tun<f32>) -> Vec3 {
    Vec3::new(x.get(), y.get(), z.get())
}

/// The same at the source's values (`Default for CamTune`, the page's reset).
fn axes_default(x: &motor_tune::Tun<f32>, y: &motor_tune::Tun<f32>, z: &motor_tune::Tun<f32>) -> Vec3 {
    Vec3::new(x.default_value(), y.default_value(), z.default_value())
}

/// Put a vector into three axis tunables.
fn set_axes(x: &motor_tune::Tun<f32>, y: &motor_tune::Tun<f32>, z: &motor_tune::Tun<f32>, v: Vec3) {
    x.set(v.x);
    y.set(v.y);
    z.set(v.z);
}

/// The chase camera looking back: the eye's car-space offset (the view is turned round).
fn chase_back_eye() -> Vec3 {
    axes(&CHASE_BACK_EYE_X, &CHASE_BACK_EYE_Y, &CHASE_BACK_EYE_Z)
}

/// The in-car camera looking back (and the bumper camera, which shares it).
fn incar_back_eye() -> Vec3 {
    axes(&INCAR_BACK_EYE_X, &INCAR_BACK_EYE_Y, &INCAR_BACK_EYE_Z)
}

/// Camera views. The game has in-car, chase and TV cameras plus a held look-back key
/// (mode flags 0x4b7b10, 0x704dfc, 0x704e00, 0x704e04 set in 0x45a210).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CamMode {
    Chase,
    FarChase,
    InCar,
    Bumper,
    /// the track's `GoalCamera`, fixed eye and look-at point, from the player's finish until
    /// the results (gameplay videos) [O]
    Goal,
    /// the TV camera: trackside shots along `CameraSplineName`, F3 (0x45741c) [O]; a track
    /// without stations, or looking back, shows the chase view
    Tv,
}

impl CamMode {
    pub fn next(self) -> CamMode {
        match self {
            CamMode::Chase => CamMode::FarChase,
            CamMode::FarChase => CamMode::InCar,
            CamMode::InCar => CamMode::Bumper,
            CamMode::Bumper | CamMode::Goal | CamMode::Tv => CamMode::Chase,
        }
    }
    pub fn parse(s: &str) -> Option<CamMode> {
        Some(match s {
            "chase" => CamMode::Chase,
            "far" => CamMode::FarChase,
            "incar" => CamMode::InCar,
            "bumper" => CamMode::Bumper,
            "goal" => CamMode::Goal,
            "tv" => CamMode::Tv,
            _ => return None,
        })
    }
    /// The followed car is not drawn from inside it (0x45a210 skips it when 0x4b7b10 is set).
    pub fn hides_car(self) -> bool {
        matches!(self, CamMode::InCar | CamMode::Bumper)
    }
}

motor_tune::tunable! {
    /// A previous eye farther than this from the car is a cut (a respawn) and the chase eye starts over
    /// [design choice]. The original has no cut: its TV eye, up to 105 m out, is eased in [O, #274].
    /// (The renderer's own `CUT_DISTANCE` is the motion-blur and temporal-history cut.)
    /// range: 10..500
    /// unit: m
    /// area: Camera & drive/Chase camera
    CUT_DISTANCE: f32 = 150.0;
    /// Chase eye easing per frame, horizontally: k = sqrt(60*dt) * this (0x42c540) [O]. Remaster's Chase follow
    /// (Loose / Tight) scales it.
    /// range: 0..1
    /// area: Camera & drive/Chase camera
    CHASE_K_XZ: f32 = 0.15;
    /// Chase eye easing per frame, vertically: k = sqrt(60*dt) * this (0x42c540) [O].
    /// range: 0..1
    /// area: Camera & drive/Chase camera
    CHASE_K_Y: f32 = 0.9;
    /// How far the eased eye drops with the car's nose pitch, per unit of forward.y (0x42c540: 3.0) [O].
    /// range: 0..10
    /// unit: m
    /// area: Camera & drive/Chase camera
    CHASE_PITCH: f32 = 3.0;
    /// Largest angle the eased eye may trail behind the car's heading: pi is off, like the
    /// original (any value from pi up is off). Chase follow Loose / Tight set their own [REMASTER design choice].
    /// range: 0..3.15
    /// unit: rad
    /// area: Camera & drive/Chase camera
    CHASE_MAX_LAG: f32 = PI;
    /// Free look [design choice, not in the original]: the right stick's direction picks the
    /// chase eye's yaw round the car (up = behind, down = in front looking back, right = the
    /// car's left side), no pitch. The eye eases the short way round at this rate, and back
    /// behind the car when the stick is released.
    /// range: 1..40
    /// unit: 1/s
    /// area: Camera & drive/Chase camera
    ORBIT_RATE: f32 = 14.0;
    /// Free look [design choice]: within this angle of straight up the eye stays behind the car.
    /// range: 0..1.5707964
    /// unit: rad
    /// area: Camera & drive/Chase camera
    ORBIT_HOLD: f32 = 20.0 * PI / 180.0;
    /// Chase eye pull-in [design choice]: the gap kept in front of a wall.
    /// range: 0..3
    /// unit: m
    /// area: Camera & drive/Chase camera
    CAM_MARGIN: f32 = 0.35;
    /// Chase eye pull-in [design choice]: the closest the eye may come to the look-at point.
    /// range: 0..5
    /// unit: m
    /// area: Camera & drive/Chase camera
    CAM_MIN: f32 = 1.0;
    /// Chase eye pull-in [design choice]: the rate the eye eases back out at once the wall is gone.
    /// range: 0..50
    /// unit: 1/s
    /// area: Camera & drive/Chase camera
    CAM_OUT_RATE: f32 = 4.0;
}

/// The camera offsets and easing above in one struct: the values of one frame. `live()` reads the
/// tunables (the Constants page, the F11 Camera page and `tune.cfg` all edit those); `Default` is
/// the source's values, the Camera page's reset.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CamTune {
    pub chase_eye: Vec3,
    pub chase_look: Vec3,
    pub far_eye: Vec3,
    pub far_look: Vec3,
    pub incar_eye: Vec3,
    pub bumper_eye: Vec3,
    pub k_xz: f32,
    pub k_y: f32,
    pub max_lag: f32,
    pub margin: f32,
    pub min: f32,
    pub out_rate: f32,
}

impl CamTune {
    /// The camera values now.
    pub fn live() -> CamTune {
        CamTune {
            chase_eye: axes(&CHASE_EYE_X, &CHASE_EYE_Y, &CHASE_EYE_Z),
            chase_look: axes(&CHASE_LOOK_X, &CHASE_LOOK_Y, &CHASE_LOOK_Z),
            far_eye: axes(&FAR_EYE_X, &FAR_EYE_Y, &FAR_EYE_Z),
            far_look: axes(&FAR_LOOK_X, &FAR_LOOK_Y, &FAR_LOOK_Z),
            incar_eye: axes(&INCAR_EYE_X, &INCAR_EYE_Y, &INCAR_EYE_Z),
            bumper_eye: axes(&BUMPER_EYE_X, &BUMPER_EYE_Y, &BUMPER_EYE_Z),
            k_xz: CHASE_K_XZ.get(),
            k_y: CHASE_K_Y.get(),
            max_lag: CHASE_MAX_LAG.get(),
            margin: CAM_MARGIN.get(),
            min: CAM_MIN.get(),
            out_rate: CAM_OUT_RATE.get(),
        }
    }

    /// Put these values into the tunables (the F11 Camera page edits a copy and stores it).
    pub fn store(&self) {
        set_axes(&CHASE_EYE_X, &CHASE_EYE_Y, &CHASE_EYE_Z, self.chase_eye);
        set_axes(&CHASE_LOOK_X, &CHASE_LOOK_Y, &CHASE_LOOK_Z, self.chase_look);
        set_axes(&FAR_EYE_X, &FAR_EYE_Y, &FAR_EYE_Z, self.far_eye);
        set_axes(&FAR_LOOK_X, &FAR_LOOK_Y, &FAR_LOOK_Z, self.far_look);
        set_axes(&INCAR_EYE_X, &INCAR_EYE_Y, &INCAR_EYE_Z, self.incar_eye);
        set_axes(&BUMPER_EYE_X, &BUMPER_EYE_Y, &BUMPER_EYE_Z, self.bumper_eye);
        CHASE_K_XZ.set(self.k_xz);
        CHASE_K_Y.set(self.k_y);
        CHASE_MAX_LAG.set(self.max_lag);
        CAM_MARGIN.set(self.margin);
        CAM_MIN.set(self.min);
        CAM_OUT_RATE.set(self.out_rate);
    }

    /// These values with a chase follow setting on top.
    fn followed(mut self, f: Follow) -> CamTune {
        if f == Follow::default() {
            return self;
        }
        self.k_xz *= f.ease;
        self.k_y = (self.k_y * f.ease).min(1.0);
        self.max_lag = f.lag.unwrap_or(self.max_lag);
        self
    }
}

impl Default for CamTune {
    fn default() -> Self {
        CamTune {
            chase_eye: axes_default(&CHASE_EYE_X, &CHASE_EYE_Y, &CHASE_EYE_Z),
            chase_look: axes_default(&CHASE_LOOK_X, &CHASE_LOOK_Y, &CHASE_LOOK_Z),
            far_eye: axes_default(&FAR_EYE_X, &FAR_EYE_Y, &FAR_EYE_Z),
            far_look: axes_default(&FAR_LOOK_X, &FAR_LOOK_Y, &FAR_LOOK_Z),
            incar_eye: axes_default(&INCAR_EYE_X, &INCAR_EYE_Y, &INCAR_EYE_Z),
            bumper_eye: axes_default(&BUMPER_EYE_X, &BUMPER_EYE_Y, &BUMPER_EYE_Z),
            k_xz: CHASE_K_XZ.default_value(),
            k_y: CHASE_K_Y.default_value(),
            max_lag: CHASE_MAX_LAG.default_value(),
            margin: CAM_MARGIN.default_value(),
            min: CAM_MIN.default_value(),
            out_rate: CAM_OUT_RATE.default_value(),
        }
    }
}

/// The chase follow setting of a session (REMASTER Chase follow Loose / Normal / Tight; CLASSIC and Normal
/// are the default): scales the tuned easing and, when `lag` is set, limits how far the eye trails the heading
/// [design choice]. The game sets it every frame; motorview leaves the default.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Follow {
    /// factor on `CHASE_K_XZ` and `CHASE_K_Y` (the vertical one stays under 1)
    pub ease: f32,
    /// the largest lag in radians, None = `CHASE_MAX_LAG`
    pub lag: Option<f32>,
}

impl Default for Follow {
    fn default() -> Follow {
        Follow { ease: 1.0, lag: None }
    }
}

pub struct Session {
    pub race: Race,
    /// slot whose car the camera follows
    pub focus: usize,
    /// eased chase camera eye, game space
    eye: Option<Vec3>,
    pub cam: CamMode,
    /// the start camera flag (0x4b7b1c, set at race init): the fly-by shows while it is set and
    /// the race's `start_eye` has an eye; a camera key from the "3" on clears it (#216)
    pub fly_by: bool,
    /// chase follow setting on top of the tuned camera values (the game sets it each frame)
    pub follow: Follow,
    /// look-back key held
    pub look_back: bool,
    /// wall pull-in of the chase eye on (`place_eye`; the Camera tab of the graphic options)
    pub pull_in: bool,
    /// free-look stick, -1..1 (x right, y up); chase cameras only
    pub free_look: Vec2,
    /// eased free-look (yaw, pitch), radians
    orbit: f32,
    /// chase eye distance from the look-at point after the wall pull-in, metres
    cam_dist: Option<f32>,
    /// final chase eye offset from the car (orbit and pull-in applied), game space
    cam_off: Option<Vec3>,
    /// per racer: `shadowpoints1..4` from the .car, 4 rows x 4 points, car space
    shadow_points: Vec<Option<[Vec3; 16]>>,
    /// per racer: tyre half-widths and wheel radius, from the scene
    wheels: Vec<([f32; 4], f32)>,
    /// per racer: wheel roll angle (radians)
    /// wheel roll angles per car: front pair, rear pair (P[0xdc] / P[0xe0], 0x40ca90)
    spin: Vec<[f32; 2]>,
    /// skid marks per racer and wheel: the committed segments of the strips, oldest first, at most
    /// `SKID_RING` (the original's ring of 50 per wheel)
    skids: Vec<[std::collections::VecDeque<SkidSeg>; 4]>,
    /// per racer and wheel: the strip being laid (game space)
    skid_head: Vec<[Option<SkidHead>; 4]>,
    /// REMASTER detailed marks: the tread texture's layer (`Scene::tyre_layer`)
    tyre_layer: Option<f32>,
    /// tyre smoke and sparks
    pub fx: crate::fx::Effects,
    /// `Data/Finish/finish.mot`, the podium emblem's path in camera space
    finish_motion: Option<Motion>,
    /// DEBUG timeline (#60): the frames stepped so far and keyframes, for seeking (motorview;
    /// the game does not record one)
    pub timeline: Option<crate::timeline::Timeline>,
    /// DEBUG (#60 phase 2): the setup this run's repro file (`.mhr`) is made from; motorview sets it,
    /// the game leaves None
    pub setup: Option<Header>,
    /// REMASTER grass trail (#158): the footprints of the last `TRAIL_RELAX` seconds of sim
    /// time, oldest first (`Frame::trail`), and per racer where and when it was last stamped
    trail: std::collections::VecDeque<TrailStamp>,
    trail_last: Vec<(Vec3, f32)>,
    /// CHEAT tribute to tron, REMASTER Neon grid (#320): per racer the points of its light wall of the last
    /// `LIGHT_WALL_LIFE` seconds of sim time, oldest first, and where the car is now (the wall's live end)
    light_walls: Vec<std::collections::VecDeque<WallPt>>,
    light_head: Vec<Option<WallPt>>,
    /// simulated seconds stepped so far (the trail's clock; `rewind` takes them back)
    pub sim_secs: f32,
}

motor_tune::tunable! {
    /// REMASTER grass trail: a car stamps a new footprint once it has moved this far since its last [design choice].
    /// range: 0.02..2
    /// unit: m
    /// area: Camera & drive/Effects
    TRAIL_STEP_M: f32 = 0.25;
    /// REMASTER grass trail: a car stamps a new footprint once this long has passed since its last [design choice].
    /// range: 0.01..1
    /// unit: s
    /// area: Camera & drive/Effects
    TRAIL_STEP_S: f32 = 0.1;
    /// REMASTER grass trail: the gap since the last footprint is filled along the car's velocity, at most this long [design choice].
    /// range: 0..1
    /// unit: s
    /// area: Camera & drive/Effects
    TRAIL_TAIL_MAX: f32 = 0.25;
    /// The finish emblem's motion is read with this many frames per second (0x44b62f → 0x456be4) [O].
    /// range: 10..240
    /// unit: frames/s
    /// area: Camera & drive/Effects
    FINISH_FPS: f32 = 60.0;
    /// Front wheels turn by the smoothed steering times this (0x40cbb2, 0x4cf3ab) [O].
    /// range: 0..1.5
    /// unit: rad
    /// area: Camera & drive/Effects
    WHEEL_STEER: f32 = 0.7;
}

// Skid marks, CLASSIC (the mark code 0x446644, 08-missing-features 8.6): which wheels mark is
// `Car::skid_marking`, the commit distance is `skid_step2`; the constants below are the rest of the original's spacing.
motor_tune::tunable! {
    /// Skid marks: a horizontal step² above this (12 m, 0x4d3b7d) restarts the strip [O].
    /// range: 1..1000
    /// unit: m²
    /// area: Camera & drive/Skid marks
    SKID_RESTART2: f32 = 144.0;
    /// Skid marks: a horizontal step² below this (0.1 m, 0x4d3b85) lays nothing [O].
    /// range: 0..1
    /// unit: m²
    /// area: Camera & drive/Skid marks
    SKID_MIN_STEP2: f32 = 0.01;
    /// Skid marks: the strip's half-width as a share of the tyre's (0x4d357d) [O].
    /// range: 0.01..1
    /// unit: m
    /// area: Camera & drive/Skid marks
    SKID_HALF_WIDTH: f32 = 0.1;
    /// Skid marks: the strip's lift above the ground along its normal (0x4d3585) [O].
    /// range: 0..0.2
    /// unit: m
    /// area: Camera & drive/Skid marks
    SKID_LIFT: f32 = 0.01;
    /// Skid marks: segments kept per wheel (the original's ring of 50, `cmp 0x32` at 0x446734) [O].
    /// range: 1..500
    /// area: Camera & drive/Skid marks
    SKID_RING: usize = 50;
    /// REMASTER detailed skid marks [design choice]: one tile of the tread texture covers this many metres
    /// along the mark (the tread's own tile on the wheels, procgen.rs `TYRE_TILE`).
    /// range: 0.05..4
    /// unit: m
    /// area: Camera & drive/Skid marks
    SKID_TILE: f32 = 0.4;
    /// REMASTER detailed skid marks [design choice]: the sideways speed, in multiples of the surface's limit
    /// above it, that makes a mark full strength.
    /// range: 0.05..10
    /// area: Camera & drive/Skid marks
    SKID_FULL: f32 = 1.0;
    /// REMASTER detailed skid marks [design choice]: the strength of a mark at the surface's limit.
    /// range: 0..1
    /// area: Camera & drive/Skid marks
    SKID_MIN: f32 = 0.3;
    /// Skid marks: a bend sharper than this (cosine of half the turn) is not mitred [design choice].
    /// range: 0..1
    /// area: Camera & drive/Skid marks
    SKID_MITRE_MIN: f32 = 0.7;
}

/// Mark colour by the surface's SkidMark kind (0..255 scale as the original stores it). CLASSIC:
/// grass (80, 60, 20) and gravel (40, 35, 20) are literal constants of the mark code (0x44688a,
/// 0x4468aa); asphalt and concrete store `ebx` = `[0x4ebe98]` = 0 (0x44671c, 0x44667a), so black
/// [O]. The blend multiplies the frame by it. The kinds the original does not mark (0, >4) return
/// `None`.
fn skid_colour(kind: u32) -> Option<[f32; 4]> {
    let rgb = match kind {
        1 | 2 => [0.0, 0.0, 0.0],
        3 => [80.0 / 255.0, 60.0 / 255.0, 20.0 / 255.0],
        4 => [40.0 / 255.0, 35.0 / 255.0, 20.0 / 255.0],
        _ => return None,
    };
    Some([rgb[0], rgb[1], rgb[2], 1.0])
}

/// One laid piece of a skid strip, ground points `q` to `p` (game space). `side` is the half-width
/// vector across the strip (`dir x normal`, tyre half-width long); `side_q` / `side_p` are the
/// same mitred with the neighbouring segments so a bend leaves no seam. `v` is the metres along
/// the strip, `s` the strength (0..1) at each end.
#[derive(Clone, Copy)]
struct SkidSeg {
    q: Vec3,
    p: Vec3,
    side: Vec3,
    side_q: Vec3,
    side_p: Vec3,
    kind: u32,
    v0: f32,
    v1: f32,
    s0: f32,
    s1: f32,
}

/// The strip a wheel is laying: the end of its last committed segment (where the next one starts),
/// the metres and strength there, the side vector of the last committed segment (for the mitre),
/// whether the ring's newest segment is this strip's, and the live segment: it follows the wheel
/// every frame until it is long enough to be committed to the ring (0x446ad8).
#[derive(Clone, Copy)]
struct SkidHead {
    p: Vec3,
    v: f32,
    s: f32,
    prev_side: Option<Vec3>,
    chained: bool,
    open: Option<SkidSeg>,
}

/// Half-width vector at the joint of two segments: the mean of their sides stretched so both edges
/// meet. `b` itself where the strip turns sharply.
fn skid_mitre(a: Vec3, b: Vec3) -> Vec3 {
    let m = a + b;
    let hw = b.length();
    if m.length() < 2.0 * hw * SKID_MITRE_MIN.get() {
        return b;
    }
    m * (2.0 * hw * hw / m.length_squared())
}

motor_tune::tunable! {
    /// CHEAT tribute to tron, REMASTER Neon grid (#320 phase 3, design choice): a light wall lasts this long.
    /// range: 0.2..20
    /// unit: s
    /// area: Camera & drive/Effects
    pub LIGHT_WALL_LIFE: f32 = 2.5;
    /// Light wall: a new wall point every this many metres ...
    /// range: 0.05..5
    /// unit: m
    /// area: Camera & drive/Effects
    LIGHT_WALL_STEP_M: f32 = 0.5;
    /// ... or this many seconds, whichever comes first.
    /// range: 0.005..1
    /// unit: s
    /// area: Camera & drive/Effects
    LIGHT_WALL_STEP_S: f32 = 0.05;
    /// Light wall: the wall's height along the car's up axis.
    /// range: 0.05..3
    /// unit: m
    /// area: Camera & drive/Effects
    pub LIGHT_WALL_HEIGHT: f32 = 0.6;
    /// Light wall: a car this far from its wall's newest point (a respawn) starts a new wall.
    /// range: 1..100
    /// unit: m
    /// area: Camera & drive/Effects
    LIGHT_WALL_BREAK: f32 = 12.0;
}
/// the followed car's wall, cyan like the grid, and the rivals', orange (gamma space)
pub const LIGHT_WALL_YOU: [f32; 3] = [0.35, 0.9, 1.0];
pub const LIGHT_WALL_RIVAL: [f32; 3] = [1.0, 0.45, 0.08];

/// One point of a light wall: the ground point (game space), the car's up axis and the sim time it was laid.
#[derive(Clone, Copy, Debug, PartialEq)]
struct WallPt {
    pos: Vec3,
    up: Vec3,
    t: f32,
}

/// Add `pt` to a wall when it is `LIGHT_WALL_STEP_M` / `_S` past the newest point (a jump of `LIGHT_WALL_BREAK`
/// starts a new wall) and drop the points older than `LIGHT_WALL_LIFE`.
fn push_wall_point(ring: &mut std::collections::VecDeque<WallPt>, pt: WallPt) {
    let (brk, step_m, step_s, life) = (LIGHT_WALL_BREAK.get(), LIGHT_WALL_STEP_M.get(), LIGHT_WALL_STEP_S.get(), LIGHT_WALL_LIFE.get());
    if ring.back().is_some_and(|b| b.pos.distance(pt.pos) > brk) {
        ring.clear();
    }
    if ring.back().is_none_or(|b| b.pos.distance(pt.pos) >= step_m || pt.t - b.t >= step_s) {
        ring.push_back(pt);
    }
    while ring.front().is_some_and(|f| pt.t - f.t > life) {
        ring.pop_front();
    }
}

/// One quad (six vertices, view space) per pair of neighbouring wall points, ground to `LIGHT_WALL_HEIGHT`:
/// uv = (age 0..1 of `LIGHT_WALL_LIFE`, 0 bottom / 1 top), colour `rgb`.
fn wall_quads<'a>(out: &mut Vec<Vertex>, pts: impl Iterator<Item = &'a WallPt>, now: f32, rgb: [f32; 3]) {
    let (height, life) = (LIGHT_WALL_HEIGHT.get(), LIGHT_WALL_LIFE.get());
    let at = |p: &WallPt, top: bool| {
        let x = if top { p.pos + p.up * height } else { p.pos };
        let age = ((now - p.t) / life).clamp(0.0, 1.0);
        Vertex { pos: (x * FLIP).into(), uv: [age, if top { 1.0 } else { 0.0 }], color: [rgb[0], rgb[1], rgb[2], 1.0], layer: -1.0, normal: [0.0; 3], env: 0.0 }
    };
    let mut prev: Option<&WallPt> = None;
    for p in pts {
        if let Some(q) = prev {
            let (a, b, c, d) = (at(q, false), at(q, true), at(p, true), at(p, false));
            out.extend([a, b, c, a, c, d]);
        }
        prev = Some(p);
    }
}

pub fn shadow_points(root: &Path, car: u32) -> Option<[Vec3; 16]> {
    let c = motor_data::fs::config(root, &format!("Game/car{car:02}.car")).ok()?;
    let mut pts = [Vec3::ZERO; 16];
    for row in 0..4 {
        let v: Vec<f32> = c.get(&format!("shadowpoints{}", row + 1))?.values.iter().filter_map(|x| x.parse().ok()).collect();
        for col in 0..4 {
            pts[row * 4 + col] = Vec3::new(*v.get(col * 3)?, *v.get(col * 3 + 1)?, *v.get(col * 3 + 2)?);
        }
    }
    Some(pts)
}

/// The point every camera hangs off: the car's model origin, the physics position raised by the ride
/// height along the car's up. CLASSIC [O] (#274): the traced in-car eye is 1.0 m up from the car
/// matrix's position, which `origreplay` showed to be this point, not `Car::pos`.
pub fn cam_origin(car: &Car) -> Vec3 {
    car.pos + car.rot * Vec3::new(0.0, car.p.ride_height(), 0.0)
}

/// `cam_origin` of a car (parameters `p`) drawn at `v` (#374: the interpolated view pose, `Race::view_pose`).
pub fn cam_origin_at(p: &motor_sim::car::CarParams, v: &ViewPose) -> Vec3 {
    v.pos + v.rot * Vec3::new(0.0, p.ride_height(), 0.0)
}

/// CHEAT supercars / grem (0x42c8e0): the camera's rows, game space: right +x, up +z (north up), forward -y.
const TOP_DOWN_ROT: Mat3 = Mat3::from_cols(Vec3::X, Vec3::Z, Vec3::NEG_Y);

/// CHEAT supercars / grem (0x42c8e0): the eye 25 m above the body origin and 12 m along the heading's x and z.
fn top_down_eye(origin: Vec3, fwd: Vec3) -> Vec3 {
    origin + Vec3::new(fwd.x * motor_sim::cheats::TOPDOWN_AHEAD, motor_sim::cheats::TOPDOWN_UP, fwd.z * motor_sim::cheats::TOPDOWN_AHEAD)
}

/// Grid line-up: 7 opponents, then the player's car at the back (slot 7). [design choice]
pub fn lineup(player_car: u32) -> Vec<u32> {
    let mut cars: Vec<u32> = (1..=10).filter(|&c| c != player_car).take(7).collect();
    cars.push(player_car);
    cars
}

/// Ease the chase eye (world position) towards its spot behind the car at `pos` / `rot`;
/// `local` is the eye's car-space offset. Shared by the followed car and `PoseCam`.
/// CLASSIC, the game's 0x42c540 [O]: the previous eye is put back to the eye distance
/// `|local|` from the car's new position along its old direction (no speed lag, cornering
/// lags), then eased towards the target with k = sqrt(60*dt) * (0.15 x/z, 0.9 y), each clamped
/// to 0.05..0.9 (0x42b780). The target's height is the local height minus 3 * forward.y
/// (car[+0x1c]) instead of the rotated one. Scaling that for other eye distances (far chase,
/// look back) is ours [I]. There is no lag clamp in the original: `max_lag` below pi is a
/// REMASTER design choice (Chase follow Loose / Tight).
fn ease_eye(prev: Option<Vec3>, pos: Vec3, rot: Mat3, local: Vec3, t: &CamTune, dt: f32) -> Vec3 {
    let fwd_y = (rot * Vec3::Z).y;
    let mut off = rot * local;
    // the drop is CHASE_PITCH at the source's eye distance and scales with the eye's distance from there on
    off.y = local.y + local.z * fwd_y * (CHASE_PITCH.get() / (-CHASE_EYE_Z.default_value()).max(0.1));
    let cut = CUT_DISTANCE.get();
    match prev {
        // a respawn or a cut moves the car far from the old eye: start over [design choice]
        Some(e) if e.distance(pos) < cut => {
            let from = pos + (e - pos).normalize_or_zero() * local.length();
            let r = (60.0 * dt).sqrt();
            let (kxz, ky) = ((t.k_xz * r).clamp(0.05, 0.9), (t.k_y * r).clamp(0.05, 0.9));
            let d = pos + off - from;
            let eased = from + Vec3::new(d.x * kxz, d.y * ky, d.z * kxz) - pos;
            if t.max_lag >= PI {
                return pos + eased;
            }
            // REMASTER: keep the eye at the target's distance and within max_lag of straight
            // behind, so hard corners and slides don't swing the camera wide
            let (tf, ef) = (Vec3::new(off.x, 0.0, off.z), Vec3::new(eased.x, 0.0, eased.z));
            let lag = tf.cross(ef).y.atan2(tf.dot(ef)).clamp(-t.max_lag, t.max_lag);
            let flat = glam::Quat::from_rotation_y(lag) * tf;
            pos + Vec3::new(flat.x, eased.y, flat.z)
        }
        _ => pos + off,
    }
}

/// CHEAT ignition / uds, the game's 0x42c944 (both chase functions jump to it, #318) [O]: `ease_eye` with other
/// constants. The target is `IGNITION_BACK` behind along forward x / z (not flattened) and `IGNITION_UP` up, lowered by
/// `IGNITION_PITCH` x forward.y; the previous eye is put back to the distance of (-12 fwd.x, 17, -12 fwd.z) and eased per
/// world axis by `IGNITION_K` (x lags, z snaps). The original applies k once per drawn frame; ours per 1/60 s,
/// 1 - (1 - k)^(60 dt), the same at any frame rate (design choice).
fn ignition_eye(prev: Option<Vec3>, pos: Vec3, rot: Mat3, dt: f32) -> Vec3 {
    use motor_sim::cheats::{IGNITION_BACK, IGNITION_K, IGNITION_PITCH, IGNITION_UP};
    let fwd = rot * Vec3::Z;
    let off = Vec3::new(-IGNITION_BACK * fwd.x, IGNITION_UP, -IGNITION_BACK * fwd.z);
    let target = pos + Vec3::new(off.x, IGNITION_UP - IGNITION_PITCH * fwd.y, off.z);
    match prev {
        Some(e) if e.distance(pos) < CUT_DISTANCE.get() => {
            let from = pos + (e - pos).normalize_or_zero() * off.length();
            let k = IGNITION_K.map(|k| 1.0 - (1.0 - k).powf(60.0 * dt));
            let d = target - from;
            from + Vec3::new(d.x * k[0], d.y * k[1], d.z * k[2])
        }
        _ => target,
    }
}

/// APPROX: pull the eye in along the ray from the look-at point when the track's collision
/// mesh is in the way (visual-only geometry is not tested). The COL ray is one-sided, so it hits
/// the faces of walls, ground and ceilings that face the track. Returns the eye offset from
/// `pos`; `dist` is the eased distance kept between frames.
#[allow(clippy::too_many_arguments)]
fn pull_eye(world: &motor_sim::World, pos: Vec3, off: Vec3, look: Vec3, t: &CamTune, enabled: bool, dist: &mut Option<f32>, dt: f32) -> Vec3 {
    // CLASSIC: no pull-in, so no distance easing either: the eye is where the easing put it [O, #274]
    if !enabled {
        *dist = None;
        return off;
    }
    let (pivot, v) = (pos + look, off - look);
    let (len, dir) = (v.length(), v.normalize_or_zero());
    let free = world.raycast(pivot, pivot + dir * (len + t.margin)).map_or(len, |h| (h.point.distance(pivot) - t.margin).clamp(t.min.min(len), len));
    let d = match *dist {
        Some(d) if d < free => d + (free - d) * (1.0 - (-t.out_rate * dt).exp()),
        _ => free,
    };
    *dist = Some(d);
    look + dir * d
}

/// A camera's (eye, look-at point), view space, for a car at `pos` / `rot` (`cam_off`: its
/// eased, pulled-in chase offset).
fn view_of(race: &Race, mode: CamMode, look_back: bool, pos: Vec3, rot: Mat3, cam_off: Option<Vec3>, t: &CamTune) -> (Vec3, Vec3) {
    let fwd = rot * Vec3::Z;
    let fixed = |local: Vec3, dir: Vec3| {
        let eye = pos + rot * local;
        (eye * FLIP, (eye + dir) * FLIP)
    };
    match (mode, look_back) {
        // CLASSIC: fixed, look back ignored [I]; tracks without the key keep the chase view
        (CamMode::Goal, _) if race.goal_camera.is_some() => {
            let (eye, at) = race.goal_camera.unwrap_or_default();
            (eye * FLIP, at * FLIP)
        }
        (CamMode::InCar, false) => fixed(t.incar_eye, fwd),
        (CamMode::InCar | CamMode::Bumper, true) => fixed(incar_back_eye(), -fwd),
        (CamMode::Bumper, false) => fixed(t.bumper_eye, fwd),
        (_, true) => {
            let eye = pos + cam_off.unwrap_or(rot * chase_back_eye());
            (eye * FLIP, (eye - fwd) * FLIP)
        }
        (mode, false) => {
            let eye = pos + cam_off.unwrap_or(rot * t.chase_eye);
            let look = if mode == CamMode::FarChase { t.far_look } else { t.chase_look };
            (eye * FLIP, (pos + look) * FLIP)
        }
    }
}

/// The camera to render with for a car at `pos` / `rot`: the TV camera's shot (position, bank,
/// zoom and far plane), else `view_of`. `clear`: the TV eye is pulled in front of geometry that hides
/// the car (the wall pull-in, REMASTER); CLASSIC [O, #274] draws the station's shot as it is.
#[allow(clippy::too_many_arguments)]
fn camera_of(race: &Race, mode: CamMode, look_back: bool, pos: Vec3, rot: Mat3, cam_off: Option<Vec3>, t: &CamTune, clear: bool) -> Camera {
    if mode == CamMode::Tv && !look_back {
        if let Some(mut shot) = motor_sim::tvcam::shot(&race.tv_camera, pos, rot) {
            if clear {
                shot.clear_view(&race.world, pos);
            }
            return Camera::from_rot(shot.eye, shot.rot, shot.zoom, shot.far_k);
        }
    }
    // CLASSIC [O] (#274): the in-car views are the car's own matrix, they bank and pitch with it
    if matches!(mode, CamMode::InCar | CamMode::Bumper) {
        let (local, turn) = match (mode, look_back) {
            (CamMode::Bumper, false) => (t.bumper_eye, Vec3::ONE),
            (_, false) => (t.incar_eye, Vec3::ONE),
            _ => (incar_back_eye(), Vec3::new(-1.0, 1.0, -1.0)),
        };
        return Camera::from_rot(pos + rot * local, rot * Mat3::from_diagonal(turn), 1.0, 1.0);
    }
    let (eye, at) = view_of(race, mode, look_back, pos, rot, cam_off, t);
    let look = (at - eye).normalize_or(Vec3::NEG_Z);
    Camera::new(eye, look.x.atan2(-look.z), look.y.asin())
}

/// A camera following a pose instead of a racer (the ghost view window): the same easing, wall
/// pull-in and views as the followed car, without free look. Modes cycle chase, far chase,
/// in-car, bumper and TV.
#[derive(Debug, Clone, Copy)]
pub struct PoseCam {
    pub mode: CamMode,
    eye: Option<Vec3>,
    dist: Option<f32>,
    off: Option<Vec3>,
}

impl PoseCam {
    pub const MODES: [CamMode; 5] = [CamMode::Chase, CamMode::FarChase, CamMode::InCar, CamMode::Bumper, CamMode::Tv];

    pub fn new(mode: CamMode) -> PoseCam {
        PoseCam { mode, eye: None, dist: None, off: None }
    }

    pub fn next(&mut self) {
        let i = Self::MODES.iter().position(|&m| m == self.mode).map_or(0, |i| (i + 1) % Self::MODES.len());
        *self = PoseCam::new(Self::MODES[i]);
    }

    /// Ease towards the pose (`dt` seconds since the last call); the camera for it.
    pub fn camera(&mut self, race: &Race, t: &CamTune, pos: Vec3, rot: Mat3, dt: f32) -> Camera {
        let local = if self.mode == CamMode::FarChase { t.far_eye } else { t.chase_eye };
        let eye = ease_eye(self.eye, pos, rot, local, t, dt);
        self.eye = Some(eye);
        let off = eye - pos;
        let look = if self.mode == CamMode::FarChase { t.far_look } else { t.chase_look };
        self.off = Some(pull_eye(&race.world, pos, off, look, t, true, &mut self.dist, dt));
        camera_of(race, self.mode, false, pos, rot, self.off, t, true)
    }
}

impl Session {
    pub fn new(root: &Path, track: u32, cars: &[u32], player: Option<usize>, difficulty: Difficulty) -> Result<Session, String> {
        Self::with_options(root, track, cars, player, difficulty, None, true)
    }

    /// `laps`: None = the track's default; `catchup` = the AI rubber band (AI vector [10]).
    pub fn with_options(root: &Path, track: u32, cars: &[u32], player: Option<usize>, difficulty: Difficulty, laps: Option<u32>, catchup: bool) -> Result<Session, String> {
        Self::with_field(root, track, cars, player, AiRow::Difficulty(difficulty), laps, catchup, None)
    }

    /// A race with a chosen AI row and, optionally, the driver of every slot (league races).
    #[allow(clippy::too_many_arguments)]
    pub fn with_field(root: &Path, track: u32, cars: &[u32], player: Option<usize>, row: AiRow, laps: Option<u32>, catchup: bool, drivers: Option<&[Driver]>) -> Result<Session, String> {
        let mut race = Race::load_field(root, track, cars, player, laps, row, drivers)?;
        if !catchup {
            race.ai_vector[10] = 0.0;
        }
        let focus = player.unwrap_or(cars.len() - 1);
        let shadow_points = cars.iter().map(|&c| shadow_points(root, c)).collect();
        let n = cars.len();
        Ok(Session {
            race,
            focus,
            eye: None,
            cam: CamMode::Chase,
            fly_by: true,
            follow: Follow::default(),
            look_back: false,
            free_look: Vec2::ZERO,
            pull_in: true,
            orbit: 0.0,
            cam_dist: None,
            cam_off: None,
            shadow_points,
            wheels: vec![([0.15; 4], 0.35); n],
            spin: vec![[0.0; 2]; n],
            skids: (0..n).map(|_| Default::default()).collect(),
            skid_head: vec![[None; 4]; n],
            tyre_layer: None,
            timeline: None,
            setup: None,
            trail: std::collections::VecDeque::new(),
            trail_last: vec![(Vec3::ZERO, f32::NEG_INFINITY); n],
            light_walls: vec![Default::default(); n],
            light_head: vec![None; n],
            sim_secs: 0.0,
            fx: crate::fx::Effects::new(n),
            finish_motion: motor_data::fs::read(root, "Data/Finish/finish.mot")
                .ok()
                .and_then(|b| motor_data::tbl::load(&b, None).ok())
                .and_then(|t| motor_data::cfg::parse_lwmo(&t.data).ok()),
        })
    }

    /// The race a recording was made in, ready for its first frame (`Recording::load_race`, with
    /// the session around it); `setup` is left to the caller.
    pub fn for_recording(root: &Path, rec: &Recording) -> Result<Session, String> {
        let h = &rec.header;
        // MHR2 names the driver of each slot; MHR1 has the fixed line-up of the first 8 profiles
        let drivers = rec.drivers(root);
        let mut sess = Self::with_field(root, h.track, &h.cars, h.player, h.row, Some(h.laps), h.catchup, drivers.as_deref())?;
        if h.rules != Rules::Normal {
            sess.race.set_rules(h.rules);
        }
        if let Some(r) = h.player.and_then(|p| sess.race.racers.get_mut(p)) {
            r.car.manual = h.manual;
        }
        // #374: the physics the run was made on (MHR3 names the fixed tick; older files step once per frame)
        sess.race.set_fixed_tick(h.tick_hz);
        Ok(sess)
    }

    /// The slot motorview's hand driving feeds the keys to (`step_as`): the followed car, `None`
    /// when that is the human's own (a normal run).
    pub fn hand_slot(&self) -> Option<usize> {
        let human = self.race.racers.iter().position(|r| r.human);
        (human != Some(self.focus)).then_some(self.focus)
    }

    /// CyclePlayers (F4): follow the next car.
    pub fn cycle_focus(&mut self) {
        self.focus = (self.focus + 1) % self.race.racers.len().max(1);
        self.eye = None;
        (self.orbit, self.cam_dist, self.cam_off) = (0.0, None, None);
    }

    /// DEBUG: start recording the timeline from the race as it is now (#60).
    pub fn record_timeline(&mut self) {
        self.timeline = Some(crate::timeline::Timeline::new(&self.race));
    }

    /// One sim step. With a timeline in the past the recorded frame is replayed (so `dt` and an
    /// idle `input` are ignored); a driving input takes over from that moment.
    pub fn step(&mut self, dt: f32, input: Input) {
        self.step_as(dt, input, None);
    }

    /// `step` with `input` driving slot `hand` (#60 hand driving, `Race::step_as`); `None` = the human's car.
    pub fn step_as(&mut self, dt: f32, input: Input, hand: Option<usize>) {
        let dt = match self.timeline.as_mut() {
            Some(t) => t.step(&mut self.race, dt, input, hand),
            None => {
                self.race.step_as(dt, input, hand);
                dt
            }
        };
        self.sim_secs += dt;
        self.record_trail();
        self.follow(dt);
        for (i, r) in self.race.racers.iter().enumerate() {
            let radius = self.wheels.get(i).map_or(0.35, |w| w.1).max(0.1) * r.car.p.scale;
            let k = self.spin_scale(radius);
            for (a, rate) in self.spin[i].iter_mut().zip(r.car.wheel_spin) {
                *a = (*a + rate * k * dt) % std::f32::consts::TAU;
            }
        }
        self.lay_skids();
        self.raise_light_walls();
        let cars: Vec<_> = self
            .race
            .racers
            .iter()
            .zip(&self.wheels)
            .enumerate()
            .map(|(i, (r, &(_, radius)))| {
                let radius = radius * r.car.p.scale;
                let v = self.race.view_pose(i);
                let ground = |w| v.pos + v.rot * Self::wheel_at(&r.car.p, v.wheel_ext[w], w, radius).1;
                (&r.car, v, [ground(2), ground(3)])
            })
            .collect();
        let skip = self.hides_car().then_some(self.focus);
        self.fx.bubbles = self.underwater();
        self.fx.step(dt, &self.race.world, &cars, skip);
    }

    /// REMASTER grass trail (#158): a footprint per racer with a wheel on a grass or gravel
    /// surface (Material.mat ContactAnimID 2 or 3, 08-missing-features §8.6; so a car on a
    /// bridge deck does not flatten the grass below it) once it has moved `TRAIL_STEP_M` or
    /// `TRAIL_STEP_S` since its last one; footprints older than `TRAIL_RELAX` drop off.
    /// View space like the car models (forward = the model's -z axis).
    fn record_trail(&mut self) {
        let now = self.sim_secs;
        let Session { race, trail, trail_last, .. } = self;
        let (step_m, step_s, tail_max) = (TRAIL_STEP_M.get(), TRAIL_STEP_S.get(), TRAIL_TAIL_MAX.get().max(0.0));
        for (i, (r, last)) in race.racers.iter().zip(trail_last.iter_mut()).enumerate() {
            let on = r.car.wheel_mat.iter().flatten().any(|&m| race.world.materials.get(m).is_some_and(|m| matches!(m.contact_anim, 2 | 3)));
            // #374: where the car is drawn
            let v = race.view_pose(i);
            let pos = v.pos * FLIP;
            if !on || pos.distance(last.0) < step_m && now - last.1 < step_s {
                continue;
            }
            let tail = if last.1.is_finite() { (now - last.1).clamp(0.0, tail_max) } else { 0.0 };
            *last = (pos, now);
            let c = &r.car.p.corners;
            let half = [c.iter().map(|v| v.z.abs()).fold(0.0, f32::max), c.iter().map(|v| v.x.abs()).fold(0.0, f32::max)];
            let fwd = v.rot * Vec3::Z * FLIP;
            let vel = r.car.world_vel() * FLIP;
            trail.push_back(TrailStamp { pos: [pos.x, pos.z], fwd: [fwd.x, fwd.z], half, vel: [vel.x, vel.z], time: now, tail, _pad: [0.0; 2] });
        }
        while trail.front().is_some_and(|t| t.time < now - TRAIL_RELAX.get()) {
            trail.pop_front();
        }
    }

    /// Camera only, the race standing still (the game's F8 freeze): the chase eye keeps easing
    /// and free look keeps orbiting [design choice].
    pub fn step_camera(&mut self, dt: f32) {
        self.follow(dt);
    }

    /// DEBUG rewind: go back `dt` seconds of simulated time (to the nearest recorded step);
    /// false when already at the start of the timeline. Skid marks and smoke are not rewound.
    pub fn rewind(&mut self, dt: f32) -> bool {
        let Some((start, now, _)) = self.timeline.as_ref().map(|t| t.span()) else { return false };
        if now <= start + 1e-4 {
            return false;
        }
        self.seek(now - dt as f64).is_some()
    }

    /// DEBUG: put the race at timeline second `to` (clamped to the recorded run). Returns the sim
    /// seconds moved (negative = back), or None without a timeline. Skid marks and smoke are not
    /// rewound; a forward jump leaves no grass footprints for the part skipped.
    pub fn seek(&mut self, to: f64) -> Option<f64> {
        let moved = self.timeline.as_mut()?.seek(&mut self.race, to) as f32;
        // the trail's clock moves too: the renderer clears and replays the footprints left
        self.sim_secs = (self.sim_secs + moved).max(0.0);
        let now = self.sim_secs;
        while self.trail.back().is_some_and(|t| t.time > now) {
            self.trail.pop_back();
        }
        for l in self.trail_last.iter_mut().filter(|l| l.1 > now) {
            l.1 = f32::NEG_INFINITY;
        }
        self.clear_light_walls();
        for (i, r) in self.race.racers.iter().enumerate() {
            let radius = self.wheels.get(i).map_or(0.35, |w| w.1).max(0.1) * r.car.p.scale;
            let k = self.spin_scale(radius);
            for (a, rate) in self.spin[i].iter_mut().zip(r.car.wheel_spin) {
                *a = (*a + rate * k * moved) % std::f32::consts::TAU;
            }
        }
        self.eye = None;
        self.follow(moved.abs().min(0.1));
        Some(moved as f64)
    }

    /// REMASTER demo play (#346): the race was put at another moment (a seek in `playback`). The eased chase eye starts
    /// again at the car, and the marks the old moment left (skid marks, smoke, sparks, the grass trail) go. The
    /// wheels' roll keeps its angle.
    pub fn jumped(&mut self) {
        self.eye = None;
        self.fx.clear();
        self.skids.iter_mut().flatten().for_each(|s| s.clear());
        self.skid_head.iter_mut().flatten().for_each(|h| *h = None);
        self.trail.clear();
        self.trail_last.iter_mut().for_each(|l| l.1 = f32::NEG_INFINITY);
        self.clear_light_walls();
        self.follow(1.0 / 60.0);
    }

    /// DEBUG: the race back to the state it was loaded in (with a timeline: a new run from there).
    pub fn restart(&mut self) {
        match self.timeline.as_mut() {
            Some(t) => t.restart(&mut self.race),
            None => self.race.restart(),
        }
    }

    /// DEBUG (#60 phase 2): the run as a repro file: `setup` plus every recorded frame, with the
    /// human's finish and best lap as they stand at the end of the run.
    pub fn recording(&mut self) -> Result<Recording, String> {
        let mut header = self.setup.clone().ok_or("this run has no setup to record")?;
        // #374: the file names the physics the frames were stepped on, so a replay makes the same ticks
        header.tick_hz = self.race.fixed_hz();
        let t = self.timeline.as_mut().ok_or("no timeline")?;
        if !t.whole() {
            return Err("the start of the run was dropped (more than 10 minutes)".into());
        }
        let (race_ms, best_lap_ms) = t.at_end(&mut self.race, |r| r.racers.iter().find(|x| x.human).map_or((None, None), |h| (h.progress.finished_ms, h.progress.best_lap_ms)));
        (header.race_ms, header.best_lap_ms) = (race_ms, best_lap_ms);
        Ok(Recording { header, frames: t.frames().to_vec() })
    }

    /// Tyre half-widths and radius per racer, from the loaded scene.
    pub fn set_wheels(&mut self, scene: &crate::scene::Scene) {
        self.fx.layers = scene.fx.clone();
        self.tyre_layer = scene.tyre_layer;
        for (i, dst) in self.wheels.iter_mut().enumerate() {
            if let (Some(hw), Some(h)) = (scene.wheel_half_width.get(i), scene.wheel_hubs.get(i)) {
                *dst = (*hw, h.1);
            }
        }
    }

    /// Wheel `w` of car `car`: centre and ground contact in body space (game axes). The game
    /// puts the wheel object at PhyWheel x * 1.25, offset along the spring by the live
    /// extension (0x40cb46). Here the tyre's bottom sits on that point [I: the frame of the
    /// hub against the ray is not pinned down].
    pub fn wheel_local(car: &Car, w: usize, radius: f32) -> (Vec3, Vec3) {
        Self::wheel_at(&car.p, car.wheel_ext[w], w, radius)
    }

    /// `wheel_local` from the parameters and one spring extension.
    fn wheel_at(p: &motor_sim::car::CarParams, ext: f32, w: usize, radius: f32) -> (Vec3, Vec3) {
        let hub = p.wheels[w];
        let contact = Vec3::new(hub.x * WHEEL_X_SCALE, hub.y - p.spring_len * ext, hub.z);
        (contact + Vec3::Y * radius, contact)
    }

    /// A sideways slide by the game's own test: lateral speed above the surface's
    /// SpinSpeedLimit x grip[0] (0x47db00; braking and wheelspin don't count here).
    fn sideways(&self, slot: usize, wheel: usize) -> bool {
        let r = &self.race.racers[slot];
        let car = &r.car;
        let mat = car.wheel_mat[wheel].and_then(|m| self.race.world.materials.get(m));
        let limit = mat.map_or(f32::MAX, |m| m.spin_speed_limit * car.p.grip[0]);
        car.vel.x.abs() > limit && r.telemetry.kmh.abs() > 20.0
    }

    /// A wheel on the ground slides sideways (statistics: drift distance).
    pub fn drifting(&self, slot: usize) -> bool {
        let car = &self.race.racers[slot].car;
        (0..4).any(|w| car.wheel_contact[w] && self.sideways(slot, w))
    }

    /// Extend skid strips under marking wheels (CLASSIC 0x446644, 08-missing-features 8.6): a ground
    /// point under the wheel (a ray down from its physics hub) is joined to where its strip last
    /// ended; the segment grows with the wheel until it is `t` long, then goes into the wheel's ring.
    #[allow(clippy::needless_range_loop)] // per-wheel arrays indexed in parallel
    fn lay_skids(&mut self) {
        // CHEAT ramlosa / h2o (0x44667a, #317): every ring is cleared every frame, so there are no marks
        if self.underwater() {
            self.skids.iter_mut().flatten().for_each(std::collections::VecDeque::clear);
            self.skid_head.iter_mut().flatten().for_each(|h| *h = None);
            return;
        }
        for (i, r) in self.race.racers.iter().enumerate() {
            let car = &r.car;
            let radius = self.wheels[i].1 * car.p.scale;
            // 0x4467d5..0x446926: which wheels ask to mark (the rule is Car::skid_marking)
            let marking = car.skid_marking();
            // 0x4469ac..0x4469e5: the step grows with the body speed, capped at 3 m
            let step2 = skid_step2(car.vel);
            for w in 0..4 {
                let mat = car.wheel_mat[w].and_then(|m| self.race.world.materials.get(m));
                let limit = mat.map_or(f32::MAX, |m| m.spin_speed_limit * car.p.grip[0]);
                // CLASSIC: only surfaces with a SkidMark kind take marks (0x446705)
                let kind = mat.map_or(0, |m| m.skid_mark);
                if !(marking[w] && car.wheel_contact[w]) || skid_colour(kind).is_none() {
                    Self::end_skid(&mut self.skids[i][w], self.skid_head[i][w].take());
                    continue;
                }
                // #374: the hub of the wheel as drawn (the view pose), so the strip ends under the tyre
                let v = self.race.view_pose(i);
                let (centre, _) = Self::wheel_at(&car.p, v.wheel_ext[w], w, radius);
                let hub = v.pos + v.rot * centre;
                let Some(hit) = self.race.world.raycast(hub, hub - v.rot.y_axis * (radius + 1.0)) else { continue };
                let p = hit.point + hit.normal * SKID_LIFT.get();
                // REMASTER detailed marks: harder slides leave darker marks
                let min = SKID_MIN.get();
                let s = (min + (1.0 - min) * (car.vel.x.abs() / limit - 1.0) / SKID_FULL.get()).clamp(min.min(1.0), 1.0);
                let ring = &mut self.skids[i][w];
                let head = &mut self.skid_head[i][w];
                let Some(h) = head.as_mut() else {
                    *head = Some(SkidHead { p, v: 0.0, s, prev_side: None, chained: false, open: None });
                    continue;
                };
                // horizontal distance² from the segment's start (0x44697f)
                let d2 = (p.x - h.p.x).powi(2) + (p.z - h.p.z).powi(2);
                if d2 > SKID_RESTART2.get() {
                    // 0x446a00: the original leaves the strip stuck until the marking stops; ours
                    // starts a new strip here [design choice: a respawn or jump leaves no stuck wheel]
                    Self::end_skid(ring, head.take());
                    *head = Some(SkidHead { p, v: 0.0, s, prev_side: None, chained: false, open: None });
                    continue;
                }
                if d2 <= SKID_MIN_STEP2.get() {
                    h.open = None;
                    continue;
                }
                let d = h.p.distance(p);
                let side = (p - h.p).normalize_or_zero().cross(hit.normal).normalize_or_zero() * (SKID_HALF_WIDTH.get() * car.p.scale);
                let (side_q, s0) = match h.prev_side {
                    Some(prev) => (skid_mitre(prev, side), h.s),
                    None => (side, 0.0),
                };
                let seg = SkidSeg { q: h.p, p, side, side_q, side_p: side, kind, v0: h.v, v1: h.v + d, s0, s1: s };
                if d2 > step2 {
                    // committed (0x446a30): the next segment starts at this end
                    if let (true, Some(prev)) = (h.chained, ring.back_mut()) {
                        prev.side_p = side_q;
                    }
                    ring.push_back(seg);
                    // `while`: a lowered SKID_RING trims the strips that are already longer
                    while ring.len() > SKID_RING.get() {
                        ring.pop_front();
                    }
                    (h.open, h.p, h.v, h.s, h.prev_side, h.chained) = (None, p, h.v + d, s, Some(side), true);
                } else {
                    h.open = Some(seg);
                }
            }
        }
    }

    /// CHEAT tribute to tron, REMASTER Neon grid (#320 phase 3, design choice): each racer's light wall grows from
    /// the middle of its rear wheels' ground points. Only in a Tron race (nothing is kept otherwise); the Classic
    /// view does not draw it, so F9 shows it at once.
    fn raise_light_walls(&mut self) {
        if !self.tron() {
            return;
        }
        let now = self.sim_secs;
        for (i, r) in self.race.racers.iter().enumerate() {
            let radius = self.wheel_radius(i) * r.car.p.scale;
            let v = self.race.view_pose(i);
            let ground = |w| v.pos + v.rot * Self::wheel_at(&r.car.p, v.wheel_ext[w], w, radius).1;
            let pt = WallPt { pos: (ground(2) + ground(3)) * 0.5, up: v.rot * Vec3::Y, t: now };
            push_wall_point(&mut self.light_walls[i], pt);
            self.light_head[i] = Some(pt);
        }
    }

    fn clear_light_walls(&mut self) {
        self.light_walls.iter_mut().for_each(std::collections::VecDeque::clear);
        self.light_head.iter_mut().for_each(|h| *h = None);
    }

    /// CHEAT tribute to tron, REMASTER Neon grid (#320): the light walls' triangles (view space, `fs_light_wall`):
    /// the followed car's in `LIGHT_WALL_YOU`, the others in `LIGHT_WALL_RIVAL`. Empty outside a Tron race.
    pub fn light_walls(&self) -> Vec<Vertex> {
        let mut out = Vec::new();
        if !self.tron() {
            return out;
        }
        for (i, (ring, head)) in self.light_walls.iter().zip(&self.light_head).enumerate() {
            let rgb = if i == self.focus { LIGHT_WALL_YOU } else { LIGHT_WALL_RIVAL };
            let live = head.filter(|h| ring.back() != Some(h));
            wall_quads(&mut out, ring.iter().chain(live.iter()), self.sim_secs, rgb);
        }
        out
    }

    /// A strip ends: its last committed segment fades out (REMASTER detailed marks); the live
    /// segment is dropped, as the original clears its flags (0x44692b).
    fn end_skid(ring: &mut std::collections::VecDeque<SkidSeg>, head: Option<SkidHead>) {
        if let (Some(SkidHead { chained: true, .. }), Some(last)) = (head, ring.back_mut()) {
            last.s1 = 0.0;
        }
    }

    /// Skid-mark triangles (view space), six per segment. `detailed` (REMASTER, needs the tread
    /// layer): mitred strips with tread uv and strength for `fs_skid`; otherwise the original's flat
    /// quads in the type colour.
    pub fn skid_marks(&self, detailed: bool) -> Vec<Vertex> {
        let open = self.skid_head.iter().flatten().flatten().filter_map(|h| h.open.as_ref());
        let rings = self.skids.iter().flatten().flatten();
        let segs = rings.chain(open);
        // capacity is a hint only: a typed SKID_RING of millions must not reserve gigabytes
        let mut out = Vec::with_capacity(self.skids.len() * 4 * SKID_RING.get().min(500) * 6);
        let tile = SKID_TILE.get();
        for g in segs {
            let Some(rgba) = skid_colour(g.kind) else { continue };
            // the kind rides in `env` of the detailed vertices only: the quads stay as they were
            let at = |x: Vec3, uv: [f32; 2], color: [f32; 4], across: f32, layer: f32| Vertex { pos: (x * FLIP).into(), uv, color, layer, normal: [across, 0.0, 0.0], env: if layer >= 0.0 { g.kind as f32 } else { 0.0 } };
            if let (true, Some(layer)) = (detailed, self.tyre_layer) {
                // u: across the tread in tile units (the strip's middle on the tile's middle); v:
                // metres along the strip in tiles. Edge distance (-1..1) rides in the normal.
                let hw = g.side.length() / tile;
                // REMASTER detailed marks keep their rubber a touch lighter than pure black
                let c = |s: f32| [rgba[0].max(0.03), rgba[1].max(0.03), rgba[2].max(0.03), s];
                let (v0, v1) = (g.v0 / tile, g.v1 / tile);
                let a = at(g.q - g.side_q, [0.5 - hw, v0], c(g.s0), -1.0, layer);
                let b = at(g.q + g.side_q, [0.5 + hw, v0], c(g.s0), 1.0, layer);
                let cc = at(g.p + g.side_p, [0.5 + hw, v1], c(g.s1), 1.0, layer);
                let d = at(g.p - g.side_p, [0.5 - hw, v1], c(g.s1), -1.0, layer);
                out.extend([a, b, cc, a, cc, d]);
            } else {
                let (a, b, c, d) = (at(g.q - g.side, [0.0; 2], rgba, 0.0, -1.0), at(g.q + g.side, [0.0; 2], rgba, 0.0, -1.0), at(g.p + g.side, [0.0; 2], rgba, 0.0, -1.0), at(g.p - g.side, [0.0; 2], rgba, 0.0, -1.0));
                out.extend([a, b, c, a, c, d]);
            }
        }
        out
    }

    /// Ease the chase eye towards its target behind the focused car (`ease_eye`, 0x42c540).
    fn follow(&mut self, dt: f32) {
        // CHEAT supercars / grem, ignition / uds (0x45b1f0): the in-car, TV, fly-by and goal camera flags are cleared and
        // chase set every frame; ignition has only the one chase view (its far chase is ours)
        let ignition = self.ignition();
        if (ignition && self.cam != CamMode::Chase) || (self.top_down() && self.cam.hides_car()) {
            self.cam = CamMode::Chase;
        }
        let t = self.base_tune();
        let (origin, rot) = self.view_origin();
        let local = match (self.cam, self.look_back) {
            (CamMode::FarChase, false) => t.far_eye,
            (_, true) => chase_back_eye(),
            _ => t.chase_eye,
        };
        // CLASSIC [O] (#274 trace, frame of the key press): looking back in a chase view puts the eye
        // at its fixed spot at once, no easing; the eased eye follows it, so letting go sweeps it
        // back round the car the way the original does
        let eye = match self.look_back && matches!(self.cam, CamMode::Chase | CamMode::FarChase) {
            true => origin + rot * chase_back_eye(),
            // CHEAT ignition / uds (0x42c540 -> 0x42c944); look back keeps the fixed back view (0x704e04 is not cleared)
            false if ignition => ignition_eye(self.eye, origin, rot, dt),
            false => ease_eye(self.eye, origin, rot, local, &t, dt),
        };
        self.eye = Some(eye);
        self.place_eye(eye - origin, dt);
        // CLASSIC [O] (#274 trace): the easing starts from the eye the previous frame drew, in whatever
        // view that was: the first chase frame after in-car, look back or TV is eased from that eye
        // (the TV eye 30 m away swings in over 0.4 s), not from a chase eye kept up in the background
        if !matches!(self.cam, CamMode::Chase | CamMode::FarChase) || self.flying() {
            self.eye = Some(self.base_camera().pos * FLIP);
        }
    }

    /// Free look and wall pull-in on top of the eased chase offset `off` (both design choices;
    /// `self.eye` keeps the plain eased offset so neither feeds back into the lag easing).
    fn place_eye(&mut self, off: Vec3, dt: f32) {
        let chase = matches!(self.cam, CamMode::Chase | CamMode::FarChase);
        let stick = if chase && !self.look_back { self.free_look.clamp(Vec2::NEG_ONE, Vec2::ONE) } else { Vec2::ZERO };
        // the stick points the camera: up looks ahead (eye behind the car), down looks back
        // (eye in front), right shows the car's left side; up to ORBIT_HOLD off straight up
        // the eye stays behind
        let target = if stick == Vec2::ZERO { 0.0 } else {
            let a = stick.x.atan2(stick.y);
            if a.abs() <= ORBIT_HOLD.get() { 0.0 } else { a }
        };
        let d = (target - self.orbit + PI).rem_euclid(TAU) - PI;
        self.orbit = (self.orbit + d * (1.0 - (-ORBIT_RATE.get() * dt).exp()) + PI).rem_euclid(TAU) - PI;
        let off = Quat::from_rotation_y(self.orbit) * off;
        let (origin, t) = (self.view_origin().0, self.tune_now());
        let look = match (self.cam, self.look_back) {
            (CamMode::FarChase, false) => t.far_look,
            _ => t.chase_look,
        };
        self.cam_off = Some(pull_eye(&self.race.world, origin, off, look, &t, self.pull_in, &mut self.cam_dist, dt));
    }

    /// Current camera (eye, look-at point), view space.
    pub fn chase_view(&self) -> (Vec3, Vec3) {
        let (origin, rot) = self.view_origin();
        view_of(&self.race, self.cam, self.look_back, origin, rot, self.cam_off, &self.tune_now())
    }

    /// #374: the followed car's camera origin (`cam_origin`) and axes where it is drawn (`Race::view_pose`).
    pub fn view_origin(&self) -> (Vec3, Mat3) {
        let v = self.race.view_pose(self.focus);
        (cam_origin_at(&self.race.racers[self.focus].car.p, &v), v.rot)
    }

    /// The tuned camera values with this session's chase follow setting on top.
    pub fn base_tune(&self) -> CamTune {
        CamTune::live().followed(self.follow)
    }

    /// The camera tuning of this frame: under ignition / uds the chase looks 2 m ahead of the car (`0x4b02b8`, rotated by
    /// the body, `0x42b844`) instead of `chase_look` (#318).
    fn tune_now(&self) -> CamTune {
        let mut t = self.base_tune();
        if self.ignition() {
            t.chase_look = self.view_origin().1 * Vec3::new(0.0, 0.0, motor_sim::cheats::IGNITION_AHEAD);
        }
        t
    }

    /// The start fly-by is showing: its eye looks at the followed car (0x45b6a4..0x45ba51, #216).
    /// CHEAT ignition / uds (0x45b1f0): the fly-by flag is cleared every frame, the race starts behind the car.
    pub fn flying(&self) -> bool {
        self.fly_by && self.race.start_eye().is_some() && !self.ignition()
    }

    /// The followed car is not drawn from inside it, but it is during the fly-by.
    fn hides_car(&self) -> bool {
        self.cam.hides_car() && !self.flying() && !self.top_down() && !self.ignition()
    }

    /// lemmy / ace "Hell mode" (#316): `Some(lightning)` while the cheat is on, the flash rolled on a 1/30 s tick of the
    /// race clock (`cheats::hell_flash`, a design choice: the original rolls per drawn frame).
    pub fn hell(&self) -> Option<bool> {
        motor_sim::cheats::hell(self.race.name_cheat).then(|| motor_sim::cheats::hell_flash(self.sim_secs))
    }

    /// CHEAT tribute to tron (#320, `0x48e964`): the Glide Tron look; the scene must be loaded with its codes
    /// (`scene::SceneCheat::tron`), which the game does for your own race only.
    pub fn tron(&self) -> bool {
        motor_sim::cheats::tron(self.race.name_cheat)
    }

    /// CHEAT ramlosa / h2o "Underwater" (#317): the teal look and ceiling sky, bubbles from the rear wheels, no skid marks;
    /// the half-speed race is the sim's (`Race::step_as`).
    pub fn underwater(&self) -> bool {
        motor_sim::cheats::underwater(self.race.name_cheat)
    }

    /// CHEAT ramlosa / h2o (#317): `Some(phase)` of the picture's wobble (`0x4419b4`) while the cheat is on; the phase runs
    /// on the session clock (`cheats::wobble_phase`, a design choice: the original steps it per drawn frame).
    pub fn wobble(&self) -> Option<f32> {
        self.underwater().then(|| motor_sim::cheats::wobble_phase(self.sim_secs))
    }

    /// CHEAT supercars / grem (#319): the view is straight down on the followed car.
    pub fn top_down(&self) -> bool {
        motor_sim::cheats::top_down(self.race.name_cheat)
    }

    /// CHEAT ignition / uds (#318): the high chase camera (`ignition_eye`).
    pub fn ignition(&self) -> bool {
        motor_sim::cheats::ignition(self.race.name_cheat)
    }

    /// Skip the start fly-by to the "3" (our design, #216) and give the player's camera back.
    pub fn skip_intro(&mut self) {
        if let Some(t) = self.timeline.as_mut() {
            t.note_skip();
        }
        self.race.skip_intro();
        self.fly_by = false;
    }

    /// The camera to render with: the start fly-by's, the TV camera's shot (position, bank, zoom
    /// and far plane), else `chase_view`.
    pub fn camera(&self) -> Camera {
        let base = self.base_camera();
        if !self.top_down() {
            return base;
        }
        // CHEAT supercars / grem (0x439f7a -> 0x42c8e0, #319) [O]: rows right (1,0,0), up (0,0,1), forward (0,-1,0),
        // north up whatever the car does; eye = body origin + 12 m along the heading (x, z) + 25 m up. Zoom and
        // far plane stay those of the mode.
        let (origin, rot) = self.view_origin();
        Camera::from_rot(top_down_eye(origin, rot * Vec3::Z), TOP_DOWN_ROT, base.zoom, base.far_k)
    }

    /// The camera of the mode without the cheats: where the sound is heard from (the original leaves
    /// its listener `0x704e6c` at the working camera) and what the eased eye starts from.
    pub fn base_camera(&self) -> Camera {
        let (origin, rot) = self.view_origin();
        if let Some(eye) = self.race.start_eye().filter(|_| self.flying()) {
            // CLASSIC 0x42c3a4: look at the car's model origin; look back and free look do not apply [I]
            let target = origin;
            let look = ((target - eye) * FLIP).normalize_or(Vec3::NEG_Z);
            return Camera::new(eye * FLIP, look.x.atan2(-look.z), look.y.asin());
        }
        camera_of(&self.race, self.cam, self.look_back, origin, rot, self.cam_off, &self.tune_now(), self.pull_in)
    }

    /// Rear-view mirror camera (eye, look-at point), view space: the in-car look-back eye
    /// (design choice; the original's mirror camera is [U]).
    pub fn mirror_view(&self) -> (Vec3, Vec3) {
        let v = self.race.view_pose(self.focus);
        let eye = v.pos + v.rot * incar_back_eye();
        (eye * FLIP, (eye - v.rot * Vec3::Z) * FLIP)
    }

    /// Car `slot`'s body where it is drawn (#374: `Race::view_pose`).
    fn model_of(&self, slot: usize) -> Mat4 {
        let v = self.race.view_pose(slot);
        Self::body_model(&self.race.racers[slot].car.p, v.pos, v.rot, &v.wheel_ext, 1.0)
    }

    /// The body's model matrix at a pose. CHEAT gulliver: parameters at a size (`p.scale`, the
    /// physics' own) draw the mesh at that size about the model origin; `scale` shrinks a
    /// full-size pose on top (recorded cars, `physics_frame`; 1.0 = none).
    fn body_model(p: &motor_sim::car::CarParams, pos: Vec3, rot: Mat3, ext: &[f32; 4], scale: f32) -> Mat4 {
        let s = Mat3::from_diagonal(FLIP);
        let m = if scale == 1.0 {
            let origin = pos + rot * Vec3::new(0.0, p.ride_height(), 0.0);
            Mat4::from_translation(origin * FLIP) * Mat4::from_mat3(s * rot * s)
        } else {
            Self::physics_frame(p, pos, rot, ext, scale) * Mat4::from_translation(Vec3::new(0.0, p.ride_height(), 0.0) * FLIP)
        };
        Self::sized(m, p.scale)
    }

    /// `m` with the mesh scaled by `scale` in its own frame (CHEAT gulliver; 1.0 = `m` unchanged).
    fn sized(m: Mat4, scale: f32) -> Mat4 {
        if scale == 1.0 { m } else { m * Mat4::from_scale(Vec3::splat(scale)) }
    }

    /// The car's physics frame (origin `pos`, axes `rot`), view space. CHEAT gulliver: shrunk by
    /// `scale` about the ground under the tyres (the mean contact height), so a full-size pose
    /// (a recorded car) sits on the road.
    fn physics_frame(p: &motor_sim::car::CarParams, pos: Vec3, rot: Mat3, ext: &[f32; 4], scale: f32) -> Mat4 {
        let s = Mat3::from_diagonal(FLIP);
        let frame = Mat4::from_translation(pos * FLIP) * Mat4::from_mat3(s * rot * s);
        if scale == 1.0 {
            return frame;
        }
        let ground = (0..4).map(|w| p.wheels[w].y - p.spring_len * ext[w]).sum::<f32>() / 4.0;
        let g = Vec3::new(0.0, ground, 0.0) * FLIP;
        frame * Mat4::from_translation(g) * Mat4::from_scale(Vec3::splat(scale)) * Mat4::from_translation(-g)
    }

    /// REMASTER, Enhanced car paint: where to capture the env cube (above the followed car's body
    /// origin, view space) and that car's meshes, left out of the capture.
    pub fn env_probe(&self) -> (Vec3, std::ops::Range<usize>) {
        let m = self.model_of(self.focus);
        (m.transform_point3(Vec3::Y * ENV_PROBE_HEIGHT.get()), self.focus * CAR_MESHES..(self.focus + 1) * CAR_MESHES)
    }

    /// (mesh index, model matrix) for every car; meshes are loaded in grid-slot order.
    pub fn models(&self) -> Vec<(usize, Mat4)> {
        self.models_hiding(self.hides_car().then_some(self.focus))
    }

    /// The body (mesh index, model matrix) of the car a view leaves out (`hide`, or by default
    /// the followed car when the camera hides it): its lights still light the scene (REMASTER
    /// car lights).
    pub fn hidden_bodies(&self, hide: Option<usize>) -> Vec<(usize, Mat4)> {
        let hide = hide.or(self.hides_car().then_some(self.focus));
        hide.filter(|&i| i < self.race.racers.len()).map(|i| (i * CAR_MESHES, self.model_of(i))).into_iter().collect()
    }

    /// `models` without car `hide` (in-car views, the rear mirror).
    pub fn models_hiding(&self, hide: Option<usize>) -> Vec<(usize, Mat4)> {
        let mut out = Vec::new();
        // Knockout: eliminated cars are gone from the track
        for (i, r) in self.race.racers.iter().enumerate().filter(|&(i, r)| Some(i) != hide && r.progress.eliminated_ms.is_none()) {
            let car = &r.car;
            let (_, radius) = self.wheels[i];
            // #374: where the car is drawn (a remote car: its latest pose)
            let v = self.race.view_pose(i);
            if r.remote {
                // a remote racer keeps its full-size parameters: shrunk as a recorded pose
                let scale = self.race.car_scale.0;
                out.push((i * CAR_MESHES, Self::body_model(&car.p, v.pos, v.rot, &v.wheel_ext, scale)));
                out.extend(Self::wheel_models(&car.p, v.pos, v.rot, &v.wheel_ext, v.steer_smooth, self.spin[i], radius, i * CAR_MESHES, scale));
                continue;
            }
            out.push((i * CAR_MESHES, self.model_of(i)));
            out.extend(Self::wheel_models(&car.p, v.pos, v.rot, &v.wheel_ext, v.steer_smooth, self.spin[i], radius * car.p.scale, i * CAR_MESHES, 1.0));
        }
        out
    }

    /// The four wheel meshes after the body at `base` (mesh index, model matrix): in the physics
    /// frame (not the body's ride-height-adjusted one). `radius` is the tyre's at `p.scale`.
    #[allow(clippy::too_many_arguments)]
    fn wheel_models(p: &motor_sim::car::CarParams, pos: Vec3, rot: Mat3, ext: &[f32; 4], steer_smooth: f32, spin: [f32; 2], radius: f32, base: usize, scale: f32) -> Vec<(usize, Mat4)> {
        let frame = Self::physics_frame(p, pos, rot, ext, scale);
        let steer = steer_smooth * WHEEL_STEER.get();
        (0..4)
            .map(|w| {
                let (centre, _) = Self::wheel_at(p, ext[w], w, radius);
                // front wheels (FL, FR) steer; all roll about the axle (view x), forward = -z
                let yaw = if w < 2 { -steer } else { 0.0 };
                let m = frame * Mat4::from_translation(centre * FLIP) * Mat4::from_rotation_y(yaw) * Mat4::from_rotation_x(-spin[w / 2]);
                (base + 1 + w, Self::sized(m, p.scale))
            })
            .collect()
    }

    /// DESIGN CHOICE (Time Attack ghost): body and wheels of the player's car (`slot`) at a
    /// recorded pose, as `models` draws a car; meshes are the player's own (same car).
    pub fn ghost_models(&self, slot: usize, pose: &motor_sim::replay::GhostPose) -> Vec<(usize, Mat4)> {
        // recorded poses are full-size: the full-size parameters, shrunk about the ground [I: a
        // ghost recorded with the cheat on sits low]
        Self::pose_models(&self.race.racers[slot].car.p.scaled(1.0, 1.0), slot, self.wheels[slot].1, pose, self.race.car_scale.0)
    }

    /// Body and wheels of the car whose meshes start at car slot `slot` (parameters `p`, wheel radius
    /// `radius`) at `pose`, laid out as `models` does. Ghost mode draws the recorded cars with it;
    /// their slots lie past the racers (the scene loads their meshes after the grid's).
    pub fn pose_models(p: &motor_sim::car::CarParams, slot: usize, radius: f32, pose: &motor_sim::replay::GhostPose, scale: f32) -> Vec<(usize, Mat4)> {
        let body = Self::body_model(p, pose.pos, pose.rot, &pose.wheel_ext, scale);
        let spin = [(pose.roll / radius.max(0.1)) % std::f32::consts::TAU; 2];
        let mut out = vec![(slot * CAR_MESHES, body)];
        out.extend(Self::wheel_models(p, pose.pos, pose.rot, &pose.wheel_ext, pose.steer_smooth, spin, radius, slot * CAR_MESHES, scale));
        out
    }

    /// (mesh index, model matrix) for every scene object (`Scene::props` order).
    pub fn prop_models(&self) -> Vec<(usize, Mat4)> {
        let s = Mat3::from_diagonal(FLIP);
        self.race
            .props
            .props
            .iter()
            .map(|p| {
                let q = &p.pose;
                (p.mesh, Mat4::from_translation(q.pos * FLIP) * Mat4::from_mat3(s * q.rot * s))
            })
            .collect()
    }

    /// CLASSIC podium emblem (0x44b6e8, called from 0x45c4f7) [O]: once the player has
    /// finished, `first/second/third.myo` for places 1–3 flies along `finish.mot`. The keys are
    /// in camera space: the object matrix is the pose times the camera's (0x42bca0 with camera
    /// +0x1c, its camera-to-world matrix). The clock starts at the finish and holds 0.01 s
    /// before the last key; the motion ends centred 1.76 ahead of the eye. (mesh, model) as
    /// `prop_models`; the mesh follows the .lws ones (`scene::FINISH_EMBLEMS`).
    pub fn finish_emblem(&self, cam: &Camera) -> Option<(usize, Mat4)> {
        let m = self.finish_motion.as_ref()?;
        let slot = self.race.racers.iter().position(|r| r.human).unwrap_or(self.focus);
        let at = self.race.racers[slot].progress.finished_ms?;
        let place = self.race.standings().iter().position(|&s| s == slot).filter(|&p| p < 3)?;
        let fps = FINISH_FPS.get();
        let end = m.keys.last()?.frame / fps;
        // `.max(0.0)`: a typed FINISH_FPS that makes the motion shorter than 0.01 s must not make clamp panic
        let t = (((self.race.clock_ms - at as f64) * 0.001) as f32).clamp(0.0, (end - 0.01).max(0.0));
        let q = props::Pose::from_channels(&props::evaluate(m, t * fps, false));
        let (f, r) = (cam.forward(), cam.right());
        let cam_to_world = Mat4::from_cols(r.extend(0.0), r.cross(f).extend(0.0), (-f).extend(0.0), cam.pos.extend(1.0));
        let s = Mat3::from_diagonal(FLIP);
        let model = cam_to_world * Mat4::from_translation(q.pos * FLIP) * Mat4::from_mat3(s * q.rot * s);
        Some((self.race.props.meshes.len() + place, model))
    }

    /// Per scene object (as `prop_models`): its animated scale, which multiplies its halos'
    /// colour (0x4354fc) and leaves the mesh alone.
    pub fn prop_glows(&self) -> Vec<Vec3> {
        self.race.props.props.iter().map(|p| p.pose.scale).collect()
    }

    /// Factor from `Car::wheel_spin` (rad/s of a 0.5 m wheel: 2 vz, 0x40ca90 has no radius) to the
    /// roll rate of a tyre of `radius`.
    fn spin_scale(&self, radius: f32) -> f32 {
        if self.fx.remaster {
            0.5 / radius // REMASTER: the tyre rolls with the ground
        } else {
            1.0 // CLASSIC: angle += rate * dt as in 0x40ca90
        }
    }

    /// Per car in grid-slot order: the front and rear wheels' spin angles (radians), as `models`
    /// turns them.
    pub fn wheel_spins(&self) -> &[[f32; 2]] {
        &self.spin
    }

    /// Per car in grid-slot order (car mesh index / `CAR_MESHES`): this frame's brake input,
    /// which brightens its tail lights in Remaster [design choice].
    pub fn car_brakes(&self) -> Vec<f32> {
        self.race.racers.iter().map(|r| r.brake).collect()
    }

    /// REMASTER grass: per racer its velocity (view space) and hull half extents, for the
    /// blades it presses (`Frame::car_motion`).
    pub fn car_motions(&self) -> Vec<crate::render::CarMotion> {
        self.race
            .racers
            .iter()
            .map(|r| {
                let c = &r.car.p.corners;
                let half = [c.iter().map(|v| v.z.abs()).fold(0.0, f32::max), c.iter().map(|v| v.x.abs()).fold(0.0, f32::max)];
                crate::render::CarMotion { vel: r.car.world_vel() * FLIP, half }
            })
            .collect()
    }

    /// Car shadows (0x438308): a ground plane from three downward rays under shadow points
    /// 5, 6 and 10; the 16 points are dropped vertically onto it and drawn as a 3x3 grid of
    /// quads. APPROX: the original starts the first ray at the midpoint of points 5 and 6.
    pub fn shadows(&self) -> Vec<Vertex> {
        self.shadows_except(None)
    }

    /// `shadows` without car `skip`'s (a ghost-like car casts none).
    pub fn shadows_except(&self, skip: Option<usize>) -> Vec<Vertex> {
        let mut out = Vec::new();
        for (i, (r, pts)) in self.race.racers.iter().zip(&self.shadow_points).enumerate().filter(|(i, (r, _))| Some(*i) != skip && r.progress.eliminated_ms.is_none()) {
            let Some(pts) = pts else { continue };
            let scale = if r.remote { self.race.car_scale.0 } else { r.car.p.scale };
            let v = self.race.view_pose(i);
            self.shadow_quads(v.pos, v.rot, pts, scale, &mut out);
        }
        out
    }

    /// The shadow of a car at `pos`/`rot` with shadow points `pts` (see `shadows`), appended to `out`.
    /// `scale`: the car's size (CHEAT gulliver, 1.0 = full).
    pub fn shadow_quads(&self, pos: Vec3, rot: Mat3, pts: &[Vec3; 16], scale: f32, out: &mut Vec<Vertex>) {
        // CHEAT gulliver: the shadow shrinks about the car's own origin (the ground point under it)
        let world: Vec<Vec3> = pts.iter().map(|&p| pos + rot * (Vec3::new(p.x * scale, p.y, p.z * scale))).collect();
        let (up, down) = (SHADOW_RAY_UP.get(), SHADOW_RAY_DOWN.get());
        let hit = |p: Vec3| self.race.world.raycast(p + Vec3::Y * up, p - Vec3::Y * down).map(|h| h.point);
        let (Some(a), Some(b), Some(c)) = (hit(world[5]), hit(world[6]), hit(world[10])) else { return };
        let mut n = (b - a).cross(c - a).normalize_or_zero();
        if n.y < 0.0 {
            n = -n;
        }
        if n.y < SHADOW_MIN_NY.get() {
            return;
        }
        let colour = [SHADOW_RGB[0], SHADOW_RGB[1], SHADOW_RGB[2], SHADOW_ALPHA.get()];
        // drop onto the plane n.(q - a) = 0, plus a hair against z-fighting
        let drop = |p: Vec3| {
            let y = a.y - (n.x * (p.x - a.x) + n.z * (p.z - a.z)) / n.y;
            Vertex { pos: (Vec3::new(p.x, y + 0.02, p.z) * FLIP).into(), uv: [0.0; 2], color: colour, layer: -1.0, normal: [0.0; 3], env: 0.0 }
        };
        let v: Vec<Vertex> = world.iter().map(|&p| drop(p)).collect();
        for row in 0..3 {
            for col in 0..3 {
                let i = row * 4 + col;
                out.extend([v[i], v[i + 1], v[i + 5], v[i], v[i + 5], v[i + 4]]);
            }
        }
    }

    /// Focused car position (model ground plane) and flat forward direction, view space.
    pub fn pose_view(&self) -> (Vec3, Vec3) {
        let ride = self.race.racers[self.focus].car.p.ride_height();
        let v = self.race.view_pose(self.focus);
        let fwd = v.rot * Vec3::Z * FLIP;
        (v.pos * FLIP + Vec3::Y * ride, Vec3::new(fwd.x, 0.0, fwd.z).normalize_or(Vec3::NEG_Z))
    }

    /// Wheel radius of racer `slot` (from the scene; 0.35 until `set_wheels`).
    pub fn wheel_radius(&self, slot: usize) -> f32 {
        self.wheels.get(slot).map_or(0.35, |w| w.1)
    }

    /// The per-frame scene inputs of a race view (shared by every `Frame` built from a session).
    /// `shadows`: projected car shadows (off when the view has real ones); `skids`: skid marks,
    /// `detailed` the Remaster ones.
    pub fn frame_parts(&self, cam: &Camera, shadows: bool, skids: bool, detailed: bool) -> FrameParts {
        let mut props = self.prop_models();
        // the followed car's podium emblem once it has finished (as in the game)
        props.extend(self.finish_emblem(cam));
        FrameParts {
            cars: self.models(),
            shadows: if shadows { self.shadows() } else { Vec::new() },
            skids: if skids { self.skid_marks(detailed) } else { Vec::new() },
            props,
            glows: self.prop_glows(),
            brakes: self.car_brakes(),
            spins: self.wheel_spins().to_vec(),
            motion: self.car_motions(),
            trail: self.trail.iter().copied().collect(),
            sim_secs: self.sim_secs,
            hidden: self.hidden_bodies(None),
            ghost: Vec::new(),
            sprites: self.fx.sprites(),
            env_probe: Some(self.env_probe()),
            frost: None,
            // CLASSIC, CHEAT supercars / grem, ignition / uds: OBJ3 / OBJ2 cars on a black background (0x4392b4, 0x43957c, 0x443a99)
            cheat_level: motor_sim::cheats::car_level(self.race.name_cheat),
            hell: self.hell(),
            underwater: self.wobble(),
            tron: self.tron(),
            light_walls: self.light_walls(),
        }
    }

    /// Put the followed car back on the racing line. Not before the start (fly-by and 3-2-1):
    /// every press would move the car further ahead of the grid (#301, [design choice]: the
    /// original has no respawn key). Returns whether it respawned.
    pub fn respawn_focus(&mut self) -> bool {
        if !self.race.started() {
            return false;
        }
        if let Some(t) = self.timeline.as_mut() {
            t.note_respawn(self.focus);
        }
        self.race.respawn(self.focus);
        true
    }

    /// One-line race HUD for the window title.
    pub fn hud(&self) -> String {
        let r = &self.race.racers[self.focus];
        let p = &r.progress;
        let t = &r.telemetry;
        let pos = self.race.standings().iter().position(|&i| i == self.focus).unwrap_or(0) + 1;
        let gear = if t.gear == 0 { "R".to_string() } else { t.gear.to_string() };
        let mut s = format!("P{pos}/{}  Lap {}/{}  ", self.race.racers.len(), p.lap, p.laps);
        if !self.race.started() {
            s += &format!("— {} —  ", self.race.countdown.ceil().max(1.0));
        } else if let Some(f) = p.finished_ms {
            s += &format!("FINISHED {}  ", fmt_ms(f));
        } else {
            s += &fmt_ms(self.race.clock_ms as u32);
            s += "  ";
        }
        s += &format!("{:3.0} km/h  gear {gear}", t.kmh);
        if let Some(l) = p.last_lap_ms {
            s += &format!("  last {}", fmt_ms(l));
        }
        if let Some(b) = p.best_lap_ms {
            s += &format!("  best {}", fmt_ms(b));
        }
        if let Some((cp, at)) = p.last_checkpoint {
            if (self.race.clock_ms as u32).saturating_sub(at) < 3000 {
                s += &format!("  CHECKPOINT {cp} {}", fmt_ms(at.saturating_sub(p.lap_start_ms)));
            }
        }
        if p.wrong_way {
            s += "  WRONG WAY";
        }
        s
    }
}

/// Owned per-frame inputs of a race view (`Session::frame_parts`); `frame` borrows them.
#[derive(Default)]
pub struct FrameParts {
    pub cars: Vec<(usize, Mat4)>,
    pub shadows: Vec<Vertex>,
    pub skids: Vec<Vertex>,
    pub props: Vec<(usize, Mat4)>,
    pub glows: Vec<Vec3>,
    pub brakes: Vec<f32>,
    pub spins: Vec<[f32; 2]>,
    /// REMASTER grass: per racer, how it presses the blades
    pub motion: Vec<crate::render::CarMotion>,
    /// REMASTER grass trail: the footprints of the last `TRAIL_RELAX` seconds and the sim clock
    pub trail: Vec<TrailStamp>,
    pub sim_secs: f32,
    pub hidden: Vec<(usize, Mat4)>,
    /// Time Attack ghost (`Session::ghost_models`)
    pub ghost: Vec<(usize, Mat4)>,
    pub sprites: Vec<crate::fx::Sprite>,
    pub env_probe: Option<(Vec3, std::ops::Range<usize>)>,
    /// REMASTER: the scene blurred behind these cards (the Glass pause menu, #300)
    pub frost: Option<crate::render::SceneFrost>,
    /// CLASSIC: every car at least this body level, a black background (`cheats::car_level`)
    pub cheat_level: Option<usize>,
    pub hell: Option<bool>,
    pub underwater: Option<f32>,
    pub tron: bool,
    /// CHEAT tribute to tron, REMASTER Neon grid (#320): `Session::light_walls`
    pub light_walls: Vec<Vertex>,
}

impl FrameParts {
    pub fn frame(&self) -> crate::render::Frame<'_> {
        let mut f = crate::render::Frame::new(&self.cars, &self.shadows);
        f.skids = &self.skids;
        f.props = &self.props;
        f.prop_glow = &self.glows;
        f.car_brake = &self.brakes;
        f.car_spin = &self.spins;
        f.car_motion = &self.motion;
        f.trail = &self.trail;
        f.sim_time = self.sim_secs;
        f.light_cars = &self.hidden;
        f.ghost_cars = &self.ghost;
        f.sprites = &self.sprites;
        f.env_probe = self.env_probe.clone();
        f.frost = self.frost;
        f.cheat_level = self.cheat_level;
        f.hell = self.hell;
        f.underwater = self.underwater;
        f.tron = self.tron;
        f.light_walls = &self.light_walls;
        f
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Steady driving settles on the target spot (height = local height, pitch term 3 * forward.y);
    /// a turn lags and then catches up; k is clamped at both ends of the frame rate.
    #[test]
    fn top_down_eye_is_above_and_ahead() {
        let eye = top_down_eye(Vec3::new(10.0, 1.5, 20.0), Vec3::new(0.0, 0.3, 1.0));
        assert_eq!(eye, Vec3::new(10.0, 26.5, 32.0));
        // the camera looks straight down with north up whatever the car does
        let cam = Camera::from_rot(eye, TOP_DOWN_ROT, 1.0, 1.0);
        assert!((cam.pitch + std::f32::consts::FRAC_PI_2).abs() < 1e-4, "{}", cam.pitch);
        assert!(cam.right().dot(Vec3::X) > 0.999);
    }

    /// CHEAT ignition / uds (0x42c944): the target is 12 m back along forward x / z and 17 - 3 forward.y up; a step at
    /// 60 fps moves x by 5 %, y by 90 % and z all the way, from the eye put back to the target's distance.
    #[test]
    fn ignition_eye_is_high_behind_and_eases_per_axis() {
        let pos = Vec3::new(10.0, 1.0, 20.0);
        assert_eq!(ignition_eye(None, pos, Mat3::IDENTITY, 1.0 / 60.0), pos + Vec3::new(0.0, 17.0, -12.0));
        let prev = pos + Vec3::new(8.0, 15.0, -10.0);
        let from = pos + (prev - pos).normalize() * Vec3::new(0.0, 17.0, -12.0).length();
        let eye = ignition_eye(Some(prev), pos, Mat3::IDENTITY, 1.0 / 60.0);
        assert!((eye.x - (from.x + (pos.x - from.x) * 0.05)).abs() < 1e-4, "{eye} {from}");
        assert!((eye.y - (from.y + (pos.y + 17.0 - from.y) * 0.9)).abs() < 1e-4, "{eye} {from}");
        assert!((eye.z - (pos.z - 12.0)).abs() < 1e-4, "{eye}");
        // an eye on the target stays there at any frame time
        let rot = Mat3::from_rotation_y(0.7);
        let on = ignition_eye(None, pos, rot, 0.0);
        for dt in [1.0 / 30.0, 1.0 / 60.0, 1.0 / 144.0] {
            assert!(ignition_eye(Some(on), pos, rot, dt).distance(on) < 1e-4);
        }
    }

    #[test]
    fn chase_eye_eases_like_the_original() {
        let t = CamTune::default();
        let (mut pos, rot) = (Vec3::new(5.0, 1.0, 7.0), Mat3::IDENTITY);
        let mut eye = None;
        for _ in 0..200 {
            pos += Vec3::Z * 0.5;
            eye = Some(ease_eye(eye, pos, rot, t.chase_eye, &t, 1.0 / 60.0));
        }
        // moving at 30 m/s the re-projection leaves the eye a little behind the target
        let off = eye.unwrap() - pos;
        assert!((off - t.chase_eye).length() < 0.1 && off.z < t.chase_eye.z, "{off:?}");
        let mut still = eye;
        for _ in 0..200 {
            still = Some(ease_eye(still, pos, rot, t.chase_eye, &t, 1.0 / 60.0));
        }
        assert!((still.unwrap() - pos - t.chase_eye).length() < 1e-3);
        // nose up 0.2: the target drops by 3 * forward.y, not by the rotated offset
        let pitched = Mat3::from_rotation_x(-0.2);
        let fy = (pitched * Vec3::Z).y;
        let e = ease_eye(None, pos, pitched, t.chase_eye, &t, 1.0 / 60.0) - pos;
        assert!((e.y - (2.0 - 3.0 * fy)).abs() < 1e-5, "{e:?}");
        // a 90 degree turn: the eye trails, then settles behind the new heading
        let turned = Mat3::from_rotation_y(-std::f32::consts::FRAC_PI_2);
        let mut e = eye;
        let first = ease_eye(e, pos, turned, t.chase_eye, &t, 1.0 / 60.0) - pos;
        assert!(first.x.abs() < 1.0 && first.z < -3.0, "{first:?}");
        for _ in 0..300 {
            e = Some(ease_eye(e, pos, turned, t.chase_eye, &t, 1.0 / 60.0));
        }
        let end = e.unwrap() - pos;
        assert!((end - turned * t.chase_eye).length() < 1e-2, "{end:?}");
        // k stays within 0.05..0.9 at 240 Hz and at 5 Hz
        let (lo, hi) = (ease_eye(eye, pos, turned, t.chase_eye, &t, 1.0 / 240.0), ease_eye(eye, pos, turned, t.chase_eye, &t, 0.2));
        assert!(lo.is_finite() && hi.is_finite());
    }

    /// The source's camera values (#398 phase 4): `Default` is what the F11 Camera page resets to, `live()` the same
    /// until a tunable is changed (no lib test changes one: they are statics of the test process).
    #[test]
    fn camera_defaults_are_the_originals() {
        let t = CamTune::default();
        assert_eq!(t.chase_eye, Vec3::new(0.0, 2.0, -4.5));
        assert_eq!(t.chase_look, Vec3::new(0.0, 1.5, 0.0));
        assert_eq!(t.far_eye, Vec3::new(0.0, 3.2, -8.0));
        assert_eq!(t.far_look, Vec3::new(0.0, 1.2, 0.0));
        assert_eq!(t.incar_eye, Vec3::new(0.0, 1.0, 0.0));
        assert_eq!(t.bumper_eye, Vec3::new(0.0, 0.55, 2.1));
        assert_eq!((t.k_xz, t.k_y, t.max_lag), (0.15, 0.9, PI));
        assert_eq!((t.margin, t.min, t.out_rate), (0.35, 1.0, 4.0));
        assert_eq!(chase_back_eye(), Vec3::new(0.0, 2.0, 5.0));
        assert_eq!(incar_back_eye(), Vec3::new(0.0, 1.2, -1.0));
        assert_eq!(CamTune::live(), t);
    }

    /// Chase follow (REMASTER Loose / Tight) scales the tuned easing and sets the lag; Normal and CLASSIC (the default
    /// `Follow`) leave the values as they are.
    #[test]
    fn follow_scales_the_easing() {
        let t = CamTune::default();
        assert_eq!(t.followed(Follow::default()), t);
        let loose = t.followed(Follow { ease: 0.5, lag: Some(0.5) });
        assert_eq!((loose.k_xz, loose.k_y, loose.max_lag), (t.k_xz * 0.5, t.k_y * 0.5, 0.5));
        let tight = t.followed(Follow { ease: 1.8, lag: Some(0.15) });
        assert_eq!((tight.k_xz, tight.k_y, tight.max_lag), (t.k_xz * 1.8, 1.0, 0.15));
        // Normal: ease 1, no lag limit
        let normal = t.followed(Follow { ease: 1.0, lag: None });
        assert_eq!(normal, t);
        assert_eq!(loose.chase_eye, t.chase_eye);
    }

    /// Letting go of look back starts the eased eye at the look-back spot in front of the car and
    /// sweeps it round through the car to the chase spot (traced original, #274: z 5.0, 3.1, 2.5,
    /// 1.9, 1.2, 0.2, -1.4, -3.4, -4.4 at 60 fps, 160 km/h; ours differs by under 0.7 m on the way).
    #[test]
    fn look_back_release_sweeps_round_the_car() {
        let t = CamTune::default();
        let (mut pos, rot) = (Vec3::new(5.0, 1.0, 7.0), Mat3::IDENTITY);
        let mut eye = pos + chase_back_eye();
        let mut z = Vec::new();
        for _ in 0..40 {
            pos += Vec3::Z * 0.73;
            eye = ease_eye(Some(eye), pos, rot, t.chase_eye, &t, 1.0 / 60.0);
            z.push((eye - pos).z);
        }
        assert!((z[0] - 3.12).abs() < 0.1, "{z:?}");
        assert!(z[..9].windows(2).all(|w| w[1] < w[0]) && z[5] < 0.0 && z[8] < -4.4, "{z:?}");
        // at 0.73 m a frame the settled eye trails the target by 0.1 m, as the trace's -4.6
        assert!((z[39] - t.chase_eye.z).abs() < 0.15, "{z:?}");
    }

    /// Rewinding goes back to the state of that moment (needs the install).
    #[test]
    fn rewind_returns_to_the_earlier_state() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../installed_game");
        if !root.exists() {
            return;
        }
        let cars = lineup(1);
        let mut s = Session::new(&root, 1, &cars, Some(cars.len() - 1), Difficulty::Medium).unwrap();
        s.record_timeline();
        let dt = 1.0 / 60.0;
        for _ in 0..1320 {
            s.step(dt, Input::default());
        }
        let (then, clock) = (s.race.racers.iter().map(|r| r.car.pos).collect::<Vec<_>>(), s.race.clock_ms);
        for _ in 0..120 {
            s.step(dt, Input::default());
        }
        assert!(s.rewind(120.0 * dt));
        assert_eq!(then, s.race.racers.iter().map(|r| r.car.pos).collect::<Vec<_>>());
        assert_eq!(clock, s.race.clock_ms);
        // the history ends where the recording began
        assert!(s.rewind(1000.0) && !s.rewind(1.0));
    }

    /// Respawn does nothing during the fly-by and the countdown (#301) and works once the race
    /// has started (needs the install).
    #[test]
    fn respawn_waits_for_the_start() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../installed_game");
        if !root.exists() {
            return;
        }
        let cars = lineup(2);
        let mut s = Session::new(&root, 2, &cars, Some(cars.len() - 1), Difficulty::Medium).unwrap();
        s.record_timeline();
        let dt = 1.0 / 60.0;
        let pos = |s: &Session| s.race.racers[s.focus].car.pos;
        let grid = pos(&s);
        assert!(!s.respawn_focus());
        s.skip_intro();
        s.step(dt, Input::default());
        assert!(!s.race.started() && !s.respawn_focus());
        assert!(pos(&s).distance(grid) < 0.1, "the car stays on its grid slot (it only settles on its springs)");
        assert!(s.timeline.as_ref().unwrap().span().1 > 0.0, "an ignored press keeps the timeline");
        while !s.race.started() {
            s.step(dt, Input::default());
        }
        assert!(s.respawn_focus());
        assert!(pos(&s).distance(grid) > 5.0, "after the start it moves the car");
    }

    /// The timeline reaches past 15 s, keeps the future of a rewind, replays it exactly, and a
    /// respawn in the past takes over (needs the install).
    #[test]
    fn timeline_seeks_replays_and_takes_over() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../installed_game");
        if !root.exists() {
            return;
        }
        let cars = lineup(2);
        let mut s = Session::new(&root, 2, &cars, Some(cars.len() - 1), Difficulty::Medium).unwrap();
        s.record_timeline();
        s.race.autopilot = true;
        let dt = 1.0 / 60.0;
        let state = |s: &Session| (s.race.racers.iter().map(|r| (r.car.pos, r.car.rot)).collect::<Vec<_>>(), s.race.clock_ms);
        for _ in 0..(40.0 / dt) as usize {
            s.step(dt, Input::default());
        }
        let end = state(&s);
        // 30 s back is further than the old 15 s of history
        let moved = s.seek(10.0).unwrap();
        assert!((moved + 30.0).abs() < 0.05, "{moved}");
        assert_ne!(end, state(&s));
        // forward again by seeking, and by playing the recorded frames: both give the same end state
        s.seek(1000.0).unwrap();
        assert_eq!(end, state(&s));
        s.seek(10.0).unwrap();
        for _ in 0..(30.0 / dt) as usize {
            s.step(dt, Input::default());
        }
        assert_eq!(end, state(&s));
        // a respawn at 20 s drops the future
        s.seek(20.0).unwrap();
        s.respawn_focus();
        s.step(dt, Input::default());
        let (start, now, last) = s.timeline.as_ref().unwrap().span();
        assert!(start == 0.0 && (now - last).abs() < 1e-9 && now < 20.1, "{start} {now} {last}");
        // and what was recorded since replays exactly
        let after = state(&s);
        for _ in 0..120 {
            s.step(dt, Input::default());
        }
        let later = state(&s);
        s.seek(now).unwrap();
        assert_eq!(after, state(&s));
        s.seek(1000.0).unwrap();
        assert_eq!(later, state(&s));
    }

    /// A recorded run is a repro file: saved, read back and loaded as a new session it ends in
    /// the same state, also after a Restart and with a respawn in it (needs the install).
    #[test]
    fn run_file_round_trips() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../installed_game");
        if !root.exists() {
            return;
        }
        let cars = lineup(2);
        let player = cars.len() - 1;
        let mut s = Session::new(&root, 2, &cars, Some(player), Difficulty::Medium).unwrap();
        s.setup = Some(Header {
            track: 2,
            laps: s.race.laps,
            catchup: true,
            rules: Rules::Normal,
            manual: false,
            row: AiRow::Difficulty(Difficulty::Medium),
            player: Some(player),
            cars: cars.clone(),
            track_name: "Test".into(),
            nick: String::new(),
            car_name: String::new(),
            drivers: Vec::new(),
            tick_hz: None,
            race_ms: None,
            best_lap_ms: None,
        });
        s.record_timeline();
        // #374: the run on the fixed tick, stepped at 144 fps (the hand-driven test below stays per-frame)
        s.race.set_fixed_tick(Some(60.0));
        let dt = 1.0 / 144.0;
        let state = |s: &Session| (s.race.racers.iter().map(|r| (r.car.pos, r.car.rot)).collect::<Vec<_>>(), s.race.clock_ms, s.race.countdown);
        let drive = |s: &mut Session, secs: f32| {
            s.skip_intro();
            for i in 0..(secs / dt) as usize {
                let mut input = Input { throttle: 1.0, steer: (i as f32 * 0.03).sin() * 0.5, ..Input::default() };
                input.shift_up = i % 97 == 0;
                if i == (secs / dt) as usize / 2 {
                    s.respawn_focus();
                }
                s.step(dt, input);
            }
        };
        let round_trip = |s: &mut Session| {
            let before = state(s);
            let rec = Recording::from_bytes(&s.recording().unwrap().to_bytes()).unwrap();
            assert_eq!(rec.header.tick_hz, Some(60.0), "the file names the tick");
            // saving moves nothing
            assert_eq!(before, state(s));
            let mut t = Session::for_recording(&root, &rec).unwrap();
            t.timeline = Some(crate::timeline::Timeline::from_frames(&mut t.race, &rec.frames));
            assert_eq!(state(&t).1, 0.0, "a loaded run stands at its start");
            t.seek(1e6).unwrap();
            assert_eq!(before, state(&t));
        };
        drive(&mut s, 30.0);
        round_trip(&mut s);
        // a Restart puts everything back and the next run is recorded from there
        s.restart();
        assert_eq!(s.timeline.as_ref().unwrap().span(), (0.0, 0.0, 0.0));
        drive(&mut s, 12.0);
        round_trip(&mut s);
    }

    /// Driving an AI car by hand (#60): the run saved with it replays to the same state, and a
    /// takeover in the past with a hand slot drops the future (needs the install).
    #[test]
    fn hand_driven_run_round_trips() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../installed_game");
        if !root.exists() {
            return;
        }
        let cars = lineup(2);
        let player = cars.len() - 1;
        let mut s = Session::new(&root, 2, &cars, Some(player), Difficulty::Medium).unwrap();
        s.setup = Some(Header {
            track: 2,
            laps: s.race.laps,
            catchup: true,
            rules: Rules::Normal,
            manual: false,
            row: AiRow::Difficulty(Difficulty::Medium),
            player: Some(player),
            cars: cars.clone(),
            track_name: "Test".into(),
            nick: String::new(),
            car_name: String::new(),
            drivers: Vec::new(),
            tick_hz: None,
            race_ms: None,
            best_lap_ms: None,
        });
        s.record_timeline();
        s.skip_intro();
        let dt = 1.0 / 60.0;
        let state = |s: &Session| (s.race.racers.iter().map(|r| (r.car.pos, r.car.rot)).collect::<Vec<_>>(), s.race.clock_ms);
        s.focus = 2;
        assert_eq!(s.hand_slot(), Some(2));
        for i in 0..2400 {
            let input = if i < 900 { Input::default() } else { Input { throttle: 1.0, steer: (i as f32 * 0.02).sin() * 0.6, ..Input::default() } };
            s.step_as(dt, input, s.hand_slot());
        }
        let before = state(&s);
        let rec = Recording::from_bytes(&s.recording().unwrap().to_bytes()).unwrap();
        assert!(rec.frames.iter().all(|f| f.hand == Some(2)));
        let mut t = Session::for_recording(&root, &rec).unwrap();
        t.timeline = Some(crate::timeline::Timeline::from_frames(&mut t.race, &rec.frames));
        t.seek(1e6).unwrap();
        assert_eq!(before, state(&t));
        // a key press in the past takes over and the future is gone
        t.seek(10.0).unwrap();
        t.step_as(dt, Input { throttle: 1.0, ..Input::default() }, Some(2));
        assert!(!t.timeline.as_ref().unwrap().in_past());
        assert!(t.timeline.as_ref().unwrap().span().2 < 12.0);
        // the player's own car is the human slot: no hand slot
        t.focus = player;
        assert_eq!(t.hand_slot(), None);
    }

    /// The grass trail's footprints stay within `TRAIL_RELAX` of the sim clock, in time order,
    /// and a rewind takes the clock and the footprints back (needs the install).
    #[test]
    fn trail_footprints_follow_the_sim_clock() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../installed_game");
        if !root.exists() {
            return;
        }
        let cars = lineup(2);
        let mut s = Session::new(&root, 2, &cars, Some(cars.len() - 1), Difficulty::Medium).unwrap();
        s.record_timeline();
        s.race.autopilot = true;
        let dt = 1.0 / 60.0;
        let check = |s: &Session| {
            let now = s.sim_secs;
            assert!(s.trail.iter().all(|t| t.time >= now - TRAIL_RELAX.get() && t.time <= now), "footprints outside the relax window");
            assert!(s.trail.iter().zip(s.trail.iter().skip(1)).all(|(a, b)| a.time <= b.time), "footprints out of order");
        };
        for _ in 0..1200 {
            s.step(dt, Input::default());
        }
        check(&s);
        assert!((s.sim_secs - 20.0).abs() < 1e-3);
        assert!(s.rewind(120.0 * dt));
        assert!((s.sim_secs - 18.0).abs() < 1e-2);
        check(&s);
        for _ in 0..60 {
            s.step(dt, Input::default());
        }
        check(&s);
    }

    /// CHEAT tribute to tron (#320 phase 3): a wall point every LIGHT_WALL_STEP_M (or _S), none older than
    /// LIGHT_WALL_LIFE, and a jump of LIGHT_WALL_BREAK starts the wall again
    #[test]
    fn light_wall_points_space_age_and_break() {
        let pt = |x: f32, t: f32| WallPt { pos: Vec3::new(x, 0.0, 0.0), up: Vec3::Y, t };
        let mut ring = std::collections::VecDeque::new();
        // 0.1 m and 1 ms apart: one point in five
        for i in 0..50 {
            push_wall_point(&mut ring, pt(i as f32 * 0.1, i as f32 * 0.001));
        }
        assert_eq!(ring.len(), 10);
        assert!(ring.iter().zip(ring.iter().skip(1)).all(|(a, b)| b.pos.x - a.pos.x >= LIGHT_WALL_STEP_M.get() - 1e-4));
        // standing still: a point every LIGHT_WALL_STEP_S, and the old ones go
        ring.clear();
        for i in 0..400 {
            push_wall_point(&mut ring, pt(0.0, i as f32 * 0.01));
        }
        let now = 399.0 * 0.01;
        assert!(ring.iter().all(|p| now - p.t <= LIGHT_WALL_LIFE.get()));
        assert!(ring.iter().zip(ring.iter().skip(1)).all(|(a, b)| b.t - a.t <= LIGHT_WALL_STEP_S.get() + 0.01 + 1e-4));
        assert!(now - ring[0].t > LIGHT_WALL_LIFE.get() - LIGHT_WALL_STEP_S.get() - 0.01);
        // a respawn far away
        push_wall_point(&mut ring, pt(LIGHT_WALL_BREAK.get() + 1.0, now + 0.01));
        assert_eq!(ring.len(), 1);
    }

    /// One quad per pair of points, ground to LIGHT_WALL_HEIGHT along up, uv = (age, 0 / 1), the given colour
    #[test]
    fn light_wall_quads() {
        let pts = [0.0, 1.0, 2.0].map(|x| WallPt { pos: Vec3::new(x, 0.0, 0.0), up: Vec3::Y, t: x });
        let mut out = Vec::new();
        wall_quads(&mut out, pts.iter(), 2.0, LIGHT_WALL_RIVAL);
        assert_eq!(out.len(), 12);
        let top = out.iter().filter(|v| v.uv[1] == 1.0).map(|v| v.pos[1]).fold(f32::MIN, f32::max);
        assert!((top - LIGHT_WALL_HEIGHT.get()).abs() < 1e-5);
        assert!(out.iter().all(|v| v.color[..3] == LIGHT_WALL_RIVAL && v.layer < 0.0));
        // the oldest end (laid at t = 0) is 2 s of the 2.5 s life old, the newest new
        let ages: Vec<f32> = out.iter().map(|v| v.uv[0]).collect();
        assert!(ages.iter().any(|&a| (a - 2.0 / LIGHT_WALL_LIFE.get()).abs() < 1e-5));
        assert!(ages.contains(&0.0));
        // one point: no wall
        out.clear();
        wall_quads(&mut out, pts[..1].iter(), 2.0, LIGHT_WALL_YOU);
        assert!(out.is_empty());
    }
}
