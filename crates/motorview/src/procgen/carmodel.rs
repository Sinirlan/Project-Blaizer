//! REMASTER (#478) car bodies from model files: `remaster/cars/carNN.glb` (or `.gltf`; `--cars-dir`
//! first, as `bodyfile`) replaces the car's generated body, which replaces the original MYO one.
//! Remaster only: Classic always draws the MYO meshes. The contract (09-remaster 9.44):
//! - glTF 2.0, +Y up, the car's front towards +Z, its right towards -X (the glTF convention; Blender's
//!   exporter gives it for a car modelled nose towards -Y). View space is (-x, y, -z) of that: a turn
//!   of 180° about Y, so the winding stays.
//! - nodes `wheel_FL`, `wheel_FR`, `wheel_BR`, `wheel_BL`: their origin is the hub centre. The model
//!   is scaled (uniformly) and moved so those hubs land on the simulated ones; without two of them it
//!   is fitted to the MYO body's bounding box. The meshes under a wheel node (its own and its
//!   children's) are that wheel, drawn centred on the node's origin, spun about the car's x axis and
//!   steered like the generated one (`wheel`); a corner without them keeps the generated wheel.
//! - materials by name (lower case, a Blender `.001` suffix allowed): `paint1`..`paint3` take
//!   CarColour1..3 and the zone's finish, `glass` reflects like the MYO glass, `chrome` is polished
//!   metal, `lamp_front` / `lamp_rear` / `light_brake` are self-lit and carry the car's halos
//!   (`lamps`), `caliper` (in a wheel) does not spin, the rest keep their base colour (times COLOR_0
//!   and the base colour texture).
//! - nodes named `ref_*` (and everything under them) are references for the modeller (the blueprint's
//!   original body, ground and tyre outlines): they are skipped.
//!
//! Every choice here is a design choice [design choice], not from the original.

use crate::scene::Vertex;
use crate::texpack::Map;
use glam::{Mat3, Mat4, Vec3};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU8, Ordering};

/// above this many triangles a body gets a warning: 8 cars of it on track cost frame time [design choice]
pub const CAR_TRI_BUDGET: usize = 80_000;

/// a fitted track width or wheelbase further than this (fraction) from the simulated one gets a warning
pub const WHEEL_FIT_TOLERANCE: f32 = 0.05;

/// an imported wheel's tyre centre further than this (m) from its node's origin gets a warning: it
/// spins about the origin, so it wobbles [design choice]
pub const WHEEL_CENTRE_TOLERANCE: f32 = 0.005;

/// lamp parts whose centres are closer than this (m) give one halo (a cluster of projectors, a split
/// lens) [design choice]
pub const LAMP_MERGE: f32 = 0.15;

/// The wheel node names, in `Scene::wheel_hubs` order (FL, FR, BR, BL).
pub const WHEEL_NODES: [&str; 4] = ["wheel_FL", "wheel_FR", "wheel_BR", "wheel_BL"];

/// The places `car`'s model file may be: `carNN.glb`, `carNN.gltf`, where `bodyfile` looks for recipes
/// (whether or not they exist: motorview watches them for a file that appears).
pub fn candidates(root: &Path, car: u32) -> [PathBuf; 2] {
    ["glb", "gltf"].map(|ext| super::bodyfile::override_path(root, &format!("car{car:02}.{ext}")))
}

/// The model file of `car`: `carNN.glb`, else `carNN.gltf`, where `bodyfile` looks for recipes.
pub fn path(root: &Path, car: u32) -> Option<PathBuf> {
    candidates(root, car).into_iter().find(|p| p.is_file())
}

/// Plain materials the contract itself names (`rubber`, the blueprint's `trim_*`): no warning for them.
pub fn expected_plain(name: &str) -> bool {
    let n = name.to_ascii_lowercase();
    n.starts_with("trim") || n.starts_with("rubber")
}

/// Which body a car shows in Remaster: the chain's first link that exists (`Imported`, the default), or
/// one link forced for an A/B comparison in motorview (`--car-source`, the ribbon's Car model). The game
/// never sets it. A global like `bodyfile::set_dir`, so the loaders keep their signatures [design choice].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum CarSource {
    /// the model file, else the recipe, else the original (the chain)
    #[default]
    Imported,
    /// the recipe, else the original (the model file is ignored)
    Generated,
    /// the original MYO model (file and recipe ignored)
    Original,
}

impl CarSource {
    pub const ALL: [CarSource; 3] = [CarSource::Imported, CarSource::Generated, CarSource::Original];

    pub fn name(self) -> &'static str {
        match self {
            CarSource::Imported => "Imported",
            CarSource::Generated => "Generated",
            CarSource::Original => "Original",
        }
    }

    /// `--car-source` value (case-insensitive)
    pub fn parse(s: &str) -> Option<CarSource> {
        CarSource::ALL.into_iter().find(|c| c.name().eq_ignore_ascii_case(s))
    }
}

static SOURCE: AtomicU8 = AtomicU8::new(0);

/// Force a link of the chain (`CarSource`); takes effect at the next scene load.
pub fn set_source(s: CarSource) {
    SOURCE.store(s as u8, Ordering::Relaxed);
}

pub fn source() -> CarSource {
    CarSource::ALL[(SOURCE.load(Ordering::Relaxed) as usize).min(2)]
}

/// What a material name asks of the paint system.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mat {
    /// CarColour1..3 (0..2) with that zone's finish
    Paint(usize),
    /// reflects like the MYO glass (`Vertex::env` 1)
    Glass,
    /// polished metal (`procgen::METAL_ENV`)
    Chrome,
    /// self-lit in its own colour (`procgen::LAMP_ENV`), a halo at each lamp (`lamps`)
    Lamp(Lamp),
    /// a brake caliper inside a wheel: turned back by the spin (`procgen::CALIPER_ENV`)
    Caliper,
    /// its own base colour (rubber and every other name)
    Plain,
}

/// The kind of a lamp material.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Lamp {
    /// `lamp_front`: head lamps (the car's front halos)
    Front,
    /// `lamp_rear`: tail lamps (the rear halos, which brighten when braking)
    Rear,
    /// `light_brake`: brake lights; Remaster brightens every rear halo with the brake input, so they
    /// are rear halos too
    Brake,
}

