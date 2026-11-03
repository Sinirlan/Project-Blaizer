//! REMASTER modern front end (09-remaster 9.24): vector shapes for the menu (dial, glass plate,
//! needle, dots), tessellated with lyon into coloured triangles that `Batch::mesh` draws in the
//! UI pass. Our own addition, not game data [design choice].
//!
//! The UI pass has no multisampling, so edges are antialiased here: a filled shape gets a
//! one-sided feather ring (inner vertices at the contour with the fill's alpha, outer ones a
//! pixel out with alpha 0), and a stroke is stacked from three strokes of falling width, so its
//! stair-steps are spread over three low-contrast levels. Glows are the same stack, wider.

use lyon_tessellation::math::point;
use lyon_tessellation::path::Path;
use lyon_tessellation::{
    BuffersBuilder, FillOptions, FillTessellator, FillVertex, FillVertexConstructor, LineCap, LineJoin, Side, StrokeOptions, StrokeTessellator, StrokeVertex, StrokeVertexConstructor,
    VertexBuffers,
};
use std::sync::OnceLock;

pub type P = [f32; 2];
pub type Rgba = [f32; 4];

#[derive(Clone, Copy, Debug)]
pub struct V {
    pub pos: P,
    pub color: Rgba,
}

/// Triangles with a colour per vertex, in canvas pixels.
#[derive(Default)]
pub struct Mesh {
    pub verts: Vec<V>,
    pub idx: Vec<u32>,
    /// REMASTER: glow sources (crisp shapes). The GPU blurs them (`Gpu::draw_glow`) and adds
    /// the result over the batch, so a glow is one gaussian, not a stack of strokes.
    pub glow_verts: Vec<V>,
    pub glow_idx: Vec<u32>,
}

/// strokes stacked per antialiased stroke edge (`Mesh::stroke`)
const STROKE_AA_LAYERS: usize = 6;

/// Peak alpha of a blurred glow source against the old stacked strokes' `color[3]`
const GLOW_GAIN: f32 = 1.4;

/// How a stroke ends and turns.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Ends {
    /// flat caps, mitred corners (ticks)
    Butt,
    /// round caps and joins (rim, needle outline)
    Round,
}

fn path(pts: &[P], closed: bool) -> Path {
    let mut b = Path::builder();
    b.begin(point(pts[0][0], pts[0][1]));
    for p in &pts[1..] {
        b.line_to(point(p[0], p[1]));
    }
    b.end(closed);
    b.build()
}

/// shoelace area (sign = winding)
fn area(pts: &[P]) -> f32 {
    let mut a = 0.0;
    for (i, p) in pts.iter().enumerate() {
        let q = pts[(i + 1) % pts.len()];
        a += p[0] * q[1] - q[0] * p[1];
    }
    a * 0.5
}

/// Which side of an eastbound lyon stroke is `Side::Positive`, as the sign of its y offset.
fn positive_y() -> f32 {
    static S: OnceLock<f32> = OnceLock::new();
    *S.get_or_init(|| {
        struct C;
        impl StrokeVertexConstructor<(Side, f32)> for C {
            fn new_vertex(&mut self, v: StrokeVertex) -> (Side, f32) {
                (v.side(), v.position().y)
            }
        }
        let mut buf: VertexBuffers<(Side, f32), u32> = VertexBuffers::new();
        let p = path(&[[0.0, 0.0], [10.0, 0.0]], false);
        let _ = StrokeTessellator::new().tessellate_path(&p, &StrokeOptions::default().with_line_width(2.0), &mut BuffersBuilder::new(&mut buf, C));
        buf.vertices.iter().find(|(s, _)| *s == Side::Positive).map_or(1.0, |&(_, y)| y.signum())
    })
}

struct FillCtor<'a>(&'a dyn Fn(P) -> Rgba);
impl FillVertexConstructor<V> for FillCtor<'_> {
    fn new_vertex(&mut self, v: FillVertex) -> V {
        let p = v.position();
        V { pos: [p.x, p.y], color: (self.0)([p.x, p.y]) }
    }
}

struct StrokeCtor<F: Fn(&StrokeVertex) -> V>(F);
impl<F: Fn(&StrokeVertex) -> V> StrokeVertexConstructor<V> for StrokeCtor<F> {
    fn new_vertex(&mut self, v: StrokeVertex) -> V {
        (self.0)(&v)
    }
}

