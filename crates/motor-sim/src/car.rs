//! The car model. Section numbers refer to re-notes/03-physics.md.

use crate::body::{world_collide, world_post, CollideBody, Hull};
use crate::props::Pose;
use crate::world::{Material, World};
use glam::{Mat3, Vec3};
use motor_data::cfg::Config;

/// Hard-coded inertia (0x40ced0).
pub(crate) const INERTIA: Vec3 = Vec3::new(3500.0, 2000.0, 1500.0);
/// Design choice: with no wheel down the body levels itself towards world up (rad/s^2 per rad
/// of tilt) and roll/pitch rates keep this share per second, so wall rides and jumps land on
/// the wheels (Goldbridge hairpin: a 45-degree ramp at the foot of the wall, nodes 1300-1400).
const AIR_LEVEL: f32 = 6.0;
const AIR_TILT_DAMP: f32 = 0.1;
/// Design choice: surfaces steeper than this (world normal y; 31 degrees) are walls: the respawn
/// ground search skips them and a hull hit on one counts for the wall drag. Walls on several
/// tracks stand on 30-55 degree chamfers (world ny 0.6-0.85: Goldbridge concrete, Okkun steel);
/// road surfaces have ny >= 0.89 (slopescan example). The wheel rays and the hull response take
/// any face, like the original's.
pub const WHEEL_MIN_NY: f32 = 0.86;
/// Design choice (user report: wall riding cost too little): after this long against a wall
/// (s, contact gaps shorter than `WALL_GRACE` don't count as leaving it) the car loses
/// `WALL_DRAG_RATE` of its horizontal speed per second. Applies to every car.
pub const WALL_DRAG_AFTER: f32 = 1.0;
pub const WALL_DRAG_RATE: f32 = 0.1;
const WALL_GRACE: f32 = 0.25;

/// Our pieces that the original does not have, as switches for the validation against it (#266,
/// `examples/origreplay.rs`). The game always runs the default.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DesignChoices {
    /// the airborne levelling (`AIR_LEVEL`, `AIR_TILT_DAMP`)
    pub air_level: bool,
    /// the wall drag (`WALL_DRAG_AFTER`, `WALL_DRAG_RATE`)
    pub wall_drag: bool,
    /// the inner step the damper is scaled to (`SUSP_DAMP_REF_H3`); None = the original's raw
    /// 65000 per inner step, whatever the frame rate
    pub damper_ref: Option<f32>,
}

impl Default for DesignChoices {
    fn default() -> Self {
        DesignChoices { air_level: true, wall_drag: true, damper_ref: Some(SUSP_DAMP_REF_H3) }
    }
}
/// Steering force reaches full strength above this speed, m/s (P[0xc] = 4.0).
const FULL_STEER_SPEED: f32 = 4.0;
/// Suspension damper constant in normal mode (0x75689c = 65000).
pub const SUSP_DAMP: f32 = 65000.0;
/// The original applies the damper per inner iteration without dividing by the step, so its
/// strength depends on the frame rate. We scale it to the step the game ran at on 1998
/// hardware (~30 fps: one substep of 1/30 s, inner steps of 1/90 s). [APPROX: frame rate]
const SUSP_DAMP_REF_H3: f32 = 1.0 / 90.0;
/// Pitch on acceleration/braking (0x47cdf4): L = (500 - 3 vz) * dvz/dt, clamped +-8000 N.
const PITCH_K0: f32 = 500.0;
const PITCH_KV: f32 = 3.0;
const PITCH_MAX: f32 = 8000.0;
/// Body roll in corners per wheel: torque_z -= sqrt|vz| * vx * k (80, or 10 while sliding).
const ROLL_K: f32 = 80.0;
const ROLL_K_SLIDING: f32 = 10.0;
/// Bumpy surfaces: per-wheel pitch/roll torques, clamped +-2000 (0x4d7ea6).
const BUMP_MAX: f32 = 2000.0;
/// Bump phase wraps at 256*pi (0x4d7ed6).
const BUMP_WRAP: f32 = 804.247_7;
/// Downforce: F_y = -0.05 * m * v_forward (0x40ec14, 0x4cf5bb).
const DOWNFORCE: f32 = -0.05;
/// No throttle force at a standstill: below 0.7 m/s and this much drive force the forward speed
/// decays (0x47dd46).
const CREEP_DRIVE: f32 = 500.0;
/// Q = rpm * 0.001 * taper * SPIN_T[g] (+ |steer| * SPIN_STEER_Q in 1st gear): the engine torque
/// estimate the wheelspin flag compares with the surface limit (0x4ade38, 0x4cf4bb)
const SPIN_T: [f32; 4] = [0.0, 400.0, 350.0, 330.0];
const SPIN_STEER_Q: f32 = 500.0;
/// cap on the torque bracket of the wheelspin law, see `Car::curve` [APPROX]
const SPIN_CURVE_CAP: f32 = 20.0;
/// The model origin sits this far above the physics origin along body up: world_step lowers every
/// car's matrix by 0.32 for the substeps and raises it again after them (0x481014 / 0x481361).
pub const MODEL_OFFSET: f32 = 0.32;
/// Physics substep length and count (0x4814ac).
pub const MAX_STEP: f32 = 0.04;
const MAX_STEPS: usize = 10;

/// The substep lengths of a frame of `dt` seconds (0x4814ac).
pub fn substeps(dt: f32) -> impl Iterator<Item = f32> {
    let mut remaining = dt.clamp(1e-5, 1.0);
    (0..MAX_STEPS).map_while(move |_| {
        let h = (remaining > 0.0).then(|| remaining.min(MAX_STEP));
        remaining -= MAX_STEP;
        h
    })
}

/// Grip scale at GripLevel 10 / 0 (0x4ade18, 0x4ade28; 03-physics 3.1).
const GRIP_MAX: [f32; 4] = [1.3, 1.3, 0.75, 0.95];
const GRIP_MIN: [f32; 4] = [0.75, 0.75, 1.3, 1.1];

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Input {
    /// 0..1
    pub throttle: f32,
    /// 0..1
    pub brake: f32,
    /// -1 (left) .. +1 (right)
    pub steer: f32,
    /// shift keys held (`ctrl+4` bits 0x10 / 0x20); used by the manual gearbox only
    pub shift_up: bool,
    pub shift_down: bool,
    /// HandBrake held (`ctrl+0x1c` → `P[0x2b]`, 0x40ec8c)
    pub handbrake: bool,
    /// the grid hold: mode 1 of the car step (0x47cdf4, 03-physics 3.4 "Arguments"), passed while
    /// the race clock has not started. Tyre forces are off and all horizontal speed is removed
    /// each substep; the springs still settle.
    pub hold: bool,
}

/// Digital (keyboard) steering as the game builds it each frame (0x45bf34 / 0x45bfd6):
/// a fresh press gives half lock; holding ramps by 0.6/s towards full lock; release is 0.
#[derive(Debug, Clone, Copy, Default)]
pub struct SteerKeys {
    value: f32,
}

impl SteerKeys {
    const HALF: f32 = 0.5;
    const HELD: f32 = 0.49;
    const RAMP: f32 = 0.6;

    pub fn update(&mut self, left: bool, right: bool, dt: f32) -> f32 {
        let prev = self.value;
        self.value = match (left, right) {
            (false, true) if prev > Self::HELD => (prev + dt * Self::RAMP).min(1.0),
            (false, true) => Self::HALF,
            (true, false) if prev < -Self::HELD => (prev - dt * Self::RAMP).max(-1.0),
            (true, false) => -Self::HALF,
            _ => 0.0,
        };
        self.value
    }
}

/// Physics parameters derived from a `Game/carNN.car` file (§2.1, §3.1).
#[derive(Debug, Clone)]
pub struct CarParams {
    pub name: String,
    pub mass: f32,
    /// km/h top speed per gear; index 0 = reverse
    pub gear_top: Vec<f32>,
    pub min_rpm: f32,
    pub max_rpm: f32,
    pub acc_force: f32,
    pub brake_force: f32,
    pub turn_force: f32,
    pub com: Vec3,
    /// FL, FR, BR, BL hub positions
    pub wheels: [Vec3; 4],
    /// FL, FR, BR, BL hull corners
    pub corners: [Vec3; 4],
    pub spring_len: f32,
    pub spring_k: f32,
    /// the suspension damper `SUSP` (`0x75689c`): `SUSP_DAMP` = 65000, 21666 for the demon / grem cheat (#313), -50000 for g-ride / west (#314)
    pub damper: f32,
    /// grip[0..3]: slide-speed scale, slide-yaw scale, lateral friction scale, yaw friction scale
    pub grip: [f32; 4],
    /// the car's `.COL` (CollisionName): a convex hull, for contacts with props (04 §4.9.1).
    /// `from_config_levels` leaves a box over the corners; `Race` loads the file.
    pub hull: Hull,
    /// CHEAT gulliver: the size the geometry above is at (1.0 = as loaded; `scaled`).
    pub scale: f32,
    /// CHEAT gulliver: how quickly the body turns, as the car size it feels like relative to
    /// `scale` (1.0 = the full-size response; `inertia`).
    pub feel: f32,
}

fn v3(c: &Config, k: &str) -> Result<Vec3, String> {
    let e = c.get(k).ok_or(format!("car file: no {k}"))?;
    Ok(Vec3::new(e.f32(0).unwrap_or(0.0), e.f32(1).unwrap_or(0.0), e.f32(2).unwrap_or(0.0)))
}

