//! REMASTER tree models (#389, re-notes/12-world-fill 12.12): the original's tree and bush cards are
//! drawn as 3D models from our tree pack (`assets/trees/tree-pack.gltf`, embedded).
//!
//! [O] (MYW scan, `re-notes/tools/plant_tex.py`): the game has no vegetation models. Every tree is
//! world geometry: one flat or crossed card (2-8 polygons) with one colour-keyed picture, sharing no
//! vertex with anything else. So a card is a connected group of world triangles with a tree texture;
//! the Remaster world draw leaves it out (`Swap::hide`) and a pack model stands where it stood.
//! Pines and cypresses (`tree7`, `_tree1`) keep their cards: the pack has no conifer (design choice).
//! Classic is untouched: it draws `Scene::tris` as before.

use std::collections::HashMap;
use std::sync::OnceLock;

use glam::{Mat3, Mat4, Vec3};

use crate::scene::{Look, Vertex, TEX};

/// The tree pack (glTF 2.0, our own assets): one node per model, named `Tree-01-1` and so on.
static PACK_GLTF: &[u8] = include_bytes!("../../../../assets/trees/tree-pack.gltf");

/// Card texture (lower case, no extension) -> the pack models that replace it, one picked per tree
/// by its position. A family per picture keeps one picture's trees alike (design choice).
const SWAP: &[(&str, &[&str])] = &[
    ("tree3", &["Tree-01-1", "Tree-01-2", "Tree-01-3", "Tree-01-4"]),
    ("fetree", &["Tree-01-1", "Tree-01-2", "Tree-01-3", "Tree-01-4"]),
    ("tree5", &["Tree-02-1", "Tree-02-2", "Tree-02-3", "Tree-02-4"]),
    ("tree2", &["Tree-02-1", "Tree-02-2", "Tree-02-3", "Tree-02-4"]),
    ("tree6", &["Tree-03-1", "Tree-03-2", "Tree-03-3", "Tree-03-4"]),
    ("_ettree1", &["Tree-03-3", "Tree-03-4"]),
    ("_ettree2", &["Tree-03-1", "Tree-03-2", "Tree-03-3", "Tree-03-4"]),
    ("_ettree3", &["Tree-03-1", "Tree-03-2", "Tree-03-3", "Tree-03-4"]),
    ("buske", &["Bush-01", "Bush-02", "Bush-03", "Bush-04", "Bush-05"]),
];

/// The models of a card texture (any case, with or without extension), or None for a texture that
/// is not swapped.
pub fn models_for(texture: &str) -> Option<&'static [&'static str]> {
    let k = texture.split('.').next().unwrap_or("").to_ascii_lowercase();
    SWAP.iter().find(|(t, _)| *t == k).map(|(_, m)| *m)
}

/// One original tree: where it stands and how big it is (view space).
#[derive(Clone, Debug, PartialEq)]
pub struct Site {
    /// the card's texture (as named in the world)
    pub texture: String,
    /// the foot of the card: its footprint centre at its lowest point
    pub base: Vec3,
    pub height: f32,
    /// the card's widest horizontal extent
    pub width: f32,
    /// the card's mean vertex colour: the original's baked light at the tree
    pub tint: [f32; 3],
    /// the card's texture layer (`Scene::layers`), if textured
    pub layer: Option<usize>,
}

/// The trees of a world: the sites and, per world triangle, whether it is a card that a model replaces.
#[derive(Default)]
pub struct Swap {
    pub sites: Vec<Site>,
    pub hide: Vec<bool>,
}

/// Cards whose footprint centres are closer than this fraction of the taller one's height are one
/// tree (crossed cards that do not share a vertex).
const MERGE: f32 = 0.25;
/// Cards lower than this (m) are not trees (a stray triangle).
const MIN_HEIGHT: f32 = 0.5;

