//! REMASTER irradiance probes (#83): a sparse volume of L1 SH probes along the road, baked on
//! the CPU from the pre-lit world. Each probe holds the sky's visibility and the light bounced
//! off the world (vertex colour × texture colour), both cosine-convolved, so the ambient of a lit
//! object becomes `lin(sky_ambient(n)) * vis(n) + bounce(n)`. The MYW point lights stay the
//! per-pixel direct term (09-remaster 9.24) [design choice].
//!
//! Layout: bricks of `CELLS`³ cells (`SIDE`³ probes, the border probes repeated in both bricks,
//! so hardware trilinear filtering needs no seams), `STEP` metres apart, on a lattice anchored
//! at the view-space origin. All positions are view space (`Scene::tris`).

use crate::scene::{Scene, Vertex};
use glam::{Quat, Vec3};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

/// Probe spacing, metres [APPROX: design choice, 2 m resolves a car's length].
pub const STEP: f32 = 2.0;
/// Cells per brick axis.
pub const CELLS: i32 = 4;
/// Probes per brick axis.
pub const SIDE: usize = CELLS as usize + 1;
/// Brick edge, metres.
pub const BRICK: f32 = STEP * CELLS as f32;
/// Rays per probe [APPROX: design choice; irradiance is low-frequency].
pub const RAYS: usize = 64;
/// Ray length, metres: anything further counts as sky [APPROX].
pub const MAX_T: f32 = 150.0;
/// A probe whose upward rays (`UP_CONE`) meet a back face first more often than this is under
/// or inside geometry; its value comes from its neighbours. Upward rays only, because the
/// world's walls are one-sided: a big wall facing away covers half of any probe's sphere with
/// back faces, but at most half of its upward cone, while the ground seen from below covers
/// all of it [APPROX: design choice; the usual test, a quarter of all rays, assumes closed meshes].
pub const INVALID_BACK: f32 = 0.75;
/// The upward cone of the inside test: ray direction y above this (a quarter of the sphere).
const UP_CONE: f32 = 0.5;
const GAMMA: f32 = 2.2;
/// Rays per probe towards the sun (#192), spread over a cone of `SUN_CONE` radians round it:
/// `Probe::sun` is the share that reach the sky, so the mouth of a tunnel fades over about one
/// probe step instead of flipping between probes [APPROX: design choice; the real sun is 0.27°].
pub const SUN_RAYS: usize = 16;
pub const SUN_CONE: f32 = 0.05;

/// One probe: L1 SH with the cosine lobe and the basis constants folded in, so
/// `c[0] + c[1] y + c[2] z + c[3] x` is the irradiance towards normal (x, y, z), divided by pi
/// (the convention of `sky::irradiance_sh`, cut to L1).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Probe {
    /// sky visibility: 1 in open space for any normal
    pub vis: [f32; 4],
    /// bounced light, linear, per channel, in the units of the linear ambient
    pub bounce: [[f32; 4]; 3],
    /// share of the upward rays (`UP_CONE`) that meet a back face first
    pub back: f32,
    /// share of the sun (0..1) the world lets through (#192); 1 in open space
    pub sun: f32,
}

impl Probe {
    /// Open space: all sky, no bounce.
    pub const OPEN: Probe = Probe { vis: [1.0, 0.0, 0.0, 0.0], bounce: [[0.0; 4]; 3], back: 0.0, sun: 1.0 };

    pub fn valid(&self) -> bool {
        self.back <= INVALID_BACK
    }

    /// (sky visibility, bounced rgb) towards unit normal `n`.
    pub fn eval(&self, n: Vec3) -> (f32, Vec3) {
        let e = |c: &[f32; 4]| (c[0] + c[1] * n.y + c[2] * n.z + c[3] * n.x).max(0.0);
        (e(&self.vis).min(1.0), Vec3::new(e(&self.bounce[0]), e(&self.bounce[1]), e(&self.bounce[2])))
    }

    fn lerp(&self, o: &Probe, t: f32) -> Probe {
        let l4 = |a: &[f32; 4], b: &[f32; 4]| std::array::from_fn(|k| a[k] + (b[k] - a[k]) * t);
        Probe { vis: l4(&self.vis, &o.vis), bounce: std::array::from_fn(|c| l4(&self.bounce[c], &o.bounce[c])), back: self.back + (o.back - self.back) * t, sun: self.sun + (o.sun - self.sun) * t }
    }
}

/// Ray hit on a world triangle.
#[derive(Clone, Copy, Debug)]
pub struct Hit {
    pub t: f32,
    pub tri: usize,
    /// barycentrics of the 2nd and 3rd corner
    pub u: f32,
    pub v: f32,
}

/// Möller–Trumbore, both faces: (t, u, v).
fn triangle(o: Vec3, d: Vec3, [a, b, c]: &[Vec3; 3]) -> Option<(f32, f32, f32)> {
    let (e1, e2) = (*b - *a, *c - *a);
    let p = d.cross(e2);
    let det = e1.dot(p);
    if det.abs() < 1e-12 {
        return None;
    }
    let inv = 1.0 / det;
    let s = o - *a;
    let u = s.dot(p) * inv;
    if !(0.0..=1.0).contains(&u) {
        return None;
    }
    let q = s.cross(e1);
    let v = d.dot(q) * inv;
    if v < 0.0 || u + v > 1.0 {
        return None;
    }
    let t = e2.dot(q) * inv;
    (t > 1e-4).then_some((t, u, v))
}

struct Node {
    lo: Vec3,
    hi: Vec3,
    /// leaf: first index into `Bvh::order`; inner: the left child (the right one follows it)
    first: u32,
    /// triangles in a leaf, 0 for an inner node
    count: u32,
}

/// Bounding volume hierarchy over a triangle list (median split on the longest axis).
pub struct Bvh {
    tris: Vec<[Vec3; 3]>,
    order: Vec<u32>,
    nodes: Vec<Node>,
}

const LEAF: usize = 4;