fn f(c: &Config, k: &str) -> Result<f32, String> {
    c.get(k).and_then(|e| e.f32(0)).ok_or(format!("car file: no {k}"))
}

impl CarParams {
    /// Height of the model origin above the physics origin (`Car::pos`), along body up:
    /// `MODEL_OFFSET` [O 0x481014]. (It used to solve the resting spring, 0.39 m for car01.)
    pub fn ride_height(&self) -> f32 {
        MODEL_OFFSET * self.scale
    }

    /// Bounding radius for the contact early-out (`body::CAR_RADIUS`, times `scale`).
    pub fn radius(&self) -> f32 {
        crate::body::CAR_RADIUS * self.scale
    }

    /// The body's inertia: `INERTIA`, times `scale * feel` for a scaled car.
    pub fn inertia(&self) -> Vec3 {
        INERTIA * (self.scale * self.feel)
    }

    /// CHEAT gulliver (ours, 08 §8.14): the car's geometry at size `s` (from the current
    /// `scale`, so `scaled(1.0)` gives the loaded values back): centre of mass, hubs, hull
    /// corners and planes, spring length. The spring rate goes up by as much as the length goes
    /// down, so the spring force at a share of its travel and the resting share (`rest_ext`) stay;
    /// mass, forces, gears and grip stay. Design choice: the inertia (`inertia`) scales with the
    /// levers (×s keeps the full-size angular response) and by `feel` on top, so the small car
    /// turns, rolls and pitches like a car of size `feel` (1/feel times quicker).
    pub fn scaled(&self, s: f32, feel: f32) -> CarParams {
        let k = s / self.scale;
        let mut p = self.clone();
        p.com *= k;
        p.wheels = self.wheels.map(|w| w * k);
        p.corners = self.corners.map(|c| c * k);
        p.spring_len *= k;
        p.spring_k /= k;
        p.hull = self.hull.scaled(k);
        (p.scale, p.feel) = (s, feel);
        p
    }

    /// CHEAT demon / grem (`0x40ced0`, #313): the spring length times `len`, the rate times `rate`
    /// and the damper set to `damper` (a ratio of the current values, so it composes with `scaled`).
    pub fn with_suspension(&self, len: f32, rate: f32, damper: f32) -> CarParams {
        let mut p = self.clone();
        p.spring_len *= len;
        p.spring_k *= rate;
        p.damper = damper;
        p
    }

    /// Spring extension (0 compressed .. 1 free) of a wheel carrying a quarter of the weight on
    /// flat ground [D]: the spring law of `substep`, `len * k * c * (c + 0.1) = m * g / 4`,
    /// solved for the compression `c`.
    pub fn rest_ext(&self, gravity: f32) -> f32 {
        let load = self.mass * gravity / 4.0;
        let q = load / (self.spring_len * self.spring_k);
        let c = (-0.1 + (0.01 + 4.0 * q).sqrt()) / 2.0;
        1.0 - c.clamp(0.0, 1.0)
    }

    /// Height of flat ground above the physics origin (`Car::pos`) when the car rests on it [D]:
    /// the mean hub height less the spring at `rest_ext`. The model origin is `MODEL_OFFSET` up,
    /// so the body sits `rest_ground - MODEL_OFFSET` below the ground (Titan 6.4 cm, R-Build 2
    /// 1.4 cm): the tyre bottoms stand that far above the model's y = 0 (drawing and the
    /// garage car use it, the physics does not).
    pub fn rest_ground(&self, gravity: f32) -> f32 {
        let hub_y = self.wheels.iter().map(|w| w.y).sum::<f32>() / 4.0;
        hub_y - self.spring_len * self.rest_ext(gravity)
    }

    pub fn from_config(c: &Config) -> Result<CarParams, String> {
        Self::from_config_levels(c, Self::levels(c))
    }

    /// (SpeedLevel, AccLevel, GripLevel) as written in the car file (clamped to 0..10).
    pub fn levels(c: &Config) -> (f32, f32, f32) {
        let level = |k: &str| c.get(k).and_then(|e| e.i64(0)).unwrap_or(5).clamp(0, 10) as f32;
        (level("SpeedLevel"), level("AccLevel"), level("GripLevel"))
    }

    /// AI cars (0x40ccc0): blend each level towards the difficulty vector `a`, fixed turn
    /// force 14000, blended brake coefficient. Levels above 10 become 5 (original quirk).
    pub fn for_ai(c: &Config, a: &[f32; 11]) -> Result<CarParams, String> {
        let (sl, al, gl) = Self::levels(c);
        let blend = |lv: f32, target: f32, k: f32| {
            let v = lv + (target.round() - lv) * k;
            if v > 10.0 { 5.0 } else { v }
        };
        let mut p = Self::from_config_levels(c, (blend(sl, a[0], a[1]), blend(al, a[2], a[3]), blend(gl, a[4], a[5])))?;
        p.turn_force = 14000.0;
        p.brake_force += (a[6] - p.brake_force) * a[7];
        Ok(p)
    }

    pub fn from_config_levels(c: &Config, (speed_l, acc_l, grip_l): (f32, f32, f32)) -> Result<CarParams, String> {
        let gears = c.get("Gears").and_then(|e| e.i64(0)).unwrap_or(5) as usize;
        let ratios: Vec<f32> = c.get("PhyGearRatio").map(|e| e.values.iter().filter_map(|v| v.parse().ok()).collect()).unwrap_or_default();
        if ratios.len() < gears + 1 {
            return Err("car file: PhyGearRatio shorter than Gears+1".into());
        }
        // §3.1: forward gears scaled by SpeedLevel; reverse unscaled
        let scale = (speed_l - 5.0) * 0.05 + 1.05;
        let gear_top = (0..=gears).map(|i| if i == 0 { ratios[0] } else { ratios[i] * scale }).collect();
        let mut grip = [0.0; 4];
        for k in 0..4 {
            grip[k] = GRIP_MIN[k] + 0.1 * grip_l * (GRIP_MAX[k] - GRIP_MIN[k]);
        }
        let corners = [v3(c, "PhyCornerFL")?, v3(c, "PhyCornerFR")?, v3(c, "PhyCornerBR")?, v3(c, "PhyCornerBL")?];
        let (lo, hi) = corners.iter().fold((Vec3::splat(f32::MAX), Vec3::splat(f32::MIN)), |(lo, hi), &p| (lo.min(p), hi.max(p)));
        Ok(CarParams {
            name: c.get("CarName").map(|e| e.values.join(" ")).unwrap_or_default(),
            mass: f(c, "PhyWeight")?,
            gear_top,
            min_rpm: f(c, "PhyMinRPM")?,
            max_rpm: f(c, "PhyMaxRPM")?,
            acc_force: f(c, "PhyAccForce")? + (acc_l - 5.0) * 250.0,
            brake_force: f(c, "PhyBrakeForce")?,
            turn_force: f(c, "PhyTurnForce")?,
            com: v3(c, "PhyCenterMass")?,
            wheels: [v3(c, "PhyWheelFL")?, v3(c, "PhyWheelFR")?, v3(c, "PhyWheelBR")?, v3(c, "PhyWheelBL")?],
            corners,
            spring_len: f(c, "PhySpringLength")?,
            spring_k: f(c, "PhySpringStrength")?,
            damper: SUSP_DAMP,
            grip,
            hull: Hull::from_box(lo, hi + Vec3::Y),
            scale: 1.0,
            feel: 1.0,
        })
    }
}

