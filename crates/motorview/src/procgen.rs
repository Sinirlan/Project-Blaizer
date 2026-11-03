//! REMASTER procedural car parts (09-remaster 9.20). Remaster only: Classic draws the MYO meshes.
//! Step 2 of the plan is in: wheels; the body is in `body` (9.22). Everything here is a design
//! choice; the inputs measured from the originals are marked [O].
//!
//! Output is in car-local view space like `scene::build_car` (game z negated), as plain
//! triangle lists of `scene::Vertex`, CCW from the front like `scene::emit`.

pub mod aero;
pub mod body;
pub mod bodyfile;
pub mod cuts;
pub mod exhaust;
pub mod fill;
pub mod grass;
pub mod lamps;
pub mod rimdetail;
pub mod rimfile;
pub mod rimplate;
pub mod smooth;
pub mod vents;
pub mod water;

pub use rimdetail::{Lugs, Rivets};
pub use rimplate::WindowPlate;

use crate::scene::Vertex;
use glam::{Mat3, Vec2, Vec3};

/// Segments around the axle. A multiple of 4, so rings have vertices at y = ±r and z = ±r and
/// the bounding box is exactly the original's. 32 (was 48, #218): the tyre's outline is a 32-gon
/// with 1.7 mm of chord sag at 0.35 m.
const SEGMENTS: usize = 32;
/// Rim face radius as a fraction of the tyre radius (about a 17" rim in a 25" tyre).
const RIM: f32 = 0.70;
/// [O] the original alloy textures draw the rim's outer lip at about 27 of the 30 texels that
/// span the wheel radius (radial profile of xALG0/xFAL0/xFEL0.iff, then black to the edge).
/// The generated face maps that circle to the rim radius.
const RIM_TEXTURE: f32 = 0.9;
/// how far the centre of the rim face sits inside the face edge, × half-width (a dished alloy)
const DISH: f32 = 0.30;
/// the face edge inside the tyre's outer wall, × half-width
const FACE_INSET: f32 = 0.18;
/// tyre bead (where rubber meets the lip) above the rim radius, × radius
const BEAD: f32 = 0.03;
/// superellipse exponent of the tyre's cross-section: 2 is an ellipse, higher is squarer
const TYRE_SHAPE: f32 = 3.2;
/// the inner face (towards the car) sits this far in, × half-width
const INNER_INSET: f32 = 0.55;
/// Rubber colour (sRGB bytes). [O] the original tread faces are (1, 1, 1), which per-pixel
/// light leaves pitch black; slightly lifted so the sidewall's shape reads.
const TYRE_RGB: [u8; 3] = [38, 38, 38];
/// `Vertex::env` of a textured tread (generated wheels): the texture layer is the rubber's colour,
/// the next layer its normal (rgb) and roughness (a), shaded matte [design choice]
pub const TYRE_ENV: f32 = 7.0;
/// the tread texture's mean (assets/textures/tyre/colour.png, 37.6 of 255), so a tread tinted
/// `tyre_rgb` averages that colour and meets the plain sidewalls
const TYRE_TEX_MEAN: f32 = 37.6 / 255.0;
/// one tile of the tread texture covers this many metres, square [design choice]
const TYRE_TILE: f32 = 0.4;

/// The barrel and tyre of a wheel (REMASTER, editable in `rimshop`). The defaults are the
/// constants above, so a style without a file keeps the mesh it always had.
#[derive(Clone, Debug, PartialEq)]
pub struct Barrel {
    /// rim face radius, fraction of the tyre radius
    pub rim: f32,
    /// how far the face centre sits inside the face edge, × half-width
    pub dish: f32,
    /// the face edge inside the tyre's outer wall, × half-width
    pub face_inset: f32,
    /// tyre bead above the rim radius, × radius
    pub bead: f32,
    /// tyre cross-section superellipse exponent
    pub tyre_shape: f32,
    /// the inner face's inset, × half-width
    pub inner_inset: f32,
    /// where the flat outer ring starts, fraction of the rim radius
    pub lip_ring: f32,
    /// edge chamfer of the flange and star plate, metres
    pub chamfer: f32,
    /// height of the barrel floor behind the spokes, × half-width
    pub floor: f32,
    pub tyre_rgb: [u8; 3],
    /// the face's depth profile as (t = radius / rim radius, depth × half-width, corner) points
    /// sorted by t; a corner (1) ends one smooth run and starts the next, so the curve is sharp
    /// there, and two corners at one t make a vertical step (#337); `None`: the power law
    /// `dish * (1 - t)^1.5`
    pub dish_curve: Option<Vec<[f32; 3]>>,
}

impl Default for Barrel {
    fn default() -> Self {
        Barrel { rim: RIM, dish: DISH, face_inset: FACE_INSET, bead: BEAD, tyre_shape: TYRE_SHAPE, inner_inset: INNER_INSET, lip_ring: LIP_RING, chamfer: 0.004, floor: 0.05, tyre_rgb: TYRE_RGB, dish_curve: None }
    }
}

impl Barrel {
    /// the face surface's depth below the face edge at `t` (0 hub .. 1 rim), metres
    pub fn depth(&self, h: f32, t: f32) -> f32 {
        match &self.dish_curve {
            Some(c) if c.len() >= 2 => dish_at(c, t) * h,
            _ => self.dish * h * (1.0 - t).powf(1.5),
        }
    }

    /// the t of every corner inside the face (0 < t < 1), sorted, a step once
    pub fn corners(&self) -> Vec<f32> {
        let mut v: Vec<f32> = self.dish_curve.iter().flatten().filter(|p| p[2] != 0.0 && p[0] > 0.0 && p[0] < 1.0).map(|p| p[0]).collect();
        v.sort_by(f32::total_cmp);
        v.dedup();
        v
    }
}

/// The smooth runs of a dish curve: its points split at the corners (a corner point ends a run
/// and starts the next).
pub fn dish_runs(c: &[[f32; 3]]) -> Vec<Vec<(f32, f32)>> {
    let mut runs: Vec<Vec<(f32, f32)>> = vec![Vec::new()];
    for (i, p) in c.iter().enumerate() {
        if let Some(r) = runs.last_mut() {
            r.push((p[0], p[1]));
        }
        if p[2] != 0.0 && i > 0 && i + 1 < c.len() {
            runs.push(vec![(p[0], p[1])]);
        }
    }
    runs
}

/// The dish curve at `t`: one `curve` through all points when none is a corner (bit for bit as
/// before corners existed), else the last run that starts at or before `t` (right-continuous at a
/// step).
pub fn dish_at(c: &[[f32; 3]], t: f32) -> f32 {
    if c.iter().all(|p| p[2] == 0.0) {
        return curve(&c.iter().map(|p| (p[0], p[1])).collect::<Vec<_>>(), t);
    }
    let runs = dish_runs(c);
    let run = runs.iter().rev().find(|r| r[0].0 <= t).unwrap_or(&runs[0]);
    curve(run, t)
}

/// Smooth curve through points sorted by x (cubic Hermite, finite-difference tangents); flat
/// outside the first and last point.
pub fn curve(pts: &[(f32, f32)], x: f32) -> f32 {
    let n = pts.len();
    if n == 0 {
        return 0.0;
    }
    if n == 1 || x <= pts[0].0 {
        return pts[0].1;
    }
    if x >= pts[n - 1].0 {
        return pts[n - 1].1;
    }
    let i = pts.windows(2).position(|w| x < w[1].0).unwrap_or(n - 2);
    let slope = |a: usize, b: usize| { let dx = pts[b].0 - pts[a].0; if dx.abs() < 1e-9 { 0.0 } else { (pts[b].1 - pts[a].1) / dx } };
    let (m0, m1) = (slope(i.saturating_sub(1), i + 1), slope(i, (i + 2).min(n - 1)));
    let (p0, p1) = (pts[i], pts[i + 1]);
    let dx = p1.0 - p0.0;
    if dx.abs() < 1e-9 {
        return p0.1;
    }
    let t = (x - p0.0) / dx;
    let (t2, t3) = (t * t, t * t * t);
    (2.0 * t3 - 3.0 * t2 + 1.0) * p0.1 + (t3 - 2.0 * t2 + t) * dx * m0 + (-2.0 * t3 + 3.0 * t2) * p1.1 + (t3 - t2) * dx * m1
}

/// How far a profile may stray from its polyline before it needs another point: the chord sag of
/// the ring (`SEGMENTS`) at radius `r`, so profile and ring are equally fine (#218).
fn ring_tol(r: f32) -> f32 {
    r * (1.0 - (std::f32::consts::PI / SEGMENTS as f32).cos())
}

/// Ramer-Douglas-Peucker: the indices (sorted, both ends always kept) of the points that keep the
/// polyline within `tol` of all of `pts`, distance measured in all `N` components.
pub fn simplify<const N: usize>(pts: &[[f32; N]], tol: f32) -> Vec<usize> {
    let n = pts.len();
    if n <= 2 {
        return (0..n).collect();
    }
    // distance from p to the segment a-b
    let dist = |p: &[f32; N], a: &[f32; N], b: &[f32; N]| -> f32 {
        let (mut ab2, mut apab) = (0.0, 0.0);
        for k in 0..N {
            let d = b[k] - a[k];
            ab2 += d * d;
            apab += (p[k] - a[k]) * d;
        }
        let t = if ab2 < 1e-18 { 0.0 } else { (apab / ab2).clamp(0.0, 1.0) };
        (0..N).map(|k| { let d = p[k] - (a[k] + t * (b[k] - a[k])); d * d }).sum::<f32>().sqrt()
    };
    let mut keep = vec![false; n];
    keep[0] = true;
    keep[n - 1] = true;
    let mut stack = vec![(0, n - 1)];
    while let Some((i, j)) = stack.pop() {
        if j <= i + 1 {
            continue;
        }
        let (k, d) = (i + 1..j).map(|k| (k, dist(&pts[k], &pts[i], &pts[j]))).fold((i + 1, -1.0), |m, x| if x.1 > m.1 { x } else { m });
        if d > tol {
            keep[k] = true;
            stack.push((i, k));
            stack.push((k, j));
        }
    }
    (0..n).filter(|&i| keep[i]).collect()
}

