//! REMASTER added aero parts for the generated bodies (09-remaster 9.22): the front splitter,
//! a thin carbon blade under the nose (after the user's Audi R8 reference), the R-Build 2's
//! rear wing on pylons and its diffuser fins (#171 phase 3, after the user's reference
//! render). The MYO has none; the splitter's outline is the cage's bottom front edge [O], the
//! rest a design choice. (The Titan's wing legs are part of its thickened wing,
//! `super::cuts::thicken`.)
//!
//! Input in car-local game space; output in view space (game z negated) like `body::body`.

use super::body::{tri, Face};
use super::cuts::{facing, hits};
use super::lamps::vert;
use crate::scene::Vertex;
use glam::{Vec2, Vec3};

#[derive(Clone, Debug, PartialEq)]
pub struct Splitter {
    /// the body's bottom front edge from the middle (x 0) to the blade's outer end, game
    /// space; mirrored to -x
    pub edge: Vec<[f32; 3]>,
    /// the blade sticks out this far past the edge (m, in the ground plane) and runs this far
    /// back under the body
    pub reach: f32,
    pub depth: f32,
    /// blade thickness (m), hanging below the edge
    pub thickness: f32,
    /// 0: the outline straight between the `edge` points and a flat front face (the R-Build 2);
    /// N: a curve through them and a half-round front in N segments (the Titan, #291)
    pub nose: usize,
    /// the reach and thickness fade to nothing over this length (m) at each outer end, so the
    /// blade runs into the bodywork; 0: a square end
    pub taper: f32,
    /// carbon fibre (needs the weave's texture layer) instead of matte black
    pub carbon: bool,
}

/// matte carbon black, a shade darker than the vent bars
const BLADE: [f32; 4] = [0.045, 0.045, 0.05, 1.0];
/// the curved outline's point spacing (m)
const CURVE_STEP: f32 = 0.02;

/// `pts` resampled every `CURVE_STEP` along a centripetal Catmull-Rom curve through them (the
/// ends continue straight): no overshoot or cusp at the outline's uneven spans.
fn curve(pts: &[Vec3]) -> Vec<Vec3> {
    let n = pts.len();
    let at = |i: isize| -> Vec3 {
        match i {
            -1 => 2.0 * pts[0] - pts[1],
            i if i as usize >= n => 2.0 * pts[n - 1] - pts[n - 2],
            i => pts[i as usize],
        }
    };
    let mut out = Vec::new();
    for k in 0..n - 1 {
        let p = [at(k as isize - 1), at(k as isize), at(k as isize + 1), at(k as isize + 2)];
        // knots by the square root of the chord lengths (Barry-Goldman)
        let mut t = [0.0f32; 4];
        for i in 1..4 {
            t[i] = t[i - 1] + p[i].distance(p[i - 1]).sqrt().max(1e-4);
        }
        let lerp = |a: Vec3, b: Vec3, ta: f32, tb: f32, u: f32| a + (b - a) * ((u - ta) / (tb - ta));
        let m = ((p[1].distance(p[2]) / CURVE_STEP).ceil() as usize).max(1);
        for j in 0..m {
            let u = t[1] + (t[2] - t[1]) * j as f32 / m as f32;
            let (a1, a2, a3) = (lerp(p[0], p[1], t[0], t[1], u), lerp(p[1], p[2], t[1], t[2], u), lerp(p[2], p[3], t[2], t[3], u));
            let (b1, b2) = (lerp(a1, a2, t[0], t[2], u), lerp(a2, a3, t[1], t[3], u));
            out.push(lerp(b1, b2, t[1], t[2], u));
        }
    }
    out.push(pts[n - 1]);
    out
}

