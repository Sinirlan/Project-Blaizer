//! REMASTER lamp insides for the generated bodies (09-remaster 9.22 phase 3). The housings are
//! recessed by `body::inset_glass` before the bevel; this adds what sits in them, placed from
//! the MYO halo records [O]: projector units or a chrome reflector bowl with glowing bulbs
//! (headlights), glowing LED dots and bars (tail lights). All of it a design choice.
//!
//! Input in car-local game space; output in view space (game z negated) like `body::body`.

use super::body::{tri, Face, Glass};
use super::{LAMP_ENV, METAL_ENV};
use crate::scene::Vertex;
use glam::{Vec2, Vec3};

/// One kind of lamp on a car.
#[derive(Clone, Debug, PartialEq)]
pub struct Lamp {
    /// the recess; `housing.rgb` tags the lamp's faces
    pub housing: Glass,
    pub kind: LampKind,
}

#[derive(Clone, Debug, PartialEq)]
pub enum LampKind {
    /// a projector assembly (after the user's headlight photo and reference model, #174 phase 2):
    /// the recess floor becomes a bare housing of colour `floor` (the lamp texture goes), and
    /// `count` projector pods of `radius` (m) stand in it along the halo segment, their places
    /// spread over its middle `span` (1: the whole segment, one in the middle of each share), each
    /// moved across the lens to midway between its edges there; `rows` of them stacked up the
    /// lens (`ROW_PITCH` radii apart; 1 = one row on the segment). A pod (`projector`) points
    /// along the car whatever the lens's slope: a dark barrel, a chrome bezel, a glowing LED ring,
    /// a dark glass lens. `paths` light up like a `Leds` cluster's, in its frame with `inset`
    /// (facet widths); none = pods only.
    Projectors { count: usize, radius: f32, rows: usize, span: f32, floor: [u8; 3], paths: Vec<LedPath>, inset: f32 },
    /// an integrated assembly (after the user's fog-lamp photo): one chrome reflector bowl over
    /// the whole lens, dipping from the lens's rim down to the recess floor at each bulb, `count`
    /// bulbs along the halo segment in `rows` rows `pitch` (m) apart; a glowing dome of radius
    /// `bulb` (m) at each. `paths` / `inset` as for `Projectors`.
    Reflector { count: usize, rows: usize, pitch: f32, bulb: f32, paths: Vec<LedPath>, inset: f32 },
    /// a tail-light cluster after a hand-drawn layout, on the lens's main facet, in a frame along
    /// its edges: origin `inset` in from both the inner and the bottom edge, x along the bottom
    /// edge outwards, y up the inner edge. Units are the facet's width. Every `paths` entry lights
    /// up as dots or a bar and also yields `LampStrip`s for the halo to follow.
    Leds { paths: Vec<LedPath>, inset: f32 },
}

/// One pattern of a `LampKind::Leds` cluster: the polyline `pts` in the cluster's frame, drawn
/// `repeat` times, each copy `step` further than the one before.
#[derive(Clone, Debug, PartialEq)]
pub struct LedPath {
    pub pts: Vec<[f32; 2]>,
    pub repeat: usize,
    pub step: [f32; 2],
    pub style: LedStyle,
    /// a dot needs the lens this far (facet widths) further along its path: a column that stops
    /// short of the lens's top edge (0: only the dot itself must fit)
    pub margin: f32,
    /// the pattern is lit only where the lens reaches this far (facet widths) past it on all
    /// four sides: slats that stop short of a rim tube (0: the lens's edge)
    pub clip: f32,
    /// the pattern's runs give the halo strips to follow; off for small fill patterns (slats,
    /// dashes), whose glows would blur into one
    pub halo: bool,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum LedStyle {
    /// dots of radius `dot` about `pitch` apart with both ends of each segment on one
    Dots { dot: f32, pitch: f32 },
    /// a continuous light tube `width` wide
    Bar { width: f32 },
    /// a light tube `width` wide round the lens's outline, `gap` in from its edge; `closed`
    /// all the way round, else open along the inner end, stopping `gap + width` short of
    /// its line (`pts` unused)
    Rim { width: f32, gap: f32, closed: bool },
}

/// The straight run of a lit pattern (flat ends, those of a run reaching half a dot pitch past its
/// end dots), for the lamp's halo to follow [design choice]: the
/// halo record `halo` (an index into the `halos` given to `lamps`) is drawn as this segment
/// (car-local game space, like `HaloSeg`) of half-width `width` (m).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LampStrip {
    pub halo: usize,
    pub p0: [f32; 3],
    pub p1: [f32; 3],
    pub width: f32,
    /// the strip's p0 / p1 end meets another strip of the run at a bend (they cross-fade)
    pub join: [bool; 2],
}

/// A MYO halo record in car-local game space: the segment's ends and the light's colour.
pub struct HaloSeg {
    pub p0: [f32; 3],
    pub p1: [f32; 3],
    pub rgb: [u8; 3],
}

/// a halo further than this (m) from a lamp's faces gets nothing
const REACH: f32 = 0.2;
/// the lenses and bars sit this far (m) off the pane, against z-fighting
const LIFT: f32 = 0.002;
/// a projector unit's segments round
const UNIT_SEGMENTS: usize = 24;
/// where a projector pod points: along the car (game +z)
const AIM: Vec3 = Vec3::Z;
/// a pod's face stays this far (m) under the lens's surface
const CLEAR: f32 = 0.006;
/// a pod's face, outside in: (radius, offset forward from the barrel's end), in pod radii; a
/// chrome bezel with a rolled outer edge, its inner step back, the LED ring, a dark step down to
/// the lens [design choice, after the reference model's pods]
const FACE_PROFILE: [(f32, f32); 7] = [(1.0, 0.0), (0.96, 0.1), (0.9, 0.12), (0.84, 0.12), (0.82, 0.06), (0.62, 0.06), (0.6, 0.0)];
/// the colour and `Vertex::env` of each band of `FACE_PROFILE` (None: the lamp's glow)
const FACE_LOOK: [Option<([f32; 4], f32)>; 6] =
    [Some((CHROME, METAL_ENV)), Some((CHROME, METAL_ENV)), Some((CHROME, METAL_ENV)), Some((CHROME, METAL_ENV)), None, Some((CUP_BACK, 0.0))];
/// the lens dome: how far it stands proud of its rim (pod radii), its rings rim to centre
const LENS_SAG: f32 = 0.15;
const LENS_RINGS: usize = 4;
/// a pod's barrel: dark, under the lacquer
const BARREL: [f32; 4] = [0.08, 0.08, 0.09, 1.0];
/// `Vertex::env` of the body paint (the cage's Gouraud faces): shaded under the car-shading
/// option's lacquer (Enhanced: clear coat), so a dark colour stays dark and still mirrors the sky
/// (`METAL_ENV` keeps only a colour's hue)
const GLOSS_ENV: f32 = 1.0;
/// the projector lens: dark glass under the lacquer
const LENS_GLASS: [f32; 4] = [0.12, 0.14, 0.18, 1.0];
/// how far (`SCAN` steps) a projector unit's position looks for the lens's edges across it
const ACROSS_N: usize = 60;
/// rows of units: centre spacing in barrel radii
const ROW_PITCH: f32 = 2.2;
/// a reflector bowl's grid (m): the bowl is sampled this finely over the lens outline
const BOWL_STEP: f32 = 0.005;
/// bisection steps finding where a bowl cell's edge leaves the lens (`BOWL_STEP` / 4096, ~1 µm)
const EDGE_BISECT: usize = 12;
/// two lens faces whose normals' dot product is under this (about 1°) meet in a fold, and a bowl
/// cell across it is cut along their shared edge (#359)
const FOLD_COS: f32 = 0.99985;
/// at most this many fold cuts in one bowl cell (three faces meeting in it need two)
const FOLD_CUTS: usize = 4;
/// a bulb dome's segments round and rings up
const DOME_SEGMENTS: usize = 10;
const DOME_RINGS: usize = 3;
const CHROME: [f32; 4] = [0.8, 0.8, 0.82, 1.0];
const CUP_BACK: [f32; 4] = [0.06, 0.06, 0.06, 1.0];
/// the outline scan of a tail cluster: spacing (m) and samples a side (±0.5 m)
const SCAN: f32 = 0.005;
const SCAN_N: usize = 200;
/// an LED dot's segments
const DOT_SEGMENTS: usize = 12;
/// a bar's pieces along its path (m)
const BAR_STEP: f32 = 0.01;
/// the stretch of the outline the bottom edge is fitted to, in facet widths from the inner end
/// (clear of a corner sliver and of a lower outer part)
const BOTTOM_FIT: (f32, f32) = (0.1, 0.5);
/// a lens of more faces than this was faired by a `Smooth` zone into small triangles (a cage lens
/// is 1-6 facets); `cluster` then frames it round the pane's mean normal
const FAIRED_PANE: usize = 8;