/// The rim face's look, from the original wheel's textured polygons.
#[derive(Clone, Copy, Debug)]
pub struct WheelFace {
    /// texture array layer of the alloy
    pub layer: f32,
    /// u and v as `a * y + b * z + c` of a point on the original face (wheel-local game axes)
    pub fit: [[f32; 3]; 2],
    /// face colour of the textured polygons (0..1), the texture is multiplied in
    pub colour: [f32; 4],
    /// lip colour: the face colour times the texture's mean on its outer ring
    pub lip: [f32; 4],
}

/// The wheel in game axes: x the axle, y up, z forward; rims drawn on `side` (-1: -x, +1: +x).
#[derive(Clone, Debug)]
pub struct WheelSpec {
    pub radius: f32,
    pub half_width: f32,
    pub side: f32,
    pub face: Option<WheelFace>,
    /// a 3D rim in place of the textured face (needs `face`)
    pub rim: Option<Rim>,
    /// ObjectBrightness, multiplied into the untextured colours like `scene::emit`
    pub scale: [f32; 3],
    /// REMASTER: the first texture layer of the tread's rubber (colour, then normal + roughness;
    /// `scene::Layers::tyre`); `None`: a flat tyre
    pub tyre: Option<f32>,
}

/// Least-squares affine map from (y, z) to uv over the original face's vertices. `None`
/// when the samples don't span the face.
pub fn fit_face(samples: &[([f32; 2], [f32; 2])]) -> Option<[[f32; 3]; 2]> {
    // normal equations: sum(p p^T) x = sum(p t), p = (y, z, 1)
    let mut m = Mat3::ZERO;
    let (mut bu, mut bv) = (Vec3::ZERO, Vec3::ZERO);
    for &([y, z], [u, v]) in samples {
        let p = Vec3::new(y, z, 1.0);
        m += Mat3::from_cols(p * p.x, p * p.y, p * p.z);
        bu += p * u;
        bv += p * v;
    }
    if m.determinant().abs() < 1e-9 {
        return None;
    }
    let inv = m.inverse();
    Some([(inv * bu).into(), (inv * bv).into()])
}

/// One profile point of the lathe: distance out along the rim side `o` (half-width at the
/// tyre wall), radius `r`, and what it is.
#[derive(Clone, Copy, PartialEq)]
enum Part {
    Face,
    Lip,
    Tyre,
    Inner,
}

/// Build a wheel centred on its hub. The tyre's cross-section is a superellipse reaching
/// exactly ±half-width and ±radius, so hub placement, contact and skid-mark width are the
/// original's (`scene::build_car` measures them on the MYO mesh).
pub fn wheel(w: &WheelSpec) -> Vec<Vertex> {
    let (r, h) = (w.radius, w.half_width);
    let b = w.rim.as_ref().map(|r| r.style.barrel.clone()).unwrap_or_default();
    let rim = b.rim * r;
    let bead = rim + b.bead * r;
    // tyre section: centre radius and half-height, so the top reaches r
    let rc = (bead + r) * 0.5 - 0.02 * r;
    let hh = r - rc;
    let e = 2.0 / b.tyre_shape;
    let sp = |t: f32| t.signum() * t.abs().powf(e);
    let tyre_at = |th: f32| Vec2::new(h * sp(th.cos()), rc + hh * sp(th.sin()));
    // angle where the cross-section comes down to the bead
    let th0 = -((rc - bead) / hh).clamp(0.0, 1.0).powf(b.tyre_shape / 2.0).asin();
    let lin = |a: f32, b: f32, n: usize| (0..=n).map(move |i| a + (b - a) * i as f32 / n as f32);
    // tyre angles in six sections: th0..0 (outer wall), then the crown 0..pi/4..pi/2..3pi/4..pi, then
    // pi..pi-th0 (inner wall). The break angles always stay: 0, pi/2, pi give the exact bounding
    // box, pi/4 and 3pi/4 are the tread's edges (#218). Each section is sampled densely and thinned
    // to the ring's chord sag (`simplify`); the inner wall, in the arch, at twice that.
    let pi = std::f32::consts::PI;
    let tol = ring_tol(r);
    let breaks = [th0, 0.0, pi / 4.0, pi / 2.0, pi * 0.75, pi, pi - th0];
    let mut angles = vec![th0];
    let mut tread = (0, 0);
    for (s, ab) in breaks.windows(2).enumerate() {
        if (ab[1] - ab[0]).abs() > 1e-6 {
            let dense: Vec<f32> = lin(ab[0], ab[1], 16).collect();
            let pts: Vec<[f32; 2]> = dense.iter().map(|&a| tyre_at(a).to_array()).collect();
            angles.extend(simplify(&pts, if s == 5 { 2.0 * tol } else { tol }).into_iter().skip(1).map(|k| dense[k]));
        }
        match s {
            1 => tread.0 = angles.len() - 1,
            3 => tread.1 = angles.len() - 1,
            _ => {}
        }
    }
    let tyre: Vec<Vec2> = angles.iter().map(|&a| tyre_at(a)).collect();
    let (outer_bead, inner_bead) = (tyre[0], *tyre.last().unwrap());
    // the face: a shallow dish from the hub to the rim radius
    let face_edge = h * (1.0 - b.face_inset);
    let dense: Vec<[f32; 2]> = lin(0.0, 1.0, 32).map(|t| [face_edge - b.depth(h, t), t * rim]).collect();
    let face: Vec<Vec2> = simplify(&dense, tol).into_iter().map(|i| Vec2::from(dense[i])).collect();
    // the lip: from the face edge out and up to the bead
    let lip = vec![Vec2::new(face_edge, rim), Vec2::new(outer_bead.x + 0.01 * h, (rim + outer_bead.y) * 0.5), outer_bead];
    // the inner side: a dark disc closing the wheel
    let inner_o = -h * (1.0 - b.inner_inset);
    let inner = vec![inner_bead, Vec2::new(inner_o, inner_bead.y), Vec2::new(inner_o, 0.0)];

    let rgba = |c: [u8; 3]| [0, 1, 2].map(|i| (c[i] as f32 * w.scale[i] / 255.0).min(1.0));
    let tyre_c = { let c = rgba(b.tyre_rgb); [c[0], c[1], c[2], 1.0] };
    let inner_c = { let c = rgba(b.tyre_rgb.map(|x| x / 2)); [c[0], c[1], c[2], 1.0] };
    let grey = { let c = rgba([94; 3]); [c[0], c[1], c[2], 1.0] };
    let mut out = Vec::new();
    let rim3d = w.rim.as_ref().zip(w.face);
    for (part, prof) in [(Part::Face, &face), (Part::Lip, &lip), (Part::Tyre, &tyre), (Part::Inner, &inner)] {
        if part == Part::Face && rim3d.is_some() {
            continue;
        }
        if let (Part::Tyre, Some(layer)) = (part, w.tyre) {
            // only the contact patch takes the rubber texture: the crown, 45 degrees (about 0.8 of
            // the half-width) either side of the top (the profile points at pi/4 and 3pi/4); the
            // shoulders and sidewalls stay plain. One profile, so the lighting is smooth across the edges
            let (t0, t1) = tread;
            let tread_c = { let c = rgba(b.tyre_rgb).map(|x| (x / TYRE_TEX_MEAN).min(2.0)); [c[0], c[1], c[2], 1.0] };
            let cols = vec![tyre_c; tyre.len()];
            strip_ex(&mut out, w, &tyre, &cols, false, 0..t0, None);
            strip_ex(&mut out, w, &tyre, &cols, false, t1..tyre.len() - 1, None);
            strip_ex(&mut out, w, &tyre, &vec![tread_c; tyre.len()], false, t0..t1, Some(layer));
            continue;
        }
        let colour = match (part, w.face) {
            (Part::Face, Some(f)) => f.colour,
            (Part::Lip, Some(f)) => f.lip,
            (Part::Face | Part::Lip, None) => grey,
            (Part::Tyre, _) => tyre_c,
            (Part::Inner, _) => inner_c,
        };
        let textured = part == Part::Face && w.face.is_some();
        let first = out.len();
        strip(&mut out, w, prof, &vec![colour; prof.len()], textured);
        if part == Part::Lip && rim3d.is_some() {
            out[first..].iter_mut().for_each(|v| v.env = METAL_ENV);
        }
    }
    if let Some((rim_spec, f)) = rim3d {
        let top = |m: f32| face_edge - b.depth(h, (m / rim).clamp(0.0, 1.0));
        rim_3d(&mut out, w, rim_spec, f, rim, h, &top);
    }
    out
}

/// Sweep one profile strip around the axle with smooth normals inside the strip (the strips
/// meet at creases).
fn strip(out: &mut Vec<Vertex>, w: &WheelSpec, prof: &[Vec2], colours: &[[f32; 4]], textured: bool) {
    strip_ex(out, w, prof, colours, textured, 0..prof.len() - 1, None);
}

/// `strip` over the segments `span` of the profile (the normals still come from all of it), with
/// the tread texture on layer `tread` (uv: across the profile and around the wheel in whole tiles).
fn strip_ex(out: &mut Vec<Vertex>, w: &WheelSpec, prof: &[Vec2], colours: &[[f32; 4]], textured: bool, span: std::ops::Range<usize>, tread: Option<f32>) {
    // distance along the profile from the span's start, and tiles round the wheel
    let mut across = vec![0.0f32; prof.len()];
    for i in span.start + 1..prof.len() {
        across[i] = across[i - 1] + prof[i].distance(prof[i - 1]);
    }
    let around = (std::f32::consts::TAU * w.radius / TYRE_TILE).round().max(1.0);
    // profile normals: (dr, -do) points out of the wheel for every strip as they are laid out
    let seg: Vec<Vec2> = prof.windows(2).map(|p| { let d = p[1] - p[0]; Vec2::new(d.y, -d.x).normalize_or_zero() }).collect();
    let pn: Vec<Vec2> = (0..prof.len()).map(|i| (seg.get(i.wrapping_sub(1)).copied().unwrap_or(Vec2::ZERO) + seg.get(i).copied().unwrap_or(Vec2::ZERO)).normalize_or(Vec2::X)).collect();
    let (face, s) = (w.face.filter(|_| textured), w.side);
    // the original's texture coordinates of a face point, scaled so RIM_TEXTURE lands on the rim
    let k = RIM_TEXTURE / RIM;
    let vert = |i: usize, j: usize| -> Vertex {
        let a = j as f32 / SEGMENTS as f32 * std::f32::consts::TAU;
        let (sa, ca) = a.sin_cos();
        let (p, n) = (prof[i], pn[i]);
        let game = Vec3::new(s * p.x, p.y * sa, p.y * ca);
        let normal = Vec3::new(s * n.x, n.y * sa, n.y * ca);
        let (uv, layer) = match face {
            _ if tread.is_some() => ([across[i] / TYRE_TILE, j as f32 / SEGMENTS as f32 * around], tread.unwrap_or(-1.0)),
            Some(f) => {
                let (y, z) = (game.y * k, game.z * k);
                let at = |c: [f32; 3]| c[0] * y + c[1] * z + c[2];
                ([at(f.fit[0]), at(f.fit[1])], f.layer)
            }
            None => ([0.0; 2], -1.0),
        };
        Vertex { pos: [game.x, game.y, -game.z], uv, color: colours[i], layer, normal: [normal.x, normal.y, -normal.z], env: if tread.is_some() { TYRE_ENV } else { 0.0 } }
    };
    for i in span {
        for j in 0..SEGMENTS {
            let (a, b, c, d) = (vert(i, j), vert(i, j + 1), vert(i + 1, j + 1), vert(i + 1, j));
            for t in [[a, b, c], [a, c, d]] {
                let p = t.map(|v| Vec3::from(v.pos));
                let cross = (p[1] - p[0]).cross(p[2] - p[0]);
                if cross.length_squared() < 1e-14 {
                    continue; // collapsed at the hub
                }
                let n: Vec3 = t.iter().map(|v| Vec3::from(v.normal)).sum();
                out.extend(if cross.dot(n) < 0.0 { [t[0], t[2], t[1]] } else { t });
            }
        }
    }
}

