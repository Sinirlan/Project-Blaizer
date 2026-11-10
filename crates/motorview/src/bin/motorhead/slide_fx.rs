//! CLASSIC 0x460880 while a screen slides (#263 phase 2): the per-slot sprite effects.
//!
//! The menu sprites are compiled code, one routine per row; the blitter 0x401113 runs them from a
//! row table (destination offset, source row), so every effect is a different row table: rows
//! shifted sideways, spread apart or squeezed together, never a horizontal scale. The slot's
//! `+0x14` (`SprPos::effect`) picks the effect, `+0x1c` is its value; at slide 0 the drawer does
//! the settled draw instead. Spec: re-notes/05-formats.md "sprpos.dta" (slide effects).

use crate::league::Rng;
use motorview::ui::Batch;

/// The menu's 8-bit buffer (set up at 0x4611ff): 960 bytes a row, the 640 x 400 view starting at
/// column 160 of row 120 (a guard band all round) [O]. The row-table blitter clips only by a row's
/// start address, so a row shifted far sideways runs on into the neighbouring scanline.
const PITCH: i32 = 960;
const VIEW_W: i32 = 640;
const VIEW_H: i32 = 400;
const GUARD_X: i32 = 160;

/// One entry of the row table: source row `src` drawn `dy` rows below and `dx` pixels right of the
/// sprite's position.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Row {
    pub dy: i32,
    pub dx: i32,
    pub src: u32,
}

/// The 4096-step sine table at 0x4a7df0 (equal to `sin` within 1e-7) [O].
fn sin_tab(i: i32) -> f32 {
    ((i & 0xfff) as f32 * std::f32::consts::TAU / 4096.0).sin()
}

/// x87 `frndint` in chop mode (0x4914be) then `fistp`.
fn chop(v: f32) -> i32 {
    v.trunc() as i32
}

/// CLASSIC 0x40a348: source row i at `i * f` (rows spread from the top, gaps between them when f > 1).
fn spread_top(f: f32, h: u32) -> Vec<Row> {
    let k = chop(f * 1024.0) as u32;
    let edi = k.wrapping_mul(h.wrapping_sub(1)) >> 10;
    let div = h.saturating_sub(1).max(1); // h = 1 divides by zero in the original
    (0..h).map(|i| Row { dy: (edi.wrapping_mul(i) / div) as i32, dx: 0, src: i }).collect()
}

/// CLASSIC 0x40a41c: rows spread (f > 1), squeezed (0 < f < 1) or turned over (f < 0) about the
/// middle row.
fn spread_mid(f: f32, h: u32) -> Vec<Row> {
    let h = h as i32;
    let k = chop(-f * 1024.0);
    let div = (h - 1).max(1);
    let ebp = k.wrapping_mul(h - 1) >> 10;
    let half = h / 2;
    (0..h).map(|i| Row { dy: (half - i).wrapping_mul(ebp) / div + half, dx: 0, src: i as u32 }).collect()
}

/// CLASSIC 0x40a518: the sprite resampled to `(h - 1) * f` rows from the top (for f < 1 after the
/// 0x40a348 pass).
fn stretch_top(f: f32, h: u32) -> Vec<Vec<Row>> {
    let mut passes = Vec::new();
    if f < 1.0 {
        passes.push(spread_top(f, h));
    }
    let k = chop(f * 1024.0) as u32;
    let n = h.wrapping_sub(1).wrapping_mul(k) >> 10;
    passes.push((0..n.min(4096)).map(|j| Row { dy: j as i32, dx: 0, src: j.wrapping_mul(h) / n }).collect());
    passes
}

/// CLASSIC 0x40a64c: every row shifted sideways by `((g & 0x1fff) - bias) * amp >> 16` pixels, `g` a
/// running product of the global seed 0x4addf4, the row and a multiplier that starts at the slot
/// number and grows by 0x6c9 a row.
fn jitter(amp: i32, bias: i32, slot: usize, h: u32, seed: &mut u32) -> Vec<Row> {
    let mut s = slot as u32;
    (0..h)
        .map(|i| {
            *seed = seed.wrapping_add(i).wrapping_mul(s);
            s = s.wrapping_add(0x6c9);
            let dx = ((*seed & 0x1fff) as i32 - bias).wrapping_mul(amp) >> 16;
            Row { dy: i as i32, dx, src: i }
        })
        .collect()
}

