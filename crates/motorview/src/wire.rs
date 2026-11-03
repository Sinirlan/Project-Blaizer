//! DEBUG (motorview): wireframe styles for spotting faulty geometry (00-debuger "Wire styles").
//!
//! `plain` turns a triangle list into welded unique edges plus a grey fill; `analyse` also
//! classifies every triangle and edge (mesh analysis, like Blender's): slivers, tiny and
//! flipped triangles, boundary and non-manifold edges, creases. Pure CPU, no wgpu.

use crate::scene::Vertex;
use glam::Vec3;
use std::collections::HashMap;

/// F4 cycles these.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum WireStyle {
    #[default]
    Off,
    /// flat edge colours, each edge once
    Lines,
    /// grey head-light shaded fill under the edges (hidden-line), back faces red
    Solid,
    /// Solid plus the mesh analysis colours (cars only)
    Check,
}

impl WireStyle {
    pub const ALL: [WireStyle; 4] = [WireStyle::Off, WireStyle::Lines, WireStyle::Solid, WireStyle::Check];

    pub fn name(self) -> &'static str {
        match self {
            WireStyle::Off => "off",
            WireStyle::Lines => "lines",
            WireStyle::Solid => "solid",
            WireStyle::Check => "check",
        }
    }

    pub fn parse(s: &str) -> Option<WireStyle> {
        WireStyle::ALL.into_iter().find(|w| w.name() == s)
    }

    pub fn next(self) -> WireStyle {
        WireStyle::ALL[(self as usize + 1) % 4]
    }

    pub fn prev(self) -> WireStyle {
        WireStyle::ALL[(self as usize + 3) % 4]
    }

    pub fn on(self) -> bool {
        self != WireStyle::Off
    }

    /// draws the grey / analysed fill under the edges
    pub fn fill(self) -> bool {
        matches!(self, WireStyle::Solid | WireStyle::Check)
    }
}

/// What the analysis found in one mesh.
#[derive(Clone, Copy, Default, PartialEq, Eq, Debug)]
pub struct WireStats {
    pub tris: usize,
    /// smallest angle under `SLIVER_DEG`, but not a strip
    pub slivers: usize,
    /// slivers in pairs that make a thin flat quad with normals varying only across it (bevel strips)
    pub strips: usize,
    /// area under `TINY_AREA`
    pub tiny: usize,
    /// winding against the stored vertex normals
    pub flipped: usize,
    /// edges used by one triangle (holes, open strips)
    pub boundary: usize,
    /// edges used by three or more triangles (folded strips, doubled faces)
    pub nonmanifold: usize,
}

/// A run of overlay text: plain (`None`) or in a colour.
pub type Span = (String, Option<[f32; 4]>);

impl WireStats {
    /// The counts line as runs; each class is in the colour the mesh draws it with, so the
    /// line is the legend of the Check colours.
    pub fn spans(&self) -> Vec<Span> {
        let plain = |s: String| (s, None);
        let class = |s: String, c: [f32; 4]| (s, Some(c));
        vec![
            plain(format!("mesh check: {} tris, ", self.tris)),
            class(format!("slivers {}", self.slivers), SLIVER),
            plain(", ".into()),
            class(format!("strips {}", self.strips), STRIP),
            plain(", ".into()),
            class(format!("tiny {}", self.tiny), TINY),
            plain(", ".into()),
            class(format!("flipped {}", self.flipped), FLIPPED),
            plain(", ".into()),
            class(format!("boundary edges {}", self.boundary), BOUNDARY),
            plain(", ".into()),
            class(format!("non-manifold {}", self.nonmanifold), NONMANIFOLD),
        ]
    }

    pub fn line(&self) -> String {
        self.spans().into_iter().map(|s| s.0).collect()
    }
}

/// The edge, fill and x-ray vertex lists of one mesh (mesh space, like the source triangles).
#[derive(Default)]
pub struct WireSet {
    pub lines: Vec<Vertex>,
    /// drawn without the depth test (non-manifold edges)
    pub xray: Vec<Vertex>,
    pub fill: Vec<Vertex>,
    pub stats: WireStats,
}

