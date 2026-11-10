//! 2D layer: a texture atlas of the game's own front-end and HUD art, fonts, and a quad
//! batch drawn on a virtual canvas (640x400 for menus, 640x480 for the HUD, like the
//! original) scaled to the window.
//!
//! Art sources (re-notes/05-formats.md): `Data/MENU.SPR` (8-bit compiled sprites: titles,
//! dials, labels in 8 highlight frames, letters), `Data/d3d/*.tga` (Direct3D HUD, magenta
//! key), `Data/d3d/myfont.iff` (16x16-cell ASCII font), backgrounds `Data/back.tga`,
//! `Data/Results.TGA`, `Data/Loading.TGA`, per-track and per-car `_menuimg.TGA` pictures and the original minimaps,
//! and driver portraits `League/Profiles/Gfx/*.tga`.

use crate::revmeter::{self, RevMeterTune};
use crate::ui_font::{self, HudFont};
use crate::ui_sdf::{SdfAtlas, Tile, BORDER, RANGE};
use crate::ui_vector;
use motor_data::{fnt, fs, iff, spr, tga};
use std::cell::OnceCell;
use std::collections::HashMap;
use std::path::Path;

pub const ATLAS: u32 = 4096;
const PAD: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rect {
    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,
}

/// Shelf-packed RGBA atlas. An image the shelf has no room for (`reserve`) goes into the first hole
/// between the placed ones, so what is already placed never moves.
pub struct Atlas {
    pub rgba: Vec<u8>,
    rects: HashMap<String, Rect>,
    cursor: (u32, u32, u32),
}

impl Default for Atlas {
    fn default() -> Self {
        let mut a = Atlas { rgba: vec![0; (ATLAS * ATLAS * 4) as usize], rects: HashMap::new(), cursor: (0, 0, 0) };
        a.add("white", 4, 4, &[255; 64]);
        a
    }
}

impl Atlas {
    pub fn add(&mut self, name: &str, w: u32, h: u32, rgba: &[u8]) -> Option<Rect> {
        if rgba.len() < (w * h * 4) as usize {
            return None;
        }
        let rect = self.reserve(name, w, h)?;
        self.put(rect, name, w, h, rgba)
    }

    /// Room for an image of up to w × h, filled later by `put`.
    pub fn reserve(&mut self, name: &str, w: u32, h: u32) -> Option<Rect> {
        if w == 0 || h == 0 {
            return None;
        }
        let (mut x, mut y, mut row) = self.cursor;
        if x + w + PAD > ATLAS {
            (x, y, row) = (0, y + row + PAD, 0);
        }
        if y + h + PAD > ATLAS {
            // the shelf is out of rows: look for a hole under or beside the placed images
            let Some(rect) = self.find_hole(w, h) else {
                eprintln!("ui atlas full, dropping {name}");
                return None;
            };
            self.rects.insert(name.to_string(), rect);
            return Some(rect);
        }
        let rect = Rect { x, y, w, h };
        self.cursor = (x + w + PAD, y, row.max(h));
        self.rects.insert(name.to_string(), rect);
        Some(rect)
    }

    /// The lowest, then leftmost, spot for a w × h image that overlaps nothing placed (with PAD
    /// between): candidates are the atlas corner and the right and bottom edges of placed images.
    fn find_hole(&self, w: u32, h: u32) -> Option<Rect> {
        let apart = |a: &Rect, b: &Rect| a.x + a.w + PAD <= b.x || b.x + b.w + PAD <= a.x || a.y + a.h + PAD <= b.y || b.y + b.h + PAD <= a.y;
        let mut best: Option<Rect> = None;
        let corners = self.rects.values().flat_map(|r| [(r.x + r.w + PAD, r.y), (r.x, r.y + r.h + PAD)]);
        for (x, y) in std::iter::once((0, 0)).chain(corners) {
            let c = Rect { x, y, w, h };
            if x + w + PAD > ATLAS || y + h + PAD > ATLAS || best.is_some_and(|b| (b.y, b.x) <= (y, x)) {
                continue;
            }
            if self.rects.values().all(|o| apart(&c, o)) {
                best = Some(c);
            }
        }
        best
    }

    /// Copy a w × h image into the top left of `slot` (from `reserve`) under `name`.
    pub fn put(&mut self, slot: Rect, name: &str, w: u32, h: u32, rgba: &[u8]) -> Option<Rect> {
        if w > slot.w || h > slot.h || rgba.len() < (w * h * 4) as usize {
            return None;
        }
        for r in 0..h {
            let src = &rgba[(r * w * 4) as usize..][..(w * 4) as usize];
            self.rgba[(((slot.y + r) * ATLAS + slot.x) * 4) as usize..][..(w * 4) as usize].copy_from_slice(src);
        }
        let rect = Rect { x: slot.x, y: slot.y, w, h };
        self.rects.insert(name.to_string(), rect);
        Some(rect)
    }

    pub fn remove(&mut self, name: &str) {
        self.rects.remove(name);
    }

    pub fn get(&self, name: &str) -> Option<Rect> {
        self.rects.get(name).copied()
    }

    pub fn names(&self) -> impl Iterator<Item = (&String, Rect)> {
        self.rects.iter().map(|(n, r)| (n, *r))
    }
}

/// Proportional font cut from a fixed grid (myfont.iff: 16 columns of 16x12 cells, char 0
/// at the top left, so ' ' starts at y = 24).
pub struct GridFont {
    pub image: String,
    /// per char 32..=255: (x, y in the image, glyph width)
    glyphs: Vec<(u32, u32, u32)>,
    pub cell: u32,
    pub cell_h: u32,
}

impl GridFont {
    fn new(image: &str, w: u32, h: u32, rgba: &[u8], cell: u32, cell_h: u32) -> GridFont {
        let cols = w / cell;
        let mut glyphs = Vec::new();
        for c in 32u32..256 {
            let (cx, cy) = ((c % cols) * cell, (c / cols) * cell_h);
            if cy + cell_h > h {
                glyphs.push((0, 0, cell / 3));
                continue;
            }
            let mut used = (cell, 0);
            for y in cy..cy + cell_h {
                for x in cx..cx + cell {
                    let i = ((y * w + x) * 4) as usize;
                    if rgba.get(i + 3).is_some_and(|&a| a > 0) {
                        used = (used.0.min(x - cx), used.1.max(x - cx + 1));
                    }
                }
            }
            if used.1 == 0 {
                used = (0, cell / 3); // space
            }
            glyphs.push((cx + used.0, cy, used.1 - used.0));
        }
        GridFont { image: image.into(), glyphs, cell, cell_h }
    }

    pub(crate) fn glyph(&self, c: char) -> (u32, u32, u32) {
        let i = (c as u32).clamp(32, 255) - 32;
        self.glyphs[i as usize]
    }

    pub fn width(&self, s: &str) -> f32 {
        s.chars().map(|c| self.glyph(c).2 as f32 + 1.0).sum()
    }
}

/// The original front end's font, `Data/FONT1.FNT` (motor_data::fnt): the glyph masks rasterised
/// white on transparent into a 16-column grid of equal cells, drawn by `Batch::fnt_text`.
pub struct FntFont {
    pub image: String,
    pub height: u32,
    /// per char 32..=255: (x, y in the image, glyph width, advance)
    glyphs: Vec<(u32, u32, u32, u32)>,
}

impl FntFont {
    pub(crate) fn glyph(&self, c: char) -> (u32, u32, u32, u32) {
        self.glyphs[(c as u32).clamp(32, 255) as usize - 32]
    }

    pub fn width(&self, s: &str) -> f32 {
        s.chars().map(|c| self.glyph(c).3 as f32).sum()
    }
}

fn load_fnt(root: &Path, atlas: &mut Atlas) -> Option<FntFont> {
    let font = fnt::Font::parse(&fs::read(root, "Data/FONT1.FNT").ok()?, 1).ok()?;
    let (cols, cell_w) = (16u32, font.glyphs[32..].iter().map(|g| g.w).max()?.max(1));
    let (w, h) = (cols * cell_w, 14 * font.height);
    let mut rgba = vec![0u8; (w * h * 4) as usize];
    let mut glyphs = Vec::new();
    for c in 32u32..256 {
        let (cx, cy) = (((c - 32) % cols) * cell_w, ((c - 32) / cols) * font.height);
        let g = font.glyph(char::from_u32(c)?);
        for (y, row) in g.rows.iter().enumerate() {
            for x in 0..g.w {
                if row >> x & 1 != 0 {
                    let i = (((cy + y as u32) * w + cx + x) * 4) as usize;
                    rgba[i..i + 4].copy_from_slice(&[255; 4]);
                }
            }
        }
        glyphs.push((cx, cy, g.w, g.advance));
    }
    atlas.add("fnt1", w, h, &rgba)?;
    Some(FntFont { image: "fnt1".into(), height: font.height, glyphs })
}

