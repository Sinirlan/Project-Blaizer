//! REMASTER proximity arrows (09-remaster 9.21 phase 7; all of it a design choice): a glass
//! `»` round your car pointing at each opponent close behind or alongside that is out of
//! view, green when one is first seen and red within half a metre. The arrows lie in the world, on an ellipse round
//! the car's footprint above the road, and are drawn in the HUD batch: each is Overload's
//! `»` field, projected with the camera and extruded by stacking slices (`Batch::glass_glyph`).
//! APPROX of the plan's mesh pass: no depth test, so a wall in front of the arrow doesn't
//! hide it (they sit by your bumper, where little comes between).

use crate::front::RaceState;
use glam::{Vec2, Vec3};
use motor_sim::car::Car;
use motorview::drive::CamMode;
use motorview::ui::Batch;
use motorview::ui_font::HudFont;

/// Gap (m, between the cars' footprints) at which an arrow appears, and the one at which
/// it is fully red.
const RANGE: f32 = 16.0;
const NEAR: f32 = 0.5;
/// Opponents within this angle of your nose (degrees each side) get no arrow: the plan's
/// rear 240°.
const FRONT_HALF: f32 = 60.0;
/// The nearest few get an arrow; fade in/out time (s).
const MAX_ARROWS: usize = 3;
const FADE: f32 = 0.2;
/// The ellipse: this far out from the footprint's half-length and half-width (m), and the
/// arrow's height over the ground plane (m).
const OUT: f32 = 0.8;
const LIFT: f32 = 0.3;
/// The glyph's length along its point (m), its extrusion (m) and the slices that build it.
const LENGTH: f32 = 0.55;
const DEPTH: f32 = 0.15;
const SLICES: usize = 24;
/// Grid cells per side of each slice (perspective).
const GRID: usize = 6;
const GLYPH: char = '»';
/// Nearest the eye a point may be and still be projected (m).
const EYE_NEAR: f32 = 0.3;
/// The game's horizontal field of view (render.rs FOV_X).
const FOV_X: f32 = 90.0;

/// A car's footprint (PhyCorner FL, FR, BR, BL) on the ground plane, world x/z.
fn footprint(car: &Car) -> [Vec2; 4] {
    car.p.corners.map(|c| {
        let p = car.pos + car.rot * c;
        Vec2::new(p.x, p.z)
    })
}

/// Distance between two convex quads (0 when they overlap).
fn gap(a: &[Vec2; 4], b: &[Vec2; 4]) -> f32 {
    let inside = |p: Vec2, q: &[Vec2; 4]| {
        let s: Vec<f32> = (0..4).map(|i| (q[(i + 1) % 4] - q[i]).perp_dot(p - q[i])).collect();
        s.iter().all(|&x| x >= 0.0) || s.iter().all(|&x| x <= 0.0)
    };
    if a.iter().any(|&p| inside(p, b)) || b.iter().any(|&p| inside(p, a)) {
        return 0.0;
    }
    let seg = |p: Vec2, s0: Vec2, s1: Vec2| {
        let d = s1 - s0;
        let h = ((p - s0).dot(d) / d.length_squared().max(1e-9)).clamp(0.0, 1.0);
        (p - (s0 + d * h)).length()
    };
    let one_way = |a: &[Vec2; 4], b: &[Vec2; 4]| (0..4).flat_map(|i| a.iter().map(move |&p| seg(p, b[i], b[(i + 1) % 4]))).fold(f32::MAX, f32::min);
    one_way(a, b).min(one_way(b, a))
}

/// The opponents that should show an arrow, nearest first: (racer, gap).
fn candidates(r: &RaceState) -> Vec<(usize, f32)> {
    let race = &r.sess.race;
    let me = &race.racers[r.sess.focus].car;
    let mine = footprint(me);
    let fwd = me.rot * Vec3::Z;
    let fwd = Vec2::new(fwd.x, fwd.z).normalize_or(Vec2::Y);
    let centre = mine.iter().copied().sum::<Vec2>() * 0.25;
    let mut out: Vec<(usize, f32)> = race
        .racers
        .iter()
        .enumerate()
        .filter(|(i, rc)| *i != r.sess.focus && rc.progress.eliminated_ms.is_none())
        .filter_map(|(i, rc)| {
            let theirs = footprint(&rc.car);
            let to = (theirs.iter().copied().sum::<Vec2>() * 0.25 - centre).normalize_or(-fwd);
            let g = gap(&mine, &theirs);
            let behind = to.dot(fwd) < FRONT_HALF.to_radians().cos();
            (g < RANGE && behind && !on_screen(r, rc.car.pos)).then_some((i, g))
        })
        .collect();
    out.sort_by(|a, b| a.1.total_cmp(&b.1));
    out.truncate(MAX_ARROWS);
    out
}

/// Whether a car (its origin) is in front of the eye and inside the view's 90° horizontal
/// field of view: those you can see get no arrow (the user's rule: only cars behind you that
/// you can't see). The vertical extent isn't tested, as the window's aspect isn't known
/// here; a car beside or behind you leaves the view sideways or behind the eye.
fn on_screen(r: &RaceState, p: Vec3) -> bool {
    let cam = &r.cam;
    let d = Vec3::new(p.x, p.y, -p.z) - cam.pos;
    let z = d.dot(cam.forward());
    z > EYE_NEAR && d.dot(cam.right()).abs() < z * (FOV_X * 0.5).to_radians().tan()
}