/// `pts` without consecutive coincident points. A rounded rectangle whose radius is half its height has a zero-length
/// side at each end; the two points there have no usable tangent, and the outline's normals (`soft_pad`, `halo`) turn
/// to zigzag, which folds the glow's triangles over each other and draws a bright line through it.
fn dedup(pts: &[P]) -> Vec<P> {
    let mut out: Vec<P> = Vec::with_capacity(pts.len());
    for &p in pts {
        if out.last().is_none_or(|l| (l[0] - p[0]).abs() > 1e-3 || (l[1] - p[1]).abs() > 1e-3) {
            out.push(p);
        }
    }
    if out.len() > 1 && out[0] == out[out.len() - 1] {
        out.pop();
    }
    out
}

impl Mesh {
    pub fn clear(&mut self) {
        self.verts.clear();
        self.idx.clear();
        self.glow_verts.clear();
        self.glow_idx.clear();
    }

    /// Run `f` on a scratch mesh and keep what it draws as glow sources.
    fn as_glow(&mut self, f: impl FnOnce(&mut Mesh)) {
        let (v, i) = (std::mem::take(&mut self.verts), std::mem::take(&mut self.idx));
        f(self);
        let base = self.glow_verts.len() as u32;
        self.glow_verts.append(&mut self.verts);
        self.glow_idx.extend(self.idx.drain(..).map(|k| k + base));
        (self.verts, self.idx) = (v, i);
    }

    fn append(&mut self, buf: VertexBuffers<V, u32>) {
        let base = self.verts.len() as u32;
        self.verts.extend(buf.vertices);
        self.idx.extend(buf.indices.into_iter().map(|i| i + base));
    }

    /// Fill a closed polygon (nonzero winding, concave is fine); `shade` colours each vertex.
    /// `aa` (canvas px; 0 = none) is the width of the feather ring.
    pub fn fill_with(&mut self, pts: &[P], shade: &dyn Fn(P) -> Rgba, aa: f32) {
        if pts.len() < 3 {
            return;
        }
        let p = path(pts, true);
        let mut buf: VertexBuffers<V, u32> = VertexBuffers::new();
        if FillTessellator::new().tessellate_path(&p, &FillOptions::default().with_tolerance(0.05), &mut BuffersBuilder::new(&mut buf, FillCtor(shade))).is_ok() {
            self.append(buf);
        }
        if aa > 0.0 {
            // feather: positive_y() is the y offset of the Left side of an eastbound path and an
            // eastbound edge of a polygon with positive area has its outside at -y
            let outward_y = if area(pts) > 0.0 { -1.0 } else { 1.0 };
            let outer = if positive_y() == outward_y { Side::Positive } else { Side::Negative };
            let mut buf: VertexBuffers<V, u32> = VertexBuffers::new();
            let ctor = StrokeCtor(|v: &StrokeVertex| {
                let on = v.position_on_path();
                if v.side() == outer {
                    let q = v.position();
                    V { pos: [q.x, q.y], color: {
                        let c = shade([on.x, on.y]);
                        [c[0], c[1], c[2], 0.0]
                    } }
                } else {
                    V { pos: [on.x, on.y], color: shade([on.x, on.y]) }
                }
            });
            let opts = StrokeOptions::default().with_line_width(2.0 * aa).with_line_join(LineJoin::Round).with_tolerance(0.05);
            if StrokeTessellator::new().tessellate_path(&p, &opts, &mut BuffersBuilder::new(&mut buf, ctor)).is_ok() {
                self.append(buf);
            }
        }
    }

    pub fn fill(&mut self, pts: &[P], color: Rgba, aa: f32) {
        self.fill_with(pts, &|_| color, aa);
    }

    /// One plain stroke, no antialiasing; `shade` colours each vertex.
    fn stroke_raw(&mut self, pts: &[P], closed: bool, width: f32, ends: Ends, shade: &dyn Fn(P) -> Rgba) {
        if pts.len() < 2 {
            return;
        }
        let p = path(pts, closed);
        let mut buf: VertexBuffers<V, u32> = VertexBuffers::new();
        let ctor = StrokeCtor(|v: &StrokeVertex| {
            let q = v.position();
            V { pos: [q.x, q.y], color: shade([q.x, q.y]) }
        });
        let (cap, join) = if ends == Ends::Round { (LineCap::Round, LineJoin::Round) } else { (LineCap::Butt, LineJoin::Miter) };
        let opts = StrokeOptions::default().with_line_width(width).with_line_cap(cap).with_line_join(join).with_tolerance(0.05);
        if StrokeTessellator::new().tessellate_path(&p, &opts, &mut BuffersBuilder::new(&mut buf, ctor)).is_ok() {
            self.append(buf);
        }
    }

