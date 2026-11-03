//! REMASTER "Race line" HUD (#323; mockup `reference_materials/hud-overhaul/C-raceline.html`, style
//! `re-notes/19-glass-hud-design-language.md`). The Modern HUD's pieces in the Glass look, drawn
//! in the mockup's design px (`Glass::hud`: 1280 x 720, a design px = 2/3 canvas px). Phase 1: the
//! top-left cluster; phase 2: the top-right stack (clocks, POSITION, LAP); phase 3: the map disc. Every
//! path here is REMASTER; the Classic HUD never calls this module.

use crate::front::menu_modern::{circle, glass_time, hex, sector_runs, start_tick, Glass, G_DARK, G_DIM, G_LINE, G_ORANGE, G_S1, G_S2, G_S3, G_TEXT};
use crate::front::Game;
use crate::hud::{digits, digits_width};
use glam::Vec3;
use motor_sim::race::TrackProfile;
use motorview::minimap::map_point;
use motorview::revmeter::{self, RevMeterTune};
use motorview::ui::{Batch, FontStyle, RevRed, WHITE};
use motorview::ui_font::{self, HudFont};
use motorview::ui_vector::{self as vec, Mesh};
use std::f32::consts::{FRAC_PI_2, TAU};

/// Where the cluster's group sits for the layout (`hud::place`): the anchor it scales about, canvas px.
pub const CLUSTER_ANCHOR: (f32, f32) = (12.0, 8.0);

/// The fill from which the swoosh counts as at the redline (mockup `Live.rev >= .92`).
pub const REDLINE: f32 = 0.92;
/// The redline pulse: the mockup's `redPulse .22s ease-in-out infinite alternate` (a full cycle is 0.44 s)
/// takes the lit part from opacity 1 down to .55.
const PULSE_PERIOD: f32 = 0.44;
const PULSE_DIM: f32 = 0.45;
/// The tube's half-width against the full meter (mockup `thin: 0.62`).
const THIN: f32 = 0.62;
/// Where KM/H and GEAR sit: design px of speed per digit (mockup `clusterHTML`: `w = digits x 52`).
const SPEED_DIGIT: f32 = 52.0;
/// The swoosh's width, design px.
const SWOOSH_W: f32 = 300.0;
/// The dashed rule closing the group: x, y and length, design px. The mockup's is 360 long; here it ends with
/// the swoosh (six dashes shorter), the user's tweak after the shots of #323 phase 6.
const RULE: (f32, f32, f32) = (36.0, 222.0, SWOOSH_W);

/// The cluster's box at 100 % from the anchor, canvas px (the dashed rule is its widest and lowest piece).
pub fn cluster_size() -> (f32, f32) {
    let d = Glass::hud(853.0, 1.0);
    (d.l(RULE.0 + RULE.2) - CLUSTER_ANCHOR.0, d.t(RULE.1 + 1.0) - CLUSTER_ANCHOR.1)
}

/// What the cluster shows.
pub struct Cluster<'a> {
    pub kmh: u32,
    /// "R" or the gear number
    pub gear: &'a str,
    /// rev fill 0..1
    pub frac: f32,
    /// seconds, drives the redline pulse
    pub clock: f32,
}

/// `.cl`: bed, speed with KM/H and GEAR, the swoosh, the dashed rule.
pub fn cluster(b: &mut Batch, d: Glass, c: &Cluster, tune: &RevMeterTune) {
    let red = c.frac >= REDLINE;
    // bed (`.wedge`): a soft dark wash so the text reads over bright sky and grass
    let mut m = Mesh::default();
    m.ellipse([d.l(200.0), d.t(90.0)], [d.d(350.0), d.d(220.0)], [3.0 / 255.0, 8.0 / 255.0, 16.0 / 255.0, 0.55], [3.0 / 255.0, 8.0 / 255.0, 16.0 / 255.0, 0.0], 48);
    b.mesh(&m);

    // speed (`.speed`): Glass 96, tracking -2, line height .78; chromatic fringes and a glow
    let speed = c.kmh.to_string();
    let st = d.font(HudFont::Glass, 96.0, -2.0, hex(0xf4fbff, 1.0));
    let st = if red { FontStyle { color: hex(0xffb0a4, 1.0), ..st } } else { st };
    let top = d.cap_top(HudFont::Glass, 96.0, d.t(22.0), Some(96.0 * 0.78));
    let x = d.l(32.0);
    let w = digits_width(b, HudFont::Glass, &speed, &st);
    let (glow, fringe_l, fringe_r) = if red {
        ([1.0, 80.0 / 255.0, 60.0 / 255.0, 0.3], [1.0, 70.0 / 255.0, 70.0 / 255.0, 0.7], [1.0, 200.0 / 255.0, 180.0 / 255.0, 0.5])
    } else {
        ([120.0 / 255.0, 200.0 / 255.0, 1.0, 0.26], [1.0, 70.0 / 255.0, 70.0 / 255.0, 0.55], [70.0 / 255.0, 200.0 / 255.0, 1.0, 0.55])
    };
    // APPROX: the 26 px text glow as a soft pad behind the run (as `chroma_text`), the 0 2px 3px shadow as a dark copy
    let mut m = Mesh::default();
    m.soft_pad(&vec::rounded_rect(x + d.d(4.0), top + d.d(4.0), w - d.d(8.0), st.size - d.d(8.0), d.d(10.0)), d.d(26.0), glow);
    b.mesh(&m);
    digits(b, HudFont::Glass, &speed, x, top + d.d(2.0), &FontStyle { color: [0.0, 0.0, 0.0, 0.4], ..st });
    let off = d.d(1.5);
    digits(b, HudFont::Glass, &speed, x - off, top, &FontStyle { color: fringe_l, ..st });
    digits(b, HudFont::Glass, &speed, x + off, top, &FontStyle { color: fringe_r, ..st });
    digits(b, HudFont::Glass, &speed, x, top, &st);

    // KM/H (`.kmh`, `.cap.lit` at 13 px) and GEAR (`.gear`: caption and value on one baseline), both
    // following the speed's digit count at the mockup's 52 design px per digit (its fixed cells
    // are narrower than this, so the caption keeps the mockup's distance)
    let after = d.l(36.0 + SPEED_DIGIT * speed.len() as f32);
    let cap = d.font(HudFont::GlassMed, 13.0, 1.5, hex(G_TEXT, 1.0));
    b.font_text(HudFont::GlassMed, "KM/H", after + d.d(6.0), d.cap_top(HudFont::GlassMed, 13.0, d.t(82.0), None), &cap);
    let gx = after + d.d(64.0);
    let num = d.font(HudFont::Clock, 36.0, 0.0, WHITE);
    let num_top = d.cap_top(HudFont::Clock, 36.0, d.t(56.0), None);
    let cap = d.font(HudFont::GlassMed, 11.0, 1.5, hex(G_DIM, 1.0));
    let label_w = b.font_width(HudFont::GlassMed, "GEAR", &cap);
    b.font_text(HudFont::GlassMed, "GEAR", gx, d.cap_above(HudFont::GlassMed, 11.0, num_top + num.size), &cap);
    // the value's soft glow (`0 0 12px rgba(190,230,255,.5)`) as a pad, APPROX
    let vx = gx + label_w + d.d(8.0);
    let vw = b.font_width(HudFont::Clock, c.gear, &num);
    let mut m = Mesh::default();
    m.soft_pad(&vec::rounded_rect(vx + d.d(3.0), num_top + d.d(3.0), vw - d.d(6.0), num.size - d.d(6.0), d.d(6.0)), d.d(12.0), [190.0 / 255.0, 230.0 / 255.0, 1.0, 0.2]);
    b.mesh(&m);
    b.font_text(HudFont::Clock, c.gear, vx, num_top + d.d(2.0), &FontStyle { color: [0.0, 0.0, 0.0, 0.4], ..num });
    b.font_text(HudFont::Clock, c.gear, vx, num_top, &num);

    // the swoosh (`.rev`): the glass rev meter, longer and thinner; at the redline its lit part pulses
    // and the glass edge turns red
    let thin = RevMeterTune { thickness: (tune.thickness as f32 * THIN).round() as u32, ..*tune };
    let rev = if red { RevRed { dim: PULSE_DIM * (0.5 - 0.5 * (TAU * c.clock / PULSE_PERIOD).cos()), rim: 1.0 } } else { RevRed::default() };
    b.rev_meter_sized(d.l(36.0), d.t(112.0), d.d(SWOOSH_W), c.frac, &thin, rev);
    debug_assert!(revmeter::size_for(d.d(SWOOSH_W)).1 > 0.0);

    // the dashed rule (`.rule`: 6 on, 4 off, `line` .45)
    let mut m = Mesh::default();
    m.dashed([d.l(RULE.0), d.t(RULE.1 + 0.5)], [d.l(RULE.0 + RULE.2), d.t(RULE.1 + 0.5)], d.d(6.0), d.d(4.0), d.w(1.0), hex(G_LINE, 0.45), d.px);
    b.mesh(&m);
}

// ---------------------------------------------------------------------------------------------
// the top-right stack (`cornerHTML`, `.clk`, `.tr`, `.bed` of the mockup)

/// The stack's right edge (design px; the mockup's `right: 36px`) and its top.
const STACK_RIGHT: f32 = 1244.0;
const STACK_TOP: f32 = 22.0;
/// The clocks: width and row height, and the gap under them.
const CLOCK_W: f32 = 236.0;
const CLOCK_ROW: f32 = 30.0;
const CLOCK_GAP: f32 = 22.0;
/// A POSITION / LAP block starts this far under the previous one (the mockup's `y += 74`).
const BLOCK_STEP: f32 = 74.0;
/// The bracket's reach past the right edge and its arm length, design px (`.brk`: right -14, 8 wide + 2 border).
const BRACKET_OUT: f32 = 14.0;
const BRACKET_ARM: f32 = 10.0;
/// The widest a block alone gets (a bracketed `POSITION` caption over `4 / 8`), design px.
const BLOCK_W: f32 = 120.0;

