//! REMASTER world fill (re-notes/12-world-fill.md): generated geometry that covers the void past
//! the world edge and holes in the map at long view distances. `remaster/fill/trackNN.fill` in
//! the install holds one piece per line as `Key value ...` words like rimfile.rs; `#` starts a
//! comment, unknown keys are ignored, a bad line is reported and skipped, a missing file is no fill.
//!
//! ```text
//! Piece ground Texture GRAS1 UvScale 8 Point x z [y] ...    # polygon; y omitted = drape
//! Piece block  Texture WALL3 Roof ROOF1 Centre x y z Size w h d Yaw deg [Pitch deg] [Roll deg]   # y = base; angles in degrees, applied as roll, pitch, yaw
//! Piece wall   Texture WALL3 From x y z To x y z Height h
//! Piece hill   Texture ROCK1 Point x z [y] ... Height h Width w [Base d]   # ridge along a polyline; y omitted = crest at ground + h; d = feet above the ground
//! Piece patch  Texture ... Vertex x y z ...                    # 3 or 4 vertices
//! Piece skirt  Texture ... Radius r Drop d                     # meshed by the automatic pass (phase 2)
//! Piece plant  Species tree|pine|bush|rock Texture LEAF Trunk BARK Centre x y z Scale s Variant n [Yaw deg]   # low-poly tree, bush or rock; y = base; Texture = foliage / stone
//! Piece model  Name Tree-01-2 Centre x y z Scale s Yaw deg      # a model of the tree pack (trees.rs, #390); y = base, Scale 1 = the model's own size
//! Piece cloud  Centre x y z Size w h d [Yaw deg] Density k      # REMASTER local fog (#470); y = the middle, Density = extinction per 100 m at the centre
//! Piece object Name Je_tower Centre x y z Scale s Yaw deg        # a scene object of this track (#466, 12-world-fill 12.15), by .myo stem; y = base
//! Piece stamp  Name tower1 Centre x y z Scale s Yaw deg          # a copy of a Stamp's world triangles (#466, 12.16); y = where the box base lands
//! Stamp tower1 Centre x y z Size w h d Yaw deg [Floor f]         # a source box on this track's world (12.16), y = base; no mesh data stored
//! Water Texture WATT Style ocean ...                            # REMASTER water, see water.rs
//! Grass Texture Gras1+Gras2 Style meadow ... | Grass none        # REMASTER grass, see grass.rs
//! Seed n
//! ```
//! Coordinates are game coordinates (left-handed, z negated into view space when meshed).
//! Visual only: no COL, AI and physics untouched.

use crate::scene::{Look, Scene, Vertex, FINISH_EMBLEMS, SELF_LIT_ENV};
#[cfg(test)]
use crate::scene::PROP_ENV;
use glam::{Quat, Vec3};
use std::collections::{HashMap, HashSet};
use std::path::Path;

const HEADER: &str = "# Motorhead world fill v1";
const DEFAULT_UV_SCALE: f32 = 4.0;

#[derive(Clone, Debug, PartialEq)]
pub enum Shape {
    /// polygon in XZ; `None` height = drape onto the world
    Ground { points: Vec<([f32; 2], Option<f32>)> },
    /// `centre.y` is the base; the block rises `size[1]`
    /// `yaw` about +Y, `pitch` about the block's X and `roll` about its Z (degrees; see `block_rotation`)
    Block { centre: [f32; 3], size: [f32; 3], yaw: f32, pitch: f32, roll: f32, roof: Option<String> },
    Wall { from: [f32; 3], to: [f32; 3], height: f32 },
    /// ridge along a polyline; a point's `Some(y)` pins the crest there (absolute height), `None` = the
    /// ground under it + `height`. The feet of the slopes drape onto the ground, offset by `base` (m, negative = buried).
    Hill { points: Vec<([f32; 2], Option<f32>)>, height: f32, width: f32, base: f32 },
    /// 3 or 4 vertices
    Patch { verts: Vec<[f32; 3]> },
    Skirt { radius: f32, drop: f32 },
    /// Low-poly vegetation or a rock standing on `centre` (base). `scale` 1 is a tree of about 10 m;
    /// `variant` seeds the shape jitter; `yaw` turns it about +Y in degrees (0 = as generated; the editor's random
    /// rotation and the rotate gizmo set it); `trunk` is the trunk texture (None: the piece's texture).
    Plant { species: Species, centre: [f32; 3], scale: f32, variant: u32, yaw: f32, trunk: Option<String> },
    /// A model of the tree pack (`trees::names`) standing on `centre` (base), turned `yaw` degrees about +Y
    /// (game space, like a block's); `scale` 1 is the model's own size. Needs the pack's texture layers
    /// (`Scene::tree_pack_layer`); no Texture.
    Model { name: String, centre: [f32; 3], scale: f32, yaw: f32 },
    /// REMASTER local fog (#470, 12-world-fill 12.14): an ellipsoid of extra fog density round `centre` (its middle,
    /// not a base), `size` = full extent along its own x/y/z, turned `yaw` degrees about +Y like a block. `density` =
    /// extinction per 100 m at the centre (the fog file's unit), falling to 0 at the surface. Meshes nothing: the
    /// fog shaders read it (`clouds`, render.rs `pack_clouds`). No Texture.
    Cloud { centre: [f32; 3], size: [f32; 3], yaw: f32, density: f32 },
    /// A scene object of this track (#466, 12-world-fill 12.15): a `Tracks/trackN/Objects/<name>.myo` of the `.lws` scene
    /// (`objects`), by file stem, case-insensitive. Its lowest point stands on `centre` (base), it is turned `yaw` degrees
    /// about +Y (game space, like a block's) and scaled by `scale` (1 = its own size). Static, in its mesh pose; no
    /// Texture (the object's own pictures).
    Object { name: String, centre: [f32; 3], scale: f32, yaw: f32 },
    /// A copy of a `Stamp` of this file (#466, 12-world-fill 12.16), by name, case-insensitive: the world triangles in
    /// the stamp's box, its base on `centre`, turned to `yaw` degrees about +Y (the box's own axes are the building's:
    /// `yaw` = the stamp's yaw faces like the source) and scaled by `scale`. No Texture (the world's own pictures).
    Stamp { name: String, centre: [f32; 3], scale: f32, yaw: f32 },
}

/// A source box on this track's world (#466, 12-world-fill 12.16): `Stamp name Centre x y z Size w h d Yaw a [Floor f]`.
/// Game coordinates; `centre` is the base centre like a block's, `size` its full width, height and depth along its own
/// axes, turned `yaw` degrees about +Y. A world triangle belongs to it when its centroid lies in the box at least
/// `floor` above the base (the ground under a building stays behind). Only the box is stored, never mesh data.
#[derive(Clone, Debug, PartialEq)]
pub struct Stamp {
    pub name: String,
    pub centre: [f32; 3],
    pub size: [f32; 3],
    pub yaw: f32,
    pub floor: f32,
}

/// A new cloud's size (m) and density [untuned, design choice].
pub const CLOUD_SIZE: [f32; 3] = [120.0, 40.0, 120.0];
pub const CLOUD_DENSITY: f32 = 4.0;

/// A cloud as the renderer takes it (game space): centre, half extents, yaw (degrees), density per 100 m.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Cloud {
    pub centre: [f32; 3],
    pub half: [f32; 3],
    pub yaw: f32,
    pub density: f32,
}

/// Rows per cloud in the fog shaders' uniforms (`Cloud::packed`).
pub const CLOUD_ROWS: usize = 4;

impl Cloud {
    /// The shader rows (view space, z negated like the meshes): (centre, rho0 per metre) and the three rows of the
    /// matrix that takes a view-space offset from the centre into the unit sphere (un-yaw, then / half extents).
    /// `density_scale` multiplies the density (the Fog strength option).
    pub fn packed(&self, density_scale: f32) -> [[f32; 4]; CLOUD_ROWS] {
        let c = self.centre;
        let flip = glam::Mat3::from_diagonal(Vec3::new(1.0, 1.0, -1.0));
        let inv_half = Vec3::from(self.half).max(Vec3::splat(1e-3)).recip();
        // view offset -> game offset (flip) -> the cloud's own axes (inverse yaw) -> unit sphere
        let m = glam::Mat3::from_diagonal(inv_half) * glam::Mat3::from_quat(Quat::from_rotation_y(-self.yaw.to_radians())) * flip;
        let r = |i: usize| {
            let v = m.row(i);
            [v.x, v.y, v.z, 0.0]
        };
        [[c[0], c[1], -c[2], self.density / 100.0 * density_scale], r(0), r(1), r(2)]
    }

    /// The largest half extent: a sphere round the centre that holds the cloud.
    pub fn radius(&self) -> f32 {
        self.half.iter().copied().fold(0.0, f32::max)
    }
}

/// shader.wgsl / volfog.wgsl `cloud_tau` on the CPU: the optical depth of one packed cloud along `o + t dir`
/// (view space, `dir` unit), t in 0..t_max. The density `rho0 (1 - q)` is quadratic along the ray, integrated in
/// closed form round the ray's closest point to the centre. Test mirror of the shaders.
#[cfg(test)]
pub fn packed_tau(rows: &[[f32; 4]; CLOUD_ROWS], o: Vec3, dir: Vec3, t_max: f32) -> f32 {
    let row = |i: usize| Vec3::new(rows[i][0], rows[i][1], rows[i][2]);
    let d = o - row(0);
    let (u0, v) = (Vec3::new(row(1).dot(d), row(2).dot(d), row(3).dot(d)), Vec3::new(row(1).dot(dir), row(2).dot(dir), row(3).dot(dir)));
    let a = v.dot(v).max(1e-12);
    let tm = -u0.dot(v) / a;
    let pm = u0 + v * tm;
    let qmin = pm.dot(pm);
    if qmin >= 1.0 {
        return 0.0;
    }
    let half = ((1.0 - qmin) / a).sqrt();
    let (s0, s1) = ((tm - half).max(0.0) - tm, (tm + half).min(t_max) - tm);
    if s1 <= s0 {
        return 0.0;
    }
    rows[0][3] * ((1.0 - qmin) * (s1 - s0) - a * (s1 * s1 * s1 - s0 * s0 * s0) / 3.0)
}

/// The fill's cloud pieces (#470) for the fog shaders; empty and non-positive ones are left out.
pub fn clouds(f: &FillFile) -> Vec<Cloud> {
    clouds_where(f, &|_| true)
}

/// `clouds` of the pieces `keep` accepts (by index into `FillFile::pieces`): the editor leaves out clouds hidden by an eye or a layer.
pub fn clouds_where(f: &FillFile, keep: &dyn Fn(usize) -> bool) -> Vec<Cloud> {
    f.pieces
        .iter()
        .enumerate()
        .filter(|&(i, _)| keep(i))
        .filter_map(|(_, p)| match p.shape {
            Shape::Cloud { centre, size, yaw, density } if density > 0.0 && size.iter().all(|&v| v > 0.0) => {
                Some(Cloud { centre, half: size.map(|v| v * 0.5), yaw, density })
            }
            _ => None,
        })
        .collect()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Species {
    Tree,
    Pine,
    Bush,
    Rock,
}

impl Species {
    pub const ALL: [Species; 4] = [Species::Tree, Species::Pine, Species::Bush, Species::Rock];

    pub fn name(self) -> &'static str {
        ["tree", "pine", "bush", "rock"][self as usize]
    }

    fn parse(s: &str) -> Option<Species> {
        Self::ALL.into_iter().find(|k| k.name().eq_ignore_ascii_case(s))
    }

    /// Whether the species has a trunk (and so a trunk texture).
    pub fn trunk(self) -> bool {
        matches!(self, Species::Tree | Species::Pine)
    }
}

/// REMASTER default textures of the plants, from the original data [O, scan of every track's MYW
/// polygons, re-notes/tools/plant_tex.py, 12-world-fill 12.9]: vegetation is world geometry with one
/// colour-keyed picture per object (tracks 4 and 6: TREE3 .. BUSKE; no trunk part), rocks ROCK1/2
/// (tracks 4, 7) and tiling mountain rock on the others. `leaf` / `trunk` are the solid parts of the
/// picture as fractions of it (x0, y0, x1, y1; found by the same scan: the largest non-key
/// rectangle above and below 62 % of the height). None: the texture tiles (world UVs).
pub struct PlantTex {
    pub species: Species,
    pub name: &'static str,
    pub leaf: Option<[f32; 4]>,
    pub trunk: Option<[f32; 4]>,
}

const WHOLE: Option<[f32; 4]> = None;
const fn pt(species: Species, name: &'static str, leaf: [f32; 4], trunk: [f32; 4]) -> PlantTex {
    PlantTex { species, name, leaf: Some(leaf), trunk: Some(trunk) }
}

/// In order of preference (a track takes the first it has; the editor spreads over all it has).
pub const PLANT_TEXTURES: &[PlantTex] = &[
    pt(Species::Tree, "tree3.iff", [0.359, 0.181, 0.674, 0.474], [0.478, 0.621, 0.511, 0.991]),
    pt(Species::Tree, "_ettree3.iff", [0.325, 0.185, 0.588, 0.605], [0.225, 0.63, 0.412, 0.667]),
    pt(Species::Tree, "tree5.iff", [0.367, 0.33, 0.469, 0.438], [0.445, 0.625, 0.477, 0.991]),
    pt(Species::Tree, "tree6.iff", [0.353, 0.22, 0.807, 0.538], [0.454, 0.703, 0.555, 0.989]),
    pt(Species::Tree, "_ettree1.iff", [0.438, 0.094, 0.583, 0.316], [0.438, 0.692, 0.49, 0.991]),
    pt(Species::Pine, "tree7.iff", [0.31, 0.32, 0.644, 0.609], [0.218, 0.625, 0.816, 0.992]),
    pt(Species::Pine, "_tree1.iff", [0.383, 0.328, 0.594, 0.609], [0.32, 0.625, 0.617, 0.992]),
    pt(Species::Bush, "buske.iff", [0.235, 0.312, 0.852, 0.594], [0.296, 0.625, 0.79, 0.828]),
    // the cliffs have a grass fringe along the top: crop it off
    PlantTex { species: Species::Rock, name: "rock1.iff", leaf: Some([0.05, 0.3, 0.95, 0.95]), trunk: None },
    PlantTex { species: Species::Rock, name: "rock2.iff", leaf: Some([0.05, 0.3, 0.95, 0.95]), trunk: None },
    // other tracks: the mountain rock of the world, tiling
    PlantTex { species: Species::Rock, name: "_mberg1_.iff", leaf: WHOLE, trunk: None },
    PlantTex { species: Species::Rock, name: "64tberg.iff", leaf: WHOLE, trunk: None },
    PlantTex { species: Species::Rock, name: "64tberg2.iff", leaf: WHOLE, trunk: None },
    PlantTex { species: Species::Rock, name: "v3berg.iff", leaf: WHOLE, trunk: None },
    PlantTex { species: Species::Rock, name: "v3berg2.iff", leaf: WHOLE, trunk: None },
    // NolbyHills and Goldbridge have only these: grey cliff stone, tiling [O] (#477; Olympos and RuhrStadt have none)
    PlantTex { species: Species::Rock, name: "barne.iff", leaf: WHOLE, trunk: None },
    PlantTex { species: Species::Rock, name: "barne2.iff", leaf: WHOLE, trunk: None },
];

/// The table entry of a texture name (any species).
pub fn plant_tex(name: &str) -> Option<&'static PlantTex> {
    PLANT_TEXTURES.iter().find(|t| key(t.name) == key(name))
}

/// The entries for `species` whose texture this track has, in table order; the name is as the
/// track spells it.
pub fn plant_textures(species: Species, scene: &Scene) -> Vec<String> {
    PLANT_TEXTURES.iter().filter(|t| t.species == species).filter_map(|t| scene.world_textures.iter().find(|n| key(n) == key(t.name)).cloned()).collect()
}