/// The splitter's blade: a slab between the outline pushed out by `reach` and in by `depth`;
/// carbon fibre on texture layer `carbon` when the splitter asks for it.
pub fn splitter(s: &Splitter, carbon: Option<f32>) -> Vec<Vertex> {
    let carbon = carbon.filter(|_| s.carbon);
    let mut pts: Vec<Vec3> = s.edge.iter().rev().filter(|p| p[0] > 1e-4).map(|p| Vec3::new(-p[0], p[1], p[2])).collect();
    pts.extend(s.edge.iter().map(|&p| Vec3::from(p)));
    if pts.len() < 2 {
        return Vec::new();
    }
    // REMASTER (#291): a curve through the edge's points, without its kinks
    if s.nose > 0 {
        pts = curve(&pts);
    }
    let n = pts.len();
    // outward in the ground plane: the tangent turned towards +z for an edge running -x → +x
    let out: Vec<Vec3> = (0..n)
        .map(|i| {
            let t = pts[(i + 1).min(n - 1)] - pts[i.saturating_sub(1)];
            let o = Vec2::new(-t.z, t.x).normalize_or(Vec2::Y);
            Vec3::new(o.x, 0.0, o.y)
        })
        .collect();
    // the taper's scale per point: smoothstep over `taper` from either end
    let mut along = vec![0.0f32; n];
    for i in 1..n {
        along[i] = along[i - 1] + pts[i].distance(pts[i - 1]);
    }
    let scale: Vec<f32> = (0..n)
        .map(|i| {
            if s.taper <= 0.0 {
                return 1.0;
            }
            let u = (along[i].min(along[n - 1] - along[i]) / s.taper).clamp(0.0, 1.0);
            u * u * (3.0 - 2.0 * u)
        })
        .collect();
    let down: Vec<Vec3> = scale.iter().map(|k| Vec3::Y * s.thickness * k).collect();
    // the nose's radius: half the thickness (none for the flat front)
    let r: Vec<f32> = (0..n).map(|i| if s.nose > 0 { 0.5 * down[i].y } else { 0.0 }).collect();
    let front: Vec<Vec3> = (0..n).map(|i| pts[i] + out[i] * (s.reach * scale[i] - r[i])).collect();
    let back: Vec<Vec3> = (0..n).map(|i| pts[i] - out[i] * s.depth).collect();
    let mut v = Vec::new();
    let q = |p: Vec3, nrm: Vec3| match carbon {
        Some(layer) => Vertex { uv: super::carbon_uv(p, nrm), layer, ..vert(p, nrm, super::CARBON_TINT, super::CARBON_ENV) },
        None => vert(p, nrm, BLADE, 0.0),
    };
    // a quad a-b-c-d with a normal per corner
    let mut quad_n = |c: [Vec3; 4], nrm: [Vec3; 4]| {
        tri(&mut v, [q(c[0], nrm[0]), q(c[1], nrm[1]), q(c[2], nrm[2])]);
        tri(&mut v, [q(c[0], nrm[0]), q(c[2], nrm[2]), q(c[3], nrm[3])]);
    };
    // the front's profile from top to bottom: offset from `front` and normal, per point
    let prof = |i: usize, j: usize| -> (Vec3, Vec3) {
        if s.nose == 0 {
            return if j == 0 { (Vec3::ZERO, out[i]) } else { (-down[i], out[i]) };
        }
        let a = std::f32::consts::FRAC_PI_2 - std::f32::consts::PI * j as f32 / s.nose as f32;
        let nrm = out[i] * a.cos() + Vec3::Y * a.sin();
        (nrm * r[i] - Vec3::Y * r[i], nrm)
    };
    let rows = s.nose.max(1);
    for i in 0..n - 1 {
        let j = i + 1;
        let o = (out[i] + out[j]).normalize_or_zero();
        quad_n([back[i], back[j], front[j], front[i]], [Vec3::Y; 4]);
        quad_n([back[i] - down[i], front[i] - down[i], front[j] - down[j], back[j] - down[j]], [Vec3::NEG_Y; 4]);
        for k in 0..rows {
            let ((ai, an), (bi, bn)) = (prof(i, k), prof(i, k + 1));
            let ((aj, ajn), (bj, bjn)) = (prof(j, k), prof(j, k + 1));
            let (ni, nj) = if s.nose == 0 { (o, o) } else { (an, ajn) };
            let (mi, mj) = if s.nose == 0 { (o, o) } else { (bn, bjn) };
            quad_n([front[i] + ai, front[j] + aj, front[j] + bj, front[i] + bi], [ni, nj, mj, mi]);
        }
        quad_n([back[i], back[j], back[j] - down[j], back[i] - down[i]], [-o; 4]);
    }
    // square ends where the blade keeps its size
    for (i, side) in [(0, -1.0), (n - 1, 1.0)] {
        if scale[i] <= 0.0 {
            continue;
        }
        let t = Vec3::new(out[i].z, 0.0, -out[i].x) * side;
        quad_n([back[i], front[i] + out[i] * r[i], front[i] + out[i] * r[i] - down[i], back[i] - down[i]], [t; 4]);
    }
    v
}

/// A rear wing on pylons: a slab `2 × span` wide (x) and `chord` deep (z), `thickness` thick,
/// centred at (0, `y`, `z`) and tilted `angle_deg` nose-up about x; an end plate at each tip,
/// `plate` tall and as thick as the slab; two pylons at ±`pylon_x` under the slab's middle,
/// `pylon_width` along z and `pylon_thickness` across, each corner running down to the first
/// cage surface below it (the ducktail wedges), buried `BURY`. Game space, metres.
#[derive(Clone, Debug, PartialEq)]
pub struct Wing {
    pub span: f32,
    pub chord: f32,
    pub thickness: f32,
    pub y: f32,
    pub z: f32,
    pub angle_deg: f32,
    pub plate: f32,
    pub pylon_x: f32,
    pub pylon_width: f32,
    pub pylon_thickness: f32,
    /// carbon fibre (needs the weave's texture layer) instead of matte black
    pub carbon: bool,
}

/// Diffuser fins under the tail: at each of `xs` (and its mirror), a blade `thickness` thick
/// from `z0` to `z1` whose top follows the underside of the cage (buried `BURY`) and whose
/// bottom hangs `depth` under it, never below `FIN_FLOOR`.
#[derive(Clone, Debug, PartialEq)]
pub struct Fins {
    pub xs: Vec<f32>,
    pub z0: f32,
    pub z1: f32,
    pub depth: f32,
    pub thickness: f32,
    pub carbon: bool,
}

/// how far the pylons and fins reach into the surface they stand on (m)
const BURY: f32 = 0.01;
/// the fins never hang below this height over the model's ground (m): the floor is at 0.227
/// on the R-Build 2, so a fin keeps a few millimetres of clearance after the drop
const FIN_FLOOR: f32 = 0.23;
/// the fins' top is sampled this often along z (m)
const FIN_STEP: f32 = 0.01;
/// end plates are this much deeper than the slab, as a fraction of the chord
const PLATE_OVERHANG: f32 = 0.1;