/// REMASTER 3D rim shape, in fractions of the rim radius [design choice; proportions measured
/// on the car's alloy texture]. A centre cap on a star plate with `spokes` points and groups of
/// spokes running out to the outer flange; the gaps show the backing.
#[derive(Clone, Debug, PartialEq)]
pub struct RimStyle {
    /// the base count: star points, and the pitch the spoke groups count in
    pub spokes: usize,
    /// centre cap radius
    pub cap: f32,
    /// the star plate: valley radius, point radius, width of the (flat) point
    pub star: [f32; 3],
    /// the spokes (#337); empty: star plate only
    pub groups: Vec<SpokeGroup>,
    /// discs with windows cut out, full or grooved covers (#337); empty: none
    pub windows: Vec<WindowPlate>,
    /// lug nuts on the hub face (#337); `None`: none
    pub lugs: Option<Lugs>,
    /// rivets round the lip (#337); `None`: none
    pub rivets: Option<Rivets>,
    /// the centre cap's outline: 0 round, else a polygon with that many sides (3..=12)
    pub cap_sides: usize,
    /// the centre cap's dome height, × half-width (0 = flat)
    pub cap_dome: f32,
    /// inner radius of the outer flange: a shoulder rising to the lip at `LIP_RING`
    pub flange: f32,
    /// brake caliper colour (sRGB bytes) behind the spokes (9.22 phase 5)
    pub caliper: [u8; 3],
    /// where the caliper sits round the axle (y = r sin a, z = r cos a) and its half-span in
    /// radians; a span of 0 leaves the caliper out
    pub caliper_at: f32,
    pub caliper_span: f32,
    /// depth of the star plate's wall, × half-width
    pub star_depth: f32,
    pub barrel: Barrel,
}

/// How a spoke group draws each spoke: once, or with a mirrored copy.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Mirror {
    #[default]
    None,
    /// a copy with the lateral offset's sign flipped: a pair beside the centre line (the Titan's
    /// twin bars)
    Twin,
    /// a copy with the sweep and the offset flipped: an X lace (the copy sits 1 mm lower)
    Cross,
}

/// A spoke's cross-section, outer face up.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Section {
    /// a flat top with small chamfers (the original bars)
    #[default]
    Flat,
    /// a half-round top
    Round,
    /// a sharp peak along the middle, two sloped faces
    Ridge,
    /// a flat top with wide 45-degree bevels
    Facet,
}

impl Mirror {
    pub const ALL: [Mirror; 3] = [Mirror::None, Mirror::Twin, Mirror::Cross];
}

impl Section {
    pub const ALL: [Section; 4] = [Section::Flat, Section::Round, Section::Ridge, Section::Facet];
}

/// A group of identical spokes (#337): `multiple` spokes per base pitch (`RimStyle::spokes`),
/// turned by `phase` (a fraction of the base pitch). Each spoke runs along its points
/// `[radius, sweep, offset, width, height]`: radius, offset and width are fractions of the rim
/// radius, sweep turns the centre line round the axle (radians, + with the spoke angle), offset
/// moves it sideways, height lifts the top off the dish (× half-width). Points are sorted by
/// radius; `smooth` runs a Hermite curve through them in radius (`curve`), else straight
/// segments.
#[derive(Clone, Debug, PartialEq)]
pub struct SpokeGroup {
    pub multiple: usize,
    pub phase: f32,
    pub points: Vec<[f32; 5]>,
    pub mirror: Mirror,
    pub section: Section,
    /// wall depth, × half-width
    pub depth: f32,
    pub smooth: bool,
    /// the spokes run over the flange and lip ring out to their tip (radius up to 1) instead of
    /// ending under the ring (#337)
    pub onto_lip: bool,
}

/// One spoke of a group at a point along it (`SpokeGroup::at`), fractions of the rim radius.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SpokeAt {
    pub r: f32,
    /// angle off the spoke's base angle (radians)
    pub sweep: f32,
    pub off: f32,
    pub width: f32,
    /// × half-width
    pub lift: f32,
    /// angle between the spoke's path and the radius, from the sweep alone: the cross-section
    /// turns by it so a swept spoke keeps its width
    pub tilt: f32,
}

impl SpokeAt {
    /// The point `side` half-widths across the centre line (-1, 0, 1) as (along the spoke's
    /// swept radius, across it) in rim fractions.
    pub fn across(&self, side: f32) -> (f32, f32) {
        let u = self.off + side * self.width * 0.5;
        let (s, c) = self.tilt.sin_cos();
        (self.r - u * s, u * c)
    }
}

impl SpokeGroup {
    /// A straight bar (the v1 `Bar`): from, to (radius), width, offset at the root and at the
    /// lip; a positive lip offset makes a twin pair, else one bar on the centre line.
    pub fn from_bar(bar: [f32; 5], depth: f32) -> Self {
        let [f0, f1, bw, o0, o1] = bar;
        let (mirror, o0, o1) = if o1 > 0.0 { (Mirror::Twin, o0, o1) } else { (Mirror::None, 0.0, 0.0) };
        SpokeGroup { multiple: 1, phase: 0.0, points: vec![[f0, 0.0, o0, bw, 0.0], [f1, 0.0, o1, bw, 0.0]], mirror, section: Section::Flat, depth, smooth: false, onto_lip: false }
    }

    /// A v1 spoke curve: (radius, offset, width) points, smooth, optionally twinned.
    pub fn from_curve(points: &[[f32; 3]], twin: bool, depth: f32) -> Self {
        SpokeGroup {
            multiple: 1,
            phase: 0.0,
            points: points.iter().map(|p| [p[0], 0.0, p[1], p[2], 0.0]).collect(),
            mirror: if twin { Mirror::Twin } else { Mirror::None },
            section: Section::Flat,
            depth,
            smooth: true,
            onto_lip: false,
        }
    }

    /// the sign of each drawn copy: offset (and for `Cross` the sweep) times it
    pub fn copies(&self) -> &'static [f32] {
        match self.mirror {
            Mirror::None => &[1.0],
            Mirror::Twin | Mirror::Cross => &[-1.0, 1.0],
        }
    }

    /// Column `k` of the points at `t` (0 root .. 1 tip): a Hermite curve in radius when
    /// `smooth`, else straight segments parametrised in t, so two points give `a + (b - a) t`
    /// exactly (the v1 bar).
    fn column(&self, t: f32, k: usize) -> f32 {
        let p = &self.points;
        let n = p.len();
        let (r0, r1) = (p[0][0], p[n - 1][0]);
        if self.smooth {
            return curve(&p.iter().map(|q| (q[0], q[k])).collect::<Vec<_>>(), r0 + (r1 - r0) * t);
        }
        let span = r1 - r0;
        let tk = |i: usize| if span.abs() < 1e-9 { 0.0 } else { (p[i][0] - r0) / span };
        let i = (0..n - 1).find(|&i| t <= tk(i + 1)).unwrap_or(n - 2);
        let (ta, tb) = (tk(i), tk(i + 1));
        let s = if tb - ta < 1e-9 { 0.0 } else { (t - ta) / (tb - ta) };
        p[i][k] + (p[i + 1][k] - p[i][k]) * s
    }

    /// The spoke at `t` (0 root .. 1 tip) of the copy `sign` (`copies`); needs two points. The
    /// radius is linear in t, the rest follows the points. The v1 bars and curves come out bit
    /// for bit: sweep, lift and tilt are exactly 0 when their columns are.
    pub fn at(&self, t: f32, sign: f32) -> SpokeAt {
        let p = &self.points;
        let (r0, r1) = (p[0][0], p[p.len() - 1][0]);
        let r = r0 + (r1 - r0) * t;
        let any = |k: usize| p.iter().any(|q| q[k] != 0.0);
        let ssign = if self.mirror == Mirror::Cross { sign } else { 1.0 };
        let (sweep, tilt) = if any(1) {
            // r dθ/dr by a central difference in t
            let (ta, tb) = ((t - 1e-3).max(0.0), (t + 1e-3).min(1.0));
            let dr = (r1 - r0) * (tb - ta);
            let tilt = if dr.abs() < 1e-9 { 0.0 } else { (ssign * r * (self.column(tb, 1) - self.column(ta, 1)) / dr).atan() };
            (ssign * self.column(t, 1), tilt)
        } else {
            (0.0, 0.0)
        };
        let lift = if any(4) { self.column(t, 4) } else { 0.0 };
        SpokeAt { r, sweep, off: sign * self.column(t, 2), width: self.column(t, 3), lift, tilt }
    }
}

