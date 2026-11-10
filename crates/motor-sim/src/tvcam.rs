//! The TV camera (F3): trackside shots along the track's `CameraSplineName` motion, as the
//! original's `0x45741c` picks them [O]. The motion's keys are repurposed: channels 0..2 are
//! the camera station, 3..8 shot parameters, and the four numbers of the key's second line
//! select the shot type and the zoom (`0x456fdc` stores them at key +4..+0x10).

use glam::{Mat3, Vec3};
use crate::World;
use motor_data::cfg::{parse_lwmo, Config, Motion};
use std::path::{Path, PathBuf};

/// Degrees to radians as the original multiplies them (`0x4d491e` · `0x4d4926`).
const DEG: f32 = std::f32::consts::PI / 180.0;
/// Shot types (`0x4b7b34`).
const KINDS: i64 = 7;
motor_tune::tunable! {
    /// TV camera: the smallest zoom (`0x40b498`) [O].
    /// range: 0.1..5
    ZOOM_MIN: f32 = 0.5;
    /// TV camera: the largest zoom (`0x40b498`) [O].
    /// range: 1..40
    ZOOM_MAX: f32 = 10.0;
    /// TV camera: the nearest the far plane may come when the shot scales it (`0x40b564`) [O].
    /// range: 10..1000
    /// unit: m
    pub FAR_MIN: f32 = 60.0;
    /// TV camera: the farthest the far plane may go when the shot scales it (`0x40b564`) [O]; Remaster keeps a longer view-distance option.
    /// range: 10..2000
    /// unit: m
    pub FAR_MAX: f32 = 300.0;
}

#[derive(Debug, Clone)]
pub struct TvKey {
    pub pos: Vec3,
    /// channels 3..8
    pub c: [f32; 6],
    /// shot type 0..6 = `floor(linear + 0.5) mod 7` (`0x4574ce`)
    pub kind: u8,
    /// zoom per metre (key +8)
    pub zoom: f32,
    /// far-plane growth per zoomed metre, percent (key +0xc)
    pub far: f32,
    /// distance cap for the zoom (key +0x10)
    pub cap: f32,
}

/// One frame of the TV camera.
#[derive(Debug, Clone, Copy)]
pub struct TvShot {
    pub eye: Vec3,
    /// columns right, up, forward, game space
    pub rot: Mat3,
    /// 1.0 = the normal 90 degree horizontal field of view
    pub zoom: f32,
    /// multiplier of the race's far plane
    pub far_k: f32,
    /// the shot type that produced it
    pub kind: u8,
}

/// Keys of a camera spline; channels beyond the ninth are ignored.
pub fn keys(m: &Motion) -> Vec<TvKey> {
    m.keys
        .iter()
        .filter(|k| k.values.len() >= 9)
        .map(|k| TvKey {
            pos: Vec3::new(k.values[0], k.values[1], k.values[2]),
            c: [k.values[3], k.values[4], k.values[5], k.values[6], k.values[7], k.values[8]],
            kind: ((k.linear + 0.5) as i64).rem_euclid(KINDS) as u8,
            zoom: k.tension,
            far: k.continuity,
            cap: k.bias,
        })
        .collect()
}

/// Where an exported TV station file lives (#424): `<install>/remaster/tv/trackNN.mot`, an LWMO like the shipped one.
pub fn override_path(root: &Path, track: u32) -> PathBuf {
    root.join(format!("remaster/tv/track{track:02}.mot"))
}

/// The track's shipped TV motion (`CameraSplineName`, 0x45ea6d) [O]; `trk` is its `TrackN.trk`.
pub fn shipped_motion(root: &Path, trk: &Config) -> Option<Motion> {
    let name = trk.get("CameraSplineName")?.values.first()?.clone();
    let t = motor_data::tbl::load(&motor_data::fs::read(root, &name).ok()?, None).ok()?;
    parse_lwmo(&t.data).ok()
}

/// The TV stations of a track. REMASTER-era design choice (#424): a user file `remaster/tv/trackNN.mot` (written by
/// motorview's cine export) replaces the shipped motion when it parses; without it nothing differs from the original.
pub fn load(root: &Path, track: u32, trk: &Config) -> Vec<TvKey> {
    let own = std::fs::read(override_path(root, track)).ok().and_then(|b| parse_lwmo(&b).ok());
    own.or_else(|| shipped_motion(root, trk)).map(|m| keys(&m)).unwrap_or_default()
}

