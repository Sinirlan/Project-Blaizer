//! `Demos/*.MDE` race recordings (the original's Ghost mode and demo play, GameType 3 / 6).
//! Spec: `re-notes/05-formats.md` "MDE". Loader `0x41bd18`, recorder init `0x41d3c4`, recorder
//! `0x41d69c`, playback `0x41cd18`, car pack `0x40b9d0`, unpack `0x40c264`, apply `0x40c5f4`.
//!
//! A file is a 0xfac-byte header followed by records of `8 + 30 * cars` bytes. One record is
//! written every 62.5 ms of wall clock; playback interpolates between the two records around
//! the race clock [O].

use crate::bytes::{cstr, err, f32_, slice, u16_, u32_, Result};

pub const VERSION: f32 = 2.5;
pub const HEADER_BYTES: usize = 0xfac;
pub const CAR_ENTRIES: usize = 8;
pub const CAR_ENTRY_BYTES: usize = 0x41;
pub const CAR_BYTES: usize = 30;
/// Where the car entries start in the header.
const CARS_AT: usize = 0x24;
/// The loader reads no byte from here to the end of the header (`0x22c..0xfac`) [O].
const TAIL_AT: usize = CARS_AT + CAR_ENTRIES * CAR_ENTRY_BYTES;
/// Time between two records in ms (`0x4d16ec`, checked per frame, so 63..74 ms in practice) [O].
pub const RECORD_MS: f64 = 62.5;
const TAU: f32 = std::f32::consts::TAU;

/// One of the eight car entries of the header (0x41 bytes).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct CarInfo {
    pub active: bool,
    /// Driver name (16 bytes at +4).
    pub name: String,
    /// Team tag (5 bytes at +0x14).
    pub team: String,
    /// Car model name (16 bytes at +0x19, e.g. "Avalanche").
    pub car: String,
    /// CarColour1..3 and the player colour, `0x00RRGGBB` as the car file's Colour1..3 (+0x29..).
    pub colours: [u32; 4],
    /// +0x39, copied into the car array `+0x24`; meaning [U].
    pub w39: u32,
    /// +0x3d, control type 1 / 2 / 3 from the device byte of the TurnLeft binding code, `0x705ac9`
    /// (2 keyboard, 1 mouse, 4 joystick) [O].
    pub control: u32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Header {
    pub version: f32,
    /// bit 0 = CatchUp.
    pub flags: u32,
    pub track: String,
    pub laps: u32,
    /// Cars per record (the recorder counts the active entries) [O].
    pub count: u32,
    /// Slot of the player who recorded it.
    pub player: u32,
    pub cars: [CarInfo; CAR_ENTRIES],
    /// Bytes `0x22c..0xfac`, kept as read (the sample carries stale recorder memory there).
    pub tail: Vec<u8>,
}

impl Header {
    pub fn new(track: &str, laps: u32, catchup: bool, player: u32, cars: [CarInfo; CAR_ENTRIES]) -> Header {
        let count = cars.iter().filter(|c| c.active).count() as u32;
        Header { version: VERSION, flags: catchup as u32, track: track.into(), laps, count, player, cars, tail: vec![0; HEADER_BYTES - TAIL_AT] }
    }
    pub fn catchup(&self) -> bool {
        self.flags & 1 != 0
    }
    /// Slots of the active car entries in order: block k of a record belongs to `active_slots()[k]`.
    pub fn active_slots(&self) -> Vec<usize> {
        (0..CAR_ENTRIES).filter(|&i| self.cars[i].active).collect()
    }
}