/// CLASSIC 0x460880, slide != 0: the row tables slot `slot` draws its sprite (height `h`) with, one
/// table per blit. Effect 0 is the plain draw; 1 / 3 jitter right and left, 2 only to the right;
/// 4 opens from a line (`sin` of 0x400 + 4 slide), 5 turns over one and a half times (0x400 + 12 slide,
/// blitted twice); 6 / 7 / 8 stretch, spread from the top, spread about the middle by
/// `1 + value * slide / 256000`. Any other effect draws nothing while sliding [O].
pub fn passes(effect: i32, value: i32, slide: i32, slot: usize, h: u32, rng: &mut Rng, seed: &mut u32) -> Vec<Vec<Row>> {
    let plain = || (0..h).map(|i| Row { dy: i as i32, dx: 0, src: i }).collect::<Vec<_>>();
    let factor = || 1.0 + (slide as f64 * value as f64 * (1.0 / 256000.0)) as f32;
    match effect {
        0 => vec![plain()],
        1..=3 => {
            let amp = rng.next() as i32 % ((value.wrapping_mul(slide) >> 8).max(1));
            let bias = if effect == 2 { -0x1000 } else { 0x1000 };
            vec![jitter(amp, bias, slot, h, seed)]
        }
        4 => vec![spread_mid(sin_tab(0x400 + 4 * slide), h)],
        5 => {
            let rows = spread_mid(sin_tab(0x400 + 12 * slide), h);
            vec![rows.clone(), rows]
        }
        6 => stretch_top(factor(), h),
        7 => vec![spread_top(factor(), h)],
        8 => vec![spread_mid(factor(), h)],
        _ => Vec::new(),
    }
}

/// What the settled drawer (0x460880, slide 0) does with a slot this frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Settled {
    /// The plain draw 0x40a174.
    Plain,
    /// 0x40a628 with the sine step `0x400 + counter[2]`: the title / box turning over.
    Turn(i32),
    /// 0x40a64c with amp `(rand & 0x3f) + 1`, bias 0x1000 and the row seed at the start of the draw.
    Glitch { amp: i32, seed: u32 },
}

/// CLASSIC 0x460880, slide 0: the per-slot counters `0x750770` and flags `0x7507f0` that every screen
/// init clears (flags: the title and SCREEN box slots) and the drawer steps once per drawn frame [O].
/// Ours steps a slot once per menu tick however often the screen is redrawn (`frame`).
#[derive(Debug, Clone)]
pub struct Settle {
    counter: [i32; 32],
    flag: [bool; 32],
    /// The tick a slot was last stepped in, and what it drew then.
    last: [Option<(u64, Settled)>; 32],
    rng: Rng,
    /// The global row seed 0x4addf4.
    seed: u32,
}

impl Settle {
    /// A screen init: counters and flags cleared; `seed` starts rand and the row seed (the original's run
    /// on from draw to draw, ours restart so a `--shot` repeats).
    pub fn new(seed: u32) -> Self {
        let mut rng = Rng(seed);
        let seed = rng.next() << 15 | rng.next();
        Self { counter: [0; 32], flag: [false; 32], last: [None; 32], rng, seed }
    }

    /// The slots a screen init flags to turn over now and then (0x7507f2 and 0x7507f3, 4 or 5).
    pub fn flag(&mut self, slots: &[usize]) {
        self.flag = [false; 32];
        for &s in slots.iter().filter(|&&s| s < 32) {
            self.flag[s] = true;
        }
    }

