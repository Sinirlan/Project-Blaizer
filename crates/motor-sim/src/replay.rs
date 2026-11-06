//! Race recordings in our own format (`.mhr`). Not the original's `.MDE` (design choice).
//!
//! The sim is deterministic: the AI's random numbers come from a per-slot seed, and the only
//! outside inputs are the frame length, the player's `Input` and respawns. So a recording is a
//! header (everything `Race::load_field` needs) plus one record per frame. Replaying the frames
//! through `Race::step` reproduces the race exactly; a ghost run or a replay viewer only needs
//! to step a second `Race` the same way.
//!
//! Frames are per rendered frame (dt, input), also on the fixed tick: the same dts through the same
//! accumulator give the same ticks, so the header's `tick_hz` is all a replay needs (#374).
//!
//! Layout, little endian:
//! ```text
//! "MHR3"     ("MHR2" has no tick rate and plays on the per-frame path; "MHR1" has no driver list either and
//!             reads as the fixed line-up)
//! u32 track, u32 laps, u8 flags (1 catchup, 2 manual, 4 knockout), u8 ai row kind (0 difficulty,
//! 1 division), u8 ai row value, i8 player slot (-1 none), u8 car count, u32 car id × count
//! str track name, str player nick, str car name       (u8 length + Latin-1; for listings)
//! u8 driver count, str driver nick × count            (MHR2+: the profile in each slot, "" for you)
//! f32 tick hz (MHR3: the fixed physics tick the run was made on, #374; 0 = one step per frame)
//! u32 race ms (0 = not finished), u32 best lap ms (0 = none)
//! u32 frame count, then per frame:
//!   f32 dt, f32 throttle, f32 brake, f32 steer, u8 bits (1 up, 2 down, 4 handbrake),
//!   u8 respawn (0 none, else slot + 1)
//! ```
//! Input bits: 1 up, 2 down, 4 handbrake, 8 skip the start fly-by (our design, #216; older files have 0),
//! bits 4..7 the slot + 1 the input drives when it is not the human's car (our design, #60; 0 = the
//! human's, so older files read as before; at most 14 slots).

use crate::car::Input;
use crate::session::{load_drivers, AiRow, Difficulty, Driver, Race, Rules};
use glam::{Mat3, Vec3};
use std::path::Path;

const MAGIC: &[u8; 4] = b"MHR3";
const MAGIC_V2: &[u8; 4] = b"MHR2";
const MAGIC_V1: &[u8; 4] = b"MHR1";
const FRAME_BYTES: usize = 18;

