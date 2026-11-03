//! "TBL" container: optional encryption (flags bit0) and compression (flags bit1).
//!
//! motor.exe: load 0x488ba8, decrypt 0x486108, keystream 0x485ecc, unpack 0x488a90.

use crate::bytes::{err, Result};

/// key1, the string at VA 0x4c4b60.
const KEY1: &[u8] = &[0x01, 0x03, 0xfa, 0x15, 0x20, 0xfa, 0x2a, 0x80, 0xaa];
/// Fallback key used when key2 is NULL/empty (dword at VA 0x4c4b50).
const DEFAULT_KEY: &[u8] = b"EQ";

/// Port of the key schedule at 0x485ecc.
pub fn keystream(key: &[u8], n: usize) -> Vec<u8> {
    let key = if key.is_empty() { DEFAULT_KEY } else { key };
    let mut l = key.len().min(n / 2 - 1);
    let mut cur = vec![0u8; n];
    cur[..l].copy_from_slice(&key[..l]);
    let mut other = vec![0u8; n];
    let mut x = (l as u32).wrapping_pow(16);
    loop {
        l = l.min(n / 2);
        x = x.wrapping_mul(l as u32);
        for i in 0..l {
            let b = cur[i] as u32;
            x = x.wrapping_add(b);
            x = x.wrapping_mul(x).wrapping_add(b);
            other[2 * i] = x as u8;
            other[2 * i + 1] = (x >> 8) as u8;
        }
        std::mem::swap(&mut cur, &mut other);
        let done = l == n / 2;
        l *= 2;
        if done {
            break;
        }
    }
    // tail: reversed bytes of the previous source buffer
    let (mut pos, mut r) = (l, n - l);
    while r > 0 {
        r -= 1;
        cur[pos] = other[r + 1];
        pos += 1;
    }
    cur
}

/// Port of 0x486108. `key2` is NULL for every shipped file.
pub fn decrypt(buf: &mut [u8], key2: Option<&[u8]>) {
    let a = keystream(KEY1, 0xe9);
    let b = keystream(key2.unwrap_or(&[]), 0xe3);
    let mut acc = 0u8;
    for (i, c) in buf.iter_mut().enumerate() {
        let p = c
            .wrapping_sub(a[i % a.len()])
            .wrapping_sub(b[i % b.len()])
            .wrapping_sub(acc);
        acc = acc.wrapping_add(p);
        *c = p;
    }
    let mut acc = 0u8;
    for c in buf.iter_mut().rev() {
        let p = c.wrapping_sub(acc);
        acc = acc.wrapping_add(p);
        *c = p;
    }
}

struct Bits<'a> {
    src: &'a [u8],
    pos: usize,
    bit: u32,
    cur: u8,
}

impl<'a> Bits<'a> {
    fn new(src: &'a [u8], pos: usize) -> Self {
        Bits { src, pos, bit: 0, cur: 0 }
    }
    fn next(&mut self) -> Result<u32> {
        if self.bit == 0 {
            match self.src.get(self.pos) {
                Some(&b) => self.cur = b,
                None => return err("compressed bitstream truncated"),
            }
            self.pos += 1;
            self.bit = 8;
        }
        self.bit -= 1;
        Ok(((self.cur >> self.bit) & 1) as u32)
    }
    /// `width` bits, MSB first.
    fn field(&mut self, width: u32) -> Result<u32> {
        let mut r = 0u32;
        for _ in 0..width {
            r = (r << 1) | self.next()?;
        }
        Ok(r)
    }
}

/// 0x486730: header word = data_off(22) | bias(8)<<22 | (width-1)(2)<<30.
fn huff_tree(src: &[u8]) -> Result<(Vec<u32>, usize)> {
    let h = crate::bytes::u32_(src, 0)?;
    let data_off = (h & 0x3f_ffff) as usize;
    let width = ((h >> 30) + 1) as usize;
    let bias = (h >> 22) & 0xff;
    let n = data_off.saturating_sub(4) / width;
    let mut t = Vec::with_capacity(n);
    for i in 0..n {
        let raw = crate::bytes::slice(src, 4 + i * width, width)?;
        let mut v = 0u32;
        for (k, &b) in raw.iter().enumerate() {
            v |= (b as u32) << (8 * k);
        }
        if v & 1 != 0 {
            v = (((v >> 1) + bias) << 1) | 1;
        }
        t.push(v);
    }
    Ok((t, data_off))
}

