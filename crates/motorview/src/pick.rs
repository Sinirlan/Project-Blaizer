//! CPU ray picking against the rendered geometry (motorview's target overlay): world
//! triangles grouped per MYW object, car meshes and scene objects under their per-frame model
//! matrices. All in view space, like the renderer.

use crate::probes::Bvh;
use crate::scene::{Scene, CAR_MESHES};
use glam::{Mat4, Vec3};
use std::ops::Range;

/// What a ray hit.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Target {
    /// MYW object (index, file offset) and texture (index into `Scene::world_textures`)
    World { object: usize, offset: usize, texture: Option<usize> },
    /// car slot and part (0 body, 1..=4 wheels FL, FR, BR, BL)
    Car { slot: usize, part: usize },
    /// scene object instance (`Props::props` order) and its mesh
    Prop { index: usize, mesh: usize },
}

#[derive(Clone, Copy, Debug)]
pub struct Hit {
    /// distance along the (unit) ray
    pub t: f32,
    /// view space
    pub point: Vec3,
    /// the object's position, view space: the model origin of a car part or scene object,
    /// the bounding-box centre of a world object (MYW objects have no origin of their own)
    pub origin: Vec3,
    pub target: Target,
    /// the triangle hit: in `Scene::tris` for the world, else in the car or scene object mesh
    pub tri: usize,
}

/// Bounding volume of a mesh in its own space (view space for the world): box and sphere
/// around the box centre.
#[derive(Clone, Copy, Debug)]
pub struct Bounds {
    pub lo: Vec3,
    pub hi: Vec3,
    pub center: Vec3,
    pub radius: f32,
}

impl Bounds {
    fn of(tris: &[[Vec3; 3]]) -> Bounds {
        let (lo, hi) = tris.iter().flatten().fold((Vec3::MAX, Vec3::MIN), |(lo, hi), &p| (lo.min(p), hi.max(p)));
        let center = (lo + hi) * 0.5;
        let radius = tris.iter().flatten().map(|p| p.distance(center)).fold(0.0, f32::max);
        Bounds { lo, hi, center, radius }
    }
}

struct Mesh {
    tris: Vec<[Vec3; 3]>,
    bounds: Bounds,
}

impl Mesh {
    fn new(tris: Vec<[Vec3; 3]>) -> Mesh {
        let bounds = Bounds::of(&tris);
        Mesh { tris, bounds }
    }

    /// Nearest hit in this mesh's space: ray `o + t d` (d need not be unit length).
    fn cast(&self, o: Vec3, d: Vec3, max_t: f32) -> Option<(f32, usize)> {
        if self.tris.is_empty() || !sphere_hit(o, d, self.bounds.center, self.bounds.radius, max_t) {
            return None;
        }
        let mut best: Option<(f32, usize)> = None;
        for (i, t) in self.tris.iter().enumerate() {
            if let Some(h) = triangle(o, d, t) {
                if h < best.map_or(max_t, |b| b.0) {
                    best = Some((h, i));
                }
            }
        }
        best
    }
}

fn sphere_hit(o: Vec3, d: Vec3, c: Vec3, r: f32, max_t: f32) -> bool {
    let oc = c - o;
    let t = (oc.dot(d) / d.length_squared()).clamp(0.0, max_t);
    (o + d * t).distance_squared(c) <= r * r
}

/// Möller–Trumbore, both faces (nothing is back-face culled).
fn triangle(o: Vec3, d: Vec3, [a, b, c]: &[Vec3; 3]) -> Option<f32> {
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
    (t > 1e-4).then_some(t)
}

pub struct Picker {
    /// per MYW object: its bounds, triangle range in `Scene::tris`, file offset
    world: Vec<(Bounds, Range<usize>, usize)>,
    /// every world triangle in one hierarchy, objects concatenated in order
    world_bvh: Bvh,
    /// per `world_bvh` triangle: (object index, index in `Scene::tris`)
    world_tri: Vec<(u32, u32)>,
    world_texture: Vec<u32>,
    cars: Vec<Mesh>,
    props: Vec<Mesh>,
    world_textures: Vec<String>,
    prop_names: Vec<String>,
    car_names: Vec<String>,
    /// REMASTER World Editor: the fill's pieces (index into `FillFile::pieces`), view space
    fill: Vec<(usize, Mesh)>,
}