    /// An antialiased stroke: `STROKE_AA_LAYERS` strokes of falling width, `px` = one window
    /// pixel in canvas units. The layers composite to `color`'s alpha in the core, and the
    /// coverage ramp over 1 px is split into small steps, so edges stay smooth and crisp.
    pub fn stroke(&mut self, pts: &[P], closed: bool, width: f32, ends: Ends, color: Rgba, px: f32) {
        if pts.len() == 2 && !closed {
            self.segment(pts[0], pts[1], width, ends, color, px);
            return;
        }
        // each side's edge runs from +0.75 px (first, faintest) to -0.25 px (the core)
        let n = STROKE_AA_LAYERS;
        let a = color[3];
        let mut covered = 0.0f32;
        for k in 0..n {
            let ext = px * (0.75 - k as f32 / (n - 1) as f32);
            let target = a * (k + 1) as f32 / n as f32;
            let la = ((target - covered) / (1.0 - covered)).clamp(0.0, 1.0);
            covered = target;
            self.stroke_raw(pts, closed, (width + 2.0 * ext).max(0.2 * px), ends, &move |_| [color[0], color[1], color[2], la]);
        }
    }

    /// One antialiased line segment without the tessellator (the wireframes draw thousands):
    /// a strip across the line, alpha 0 | a | a | 0 over one window pixel on each side, and
    /// half a width of extra length at round ends.
    fn segment(&mut self, a: P, b: P, width: f32, ends: Ends, color: Rgba, px: f32) {
        let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
        let len = (dx * dx + dy * dy).sqrt();
        if len < 1e-6 {
            return;
        }
        let (ux, uy) = (dx / len, dy / len);
        let h = (width * 0.5).max(0.2 * px);
        let ext = if ends == Ends::Round { h } else { 0.0 };
        let (a, b) = ([a[0] - ux * ext, a[1] - uy * ext], [b[0] + ux * ext, b[1] + uy * ext]);
        let (nx, ny) = (-uy, ux);
        let clear = [color[0], color[1], color[2], 0.0];
        let base = self.verts.len() as u32;
        for p in [a, b] {
            for (o, c) in [(-(h + px), clear), (-h + 0.5 * px, color), (h - 0.5 * px, color), (h + px, clear)] {
                self.verts.push(V { pos: [p[0] + nx * o, p[1] + ny * o], color: c });
            }
        }
        for k in 0..3 {
            self.idx.extend([base + k, base + k + 1, base + 4 + k, base + k + 1, base + 5 + k, base + 4 + k]);
        }
    }

    /// A soft glow along a path: a stroke of `width + spread` blurred by the GPU (`steps` is
    /// kept for the callers).
    pub fn glow(&mut self, pts: &[P], closed: bool, width: f32, spread: f32, color: Rgba, _steps: usize) {
        let c = [color[0], color[1], color[2], (color[3] * GLOW_GAIN).min(1.0)];
        self.as_glow(|m| m.stroke_raw(pts, closed, width + spread, Ends::Round, &move |_| c));
    }

    /// A smooth glow round a path: `steps` strokes from `width + 2 * radius` down to `width`,
    /// widest first, each alpha chosen so the composited intensity at distance d from the core
    /// edge follows `peak * exp(-4 (d / radius)^2)` (a gaussian-like falloff without visible bands).
    pub fn glow_smooth(&mut self, pts: &[P], width: f32, radius: f32, color: Rgba, _steps: usize) {
        let c = [color[0], color[1], color[2], color[3].min(1.0)];
        self.as_glow(|m| m.stroke_raw(pts, false, width + radius, Ends::Round, &move |_| c));
    }

    /// A soft glow around a filled shape: the shape grown by `spread` in `steps` fills.
    pub fn glow_fill(&mut self, pts: &[P], spread: f32, color: Rgba, _steps: usize) {
        let c = [color[0], color[1], color[2], (color[3] * GLOW_GAIN).min(1.0)];
        self.as_glow(|m| {
            m.fill(pts, c, 0.0);
            m.stroke_raw(pts, true, spread, Ends::Round, &move |_| c);
        });
    }