/// Code read after each Huffman symbol (tree 0x486730, MSB-first bits).
#[derive(Clone, Copy)]
enum HuffRun {
    /// Method 2 (0x4882a4): symbols only.
    None,
    /// Method 3 (0x488338): unary run; every 0 bit repeats the symbol, a 1 bit ends the run.
    Unary,
    /// Methods 4 (0x488414, 2 bits) and 5 (0x488528, 3 bits): a delta code
    /// `0` nothing, `1` repeat, `2k` symbol-k, `2k+1` symbol+k (one extra byte).
    Delta(u32),
    /// Methods 6 (0x4886c0, 2 bits), 7 (0x4887dc, 3) and 8 (0x488924, 4): a 1 bit then a
    /// count `c` repeat the symbol `c+1` more times.
    Flag(u32),
}

fn huff_decode(src: &[u8], n: usize, run: HuffRun) -> Result<Vec<u8>> {
    let (t, data_off) = huff_tree(src)?;
    let mut bits = Bits::new(src, data_off);
    let mut out = Vec::with_capacity(n);
    while out.len() < n {
        let mut p = 0usize;
        let v = loop {
            let idx = p + bits.next()? as usize;
            let v = *t.get(idx).ok_or_else(|| crate::Error("huffman index out of tree".into()))?;
            if v & 1 != 0 {
                break v;
            }
            p += v as usize;
        };
        let sym = ((v >> 1) & 0xff) as u8;
        out.push(sym);
        match run {
            HuffRun::None => {}
            HuffRun::Unary => {
                while bits.next()? == 0 {
                    out.push(sym);
                }
            }
            HuffRun::Delta(width) => {
                let c = bits.field(width)?;
                match c {
                    0 => {}
                    1 => out.push(sym),
                    c => {
                        let k = (c / 2) as u8;
                        out.push(if c & 1 == 0 { sym.wrapping_sub(k) } else { sym.wrapping_add(k) });
                    }
                }
            }
            HuffRun::Flag(width) => {
                if bits.next()? == 1 {
                    let r = bits.field(width)?;
                    out.extend(std::iter::repeat_n(sym, r as usize + 1));
                }
            }
        }
    }
    out.truncate(n);
    Ok(out)
}

/// Method 1 (0x48824c): byte0 = escape E; `E E` -> E; `E b c` -> b repeated c+4 times.
fn rle_escape(src: &[u8], n: usize) -> Result<Vec<u8>> {
    let get = |i: usize| src.get(i).copied().ok_or_else(|| crate::Error("RLE truncated".into()));
    let esc = get(0)?;
    let mut i = 1;
    let mut out = Vec::with_capacity(n);
    while out.len() < n {
        let b = get(i)?;
        i += 1;
        if b != esc {
            out.push(b);
            continue;
        }
        let b2 = get(i)?;
        i += 1;
        if b2 == esc {
            out.push(esc);
        } else {
            let c = get(i)?;
            i += 1;
            out.extend(std::iter::repeat_n(b2, c as usize + 4));
        }
    }
    out.truncate(n);
    Ok(out)
}

/// 0x488a90: word0 = size(27) | method(4)<<27 | postfilter(1)<<31. Methods 9..15 are not
/// dispatched there (the buffer is left as is). Shipped data uses 1, 2, 6 and 8 only.
pub fn unpack(buf: &[u8]) -> Result<Vec<u8>> {
    let w = crate::bytes::u32_(buf, 0)?;
    let (n, method, post) = ((w & 0x07ff_ffff) as usize, (w >> 27) & 0xf, w >> 31);
    let src = &buf[4..];
    let mut out = match method {
        0 => src.get(..n).ok_or_else(|| crate::Error("stored data truncated".into()))?.to_vec(),
        1 => rle_escape(src, n)?,
        2 => huff_decode(src, n, HuffRun::None)?,
        3 => huff_decode(src, n, HuffRun::Unary)?,
        4 => huff_decode(src, n, HuffRun::Delta(2))?,
        5 => huff_decode(src, n, HuffRun::Delta(3))?,
        6 => huff_decode(src, n, HuffRun::Flag(2))?,
        7 => huff_decode(src, n, HuffRun::Flag(3))?,
        8 => huff_decode(src, n, HuffRun::Flag(4))?,
        m => return err(format!("TBL compression method {m} does not exist (0x488a90)")),
    };
    if post != 0 {
        // 0x486850: byte-wise prefix sum over the unpacked data.
        for i in 1..out.len() {
            out[i] = out[i].wrapping_add(out[i - 1]);
        }
    }
    Ok(out)
}

