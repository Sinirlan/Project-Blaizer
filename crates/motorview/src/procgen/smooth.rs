//! REMASTER curved panels (09-remaster 9.22). Where the original folds a panel out of a few flat
//! facets and lets its texture shade them round (the Titan's fenders), the facets are remeshed
//! into even triangles (edges cut every `step`, a lattice inside, constrained Delaunay) whose
//! points are then faired into a thin plate (the least bending surface), held to the original
//! facets by a spring per point where the zone has a `round` (the R-Build 2: the plate through
//! the outline alone lost the facets' outward bulge and its fenders read concave; the springs
//! keep the bulge and the plate rounds the folds over about `round`). The zone's outline and
//! its steeper folds stay put, so the faces round it keep their shape (their edges just get the
//! cut points); the plate meets them with its own slope, so the folds to them stay creases (soft
//! ones where the shading rounds them). Holding the panels' slopes along the outline as well
//! (#171 phase 4 took it out) pulled the plate into two planes at every outline corner, and the
//! least bending surface through those twists rippled across the whole panel. A design choice.

use super::body::{BodyStyle, Face, WELD, triangulate, weld_key};
use glam::{DVec2, DVec3, Vec2, Vec3};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

/// One zone of facets to curve: the faces of a colour tag in `from` whose centroid (x as |x|:
/// both sides) lies in one of the boxes `lo[i]..hi[i]` (zones fair one by one, so a fold between
/// two zones stays a crease: the R-Build 2's hood joins its flanks through a second box). More
/// than one tag (the R-Build 2's flank: body paint over the trim-tagged sill) rounds the folds
/// between panels of different colours; each face keeps its colour, so the paint line stays
/// where the fold was.
#[derive(Clone, Debug, PartialEq)]
pub struct Smooth {
    pub from: Vec<[u8; 3]>,
    pub lo: Vec<[f32; 3]>,
    pub hi: Vec<[f32; 3]>,
    /// folds flatter than this (degrees) are rounded; `body` shades the result smooth up to it
    pub max_deg: f32,
    /// triangle size (m): edges are cut and the inside filled at this spacing
    pub step: f32,
    /// no effect since #171 phase 4 (it relaxed the slope hold along the outline, which is gone);
    /// kept so recipes still parse and round-trip
    pub loose: bool,
    /// the length (m) over which a fold is rounded: each point is held to the original facets by
    /// a spring, so the plate follows their shape (the fenders' bulge) beyond that distance; 0 =
    /// no springs, the least bending surface through the outline alone (the Titan)
    pub round: f32,
}

/// Curve the zones of `st` (game space, after `round_arches`; `hubs` and `radius` as there, so
/// its arch lips stay out).
pub fn smooth(faces: &mut Vec<Face>, hubs: &[[f32; 3]], radius: f32, st: &BodyStyle) {
    let r = radius + st.arch_clearance;
    let on_lip = |f: &Face| {
        st.arch_clearance > 0.0
            && hubs.iter().any(|h| {
                f.pts.iter().all(|p| {
                    let d = Vec2::new(p[2] - h[2], p[1] - h[1] - st.drop).length();
                    p[0] * h[0] > 0.0 && d > r - 0.005 && d < r + st.arch_lip + 0.005
                })
            })
    };
    for z in &st.smooths {
        let boxes: Vec<(Vec3, Vec3)> = z.lo.iter().zip(&z.hi).map(|(&lo, &hi)| (Vec3::from(lo), Vec3::from(hi))).collect();
        let (members, rest): (Vec<Face>, Vec<Face>) = faces.drain(..).partition(|f| {
            let c = f.pts.iter().map(|&p| Vec3::from(p)).sum::<Vec3>() / f.pts.len() as f32;
            let c = Vec3::new(c.x.abs(), c.y, c.z);
            z.from.contains(&f.rgb) && f.smooth_deg == 0.0 && boxes.iter().any(|&(lo, hi)| c.cmpge(lo).all() && c.cmple(hi).all()) && !on_lip(f)
        });
        let (curved, outline) = curve(&members, z);
        *faces = rest.into_iter().map(|f| split_edges(f, &outline, z.step)).collect();
        faces.extend(curved);
    }
    // The cut faces' edges to each other get cut points too (at the smaller of their spacings,
    // so both sides agree): with none, the lattice's triangles fan to the two corners of such an
    // edge in slivers (the Titan's hood centre, along the centre line). Edges to uncut faces stay
    // as they are, or the cage would get T-junctions.
    let owner: HashMap<EdgeKey, f32> = faces
        .iter()
        .filter(|f| f.remesh > 0.0)
        .flat_map(|f| (0..f.pts.len()).map(move |k| ((weld_key(f.pts[k]), weld_key(f.pts[(k + 1) % f.pts.len()])), f.remesh)))
        .collect();
    for f in faces.iter_mut().filter(|f| f.remesh > 0.0) {
        let h = f.remesh;
        if let Some(g) = cut_edges(f, |a, b| owner.get(&(b, a)).map(|&o| o.min(h))) {
            *f = g;
        }
    }
}

/// an edge by its corners' weld keys, in its face's order
type EdgeKey = ([i32; 3], [i32; 3]);

/// A member face's distinct corners, CCW round its plane normal `n`.
struct Poly {
    /// source corner per corner
    src: Vec<usize>,
    p: Vec<Vec3>,
    key: Vec<[i32; 3]>,
    n: Vec3,
}

impl Poly {
    fn new(f: &Face) -> Self {
        let mut src: Vec<usize> = Vec::new();
        for k in 0..f.pts.len() {
            if src.last().is_none_or(|&l| weld_key(f.pts[l]) != weld_key(f.pts[k])) {
                src.push(k);
            }
        }
        while src.len() > 1 && weld_key(f.pts[src[0]]) == weld_key(f.pts[src[src.len() - 1]]) {
            src.pop();
        }
        let p: Vec<Vec3> = src.iter().map(|&k| Vec3::from(f.pts[k])).collect();
        let mut n: Vec3 = (0..p.len()).map(|i| p[i].cross(p[(i + 1) % p.len()])).sum();
        if n.dot(Vec3::from(f.normal)) < 0.0 {
            src.reverse();
            n = -n;
        }
        let p: Vec<Vec3> = src.iter().map(|&k| Vec3::from(f.pts[k])).collect();
        let key = p.iter().map(|q| weld_key(q.to_array())).collect();
        Poly { src, p, key, n: n.normalize_or_zero() }
    }
}

/// per poly and edge: whether it is a fold to round (a twin in the zone at most `acos(cos)` off)
fn fold_flags(polys: &[Poly], cos: f32) -> Vec<Vec<bool>> {
    let mut directed: HashMap<EdgeKey, usize> = HashMap::new();
    for (f, q) in polys.iter().enumerate() {
        for k in 0..q.p.len() {
            directed.insert((q.key[k], q.key[(k + 1) % q.p.len()]), f);
        }
    }
    polys
        .iter()
        .enumerate()
        .map(|(f, q)| {
            let m = q.p.len();
            (0..m).map(|k| directed.get(&(q.key[(k + 1) % m], q.key[k])).is_some_and(|&g| g != f && q.n.dot(polys[g].n) >= cos)).collect()
        })
        .collect()
}

