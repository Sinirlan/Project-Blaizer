//! REMASTER vents and grilles for the generated bodies (09-remaster 9.22 phase 4). The black
//! vent faces of the MYO [O] are picked per car by a box, recessed by `body::inset_glass`
//! before the bevel, and filled with slats or a hex mesh standing on the recess floor. All of
//! it a design choice.
//!
//! Input in car-local game space; output in view space (game z negated) like `body::body`.

use super::body::{tri, Face, Glass};
use super::lamps::{hit, vert};
use crate::scene::Vertex;
use glam::{Vec2, Vec3};
use std::collections::HashMap;

/// One vent (both sides of the car when it is mirrored).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Vent {
    /// the MYO face colour of the vent faces [O]
    pub from: [u8; 3],
    /// the faces of `from` whose centre, x mirrored to +x, lies in this box (game space)
    pub lo: [f32; 3],
    pub hi: [f32; 3],
    /// the recess; `housing.rgb` is a tag of its own, given to the picked faces
    pub housing: Glass,
    pub kind: VentKind,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum VentKind {
    /// an empty recess (the Titan's rear bumper: the exhausts go there, phase 5)
    Plain,
    /// bars across the car (game x) every `pitch` (m), `width` (m) wide, split by `blades`
    /// upright spine bars `spine` (m) wide along the recess standing proud of them, evenly
    /// across (the R-Build 2's louvred tail: one wide spine down the middle; `blades: 0` =
    /// plain louvres); `tilt` (degrees) leans every slat down the vent, its outer edge lower
    /// than its foot, like the blades of a window blind (the Serpent's tail grille, #174 phase
    /// 4): a slat then runs `1 / cos(tilt)` times as long, so its tip stays as high over the
    /// floor; 0 = upright. The rows are laid on the vent's mean plane and dropped onto the floor
    /// along its normal, so on a floor curving away from that plane (the Serpent's tail, faired)
    /// they sink towards its ends; `level` (#464) drops them horizontally instead, so every slat
    /// keeps one height and runs parallel to a level outline
    Slats { pitch: f32, width: f32, blades: usize, spine: f32, tilt: f32, level: bool },
    /// a honeycomb of cells `radius` (m, centre to corner) with walls `wall` (m) thick,
    /// split by `blades` upright blades standing proud of it, evenly across
    Hex { radius: f32, wall: f32, blades: usize },
    /// a stepped louvre cover like the Lamborghini Huracán's engine cover (#465, the user's
    /// photo): `count` wide plates one after the other down the vent, each spanning the floor
    /// from side to side and rising from `low` of the recess depth at its upper edge to
    /// `LOUVRE_HIGH` at its lower one, so each stands a step over the next with a riser facing
    /// down the vent; `ribs` triangular gussets `width` (m) thick, evenly across, run from the
    /// top of each step (the first plate's: the recess's upper wall) down onto the plate
    Louvres { count: usize, ribs: usize, width: f32, low: f32 },
}

/// slats stand this much of the recess depth tall, the honeycomb this much
const SLAT_HEIGHT: f32 = 0.8;
const HEX_HEIGHT: f32 = 0.6;
/// the upright blades of the honeycomb and the slats: this much of the recess depth tall, this
/// wide (m)
const BLADE_HEIGHT: f32 = 0.95;
const BLADE_WIDTH: f32 = 0.01;
/// bar samples along their length (m)
const STEP: f32 = 0.01;
/// matte dark plastic of the slats and the honeycomb, lighter than the recess floor so it reads
const BAR: [f32; 4] = [0.07, 0.07, 0.075, 1.0];
/// `Louvres`: the plates' lower edges stand this much of the recess depth over the floor, just
/// under the body surface; the gussets reach this much of a plate's length down it; the plates'
/// matt dark grey [design choice, after the photo's satin graphite]
const LOUVRE_HIGH: f32 = 0.95;
const RIB_LEN: f32 = 0.5;
const LOUVRE: [f32; 4] = [0.16, 0.16, 0.17, 1.0];
/// `Louvres`: the plates' sample spacing (m), and how far in from the floor's outline the rows
/// start (m: a foot exactly on it may miss)
const LOUVRE_STEP: f32 = 0.02;
const LOUVRE_EDGE: f32 = 0.001;

/// Tag the vent faces (`Vent::housing.rgb`) and recess them; per vent, whether it was recessed.
pub fn recess(faces: &mut Vec<Face>, vents: &[Vent]) -> Vec<bool> {
    for v in vents {
        let (lo, hi) = (Vec3::from(v.lo), Vec3::from(v.hi));
        for f in faces.iter_mut().filter(|f| f.rgb == v.from) {
            let c = f.pts.iter().map(|&p| Vec3::from(p)).sum::<Vec3>() / f.pts.len() as f32;
            let c = Vec3::new(c.x.abs(), c.y, c.z);
            if c.cmpge(lo).all() && c.cmple(hi).all() {
                f.rgb = v.housing.rgb;
            }
        }
    }
    vents.iter().map(|v| super::body::inset_glass(faces, &v.housing, &[])).collect()
}

