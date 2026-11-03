//! RAD Smacker 2 (`SMK2`) video decoder, the format of `movies/movies.pak` (05-formats "Smacker").
//!
//! Written from the public format description, not from any other decoder's source. Scope = what the
//! shipped movies use [O]: SMK2, header flags 0 (no ring frame, no interlace, no Y doubling).
//! SMK4, ring frames and interlaced / doubled video are refused with an error [design choice].
//! Audio is Smacker DPCM (Huffman-coded deltas) or plain PCM, decoded to interleaved `i16`.
//!
//! One frame = optional palette record, one record per audio track, then the video bit stream.
//! The picture is kept between frames: skipped blocks keep what the previous frame drew.

use crate::bytes::{err, slice, u32_, Error, Result};
use std::io::{Read, Seek, SeekFrom};
use std::time::Duration;

const HEADER: usize = 0x68;
const MAX_FRAMES: usize = 1 << 20;
const MAX_FRAME_BYTES: usize = 64 << 20;
/// Audio tracks a file can carry (one bit each in a frame's type byte).
pub const TRACKS: usize = 7;

/// Blocks a "run" value of a Type code stands for: 1..=59, then 128..=2048.
const RUNS: [u16; 64] = {
    let mut t = [0u16; 64];
    let mut i = 0;
    while i < 59 {
        t[i] = i as u16 + 1;
        i += 1;
    }
    t[59] = 128;
    t[60] = 256;
    t[61] = 512;
    t[62] = 1024;
    t[63] = 2048;
    t
};

/// 6-bit palette component to 8 bit: steps of 4, plus one more at 16, 32 and 48.
const fn widen(v: u8) -> u8 {
    v * 4 + v / 16
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AudioTrack {
    pub rate: u32,
    pub channels: u8,
    pub bits: u8,
    /// Smacker DPCM (Huffman deltas); otherwise plain PCM.
    pub compressed: bool,
}

#[derive(Debug, Clone)]
pub struct Info {
    pub width: u32,
    pub height: u32,
    pub frames: u32,
    /// Microseconds per frame.
    pub frame_us: u32,
    pub flags: u32,
    pub audio: [Option<AudioTrack>; TRACKS],
}

/// LSB-first bit reader. Reading past the end yields zero bits and sets `overrun`, so a damaged
/// stream ends the decode loops and is reported once, afterwards.
struct Bits<'a> {
    d: &'a [u8],
    pos: usize,
}