#[derive(Clone, Debug, PartialEq)]
pub struct Piece {
    pub shape: Shape,
    pub texture: String,
    pub uv_scale: f32,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct FillFile {
    pub seed: u32,
    pub pieces: Vec<Piece>,
    /// REMASTER water bodies (`Water ...` lines, procgen/water.rs)
    pub water: Vec<super::water::Body>,
    /// REMASTER grass bodies (`Grass ...` lines, procgen/grass.rs); none in the file = the
    /// built-in rule, `Grass none` = no grass on this track
    pub grass: Vec<super::grass::Body>,
    pub grass_none: bool,
    /// source boxes of `Shape::Stamp` copies (`Stamp ...` lines, #466)
    pub stamps: Vec<Stamp>,
}

/// A block's orientation in game space: roll about its Z, then pitch about its X, then yaw about +Y
/// (degrees). Yaw alone is the earlier format's rotation.
pub fn block_rotation(yaw: f32, pitch: f32, roll: f32) -> Quat {
    Quat::from_rotation_y(yaw.to_radians()) * Quat::from_rotation_x(pitch.to_radians()) * Quat::from_rotation_z(roll.to_radians())
}

/// The (yaw, pitch, roll) in degrees of `q`, the inverse of `block_rotation`.
pub fn block_angles(q: Quat) -> (f32, f32, f32) {
    let (y, x, z) = q.to_euler(glam::EulerRot::YXZ);
    (y.to_degrees(), x.to_degrees(), z.to_degrees())
}

/// The fill of `track`: its file under `<root>/remaster/fill/` or empty. Problems are reported.
pub fn load(root: &Path, track: u32) -> FillFile {
    let path = root.join(format!("remaster/fill/track{track:02}.fill"));
    let Ok(text) = std::fs::read_to_string(&path) else { return FillFile::default() };
    let (f, warnings) = from_text(&text);
    for w in warnings {
        eprintln!("{}: {w}", path.display());
    }
    f
}

#[derive(Default)]
struct Fields {
    texture: String,
    roof: Option<String>,
    uv: Option<f32>,
    centre: Option<[f32; 3]>,
    size: Option<[f32; 3]>,
    yaw: f32,
    pitch: f32,
    roll: f32,
    from: Option<[f32; 3]>,
    to: Option<[f32; 3]>,
    height: f32,
    width: f32,
    base: f32,
    points: Vec<Vec<f32>>,
    verts: Vec<[f32; 3]>,
    radius: f32,
    drop: f32,
    species: Option<Species>,
    name: Option<String>,
    trunk: Option<String>,
    scale: Option<f32>,
    variant: u32,
    density: Option<f32>,
    floor: f32,
}

/// `Stamp name Centre x y z Size w h d [Yaw a] [Floor f]` (the words after `Stamp`).
fn stamp(name: &str, toks: &[&str]) -> Result<Stamp, String> {
    let f = fields(toks)?;
    Ok(Stamp { name: name.to_string(), centre: f.centre.ok_or("Stamp needs Centre")?, size: f.size.ok_or("Stamp needs Size")?, yaw: f.yaw, floor: f.floor })
}

fn piece(kind: &str, toks: &[&str]) -> Result<Piece, String> {
    let f = fields(toks)?;
    piece_of(kind, f)
}

fn fields(toks: &[&str]) -> Result<Fields, String> {
    let mut f = Fields::default();
    let mut i = 0;
    while i < toks.len() {
        let key = toks[i];
        i += 1;
        if matches!(key, "Texture" | "Roof" | "Trunk" | "Species" | "Name") {
            let w = toks.get(i).ok_or(format!("{key} needs a name"))?;
            i += 1;
            match key {
                "Texture" => f.texture = w.to_string(),
                "Roof" => f.roof = Some(w.to_string()),
                "Trunk" => f.trunk = Some(w.to_string()),
                "Name" => f.name = Some(w.to_string()),
                _ => f.species = Some(Species::parse(w).ok_or(format!("unknown species '{w}'"))?),
            }
            continue;
        }
        let mut v = Vec::new();
        while let Some(x) = toks.get(i).and_then(|t| t.parse::<f32>().ok()) {
            v.push(x);
            i += 1;
        }
        let need = |k: usize| if v.len() >= k { Ok(()) } else { Err(format!("{key} needs {k} values")) };
        match key {
            "UvScale" => { need(1)?; f.uv = Some(v[0]) }
            "Centre" => { need(3)?; f.centre = Some([v[0], v[1], v[2]]) }
            "Size" => { need(3)?; f.size = Some([v[0], v[1], v[2]]) }
            "Yaw" => { need(1)?; f.yaw = v[0] }
            "Pitch" => { need(1)?; f.pitch = v[0] }
            "Roll" => { need(1)?; f.roll = v[0] }
            "From" => { need(3)?; f.from = Some([v[0], v[1], v[2]]) }
            "To" => { need(3)?; f.to = Some([v[0], v[1], v[2]]) }
            "Height" => { need(1)?; f.height = v[0] }
            "Width" => { need(1)?; f.width = v[0] }
            "Base" => { need(1)?; f.base = v[0] }
            "Point" => { need(2)?; f.points.push(v) }
            "Vertex" => { need(3)?; f.verts.push([v[0], v[1], v[2]]) }
            "Radius" => { need(1)?; f.radius = v[0] }
            "Drop" => { need(1)?; f.drop = v[0] }
            "Scale" => { need(1)?; f.scale = Some(v[0]) }
            "Variant" => { need(1)?; f.variant = v[0] as u32 }
            "Density" => { need(1)?; f.density = Some(v[0]) }
            "Floor" => { need(1)?; f.floor = v[0] }
            _ => {}
        }
    }
    Ok(f)
}

fn piece_of(kind: &str, f: Fields) -> Result<Piece, String> {
    let shape = match kind {
        "ground" => {
            if f.points.len() < 3 { return Err("ground needs 3 Point".into()) }
            Shape::Ground { points: f.points.iter().map(|p| ([p[0], p[1]], p.get(2).copied())).collect() }
        }
        "block" => Shape::Block {
            centre: f.centre.ok_or("block needs Centre")?,
            size: f.size.ok_or("block needs Size")?,
            yaw: f.yaw,
            pitch: f.pitch,
            roll: f.roll,
            roof: f.roof,
        },
        "wall" => Shape::Wall { from: f.from.ok_or("wall needs From")?, to: f.to.ok_or("wall needs To")?, height: f.height },
        "hill" => {
            if f.points.len() < 2 { return Err("hill needs 2 Point".into()) }
            Shape::Hill { points: f.points.iter().map(|p| ([p[0], p[1]], p.get(2).copied())).collect(), height: f.height, width: f.width, base: f.base }
        }
        "patch" => {
            if !(3..=4).contains(&f.verts.len()) { return Err("patch needs 3 or 4 Vertex".into()) }
            Shape::Patch { verts: f.verts }
        }
        "skirt" => Shape::Skirt { radius: f.radius, drop: f.drop },
        "plant" => Shape::Plant {
            species: f.species.ok_or("plant needs Species")?,
            centre: f.centre.ok_or("plant needs Centre")?,
            scale: f.scale.unwrap_or(1.0),
            variant: f.variant,
            yaw: f.yaw,
            trunk: f.trunk,
        },
        "model" => Shape::Model { name: f.name.ok_or("model needs Name")?, centre: f.centre.ok_or("model needs Centre")?, scale: f.scale.unwrap_or(1.0), yaw: f.yaw },
        "object" => Shape::Object { name: f.name.ok_or("object needs Name")?, centre: f.centre.ok_or("object needs Centre")?, scale: f.scale.unwrap_or(1.0), yaw: f.yaw },
        "stamp" => Shape::Stamp { name: f.name.ok_or("stamp needs Name")?, centre: f.centre.ok_or("stamp needs Centre")?, scale: f.scale.unwrap_or(1.0), yaw: f.yaw },
        "cloud" => Shape::Cloud {
            centre: f.centre.ok_or("cloud needs Centre")?,
            size: f.size.ok_or("cloud needs Size")?,
            yaw: f.yaw,
            density: f.density.ok_or("cloud needs Density")?,
        },
        _ => return Err(format!("unknown piece '{kind}'")),
    };
    Ok(Piece { shape, texture: f.texture, uv_scale: f.uv.unwrap_or(DEFAULT_UV_SCALE) })
}

/// Parse; the second value lists the lines that were skipped.
pub fn from_text(text: &str) -> (FillFile, Vec<String>) {
    let mut out = FillFile::default();
    let mut warnings = Vec::new();
    for (n, raw) in text.lines().enumerate() {
        let l = raw.split('#').next().unwrap_or("");
        let toks: Vec<&str> = l.split_whitespace().collect();
        match toks.as_slice() {
            [] => {}
            ["Seed", s, ..] => out.seed = s.parse().unwrap_or(0),
            ["Water", rest @ ..] => match super::water::parse(rest) {
                Ok(b) => out.water.push(b),
                Err(e) => warnings.push(format!("line {}: {e}", n + 1)),
            },
            ["Grass", none] if none.eq_ignore_ascii_case("none") => out.grass_none = true,
            ["Grass", rest @ ..] => match super::grass::parse(rest) {
                Ok(b) => out.grass.push(b),
                Err(e) => warnings.push(format!("line {}: {e}", n + 1)),
            },
            ["Piece", kind, rest @ ..] => match piece(kind, rest) {
                Ok(p) => out.pieces.push(p),
                Err(e) => warnings.push(format!("line {}: {e}", n + 1)),
            },
            // a second stamp of the same name is never used: the first one wins
            ["Stamp", name, rest @ ..] => match stamp(name, rest) {
                Ok(st) if out.stamps.iter().any(|o| o.name.eq_ignore_ascii_case(&st.name)) => warnings.push(format!("line {}: a second Stamp '{name}' is skipped", n + 1)),
                Ok(st) => out.stamps.push(st),
                Err(e) => warnings.push(format!("line {}: {e}", n + 1)),
            },
            _ => {}
        }
    }
    (out, warnings)
}

pub fn to_text(f: &FillFile) -> String {
    let mut t = format!("{HEADER}\nSeed {}\n", f.seed);
    let v3 = |k: &str, v: &[f32; 3]| format!(" {k} {} {} {}", v[0], v[1], v[2]);
    for p in &f.pieces {
        let mut l = String::from("Piece ");
        let tex = |l: &mut String| {
            if !p.texture.is_empty() { l.push_str(&format!(" Texture {}", p.texture)) }
            // always written: a line without it reads as DEFAULT_UV_SCALE (4) while the editor makes pieces at 8
            l.push_str(&format!(" UvScale {}", p.uv_scale));
        };
        match &p.shape {
            Shape::Ground { points } => {
                l.push_str("ground");
                tex(&mut l);
                for (xz, y) in points {
                    l.push_str(&format!(" Point {} {}", xz[0], xz[1]));
                    if let Some(y) = y { l.push_str(&format!(" {y}")) }
                }
            }
            Shape::Block { centre, size, yaw, pitch, roll, roof } => {
                l.push_str("block");
                tex(&mut l);
                if let Some(r) = roof { l.push_str(&format!(" Roof {r}")) }
                l.push_str(&v3("Centre", centre));
                l.push_str(&v3("Size", size));
                l.push_str(&format!(" Yaw {yaw}"));
                if *pitch != 0.0 { l.push_str(&format!(" Pitch {pitch}")) }
                if *roll != 0.0 { l.push_str(&format!(" Roll {roll}")) }
            }
            Shape::Wall { from, to, height } => {
                l.push_str("wall");
                tex(&mut l);
                l.push_str(&v3("From", from));
                l.push_str(&v3("To", to));
                l.push_str(&format!(" Height {height}"));
            }
            Shape::Hill { points, height, width, base } => {
                l.push_str("hill");
                tex(&mut l);
                for (xz, y) in points {
                    l.push_str(&format!(" Point {} {}", xz[0], xz[1]));
                    if let Some(y) = y { l.push_str(&format!(" {y}")) }
                }
                l.push_str(&format!(" Height {height} Width {width}"));
                if *base != 0.0 {
                    l.push_str(&format!(" Base {base}"));
                }
            }
            Shape::Patch { verts } => {
                l.push_str("patch");
                tex(&mut l);
                for q in verts { l.push_str(&v3("Vertex", q)) }
            }
            Shape::Skirt { radius, drop } => {
                l.push_str("skirt");
                tex(&mut l);
                l.push_str(&format!(" Radius {radius} Drop {drop}"));
            }
            Shape::Plant { species, centre, scale, variant, yaw, trunk } => {
                l.push_str(&format!("plant Species {}", species.name()));
                tex(&mut l);
                if let Some(t) = trunk { l.push_str(&format!(" Trunk {t}")) }
                l.push_str(&v3("Centre", centre));
                l.push_str(&format!(" Scale {scale} Variant {variant}"));
                if *yaw != 0.0 { l.push_str(&format!(" Yaw {yaw}")) }
            }
            Shape::Model { name, centre, scale, yaw } => {
                l.push_str(&format!("model Name {name}"));
                l.push_str(&v3("Centre", centre));
                l.push_str(&format!(" Scale {scale} Yaw {yaw}"));
            }
            Shape::Object { name, centre, scale, yaw } => {
                l.push_str(&format!("object Name {name}"));
                l.push_str(&v3("Centre", centre));
                l.push_str(&format!(" Scale {scale} Yaw {yaw}"));
            }
            Shape::Stamp { name, centre, scale, yaw } => {
                l.push_str(&format!("stamp Name {name}"));
                l.push_str(&v3("Centre", centre));
                l.push_str(&format!(" Scale {scale} Yaw {yaw}"));
            }
            Shape::Cloud { centre, size, yaw, density } => {
                l.push_str("cloud");
                l.push_str(&v3("Centre", centre));
                l.push_str(&v3("Size", size));
                if *yaw != 0.0 { l.push_str(&format!(" Yaw {yaw}")) }
                l.push_str(&format!(" Density {density}"));
            }
        }
        t.push_str(&l);
        t.push('\n');
    }
    for st in &f.stamps {
        t.push_str(&format!("Stamp {}{}{} Yaw {}", st.name, v3("Centre", &st.centre), v3("Size", &st.size), st.yaw));
        if st.floor != 0.0 {
            t.push_str(&format!(" Floor {}", st.floor));
        }
        t.push('\n');
    }
    for b in &f.water {
        t.push_str(&super::water::to_text(b));
        t.push('\n');
    }
    if f.grass_none {
        t.push_str("Grass none\n");
    }
    for b in &f.grass {
        t.push_str(&super::grass::to_text(b));
        t.push('\n');
    }
    t
}

/// Ear clipping of a simple polygon (either winding): triangles as index triples.
pub fn triangulate(p: &[[f32; 2]]) -> Vec<[usize; 3]> {
    let n = p.len();
    if n < 3 {
        return Vec::new();
    }
    let area: f32 = (0..n).map(|i| { let (a, b) = (p[i], p[(i + 1) % n]); a[0] * b[1] - b[0] * a[1] }).sum();
    let s = if area < 0.0 { -1.0 } else { 1.0 };
    let cross = |a: [f32; 2], b: [f32; 2], c: [f32; 2]| s * ((b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0]));
    let mut idx: Vec<usize> = (0..n).collect();
    let mut out = Vec::new();
    while idx.len() > 3 {
        let m = idx.len();
        let ear = (0..m).find(|&k| {
            let (a, b, c) = (idx[(k + m - 1) % m], idx[k], idx[(k + 1) % m]);
            if cross(p[a], p[b], p[c]) <= 1e-9 {
                return false;
            }
            !idx.iter().any(|&q| {
                q != a && q != b && q != c
                    && cross(p[a], p[b], p[q]) >= 0.0 && cross(p[b], p[c], p[q]) >= 0.0 && cross(p[c], p[a], p[q]) >= 0.0
            })
        });
        // a degenerate polygon has no ear: clip the first corner anyway so it still ends
        let k = ear.unwrap_or(0);
        out.push([idx[(k + m - 1) % m], idx[k], idx[(k + 1) % m]]);
        idx.remove(k);
    }
    out.push([idx[0], idx[1], idx[2]]);
    out
}

/// A world vertex: where the fill samples ground height and baked colour.
struct Sample {
    pos: Vec3,
    color: [f32; 4],
    normal: Vec3,
}

const CELL: f32 = 4.0;

struct World<'a> {
    look: &'a Look,
    cells: HashMap<(i32, i32), Vec<Sample>>,
    all: &'a [Vertex],
    /// meshed skirt: ground past the world edge follows it
    skirt: Option<Skirt>,
}