/// Per frame: fade each opponent's arrow towards shown / hidden.
pub fn update(r: &mut RaceState, dt: f32) {
    let n = r.sess.race.racers.len();
    r.prox.resize(n, 0.0);
    let shown = candidates(r);
    for (i, a) in r.prox.iter_mut().enumerate() {
        let target = if shown.iter().any(|x| x.0 == i) { 1.0 } else { 0.0 };
        let step = dt / FADE;
        *a = if *a < target { (*a + step).min(target) } else { (*a - step).max(target) };
    }
}

/// Canvas point of a game-space point (90° horizontal field of view, as `nameplates`).
fn project(r: &RaceState, w: f32, h: f32, p: Vec3) -> Option<[f32; 2]> {
    let cam = &r.cam;
    let (f, rt) = (cam.forward(), cam.right());
    let up = rt.cross(f);
    let d = Vec3::new(p.x, p.y, -p.z) - cam.pos;
    let z = d.dot(f);
    if z < EYE_NEAR {
        return None;
    }
    let focal = w * 0.5;
    Some([w * 0.5 + d.dot(rt) / z * focal, h * 0.5 - d.dot(up) / z * focal])
}

pub fn draw(r: &RaceState, b: &mut Batch) {
    // not from inside the car, nor from the goal camera or the start fly-by
    if matches!(r.sess.cam, CamMode::InCar | CamMode::Bumper | CamMode::Goal | CamMode::Tv) || r.sess.flying() {
        return;
    }
    let Some(bbox) = motorview::ui_font::face(HudFont::Arrow).glyph(GLYPH).and_then(|g| g.bbox) else { return };
    let k = LENGTH / (bbox[2] - bbox[0]).max(1) as f32; // metres per field source px
    let race = &r.sess.race;
    let me = &race.racers[r.sess.focus].car;
    let mine = footprint(me);
    // the footprint in body space: its centre and half sizes
    let cs = me.p.corners;
    let mid = cs.iter().copied().sum::<Vec3>() * 0.25;
    let half = cs.iter().fold(Vec2::ZERO, |m, c| m.max(Vec2::new((c.x - mid.x).abs(), (c.z - mid.z).abs())));
    let (ax, az) = (half.x + OUT, half.y + OUT);
    let ride = me.p.ride_height();
    let up = me.rot * Vec3::Y;
    let eye = Vec3::new(r.cam.pos.x, r.cam.pos.y, -r.cam.pos.z);
    let (w, h) = (b.canvas.w, b.canvas.h);
    for (i, rc) in race.racers.iter().enumerate() {
        let alpha = r.prox.get(i).copied().unwrap_or(0.0);
        if i == r.sess.focus || alpha <= 0.0 || rc.progress.eliminated_ms.is_some() {
            continue;
        }
        let theirs = footprint(&rc.car);
        let g = gap(&mine, &theirs);
        // their bearing in your body frame, onto the ellipse
        let c = theirs.iter().copied().sum::<Vec2>() * 0.25;
        let to = me.rot.transpose() * (Vec3::new(c.x, 0.0, c.y) - me.pos);
        let dir = Vec2::new(to.x, to.z).normalize_or(Vec2::NEG_Y);
        let t = 1.0 / ((dir.x / ax).powi(2) + (dir.y / az).powi(2)).sqrt();
        let at = me.pos + me.rot * Vec3::new(mid.x + dir.x * t, ride + LIFT, mid.z + dir.y * t);
        // glyph frame: x = its point (outward), y = across (the glyph's down), z = up
        let o = me.rot * Vec3::new(dir.x, 0.0, dir.y);
        let l = up.cross(o);
        let v = eye - at;
        let view = [v.dot(o), v.dot(l), v.dot(up)];
        let ramp = ((RANGE - g) / (RANGE - NEAR)).clamp(0.0, 1.0);
        for s in 0..SLICES {
            let layer = s as f32 / (SLICES - 1) as f32;
            let base = at + up * (layer * DEPTH);
            let to_canvas = |x: f32, y: f32| project(r, w, h, base + o * (x * k) + l * (y * k));
            b.glass_glyph(HudFont::Arrow, GLYPH, &to_canvas, GRID, ramp, alpha, layer, view);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn square(x: f32, z: f32) -> [Vec2; 4] {
        [Vec2::new(x, z + 1.0), Vec2::new(x + 1.0, z + 1.0), Vec2::new(x + 1.0, z), Vec2::new(x, z)]
    }

    #[test]
    fn gaps() {
        assert_eq!(gap(&square(0.0, 0.0), &square(0.5, 0.5)), 0.0);
        assert!((gap(&square(0.0, 0.0), &square(3.0, 0.0)) - 2.0).abs() < 1e-6);
        assert!((gap(&square(0.0, 0.0), &square(2.0, 2.0)) - 2f32.sqrt()).abs() < 1e-6);
    }
}