/// Everything needed to rebuild the race a recording belongs to.
#[derive(Debug, Clone, PartialEq)]
pub struct Header {
    pub track: u32,
    pub laps: u32,
    pub catchup: bool,
    /// Knockout (our design); stored as flag bit 4, so older files read as Normal
    pub rules: Rules,
    pub manual: bool,
    pub row: AiRow,
    pub player: Option<usize>,
    pub cars: Vec<u32>,
    pub track_name: String,
    pub nick: String,
    pub car_name: String,
    /// PlayerNick of the League profile driving each slot ("" for you); empty = MHR1, the fixed
    /// line-up of `load_drivers` (the single-race field is drawn at random, #217)
    pub drivers: Vec<String>,
    /// the fixed physics tick the run was made on (`Race::fixed_hz`, #374); `None` = one step per frame
    /// (MHR1/MHR2 files, which replay on that path)
    pub tick_hz: Option<f32>,
    /// race time in ms once the player finished
    pub race_ms: Option<u32>,
    pub best_lap_ms: Option<u32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Frame {
    pub dt: f32,
    pub input: Input,
    /// slot respawned this frame (the R key)
    pub respawn: Option<usize>,
    /// the player skipped the start fly-by this frame (`Race::skip_intro`)
    pub skip: bool,
    /// the slot `input` drives when it is not the human's car (motorview's hand driving, #60;
    /// `Race::step_as`); `None` = the human's car
    pub hand: Option<usize>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Recording {
    pub header: Header,
    pub frames: Vec<Frame>,
}

impl Recording {
    pub fn new(header: Header) -> Recording {
        Recording { header, frames: Vec::new() }
    }

    /// Append the frame that is about to be stepped.
    pub fn push(&mut self, dt: f32, input: Input, respawn: Option<usize>, skip: bool) {
        self.frames.push(Frame { dt, input, respawn, skip, hand: None });
    }

    /// The recorded drivers, slot by slot, from the profiles by nick; None for an MHR1 file.
    pub fn drivers(&self, root: &Path) -> Option<Vec<Driver>> {
        if self.header.drivers.is_empty() {
            return None;
        }
        let profiles = load_drivers(root);
        Some(self.header.drivers.iter().map(|n| profiles.iter().find(|d| d.nick.eq_ignore_ascii_case(n)).cloned().unwrap_or_default()).collect())
    }

    /// A new race in the recorded setup, ready for `step`.
    pub fn load_race(&self, root: &Path) -> Result<Race, String> {
        let h = &self.header;
        let drivers = self.drivers(root);
        let mut race = Race::load_field(root, h.track, &h.cars, h.player, Some(h.laps), h.row, drivers.as_deref())?;
        if h.rules != Rules::Normal {
            race.set_rules(h.rules);
        }
        if !h.catchup {
            race.ai_vector[10] = 0.0;
        }
        if let Some(r) = h.player.and_then(|p| race.racers.get_mut(p)) {
            r.car.manual = h.manual;
        }
        race.set_fixed_tick(h.tick_hz);
        Ok(race)
    }

    /// Advance a race by one recorded frame.
    pub fn step(race: &mut Race, f: &Frame) {
        if let Some(slot) = f.respawn {
            race.respawn(slot);
        }
        if f.skip {
            race.skip_intro();
        }
        race.step_as(f.dt, f.input, f.hand);
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        let h = &self.header;
        let mut o = Vec::with_capacity(96 + self.frames.len() * FRAME_BYTES);
        o.extend_from_slice(MAGIC);
        o.extend_from_slice(&h.track.to_le_bytes());
        o.extend_from_slice(&h.laps.to_le_bytes());
        o.push(h.catchup as u8 | (h.manual as u8) << 1 | ((h.rules == Rules::Elimination) as u8) << 2);
        match h.row {
            AiRow::Difficulty(d) => o.extend([0, d as u8]),
            AiRow::Division(n) => o.extend([1, n as u8]),
        }
        o.push(h.player.map_or(-1i8, |p| p as i8) as u8);
        o.push(h.cars.len() as u8);
        for c in &h.cars {
            o.extend_from_slice(&c.to_le_bytes());
        }
        for s in [&h.track_name, &h.nick, &h.car_name] {
            let b: Vec<u8> = s.chars().map(|c| if (c as u32) < 256 { c as u8 } else { b'?' }).take(255).collect();
            o.push(b.len() as u8);
            o.extend(b);
        }
        o.push(h.drivers.len() as u8);
        for n in &h.drivers {
            let b: Vec<u8> = n.chars().map(|c| if (c as u32) < 256 { c as u8 } else { b'?' }).take(255).collect();
            o.push(b.len() as u8);
            o.extend(b);
        }
        o.extend_from_slice(&h.tick_hz.unwrap_or(0.0).to_le_bytes());
        o.extend_from_slice(&h.race_ms.unwrap_or(0).to_le_bytes());
        o.extend_from_slice(&h.best_lap_ms.unwrap_or(0).to_le_bytes());
        o.extend_from_slice(&(self.frames.len() as u32).to_le_bytes());
        for f in &self.frames {
            for v in [f.dt, f.input.throttle, f.input.brake, f.input.steer] {
                o.extend_from_slice(&v.to_le_bytes());
            }
            o.push(f.input.shift_up as u8 | (f.input.shift_down as u8) << 1 | (f.input.handbrake as u8) << 2 | (f.skip as u8) << 3 | f.hand.map_or(0, |h| (h.min(14) as u8 + 1) << 4));
            o.push(f.respawn.map_or(0, |s| s as u8 + 1));
        }
        o
    }

    pub fn from_bytes(d: &[u8]) -> Result<Recording, String> {
        let mut r = Reader { d, o: 0 };
        let version = match r.take(4)? {
            m if m == MAGIC => 3,
            m if m == MAGIC_V2 => 2,
            m if m == MAGIC_V1 => 1,
            _ => return Err("not a Motorhead recording".into()),
        };
        let track = r.u32()?;
        let laps = r.u32()?;
        let flags = r.u8()?;
        let (kind, value) = (r.u8()?, r.u8()?);
        let row = match kind {
            0 => AiRow::Difficulty(match value {
                0 => Difficulty::Easy,
                1 => Difficulty::Medium,
                3 => Difficulty::Beginner,
                _ => Difficulty::Hard,
            }),
            _ => AiRow::Division(value as u32),
        };
        let player = Some(r.u8()? as i8).filter(|&p| p >= 0).map(|p| p as usize);
        let n = r.u8()? as usize;
        let cars = (0..n).map(|_| r.u32()).collect::<Result<Vec<_>, _>>()?;
        let track_name = r.string()?;
        let nick = r.string()?;
        let car_name = r.string()?;
        let drivers = if version >= 2 { (0..r.u8()?).map(|_| r.string()).collect::<Result<Vec<_>, _>>()? } else { Vec::new() };
        let tick_hz = if version >= 3 { Some(r.f32()?).filter(|&h| h > 0.0) } else { None };
        let race_ms = Some(r.u32()?).filter(|&t| t > 0);
        let best_lap_ms = Some(r.u32()?).filter(|&t| t > 0);
        let count = r.u32()? as usize;
        if r.d.len() - r.o != count * FRAME_BYTES {
            return Err("recording is truncated".into());
        }
        let mut frames = Vec::with_capacity(count);
        for _ in 0..count {
            let (dt, throttle, brake, steer) = (r.f32()?, r.f32()?, r.f32()?, r.f32()?);
            let (bits, resp) = (r.u8()?, r.u8()?);
            let input = Input { throttle, brake, steer, shift_up: bits & 1 != 0, shift_down: bits & 2 != 0, handbrake: bits & 4 != 0, hold: false };
            frames.push(Frame { dt, input, respawn: resp.checked_sub(1).map(|s| s as usize), skip: bits & 8 != 0, hand: (bits >> 4).checked_sub(1).map(|h| h as usize) });
        }
        let header = Header { track, laps, catchup: flags & 1 != 0, manual: flags & 2 != 0, rules: if flags & 4 != 0 { Rules::Elimination } else { Rules::Normal }, row, player, cars, track_name, nick, car_name, drivers, tick_hz, race_ms, best_lap_ms };
        Ok(Recording { header, frames })
    }

    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::write(path, self.to_bytes())
    }

    pub fn load(path: &Path) -> Result<Recording, String> {
        Self::from_bytes(&std::fs::read(path).map_err(|e| e.to_string())?)
    }

    /// Replay the recording and sample the player's car after every frame, up to the end of its
    /// first lap: the dev `--ghost run.mhr` ghost. Poses are keyed by the race clock, which is
    /// the lap clock of lap 1.
    pub fn ghost(&self, root: &Path) -> Result<GhostTrack, String> {
        let slot = self.header.player.ok_or("the recording has no player")?;
        let mut race = self.load_race(root)?;
        let mut poses = Vec::with_capacity(self.frames.len());
        let mut roll = 0.0;
        for f in &self.frames {
            Self::step(&mut race, f);
            let r = &race.racers[slot];
            roll += r.car.vel.z * f.dt;
            // on the fixed tick a frame between two ticks has the same state and clock as the one before
            if race.ticks_last_frame() == 0 {
                continue;
            }
            poses.push(GhostPose { clock_ms: race.clock_ms, pos: r.car.pos, rot: r.car.rot, wheel_ext: r.car.wheel_ext, steer_smooth: r.car.steer_smooth, roll });
            if r.progress.lap > 1 || r.progress.finished_ms.is_some() {
                break;
            }
        }
        let lap_ms = race.racers[slot].progress.last_lap_ms.unwrap_or(0);
        Ok(GhostTrack { poses, lap_ms })
    }

    /// Length of the recording in seconds.
    pub fn seconds(&self) -> f32 {
        self.frames.iter().map(|f| f.dt).sum()
    }
}

/// One sampled frame of a ghost: what the renderer needs to draw the car again (the ghost
/// is always the player's own car, so its parameters come from the live car).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GhostPose {
    /// ms into the lap (a recorded lap), or `Race::clock_ms` (a ghost made from a `.mhr`)
    pub clock_ms: f64,
    pub pos: Vec3,
    pub rot: Mat3,
    pub wheel_ext: [f32; 4],
    pub steer_smooth: f32,
    /// distance rolled along the body's forward axis since the start (m); wheel angle = roll / radius
    pub roll: f32,
}