/// Everything the front end and HUD draw.
pub struct Assets {
    pub atlas: Atlas,
    pub font: Option<GridFont>,
    /// the original menu font (FONT1.FNT), added after every other image
    pub fnt: Option<FntFont>,
    /// MENU.SPR sprite name -> frame count
    pub menu_frames: HashMap<String, usize>,
    /// REMASTER sharp HUD fields, built on first use (`sdf`)
    sdf: OnceCell<SdfAtlas>,
    /// REMASTER track photos (previews.rs), one at a time in the slot "tpv" (`show_preview`)
    previews: HashMap<u32, (u32, u32, Vec<u8>)>,
    preview_slot: Option<Rect>,
    preview_shown: Option<u32>,
    /// the movie's picture, one at a time in the slot "movie" (`show_movie`, #42)
    movie_slot: Option<Rect>,
}

fn magenta_key(rgba: &mut [u8]) {
    for p in rgba.chunks_mut(4) {
        if p[0] > 240 && p[1] < 16 && p[2] > 240 {
            p.copy_from_slice(&[0, 0, 0, 0]);
        }
    }
}

fn load_font(root: &Path, atlas: &mut Atlas) -> Option<GridFont> {
    let i = fs::read(root, "Data/d3d/myfont.iff").ok().and_then(|b| iff::decode(&b).ok())?;
    let mut rgba = i.rgba();
    // white glyphs on a dark backdrop: keep the bright pixels
    for p in rgba.chunks_mut(4) {
        if (p[0] as u32 + p[1] as u32 + p[2] as u32) < 300 {
            p[3] = 0;
        }
    }
    atlas.add("font", i.width, i.height, &rgba);
    Some(GridFont::new("font", i.width, i.height, &rgba, 16, 12))
}

/// REMASTER countdown art (09-remaster 9.21 phase 7; our art, `assets/hud/count/`): "cd:3",
/// "cd:2", "cd:1" and "cd:0" (GO), box-filtered from 1024² to `COUNT_SIZE` (the atlas has
/// no mips; `Batch::image_filtered` takes the rest).
pub const COUNT_SIZE: u32 = 448;
const COUNT_ART: [&[u8]; 4] = [
    include_bytes!("../../../assets/hud/count/count-go.png"),
    include_bytes!("../../../assets/hud/count/count01.png"),
    include_bytes!("../../../assets/hud/count/count02.png"),
    include_bytes!("../../../assets/hud/count/count03.png"),
];

fn load_countdown(atlas: &mut Atlas) {
    for (n, bytes) in COUNT_ART.iter().enumerate() {
        let Some((w, h, rgba)) = decode_rgba_png(bytes) else {
            eprintln!("countdown art {n}: not an 8-bit RGBA PNG");
            continue;
        };
        let (ow, oh) = (COUNT_SIZE.min(w), (h * COUNT_SIZE.min(w)).div_ceil(w));
        // premultiplied area filter (rows, then columns), so the transparent surround
        // doesn't darken the edges
        let pre: Vec<f32> = rgba.chunks(4).flat_map(|p| {
            let a = p[3] as f32 / 255.0;
            [p[0] as f32 * a, p[1] as f32 * a, p[2] as f32 * a, a]
        }).collect();
        let rows = area_filter(&pre, w, h, ow, true);
        let full = area_filter(&rows, ow, h, oh, false);
        let out: Vec<u8> = full.chunks(4).flat_map(|p| {
            let rgb = [0, 1, 2].map(|c| if p[3] > 0.0 { (p[c] / p[3]).round().min(255.0) as u8 } else { 0 });
            [rgb[0], rgb[1], rgb[2], (p[3] * 255.0).round().min(255.0) as u8]
        }).collect();
        atlas.add(&format!("cd:{n}"), ow, oh, &out);
    }
}

/// Box filter of a w × h, 4-channel image along x (`along_x`, to `n` columns) or y (to `n`
/// rows), each output texel the coverage-weighted mean of the source texels under it.
fn area_filter(src: &[f32], w: u32, h: u32, n: u32, along_x: bool) -> Vec<f32> {
    let (len, other) = if along_x { (w, h) } else { (h, w) };
    let f = len as f32 / n as f32;
    let (ow, oh) = if along_x { (n, h) } else { (w, n) };
    let mut out = vec![0.0f32; (ow * oh * 4) as usize];
    for o in 0..other {
        for i in 0..n {
            let (s0, s1) = (i as f32 * f, (i + 1) as f32 * f);
            let mut acc = [0.0f32; 4];
            for s in s0.floor() as u32..(s1.ceil() as u32).min(len) {
                let wgt = (s1.min(s as f32 + 1.0) - s0.max(s as f32)).max(0.0);
                let at = if along_x { o * w + s } else { s * w + o };
                for c in 0..4 {
                    acc[c] += src[(at * 4) as usize + c] * wgt;
                }
            }
            let to = if along_x { o * ow + i } else { i * ow + o };
            out[(to * 4) as usize..][..4].copy_from_slice(&acc.map(|x| x / f));
        }
    }
    out
}

/// An 8-bit RGBA PNG (the committed art; anything else is refused).
pub(crate) fn decode_rgba_png(bytes: &[u8]) -> Option<(u32, u32, Vec<u8>)> {
    let mut r = ::png::Decoder::new(std::io::Cursor::new(bytes)).read_info().ok()?;
    let mut buf = vec![0; r.output_buffer_size()];
    let info = r.next_frame(&mut buf).ok()?;
    if info.color_type != ::png::ColorType::Rgba || info.bit_depth != ::png::BitDepth::Eight {
        return None;
    }
    buf.truncate(info.buffer_size());
    Some((info.width, info.height, buf))
}

impl Assets {
    /// REMASTER: put track `t`'s photo into the atlas as "tpv" (the Track screen); returns the
    /// rect to upload (`Gpu::update_ui`), None when nothing changed or there is no photo.
    pub fn show_preview(&mut self, t: u32) -> Option<Rect> {
        if self.preview_shown == Some(t) {
            return None;
        }
        let slot = self.preview_slot?;
        let (w, h, rgba) = self.previews.get(&t)?;
        let r = self.atlas.put(slot, "tpv", *w, *h, rgba)?;
        self.preview_shown = Some(t);
        Some(r)
    }

    /// Put the movie's current picture (RGBA, w x h) into its slot as "movie"; returns the rect to upload (`Gpu::update_ui`).
    pub fn show_movie(&mut self, w: u32, h: u32, rgba: &[u8]) -> Option<Rect> {
        self.atlas.put(self.movie_slot?, "movie", w, h, rgba)
    }

