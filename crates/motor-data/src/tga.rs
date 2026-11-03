//! Truevision TGA, as used for the sky backgrounds (`BackgroundName`, 640x196 8-bit
//! colour-mapped), menus and the D3D HUD. Supports uncompressed colour-mapped (type 1),
//! true-colour (type 2) and grayscale (type 3, `Data/MenuBack.TGA`) images; RLE variants are
//! not used by the game data.

use crate::bytes::{err, slice, u16_, u8_, Result};

pub struct Image {
    pub width: u32,
    pub height: u32,
    /// RGBA8, top row first
    pub rgba: Vec<u8>,
    /// type 1 only: the raw colour-map indices (top row first) and the map as RGBA
    pub index: Vec<u8>,
    pub palette: Vec<[u8; 4]>,
}

pub fn decode(d: &[u8]) -> Result<Image> {
    let id_len = u8_(d, 0)? as usize;
    let (cmap_type, ty) = (u8_(d, 1)?, u8_(d, 2)?);
    let (cmap_first, cmap_len, cmap_bits) = (u16_(d, 3)? as usize, u16_(d, 5)? as usize, u8_(d, 7)?);
    let (w, h) = (u16_(d, 12)? as usize, u16_(d, 14)? as usize);
    let (bpp, desc) = (u8_(d, 16)?, u8_(d, 17)?);
    let mut p = 18 + id_len;
    let entry = |b: &[u8], bits: u8| -> [u8; 4] {
        match bits {
            32 => [b[2], b[1], b[0], b[3]],
            24 => [b[2], b[1], b[0], 255],
            _ => {
                let v = u16::from_le_bytes([b[0], b[1]]);
                let c = |s: u16| (((v >> s) & 31) * 255 / 31) as u8;
                [c(10), c(5), c(0), 255]
            }
        }
    };
    let mut palette = Vec::new();
    if cmap_type == 1 {
        let es = (cmap_bits as usize).div_ceil(8);
        let raw = slice(d, p, cmap_len * es)?;
        palette = raw.chunks(es).map(|b| entry(b, cmap_bits)).collect();
        p += cmap_len * es;
    }
    let px_bytes = (bpp as usize).div_ceil(8);
    let raw = slice(d, p, w * h * px_bytes)?;
    let mut rgba = vec![0u8; w * h * 4];
    let mut index = if ty == 1 { vec![0u8; w * h] } else { Vec::new() };
    for (i, b) in raw.chunks(px_bytes).enumerate() {
        let c = match ty {
            1 => *palette.get((b[0] as usize).wrapping_sub(cmap_first)).unwrap_or(&[0, 0, 0, 255]),
            2 => entry(b, bpp),
            3 => [b[0], b[0], b[0], 255],
            _ => return err(format!("unsupported TGA image type {ty}")),
        };
        // descriptor bit 5: origin at the top
        let (x, y) = (i % w, i / w);
        let row = if desc & 0x20 != 0 { y } else { h - 1 - y };
        rgba[(row * w + x) * 4..][..4].copy_from_slice(&c);
        if ty == 1 {
            index[row * w + x] = b[0];
        }
    }
    Ok(Image { width: w as u32, height: h as u32, rgba, index, palette })
}

#[cfg(test)]
mod tests {
    #[test]
    fn colour_mapped_bottom_up() {
        // 2x2, 2-entry 24-bit palette, bottom-left origin
        let mut d = vec![0u8, 1, 1, 0, 0, 2, 0, 24, 0, 0, 0, 0, 2, 0, 2, 0, 8, 0];
        d.extend([0, 0, 255, 255, 0, 0]); // BGR: red, blue
        d.extend([0, 1, 1, 0]); // bottom row: red blue; top row: blue red
        let img = super::decode(&d).unwrap();
        assert_eq!(&img.rgba[0..4], &[0, 0, 255, 255]);
        assert_eq!(&img.rgba[8..12], &[255, 0, 0, 255]);
        assert_eq!(img.index, vec![1, 0, 0, 1]);
        assert_eq!(img.palette.len(), 2);
    }
}
