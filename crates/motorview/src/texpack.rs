//! REMASTER texture pack (Gitea #211): upscaled colour, normal and roughness maps that replace
//! the originals' art, read from the user's install and falling back to the original per texture.
//!
//! Files, per IFF base name (`triroad.iff` -> `triroad`), first in `remaster/textures/trackNN/`,
//! then in `remaster/textures/`, matched case-insensitively:
//! `NAME.png` colour, `NAME_n.png` tangent-space normal (OpenGL, green up), `NAME_r.png`
//! roughness (grey, 1 = rough) or `NAME_nr.png` = normal rgb + roughness in alpha (the layout
//! of the carbon / tyre assets). Every part is optional. The contract: 09-remaster 9.37.

use std::path::Path;

/// One decoded map, RGBA8.
pub struct Map {
    pub w: u32,
    pub h: u32,
    pub rgba: Vec<u8>,
}

/// The files found for one texture.
#[derive(Default)]
pub struct Material {
    pub colour: Option<Map>,
    /// normal in rgb; the alpha is the roughness (255 = fully rough when there is no map)
    pub normal: Option<Map>,
    pub has_rough: bool,
}

impl Material {
    pub fn is_empty(&self) -> bool {
        self.colour.is_none() && self.normal.is_none()
    }
}

/// `TRIROAD.IFF` / `Data\x\triroad.iff` -> `triroad`
fn stem(name: &str) -> String {
    let base = name.rsplit(['\\', '/']).next().unwrap_or(name).to_ascii_lowercase();
    base.strip_suffix(".iff").map(String::from).unwrap_or(base)
}

/// Any 8/16-bit PNG (grey, grey+alpha, RGB, RGBA, palette) as RGBA8.
pub fn decode_png(bytes: &[u8]) -> Option<Map> {
    let mut dec = ::png::Decoder::new(std::io::Cursor::new(bytes));
    dec.set_transformations(::png::Transformations::EXPAND | ::png::Transformations::STRIP_16);
    let mut r = dec.read_info().ok()?;
    let mut buf = vec![0; r.output_buffer_size()];
    let info = r.next_frame(&mut buf).ok()?;
    if info.bit_depth != ::png::BitDepth::Eight || info.width == 0 || info.height == 0 {
        return None;
    }
    let src = &buf[..info.buffer_size()];
    let rgba: Vec<u8> = match info.color_type {
        ::png::ColorType::Rgba => src.to_vec(),
        ::png::ColorType::Rgb => src.as_chunks::<3>().0.iter().flat_map(|p| [p[0], p[1], p[2], 255]).collect(),
        ::png::ColorType::Grayscale => src.iter().flat_map(|&g| [g, g, g, 255]).collect(),
        ::png::ColorType::GrayscaleAlpha => src.as_chunks::<2>().0.iter().flat_map(|p| [p[0], p[0], p[0], p[1]]).collect(),
        ::png::ColorType::Indexed => return None,
    };
    Some(Map { w: info.width, h: info.height, rgba })
}

fn read_map(root: &Path, dirs: &[String], file: &str) -> Option<Map> {
    dirs.iter().find_map(|d| motor_data::fs::read(root, &format!("{d}/{file}")).ok()).and_then(|b| decode_png(&b))
}

/// The pack's files of texture `name` on `track`; `notes` collects a line per file that exists
/// but cannot be decoded (the original is used for that part).
pub fn find(root: &Path, track: u32, name: &str, notes: &mut Vec<String>) -> Material {
    let s = stem(name);
    let dirs = [format!("remaster/textures/track{track:02}"), "remaster/textures".to_string()];
    let mut get = |suffix: &str| {
        let file = format!("{s}{suffix}.png");
        let m = read_map(root, &dirs, &file);
        if m.is_none() && dirs.iter().any(|d| motor_data::fs::find_ci(root, &format!("{d}/{file}")).is_some()) {
            notes.push(format!("{file} unreadable"));
        }
        m
    };
    let colour = get("");
    let n = get("_n");
    let r = get("_r");
    let nr = get("_nr");
    let mut m = Material { colour, ..Default::default() };
    // a separate normal map wins over a packed one
    match (n, nr) {
        (Some(mut n), _) => {
            n.rgba.as_chunks_mut::<4>().0.iter_mut().for_each(|p| p[3] = 255);
            m.normal = Some(n);
        }
        (None, Some(packed)) => {
            m.normal = Some(packed);
            m.has_rough = true;
        }
        (None, None) => {}
    }
    if let Some(r) = r {
        // the roughness map (its red channel) rides in the normal's alpha; without a normal map
        // it carries a flat normal
        let n = m.normal.get_or_insert_with(|| Map { w: r.w, h: r.h, rgba: [128, 128, 255, 255].repeat((r.w * r.h) as usize) });
        let r = if (r.w, r.h) == (n.w, n.h) { r } else { resize(&r, n.w, n.h) };
        n.rgba.as_chunks_mut::<4>().0.iter_mut().zip(r.rgba.as_chunks::<4>().0.iter()).for_each(|(d, s)| d[3] = s[0]);
        m.has_rough = true;
    }
    m
}

/// What a scene keeps of the pack: square layers (RGBA8, `size`²) for the GPU's second array and,
/// per base texture layer, the ids of its colour and normal+roughness layers (-1 = the original).
#[derive(Clone, Default)]
pub struct Pack {
    pub size: u32,
    pub layers: Vec<Vec<u8>>,
    pub map: Vec<[i32; 2]>,
    /// textures with a colour, a normal and a roughness map
    pub counts: [usize; 3],
    pub notes: Vec<String>,
}

