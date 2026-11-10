//! Map easter eggs of the original (#366): the RuhrStadt club message and the Atlantika roof scroller.
//!
//! CLASSIC: both are tested inside the minimap routine `0x412c0c` (which runs only while RoadMap is not None,
//! `0x4135c2`) on the followed car's position, one frame at a time, and drawn by DrawHud `0x413008` (`0x41353a`).
//! The module holds the state and the per-tick rules; `hud.rs` draws it. Nothing is saved.
//! The original counts frames; the tick rate here is `EGG_TICK_HZ` (design choice).
use glam::Vec3;

motor_tune::tunable! {
    /// Ticks per second of the map easter eggs (the club's 200-frame wait, the scroller's speed). The original counts
    /// frames and its traces run at 60 fps [O].
    /// range: 10..240
    /// unit: Hz
    /// applies: restart
    EGG_TICK_HZ: f32 = 60.0;
}

/// RuhrStadt club door (`0x4cfa83`, `0x4cfa87`): x, z of the circle's centre and its squared radius (`0x4cfa8b`, 20 m) [O].
pub const CLUB_CENTRE: (f32, f32) = (476.0, -188.0);
pub const CLUB_R2: f32 = 400.0;
/// The club message shows once the car has stayed inside for more than this many ticks (`0x412d95`) [O].
pub const CLUB_WAIT: u32 = 200;
/// Atlantika roof spot (`0x4cfa93`, `0x4cfa9b`, `0x4cfaa3`): the 3D centre and the squared radius (`0x4cfaab`, 2 m) [O].
pub const ROOF_CENTRE: Vec3 = Vec3::new(159.0, 12.0, -436.0);
pub const ROOF_R2: f32 = 4.0;

/// Length of the original's sine table (`0x411830`: sin(i · 2π / 4096), indices wrap at 4096 [I: it can read 4096]).
pub const SINE_LEN: i32 = 4096;
/// myfont's cell (`0x410ec0`: image width / 16 and height / 16 of a 256 x 192 image) [O].
pub const FONT_W: f32 = 16.0;
pub const FONT_H: f32 = 12.0;
/// The scroller's character pitch (`0x411987`), the sine amplitude (`0x4cf91b`) and the baseline (`0x41195b`) [O].
pub const SCROLL_PITCH: i32 = 8;
pub const SCROLL_AMP: f32 = 50.0;
pub const SCROLL_Y: f32 = 200.0;
/// "HCR&D" (`0x4cfafc`): letter pitch (`0x411b02`), jump height (`0x4cf923`), the letters' scale (`0x411a8b`),
/// the shadow's reach in font pixels (`0x413ac3`) and the colour (`0x411a71`) [O].
pub const BIG_PITCH: f32 = 100.0;
pub const BIG_JUMP: f32 = 200.0;
pub const BIG_SCALE: f32 = 10.0;
pub const BIG_COLOUR: [u8; 3] = [200, 30, 30];
pub const SHADOW: f32 = 2.0;
pub const BIG_TEXT: &str = "HCR&D";

/// Which egg is showing.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Egg {
    #[default]
    None,
    /// RuhrStadt: "WELCOME TO CLUB TBL!" / "Tonights Guest DJ: Randolph Carter!"
    Club,
    /// Atlantika: bouncing "HCR&D" and the sine scrolltext
    Roof,
}

/// `sin(i · 2π / 4096)` for the original's table index (wrapped into 0..4096).
pub fn sine(i: i32) -> f32 {
    (i.rem_euclid(SINE_LEN) as f32 * std::f32::consts::TAU / SINE_LEN as f32).sin()
}

#[derive(Clone, Debug, Default)]
pub struct MapEggs {
    /// ticks the car has stayed inside the club circle (`0x4e4984`)
    club_ticks: u32,
    /// the egg to draw this frame (`0x4e4990`)
    pub active: Egg,
    /// HCR&D's bounce phase (`0x4e3d10`)
    pub big_phase: i32,
    /// scroller: index of the first character (`0x4e3cf8`), the x offset (`0x4e3cfc`, starts at width + 50), the two sine
    /// phases (`0x4e3d00` starts 1024, `0x4e3d04` starts 0)
    pub first: usize,
    pub s: Option<i32>,
    pub phase_y: i32,
    pub phase_x: i32,
    /// time carried between ticks (s)
    carry: f32,
}