/// The insides of the lamps of `faces` (after `inset_glass`), one set per halo in `halos` that
/// sits on a lamp's faces, and the strips the `Leds` patterns give those halos.
pub fn lamps(faces: &[Face], halos: &[HaloSeg], lamps: &[Lamp]) -> (Vec<Vertex>, Vec<LampStrip>) {
    let (mut out, mut strips) = (Vec::new(), Vec::new());
    for (hi, h) in halos.iter().enumerate() {
        let (a, b) = (Vec3::from(h.p0), Vec3::from(h.p1));
        let near = |f: &Face| f.pts.iter().map(|&p| seg_dist(Vec3::from(p), a, b)).fold(f32::MAX, f32::min);
        let reach = |l: &Lamp| faces.iter().filter(|f| f.rgb == l.housing.rgb).map(near).fold(f32::MAX, f32::min);
        let Some(lamp) = lamps.iter().min_by(|x, y| reach(x).total_cmp(&reach(y))).filter(|l| reach(l) < REACH) else {
            continue;
        };
        let pane: Vec<&Face> = faces.iter().filter(|f| f.rgb == lamp.housing.rgb && near(f) < REACH).collect();
        let n = pane.iter().map(|f| Vec3::from(f.normal).normalize_or_zero()).sum::<Vec3>().normalize_or(Vec3::Y);
        let onto = |q: Vec3| hit(&pane, q, n);
        let glow = [h.rgb[0] as f32 / 255.0, h.rgb[1] as f32 / 255.0, h.rgb[2] as f32 / 255.0, 1.0];
        match &lamp.kind {
            LampKind::Projectors { count, radius, rows, span, paths, inset, .. } => {
                let (count, rows) = (*count, (*rows).max(1));
                // rows stack up the lens: along the pane's normal x the segment, turned upwards
                let up = n.cross(b - a).normalize_or(Vec3::Y);
                let up = if up.y < 0.0 { -up } else { up };
                for r in 0..rows {
                    let lift = up * ((r as f32 - (rows - 1) as f32 / 2.0) * ROW_PITCH * radius);
                    for i in 0..count {
                        // across the lens: midway between its edges up and down from the segment
                        let t = 0.5 + ((i as f32 + 0.5) / count as f32 - 0.5) * span;
                        let q = a.lerp(b, t);
                        let edge = |dir: Vec3| (1..=ACROSS_N).map(|k| k as f32 * SCAN).take_while(|&d| onto(q + dir * d).is_some()).last().unwrap_or(0.0);
                        let q = q + up * ((edge(up) - edge(-up)) / 2.0);
                        let Some((c, cn)) = onto(q + lift) else { continue };
                        projector(&mut out, c, cn, *radius, lamp.housing.inset, glow);
                    }
                }
                if !paths.is_empty() {
                    cluster(&mut out, &mut strips, hi, &pane, (a + b) / 2.0, glow, paths, *inset);
                }
            }
            LampKind::Reflector { count, rows, pitch, bulb, paths, inset } => {
                let rows = (*rows).max(1);
                let up = n.cross(b - a).normalize_or(Vec3::Y);
                let up = if up.y < 0.0 { -up } else { up };
                let foci: Vec<Vec3> = (0..rows)
                    .flat_map(|r| {
                        let lift = up * ((r as f32 - (rows - 1) as f32 / 2.0) * pitch);
                        (0..*count).map(move |i| a.lerp(b, (i as f32 + 0.5) / *count as f32) + lift)
                    })
                    .collect();
                bowl(&mut out, &pane, n, a, b, &foci, lamp.housing.inset, *bulb, glow);
                if !paths.is_empty() {
                    cluster(&mut out, &mut strips, hi, &pane, (a + b) / 2.0, glow, paths, *inset);
                }
            }
            LampKind::Leds { paths, inset } => cluster(&mut out, &mut strips, hi, &pane, (a + b) / 2.0, glow, paths, *inset),
        }
    }
    (out, strips)
}

/// Distance from `p` to the segment `a`–`b`.
fn seg_dist(p: Vec3, a: Vec3, b: Vec3) -> f32 {
    let ab = b - a;
    let t = ((p - a).dot(ab) / ab.length_squared().max(1e-12)).clamp(0.0, 1.0);
    p.distance(a + ab * t)
}

/// Where the line through `q` along `n` meets the outermost of `faces`: the point and that
/// face's normal.
pub(super) fn hit(faces: &[&Face], q: Vec3, n: Vec3) -> Option<(Vec3, Vec3)> {
    hit_face(faces, q, n).map(|(p, m, _)| (p, m))
}

/// `hit`, and the index in `faces` of the face it met.
fn hit_face(faces: &[&Face], q: Vec3, n: Vec3) -> Option<FaceHit> {
    let o = q + n * 0.3;
    let mut best: Option<(f32, Vec3, usize)> = None;
    for (fi, f) in faces.iter().enumerate() {
        let p0 = Vec3::from(f.pts[0]);
        for k in 1..f.pts.len().saturating_sub(1) {
            let (e1, e2) = (Vec3::from(f.pts[k]) - p0, Vec3::from(f.pts[k + 1]) - p0);
            // Möller–Trumbore, the ray going -n
            let pv = (-n).cross(e2);
            let det = e1.dot(pv);
            if det.abs() < 1e-12 {
                continue;
            }
            let tv = o - p0;
            let u = tv.dot(pv) / det;
            let qv = tv.cross(e1);
            let v = (-n).dot(qv) / det;
            let t = e2.dot(qv) / det;
            if u >= 0.0 && v >= 0.0 && u + v <= 1.0 && t > 0.0 && best.is_none_or(|b| t < b.0) {
                best = Some((t, Vec3::from(f.normal).normalize_or(n), fi));
            }
        }
    }
    best.map(|(t, fnorm, fi)| (o - n * t, fnorm, fi))
}

pub(super) fn vert(p: Vec3, n: Vec3, color: [f32; 4], env: f32) -> Vertex {
    Vertex { pos: [p.x, p.y, -p.z], uv: [0.0; 2], color, layer: -1.0, normal: [n.x, n.y, -n.z], env }
}

/// The bare housing of a `Projectors` lamp: its recessed faces (`inset_glass` has run, so the
/// faces still tagged `housing.rgb` are the floor; the frame walls are rubber) lose the lamp
/// texture and become glossy plastic of the lamp's `floor` colour [design choice: the photo's
/// grey housing]. Other kinds keep the texture under their reflectors.
pub fn housing_floor(faces: &mut [Face], lamp: &Lamp) {
    let LampKind::Projectors { floor, .. } = lamp.kind else { return };
    let c = [floor[0] as f32 / 255.0, floor[1] as f32 / 255.0, floor[2] as f32 / 255.0, 1.0];
    for f in faces.iter_mut().filter(|f| f.rgb == lamp.housing.rgb) {
        f.layer = -1.0;
        f.env = GLOSS_ENV;
        f.colour.iter_mut().for_each(|k| *k = c);
    }
}

/// One ring of a turned part: radius (m from the axis), offset along the axis (m from the
/// face) and the surface normal there as (outwards, along the axis).
type Ring = (f32, f32, Vec2);