impl Mat {
    /// `paint2`, `Paint2.001`, `glass_tint` ...: the lower-cased name's prefix decides.
    pub fn from_name(name: Option<&str>) -> Mat {
        let n = name.unwrap_or("").to_ascii_lowercase();
        if let Some(k) = ["paint1", "paint2", "paint3"].iter().position(|p| n.starts_with(p)) {
            Mat::Paint(k)
        } else if n.starts_with("glass") {
            Mat::Glass
        } else if n.starts_with("chrome") {
            Mat::Chrome
        } else if n.starts_with("lamp_front") {
            Mat::Lamp(Lamp::Front)
        } else if n.starts_with("lamp_rear") {
            Mat::Lamp(Lamp::Rear)
        } else if n.starts_with("light_brake") {
            Mat::Lamp(Lamp::Brake)
        } else if n.starts_with("caliper") {
            Mat::Caliper
        } else {
            Mat::Plain
        }
    }
}

/// One corner, in view-space axes, model units (before the fit).
#[derive(Clone, Copy, Debug)]
pub struct MVert {
    pub pos: Vec3,
    pub normal: Vec3,
    pub uv: [f32; 2],
    /// base colour factor x COLOR_0, already in gamma space (sRGB, 0..1)
    pub color: [f32; 4],
}

#[derive(Clone, Copy, Debug)]
pub struct MTri {
    /// counter-clockwise seen from the front
    pub v: [MVert; 3],
    pub mat: Mat,
    /// index into `Model::images`
    pub image: Option<usize>,
}

/// A parsed model file.
#[derive(Default)]
pub struct Model {
    /// the body
    pub tris: Vec<MTri>,
    /// the meshes under each wheel node (FL, FR, BR, BL); empty: the generated wheel stays
    pub wheel_tris: [Vec<MTri>; 4],
    /// the `wheel_*` node origins, view-space axes, model units; FL, FR, BR, BL
    pub wheels: [Option<Vec3>; 4],
    /// base colour textures, RGBA, as stored (sRGB)
    pub images: Vec<Map>,
    /// things skipped on the way (non-triangle primitives, unreadable images)
    pub notes: Vec<String>,
    /// the names of the materials that no rule above knows (they keep their base colour), for the
    /// import check; an unnamed one is "(unnamed)"
    pub plain_materials: BTreeSet<String>,
}

/// glTF axes to view space (module doc).
fn to_view(p: Vec3) -> Vec3 {
    Vec3::new(-p.x, p.y, -p.z)
}

/// A linear colour channel in gamma space (the renderer works there, like the original).
pub fn linear_to_srgb(c: f32) -> f32 {
    let c = c.clamp(0.0, 1.0);
    if c <= 0.003_130_8 { c * 12.92 } else { 1.055 * c.powf(1.0 / 2.4) - 0.055 }
}

/// A gamma-space channel as glTF's linear one (`linear_to_srgb` reversed; `write_glb` callers).
pub fn srgb_to_linear(c: f32) -> f32 {
    let c = c.clamp(0.0, 1.0);
    if c <= 0.040_45 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
}

/// Parse the model at `path` (`.glb`, or `.gltf` with its buffers and images beside it).
pub fn load(path: &Path) -> Result<Model, String> {
    let (doc, buffers, images) = gltf::import(path).map_err(|e| e.to_string())?;
    from_doc(&doc, &buffers, &images)
}

/// `load` from bytes (a `.glb`, or a `.gltf` with everything embedded).
pub fn parse(bytes: &[u8]) -> Result<Model, String> {
    let (doc, buffers, images) = gltf::import_slice(bytes).map_err(|e| e.to_string())?;
    from_doc(&doc, &buffers, &images)
}

fn from_doc(doc: &gltf::Document, buffers: &[gltf::buffer::Data], images: &[gltf::image::Data]) -> Result<Model, String> {
    let scene = doc.default_scene().or_else(|| doc.scenes().next()).ok_or("no scene")?;
    let mut model = Model::default();
    let mut image_at: Vec<Option<Option<usize>>> = vec![None; images.len()];
    // (node, parent's matrix, the wheel it is part of)
    let mut walk = vec![];
    for n in scene.nodes() {
        walk.push((n, Mat4::IDENTITY, None));
    }
    while let Some((node, parent, mut wheel)) = walk.pop() {
        let world = parent * Mat4::from_cols_array_2d(&node.transform().matrix());
        let name = node.name().unwrap_or("").to_ascii_lowercase();
        // a reference for the modeller (the blueprint): not part of the car
        if name.starts_with("ref_") {
            continue;
        }
        if wheel.is_none() && name.starts_with("wheel_") {
            // the hub; the meshes under it are that wheel, not body
            match WHEEL_NODES.iter().position(|w| name.starts_with(&w.to_ascii_lowercase())) {
                Some(i) => {
                    model.wheels[i] = Some(to_view(world.transform_point3(Vec3::ZERO)));
                    wheel = Some(i);
                }
                None => {
                    model.notes.push(format!("node '{}' is not one of {}", node.name().unwrap_or(""), WHEEL_NODES.join(" ")));
                    continue;
                }
            }
        }
        if let Some(mesh) = node.mesh() {
            add_mesh(&mut model, wheel, &mesh, world, buffers, images, &mut image_at);
        }
        for c in node.children() {
            walk.push((c, world, wheel));
        }
    }
    if model.tris.is_empty() {
        return Err("no triangles outside the wheel nodes".into());
    }
    Ok(model)
}

