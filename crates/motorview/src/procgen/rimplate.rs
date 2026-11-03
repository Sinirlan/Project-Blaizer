//! REMASTER window plates (#337 phase 2, 09-remaster 9.23): a disc on the rim face that follows
//! the dish, with `count` copies of one window cut out (a holed disc), none (a full cover) or
//! with floors (a grooved cover). The top is tessellated by lyon with the windows as holes and
//! seed points inside, flipped to Delaunay and refined so it stays on the dish; every outline
//! gets the star plate's chamfer and wall.

use super::{tri, SEGMENTS, V};
use crate::scene::Vertex;
use glam::{Vec2, Vec3};
use lyon_tessellation::math::point;
use lyon_tessellation::path::Path;
use lyon_tessellation::{BuffersBuilder, FillOptions, FillTessellator, FillVertex, FillVertexConstructor, VertexBuffers, VertexSource};
use std::collections::HashMap;
use std::f32::consts::{PI, TAU};

/// A plate from radius `r0` to `r1` (fractions of the rim radius; `r0` 0 = solid to the hub),
/// its top `lift` above the dish (× half-width) and 1 mm under the star, so it lies over the
/// spokes (2 mm under) [design choice], its walls `depth` deep (× half-width). `count` windows
/// of one `outline` ((radius, angle off the window's centre) points, closed, rim fractions and
/// radians; edges run straight in radius and angle, so an edge at one radius is an arc) sit at
/// the spoke angle plus `phase` window pitches (`TAU / count`) and more pitches; corners are
/// rounded by `corner` (rim fractions). `groove` > 0: the windows have a floor that deep
/// (× half-width) instead of opening.
#[derive(Clone, Debug, PartialEq)]
pub struct WindowPlate {
    pub r0: f32,
    pub r1: f32,
    pub depth: f32,
    pub lift: f32,
    pub count: usize,
    pub phase: f32,
    pub outline: Vec<[f32; 2]>,
    pub corner: f32,
    pub groove: f32,
}

impl Default for WindowPlate {
    /// five round holes between the spokes, solid to the hub
    fn default() -> Self {
        WindowPlate { r0: 0.0, r1: 0.84, depth: 0.15, lift: 0.0, count: 5, phase: 0.5, outline: circle(0.55, 0.17, 16), corner: 0.0, groove: 0.0 }
    }
}

/// a point of the window's own plane (x out along its centre line, y across) as (radius, angle)
fn polar(p: Vec2) -> [f32; 2] {
    [p.length(), p.y.atan2(p.x)]
}

/// A round window of radius `rad` centred `rc` out, `n` points.
pub fn circle(rc: f32, rad: f32, n: usize) -> Vec<[f32; 2]> {
    ngon(rc, rad, n, 0.0)
}

/// A regular polygon of `sides` corners on a circle of radius `rad` centred `rc` out, the first
/// corner `turn` radians round from the outward direction.
pub fn ngon(rc: f32, rad: f32, sides: usize, turn: f32) -> Vec<[f32; 2]> {
    let n = sides.max(3);
    (0..n).map(|i| { let a = turn + i as f32 * TAU / n as f32; polar(Vec2::new(rc + rad * a.cos(), rad * a.sin())) }).collect()
}

/// A sector from radius `ri` to `ro`, `wi` and `wo` radians wide there: the inner and outer
/// edges are arcs.
pub fn trapezoid(ri: f32, ro: f32, wi: f32, wo: f32) -> Vec<[f32; 2]> {
    vec![[ri, -wi * 0.5], [ro, -wo * 0.5], [ro, wo * 0.5], [ri, wi * 0.5]]
}

/// A teardrop from radius `ri` to `ro`: the hull of a circle `wi` across at the inner end and one
/// `wo` across at the outer end (a width of 0 is a point).
pub fn teardrop(ri: f32, ro: f32, wi: f32, wo: f32) -> Vec<[f32; 2]> {
    let (a, b) = (wi.max(0.0) * 0.5, wo.max(0.0) * 0.5);
    let (ca, cb) = (ri + a, ro - b);
    let d = cb - ca;
    if d <= (a - b).abs() + 1e-6 {
        // one end holds the other: a circle
        return circle((ri + ro) * 0.5, (ro - ri) * 0.5, 16);
    }
    // the external tangents touch both circles at ±beta from the centre line
    let beta = ((a - b) / d).clamp(-1.0, 1.0).acos();
    let mut p: Vec<Vec2> = (0..=12).map(|i| { let t = -beta + 2.0 * beta * i as f32 / 12.0; Vec2::new(cb + b * t.cos(), b * t.sin()) }).collect();
    if a > 1e-6 {
        p.extend((0..=8).map(|i| { let t = beta + (TAU - 2.0 * beta) * i as f32 / 8.0; Vec2::new(ca + a * t.cos(), a * t.sin()) }));
    } else {
        p.push(Vec2::new(ca, 0.0));
    }
    p.into_iter().map(polar).collect()
}

