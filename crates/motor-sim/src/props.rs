//! Track scene objects from the LightWave scene (`SceneName`, loader 0x455390): animated
//! scenery (wings, wind turbines, airships, cranes, blinkers) and rigid props (cones, signs,
//! barrels). Details and addresses: re-notes/04-track-car-data.md §4.9.
//!
//! Each object is `<BasePath>/Objects/<name>.myo` placed by its motion. Objects with
//! `UnseenByRays` are drawn only. The others get a rigid body with the object's `.COL`
//! (mass = PolygonSize * 100); the game steps it in its world step 0x480fbc (04 §4.9.1; 0x425ae0 is
//! not a solver). The body motion is ported in `body.rs`.
//! The cars meet them in `Race::step`'s substep loop: `begin_substep`, the cars' substeps,
//! `contact`, `finish_substep` (the 0x480fbc order).

use glam::{Mat3, Vec3};
use motor_data::cfg::{self, Motion};
use motor_data::{fs, myo, tbl};
use std::path::Path;

use crate::body::{Body, Dyn, Hull};
use crate::car::Car;
use crate::world::World;

motor_tune::tunable! {
    /// Mass of a prop from its PolygonSize (0x4556f0: x 100.0) [O]. A prop takes it when the track loads.
    /// range: 1..1000
    /// unit: kg
    /// applies: restart
    MASS_PER_POLYGON_SIZE: f32 = 100.0;
}
/// Bodies at least this heavy get flag 1 (0x455ca3); no physics routine reads it, so they are
/// only slow to move (04 §4.9.1).
const HEAVY_MASS: f32 = 10_000.0;

/// Position, rotation and scale of one object (the game's 9 motion channels). The scale
/// does not reach the mesh: 0x47b8ac builds a pure rotation (0x42cc7c) and stores the scale
/// at object +0xec..f4, which the halo routine 0x4354fc multiplies into the halo colour
/// (r, g, b) [O]. That is how blinkers and flashing cones blink.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Pose {
    pub pos: Vec3,
    /// columns: the object's x (right), y (up) and z (forward) axes in world space
    pub rot: Mat3,
    pub scale: Vec3,
}

impl Pose {
    /// x y z heading pitch bank (degrees) sx sy sz, as 0x47b8ac applies them.
    pub fn from_channels(v: &[f32]) -> Pose {
        let c = |i: usize, d: f32| v.get(i).copied().unwrap_or(d);
        let rad = std::f32::consts::PI / 180.0; // 0x4d794e * 0x4d7956
        Pose {
            pos: Vec3::new(c(0, 0.0), c(1, 0.0), c(2, 0.0)),
            rot: rotation(c(3, 0.0) * rad, c(4, 0.0) * rad, c(5, 0.0) * rad),
            scale: Vec3::new(c(6, 1.0), c(7, 1.0), c(8, 1.0)),
        }
    }

    pub fn transform(&self, p: Vec3) -> Vec3 {
        self.pos + self.rot * p
    }
}

/// Rotation matrix of 0x42cc7c, called with (pitch, heading, bank) [O]. Its rows are the
/// object's axes; forward = (sin h cos p, -sin p, cos h cos p), so positive pitch dips the
/// nose and positive heading turns towards +x (LightWave's convention).
pub fn rotation(h: f32, p: f32, b: f32) -> Mat3 {
    let (sh, ch) = h.sin_cos();
    let (sp, cp) = p.sin_cos();
    let (sb, cb) = b.sin_cos();
    Mat3::from_cols(
        Vec3::new(sp * sh * sb + ch * cb, cp * sb, sp * ch * sb - sh * cb),
        Vec3::new(sp * sh * cb - ch * sb, cp * cb, sp * ch * cb + sh * sb),
        Vec3::new(sh * cp, -sp, ch * cp),
    )
}

