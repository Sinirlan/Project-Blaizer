//! REMASTER cage edits before the bevel (09-remaster 9.22, nose kit after the user's Audi R8
//! reference): a corner of a face cut off into a facet of its neighbour's look (the grille's
//! hexagonal "single frame"), and a face split into a frame and an inner face of its own tag
//! (framed intakes, recessed by `super::vents`). All of it a design choice.
//!
//! Faces are in car-local game space; the edits keep shared edges shared (new points go into
//! every face along the cut edge), so the body stays welded.

use super::body::{Face, weld_key};
use glam::Vec3;

/// Cut a corner off the faces of colour `from` at `corner` (game space, mirrored to -x too).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Chamfer {
    pub from: [u8; 3],
    pub corner: [f32; 3],
    /// how far the cut runs along the corner's edge that runs more across the car (x), and
    /// along its other edge (m)
    pub across: f32,
    pub along: f32,
}

/// Split the faces of colour `from` whose centre (|x|) lies in a box into a frame `frame` (m)
/// wide, keeping their look, round an inner face tagged `rgb`: matte black, unpainted, the
/// floor of an opening (a vent of that tag recesses it).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Panel {
    pub from: [u8; 3],
    pub lo: [f32; 3],
    pub hi: [f32; 3],
    pub frame: f32,
    pub rgb: [u8; 3],
}

/// the inner face of a panel: the black of the MYO vents (1 1 1) [O], no reflection
const OPENING: [f32; 4] = [0.02, 0.02, 0.02, 1.0];
/// weld grid (m), as `body`
const WELD: f32 = 1e-3;

fn same(a: [f32; 3], b: [f32; 3]) -> bool {
    a.iter().zip(&b).all(|(x, y)| ((x - y) / WELD).abs() < 0.5)
}

/// Corner `k` of `f` with consecutive duplicates skipped: the previous and next distinct corners.
fn neighbours(f: &Face, k: usize) -> Option<(usize, usize)> {
    let n = f.pts.len();
    let prev = (1..n).map(|d| (k + n - d) % n).find(|&j| !same(f.pts[j], f.pts[k]))?;
    let next = (1..n).map(|d| (k + d) % n).find(|&j| !same(f.pts[j], f.pts[k]))?;
    Some((prev, next))
}

/// Insert `x` (on edge p–q) into every face with that edge, colour and uv interpolated.
fn split_edge(faces: &mut [Face], p: [f32; 3], q: [f32; 3], x: Vec3) {
    for f in faces.iter_mut() {
        let n = f.pts.len();
        let Some(i) = (0..n).find(|&i| {
            let (a, b) = (f.pts[i], f.pts[(i + 1) % n]);
            (same(a, p) && same(b, q)) || (same(a, q) && same(b, p))
        }) else {
            continue;
        };
        let j = (i + 1) % n;
        let (a, b) = (Vec3::from(f.pts[i]), Vec3::from(f.pts[j]));
        let t = ((x - a).dot(b - a) / (b - a).length_squared()).clamp(0.0, 1.0);
        let colour = std::array::from_fn(|c| f.colour[i][c] + (f.colour[j][c] - f.colour[i][c]) * t);
        let uv = std::array::from_fn(|c| f.uv[i][c] + (f.uv[j][c] - f.uv[i][c]) * t);
        f.pts.insert(i + 1, x.to_array());
        f.colour.insert(i + 1, colour);
        f.uv.insert(i + 1, uv);
    }
}

/// Apply the chamfers: the corner becomes a triangle in the face's plane with the look of the
/// face across its `along` edge (the panel beside a grille), the face keeps the rest.
pub fn chamfer(faces: &mut Vec<Face>, cuts: &[Chamfer]) {
    for c in cuts {
        let mirror = [-c.corner[0], c.corner[1], c.corner[2]];
        for corner in if c.corner[0].abs() > WELD { vec![c.corner, mirror] } else { vec![c.corner] } {
            let Some((fi, k)) = faces.iter().enumerate().filter(|(_, f)| f.rgb == c.from).find_map(|(i, f)| Some((i, f.pts.iter().position(|&p| same(p, corner))?))) else {
                continue;
            };
            let Some((prev, next)) = neighbours(&faces[fi], k) else { continue };
            let f = &faces[fi];
            let p = Vec3::from(f.pts[k]);
            let (qp, qn) = (Vec3::from(f.pts[prev]), Vec3::from(f.pts[next]));
            let across_prev = (qp - p).normalize_or_zero().x.abs() > (qn - p).normalize_or_zero().x.abs();
            let (dp, dn) = if across_prev { (c.across, c.along) } else { (c.along, c.across) };
            let a = p + (qp - p).clamp_length_max(dp.min(0.45 * qp.distance(p)));
            let b = p + (qn - p).clamp_length_max(dn.min(0.45 * qn.distance(p)));
            // the look across the `along` edge
            let side = if across_prev { qn } else { qp };
            let look = faces.iter().enumerate().find(|&(i, g)| i != fi && g.pts.iter().any(|&x| same(x, p.to_array())) && g.pts.iter().any(|&x| same(x, side.to_array()))).map(|(_, g)| g.clone());
            let f = &faces[fi];
            let mut tri = Face { pts: vec![a.to_array(), p.to_array(), b.to_array()], colour: vec![f.colour[k]; 3], uv: vec![f.uv[k]; 3], ..f.clone() };
            if let Some(g) = look {
                (tri.colour, tri.layer, tri.env, tri.rgb) = (vec![g.colour[0]; 3], -1.0, g.env, g.rgb);
            }
            let (pk, qpa, qna) = (f.pts[k], qp.to_array(), qn.to_array());
            let f = &mut faces[fi];
            f.pts.splice(k..=k, [a.to_array(), b.to_array()]);
            let (col, uv) = (f.colour[k], f.uv[k]);
            f.colour.insert(k, col);
            f.uv.insert(k, uv);
            // the neighbours along the two cut edges get the new points (the cut face has
            // neither edge any more)
            split_edge(faces, pk, qpa, a);
            split_edge(faces, pk, qna, b);
            faces.push(tri);
        }
    }
}

