//! Ghost mode and Record race on the original's `.MDE` files (re-notes 05-formats "MDE", 08 §8.9).
//!
//! The file holds the car's *model* origin: x and z equal the physics position, y is higher by
//! the ride offset (`MODEL_OFFSET`, the NeoCity grid slot 0 gives (-322.96, 22.08) in the sim and
//! (-322.96, 22.36) in the sample) [O]. `pose` takes that off again, so `GhostPose` has the same
//! meaning as in `replay.rs` (physics origin).

use glam::{Mat3, Vec3};
use motor_data::mde::{self, CarState, Header, Mde, Record, RECORD_MS};

use crate::car::{Car, WorldHit};
use crate::world::Material;
use crate::replay::GhostPose;
use crate::car::Telemetry;
use crate::session::{Race, Racer};

/// A decoded car block as a pose; `ride` is the car's `ride_height()`, `roll` the distance rolled.
pub fn pose(s: &CarState, ms: f64, roll: f32, ride: f32) -> GhostPose {
    let m = s.matrix();
    let rot = Mat3::from_cols(Vec3::new(m[0], m[1], m[2]), Vec3::new(m[3], m[4], m[5]), Vec3::new(m[6], m[7], m[8]));
    GhostPose { clock_ms: ms, pos: Vec3::from(s.pos) - rot * Vec3::new(0.0, ride, 0.0), rot, wheel_ext: s.spring, steer_smooth: s.steer, roll }
}

/// The pack side (`0x40b9d0`): a live car as a car block. `brake` is this frame's brake input.
/// The spark slots A / B (`P+0x168/0x16c` SparkAnimID bit, `P+0x178/0x17c` strength, `P+0x188/0x18c`
/// hull point) come from `Car::world_hits`, bit 9 is the overrun, bit 10 the pedal above 0 (0x40bd8c..0x40bdb0)
/// [O]. Stay 0: bit 1 (the horn key, we have no horn) and the front tyre effects (the original has no slot) [O].
pub fn car_state(car: &Car, brake: f32, materials: &[Material]) -> CarState {
    let ride = car.p.ride_height();
    let at = |i: usize| car.wheel_contact[i];
    let flags = (brake > 0.15) as u16
        | (car.sliding as u16) << 2
        | (car.wheelspin as u16) << 3
        | (car.brake_locked as u16) << 4
        | (at(0) as u16) << 5
        | (at(1) as u16) << 6
        | (at(2) as u16) << 7
        | (at(3) as u16) << 8
        | (car.overrun as u16) << 9
        | ((car.pedal > 0.0) as u16) << 10;
    let fx = |i: usize| car.tyre_effect[i].map_or((0, 0.0), |(id, s)| (id as u8, s));
    let (id0, s0) = fx(0);
    let (id1, s1) = fx(1);
    // a negative strength (moving backwards) writes 0 (0x48141a) and has no spark id (0x481442)
    let spark = |i: usize| car.world_hits[i].map_or((false, 0.0, 0), |h| (h.spark_anim(materials) & 1 != 0, h.strength().max(0.0), h.point as u8));
    let (a, b) = (spark(0), spark(1));
    let m = [car.rot.x_axis, car.rot.y_axis, car.rot.z_axis];
    let m: [f32; 9] = std::array::from_fn(|i| m[i / 3][i % 3]);
    CarState {
        spring: car.wheel_ext,
        steer: car.steer_smooth,
        aux: car.wheel_spin,
        rpm: car.rpm,
        slide: car.slide,
        gear: car.gear.min(7) as u8,
        fx: [s0, s1, a.1, b.1],
        surf: [id0 & 7, id1 & 7],
        bits: [a.0, b.0],
        flags,
        w188: a.2 & 3,
        w18c: b.2 & 3,
        angles: mde::angles_from_matrix(m),
        pos: (car.pos + car.rot * Vec3::new(0.0, ride, 0.0)).to_array(),
    }
}