    /// The photo of track `t` when it is the one in the slot (atlas name "tpv").
    pub fn preview_name(&self, t: u32) -> Option<&'static str> {
        (self.preview_shown == Some(t)).then_some("tpv")
    }

    pub fn load(root: &Path) -> Assets {
        let mut atlas = Atlas::default();
        let mut menu_frames = HashMap::new();
        let tga = |rel: &str| fs::read(root, rel).ok().and_then(|b| tga::decode(&b).ok());
        for (name, rel) in [("bg:menu", "Data/back.tga"), ("bg:results", "Data/Results.TGA"), ("bg:lresults", "Data/LResults.TGA"), ("bg:loading", "Data/Loading.TGA")] {
            if let Some(i) = tga(rel) {
                atlas.add(name, i.width, i.height, &i.rgba);
            }
        }
        // MENU.SPR: 8-bit compiled sprites, every frame
        if let Ok(bytes) = fs::read(root, "Data/MENU.SPR") {
            if let Ok(bank) = spr::parse(&bytes) {
                for s in &bank.sprites {
                    let mut n = 0;
                    for f in 0..s.frames {
                        if let Some((w, h, rgba)) = bank.rgba8(s, f) {
                            atlas.add(&format!("m:{}:{f}", s.name.to_ascii_lowercase()), w as u32, h as u32, &rgba);
                            n += 1;
                        }
                    }
                    menu_frames.insert(s.name.to_ascii_lowercase(), n);
                }
            }
        }
        // Direct3D HUD images (magenta = transparent)
        if let Some(dir) = fs::find_ci(root, "Data/d3d") {
            let mut files: Vec<_> = std::fs::read_dir(dir).map(|d| d.flatten().map(|e| e.path()).collect()).unwrap_or_default();
            files.sort();
            for p in files {
                let stem = p.file_stem().map(|s| s.to_string_lossy().to_ascii_lowercase()).unwrap_or_default();
                let ext = p.extension().map(|s| s.to_string_lossy().to_ascii_lowercase()).unwrap_or_default();
                let Ok(b) = std::fs::read(&p) else { continue };
                let img = match ext.as_str() {
                    "tga" => tga::decode(&b).ok().map(|i| (i.width, i.height, i.rgba)),
                    "iff" => iff::decode(&b).ok().map(|i| (i.width, i.height, i.rgba())),
                    _ => None,
                };
                if let Some((w, h, mut rgba)) = img {
                    // meter00 exists as a 127x40 TGA and a 64x21 IFF: the TGA is the HUD's
                    let key = format!("h:{stem}");
                    if ext == "iff" && atlas.get(&key).is_some() {
                        continue;
                    }
                    magenta_key(&mut rgba);
                    atlas.add(&key, w, h, &rgba);
                }
            }
        }
        let font = load_font(root, &mut atlas);
        // per-track previews and minimaps
        for t in 1..=8u32 {
            if let Some(i) = tga(&format!("Tracks/track{t}/Data/_menuimg.TGA")) {
                atlas.add(&format!("trk:{t}"), i.width, i.height, &i.rgba);
            }
            let trk = fs::config(root, &format!("Game/Track{t}.trk")).ok();
            let map = trk.as_ref().and_then(|c| c.get("TrackMapName")).and_then(|e| e.values.first().cloned());
            if let Some(i) = map.and_then(|m| fs::read(root, &format!("{m}.iff")).ok()).and_then(|b| iff::decode(&b).ok()) {
                // the background is whatever fills the corner: magenta on most maps, black on
                // U96's (u96.iff)
                let mut rgba = i.rgba();
                let key = [rgba[0], rgba[1], rgba[2]];
                for p in rgba.chunks_mut(4) {
                    if p[..3] == key {
                        p.copy_from_slice(&[0, 0, 0, 0]);
                    }
                }
                atlas.add(&format!("map:{t}"), i.width, i.height, &rgba);
            }
        }
        // driver portraits
        let mut pics: Vec<String> = vec!["Game/player.tga".into()];
        if let Some(dir) = fs::find_ci(root, "League/Profiles/Gfx") {
            if let Ok(d) = std::fs::read_dir(dir) {
                pics.extend(d.flatten().map(|e| format!("League/Profiles/Gfx/{}", e.file_name().to_string_lossy())));
            }
        }
        // REMASTER (#94): the duotone copies of the portraits for the modern Results screen, added last below
        let mut duos: Vec<(String, u32, u32, Vec<u8>)> = Vec::new();
        for rel in pics {
            if let Some(i) = tga(&rel) {
                atlas.add(&pic_key(&rel), i.width, i.height, &i.rgba);
                duos.push((duo_key(&rel), i.width, i.height, duotone(&i.rgba, i.width, i.height)));
            }
        }
        // the Track screen's photos: one slot as big as the largest
        let previews: HashMap<u32, (u32, u32, Vec<u8>)> = (1..=8u32).filter_map(|t| Some((t, crate::previews::load(root, t)?))).collect();
        let (pw, ph) = previews.values().fold((0, 0), |(w, h), m| (w.max(m.0), h.max(m.1)));
        let preview_slot = if previews.is_empty() { None } else { atlas.reserve("tpv", pw, ph) };
        // the intro movies: one 640x480 slot when the install has them
        let movie_slot = fs::find_ci(root, "movies/movies.pak").and_then(|_| atlas.reserve("movie", 640, 480));
        load_countdown(&mut atlas);
        let fnt = load_fnt(root, &mut atlas);
        // per-car photos of the Classic Car screen (0x4614e8: the car file's FileName directory + data\_menuimg.tga), last so nothing else moves in the atlas
        for id in 1..=10u32 {
            let dir = fs::config(root, &format!("Game/car{id:02}.car"))
                .ok()
                .and_then(|c| c.get("FileName").and_then(|e| e.values.first().cloned()))
                .and_then(|f| f.rsplit_once('\\').map(|(d, _)| d.to_string()));
            if let Some(i) = dir.and_then(|d| tga(&format!("{d}/data/_menuimg.TGA"))) {
                atlas.add(&format!("car:{id}"), i.width, i.height, &i.rgba);
            }
        }
        for (key, w, h, rgba) in &duos {
            atlas.add(key, *w, *h, rgba);
        }
        Assets { atlas, font, fnt, menu_frames, sdf: OnceCell::new(), previews, preview_slot, preview_shown: None, movie_slot }
    }

    /// Just the fonts: myfont, FONT1.FNT and the mono debug font (debug overlays).
    pub fn font_only(root: &Path) -> Assets {
        let mut atlas = Atlas::default();
        let font = load_font(root, &mut atlas);
        let fnt = load_fnt(root, &mut atlas);
        Assets { atlas, font, fnt, menu_frames: HashMap::new(), sdf: OnceCell::new(), previews: HashMap::new(), preview_slot: None, preview_shown: None, movie_slot: None }
    }

    /// REMASTER: the sharp HUD's distance fields (built the first time they are asked for)
    pub fn sdf(&self) -> &SdfAtlas {
        self.sdf.get_or_init(|| SdfAtlas::build(&self.atlas, self.font.as_ref(), self.fnt.as_ref()))
    }

    pub fn frames(&self, sprite: &str) -> usize {
        self.menu_frames.get(sprite).copied().unwrap_or(0)
    }
}

/// Atlas key of a portrait from its path (case-insensitive file name).
pub fn pic_key(path: &str) -> String {
    format!("pic:{}", path.rsplit(['/', '\\']).next().unwrap_or(path).to_ascii_lowercase())
}

/// Atlas key of a portrait's duotone copy (`duotone`).
pub fn duo_key(path: &str) -> String {
    format!("duo:{}", path.rsplit(['/', '\\']).next().unwrap_or(path).to_ascii_lowercase())
}