/// The inner points of poly `q` (outline points `ring`): a triangular lattice of spacing `h`,
/// kept half a step off the outline, laid along the plane's z (or y) on a grid through the
/// car's origin, mirrored in x on the left; a face across the centre line (the R-Build 2's hood
/// stripe) gets columns symmetric in x, so its two halves fair alike.
fn lattice(q: &Poly, ring: &[Vec3], h: f32) -> Vec<Vec3> {
    let side = if q.p.iter().map(|p| p.x).sum::<f32>() < 0.0 { -1.0 } else { 1.0 };
    // a face across the centre line (the R-Build 2's hood stripe; the rows run along x there):
    // a row on the line and the column shifts by the row's distance from it, so the lattice
    // mirrors in x and the two halves fair alike; a face on one side keeps its rows half a step
    // off the origin (the Titan's mesh as before)
    let across = q.p.iter().any(|p| p.x < -0.01) && q.p.iter().any(|p| p.x > 0.01);
    let lt = (Vec3::Z - q.n * q.n.z).try_normalize().filter(|_| q.n.z.abs() < 0.9).unwrap_or_else(|| (Vec3::Y - q.n * q.n.y).normalize());
    let lb = q.n.cross(lt) * side;
    let sym = |p: Vec3| Vec2::new(p.dot(lt), p.dot(lb));
    let ring: Vec<Vec2> = ring.iter().map(|&p| sym(p)).collect();
    let nb = ring.len();
    let corners: Vec<Vec2> = q.p.iter().map(|&p| sym(p)).collect();
    let (lo, hi) = corners.iter().fold((Vec2::MAX, Vec2::MIN), |(l, u), &p| (l.min(p), u.max(p)));
    let (lo, hi) = (lo.as_dvec2(), hi.as_dvec2());
    let (hd, row) = (h as f64, h as f64 * 3f64.sqrt() / 2.0);
    let plane = q.n * q.p[0].dot(q.n);
    let mut out = Vec::new();
    for r in (lo.y / row).floor() as i64..=(hi.y / row).ceil() as i64 {
        let (y, k) = if across { (r as f64 * row, r.abs()) } else { ((r as f64 + 0.5) * row, r) };
        let shift = if k.rem_euclid(2) == 1 { 0.75 } else { 0.25 };
        for c in (lo.x / hd).floor() as i64 - 1..=(hi.x / hd).ceil() as i64 {
            let c = Vec2::new(((c as f64 + shift) * hd) as f32, y as f32);
            if inside(&corners, c) && (0..nb).all(|i| seg_dist(c, ring[i], ring[(i + 1) % nb]) > 0.55 * h) {
                out.push(plane + lt * c.x + lb * c.y);
            }
        }
    }
    out
}

/// how many pieces an edge (by its corners' weld keys, so both faces on it agree) is cut into
fn cuts(a: [i32; 3], b: [i32; 3], step: f32) -> usize {
    let d = (0..3).map(|i| ((a[i] - b[i]) as f32 * WELD).powi(2)).sum::<f32>().sqrt();
    ((d / step).round() as usize).max(1)
}

/// Cut point `i` of `n` on the edge `a`-`b`, interpolated from the end with the smaller weld
/// key, so the two faces on an edge (which walk it in opposite directions) get the same bits:
/// `a.lerp(b, i / n)` and `b.lerp(a, (n - i) / n)` differ in the last bits, and a point that
/// lands on a weld cell's border then welds as two, a crack along the fold (the Serpent's
/// windscreen, #174 phase 3: four holes framed in rubber by `inset_glass`).
fn edge_point(a: Vec3, b: Vec3, i: usize, n: usize) -> Vec3 {
    if weld_key(a.to_array()) <= weld_key(b.to_array()) { a.lerp(b, i as f32 / n as f32) } else { b.lerp(a, (n - i) as f32 / n as f32) }
}

/// The cut and faired triangles of `faces`, and the zone's outline (corner keys, CCW).
fn curve(faces: &[Face], z: &Smooth) -> (Vec<Face>, HashSet<EdgeKey>) {
    let faces = &t_junctions(faces, false);
    let polys: Vec<Poly> = faces.iter().map(Poly::new).collect();
    // a fold up to max_deg rounds; steeper ones and edges to other faces stay
    let cos = z.max_deg.to_radians().cos();
    let curved = fold_flags(&polys, cos);
    let mut outline = HashSet::new();
    let mut out = Vec::new();
    // cut points on straight edges: they stay where they are when the cuts are faired
    let mut fixed: HashSet<[i32; 3]> = HashSet::new();
    let h = z.step;
    // a face on the left whose mirror image is in the zone takes that face's mesh, mirrored: the
    // two sides then fair alike (Delaunay ties flip a few edges differently, and the stiff
    // fairing turned those into centimetre bumps on one side)
    let mirror = |k: [i32; 3]| [-k[0], k[1], k[2]];
    let shape = |q: &Poly, flip: bool| {
        let mut v: Vec<[i32; 3]> = q.key.iter().map(|&k| if flip { mirror(k) } else { k }).collect();
        v.sort();
        v
    };
    let left = |q: &Poly| q.p.iter().map(|p| p.x).sum::<f32>() < 0.0;
    let right: HashMap<Vec<[i32; 3]>, usize> = polys.iter().enumerate().filter(|(_, q)| !left(q)).map(|(g, q)| (shape(q, false), g)).collect();
    let twin: Vec<Option<usize>> = polys.iter().map(|q| if left(q) { right.get(&shape(q, true)).copied() } else { None }).collect();
    let order: Vec<usize> = (0..polys.len()).filter(|&f| twin[f].is_none()).chain((0..polys.len()).filter(|&f| twin[f].is_some())).collect();
    let mut meshes: HashMap<usize, (Vec<Vec3>, Vec<[usize; 3]>)> = HashMap::new();
    for f in order {
        let q = &polys[f];
        let m = q.p.len();
        if m < 3 {
            continue;
        }
        let face = &faces[f];
        outline.extend((0..m).filter(|&k| !curved[f][k]).map(|k| (q.key[k], q.key[(k + 1) % m])));
        let t = (q.p[1] - q.p[0]).normalize_or(Vec3::X);
        let b = q.n.cross(t);
        let flat: Vec<Vec2> = q.p.iter().map(|p| Vec2::new((*p - q.p[0]).dot(t), (*p - q.p[0]).dot(b))).collect();
        // the outline: corners and the cut points of each edge; a point on a straight edge is
        // fixed (a corner: either edge)
        let (mut p2, mut p3): (Vec<Vec2>, Vec<Vec3>) = (Vec::new(), Vec::new());
        for k in 0..m {
            let k1 = (k + 1) % m;
            let n = cuts(q.key[k], q.key[k1], h);
            for i in 0..n {
                let s = i as f32 / n as f32;
                p2.push(flat[k].lerp(flat[k1], s));
                p3.push(edge_point(q.p[k], q.p[k1], i, n));
                let edges = if i == 0 { vec![k, (k + m - 1) % m] } else { vec![k] };
                if edges.into_iter().any(|e| !curved[f][e]) {
                    fixed.insert(weld_key(p3[p3.len() - 1].to_array()));
                }
            }
        }
        let nb = p2.len();
        // inside: the `lattice`, or the mirror twin's mesh
        let (p3, tris) = if let Some(g) = twin[f] {
            let (gp, gt) = &meshes[&g];
            let mut pts: Vec<Vec3> = gp.iter().map(|&p| p * Vec3::new(-1.0, 1.0, 1.0)).collect();
            // the outline points are this face's own (its corners mirror the twin's only to the
            // weld grid): a mirrored cut point could weld apart from the one the face beside it
            // got from `split_edges`, and miss `fixed`, so the fairing pulled it off the edge (a
            // hole in the Serpent's bumper top, #174)
            let own = &p3[..nb];
            for p in pts.iter_mut().take(nb) {
                if let Some(&o) = own.iter().min_by(|a, b| a.distance_squared(*p).total_cmp(&b.distance_squared(*p))) {
                    *p = o;
                }
            }
            (pts, gt.iter().map(|t| [t[0], t[2], t[1]]).collect())
        } else {
            for p in lattice(q, &p3, h) {
                p2.push(Vec2::new((p - q.p[0]).dot(t), (p - q.p[0]).dot(b)));
                p3.push(p);
            }
            let tris = delaunay(&p2, nb);
            (p3, tris)
        };
        let p2: Vec<Vec2> = p3.iter().map(|&p| Vec2::new((p - q.p[0]).dot(t), (p - q.p[0]).dot(b))).collect();
        // colour and uv: over the corners' triangles
        let corner_tris = triangulate(&flat);
        let attr = |c: Vec2| {
            let (tri, w) = corner_tris
                .iter()
                .map(|tri| (tri, bary(c, tri.map(|i| flat[i]))))
                .max_by(|a, b| a.1.min_element().total_cmp(&b.1.min_element()))
                .expect("a polygon has a triangle");
            let w = w.max(Vec3::ZERO);
            let w = w / w.element_sum().max(1e-9);
            let colour: [f32; 4] = std::array::from_fn(|x| (0..3).map(|i| face.colour[q.src[tri[i]]][x] * w[i]).sum());
            let uv: [f32; 2] = std::array::from_fn(|x| (0..3).map(|i| face.uv[q.src[tri[i]]][x] * w[i]).sum());
            (colour, uv)
        };
        for tri in &tris {
            let a = tri.map(|i| attr(p2[i]));
            out.push(Face { pts: tri.map(|i| p3[i].to_array()).to_vec(), normal: q.n.to_array(), colour: a.map(|x| x.0).to_vec(), uv: a.map(|x| x.1).to_vec(), smooth_deg: z.max_deg, remesh: 0.0, ..face.clone() });
        }
        meshes.insert(f, (p3, tris));
    }
    // the springs' stiffness for a decay length `round` (see `fair`)
    let lambda = if z.round > 0.0 { (h / z.round).powi(4) as f64 / 4.0 } else { 0.0 };
    fair(&mut out, &fixed, lambda);
    (out, outline)
}