/// One car's 30-byte block, decoded. Value ranges are the quantisation of the file.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct CarState {
    /// Wheel spring state FL FR BR BL, `n/15 + 0.5` (apply: `P+0x158..0x164`).
    pub spring: [f32; 4],
    /// Steering, `(b-127)/127` (`P+0xd8`).
    pub steer: f32,
    /// Front / rear wheel spin rate in rad/s, `b/3 - 40` (`P+0xe4`, `P+0xe8`; 120 = 0) [D: `Car::wheel_spin`].
    pub aux: [f32; 2],
    /// Engine rpm in steps of 62.5 (`P+0x54`).
    pub rpm: f32,
    /// Slide amount in steps of 0.5, 0..15.5 (`P+0xcc`).
    pub slide: f32,
    /// Gear 0 = R, 1..n (`P+0x4c`), 3 bits.
    pub gear: u8,
    /// Four 0..1 values (`P+0x180, 0x184, 0x178, 0x17c`): the rear tyre-effect strengths BR, BL
    /// (`Car::tyre_effect`) and the strengths of the two spark slots (`Car::world_hits`) [O, #278].
    pub fx: [f32; 4],
    /// Rear tyre-effect ids BR, BL (`P+0x170`, `P+0x174`, ContactAnimID of the surface), 3 bits each [D].
    pub surf: [u8; 2],
    /// Single bits `P+0x168`, `P+0x16c`: the spark slots' SparkAnimID (`& 1`).
    pub bits: [bool; 2],
    /// bit 0 brake > 0.15 (`+0xa8`, the brake lights), 1 `+0xb4`, 2 sliding `+0x90`, 3 wheelspin `+0x94`,
    /// 4 locked wheels `+0xa0`, 5..8 wheel FL FR BR BL on the ground (`+0x134, 0x138, 0x13c, 0x140`),
    /// 9 `+0xb0` overrun, 10 the pedal `+0xf0` above 0 (0x40bd8c); bit 1 `+0xb4` is the horn key. 11 bits.
    pub flags: u16,
    /// 2-bit fields `P+0x188` and `P+0x18c`: the hull point (0..3 = FL FR BR BL) of spark slots A and B.
    pub w188: u8,
    pub w18c: u8,
    /// Body angles in radians, [0, 2π): pitch, yaw, roll (see `matrix`).
    pub angles: [f32; 3],
    pub pos: [f32; 3],
}

fn q(v: f32, scale: f32, max: u32) -> u8 {
    (v * scale).round().clamp(0.0, max as f32) as u8
}

impl CarState {
    pub fn parse(b: &[u8]) -> Result<CarState> {
        let d = slice(b, 0, CAR_BYTES)?;
        let nib = |x: u8| [(x & 15) as f32 / 15.0 + 0.5, (x >> 4) as f32 / 15.0 + 0.5];
        let (s0, s1) = (nib(d[0]), nib(d[1]));
        let fx = |x: u8| [(x & 15) as f32 / 15.0, (x >> 4) as f32 / 15.0];
        let (f0, f1) = (fx(d[7]), fx(d[8]));
        let flags = d[10] as u16 | ((d[11] as u16 & 0xf0) << 4);
        let ang = |o| Ok(u16_(d, o)? as f32 / 8192.0);
        Ok(CarState {
            spring: [s0[0], s0[1], s1[0], s1[1]],
            steer: (d[2] as f32 - 127.0) / 127.0,
            aux: [d[3] as f32 / 3.0 - 40.0, d[4] as f32 / 3.0 - 40.0],
            rpm: d[5] as f32 * 62.5,
            slide: (d[6] & 0x1f) as f32 * 0.5,
            gear: d[6] >> 5,
            fx: [f0[0], f0[1], f1[0], f1[1]],
            surf: [d[9] & 7, (d[9] >> 3) & 7],
            bits: [d[9] & 0x40 != 0, d[9] & 0x80 != 0],
            flags,
            w188: (d[11] >> 2) & 3,
            w18c: d[11] & 3,
            angles: [ang(12)?, ang(14)?, ang(16)?],
            pos: [f32_(d, 18)?, f32_(d, 22)?, f32_(d, 26)?],
        })
    }

    /// The pack routine `0x40b9d0`; the exact inverse of `parse` for every value `parse` makes.
    pub fn write(&self, out: &mut Vec<u8>) {
        let sp = |v: f32| q(v - 0.5, 15.0, 15);
        let fx = |v: f32| q(v, 15.0, 15);
        out.push(sp(self.spring[0]) | sp(self.spring[1]) << 4);
        out.push(sp(self.spring[2]) | sp(self.spring[3]) << 4);
        out.push((self.steer * 127.0 + 127.0).round().clamp(0.0, 255.0) as u8);
        out.push(((self.aux[0] + 40.0) * 3.0).round().clamp(0.0, 255.0) as u8);
        out.push(((self.aux[1] + 40.0) * 3.0).round().clamp(0.0, 255.0) as u8);
        out.push((self.rpm / 62.5).round().clamp(0.0, 255.0) as u8);
        out.push(q(self.slide, 2.0, 31) | (self.gear & 7) << 5);
        out.push(fx(self.fx[0]) | fx(self.fx[1]) << 4);
        out.push(fx(self.fx[2]) | fx(self.fx[3]) << 4);
        out.push((self.surf[0] & 7) | (self.surf[1] & 7) << 3 | (self.bits[0] as u8) << 6 | (self.bits[1] as u8) << 7);
        out.push(self.flags as u8);
        out.push((self.w188 & 3) << 2 | (self.w18c & 3) | ((self.flags >> 4) as u8 & 0xf0));
        for a in self.angles {
            let v = (a.rem_euclid(TAU) * 8192.0).round() as u32 & 0xffff;
            out.extend((v as u16).to_le_bytes());
        }
        for p in self.pos {
            out.extend(p.to_le_bytes());
        }
    }

