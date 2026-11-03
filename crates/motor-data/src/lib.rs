//! Loaders for Motorhead (Digital Illusions, 1998) data files.
//!
//! Every format here is specified in `re-notes/05-formats.md`; the Python tools in
//! `re-notes/tools/` are the reference implementation these loaders are tested against.
//! Nothing in this crate contains game data: callers pass bytes read from a user's install.

pub mod aidat;
pub mod bytes;
pub mod cfg;
pub mod col;
pub mod fnt;
pub mod fs;
pub mod iff;
pub mod lob;
pub mod mde;
pub mod myo;
pub mod myw;
pub mod pak;
pub mod pdi;
pub mod smk;
pub mod spr;
pub mod sprpos;
pub mod tbl;
pub mod tga;

pub use bytes::{Error, Result};

/// 64-bit FNV-1a, used by `motordump` for golden comparisons.
#[derive(Clone, Copy)]
pub struct Fnv(u64);

impl Default for Fnv {
    fn default() -> Self {
        Fnv(0xcbf2_9ce4_8422_2325)
    }
}

impl Fnv {
    pub fn update(&mut self, data: &[u8]) {
        for &b in data {
            self.0 ^= b as u64;
            self.0 = self.0.wrapping_mul(0x0100_0000_01b3);
        }
    }
    pub fn finish(self) -> u64 {
        self.0
    }
    pub fn of(data: &[u8]) -> u64 {
        let mut h = Fnv::default();
        h.update(data);
        h.finish()
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn fnv1a_vectors() {
        assert_eq!(super::Fnv::of(b""), 0xcbf2_9ce4_8422_2325);
        assert_eq!(super::Fnv::of(b"a"), 0xaf63_dc4c_8601_ec8c);
    }
}