impl MapEggs {
    /// Run the frame's ticks. `track` is the `TrackName` ("RuhrStadt" / "Atlantika", case ignored), `pos` the followed car,
    /// `map_on` the RoadMap setting (the original tests only while the map is not None), `text_len` the scrolltext's
    /// length in characters and `w` the screen width the scroller starts from.
    pub fn update(&mut self, track: &str, pos: Vec3, dt: f32, map_on: bool, text_len: usize, w: f32) {
        self.carry += dt.max(0.0);
        let step = 1.0 / EGG_TICK_HZ.get();
        // a long frame (a stall, a breakpoint) must not replay seconds of ticks
        self.carry = self.carry.min(step * 8.0);
        while self.carry >= step {
            self.carry -= step;
            self.tick(track, pos, map_on, text_len, w);
        }
        if !map_on {
            self.active = Egg::None;
        }
    }

    fn tick(&mut self, track: &str, pos: Vec3, map_on: bool, text_len: usize, w: f32) {
        self.active = Egg::None;
        if !map_on {
            return;
        }
        if track.eq_ignore_ascii_case("ruhrstadt") {
            // 0x412d4d..0x412da0, 0x412f5a..0x412f70
            let (dx, dz) = (pos.x - CLUB_CENTRE.0, pos.z - CLUB_CENTRE.1);
            if dx * dx + dz * dz >= CLUB_R2 {
                self.club_ticks = 0;
            } else if self.club_ticks > CLUB_WAIT {
                self.active = Egg::Club;
            } else {
                self.club_ticks += 1;
            }
        } else if track.eq_ignore_ascii_case("atlantika") && pos.distance_squared(ROOF_CENTRE) < ROOF_R2 {
            // 0x412dbf..0x412e12
            self.active = Egg::Roof;
            self.advance_roof(text_len, w);
        }
    }

    /// One frame of `0x411a5c` (HCR&D phase +10) and `0x411868` (the scroller).
    fn advance_roof(&mut self, text_len: usize, w: f32) {
        self.big_phase = (self.big_phase + 10) % SINE_LEN;
        let s = self.s.get_or_insert_with(|| {
            // first frame of a race: 0x411879..0x4118a7
            self.phase_y = 1024;
            self.phase_x = 0;
            self.first = 0;
            w as i32 + 50
        });
        *s -= 1;
        if *s < 0 {
            *s = SCROLL_PITCH;
            self.first += 1;
            if text_len == 0 || self.first >= text_len {
                self.first = 0; // the NUL: wrap to the start (0x411a39)
            }
            self.phase_y = (self.phase_y + 200) % SINE_LEN;
            self.phase_x = (self.phase_x + 50) % SINE_LEN;
        }
        self.phase_y = (self.phase_y + 10) % SINE_LEN;
        self.phase_x = (self.phase_x + 10) % SINE_LEN;
    }
}

/// One scroller character: its index into the text and the screen position of its centre (`0x4118dd..0x411987`).
/// Characters run from `s - 50 - FONT_W` rightwards in `SCROLL_PITCH` steps while they start before `w + 50 + FONT_W`.
pub fn scroll_glyphs(e: &MapEggs, text_len: usize, w: f32) -> Vec<(usize, f32, f32)> {
    let mut out = Vec::new();
    let Some(s) = e.s else { return out };
    if text_len == 0 {
        return out;
    }
    let (mut x, mut ix, mut iy, mut at) = ((s - 50) as f32 - FONT_W, e.phase_x, e.phase_y, e.first);
    while x < w + 50.0 + FONT_W {
        if at >= text_len {
            at = 0;
        }
        // `fistp` after the chop helper `0x4914be`: truncation toward zero
        let dx = (SCROLL_AMP * sine(ix)).trunc();
        let dy = (SCROLL_AMP * sine(iy)).trunc();
        out.push((at, x + dx, SCROLL_Y + dy));
        iy = (iy + 200) % SINE_LEN;
        ix = (ix + 50) % SINE_LEN;
        x += SCROLL_PITCH as f32;
        at += 1;
    }
    out
}