impl Bvh {
    pub fn new(tris: Vec<[Vec3; 3]>) -> Bvh {
        let mut order: Vec<u32> = (0..tris.len() as u32).collect();
        let centre: Vec<Vec3> = tris.iter().map(|t| (t[0] + t[1] + t[2]) / 3.0).collect();
        let mut nodes = Vec::with_capacity(tris.len() / 2 + 1);
        nodes.push(Node { lo: Vec3::ZERO, hi: Vec3::ZERO, first: 0, count: tris.len() as u32 });
        let mut todo = vec![(0usize, 0usize, tris.len())];
        while let Some((ni, a, b)) = todo.pop() {
            let (lo, hi) = order[a..b].iter().flat_map(|&i| tris[i as usize]).fold((Vec3::MAX, Vec3::MIN), |(lo, hi), p| (lo.min(p), hi.max(p)));
            (nodes[ni].lo, nodes[ni].hi) = (lo, hi);
            if b - a <= LEAF {
                (nodes[ni].first, nodes[ni].count) = (a as u32, (b - a) as u32);
                continue;
            }
            let (clo, chi) = order[a..b].iter().map(|&i| centre[i as usize]).fold((Vec3::MAX, Vec3::MIN), |(lo, hi), p| (lo.min(p), hi.max(p)));
            let ext = chi - clo;
            let axis = if ext.x >= ext.y && ext.x >= ext.z { 0 } else if ext.y >= ext.z { 1 } else { 2 };
            let m = (a + b) / 2;
            order[a..b].select_nth_unstable_by(m - a, |&i, &j| centre[i as usize][axis].total_cmp(&centre[j as usize][axis]));
            let left = nodes.len();
            nodes.push(Node { lo, hi, first: 0, count: 0 });
            nodes.push(Node { lo, hi, first: 0, count: 0 });
            (nodes[ni].first, nodes[ni].count) = (left as u32, 0);
            todo.push((left, a, m));
            todo.push((left + 1, m, b));
        }
        Bvh { tris, order, nodes }
    }

    /// Nearest hit along `o + t d` (`d` unit length), t < `max_t`.
    pub fn cast(&self, o: Vec3, d: Vec3, max_t: f32) -> Option<Hit> {
        self.cast_by(o, d, max_t, |_| true).0
    }

    /// Nearest hit on a triangle `keep` accepts, and whether a rejected one lies nearer.
    pub fn cast_by(&self, o: Vec3, d: Vec3, max_t: f32, keep: impl Fn(usize) -> bool) -> (Option<Hit>, bool) {
        let mut rejected = f32::MAX;
        if self.tris.is_empty() {
            return (None, false);
        }
        let inv = d.recip();
        let slab = |n: &Node, best: f32| -> Option<f32> {
            let (t0, t1) = ((n.lo - o) * inv, (n.hi - o) * inv);
            let (near, far) = (t0.min(t1).max_element().max(0.0), t0.max(t1).min_element().min(best));
            (near <= far).then_some(near)
        };
        let mut best: Option<Hit> = None;
        let mut stack = vec![0u32];
        while let Some(ni) = stack.pop() {
            let n = &self.nodes[ni as usize];
            if slab(n, best.map_or(max_t, |h| h.t)).is_none() {
                continue;
            }
            if n.count > 0 {
                for &i in &self.order[n.first as usize..(n.first + n.count) as usize] {
                    if let Some((t, u, v)) = triangle(o, d, &self.tris[i as usize]) {
                        if t >= best.map_or(max_t, |h| h.t) {
                            continue;
                        }
                        if keep(i as usize) {
                            best = Some(Hit { t, tri: i as usize, u, v });
                        } else {
                            rejected = rejected.min(t);
                        }
                    }
                }
                continue;
            }
            // the nearer child last, so it is popped first
            let (l, r) = (n.first, n.first + 1);
            let lim = best.map_or(max_t, |h| h.t);
            match (slab(&self.nodes[l as usize], lim), slab(&self.nodes[r as usize], lim)) {
                (Some(a), Some(b)) if a <= b => stack.extend([r, l]),
                (Some(_), Some(_)) => stack.extend([l, r]),
                (Some(_), None) => stack.push(l),
                (None, Some(_)) => stack.push(r),
                (None, None) => {}
            }
        }
        (best, rejected < best.map_or(max_t, |h| h.t))
    }
}

/// Mean colour of each texture layer, linear (texels with alpha under one half are left out:
/// colour keys).
fn layer_means(layers: &[Vec<u8>]) -> Vec<Vec3> {
    let lin: [f32; 256] = std::array::from_fn(|c| (c as f32 / 255.0).powf(GAMMA));
    layers
        .iter()
        .map(|px| {
            let (sum, n) = px.as_chunks::<4>().0.iter().filter(|t| t[3] >= 128).fold((Vec3::ZERO, 0u32), |(s, n), t| {
                (s + Vec3::new(lin[t[0] as usize], lin[t[1] as usize], lin[t[2] as usize]), n + 1)
            });
            if n == 0 { Vec3::ONE } else { sum / n as f32 }
        })
        .collect()
}

/// The world as the bake sees it: geometry, and per triangle the corner colours (linear,
/// texture mean folded in) and the front normal.
pub struct BakeWorld {
    bvh: Bvh,
    colour: Vec<[Vec3; 3]>,
    normal: Vec<Vec3>,
}

impl BakeWorld {
    pub fn new(tris: &[Vertex], layers: &[Vec<u8>]) -> BakeWorld {
        let means = layer_means(layers);
        let t = tris.as_chunks::<3>().0;
        let lin = |c: [f32; 4]| Vec3::new(c[0], c[1], c[2]).map(|x| x.max(0.0).powf(GAMMA));
        let colour = t
            .iter()
            .map(|v| {
                let m = if v[0].layer >= 0.0 { means.get(v[0].layer as usize).copied().unwrap_or(Vec3::ONE) } else { Vec3::ONE };
                v.map(|v| lin(v.color) * m)
            })
            .collect();
        let normal = t.iter().map(|v| Vec3::from(v[0].normal)).collect();
        BakeWorld { bvh: Bvh::new(t.iter().map(|v| v.map(|v| Vec3::from(v.pos))).collect()), colour, normal }
    }

    pub fn of(s: &Scene) -> BakeWorld {
        BakeWorld::new(&s.tris, &s.layers)
    }