/// POSITION: your place of the field, or OUT in Knockout once you are eliminated.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Pos {
    Place(u32, u32),
    Out,
}

/// What the top-right stack shows; a `None` piece is hidden and the rest closes up.
pub struct Stack {
    /// TOTAL, LAP TIME, BEST in ms; BEST is `Some(None)` before the first lap (`--:--.---`)
    pub clocks: [Option<Option<u32>>; 3],
    /// the best lap beats your record of the track: the orange PB after BEST
    pub pb: bool,
    pub pos: Option<Pos>,
    /// lap, laps
    pub lap: Option<(u32, u32)>,
    /// on the last lap: the LAP caption reads FINAL LAP in orange
    pub final_lap: bool,
}

/// The mockup's `mono` cell of a character in em: digits (and the dash of an empty time) .93, `:` .30, `.` .26.
/// Audiowide's digits are proportional, so every character gets a fixed cell and a counting time stays put.
fn mono_cell(ch: char) -> f32 {
    match ch {
        '0'..='9' | '-' => 0.93,
        ':' => 0.30,
        '.' => 0.26,
        _ => 0.6,
    }
}

/// Width of `s` in mono cells; `em` = the font size in canvas px.
fn mono_width(s: &str, em: f32) -> f32 {
    s.chars().map(|c| mono_cell(c) * em).sum()
}

/// `s` with every character centred in its mono cell; returns the advance.
fn mono(b: &mut Batch, font: HudFont, s: &str, x: f32, y: f32, st: &FontStyle, em: f32) -> f32 {
    let mut cx = x;
    for c in s.chars() {
        let cs = c.to_string();
        let adv = mono_cell(c) * em;
        let w = b.font_width(font, &cs, st);
        b.font_text(font, &cs, cx + (adv - w) * 0.5, y, st);
        cx += adv;
    }
    cx - x
}

/// A CSS line box of `font` at `px` with its normal line height, design px.
fn normal_line(font: HudFont, px: f32) -> f32 {
    let f = ui_font::face(font);
    (f.ascender - f.descender) / ui_font::SRC * px
}

/// A POSITION / LAP block's height: the caption's line over the big numerals (`line-height: .8` of 56 px).
fn block_h() -> f32 {
    normal_line(HudFont::GlassMed, 11.0) + 56.0 * 0.8
}

/// `.clk` / `.tr` heights down the stack: (the bottom of the last piece, every piece's top).
fn stack_layout(clocks: usize, pos: bool, lap: bool) -> (f32, [f32; 3]) {
    let mut y = STACK_TOP;
    let mut bottom = STACK_TOP;
    let mut tops = [y; 3];
    if clocks > 0 {
        bottom = y + clocks as f32 * CLOCK_ROW;
        y = bottom + CLOCK_GAP;
    }
    for (k, on) in [pos, lap].into_iter().enumerate() {
        if on {
            tops[k + 1] = y;
            bottom = y + block_h();
            y += BLOCK_STEP;
        }
    }
    (bottom, tops)
}

/// The stack's box at 100 % from its anchor (`w - 12`, 8; `hud::place`), canvas px; (0, 0) when nothing shows.
pub fn stack_size(clocks: usize, pos: bool, lap: bool) -> (f32, f32) {
    if clocks == 0 && !pos && !lap {
        return (0.0, 0.0);
    }
    let d = Glass::hud(853.0, 1.0);
    let (bottom, _) = stack_layout(clocks, pos, lap);
    let left = if clocks > 0 { CLOCK_W } else { BLOCK_W };
    (d.d(left) + (d.d(1280.0 - STACK_RIGHT) - CLUSTER_ANCHOR.0).max(0.0), d.t(bottom) - CLUSTER_ANCHOR.1)
}

/// text with its soft drop shadow (`0 1px 2px` black .9, as a dark copy)
fn shadowed(b: &mut Batch, font: HudFont, s: &str, x: f32, y: f32, st: &FontStyle, off: f32) {
    b.font_text(font, s, x, y + off, &FontStyle { color: [0.0, 0.0, 0.0, 0.3], ..*st });
    b.font_text(font, s, x, y, st);
}

/// The top-right stack: the clocks on hairlines, then POSITION and LAP as large numerals with a bracket.
pub fn stack(b: &mut Batch, d: Glass, s: &Stack) {
    let n = s.clocks.iter().flatten().count();
    if n == 0 && s.pos.is_none() && s.lap.is_none() {
        return;
    }
    let (_, tops) = stack_layout(n, s.pos.is_some(), s.lap.is_some());
    let right = d.r(STACK_RIGHT);
    // clocks (`.clk`): caption GlassMed 11 / 1.5 `dim` left, value Audiowide 17 white right, 1 px hairlines between rows
    if n > 0 {
        let left = d.r(STACK_RIGHT - CLOCK_W);
        let cap = d.font(HudFont::GlassMed, 11.0, 1.5, hex(G_DIM, 1.0));
        let num = d.font(HudFont::Clock, 17.0, 0.0, WHITE);
        let tag = d.font(HudFont::Glass, 10.0, 1.5, hex(G_ORANGE, 1.0));
        let em = d.d(17.0);
        let labels = ["TOTAL", "LAP TIME", "BEST"];
        let mut y = STACK_TOP;
        for (k, row) in s.clocks.iter().enumerate() {
            let Some(ms) = row else { continue };
            if y > STACK_TOP {
                let mut m = Mesh::default();
                let hy = d.t(y) + d.w(1.0) * 0.5;
                m.stroke(&[[left, hy], [right, hy]], false, d.w(1.0), vec::Ends::Butt, hex(G_LINE, 0.22), d.px);
                b.mesh(&m);
            }
            let cap_y = d.cap_top(HudFont::GlassMed, 11.0, d.t(y), Some(CLOCK_ROW));
            shadowed(b, HudFont::GlassMed, labels[k], left, cap_y, &cap, d.d(1.0));
            if k == 2 && s.pb {
                // PB sits on the caption's baseline, 8 px after it
                let tx = left + b.font_width(HudFont::GlassMed, labels[k], &cap) + d.d(8.0);
                b.font_text(HudFont::Glass, "PB", tx, cap_y + cap.size - tag.size, &tag);
            }
            let text = ms.map_or_else(|| "--:--.---".to_string(), glass_time);
            let w = mono_width(&text, em);
            let ny = d.cap_top(HudFont::Clock, 17.0, d.t(y), Some(CLOCK_ROW));
            mono(b, HudFont::Clock, &text, right - w, ny + d.d(1.0), &FontStyle { color: [0.0, 0.0, 0.0, 0.3], ..num }, em);
            mono(b, HudFont::Clock, &text, right - w, ny, &num, em);
            y += CLOCK_ROW;
        }
    }

    // POSITION and LAP (`.tr`): caption, then the numeral Glass 56 and ` / m` Glass 24 `dim`, a bracket on the right
    let orange = hex(G_ORANGE, 1.0);
    if let Some(pos) = s.pos {
        let (big, of) = match pos {
            Pos::Place(p, of) => (p.to_string(), Some(of)),
            Pos::Out => ("OUT".to_string(), None),
        };
        block(b, d, tops[1], ("POSITION", hex(G_DIM, 1.0)), &big, orange, [1.0, 154.0 / 255.0, 60.0 / 255.0, 0.25], of);
    }
    if let Some((lap, laps)) = s.lap {
        let cap = if s.final_lap { ("FINAL LAP", orange) } else { ("LAP", hex(G_DIM, 1.0)) };
        block(b, d, tops[2], cap, &lap.to_string(), WHITE, [190.0 / 255.0, 230.0 / 255.0, 1.0, 0.22], Some(laps));
    }
}

/// One `.tr` block at design y `y`.
#[allow(clippy::too_many_arguments)]
fn block(b: &mut Batch, d: Glass, y: f32, caption: (&str, [f32; 4]), big: &str, big_col: [f32; 4], glow: [f32; 4], of: Option<u32>) {
    let right = d.r(STACK_RIGHT);
    let cap = d.font(HudFont::GlassMed, 11.0, 1.5, caption.1);
    let cw = b.font_width(HudFont::GlassMed, caption.0, &cap);
    shadowed(b, HudFont::GlassMed, caption.0, right - cw, d.cap_top(HudFont::GlassMed, 11.0, d.t(y), None), &cap, d.d(1.0));

    let line = normal_line(HudFont::GlassMed, 11.0);
    let num = d.font(HudFont::Glass, 56.0, 0.0, big_col);
    let top = d.cap_top(HudFont::Glass, 56.0, d.t(y + line), Some(56.0 * 0.8));
    let of_text = of.map(|n| format!(" / {n}"));
    let small = d.font(HudFont::Glass, 24.0, 0.0, hex(G_DIM, 1.0));
    let ow = of_text.as_ref().map_or(0.0, |t| b.font_width(HudFont::Glass, t, &small));
    let nw = b.font_width(HudFont::Glass, big, &num);
    let x = right - ow - nw;
    // APPROX: the numeral's 14 px glow as a soft pad, the 0 2px 3px shadow as a dark copy (as the cluster)
    let mut m = Mesh::default();
    m.soft_pad(&vec::rounded_rect(x + d.d(4.0), top + d.d(4.0), nw - d.d(8.0), num.size - d.d(8.0), d.d(8.0)), d.d(14.0), glow);
    b.mesh(&m);
    b.font_text(HudFont::Glass, big, x, top + d.d(2.0), &FontStyle { color: [0.0, 0.0, 0.0, 0.4], ..num });
    b.font_text(HudFont::Glass, big, x, top, &num);
    if let Some(t) = &of_text {
        // on the numeral's baseline
        let ty = top + num.size - small.size;
        b.font_text(HudFont::Glass, t, x + nw, ty + d.d(2.0), &FontStyle { color: [0.0, 0.0, 0.0, 0.4], ..small });
        b.font_text(HudFont::Glass, t, x + nw, ty, &small);
    }

    // the bracket: `.brk` right -14, top -4, 2 px `line` .6 on its right, top and bottom
    let (x1, y0, y1) = (right + d.d(BRACKET_OUT), d.t(y - 4.0), d.t(y - 4.0 + block_h() + 4.0));
    let x0 = x1 - d.d(BRACKET_ARM);
    let w = d.w(2.0);
    let mut m = Mesh::default();
    m.stroke(&[[x0, y0 + w * 0.5], [x1 - w * 0.5, y0 + w * 0.5], [x1 - w * 0.5, y1 - w * 0.5], [x0, y1 - w * 0.5]], false, w, vec::Ends::Butt, hex(G_LINE, 0.6), d.px);
    b.mesh(&m);
}