/// REMASTER (#94, design choice): the grey ID photo as a navy to ice-blue duotone with rounded corners, so the photo's
/// white studio wall reads as glass light on the Results screen. Luminance .3 .59 .11 through the three stops of the
/// mockup's `#duo` filter (R .02 .30 .66, G .04 .42 .86, B .09 .56 .95); the corners are cut at 12 texels (6 px of the
/// 46 px table portrait) with a one texel feather. One lookup per pixel.
pub fn duotone(rgba: &[u8], w: u32, h: u32) -> Vec<u8> {
    const STOPS: [[f32; 3]; 3] = [[0.02, 0.04, 0.09], [0.30, 0.42, 0.56], [0.66, 0.86, 0.95]];
    const R: f32 = 12.0;
    let mut out = Vec::with_capacity(rgba.len());
    for (i, p) in rgba.as_chunks::<4>().0.iter().enumerate() {
        let (x, y) = ((i as u32 % w) as f32 + 0.5, (i as u32 / w) as f32 + 0.5);
        let t = ((0.3 * p[0] as f32 + 0.59 * p[1] as f32 + 0.11 * p[2] as f32) / 255.0).clamp(0.0, 1.0) * 2.0;
        let (k, f) = ((t as usize).min(1), t - (t as usize).min(1) as f32);
        // the distance into the corner circle: its centre is R texels in from both edges
        let (dx, dy) = ((R - x).max(x - (w as f32 - R)).max(0.0), (R - y).max(y - (h as f32 - R)).max(0.0));
        let cover = (R - (dx * dx + dy * dy).sqrt() + 0.5).clamp(0.0, 1.0);
        out.extend(STOPS[k].iter().zip(&STOPS[k + 1]).map(|(a, z)| ((a + (z - a) * f) * 255.0).round() as u8));
        out.push((p[3] as f32 * cover).round() as u8);
    }
    out
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct UiVertex {
    /// virtual canvas pixels
    pub pos: [f32; 2],
    pub uv: [f32; 2],
    pub color: [f32; 4],
    /// ellipse clip in canvas pixels: centre xy, radii zw; 0 radii = none
    pub clip: [f32; 4],
    /// REMASTER: w = 1 for a distance-field quad (uv into the field atlas, `color` = fill,
    /// rgb = outline colour); w = 2 + o for a `ui_font` glyph (one field, an outline o field
    /// px wide outside it); w = -1 for the glass rev meter (x = the fill's cut x, uv in Blender
    /// units, `color`/`clip` = its tune); w = -2 for a box-filtered image (`image_filtered`, `clip` = its uv rect),
    /// -5 for a proximity arrow slice
    /// (`glass_glyph`: uv into the field atlas, `color`/`clip` = its look); 0 = an atlas texel quad, the only
    /// kind Classic draws
    pub aux: [f32; 4],
}

/// How the virtual canvas maps onto the window: uniform scale, centred, or (`stretch`) each axis
/// filling the window.
#[derive(Clone, Copy, Debug)]
pub struct Canvas {
    pub w: f32,
    pub h: f32,
    /// CLASSIC menus: the picture fills the window, as the original's fullscreen display mode
    /// does on the monitor (02-architecture "Window"); false = uniform scale with bars
    pub stretch: bool,
}

impl Canvas {
    /// Menu canvas: 640x400, letterboxed.
    pub const MENU: Canvas = Canvas { w: 640.0, h: 400.0, stretch: false };

    /// The movies' screen: the original's 640x480 mode (movie.rs).
    pub const MOVIE: Canvas = Canvas { w: 640.0, h: 480.0, stretch: false };

    /// Menu canvas, filling the window when `stretch` (the Menu picture option).
    pub fn menu(stretch: bool) -> Canvas {
        Canvas { stretch, ..Canvas::MENU }
    }

    /// Canvas of the window's own size (1 canvas px = 1 window px).
    pub fn window(size: (u32, u32)) -> Canvas {
        Canvas { w: size.0 as f32, h: size.1 as f32, stretch: false }
    }

    /// HUD canvas: 480 tall, as wide as the window's aspect needs (edges anchor to edges).
    pub fn hud(size: (u32, u32)) -> Canvas {
        Canvas { w: 480.0 * size.0 as f32 / size.1.max(1) as f32, h: 480.0, stretch: false }
    }

    /// (scale x, scale y, offset x, offset y) from canvas to window pixels.
    pub fn axes(&self, size: (u32, u32)) -> (f32, f32, f32, f32) {
        let (sx, sy) = (size.0 as f32 / self.w, size.1 as f32 / self.h);
        if self.stretch {
            return (sx, sy, 0.0, 0.0);
        }
        let s = sx.min(sy);
        (s, s, (size.0 as f32 - self.w * s) * 0.5, (size.1 as f32 - self.h * s) * 0.5)
    }

    /// (scale, offset x, offset y) from canvas to window pixels; the scale of a stretched canvas
    /// is its smaller axis (for pixel-snapping sizes).
    pub fn transform(&self, size: (u32, u32)) -> (f32, f32, f32) {
        let (sx, sy, ox, oy) = self.axes(size);
        (sx.min(sy), ox, oy)
    }

    /// Window pixel rectangle of a canvas rectangle.
    pub fn to_window(&self, size: (u32, u32), x: f32, y: f32, w: f32, h: f32) -> [f32; 4] {
        let (sx, sy, ox, oy) = self.axes(size);
        [ox + x * sx, oy + y * sy, w * sx, h * sy]
    }
}

/// The advance of `s` in a HUD font (no batch needed: hit rectangles are laid out the same way the text is drawn).
pub fn text_width(font: HudFont, s: &str, style: &FontStyle) -> f32 {
    let face = ui_font::face(font);
    let k = style.size / face.cap;
    let mut w = 0.0;
    let mut prev = None;
    for c in s.chars() {
        if let Some(p) = prev {
            w += face.kerning(p, c) * k;
        }
        prev = Some(c);
        w += match ui_font::resolve(font, c) {
            Some((f, g)) => (g.advance + style.tracking) * k * face.cap / ui_font::face(f).cap,
            None => face.glyph(' ').map_or(4.0, |g| g.advance) * k,
        };
    }
    w
}

pub struct Batch<'a> {
    pub assets: &'a Assets,
    pub canvas: Canvas,
    pub verts: Vec<UiVertex>,
    clip: [f32; 4],
    /// REMASTER sharp HUD: images and glyphs that have a field draw from it
    sdf: Option<&'a SdfAtlas>,
    /// REMASTER modern menus: opacity of all font text (1.0 elsewhere)
    text_alpha: f32,
    /// REMASTER: blurred glow sources of the meshes (`Gpu::draw_glow`)
    pub glow: Vec<UiVertex>,
}

pub const WHITE: [f32; 4] = [1.0; 4];

/// REMASTER: the glass rev meter's redline look (`Batch::rev_meter_sized`): `dim` fades the lit
/// part and its halo (0 = as is, the pulse's low phase), `rim` turns the glass edge red (0..1).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct RevRed {
    pub dim: f32,
    pub rim: f32,
}

/// REMASTER modern HUD: how `Batch::font_text` draws.
#[derive(Clone, Copy, Debug)]
pub struct FontStyle {
    /// capital height, canvas px
    pub size: f32,
    pub color: [f32; 4],
    /// outline colour and width (canvas px, outward; up to ~2.5/16 em)
    pub outline: [f32; 3],
    pub outline_w: f32,
    /// synthetic italic: the glyphs lean right by this many degrees (design choice for
    /// Teko, which has no italic)
    pub skew: f32,
    /// extra advance per glyph, field px (1/16 em)
    pub tracking: f32,
}

impl FontStyle {
    pub fn new(size: f32, color: [f32; 4]) -> FontStyle {
        FontStyle { size, color, outline: [0.0; 3], outline_w: 0.0, skew: 0.0, tracking: 0.0 }
    }

    pub fn outline(self, color: [f32; 3], width: f32) -> FontStyle {
        FontStyle { outline: color, outline_w: width, ..self }
    }

    pub fn skew(self, degrees: f32) -> FontStyle {
        FontStyle { skew: degrees, ..self }
    }

    pub fn tracking(self, px: f32) -> FontStyle {
        FontStyle { tracking: px, ..self }
    }
}

