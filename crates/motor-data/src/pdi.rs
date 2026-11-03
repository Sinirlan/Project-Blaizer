//! PDI1 texture packs (Tracks/trackN/Packed/tex1.pdi, tex2.pdi): an archive of IFF ILBM
//! files. Loader 0x40fec4.

use crate::bytes::{cstr, err, i16_, slice, u16_, u32_, Result};

#[derive(Debug, Clone)]
pub struct Entry {
    pub kind: u16,
    pub prev: i16,
    pub next: i16,
    pub name: String,
    /// relative to the data section
    pub offset: usize,
    pub size: usize,
}

#[derive(Debug, Clone)]
pub struct Pack<'a> {
    pub page_size: u32,
    pub entries: Vec<Entry>,
    data: &'a [u8],
}

impl<'a> Pack<'a> {
    /// The raw IFF file of an entry.
    pub fn bytes(&self, e: &Entry) -> Result<&'a [u8]> {
        slice(self.data, e.offset, e.size)
    }
    pub fn find(&self, name: &str) -> Option<&Entry> {
        self.entries.iter().find(|e| e.name.eq_ignore_ascii_case(name))
    }
}

pub fn parse(d: &[u8]) -> Result<Pack<'_>> {
    if d.get(..4) != Some(b"PDI1") {
        return err("not a PDI1 pack");
    }
    let count = u32_(d, 8)? as usize;
    let dir = u32_(d, 16)? as usize;
    let (data_off, data_size) = (u32_(d, 20)? as usize, u32_(d, 24)? as usize);
    let entries = (0..count)
        .map(|i| {
            let p = dir + 52 * i;
            Ok(Entry {
                kind: u16_(d, p)?,
                prev: i16_(d, p + 2)?,
                next: i16_(d, p + 4)?,
                offset: u32_(d, p + 12)? as usize,
                size: u32_(d, p + 16)? as usize,
                name: cstr(d, p + 20, Some(32))?,
            })
        })
        .collect::<Result<_>>()?;
    Ok(Pack { page_size: u32_(d, 4)?, entries, data: slice(d, data_off, data_size)? })
}