/// point at arc length `d` along a polyline (from its end when `back`)
fn along(s: &[Vec2], d: f32, back: bool) -> Vec2 {
    let at = |i: usize| if back { s[s.len() - 1 - i] } else { s[i] };
    let mut left = d;
    for i in 1..s.len() {
        let (p, q) = (at(i - 1), at(i));
        let l = p.distance(q);
        if left <= l && l > 0.0 {
            return p.lerp(q, left / l);
        }
        left -= l;
    }
    at(s.len() - 1)
}

/// One window's outline as a closed loop in (y, z) metres (y = r sin, z = r cos of the angle)
/// round the wheel angle `base`: edges interpolated in radius and angle as finely as the ring
/// (`SEGMENTS`), corners rounded by a quadratic Bezier whose tangent points sit where a circle
/// of radius `corner` would touch [APPROX: about that radius].
fn window_loop(outline: &[[f32; 2]], base: f32, rr: f32, corner: f32) -> Vec<Vec2> {
    let cart = |r: f32, a: f32| { let t = base + a; Vec2::new(t.sin(), t.cos()) * r * rr };
    let n = outline.len();
    let segs: Vec<Vec<Vec2>> = (0..n).map(|k| {
        let (p, q) = (outline[k], outline[(k + 1) % n]);
        let m = ((q[1] - p[1]).abs() / (TAU / SEGMENTS as f32)).ceil().max(1.0) as usize;
        (0..=m).map(|i| { let s = i as f32 / m as f32; cart(p[0] + (q[0] - p[0]) * s, p[1] + (q[1] - p[1]) * s) }).collect()
    }).collect();
    let len = |s: &[Vec2]| s.windows(2).map(|w| w[0].distance(w[1])).sum::<f32>();
    // each corner's tangent length
    let tl: Vec<f32> = (0..n).map(|k| {
        if corner <= 0.0 {
            return 0.0;
        }
        let (sin, sout) = (&segs[(k + n - 1) % n], &segs[k]);
        let pk = sout[0];
        let din = (pk - sin[sin.len() - 2]).normalize_or_zero();
        let dout = (sout[1] - pk).normalize_or_zero();
        // only real corners: a turn under the sharp-corner angle is already round
        if din.dot(dout) > SHARP_COS {
            return 0.0;
        }
        let interior = PI - din.dot(dout).clamp(-1.0, 1.0).acos();
        (corner * rr / (interior * 0.5).tan()).min(0.45 * len(sin).min(len(sout)))
    }).collect();
    let mut out: Vec<Vec2> = Vec::new();
    for k in 0..n {
        let (sin, sout) = (&segs[(k + n - 1) % n], &segs[k]);
        if tl[k] > 0.0 {
            let (a, p, b) = (along(sin, tl[k], true), sout[0], along(sout, tl[k], false));
            // steps of an arc of radius `corner` that sag at most 0.3 mm
            let turn = PI - (a - p).normalize_or_zero().dot((b - p).normalize_or_zero()).clamp(-1.0, 1.0).acos();
            let step = 2.0 * (1.0 - (0.0003 / (corner * rr)).min(1.0)).acos();
            let steps = (turn / step.max(1e-3)).ceil().clamp(1.0, 8.0) as usize;
            out.extend((0..=steps).map(|i| { let s = i as f32 / steps as f32; a * (1.0 - s) * (1.0 - s) + p * 2.0 * s * (1.0 - s) + b * s * s }));
        } else {
            out.push(sout[0]);
        }
        // the edge's inner points between the two roundings
        let (d0, d1) = (tl[k], len(sout) - tl[(k + 1) % n]);
        let mut d = 0.0;
        for i in 1..sout.len() - 1 {
            d += sout[i].distance(sout[i - 1]);
            if d > d0 && d < d1 {
                out.push(sout[i]);
            }
        }
    }
    out.dedup_by(|a, b| a.distance(*b) < 1e-7);
    while out.len() > 1 && out[0].distance(out[out.len() - 1]) < 1e-7 {
        out.pop();
    }
    out
}

