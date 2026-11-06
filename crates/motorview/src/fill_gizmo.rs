//! REMASTER world-fill editor transform gizmos (12-world-fill 12.8, 12.9): Blender-style move
//! arrows, rotate rings and scale handles on the selected piece (or its selected vertex), drawn as
//! solid shaded meshes over the scene. Pure maths, drawing and picking; the state (mode, hot axis,
//! drag) lives in `WorldEdit`. Game space; X red, Y green, Z blue in every mode. Move and rotate
//! use the global axes, scale the piece's own (a block's sides, a wall's length and height).

use crate::debug::Gizmos;
use crate::world_edit::{handles, set_handle};
use crate::procgen::fill::{block_angles, block_rotation, Piece, Shape};
use glam::{Quat, Vec3};

pub const AXES: [Vec3; 3] = [Vec3::X, Vec3::Y, Vec3::Z];
const COLOURS: [[f32; 4]; 3] = [[0.95, 0.25, 0.25, 1.0], [0.45, 0.85, 0.25, 1.0], [0.3, 0.5, 1.0, 1.0]];
/// an axis the selected piece cannot use
const DISABLED: [f32; 4] = [0.42, 0.42, 0.42, 1.0];
const HOT: [f32; 4] = [1.0, 1.0, 0.55, 1.0];
/// gizmo length as a share of the eye distance (constant size on screen)
const SIZE_K: f32 = 0.16;
/// pick distance from an arrow or circle, pixels
const GRAB_PX: f32 = 12.0;
const SEGMENTS: usize = 64;
/// segments round a shaft or cone, along the rotate ring's tube, and the shading floor
const ROUND: usize = 10;
const TUBE: usize = 6;
const AMBIENT: f32 = 0.55;

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Mode {
    /// no gizmo: drag the handles
    #[default]
    Handles,
    Move,
    Rotate,
    /// stretch along the piece's own axes (12.9)
    Scale,
}

/// A change of a piece from the state at the start of a drag.
#[derive(Clone, Copy, Debug)]
pub enum Delta {
    Move(Vec3),
    /// about the global axes through the piece's pivot
    Rotate(Quat),
    /// along axis `axis` (index into the piece's `axes`, `dir` its direction) by `factor`
    Scale { axis: usize, dir: Vec3, factor: f32 },
}

/// Which of X, Y, Z the piece can use in `mode`. Hills turn about Y only, skirts have no position. Scale: blocks all sides, walls length and height, plants and models one
/// uniform size (any axis; a model also turns about Y), flat pieces their plan.
pub fn enabled(p: &Piece, mode: Mode) -> [bool; 3] {
    match (mode, &p.shape) {
        (Mode::Handles, _) | (_, Shape::Skirt { .. }) => [false; 3],
        (Mode::Rotate, Shape::Hill { .. }) => [false, true, false],
        (Mode::Rotate, Shape::Plant { .. }) => [false; 3],
        (Mode::Rotate, Shape::Model { .. }) => [false, true, false],
        (Mode::Scale, Shape::Wall { .. }) => [true, true, false],
        (Mode::Scale, Shape::Ground { .. }) => [true, false, true],
        _ => [true; 3],
    }
}

/// The directions of the gizmo's three axes: global, except scaling a block (its sides) or a wall
/// (along, up, across).
pub fn axes(p: &Piece, mode: Mode) -> [Vec3; 3] {
    if mode != Mode::Scale {
        return AXES;
    }
    match &p.shape {
        Shape::Block { yaw, pitch, roll, .. } => {
            let r = block_rotation(*yaw, *pitch, *roll);
            AXES.map(|a| r * a)
        }
        Shape::Wall { from, to, .. } => {
            let along = (Vec3::from(*to) - Vec3::from(*from)).normalize_or(Vec3::X);
            [along, Vec3::Y, along.cross(Vec3::Y).normalize_or(Vec3::Z)]
        }
        _ => AXES,
    }
}

/// Centre of a piece: a block's box centre, a wall's face centre, else the mean of its handles
/// (`y` for those without a height).
pub fn pivot(p: &Piece, y: f32) -> Vec3 {
    match &p.shape {
        Shape::Block { centre, size, yaw, pitch, roll, .. } => Vec3::from(*centre) + block_rotation(*yaw, *pitch, *roll) * Vec3::new(0.0, size[1] / 2.0, 0.0),
        Shape::Wall { from, to, height } => (Vec3::from(*from) + Vec3::from(*to)) / 2.0 + Vec3::Y * height / 2.0,
        _ => {
            let hs = handles(p, y);
            hs.iter().sum::<Vec3>() / hs.len().max(1) as f32
        }
    }
}