/// Finds the swapped cards in a world: `tris` (view space, 3 per triangle), the MYW objects' triangle
/// ranges, and per triangle its index into `textures` (u32::MAX = untextured).
pub fn find(tris: &[Vertex], objects: &[(std::ops::Range<usize>, usize)], tri_texture: &[u32], textures: &[String]) -> Swap {
    let swapped: Vec<bool> = textures.iter().map(|t| models_for(t).is_some()).collect();
    let is_card = |t: usize| tri_texture.get(t).and_then(|&k| swapped.get(k as usize)).copied().unwrap_or(false);
    let mut hide = vec![false; tris.len() / 3];
    let mut sites: Vec<(Site, Vec<usize>)> = Vec::new();
    for (range, _) in objects {
        let cards: Vec<usize> = range.clone().filter(|&t| is_card(t)).collect();
        if cards.is_empty() {
            continue;
        }
        // connected groups: triangles that share a vertex position (same bits) and a texture
        let mut parent: Vec<usize> = (0..cards.len()).collect();
        fn root(p: &mut [usize], mut a: usize) -> usize {
            while p[a] != a {
                p[a] = p[p[a]];
                a = p[a];
            }
            a
        }
        let mut by_vertex: HashMap<([u32; 3], u32), usize> = HashMap::new();
        for (i, &t) in cards.iter().enumerate() {
            for v in &tris[t * 3..t * 3 + 3] {
                let key = (v.pos.map(f32::to_bits), tri_texture[t]);
                match by_vertex.get(&key) {
                    Some(&j) => {
                        let (a, b) = (root(&mut parent, i), root(&mut parent, j));
                        parent[a] = b;
                    }
                    None => {
                        by_vertex.insert(key, i);
                    }
                }
            }
        }
        let mut groups: HashMap<usize, Vec<usize>> = HashMap::new();
        for (i, &t) in cards.iter().enumerate() {
            groups.entry(root(&mut parent, i)).or_default().push(t);
        }
        let mut keys: Vec<usize> = groups.keys().copied().collect();
        keys.sort_unstable();
        for k in keys {
            let group = &groups[&k];
            let site = site_of(tris, group, &textures[tri_texture[group[0]] as usize]);
            sites.push((site, group.clone()));
        }
    }
    // crossed cards that share no vertex: one tree
    let mut merged: Vec<(Site, Vec<usize>)> = Vec::new();
    for (s, g) in sites {
        let near = merged.iter_mut().find(|(m, _)| m.texture == s.texture && {
            let d = Vec3::new(m.base.x - s.base.x, 0.0, m.base.z - s.base.z).length();
            d < MERGE * m.height.max(s.height)
        });
        match near {
            Some((m, mg)) => {
                mg.extend(g);
                *m = site_of(tris, mg, &m.texture);
            }
            None => merged.push((s, g)),
        }
    }
    let mut out = Vec::new();
    for (s, g) in merged {
        if s.height < MIN_HEIGHT {
            continue;
        }
        for t in g {
            hide[t] = true;
        }
        out.push(s);
    }
    Swap { sites: out, hide }
}

fn site_of(tris: &[Vertex], group: &[usize], texture: &str) -> Site {
    let (mut lo, mut hi) = (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN));
    let mut tint = [0.0f32; 3];
    let mut n = 0.0f32;
    for &t in group {
        for v in &tris[t * 3..t * 3 + 3] {
            let p = Vec3::from(v.pos);
            (lo, hi) = (lo.min(p), hi.max(p));
            for (c, &v) in tint.iter_mut().zip(&v.color) {
                *c += v;
            }
            n += 1.0;
        }
    }
    Site {
        texture: texture.to_string(),
        base: Vec3::new((lo.x + hi.x) * 0.5, lo.y, (lo.z + hi.z) * 0.5),
        height: hi.y - lo.y,
        width: (hi.x - lo.x).max(hi.z - lo.z),
        tint: tint.map(|c| c / n.max(1.0)),
        layer: group.first().map(|&t| tris[t * 3].layer).filter(|&l| l >= 0.0).map(|l| l as usize),
    }
}

/// One model of the pack: triangles (3 per triangle) in model space, y up, base at y = 0.
struct Model {
    verts: Vec<MVert>,
    height: f32,
    /// its widest horizontal extent
    width: f32,
    /// the mean colour of its solid texels times its vertex colours (what `mesh` matches to the card)
    albedo: [f32; 3],
}

#[derive(Clone, Copy)]
struct MVert {
    pos: Vec3,
    normal: Vec3,
    uv: [f32; 2],
    /// vertex colour times the material's base colour factor
    color: [f32; 4],
    /// index into `Pack::images`, or None (untextured)
    image: Option<usize>,
}

/// The parsed pack: models by node name and the images they use, resampled to `TEX` x `TEX` RGBA.
struct Pack {
    models: HashMap<String, Model>,
    images: Vec<Vec<u8>>,
}

/// The pack, parsed once per process (a Remaster world load). None when it does not parse.
fn pack() -> Option<&'static Pack> {
    static PACK: OnceLock<Option<Pack>> = OnceLock::new();
    PACK.get_or_init(|| match parse(PACK_GLTF) {
        Ok(p) => Some(p),
        Err(e) => {
            eprintln!("trees: the tree pack does not load: {e}");
            None
        }
    })
    .as_ref()
}

