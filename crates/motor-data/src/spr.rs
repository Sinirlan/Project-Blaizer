//! Sprite banks (`Data/*.SPR`, `Tracks/*/Data/*.DTA`), loaded whole by 0x409fe8.
//!
//! Sprites are *compiled*: each row is a small x86 routine that stores the row's opaque
//! pixels relative to EAX (the destination row pointer) and returns. We interpret the
//! stores instead of running them. Pixel depth depends on the bank: MENU/MISC/DTA are 8-bit
//! indices into the bank palette; the in-game HUD banks are 16-bit (game0/GAME2 RGB555,
//! game1/GAME3 RGB565, one per display mode). The record width is in bytes.
//! Spec: re-notes/05-formats.md §SPR.

use crate::bytes::{err, i32_, slice, u32_, Result};

#[derive(Debug, Clone)]
pub struct Sprite {
    pub name: String,
    pub offset: usize,
    pub frames: usize,
    /// row width in bytes (pixels for 8-bit banks, 2 * pixels for 16-bit banks)
    pub width: usize,
    pub height: usize,
    /// per frame: offset of its row table from `offset`
    pub tables: Vec<usize>,
}

pub struct Bank<'a> {
    data: &'a [u8],
    pub sprites: Vec<Sprite>,
    /// 256 RGB entries, when the header's top bit is set
    pub palette: Option<Vec<[u8; 3]>>,
}

pub fn parse(d: &[u8]) -> Result<Bank<'_>> {
    let w = u32_(d, 0)?;
    let (n, has_pal) = ((w & 0x7fff_ffff) as usize, w >> 31 != 0);
    let mut sprites = Vec::with_capacity(n);
    for i in 0..n {
        let name = crate::bytes::latin1(slice(d, 4 + 12 * i, 8)?.split(|&b| b == 0).next().unwrap_or(&[]));
        let offset = u32_(d, 12 + 12 * i)? as usize;
        let frames = u32_(d, offset)? as usize;
        if frames == 0 || frames > 64 {
            return err(format!("sprite {name}: {frames} frames"));
        }
        let mut tables = vec![u32_(d, offset + 12)? as usize];
        for f in 1..frames {
            tables.push(u32_(d, offset + 12 + 4 * f)? as usize);
        }
        sprites.push(Sprite { name, offset, frames, width: u32_(d, offset + 4)? as usize, height: u32_(d, offset + 8)? as usize, tables });
    }
    let palette = if has_pal {
        let p = slice(d, 4 + 12 * n, 768)?;
        Some(p.chunks(3).map(|c| [c[0], c[1], c[2]]).collect())
    } else {
        None
    };
    Ok(Bank { data: d, sprites, palette })
}

/// Interpret one compiled row; `None` for code that is not a plain store sequence
/// (the gauge-fill frame of `meter` takes parameters in registers).
fn run_row(d: &[u8], mut p: usize, out: &mut [Option<u8>]) -> Option<()> {
    let mut base: i64 = 0;
    loop {
        let mut b = *d.get(p)?;
        match b {
            0xc3 => return Some(()), // ret
            0x05 => {
                // add eax, imm32
                base += i32_(d, p + 1).ok()? as i64;
                p += 5;
                continue;
            }
            _ => {}
        }
        let wide = b == 0x66; // operand-size prefix: 16-bit store
        if wide {
            p += 1;
            b = *d.get(p)?;
        }
        if b != 0xc6 && b != 0xc7 {
            return None;
        }
        let (disp, q) = match *d.get(p + 1)? {
            0x40 => (*d.get(p + 2)? as i8 as i64, p + 3),
            0x80 => (i32_(d, p + 2).ok()? as i64, p + 6),
            0x00 => (0, p + 2),
            _ => return None,
        };
        let size = if b == 0xc6 { 1 } else if wide { 2 } else { 4 };
        for k in 0..size {
            let x = usize::try_from(base + disp + k as i64).ok()?;
            *out.get_mut(x)? = Some(*d.get(q + k)?);
        }
        p = q + size;
    }
}

impl Bank<'_> {
    pub fn find(&self, name: &str) -> Option<&Sprite> {
        self.sprites.iter().find(|s| s.name.eq_ignore_ascii_case(name))
    }

    /// Raw bytes of one frame, row-major `height x width` (None = transparent).
    pub fn bytes(&self, s: &Sprite, frame: usize) -> Option<Vec<Option<u8>>> {
        let t = s.offset + *s.tables.get(frame)?;
        let mut out = vec![None; s.width * s.height];
        for r in 0..s.height {
            let row = t + u32_(self.data, t + 4 * r).ok()? as usize;
            run_row(self.data, row, &mut out[r * s.width..(r + 1) * s.width])?;
        }
        Some(out)
    }

    /// 8-bit frame through the bank palette -> (width, height, RGBA).
    pub fn rgba8(&self, s: &Sprite, frame: usize) -> Option<(usize, usize, Vec<u8>)> {
        let pal = self.palette.as_ref()?;
        let px = self.bytes(s, frame)?;
        let rgba = px
            .iter()
            .flat_map(|p| match p {
                Some(i) => {
                    let c = pal[*i as usize];
                    [c[0], c[1], c[2], 255]
                }
                None => [0; 4],
            })
            .collect();
        Some((s.width, s.height, rgba))
    }

    /// 16-bit frame (RGB555, or RGB565 when `rgb565`) -> (width in pixels, height, RGBA).
    pub fn rgba16(&self, s: &Sprite, frame: usize, rgb565: bool) -> Option<(usize, usize, Vec<u8>)> {
        let px = self.bytes(s, frame)?;
        let w = s.width / 2;
        let mut rgba = Vec::with_capacity(w * s.height * 4);
        for r in 0..s.height {
            for x in 0..w {
                match (px[r * s.width + 2 * x], px[r * s.width + 2 * x + 1]) {
                    (Some(lo), Some(hi)) => {
                        let v = lo as u16 | (hi as u16) << 8;
                        let c5 = |s: u16| ((v >> s & 31) * 255 / 31) as u8;
                        let g = if rgb565 { ((v >> 5 & 63) * 255 / 63) as u8 } else { c5(5) };
                        rgba.extend([if rgb565 { c5(11) } else { c5(10) }, g, c5(0), 255]);
                    }
                    _ => rgba.extend([0; 4]),
                }
            }
        }
        Some((w, s.height, rgba))
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn compiled_rows() {
        // one sprite, 6 bytes x 1 row: mov dword [eax+0], 01020304; add eax,4; mov byte [eax+1], 9; ret
        let mut d = vec![0u8; 4 + 12];
        d[0..4].copy_from_slice(&1u32.to_le_bytes());
        d[4..5].copy_from_slice(b"x");
        let off = d.len() as u32;
        d[12..16].copy_from_slice(&off.to_le_bytes());
        for v in [1u32, 6, 1, 16, 4] {
            d.extend(v.to_le_bytes()); // frames, width, height, table0, row0 offset (from table)
        }
        d.extend([0xc7, 0x40, 0x00, 4, 3, 2, 1, 0x05, 4, 0, 0, 0, 0xc6, 0x40, 0x01, 9, 0xc3]);
        let b = super::parse(&d).unwrap();
        let px = b.bytes(&b.sprites[0], 0).unwrap();
        assert_eq!(px, vec![Some(4), Some(3), Some(2), Some(1), None, Some(9)]);
    }
}