    /// A smooth glow round a convex shape: solid inside, the alpha falling off smoothly over
    /// `spread` outside, as strips between offset rings (no stacked strokes, so no creases or
    /// noise at the corners). `pts` = the outline; `color[3]` = the alpha of the solid part.
    pub fn soft_pad(&mut self, pts: &[P], spread: f32, color: Rgba) {
        let pts = &dedup(pts);
        let n = pts.len();
        if n < 3 {
            return;
        }
        let c = pts.iter().fold([0.0, 0.0], |a, p| [a[0] + p[0] / n as f32, a[1] + p[1] / n as f32]);
        let normals: Vec<P> = (0..n)
            .map(|i| {
                let (a, b) = (pts[(i + n - 1) % n], pts[(i + 1) % n]);
                let t = [b[0] - a[0], b[1] - a[1]];
                let l = (t[0] * t[0] + t[1] * t[1]).sqrt().max(1e-6);
                let m = [t[1] / l, -t[0] / l];
                if m[0] * (pts[i][0] - c[0]) + m[1] * (pts[i][1] - c[1]) < 0.0 { [-m[0], -m[1]] } else { m }
            })
            .collect();
        const RINGS: usize = 24;
        let inset = 3.0;
        let mut prev: Option<u32> = None;
        let centre = self.verts.len() as u32;
        self.verts.push(V { pos: c, color });
        for k in 0..=RINGS {
            let d = -inset + (spread + inset) * k as f32 / RINGS as f32;
            let u = (d / spread).clamp(0.0, 1.0);
            let a = color[3] * (1.0 - u) * (1.0 - u) * (1.0 - u * u * 0.0);
            let base = self.verts.len() as u32;
            for i in 0..n {
                self.verts.push(V { pos: [pts[i][0] + normals[i][0] * d, pts[i][1] + normals[i][1] * d], color: [color[0], color[1], color[2], a] });
            }
            match prev {
                None => {
                    for i in 0..n as u32 {
                        self.idx.extend([centre, base + i, base + (i + 1) % n as u32]);
                    }
                }
                Some(p) => {
                    for i in 0..n as u32 {
                        let j = (i + 1) % n as u32;
                        self.idx.extend([p + i, base + i, base + j, p + i, base + j, p + j]);
                    }
                }
            }
            prev = Some(base);
        }
    }

    /// A triangle fan (a disc with a colour at its centre and another on its rim).
    pub fn disc(&mut self, c: P, r: f32, inner: Rgba, outer: Rgba, segs: usize) {
        self.ellipse(c, [r, r], inner, outer, segs);
    }

    /// `disc` with radii `r` along x and y (a CSS `radial-gradient(ellipse ..)` with two stops).
    pub fn ellipse(&mut self, c: P, r: [f32; 2], inner: Rgba, outer: Rgba, segs: usize) {
        let base = self.verts.len() as u32;
        self.verts.push(V { pos: c, color: inner });
        for i in 0..=segs {
            let a = i as f32 / segs as f32 * std::f32::consts::TAU;
            self.verts.push(V { pos: [c[0] + r[0] * a.cos(), c[1] + r[1] * a.sin()], color: outer });
        }
        for i in 0..segs as u32 {
            self.idx.extend([base, base + 1 + i, base + 2 + i]);
        }
    }

    /// REMASTER Glass HUD (#294): an annulus between `r0` and `r1` whose colour follows the
    /// angle (`shade`, radians, y down, 0 = 3 o'clock) at both rims: a conic gradient. Both
    /// rims get an `aa` wide feather (alpha 0 at its far edge).
    pub fn ring(&mut self, c: P, r0: f32, r1: f32, segs: usize, shade: &dyn Fn(f32) -> Rgba, aa: f32) {
        let base = self.verts.len() as u32;
        let radii = [(r0 - aa).max(0.0), r0, r1, r1 + aa];
        for i in 0..=segs {
            let a = i as f32 / segs as f32 * std::f32::consts::TAU;
            let col = shade(a);
            for (j, &r) in radii.iter().enumerate() {
                let alpha = if j == 0 || j == 3 { 0.0 } else { col[3] };
                self.verts.push(V { pos: [c[0] + r * a.cos(), c[1] + r * a.sin()], color: [col[0], col[1], col[2], alpha] });
            }
        }
        for i in 0..segs as u32 {
            for j in 0..3 {
                let k = base + 4 * i + j;
                self.idx.extend([k, k + 1, k + 5, k, k + 5, k + 4]);
            }
        }
    }