/// Poses of one lap, for `pose_at`: the Time Attack ghost (design choice).
#[derive(Debug, Clone, Default)]
pub struct GhostTrack {
    pub poses: Vec<GhostPose>,
    /// the lap time of the run it was made from
    pub lap_ms: u32,
}

/// A loaded `.ghl`: the lap, the car it was driven in and the track's name.
#[derive(Debug, Clone)]
pub struct GhostFile {
    pub track: GhostTrack,
    pub car: u32,
    pub track_name: String,
}

const GHOST_MAGIC: &[u8; 4] = b"GHL1";
const GHOST_FLOATS: usize = 14;
/// Poses closer than this (ms) are not recorded: about 60 per second.
const GHOST_GAP_MS: f64 = 16.0;

impl GhostTrack {
    /// Layout, little endian: "GHL1", u32 car id, u32 lap ms, str track name (u8 length +
    /// Latin-1), u32 count, then per pose 14 f32: ms, pos (3), rotation quaternion (4),
    /// spring extensions (4), steering, rolled distance.
    pub fn to_bytes(&self, car: u32, track_name: &str) -> Vec<u8> {
        let mut o = Vec::with_capacity(24 + self.poses.len() * GHOST_FLOATS * 4);
        o.extend_from_slice(GHOST_MAGIC);
        o.extend_from_slice(&car.to_le_bytes());
        o.extend_from_slice(&self.lap_ms.to_le_bytes());
        let name: Vec<u8> = track_name.chars().map(|c| if (c as u32) < 256 { c as u8 } else { b'?' }).take(255).collect();
        o.push(name.len() as u8);
        o.extend(name);
        o.extend_from_slice(&(self.poses.len() as u32).to_le_bytes());
        for p in &self.poses {
            let q = glam::Quat::from_mat3(&p.rot);
            let v = [p.clock_ms as f32, p.pos.x, p.pos.y, p.pos.z, q.x, q.y, q.z, q.w, p.wheel_ext[0], p.wheel_ext[1], p.wheel_ext[2], p.wheel_ext[3], p.steer_smooth, p.roll];
            for f in v {
                o.extend_from_slice(&f.to_le_bytes());
            }
        }
        o
    }