/// the look of an aero part: the carbon weave on texture layer `carbon`, or matte black
fn look(carbon: Option<f32>) -> impl Fn(Vec3, Vec3) -> Vertex {
    move |p, nrm| match carbon {
        Some(layer) => Vertex { uv: super::carbon_uv(p, nrm), layer, ..vert(p, nrm, super::CARBON_TINT, super::CARBON_ENV) },
        None => vert(p, nrm, BLADE, 0.0),
    }
}

/// A flat quad a-b-c-d facing `nrm`, wound like the body's faces whichever way it is given.
fn quad(v: &mut Vec<Vertex>, mut c: [Vec3; 4], nrm: Vec3, q: &dyn Fn(Vec3, Vec3) -> Vertex) {
    let nrm = nrm.normalize_or(Vec3::Y);
    // the splitter's rule: the corners' cross product points against the normal
    if (c[1] - c[0]).cross(c[2] - c[0]).dot(nrm) > 0.0 {
        c.reverse();
    }
    tri(v, [q(c[0], nrm), q(c[1], nrm), q(c[2], nrm)]);
    tri(v, [q(c[0], nrm), q(c[2], nrm), q(c[3], nrm)]);
}

/// A hexahedron from its bottom and top quads (`b[0..4]`, `t[0..4]`, b[i] under t[i]); every
/// face gets the plane normal pointing away from the block's centre.
fn hexa(v: &mut Vec<Vertex>, b: [Vec3; 4], t: [Vec3; 4], q: &dyn Fn(Vec3, Vec3) -> Vertex) {
    let centre = (b.iter().sum::<Vec3>() + t.iter().sum::<Vec3>()) / 8.0;
    let mut face = |c: [Vec3; 4]| {
        let n: Vec3 = (0..4).map(|i| c[i].cross(c[(i + 1) % 4])).sum();
        let mid = c.iter().sum::<Vec3>() / 4.0;
        let n = if n.dot(mid - centre) < 0.0 { -n } else { n };
        quad(v, c, n, q);
    };
    face(b);
    face(t);
    for i in 0..4 {
        let j = (i + 1) % 4;
        face([b[i], b[j], t[j], t[i]]);
    }
}

/// REMASTER a wing rebuilt over the cage's own (the Serpent's swept rear wing, #174 phase 4,
/// the user's request "recreate it from scratch but keep its overall shape, with struts"): the
/// cage faces of tag `from` with every corner (x as |x|) in the box `lo`..`hi` go, and a lofted
/// aerofoil in their look takes their place, with its struts. The sections run along the car
/// (game z) from the leading edge `le` to the trailing edge `te` at each station across the
/// span, a NACA four-digit thickness (closed trailing edge) on a parabolic camber line; the
/// tips are flat caps. As cage faces they go through the bevel and the refine like the rest of
/// the body: the loft is shaded smooth (`FOIL_SMOOTH`), the trailing edge and the tip caps are
/// creases. Design choice throughout.
#[derive(Clone, Debug, PartialEq)]
pub struct Foil {
    /// the cage wing's tags; with several (#177 phase 3, [O] the Avalanche's wing is white with
    /// an orange stripe down its middle) each strip of the loft between two sections, each tip
    /// cap and each strut takes the look of the cage wing face over its middle (`look_at`)
    pub from: Vec<[u8; 3]>,
    pub lo: [f32; 3],
    pub hi: [f32; 3],
    /// the leading and trailing edges from the middle (x 0) out to the tip, game space, mirrored
    /// to -x, x rising; a curve through the points (`curve`), so the outline has no kinks. The
    /// span ends at the smaller of the two last x (the tip cap)
    pub le: Vec<[f32; 3]>,
    pub te: Vec<[f32; 3]>,
    /// the section's thickness and its camber, fractions of the chord; a negative camber bows
    /// the section down (a downforce wing: flatter on top, rounder underneath)
    pub thickness: f32,
    pub camber: f32,
    /// spacing (m) of the sections across the span
    pub step: f32,
    /// the struts: per entry (x, fraction along the chord) of the strut's middle, mirrored; each
    /// stands from inside the wing (its camber line) down to the first cage surface under it,
    /// buried `BURY`; one with nothing under a corner is left out
    pub struts: Vec<[f32; 2]>,
    /// a strut's length along z and its thickness across (m): a symmetric aerofoil in plan
    pub strut_chord: f32,
    pub strut_width: f32,
}

/// the loft and the struts are shaded smooth across folds flatter than this (degrees): the
/// round leading edge; the trailing edge and the caps fold further
const FOIL_SMOOTH: f32 = 45.0;
/// points per surface of a wing section, of a strut's side (half-cosine spacing: dense at the
/// round leading edge, even towards the sharp trailing edge, where full cosine spacing put
/// points under the 1 mm weld grid apart on a short chord and they welded into non-manifold
/// edges)
const FOIL_N: usize = 14;
const STRUT_N: usize = 8;

/// NACA four-digit half thickness at chord fraction `s` for thickness `t`, closed trailing edge
fn naca(s: f32, t: f32) -> f32 {
    5.0 * t * (0.2969 * s.sqrt() - 0.1260 * s - 0.3516 * s * s + 0.2843 * s.powi(3) - 0.1036 * s.powi(4))
}