/// Apply the panels: per picked face, a frame of quads in its own look and the inner face
/// (same plane, corners moved `frame` inside both their edges) tagged with the panel's `rgb`.
pub fn panels(faces: &mut Vec<Face>, panels: &[Panel]) {
    for pn in panels {
        let (lo, hi) = (Vec3::from(pn.lo), Vec3::from(pn.hi));
        let mut out = Vec::new();
        for f in faces.drain(..) {
            let c = f.pts.iter().map(|&p| Vec3::from(p)).sum::<Vec3>() / f.pts.len() as f32;
            let c = Vec3::new(c.x.abs(), c.y, c.z);
            if f.rgb != pn.from || !c.cmpge(lo).all() || !c.cmple(hi).all() {
                out.push(f);
                continue;
            }
            // distinct corners, CCW round the plane normal
            let mut o: Vec<usize> = (0..f.pts.len()).filter(|&k| !same(f.pts[k], f.pts[(k + 1) % f.pts.len()])).collect();
            let n = Vec3::from(f.normal).normalize_or(Vec3::Y);
            let area: Vec3 = (0..o.len()).map(|i| Vec3::from(f.pts[o[i]]).cross(Vec3::from(f.pts[o[(i + 1) % o.len()]]))).sum();
            if area.dot(n) < 0.0 {
                o.reverse();
            }
            let m = o.len();
            if m < 3 {
                out.push(f);
                continue;
            }
            let at = |i: usize| Vec3::from(f.pts[o[i % m]]);
            let inner: Vec<Vec3> = (0..m)
                .map(|i| {
                    let (prev, v, next) = (at(i + m - 1), at(i), at(i + 1));
                    let n1 = n.cross((v - prev).normalize_or_zero());
                    let n2 = n.cross((next - v).normalize_or_zero());
                    let d = n1.dot(n2);
                    let off = if d < -0.95 { n1 * pn.frame } else { (n1 + n2) * (pn.frame / (1.0 + d)) };
                    v + off.clamp_length_max(4.0 * pn.frame)
                })
                .collect();
            for i in 0..m {
                let j = (i + 1) % m;
                out.push(Face {
                    pts: vec![at(i).to_array(), at(j).to_array(), inner[j].to_array(), inner[i].to_array()],
                    colour: vec![f.colour[o[i]], f.colour[o[j]], f.colour[o[j]], f.colour[o[i]]],
                    uv: vec![f.uv[o[i]], f.uv[o[j]], f.uv[o[j]], f.uv[o[i]]],
                    ..f.clone()
                });
            }
            out.push(Face { pts: inner.iter().map(|p| p.to_array()).collect(), colour: vec![OPENING; m], uv: vec![[0.0; 2]; m], layer: -1.0, env: 0.0, rgb: pn.rgb, ..f.clone() });
        }
        *faces = out;
    }
}

/// Copy the faces lying wholly inside a box to the other side (x negated): the Titan's single
/// (left) mirror gets a right-hand twin (9.22 phase 5).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MirrorCopy {
    pub lo: [f32; 3],
    pub hi: [f32; 3],
}

/// Move every cage point inside a box by `by`, and the points inside the box's x mirror by
/// `by` with x negated (deeper side scoops). Every face holding a point moves it, so the body
/// stays welded.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Push {
    pub lo: [f32; 3],
    pub hi: [f32; 3],
    pub by: [f32; 3],
}

/// REMASTER (#465): every face reaching into a box (x as |x|: both sides) cut by the plane
/// `axis` (0 x, 1 y, 2 z) = `at` (for x: at +`at` and -`at`), the cut points put into every face
/// along the cut edges, so the cage stays welded. A `Smooth` zone rounds or keeps a fold per
/// cage edge, so a fold that is to stay a crease over part of its length and round over the
/// rest (the ASC's scoop lips, running back to the rear arch) is cut where that changes, and
/// each piece's box says which (`Smooth::max_degs`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Slice {
    pub lo: [f32; 3],
    pub hi: [f32; 3],
    pub axis: usize,
    pub at: f32,
}

/// Rebuild the faces lying wholly inside a box as a plate of even thickness: the upward faces
/// (the skin) stay, the rest (the original's tapering walls) go, and the skin gets a copy
/// `thickness` under it (each corner moved along the skin's normal, mitred at the creases),
/// facing down, and walls round its outline. Before that, `splits` add corners on skin edges;
/// with `legs` the plate's innermost edge (|x| within `LEG_EDGE` of its least, per side)
/// runs down to the cage below as a leg as thick as the plate. Mirrored to -x like the cage.
/// (The Titan's wing halves are shells open underneath and along the leading edge, their back
/// walls 4 cm tall [O].)
#[derive(Clone, Debug, PartialEq)]
pub struct Thicken {
    pub lo: [f32; 3],
    pub hi: [f32; 3],
    pub thickness: f32,
    /// (edge end, edge end, new corner between them), game space at +x or -x
    pub splits: Vec<([f32; 3], [f32; 3], [f32; 3])>,
    pub legs: bool,
}