fn tris_of(v: &[crate::scene::Vertex]) -> Vec<[Vec3; 3]> {
    v.as_chunks::<3>().0.iter().map(|t| [0, 1, 2].map(|k| Vec3::from(t[k].pos))).collect()
}

impl Picker {
    pub fn new(s: &Scene) -> Picker {
        let (mut world, mut all, mut world_tri) = (Vec::new(), Vec::new(), Vec::new());
        for (object, (r, off)) in s.world_objects.iter().enumerate() {
            let tris = tris_of(&s.tris[r.start * 3..r.end * 3]);
            world.push((Bounds::of(&tris), r.clone(), *off));
            world_tri.extend(r.clone().map(|g| (object as u32, g as u32)));
            all.extend(tris);
        }
        Picker {
            world,
            world_bvh: Bvh::new(all),
            world_tri,
            world_texture: s.world_tri_texture.clone(),
            cars: s.cars.iter().map(|m| Mesh::new(tris_of(m))).collect(),
            props: s.props.iter().map(|m| Mesh::new(tris_of(m))).collect(),
            world_textures: s.world_textures.clone(),
            prop_names: s.prop_names.clone(),
            car_names: s.car_names.clone(),
            fill: Vec::new(),
        }
    }

    /// Take the fill mesh: `pieces` are (piece index, vertex range in `verts`), as `fill::mesh_pieces` gives.
    pub fn set_fill(&mut self, verts: &[crate::scene::Vertex], pieces: &[(usize, Range<usize>)]) {
        self.fill = pieces.iter().filter(|(_, r)| r.len() >= 3).map(|(i, r)| (*i, Mesh::new(tris_of(&verts[r.start / 3 * 3..r.end / 3 * 3])))).collect();
    }

    /// Nearest fill piece along `o + t dir` (view space, unit `dir`): (piece index, t). `skip` drops pieces
    /// (hidden ones).
    pub fn cast_fill(&self, o: Vec3, dir: Vec3, max_t: f32, skip: &dyn Fn(usize) -> bool) -> Option<(usize, f32)> {
        self.fill.iter().filter(|(i, _)| !skip(*i)).filter_map(|(i, m)| m.cast(o, dir, max_t).map(|(t, _)| (*i, t))).min_by(|a, b| a.1.total_cmp(&b.1))
    }