impl Pack {
    pub fn stats(&self) -> String {
        if self.layers.is_empty() && self.notes.is_empty() {
            return String::new();
        }
        let mut s = format!("; texture pack {}px: {} colour, {} normal, {} roughness", self.size, self.counts[0], self.counts[1], self.counts[2]);
        if !self.notes.is_empty() {
            s.push_str(&format!(" ({})", self.notes.join(", ")));
        }
        s
    }
}

/// Area-average halving while the source is at least twice the target and even.
fn halve(m: &Map) -> Map {
    let (w, h) = ((m.w / 2) as usize, (m.h / 2) as usize);
    let mut out = vec![0u8; w * h * 4];
    for y in 0..h {
        for x in 0..w {
            for c in 0..4 {
                let at = |dx: usize, dy: usize| m.rgba[((2 * y + dy) * m.w as usize + 2 * x + dx) * 4 + c] as u32;
                out[(y * w + x) * 4 + c] = ((at(0, 0) + at(1, 0) + at(0, 1) + at(1, 1) + 2) / 4) as u8;
            }
        }
    }
    Map { w: w as u32, h: h as u32, rgba: out }
}

/// Resample to `w` x `h`: box halving while it still reduces, then bilinear with the texture's
/// repeat (the road tiles).
pub fn resize(m: &Map, w: u32, h: u32) -> Map {
    if (m.w, m.h) == (w, h) {
        return Map { w, h, rgba: m.rgba.clone() };
    }
    let mut cur = Map { w: m.w, h: m.h, rgba: m.rgba.clone() };
    while cur.w >= 2 * w && cur.h >= 2 * h && cur.w.is_multiple_of(2) && cur.h.is_multiple_of(2) {
        cur = halve(&cur);
    }
    let (sw, sh) = (cur.w as i64, cur.h as i64);
    let mut out = vec![0u8; (w * h * 4) as usize];
    for y in 0..h as i64 {
        let fy = (y as f32 + 0.5) * sh as f32 / h as f32 - 0.5;
        let (y0, ty) = (fy.floor(), fy - fy.floor());
        for x in 0..w as i64 {
            let fx = (x as f32 + 0.5) * sw as f32 / w as f32 - 0.5;
            let (x0, tx) = (fx.floor(), fx - fx.floor());
            let tap = |dx: i64, dy: i64, c: usize| cur.rgba[((((y0 as i64 + dy).rem_euclid(sh)) * sw + (x0 as i64 + dx).rem_euclid(sw)) * 4) as usize + c] as f32;
            for c in 0..4 {
                let top = tap(0, 0, c) * (1.0 - tx) + tap(1, 0, c) * tx;
                let bot = tap(0, 1, c) * (1.0 - tx) + tap(1, 1, c) * tx;
                out[((y * w as i64 + x) * 4) as usize + c] = (top * (1.0 - ty) + bot * ty + 0.5) as u8;
            }
        }
    }
    Map { w, h, rgba: out }
}

/// The edge of the square layers for the pack's largest map: a power of two in 256..=1024,
/// halved while `layers` of them would pass `BUDGET`.
pub fn layer_size(largest: u32, layers: usize) -> u32 {
    const BUDGET: usize = 768 << 20;
    let mut s = largest.max(1).next_power_of_two().clamp(256, 1024) as usize;
    // x 4/3 for the mip chain
    while s > 256 && layers * s * s * 4 / 3 * 4 > BUDGET {
        s /= 2;
    }
    s as u32
}

#[cfg(test)]
mod tests {
    use super::*;

    fn png_bytes(w: u32, h: u32, color: ::png::ColorType, data: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        let mut e = ::png::Encoder::new(&mut out, w, h);
        e.set_color(color);
        e.set_depth(::png::BitDepth::Eight);
        e.write_header().unwrap().write_image_data(data).unwrap();
        out
    }

    #[test]
    fn stem_strips_path_and_extension() {
        assert_eq!(stem("Data\\TRIROAD.IFF"), "triroad");
        assert_eq!(stem("road"), "road");
    }

    #[test]
    fn decodes_grey_and_rgb() {
        let g = decode_png(&png_bytes(2, 1, ::png::ColorType::Grayscale, &[10, 200])).unwrap();
        assert_eq!(g.rgba, [10, 10, 10, 255, 200, 200, 200, 255]);
        let c = decode_png(&png_bytes(1, 1, ::png::ColorType::Rgb, &[1, 2, 3])).unwrap();
        assert_eq!(c.rgba, [1, 2, 3, 255]);
        assert!(decode_png(b"not a png").is_none());
    }

    #[test]
    fn resize_keeps_flat_colour_and_wraps() {
        let m = Map { w: 4, h: 4, rgba: [100, 120, 140, 255].repeat(16) };
        let up = resize(&m, 16, 16);
        assert!(up.rgba.as_chunks::<4>().0.iter().all(|p| p == &[100, 120, 140, 255]));
        let down = resize(&Map { w: 64, h: 64, rgba: [9, 9, 9, 255].repeat(64 * 64) }, 8, 8);
        assert!(down.rgba.as_chunks::<4>().0.iter().all(|p| p == &[9, 9, 9, 255]));
        // a stripe at the left edge blends with the right edge through the wrap
        let mut s = Map { w: 4, h: 1, rgba: [0, 0, 0, 255].repeat(4) };
        s.rgba[0] = 255;
        let r = resize(&s, 8, 1);
        assert!(r.rgba[7 * 4] > 0 && r.rgba[0] > r.rgba[7 * 4]);
    }

    #[test]
    fn layer_size_clamps() {
        assert_eq!(layer_size(100, 4), 256);
        assert_eq!(layer_size(600, 4), 1024);
        assert_eq!(layer_size(4096, 4), 1024);
        assert!(layer_size(1024, 400) < 1024);
    }
}
