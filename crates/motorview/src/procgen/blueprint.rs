//! REMASTER (#478 phase 3): a car as a model-file blueprint, the starting point of a hand-made model
//! (`motorview --export-blueprint N out.glb`), and the writer behind `examples/glbcar.rs`.
//!
//! The file follows the model contract (`carmodel`, 09-remaster 9.44): glTF axes, the car's own
//! metres (scale 1, hubs where the simulation has them), materials named `paint1..3` / `glass_*` /
//! `trim_*` / `lamp_front` / `lamp_rear` (the car's paint zones found by colour), its textures, and
//! - `body`: the generated body when the car has a recipe, else the original (OBJ0) one;
//! - `wheel_FL/FR/BR/BL`: empty nodes on the hubs (the loader then keeps the generated wheels, so a
//!   modeller can build the body first, or add a wheel mesh under the node);
//! - `ref_*` nodes, which the loader skips: `ref_original` (the original body, when `body` is the
//!   generated one), `ref_tyre_FL..BL` (the wheel's size, 16 sides) and `ref_ground` (a quad under
//!   the tyres). Delete them or leave them: the game ignores them.
//!
//! Every choice here is a design choice [design choice].

use super::carmodel::{self, GlbNode, GlbPart, WHEEL_NODES};
use super::{wheel, WheelSpec, CALIPER_ENV, METAL_ENV, TYRE_ENV};
use crate::scene::{self, ShowroomCar, Vertex};
use glam::Vec3;
use std::collections::BTreeMap;
use std::path::Path;

/// textured faces whose middle is this close (m) to a light halo are the lamp [design choice]
const LAMP_REACH: f32 = 0.15;

/// sides of a `ref_tyre_*` cylinder
const TYRE_SIDES: usize = 16;

/// what `build` writes
pub struct Opts {
    /// the file's root node scales by this and moves by `offset` (the loader's fit has to undo them)
    pub scale: f32,
    pub offset: [f32; 3],
    /// write the four `wheel_*` nodes
    pub wheel_nodes: bool,
    /// which of them hold the generated wheel's mesh (the others are empty)
    pub wheel_meshes: [bool; 4],
    /// textured faces round a halo of the original body are `lamp_front` / `lamp_rear`
    pub lamps: bool,
    /// add the `ref_*` nodes
    pub refs: bool,
}

/// What `build` made.
pub struct Built {
    pub bytes: Vec<u8>,
    pub tris: usize,
    pub lamp_tris: usize,
    pub materials: usize,
    pub images: usize,
    pub wheel_nodes: usize,
    /// the wheel nodes that hold a mesh, "FL" ...
    pub wheel_meshes: Vec<String>,
}

/// view space (right +x, front -z) to glTF (right -x, front +z)
fn gl(p: [f32; 3]) -> [f32; 3] {
    [-p[0], p[1], -p[2]]
}

/// One part per (material, texture layer); `layers` collects the texture layers written as images.
struct Parts {
    parts: BTreeMap<(String, i32), GlbPart>,
}

impl Parts {
    fn new() -> Parts {
        Parts { parts: BTreeMap::new() }
    }

    /// triangle `t` (view space, less `centre`) into the part `name`
    fn add(&mut self, layers: &mut Vec<usize>, name: String, t: &[Vertex], centre: Vec3) {
        let v = &t[0];
        let layer = v.layer as i32;
        let image = (layer >= 0).then(|| {
            layers.iter().position(|&l| l == layer as usize).unwrap_or_else(|| {
                layers.push(layer as usize);
                layers.len() - 1
            })
        });
        let p = self.parts.entry((name.clone(), layer)).or_insert_with(|| GlbPart {
            material: name,
            color: [v.color[0], v.color[1], v.color[2], 1.0].map(carmodel::srgb_to_linear),
            image,
            pos: Vec::new(),
            normal: Vec::new(),
            uv: Vec::new(),
        });
        for v in t {
            p.pos.push(gl((Vec3::from(v.pos) - centre).to_array()));
            p.normal.push(gl(v.normal));
            p.uv.push(v.uv);
        }
    }