// ---------------------------------------------------------------------------------------------
// the map disc (`mapHTML`, `mapDiscHTML`, `mapSVG` of the mockup)

/// The disc's centre (design px) and the reach of everything that turns round it (tick ring and N chip).
const MAP_C: (f32, f32) = (1143.0, 585.0);
const MAP_REACH: f32 = 100.0;
/// The bezel's outer radius, the face's radius (bezel 6 + gap 4 in) and the tick ring's radius, design px.
const MAP_R: f32 = 82.0;
const MAP_FACE: f32 = MAP_R - 10.0;
const MAP_RING: f32 = MAP_R + 8.0;
/// Where the map's group is anchored (`hud::place` scales about it): canvas px in from the right / bottom edge.
pub const MAP_INSET: (f32, f32) = (10.0, 10.0);
/// REMASTER: capital height of the place numbers over the dots (canvas px; larger than the mockup's ~3 so
/// they stay legible, as today's HUD [design choice]).
const MAP_NR_CAP: f32 = 6.0;
/// The dots, design px (mockup `3.8` / `6` svg units at 127 / 200).
const DOT: f32 = 2.4;
const DOT_ME: f32 = 3.8;

/// One car on the map.
pub struct MapDot {
    pub pos: Vec3,
    pub colour: [u8; 3],
    /// the number over the dot (None: no number, a ghost, or your own dot)
    pub place: Option<u32>,
    pub me: bool,
    pub ghost: bool,
    /// CHEAT avenger / zx (#322): drawn as the original's ninja picture instead of a dot
    pub ninja: bool,
}

/// CHEAT avenger / zx (#322): the ninja's height on the face, design px (about three times your dot's height).
const NINJA_H: f32 = 14.0;

/// A sprite's pixels as horizontal runs `(y, x0, x1, light)`, `x1` exclusive: the transparent pixels (alpha 0, the
/// magenta key) are left out, a white one is `light`, a black one is not. `rgba` is `w * h` tightly packed.
fn pixel_runs(rgba: &[u8], w: usize, h: usize) -> Vec<(usize, usize, usize, bool)> {
    let mut runs = Vec::new();
    for y in 0..h {
        let mut x = 0;
        while x < w {
            let px = &rgba[(y * w + x) * 4..][..4];
            if px[3] == 0 {
                x += 1;
                continue;
            }
            let light = px[0] > 127;
            let x0 = x;
            while x < w {
                let q = &rgba[(y * w + x) * 4..][..4];
                if q[3] == 0 || (q[0] > 127) != light {
                    break;
                }
                x += 1;
            }
            runs.push((y, x0, x, light));
        }
    }
    runs
}

/// REMASTER (design choice): the ninja as vector rectangles, one per run of the original picture's pixels, so it stays
/// sharp at any HUD scale (the sprite sampler is linear). The picture's white is the driver's colour, its black the
/// dark outline. Centred on `at`, `NINJA_H` tall.
struct Ninja {
    w: usize,
    h: usize,
    runs: Vec<(usize, usize, usize, bool)>,
}

impl Ninja {
    /// From `avenger.tga` in the HUD atlas; None when the install lacks it (the dot is drawn then).
    fn load(b: &Batch) -> Option<Ninja> {
        let a = &b.assets.atlas;
        let r = a.get("h:avenger")?;
        let (w, h) = (r.w as usize, r.h as usize);
        let mut rgba = Vec::with_capacity(w * h * 4);
        for y in 0..h {
            let o = ((r.y as usize + y) * motorview::ui::ATLAS as usize + r.x as usize) * 4;
            rgba.extend_from_slice(a.rgba.get(o..o + w * 4)?);
        }
        Some(Ninja { w, h, runs: pixel_runs(&rgba, w, h) })
    }

    fn draw(&self, m: &mut Mesh, d: Glass, at: [f32; 2], light: [f32; 4], dark: [f32; 4]) {
        let k = d.d(NINJA_H) / self.h as f32;
        let (x0, y0) = (at[0] - self.w as f32 * k * 0.5, at[1] - self.h as f32 * k * 0.5);
        for &(y, a, z, l) in &self.runs {
            let (l0, l1, t0, t1) = (x0 + a as f32 * k, x0 + z as f32 * k, y0 + y as f32 * k, y0 + (y + 1) as f32 * k);
            m.fill(&[[l0, t0], [l1, t0], [l1, t1], [l0, t1]], if l { light } else { dark }, 0.0);
        }
    }
}

/// What the face shows: the whole lap, or a window round you.
#[derive(Clone, Copy)]
pub enum MapFrame {
    Circuit,
    /// centred on `at`, `metres` across the face, the map turned `turn` radians clockwise on screen
    Close { at: Vec3, metres: f32, turn: f32 },
}

/// The map's box at 100 % from its anchor (canvas px in from the right / bottom edge by `MAP_INSET`): everything
/// within `MAP_REACH` of the disc's centre, so the box does not change as the ring and the N chip turn.
pub fn map_size() -> (f32, f32) {
    let d = Glass::hud(853.0, 1.0);
    (d.d(1280.0 - MAP_C.0 + MAP_REACH) - MAP_INSET.0, d.d(720.0 - MAP_C.1 + MAP_REACH) - MAP_INSET.1)
}

/// The parts of the open polyline `run` that lie inside the disc (`c`, `r`), each as its own polyline; the
/// cuts sit on the circle. (Vector meshes in the sharp pipeline ignore the ellipse clip.)
fn clip_to_disc(run: &[[f32; 2]], c: [f32; 2], r: f32) -> Vec<Vec<[f32; 2]>> {
    let mut out: Vec<Vec<[f32; 2]>> = Vec::new();
    let mut open = false;
    for w in run.windows(2) {
        let (a, v) = (w[0], [w[1][0] - w[0][0], w[1][1] - w[0][1]]);
        // |a + t v - c|^2 = r^2
        let (f, qa) = ([a[0] - c[0], a[1] - c[1]], v[0] * v[0] + v[1] * v[1]);
        let (qb, qc) = (2.0 * (f[0] * v[0] + f[1] * v[1]), f[0] * f[0] + f[1] * f[1] - r * r);
        let disc = qb * qb - 4.0 * qa * qc;
        if qa < 1e-9 || disc <= 0.0 {
            open = false;
            continue;
        }
        let (t0, t1) = ((-qb - disc.sqrt()) / (2.0 * qa), (-qb + disc.sqrt()) / (2.0 * qa));
        let (lo, hi) = (t0.max(0.0), t1.min(1.0));
        if lo >= hi {
            open = false;
            continue;
        }
        let at = |t: f32| [a[0] + v[0] * t, a[1] + v[1] * t];
        if !(open && lo <= 0.0) {
            out.push(vec![at(lo)]);
        }
        out.last_mut().unwrap().push(at(hi));
        open = hi >= 1.0;
    }
    out
}