/// The five HCR&D letters' top-left corners for screen `w` x `h` (`0x411a5c`).
pub fn big_letters(e: &MapEggs, w: f32, h: f32) -> Vec<(char, f32, f32)> {
    let base = h - BIG_SCALE * FONT_H;
    let (mut x, mut ph) = ((w - 500.0) * 0.5, e.big_phase);
    BIG_TEXT
        .chars()
        .map(|c| {
            let y = base - (sine(ph).abs() * BIG_JUMP).trunc();
            let r = (c, x.trunc(), y);
            ph = (ph + 200) % SINE_LEN;
            x += BIG_PITCH;
            r
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(e: &mut MapEggs, track: &str, pos: Vec3, ticks: u32) {
        for _ in 0..ticks {
            e.update(track, pos, 1.0 / EGG_TICK_HZ.get(), true, 100, 640.0);
        }
    }

    #[test]
    fn club_shows_after_the_wait_and_resets_outside() {
        let mut e = MapEggs::default();
        let door = Vec3::new(CLUB_CENTRE.0 + 5.0, 3.0, CLUB_CENTRE.1);
        run(&mut e, "RuhrStadt", door, CLUB_WAIT + 1);
        assert_eq!(e.active, Egg::None, "201 ticks in: the counter has just passed 200");
        run(&mut e, "RuhrStadt", door, 1);
        assert_eq!(e.active, Egg::Club);
        run(&mut e, "RuhrStadt", Vec3::new(CLUB_CENTRE.0 + 21.0, 3.0, CLUB_CENTRE.1), 1);
        assert_eq!(e.active, Egg::None);
        run(&mut e, "RuhrStadt", door, 5);
        assert_eq!(e.active, Egg::None, "leaving restarts the wait");
    }

    #[test]
    fn other_tracks_and_map_off_show_nothing() {
        let mut e = MapEggs::default();
        run(&mut e, "RedRock", Vec3::new(CLUB_CENTRE.0, 0.0, CLUB_CENTRE.1), 400);
        assert_eq!(e.active, Egg::None);
        let mut e = MapEggs::default();
        for _ in 0..400 {
            e.update("RuhrStadt", Vec3::new(CLUB_CENTRE.0, 0.0, CLUB_CENTRE.1), 1.0 / 60.0, false, 10, 640.0);
        }
        assert_eq!(e.active, Egg::None);
    }

    #[test]
    fn roof_is_a_two_metre_sphere() {
        let mut e = MapEggs::default();
        run(&mut e, "Atlantika", ROOF_CENTRE + Vec3::new(1.9, 0.0, 0.0), 1);
        assert_eq!(e.active, Egg::Roof);
        run(&mut e, "Atlantika", ROOF_CENTRE + Vec3::new(0.0, 2.1, 0.0), 1);
        assert_eq!(e.active, Egg::None);
    }

    #[test]
    fn scroller_moves_one_pixel_a_tick_and_a_character_every_eight() {
        let mut e = MapEggs::default();
        run(&mut e, "Atlantika", ROOF_CENTRE, 1);
        assert_eq!(e.s, Some(640 + 50 - 1));
        run(&mut e, "Atlantika", ROOF_CENTRE, 690);
        assert_eq!((e.s, e.first), (Some(8), 1), "691 ticks: s ran 690 -> -1 and reloaded");
        // continuous phases: one tick later nothing jumps
        let before = scroll_glyphs(&e, 100, 640.0);
        assert!(before.iter().all(|g| g.2 >= SCROLL_Y - SCROLL_AMP && g.2 <= SCROLL_Y + SCROLL_AMP));
    }

    #[test]
    fn big_letters_stand_100_apart_and_stay_on_screen() {
        let e = MapEggs::default();
        let l = big_letters(&e, 640.0, 480.0);
        assert_eq!(l.len(), 5);
        assert_eq!(l[1].1 - l[0].1, 100.0);
        assert_eq!(l[0].1, 70.0);
        assert!(l.iter().all(|&(_, _, y)| (480.0 - 120.0 - BIG_JUMP..=480.0 - 120.0).contains(&y)));
    }
}