impl<'a> World<'a> {
    fn new(s: &'a Scene) -> Self {
        let mut cells: HashMap<(i32, i32), Vec<Sample>> = HashMap::new();
        for v in &s.tris {
            cells.entry(Self::cell(v.pos[0], v.pos[2])).or_default().push(Sample { pos: Vec3::from(v.pos), color: v.color, normal: Vec3::from(v.normal) });
        }
        World { look: &s.look, cells, all: &s.tris, skirt: None }
    }
    fn cell(x: f32, z: f32) -> (i32, i32) {
        ((x / CELL).floor() as i32, (z / CELL).floor() as i32)
    }
    fn near(&self, x: f32, z: f32, n: i32) -> impl Iterator<Item = &Sample> {
        let (cx, cz) = Self::cell(x, z);
        (-n..=n).flat_map(move |dx| (-n..=n).map(move |dz| (dx, dz))).flat_map(move |(dx, dz)| self.cells.get(&(cx + dx, cz + dz)).into_iter().flatten())
    }
    /// APPROX: the lowest world vertex within 6 m (not a roof above the ground); beyond the
    /// world the nearest vertex of all.
    fn ground(&self, x: f32, z: f32) -> f32 {
        let lo = self.near(x, z, 2).filter(|s| (s.pos.x - x).powi(2) + (s.pos.z - z).powi(2) <= 36.0).map(|s| s.pos.y).fold(f32::MAX, f32::min);
        if lo < f32::MAX {
            return lo;
        }
        if let Some(k) = &self.skirt {
            return k.height(x, z);
        }
        self.all.iter().min_by(|a, b| {
            let d = |v: &&Vertex| (v.pos[0] - x).powi(2) + (v.pos[2] - z).powi(2);
            d(a).total_cmp(&d(b))
        }).map_or(0.0, |v| v.pos[1])
    }
    /// The baked colour of the nearest world vertex within 12 m facing the same way; else the
    /// track's sun and ambient · N·L (APPROX: the real bake also holds WorldBrightness).
    fn colour(&self, p: Vec3, n: Vec3) -> [f32; 4] {
        let best = self.near(p.x, p.z, 3)
            .filter(|s| s.normal.dot(n) > 0.5 && (s.pos - p).length_squared() <= 144.0)
            .min_by(|a, b| (a.pos - p).length_squared().total_cmp(&(b.pos - p).length_squared()));
        if let Some(s) = best {
            return s.color;
        }
        let l = n.dot(Vec3::from(self.look.sun_dir)).max(0.0);
        let c = |i: usize| (self.look.object_ambient[i] + self.look.sun[i] * l).clamp(0.0, 1.0);
        [c(0), c(1), c(2), 1.0]
    }
}

/// Cell size (m) of `GroundLight`'s grid: coarser than `CELL`, the ground's polygons are large.
const LIGHT_CELL: f32 = 16.0;
/// How far (m) above or below a point the ground under it may be, for `GroundLight::at`.
const LIGHT_REACH: f32 = 6.0;

/// The ground's baked light as the screen shows it (#390): the upward-facing triangles of the world and
/// of the fill's own pieces, by grid cell, for the Gouraud colour under a point. A placed model takes its
/// light from here, so it is as dark as the ground it stands on, wherever on a big polygon that is.
struct GroundLight {
    tris: Vec<[(Vec3, [f32; 3]); 3]>,
    cells: HashMap<(i32, i32), Vec<u32>>,
}

impl GroundLight {
    /// From whole triangles (3 vertices each); steep ones (walls, cards) are left out.
    fn new<'v>(tris: impl Iterator<Item = &'v [Vertex; 3]>) -> Self {
        let cell = |x: f32, z: f32| ((x / LIGHT_CELL).floor() as i32, (z / LIGHT_CELL).floor() as i32);
        let mut g = GroundLight { tris: Vec::new(), cells: HashMap::new() };
        for t in tris {
            let p = [0, 1, 2].map(|k| Vec3::from(t[k].pos));
            let n = (p[1] - p[0]).cross(p[2] - p[0]);
            if n.y.abs() < 0.3 * n.length() || n.length_squared() < 1e-12 {
                continue;
            }
            let id = g.tris.len() as u32;
            g.tris.push([0, 1, 2].map(|k| (p[k], [t[k].color[0], t[k].color[1], t[k].color[2]])));
            let (lo, hi) = (cell(p[0].x.min(p[1].x).min(p[2].x), p[0].z.min(p[1].z).min(p[2].z)), cell(p[0].x.max(p[1].x).max(p[2].x), p[0].z.max(p[1].z).max(p[2].z)));
            for cx in lo.0..=hi.0 {
                for cz in lo.1..=hi.1 {
                    g.cells.entry((cx, cz)).or_default().push(id);
                }
            }
        }
        g
    }
    /// The interpolated vertex colour of the ground triangle under `p` (view space) whose height there is
    /// nearest `p.y`, within `LIGHT_REACH`; None off the ground.
    fn at(&self, p: Vec3) -> Option<[f32; 3]> {
        let key = ((p.x / LIGHT_CELL).floor() as i32, (p.z / LIGHT_CELL).floor() as i32);
        let mut best: Option<(f32, [f32; 3])> = None;
        for &id in self.cells.get(&key).into_iter().flatten() {
            let [(a, ca), (b, cb), (c, cc)] = self.tris[id as usize];
            let d = (b.x - a.x) * (c.z - a.z) - (c.x - a.x) * (b.z - a.z);
            if d.abs() < 1e-9 {
                continue;
            }
            let wb = ((p.x - a.x) * (c.z - a.z) - (c.x - a.x) * (p.z - a.z)) / d;
            let wc = ((b.x - a.x) * (p.z - a.z) - (p.x - a.x) * (b.z - a.z)) / d;
            let wa = 1.0 - wb - wc;
            const E: f32 = -1e-4;
            if wa < E || wb < E || wc < E {
                continue;
            }
            let dy = (wa * a.y + wb * b.y + wc * c.y - p.y).abs();
            if dy <= LIGHT_REACH && best.is_none_or(|(d0, _)| dy < d0) {
                best = Some((dy, [0, 1, 2].map(|k| wa * ca[k] + wb * cb[k] + wc * cc[k])));
            }
        }
        best.map(|(_, c)| c)
    }
}

fn key(name: &str) -> String {
    name.split('.').next().unwrap_or("").to_ascii_lowercase()
}

/// How a plant primitive maps a view-space point to a texture coordinate.
#[derive(Clone, Copy)]
enum Uv {
    /// world metres / `s` (tiling)
    World(f32),
    /// the same on a tapered prism (trunks)
    Prism(f32),
    /// a picture's `rect` (fractions) spread over the object: round it (mirrored, no seam) and up
    /// from `base` over `height`
    Crop { rect: [f32; 4], base: Vec3, height: f32 },
}

impl Uv {
    fn at(self, q: Vec3) -> [f32; 2] {
        match self {
            Uv::World(s) => [q.x / s, q.z / s],
            Uv::Prism(s) => [q.x / s + q.z / s, q.y / s],
            Uv::Crop { rect: [x0, y0, x1, y1], base, height } => {
                let u = (q.z - base.z).atan2(q.x - base.x).abs() / std::f32::consts::PI;
                let v = ((q.y - base.y) / height.max(1e-3)).clamp(0.0, 1.0);
                [x0 + (x1 - x0) * u, y1 - (y1 - y0) * v]
            }
        }
    }
}

/// CLASSIC `D3D_PROP_TEX_FACE` (shader.wgsl, 0x4407b6): a lit scene object's textured face is 200/255 of ObjectBrightness.
const PROP_TEX_FACE: f32 = 200.0 / 255.0;

/// The scene objects a fill can place (`Shape::Object`): the `.lws` scene's `.myo` meshes that have triangles, with their
/// file stems; the podium emblems that follow them in `Scene::props` are not scenery. `names` / `props` are
/// `Scene::prop_names` / `Scene::props`.
fn object_list<'a>(names: &'a [String], props: &'a [Vec<Vertex>]) -> impl Iterator<Item = (&'a str, &'a [Vertex])> {
    let scenery = props.len().saturating_sub(FINISH_EMBLEMS.len());
    names.iter().zip(props).take(scenery).filter(|(_, m)| !m.is_empty()).map(|(n, m)| (n.as_str(), m.as_slice()))
}

/// A placeable scene object as the editor lists it: its stem and its box once standing on its base (game axes, unscaled,
/// unturned: x across, y up from 0, z along; see `place_object`).
#[derive(Clone, Debug, PartialEq)]
pub struct ObjectInfo {
    pub name: String,
    pub lo: Vec3,
    pub hi: Vec3,
}

/// This track's placeable scene objects, in scene order (the editor's Structure list and outlines).
pub fn objects(s: &Scene) -> Vec<ObjectInfo> {
    let mut seen = HashSet::new();
    object_list(&s.prop_names, &s.props)
        .filter(|(n, _)| seen.insert(n.to_ascii_lowercase()))
        .map(|(name, mesh)| {
            let (lo, hi) = mesh.iter().fold((Vec3::splat(f32::MAX), Vec3::splat(f32::MIN)), |(lo, hi), v| (lo.min(Vec3::from(v.pos)), hi.max(Vec3::from(v.pos))));
            // object-local view space -> standing on its base, game axes (z negated)
            ObjectInfo { name: name.to_string(), lo: Vec3::new(lo.x, 0.0, -hi.z), hi: Vec3::new(hi.x, hi.y - lo.y, -lo.z) }
        })
        .collect()
}

/// A scene object's triangles (`mesh`, object-local view space) standing on `base` (view space): its lowest point there,
/// scaled by `scale`, turned `turn` radians about the vertical axis through its origin. Each vertex keeps its picture,
/// UVs and normal (turned) and takes `face` (a textured face: `tex_face`, the original's constant; else its own colour)
/// times `light(point, normal)`.
///
/// APPROX [I]: the original lights a scene object per frame from the cell's point lights and ObjectAmbient (0x43ecfc);
/// a baked colour stands in, the fill's own rule (`World::colour`), so the copy is lit like the ground round it.
/// Env-mapped polygons (`PROP_ENV`) become plain textured ones, retro-reflective sign faces plain ones; blinker halos and
/// the object's animation are not copied. A face the original draws unlit (`SELF_LIT_ENV`) stays unlit.
pub(crate) fn place_object(mesh: &[Vertex], base: Vec3, scale: f32, turn: f32, tex_face: [f32; 3], light: &dyn Fn(Vec3, Vec3) -> [f32; 4], out: &mut Vec<Vertex>) {
    let low = mesh.iter().map(|v| v.pos[1]).fold(f32::MAX, f32::min);
    let (q, k) = (Quat::from_rotation_y(turn), scale.max(0.01));
    out.extend(mesh.iter().map(|v| {
        let p = base + q * ((Vec3::from(v.pos) - Vec3::Y * low) * k);
        let n = (q * Vec3::from(v.normal)).normalize_or(Vec3::Y);
        let textured = v.layer >= 0.0;
        let face = if textured { tex_face } else { [v.color[0], v.color[1], v.color[2]] };
        let l = if !textured && v.env == SELF_LIT_ENV { [1.0; 4] } else { light(p, n) };
        Vertex { pos: p.into(), uv: v.uv, color: [face[0] * l[0], face[1] * l[1], face[2] * l[2], 1.0], layer: v.layer, normal: n.into(), env: 0.0 }
    }));
}

/// The world triangles of `st` (#466, 12-world-fill 12.16): every triangle of `tris` (`Scene::tris`, view space) whose
/// centroid lies inside the stamp's turned box at least `floor` above its base, and that `skip` (triangle index) keeps.
/// Returned in the box's own frame, game axes (x across, y up from the base, z along): position and normal moved,
/// colour, picture, UVs and env as they are. A full scan; a few tens of thousands of triangles cost well under a
/// millisecond, and the mesher keeps one result per stamp (design choice: no cell culling).
pub(crate) fn stamp_tris(tris: &[Vertex], skip: &dyn Fn(usize) -> bool, st: &Stamp) -> Vec<Vertex> {
    let inv = Quat::from_rotation_y(st.yaw.to_radians()).inverse();
    let (c, half) = (Vec3::from(st.centre), Vec3::new(st.size[0] * 0.5, st.size[1], st.size[2] * 0.5));
    let game = |v: [f32; 3]| Vec3::new(v[0], v[1], -v[2]);
    let mut out = Vec::new();
    for (i, t) in tris.as_chunks::<3>().0.iter().enumerate() {
        let local = inv * ((game(t[0].pos) + game(t[1].pos) + game(t[2].pos)) / 3.0 - c);
        if local.x.abs() > half.x || local.z.abs() > half.z || local.y < st.floor || local.y > half.y || skip(i) {
            continue;
        }
        out.extend(t.iter().map(|v| Vertex { pos: (inv * (game(v.pos) - c)).into(), normal: (inv * game(v.normal)).into(), ..*v }));
    }
    out
}

/// A stamp's triangles (`local`, from `stamp_tris`) as a copy: its base on `base` (game space), scaled by `scale`, turned
/// `yaw` degrees about +Y (game space, like a block), back into view space. Mirroring z on the way in and out keeps the
/// winding.
///
/// APPROX [I]: the triangles keep the Gouraud colours the original baked at the source spot, so a copy is lit as the
/// source was; the dynamic point lights use the copied plane normals.
pub(crate) fn place_stamp(local: &[Vertex], base: Vec3, scale: f32, yaw: f32, out: &mut Vec<Vertex>) {
    let (q, k) = (Quat::from_rotation_y(yaw.to_radians()), scale.max(0.01));
    let view = |g: Vec3| [g.x, g.y, -g.z];
    out.extend(local.iter().map(|v| Vertex { pos: view(base + q * (Vec3::from(v.pos) * k)), normal: view(q * Vec3::from(v.normal)), ..*v }));
}

/// `fit_stamp`: two pieces of one building when their footprints (XZ bounds) overlap by at least this share of the
/// smaller one [design choice; NeoCity scan: 0.5 joins roofs and podiums, 0.1 starts chaining neighbours]
const FIT_OVERLAP: f32 = 0.5;
/// `fit_stamp`: a piece joins only if the group stays within this many metres in x and z (stops a chain of
/// buildings running down a street; NeoCity's biggest tower is 74 m, and 120 to 220 m find the same buildings there)
/// [design choice]
const FIT_SPAN: f32 = 160.0;
/// `fit_stamp`: height ranges that come within this many metres count as touching
const FIT_TOUCH: f32 = 0.5;
/// `fit_stamp`: the fitted box's margin on its sides and top, so triangles lying on a face are cut
const FIT_PAD: f32 = 0.1;

/// A stamp box fitted round a building (`fit_stamp`): the stamp's `centre`, `size` and `yaw`, how many welded pieces the
/// building was found in, and their triangle count.
#[derive(Clone, Debug, PartialEq)]
pub struct StampFit {
    pub centre: [f32; 3],
    pub size: [f32; 3],
    pub yaw: f32,
    pub parts: usize,
    pub tris: usize,
}