/// Live state of one car.
#[derive(Debug, Clone)]
pub struct Car {
    pub p: CarParams,
    /// world position of the body origin (the model's origin)
    pub pos: Vec3,
    /// body axes in world space (columns: right, up, forward)
    pub rot: Mat3,
    /// linear velocity, body frame (x right, y up, z forward)
    pub vel: Vec3,
    /// angular velocity, body frame
    pub ang: Vec3,
    /// 0 = reverse, 1..n
    pub gear: usize,
    /// manual gearbox (P[0xe]); shifts with `Input::shift_up` / `shift_down`
    pub manual: bool,
    /// a shift key went down and the shift waits for its release (latches P[0x16]/P[0x17])
    shift_pending: [bool; 2],
    pub rpm: f32,
    /// rear tyres spinning P[0x25] (03-physics 3.2): the power-slide steering, 0.8 tyre scale, engine-speed wheels
    pub wheelspin: bool,
    /// clutch in P[0x27]: the engine free-revs (airborne, grid hold, shift key held)
    pub clutch: bool,
    /// wheelspin threshold of the surface P[0x35]: mean of the rear wheels' Spin/NoSpinWheelLimit
    pub spin_limit: f32,
    pub sliding: bool,
    pub wheel_contact: [bool; 4],
    /// spring extension per wheel, 0 = fully compressed .. 1 = free (P+0x158)
    pub wheel_ext: [f32; 4],
    /// material index under each wheel (None = no contact)
    pub wheel_mat: [Option<usize>; 4],
    /// ground point and normal y under each wheel, from the same query as `wheel_mat` (telemetry for
    /// the trace audit, #279; the original's COL hit record `P+0x1a8+0x44i`, +0x30 / +0x20)
    pub wheel_ground: [Option<(Vec3, f32)>; 4],
    /// largest closing speed (m/s) removed by hull collisions during the last frame
    pub impact: f32,
    /// strongest hit against another body (prop or car) during the last frame: rb+0x08 /
    /// rb+0x0c / rb+0x734, which the car sound update 0x483d4c turns into a hit sound
    pub bump: Option<Bump>,
    /// the frame's two world-hit slots (0x47f5e8 step 8): A = the strongest impulse (rb+0x730/
    /// 0x73c/0x74c/0x75c), B = the largest metric (rb+0x740/0x750/0x760). Slot 0 feeds the hit
    /// sounds (0x483d4c), both feed the sparks (0x4464fa)
    pub world_hits: [Option<WorldHit>; 2],
    /// the material of the latest hit (rb+0x72c)
    pub hit_material: Option<usize>,
    /// a hull point touched the world during the last frame (scraping)
    pub scraping: bool,
    /// a hull point touched a wall (a surface too steep for the wheels) during the last frame
    pub on_wall: bool,
    /// time spent against walls (s); `WALL_GRACE` gaps keep it, longer ones reset it
    pub wall_time: f32,
    wall_gap: f32,
    pub steer_smooth: f32,
    /// `vel` and `ang` at the start of the previous substep (P+0x190 / P+0x19c): the extra half
    /// step of 0x47cdf4 step 0 and the pitch term use them
    prev_vel: Vec3,
    prev_ang: Vec3,
    /// 1/ny^3 of each wheel's last hit (local [170+4i] of 0x47cdf4): the weight of the lateral
    /// friction sums (3.5)
    wheel_w: [f32; 4],
    /// slide amount P[0x33] (skid sound and marks; eases the turn reduction while sliding)
    pub slide: f32,
    /// locked wheels P[0x28] (c+0xa0): the applied brake is at least 0.9 of the brake force
    /// (0x47d9d9); the skid-mark code reads it (0x4467eb, 08-missing-features 8.6)
    pub brake_locked: bool,
    /// the driving pedal P[0x3c] (c+0xf0): the throttle, or the brake in reverse with the automatic
    /// gearbox (0x40ecb6 / 0x40f009); the engine sound's on/off levels read it (07-audio 7.2)
    pub pedal: f32,
    /// overrun P[0x2c] (c+0xb0): the engine brake ran above 0.7 MaxRPM in the last engine step
    /// (0x40e175, 0x40e246, 0x40eb49; cleared 0x40dcf5); plays the exhaust burst (07-audio 7.2)
    pub overrun: bool,
    /// front / rear wheel spin rate P[0xe4] / P[0xe8] (0x47dcd7; 2 vz, or the engine speed when
    /// the rear wheels are free), rad/s: 0x40ca90 turns the wheel meshes by them
    pub wheel_spin: [f32; 2],
    /// tyre effect of the rear wheels BR, BL: (ContactAnimID of the surface, strength 0..1),
    /// P[0x170+4r] / P[0x180+4r] for the smoke spawner (0x445f84, 08-missing-features)
    pub tyre_effect: [Option<(u32, f32)>; 2],
    /// driver type P[0x23] != 0 [I: AI]: the hull keeps its 4 points with fewer than 3 wheels down
    /// (0x47d7c0); a human's gets the 3 roof points
    pub ai_driver: bool,
    /// bump phase (rb+0x18)
    bump_phase: f32,
    /// the hull points the world step tests (rb+0x284 / rb+0x288), body space: 4, or 7 with the roof
    /// points when fewer than 3 wheels touch (set at the end of the car step, 0x47d7c0)
    hull: Vec<Vec3>,
    /// pose at the start of the substep (rb+0x70): where the hull points' segments begin
    start: Pose,
    /// this substep's hit record per hull point (rb+0x508): world normal and penetration
    records: Vec<Option<(Vec3, f32)>>,
    /// the hull contacts of the last substep (rb+0x2e8 records with the hit flag): the planes
    /// that cut the separation of car-car and car-prop contacts (0x480400)
    pub hull_hits: Vec<(Vec3, f32)>,
    /// our non-original pieces, on by default (validation switches)
    pub design: DesignChoices,
}

/// A hit against another body in a frame (rb+0x0c, rb+0x734 of the car's body).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Bump {
    /// the other body's id (rb+0x14): a prop's `ObjectColSound` number, 0 for a car [I]
    pub id: u16,
    /// the strongest `-2 * closing speed` (m/s) of the frame
    pub impact: f32,
}

/// A hull hit against the world kept in a slot of the frame (0x47f5e8 step 8, copied to car+0xd0,
/// +0x168, +0x178, +0x188 at 0x4813c7). Slot B keeps no impulse in the original (ours carries it, unread).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WorldHit {
    /// |j|, the impulse of the hit
    pub impulse: f32,
    /// the surface material
    pub material: usize,
    /// |n.x| * vz after the impulse: how much the car rubs along the surface
    pub metric: f32,
    /// the hull point's index (0..3 are the corners FL, FR, BR, BL)
    pub point: usize,
}

impl WorldHit {
    /// The rub strength car+0x178+4i, `min(metric * 0.03, 1)` (0x48141a); negative when moving
    /// backwards, which clears the spark id.
    pub fn strength(&self) -> f32 {
        (self.metric * 0.03).min(1.0)
    }