    /// Nearest ground along `o + t dir`: the original world or a fill piece (not those `skip`ped), t.
    /// Cars and scene objects are not ground. REMASTER World Editor placement.
    pub fn ground_t(&self, o: Vec3, dir: Vec3, max_t: f32, skip: &dyn Fn(usize) -> bool) -> Option<f32> {
        let world = self.cast(o, dir, max_t, &[], &[]).map(|h| h.t);
        let fill = self.cast_fill(o, dir, max_t, skip).map(|(_, t)| t);
        match (world, fill) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        }
    }

    /// Bounds of every MYW object (`Target::World::object` order), view space.
    pub fn world_bounds(&self) -> impl Iterator<Item = Bounds> + '_ {
        self.world.iter().map(|w| w.0)
    }

    /// Bounds of a car mesh (car-local) or a scene object mesh (object-local); None if empty.
    pub fn car_bounds(&self, mesh: usize) -> Option<Bounds> {
        self.cars.get(mesh).filter(|m| !m.tris.is_empty()).map(|m| m.bounds)
    }

    pub fn prop_bounds(&self, mesh: usize) -> Option<Bounds> {
        self.props.get(mesh).filter(|m| !m.tris.is_empty()).map(|m| m.bounds)
    }

    /// Short description of a target: kind, id and the game's own name for it.
    pub fn label(&self, t: Target) -> String {
        let name = |v: &[String], i: usize| v.get(i).cloned().unwrap_or_else(|| "?".into());
        match t {
            Target::World { object, offset, texture } => {
                let tex = texture.map_or("untextured".into(), |i| name(&self.world_textures, i));
                format!("world object #{object} (MYW +{offset:#x}), texture {tex}")
            }
            Target::Car { slot, part } => {
                let part = ["body", "wheel FL", "wheel FR", "wheel BR", "wheel BL"].get(part).copied().unwrap_or("?");
                format!("car slot {slot}: {} {part}", name(&self.car_names, slot))
            }
            Target::Prop { index, mesh } => format!("scene object #{index}: {}", name(&self.prop_names, mesh)),
        }
    }

    /// Nearest hit along `o + t dir` (view space, `dir` unit length), t <= `max_t`.
    /// `cars` / `props` are the frame's (mesh index, model matrix) lists.
    pub fn cast(&self, o: Vec3, dir: Vec3, max_t: f32, cars: &[(usize, Mat4)], props: &[(usize, Mat4)]) -> Option<Hit> {
        let mut best: Option<Hit> = None;
        let mut keep = |(t, tri): (f32, usize), origin: Vec3, target: Target| {
            if t < best.map_or(max_t, |b| b.t) {
                best = Some(Hit { t, point: o + dir * t, origin, target, tri });
            }
        };
        if let Some(h) = self.world_bvh.cast(o, dir, max_t) {
            let (object, tri) = self.world_tri[h.tri];
            let (object, tri) = (object as usize, tri as usize);
            let (bounds, _, offset) = &self.world[object];
            let texture = self.world_texture.get(tri).filter(|&&x| x != u32::MAX).map(|&x| x as usize);
            keep((h.t, tri), bounds.center, Target::World { object, offset: *offset, texture });
        }
        // instances: the ray goes into model space unnormalised, so t stays the view-space t
        let instance = |m: &Mesh, model: &Mat4| {
            let inv = model.inverse();
            m.cast(inv.transform_point3(o), inv.transform_vector3(dir), max_t)
        };
        for &(mesh, model) in cars {
            if let Some(h) = self.cars.get(mesh).and_then(|m| instance(m, &model)) {
                keep(h, model.w_axis.truncate(), Target::Car { slot: mesh / CAR_MESHES, part: mesh % CAR_MESHES });
            }
        }
        for (index, &(mesh, model)) in props.iter().enumerate() {
            if let Some(h) = self.props.get(mesh).and_then(|m| instance(m, &model)) {
                keep(h, model.w_axis.truncate(), Target::Prop { index, mesh });
            }
        }
        best
    }
}