/// how close to the plate's innermost |x| an edge's ends must be to grow a leg (m)
const LEG_EDGE: f32 = 0.01;
/// how far the legs reach into the surface below (m)
const LEG_BURY: f32 = 0.01;

/// Faces of colour `from` whose centre (|x|) lies in a box (carbon parts).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Pick {
    pub from: [u8; 3],
    pub lo: [f32; 3],
    pub hi: [f32; 3],
}

/// Faces of colour `from` whose centre (|x|) lies in a box take the look (colour, texture,
/// finish) and the tag of the first face tagged `like`: the R-Build 2's trim-coloured deck
/// painted like its wing wedges (#171 phase 3, the user's request).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Paint {
    pub from: [u8; 3],
    pub lo: [f32; 3],
    pub hi: [f32; 3],
    pub like: [u8; 3],
}

fn inside(p: [f32; 3], lo: [f32; 3], hi: [f32; 3]) -> bool {
    (0..3).all(|i| p[i] >= lo[i] && p[i] <= hi[i])
}

/// the plane normal of a polygon (Newell), turned to the side of `like`
fn newell(pts: &[[f32; 3]], like: Vec3) -> Vec3 {
    let n: Vec3 = (0..pts.len()).map(|i| Vec3::from(pts[i]).cross(Vec3::from(pts[(i + 1) % pts.len()]))).sum();
    let n = n.normalize_or(like);
    if n.dot(like) < 0.0 { -n } else { n }
}

/// Apply the mirror copies.
pub fn mirror_copies(faces: &mut Vec<Face>, copies: &[MirrorCopy]) {
    for c in copies {
        let twins: Vec<Face> = faces
            .iter()
            .filter(|f| f.pts.iter().all(|&p| inside(p, c.lo, c.hi)))
            .map(|f| {
                let flip = |p: [f32; 3]| [-p[0], p[1], p[2]];
                Face {
                    pts: f.pts.iter().rev().map(|&p| flip(p)).collect(),
                    normal: flip(f.normal),
                    colour: f.colour.iter().rev().copied().collect(),
                    uv: f.uv.iter().rev().copied().collect(),
                    ..f.clone()
                }
            })
            .collect();
        faces.extend(twins);
    }
}

/// Apply the pushes; the moved faces get their plane normals again.
pub fn push(faces: &mut [Face], pushes: &[Push]) {
    for ps in pushes {
        let mirror = |p: [f32; 3]| [-p[0], p[1], p[2]];
        let (mlo, mhi) = ([-ps.hi[0], ps.lo[1], ps.lo[2]], [-ps.lo[0], ps.hi[1], ps.hi[2]]);
        for f in faces.iter_mut() {
            let mut moved = false;
            for p in f.pts.iter_mut() {
                if inside(*p, ps.lo, ps.hi) {
                    *p = (Vec3::from(*p) + Vec3::from(ps.by)).to_array();
                    moved = true;
                } else if inside(*p, mlo, mhi) {
                    *p = (Vec3::from(*p) + Vec3::from(mirror(ps.by))).to_array();
                    moved = true;
                }
            }
            if moved {
                f.normal = newell(&f.pts, Vec3::from(f.normal)).to_array();
            }
        }
    }
}

/// Apply the slices (before the pushes, so a pushed corner bends a face only up to the cut).
pub fn slice(faces: &mut Vec<Face>, slices: &[Slice]) {
    /// how close (m) a face must come to the box to count as reaching into it
    const ON: f32 = 1e-5;
    for s in slices {
        let reaches = |f: &Face| {
            let lo = |i: usize| f.pts.iter().map(|p| p[i]).fold(f32::MAX, f32::min);
            let hi = |i: usize| f.pts.iter().map(|p| p[i]).fold(f32::MIN, f32::max);
            let (x0, x1) = (lo(0), hi(0));
            let (ax0, ax1) = if x0 < 0.0 && x1 > 0.0 { (0.0, x1.max(-x0)) } else { (x0.abs().min(x1.abs()), x0.abs().max(x1.abs())) };
            ax1 > s.lo[0] + ON && ax0 < s.hi[0] - ON && (1..3).all(|i| hi(i) > s.lo[i] + ON && lo(i) < s.hi[i] - ON)
        };
        let planes: &[f32] = if s.axis == 0 { &[s.at, -s.at] } else { &[s.at] };
        let mut made: Vec<[f32; 3]> = Vec::new();
        let mut out = Vec::with_capacity(faces.len());
        for f in faces.drain(..) {
            if !reaches(&f) {
                out.push(f);
                continue;
            }
            let mut pieces = vec![f];
            for &c in planes {
                let mut next = Vec::new();
                for g in pieces {
                    for g in super::diffuser::convex_pieces(g) {
                        match super::diffuser::split(&g, s.axis, c, &mut made) {
                            Some((a, b)) => next.extend([a, b]),
                            None => next.push(g),
                        }
                    }
                }
                pieces = next;
            }
            out.extend(pieces);
        }
        // the neighbours take the cut points on their edges
        made.sort_by(|a, b| a[0].total_cmp(&b[0]).then(a[1].total_cmp(&b[1])).then(a[2].total_cmp(&b[2])));
        made.dedup_by(|a, b| weld_key(*a) == weld_key(*b));
        *faces = out.iter().map(|f| super::diffuser::close_t(f, &made)).collect();
    }
}

