//! REMASTER rear diffusers carved into the cage (#388, the Serpent's after the reference model
//! `reference_materials/models/veyron.blend`). The cage faces round the lower rear are sliced
//! along the channel's lines (its side walls, the keel's sides, a row every `step` along the
//! ramp), then every point inside the channel is lifted onto the roof: a ramp rising from the
//! floor to the tail between two side walls, with a keel down its middle whose bottom follows
//! a profile of its own. The cage stays one closed shell (the slices' points go into the
//! neighbours' edges too), so the bevel, the curved zones and the refine treat the diffuser
//! like any other panel. Each piece keeps the look of the face it was cut from: the ramp and
//! the walls the underside's, the keel's back face the rear bumper's. A design choice; the
//! original has none.
//!
//! Game space, metres (x right, y up, z forward; the tail faces -z).

use super::body::{Face, weld_key};
use glam::{Vec2, Vec3};
use std::collections::{HashMap, HashSet};

/// One diffuser, mirrored in x = 0.
#[derive(Clone, Debug, PartialEq)]
pub struct Diffuser {
    /// the box the work stays in (x as |x|: both sides): faces reaching into it are sliced and
    /// only points inside it move; the ramp starts at its `hi` z
    pub lo: [f32; 3],
    pub hi: [f32; 3],
    /// half the channel's width at the roof (m): where the side walls start
    pub width: f32,
    /// how far each side wall leans out from the roof down to the floor (m)
    pub wall: f32,
    /// the roof's height along the car, (z, y) from the front to the tail, straight between the
    /// points and held past the ends; points it would put lower stay where they are
    pub roof: Vec<[f32; 2]>,
    /// the keel's bottom, (z, y) likewise (never over the roof); a profile under the cage
    /// leaves the keel's bottom where the cage has it
    pub keel: Vec<[f32; 2]>,
    /// the keel's sides run straight from (`keel_bottom`, its bottom) to (`keel_top`, the roof),
    /// half widths (m)
    pub keel_bottom: f32,
    pub keel_top: f32,
    /// the corner where each side meets the keel's bottom rounded over this much (m) along both
    /// (a quadratic curve, the legs shortened to fit; 0: a sharp corner), #388 after the user's
    /// sketch: a U round the exhaust tip
    pub keel_round: f32,
    /// the ramp is sliced across every this much along z (m), as far as the profiles' last
    /// points
    pub step: f32,
}

/// how close (m) a point must lie to a slicing plane or an edge to count as on it
const ON: f32 = 1e-5;
/// a face whose normal's y is under minus this faces down (the underside the ramp is made of)
const DOWN: f32 = 0.3;
/// the keel's sides are sliced along the car at least this often across (m), so the rounded
/// corner reads as a curve
const KEEL_STEP: f32 = 0.008;

/// `prof` at `z`: straight between its points (z falling), held past the ends.
fn at(prof: &[[f32; 2]], z: f32) -> f32 {
    let (first, last) = (prof[0], prof[prof.len() - 1]);
    if z >= first[0] {
        return first[1];
    }
    if z <= last[0] {
        return last[1];
    }
    for w in prof.windows(2) {
        let (a, b) = (w[0], w[1]);
        if z <= a[0] && z >= b[0] {
            let t = if (a[0] - b[0]).abs() < 1e-9 { 0.0 } else { (a[0] - z) / (a[0] - b[0]) };
            return a[1] + (b[1] - a[1]) * t;
        }
    }
    last[1]
}

impl Diffuser {
    /// whether `p` lies in the box (x as |x|)
    fn inside(&self, p: [f32; 3]) -> bool {
        let q = [p[0].abs(), p[1], p[2]];
        (0..3).all(|i| q[i] >= self.lo[i] - ON && q[i] <= self.hi[i] + ON)
    }

