//! REMASTER sharp HUD (09-remaster 9.12): the race HUD's art as signed distance fields, so it
//! draws sharp at the window's own resolution instead of a bilinear blow-up of 640x480 art.
//! Built at first use from the install's own images; nothing is shipped.
//!
//! The HUD sprites are a flat fill inside a dark outline about 1 px wide, and the levels
//! between the two are the art's antialiasing [O: Data/d3d/*.tga, the minimap IFFs]. Each
//! sprite becomes two fields: R = the silhouette (alpha), G = the fill (the level between
//! outline and fill colour). The shader draws the outline colour inside R and the fill
//! colour inside G, both with exact edge coverage. Antialiased sprites are contoured on the
//! bilinear 0.5 iso-line of those levels, so slopes and curves come out smooth; binary ones
//! (myfont, the tiny map numbers) as texel squares, so pixel fonts stay pixel-exact
//! [design choice].

use crate::ui::{Atlas, FntFont, GridFont, Rect, ATLAS};
use crate::ui_font::{self, HudFont};
use std::collections::HashMap;

/// distance range of a channel in source pixels each way (0..1 in the texture); also the
/// border around each tile, so the silhouette's fall-off fits. ui.wgsl's `SDF_RANGE`.
pub const RANGE: f32 = 3.0;
pub const BORDER: u32 = 3;
/// stored texels per source pixel; images over `BIG` pixels (the minimaps) get half
const SCALE: u32 = 4;
const BIG: u32 = 64 * 64;
const WIDTH: u32 = 2048;
const PAD: u32 = 1;

#[derive(Clone, Copy, Debug)]
pub struct Tile {
    /// texels in the field atlas, `BORDER` source pixels around the image
    pub rect: Rect,
    /// source image size, pixels
    pub size: (u32, u32),
    pub fill: [f32; 3],
    pub outline: [f32; 3],
}

/// The field atlas: RG8 texels, shelf packed.
pub struct SdfAtlas {
    pub width: u32,
    pub height: u32,
    pub data: Vec<u8>,
    tiles: HashMap<String, Tile>,
    glyphs: HashMap<char, Tile>,
    /// FONT1.FNT glyphs (`Batch::fnt_text`)
    fnts: HashMap<char, Tile>,
    /// REMASTER modern HUD fonts (9.21): `size` is the glyph's `ui_font` bbox
    fonts: HashMap<(HudFont, char), Tile>,
}

impl SdfAtlas {
    /// Fields for the minimap art and the myfont glyphs found in `atlas`, and
    /// the modern HUD fonts' glyphs.
    pub fn build(atlas: &Atlas, font: Option<&GridFont>, fnt: Option<&FntFont>) -> SdfAtlas {
        let mut jobs: Vec<(Key, Rect)> = atlas
            .names()
            // the minimap art (drawn when there is no generated map); the HUD art's fields
            // went with 9.21 phase 7 (the modern HUD draws fonts and shaders)
            .filter(|(n, _)| n.starts_with("map:"))
            .map(|(n, r)| (Key::Name(n.clone()), r))
            .collect();
        if let Some(f) = font {
            if let Some(r) = atlas.get(&f.image) {
                // printable ASCII; other chars draw from the bitmap
                for c in (33u8..=126).map(char::from) {
                    let (gx, gy, gw) = f.glyph(c);
                    jobs.push((Key::Glyph(c), Rect { x: r.x + gx, y: r.y + gy, w: gw, h: f.cell_h }));
                }
            }
        }
        // the original menu font (FONT1.FNT): every char with ink, texel squares like myfont
        if let Some(f) = fnt {
            if let Some(r) = atlas.get(&f.image) {
                for c in (33u32..=255).filter_map(char::from_u32) {
                    let (gx, gy, gw, _) = f.glyph(c);
                    if gw > 0 {
                        jobs.push((Key::Fnt(c), Rect { x: r.x + gx, y: r.y + gy, w: gw, h: f.height }));
                    }
                }
            }
        }
        let none = Rect { x: 0, y: 0, w: 0, h: 0 };
        for f in HudFont::ALL {
            jobs.extend(ui_font::charset(f).into_iter().filter(|&c| ui_font::face(f).glyph(c).is_some()).map(|c| (Key::Font(f, c), none)));
        }
        // on all cores; tallest first packs the shelves tighter
        let threads = std::thread::available_parallelism().map_or(4, |n| n.get());
        let chunk = jobs.len().div_ceil(threads).max(1);
        let mut fields: Vec<(Key, Field)> = std::thread::scope(|sc| {
            let work: Vec<_> = jobs
                .chunks(chunk)
                .map(|part| {
                    sc.spawn(move || {
                        part.iter()
                            .filter_map(|(k, r)| {
                                let f = match k {
                                    Key::Font(font, c) => Field::glyph(*font, *c),
                                    _ => Field::new(&pixels(atlas, *r), r.w, r.h),
                                };
                                Some((k.clone(), f?))
                            })
                            .collect::<Vec<_>>()
                    })
                })
                .collect();
            work.into_iter().flat_map(|h| h.join().unwrap_or_default()).collect()
        });
        // the menu font's fields go last, so they leave every other tile where it was
        fields.sort_by_key(|(k, f)| (matches!(k, Key::Fnt(_)), std::cmp::Reverse(f.h)));
        let mut out = SdfAtlas { width: WIDTH, height: 0, data: Vec::new(), tiles: HashMap::new(), glyphs: HashMap::new(), fnts: HashMap::new(), fonts: HashMap::new() };
        let (mut x, mut y, mut row) = (0, 0, 0);
        for (key, f) in fields {
            if x + f.w + PAD > WIDTH {
                (x, y, row) = (0, y + row + PAD, 0);
            }
            out.height = out.height.max(y + f.h + PAD);
            out.data.resize((WIDTH * out.height * 2) as usize, 0);
            for r in 0..f.h {
                let dst = (((y + r) * WIDTH + x) * 2) as usize;
                out.data[dst..][..(f.w * 2) as usize].copy_from_slice(&f.texels[(r * f.w * 2) as usize..][..(f.w * 2) as usize]);
            }
            let tile = Tile { rect: Rect { x, y, w: f.w, h: f.h }, size: f.size, fill: f.fill, outline: f.outline };
            match key {
                Key::Name(n) => out.tiles.insert(n, tile),
                Key::Glyph(c) => out.glyphs.insert(c, tile),
                Key::Fnt(c) => out.fnts.insert(c, tile),
                Key::Font(f, c) => out.fonts.insert((f, c), tile),
            };
            x += f.w + PAD;
            row = row.max(f.h);
        }
        out.height = out.height.max(1);
        out.data.resize((WIDTH * out.height * 2) as usize, 0);
        out
    }