    fn done(self) -> Vec<GlbPart> {
        self.parts.into_values().collect()
    }
}

/// distance from `p` to the segment a-b
fn to_segment(p: Vec3, a: Vec3, b: Vec3) -> f32 {
    let ab = b - a;
    let t = if ab.length_squared() > 0.0 { ((p - a).dot(ab) / ab.length_squared()).clamp(0.0, 1.0) } else { 0.0 };
    p.distance(a + ab * t)
}

/// The contract material of triangle `t` of a car body, and whether it is a lamp face: the paint zone
/// by colour (`own`: CarColour1..3, 0..1), `lamp_front` / `lamp_rear` for textured faces at a halo,
/// else `glass_r_g_b` (the paint band's other colours) or `trim_r_g_b`.
fn material(t: &[Vertex], own: &[[f32; 3]; 3], halos: &[scene::HaloInst], lamps: bool) -> (String, bool) {
    let v = &t[0];
    let rgb = [v.color[0], v.color[1], v.color[2]];
    let paint = (0.5..2.5).contains(&v.env) && v.layer < 0.0;
    let zone = own.iter().position(|o| paint && o.iter().zip(rgb).all(|(a, b)| (a - b).abs() < 2e-3));
    let byte = rgb.map(|c| (c * 255.0).round() as u8);
    let mid = t.iter().map(|v| Vec3::from(v.pos)).sum::<Vec3>() / 3.0;
    let lamp = lamps && !paint && v.layer >= 0.0 && halos.iter().any(|h| to_segment(mid, h.p0, h.p1) < LAMP_REACH);
    match zone {
        Some(k) => (format!("paint{}", k + 1), false),
        None if lamp => (if mid.z < 0.0 { "lamp_front".into() } else { "lamp_rear".into() }, true),
        None if paint => (format!("glass_{}_{}_{}", byte[0], byte[1], byte[2]), false),
        None => (format!("trim_{}_{}_{}", byte[0], byte[1], byte[2]), false),
    }
}

/// `body` (view space triangles, `c`'s layers) as a contract `.glb`: the body, the wheel nodes on `c`'s
/// hubs and, with `o.refs`, the reference nodes (`original`: the original body as `ref_original`, when
/// `body` is another one). `own`: the car's CarColour1..3 as 0..1.
pub fn build(c: &ShowroomCar, body: &[Vertex], original: Option<&[Vertex]>, own: [[f32; 3]; 3], o: &Opts) -> Built {
    let mut layers: Vec<usize> = Vec::new();
    let mut parts = Parts::new();
    let mut lamp_tris = 0;
    for t in body.as_chunks::<3>().0 {
        let (name, lamp) = material(t, &own, &c.halos, o.lamps);
        lamp_tris += lamp as usize;
        parts.add(&mut layers, name, t, Vec3::ZERO);
    }
    // the hub nodes, each holding its generated wheel when asked (centred on the hub, as the renderer wants it)
    let mut nodes: Vec<GlbNode> = Vec::new();
    let mut wheel_nodes = 0;
    for (i, (name, hub)) in WHEEL_NODES.iter().zip(c.hubs).enumerate().filter(|(_, (_, h))| o.wheel_nodes && *h != Vec3::ZERO) {
        let mut wp = Parts::new();
        if let Some(w) = c.wheels[i].filter(|_| o.wheel_meshes[i]) {
            let spec = WheelSpec { radius: w.radius, half_width: w.half_width, side: w.side, face: w.face, rim: None, scale: w.scale, tyre: w.tyre };
            for t in wheel(&spec).as_chunks::<3>().0 {
                let env = t[0].env;
                let name = match env {
                    e if e == TYRE_ENV => "rubber".to_string(),
                    e if e == METAL_ENV => "chrome".to_string(),
                    e if e == CALIPER_ENV => "caliper".to_string(),
                    _ => format!("wheel_{}", t[0].layer as i32),
                };
                wp.add(&mut layers, name, t, Vec3::ZERO);
            }
        }
        nodes.push(GlbNode { name, at: gl(hub.to_array()), parts: wp.done() });
        wheel_nodes += 1;
    }
    if o.refs {
        if let Some(orig) = original {
            let mut rp = Parts::new();
            for t in orig.as_chunks::<3>().0 {
                rp.add(&mut layers, material(t, &own, &c.halos, true).0, t, Vec3::ZERO);
            }
            nodes.push(GlbNode { name: "ref_original", at: [0.0; 3], parts: rp.done() });
        }
        nodes.extend(reference_nodes(c));
    }
    let images: Vec<Vec<u8>> = layers.iter().map(|&l| png(&c.layers[l], scene::TEX)).collect();
    let parts = parts.done();
    let bytes = carmodel::write_glb(&parts, &images, &nodes, o.scale, o.offset);
    let wheel_meshes = nodes.iter().filter(|n| n.name.starts_with("wheel_") && !n.parts.is_empty()).map(|n| n.name["wheel_".len()..].to_string()).collect();
    Built { bytes, tris: parts.iter().map(|p| p.pos.len() / 3).sum(), lamp_tris, materials: parts.len(), images: images.len(), wheel_nodes, wheel_meshes }
}