    /// whether a face reaches into the box, `xmax` standing in for its `hi` x
    fn reaches(&self, f: &Face, xmax: f32) -> bool {
        let lo = |i: usize| f.pts.iter().map(|p| p[i]).fold(f32::MAX, f32::min);
        let hi = |i: usize| f.pts.iter().map(|p| p[i]).fold(f32::MIN, f32::max);
        let (x0, x1) = (lo(0), hi(0));
        // the face's |x| range
        let (ax0, ax1) = if x0 < 0.0 && x1 > 0.0 { (0.0, x1.max(-x0)) } else { (x0.abs().min(x1.abs()), x0.abs().max(x1.abs())) };
        ax1 > self.lo[0] + ON && ax0 < xmax - ON && (1..3).all(|i| hi(i) > self.lo[i] + ON && lo(i) < self.hi[i] - ON)
    }

    /// The height `p` is lifted to (its own when the diffuser leaves it).
    fn lift(&self, p: [f32; 3]) -> f32 {
        if !self.inside(p) || self.roof.is_empty() {
            return p[1];
        }
        let (ax, y, z) = (p[0].abs(), p[1], p[2]);
        let r = at(&self.roof, z);
        let k = if self.keel.is_empty() { r } else { at(&self.keel, z).min(r) };
        let target = self.keel_y(ax, k, r);
        let wf = if self.wall > 0.0 {
            ((self.width + self.wall - ax) / self.wall).clamp(0.0, 1.0)
        } else if ax <= self.width {
            1.0
        } else {
            0.0
        };
        y + (target - y).max(0.0) * wf
    }

    /// The keel's cross-section: the height at |x| = `ax` of a keel with its bottom at `k` under
    /// the roof `r`: flat to the rounded corner, then straight up its side to the roof.
    fn keel_y(&self, ax: f32, k: f32, r: f32) -> f32 {
        let (q, t) = (self.keel_bottom, self.keel_top);
        if t <= q || ax >= t {
            return r;
        }
        let side = Vec2::new(t - q, r - k);
        let len = side.length();
        if len < 1e-6 {
            return r;
        }
        let corner = Vec2::new(q, k);
        let a = Vec2::new(q - self.keel_round.min(q), k);
        let b = corner + side / len * self.keel_round.min(0.5 * len);
        if ax <= a.x {
            return k;
        }
        if ax >= b.x {
            return k + side.y * (ax - q) / side.x;
        }
        // the quadratic curve a -> corner -> b: x(s) = a + 2 s (c - a) + s^2 (a - 2 c + b), rising
        let (al, be, ga) = (a.x - 2.0 * corner.x + b.x, 2.0 * (corner.x - a.x), a.x - ax);
        let s = if al.abs() < 1e-9 { -ga / be } else { (-be + (be * be - 4.0 * al * ga).max(0.0).sqrt()) / (2.0 * al) };
        let s = s.clamp(0.0, 1.0);
        k + s * s * (b.y - k)
    }
}

/// `a` + (`b` - `a`) `t` with `t` taken from the end with the smaller weld key, so that the two
/// faces on an edge get the very same point whichever way round they run it.
fn edge_lerp<T: Copy>(a: [f32; 3], b: [f32; 3], axis: usize, c: f32, va: T, vb: T, mix: impl Fn(T, T, f32) -> T) -> ([f32; 3], T) {
    let swap = weld_key(b) < weld_key(a);
    let (p, q, vp, vq) = if swap { (b, a, vb, va) } else { (a, b, va, vb) };
    let t = (c - p[axis]) / (q[axis] - p[axis]);
    let mut r = [0.0; 3];
    for i in 0..3 {
        r[i] = p[i] + (q[i] - p[i]) * t;
    }
    r[axis] = c;
    (r, mix(vp, vq, t))
}

fn mix4(a: [f32; 4], b: [f32; 4], t: f32) -> [f32; 4] {
    std::array::from_fn(|i| a[i] + (b[i] - a[i]) * t)
}
fn mix2(a: [f32; 2], b: [f32; 2], t: f32) -> [f32; 2] {
    std::array::from_fn(|i| a[i] + (b[i] - a[i]) * t)
}