fn parse(bytes: &[u8]) -> Result<Pack, String> {
    let (doc, buffers, images) = gltf::import_slice(bytes).map_err(|e| e.to_string())?;
    let mut models = HashMap::new();
    let mut image_index: HashMap<usize, usize> = HashMap::new();
    let mut out_images = Vec::new();
    for node in doc.nodes() {
        let (Some(name), Some(mesh)) = (node.name(), node.mesh()) else { continue };
        // the node's rotation and scale, not its place in the pack's layout
        let (_, r, s) = node.transform().decomposed();
        let m = Mat4::from_scale_rotation_translation(Vec3::from(s), glam::Quat::from_array(r), Vec3::ZERO);
        let nm = Mat3::from_mat4(m).inverse().transpose();
        let mut verts = Vec::new();
        for prim in mesh.primitives() {
            if prim.mode() != gltf::mesh::Mode::Triangles {
                continue;
            }
            let reader = prim.reader(|b| buffers.get(b.index()).map(|d| &d[..]));
            let Some(pos) = reader.read_positions() else { continue };
            let pos: Vec<Vec3> = pos.map(|p| m.transform_point3(Vec3::from(p))).collect();
            let normals: Vec<Vec3> = reader.read_normals().map_or_else(|| vec![Vec3::Y; pos.len()], |n| n.map(|n| (nm * Vec3::from(n)).normalize_or(Vec3::Y)).collect());
            let pbr = prim.material().pbr_metallic_roughness();
            let factor = pbr.base_color_factor();
            let tex = pbr.base_color_texture();
            let uvs: Vec<[f32; 2]> = reader.read_tex_coords(tex.as_ref().map_or(0, |t| t.tex_coord())).map_or_else(|| vec![[0.0; 2]; pos.len()], |t| t.into_f32().collect());
            let cols: Vec<[f32; 4]> = reader.read_colors(0).map_or_else(|| vec![[1.0; 4]; pos.len()], |c| c.into_rgba_f32().collect());
            let image = match &tex {
                Some(t) => {
                    let src = t.texture().source().index();
                    match image_index.get(&src) {
                        Some(&i) => Some(i),
                        None => match images.get(src).map(resample) {
                            Some(Some(px)) => {
                                out_images.push(px);
                                image_index.insert(src, out_images.len() - 1);
                                Some(out_images.len() - 1)
                            }
                            _ => None,
                        },
                    }
                }
                None => None,
            };
            let index: Vec<u32> = reader.read_indices().map_or_else(|| (0..pos.len() as u32).collect(), |i| i.into_u32().collect());
            for &i in &index {
                let i = i as usize;
                let (Some(&p), Some(&n), Some(&uv), Some(&c)) = (pos.get(i), normals.get(i), uvs.get(i), cols.get(i)) else { continue };
                verts.push(MVert { pos: p, normal: n, uv, color: [0, 1, 2, 3].map(|k| c[k] * factor[k]), image });
            }
        }
        if verts.is_empty() {
            continue;
        }
        let lo = verts.iter().map(|v| v.pos.y).fold(f32::MAX, f32::min);
        let hi = verts.iter().map(|v| v.pos.y).fold(f32::MIN, f32::max);
        for v in &mut verts {
            v.pos.y -= lo;
        }
        let mut albedo = [0.0f32; 3];
        for v in &verts {
            let t = v.image.and_then(|i| out_images.get(i)).map_or([1.0; 3], |px| solid_mean(px));
            for k in 0..3 {
                albedo[k] += t[k] * v.color[k] / verts.len() as f32;
            }
        }
        let span = |f: fn(&MVert) -> f32| verts.iter().map(f).fold(f32::MIN, f32::max) - verts.iter().map(f).fold(f32::MAX, f32::min);
        let width = span(|v| v.pos.x).max(span(|v| v.pos.z)).max(1e-3);
        models.insert(name.to_string(), Model { verts, height: (hi - lo).max(1e-3), width, albedo });
    }
    if models.is_empty() {
        return Err("no tree models in the pack".into());
    }
    Ok(Pack { models, images: out_images })
}

/// The mean colour (0..1) of an RGBA layer's solid texels (alpha >= 0.5, the ones drawn).
fn solid_mean(px: &[u8]) -> [f32; 3] {
    let (mut sum, mut n) = ([0.0f64; 3], 0.0f64);
    for p in px.as_chunks::<4>().0.iter().filter(|p| p[3] >= 128) {
        for k in 0..3 {
            sum[k] += p[k] as f64;
        }
        n += 1.0;
    }
    if n == 0.0 {
        return [1.0; 3];
    }
    sum.map(|s| (s / n / 255.0) as f32)
}

/// Leaf cut-outs keep this much more alpha than their box-filtered coverage: the shader keeps a texel
/// at alpha >= 0.5, and averaging thin leaves down to `TEX` and down the mip chain would otherwise thin
/// the crowns to a few twigs at a distance (the original's cards are solid there) [design choice].
const ALPHA_BOOST: f64 = 2.0;