impl Default for RimStyle {
    /// the Titan's
    fn default() -> Self {
        // Titan, bluefel0.iff [O: radial and angular profiles of the texture]: solid centre to
        // 0.25, a five-pointed star with valleys at 0.37 whose points split into twin parallel
        // bars (about 0.07 wide, ±0.10 off centre) running out over a darker shoulder (0.75 to
        // 0.88) to the bright lip. Drawn with the star at half that size (the user's call): the
        // bars start together inside its points and spread to ±0.11 at the lip, a little wider
        // than the texture's so they hold up at a distance; the cap is a small raised centre.
        RimStyle {
            spokes: 5,
            cap: 0.06,
            star: [0.185, 0.28, 0.20],
            groups: vec![SpokeGroup::from_bar([0.24, 0.93, 0.09, 0.05, 0.11], 0.12)],
            windows: Vec::new(),
            lugs: None,
            rivets: None,
            cap_sides: 0,
            cap_dome: 0.0,
            flange: 0.76,
            caliper: TITAN_CALIPER,
            caliper_at: CALIPER_AT,
            caliper_span: CALIPER_SPAN,
            star_depth: 0.15,
            barrel: Barrel::default(),
        }
    }
}

/// The built-in 3D rim of car `car` (`Game/carNN.car`), if it has one. Cars without one keep
/// the textured face. `rimfile::load` prefers a file made in `rimshop`.
pub fn rim_style(car: u32) -> Option<RimStyle> {
    match car {
        10 => Some(RimStyle::default()),
        _ => None,
    }
}

/// the Titan's calipers: blue, after the user's reference render (9.22 phase 5)
const TITAN_CALIPER: [u8; 3] = [30, 60, 200];

/// where the rim's flat outer ring starts (fraction of the rim radius) [O: the alloys' bright
/// ring from about 0.88]
const LIP_RING: f32 = 0.9;

/// `Vertex::env` of polished rim metal: Remaster shades it as polished aluminium (shader.wgsl
/// `METAL_F0`); generated meshes are never drawn in Classic
pub const METAL_ENV: f32 = 3.0;
/// `Vertex::env` of self-lit lamp parts (lenses, LED bars): drawn in their own colour, unlit
pub const LAMP_ENV: f32 = 4.0;

/// `Vertex::env` of a brake caliper: it must not spin with the wheel, so `vs_remaster` and
/// `vs_shadow` turn it back by the wheel's spin angle (the car uniform's `spot_n.z`) and shade
/// it as plain lit paint
pub const CALIPER_ENV: f32 = 5.0;
/// `Vertex::env` of carbon fibre (generated parts): the texture layer is the weave's colour,
/// the next layer its normal (rgb) and roughness (a), shaded under a clear coat
pub const CARBON_ENV: f32 = 6.0;

/// one carbon weave tile (the texture layer) covers this many metres [design choice: about
/// 4 mm tows, a 2×2 twill]
pub const CARBON_TILE: f32 = 0.04;
/// vertex colour of carbon parts, times the weave texture (mean about 0.18) [design choice]
pub const CARBON_TINT: [f32; 4] = [0.7, 0.7, 0.72, 1.0];

/// Carbon weave uv of a point (game space) on a surface facing `n`: flat on the plane it faces
/// most (x/y from behind or ahead, z/y from the side, x/z from above or below).
pub fn carbon_uv(p: Vec3, n: Vec3) -> [f32; 2] {
    let a = n.abs();
    let (u, v) = if a.z >= a.x && a.z >= a.y { (p.x, -p.y) } else if a.x >= a.y { (p.z, -p.y) } else { (p.x, p.z) };
    [u / CARBON_TILE, v / CARBON_TILE]
}

/// Rings of `RimLook::backing` from the hub (0) to the rim (1).
pub const BACKING_RINGS: usize = 16;

/// What the texture analysis reads off a wheel's alloy (`analyse`); colours are texture
/// colours 0..1, to be multiplied by the face colour.
#[derive(Clone, Copy, Debug)]
pub struct RimLook {
    /// the strongest angular harmonic from 3 to 12 at 0.5..0.7 of the rim radius
    pub spokes: usize,
    /// wheel angle (y = r sin a, z = r cos a) of the first spoke's centre
    pub phase: f32,
    /// mean of the brightest quarter of the face
    pub metal: [f32; 3],
    /// the colour in the middle of the gaps between spokes, per ring
    pub backing: [[f32; 3]; BACKING_RINGS],
}

/// A wheel's 3D rim: its shape and what its texture says.
#[derive(Clone, Debug)]
pub struct Rim {
    pub style: RimStyle,
    pub look: RimLook,
}

/// Read the spoke count and angle, the metal colour and the colour seen through the gaps off
/// the alloy texture. `sample(y, z)` is the texture colour at a point of the original face
/// (wheel-local game axes, metres); `spokes` overrides the detected count for the phase.
pub fn analyse(radius: f32, sample: &dyn Fn(f32, f32) -> [f32; 3], spokes: Option<usize>) -> RimLook {
    const N: usize = 256;
    let tau = std::f32::consts::TAU;
    // f: fraction of the rim radius on the generated wheel, which is RIM_TEXTURE of the
    // original wheel's radius on its face
    let at = |f: f32, a: f32| sample(f * RIM_TEXTURE * radius * a.sin(), f * RIM_TEXTURE * radius * a.cos());
    let luma = |c: [f32; 3]| 0.299 * c[0] + 0.587 * c[1] + 0.114 * c[2];
    let rings: Vec<Vec<f32>> = [0.5, 0.6, 0.7].iter().map(|&f| (0..N).map(|k| luma(at(f, k as f32 / N as f32 * tau))).collect()).collect();
    // the n-th Fourier coefficient summed over the rings
    let coeff = |n: usize| -> Vec2 {
        rings.iter().flat_map(|ring| {
            let m = ring.iter().sum::<f32>() / N as f32;
            ring.iter().enumerate().map(move |(k, &b)| {
                let a = n as f32 * k as f32 / N as f32 * tau;
                Vec2::new(a.cos(), -a.sin()) * (b - m)
            })
        }).sum()
    };
    let detected = (3..=12).max_by(|&a, &b| coeff(a).length().total_cmp(&coeff(b).length())).unwrap_or(5);
    let n = spokes.unwrap_or(detected);
    // b(a) ~ cos(n (a - a0)) gives a coefficient along -n a0
    let c = coeff(n);
    let phase = -c.y.atan2(c.x) / n as f32;
    let mut face: Vec<[f32; 3]> = (1..=20).flat_map(|i| (0..64).map(move |k| (i as f32 / 20.0, k as f32 / 64.0 * tau))).map(|(f, a)| at(f, a)).collect();
    face.sort_by(|a, b| luma(*b).total_cmp(&luma(*a)));
    let top = &face[..face.len() / 4];
    let metal = [0, 1, 2].map(|i| top.iter().map(|c| c[i]).sum::<f32>() / top.len() as f32);
    let backing = std::array::from_fn(|i| {
        let f = i as f32 / (BACKING_RINGS - 1) as f32;
        let gaps: Vec<[f32; 3]> = (0..n).map(|k| at(f, phase + (k as f32 + 0.5) * tau / n as f32)).collect();
        [0, 1, 2].map(|c| gaps.iter().map(|g| g[c]).sum::<f32>() / n as f32)
    });
    RimLook { spokes: detected, phase, metal, backing }
}

/// One rim vertex in the wheel's own axes with the rim on +x (mirrored to `side` on output).
#[derive(Clone, Copy)]
struct V {
    p: Vec3,
    n: Vec3,
    c: [f32; 4],
}

/// Emit a triangle: mirror to the wheel's side, to view space, wound to face along its normals.
fn tri(out: &mut Vec<Vertex>, side: f32, t: [V; 3]) {
    let m = |v: Vec3| Vec3::new(side * v.x, v.y, -v.z);
    let vs = t.map(|v| Vertex { pos: m(v.p).into(), uv: [0.0; 2], color: v.c, layer: -1.0, normal: m(v.n).into(), env: 0.0 });
    let p = vs.map(|v| Vec3::from(v.pos));
    let cross = (p[1] - p[0]).cross(p[2] - p[0]);
    if cross.length_squared() < 1e-14 {
        return;
    }
    let n: Vec3 = vs.iter().map(|v| Vec3::from(v.normal)).sum();
    out.extend(if cross.dot(n) < 0.0 { [vs[0], vs[2], vs[1]] } else { vs });
}