/// Smallest triangle angle (degrees) below which a triangle is a sliver.
pub const SLIVER_DEG: f32 = 5.0;
/// Tolerance (degrees) of a strip pair: fold across the diagonal, long sides parallel, the
/// vertex normals along each long side. 10, not 5: strips on curved panels turn 5-10 degrees
/// along a 5 cm segment; the fans that streak sweep 40 and more (Titan, #224).
pub const STRIP_DEG: f32 = 10.0;
/// Narrowest strip half (m), its height over its longest edge: 0.2 mm. Thinner pairs are
/// near-degenerate (collinear caps), not bevels.
pub const STRIP_MIN_WIDTH: f32 = 2e-4;
/// Triangle area (m²) below which it is tiny: 0.1 mm².
pub const TINY_AREA: f32 = 1e-7;
/// Dihedral angle (degrees) from which an edge is a crease, not the lattice's own edge.
pub const CREASE_DEG: f32 = 2.0;

/// dark slate: reads against the sky and the yellow / orange / red marks
pub const FILL: [f32; 4] = [0.27, 0.30, 0.35, 1.0];
const SLIVER: [f32; 4] = [1.0, 0.9, 0.2, 1.0];
/// a dimmer sliver yellow: harmless bevel strips
const STRIP: [f32; 4] = [0.55, 0.50, 0.30, 1.0];
const TINY: [f32; 4] = [1.0, 0.5, 0.1, 1.0];
const FLIPPED: [f32; 4] = [1.0, 0.2, 0.9, 1.0];
const FLAT_EDGE: [f32; 4] = [0.42, 0.45, 0.52, 1.0];
const BOUNDARY: [f32; 4] = [1.0, 0.45, 0.0, 1.0];
const NONMANIFOLD: [f32; 4] = [1.0, 0.05, 0.05, 1.0];

/// 0.1 mm, the tolerance of the body's watertight test (procgen/body.rs).
fn weld(p: [f32; 3]) -> [i32; 3] {
    p.map(|c| (c * 1e4).round() as i32)
}

/// A line vertex: flat colour, no normal (the flat shader does not light it).
fn line_vertex(v: Vertex, color: [f32; 4]) -> Vertex {
    Vertex { color, layer: -1.0, env: 0.0, normal: [0.0; 3], uv: [0.0; 2], ..v }
}

fn fill_vertex(v: Vertex, color: [f32; 4]) -> Vertex {
    Vertex { color, layer: -1.0, env: 0.0, uv: [0.0; 2], ..v }
}

/// Edges of a triangle list with zero normals (flat), shared edges drawn twice: for the
/// per-frame prop edges, where welding every frame is not worth it.
pub fn flat_edges(tris: &[Vertex], color: [f32; 4]) -> Vec<Vertex> {
    let mut out = Vec::with_capacity(tris.len() * 2);
    for t in tris.as_chunks::<3>().0 {
        for k in 0..3 {
            out.push(line_vertex(t[k], color));
            out.push(line_vertex(t[(k + 1) % 3], color));
        }
    }
    out
}

struct Edge {
    n: u32,
    tris: [u32; 2],
    v: [Vertex; 2],
}

struct Mesh<'a> {
    tris: &'a [[Vertex; 3]],
    /// per triangle: the geometric normal (zero if degenerate)
    normal: Vec<Vec3>,
    /// per triangle side k (corner k to k+1): its edge, `u32::MAX` for a collapsed side
    tri_edges: Vec<[u32; 3]>,
    edges: Vec<Edge>,
}

fn build(tris: &[Vertex]) -> Mesh<'_> {
    let tris = tris.as_chunks::<3>().0;
    let mut ids: HashMap<[i32; 3], u32> = HashMap::new();
    let mut map: HashMap<(u32, u32), usize> = HashMap::new();
    let mut edges: Vec<Edge> = Vec::new();
    let mut normal = Vec::with_capacity(tris.len());
    let mut tri_edges = Vec::with_capacity(tris.len());
    for (ti, t) in tris.iter().enumerate() {
        let p = t.map(|v| Vec3::from(v.pos));
        normal.push((p[1] - p[0]).cross(p[2] - p[0]).normalize_or_zero());
        let id = [0, 1, 2].map(|k| {
            let len = ids.len() as u32;
            *ids.entry(weld(t[k].pos)).or_insert(len)
        });
        let mut te = [u32::MAX; 3];
        for k in 0..3 {
            let (a, b) = (id[k], id[(k + 1) % 3]);
            if a == b {
                continue;
            }
            let key = (a.min(b), a.max(b));
            te[k] = match map.get(&key) {
                Some(&ei) => {
                    let e = &mut edges[ei];
                    if e.n == 1 {
                        e.tris[1] = ti as u32;
                    }
                    e.n += 1;
                    ei as u32
                }
                None => {
                    map.insert(key, edges.len());
                    edges.push(Edge { n: 1, tris: [ti as u32, 0], v: [t[k], t[(k + 1) % 3]] });
                    edges.len() as u32 - 1
                }
            };
        }
        tri_edges.push(te);
    }
    Mesh { tris, normal, tri_edges, edges }
}