/// A pack image as one `TEX` x `TEX` RGBA layer: a box filter, the see-through texels coloured like
/// the mean of the solid ones (so mip levels and filtering show no dark fringe round the leaves).
fn resample(img: &gltf::image::Data) -> Option<Vec<u8>> {
    use gltf::image::Format;
    let ch = match img.format {
        Format::R8G8B8 => 3,
        Format::R8G8B8A8 => 4,
        _ => return None,
    };
    let (w, h, t) = (img.width as usize, img.height as usize, TEX as usize);
    let mut out = vec![0u8; t * t * 4];
    let mut mean = [0.0f64; 3];
    let mut solid = 0.0f64;
    for y in 0..t {
        for x in 0..t {
            let (x0, x1) = (x * w / t, ((x + 1) * w / t).max(x * w / t + 1));
            let (y0, y1) = (y * h / t, ((y + 1) * h / t).max(y * h / t + 1));
            let mut acc = [0.0f64; 4];
            for sy in y0..y1 {
                for sx in x0..x1 {
                    let p = &img.pixels[(sy * w + sx) * ch..][..ch];
                    let a = if ch == 4 { p[3] as f64 / 255.0 } else { 1.0 };
                    for k in 0..3 {
                        acc[k] += p[k] as f64 * a;
                    }
                    acc[3] += a;
                }
            }
            let n = ((x1 - x0) * (y1 - y0)) as f64;
            let o = &mut out[(y * t + x) * 4..][..4];
            if acc[3] > 0.0 {
                for k in 0..3 {
                    o[k] = (acc[k] / acc[3]).round().clamp(0.0, 255.0) as u8;
                    mean[k] += acc[k];
                }
                solid += acc[3];
            }
            o[3] = (acc[3] / n * ALPHA_BOOST * 255.0).round().clamp(0.0, 255.0) as u8;
        }
    }
    let fill = mean.map(|m| (m / solid.max(1e-6)).round().clamp(0.0, 255.0) as u8);
    for o in out.as_chunks_mut::<4>().0.iter_mut().filter(|o| o[3] == 0) {
        o[..3].copy_from_slice(&fill);
    }
    Some(out)
}

/// What `mesh` gives: the model triangles (world-style vertices, view space). Their texture layers are
/// the pack's (`pack_layers`), appended to `Scene::layers` once at `first_layer`.
pub struct Mesh {
    pub verts: Vec<Vertex>,
}

/// The pack's images as texture layers (`TEX` x `TEX` RGBA), in the pack's own order: a model vertex
/// of image `i` uses layer `first_layer + i`. Empty when the pack does not load. The scene appends
/// them to `Scene::layers` on every track (#390), so tree cards and placed models share them.
pub fn pack_layers() -> Vec<Vec<u8>> {
    pack().map_or_else(Vec::new, |p| p.images.clone())
}

/// Every model's name in the pack, sorted (tree, bush, hedge, grass, clover and flower models).
pub fn names() -> Vec<&'static str> {
    let mut n: Vec<&'static str> = pack().map_or_else(Vec::new, |p| p.models.keys().map(String::as_str).collect());
    n.sort_unstable();
    n
}

/// The models grouped by family, the name without its trailing `-N` (`Tree-01-2` -> `Tree-01`,
/// `Bush-03` -> `Bush`); families and members sorted.
pub fn families() -> Vec<(&'static str, Vec<&'static str>)> {
    let mut out: Vec<(&'static str, Vec<&'static str>)> = Vec::new();
    for n in names() {
        let f = n.rsplit_once('-').filter(|(_, tail)| tail.chars().all(|c| c.is_ascii_digit())).map_or(n, |(f, _)| f);
        match out.last_mut() {
            Some((k, m)) if *k == f => m.push(n),
            _ => out.push((f, vec![n])),
        }
    }
    out
}

/// A model's height in metres at scale 1 (None: no such model).
pub fn height(name: &str) -> Option<f32> {
    pack()?.models.get(name).map(|m| m.height)
}

/// A small integer hash of a position (stable variant and turn per tree).
fn hash(p: Vec3) -> u32 {
    let q = |f: f32| (f * 4.0).round() as i32 as u32;
    let mut h = q(p.x).wrapping_mul(0x9E37_79B1) ^ q(p.y).wrapping_mul(0x85EB_CA77) ^ q(p.z).wrapping_mul(0xC2B2_AE3D);
    h ^= h >> 15;
    h = h.wrapping_mul(0x2C1B_3C6D);
    h ^ (h >> 12)
}