/// The rim's metal and backing (09-remaster 9.20 "3D rims"): the parts overlap rather than
/// being cut into one surface; their tops are offset by a few millimetres so the one meant to
/// show wins the depth test (cap over arms over bars, flange over bars).
fn rim_3d(out: &mut Vec<Vertex>, w: &WheelSpec, rim: &Rim, face: WheelFace, rr: f32, h: f32, top: &dyn Fn(f32) -> f32) {
    let (st, look) = (&rim.style, &rim.look);
    let tint = |c: [f32; 3]| [face.colour[0] * c[0], face.colour[1] * c[1], face.colour[2] * c[2], 1.0];
    let metal = tint(look.metal);
    let tau = std::f32::consts::TAU;
    let tol = ring_tol(w.radius);
    // the barrel's floor behind the spokes
    let back = st.barrel.floor * h;
    // the floor, coloured like the texture's gaps ring by ring
    // (sampled at 16 rings, thinned to the ones where the colour bends)
    let dense: Vec<Vec2> = (0..=16).map(|i| Vec2::new(back, st.flange * rr * i as f32 / 16.0)).collect();
    let dense_cols: Vec<[f32; 4]> = dense.iter().map(|p| {
        let x = (p.y / rr).clamp(0.0, 1.0) * (BACKING_RINGS - 1) as f32;
        let (i, t) = (x.floor() as usize, x.fract());
        let (a, b) = (look.backing[i], look.backing[(i + 1).min(BACKING_RINGS - 1)]);
        tint([0, 1, 2].map(|c| a[c] + (b[c] - a[c]) * t))
    }).collect();
    let pts: Vec<[f32; 4]> = dense_cols.iter().enumerate().map(|(i, c)| [i as f32 / 16.0, c[0], c[1], c[2]]).collect();
    let keep = simplify(&pts, 0.02);
    let floor: Vec<Vec2> = keep.iter().map(|&i| dense[i]).collect();
    let cols: Vec<[f32; 4]> = keep.iter().map(|&i| dense_cols[i]).collect();
    strip(out, w, &floor, &cols, false);
    // everything after the floor is polished metal
    let first = out.len();
    // the cap, its wall down into the star
    let cap = [Vec2::new(top(0.0) + 0.004, 0.0), Vec2::new(top(0.6 * st.cap * rr) + 0.004, 0.6 * st.cap * rr), Vec2::new(top(st.cap * rr), st.cap * rr), Vec2::new(top(st.cap * rr) - 0.2 * h, st.cap * rr)];
    if st.cap_sides == 0 && st.cap_dome == 0.0 {
        strip(out, w, &cap, &[metal; 4], false);
    } else {
        rimdetail::cap(out, w, st, look.phase, rr, h, top, metal);
    }
    // the flange: a wall up from the floor, a shoulder sloping up to the flat ring, the ring
    // out to the lip (the bars run up the shoulder and end under the ring); a dish with a corner
    // out there makes a stepped lip instead
    let ch = st.barrel.chamfer;
    let flange = Flange::new(st, rr, h, back, ring_tol(w.radius), top);
    for f in &flange.strips {
        strip(out, w, f, &vec![metal; f.len()], false);
    }
    // a spoke that runs onto the lip rides over the flange
    let lip_top = |m: f32| top(m).max(flange.at(m) + 0.004);
    let breaks = st.barrel.corners();
    star(out, w.side, rr, top, look.phase, st, st.star_depth * h, metal);
    // the window plates (#337), 1 mm under the star and over the spokes; the star plate covers
    // them inside 0.9 of its valley radius (or the cap)
    let hidden = 0.9 * st.star[0].max(st.cap) * rr;
    for p in &st.windows {
        rimplate::plate(out, w.side, rr, h, top, look.phase, p, ch, hidden, metal);
    }
    // the spoke groups (#337), each spoke along its points in as many steps as its shape needs
    let n = st.spokes;
    for g in st.groups.iter().filter(|g| g.points.len() >= 2 && g.multiple > 0) {
        let m = n * g.multiple;
        // the Cross copy 1 mm under the other, so the crossing does not z-fight [design choice]
        let copies: Vec<(f32, f32, Vec<f32>)> = g.copies().iter().map(|&sign| {
            let drop = if g.mirror == Mirror::Cross && sign < 0.0 { 0.003 } else { 0.002 };
            let t: &dyn Fn(f32) -> f32 = if g.onto_lip { &lip_top } else { top };
            (sign, drop, bar_steps(rr, h, t, &|t| g.at(t, sign), tol, &breaks))
        }).collect();
        for k in 0..m {
            let mut a = look.phase + k as f32 * tau / m as f32;
            if g.phase != 0.0 {
                a += g.phase * tau / n as f32;
            }
            for (sign, drop, ts) in &copies {
                let t: &dyn Fn(f32) -> f32 = if g.onto_lip { &lip_top } else { top };
                bar(out, w.side, rr, h, t, a, g.section, &|t| g.at(t, *sign), ts, g.depth * h, *drop, g.onto_lip, metal);
            }
        }
    }
    // the details (#337): lug nuts on the hub, rivets on the lip
    if let Some(l) = &st.lugs {
        rimdetail::lugs(out, w.side, rr, h, top, look.phase, l, ch, metal);
    }
    if let Some(r) = &st.rivets {
        rimdetail::rivets(out, w.side, rr, &|m| flange.at(m), look.phase, r, metal);
    }
    out[first..].iter_mut().for_each(|v| v.env = METAL_ENV);
    let first = out.len();
    let col = [0, 1, 2].map(|i| (st.caliper[i] as f32 * w.scale[i] / 255.0).min(1.0));
    if st.caliper_span > 0.0 {
        caliper(out, w.side, rr, h, [col[0], col[1], col[2], 1.0], st.caliper_at, st.caliper_span);
    }
    out[first..].iter_mut().for_each(|v| v.env = CALIPER_ENV);
}

/// The flange as lathe strips (offset out, radius; metres) and its top surface from the foot to
/// the lip as a polyline (`at`). Without a corner out there it is the shoulder and the flat ring
/// of old; with one (a stepped lip, #337) it follows the dish +2 mm from the foot to the rim,
/// in pieces cut at the corners, each step a chamfer and a vertical riser.
struct Flange {
    strips: Vec<Vec<Vec2>>,
    surf: Vec<Vec2>,
}

impl Flange {
    fn new(st: &RimStyle, rr: f32, h: f32, back: f32, tol: f32, top: &dyn Fn(f32) -> f32) -> Self {
        let (fl, ring, ch) = (st.flange * rr, st.barrel.lip_ring * rr, st.barrel.chamfer);
        const EPS: f32 = 1e-5;
        let mut cuts: Vec<f32> = st.barrel.corners().iter().map(|t| t * rr).filter(|&m| m > fl + EPS && m < rr - EPS).collect();
        if cuts.is_empty() {
            let p = vec![Vec2::new(back, fl), Vec2::new(top(fl) - 0.25 * h, fl), Vec2::new(top(ring) + 0.002 - ch, ring - ch), Vec2::new(top(ring) + 0.002, ring), Vec2::new(top(rr), rr)];
            return Flange { surf: p[1..].to_vec(), strips: vec![p] };
        }
        cuts.insert(0, fl);
        cuts.push(rr);
        // the pieces between the cuts, each sampled on its own side of the steps and thinned
        let off = |m: f32| 0.002 * ((rr - m) / (0.03 * rr)).clamp(0.0, 1.0);
        let pieces: Vec<Vec<Vec2>> = cuts.windows(2).map(|ab| {
            let (a, b) = (ab[0], ab[1]);
            let dense: Vec<[f32; 2]> = (0..=12).map(|i| {
                let m = a + (b - a) * i as f32 / 12.0;
                [top(m.clamp(a + EPS, b - EPS)) + off(m), m]
            }).collect();
            simplify(&dense, tol).into_iter().map(|i| Vec2::from(dense[i])).collect()
        }).collect();
        let surf: Vec<Vec2> = pieces.iter().flatten().copied().collect();
        let mut strips = vec![vec![Vec2::new(back, fl), Vec2::new(pieces[0][0].x, fl)]];
        let mut pieces = pieces;
        let mut risers: Vec<Vec<Vec<Vec2>>> = Vec::new();
        for k in 0..pieces.len() - 1 {
            let (lo, hi) = (pieces[k].last().unwrap().x, pieces[k + 1][0].x);
            let c = cuts[k + 1];
            let dh = hi - lo;
            let mut r = Vec::new();
            if dh.abs() > 1e-4 {
                let cc = ch.min(0.5 * dh.abs());
                if dh < 0.0 {
                    // the inner tier is higher: its edge is chamfered, the riser drops outward
                    pieces[k].last_mut().unwrap().y = c - cc;
                    r.push(vec![Vec2::new(lo, c - cc), Vec2::new(lo - cc, c)]);
                    r.push(vec![Vec2::new(lo - cc, c), Vec2::new(hi, c)]);
                } else {
                    pieces[k + 1][0].y = c + cc;
                    r.push(vec![Vec2::new(lo, c), Vec2::new(hi - cc, c)]);
                    r.push(vec![Vec2::new(hi - cc, c), Vec2::new(hi, c + cc)]);
                }
            }
            risers.push(r);
        }
        for (k, piece) in pieces.into_iter().enumerate() {
            strips.push(piece);
            if let Some(r) = risers.get_mut(k) {
                strips.append(r);
            }
        }
        Flange { strips, surf }
    }

    /// the surface's offset at radius `m` (right-continuous at a step); very low under the foot
    fn at(&self, m: f32) -> f32 {
        let s = &self.surf;
        if m < s[0].y {
            return f32::MIN;
        }
        let i = s.iter().rposition(|p| p.y <= m).unwrap_or(0);
        match s.get(i + 1) {
            Some(q) if q.y > s[i].y => s[i].x + (q.x - s[i].x) * (m - s[i].y) / (q.y - s[i].y),
            _ => s[i].x,
        }
    }
}

/// default caliper position round the axle (y = r sin a, z = r cos a): above and behind it
const CALIPER_AT: f32 = 2.1;
/// its half-span round the axle (radians)
const CALIPER_SPAN: f32 = 0.36;

/// A brake caliper in the barrel behind the spokes: an arc block from 0.48 to 0.72 of the rim
/// radius (inside the flange) and from just off the floor to under the bars, edges chamfered.
/// It is drawn with the wheel but not spun (`CALIPER_ENV`).
fn caliper(out: &mut Vec<Vertex>, side: f32, rr: f32, h: f32, c: [f32; 4], at_angle: f32, span: f32) {
    const STEPS: usize = 8;
    let (r0, r1, x0, x1) = (0.48 * rr, 0.72 * rr, 0.08 * h, 0.45 * h);
    let ch = 0.004;
    // the cross-section (x out, r) round the block, with its smooth normals
    let sec = [(x0, r0), (x1 - ch, r0), (x1, r0 + ch), (x1, r1 - ch), (x1 - ch, r1), (x0, r1)];
    let nrm = [(0.0, -1.0), (0.38, -0.92), (0.92, -0.38), (0.92, 0.38), (0.38, 0.92), (0.0, 1.0)];
    let at = |a: f32, x: f32, r: f32| Vec3::new(x, r * a.sin(), r * a.cos());
    let ring = |a: f32| -> Vec<V> {
        let er = Vec3::new(0.0, a.sin(), a.cos());
        sec.iter().zip(nrm).map(|(&(x, r), (nx, nr))| V { p: at(a, x, r), n: (Vec3::X * nx + er * nr).normalize(), c }).collect()
    };
    let rings: Vec<Vec<V>> = (0..=STEPS).map(|i| ring(at_angle - span + 2.0 * span * i as f32 / STEPS as f32)).collect();
    for i in 0..STEPS {
        for j in 0..sec.len() - 1 {
            let (p, q, r, s) = (rings[i][j], rings[i][j + 1], rings[i + 1][j + 1], rings[i + 1][j]);
            tri(out, side, [p, q, r]);
            tri(out, side, [p, r, s]);
        }
    }
    // the two ends, flat
    for (a, sign) in [(at_angle - span, -1.0), (at_angle + span, 1.0)] {
        let n = Vec3::new(0.0, a.cos(), -a.sin()) * sign;
        let v: Vec<V> = sec.iter().map(|&(x, r)| V { p: at(a, x, r), n, c }).collect();
        for k in 1..v.len() - 1 {
            tri(out, side, [v[0], v[k], v[k + 1]]);
        }
    }
}