/// `mesh`'s triangles into the body, or into wheel `wheel`'s.
fn add_mesh(model: &mut Model, wheel: Option<usize>, mesh: &gltf::Mesh, world: Mat4, buffers: &[gltf::buffer::Data], images: &[gltf::image::Data], image_at: &mut [Option<Option<usize>>]) {
    let nm = Mat3::from_mat4(world).inverse().transpose();
    // a mirrored node turns its triangles round
    let mirrored = Mat3::from_mat4(world).determinant() < 0.0;
    for prim in mesh.primitives() {
        if prim.mode() != gltf::mesh::Mode::Triangles {
            model.notes.push(format!("mesh '{}': a {:?} primitive skipped (triangles only)", mesh.name().unwrap_or(""), prim.mode()));
            continue;
        }
        let reader = prim.reader(|b| buffers.get(b.index()).map(|d| &d[..]));
        let Some(pos) = reader.read_positions() else { continue };
        let pos: Vec<Vec3> = pos.map(|p| world.transform_point3(Vec3::from(p))).collect();
        let material = prim.material();
        let pbr = material.pbr_metallic_roughness();
        let factor = pbr.base_color_factor();
        let tex = pbr.base_color_texture();
        let uvs: Vec<[f32; 2]> = reader.read_tex_coords(tex.as_ref().map_or(0, |t| t.tex_coord())).map_or_else(|| vec![[0.0; 2]; pos.len()], |t| t.into_f32().collect());
        let cols: Vec<[f32; 4]> = reader.read_colors(0).map_or_else(|| vec![[1.0; 4]; pos.len()], |c| c.into_rgba_f32().collect());
        let image = tex.and_then(|t| {
            let src = t.texture().source().index();
            *image_at.get_mut(src)?.get_or_insert_with(|| match images.get(src).map(rgba) {
                Some(Ok(m)) => {
                    model.images.push(m);
                    Some(model.images.len() - 1)
                }
                Some(Err(e)) => {
                    model.notes.push(format!("image {src}: {e}"));
                    None
                }
                None => None,
            })
        });
        let mut index: Vec<u32> = reader.read_indices().map_or_else(|| (0..pos.len() as u32).collect(), |i| i.into_u32().collect());
        index.truncate(index.len() / 3 * 3);
        if index.iter().any(|&i| i as usize >= pos.len()) {
            model.notes.push(format!("mesh '{}': an index out of range, primitive skipped", mesh.name().unwrap_or("")));
            continue;
        }
        if mirrored {
            index.as_chunks_mut::<3>().0.iter_mut().for_each(|t| t.swap(1, 2));
        }
        let normals: Vec<Vec3> = match reader.read_normals() {
            Some(n) => n.map(|n| (nm * Vec3::from(n)).normalize_or(Vec3::Y)).collect(),
            // none in the file: smooth ones, the face normals (area weighted) averaged per index
            None => {
                let mut acc = vec![Vec3::ZERO; pos.len()];
                for t in index.as_chunks::<3>().0 {
                    let [a, b, c] = t.map(|i| pos[i as usize]);
                    let n = (b - a).cross(c - a);
                    t.iter().for_each(|&i| acc[i as usize] += n);
                }
                acc.into_iter().map(|n| n.normalize_or(Vec3::Y)).collect()
            }
        };
        let mat = Mat::from_name(material.name());
        if mat == Mat::Plain {
            model.plain_materials.insert(material.name().unwrap_or("(unnamed)").to_string());
        }
        let corner = |i: u32| {
            let i = i as usize;
            let c = cols.get(i).copied().unwrap_or([1.0; 4]);
            let lin: [f32; 4] = std::array::from_fn(|k| c[k] * factor[k]);
            MVert {
                pos: to_view(pos[i]),
                normal: to_view(normals[i]),
                uv: uvs.get(i).copied().unwrap_or([0.0; 2]),
                color: [linear_to_srgb(lin[0]), linear_to_srgb(lin[1]), linear_to_srgb(lin[2]), lin[3]],
            }
        };
        let out = match wheel {
            Some(i) => &mut model.wheel_tris[i],
            None => &mut model.tris,
        };
        for t in index.as_chunks::<3>().0 {
            let v = t.map(corner);
            out.push(MTri { v, mat, image });
            if material.double_sided() {
                let back = |m: MVert| MVert { normal: -m.normal, ..m };
                out.push(MTri { v: [back(v[0]), back(v[2]), back(v[1])], mat, image });
            }
        }
    }
}

/// A decoded glTF image as RGBA8.
fn rgba(img: &gltf::image::Data) -> Result<Map, String> {
    use gltf::image::Format;
    let ch = match img.format {
        Format::R8 => 1,
        Format::R8G8 => 2,
        Format::R8G8B8 => 3,
        Format::R8G8B8A8 => 4,
        f => return Err(format!("{f:?} images are not read (8-bit only)")),
    };
    let rgba = img
        .pixels
        .chunks_exact(ch)
        .flat_map(|p| match ch {
            1 => [p[0], p[0], p[0], 255],
            2 => [p[0], p[0], p[0], p[1]],
            3 => [p[0], p[1], p[2], 255],
            _ => [p[0], p[1], p[2], p[3]],
        })
        .collect();
    Ok(Map { w: img.width, h: img.height, rgba })
}

/// How the model is placed: view = `scale` x model + `offset`.
#[derive(Clone, Debug, PartialEq)]
pub struct Fit {
    pub scale: f32,
    pub offset: Vec3,
    /// fitted on the wheel nodes: the largest hub miss (m); None for the bounding-box fit
    pub hub_err: Option<f32>,
    pub warnings: Vec<String>,
}

impl Fit {
    pub fn apply(&self, p: Vec3) -> Vec3 {
        p * self.scale + self.offset
    }
}

/// Place `model` on the car: its wheel nodes on `hubs` (view space; `Vec3::ZERO` = no such wheel)
/// when two or more pair up, else its bounding box on `body_box` (the MYO body's, view space).
pub fn fit(model: &Model, hubs: &[Vec3; 4], body_box: Option<(Vec3, Vec3)>) -> Result<Fit, String> {
    let pairs: Vec<(Vec3, Vec3)> = model.wheels.iter().zip(hubs).filter_map(|(w, h)| w.filter(|_| *h != Vec3::ZERO).map(|w| (w, *h))).collect();
    let mut warnings = Vec::new();
    if pairs.len() >= 2 {
        let n = pairs.len() as f32;
        let pc = pairs.iter().map(|p| p.0).sum::<Vec3>() / n;
        let hc = pairs.iter().map(|p| p.1).sum::<Vec3>() / n;
        let den: f32 = pairs.iter().map(|p| (p.0 - pc).length_squared()).sum();
        let num: f32 = pairs.iter().map(|p| (p.0 - pc).dot(p.1 - hc)).sum();
        if den > 1e-12 && num > 0.0 {
            let scale = num / den;
            let offset = hc - pc * scale;
            let f = |p: Vec3| p * scale + offset;
            let hub_err = pairs.iter().map(|p| (f(p.0) - p.1).length()).fold(0.0, f32::max);
            if let ([Some(fl), Some(fr), Some(br), Some(bl)], [hfl, hfr, hbr, hbl]) = (model.wheels, *hubs) {
                let checks = [
                    ("front track", (f(fl) - f(fr)).length(), (hfl - hfr).length()),
                    ("rear track", (f(bl) - f(br)).length(), (hbl - hbr).length()),
                    ("wheelbase", ((f(fl) + f(fr)) - (f(bl) + f(br))).length() / 2.0, ((hfl + hfr) - (hbl + hbr)).length() / 2.0),
                ];
                for (what, got, want) in checks {
                    if want > 0.0 && (got / want - 1.0).abs() > WHEEL_FIT_TOLERANCE {
                        warnings.push(format!("{what} {got:.3} m, the car's is {want:.3} m"));
                    }
                }
            }
            return Ok(Fit { scale, offset, hub_err: Some(hub_err), warnings });
        }
        warnings.push("the wheel nodes do not span the car the right way round: fitted to the body's box".into());
    } else if model.wheels.iter().any(Option::is_some) {
        warnings.push("fewer than two wheel nodes on the car's wheels: fitted to the body's box".into());
    }
    let (lo, hi) = body_box.ok_or("no wheel nodes and no original body to fit to")?;
    let (mlo, mhi) = model.tris.iter().flat_map(|t| t.v.iter().map(|v| v.pos)).fold((Vec3::MAX, Vec3::MIN), |(a, b), p| (a.min(p), b.max(p)));
    let len = mhi.z - mlo.z;
    if len <= 1e-6 {
        return Err("the model has no length".into());
    }
    let scale = (hi.z - lo.z) / len;
    let (c, mc) = ((lo + hi) / 2.0, (mlo + mhi) / 2.0);
    let offset = Vec3::new(c.x - mc.x * scale, lo.y - mlo.y * scale, c.z - mc.z * scale);
    Ok(Fit { scale, offset, hub_err: None, warnings })
}