/// How far a model's colour may be scaled to match its card, per channel
const MATCH: (f32, f32) = (0.4, 4.0);
/// The crown's share of a card's width (the picture's margin is see-through), and how far a model is
/// stretched sideways to it: the pack's trees are slimmer than the original's wide crowns [design choice]
const CROWN: f32 = 0.85;
const WIDEN: (f32, f32) = (0.8, 1.6);

/// The models for `sites`, each scaled to its card's height and turned by a per-tree angle. Coloured
/// like the card: the model's mean colour is scaled per channel to the card picture's mean (`layers`,
/// `Scene::layers`) times the card's baked light, so each track keeps its foliage hue and its light at
/// the tree; then a soft sun term and darker low down (APPROX: the world is pre-lit, so the models get
/// a look-alike bake, not real lighting). `first_layer`: the layer of the pack's first image (`pack_layers`). Empty
/// when the pack does not load.
pub fn mesh(sites: &[Site], look: &Look, layers_in: &[Vec<u8>], first_layer: usize) -> Mesh {
    let Some(pack) = pack() else { return Mesh { verts: Vec::new() } };
    let sun = Vec3::from(look.sun_dir).normalize_or(Vec3::Y);
    let mut verts = Vec::new();
    for s in sites {
        let Some(names) = models_for(&s.texture) else { continue };
        let h = hash(s.base);
        let Some(model) = pack.models.get(names[h as usize % names.len()]) else { continue };
        let scale = s.height / model.height;
        let widen = (CROWN * s.width / (model.width * scale)).clamp(WIDEN.0, WIDEN.1);
        let size = Vec3::new(scale * widen, scale, scale * widen);
        let card = s.layer.and_then(|l| layers_in.get(l)).map_or([1.0; 3], |px| solid_mean(px));
        let tint: [f32; 3] = [0, 1, 2].map(|k| s.tint[k] * (card[k] / model.albedo[k].max(1e-3)).clamp(MATCH.0, MATCH.1));
        let turn = Mat3::from_rotation_y((h >> 8) as f32 / (1u32 << 24) as f32 * std::f32::consts::TAU);
        emit(model, s.base, size, turn, &move |_| tint, sun, first_layer, &mut verts);
    }
    Mesh { verts }
}

/// One model at `base` (view space): scaled by `size` (per axis), turned by `turn`, its vertex colours
/// times `tint` (of the vertex's offset from `base`, view space) and a soft sun term, darker low down.
/// Texture layers count from `first_layer`.
#[allow(clippy::too_many_arguments)]
fn emit(model: &Model, base: Vec3, size: Vec3, turn: Mat3, tint: &dyn Fn(Vec3) -> [f32; 3], sun: Vec3, first_layer: usize, out: &mut Vec<Vertex>) {
    for v in &model.verts {
        let layer = v.image.map_or(-1.0, |i| (first_layer + i) as f32);
        let n = (turn * (v.normal / size)).normalize_or(Vec3::Y);
        let up = (v.pos.y / model.height).clamp(0.0, 1.0);
        // about 1 on average: the card's colour is the mean the model keeps
        let light = (0.85 + 0.3 * n.dot(sun).max(0.0)) * (0.7 + 0.45 * up);
        let d = turn * (v.pos * size);
        let t = tint(d);
        let c = [0, 1, 2].map(|k| (t[k] * v.color[k] * light).clamp(0.0, 1.0));
        out.push(Vertex {
            pos: (base + d).into(),
            uv: v.uv,
            color: [c[0], c[1], c[2], v.color[3]],
            layer,
            normal: n.into(),
            env: 0.0,
        });
    }
}

/// How far the track's tree light may differ from its ground light, per channel (`Match::light`)
const LIGHT_RATIO: (f32, f32) = (0.5, 1.5);

/// A track's own trees measured for the placed models (#390): what the original's tree cards look like
/// next to the ground they stand on, so a placed model looks like them.
#[derive(Clone, Debug, PartialEq)]
pub struct Match {
    /// the cards' baked light over the ground's light under them, per channel (median over the cards)
    pub light: [f32; 3],
    /// per card picture: the models it is swapped for and its mean colour (the track's foliage hue)
    cards: Vec<(&'static [&'static str], [f32; 3])>,
}

impl Match {
    /// The colour scale that brings `model` (pack model `name`) to the track's foliage, like `mesh` does
    /// for a card: to the pictures that swap for it, else (other trees, bushes, hedges) to all of them;
    /// grass, clover and flowers keep their own colour.
    fn hue(&self, name: &str, model: &Model) -> [f32; 3] {
        let own: Vec<[f32; 3]> = self.cards.iter().filter(|(m, _)| m.contains(&name)).map(|(_, c)| *c).collect();
        let foliage = ["Tree-", "Bush", "Hedge"].iter().any(|f| name.starts_with(f));
        let pick = if !own.is_empty() { own } else if foliage { self.cards.iter().map(|(_, c)| *c).collect() } else { return [1.0; 3] };
        let n = pick.len().max(1) as f32;
        [0, 1, 2].map(|k| (pick.iter().map(|c| c[k]).sum::<f32>() / n / model.albedo[k].max(1e-3)).clamp(MATCH.0, MATCH.1))
    }
}