    /// The probe at `p`. `dirs` are unit directions spread evenly over the sphere; they are
    /// turned by a rotation hashed from `p`, so neighbouring probes sample differently. `sun`
    /// are the rays towards the sun (`sun_rays`).
    pub fn probe(&self, p: Vec3, dirs: &[Vec3], sun: &[Vec3]) -> Probe {
        let rot = hash_rotation(p);
        let w = 4.0 * std::f32::consts::PI / dirs.len() as f32;
        // (1/pi) of the folded L1 projection: band 0 = Y00² = 1/(4 pi), band 1 = (2/3) Y1² = 1/(2 pi)
        let (k0, k1) = (w / (4.0 * std::f32::consts::PI), w / (2.0 * std::f32::consts::PI));
        let mut pr = Probe { vis: [0.0; 4], bounce: [[0.0; 4]; 3], back: 0.0, sun: 1.0 };
        let add = |c: &mut [f32; 4], d: Vec3, f: f32| {
            c[0] += k0 * f;
            c[1] += k1 * f * d.y;
            c[2] += k1 * f * d.z;
            c[3] += k1 * f * d.x;
        };
        let (mut up, mut back) = (0, 0);
        for &d0 in dirs {
            let d = rot * d0;
            // back faces are culled, as the world is drawn [O 05-formats "Back-face culling"]:
            // the ray goes on through them (a first hit on one still counts towards `back`)
            let (hit, back_first) = self.bvh.cast_by(p, d, MAX_T, |t| self.normal[t].dot(d) <= 0.0);
            if d.y > UP_CONE {
                up += 1;
                back += back_first as u32;
            }
            let Some(h) = hit else {
                add(&mut pr.vis, d, 1.0);
                continue;
            };
            let c = self.colour[h.tri];
            let l = c[0] * (1.0 - h.u - h.v) + c[1] * h.u + c[2] * h.v;
            for (ch, b) in pr.bounce.iter_mut().enumerate() {
                add(b, d, l[ch]);
            }
        }
        pr.back = back as f32 / up.max(1) as f32;
        if !sun.is_empty() {
            // the same culled world as the sky rays: a tunnel roof faces the probe below it
            let lit = sun.iter().filter(|&&d| self.bvh.cast_by(p, d, MAX_T, |t| self.normal[t].dot(d) <= 0.0).0.is_none()).count();
            pr.sun = lit as f32 / sun.len() as f32;
        }
        pr
    }
}

/// `SUN_RAYS` directions in the cone of `SUN_CONE` round the unit direction to the sun `sun`
/// (a Fibonacci disk); none when the sun is below the horizon (the night look has no sun to
/// shadow, and `Probe::sun` stays 1).
pub fn sun_rays(sun: Vec3) -> Vec<Vec3> {
    let s = sun.normalize_or_zero();
    if s.y <= 0.0 {
        return Vec::new();
    }
    let (a, b) = s.any_orthonormal_pair();
    let golden = std::f32::consts::PI * (3.0 - 5f32.sqrt());
    (0..SUN_RAYS)
        .map(|i| {
            let r = SUN_CONE * ((i as f32 + 0.5) / SUN_RAYS as f32).sqrt();
            let t = golden * i as f32;
            (s + (a * t.cos() + b * t.sin()) * r).normalize()
        })
        .collect()
}

/// `n` directions on a Fibonacci sphere.
pub fn sphere_dirs(n: usize) -> Vec<Vec3> {
    let golden = std::f32::consts::PI * (3.0 - 5f32.sqrt());
    (0..n)
        .map(|i| {
            let y = 1.0 - 2.0 * (i as f32 + 0.5) / n as f32;
            let r = (1.0 - y * y).max(0.0).sqrt();
            let a = golden * i as f32;
            Vec3::new(r * a.cos(), y, r * a.sin())
        })
        .collect()
}

/// A uniformly distributed rotation from a hash of `p` (deterministic).
fn hash_rotation(p: Vec3) -> Quat {
    let mut h = (p.x.to_bits() as u64).wrapping_mul(0x9e37_79b9_7f4a_7c15) ^ (p.y.to_bits() as u64).wrapping_mul(0xc2b2_ae3d_27d4_eb4f) ^ (p.z.to_bits() as u64).wrapping_mul(0x1656_67b1_9e37_79f9);
    let mut next = || {
        h ^= h >> 33;
        h = h.wrapping_mul(0xff51_afd7_ed55_8ccd);
        h ^= h >> 33;
        (h >> 40) as f32 / (1u64 << 24) as f32
    };
    // Shoemake's uniform random quaternion
    let (u1, u2, u3) = (next(), next(), next());
    let tau = std::f32::consts::TAU;
    let (a, b) = ((1.0 - u1).sqrt(), u1.sqrt());
    Quat::from_xyzw(a * (tau * u2).sin(), a * (tau * u2).cos(), b * (tau * u3).sin(), b * (tau * u3).cos()).normalize()
}

/// Bricks around the road: every brick a COL `asphalt` face reaches (game coordinates, z
/// negated here), then one brick more on each side and one above (cars and roadside props
/// stand there) [APPROX: design choice].
pub fn road_bricks(world: &motor_sim::world::World) -> Vec<[i32; 3]> {
    let asphalt = crate::scene::asphalt_materials(world);
    let mut core: HashSet<[i32; 3]> = HashSet::new();
    for f in &world.col.faces {
        if !asphalt.get(world.material_of(f.material)).copied().unwrap_or(false) {
            continue;
        }
        let pts = crate::debug::face_outline(&world.col, f);
        if pts.is_empty() {
            continue;
        }
        let (lo, hi) = pts.iter().map(|p| *p * Vec3::new(1.0, 1.0, -1.0)).fold((Vec3::MAX, Vec3::MIN), |(lo, hi), p| (lo.min(p), hi.max(p)));
        let (a, b) = ((lo / BRICK).floor().as_ivec3(), (hi / BRICK).floor().as_ivec3());
        for x in a.x..=b.x {
            for y in a.y..=b.y {
                for z in a.z..=b.z {
                    core.insert([x, y, z]);
                }
            }
        }
    }
    let mut out: HashSet<[i32; 3]> = HashSet::new();
    for [x, y, z] in core {
        for dx in -1..=1 {
            for dz in -1..=1 {
                for dy in 0..=1 {
                    out.insert([x + dx, y + dy, z + dz]);
                }
            }
        }
    }
    let mut out: Vec<[i32; 3]> = out.into_iter().collect();
    out.sort_unstable();
    out
}

/// The baked volume.
pub struct ProbeVolume {
    /// brick coordinates (corner at `b * BRICK`), sorted
    pub bricks: Vec<[i32; 3]>,
    index: HashMap<[i32; 3], u32>,
    /// `SIDE`³ per brick, x fastest, then y, then z
    pub probes: Vec<Probe>,
    pub stats: BakeStats,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct BakeStats {
    pub triangles: usize,
    /// distinct lattice points baked (bricks share their borders)
    pub points: usize,
    /// points inside geometry, filled from their neighbours
    pub invalid: usize,
    /// invalid points with no valid point near: open space
    pub unfilled: usize,
    /// points that see less than half of the sun
    pub shaded: usize,
    pub bvh_ms: f32,
    pub bake_ms: f32,
}

impl std::fmt::Display for BakeStats {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        let pct = |n: usize| 100.0 * n as f32 / self.points.max(1) as f32;
        write!(
            f,
            "{} tris (BVH {:.0} ms), {} probes, {} invalid ({:.1} %, {} unfilled), {:.1} % in the shade, bake {:.0} ms",
            self.triangles,
            self.bvh_ms,
            self.points,
            self.invalid,
            pct(self.invalid),
            self.unfilled,
            pct(self.shaded),
            self.bake_ms
        )
    }
}

/// Index of a lattice point inside a brick.
fn local(i: usize, j: usize, k: usize) -> usize {
    i + SIDE * (j + SIDE * k)
}