    /// car+0x168+4i: the material's SparkAnimID, 0 when the strength is negative (0x481442).
    /// Materials beyond the table read as material 0 (0x4813f6).
    pub fn spark_anim(&self, materials: &[Material]) -> u32 {
        if self.strength() < 0.0 {
            return 0;
        }
        materials.get(self.material).or(materials.first()).map_or(0, |m| m.spark_anim)
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct Telemetry {
    pub kmh: f32,
    pub gear: usize,
    pub rpm: f32,
    pub drive_force: f32,
    pub contacts: usize,
    pub sliding: bool,
    pub wheelspin: bool,
}

impl Car {
    pub fn new(p: CarParams, pos: Vec3, forward: Vec3) -> Car {
        let mut c = Car {
            p,
            pos,
            rot: Mat3::IDENTITY,
            vel: Vec3::ZERO,
            ang: Vec3::ZERO,
            gear: 1,
            manual: false,
            shift_pending: [false; 2],
            rpm: 0.0,
            wheelspin: false,
            clutch: false,
            spin_limit: 0.0,
            sliding: false,
            wheel_contact: [false; 4],
            wheel_ext: [1.0; 4],
            wheel_mat: [None; 4],
            wheel_ground: [None; 4],
            impact: 0.0,
            bump: None,
            world_hits: [None; 2],
            hit_material: None,
            scraping: false,
            on_wall: false,
            wall_time: 0.0,
            wall_gap: 0.0,
            steer_smooth: 0.0,
            prev_vel: Vec3::ZERO,
            prev_ang: Vec3::ZERO,
            wheel_w: [1.0; 4],
            slide: 0.0,
            brake_locked: false,
            pedal: 0.0,
            overrun: false,
            wheel_spin: [0.0; 2],
            tyre_effect: [None; 2],
            ai_driver: false,
            bump_phase: 0.0,
            hull: Vec::new(),
            start: Pose { pos, rot: Mat3::IDENTITY, scale: Vec3::ONE },
            records: Vec::new(),
            hull_hits: Vec::new(),
            design: DesignChoices::default(),
        };
        c.place(pos, forward);
        c.hull = c.hull_points();
        c.records = vec![None; c.hull.len()];
        c
    }

    /// CHEAT gulliver: switch to `self.p.scaled(s, feel)` where the car stands; the body moves
    /// along its up axis by the change of the resting ground height, so the tyres keep to the
    /// road (a car growing next to a wall can end up in it: the stuck reset takes it out).
    pub fn rescale(&mut self, s: f32, feel: f32, gravity: f32) {
        let p = self.p.scaled(s, feel);
        self.reparam(p, gravity);
    }

    /// CHEAT demon / grem (#313): switch to `self.p.with_suspension(len, rate, damper)` where the car stands.
    pub fn resuspend(&mut self, len: f32, rate: f32, damper: f32, gravity: f32) {
        let p = self.p.with_suspension(len, rate, damper);
        self.reparam(p, gravity);
    }

    /// Take new parameters where the car stands: the body moves along its up axis by the change of
    /// the resting ground height; the hull and the stuck-reset pose follow.
    fn reparam(&mut self, p: CarParams, gravity: f32) {
        self.pos += self.rot.y_axis * (self.p.rest_ground(gravity) - p.rest_ground(gravity));
        self.p = p;
        self.hull = self.hull_points();
        self.records = vec![None; self.hull.len()];
        self.start = Pose { pos: self.pos, rot: self.rot, scale: Vec3::ONE };
    }

    /// Put the car at `pos` (body origin) facing `forward` (projected flat), at rest.
    pub fn place(&mut self, pos: Vec3, forward: Vec3) {
        let fwd = Vec3::new(forward.x, 0.0, forward.z).normalize_or(Vec3::Z);
        // left-handed axes: right = up x forward
        let right = Vec3::Y.cross(fwd);
        self.rot = Mat3::from_cols(right, Vec3::Y, fwd);
        self.pos = pos;
        self.vel = Vec3::ZERO;
        self.ang = Vec3::ZERO;
        self.gear = 1;
        self.shift_pending = [false; 2];
        self.rpm = self.p.min_rpm;
        (self.wheelspin, self.clutch, self.spin_limit) = (false, false, 0.0);
        self.wheel_ext = [1.0; 4];
        // the original never resets its previous-step velocities; a placed car starts from rest
        (self.prev_vel, self.prev_ang) = (Vec3::ZERO, Vec3::ZERO);
        (self.slide, self.wheel_w, self.brake_locked) = (0.0, [1.0; 4], false);
        (self.pedal, self.overrun) = (0.0, false);
        (self.wheel_spin, self.tyre_effect) = ([0.0; 2], [None; 2]);
        (self.wall_time, self.wall_gap) = (0.0, 0.0);
        self.hull_hits.clear();
        self.records.iter_mut().for_each(|r| *r = None);
    }

    /// Set the dynamic state to a recorded one (`origreplay`, #266): body origin, orientation,
    /// body-frame velocities, and `prev` = the velocities at the start of the previous substep
    /// (P+0x190 / P+0x19c) that the half step of step 0 uses (`None`: the current ones, as if the
    /// car had been moving evenly). The engine and gearbox are the caller's.
    pub fn set_state(&mut self, pos: Vec3, rot: Mat3, vel: Vec3, ang: Vec3, prev: Option<(Vec3, Vec3)>) {
        (self.pos, self.rot, self.vel, self.ang) = (pos, rot, vel, ang);
        (self.prev_vel, self.prev_ang) = prev.unwrap_or((vel, ang));
        self.start = Pose { pos, rot, scale: Vec3::ONE };
    }

    /// `vel` and `ang` at the start of the previous substep (P+0x190 / P+0x19c).
    pub fn prev_state(&self) -> (Vec3, Vec3) {
        (self.prev_vel, self.prev_ang)
    }

    pub fn kmh(&self) -> f32 {
        self.vel.z * 3.6
    }

    /// Linear velocity in world space (`vel` is in the body frame).
    pub fn world_vel(&self) -> Vec3 {
        self.rot * self.vel
    }

    /// Advance by one rendered frame; splits into <= 10 substeps of <= 0.04 s (0x4814ac).
    pub fn step_frame(&mut self, dt: f32, input: Input, world: &World) -> Telemetry {
        let mut t = Telemetry::default();
        self.begin_frame();
        for h in substeps(dt) {
            t = self.substep(h, input, world);
            self.collide_world(h, world, 3);
        }
        self.end_frame(dt);
        t
    }

    /// Reset the per-frame contact flags. `Race` drives the substeps itself so that props can
    /// act between them (`step_frame` is `begin_frame`, `substep`s, `end_frame`).
    pub fn begin_frame(&mut self) {
        self.impact = 0.0;
        self.bump = None;
        self.world_hits = [None; 2];
        self.hit_material = None;
        self.scraping = false;
        self.on_wall = false;
    }

    pub fn end_frame(&mut self, dt: f32) {
        self.wall_drag(dt.clamp(0.0, 1.0));
    }

    /// Slow a car that keeps riding a wall (see `WALL_DRAG_AFTER`).
    fn wall_drag(&mut self, dt: f32) {
        if self.on_wall {
            self.wall_time += dt;
            self.wall_gap = 0.0;
        } else {
            self.wall_gap += dt;
            if self.wall_gap > WALL_GRACE {
                self.wall_time = 0.0;
            }
        }
        if self.design.wall_drag && self.wall_time > WALL_DRAG_AFTER {
            let k = (1.0 - WALL_DRAG_RATE).powf(dt);
            (self.vel.x, self.vel.z) = (self.vel.x * k, self.vel.z * k);
        }
    }

    /// §3.2 "Traction-limit flag" and "Wheelspin flag" (0x40dc1d..0x40dce8), the head of both
    /// gearbox paths. `thr` is the clamped throttle before the locked-wheel cut. Updates
    /// `wheelspin` (P[0x25]) and returns (P[0x26], Q, g): the traction-limit flag, the engine
    /// torque estimate it compares with `spin_limit`, and the gear after a pending shift key.
    // CLASSIC 0x40db88
    fn traction(&mut self, thr: f32, steer: f32) -> (bool, f32, usize) {
        let gears = self.p.gear_top.len() - 1;
        let mut g = self.gear;
        if self.shift_pending[0] && g < gears {
            g += 1;
        }
        if self.shift_pending[1] {
            g = if self.gear != 0 { g - 1 } else { 1 };
        }
        let mut q0 = SPIN_T.get(g).copied().unwrap_or(0.0);
        if self.gear == 1 {
            q0 += steer.abs() * SPIN_STEER_Q;
        }
        let top = self.p.gear_top[self.gear];
        let kmh = self.vel.z * 3.6;
        let taper = if kmh >= 0.3 * top { (top - kmh) / (0.7 * top) } else { 1.0 };
        let q = self.rpm * 0.001 * taper * q0;
        let spin = q > self.spin_limit && thr > 0.1 && g > 0;
        // AI cars never spin in Single/League races (0x74efc0 is set there, P[0x23] == 2) [D]
        let spin = spin && !self.ai_driver;
        self.wheelspin = spin && !self.clutch;
        (spin, q, g)
    }

    /// Drive force and the new rpm candidate for a pedal position: the speed-bound torque curve,
    /// or the wheelspin law when `spin` carries Q (P[0x26], 0x40dd9c..0x40df0c / 0x40e65c..0x40e7c6).
    /// `g` is the gear after a pending shift key (it limits the curve's x in the human path).
    // CLASSIC 0x40db88
    fn curve(&self, spin: Option<f32>, g: usize, pedal: f32, acc: f32, h: f32) -> (f32, f32) {
        let p = &self.p;
        let kmh = (self.vel.z * 3.6).abs();
        let top = p.gear_top[self.gear];
        let (mut f, cand) = match spin {
            Some(q) => {
                // the revs sag by 0.625^h and gain little; the old rpm, unclamped, feeds the curve and
                // the drive is scaled by 0.7 P35 / (0.0005 Q^2) (03-physics 3.2)
                let cand = (0.625f32.powf(h) * (self.rpm + h * 27.5 * pedal * (80.0 + 0.02 * (q - self.spin_limit)))).min(p.max_rpm);
                let x = (self.rpm - p.min_rpm) * 0.001;
                // APPROX: the original's 2 / (MaxRPM * 0.00097 - x) blows up around 10 700 rpm, which a
                // free-revved engine reaches; the bracket is capped (design choice)
                let bracket = (x.max(0.0).sqrt() - 2.0 / (p.max_rpm * 0.00097 - x)).clamp(-SPIN_CURVE_CAP, SPIN_CURVE_CAP);
                (0.7 * self.spin_limit / (0.0005 * q * q) * bracket * pedal * acc, cand)
            }
            None => {
                let cand = (p.min_rpm + (p.max_rpm - p.min_rpm) * kmh / top).max(p.min_rpm + 1.0);
                let x = ((cand - p.min_rpm) * 0.001).min(p.max_rpm * 0.000935 - g as f32 * 0.15);
                ((x.max(0.0).sqrt() - 2.0 / (p.max_rpm * 0.001 - x)) * pedal * acc, cand)
            }
        };
        if kmh < 10.0 {
            f += (10.0 - kmh) * pedal * 300.0; // launch assist
        }
        (if f.is_finite() { f } else { 0.0 }, cand)
    }

    /// The clutch state P[0x27] (0x40e020..0x40e0e6, 0x40ebb7, 0x40ebdb): the rpm candidate is
    /// committed only with no shift key pending, a rear wheel on the ground (the previous car step's
    /// flags) and no grid hold. Otherwise the engine free-revs with the clutch in.
    // CLASSIC 0x40db88
    fn clutch_rpm(&mut self, cand: f32, pedal: f32, h: f32, hold: bool) {
        let pending = self.shift_pending[0] || self.shift_pending[1];
        if !pending && (self.wheel_contact[2] || self.wheel_contact[3]) && !hold {
            self.clutch = false;
            self.rpm = cand;
            return;
        }
        self.clutch = true;
        if pedal > 0.1 {
            let k = ((self.p.max_rpm - self.rpm + 3000.0) * 0.1 * pedal).max(10.0);
            self.rpm += 25.0 * h * k;
        } else {
            self.rpm = (self.rpm - 6000.0 * h).max(self.p.min_rpm);
        }
    }

    /// The engine brake (0x40e162 / 0x40e233 automatic, 0x40eabb manual): `k * rpm / top` against the
    /// direction of travel, from the stored rpm. `k` is 13, the manual path grows it over the top speed.
    // CLASSIC 0x40db88
    fn engine_brake(&mut self, vz: f32, k: f32) -> f32 {
        self.overrun = self.rpm > 0.7 * self.p.max_rpm;
        let f = k * self.rpm / self.p.gear_top[self.gear];
        if vz > 0.0 {
            -f
        } else {
            f
        }
    }

    /// §3.2 engine + automatic gearbox (0x40db88, automatic path). Returns drive force.
    /// The original compares the signed speed `rb+0x50` in m/s with the shift thresholds (2 m/s).
    fn engine(&mut self, inp: Input, h: f32) -> f32 {
        self.overrun = false;
        if self.manual {
            return self.engine_manual(inp, h);
        }
        let (thr, brk) = (inp.throttle.clamp(0.0, 1.0), inp.brake.clamp(0.0, 1.0));
        let (spin, q, g) = self.traction(thr, inp.steer);
        let vz = self.vel.z;
        // locked wheels P[0x28] (0x40dd47 / 0x40e203): the pedal that brakes in this gear is above 0.9
        // (the brake going forward, the throttle in reverse); it cuts the opposite pedal after the
        // traction flag has read it (0x40dd51 / 0x40e20d)
        let locked = if self.gear == 0 { thr } else { brk } > 0.9;
        let (thr, brk) = match (locked, self.gear) {
            (true, 0) => (thr, 0.0),
            (true, _) => (0.0, brk),
            _ => (thr, brk),
        };
        // in reverse the brake pedal drives (negative pedal)
        let pedal = if self.gear == 0 { brk } else { thr };
        let signed = if self.gear == 0 { -pedal } else { pedal };
        let (mut f, cand) = self.curve(spin.then_some(q), g, signed, self.p.acc_force, h);
        let max_rpm = self.p.max_rpm;
        // engine braking, on the stored rpm and not with the clutch in; the automatic path has no
        // over-top-speed variant
        if pedal < 0.15 && !self.clutch {
            f += self.engine_brake(vz, 13.0);
        }
        // automatic shifting runs after the force, on the new rpm candidate (0x40df13..0x40dfd9)
        if vz >= 2.0 {
            if self.gear == 0 {
                self.gear = 1;
            } else if self.gear > 1 && cand < 0.6 * max_rpm {
                self.gear -= 1;
            }
            if cand > 0.912 * max_rpm && self.gear < self.p.gear_top.len() - 1 {
                self.gear += 1;
            }
        }
        if vz <= -2.0 && brk > 0.05 {
            self.gear = 0;
        }
        // standstill; locked wheels in a forward gear skip it (0x40df8b)
        if vz.abs() < 2.0 && !(locked && self.gear != 0) {
            if brk > 0.25 && thr < 0.25 {
                self.gear = 0;
            }
            if brk < 0.25 && thr > 0.25 {
                self.gear = 1;
            }
        }
        self.clutch_rpm(cand, signed, h, inp.hold);
        f
    }

    /// §3.2 engine + manual gearbox (0x40db88, manual path 0x40e5f5..0x40e8fa) [O].
    fn engine_manual(&mut self, inp: Input, h: f32) -> f32 {
        let brk = inp.brake.clamp(0.0, 1.0);
        let (spin, q, g) = self.traction(inp.throttle.clamp(0.0, 1.0), inp.steer);
        // a full brake locks the wheels and cuts the throttle (0x40e62c)
        let thr = if brk > 0.9 { 0.0 } else { inp.throttle.clamp(0.0, 1.0) };
        let vz = self.vel.z;
        let top = self.p.gear_top[self.gear];
        // the manual gearbox gets 10 % more acc force (0x40ced0)
        let (mut f, cand) = self.curve(spin.then_some(q), g, thr, self.p.acc_force * 1.1, h);
        // a held shift key works as the clutch: no drive (0x40e7d2)
        if inp.shift_up || inp.shift_down {
            f = 0.0;
        }
        // the throttle drives in reverse too (negated by the gear after a pending shift, 0x40e7fe)
        if g == 0 {
            f = -f;
        }
        if thr < 0.15 && !self.clutch {
            // 0x40eabb: |vz| in m/s against the gear's top speed in km/h, so the over-top growth never
            // fires at real speeds [O]
            let k = if vz.abs() > top { vz.abs() / top * 13.0 } else { 13.0 };
            f += self.engine_brake(vz, k);
        }
        // service brakes, ramped in below 1 m/s (0x40e831)
        f -= vz.signum() * vz.abs().min(1.0) * brk * self.p.brake_force;
        // shifts happen when the key is released (0x40e88f / 0x40e8c9)
        let gears = self.p.gear_top.len() - 1;
        if inp.shift_down {
            self.shift_pending[1] = true;
        } else if std::mem::take(&mut self.shift_pending[1]) {
            if self.gear == 1 && vz < 5.0 {
                self.gear = 0; // reverse only below 5 m/s
            } else if self.gear > 1 {
                self.gear -= 1;
            }
        }
        if inp.shift_up {
            self.shift_pending[0] = true;
        } else if std::mem::take(&mut self.shift_pending[0]) && self.gear < gears {
            self.gear += 1;
        }
        self.clutch_rpm(cand, thr, h, inp.hold);
        f
    }

    /// One physics substep of `h` seconds: the force setup 0x40ec14 (3.3) and the car step
    /// 0x47cdf4 (3.4, 3.5), in the original's order. `Race` calls it per car and substep.
    // CLASSIC 0x40ec14 + 0x47cdf4
    #[allow(clippy::needless_range_loop)] // per-wheel arrays are indexed in parallel
    pub fn substep(&mut self, h: f32, inp: Input, w: &World) -> Telemetry {
        let p = self.p.clone();
        let m = p.mass;
        // rb+0x70: the hull segments of the world step start from here (0x481026)
        self.start = Pose { pos: self.pos, rot: self.rot, scale: Vec3::ONE };
        // ---- §3.3 forces set up per substep (0x40ec14)
        self.pedal = if !self.manual && self.gear == 0 { inp.brake } else { inp.throttle };
        let drive = self.engine(inp, h);
        let s = inp.steer.clamp(-1.0, 1.0);
        let vz = self.vel.z;
        let mut steer_f = p.turn_force * s;
        if vz.abs() < FULL_STEER_SPEED {
            steer_f *= vz.abs() / FULL_STEER_SPEED;
        }
        if vz < 0.0 {
            steer_f = -steer_f;
        }
        self.steer_smooth = (self.steer_smooth + (s - self.steer_smooth) * 0.2).clamp(-1.0, 1.0);
        // body-frame tyre forces: front wheels steer, rear wheels drive (x2 each, 0x4cf5e3);
        // the grid hold (mode 1) zeroes them (step 1, 0x47dfde)
        let tyre = if inp.hold {
            [Vec3::ZERO; 4]
        } else if self.wheelspin {
            // power slide (0x40ef50): the front gets more steering force, the rear pushes the other
            // way by a share that wobbles with the bump phase, so the tail steps out
            let b = (1.0 + (self.bump_phase * 0.033).sin()) * 0.25;
            let (front, rear) = ((b + 0.5) * steer_f, -(1.0 - b) * steer_f);
            [Vec3::new(front, 0.0, 0.0), Vec3::new(front, 0.0, 0.0), Vec3::new(rear, 0.0, drive * 2.0), Vec3::new(rear, 0.0, drive * 2.0)]
        } else {
            [Vec3::new(steer_f, 0.0, 0.0), Vec3::new(steer_f, 0.0, 0.0), Vec3::new(0.0, 0.0, drive * 2.0), Vec3::new(0.0, 0.0, drive * 2.0)]
        };
        let reverse_brakes = self.gear == 0 && !self.manual;
        let mut applied_brake = if reverse_brakes { inp.throttle } else { inp.brake }.clamp(0.0, 1.0) * p.brake_force;
        // handbrake: at least half the brake force in the grip model's brake factor (0x40dfe0) [O]
        if inp.handbrake && applied_brake < 0.5 * p.brake_force {
            applied_brake += 0.5 * p.brake_force;
        }
        // locked wheels P[0x28]: brake pedal at 0.9 or more
        let locked = applied_brake >= 0.9 * p.brake_force;
        // Gravity and downforce go into the wheels' extra-force slots (0x40ed94..0x40eef4), body
        // frame from the pose at the start of the substep: a quarter of the weight at every hub,
        // the world-vertical downforce split 0.3/0.3/0.2/0.2 FL/FR/BR/BL. Applied at the hubs
        // they make a torque about the centre of mass.
        let to_body = self.rot.transpose();
        let gravity_b = to_body * Vec3::new(0.0, -w.gravity * m, 0.0);
        let downforce_b = to_body * Vec3::new(0.0, DOWNFORCE * m * vz, 0.0);
        let mut extra: [Vec3; 4] = std::array::from_fn(|i| gravity_b * 0.25 + downforce_b * if i < 2 { 0.3 } else { 0.2 });

        // ---- §3.4 step 0 (0x47ce14): the second half of the previous step, an extra half step
        // proportional to the change of v and ω since the previous call
        let (vold, wold) = (self.prev_vel, self.prev_ang);
        (self.prev_vel, self.prev_ang) = (self.vel, self.ang);
        self.rotate_move((self.ang - wold) * (0.5 * h), (self.vel - vold) * (0.5 * h));

        // ---- §3.4 step 2 (0x47d1d5): pre-pass over the wheels, once per call; the contact
        // flags and materials are the previous call's
        let n_on = self.wheel_contact.iter().filter(|&&c| c).count();
        let vz = self.vel.z;
        let mut scale = [1.0f32; 4]; // tyre-force scale s[i]
        let mut torque0 = Vec3::ZERO;
        for i in 0..4 {
            if let Some(mi) = self.wheel_mat[i] {
                let mat = &w.materials[mi];
                let pitch = (vz * 80.0 * (self.bump_phase * 0.75 + vz).sin()).clamp(-BUMP_MAX, BUMP_MAX);
                torque0.x += mat.bumpy * pitch;
                self.bump_phase += h * 7.6;
                let roll = (vz * 140.0 * (self.bump_phase * 0.63 + vz + 0.7).cos()).clamp(-BUMP_MAX, BUMP_MAX);
                torque0.z += mat.bumpy * roll;
                self.bump_phase += h * 8.0;
                // the turn reduction of a sliding car eases with the slide amount (10/slide, at most 1)
                scale[i] *= if self.sliding { mat.spin_turn_reduce * (10.0 / self.slide).clamp(0.0, 1.0) } else { mat.nospin_turn_reduce };
                if self.wheelspin && self.gear > 0 && !self.clutch {
                    // wheelspin (0x47e06a, 0x47e087): less lateral grip, and the rear hubs shake
                    scale[i] *= 0.8;
                    if i >= 2 {
                        let (t, a) = (vz.min(5.0), 0.5 * self.bump_phase);
                        extra[i].x += (0.73 * (0.26 * a + 0.8).sin()).sin() * t * 500.0 + (0.11 * a + 1.6).sin() + (0.06 * a + 4.7).sin();
                    }
                }
                if self.bump_phase > BUMP_WRAP {
                    self.bump_phase -= BUMP_WRAP;
                }
            }
            if n_on == 4 {
                // pitch: braking (L > 0) loads the front, accelerating loads the rear
                let l = ((vold.z - vz) / h * (PITCH_K0 - vz * PITCH_KV)).clamp(-PITCH_MAX, PITCH_MAX);
                extra[i].y += match (l > 0.0, i < 2) {
                    (true, true) => -l,
                    (true, false) => 0.5 * l,
                    (false, true) => -0.5 * l,
                    (false, false) => l,
                };
                // roll in corners, with the sign of -vx for both signs of vz
                let k = if self.sliding { ROLL_K_SLIDING } else { ROLL_K };
                torque0.z -= k * self.vel.x * vz.abs().sqrt();
            }
        }
        // CHEAT gulliver: these torques have no lever, so they shrink with the car like the
        // levered ones (`inertia`)
        torque0 *= p.scale;

        // ---- §3.4 step 5 (0x47e1d2): three inner iterations of h/3, k = 2, 1, 0
        let hs = h / 3.0;
        let damp = self.design.damper_ref.map_or(p.damper, |r| p.damper * r / hs); // APPROX: frame-rate scaling, see SUSP_DAMP_REF_H3
        let spring = Vec3::new(0.0, -p.spring_len, 0.0);
        // the plane of each wheel's latest query hit (the local hit records of the loop)
        let mut planes: [Option<(Vec3, f32, usize)>; 4] = [None; 4];
        for k in (0..3).rev() {
            let s_w = self.rot * spring; // S: wheel 0's spring vector serves all four
            let mut force = Vec3::ZERO;
            let mut torque = torque0;
            for i in 0..4 {
                let mut fi = extra[i];
                let hub_w = self.pos + self.rot * p.wheels[i];
                // the segment runs from PhySpringLength above the hub (t = 0) to as far below it
                // (t = 1); t = 0.5 is the hub and only the lower half counts as contact
                let (a, b) = (hub_w - s_w, hub_w + s_w);
                let hit = if k == 0 && self.wheel_contact[i] {
                    // third iteration, a wheel that touched in the second: re-test its plane (0x416480)
                    planes[i].and_then(|(n, d, mat)| World::plane_hit(n, d, a, b).map(|t| (t, n, mat)))
                } else {
                    w.raycast(a, b).map(|hit| {
                        planes[i] = Some((hit.normal, hit.plane_d, hit.material));
                        (hit.t, hit.normal, hit.material)
                    })
                };
                self.wheel_contact[i] = false;
                self.wheel_mat[i] = None;
                self.wheel_ground[i] = None;
                match hit {
                    None => self.wheel_ext[i] = 1.0,
                    Some((t, ..)) if t <= 0.5 => self.wheel_ext[i] = 0.5, // ground above the hub: no contact
                    Some((t, n, mat)) => {
                        let u = ((t - 0.5) * 2.0).clamp(0.0, 1.0); // 0 = fully compressed, 1 = free
                        let c = (1.0 - u).max(0.0);
                        let uprev = std::mem::replace(&mut self.wheel_ext[i], u);
                        let ny = n.y;
                        let ny3 = ny * ny * ny;
                        self.wheel_w[i] = if ny3 == 0.0 { 10000.0 } else { 1.0 / ny3 };
                        let nb = self.rot.transpose() * n; // contact normal, body frame
                        let tf = Vec3::new(scale[i] * tyre[i].x, tyre[i].y, tyre[i].z); // only x is scaled
                        // tyre force in the contact plane, weighted ny^3 * nb.y^2
                        fi += (tf - nb * tf.dot(nb)) * (ny3 * nb.y * nb.y);
                        // damper (up while compressing) and progressive spring, along body up
                        fi.y += (uprev - u) * damp + p.spring_len * p.spring_k * c * (c + 0.1) * ny * ny;
                        self.wheel_contact[i] = true;
                        self.wheel_mat[i] = Some(mat);
                        self.wheel_ground[i] = Some((a + (b - a) * t, ny));
                    }
                }
                force += fi;
                torque += (p.wheels[i] - p.com).cross(fi); // about the CoM, from the hub
            }
            self.vel += force * (hs / m);
            self.ang += torque / p.inertia() * hs;
            if self.vel.x.abs() < 0.7 && self.vel.z.abs() < 3.0 {
                self.vel.x = 0.0; // stop creep
            }
            self.rotate_move(self.ang * hs, self.vel * hs);
        }

        // ---- §3.5 grip: velocity retention per second (step 6, once per call with the full h)
        self.grip(h, applied_brake, inp.handbrake, inp.hold, w);

        // ---- §3.4 step 7 (0x47dcd7): wheel spin rates, creep stop, tyre effects
        let vz = self.vel.z;
        let (spin_a, spin_b) = if locked { (0.0, 0.0) } else { (2.0 * vz, p.gear_top[self.gear] * 6e-5 * self.rpm) };
        let rear_down = self.wheel_contact[2] || self.wheel_contact[3];
        // a wheelspin car shows the engine speed; so does one with both rear wheels in the air and
        // no shift key pending (0x47dd0f, 0x47ecdc..0x47ed35)
        let rear_spin = if self.wheelspin {
            spin_b
        } else if rear_down || self.shift_pending[0] || self.shift_pending[1] {
            spin_a
        } else if self.gear > 0 {
            spin_b
        } else {
            -spin_b
        };
        self.wheel_spin = [spin_a, rear_spin];
        if vz.abs() < 0.7 && drive.abs() < CREEP_DRIVE {
            self.vel.z = if locked { 0.0 } else { vz * 0.8f32.powf(h) };
        }
        // P[0x35]: the mean over the rear wheels on a material of the limit that applies (0x47dd84..0x47de57)
        let limits: Vec<f32> = (2..4).filter_map(|i| self.wheel_mat[i]).map(|m| if self.wheelspin { w.materials[m].spin_wheel_limit } else { w.materials[m].nospin_wheel_limit }).collect();
        self.spin_limit = limits.iter().sum::<f32>() / limits.len().max(1) as f32;
        for r in 0..2 {
            self.tyre_effect[r] = None;
            let Some(mi) = self.wheel_mat[2 + r] else { continue };
            let kind = w.materials[mi].contact_anim;
            if kind == 0 {
                continue;
            }
            // the later rule wins
            if self.sliding && vz > 3.0 {
                self.tyre_effect[r] = Some((kind, (0.5 * self.slide).min(1.0)));
            }
            if self.wheelspin {
                self.tyre_effect[r] = Some((kind, (0.000_175 * self.rpm).min(1.0)));
            }
            if locked && vz > 3.0 {
                self.tyre_effect[r] = Some((kind, (0.1 * vz).min(1.0)));
            }
        }

        // the hull set for the world step, after the iterations (0x47d7c0), and cleared hit records
        // (0x481067); the passes, the resting pass and the copy follow in `collide_world`
        self.hull = self.hull_points();
        self.records = vec![None; self.hull.len()];

        Telemetry {
            kmh: self.kmh(),
            gear: self.gear,
            rpm: self.rpm,
            drive_force: drive,
            contacts: self.wheel_contact.iter().filter(|&&c| c).count(),
            sliding: self.sliding,
            wheelspin: self.wheelspin,
        }
    }

    /// Turn the body about its own axes, X then Y then Z (0x42bf84 / 0x42c030 / 0x42c0d8), move
    /// it along the turned axes (0x42bf0c..0x42bf5c) and re-express the body-frame velocity so
    /// the world velocity is unchanged (0x47cdf4 builds the same rotation from identity, "v = R v")
    /// [D: the sign convention of the original's rotations was not traced; this keeps the world
    /// velocity, the only reading under which the cars are drivable]. No orthonormalisation
    /// here: the original does that once per substep.
    fn rotate_move(&mut self, ang: Vec3, lin: Vec3) {
        let r = Mat3::from_rotation_x(ang.x) * Mat3::from_rotation_y(ang.y) * Mat3::from_rotation_z(ang.z);
        self.rot *= r;
        self.pos += self.rot * lin;
        self.vel = r.transpose() * self.vel;
    }

    /// §3.5 (0x47d68a..0x47dcd7). `hold` is the grid hold (mode 1).
    fn grip(&mut self, h: f32, applied_brake: f32, handbrake: bool, hold: bool, w: &World) {
        // the wheels with a material after the last iteration, with their 1/ny^3 weights
        let on: Vec<(&crate::world::Material, f32)> = (0..4).filter_map(|i| self.wheel_mat[i].map(|m| (&w.materials[m], self.wheel_w[i]))).collect();
        let n = on.len() as f32;
        if on.is_empty() {
            // airborne (0x47ebb3)
            self.sliding = false;
            let kv = 0.99f32.powf(h);
            self.vel.x *= kv;
            self.vel.z *= kv;
            self.ang *= 0.95f32.powf(h);
            if !self.design.air_level {
                return;
            }
            // design choice: level towards world up (AIR_LEVEL, AIR_TILT_DAMP)
            let up = self.rot.y_axis;
            let axis = self.rot.transpose() * up.cross(Vec3::Y);
            let tilt = up.angle_between(Vec3::Y);
            let lev = if axis.length_squared() > 1e-8 { axis.normalize() * tilt * AIR_LEVEL * h } else { Vec3::ZERO };
            let kt = AIR_TILT_DAMP.powf(h);
            self.ang.x = self.ang.x * kt + lev.x;
            self.ang.z = self.ang.z * kt + lev.z;
            return;
        }
        let avg = |f: fn(&crate::world::Material) -> f32| on.iter().map(|(m, _)| f(m)).sum::<f32>() / n;
        let avg_w = |f: fn(&crate::world::Material) -> f32| on.iter().map(|(m, wi)| f(m) * wi).sum::<f32>() / n;
        let g = self.p.grip;
        let sl = avg(|m| m.spin_speed_limit) * g[0];
        let rl = avg(|m| m.spin_rot_limit) * g[1];
        // brake factor (0x47d8ee); the grid hold forces it to 0, so all horizontal speed goes (0x47d9d9)
        let bf = self.p.brake_force;
        let q = (1.1 / bf - 1.0) / 6.0;
        let vzs = self.vel.z.abs();
        let taper = if vzs >= 6.0 { 1.0 } else { (6.0 * q + 1.0) - q * vzs };
        let a = if hold { 0.0 } else { (1.0 - applied_brake * (1.0 - 0.007 * vzs) * taper).clamp(0.0, 1.0) };
        let cap = |x: f32| x.min(0.99999);
        let (xs, xn) = (cap(avg_w(|m| m.x_spin) * g[2] * a), cap(avg_w(|m| m.x_nospin) * g[2] * a));
        let (zs, zn) = (cap(avg(|m| m.z_spin) * a), cap(avg(|m| m.z_nospin) * a));
        let (rs, rn) = (cap(avg(|m| m.rot_spin) * g[3]), cap(avg(|m| m.rot_nospin) * g[3]));
        let yaw_term = self.ang.y.abs() * (self.vel.z * self.vel.z + self.vel.x * self.vel.x);
        let locked = applied_brake >= 0.9 * bf;
        self.brake_locked = locked;
        let lateral = self.vel.x.abs() - sl;
        // the handbrake (P[0x2b], set at 0x40ec8f) always slides (§3.5); reverse never does
        self.sliding = self.gear != 0 && (handbrake || lateral > 0.0 || yaw_term > rl || self.wheelspin || locked);
        if self.sliding {
            // slide amount P[0x33] (0x47db1c..0x47dc19): the last rule that applies wins
            if handbrake {
                self.slide = 0.0;
            }
            if lateral > 0.0 {
                self.slide = lateral;
            }
            if yaw_term > rl {
                self.slide = 0.001 * yaw_term;
            }
            if self.wheelspin {
                self.slide = (self.gear as f32 * 500.0 + self.rpm) * 0.0007;
            }
            if locked {
                self.slide = (self.vel.x.abs() + self.vel.z.abs()) * 0.3;
            }
        } else {
            self.slide = 0.0; // 0x47eb3b
        }
        let (x, z, r) = if self.sliding { (xs, zs, rs) } else { (xn, zn, rn) };
        // design choice: a wheel whose hit normal points down (ny < 0) gives a negative weight 1/ny^3 and
        // so a negative base for `powf`: NaN, and a car that is gone for good. The original's clamp is above only.
        let (x, z, r) = (x.max(0.0), z.max(0.0), r.max(0.0));
        self.vel.x *= x.powf(h);
        self.vel.z *= z.powf(h);
        self.ang.x *= 5e-5f32.powf(h);
        self.ang.y *= r.powf(h);
        self.ang.z *= r.powf(h);
    }

    /// Hull test points in body space: 4 corners at CoM height and the three roof points when
    /// fewer than 3 wheels touch and the driver is not an AI (0x47d7c0..0x47d7e1).
    fn hull_points(&self) -> Vec<Vec3> {
        let mut pts: Vec<Vec3> = self.p.corners.iter().map(|c| Vec3::new(c.x, self.p.com.y, c.z)).collect();
        if !self.ai_driver && self.wheel_contact.iter().filter(|&&c| c).count() < 3 {
            pts.extend([Vec3::new(0.0, 1.3, 1.0), Vec3::new(1.0, 1.3, -1.0), Vec3::new(-1.0, 1.3, -1.0)].map(|r| r * self.p.scale));
        }
        pts
    }

    /// The hull points a prop or another car can be hit by.
    pub(crate) fn body_points(&self) -> Vec<Vec3> {
        self.hull.clone()
    }

    fn collide_body(&self) -> CollideBody<'_> {
        CollideBody { points: &self.hull, com: self.p.com, mass: self.p.mass, inertia: self.p.inertia(), scale: self.p.scale * self.p.feel, car: true }
    }

    /// One pass of the world collide `0x47f5e8` for this car (the car path, 04 §4.9.1), the hull
    /// against the track. Returns whether a hull point hit; the caller loops up to three passes
    /// with the car-car contacts in between (`Race::step`, 0x481149).
    // CLASSIC 0x47f5e8
    pub fn world_collide(&mut self, w: &World, h: f32) -> bool {
        let mut pose = Pose { pos: self.pos, rot: self.rot, scale: Vec3::ONE };
        let (mut vel, mut ang) = (self.vel, self.ang);
        let mut records = std::mem::take(&mut self.records);
        let rb = self.collide_body();
        let impact = world_collide(&rb, &mut vel, &mut ang, &mut pose, &self.start, h, &|a, b| w.raycast(a, b), &mut |k, hit, d| {
            records[k] = Some((hit.normal, (-d.dot(hit.normal) * (1.0 - hit.t)).max(0.0)));
        });
        self.records = records;
        let Some(hit) = impact else { return false };
        (self.pos, self.rot, self.vel, self.ang) = (pose.pos, pose.rot, vel, ang);
        // step 8: rb+0x72c is the nearest hit's material
        self.hit_material = Some(hit.hit.material);
        keep_hit(&mut self.world_hits, WorldHit { impulse: hit.impulse.abs(), material: hit.hit.material, metric: hit.metric, point: hit.point });
        // wall drag (ours)
        self.scraping = true;
        // a wall: steeper than the wheels take (WHEEL_MIN_NY) and not a ceiling
        self.on_wall |= (-0.5..WHEEL_MIN_NY).contains(&hit.hit.normal.y);
        if hit.closing < 0.0 {
            self.impact = self.impact.max(-hit.closing);
        }
        true
    }

    /// The car's resting pass `0x47eda0`, after the collide passes.
    // CLASSIC 0x47eda0
    pub fn world_post(&mut self, w: &World, h: f32) {
        let mut pose = Pose { pos: self.pos, rot: self.rot, scale: Vec3::ONE };
        let material = world_post(&self.hull, &mut self.vel, &mut pose, &self.start, h, &|a, b| w.raycast(a, b));
        self.pos = pose.pos;
        if let Some(m) = material {
            self.hit_material.get_or_insert(m);
        }
    }

    /// The world step of one substep for this car alone: up to `passes` collide passes, the
    /// resting pass, and `end_substep`. `Race` runs the same with car-car contacts in the loop.
    pub fn collide_world(&mut self, h: f32, w: &World, passes: usize) {
        for _ in 0..passes {
            if !self.world_collide(w, h) {
                break;
            }
        }
        self.world_post(w, h);
        self.end_substep();
    }

    /// The hit records go to the last-substep copy (0x4811cd) and the matrix is re-orthonormalised
    /// (0x42c180, at the end of the world step).
    pub fn end_substep(&mut self) {
        self.hull_hits = self.records.iter().flatten().copied().collect();
        self.rot = orthonormalize(self.rot);
    }
}

/// The slot update of `0x47f5e8` step 8 (0x47fd9b..0x47fede): a stronger impulse pushes slot A
/// to slot B and takes A; otherwise a larger metric takes B. The empty slots read 0 (0x4815d6),
/// so a negative metric never takes B.
fn keep_hit(slots: &mut [Option<WorldHit>; 2], new: WorldHit) {
    let [a, b] = *slots;
    if new.impulse > a.map_or(0.0, |a| a.impulse) {
        *slots = [Some(new), a];
    } else if new.metric > b.map_or(0.0, |b| b.metric) {
        slots[1] = Some(new);
    }
}

/// The slide amount P[0xcc] above which a wheel marks the ground (0x4d3b65, 08-missing-features 8.6).
pub const SKID_SLIDE: f32 = 8.0;

/// A skid segment is committed once the wheel is `t` metres from its start (compared as d² > t²):
/// `t = (vx² + vz² + 650) * 0.0015` from the body velocity, squared again and capped at 9
/// (0x4469ac..0x4469e5; 0x4d3b6d = 650, 0x4d3b55 = 0.0015, 0x4d3b75 = 9). Returns that t² cap.
pub fn skid_step2(vel: Vec3) -> f32 {
    let t = (vel.x * vel.x + vel.z * vel.z + 650.0) * 0.0015;
    (t * t).min(9.0)
}

impl Car {
    /// Which wheels (FL, FR, BR, BL) the skid-mark code asks to mark, before the wheel-on-ground and
    /// surface tests (CLASSIC 0x4467d5..0x446926): the rear wheels on a slide amount above `SKID_SLIDE`,
    /// wheelspin P[0x25] or locked brakes P[0x28]; the front wheels on locked brakes, or a big slide
    /// without wheelspin.
    pub fn skid_marking(&self) -> [bool; 4] {
        let slid = self.slide > SKID_SLIDE;
        let front = self.brake_locked || (!self.wheelspin && slid);
        let rear = self.brake_locked || self.wheelspin || slid;
        [front, front, rear, rear]
    }
}

impl Car {
    /// The hull points that hit the world in the last substep's collide passes as a bit mask
    /// (bit k = point k), and the smallest world normal y among them: the tracer's `hull_hits` /
    /// `hull_ny` (#266).
    pub fn hull_contacts(&self) -> (u32, Option<f32>) {
        self.records.iter().enumerate().filter_map(|(k, r)| r.map(|(n, _)| (k, n.y))).fold((0, None), |(mask, ny), (k, y)| (mask | 1 << k, Some(ny.map_or(y, |m: f32| m.min(y)))))
    }