/// The insides of the recessed vents of `faces` (after `recess`): per connected region of a
/// vent's floor, slats or a honeycomb in a frame on the region's mean plane.
pub fn vents(faces: &[Face], vents: &[Vent], recessed: &[bool]) -> Vec<Vertex> {
    let mut out = Vec::new();
    for (v, _) in vents.iter().zip(recessed).filter(|(_, &r)| r) {
        let floor: Vec<&Face> = faces.iter().filter(|f| f.rgb == v.housing.rgb).collect();
        for region in regions(&floor) {
            let n = region.iter().map(|f| normal(f) * area(f)).sum::<Vec3>().normalize_or(Vec3::Y);
            let su = Vec3::X.reject_from(n).try_normalize().unwrap_or_else(|| n.any_orthonormal_vector());
            let sv = n.cross(su);
            let pts: Vec<Vec3> = region.iter().flat_map(|f| f.pts.iter().map(|&p| Vec3::from(p))).collect();
            let c = pts.iter().sum::<Vec3>() / pts.len() as f32;
            let (mut lo, mut hi) = (Vec2::MAX, Vec2::MIN);
            for p in &pts {
                let q = Vec2::new((*p - c).dot(su), (*p - c).dot(sv));
                lo = lo.min(q);
                hi = hi.max(q);
            }
            // level slats: the plane's rows (su runs across the car, so a row is one height on
            // it) go onto the floor along the normal's horizontal part
            let drop = match v.kind {
                VentKind::Slats { level: true, .. } => Vec3::new(n.x, 0.0, n.z).try_normalize().unwrap_or(n),
                _ => n,
            };
            let at = |q: Vec2| hit(&region, c + su * q.x + sv * q.y, drop);
            let depth = v.housing.inset;
            // `blades` upright bars `w` wide along v, evenly spaced across u, standing proud of
            // the fill
            let spines = |out: &mut Vec<Vertex>, blades: usize, w: f32| {
                for b in 1..=blades {
                    let u = lo.x + (hi.x - lo.x) * b as f32 / (blades + 1) as f32;
                    bar(out, &at, Vec2::new(u, lo.y), Vec2::new(u, hi.y), w, depth * BLADE_HEIGHT, 0.0);
                }
            };
            match v.kind {
                VentKind::Plain => {}
                VentKind::Slats { pitch, width, blades, spine, tilt, .. } => {
                    let lean = tilt.to_radians();
                    // a leaning slat's tip lies `h tan(lean)` down the vent from its foot: the
                    // rows keep that much off the vent's lower end, so the lowest tip stays over
                    // the floor (v runs down the vent when `sv` points down)
                    let margin = depth * SLAT_HEIGHT * lean.tan();
                    let (lo_y, hi_y) = if sv.y < 0.0 { (lo.y, hi.y - margin) } else { (lo.y + margin, hi.y) };
                    let mut y = lo_y + (hi_y - lo_y).rem_euclid(pitch) / 2.0;
                    while y <= hi_y {
                        bar(&mut out, &at, Vec2::new(lo.x, y), Vec2::new(hi.x, y), width, depth * SLAT_HEIGHT, lean);
                        y += pitch;
                    }
                    spines(&mut out, blades, spine);
                }
                VentKind::Hex { radius, wall, blades } => {
                    // flat-topped cells; each draws its three upper edges, its neighbours the rest
                    let (du, dv) = (1.5 * radius, 3f32.sqrt() * radius);
                    let corner = |k: usize| Vec2::from_angle(k as f32 * std::f32::consts::FRAC_PI_3) * radius;
                    let (i0, i1) = ((lo.x / du).floor() as i32 - 1, (hi.x / du).ceil() as i32 + 1);
                    let (j0, j1) = ((lo.y / dv).floor() as i32 - 1, (hi.y / dv).ceil() as i32 + 1);
                    for i in i0..=i1 {
                        for j in j0..=j1 {
                            let m = Vec2::new(i as f32 * du, (j as f32 + 0.5 * i.rem_euclid(2) as f32) * dv);
                            for k in 0..3 {
                                bar(&mut out, &at, m + corner(k), m + corner(k + 1), wall, depth * HEX_HEIGHT, 0.0);
                            }
                        }
                    }
                    spines(&mut out, blades, BLADE_WIDTH);
                }
                VentKind::Louvres { count, ribs, width, low } => {
                    // down the vent: v from its upper edge to its lower one
                    let (along, v0, v1) = if sv.y < 0.0 { (sv, lo.y, hi.y) } else { (-sv, hi.y, lo.y) };
                    let cover = Louvre { at: &at, lo: lo.x, hi: hi.x, along, v0, v1, depth, low };
                    cover.build(&mut out, count, ribs, width);
                }
            }
        }
    }
    out
}