/// barycentric weights of `c` in triangle `t`
fn bary(c: Vec2, t: [Vec2; 3]) -> Vec3 {
    let area = (t[1] - t[0]).perp_dot(t[2] - t[0]);
    if area.abs() < 1e-12 {
        return Vec3::splat(-1.0);
    }
    let w1 = (c - t[0]).perp_dot(t[2] - t[0]) / area;
    let w2 = (t[1] - t[0]).perp_dot(c - t[0]) / area;
    Vec3::new(1.0 - w1 - w2, w1, w2)
}

/// whether `c` lies inside polygon `p` (even-odd)
fn inside(p: &[Vec2], c: Vec2) -> bool {
    let mut odd = false;
    for i in 0..p.len() {
        let (a, b) = (p[i], p[(i + 1) % p.len()]);
        if (a.y > c.y) != (b.y > c.y) && c.x < a.x + (c.y - a.y) / (b.y - a.y) * (b.x - a.x) {
            odd = !odd;
        }
    }
    odd
}

fn seg_dist(c: Vec2, a: Vec2, b: Vec2) -> f32 {
    let d = b - a;
    let s = ((c - a).dot(d) / d.length_squared().max(1e-12)).clamp(0.0, 1.0);
    c.distance(a + d * s)
}

/// Constrained Delaunay triangulation of the polygon `p[..nb]` (CCW, its edges kept) with the
/// points `p[nb..]` inside: ear clipping, then each inner point splits the triangle (or the two
/// on the edge) it lies on, Lawson flips after each step until every free edge is locally
/// Delaunay. The triangles keep their neighbours, so a step only revisits the edges it touched.
fn delaunay(p: &[Vec2], nb: usize) -> Vec<[usize; 3]> {
    let mut m = Cdt::new(p, nb, triangulate(&p[..nb]));
    // Delaunay after every step: the result doesn't hang on the ear clipping's order
    let all: Vec<usize> = (0..m.tris.len()).collect();
    m.legalize(&all);
    let mut last = 0;
    for (i, &pi) in p.iter().enumerate().skip(nb) {
        let Some(j) = m.locate(pi, last) else { continue };
        let w = m.bary(j, pi);
        let touched = match (0..3).find(|&k| w[k] < 1e-6) {
            None => m.split(j, i),
            // on the edge opposite corner k: split both triangles on it
            Some(k) => match m.split_edge(j, k, i) {
                Some(t) => t,
                None => continue,
            },
        };
        last = touched[0];
        m.legalize(&touched);
    }
    m.tris
}

/// no neighbour: the outline, or a hole
const NONE: usize = usize::MAX;

/// A triangulation with neighbour links for `delaunay`.
struct Cdt<'a> {
    p: &'a [Vec2],
    nb: usize,
    tris: Vec<[usize; 3]>,
    /// the triangle across edge k (corner k to corner k + 1)
    adj: Vec<[usize; 3]>,
}

impl<'a> Cdt<'a> {
    fn new(p: &'a [Vec2], nb: usize, tris: Vec<[usize; 3]>) -> Self {
        let mut edge: HashMap<(usize, usize), usize> = HashMap::new();
        for (j, t) in tris.iter().enumerate() {
            for k in 0..3 {
                edge.insert((t[k], t[(k + 1) % 3]), j);
            }
        }
        let adj = tris.iter().map(|t| std::array::from_fn(|k| edge.get(&(t[(k + 1) % 3], t[k])).copied().unwrap_or(NONE))).collect();
        Cdt { p, nb, tris, adj }
    }

    /// an outline edge: never flipped or split
    fn fixed(&self, a: usize, b: usize) -> bool {
        a < self.nb && b < self.nb && (b == (a + 1) % self.nb || a == (b + 1) % self.nb)
    }

    fn bary(&self, j: usize, c: Vec2) -> Vec3 {
        bary(c, self.tris[j].map(|k| self.p[k]))
    }

    /// d inside the circle through CCW a, b, c
    fn in_circle(&self, a: usize, b: usize, c: usize, d: usize) -> bool {
        let [a, b, c, d] = [a, b, c, d].map(|k| self.p[k].as_dvec2());
        let (a, b, c) = (a - d, b - d, c - d);
        let det = a.length_squared() * b.perp_dot(c) - b.length_squared() * a.perp_dot(c) + c.length_squared() * a.perp_dot(b);
        det > 1e-14
    }

    /// the edge index of directed edge a→b in triangle j
    fn edge_of(&self, j: usize, a: usize, b: usize) -> Option<usize> {
        (0..3).find(|&k| self.tris[j][k] == a && self.tris[j][(k + 1) % 3] == b)
    }

    /// point triangle `n`'s side of edge a→b (as `n` sees it, b→a) at `to`
    fn relink(&mut self, n: usize, a: usize, b: usize, to: usize) {
        if n != NONE {
            if let Some(k) = self.edge_of(n, b, a) {
                self.adj[n][k] = to;
            }
        }
    }

    /// The triangle holding `c`: a walk from `start` towards it (the lattice comes row by row,
    /// so it is short), else the one it lies most inside (a concave outline stops the walk).
    fn locate(&self, c: Vec2, start: usize) -> Option<usize> {
        let mut j = start.min(self.tris.len().saturating_sub(1));
        for _ in 0..self.tris.len() {
            let w = self.bary(j, c);
            if w.min_element() >= -1e-6 {
                return Some(j);
            }
            // weight k < 0: c lies beyond the edge opposite corner k
            let k = (0..3).min_by(|&x, &y| w[x].total_cmp(&w[y])).expect("three weights");
            match self.adj[j][(k + 1) % 3] {
                NONE => break,
                n => j = n,
            }
        }
        let (j, w) = self.tris.iter().enumerate().map(|(j, t)| (j, bary(c, t.map(|k| self.p[k])))).max_by(|a, b| a.1.min_element().total_cmp(&b.1.min_element()))?;
        (w.min_element() >= -1e-6).then_some(j)
    }

    /// point i inside triangle j: three triangles round it
    fn split(&mut self, j: usize, i: usize) -> Vec<usize> {
        let t = self.tris[j];
        let a = self.adj[j];
        let (n1, n2) = (self.tris.len(), self.tris.len() + 1);
        self.tris[j] = [t[0], t[1], i];
        self.adj[j] = [a[0], n1, n2];
        self.tris.push([t[1], t[2], i]);
        self.adj.push([a[1], n2, j]);
        self.tris.push([t[2], t[0], i]);
        self.adj.push([a[2], j, n1]);
        self.relink(a[1], t[1], t[2], n1);
        self.relink(a[2], t[2], t[0], n2);
        vec![j, n1, n2]
    }

