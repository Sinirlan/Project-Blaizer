//! REMASTER rim details (#337 phase 3, 09-remaster 9.23): lug nuts on the hub face, rivets on the
//! lip, and the centre cap's polygon / dome shape. Small parts laid over the dish like the
//! caliper and the star: prisms and domes in the wheel's own axes (x out of the rim face), each
//! a few dozen triangles.

use super::{strip, tri, WheelSpec, RimStyle, V};
use crate::scene::Vertex;
use glam::{Vec2, Vec3};
use std::f32::consts::{FRAC_PI_2, TAU};

/// A ring of lug nuts: `count` round the axle at `radius` (rim fractions), each `size` across
/// the corners (circumradius, rim fractions) with `sides` flats (0 = round), standing `height`
/// (× half-width) above the dish; `phase` turns the ring in lug pitches off the first spoke.
#[derive(Clone, Debug, PartialEq)]
pub struct Lugs {
    pub count: usize,
    pub radius: f32,
    pub size: f32,
    pub sides: usize,
    pub height: f32,
    pub phase: f32,
}

impl Default for Lugs {
    /// five hex nuts
    fn default() -> Self {
        Lugs { count: 5, radius: 0.14, size: 0.03, sides: 6, height: 0.06, phase: 0.0 }
    }
}

/// A ring of rivets: `count` low domes at `radius` (rim fractions, on the flange), `size` the
/// dome's radius (rim fractions); they sit half a pitch off the first spoke.
#[derive(Clone, Debug, PartialEq)]
pub struct Rivets {
    pub count: usize,
    pub radius: f32,
    pub size: f32,
}

impl Default for Rivets {
    fn default() -> Self {
        Rivets { count: 10, radius: 0.95, size: 0.014 }
    }
}

/// segments of a round lug
const ROUND: usize = 10;
/// segments round a rivet
const DOME_SIDES: usize = 8;

/// the (y, z) unit vector at angle `a` (y = sin a, z = cos a, like everywhere on the rim)
fn dir(a: f32) -> Vec2 {
    Vec2::new(a.sin(), a.cos())
}

/// The lug nuts: prisms from 2 mm under the dish's lowest point beneath them to `height` over
/// its highest, the top edge chamfered `ch`. Flats are sharp, round lugs smooth.
#[allow(clippy::too_many_arguments)]
pub(super) fn lugs(out: &mut Vec<Vertex>, side: f32, rr: f32, h: f32, top: &dyn Fn(f32) -> f32, phase: f32, l: &Lugs, ch: f32, c: [f32; 4]) {
    if l.count == 0 {
        return;
    }
    let (n, smooth) = if l.sides >= 3 { (l.sides.min(12), false) } else { (ROUND, true) };
    let (cr, sz) = (l.radius * rr, l.size * rr);
    let ch = ch.min(0.4 * sz);
    let v = |o: f32, q: Vec2, nrm: Vec3| V { p: Vec3::new(o, q.x, q.y), n: nrm.normalize_or(Vec3::X), c };
    for i in 0..l.count {
        let a = phase + (i as f32 + l.phase) * TAU / l.count as f32;
        let ctr = dir(a) * cr;
        let angle = |k: usize| a + k as f32 * TAU / n as f32;
        let corner = |k: usize, rad: f32| ctr + dir(angle(k)) * rad;
        let tops: Vec<f32> = std::iter::once(top(cr)).chain((0..n).map(|k| top(corner(k, sz).length()))).collect();
        let base = tops.iter().cloned().fold(f32::MAX, f32::min) - 0.002;
        let o_top = tops.iter().cloned().fold(f32::MIN, f32::max) + l.height * h;
        for k in 0..n {
            let j = (k + 1) % n;
            // outward normals of the two ends of this side: its flat's, or each corner's own
            let flat = dir(angle(k) + std::f32::consts::PI / n as f32);
            let (e0, e1) = if smooth { (dir(angle(k)), dir(angle(j))) } else { (flat, flat) };
            let side_n = |e: Vec2| Vec3::new(0.0, e.x, e.y);
            let slope_n = |e: Vec2| Vec3::new(0.7, e.x * 0.7, e.y * 0.7);
            let (p0, p1) = (corner(k, sz), corner(j, sz));
            let (q0, q1) = (corner(k, sz - ch), corner(j, sz - ch));
            // wall, chamfer, top
            let wall = [v(base, p0, side_n(e0)), v(base, p1, side_n(e1)), v(o_top - ch, p1, side_n(e1)), v(o_top - ch, p0, side_n(e0))];
            let bevel = [v(o_top - ch, p0, slope_n(e0)), v(o_top - ch, p1, slope_n(e1)), v(o_top, q1, slope_n(e1)), v(o_top, q0, slope_n(e0))];
            for q in [wall, bevel] {
                tri(out, side, [q[0], q[1], q[2]]);
                tri(out, side, [q[0], q[2], q[3]]);
            }
            tri(out, side, [v(o_top, ctr, Vec3::X), v(o_top, q0, Vec3::X), v(o_top, q1, Vec3::X)]);
        }
    }
}