/// `base` changed by `delta`; `vertex` limits a move to one handle. Draped ground points get a
/// height once a move or tilt gives them one. Scaling a block keeps its base centre (design choice:
/// it grows up from the ground); a plant scales uniformly.
pub fn apply(base: &Piece, delta: Delta, vertex: Option<usize>, y: f32) -> Piece {
    let mut p = base.clone();
    let hs = handles(base, y);
    let pv = pivot(base, y);
    let pin = match delta {
        Delta::Move(d) => d.y.abs() > 1e-4,
        Delta::Rotate(q) => (q * Vec3::Y).distance(Vec3::Y) > 1e-4,
        Delta::Scale { .. } => false,
    };
    let put = |p: &mut Piece, i: usize, to: Vec3| {
        set_handle(p, i, to);
        if pin {
            // a draped point gets a height once the change has a vertical part
            match &mut p.shape {
                Shape::Ground { points } => {
                    if let Some(q) = points.get_mut(i).filter(|q| q.1.is_none()) {
                        q.1 = Some(to.y);
                    }
                }
                Shape::Hill { points, .. } => {
                    if let Some(q) = points.get_mut(i).filter(|q| q.1.is_none()) {
                        q.1 = Some(to.y);
                    }
                }
                _ => {}
            }
        }
    };
    match delta {
        Delta::Move(d) => {
            let only = vertex.filter(|&i| i < hs.len());
            for (i, h) in hs.iter().enumerate().filter(|(i, _)| only.is_none_or(|o| o == *i)) {
                put(&mut p, i, *h + d);
            }
        }
        Delta::Rotate(q) => {
            if let Shape::Model { yaw, .. } = &mut p.shape {
                // about Y only (the gizmo offers no other axis): the turn adds to the model's yaw
                let (a, _, _) = q.to_euler(glam::EulerRot::YXZ);
                *yaw = (*yaw + a.to_degrees()).rem_euclid(360.0);
            } else if let Shape::Block { centre, yaw, pitch, roll, .. } = &mut p.shape {
                let r = q * block_rotation(*yaw, *pitch, *roll);
                *centre = (pv + q * (Vec3::from(*centre) - pv)).into();
                (*yaw, *pitch, *roll) = block_angles(r);
            } else {
                for (i, h) in hs.iter().enumerate() {
                    put(&mut p, i, pv + q * (*h - pv));
                }
            }
        }
        Delta::Scale { axis, dir, factor } => {
            let f = factor.clamp(0.05, 20.0);
            match &mut p.shape {
                Shape::Block { size, .. } => size[axis] = (size[axis] * f).max(0.1),
                Shape::Plant { scale, .. } | Shape::Model { scale, .. } => *scale = (*scale * f).clamp(0.05, 50.0),
                Shape::Wall { height, .. } if axis == 1 => *height = (*height * f).max(0.1),
                Shape::Hill { height, .. } if axis == 1 => *height = (*height * f).max(0.5),
                Shape::Skirt { .. } => {}
                _ => {
                    // the handles' distance from the pivot along `dir` grows by `f`
                    for (i, h) in hs.iter().enumerate() {
                        put(&mut p, i, *h + dir * dir.dot(*h - pv) * (f - 1.0));
                    }
                }
            }
        }
    }
    p
}

/// `apply` for a hill whose draped ridge points are shown at `crest` (the heights of the last mesh,
/// one per point): they are pinned there for the change and freed again where it left them alone, so a
/// point stays draped unless the change had a vertical part. Other pieces, or a `crest` of the wrong
/// length, go to `apply` as they are.
pub fn apply_crest(base: &Piece, delta: Delta, vertex: Option<usize>, y: f32, crest: &[f32]) -> Piece {
    let Shape::Hill { points, .. } = &base.shape else { return apply(base, delta, vertex, y) };
    if crest.len() != points.len() {
        return apply(base, delta, vertex, y);
    }
    let mut baked = base.clone();
    if let Shape::Hill { points: bp, .. } = &mut baked.shape {
        for (q, &c) in bp.iter_mut().zip(crest) {
            q.1.get_or_insert(c);
        }
    }
    let mut out = apply(&baked, delta, vertex, y);
    if let Shape::Hill { points: op, .. } = &mut out.shape {
        for ((o, b), &c) in op.iter_mut().zip(points).zip(crest) {
            if b.1.is_none() && o.1.is_some_and(|v| (v - c).abs() < 1e-3) {
                o.1 = None;
            }
        }
    }
    out
}