/// The building that triangle `seed` of `tris` (`Scene::tris`, view space) belongs to, as a stamp box (#475, 12-world-fill
/// 12.16). The buildings are baked into the world in 19 m cells with no record of which parts belong together, so:
/// 1. triangles sharing a vertex (1 cm grid) are one piece (`water::pieces`; those `skip` keeps out do not count). The
///    biggest piece is the ground sheet; buildings stand on it, usually not welded to it;
/// 2. from the seed's piece, a piece joins when its footprint overlaps the group's by `FIT_OVERLAP` of the smaller, its
///    height range touches the group's within `FIT_TOUCH` and the group stays within `FIT_SPAN`; again until nothing
///    joins (a roof, a podium and the tower are several pieces);
/// 3. the box is turned to the walls (area weighted, mod 90 degrees), sized to the group in that frame plus `FIT_PAD`,
///    its base on the group's lowest point.
///
/// None: the seed is not in the world, is `skip`ped, or is in the ground sheet (a building welded into the ground cannot
/// be told from it: the caller keeps its default box).
pub(crate) fn fit_stamp(tris: &[Vertex], skip: &dyn Fn(usize) -> bool, seed: usize) -> Option<StampFit> {
    let game = |v: &Vertex| Vec3::new(v.pos[0], v.pos[1], -v.pos[2]);
    let kept: Vec<usize> = (0..tris.len() / 3).filter(|&i| !skip(i)).collect();
    let g: Vec<[[f32; 3]; 3]> = kept.iter().map(|&i| std::array::from_fn(|k| game(&tris[i * 3 + k]).into())).collect();
    let ids = super::water::pieces(&g);
    let seed_id = ids[kept.iter().position(|&i| i == seed)?];
    // per piece: triangle count and bounds
    let mut pieces: HashMap<usize, (usize, Vec3, Vec3)> = HashMap::new();
    for (t, &id) in g.iter().zip(&ids) {
        let e = pieces.entry(id).or_insert((0, Vec3::MAX, Vec3::MIN));
        e.0 += 1;
        for p in t {
            (e.1, e.2) = (e.1.min(Vec3::from(*p)), e.2.max(Vec3::from(*p)));
        }
    }
    let ground = pieces.iter().max_by_key(|(id, p)| (p.0, std::cmp::Reverse(**id))).map(|(&id, _)| id)?;
    if seed_id == ground {
        return None;
    }
    let foot = |lo: Vec3, hi: Vec3| (hi.x - lo.x).max(0.5) * (hi.z - lo.z).max(0.5);
    let (mut group, mut lo, mut hi) = (vec![seed_id], pieces[&seed_id].1, pieces[&seed_id].2);
    // sorted ids: the same building whatever the hash order
    let mut others: Vec<usize> = pieces.keys().copied().filter(|&id| id != ground && id != seed_id).collect();
    others.sort_unstable();
    loop {
        let before = group.len();
        others.retain(|id| {
            let (_, plo, phi) = pieces[id];
            let (ox, oz) = (hi.x.min(phi.x) - lo.x.max(plo.x), hi.z.min(phi.z) - lo.z.max(plo.z));
            let (ga, pa) = (foot(lo, hi), foot(plo, phi));
            let span = hi.max(phi) - lo.min(plo);
            let joins = ox > 0.0 && oz > 0.0 && plo.y - FIT_TOUCH <= hi.y && lo.y - FIT_TOUCH <= phi.y && span.x <= FIT_SPAN && span.z <= FIT_SPAN && ox * oz >= FIT_OVERLAP * ga.min(pa);
            if joins {
                (lo, hi) = (lo.min(plo), hi.max(phi));
                group.push(*id);
            }
            !joins
        });
        if group.len() == before {
            break;
        }
    }
    let mine: Vec<&[[f32; 3]; 3]> = g.iter().zip(&ids).filter(|(_, id)| group.contains(id)).map(|(t, _)| t).collect();
    // the walls' turn: 4 x angle averaged, so a wall and the wall across it (and the one at a right angle) agree
    let (mut s, mut c) = (0.0f32, 0.0f32);
    for t in &mine {
        let (a, b, d) = (Vec3::from(t[0]), Vec3::from(t[1]), Vec3::from(t[2]));
        let n = (b - a).cross(d - a);
        if n.length() > 1e-6 && n.y.abs() < 0.3 * n.length() {
            // the box's x axis is (cos yaw, 0, -sin yaw) in game space
            let phi = (-n.z).atan2(n.x);
            let w = n.length();
            (s, c) = (s + w * (4.0 * phi).sin(), c + w * (4.0 * phi).cos());
        }
    }
    let yaw = if s.abs() + c.abs() > 1e-4 { (s.atan2(c) / 4.0).to_degrees() } else { 0.0 };
    let inv = Quat::from_rotation_y(yaw.to_radians()).inverse();
    let (mut llo, mut lhi) = (Vec3::MAX, Vec3::MIN);
    for p in mine.iter().flat_map(|t| t.iter()) {
        let q = inv * Vec3::from(*p);
        (llo, lhi) = (llo.min(q), lhi.max(q));
    }
    let mid = (llo + lhi) * 0.5;
    let base = Quat::from_rotation_y(yaw.to_radians()) * Vec3::new(mid.x, llo.y, mid.z);
    Some(StampFit {
        centre: base.into(),
        size: [lhi.x - llo.x + 2.0 * FIT_PAD, lhi.y - llo.y + FIT_PAD, lhi.z - llo.z + 2.0 * FIT_PAD],
        yaw,
        parts: group.len(),
        tris: mine.len(),
    })
}

struct Mesher<'a> {
    w: World<'a>,
    /// the scene, for `Shape::Object`'s meshes (`Scene::props`) and `Shape::Stamp`'s triangles (`Scene::tris`)
    scene: &'a Scene,
    /// the file's stamps, and per stamp name (lowercase) its cut triangles (`stamp_tris`; None: no such stamp)
    stamps: &'a [Stamp],
    cut: HashMap<String, Option<Vec<Vertex>>>,
    layers: HashMap<String, f32>,
    missing: HashSet<String>,
    out: Vec<Vertex>,
    /// the scene's first tree pack layer (`Scene::tree_pack_layer`), for `Shape::Model`
    pack_layer: Option<usize>,
    /// for `Shape::Model` (#390), built after the other pieces: the ground's light (world and fill) and
    /// the track's own trees measured against it (None: no tree cards on this track)
    light: Option<GroundLight>,
    tree_match: Option<super::trees::Match>,
    /// the meshed skirt's centre (view space) and the world's lowest y
    skirt: Option<(Vec3, f32)>,
    /// the ridge heights of the hill meshed last (pinned or draped), for the editor's handles
    crest: Vec<f32>,
    /// a turn applied to every triangle while a plant is meshed: (pivot, rotation), view space
    turn: Option<(Vec3, Quat)>,
}

impl Mesher<'_> {
    fn layer(&mut self, name: &str) -> f32 {
        if let Some(&l) = self.layers.get(&key(name)) {
            return l;
        }
        if self.missing.insert(key(name)) {
            eprintln!("fill: texture '{name}' is not in this track's set");
        }
        -1.0
    }
    /// One triangle in view space. `hint`: the side the normal should face.
    fn tri(&mut self, p: [Vec3; 3], uv: [[f32; 2]; 3], layer: f32, hint: Option<Vec3>) {
        let (mut p, mut uv) = (p, uv);
        // a turned plant: the points about its base, the side hint with them
        let hint = match self.turn {
            Some((at, q)) => {
                p = p.map(|v| at + q * (v - at));
                hint.map(|h| q * h)
            }
            None => hint,
        };
        let mut n = (p[1] - p[0]).cross(p[2] - p[0]).normalize_or_zero();
        if n == Vec3::ZERO {
            return;
        }
        if hint.is_some_and(|h| n.dot(h) < 0.0) {
            p.swap(1, 2);
            uv.swap(1, 2);
            n = -n;
        }
        for k in 0..3 {
            self.out.push(Vertex { pos: p[k].into(), uv: uv[k], color: self.w.colour(p[k], n), layer, normal: n.into(), env: 0.0 });
        }
    }
    /// A quad a-b-c-d (in order round it).
    fn quad(&mut self, q: [Vec3; 4], uv: [[f32; 2]; 4], layer: f32, hint: Option<Vec3>) {
        self.tri([q[0], q[1], q[2]], [uv[0], uv[1], uv[2]], layer, hint);
        self.tri([q[0], q[2], q[3]], [uv[0], uv[2], uv[3]], layer, hint);
    }
    /// Flat UVs in world metres.
    fn flat(p: Vec3, s: f32) -> [f32; 2] {
        [p.x / s, p.z / s]
    }
    /// A vertical face: along the bottom edge, up the height.
    fn upright(a: Vec3, b: Vec3, up: f32, s: f32) -> [[f32; 2]; 4] {
        let u = (b - a).length() / s;
        [[0.0, a.y / s], [u, b.y / s], [u, (b.y + up) / s], [0.0, (a.y + up) / s]]
    }

    /// A cone from a ring of `segs` points round `base` (radius `r`) up to the apex `h` above.
    fn cone(&mut self, base: Vec3, r: f32, h: f32, segs: usize, (layer, uv): (f32, Uv)) {
        let at = |k: usize| {
            let a = k as f32 / segs as f32 * std::f32::consts::TAU;
            base + Vec3::new(a.cos() * r, 0.0, a.sin() * r)
        };
        let apex = base + Vec3::Y * h;
        for k in 0..segs {
            let (a, b) = (at(k), at(k + 1));
            let out = (a + b) / 2.0 - base;
            self.tri([a, b, apex], [a, b, apex].map(|q| uv.at(q)), layer, Some(out + Vec3::Y * r / h.max(0.1) * 0.3));
        }
    }

    /// A tapered prism: radius `r0` at `base`, `r1` at `h` above.
    fn prism(&mut self, base: Vec3, r0: f32, r1: f32, h: f32, segs: usize, (layer, uv): (f32, Uv)) {
        let ring = |k: usize, r: f32, y: f32| {
            let a = k as f32 / segs as f32 * std::f32::consts::TAU;
            base + Vec3::new(a.cos() * r, y, a.sin() * r)
        };
        for k in 0..segs {
            let q = [ring(k, r0, 0.0), ring(k + 1, r0, 0.0), ring(k + 1, r1, h), ring(k, r1, h)];
            let out = (q[0] + q[1]) / 2.0 - base;
            self.quad(q, q.map(|v| uv.at(v)), layer, Some(out.with_y(0.0)));
        }
    }

    /// A jittered ellipsoid (3 rings of `segs` points and two poles) centred on `c`.
    fn blob(&mut self, c: Vec3, radii: Vec3, jitter: f32, segs: usize, rng: &mut Rng, (layer, uv): (f32, Uv)) {
        const LATS: [f32; 3] = [-0.6, 0.0, 0.6];
        let jit = |rng: &mut Rng| 1.0 + rng.range(-jitter, jitter);
        let pole = |sgn: f32, j: f32| c + Vec3::new(0.0, sgn * radii.y * j, 0.0);
        let (bottom, top) = (pole(-1.0, jit(rng)), pole(1.0, jit(rng)));
        let rings: Vec<Vec<Vec3>> = LATS.iter().map(|&lat| {
            let (lat_r, lat_y) = ((lat * std::f32::consts::FRAC_PI_2).cos(), (lat * std::f32::consts::FRAC_PI_2).sin());
            (0..segs).map(|k| {
                let a = k as f32 / segs as f32 * std::f32::consts::TAU;
                let j = jit(rng);
                c + Vec3::new(a.cos() * lat_r * radii.x, lat_y * radii.y, a.sin() * lat_r * radii.z) * j
            }).collect()
        }).collect();
        let uv = |q: Vec3| uv.at(q);
        for k in 0..segs {
            let n = (k + 1) % segs;
            let out = |a: Vec3, b: Vec3| (a + b) / 2.0 - c;
            let (r0, r1, r2) = (&rings[0], &rings[1], &rings[2]);
            self.tri([bottom, r0[n], r0[k]], [bottom, r0[n], r0[k]].map(uv), layer, Some(out(r0[k], r0[n]) - Vec3::Y));
            for (lo, hi) in [(r0, r1), (r1, r2)] {
                let q = [lo[k], lo[n], hi[n], hi[k]];
                self.quad(q, q.map(uv), layer, Some(out(lo[k], hi[n])));
            }
            self.tri([top, r2[k], r2[n]], [top, r2[k], r2[n]].map(uv), layer, Some(out(r2[k], r2[n]) + Vec3::Y));
        }
    }

    /// REMASTER vegetation and rocks, view-space `base`; `tex` = [foliage / stone, trunk], each
    /// a (layer, the picture's solid part; None: tile the texture).
    fn plant(&mut self, species: Species, base: Vec3, scale: f32, variant: u32, tex: [(f32, Option<[f32; 4]>); 2], s: f32) {
        let mut rng = Rng(variant.wrapping_mul(2_654_435_761) | 1);
        let k = scale.max(0.05);
        let ((leaf, leaf_crop), (wood, wood_crop)) = (tex[0], tex[1]);
        let leaf_uv = |at: Vec3, h: f32| leaf_crop.map_or(Uv::World(s), |rect| Uv::Crop { rect, base: at, height: h });
        let wood_uv = |at: Vec3, h: f32| wood_crop.map_or(Uv::Prism(s), |rect| Uv::Crop { rect, base: at, height: h });
        match species {
            Species::Tree => {
                self.prism(base, 0.35 * k, 0.22 * k, 4.2 * k, 6, (wood, wood_uv(base, 4.2 * k)));
                let r = rng.range(2.8, 3.5) * k;
                let c = base + Vec3::Y * 6.8 * k;
                self.blob(c, Vec3::new(r, r * 1.1, r), 0.18, 7, &mut rng, (leaf, leaf_uv(c - Vec3::Y * r * 1.1, r * 2.2)));
            }
            Species::Pine => {
                self.prism(base, 0.3 * k, 0.2 * k, 3.0 * k, 6, (wood, wood_uv(base, 3.0 * k)));
                for (y, r, h) in [(2.4, 3.2, 5.0), (5.2, 2.5, 4.5), (8.0, 1.7, 4.0)] {
                    let at = base + Vec3::Y * y * k;
                    self.cone(at, r * k, h * k, 8, (leaf, leaf_uv(at, h * k)));
                }
            }
            Species::Bush => {
                let r = rng.range(1.3, 1.8) * k;
                let c = base + Vec3::Y * 0.6 * k;
                self.blob(c, Vec3::new(r, r * 0.75, r), 0.2, 7, &mut rng, (leaf, leaf_uv(c - Vec3::Y * r * 0.75, r * 1.5)));
            }
            Species::Rock => {
                let (rx, ry, rz) = (rng.range(1.4, 2.2) * k, rng.range(0.8, 1.2) * k, rng.range(1.2, 1.9) * k);
                let c = base + Vec3::Y * 0.4 * k;
                self.blob(c, Vec3::new(rx, ry, rz), 0.3, 6, &mut rng, (leaf, leaf_uv(c - Vec3::Y * ry, ry * 2.0)));
            }
        }
    }

    fn piece(&mut self, p: &Piece) {
        let s = if p.uv_scale > 0.0 { p.uv_scale } else { DEFAULT_UV_SCALE };
        let layer = if matches!(p.shape, Shape::Model { .. } | Shape::Cloud { .. } | Shape::Object { .. } | Shape::Stamp { .. }) { -1.0 } else { self.layer(&p.texture) };
        let view = |v: [f32; 3]| Vec3::new(v[0], v[1], -v[2]);
        match &p.shape {
            Shape::Ground { points } => {
                let pts: Vec<Vec3> = points.iter().map(|&([x, z], y)| Vec3::new(x, y.unwrap_or_else(|| self.w.ground(x, -z)), -z)).collect();
                let flat: Vec<[f32; 2]> = pts.iter().map(|v| [v.x, v.z]).collect();
                for t in triangulate(&flat) {
                    let v = t.map(|i| pts[i]);
                    self.tri(v, v.map(|q| Self::flat(q, s)), layer, Some(Vec3::Y));
                }
            }
            Shape::Block { centre, size, yaw, pitch, roll, roof } => {
                let rot = block_rotation(*yaw, *pitch, *roll);
                let c = view(*centre);
                // game-space rotation of the local offset, then z negated into view space
                let local = |v: Vec3| {
                    let r = rot * v;
                    Vec3::new(r.x, r.y, -r.z)
                };
                let up = local(Vec3::Y);
                let corner = |sx: f32, sz: f32, y: f32| c + local(Vec3::new(sx * size[0] / 2.0, y, sz * size[2] / 2.0));
                let ring = [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)];
                let middle = c + up * size[1] / 2.0;
                for k in 0..4 {
                    let (a, b) = (ring[k], ring[(k + 1) % 4]);
                    let (pa, pb) = (corner(a.0, a.1, 0.0), corner(b.0, b.1, 0.0));
                    let mid = (pa + pb) / 2.0 + up * size[1] / 2.0;
                    self.quad([pa, pb, pb + up * size[1], pa + up * size[1]], Self::upright(pa, pb, size[1], s), layer, Some(mid - middle));
                }
                let top: [Vec3; 4] = ring.map(|(x, z)| corner(x, z, size[1]));
                let rl = roof.as_deref().map_or(layer, |r| self.layer(r));
                self.quad(top, top.map(|q| Self::flat(q, s)), rl, Some(up));
            }
            Shape::Wall { from, to, height } => {
                let (a, b) = (view(*from), view(*to));
                self.quad([a, b, b + Vec3::Y * height, a + Vec3::Y * height], Self::upright(a, b, *height, s), layer, None);
            }
            Shape::Hill { points, height, width, base } => {
                // ridge points on the ground + height, or at their pinned y; the slopes run out to ±width/2 on the ground
                // + base (design choice: Base offsets the feet only, the crest stays ground + Height)
                let r: Vec<Vec3> = points.iter().map(|&([x, z], y)| Vec3::new(x, y.unwrap_or_else(|| self.w.ground(x, -z) + height), -z)).collect();
                self.crest = r.iter().map(|v| v.y).collect();
                let mut side = Vec::new();
                for i in 0..r.len() {
                    let seg = |a: usize, b: usize| (r[b] - r[a]).with_y(0.0).normalize_or_zero();
                    let d = match i {
                        0 => seg(0, 1),
                        i if i + 1 == r.len() => seg(i - 1, i),
                        i => (seg(i - 1, i) + seg(i, i + 1)).normalize_or_zero(),
                    };
                    let perp = Vec3::new(-d.z, 0.0, d.x) * width / 2.0;
                    let base = |sgn: f32| {
                        let q = r[i] + perp * sgn;
                        Vec3::new(q.x, self.w.ground(q.x, q.z) + base, q.z)
                    };
                    side.push((base(-1.0), base(1.0)));
                }
                // UVs follow the surface (u along the crest, v up the slope, both in metres on the slope, 0 on the
                // crest) so a steep or raised hill is not smeared by a top-down projection
                let mut along = vec![0.0f32];
                for i in 1..r.len() {
                    along.push(along[i - 1] + r[i].distance(r[i - 1]));
                }
                let drop = |i: usize| (r[i].distance(side[i].0), r[i].distance(side[i].1));
                for i in 0..r.len() - 1 {
                    let (j, up) = (i + 1, Vec3::Y);
                    let (a, b) = (along[i] / s, along[j] / s);
                    let ((li, ri), (lj, rj)) = (drop(i), drop(j));
                    let l = [side[i].0, side[j].0, r[j], r[i]];
                    let rt = [r[i], r[j], side[j].1, side[i].1];
                    self.quad(l, [[a, -li / s], [b, -lj / s], [b, 0.0], [a, 0.0]], layer, Some(up));
                    self.quad(rt, [[a, 0.0], [b, 0.0], [b, rj / s], [a, ri / s]], layer, Some(up));
                }
            }
            Shape::Patch { verts } => {
                let v: Vec<Vec3> = verts.iter().map(|&q| view(q)).collect();
                let uv: Vec<[f32; 2]> = v.iter().map(|&q| Self::flat(q, s)).collect();
                if v.len() == 4 {
                    self.quad([v[0], v[1], v[2], v[3]], [uv[0], uv[1], uv[2], uv[3]], layer, None);
                } else {
                    self.tri([v[0], v[1], v[2]], [uv[0], uv[1], uv[2]], layer, None);
                }
            }
            Shape::Plant { species, centre, scale, variant, yaw, trunk } => {
                let trunk_name = trunk.as_deref().unwrap_or(&p.texture);
                let trunk_layer = trunk.as_deref().map_or(layer, |t| self.layer(t));
                let tex = [(layer, plant_tex(&p.texture).and_then(|t| t.leaf)), (trunk_layer, plant_tex(trunk_name).and_then(|t| t.trunk))];
                let base = view(*centre);
                // game yaw about +Y is the opposite turn in view space (z negated)
                self.turn = (*yaw != 0.0).then(|| (base, Quat::from_rotation_y(-yaw.to_radians())));
                self.plant(*species, base, *scale, *variant, tex, s);
                self.turn = None;
            }
            Shape::Model { name, centre, scale, yaw } => {
                let Some(first) = self.pack_layer else {
                    if self.missing.insert("<tree pack>".into()) {
                        eprintln!("fill: the tree pack's texture layers are not in this scene, model pieces skipped");
                    }
                    return;
                };
                let base = view(*centre);
                // the ground's light under it and round its crown (world and fill ground); off the ground,
                // the nearest world vertex's
                let c = self.w.colour(base + Vec3::Y * 0.1, Vec3::Y);
                let g = self.light.as_ref();
                let ground = |p: Vec3| g.and_then(|g| g.at(p));
                let light = super::trees::Light { ground: &ground, fallback: [c[0], c[1], c[2]], track: self.tree_match.as_ref() };
                // game yaw about +Y is the opposite turn in view space (z negated)
                let sun = self.w.look.sun_dir;
                if !super::trees::place(name, base, *scale, -yaw.to_radians(), &light, sun, first, &mut self.out) && self.missing.insert(format!("<model {name}>")) {
                    eprintln!("fill: no model '{name}' in the tree pack");
                }
            }
            Shape::Object { name, centre, scale, yaw } => {
                let scene = self.scene;
                let Some((_, mesh)) = object_list(&scene.prop_names, &scene.props).find(|(n, _)| n.eq_ignore_ascii_case(name)) else {
                    if self.missing.insert(format!("<object {name}>")) {
                        eprintln!("fill: no scene object '{name}' on this track, piece skipped");
                    }
                    return;
                };
                // game yaw about +Y is the opposite turn in view space (z negated)
                let tex_face = scene.object_scale.map(|k| k * PROP_TEX_FACE);
                let w = &self.w;
                place_object(mesh, view(*centre), *scale, -yaw.to_radians(), tex_face, &|p, n| w.colour(p, n), &mut self.out);
            }
            Shape::Stamp { name, centre, scale, yaw } => {
                let (scene, stamps) = (self.scene, self.stamps);
                let cut = self.cut.entry(name.to_ascii_lowercase()).or_insert_with(|| {
                    // APPROX: what the Remaster world draw leaves out is not copied (water the surface replaces, tree
                    // cards the models replace)
                    let hidden = |i: usize| scene.water_hide.get(i).copied().unwrap_or(false) || scene.tree_hide.get(i).copied().unwrap_or(false);
                    stamps.iter().find(|st| st.name.eq_ignore_ascii_case(name)).map(|st| stamp_tris(&scene.tris, &hidden, st))
                });
                let Some(local) = cut else {
                    if self.missing.insert(format!("<stamp {name}>")) {
                        eprintln!("fill: no Stamp '{name}' in the fill file, piece skipped");
                    }
                    return;
                };
                place_stamp(local, Vec3::from(*centre), *scale, *yaw, &mut self.out);
            }
            Shape::Skirt { radius, drop } => {
                let (o, c) = outline(self.w.all);
                if o.len() < 3 {
                    return;
                }
                let min_y = self.w.all.iter().map(|v| v.pos[1]).fold(f32::MAX, f32::min);
                let k = Skirt::new(o, c, *radius, *drop, min_y);
                let rings = k.rings();
                for i in 0..rings.len() {
                    let (a, b) = (&rings[i], &rings[(i + 1) % rings.len()]);
                    for r in 0..3 {
                        let q = [a[r], b[r], b[r + 1], a[r + 1]];
                        self.quad(q, q.map(|v| Self::flat(v, s)), layer, Some(Vec3::Y));
                    }
                }
                self.skirt = Some((k.centre, min_y));
                self.w.skirt = Some(k);
            }
            // drawn by the fog shaders, not meshed
            Shape::Cloud { .. } => {}
        }
    }
}

