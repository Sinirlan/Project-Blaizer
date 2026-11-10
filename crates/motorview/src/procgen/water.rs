//! REMASTER water (re-notes/12-world-fill.md 12.8): a procedural surface that replaces the
//! original's flat water texture. The original has no animated water, so Classic keeps the
//! texture; Remaster draws these meshes with `vs_water` / `fs_water` (shader.wgsl).
//!
//! Two kinds of body, both in `remaster/fill/trackNN.fill` next to the fill pieces:
//! ```text
//! Water Texture WATT Style ocean [params]                    # every world polygon with this texture
//! Water Texture WATT+Sandv2 ...                              # + the flat Sandv2 at WATT's levels
//! Water Texture WATT+Sandv2+untextured ...                   # + the flat untextured polygons at those levels (#456)
//! Water Texture WATT Piece x y z Style still                 # only the connected piece of WATT at that point
//! Water Area Level y Point x z Point x z ... Style still     # a new flat area (Atlantika's sea)
//! Water Area ... Bed Sandv2                                  # + a sloping bottom in that world texture
//! params: Deep r g b  Shallow r g b  Clarity 0..1  WaveHeight m  WaveLength m  Speed m/s
//!         Direction deg  Chop 0..1  Foam 0..1  Reflect 0..1
//!         Shore r g b  Visibility m  Extinction m  Refraction 0..1  BedDepth m
//!         Tiling x  Normals 0..1  ShoreFoam m  FoamHeight m  Caustics 0..1   (re-notes/16-water.md)
//! ```
//! A `Piece` body takes the connected piece of its texture under the point; a body of the same texture
//! without `Piece` keeps the rest (re-notes/16-water.md "Pieces").
//! `Style` sets the defaults, a parameter overrides it. Coordinates are game coordinates.
//! At most `MAX_BODIES` bodies (the shader reads them from the globals).

use crate::scene::{Scene, Vertex};