/// A projector pod on the floor point `c` (normal `cn`) of a housing `depth` deep, `r` its
/// radius, after the user's reference model (#174 phase 2, `koenigsegg+ccx.blend`): the pod
/// points along the car (`AIM`, game +z) whatever the lens's slope, its round face upright and
/// as high in the housing as the lens's surface lets it (`CLEAR` under it), its barrel running
/// back until it is under the floor, which rises behind it on a lens sloping up the bonnet (the
/// pods step back up the lamp). The face: a chrome bezel, a glowing LED ring, a dark glass lens
/// [design choice]. A lens facing along `AIM` has no room above the floor: the face then lies
/// parallel to it, `CLEAR` under it.
fn projector(out: &mut Vec<Vertex>, c: Vec3, cn: Vec3, r: f32, depth: f32, glow: [f32; 4]) {
    let axis = AIM;
    // up in the face's plane, the way the lens's normal leans out of it
    let lean = cn.reject_from(axis);
    let (s, k) = (lean.length(), cn.dot(axis));
    let face_depth = FACE_PROFILE.iter().map(|p| p.1).fold(LENS_SAG, f32::max) * r;
    let room = depth - CLEAR - face_depth * k.max(0.0);
    let (p, len) = if s < 0.3 {
        let p = c + cn * room.max(0.0);
        (p, room.max(0.0) + LIFT * 2.0)
    } else {
        let up = lean / s;
        let p = c + up * (room / s - r);
        // the barrel's top line sinks under the floor at `k` per metre back
        (p, ((p + up * r - c).dot(cn) + LIFT * 2.0) / k.max(0.2))
    };
    let u = axis.any_orthonormal_vector();
    let v = axis.cross(u);
    let e = |j: usize| {
        let a = j as f32 / UNIT_SEGMENTS as f32 * std::f32::consts::TAU;
        u * a.cos() + v * a.sin()
    };
    let at = |ring: Ring, e: Vec3| (p + e * ring.0 + axis * ring.1, (e * ring.2.x + axis * ring.2.y).normalize_or(axis));
    let mut band = |a: Ring, b: Ring, colour: [f32; 4], env: f32| {
        for j in 0..UNIT_SEGMENTS {
            let (e0, e1) = (e(j), e(j + 1));
            let [q0, q1, q2, q3] = [at(a, e0), at(a, e1), at(b, e1), at(b, e0)];
            let m = |x: (Vec3, Vec3)| vert(x.0, x.1, colour, env);
            tri(out, [m(q0), m(q1), m(q2)]);
            if b.0 > 1e-6 {
                tri(out, [m(q0), m(q2), m(q3)]);
            }
        }
    };
    band((r, -len, Vec2::X), (r, 0.0, Vec2::X), BARREL, GLOSS_ENV);
    // the face's flat bands, outside in: each segment's normal is its direction turned outwards
    // (the profile runs towards the axis)
    for (j, w) in FACE_PROFILE.windows(2).enumerate() {
        let (a, b) = ((w[0].0 * r, w[0].1 * r), (w[1].0 * r, w[1].1 * r));
        let nn = Vec2::new(b.1 - a.1, a.0 - b.0).normalize_or(Vec2::Y);
        let (colour, env) = match FACE_LOOK[j] {
            Some(look) => look,
            None => (glow, LAMP_ENV),
        };
        band((a.0, a.1, nn), (b.0, b.1, nn), colour, env);
    }
    // the lens: a spherical cap from the profile's last ring, `LENS_SAG` radii proud
    let &(a, base) = FACE_PROFILE.last().unwrap_or(&(0.0, 0.0));
    let (a, base, sag) = (a * r, base * r, LENS_SAG * r);
    let big = (a * a + sag * sag) / (2.0 * sag);
    let centre = base + sag - big;
    let ring = |rho: f32| -> Ring {
        let z = centre + (big * big - rho * rho).max(0.0).sqrt();
        (rho, z, Vec2::new(rho, z - centre) / big)
    };
    for j in 0..LENS_RINGS {
        let (r0, r1) = (a * (1.0 - j as f32 / LENS_RINGS as f32), a * (1.0 - (j + 1) as f32 / LENS_RINGS as f32));
        band(ring(r0), ring(r1), LENS_GLASS, GLOSS_ENV);
    }
}

/// The reflector bowl of a `Reflector` lamp: a chrome sheet over the lens (grid `BOWL_STEP` in a
/// frame on the pane, u along the halo segment), lifted off the floor by `depth` × (distance to
/// the nearest focus / the farthest such distance on the lens)², so it meets the surface at the
/// lens's farthest rim point and dips to the floor at each focus (creases between foci like a
/// multi-reflector lamp); a glowing dome of radius `bulb` at each focus on the floor. The grid's
/// cells are clipped to the lens's outline where it crosses them (#357), so the sheet ends on the
/// outline, not on a staircase of whole cells, where the lens's edge is slanted to the grid; and
/// cut where the floor folds (two of its faces at an angle, `FOLD_COS`) along the faces' shared
/// edge (#359), so the sheet's crease is that straight line, not a zigzag of cells drawn flat
/// across it. A point on the fold is lifted along both faces' mitre, so the two sides share it
/// and the sheet has no crack there.
#[allow(clippy::too_many_arguments)]
fn bowl(out: &mut Vec<Vertex>, pane: &[&Face], n: Vec3, a: Vec3, b: Vec3, foci: &[Vec3], depth: f32, bulb: f32, glow: [f32; 4]) {
    let u = (b - a).reject_from(n).normalize_or(n.any_orthonormal_vector());
    let v = n.cross(u);
    let mid = (a + b) / 2.0;
    let ray = |q: Vec2| mid + u * q.x + v * q.y;
    let at = |s: f32, t: f32| hit_face(pane, ray(Vec2::new(s, t)), n);
    let half = ((SCAN * SCAN_N as f32 / 2.0) / BOWL_STEP).ceil() as i32;
    let x = |i: i32| i as f32 * BOWL_STEP;
    let grid: Vec<Vec<Option<FaceHit>>> = (-half..=half).map(|i| (-half..=half).map(|j| at(x(i), x(j))).collect()).collect();
    let f2: Vec<Vec2> = foci.iter().map(|c| Vec2::new((*c - mid).dot(u), (*c - mid).dot(v))).collect();
    let near = |p: Vec2| f2.iter().map(|c| p.distance(*c)).fold(f32::MAX, f32::min);
    let mut reach = 0.0f32;
    for (i, col) in grid.iter().enumerate() {
        for (j, h) in col.iter().enumerate() {
            if h.is_some() {
                reach = reach.max(near(Vec2::new(x(i as i32 - half), x(j as i32 - half))));
            }
        }
    }
    if reach < 1e-3 || f2.is_empty() {
        return;
    }
    let height = |p: Vec2| depth * (near(p) / reach).powi(2).min(1.0);
    // the sheet: a point lifted off its floor point by the height field, shaded by the height
    // field's slope
    let point_at = |s: SheetPt| -> Vertex {
        let (q, d) = (s.q, BOWL_STEP / 2.0);
        let (ds, dt) = ((height(q + Vec2::X * d) - height(q - Vec2::X * d)) / (2.0 * d), (height(q + Vec2::Y * d) - height(q - Vec2::Y * d)) / (2.0 * d));
        let nn = (s.fnorm - u * ds - v * dt).normalize_or(s.fnorm);
        vert(s.p + s.lift * (height(q) + LIFT), nn, CHROME, METAL_ENV)
    };
    let floor_pt = |q: Vec2, (p, fnorm, face): FaceHit| SheetPt { q, p, fnorm, lift: fnorm, face };
    let frame = |p: Vec3| Vec2::new((p - mid).dot(u), (p - mid).dot(v));
    let corner = |i: usize, j: usize| Vec2::new(x(i as i32 - half), x(j as i32 - half));
    // where a cell edge leaves the lens: bisect between its inside and outside corner, keeping
    // the last point that hits (always taken as inside → outside, so the two cells sharing the
    // edge get the same point and the sheet has no crack)
    let edge_hit = |inside: Vec2, mut hit_in: FaceHit, outside: Vec2| -> SheetPt {
        let (mut lo, mut hi) = (0.0f32, 1.0f32);
        for _ in 0..EDGE_BISECT {
            let m = (lo + hi) / 2.0;
            let q = inside.lerp(outside, m);
            match at(q.x, q.y) {
                Some(h) => (lo, hit_in) = (m, h),
                None => hi = m,
            }
        }
        floor_pt(inside.lerp(outside, lo), hit_in)
    };
    // the fold between the floor's faces under `a` and `b`, if they share an edge: the two
    // corners they have in common farthest apart, in the frame and on the floor (the faces in
    // index order, so every cell on it gets the same line)
    let seam = |fa: usize, fb: usize| -> Option<(Vec2, Vec2, Vec3, Vec3)> {
        let (fa, fb) = (fa.min(fb), fa.max(fb));
        let common: Vec<Vec3> = pane[fa].pts.iter().map(|&p| Vec3::from(p)).filter(|p| pane[fb].pts.iter().any(|&r| p.distance(Vec3::from(r)) < 1e-5)).collect();
        let mut ends: Option<(f32, Vec3, Vec3)> = None;
        for (k, &p) in common.iter().enumerate() {
            for &r in &common[k + 1..] {
                if ends.is_none_or(|e| p.distance(r) > e.0) {
                    ends = Some((p.distance(r), p, r));
                }
            }
        }
        let (len, p0, p1) = ends.filter(|e| e.0 > 1e-4)?;
        let (q0, q1) = (frame(p0), frame(p1));
        // an edge along the ray (end-on in the frame) gives no line
        (q0.distance(q1) > len * 1e-3).then_some((q0, q1, p0, p1))
    };
    // `poly` cut along that fold: the pieces either side, each with the crossings on it
    let fold = |poly: &[SheetPt], a: SheetPt, b: SheetPt| -> Option<(Vec<SheetPt>, Vec<SheetPt>)> {
        let (na, nb) = (a.fnorm, b.fnorm);
        if na.dot(nb) < -0.9 {
            return None;
        }
        let (q0, q1, p0, p1) = seam(a.face, b.face)?;
        let dq = q1 - q0;
        let side = |q: Vec2| dq.perp_dot(q - q0);
        let (da, db) = (side(a.q), side(b.q));
        if da * db >= 0.0 {
            return None;
        }
        // the mitre: m · na = m · nb = 1, so a point on the fold lifted by h along it is h off
        // both faces' planes
        let mitre = (na + nb) / (1.0 + na.dot(nb));
        let (mut pa, mut pb) = (Vec::with_capacity(8), Vec::with_capacity(8));
        for k in 0..poly.len() {
            let (s, e) = (poly[k], poly[(k + 1) % poly.len()]);
            let (gs, ge) = (side(s.q) * da.signum(), side(e.q) * da.signum());
            if gs >= 0.0 {
                pa.push(s);
            }
            if gs <= 0.0 {
                pb.push(s);
            }
            if gs * ge < 0.0 {
                // taken in one direction along the edge, so the cell on its other side gets the
                // same point
                let (from, to, g0, g1) = if (s.q.x, s.q.y) < (e.q.x, e.q.y) { (s.q, e.q, gs, ge) } else { (e.q, s.q, ge, gs) };
                let q = from.lerp(to, g0 / (g0 - g1));
                let p = p0.lerp(p1, (q - q0).dot(dq) / dq.length_squared());
                pa.push(SheetPt { q, p, fnorm: na, lift: mitre, face: a.face });
                pb.push(SheetPt { q, p, fnorm: nb, lift: mitre, face: b.face });
            }
        }
        (pa.len() >= 3 && pb.len() >= 3).then_some((pa, pb))
    };
    let mut fan = |poly: &[SheetPt]| {
        let first = point_at(poly[0]);
        let mut prev = point_at(poly[1]);
        for &s in &poly[2..] {
            let next = point_at(s);
            tri(out, [first, prev, next]);
            prev = next;
        }
    };
    // two points of `poly` either side of a fold
    let folded = |a: &SheetPt, b: &SheetPt| a.fnorm.dot(b.fnorm) < FOLD_COS;
    let cut = |poly: &[SheetPt]| {
        let pairs = poly.iter().enumerate().flat_map(|(k, a)| poly[k + 1..].iter().map(move |b| (a, b)));
        pairs.filter(|(a, b)| folded(a, b)).find_map(|(a, b)| fold(poly, *a, *b))
    };
    // each cell clipped to the lens's outline (marching squares): its corners in order with, on
    // each edge whose ends differ, the crossing; a whole cell gives the two triangles of its quad.
    // A cell across a fold is cut along it, piece by piece.
    let n_grid = grid.len();
    let mut poly: Vec<SheetPt> = Vec::with_capacity(8);
    let mut pieces: Vec<(Vec<SheetPt>, usize)> = Vec::new();
    for i in 0..n_grid - 1 {
        for j in 0..n_grid - 1 {
            let cs = [(i, j), (i + 1, j), (i + 1, j + 1), (i, j + 1)];
            if cs.iter().all(|&(a, b)| grid[a][b].is_none()) {
                continue;
            }
            poly.clear();
            for k in 0..4 {
                let ((a0, b0), (a1, b1)) = (cs[k], cs[(k + 1) % 4]);
                let (h0, h1) = (grid[a0][b0], grid[a1][b1]);
                if let Some(h) = h0 {
                    poly.push(floor_pt(corner(a0, b0), h));
                }
                match (h0, h1) {
                    (Some(h), None) => poly.push(edge_hit(corner(a0, b0), h, corner(a1, b1))),
                    (None, Some(h)) => poly.push(edge_hit(corner(a1, b1), h, corner(a0, b0))),
                    _ => {}
                }
            }
            if poly.len() < 3 {
                continue;
            }
            if !poly.iter().any(|s| folded(s, &poly[0])) {
                fan(&poly);
                continue;
            }
            pieces.push((poly.clone(), 0));
            while let Some((piece, cuts)) = pieces.pop() {
                match (cuts < FOLD_CUTS).then(|| cut(&piece)).flatten() {
                    Some((pa, pb)) => pieces.extend([(pa, cuts + 1), (pb, cuts + 1)]),
                    None => fan(&piece),
                }
            }
        }
    }
    // the bulbs: a glowing dome on the floor at each focus that lies on the lens
    for c in &f2 {
        let Some((p, fnorm, _)) = at(c.x, c.y) else { continue };
        let e = |k: usize| {
            let ang = k as f32 / DOME_SEGMENTS as f32 * std::f32::consts::TAU;
            u * ang.cos() + v * ang.sin()
        };
        let ring = |r: usize, e: Vec3| {
            let t = r as f32 / DOME_RINGS as f32 * std::f32::consts::FRAC_PI_2;
            let dir = (e * t.cos() + fnorm * t.sin()).normalize();
            (p + fnorm * LIFT + dir * bulb, dir)
        };
        for k in 0..DOME_SEGMENTS {
            let (e0, e1) = (e(k), e(k + 1));
            for r in 0..DOME_RINGS {
                let g = |q: (Vec3, Vec3)| vert(q.0, q.1, glow, LAMP_ENV);
                let [q0, q1, q2, q3] = [ring(r, e0), ring(r, e1), ring(r + 1, e1), ring(r + 1, e0)];
                tri(out, [g(q0), g(q1), g(q2)]);
                if r + 1 < DOME_RINGS {
                    tri(out, [g(q0), g(q2), g(q3)]);
                }
            }
        }
    }
}

