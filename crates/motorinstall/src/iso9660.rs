//! ISO 9660 file system with Joliet names (ECMA-119 + Joliet SVD), read-only, enough for the
//! data track of a CD: volume descriptors from sector 16, directory records, file extents.

use std::fs::File;
use std::io::{self, Read};

use crate::disc::{DataTrack, USER};

#[derive(Clone, Debug)]
pub struct Entry {
    pub name: String,
    pub lba: u64,
    pub size: u64,
    pub dir: bool,
}

pub struct Iso {
    track: DataTrack,
    root: Entry,
    joliet: bool,
}

impl Iso {
    /// Read the volume descriptors; the Joliet tree (long, mixed-case names) wins over the
    /// primary one when the disc has both.
    pub fn open(track: DataTrack) -> Result<Iso, String> {
        let err = |e: io::Error| format!("{}: {e}", track.file.display());
        let mut f = File::open(&track.file).map_err(err)?;
        let mut primary = None;
        let mut joliet = None;
        for lba in 16..64 {
            let s = track.read(&mut f, lba, 1).map_err(err)?;
            if s.len() < USER as usize || &s[1..6] != b"CD001" {
                break;
            }
            match s[0] {
                1 if primary.is_none() => primary = Some(record(&s[156..190], false)),
                2 if matches!(&s[88..91], b"%/@" | b"%/C" | b"%/E") => joliet = Some(record(&s[156..190], true)),
                255 => break,
                _ => {}
            }
        }
        let (root, joliet) = match (joliet, primary) {
            (Some(r), _) => (r, true),
            (None, Some(r)) => (r, false),
            _ => return Err(format!("{}: no ISO 9660 volume", track.file.display())),
        };
        let root = root.ok_or("bad ISO 9660 root directory")?;
        Ok(Iso { track, root, joliet })
    }

    pub fn read_dir(&self, dir: &Entry) -> Result<Vec<Entry>, String> {
        let mut f = File::open(&self.track.file).map_err(|e| e.to_string())?;
        let data = self.track.read(&mut f, dir.lba, dir.size.div_ceil(USER)).map_err(|e| e.to_string())?;
        let mut out = Vec::new();
        let mut p = 0;
        let end = (dir.size as usize).min(data.len());
        while p < end {
            let len = data[p] as usize;
            if len == 0 {
                // records do not cross sectors: the rest of this one is padding
                p = (p / USER as usize + 1) * USER as usize;
                continue;
            }
            if p + len > end {
                break;
            }
            // "." and ".." have empty names
            out.extend(record(&data[p..p + len], self.joliet).filter(|e| !e.name.is_empty()));
            p += len;
        }
        Ok(out)
    }

    /// Look a path up case-insensitively ('/' or '\\' separators).
    pub fn find(&self, path: &str) -> Result<Option<Entry>, String> {
        let mut cur = self.root.clone();
        for part in path.split(['/', '\\']).filter(|p| !p.is_empty()) {
            if !cur.dir {
                return Ok(None);
            }
            match self.read_dir(&cur)?.into_iter().find(|e| e.name.eq_ignore_ascii_case(part)) {
                Some(e) => cur = e,
                None => return Ok(None),
            }
        }
        Ok(Some(cur))
    }

    pub fn reader(&self, e: &Entry) -> Result<FileReader, String> {
        let file = File::open(&self.track.file).map_err(|e| format!("{}: {e}", self.track.file.display()))?;
        Ok(FileReader { track: self.track.clone(), file, lba: e.lba, left: e.size, buf: Vec::new(), at: 0 })
    }
}

/// A directory record ("." and ".." get an empty name); `None` when malformed.
fn record(r: &[u8], joliet: bool) -> Option<Entry> {
    if r.len() < 34 {
        return None;
    }
    let lba = u32::from_le_bytes(r[2..6].try_into().ok()?) as u64;
    let size = u32::from_le_bytes(r[10..14].try_into().ok()?) as u64;
    let dir = r[25] & 2 != 0;
    let n = r[32] as usize;
    let raw = r.get(33..33 + n)?;
    let name = if raw == [0] || raw == [1] {
        String::new()
    } else if joliet {
        joliet_name(raw)
    } else {
        String::from_utf8_lossy(raw).into_owned()
    };
    let name = name.split(';').next().unwrap_or("").trim_end_matches('.').to_string();
    Some(Entry { name, lba, size, dir })
}

/// Joliet names are UCS-2 big-endian.
pub fn joliet_name(raw: &[u8]) -> String {
    let units: Vec<u16> = raw.as_chunks::<2>().0.iter().map(|&c| u16::from_be_bytes(c)).collect();
    String::from_utf16_lossy(&units)
}

/// Streams one file's extent off the data track.
pub struct FileReader {
    track: DataTrack,
    file: File,
    lba: u64,
    left: u64,
    buf: Vec<u8>,
    at: usize,
}

impl Read for FileReader {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        if self.at == self.buf.len() {
            if self.left == 0 {
                return Ok(0);
            }
            const CHUNK: u64 = 256;
            let n = self.left.div_ceil(USER).min(CHUNK);
            self.buf = self.track.read(&mut self.file, self.lba, n)?;
            self.buf.truncate(self.left.min(n * USER) as usize);
            if self.buf.is_empty() {
                return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "disc image ends inside a file"));
            }
            self.lba += n;
            self.left -= self.buf.len() as u64;
            self.at = 0;
        }
        let k = out.len().min(self.buf.len() - self.at);
        out[..k].copy_from_slice(&self.buf[self.at..self.at + k]);
        self.at += k;
        Ok(k)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names() {
        assert_eq!(joliet_name(&[0, b'm', 0, b'o', 0, b'v', 0, b'i', 0, b'e', 0, b's']), "movies");
        let mut r = vec![0u8; 33];
        r[25] = 0;
        r[32] = 10;
        r.extend_from_slice(b"MOTOR.TXT;1");
        r[32] = 11;
        r[2] = 7;
        r[10] = 9;
        let e = record(&r, false).unwrap();
        assert_eq!((e.name.as_str(), e.lba, e.size, e.dir), ("MOTOR.TXT", 7, 9, false));
    }
}