    /// point i on the edge opposite corner k of triangle j: both triangles on it split in two
    /// (none if the edge is the outline's)
    fn split_edge(&mut self, j: usize, k: usize, i: usize) -> Option<Vec<usize>> {
        let t = self.tris[j];
        let (o, e0, e1) = (t[k], t[(k + 1) % 3], t[(k + 2) % 3]);
        let u = self.adj[j][(k + 1) % 3];
        if u == NONE || self.fixed(e0, e1) {
            return None;
        }
        let m = self.edge_of(u, e1, e0)?;
        let d = self.tris[u][(m + 2) % 3];
        let (x1, x2) = (self.adj[j][k], self.adj[j][(k + 2) % 3]);
        let (y1, y2) = (self.adj[u][(m + 1) % 3], self.adj[u][(m + 2) % 3]);
        let (na, nb) = (self.tris.len(), self.tris.len() + 1);
        self.tris[j] = [o, e0, i];
        self.adj[j] = [x1, nb, na];
        self.tris[u] = [e1, i, d];
        self.adj[u] = [na, nb, y2];
        self.tris.push([o, i, e1]);
        self.adj.push([j, u, x2]);
        self.tris.push([i, e0, d]);
        self.adj.push([j, y1, u]);
        self.relink(x2, e1, o, na);
        self.relink(y1, e0, d, nb);
        Some(vec![j, na, u, nb])
    }

    /// Lawson flips from the edges of triangles `start` until no free edge is left whose far
    /// corner lies inside its triangle's circle.
    fn legalize(&mut self, start: &[usize]) {
        let mut stack: Vec<(usize, usize, usize)> = start.iter().flat_map(|&j| (0..3).map(move |k| (j, k))).map(|(j, k)| (j, self.tris[j][k], self.tris[j][(k + 1) % 3])).collect();
        let mut budget = 50 * self.tris.len() + 1000;
        while let Some((j, a, b)) = stack.pop() {
            // the edge may have been flipped away since it was pushed
            let Some(k) = self.edge_of(j, a, b) else { continue };
            let u = self.adj[j][k];
            if u == NONE || self.fixed(a, b) {
                continue;
            }
            let c = self.tris[j][(k + 2) % 3];
            let Some(m) = self.edge_of(u, b, a) else { continue };
            let d = self.tris[u][(m + 2) % 3];
            if !self.in_circle(a, b, c, d) {
                continue;
            }
            if budget == 0 {
                break;
            }
            budget -= 1;
            let (bc, ca) = (self.adj[j][(k + 1) % 3], self.adj[j][(k + 2) % 3]);
            let (ad, db) = (self.adj[u][(m + 1) % 3], self.adj[u][(m + 2) % 3]);
            self.tris[j] = [a, d, c];
            self.adj[j] = [ad, u, ca];
            self.tris[u] = [d, b, c];
            self.adj[u] = [db, bc, j];
            self.relink(ad, a, d, j);
            self.relink(bc, b, c, u);
            stack.extend([(j, a, d), (j, c, a), (u, d, b), (u, b, c)]);
        }
    }
}

/// Thin-plate fairing of the remeshed points: the points but the `fixed` ones move so that the
/// surface bends as little as possible (least squares on the cotangent Laplacian of every point
/// off the outline, solved by conjugate gradients), each held where it started by a spring of
/// stiffness `lambda` (0: none). That is a plate on an elastic foundation: with the Laplacian's
/// h²/4 scale on a lattice of spacing h, a fold is rounded over a decay length h / (4 λ)^¼ and
/// the plate follows the facets beyond it, so their bulge stays (the Titan's zones have no
/// springs: the plate through the outline alone, #171 phase 4; the R-Build 2's lost its fenders'
/// bulge that way). The outline keeps its place; inside, no fold is left. Ordered maps: the same
/// mesh every run.
fn fair(tris: &mut [Face], fixed: &HashSet<[i32; 3]>, lambda: f64) {
    let mut id: BTreeMap<[i32; 3], usize> = BTreeMap::new();
    for t in tris.iter() {
        for &p in &t.pts {
            let n = id.len();
            id.entry(weld_key(p)).or_insert(n);
        }
    }
    let mut pos = vec![DVec3::ZERO; id.len()];
    let mut nbr: Vec<BTreeSet<usize>> = vec![BTreeSet::new(); id.len()];
    for t in tris.iter() {
        let k: Vec<usize> = t.pts.iter().map(|&p| id[&weld_key(p)]).collect();
        for i in 0..k.len() {
            pos[k[i]] = Vec3::from(t.pts[i]).as_dvec3();
            nbr[k[i]].extend(k.iter().copied().filter(|&j| j != k[i]));
        }
    }
    let nbr: Vec<Vec<usize>> = nbr.into_iter().map(|s| s.into_iter().collect()).collect();
    let mut is_fixed = vec![false; id.len()];
    for (k, &i) in &id {
        is_fixed[i] = fixed.contains(k);
    }
    // rows and unknowns: every point off the outline (its first ring too: holding that ring in
    // the panels' planes, for their slopes, twisted the plate at every outline corner and
    // rippled it; #171 phase 4)
    let free: Vec<bool> = is_fixed.iter().map(|f| !f).collect();
    if !free.contains(&true) {
        return;
    }
    // the rows as flat neighbour lists: (row, its neighbours' range in `col` and `w`)
    let rows: Vec<(usize, std::ops::Range<usize>)> = (0..id.len())
        .filter(|&r| free[r] && !nbr[r].is_empty())
        .scan(0, |at, r| {
            let span = *at..*at + nbr[r].len();
            *at = span.end;
            Some((r, span))
        })
        .collect();
    let col: Vec<usize> = rows.iter().flat_map(|(r, _)| nbr[*r].iter().copied()).collect();
    // neighbour weights: cotangents (cot α + cot β of the angles facing the edge, the discrete
    // Laplace–Beltrami operator), each row summing to 1. They vanish on a flat mesh whatever its
    // connectivity, so the lattices of two panels stitched along a rounded fold (and the Delaunay's
    // tie flips) leave no trace; 1/length weights rippled the surface along every such seam
    // (#171 phase 4). Obtuse corners are clamped: a negative weight pulls a point the wrong way.
    let mut cot: HashMap<(usize, usize), f64> = HashMap::new();
    for t in tris.iter() {
        let k: Vec<usize> = t.pts.iter().map(|&p| id[&weld_key(p)]).collect();
        for i in 0..3 {
            let (a, b, c) = (pos[k[i]], pos[k[(i + 1) % 3]], pos[k[(i + 2) % 3]]);
            let (u, v) = (b - a, c - a);
            let w = (u.dot(v) / u.cross(v).length().max(1e-12)).max(0.0);
            let e = (k[(i + 1) % 3].min(k[(i + 2) % 3]), k[(i + 1) % 3].max(k[(i + 2) % 3]));
            *cot.entry(e).or_insert(0.0) += w;
        }
    }
    let w: Vec<f64> = rows
        .iter()
        .flat_map(|(r, _)| {
            let raw: Vec<f64> = nbr[*r].iter().map(|&j| cot.get(&((*r).min(j), (*r).max(j))).copied().unwrap_or(0.0).max(1e-6)).collect();
            let sum: f64 = raw.iter().sum();
            raw.into_iter().map(move |x| x / sum)
        })
        .collect();
    let lap = |x: &[DVec3], out: &mut [DVec3]| {
        for (r, span) in &rows {
            out[*r] = col[span.clone()].iter().zip(&w[span.clone()]).map(|(&j, &w)| x[j] * w).sum::<DVec3>() - x[*r];
        }
    };
    // Lᵀy on the unknowns
    let lap_t = |y: &[DVec3], out: &mut [DVec3]| {
        out.fill(DVec3::ZERO);
        for (r, span) in &rows {
            let y = y[*r];
            out[*r] -= y;
            for (&j, &w) in col[span.clone()].iter().zip(&w[span.clone()]) {
                out[j] += y * w;
            }
        }
        for (o, &f) in out.iter_mut().zip(&free) {
            if !f {
                *o = DVec3::ZERO;
            }
        }
    };
    let dot = |a: &[DVec3], b: &[DVec3]| a.iter().zip(b).fold(DVec3::ZERO, |s, (x, y)| s + *x * *y);
    // CG on (LᵀL + λI) x = λ x₀ (the fixed points in the right-hand side too), the three axes at
    // once, in f64 and to a tight residual: short edges make it stiff, and stopping early left
    // soft bumps. Starting from x₀ the residual is -LᵀL x₀ with or without the springs.
    // (A Jacobi preconditioner was tried: LᵀL's diagonal is near even, it saved no iterations.)
    let n = id.len();
    let (mut tmp, mut ad) = (vec![DVec3::ZERO; n], vec![DVec3::ZERO; n]);
    lap(&pos, &mut tmp);
    lap_t(&tmp, &mut ad);
    let mut r: Vec<DVec3> = ad.iter().map(|v| -*v).collect();
    let mut d = r.clone();
    let mut rr = dot(&r, &r);
    let tol = rr * 1e-24;
    for _ in 0..50 * n {
        if rr.cmple(tol).all() {
            break;
        }
        lap(&d, &mut tmp);
        lap_t(&tmp, &mut ad);
        if lambda > 0.0 {
            // d is zero on the fixed points, so this touches the unknowns only
            for (a, d) in ad.iter_mut().zip(&d) {
                *a += *d * lambda;
            }
        }
        let alpha = rr / dot(&d, &ad).max(DVec3::splat(1e-300));
        for i in 0..n {
            pos[i] += d[i] * alpha;
            r[i] -= ad[i] * alpha;
        }
        let next = dot(&r, &r);
        let beta = next / rr.max(DVec3::splat(1e-300));
        rr = next;
        for i in 0..n {
            d[i] = r[i] + d[i] * beta;
        }
    }
    for t in tris.iter_mut() {
        for p in t.pts.iter_mut() {
            *p = pos[id[&weld_key(*p)]].as_vec3().to_array();
        }
    }
}

