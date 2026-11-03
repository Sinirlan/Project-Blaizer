//! `movies/movies.pak`: the `DIVI` container of the intro and league movies (05-formats "movies.pak").
//!
//! `"DIVI"`, `u32 count`, `u32 data_start`, then 64 slots of `{u32 flag, u32 offset, u32 open_flags}`
//! from 0xc. An entry ends where the next one starts (the last at the end of the file).

use crate::bytes::{err, slice, u32_, Result};
use std::io::{Read, Seek, SeekFrom};

const SLOTS: usize = 64;

/// `SmackOpen` flag: every picture line is shown twice (a 640x240 movie fills 640x480) [O].
pub const Y_DOUBLE: u32 = 0x10_0000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub index: usize,
    pub flag: u32,
    pub offset: u64,
    pub size: u64,
    /// The flags the original passes to `SmackOpen` (ORed with `0x1000`, file handle) [O]: audio
    /// tracks 1..7 in bits 13..19 (`0xfe000`) and, for the 640x240 movies, `Y_DOUBLE` [O: SMACKW32 0x10004b69].
    pub open_flags: u32,
}

impl Entry {
    /// The picture is shown with every line doubled.
    pub fn y_double(&self) -> bool {
        self.open_flags & Y_DOUBLE != 0
    }
}

/// Parse the directory. `head` = the first `data_start` bytes of the file (at least 0x30c are
/// needed), `file_len` = the whole file's size, which ends the last entry.
pub fn parse(head: &[u8], file_len: u64) -> Result<Vec<Entry>> {
    if slice(head, 0, 4)? != b"DIVI" {
        return err("not a DIVI pak");
    }
    let n = u32_(head, 4)? as usize;
    if n > SLOTS {
        return err(format!("pak lists {n} entries, the directory holds {SLOTS}"));
    }
    let mut out: Vec<Entry> = Vec::with_capacity(n);
    for i in 0..n {
        let o = 12 + 12 * i;
        out.push(Entry {
            index: i,
            flag: u32_(head, o)?,
            offset: u32_(head, o + 4)? as u64,
            size: 0,
            open_flags: u32_(head, o + 8)?,
        });
    }
    for i in 0..n {
        let end = if i + 1 < n { out[i + 1].offset } else { file_len };
        if end < out[i].offset || end > file_len {
            return err(format!("pak entry {i} ends at {end:#x}, before it starts at {:#x}", out[i].offset));
        }
        out[i].size = end - out[i].offset;
    }
    Ok(out)
}

/// Read the directory of an open pak.
pub fn read_dir<R: Read + Seek>(r: &mut R) -> Result<Vec<Entry>> {
    let io = |e: std::io::Error| crate::Error(format!("pak: {e}"));
    let len = r.seek(SeekFrom::End(0)).map_err(io)?;
    r.seek(SeekFrom::Start(0)).map_err(io)?;
    let mut head = vec![0u8; (12 + 12 * SLOTS).min(len as usize)];
    r.read_exact(&mut head).map_err(io)?;
    parse(&head, len)
}

/// An entry of the pak seen as its own stream (offset 0 = the start of the entry), so the movie
/// decoder can seek inside it without knowing about the container.
pub struct Section<R> {
    r: R,
    start: u64,
    len: u64,
    pos: u64,
}

impl<R: Read + Seek> Section<R> {
    pub fn new(mut r: R, e: &Entry) -> std::io::Result<Self> {
        r.seek(SeekFrom::Start(e.offset))?;
        Ok(Section { r, start: e.offset, len: e.size, pos: 0 })
    }
}

impl<R: Read + Seek> Read for Section<R> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let left = self.len.saturating_sub(self.pos) as usize;
        let want = buf.len().min(left);
        let n = self.r.read(&mut buf[..want])?;
        self.pos += n as u64;
        Ok(n)
    }
}

impl<R: Read + Seek> Seek for Section<R> {
    fn seek(&mut self, to: SeekFrom) -> std::io::Result<u64> {
        let p = match to {
            SeekFrom::Start(p) => Some(p),
            SeekFrom::End(d) => self.len.checked_add_signed(d),
            SeekFrom::Current(d) => self.pos.checked_add_signed(d),
        };
        let Some(p) = p else {
            return Err(std::io::Error::new(std::io::ErrorKind::InvalidInput, "seek before the start of the entry"));
        };
        self.r.seek(SeekFrom::Start(self.start + p))?;
        self.pos = p;
        Ok(p)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    fn sample() -> Vec<u8> {
        let mut d = vec![0u8; 12 + 12 * SLOTS];
        d[..4].copy_from_slice(b"DIVI");
        d[4..8].copy_from_slice(&2u32.to_le_bytes());
        let start = d.len() as u32;
        d[8..12].copy_from_slice(&start.to_le_bytes());
        for (i, (flag, off, buf)) in [(1u32, 780u32, 7u32), (0, 784, 9)].iter().enumerate() {
            d[12 + 12 * i..][..4].copy_from_slice(&flag.to_le_bytes());
            d[16 + 12 * i..][..4].copy_from_slice(&off.to_le_bytes());
            d[20 + 12 * i..][..4].copy_from_slice(&buf.to_le_bytes());
        }
        d.extend_from_slice(b"AAAABBBBBB");
        d
    }

    #[test]
    fn directory_and_sections() {
        let d = sample();
        let ents = read_dir(&mut Cursor::new(&d)).unwrap();
        assert_eq!(ents.len(), 2);
        assert_eq!((ents[0].offset, ents[0].size, ents[0].open_flags, ents[0].flag), (780, 4, 7, 1));
        assert_eq!((ents[1].offset, ents[1].size), (784, 6));
        let mut s = Section::new(Cursor::new(&d), &ents[1]).unwrap();
        let mut all = Vec::new();
        s.read_to_end(&mut all).unwrap();
        assert_eq!(all, b"BBBBBB");
        s.seek(SeekFrom::Start(2)).unwrap();
        let mut two = [0u8; 2];
        s.read_exact(&mut two).unwrap();
        assert_eq!(&two, b"BB");
        assert_eq!(s.seek(SeekFrom::End(0)).unwrap(), 6);
    }

    #[test]
    fn rejects_other_files() {
        assert!(parse(b"RIFF\0\0\0\0", 8).is_err());
    }
}