/// chord fraction of point `j` of `n` (half-cosine spacing)
fn cos_s(j: usize, n: usize) -> f32 {
    1.0 - (std::f32::consts::FRAC_PI_2 * j as f32 / n as f32).cos()
}

/// The polyline's point at `x` (linear between its points, x rising; the ends held).
fn at_x(pts: &[Vec3], x: f32) -> Vec3 {
    let Some(i) = pts.windows(2).position(|w| x <= w[1].x) else { return *pts.last().unwrap_or(&Vec3::ZERO) };
    let (a, b) = (pts[i], pts[i + 1]);
    if b.x - a.x < 1e-6 {
        return if x <= a.x { a } else { b };
    }
    a.lerp(b, ((x - a.x) / (b.x - a.x)).clamp(0.0, 1.0))
}

/// A wing section at station `x`: leading edge, chord vector, its up (the chord's normal
/// nearest +y), and the ring of `2 FOIL_N` points: the trailing edge, up the top to the leading
/// edge, back along the bottom.
struct Section {
    le: Vec3,
    d: Vec3,
    up: Vec3,
    ring: Vec<Vec3>,
}

impl Section {
    fn new(fo: &Foil, le: &[Vec3], te: &[Vec3], x: f32) -> Section {
        let flip = |p: Vec3| Vec3::new(x, p.y, p.z);
        let (l, t) = (flip(at_x(le, x.abs())), flip(at_x(te, x.abs())));
        let d = t - l;
        let up = (Vec3::Y - d.normalize_or(Vec3::NEG_Z) * d.normalize_or(Vec3::NEG_Z).y).normalize_or(Vec3::Y);
        let c = d.length();
        let pt = |s: f32, side: f32| l + d * s + up * (c * (fo.camber * 4.0 * s * (1.0 - s) + side * naca(s, fo.thickness)));
        let mut ring: Vec<Vec3> = (0..=FOIL_N).map(|k| pt(cos_s(FOIL_N - k, FOIL_N), 1.0)).collect();
        ring.extend((1..FOIL_N).map(|k| pt(cos_s(k, FOIL_N), -1.0)));
        Section { le: l, d, up, ring }
    }
    /// the camber line's point at chord fraction `s`
    fn camber(&self, fo: &Foil, s: f32) -> Vec3 {
        self.le + self.d * s + self.up * (self.d.length() * fo.camber * 4.0 * s * (1.0 - s))
    }
}

/// Whether (x, z) lies inside the face's outline seen from above (crossing count).
fn over(f: &Face, x: f32, z: f32) -> bool {
    let n = f.pts.len();
    (0..n).filter(|&i| {
        let (a, b) = (f.pts[i], f.pts[(i + 1) % n]);
        (a[2] > z) != (b[2] > z) && x < a[0] + (b[0] - a[0]) * (z - a[2]) / (b[2] - a[2])
    }).count() % 2 == 1
}

/// REMASTER (#177 phase 3): the look for a part of the rebuilt wing at (x, z): that of the first
/// face of the tag of the picked face over it (the faces seen from above: tops, the sloping back,
/// undersides; not the tips' upright walls), else of the one whose middle is nearest. One face
/// per tag, so with one tag every part looks like the first picked face (the Serpent's, as before).
fn look_at(picked: &[Face], x: f32, z: f32) -> &Face {
    let flat: Vec<&Face> = picked.iter().filter(|f| f.normal[1].abs() > 0.5 * Vec3::from(f.normal).length()).collect();
    let mid = |f: &Face| f.pts.iter().fold(Vec2::ZERO, |s, p| s + Vec2::new(p[0], p[2])) / f.pts.len() as f32;
    let here = flat
        .iter()
        .find(|f| over(f, x, z))
        .or_else(|| flat.iter().min_by(|a, b| mid(a).distance(Vec2::new(x, z)).total_cmp(&mid(b).distance(Vec2::new(x, z)))))
        .map_or(picked[0].rgb, |f| f.rgb);
    picked.iter().find(|f| f.rgb == here).unwrap_or(&picked[0])
}