    pub fn tile(&self, name: &str) -> Option<&Tile> {
        self.tiles.get(name)
    }

    pub fn glyph(&self, c: char) -> Option<&Tile> {
        self.glyphs.get(&c)
    }

    /// A FONT1.FNT glyph's field
    pub fn fnt_glyph(&self, c: char) -> Option<&Tile> {
        self.fnts.get(&c)
    }

    /// REMASTER modern HUD: a `ui_font` glyph's field
    pub fn font_glyph(&self, font: HudFont, c: char) -> Option<&Tile> {
        self.fonts.get(&(font, c))
    }
}

#[derive(Clone)]
enum Key {
    Name(String),
    Glyph(char),
    Fnt(char),
    Font(HudFont, char),
}

/// RGBA pixels of an atlas rectangle.
fn pixels(atlas: &Atlas, r: Rect) -> Vec<u8> {
    (0..r.h).flat_map(|y| atlas.rgba[(((r.y + y) * ATLAS + r.x) * 4) as usize..][..(r.w * 4) as usize].iter().copied()).collect()
}

/// One sprite's two fields, encoded.
struct Field {
    w: u32,
    h: u32,
    size: (u32, u32),
    texels: Vec<u8>,
    fill: [f32; 3],
    outline: [f32; 3],
}