/// The map disc: tick ring and N chip (turning with the map), the metal bezel, the grid face with the lap in
/// the three sector colours, the start tick and the cars. REMASTER (#323 phase 3).
/// APPROX: the mockup's 4 px glow under the lap is two wider translucent strokes (the glow pass would add a
/// render pass and does not follow the HUD's placement); the gloss sits under the lap, not over it.
/// N is a best guess [I]: the track data has no compass, so up in the unturned map stands for north.
pub fn map(b: &mut Batch, d: Glass, info: [f32; 4], p: &TrackProfile, frame: MapFrame, dots: &[MapDot]) {
    if p.outline.len() < 2 {
        return;
    }
    let c = [d.r(MAP_C.0), d.b(MAP_C.1)];
    let (rb, rf) = (d.d(MAP_R), d.d(MAP_FACE));
    let turn = if let MapFrame::Close { turn, .. } = frame { turn } else { 0.0 };

    // the tick ring (80 ticks, 5 x 1, the first at north) and the N chip turn with the map
    let mut m = Mesh::default();
    let north = -FRAC_PI_2 + turn;
    for i in 0..80 {
        if i <= 2 || i >= 78 {
            continue; // the chip sits here (+-9 degrees)
        }
        let a = north + i as f32 / 80.0 * TAU;
        let dir = [a.cos(), a.sin()];
        let (r0, r1) = (rb + d.d(5.5), rb + d.d(10.5));
        m.stroke(&[[c[0] + dir[0] * r0, c[1] + dir[1] * r0], [c[0] + dir[0] * r1, c[1] + dir[1] * r1]], false, d.w(1.0), vec::Ends::Butt, hex(G_TEXT, 0.5), d.px);
    }
    let chip_c = [c[0] + north.cos() * d.d(MAP_RING), c[1] + north.sin() * d.d(MAP_RING)];
    let chip = circle(chip_c, d.d(10.0));
    m.halo(&chip, d.d(6.0), hex(0x4aa8ff, 0.8));
    m.fill(&chip, hex(G_DARK, 1.0), d.px);
    m.stroke(&chip, true, d.w(1.5), vec::Ends::Round, hex(G_S1, 1.0), d.px);

    // bezel, face, grid, gloss and inset shadow (the menus' small disc)
    Game::glass_bezel_small(&mut m, d, c, rb);
    m.disc_at(c, rf, [c[0], c[1] - 0.4 * rf], [120.0 / 255.0, 190.0 / 255.0, 1.0, 0.24], [8.0 / 255.0, 20.0 / 255.0, 40.0 / 255.0, 0.9], 96);
    Game::disc_overlays(&mut m, d, c, rf, 22.0, hex(G_LINE, 0.12), (0.2, 0.34), (0.0, 0.0), (30.0, 0.6));
    b.mesh(&m);

    // the lap and the cars in map pixels -> canvas: the whole lap fitted into the face, or the window round you
    let map = |q: Vec3| map_point(info, q);
    let pts: Vec<(f32, f32)> = p.outline.iter().map(|&q| map(Vec3::from_array(q))).collect();
    let to: Box<dyn Fn((f32, f32)) -> [f32; 2]> = match frame {
        MapFrame::Circuit => {
            let (mut lo, mut hi) = ((f32::MAX, f32::MAX), (f32::MIN, f32::MIN));
            for &(x, y) in &pts {
                lo = (lo.0.min(x), lo.1.min(y));
                hi = (hi.0.max(x), hi.1.max(y));
            }
            let (mx, my) = ((lo.0 + hi.0) * 0.5, (lo.1 + hi.1) * 0.5);
            let half = (((hi.0 - lo.0) * 0.5).powi(2) + ((hi.1 - lo.1) * 0.5).powi(2)).sqrt().max(1e-3);
            let k = 0.84 * rf / half;
            Box::new(move |(x, y)| [c[0] + (x - mx) * k, c[1] + (y - my) * k])
        }
        MapFrame::Close { at, metres, turn } => {
            let (o, e) = (map(at), map(at + Vec3::X));
            let per_m = ((e.0 - o.0).powi(2) + (e.1 - o.1).powi(2)).sqrt().max(1e-4);
            let k = 2.0 * rf / metres.max(1.0) / per_m;
            let (sn, cs) = turn.sin_cos();
            Box::new(move |(x, y)| {
                let (vx, vy) = ((x - o.0) * k, (y - o.1) * k);
                [c[0] + vx * cs - vy * sn, c[1] + vx * sn + vy * cs]
            })
        }
    };
    let line: Vec<[f32; 2]> = pts.iter().map(|&q| to(q)).collect();
    let mut m = Mesh::default();
    for (run, col) in sector_runs(p, &line).iter().zip([G_S1, G_S2, G_S3]) {
        for piece in clip_to_disc(run, c, rf) {
            // the glow, then the 3 px line
            m.stroke(&piece, false, d.d(11.0), vec::Ends::Round, hex(col, 0.07), d.px);
            m.stroke(&piece, false, d.d(7.0), vec::Ends::Round, hex(col, 0.14), d.px);
            m.stroke(&piece, false, d.w(3.0), vec::Ends::Round, hex(col, 1.0), d.px);
        }
    }
    // the start / finish line: a white tick across the track
    if (line[0][0] - c[0]).hypot(line[0][1] - c[1]) < rf - d.d(4.0) {
        start_tick(&mut m, &line, d.d(4.5), d.w(1.5), d.px);
    }
    b.mesh(&m);

    // the cars: rivals first, you on top; those near the face's edge fade out, those past it are dropped
    let mut order: Vec<&MapDot> = dots.iter().collect();
    order.sort_by_key(|x| x.me);
    let mut m = Mesh::default();
    let mut nrs: Vec<([f32; 2], f32, [f32; 4], u32)> = Vec::new();
    let ninja = dots.iter().any(|x| x.ninja).then(|| Ninja::load(b)).flatten();
    for dot in order {
        let at = to(map(dot.pos));
        let dist = ((at[0] - c[0]).powi(2) + (at[1] - c[1]).powi(2)).sqrt() / rf;
        if dist > 0.95 {
            continue;
        }
        let fade = 1.0 - smooth(0.75, 1.0, dist);
        let col = (u32::from(dot.colour[0]) << 16) | (u32::from(dot.colour[1]) << 8) | u32::from(dot.colour[2]);
        let (rd, colour) = if dot.ghost { (d.d(DOT), [0.75, 0.9, 1.0, 0.55 * fade]) } else { (d.d(if dot.me { DOT_ME } else { DOT }), hex(col, fade)) };
        if let (true, Some(n)) = (dot.ninja, &ninja) {
            // CHEAT avenger / zx (#322)
            let rd = d.d(NINJA_H) * 0.5;
            if dot.me {
                m.halo(&circle(at, d.d(DOT_ME)), d.d(7.0), hex(col, 0.7 * fade));
            }
            n.draw(&mut m, d, at, hex(col, fade), hex(G_DARK, fade));
            if let Some(nr) = dot.place {
                nrs.push((at, rd, hex(col, fade), nr));
            }
            continue;
        }
        let shape = circle(at, rd);
        if dot.me {
            m.halo(&shape, d.d(7.0), hex(col, 0.7 * fade));
            m.fill(&shape, colour, d.px);
            m.stroke(&shape, true, d.w(1.3), vec::Ends::Round, [1.0, 1.0, 1.0, fade], d.px);
        } else {
            m.fill(&shape, colour, d.px);
            if !dot.ghost {
                m.stroke(&shape, true, d.w(0.8), vec::Ends::Round, hex(G_DARK, fade), d.px);
            }
        }
        if let Some(n) = dot.place {
            nrs.push((at, rd, hex(col, fade), n));
        }
    }
    b.mesh(&m);
    // place numbers, upright, in the driver's colour with a dark outline
    for (at, rd, col, n) in nrs {
        let st = FontStyle::new(MAP_NR_CAP, col).outline([0.0, 0.0, 0.0], 0.7);
        let text = n.to_string();
        let w = b.font_width(HudFont::Clock, &text, &st);
        b.font_text(HudFont::Clock, &text, at[0] - w * 0.5, at[1] - rd - d.d(1.5) - MAP_NR_CAP, &st);
    }

    // the dark gap over the face's edge, then the N (upright on its chip)
    let mut m = Mesh::default();
    Game::gap_ring(&mut m, d, c, rf, rb - d.d(6.0));
    b.mesh(&m);
    let st = d.font(HudFont::Glass, 13.0, 0.0, hex(0xf4fbff, 1.0));
    let w = b.font_width(HudFont::Glass, "N", &st);
    b.font_text(HudFont::Glass, "N", chip_c[0] - w * 0.5, d.cap_above(HudFont::Glass, 13.0, chip_c[1] + d.d(4.2)), &st);
}

// ---------------------------------------------------------------------------------------------
// the splits (`splitsHTML`, `.sl`) and the centre messages (`centreHTML`, `.cmsg`, `.msg.warn`)

/// The splits' left edge and bottom (design px; the mockup's `left: 36px; bottom: 36px`), a row's height and width.
const SPLIT_LEFT: f32 = 36.0;
const SPLIT_BOTTOM: f32 = 684.0;
const SPLIT_ROW: f32 = 26.0;
const SPLIT_W: f32 = 250.0;
/// The caption's distance to the first row (`margin-bottom: 4px`).
const SPLIT_CAP_GAP: f32 = 4.0;
/// Where the splits group is anchored for the layout (`hud::place` scales about it): canvas px, the left edge
/// and 10 px above the bottom.
pub const SPLITS_ANCHOR: (f32, f32) = (12.0, 470.0);

/// A row's gap cell.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RowGap {
    /// ms to you (negative: ahead of you)
    Ms(i64),
    /// REMASTER live gap (#302): whole laps to you (negative: ahead of you)
    Laps(i32),
    /// REMASTER live gap: it cannot be told (a car out of the race, none passed there yet)
    Dashes,
}

impl RowGap {
    /// The cell's text: `-0.678`, `+1:05.204`, `-1 LAP`, `+2 LAPS`, `--.---`.
    pub fn text(self) -> String {
        match self {
            RowGap::Ms(ms) => glass_gap(ms),
            RowGap::Laps(n) => format!("{}{} LAP{}", if n < 0 { '-' } else { '+' }, n.unsigned_abs(), if n.unsigned_abs() == 1 { "" } else { "S" }),
            RowGap::Dashes => "--.---".to_string(),
        }
    }

    /// The checkpoint list: green ahead of you, red behind. The live gaps (`live`, #302, the user's choice): red for the
    /// car ahead (it is beating you), green for the car behind. Plain on a tie, dim for dashes.
    fn colour(self, live: bool) -> u32 {
        const GREEN: u32 = 0x9ff5c2;
        const RED: u32 = 0xff8a7a;
        let (ahead, behind) = if live { (RED, GREEN) } else { (GREEN, RED) };
        match self {
            RowGap::Ms(x) if x < 0 => ahead,
            RowGap::Ms(x) if x > 0 => behind,
            RowGap::Laps(n) if n < 0 => ahead,
            RowGap::Laps(n) if n > 0 => behind,
            RowGap::Dashes => G_DIM,
            _ => G_TEXT,
        }
    }
}

/// One row of the splits: a car at the checkpoint, a Time Attack reference, or (live gaps) a neighbour.
pub struct SplitRow {
    /// the car's place at the checkpoint (None: a Time Attack row)
    pub place: Option<u32>,
    pub colour: [u8; 3],
    pub name: String,
    /// the gap to you; None for your own row
    pub gap: Option<RowGap>,
    /// your own row: your time at the checkpoint, shown in the gap column in white
    pub time: Option<u32>,
    pub you: bool,
}

/// The bottom-left block: a caption over its rows. A `None` row is an empty slot: it keeps the block's height,
/// so your own row stays put in the live gaps block (#302) when you lead or run last.
pub struct Splits {
    pub caption: String,
    pub rows: Vec<Option<SplitRow>>,
    /// the live gaps block (#302): the colours of the gaps are the other way round
    pub live: bool,
}