/// A point of the reflector bowl's sheet (`bowl`): its frame position `q`, its floor point `p`,
/// the floor normal `fnorm` it is shaded with and the direction `lift` the height field raises
/// it along (the floor's normal; on a fold the mitre of the faces either side).
#[derive(Clone, Copy)]
struct SheetPt {
    q: Vec2,
    p: Vec3,
    fnorm: Vec3,
    lift: Vec3,
    /// the index of its floor face in the lens's faces
    face: usize,
}

/// The face's area.
fn area(f: &Face) -> f32 {
    let p0 = Vec3::from(f.pts[0]);
    (1..f.pts.len().saturating_sub(1)).map(|k| (Vec3::from(f.pts[k]) - p0).cross(Vec3::from(f.pts[k + 1]) - p0).length() / 2.0).sum()
}

/// The `LampKind::Leds` tail light on the lens `pane`, laid out in a frame on its main facet
/// (the largest face and those within about 30° of it): u across the car towards its middle, v
/// up the facet, from the halo's midpoint `mid`. Straight lines fitted to the outline's inner
/// and bottom edges give the layout's frame; `inset` is in facet widths like the paths.
#[allow(clippy::too_many_arguments)]
fn cluster(out: &mut Vec<Vertex>, strips: &mut Vec<LampStrip>, halo: usize, pane: &[&Face], mid: Vec3, glow: [f32; 4], paths: &[LedPath], inset: f32) {
    let Some(big) = pane.iter().max_by(|x, y| area(x).total_cmp(&area(y))) else { return };
    // REMASTER: a lens faired by a `Smooth` zone (the Serpent's tail) is dozens of small
    // triangles, and the biggest may lie at its edge, where the fairing tilts it towards the body
    // round it: the main facet is then taken round the area-weighted mean normal. A lens of a few
    // cage facets keeps its biggest one (the Titan's six-face cluster), as before.
    let nm = if pane.len() > FAIRED_PANE {
        pane.iter().map(|f| Vec3::from(f.normal).normalize_or_zero() * area(f)).sum::<Vec3>().normalize_or(Vec3::Y)
    } else {
        Vec3::from(big.normal).normalize_or(Vec3::Y)
    };
    let main: Vec<&Face> = pane.iter().copied().filter(|f| Vec3::from(f.normal).normalize_or_zero().dot(nm) > 0.85).collect();
    let su = Vec3::new(-mid.x.signum(), 0.0, 0.0).reject_from(nm).normalize_or_zero();
    let mut sv = nm.cross(su);
    if sv.y < 0.0 {
        sv = -sv;
    }
    let at = |u: f32, v: f32| hit(&main, mid + su * u + sv * v, nm).map(|(p, n)| (p + n * LIFT, n));
    // the outline on a SCAN grid
    let x = |i: usize| (i as f32 - SCAN_N as f32 / 2.0) * SCAN;
    let inside: Vec<Vec<bool>> = (0..=SCAN_N).map(|i| (0..=SCAN_N).map(|j| at(x(i), x(j)).is_some()).collect()).collect();
    let bottom: Vec<(f32, f32)> = (0..=SCAN_N).filter_map(|i| inside[i].iter().position(|&b| b).map(|j| (x(i), x(j)))).collect();
    let (Some(&(u0, _)), Some(&(u1, _))) = (bottom.first(), bottom.last()) else { return };
    let w = u1 - u0;
    let inset = inset * w;
    // bottom edge v = a + b u over BOTTOM_FIT; inner edge u = c + d v over the middle of its
    // height there
    let fit_b: Vec<(f32, f32)> = bottom.iter().copied().filter(|c| c.0 <= u1 - BOTTOM_FIT.0 * w && c.0 >= u1 - BOTTOM_FIT.1 * w).collect();
    let Some((a, b)) = line_fit(&fit_b) else { return };
    let ut = u1 - BOTTOM_FIT.0 * w;
    let it = ((ut / SCAN + SCAN_N as f32 / 2.0).round() as usize).min(SCAN_N);
    let Some(top) = (0..=SCAN_N).rev().find(|&j| inside[it][j]).map(x) else { return };
    let vb = a + b * ut;
    let (lo, hi) = (vb + (top - vb) * 0.2, top - (top - vb) * 0.2);
    let fit_i: Vec<(f32, f32)> = (0..=SCAN_N)
        .filter(|&j| x(j) >= lo && x(j) <= hi)
        .filter_map(|j| (0..=SCAN_N).rev().find(|&i| inside[i][j]).map(|i| (x(j), x(i))))
        .collect();
    let Some((c, d)) = line_fit(&fit_i) else { return };
    // the two edges moved `inset` inwards meet at the origin
    let (kb, ki) = (inset * (1.0 + b * b).sqrt(), inset * (1.0 + d * d).sqrt());
    let ou = (c + d * (a + kb) - ki) / (1.0 - d * b);
    let o = Vec2::new(ou, a + b * ou + kb);
    let (ex, ey) = (Vec2::new(-1.0, -b).normalize(), Vec2::new(d, 1.0).normalize());
    let place = |p: [f32; 2]| o + (ex * p[0] + ey * p[1]) * w;
    let mut unlit = Vec::new();
    let mut no_halo = Vec::new();
    for path in paths {
        let strips: &mut Vec<LampStrip> = if path.halo { &mut *strips } else { &mut no_halo };
        let copies = path.repeat.max(1);
        let frame = |r: f32| -> Vec<Vec2> { path.pts.iter().map(|p| place([p[0] + path.step[0] * r, p[1] + path.step[1] * r])).collect() };
        // the lens shrunk by the path's clip
        let cw = path.clip * w;
        let at_clip = |u: f32, v: f32| at(u, v).filter(|_| cw <= 0.0 || [(cw, 0.0), (-cw, 0.0), (0.0, cw), (0.0, -cw)].iter().all(|&(du, dv)| at(u + du, v + dv).is_some()));
        let at = &at_clip;
        match path.style {
            LedStyle::Dots { dot, pitch } => {
                let size = [dot * w, pitch * w, path.margin * w];
                // copies of a pattern (rows) share one strip along their middle, as wide as
                // the pattern: a strip a row would cross its neighbours' at the bends
                let mut own = Vec::new();
                for r in 0..copies {
                    dots(out, if copies == 1 { strips } else { &mut own }, halo, at, &frame(r as f32), size, size[0], glow);
                }
                if copies > 1 {
                    let half = (copies - 1) as f32 / 2.0;
                    let spread = Vec2::from(path.step).length() * half * w;
                    dots(&mut unlit, strips, halo, at, &frame(half), size, size[0] + spread, glow);
                }
            }
            LedStyle::Bar { width } => {
                for r in 0..copies {
                    bar(out, strips, halo, at, &frame(r as f32), width * w / 2.0, glow);
                }
            }
            LedStyle::Rim { width, gap, closed } => {
                // the lens's convex hull (its outline's per-column lowest and highest points;
                // the staircase of a sloped edge is simplified away), run clockwise in (u, v)
                // from the corner nearest the inner edge's line u = c + d v, then moved `gap`
                // inwards (mitred corners); the inner end closes the polygon
                let tops: Vec<(f32, f32)> = (0..=SCAN_N).filter_map(|i| inside[i].iter().rposition(|&b| b).map(|j| (x(i), x(j)))).collect();
                let mut ring = hull(&bottom.iter().chain(tops.iter()).map(|c| Vec2::new(c.0, c.1)).collect::<Vec<_>>());
                ring.reverse();
                let start = (0..ring.len()).max_by(|&i, &j| (ring[i].x - d * ring[i].y).total_cmp(&(ring[j].x - d * ring[j].y))).unwrap_or(0);
                ring.rotate_left(start);
                let ring = simplify(&ring, SCAN * 1.2);
                let n = ring.len();
                let mut ring: Vec<Vec2> = (0..n)
                    .map(|i| {
                        let (prev, v, next) = (ring[(i + n - 1) % n], ring[i], ring[(i + 1) % n]);
                        // the ring runs clockwise in (u, v): inwards is to the right of each edge
                        let inward = |t: Vec2| Vec2::new(t.y, -t.x);
                        let (n1, n2) = (inward((v - prev).normalize_or_zero()), inward((next - v).normalize_or_zero()));
                        let dd = n1.dot(n2);
                        let off = if dd < -0.95 { n1 } else { (n1 + n2) / (1.0 + dd) };
                        v + (off * gap * w).clamp_length_max(4.0 * gap * w)
                    })
                    .collect();
                if closed {
                    if let Some(&first) = ring.first() {
                        ring.push(first);
                    }
                    bar(out, strips, halo, at, &ring, width * w / 2.0, glow);
                } else {
                    // open: the ring cut `gap + width` short of the inner edge's line u = c + d v
                    // (the lens lies at u below it), its pieces drawn one by one
                    let clear = (gap + width) * w;
                    let g = |p: Vec2| (c + d * p.y - p.x) / (1.0 + d * d).sqrt();
                    let cut = |p: Vec2, q: Vec2| p + (q - p) * ((g(p) - clear) / (g(p) - g(q)));
                    let mut run: Vec<Vec2> = Vec::new();
                    for s in ring.windows(2) {
                        let (p, q) = (s[0], s[1]);
                        match (g(p) >= clear, g(q) >= clear) {
                            (true, true) => {
                                if run.is_empty() {
                                    run.push(p);
                                }
                                run.push(q);
                            }
                            (true, false) => {
                                if run.is_empty() {
                                    run.push(p);
                                }
                                run.push(cut(p, q));
                                bar(out, strips, halo, at, &run, width * w / 2.0, glow);
                                run.clear();
                            }
                            (false, true) => run.extend([cut(q, p), q]),
                            (false, false) => {}
                        }
                    }
                    if run.len() > 1 {
                        bar(out, strips, halo, at, &run, width * w / 2.0, glow);
                    }
                }
            }
        }
    }
}

