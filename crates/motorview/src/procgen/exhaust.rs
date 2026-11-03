//! REMASTER exhaust tips for the generated bodies (09-remaster 9.22 phase 5), after the user's
//! layout drawing and reference render: two mirrored tips side by side in the rear bumper's
//! recess, each with a straight inner side, 45° chamfers on three corners and an outer side
//! leaning in towards the top; every corner softened by a fillet (the user's call, after the
//! reference render); a polished rim, a dark inside. The
//! MYO has none; the Titan's bumper recess they sit in is [O] (phase 4), the rest a design choice.
//!
//! Input in car-local game space (the car's back faces -z); output in view space like
//! `body::body`.

use super::body::tri;
use super::lamps::vert;
use crate::scene::Vertex;
use glam::{Vec2, Vec3};

/// A pair of tips mirrored in x = 0, centred on `y`, their lip `z` (the most rearward point).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Exhaust {
    /// the inner (straight) side's distance from the centre line (m): half the gap
    pub x: f32,
    pub y: f32,
    pub z: f32,
    /// outer size (m): from the inner side to the outer side's foot, and the height
    pub width: f32,
    pub height: f32,
    /// the 45° chamfers on the inner corners and the lower outer one (m)
    pub chamfer: f32,
    /// how far the outer side leans in at the top (m)
    pub slant: f32,
    /// the fillet radius on every corner of the outline (m, at least `lip`)
    pub round: f32,
    /// rim thickness (m)
    pub lip: f32,
    /// how far the pipe runs forward into the body from the lip (m)
    pub pipe: f32,
    /// how deep the dark inside goes before the end plate (m)
    pub depth: f32,
}

/// polished steel, shaded as metal (`super::METAL_ENV`)
const STEEL: [f32; 4] = [0.75, 0.75, 0.78, 1.0];
/// sooty inside
const SOOT: [f32; 4] = [0.04, 0.035, 0.03, 1.0];

/// The +x tip's sharp outline (x, y round `e.y`), counter-clockwise, convex.
fn corners(e: &Exhaust) -> Vec<Vec2> {
    let (x0, x1, h, c) = (e.x, e.x + e.width, 0.5 * e.height, e.chamfer);
    vec![
        Vec2::new(x0, h - c),
        Vec2::new(x0, -h + c),
        Vec2::new(x0 + c, -h),
        Vec2::new(x1 - c, -h),
        Vec2::new(x1, -h + c),
        Vec2::new(x1 - e.slant, h),
        Vec2::new(x0 + c, h),
    ]
}

/// The outward normal of the edge a → b of a counter-clockwise outline.
fn edge_normal(a: Vec2, b: Vec2) -> Vec2 {
    let d = b - a;
    Vec2::new(d.y, -d.x).normalize()
}

/// A convex counter-clockwise outline moved in by `by`, point for point (mitred corners).
fn inset(p: &[Vec2], by: f32) -> Vec<Vec2> {
    let n = p.len();
    (0..n)
        .map(|i| {
            let (a, b) = (edge_normal(p[(i + n - 1) % n], p[i]), edge_normal(p[i], p[(i + 1) % n]));
            let m = (a + b).normalize();
            p[i] - m * (by / m.dot(a))
        })
        .collect()
}

/// an outline: points with their outward normals
type Outline = Vec<(Vec2, Vec2)>;

/// radians per fillet segment (at most)
const FILLET_STEP: f32 = std::f32::consts::PI / 12.0;
/// the longest straight piece of an outline (m), and the spacing of the end plate's lattice
const STEP: f32 = 0.02;
/// the most a wall's quad is taller than wide: its triangles keep an angle over 7° (the mesh
/// check's slivers are under 5°, `wire::SLIVER_DEG`)
const ASPECT: f32 = 8.0;

/// `p` with each corner rounded to radius `r` round the centre `inset(p, by)` (the outline's
/// own fillet centre for `by` = the outline's radius), as points with their outward normals;
/// the straight runs between the fillets cut every `STEP` at most (the 6 mm rim along a 21 cm
/// side was a pair of slivers). Outlines of the same `p` correspond point for point, so the rim
/// between them is even.
fn rounded(p: &[Vec2], centres: &[Vec2], r: f32) -> Outline {
    let n = p.len();
    let mut out = Vec::new();
    for i in 0..n {
        let j = (i + 1) % n;
        let (a, b) = (edge_normal(p[(i + n - 1) % n], p[i]), edge_normal(p[i], p[j]));
        let turn = a.angle_to(b);
        let steps = (turn / FILLET_STEP).ceil().max(1.0) as usize;
        for k in 0..=steps {
            let nn = Vec2::from_angle(turn * k as f32 / steps as f32).rotate(a);
            out.push((centres[i] + nn * r, nn));
        }
        // the run to the next fillet is as long as the centres are apart, whatever `r`
        let (from, to) = (centres[i] + b * r, centres[j] + b * r);
        let cuts = (centres[i].distance(centres[j]) / STEP).ceil() as usize;
        for k in 1..cuts {
            out.push((from.lerp(to, k as f32 / cuts as f32), b));
        }
    }
    out
}

/// The +x tip's outer and inner (rim's inside) outlines, point for point.
fn outlines(e: &Exhaust) -> (Outline, Outline) {
    let p = corners(e);
    let r = e.round.max(e.lip);
    let centres = inset(&p, r);
    (rounded(&p, &centres, r), rounded(&p, &centres, r - e.lip))
}