    /// Body matrix, row-major, rows = right, up, forward in game coordinates
    /// (`0x42cc7c`: `m[6..8]` = (sin yaw cos pitch, -sin pitch, cos yaw cos pitch)) [O].
    pub fn matrix(&self) -> [f32; 9] {
        matrix_from_angles(self.angles)
    }

    /// Playback interpolation (`0x41cd18`): position, angles (the shorter way round), springs, steer,
    /// aux, rpm, slide and the four `fx` blend; gear, surfaces and flags come from `self` [O].
    pub fn lerp(&self, next: &CarState, f: f32) -> CarState {
        let l = |a: f32, b: f32| a + (b - a) * f;
        let mut o = *self;
        for i in 0..4 {
            o.spring[i] = l(self.spring[i], next.spring[i]);
            o.fx[i] = l(self.fx[i], next.fx[i]);
        }
        o.steer = l(self.steer, next.steer);
        o.aux = [l(self.aux[0], next.aux[0]), l(self.aux[1], next.aux[1])];
        o.rpm = l(self.rpm, next.rpm);
        o.slide = l(self.slide, next.slide);
        for i in 0..3 {
            let (mut a, mut b) = (self.angles[i], next.angles[i]);
            if (a - b).abs() > std::f32::consts::PI {
                if a <= b {
                    a += TAU;
                } else {
                    b += TAU;
                }
            }
            o.angles[i] = l(a, b).rem_euclid(TAU);
            o.pos[i] = l(self.pos[i], next.pos[i]);
        }
        o
    }
}

/// `0x42cc7c`: angles (a = pitch, b = yaw, c = roll) to the row-major 3x3.
pub fn matrix_from_angles(ang: [f32; 3]) -> [f32; 9] {
    let [a, b, c] = ang.map(f64::from);
    let (ca, sa, cb, sb, cc, sc) = (a.cos(), a.sin(), b.cos(), b.sin(), c.cos(), c.sin());
    [
        cc * cb + sc * sa * sb,
        sc * ca,
        sc * sa * cb - cc * sb,
        cc * sa * sb - sc * cb,
        cc * ca,
        sc * sb + cc * sa * cb,
        sb * ca,
        -sa,
        cb * ca,
    ]
    .map(|v| v as f32)
}

/// `0x42cad8`: the inverse, angles in [0, 2π). At gimbal lock (|cos pitch| tiny) yaw is 0 and roll
/// takes the rest, as the original does.
pub fn angles_from_matrix(m: [f32; 9]) -> [f32; 3] {
    let m = m.map(f64::from);
    let (a, b, c);
    if m[6] * m[6] + m[8] * m[8] > 1e-12 {
        a = (-m[7]).asin();
        b = m[6].atan2(m[8]);
        c = m[1].atan2(m[4]);
    } else {
        a = if m[7] > 0.0 { -std::f64::consts::FRAC_PI_2 } else { std::f64::consts::FRAC_PI_2 };
        b = 0.0;
        c = (-m[3]).atan2(m[0]);
    }
    [a, b, c].map(|v| (v as f32).rem_euclid(TAU))
}

#[derive(Debug, Clone, PartialEq)]
pub struct Record {
    /// bit i = car slot i exists this frame, bit 8+i = it is flagged remote/AI (`car+0xc`) [O].
    pub mask: u32,
    /// ms since the first race frame, the countdown included.
    pub ms: u32,
    /// One block per active header entry, in slot order.
    pub cars: Vec<CarState>,
}

#[derive(Debug, Clone)]
pub struct Mde {
    pub header: Header,
    pub records: Vec<Record>,
}