/// Apply the foils: the picked cage faces out, the wing and its struts in.
pub fn foils(faces: &mut Vec<Face>, foils: &[Foil]) {
    for fo in foils {
        let picked = |f: &Face| fo.from.contains(&f.rgb) && f.pts.iter().all(|p| (0..3).all(|i| (if i == 0 { p[0].abs() } else { p[i] }) >= fo.lo[i] && (if i == 0 { p[0].abs() } else { p[i] }) <= fo.hi[i]));
        let cage: Vec<Face> = faces.iter().filter(|f| picked(f)).cloned().collect();
        if cage.is_empty() {
            continue;
        }
        faces.retain(|f| !picked(f));
        let mirrored = |pts: &[[f32; 3]]| -> Vec<Vec3> {
            let mut all: Vec<Vec3> = pts.iter().rev().filter(|p| p[0] > 1e-4).map(|p| Vec3::new(-p[0], p[1], p[2])).collect();
            all.extend(pts.iter().map(|&p| Vec3::from(p)));
            if all.len() > 2 { curve(&all) } else { all }.into_iter().filter(|p| p.x >= -1e-4).collect()
        };
        let (le, te) = (mirrored(&fo.le), mirrored(&fo.te));
        let (Some(a), Some(b)) = (le.last(), te.last()) else { continue };
        let span = a.x.min(b.x);
        if span <= 0.0 || fo.step <= 0.0 {
            continue;
        }
        let n = (span / fo.step).ceil() as i32;
        let secs: Vec<Section> = (-n..=n).map(|i| Section::new(fo, &le, &te, span * i as f32 / n as f32)).collect();
        let mut added: Vec<Face> = Vec::new();
        let mut add = |src: &Face, pts: [Vec3; 3], out: Vec3| added.push(facing(src, pts.map(|p| p.to_array()).to_vec(), out));
        let m = 2 * FOIL_N;
        let chord_mid = |s: &Section| s.le + s.d * 0.5;
        for w in secs.windows(2) {
            let (a, b) = (&w[0].ring, &w[1].ring);
            let up = (w[0].up + w[1].up).normalize_or(Vec3::Y);
            let c = (chord_mid(&w[0]) + chord_mid(&w[1])) * 0.5;
            let src = look_at(&cage, c.x, c.z);
            for k in 0..m {
                let k2 = (k + 1) % m;
                // the top runs from the trailing edge (0) to the leading edge (FOIL_N)
                let out = if k < FOIL_N { up } else { -up };
                add(src, [a[k], a[k2], b[k2]], out);
                add(src, [a[k], b[k2], b[k]], out);
            }
        }
        // the tip caps
        for (s, side) in [(secs.first(), -1.0), (secs.last(), 1.0)] {
            if let Some(s) = s {
                let c = chord_mid(s);
                added.push(facing(look_at(&cage, c.x, c.z), s.ring.iter().map(|p| p.to_array()).collect(), Vec3::X * side));
            }
        }
        // the struts: a symmetric aerofoil in plan along z, from the wing's camber line down
        for &[sx, f] in &fo.struts {
            for side in [-1.0, 1.0] {
                let sec = Section::new(fo, &le, &te, sx * side);
                let mid = sec.camber(fo, f);
                let half = fo.strut_chord / 2.0;
                let tr = fo.strut_width / fo.strut_chord.max(1e-4);
                let mut ring: Vec<Vec3> = (0..=STRUT_N).map(|k| { let s = cos_s(k, STRUT_N); Vec3::new(mid.x + fo.strut_chord * naca(s, tr), 0.0, mid.z + half - s * fo.strut_chord) }).collect();
                ring.extend((1..STRUT_N).rev().map(|k| { let s = cos_s(k, STRUT_N); Vec3::new(mid.x - fo.strut_chord * naca(s, tr), 0.0, mid.z + half - s * fo.strut_chord) }));
                // each corner's top on the wing's camber line over it, its foot under it
                let tops: Vec<Vec3> = ring
                    .iter()
                    .map(|p| {
                        let s = ((p.z - sec.le.z) / sec.d.z).clamp(0.0, 1.0);
                        Vec3::new(p.x, sec.camber(fo, s).y, p.z)
                    })
                    .collect();
                let feet: Option<Vec<Vec3>> = tops.iter().map(|t| hits(faces, t.x, t.z).into_iter().find(|&y| y < t.y - 0.01).map(|y| Vec3::new(t.x, y - BURY, t.z))).collect();
                let Some(feet) = feet else { continue };
                let src = look_at(&cage, mid.x, mid.z);
                let r = ring.len();
                for k in 0..r {
                    let k2 = (k + 1) % r;
                    let mid_xz = (ring[k] + ring[k2]) / 2.0;
                    let out = Vec3::new(mid_xz.x - mid.x, 0.0, mid_xz.z - mid.z);
                    added.push(facing(src, vec![feet[k].to_array(), feet[k2].to_array(), tops[k2].to_array(), tops[k].to_array()], out));
                }
            }
        }
        for f in &mut added {
            f.smooth_deg = FOIL_SMOOTH;
            f.remesh = 0.0;
        }
        faces.extend(added);
    }
}

/// The wing: slab, end plates and the pylons standing on `faces` (a pylon with nothing under
/// one of its corners is left out).
pub fn wing(w: &Wing, faces: &[Face], carbon: Option<f32>) -> Vec<Vertex> {
    let q = look(carbon.filter(|_| w.carbon));
    let mut v = Vec::new();
    let (sa, ca) = w.angle_deg.to_radians().sin_cos();
    let c = Vec3::new(0.0, w.y, w.z);
    let (ax, az, ay) = (Vec3::X * w.span, Vec3::new(0.0, sa, ca) * (w.chord / 2.0), Vec3::new(0.0, ca, -sa) * (w.thickness / 2.0));
    let corners = |dy: f32| [c - ax - az + ay * dy, c + ax - az + ay * dy, c + ax + az + ay * dy, c - ax + az + ay * dy];
    hexa(&mut v, corners(-1.0), corners(1.0), &q);
    // end plates: upright boxes round each tip
    let (hx, hy, hz) = (w.thickness / 2.0, w.plate / 2.0, w.chord * (0.5 + PLATE_OVERHANG));
    for side in [-1.0, 1.0] {
        let pc = Vec3::new(side * w.span, w.y, w.z);
        let ring = |dy: f32| [pc + Vec3::new(-hx, dy * hy, -hz), pc + Vec3::new(hx, dy * hy, -hz), pc + Vec3::new(hx, dy * hy, hz), pc + Vec3::new(-hx, dy * hy, hz)];
        hexa(&mut v, ring(-1.0), ring(1.0), &q);
    }
    // pylons: from inside the slab down to the surface under each corner
    let (px, pz) = (w.pylon_thickness / 2.0, w.pylon_width / 2.0);
    for side in [-1.0, 1.0] {
        let x = side * w.pylon_x;
        let top = [Vec3::new(x - px, w.y, w.z - pz), Vec3::new(x + px, w.y, w.z - pz), Vec3::new(x + px, w.y, w.z + pz), Vec3::new(x - px, w.y, w.z + pz)];
        let ground = |p: Vec3| hits(faces, p.x, p.z).into_iter().find(|&y| y < w.y - w.thickness).map(|y| Vec3::new(p.x, y - BURY, p.z));
        if let [Some(a), Some(b), Some(cc), Some(d)] = top.map(ground) {
            hexa(&mut v, [a, b, cc, d], top, &q);
        }
    }
    v
}