/// The paint the `paint1..3` materials take.
#[derive(Clone, Copy, Debug)]
pub struct Paint {
    /// CarColour1..3 as vertex colours (`scene::rgba_obj`)
    pub colours: [[f32; 4]; 3],
    /// each zone's `Vertex::env` (1 gloss, `scene::METALLIC_ENV`, `scene::PEARL_ENV`)
    pub env: [f32; 3],
}

/// The fitted body as car-local view-space triangles. `scale`: ObjectBrightness (the MYO faces'
/// `rgba_obj`); `layer`: the texture array layer of `Model::images[i]` (None: untextured).
pub fn vertices(model: &Model, fit: &Fit, paint: &Paint, scale: [f32; 3], layer: &dyn Fn(usize) -> Option<f32>) -> Vec<Vertex> {
    emit(&model.tris, fit, Vec3::ZERO, paint, scale, layer)
}

/// `tris` fitted, less `centre` (view space, after the fit).
fn emit(tris: &[MTri], fit: &Fit, centre: Vec3, paint: &Paint, scale: [f32; 3], layer: &dyn Fn(usize) -> Option<f32>) -> Vec<Vertex> {
    let mut out = Vec::with_capacity(tris.len() * 3);
    for t in tris {
        let layer = t.image.and_then(layer).unwrap_or(-1.0);
        for v in &t.v {
            let own = [v.color[0] * scale[0], v.color[1] * scale[1], v.color[2] * scale[2], 1.0];
            let (color, env) = match t.mat {
                Mat::Paint(k) => (paint.colours[k], paint.env[k]),
                Mat::Glass => (own, 1.0),
                Mat::Chrome => (own, super::METAL_ENV),
                Mat::Lamp(_) => (own, super::LAMP_ENV),
                Mat::Caliper => (own, super::CALIPER_ENV),
                Mat::Plain => (own, 0.0),
            };
            let uv = if layer >= 0.0 { v.uv } else { [0.0; 2] };
            out.push(Vertex { pos: (fit.apply(v.pos) - centre).to_array(), uv, color, layer, normal: v.normal.to_array(), env });
        }
    }
    out
}

/// What the simulation has for one wheel, to check an imported one against.
#[derive(Clone, Copy, Debug)]
pub struct SimWheel {
    pub radius: f32,
    pub half_width: f32,
}

/// Wheel `i` (FL, FR, BR, BL) of the file, fitted and centred on its node's origin as the renderer
/// wants a wheel mesh (it adds the hub, the steering and the spin), with the checks against `sim`
/// (radius, half-width, origin off the tyre's centre). None when the file has no meshes under that
/// node: the generated wheel stays.
pub fn wheel(model: &Model, fit: &Fit, i: usize, sim: SimWheel, paint: &Paint, scale: [f32; 3], layer: &dyn Fn(usize) -> Option<f32>) -> Option<(Vec<Vertex>, Vec<String>)> {
    let (hub, tris) = (model.wheels[i]?, &model.wheel_tris[i]);
    if tris.is_empty() {
        return None;
    }
    let centre = fit.apply(hub);
    let verts = emit(tris, fit, centre, paint, scale, layer);
    let (lo, hi) = verts.iter().map(|v| Vec3::from(v.pos)).fold((Vec3::MAX, Vec3::MIN), |(a, b), p| (a.min(p), b.max(p)));
    let size = hi - lo;
    let (radius, half_width) = ((size.y + size.z) / 4.0, size.x / 2.0);
    let mut warnings = Vec::new();
    let name = WHEEL_NODES[i];
    for (what, got, want) in [("radius", radius, sim.radius), ("half-width", half_width, sim.half_width)] {
        if want > 0.0 && (got / want - 1.0).abs() > WHEEL_FIT_TOLERANCE {
            warnings.push(format!("{name} {what} {got:.3} m, the car's is {want:.3} m"));
        }
    }
    let off = Vec3::new(0.0, (lo.y + hi.y) / 2.0, (lo.z + hi.z) / 2.0).length();
    if off > WHEEL_CENTRE_TOLERANCE {
        warnings.push(format!("{name}'s origin is {:.0} mm off its tyre's centre: it wobbles", off * 1000.0));
    }
    Some((verts, warnings))
}

/// One lamp of the body: where its halo goes.
#[derive(Clone, Copy, Debug)]
pub struct LampSpot {
    pub kind: Lamp,
    /// the area-weighted centre of its faces (car-local view space, fitted)
    pub centre: Vec3,
    /// its faces' area (m²)
    pub area: f32,
}