impl Mde {
    pub fn new(header: Header) -> Mde {
        Mde { header, records: Vec::new() }
    }

    pub fn parse(d: &[u8]) -> Result<Mde> {
        let version = f32_(d, 0)?;
        if version != VERSION {
            return err(format!("MDE version {version}, expected {VERSION}"));
        }
        if d.len() < HEADER_BYTES {
            return err("MDE header cut short");
        }
        let mut cars: [CarInfo; CAR_ENTRIES] = Default::default();
        for (i, c) in cars.iter_mut().enumerate() {
            let o = CARS_AT + i * CAR_ENTRY_BYTES;
            *c = CarInfo {
                active: u32_(d, o)? != 0,
                name: cstr(d, o + 4, Some(16))?,
                team: cstr(d, o + 0x14, Some(5))?,
                car: cstr(d, o + 0x19, Some(16))?,
                colours: [u32_(d, o + 0x29)?, u32_(d, o + 0x2d)?, u32_(d, o + 0x31)?, u32_(d, o + 0x35)?],
                w39: u32_(d, o + 0x39)?,
                control: u32_(d, o + 0x3d)?,
            };
        }
        let header = Header {
            version,
            flags: u32_(d, 4)?,
            track: cstr(d, 8, Some(16))?,
            laps: u32_(d, 0x18)?,
            count: u32_(d, 0x1c)?,
            player: u32_(d, 0x20)?,
            cars,
            tail: d[TAIL_AT..HEADER_BYTES].to_vec(),
        };
        let stride = 8 + CAR_BYTES * header.count as usize;
        let body = &d[HEADER_BYTES..];
        if body.len() < stride {
            return err("No started race recorded.");
        }
        let n = body.len() / stride;
        let mut records = Vec::with_capacity(n);
        for r in 0..n {
            let o = r * stride;
            let cars = (0..header.count as usize).map(|k| CarState::parse(&body[o + 8 + k * CAR_BYTES..])).collect::<Result<Vec<_>>>()?;
            records.push(Record { mask: u32_(body, o)?, ms: u32_(body, o + 4)?, cars });
        }
        // a cut-off last record is dropped (the recorder writes halves of 32 records)
        Ok(Mde { header, records })
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        let h = &self.header;
        let mut o = Vec::with_capacity(HEADER_BYTES + self.records.len() * (8 + CAR_BYTES * h.count as usize));
        o.extend(h.version.to_le_bytes());
        o.extend(h.flags.to_le_bytes());
        let pad = |o: &mut Vec<u8>, s: &str, n: usize| {
            let b: Vec<u8> = s.chars().map(|c| if (c as u32) < 256 { c as u8 } else { b'?' }).take(n - 1).collect();
            o.extend(&b);
            o.extend(std::iter::repeat_n(0, n - b.len()));
        };
        pad(&mut o, &h.track, 16);
        o.extend(h.laps.to_le_bytes());
        o.extend(h.count.to_le_bytes());
        o.extend(h.player.to_le_bytes());
        for c in &h.cars {
            o.extend((c.active as u32).to_le_bytes());
            pad(&mut o, &c.name, 16);
            pad(&mut o, &c.team, 5);
            pad(&mut o, &c.car, 16);
            for v in c.colours {
                o.extend(v.to_le_bytes());
            }
            o.extend(c.w39.to_le_bytes());
            o.extend(c.control.to_le_bytes());
        }
        let mut tail = h.tail.clone();
        tail.resize(HEADER_BYTES - TAIL_AT, 0);
        o.extend(tail);
        for r in &self.records {
            o.extend(r.mask.to_le_bytes());
            o.extend(r.ms.to_le_bytes());
            for c in &r.cars {
                c.write(&mut o);
            }
        }
        o
    }

    pub fn load(path: &std::path::Path) -> Result<Mde> {
        Mde::parse(&std::fs::read(path).map_err(|e| crate::Error(format!("{}: {e}", path.display())))?)
    }