// ---- 12.4 automatic pass ----

/// Welded-vertex key: a 1 cm grid.
fn weld(p: [f32; 3]) -> (i32, i32, i32) {
    let q = |v: f32| (v * 100.0).round() as i32;
    (q(p[0]), q(p[1]), q(p[2]))
}

fn shoelace(l: &[Vec3]) -> f32 {
    (0..l.len()).map(|i| { let (a, b) = (l[i], l[(i + 1) % l.len()]); a.x * b.z - b.x * a.z }).sum::<f32>() / 2.0
}

fn hull_area(pts: &[Vec3]) -> f32 {
    let mut p: Vec<(f32, f32)> = pts.iter().map(|v| (v.x, v.z)).collect();
    p.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    p.dedup();
    if p.len() < 3 {
        return 0.0;
    }
    let cr = |o: (f32, f32), a: (f32, f32), b: (f32, f32)| (a.0 - o.0) * (b.1 - o.1) - (a.1 - o.1) * (b.0 - o.0);
    let mut h: Vec<(f32, f32)> = Vec::new();
    for pass in 0..2 {
        let start = h.len();
        for &q in if pass == 0 { p.clone() } else { p.iter().rev().copied().collect() }.iter() {
            while h.len() >= start + 2 && cr(h[h.len() - 2], h[h.len() - 1], q) <= 0.0 {
                h.pop();
            }
            h.push(q);
        }
        h.pop();
    }
    let v: Vec<Vec3> = h.iter().map(|&(x, z)| Vec3::new(x, 0.0, z)).collect();
    shoelace(&v).abs()
}

/// Edges used by one triangle only, owned by a near-horizontal face (view space).
pub fn open_edges(tris: &[Vertex]) -> Vec<[Vec3; 2]> {
    let mut ids: HashMap<(i32, i32, i32), u32> = HashMap::new();
    let mut pos: Vec<Vec3> = Vec::new();
    let mut count: HashMap<(u32, u32), (u32, bool)> = HashMap::new();
    for t in tris.as_chunks::<3>().0.iter() {
        let p = [0, 1, 2].map(|k| Vec3::from(t[k].pos));
        let n = (p[1] - p[0]).cross(p[2] - p[0]).normalize_or_zero();
        let flat = n.y.abs() > 0.7;
        let id = [0, 1, 2].map(|k| *ids.entry(weld(t[k].pos)).or_insert_with(|| { pos.push(p[k]); pos.len() as u32 - 1 }));
        for k in 0..3 {
            let (a, b) = (id[k], id[(k + 1) % 3]);
            if a == b {
                continue;
            }
            let e = count.entry((a.min(b), a.max(b))).or_insert((0, false));
            e.0 += 1;
            e.1 |= flat;
        }
    }
    count.into_iter().filter(|(_, (n, flat))| *n == 1 && *flat).map(|((a, b), _)| [pos[a as usize], pos[b as usize]]).collect()
}

/// The largest closed loop of open edges.
fn largest_loop(edges: &[[Vec3; 2]]) -> Vec<Vec3> {
    let mut ids: HashMap<(i32, i32, i32), usize> = HashMap::new();
    let mut pts: Vec<Vec3> = Vec::new();
    let mut adj: HashMap<usize, Vec<usize>> = HashMap::new();
    for e in edges {
        let id = e.map(|p| *ids.entry(weld(p.into())).or_insert_with(|| { pts.push(p); pts.len() - 1 }));
        adj.entry(id[0]).or_default().push(id[1]);
        adj.entry(id[1]).or_default().push(id[0]);
    }
    let mut used: HashSet<(usize, usize)> = HashSet::new();
    let mut best: Vec<Vec3> = Vec::new();
    let mut best_area = 0.0;
    for e in edges {
        let (s, mut cur) = (ids[&weld(e[0].into())], ids[&weld(e[1].into())]);
        if used.contains(&(s.min(cur), s.max(cur))) {
            continue;
        }
        used.insert((s.min(cur), s.max(cur)));
        let (mut path, mut prev) = (vec![s], s);
        while cur != s {
            path.push(cur);
            let next = adj[&cur].iter().copied().find(|&n| n != prev && !used.contains(&(n.min(cur), n.max(cur))));
            let Some(n) = next else { break };
            used.insert((n.min(cur), n.max(cur)));
            (prev, cur) = (cur, n);
        }
        if cur == s && path.len() >= 3 {
            let l: Vec<Vec3> = path.iter().map(|&i| pts[i]).collect();
            let a = shoelace(&l).abs();
            if a > best_area {
                (best, best_area) = (l, a);
            }
        }
    }
    best
}

/// Sector-wise farthest vertex around `c` [I]: the fallback when no clean outer loop exists.
fn radial_outline(tris: &[Vertex], c: Vec3) -> Vec<Vec3> {
    const N: usize = 128;
    let mut far: [Option<Vec3>; N] = [None; N];
    for v in tris {
        let p = Vec3::from(v.pos);
        let a = (p.z - c.z).atan2(p.x - c.x);
        let k = (((a / std::f32::consts::TAU + 0.5) * N as f32) as usize).min(N - 1);
        if far[k].is_none_or(|q| (q.x - c.x).hypot(q.z - c.z) < (p.x - c.x).hypot(p.z - c.z)) {
            far[k] = Some(p);
        }
    }
    let mut out = Vec::new();
    for k in 0..N {
        let p = far[k].or_else(|| {
            let (prev, next) = ((1..N).find_map(|d| far[(k + N - d) % N]), (1..N).find_map(|d| far[(k + d) % N]));
            prev.zip(next).map(|(a, b)| (a + b) / 2.0)
        });
        out.extend(p);
    }
    out
}

/// The world's outer boundary in view space (closed, ~4 m spacing, at most 256 points, smoothed
/// in XZ), plus its centre. Empty when the world has no vertices.
pub fn outline(tris: &[Vertex]) -> (Vec<Vec3>, Vec3) {
    if tris.is_empty() {
        return (Vec::new(), Vec3::ZERO);
    }
    let all: Vec<Vec3> = tris.iter().map(|v| Vec3::from(v.pos)).collect();
    let centre = all.iter().sum::<Vec3>() / all.len() as f32;
    let l = largest_loop(&open_edges(tris));
    let l = if l.len() >= 3 && shoelace(&l).abs() >= 0.5 * hull_area(&all) { l } else { radial_outline(tris, centre) };
    if l.len() < 3 {
        return (Vec::new(), centre);
    }
    let seg: Vec<f32> = (0..l.len()).map(|i| (l[(i + 1) % l.len()] - l[i]).with_y(0.0).length()).collect();
    let total: f32 = seg.iter().sum();
    let n = ((total / 4.0) as usize).clamp(8, 256);
    let step = total / n as f32;
    let (mut o, mut i, mut run) = (Vec::with_capacity(n), 0, 0.0);
    for k in 0..n {
        let d = k as f32 * step;
        while i + 1 < l.len() && run + seg[i] < d {
            run += seg[i];
            i += 1;
        }
        let t = if seg[i] > 1e-6 { ((d - run) / seg[i]).clamp(0.0, 1.0) } else { 0.0 };
        o.push(l[i].lerp(l[(i + 1) % l.len()], t));
    }
    for _ in 0..2 {
        let src = o.clone();
        for k in 0..n {
            let (a, b) = (src[(k + n - 1) % n], src[(k + 1) % n]);
            let m = (a + src[k] + b) / 3.0;
            o[k] = Vec3::new(m.x, src[k].y, m.z);
        }
    }
    let c = o.iter().sum::<Vec3>() / n as f32;
    (o, Vec3::new(c.x, centre.y, c.z))
}

/// The skirt: the outline fanned out to `radius` from `centre`, sinking to `floor`.
pub struct Skirt {
    pub outline: Vec<Vec3>,
    pub centre: Vec3,
    pub radius: f32,
    pub floor: f32,
}

const RING_T: [f32; 4] = [0.0, 0.05, 0.3, 1.0];

fn smooth(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

impl Skirt {
    pub fn new(outline: Vec<Vec3>, centre: Vec3, radius: f32, drop: f32, min_y: f32) -> Skirt {
        Skirt { outline, centre, radius, floor: min_y - drop }
    }
    fn reach(&self, p: Vec3) -> f32 {
        (self.radius - (p - self.centre).with_y(0.0).length()).max(1.0)
    }
    /// Height of the skirt surface at `(x, z)`, from the nearest outline point.
    pub fn height(&self, x: f32, z: f32) -> f32 {
        let Some(p) = self.outline.iter().min_by(|a, b| {
            let d = |p: &&Vec3| (p.x - x).powi(2) + (p.z - z).powi(2);
            d(a).total_cmp(&d(b))
        }) else { return self.floor };
        let d = (p.x - x).hypot(p.z - z);
        let s = smooth(d / self.reach(*p));
        (p.y - 0.05) * (1.0 - s) + self.floor.min(p.y) * s
    }
    /// Per outline point: the four ring positions (view space), inner to outer.
    fn rings(&self) -> Vec<[Vec3; 4]> {
        self.outline.iter().map(|&p| {
            let dir = (p - self.centre).with_y(0.0).normalize_or_zero();
            let ext = self.reach(p);
            RING_T.map(|t| {
                let s = smooth(t);
                let q = p + dir * ext * t;
                Vec3::new(q.x, (p.y - 0.05) * (1.0 - s) + self.floor.min(p.y) * s, q.z)
            })
        }).collect()
    }
}

/// Point in polygon (even-odd) in XZ.
pub fn inside(poly: &[Vec3], x: f32, z: f32) -> bool {
    let mut c = false;
    for i in 0..poly.len() {
        let (a, b) = (poly[i], poly[(i + 1) % poly.len()]);
        if (a.z > z) != (b.z > z) && x < a.x + (z - a.z) / (b.z - a.z) * (b.x - a.x) {
            c = !c;
        }
    }
    c
}

/// Tiny deterministic generator (xorshift32) so a seed always gives the same skyline.
struct Rng(u32);

impl Rng {
    fn next(&mut self) -> f32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 17;
        self.0 ^= self.0 << 5;
        (self.0 >> 8) as f32 / (1u32 << 24) as f32
    }
    fn range(&mut self, a: f32, b: f32) -> f32 {
        a + (b - a) * self.next()
    }
}