fn grey_fill(m: &Mesh) -> Vec<Vertex> {
    m.tris.iter().flatten().map(|&v| fill_vertex(v, FILL)).collect()
}

/// Welded unique edges in `color` and the grey fill (Lines and Solid).
pub fn plain(tris: &[Vertex], color: [f32; 4]) -> WireSet {
    let m = build(tris);
    let lines = m.edges.iter().flat_map(|e| e.v.map(|v| line_vertex(v, color))).collect();
    WireSet { lines, xray: Vec::new(), fill: grey_fill(&m), stats: WireStats { tris: m.tris.len(), ..Default::default() } }
}

/// One triangle's classes before the strips are paired.
#[derive(Clone, Copy)]
struct Tri {
    flipped: bool,
    tiny: bool,
    sliver: bool,
    /// the side (corner k to k+1) of the shortest edge
    short: usize,
    /// height over the longest edge (m)
    width: f32,
}

fn classify(t: &[Vertex; 3], gn: Vec3) -> Tri {
    let p = t.map(|v| Vec3::from(v.pos));
    let c = (p[1] - p[0]).cross(p[2] - p[0]);
    let area = c.length() / 2.0;
    let vn: Vec3 = t.iter().map(|v| Vec3::from(v.normal)).sum();
    let flipped = vn.length_squared() > 1e-8 && c.dot(vn) < 0.0;
    let tiny = area < TINY_AREA;
    // smallest angle from the edge vectors (a degenerate triangle has none worth measuring)
    let min_angle = if tiny || gn == Vec3::ZERO {
        0.0
    } else {
        (0..3)
            .map(|k| {
                let (a, b) = ((p[(k + 1) % 3] - p[k]).normalize_or_zero(), (p[(k + 2) % 3] - p[k]).normalize_or_zero());
                a.dot(b).clamp(-1.0, 1.0).acos().to_degrees()
            })
            .fold(180.0, f32::min)
    };
    let len = [0, 1, 2].map(|k| p[k].distance(p[(k + 1) % 3]));
    let short = (0..3).min_by(|&a, &b| len[a].total_cmp(&len[b])).unwrap_or(0);
    let width = 2.0 * area / len.into_iter().fold(f32::MIN_POSITIVE, f32::max);
    Tri { flipped, tiny, sliver: !tiny && min_angle < SLIVER_DEG, short, width }
}

/// A sliver that is one half of a thin flat quad (a bevel strip), its vertex normals changing
/// only across the strip: harmless to shade, unlike a fan that sweeps the normals along its
/// length. Either of its two long edges may be the quad's diagonal: in a tapered strip a long
/// side can outgrow the diagonal.
fn strip(m: &Mesh, info: &[Tri], t: usize) -> bool {
    (0..3).filter(|&k| k != info[t].short).any(|k| strip_across(m, info, t, k))
}

/// `strip` with the twin across side `tk` of `t`.
fn strip_across(m: &Mesh, info: &[Tri], t: usize, tk: usize) -> bool {
    let cos = STRIP_DEG.to_radians().cos();
    let ei = m.tri_edges[t][tk];
    if ei == u32::MAX || m.edges[ei as usize].n != 2 {
        return false;
    }
    let e = &m.edges[ei as usize];
    let u = (if e.tris[0] as usize == t { e.tris[1] } else { e.tris[0] }) as usize;
    // the shared edge is long in the twin too (its short edge is the strip's other end)
    let uk = (0..3).find(|&k| m.tri_edges[u][k] == ei).unwrap_or(info[u].short);
    let thick = |i: usize| info[i].width >= STRIP_MIN_WIDTH;
    if uk == info[u].short || info[u].flipped || !thick(t) || !thick(u) || m.normal[t].dot(m.normal[u]) < cos {
        return false;
    }
    // each half's long side runs from the shared edge's end away from its apex to the apex
    let side = |i: usize, k: usize| {
        let tri = &m.tris[i];
        let apex = tri[(k + 2) % 3];
        let d = |v: Vertex| Vec3::from(v.pos).distance_squared(Vec3::from(apex.pos));
        let (a, b) = (tri[k], tri[(k + 1) % 3]);
        (if d(a) > d(b) { a } else { b }, apex)
    };
    let ((x, c), (y, d)) = (side(t, tk), side(u, uk));
    // the same far end is a kite (both apexes at one end), not a quad
    if weld(x.pos) == weld(y.pos) {
        return false;
    }
    let dir = |a: Vertex, b: Vertex| (Vec3::from(b.pos) - Vec3::from(a.pos)).normalize_or_zero();
    let same_normal = |a: Vertex, b: Vertex| Vec3::from(a.normal).normalize_or_zero().dot(Vec3::from(b.normal).normalize_or_zero()) >= cos;
    dir(x, c).dot(dir(y, d)).abs() >= cos && same_normal(x, c) && same_normal(y, d)
}