/// How far (m) a corner may lie off an open edge and still be cut into it (`t_junctions` with
/// `open_only`, the `Refine` `seams` pass). [O] Half the weld cell's reach (`WELD_T`): on the
/// Serpent the sill's flare is already cut into 2.7 cm steps that sit 1-2 mm off the long
/// underside edge (the original's corners differ by that much), and cut at 2 mm those corners
/// pulled the edge's points off its line into 6 non-manifold edges along the rear sill (#387).
/// At 1 mm they are not cut and the closing of the other cars' seams stays (the ASC II's needs
/// more than 0.5 mm; measured with the mesh check, #387).
const SEAM_TOL: f32 = 1e-3;

/// `faces` with each edge cut at the other faces' corners lying on it (the original meets a long
/// edge with two short ones in places), so that every fold is an edge pair. With `open_only`
/// only an edge without its twin (the long side of a T-junction) is cut: a corner lying on a
/// paired edge belongs to a third face sitting on the seam (a mirror base on the body), and cut
/// in, it put three or four faces on the pieces (#177: the Python's non-manifold edges 7 -> 57).
fn t_junctions(faces: &[Face], open_only: bool) -> Vec<Face> {
    let mut corners: Vec<Vec3> = Vec::new();
    let mut seen = HashSet::new();
    for p in faces.iter().flat_map(|f| f.pts.iter()) {
        if seen.insert(weld_key(*p)) {
            corners.push(Vec3::from(*p));
        }
    }
    let edges: HashSet<([i32; 3], [i32; 3])> = faces.iter().flat_map(|f| (0..f.pts.len()).map(move |k| (weld_key(f.pts[k]), weld_key(f.pts[(k + 1) % f.pts.len()])))).collect();
    let tol = if open_only { SEAM_TOL } else { WELD_T };
    faces
        .iter()
        .map(|f| {
            let n = f.pts.len();
            let mut g = Face { pts: Vec::new(), colour: Vec::new(), uv: Vec::new(), ..f.clone() };
            for k in 0..n {
                let (a, b) = (Vec3::from(f.pts[k]), Vec3::from(f.pts[(k + 1) % n]));
                g.pts.push(a.to_array());
                g.colour.push(f.colour[k]);
                g.uv.push(f.uv[k]);
                let len = a.distance(b);
                if len < 1e-6 || (open_only && edges.contains(&(weld_key(f.pts[(k + 1) % n]), weld_key(f.pts[k])))) {
                    continue;
                }
                let d = (b - a) / len;
                let mut on: Vec<(f32, Vec3)> = corners
                    .iter()
                    .map(|&c| ((c - a).dot(d), c))
                    .filter(|&(t, c)| t > WELD_T && t < len - WELD_T && (a + d * t).distance(c) < tol)
                    .collect();
                on.sort_by(|x, y| x.0.total_cmp(&y.0));
                let (ua, ub) = (Vec2::from(f.uv[k]), Vec2::from(f.uv[(k + 1) % n]));
                for (t, c) in on {
                    g.pts.push(c.to_array());
                    g.colour.push(f.colour[k]);
                    g.uv.push(ua.lerp(ub, t / len).into());
                }
            }
            g
        })
        .collect()
}

/// a flat face's mesh: its points (with the index of the ring corner each one is) and triangles
pub(super) type FlatMesh = (Vec<(Vec3, Option<usize>)>, Vec<[usize; 3]>);

/// The triangles of the flat polygon `pts` (plane normal `n`, either winding) with a lattice
/// of spacing `h` inside (`lattice`, so mirror-image faces mesh alike): the points, the ring
/// first (each with its index in `pts`) then the inner ones (`None`), and the triangles over
/// them by the constrained Delaunay `delaunay`. For the faces a zone's outline cut
/// (`split_edges`, `Face::remesh`): ear clipping fanned their cut points to the far corners.
pub(super) fn flat_mesh(pts: &[Vec3], n: Vec3, h: f32) -> FlatMesh {
    // the ring CCW round n, as the Delaunay wants it
    let mut order: Vec<usize> = (0..pts.len()).collect();
    let newell: Vec3 = (0..pts.len()).map(|i| pts[i].cross(pts[(i + 1) % pts.len()])).sum();
    if newell.dot(n) < 0.0 {
        order.reverse();
    }
    let ring: Vec<Vec3> = order.iter().map(|&i| pts[i]).collect();
    let q = Poly { src: order.clone(), p: ring.clone(), key: ring.iter().map(|p| weld_key(p.to_array())).collect(), n };
    // in the plane (`body` projects the same way, and `lattice` along n): a warped face's edge
    // leans out of it
    let t = (q.p[1] - q.p[0]).reject_from_normalized(n).try_normalize().unwrap_or_else(|| n.any_orthonormal_vector());
    let b = n.cross(t);
    let flat = |p: Vec3| Vec2::new((p - q.p[0]).dot(t), (p - q.p[0]).dot(b));
    let mut p3: Vec<(Vec3, Option<usize>)> = order.iter().map(|&i| (pts[i], Some(i))).collect();
    let mut p2: Vec<Vec2> = ring.iter().map(|&p| flat(p)).collect();
    let nb = p2.len();
    for p in lattice(&q, &ring, h) {
        p2.push(flat(p));
        p3.push((p, None));
    }
    let tris = delaunay(&p2, nb);
    (p3, tris)
}

/// Mean value coordinates of `q` inside the polygon `ring` (either winding; Floater 2003):
/// weights summing to 1 that interpolate the corners' values smoothly across the inside.
pub(super) fn mean_value(ring: &[Vec2], q: Vec2) -> Vec<f32> {
    let n = ring.len();
    let d: Vec<DVec2> = ring.iter().map(|&v| (v - q).as_dvec2()).collect();
    let r: Vec<f64> = d.iter().map(|d| d.length().max(1e-12)).collect();
    // tan of half the angle at q between corner i and i + 1, as sin / (1 + cos): (r r - d.d) /
    // cross lost its digits to cancellation where a far point sees a short edge (the cut
    // points, the arch's arc), and the weights threw points centimetres off
    let half_tan: Vec<f64> = (0..n)
        .map(|i| {
            let j = (i + 1) % n;
            let den = r[i] * r[j] + d[i].dot(d[j]);
            if den < 1e-18 { 0.0 } else { d[i].perp_dot(d[j]) / den }
        })
        .collect();
    let w: Vec<f64> = (0..n).map(|i| (half_tan[(i + n - 1) % n] + half_tan[i]) / r[i]).collect();
    let sum: f64 = w.iter().sum();
    if sum.abs() < 1e-12 {
        return vec![1.0 / n as f32; n];
    }
    w.into_iter().map(|x| (x / sum) as f32).collect()
}