pub const MAX_BODIES: usize = 4;
/// vec4 rows per body in the shader's globals (`Params::packed`)
pub const ROWS: usize = 7;
/// `Vertex::env` of a water vertex is `WATER_ENV + body index`
pub const WATER_ENV: f32 = 16.0;
/// longest triangle edge of an ocean surface (metres): the vertex shader displaces the mesh
const OCEAN_EDGE: f32 = 4.0;
/// how close (metres) an extra texture's triangle must lie to a level of the water's own to join it
const LEVEL_TOLERANCE: f32 = 0.05;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Style {
    #[default]
    Still,
    Ocean,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Params {
    pub style: Style,
    pub deep: [f32; 3],
    pub shallow: [f32; 3],
    pub clarity: f32,
    pub wave_height: f32,
    pub wave_length: f32,
    pub speed: f32,
    pub direction: f32,
    pub chop: f32,
    pub foam: f32,
    pub reflect: f32,
    /// surface colour in the shallows near the shore (tuxalin `_ShoreColor`)
    pub shore: [f32; 3],
    /// how far down (along the view) the bottom stays visible, metres
    pub visibility: f32,
    /// vertical depth at which the colour is all `deep`, metres (red goes at 0.4 of it, blue at 1.6)
    pub extinction: f32,
    /// how far the ripples bend the view of the bottom, 0..1
    pub refraction: f32,
    /// depth of the generated bed under a texture body, metres (0 = none: bottomless)
    pub bed_depth: f32,
    /// normal map repeats per 20 m (tuxalin `_TextureTiling`)
    pub tiling: f32,
    /// how much the normal map tilts the surface, 0..1 (tuxalin `_NormalIntensity`)
    pub normals: f32,
    /// width of the foam band at the shore, metres of water depth (tuxalin `_FoamRanges.x`)
    pub shore_foam: f32,
    /// swell height above the rest level where the crests foam, metres (tuxalin `_FoamRanges.z`)
    pub foam_height: f32,
    /// strength of the moving light patterns on the shallow bottom, 0..1 (0 = none)
    pub caustics: f32,
    /// texture bodies: interior holes of the original polygons up to this area (m²) are closed, so
    /// they are water, not shore (0 = leave them) [design choice]
    pub fill_holes: f32,
}

impl Params {
    /// The defaults of a style [design choice].
    pub fn new(style: Style) -> Self {
        match style {
            Style::Still => Params { style, deep: [0.04, 0.16, 0.22], shallow: [0.2, 0.45, 0.5], clarity: 0.6, wave_height: 0.02, wave_length: 1.5, speed: 0.15, direction: 30.0, chop: 0.0, foam: 0.0, reflect: 1.0, shore: [0.45, 0.6, 0.55], visibility: 3.0, extinction: 4.0, refraction: 0.5, bed_depth: 1.5, tiling: 3.0, normals: 0.3, shore_foam: 0.3, foam_height: 10.0, caustics: 0.6, fill_holes: 0.0 },
            Style::Ocean => Params { style, deep: [0.02, 0.1, 0.2], shallow: [0.12, 0.4, 0.5], clarity: 0.3, wave_height: 1.2, wave_length: 28.0, speed: 2.5, direction: 20.0, chop: 0.6, foam: 0.7, reflect: 1.0, shore: [0.4, 0.62, 0.6], visibility: 6.0, extinction: 8.0, refraction: 0.5, bed_depth: 3.0, tiling: 1.0, normals: 0.5, shore_foam: 0.8, foam_height: 0.35, caustics: 0.4, fill_holes: 0.0 },
        }
    }

    /// The `ROWS` vec4 the shader reads: (deep, clarity), (shallow, height), (length, speed,
    /// direction in radians, chop), (style, foam, reflect, shore foam), (shore, visibility),
    /// (extinction, refraction, tiling, normals), (foam height, caustics, 0, 0).
    pub fn packed(&self) -> [[f32; 4]; ROWS] {
        [
            [self.deep[0], self.deep[1], self.deep[2], self.clarity],
            [self.shallow[0], self.shallow[1], self.shallow[2], self.wave_height],
            [self.wave_length, self.speed, self.direction.to_radians(), self.chop],
            [(self.style == Style::Ocean) as u32 as f32, self.foam, self.reflect, self.shore_foam],
            [self.shore[0], self.shore[1], self.shore[2], self.visibility],
            [self.extinction, self.refraction, self.tiling, self.normals],
            [self.foam_height, self.caustics, 0.0, 0.0],
        ]
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Source {
    /// the world polygons with this texture (case-insensitive); `piece` (game coordinates): only
    /// the connected piece of them that contains (or lies nearest) that point
    Texture { name: String, piece: Option<[f32; 3]> },
    /// a flat polygon in XZ at height `level`
    Area { level: f32, points: Vec<[f32; 2]> },
}

#[derive(Clone, Debug, PartialEq)]
pub struct Body {
    pub source: Source,
    pub params: Params,
    /// the world texture of the bottom under an area (None: bottomless; texture bodies make
    /// theirs from the hidden polygons and ignore it)
    pub bed: Option<String>,
}

/// Parse the words after `Water`.
pub fn parse(toks: &[&str]) -> Result<Body, String> {
    let (area, mut i) = match toks.first() {
        Some(&"Texture") => (false, 0),
        Some(&"Area") => (true, 1),
        _ => return Err("Water needs Texture or Area".into()),
    };
    let mut tex = None;
    let (mut level, mut points) = (0.0, Vec::new());
    let mut kv: Vec<(&str, Vec<f32>)> = Vec::new();
    let mut style = Style::Still;
    let mut bed = None;
    let mut piece = None;
    while i < toks.len() {
        let key = toks[i];
        i += 1;
        if key == "Texture" || key == "Style" || key == "Bed" {
            let w = *toks.get(i).ok_or(format!("{key} needs a name"))?;
            i += 1;
            if key == "Texture" {
                tex = Some(w.to_string());
            } else if key == "Bed" {
                bed = Some(w.to_string());
            } else {
                style = match w.to_ascii_lowercase().as_str() {
                    "still" => Style::Still,
                    "ocean" => Style::Ocean,
                    _ => return Err(format!("unknown water style '{w}'")),
                };
            }
            continue;
        }
        let mut v = Vec::new();
        while let Some(x) = toks.get(i).and_then(|t| t.parse::<f32>().ok()) {
            v.push(x);
            i += 1;
        }
        kv.push((key, v));
    }
    let mut p = Params::new(style);
    for (key, v) in &kv {
        let need = |k: usize| if v.len() >= k { Ok(()) } else { Err(format!("{key} needs {k} values")) };
        match *key {
            "Level" => { need(1)?; level = v[0] }
            "Point" => { need(2)?; points.push([v[0], v[1]]) }
            "Piece" => { need(3)?; piece = Some([v[0], v[1], v[2]]) }
            "Deep" => { need(3)?; p.deep = [v[0], v[1], v[2]] }
            "Shallow" => { need(3)?; p.shallow = [v[0], v[1], v[2]] }
            "Clarity" => { need(1)?; p.clarity = v[0].clamp(0.0, 1.0) }
            "WaveHeight" => { need(1)?; p.wave_height = v[0].max(0.0) }
            "WaveLength" => { need(1)?; p.wave_length = v[0].max(0.05) }
            "Speed" => { need(1)?; p.speed = v[0] }
            "Direction" => { need(1)?; p.direction = v[0] }
            "Chop" => { need(1)?; p.chop = v[0].clamp(0.0, 1.0) }
            "Foam" => { need(1)?; p.foam = v[0].clamp(0.0, 1.0) }
            "Reflect" => { need(1)?; p.reflect = v[0].clamp(0.0, 1.0) }
            "Shore" => { need(3)?; p.shore = [v[0], v[1], v[2]] }
            "Visibility" => { need(1)?; p.visibility = v[0].max(0.1) }
            "Extinction" => { need(1)?; p.extinction = v[0].max(0.1) }
            "Refraction" => { need(1)?; p.refraction = v[0].clamp(0.0, 1.0) }
            "BedDepth" => { need(1)?; p.bed_depth = v[0].max(0.0) }
            "Tiling" => { need(1)?; p.tiling = v[0].max(0.01) }
            "Normals" => { need(1)?; p.normals = v[0].clamp(0.0, 1.0) }
            "ShoreFoam" => { need(1)?; p.shore_foam = v[0].max(0.0) }
            "FoamHeight" => { need(1)?; p.foam_height = v[0].max(0.0) }
            "Caustics" => { need(1)?; p.caustics = v[0].clamp(0.0, 1.0) }
            "FillHoles" => { need(1)?; p.fill_holes = v[0].max(0.0) }
            _ => {}
        }
    }
    let source = if area {
        if points.len() < 3 {
            return Err("Water Area needs 3 Point".into());
        }
        Source::Area { level, points }
    } else {
        Source::Texture { name: tex.ok_or("Water Texture needs a name")?, piece }
    };
    Ok(Body { source, params: p, bed })
}

/// The line for `Water` (without the keyword), `parse`'s inverse. Only values that differ from
/// the style's defaults are written.
pub fn to_text(b: &Body) -> String {
    let mut l = String::from("Water");
    match &b.source {
        Source::Texture { name, piece } => {
            l.push_str(&format!(" Texture {name}"));
            if let Some(p) = piece {
                l.push_str(&format!(" Piece {} {} {}", p[0], p[1], p[2]));
            }
        }
        Source::Area { level, points } => {
            l.push_str(&format!(" Area Level {level}"));
            for p in points {
                l.push_str(&format!(" Point {} {}", p[0], p[1]));
            }
        }
    }
    if let Some(t) = &b.bed {
        l.push_str(&format!(" Bed {t}"));
    }
    let (p, d) = (&b.params, Params::new(b.params.style));
    l.push_str(if p.style == Style::Ocean { " Style ocean" } else { " Style still" });
    let c3 = |l: &mut String, k: &str, v: [f32; 3], d: [f32; 3]| if v != d { l.push_str(&format!(" {k} {} {} {}", v[0], v[1], v[2])) };
    c3(&mut l, "Deep", p.deep, d.deep);
    c3(&mut l, "Shallow", p.shallow, d.shallow);
    let c1 = |l: &mut String, k: &str, v: f32, d: f32| if v != d { l.push_str(&format!(" {k} {v}")) };
    c1(&mut l, "Clarity", p.clarity, d.clarity);
    c1(&mut l, "WaveHeight", p.wave_height, d.wave_height);
    c1(&mut l, "WaveLength", p.wave_length, d.wave_length);
    c1(&mut l, "Speed", p.speed, d.speed);
    c1(&mut l, "Direction", p.direction, d.direction);
    c1(&mut l, "Chop", p.chop, d.chop);
    c1(&mut l, "Foam", p.foam, d.foam);
    c1(&mut l, "Reflect", p.reflect, d.reflect);
    c3(&mut l, "Shore", p.shore, d.shore);
    c1(&mut l, "Visibility", p.visibility, d.visibility);
    c1(&mut l, "Extinction", p.extinction, d.extinction);
    c1(&mut l, "Refraction", p.refraction, d.refraction);
    c1(&mut l, "BedDepth", p.bed_depth, d.bed_depth);
    c1(&mut l, "Tiling", p.tiling, d.tiling);
    c1(&mut l, "Normals", p.normals, d.normals);
    c1(&mut l, "ShoreFoam", p.shore_foam, d.shore_foam);
    c1(&mut l, "FoamHeight", p.foam_height, d.foam_height);
    c1(&mut l, "Caustics", p.caustics, d.caustics);
    c1(&mut l, "FillHoles", p.fill_holes, d.fill_holes);
    l
}

/// What `build` makes of a fill's water bodies.
#[derive(Default)]
pub struct Mesh {
    pub verts: Vec<Vertex>,
    /// per body: `Params::packed`
    pub params: Vec<[[f32; 4]; ROWS]>,
    /// per world triangle (`Scene::tris` / 3): true = tagged as water, left out of the Remaster
    /// world draw (empty: none)
    pub hide: Vec<bool>,
    /// the bed under texture bodies (the hidden triangles) and under areas with a `Bed` texture,
    /// subdivided and lowered away from the shore (`bed_drop`); drawn with the Remaster world in
    /// their place
    pub bed: Vec<Vertex>,
}

/// `color[0]` = distance to the body's shoreline over `calm_distance` (0 at the shore, 1 = full
/// swell; `vs_water` scales the swells by it), the rest 1.
fn vertex(p: [f32; 3], body: usize, shore: f32) -> Vertex {
    Vertex { pos: p, uv: [p[0], -p[2]], color: [shore, 1.0, 1.0, 1.0], layer: -1.0, normal: [0.0, 1.0, 0.0], env: WATER_ENV + body as f32 }
}

/// How far from the shore the ocean swells reach full height [design choice, APPROX]: twice the
/// wave length, so a pool or a narrow channel stays nearly flat and no crest rises over a rim
/// or pulls away from a wall (the hidden texture left nothing under the surface there).
pub fn calm_distance(p: &Params) -> f32 {
    (2.0 * p.wave_length).max(4.0)
}

/// The bed's fall away from the shore (metres down per metre in) [design choice]
const BED_SLOPE: f32 = 0.25;

/// How far below the surface the bed lies at `shore` metres from the shoreline: a 1:4 bank down
/// to `BedDepth`. The original water texture was the floor of its basin (nothing lies under it),
/// so without a bed the water would be bottomless and the shore colours would never show.
fn bed_drop(p: &Params, shore: f32) -> f32 {
    (shore * BED_SLOPE).min(p.bed_depth)
}

/// The shoreline of a triangle soup: the edges only one triangle uses (endpoints matched on a
/// 1 cm grid, so the world's shared vertices pair up).
pub(crate) fn shoreline(tris: &[[[f32; 3]; 3]]) -> Vec<[[f32; 3]; 2]> {
    use std::collections::HashMap;
    let key = grid_key;
    // edge key (sorted endpoint keys) -> the edge and how many triangles use it
    type Key = ([i64; 3], [i64; 3]);
    let mut edges: HashMap<Key, ([[f32; 3]; 2], u32)> = HashMap::new();
    for t in tris {
        for k in 0..3 {
            let (a, b) = (t[k], t[(k + 1) % 3]);
            let (ka, kb) = (key(a), key(b));
            let e = edges.entry(if ka < kb { (ka, kb) } else { (kb, ka) }).or_insert(([a, b], 0));
            e.1 += 1;
        }
    }
    edges.into_values().filter(|e| e.1 == 1).map(|e| e.0).collect()
}

/// Distance from `p` to the nearest shoreline segment, capped at `cap` (beyond it the swells are
/// at full height anyway). The segments are bucketed in an XZ grid of `cap`-sized cells.
pub(crate) struct ShoreGrid {
    cell: f32,
    cells: std::collections::HashMap<(i32, i32), Vec<usize>>,
    segs: Vec<[[f32; 3]; 2]>,
}

impl ShoreGrid {
    pub(crate) fn new(segs: Vec<[[f32; 3]; 2]>, cell: f32) -> Self {
        let mut cells: std::collections::HashMap<(i32, i32), Vec<usize>> = Default::default();
        let c = |v: f32| (v / cell).floor() as i32;
        for (i, s) in segs.iter().enumerate() {
            for x in c(s[0][0].min(s[1][0]))..=c(s[0][0].max(s[1][0])) {
                for z in c(s[0][2].min(s[1][2]))..=c(s[0][2].max(s[1][2])) {
                    cells.entry((x, z)).or_default().push(i);
                }
            }
        }
        ShoreGrid { cell, cells, segs }
    }

    pub(crate) fn distance(&self, p: [f32; 3]) -> f32 {
        let (cx, cz) = ((p[0] / self.cell).floor() as i32, (p[2] / self.cell).floor() as i32);
        let mut best = self.cell;
        for x in cx - 1..=cx + 1 {
            for z in cz - 1..=cz + 1 {
                for &i in self.cells.get(&(x, z)).map(Vec::as_slice).unwrap_or(&[]) {
                    best = best.min(segment_distance(p, self.segs[i]));
                }
            }
        }
        best
    }
}

fn segment_distance(p: [f32; 3], s: [[f32; 3]; 2]) -> f32 {
    let sub = |a: [f32; 3], b: [f32; 3]| [a[0] - b[0], a[1] - b[1], a[2] - b[2]];
    let dot = |a: [f32; 3], b: [f32; 3]| a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
    let (d, w) = (sub(s[1], s[0]), sub(p, s[0]));
    let t = (dot(w, d) / dot(d, d).max(1e-12)).clamp(0.0, 1.0);
    let q = sub(w, [d[0] * t, d[1] * t, d[2] * t]);
    dot(q, q).sqrt()
}

/// Split the longest edge until none is longer than `max` (a triangle in view space).
fn subdivide(t: [[f32; 3]; 3], max: f32, out: &mut Vec<[[f32; 3]; 3]>, depth: u32) {
    subdivide_by(t, max, out, depth, &|p| *p, &|a, b| mid(*a, *b));
}

/// `subdivide` for world vertices: every attribute is interpolated (the bed keeps the texture
/// and baked colours of the triangles it is made from).
fn subdivide_vertices(t: [Vertex; 3], max: f32, out: &mut Vec<[Vertex; 3]>) {
    let half = |a: &Vertex, b: &Vertex| Vertex {
        pos: mid(a.pos, b.pos),
        uv: [(a.uv[0] + b.uv[0]) / 2.0, (a.uv[1] + b.uv[1]) / 2.0],
        color: std::array::from_fn(|i| (a.color[i] + b.color[i]) / 2.0),
        normal: mid(a.normal, b.normal),
        ..*a
    };
    subdivide_by(t, max, out, 0, &|v| v.pos, &half);
}

fn mid(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [(a[0] + b[0]) / 2.0, (a[1] + b[1]) / 2.0, (a[2] + b[2]) / 2.0]
}

/// Split every edge longer than `max`, whichever the triangle: an edge is split (or not) by its own
/// length, so two triangles sharing it put the same vertices on it and the swell displacement cannot
/// open a crack between them (splitting only a triangle's longest edge left T-junctions on the
/// diagonals of an area). Depth only guards degenerate input.
fn subdivide_by<T: Copy>(t: [T; 3], max: f32, out: &mut Vec<[T; 3]>, depth: u32, pos: &dyn Fn(&T) -> [f32; 3], half: &dyn Fn(&T, &T) -> T) {
    let d = |a: &T, b: &T| {
        let (a, b) = (pos(a), pos(b));
        ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)).sqrt()
    };
    let long: [bool; 3] = std::array::from_fn(|i| d(&t[i], &t[(i + 1) % 3]) > max);
    let n = long.iter().filter(|&&l| l).count();
    if n == 0 || depth >= 14 {
        out.push(t);
        return;
    }
    // midpoints of the long edges; edge i runs t[i] -> t[i + 1]
    let m: [Option<T>; 3] = std::array::from_fn(|i| long[i].then(|| half(&t[i], &t[(i + 1) % 3])));
    let mut parts: Vec<[T; 3]> = Vec::new();
    match n {
        3 => {
            let (m0, m1, m2) = (m[0].unwrap_or(t[0]), m[1].unwrap_or(t[1]), m[2].unwrap_or(t[2]));
            parts.extend([[t[0], m0, m2], [m0, t[1], m1], [m2, m1, t[2]], [m0, m1, m2]]);
        }
        2 => {
            // v is the corner between the two long edges, `ma` on the edge into it and `mb` on the one out
            let v = (0..3).find(|&i| long[(i + 2) % 3] && long[i]).unwrap_or(0);
            let (ma, mb) = (m[(v + 2) % 3].unwrap_or(t[v]), m[v].unwrap_or(t[v]));
            let (p, q) = (t[(v + 1) % 3], t[(v + 2) % 3]);
            // ma lies between q and v, mb between v and p
            parts.extend([[ma, t[v], mb], [mb, p, q], [mb, q, ma]]);
        }
        _ => {
            let i = long.iter().position(|&l| l).unwrap_or(0);
            let mi = m[i].unwrap_or(t[i]);
            parts.extend([[t[i], mi, t[(i + 2) % 3]], [mi, t[(i + 1) % 3], t[(i + 2) % 3]]]);
        }
    }
    for part in parts {
        subdivide_by(part, max, out, depth + 1, pos, half);
    }
}

/// The reserved texture name of the world's untextured polygons (`Scene::world_tri_texture` = `u32::MAX`),
/// for `Water Texture WATT+untextured` (#456).
const UNTEXTURED: &str = "untextured";

/// The world texture indices (`Scene::world_tri_texture`) named `name` in `names`, case-insensitive, with or
/// without `.iff`. `untextured` is the polygons without a texture.
fn texture_ids(names: &[String], name: &str) -> Vec<u32> {
    let want = name.to_ascii_lowercase();
    let want = want.strip_suffix(".iff").unwrap_or(&want).to_string();
    if want == UNTEXTURED {
        return vec![u32::MAX];
    }
    let ids: Vec<u32> = names.iter().enumerate()
        .filter(|(_, n)| { let n = n.to_ascii_lowercase(); n.strip_suffix(".iff").unwrap_or(&n) == want })
        .map(|(i, _)| i as u32).collect();
    if ids.is_empty() {
        eprintln!("water: no world texture '{name}'");
    }
    ids
}

/// The world triangles (indices into `Scene::world_tri_texture`) of texture `name`. `WATT+Sandv2`:
/// the first texture is the water; the others join only where they lie flat at one of its levels
/// (the original's dark shallows, Olympos `Sandv2` at y −32.2). `untextured` is a texture too: Olympos's far sea
/// is 429 flat untextured quads at y −32.2 west of `WATT` (#456).
fn texture_tris(s: &Scene, name: &str) -> Vec<usize> {
    let ids = |name: &str| texture_ids(&s.world_textures, name);
    let mut names = name.split('+').filter(|n| !n.is_empty());
    let main = names.next().map(ids).unwrap_or_default();
    let extra: Vec<u32> = names.flat_map(ids).collect();
    let tri_at = |ti: usize| (s.tris.len() >= ti * 3 + 3).then(|| &s.tris[ti * 3..ti * 3 + 3]);
    let mut levels: Vec<f32> = Vec::new();
    for (ti, &t) in s.world_tri_texture.iter().enumerate() {
        if let Some(v) = tri_at(ti).filter(|_| main.contains(&t)) {
            let y = v[0].pos[1];
            if v.iter().all(|q| (q.pos[1] - y).abs() < LEVEL_TOLERANCE) && !levels.iter().any(|l| (l - y).abs() < LEVEL_TOLERANCE) {
                levels.push(y);
            }
        }
    }
    let mut out = Vec::new();
    for (ti, &t) in s.world_tri_texture.iter().enumerate() {
        let Some(v) = tri_at(ti) else { continue };
        if main.contains(&t) || (extra.contains(&t) && levels.iter().any(|l| v.iter().all(|q| (q.pos[1] - l).abs() < LEVEL_TOLERANCE))) {
            out.push(ti);
        }
    }
    out
}

/// Triangles closing the interior holes of a triangle soup: the closed loops of its shoreline
/// (1 cm grid, every vertex on exactly two shore edges) that enclose at most `max_area` m² in XZ.
/// The loop heights are kept, so a hole in flat water is closed flat. A loop that touches another
/// one or the outer edge at a vertex is left alone.
pub(crate) fn hole_patches(tris: &[[[f32; 3]; 3]], max_area: f32) -> Vec<[[f32; 3]; 3]> {
    use std::collections::HashMap;
    // vertex key -> its shore neighbours and its position
    type Nodes = HashMap<[i64; 3], (Vec<[i64; 3]>, [f32; 3])>;
    let mut at = Nodes::new();
    for [a, b] in shoreline(tris) {
        let (ka, kb) = (grid_key(a), grid_key(b));
        at.entry(ka).or_insert((Vec::new(), a)).0.push(kb);
        at.entry(kb).or_insert((Vec::new(), b)).0.push(ka);
    }
    let mut seen: std::collections::HashSet<[i64; 3]> = Default::default();
    let mut loops = Vec::new();
    let mut out = Vec::new();
    for &start in at.keys() {
        if seen.contains(&start) {
            continue;
        }
        // walk the loop; `simple` = all its vertices have exactly two neighbours
        let (mut ring, mut simple, mut prev, mut cur) = (Vec::new(), true, [i64::MIN; 3], start);
        loop {
            seen.insert(cur);
            let (nb, pos) = &at[&cur];
            ring.push(*pos);
            simple &= nb.len() == 2;
            let Some(&next) = nb.iter().find(|&&k| k != prev).filter(|_| nb.len() == 2) else { simple = false; break };
            prev = cur;
            cur = next;
            if cur == start || seen.contains(&cur) {
                simple &= cur == start;
                break;
            }
        }
        if simple && ring.len() >= 3 {
            let pts: Vec<[f32; 2]> = ring.iter().map(|p| [p[0], p[2]]).collect();
            let area = (0..pts.len()).map(|i| { let (a, b) = (pts[i], pts[(i + 1) % pts.len()]); a[0] * b[1] - b[0] * a[1] }).sum::<f32>().abs() / 2.0;
            loops.push((ring, pts, area));
        }
    }
    // a hole lies inside a larger loop (the outer edge, which is never closed)
    let inside = |p: [f32; 2], poly: &[[f32; 2]]| {
        let n = poly.len();
        (0..n).filter(|&i| {
            let (a, b) = (poly[i], poly[(i + 1) % n]);
            (a[1] > p[1]) != (b[1] > p[1]) && p[0] < a[0] + (p[1] - a[1]) / (b[1] - a[1]) * (b[0] - a[0])
        }).count() % 2 == 1
    };
    for (ring, pts, area) in &loops {
        if *area <= max_area && loops.iter().any(|(_, o, oa)| oa > area && inside(pts[0], o)) {
            out.extend(super::fill::triangulate(pts).into_iter().map(|t| [ring[t[0]], ring[t[1]], ring[t[2]]]));
        }
    }
    out
}

/// A vertex on the 1 cm grid, so the world's shared vertices pair up.
fn grid_key(p: [f32; 3]) -> [i64; 3] {
    p.map(|c| (c * 100.0).round() as i64)
}

/// The connected piece of each triangle: triangles sharing a vertex (1 cm grid) are one piece.
/// Returns a piece id per triangle (the lowest triangle index of the piece).
pub(crate) fn pieces(tris: &[[[f32; 3]; 3]]) -> Vec<usize> {
    use std::collections::HashMap;
    fn find(parent: &mut [usize], mut i: usize) -> usize {
        while parent[i] != i {
            parent[i] = parent[parent[i]];
            i = parent[i];
        }
        i
    }
    let mut parent: Vec<usize> = (0..tris.len()).collect();
    let mut first: HashMap<[i64; 3], usize> = HashMap::new();
    for (ti, t) in tris.iter().enumerate() {
        for &p in t {
            let other = *first.entry(grid_key(p)).or_insert(ti);
            let (a, b) = (find(&mut parent, ti), find(&mut parent, other));
            // the lower index becomes the root, so ids are the piece's lowest triangle
            parent[a.max(b)] = a.min(b);
        }
    }
    (0..tris.len()).map(|i| find(&mut parent, i)).collect()
}

/// The triangle at point `p` (scene space): the one whose XZ outline contains it, nearest in
/// height; none contain it: the one with the nearest centroid.
pub(crate) fn pick(tris: &[[[f32; 3]; 3]], p: [f32; 3]) -> Option<usize> {
    let inside = |t: &[[f32; 3]; 3]| -> Option<f32> {
        let d = (t[1][2] - t[2][2]) * (t[0][0] - t[2][0]) + (t[2][0] - t[1][0]) * (t[0][2] - t[2][2]);
        if d.abs() < 1e-9 {
            return None;
        }
        let a = ((t[1][2] - t[2][2]) * (p[0] - t[2][0]) + (t[2][0] - t[1][0]) * (p[2] - t[2][2])) / d;
        let b = ((t[2][2] - t[0][2]) * (p[0] - t[2][0]) + (t[0][0] - t[2][0]) * (p[2] - t[2][2])) / d;
        let c = 1.0 - a - b;
        (a >= -1e-4 && b >= -1e-4 && c >= -1e-4).then(|| (a * t[0][1] + b * t[1][1] + c * t[2][1] - p[1]).abs())
    };
    let best = tris.iter().enumerate().filter_map(|(i, t)| inside(t).map(|dy| (i, dy))).min_by(|a, b| a.1.total_cmp(&b.1));
    if let Some((i, _)) = best {
        return Some(i);
    }
    let dist = |t: &[[f32; 3]; 3]| {
        let c: [f32; 3] = std::array::from_fn(|k| (t[0][k] + t[1][k] + t[2][k]) / 3.0);
        (0..3).map(|k| (c[k] - p[k]).powi(2)).sum::<f32>()
    };
    tris.iter().enumerate().min_by(|a, b| dist(a.1).total_cmp(&dist(b.1))).map(|(i, _)| i)
}

/// Metres per repeat of a bed texture [design choice: the fill ground's default].
const BED_TILE: f32 = 4.0;

/// How the track's own polygons with texture `name` look: its layer and their mean baked colour
/// (weighted by area), for a bed made from nothing (`Water Area ... Bed NAME`). None: the track
/// has no such textured polygon.
fn bed_look(s: &Scene, name: &str) -> Option<(f32, [f32; 4])> {
    let ids = texture_ids(&s.world_textures, name);
    let (mut layer, mut sum, mut area) = (None, [0.0f32; 4], 0.0f32);
    for ti in s.world_tri_texture.iter().enumerate().filter(|(_, t)| ids.contains(t)).map(|(ti, _)| ti) {
        let Some(v) = s.tris.get(ti * 3..ti * 3 + 3).filter(|v| v[0].layer >= 0.0) else { continue };
        layer.get_or_insert(v[0].layer);
        let e = |a: usize, b: usize| std::array::from_fn::<f32, 3, _>(|i| v[b].pos[i] - v[a].pos[i]);
        let (p, q) = (e(0, 1), e(0, 2));
        let c = [p[1] * q[2] - p[2] * q[1], p[2] * q[0] - p[0] * q[2], p[0] * q[1] - p[1] * q[0]];
        let a = (c[0] * c[0] + c[1] * c[1] + c[2] * c[2]).sqrt() / 2.0;
        for (k, c) in sum.iter_mut().enumerate() {
            *c += a * (v[0].color[k] + v[1].color[k] + v[2].color[k]) / 3.0;
        }
        area += a;
    }
    let layer = layer?;
    Some((layer, if area > 0.0 { sum.map(|c| c / area) } else { [1.0; 4] }))
}

/// A bed triangle made from nothing: `t` with the bed texture's look, uv = world metres /
/// `BED_TILE`, wound to face up like the fill's ground (the surface is drawn double-sided).
fn floor_tri(t: &[[f32; 3]; 3], layer: f32, color: [f32; 4]) -> [Vertex; 3] {
    let v = |p: [f32; 3]| Vertex { pos: p, uv: [p[0] / BED_TILE, p[2] / BED_TILE], color, layer, normal: [0.0, 1.0, 0.0], env: 0.0 };
    let (p, q) = ([t[1][0] - t[0][0], t[1][2] - t[0][2]], [t[2][0] - t[0][0], t[2][2] - t[0][2]]);
    if p[1] * q[0] - p[0] * q[1] > 0.0 { [v(t[0]), v(t[1]), v(t[2])] } else { [v(t[0]), v(t[2]), v(t[1])] }
}

/// Mesh the bodies of `bodies` against the world of `s`. Texture bodies take the world
/// triangles (same vertices, so the surface sits exactly where the original's did); areas are
/// triangulated. The first `MAX_BODIES` bodies count.
pub fn build(bodies: &[Body], s: &Scene) -> Mesh {
    let mut m = Mesh::default();
    if bodies.len() > MAX_BODIES {
        eprintln!("water: only the first {MAX_BODIES} of {} bodies are used", bodies.len());
    }
    let bodies = &bodies[..bodies.len().min(MAX_BODIES)];
    // the world triangles of each texture body; a `Piece` body keeps the connected piece at its point
    let sets: Vec<Option<Vec<usize>>> = bodies.iter().map(|b| match &b.source {
        Source::Texture { name, piece } => {
            let all = texture_tris(s, name);
            Some(match piece {
                Some(p) => {
                    let pos: Vec<[[f32; 3]; 3]> = all.iter().map(|&ti| std::array::from_fn(|k| s.tris[ti * 3 + k].pos)).collect();
                    // game coordinates -> scene (z negated)
                    match pick(&pos, [p[0], p[1], -p[2]]) {
                        Some(hit) => {
                            let id = pieces(&pos);
                            all.iter().enumerate().filter(|(i, _)| id[*i] == id[hit]).map(|(_, &ti)| ti).collect()
                        }
                        None => {
                            eprintln!("water: no '{name}' triangle for piece {} {} {}", p[0], p[1], p[2]);
                            Vec::new()
                        }
                    }
                }
                None => all,
            })
        }
        Source::Area { .. } => None,
    }).collect();
    let claimed: std::collections::HashSet<usize> = bodies.iter().zip(&sets)
        .filter(|(b, _)| matches!(b.source, Source::Texture { piece: Some(_), .. }))
        .flat_map(|(_, set)| set.iter().flatten().copied()).collect();
    for (bi, b) in bodies.iter().enumerate() {
        m.params.push(b.params.packed());
        let ocean = b.params.style == Style::Ocean;
        let mut tris: Vec<[[f32; 3]; 3]> = Vec::new();
        let mut floor: Vec<[Vertex; 3]> = Vec::new();
        match &b.source {
            Source::Texture { piece, name } => {
                // the bed of joined untextured polygons (#456) takes the first texture's look, so the dark flat colour
                // of the original's far sea does not show as a seam against the `WATT` bed [design choice]
                let main_look = name.split('+').next().and_then(|n| bed_look(s, n));
                for &ti in sets[bi].as_deref().unwrap_or_default() {
                    // a body without `Piece` leaves the pieces other bodies claimed to them
                    if piece.is_none() && claimed.contains(&ti) {
                        continue;
                    }
                    let v = &s.tris[ti * 3..ti * 3 + 3];
                    match main_look.filter(|_| s.world_tri_texture.get(ti) == Some(&u32::MAX)) {
                        Some((layer, color)) => floor.push(floor_tri(&[v[0].pos, v[1].pos, v[2].pos], layer, color)),
                        None => floor.push([v[0], v[1], v[2]]),
                    }
                    if m.hide.is_empty() {
                        m.hide = vec![false; s.tris.len() / 3];
                    }
                    m.hide[ti] = true;
                    tris.push([v[0].pos, v[1].pos, v[2].pos]);
                }
                // interior holes of the original's polygons become water too (`FillHoles`); they
                // have no world triangle to hide, and take the bed's look from the texture
                if b.params.fill_holes > 0.0 {
                    let patch = hole_patches(&tris, b.params.fill_holes);
                    if let Some((layer, color)) = main_look {
                        floor.extend(patch.iter().map(|t| floor_tri(t, layer, color)));
                    }
                    tris.extend(patch);
                }
            }
            Source::Area { level, points } => {
                for t in super::fill::triangulate(points) {
                    let p = |i: usize| [points[t[i]][0], *level, -points[t[i]][1]];
                    tris.push([p(0), p(1), p(2)]);
                }
                // a `Bed` texture gives the area a floor (texture bodies have the hidden polygons)
                if let Some((layer, color)) = b.bed.as_deref().and_then(|n| bed_look(s, n)) {
                    floor.extend(tris.iter().map(|t| floor_tri(t, layer, color)));
                }
            }
        }
        // the shore distance calms the ocean's swells (still water is not displaced) and shapes the bed
        let calm = calm_distance(&b.params);
        let bed = b.params.bed_depth > 0.0 && !floor.is_empty();
        let reach = if bed { calm.max(b.params.bed_depth / BED_SLOPE) } else { calm };
        let grid = (ocean || bed).then(|| ShoreGrid::new(shoreline(&tris), reach));
        let mut fine = Vec::new();
        for &t in &tris {
            if ocean { subdivide(t, OCEAN_EDGE, &mut fine, 0) } else { fine.push(t) }
        }
        for t in fine {
            m.verts.extend(t.iter().map(|&p| vertex(p, bi, if ocean { grid.as_ref().map_or(1.0, |g| (g.distance(p) / calm).min(1.0)) } else { 1.0 })));
        }
        if let Some(g) = grid.as_ref().filter(|_| bed) {
            let mut fine = Vec::new();
            for t in floor {
                subdivide_vertices(t, OCEAN_EDGE, &mut fine);
            }
            for mut v in fine.into_iter().flatten() {
                v.pos[1] -= bed_drop(&b.params, g.distance(v.pos));
                m.bed.push(v);
            }
        }
    }
    m
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        let line = "Texture WATT Style ocean WaveHeight 2 Deep 0 0 1 Shore 1 0.5 0 Extinction 3 Tiling 2 FoamHeight 0.5";
        let b = parse(&line.split_whitespace().collect::<Vec<_>>()).unwrap();
        assert_eq!(b.params.wave_height, 2.0);
        assert_eq!(b.params.deep, [0.0, 0.0, 1.0]);
        assert_eq!((b.params.shore, b.params.extinction), ([1.0, 0.5, 0.0], 3.0));
        assert_eq!((b.params.tiling, b.params.foam_height), (2.0, 0.5));
        assert_eq!(bed_drop(&b.params, 2.0), 0.5);
        assert_eq!(bed_drop(&b.params, 100.0), b.params.bed_depth);
        let t = to_text(&b);
        let b2 = parse(&t.split_whitespace().skip(1).collect::<Vec<_>>()).unwrap();
        assert_eq!(b, b2);
        let a = parse(&["Area", "Level", "-3", "Point", "0", "0", "Point", "9", "0", "Point", "9", "9"]).unwrap();
        assert_eq!(parse(&to_text(&a).split_whitespace().skip(1).collect::<Vec<_>>()).unwrap(), a);
        assert!(parse(&["Area", "Point", "0", "0"]).is_err());
        // an area's bed texture and the caustics strength survive the file, and are written only when set
        let a = parse(&["Area", "Level", "-3", "Point", "0", "0", "Point", "9", "0", "Point", "9", "9", "Bed", "Sandb.iff", "Caustics", "0.2"]).unwrap();
        assert_eq!((a.bed.as_deref(), a.params.caustics), (Some("Sandb.iff"), 0.2));
        assert_eq!(parse(&to_text(&a).split_whitespace().skip(1).collect::<Vec<_>>()).unwrap(), a);
        assert!(!to_text(&b).contains("Bed") && !to_text(&b).contains("Caustics"));
        assert_eq!(a.params.packed()[6][1], 0.2);
    }

    #[test]
    fn untextured_is_a_texture_name() {
        let names = vec!["Watt.iff".to_string(), "Sandv2.iff".to_string()];
        assert_eq!(texture_ids(&names, "UNTEXTURED"), vec![u32::MAX]);
        assert_eq!(texture_ids(&names, "watt"), vec![0]);
        let b = parse(&["Texture", "WATT+Sandv2+untextured", "Style", "ocean"]).unwrap();
        assert_eq!(b.source, Source::Texture { name: "WATT+Sandv2+untextured".into(), piece: None });
        assert_eq!(parse(&to_text(&b).split_whitespace().skip(1).collect::<Vec<_>>()).unwrap(), b);
    }

    #[test]
    fn piece_splits_a_texture_and_finds_its_point() {
        let sq = |x: f32, y: f32| {
            let (a, b, c, d) = ([x, y, 0.0], [x + 10.0, y, 0.0], [x + 10.0, y, 10.0], [x, y, 10.0]);
            [[a, b, c], [a, c, d]]
        };
        // two squares of two triangles, apart: two pieces
        let tris: Vec<[[f32; 3]; 3]> = sq(0.0, -3.0).into_iter().chain(sq(100.0, -5.0)).collect();
        let id = pieces(&tris);
        assert_eq!(id, vec![0, 0, 2, 2]);
        assert_eq!(pick(&tris, [105.0, -5.0, 2.0]).map(|i| id[i]), Some(2));
        assert_eq!(pick(&tris, [5.0, -3.0, 8.0]).map(|i| id[i]), Some(0));
        // off both: the nearest centroid
        assert_eq!(pick(&tris, [60.0, 0.0, 5.0]).map(|i| id[i]), Some(2));
        let t = "Texture WATT Piece 22 -15.2 -1240 Style still";
        let b = parse(&t.split_whitespace().collect::<Vec<_>>()).unwrap();
        assert_eq!(b.source, Source::Texture { name: "WATT".into(), piece: Some([22.0, -15.2, -1240.0]) });
        assert_eq!(parse(&to_text(&b).split_whitespace().skip(1).collect::<Vec<_>>()).unwrap(), b);
    }

    #[test]
    fn holes_are_closed_up_to_their_area() {
        // a 3 x 3 grid of 10 m cells without the middle one: a 100 m² hole in a 900 m² outline
        let mut tris = Vec::new();
        for (cx, cz) in (0..9).map(|i| (i % 3, i / 3)).filter(|&c| c != (1, 1)) {
            let (x0, z0, x1, z1) = (cx as f32 * 10.0, cz as f32 * 10.0, cx as f32 * 10.0 + 10.0, cz as f32 * 10.0 + 10.0);
            let (a, b, c, d) = ([x0, -3.0, z0], [x1, -3.0, z0], [x1, -3.0, z1], [x0, -3.0, z1]);
            tris.extend([[a, b, c], [a, c, d]]);
        }
        assert!(hole_patches(&tris, 50.0).is_empty());
        let patch = hole_patches(&tris, 150.0);
        assert_eq!(patch.len(), 2);
        let area: f32 = patch.iter().map(|t| ((t[1][0] - t[0][0]) * (t[2][2] - t[0][2]) - (t[1][2] - t[0][2]) * (t[2][0] - t[0][0])).abs() / 2.0).sum();
        assert!((area - 100.0).abs() < 1e-3);
        // closed: only the outer 12 edges are left as shore
        let mut all = tris.clone();
        all.extend(patch);
        assert_eq!(shoreline(&all).len(), 12);
        // the outer boundary is never a hole, however large the limit
        assert_eq!(hole_patches(&tris, 1e9).len(), 2);
        let t = parse(&["Texture", "Pyttesnu", "Style", "ocean", "FillHoles", "1500"]).unwrap();
        assert_eq!(t.params.fill_holes, 1500.0);
        assert_eq!(parse(&to_text(&t).split_whitespace().skip(1).collect::<Vec<_>>()).unwrap(), t);
    }

    #[test]
    fn shore_distance() {
        // a 20 x 20 square of two triangles: the diagonal is inside, the four sides are shore
        let (a, b, c, d) = ([0.0, 0.0, 0.0], [20.0, 0.0, 0.0], [20.0, 0.0, 20.0], [0.0, 0.0, 20.0]);
        let shore = shoreline(&[[a, b, c], [a, c, d]]);
        assert_eq!(shore.len(), 4);
        let g = ShoreGrid::new(shore, 50.0);
        assert!((g.distance([10.0, 0.0, 10.0]) - 10.0).abs() < 1e-4);
        assert!((g.distance([3.0, 0.0, 10.0]) - 3.0).abs() < 1e-4);
        assert!(g.distance([0.0, 0.0, 5.0]) < 1e-4);
    }

    #[test]
    fn subdivided_neighbours_share_their_edge_vertices() {
        // two triangles of a quad: the shared diagonal must carry the same points from both sides
        let (a, b, c, d) = ([0.0, 0.0, 0.0], [37.0, 0.0, 3.0], [41.0, 0.0, 29.0], [-2.0, 0.0, 22.0]);
        let (mut t1, mut t2) = (Vec::new(), Vec::new());
        subdivide([a, b, c], 4.0, &mut t1, 0);
        subdivide([a, c, d], 4.0, &mut t2, 0);
        let on_diagonal = |ts: &[[[f32; 3]; 3]]| {
            let mut v: Vec<[i32; 3]> = ts.iter().flatten().filter(|p| {
                let (ac, ap) = ((c[0] - a[0], c[2] - a[2]), (p[0] - a[0], p[2] - a[2]));
                (ac.0 * ap.1 - ac.1 * ap.0).abs() < 1e-2
            }).map(|p| p.map(|x| (x * 100.0).round() as i32)).collect();
            v.sort();
            v.dedup();
            v
        };
        assert!(on_diagonal(&t1).len() > 4);
        assert_eq!(on_diagonal(&t1), on_diagonal(&t2));
    }

    #[test]
    fn subdivide_limits_edges() {
        let mut out = Vec::new();
        subdivide([[0.0, 0.0, 0.0], [20.0, 0.0, 0.0], [0.0, 0.0, 20.0]], 4.0, &mut out, 0);
        assert!(out.len() > 8);
        for t in out {
            for k in 0..3 {
                let (a, b) = (t[k], t[(k + 1) % 3]);
                assert!(((a[0] - b[0]).powi(2) + (a[2] - b[2]).powi(2)).sqrt() <= 4.0 + 1e-3);
            }
        }
    }
}