/// The outline of one window as it is built (densified and rounded), as (radius, angle)
/// points round its centre in rim fractions: for the editor's checks.
pub fn window_outline(p: &WindowPlate) -> Vec<[f32; 2]> {
    window_loop(&p.outline, 0.0, 1.0, p.corner).into_iter().map(|q| [q.length(), q.x.atan2(q.y)]).collect()
}

/// a ring of `SEGMENTS` points at radius `r` (metres), at the lathe's angles so it meets the
/// flange strip edge to edge
fn ring(r: f32) -> Vec<Vec2> {
    (0..SEGMENTS).map(|j| { let a = j as f32 / SEGMENTS as f32 * TAU; Vec2::new(a.sin(), a.cos()) * r }).collect()
}

/// Insert midpoints into a closed loop where the dish `f` bends more than `tol` off an edge's
/// chord (up to four halvings), so walls and the top follow the dish.
fn split_loop(pts: &[Vec2], f: &dyn Fn(Vec2) -> f32, tol: f32) -> Vec<Vec2> {
    fn push(out: &mut Vec<Vec2>, a: Vec2, b: Vec2, f: &dyn Fn(Vec2) -> f32, tol: f32, depth: u32) {
        let m = (a + b) * 0.5;
        if depth > 0 && (f(m) - 0.5 * (f(a) + f(b))).abs() > tol {
            push(out, a, m, f, tol, depth - 1);
            out.push(m);
            push(out, m, b, f, tol, depth - 1);
        }
    }
    let mut out = Vec::new();
    for i in 0..pts.len() {
        out.push(pts[i]);
        push(&mut out, pts[i], pts[(i + 1) % pts.len()], f, tol, 4);
    }
    out
}

/// lyon works in millimetres (its fixed tolerances are for screen-sized coordinates)
const MM: f32 = 1000.0;

/// Fill closed loops even-odd with lyon: (positions, triangles); the loops' own points come
/// back bit for bit, so the top meets the walls exactly. `None` if lyon fails.
fn fill(loops: &[&[Vec2]]) -> Option<(Vec<Vec2>, Vec<[u32; 3]>)> {
    struct Ctor<'a>(&'a [Vec2], &'a [usize]);
    impl FillVertexConstructor<Vec2> for Ctor<'_> {
        fn new_vertex(&mut self, v: FillVertex) -> Vec2 {
            if let Some(VertexSource::Endpoint { id }) = v.sources().next() {
                if let Some(&k) = self.1.get(id.to_usize()).filter(|&&k| k != usize::MAX) {
                    return self.0[k];
                }
            }
            let p = v.position();
            Vec2::new(p.x, p.y) / MM
        }
    }
    let (mut pts, mut ids) = (Vec::new(), Vec::new());
    let mut b = Path::builder();
    for l in loops.iter().filter(|l| l.len() >= 3) {
        for (i, p) in l.iter().enumerate() {
            let q = point(p.x * MM, p.y * MM);
            let id = if i == 0 { b.begin(q) } else { b.line_to(q) }.to_usize();
            if ids.len() <= id {
                ids.resize(id + 1, usize::MAX);
            }
            ids[id] = pts.len();
            pts.push(*p);
        }
        b.end(true);
    }
    let path = b.build();
    let mut buf: VertexBuffers<Vec2, u32> = VertexBuffers::new();
    FillTessellator::new().tessellate_path(&path, &FillOptions::default(), &mut BuffersBuilder::new(&mut buf, Ctor(&pts, &ids))).ok()?;
    Some((buf.vertices, buf.indices.as_chunks::<3>().0.to_vec()))
}

/// spacing of the seed points inside the top, metres [design choice: about one ring segment at
/// the lip]
const SEED: f32 = 0.035;