/// LightWave TCB (Kochanek-Bartels) evaluation at `frame`, LightWave's own algorithm: the
/// Hermite basis at 0x455fa0 (h2 = 3t^2 - 2t^3, h4 = t^3 - t^2, h3 = h4 - t^2 + t) [O],
/// tangents scaled for uneven key spacing [I: the LW SDK code, matching the basis].
/// `repeat` wraps the time over the key range (EndBehavior 2), else it holds the end keys
/// (0x456270).
pub fn evaluate(m: &Motion, frame: f32, repeat: bool) -> Vec<f32> {
    let k = &m.keys;
    let Some(first) = k.first() else { return Vec::new() };
    let last = &k[k.len() - 1];
    let span = last.frame - first.frame;
    if k.len() == 1 || span <= 0.0 {
        return first.values.clone();
    }
    let f = if repeat { first.frame + (frame - first.frame).rem_euclid(span) } else { frame.clamp(first.frame, last.frame) };
    let i = k.partition_point(|key| key.frame <= f).clamp(1, k.len() - 1) - 1;
    let (k0, k1) = (&k[i], &k[i + 1]);
    let len = (k1.frame - k0.frame).max(1e-6);
    let t = ((f - k0.frame) / len).clamp(0.0, 1.0);
    let n = k0.values.len().min(k1.values.len());
    if k1.linear != 0.0 {
        return (0..n).map(|c| k0.values[c] + (k1.values[c] - k0.values[c]) * t).collect();
    }
    let h2 = 3.0 * t * t - 2.0 * t * t * t;
    let h1 = 1.0 - h2;
    let h4 = t * t * t - t * t;
    let h3 = h4 - t * t + t;
    let dd0a = (1.0 - k0.tension) * (1.0 + k0.continuity) * (1.0 + k0.bias);
    let dd0b = (1.0 - k0.tension) * (1.0 - k0.continuity) * (1.0 - k0.bias);
    let ds1a = (1.0 - k1.tension) * (1.0 - k1.continuity) * (1.0 + k1.bias);
    let ds1b = (1.0 - k1.tension) * (1.0 + k1.continuity) * (1.0 - k1.bias);
    let prev = (i > 0).then(|| &k[i - 1]);
    let next = k.get(i + 2);
    (0..n)
        .map(|c| {
            let d10 = k1.values[c] - k0.values[c];
            let dd0 = match prev {
                Some(p) => len / (k1.frame - p.frame) * (dd0a * (k0.values[c] - p.values[c]) + dd0b * d10),
                None => 0.5 * (dd0a + dd0b) * d10,
            };
            let ds1 = match next {
                Some(q) => len / (q.frame - k0.frame) * (ds1a * d10 + ds1b * (q.values[c] - k1.values[c])),
                None => 0.5 * (ds1a + ds1b) * d10,
            };
            k0.values[c] * h1 + k1.values[c] * h2 + dd0 * h3 + ds1 * h4
        })
        .collect()
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Kind {
    /// UnseenByRays: drawn only
    Scenery,
    /// rigid body; `mass` in kg
    Body { mass: f32 },
}

#[derive(Clone, Debug)]
pub struct Prop {
    /// index into `Props::meshes`
    pub mesh: usize,
    pub kind: Kind,
    /// None for a single-key object (static pose)
    motion: Option<Motion>,
    /// a car hit this animated body: its motion is gone until the restart and it is a free body
    /// (0x45b464 clears obj+0xc4 and the scale; restart 0x455e0b puts the motion back) [O]
    lost: bool,
    repeat: bool,
    rest: Pose,
    pub pose: Pose,
    /// `ObjectEnvSound` id of the track's .esd (ObjEdgeColor, 1st value); 0 = none
    pub env_sound: u16,
    /// rigid body of a `Kind::Body` object
    body: Option<Body>,
    dynamics: Dyn,
    /// pose at the start of the substep (rb+0x70); transient, set by `begin_substep`
    prev: Pose,
}

/// One object mesh: file stem plus the MYO bounding sphere (object space).
#[derive(Clone, Debug)]
pub struct PropMesh {
    pub name: String,
    /// bounding box (object space)
    pub min: Vec3,
    pub max: Vec3,
    pub center: Vec3,
    pub radius: f32,
}

#[derive(Clone, Debug, Default)]
pub struct Props {
    pub meshes: Vec<PropMesh>,
    pub props: Vec<Prop>,
    fps: f32,
    /// animation clock, seconds since load. The game uses the wall clock since a start stamp
    /// (0x45acfc, `0x704e38`) [I: when that stamp is taken]
    time: f32,
}

/// The moving part of the scene objects (rewind): the clock and every object's pose and body state.
#[derive(Clone, Debug)]
pub struct PropState {
    time: f32,
    props: Vec<(Pose, Dyn, bool)>,
}

impl Props {
    pub fn state(&self) -> PropState {
        PropState { time: self.time, props: self.props.iter().map(|p| (p.pose, p.dynamics, p.lost)).collect() }
    }

    pub fn set_state(&mut self, s: &PropState) {
        self.time = s.time;
        for (p, &(pose, dynamics, lost)) in self.props.iter_mut().zip(&s.props) {
            (p.pose, p.dynamics, p.lost) = (pose, dynamics, lost);
        }
    }
}

impl Prop {
    /// Body-frame velocity and angular velocity of a body (rad/s; zero for scenery and sleepers).
    pub fn velocity(&self) -> (Vec3, Vec3) {
        (self.dynamics.vel, self.dynamics.ang)
    }
}

impl Props {
    /// The track's scene, or no props when it has none. Missing meshes are skipped.
    pub fn load(root: &Path, trk: &cfg::Config) -> Result<Props, String> {
        let first = |k: &str| trk.get(k).and_then(|e| e.values.first().cloned());
        let (Some(scene), Some(base)) = (first("SceneName"), first("BasePath")) else { return Ok(Props::default()) };
        let text = tbl::load(&fs::read(root, &scene)?, None).map_err(|e| format!("{scene}: {e}"))?;
        let s = cfg::parse_lwsc(&text.data).map_err(|e| format!("{scene}: {e}"))?;
        let mut out = Props { fps: s.fps.max(1.0), ..Props::default() };
        for o in s.objects {
            let mesh = match out.meshes.iter().position(|m| m.name.eq_ignore_ascii_case(&o.name)) {
                Some(i) => i,
                None => {
                    let Ok(m) = fs::read(root, &format!("{base}/Objects/{}.myo", o.name)).and_then(|b| myo::parse(&b).map_err(|e| e.to_string())) else {
                        continue;
                    };
                    let (min, max) = m.vertices.iter().fold((Vec3::splat(f32::MAX), Vec3::splat(f32::MIN)), |(lo, hi), &v| (lo.min(Vec3::from(v)), hi.max(Vec3::from(v))));
                    let (min, max) = if m.vertices.is_empty() { (Vec3::ZERO, Vec3::ZERO) } else { (min, max) };
                    out.meshes.push(PropMesh { name: o.name.clone(), min, max, center: Vec3::from(m.center), radius: m.radius });
                    out.meshes.len() - 1
                }
            };
            let Ok(motion) = o.motion else { continue };
            let rest = Pose::from_channels(motion.keys.first().map_or(&[][..], |k| &k.values));
            let mut body = None;
            let kind = if o.unseen_by_rays {
                Kind::Scenery
            } else {
                // PolygonSize 0: a sphere of water from the bounding radius (0x455d3a)
                let r = out.meshes[mesh].radius;
                let mass = o.polygon_size.filter(|&s| s != 0.0).map_or(4.0 / 3.0 * std::f32::consts::PI * r * r * r * 1000.0, |s| s * MASS_PER_POLYGON_SIZE.get());
                let m = &out.meshes[mesh];
                let shape = fs::read(root, &format!("{base}/Objects/{}.col", o.name)).ok().and_then(|b| motor_data::col::parse(&b).ok()).map(|c| Hull::from_col(&c));
                let b = Body::new(mass, m.min, m.max, r).with_id(o.col_sound);
                body = Some(shape.map_or(b.clone(), |h| b.with_shape(h)));
                Kind::Body { mass }
            };
            let motion = (motion.keys.len() > 1).then_some(motion);
            out.props.push(Prop { mesh, kind, motion, lost: false, repeat: o.end_behavior & 2 != 0, rest, pose: rest, env_sound: o.env_sound, body, dynamics: Dyn::default(), prev: rest });
        }
        Ok(out)
    }

    /// The animation clock: (seconds since load, the frame a scene object's motion is at).
    pub fn clock(&self) -> (f32, f32) {
        (self.time, self.time * self.fps)
    }

    /// Debug text for object `i` (motorview's inspector): kind, motion and its TCB keys around
    /// the current frame, rest pose and the loose-body state.
    pub fn debug(&self, i: usize) -> Vec<String> {
        let Some(p) = self.props.get(i) else { return Vec::new() };
        let v3 = |v: Vec3| format!("{:.2}, {:.2}, {:.2}", v.x, v.y, v.z);
        let kind = match p.kind {
            Kind::Scenery => "scenery (UnseenByRays)".to_string(),
            Kind::Body { mass } if mass >= HEAVY_MASS => format!("body {mass:.0} kg (heavy)"),
            Kind::Body { mass } => format!("body {mass:.0} kg"),
        };
        let mut out = vec![format!("kind: {kind}"), format!("rest pos: {}", v3(p.rest.pos)), format!("scale: {}", v3(p.pose.scale))];
        let frame = self.time * self.fps;
        match &p.motion {
            None => out.push("motion: none (one key)".into()),
            Some(m) => {
                let last = m.keys.last().map_or(0.0, |k| k.frame);
                let at = if p.repeat && last > 0.0 { frame % last } else { frame.min(last) };
                let k = m.keys.iter().rposition(|k| k.frame <= at).unwrap_or(0);
                out.push(format!("motion: {} keys, frame {at:.1} / {last:.0} at {} fps{}{}", m.keys.len(), self.fps, if p.repeat { ", repeat" } else { "" }, if p.lost { ", lost on a hit" } else { "" }));
                for (j, key) in m.keys.iter().enumerate().skip(k.saturating_sub(1)).take(3) {
                    let vals: Vec<String> = key.values.iter().map(|x| format!("{x:.2}")).collect();
                    let mark = if j == k { '>' } else { ' ' };
                    out.push(format!("{mark}key {j} @{:.0} t {:.2} c {:.2} b {:.2} lin {:.0}: {}", key.frame, key.tension, key.continuity, key.bias, key.linear, vals.join(" ")));
                }
            }
        }
        if matches!(p.kind, Kind::Body { .. }) {
            out.push(format!("{} vel {} spin {}", if p.dynamics.awake { "awake" } else { "asleep" }, v3(p.dynamics.vel), v3(p.dynamics.ang)));
        }
        out
    }

    /// Back to the scene's start poses (race restart, 0x455da0): a motion lost on a hit comes back
    /// (0x455e0b). The clock keeps running.
    pub fn reset(&mut self) {
        for p in &mut self.props {
            p.pose = p.rest;
            p.dynamics = Dyn::default();
            p.lost = false;
        }
    }

    /// Advance the animations by one frame (the loose props move in the substeps).
    pub fn animate(&mut self, dt: f32) {
        self.time += dt;
        let frame = self.time * self.fps;
        for p in &mut self.props {
            let Some(m) = p.motion.as_ref().filter(|_| !p.lost) else { continue };
            p.pose = Pose::from_channels(&evaluate(m, frame, p.repeat));
        }
    }

    /// `0x480fbc` step 1, once per physics substep of `h` seconds: remember the poses and
    /// integrate the awake bodies.
    pub fn begin_substep(&mut self, h: f32, gravity: f32) {
        for p in &mut self.props {
            p.prev = p.pose;
            if let Some(b) = &p.body {
                b.begin(&mut p.dynamics, &mut p.pose, h, gravity);
            }
        }
    }

    /// `0x480fbc` step 3: a car against every prop, both ways (call after the car's substep).
    /// An animated body takes part at its motion's pose (asleep, at rest); a hit wakes it, and it
    /// loses its motion and its light (scale 0) for the rest of the race (0x45b42c..0x45b4ac, after
    /// the world step; here at once, which is the same: the motion is only applied per frame) [O].
    pub fn contact(&mut self, car: &mut Car) {
        for p in &mut self.props {
            if let Some(b) = &p.body {
                if b.car_contact(&mut p.dynamics, &mut p.pose, car) && p.motion.is_some() && !p.lost {
                    p.lost = true;
                    p.pose.scale = Vec3::ZERO;
                }
            }
        }
    }

    /// `0x480fbc` step 4: the awake bodies against the track.
    pub fn finish_substep(&mut self, h: f32, world: &World) {
        let cast = |a: Vec3, b: Vec3| world.raycast(a, b);
        for p in &mut self.props {
            if let Some(b) = &p.body {
                b.finish(&mut p.dynamics, &mut p.pose, &p.prev, h, &cast);
            }
        }
    }
}


#[cfg(test)]
mod tests {
    use super::*;
    use motor_data::cfg::MotionKey;

    fn key(frame: f32, x: f32) -> MotionKey {
        MotionKey { values: vec![x, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0], frame, linear: 0.0, tension: 0.0, continuity: 0.0, bias: 0.0 }
    }

    #[test]
    fn tcb_hits_keys_and_loops() {
        let m = Motion { version: 1, channels: 9, keys: vec![key(0.0, 0.0), key(10.0, 5.0), key(20.0, 0.0)] };
        assert!((evaluate(&m, 10.0, false)[0] - 5.0).abs() < 1e-5);
        assert!((evaluate(&m, 25.0, true)[0] - evaluate(&m, 5.0, false)[0]).abs() < 1e-5);
        assert_eq!(evaluate(&m, 30.0, false)[0], 0.0);
        // smooth through the middle key: symmetric about it
        assert!((evaluate(&m, 8.0, false)[0] - evaluate(&m, 12.0, false)[0]).abs() < 1e-5);
    }

    #[test]
    fn rotation_axes() {
        let r = rotation(90f32.to_radians(), 0.0, 0.0);
        assert!((r.z_axis - Vec3::X).length() < 1e-6, "heading 90 faces +x");
        let r = rotation(0.0, 90f32.to_radians(), 0.0);
        assert!((r.z_axis + Vec3::Y).length() < 1e-6, "pitch 90 faces down");
        assert!((rotation(0.3, 0.2, 0.1).determinant() - 1.0).abs() < 1e-5);
    }
}