/// The convex hull of `pts` (Andrew's monotone chain), counter-clockwise, no repeated point.
fn hull(pts: &[Vec2]) -> Vec<Vec2> {
    let mut p = pts.to_vec();
    p.sort_by(|a, b| a.x.total_cmp(&b.x).then(a.y.total_cmp(&b.y)));
    p.dedup();
    if p.len() < 3 {
        return p;
    }
    let mut h: Vec<Vec2> = Vec::new();
    for _ in 0..2 {
        let start = h.len();
        for &q in &p {
            while h.len() >= start + 2 && (h[h.len() - 1] - h[h.len() - 2]).perp_dot(q - h[h.len() - 2]) <= 0.0 {
                h.pop();
            }
            h.push(q);
        }
        h.pop();
        p.reverse();
    }
    h
}

/// Douglas–Peucker: the polyline without the points within `tol` of the line between the
/// kept ones round them.
fn simplify(pts: &[Vec2], tol: f32) -> Vec<Vec2> {
    if pts.len() < 3 {
        return pts.to_vec();
    }
    let (a, b) = (pts[0], pts[pts.len() - 1]);
    let dist = |p: Vec2| {
        let ab = b - a;
        let t = ((p - a).dot(ab) / ab.length_squared().max(1e-12)).clamp(0.0, 1.0);
        p.distance(a + ab * t)
    };
    let (k, dk) = (1..pts.len() - 1).map(|k| (k, dist(pts[k]))).fold((0, 0.0), |m, c| if c.1 > m.1 { c } else { m });
    if dk <= tol {
        return vec![a, b];
    }
    let mut left = simplify(&pts[..=k], tol);
    left.pop();
    left.extend(simplify(&pts[k..], tol));
    left
}

/// The dots of one polyline (`size` = radius, pitch, margin in m); each run of lit dots gives
/// strips of half-width `strip_width`.
#[allow(clippy::too_many_arguments)]
fn dots(out: &mut Vec<Vertex>, strips: &mut Vec<LampStrip>, halo: usize, at: &dyn Fn(f32, f32) -> Option<Hit>, pts: &[Vec2], size: [f32; 3], strip_width: f32, glow: [f32; 4]) {
    let [dot, pitch, margin] = size;
    let mut run: Vec<Vec3> = Vec::new();
    for (k, s) in pts.windows(2).enumerate() {
        let (p, q) = (s[0], s[1]);
        let n = (p.distance(q) / pitch).round().max(1.0) as usize;
        let ahead = (q - p).normalize_or_zero() * margin;
        // a segment's first dot is the last of the one before
        for m in (if k == 0 { 0 } else { 1 })..=n {
            let c = p.lerp(q, m as f32 / n as f32);
            let fits = margin == 0.0 || at(c.x + ahead.x, c.y + ahead.y).is_some();
            match fits.then(|| disc(out, at, (c.x, c.y), dot, glow)).flatten() {
                Some(on_lens) => run.push(on_lens),
                None => strips_of(strips, halo, &mut run, strip_width, pitch / 2.0),
            }
        }
    }
    strips_of(strips, halo, &mut run, strip_width, pitch / 2.0);
}