/// Gizmo length for a pivot seen from `eye`.
pub fn length(pivot: Vec3, eye: Vec3) -> f32 {
    (pivot.distance(eye) * SIZE_K).max(0.5)
}

/// The circle of axis `i`: `SEGMENTS + 1` points, the last equal to the first.
fn circle(pivot: Vec3, i: usize, r: f32) -> Vec<Vec3> {
    let (u, v) = (AXES[(i + 1) % 3], AXES[(i + 2) % 3]);
    (0..=SEGMENTS)
        .map(|k| {
            let a = k as f32 / SEGMENTS as f32 * std::f32::consts::TAU;
            pivot + (u * a.cos() + v * a.sin()) * r
        })
        .collect()
}

/// A surface point and its outward normal.
type Pn = (Vec3, Vec3);

/// One shaded quad. It is skipped when it faces away from `eye` (the pass has no depth test, so
/// the far side of a closed shape must not paint over the near side).
fn facet(g: &mut Gizmos, q: [Pn; 4], c: [f32; 4], eye: Vec3) {
    let centre = q.iter().map(|v| v.0).sum::<Vec3>() / 4.0;
    let nrm = q.iter().map(|v| v.1).sum::<Vec3>().normalize_or_zero();
    let facing = nrm.dot((eye - centre).normalize_or_zero());
    if facing <= 0.0 {
        return;
    }
    let shade = AMBIENT + (1.0 - AMBIENT) * facing;
    let col = [c[0] * shade, c[1] * shade, c[2] * shade, 1.0];
    g.tri(q[0].0, q[1].0, q[2].0, col);
    g.tri(q[0].0, q[2].0, q[3].0, col);
}

/// Solid quads between neighbouring `rings` (`wrap`: the last joins the first), each ring closed.
fn skin(g: &mut Gizmos, rings: &[Vec<Pn>], wrap: bool, c: [f32; 4], eye: Vec3) {
    let n = rings.len();
    for r in 0..if wrap { n } else { n.saturating_sub(1) } {
        let (a, b) = (&rings[r], &rings[(r + 1) % n]);
        for k in 0..a.len() {
            let k2 = (k + 1) % a.len();
            facet(g, [a[k], a[k2], b[k2], b[k]], c, eye);
        }
    }
}

/// A ring of `n` points of radius `r` about `centre` in the plane of `u`, `v`, normals radial
/// tilted by `axial` along `w` (a cone's slant, a cap's face).
fn ring(centre: Vec3, u: Vec3, v: Vec3, w: Vec3, r: f32, axial: f32, n: usize) -> Vec<Pn> {
    (0..n)
        .map(|k| {
            let a = k as f32 / n as f32 * std::f32::consts::TAU;
            let radial = u * a.cos() + v * a.sin();
            (centre + radial * r, (radial + w * axial).normalize_or_zero())
        })
        .collect()
}

/// A tapered rod from `a` to `b` (radii `r0`, `r1`), with flat caps.
fn rod(g: &mut Gizmos, a: Vec3, b: Vec3, r0: f32, r1: f32, c: [f32; 4], eye: Vec3) {
    let w = (b - a).normalize_or_zero();
    let u = w.cross(if w.y.abs() > 0.9 { Vec3::X } else { Vec3::Y }).normalize_or_zero();
    let v = w.cross(u);
    let slant = (r0 - r1) / (b - a).length().max(1e-4);
    let side = |p: Vec3, r: f32| ring(p, u, v, w, r, slant, ROUND);
    skin(g, &[side(a, r0), side(b, r1)], false, c, eye);
    // caps: a ring at the rim facing along the axis, then the centre
    let cap = |p: Vec3, r: f32, dir: f32| [ring(p, u, v, w, r, dir * 1e3, ROUND), ring(p, u, v, w, 0.0, dir * 1e3, ROUND)];
    skin(g, &cap(a, r0, -1.0), false, c, eye);
    skin(g, &cap(b, r1, 1.0), false, c, eye);
}