/// The design y of the block's top for `rows` rows.
fn splits_top(rows: usize) -> f32 {
    SPLIT_BOTTOM - rows as f32 * SPLIT_ROW - SPLIT_CAP_GAP - normal_line(HudFont::GlassMed, 11.0)
}

/// The block's box at 100 % from its anchor, canvas px; (0, 0) without rows.
pub fn splits_size(rows: usize) -> (f32, f32) {
    if rows == 0 {
        return (0.0, 0.0);
    }
    let d = Glass::hud(853.0, 1.0);
    (d.l(SPLIT_LEFT + SPLIT_W) - SPLITS_ANCHOR.0, SPLITS_ANCHOR.1 - d.t(splits_top(rows)))
}

/// A gap as the mockup's `fmtGap`: `+1.214`, `-0.678`, from a minute `+1:05.204`.
pub fn glass_gap(ms: i64) -> String {
    let a = ms.unsigned_abs();
    let sign = if ms < 0 { '-' } else { '+' };
    if a >= 60_000 {
        format!("{sign}{}:{:02}.{:03}", a / 60_000, (a / 1000) % 60, a % 1000)
    } else {
        format!("{sign}{}.{:03}", a / 1000, a % 1000)
    }
}

/// The splits: bed, caption, and a row each of place, colour bar, name and gap; your row is white with a blue marker.
pub fn splits(b: &mut Batch, d: Glass, s: &Splits) {
    if s.rows.is_empty() {
        return;
    }
    let top = splits_top(s.rows.len());
    // bed (`.bed` 560 x 340 at -140 / -140): a soft dark wash behind the block
    let mut m = Mesh::default();
    m.ellipse([d.l(140.0), d.t(690.0)], [d.d(280.0), d.d(170.0)], [3.0 / 255.0, 8.0 / 255.0, 16.0 / 255.0, 0.6], [3.0 / 255.0, 8.0 / 255.0, 16.0 / 255.0, 0.0], 48);
    b.mesh(&m);

    let left = d.l(SPLIT_LEFT);
    let cap = d.font(HudFont::GlassMed, 11.0, 1.5, hex(G_DIM, 1.0));
    shadowed(b, HudFont::GlassMed, &s.caption, left, d.cap_top(HudFont::GlassMed, 11.0, d.t(top), None), &cap, d.d(1.0));
    let place = d.font(HudFont::Clock, 11.0, 0.0, hex(G_DIM, 1.0));
    let em = d.d(13.0);
    let mut y = top + normal_line(HudFont::GlassMed, 11.0) + SPLIT_CAP_GAP;
    for row in &s.rows {
        let Some(row) = row else {
            y += SPLIT_ROW;
            continue;
        };
        let col = (u32::from(row.colour[0]) << 16) | (u32::from(row.colour[1]) << 8) | u32::from(row.colour[2]);
        if let Some(p) = row.place {
            shadowed(b, HudFont::Clock, &p.to_string(), left, d.cap_top(HudFont::Clock, 11.0, d.t(y), Some(SPLIT_ROW)), &place, d.d(1.0));
        }
        // the colour bar (3 x 14, r 2, a 6 px glow)
        let bar = vec::rounded_rect(left + d.d(24.0), d.t(y + (SPLIT_ROW - 14.0) * 0.5), d.d(3.0), d.d(14.0), d.d(1.5));
        let mut m = Mesh::default();
        m.soft_pad(&bar, d.d(6.0), hex(col, 0.6));
        m.fill(&bar, hex(col, 1.0), d.px);
        if row.you {
            // `.you::before`: a 2 px bar 12 px left of the row with an 8 px glow
            let mark = vec::rounded_rect(left - d.d(12.0), d.t(y + 3.0), d.d(2.0), d.d(SPLIT_ROW - 6.0), d.d(1.0));
            m.soft_pad(&mark, d.d(8.0), hex(0x4aa8ff, 0.8));
            m.fill(&mark, hex(G_S1, 1.0), d.px);
        }
        b.mesh(&m);
        let name = d.font(HudFont::Glass, 16.0, 0.5, if row.you { WHITE } else { hex(G_TEXT, 1.0) });
        shadowed(b, HudFont::Glass, &row.name, left + d.d(37.0), d.cap_top(HudFont::Glass, 16.0, d.t(y), Some(SPLIT_ROW)), &name, d.d(1.0));
        // the gap to you (green ahead, red behind), or on your own row your time (white)
        let cell = row.gap.map(|g| (g.text(), hex(g.colour(s.live), 1.0))).or_else(|| row.time.map(|ms| (glass_time(ms), WHITE)));
        if let Some((text, c)) = cell {
            let st = d.font(HudFont::Clock, 13.0, 0.0, c);
            let w = mono_width(&text, em);
            let gy = d.cap_top(HudFont::Clock, 13.0, d.t(y), Some(SPLIT_ROW));
            mono(b, HudFont::Clock, &text, left + d.d(SPLIT_W) - w, gy + d.d(1.0), &FontStyle { color: [0.0, 0.0, 0.0, 0.3], ..st }, em);
            mono(b, HudFont::Clock, &text, left + d.d(SPLIT_W) - w, gy, &st, em);
        }
        y += SPLIT_ROW;
    }
}

/// A centre message; they stack from the top, none over another.
pub enum Msg {
    /// BEST LAP (orange) or LAST LAP (white) with the lap time; `pb` adds the PB pill
    Lap { best: bool, ms: u32, pb: bool },
    /// the notice when the last lap starts
    FinalLap,
    /// FINISHED p / n over LAST LAP and RACE TIME
    Finished { pos: u32, of: u32, last: u32, total: u32 },
    /// Knockout: you are out
    Eliminated,
    /// the 4 s notice of Record race / Ghost mode / Demo play (#345), white, a size under the lap messages
    Notice(String),
}

/// The first message's line top and the gap between messages, design px (`.cmsg` at top 48).
const MSG_TOP: f32 = 48.0;
const MSG_GAP: f32 = 10.0;
/// The sub line of FINISHED sits this far under the title (`.sub` margin-top 6).
const MSG_SUB_GAP: f32 = 6.0;

fn msg_title_h() -> f32 {
    normal_line(HudFont::Glass, 30.0)
}

fn msg_sub_h() -> f32 {
    normal_line(HudFont::Glass, 14.0).max(normal_line(HudFont::Clock, 16.0))
}

/// The messages' column, each centred on the canvas (`.cmsg`): Glass 30 / 3 with a 18 px glow in its colour, the
/// times in Audiowide; orange for BEST LAP, FINAL LAP and FINISHED, white for LAST LAP.
pub fn messages(b: &mut Batch, d: Glass, msgs: &[Msg]) {
    let mut y = MSG_TOP;
    for m in msgs {
        let h = match m {
            Msg::Lap { best, ms, pb } => {
                let (label, col) = if *best { ("BEST LAP", G_ORANGE) } else { ("LAST LAP", 0xffffff) };
                title(b, d, y, label, col, Some(glass_time(*ms)), *pb);
                msg_title_h()
            }
            Msg::FinalLap => {
                title(b, d, y, "FINAL LAP", G_ORANGE, None, false);
                msg_title_h()
            }
            Msg::Eliminated => {
                title(b, d, y, "ELIMINATED", G_ORANGE, None, false);
                msg_title_h()
            }
            Msg::Notice(text) => {
                note(b, d, y, text);
                normal_line(HudFont::Glass, NOTE_PX)
            }
            Msg::Finished { pos, of, last, total } => {
                title(b, d, y, &format!("FINISHED {pos} / {of}"), G_ORANGE, None, false);
                sub(b, d, y + msg_title_h() + MSG_SUB_GAP, &[("LAST LAP", glass_time(*last)), ("RACE TIME", glass_time(*total))]);
                msg_title_h() + MSG_SUB_GAP + msg_sub_h()
            }
        };
        y += h + MSG_GAP;
    }
}

/// `.cmsg` at design y `y`: the label, then (`.num`, margin 16) the time in white and (`.tag-v`, margin 14) the PB pill.
fn title(b: &mut Batch, d: Glass, y: f32, label: &str, col: u32, time: Option<String>, pb: bool) {
    let st = d.font(HudFont::Glass, 30.0, 3.0, hex(col, 1.0));
    let em = d.d(30.0);
    let tag = d.font(HudFont::Glass, 10.0, 1.5, hex(G_ORANGE, 1.0));
    let tag_w = b.font_width(HudFont::Glass, "PB", &tag) + d.d(18.0);
    let lw = b.font_width(HudFont::Glass, label, &st);
    let tw = time.as_ref().map_or(0.0, |t| mono_width(t, em));
    let total = lw + time.as_ref().map_or(0.0, |_| d.d(16.0) + tw) + if pb { d.d(14.0) + tag_w } else { 0.0 };
    let x = d.cw * 0.5 - total * 0.5;
    let top = d.cap_top(HudFont::Glass, 30.0, d.t(y), None);
    let glow = if col == G_ORANGE { [1.0, 154.0 / 255.0, 60.0 / 255.0, 0.3] } else { [190.0 / 255.0, 230.0 / 255.0, 1.0, 0.26] };
    // APPROX: the 18 px glow as a soft pad behind the line, the `0 2px 4px` shadow as a dark copy (as the cluster)
    let mut m = Mesh::default();
    m.soft_pad(&vec::rounded_rect(x + d.d(4.0), top + d.d(4.0), total - d.d(8.0), st.size - d.d(8.0), d.d(8.0)), d.d(18.0), glow);
    b.mesh(&m);
    b.font_text(HudFont::Glass, label, x, top + d.d(2.0), &FontStyle { color: [0.0, 0.0, 0.0, 0.4], ..st });
    b.font_text(HudFont::Glass, label, x, top, &st);
    let mut cx = x + lw;
    if let Some(t) = &time {
        cx += d.d(16.0);
        // on the label's baseline
        let ty = d.cap_above(HudFont::Clock, 30.0, top + st.size);
        let num = d.font(HudFont::Clock, 30.0, 0.0, WHITE);
        mono(b, HudFont::Clock, t, cx, ty + d.d(2.0), &FontStyle { color: [0.0, 0.0, 0.0, 0.4], ..num }, em);
        mono(b, HudFont::Clock, t, cx, ty, &num, em);
        cx += tw;
    }
    if pb {
        // `.tag-v`: 18 tall, r 7, 1 px orange .6 rim over `--tag-back`, raised 6
        cx += d.d(14.0);
        let pill = vec::rounded_rect(cx, top + st.size * 0.5 - d.d(9.0 + 3.0), tag_w, d.d(18.0), d.d(7.0));
        let mut m = Mesh::default();
        m.fill(&pill, hex(0x0a1626, 1.0), d.px);
        m.stroke(&pill, true, d.w(1.0), vec::Ends::Round, hex(G_ORANGE, 0.6), d.px);
        b.mesh(&m);
        b.font_text(HudFont::Glass, "PB", cx + d.d(9.0), d.cap_above(HudFont::Glass, 10.0, top + st.size * 0.5 - d.d(3.0) + tag.size * 0.5), &tag);
    }
}