/// A bar of half-width `hw` along a polyline: a ribbon of `BAR_STEP` pieces on the lens.
fn bar(out: &mut Vec<Vertex>, strips: &mut Vec<LampStrip>, halo: usize, at: &dyn Fn(f32, f32) -> Option<Hit>, pts: &[Vec2], hw: f32, glow: [f32; 4]) {
    let mut samples: Vec<Vec2> = Vec::new();
    for (k, s) in pts.windows(2).enumerate() {
        let n = (s[0].distance(s[1]) / BAR_STEP).ceil().max(1.0) as usize;
        samples.extend(((if k == 0 { 0 } else { 1 })..=n).map(|m| s[0].lerp(s[1], m as f32 / n as f32)));
    }
    // each sample's across-direction bisects its two segments (mitred corners)
    let dir = |i: usize| (samples[(i + 1).min(samples.len() - 1)] - samples[i.saturating_sub(1)]).normalize_or_zero();
    let side = |i: usize| Vec2::new(-dir(i).y, dir(i).x) * hw;
    let g = |p: Hit| vert(p.0, p.1, glow, LAMP_ENV);
    let mut run: Vec<Vec3> = Vec::new();
    for i in 0..samples.len() {
        let (c, sd) = (samples[i], side(i));
        let edge = |sign: f32| at(c.x + sd.x * sign, c.y + sd.y * sign);
        let (Some(mid), Some(l), Some(r)) = (at(c.x, c.y), edge(-1.0), edge(1.0)) else {
            strips_of(strips, halo, &mut run, hw, 0.0);
            continue;
        };
        if !run.is_empty() {
            // the piece from the last sample to this one
            let (pc, ps) = (samples[i - 1], side(i - 1));
            if let (Some(pl), Some(pr)) = (at(pc.x - ps.x, pc.y - ps.y), at(pc.x + ps.x, pc.y + ps.y)) {
                tri(out, [g(pl), g(l), g(r)]);
                tri(out, [g(pl), g(r), g(pr)]);
            }
        }
        run.push(mid.0);
    }
    strips_of(strips, halo, &mut run, hw, 0.0);
}

/// Strips of half-width `width` along a run of lit points (consecutive ones, on the lens), as
/// few as the run's bends need, then clears it. Ends are flat: strips meet end to end so their
/// glow adds up evenly, and the run's two ends reach `ext` (m) past the first and last point,
/// the share of the end LED. A single point is a strip of no length.
fn strips_of(strips: &mut Vec<LampStrip>, halo: usize, run: &mut Vec<Vec3>, width: f32, ext: f32) {
    let tol = width * 0.25;
    let first = strips.len();
    if run.len() == 1 {
        strips.push(LampStrip { halo, p0: run[0].to_array(), p1: run[0].to_array(), width, join: [false; 2] });
    }
    let mut s = 0;
    while s + 1 < run.len() {
        let mut e = s + 1;
        while e + 1 < run.len() && run[s + 1..=e + 1].iter().all(|&p| seg_dist(p, run[s], run[e + 1]) < tol) {
            e += 1;
        }
        strips.push(LampStrip { halo, p0: run[s].to_array(), p1: run[e].to_array(), width, join: [s > 0, false] });
        s = e;
    }
    let n = strips.len();
    strips[first..n.saturating_sub(1).max(first)].iter_mut().for_each(|l| l.join[1] = true);
    if let [.., last] = &mut strips[first..] {
        let (p0, p1) = (Vec3::from(last.p0), Vec3::from(last.p1));
        last.p1 = (p1 + (p1 - p0).normalize_or_zero() * ext).to_array();
    }
    if let Some(f) = strips.get_mut(first) {
        let (p0, p1) = (Vec3::from(f.p0), Vec3::from(f.p1));
        f.p0 = (p0 - (p1 - p0).normalize_or_zero() * ext).to_array();
    }
    run.clear();
}

/// Least squares y = a + b x through `pts` (x, y).
fn line_fit(pts: &[(f32, f32)]) -> Option<(f32, f32)> {
    let n = pts.len() as f32;
    let (sx, sy) = pts.iter().fold((0.0, 0.0), |s, p| (s.0 + p.0, s.1 + p.1));
    let (mx, my) = (sx / n, sy / n);
    let (sxx, sxy) = pts.iter().fold((0.0, 0.0), |s, p| (s.0 + (p.0 - mx).powi(2), s.1 + (p.0 - mx) * (p.1 - my)));
    (pts.len() >= 2 && sxx > 1e-9).then(|| (my - sxy / sxx * mx, sxy / sxx))
}

/// A glowing disc of radius `r` at `c` in a cluster's (u, v) frame, kept only if all of it lands
/// on the lens (`at`): its centre on the lens, or None.
fn disc(out: &mut Vec<Vertex>, at: &dyn Fn(f32, f32) -> Option<Hit>, c: (f32, f32), r: f32, glow: [f32; 4]) -> Option<Vec3> {
    let m = at(c.0, c.1)?;
    let rim: Vec<Hit> = (0..=DOT_SEGMENTS)
        .map(|k| {
            let a = k as f32 / DOT_SEGMENTS as f32 * std::f32::consts::TAU;
            at(c.0 + r * a.cos(), c.1 + r * a.sin())
        })
        .collect::<Option<_>>()?;
    let g = |p: Hit| vert(p.0, p.1, glow, LAMP_ENV);
    for s in rim.windows(2) {
        tri(out, [g(m), g(s[0]), g(s[1])]);
    }
    Some(m.0)
}

/// a point on the lens and its normal
type Hit = (Vec3, Vec3);
/// a `Hit` and the index of the face it met
type FaceHit = (Vec3, Vec3, usize);

#[cfg(test)]
mod tests {
    use super::*;

    /// a projector pod's triangles: the barrel, the face's bands, the lens's rings and its apex fan
    const UNIT_TRIS: usize = UNIT_SEGMENTS * (2 + (FACE_PROFILE.len() - 1) * 2 + (LENS_RINGS - 1) * 2 + 1);
    /// those of its LED ring
    const LIT_TRIS: usize = UNIT_SEGMENTS * 2;
    /// the lens's apex on a lens facing along `AIM` (`depth` deep): `CLEAR` under the surface
    fn apex(depth: f32) -> f32 {
        depth - CLEAR
    }

    /// on a lens sloping up the bonnet (normal mostly up, like the Serpent's), the pods still
    /// point along the car: their LED rings face +z; nothing rises past the lens's surface
    #[test]
    fn pods_point_along_the_car() {
        let rgb = [127, 127, 47];
        let n = Vec3::new(0.0, 0.87, 0.49).normalize();
        let t = n.cross(Vec3::X) * 0.2;
        let pts = [Vec3::ZERO, Vec3::X * 0.4, Vec3::X * 0.4 + t, t].map(|p| p.to_array()).to_vec();
        let faces = vec![Face { pts, normal: n.to_array(), ..quad([[0.0; 3]; 4], rgb) }];
        let mid = t / 2.0;
        let halos = [HaloSeg { p0: mid.to_array(), p1: (mid + Vec3::X * 0.4).to_array(), rgb: [251, 254, 131] }];
        let depth = 0.09;
        let housing = Glass { rgb, frame: 0.0, inset: depth, tint: 0.5 };
        let lamp = Lamp { housing, kind: LampKind::Projectors { count: 2, radius: 0.035, rows: 1, span: 0.6, floor: [40; 3], paths: Vec::new(), inset: 0.0 } };
        let (v, _) = lamps(&faces, &halos, std::slice::from_ref(&lamp));
        assert_eq!(v.len(), 2 * UNIT_TRIS * 3);
        let lit: Vec<&Vertex> = v.iter().filter(|p| p.env == LAMP_ENV).collect();
        assert_eq!(lit.len(), 2 * LIT_TRIS * 3);
        // view space is game space with z negated: +z (forward) is -z there
        assert!(lit.iter().all(|p| p.normal[2] < -0.999), "{:?}", lit[0].normal);
        let game = |p: &Vertex| Vec3::new(p.pos[0], p.pos[1], -p.pos[2]);
        let top = v.iter().map(|p| game(p).dot(n)).fold(f32::MIN, f32::max);
        assert!(top <= depth - CLEAR + 1e-4, "{top}");
        // the faces stand above the floor: the LED rings' lowest point
        assert!(lit.iter().map(|p| game(p).dot(n)).fold(f32::MAX, f32::min) > 0.0);
    }

    /// a span of 0.5 puts two units at a quarter and three quarters of the middle half; the
    /// housing floor loses its texture and takes the floor colour, glossy
    #[test]
    fn projectors_span_and_floor() {
        let rgb = [127, 127, 47];
        let mut faces = vec![quad([[0.0, 0.0, 0.0], [0.4, 0.0, 0.0], [0.4, 0.1, 0.0], [0.0, 0.1, 0.0]], rgb)];
        faces[0].layer = 3.0;
        let halos = [HaloSeg { p0: [0.0, 0.05, 0.0], p1: [0.4, 0.05, 0.0], rgb: [251, 254, 131] }];
        let housing = Glass { rgb, frame: 0.0, inset: 0.03, tint: 0.5 };
        let lamp = Lamp { housing, kind: LampKind::Projectors { count: 2, radius: 0.02, rows: 1, span: 0.5, floor: [102, 102, 102], paths: Vec::new(), inset: 0.0 } };
        let (v, _) = lamps(&faces, &halos, std::slice::from_ref(&lamp));
        let top = apex(0.03);
        let mut xs: Vec<f32> = v.iter().filter(|p| (-p.pos[2] - top).abs() < 1e-5).map(|p| (p.pos[0] * 1000.0).round() / 1000.0).collect();
        xs.dedup();
        assert_eq!(xs, vec![0.15, 0.25]);
        housing_floor(&mut faces, &lamp);
        assert!(faces[0].layer < 0.0 && faces[0].env == GLOSS_ENV && faces[0].colour.iter().all(|c| (c[0] - 0.4).abs() < 1e-6));
    }