/// One `VentKind::Louvres` cover over a floor region: `at` maps a point of the vent's plane
/// frame onto the floor; u runs across `lo..hi`, v down the vent from `v0` to `v1` (the plane
/// direction `along`); `depth` is the recess depth, `low` the plates' upper edge height in it.
struct Louvre<'a> {
    at: &'a dyn Fn(Vec2) -> Option<(Vec3, Vec3)>,
    lo: f32,
    hi: f32,
    along: Vec3,
    v0: f32,
    v1: f32,
    depth: f32,
    low: f32,
}

impl Louvre<'_> {
    /// the plane coordinate `v` a fraction `t` down the vent
    fn v(&self, t: f32) -> f32 {
        self.v0 + (self.v1 - self.v0) * t
    }

    /// The floor's extent across at `v` (u of its first and last foot, 0.1 mm), or None where
    /// the row misses the floor.
    fn span(&self, v: f32) -> Option<(f32, f32)> {
        let on = |u: f32| (self.at)(Vec2::new(u, v)).is_some();
        let n = ((self.hi - self.lo) / 0.005).ceil().max(1.0) as usize;
        let us: Vec<f32> = (0..=n).map(|i| self.lo + (self.hi - self.lo) * i as f32 / n as f32).collect();
        let first = us.iter().position(|&u| on(u))?;
        let last = us.iter().rposition(|&u| on(u))?;
        // bisect each end between the last miss and the first hit
        let edge = |mut inside: f32, mut outside: f32| {
            while (inside - outside).abs() > 1e-4 {
                let m = (inside + outside) / 2.0;
                if on(m) { inside = m } else { outside = m }
            }
            inside
        };
        let a = if first > 0 { edge(us[first], us[first - 1]) } else { us[0] };
        let b = if last < n { edge(us[last], us[last + 1]) } else { us[n] };
        Some((a + LOUVRE_EDGE, b - LOUVRE_EDGE))
    }

    /// the point a fraction `s` across the floor's span `(a, b)` at `v`, `h` (m) over the floor,
    /// with the floor's normal
    fn point(&self, (a, b): (f32, f32), s: f32, v: f32, h: f32) -> Option<(Vec3, Vec3)> {
        let (p, n) = (self.at)(Vec2::new(a + (b - a) * s, v))?;
        Some((p + n * h, n))
    }

    /// the plates, their risers and the gussets
    fn build(&self, out: &mut Vec<Vertex>, count: usize, ribs: usize, width: f32) {
        if count == 0 {
            return;
        }
        let (high, low) = (self.depth * LOUVRE_HIGH, self.depth * self.low);
        // the rows stay `LOUVRE_EDGE` inside the floor's upper and lower edges
        let pad = LOUVRE_EDGE / (self.v1 - self.v0).abs().max(1e-4);
        let t_of = |k: f32| pad + (1.0 - 2.0 * pad) * k / count as f32;
        let len = (self.v1 - self.v0).abs() / count as f32;
        let wide = (self.hi - self.lo).abs();
        let cols = ((wide / LOUVRE_STEP).ceil() as usize).max(1);
        let rows = ((len / LOUVRE_STEP).ceil() as usize).max(1);
        let lv = |p: Vec3, n: Vec3| vert(p, n, LOUVRE, 0.0);
        for k in 0..count {
            let (ta, tb) = (t_of(k as f32), t_of(k as f32 + 1.0));
            // the plate's top: rows down it, each `cols` samples across the floor's span there
            let grid: Vec<Option<Vec<(Vec3, Vec3)>>> = (0..=rows)
                .map(|r| {
                    let s = r as f32 / rows as f32;
                    let v = self.v(ta + (tb - ta) * s);
                    let h = low + (high - low) * s;
                    let span = self.span(v)?;
                    (0..=cols).map(|c| self.point(span, c as f32 / cols as f32, v, h)).collect()
                })
                .collect();
            let p = |r: usize, c: usize| grid[r].as_ref().map(|row| row[c].0);
            // a vertex normal from its neighbours across and down the plate, on the floor's side
            let normal = |r: usize, c: usize| -> Option<Vec3> {
                let n0 = grid[r].as_ref()?[c].1;
                let du = p(r, (c + 1).min(cols))? - p(r, c.saturating_sub(1))?;
                let dv = p((r + 1).min(rows), c)? - p(r.saturating_sub(1), c)?;
                let n = du.cross(dv).normalize_or(n0);
                Some(if n.dot(n0) < 0.0 { -n } else { n })
            };
            for r in 0..rows {
                for c in 0..cols {
                    let q = [(r, c), (r, c + 1), (r + 1, c + 1), (r + 1, c)];
                    let Some(v) = q.iter().map(|&(r, c)| Some(lv(p(r, c)?, normal(r, c)?))).collect::<Option<Vec<_>>>() else { continue };
                    tri(out, [v[0], v[1], v[2]]);
                    tri(out, [v[0], v[2], v[3]]);
                }
            }
            // the walls: down the plate's two ends to the floor, across its lower edge (the
            // riser) to the next plate's upper edge or, the last one, the floor; the first
            // plate's upper edge to the floor
            let foot = |r: usize, c: usize, h: f32| grid[r].as_ref().map(|row| row[c].0 - row[c].1 * h);
            let wall = |out: &mut Vec<Vertex>, a: Vec3, b: Vec3, a1: Vec3, b1: Vec3, n: Vec3| {
                tri(out, [lv(a, n), lv(b, n), lv(b1, n)]);
                tri(out, [lv(a, n), lv(b1, n), lv(a1, n)]);
            };
            for r in 0..rows {
                let h = |r: usize| low + (high - low) * r as f32 / rows as f32;
                for (c, inner) in [(0, 1), (cols, cols - 1)] {
                    let (Some(a), Some(b), Some(i)) = (p(r, c), p(r + 1, c), p(r, inner)) else { continue };
                    let (Some(a1), Some(b1)) = (foot(r, c, h(r)), foot(r + 1, c, h(r + 1))) else { continue };
                    wall(out, a, b, a1, b1, (a - i).normalize_or_zero());
                }
            }
            let drop = if k + 1 < count { high - low } else { high };
            for c in 0..cols {
                let (Some(a), Some(b), Some(a1), Some(b1)) = (p(rows, c), p(rows, c + 1), foot(rows, c, drop), foot(rows, c + 1, drop)) else { continue };
                wall(out, a, b, a1, b1, self.along);
            }
            if k == 0 {
                for c in 0..cols {
                    let (Some(a), Some(b), Some(a1), Some(b1)) = (p(0, c), p(0, c + 1), foot(0, c, low), foot(0, c + 1, low)) else { continue };
                    wall(out, a, b, a1, b1, -self.along);
                }
            }
            // the gussets: a wedge at each rib from the step's top (the riser of the plate
            // before, `high` over the floor) down onto this plate `RIB_LEN` of its length on
            for b in 1..=ribs {
                let s = b as f32 / (ribs + 1) as f32;
                let tc = ta + (tb - ta) * RIB_LEN;
                let hc = low + (high - low) * RIB_LEN;
                let (va, vc) = (self.v(ta), self.v(tc));
                // the rib's two faces, `width` apart across, each the triangle top / foot / tip
                let (Some(ja), Some(jc)) = (self.span(va), self.span(vc)) else { continue };
                let side = |ds: f32| -> Option<[Vec3; 3]> {
                    let sa = s + ds / (ja.1 - ja.0).max(1e-4);
                    let sc = s + ds / (jc.1 - jc.0).max(1e-4);
                    Some([self.point(ja, sa, va, high)?.0, self.point(ja, sa, va, low)?.0, self.point(jc, sc, vc, hc)?.0])
                };
                let (Some(l), Some(r)) = (side(-width / 2.0), side(width / 2.0)) else { continue };
                let across = (r[0] - l[0]).normalize_or_zero();
                tri(out, [lv(l[0], -across), lv(l[1], -across), lv(l[2], -across)]);
                tri(out, [lv(r[0], across), lv(r[1], across), lv(r[2], across)]);
                // its sloping top, from the step's top edge to the tip
                let top = (r[2] - r[0]).cross(across).normalize_or_zero();
                let top = if top.dot(self.along) > 0.0 { top } else { -top };
                wall(out, l[0], r[0], l[2], r[2], top);
            }
        }
    }
}