/// `f` cut by the plane `p[axis]` = `c` into the pieces under and over it, or None when the
/// plane misses it (a convex face: the cage's quads and triangles). The cut points go into
/// `made`.
fn split(f: &Face, axis: usize, c: f32, made: &mut Vec<[f32; 3]>) -> Option<(Face, Face)> {
    let d: Vec<f32> = f.pts.iter().map(|p| p[axis] - c).collect();
    if d.iter().all(|&x| x >= -ON) || d.iter().all(|&x| x <= ON) {
        return None;
    }
    let empty = || Face { pts: Vec::new(), colour: Vec::new(), uv: Vec::new(), ..f.clone() };
    let (mut under, mut over) = (empty(), empty());
    let n = f.pts.len();
    for i in 0..n {
        let j = (i + 1) % n;
        if d[i] <= ON {
            under.pts.push(f.pts[i]);
            under.colour.push(f.colour[i]);
            under.uv.push(f.uv[i]);
        }
        if d[i] >= -ON {
            over.pts.push(f.pts[i]);
            over.colour.push(f.colour[i]);
            over.uv.push(f.uv[i]);
        }
        if (d[i] < -ON && d[j] > ON) || (d[i] > ON && d[j] < -ON) {
            let (p, col) = edge_lerp(f.pts[i], f.pts[j], axis, c, f.colour[i], f.colour[j], mix4);
            let (_, uv) = edge_lerp(f.pts[i], f.pts[j], axis, c, f.uv[i], f.uv[j], mix2);
            for g in [&mut under, &mut over] {
                g.pts.push(p);
                g.colour.push(col);
                g.uv.push(uv);
            }
            made.push(p);
        }
    }
    (under.pts.len() >= 3 && over.pts.len() >= 3).then_some((under, over))
}

/// Whether the polygon is convex (seen along its normal).
fn convex(f: &Face) -> bool {
    let n = Vec3::from(f.normal);
    let p: Vec<Vec3> = f.pts.iter().map(|&p| Vec3::from(p)).collect();
    let k = p.len();
    let turns: Vec<f32> = (0..k).map(|i| (p[(i + 1) % k] - p[i]).cross(p[(i + 2) % k] - p[(i + 1) % k]).dot(n)).collect();
    turns.iter().all(|&t| t >= -1e-9) || turns.iter().all(|&t| t <= 1e-9)
}

/// `f` as triangles: ear clipping (`body::triangulate`, which skips the straight corners of
/// cut points) round its own winding's normal.
fn triangles(f: &Face) -> Vec<Face> {
    let raw: Vec3 = (0..f.pts.len()).map(|i| Vec3::from(f.pts[i]).cross(Vec3::from(f.pts[(i + 1) % f.pts.len()]))).sum();
    let n = raw.normalize_or(Vec3::from(f.normal));
    let t = n.any_orthonormal_vector();
    let b = n.cross(t);
    let flat: Vec<Vec2> = f.pts.iter().map(|&p| Vec2::new(Vec3::from(p).dot(t), Vec3::from(p).dot(b))).collect();
    super::body::triangulate(&flat)
        .into_iter()
        .map(|tri| {
            let pts: Vec<[f32; 3]> = tri.iter().map(|&k| f.pts[k]).collect();
            let normal = newell(&pts, Vec3::from(f.normal)).to_array();
            Face { pts, normal, colour: tri.iter().map(|&k| f.colour[k]).collect(), uv: tri.iter().map(|&k| f.uv[k]).collect(), ..f.clone() }
        })
        .collect()
}

/// `f` as triangles when it is not convex (the slicing cuts convex faces), else as it is.
fn convex_pieces(f: Face) -> Vec<Face> {
    if f.pts.len() <= 3 || convex(&f) { vec![f] } else { triangles(&f) }
}

/// Every face reaching into the box (`xmax` for its x) cut by the plane `p[axis]` = `c`; with
/// `down` only the faces facing down.
fn slice(faces: Vec<Face>, d: &Diffuser, xmax: f32, axis: usize, c: f32, down: bool, made: &mut Vec<[f32; 3]>) -> Vec<Face> {
    let mut out = Vec::with_capacity(faces.len());
    for f in faces {
        if !d.reaches(&f, xmax) || (down && newell(&f.pts, Vec3::from(f.normal)).y > -DOWN) {
            out.push(f);
            continue;
        }
        for g in convex_pieces(f) {
            match split(&g, axis, c, made) {
                Some((a, b)) => out.extend([a, b]),
                None => out.push(g),
            }
        }
    }
    out
}

