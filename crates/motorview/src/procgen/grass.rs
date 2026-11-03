//! REMASTER grass (Gitea #30, re-notes/18-grass.md): shell turf over the world's grass
//! textures. The original draws a flat olive texture; Classic keeps it. Remaster draws every
//! tagged polygon `Shells` more times, lifted along its smoothed normal, and `fs_grass`
//! (grass.wgsl) keeps only the pixels inside hashed blade cells, so the verge gets depth next
//! to the car and fades back to the plain texture past `Reach`. The base polygon stays in the
//! world draw (it is shell 0), so nothing of the original is hidden.
//!
//! Bodies come from `remaster/fill/trackNN.fill` next to the water:
//! ```text
//! Grass Texture Gras1+Gras2 Style meadow [params]   # the world polygons with these textures
//! Grass none                                        # no grass on this track
//! params: Height m  Density blades/m  Thickness 0..1  Shells n  Reach m  Sway m
//!         RootShade 0..1  EdgeFade m  Tint r g b  Tip r g b  TipMix 0..1
//!         BladeDensity cells/m  BladeWidth m  BladeReach m  BladeBend 0..1   (the Ultra blades)
//! ```
//! `Style` sets the defaults, a parameter overrides it. A track without a `Grass` line gets
//! `auto_bodies`: every world texture whose stem starts with `gras` (but not the `grasmur`
//! wall), `Grasfad*` as a verge, `GrasPA*` as patches, the rest as a meadow [design choice].
//! At most `MAX_BODIES` bodies (the shader reads them from the globals).

use super::water::ShoreGrid;
use crate::scene::{Scene, Vertex};

pub const MAX_BODIES: usize = 4;
/// vec4 rows per body in the shader's globals (`Params::packed`)
pub const ROWS: usize = 5;
/// REMASTER Ultra blades (Gitea #31): the most blade records the compute pass keeps per
/// frame (32 bytes each, 64 MB; grass.wgsl `BLADE_MAX` is set from this). Sized for the
/// 250 % grass range: track 2's start counts 106k blades at 100 % and 660k at 250 % [design choice]
pub const ULTRA_MAX_BLADES: u32 = 1 << 21;
/// REMASTER Ultra blades: bent segments per blade (a strip of 2 x (segments + 1) vertices)
pub const BLADE_SEGMENTS: u32 = 3;
/// `Vertex::env` of a grass vertex is `GRASS_ENV + body index` (above the water's `WATER_ENV`)
pub const GRASS_ENV: f32 = 32.0;
/// a tagged triangle joins only when it faces up this much (its normal's y)
const UP_MIN: f32 = 0.5;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Style {
    #[default]
    Meadow,
    /// the `Grasfad*` fades along the road: shorter, denser
    Verge,
    /// the `GrasPA*` patches: short stubble
    Patch,
}