    /// The track and the car it belongs to.
    pub fn from_bytes(d: &[u8]) -> Result<(GhostTrack, u32), String> {
        let f = Self::parse(d)?;
        Ok((f.track, f.car))
    }

    /// The ghost with the track name it was saved for (motorview's ghost loader).
    pub fn parse(d: &[u8]) -> Result<GhostFile, String> {
        let mut r = Reader { d, o: 0 };
        if r.take(4)? != GHOST_MAGIC {
            return Err("not a Motorhead ghost".into());
        }
        let car = r.u32()?;
        let lap_ms = r.u32()?;
        let track_name = r.string()?;
        let n = r.u32()? as usize;
        if r.d.len() - r.o != n * GHOST_FLOATS * 4 {
            return Err("ghost is truncated".into());
        }
        let mut poses = Vec::with_capacity(n);
        for _ in 0..n {
            let mut v = [0.0f32; GHOST_FLOATS];
            for f in &mut v {
                *f = r.f32()?;
            }
            let q = glam::Quat::from_xyzw(v[4], v[5], v[6], v[7]).normalize();
            poses.push(GhostPose { clock_ms: v[0] as f64, pos: Vec3::new(v[1], v[2], v[3]), rot: Mat3::from_quat(q), wheel_ext: [v[8], v[9], v[10], v[11]], steer_smooth: v[12], roll: v[13] });
        }
        Ok(GhostFile { track: GhostTrack { poses, lap_ms }, car, track_name })
    }

