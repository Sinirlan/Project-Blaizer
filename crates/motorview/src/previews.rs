//! REMASTER track photos for the modern Track screen (14-menu-overhaul #10; design choice).
//! `<install>/remaster/ui/previews/trackNN.png` is a 6:5 picture of the track made by the user in
//! motorview (never committed); the modern Track screen shows it in place of the original's
//! 324 x 270 `_menuimg` (`trk:N`) when it exists.

use crate::png;
use motor_data::fs;
use std::path::Path;

/// Where the game looks for the photos, relative to the install.
pub const DIR: &str = "remaster/ui/previews";
/// The photos' aspect (the original's 324 x 270).
pub const ASPECT: (u32, u32) = (6, 5);
/// Largest stored size; bigger photos are box-filtered down so the atlas slot stays bounded.
pub const MAX: (u32, u32) = (1020, 850);

/// Track `track`'s photo (w, h, RGBA), centre-cropped to 6:5 and capped at `MAX`.
pub fn load(root: &Path, track: u32) -> Option<(u32, u32, Vec<u8>)> {
    let name = format!("{DIR}/track{track:02}.png");
    let (w, h, rgba) = png::read(&fs::read(root, &name).ok()?).map_err(|e| eprintln!("{name}: {e}")).ok()?;
    let (w, h, rgba) = crop(w, h, &rgba);
    Some(shrink(w, h, rgba))
}

/// Centre-crop to `ASPECT`.
pub fn crop(w: u32, h: u32, rgba: &[u8]) -> (u32, u32, Vec<u8>) {
    let (cw, ch) = if w * ASPECT.1 > h * ASPECT.0 { (h * ASPECT.0 / ASPECT.1, h) } else { (w, w * ASPECT.1 / ASPECT.0) };
    if (cw, ch) == (w, h) {
        return (w, h, rgba.to_vec());
    }
    let (x0, y0) = ((w - cw) / 2, (h - ch) / 2);
    let mut out = Vec::with_capacity((cw * ch * 4) as usize);
    for y in 0..ch {
        out.extend_from_slice(&rgba[(((y0 + y) * w + x0) * 4) as usize..][..(cw * 4) as usize]);
    }
    (cw, ch, out)
}

/// Box-filter down by the smallest whole factor that fits `MAX`.
pub fn shrink(w: u32, h: u32, rgba: Vec<u8>) -> (u32, u32, Vec<u8>) {
    let k = w.div_ceil(MAX.0).max(h.div_ceil(MAX.1)).max(1);
    if k == 1 {
        return (w, h, rgba);
    }
    let (nw, nh) = (w / k, h / k);
    let mut out = vec![0u8; (nw * nh * 4) as usize];
    for y in 0..nh {
        for x in 0..nw {
            let mut s = [0u32; 4];
            for (dy, dx) in (0..k).flat_map(|dy| (0..k).map(move |dx| (dy, dx))) {
                let i = (((y * k + dy) * w + x * k + dx) * 4) as usize;
                for c in 0..4 {
                    s[c] += rgba[i + c] as u32;
                }
            }
            for c in 0..4 {
                out[((y * nw + x) * 4) as usize + c] = (s[c] / (k * k)) as u8;
            }
        }
    }
    (nw, nh, out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crops_to_aspect_and_caps() {
        let (w, h, _) = crop(1920, 1080, &vec![0; 1920 * 1080 * 4]);
        assert_eq!((w, h), (1296, 1080));
        let (w, h, _) = shrink(w, h, vec![0; (w * h * 4) as usize]);
        assert_eq!((w, h), (648, 540));
        let (w, h, _) = crop(1000, 1000, &vec![0; 1000 * 1000 * 4]);
        assert_eq!((w, h), (1000, 833));
    }
}