/// `f` with the points of `made` lying on its edges put in (T-junctions closed).
fn close_t(f: &Face, made: &[[f32; 3]]) -> Face {
    let n = f.pts.len();
    let mut g = Face { pts: Vec::new(), colour: Vec::new(), uv: Vec::new(), ..f.clone() };
    for k in 0..n {
        let (a, b) = (Vec3::from(f.pts[k]), Vec3::from(f.pts[(k + 1) % n]));
        g.pts.push(f.pts[k]);
        g.colour.push(f.colour[k]);
        g.uv.push(f.uv[k]);
        let len = a.distance(b);
        if len < 1e-6 {
            continue;
        }
        let dir = (b - a) / len;
        let mut on: Vec<(f32, [f32; 3])> = made
            .iter()
            .map(|&p| ((Vec3::from(p) - a).dot(dir), p))
            .filter(|&(t, p)| t > 1e-4 && t < len - 1e-4 && (a + dir * t).distance(Vec3::from(p)) < 2e-4)
            .collect();
        on.sort_by(|x, y| x.0.total_cmp(&y.0));
        on.dedup_by(|x, y| weld_key(x.1) == weld_key(y.1));
        for (t, p) in on {
            if weld_key(p) == weld_key(f.pts[k]) || weld_key(p) == weld_key(f.pts[(k + 1) % n]) {
                continue;
            }
            g.pts.push(p);
            g.colour.push(mix4(f.colour[k], f.colour[(k + 1) % n], t / len));
            g.uv.push(mix2(f.uv[k], f.uv[(k + 1) % n], t / len));
        }
    }
    g
}

/// the polygon's plane normal (Newell), turned to the side of `like`
fn newell(pts: &[[f32; 3]], like: Vec3) -> Vec3 {
    let n: Vec3 = (0..pts.len()).map(|i| Vec3::from(pts[i]).cross(Vec3::from(pts[(i + 1) % pts.len()]))).sum();
    let n = n.normalize_or(like);
    if n.dot(like) < 0.0 { -n } else { n }
}

/// the longest hole `seal` closes, in edges
const SEAL_EDGES: usize = 8;

/// How far (m) a hole's corners may lie off its plane for `seal` to close it
const SEAL_FLAT: f32 = 2e-3;