/// REMASTER (#388): a smooth bulge of the finished cage. Every point moves by `by` (m) times
/// three weights, one per axis: `x` of |x| (the points on the other side move mirrored), `y` of
/// y, `z` of z, each a table of `(coordinate, weight)` interpolated along a smooth curve with
/// level ends and held at the first and last weight beyond them; a missing table weighs 1. The
/// tables bound the bulge: one that falls to 0 at both ends leaves the rest of the car where it
/// was, so the faces stay welded (the same point always moves the same way). Run after the
/// refine's cuts, because a cage outline of three corners cannot form an arc: it is the cut
/// points along it that the bulge carries onto the curve. The Serpent's rear corner.
#[derive(Clone, Debug, PartialEq)]
pub struct Warp {
    pub by: [f32; 3],
    pub x: Vec<[f32; 2]>,
    pub y: Vec<[f32; 2]>,
    pub z: Vec<[f32; 2]>,
}

/// The weight of `v` on a `Warp` table: a cubic Hermite curve through the points, level at the
/// first and last (so the weight leaves its end value without a kink) and monotone between
/// two points that agree on their direction, the end values beyond.
fn table_weight(t: &[[f32; 2]], v: f32) -> f32 {
    let (Some(first), Some(last)) = (t.first(), t.last()) else { return 1.0 };
    if v <= first[0] {
        return first[1];
    }
    if v >= last[0] {
        return last[1];
    }
    let i = t.windows(2).position(|w| v < w[1][0]).unwrap_or(t.len() - 2);
    let (a, b) = (t[i], t[i + 1]);
    let h = (b[0] - a[0]).max(1e-6);
    // the slope at a node (Fritsch-Carlson, monotone: no overshoot past a table value): the
    // weighted harmonic mean of the two secants, 0 at a turn and at the ends
    let slope = |k: usize| {
        if k == 0 || k == t.len() - 1 {
            return 0.0;
        }
        let (h0, h1) = (t[k][0] - t[k - 1][0], t[k + 1][0] - t[k][0]);
        let (d0, d1) = ((t[k][1] - t[k - 1][1]) / h0.max(1e-6), (t[k + 1][1] - t[k][1]) / h1.max(1e-6));
        if d0 * d1 <= 0.0 {
            return 0.0;
        }
        let (w0, w1) = (2.0 * h1 + h0, h1 + 2.0 * h0);
        (w0 + w1) / (w0 / d0 + w1 / d1)
    };
    let (m0, m1) = (slope(i) * h, slope(i + 1) * h);
    let s = (v - a[0]) / h;
    let (s2, s3) = (s * s, s * s * s);
    (2.0 * s3 - 3.0 * s2 + 1.0) * a[1] + (s3 - 2.0 * s2 + s) * m0 + (3.0 * s2 - 2.0 * s3) * b[1] + (s3 - s2) * m1
}

/// Where `p` (game space) lands after the warps, applied in order like `warp`. The warp is a pure
/// function of position, so the added parts built from the unwarped cage (lamp insides, vent
/// slats, the lit strips, the halo anchors) follow the body through it (the Serpent's tail lamps
/// sit in its rear bulge).
pub fn warp_point(p: [f32; 3], warps: &[Warp]) -> [f32; 3] {
    let mut p = p;
    for w in warps {
        let k = table_weight(&w.x, p[0].abs()) * table_weight(&w.y, p[1]) * table_weight(&w.z, p[2]);
        if k != 0.0 {
            let side = if p[0] < 0.0 { -1.0 } else { 1.0 };
            p = [p[0] + w.by[0] * side * k, p[1] + w.by[1] * k, p[2] + w.by[2] * k];
        }
    }
    p
}

/// Apply the warps; the moved faces get their plane normals again.
pub fn warp(faces: &mut [Face], warps: &[Warp]) {
    for w in warps {
        for f in faces.iter_mut() {
            let mut moved = false;
            for p in f.pts.iter_mut() {
                let k = table_weight(&w.x, p[0].abs()) * table_weight(&w.y, p[1]) * table_weight(&w.z, p[2]);
                if k != 0.0 {
                    let side = if p[0] < 0.0 { -1.0 } else { 1.0 };
                    *p = [p[0] + w.by[0] * side * k, p[1] + w.by[1] * k, p[2] + w.by[2] * k];
                    moved = true;
                }
            }
            if moved {
                f.normal = newell(&f.pts, Vec3::from(f.normal)).to_array();
            }
        }
    }
}

/// The y of every cage surface above or below (x, z), highest first.
pub(super) fn hits(faces: &[Face], x: f32, z: f32) -> Vec<f32> {
    let mut ys: Vec<f32> = Vec::new();
    for f in faces {
        let p: Vec<Vec3> = f.pts.iter().map(|&p| Vec3::from(p)).collect();
        for k in 1..p.len().saturating_sub(1) {
            let (a, b, c) = (p[0], p[k], p[k + 1]);
            // barycentric in the xz plane
            let (ab, ac, aq) = (glam::Vec2::new(b.x - a.x, b.z - a.z), glam::Vec2::new(c.x - a.x, c.z - a.z), glam::Vec2::new(x - a.x, z - a.z));
            let d = ab.perp_dot(ac);
            if d.abs() < 1e-9 {
                continue;
            }
            let (v, w) = (aq.perp_dot(ac) / d, ab.perp_dot(aq) / d);
            if v >= 0.0 && w >= 0.0 && v + w <= 1.0 {
                ys.push(a.y + v * (b.y - a.y) + w * (c.y - a.y));
            }
        }
    }
    ys.sort_by(|a, b| b.total_cmp(a));
    ys.dedup_by(|a, b| (*a - *b).abs() < 1e-3);
    ys
}