/// how close (m) a corner must lie to an edge to cut it
const WELD_T: f32 = 2e-3;

/// `f` with the edges `spacing` names (by the corners' weld keys, in `f`'s order) cut every
/// that many metres (`cuts`, so the face across the edge gets the same points); None when no
/// edge is named.
fn cut_edges(f: &Face, spacing: impl Fn([i32; 3], [i32; 3]) -> Option<f32>) -> Option<Face> {
    let n = f.pts.len();
    let at: Vec<Option<f32>> = (0..n).map(|k| spacing(weld_key(f.pts[k]), weld_key(f.pts[(k + 1) % n]))).collect();
    if at.iter().all(|s| s.is_none()) {
        return None;
    }
    let mut g = Face { pts: Vec::new(), colour: Vec::new(), uv: Vec::new(), ..f.clone() };
    for (k, &cut) in at.iter().enumerate() {
        let (a, b) = (f.pts[k], f.pts[(k + 1) % n]);
        g.pts.push(a);
        g.colour.push(f.colour[k]);
        g.uv.push(f.uv[k]);
        let Some(h) = cut else { continue };
        let (ua, ub) = (Vec2::from(f.uv[k]), Vec2::from(f.uv[(k + 1) % n]));
        let level = cuts(weld_key(a), weld_key(b), h);
        for i in 1..level {
            let t = i as f32 / level as f32;
            g.pts.push(edge_point(Vec3::from(a), Vec3::from(b), i, level).to_array());
            g.colour.push(f.colour[k]);
            g.uv.push(ua.lerp(ub, t).into());
        }
    }
    Some(g)
}

/// `faces` for `body` with every edge cut every `step` (m; `cuts`, so the two faces on an edge
/// get the same points) and each face marked for the lattice remesh at that spacing (one an
/// outline cut keeps its own): ear clipping fanned a long panel's corners across it in slivers
/// (the Titan's door band, sill and the panel before the rear arch), and Gouraud shading swept
/// the corners' normals along them. The lamps and vents read the uncut faces: their layouts
/// count the facets' edges. A design choice. With `seams` every open edge is first cut at the
/// corners lying on it (`t_junctions`): [O] the Avalanche's bonnet slot meets the nose with one
/// edge against three, and cut evenly each way the points welded in some stretches and not in
/// others, so the bevel tore the nose along the slot in notches (#177).
pub fn refine(faces: &[Face], step: f32, seams: bool) -> Vec<Face> {
    let split;
    let faces = if seams {
        split = t_junctions(faces, true);
        &split[..]
    } else {
        faces
    };
    faces
        .iter()
        .map(|f| {
            let mut g = cut_edges(f, |_, _| Some(step)).unwrap_or_else(|| f.clone());
            if g.remesh == 0.0 {
                g.remesh = step;
            }
            g
        })
        .collect()
}