fn normal(f: &Face) -> Vec3 {
    Vec3::from(f.normal).normalize_or_zero()
}

fn area(f: &Face) -> f32 {
    let p0 = Vec3::from(f.pts[0]);
    (1..f.pts.len().saturating_sub(1)).map(|k| (Vec3::from(f.pts[k]) - p0).cross(Vec3::from(f.pts[k + 1]) - p0).length() / 2.0).sum()
}

/// `faces` split into groups that share a corner (1 mm).
fn regions<'a>(faces: &[&'a Face]) -> Vec<Vec<&'a Face>> {
    let key = |p: [f32; 3]| p.map(|c| (c * 1000.0).round() as i32);
    let mut parent: Vec<usize> = (0..faces.len()).collect();
    fn root(parent: &mut [usize], mut i: usize) -> usize {
        while parent[i] != i {
            parent[i] = parent[parent[i]];
            i = parent[i];
        }
        i
    }
    let mut seen: HashMap<[i32; 3], usize> = HashMap::new();
    for (i, f) in faces.iter().enumerate() {
        for &p in &f.pts {
            if let Some(&j) = seen.get(&key(p)) {
                let (a, b) = (root(&mut parent, i), root(&mut parent, j));
                parent[a] = b;
            } else {
                seen.insert(key(p), i);
            }
        }
    }
    let mut groups: HashMap<usize, Vec<&Face>> = HashMap::new();
    for (i, &f) in faces.iter().enumerate() {
        groups.entry(root(&mut parent, i)).or_default().push(f);
    }
    let mut g: Vec<Vec<&Face>> = groups.into_values().collect();
    g.sort_by(|a, b| (a[0] as *const Face).cmp(&(b[0] as *const Face)));
    g
}