/// The track's `Match` from its tree cards (`Swap::sites`): `layers` the scene's texture layers, `ground` the
/// ground's baked light at a view-space point (None off the ground). None without cards (APPROX: the
/// original lit its cards with their own normals; one ratio per track stands in for that).
pub fn calibrate(sites: &[Site], layers: &[Vec<u8>], ground: &dyn Fn(Vec3) -> Option<[f32; 3]>) -> Option<Match> {
    if sites.is_empty() {
        return None;
    }
    let mut cards: Vec<(&'static [&'static str], [f32; 3])> = Vec::new();
    let mut seen = Vec::new();
    let mut ratios: [Vec<f32>; 3] = Default::default();
    for s in sites {
        if let (Some(models), Some(l)) = (models_for(&s.texture), s.layer) {
            if !seen.contains(&l) {
                seen.push(l);
                if let Some(px) = layers.get(l) {
                    cards.push((models, solid_mean(px)));
                }
            }
        }
        if let Some(g) = ground(s.base) {
            for k in 0..3 {
                ratios[k].push(s.tint[k] / g[k].max(0.02));
            }
        }
    }
    let light = ratios.map(|mut r| {
        if r.is_empty() {
            return 1.0;
        }
        r.sort_unstable_by(f32::total_cmp);
        r[r.len() / 2].clamp(LIGHT_RATIO.0, LIGHT_RATIO.1)
    });
    Some(Match { light, cards })
}

/// How a placed model is lit (#390).
pub struct Light<'a> {
    /// the ground's baked light (0..1) at a view-space point, as the screen shows it; None off the ground
    pub ground: &'a dyn Fn(Vec3) -> Option<[f32; 3]>,
    /// the light when there is no ground under the model's base
    pub fallback: [f32; 3],
    /// the track's own trees (None: no tree cards; the ground's light as is and the model's own colour)
    pub track: Option<&'a Match>,
}

impl Light<'static> {
    /// One light everywhere (no ground, no track match).
    pub fn flat(c: [f32; 3]) -> Self {
        fn none(_: Vec3) -> Option<[f32; 3]> {
            None
        }
        Light { ground: &none, fallback: c, track: None }
    }
}