/// The fins under `faces`.
pub fn fins(f: &Fins, faces: &[Face], carbon: Option<f32>) -> Vec<Vertex> {
    let q = look(carbon.filter(|_| f.carbon));
    let mut v = Vec::new();
    let (z0, z1) = (f.z0.min(f.z1), f.z0.max(f.z1));
    let n = ((z1 - z0) / FIN_STEP).ceil().max(1.0) as usize;
    let h = f.thickness / 2.0;
    let xs = f.xs.iter().flat_map(|&x| if x.abs() < 1e-4 { vec![0.0] } else { vec![x, -x] });
    for x in xs {
        // (top, bottom) per z sample, where the cage has an underside
        let profile: Vec<(f32, f32, f32)> = (0..=n)
            .filter_map(|k| {
                let z = z0 + (z1 - z0) * k as f32 / n as f32;
                let top = hits(faces, x, z).pop()?;
                let bottom = (top - f.depth).max(FIN_FLOOR);
                (bottom < top).then_some((z, top + BURY, bottom))
            })
            .collect();
        for s in profile.windows(2) {
            let [(za, ta, ba), (zb, tb, bb)] = [s[0], s[1]];
            for side in [-1.0, 1.0] {
                let xx = x + side * h;
                quad(&mut v, [Vec3::new(xx, ta, za), Vec3::new(xx, tb, zb), Vec3::new(xx, bb, zb), Vec3::new(xx, ba, za)], Vec3::X * side, &q);
            }
            quad(&mut v, [Vec3::new(x - h, ba, za), Vec3::new(x + h, ba, za), Vec3::new(x + h, bb, zb), Vec3::new(x - h, bb, zb)], Vec3::NEG_Y, &q);
        }
        if let (Some(&(za, ta, ba)), Some(&(zb, tb, bb))) = (profile.first(), profile.last()) {
            quad(&mut v, [Vec3::new(x - h, ta, za), Vec3::new(x + h, ta, za), Vec3::new(x + h, ba, za), Vec3::new(x - h, ba, za)], Vec3::NEG_Z, &q);
            quad(&mut v, [Vec3::new(x - h, tb, zb), Vec3::new(x + h, tb, zb), Vec3::new(x + h, bb, zb), Vec3::new(x - h, bb, zb)], Vec3::Z, &q);
        }
    }
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    /// a straight edge along x at z 1: the blade spans -1..1, z 1 - depth .. 1 + reach, and
    /// hangs `thickness` below the edge
    #[test]
    fn blade_round_the_edge() {
        static EDGE: [[f32; 3]; 2] = [[0.0, 0.2, 1.0], [1.0, 0.2, 1.0]];
        let v = splitter(&Splitter { edge: EDGE.to_vec(), reach: 0.05, depth: 0.1, thickness: 0.01, nose: 0, taper: 0.0, carbon: false }, None);
        assert!(!v.is_empty());
        for p in &v {
            let (x, y, z) = (p.pos[0], p.pos[1], -p.pos[2]);
            assert!((-1.0 - 1e-5..=1.0 + 1e-5).contains(&x), "{x}");
            assert!((0.19 - 1e-5..=0.2 + 1e-5).contains(&y), "{y}");
            assert!((0.9 - 1e-5..=1.05 + 1e-5).contains(&z), "{z}");
        }
        assert!(v.iter().any(|p| (-p.pos[2] - 1.05).abs() < 1e-5));
    }

    fn flat(y: f32, rgb: [u8; 3]) -> Face {
        let pts = vec![[-2.0, y, -3.0], [2.0, y, -3.0], [2.0, y, 3.0], [-2.0, y, 3.0]];
        Face { pts, normal: [0.0, 1.0, 0.0], colour: vec![[0.5; 4]; 4], uv: vec![[0.0; 2]; 4], layer: -1.0, env: 0.0, rgb, smooth_deg: 0.0, remesh: 0.0 }
    }

    /// over a flat deck at y 1: the slab spans ±span, the end plates stand at the tips, and the
    /// pylons end `BURY` under the deck
    #[test]
    fn wing_on_pylons() {
        let w = Wing { span: 0.8, chord: 0.2, thickness: 0.02, y: 1.1, z: -1.7, angle_deg: 0.0, plate: 0.06, pylon_x: 0.4, pylon_width: 0.1, pylon_thickness: 0.03, carbon: false };
        let deck = [flat(1.0, [1, 2, 3])];
        let v = wing(&w, &deck, None);
        let xs: Vec<f32> = v.iter().map(|p| p.pos[0]).collect();
        let max_x = xs.iter().cloned().fold(f32::MIN, f32::max);
        assert!((max_x - 0.81).abs() < 1e-4, "{max_x}");
        let min_y = v.iter().map(|p| p.pos[1]).fold(f32::MAX, f32::min);
        assert!((min_y - (1.0 - BURY)).abs() < 1e-5, "{min_y}");
        // the pylon's feet sit at ±pylon_x
        assert!(v.iter().any(|p| (p.pos[1] - (1.0 - BURY)).abs() < 1e-5 && (p.pos[0].abs() - 0.415).abs() < 1e-4));
        // nothing under the slab: no pylons, the lowest point is the end plates' bottom
        let v = wing(&w, &[], None);
        let min_y = v.iter().map(|p| p.pos[1]).fold(f32::MAX, f32::min);
        assert!((min_y - 1.07).abs() < 1e-5, "{min_y}");
    }

    /// a swept foil over a flat deck (#174 phase 4): the picked face goes, the loft and its caps
    /// are closed (every edge on two faces), shaded smooth, span ±tip; each strut stands from
    /// inside the wing down to `BURY` under the deck
    #[test]
    fn foil_on_struts() {
        let mut wing = flat(1.2, [9, 9, 9]);
        wing.pts = vec![[-0.5, 1.2, -1.8], [0.5, 1.2, -1.8], [0.5, 1.2, -1.5], [-0.5, 1.2, -1.5]];
        let mut faces = vec![flat(1.0, [1, 2, 3]), wing];
        let fo = Foil {
            from: vec![[9, 9, 9]],
            lo: [0.0, 1.1, -1.9],
            hi: [0.6, 1.3, -1.4],
            le: vec![[0.0, 1.2, -1.5], [0.3, 1.2, -1.45], [0.5, 1.21, -1.4]],
            te: vec![[0.0, 1.2, -1.8], [0.3, 1.2, -1.78], [0.5, 1.21, -1.7]],
            thickness: 0.12,
            camber: -0.03,
            step: 0.05,
            struts: vec![[0.25, 0.5]],
            strut_chord: 0.1,
            strut_width: 0.012,
        };
        foils(&mut faces, &[fo]);
        assert!(faces.iter().all(|f| f.rgb != [9, 9, 9] || f.smooth_deg == FOIL_SMOOTH), "the cage wing is gone");
        let added: Vec<&Face> = faces.iter().filter(|f| f.rgb == [9, 9, 9]).collect();
        let max_x = added.iter().flat_map(|f| f.pts.iter()).map(|p| p[0]).fold(f32::MIN, f32::max);
        assert!((max_x - 0.5).abs() < 1e-4, "{max_x}");
        // the struts: the faces reaching under y 1.1
        let (struts, wing): (Vec<&Face>, Vec<&Face>) = added.iter().partition(|f| f.pts.iter().any(|p| p[1] < 1.1));
        let feet: Vec<[f32; 3]> = struts.iter().flat_map(|f| f.pts.iter().copied()).filter(|p| p[1] < 1.1).collect();
        assert!(!feet.is_empty() && feet.iter().all(|p| (p[1] - (1.0 - BURY)).abs() < 1e-5 && (p[0].abs() - 0.25).abs() <= 0.007), "{feet:?}");
        assert!(feet.iter().any(|p| p[0] > 0.0) && feet.iter().any(|p| p[0] < 0.0));
        let key = |p: [f32; 3]| p.map(|c| (c * 1e4).round() as i64);
        let mut uses: std::collections::HashMap<([i64; 3], [i64; 3]), usize> = Default::default();
        for f in &wing {
            for i in 0..f.pts.len() {
                let (a, b) = (key(f.pts[i]), key(f.pts[(i + 1) % f.pts.len()]));
                *uses.entry(if a < b { (a, b) } else { (b, a) }).or_default() += 1;
            }
        }
        assert!(uses.values().all(|&n| n == 2), "open or doubled edges: {}", uses.values().filter(|&&n| n != 2).count());
    }

    /// #177 phase 3: a wing in two tags (a stripe down its middle, like the Avalanche's): each
    /// strip of the loft takes the tag of the cage face over it, so the stripe stays where it
    /// was, and the struts out on the sides take the sides' look
    #[test]
    fn foil_keeps_a_stripe() {
        let part = |x0: f32, x1: f32, rgb: [u8; 3]| {
            let mut f = flat(1.2, rgb);
            f.pts = vec![[x0, 1.2, -1.8], [x1, 1.2, -1.8], [x1, 1.2, -1.5], [x0, 1.2, -1.5]];
            f
        };
        let mut faces = vec![flat(1.0, [1, 2, 3]), part(-0.5, -0.2, [9, 9, 9]), part(-0.2, 0.2, [7, 7, 7]), part(0.2, 0.5, [9, 9, 9])];
        let fo = Foil {
            from: vec![[9, 9, 9], [7, 7, 7]],
            lo: [0.0, 1.1, -1.9],
            hi: [0.6, 1.3, -1.4],
            le: vec![[0.0, 1.2, -1.5], [0.5, 1.2, -1.5]],
            te: vec![[0.0, 1.2, -1.8], [0.5, 1.2, -1.8]],
            thickness: 0.12,
            camber: -0.03,
            step: 0.05,
            struts: vec![[0.35, 0.5]],
            strut_chord: 0.1,
            strut_width: 0.012,
        };
        foils(&mut faces, &[fo]);
        let xs = |rgb: [u8; 3]| -> Vec<f32> { faces.iter().filter(|f| f.rgb == rgb).flat_map(|f| f.pts.iter().map(|p| p[0].abs())).collect() };
        let (stripe, sides) = (xs([7, 7, 7]), xs([9, 9, 9]));
        assert!(!stripe.is_empty() && stripe.iter().all(|&x| x <= 0.2 + 1e-4), "the stripe stays in the middle");
        assert!(!sides.is_empty() && sides.iter().all(|&x| x >= 0.2 - 1e-4), "the sides stay outside it");
        assert!(faces.iter().any(|f| f.rgb == [9, 9, 9] && f.pts.iter().any(|p| p[1] < 1.1)), "the struts look like the sides");
        assert!(faces.iter().all(|f| f.rgb == [1, 2, 3] || f.smooth_deg == FOIL_SMOOTH), "the cage wing is gone");
    }

    /// under a flat floor at y 0.3 the fins hang `depth` below it; under one at 0.26 they stop at
    /// `FIN_FLOOR`; both sides get one
    #[test]
    fn fins_under_the_floor() {
        let f = Fins { xs: vec![0.3], z0: -2.0, z1: -1.8, depth: 0.05, thickness: 0.01, carbon: false };
        let v = fins(&f, &[flat(0.3, [0; 3])], None);
        let ys: Vec<f32> = v.iter().map(|p| p.pos[1]).collect();
        let (lo, hi) = (ys.iter().cloned().fold(f32::MAX, f32::min), ys.iter().cloned().fold(f32::MIN, f32::max));
        assert!((lo - 0.25).abs() < 1e-5 && (hi - (0.3 + BURY)).abs() < 1e-5, "{lo} {hi}");
        assert!(v.iter().any(|p| p.pos[0] > 0.29) && v.iter().any(|p| p.pos[0] < -0.29));
        let v = fins(&f, &[flat(0.26, [0; 3])], None);
        let lo = v.iter().map(|p| p.pos[1]).fold(f32::MAX, f32::min);
        assert!((lo - FIN_FLOOR).abs() < 1e-5, "{lo}");
    }

    /// a round nose and tapered ends (#291): the lip still reaches `reach` in the middle and
    /// hangs `thickness`, its normals turn from up through out to down, and at both ends the
    /// blade has shrunk onto the edge
    #[test]
    fn round_nose_tapers() {
        static EDGE: [[f32; 3]; 2] = [[0.0, 0.2, 1.0], [1.0, 0.2, 1.0]];
        let v = splitter(&Splitter { edge: EDGE.to_vec(), reach: 0.05, depth: 0.1, thickness: 0.01, nose: 4, taper: 0.3, carbon: false }, None);
        let pts: Vec<Vec3> = v.iter().map(|p| Vec3::new(p.pos[0], p.pos[1], -p.pos[2])).collect();
        let z_max = pts.iter().map(|p| p.z).fold(f32::MIN, f32::max);
        assert!((z_max - 1.05).abs() < 1e-4, "{z_max}");
        assert!(pts.iter().all(|p| p.y >= 0.19 - 1e-5 && p.y <= 0.2 + 1e-5 && p.z <= 1.05 + 1e-5));
        // the half-round: some normal points straight out (+z) at mid height
        assert!(v.iter().any(|p| -p.normal[2] > 0.99 && (p.pos[1] - 0.195).abs() < 1e-4));
        for end in [-1.0f32, 1.0] {
            let at: Vec<&Vec3> = pts.iter().filter(|p| (p.x - end).abs() < 1e-4).collect();
            assert!(!at.is_empty() && at.iter().all(|p| p.z <= 1.0 + 1e-5 && (p.y - 0.2).abs() < 1e-5), "{at:?}");
        }
    }

    /// carbon: the blade takes the layer and its uv runs one tile per `CARBON_TILE`
    #[test]
    fn carbon_blade() {
        static EDGE: [[f32; 3]; 2] = [[0.0, 0.2, 1.0], [1.0, 0.2, 1.0]];
        let v = splitter(&Splitter { edge: EDGE.to_vec(), reach: 0.05, depth: 0.1, thickness: 0.01, nose: 0, taper: 0.0, carbon: true }, Some(7.0));
        assert!(v.iter().all(|p| p.layer == 7.0 && p.env == crate::procgen::CARBON_ENV));
        // the top: u along x
        let us: Vec<f32> = v.iter().filter(|p| p.normal[1] > 0.9).map(|p| p.uv[0]).collect();
        let span = us.iter().cloned().fold(f32::MIN, f32::max) - us.iter().cloned().fold(f32::MAX, f32::min);
        assert!((span - 2.0 / crate::procgen::CARBON_TILE).abs() < 1e-2, "{span}");
    }
}