/// Mesh analysis (Check): classified fill and edges, and the counts.
pub fn analyse(tris: &[Vertex]) -> WireSet {
    let m = build(tris);
    let mut stats = WireStats { tris: m.tris.len(), ..Default::default() };
    let info: Vec<Tri> = m.tris.iter().zip(&m.normal).map(|(t, &gn)| classify(t, gn)).collect();
    let mut fill = Vec::with_capacity(tris.len());
    for (i, (t, c)) in m.tris.iter().zip(&info).enumerate() {
        let strip = c.sliver && !c.flipped && strip(&m, &info, i);
        stats.flipped += c.flipped as usize;
        stats.tiny += c.tiny as usize;
        stats.strips += strip as usize;
        stats.slivers += (c.sliver && !strip) as usize;
        let colour = if c.flipped {
            FLIPPED
        } else if c.tiny {
            TINY
        } else if strip {
            STRIP
        } else if c.sliver {
            SLIVER
        } else {
            FILL
        };
        fill.extend(t.iter().map(|&v| fill_vertex(v, colour)));
    }
    let (mut lines, mut xray) = (Vec::new(), Vec::new());
    for e in &m.edges {
        let (colour, list) = match e.n {
            1 => {
                stats.boundary += 1;
                (BOUNDARY, &mut lines)
            }
            2 => {
                let (a, b) = (m.normal[e.tris[0] as usize], m.normal[e.tris[1] as usize]);
                let deg = a.dot(b).clamp(-1.0, 1.0).acos().to_degrees();
                // flat edges are faint; a crease gets brighter with its angle, a fold white
                let colour = if deg < CREASE_DEG {
                    FLAT_EDGE
                } else {
                    let k = (deg / 90.0).clamp(0.0, 1.0);
                    let g = 0.55 * (1.0 - k) + 1.0 * k;
                    [g, g, g, 1.0]
                };
                (colour, &mut lines)
            }
            _ => {
                stats.nonmanifold += 1;
                (NONMANIFOLD, &mut xray)
            }
        };
        list.extend(e.v.map(|v| line_vertex(v, colour)));
    }
    WireSet { lines, xray, fill, stats }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(p: [f32; 3], n: [f32; 3]) -> Vertex {
        Vertex { pos: p, uv: [0.0; 2], color: [1.0; 4], layer: -1.0, normal: n, env: 0.0 }
    }

    /// a closed tetrahedron, wound to face along the normals (c·n > 0, body.rs `tri`)
    fn tetra() -> Vec<Vertex> {
        let p = [Vec3::new(0.0, 0.0, 0.0), Vec3::new(1.0, 0.0, 0.0), Vec3::new(0.0, 1.0, 0.0), Vec3::new(0.0, 0.0, 1.0)];
        let centre = p.iter().sum::<Vec3>() / 4.0;
        let mut out = Vec::new();
        for f in [[0, 1, 2], [0, 3, 1], [0, 2, 3], [1, 3, 2]] {
            let q = f.map(|i| p[i]);
            let n = (q[1] - q[0]).cross(q[2] - q[0]).normalize();
            let n = if n.dot((q[0] + q[1] + q[2]) / 3.0 - centre) < 0.0 { -n } else { n };
            let order = if (q[1] - q[0]).cross(q[2] - q[0]).dot(n) > 0.0 { [0, 1, 2] } else { [0, 2, 1] };
            out.extend(order.map(|k| v(q[k].into(), n.into())));
        }
        out
    }

    #[test]
    fn closed_tetra_is_clean() {
        let s = analyse(&tetra()).stats;
        assert_eq!(s, WireStats { tris: 4, ..Default::default() });
    }

    #[test]
    fn shared_edges_drawn_once() {
        let w = plain(&tetra(), [1.0; 4]);
        assert_eq!(w.lines.len() / 2, 6);
        assert_eq!(w.fill.len(), 12);
    }

    #[test]
    fn reversed_triangle_is_flipped() {
        let mut t = tetra();
        t.swap(1, 2);
        assert_eq!(analyse(&t).stats.flipped, 1);
    }

    #[test]
    fn sliver_and_tiny_and_boundary() {
        let n = [0.0, 0.0, 1.0];
        // a 1-degree apex over a 1 m base: area under half a square metre but a long thin spike
        let sliver = vec![v([0.0, 0.0, 0.0], n), v([1.0, 0.0, 0.0], n), v([1.0, 0.0175, 0.0], n)];
        let s = analyse(&sliver).stats;
        assert_eq!((s.slivers, s.tiny, s.boundary), (1, 0, 3));
        let tiny = vec![v([0.0, 0.0, 0.0], n), v([0.0003, 0.0, 0.0], n), v([0.0, 0.0003, 0.0], n)];
        assert_eq!(analyse(&tiny).stats.tiny, 1);
    }

    /// a 4 cm x 2 mm quad in z = 0 split along its long diagonal (0,0)-(4 cm, 2 mm); `n(x, y)`
    /// gives each corner's normal
    fn quad(n: impl Fn(f32, f32) -> [f32; 3]) -> Vec<Vertex> {
        let c = |x: f32, y: f32| v([x, y, 0.0], n(x, y));
        let (l, w) = (0.04, 0.002);
        vec![c(0.0, 0.0), c(l, 0.0), c(l, w), c(0.0, 0.0), c(l, w), c(0.0, w)]
    }

    #[test]
    fn bevel_strip_is_not_a_sliver() {
        // normals turn across the strip (a bevel between two faces), not along it
        let across = |_: f32, y: f32| Vec3::new(0.0, -0.5 + y / 0.002, 1.0).normalize().into();
        let s = analyse(&quad(across)).stats;
        assert_eq!((s.strips, s.slivers), (2, 0));
        // normals that sweep along it shade a streak: still slivers
        let along = |x: f32, _: f32| Vec3::new(-0.5 + x / 0.04, 0.0, 1.0).normalize().into();
        let s = analyse(&quad(along)).stats;
        assert_eq!((s.strips, s.slivers), (0, 2));
    }

    #[test]
    fn kite_is_not_a_strip() {
        // both apexes near the same end of the diagonal: a thin kite, no parallel long sides
        let n = [0.0, 0.0, 1.0];
        let t = vec![
            v([0.0, 0.0, 0.0], n),
            v([0.04, 0.0, 0.0], n),
            v([0.002, 0.001, 0.0], n),
            v([0.0, 0.0, 0.0], n),
            v([0.002, -0.001, 0.0], n),
            v([0.04, 0.0, 0.0], n),
        ];
        let s = analyse(&t).stats;
        assert_eq!((s.strips, s.slivers), (0, 2));
    }

    #[test]
    fn three_triangles_on_one_edge_are_nonmanifold() {
        let n = [0.0, 1.0, 0.0];
        let mut t = Vec::new();
        for z in [1.0f32, -1.0, 2.0] {
            t.extend([v([0.0, 0.0, 0.0], n), v([1.0, 0.0, 0.0], n), v([0.5, z.abs().max(0.5), z], n)]);
        }
        let w = analyse(&t);
        assert_eq!(w.stats.nonmanifold, 1);
        assert_eq!(w.xray.len(), 2);
    }

    #[test]
    fn counts_line_is_plain_text_of_the_spans() {
        let s = WireStats { tris: 10, slivers: 2, strips: 5, tiny: 0, flipped: 1, boundary: 3, nonmanifold: 4 };
        assert_eq!(s.line(), "mesh check: 10 tris, slivers 2, strips 5, tiny 0, flipped 1, boundary edges 3, non-manifold 4");
        assert_eq!(s.spans().iter().filter(|(_, c)| c.is_some()).count(), 6);
    }

    #[test]
    fn style_cycle() {
        assert_eq!(WireStyle::Check.next(), WireStyle::Off);
        assert_eq!(WireStyle::Off.prev(), WireStyle::Check);
        assert_eq!(WireStyle::parse("solid"), Some(WireStyle::Solid));
    }
}