/// Result of loading a possibly-TBL file.
pub struct Loaded {
    /// `None` when the file is not a TBL container (data returned unchanged).
    pub flags: Option<u8>,
    pub data: Vec<u8>,
}

pub fn load(file: &[u8], key2: Option<&[u8]>) -> Result<Loaded> {
    if file.len() <= 4 || &file[..3] != b"TBL" {
        return Ok(Loaded { flags: None, data: file.to_vec() });
    }
    let flags = file[3];
    let mut body = file[4..].to_vec();
    if flags & 1 != 0 {
        decrypt(&mut body, key2);
    }
    if flags & 2 != 0 {
        body = unpack(&body)?;
    }
    Ok(Loaded { flags: Some(flags), data: body })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keystream_lengths() {
        assert_eq!(keystream(KEY1, 0xe9).len(), 0xe9);
        assert_eq!(keystream(&[], 0xe3), keystream(DEFAULT_KEY, 0xe3));
    }

    #[test]
    fn rle_escape_codes() {
        // escape 0xff: literal 1, escaped literal 0xff, run of 7 x (2+4)
        let src = [0xff, 1, 0xff, 0xff, 0xff, 7, 2];
        assert_eq!(rle_escape(&src, 8).unwrap(), vec![1, 0xff, 7, 7, 7, 7, 7, 7]);
    }

    #[test]
    fn stored_method() {
        let mut buf = (3u32).to_le_bytes().to_vec();
        buf.extend_from_slice(b"abcd");
        assert_eq!(unpack(&buf).unwrap(), b"abc");
    }

    #[test]
    fn huffman_two_symbols() {
        // tree: entries [leaf 'a', leaf 'b'] (width 1, bias 0); data_off = 4 + 2 = 6
        let h: u32 = 6;
        let mut src = h.to_le_bytes().to_vec();
        src.extend_from_slice(&[(b'a' << 1) | 1, (b'b' << 1) | 1]);
        src.push(0b0110_0000); // bits 0,1,1,0 -> a b b a
        assert_eq!(huff_decode(&src, 4, HuffRun::None).unwrap(), b"abba");
    }

    fn two_symbol_tree(bits: u8) -> Vec<u8> {
        let mut src = 6u32.to_le_bytes().to_vec();
        src.extend_from_slice(&[(b'a' << 1) | 1, (b'b' << 1) | 1, bits]);
        src
    }

    #[test]
    fn huffman_run_codes() {
        // method 3: a, run 0,0 (two repeats), 1 (end); b, 1 -> a a a b
        let src = two_symbol_tree(0b0001_1100);
        assert_eq!(huff_decode(&src, 4, HuffRun::Unary).unwrap(), b"aaab");
        // method 4: a, code 10 (a-1); b, code 11 (b+1) -> a ` b c
        let src = two_symbol_tree(0b0101_1100);
        assert_eq!(huff_decode(&src, 4, HuffRun::Delta(2)).unwrap(), b"a`bc");
        // method 7: a, flag 1, count 010 -> a + 3 repeats
        let src = two_symbol_tree(0b0101_0000);
        assert_eq!(huff_decode(&src, 4, HuffRun::Flag(3)).unwrap(), b"aaaa");
    }

    #[test]
    fn postfilter_prefix_sum() {
        let mut buf = (4u32 | 1 << 31).to_le_bytes().to_vec();
        buf.extend_from_slice(&[1, 1, 1, 1]);
        assert_eq!(unpack(&buf).unwrap(), vec![1, 2, 3, 4]);
    }

    #[test]
    fn non_tbl_passthrough() {
        let l = load(b"hello world", None).unwrap();
        assert!(l.flags.is_none());
        assert_eq!(l.data, b"hello world");
    }
}