/// A cube of half-size `h` with sides along `ax`.
fn cube(g: &mut Gizmos, centre: Vec3, ax: [Vec3; 3], h: f32, c: [f32; 4], eye: Vec3) {
    for (i, sign) in (0..3).flat_map(|i| [(i, 1.0), (i, -1.0)]) {
        let (n, u, v) = (ax[i] * sign, ax[(i + 1) % 3], ax[(i + 2) % 3]);
        let p = |a: f32, b: f32| (centre + n * h + (u * a + v * b) * h, n);
        facet(g, [p(-1.0, -1.0), p(1.0, -1.0), p(1.0, 1.0), p(-1.0, 1.0)], c, eye);
    }
}

/// A torus about axis `w` through `centre`: main radius `r`, tube radius `t`.
fn torus(g: &mut Gizmos, centre: Vec3, w: Vec3, r: f32, t: f32, c: [f32; 4], eye: Vec3) {
    let u = w.cross(if w.y.abs() > 0.9 { Vec3::X } else { Vec3::Y }).normalize_or_zero();
    let v = w.cross(u);
    let rings: Vec<Vec<Pn>> = (0..SEGMENTS)
        .map(|k| {
            let a = k as f32 / SEGMENTS as f32 * std::f32::consts::TAU;
            let radial = u * a.cos() + v * a.sin();
            ring(centre + radial * r, radial, w, Vec3::ZERO, t, 0.0, TUBE)
        })
        .collect();
    skin(g, &rings, true, c, eye);
}

/// Draw the gizmo of `mode` at `pivot` along `ax`; `hot` is the hovered or dragged axis.
pub fn draw(g: &mut Gizmos, mode: Mode, pivot: Vec3, en: [bool; 3], ax: [Vec3; 3], hot: Option<usize>, eye: Vec3) {
    let l = length(pivot, eye);
    for i in 0..3 {
        let c = if hot == Some(i) { HOT } else if en[i] { COLOURS[i] } else { DISABLED };
        match mode {
            Mode::Move => {
                rod(g, pivot, pivot + ax[i] * l * 0.85, 0.035 * l, 0.035 * l, c, eye);
                rod(g, pivot + ax[i] * l * 0.85, pivot + ax[i] * l * 1.2, 0.1 * l, 0.0, c, eye);
            }
            Mode::Scale => {
                rod(g, pivot, pivot + ax[i] * l * 0.95, 0.035 * l, 0.035 * l, c, eye);
                cube(g, pivot + ax[i] * l * 1.05, ax, 0.08 * l, c, eye);
            }
            Mode::Rotate => torus(g, pivot, AXES[i], l * 0.9, 0.03 * l, c, eye),
            Mode::Handles => {}
        }
    }
}

fn dist_to_segment(p: (f32, f32), a: (f32, f32), b: (f32, f32)) -> f32 {
    let (ab, ap) = ((b.0 - a.0, b.1 - a.1), (p.0 - a.0, p.1 - a.1));
    let len2 = ab.0 * ab.0 + ab.1 * ab.1;
    let t = if len2 < 1e-6 { 0.0 } else { ((ap.0 * ab.0 + ap.1 * ab.1) / len2).clamp(0.0, 1.0) };
    (p.0 - (a.0 + ab.0 * t)).hypot(p.1 - (a.1 + ab.1 * t))
}

/// The axis whose arrow or circle is within `GRAB_PX` of `cursor` (pixels); `project` maps game
/// space to pixels.
pub fn pick(mode: Mode, pivot: Vec3, en: [bool; 3], ax: [Vec3; 3], eye: Vec3, project: &dyn Fn(Vec3) -> Option<(f32, f32)>, cursor: (f32, f32)) -> Option<usize> {
    let l = length(pivot, eye);
    (0..3)
        .filter(|&i| en[i])
        .filter_map(|i| {
            let pts = match mode {
                Mode::Move => vec![pivot, pivot + ax[i] * l * 1.2],
                Mode::Scale => vec![pivot, pivot + ax[i] * l * 1.15],
                Mode::Rotate => circle(pivot, i, l * 0.9),
                Mode::Handles => return None,
            };
            let px: Vec<Option<(f32, f32)>> = pts.into_iter().map(project).collect();
            let d = px.windows(2).filter_map(|w| Some(dist_to_segment(cursor, w[0]?, w[1]?))).min_by(f32::total_cmp)?;
            (d <= GRAB_PX).then_some((i, d))
        })
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(i, _)| i)
}