    /// Hull point `k` in body space (P+0x540 + 12k, read when the sparks spawn, 0x446530).
    pub fn hull_point(&self, k: usize) -> Vec3 {
        self.hull.get(k).copied().unwrap_or(Vec3::ZERO)
    }
}

fn orthonormalize(m: Mat3) -> Mat3 {
    let z = m.z_axis.normalize();
    let x = m.y_axis.cross(z).normalize();
    let y = z.cross(x);
    Mat3::from_cols(x, y, z)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hit(impulse: f32, metric: f32, point: usize) -> WorldHit {
        WorldHit { impulse, material: 0, metric, point }
    }

    #[test]
    fn slots_follow_step_8() {
        let mut c = [None; 2];
        keep_hit(&mut c, hit(10.0, 5.0, 0));
        assert_eq!(c, [Some(hit(10.0, 5.0, 0)), None]);
        // weaker impulse, larger metric: B only
        keep_hit(&mut c, hit(4.0, 20.0, 1));
        assert_eq!(c, [Some(hit(10.0, 5.0, 0)), Some(hit(4.0, 20.0, 1))]);
        // weaker on both: unchanged
        keep_hit(&mut c, hit(3.0, 7.0, 2));
        assert_eq!(c[1], Some(hit(4.0, 20.0, 1)));
        // stronger impulse: A moves to B
        keep_hit(&mut c, hit(30.0, 1.0, 3));
        assert_eq!(c, [Some(hit(30.0, 1.0, 3)), Some(hit(10.0, 5.0, 0))]);
        // a negative metric never takes an empty B
        let mut c = [None; 2];
        keep_hit(&mut c, hit(1.0, 0.0, 0));
        keep_hit(&mut c, hit(0.5, -3.0, 1));
        assert_eq!(c[1], None);
    }

    fn test_car() -> Car {
        let wheels = [Vec3::new(-0.8, 0.0, 1.3), Vec3::new(0.8, 0.0, 1.3), Vec3::new(0.8, 0.0, -1.3), Vec3::new(-0.8, 0.0, -1.3)];
        let p = CarParams {
            name: "test".into(),
            mass: 1000.0,
            gear_top: vec![50.0, 77.0, 121.0, 176.0, 225.0, 275.0, 330.0],
            min_rpm: 1000.0,
            max_rpm: 10000.0,
            acc_force: 5000.0,
            brake_force: 1.0,
            turn_force: 1000.0,
            com: Vec3::ZERO,
            wheels,
            corners: wheels,
            spring_len: 0.3,
            spring_k: 1000.0,
            damper: SUSP_DAMP,
            grip: [1.0; 4],
            hull: Hull::from_box(Vec3::splat(-1.0), Vec3::splat(1.0)),
            scale: 1.0,
            feel: 1.0,
        };
        Car::new(p, Vec3::ZERO, Vec3::Z)
    }

    #[test]
    fn gulliver_scale_keeps_the_rest_and_comes_back() {
        let p = test_car().p;
        let small = p.scaled(0.2, 0.6);
        assert!((small.rest_ext(15.81) - p.rest_ext(15.81)).abs() < 1e-5);
        assert!((small.rest_ground(15.81) - 0.2 * p.rest_ground(15.81)).abs() < 1e-5);
        assert!((small.ride_height() - 0.2 * p.ride_height()).abs() < 1e-6);
        assert!((small.inertia() - INERTIA * 0.12).abs().max_element() < 1e-2);
        // a point inside the full hull is inside the small one at 0.2 of the way
        let inside = Vec3::new(0.5, 0.5, -0.5);
        assert!(p.hull.exit(inside, Vec3::Y).is_some() && small.hull.exit(inside * 0.2, Vec3::Y).is_some());
        assert!(small.hull.exit(inside * 0.2 + Vec3::Y * 0.3, Vec3::Y).is_none());
        let back = small.scaled(1.0, 1.0);
        assert!((back.wheels[0] - p.wheels[0]).length() < 1e-6 && (back.spring_k - p.spring_k).abs() < 1e-3);
        assert_eq!((back.scale, back.feel), (1.0, 1.0));
    }

    #[test]
    fn skid_marking_is_the_original_rule() {
        let mut c = test_car();
        for bits in 0..8u32 {
            (c.brake_locked, c.wheelspin, c.slide) = (bits & 1 != 0, bits & 2 != 0, if bits & 4 != 0 { 9.0 } else { 8.0 });
            let slid = c.slide > 8.0;
            let (front, rear) = (c.brake_locked || (!c.wheelspin && slid), c.brake_locked || c.wheelspin || slid);
            assert_eq!(c.skid_marking(), [front, front, rear, rear], "bits {bits}");
        }
    }

    #[test]
    fn wheelspin_flag_has_hysteresis_and_skips_ai() {
        let mut c = test_car();
        // 1st gear, standstill, 8000 rpm: Q = 8 * 400 = 3200
        c.rpm = 8000.0;
        c.spin_limit = 3000.0;
        assert!(c.traction(1.0, 0.0).0);
        // 7000 rpm: Q = 2800 starts nothing at the gripping limit, but keeps spinning under the spinning one
        c.rpm = 7000.0;
        assert!(!c.traction(1.0, 0.0).0);
        c.spin_limit = 2000.0;
        assert!(c.traction(1.0, 0.0).0);
        // no throttle, or the clutch in: no wheelspin flag
        assert!(!c.traction(0.05, 0.0).0);
        c.clutch = true;
        assert!(c.traction(1.0, 0.0).0 && !c.wheelspin);
        // AI cars never spin
        c.clutch = false;
        c.ai_driver = true;
        assert!(!c.traction(1.0, 0.0).0 && !c.wheelspin);
        // 4th gear has no torque entry
        c.ai_driver = false;
        c.gear = 4;
        assert!(!c.traction(1.0, 0.0).0);
    }

    #[test]
    fn spin_law_follows_the_formula() {
        let mut c = test_car();
        c.rpm = 6000.0;
        c.spin_limit = 2000.0;
        let (spin, q, g) = c.traction(1.0, 0.0);
        assert!(spin && g == 1);
        let h = 0.02;
        let (f, cand) = c.curve(Some(q), g, 1.0, 5000.0, h);
        let want = 0.625f32.powf(h) * (6000.0 + h * 27.5 * (80.0 + 0.02 * (q - 2000.0)));
        assert!((cand - want).abs() < 1e-2, "{cand} vs {want}");
        let x = 5.0f32;
        let want_f = 0.7 * 2000.0 / (0.0005 * q * q) * (x.sqrt() - 2.0 / (9.7 - x)) * 5000.0 + 10.0 * 300.0;
        assert!((f - want_f).abs() < 1.0, "{f} vs {want_f}");
        // the cap keeps the singularity near 10 700 rpm finite
        c.rpm = 10700.0;
        assert!(c.curve(Some(q), g, 1.0, 5000.0, h).0.abs() < 1e6);
    }

    #[test]
    fn clutch_commits_or_free_revs() {
        let mut c = test_car();
        c.rpm = 5000.0;
        // rear wheel down, no key: the candidate is committed
        c.wheel_contact = [false, false, true, false];
        c.clutch_rpm(3000.0, 1.0, 0.02, false);
        assert!(!c.clutch && c.rpm == 3000.0);
        // grid hold, a pending shift key, or both rear wheels airborne: free revs
        for (contact, pending, hold) in [(true, false, true), (true, true, false), (false, false, false)] {
            let mut c = test_car();
            c.rpm = 5000.0;
            c.wheel_contact = [false, false, contact, false];
            c.shift_pending[0] = pending;
            c.clutch_rpm(3000.0, 1.0, 0.02, hold);
            assert!(c.clutch);
            assert!((c.rpm - (5000.0 + 25.0 * 0.02 * 800.0)).abs() < 1e-2, "{}", c.rpm);
            c.clutch_rpm(3000.0, 0.0, 0.02, hold);
            assert!((c.rpm - (5400.0 - 6000.0 * 0.02)).abs() < 1e-2);
        }
        // never below the idle speed
        let mut c = test_car();
        c.rpm = 1010.0;
        c.clutch_rpm(3000.0, 0.0, 0.02, true);
        assert_eq!(c.rpm, 1000.0);
    }

    #[test]
    fn strength_and_spark_id() {
        let mats = [Material { spark_anim: 1, ..Default::default() }];
        assert!((hit(1.0, 10.0, 0).strength() - 0.3).abs() < 1e-6);
        assert_eq!(hit(1.0, 100.0, 0).strength(), 1.0);
        assert_eq!(hit(1.0, 10.0, 0).spark_anim(&mats), 1);
        // moving backwards clears the id (0x481442)
        assert_eq!(hit(1.0, -10.0, 0).spark_anim(&mats), 0);
    }
}