/// Seed points on a triangular grid `SEED` apart inside the region the `loops` close (even-odd),
/// at least half a spacing from every outline and outside radius `hidden`.
fn seeds(loops: &[&[Vec2]], hidden: f32) -> Vec<Vec2> {
    let edges = || loops.iter().flat_map(|l| (0..l.len()).map(move |i| (l[i], l[(i + 1) % l.len()])));
    let (lo, hi) = loops.iter().flat_map(|l| l.iter()).fold((Vec2::splat(f32::MAX), Vec2::splat(f32::MIN)), |(lo, hi), &p| (lo.min(p), hi.max(p)));
    let inside = |p: Vec2| edges().filter(|&(a, b)| (a.y > p.y) != (b.y > p.y) && p.x < a.x + (p.y - a.y) / (b.y - a.y) * (b.x - a.x)).count() % 2 == 1;
    let clear = |p: Vec2| edges().all(|(a, b)| { let d = b - a; let t = ((p - a).dot(d) / d.length_squared().max(1e-12)).clamp(0.0, 1.0); p.distance(a + d * t) >= 0.5 * SEED });
    let dy = SEED * 0.866;
    let mut out = Vec::new();
    for i in 0..=((hi.y - lo.y) / dy) as usize {
        let y = lo.y + i as f32 * dy;
        let mut x = lo.x + if i % 2 == 1 { 0.5 * SEED } else { 0.0 };
        while x <= hi.x {
            let p = Vec2::new(x, y);
            if p.length() >= hidden && inside(p) && clear(p) {
                out.push(p);
            }
            x += SEED;
        }
    }
    out
}

/// Lawson edge flips to a Delaunay triangulation: an inner edge whose opposite corner lies in
/// the other triangle's circumcircle turns to the other diagonal. Outline edges (one triangle)
/// stay. Deterministic: triangles in order, each flipped at most once per pass.
fn delaunay(pos: &[Vec2], tris: &mut [[u32; 3]]) {
    let key = |a: u32, b: u32| ((a.min(b) as u64) << 32) | a.max(b) as u64;
    let p = |i: u32| pos[i as usize].as_dvec2();
    let orient = |a: u32, b: u32, c: u32| (p(b) - p(a)).perp_dot(p(c) - p(a));
    for _ in 0..64 {
        let mut by_edge: HashMap<u64, Vec<usize>> = HashMap::new();
        for (k, t) in tris.iter().enumerate() {
            for e in 0..3 {
                by_edge.entry(key(t[e], t[(e + 1) % 3])).or_default().push(k);
            }
        }
        let mut touched = vec![false; tris.len()];
        let mut flipped = false;
        for k in 0..tris.len() {
            for e in 0..3 {
                if touched[k] {
                    break;
                }
                let t = tris[k];
                let (a, b, c) = (t[e], t[(e + 1) % 3], t[(e + 2) % 3]);
                let Some(&o) = by_edge[&key(a, b)].iter().find(|&&o| o != k) else { continue };
                if touched[o] {
                    continue;
                }
                let Some(&d) = tris[o].iter().find(|&&v| v != a && v != b) else { continue };
                // convex quad: c and d on opposite sides of a-b, a and b on opposite sides of c-d
                if orient(a, b, c) * orient(a, b, d) >= 0.0 || orient(c, d, a) * orient(c, d, b) >= 0.0 {
                    continue;
                }
                let (pa, pb, pc) = (p(a) - p(d), p(b) - p(d), p(c) - p(d));
                let det = (pa.x * pa.x + pa.y * pa.y) * pb.perp_dot(pc) - (pb.x * pb.x + pb.y * pb.y) * pa.perp_dot(pc) + (pc.x * pc.x + pc.y * pc.y) * pa.perp_dot(pb);
                let det = if orient(a, b, c) < 0.0 { -det } else { det };
                if det > 1e-18 {
                    tris[k] = [c, a, d];
                    tris[o] = [c, d, b];
                    touched[k] = true;
                    touched[o] = true;
                    flipped = true;
                }
            }
        }
        if !flipped {
            break;
        }
    }
}

/// The triangles of the region the `loops` close: lyon fills it with seed points (`seeds`)
/// put in as 1 µm triangles (holes under even-odd, too small to see), since lyon takes no
/// loose points; then Delaunay flips and the dish refinement.
fn mesh(loops: &[&[Vec2]], f: &dyn Fn(Vec2) -> f32, hidden: f32) -> Option<(Vec<Vec2>, Vec<[u32; 3]>)> {
    let dots: Vec<[Vec2; 3]> = seeds(loops, hidden).into_iter().map(|p| [p, p + Vec2::new(1e-6, 0.0), p + Vec2::new(0.0, 1e-6)]).collect();
    let all: Vec<&[Vec2]> = loops.iter().copied().chain(dots.iter().map(|d| d.as_slice())).collect();
    let (mut pos, mut tris) = fill(&all)?;
    delaunay(&pos, &mut tris);
    refine(&mut pos, &mut tris, f, DISH_TOL, hidden);
    Some((pos, tris))
}