impl<'a> Bits<'a> {
    fn new(d: &'a [u8]) -> Self {
        Bits { d, pos: 0 }
    }
    fn bit(&mut self) -> u32 {
        let b = self.d.get(self.pos >> 3).map_or(0, |&b| (b >> (self.pos & 7)) as u32 & 1);
        self.pos += 1;
        b
    }
    fn bits(&mut self, n: u32) -> u32 {
        (0..n).fold(0, |v, i| v | self.bit() << i)
    }
    fn overrun(&self) -> bool {
        self.pos > self.d.len() * 8
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Node {
    Leaf(u16),
    /// Big trees only: "the cached value number i".
    Cache(u8),
    Branch(u32, u32),
}

/// A Huffman tree read from the header: bit 0 goes left, bit 1 right.
#[derive(Debug, Clone)]
struct Tree {
    nodes: Vec<Node>,
}

impl Tree {
    /// Absent tree (its "present" bit was 0): every code decodes to 0 without reading a bit.
    fn empty() -> Tree {
        Tree { nodes: vec![Node::Leaf(0)] }
    }

    fn read(b: &mut Bits, leaf: &mut dyn FnMut(&mut Bits) -> Node) -> Tree {
        let mut nodes = Vec::new();
        Self::read_node(b, leaf, &mut nodes);
        Tree { nodes }
    }

    fn read_node(b: &mut Bits, leaf: &mut dyn FnMut(&mut Bits) -> Node, nodes: &mut Vec<Node>) -> u32 {
        let me = nodes.len() as u32;
        if b.overrun() || nodes.len() > 1 << 17 {
            nodes.push(Node::Leaf(0));
        } else if b.bit() == 1 {
            nodes.push(Node::Branch(0, 0));
            let l = Self::read_node(b, leaf, nodes);
            let r = Self::read_node(b, leaf, nodes);
            nodes[me as usize] = Node::Branch(l, r);
        } else {
            let n = leaf(b);
            nodes.push(n);
        }
        me
    }

    fn walk(&self, b: &mut Bits) -> Node {
        let mut n = 0usize;
        loop {
            match self.nodes[n] {
                Node::Branch(l, r) => n = if b.bit() == 1 { r } else { l } as usize,
                leaf => return leaf,
            }
        }
    }

    /// 8-bit tree.
    fn decode8(&self, b: &mut Bits) -> u16 {
        match self.walk(b) {
            Node::Leaf(v) => v,
            _ => 0,
        }
    }

    /// 16-bit tree with the 3-entry move-to-front cache.
    fn decode16(&self, b: &mut Bits, cache: &mut [u16; 3]) -> u16 {
        let v = match self.walk(b) {
            Node::Leaf(v) => v,
            Node::Cache(i) => cache[i as usize],
            Node::Branch(..) => 0,
        };
        if cache[0] != v {
            cache[2] = cache[1];
            cache[1] = cache[0];
            cache[0] = v;
        }
        v
    }
}

/// An 8-bit tree: a "present" bit (1), the tree, an unused 0 bit. Absent = always 0 [I].
fn read_tree8(b: &mut Bits) -> Tree {
    if b.bit() == 0 {
        return Tree::empty();
    }
    let t = Tree::read(b, &mut |b| Node::Leaf(b.bits(8) as u16));
    b.bit();
    t
}

/// One of the four 16-bit trees: a "present" bit, the lo and hi byte trees, the three cache
/// escape values, the tree itself and an unused 0 bit. Leaf = a lo and a hi code from the byte
/// trees; a value equal to an escape stands for that slot of the cache.
fn read_big_tree(b: &mut Bits) -> Tree {
    if b.bit() == 0 {
        return Tree::empty();
    }
    let lo = read_tree8(b);
    let hi = read_tree8(b);
    let mut esc = [0u16; 3];
    for e in &mut esc {
        *e = b.bits(16) as u16;
    }
    let tree = Tree::read(b, &mut |b| {
        let v = lo.decode8(b) | hi.decode8(b) << 8;
        match esc.iter().position(|&e| e == v) {
            Some(i) => Node::Cache(i as u8),
            None => Node::Leaf(v),
        }
    });
    b.bit();
    tree
}

struct Trees {
    mmap: Tree,
    mclr: Tree,
    full: Tree,
    kind: Tree,
}

/// Block types, the low two bits of a Type code.
const MONO: u16 = 0;
const FULL: u16 = 1;
const SKIP: u16 = 2;

pub struct Smacker<R> {
    r: R,
    info: Info,
    /// Where the first frame starts in the stream.
    data_start: u64,
    sizes: Vec<u32>,
    types: Vec<u8>,
    trees: Trees,
    frame: u32,
    buf: Vec<u8>,
    pixels: Vec<u8>,
    palette: [[u8; 3]; 256],
    /// This frame's samples per track, interleaved.
    audio: [Vec<i16>; TRACKS],
}

fn io_err(e: std::io::Error) -> Error {
    Error(format!("smk: {e}"))
}

impl<R: Read + Seek> Smacker<R> {
    /// `r` is positioned at the `SMK2` magic; nothing before it is touched.
    pub fn open(mut r: R) -> Result<Self> {
        let base = r.stream_position().map_err(io_err)?;
        let mut head = [0u8; HEADER];
        r.read_exact(&mut head).map_err(io_err)?;
        match &head[..4] {
            b"SMK2" => {}
            b"SMK4" => return err("smk: SMK4 is not supported"),
            _ => return err("smk: not a Smacker file"),
        }
        let width = u32_(&head, 4)?;
        let height = u32_(&head, 8)?;
        let frames = u32_(&head, 12)?;
        let rate = u32_(&head, 16)? as i32;
        let flags = u32_(&head, 20)?;
        if flags != 0 {
            return err(format!("smk: header flags {flags:#x} (ring frame / Y scaling) are not supported"));
        }
        if width == 0 || height == 0 || width % 4 != 0 || height % 4 != 0 || width > 4096 || height > 4096 {
            return err(format!("smk: size {width}x{height} (multiples of 4 up to 4096 are supported)"));
        }
        if frames as usize > MAX_FRAMES {
            return err(format!("smk: {frames} frames"));
        }
        // > 0: milliseconds, < 0: units of 10 microseconds, 0: 100 ms
        let frame_us = match rate {
            0 => 100_000,
            r if r > 0 => r as u32 * 1000,
            r => r.unsigned_abs() * 10,
        };
        let trees_size = u32_(&head, 52)? as usize;
        let mut audio = [None; TRACKS];
        for (i, a) in audio.iter_mut().enumerate() {
            let w = u32_(&head, 72 + 4 * i)?;
            let (rate, f) = (w & 0xff_ffff, w >> 24);
            if rate == 0 && f & 0x40 == 0 {
                continue;
            }
            if f & 0x0c != 0 {
                return err(format!("smk: audio track {i} uses Bink audio (flags {f:#x})"));
            }
            *a = Some(AudioTrack {
                rate,
                channels: if f & 0x10 != 0 { 2 } else { 1 },
                bits: if f & 0x20 != 0 { 16 } else { 8 },
                compressed: f & 0x80 != 0,
            });
        }
        let n = frames as usize;
        let mut tables = vec![0u8; 5 * n];
        r.read_exact(&mut tables).map_err(io_err)?;
        let sizes: Vec<u32> =
            (0..n).map(|i| u32::from_le_bytes(tables[4 * i..4 * i + 4].try_into().unwrap()) & !3).collect();
        let types = tables[4 * n..].to_vec();
        if trees_size > MAX_FRAME_BYTES {
            return err(format!("smk: tree block of {trees_size} bytes"));
        }
        let mut tree_data = vec![0u8; trees_size];
        r.read_exact(&mut tree_data).map_err(io_err)?;
        let mut b = Bits::new(&tree_data);
        let trees = Trees {
            mmap: read_big_tree(&mut b),
            mclr: read_big_tree(&mut b),
            full: read_big_tree(&mut b),
            kind: read_big_tree(&mut b),
        };
        if b.overrun() {
            return err("smk: tree block ends early");
        }
        let data_start = base + (HEADER + 5 * n + trees_size) as u64;
        Ok(Smacker {
            r,
            info: Info { width, height, frames, frame_us, flags, audio },
            data_start,
            sizes,
            types,
            trees,
            frame: 0,
            buf: Vec::new(),
            pixels: vec![0; (width * height) as usize],
            palette: [[0; 3]; 256],
            audio: Default::default(),
        })
    }

    pub fn info(&self) -> &Info {
        &self.info
    }

    pub fn frame_duration(&self) -> Duration {
        Duration::from_micros(self.info.frame_us as u64)
    }

    /// Frames decoded so far (the index `next_frame` will decode).
    pub fn frame_index(&self) -> u32 {
        self.frame
    }

    /// 8-bit palette indices of the current picture, `width * height`, top row first.
    pub fn pixels(&self) -> &[u8] {
        &self.pixels
    }

    pub fn palette(&self) -> &[[u8; 3]; 256] {
        &self.palette
    }

    /// Samples of `track` that belong to the current frame, interleaved when stereo (`i16`, 8-bit
    /// tracks widened). Empty when the frame carries none. Play the frames' samples back to back.
    pub fn audio(&self, track: usize) -> &[i16] {
        self.audio.get(track).map_or(&[], Vec::as_slice)
    }

    /// The current picture as `RGBA8` into `out` (`width * height * 4` bytes).
    pub fn rgba(&self, out: &mut [u8]) {
        let (out, _) = out.as_chunks_mut::<4>();
        for (o, &p) in out.iter_mut().zip(&self.pixels) {
            let [r, g, b] = self.palette[p as usize];
            *o = [r, g, b, 255];
        }
    }

    /// The current picture as `RGB8` into `out` (`width * height * 3` bytes).
    pub fn rgb(&self, out: &mut [u8]) {
        let (out, _) = out.as_chunks_mut::<3>();
        for (o, &p) in out.iter_mut().zip(&self.pixels) {
            *o = self.palette[p as usize];
        }
    }

    /// Start over: the next `next_frame` decodes frame 0 again.
    pub fn rewind(&mut self) -> Result<()> {
        self.r.seek(SeekFrom::Start(self.data_start)).map_err(io_err)?;
        self.frame = 0;
        self.pixels.fill(0);
        self.palette = [[0; 3]; 256];
        self.audio.iter_mut().for_each(Vec::clear);
        Ok(())
    }

    /// Decode the next frame. `Ok(false)` after the last one.
    pub fn next_frame(&mut self) -> Result<bool> {
        let i = self.frame as usize;
        if i >= self.sizes.len() {
            return Ok(false);
        }
        let size = self.sizes[i] as usize;
        if size > MAX_FRAME_BYTES {
            return err(format!("smk: frame {i} of {size} bytes"));
        }
        let mut buf = std::mem::take(&mut self.buf);
        buf.resize(size, 0);
        let res = self.r.read_exact(&mut buf).map_err(io_err).and_then(|()| self.decode_frame(&buf, self.types[i]));
        self.buf = buf;
        res.map_err(|e| Error(format!("frame {i}: {e}")))?;
        self.frame += 1;
        Ok(true)
    }

    fn decode_frame(&mut self, f: &[u8], ty: u8) -> Result<()> {
        let mut at = 0;
        if ty & 1 != 0 {
            let len = *f.first().ok_or_else(|| Error("empty frame".into()))? as usize * 4;
            self.read_palette(slice(f, 1, len.max(1) - 1)?)?;
            at = len;
        }
        for track in 0..TRACKS {
            self.audio[track].clear();
            if ty >> (track + 1) & 1 != 0 {
                let len = u32_(f, at)? as usize;
                if len < 4 {
                    return err(format!("audio record {track} of {len} bytes"));
                }
                let rec = slice(f, at + 4, len - 4)?;
                at += len;
                self.decode_audio(track, rec)?;
            }
        }
        self.decode_video(f.get(at..).unwrap_or(&[]))
    }

    /// Palette record (after its length byte): skip n, copy n entries from the previous palette,
    /// or set one entry from 3 bytes of 6-bit components.
    fn read_palette(&mut self, rec: &[u8]) -> Result<()> {
        let old = self.palette;
        let (mut i, mut p) = (0usize, 0usize);
        while i < 256 {
            let c = *rec.get(p).ok_or_else(|| Error("palette record ends early".into()))?;
            p += 1;
            if c & 0x80 != 0 {
                i += (c & 0x7f) as usize + 1;
            } else if c & 0x40 != 0 {
                let n = (c & 0x3f) as usize + 1;
                let from = *rec.get(p).ok_or_else(|| Error("palette record ends early".into()))? as usize;
                p += 1;
                if i + n > 256 || from + n > 256 {
                    return err("palette copy out of range");
                }
                self.palette[i..i + n].copy_from_slice(&old[from..from + n]);
                i += n;
            } else {
                let rgb = slice(rec, p, 2)?;
                p += 2;
                self.palette[i] = [widen(c & 0x3f), widen(rgb[0] & 0x3f), widen(rgb[1] & 0x3f)];
                i += 1;
            }
        }
        if i > 256 {
            return err("palette skip out of range");
        }
        Ok(())
    }

    /// One audio record (after its length): plain samples, or `u32` unpacked size and the DPCM
    /// stream: flag bits (data present, stereo, 16 bit), one 8-bit tree per byte lane, the start
    /// value of each channel (the first samples), then Huffman deltas added to each channel.
    fn decode_audio(&mut self, track: usize, rec: &[u8]) -> Result<()> {
        let Some(t) = self.info.audio[track] else {
            return err(format!("audio record for track {track}, which the header does not declare"));
        };
        let out = &mut self.audio[track];
        if !t.compressed {
            if t.bits == 16 {
                out.extend(rec.as_chunks::<2>().0.iter().map(|c| i16::from_le_bytes(*c)));
            } else {
                out.extend(rec.iter().map(|&v| (v as i16 - 128) << 8));
            }
            return Ok(());
        }
        let bytes = u32_(rec, 0)? as usize;
        if bytes > MAX_FRAME_BYTES {
            return err(format!("audio record unpacks to {bytes} bytes"));
        }
        let mut b = Bits::new(&rec[4..]);
        if b.bit() == 0 {
            return Ok(());
        }
        let (stereo, wide) = (b.bit() == 1, b.bit() == 1);
        if stereo != (t.channels == 2) || wide != (t.bits == 16) {
            return err(format!("audio record format differs from the header of track {track}"));
        }
        let chans = 1 + stereo as usize;
        let lanes = chans << wide as usize;
        let trees: Vec<Tree> = (0..lanes).map(|_| read_tree8(&mut b)).collect();
        let total = bytes >> wide as usize;
        let mut pred = [0i32; 2];
        for c in (0..chans).rev() {
            pred[c] = if wide { (b.bits(16) as u16).swap_bytes() as i32 } else { b.bits(8) as i32 };
        }
        out.reserve(total);
        let put = |out: &mut Vec<i16>, v: i32| out.push(if wide { v as i16 } else { (((v & 0xff) - 128) << 8) as i16 });
        for &p in &pred[..chans] {
            put(out, p);
        }
        let mask = if wide { 0xffff } else { 0xff };
        while out.len() < total && !b.overrun() {
            for c in 0..chans {
                let mut d = trees[c << wide as usize].decode8(&mut b) as i32;
                if wide {
                    d |= (trees[2 * c + 1].decode8(&mut b) as i32) << 8;
                }
                pred[c] = pred[c].wrapping_add(d) & mask;
                put(out, pred[c]);
            }
        }
        if b.overrun() {
            return err("audio data ends early");
        }
        Ok(())
    }

    fn decode_video(&mut self, data: &[u8]) -> Result<()> {
        let mut b = Bits::new(data);
        // the four caches (MMap, MClr, Full, Type) start at 0 in every frame
        let mut cache = [[0u16; 3]; 4];
        let t = &self.trees;
        let w = self.info.width as usize;
        let bw = w / 4;
        let blocks = bw * (self.info.height as usize / 4);
        let px = &mut self.pixels;
        let mut blk = 0;
        while blk < blocks && !b.overrun() {
            let code = t.kind.decode16(&mut b, &mut cache[3]);
            let run = RUNS[(code >> 2 & 63) as usize] as usize;
            let end = (blk + run).min(blocks);
            let kind = code & 3;
            if kind == SKIP {
                blk = end;
                continue;
            }
            while blk < end {
                let o = blk / bw * 4 * w + blk % bw * 4;
                match kind {
                    MONO => {
                        let clr = t.mclr.decode16(&mut b, &mut cache[1]);
                        let mut map = t.mmap.decode16(&mut b, &mut cache[0]);
                        let (c0, c1) = (clr as u8, (clr >> 8) as u8);
                        for y in 0..4 {
                            for x in 0..4 {
                                px[o + y * w + x] = if map >> x & 1 != 0 { c1 } else { c0 };
                            }
                            map >>= 4;
                        }
                    }
                    FULL => {
                        // SMK2: per row, the right pair comes first
                        for y in 0..4 {
                            let right = t.full.decode16(&mut b, &mut cache[2]);
                            let left = t.full.decode16(&mut b, &mut cache[2]);
                            px[o + y * w..][..4].copy_from_slice(&[
                                left as u8,
                                (left >> 8) as u8,
                                right as u8,
                                (right >> 8) as u8,
                            ]);
                        }
                    }
                    _ => {
                        let c = (code >> 8) as u8;
                        for y in 0..4 {
                            px[o + y * w..][..4].fill(c);
                        }
                    }
                }
                blk += 1;
            }
        }
        if b.overrun() {
            return err("video data ends early");
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Bits LSB first, as the format packs them.
    fn pack(bits: &[u8]) -> Vec<u8> {
        let mut out = vec![0u8; bits.len().div_ceil(8)];
        for (i, &b) in bits.iter().enumerate() {
            out[i / 8] |= b << (i % 8);
        }
        out
    }

    fn byte_bits(v: u8) -> impl Iterator<Item = u8> {
        (0..8).map(move |i| v >> i & 1)
    }

    #[test]
    fn bit_reader_is_lsb_first() {
        let mut b = Bits::new(&[0b1010_0110, 0x01]);
        assert_eq!(b.bit(), 0);
        assert_eq!(b.bit(), 1);
        assert_eq!(b.bits(3), 0b001);
        assert_eq!(b.bits(5), 0b0_1101);
        assert!(!b.overrun());
        assert_eq!(b.bits(8), 0);
        assert!(b.overrun());
    }

    #[test]
    fn tiny_tree_decodes() {
        // branch(leaf 5, branch(leaf 7, leaf 9)), then the unused 0 bit
        let mut bits = vec![1, 0];
        bits.extend(byte_bits(5));
        bits.push(1);
        bits.push(0);
        bits.extend(byte_bits(7));
        bits.push(0);
        bits.extend(byte_bits(9));
        bits.push(0);
        let bytes = pack(&bits);
        let mut b = Bits::new(&bytes);
        let t = Tree::read(&mut b, &mut |b| Node::Leaf(b.bits(8) as u16));
        assert_eq!(t.nodes.len(), 5);
        // codes: 0 -> 5, 10 -> 7, 11 -> 9
        let data = pack(&[1, 1, 0, 1, 0]);
        let mut d = Bits::new(&data);
        assert_eq!([t.decode8(&mut d), t.decode8(&mut d), t.decode8(&mut d)], [9, 5, 7]);
    }

    #[test]
    fn big_tree_cache_is_move_to_front() {
        // leaf A = cache 0, leaf B = cache 1, leaf C = 0x1234; codes 00, 01, 1
        let mut cache = [0u16; 3];
        let t = Tree { nodes: vec![
            Node::Branch(1, 4),
            Node::Branch(2, 3),
            Node::Cache(0),
            Node::Cache(1),
            Node::Leaf(0x1234),
        ] };
        let data = pack(&[1, 0, 1, 0, 0]);
        let mut d = Bits::new(&data);
        assert_eq!(t.decode16(&mut d, &mut cache), 0x1234); // cache = [1234, 0, 0]
        assert_eq!(cache, [0x1234, 0, 0]);
        assert_eq!(t.decode16(&mut d, &mut cache), 0); // cache 1 = 0 -> front
        assert_eq!(cache, [0, 0x1234, 0]);
        assert_eq!(t.decode16(&mut d, &mut cache), 0); // cache 0 stays
        assert_eq!(cache, [0, 0x1234, 0]);
    }

    #[test]
    fn six_bit_table_ends() {
        assert_eq!([widen(0), widen(1), widen(16), widen(32), widen(48), widen(63)], [0, 4, 65, 130, 195, 255]);
    }

    /// A decoder with no stream behind it, for the record parsers.
    fn bare(audio: [Option<AudioTrack>; TRACKS]) -> Smacker<std::io::Cursor<Vec<u8>>> {
        Smacker {
            r: std::io::Cursor::new(Vec::new()),
            info: Info { width: 4, height: 4, frames: 0, frame_us: 40_000, flags: 0, audio },
            data_start: 0,
            sizes: vec![],
            types: vec![],
            trees: Trees { mmap: Tree::empty(), mclr: Tree::empty(), full: Tree::empty(), kind: Tree::empty() },
            frame: 0,
            buf: vec![],
            pixels: vec![0; 16],
            palette: [[0; 3]; 256],
            audio: Default::default(),
        }
    }

    #[test]
    fn palette_record_skip_copy_set() {
        let mut s = bare([None; TRACKS]);
        s.palette[3] = [1, 2, 3];
        // set entry 0 = (63, 32, 0); skip 2 (1..=2); copy 2 entries from old 3 to 3..=4;
        // skip the remaining 251 in two skips
        let mut rec = vec![63u8, 32, 0, 0x81, 0x41, 3];
        rec.extend([0x80 | 127, 0x80 | 122]);
        s.read_palette(&rec).unwrap();
        assert_eq!(s.palette[0], [255, 130, 0]);
        assert_eq!(s.palette[1], [0, 0, 0]);
        assert_eq!(s.palette[3], [1, 2, 3]);
        assert_eq!(s.palette[4], [0, 0, 0]);
        // a record that ends early is an error
        assert!(s.read_palette(&[63, 32]).is_err());
    }

    #[test]
    fn audio_dpcm_16_bit_mono() {
        let track = AudioTrack { rate: 22050, channels: 1, bits: 16, compressed: true };
        let mut s = bare([Some(track), None, None, None, None, None, None]);
        // 6 bytes = 3 samples; flags present, mono, 16 bit; two one-leaf trees (low byte 1, high
        // byte 0: every delta is +1); the start value 0x0201 is stored byte-swapped
        let mut bits = vec![1, 0, 1];
        for lane in [1u8, 0] {
            bits.extend([1, 0]);
            bits.extend((0..8).map(|i| lane >> i & 1));
            bits.push(0);
        }
        bits.extend((0..16).map(|i| (0x0102u16 >> i & 1) as u8));
        let mut rec = 6u32.to_le_bytes().to_vec();
        rec.extend(pack(&bits));
        s.decode_audio(0, &rec).unwrap();
        assert_eq!(s.audio(0), [0x201, 0x202, 0x203]);
        // a record whose flags disagree with the header is refused
        let mut st = rec.clone();
        st[4] |= 2;
        assert!(s.decode_audio(0, &st).is_err());
    }

    #[test]
    fn audio_plain_pcm() {
        let t = |bits| Some(AudioTrack { rate: 11025, channels: 1, bits, compressed: false });
        let mut s = bare([t(16), t(8), None, None, None, None, None]);
        s.decode_audio(0, &[0x34, 0x12, 0xff, 0xff]).unwrap();
        assert_eq!(s.audio(0), [0x1234, -1]);
        s.decode_audio(1, &[128, 129, 0]).unwrap();
        assert_eq!(s.audio(1), [0, 256, -32768]);
        // an undeclared track is an error
        assert!(s.decode_audio(2, &[0, 0]).is_err());
    }
}
