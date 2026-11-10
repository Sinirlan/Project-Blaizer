//! Menu positions (`Data/sprpos.dta`), loaded by 0x4611ad into 0x750660.
//!
//! 32 sets (one per front-end screen) of 32 slots of 8 `i32`. The accessors 0x460794 / 0x4607d0
//! return x (+0x0) and y (+0x8) of `slots[set][slot]`, blended toward x (+0x4) / y (+0xc) by the
//! slide factor `[0x75066c]` / 256 (`slide_factor`: 256 on a screen's first frame, 0 once it has
//! settled 150 ms later; the same going back up while it leaves). The text drawers add code
//! offsets to the slot positions; some slots hold an offset in a field the name does not suggest
//! (the single summary reads its value column from slot 0x16's *y*). Spec: re-notes/05-formats.md.

use crate::bytes::{err, slice, Result};

pub const SETS: usize = 32;
pub const SLOTS: usize = 32;
const FIELDS: usize = 8;
/// `[0x4bdd58]`: how long a screen takes to slide in (and out), ms [O].
pub const SLIDE_MS: u32 = 150;

/// CLASSIC 0x4627e0: the slide factor `[0x75066c]` of a screen whose clock (descriptor `+0`) reads `ms`:
/// `256 − 256 · min(1, ms / 150)`, truncated (0x4914be is frndint in chop mode) [O].
pub fn slide_factor(ms: u32) -> i32 {
    (256.0 - 256.0 * (ms as f64 / SLIDE_MS as f64).min(1.0)).trunc() as i32
}

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

    /// 0x46082c / 0x460864: the slot's slide effect (`+0x14`, 0..8) and its value (`+0x1c`), read by
    /// the per-slot drawer 0x460880 while the screen slides [O].
    pub fn effect(&self, set: usize, slot: usize) -> (i32, i32) {
        let s = self.slot(set, slot);
        (s[5], s[7])
    }

    /// The slot position `slide`/256 of the way from its resting (x, y) to its (+4, +0xc) one.
    pub fn at(&self, set: usize, slot: usize, slide: i32) -> (i32, i32) {
        let s = self.slot(set, slot);
        let mix = |a: i32, b: i32| a + (((b - a) * slide) >> 8);
        (mix(s[0], s[1]), mix(s[2], s[3]))
    }

    /// The table as the accessors 0x460794 / 0x4607d0 see it at slide factor `slide` (`slide_factor`).
    pub fn slid(&self, slide: i32) -> Slid<'_> {
        Slid { sp: self, slide }
    }
}

/// `SprPos` at one slide factor: `x` / `y` are the positions the original draws at that instant.
#[derive(Debug, Clone, Copy)]
pub struct Slid<'a> {
    sp: &'a SprPos,
    slide: i32,
}

impl Slid<'_> {
    /// 0x460794
    pub fn x(&self, set: usize, slot: usize) -> i32 {
        self.sp.at(set, slot, self.slide).0
    }

    /// 0x4607d0
    pub fn y(&self, set: usize, slot: usize) -> i32 {
        self.sp.at(set, slot, self.slide).1
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
        assert_eq!((p.slid(128).x(10, 20), p.slid(128).y(10, 20)), (208, -90));
        assert_eq!(p.effect(10, 20), (0, 0));
        assert!(SprPos::parse(&d[1..]).is_err());
    }

    #[test]
    fn slide_clock() {
        assert_eq!(slide_factor(0), 256);
        assert_eq!(slide_factor(1), 254); // 256 - 1.71, chopped
        assert_eq!(slide_factor(75), 128);
        assert_eq!(slide_factor(149), 1);
        assert_eq!(slide_factor(150), 0);
        assert_eq!(slide_factor(167), 0);
    }

    #[test]
    fn install_table() {
        let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../installed_game/Data/sprpos.dta");
        let Ok(d) = std::fs::read(p) else { return };
        let t = SprPos::parse(&d).unwrap();
        // the single summary: set 10, base slots 10 + 20, row pitch slot 0x15, value column slot 0x16
        assert_eq!((t.x(10, 10) + t.x(10, 20), t.y(10, 10) + t.y(10, 20)), (128, 334));
        assert_eq!((t.x(10, 0x15), t.y(10, 0x15), t.x(10, 0x16), t.y(10, 0x16)), (197, 19, 72, 111));
        // Main: dots spread from the top, the box turns over, the ring spreads, the title jitters
        assert_eq!([t.effect(0, 1), t.effect(0, 2), t.effect(0, 3), t.effect(0, 5)], [(7, 2048), (5, 0), (8, 1280), (3, 16384)]);
    }
}