/// `ref_tyre_*` (each wheel's size) and `ref_ground` (a quad under the tyres), on the hubs of `c`.
fn reference_nodes(c: &ShowroomCar) -> Vec<GlbNode<'static>> {
    const NAMES: [&str; 4] = ["ref_tyre_FL", "ref_tyre_FR", "ref_tyre_BR", "ref_tyre_BL"];
    let mut out = Vec::new();
    let (mut ground, mut mid, mut n) = (f32::MAX, Vec3::ZERO, 0.0);
    for (name, (w, hub)) in NAMES.iter().zip(c.wheels.iter().zip(c.hubs)) {
        let Some(w) = w.filter(|_| hub != Vec3::ZERO) else { continue };
        ground = ground.min(hub.y - w.radius);
        mid += hub;
        n += 1.0;
        out.push(GlbNode { name, at: gl(hub.to_array()), parts: vec![reference("ref_tyre", [0.1, 0.1, 0.1, 1.0], tyre(w.radius, w.half_width))] });
    }
    if n > 0.0 {
        // 6 m wide, 10 m long, centred between the hubs, in glTF axes; two triangles facing up
        let g = gl((mid / n).to_array());
        let q = [[g[0] - 3.0, ground, g[2] - 5.0], [g[0] - 3.0, ground, g[2] + 5.0], [g[0] + 3.0, ground, g[2] + 5.0], [g[0] + 3.0, ground, g[2] - 5.0]].map(Vec3::from);
        let tris = [face(q[0], q[1], q[2], Vec3::Y), face(q[0], q[2], q[3], Vec3::Y)].concat();
        out.push(GlbNode { name: "ref_ground", at: [0.0; 3], parts: vec![reference("ref_ground", [0.3, 0.3, 0.3, 1.0], tris)] });
    }
    out
}

/// A tyre's cylinder about the node origin, along x (glTF axes): `TYRE_SIDES` sides and two caps.
fn tyre(radius: f32, half_width: f32) -> Vec<[f32; 3]> {
    let ring = |x: f32, k: usize| {
        let a = std::f32::consts::TAU * k as f32 / TYRE_SIDES as f32;
        Vec3::new(x, radius * a.cos(), radius * a.sin())
    };
    let mut tris = Vec::new();
    for k in 0..TYRE_SIDES {
        let (a, b, c, d) = (ring(-half_width, k), ring(-half_width, k + 1), ring(half_width, k), ring(half_width, k + 1));
        let m = (a + b + c + d) / 4.0;
        let radial = Vec3::new(0.0, m.y, m.z);
        tris.extend(face(a, c, d, radial).into_iter().chain(face(a, d, b, radial)));
        tris.extend(face(Vec3::new(-half_width, 0.0, 0.0), b, a, -Vec3::X));
        tris.extend(face(Vec3::new(half_width, 0.0, 0.0), c, d, Vec3::X));
    }
    tris
}

