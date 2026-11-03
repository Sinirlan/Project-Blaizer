//! Menu positions (`Data/sprpos.dta`), loaded by 0x4611ad into 0x750660.
//!
//! 32 sets (one per front-end screen) of 32 slots of 8 `i32`. The accessors 0x460794 / 0x4607d0
//! return x (+0x0) and y (+0x8) of `slots[set][slot]`, blended toward x (+0x4) / y (+0xc) by the
//! slide-in factor `[0x75066c]` / 256 (0 once a screen has settled). The text drawers add code
//! offsets to the slot positions; some slots hold an offset in a field the name does not suggest
//! (the single summary reads its value column from slot 0x16's *y*). Spec: re-notes/05-formats.md.

use crate::bytes::{err, slice, Result};

pub const SETS: usize = 32;
pub const SLOTS: usize = 32;
const FIELDS: usize = 8;

#[derive(Debug, Clone)]
pub struct SprPos {
    v: Vec<i32>,
}

impl Default for SprPos {
    /// An empty table (every slot at 0, 0): the text lands in the corner, but nothing panics.
    fn default() -> Self {
        SprPos { v: vec![0; SETS * SLOTS * FIELDS] }
    }
}

impl SprPos {
    pub fn parse(d: &[u8]) -> Result<SprPos> {
        if d.len() != SETS * SLOTS * FIELDS * 4 {
            return err(format!("sprpos.dta is {} bytes, expected {}", d.len(), SETS * SLOTS * FIELDS * 4));
        }
        let v = slice(d, 0, d.len())?.as_chunks::<4>().0.iter().map(|c| i32::from_le_bytes(*c)).collect();
        Ok(SprPos { v })
    }

    /// The eight fields of one slot.
    pub fn slot(&self, set: usize, slot: usize) -> [i32; FIELDS] {
        let o = (set % SETS * SLOTS + slot % SLOTS) * FIELDS;
        std::array::from_fn(|i| self.v[o + i])
    }

    /// 0x460794: x of a slot at rest.
    pub fn x(&self, set: usize, slot: usize) -> i32 {
        self.slot(set, slot)[0]
    }

    /// 0x4607d0: y of a slot at rest.
    pub fn y(&self, set: usize, slot: usize) -> i32 {
        self.slot(set, slot)[2]
    }

    /// The slot position `slide`/256 of the way from its resting (x, y) to its (+4, +0xc) one.
    pub fn at(&self, set: usize, slot: usize, slide: i32) -> (i32, i32) {
        let s = self.slot(set, slot);
        let mix = |a: i32, b: i32| a + (((b - a) * slide) >> 8);
        (mix(s[0], s[1]), mix(s[2], s[3]))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slots_and_slide() {
        let mut d = vec![0u8; SETS * SLOTS * FIELDS * 4];
        let o = (10 * SLOTS + 20) * FIELDS * 4;
        for (i, v) in [80i32, 336, 38, -218].into_iter().enumerate() {
            d[o + i * 4..o + i * 4 + 4].copy_from_slice(&v.to_le_bytes());
        }
        let p = SprPos::parse(&d).unwrap();
        assert_eq!((p.x(10, 20), p.y(10, 20)), (80, 38));
        assert_eq!(p.at(10, 20, 0), (80, 38));
        assert_eq!(p.at(10, 20, 256), (336, -218));
        assert_eq!(p.at(10, 20, 128), (208, -90));
        assert!(SprPos::parse(&d[1..]).is_err());
    }

    #[test]
    fn install_table() {
        let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../installed_game/Data/sprpos.dta");
        let Ok(d) = std::fs::read(p) else { return };
        let t = SprPos::parse(&d).unwrap();
        // the single summary: set 10, base slots 10 + 20, row pitch slot 0x15, value column slot 0x16
        assert_eq!((t.x(10, 10) + t.x(10, 20), t.y(10, 10) + t.y(10, 20)), (128, 334));
        assert_eq!((t.x(10, 0x15), t.y(10, 0x15), t.x(10, 0x16), t.y(10, 0x16)), (197, 19, 72, 111));
    }
}