/// Split inner edges (used by two triangles) at their midpoint wherever the dish `f` bends
/// more than `tol` off them or they are longer than `MAX_EDGE`, up to six passes; edges with both ends inside radius `hidden`
/// (under the star plate) stay. The choice is per edge, so neighbours agree and the mesh stays
/// conforming; outline edges never split (the walls share them).
fn refine(pos: &mut Vec<Vec2>, tris: &mut Vec<[u32; 3]>, f: &dyn Fn(Vec2) -> f32, tol: f32, hidden: f32) {
    let key = |a: u32, b: u32| ((a.min(b) as u64) << 32) | a.max(b) as u64;
    for _ in 0..6 {
        let mut uses: HashMap<u64, u32> = HashMap::new();
        for t in tris.iter() {
            for e in 0..3 {
                *uses.entry(key(t[e], t[(e + 1) % 3])).or_default() += 1;
            }
        }
        let mut mids: HashMap<u64, u32> = HashMap::new();
        for t in tris.iter() {
            for e in 0..3 {
                let (a, b) = (t[e], t[(e + 1) % 3]);
                let k = key(a, b);
                if uses[&k] < 2 || mids.contains_key(&k) {
                    continue;
                }
                let (pa, pb) = (pos[a as usize], pos[b as usize]);
                let m = (pa + pb) * 0.5;
                if pa.length().max(pb.length()) < hidden {
                    continue;
                }
                if pa.distance(pb) > MAX_EDGE || (f(m) - 0.5 * (f(pa) + f(pb))).abs() > tol {
                    mids.insert(k, pos.len() as u32);
                    pos.push(m);
                }
            }
        }
        if mids.is_empty() {
            break;
        }
        let mut next = Vec::with_capacity(tris.len() * 2);
        for t in tris.iter() {
            let m = [0, 1, 2].map(|e| mids.get(&key(t[e], t[(e + 1) % 3])).copied());
            let at = |r: usize| (t[r % 3], m[r % 3]);
            match m.iter().filter(|x| x.is_some()).count() {
                0 => next.push(*t),
                1 => {
                    // rotate the split edge to a-b
                    let r = m.iter().position(|x| x.is_some()).unwrap_or(0);
                    let ((a, mab), (b, _), (c, _)) = (at(r), at(r + 1), at(r + 2));
                    let mab = mab.unwrap_or(a);
                    next.extend([[a, mab, c], [mab, b, c]]);
                }
                2 => {
                    // rotate the whole edge to c-a
                    let r = (m.iter().position(|x| x.is_none()).unwrap_or(0) + 1) % 3;
                    let ((a, mab), (b, mbc), (c, _)) = (at(r), at(r + 1), at(r + 2));
                    let (mab, mbc) = (mab.unwrap_or(a), mbc.unwrap_or(b));
                    next.extend([[mab, b, mbc], [a, mab, mbc], [a, mbc, c]]);
                }
                _ => {
                    let [mab, mbc, mca] = m.map(|x| x.unwrap_or(0));
                    let [a, b, c] = *t;
                    next.extend([[a, mab, mca], [mab, b, mbc], [mca, mbc, c], [mab, mbc, mca]]);
                }
            }
        }
        *tris = next;
    }
}

/// how far the top may bend off a triangle edge before it is split, metres [design choice: the
/// 1 mm between the plate and the star; on a dish that bulges out, as the power law does, the
/// chords sink under it and never reach the star]
const DISH_TOL: f32 = 0.001;
/// longest inner edge of the top, metres [design choice: normals interpolated along long
/// triangles show as seams on polished metal; the seeds keep edges about `SEED` long, this
/// catches the rest]
const MAX_EDGE: f32 = 0.06;
/// cosine of the turn above which an outline corner is sharp (two normals), 35 degrees
const SHARP_COS: f32 = 0.819;

/// One closed outline of the plate: its points, the in-plane normal pointing away from the
/// metal (averaged) per point, the wall's (point, normal) entries (sharp corners twice), its
/// wall depth (metres), and whether it is a window with a floor.
struct Loop {
    pts: Vec<Vec2>,
    avg: Vec<Vec2>,
    wall: Vec<(usize, Vec2)>,
    depth: f32,
    floor: bool,
}