/// The body's lamps: per lamp material kind, its connected parts (corners welded at 0.1 mm), merged
/// while two centres are closer than `LAMP_MERGE`. Front first, then rear and brake; within a kind
/// in file order.
pub fn lamps(model: &Model, fit: &Fit) -> Vec<LampSpot> {
    let mut out = Vec::new();
    for kind in [Lamp::Front, Lamp::Rear, Lamp::Brake] {
        let tris: Vec<[Vec3; 3]> = model.tris.iter().filter(|t| t.mat == Mat::Lamp(kind)).map(|t| t.v.map(|v| fit.apply(v.pos))).collect();
        if tris.is_empty() {
            continue;
        }
        // union-find over the triangles, joined where they share a welded corner
        let mut parent: Vec<usize> = (0..tris.len()).collect();
        fn root(p: &mut [usize], mut i: usize) -> usize {
            while p[i] != i {
                p[i] = p[p[i]];
                i = p[i];
            }
            i
        }
        let mut seen: std::collections::HashMap<[i64; 3], usize> = std::collections::HashMap::new();
        for (k, t) in tris.iter().enumerate() {
            for p in t {
                let key = (*p * 10_000.0).round().as_i64vec3().to_array();
                let other = *seen.entry(key).or_insert(k);
                let (a, b) = (root(&mut parent, k), root(&mut parent, other));
                parent[a] = b;
            }
        }
        let mut parts: Vec<(usize, Vec3, f32)> = Vec::new();
        for (k, t) in tris.iter().enumerate() {
            let r = root(&mut parent, k);
            let area = (t[1] - t[0]).cross(t[2] - t[0]).length() / 2.0;
            let c = (t[0] + t[1] + t[2]) / 3.0;
            match parts.iter_mut().find(|p| p.0 == r) {
                Some(p) => {
                    p.1 += c * area;
                    p.2 += area;
                }
                None => parts.push((r, c * area, area)),
            }
        }
        let mut spots: Vec<(Vec3, f32)> = parts.into_iter().filter(|p| p.2 > 0.0).map(|p| (p.1, p.2)).collect();
        // merge the closest pair while it is closer than LAMP_MERGE (sums of centre x area, area)
        loop {
            let centre = |s: &(Vec3, f32)| s.0 / s.1;
            let mut best: Option<(usize, usize, f32)> = None;
            for a in 0..spots.len() {
                for b in a + 1..spots.len() {
                    let d = centre(&spots[a]).distance(centre(&spots[b]));
                    if d < LAMP_MERGE && best.is_none_or(|x| d < x.2) {
                        best = Some((a, b, d));
                    }
                }
            }
            let Some((a, b, _)) = best else { break };
            let s = spots.swap_remove(b);
            spots[a].0 += s.0;
            spots[a].1 += s.1;
        }
        out.extend(spots.into_iter().map(|(sum, area)| LampSpot { kind, centre: sum / area, area }));
    }
    out
}

/// One part for `write_glb`: a triangle list in glTF axes (+Y up, front +Z, right -X), one material.
pub struct GlbPart {
    pub material: String,
    /// linear base colour factor
    pub color: [f32; 4],
    /// index into `write_glb`'s `images`
    pub image: Option<usize>,
    pub pos: Vec<[f32; 3]>,
    pub normal: Vec<[f32; 3]>,
    pub uv: Vec<[f32; 2]>,
}

/// A child node of `write_glb`'s root: a wheel's hub, with its meshes (positions relative to `at`).
pub struct GlbNode<'a> {
    pub name: &'a str,
    /// in the root's frame
    pub at: [f32; 3],
    /// empty: an empty node
    pub parts: Vec<GlbPart>,
}