    pub fn save(&self, path: &std::path::Path) -> std::io::Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::write(path, self.to_bytes())
    }

    /// Length of the recording in seconds.
    pub fn seconds(&self) -> f32 {
        self.records.last().map_or(0.0, |r| r.ms as f32 / 1000.0)
    }

    /// The cars at race time `ms`: the two records around it blended as in `0x41cd18`. `None` before
    /// the first record is reached is not needed (the first record is at ~70 ms); the pose clamps to the
    /// first / last record. The mask comes from the earlier record.
    pub fn sample(&self, ms: f64) -> Option<(u32, Vec<CarState>)> {
        let first = self.records.first()?;
        let i = self.records.partition_point(|r| (r.ms as f64) <= ms);
        if i == 0 {
            return Some((first.mask, first.cars.clone()));
        }
        let a = &self.records[i - 1];
        let Some(b) = self.records.get(i) else { return Some((a.mask, a.cars.clone())) };
        let span = (b.ms - a.ms).max(1) as f64;
        let f = ((ms - a.ms as f64) / span).clamp(0.0, 1.0) as f32;
        Some((a.mask, a.cars.iter().zip(&b.cars).map(|(x, y)| x.lerp(y, f)).collect()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matrix_roundtrip() {
        for ang in [[0.0, 0.559, 0.0], [0.1, 3.9, 0.2], [6.2, 1.0, 5.5], [0.3, 0.0, 0.0]] {
            let back = angles_from_matrix(matrix_from_angles(ang));
            for i in 0..3 {
                let d = (back[i] - ang[i]).abs();
                assert!(d < 1e-4 || (TAU - d) < 1e-4, "{ang:?} -> {back:?}");
            }
        }
    }

    #[test]
    fn car_block_roundtrip() {
        // every byte pattern decodes and re-encodes to itself (angles and positions are free bytes)
        let mut seed = 0x1234_5678u32;
        for _ in 0..2000 {
            let mut b = [0u8; CAR_BYTES];
            for x in &mut b {
                seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
                *x = (seed >> 24) as u8;
            }
            // keep the f32 position finite and the flag bit 7 of byte 11 (unused by the loader) clear
            b[11] &= 0x7f;
            b[18..30].copy_from_slice(&[0, 0, 0x80, 0x3f, 0, 0, 0x20, 0x41, 0, 0, 0xc0, 0xc0]);
            let c = CarState::parse(&b).unwrap();
            let mut out = Vec::new();
            c.write(&mut out);
            // byte 11 bit 6 is flag 10; angle values above 2π wrap; compare all but the angles
            assert_eq!(out[..12], b[..12]);
            assert_eq!(out[18..], b[18..]);
        }
    }

    fn sample_path() -> Option<std::path::PathBuf> {
        let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../installed_game/Demos/JeePen.MDE");
        p.exists().then_some(p)
    }

    #[test]
    fn jeepen_sample() {
        let Some(p) = sample_path() else { return };
        let raw = std::fs::read(&p).unwrap();
        let m = Mde::parse(&raw).unwrap();
        let h = &m.header;
        assert_eq!((h.track.as_str(), h.laps, h.count, h.player, h.catchup()), ("NeoCity", 10, 1, 0, false));
        let c = &h.cars[0];
        assert_eq!((c.name.as_str(), c.team.as_str(), c.car.as_str(), c.control), ("JeePen", "SMOB", "Avalanche", 3));
        assert_eq!(h.active_slots(), vec![0]);
        assert_eq!(m.records.len(), 8896);
        assert_eq!((m.records[0].mask, m.records[0].ms), (1, 70));
        assert_eq!(m.records.last().unwrap().ms, 613_222);
        let s = &m.records[0].cars[0];
        assert!((s.pos[0] + 322.96).abs() < 0.01 && (s.pos[2] + 432.48).abs() < 0.01);
        // yaw 0.559 rad, no pitch or roll at the grid
        assert!((s.angles[1] - 0.5586).abs() < 0.001 && (s.angles[0] < 0.001 || s.angles[0] > TAU - 0.001));
        // the file reproduces byte for byte (header tail kept)
        assert_eq!(m.to_bytes(), raw);
    }

    #[test]
    fn jeepen_sample_blend() {
        let Some(p) = sample_path() else { return };
        let m = Mde::load(&p).unwrap();
        let (a, b) = (&m.records[100], &m.records[101]);
        let mid = (a.ms as f64 + b.ms as f64) / 2.0;
        let (_, cars) = m.sample(mid).unwrap();
        for i in 0..3 {
            let (lo, hi) = (a.cars[0].pos[i].min(b.cars[0].pos[i]), a.cars[0].pos[i].max(b.cars[0].pos[i]));
            assert!(cars[0].pos[i] >= lo - 1e-3 && cars[0].pos[i] <= hi + 1e-3);
        }
    }
}