/// A face in `src`'s look (its first corner's colour and uv) wound to face `out`.
pub(super) fn facing(src: &Face, mut pts: Vec<[f32; 3]>, out: Vec3) -> Face {
    let raw: Vec3 = (0..pts.len()).map(|i| Vec3::from(pts[i]).cross(Vec3::from(pts[(i + 1) % pts.len()]))).sum();
    if raw.dot(out) < 0.0 {
        pts.reverse();
    }
    let normal = newell(&pts, out).to_array();
    let n = pts.len();
    Face { pts, normal, colour: vec![src.colour[0]; n], uv: vec![src.uv[0]; n], ..src.clone() }
}

/// Apply the thickens. The bottom, the walls and the legs take the look of the skin face they
/// hang from, so the plate reads the same from below as from above.
pub fn thicken(faces: &mut Vec<Face>, thickens: &[Thicken]) {
    let key = |p: [f32; 3]| p.map(|c| (c / WELD).round() as i64);
    let edge_key = |a: [i64; 3], b: [i64; 3]| if a < b { (a, b) } else { (b, a) };
    for t in thickens {
        let picked = |f: &Face| f.pts.iter().all(|&p| inside(p, t.lo, t.hi));
        let mut skin: Vec<Face> = faces.iter().filter(|f| picked(f) && newell(&f.pts, Vec3::from(f.normal)).y > 0.5).cloned().collect();
        if skin.is_empty() {
            continue;
        }
        faces.retain(|f| !picked(f));
        let flip = |p: [f32; 3]| [-p[0], p[1], p[2]];
        for &(a, b, c) in &t.splits {
            for (a, b, c) in [(a, b, c), (flip(a), flip(b), flip(c))] {
                for f in skin.iter_mut() {
                    let n = f.pts.len();
                    let Some(i) = (0..n).find(|&i| { let (p, q) = (f.pts[i], f.pts[(i + 1) % n]); (same(p, a) && same(q, b)) || (same(p, b) && same(q, a)) }) else { continue };
                    f.pts.insert(i + 1, c);
                    f.colour.insert(i + 1, f.colour[i]);
                    f.uv.insert(i + 1, f.uv[i]);
                    f.normal = newell(&f.pts, Vec3::from(f.normal)).to_array();
                }
            }
        }
        // per corner: the skin normals round it -> the mitred offset to the underside
        let mut normals: std::collections::HashMap<[i64; 3], Vec<Vec3>> = Default::default();
        for f in &skin {
            let n = newell(&f.pts, Vec3::Y);
            for &p in &f.pts {
                let e = normals.entry(key(p)).or_default();
                if !e.iter().any(|m| m.dot(n) > 0.9999) {
                    e.push(n);
                }
            }
        }
        let below = |p: [f32; 3]| {
            let ns = &normals[&key(p)];
            let avg = ns.iter().sum::<Vec3>().normalize_or(Vec3::Y);
            let cos = ns.iter().map(|n| n.dot(avg)).fold(1.0, f32::min).max(0.5);
            (Vec3::from(p) - avg * (t.thickness / cos)).to_array()
        };
        // undirected edge uses within the skin: the outline is used once
        let mut uses: std::collections::HashMap<([i64; 3], [i64; 3]), usize> = Default::default();
        for f in &skin {
            for i in 0..f.pts.len() {
                let (a, b) = (key(f.pts[i]), key(f.pts[(i + 1) % f.pts.len()]));
                if a != b {
                    *uses.entry(edge_key(a, b)).or_default() += 1;
                }
            }
        }
        // the innermost |x| per side, for the legs
        let inner = |side: f32| skin.iter().flat_map(|f| f.pts.iter()).filter(|p| p[0] * side > 0.0).map(|p| p[0].abs()).fold(f32::MAX, f32::min);
        let inner = [inner(-1.0), inner(1.0)];
        let at_inner = |p: [f32; 3]| p[0].abs() < inner[usize::from(p[0] > 0.0)] + LEG_EDGE;
        let ground = |p: [f32; 3]| hits(faces, p[0], p[2]).into_iter().find(|&y| y < p[1] - 1e-3).map(|y| [p[0], y - LEG_BURY, p[2]]);
        let mut added = Vec::new();
        for f in &skin {
            let n = f.pts.len();
            let pts: Vec<[f32; 3]> = f.pts.iter().rev().map(|&p| below(p)).collect();
            let normal = newell(&pts, -Vec3::from(f.normal)).to_array();
            added.push(Face { pts, normal, colour: f.colour.iter().rev().copied().collect(), uv: f.uv.iter().rev().copied().collect(), ..f.clone() });
            // walls: the skin runs a -> b counter-clockwise round its normal, so outward is
            // (b - a) x normal; the wall goes b -> a -> a below -> b below
            for i in 0..n {
                let j = (i + 1) % n;
                let (a, b) = (key(f.pts[i]), key(f.pts[j]));
                if a == b || uses[&edge_key(a, b)] != 1 {
                    continue;
                }
                let (pa, pb) = (f.pts[i], f.pts[j]);
                let out = (Vec3::from(pb) - Vec3::from(pa)).cross(Vec3::from(f.normal));
                let (ua, ub) = (below(pa), below(pb));
                let leg = (t.legs && at_inner(pa) && at_inner(pb)).then(|| [pa, pb, ua, ub].map(ground));
                if let Some([Some(ga), Some(gb), Some(gua), Some(gub)]) = leg {
                    // the leg: the edge's two faces down to the ground, capped at both ends
                    let along = (Vec3::from(pb) - Vec3::from(pa)).normalize_or_zero();
                    added.push(facing(f, vec![pb, pa, ga, gb], out));
                    added.push(facing(f, vec![ua, ub, gub, gua], -out));
                    added.push(facing(f, vec![pa, ua, gua, ga], -along));
                    added.push(facing(f, vec![pb, ub, gub, gb], along));
                    continue;
                }
                let pts = vec![pb, pa, ua, ub];
                let normal = newell(&pts, out).to_array();
                added.push(Face { pts, normal, colour: vec![f.colour[j], f.colour[i], f.colour[i], f.colour[j]], uv: vec![f.uv[j], f.uv[i], f.uv[i], f.uv[j]], ..f.clone() });
            }
        }
        faces.extend(skin);
        faces.extend(added);
    }
}

