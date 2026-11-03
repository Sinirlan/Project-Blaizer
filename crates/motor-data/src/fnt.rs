//! Bitmap fonts (`Data/FONT1.FNT`, `FONT1S`, `FONT2`, `FONT2S`), loaded by 0x489d84.
//!
//! Like the sprite banks, every glyph is *compiled*: a run of x86 stores of one colour register
//! into the destination, then a `jmp` back (the loader patches it). We interpret the stores, which
//! gives a 1-bit mask per glyph. FONT1 / FONT1S are 8-bit (1 byte per pixel), FONT2 / FONT2S the
//! 16-bit twins (2 bytes per pixel; the same shapes). Spec: re-notes/05-formats.md §FNT.

use crate::bytes::{err, slice, u32_, Result};

/// One character: `advance` and the pixel box are in pixels.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Glyph {
    pub advance: u32,
    /// box width: the advance, or more when a pixel lies beyond it
    pub w: u32,
    /// one `u32` per row, bit `x` set = pixel (x, row); `height` rows
    pub rows: Vec<u32>,
}

#[derive(Debug, Clone)]
pub struct Font {
    pub height: u32,
    /// 256 characters
    pub glyphs: Vec<Glyph>,
}

/// Interpret one glyph's code; `bpp` = bytes per pixel (1 or 2).
fn run_glyph(d: &[u8], mut p: usize, bpp: usize, height: usize, rows: &mut [u32]) -> Result<()> {
    let mut row = 0usize;
    loop {
        let b = *d.get(p).ok_or_else(|| crate::bytes::Error("glyph code past end".into()))?;
        let b1 = *d.get(p + 1).unwrap_or(&0);
        // (bytes stored, displacement byte offset, code length)
        let (n, disp_at, len) = match (b, b1) {
            (0xe9, _) => return Ok(()),                  // jmp (back into the blitter)
            (0x01, 0xf8) => { row += 1; p += 2; continue; } // add eax, edi: next row
            (0x01, 0xf0) => { row += 2; p += 2; continue; } // add eax, esi: two rows down
            (0x88, 0x58) => (1, 2, 3),                   // mov [eax+d8], bl
            (0x89, 0x58) => (4, 2, 3),                   // mov [eax+d8], ebx
            (0xdd, 0x50) => (8, 2, 3),                   // fst qword [eax+d8]
            (0x66, 0x89) if d.get(p + 2) == Some(&0x58) => (2, 3, 4), // mov [eax+d8], bx
            _ => return err(format!("glyph code {b:02x} {b1:02x} at {p:#x}")),
        };
        let disp = *d.get(p + disp_at).ok_or_else(|| crate::bytes::Error("glyph code past end".into()))? as i8 as i64;
        if row >= height {
            return err(format!("glyph row {row} past height {height}"));
        }
        for k in 0..n as i64 {
            let x = disp + k;
            if x < 0 || x as usize / bpp >= 32 {
                return err(format!("glyph store at x = {x}"));
            }
            rows[row] |= 1 << (x as usize / bpp);
        }
        p += len;
    }
}

impl Font {
    /// `bpp`: 1 for FONT1 / FONT1S, 2 for FONT2 / FONT2S.
    pub fn parse(d: &[u8], bpp: usize) -> Result<Font> {
        if !(1..=2).contains(&bpp) {
            return err(format!("bytes per pixel {bpp}"));
        }
        let height = u32_(d, 0)?;
        if height == 0 || height > 64 {
            return err(format!("font height {height}"));
        }
        let mut glyphs = Vec::with_capacity(256);
        for c in 0..256 {
            let offset = u32_(d, 4 + 8 * c)? as usize;
            let advance = u32_(d, 8 + 8 * c)? / bpp as u32;
            slice(d, offset, 1)?;
            let mut rows = vec![0u32; height as usize];
            run_glyph(d, offset, bpp, height as usize, &mut rows)?;
            let ink = rows.iter().map(|r| 32 - r.leading_zeros()).max().unwrap_or(0);
            glyphs.push(Glyph { advance, w: advance.max(ink), rows });
        }
        Ok(Font { height, glyphs })
    }

    pub fn glyph(&self, c: char) -> &Glyph {
        &self.glyphs[(c as usize).min(255)]
    }

    /// Advance of a string, as 0x489bfc sums it (chars above 255 count as 255).
    pub fn width(&self, s: &str) -> u32 {
        s.chars().map(|c| self.glyph(c).advance).sum()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// a 2-row font of 256 copies of one glyph: every opcode once
    fn synthetic() -> Vec<u8> {
        let mut d = vec![0u8; 4 + 8 * 256];
        d[0..4].copy_from_slice(&2u32.to_le_bytes());
        let off = d.len() as u32;
        for c in 0..256 {
            d[4 + 8 * c..8 + 8 * c].copy_from_slice(&off.to_le_bytes());
            d[8 + 8 * c..12 + 8 * c].copy_from_slice(&6u32.to_le_bytes());
        }
        // row 0: byte at 0, word at 1..3, dword... ; row 1: qword at 0
        d.extend([0x88, 0x58, 0x00, 0x66, 0x89, 0x58, 0x02, 0x01, 0xf8, 0xdd, 0x50, 0x00, 0xe9, 0, 0, 0, 0]);
        d
    }

    #[test]
    fn opcodes() {
        let f = Font::parse(&synthetic(), 1).unwrap();
        let g = f.glyph('A');
        assert_eq!(g.rows, vec![0b1101, 0xff]);
        assert_eq!((g.advance, g.w), (6, 8));
        assert_eq!(f.width("AB"), 12);
        // 16-bit: bytes halve into pixels
        let f2 = Font::parse(&synthetic(), 2).unwrap();
        assert_eq!(f2.glyph('A').advance, 3);
        assert_eq!(f2.glyph('A').rows, vec![0b11, 0b1111]);
    }

    #[test]
    fn rejects_garbage() {
        let mut d = synthetic();
        let n = d.len();
        d[n - 5] = 0x90; // nop where the jmp was
        assert!(Font::parse(&d, 1).is_err());
    }

    fn path(name: &str) -> Option<std::path::PathBuf> {
        let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../installed_game/Data").join(name);
        p.exists().then_some(p)
    }

    #[test]
    fn install_fonts() {
        let (Some(a), Some(b)) = (path("FONT1.FNT"), path("FONT2.FNT")) else { return };
        let f1 = Font::parse(&std::fs::read(a).unwrap(), 1).unwrap();
        let f2 = Font::parse(&std::fs::read(b).unwrap(), 2).unwrap();
        assert_eq!(f1.height, 19);
        let a = f1.glyph('A');
        assert_eq!((a.advance, a.rows.len()), (10, 19));
        assert_eq!(a.rows[1], 0b0111_1111 << 1);
        // FONT2 draws the same shapes as FONT1 except chars 254 / 255
        for c in 0..256 {
            assert_eq!(f1.glyphs[c] == f2.glyphs[c], c < 254, "char {c}");
        }
        for n in ["FONT1S.FNT", "FONT2S.FNT"] {
            let f = Font::parse(&std::fs::read(path(n).unwrap()).unwrap(), if n.contains('2') { 2 } else { 1 }).unwrap();
            assert_eq!(f.height, 8);
        }
    }
}