/// The star plate: one outline through the valleys and the flat points, a dished top, a
/// chamfered edge and a wall `depth` deep.
#[allow(clippy::too_many_arguments)]
fn star(out: &mut Vec<Vertex>, side: f32, rr: f32, top: &dyn Fn(f32) -> f32, phase: f32, st: &RimStyle, depth: f32, c: [f32; 4]) {
    let ch = st.barrel.chamfer;
    let n = st.spokes;
    let [valley, tip, tip_w] = st.star;
    let pitch = std::f32::consts::TAU / n as f32;
    let er = |a: f32| Vec2::new(a.sin(), a.cos());
    let et = |a: f32| Vec2::new(a.cos(), -a.sin());
    // corners in increasing angle, (y, z) in metres
    let corners: Vec<Vec2> = (0..n).flat_map(|k| {
        let a = phase + k as f32 * pitch;
        [er(a - pitch / 2.0) * valley * rr, er(a) * tip * rr - et(a) * tip_w * rr / 2.0, er(a) * tip * rr + et(a) * tip_w * rr / 2.0]
    }).collect();
    // edges split so the top follows the dish
    const SPLIT: usize = 2;
    let outline: Vec<Vec2> = (0..corners.len()).flat_map(|i| {
        let (p, q) = (corners[i], corners[(i + 1) % corners.len()]);
        (0..SPLIT).map(move |j| p.lerp(q, j as f32 / SPLIT as f32))
    }).collect();
    let m = outline.len();
    // outward normal in the plane at each outline point (corners averaged)
    let edge_n = |i: usize| { let d = outline[(i + 1) % m] - outline[i]; Vec2::new(d.y, -d.x).normalize_or_zero() };
    let nrm: Vec<Vec2> = (0..m).map(|i| (edge_n((i + m - 1) % m) + edge_n(i)).normalize_or_zero()).collect();
    // keep the winding's outward side outward whichever way the angle runs
    let sign = if nrm[0].dot(outline[0]) < 0.0 { -1.0 } else { 1.0 };
    let v = |q: Vec2, lift: f32, n: Vec3| V { p: Vec3::new(top(q.length()) + lift, q.x, q.y), n: n.normalize(), c };
    let rings: Vec<Vec<V>> = [0.0, 0.5, 1.0]
        .iter()
        .map(|&t| (0..m).map(|i| v((outline[i] - nrm[i] * sign * ch) * t, 0.0, Vec3::X)).collect())
        .chain([
            (0..m).map(|i| { let e = nrm[i] * sign; v(outline[i] - e * ch, 0.0, Vec3::new(0.92, e.x * 0.38, e.y * 0.38)) }).collect(),
            (0..m).map(|i| { let e = nrm[i] * sign; v(outline[i], -ch, Vec3::new(0.38, e.x * 0.92, e.y * 0.92)) }).collect(),
            (0..m).map(|i| { let e = nrm[i] * sign; v(outline[i], -depth, Vec3::new(0.0, e.x, e.y)) }).collect(),
        ])
        .collect();
    for r in rings.windows(2) {
        for i in 0..m {
            let j = (i + 1) % m;
            tri(out, side, [r[0][i], r[0][j], r[1][j]]);
            tri(out, side, [r[0][i], r[1][j], r[1][i]]);
        }
    }
}

/// The `t` values (0..1 along a bar) it needs: sampled 32 times, thinned so the bar's radius,
/// offset, width, dished top, sweep (as arc length) and lift stay within `tol`. The top counts
/// three times, because a chord over the dish stands proud of it and the bars sit only 2 mm
/// under the star. (The sweep and lift columns come last: zero columns leave the choice of the
/// v1 bars as it was.) `breaks` are radii (rim fractions) where the dish steps: the samples
/// get a pair either side of each.
fn bar_steps(rr: f32, h: f32, top: &dyn Fn(f32) -> f32, at: &dyn Fn(f32) -> SpokeAt, tol: f32, breaks: &[f32]) -> Vec<f32> {
    const DENSE: usize = 32;
    let mut ts: Vec<f32> = (0..=DENSE).map(|i| i as f32 / DENSE as f32).collect();
    let (r0, r1) = (at(0.0).r, at(1.0).r);
    if (r1 - r0).abs() > 1e-6 {
        for &b in breaks {
            let tb = (b - r0) / (r1 - r0);
            if tb > 0.0 && tb < 1.0 {
                ts.extend([(tb - 0.002).max(0.0), (tb + 0.002).min(1.0)]);
            }
        }
        ts.sort_by(f32::total_cmp);
        ts.dedup();
    }
    let pts: Vec<[f32; 6]> = ts.iter().map(|&t| { let s = at(t); [s.r * rr, s.off * rr, s.width * rr, 3.0 * top(s.r * rr), s.sweep * s.r * rr, s.lift * h] }).collect();
    simplify(&pts, tol).into_iter().map(|i| ts[i]).collect()
}

/// A spoke's cross-section as (across, up, normal across, normal up) points from the left foot
/// over the top to the right foot; `up` 0 is the top. A point given twice with two normals is a
/// sharp crease (the zero-width quads between them are dropped by `tri`).
fn spoke_section(sec: Section, hw: f32, depth: f32) -> Vec<(f32, f32, f32, f32)> {
    let wall = |u: f32, v: f32, s: f32| (u, v, s, 0.0);
    match sec {
        Section::Flat => {
            // CLASSIC-era generated bar (9.20): chamfers at most 6 mm, smooth normals
            let ch = (0.35 * hw).min(0.006);
            let prof = [(-hw, -depth), (-hw, -ch), (-hw + ch, 0.0), (hw - ch, 0.0), (hw, -ch), (hw, -depth)];
            let norms = [(-1.0, 0.0), (-0.92, 0.38), (-0.38, 0.92), (0.38, 0.92), (0.92, 0.38), (1.0, 0.0)];
            prof.iter().zip(norms).map(|(&(u, v), (nu, nv))| (u, v, nu, nv)).collect()
        }
        Section::Round => {
            // a half ellipse hw wide and up to hw high, walls below it
            let ry = hw.min(0.8 * depth);
            let mut p = vec![wall(-hw, -depth, -1.0)];
            p.extend((0..=6).map(|i| {
                let th = std::f32::consts::PI * (1.0 - i as f32 / 6.0);
                let (s, c) = th.sin_cos();
                let n = Vec2::new(c / hw, s / ry).normalize();
                (hw * c, -ry + ry * s, n.x, n.y)
            }));
            p.push(wall(hw, -depth, 1.0));
            p
        }
        Section::Ridge => {
            let e = (0.6 * hw).min(0.8 * depth);
            let n = Vec2::new(-e, hw).normalize();
            vec![wall(-hw, -depth, -1.0), wall(-hw, -e, -1.0), (-hw, -e, n.x, n.y), (0.0, 0.0, n.x, n.y), (0.0, 0.0, -n.x, n.y), (hw, -e, -n.x, n.y), wall(hw, -e, 1.0), wall(hw, -depth, 1.0)]
        }
        Section::Facet => {
            let b = (0.35 * hw).min(0.8 * depth);
            let d = std::f32::consts::FRAC_1_SQRT_2;
            vec![
                wall(-hw, -depth, -1.0), wall(-hw, -b, -1.0),
                (-hw, -b, -d, d), (-hw + b, 0.0, -d, d),
                (-hw + b, 0.0, 0.0, 1.0), (hw - b, 0.0, 0.0, 1.0),
                (hw - b, 0.0, d, d), (hw, -b, d, d),
                wall(hw, -b, 1.0), wall(hw, -depth, 1.0),
            ]
        }
    }
}