/// Apply the paints (after the panels, before the carbon): a paint whose donor tag is on no
/// face does nothing.
pub fn paint(faces: &mut [Face], paints: &[Paint]) {
    for pt in paints {
        let Some(donor) = faces.iter().find(|f| f.rgb == pt.like).cloned() else { continue };
        for f in faces.iter_mut() {
            let c = f.pts.iter().map(|&p| Vec3::from(p)).sum::<Vec3>() / f.pts.len() as f32;
            if f.rgb != pt.from || !inside([c.x.abs(), c.y, c.z], pt.lo, pt.hi) {
                continue;
            }
            let n = f.pts.len();
            f.colour = vec![donor.colour[0]; n];
            f.uv = vec![donor.uv[0]; n];
            (f.layer, f.env, f.rgb) = (donor.layer, donor.env, donor.rgb);
        }
    }
}

/// Retag the picked faces as carbon fibre (texture layer `layer`, `super::CARBON_ENV`), mapped
/// flat onto the plane they face most, one weave tile every `super::CARBON_TILE` metres.
pub fn carbon(faces: &mut [Face], picks: &[Pick], layer: f32) {
    for pk in picks {
        for f in faces.iter_mut() {
            let c = f.pts.iter().map(|&p| Vec3::from(p)).sum::<Vec3>() / f.pts.len() as f32;
            if f.rgb != pk.from || !inside([c.x.abs(), c.y, c.z], pk.lo, pk.hi) {
                continue;
            }
            f.uv = f.pts.iter().map(|&p| super::carbon_uv(Vec3::from(p), Vec3::from(f.normal))).collect();
            f.colour = vec![super::CARBON_TINT; f.pts.len()];
            (f.layer, f.env) = (layer, super::CARBON_ENV);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn face(pts: &[[f32; 3]], rgb: [u8; 3]) -> Face {
        Face { pts: pts.to_vec(), normal: [0.0, 0.0, 1.0], colour: vec![[0.5; 4]; pts.len()], uv: vec![[0.0; 2]; pts.len()], layer: -1.0, env: 0.0, rgb, smooth_deg: 0.0, remesh: 0.0 }
    }

    /// welded edges: every directed edge has its reverse in another face
    fn closed(faces: &[Face]) -> bool {
        let edges: Vec<([f32; 3], [f32; 3])> = faces.iter().flat_map(|f| (0..f.pts.len()).map(|i| (f.pts[i], f.pts[(i + 1) % f.pts.len()]))).collect();
        edges.iter().all(|&(a, b)| edges.iter().any(|&(c, d)| same(a, d) && same(b, c)))
    }

    /// a grille quad between two panels and a floor, cut at its lower right corner: the cut
    /// face, the triangle in the right panel's look, the panel and the floor get the new
    /// points, so the mesh stays closed
    #[test]
    fn chamfer_keeps_the_mesh_welded() {
        // a flat square split in four: grille (1), panel right (2), floor below (3), top (4)
        let g = face(&[[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [1.0, 1.0, 0.0], [0.0, 1.0, 0.0]], [1; 3]);
        let p = face(&[[1.0, 0.0, 0.0], [2.0, 0.0, 0.0], [2.0, 1.0, 0.0], [1.0, 1.0, 0.0]], [2; 3]);
        let mut faces = vec![g, p];
        chamfer(&mut faces, &[Chamfer { from: [1; 3], corner: [1.0, 0.0, 0.0], across: 0.2, along: 0.4 }]);
        assert_eq!(faces.len(), 3);
        let g = faces.iter().find(|f| f.rgb == [1; 3] && f.pts.len() == 5).expect("cut grille");
        assert!(g.pts.iter().any(|&p| same(p, [0.8, 0.0, 0.0])) && g.pts.iter().any(|&p| same(p, [1.0, 0.4, 0.0])));
        let t = faces.iter().find(|f| f.pts.len() == 3).expect("corner");
        assert_eq!(t.rgb, [2; 3]);
        let p = faces.iter().find(|f| f.rgb == [2; 3] && f.pts.len() == 5).expect("panel with the new point");
        assert!(p.pts.iter().any(|&q| same(q, [1.0, 0.4, 0.0])));
    }

    /// a square panel: four frame quads and the tagged inner face `frame` inside, closed
    #[test]
    fn panel_frame() {
        let mut faces = vec![face(&[[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [1.0, 1.0, 0.0], [0.0, 1.0, 0.0]], [2; 3])];
        panels(&mut faces, &[Panel { from: [2; 3], lo: [0.0; 3], hi: [1.0; 3], frame: 0.1, rgb: [9; 3] }]);
        assert_eq!(faces.len(), 5);
        let inner = faces.iter().find(|f| f.rgb == [9; 3]).expect("inner");
        for p in &inner.pts {
            assert!((p[0] - 0.1).abs() < 1e-5 || (p[0] - 0.9).abs() < 1e-5, "{p:?}");
            assert!((p[1] - 0.1).abs() < 1e-5 || (p[1] - 0.9).abs() < 1e-5, "{p:?}");
        }
        let mut open = faces.clone();
        open.push(face(&[[0.0, 0.0, 0.0], [0.0, 1.0, 0.0], [1.0, 1.0, 0.0], [1.0, 0.0, 0.0]], [0; 3]));
        assert!(closed(&open));
    }

    /// a box at x < 0 copied to the right: the copy is its mirror image, facing the other way
    #[test]
    fn mirror_copy_is_symmetric() {
        let mut faces = vec![face(&[[-1.0, 0.0, 0.0], [-0.5, 0.0, 0.0], [-0.5, 1.0, 0.0]], [5; 3]), face(&[[0.5, 0.0, 0.0], [1.0, 0.0, 0.0], [1.0, 1.0, 0.0]], [6; 3])];
        faces[0].normal = [-1.0, 0.0, 0.0];
        mirror_copies(&mut faces, &[MirrorCopy { lo: [-1.1, -0.1, -0.1], hi: [-0.4, 1.1, 0.1] }]);
        assert_eq!(faces.len(), 3);
        let c = &faces[2];
        assert_eq!((c.rgb, c.normal), ([5; 3], [1.0, 0.0, 0.0]));
        for p in &faces[0].pts {
            assert!(c.pts.iter().any(|&q| same(q, [-p[0], p[1], p[2]])));
        }
    }

    /// a push moves the points in its box and their mirrors, in every face holding them, and
    /// nothing else
    #[test]
    fn push_moves_box_points_only() {
        let q = |x0: f32, x1: f32| face(&[[x0, 0.0, 0.0], [x1, 0.0, 0.0], [x1, 1.0, 0.0], [x0, 1.0, 0.0]], [1; 3]);
        let mut faces = vec![q(0.0, 0.5), q(0.5, 1.0), q(-0.5, 0.0), q(-1.0, -0.5)];
        push(&mut faces, &[Push { lo: [0.4, -0.1, -0.1], hi: [0.6, 0.1, 0.1], by: [0.0, 0.0, -0.2] }]);
        let moved: Vec<[f32; 3]> = faces.iter().flat_map(|f| f.pts.iter().copied()).filter(|p| p[2] != 0.0).collect();
        assert!(moved.iter().all(|p| same(*p, [0.5, 0.0, -0.2]) || same(*p, [-0.5, 0.0, -0.2])), "{moved:?}");
        assert_eq!(moved.len(), 4);
    }

    /// two quads side by side along z, sliced across the first one: the plane cuts it in two, the
    /// cut point goes into the neighbour's edge too (still welded), and a face out of the box stays
    #[test]
    fn slice_cuts_and_closes() {
        let quad = |x0: f32, x1: f32, rgb: [u8; 3]| face(&[[x0, 0.5, -1.0], [x1, 0.5, -1.0], [x1, 0.5, 1.0], [x0, 0.5, 1.0]], rgb);
        let mut faces = vec![quad(0.0, 1.0, [1, 1, 1]), quad(1.0, 2.0, [2, 2, 2]), quad(3.0, 4.0, [3, 3, 3])];
        slice(&mut faces, &[Slice { lo: [0.0, 0.0, -2.0], hi: [1.0, 1.0, 2.0], axis: 2, at: 0.25 }]);
        assert_eq!(faces.len(), 4);
        assert_eq!(faces.iter().filter(|f| f.rgb == [1, 1, 1]).count(), 2);
        // the neighbour got the cut point (1, 0.5, 0.25) on its edge
        let nb = faces.iter().find(|f| f.rgb == [2, 2, 2]).unwrap();
        assert_eq!(nb.pts.len(), 5);
        assert!(nb.pts.iter().any(|p| (p[0] - 1.0).abs() < 1e-6 && (p[2] - 0.25).abs() < 1e-6));
        assert_eq!(faces.iter().find(|f| f.rgb == [3, 3, 3]).unwrap().pts.len(), 4);
        // both pieces of the cut quad end at the cut point too
        assert!(faces.iter().filter(|f| f.rgb == [1, 1, 1]).all(|f| f.pts.iter().any(|p| (p[0] - 1.0).abs() < 1e-6 && (p[2] - 0.25).abs() < 1e-6)));
    }

    /// a tapering shell (sloped top, a tall back wall, a short front wall) becomes a slab of
    /// even thickness under the top: closed, its bottom facing down, `thickness` under the top
    #[test]
    fn thicken_makes_an_even_plate() {
        let p = |x: f32, y: f32, z: f32| [x, y, z];
        let mut faces = vec![
            face(&[p(0.0, 1.0, 0.0), p(0.0, 1.2, 1.0), p(1.0, 1.2, 1.0), p(1.0, 1.0, 0.0)], [7; 3]),
            face(&[p(0.0, 0.5, 0.0), p(0.0, 1.0, 0.0), p(1.0, 1.0, 0.0), p(1.0, 0.5, 0.0)], [7; 3]),
            face(&[p(1.0, 1.2, 1.0), p(0.0, 1.2, 1.0), p(0.0, 1.1, 1.0), p(1.0, 1.1, 1.0)], [7; 3]),
        ];
        faces[0].normal = [0.0, 1.0, 0.0];
        thicken(&mut faces, &[Thicken { lo: [-0.1, 0.4, -0.1], hi: [1.1, 1.3, 1.1], thickness: 0.05, splits: Vec::new(), legs: false }]);
        assert_eq!(faces.len(), 6);
        assert!(closed(&faces));
        let (top, bottom) = (&faces[0], &faces[1]);
        assert!(bottom.normal[1] < -0.9, "{:?}", bottom.normal);
        let n = newell(&top.pts, Vec3::Y);
        for q in &bottom.pts {
            let d = n.dot(Vec3::from(top.pts[0]) - Vec3::from(*q));
            assert!((d - 0.05).abs() < 1e-4, "{d}");
        }
        // the walls face out of the plate
        for f in &faces[2..] {
            let c = f.pts.iter().map(|&p| Vec3::from(p)).sum::<Vec3>() / 4.0;
            assert!(Vec3::from(f.normal).dot(c - Vec3::new(0.5, 1.1, 0.5)) > 0.0, "{:?}", f.normal);
        }
    }

    /// a flat plate (x 0.2..0.6) over a floor at y 0.5, its front edge split: the edge at x 0.2
    /// grows a leg down into the floor, every other edge a wall; the split corner is on the skin
    #[test]
    fn thicken_legs_reach_the_floor() {
        let p = |x: f32, y: f32, z: f32| [x, y, z];
        let mut faces = vec![
            face(&[p(0.2, 1.0, 0.0), p(0.2, 1.0, 1.0), p(0.6, 1.0, 1.0), p(0.6, 1.0, 0.0)], [7; 3]),
            face(&[p(-1.0, 0.5, -1.0), p(-1.0, 0.5, 2.0), p(1.0, 0.5, 2.0), p(1.0, 0.5, -1.0)], [3; 3]),
        ];
        faces[0].normal = [0.0, 1.0, 0.0];
        static SPLITS: [([f32; 3], [f32; 3], [f32; 3]); 1] = [([0.6, 1.0, 1.0], [0.2, 1.0, 1.0], [0.4, 1.0, 1.0])];
        thicken(&mut faces, &[Thicken { lo: [0.1, 0.9, -0.1], hi: [0.7, 1.1, 1.1], thickness: 0.04, splits: SPLITS.to_vec(), legs: true }]);
        let skin = faces.iter().find(|f| f.rgb == [7; 3] && f.normal[1] > 0.9).expect("skin");
        assert_eq!(skin.pts.len(), 5);
        // floor, skin, bottom, 4 walls (the split edge in two), 4 leg faces
        assert_eq!(faces.len(), 1 + 1 + 1 + 4 + 4);
        let low: Vec<[f32; 3]> = faces.iter().filter(|f| f.rgb == [7; 3]).flat_map(|f| f.pts.iter().copied()).filter(|p| p[1] < 0.6).collect();
        assert_eq!(low.len(), 4 * 2);
        assert!(low.iter().all(|p| (p[1] - 0.49).abs() < 1e-5 && p[0] < 0.21), "{low:?}");
    }

    /// a table's curve passes its points, leaves its ends level and holds their weights beyond
    #[test]
    fn warp_table_is_smooth() {
        let t = [[0.0, 0.0], [1.0, 1.0], [2.0, 1.0], [3.0, 0.0]];
        for p in t {
            assert!((table_weight(&t, p[0]) - p[1]).abs() < 1e-6);
        }
        assert_eq!((table_weight(&t, -1.0), table_weight(&t, 9.0)), (0.0, 0.0));
        assert!(table_weight(&t, 0.01) < 0.01, "level start");
        assert!((table_weight(&t, 1.5) - 1.0).abs() < 1e-6, "flat between equal weights");
        assert_eq!(table_weight(&[], 3.0), 1.0);
    }

    /// two faces sharing an edge: the shared points move alike (still welded), a point beyond
    /// the tables' weights stays, the other side of the car moves mirrored
    #[test]
    fn warp_moves_shared_points_alike() {
        let p = |x: f32, y: f32, z: f32| [x, y, z];
        let mut faces = vec![face(&[p(0.4, 0.0, 1.0), p(0.9, 0.0, 1.0), p(0.9, 1.0, 1.0), p(0.4, 1.0, 1.0)], [1; 3]), face(&[p(0.9, 0.0, 1.0), p(0.9, 0.0, 2.0), p(0.9, 1.0, 2.0), p(0.9, 1.0, 1.0)], [2; 3]), face(&[p(-0.9, 0.0, 1.0), p(-0.9, 1.0, 1.0), p(-0.4, 1.0, 1.0)], [3; 3])];
        let w = Warp { by: [0.1, 0.0, -0.2], x: vec![[0.4, 0.0], [0.8, 1.0], [1.0, 1.0]], y: Vec::new(), z: vec![[-0.5, 0.0], [0.5, 1.0]] };
        warp(&mut faces, &[w]);
        // the shared point (0.9, 0, 1) of both faces: weight 1 on every axis
        assert_eq!(faces[0].pts[1], faces[1].pts[0]);
        assert!((faces[0].pts[1][0] - 1.0).abs() < 1e-5 && (faces[0].pts[1][2] - 0.8).abs() < 1e-5, "{:?}", faces[0].pts[1]);
        // x 0.4 weighs 0: stays
        assert_eq!(faces[0].pts[0], [0.4, 0.0, 1.0]);
        // the left side: the same bulge, out to the left
        assert!((faces[2].pts[0][0] + 1.0).abs() < 1e-5 && (faces[2].pts[0][2] - 0.8).abs() < 1e-5, "{:?}", faces[2].pts[0]);
    }
}