/// Distance along the axis direction `a` through `pivot` of the point nearest the mouse ray.
pub fn axis_param(pivot: Vec3, a: Vec3, ray: (Vec3, Vec3)) -> Option<f32> {
    let (o, d) = ray;
    let w = pivot - o;
    let (b, e, f) = (a.dot(d), d.dot(w), a.dot(w));
    let denom = 1.0 - b * b;
    (denom > 1e-4).then(|| (b * e - f) / denom)
}

/// Angle (radians) of the mouse ray's hit on the plane through `pivot` normal to axis `i`, from
/// the next axis towards the one after (so +angle turns the same way as `Quat::from_axis_angle`).
pub fn plane_angle(pivot: Vec3, i: usize, ray: (Vec3, Vec3)) -> Option<f32> {
    let (n, (o, d)) = (AXES[i], ray);
    let denom = n.dot(d);
    if denom.abs() < 1e-3 {
        return None;
    }
    let t = (pivot - o).dot(n) / denom;
    if t < 0.0 {
        return None;
    }
    let v = o + d * t - pivot;
    Some(v.dot(AXES[(i + 2) % 3]).atan2(v.dot(AXES[(i + 1) % 3])))
}

/// Ring segments of the skirt gizmo.
const SKIRT_SEGMENTS: usize = 128;

/// The skirt gizmo (12.11): its outer ring at `floor` and, on the ring point nearest `eye`, an arrow
/// up the Y axis. The ring sets the radius, the arrow the drop (the floor's depth under the world).
/// Both are hot (index 0 ring, 1 arrow) when hovered or dragged.
pub fn skirt_ring(centre: Vec3, radius: f32, floor: f32) -> Vec<Vec3> {
    (0..SKIRT_SEGMENTS)
        .map(|k| {
            let a = k as f32 / SKIRT_SEGMENTS as f32 * std::f32::consts::TAU;
            Vec3::new(centre.x + a.cos() * radius, floor, centre.z + a.sin() * radius)
        })
        .collect()
}

/// Foot of the skirt's arrow: the ring point nearest `eye`.
pub fn skirt_arrow(centre: Vec3, radius: f32, floor: f32, eye: Vec3) -> Vec3 {
    let dir = (eye - centre).with_y(0.0).normalize_or(Vec3::X);
    Vec3::new(centre.x + dir.x * radius, floor, centre.z + dir.z * radius)
}

pub fn skirt_draw(g: &mut Gizmos, centre: Vec3, radius: f32, floor: f32, drop: f32, hot: Option<usize>, eye: Vec3) {
    let ring = skirt_ring(centre, radius, floor);
    g.polyline(&ring, true, if hot == Some(0) { HOT } else { COLOURS[2] }, true);
    let foot = skirt_arrow(centre, radius, floor, eye);
    let l = length(foot, eye);
    let c = if hot == Some(1) { HOT } else { COLOURS[1] };
    rod(g, foot, foot + Vec3::Y * l * 0.85, 0.035 * l, 0.035 * l, c, eye);
    rod(g, foot + Vec3::Y * l * 0.85, foot + Vec3::Y * l * 1.2, 0.1 * l, 0.0, c, eye);
    g.label(foot + Vec3::Y * l * 1.3, format!("radius {radius:.0} m, drop {drop:.0} m"), c);
}

/// 0 when the cursor is on the ring, 1 on the arrow (the arrow wins where they meet).
pub fn skirt_pick(centre: Vec3, radius: f32, floor: f32, eye: Vec3, project: &dyn Fn(Vec3) -> Option<(f32, f32)>, cursor: (f32, f32)) -> Option<usize> {
    let foot = skirt_arrow(centre, radius, floor, eye);
    let l = length(foot, eye);
    let near = |pts: &[Vec3], closed: bool| {
        let px: Vec<Option<(f32, f32)>> = pts.iter().map(|&p| project(p)).collect();
        let n = px.len();
        (0..if closed { n } else { n - 1 }).filter_map(|i| Some(dist_to_segment(cursor, px[i]?, px[(i + 1) % n]?))).any(|d| d <= GRAB_PX)
    };
    if near(&[foot, foot + Vec3::Y * l * 1.2], false) {
        Some(1)
    } else {
        near(&skirt_ring(centre, radius, floor), true).then_some(0)
    }
}