    pub fn save(&self, path: &Path, car: u32, track_name: &str) -> std::io::Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::write(path, self.to_bytes(car, track_name))
    }

    pub fn load(path: &Path) -> Result<(GhostTrack, u32), String> {
        Self::from_bytes(&std::fs::read(path).map_err(|e| e.to_string())?)
    }

    /// The ghost at race clock `ms`, interpolated; `None` before the first and after the last frame.
    pub fn pose_at(&self, ms: f64) -> Option<GhostPose> {
        let (first, last) = (self.poses.first()?, self.poses.last()?);
        if ms < first.clock_ms || ms > last.clock_ms {
            return None;
        }
        let i = self.poses.partition_point(|p| p.clock_ms <= ms);
        let b = self.poses[i.min(self.poses.len() - 1)];
        let a = self.poses[i.saturating_sub(1)];
        let span = b.clock_ms - a.clock_ms;
        let t = if span > 0.0 { ((ms - a.clock_ms) / span) as f32 } else { 0.0 };
        let q = glam::Quat::from_mat3(&a.rot).slerp(glam::Quat::from_mat3(&b.rot), t);
        let mix = |x: f32, y: f32| x + (y - x) * t;
        Some(GhostPose {
            clock_ms: ms,
            pos: a.pos.lerp(b.pos, t),
            rot: Mat3::from_quat(q),
            wheel_ext: std::array::from_fn(|w| mix(a.wheel_ext[w], b.wheel_ext[w])),
            steer_smooth: mix(a.steer_smooth, b.steer_smooth),
            roll: mix(a.roll, b.roll),
        })
    }
}

/// Samples the player's car through a lap into a `GhostTrack` (Time Attack ghost).
#[derive(Debug, Clone, Default)]
pub struct LapRecorder {
    poses: Vec<GhostPose>,
    roll: f32,
}

impl LapRecorder {
    fn pose(&self, ms: f64, car: &crate::car::Car) -> GhostPose {
        GhostPose { clock_ms: ms, pos: car.pos, rot: car.rot, wheel_ext: car.wheel_ext, steer_smooth: car.steer_smooth, roll: self.roll }
    }

    /// After a stepped frame of `dt`: the car, `ms` into the current lap (negative = before the start).
    pub fn push(&mut self, ms: f64, dt: f32, car: &crate::car::Car) {
        self.roll += car.vel.z * dt;
        if ms < 0.0 || self.poses.last().is_some_and(|l| ms - l.clock_ms < GHOST_GAP_MS) {
            return;
        }
        let p = self.pose(ms, car);
        self.poses.push(p);
    }

    /// The frame that completed a lap of `lap_ms`: closes the lap with the car at the line and
    /// starts the next one empty.
    pub fn finish(&mut self, lap_ms: u32, dt: f32, car: &crate::car::Car) -> GhostTrack {
        self.roll += car.vel.z * dt;
        let last = self.pose(lap_ms as f64, car);
        self.poses.push(last);
        self.roll = 0.0;
        GhostTrack { poses: std::mem::take(&mut self.poses), lap_ms }
    }

    /// Forget the lap in progress (Restart).
    pub fn clear(&mut self) {
        self.poses.clear();
        self.roll = 0.0;
    }
}

struct Reader<'a> {
    d: &'a [u8],
    o: usize,
}