/// Line-list vertices for the edges of a triangle list (each shared edge drawn twice).
pub fn edges(tris: &[crate::scene::Vertex], color: [f32; 4]) -> Vec<crate::scene::Vertex> {
    let mut out = Vec::with_capacity(tris.len() * 2);
    for t in tris.as_chunks::<3>().0 {
        for k in 0..3 {
            for v in [t[k], t[(k + 1) % 3]] {
                out.push(crate::scene::Vertex { color, layer: -1.0, env: 0.0, ..v });
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scene::Vertex;

    fn quad_at(y: f32) -> Vec<Vertex> {
        let v = |x: f32, z: f32| Vertex { pos: [x, y, z], ..bytemuck::Zeroable::zeroed() };
        vec![v(-1.0, -1.0), v(1.0, -1.0), v(1.0, 1.0), v(-1.0, -1.0), v(1.0, 1.0), v(-1.0, 1.0)]
    }

    #[test]
    fn cast_fill_returns_the_nearest_piece_and_honours_skip() {
        let mut verts = quad_at(5.0);
        verts.extend(quad_at(2.0));
        let mut p = Picker { world: Vec::new(), world_bvh: Bvh::new(Vec::new()), world_tri: Vec::new(), world_texture: Vec::new(), cars: Vec::new(), props: Vec::new(), world_textures: Vec::new(), prop_names: Vec::new(), car_names: Vec::new(), fill: Vec::new() };
        p.set_fill(&verts, &[(7, 0..6), (3, 6..12)]);
        let (o, d) = (Vec3::new(0.2, 10.0, 0.1), Vec3::NEG_Y);
        let (i, t) = p.cast_fill(o, d, 100.0, &|_| false).unwrap();
        assert_eq!(i, 7);
        assert!((t - 5.0).abs() < 1e-4);
        assert_eq!(p.cast_fill(o, d, 100.0, &|i| i == 7).unwrap().0, 3);
        assert!(p.cast_fill(Vec3::new(9.0, 10.0, 0.0), d, 100.0, &|_| false).is_none());
        // the ground a thing placed on a user's piece stands on: its surface, not what lies below it
        assert!((p.ground_t(o, d, 100.0, &|_| false).unwrap() - 5.0).abs() < 1e-4);
        assert!((p.ground_t(o, d, 100.0, &|i| i == 7).unwrap() - 8.0).abs() < 1e-4);
        assert!(p.ground_t(o, d, 100.0, &|_| true).is_none());
    }

    /// The world hierarchy finds the same nearest triangle as testing every triangle.
    #[test]
    fn world_bvh_matches_brute_force() {
        let mut seed = 12345u32;
        let mut rnd = move || {
            seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
            (seed >> 8) as f32 / (1u32 << 24) as f32
        };
        let mut verts = Vec::new();
        let mut objects = Vec::new();
        for _ in 0..12 {
            let first = verts.len() / 3;
            let c = Vec3::new(rnd() * 60.0 - 30.0, rnd() * 10.0, rnd() * 60.0 - 30.0);
            for _ in 0..120 {
                let at = c + Vec3::new(rnd() * 8.0, rnd() * 8.0, rnd() * 8.0);
                for _ in 0..3 {
                    let p = at + Vec3::new(rnd() * 3.0, rnd() * 3.0, rnd() * 3.0);
                    verts.push(Vertex { pos: p.to_array(), ..bytemuck::Zeroable::zeroed() });
                }
            }
            objects.push((first..verts.len() / 3, objects.len() * 16));
        }
        let mut p = Picker { world: Vec::new(), world_bvh: Bvh::new(Vec::new()), world_tri: Vec::new(), world_texture: Vec::new(), cars: Vec::new(), props: Vec::new(), world_textures: Vec::new(), prop_names: Vec::new(), car_names: Vec::new(), fill: Vec::new() };
        let (mut all, mut tri_of) = (Vec::new(), Vec::new());
        for (object, (r, off)) in objects.iter().enumerate() {
            let tris = tris_of(&verts[r.start * 3..r.end * 3]);
            p.world.push((Bounds::of(&tris), r.clone(), *off));
            tri_of.extend(r.clone().map(|g| (object as u32, g as u32)));
            all.extend(tris);
        }
        p.world_bvh = Bvh::new(all);
        p.world_tri = tri_of;
        let brute: Vec<Mesh> = objects.iter().map(|(r, _)| Mesh::new(tris_of(&verts[r.start * 3..r.end * 3]))).collect();
        let mut hits = 0;
        for _ in 0..300 {
            let o = Vec3::new(rnd() * 80.0 - 40.0, rnd() * 30.0 - 5.0, rnd() * 80.0 - 40.0);
            let d = (Vec3::new(rnd() * 60.0 - 30.0, rnd() * 14.0, rnd() * 60.0 - 30.0) - o).normalize();
            let want = brute.iter().enumerate().filter_map(|(i, m)| m.cast(o, d, 500.0).map(|(t, _)| (t, i))).min_by(|a, b| a.0.total_cmp(&b.0));
            let got = p.cast(o, d, 500.0, &[], &[]);
            match (want, got) {
                (None, None) => {}
                (Some((t, i)), Some(h)) => {
                    hits += 1;
                    assert!((t - h.t).abs() < 1e-4, "t {t} vs {}", h.t);
                    assert!(matches!(h.target, Target::World { object, .. } if object == i));
                }
                (a, b) => panic!("brute {a:?} vs bvh {:?}", b.map(|h| h.t)),
            }
        }
        assert!(hits > 20, "test rays should hit something ({hits})");
    }
}