/// A minimal `.glb`: a root node `car` (uniform `scale`, then `offset`) holding a node `body` with
/// one primitive per part and the `wheels` nodes, each with its own mesh when it has parts.
/// `images` are PNG files. The tests' and the `glbcar` example's writer.
pub fn write_glb(parts: &[GlbPart], images: &[Vec<u8>], wheels: &[GlbNode], scale: f32, offset: [f32; 3]) -> Vec<u8> {
    let mut bin: Vec<u8> = Vec::new();
    let (mut views, mut accessors, mut materials, mut meshes) = (vec![], vec![], vec![], vec![]);
    let mut view = |bin: &mut Vec<u8>, bytes: &[u8]| -> usize {
        let at = bin.len();
        bin.extend_from_slice(bytes);
        while !bin.len().is_multiple_of(4) {
            bin.push(0);
        }
        views.push(format!(r#"{{"buffer":0,"byteOffset":{at},"byteLength":{}}}"#, bytes.len()));
        views.len() - 1
    };
    let floats = |v: &[f32]| v.iter().flat_map(|f| f.to_le_bytes()).collect::<Vec<u8>>();
    let list = |v: &[f32]| v.iter().map(|f| format!("{f:?}")).collect::<Vec<_>>().join(",");
    let mut mesh = |name: &str, parts: &[GlbPart], bin: &mut Vec<u8>| -> Option<usize> {
        let mut prims = vec![];
        for p in parts.iter().filter(|p| !p.pos.is_empty()) {
            let n = p.pos.len();
            let (lo, hi) = p.pos.iter().fold(([f32::MAX; 3], [f32::MIN; 3]), |(a, b), q| (std::array::from_fn(|k| a[k].min(q[k])), std::array::from_fn(|k| b[k].max(q[k]))));
            let pv = view(bin, &floats(p.pos.as_flattened()));
            accessors.push(format!(r#"{{"bufferView":{pv},"componentType":5126,"count":{n},"type":"VEC3","min":[{}],"max":[{}]}}"#, list(&lo), list(&hi)));
            let mut attrs = format!(r#""POSITION":{}"#, accessors.len() - 1);
            if p.normal.len() == n {
                let nv = view(bin, &floats(p.normal.as_flattened()));
                accessors.push(format!(r#"{{"bufferView":{nv},"componentType":5126,"count":{n},"type":"VEC3"}}"#));
                attrs += &format!(r#","NORMAL":{}"#, accessors.len() - 1);
            }
            if p.uv.len() == n {
                let uv = view(bin, &floats(p.uv.as_flattened()));
                accessors.push(format!(r#"{{"bufferView":{uv},"componentType":5126,"count":{n},"type":"VEC2"}}"#));
                attrs += &format!(r#","TEXCOORD_0":{}"#, accessors.len() - 1);
            }
            let tex = p.image.map_or(String::new(), |i| format!(r#","baseColorTexture":{{"index":{i}}}"#));
            let name = p.material.replace(['"', '\\'], "_");
            materials.push(format!(r#"{{"name":"{name}","pbrMetallicRoughness":{{"baseColorFactor":[{}]{tex}}}}}"#, list(&p.color)));
            prims.push(format!(r#"{{"attributes":{{{attrs}}},"material":{}}}"#, materials.len() - 1));
        }
        if prims.is_empty() {
            return None;
        }
        meshes.push(format!(r#"{{"name":"{name}","primitives":[{}]}}"#, prims.join(",")));
        Some(meshes.len() - 1)
    };
    let mut nodes = vec![String::new()];
    let body = mesh("body", parts, &mut bin).map_or(String::new(), |m| format!(r#","mesh":{m}"#));
    nodes.push(format!(r#"{{"name":"body"{body}}}"#));
    for w in wheels {
        let m = mesh(w.name, &w.parts, &mut bin).map_or(String::new(), |m| format!(r#","mesh":{m}"#));
        nodes.push(format!(r#"{{"name":"{}","translation":[{}]{m}}}"#, w.name, list(&w.at)));
    }
    let image_views: Vec<usize> = images.iter().map(|png| view(&mut bin, png)).collect();
    let children = (1..nodes.len()).map(|i| i.to_string()).collect::<Vec<_>>().join(",");
    let s = format!("{scale:?}");
    nodes[0] = format!(r#"{{"name":"car","scale":[{s},{s},{s}],"translation":[{}],"children":[{children}]}}"#, list(&offset));
    let images_json = image_views.iter().map(|v| format!(r#"{{"bufferView":{v},"mimeType":"image/png"}}"#)).collect::<Vec<_>>().join(",");
    let textures_json = (0..images.len()).map(|i| format!(r#"{{"source":{i}}}"#)).collect::<Vec<_>>().join(",");
    let mut json = format!(
        r#"{{"asset":{{"version":"2.0","generator":"motorhead carmodel"}},"scene":0,"scenes":[{{"nodes":[0]}}],"nodes":[{}],"meshes":[{}],"materials":[{}],"accessors":[{}],"bufferViews":[{}],"buffers":[{{"byteLength":{}}}]{}}}"#,
        nodes.join(","),
        meshes.join(","),
        materials.join(","),
        accessors.join(","),
        views.join(","),
        bin.len(),
        if images.is_empty() { String::new() } else { format!(r#","images":[{images_json}],"textures":[{textures_json}]"#) },
    )
    .into_bytes();
    while !json.len().is_multiple_of(4) {
        json.push(b' ');
    }
    let total = 12 + 8 + json.len() + 8 + bin.len();
    let mut out = Vec::with_capacity(total);
    out.extend_from_slice(b"glTF");
    out.extend_from_slice(&2u32.to_le_bytes());
    out.extend_from_slice(&(total as u32).to_le_bytes());
    out.extend_from_slice(&(json.len() as u32).to_le_bytes());
    out.extend_from_slice(b"JSON");
    out.extend_from_slice(&json);
    out.extend_from_slice(&(bin.len() as u32).to_le_bytes());
    out.extend_from_slice(b"BIN\0");
    out.extend_from_slice(&bin);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A 4 m x 1.8 m x 1.2 m box in glTF axes (front +Z), CCW outward, one material per face pair.
    fn box_parts(materials: [&str; 6]) -> Vec<GlbPart> {
        box_at(materials, Vec3::new(0.9, 0.6, 2.0), Vec3::ZERO)
    }

    /// A box of half-size `half` centred on `at` (glTF axes), CCW outward, one material per face pair.
    fn box_at(materials: [&str; 6], half: Vec3, at: Vec3) -> Vec<GlbPart> {
        // +X (the car's left), -X (right), +Y, -Y, +Z (front), -Z (back)
        let faces: [([f32; 3], [f32; 3], [f32; 3]); 6] = [
            ([1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, -1.0]),
            ([-1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]),
            ([0.0, 1.0, 0.0], [0.0, 0.0, -1.0], [1.0, 0.0, 0.0]),
            ([0.0, -1.0, 0.0], [0.0, 0.0, 1.0], [1.0, 0.0, 0.0]),
            ([0.0, 0.0, 1.0], [0.0, 1.0, 0.0], [1.0, 0.0, 0.0]),
            ([0.0, 0.0, -1.0], [0.0, 1.0, 0.0], [-1.0, 0.0, 0.0]),
        ];
        faces
            .iter()
            .zip(materials)
            .map(|((n, u, v), m)| {
                let (n, u, v) = (Vec3::from(*n), Vec3::from(*u), Vec3::from(*v));
                let c = n * half + at;
                let (u, v) = (u * half, v * half);
                // v x u = n: the quad below is CCW seen from outside (the axes test checks it)
                let q = [c - u - v, c - u + v, c + u + v, c + u - v];
                let tri = [q[0], q[1], q[2], q[0], q[2], q[3]];
                GlbPart { material: m.into(), color: [0.5, 0.5, 0.5, 1.0], image: None, pos: tri.iter().map(|p| p.to_array()).collect(), normal: vec![n.to_array(); 6], uv: Vec::new() }
            })
            .collect()
    }

    /// glTF wheel positions of the box (front +Z, left +X), at the hub height 0.
    fn gl_wheels() -> Vec<GlbNode<'static>> {
        [("wheel_FL", [0.8, -0.3, 1.3]), ("wheel_FR", [-0.8, -0.3, 1.3]), ("wheel_BR", [-0.8, -0.3, -1.3]), ("wheel_BL", [0.8, -0.3, -1.3])].into_iter().map(|(name, at)| GlbNode { name, at, parts: Vec::new() }).collect()
    }

    /// The hubs the model's wheels should land on (view space: right +x, front -z), raised 0.2 m.
    fn hubs() -> [Vec3; 4] {
        gl_wheels().iter().map(|w| to_view(Vec3::from(w.at)) + Vec3::Y * 0.2).collect::<Vec<_>>().try_into().unwrap()
    }

    /// A wheel 0.3 m in radius, 0.12 m in half-width, as a box round its node's origin (+ `off`).
    fn tyre(off: Vec3) -> Vec<GlbPart> {
        box_at(["rubber", "rubber", "rubber", "rubber", "caliper", "rubber"], Vec3::new(0.12, 0.3, 0.3), off)
    }

    fn sim() -> SimWheel {
        SimWheel { radius: 0.3, half_width: 0.12 }
    }

    fn paint() -> Paint {
        Paint { colours: [[1.0, 0.0, 0.0, 1.0], [0.0, 1.0, 0.0, 1.0], [0.0, 0.0, 1.0, 1.0]], env: [1.0; 3] }
    }

    #[test]
    fn axes_put_the_nose_at_minus_z_and_the_right_at_plus_x() {
        let m = parse(&write_glb(&box_parts(["l", "r", "top", "bottom", "front", "back"]), &[], &gl_wheels(), 1.0, [0.0; 3])).unwrap();
        let centre = |name: &str| {
            let k = ["l", "r", "top", "bottom", "front", "back"].iter().position(|n| *n == name).unwrap();
            let t = &m.tris[k * 2];
            (t.v[0].pos + t.v[1].pos + t.v[2].pos) / 3.0
        };
        assert!(centre("front").z < -1.0 && centre("back").z > 1.0);
        assert!(centre("r").x > 0.5 && centre("l").x < -0.5);
        // FL is front left: view -z, -x
        let fl = m.wheels[0].unwrap();
        assert!(fl.z < 0.0 && fl.x < 0.0, "{fl}");
        // the winding: every triangle CCW seen from outside, its geometric normal along its vertex normal
        for t in &m.tris {
            let n = (t.v[1].pos - t.v[0].pos).cross(t.v[2].pos - t.v[0].pos);
            assert!(n.dot(t.v[0].normal) > 0.0);
        }
    }

    #[test]
    fn wheel_fit_recovers_scale_and_offset_within_a_millimetre() {
        // modelled in centimetres, moved off the origin: the fit must undo both
        let m = parse(&write_glb(&box_parts(["paint1"; 6]), &[], &gl_wheels(), 100.0, [12.0, -40.0, 75.0])).unwrap();
        let f = fit(&m, &hubs(), None).unwrap();
        assert!(f.hub_err.unwrap() < 1e-3, "{f:?}");
        assert!((f.scale - 0.01).abs() < 1e-6 && f.warnings.is_empty(), "{f:?}");
        // the hubs are 0.2 m higher than in the model: so is the body's top (0.6)
        let top = m.tris.iter().flat_map(|t| t.v.iter()).map(|v| f.apply(v.pos).y).fold(f32::MIN, f32::max);
        assert!((top - 0.8).abs() < 1e-3, "{top}");
    }

    #[test]
    fn wheels_off_the_car_warn() {
        let mut w = gl_wheels();
        w.iter_mut().for_each(|w| w.at[2] *= 1.2);
        let m = parse(&write_glb(&box_parts(["paint1"; 6]), &[], &w, 1.0, [0.0; 3])).unwrap();
        let f = fit(&m, &hubs(), None).unwrap();
        // the scale splits the difference: the track comes out narrow
        assert!(f.warnings.iter().any(|w| w.contains("track")), "{f:?}");
    }

    #[test]
    fn without_wheels_the_box_fits_the_body() {
        let m = parse(&write_glb(&box_parts(["paint1"; 6]), &[], &[], 3.0, [5.0; 3])).unwrap();
        assert!(m.wheels.iter().all(Option::is_none));
        let body = (Vec3::new(-1.0, 0.1, -2.5), Vec3::new(1.0, 1.3, 2.5));
        let f = fit(&m, &[Vec3::ZERO; 4], Some(body)).unwrap();
        let (lo, hi) = m.tris.iter().flat_map(|t| t.v.iter()).map(|v| f.apply(v.pos)).fold((Vec3::MAX, Vec3::MIN), |(a, b), p| (a.min(p), b.max(p)));
        assert!((lo.z - body.0.z).abs() < 1e-4 && (hi.z - body.1.z).abs() < 1e-4 && (lo.y - body.0.y).abs() < 1e-4, "{lo} {hi}");
        assert!(fit(&m, &[Vec3::ZERO; 4], None).is_err());
    }

    #[test]
    fn materials_take_the_livery_and_finish() {
        let m = parse(&write_glb(&box_parts(["Paint2.001", "paint1", "glass", "chrome", "rubber", "paint3"]), &[], &gl_wheels(), 1.0, [0.0; 3])).unwrap();
        let paint = Paint { colours: [[1.0, 0.0, 0.0, 1.0], [0.0, 1.0, 0.0, 1.0], [0.0, 0.0, 1.0, 1.0]], env: [1.0, crate::scene::METALLIC_ENV, crate::scene::PEARL_ENV] };
        let f = fit(&m, &hubs(), None).unwrap();
        let v = vertices(&m, &f, &paint, [1.0; 3], &|_| None);
        // two triangles (6 vertices) per box face, in the parts' order
        let face = |k: usize| v[k * 6];
        assert_eq!((face(0).color, face(0).env), ([0.0, 1.0, 0.0, 1.0], crate::scene::METALLIC_ENV));
        assert_eq!((face(1).color, face(1).env), ([1.0, 0.0, 0.0, 1.0], 1.0));
        assert_eq!(face(2).env, 1.0);
        assert_eq!(face(3).env, super::super::METAL_ENV);
        assert_eq!(face(4).env, 0.0);
        assert_eq!(face(5).env, crate::scene::PEARL_ENV);
        // linear 0.5 grey comes out in gamma space
        assert!((face(4).color[0] - linear_to_srgb(0.5)).abs() < 1e-6 && (face(4).color[0] - 0.7354).abs() < 1e-3);
        assert!(v.iter().all(|v| v.layer == -1.0));
    }

    #[test]
    fn srgb_round_trips() {
        for c in [0.0, 0.002, 0.2, 0.5, 1.0] {
            assert!((srgb_to_linear(linear_to_srgb(c)) - c).abs() < 1e-5);
        }
    }

    #[test]
    fn textures_load() {
        let mut parts = box_parts(["a"; 6]);
        parts[0].image = Some(0);
        parts[0].uv = vec![[0.0, 0.0]; 6];
        let mut png = Vec::new();
        let mut e = ::png::Encoder::new(&mut png, 2, 2);
        e.set_color(::png::ColorType::Rgb);
        e.set_depth(::png::BitDepth::Eight);
        e.write_header().unwrap().write_image_data(&[255, 0, 0, 0, 255, 0, 0, 0, 255, 9, 9, 9]).unwrap();
        let bytes = write_glb(&parts, &[png], &[], 1.0, [0.0; 3]);
        let m = parse(&bytes).unwrap();
        assert_eq!(m.images.len(), 1);
        assert_eq!(&m.images[0].rgba[..8], &[255, 0, 0, 255, 0, 255, 0, 255]);
        assert_eq!(m.tris[0].image, Some(0));
        assert!(m.tris[2].image.is_none());
        assert_eq!(m.tris.len(), 12);
    }

    #[test]
    fn material_names() {
        assert_eq!(Mat::from_name(Some("PAINT3")), Mat::Paint(2));
        assert_eq!(Mat::from_name(Some("glass.002")), Mat::Glass);
        assert_eq!(Mat::from_name(Some("tyre")), Mat::Plain);
        assert_eq!(Mat::from_name(None), Mat::Plain);
        assert_eq!(Mat::from_name(Some("lamp_front.001")), Mat::Lamp(Lamp::Front));
        assert_eq!(Mat::from_name(Some("Lamp_Rear")), Mat::Lamp(Lamp::Rear));
        assert_eq!(Mat::from_name(Some("LIGHT_BRAKE")), Mat::Lamp(Lamp::Brake));
        assert_eq!(Mat::from_name(Some("caliper_red")), Mat::Caliper);
    }

    #[test]
    fn ref_nodes_are_skipped_and_plain_materials_listed() {
        let mut nodes = gl_wheels();
        nodes.push(GlbNode { name: "ref_original", at: [0.0, 0.0, 0.0], parts: box_at(["paint1", "paint1", "paint1", "paint1", "paint1", "paint1"], Vec3::new(5.0, 5.0, 5.0), Vec3::ZERO) });
        let body = box_parts(["paint1", "Tyre", "glass", "paint2", "Chrome.001", "trim_a"]);
        let m = parse(&write_glb(&body, &[], &nodes, 1.0, [0.0; 3])).unwrap();
        // the 10 m reference box is not in the body: 6 faces x 2 triangles
        assert_eq!(m.tris.len(), 12);
        assert_eq!(m.plain_materials.iter().map(String::as_str).collect::<Vec<_>>(), ["Tyre", "trim_a"]);
    }

    #[test]
    fn car_sources_parse() {
        assert_eq!(CarSource::parse("generated"), Some(CarSource::Generated));
        assert_eq!(CarSource::parse("ORIGINAL"), Some(CarSource::Original));
        assert_eq!(CarSource::parse("imported"), Some(CarSource::Imported));
        assert_eq!(CarSource::parse("mine"), None);
        assert_eq!(CarSource::default(), CarSource::Imported);
    }

    #[test]
    fn wheel_meshes_are_wheels_centred_on_their_hub() {
        // in centimetres and off the origin, as the fit test; FL and BR have meshes, FR and BL are empty
        let mut w = gl_wheels();
        w[0].parts = tyre(Vec3::ZERO);
        w[2].parts = tyre(Vec3::ZERO);
        let m = parse(&write_glb(&box_parts(["paint1"; 6]), &[], &w, 100.0, [12.0, -40.0, 75.0])).unwrap();
        assert_eq!(m.tris.len(), 12, "the wheels are not body");
        assert_eq!((m.wheel_tris[0].len(), m.wheel_tris[1].len(), m.wheel_tris[2].len()), (12, 0, 12));
        let f = fit(&m, &hubs(), None).unwrap();
        let (v, warn) = wheel(&m, &f, 0, sim(), &paint(), [1.0; 3], &|_| None).unwrap();
        assert!(warn.is_empty(), "{warn:?}");
        let (lo, hi) = v.iter().map(|v| Vec3::from(v.pos)).fold((Vec3::MAX, Vec3::MIN), |(a, b), p| (a.min(p), b.max(p)));
        assert!(((lo + hi) / 2.0).length() < 1e-3 && (hi.y - 0.3).abs() < 1e-3, "{lo} {hi}");
        // the caliper side is turned back by the spin; the tyre is plain
        assert!(v.iter().any(|v| v.env == super::super::CALIPER_ENV) && v.iter().any(|v| v.env == 0.0));
        assert!(wheel(&m, &f, 1, sim(), &paint(), [1.0; 3], &|_| None).is_none(), "an empty node keeps the generated wheel");
    }

    #[test]
    fn wrong_wheels_warn() {
        let mut w = gl_wheels();
        // 20 % too big, and its origin 2 cm above the tyre's centre
        w[0].parts = box_at(["rubber"; 6], Vec3::new(0.12, 0.36, 0.36), Vec3::ZERO);
        w[1].parts = tyre(Vec3::new(0.0, -0.02, 0.0));
        let m = parse(&write_glb(&box_parts(["paint1"; 6]), &[], &w, 1.0, [0.0; 3])).unwrap();
        let f = fit(&m, &hubs(), None).unwrap();
        let big = wheel(&m, &f, 0, sim(), &paint(), [1.0; 3], &|_| None).unwrap().1;
        assert!(big.iter().any(|w| w.contains("wheel_FL radius")) && !big.iter().any(|w| w.contains("half-width")), "{big:?}");
        let off = wheel(&m, &f, 1, sim(), &paint(), [1.0; 3], &|_| None).unwrap().1;
        assert!(off.iter().any(|w| w.contains("20 mm off")), "{off:?}");
    }

    #[test]
    fn lamps_give_one_spot_per_lamp() {
        let mut parts = box_parts(["paint1"; 6]);
        let lens = |at: [f32; 3], m: &str| box_at([m; 6], Vec3::new(0.1, 0.04, 0.02), Vec3::from(at));
        // two head lamps (glTF: front +Z, left +X), one of them in two touching halves
        parts.extend(lens([0.6, 0.2, 2.0], "lamp_front"));
        parts.extend(lens([-0.5, 0.2, 2.0], "lamp_front"));
        parts.extend(lens([-0.7, 0.2, 2.0], "lamp_front.001"));
        // a tail lamp in two pieces 4 cm apart, centres 12 cm (merged), and a brake light in the middle
        let piece = |x: f32| box_at(["lamp_rear"; 6], Vec3::new(0.04, 0.04, 0.02), Vec3::new(x, 0.3, -2.0));
        parts.extend(piece(0.6));
        parts.extend(piece(0.72));
        parts.extend(lens([0.0, 0.5, -2.0], "light_brake"));
        let m = parse(&write_glb(&parts, &[], &gl_wheels(), 1.0, [0.0; 3])).unwrap();
        let f = fit(&m, &hubs(), None).unwrap();
        let spots = lamps(&m, &f);
        let kinds: Vec<Lamp> = spots.iter().map(|s| s.kind).collect();
        assert_eq!(kinds, [Lamp::Front, Lamp::Front, Lamp::Rear, Lamp::Brake], "{spots:?}");
        // view space: front -z, the glTF left (+X) is view -x; the hubs raised the car 0.2 m
        let mut xs: Vec<f32> = spots[..2].iter().map(|s| s.centre.x).collect();
        xs.sort_by(f32::total_cmp);
        assert!((xs[0] + 0.6).abs() < 1e-3 && (xs[1] - 0.6).abs() < 1e-3, "{xs:?}");
        assert!(spots[..2].iter().all(|s| (s.centre.z + 2.0).abs() < 1e-3 && (s.centre.y - 0.4).abs() < 1e-3));
        assert!((spots[2].centre.x + 0.66).abs() < 1e-3 && spots[2].centre.z > 1.9, "{:?}", spots[2]);
        // the lens faces are self-lit
        let v = vertices(&m, &f, &paint(), [1.0; 3], &|_| None);
        assert_eq!(v.iter().filter(|v| v.env == super::super::LAMP_ENV).count(), 6 * 36);
    }
}