    /// The draw of `slot` in menu tick `tick` (the sprite has `h` rows). The first draw of a tick steps the
    /// counter and rolls the dice, a repeat in the same tick draws the same again.
    pub fn frame(&mut self, slot: usize, tick: u64, h: u32) -> Settled {
        if slot >= 32 {
            return Settled::Plain;
        }
        if let Some((t, s)) = self.last[slot] {
            if t == tick {
                return s;
            }
        }
        let s = self.step(slot, h);
        self.last[slot] = Some((tick, s));
        s
    }

    /// CLASSIC 0x460880 from the first rand().
    fn step(&mut self, slot: usize, h: u32) -> Settled {
        let c = self.counter[slot];
        // a turn over runs while the counter is above 0x14 (0x1000 down by 0x100 a frame) and starts at
        // random from rest (rand() < 10, 0x4608ac); only a flagged slot does it, an unflagged one draws plain
        let turn = |me: &mut Self| {
            if !me.flag[slot] {
                return Settled::Plain;
            }
            let angle = 0x400 + me.counter[2];
            me.counter[slot] = if c > 0 { c - 0x100 } else { 0x1000 };
            Settled::Turn(angle)
        };
        if self.rng.next() < 10 || c > 0x14 {
            return turn(self);
        }
        // the glitch: starts at random (rand & 0x7ff < 10) and runs while the counter counts down from rand % 20
        if self.rng.next() & 0x7ff >= 10 && c <= 0 {
            return Settled::Plain;
        }
        let amp = (self.rng.next() & 0x3f) as i32 + 1;
        self.counter[slot] = if c > 0 { c - 1 } else { (self.rng.next() % 20) as i32 };
        let seed = self.seed;
        jitter(amp, 0x1000, slot, h, &mut self.seed); // the draw moves the global seed on
        Settled::Glitch { amp, seed }
    }
}

/// The row tables a settled draw of `slot` (height `h`) blits; none for `Plain` (the caller draws it).
pub fn settled_passes(s: Settled, slot: usize, h: u32) -> Vec<Vec<Row>> {
    match s {
        Settled::Plain => Vec::new(),
        Settled::Turn(angle) => vec![spread_mid(sin_tab(angle), h)],
        Settled::Glitch { amp, mut seed } => vec![jitter(amp, 0x1000, slot, h, &mut seed)],
    }
}