/// The unpack side of the spark slots (apply 0x40c5f4): the spark id bit (`P+0x168/0x16c`), the strength
/// (`P+0x178/0x17c`) and the hull point (`P+0x188/0x18c`) are restored; the impulse (`P+0xd0`) and the hit
/// material (`P+0x154`) are not [O], so the weak / hard hit sounds stay silent. The material is any one with
/// SparkAnimID 1, which is all `spawn_sparks` asks of it; the metric is `strength / 0.03` so that
/// `WorldHit::strength` gives the recorded value back.
pub fn spark_hits(s: &CarState, materials: &[Material]) -> [Option<WorldHit>; 2] {
    let material = materials.iter().position(|m| m.spark_anim == 1);
    let slot = |i: usize, point: u8| {
        let material = material.filter(|_| s.bits[i])?;
        Some(WorldHit { impulse: 0.0, material, metric: s.fx[2 + i] / 0.03, point: point as usize })
    };
    [slot(0, s.w188), slot(1, s.w18c)]
}

/// Pose a remote racer from a recorded car block (Ghost mode's demo play): position, orientation, springs,
/// steering, engine, tyre effects and the flags the HUD, sound and effects read. The speed is the
/// movement since the last pose over `dt`, in the body frame like `Car::vel` (a jump of 5 m or more is
/// a restart and gives none).
pub fn drive_remote(r: &mut Racer, s: &CarState, dt: f32, materials: &[Material]) {
    let ride = r.car.p.ride_height();
    let p = pose(s, 0.0, 0.0, ride);
    let car = &mut r.car;
    let step = p.pos - car.pos;
    car.vel = if dt > 1e-4 && step.length() < 5.0 { p.rot.transpose() * (step / dt) } else { Vec3::ZERO };
    car.pos = p.pos;
    car.rot = p.rot;
    car.wheel_ext = s.spring;
    car.steer_smooth = s.steer;
    car.rpm = s.rpm;
    car.gear = s.gear as usize;
    car.slide = s.slide;
    car.wheel_spin = s.aux;
    car.sliding = s.flags & 4 != 0;
    car.wheelspin = s.flags & 8 != 0;
    car.brake_locked = s.flags & 16 != 0;
    car.wheel_contact = std::array::from_fn(|i| s.flags & (32 << i) != 0);
    car.tyre_effect = [0, 1].map(|i| (s.fx[i] > 0.0).then_some((s.surf[i] as u32, s.fx[i])));
    car.world_hits = spark_hits(s, materials);
    r.brake = (s.flags & 1) as f32;
    r.telemetry = Telemetry {
        kmh: car.vel.z.abs() * 3.6,
        gear: car.gear,
        rpm: car.rpm,
        drive_force: 0.0,
        contacts: car.wheel_contact.iter().filter(|&&c| c).count(),
        sliding: car.sliding,
        wheelspin: car.wheelspin,
    };
}

/// The green light on the `.MDE` clock. The original's recorder and playback run only from the
/// "3" on (race clock base + 6000 ms, `0x45ae55` -> `0x41cd18` / `0x41d69c`), 6 s before GO (the
/// sample's car first moves between 5975 and 6042 ms) [O]. Our pre-race is the same 12 s, so the
/// clock is negative during the fly-by (the cars stand on their first pose) and nothing is recorded.
pub const GO_MS: f64 = 6000.0;

/// The time of the `.MDE` clock now: 0 at the "3", `GO_MS` at the green light, the race clock after it.
pub fn race_ms(race: &Race) -> f64 {
    if race.countdown > 0.0 {
        GO_MS - race.countdown as f64 * 1000.0
    } else {
        GO_MS + race.clock_ms
    }
}

/// `race_ms` of the drawn view (#374): on the fixed tick the cars are drawn up to one tick behind the race clock, so a
/// recorded car posed for drawing is looked up that much earlier. `race_ms` on the per-frame path.
pub fn view_ms(race: &Race) -> f64 {
    race_ms(race) - race.view_lag() as f64 * 1000.0
}

/// Writes records while a race runs (Record race): one every `RECORD_MS` of race time.
pub struct DemoRecorder {
    mde: Mde,
    /// race ms of the next record
    next: f64,
    /// the slot of every active header entry, in order
    slots: Vec<usize>,
}

impl DemoRecorder {
    pub fn new(header: Header) -> DemoRecorder {
        let slots = header.active_slots();
        DemoRecorder { mde: Mde::new(header), next: 0.0, slots }
    }

