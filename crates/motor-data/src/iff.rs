//! Amiga IFF ILBM textures: BMHD / CMAP / BODY, ByteRun1 or uncompressed, 1-8 planes.

use crate::bytes::{err, Result};

#[derive(Debug, Clone)]
pub struct Image {
    pub width: u32,
    pub height: u32,
    pub planes: u8,
    /// palette index per pixel
    pub indices: Vec<u8>,
    pub palette: Vec<[u8; 3]>,
    /// colour index that is transparent (masking == 2)
    pub transparent: Option<u16>,
}

impl Image {
    /// RGBA8 pixels; transparent index -> alpha 0, out-of-palette -> magenta.
    pub fn rgba(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(self.indices.len() * 4);
        for &v in &self.indices {
            let c = self.palette.get(v as usize).copied().unwrap_or([255, 0, 255]);
            out.extend_from_slice(&c);
            out.push(if self.transparent == Some(v as u16) { 0 } else { 255 });
        }
        out
    }
}

fn be16(d: &[u8], o: usize) -> u16 {
    u16::from_be_bytes([d[o], d[o + 1]])
}
fn be32(d: &[u8], o: usize) -> u32 {
    u32::from_be_bytes([d[o], d[o + 1], d[o + 2], d[o + 3]])
}

fn byterun1(src: &[u8], n: usize) -> Vec<u8> {
    let mut out = Vec::with_capacity(n);
    let mut i = 0;
    while out.len() < n && i < src.len() {
        let c = src[i];
        i += 1;
        if c < 128 {
            let end = (i + c as usize + 1).min(src.len());
            out.extend_from_slice(&src[i..end]);
            i += c as usize + 1;
        } else if c > 128 {
            let Some(&b) = src.get(i) else { break };
            out.extend(std::iter::repeat_n(b, 257 - c as usize));
            i += 1;
        }
    }
    out.truncate(n);
    out
}

pub fn decode(d: &[u8]) -> Result<Image> {
    if d.len() < 12 || &d[..4] != b"FORM" || &d[8..12] != b"ILBM" {
        return err("not an ILBM");
    }
    let end = d.len().min(8 + be32(d, 4) as usize);
    let (mut bmhd, mut cmap, mut body) = (None, None, None);
    let mut p = 12;
    while p + 8 <= end {
        let id = &d[p..p + 4];
        let sz = be32(d, p + 4) as usize;
        let data = &d[p + 8..(p + 8 + sz).min(d.len())];
        match id {
            b"BMHD" => bmhd = Some(data),
            b"CMAP" => cmap = Some(data),
            b"BODY" => body = Some(data),
            _ => {}
        }
        p += 8 + sz + (sz & 1);
    }
    let (Some(h), Some(body)) = (bmhd, body) else { return err("ILBM missing BMHD or BODY") };
    if h.len() < 20 {
        return err("short BMHD");
    }
    let (w, ht, planes, masking, comp, transp) = (be16(h, 0) as usize, be16(h, 2) as usize, h[8], h[9], h[10], be16(h, 12));
    let rowbytes = w.div_ceil(16) * 2;
    let nplanes = planes as usize + usize::from(masking == 1);
    let size = rowbytes * nplanes * ht;
    let body = if comp == 1 { byterun1(body, size) } else { body[..size.min(body.len())].to_vec() };
    if body.len() < size {
        return err("ILBM body too short");
    }
    let palette = match cmap {
        Some(c) => (0..c.len().saturating_sub(2)).step_by(3).map(|i| [c[i], c[i + 1], c[i + 2]]).collect(),
        None => (0..=255u8).map(|i| [i, i, i]).collect(),
    };
    let mut indices = Vec::with_capacity(w * ht);
    for y in 0..ht {
        let base = y * rowbytes * nplanes;
        for x in 0..w {
            let (byte, bit) = (x >> 3, 7 - (x & 7));
            let mut v = 0u8;
            for pl in 0..planes as usize {
                if body[base + pl * rowbytes + byte] >> bit & 1 != 0 {
                    v |= 1 << pl;
                }
            }
            indices.push(v);
        }
    }
    Ok(Image {
        width: w as u32,
        height: ht as u32,
        planes,
        indices,
        palette,
        transparent: (masking == 2).then_some(transp),
    })
}