/// The tips: per tip an outer pipe wall (metal) from the lip forward, the lip's flat ring
/// facing back, the inner wall (soot) and its end plate. The walls shade smooth round the
/// fillets; they are cut into rings along the pipe so their quads keep within `ASPECT` of the
/// fillets' short segments, and the end plate is a lattice (`smooth::flat_mesh`): its fan from
/// the centre was all slivers.
pub fn exhausts(e: &Exhaust) -> Vec<Vertex> {
    let mut v = Vec::new();
    let (right_out, right_in) = outlines(e);
    let m = super::METAL_ENV;
    let (zl, zf, zd) = (e.z, e.z + e.pipe, e.z + e.depth);
    for sx in [1.0, -1.0] {
        // the -x tip is the mirror image (`tri` sorts out the winding)
        let mirror = |q: &(Vec2, Vec2)| (Vec2::new(sx * q.0.x, q.0.y), Vec2::new(sx * q.1.x, q.1.y));
        let outer: Outline = right_out.iter().map(mirror).collect();
        let inner: Outline = right_in.iter().map(mirror).collect();
        let at = |p: Vec2, z: f32| Vec3::new(p.x, e.y + p.y, z);
        // the outer pipe wall, and the inside with its normals inwards
        let mut wall = |ring: &Outline, z1: f32, c: [f32; 4], env: f32, sign: f32| {
            let n = ring.len();
            let short = (0..n).map(|i| ring[i].0.distance(ring[(i + 1) % n].0)).filter(|&d| d > 1e-4).fold(f32::MAX, f32::min);
            let rows = ((z1 - zl).abs() / (ASPECT * short)).ceil().max(1.0) as usize;
            let side = |q: (Vec2, Vec2), r: usize| vert(at(q.0, zl + (z1 - zl) * r as f32 / rows as f32), Vec3::new(q.1.x, q.1.y, 0.0) * sign, c, env);
            for r in 0..rows {
                for i in 0..n {
                    let (q0, q1) = (ring[i], ring[(i + 1) % n]);
                    tri(&mut v, [side(q0, r), side(q1, r), side(q1, r + 1)]);
                    tri(&mut v, [side(q0, r), side(q1, r + 1), side(q0, r + 1)]);
                }
            }
        };
        wall(&outer, zf, STEEL, m, 1.0);
        wall(&inner, zd, SOOT, 0.0, -1.0);
        // the lip's ring, facing back
        let back = |p: Vec2| vert(at(p, zl), Vec3::NEG_Z, STEEL, m);
        let n = outer.len();
        for i in 0..n {
            let j = (i + 1) % n;
            tri(&mut v, [back(outer[i].0), back(outer[j].0), back(inner[j].0)]);
            tri(&mut v, [back(outer[i].0), back(inner[j].0), back(inner[i].0)]);
        }
        // the end plate, facing back (a fillet of no radius left its points on one spot)
        let mut ring: Vec<Vec3> = Vec::new();
        for q in &inner {
            let p = at(q.0, zd);
            if ring.last().is_none_or(|l| l.distance(p) > 1e-4) {
                ring.push(p);
            }
        }
        if ring.len() > 2 && ring[0].distance(ring[ring.len() - 1]) <= 1e-4 {
            ring.pop();
        }
        let (pts, tris) = super::smooth::flat_mesh(&ring, Vec3::NEG_Z, STEP);
        for t in tris {
            tri(&mut v, t.map(|k| vert(pts[k].0, Vec3::NEG_Z, SOOT, 0.0)));
        }
    }
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    const E: Exhaust = Exhaust { x: 0.025, y: 0.5, z: -2.03, width: 0.26, height: 0.11, chamfer: 0.02, slant: 0.05, round: 0.012, lip: 0.006, pipe: 0.08, depth: 0.06 };

    /// two tips inside their boxes, from the lip forward by the pipe's length
    #[test]
    fn tips_in_place() {
        let v = exhausts(&E);
        assert!(!v.is_empty());
        for p in &v {
            let (x, y, z) = (p.pos[0], p.pos[1], -p.pos[2]);
            assert!((0.025 - 1e-5..=0.285 + 1e-5).contains(&x.abs()), "{x}");
            assert!((y - 0.5).abs() <= 0.055 + 1e-5, "{y}");
            assert!((-2.03 - 1e-5..=-1.95 + 1e-5).contains(&z), "{z}");
        }
        assert!(v.iter().any(|p| p.pos[0] > 0.0) && v.iter().any(|p| p.pos[0] < 0.0));
    }

    /// every triangle keeps its smallest angle over the mesh check's sliver limit: the rim
    /// along the straight sides, the walls round the fillets and the end plate's fan were under
    #[test]
    fn no_slivers() {
        let v = exhausts(&E);
        for t in v.as_chunks::<3>().0 {
            let p = [0, 1, 2].map(|k| Vec3::from(t[k].pos));
            let least = (0..3).map(|k| (p[(k + 1) % 3] - p[k]).angle_between(p[(k + 2) % 3] - p[k]).to_degrees()).fold(180.0, f32::min);
            assert!(least > crate::wire::SLIVER_DEG, "{least} at {p:?}");
        }
    }

    /// the rim is `lip` thick all round, fillets included, and the outline keeps to its corners
    #[test]
    fn even_rim() {
        let (o, i) = outlines(&E);
        assert_eq!(o.len(), i.len());
        let p = corners(&E);
        for (a, b) in o.iter().zip(&i) {
            assert!(((a.0 - b.0).length() - E.lip).abs() < 1e-5);
            assert!(((a.0 - b.0).normalize() - a.1).length() < 1e-4);
            for k in 0..p.len() {
                let nn = edge_normal(p[k], p[(k + 1) % p.len()]);
                assert!((a.0 - p[k]).dot(nn) <= 1e-5, "outside edge {k}");
            }
        }
    }
}