impl Loop {
    /// `hole`: the metal is outside the loop (a window, the plate's inner edge)
    fn new(pts: Vec<Vec2>, hole: bool, depth: f32, floor: bool) -> Self {
        let m = pts.len();
        let area: f32 = (0..m).map(|i| { let (p, q) = (pts[i], pts[(i + 1) % m]); p.x * q.y - q.x * p.y }).sum();
        // (d.y, -d.x) is the outside of a loop of positive area
        let s = if area > 0.0 { 1.0 } else { -1.0 } * if hole { -1.0 } else { 1.0 };
        let en: Vec<Vec2> = (0..m).map(|i| { let d = pts[(i + 1) % m] - pts[i]; Vec2::new(d.y, -d.x).normalize_or_zero() * s }).collect();
        let (mut avg, mut wall) = (Vec::with_capacity(m), Vec::with_capacity(m));
        for i in 0..m {
            let (a, b) = (en[(i + m - 1) % m], en[i]);
            avg.push((a + b).normalize_or(b));
            if a.dot(b) < SHARP_COS {
                wall.push((i, a));
                wall.push((i, b));
            } else {
                wall.push((i, avg[i]));
            }
        }
        Loop { pts, avg, wall, depth, floor }
    }
}

/// Build the plate `p` (09-remaster 9.23): `top(r)` is the dish height at radius r (metres),
/// `phase` the spoke angle, `ch` the chamfer, `hidden` the radius the star plate covers (the
/// top is not refined under it). Draws nothing for a window of fewer than three points or if
/// lyon fails.
#[allow(clippy::too_many_arguments)]
pub(super) fn plate(out: &mut Vec<Vertex>, side: f32, rr: f32, h: f32, top: &dyn Fn(f32) -> f32, phase: f32, p: &WindowPlate, ch: f32, hidden: f32, c: [f32; 4]) {
    if p.count > 0 && p.outline.len() < 3 {
        return;
    }
    let f = |q: Vec2| top(q.length());
    let mut loops = vec![Loop::new(split_loop(&ring(p.r1 * rr), &f, DISH_TOL), false, p.depth * h, false)];
    if p.r0 > 0.0 {
        loops.push(Loop::new(split_loop(&ring(p.r0 * rr), &f, DISH_TOL), true, p.depth * h, false));
    }
    let (wall_depth, floor) = if p.groove > 0.0 { (p.groove * h, true) } else { (p.depth * h, false) };
    for k in 0..p.count {
        let base = phase + (k as f32 + p.phase) * TAU / p.count as f32;
        let pts = split_loop(&window_loop(&p.outline, base, rr, p.corner), &f, DISH_TOL);
        if pts.len() >= 3 {
            loops.push(Loop::new(pts, true, wall_depth, floor));
        }
    }
    // a point `down` under the plate's top, with the dish's normal there
    let lift = p.lift * h - 0.001;
    let surf = |q: Vec2, down: f32| -> V {
        let r = q.length();
        let n = if r < 1e-5 {
            Vec3::X
        } else {
            let (r0, r1) = ((r - 1e-4).max(0.0), r + 1e-4);
            let s = (top(r1) - top(r0)) / (r1 - r0);
            let u = q / r;
            Vec3::new(1.0, -s * u.x, -s * u.y).normalize()
        };
        V { p: Vec3::new(top(r) + lift - down, q.x, q.y), n, c }
    };
    let mut emit = |pos: &[Vec2], tris: &[[u32; 3]], down: f32| {
        for t in tris {
            tri(out, side, t.map(|i| surf(pos[i as usize], down)));
        }
    };
    // the top: every outline moved in by the chamfer
    let inset: Vec<Vec<Vec2>> = loops.iter().map(|l| l.pts.iter().zip(&l.avg).map(|(&q, &e)| q - e * ch).collect()).collect();
    let refs: Vec<&[Vec2]> = inset.iter().map(|l| l.as_slice()).collect();
    let Some((pos, tris)) = mesh(&refs, &f, hidden) else { return };
    emit(&pos, &tris, 0.0);
    // the groove floors
    for l in loops.iter().filter(|l| l.floor) {
        if let Some((pos, tris)) = mesh(&[&l.pts], &f, hidden) {
            emit(&pos, &tris, l.depth);
        }
    }
    // chamfer and wall along every outline (the star plate's rings and normals)
    for (l, ins) in loops.iter().zip(&inset) {
        let at = |q: Vec2, down: f32, n: Vec3| V { p: Vec3::new(top(q.length()) + lift - down, q.x, q.y), n: n.normalize(), c };
        let rings: [Vec<V>; 3] = [
            l.wall.iter().map(|&(i, e)| at(ins[i], 0.0, Vec3::new(0.92, e.x * 0.38, e.y * 0.38))).collect(),
            l.wall.iter().map(|&(i, e)| at(l.pts[i], ch, Vec3::new(0.38, e.x * 0.92, e.y * 0.92))).collect(),
            l.wall.iter().map(|&(i, e)| at(l.pts[i], l.depth, Vec3::new(0.0, e.x, e.y))).collect(),
        ];
        let m = l.wall.len();
        for r in rings.windows(2) {
            for i in 0..m {
                let j = (i + 1) % m;
                tri(out, side, [r[0][i], r[0][j], r[1][j]]);
                tri(out, side, [r[0][i], r[1][j], r[1][i]]);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::{wheel, Rim, RimLook, RimStyle, WheelFace, WheelSpec, BACKING_RINGS, METAL_ENV};
    use super::*;

    const R: f32 = 0.353;
    const HW: f32 = 0.153;
    const PHASE: f32 = 0.2;

    fn build(p: &WindowPlate) -> Vec<Vertex> {
        let style = RimStyle { groups: Vec::new(), windows: vec![p.clone()], ..Default::default() };
        let look = RimLook { spokes: style.spokes, phase: PHASE, metal: [0.8; 3], backing: [[0.1; 3]; BACKING_RINGS] };
        let face = WheelFace { layer: 0.0, fit: [[1.0, 0.0, 0.5], [0.0, 1.0, 0.5]], colour: [0.4; 4], lip: [0.3; 4] };
        wheel(&WheelSpec { radius: R, half_width: HW, side: 1.0, face: Some(face), rim: Some(Rim { style, look }), scale: [1.0; 3], tyre: None })
    }

    /// does a metal triangle facing out of the face (normal.x > 0.5) cover the face point at
    /// rim fraction `r`, wheel angle `a`?
    fn covered(v: &[Vertex], r: f32, a: f32) -> bool {
        let rr = R * super::super::RIM;
        let q = Vec2::new(a.sin(), -a.cos()) * r * rr; // view space: z negated
        v.chunks(3).filter(|t| t[0].env == METAL_ENV && t.iter().all(|v| v.normal[0] > 0.5)).any(|t| {
            let p: Vec<Vec2> = t.iter().map(|v| Vec2::new(v.pos[1], v.pos[2])).collect();
            let s = |a: Vec2, b: Vec2| (b - a).perp_dot(q - a);
            let (d0, d1, d2) = (s(p[0], p[1]), s(p[1], p[2]), s(p[2], p[0]));
            (d0 >= 0.0 && d1 >= 0.0 && d2 >= 0.0) || (d0 <= 0.0 && d1 <= 0.0 && d2 <= 0.0)
        })
    }

    fn faces_out(v: &[Vertex]) -> bool {
        v.chunks(3).all(|t| {
            let p = t.iter().map(|v| Vec3::from(v.pos)).collect::<Vec<_>>();
            let n: Vec3 = t.iter().map(|v| Vec3::from(v.normal)).sum();
            (p[1] - p[0]).cross(p[2] - p[0]).dot(n) > 0.0
        })
    }

    /// holes really open (nothing on top at their centres), the metal between them is there,
    /// the wheel keeps the original's box and every triangle faces along its normals
    #[test]
    fn holed_plate_builds() {
        for outline in [circle(0.55, 0.17, 16), ngon(0.55, 0.16, 6, 0.0), teardrop(0.3, 0.78, 0.06, 0.3), trapezoid(0.35, 0.75, 0.4, 0.5)] {
            for corner in [0.0, 0.02] {
                let p = WindowPlate { outline: outline.clone(), corner, ..Default::default() };
                let v = build(&p);
                let (lo, hi) = v.iter().fold((Vec3::splat(f32::MAX), Vec3::splat(f32::MIN)), |(lo, hi), v| (lo.min(v.pos.into()), hi.max(v.pos.into())));
                let want = Vec3::new(HW, R, R);
                assert!((hi - want).abs().max_element() < 1e-5 && (lo + want).abs().max_element() < 1e-5);
                assert!(faces_out(&v));
                let o = window_outline(&p);
                let rc = o.iter().map(|q| q[0]).fold(f32::MAX, f32::min) * 0.5 + o.iter().map(|q| q[0]).fold(0.0, f32::max) * 0.5;
                for k in 0..p.count {
                    let a = PHASE + (k as f32 + p.phase) * TAU / p.count as f32;
                    assert!(!covered(&v, rc, a), "hole {k} covered");
                    assert!(covered(&v, rc, a + 0.5 * TAU / p.count as f32), "no metal after hole {k}");
                }
                assert!(v.len() / 3 < 4000, "{} triangles", v.len() / 3);
            }
        }
    }

    /// on a flat face: the walls are two quad rings per outline, one quad per outline edge (sharp
    /// corners add no faces)
    #[test]
    fn wall_quads_per_outline() {
        let flat = |_: f32| 0.0;
        let rr = 0.25;
        for (outline, at_least) in [(circle(0.55, 0.17, 16), 16), (ngon(0.55, 0.16, 6, 0.0), 6)] {
            let p = WindowPlate { r0: 0.2, outline, ..Default::default() };
            let per = window_outline(&p).len();
            assert!(per >= at_least);
            let mut v = Vec::new();
            plate(&mut v, 1.0, rr, HW, &flat, 0.0, &p, 0.004, 0.0, [1.0; 4]);
            let top = v.chunks(3).filter(|t| t.iter().all(|v| (v.pos[0] + 0.001).abs() < 1e-6)).count();
            let walls = v.len() / 3 - top;
            assert!(top > 0);
            assert_eq!(walls, 2 * 2 * (p.count * per + 2 * SEGMENTS), "{per}");
        }
    }

    /// groove windows keep a floor at the groove depth; their walls go no deeper
    #[test]
    fn grooves_have_floors() {
        let flat = |_: f32| 0.0;
        let p = WindowPlate { outline: trapezoid(0.3, 0.8, 0.12, 0.2), groove: 0.04, count: 8, ..Default::default() };
        let mut v = Vec::new();
        plate(&mut v, 1.0, 0.25, HW, &flat, 0.0, &p, 0.004, 0.0, [1.0; 4]);
        let floor = -0.001 - 0.04 * HW;
        let n = v.chunks(3).filter(|t| t.iter().all(|v| (v.pos[0] - floor).abs() < 1e-6 && v.normal[0] > 0.99)).count();
        assert!(n >= 8 * 2, "{n} floor triangles");
        // only the plate's outer wall reaches the plate depth
        let deep: Vec<f32> = v.iter().filter(|v| v.pos[0] < floor - 1e-6).map(|v| Vec2::new(v.pos[1], v.pos[2]).length()).collect();
        assert!(!deep.is_empty() && deep.iter().all(|&r| (r - 0.84 * 0.25).abs() < 1e-4));
    }

    /// no windows: a full cover over the whole face
    #[test]
    fn cover_builds() {
        let v = build(&WindowPlate { count: 0, outline: Vec::new(), ..Default::default() });
        assert!(faces_out(&v));
        for a in [0.0, 1.0, 2.5] {
            assert!(covered(&v, 0.5, a) && covered(&v, 0.8, a));
        }
    }

    /// the generators stay inside their radii and widths; rounding keeps a window inside its
    /// sharp outline
    #[test]
    fn outlines() {
        let r = |o: &[[f32; 2]]| (o.iter().map(|q| q[0]).fold(f32::MAX, f32::min), o.iter().map(|q| q[0]).fold(0.0, f32::max));
        let (lo, hi) = r(&circle(0.5, 0.1, 16));
        assert!((lo - 0.4).abs() < 1e-5 && (hi - 0.6).abs() < 1e-5);
        let (lo, hi) = r(&teardrop(0.3, 0.8, 0.0, 0.2));
        assert!((lo - 0.3).abs() < 1e-5 && (hi - 0.8).abs() < 1e-3, "{lo} {hi}");
        let (lo, hi) = r(&teardrop(0.3, 0.8, 0.1, 0.2));
        assert!((lo - 0.3).abs() < 1e-3 && (hi - 0.8).abs() < 1e-3, "{lo} {hi}");
        let t = trapezoid(0.3, 0.8, 0.1, 0.2);
        let sharp = WindowPlate { outline: t.clone(), ..Default::default() };
        let round = WindowPlate { outline: t, corner: 0.03, ..Default::default() };
        let (so, ro) = (window_outline(&sharp), window_outline(&round));
        assert!(ro.len() > so.len(), "{} {}", ro.len(), so.len());
        // the outer arc is an arc: every point of it at radius 0.8
        assert!(so.iter().filter(|q| q[0] > 0.79).count() >= 2);
        let (lo, hi) = r(&ro);
        // (rounding an inner arc corner cuts its chord, a hair inside the arc)
        assert!(lo > 0.3 - 1e-3 && hi < 0.8 + 1e-5, "{lo} {hi}");
        assert!(ro.iter().all(|q| q[1].abs() <= 0.1 + 1e-4));
    }
}

