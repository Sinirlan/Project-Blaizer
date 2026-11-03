//! InstallShield 5 cabinet (`DATA1.CAB` of the Motorhead CD), single volume with the header
//! inside the cab. Layout [O] on the user's disc, re-notes/01-inventory.md "Install media":
//!   +0x00 "ISc(" 0x28635349, +0x04 version 0x01000004, +0x0c descriptor offset (0x200)
//!   descriptor +0x0c file table offset, +0x1c directory count, +0x28 file count
//!   file table: u32 offsets (from the table start) of the directory names, then of the
//!   0x3a-byte file records: +0x00 name offset, +0x04 directory, +0x08 flags (4 compressed,
//!   8 invalid), +0x0a expanded size, +0x0e compressed size, +0x26 data offset.
//!   Compressed data = repeated [u16 length][raw deflate block].

const SIGNATURE: u32 = 0x2863_5349;
const FLAG_COMPRESSED: u16 = 4;
const FLAG_INVALID: u16 = 8;
const FLAG_OBFUSCATED: u16 = 2;

pub struct CabFile {
    pub name: String,
    pub dir: usize,
    pub flags: u16,
    pub size: u64,
    csize: u64,
    offset: u64,
}

impl CabFile {
    pub fn valid(&self) -> bool {
        self.flags & FLAG_INVALID == 0
    }
}

pub struct Cab<'a> {
    data: &'a [u8],
    /// install-relative directory paths, '\\' separated ("" is the install root)
    pub dirs: Vec<String>,
    pub files: Vec<CabFile>,
}

fn u32_at(d: &[u8], at: usize) -> Result<u32, String> {
    d.get(at..at + 4).map(|b| u32::from_le_bytes(b.try_into().unwrap())).ok_or_else(|| "cab header is truncated".into())
}

fn cstr(d: &[u8], at: usize) -> Result<String, String> {
    let s = d.get(at..).ok_or("cab name offset out of range")?;
    let end = s.iter().position(|&b| b == 0).ok_or("cab name is not terminated")?;
    Ok(String::from_utf8_lossy(&s[..end]).into_owned())
}