/// One sample across a bar: its left and right foot on the floor, each with the floor's normal;
/// `None` where a foot misses the floor (the bar is cut there).
type Sample = Option<[(Vec3, Vec3); 2]>;

/// a run of samples is one straight piece when its inner feet lie this close (m) to the line
/// between its end feet
const MERGE_TOL: f32 = 2e-4;

/// `s` with the inner samples of straight runs dropped (#372): a run over one floor plane (the
/// same normal at every foot) whose inner feet lie within `MERGE_TOL` of the chord between its
/// end feet becomes one segment. The cuts (`None`) and the samples next to them stay, so the
/// bar is clipped as before; over a fold the runs break at the samples either side of it.
fn merge(s: &[Sample]) -> Vec<Sample> {
    let straight = |run: &[Sample]| {
        let (Some(first), Some(last)) = (run[0], run[run.len() - 1]) else { return false };
        run.iter().all(|q| {
            q.is_some_and(|q| {
                (0..2).all(|k| {
                    let (a, b, p) = (first[k].0, last[k].0, q[k].0);
                    let along = (b - a).normalize_or_zero();
                    let off = (p - a).reject_from_normalized(along).length();
                    (q[k].1 - first[k].1).length() < 1e-4 && off <= MERGE_TOL
                })
            })
        })
    };
    let mut out = Vec::with_capacity(s.len());
    let mut i = 0;
    while i < s.len() {
        out.push(s[i]);
        if s[i].is_none() {
            i += 1;
            continue;
        }
        let mut j = i + 1;
        while j + 1 < s.len() && straight(&s[i..=j + 1]) {
            j += 1;
        }
        i = j;
    }
    out
}

/// A bar `w` wide and `h` tall standing on the floor (`at`: a point of the vent's plane frame
/// onto the floor, with the floor's normal) from `a` to `b`: a top and two walls, kept where
/// both its edges land on the floor. `lean` (radians) tips it over towards its lower side,
/// `h / cos(lean)` long so its top stays `h` over the floor.
#[allow(clippy::too_many_arguments)]
fn bar(out: &mut Vec<Vertex>, at: &dyn Fn(Vec2) -> Option<(Vec3, Vec3)>, a: Vec2, b: Vec2, w: f32, h: f32, lean: f32) {
    let d = b - a;
    let len = d.length();
    if len < 1e-4 {
        return;
    }
    let side = d.perp() / len * (w / 2.0);
    let n = (len / STEP).ceil() as usize;
    let s: Vec<Sample> = (0..=n)
        .map(|i| {
            let p = a + d * (i as f32 / n as f32);
            Some([at(p - side)?, at(p + side)?])
        })
        .collect();
    let s = merge(&s);
    if lean != 0.0 {
        leaning(out, &s, h, lean);
        return;
    }
    for pair in s.windows(2) {
        let (Some([l0, r0]), Some([l1, r1])) = (pair[0], pair[1]) else { continue };
        let up = |q: (Vec3, Vec3)| q.0 + q.1 * h;
        let top = |p: Vec3, q: (Vec3, Vec3)| vert(p, q.1, BAR, 0.0);
        tri(out, [top(up(l0), l0), top(up(r0), r0), top(up(r1), r1)]);
        tri(out, [top(up(l0), l0), top(up(r1), r1), top(up(l1), l1)]);
        // the walls face away from the bar's middle
        for (e0, e1, o0, o1) in [(l0, l1, r0, r1), (r0, r1, l0, l1)] {
            let wn = (e0.0 - o0.0 + e1.0 - o1.0).normalize_or_zero();
            let wv = |p: Vec3| vert(p, wn, BAR, 0.0);
            tri(out, [wv(e0.0), wv(e1.0), wv(up(e1))]);
            tri(out, [wv(e0.0), wv(up(e1)), wv(up(e0))]);
        }
    }
}