/// A spoke across the rim face from base angle `a`; `at(t)` (t 0..1 along it) gives its radius,
/// sweep, offset, width, lift and tilt (`SpokeAt`) at the `ts` given, `depth` deep, its top
/// following the dish `top` less `drop` plus the lift, its cross-section `sec` square to its
/// path. Both ends are buried (in the star and under the lip ring), unless `cap`: the tip is on
/// the lip and closed with a flat end.
#[allow(clippy::too_many_arguments)]
fn bar(out: &mut Vec<Vertex>, side: f32, rr: f32, h: f32, top: &dyn Fn(f32) -> f32, a: f32, sec: Section, at: &dyn Fn(f32) -> SpokeAt, ts: &[f32], depth: f32, drop: f32, cap: bool, c: [f32; 4]) {
    let frame = |a: f32| { let (sa, ca) = a.sin_cos(); (Vec3::new(0.0, sa, ca), Vec3::new(0.0, ca, -sa)) };
    let base = frame(a);
    let ring = |t: f32| -> Vec<V> {
        let s = at(t);
        // the swept frame and the cross-section's direction (exactly the unswept one at 0)
        let (er, et) = if s.sweep == 0.0 { base } else { frame(a + s.sweep) };
        let et = if s.tilt == 0.0 { et } else { et * s.tilt.cos() - er * s.tilt.sin() };
        let r = s.r * rr;
        let hw = s.width * rr * 0.5;
        spoke_section(sec, hw, depth).into_iter().map(|(u, v, nu, nv)| {
            let q = er * r + et * (s.off * rr + u);
            let mut o = top(q.length()) - drop;
            if s.lift != 0.0 {
                o += s.lift * h;
            }
            V { p: Vec3::new(o + v, q.y, q.z), n: (et * nu + Vec3::X * nv).normalize(), c }
        }).collect()
    };
    let rings: Vec<Vec<V>> = ts.iter().map(|&t| ring(t)).collect();
    for i in 0..ts.len() - 1 {
        for j in 0..rings[i].len() - 1 {
            let (p, q, r, s) = (rings[i][j], rings[i][j + 1], rings[i + 1][j + 1], rings[i + 1][j]);
            tri(out, side, [p, q, r]);
            tri(out, side, [p, r, s]);
        }
    }
    if cap && rings.len() >= 2 {
        let mid = |r: &[V]| r.iter().fold(Vec3::ZERO, |s, v| s + v.p) / r.len() as f32;
        let (a, b) = (&rings[rings.len() - 2], &rings[rings.len() - 1]);
        let n = (mid(b) - mid(a)).normalize_or_zero();
        let end: Vec<V> = b.iter().map(|v| V { n, ..*v }).collect();
        for k in 1..end.len() - 1 {
            tri(out, side, [end[0], end[k], end[k + 1]]);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bbox(v: &[Vertex]) -> (Vec3, Vec3) {
        v.iter().fold((Vec3::splat(f32::MAX), Vec3::splat(f32::MIN)), |(lo, hi), v| (lo.min(v.pos.into()), hi.max(v.pos.into())))
    }

    /// the generated wheel fills exactly the original's [-hw, hw] × [-r, r] × [-r, r]
    #[test]
    fn wheel_bbox_matches_original() {
        for (r, hw, side) in [(0.353, 0.153, -1.0), (0.353, 0.177, 1.0), (0.332, 0.145, 1.0)] {
            let v = wheel(&WheelSpec { radius: r, half_width: hw, side, face: None, rim: None, scale: [1.0; 3], tyre: None });
            let (lo, hi) = bbox(&v);
            let want = Vec3::new(hw, r, r);
            assert!((hi - want).abs().max_element() < 1e-5 && (lo + want).abs().max_element() < 1e-5, "{lo} {hi} vs {want}");
        }
    }

    /// the Titan rim builds, every triangle faces along its normals, the size stays the original's
    #[test]
    fn rim_builds() {
        let style = rim_style(10).unwrap();
        // a five-spoke test pattern: bright along a = 0.3 + k 72°
        let sample = |y: f32, z: f32| [(5.0 * (y.atan2(z) - 0.3)).cos().max(0.0); 3];
        let look = analyse(0.353, &sample, None);
        assert_eq!(look.spokes, 5);
        assert!((look.phase - 0.3).abs() < 0.02, "{}", look.phase);
        let face = WheelFace { layer: 0.0, fit: [[1.0, 0.0, 0.5], [0.0, 1.0, 0.5]], colour: [0.4, 0.4, 0.4, 1.0], lip: [0.3, 0.3, 0.3, 1.0] };
        let spec = WheelSpec { radius: 0.353, half_width: 0.153, side: -1.0, face: Some(face), rim: Some(Rim { style: style.clone(), look }), scale: [1.0; 3], tyre: None };
        let v = wheel(&spec);
        let (lo, hi) = bbox(&v);
        assert!((hi - Vec3::new(0.153, 0.353, 0.353)).abs().max_element() < 1e-5 && (lo + Vec3::new(0.153, 0.353, 0.353)).abs().max_element() < 1e-5);
        assert!(v.len() / 3 < 3000, "{} triangles", v.len() / 3);
        for t in v.chunks(3) {
            let p = t.iter().map(|v| Vec3::from(v.pos)).collect::<Vec<_>>();
            let n: Vec3 = t.iter().map(|v| Vec3::from(v.normal)).sum();
            assert!((p[1] - p[0]).cross(p[2] - p[0]).dot(n) > 0.0);
        }
        // the caliper sits inside the flange, behind the spokes, above and behind the axle
        let cal: Vec<&Vertex> = v.iter().filter(|v| v.env == CALIPER_ENV).collect();
        assert!(!cal.is_empty());
        for p in &cal {
            let (x, y, z) = (-p.pos[0], p.pos[1], -p.pos[2]);
            assert!(Vec2::new(y, z).length() < style.flange * RIM * 0.353 + 1e-4, "{y} {z}");
            assert!((0.0..=0.5 * 0.153).contains(&x), "{x}");
            assert!(y > 0.0 && z < 0.0, "{y} {z}");
        }
        println!("titan wheel: {} triangles", v.len() / 3);
    }

    /// FNV-1a over the vertex bytes of the Titan rim and a plain wheel: guards refactors of the
    /// generator (the default style must keep producing the same mesh; hashes renewed when the tyre
    /// colour went from 18 to 38, the tread texture's mean, and for 32 segments + adaptive profiles, #218)
    #[test]
    fn titan_mesh_is_stable() {
        let fnv = |v: &[Vertex]| bytemuck::cast_slice::<Vertex, u8>(v).iter().fold(0xcbf29ce484222325u64, |h, &b| (h ^ b as u64).wrapping_mul(0x100000001b3));
        let sample = |y: f32, z: f32| [(5.0 * (y.atan2(z) - 0.3)).cos().max(0.0); 3];
        let look = analyse(0.353, &sample, None);
        let face = WheelFace { layer: 0.0, fit: [[1.0, 0.0, 0.5], [0.0, 1.0, 0.5]], colour: [0.4, 0.4, 0.4, 1.0], lip: [0.3, 0.3, 0.3, 1.0] };
        let spec = WheelSpec { radius: 0.353, half_width: 0.153, side: -1.0, face: Some(face), rim: Some(Rim { style: rim_style(10).unwrap(), look }), scale: [1.0; 3], tyre: None };
        let plain = WheelSpec { rim: None, face: None, ..spec.clone() };
        assert_eq!((fnv(&wheel(&spec)), fnv(&wheel(&plain))), (0x838442df59473d5f, 0x2b8936ede9b42ddc));
    }

    /// v1 rim files (#337 phase 1): the installed car09 / car10 rims, a twin spoke curve and a
    /// three-point curve keep the exact mesh they had before spoke groups (hashes taken on the
    /// old `bar` / `SpokeCurve` generator). The floats are hashed rounded to 1e-4: the spline rim's
    /// last bits differ between libm implementations (Windows CI), the mesh does not
    #[test]
    fn legacy_rims_are_stable() {
        let fnv = |v: &[Vertex]| bytemuck::cast_slice::<Vertex, f32>(v).iter().fold(0xcbf29ce484222325u64, |h, &x| ((x * 1e4).round() as i32).to_le_bytes().iter().fold(h, |h, &b| (h ^ b as u64).wrapping_mul(0x100000001b3)));
        let car09 = "# Motorhead rim v1 (rimshop)\nSpokes 5\nCap 0.3\nStar 0.0502 0.3 0.22\nBar 0.05 0.93 0.3 0 0\nFlange 0.9\nCaliper 200 30 30\nCaliperAt 2.5\nCaliperSpan 0.6\nDepth 0.02 0.24\nRim 0.7\nDish 0.3\nFaceInset 0.32\nBead 0.1\nTyreShape 4.9\nInnerInset 0.55\nLipRing 0.98\nChamfer 0.006\nFloor 0.28\nTyre 38 38 38\n";
        let car10 = "# Motorhead rim v1 (rimshop)\nSpokes 5\nCap 0\nStar 0.185 0.28 0.2\nBar 0.24 0.93 0.09 0.05 0.11\nFlange 0.9\nCaliper 30 60 200\nCaliperAt 2.1\nCaliperSpan 0.36\nDepth 0.15 0.2\nRim 0.79\nDish 0.34\nFaceInset 0.17\nBead 0.05\nTyreShape 5.3\nInnerInset 0.88\nLipRing 0.98\nChamfer 0.0057\nFloor 0.3\nTyre 38 38 38\n";
        let twin = "Spokes 7\nSpokeTwin 1\nSpokePoint 0.24 0.035 0.055\nSpokePoint 0.93 0.07 0.055\nDishPoint 0 0.3\nDishPoint 0.5 0.12\nDishPoint 1 0\n";
        let curved = "Spokes 6\nCap 0.07\nStar 0.2 0.3 0.18\nSpokeTwin 0\nSpokePoint 0.2 0 0.11\nSpokePoint 0.5 0.12 0.08\nSpokePoint 0.93 0.28 0.07\n";
        let face = WheelFace { layer: 0.0, fit: [[1.0, 0.0, 0.5], [0.0, 1.0, 0.5]], colour: [0.4, 0.4, 0.4, 1.0], lip: [0.3, 0.3, 0.3, 1.0] };
        let got: Vec<u64> = [car09, car10, twin, curved].iter().map(|t| {
            let style = rimfile::from_text(t).unwrap();
            let look = RimLook { spokes: style.spokes, phase: 0.7, metal: [0.8; 3], backing: [[0.1; 3]; BACKING_RINGS] };
            fnv(&wheel(&WheelSpec { radius: 0.353, half_width: 0.153, side: -1.0, face: Some(face), rim: Some(Rim { style, look }), scale: [1.0; 3], tyre: Some(3.0) }))
        }).collect();
        assert_eq!(got, vec![0xcc01e853f2dc24aa, 0xa4691b99084d3291, 0x91fbca40fe47272b, 0x83fafdc73d29adfe]);
    }

    /// every mirror and section kind with sweep, lift and a phase builds facing out inside the
    /// original box; a swept spoke really turns, its cross-section keeps the width (#337)
    #[test]
    fn spoke_groups_build() {
        let face = WheelFace { layer: 0.0, fit: [[1.0, 0.0, 0.5], [0.0, 1.0, 0.5]], colour: [0.4; 4], lip: [0.3; 4] };
        for mirror in Mirror::ALL {
            for section in Section::ALL {
                for smooth in [false, true] {
                    let g = SpokeGroup { multiple: 2, phase: 0.3, points: vec![[0.2, 0.0, 0.0, 0.08, 0.0], [0.6, 0.4, 0.02, 0.06, 0.05], [0.9, 1.1, 0.03, 0.05, -0.05]], mirror, section, depth: 0.1, smooth, onto_lip: false };
                    let style = RimStyle { groups: vec![g.clone()], ..Default::default() };
                    let look = RimLook { spokes: 5, phase: 0.2, metal: [0.8; 3], backing: [[0.1; 3]; BACKING_RINGS] };
                    let v = wheel(&WheelSpec { radius: 0.353, half_width: 0.153, side: 1.0, face: Some(face), rim: Some(Rim { style, look }), scale: [1.0; 3], tyre: None });
                    let (lo, hi) = bbox(&v);
                    let want = Vec3::new(0.153, 0.353, 0.353);
                    assert!((hi - want).abs().max_element() < 1e-5 && (lo + want).abs().max_element() < 1e-5, "{mirror:?} {section:?}");
                    for t in v.chunks(3) {
                        let p = t.iter().map(|v| Vec3::from(v.pos)).collect::<Vec<_>>();
                        let n: Vec3 = t.iter().map(|v| Vec3::from(v.normal)).sum();
                        assert!((p[1] - p[0]).cross(p[2] - p[0]).dot(n) > 0.0, "{mirror:?} {section:?}");
                    }
                    // 20 spokes here: twice a usual rim
                    assert!(v.len() / 3 < 6000, "{mirror:?} {section:?}: {} triangles", v.len() / 3);
                    let tip = g.at(1.0, 1.0);
                    assert!((tip.sweep - 1.1).abs() < 1e-5 && tip.tilt > 0.5, "{tip:?}");
                    if mirror == Mirror::Cross {
                        assert!((g.at(1.0, -1.0).sweep + 1.1).abs() < 1e-5);
                    }
                    // the edge points sit a full width apart square to the path
                    let s = g.at(0.5, 1.0);
                    let (a, b) = (s.across(-1.0), s.across(1.0));
                    assert!(((a.0 - b.0).hypot(a.1 - b.1) - s.width).abs() < 1e-5);
                }
            }
        }
    }

    /// a dish corner is sharp, two at one t are a vertical step, none is the old curve bit for bit (#337)
    #[test]
    fn dish_corners_and_steps() {
        let smooth = vec![[0.0, 0.4, 0.0], [0.5, 0.2, 0.0], [1.0, 0.0, 0.0]];
        let old: Vec<(f32, f32)> = smooth.iter().map(|p| (p[0], p[1])).collect();
        for i in 0..=20 {
            let t = i as f32 / 20.0;
            assert_eq!(dish_at(&smooth, t), curve(&old, t));
        }
        // one corner: continuous, but the slope jumps
        let sharp = vec![[0.0, 0.4, 0.0], [0.5, 0.2, 1.0], [1.0, 0.1, 0.0]];
        let slope = |t: f32| (dish_at(&sharp, t + 1e-3) - dish_at(&sharp, t - 1e-3)) / 2e-3;
        assert!((dish_at(&sharp, 0.5 - 1e-5) - dish_at(&sharp, 0.5 + 1e-5)).abs() < 1e-3);
        assert!((slope(0.499) - slope(0.501)).abs() > 0.05, "{} {}", slope(0.499), slope(0.501));
        // a step: 0.15 deep at t = 0.8, right-continuous
        let step = vec![[0.0, 0.4, 0.0], [0.8, 0.2, 1.0], [0.8, 0.05, 1.0], [1.0, 0.0, 0.0]];
        assert!((dish_at(&step, 0.8 - 1e-4) - dish_at(&step, 0.8 + 1e-4) - 0.15).abs() < 0.01);
        assert_eq!(dish_at(&step, 0.8), 0.05);
        let b = Barrel { dish_curve: Some(step), ..Default::default() };
        assert_eq!(b.corners(), vec![0.8]);
        assert!(Barrel::default().corners().is_empty());
    }

    fn build_with(style: RimStyle) -> Vec<Vertex> {
        let look = RimLook { spokes: style.spokes, phase: 0.2, metal: [0.8; 3], backing: [[0.1; 3]; BACKING_RINGS] };
        let face = WheelFace { layer: 0.0, fit: [[1.0, 0.0, 0.5], [0.0, 1.0, 0.5]], colour: [0.4; 4], lip: [0.3; 4] };
        wheel(&WheelSpec { radius: 0.353, half_width: 0.153, side: 1.0, face: Some(face), rim: Some(Rim { style, look }), scale: [1.0; 3], tyre: None })
    }

    fn assert_good(v: &[Vertex], what: &str) {
        let (lo, hi) = bbox(v);
        let want = Vec3::new(0.153, 0.353, 0.353);
        assert!((hi - want).abs().max_element() < 1e-5 && (lo + want).abs().max_element() < 1e-5, "{what}: box");
        for t in v.chunks(3) {
            let p = t.iter().map(|v| Vec3::from(v.pos)).collect::<Vec<_>>();
            let n: Vec3 = t.iter().map(|v| Vec3::from(v.normal)).sum();
            assert!((p[1] - p[0]).cross(p[2] - p[0]).dot(n) > 0.0, "{what}: winding");
        }
    }

    /// stepped lip, spokes onto the lip, rivets, lugs and shaped caps build inside the original
    /// box facing out; the stepped lip really has a riser; an onto-lip spoke reaches the rim
    #[test]
    fn details_and_lips_build() {
        let base = build_with(RimStyle::default());
        // lugs: the default hex ring adds its prisms and nothing else
        let lugs = build_with(RimStyle { lugs: Some(Lugs::default()), ..Default::default() });
        assert_good(&lugs, "lugs");
        assert_eq!(lugs.len() - base.len(), 5 * 6 * 5 * 3);
        let rivets = build_with(RimStyle { rivets: Some(Rivets::default()), ..Default::default() });
        assert_good(&rivets, "rivets");
        assert_eq!(rivets.len() - base.len(), 10 * 40 * 3);
        for (sides, dome) in [(0, 0.15), (6, 0.0), (6, 0.2), (5, 0.1)] {
            let v = build_with(RimStyle { cap: 0.1, cap_sides: sides, cap_dome: dome, ..Default::default() });
            assert_good(&v, &format!("cap {sides} {dome}"));
            assert_ne!(bytemuck::cast_slice::<Vertex, u8>(&v), bytemuck::cast_slice::<Vertex, u8>(&build_with(RimStyle { cap: 0.1, ..Default::default() })));
        }
        // a stepped lip: the face has an upright riser (normals square to the axle) out at 0.8
        let mut style = RimStyle::default();
        style.barrel.dish_curve = Some(vec![[0.0, 0.34, 0.0], [0.45, 0.26, 0.0], [0.8, 0.2, 1.0], [0.8, 0.05, 1.0], [1.0, 0.03, 0.0]]);
        style.flange = 0.7;
        style.groups[0].onto_lip = true;
        style.groups[0].points = vec![[0.2, 0.0, 0.0, 0.1, 0.0], [0.99, 0.0, 0.0, 0.07, 0.0]];
        let v = build_with(style.clone());
        assert_good(&v, "stepped lip");
        let rr = 0.353 * 0.7;
        let riser = v.iter().filter(|p| p.normal[0].abs() < 0.05 && (Vec2::new(p.normal[1], p.normal[2]).length() - 1.0).abs() < 0.05 && ((Vec2::new(p.pos[1], p.pos[2]).length() / rr) - 0.8).abs() < 0.005).count();
        assert!(riser > 20, "{riser}");
        // at the spoke tip (0.99 of the rim) the onto-lip spoke rides over the flange (and is
        // closed by an end cap); the buried one stays under it
        let band_top = |v: &[Vertex]| v.iter().filter(|p| p.env == METAL_ENV && (0.985..0.995).contains(&(Vec2::new(p.pos[1], p.pos[2]).length() / rr))).map(|p| p.pos[0]).fold(f32::MIN, f32::max);
        let onto = band_top(&v);
        style.groups[0].onto_lip = false;
        let under = build_with(style);
        assert!(onto > band_top(&under) + 0.001, "{onto} {}", band_top(&under));
        assert!(v.len() > under.len());
        println!("stepped lip {} triangles", v.len() / 3);
    }

    /// the plain textured-face wheel stays under its budget, and a straight line / an arc simplify
    /// as they should (#218)
    #[test]
    fn plain_wheel_is_light_and_simplify_works() {
        let v = wheel(&WheelSpec { radius: 0.353, half_width: 0.153, side: -1.0, face: None, rim: None, scale: [1.0; 3], tyre: None });
        assert!(v.len() / 3 < 1800, "{} triangles", v.len() / 3);
        println!("plain wheel: {} triangles", v.len() / 3);
        assert_eq!(simplify(&[[0.0, 0.0], [1.0, 1.0], [2.0, 2.0], [3.0, 3.0]], 1e-4), vec![0, 3]);
        let arc: Vec<[f32; 2]> = (0..=40).map(|i| { let a = i as f32 / 40.0 * 1.5; [a.cos(), a.sin()] }).collect();
        let keep = simplify(&arc, 0.01);
        assert!(keep.len() > 2 && keep.len() < arc.len());
        // every dropped point is within tol of the kept polyline (chord sag of a unit arc)
        for w in keep.windows(2) {
            for k in w[0]..=w[1] {
                let (a, b) = (Vec2::from(arc[w[0]]), Vec2::from(arc[w[1]]));
                let (p, d) = (Vec2::from(arc[k]), b - a);
                let t = ((p - a).dot(d) / d.length_squared()).clamp(0.0, 1.0);
                assert!((p - (a + d * t)).length() <= 0.01 + 1e-5);
            }
        }
    }

    /// the textured tread covers the crown from 45 to 135 degrees of the section, whatever the
    /// tyre's shape, and ends exactly there (#218)
    #[test]
    fn tread_spans_the_crown() {
        for shape in [3.2, 5.3] {
            let style = RimStyle { barrel: Barrel { tyre_shape: shape, ..Default::default() }, ..Default::default() };
            let look = RimLook { spokes: 5, phase: 0.0, metal: [0.8; 3], backing: [[0.1; 3]; BACKING_RINGS] };
            let face = WheelFace { layer: 0.0, fit: [[1.0, 0.0, 0.5], [0.0, 1.0, 0.5]], colour: [0.4; 4], lip: [0.3; 4] };
            let h = 0.153;
            let v = wheel(&WheelSpec { radius: 0.353, half_width: h, side: 1.0, face: Some(face), rim: Some(Rim { style, look }), scale: [1.0; 3], tyre: Some(3.0) });
            let x: Vec<f32> = v.iter().filter(|v| v.env == TYRE_ENV).map(|v| v.pos[0].abs()).collect();
            assert!(!x.is_empty());
            let edge = h * std::f32::consts::FRAC_1_SQRT_2.powf(2.0 / shape);
            let widest = x.iter().cloned().fold(0.0, f32::max);
            assert!((widest - edge).abs() < 1e-5, "shape {shape}: tread reaches {widest}, want {edge}");
        }
    }

    /// every triangle faces away from the hub on the tread and outwards on the rim side
    #[test]
    fn wheel_faces_out() {
        let v = wheel(&WheelSpec { radius: 0.35, half_width: 0.15, side: 1.0, face: None, rim: None, scale: [1.0; 3], tyre: None });
        for t in v.chunks(3) {
            let p = t.iter().map(|v| Vec3::from(v.pos)).collect::<Vec<_>>();
            let n: Vec3 = t.iter().map(|v| Vec3::from(v.normal)).sum();
            assert!((p[1] - p[0]).cross(p[2] - p[0]).dot(n) > 0.0);
        }
    }

    #[test]
    fn fit_recovers_affine_map() {
        let f = |y: f32, z: f32| [0.4 - 0.9 * y, 0.5 + 0.8 * z];
        let s: Vec<_> = [(0.0, 0.0), (0.3, 0.0), (0.0, 0.3), (-0.2, 0.1)].iter().map(|&(y, z)| ([y, z], f(y, z))).collect();
        let fit = fit_face(&s).unwrap();
        assert!((fit[0][0] + 0.9).abs() < 1e-4 && (fit[0][2] - 0.4).abs() < 1e-4 && (fit[1][1] - 0.8).abs() < 1e-4);
    }
}