/// Close the holes the cage has inside the box with a face each, before anything is cut. [O] The
/// Serpent's underside ends in a straight edge across the tail (z -2.174 from x -0.389 to 0.389)
/// but the rear bumper 138/193 standing on it ends in a shallow V (z -2.183 at x 0, -2.174 at its
/// ends): a wedge-shaped slit 9 mm wide at the middle, black and out of sight under the car until
/// the ramp lifted it to the roof, where it showed the inside of the body from behind. A hole is a
/// ring of open edges (no face runs them the other way) inside the box; the face closing it runs
/// the ring the other way round, so it joins the faces it closes and takes the look of the one
/// it faces most like (the underside's here), and the slicing then cuts it like the rest.
fn seal(faces: &mut Vec<Face>, d: &Diffuser) {
    type Key = [i32; 3];
    let mut runs: HashMap<(Key, Key), usize> = HashMap::new();
    let mut at_key: HashMap<Key, [f32; 3]> = HashMap::new();
    for (i, f) in faces.iter().enumerate() {
        for k in 0..f.pts.len() {
            let (a, b) = (f.pts[k], f.pts[(k + 1) % f.pts.len()]);
            if weld_key(a) != weld_key(b) {
                runs.insert((weld_key(a), weld_key(b)), i);
                at_key.entry(weld_key(a)).or_insert(a);
                at_key.entry(weld_key(b)).or_insert(b);
            }
        }
    }
    // each open edge turned round (b -> a), by its start: where a hole's ring goes on from there
    let mut onward: HashMap<Key, Vec<(Key, usize)>> = HashMap::new();
    let mut open: Vec<(Key, Key, usize)> = runs.iter().filter(|&(&(a, b), _)| !runs.contains_key(&(b, a)) && d.inside(at_key[&a]) && d.inside(at_key[&b])).map(|(&(a, b), &i)| (a, b, i)).collect();
    open.sort();
    for (a, b, i) in open {
        onward.entry(b).or_default().push((a, i));
    }
    let mut starts: Vec<Key> = onward.keys().copied().collect();
    starts.sort();
    let mut done: HashSet<Key> = HashSet::new();
    let mut closing: Vec<Face> = Vec::new();
    for s in starts {
        if done.contains(&s) {
            continue;
        }
        // follow the ring; a corner with two ways on, a dead end or a long ring is no simple hole
        let (mut ring, mut cur) = (Vec::<(Key, usize)>::new(), s);
        let shut = loop {
            let Some(&[(end, owner)]) = onward.get(&cur).map(Vec::as_slice) else { break false };
            ring.push((cur, owner));
            cur = end;
            if cur == s {
                break true;
            }
            if ring.len() >= SEAL_EDGES || ring.iter().any(|r| r.0 == cur) {
                break false;
            }
        };
        done.extend(ring.iter().map(|r| r.0));
        if !shut || ring.len() < 3 {
            continue;
        }
        let pts: Vec<[f32; 3]> = ring.iter().map(|r| at_key[&r.0]).collect();
        let raw: Vec3 = (0..pts.len()).map(|i| Vec3::from(pts[i]).cross(Vec3::from(pts[(i + 1) % pts.len()]))).sum();
        let n = raw.normalize_or_zero();
        let mid = pts.iter().map(|&p| Vec3::from(p)).sum::<Vec3>() / pts.len() as f32;
        if n == Vec3::ZERO || pts.iter().any(|&p| (Vec3::from(p) - mid).dot(n).abs() > SEAL_FLAT) {
            continue;
        }
        // the look of the face round the hole that this one faces most like
        let look = ring.iter().map(|r| &faces[r.1]).max_by(|a, b| n.dot(Vec3::from(a.normal)).total_cmp(&n.dot(Vec3::from(b.normal)))).expect("a ring has edges");
        if n.dot(Vec3::from(look.normal)) <= 0.0 {
            continue;
        }
        let near = |p: [f32; 3]| (0..look.pts.len()).min_by(|&a, &b| Vec3::from(look.pts[a]).distance(Vec3::from(p)).total_cmp(&Vec3::from(look.pts[b]).distance(Vec3::from(p)))).expect("a face has corners");
        let from: Vec<usize> = pts.iter().map(|&p| near(p)).collect();
        closing.push(Face { colour: from.iter().map(|&k| look.colour[k]).collect(), uv: from.iter().map(|&k| look.uv[k]).collect(), pts, normal: n.to_array(), ..look.clone() });
    }
    faces.extend(closing);
}