impl Reader<'_> {
    fn take(&mut self, n: usize) -> Result<&[u8], String> {
        let b = self.d.get(self.o..self.o + n).ok_or("recording is truncated")?;
        self.o += n;
        Ok(b)
    }
    fn u8(&mut self) -> Result<u8, String> {
        Ok(self.take(1)?[0])
    }
    fn u32(&mut self) -> Result<u32, String> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }
    fn f32(&mut self) -> Result<f32, String> {
        Ok(f32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }
    fn string(&mut self) -> Result<String, String> {
        let n = self.u8()? as usize;
        Ok(self.take(n)?.iter().map(|&c| c as char).collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ghost_file_keeps_track_name_and_car() {
        let pose = |ms: f64| GhostPose { clock_ms: ms, pos: Vec3::new(ms as f32, 1.0, 2.0), rot: Mat3::IDENTITY, wheel_ext: [0.1; 4], steer_smooth: 0.2, roll: 3.0 };
        let g = GhostTrack { poses: vec![pose(0.0), pose(16.0)], lap_ms: 90_000 };
        let f = GhostTrack::parse(&g.to_bytes(7, "Red Rock")).unwrap();
        assert_eq!((f.car, f.track_name.as_str(), f.track.lap_ms, f.track.poses.len()), (7, "Red Rock", 90_000, 2));
        assert!(f.track.pose_at(8.0).is_some() && f.track.pose_at(20.0).is_none());
    }

    fn sample() -> Recording {
        let header = Header {
            track: 2,
            laps: 3,
            catchup: true,
            rules: Rules::Normal,
            manual: true,
            row: AiRow::Difficulty(Difficulty::Hard),
            player: Some(7),
            cars: vec![1, 2, 3],
            track_name: "NeoCity".into(),
            nick: "Ann".into(),
            car_name: "Kobra X3".into(),
            drivers: vec!["Abe".into(), "Okkun".into(), String::new()],
            tick_hz: Some(60.0),
            race_ms: Some(91_234),
            best_lap_ms: None,
        };
        let mut rec = Recording::new(header);
        rec.push(0.016, Input { throttle: 1.0, steer: -0.5, shift_up: true, ..Default::default() }, None, false);
        rec.push(0.017, Input { brake: 0.25, handbrake: true, ..Default::default() }, Some(7), true);
        rec
    }

    #[test]
    fn beginner_round_trip() {
        let mut rec = sample();
        rec.header.row = AiRow::Difficulty(Difficulty::Beginner);
        assert_eq!(Recording::from_bytes(&rec.to_bytes()).unwrap(), rec);
    }

    #[test]
    fn round_trip() {
        let rec = sample();
        assert_eq!(Recording::from_bytes(&rec.to_bytes()).unwrap(), rec);
    }

    /// The slot a frame's input drives (hand driving, #60) sits in the spare bits of the input byte
    /// and does not disturb the other bits; frames without it read as the human's car.
    #[test]
    fn hand_slot_round_trips() {
        let mut rec = sample();
        for hand in [None, Some(0), Some(3), Some(7)] {
            let mut f = Frame { dt: 0.02, input: Input { throttle: 0.5, shift_down: true, handbrake: true, ..Default::default() }, respawn: Some(2), skip: true, hand };
            rec.frames.push(f);
            f.skip = false;
            rec.frames.push(f);
        }
        let back = Recording::from_bytes(&rec.to_bytes()).unwrap();
        assert_eq!(back, rec);
        assert_eq!(back.frames[0].hand, None);
        assert_eq!(back.frames.last().unwrap().hand, Some(7));
    }

    /// Needs the game install (skipped without it): hand-driving an AI car changes its path, the
    /// human's car coasts, and `step_as(None)` is `step`.
    #[test]
    fn hand_driving_an_ai_car() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../installed_game");
        if !root.exists() {
            return;
        }
        let mut rec = sample();
        rec.header.cars = vec![1, 2, 3, 4];
        rec.header.player = Some(3);
        rec.frames.clear();
        let (mut ai, mut hand, mut plain) = (rec.load_race(&root).unwrap(), rec.load_race(&root).unwrap(), rec.load_race(&root).unwrap());
        for r in [&mut ai, &mut hand, &mut plain] {
            r.skip_intro();
        }
        let throttle = Input { throttle: 1.0, ..Default::default() };
        for i in 0..1800 {
            let dt = 1.0 / 60.0;
            if i < 600 {
                // the countdown and the start
                ai.step(dt, Input::default());
                plain.step_as(dt, Input::default(), None);
                hand.step_as(dt, Input::default(), Some(1));
            } else {
                ai.step(dt, throttle);
                plain.step_as(dt, throttle, None);
                hand.step_as(dt, Input { steer: 1.0, ..Default::default() }, Some(1));
            }
        }
        assert_eq!(ai.racers[3].car.pos, plain.racers[3].car.pos, "step_as(None) is step");
        // slot 1 followed no line any more, the player's car stopped, the others raced on
        assert_ne!(ai.racers[1].car.pos, hand.racers[1].car.pos);
        assert!(hand.racers[3].car.kmh() < ai.racers[3].car.kmh());
    }

    /// Bytes from the end of the header (race ms, best lap, frame count) back to the driver count.
    fn tail(rec: &Recording) -> usize {
        rec.frames.len() * FRAME_BYTES + 12
    }

    #[test]
    fn mhr3_keeps_the_tick() {
        for hz in [Some(60.0), Some(144.5), None] {
            let mut rec = sample();
            rec.header.tick_hz = hz;
            let b = rec.to_bytes();
            assert_eq!(&b[..4], b"MHR3");
            assert_eq!(Recording::from_bytes(&b).unwrap(), rec);
        }
    }

    #[test]
    fn mhr2_files_still_load_on_the_per_frame_path() {
        let rec = sample();
        let mut b = rec.to_bytes();
        // an MHR2 file: the old magic and no tick rate (the f32 before race ms, best lap, frame count)
        b.drain(b.len() - tail(&rec) - 4..b.len() - tail(&rec));
        b[3] = b'2';
        let old = Recording::from_bytes(&b).unwrap();
        assert_eq!(old.header.tick_hz, None);
        assert_eq!(old, Recording { header: Header { tick_hz: None, ..rec.header.clone() }, ..rec });
    }

    #[test]
    fn mhr1_files_still_load() {
        let mut rec = sample();
        rec.header.drivers.clear();
        rec.header.tick_hz = None;
        let mut b = rec.to_bytes();
        // an MHR1 file: the old magic, no tick rate and no driver count (the bytes before race ms, best lap, frame count)
        let end = b.len() - tail(&rec);
        b.drain(end - 4..end);
        b.remove(end - 5);
        b[3] = b'1';
        assert_eq!(Recording::from_bytes(&b).unwrap(), rec);
    }

    #[test]
    fn rejects_damage() {
        let b = sample().to_bytes();
        assert!(Recording::from_bytes(&b[..b.len() - 1]).is_err());
        assert!(Recording::from_bytes(b"nope").is_err());
    }

    /// Replaying the frames in a second race gives the same state as the live one.
    /// Needs the game install (skipped without it).
    #[test]
    fn replay_matches_live_race() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../installed_game");
        if !root.exists() {
            return;
        }
        let mut rec = sample();
        rec.header.cars = vec![1, 2, 3, 4];
        rec.header.player = Some(3);
        rec.header.race_ms = None;
        rec.frames.clear();
        let mut live = rec.load_race(&root).unwrap();
        for i in 0..1800 {
            let t = i as f32 * 0.05;
            let input = Input { throttle: 1.0, steer: (t.sin() * 0.4), shift_up: i % 97 == 0, ..Default::default() };
            let respawn = (i == 1500).then_some(3);
            rec.push(0.016 + (i % 3) as f32 * 0.004, input, respawn, i == 100);
            let f = *rec.frames.last().unwrap();
            Recording::step(&mut live, &f);
        }
        let rec = Recording::from_bytes(&rec.to_bytes()).unwrap();
        let mut again = rec.load_race(&root).unwrap();
        for f in &rec.frames {
            Recording::step(&mut again, f);
        }
        for (a, b) in live.racers.iter().zip(&again.racers) {
            assert_eq!(a.car.pos, b.car.pos);
            assert_eq!(a.progress.lap, b.progress.lap);
        }
        // the ghost track ends where the live car did, and `pose_at` finds that frame
        let g = rec.ghost(&root).unwrap();
        let last = g.poses.last().unwrap();
        assert_eq!(last.pos, live.racers[3].car.pos);
        assert_eq!(g.pose_at(last.clock_ms).unwrap().pos, last.pos);
    }

    /// #374: a recording made on the fixed tick replays to the same state at any frame length (the tick itself
    /// is the same at every rate: session::tick_tests). Needs the game install (skipped without it).
    #[test]
    fn tick_replay_matches_live_race() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../installed_game");
        if !root.exists() {
            return;
        }
        for (name, frame_s) in [("30 fps", 1.0 / 30.0), ("60 fps", 1.0 / 60.0), ("144 fps", 1.0 / 144.0)] {
            let mut rec = sample();
            rec.header.cars = vec![1, 2, 3, 4];
            rec.header.player = Some(3);
            rec.header.race_ms = None;
            rec.header.tick_hz = Some(60.0);
            rec.frames.clear();
            let mut live = rec.load_race(&root).unwrap();
            assert_eq!(live.fixed_hz(), Some(60.0), "load_race turns the tick on");
            let frames = (30.0 / frame_s) as usize;
            for i in 0..frames {
                let t = i as f32 * frame_s;
                let input = Input { throttle: 1.0, steer: (t * 1.2).sin() * 0.4, shift_up: i % 97 == 0, ..Default::default() };
                rec.push(frame_s, input, None, i == 10);
                let f = *rec.frames.last().unwrap();
                Recording::step(&mut live, &f);
            }
            let rec = Recording::from_bytes(&rec.to_bytes()).unwrap();
            let mut again = rec.load_race(&root).unwrap();
            for f in &rec.frames {
                Recording::step(&mut again, f);
            }
            for (a, b) in live.racers.iter().zip(&again.racers) {
                assert_eq!((a.car.pos, a.car.rot, a.car.vel), (b.car.pos, b.car.rot, b.car.vel), "{name}");
            }
            assert_eq!(live.clock_ms, again.clock_ms, "{name}");
            // the ghost has a pose per tick, not per frame
            let g = rec.ghost(&root).unwrap();
            assert!(g.poses.len() <= 30 * 60 + 2, "{name}: {} poses", g.poses.len());
        }
    }

    #[test]
    fn ghost_pose_interpolates() {
        let pose = |ms: f64, x: f32| GhostPose { clock_ms: ms, pos: Vec3::new(x, 0.0, 0.0), rot: Mat3::IDENTITY, wheel_ext: [0.5; 4], steer_smooth: 0.0, roll: x };
        let g = GhostTrack { poses: vec![pose(0.0, 0.0), pose(100.0, 10.0), pose(200.0, 30.0)], lap_ms: 200 };
        assert!(g.pose_at(-1.0).is_none() && g.pose_at(201.0).is_none());
        assert_eq!(g.pose_at(50.0).unwrap().pos.x, 5.0);
        assert_eq!(g.pose_at(150.0).unwrap().pos.x, 20.0);
        assert_eq!(g.pose_at(200.0).unwrap().pos.x, 30.0);
    }

    #[test]
    fn ghost_file_round_trip() {
        let pose = |ms: f64, x: f32| GhostPose { clock_ms: ms, pos: Vec3::new(x, 1.0, 2.0), rot: Mat3::from_rotation_y(0.3), wheel_ext: [0.1, 0.2, 0.3, 0.4], steer_smooth: 0.5, roll: x * 2.0 };
        let g = GhostTrack { poses: vec![pose(0.0, 0.0), pose(16.0, 1.0)], lap_ms: 90_000 };
        let (b, car) = GhostTrack::from_bytes(&g.to_bytes(7, "NeoCity")).unwrap();
        assert_eq!((car, b.lap_ms, b.poses.len()), (7, 90_000, 2));
        assert_eq!(b.poses[1].pos, g.poses[1].pos);
        assert!((b.poses[1].rot.y_axis - g.poses[1].rot.y_axis).length() < 1e-5);
        assert!(GhostTrack::from_bytes(&g.to_bytes(7, "x")[..30]).is_err());
    }

    /// The recorder samples at most every 16 ms, closes a lap at its time and starts the next empty.
    /// Needs the game install (skipped without it).
    #[test]
    fn lap_recorder_splits_laps() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../installed_game");
        if !root.exists() {
            return;
        }
        let mut h = sample().header;
        (h.cars, h.player) = (vec![1], Some(0));
        let mut race = Recording::new(h).load_race(&root).unwrap();
        let mut rec = LapRecorder::default();
        for i in 0..100 {
            race.step(0.008, Input::default());
            rec.push(i as f64 * 8.0, 0.008, &race.racers[0].car);
        }
        let lap = rec.finish(800, 0.008, &race.racers[0].car);
        assert_eq!(lap.lap_ms, 800);
        assert!(lap.poses.len() <= 52 && lap.poses.len() >= 48, "{}", lap.poses.len());
        assert_eq!(lap.poses.last().unwrap().clock_ms, 800.0);
        assert!(lap.pose_at(400.0).is_some());
        assert_eq!(rec.finish(10, 0.008, &race.racers[0].car).poses.len(), 1);
    }
}