impl Style {
    pub const ALL: [Style; 3] = [Style::Meadow, Style::Verge, Style::Patch];
    pub const NAMES: [&'static str; 3] = ["meadow", "verge", "patch"];
    pub fn name(self) -> &'static str {
        Self::NAMES[self as usize]
    }
    pub fn parse(s: &str) -> Option<Self> {
        Self::NAMES.iter().position(|n| n.eq_ignore_ascii_case(s)).map(|i| Self::ALL[i])
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Params {
    pub style: Style,
    /// blade height at the top shell, metres
    pub height: f32,
    /// blade cells per metre
    pub density: f32,
    /// blade diameter at the root as a share of its cell, 0..2 (1: the blades touch)
    pub thickness: f32,
    /// shell count at the Full level (the level scales it, `GrassLevel::factor`)
    pub shells: u32,
    /// distance (metres) past which only the base texture shows; the shells thin out towards it
    pub reach: f32,
    /// wind lean of the tips, metres
    pub sway: f32,
    /// darkening at the root, 1 at the tip
    pub root_shade: f32,
    /// blade height fades to zero this close (metres) to the grass's outline
    pub edge_fade: f32,
    /// colour scale of the blades (the original texel keeps the track's colours)
    pub tint: [f32; 3],
    /// colour scale at the tips, mixed in by `tip_mix` × height
    pub tip: [f32; 3],
    pub tip_mix: f32,
    /// Ultra blades (18-grass.md "Ultra"): blade cells per metre
    pub blade_density: f32,
    /// Ultra blades: width at the root, metres
    pub blade_width: f32,
    /// Ultra blades: distance (metres) out to which blades are placed; the shells take over
    /// from 60 % of it
    pub blade_reach: f32,
    /// Ultra blades: how far the tips bend over (0 straight .. 1 = grass.wgsl `BLADE_BEND_MAX`)
    pub blade_bend: f32,
}

impl Params {
    /// The defaults of a style [design choice].
    pub fn new(style: Style) -> Self {
        // tuned on tracks 2, 3, 4, 5 and 8 (phase 2): clumps of ~10 cm rather than single blades,
        // since a 5 cm blade is under a pixel from the chase camera and reads as noise
        let base = Params { style, height: 0.35, density: 10.0, thickness: 1.3, shells: 16, reach: 80.0, sway: 0.08, root_shade: 0.45, edge_fade: 1.0, tint: [1.0; 3], tip: [1.1, 1.12, 0.85], tip_mix: 0.6, blade_density: 18.0, blade_width: 0.03, blade_reach: 35.0, blade_bend: 0.5 };
        match style {
            Style::Meadow => base,
            Style::Verge => Params { height: 0.22, density: 14.0, thickness: 1.2, sway: 0.05, blade_density: 20.0, blade_width: 0.025, blade_bend: 0.4, ..base },
            Style::Patch => Params { height: 0.12, density: 16.0, thickness: 1.1, sway: 0.04, blade_density: 24.0, blade_width: 0.02, blade_reach: 30.0, blade_bend: 0.3, ..base },
        }
    }

    /// The `ROWS` vec4 the shader reads: (height, density, thickness, shells), (reach, sway,
    /// root shade, edge fade), (tint, tip mix), (tip, 0), (blade density, width, reach, bend).
    pub fn packed(&self) -> [[f32; 4]; ROWS] {
        [
            [self.height, self.density, self.thickness, self.shells as f32],
            [self.reach, self.sway, self.root_shade, self.edge_fade],
            [self.tint[0], self.tint[1], self.tint[2], self.tip_mix],
            [self.tip[0], self.tip[1], self.tip[2], 0.0],
            [self.blade_density, self.blade_width, self.blade_reach, self.blade_bend],
        ]
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Body {
    /// world texture stems (case-insensitive, `.iff` optional)
    pub textures: Vec<String>,
    pub params: Params,
}

/// Parse the words after `Grass` (not `none`: fill.rs handles that).
pub fn parse(toks: &[&str]) -> Result<Body, String> {
    let mut textures = Vec::new();
    let mut kv: Vec<(&str, Vec<f32>)> = Vec::new();
    let mut style = Style::Meadow;
    let mut i = 0;
    while i < toks.len() {
        let key = toks[i];
        i += 1;
        if key == "Texture" || key == "Style" {
            let w = *toks.get(i).ok_or(format!("{key} needs a name"))?;
            i += 1;
            if key == "Texture" {
                textures.extend(w.split('+').filter(|n| !n.is_empty()).map(str::to_string));
            } else {
                style = Style::parse(w).ok_or(format!("unknown grass style '{w}'"))?;
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
    if textures.is_empty() {
        return Err("Grass needs Texture (or none)".into());
    }
    let mut p = Params::new(style);
    for (key, v) in &kv {
        let need = |k: usize| if v.len() >= k { Ok(()) } else { Err(format!("{key} needs {k} values")) };
        match *key {
            "Height" => { need(1)?; p.height = v[0].max(0.0) }
            "Density" => { need(1)?; p.density = v[0].max(0.1) }
            "Thickness" => { need(1)?; p.thickness = v[0].clamp(0.0, 2.0) }
            "Shells" => { need(1)?; p.shells = (v[0].round() as u32).clamp(1, 64) }
            "Reach" => { need(1)?; p.reach = v[0].max(1.0) }
            "Sway" => { need(1)?; p.sway = v[0].max(0.0) }
            "RootShade" => { need(1)?; p.root_shade = v[0].clamp(0.0, 1.0) }
            "EdgeFade" => { need(1)?; p.edge_fade = v[0].max(0.01) }
            "Tint" => { need(3)?; p.tint = [v[0], v[1], v[2]] }
            "Tip" => { need(3)?; p.tip = [v[0], v[1], v[2]] }
            "TipMix" => { need(1)?; p.tip_mix = v[0].clamp(0.0, 1.0) }
            "BladeDensity" => { need(1)?; p.blade_density = v[0].max(1.0) }
            "BladeWidth" => { need(1)?; p.blade_width = v[0].clamp(0.005, 0.1) }
            "BladeReach" => { need(1)?; p.blade_reach = v[0].max(1.0) }
            "BladeBend" => { need(1)?; p.blade_bend = v[0].clamp(0.0, 1.0) }
            _ => {}
        }
    }
    Ok(Body { textures, params: p })
}

/// The line for `Grass` (without the keyword), `parse`'s inverse. Only values that differ from
/// the style's defaults are written.
pub fn to_text(b: &Body) -> String {
    let mut l = format!("Grass Texture {} Style {}", b.textures.join("+"), b.params.style.name());
    let (p, d) = (&b.params, Params::new(b.params.style));
    let c1 = |l: &mut String, k: &str, v: f32, d: f32| if v != d { l.push_str(&format!(" {k} {v}")) };
    let c3 = |l: &mut String, k: &str, v: [f32; 3], d: [f32; 3]| if v != d { l.push_str(&format!(" {k} {} {} {}", v[0], v[1], v[2])) };
    c1(&mut l, "Height", p.height, d.height);
    c1(&mut l, "Density", p.density, d.density);
    c1(&mut l, "Thickness", p.thickness, d.thickness);
    if p.shells != d.shells {
        l.push_str(&format!(" Shells {}", p.shells));
    }
    c1(&mut l, "Reach", p.reach, d.reach);
    c1(&mut l, "Sway", p.sway, d.sway);
    c1(&mut l, "RootShade", p.root_shade, d.root_shade);
    c1(&mut l, "EdgeFade", p.edge_fade, d.edge_fade);
    c3(&mut l, "Tint", p.tint, d.tint);
    c3(&mut l, "Tip", p.tip, d.tip);
    c1(&mut l, "TipMix", p.tip_mix, d.tip_mix);
    c1(&mut l, "BladeDensity", p.blade_density, d.blade_density);
    c1(&mut l, "BladeWidth", p.blade_width, d.blade_width);
    c1(&mut l, "BladeReach", p.blade_reach, d.blade_reach);
    c1(&mut l, "BladeBend", p.blade_bend, d.blade_bend);
    l
}

/// A world texture name's stem, lower case (`Gras1.iff` -> `gras1`)
fn stem(name: &str) -> String {
    let n = name.to_ascii_lowercase();
    n.strip_suffix(".iff").unwrap_or(&n).to_string()
}

/// The style a grass texture gets by its name, None for the ones that are no ground grass
/// (`grasmur` is a wall) [design choice: the data has no material; 18-grass.md].
fn style_of(name: &str) -> Option<Style> {
    let s = stem(name);
    if !s.starts_with("gras") || s.starts_with("grasmur") {
        return None;
    }
    Some(if s.starts_with("grasfad") { Style::Verge } else if s.starts_with("graspa") { Style::Patch } else { Style::Meadow })
}

/// The bodies of a track without a `Grass` line: one per style with the world's grass textures
/// (each name once, in style order), none for a track without grass.
pub fn auto_bodies(world_textures: &[String]) -> Vec<Body> {
    Style::ALL
        .iter()
        .filter_map(|&style| {
            let mut textures: Vec<String> = Vec::new();
            for n in world_textures.iter().filter(|n| style_of(n) == Some(style)) {
                let s = stem(n);
                if !textures.contains(&s) {
                    textures.push(s);
                }
            }
            (!textures.is_empty()).then(|| Body { textures, params: Params::new(style) })
        })
        .collect()
}

/// The bodies a track draws: the fill's `Grass` lines, else `auto_bodies` over the world's
/// textures; none after `Grass none`.
pub fn track_bodies(fill: &super::fill::FillFile, world_textures: &[String]) -> Vec<Body> {
    if fill.grass_none {
        Vec::new()
    } else if fill.grass.is_empty() {
        auto_bodies(world_textures)
    } else {
        fill.grass.clone()
    }
}

/// What `build` makes of the bodies.
#[derive(Default)]
pub struct Mesh {
    /// the tagged world triangles with smoothed normals, `color.a` = distance to the grass's
    /// outline at the vertex (capped at the body's `EdgeFade`; the shader's fallback when there
    /// is no `edge`), `env` = `GRASS_ENV` + body
    pub verts: Vec<Vertex>,
    /// the outline distance per pixel (`EdgeMap`), None without grass
    pub edge: Option<EdgeMap>,
    /// per body: `Params::packed`
    pub params: Vec<[[f32; 4]; ROWS]>,
    /// per body: its style's name and triangle count (`Scene::stats`)
    pub counts: Vec<(&'static str, usize)>,
}

/// Mesh the bodies against the world of `s`: the up-facing world triangles with the bodies'
/// textures (same vertices, so the shells sit exactly on the original polygons), and the
/// triangles of the world fill `fill` (fill.rs `mesh`: the ground and patch pieces) whose
/// texture layer is one of the bodies'. The first `MAX_BODIES` bodies count.
pub fn build(bodies: &[Body], s: &Scene, fill: &[Vertex]) -> Mesh {
    let mut m = Mesh::default();
    if bodies.len() > MAX_BODIES {
        eprintln!("grass: only the first {MAX_BODIES} of {} bodies are used", bodies.len());
    }
    let bodies = &bodies[..bodies.len().min(MAX_BODIES)];
    let mut tris: Vec<([Vertex; 3], usize)> = Vec::new();
    let mut grass_tri = vec![false; s.world_tri_texture.len()];
    for (bi, b) in bodies.iter().enumerate() {
        m.params.push(b.params.packed());
        let before = tris.len();
        let mut ids: Vec<u32> = Vec::new();
        for name in &b.textures {
            let want = stem(name);
            let found: Vec<u32> = s.world_textures.iter().enumerate().filter(|(_, n)| stem(n) == want).map(|(i, _)| i as u32).collect();
            if found.is_empty() {
                eprintln!("grass: no world texture '{name}'");
            }
            ids.extend(found);
        }
        for (ti, t) in s.world_tri_texture.iter().enumerate() {
            if ids.contains(t) && s.tris.len() >= ti * 3 + 3 {
                let v = &s.tris[ti * 3..ti * 3 + 3];
                if v[0].normal[1] > UP_MIN {
                    tris.push(([v[0], v[1], v[2]], bi));
                    grass_tri[ti] = true;
                }
            }
        }
        // the fill's pieces carry only a texture layer: the layers of the body's textures, read
        // off the world's own triangles
        let layers: Vec<f32> = s.world_tri_texture.iter().enumerate().filter(|(_, t)| ids.contains(t)).filter_map(|(ti, _)| s.tris.get(ti * 3).map(|v| v.layer)).filter(|&l| l >= 0.0).collect();
        for v in fill.as_chunks::<3>().0 {
            if layers.contains(&v[0].layer) && v[0].normal[1] > UP_MIN {
                tris.push(([v[0], v[1], v[2]], bi));
            }
        }
        m.counts.push((b.params.style.name(), tris.len() - before));
    }
    let fades: Vec<f32> = bodies.iter().map(|b| b.params.edge_fade).collect();
    // the outline is where grass meets anything else: the edges of the other world triangles
    // (the road, kerbs, wall bases). Matching the grass's own edges would not do: the original's
    // polygons rarely share vertices with their neighbours (T-junctions, own vertices per
    // polygon), so most grass edges look like a border [design choice]
    let mut segs = Vec::new();
    for (ti, is_grass) in grass_tri.iter().enumerate() {
        if !is_grass && s.tris.len() >= ti * 3 + 3 {
            let v = &s.tris[ti * 3..ti * 3 + 3];
            for k in 0..3 {
                segs.push([v[k].pos, v[(k + 1) % 3].pos]);
            }
        }
    }
    (m.verts, m.edge) = shells_mesh(&tris, &fades, segs);
    m
}

/// The shell mesh of tagged triangles: normals averaged over shared positions (1 cm grid, so
/// adjacent faces of a bank lift together), `color.a` = distance to the nearest of the `border`
/// segments (the edges of the non-grass world), capped at the vertex's body's fade; and the
/// `EdgeMap` of the same distance per texel.
fn shells_mesh(tris: &[([Vertex; 3], usize)], fades: &[f32], border: Vec<[[f32; 3]; 2]>) -> (Vec<Vertex>, Option<EdgeMap>) {
    use std::collections::HashMap;
    let key = |p: [f32; 3]| p.map(|c| (c * 100.0).round() as i64);
    let mut sum: HashMap<[i64; 3], [f32; 3]> = HashMap::new();
    for (t, _) in tris {
        for v in t {
            let e = sum.entry(key(v.pos)).or_insert([0.0; 3]);
            for (a, b) in e.iter_mut().zip(v.normal) {
                *a += b;
            }
        }
    }
    let cap = fades.iter().copied().fold(0.01_f32, f32::max);
    let grid = ShoreGrid::new(border, cap);
    let mut out = Vec::with_capacity(tris.len() * 3);
    for (t, bi) in tris {
        let fade = fades.get(*bi).copied().unwrap_or(cap);
        out.extend(t.map(|v| {
            let n = sum[&key(v.pos)];
            let len = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
            let normal = if len > 1e-6 { n.map(|c| c / len) } else { v.normal };
            let edge = grid.distance(v.pos).min(fade);
            Vertex { color: [v.color[0], v.color[1], v.color[2], edge], normal, env: GRASS_ENV + *bi as f32, ..v }
        }));
    }
    let map = EdgeMap::rasterise(tris.iter().map(|(t, _)| t.map(|v| v.pos)), &grid, cap);
    (out, map)
}

/// The outline distance per pixel [design choice, Gitea #202]: a map over the grass's XZ
/// bounds, `TEXEL` metres per texel (coarser past `MAX_SIZE` texels a side), holding the
/// distance from the grass surface to the nearest non-grass edge as `cap` × value / 255, 0 off
/// the grass. The original's verges are one polygon wide with every vertex on the border, so a
/// per-vertex distance cannot ramp inside the strip (the earlier centroid splits made a tent per
/// triangle); the shader samples this instead (grass.wgsl `fs_grass`). The distance is taken in
/// 3D from the grass surface, so a road on a bridge does not fade the grass below it; two grass
/// layers at the same XZ share one texel (the smaller distance wins) [APPROX].
#[derive(Clone, Debug, PartialEq)]
pub struct EdgeMap {
    /// x, z of the map's corner (the vertex coordinate space)
    pub origin: [f32; 2],
    /// metres per texel
    pub texel: f32,
    /// texels along x and z
    pub size: [u32; 2],
    /// metres at 255
    pub cap: f32,
    /// row-major, z rows of x texels
    pub data: Vec<u8>,
}

/// Edge map texel size (m) and the most texels a side (16 MB of R8 at most)
const TEXEL: f32 = 0.5;
const MAX_SIZE: u32 = 4096;

impl EdgeMap {
    /// The map of the triangles `tris` (their positions): every texel whose centre is inside a
    /// triangle, or within half a texel of one (so a strip narrower than a texel still gets its
    /// texels), takes `grid`'s distance from the surface point there, capped at `cap`.
    fn rasterise(tris: impl Iterator<Item = [[f32; 3]; 3]> + Clone, grid: &ShoreGrid, cap: f32) -> Option<EdgeMap> {
        let (mut lo, mut hi) = ([f32::MAX; 2], [f32::MIN; 2]);
        for t in tris.clone() {
            for p in t {
                lo = [lo[0].min(p[0]), lo[1].min(p[2])];
                hi = [hi[0].max(p[0]), hi[1].max(p[2])];
            }
        }
        if lo[0] > hi[0] {
            return None;
        }
        let extent = (hi[0] - lo[0]).max(hi[1] - lo[1]) + 2.0 * TEXEL;
        let texel = TEXEL.max(extent / (MAX_SIZE - 2) as f32);
        let origin = [lo[0] - texel, lo[1] - texel];
        let size = [((hi[0] - origin[0]) / texel).ceil() as u32 + 1, ((hi[1] - origin[1]) / texel).ceil() as u32 + 1];
        let n = size[0] as usize * size[1] as usize;
        let mut data = vec![0u8; n];
        let mut covered = vec![false; n];
        let margin = texel * 0.5 * std::f32::consts::SQRT_2;
        for t in tris {
            let (a, b, c) = ([t[0][0], t[0][2]], [t[1][0], t[1][2]], [t[2][0], t[2][2]]);
            let area = (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0]);
            if area.abs() < 1e-6 {
                continue;
            }
            let tx = |p: [f32; 2]| ((p[0] - origin[0]) / texel, (p[1] - origin[1]) / texel);
            let (x0, x1) = (a[0].min(b[0]).min(c[0]) - margin, a[0].max(b[0]).max(c[0]) + margin);
            let (z0, z1) = (a[1].min(b[1]).min(c[1]) - margin, a[1].max(b[1]).max(c[1]) + margin);
            let (ix0, ix1) = (tx([x0, z0]).0.floor().max(0.0) as u32, (tx([x1, z1]).0.ceil() as u32).min(size[0] - 1));
            let (iz0, iz1) = (tx([x0, z0]).1.floor().max(0.0) as u32, (tx([x1, z1]).1.ceil() as u32).min(size[1] - 1));
            for iz in iz0..=iz1 {
                for ix in ix0..=ix1 {
                    let p = [origin[0] + (ix as f32 + 0.5) * texel, origin[1] + (iz as f32 + 0.5) * texel];
                    // barycentric coordinates of the texel centre
                    let w0 = ((b[0] - p[0]) * (c[1] - p[1]) - (b[1] - p[1]) * (c[0] - p[0])) / area;
                    let w1 = ((c[0] - p[0]) * (a[1] - p[1]) - (c[1] - p[1]) * (a[0] - p[0])) / area;
                    let w2 = 1.0 - w0 - w1;
                    let inside = w0 >= 0.0 && w1 >= 0.0 && w2 >= 0.0;
                    if !inside && [(a, b), (b, c), (c, a)].iter().all(|&(s, e)| seg_distance_2d(p, s, e) > margin) {
                        continue;
                    }
                    // the surface height there (the corners clamped for a texel just outside)
                    let w = [w0.max(0.0), w1.max(0.0), w2.max(0.0)];
                    let ws = (w[0] + w[1] + w[2]).max(1e-6);
                    let y = (w[0] * t[0][1] + w[1] * t[1][1] + w[2] * t[2][1]) / ws;
                    let d = grid.distance([p[0], y, p[1]]).min(cap);
                    let v = (d / cap * 255.0).round() as u8;
                    let i = iz as usize * size[0] as usize + ix as usize;
                    data[i] = if covered[i] { data[i].min(v) } else { v };
                    covered[i] = true;
                }
            }
        }
        Some(EdgeMap { origin, texel, size, cap, data })
    }

    /// The shader's transform (grass.wgsl): x, z of the corner and 1 / the extent along x, z,
    /// so uv = (p.xz - xy) * zw.
    pub fn transform(&self) -> [f32; 4] {
        [self.origin[0], self.origin[1], 1.0 / (self.size[0] as f32 * self.texel), 1.0 / (self.size[1] as f32 * self.texel)]
    }

    /// The map's value at a point (nearest texel), metres; 0 outside the map.
    #[cfg(test)]
    fn at(&self, x: f32, z: f32) -> f32 {
        let (ix, iz) = (((x - self.origin[0]) / self.texel).floor(), ((z - self.origin[1]) / self.texel).floor());
        if ix < 0.0 || iz < 0.0 || ix >= self.size[0] as f32 || iz >= self.size[1] as f32 {
            return 0.0;
        }
        self.data[iz as usize * self.size[0] as usize + ix as usize] as f32 / 255.0 * self.cap
    }
}

fn seg_distance_2d(p: [f32; 2], s: [f32; 2], e: [f32; 2]) -> f32 {
    let d = [e[0] - s[0], e[1] - s[1]];
    let w = [p[0] - s[0], p[1] - s[1]];
    let t = ((w[0] * d[0] + w[1] * d[1]) / (d[0] * d[0] + d[1] * d[1]).max(1e-12)).clamp(0.0, 1.0);
    let q = [w[0] - d[0] * t, w[1] - d[1] * t];
    (q[0] * q[0] + q[1] * q[1]).sqrt()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auto_rule() {
        let names: Vec<String> = ["Gras1.iff", "GRAS2.IFF", "Grasfad1.iff", "GrasPA2.iff", "grasmur.iff", "Grasokk.iff", "Sandv2.iff", "gras1.iff"].map(String::from).to_vec();
        let b = auto_bodies(&names);
        assert_eq!(b.len(), 3);
        assert_eq!((b[0].params.style, b[0].textures.clone()), (Style::Meadow, vec!["gras1".to_string(), "gras2".into(), "grasokk".into()]));
        assert_eq!((b[1].params.style, b[1].textures.clone()), (Style::Verge, vec!["grasfad1".to_string()]));
        assert_eq!((b[2].params.style, b[2].textures.clone()), (Style::Patch, vec!["graspa2".to_string()]));
        assert!(auto_bodies(&["Road.iff".to_string(), "grasmur.iff".into()]).is_empty());
    }

    #[test]
    fn round_trip() {
        let line = "Texture Gras1+Gras2 Style verge Height 0.3 Shells 8 Tint 0.9 1 0.8 TipMix 0.25 BladeWidth 0.04 BladeReach 0.5";
        let b = parse(&line.split_whitespace().collect::<Vec<_>>()).unwrap();
        assert_eq!(b.textures, vec!["Gras1".to_string(), "Gras2".into()]);
        assert_eq!((b.params.style, b.params.height, b.params.shells), (Style::Verge, 0.3, 8));
        assert_eq!((b.params.tint, b.params.tip_mix), ([0.9, 1.0, 0.8], 0.25));
        // the blade keys, clamped (reach at least 1 m)
        assert_eq!((b.params.blade_width, b.params.blade_reach), (0.04, 1.0));
        // the verge's own defaults stay
        assert_eq!(b.params.density, Params::new(Style::Verge).density);
        assert_eq!(b.params.blade_density, 20.0);
        assert_eq!(b.params.packed()[4], [20.0, 0.04, 1.0, 0.4]);
        let t = to_text(&b);
        let b2 = parse(&t.split_whitespace().skip(1).collect::<Vec<_>>()).unwrap();
        assert_eq!(b, b2);
        assert!(parse(&["Style", "meadow"]).is_err());
        assert!(parse(&["Texture", "Gras1", "Style", "lawn"]).is_err());
    }

    #[test]
    fn edge_distance_and_smoothed_normals() {
        // a 20 x 20 square fanned around its centre: the corners lie on the outline, the centre
        // is 10 m in (capped at the fade); the fan's faces tilt, the shared normals average
        let v = |p: [f32; 3], n: [f32; 3]| Vertex { pos: p, uv: [0.0; 2], color: [0.5, 0.6, 0.4, 1.0], layer: 0.0, normal: n, env: 0.0 };
        let c = [10.0, 0.0, 10.0];
        let corners = [[0.0, 0.0, 0.0], [20.0, 0.0, 0.0], [20.0, 0.0, 20.0], [0.0, 0.0, 20.0]];
        let tilt = [[0.6, 0.8, 0.0], [-0.6, 0.8, 0.0], [0.0, 0.8, 0.6], [0.0, 0.8, -0.6]];
        let tris: Vec<([Vertex; 3], usize)> = (0..4).map(|i| ([v(c, tilt[i]), v(corners[i], tilt[i]), v(corners[(i + 1) % 4], tilt[i])], 0)).collect();
        let border: Vec<[[f32; 3]; 2]> = (0..4).map(|i| [corners[i], corners[(i + 1) % 4]]).collect();
        let (out, map) = shells_mesh(&tris, &[1.0], border);
        assert_eq!(out.len(), 12);
        // the edge map: full at the centre, nothing outside, a ramp at the border
        let map = map.unwrap();
        assert!((map.at(10.0, 10.0) - 1.0).abs() < 1e-3);
        assert!((map.at(10.0, 1.25) - 1.0).abs() < 0.3);
        assert!(map.at(10.0, 0.25) < 0.5);
        assert_eq!(map.at(10.0, -2.0), 0.0);
        assert_eq!(map.at(50.0, 50.0), 0.0);
        let t = map.transform();
        assert!(((10.0 - t[0]) * t[2] - 0.5).abs() < 0.1);
        for o in &out {
            let edge = o.color[3];
            if o.pos == c {
                assert!((edge - 1.0).abs() < 1e-4, "centre edge {edge}");
                // the four tilts cancel: straight up
                assert!((o.normal[1] - 1.0).abs() < 1e-4 && o.normal[0].abs() < 1e-4 && o.normal[2].abs() < 1e-4);
            } else {
                assert!(edge < 1e-4, "corner edge {edge}");
                assert!(o.normal[1] > 0.7 && o.normal[1] <= 1.0 + 1e-6);
            }
            assert_eq!(o.env, GRASS_ENV);
            assert_eq!(&o.color[..3], &[0.5, 0.6, 0.4]);
        }
    }
}