impl Field {
    /// None for an empty sprite (the space glyph).
    fn new(rgba: &[u8], w: u32, h: u32) -> Option<Field> {
        let n = (w * h) as usize;
        let lum = |i: usize| rgba[i * 4..i * 4 + 3].iter().copied().max().unwrap_or(0) as f32 / 255.0;
        let opaque = |i: usize| rgba[i * 4 + 3] > 0;
        let top = (0..n).filter(|&i| opaque(i)).map(lum).fold(f32::NAN, f32::max);
        if top.is_nan() {
            return None;
        }
        // two levels (1D 2-means over the opaque pixels): outline and fill; each colour is
        // its level's most common one, so antialiasing shades and a few odd pixels (the
        // minimaps' white start line) don't shift it
        let (mut lo, mut hi) = (0.0f32, top);
        for _ in 0..8 {
            let (mut s, mut k) = ([0.0f32; 2], [0u32; 2]);
            for i in (0..n).filter(|&i| opaque(i)) {
                let c = (lum(i) > (lo + hi) * 0.5) as usize;
                s[c] += lum(i);
                k[c] += 1;
            }
            lo = if k[0] > 0 { s[0] / k[0] as f32 } else { lo };
            hi = if k[1] > 0 { s[1] / k[1] as f32 } else { hi };
        }
        let split = (lo + hi) * 0.5;
        let mode = |bright: bool| -> Option<([f32; 3], f32)> {
            let mut count: HashMap<[u8; 3], u32> = HashMap::new();
            for i in (0..n).filter(|&i| opaque(i) && (lum(i) > split) == bright) {
                *count.entry([rgba[i * 4], rgba[i * 4 + 1], rgba[i * 4 + 2]]).or_default() += 1;
            }
            let (c, _) = count.into_iter().max_by_key(|&(c, k)| (k, c))?;
            Some((c.map(|v| v as f32 / 255.0), c.iter().copied().max().unwrap_or(0) as f32 / 255.0))
        };
        let (fill, fill_lum) = mode(true).or_else(|| mode(false))?;
        // a real outline is much darker than the fill (not just a shade of it, as on mapnr)
        let dark = mode(false).filter(|&(_, l)| l < 0.35 * fill_lum);
        let (outline, low) = dark.unwrap_or((fill, 0.0));
        // fill level: outline 0 .. fill 1; without an outline the whole silhouette is fill
        let inner: Vec<f32> = (0..n)
            .map(|i| match (opaque(i), dark.is_some()) {
                (false, _) => 0.0,
                (true, true) => ((lum(i) - low) / (fill_lum - low).max(1e-3)).clamp(0.0, 1.0),
                (true, false) => 1.0,
            })
            .collect();
        let outer: Vec<f32> = (0..n).map(|i| opaque(i) as u32 as f32).collect();
        let smooth = w.max(h) > 8 && (0..n).any(|i| opaque(i) && (0.1..0.9).contains(&(lum(i) / top)));
        let scale = if w * h > BIG { SCALE / 2 } else { SCALE };
        let (r, g) = (channel(&outer, w, h, scale, smooth), channel(&inner, w, h, scale, smooth));
        let texels = r.iter().zip(&g).flat_map(|(&a, &b)| [a, b]).collect();
        let (tw, th) = ((w + 2 * BORDER) * scale, (h + 2 * BORDER) * scale);
        Some(Field { w: tw, h: th, size: (w, h), texels, fill, outline })
    }
}

impl Field {
    /// A TrueType glyph (`ui_font`): one silhouette in both channels, white.
    fn glyph(font: HudFont, c: char) -> Option<Field> {
        let [x0, y0, x1, y1] = ui_font::face(font).glyph(c)?.bbox?;
        let (inside, gw, gh, sup) = ui_font::raster(font, c, BORDER)?;
        let r = encode(&inside, gw, gh, ui_font::FINE, sup);
        let texels = r.iter().flat_map(|&a| [a, a]).collect();
        Some(Field { w: gw / ui_font::FINE, h: gh / ui_font::FINE, size: ((x1 - x0) as u32, (y1 - y0) as u32), texels, fill: [1.0; 3], outline: [1.0; 3] })
    }
}

/// Signed distance to the 0.5 level of `cov` (bilinear between pixel centres if `smooth`,
/// else texel squares), sampled at `scale` texels per pixel over the image plus `BORDER`,
/// encoded 0.5 + d / (2 RANGE), inside positive.
fn channel(cov: &[f32], w: u32, h: u32, scale: u32, smooth: bool) -> Vec<u8> {
    // a smooth level is classified on a grid twice as fine and the 2x2 distances averaged
    // (halves the stair error of the binary grid); texel squares sit on the grid already
    let fine = if smooth { 2 } else { 1 };
    let sup = scale * fine;
    let (gw, gh) = ((w + 2 * BORDER) * sup, (h + 2 * BORDER) * sup);
    let at = |x: i32, y: i32| if x < 0 || y < 0 || x >= w as i32 || y >= h as i32 { 0.0 } else { cov[(y as u32 * w + x as u32) as usize] };
    let inside: Vec<bool> = (0..gh)
        .flat_map(|gy| (0..gw).map(move |gx| (gx, gy)))
        .map(|(gx, gy)| {
            let (px, py) = ((gx as f32 + 0.5) / sup as f32 - BORDER as f32, (gy as f32 + 0.5) / sup as f32 - BORDER as f32);
            let v = if smooth {
                let (fx, fy) = (px - 0.5, py - 0.5);
                let (x0, y0) = (fx.floor(), fy.floor());
                let (tx, ty) = (fx - x0, fy - y0);
                let (x0, y0) = (x0 as i32, y0 as i32);
                let top = at(x0, y0) * (1.0 - tx) + at(x0 + 1, y0) * tx;
                let bot = at(x0, y0 + 1) * (1.0 - tx) + at(x0 + 1, y0 + 1) * tx;
                top * (1.0 - ty) + bot * ty
            } else {
                at(px.floor() as i32, py.floor() as i32)
            };
            v >= 0.5
        })
        .collect();
    encode(&inside, gw, gh, fine, sup)
}