/// Texture usage of the world: the ground, wall and roof textures and whether it is a city.
pub struct Usage {
    pub ground: String,
    pub wall: String,
    pub roof: String,
    pub city: bool,
}

/// APPROX: textures are chosen by what the world does with them (the names live in the track's
/// own set and differ per track): ground = most flat area near the outline, wall = most upright
/// area, roof = most flat area above the ground. City when upright area is over 35 % of flat [I].
pub fn usage(s: &Scene, outline: &[Vec3]) -> Usage {
    let names = &s.world_textures;
    let mut flat_edge = vec![0.0f32; names.len()];
    let (mut flat, mut upright) = (vec![0.0f32; names.len()], vec![0.0f32; names.len()]);
    let low = outline.iter().map(|p| p.y).fold(f32::MAX, f32::min);
    let mut roofs = vec![0.0f32; names.len()];
    for (i, t) in s.tris.as_chunks::<3>().0.iter().enumerate() {
        let Some(&ti) = s.world_tri_texture.get(i) else { continue };
        let ti = ti as usize;
        if ti >= names.len() {
            continue;
        }
        let p = [0, 1, 2].map(|k| Vec3::from(t[k].pos));
        let c = (p[0] + p[1] + p[2]) / 3.0;
        let cr = (p[1] - p[0]).cross(p[2] - p[0]);
        let area = cr.length() / 2.0;
        let n = cr.normalize_or_zero();
        if n.y.abs() > 0.7 {
            flat[ti] += area;
            if c.y > low + 6.0 {
                roofs[ti] += area;
            }
            if outline.iter().any(|o| (o.x - c.x).powi(2) + (o.z - c.z).powi(2) < 900.0) {
                flat_edge[ti] += area;
            }
        } else if n.y.abs() < 0.3 {
            upright[ti] += area;
        }
    }
    let top = |v: &[f32]| v.iter().enumerate().filter(|(_, &a)| a > 0.0).max_by(|a, b| a.1.total_cmp(b.1)).map(|(i, _)| names[i].clone());
    let ground = top(&flat_edge).or_else(|| top(&flat)).unwrap_or_default();
    let wall = top(&upright).unwrap_or_else(|| ground.clone());
    let roof = top(&roofs).unwrap_or_else(|| wall.clone());
    let city = upright.iter().sum::<f32>() > 0.35 * flat.iter().sum::<f32>();
    Usage { ground, wall, roof, city }
}

/// The automatic pass: a skirt over the void past the world edge and a seeded skyline ring, as
/// ordinary pieces to edit or delete (12.4). Empty when the scene has no world.
pub fn auto(s: &Scene, seed: u32) -> FillFile {
    let (o, c) = outline(&s.tris);
    if o.is_empty() {
        return FillFile { seed, pieces: Vec::new(), ..Default::default() };
    }
    let u = usage(s, &o);
    eprintln!("fill auto: {} (ground {}, wall {}, roof {})", if u.city { "city" } else { "open" }, u.ground, u.wall, u.roof);
    let reach = o.iter().map(|p| (*p - c).with_y(0.0).length()).fold(0.0, f32::max);
    let radius = (s.view_distance * crate::render::View::MAX_DISTANCE + 200.0).max(reach + 200.0);
    let min_y = s.tris.iter().map(|v| v.pos[1]).fold(f32::MAX, f32::min);
    let drop = 30.0;
    eprintln!("fill auto: outline {} points, centre ({:.0}, {:.0}, {:.0}), reach {reach:.0} m, skirt radius {radius:.0} m", o.len(), c.x, c.y, -c.z);
    let skirt = Skirt::new(o, c, radius, drop, min_y);
    let mut pieces = vec![Piece { shape: Shape::Skirt { radius, drop }, texture: u.ground.clone(), uv_scale: 16.0 }];
    let mut rng = Rng(seed.wrapping_mul(2_654_435_761).max(1));
    let game = |v: Vec3, y: f32| [v.x, y, -v.z];
    // skyline stays outside the world and beyond the track's own view distance, so it never shows
    // at the original range (the world may be thin or concave: a ring about the centre can cross it)
    let gap = s.view_distance.max(40.0);
    let near_edge = |x: f32, z: f32| inside(&skirt.outline, x, z) || skirt.outline.iter().any(|p| (p.x - x).hypot(p.z - z) < gap);
    if u.city {
        for f in [0.62, 0.74, 0.86] {
            let mut a = rng.range(0.0, 1.0);
            while a < std::f32::consts::TAU {
                let r = radius * (f + rng.range(-0.03, 0.03));
                let p = c + Vec3::new(a.cos() * r, 0.0, a.sin() * r);
                let step = rng.range(6.0, 10.0).to_radians();
                a += step;
                if near_edge(p.x, p.z) {
                    continue;
                }
                let base = skirt.height(p.x, p.z) - 2.0;
                let gz = -p.z;
                let yaw = (p.x - c.x).atan2(gz + c.z).to_degrees() + rng.range(-15.0, 15.0);
                let size = [rng.range(20.0, 60.0), rng.range(25.0, 120.0), rng.range(20.0, 60.0)];
                pieces.push(Piece { shape: Shape::Block { centre: game(p, base), size, yaw, pitch: 0.0, roll: 0.0, roof: Some(u.roof.clone()) }, texture: u.wall.clone(), uv_scale: 8.0 });
            }
        }
    } else {
        for _ in 0..rng.range(8.0, 15.0) as usize {
            let (a0, span) = (rng.range(0.0, std::f32::consts::TAU), rng.range(15.0, 35.0).to_radians());
            let r = radius * rng.range(0.6, 0.9);
            let arc = |k: usize| {
                let (a, rr) = (a0 + span * k as f32 / 4.0, r);
                c + Vec3::new(a.cos() * rr, 0.0, a.sin() * rr)
            };
            if (0..5).any(|k| near_edge(arc(k).x, arc(k).z)) {
                continue;
            }
            let points: Vec<([f32; 2], Option<f32>)> = (0..5).map(|k| {
                let (a, rr) = (a0 + span * k as f32 / 4.0, r * (1.0 + rng.range(-0.03, 0.03)));
                let p = c + Vec3::new(a.cos() * rr, 0.0, a.sin() * rr);
                ([p.x, -p.z], None)
            }).collect();
            let height = rng.range(40.0, 150.0);
            pieces.push(Piece { shape: Shape::Hill { points, height, width: height * rng.range(3.0, 5.0), base: 0.0 }, texture: u.wall.clone(), uv_scale: 16.0 });
        }
    }
    FillFile { seed, pieces, ..Default::default() }
}

/// `count` points over a disc of `radius` round `centre` (game space), each with a random scale factor
/// (0.7 to 1.3; 1 for a single click, which also stays on the click) and its ground height.
fn disc(centre: Vec3, radius: f32, count: usize, seed: u32, scene: &Scene, mut each: impl FnMut(usize, [f32; 3], f32, &mut Rng) -> Shape) -> Vec<Shape> {
    let world = (count > 1).then(|| World::new(scene));
    let mut rng = Rng(seed.wrapping_mul(2_246_822_519) | 1);
    (0..count)
        .map(|i| {
            // sqrt for an even density over the disc; the first plant of a single click stays on the click
            let (a, d) = (rng.range(0.0, std::f32::consts::TAU), if count == 1 { 0.0 } else { radius * rng.next().sqrt() });
            let (x, z) = (centre.x + a.cos() * d, centre.z + a.sin() * d);
            let y = world.as_ref().map_or(centre.y, |w| w.ground(x, -z));
            let k = if count == 1 { 1.0 } else { rng.range(0.7, 1.3) };
            each(i, [x, y, z], k, &mut rng)
        })
        .collect()
}

/// `count` plants of `species` scattered over a disc of `radius` round `centre` (game space) with
/// random scale (0.7 to 1.3 of `scale`), variant and yaw; bases follow the world's ground (a single plant stays at `centre`).
pub fn scatter(species: Species, centre: Vec3, radius: f32, count: usize, scale: f32, seed: u32, scene: &Scene) -> Vec<Shape> {
    disc(centre, radius, count, seed, scene, |i, c, k, rng| Shape::Plant { species, centre: c, scale: scale * k, variant: seed.wrapping_add(i as u32).wrapping_mul(97), yaw: rng.range(0.0, 360.0), trunk: None })
}

/// `count` pack models scattered like `scatter`, each turned by a random yaw; scale x 0.8 to 1.2 for a
/// scatter, `scale` as is for a single click (#390). `groups` are the brush's ticked entries (a family is
/// its members, a model is itself): each model picks one group at random (equally likely), then one member
/// of it. A single group draws exactly as a single choice did.
pub fn scatter_models(groups: &[Vec<&str>], centre: Vec3, radius: f32, count: usize, scale: f32, seed: u32, scene: &Scene) -> Vec<Shape> {
    let groups: Vec<&Vec<&str>> = groups.iter().filter(|g| !g.is_empty()).collect();
    if groups.is_empty() {
        return Vec::new();
    }
    disc(centre, radius, count, seed, scene, |_, c, _, rng| {
        let g = if groups.len() > 1 { groups[(rng.next() * groups.len() as f32) as usize % groups.len()] } else { groups[0] };
        let name = g[(rng.next() * g.len() as f32) as usize % g.len()];
        let k = if count == 1 { 1.0 } else { rng.range(0.8, 1.2) };
        Shape::Model { name: name.to_string(), centre: c, scale: scale * k, yaw: rng.range(0.0, 360.0) }
    })
}

/// The world's ground height under each game-space (x, z) point.
pub fn ground_heights(scene: &Scene, pts: &[[f32; 2]]) -> Vec<f32> {
    let w = World::new(scene);
    pts.iter().map(|&[x, z]| w.ground(x, -z)).collect()
}

/// The fill's triangles in view space, world-style (baked-looking colour, plane normals, env 0).
pub fn mesh(f: &FillFile, s: &Scene) -> Vec<Vertex> {
    mesh_pieces(f, s).verts
}

/// The fill mesh with what the editor needs to pick it (REMASTER World Editor, 12.11).
pub struct FillMesh {
    pub verts: Vec<Vertex>,
    /// per piece (index into `FillFile::pieces`): its vertex range in `verts`
    pub pieces: Vec<(usize, std::ops::Range<usize>)>,
    /// the skirt's centre (view space) and the world's lowest y; None without a skirt piece
    pub skirt: Option<(Vec3, f32)>,
    /// per hill piece (index into `FillFile::pieces`): the ridge height at each of its points, pinned or draped
    pub hills: Vec<(usize, Vec<f32>)>,
}

pub fn mesh_pieces(f: &FillFile, s: &Scene) -> FillMesh {
    if f.pieces.is_empty() {
        return FillMesh { verts: Vec::new(), pieces: Vec::new(), skirt: None, hills: Vec::new() };
    }
    // texture name -> layer, from the world's own triangles
    let mut layers = HashMap::new();
    for (i, &t) in s.world_tri_texture.iter().enumerate() {
        if let (Some(name), Some(v)) = (s.world_textures.get(t as usize), s.tris.get(i * 3)) {
            if v.layer >= 0.0 {
                layers.entry(key(name)).or_insert(v.layer);
            }
        }
    }
    let mut m = Mesher { w: World::new(s), scene: s, stamps: &f.stamps, cut: HashMap::new(), layers, missing: HashSet::new(), out: Vec::new(), pack_layer: s.tree_pack_layer, light: None, tree_match: None, skirt: None, crest: Vec::new(), turn: None };
    // skirts first: draped pieces outside the world sit on them; models last: they take the light of the
    // ground pieces under them
    let (skirts, rest): (Vec<_>, Vec<_>) = f.pieces.iter().enumerate().partition(|(_, p)| matches!(p.shape, Shape::Skirt { .. }));
    let (models, rest): (Vec<_>, Vec<_>) = rest.into_iter().partition(|(_, p)| matches!(p.shape, Shape::Model { .. }));
    let mut pieces = Vec::with_capacity(f.pieces.len());
    let mut hills = Vec::new();
    for (i, p) in skirts.into_iter().chain(rest) {
        let start = m.out.len();
        m.piece(p);
        if matches!(p.shape, Shape::Hill { .. }) {
            hills.push((i, std::mem::take(&mut m.crest)));
        }
        pieces.push((i, start..m.out.len()));
    }
    if !models.is_empty() && m.pack_layer.is_some() {
        let g = GroundLight::new(s.tris.as_chunks::<3>().0.iter().chain(m.out.as_chunks::<3>().0));
        m.tree_match = super::trees::calibrate(&s.tree_sites, &s.layers, &|p| g.at(p));
        m.light = Some(g);
    }
    for (i, p) in models {
        let start = m.out.len();
        m.piece(p);
        pieces.push((i, start..m.out.len()));
    }
    FillMesh { verts: m.out, pieces, skirt: m.skirt, hills }
}

/// The vertices of `mesh` without the pieces `hidden` names (by index into `FillFile::pieces`), and their ranges re-based to
/// the shorter list: what the editor draws and meshes grass over while an eye or a layer hides pieces (#477).
pub fn without_hidden(mesh: &FillMesh, hidden: &dyn Fn(usize) -> bool) -> (Vec<Vertex>, Vec<(usize, std::ops::Range<usize>)>) {
    let mut verts = Vec::with_capacity(mesh.verts.len());
    let mut pieces = Vec::with_capacity(mesh.pieces.len());
    for (i, r) in &mesh.pieces {
        if hidden(*i) {
            continue;
        }
        let start = verts.len();
        verts.extend_from_slice(&mesh.verts[r.clone()]);
        pieces.push((*i, start..verts.len()));
    }
    (verts, pieces)
}