    fn quad(pts: [[f32; 3]; 4], rgb: [u8; 3]) -> Face {
        Face { pts: pts.to_vec(), normal: [0.0, 0.0, 1.0], colour: vec![[0.5; 4]; 4], uv: vec![[0.0; 2]; 4], layer: -1.0, env: 0.0, rgb, smooth_deg: 0.0, remesh: 0.0 }
    }

    /// pods stand on a flat lens facing +z, inside its outline and under its surface; the LED cluster
    /// stays on it, every dot whole
    #[test]
    fn insides_on_the_lens() {
        let rgb = [127, 0, 1];
        let faces = vec![quad([[0.0, 0.0, 0.0], [0.4, 0.0, 0.0], [0.4, 0.1, 0.0], [0.0, 0.1, 0.0]], rgb)];
        let halos = [HaloSeg { p0: [0.0, 0.05, 0.0], p1: [0.4, 0.05, 0.0], rgb: [255, 38, 38] }];
        let housing = Glass { rgb, frame: 0.0, inset: 0.03, tint: 0.5 };
        let units = |count: usize, radius: f32, rows: usize, paths: Vec<LedPath>| LampKind::Projectors { count, radius, rows, span: 1.0, floor: [70; 3], paths, inset: 0.03 };
        let (cups, no_strips) = lamps(&faces, &halos, &[Lamp { housing, kind: units(3, 0.03, 1, Vec::new()) }]);
        assert!(no_strips.is_empty());
        assert_eq!(cups.len(), 3 * UNIT_TRIS * 3);
        for p in cups.iter().map(|v| Vec3::new(v.pos[0], v.pos[1], -v.pos[2])) {
            assert!(p.x > 0.0 && p.x < 0.4 && p.y > 0.0 && p.y < 0.1 && p.z >= -2.0 * LIFT - 1e-5 && p.z <= apex(0.03) + 1e-5, "{p}");
        }
        assert_eq!(cups.iter().filter(|v| v.env == LAMP_ENV).count(), 3 * LIT_TRIS * 3);
        let paths = vec![
            LedPath { pts: vec![[0.0, 0.0], [0.5, 0.0]], repeat: 2, step: [0.0, 0.05], style: LedStyle::Dots { dot: 0.015, pitch: 0.05 }, margin: 0.0, clip: 0.0, halo: true },
            LedPath { pts: vec![[0.0, 0.1], [0.0, 2.0]], repeat: 1, step: [0.0, 0.0], style: LedStyle::Dots { dot: 0.015, pitch: 0.05 }, margin: 0.04, clip: 0.0, halo: true },
        ];
        let tail = Lamp { housing, kind: LampKind::Leds { paths, inset: 0.04 } };
        let (lights, strips) = lamps(&faces, &halos, std::slice::from_ref(&tail));
        // one strip along the middle of the two rows (as wide as both) and one for the column, both
        // following the one halo
        assert_eq!(strips.len(), 2);
        assert!(strips.iter().all(|s| s.halo == 0));
        assert!(strips[0].width > strips[1].width);
        assert!(strips.iter().all(|s| [s.p0, s.p1].iter().all(|p| p[0] >= 0.0 && p[0] <= 0.4 && p[1] >= 0.0 && p[1] <= 0.1)));
        assert!(lights.iter().all(|v| (v.pos[2] + LIFT).abs() < 1e-5 && v.pos[0] >= -1e-5 && v.pos[0] <= 0.4 + 1e-5 && v.pos[1] >= -1e-5 && v.pos[1] <= 0.1 + 1e-5));
        // two rows of 11 dots and 2 more up the column, of DOT_SEGMENTS triangles
        assert_eq!(lights.len(), 24 * DOT_SEGMENTS * 3);
        assert!(lights.iter().all(|v| v.env == LAMP_ENV && v.color == [1.0, 38.0 / 255.0, 38.0 / 255.0, 1.0]));
        // a halo away from the lamp gets nothing
        let far = [HaloSeg { p0: [0.0, 1.0, 0.0], p1: [0.4, 1.0, 0.0], rgb: [255; 3] }];
        let (nothing, no_strips) = lamps(&faces, &far, std::slice::from_ref(&tail));
        assert!(nothing.is_empty() && no_strips.is_empty());
    }

    /// a light tube round a trapezoid lens: closed, it hugs the whole outline `gap` in; open,
    /// it stays clear of the inner end (x 0, the end nearer the car's middle)
    #[test]
    fn rim_round_the_lens() {
        let rgb = [127, 8, 8];
        let faces = vec![quad([[0.0, 0.0, 0.0], [0.4, 0.03, 0.0], [0.4, 0.1, 0.0], [0.0, 0.13, 0.0]], rgb)];
        let halos = [HaloSeg { p0: [0.0, 0.06, 0.0], p1: [0.4, 0.06, 0.0], rgb: [255, 38, 38] }];
        let housing = Glass { rgb, frame: 0.0, inset: 0.03, tint: 0.5 };
        let rim = |closed: bool, halo: bool| LedPath { pts: Vec::new(), repeat: 1, step: [0.0; 2], style: LedStyle::Rim { width: 0.03, gap: 0.03, closed }, margin: 0.0, clip: 0.0, halo };
        let (lit, strips) = lamps(&faces, &halos, &[Lamp { housing, kind: LampKind::Leds { paths: vec![rim(true, true)], inset: 0.0 } }]);
        let xs: Vec<f32> = lit.iter().map(|v| v.pos[0]).collect();
        let (x0, x1) = (xs.iter().cloned().fold(f32::MAX, f32::min), xs.iter().cloned().fold(f32::MIN, f32::max));
        assert!(x0 < 0.012 && x0 > 0.0 && x1 > 0.388 && x1 < 0.4, "{x0} {x1}");
        assert!(lit.iter().all(|v| v.pos[1] > 0.0 && v.pos[1] < 0.13));
        assert!(strips.len() >= 4, "{}", strips.len());
        // open: nothing within the gap plus the width of the inner end, and no strips without a halo
        let (lit, strips) = lamps(&faces, &halos, &[Lamp { housing, kind: LampKind::Leds { paths: vec![rim(false, false)], inset: 0.0 } }]);
        let x0 = lit.iter().map(|v| v.pos[0]).fold(f32::MAX, f32::min);
        assert!(x0 > 0.02 && x0 < 0.04, "{x0}");
        assert!(lit.iter().any(|v| v.pos[0] > 0.388));
        assert!(strips.is_empty());
    }

    /// two rows of three pods: six lenses, three per height, all on the lens; with a bar path the
    /// daytime bar comes too, with its strip
    #[test]
    fn rows_of_cups() {
        let rgb = [127, 122, 61];
        let faces = vec![quad([[0.0, 0.0, 0.0], [0.4, 0.0, 0.0], [0.4, 0.1, 0.0], [0.0, 0.1, 0.0]], rgb)];
        let halos = [HaloSeg { p0: [0.0, 0.05, 0.0], p1: [0.4, 0.05, 0.0], rgb: [247, 249, 151] }];
        let housing = Glass { rgb, frame: 0.0, inset: 0.03, tint: 0.5 };
        let matrix = |paths: Vec<LedPath>| Lamp { housing, kind: LampKind::Projectors { count: 3, radius: 0.012, rows: 2, span: 1.0, floor: [70; 3], paths, inset: 0.03 } };
        let (cups, strips) = lamps(&faces, &halos, &[matrix(Vec::new())]);
        assert!(strips.is_empty());
        assert_eq!(cups.len(), 6 * UNIT_TRIS * 3);
        // the lens tips (each dome's apex, UNIT_SEGMENTS copies) sit at two heights up the lens
        let top = apex(0.03);
        let mut ys: Vec<f32> = cups.iter().filter(|v| (-v.pos[2] - top).abs() < 1e-5).map(|v| (v.pos[1] * 1000.0).round() / 1000.0).collect();
        ys.sort_by(f32::total_cmp);
        ys.dedup();
        assert_eq!(ys.len(), 2, "{ys:?}");
        assert!((ys[1] - ys[0] - ROW_PITCH * 0.012).abs() < 2e-3, "{ys:?}");
        assert!(cups.iter().all(|v| v.pos[0] > 0.0 && v.pos[0] < 0.4 && v.pos[1] > 0.0 && v.pos[1] < 0.1));
        let bar = LedPath { pts: vec![[0.0, 0.0], [0.6, 0.0]], repeat: 1, step: [0.0; 2], style: LedStyle::Bar { width: 0.03 }, margin: 0.0, clip: 0.0, halo: true };
        let (lit, strips) = lamps(&faces, &halos, &[matrix(vec![bar])]);
        assert!(lit.len() > cups.len());
        assert_eq!(strips.len(), 1);
    }