impl<'a> Batch<'a> {
    pub fn new(assets: &'a Assets, canvas: Canvas) -> Batch<'a> {
        Batch { assets, canvas, verts: Vec::new(), clip: [0.0; 4], sdf: None, text_alpha: 1.0, glow: Vec::new() }
    }

    /// REMASTER: a batch that draws HUD images and myfont from distance fields (draw it with
    /// `Gpu::draw_ui`'s field atlas)
    pub fn sharp(assets: &'a Assets, canvas: Canvas) -> Batch<'a> {
        Batch { sdf: Some(assets.sdf()), ..Batch::new(assets, canvas) }
    }

    /// REMASTER: `sharp` with all font text at `alpha` opacity (the modern menus, MENU_TEXT_ALPHA)
    pub fn sharp_text(assets: &'a Assets, canvas: Canvas, alpha: f32) -> Batch<'a> {
        Batch { text_alpha: alpha, ..Batch::sharp(assets, canvas) }
    }

    /// `sharp` picks `Batch::sharp` (REMASTER) over `Batch::new`
    pub fn new_for(assets: &'a Assets, canvas: Canvas, sharp: bool) -> Batch<'a> {
        if sharp {
            Batch::sharp(assets, canvas)
        } else {
            Batch::new(assets, canvas)
        }
    }

    pub fn is_sharp(&self) -> bool {
        self.sdf.is_some()
    }

    /// REMASTER: a field tile drawn where its source image covers (x, y, w, h); the quad
    /// grows by the tile's border. Fill and outline are tinted like the bitmap would be.
    fn field_quad(&mut self, t: &Tile, x: f32, y: f32, w: f32, h: f32, tint: [f32; 4]) {
        let color = [t.fill[0] * tint[0], t.fill[1] * tint[1], t.fill[2] * tint[2], tint[3]];
        let aux = [t.outline[0] * tint[0], t.outline[1] * tint[1], t.outline[2] * tint[2], 1.0];
        self.field_verts(t, [x, y, w, h], color, aux, (0.0, 0.0));
    }

    /// Field quad over `rect` (+ the tile border); `shear` = (baseline y, dx per px up).
    fn field_verts(&mut self, t: &Tile, rect: [f32; 4], color: [f32; 4], aux: [f32; 4], shear: (f32, f32)) {
        let Some(sdf) = self.sdf else { return };
        let [x, y, w, h] = rect;
        let (kx, ky) = (w / t.size.0 as f32, h / t.size.1 as f32);
        let b = BORDER as f32;
        let (x0, y0, x1, y1) = (x - b * kx, y - b * ky, x + w + b * kx, y + h + b * ky);
        let (aw, ah) = (sdf.width as f32, sdf.height as f32);
        let (u0, v0, u1, v1) = (t.rect.x as f32 / aw, t.rect.y as f32 / ah, (t.rect.x + t.rect.w) as f32 / aw, (t.rect.y + t.rect.h) as f32 / ah);
        let clip = self.clip;
        let v = |px: f32, py: f32, u: f32, vv: f32| UiVertex { pos: [px + (shear.0 - py) * shear.1, py], uv: [u, vv], color, clip, aux };
        let (p0, p1, p2, p3) = (v(x0, y0, u0, v0), v(x1, y0, u1, v0), v(x1, y1, u1, v1), v(x0, y1, u0, v1));
        self.verts.extend([p0, p1, p2, p0, p2, p3]);
    }

    /// REMASTER glass rev meter (`revmeter`, `rev_meter` in ui.wgsl) with its box at (x, y)
    /// (`revmeter::size()`), filled to `frac`; one quad, grown by the halo. Draws only in a
    /// sharp batch. The fill ends at a vertical cut (`revmeter::cut_x`).
    pub fn rev_meter(&mut self, x: f32, y: f32, frac: f32, tune: &RevMeterTune) {
        self.rev_meter_sized(x, y, revmeter::WIDTH, frac, tune, RevRed::default());
    }

    /// `rev_meter` for a meter `width` canvas px wide, with the redline look `red` (the Race
    /// line HUD, #323: longer than the original's 127 px and pulsing at the redline).
    pub fn rev_meter_sized(&mut self, x: f32, y: f32, width: f32, frac: f32, tune: &RevMeterTune, red: RevRed) {
        if self.sdf.is_none() {
            return;
        }
        let k = revmeter::scale_for(width);
        let [bx0, bz0, bx1, bz1] = revmeter::BOUNDS;
        // a thicker tube than the fit needs more room
        let m = revmeter::halo_room() + (tune.thickness as f32 / 100.0 - 1.0).max(0.0);
        let (u0, v0, u1, v1) = (bx0 - m, bz1 + m, bx1 + m, bz0 - m);
        let (x0, y0) = (x - m * k, y - m * k);
        let (x1, y1) = (x + (bx1 - bx0 + m) * k, y + (bz1 - bz0 + m) * k);
        let pc = |v: u32| v as f32 / 100.0;
        let color = [pc(tune.glow), pc(tune.inner_glow), pc(tune.glass_alpha), pc(tune.spec)];
        let clip = [pc(tune.bloom), pc(tune.thickness), pc(tune.depth), pc(tune.roundness)];
        let aux = [revmeter::cut_x(frac), red.dim, red.rim, -1.0];
        let v = |px: f32, py: f32, u: f32, vv: f32| UiVertex { pos: [px, py], uv: [u, vv], color, clip, aux };
        let (p0, p1, p2, p3) = (v(x0, y0, u0, v0), v(x1, y0, u1, v0), v(x1, y1, u1, v1), v(x0, y1, u0, v1));
        self.verts.extend([p0, p1, p2, p0, p2, p3]);
    }

    /// Clip following quads to an ellipse (centre, radii) until `clip_none`.
    pub fn clip_ellipse(&mut self, cx: f32, cy: f32, rx: f32, ry: f32) {
        self.clip = [cx, cy, rx, ry];
    }
    /// Draw only outside an ellipse (a window onto a 3D view behind this layer).
    pub fn clip_outside(&mut self, cx: f32, cy: f32, rx: f32, ry: f32) {
        self.clip = [cx, cy, -rx, -ry];
    }
    pub fn clip_none(&mut self) {
        self.clip = [0.0; 4];
    }

    #[allow(clippy::too_many_arguments)]
    fn quad(&mut self, x: f32, y: f32, w: f32, h: f32, r: Rect, uv_frac: [f32; 4], color: [f32; 4]) {
        let a = ATLAS as f32;
        let (u0, v0) = ((r.x as f32 + uv_frac[0] * r.w as f32) / a, (r.y as f32 + uv_frac[1] * r.h as f32) / a);
        let (u1, v1) = ((r.x as f32 + uv_frac[2] * r.w as f32) / a, (r.y as f32 + uv_frac[3] * r.h as f32) / a);
        let clip = self.clip;
        let v = |px: f32, py: f32, u: f32, vv: f32| UiVertex { pos: [px, py], uv: [u, vv], color, clip, aux: [0.0; 4] };
        let (p0, p1, p2, p3) = (v(x, y, u0, v0), v(x + w, y, u1, v0), v(x + w, y + h, u1, v1), v(x, y + h, u0, v1));
        self.verts.extend([p0, p1, p2, p0, p2, p3]);
    }

    pub fn size(&self, name: &str) -> (f32, f32) {
        self.assets.atlas.get(name).map_or((0.0, 0.0), |r| (r.w as f32, r.h as f32))
    }

    /// Draw an atlas image at its own size; returns its width (0 if missing).
    pub fn image(&mut self, name: &str, x: f32, y: f32) -> f32 {
        self.image_tint(name, x, y, WHITE)
    }

    pub fn image_tint(&mut self, name: &str, x: f32, y: f32, color: [f32; 4]) -> f32 {
        let Some(r) = self.assets.atlas.get(name) else { return 0.0 };
        if let Some(t) = self.sdf.and_then(|s| s.tile(name)) {
            self.field_quad(t, x, y, r.w as f32, r.h as f32, color); // REMASTER
        } else {
            self.quad(x, y, r.w as f32, r.h as f32, r, [0.0, 0.0, 1.0, 1.0], color);
        }
        r.w as f32
    }

    pub fn image_scaled(&mut self, name: &str, x: f32, y: f32, w: f32, h: f32, color: [f32; 4]) {
        if let Some(r) = self.assets.atlas.get(name) {
            self.quad(x, y, w, h, r, [0.0, 0.0, 1.0, 1.0], color);
        }
    }

    /// REMASTER: an image drawn smaller than its texels, box-filtered (`aux.w = -2`: `fs_ui_remaster`
    /// takes 4 x 4 taps over the pixel's footprint, the atlas has no mips), tinted by `color` (the countdown art).
    pub fn image_filtered(&mut self, name: &str, x: f32, y: f32, w: f32, h: f32, color: [f32; 4]) {
        let Some(r) = self.assets.atlas.get(name) else { return };
        let start = self.verts.len();
        self.quad(x, y, w, h, r, [0.0, 0.0, 1.0, 1.0], color);
        let a = ATLAS as f32;
        let uv = [(r.x as f32 + 0.5) / a, (r.y as f32 + 0.5) / a, (r.x + r.w) as f32 / a - 0.5 / a, (r.y + r.h) as f32 / a - 0.5 / a];
        for v in &mut self.verts[start..] {
            v.aux = [0.0, 0.0, 0.0, -2.0];
            v.clip = uv;
        }
    }

    /// REMASTER proximity arrow (`aux.w = -5`, 09-remaster 9.21 phase 7): one slice of a
    /// `ui_font` glyph extruded in 3D. `at(x, y)` takes a point of the glyph's plane (field
    /// source px from the bbox centre, y down) to the canvas, None behind the eye; the tile
    /// is drawn as an `n` × `n` grid so the projection's perspective holds. `ramp` = place on
    /// the rev meter's colour ramp, `layer` 0 (bottom) .. 1 (top: the bevelled glass face),
    /// `view` = the direction to the eye in the glyph's frame (x, y as above, z up). Draws
    /// only in a sharp batch.
    #[allow(clippy::too_many_arguments)]
    pub fn glass_glyph(&mut self, font: HudFont, c: char, at: &dyn Fn(f32, f32) -> Option<[f32; 2]>, n: usize, ramp: f32, alpha: f32, layer: f32, view: [f32; 3]) {
        self.glass_glyph_tinted(font, c, at, n, ramp, alpha, layer, view, false);
    }

    /// `glass_glyph`; `blue` swaps the rev meter's colour ramp for electric blue (menu labels).
    #[allow(clippy::too_many_arguments)]
    pub fn glass_glyph_tinted(&mut self, font: HudFont, c: char, at: &dyn Fn(f32, f32) -> Option<[f32; 2]>, n: usize, ramp: f32, alpha: f32, layer: f32, view: [f32; 3], blue: bool) {
        let Some(sdf) = self.sdf else { return };
        let (Some(t), Some([x0, y0, x1, y1])) = (sdf.font_glyph(font, c), ui_font::face(font).glyph(c).and_then(|g| g.bbox)) else { return };
        // the tile = bbox + BORDER source px each side, TEXELS texels per source px
        let b = BORDER as f32;
        let (cx, cy) = ((x0 + x1) as f32 * 0.5, (y0 + y1) as f32 * 0.5);
        let (gx0, gy0) = (x0 as f32 - b - cx, y0 as f32 - b - cy);
        let (gw, gh) = ((x1 - x0) as f32 + 2.0 * b, (y1 - y0) as f32 + 2.0 * b);
        let (aw, ah) = (sdf.width as f32, sdf.height as f32);
        let color = [ramp, alpha, layer, if blue { 1.0 } else { 0.0 }];
        let clip = [view[0], view[1], view[2], 0.0];
        let mut grid = Vec::with_capacity((n + 1) * (n + 1));
        for j in 0..=n {
            for i in 0..=n {
                let (u, v) = (i as f32 / n as f32, j as f32 / n as f32);
                let Some(pos) = at(gx0 + u * gw, gy0 + v * gh) else { return };
                let uv = [(t.rect.x as f32 + u * t.rect.w as f32) / aw, (t.rect.y as f32 + v * t.rect.h as f32) / ah];
                grid.push(UiVertex { pos, uv, color, clip, aux: [0.0, 0.0, 0.0, -5.0] });
            }
        }
        for j in 0..n {
            for i in 0..n {
                let k = j * (n + 1) + i;
                let (p0, p1, p2, p3) = (grid[k], grid[k + 1], grid[k + n + 2], grid[k + n + 1]);
                self.verts.extend([p0, p1, p2, p0, p2, p3]);
            }
        }
    }

    /// REMASTER: `s` in the rev meter's glass, electric blue (`glass_glyph_tinted`): flat on the
    /// canvas at (x, y .. y + size), `SLICES` layers stacked `depth` canvas px up for the
    /// extrusion. Capitals of the face only (no fallback); returns the advance. Sharp batch only.
    #[allow(clippy::too_many_arguments)]
    pub fn glass_text(&mut self, font: HudFont, s: &str, x: f32, y: f32, size: f32, tracking: f32, alpha: f32, depth: f32) -> f32 {
        let alpha = alpha * self.text_alpha;
        const SLICES: usize = 3;
        let face = ui_font::face(font);
        let k = size / face.cap;
        let base = y + size;
        let mut cx = x;
        let mut prev = None;
        for c in s.chars() {
            if let Some(p) = prev {
                cx += face.kerning(p, c) * k;
            }
            prev = Some(c);
            let Some(g) = face.glyph(c) else {
                cx += face.glyph(' ').map_or(4.0, |g| g.advance) * k;
                continue;
            };
            if let Some([x0, y0, x1, y1]) = g.bbox {
                let gc = [cx + (x0 + x1) as f32 * 0.5 * k, base + (y0 + y1) as f32 * 0.5 * k];
                for l in 0..SLICES {
                    let layer = l as f32 / (SLICES - 1) as f32;
                    let up = depth * (layer - 1.0);
                    let at = |gx: f32, gy: f32| Some([gc[0] + gx * k, gc[1] + gy * k + up]);
                    self.glass_glyph_tinted(font, c, &at, 1, 0.0, alpha, layer, [-0.35, -0.3, 0.85], true);
                }
            }
            cx += (g.advance + tracking) * k;
        }
        cx - x
    }

    /// A sub-rectangle (source pixels) of an image, drawn at its own size.
    #[allow(clippy::too_many_arguments)]
    pub fn image_sub(&mut self, name: &str, sx: f32, sy: f32, sw: f32, sh: f32, x: f32, y: f32) {
        if let Some(r) = self.assets.atlas.get(name) {
            let (w, h) = (r.w as f32, r.h as f32);
            self.quad(x, y, sw, sh, r, [sx / w, sy / h, (sx + sw) / w, (sy + sh) / h], WHITE);
        }
    }

    /// Filled ring sector from angle a0 to a1 (radians, clockwise on screen from +x).
    #[allow(clippy::too_many_arguments)]
    pub fn wedge(&mut self, cx: f32, cy: f32, r0: f32, r1: f32, a0: f32, a1: f32, color: [f32; 4]) {
        let n = (((a1 - a0).abs() * 12.0) as usize).max(1);
        for k in 0..n {
            let (t0, t1) = (a0 + (a1 - a0) * k as f32 / n as f32, a0 + (a1 - a0) * (k + 1) as f32 / n as f32);
            let p = |r: f32, t: f32| [cx + r * t.cos(), cy + r * t.sin()];
            self.triangle([p(r0, t0), p(r1, t0), p(r1, t1)], color);
            self.triangle([p(r0, t0), p(r1, t1), p(r0, t1)], color);
        }
    }

    /// Left `frac` of an image (gauge fills).
    pub fn image_left(&mut self, name: &str, x: f32, y: f32, frac: f32) {
        if let Some(r) = self.assets.atlas.get(name) {
            let f = frac.clamp(0.0, 1.0);
            self.quad(x, y, r.w as f32 * f, r.h as f32, r, [0.0, 0.0, f, 1.0], WHITE);
        }
    }

    /// A MENU.SPR sprite frame (frame clamped to what exists).
    pub fn menu(&mut self, sprite: &str, frame: usize, x: f32, y: f32) -> f32 {
        let n = self.assets.frames(sprite).max(1);
        self.image(&format!("m:{sprite}:{}", frame.min(n - 1)), x, y)
    }

    /// Four rectangles round the canvas, out to the window's edge, in `color`: bars that hide
    /// whatever a uniform-scale canvas leaves uncovered (a viewport clear colour, a 3D scene).
    pub fn bars(&mut self, size: (u32, u32), color: [f32; 4]) {
        let (sx, sy, ox, oy) = self.canvas.axes(size);
        let (l, t) = (ox / sx, oy / sy);
        let (w, h) = (self.canvas.w, self.canvas.h);
        let (fw, fh) = (size.0 as f32 / sx, size.1 as f32 / sy);
        if l > 0.0 {
            self.rect(-l, -t, l, fh, color);
            self.rect(w, -t, l, fh, color);
        }
        if t > 0.0 {
            self.rect(-l, -t, fw, t, color);
            self.rect(-l, h, fw, t, color);
        }
    }

    pub fn rect(&mut self, x: f32, y: f32, w: f32, h: f32, color: [f32; 4]) {
        if let Some(r) = self.assets.atlas.get("white") {
            self.quad(x, y, w, h, r, [0.25, 0.25, 0.75, 0.75], color);
        }
    }

    /// A rectangle fading vertically from `top` to `bottom` (per-vertex colours).
    pub fn rect_fade(&mut self, x: f32, y: f32, w: f32, h: f32, top: [f32; 4], bottom: [f32; 4]) {
        let Some(r) = self.assets.atlas.get("white") else { return };
        let a = ATLAS as f32;
        let uv = [(r.x as f32 + 2.0) / a, (r.y as f32 + 2.0) / a];
        let clip = self.clip;
        let v = |px: f32, py: f32, color: [f32; 4]| UiVertex { pos: [px, py], uv, color, clip, aux: [0.0; 4] };
        let (p0, p1, p2, p3) = (v(x, y, top), v(x + w, y, top), v(x + w, y + h, bottom), v(x, y + h, bottom));
        self.verts.extend([p0, p1, p2, p0, p2, p3]);
    }

    /// Thick line as a rotated quad.
    pub fn line(&mut self, x0: f32, y0: f32, x1: f32, y1: f32, width: f32, color: [f32; 4]) {
        let Some(r) = self.assets.atlas.get("white") else { return };
        let (dx, dy) = (x1 - x0, y1 - y0);
        let len = (dx * dx + dy * dy).sqrt().max(1e-3);
        let (nx, ny) = (-dy / len * width * 0.5, dx / len * width * 0.5);
        let a = ATLAS as f32;
        let (u, v) = ((r.x as f32 + 2.0) / a, (r.y as f32 + 2.0) / a);
        let clip = self.clip;
        let p = |px: f32, py: f32| UiVertex { pos: [px, py], uv: [u, v], color, clip, aux: [0.0; 4] };
        let (q0, q1, q2, q3) = (p(x0 + nx, y0 + ny), p(x1 + nx, y1 + ny), p(x1 - nx, y1 - ny), p(x0 - nx, y0 - ny));
        self.verts.extend([q0, q1, q2, q0, q2, q3]);
    }

    /// A filled triangle (dial needles).
    pub fn triangle(&mut self, p: [[f32; 2]; 3], color: [f32; 4]) {
        let Some(r) = self.assets.atlas.get("white") else { return };
        let a = ATLAS as f32;
        let uv = [(r.x as f32 + 2.0) / a, (r.y as f32 + 2.0) / a];
        let clip = self.clip;
        self.verts.extend(p.map(|pos| UiVertex { pos, uv, color, clip, aux: [0.0; 4] }));
    }

    /// REMASTER front end: lyon-tessellated vector triangles with a colour per vertex.
    /// (The sharp pipeline's dithered mesh branch in ui.wgsl returns before the ellipse clip:
    /// cut a mesh to a disc geometrically, `clip_ellipse` only cuts quads and text.)
    pub fn mesh(&mut self, m: &ui_vector::Mesh) {
        let Some(r) = self.assets.atlas.get("white") else { return };
        let a = ATLAS as f32;
        let uv = [(r.x as f32 + 2.0) / a, (r.y as f32 + 2.0) / a];
        let clip = self.clip;
        // the sharp pipeline dithers vector meshes (ui.wgsl, aux.w = -6)
        let aux = if self.sdf.is_some() { [0.0, 0.0, 0.0, -6.0] } else { [0.0; 4] };
        for &i in &m.idx {
            let v = m.verts[i as usize];
            self.verts.push(UiVertex { pos: v.pos, uv, color: v.color, clip, aux });
        }
        for &i in &m.glow_idx {
            let v = m.glow_verts[i as usize];
            self.glow.push(UiVertex { pos: v.pos, uv, color: v.color, clip, aux });
        }
    }

    /// Text in the myfont grid font, `scale` x 12 px tall; returns the advance.
    pub fn text(&mut self, s: &str, x: f32, y: f32, scale: f32, color: [f32; 4]) -> f32 {
        let Some(font) = &self.assets.font else { return 0.0 };
        let Some(r) = self.assets.atlas.get(&font.image) else { return 0.0 };
        let cell = font.cell_h as f32;
        let mut cx = x;
        for c in s.chars() {
            let (gx, gy, gw) = font.glyph(c);
            if let Some(sdf) = self.sdf {
                // REMASTER: chars without a field (space) are empty
                if let Some(t) = sdf.glyph(c) {
                    self.field_quad(t, cx, y, gw as f32 * scale, cell * scale, color);
                }
            } else {
                let frac = [gx as f32 / r.w as f32, gy as f32 / r.h as f32, (gx + gw) as f32 / r.w as f32, (gy as f32 + cell) / r.h as f32];
                self.quad(cx, y, gw as f32 * scale, cell * scale, r, frac, color);
            }
            cx += (gw as f32 + 1.0) * scale;
        }
        cx - x
    }

    /// `text` with everything above `min_y` cut away (a glyph row is cropped, not dropped): CLASSIC Tron scroller, whose
    /// text the original's depth test hides outside its band (#320). Grid font only; a sharp batch draws it whole.
    pub fn text_from(&mut self, s: &str, x: f32, y: f32, scale: f32, color: [f32; 4], min_y: f32) -> f32 {
        let cell = self.assets.font.as_ref().map_or(0.0, |f| f.cell_h as f32) * scale;
        if self.sdf.is_some() || y >= min_y {
            return self.text(s, x, y, scale, color);
        }
        let Some(font) = &self.assets.font else { return 0.0 };
        let Some(r) = self.assets.atlas.get(&font.image) else { return 0.0 };
        let cut = ((min_y - y) / cell).clamp(0.0, 1.0);
        let mut cx = x;
        for c in s.chars() {
            let (gx, gy, gw) = font.glyph(c);
            if cut < 1.0 {
                let top = gy as f32 + font.cell_h as f32 * cut;
                let frac = [gx as f32 / r.w as f32, top / r.h as f32, (gx + gw) as f32 / r.w as f32, (gy as f32 + font.cell_h as f32) / r.h as f32];
                self.quad(cx, y + cell * cut, gw as f32 * scale, cell * (1.0 - cut), r, frac, color);
            }
            cx += (gw as f32 + 1.0) * scale;
        }
        cx - x
    }

    pub fn text_width(&self, s: &str, scale: f32) -> f32 {
        self.assets.font.as_ref().map_or(0.0, |f| f.width(s) * scale)
    }

    /// Text in the original menu font (FONT1.FNT), `scale` canvas px per font px, `y` = the top of
    /// the glyph box (`FntFont::height` tall); returns the advance.
    pub fn fnt_text(&mut self, s: &str, x: f32, y: f32, scale: f32, color: [f32; 4]) -> f32 {
        let Some(font) = &self.assets.fnt else { return 0.0 };
        let Some(r) = self.assets.atlas.get(&font.image) else { return 0.0 };
        let cell = font.height as f32;
        let mut cx = x;
        for c in s.chars() {
            let (gx, gy, gw, adv) = font.glyph(c);
            if gw > 0 {
                if let Some(sdf) = self.sdf {
                    // REMASTER: chars without a field (blank glyphs) are empty
                    if let Some(t) = sdf.fnt_glyph(c) {
                        self.field_quad(t, cx, y, gw as f32 * scale, cell * scale, color);
                    }
                } else {
                    let frac = [gx as f32 / r.w as f32, gy as f32 / r.h as f32, (gx + gw) as f32 / r.w as f32, (gy as f32 + cell) / r.h as f32];
                    self.quad(cx, y, gw as f32 * scale, cell * scale, r, frac, color);
                }
            }
            cx += adv as f32 * scale;
        }
        cx - x
    }

    pub fn fnt_width(&self, s: &str, scale: f32) -> f32 {
        self.assets.fnt.as_ref().map_or(0.0, |f| f.width(s) * scale)
    }

    /// REMASTER modern HUD (9.21): text in a TrueType HUD font, its capitals spanning
    /// `y .. y + style.size`; returns the advance. Draws only in a sharp batch (the fields
    /// live in its atlas); chars the font lacks come from its fallback, unknown ones advance
    /// by a space.
    pub fn font_text(&mut self, font: HudFont, s: &str, x: f32, y: f32, style: &FontStyle) -> f32 {
        let face = ui_font::face(font);
        let k = style.size / face.cap; // canvas px per field source px
        let base = y + style.size;
        let tan = style.skew.to_radians().tan();
        // outline width in field px, within the field's range (it grows outward)
        let o = (style.outline_w / k).clamp(0.0, RANGE - 0.5);
        let aux = [style.outline[0], style.outline[1], style.outline[2], 2.0 + o];
        let color = [style.color[0], style.color[1], style.color[2], style.color[3] * self.text_alpha];
        let mut cx = x;
        let mut prev = None;
        for c in s.chars() {
            if let Some(p) = prev {
                cx += face.kerning(p, c) * k;
            }
            prev = Some(c);
            let Some((f, g)) = ui_font::resolve(font, c) else {
                cx += face.glyph(' ').map_or(4.0, |g| g.advance) * k;
                continue;
            };
            // a fallback glyph is sized to this face's capitals
            let kg = k * face.cap / ui_font::face(f).cap;
            if let (Some([x0, y0, x1, y1]), Some(t)) = (g.bbox, self.sdf.and_then(|sd| sd.font_glyph(f, c))) {
                let rect = [cx + x0 as f32 * kg, base + y0 as f32 * kg, (x1 - x0) as f32 * kg, (y1 - y0) as f32 * kg];
                self.field_verts(t, rect, color, aux, (base, tan));
            }
            cx += (g.advance + style.tracking) * kg;
        }
        cx - x
    }

    /// How many vertices the batch holds (with `rotate_from`: what is drawn after this point).
    pub fn vert_len(&self) -> usize {
        self.verts.len()
    }

    /// Turn the vertices from `from` on (quads and text drawn since `vert_len`) by `angle` radians about `pivot` (canvas
    /// px), as CSS `rotate()` in this y-down space: -π/2 stands text up, reading upwards (a chart's vertical caption, #429).
    pub fn rotate_from(&mut self, from: usize, pivot: [f32; 2], angle: f32) {
        let (s, c) = angle.sin_cos();
        for v in self.verts.iter_mut().skip(from) {
            let (dx, dy) = (v.pos[0] - pivot[0], v.pos[1] - pivot[1]);
            v.pos = [pivot[0] + dx * c - dy * s, pivot[1] + dx * s + dy * c];
        }
    }

    /// REMASTER Glass HUD (#294): `font_text` with a vertical gradient over the capitals' box:
    /// `stops` = (fraction of `style.size` from the cap top, colour), ascending. Each glyph quad
    /// is cut into bands at the stops, the colour interpolated per vertex (`style.color` is the
    /// alpha multiplier). Returns the advance.
    pub fn font_text_graded(&mut self, font: HudFont, s: &str, x: f32, y: f32, style: &FontStyle, stops: &[(f32, [f32; 4])]) -> f32 {
        let face = ui_font::face(font);
        let k = style.size / face.cap;
        let base = y + style.size;
        let tan = style.skew.to_radians().tan();
        let o = (style.outline_w / k).clamp(0.0, RANGE - 0.5);
        let aux = [style.outline[0], style.outline[1], style.outline[2], 2.0 + o];
        let alpha = style.color[3] * self.text_alpha;
        // the colour at canvas y: linear between the stops, held outside them
        let at = |py: f32| -> [f32; 4] {
            let t = (py - y) / style.size;
            let (mut lo, mut hi) = (stops[0], stops[stops.len() - 1]);
            for w in stops.windows(2) {
                if t >= w[0].0 && t <= w[1].0 {
                    (lo, hi) = (w[0], w[1]);
                }
            }
            let u = if hi.0 > lo.0 { ((t - lo.0) / (hi.0 - lo.0)).clamp(0.0, 1.0) } else { 0.0 };
            let mut c: [f32; 4] = std::array::from_fn(|i| lo.1[i] + (hi.1[i] - lo.1[i]) * u);
            c[3] *= alpha;
            c
        };
        let cuts: Vec<f32> = stops.iter().map(|st| y + st.0 * style.size).collect();
        let mut cx = x;
        let mut prev = None;
        for c in s.chars() {
            if let Some(p) = prev {
                cx += face.kerning(p, c) * k;
            }
            prev = Some(c);
            let Some((f, g)) = ui_font::resolve(font, c) else {
                cx += face.glyph(' ').map_or(4.0, |g| g.advance) * k;
                continue;
            };
            let kg = k * face.cap / ui_font::face(f).cap;
            if let (Some([x0, y0, x1, y1]), Some(t)) = (g.bbox, self.sdf.and_then(|sd| sd.font_glyph(f, c))) {
                let rect = [cx + x0 as f32 * kg, base + y0 as f32 * kg, (x1 - x0) as f32 * kg, (y1 - y0) as f32 * kg];
                self.field_bands(t, rect, &at, &cuts, aux, (base, tan));
            }
            cx += (g.advance + style.tracking) * kg;
        }
        cx - x
    }

    /// `field_verts` cut into horizontal bands at the canvas rows `cuts` (those inside the quad),
    /// each vertex coloured by `at(y)`.
    fn field_bands(&mut self, t: &Tile, rect: [f32; 4], at: &dyn Fn(f32) -> [f32; 4], cuts: &[f32], aux: [f32; 4], shear: (f32, f32)) {
        let Some(sdf) = self.sdf else { return };
        let [x, y, w, h] = rect;
        let (kx, ky) = (w / t.size.0 as f32, h / t.size.1 as f32);
        let b = BORDER as f32;
        let (x0, y0, x1, y1) = (x - b * kx, y - b * ky, x + w + b * kx, y + h + b * ky);
        let (aw, ah) = (sdf.width as f32, sdf.height as f32);
        let (u0, v0, u1, v1) = (t.rect.x as f32 / aw, t.rect.y as f32 / ah, (t.rect.x + t.rect.w) as f32 / aw, (t.rect.y + t.rect.h) as f32 / ah);
        let clip = self.clip;
        let mut rows: Vec<f32> = vec![y0];
        rows.extend(cuts.iter().copied().filter(|&c| c > y0 && c < y1));
        rows.push(y1);
        let v = |px: f32, py: f32, u: f32, vv: f32| UiVertex { pos: [px + (shear.0 - py) * shear.1, py], uv: [u, vv], color: at(py), clip, aux };
        for r in rows.windows(2) {
            let (ya, yb) = (r[0], r[1]);
            let (va, vb) = (v0 + (v1 - v0) * (ya - y0) / (y1 - y0), v0 + (v1 - v0) * (yb - y0) / (y1 - y0));
            let (p0, p1, p2, p3) = (v(x0, ya, u0, va), v(x1, ya, u1, va), v(x1, yb, u1, vb), v(x0, yb, u0, vb));
            self.verts.extend([p0, p1, p2, p0, p2, p3]);
        }
    }

    pub fn font_width(&self, font: HudFont, s: &str, style: &FontStyle) -> f32 {
        text_width(font, s, style)
    }

    /// Text in MENU.SPR's letter sprites (a-z, 0-9, punctuation); returns the advance.
    pub fn menu_text(&mut self, s: &str, x: f32, y: f32, color: [f32; 4]) -> f32 {
        let mut cx = x;
        for c in s.chars().map(|c| c.to_ascii_lowercase()) {
            let name = match c {
                ' ' => {
                    cx += 6.0;
                    continue;
                }
                '.' => "punkt".to_string(),
                ':' => "kolon".to_string(),
                '-' => "minus".to_string(),
                '+' => "plus".to_string(),
                '?' => "quest".to_string(),
                '/' => "slash".to_string(),
                _ => c.to_string(),
            };
            let w = self.image_tint(&format!("m:{name}:0"), cx, y, color);
            cx += if w > 0.0 { w + 1.0 } else { 8.0 };
        }
        cx - x
    }

    /// HUD digits from the D3D set (`bnum`/`snum` + digit + "00"), with ':' and '/'.
    /// HUD glyph name for a character of a number.
    fn hud_glyph(c: char, big: bool) -> Option<String> {
        Some(match (c, big) {
            (d @ '0'..='9', true) => format!("h:bnum{d}00"),
            (d @ '0'..='9', false) => format!("h:snum{d}00"),
            (':', _) => "h:kolon00".into(),
            ('/', _) => "h:tof00".into(),
            ('-', _) => "h:minus00".into(),
            ('+', _) => "h:plus00".into(),
            ('R', _) => "h:snumr00".into(),
            _ => return None,
        })
    }

    /// Advance of one HUD character: digits share one cell (the widest digit, '0'), so
    /// changing numbers don't shift sideways; separators keep their own width.
    fn hud_advance(&self, c: char, big: bool) -> f32 {
        match Self::hud_glyph(c, big) {
            Some(_) if c.is_ascii_digit() || c == 'R' => self.size(&Self::hud_glyph('0', big).unwrap_or_default()).0 + 1.0,
            Some(name) => self.size(&name).0 + 1.0,
            None => 8.0,
        }
    }

    /// HUD digits from the D3D set (`bnum`/`snum` + digit + "00"), with ':' and '/', in
    /// fixed-width digit cells.
    pub fn hud_number(&mut self, s: &str, x: f32, y: f32, big: bool) -> f32 {
        let mut cx = x;
        for c in s.chars() {
            let adv = self.hud_advance(c, big);
            if let Some(name) = Self::hud_glyph(c, big) {
                let w = self.size(&name).0;
                self.image(&name, cx + ((adv - 1.0 - w) * 0.5).round(), y);
            }
            cx += adv;
        }
        cx - x
    }

    pub fn hud_number_width(&self, s: &str, big: bool) -> f32 {
        s.chars().map(|c| self.hud_advance(c, big)).sum()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Black, mid grey and white map to the duotone's three stops; the corners are cut, the middle is opaque.
    #[test]
    fn duotone_stops_and_corners() {
        let px = |v: u8| [v, v, v, 255];
        let img: Vec<u8> = [0u8, 128, 255].iter().flat_map(|&v| (0..30 * 30).flat_map(move |_| px(v))).collect();
        let (dark, mid, light) = (duotone(&img[..3600], 30, 30), duotone(&img[3600..7200], 30, 30), duotone(&img[7200..], 30, 30));
        let at = |v: &[u8], x: usize, y: usize| -> [u8; 4] { v[(y * 30 + x) * 4..][..4].try_into().unwrap() };
        assert_eq!(at(&dark, 15, 15), [5, 10, 23, 255]);
        assert_eq!(at(&light, 15, 15), [168, 219, 242, 255]);
        let m = at(&mid, 15, 15);
        assert!((m[0] as i32 - 77).abs() <= 1 && (m[2] as i32 - 143).abs() <= 1 && m[3] == 255);
        assert_eq!(at(&dark, 0, 0)[3], 0);
        assert_eq!(at(&dark, 29, 29)[3], 0);
        assert_eq!(at(&dark, 15, 0)[3], 255);
    }

    #[test]
    fn full_shelf_falls_back_to_a_hole() {
        let mut a = Atlas::default();
        let tall = a.reserve("tall", 1000, 4000).unwrap();
        let strip = a.reserve("strip", 3000, 50).unwrap();
        // no shelf row is left for this one, but there is room under the strip
        let big = a.reserve("big", 2000, 3000).unwrap();
        assert_eq!((big.x, big.y), (tall.x + tall.w + PAD, strip.y + strip.h + PAD));
        assert!(a.reserve("huge", 4000, 4000).is_none());
    }

    #[test]
    fn shelf_layout_is_unchanged() {
        let mut a = Atlas::default();
        let one = a.reserve("one", 640, 480).unwrap();
        let two = a.reserve("two", 3500, 100).unwrap();
        assert_eq!((one.x, one.y), (5, 0));
        assert_eq!((two.x, two.y), (0, 481));
    }
}