/// A placed model for the fill (#390): pack model `name` standing on `base` (view space), `scale` 1 =
/// its own size, `turn` radians about the view Y axis, `sun_dir` the track's (`Look::sun_dir`). Lit like
/// the ground under it: `light.ground` at the base and at the crown's edge towards +-x and +-z, blended
/// across the model (a tree on the edge of a dark patch is dark on that side), times the track's
/// `Match`. Appends its triangles to `out`; false when the pack has no such model.
#[allow(clippy::too_many_arguments)]
pub fn place(name: &str, base: Vec3, scale: f32, turn: f32, light: &Light, sun_dir: [f32; 3], first_layer: usize, out: &mut Vec<Vertex>) -> bool {
    let Some(model) = pack().and_then(|p| p.models.get(name)) else { return false };
    let sun = Vec3::from(sun_dir).normalize_or(Vec3::Y);
    let scale = scale.max(0.01);
    let r = (0.5 * model.width * scale).max(0.1);
    let c = (light.ground)(base).unwrap_or(light.fallback);
    let at = |d: Vec3| (light.ground)(base + d).unwrap_or(c);
    let (px, nx, pz, nz) = (at(Vec3::X * r), at(-Vec3::X * r), at(Vec3::Z * r), at(-Vec3::Z * r));
    let (hue, k) = light.track.map_or(([1.0; 3], [1.0; 3]), |m| (m.hue(name, model), m.light));
    let tint = move |d: Vec3| {
        let (u, w) = ((d.x / r).clamp(-1.0, 1.0), (d.z / r).clamp(-1.0, 1.0));
        // corner weights add to at most 1: no extrapolation past the samples
        let f = (u.abs() + w.abs()).max(1.0);
        let (u, w) = (u / f, w / f);
        let (sx, sz) = (if u >= 0.0 { px } else { nx }, if w >= 0.0 { pz } else { nz });
        [0, 1, 2].map(|i| (c[i] + u.abs() * (sx[i] - c[i]) + w.abs() * (sz[i] - c[i])) * k[i] * hue[i])
    };
    emit(model, base, Vec3::splat(scale), Mat3::from_rotation_y(turn), &tint, sun, first_layer, out);
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    fn quad(out: &mut Vec<Vertex>, a: Vec3, b: Vec3, c: Vec3, d: Vec3) {
        for p in [a, b, c, a, c, d] {
            out.push(Vertex { pos: p.into(), uv: [0.0; 2], color: [0.5, 0.6, 0.7, 1.0], layer: 0.0, normal: [0.0, 0.0, 1.0], env: 0.0 });
        }
    }

    /// A crossed card (two quads that share no vertex) and a grass patch: one tree, the grass stays.
    #[test]
    fn crossed_card_is_one_tree_and_grass_stays() {
        let mut t = Vec::new();
        let (o, h) = (Vec3::new(10.0, 2.0, -5.0), 6.0);
        quad(&mut t, o + Vec3::new(-2.0, 0.0, 0.0), o + Vec3::new(2.0, 0.0, 0.0), o + Vec3::new(2.0, h, 0.0), o + Vec3::new(-2.0, h, 0.0));
        quad(&mut t, o + Vec3::new(0.0, 0.0, -2.0), o + Vec3::new(0.0, 0.0, 2.0), o + Vec3::new(0.0, h, 2.0), o + Vec3::new(0.0, h, -2.0));
        quad(&mut t, Vec3::new(9.0, 2.0, -6.0), Vec3::new(11.0, 2.0, -6.0), Vec3::new(11.0, 2.0, -4.0), Vec3::new(9.0, 2.0, -4.0));
        let textures = vec!["Tree3.iff".to_string(), "gras1.iff".to_string()];
        let tri_tex = vec![0, 0, 0, 0, 1, 1];
        let s = find(&t, &[(0..6, 0)], &tri_tex, &textures);
        assert_eq!(s.sites.len(), 1);
        assert_eq!(s.hide, vec![true, true, true, true, false, false]);
        let site = &s.sites[0];
        assert!((site.height - h).abs() < 1e-5 && (site.base - o).length() < 1e-5, "{site:?}");
        assert!((site.tint[1] - 0.6).abs() < 1e-5);
    }

    /// Pines keep their cards; a card under `MIN_HEIGHT` is no tree.
    #[test]
    fn pines_and_tiny_cards_stay() {
        let mut t = Vec::new();
        quad(&mut t, Vec3::ZERO, Vec3::X, Vec3::new(1.0, 5.0, 0.0), Vec3::new(0.0, 5.0, 0.0));
        quad(&mut t, Vec3::new(20.0, 0.0, 0.0), Vec3::new(21.0, 0.0, 0.0), Vec3::new(21.0, 0.2, 0.0), Vec3::new(20.0, 0.2, 0.0));
        let textures = vec!["tree7.iff".to_string(), "buske.iff".to_string()];
        let s = find(&t, &[(0..4, 0)], &[0, 0, 1, 1], &textures);
        assert!(s.sites.is_empty());
        assert!(s.hide.iter().all(|h| !h));
    }

    /// The embedded pack has every model the table names.
    #[test]
    fn pack_has_every_model() {
        let p = pack().expect("the tree pack loads");
        for (_, names) in SWAP {
            for n in *names {
                let m = p.models.get(*n).unwrap_or_else(|| panic!("{n} missing"));
                assert!(!m.verts.is_empty() && m.verts.len().is_multiple_of(3));
                let bad = m.verts.iter().filter(|v| !(v.pos.is_finite() && v.normal.is_finite() && v.uv.iter().chain(&v.color).all(|f| f.is_finite()))).count();
                assert_eq!(bad, 0, "{n}: {bad} vertices not finite");
            }
        }
        assert!(p.images.iter().all(|i| i.len() == (TEX * TEX * 4) as usize));
    }

    /// Every pack model (hedge, grass, flowers too) loads with its images in range, in families.
    #[test]
    fn all_models_and_families() {
        let p = pack().expect("the tree pack loads");
        assert_eq!(names().len(), p.models.len());
        assert!(names().len() >= 30);
        assert_eq!(pack_layers().len(), p.images.len());
        for m in p.models.values() {
            assert!(m.verts.iter().all(|v| v.image.is_none_or(|i| i < p.images.len())));
            assert!(m.verts.iter().all(|v| v.pos.y >= -1e-5), "base at y = 0");
        }
        let fam = families();
        let members = |f: &str| fam.iter().find(|(k, _)| *k == f).map(|(_, m)| m.len());
        assert_eq!(members("Tree-01"), Some(4));
        assert_eq!(members("Bush"), Some(5));
        assert_eq!(members("Hedge"), Some(1));
        assert_eq!(fam.iter().map(|(_, m)| m.len()).sum::<usize>(), names().len());
    }

    /// A placed model has the model's triangles, stands on its base and turns about the view Y axis.
    #[test]
    fn place_stands_on_base_and_turns() {
        let sun = [0.3, 0.8, 0.5];
        let base = Vec3::new(5.0, 3.0, -2.0);
        let (mut a, mut b) = (Vec::new(), Vec::new());
        assert!(place("Tree-01-2", base, 1.0, 0.0, &Light::flat([1.0; 3]), sun, 7, &mut a));
        assert!(place("Tree-01-2", base, 2.0, std::f32::consts::FRAC_PI_2, &Light::flat([1.0; 3]), sun, 7, &mut b));
        assert!(!place("Nope", base, 1.0, 0.0, &Light::flat([1.0; 3]), sun, 7, &mut a));
        assert_eq!(a.len(), pack().unwrap().models["Tree-01-2"].verts.len());
        assert_eq!(a.len(), b.len());
        let lo = a.iter().map(|v| v.pos[1]).fold(f32::MAX, f32::min);
        let hi = a.iter().map(|v| v.pos[1]).fold(f32::MIN, f32::max);
        assert!((lo - 3.0).abs() < 1e-4 && (hi - 3.0 - height("Tree-01-2").unwrap()).abs() < 1e-3);
        assert!(a.iter().all(|v| v.layer < 0.0 || v.layer >= 7.0));
        // scale 2 doubles the height; a quarter turn takes a vertex at offset (x, z) to (z, -x)
        let hi2 = b.iter().map(|v| v.pos[1]).fold(f32::MIN, f32::max);
        assert!(((hi2 - 3.0) - 2.0 * (hi - 3.0)).abs() < 1e-3);
        let i = (0..a.len()).max_by(|&i, &j| (a[i].pos[0] - base.x).abs().total_cmp(&(a[j].pos[0] - base.x).abs())).unwrap();
        let (dx, dz) = (a[i].pos[0] - base.x, a[i].pos[2] - base.z);
        assert!((b[i].pos[0] - base.x - 2.0 * dz).abs() < 1e-3 && (b[i].pos[2] - base.z + 2.0 * dx).abs() < 1e-3);
    }

    /// A placed model on the edge of a dark patch is dark on that side and light on the other.
    #[test]
    fn place_follows_the_ground_light() {
        let base = Vec3::new(5.0, 0.0, 5.0);
        // dark towards -x, light towards +x
        let ground = |p: Vec3| Some([((p.x - base.x) / 10.0 + 0.5).clamp(0.0, 1.0); 3]);
        let light = Light { ground: &ground, fallback: [1.0; 3], track: None };
        let mut a = Vec::new();
        assert!(place("Tree-01-2", base, 1.0, 0.0, &light, [0.0, 1.0, 0.0], 0, &mut a));
        let side = |s: f32| {
            let v: Vec<&Vertex> = a.iter().filter(|v| (v.pos[0] - base.x) * s > 1.0).collect();
            v.iter().map(|v| v.color[1]).sum::<f32>() / v.len().max(1) as f32
        };
        assert!(side(1.0) > 1.5 * side(-1.0), "light side {} dark side {}", side(1.0), side(-1.0));
    }

    /// The track's match: the cards' light over the ground's (median), the pictures by model; flowers keep their colour.
    #[test]
    fn calibrate_measures_the_cards() {
        let site = |x: f32, tint: f32| Site { texture: "Tree3.iff".into(), base: Vec3::new(x, 0.0, 0.0), height: 6.0, width: 4.0, tint: [tint; 3], layer: Some(0) };
        let layers = vec![[40u8, 120, 20, 255].repeat(16)];
        // ground 0.8 everywhere; cards at 0.4, 0.6, 0.64 -> ratios 0.5, 0.75, 0.8
        let ground = |_: Vec3| Some([0.8; 3]);
        let m = calibrate(&[site(0.0, 0.4), site(10.0, 0.6), site(20.0, 0.64)], &layers, &ground).unwrap();
        assert!((m.light[0] - 0.75).abs() < 1e-5, "{:?}", m.light);
        assert_eq!(m.cards.len(), 1, "one picture");
        assert!(calibrate(&[], &layers, &ground).is_none());
        let p = pack().unwrap();
        let tree = &p.models["Tree-01-2"];
        let h = m.hue("Tree-01-2", tree);
        assert!((h[1] - (120.0 / 255.0 / tree.albedo[1]).clamp(MATCH.0, MATCH.1)).abs() < 1e-4);
        if let Some(f) = names().into_iter().find(|n| n.starts_with("Flower")) {
            assert_eq!(m.hue(f, &p.models[f]), [1.0; 3]);
        }
    }
}
