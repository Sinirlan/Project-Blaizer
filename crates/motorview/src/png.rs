//! Minimal PNG writer (stored deflate blocks), enough for screenshots.

use std::io::Write;
use std::path::Path;

fn crc32(data: &[u8]) -> u32 {
    let mut c = 0xffff_ffffu32;
    for &b in data {
        c ^= b as u32;
        for _ in 0..8 {
            c = if c & 1 != 0 { 0xedb8_8320 ^ (c >> 1) } else { c >> 1 };
        }
    }
    !c
}

fn adler32(data: &[u8]) -> u32 {
    let (mut a, mut b) = (1u32, 0u32);
    for &x in data {
        a = (a + x as u32) % 65521;
        b = (b + a) % 65521;
    }
    (b << 16) | a
}

fn chunk(out: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
    out.extend_from_slice(&(data.len() as u32).to_be_bytes());
    let mut body = kind.to_vec();
    body.extend_from_slice(data);
    out.extend_from_slice(&body);
    out.extend_from_slice(&crc32(&body).to_be_bytes());
}

/// Write RGBA8 pixels as a PNG.
pub fn write(path: &Path, w: u32, h: u32, rgba: &[u8]) -> std::io::Result<()> {
    let mut raw = Vec::with_capacity((w * 4 + 1) as usize * h as usize);
    for row in rgba.chunks((w * 4) as usize) {
        raw.push(0);
        raw.extend_from_slice(row);
    }
    let mut z = vec![0x78, 0x01];
    for (i, block) in raw.chunks(65535).enumerate() {
        let last = (i + 1) * 65535 >= raw.len();
        z.push(last as u8);
        z.extend_from_slice(&(block.len() as u16).to_le_bytes());
        z.extend_from_slice(&(!(block.len() as u16)).to_le_bytes());
        z.extend_from_slice(block);
    }
    z.extend_from_slice(&adler32(&raw).to_be_bytes());
    let mut png = b"\x89PNG\r\n\x1a\n".to_vec();
    let mut ihdr = Vec::new();
    ihdr.extend_from_slice(&w.to_be_bytes());
    ihdr.extend_from_slice(&h.to_be_bytes());
    ihdr.extend_from_slice(&[8, 6, 0, 0, 0]);
    chunk(&mut png, b"IHDR", &ihdr);
    chunk(&mut png, b"IDAT", &z);
    chunk(&mut png, b"IEND", &[]);
    std::fs::File::create(path)?.write_all(&png)
}

/// Read a PNG as `write` makes it (RGBA8, stored deflate blocks, no row filters); other PNGs
/// are refused rather than decoded, there is no inflater here.
pub fn read(png: &[u8]) -> Result<(u32, u32, Vec<u8>), String> {
    let be = |b: &[u8]| u32::from_be_bytes([b[0], b[1], b[2], b[3]]);
    if png.len() < 8 || &png[..8] != b"\x89PNG\r\n\x1a\n" {
        return Err("not a PNG".into());
    }
    let (mut at, mut size, mut z) = (8, None, Vec::new());
    while at + 8 <= png.len() {
        let len = be(&png[at..]) as usize;
        let data = png.get(at + 8..at + 8 + len).ok_or("truncated PNG")?;
        match &png[at + 4..at + 8] {
            b"IHDR" if len >= 13 => {
                if data[8..13] != [8, 6, 0, 0, 0] {
                    return Err("only 8-bit RGBA, non-interlaced PNGs are read".into());
                }
                size = Some((be(data), be(&data[4..])));
            }
            b"IDAT" => z.extend_from_slice(data),
            b"IEND" => break,
            _ => {}
        }
        at += 12 + len;
    }
    let (w, h) = size.ok_or("no IHDR")?;
    let mut raw = Vec::with_capacity((w * 4 + 1) as usize * h as usize);
    let mut p = 2;
    loop {
        let head = *z.get(p).ok_or("truncated deflate stream")?;
        if head & 6 != 0 {
            return Err("compressed PNG (only stored blocks are read)".into());
        }
        let len = u16::from_le_bytes([z[p + 1], z[p + 2]]) as usize;
        raw.extend_from_slice(z.get(p + 5..p + 5 + len).ok_or("truncated deflate block")?);
        p += 5 + len;
        if head & 1 != 0 {
            break;
        }
    }
    let row = (w * 4) as usize;
    if raw.len() < (row + 1) * h as usize {
        return Err("short image data".into());
    }
    let mut rgba = Vec::with_capacity(row * h as usize);
    for r in raw.chunks(row + 1).take(h as usize) {
        if r[0] != 0 {
            return Err("filtered PNG rows (only filter 0 is read)".into());
        }
        rgba.extend_from_slice(&r[1..]);
    }
    Ok((w, h, rgba))
}

#[cfg(test)]
mod tests {
    #[test]
    fn round_trip() {
        let path = std::env::temp_dir().join(format!("motorview_png_{}.png", std::process::id()));
        let px: Vec<u8> = (0..3 * 2 * 4).map(|i| i as u8 * 7).collect();
        super::write(&path, 3, 2, &px).unwrap();
        let back = super::read(&std::fs::read(&path).unwrap()).unwrap();
        let _ = std::fs::remove_file(&path);
        assert_eq!(back, (3, 2, px));
    }
}