/// Rotation from the game's Euler angles `(pitch, heading, bank)` (`0x42cc7c`).
pub fn euler_to_rot(e: Vec3) -> Mat3 {
    let (sa, ca) = e.x.sin_cos();
    let (sb, cb) = e.y.sin_cos();
    let (sc, cc) = e.z.sin_cos();
    let right = Vec3::new(cc * cb + sc * sa * sb, sc * ca, sc * sa * cb - cc * sb);
    let up = Vec3::new(cc * sa * sb - sc * cb, cc * ca, sc * sb + cc * sa * cb);
    let fwd = Vec3::new(sb * ca, -sa, cb * ca);
    Mat3::from_cols(right, up, fwd)
}

/// Inverse of `euler_to_rot` (`0x42cad8`); at the poles the heading is taken as 0.
pub fn rot_to_euler(m: Mat3) -> Vec3 {
    let (right, up, fwd) = (m.x_axis, m.y_axis, m.z_axis);
    let pitch = (-fwd.y).clamp(-1.0, 1.0).asin();
    if (up.y * up.y + right.y * right.y) < 1e-6 {
        return Vec3::new(pitch, 0.0, 0.0);
    }
    Vec3::new(pitch, fwd.x.atan2(fwd.z), right.y.atan2(up.y))
}

/// Look from `eye` at `target` with no roll (`0x42c334`); None if they coincide.
fn look_at(eye: Vec3, target: Vec3) -> Option<Mat3> {
    let f = (target - eye).try_normalize()?;
    let r = Vec3::new(f.z, 0.0, -f.x).try_normalize().unwrap_or(Vec3::X);
    Some(Mat3::from_cols(r, f.cross(r), f))
}

/// The shot for a car at `car` with orientation `car_rot` (columns right, up, forward): the
/// camera station nearest the car (brute force each frame, `0x45604c`), then its shot type.
pub fn shot(keys: &[TvKey], car: Vec3, car_rot: Mat3) -> Option<TvShot> {
    let k = keys.iter().min_by(|a, b| a.pos.distance_squared(car).total_cmp(&b.pos.distance_squared(car)))?;
    let d = car.distance(k.pos);
    let u = if d > 0.0 { (car - k.pos) / d } else { Vec3::ZERO };
    let c = Vec3::new(k.c[0], k.c[1], k.c[2]);
    let c2 = Vec3::new(k.c[3], k.c[4], k.c[5]);
    // slide towards the car: (c3, c4·c3, c3) of the offset (0x45741c case 1: the y factor is
    // c4·c3, and c5 is unused)
    let toward = |s: f32| k.pos + Vec3::new(c.x, c.y * c.x, c.x) * u * s;
    let (eye, rot) = match k.kind {
        0 => (k.pos, look_at(k.pos, car)),
        1 => {
            let eye = toward(d);
            (eye, look_at(eye, car))
        }
        2 => (k.pos, Some(euler_to_rot(c * DEG))),
        // mounted on the car: offset in its frame, its angles plus the key's
        3 => (car + car_rot * c, Some(euler_to_rot(rot_to_euler(car_rot) + c2 * DEG))),
        4 => (car + c, look_at(car + c, car)),
        5 => {
            let a = c.x * d * 0.1;
            let eye = car + Vec3::new(a.sin() * c.y, c.z, a.cos() * c.y);
            (eye, look_at(eye, car))
        }
        _ => {
            let eye = toward(d);
            (eye, Some(euler_to_rot(c2 * DEG)))
        }
    };
    // zoom and far plane from the distance to the car (0x4574c1.., 0x40b498, 0x40b564)
    let r = car.distance(eye).min(k.cap) * k.zoom;
    let r = r.max(1.0);
    // `.max(lo)`: a typed ZOOM_MAX under ZOOM_MIN must not make clamp panic
    let (zoom_lo, zoom_hi) = (ZOOM_MIN.get(), ZOOM_MAX.get());
    let zoom = (if k.zoom < 0.0 { -k.zoom } else { r }).clamp(zoom_lo, zoom_hi.max(zoom_lo));
    let far_k = 1.0 + r * 0.01 * k.far;
    Some(TvShot { eye, rot: rot?, zoom, far_k, kind: k.kind })
}