/// `.cmsg.note` (#345): Glass 24 / 4 in white with the lap messages' glow, centred at design y `y`.
const NOTE_PX: f32 = 24.0;

fn note(b: &mut Batch, d: Glass, y: f32, text: &str) {
    let st = d.font(HudFont::Glass, NOTE_PX, 4.0, WHITE);
    let tw = b.font_width(HudFont::Glass, text, &st);
    let x = d.cw * 0.5 - tw * 0.5;
    let top = d.cap_top(HudFont::Glass, NOTE_PX, d.t(y), None);
    // APPROX: the 18 px glow as a soft pad behind the line, the `0 2px 4px` shadow as a dark copy (as `title`)
    let mut m = Mesh::default();
    m.soft_pad(&vec::rounded_rect(x + d.d(4.0), top, tw - d.d(8.0), st.size, d.d(8.0)), d.d(18.0), [190.0 / 255.0, 230.0 / 255.0, 1.0, 0.26]);
    b.mesh(&m);
    b.font_text(HudFont::Glass, text, x, top + d.d(2.0), &FontStyle { color: [0.0, 0.0, 0.0, 0.4], ..st });
    b.font_text(HudFont::Glass, text, x, top, &st);
}

/// `.cmsg .sub` at design y `y`: caption (Glass 14 / 2 `text`), time (Audiowide 16 white), the pairs 24 px apart.
fn sub(b: &mut Batch, d: Glass, y: f32, pairs: &[(&str, String)]) {
    let cap = d.font(HudFont::Glass, 14.0, 2.0, hex(G_TEXT, 1.0));
    let num = d.font(HudFont::Clock, 16.0, 0.0, WHITE);
    let em = d.d(16.0);
    let widths: Vec<(f32, f32)> = pairs.iter().map(|(l, t)| (b.font_width(HudFont::Glass, l, &cap), mono_width(t, em))).collect();
    let total: f32 = widths.iter().map(|(l, t)| l + d.d(8.0) + t).sum::<f32>() + d.d(24.0) * (pairs.len() as f32 - 1.0);
    let top = d.cap_top(HudFont::Glass, 14.0, d.t(y), None);
    let mut x = d.cw * 0.5 - total * 0.5;
    for ((label, time), (lw, tw)) in pairs.iter().zip(widths) {
        shadowed(b, HudFont::Glass, label, x, top, &cap, d.d(1.0));
        x += lw + d.d(8.0);
        let ty = d.cap_above(HudFont::Clock, 16.0, top + cap.size);
        mono(b, HudFont::Clock, time, x, ty + d.d(1.0), &FontStyle { color: [0.0, 0.0, 0.0, 0.3], ..num }, em);
        mono(b, HudFont::Clock, time, x, ty, &num, em);
        x += tw + d.d(24.0);
    }
}

/// WRONG WAY (`.msg.warn`): a warning triangle and Glass 40 / 2 in red with a red glow, centred at design y 280.
/// The 200 ms blink is the caller's.
pub fn wrong_way(b: &mut Batch, d: Glass) {
    let red = hex(0xff5a4a, 1.0);
    let st = d.font(HudFont::Glass, 40.0, 2.0, red);
    let text = "WRONG WAY";
    let tw = b.font_width(HudFont::Glass, text, &st);
    let icon = ui_font::icon::ALERT.to_string();
    let size = d.d(36.0);
    let isz = size * ui_font::face(HudFont::Icons).cap / ui_font::SRC;
    let iw = b.font_width(HudFont::Icons, &icon, &FontStyle::new(isz, red)).max(size);
    let gap = d.d(10.0);
    let total = iw + gap + tw;
    let x = d.cw * 0.5 - total * 0.5;
    let top = d.cap_top(HudFont::Glass, 40.0, d.t(280.0), None);
    let base = top + st.size;
    let mut m = Mesh::default();
    m.soft_pad(&vec::rounded_rect(x + d.d(4.0), top + d.d(4.0), total - d.d(8.0), st.size - d.d(8.0), d.d(10.0)), d.d(18.0), [1.0, 60.0 / 255.0, 40.0 / 255.0, 0.3]);
    b.mesh(&m);
    // the icon's bottom sits 6 under the baseline (`vertical-align: -6px`)
    let itop = base + d.d(6.0) - size;
    b.font_text(HudFont::Icons, &icon, x, itop + size - isz + d.d(2.0), &FontStyle::new(isz, [0.0, 0.0, 0.0, 0.4]));
    b.font_text(HudFont::Icons, &icon, x, itop + size - isz, &FontStyle::new(isz, red));
    b.font_text(HudFont::Glass, text, x + iw + gap, top + d.d(2.0), &FontStyle { color: [0.0, 0.0, 0.0, 0.4], ..st });
    b.font_text(HudFont::Glass, text, x + iw + gap, top, &st);
}

// ---------------------------------------------------------------------------------------------
// tribute to tron scroller (#320 phase 2)

/// One line of the Tron greeting in the Glass face (design choice; the original's myfont is Classic only), its line box
/// at canvas `y`, the left inset of the original's layout. It rises into the band through the soft edge `y0..y1`
/// (canvas px): fully clear above `y0`, full at `y1`, instead of the Classic hard cut.
pub fn tron_line(b: &mut Batch, d: Glass, text: &str, y: f32, y0: f32, y1: f32) {
    const PX: f32 = 22.0;
    let mid = y + d.d(PX) * 0.5;
    let t = ((mid - y0) / (y1 - y0)).clamp(0.0, 1.0);
    let a = t * t * (3.0 - 2.0 * t);
    if a <= 0.0 {
        return;
    }
    let st = d.font(HudFont::Glass, PX, 2.0, [1.0, 1.0, 1.0, a]);
    let top = d.cap_top(HudFont::Glass, PX, y, None);
    shadowed(b, HudFont::Glass, text, d.d(48.0), top, &st, d.d(1.0));
}

// ---------------------------------------------------------------------------------------------
// Record / Ghost / Demo tags (#345, mockup C `.mt`)

/// What the race does besides racing: Record race writes a `.MDE` (REC), the recorded cars play beside you (GHOST),
/// a recorded demo is watched (DEMO).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Mode {
    Rec,
    Ghost,
    Demo,
}

/// Design px of `.mt`: the row's top and line height, the gap between its parts, the hairlines' length,
/// the icon's gap to its label, `text-indent`, the divider's padding, the REC dot's period (s).
const MODE_TOP: f32 = 14.0;
const MODE_LINE: f32 = 22.0;
const MODE_GAP: f32 = 18.0;
const MODE_RULE: f32 = 70.0;
const MODE_ICON_GAP: f32 = 8.0;
const MODE_INDENT: f32 = 4.0;
const MODE_DIVIDER: f32 = 19.0;
const MODE_DOT: f32 = 11.0;
const MODE_PULSE: f32 = 1.2;

impl Mode {
    /// label, text colour, glow (rgb of the text-shadow, alpha as the lap messages' glow)
    fn look(self) -> (&'static str, u32, [f32; 4]) {
        match self {
            Mode::Rec => ("REC", 0xff8a7a, [1.0, 80.0 / 255.0, 60.0 / 255.0, 0.3]),
            Mode::Ghost => ("GHOST", 0x8fd0ff, [120.0 / 255.0, 200.0 / 255.0, 1.0, 0.3]),
            Mode::Demo => ("DEMO", 0xf4fbff, [190.0 / 255.0, 230.0 / 255.0, 1.0, 0.26]),
        }
    }

    /// The icon's glyph; REC has a dot instead.
    fn glyph(self) -> Option<char> {
        match self {
            Mode::Rec => None,
            Mode::Ghost => Some(ui_font::icon::GHOST),
            Mode::Demo => Some(ui_font::icon::VIDEO),
        }
    }
}