/// The rivets: half-ellipsoid domes `0.6 size` high on the flange surface (`surface(radius)`
/// gives its offset at a radius), smooth normals.
pub(super) fn rivets(out: &mut Vec<Vertex>, side: f32, rr: f32, surface: &dyn Fn(f32) -> f32, phase: f32, r: &Rivets, c: [f32; 4]) {
    if r.count == 0 {
        return;
    }
    let (rad, s) = (r.radius * rr, r.size * rr);
    let hgt = 0.6 * s;
    let base = surface(rad);
    let base = if base < -1.0 { 0.0 } else { base } - 0.0008;
    // rings from the foot (theta 0) to the apex (theta 90 degrees)
    const RINGS: usize = 3;
    let ring = |j: usize| {
        let th = FRAC_PI_2 * j as f32 / RINGS as f32;
        (s * th.cos(), hgt * th.sin(), Vec2::new(th.cos() / s, th.sin() / hgt))
    };
    for i in 0..r.count {
        let a = phase + (i as f32 + 0.5) * TAU / r.count as f32;
        let ctr = dir(a) * rad;
        let pt = |j: usize, k: usize| {
            let (rho, z, nn) = ring(j);
            let d = dir(a + k as f32 * TAU / DOME_SIDES as f32);
            let n = Vec3::new(nn.y, d.x * nn.x, d.y * nn.x);
            V { p: Vec3::new(base + z, ctr.x + d.x * rho, ctr.y + d.y * rho), n: n.normalize_or(Vec3::X), c }
        };
        for j in 0..RINGS {
            for k in 0..DOME_SIDES {
                let (p, q, rr_, s_) = (pt(j, k), pt(j, k + 1), pt(j + 1, k + 1), pt(j + 1, k));
                tri(out, side, [p, q, rr_]);
                tri(out, side, [p, rr_, s_]);
            }
        }
    }
}

/// The profile fractions of the cap's dome (of its radius)
const CAP_RINGS: [f32; 5] = [0.0, 0.35, 0.6, 0.8, 1.0];