/// Carve the diffusers into the cage (after the arches, before the curved zones).
pub fn diffusers(faces: &mut Vec<Face>, ds: &[Diffuser]) {
    for d in ds {
        if d.roof.is_empty() {
            continue;
        }
        seal(faces, d);
        let mut made: Vec<[f32; 3]> = Vec::new();
        let mut cut = std::mem::take(faces);
        // the ramp's start, then the channel's lines along the car, then the rows across it
        cut = slice(cut, d, d.hi[0], 2, d.hi[2], false, &mut made);
        let mut xs: Vec<f32> = vec![d.keel_top, d.width, d.width + d.wall];
        // a rounded keel's cross-section bends all across its corner (whose end moves with the
        // keel's depth): evenly from the corner's start; the sharp corner's own line would cut a
        // sliver beside one of those
        let x0 = d.keel_bottom - d.keel_round.min(d.keel_bottom);
        if d.keel_round > 0.0 && d.keel_top > x0 {
            let n = ((d.keel_top - x0) / KEEL_STEP).ceil() as usize;
            xs.extend((0..n).map(|i| x0 + (d.keel_top - x0) * i as f32 / n as f32));
        } else {
            xs.push(d.keel_bottom);
        }
        xs.retain(|&x| x > 1e-4 && x < d.hi[0]);
        xs.sort_by(f32::total_cmp);
        xs.dedup_by(|a, b| (*a - *b).abs() < 1e-4);
        for &x in &xs {
            for c in [x, -x] {
                cut = slice(cut, d, d.hi[0], 0, c, false, &mut made);
            }
        }
        let channel = (d.width + d.wall).min(d.hi[0]);
        // the rows stop at the profiles' last points, past which the heights hold: the tail's
        // upright faces lean a centimetre or two along z, and a row there cut them aslant
        let end = d.roof.iter().chain(&d.keel).map(|p| p[0]).fold(f32::MAX, f32::min).max(d.lo[2]);
        let mut zs: Vec<f32> = Vec::new();
        let mut z = d.hi[2] - d.step;
        while d.step > 0.0 && z > end + 1e-3 {
            zs.push(z);
            z -= d.step;
        }
        zs.extend(d.roof.iter().chain(&d.keel).map(|p| p[0]).filter(|&z| z < d.hi[2] - 1e-3 && z >= end));
        zs.sort_by(|a, b| b.total_cmp(a));
        zs.dedup_by(|a, b| (*a - *b).abs() < 1e-3);
        // the rows cut the underside only: [O] the Serpent's rear bumper leans back 2 deg (139, n
        // 0.02 0.02 -1.00) and 140 runs aslant (n 0.37 0.04 -0.93), so a row cut them over the
        // tail edge, and their points under the roof, lifted straight up, landed on the ramp's
        // own (faces on faces: 56 edges on three or four). Uncut, an upright face's only points
        // under the roof are on its foot, which the ramp shares.
        for &z in &zs {
            cut = slice(cut, d, channel, 2, z, true, &mut made);
        }
        // the neighbours take the cut points on their edges
        made.sort_by(|a, b| a[0].total_cmp(&b[0]).then(a[1].total_cmp(&b[1])).then(a[2].total_cmp(&b[2])));
        made.dedup_by(|a, b| weld_key(*a) == weld_key(*b));
        let closed: Vec<Face> = cut.iter().map(|f| close_t(f, &made)).collect();
        // lift; a face the lift warps stays one (`body` and the refine mesh it from its corners):
        // cut into triangles, a chord passing a millimetre or two from a corner was
        // cut again at that corner by the curved zones' T-junction pass (`smooth::t_junctions`,
        // 2 mm), and the two triangles on it made the rear bumper's zone non-manifold
        for mut f in closed {
            let before = f.pts.clone();
            for p in f.pts.iter_mut() {
                p[1] = d.lift(*p);
            }
            if f.pts != before {
                f.normal = newell(&f.pts, Vec3::from(f.normal)).to_array();
            }
            faces.push(f);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn quad(pts: [[f32; 3]; 4], normal: [f32; 3], rgb: [u8; 3]) -> Face {
        Face { pts: pts.to_vec(), normal, colour: vec![[1.0; 4]; 4], uv: vec![[0.0; 2]; 4], layer: -1.0, env: 0.0, rgb, smooth_deg: 0.0, remesh: 0.0 }
    }

    /// a closed box: floor y 0.2, top 0.8, x +-0.8, z -2..0; its tail's top `lean` further
    /// forward than its foot
    fn block(lean: f32) -> Vec<Face> {
        let (x0, x1, y0, y1, z0, z1) = (-0.8, 0.8, 0.2, 0.8, -2.0, 0.0);
        let zt = z0 + lean;
        vec![
            quad([[x0, y0, z0], [x1, y0, z0], [x1, y0, z1], [x0, y0, z1]], [0.0, -1.0, 0.0], [0, 0, 0]),
            quad([[x0, y1, zt], [x0, y1, z1], [x1, y1, z1], [x1, y1, zt]], [0.0, 1.0, 0.0], [9, 9, 9]),
            quad([[x0, y0, z0], [x0, y1, zt], [x1, y1, zt], [x1, y0, z0]], [0.0, 0.0, -1.0], [9, 9, 9]),
            quad([[x0, y0, z1], [x1, y0, z1], [x1, y1, z1], [x0, y1, z1]], [0.0, 0.0, 1.0], [9, 9, 9]),
            quad([[x0, y0, z0], [x0, y0, z1], [x0, y1, z1], [x0, y1, zt]], [-1.0, 0.0, 0.0], [9, 9, 9]),
            quad([[x1, y0, z0], [x1, y1, zt], [x1, y1, z1], [x1, y0, z1]], [1.0, 0.0, 0.0], [9, 9, 9]),
        ]
    }

    fn diffuser() -> Diffuser {
        Diffuser {
            lo: [0.0, 0.1, -2.1],
            hi: [0.6, 0.7, -1.4],
            width: 0.45,
            wall: 0.02,
            roof: vec![[-1.4, 0.2], [-1.7, 0.3], [-1.98, 0.48], [-2.0, 0.5]],
            keel: vec![[-1.4, 0.0], [-2.0, 0.0]],
            keel_bottom: 0.06,
            keel_top: 0.12,
            keel_round: 0.0,
            step: 0.05,
        }
    }

    /// every edge of the carved block on exactly two faces, each way round once; also with the
    /// tail leaning back like the Serpent's (its points under the roof once landed on the ramp)
    #[test]
    fn stays_closed() {
        for lean in [0.0, 0.06] {
            let mut faces = block(lean);
            diffusers(&mut faces, &[diffuser()]);
            assert_eq!(open_edges(&faces), 0, "open or doubled edges, lean {lean}");
        }
    }

    /// `block` with the tail's foot a shallow V (the Serpent's: its middle 9 mm further back than
    /// the floor's straight rear edge), the floor and the tail meeting in a wedge-shaped hole
    fn wedge_block() -> Vec<Face> {
        let (x0, x1, y0, y1, z0, z1, v) = (-0.55, 0.55, 0.2, 0.8, -2.0, 0.0, 0.009);
        vec![
            quad([[x0, y0, z0], [x1, y0, z0], [x1, y0, z1], [x0, y0, z1]], [0.0, -1.0, 0.0], [0, 0, 0]),
            Face { pts: vec![[x0, y1, z0], [x0, y1, z1], [x1, y1, z1], [x1, y1, z0], [0.0, y1, z0 - v]], colour: vec![[1.0; 4]; 5], uv: vec![[0.0; 2]; 5], ..quad([[0.0; 3]; 4], [0.0, 1.0, 0.0], [9, 9, 9]) },
            quad([[x0, y0, z0], [x0, y1, z0], [0.0, y1, z0 - v], [0.0, y0, z0 - v]], [0.0, 0.0, -1.0], [9, 9, 9]),
            quad([[0.0, y0, z0 - v], [0.0, y1, z0 - v], [x1, y1, z0], [x1, y0, z0]], [0.0, 0.0, -1.0], [9, 9, 9]),
            quad([[x0, y0, z1], [x1, y0, z1], [x1, y1, z1], [x0, y1, z1]], [0.0, 0.0, 1.0], [9, 9, 9]),
            quad([[x0, y0, z0], [x0, y0, z1], [x0, y1, z1], [x0, y1, z0]], [-1.0, 0.0, 0.0], [9, 9, 9]),
            quad([[x1, y0, z0], [x1, y1, z0], [x1, y1, z1], [x1, y0, z1]], [1.0, 0.0, 0.0], [9, 9, 9]),
        ]
    }

    /// #388: a hole in the cage inside the box (the wedge between the floor and a V-footed tail)
    /// is closed with a face of the floor's look, before and after the carving
    #[test]
    fn closes_the_wedge() {
        let mut faces = wedge_block();
        assert!(open_edges(&faces) > 0, "the block has the wedge");
        seal(&mut faces, &diffuser());
        assert_eq!(faces.len(), 8, "one face closing it");
        let hole = &faces[7];
        assert_eq!(hole.rgb, [0, 0, 0], "the floor's look");
        assert!(hole.normal[1] < -0.99, "facing down: {:?}", hole.normal);
        assert_eq!(open_edges(&faces), 0, "closed");
        let mut faces = wedge_block();
        diffusers(&mut faces, &[diffuser()]);
        assert_eq!(open_edges(&faces), 0, "closed after the carving");
    }

    /// a hole outside the box stays as the cage has it
    #[test]
    fn leaves_holes_outside_the_box_alone() {
        let mut faces = wedge_block();
        let d = Diffuser { lo: [0.0, 0.1, -1.9], ..diffuser() };
        seal(&mut faces, &d);
        assert_eq!(faces.len(), 7);
    }

    fn open_edges(faces: &[Face]) -> usize {
        let mut uses: HashMap<([i32; 3], [i32; 3]), i32> = HashMap::new();
        for f in faces {
            let n = f.pts.len();
            for k in 0..n {
                *uses.entry((weld_key(f.pts[k]), weld_key(f.pts[(k + 1) % n]))).or_default() += 1;
            }
        }
        uses.iter().filter(|&(&(a, b), &c)| c != 1 || uses.get(&(b, a)) != Some(&1)).count()
    }

    /// at the tail the roof is up at 0.5 inside the walls, the keel's bottom and the floor
    /// outside the walls stay at 0.2, and nothing in front of the ramp moved
    #[test]
    fn carves_the_channel() {
        let mut faces = block(0.0);
        diffusers(&mut faces, &[diffuser()]);
        let tail: Vec<[f32; 3]> = faces.iter().flat_map(|f| f.pts.clone()).filter(|p| (p[2] + 2.0).abs() < 1e-4).collect();
        let lowest = |x: f32| tail.iter().filter(|p| (p[0] - x).abs() < 1e-4).map(|p| p[1]).fold(f32::MAX, f32::min);
        for x in [0.12, 0.45, -0.12, -0.45] {
            assert!((lowest(x) - 0.5).abs() < 1e-4, "roof at x {x}: {}", lowest(x));
        }
        for x in [0.06, -0.06, 0.47, -0.47, 0.8] {
            assert!((lowest(x) - 0.2).abs() < 1e-4, "keel or outside the walls at x {x}: {}", lowest(x));
        }
        assert!(faces.iter().flat_map(|f| f.pts.iter()).filter(|p| p[2] > -1.4 + 1e-4).all(|p| p[1] == 0.2 || p[1] == 0.8));
    }

    /// #388: the rounded keel is flat to the corner's start, then never under the sharp corner
    /// it rounds (it carves more), rising, and on the straight side past the corner
    #[test]
    fn keel_corner_rounds() {
        let d = Diffuser { keel_bottom: 0.06, keel_top: 0.09, keel_round: 0.03, ..diffuser() };
        let sharp = Diffuser { keel_round: 0.0, ..d.clone() };
        let (k, r) = (0.4, 0.52);
        let ys: Vec<f32> = (0..=100).map(|i| d.keel_y(0.1 * i as f32 / 100.0, k, r)).collect();
        assert!(ys.windows(2).all(|w| w[1] >= w[0] - 1e-6), "rising");
        for i in 0..=100 {
            let x = 0.1 * i as f32 / 100.0;
            let (y, ys) = (d.keel_y(x, k, r), sharp.keel_y(x, k, r));
            assert!(y >= ys - 1e-6, "under the sharp corner at {x}: {y} < {ys}");
            if x <= 0.03 || x >= 0.075 {
                assert!((y - ys).abs() < 1e-5, "off the outline at {x}: {y} vs {ys}");
            }
        }
        assert!(d.keel_y(0.06, k, r) > k + 0.005, "the corner itself is cut away");
    }

    /// the carved block with a rounded keel stays closed too
    #[test]
    fn rounded_keel_stays_closed() {
        let mut faces = block(0.06);
        let d = Diffuser { keel: vec![[-1.6, 0.25], [-2.0, 0.35]], keel_round: 0.03, ..diffuser() };
        diffusers(&mut faces, &[d]);
        assert_eq!(open_edges(&faces), 0);
    }

    #[test]
    fn profile_holds_past_its_ends() {
        let p = [[-1.0, 0.2], [-2.0, 0.6]];
        assert_eq!(at(&p, 0.0), 0.2);
        assert!((at(&p, -1.5) - 0.4).abs() < 1e-6);
        assert_eq!(at(&p, -3.0), 0.6);
    }
}