    /// REMASTER Glass HUD (#295): `ring` along any closed convex outline `pts` (not repeating
    /// its first point): a band from `d0` to `d1` away from the outline (negative = towards
    /// `c`, the centre), coloured `shade0` at `d0` and `shade1` at `d1` (the point's colour),
    /// with an `aa` wide feather beyond both edges. The metal bezel of the dial's flat-bottomed face.
    pub fn band(&mut self, pts: &[P], c: P, (d0, d1): (f32, f32), shade0: &dyn Fn(P) -> Rgba, shade1: &dyn Fn(P) -> Rgba, aa: f32) {
        let n = pts.len();
        if n < 3 {
            return;
        }
        let base = self.verts.len() as u32;
        let nrm = |a: P, b: P| {
            let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
            let l = (dx * dx + dy * dy).sqrt().max(1e-6);
            [dy / l, -dx / l]
        };
        for i in 0..=n {
            let (p, prev, next) = (pts[i % n], pts[(i + n - 1) % n], pts[(i + 1) % n]);
            // the mitre: the mean of the two edge normals (scaled so the band keeps its width at a
            // corner), pointing away from the centre
            let (n0, n1) = (nrm(prev, p), nrm(p, next));
            let mut m = [n0[0] + n1[0], n0[1] + n1[1]];
            let ml = (m[0] * m[0] + m[1] * m[1]).sqrt().max(1e-6);
            m = [m[0] / ml, m[1] / ml];
            if m[0] * (p[0] - c[0]) + m[1] * (p[1] - c[1]) < 0.0 {
                m = [-m[0], -m[1]];
            }
            // (not for an inset: the mitre spike would shoot into the face)
            let k = if d1 <= 0.0 { 1.0 } else { 1.0 / (m[0] * n0[0] + m[1] * n0[1]).abs().max(0.5) };
            let (c0, c1) = (shade0(p), shade1(p));
            for (off, col) in [(d0 - aa, c0), (d0, c0), (d1, c1), (d1 + aa, c1)] {
                let alpha = if off < d0 || off > d1 { 0.0 } else { col[3] };
                self.verts.push(V { pos: [p[0] + m[0] * off * k, p[1] + m[1] * off * k], color: [col[0], col[1], col[2], alpha] });
            }
        }
        for i in 0..n as u32 {
            for j in 0..3 {
                let k = base + 4 * i + j;
                self.idx.extend([k, k + 1, k + 5, k, k + 5, k + 4]);
            }
        }
    }

    /// A radial gradient disc whose colour runs from `inner` at `focus` (inside the disc) to
    /// `outer` at the rim: the focus is the fan's centre, so an off-centre highlight works.
    pub fn disc_at(&mut self, c: P, r: f32, focus: P, inner: Rgba, outer: Rgba, segs: usize) {
        let base = self.verts.len() as u32;
        self.verts.push(V { pos: focus, color: inner });
        for i in 0..=segs {
            let a = i as f32 / segs as f32 * std::f32::consts::TAU;
            self.verts.push(V { pos: [c[0] + r * a.cos(), c[1] + r * a.sin()], color: outer });
        }
        for i in 0..segs as u32 {
            self.idx.extend([base, base + 1 + i, base + 2 + i]);
        }
    }

    /// `n` radial ticks round `c`, from `r` outward by `len`, `width` wide (tangential), the
    /// first at `from` (radians, y down); flat-ended antialiased segments (an SVG dashed
    /// circle's dashes).
    #[allow(clippy::too_many_arguments)]
    pub fn ticks(&mut self, c: P, r: f32, n: usize, len: f32, width: f32, color: Rgba, from: f32, px: f32) {
        for i in 0..n {
            let a = from + i as f32 / n as f32 * std::f32::consts::TAU;
            let d = [a.cos(), a.sin()];
            self.stroke(&[[c[0] + d[0] * r, c[1] + d[1] * r], [c[0] + d[0] * (r + len), c[1] + d[1] * (r + len)]], false, width, Ends::Butt, color, px);
        }
    }