    /// After a stepped frame: `cars[k]` and its brake input are the racer of the k-th active entry.
    pub fn push(&mut self, ms: f64, cars: &[(&Car, f32)], materials: &[Material]) {
        if ms < self.next || cars.len() != self.slots.len() {
            return;
        }
        self.next = ms + RECORD_MS;
        let mask = self.slots.iter().fold(0u32, |m, &s| m | 1 << s);
        self.mde.records.push(Record { mask, ms: ms.max(1.0) as u32, cars: cars.iter().map(|&(c, b)| car_state(c, b, materials)).collect() });
    }

    /// Restart: forget what was recorded.
    pub fn clear(&mut self) {
        self.mde.records.clear();
        self.next = 0.0;
    }

    pub fn len(&self) -> usize {
        self.mde.records.len()
    }

    pub fn is_empty(&self) -> bool {
        self.mde.records.is_empty()
    }

    pub fn finish(self) -> Mde {
        self.mde
    }

    /// What has been recorded so far, the recorder running on.
    pub fn snapshot(&self) -> Mde {
        self.mde.clone()
    }
}

/// Distance each car block rolled up to every record (m, along the body's forward axis): the wheels'
/// turn at record `i` is `roll[k][i] / radius`. One vector per car block.
pub fn rolls(mde: &Mde) -> Vec<Vec<f32>> {
    let n = mde.header.count as usize;
    (0..n)
        .map(|k| {
            let mut roll = 0.0;
            let mut last: Option<Vec3> = None;
            mde.records
                .iter()
                .map(|r| {
                    let c = &r.cars[k];
                    let p = Vec3::from(c.pos);
                    if let Some(l) = last {
                        let m = c.matrix();
                        roll += (p - l).dot(Vec3::new(m[6], m[7], m[8]));
                    }
                    last = Some(p);
                    roll
                })
                .collect()
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The spark slots, overrun and pedal reach the car block (#278).
    #[test]
    fn car_block_carries_spark_slots() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../installed_game");
        let Ok(cfg) = motor_data::fs::config(&root, "Game/car07.car") else { return };
        let mut car = Car::new(crate::car::CarParams::from_config(&cfg).unwrap(), Vec3::ZERO, Vec3::Z);
        let mats = vec![crate::world::Material { spark_anim: 1, ..Default::default() }];
        car.world_hits[1] = Some(crate::car::WorldHit { impulse: 1.0, material: 0, metric: 20.0, point: 3 });
        (car.overrun, car.pedal) = (true, 1.0);
        let s = car_state(&car, 0.0, &mats);
        assert_eq!((s.bits, s.w188, s.w18c), ([false, true], 0, 3));
        assert_eq!(s.fx[2], 0.0);
        assert!((s.fx[3] - 0.6).abs() < 1e-6);
        assert_eq!(s.flags >> 9, 3);
        // moving backwards: no spark id, no strength
        car.world_hits[1] = Some(crate::car::WorldHit { impulse: 1.0, material: 0, metric: -5.0, point: 1 });
        let s = car_state(&car, 0.0, &mats);
        assert_eq!((s.bits, s.fx[3]), ([false, false], 0.0));
    }

    /// The spark slots come back from a car block as `spark_hits` (#290).
    #[test]
    fn spark_slots_round_trip() {
        let mats = vec![Material::default(), Material { spark_anim: 1, ..Default::default() }];
        let s = CarState { bits: [true, false], fx: [0.0, 0.0, 0.6, 0.4], w188: 2, ..Default::default() };
        let mut bytes = Vec::new();
        s.write(&mut bytes);
        let s = CarState::parse(&bytes).unwrap();
        let h = spark_hits(&s, &mats);
        let a = h[0].expect("slot A");
        assert_eq!((a.material, a.point, a.impulse), (1, 2, 0.0));
        assert!((a.strength() - 0.6).abs() < 1e-6);
        assert_eq!(a.spark_anim(&mats), 1);
        assert_eq!(h[1], None, "a clear bit leaves the slot empty");
        // no material throws sparks: nothing to show
        assert_eq!(spark_hits(&s, &[Material::default()]), [None, None]);
    }

    #[test]
    fn jeepen_rolls_forward() {
        let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../installed_game/Demos/JeePen.MDE");
        let Ok(m) = Mde::load(&p) else { return };
        let r = rolls(&m);
        // ten laps of NeoCity: several kilometres
        assert!(*r[0].last().unwrap() > 5000.0, "{}", r[0].last().unwrap());
    }
}