/// Blit row tables of the atlas image `name` (`w` x `h`) at (x, y) the way 0x401113 does: a row whose
/// start lies before the view is skipped, one at or past its end stops the table; a row running past
/// the buffer's 960-byte pitch carries on at the start of the next one. Only the 640 x 400 view shows.
pub fn blit(b: &mut Batch, name: &str, x: i32, y: i32, w: i32, passes: &[Vec<Row>]) {
    for rows in passes {
        for r in rows {
            let start = (y + r.dy) * PITCH + x + r.dx;
            if start >= VIEW_H * PITCH {
                break;
            }
            if start < 0 {
                continue;
            }
            // the row's bytes from `start` on, cut where they cross into the next buffer row
            let (mut a, mut sx) = (start, 0);
            while sx < w {
                let (row, col) = ((a + GUARD_X).div_euclid(PITCH), (a + GUARD_X).rem_euclid(PITCH) - GUARD_X);
                let n = (w - sx).min(PITCH - GUARD_X - col);
                let (l, rgt) = (col.max(0), (col + n).min(VIEW_W));
                if rgt > l && (0..VIEW_H).contains(&row) {
                    let s0 = (sx + l - col) as f32;
                    b.image_sub(name, s0, r.src as f32, (rgt - l) as f32, 1.0, l as f32, row as f32);
                }
                sx += n;
                a += n;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_and_unknown() {
        let (mut rng, mut seed) = (Rng(1), 0);
        let p = passes(0, 0, 100, 3, 4, &mut rng, &mut seed);
        assert_eq!(p, vec![(0..4).map(|i| Row { dy: i, dx: 0, src: i as u32 }).collect::<Vec<_>>()]);
        assert!(passes(-1, 0, 100, 3, 4, &mut rng, &mut seed).is_empty());
    }

    #[test]
    fn spread_from_top() {
        // dots on Main: 1 + 2048 * 256 / 256000 = 3.048: rows 3 apart
        let p = passes(7, 2048, 256, 1, 4, &mut Rng(1), &mut 0);
        assert_eq!(p[0].iter().map(|r| r.dy).collect::<Vec<_>>(), [0, 3, 6, 9]);
    }

    #[test]
    fn turn_over() {
        // effect 5 at slide 128: sin(0x400 + 0x600) = sin(0xa00) = -0.707, so the sprite is upside down
        let p = passes(5, 0, 128, 2, 5, &mut Rng(1), &mut 0);
        assert_eq!(p.len(), 2);
        let dy: Vec<i32> = p[0].iter().map(|r| r.dy).collect();
        assert!(dy[0] > dy[4], "{dy:?}");
        // effect 4 at full slide is flat: every row on the middle one
        let p = passes(4, 0, 256, 2, 5, &mut Rng(1), &mut 0);
        assert!(p[0].iter().all(|r| r.dy == 2), "{:?}", p[0]);
    }

    #[test]
    fn stretch_resamples() {
        // f = 1 + 4864 * 256 / 256000 = 5.864: (h - 1) * f rows, each a source row
        let p = passes(6, 4864, 256, 4, 10, &mut Rng(1), &mut 0);
        assert_eq!(p.len(), 1);
        assert_eq!(p[0].len(), 52);
        assert_eq!(p[0].last().unwrap().src, 9);
    }

    #[test]
    fn jitter_bounds() {
        let (mut rng, mut seed) = (Rng(7), 12345);
        for _ in 0..50 {
            for r in &passes(1, 1024, 256, 6, 20, &mut rng, &mut seed)[0] {
                assert!((-64..64).contains(&r.dx), "{r:?}");
            }
            for r in &passes(2, 1024, 256, 6, 20, &mut rng, &mut seed)[0] {
                assert!((0..192).contains(&r.dx), "{r:?}");
            }
        }
        // a negative value never shifts
        assert!(passes(1, -992, 256, 4, 20, &mut rng, &mut seed)[0].iter().all(|r| r.dx == 0));
    }
}

#[cfg(test)]
mod settle_tests {
    use super::*;

    #[test]
    fn turn_runs_sixteen_frames_and_rests() {
        let mut st = Settle::new(1);
        st.flag(&[2]);
        st.counter[2] = 0x1000; // a turn over just started
        // the counter steps 0x1000 .. 0x100 (16 draws), the angle is read before each step
        for t in 0..16 {
            assert_eq!(st.frame(2, t, 8), Settled::Turn(0x400 + 0x1000 - 0x100 * t as i32), "{t}");
        }
        assert_eq!(st.counter[2], 0);
    }

    #[test]
    fn same_tick_draws_the_same() {
        let mut st = Settle::new(5);
        st.counter[4] = 7;
        let a = st.frame(4, 9, 20);
        assert_eq!(st.frame(4, 9, 20), a);
        assert_eq!(st.counter[4], 6);
    }

    #[test]
    fn unflagged_never_turns_and_glitches_end() {
        let mut st = Settle::new(3);
        let (mut glitch, mut turn) = (0, 0);
        for t in 0..200_000 {
            match st.frame(5, t, 10) {
                Settled::Turn(_) => turn += 1,
                Settled::Glitch { amp, .. } => {
                    glitch += 1;
                    assert!((1..=64).contains(&amp));
                }
                Settled::Plain => {}
            }
        }
        assert_eq!(turn, 0);
        // a glitch starts 0.49 % of the frames at rest and lasts 1 + rand % 20 (mean 10.5): about 5 % of the time
        assert!((7_000..13_000).contains(&glitch), "{glitch}");
    }

    #[test]
    fn flagged_turns_once_in_a_while() {
        let mut st = Settle::new(11);
        st.flag(&[2]);
        let turns = (0..600_000).filter(|&t| matches!(st.frame(2, t, 10), Settled::Turn(_))).count();
        // rest -> start 10 / 32768 a frame, then 16 frames: about 600000 * 3e-4 * 16 = 3000 turn frames
        assert!((800..9000).contains(&turns), "{turns}");
    }
}