impl<'a> Cab<'a> {
    pub fn parse(data: &'a [u8]) -> Result<Cab<'a>, String> {
        if u32_at(data, 0)? != SIGNATURE {
            return Err("not an InstallShield cabinet".into());
        }
        let version = u32_at(data, 4)?;
        if version >> 24 != 1 {
            return Err(format!("InstallShield cabinet version {version:#x} is not supported (only 5.x)"));
        }
        let desc = u32_at(data, 0x0c)? as usize;
        let table = desc + u32_at(data, desc + 0x0c)? as usize;
        let ndirs = u32_at(data, desc + 0x1c)? as usize;
        let nfiles = u32_at(data, desc + 0x28)? as usize;
        let entry = |i: usize| -> Result<usize, String> { Ok(table + u32_at(data, table + 4 * i)? as usize) };
        let dirs = (0..ndirs).map(|i| cstr(data, entry(i)?)).collect::<Result<Vec<_>, _>>()?;
        let mut files = Vec::with_capacity(nfiles);
        for i in 0..nfiles {
            let r = entry(ndirs + i)?;
            let flags = data.get(r + 8..r + 10).map(|b| u16::from_le_bytes([b[0], b[1]])).ok_or("cab record truncated")?;
            let dir = u32_at(data, r + 4)? as usize;
            if flags & FLAG_INVALID != 0 {
                // unused slots carry garbage name offsets
                files.push(CabFile { name: String::new(), dir: 0, flags, size: 0, csize: 0, offset: 0 });
                continue;
            }
            if dir >= dirs.len() {
                return Err(format!("cab file {i}: directory {dir} out of range"));
            }
            files.push(CabFile {
                name: cstr(data, table + u32_at(data, r)? as usize)?,
                dir,
                flags,
                size: u32_at(data, r + 0x0a)? as u64,
                csize: u32_at(data, r + 0x0e)? as u64,
                offset: u32_at(data, r + 0x26)? as u64,
            });
        }
        Ok(Cab { data, dirs, files })
    }

    /// The file's install-relative path, '\\' separated.
    pub fn path(&self, f: &CabFile) -> String {
        match self.dirs[f.dir].as_str() {
            "" => f.name.clone(),
            d => format!("{d}\\{}", f.name),
        }
    }

    pub fn extract(&self, f: &CabFile) -> Result<Vec<u8>, String> {
        let name = self.path(f);
        if f.flags & FLAG_OBFUSCATED != 0 {
            return Err(format!("{name}: obfuscated cab data is not supported"));
        }
        let (a, b) = (f.offset as usize, (f.offset + f.csize) as usize);
        let src = self.data.get(a..b).ok_or_else(|| format!("{name}: data lies outside the cab (split cabinets are not supported)"))?;
        if f.flags & FLAG_COMPRESSED == 0 {
            return Ok(src.get(..f.size as usize).ok_or_else(|| format!("{name}: stored data is short"))?.to_vec());
        }
        let mut out = Vec::with_capacity(f.size as usize);
        let mut p = 0;
        while p < src.len() {
            let n = src.get(p..p + 2).map(|b| u16::from_le_bytes([b[0], b[1]]) as usize).ok_or_else(|| format!("{name}: truncated block"))?;
            let block = src.get(p + 2..p + 2 + n).ok_or_else(|| format!("{name}: truncated block"))?;
            out.extend(miniz_oxide::inflate::decompress_to_vec(block).map_err(|e| format!("{name}: bad deflate data ({e:?})"))?);
            p += 2 + n;
        }
        if out.len() as u64 != f.size {
            return Err(format!("{name}: unpacked {} bytes, expected {}", out.len(), f.size));
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A one-file cab built by hand: descriptor at 0x40, table at 0x80.
    fn tiny(payload: &[u8], flags: u16, stored: &[u8]) -> Vec<u8> {
        let mut d = vec![0u8; 0x200];
        d[0..4].copy_from_slice(&SIGNATURE.to_le_bytes());
        d[4..8].copy_from_slice(&0x0100_0004u32.to_le_bytes());
        d[0x0c..0x10].copy_from_slice(&0x40u32.to_le_bytes());
        d[0x4c..0x50].copy_from_slice(&0x40u32.to_le_bytes()); // table = 0x40 + 0x40
        d[0x5c..0x60].copy_from_slice(&1u32.to_le_bytes()); // 1 dir
        d[0x68..0x6c].copy_from_slice(&1u32.to_le_bytes()); // 1 file
        d[0x80..0x84].copy_from_slice(&0x10u32.to_le_bytes()); // dir name at table + 0x10
        d[0x84..0x88].copy_from_slice(&0x20u32.to_le_bytes()); // record at table + 0x20
        d[0x90..0x94].copy_from_slice(b"Data");
        let r = 0xa0;
        d[r..r + 4].copy_from_slice(&0x60u32.to_le_bytes()); // name at table + 0x60
        d[r + 8..r + 10].copy_from_slice(&flags.to_le_bytes());
        d[r + 0x0a..r + 0x0e].copy_from_slice(&(payload.len() as u32).to_le_bytes());
        d[r + 0x0e..r + 0x12].copy_from_slice(&(stored.len() as u32).to_le_bytes());
        d[r + 0x26..r + 0x2a].copy_from_slice(&0x200u32.to_le_bytes());
        d[0xe0..0xe5].copy_from_slice(b"a.txt");
        d.extend_from_slice(stored);
        d
    }

    #[test]
    fn compressed_blocks() {
        let text = b"motorhead motorhead motorhead";
        let mut stored = Vec::new();
        for half in [&text[..10], &text[10..]] {
            let z = miniz_oxide::deflate::compress_to_vec(half, 6);
            stored.extend((z.len() as u16).to_le_bytes());
            stored.extend(z);
        }
        let d = tiny(text, FLAG_COMPRESSED, &stored);
        let cab = Cab::parse(&d).unwrap();
        assert_eq!(cab.path(&cab.files[0]), "Data\\a.txt");
        assert_eq!(cab.extract(&cab.files[0]).unwrap(), text);
    }

    #[test]
    fn stored_and_errors() {
        let d = tiny(b"abc", 0, b"abc");
        let cab = Cab::parse(&d).unwrap();
        assert_eq!(cab.extract(&cab.files[0]).unwrap(), b"abc");
        assert!(Cab::parse(&d[..0x40]).is_err());
        assert!(Cab::parse(b"MSCF....").is_err());
    }
}