/// The tags, top centre between two hairlines (`.mt`): an icon (REC: a red dot that pulses, `t` = the clock) and the label
/// in Glass 18 / 4, a divider between two tags. Drawn at design y 14, above the message stack.
pub fn mode_tags(b: &mut Batch, d: Glass, t: f32, modes: &[Mode]) {
    if modes.is_empty() {
        return;
    }
    let style = |m: Mode| d.font(HudFont::Glass, 18.0, 4.0, hex(m.look().1, 1.0));
    let lead = |m: Mode| d.d(if m == Mode::Rec { MODE_DOT } else { MODE_LINE });
    // one tag's width: the icon, its gap, the indent and the label (whose width holds the trailing tracking, as in CSS)
    let width = |m: Mode| lead(m) + d.d(MODE_ICON_GAP + MODE_INDENT) + b.font_width(HudFont::Glass, m.look().0, &style(m));
    let n = modes.len() as f32;
    let tags: f32 = modes.iter().map(|&m| width(m)).sum::<f32>() + (n - 1.0) * (d.d(MODE_GAP + MODE_DIVIDER));
    let total = tags + 2.0 * d.d(MODE_RULE + MODE_GAP);
    let x0 = d.cw * 0.5 - total * 0.5;
    let (top, h) = (d.t(MODE_TOP), d.d(MODE_LINE));
    let mid = top + h * 0.5;
    let line = d.w(1.0);
    let rule = d.d(MODE_RULE);
    let mut m = Mesh::default();
    // the hairlines fade out away from the tags
    for (x, near_right) in [(x0, true), (x0 + total - rule, false)] {
        let ramp = move |q: [f32; 2]| {
            let k = ((q[0] - x) / rule).clamp(0.0, 1.0);
            [190.0 / 255.0, 230.0 / 255.0, 1.0, 0.55 * if near_right { k } else { 1.0 - k }]
        };
        m.fill_with(&vec::rounded_rect(x, mid - line * 0.5, rule, line, line * 0.5), &ramp, d.px);
    }
    let mut x = x0 + rule + d.d(MODE_GAP);
    let mut labels = Vec::new();
    for (i, &mode) in modes.iter().enumerate() {
        if i > 0 {
            m.stroke(&[[x, top], [x, top + h]], false, line, vec::Ends::Butt, hex(G_LINE, 0.28), d.px);
            x += d.d(MODE_DIVIDER);
        }
        let (label, col, glow) = mode.look();
        let w = width(mode);
        // APPROX: the 12 px text-shadow glow as a soft pad behind the tag
        m.soft_pad(&vec::rounded_rect(x + d.d(4.0), top, w - d.d(8.0), h, d.d(8.0)), d.d(12.0), glow);
        if mode == Mode::Rec {
            // `.dot`: 11 px #ff4a3a with a glow, opacity 1 -> .25 -> 1 over 1.2 s
            let a = 1.0 - 0.75 * (0.5 - 0.5 * (t * TAU / MODE_PULSE).cos());
            let c = [x + d.d(MODE_DOT * 0.5), mid];
            m.halo(&circle(c, d.d(MODE_DOT * 0.5)), d.d(10.0), [1.0, 74.0 / 255.0, 58.0 / 255.0, 0.6 * a]);
            m.fill(&circle(c, d.d(MODE_DOT * 0.5)), [1.0, 74.0 / 255.0, 58.0 / 255.0, a], d.px);
        }
        labels.push((mode, label, hex(col, 1.0), x));
        x += w + d.d(MODE_GAP);
    }
    b.mesh(&m);
    let size = d.d(MODE_LINE);
    let isz = size * ui_font::face(HudFont::Icons).cap / ui_font::SRC;
    let text_top = d.cap_top(HudFont::Glass, 18.0, top, Some(MODE_LINE));
    for (mode, label, col, x) in labels {
        if let Some(g) = mode.glyph() {
            let g = g.to_string();
            let iy = top + (size - isz) * 0.5;
            b.font_text(HudFont::Icons, &g, x, iy + d.d(1.0), &FontStyle::new(isz, [0.0, 0.0, 0.0, 0.4]));
            b.font_text(HudFont::Icons, &g, x, iy, &FontStyle::new(isz, col));
        }
        let tx = x + lead(mode) + d.d(MODE_ICON_GAP + MODE_INDENT);
        let st = style(mode);
        b.font_text(HudFont::Glass, label, tx, text_top + d.d(1.0), &FontStyle { color: [0.0, 0.0, 0.0, 0.4], ..st });
        b.font_text(HudFont::Glass, label, tx, text_top, &st);
    }
}

// ---------------------------------------------------------------------------------------------
// the name plates (`plateHTML`, `.npl`)

/// A rival's plate: where the camera sees the point 1.8 m over the car (canvas px; the card's bottom centre),
/// the distance along the view (m), the rival's place and driver.
pub struct Plate {
    pub x: f32,
    pub y: f32,
    pub z: f32,
    pub place: u32,
    pub colour: [u8; 3],
    pub name: String,
}

/// Name plates "See-through" = a glass card (`.npl.glass`), "Flat" = place, bar and name as text with a shadow
/// (`.npl.flat`), `Bare` = See-through with its frame switched off (HUD option "Plate frame"): the colour bar and
/// the name only [design choice].
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PlateStyle {
    Glass,
    Flat,
    Bare,
}

impl PlateStyle {
    /// whether the place number shows
    fn place(self) -> bool {
        self != PlateStyle::Bare
    }
}

/// The plates are drawn at this fraction of the mockup's size [design choice: 25 % smaller than the mockup].
const PLATE_SIZE: f32 = 0.75;

/// The distances the game shows plates at (m); `far` runs 0..1 between them.
const PLATE_NEAR: f32 = 4.0;
const PLATE_FAR: f32 = 140.0;
/// The mockup's card: height, corner radius (the Flat style keeps the height), `--name` text colour.
const PLATE_H: f32 = 24.0;
const PLATE_NAME: u32 = 0xf4fbff;

/// (scale, opacity) of a plate `z` m away: it shrinks by up to 28 % and loses up to half its opacity (mockup values).
pub fn plate_fade(z: f32) -> (f32, f32) {
    let far = ((z - PLATE_NEAR) / (PLATE_FAR - PLATE_NEAR)).clamp(0.0, 1.0);
    (1.0 - 0.28 * far, 1.0 - 0.5 * far)
}

/// A plate's card on the canvas.
#[derive(Clone, Copy, Debug)]
struct PlateBox {
    x: f32,
    y: f32,
    w: f32,
    h: f32,
}

/// Whether `a` and `b` overlap by more than half of the smaller one.
fn covered(a: &PlateBox, b: &PlateBox) -> bool {
    let w = (a.x + a.w).min(b.x + b.w) - a.x.max(b.x);
    let h = (a.y + a.h).min(b.y + b.h) - a.y.max(b.y);
    w > 0.0 && h > 0.0 && w * h > 0.5 * (a.w * a.h).min(b.w * b.h)
}

/// Extra points along the edges (every `step`), so a per-vertex gradient follows across a long straight side.
fn dense(pts: &[[f32; 2]], step: f32) -> Vec<[f32; 2]> {
    let mut out = Vec::new();
    for (i, a) in pts.iter().enumerate() {
        let c = pts[(i + 1) % pts.len()];
        let n = ((c[0] - a[0]).hypot(c[1] - a[1]) / step).ceil().max(1.0) as usize;
        out.extend((0..n).map(|k| {
            let t = k as f32 / n as f32;
            [a[0] + (c[0] - a[0]) * t, a[1] + (c[1] - a[1]) * t]
        }));
    }
    out
}

/// The card of `p` at scale `sc`, the widths of its place and name beside it: `.npl` is a row of padding, the place,
/// 8, the 3 px bar, 8, the name and padding; the card's bottom centre sits on the plate's point (`translate(-50%, -100%)`).
fn plate_box(b: &Batch, d: Glass, p: &Plate, style: PlateStyle, sc: f32) -> (PlateBox, f32, f32) {
    let pl = if style.place() { b.font_width(HudFont::Clock, &p.place.to_string(), &d.font(HudFont::Clock, 11.0 * sc, 0.0, WHITE)) } else { 0.0 };
    let nm = b.font_width(HudFont::Glass, &p.name, &d.font(HudFont::Glass, 15.0 * sc, 0.6 * sc, WHITE));
    let pad = if style == PlateStyle::Glass { 9.0 + 12.0 } else { 12.0 };
    // the gap after the place only where there is a place
    let w = d.d((pad + if style.place() { 8.0 } else { 0.0 } + 3.0 + 8.0) * sc) + pl + nm;
    let h = d.d(PLATE_H * sc);
    (PlateBox { x: p.x - w * 0.5, y: p.y - h, w, h }, pl, nm)
}

/// The rivals' plates, far to near so a near one lies over a far one; a far plate that a nearer one covers by more
/// than half is left out. They are drawn first of the HUD, under its groups and messages.
pub fn plates(b: &mut Batch, d: Glass, style: PlateStyle, mut list: Vec<Plate>) {
    list.sort_by(|a, c| a.z.total_cmp(&c.z));
    let (cw, ch) = (b.canvas.w, b.canvas.h);
    let mut kept: Vec<(PlateBox, usize, f32, f32, f32, f32)> = Vec::new();
    for (i, p) in list.iter().enumerate() {
        let (sc, op) = plate_fade(p.z);
        let sc = sc * PLATE_SIZE;
        let (bx, pl, nm) = plate_box(b, d, p, style, sc);
        if bx.x + bx.w < 0.0 || bx.x > cw || bx.y + bx.h < 0.0 || bx.y > ch || kept.iter().any(|k| covered(&k.0, &bx)) {
            continue;
        }
        kept.push((bx, i, sc, op, pl, nm));
    }
    for (bx, i, sc, op, pl, _) in kept.into_iter().rev() {
        draw_plate(b, d, &list[i], style, &bx, sc, op, pl);
    }
}