    /// a reflector bowl covers the lens between the floor and the surface (depth), lowest at the
    /// bulbs; the domes glow at the foci
    #[test]
    fn bowl_over_the_lens() {
        let rgb = [127, 122, 61];
        let faces = vec![quad([[0.0, 0.0, 0.0], [0.4, 0.0, 0.0], [0.4, 0.1, 0.0], [0.0, 0.1, 0.0]], rgb)];
        let halos = [HaloSeg { p0: [0.0, 0.05, 0.0], p1: [0.4, 0.05, 0.0], rgb: [247, 249, 151] }];
        let housing = Glass { rgb, frame: 0.0, inset: 0.03, tint: 0.5 };
        let lamp = Lamp { housing, kind: LampKind::Reflector { count: 2, rows: 1, pitch: 0.0, bulb: 0.01, paths: Vec::new(), inset: 0.0 } };
        let (v, strips) = lamps(&faces, &halos, &[lamp]);
        assert!(strips.is_empty());
        let chrome: Vec<&Vertex> = v.iter().filter(|p| p.env == METAL_ENV).collect();
        assert!(chrome.len() > 1000);
        // inside the outline, between the floor and the surface (z up to the depth); the rim reaches it
        let zmax = chrome.iter().map(|p| -p.pos[2]).fold(f32::MIN, f32::max);
        assert!((zmax - 0.03 - LIFT).abs() < 1e-4, "{zmax}");
        assert!(chrome.iter().all(|p| p.pos[0] >= -1e-4 && p.pos[0] <= 0.4 + 1e-4 && p.pos[1] >= -1e-4 && p.pos[1] <= 0.1 + 1e-4 && -p.pos[2] >= LIFT - 1e-5));
        // at a focus (0.1, 0.05) the sheet touches the floor
        let at_focus = chrome.iter().filter(|p| (p.pos[0] - 0.1).abs() < 3e-3 && (p.pos[1] - 0.05).abs() < 3e-3).map(|p| -p.pos[2]).fold(f32::MAX, f32::min);
        assert!(at_focus < 0.003, "{at_focus}");
        let domes: Vec<&Vertex> = v.iter().filter(|p| p.env == LAMP_ENV).collect();
        assert_eq!(domes.len(), 2 * DOME_SEGMENTS * (2 * DOME_RINGS - 1) * 3);
        assert!(domes.iter().all(|p| -p.pos[2] <= 0.01 + LIFT + 1e-5));
    }

    /// on a lens with a slanted end the bowl reaches the outline (#357): it covers the whole lens
    /// (a staircase of whole grid cells left about 0.7 % bare) and nothing pokes past the slant
    #[test]
    fn bowl_edge_on_a_slanted_lens() {
        let rgb = [127, 122, 61];
        // the right end runs from (0.4, 0) to (0.363, 0.1): x = 0.4 - 0.37 y
        let faces = vec![quad([[0.0, 0.0, 0.0], [0.4, 0.0, 0.0], [0.363, 0.1, 0.0], [0.0, 0.1, 0.0]], rgb)];
        let halos = [HaloSeg { p0: [0.0, 0.05, 0.0], p1: [0.38, 0.05, 0.0], rgb: [247, 249, 151] }];
        let housing = Glass { rgb, frame: 0.0, inset: 0.03, tint: 0.5 };
        let lamp = Lamp { housing, kind: LampKind::Reflector { count: 2, rows: 1, pitch: 0.0, bulb: 0.01, paths: Vec::new(), inset: 0.0 } };
        let (v, _) = lamps(&faces, &halos, &[lamp]);
        let tris: Vec<&[Vertex]> = v.chunks(3).filter(|t| t[0].env == METAL_ENV).collect();
        let covered: f32 = tris
            .iter()
            .map(|t| {
                let (a, b, c) = (Vec2::from_slice(&t[0].pos), Vec2::from_slice(&t[1].pos), Vec2::from_slice(&t[2].pos));
                (b - a).perp_dot(c - a).abs() / 2.0
            })
            .sum();
        let lens = 0.1 * (0.4 + 0.363) / 2.0;
        assert!((covered / lens - 1.0).abs() < 1e-3, "{covered} of {lens}");
        assert!(tris.iter().flat_map(|t| t.iter()).all(|p| p.pos[0] <= 0.4 - 0.37 * p.pos[1] + 1e-5));
    }

    /// on a lens folding 30° round a corner (two faces sharing the edge x = 0.25) the bowl is cut
    /// along the fold (#359): no triangle reaches across the plane through the fold along the
    /// mitre of the two faces, and the triangles either side share their edges on it (no crack)
    #[test]
    fn bowl_folds_on_a_straight_line() {
        let rgb = [125, 126, 82];
        let (c, s) = (30f32.to_radians().cos(), 30f32.to_radians().sin());
        let mut a = quad([[0.0, 0.0, 0.0], [0.25, 0.0, 0.0], [0.25, 0.1, 0.0], [0.0, 0.1, 0.0]], rgb);
        a.normal = [0.0, 0.0, 1.0];
        let end = [0.25 + 0.15 * c, -0.15 * s];
        let mut b = quad([[0.25, 0.0, 0.0], [end[0], 0.0, end[1]], [end[0], 0.1, end[1]], [0.25, 0.1, 0.0]], rgb);
        b.normal = [s, 0.0, c];
        let halos = [HaloSeg { p0: [0.0, 0.05, 0.0], p1: [end[0], 0.05, end[1]], rgb: [252, 255, 134] }];
        let housing = Glass { rgb, frame: 0.0, inset: 0.03, tint: 0.5 };
        let lamp = Lamp { housing, kind: LampKind::Reflector { count: 2, rows: 1, pitch: 0.0, bulb: 0.01, paths: Vec::new(), inset: 0.0 } };
        let (v, _) = lamps(&[a, b], &halos, &[lamp]);
        let tris: Vec<&[Vertex]> = v.chunks(3).filter(|t| t[0].env == METAL_ENV).collect();
        // the mitre plane: through the fold (x = 0.25, z = 0) along y and (na + nb); in view
        // space z is negated
        let m = Vec3::new(s, 0.0, 1.0 + c);
        let k = Vec3::Y.cross(m).normalize();
        let side = |p: &Vertex| (Vec3::new(p.pos[0], p.pos[1], -p.pos[2]) - Vec3::new(0.25, 0.0, 0.0)).dot(k);
        assert!(tris.iter().all(|t| t.iter().all(|p| side(p) <= 1e-5) || t.iter().all(|p| side(p) >= -1e-5)));
        // every edge on the fold belongs to two triangles, one either side
        let key = |p: &Vertex| p.pos.map(f32::to_bits);
        let mut edges: std::collections::HashMap<([u32; 3], [u32; 3]), usize> = std::collections::HashMap::new();
        for t in &tris {
            for e in 0..3 {
                let (p, q) = (&t[e], &t[(e + 1) % 3]);
                if side(p).abs() < 1e-5 && side(q).abs() < 1e-5 && key(p) != key(q) {
                    *edges.entry(if key(p) < key(q) { (key(p), key(q)) } else { (key(q), key(p)) }).or_default() += 1;
                }
            }
        }
        assert!(edges.len() >= 10, "{}", edges.len());
        assert!(edges.values().all(|&n| n == 2), "{edges:?}");
    }

    /// a bar is a ribbon on the lens, whole pieces only, and gives one strip per straight run
    #[test]
    fn bar_on_the_lens() {
        let rgb = [127, 0, 1];
        let faces = vec![quad([[0.0, 0.0, 0.0], [0.4, 0.0, 0.0], [0.4, 0.1, 0.0], [0.0, 0.1, 0.0]], rgb)];
        let halos = [HaloSeg { p0: [0.0, 0.05, 0.0], p1: [0.4, 0.05, 0.0], rgb: [255, 38, 38] }];
        let housing = Glass { rgb, frame: 0.0, inset: 0.03, tint: 0.5 };
        let paths = vec![LedPath { pts: vec![[0.0, 0.0], [0.5, 0.0]], repeat: 1, step: [0.0; 2], style: LedStyle::Bar { width: 0.04 }, margin: 0.0, clip: 0.0, halo: true }];
        let (bar, strips) = lamps(&faces, &halos, &[Lamp { housing, kind: LampKind::Leds { paths, inset: 0.04 } }]);
        assert!(!bar.is_empty() && bar.len() % 3 == 0);
        assert_eq!(strips.len(), 1);
        assert!(bar.iter().all(|v| v.pos[1] >= -1e-5 && v.pos[1] <= 0.1 + 1e-5));
    }
}