/// Triangle abc, turned round when it faces away from `out`.
fn face(a: Vec3, b: Vec3, c: Vec3, out: Vec3) -> [[f32; 3]; 3] {
    if (b - a).cross(c - a).dot(out) < 0.0 { [a, c, b] } else { [a, b, c] }.map(|p| p.to_array())
}

/// A reference part: flat-lit `colour` (linear), the normals of each triangle's face.
fn reference(name: &str, colour: [f32; 4], tris: Vec<[f32; 3]>) -> GlbPart {
    let normal = tris
        .as_chunks::<3>()
        .0
        .iter()
        .flat_map(|t| {
            let [a, b, c] = t.map(Vec3::from);
            [(b - a).cross(c - a).normalize_or(Vec3::Y).to_array(); 3]
        })
        .collect();
    GlbPart { material: name.into(), color: colour, image: None, pos: tris, normal, uv: Vec::new() }
}

fn png(rgba: &[u8], size: u32) -> Vec<u8> {
    let mut out = Vec::new();
    let mut e = png::Encoder::new(&mut out, size, size);
    e.set_color(png::ColorType::Rgba);
    e.set_depth(png::BitDepth::Eight);
    e.write_header().and_then(|mut w| w.write_image_data(rgba)).expect("png");
    out
}

/// Car `car`'s blueprint written to `out` (folders made as needed); the summary line. The body is the
/// generated one when the car has a recipe (`remaster/cars/carNN.body`, `--cars-dir` included), else the
/// original; the car's own model file is not used (the source is forced to Generated meanwhile).
pub fn export(root: &Path, track: u32, car: u32, out: &Path) -> Result<String, String> {
    let prev = carmodel::source();
    carmodel::set_source(carmodel::CarSource::Generated);
    let c = scene::load_showroom(root, track, car);
    carmodel::set_source(prev);
    let c = c?;
    let own = scene::car_colours(root, car).map(|c| c.map(|b| b as f32 / 255.0));
    let generated = !c.generated.is_empty();
    let (body, original): (&[Vertex], Option<&[Vertex]>) = if generated { (&c.generated, Some(&c.body)) } else { (&c.body, None) };
    // no wheel meshes: the nodes mark the hubs, the loader keeps the generated wheels until a model brings its own
    let opts = Opts { scale: 1.0, offset: [0.0; 3], wheel_nodes: true, wheel_meshes: [false; 4], lamps: !generated, refs: true };
    let built = build(&c, body, original, own, &opts);
    if let Some(d) = out.parent().filter(|d| !d.as_os_str().is_empty()) {
        std::fs::create_dir_all(d).map_err(|e| format!("{}: {e}", d.display()))?;
    }
    std::fs::write(out, &built.bytes).map_err(|e| format!("{}: {e}", out.display()))?;
    Ok(format!(
        "car{car:02} '{}': blueprint of the {} body, {} triangles, {} materials, {} textures, {} wheel nodes, scale 1 -> {}",
        c.name,
        if generated { "generated" } else { "original" },
        built.tris,
        built.materials,
        built.images,
        built.wheel_nodes,
        out.display()
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scene::ShowroomWheel;

    fn v(p: [f32; 3], rgb: [f32; 3], env: f32) -> Vertex {
        Vertex { pos: p, uv: [0.0; 2], color: [rgb[0], rgb[1], rgb[2], 1.0], layer: -1.0, normal: [0.0, 1.0, 0.0], env }
    }

    /// a car of one paint triangle and one trim triangle, hubs 1.6 m apart and 2.7 m long (view space)
    fn car() -> ShowroomCar {
        let w = Some(ShowroomWheel { radius: 0.3, half_width: 0.12, side: 1.0, face: None, tyre: None, scale: [1.0; 3] });
        ShowroomCar {
            name: "test".into(),
            body: Vec::new(),
            generated: Vec::new(),
            layers: Vec::new(),
            hubs: [Vec3::new(0.8, 0.3, -1.3), Vec3::new(-0.8, 0.3, -1.3), Vec3::new(-0.8, 0.3, 1.4), Vec3::new(0.8, 0.3, 1.4)],
            wheels: [w; 4],
            halos: Vec::new(),
            model: None,
        }
    }

    fn body() -> Vec<Vertex> {
        let red = [1.0, 0.0, 0.0];
        let grey = [0.2, 0.2, 0.2];
        vec![
            v([-1.0, 0.2, -2.0], red, 1.0),
            v([1.0, 0.2, -2.0], red, 1.0),
            v([0.0, 0.9, 0.0], red, 1.0),
            v([-1.0, 0.2, 2.0], grey, 0.0),
            v([1.0, 0.2, 2.0], grey, 0.0),
            v([0.0, 0.9, 0.5], grey, 0.0),
        ]
    }

    fn opts() -> Opts {
        Opts { scale: 1.0, offset: [0.0; 3], wheel_nodes: true, wheel_meshes: [false; 4], lamps: false, refs: true }
    }

    #[test]
    fn tyre_outlines_face_out() {
        let tris = tyre(0.3, 0.12);
        assert_eq!(tris.len(), TYRE_SIDES * 12);
        for t in tris.as_chunks::<3>().0 {
            let [a, b, c] = t.map(Vec3::from);
            let n = (b - a).cross(c - a);
            let m = (a + b + c) / 3.0;
            // a cap (all points at x = ±half) faces ±x, a side faces away from the axis
            let out = if (m.x.abs() - 0.12).abs() < 1e-4 && a.x == b.x && b.x == c.x { Vec3::new(m.x, 0.0, 0.0) } else { Vec3::new(0.0, m.y, m.z) };
            assert!(n.dot(out) > 0.0, "{t:?} faces in");
        }
    }

    #[test]
    fn blueprint_loads_back_on_the_hubs_without_its_references() {
        let c = car();
        let b = build(&c, &body(), Some(&body()), [[1.0, 0.0, 0.0], [0.0; 3], [0.0; 3]], &opts());
        // the paint triangle is paint1, the grey one trim
        assert_eq!((b.tris, b.wheel_nodes, b.wheel_meshes.len()), (2, 4, 0));
        let m = carmodel::parse(&b.bytes).unwrap();
        // ref_original, the tyres and the ground are not the car
        assert_eq!(m.tris.len(), 2);
        assert!(m.wheel_tris.iter().all(Vec::is_empty));
        let fit = carmodel::fit(&m, &c.hubs, None).unwrap();
        assert!(fit.hub_err.unwrap() < 1e-3, "{:?}", fit.hub_err);
        assert!((fit.scale - 1.0).abs() < 1e-4);
        assert!(m.tris.iter().any(|t| t.mat == carmodel::Mat::Paint(0)));
        assert_eq!(m.plain_materials.iter().map(String::as_str).collect::<Vec<_>>(), ["trim_51_51_51"]);
    }

    #[test]
    fn blueprint_without_refs_has_none_and_keeps_the_wheel_nodes() {
        let c = car();
        let with = build(&c, &body(), Some(&body()), [[1.0, 0.0, 0.0], [0.0; 3], [0.0; 3]], &opts());
        let without = build(&c, &body(), None, [[1.0, 0.0, 0.0], [0.0; 3], [0.0; 3]], &Opts { refs: false, ..opts() });
        assert!(without.bytes.len() < with.bytes.len());
        let text = String::from_utf8_lossy(&without.bytes);
        assert!(!text.contains("ref_") && text.contains("wheel_BL"));
        assert!(String::from_utf8_lossy(&with.bytes).contains("ref_ground"));
    }
}