/// One plate: `.npl` with the colour `--c` of the driver. APPROX: the mockup's `backdrop-filter` blur is the tint and
/// the sheen (no per-plate blur pass); the CSS text shadows are dark copies and a soft pad.
#[allow(clippy::too_many_arguments)]
fn draw_plate(b: &mut Batch, d: Glass, p: &Plate, style: PlateStyle, bx: &PlateBox, sc: f32, op: f32, pl_w: f32) {
    let u = |v: f32| d.d(v) * sc;
    let w1 = u(1.0).max(d.px);
    let col = (u32::from(p.colour[0]) << 16) | (u32::from(p.colour[1]) << 8) | u32::from(p.colour[2]);
    let (x, y, w, h) = (bx.x, bx.y, bx.w, bx.h);
    let glass = style == PlateStyle::Glass;
    let mut m = Mesh::default();
    if glass {
        let r = u(12.0);
        let body = vec::rounded_rect(x, y, w, h, r);
        m.shadow(&body, 0.0, u(8.0), u(18.0), [0.0, 0.0, 0.0, 0.4 * op]);
        m.halo(&body, u(14.0), hex(col, 0.22 * op));
        // the fill: the sheen (white .20, .05 at 55 %, navy .22) under a wash of the driver's colour (30 %, from the left)
        let c = hex(col, 1.0);
        let shade = |q: [f32; 2]| -> [f32; 4] {
            let ty = ((q[1] - y) / h).clamp(0.0, 1.0);
            let (sheen, sa) = if ty < 0.55 {
                ([1.0; 3], 0.20 + (0.05 - 0.20) * ty / 0.55)
            } else {
                let t = (ty - 0.55) / 0.45;
                let navy = [6.0 / 255.0, 14.0 / 255.0, 28.0 / 255.0];
                ([1.0 + (navy[0] - 1.0) * t, 1.0 + (navy[1] - 1.0) * t, 1.0 + (navy[2] - 1.0) * t], 0.05 + (0.22 - 0.05) * t)
            };
            let t = (((q[0] - x) / (0.7 * w)).powi(2) + ((q[1] - (y + h * 0.5)) / (1.6 * h)).powi(2)).sqrt();
            let aw = 0.30 * (1.0 - t / 0.7).max(0.0);
            let a = aw + sa * (1.0 - aw);
            let mix = |k: usize| (c[k] * aw + sheen[k] * sa * (1.0 - aw)) / a;
            [mix(0), mix(1), mix(2), a * op]
        };
        m.fill_with(&dense(&body, u(3.0)), &shade, d.px);
        m.stroke(&body, true, w1, vec::Ends::Round, hex(G_LINE, 0.38 * op), d.px);
        m.stroke(&[[x + r, y + h - u(1.5)], [x + w - r, y + h - u(1.5)]], false, w1, vec::Ends::Butt, [1.0, 1.0, 1.0, 0.08 * op], d.px);
        // the stem (1 x 9, `line` .7 fading to .1) and the 5 px dot 8 px under the card
        let cx = p.x;
        for (k, a) in [0.6, 0.4, 0.2].into_iter().enumerate() {
            let y0 = y + h + u(3.0) * k as f32;
            m.stroke(&[[cx, y0], [cx, y0 + u(3.0)]], false, w1, vec::Ends::Butt, hex(G_LINE, a * op), d.px);
        }
        let dot = [cx, y + h + u(8.0 + 2.5)];
        m.disc(dot, u(8.0), hex(col, 0.35 * op), hex(col, 0.0), 24);
        m.disc(dot, u(2.5), hex(col, op), hex(col, op), 16);
    } else {
        // Flat: a soft dark pad behind the text keeps it readable on bright ground
        m.soft_pad(&vec::rounded_rect(x, y + u(4.0), w, h - u(8.0), u(8.0)), u(8.0), [0.0, 0.0, 0.0, 0.45 * op]);
    }
    // the colour bar (3 x 15, 3 x 12 when flat) with its glow
    let bh = if glass { 15.0 } else { 12.0 };
    let pad = if glass { 9.0 } else { 6.0 };
    let bar_x = x + u(pad + if style.place() { 8.0 } else { 0.0 }) + pl_w;
    let bar = vec::rounded_rect(bar_x, y + (h - u(bh)) * 0.5, u(3.0), u(bh), u(1.5));
    m.soft_pad(&bar, u(8.0), hex(col, 0.6 * op));
    m.fill(&bar, hex(col, op), d.px);
    b.mesh(&m);
    // the place, then the name (Glass 15 / .6 in `--name`)
    if style.place() {
        let place = d.font(HudFont::Clock, 11.0 * sc, 0.0, hex(G_DIM, op));
        b.font_text(HudFont::Clock, &p.place.to_string(), x + u(pad), d.cap_top(HudFont::Clock, 11.0 * sc, y, Some(PLATE_H * sc)), &place);
    }
    let name = d.font(HudFont::Glass, 15.0 * sc, 0.6 * sc, hex(PLATE_NAME, op));
    let (nx, ny) = (bar_x + u(3.0 + 8.0), d.cap_top(HudFont::Glass, 15.0 * sc, y, Some(PLATE_H * sc)));
    let shade = if glass { 0.35 } else { 0.9 };
    b.font_text(HudFont::Glass, &p.name, nx, ny + u(1.0), &FontStyle { color: [0.0, 0.0, 0.0, shade * op], ..name });
    b.font_text(HudFont::Glass, &p.name, nx, ny, &name);
}

fn smooth(e0: f32, e1: f32, x: f32) -> f32 {
    let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pixel_runs_split_by_shade_and_skip_the_key() {
        // 3 x 2: row 0 = white white black, row 1 = clear white clear
        let px = |r: u8, a: u8| [r, r, r, a];
        let img: Vec<u8> = [px(255, 255), px(255, 255), px(0, 255), px(0, 0), px(255, 255), px(0, 0)].concat();
        assert_eq!(pixel_runs(&img, 3, 2), vec![(0, 0, 2, true), (0, 2, 3, false), (1, 1, 2, true)]);
    }

    #[test]
    fn stack_closes_up_when_pieces_hide() {
        let all = stack_size(3, true, true);
        assert!(all.0 > 0.0 && all.1 > 0.0);
        assert!(stack_size(3, false, false).1 < all.1);
        assert!(stack_size(0, true, true).1 < all.1);
        assert!(stack_size(0, true, false).1 < stack_size(0, true, true).1);
        assert_eq!(stack_size(0, false, false), (0.0, 0.0));
        // the stack fits the corner: clear of the map below (480 canvas tall) at 100 %
        assert!(all.1 + 8.0 < 240.0, "{all:?}");
    }

    #[test]
    fn mono_cells_keep_a_time_the_same_width() {
        let em = 10.0;
        assert_eq!(mono_width("01:47.326", em), mono_width("11:11.111", em));
        assert_eq!(mono_width("--:--.---", em), mono_width("00:00.000", em));
    }

    #[test]
    fn clip_to_disc_cuts_on_the_circle() {
        let c = [0.0, 0.0];
        // a chord through the disc: one piece from rim to rim
        let one = clip_to_disc(&[[-3.0, 0.0], [3.0, 0.0]], c, 1.0);
        assert_eq!(one.len(), 1);
        assert!((one[0][0][0] + 1.0).abs() < 1e-4 && (one[0][1][0] - 1.0).abs() < 1e-4);
        // a path that leaves and comes back is two pieces; one that stays inside stays whole and joined
        let two = clip_to_disc(&[[-0.5, 0.0], [2.0, 0.0], [2.0, 0.5], [0.5, 0.5], [0.5, -0.5]], c, 1.0);
        assert_eq!(two.len(), 2, "{two:?}");
        let whole = clip_to_disc(&[[-0.5, 0.0], [0.0, 0.5], [0.5, 0.0]], c, 1.0);
        assert_eq!((whole.len(), whole[0].len()), (1, 3));
        // all outside
        assert!(clip_to_disc(&[[2.0, 2.0], [3.0, 2.0]], c, 1.0).is_empty());
        for piece in two.iter().flatten() {
            assert!(piece[0].hypot(piece[1]) <= 1.0 + 1e-4);
        }
    }

    #[test]
    fn map_box_clears_the_stack() {
        let (w, h) = map_size();
        assert!(w > 100.0 && h > 100.0, "{w} {h}");
        // stack + map + the gaps fit the 480 canvas at 100 %
        assert!(8.0 + stack_size(3, true, true).1 + 16.0 + h + MAP_INSET.1 <= 480.0);
        // the reach covers the disc, its tick ring and the N chip at any angle
        let d = Glass::hud(853.0, 1.0);
        assert!(d.d(MAP_REACH) >= d.d(MAP_RING + 10.0));
    }

    #[test]
    fn gaps_read_like_the_mockup() {
        assert_eq!(glass_gap(-1214), "-1.214");
        assert_eq!(glass_gap(678), "+0.678");
        assert_eq!(glass_gap(65_204), "+1:05.204");
        // the live gaps (#302): laps and dashes
        assert_eq!(RowGap::Ms(-678).text(), "-0.678");
        assert_eq!(RowGap::Laps(-1).text(), "-1 LAP");
        assert_eq!(RowGap::Laps(2).text(), "+2 LAPS");
        assert_eq!(RowGap::Dashes.text(), "--.---");
        // colours: the checkpoint list is green ahead / red behind, the live gaps the other way round
        assert_eq!(RowGap::Ms(-100).colour(false), RowGap::Ms(100).colour(true));
        assert_eq!(RowGap::Ms(100).colour(false), RowGap::Laps(-1).colour(true));
        assert_ne!(RowGap::Ms(-100).colour(true), RowGap::Ms(-100).colour(false));
        assert_eq!(RowGap::Dashes.colour(true), RowGap::Dashes.colour(false));
        assert_eq!(splits_size(0), (0.0, 0.0));
        assert!(splits_size(8).1 > splits_size(3).1);
    }

    #[test]
    fn plates_fade_with_distance() {
        assert_eq!(plate_fade(4.0), (1.0, 1.0));
        let (s, o) = plate_fade(140.0);
        assert!((s - 0.72).abs() < 1e-5 && (o - 0.5).abs() < 1e-5);
        assert_eq!(plate_fade(1.0), (1.0, 1.0));
        assert_eq!(plate_fade(500.0), plate_fade(140.0));
    }

    #[test]
    fn plates_that_stack_are_culled() {
        let a = PlateBox { x: 100.0, y: 100.0, w: 60.0, h: 17.0 };
        // mostly on top of `a`: covered; side by side: not
        assert!(covered(&a, &PlateBox { x: 110.0, y: 104.0, w: 60.0, h: 17.0 }));
        assert!(!covered(&a, &PlateBox { x: 170.0, y: 100.0, w: 60.0, h: 17.0 }));
        // a corner overlap of less than half stays
        assert!(!covered(&a, &PlateBox { x: 140.0, y: 110.0, w: 60.0, h: 17.0 }));
    }
}