/// Distance from the centre, in plan, of the mouse ray's hit on the plane `y = floor`.
pub fn ring_param(centre: Vec3, floor: f32, ray: (Vec3, Vec3)) -> Option<f32> {
    let (o, d) = ray;
    if d.y.abs() < 1e-4 {
        return None;
    }
    let t = (floor - o.y) / d.y;
    (t > 0.0).then(|| (o + d * t - centre).with_y(0.0).length())
}

/// `b - a` wrapped to -pi..pi.
pub fn angle_diff(a: f32, b: f32) -> f32 {
    let d = (b - a) % std::f32::consts::TAU;
    if d > std::f32::consts::PI {
        d - std::f32::consts::TAU
    } else if d < -std::f32::consts::PI {
        d + std::f32::consts::TAU
    } else {
        d
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::procgen::fill;

    fn piece(text: &str) -> Piece {
        fill::from_text(text).0.pieces.remove(0)
    }

    #[test]
    fn block_angles_round_trip() {
        for (y, p, r) in [(30.0, 0.0, 0.0), (-120.0, 20.0, 0.0), (45.0, -35.0, 15.0)] {
            let (a, b, c) = block_angles(block_rotation(y, p, r));
            assert!((a - y).abs() < 1e-3 && (b - p).abs() < 1e-3 && (c - r).abs() < 1e-3, "{y} {p} {r} -> {a} {b} {c}");
        }
    }

    #[test]
    fn rotating_a_block_about_its_centre_keeps_the_centre() {
        let p = piece("Piece block Texture A Centre 10 2 5 Size 4 6 8 Yaw 20\n");
        let before = pivot(&p, 0.0);
        let q = apply(&p, Delta::Rotate(Quat::from_rotation_x(0.5)), None, 0.0);
        assert!(pivot(&q, 0.0).distance(before) < 1e-4);
        let Shape::Block { pitch, .. } = q.shape else { panic!() };
        assert!(pitch.abs() > 1.0);
    }

    #[test]
    fn move_one_vertex_and_pin_draped_ground() {
        let p = piece("Piece ground Texture A Point 0 0 Point 10 0 Point 10 10\n");
        let q = apply(&p, Delta::Move(Vec3::new(0.0, 3.0, 0.0)), Some(1), 1.0);
        let Shape::Ground { points } = q.shape else { panic!() };
        assert_eq!((points[0].1, points[1].1, points[2].1), (None, Some(4.0), None));
    }

    #[test]
    fn a_hill_moves_up_and_pins_only_what_moved() {
        let p = piece("Piece hill Texture A Point 0 0 Point 10 0 Point 20 0 Height 4 Width 6\n");
        assert_eq!(enabled(&p, Mode::Move), [true; 3]);
        let crest = [5.0, 7.0, 9.0];
        let pts = |q: Piece| match q.shape {
            Shape::Hill { points, .. } => points,
            _ => panic!(),
        };
        // one vertex up 3: it gets its meshed height + 3, the others stay draped
        let q = pts(apply_crest(&p, Delta::Move(Vec3::new(0.0, 3.0, 0.0)), Some(1), 0.0, &crest));
        assert_eq!((q[0].1, q[1].1, q[2].1), (None, Some(10.0), None));
        // the whole piece up 3: every point pins at its own meshed height + 3
        let q = pts(apply_crest(&p, Delta::Move(Vec3::new(0.0, 3.0, 0.0)), None, 0.0, &crest));
        assert_eq!((q[0].1, q[1].1, q[2].1), (Some(8.0), Some(10.0), Some(12.0)));
        // a sideways move or a yaw leaves them draped
        let q = pts(apply_crest(&p, Delta::Move(Vec3::new(4.0, 0.0, 2.0)), None, 0.0, &crest));
        assert!(q.iter().all(|p| p.1.is_none()) && q[0].0 == [4.0, 2.0]);
        let q = pts(apply_crest(&p, Delta::Rotate(Quat::from_rotation_y(0.7)), None, 0.0, &crest));
        assert!(q.iter().all(|p| p.1.is_none()));
        // a pinned point keeps following a vertical move
        let p = piece("Piece hill Texture A Point 0 0 2 Point 10 0 Height 4 Width 6\n");
        let q = pts(apply_crest(&p, Delta::Move(Vec3::new(0.0, -1.0, 0.0)), Some(0), 0.0, &[2.0, 7.0]));
        assert_eq!((q[0].1, q[1].1), (Some(1.0), None));
    }

    #[test]
    fn yaw_keeps_ground_draped() {
        let p = piece("Piece ground Texture A Point 0 0 Point 10 0 Point 10 10\n");
        let q = apply(&p, Delta::Rotate(Quat::from_rotation_y(0.7)), None, 0.0);
        let Shape::Ground { points } = q.shape else { panic!() };
        assert!(points.iter().all(|p| p.1.is_none()));
    }

    fn scale(p: &Piece, axis: usize, factor: f32) -> Piece {
        apply(p, Delta::Scale { axis, dir: axes(p, Mode::Scale)[axis], factor }, None, 0.0)
    }

    #[test]
    fn scaling_a_block_keeps_its_base_centre() {
        let p = piece("Piece block Texture A Centre 10 2 5 Size 4 6 8 Yaw 20\n");
        let Shape::Block { centre, size, .. } = scale(&p, 1, 2.0).shape else { panic!() };
        assert_eq!((centre, size), ([10.0, 2.0, 5.0], [4.0, 12.0, 8.0]));
    }

    #[test]
    fn scaling_a_wall_stretches_it_about_its_middle() {
        let p = piece("Piece wall Texture A From 0 0 0 To 10 0 0 Height 4\n");
        let Shape::Wall { from, to, .. } = scale(&p, 0, 2.0).shape else { panic!() };
        assert_eq!((from, to), ([-5.0, 0.0, 0.0], [15.0, 0.0, 0.0]));
        let Shape::Wall { height, .. } = scale(&p, 1, 0.5).shape else { panic!() };
        assert_eq!(height, 2.0);
    }

    #[test]
    fn scaling_a_plant_is_uniform_from_any_axis() {
        let p = piece("Piece plant Texture A Species tree Centre 0 0 0 Scale 1.5\n");
        for axis in 0..3 {
            let Shape::Plant { scale: k, .. } = scale(&p, axis, 2.0).shape else { panic!() };
            assert_eq!(k, 3.0);
        }
    }

    #[test]
    fn a_model_turns_about_y_and_scales_uniformly() {
        let p = piece("Piece model Name Tree-01-1 Centre 3 1 4 Scale 2 Yaw 350\n");
        assert_eq!(enabled(&p, Mode::Rotate), [false, true, false]);
        let q = apply(&p, Delta::Rotate(Quat::from_rotation_y(30f32.to_radians())), None, 0.0);
        let Shape::Model { yaw, centre, scale: sc, .. } = q.shape else { panic!() };
        assert!((yaw - 20.0).abs() < 1e-3 && centre == [3.0, 1.0, 4.0] && sc == 2.0, "{yaw}");
        for axis in 0..3 {
            let Shape::Model { scale: k, yaw, .. } = scale(&p, axis, 2.0).shape else { panic!() };
            assert_eq!((k, yaw), (4.0, 350.0));
        }
    }

    #[test]
    fn scaling_a_hill_changes_its_height_or_plan() {
        let p = piece("Piece hill Texture A Point 0 0 Point 10 0 Height 4 Width 6\n");
        let Shape::Hill { height, .. } = scale(&p, 1, 2.0).shape else { panic!() };
        assert_eq!(height, 8.0);
        let Shape::Hill { points, .. } = scale(&p, 0, 2.0).shape else { panic!() };
        assert_eq!(points.iter().map(|q| q.0).collect::<Vec<_>>(), vec![[-5.0, 0.0], [15.0, 0.0]]);
    }

    #[test]
    fn drag_maths() {
        // looking straight down at +X on the Y circle: the angle from +Z towards +X is 90 degrees
        let ray = (Vec3::new(5.0, 10.0, 0.0), Vec3::NEG_Y);
        assert!((plane_angle(Vec3::ZERO, 1, ray).unwrap() - std::f32::consts::FRAC_PI_2).abs() < 1e-5);
        // the X axis' nearest point to a ray crossing it at x = 3
        let t = axis_param(Vec3::ZERO, Vec3::X, (Vec3::new(3.0, 5.0, 0.0), Vec3::NEG_Y)).unwrap();
        assert!((t - 3.0).abs() < 1e-4);
        assert!((angle_diff(3.0, -3.0) - (std::f32::consts::TAU - 6.0)).abs() < 1e-5);
    }
}