impl ProbeVolume {
    /// Bakes `bricks` against `w` on every core, the sun towards unit `sun` (view space).
    pub fn bake(w: &BakeWorld, bricks: Vec<[i32; 3]>, sun: Vec3) -> ProbeVolume {
        let threads = std::thread::available_parallelism().map_or(4, |n| n.get());
        ProbeVolume::bake_on(w, bricks, sun, threads, &AtomicBool::new(false)).expect("bake not cancelled")
    }

    /// Bakes on `threads` threads; None when `cancel` is set before it is done.
    pub fn bake_on(w: &BakeWorld, bricks: Vec<[i32; 3]>, sun: Vec3, threads: usize, cancel: &AtomicBool) -> Option<ProbeVolume> {
        let t0 = std::time::Instant::now();
        let mut points: Vec<[i32; 3]> = bricks.iter().flat_map(|b| (0..SIDE * SIDE * SIDE).map(move |n| [b[0] * CELLS + (n % SIDE) as i32, b[1] * CELLS + (n / SIDE % SIDE) as i32, b[2] * CELLS + (n / (SIDE * SIDE)) as i32])).collect();
        points.sort_unstable();
        points.dedup();
        let dirs = sphere_dirs(RAYS);
        let sun = sun_rays(sun);
        let next = AtomicUsize::new(0);
        const CHUNK: usize = 256;
        let mut baked: Vec<Probe> = vec![Probe::OPEN; points.len()];
        let parts: Vec<Vec<(usize, Vec<Probe>)>> = std::thread::scope(|sc| {
            let hs: Vec<_> = (0..threads)
                .map(|_| {
                    sc.spawn(|| {
                        let mut mine = Vec::new();
                        loop {
                            let a = next.fetch_add(CHUNK, Ordering::Relaxed);
                            if a >= points.len() || cancel.load(Ordering::Relaxed) {
                                break mine;
                            }
                            let b = (a + CHUNK).min(points.len());
                            mine.push((a, points[a..b].iter().map(|q| w.probe(Vec3::new(q[0] as f32, q[1] as f32, q[2] as f32) * STEP, &dirs, &sun)).collect()));
                        }
                    })
                })
                .collect();
            hs.into_iter().map(|h| h.join().expect("probe bake thread")).collect()
        });
        if cancel.load(Ordering::Relaxed) {
            return None;
        }
        for (a, ps) in parts.into_iter().flatten() {
            baked[a..a + ps.len()].copy_from_slice(&ps);
        }
        let mut stats = BakeStats { points: points.len(), invalid: baked.iter().filter(|p| !p.valid()).count(), ..Default::default() };
        stats.unfilled = fill_invalid(&points, &mut baked);
        stats.shaded = baked.iter().filter(|p| p.sun < 0.5).count();
        let at: HashMap<[i32; 3], usize> = points.iter().enumerate().map(|(i, &q)| (q, i)).collect();
        let mut probes = Vec::with_capacity(bricks.len() * SIDE * SIDE * SIDE);
        for b in &bricks {
            for k in 0..SIDE {
                for j in 0..SIDE {
                    for i in 0..SIDE {
                        probes.push(baked[at[&[b[0] * CELLS + i as i32, b[1] * CELLS + j as i32, b[2] * CELLS + k as i32]]]);
                    }
                }
            }
        }
        stats.bake_ms = t0.elapsed().as_secs_f32() * 1000.0;
        let index = bricks.iter().enumerate().map(|(i, &b)| (b, i as u32)).collect();
        Some(ProbeVolume { bricks, index, probes, stats })
    }

    /// Bakes the bricks along the road of a loaded track, the sun towards `sun`.
    pub fn bake_scene(s: &Scene, world: &motor_sim::world::World, sun: Vec3) -> ProbeVolume {
        BakeSource::new(s, world).bake(sun)
    }

    /// The probes of brick `b` (None: no brick there).
    pub fn brick(&self, b: [i32; 3]) -> Option<&[Probe]> {
        let i = *self.index.get(&b)? as usize * SIDE * SIDE * SIDE;
        Some(&self.probes[i..i + SIDE * SIDE * SIDE])
    }

    /// The probe interpolated at `p` (trilinear inside its brick), None outside the bricks.
    pub fn at(&self, p: Vec3) -> Option<Probe> {
        let b = (p / BRICK).floor();
        let ps = self.brick([b.x as i32, b.y as i32, b.z as i32])?;
        let f = ((p - b * BRICK) / STEP).clamp(Vec3::ZERO, Vec3::splat(CELLS as f32 - 1e-4));
        let (i, t) = (f.floor(), f.fract());
        let (i, j, k) = (i.x as usize, i.y as usize, i.z as usize);
        let x = |j: usize, k: usize| ps[local(i, j, k)].lerp(&ps[local(i + 1, j, k)], t.x);
        let y = |k: usize| x(j, k).lerp(&x(j + 1, k), t.y);
        Some(y(k).lerp(&y(k + 1), t.z))
    }

    /// (sky visibility, bounced rgb) at `p` towards unit normal `n`; open space outside the bricks.
    pub fn irradiance(&self, p: Vec3, n: Vec3) -> (f32, Vec3) {
        self.at(p).unwrap_or(Probe::OPEN).eval(n)
    }

    /// Every probe of the bricks in `keep` with its position (border probes once per brick;
    /// `Probe::valid` still tells the ones that were inside geometry).
    pub fn points<'a>(&'a self, keep: impl Fn(&[i32; 3]) -> bool + 'a) -> impl Iterator<Item = (Vec3, Probe)> + 'a {
        let n = SIDE * SIDE * SIDE;
        self.bricks.iter().enumerate().filter(move |(_, b)| keep(b)).flat_map(move |(bi, b)| {
            (0..n).map(move |l| {
                let q = [b[0] * CELLS + (l % SIDE) as i32, b[1] * CELLS + (l / SIDE % SIDE) as i32, b[2] * CELLS + (l / (SIDE * SIDE)) as i32];
                (Vec3::new(q[0] as f32, q[1] as f32, q[2] as f32) * STEP, self.probes[bi * n + l])
            })
        })
    }

    /// The probes within `r` of `eye`.
    pub fn near(&self, eye: Vec3, r: f32) -> Vec<(Vec3, Probe)> {
        let centre = |b: &[i32; 3]| (Vec3::new(b[0] as f32, b[1] as f32, b[2] as f32) + 0.5) * BRICK;
        self.points(|b| centre(b).distance(eye) <= r + BRICK).filter(|(p, _)| p.distance(eye) <= r).collect()
    }
}