    /// A dashed line from `a` to `b`: `on` long dashes with `off` gaps, `width` wide, flat ends.
    #[allow(clippy::too_many_arguments)]
    pub fn dashed(&mut self, a: P, b: P, on: f32, off: f32, width: f32, color: Rgba, px: f32) {
        let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
        let len = (dx * dx + dy * dy).sqrt();
        if len < 1e-3 || on <= 0.0 {
            return;
        }
        let (ux, uy) = (dx / len, dy / len);
        let mut s = 0.0;
        while s < len {
            let e = (s + on).min(len);
            self.stroke(&[[a[0] + ux * s, a[1] + uy * s], [a[0] + ux * e, a[1] + uy * e]], false, width, Ends::Butt, color, px);
            s += on + off;
        }
    }

    /// The outside half of `soft_pad`: strips from the convex outline (alpha `color[3]`) out to
    /// `spread` (alpha 0), nothing inside. A CSS box-shadow's or glow's spill round a shape.
    pub fn halo(&mut self, pts: &[P], spread: f32, color: Rgba) {
        self.halo_dir(pts, spread, [0.0, 0.0], color);
    }

    /// `halo` whose reach leans in a direction: `spread + shift . normal` at each outline point
    /// (an offset box-shadow without a band inside the shape or a gap under it).
    pub fn halo_dir(&mut self, pts: &[P], spread: f32, shift: P, color: Rgba) {
        let pts = &dedup(pts);
        let n = pts.len();
        if n < 3 || spread <= 0.0 {
            return;
        }
        let c = pts.iter().fold([0.0, 0.0], |a, p| [a[0] + p[0] / n as f32, a[1] + p[1] / n as f32]);
        let normals: Vec<P> = (0..n)
            .map(|i| {
                let (a, b) = (pts[(i + n - 1) % n], pts[(i + 1) % n]);
                let t = [b[0] - a[0], b[1] - a[1]];
                let l = (t[0] * t[0] + t[1] * t[1]).sqrt().max(1e-6);
                let m = [t[1] / l, -t[0] / l];
                if m[0] * (pts[i][0] - c[0]) + m[1] * (pts[i][1] - c[1]) < 0.0 { [-m[0], -m[1]] } else { m }
            })
            .collect();
        const RINGS: usize = 20;
        let reach: Vec<f32> = normals.iter().map(|m| (spread + shift[0] * m[0] + shift[1] * m[1]).max(0.2)).collect();
        let mut prev: Option<u32> = None;
        for k in 0..=RINGS {
            let u = k as f32 / RINGS as f32;
            let a = color[3] * (1.0 - u) * (1.0 - u);
            let base = self.verts.len() as u32;
            for i in 0..n {
                let d = reach[i] * u;
                self.verts.push(V { pos: [pts[i][0] + normals[i][0] * d, pts[i][1] + normals[i][1] * d], color: [color[0], color[1], color[2], a] });
            }
            if let Some(p) = prev {
                for i in 0..n as u32 {
                    let j = (i + 1) % n as u32;
                    self.idx.extend([p + i, base + i, base + j, p + i, base + j, p + j]);
                }
            }
            prev = Some(base);
        }
    }

    /// A drop shadow's spill round a convex outline (CSS `box-shadow dx dy blur`): the soft
    /// edge reaches `blur` plus the offset on the shadow's side, less on the other, starting at
    /// the shape itself; `color` is usually black with the shadow's alpha.
    pub fn shadow(&mut self, pts: &[P], dx: f32, dy: f32, blur: f32, color: Rgba) {
        self.halo_dir(pts, blur, [dx, dy], color);
    }

    /// An annulus with one colour at the inner rim and another at the outer: a radial gradient
    /// band (an inset shadow, a vignette).
    pub fn annulus(&mut self, c: P, r0: f32, r1: f32, inner: Rgba, outer: Rgba, segs: usize) {
        let base = self.verts.len() as u32;
        for i in 0..=segs {
            let a = i as f32 / segs as f32 * std::f32::consts::TAU;
            self.verts.push(V { pos: [c[0] + r0 * a.cos(), c[1] + r0 * a.sin()], color: inner });
            self.verts.push(V { pos: [c[0] + r1 * a.cos(), c[1] + r1 * a.sin()], color: outer });
        }
        for i in 0..segs as u32 {
            let k = base + 2 * i;
            self.idx.extend([k, k + 1, k + 3, k, k + 3, k + 2]);
        }
    }