/// The centre cap when it is not the flat round one: a dome `cap_dome` high (× half-width) and
/// round (a lathe) or a polygon with `cap_sides` flat facets, its corners at the first spoke's
/// angle. The wall goes down into the star as the flat cap's does.
#[allow(clippy::too_many_arguments)]
pub(super) fn cap(out: &mut Vec<Vertex>, w: &WheelSpec, st: &RimStyle, phase: f32, rr: f32, h: f32, top: &dyn Fn(f32) -> f32, c: [f32; 4]) {
    let (cr, dome) = (st.cap * rr, st.cap_dome * h);
    let o = |f: f32, m: f32| top(m) + 0.004 + dome * (1.0 - f * f);
    let foot = top(cr) - 0.2 * h;
    if st.cap_sides < 3 {
        let mut prof: Vec<Vec2> = CAP_RINGS.iter().map(|&f| Vec2::new(o(f, f * cr), f * cr)).collect();
        prof.push(Vec2::new(foot, cr));
        strip(out, w, &prof, &vec![c; prof.len()], false);
        return;
    }
    let n = st.cap_sides.min(12);
    for k in 0..n {
        let (b0, b1) = (phase + k as f32 * TAU / n as f32, phase + (k + 1) as f32 * TAU / n as f32);
        let (d0, d1, e) = (dir(b0), dir(b1), dir((b0 + b1) * 0.5));
        let facet = |fa: f32, fb: f32| {
            let pt = |f: f32, d: Vec2| { let q = d * cr * f; Vec3::new(o(f, q.length()), q.x, q.y) };
            let (a, b, cc, dd) = (pt(fa, d0), pt(fa, d1), pt(fb, d1), pt(fb, d0));
            let mut nrm = (cc - a).cross(dd - b).normalize_or(Vec3::X);
            if nrm.x < 0.0 {
                nrm = -nrm;
            }
            [a, b, cc, dd].map(|p| V { p, n: nrm, c })
        };
        for j in 0..CAP_RINGS.len() - 1 {
            let q = facet(CAP_RINGS[j], CAP_RINGS[j + 1]);
            tri(out, w.side, [q[0], q[1], q[2]]);
            tri(out, w.side, [q[0], q[2], q[3]]);
        }
        let edge = o(1.0, cr);
        let n3 = Vec3::new(0.0, e.x, e.y);
        let wall = [(edge, d0), (edge, d1), (foot, d1), (foot, d0)].map(|(x, d)| V { p: Vec3::new(x, d.x * cr, d.y * cr), n: n3, c });
        tri(out, w.side, [wall[0], wall[1], wall[2]]);
        tri(out, w.side, [wall[0], wall[2], wall[3]]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RR: f32 = 0.353 * 0.7;
    const H: f32 = 0.153;
    const C: [f32; 4] = [0.8; 4];

    fn flat_top(_: f32) -> f32 {
        0.1
    }

    fn faces_out(v: &[Vertex]) -> bool {
        v.chunks(3).all(|t| {
            let p: Vec<Vec3> = t.iter().map(|v| Vec3::from(v.pos)).collect();
            let n: Vec3 = t.iter().map(|v| Vec3::from(v.normal)).sum();
            (p[1] - p[0]).cross(p[2] - p[0]).dot(n) > 0.0
        })
    }

    /// n round flats make a prism of 4 triangles per side plus the top fan's one; a round lug
    /// has ROUND sides; the nuts stay within `size` of their centres
    #[test]
    fn lugs_count_and_extent() {
        for (sides, n) in [(6, 6), (4, 4), (0, ROUND)] {
            let l = Lugs { count: 5, sides, ..Default::default() };
            let mut v = Vec::new();
            lugs(&mut v, 1.0, RR, H, &flat_top, 0.3, &l, 0.004, C);
            assert_eq!(v.len() / 3, 5 * n * 5, "sides {sides}");
            assert!(faces_out(&v));
            // every vertex is within `size` of one of the five centres
            for p in &v {
                let q = Vec2::new(p.pos[1], -p.pos[2]);
                let d = (0..5).map(|t| (q - dir(0.3 + t as f32 * TAU / 5.0) * l.radius * RR).length()).fold(f32::MAX, f32::min);
                assert!(d <= l.size * RR + 1e-5, "{d}");
            }
            let x: Vec<f32> = v.iter().map(|p| p.pos[0]).collect();
            let top = x.iter().cloned().fold(f32::MIN, f32::max);
            assert!((top - (0.1 + 0.06 * H)).abs() < 1e-5, "{top}");
        }
        let mut v = Vec::new();
        lugs(&mut v, 1.0, RR, H, &flat_top, 0.0, &Lugs { count: 0, ..Default::default() }, 0.004, C);
        assert!(v.is_empty());
    }

    /// a rivet is rings × sides × 2 triangles less the collapsed apex ones, all on the flange
    #[test]
    fn rivets_build() {
        let r = Rivets { count: 6, radius: 0.95, size: 0.02 };
        let mut v = Vec::new();
        rivets(&mut v, -1.0, RR, &|_| 0.2, 0.0, &r, C);
        assert_eq!(v.len() / 3, 6 * (DOME_SIDES * 2 * 3 - DOME_SIDES));
        assert!(faces_out(&v));
        let x = v.iter().map(|p| p.pos[0].abs()).fold(0.0f32, f32::max);
        assert!((x - (0.2 - 0.0008 + 0.6 * 0.02 * RR)).abs() < 1e-5, "{x}");
    }
}