/// What a loaded track keeps to bake its volume when it is first wanted: the world as the bake
/// sees it and the bricks along its road (tens of milliseconds at load; `bake` is the slow part).
/// The sun direction is given to each bake: it depends on the view (the HDRI's sun, 9.32).
pub struct BakeSource {
    pub world: BakeWorld,
    pub bricks: Vec<[i32; 3]>,
    triangles: usize,
    bvh_ms: f32,
    /// a hash of the bake's input and constants (the disk cache's key with the sun, `key`)
    base: u64,
}

impl BakeSource {
    pub fn new(s: &Scene, world: &motor_sim::world::World) -> BakeSource {
        let t0 = std::time::Instant::now();
        let w = BakeWorld::of(s);
        let bvh_ms = t0.elapsed().as_secs_f32() * 1000.0;
        let bricks = road_bricks(world);
        let base = cache_key(&w, &bricks);
        BakeSource { world: w, bricks, triangles: s.tris.len() / 3, bvh_ms, base }
    }

    /// The disk cache's key of the volume with the sun towards `sun`.
    pub fn key(&self, sun: Vec3) -> u64 {
        sun_key(self.base, sun)
    }

    pub fn bake(&self, sun: Vec3) -> ProbeVolume {
        let threads = std::thread::available_parallelism().map_or(4, |n| n.get());
        self.bake_on(sun, threads, &AtomicBool::new(false)).expect("bake not cancelled")
    }

    pub fn bake_on(&self, sun: Vec3, threads: usize, cancel: &AtomicBool) -> Option<ProbeVolume> {
        let mut v = ProbeVolume::bake_on(&self.world, self.bricks.clone(), sun, threads, cancel)?;
        (v.stats.triangles, v.stats.bvh_ms) = (self.triangles, self.bvh_ms);
        Some(v)
    }

    /// The packed volume from the disk cache when it holds this source's with this sun.
    pub fn cached(&self, sun: Vec3) -> Option<Packed> {
        let key = self.key(sun);
        read_cache(&cache_dir()?.join(format!("{key:016x}.probes")), key)
    }

    /// Bakes and packs the volume and writes it to the disk cache (None: cancelled, or no bricks).
    pub fn bake_packed(&self, sun: Vec3, threads: usize, cancel: &AtomicBool) -> Option<Packed> {
        let v = self.bake_on(sun, threads, cancel)?;
        println!("local light: {} bricks, {}", v.bricks.len(), v.stats);
        let p = v.pack()?;
        if let Some(dir) = cache_dir() {
            if let Err(e) = write_cache(&dir, self.key(sun), &p) {
                eprintln!("local light cache {}: {e}", dir.display());
            }
        }
        Some(p)
    }
}

/// A probe volume baked on a worker thread (on all cores but one, so the frame keeps its own);
/// dropping it cancels the bake.
pub struct Baking {
    rx: std::sync::mpsc::Receiver<Option<Packed>>,
    cancel: std::sync::Arc<AtomicBool>,
}

impl Baking {
    pub fn start(src: std::sync::Arc<BakeSource>, sun: Vec3) -> Baking {
        let (tx, rx) = std::sync::mpsc::channel();
        let cancel = std::sync::Arc::new(AtomicBool::new(false));
        let stop = cancel.clone();
        let threads = std::thread::available_parallelism().map_or(4, |n| n.get()).saturating_sub(1).max(1);
        std::thread::spawn(move || tx.send(src.bake_packed(sun, threads, &stop)));
        Baking { rx, cancel }
    }

    /// The result once the bake is over (inner None: nothing to show), else None.
    pub fn poll(&self) -> Option<Option<Packed>> {
        match self.rx.try_recv() {
            Ok(p) => Some(p),
            Err(std::sync::mpsc::TryRecvError::Empty) => None,
            Err(std::sync::mpsc::TryRecvError::Disconnected) => Some(None),
        }
    }
}

impl Drop for Baking {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}

/// Where baked volumes are kept: %LOCALAPPDATA%\Motorhead\cache\probes on Windows,
/// $XDG_CACHE_HOME/motorhead/probes or ~/.cache/motorhead/probes elsewhere.
pub fn cache_dir() -> Option<PathBuf> {
    let base = if cfg!(windows) {
        PathBuf::from(std::env::var_os("LOCALAPPDATA")?).join("Motorhead").join("cache")
    } else if let Some(x) = std::env::var_os("XDG_CACHE_HOME").filter(|x| !x.is_empty()) {
        PathBuf::from(x).join("motorhead")
    } else {
        PathBuf::from(std::env::var_os("HOME")?).join(".cache").join("motorhead")
    };
    Some(base.join("probes"))
}

/// Raise when the bake or the file layout changes in a way the hashed input does not show.
const CACHE_VERSION: u64 = 2;
const CACHE_MAGIC: &[u8; 8] = b"MHPROBES";
/// Volumes kept in the cache (the newest; one per track and look, so a few spare) [design choice].
const CACHE_FILES: usize = 24;

/// FNV-1a over 32-bit words of the bake input, the bricks and the bake constants.
fn cache_key(w: &BakeWorld, bricks: &[[i32; 3]]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    let mut put = |x: u32| {
        h ^= x as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    };
    for x in [CACHE_VERSION as u32, STEP.to_bits(), CELLS as u32, RAYS as u32, MAX_T.to_bits(), INVALID_BACK.to_bits(), UP_CONE.to_bits(), GAMMA.to_bits(), SUN_RAYS as u32, SUN_CONE.to_bits()] {
        put(x);
    }
    put(w.colour.len() as u32);
    for (i, c) in w.colour.iter().enumerate() {
        for v in w.bvh.tris[i].iter().chain(c).chain([&w.normal[i]]) {
            for f in v.to_array() {
                put(f.to_bits());
            }
        }
    }
    for x in bricks.as_flattened() {
        put(*x as u32);
    }
    h
}