/// Give a face outside the zone the zone's cut points on the edges it shares with its outline,
/// and mark it for the lattice remesh (`Face::remesh`, `body`: ear clipping fanned the cut
/// points to its far corners) at twice the step: it is flat, so the spacing only has to keep
/// the cut points' triangles short.
fn split_edges(f: Face, outline: &HashSet<EdgeKey>, step: f32) -> Face {
    match cut_edges(&f, |a, b| outline.contains(&(b, a)).then_some(step)) {
        Some(g) => Face { remesh: 2.0 * step, ..g },
        None => f,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// the previous `delaunay` (full rescans), for the equivalence test: constrained Delaunay triangulation of the polygon `p[..nb]` (CCW, its edges kept) with the
    /// points `p[nb..]` inside: ear clipping, then each inner point splits the triangle (or the two
    /// on the edge) it lies on, Lawson flips after each step until every free edge is locally
    /// Delaunay.
    fn delaunay_ref(p: &[Vec2], nb: usize) -> Vec<[usize; 3]> {
        let fixed = |a: usize, b: usize| a < nb && b < nb && (b == (a + 1) % nb || a == (b + 1) % nb);
        // d inside the circle through CCW a, b, c
        let in_circle = |a: usize, b: usize, c: usize, d: usize| {
            let [a, b, c, d] = [a, b, c, d].map(|k| p[k].as_dvec2());
            let (a, b, c) = (a - d, b - d, c - d);
            let det = a.length_squared() * b.perp_dot(c) - b.length_squared() * a.perp_dot(c) + c.length_squared() * a.perp_dot(b);
            det > 1e-14
        };
        let legalize = |tris: &mut Vec<[usize; 3]>| {
            'flip: for _ in 0..50 * tris.len() {
                let mut edge: HashMap<(usize, usize), usize> = HashMap::new();
                for (j, t) in tris.iter().enumerate() {
                    for k in 0..3 {
                        edge.insert((t[k], t[(k + 1) % 3]), j);
                    }
                }
                for j in 0..tris.len() {
                    for k in 0..3 {
                        let (a, b, c) = (tris[j][k], tris[j][(k + 1) % 3], tris[j][(k + 2) % 3]);
                        if fixed(a, b) {
                            continue;
                        }
                        let Some(&u) = edge.get(&(b, a)) else { continue };
                        let d = tris[u].iter().copied().find(|&x| x != a && x != b).expect("a triangle has a third corner");
                        if in_circle(a, b, c, d) {
                            tris[j] = [a, d, c];
                            tris[u] = [d, b, c];
                            continue 'flip;
                        }
                    }
                }
                break;
            }
        };
        // Delaunay after every step: the result doesn't hang on the ear clipping's order
        let mut tris = triangulate(&p[..nb]);
        legalize(&mut tris);
        for i in nb..p.len() {
            let Some((j, w)) = tris.iter().enumerate().map(|(j, t)| (j, bary(p[i], t.map(|k| p[k])))).max_by(|a, b| a.1.min_element().total_cmp(&b.1.min_element())) else {
                continue;
            };
            if w.min_element() < -1e-6 {
                continue;
            }
            let t = tris[j];
            match (0..3).find(|&k| w[k] < 1e-6) {
                None => {
                    tris[j] = [t[0], t[1], i];
                    tris.push([t[1], t[2], i]);
                    tris.push([t[2], t[0], i]);
                }
                // on the edge opposite corner k: split both triangles on it
                Some(k) => {
                    let (o, e0, e1) = (t[k], t[(k + 1) % 3], t[(k + 2) % 3]);
                    let twin = tris.iter().position(|u| (0..3).any(|m| u[m] == e1 && u[(m + 1) % 3] == e0));
                    let Some(u) = twin.filter(|_| !fixed(e0, e1)) else { continue };
                    let d = tris[u].iter().copied().find(|&x| x != e0 && x != e1).expect("a triangle has a third corner");
                    tris[j] = [o, e0, i];
                    tris.push([o, i, e1]);
                    tris[u] = [e1, i, d];
                    tris.push([i, e0, d]);
                }
            }
            legalize(&mut tris);
        }
        tris
    }

    /// the linked triangulation is the one the full rescans gave (the same triangles, in
    /// another order), round a convex and a concave outline with a lattice inside
    #[test]
    fn delaunay_matches_reference() {
        let outlines: [Vec<Vec2>; 2] = [
            vec![Vec2::new(0.0, 0.0), Vec2::new(1.0, 0.05), Vec2::new(1.1, 0.7), Vec2::new(0.4, 1.0), Vec2::new(-0.1, 0.6)],
            vec![Vec2::new(0.0, 0.0), Vec2::new(1.0, 0.0), Vec2::new(1.0, 1.0), Vec2::new(0.55, 0.35), Vec2::new(0.0, 1.0)],
        ];
        for ring in outlines {
            // the outline cut every 0.05, a lattice inside at 0.04 kept off it
            let mut p: Vec<Vec2> = Vec::new();
            for k in 0..ring.len() {
                let (a, b) = (ring[k], ring[(k + 1) % ring.len()]);
                let n = (a.distance(b) / 0.05).round().max(1.0) as usize;
                p.extend((0..n).map(|i| a.lerp(b, i as f32 / n as f32)));
            }
            let nb = p.len();
            for r in 0..30 {
                for c in 0..30 {
                    let q = Vec2::new((c as f32 + if r % 2 == 1 { 0.75 } else { 0.25 }) * 0.04 - 0.1, (r as f32 + 0.5) * 0.04 * 0.866);
                    if inside(&ring, q) && (0..nb).all(|i| seg_dist(q, p[i], p[(i + 1) % nb]) > 0.022) {
                        p.push(q);
                    }
                }
            }
            let norm = |tris: Vec<[usize; 3]>| {
                let mut v: Vec<[usize; 3]> = tris
                    .into_iter()
                    .map(|t| {
                        let r = (0..3).min_by_key(|&k| t[k]).unwrap();
                        [t[r], t[(r + 1) % 3], t[(r + 2) % 3]]
                    })
                    .collect();
                v.sort();
                v
            };
            assert!(p.len() > nb + 100);
            assert_eq!(norm(delaunay(&p, nb)), norm(delaunay_ref(&p, nb)));
        }
    }

    fn face(pts: &[[f32; 3]], normal: [f32; 3]) -> Face {
        Face { pts: pts.to_vec(), normal, colour: vec![[0.5; 4]; pts.len()], uv: vec![[0.0; 2]; pts.len()], layer: -1.0, env: 1.0, rgb: [200; 3], smooth_deg: 0.0, remesh: 0.0 }
    }

    /// #177: one long edge against two short ones (a T-junction, the Avalanche's bonnet slot):
    /// after `refine` every cut edge has its twin, so the bevel sees one crease all along
    #[test]
    fn refine_pairs_the_edges_of_a_t_junction() {
        let slot = face(&[[0.0, 0.0, 0.0], [0.7, 0.0, 0.0], [0.7, 0.1, -0.05], [0.0, 0.1, -0.05]], [0.0, 0.45, 0.9]);
        let left = face(&[[0.0, 0.0, 0.0], [0.0, -0.1, 0.3], [0.27, -0.1, 0.3], [0.27, 0.0, 0.0]], [0.0, 0.95, 0.3]);
        let right = face(&[[0.27, 0.0, 0.0], [0.27, -0.1, 0.3], [0.7, -0.1, 0.3], [0.7, 0.0, 0.0]], [0.0, 0.95, 0.3]);
        let out = refine(&[slot, left, right], 0.04, true);
        let mut edges: HashMap<([i32; 3], [i32; 3]), usize> = HashMap::new();
        for f in &out {
            for k in 0..f.pts.len() {
                *edges.entry((weld_key(f.pts[k]), weld_key(f.pts[(k + 1) % f.pts.len()]))).or_default() += 1;
            }
        }
        let on_seam = |k: [i32; 3]| k[1] == 0 && k[2] == 0;
        for (&(a, b), _) in edges.iter().filter(|((a, b), _)| on_seam(*a) && on_seam(*b)) {
            assert!(edges.contains_key(&(b, a)), "seam edge {a:?} -> {b:?} has no twin");
        }
    }

    /// a roof of two quads folded 30° (15° each way): the ridge rounds (its middle sinks), the
    /// outline stays exactly where it was, and every cut edge is shared by two triangles or lies
    /// on the outline
    #[test]
    fn fold_rounds_outline_stays() {
        let (s, c) = (15f32.to_radians().sin() * 0.5, 15f32.to_radians().cos() * 0.5);
        let roof = [
            face(&[[-0.5, 0.0, 0.0], [-0.5, 0.0, 1.0], [-0.5 + c, s, 1.0], [-0.5 + c, s, 0.0]], [-s, c, 0.0]),
            face(&[[-0.5 + c, s, 0.0], [-0.5 + c, s, 1.0], [-0.5 + 2.0 * c, 0.0, 1.0], [-0.5 + 2.0 * c, 0.0, 0.0]], [s, c, 0.0]),
        ];
        let z = Smooth { from: vec![[200; 3]], lo: vec![[-9.0; 3]], hi: vec![[9.0; 3]], max_deg: 50.0, step: 0.1, loose: false, round: 0.0 };
        let (tris, outline) = curve(&roof, &z);
        assert_eq!(outline.len(), 6);
        // the remesh covers both panels
        let area: f32 = tris.iter().map(|t| (Vec3::from(t.pts[1]) - Vec3::from(t.pts[0])).cross(Vec3::from(t.pts[2]) - Vec3::from(t.pts[0])).length() / 2.0).sum();
        assert!(area > 0.95 && area < 1.0, "{area}");
        let pts: Vec<Vec3> = tris.iter().flat_map(|t| t.pts.iter()).map(|&p| Vec3::from(p)).collect();
        let ridge = pts.iter().filter(|p| (p.z - 0.5).abs() < 0.13).fold(f32::MIN, |m, p| m.max(p.y));
        assert!(ridge < s - 1e-3, "{ridge} vs {s}");
        // the gable ends (z 0 and 1) and the eaves keep their straight lines
        let on_plane = |p: &Vec3| (p.y - s * (1.0 - (p.x + 0.5 - c).abs() / c)).abs() < 1e-4;
        assert!(pts.iter().filter(|p| p.z.abs() < 1e-5 || (p.z - 1.0).abs() < 1e-5).all(on_plane));
        assert!(pts.iter().filter(|p| (p.x + 0.5).abs() < 1e-5).all(|p| p.y.abs() < 1e-5));
        let mut edges: HashMap<[[i32; 3]; 2], i32> = HashMap::new();
        for t in &tris {
            for i in 0..3 {
                let (a, b) = (weld_key(t.pts[i]), weld_key(t.pts[(i + 1) % 3]));
                *edges.entry(if a < b { [a, b] } else { [b, a] }).or_default() += 1;
            }
        }
        let x1 = ((-0.5 + 2.0 * c) * 1000.0).round() as i32;
        let rim = |k: &[i32; 3]| k[2] == 0 || k[2] == 1000 || k[0] == -500 || k[0] == x1;
        assert!(edges.iter().all(|(e, &n)| n == 2 || (n == 1 && rim(&e[0]) && rim(&e[1]))));
    }

    /// the same roof, its ends on the outline too: the outline holds its points only, not the
    /// panels' slopes, so the ridge also rounds next to the ends (holding the slopes there
    /// rippled the fenders, #171 phase 4), and `loose` changes nothing
    #[test]
    fn outline_holds_points_not_slopes() {
        let (s, c) = (15f32.to_radians().sin() * 0.5, 15f32.to_radians().cos() * 0.5);
        let roof = [
            face(&[[-0.5, 0.0, 0.0], [-0.5, 0.0, 1.0], [-0.5 + c, s, 1.0], [-0.5 + c, s, 0.0]], [-s, c, 0.0]),
            face(&[[-0.5 + c, s, 0.0], [-0.5 + c, s, 1.0], [-0.5 + 2.0 * c, 0.0, 1.0], [-0.5 + 2.0 * c, 0.0, 0.0]], [s, c, 0.0]),
        ];
        let ridge = |loose: bool| {
            let z = Smooth { from: vec![[200; 3]], lo: vec![[-9.0; 3]], hi: vec![[9.0; 3]], max_deg: 50.0, step: 0.08, loose, round: 0.0 };
            let (tris, _) = curve(&roof, &z);
            tris.iter().flat_map(|t| t.pts.iter()).filter(|p| p[2] > 0.01 && p[2] < 0.2).fold(f32::MIN, |m, p| m.max(p[1]))
        };
        let (held, loose) = (ridge(false), ridge(true));
        assert!(held < s - 1e-3 && (held - loose).abs() < 1e-6, "{held} vs {loose}, ridge {s}");
    }

    /// the same roof with springs (`round`): the ridge still rounds (it sinks below the fold)
    /// but less than the free plate's, since the plate follows the facets beyond `round`; the
    /// outline stays put either way
    #[test]
    fn round_keeps_the_bulge() {
        let (s, c) = (15f32.to_radians().sin() * 0.5, 15f32.to_radians().cos() * 0.5);
        let roof = [
            face(&[[-0.5, 0.0, 0.0], [-0.5, 0.0, 1.0], [-0.5 + c, s, 1.0], [-0.5 + c, s, 0.0]], [-s, c, 0.0]),
            face(&[[-0.5 + c, s, 0.0], [-0.5 + c, s, 1.0], [-0.5 + 2.0 * c, 0.0, 1.0], [-0.5 + 2.0 * c, 0.0, 0.0]], [s, c, 0.0]),
        ];
        let ridge = |round: f32| {
            let z = Smooth { from: vec![[200; 3]], lo: vec![[-9.0; 3]], hi: vec![[9.0; 3]], max_deg: 50.0, step: 0.08, loose: false, round };
            let (tris, _) = curve(&roof, &z);
            let pts: Vec<[f32; 3]> = tris.iter().flat_map(|t| t.pts.iter().copied()).collect();
            // the outline's corners stay
            assert!(pts.iter().any(|p| (Vec3::from(*p) - Vec3::new(-0.5 + c, s, 1.0)).length() < 1e-5));
            // the ridge's height mid-way along it
            pts.iter().filter(|p| (p[2] - 0.5).abs() < 0.05 && (p[0] - (-0.5 + c)).abs() < 0.03).fold(f32::MIN, |m, p| m.max(p[1]))
        };
        let (free, held) = (ridge(0.0), ridge(0.15));
        assert!(held > free + 1e-3 && held < s - 1e-3, "{held} vs free {free}, ridge {s}");
    }

    /// a face across the centre line gets a lattice symmetric in x, with a row on the line (its
    /// two halves then fair alike); one on a side keeps its rows half a step off the origin
    #[test]
    fn lattice_across_the_centre_is_symmetric() {
        let sym = |pts: &[Vec3]| pts.iter().all(|p| pts.iter().any(|m| (m.x + p.x).abs() < 1e-4 && (m.z - p.z).abs() < 1e-4));
        let row = 0.05 * 3f32.sqrt() / 2.0;
        let q = Poly::new(&face(&[[-0.3, 0.0, 0.0], [0.3, 0.0, 0.0], [0.3, 0.0, 1.0], [-0.3, 0.0, 1.0]], [0.0, 1.0, 0.0]));
        let across = lattice(&q, &q.p, 0.05);
        assert!(across.len() > 50 && sym(&across) && across.iter().any(|p| p.x.abs() < 1e-5), "{}", across.len());
        let q = Poly::new(&face(&[[0.2, 0.0, 0.0], [0.8, 0.0, 0.0], [0.8, 0.0, 1.0], [0.2, 0.0, 1.0]], [0.0, 1.0, 0.0]));
        let side = lattice(&q, &q.p, 0.05);
        assert!(side.len() > 50 && side.iter().all(|p| ((p.x / row).fract().abs() - 0.5).abs() < 1e-3), "{side:?}");
    }

    /// a square whose bottom edge carries cut points every 2 cm: the lattice remesh keeps every
    /// triangle short (ear clipping fanned the cut points to the far corners), covers the whole
    /// square, and keeps the ring's points with their indices
    #[test]
    fn flat_mesh_has_no_slivers() {
        // the other edges cut at the lattice's spacing, as `smooth` cuts the edges between two
        // remeshed faces (an uncut edge stays one long edge of the triangles along it)
        let mut pts: Vec<Vec3> = (0..20).map(|i| Vec3::new(i as f32 * 0.02, 0.0, 0.0)).collect();
        pts.extend((0..10).map(|i| Vec3::new(0.4, 0.0, i as f32 * 0.04)));
        pts.extend((0..10).map(|i| Vec3::new(0.4 - i as f32 * 0.04, 0.0, 0.4)));
        pts.extend((0..10).map(|i| Vec3::new(0.0, 0.0, 0.4 - i as f32 * 0.04)));
        let (p, tris) = flat_mesh(&pts, Vec3::Y, 0.04);
        assert!(p.iter().filter(|q| q.1.is_none()).count() > 20);
        // the ring comes first, CCW round the normal (reversed here), each point with its index
        assert!((0..pts.len()).all(|i| p[..pts.len()].iter().any(|q| q.1 == Some(i) && q.0 == pts[i])));
        let area: f32 = tris.iter().map(|t| (p[t[1]].0 - p[t[0]].0).cross(p[t[2]].0 - p[t[0]].0).length() / 2.0).sum();
        assert!((area - 0.16).abs() < 1e-4, "{area}");
        let longest = tris.iter().flat_map(|t| [(t[0], t[1]), (t[1], t[2]), (t[2], t[0])]).map(|(a, b)| p[a].0.distance(p[b].0)).fold(0.0, f32::max);
        assert!(longest < 0.1, "{longest}");
        // mirrored in x, the mesh mirrors
        let mirrored: Vec<Vec3> = pts.iter().rev().map(|p| *p * Vec3::new(-1.0, 1.0, 1.0)).collect();
        let (m, _) = flat_mesh(&mirrored, Vec3::Y, 0.04);
        assert!(p.iter().all(|q| m.iter().any(|r| (r.0 + q.0 * Vec3::new(1.0, -1.0, -1.0)).abs().max_element() < 1e-5)));
    }

    /// the two faces on an edge walk it in opposite directions and still get the same cut
    /// points, bit for bit (the Serpent's windscreen fold welded apart, #174 phase 3)
    #[test]
    fn cut_points_match_both_ways() {
        let (a, b) = (Vec3::new(0.0, 1.138, 0.406), Vec3::new(0.25, 0.805, 1.315));
        for n in 1..60 {
            for i in 0..=n {
                assert_eq!(edge_point(a, b, i, n).to_array(), edge_point(b, a, n - i, n).to_array());
            }
        }
        let tri = |p: [Vec3; 3]| face(&p.map(|q| q.to_array()), [0.0, 1.0, 0.0]);
        let (l, r) = (tri([a, b, Vec3::new(-0.25, 0.805, 1.315)]), tri([b, a, Vec3::new(0.5, 0.8, 0.4)]));
        let keys = |f: &Face| -> HashSet<[i32; 3]> { refine(std::slice::from_ref(f), 0.025, false)[0].pts.iter().map(|&p| weld_key(p)).collect() };
        // every cut point of the shared edge (and its two corners) welds to the other face's
        let n = cuts(weld_key(a.to_array()), weld_key(b.to_array()), 0.025);
        assert_eq!(keys(&l).intersection(&keys(&r)).count(), n + 1);
    }

    /// mean value coordinates sum to 1 and reproduce the point (affine precision), either winding
    #[test]
    fn mean_value_reproduces_the_point() {
        let ring = [Vec2::new(0.0, 0.0), Vec2::new(1.0, 0.0), Vec2::new(1.2, 0.8), Vec2::new(0.5, 1.0), Vec2::new(-0.1, 0.6)];
        let rev: Vec<Vec2> = ring.iter().rev().copied().collect();
        for q in [Vec2::new(0.5, 0.4), Vec2::new(0.9, 0.3), Vec2::new(0.2, 0.55)] {
            for r in [&ring[..], &rev[..]] {
                let w = mean_value(r, q);
                assert!((w.iter().sum::<f32>() - 1.0).abs() < 1e-5);
                assert!(w.iter().all(|&x| x > 0.0));
                let back: Vec2 = w.iter().zip(r).map(|(&w, &v)| v * w).sum();
                assert!(back.distance(q) < 1e-5, "{back} vs {q}");
            }
        }
    }
}