    /// A disc as a polar lattice (`rings` x `segs`), each vertex coloured by `shade` (canvas
    /// point): any gradient, kept inside the disc.
    pub fn shade_disc(&mut self, c: P, r: f32, rings: usize, segs: usize, shade: &dyn Fn(P) -> Rgba) {
        let base = self.verts.len() as u32;
        self.verts.push(V { pos: c, color: shade(c) });
        for k in 1..=rings {
            let rr = r * k as f32 / rings as f32;
            for i in 0..segs {
                let a = i as f32 / segs as f32 * std::f32::consts::TAU;
                let p = [c[0] + rr * a.cos(), c[1] + rr * a.sin()];
                self.verts.push(V { pos: p, color: shade(p) });
            }
        }
        let ring = |k: usize, i: usize| if k == 0 { base } else { base + 1 + ((k - 1) * segs + i % segs) as u32 };
        for k in 1..=rings {
            for i in 0..segs {
                let (a, b) = (ring(k, i), ring(k, i + 1));
                if k == 1 {
                    self.idx.extend([base, a, b]);
                } else {
                    let (pa, pb) = (ring(k - 1, i), ring(k - 1, i + 1));
                    self.idx.extend([pa, a, b, pa, b, pb]);
                }
            }
        }
    }

    /// A rectangle as an `nx` x `ny` lattice of quads, each vertex coloured by `shade` (canvas
    /// point): any gradient, kept inside the rectangle.
    #[allow(clippy::too_many_arguments)]
    pub fn shade_rect(&mut self, x: f32, y: f32, w: f32, h: f32, nx: usize, ny: usize, shade: &dyn Fn(P) -> Rgba) {
        let base = self.verts.len() as u32;
        for j in 0..=ny {
            for i in 0..=nx {
                let p = [x + w * i as f32 / nx as f32, y + h * j as f32 / ny as f32];
                self.verts.push(V { pos: p, color: shade(p) });
            }
        }
        let row = (nx + 1) as u32;
        for j in 0..ny as u32 {
            for i in 0..nx as u32 {
                let k = base + j * row + i;
                self.idx.extend([k, k + 1, k + row + 1, k, k + row + 1, k + row]);
            }
        }
    }

    /// An axis-aligned rectangle with a colour at each corner (tl, tr, br, bl).
    pub fn quad(&mut self, x: f32, y: f32, w: f32, h: f32, c: [Rgba; 4]) {
        let base = self.verts.len() as u32;
        self.verts.extend([V { pos: [x, y], color: c[0] }, V { pos: [x + w, y], color: c[1] }, V { pos: [x + w, y + h], color: c[2] }, V { pos: [x, y + h], color: c[3] }]);
        self.idx.extend([base, base + 1, base + 2, base, base + 2, base + 3]);
    }
}

/// Points on an arc: centre, radius, from/to angle (radians, y down: increasing = clockwise
/// on screen), `n` segments (n + 1 points).
pub fn arc(c: P, r: f32, a0: f32, a1: f32, n: usize) -> Vec<P> {
    (0..=n)
        .map(|i| {
            let a = a0 + (a1 - a0) * i as f32 / n as f32;
            [c[0] + r * a.cos(), c[1] + r * a.sin()]
        })
        .collect()
}

/// A rounded rectangle's outline (clockwise on screen), `r` = corner radius.
pub fn rounded_rect(x: f32, y: f32, w: f32, h: f32, r: f32) -> Vec<P> {
    use std::f32::consts::{FRAC_PI_2, PI};
    let r = r.min(w * 0.5).min(h * 0.5);
    let mut v = Vec::new();
    for (cx, cy, a0) in [(x + w - r, y + r, -FRAC_PI_2), (x + w - r, y + h - r, 0.0), (x + r, y + h - r, FRAC_PI_2), (x + r, y + r, PI)] {
        v.extend(arc([cx, cy], r, a0, a0 + FRAC_PI_2, 6));
    }
    v
}

#[cfg(test)]
mod tests {
    use super::*;


