//! Menu positions (`Data/sprpos.dta`), loaded by 0x4611ad into 0x750660.
//!
//! 32 sets (one per front-end screen) of 32 slots of 8 `i32`. The accessors 0x460794 / 0x4607d0
//! return x (+0x0) and y (+0x8) of `slots[set][slot]`, blended toward x (+0x4) / y (+0xc) by the
//! slide factor `[0x75066c]` / 256 (`slide_factor`: 256 on a screen's first frame, 0 once it has
//! settled 150 ms later; the same going back up while it leaves). The text drawers add code
//! offsets to the slot positions; some slots hold an offset in a field the name does not suggest
//! (the single summary reads its value column from slot 0x16's *y*). Spec: re-notes/05-formats.md.
//!
//! A patch 3.0 file (made for the 3.0 rows we do not have) is put back to the 1998 layout on load (`SprPos::parse`, #502).

use crate::bytes::{err, slice, Result};

pub const SETS: usize = 32;
pub const SLOTS: usize = 32;
const FIELDS: usize = 8;
/// `[0x4bdd58]`: how long a screen takes to slide in (and out), ms [O].
pub const SLIDE_MS: u32 = 150;

/// The 1998 values (set, slot, the eight fields) of the 29 slots patch 3.0's `sprpos.dta` moves, in sets 5 (Graphics), 6 (Sound),
/// 8 (Controls), 9 (Multi) and 19 (Ghost); every other slot is identical in both files. Facts read from the 1998 install, listed
/// in re-notes/05-formats.md "Patch 3.0 front-end files" [O].
const RESTORE_1998: [(usize, usize, [i32; FIELDS]); 29] = [
    (5, 0x4, [74, 32, 82, 92, 0, 8, 0, -768]),
    (5, 0x18, [211, 0, 13, 0, 0, 0, 0, 0]),
    (6, 0x4, [6, 657, 85, 86, 0, 7, 0, 768]),
    (6, 0x5, [251, -160, 91, 92, 0, 8, 0, 2048]),
    (6, 0x6, [171, 171, 86, -22, 0, 7, 0, -976]),
    (6, 0x7, [419, 359, 91, 84, 0, 0, 0, 0]),
    (6, 0x8, [132, -127, 104, 318, 0, 5, 0, 0]),
    (6, 0x9, [322, 653, 104, -43, 0, 5, 0, 0]),
    (6, 0xa, [253, 254, 204, -19, 0, 1, 0, 1024]),
    (6, 0xf, [236, 236, 100, -55, 0, 1, 0, 1024]),
    (6, 0x10, [214, -107, 172, 173, 0, 1, 0, 1024]),
    (6, 0x11, [183, 183, 206, 408, 0, 1, 0, 1024]),
    (6, 0x18, [372, -34, 145, 146, 0, 1, 0, 0]),
    (6, 0x19, [182, -146, 145, 146, 0, 5, 0, 0]),
    (8, 0x4, [19, -249, 212, 212, 0, 6, 0, 4864]),
    (8, 0x18, [256, 647, 96, 96, 0, 0, 0, 0]),
    (8, 0x19, [384, -157, 96, 98, 0, 0, 0, 0]),
    (8, 0x1a, [0, 0, 0, 0, 0, 0, 0, 0]),
    (8, 0x1b, [0, 0, 0, 0, 0, 0, 0, 0]),
    (8, 0x1c, [0, 0, 0, 0, 0, 0, 0, 0]),
    (8, 0x1d, [9, 9, 0, 0, 0, 0, 0, 0]),
    (8, 0x1e, [117, 125, 27, 0, 0, 0, 0, 0]),
    (8, 0x1f, [75, -41, 347, 399, 0, 0, 0, 0]),
    (9, 0x0, [609, 488, 232, 278, 0, 0, 0, 0]),
    (9, 0x1f, [572, 614, 374, 404, 0, 0, 0, 0]),
    (19, 0x6, [225, -125, 101, 101, 0, 0, 0, 0]),
    (19, 0x8, [0, 0, 0, 0, 0, 0, 0, 0]),
    (19, 0xa, [0, 0, 0, 0, 0, 0, 0, 0]),
    (19, 0xc, [0, 0, 0, 0, 0, 0, 0, 0]),
];

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
        let mut t = SprPos { v };
        if t.is_patch30() {
            // design choice (#502): Classic keeps the 1998 layout on every install, the 3.0 rows are not ours
            for (set, slot, f) in RESTORE_1998 {
                let o = (set * SLOTS + slot) * FIELDS;
                t.v[o..o + FIELDS].copy_from_slice(&f);
            }
        }
        Ok(t)
    }

    /// Whether this is patch 3.0's table: Controls slot 0x1a (unused in 1998, all zero) holds a value and the Graphics row
    /// pitch (slot 0x18 y) is 12 instead of 13 [O]. Two markers, not a whole-table match, so a later 3.x file is caught too.
    fn is_patch30(&self) -> bool {
        self.slot(8, 0x1a) != [0; FIELDS] && self.slot(5, 0x18)[2] == 12
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

    /// The 3.0 values of the 29 moved slots (same order as `RESTORE_1998`).
    const PATCH30: [(usize, usize, [i32; FIELDS]); 29] = [
        (5, 0x4, [74, 32, 77, 92, 0, 8, 0, -768]),
        (5, 0x18, [211, 0, 12, 0, 0, 0, 0, 0]),
        (6, 0x4, [6, 657, 105, 86, 0, 7, 0, 768]),
        (6, 0x5, [251, -160, 111, 92, 0, 8, 0, 2048]),
        (6, 0x6, [171, 171, 106, -22, 0, 7, 0, -976]),
        (6, 0x7, [6, 120, 86, -14, 0, 0, 0, 0]),
        (6, 0x8, [132, -127, 124, 318, 0, 5, 0, 0]),
        (6, 0x9, [322, 653, 124, -43, 0, 5, 0, 0]),
        (6, 0xa, [253, 254, 224, -19, 0, 1, 0, 1024]),
        (6, 0xf, [236, 236, 120, -55, 0, 1, 0, 1024]),
        (6, 0x10, [214, -107, 192, 173, 0, 1, 0, 1024]),
        (6, 0x11, [183, 183, 226, 408, 0, 1, 0, 1024]),
        (6, 0x18, [372, -34, 165, 146, 0, 1, 0, 0]),
        (6, 0x19, [182, -146, 165, 146, 0, 5, 0, 0]),
        (8, 0x4, [19, -249, 204, 212, 0, 6, 0, 4864]),
        (8, 0x18, [214, 647, 93, 96, 0, 0, 0, 0]),
        (8, 0x19, [361, -157, 92, 98, 0, 0, 0, 0]),
        (8, 0x1a, [2, 0, 2, 0, 0, 0, 0, 0]),
        (8, 0x1b, [28, 0, 0, 0, 0, 0, 0, 0]),
        (8, 0x1c, [124, 0, -2, 0, 0, 0, 0, 0]),
        (8, 0x1d, [2, 9, 0, 0, 0, 0, 0, 0]),
        (8, 0x1e, [122, 125, 20, 0, 0, 0, 0, 0]),
        (8, 0x1f, [72, -41, 306, 399, 0, 0, 0, 0]),
        (9, 0x0, [140, 488, 108, 278, 0, 0, 0, 0]),
        (9, 0x1f, [201, 638, 114, 114, 0, 0, 0, 0]),
        (19, 0x6, [20, -230, 160, 160, 0, 0, 0, 0]),
        (19, 0x8, [38, 0, 16, 0, 0, 0, 0, 0]),
        (19, 0xa, [225, -80, 101, 0, 0, 0, 0, 0]),
        (19, 0xc, [225, -324, 350, 350, 0, 0, 0, 0]),
    ];

    fn put(d: &mut [u8], set: usize, slot: usize, f: [i32; FIELDS]) {
        let o = (set * SLOTS + slot) * FIELDS * 4;
        for (i, v) in f.into_iter().enumerate() {
            d[o + i * 4..o + i * 4 + 4].copy_from_slice(&v.to_le_bytes());
        }
    }

    #[test]
    fn patch30_table_restored() {
        // synthetic: only the two markers set, every other slot 0
        let mut d = vec![0u8; SETS * SLOTS * FIELDS * 4];
        assert_eq!(SprPos::parse(&d).unwrap().slot(5, 0x18)[2], 0, "no markers: left alone");
        put(&mut d, 8, 0x1a, [2, 0, 2, 0, 0, 0, 0, 0]);
        put(&mut d, 5, 0x18, [211, 0, 12, 0, 0, 0, 0, 0]);
        let t = SprPos::parse(&d).unwrap();
        assert_eq!(t.slot(8, 0x1a), [0; FIELDS]);
        assert_eq!(t.slot(5, 0x18), [211, 0, 13, 0, 0, 0, 0, 0]);
        // one marker alone is not a 3.0 table
        let mut d = vec![0u8; SETS * SLOTS * FIELDS * 4];
        put(&mut d, 5, 0x18, [211, 0, 12, 0, 0, 0, 0, 0]);
        assert_eq!(SprPos::parse(&d).unwrap().slot(5, 0x18)[2], 12);
    }

    #[test]
    fn patch30_install_table() {
        let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../installed_game/Data/sprpos.dta");
        let Ok(d) = std::fs::read(p) else { return };
        let raw = SprPos { v: d.as_chunks::<4>().0.iter().map(|c| i32::from_le_bytes(*c)).collect() };
        if raw.is_patch30() {
            return; // the install is itself patched: nothing to compare against
        }
        assert_eq!(SprPos::parse(&d).unwrap().v, raw.v, "an unpatched file is not touched");
        let mut d3 = d.clone();
        for (set, slot, f) in PATCH30 {
            put(&mut d3, set, slot, f);
        }
        let t3 = SprPos::parse(&d3).unwrap();
        assert_eq!(t3.v, raw.v, "a 3.0 table comes back as the 1998 one");
        // the restore list is exactly the slots that differ
        for (set, slot, f) in RESTORE_1998 {
            assert_eq!(raw.slot(set, slot), f);
        }
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