/// A bare textured vertex at `pos` for tests (here and in the editor's).
#[cfg(test)]
pub(crate) fn test_vertex(pos: [f32; 3]) -> Vertex {
    Vertex { pos, uv: [0.0; 2], color: [1.0; 4], layer: 0.0, normal: [0.0, 1.0, 0.0], env: 0.0 }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plant_table_is_findable_and_unique() {
        assert_eq!(plant_tex("TREE3.IFF").map(|t| t.species), Some(Species::Tree));
        assert_eq!(plant_tex("buske").map(|t| t.species), Some(Species::Bush));
        assert!(plant_tex("gras2.iff").is_none());
        assert_eq!(plant_tex("Barne.iff").map(|t| t.species), Some(Species::Rock));
        let mut names: Vec<String> = PLANT_TEXTURES.iter().map(|t| key(t.name)).collect();
        names.sort();
        names.dedup();
        assert_eq!(names.len(), PLANT_TEXTURES.len());
        for t in PLANT_TEXTURES {
            for r in t.leaf.iter().chain(&t.trunk) {
                assert!(r[0] < r[2] && r[1] < r[3] && r.iter().all(|v| (0.0..=1.0).contains(v)), "{}", t.name);
            }
        }
    }

    #[test]
    fn crop_uv_stays_in_the_rect() {
        let uv = Uv::Crop { rect: [0.2, 0.1, 0.6, 0.5], base: Vec3::ZERO, height: 2.0 };
        for q in [Vec3::new(1.0, 0.0, 0.0), Vec3::new(-1.0, 3.0, 0.5), Vec3::new(0.0, 1.0, -1.0)] {
            let [u, v] = uv.at(q);
            assert!((0.2 - 1e-5..=0.6 + 1e-5).contains(&u) && (0.1 - 1e-5..=0.5 + 1e-5).contains(&v), "{u} {v}");
        }
        // the top of the object is the top of the rect
        assert!((uv.at(Vec3::new(1.0, 2.0, 0.0))[1] - 0.1).abs() < 1e-5);
    }

    const SAMPLE: &str = "# c\nSeed 7\nPiece ground Texture GRAS1 UvScale 8 Point 0 0 Point 10 0 2 Point 10 10 Point 0 10\n\
        Piece block Texture WALL3 Roof ROOF1 Centre 1 2 3 Size 4 5 6 Yaw 30\n\
        Piece wall Texture WALL3 From 0 0 0 To 5 0 5 Height 3\n\
        Piece hill Texture ROCK1 Point 0 0 Point 5 5 Height 4 Width 6\n\
        Piece patch Texture WALL3 Vertex 0 0 0 Vertex 1 0 0 Vertex 1 1 0\n\
        Piece skirt Texture GRAS1 Radius 900 Drop 20\n\
        Piece plant Species pine Texture LEAF Trunk BARK Centre 1 2 3 Scale 1.5 Variant 4 Yaw 40\n\
        Piece model Name Tree-01-2 Centre 4 5 6 Scale 1.25 Yaw 90\n\
        Grass Texture Gras1+Grasokk Style verge Height 0.25 Tip 1 1.1 0.8\n";

    #[test]
    fn round_trip() {
        let (f, w) = from_text(SAMPLE);
        assert!(w.is_empty(), "{w:?}");
        assert_eq!(f.pieces.len(), 8);
        assert_eq!(f.pieces[7].shape, Shape::Model { name: "Tree-01-2".into(), centre: [4.0, 5.0, 6.0], scale: 1.25, yaw: 90.0 });
        assert!(matches!(f.pieces[6].shape, Shape::Plant { yaw, variant: 4, .. } if yaw == 40.0));
        assert!(to_text(&f).contains("Variant 4 Yaw 40"));
        // an unturned plant writes no Yaw, an old file reads as 0
        let (old, _) = from_text("Piece plant Species tree Texture A Centre 1 2 3 Scale 1 Variant 2\n");
        assert!(matches!(old.pieces[0].shape, Shape::Plant { yaw, .. } if yaw == 0.0));
        assert!(!to_text(&old).contains("Yaw"));
        assert_eq!(f.seed, 7);
        assert_eq!((f.grass.len(), f.grass[0].params.height, f.grass_none), (1, 0.25, false));
        assert!(from_text("Grass none\n").0.grass_none);
        let (g, w) = from_text(&to_text(&f));
        assert!(w.is_empty());
        assert_eq!(f, g);
    }

    #[test]
    fn hidden_pieces_leave_the_mesh_and_the_ranges_close_up() {
        let v = |x: f32| test_vertex([x, 0.0, 0.0]);
        let mesh = FillMesh { verts: (0..9).map(|n| v(n as f32)).collect(), pieces: vec![(4, 0..3), (1, 3..6), (7, 6..9)], skirt: None, hills: Vec::new() };
        let (verts, pieces) = without_hidden(&mesh, &|i| i == 1);
        assert_eq!(pieces, vec![(4, 0..3), (7, 3..6)]);
        assert_eq!(verts.iter().map(|p| p.pos[0]).collect::<Vec<_>>(), [0.0, 1.0, 2.0, 6.0, 7.0, 8.0]);
        assert_eq!(without_hidden(&mesh, &|_| false).0.len(), 9);
        assert!(without_hidden(&mesh, &|_| true).0.is_empty());
    }

    #[test]
    fn hidden_clouds_are_left_out() {
        let (f, _) = from_text("Piece cloud Centre 1 20 -3 Size 100 30 80 Density 4\nPiece cloud Centre 0 0 0 Size 10 10 10 Density 2\n");
        assert_eq!(clouds(&f).len(), 2);
        assert_eq!(clouds_where(&f, &|i| i != 0).len(), 1);
    }

    #[test]
    fn every_textured_piece_writes_its_uv_scale() {
        // a line without UvScale reads as 4 while the editor makes pieces at 8: the file always says which
        let (f, _) = from_text("Piece wall Texture WALL3 From 0 0 0 To 5 0 5 Height 3\nPiece wall Texture WALL3 UvScale 8 From 0 0 0 To 5 0 5 Height 3\n");
        assert_eq!((f.pieces[0].uv_scale, f.pieces[1].uv_scale), (4.0, 8.0));
        let text = to_text(&f);
        assert_eq!(text.matches("UvScale 4").count(), 1);
        assert_eq!(text.matches("UvScale 8").count(), 1);
        assert_eq!(from_text(&text).0, f);
    }

    #[test]
    fn cloud_round_trips() {
        let (f, w) = from_text("Piece cloud Centre 1 20 -3 Size 100 30 80 Yaw 15 Density 4.5\nPiece cloud Centre 0 0 0 Size 10 10 10 Density 2\nPiece cloud Centre 0 0 0 Size 10 10 10\n");
        assert_eq!(w.len(), 1, "{w:?}");
        assert_eq!(f.pieces[0].shape, Shape::Cloud { centre: [1.0, 20.0, -3.0], size: [100.0, 30.0, 80.0], yaw: 15.0, density: 4.5 });
        let text = to_text(&f);
        assert!(!text.contains("Texture") && text.matches("Yaw").count() == 1, "{text}");
        assert_eq!(from_text(&text).0, f);
        let c = clouds(&f);
        assert_eq!(c.len(), 2);
        assert_eq!(c[0].half, [50.0, 15.0, 40.0]);
    }

    #[test]
    fn cloud_tau_closed_form() {
        // a sphere of radius 50 (density 3 per 100 m) 200 m ahead on view -z (game +z)
        let c = Cloud { centre: [0.0, 10.0, 200.0], half: [50.0; 3], yaw: 0.0, density: 3.0 };
        let rows = c.packed(1.0);
        let o = Vec3::new(0.0, 10.0, 0.0);
        let ahead = Vec3::new(0.0, 0.0, -1.0);
        // through the centre: rho0 x 4R/3
        let full = packed_tau(&rows, o, ahead, 1e7);
        assert!((full - 0.03 * 4.0 * 50.0 / 3.0).abs() < 1e-3, "{full}");
        // stopping at the centre: half of it; before the cloud: none; missing it: none
        assert!((packed_tau(&rows, o, ahead, 200.0) - full * 0.5).abs() < 1e-3);
        assert_eq!(packed_tau(&rows, o, ahead, 140.0), 0.0);
        assert_eq!(packed_tau(&rows, o, Vec3::new(0.0, 0.0, 1.0), 1e7), 0.0);
        assert_eq!(packed_tau(&rows, o, Vec3::Y, 1e7), 0.0);
        // fog strength scales it
        assert!((packed_tau(&c.packed(2.0), o, ahead, 1e7) - 2.0 * full).abs() < 1e-3);
        // a long flat cloud turned 90 degrees: its long axis (game x) now runs along game z, so the view ray along it
        // crosses 2 x 100 m instead of 2 x 10 m
        let flat = Cloud { centre: [0.0, 10.0, 200.0], half: [100.0, 50.0, 10.0], yaw: 0.0, density: 3.0 };
        let turned = Cloud { yaw: 90.0, ..flat };
        let (t0, t1) = (packed_tau(&flat.packed(1.0), o, ahead, 1e7), packed_tau(&turned.packed(1.0), o, ahead, 1e7));
        assert!((t1 / t0 - 10.0).abs() < 1e-2, "{t0} {t1}");
    }

    #[test]
    fn bad_lines_are_skipped() {
        let (f, w) = from_text("Piece ground Point 0 0\nPiece nonsense\nPiece wall From 0 0 0\nPiece patch Vertex 0 0 0 Vertex 1 0 0 Vertex 1 1 0\n");
        assert_eq!(f.pieces.len(), 1);
        assert_eq!(w.len(), 3);
    }

    /// The ground light is the Gouraud colour under a point: interpolated across a quad, the nearest
    /// height wins, walls and points off the ground give none.
    #[test]
    fn hill_point_height_round_trips() {
        let (f, bad) = from_text("Piece hill Texture A Point 0 0 Point 5 5 12.5 Height 4 Width 6\n");
        assert!(bad.is_empty(), "{bad:?}");
        let Shape::Hill { points, .. } = &f.pieces[0].shape else { panic!() };
        assert_eq!((points[0].1, points[1].1), (None, Some(12.5)));
        let text = to_text(&f);
        assert!(text.contains("Point 0 0 Point 5 5 12.5 Height"), "{text}");
        assert_eq!(from_text(&text).0.pieces, f.pieces);
        assert!(!text.contains("Base"), "{text}");
    }

    #[test]
    fn hill_base_round_trips() {
        let (f, bad) = from_text("Piece hill Texture A Point 0 0 Point 5 5 Height 4 Width 6 Base -3\n");
        assert!(bad.is_empty(), "{bad:?}");
        let Shape::Hill { base, .. } = &f.pieces[0].shape else { panic!() };
        assert_eq!(*base, -3.0);
        let text = to_text(&f);
        assert!(text.contains("Width 6 Base -3"), "{text}");
        assert_eq!(from_text(&text).0.pieces, f.pieces);
    }

    #[test]
    fn ground_light_interpolates_under_a_point() {
        let v = |x: f32, y: f32, z: f32, c: f32| Vertex { pos: [x, y, z], uv: [0.0; 2], color: [c, c, c, 1.0], layer: 0.0, normal: [0.0, 1.0, 0.0], env: 0.0 };
        // a 40 m quad dark at x 0, light at x 40; a roof 20 m above it; a wall
        let mut t = vec![v(0.0, 0.0, 0.0, 0.0), v(40.0, 0.0, 0.0, 1.0), v(40.0, 0.0, 40.0, 1.0), v(0.0, 0.0, 0.0, 0.0), v(40.0, 0.0, 40.0, 1.0), v(0.0, 0.0, 40.0, 0.0)];
        t.extend([v(0.0, 20.0, 0.0, 0.5), v(40.0, 20.0, 0.0, 0.5), v(40.0, 20.0, 40.0, 0.5)]);
        t.extend([v(10.0, 0.0, 10.0, 0.9), v(10.0, 9.0, 10.0, 0.9), v(10.0, 9.0, 30.0, 0.9)]);
        let g = GroundLight::new(t.as_chunks::<3>().0.iter());
        assert_eq!(g.tris.len(), 3, "the wall is left out");
        let at = |x, y, z| g.at(Vec3::new(x, y, z)).map(|c| c[0]);
        assert!((at(10.0, 0.0, 20.0).unwrap() - 0.25).abs() < 1e-5);
        assert!((at(30.0, 1.0, 35.0).unwrap() - 0.75).abs() < 1e-5);
        assert_eq!(at(30.0, 19.0, 10.0), Some(0.5));
        assert_eq!(at(30.0, 10.0, 10.0), None, "over 6 m from both");
        assert_eq!(at(50.0, 0.0, 10.0), None);
    }

    #[test]
    fn model_line_defaults_and_needs_a_name() {
        let (f, w) = from_text("Piece model Name Bush-01 Centre 1 2 3\nPiece model Centre 1 2 3\n");
        assert_eq!(f.pieces.len(), 1);
        assert_eq!(f.pieces[0].shape, Shape::Model { name: "Bush-01".into(), centre: [1.0, 2.0, 3.0], scale: 1.0, yaw: 0.0 });
        assert_eq!(w.len(), 1);
        // a written model line has no Texture
        assert!(!to_text(&f).contains("Texture"));
    }

    #[test]
    fn object_line_defaults_and_needs_a_name_and_centre() {
        let (f, w) = from_text("Piece object Name Je_tower Centre 1 2 3\nPiece object Centre 1 2 3\nPiece object Name Je_tower\nPiece object Name AirShipnn Centre -4 30 8 Scale 2.5 Yaw 135\n");
        assert_eq!(f.pieces.len(), 2);
        assert_eq!(w.len(), 2, "{w:?}");
        assert_eq!(f.pieces[0].shape, Shape::Object { name: "Je_tower".into(), centre: [1.0, 2.0, 3.0], scale: 1.0, yaw: 0.0 });
        assert_eq!(f.pieces[1].shape, Shape::Object { name: "AirShipnn".into(), centre: [-4.0, 30.0, 8.0], scale: 2.5, yaw: 135.0 });
        // a written object line has no Texture, and reads back the same
        let text = to_text(&f);
        assert!(!text.contains("Texture") && text.contains("Piece object Name AirShipnn Centre -4 30 8 Scale 2.5 Yaw 135"), "{text}");
        assert_eq!(from_text(&text).0, f);
    }

    #[test]
    fn stamp_lines_round_trip_and_bad_ones_are_skipped() {
        let (f, w) = from_text(concat!(
            "Stamp tower1 Centre 1 2 3 Size 10 40 12 Yaw 30 Floor 0.5\n",
            "Stamp hall Centre -5 0 8 Size 4 5 6\n",
            "Stamp TOWER1 Centre 0 0 0 Size 1 1 1\n",
            "Stamp nobox Centre 0 0 0\n",
            "Piece stamp Name tower1 Centre 400 0 -20 Scale 2 Yaw 90\n",
            "Piece stamp Name hall Centre 1 2 3\n",
            "Piece stamp Centre 1 2 3\n",
        ));
        assert_eq!(w.len(), 3, "{w:?}");
        assert_eq!(f.stamps, [Stamp { name: "tower1".into(), centre: [1.0, 2.0, 3.0], size: [10.0, 40.0, 12.0], yaw: 30.0, floor: 0.5 }, Stamp { name: "hall".into(), centre: [-5.0, 0.0, 8.0], size: [4.0, 5.0, 6.0], yaw: 0.0, floor: 0.0 }]);
        assert_eq!(f.pieces[0].shape, Shape::Stamp { name: "tower1".into(), centre: [400.0, 0.0, -20.0], scale: 2.0, yaw: 90.0 });
        assert_eq!(f.pieces[1].shape, Shape::Stamp { name: "hall".into(), centre: [1.0, 2.0, 3.0], scale: 1.0, yaw: 0.0 });
        let text = to_text(&f);
        assert!(text.contains("Stamp tower1 Centre 1 2 3 Size 10 40 12 Yaw 30 Floor 0.5\n") && text.contains("Stamp hall Centre -5 0 8 Size 4 5 6 Yaw 0\n"), "{text}");
        assert!(text.contains("Piece stamp Name tower1 Centre 400 0 -20 Scale 2 Yaw 90") && !text.contains("Texture"), "{text}");
        assert_eq!(from_text(&text).0, f);
        // a file without stamps writes no Stamp line
        assert!(!to_text(&from_text("Piece object Name A Centre 0 0 0\n").0).contains("Stamp"));
    }

    /// A triangle (view space) with its centroid at game point `g`, 1 m across.
    fn tri_at(g: Vec3) -> [Vertex; 3] {
        let v = |d: Vec3| Vertex { pos: [g.x + d.x, g.y + d.y, -(g.z + d.z)], uv: [0.5, 0.25], color: [0.3, 0.6, 0.9, 1.0], layer: 4.0, normal: [0.0, 0.0, -1.0], env: 8.0 };
        [v(Vec3::new(-0.5, -0.5, 0.0)), v(Vec3::new(0.5, -0.5, 0.0)), v(Vec3::new(0.0, 1.0, 0.0))]
    }

    /// The cut keeps the triangles whose centroid is in the turned box above the floor, in the box's own frame, with
    /// their colour, picture and env; `skip` drops flagged ones.
    #[test]
    fn stamp_tris_cuts_the_turned_box_above_the_floor() {
        // a box 10 wide (x), 20 tall, 4 deep (z), base at (100, 5, 50), turned 90 degrees: its x runs along game -z
        let st = Stamp { name: "b".into(), centre: [100.0, 5.0, 50.0], size: [10.0, 20.0, 4.0], yaw: 90.0, floor: 1.0 };
        let c = Vec3::new(100.0, 5.0, 50.0);
        let at = [
            c + Vec3::new(0.0, 10.0, -4.0), // in: 4 m along the box's x
            c + Vec3::new(0.0, 10.0, 4.0),  // in: -4 m along x
            c + Vec3::new(4.0, 10.0, 0.0),  // out: 4 m along its z (half depth 2)
            c + Vec3::new(0.0, 0.5, 0.0),   // out: under the floor
            c + Vec3::new(0.0, 21.0, 0.0),  // out: over the top
            c + Vec3::new(1.0, 3.0, 1.0),   // in, but skipped
        ];
        let tris: Vec<Vertex> = at.iter().flat_map(|&g| tri_at(g)).collect();
        let cut = stamp_tris(&tris, &|i| i == 5, &st);
        assert_eq!(cut.len(), 6);
        let centroid = |t: &[Vertex]| t.iter().map(|v| Vec3::from(v.pos)).sum::<Vec3>() / 3.0;
        assert!(centroid(&cut[..3]).distance(Vec3::new(4.0, 10.0, 0.0)) < 1e-4, "{}", centroid(&cut[..3]));
        assert!(centroid(&cut[3..]).distance(Vec3::new(-4.0, 10.0, 0.0)) < 1e-4);
        // the normal (game -z... view +z) turns into the box frame; the rest stays
        assert!(Vec3::from(cut[0].normal).distance(Vec3::new(-1.0, 0.0, 0.0)) < 1e-5, "{:?}", cut[0].normal);
        assert!(cut.iter().all(|v| v.color == [0.3, 0.6, 0.9, 1.0] && v.layer == 4.0 && v.uv == [0.5, 0.25] && v.env == 8.0));
    }

    /// A copy: the cut's base on the piece's base, scaled, turned by the piece's yaw, back in view space; the turn of a
    /// copy at the stamp's yaw puts the triangles where they were.
    #[test]
    fn place_stamp_puts_the_cut_back_turned_and_scaled() {
        let st = Stamp { name: "b".into(), centre: [100.0, 5.0, 50.0], size: [10.0, 20.0, 4.0], yaw: 30.0, floor: 0.0 };
        let tris: Vec<Vertex> = [Vec3::new(101.0, 8.0, 50.5), Vec3::new(98.0, 15.0, 49.0)].iter().flat_map(|&g| tri_at(g)).collect();
        let cut = stamp_tris(&tris, &|_| false, &st);
        assert_eq!(cut.len(), 6);
        // at the stamp's own place and yaw: the source again
        let mut back = Vec::new();
        place_stamp(&cut, Vec3::from(st.centre), 1.0, st.yaw, &mut back);
        for (a, b) in back.iter().zip(&tris) {
            assert!(Vec3::from(a.pos).distance(Vec3::from(b.pos)) < 1e-3 && Vec3::from(a.normal).distance(Vec3::from(b.normal)) < 1e-5);
        }
        // elsewhere, twice the size, a quarter turn from the box frame: local (x, z) goes to game (z, -x) x 2
        let mut out = Vec::new();
        let base = Vec3::new(-300.0, 2.0, 700.0);
        place_stamp(&cut, base, 2.0, 90.0, &mut out);
        for (o, l) in out.iter().zip(&cut) {
            let g = Vec3::from(o.pos) * Vec3::new(1.0, 1.0, -1.0);
            let e = base + Vec3::new(l.pos[2], l.pos[1], -l.pos[0]) * 2.0;
            assert!(g.distance(e) < 1e-3, "{g} {e}");
        }
    }

    /// A triangle (view space) from game points.
    fn tri_g(a: Vec3, b: Vec3, c: Vec3) -> [Vertex; 3] {
        let v = |p: Vec3| Vertex { pos: [p.x, p.y, -p.z], uv: [0.0; 2], color: [1.0; 4], layer: 0.0, normal: [0.0, 1.0, 0.0], env: 0.0 };
        [v(a), v(b), v(c)]
    }

    /// A closed box (12 triangles) with its base centre at `c`, `size` wide, tall, deep, turned `yaw` degrees.
    fn box_tris(c: Vec3, size: Vec3, yaw: f32) -> Vec<Vertex> {
        let q = Quat::from_rotation_y(yaw.to_radians());
        let p = |i: usize| c + q * Vec3::new(if i & 1 == 0 { -0.5 } else { 0.5 } * size.x, if i & 2 == 0 { 0.0 } else { size.y }, if i & 4 == 0 { -0.5 } else { 0.5 } * size.z);
        [[0, 1, 3, 2], [4, 5, 7, 6], [0, 1, 5, 4], [2, 3, 7, 6], [0, 2, 6, 4], [1, 3, 7, 5]]
            .iter()
            .flat_map(|f| [tri_g(p(f[0]), p(f[1]), p(f[2])), tri_g(p(f[0]), p(f[2]), p(f[3]))])
            .flatten()
            .collect()
    }

    /// A ground sheet at y = 0, 20 x 20 quads of 10 m from (-100, -100): the biggest piece.
    fn ground_tris() -> Vec<Vertex> {
        let mut t = Vec::new();
        for i in 0..20 {
            for j in 0..20 {
                let p = |a: i32, b: i32| Vec3::new(-100.0 + (i + a) as f32 * 10.0, 0.0, -100.0 + (j + b) as f32 * 10.0);
                t.extend(tri_g(p(0, 0), p(1, 0), p(1, 1)));
                t.extend(tri_g(p(0, 0), p(1, 1), p(0, 1)));
            }
        }
        t
    }

    /// The parts of one building (podium, tower on it, roof cap on that: three unwelded pieces) are found from a click
    /// on any of them; a box 1 m away and the ground sheet are not.
    #[test]
    fn fit_stamp_joins_the_parts_of_a_building_and_leaves_the_neighbours() {
        let mut tris = ground_tris();
        let n_ground = tris.len() / 3;
        let part = |tris: &mut Vec<Vertex>, c: Vec3, s: Vec3| {
            let first = tris.len() / 3;
            tris.extend(box_tris(c, s, 0.0));
            first
        };
        let podium = part(&mut tris, Vec3::new(13.3, 0.0, 13.3), Vec3::new(20.0, 10.0, 20.0));
        let tower = part(&mut tris, Vec3::new(13.3, 10.0, 13.3), Vec3::new(10.0, 40.0, 10.0));
        let cap = part(&mut tris, Vec3::new(13.3, 50.0, 13.3), Vec3::new(6.0, 5.0, 6.0));
        let next = part(&mut tris, Vec3::new(44.3, 0.0, 13.3), Vec3::new(20.0, 30.0, 20.0));
        let none = |_: usize| false;
        for seed in [podium, tower + 3, cap + 5] {
            let f = fit_stamp(&tris, &none, seed).unwrap_or_else(|| panic!("seed {seed}"));
            assert_eq!((f.parts, f.tris), (3, 36), "seed {seed}");
            assert!(Vec3::from(f.centre).distance(Vec3::new(13.3, 0.0, 13.3)) < 1e-3, "{:?}", f.centre);
            assert!(Vec3::from(f.size).distance(Vec3::new(20.2, 55.1, 20.2)) < 1e-3, "{:?}", f.size);
            assert!(f.yaw.abs() < 1e-3, "{}", f.yaw);
        }
        // the neighbour is a building of its own
        let f = fit_stamp(&tris, &none, next + 2).unwrap();
        assert_eq!((f.parts, f.size[1]), (1, 30.1));
        // the ground sheet is the ground; a skipped seed is nothing; a skipped tower leaves the cap off the podium
        assert!(fit_stamp(&tris, &none, 0).is_none() && n_ground > 100);
        assert!(fit_stamp(&tris, &|i| i == cap + 5, cap + 5).is_none());
        let f = fit_stamp(&tris, &|i| (tower..tower + 12).contains(&i), podium).unwrap();
        assert_eq!((f.parts, f.size[1]), (1, 10.1));
    }

    /// A building turned to 30 degrees gets a box turned to it: the yaw is the walls' and the size the building's own.
    #[test]
    fn fit_stamp_turns_the_box_to_the_walls() {
        for yaw in [30.0, -20.0, 44.0] {
            let mut tris = ground_tris();
            let first = tris.len() / 3;
            tris.extend(box_tris(Vec3::new(31.7, 3.0, -22.4), Vec3::new(14.0, 33.0, 9.0), yaw));
            let f = fit_stamp(&tris, &|_| false, first).unwrap();
            assert!((f.yaw - yaw).abs() < 1e-2, "{} vs {yaw}", f.yaw);
            assert!(Vec3::from(f.size).distance(Vec3::new(14.2, 33.1, 9.2)) < 1e-2, "{:?}", f.size);
            assert!(Vec3::from(f.centre).distance(Vec3::new(31.7, 3.0, -22.4)) < 1e-2, "{:?}", f.centre);
            // the fitted box cuts the building: its 10 triangles above the floor (the base face, level with the ground, stays)
            let st = Stamp { name: "f".into(), centre: f.centre, size: f.size, yaw: f.yaw, floor: 0.5 };
            assert_eq!(stamp_tris(&tris, &|_| false, &st).len() / 3, 10);
        }
    }

    /// The meshes `Scene::props` holds: the .lws objects with triangles, not the three podium emblems after them.
    #[test]
    fn object_list_skips_empty_meshes_and_the_emblems() {
        let tri = vec![test_vertex([0.0; 3]); 3];
        let names: Vec<String> = ["Je_tower", "blinkers0", "AirShipnn", "first", "second", "third"].map(String::from).to_vec();
        let props = vec![tri.clone(), Vec::new(), tri.clone(), tri.clone(), tri.clone(), tri];
        let list: Vec<&str> = object_list(&names, &props).map(|(n, _)| n).collect();
        assert_eq!(list, ["Je_tower", "AirShipnn"]);
        // no scene objects, only emblems (or nothing loaded)
        assert_eq!(object_list(&names[3..], &props[3..]).count(), 0);
        assert_eq!(object_list(&[], &[]).count(), 0);
    }

    /// A placed object has the mesh's triangles, its lowest point on the base, scales about its origin, turns about the
    /// view Y axis, and takes the face colour (a textured face the original's constant) times the light.
    #[test]
    fn place_object_stands_on_base_turns_and_lights() {
        let v = |x: f32, y: f32, z: f32, layer: f32, env: f32| Vertex { pos: [x, y, z], uv: [0.25, 0.5], color: [0.2, 0.4, 0.6, 1.0], layer, normal: [1.0, 0.0, 0.0], env };
        // origin 2 m under the lowest point of the object: x 3, y 2..12
        let mesh = vec![v(3.0, 2.0, 0.0, 7.0, PROP_ENV), v(3.0, 12.0, 0.0, 7.0, PROP_ENV), v(3.0, 12.0, 4.0, 7.0, PROP_ENV), v(3.0, 2.0, 0.0, -1.0, 0.0), v(3.0, 2.0, 0.0, -1.0, SELF_LIT_ENV), v(3.0, 2.0, 0.0, -1.0, 0.0)];
        let base = Vec3::new(5.0, 3.0, -2.0);
        let half = |_: Vec3, _: Vec3| [0.5, 0.5, 0.5, 1.0];
        let (mut a, mut b) = (Vec::new(), Vec::new());
        place_object(&mesh, base, 1.0, 0.0, [0.8; 3], &half, &mut a);
        place_object(&mesh, base, 2.0, std::f32::consts::FRAC_PI_2, [0.8; 3], &half, &mut b);
        assert_eq!((a.len(), b.len()), (6, 6));
        // lowest point on the base, 10 m tall at scale 1 and 20 m at scale 2; x, z keep the object's own offsets
        assert!((a.iter().map(|q| q.pos[1]).fold(f32::MAX, f32::min) - 3.0).abs() < 1e-5);
        assert!((a.iter().map(|q| q.pos[1]).fold(f32::MIN, f32::max) - 13.0).abs() < 1e-5);
        assert!((b.iter().map(|q| q.pos[1]).fold(f32::MIN, f32::max) - 23.0).abs() < 1e-5);
        assert!((a[2].pos[0] - 8.0).abs() < 1e-5 && (a[2].pos[2] - 2.0).abs() < 1e-5);
        // a quarter turn takes an offset (x, z) to (z, -x), doubled by the scale; the normal turns with it
        let (dx, dz) = (a[2].pos[0] - base.x, a[2].pos[2] - base.z);
        assert!((b[2].pos[0] - base.x - 2.0 * dz).abs() < 1e-4 && (b[2].pos[2] - base.z + 2.0 * dx).abs() < 1e-4);
        assert!(Vec3::from(b[2].normal).distance(Vec3::new(0.0, 0.0, -1.0)) < 1e-5);
        // pictures and UVs stay; env goes
        assert!(a[..3].iter().all(|q| q.layer == 7.0 && q.uv == [0.25, 0.5] && q.env == 0.0));
        assert!(a[3..].iter().all(|q| q.layer == -1.0 && q.env == 0.0));
        // textured: constant x light; untextured: own colour x light; self-lit: own colour, no light
        assert_eq!(a[0].color, [0.4, 0.4, 0.4, 1.0]);
        assert!(a[3].color.iter().zip([0.1, 0.2, 0.3, 1.0]).all(|(c, e)| (c - e).abs() < 1e-6));
        assert_eq!(a[4].color, [0.2, 0.4, 0.6, 1.0]);
    }

    #[test]
    fn missing_file_is_empty() {
        assert_eq!(load(Path::new("/nonexistent"), 1), FillFile::default());
    }

    fn area(p: &[[f32; 2]], t: [usize; 3]) -> f32 {
        let (a, b, c) = (p[t[0]], p[t[1]], p[t[2]]);
        ((b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0])) / 2.0
    }

    fn grid(n: usize, hole: bool) -> Vec<Vertex> {
        let v = |x: usize, z: usize| Vertex { pos: [x as f32 * 5.0, 0.0, z as f32 * 5.0], uv: [0.0; 2], color: [1.0; 4], layer: 0.0, normal: [0.0, 1.0, 0.0], env: 0.0 };
        let mut out = Vec::new();
        for x in 0..n {
            for z in 0..n {
                if hole && x == n / 2 && z == n / 2 {
                    continue;
                }
                out.extend([v(x, z), v(x, z + 1), v(x + 1, z + 1), v(x, z), v(x + 1, z + 1), v(x + 1, z)]);
            }
        }
        out
    }

    #[test]
    fn outline_of_a_grid() {
        let (o, c) = outline(&grid(10, false));
        assert!(o.len() >= 8);
        // the 50 x 50 square, up to smoothing at the corners
        assert!((shoelace(&o).abs() - 2500.0).abs() < 120.0, "{}", shoelace(&o));
        assert!((c.x - 25.0).abs() < 1.0 && (c.z - 25.0).abs() < 1.0);
        // an interior hole is not the outer loop
        let (h, _) = outline(&grid(10, true));
        assert!((shoelace(&h).abs() - 2500.0).abs() < 120.0);
        assert_eq!(outline(&[]).0.len(), 0);
    }

    #[test]
    fn skirt_never_rises_and_reaches_radius() {
        let (o, c) = outline(&grid(10, false));
        let k = Skirt::new(o, c, 500.0, 20.0, 0.0);
        for ring in k.rings() {
            assert!(ring.iter().all(|p| p.y <= 0.0));
            assert!(((ring[3] - c).with_y(0.0).length() - 500.0).abs() < 1.0);
            assert!((ring[3].y + 20.0).abs() < 1e-3);
        }
        assert!(k.height(200.0, 25.0) < -1.0);
        assert!(k.height(52.0, 25.0) <= 0.0);
    }

    #[test]
    fn point_in_polygon() {
        let sq = [Vec3::new(0.0, 0.0, 0.0), Vec3::new(10.0, 0.0, 0.0), Vec3::new(10.0, 0.0, 10.0), Vec3::new(0.0, 0.0, 10.0)];
        assert!(inside(&sq, 5.0, 5.0));
        assert!(!inside(&sq, 15.0, 5.0) && !inside(&sq, 5.0, -1.0));
    }

    #[test]
    fn rng_is_deterministic() {
        let (mut a, mut b) = (Rng(5), Rng(5));
        for _ in 0..10 {
            assert_eq!(a.next(), b.next());
        }
    }

    #[test]
    fn ear_clipping_concave() {
        // an L, 3 square units, once each winding
        let l = [[0.0, 0.0], [2.0, 0.0], [2.0, 1.0], [1.0, 1.0], [1.0, 2.0], [0.0, 2.0]];
        for poly in [l.to_vec(), l.iter().rev().copied().collect()] {
            let t = triangulate(&poly);
            assert_eq!(t.len(), 4);
            let sum: f32 = t.iter().map(|&t| area(&poly, t).abs()).sum();
            assert!((sum - 3.0).abs() < 1e-5, "{sum}");
            // no triangle covers the notch at (1.5, 1.5)
            let inside = |t: [usize; 3]| {
                let (a, b, c) = (poly[t[0]], poly[t[1]], poly[t[2]]);
                let e = |p: [f32; 2], q: [f32; 2]| (q[0] - p[0]) * (1.5 - p[1]) - (q[1] - p[1]) * (1.5 - p[0]);
                let s = [e(a, b), e(b, c), e(c, a)];
                s.iter().all(|&x| x >= 0.0) || s.iter().all(|&x| x <= 0.0)
            };
            assert!(!t.iter().any(|&t| inside(t)));
        }
    }
}