    /// A glow round a pill (radius = half the height, so two points coincide at each end) is one layer thick everywhere:
    /// no triangle of its fan or strips lies over another.
    #[test]
    fn soft_pad_pill_has_no_overlap() {
        for (h, spread) in [(10.288f32, 12.0f32), (12.86, 12.0), (7.5, 12.0)] {
            let pill = rounded_rect(335.0, 36.0, 182.0, h, 5.33);
            let mut m = Mesh::default();
            m.soft_pad(&pill, spread, [1.0, 1.0, 1.0, 0.26]);
            let mut worst = 0;
            for xi in 0..60 {
                for yi in 0..40 {
                    let (px, py) = (336.0 + xi as f32 * 3.01, 36.0 - 10.0 + yi as f32 * (h + 20.0) / 40.0 + 0.0137);
                    let mut cover = 0;
                    for t in m.idx.chunks(3) {
                        let (p0, p1, p2) = (m.verts[t[0] as usize].pos, m.verts[t[1] as usize].pos, m.verts[t[2] as usize].pos);
                        let d = (p1[1] - p2[1]) * (p0[0] - p2[0]) + (p2[0] - p1[0]) * (p0[1] - p2[1]);
                        if d.abs() < 1e-9 {
                            continue;
                        }
                        let l0 = ((p1[1] - p2[1]) * (px - p2[0]) + (p2[0] - p1[0]) * (py - p2[1])) / d;
                        let l1 = ((p2[1] - p0[1]) * (px - p2[0]) + (p0[0] - p2[0]) * (py - p2[1])) / d;
                        if l0 >= 0.0 && l1 >= 0.0 && l0 + l1 <= 1.0 {
                            cover += 1;
                        }
                    }
                    worst = worst.max(cover);
                }
            }
            assert!(worst <= 1, "h {h}: a point lies under {worst} triangles");
        }
    }

    /// The feather ring of a filled shape lies outside it, whichever way it is wound.
    #[test]
    fn feather_is_outside() {
        let sq = [[10.0, 10.0], [20.0, 10.0], [20.0, 20.0], [10.0, 20.0]];
        let rev: Vec<P> = sq.iter().rev().copied().collect();
        for pts in [sq.to_vec(), rev] {
            let mut m = Mesh::default();
            m.fill(&pts, [1.0; 4], 2.0);
            // the transparent vertices are the ring's outer edge: all outside the square
            let out: Vec<_> = m.verts.iter().filter(|v| v.color[3] == 0.0).collect();
            assert!(!out.is_empty());
            assert!(out.iter().all(|v| v.pos[0] < 10.0 || v.pos[0] > 20.0 || v.pos[1] < 10.0 || v.pos[1] > 20.0));
            // and the opaque ones stay on or inside it
            assert!(m.verts.iter().filter(|v| v.color[3] > 0.0).all(|v| v.pos.iter().all(|&c| (10.0 - 1e-3..=20.0 + 1e-3).contains(&c))));
        }
    }

    /// A single line segment is one strip: alpha 0 | a | a | 0 across it, at both ends.
    #[test]
    fn segment_strip_alphas() {
        let mut m = Mesh::default();
        m.stroke(&[[0.0, 0.0], [50.0, 0.0]], false, 2.0, Ends::Butt, [1.0, 1.0, 1.0, 0.5], 0.5);
        let alphas: Vec<f32> = m.verts.iter().map(|v| v.color[3]).collect();
        assert_eq!(alphas, [0.0, 0.5, 0.5, 0.0, 0.0, 0.5, 0.5, 0.0]);
    }

    /// A polyline stacks `STROKE_AA_LAYERS` strokes; their alphas composite to the colour's alpha.
    #[test]
    fn stroke_layers_composite_to_alpha() {
        let mut m = Mesh::default();
        m.stroke(&[[0.0, 0.0], [25.0, 0.0], [50.0, 0.0]], false, 2.0, Ends::Butt, [1.0, 1.0, 1.0, 0.5], 0.5);
        // every vertex of a layer shares its alpha, and the layers are drawn one after another
        let mut layers: Vec<f32> = Vec::new();
        for v in &m.verts {
            if layers.last() != Some(&v.color[3]) {
                layers.push(v.color[3]);
            }
        }
        assert_eq!(layers.len(), STROKE_AA_LAYERS);
        let under: f32 = layers.iter().map(|a| 1.0 - a).product();
        assert!((under - 0.5).abs() < 1e-4, "{under}");
    }
}