/// `bar`'s leaning plank over its floor samples `s` (left and right foot, each with the floor's
/// normal): the floor normal turned by `lean` towards the lower foot gives the plank's
/// direction; the two walls are its lower and upper faces, the top its outer edge.
fn leaning(out: &mut Vec<Vertex>, s: &[Sample], h: f32, lean: f32) {
    let (sl, cl) = lean.sin_cos();
    let len = h / cl;
    for pair in s.windows(2) {
        let (Some([l0, r0]), Some([l1, r1])) = (pair[0], pair[1]) else { continue };
        // per sample: the direction to the lower foot, the plank's direction and the normal of
        // its face on the left foot's side
        let frame = |l: (Vec3, Vec3), r: (Vec3, Vec3)| {
            let across = (l.0 - r.0).normalize_or_zero();
            let down = if across.y > 0.0 { -across } else { across };
            let n = (l.1 + r.1).normalize_or(l.1);
            let d = n * cl + down * sl;
            (d, across * cl - n * sl * across.dot(down).signum())
        };
        let ((d0, side0), (d1, side1)) = (frame(l0, r0), frame(l1, r1));
        let up = |q: (Vec3, Vec3), d: Vec3| q.0 + d * len;
        let top = |p: Vec3, d: Vec3| vert(p, d, BAR, 0.0);
        tri(out, [top(up(l0, d0), d0), top(up(r0, d0), d0), top(up(r1, d1), d1)]);
        tri(out, [top(up(l0, d0), d0), top(up(r1, d1), d1), top(up(l1, d1), d1)]);
        for (e0, e1, n0, n1) in [(l0, l1, side0, side1), (r0, r1, -side0, -side1)] {
            tri(out, [vert(e0.0, n0, BAR, 0.0), vert(e1.0, n1, BAR, 0.0), vert(up(e1, d1), n1, BAR, 0.0)]);
            tri(out, [vert(e0.0, n0, BAR, 0.0), vert(up(e1, d1), n1, BAR, 0.0), vert(up(e0, d0), n0, BAR, 0.0)]);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TAG: [u8; 3] = [3, 3, 3];

    fn quad(x0: f32, x1: f32) -> Face {
        Face { pts: vec![[x0, 0.0, 0.0], [x1, 0.0, 0.0], [x1, 0.1, 0.0], [x0, 0.1, 0.0]], normal: [0.0, 0.0, 1.0], colour: vec![[0.0; 4]; 4], uv: vec![[0.0; 2]; 4], layer: -1.0, env: 0.0, rgb: TAG, smooth_deg: 0.0, remesh: 0.0 }
    }

    /// slats and honeycomb on a flat floor facing +z: inside its outline, between the floor and
    /// the bar height; two separate floors are two regions
    #[test]
    fn insides_on_the_floor() {
        let faces = vec![quad(0.0, 0.3), quad(0.5, 0.7)];
        let housing = Glass { rgb: TAG, frame: 0.0, inset: 0.03, tint: 0.5 };
        let refs: Vec<&Face> = faces.iter().collect();
        assert_eq!(regions(&refs).len(), 2);
        for (kind, hf) in [(VentKind::Slats { pitch: 0.02, width: 0.006, blades: 0, spine: 0.01, tilt: 0.0, level: false }, SLAT_HEIGHT), (VentKind::Hex { radius: 0.012, wall: 0.003, blades: 1 }, BLADE_HEIGHT)] {
            let v = Vent { from: [1; 3], lo: [0.0; 3], hi: [1.0; 3], housing, kind };
            let out = vents(&faces, &[v], &[true]);
            assert!(!out.is_empty());
            assert!(out.iter().any(|p| p.pos[0] > 0.5));
            for p in &out {
                let (x, y, z) = (p.pos[0], p.pos[1], -p.pos[2]);
                assert!(((-1e-5..=0.3 + 1e-5).contains(&x) || (0.5 - 1e-5..=0.7 + 1e-5).contains(&x)) && (-1e-5..=0.1 + 1e-5).contains(&y), "{x} {y}");
                assert!((-1e-5..=0.03 * hf + 1e-5).contains(&z), "{z}");
            }
            assert!(vents(&faces, &[v], &[false]).is_empty());
        }
    }

    /// slats with a spine: one upright bar down the middle of each region, taller than the
    /// louvres; without it nothing stands above the slat height
    #[test]
    fn slats_with_a_spine() {
        let faces = vec![quad(0.0, 0.3), quad(0.5, 0.7)];
        let housing = Glass { rgb: TAG, frame: 0.0, inset: 0.03, tint: 0.5 };
        let v = |blades| Vent { from: [1; 3], lo: [0.0; 3], hi: [1.0; 3], housing, kind: VentKind::Slats { pitch: 0.02, width: 0.006, blades, spine: 0.02, tilt: 0.0, level: false } };
        let plain = vents(&faces, &[v(0)], &[true]);
        assert!(plain.iter().all(|p| -p.pos[2] <= 0.03 * SLAT_HEIGHT + 1e-5));
        let spined = vents(&faces, &[v(1)], &[true]);
        assert!(spined.len() > plain.len());
        let tall: Vec<&Vertex> = spined.iter().filter(|p| -p.pos[2] > 0.03 * SLAT_HEIGHT + 1e-4).collect();
        assert!(!tall.is_empty());
        for p in &tall {
            let x = p.pos[0];
            assert!((x - 0.15).abs() <= 0.01 + 1e-5 || (x - 0.6).abs() <= 0.01 + 1e-5, "{x}");
            assert!(-p.pos[2] <= 0.03 * BLADE_HEIGHT + 1e-5);
        }
        assert!(tall.iter().any(|p| (p.pos[0] - 0.14).abs() < 1e-4) && tall.iter().any(|p| (p.pos[0] - 0.61).abs() < 1e-4));
    }
    /// #372: on a flat floor every slat is one segment (6 triangles): no vertex inside its
    /// length; the honeycomb too keeps no sample between a wall's two corners
    #[test]
    fn flat_bars_are_one_segment() {
        let faces = vec![quad(0.0, 0.3)];
        let housing = Glass { rgb: TAG, frame: 0.0, inset: 0.03, tint: 0.5 };
        let v = Vent { from: [1; 3], lo: [0.0; 3], hi: [1.0; 3], housing, kind: VentKind::Slats { pitch: 0.02, width: 0.006, blades: 0, spine: 0.01, tilt: 0.0, level: false } };
        let out = vents(&faces, &[v], &[true]);
        assert!(!out.is_empty() && out.len().is_multiple_of(18));
        assert!(out.iter().all(|p| p.pos[0] < 1e-4 || p.pos[0] > 0.3 - 1e-4), "a slat split along its length");
        // a straight row of samples on one plane collapses to its ends; a cut stays a cut
        let foot = |x: f32| Some([(Vec3::new(x, 0.0, 0.0), Vec3::Z), (Vec3::new(x, 0.01, 0.0), Vec3::Z)]);
        let s: Vec<Sample> = vec![foot(0.0), foot(0.01), foot(0.02), None, foot(0.04), foot(0.05), foot(0.06)];
        let m = merge(&s);
        assert_eq!(m.len(), 5);
        assert!(m[2].is_none() && m[0] == s[0] && m[1] == s[2] && m[3] == s[4] && m[4] == s[6]);
    }

    /// #372: over a folded floor (two planes meeting at 30 deg) a slat keeps its samples either
    /// side of the fold, and every vertex stays within the slat height of a floor plane
    #[test]
    fn folded_floor_keeps_the_fold() {
        let (c, s) = (30f32.to_radians().cos(), 30f32.to_radians().sin());
        // plane A: x 0..0.15 at z 0; plane B: from x 0.15 rising to z 0.15 s over 0.15 c
        let a = Face { pts: vec![[0.0, 0.0, 0.0], [0.15, 0.0, 0.0], [0.15, 0.1, 0.0], [0.0, 0.1, 0.0]], ..quad(0.0, 0.1) };
        let e = |x: f32| [0.15 + x * c, 0.0, x * s];
        let f = |x: f32| [0.15 + x * c, 0.1, x * s];
        let b = Face { pts: vec![e(0.0), e(0.15), f(0.15), f(0.0)], normal: [-s, 0.0, c], ..quad(0.0, 0.1) };
        let housing = Glass { rgb: TAG, frame: 0.0, inset: 0.03, tint: 0.5 };
        let v = Vent { from: [1; 3], lo: [0.0; 3], hi: [1.0; 3], housing, kind: VentKind::Slats { pitch: 0.05, width: 0.006, blades: 0, spine: 0.01, tilt: 0.0, level: false } };
        let out = vents(&[a, b], &[v], &[true]);
        assert!(!out.is_empty());
        // per slat: one segment on A, the step across the fold, one segment on B
        assert!(out.len().is_multiple_of(54), "{}", out.len());
        let (na, nb) = (Vec3::Z, Vec3::new(-s, 0.0, c));
        let ob = Vec3::new(0.15, 0.0, 0.0);
        for q in &out {
            let g = Vec3::new(q.pos[0], q.pos[1], -q.pos[2]);
            let (ha, hb) = (g.dot(na), (g - ob).dot(nb));
            let near = |h: f32| (-1e-4..=0.03 * SLAT_HEIGHT + 1e-4).contains(&h);
            assert!(near(ha) || near(hb), "{g}");
        }
    }

    /// #465: louvre plates on a flat floor stay inside its outline and under `LOUVRE_HIGH` × the
    /// depth; each plate rises from `low` at its upper edge to the full height at its lower one,
    /// where it steps down to the next plate's upper edge
    #[test]
    fn louvres_step_down_the_vent() {
        let faces = vec![quad(0.0, 0.3)];
        let housing = Glass { rgb: TAG, frame: 0.0, inset: 0.05, tint: 0.5 };
        let v = Vent { from: [1; 3], lo: [0.0; 3], hi: [1.0; 3], housing, kind: VentKind::Louvres { count: 2, ribs: 2, width: 0.01, low: 0.2 } };
        let out = vents(&faces, &[v], &[true]);
        assert!(!out.is_empty() && out.len().is_multiple_of(3));
        let (high, low) = (0.05 * LOUVRE_HIGH, 0.05 * 0.2);
        for p in &out {
            let (x, y, h) = (p.pos[0], p.pos[1], -p.pos[2]);
            assert!((-1e-4..=0.3 + 1e-4).contains(&x) && (-1e-4..=0.1 + 1e-4).contains(&y), "{x} {y}");
            assert!((-1e-4..=high + 1e-4).contains(&h), "{h}");
        }
        // the floor faces +z, so its plane's v (`sv`) points up +y and down the vent is -y: the
        // first plate runs from y 0.1 to 0.05, the second from 0.05 to 0
        let at = |y: f32, h: f32| out.iter().any(|p| (p.pos[1] - y).abs() < 2e-3 && (-p.pos[2] - h).abs() < 1e-3);
        assert!(at(0.1, low) && at(0.05, high) && at(0.05, low) && at(0.0, high));
        // at full height on the first plate's upper edge only the gussets (against the recess's
        // upper wall), `width` thick at a third and two thirds across
        let tops: Vec<f32> = out.iter().filter(|p| (p.pos[1] - 0.1).abs() < 2e-3 && (-p.pos[2] - high).abs() < 1e-3).map(|p| p.pos[0]).collect();
        assert!(!tops.is_empty() && tops.iter().all(|&x| (x - 0.1).abs() < 0.006 || (x - 0.2).abs() < 0.006), "{tops:?}");
        assert!(vents(&faces, &[v], &[false]).is_empty());
    }

    /// window-blind slats (#174 phase 4) on a floor tilted 45 deg like the Serpent's tail grille
    /// (facing back and up): leaning 45 deg down the vent, their outer edges face straight back
    /// (level slats), and nothing rises more than the slat height off the floor
    #[test]
    fn leaning_slats_lie_level() {
        let (c, s) = (std::f32::consts::FRAC_1_SQRT_2, std::f32::consts::FRAC_1_SQRT_2);
        // the floor: x 0..0.3, 0.1 m up its slope (0, s, c) from z 0; its normal (0, s, -c)
        let p = |x: f32, v: f32| [x, v * s, v * c];
        let floor = Face { pts: vec![p(0.0, 0.0), p(0.0, 0.1), p(0.3, 0.1), p(0.3, 0.0)], normal: [0.0, s, -c], colour: vec![[0.0; 4]; 4], uv: vec![[0.0; 2]; 4], layer: -1.0, env: 0.0, rgb: TAG, smooth_deg: 0.0, remesh: 0.0 };
        let housing = Glass { rgb: TAG, frame: 0.0, inset: 0.03, tint: 0.5 };
        let v = Vent { from: [1; 3], lo: [0.0; 3], hi: [1.0; 3], housing, kind: VentKind::Slats { pitch: 0.02, width: 0.004, blades: 0, spine: 0.0, tilt: 45.0, level: false } };
        let out = vents(&[floor], &[v], &[true]);
        assert!(!out.is_empty());
        // game -z (straight back) is view +z
        assert!(out.iter().any(|q| q.normal[2] > 0.999), "no slat edge faces straight back");
        let n = Vec3::new(0.0, s, -c);
        for q in &out {
            let g = Vec3::new(q.pos[0], q.pos[1], -q.pos[2]);
            assert!(g.dot(n) <= 0.03 * SLAT_HEIGHT + 1e-4 && g.dot(n) >= -1e-4, "{g}");
        }
    }
}