/// `base` (`cache_key`) with the sun direction folded in.
fn sun_key(base: u64, sun: Vec3) -> u64 {
    let mut h = base;
    for f in sun.to_array() {
        h ^= f.to_bits() as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

fn read_cache(path: &std::path::Path, key: u64) -> Option<Packed> {
    let raw = miniz_oxide::inflate::decompress_to_vec(&std::fs::read(path).ok()?).ok()?;
    let (head, rest) = raw.split_at_checked(8 + 8 + 9 * 4)?;
    let u = |i: usize| u32::from_le_bytes(head[16 + 4 * i..20 + 4 * i].try_into().unwrap());
    if &head[..8] != CACHE_MAGIC || u64::from_le_bytes(head[8..16].try_into().unwrap()) != key {
        return None;
    }
    let (index_size, origin, atlas_size) = ([u(0), u(1), u(2)], [u(3) as i32, u(4) as i32, u(5) as i32], [u(6), u(7), u(8)]);
    let ni = index_size.iter().product::<u32>() as usize * 4;
    let na = atlas_size.iter().product::<u32>() as usize * 4 * 2;
    let ns = na / 32;
    if rest.len() != ni + na + ns {
        return None;
    }
    let index = rest[..ni].as_chunks::<4>().0.iter().map(|b| u32::from_le_bytes(*b)).collect();
    let atlas = rest[ni..ni + na].as_chunks::<2>().0.iter().map(|b| u16::from_le_bytes(*b)).collect();
    let sun = rest[ni + na..].to_vec();
    Some(Packed { index, index_size, origin, atlas, atlas_size, sun })
}

/// Writes `p` as `<key>.probes` (through a temporary file, so a reader never sees half of it)
/// and keeps only the newest `CACHE_FILES`.
fn write_cache(dir: &std::path::Path, key: u64, p: &Packed) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    let mut raw = Vec::with_capacity(52 + p.index.len() * 4 + p.atlas.len() * 2 + p.sun.len());
    raw.extend_from_slice(CACHE_MAGIC);
    raw.extend_from_slice(&key.to_le_bytes());
    for x in p.index_size.iter().chain(&p.origin.map(|x| x as u32)).chain(&p.atlas_size) {
        raw.extend_from_slice(&x.to_le_bytes());
    }
    raw.extend(p.index.iter().flat_map(|x| x.to_le_bytes()));
    raw.extend(p.atlas.iter().flat_map(|x| x.to_le_bytes()));
    raw.extend_from_slice(&p.sun);
    let name = format!("{key:016x}.probes");
    let tmp = dir.join(format!("{name}.tmp"));
    std::fs::write(&tmp, miniz_oxide::deflate::compress_to_vec(&raw, 1))?;
    std::fs::rename(&tmp, dir.join(name))?;
    let mut files: Vec<(std::time::SystemTime, PathBuf)> = std::fs::read_dir(dir)?
        .flatten()
        .filter(|e| e.path().extension().is_some_and(|x| x == "probes"))
        .filter_map(|e| Some((e.metadata().ok()?.modified().ok()?, e.path())))
        .collect();
    files.sort_by_key(|f| std::cmp::Reverse(f.0));
    for (_, f) in files.iter().skip(CACHE_FILES) {
        let _ = std::fs::remove_file(f);
    }
    Ok(())
}

/// Atlas slots per x and y of a slab (shader.wgsl `IRR_SLOTS`).
pub const ATLAS_SLOTS: usize = 32;

/// The volume as GPU textures (render.rs `LocalLightGpu`, shader.wgsl `local_ambient`).
pub struct Packed {
    /// indirection, one texel per brick over the bricks' box: slot + 1, 0 = no brick
    pub index: Vec<u32>,
    pub index_size: [u32; 3],
    /// brick coordinates of the box's low corner
    pub origin: [i32; 3],
    /// atlas, RGBA16F (f16 bits): slot `s` at (s % ATLAS_SLOTS, s / ATLAS_SLOTS % ATLAS_SLOTS,
    /// s / ATLAS_SLOTS²) × `SIDE` texels; the SH channels vis, bounce r, g, b stacked in z, a
    /// quarter of the depth each
    pub atlas: Vec<u16>,
    pub atlas_size: [u32; 3],
    /// the sun's share (`Probe::sun`, R8 unorm), laid out like one channel of the atlas: its
    /// size is `atlas_size` with a quarter of the depth (#192)
    pub sun: Vec<u8>,
}

impl ProbeVolume {
    /// The textures of the volume (None: no bricks).
    pub fn pack(&self) -> Option<Packed> {
        if self.bricks.is_empty() {
            return None;
        }
        let lo = self.bricks.iter().fold([i32::MAX; 3], |m, b| std::array::from_fn(|a| m[a].min(b[a])));
        let hi = self.bricks.iter().fold([i32::MIN; 3], |m, b| std::array::from_fn(|a| m[a].max(b[a])));
        let size: [u32; 3] = std::array::from_fn(|a| (hi[a] - lo[a] + 1) as u32);
        let mut index = vec![0u32; (size[0] * size[1] * size[2]) as usize];
        for (slot, b) in self.bricks.iter().enumerate() {
            let [x, y, z] = std::array::from_fn(|a| (b[a] - lo[a]) as u32);
            index[(x + size[0] * (y + size[1] * z)) as usize] = slot as u32 + 1;
        }
        let n = self.bricks.len();
        let per_slab = ATLAS_SLOTS * ATLAS_SLOTS;
        let rows = n.min(per_slab).div_ceil(ATLAS_SLOTS);
        let slabs = n.div_ceil(per_slab);
        let (w, h, d) = (ATLAS_SLOTS * SIDE, rows * SIDE, slabs * SIDE);
        let mut atlas = vec![0u16; w * h * d * 4 * 4];
        let mut sun = vec![255u8; w * h * d];
        let f = |x: f32| half::f16::from_f32(x).to_bits();
        let cube = SIDE * SIDE * SIDE;
        for slot in 0..n {
            let (sx, sy, sz) = (slot % ATLAS_SLOTS * SIDE, slot / ATLAS_SLOTS % ATLAS_SLOTS * SIDE, slot / per_slab * SIDE);
            for l in 0..cube {
                let p = &self.probes[slot * cube + l];
                let (i, j, k) = (l % SIDE, l / SIDE % SIDE, l / (SIDE * SIDE));
                sun[(sx + i) + w * ((sy + j) + h * (sz + k))] = (p.sun.clamp(0.0, 1.0) * 255.0).round() as u8;
                for (c, sh) in [&p.vis, &p.bounce[0], &p.bounce[1], &p.bounce[2]].into_iter().enumerate() {
                    let t = (sx + i) + w * ((sy + j) + h * (c * d + sz + k));
                    for (e, v) in sh.iter().enumerate() {
                        atlas[t * 4 + e] = f(*v);
                    }
                }
            }
        }
        Some(Packed { index, index_size: size, origin: lo, atlas, atlas_size: [w as u32, h as u32, (d * 4) as u32], sun })
    }
}

/// Invalid probes take the mean of their valid lattice neighbours (6-connected), wave by wave;
/// `back` is kept, so they still read as invalid. Returns how many were left (set to open space).
fn fill_invalid(points: &[[i32; 3]], probes: &mut [Probe]) -> usize {
    let at: HashMap<[i32; 3], usize> = points.iter().enumerate().map(|(i, &q)| (q, i)).collect();
    let mut ok: Vec<bool> = probes.iter().map(Probe::valid).collect();
    let mut todo: Vec<usize> = (0..points.len()).filter(|&i| !ok[i]).collect();
    const STEPS: [[i32; 3]; 6] = [[1, 0, 0], [-1, 0, 0], [0, 1, 0], [0, -1, 0], [0, 0, 1], [0, 0, -1]];
    while !todo.is_empty() {
        let mut wave = Vec::new();
        for &i in &todo {
            let q = points[i];
            let near: Vec<usize> = STEPS.iter().filter_map(|s| at.get(&[q[0] + s[0], q[1] + s[1], q[2] + s[2]]).copied()).filter(|&n| ok[n]).collect();
            if near.is_empty() {
                continue;
            }
            let mut m = Probe { vis: [0.0; 4], bounce: [[0.0; 4]; 3], back: probes[i].back, sun: 0.0 };
            let k = 1.0 / near.len() as f32;
            for &n in &near {
                let p = &probes[n];
                m.sun += p.sun * k;
                for c in 0..4 {
                    m.vis[c] += p.vis[c] * k;
                    for ch in 0..3 {
                        m.bounce[ch][c] += p.bounce[ch][c] * k;
                    }
                }
            }
            wave.push((i, m));
        }
        if wave.is_empty() {
            break;
        }
        for (i, m) in wave {
            probes[i] = m;
            ok[i] = true;
        }
        todo.retain(|&i| !ok[i]);
    }
    for &i in &todo {
        probes[i] = Probe { back: probes[i].back, ..Probe::OPEN };
    }
    todo.len()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn quad(out: &mut Vec<Vertex>, c: Vec3, u: Vec3, v: Vec3, colour: f32) {
        let n = u.cross(v).normalize();
        let vert = |p: Vec3| Vertex { pos: p.into(), uv: [0.0; 2], color: [colour, colour, colour, 1.0], layer: -1.0, normal: n.into(), env: 0.0 };
        let [a, b, cc, d] = [c - u - v, c + u - v, c + u + v, c - u + v];
        out.extend([a, b, cc, a, cc, d].map(vert));
    }

    /// A cube of half-size `h` round the origin, faces turned inwards.
    fn cube(h: f32) -> Vec<Vertex> {
        let mut t = Vec::new();
        for axis in [Vec3::X, Vec3::Y, Vec3::Z] {
            for s in [-1.0, 1.0] {
                let c = axis * h * s;
                let (u, v) = if axis == Vec3::Y { (Vec3::X, Vec3::Z) } else { (Vec3::Y, axis.cross(Vec3::Y)) };
                // inward normal = -c
                let (u, v) = if u.cross(v).dot(-c) > 0.0 { (u, v) } else { (v, u) };
                quad(&mut t, c, u * h, v * h, 1.0);
            }
        }
        t
    }

    fn close(a: f32, b: f32, tol: f32) -> bool {
        (a - b).abs() <= tol
    }

    #[test]
    fn open_space_sees_all_sky() {
        let w = BakeWorld::new(&[], &[]);
        let p = w.probe(Vec3::ZERO, &sphere_dirs(256), &[]);
        for n in [Vec3::Y, Vec3::X, Vec3::NEG_Z] {
            let (vis, b) = p.eval(n);
            assert!(close(vis, 1.0, 0.03), "{vis}");
            assert_eq!(b, Vec3::ZERO);
        }
    }

    #[test]
    fn closed_white_box_is_all_bounce() {
        let w = BakeWorld::new(&cube(3.0), &[]);
        let p = w.probe(Vec3::new(0.3, -0.2, 0.1), &sphere_dirs(256), &[]);
        assert_eq!(p.back, 0.0);
        for n in [Vec3::Y, Vec3::NEG_Y, Vec3::X, Vec3::Z] {
            let (vis, b) = p.eval(n);
            assert!(close(vis, 0.0, 1e-6));
            assert!(close(b.x, 1.0, 0.03) && close(b.y, 1.0, 0.03), "{b}");
        }
    }

    #[test]
    fn inside_out_box_is_invalid() {
        // the same cube seen from inside with its faces turned outwards: all back faces (culled,
        // so the probe sees the sky, but it is marked as inside)
        let mut t = cube(3.0);
        for v in &mut t {
            v.normal = (-Vec3::from(v.normal)).into();
        }
        let p = BakeWorld::new(&t, &[]).probe(Vec3::ZERO, &sphere_dirs(64), &[]);
        assert!(!p.valid() && p.back == 1.0);
        assert!(close(p.eval(Vec3::Y).0, 1.0, 0.05));
    }

    #[test]
    fn floor_lights_the_lower_half() {
        let mut t = Vec::new();
        quad(&mut t, Vec3::new(0.0, -1.0, 0.0), Vec3::Z * 1000.0, Vec3::X * 1000.0, 0.5);
        let p = BakeWorld::new(&t, &[]).probe(Vec3::ZERO, &sphere_dirs(1024), &[]);
        let (up, b_up) = p.eval(Vec3::Y);
        let (down, b_down) = p.eval(Vec3::NEG_Y);
        let (side, _) = p.eval(Vec3::X);
        assert!(close(up, 1.0, 0.03) && close(down, 0.0, 0.03) && close(side, 0.5, 0.03), "{up} {down} {side}");
        // a radiance of lin(0.5) from below
        let l = 0.5f32.powf(GAMMA);
        assert!(close(b_down.x, l, 0.02) && close(b_up.x, 0.0, 0.02), "{b_down} {b_up}");
    }

    #[test]
    fn under_the_floor_is_invalid_beside_a_wall_is_not() {
        // a probe under the ground sees its back over the whole upper half
        let mut t = Vec::new();
        quad(&mut t, Vec3::new(0.0, 1.0, 0.0), Vec3::Z * 1000.0, Vec3::X * 1000.0, 0.5);
        assert!(!BakeWorld::new(&t, &[]).probe(Vec3::ZERO, &sphere_dirs(256), &[]).valid());
        // a one-sided wall 1 m away, facing away from the probe: culled, so it lets the sky through
        let mut t = Vec::new();
        quad(&mut t, Vec3::new(0.0, 0.0, -1.0), Vec3::Y * 50.0, Vec3::X * 50.0, 0.5);
        let p = BakeWorld::new(&t, &[]).probe(Vec3::ZERO, &sphere_dirs(256), &[]);
        assert!(p.valid() && close(p.eval(Vec3::NEG_Z).0, 1.0, 0.05), "{} {}", p.back, p.eval(Vec3::NEG_Z).0);
    }

    #[test]
    fn roof_blocks_the_sun_overhead_not_from_the_side() {
        // a 20 m wide roof 3 m up, facing down; no walls: a low sun from the side passes under it
        let mut t = Vec::new();
        quad(&mut t, Vec3::new(0.0, 3.0, 0.0), Vec3::X * 10.0, Vec3::Z * 1000.0, 0.5);
        let w = BakeWorld::new(&t, &[]);
        let dirs = sphere_dirs(64);
        assert_eq!(w.probe(Vec3::ZERO, &dirs, &sun_rays(Vec3::Y)).sun, 0.0);
        assert_eq!(w.probe(Vec3::ZERO, &dirs, &sun_rays(Vec3::new(0.97, 0.2, 0.0).normalize())).sun, 1.0);
        // half under the edge: part of the cone passes
        let edge = Vec3::new(10.0, 0.0, 0.0);
        let s = w.probe(edge, &dirs, &sun_rays(Vec3::Y)).sun;
        assert!(s > 0.2 && s < 0.8, "{s}");
        // open space, and a sun below the horizon
        assert_eq!(BakeWorld::new(&[], &[]).probe(Vec3::ZERO, &dirs, &sun_rays(Vec3::Y)).sun, 1.0);
        assert!(sun_rays(Vec3::new(0.0, -0.1, 1.0)).is_empty());
    }

    #[test]
    fn bvh_matches_brute_force() {
        let mut seed = 7u32;
        let mut r = || {
            seed ^= seed << 13;
            seed ^= seed >> 17;
            seed ^= seed << 5;
            seed as f32 / u32::MAX as f32 * 2.0 - 1.0
        };
        let tris: Vec<[Vec3; 3]> = (0..500)
            .map(|_| {
                let c = Vec3::new(r(), r(), r()) * 20.0;
                [c, c + Vec3::new(r(), r(), r()) * 3.0, c + Vec3::new(r(), r(), r()) * 3.0]
            })
            .collect();
        let bvh = Bvh::new(tris.clone());
        for _ in 0..500 {
            let o = Vec3::new(r(), r(), r()) * 25.0;
            let d = Vec3::new(r(), r(), r()).normalize_or(Vec3::X);
            let brute = tris.iter().enumerate().filter_map(|(i, t)| triangle(o, d, t).filter(|h| h.0 < 100.0).map(|h| (h.0, i))).min_by(|a, b| a.0.total_cmp(&b.0));
            let got = bvh.cast(o, d, 100.0).map(|h| (h.t, h.tri));
            assert_eq!(brute.map(|b| b.1), got.map(|g| g.1));
        }
    }

    #[test]
    fn volume_interpolates_and_fills() {
        // one brick in a white box: every probe sees only bounce; the box's walls are outside
        let w = BakeWorld::new(&cube(20.0), &[]);
        let v = ProbeVolume::bake(&w, vec![[0, 0, 0]], Vec3::Y);
        assert_eq!(v.stats.points, SIDE * SIDE * SIDE);
        let (vis, b) = v.irradiance(Vec3::new(3.3, 4.1, 5.9), Vec3::Y);
        assert!(close(vis, 0.0, 1e-6) && close(b.y, 1.0, 0.05), "{vis} {b}");
        // outside the brick: open space
        assert_eq!(v.irradiance(Vec3::new(-1.0, 1.0, 1.0), Vec3::Y).0, 1.0);
        // an invalid point is filled from its neighbours
        let points = vec![[0, 0, 0], [1, 0, 0]];
        let mut probes = vec![Probe { back: 1.0, ..Probe::OPEN }, Probe { vis: [0.5, 0.0, 0.0, 0.0], ..Probe::OPEN }];
        probes[1].sun = 0.25;
        assert_eq!(fill_invalid(&points, &mut probes), 0);
        assert_eq!((probes[0].vis[0], probes[0].sun), (0.5, 0.25));
    }

    #[test]
    fn pack_places_bricks() {
        let probe = |x: f32| Probe { vis: [x, 0.0, 0.0, 0.0], bounce: [[0.0; 4], [x * 2.0, 0.0, 0.0, 0.0], [0.0; 4]], back: 0.0, sun: x };
        let bricks = vec![[-2, 0, 5], [1, 1, 5]];
        let probes = (0..2).flat_map(|b| (0..SIDE * SIDE * SIDE).map(move |l| probe((b * 1000 + l) as f32 / 4096.0))).collect();
        let v = ProbeVolume { index: bricks.iter().enumerate().map(|(i, &b)| (b, i as u32)).collect(), bricks, probes, stats: Default::default() };
        let p = v.pack().unwrap();
        assert_eq!((p.origin, p.index_size), ([-2, 0, 5], [4, 2, 1]));
        assert_eq!((p.index[0], p.index[3 + 4], p.index[1]), (1, 2, 0));
        assert_eq!(p.atlas_size, [(ATLAS_SLOTS * SIDE) as u32, SIDE as u32, (SIDE * 4) as u32]);
        // probe (1, 2, 3) of slot 1, bounce r = channel 1
        let [w, h] = [p.atlas_size[0] as usize, p.atlas_size[1] as usize];
        let l = local(1, 2, 3);
        let t = (SIDE + 1) + w * (2 + h * (SIDE + 3));
        assert_eq!(half::f16::from_bits(p.atlas[t * 4]).to_f32(), v.probes[SIDE * SIDE * SIDE + l].bounce[0][0]);
        // the sun of the same probe, one channel's depth
        assert_eq!(p.sun.len(), p.atlas.len() / 16);
        let t = (SIDE + 1) + w * (2 + h * 3);
        assert_eq!(p.sun[t], (v.probes[SIDE * SIDE * SIDE + l].sun * 255.0).round() as u8);
    }

    #[test]
    fn cache_round_trip_and_key() {
        let w = BakeWorld::new(&cube(3.0), &[]);
        let bricks = vec![[-1, -1, -1], [0, -1, -1]];
        let key = cache_key(&w, &bricks);
        assert_ne!(key, cache_key(&w, &bricks[..1]));
        assert_ne!(key, cache_key(&BakeWorld::new(&cube(3.5), &[]), &bricks));
        assert_ne!(sun_key(key, Vec3::Y), sun_key(key, Vec3::new(0.0, 0.8, 0.6)));
        let p = ProbeVolume::bake(&w, bricks, Vec3::Y).pack().unwrap();
        let dir = std::env::temp_dir().join(format!("motorview-probes-test-{}", std::process::id()));
        write_cache(&dir, key, &p).unwrap();
        let path = dir.join(format!("{key:016x}.probes"));
        let q = read_cache(&path, key).unwrap();
        assert!(read_cache(&path, key ^ 1).is_none());
        std::fs::remove_dir_all(&dir).unwrap();
        assert_eq!((q.index, q.index_size, q.origin, q.atlas, q.atlas_size, q.sun), (p.index, p.index_size, p.origin, p.atlas, p.atlas_size, p.sun));
    }

    #[test]
    fn cancelled_bake_gives_nothing() {
        let w = BakeWorld::new(&cube(3.0), &[]);
        assert!(ProbeVolume::bake_on(&w, vec![[0, 0, 0]], Vec3::Y, 2, &AtomicBool::new(true)).is_none());
    }
}