/// Signed distance to the edge of `inside` (a grid of `sup` cells per source pixel),
/// averaged over `fine` x `fine` cells per texel and encoded 0.5 + d / (2 RANGE).
fn encode(inside: &[bool], gw: u32, gh: u32, fine: u32, sup: u32) -> Vec<u8> {
    let to_out = edt(inside, gw, gh, false);
    let to_in = edt(inside, gw, gh, true);
    let d = |i: usize| if inside[i] { to_out[i] - 0.5 } else { 0.5 - to_in[i] };
    let (tw, th) = (gw / fine, gh / fine);
    let mut out = Vec::with_capacity((tw * th) as usize);
    for ty in 0..th {
        for tx in 0..tw {
            let mut s = 0.0;
            for (x, y) in (0..fine * fine).map(|k| (tx * fine + k % fine, ty * fine + k / fine)) {
                s += d((y * gw + x) as usize);
            }
            let s = s / (fine * fine * sup) as f32;
            out.push(((0.5 + s / (2.0 * RANGE)).clamp(0.0, 1.0) * 255.0).round() as u8);
        }
    }
    out
}

/// Euclidean distance (cells) from each cell to the nearest cell whose `mask` is `target`
/// (Felzenszwalb & Huttenlocher's two-pass transform).
pub fn edt(mask: &[bool], w: u32, h: u32, target: bool) -> Vec<f32> {
    const INF: f32 = 1e20;
    let (w, h) = (w as usize, h as usize);
    let mut f: Vec<f32> = mask.iter().map(|&m| if m == target { 0.0 } else { INF }).collect();
    let n = w.max(h);
    let (mut line, mut out, mut v, mut z) = (vec![0.0; n], vec![0.0; n], vec![0usize; n], vec![0.0; n + 1]);
    for x in 0..w {
        for y in 0..h {
            line[y] = f[y * w + x];
        }
        dt1(&line[..h], &mut out[..h], &mut v, &mut z);
        for y in 0..h {
            f[y * w + x] = out[y];
        }
    }
    for y in 0..h {
        line[..w].copy_from_slice(&f[y * w..][..w]);
        dt1(&line[..w], &mut out[..w], &mut v, &mut z);
        f[y * w..][..w].copy_from_slice(&out[..w]);
    }
    f.iter().map(|d| d.sqrt()).collect()
}

/// 1D squared distance transform of `f` (lower envelope of parabolas).
fn dt1(f: &[f32], out: &mut [f32], v: &mut [usize], z: &mut [f32]) {
    let sq = |q: usize| (q * q) as f32;
    let mut k = 0;
    v[0] = 0;
    z[0] = f32::NEG_INFINITY;
    z[1] = f32::INFINITY;
    for q in 1..f.len() {
        // z[0] = -inf stops the walk at the first parabola
        let mut s = ((f[q] + sq(q)) - (f[v[k]] + sq(v[k]))) / (2.0 * (q - v[k]) as f32);
        while s <= z[k] {
            k -= 1;
            s = ((f[q] + sq(q)) - (f[v[k]] + sq(v[k]))) / (2.0 * (q - v[k]) as f32);
        }
        k += 1;
        v[k] = q;
        z[k] = s;
        z[k + 1] = f32::INFINITY;
    }
    k = 0;
    for (q, o) in out.iter_mut().enumerate() {
        while z[k + 1] < q as f32 {
            k += 1;
        }
        let d = q as f32 - v[k] as f32;
        *o = d * d + f[v[k]];
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn decode(b: u8) -> f32 {
        (b as f32 / 255.0 - 0.5) * 2.0 * RANGE
    }

    #[test]
    fn square_edge_on_texel_edge() {
        // a 4x4 opaque white square: the R field crosses zero on the image edge
        let rgba = [255u8; 4 * 4 * 4];
        let f = Field::new(&rgba, 4, 4).unwrap();
        assert_eq!((f.w, f.h), ((4 + 2 * BORDER) * SCALE, (4 + 2 * BORDER) * SCALE));
        let r = |x: u32, y: u32| decode(f.texels[((y * f.w + x) * 2) as usize]);
        let y = (BORDER + 2) * SCALE; // middle row
        let edge = BORDER * SCALE; // first texel inside the image
        assert!(r(edge - 1, y) < 0.0 && r(edge, y) > 0.0, "{} {}", r(edge - 1, y), r(edge, y));
        // texel centres sit 1/8 px either side of the edge
        assert!((r(edge, y) - 0.125).abs() < 0.03 && (r(edge - 1, y) + 0.125).abs() < 0.03);
        // deep inside: 2 px to the nearest edge
        assert!((r(edge + 2 * SCALE, y) - 2.0).abs() < 0.2);
    }

    #[test]
    fn edt_line() {
        let mask = [false, false, true, false, false, false];
        let d = edt(&mask, 6, 1, true);
        assert_eq!(d, vec![2.0, 1.0, 0.0, 1.0, 2.0, 3.0]);
    }
}