motor_tune::tunable! {
    /// TV camera wall clearance [design choice]: the gap kept in front of the geometry that hides the car.
    /// range: 0..3
    /// unit: m
    CLEAR_MARGIN: f32 = 0.4;
    /// TV camera wall clearance [design choice]: the closest the eye may come to the car's look-at point.
    /// range: 0..10
    /// unit: m
    CLEAR_MIN: f32 = 1.5;
}

impl TvShot {
    /// APPROX [design choice]: stations and offsets that put the eye behind a wall or under the
    /// road (about a third of the shipped shots over an AI lap) pull the eye along the line to the
    /// car up to the collision mesh, as the chase camera does. The view direction stays and the
    /// zoom shrinks with the distance. Returns whether the eye moved.
    pub fn clear_view(&mut self, world: &World, car: Vec3) -> bool {
        let pivot = car + Vec3::Y;
        let Some(hit) = world.raycast(pivot, self.eye) else { return false };
        let (len, hit_len) = (pivot.distance(self.eye), pivot.distance(hit.point));
        let dir = (self.eye - pivot) / len.max(1e-3);
        let keep = (hit_len - CLEAR_MARGIN.get()).max(CLEAR_MIN.get()).min(len);
        self.eye = pivot + dir * keep;
        if self.zoom > 1.0 {
            self.zoom = (self.zoom * keep / len).max(1.0);
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(kind: u8, pos: Vec3, c: [f32; 6], zoom: f32, cap: f32) -> TvKey {
        TvKey { pos, c, kind, zoom, far: 0.0, cap }
    }

    #[test]
    fn nearest_key_and_look_at() {
        let ks = [key(0, Vec3::new(0.0, 2.0, 0.0), [0.0; 6], 1.0, 50.0), key(0, Vec3::new(100.0, 2.0, 0.0), [0.0; 6], 1.0, 50.0)];
        let s = shot(&ks, Vec3::new(90.0, 0.0, 10.0), Mat3::IDENTITY).unwrap();
        assert_eq!(s.eye, Vec3::new(100.0, 2.0, 0.0));
        let to_car = (Vec3::new(90.0, 0.0, 10.0) - s.eye).normalize();
        assert!(s.rot.z_axis.distance(to_car) < 1e-5);
        assert!(s.rot.y_axis.y > 0.0);
    }

    #[test]
    fn euler_roundtrip() {
        let e = Vec3::new(0.3, -1.2, 0.4);
        assert!(rot_to_euler(euler_to_rot(e)).distance(e) < 1e-4);
        assert!(euler_to_rot(Vec3::ZERO).abs_diff_eq(Mat3::IDENTITY, 1e-6));
    }

    #[test]
    fn zoom_rules() {
        let car = Vec3::new(0.0, 0.0, 40.0);
        // capped distance 10 * 0.3 = 3
        let s = shot(&[key(0, Vec3::ZERO, [0.0; 6], 0.3, 10.0)], car, Mat3::IDENTITY).unwrap();
        assert!((s.zoom - 3.0).abs() < 1e-5);
        // negative zoom is a fixed zoom of -a
        let s = shot(&[key(0, Vec3::ZERO, [0.0; 6], -2.0, 10.0)], car, Mat3::IDENTITY).unwrap();
        assert!((s.zoom - 2.0).abs() < 1e-5);
        // tiny products clamp to 1
        let s = shot(&[key(0, Vec3::ZERO, [0.0; 6], 0.001, 10.0)], car, Mat3::IDENTITY).unwrap();
        assert_eq!(s.zoom, 1.0);
    }

    #[test]
    fn mounted_shot_follows_car() {
        let ks = [key(3, Vec3::ZERO, [0.0, 1.0, 2.0, 0.0, 0.0, 0.0], 1.0, 10.0)];
        let rot = euler_to_rot(Vec3::new(0.0, 0.5, 0.0));
        let s = shot(&ks, Vec3::new(5.0, 0.0, 0.0), rot).unwrap();
        assert!(s.eye.distance(Vec3::new(5.0, 0.0, 0.0) + rot * Vec3::new(0.0, 1.0, 2.0)) < 1e-5);
        assert!(s.rot.z_axis.distance(rot.z_axis) < 1e-4);
    }
}
