//! REMASTER Personal screen in the Glass HUD (#91, concept C · Paddock of
//! `reference_materials/personal-overhaul/C-paddock.html`, styles in its `assets/personal.css`): the pause
//! menu's tab bar over five purpose-built bodies. Phases 1-6: the frame, the tab bar, the Driver tab with typing
//! in place, the Livery tab with its palette picker, the Chat keys tab, the car previews in their live paint, the
//! pad keyboard, the Career tab from stats.cfg and the Records tab on the Glass table (table.rs). Design px through
//! `Glass`, the rules are 19-glass-hud-design-language.

use super::pause_settings::{cap_body, line_h, wrap_text, PauseItem, PauseRow, PauseVal};
use super::table::{Cell, GlassTable, TableCol, TableRow};
use super::*;

/// The tabs: name and icon.
const TABS: [(&str, char); 5] = [
    ("DRIVER", ui_font::icon::USER),
    ("LIVERY", ui_font::icon::PAINT_BUCKET),
    ("CHAT KEYS", ui_font::icon::MESSAGE_SQUARE),
    ("CAREER", ui_font::icon::TROPHY),
    ("RECORDS", ui_font::icon::BAR_CHART),
];
const DRIVER: usize = 0;
const LIVERY: usize = 1;
const CHAT: usize = 2;
const CAREER: usize = 3;
const RECORDS: usize = 4;

/// The Driver tab's rows as the Classic screen numbers them (`PERSONAL_ROWS`): Name, Team, Colour,
/// Confirmations, Intro movies. The row ids are what `Game::editing` holds.
const DRIVER_ROWS: [usize; 5] = [0, 1, 2, 17, 18];

/// The cards of the tabs (design px): the Driver tab's rows card and its on-track card, and the full-width body.
const DRIVER_CARD: (f32, f32, f32, f32) = (62.0, 228.0, 560.0, 372.0);
const TRACK_CARD: (f32, f32, f32, f32) = (642.0, 228.0, 576.0, 372.0);
/// The Livery tab (`liveryHTML`, its #135 state): the slot cards (Custom paint, Paint 1-3), 270 x 64 from (62, 228),
/// 77 apart; the palette card, widened for the 4 x 20 grid; the disc's centre and bezel radius, 40 px left of the
/// Career tab's to make room [design choice].
const LIVERY_CARDS: usize = 4;
const SLOT: (f32, f32, f32, f32) = (62.0, 228.0, 270.0, 64.0);
const SLOT_STEP: f32 = 77.0;
const PALETTE_CARD: (f32, f32, f32, f32) = (776.0, 228.0, 442.0, 372.0);
const LIVERY_DISC: ((f32, f32), f32) = ((572.0, 414.0), 150.0);
const CAREER_DISC: ((f32, f32), f32) = ((612.0, 414.0), 150.0);
/// The palette grid (#135): 4 rows of 20 swatches, 17 px, 3 apart, from (22, 80) in the card.
const PAL_COLS: usize = 20;
const PAL_ROWS: usize = 4;
const PAL_CELL: f32 = 17.0;
const PAL_GAP: f32 = 3.0;
/// The Chat keys tab (`chatHTML`): ten cards 571 x 60, two columns 585 apart, rows 68 apart from (62, 228).
const CHAT_KEYS: usize = 10;
const KEY_CARD: (f32, f32) = (571.0, 60.0);
/// The Career tab (`careerHTML`): eight stat cards 84 px high, 96 apart from y 228, the left four 270 wide at x 62, the
/// right four 326 wide at x 892; the win disc (the Livery disc's place) with its arc 18 px outside the bezel.
const CAREER_CARDS: usize = 8;
const STAT_H: f32 = 84.0;
const STAT_STEP: f32 = 96.0;
const WIN_ARC_R: f32 = 168.0;
/// The Records tab (`recordsHTML`): one panel over the body, the subtab row at (18, 10) in it, the headings at y 42, the
/// rows from y 62, 36 px high for the circuits and 29.4 for the cars.
const RECORDS_PANEL: (f32, f32, f32, f32) = (62.0, 228.0, 1156.0, 372.0);
const SUBTABS: [&str; 2] = ["CIRCUITS", "CARS"];
const RECORD_ROW_H: [f32; 2] = [36.0, 29.4];

/// Help texts of the Driver rows (`HELP` of the mockup, in the game's voice; [O] where they state what the game does).
const HELP_NAME: &str = "Your name in races, on the results and in the rankings. Up to 15 characters.";
const HELP_TEAM: &str = "A short team tag after your name, as in \"SIN (DICE)\". Up to 4 characters.";
const HELP_COLOUR: &str = "Your driver colour: your dot on the map, your name plate and your row in the standings.";
// [O] only the exit requester reads the setting (front.rs, Main's Esc): `NoExit` and `Off` both skip it
const HELP_CONFIRM: &str = "Whether the game asks before it quits. On asks; Off and No exit do not.";
const HELP_INTRO: &str = "Play the intro movies when the game starts.";
const HELP_CUSTOM: &str = "Race in your own paint. Off: every car keeps its stock colours.";
// [O] 08-missing-features 8.5: the third zone is on three of the shipped cars
const HELP_PAINT: [&str; 3] = [
    "The main paint zone of every car.",
    "The second paint zone: stripes, sills or the lower body, depending on the car.",
    "A third zone that only the Kobra X3, Adder Mk2 and ASC II have.",
];
/// The palette's line on the finish tabs (#135), under the paint's help.
const HELP_FINISH: &str = "Gloss, metallic or pearl: the tabs over the grid. Every paint keeps its own finish.";
/// The confirmations as the modern screen names them (`CONFIRMATIONS` is the file's spelling).
const CONFIRM_NAMES: [&str; 3] = ["Off", "On", "No exit"];

/// `.note`: the small captions' paler blue.
const NOTE: [f32; 4] = [143.0 / 255.0, 181.0 / 255.0, 201.0 / 255.0, 0.75];

/// The sample field of the Driver card: the rivals' gaps and the player's, fixed numbers [design choice].
const SAMPLE_GAPS: [&str; 3] = ["+2.204", "+3.418", "+4.096"];

/// The car previews (09-remaster 9.39) [design choice]: a long lens (zoom 4 = about 28 deg across) for the
/// mockup's near-flat drawing, 20 deg above the car, at a 3/4 front view (50 deg off the nose, to its left). The
/// Livery car turns on from there at the Car screen's rate, its body's bounding sphere 6 px inside the disc's face
/// (r 134). The Driver card's stands still, as wide as the mockup's (270 px) with its wheels above STANDINGS.
pub(super) const PREVIEW_ZOOM: f32 = 4.0;
pub(super) const PREVIEW_PITCH: f32 = 0.35;
pub(super) const PREVIEW_ANGLE: f32 = 0.87;
pub(super) const LIVERY_TURN: f32 = 0.35;
const LIVERY_ACROSS: f32 = 256.0;
const DRIVER_CAR: ((f32, f32), f32) = ((823.0, 344.0), 330.0);
/// A car preview's pose (`Game::preview_pose`).
pub(super) struct PreviewPose {
    pub cam: Camera,
    pub cars: Vec<(usize, Mat4)>,
    pub viewport: [f32; 4],
    /// the body's model matrix (view space) and the road's height in model space
    pub model: Mat4,
    pub ground: f32,
    view: Mat4,
    /// tan of the lens's half angle, the square's centre and side in canvas px
    half: f32,
    centre: [f32; 2],
    across: f32,
}

impl PreviewPose {
    /// A point of the car's model space on the canvas (the viewport's square is the lens's whole width).
    pub fn project(&self, p: Vec3) -> [f32; 2] {
        let q = self.view.transform_point3(self.model.transform_point3(p));
        let k = 0.5 * self.across / (-q.z).max(1e-3) / self.half;
        [self.centre[0] + q.x * k, self.centre[1] - q.y * k]
    }
}

/// `.zone-lit`: the picked zone's brightness up to 1.45 and back, 1.1 s each way.
const ZONE_GAIN: f32 = 0.45;
const ZONE_PERIOD: f32 = 2.2;

/// A colour the swatch row offers: the strip cell it sets, its colour and its name.
struct Swatch {
    cell: usize,
    rgb: [u8; 3],
    name: &'static str,
}

/// The names of the strip's colours [design choice: a swatch needs words for colour-blind players]; a cell
/// takes the name of the nearest of these.
const SWATCH_NAMES: [([u8; 3], &str); 10] = [
    ([0x00, 0x00, 0x00], "Black"),
    ([0x3c, 0x7b, 0x3c], "Moss"),
    ([0x4e, 0x46, 0x0d], "Olive"),
    ([0x47, 0x0d, 0x0e], "Oxblood"),
    ([0x0d, 0x0c, 0x8c], "Navy"),
    ([0x3b, 0x8e, 0x47], "Green"),
    ([0x2a, 0x51, 0xfd], "Cobalt"),
    ([0xa3, 0xfa, 0xfa], "Ice"),
    ([0xc9, 0xa7, 0x26], "Gold"),
    ([0x62, 0x2e, 0xea], "Violet"),
];

fn dist2(a: [u8; 3], b: [u8; 3]) -> i32 {
    (0..3).map(|k| (a[k] as i32 - b[k] as i32).pow(2)).sum()
}

/// The cells of the colour strip a player can reach, as swatches. CLASSIC 0x45f54c picks the first of
/// equal cells, so the other black ones cannot be chosen; a near-black cell that remains (#0a0000) is folded
/// into black [design choice]. The default palette leaves ten.
fn strip_swatches(strip: &[[u8; 3]; STRIP_CELLS]) -> Vec<Swatch> {
    let near_black = |c: [u8; 3]| c.iter().all(|&v| v < 16);
    let mut out: Vec<Swatch> = Vec::new();
    for (cell, &rgb) in strip.iter().enumerate() {
        if strip_index(strip, rgb) != cell || (near_black(rgb) && out.iter().any(|s| near_black(s.rgb))) {
            continue;
        }
        let name = SWATCH_NAMES.iter().min_by_key(|(c, _)| dist2(*c, rgb)).map_or("", |(_, n)| n);
        out.push(Swatch { cell, rgb, name });
    }
    out
}

/// The swatch showing the colour `c`: the one of its nearest strip cell, the first (black) for a folded cell.
fn swatch_at(strip: &[[u8; 3]; STRIP_CELLS], swatches: &[Swatch], c: [u8; 3]) -> usize {
    let cell = strip_index(strip, c);
    swatches.iter().position(|s| s.cell == cell).unwrap_or(0)
}

/// The name a colour shows: its swatch's for a colour of the strip (the folded near-black too), CUSTOM for any
/// other [design choice: the chip shows the real colour, so a nearby swatch's name would disagree with it].
fn swatch_name(strip: &[[u8; 3]; STRIP_CELLS], swatches: &[Swatch], c: [u8; 3]) -> &'static str {
    if strip.contains(&c) {
        swatches[swatch_at(strip, swatches, c)].name
    } else {
        "Custom"
    }
}

/// Left / Right on a colour: the next or previous swatch from the nearest one (as CLASSIC 0x45f634 steps from the
/// nearest cell), stopping at the ends where the strip wrapped; the end sound when it cannot move.
fn swatch_step(strip: &[[u8; 3]; STRIP_CELLS], c: &mut [u8; 3], right: bool) -> Option<u32> {
    let sw = strip_swatches(strip);
    let at = swatch_at(strip, &sw, *c);
    match if right { (at + 1 < sw.len()).then_some(at + 1) } else { at.checked_sub(1) } {
        Some(to) => {
            *c = sw[to].rgb;
            None
        }
        None => Some(ev::CURSOR_END),
    }
}

/// The hues of the paint grid's columns 3..19, evenly round the circle from red.
const HUES: [&str; 17] = ["RED", "VERMILION", "ORANGE", "AMBER", "YELLOW", "LIME", "GREEN", "EMERALD", "TEAL", "CYAN", "SKY", "AZURE", "BLUE", "INDIGO", "VIOLET", "PURPLE", "MAGENTA"];

/// A swatch of the paint grid: its colour and its name.
pub(super) struct PaintSwatch {
    pub rgb: [u8; 3],
    pub name: String,
}

/// CSS `hsl()` in gamma space: hue in degrees, saturation and lightness 0..1.
fn hsl(h: f32, s: f32, l: f32) -> [u8; 3] {
    let a = s * l.min(1.0 - l);
    let f = |n: f32| {
        let k = (n + h / 30.0) % 12.0;
        l - a * (k - 3.0).min(9.0 - k).clamp(-1.0, 1.0)
    };
    [f(0.0), f(8.0), f(4.0)].map(|v| (v * 255.0).round() as u8)
}

/// REMASTER the paint grid (#135, the mockup's GRID135) [design choice]: 4 x 20 swatches, row by row. Columns 0..2 are
/// twelve neutrals from white to black (down each column, then across); columns 3..19 are the seventeen hues in four
/// shades by row: light, full, dark, deep. Gamma-space RGB, like every colour the game paints with.
pub(super) fn paint_grid() -> &'static [PaintSwatch] {
    static GRID: std::sync::OnceLock<Vec<PaintSwatch>> = std::sync::OnceLock::new();
    GRID.get_or_init(|| {
        let mut g = Vec::with_capacity(PAL_COLS * PAL_ROWS);
        for r in 0..PAL_ROWS {
            for c in 0..PAL_COLS {
                g.push(if c < 3 {
                    let k = c * 4 + r;
                    let v = 1.0 - k as f32 / 11.0;
                    let name = match k {
                        0 => "WHITE".to_string(),
                        11 => "BLACK".to_string(),
                        _ => format!("GREY {}", (v * 100.0).round()),
                    };
                    PaintSwatch { rgb: hsl(0.0, 0.0, v), name }
                } else {
                    let h = (c - 3) as f32 / HUES.len() as f32 * 360.0;
                    let rgb = hsl(h, [0.75, 0.85, 0.8, 0.7][r], [0.72, 0.52, 0.34, 0.2][r]);
                    PaintSwatch { rgb, name: format!("{}{}", ["LIGHT ", "", "DARK ", "DEEP "][r], HUES[c - 3]) }
                });
            }
        }
        g
    })
}

/// The grid swatch nearest to `c` (where the palette cursor opens: any RGB of settings.cfg still works).
fn grid_at(c: [u8; 3]) -> usize {
    paint_grid().iter().enumerate().min_by_key(|(_, s)| dist2(s.rgb, c)).map_or(0, |(i, _)| i)
}

/// A paint's name: its swatch's, CUSTOM for a colour off the grid (as `swatch_name`).
fn grid_name(c: [u8; 3]) -> &'static str {
    paint_grid().iter().find(|s| s.rgb == c).map_or("CUSTOM", |s| s.name.as_str())
}

/// Left / Right on a paint card: the next or previous grid swatch in reading order from the nearest one, stopping at
/// the ends with the end sound.
fn grid_step(c: &mut [u8; 3], right: bool) -> Option<u32> {
    let at = grid_at(*c);
    let n = paint_grid().len();
    match if right { (at + 1 < n).then_some(at + 1) } else { at.checked_sub(1) } {
        Some(to) => {
            *c = paint_grid()[to].rgb;
            None
        }
        None => Some(ev::CURSOR_END),
    }
}

/// Up / Down in a list of `n` rows: the ends stop, with the end sound (Classic's list does too).
fn step_rows(sel: &mut usize, n: usize, down: bool) -> Option<u32> {
    if down && *sel + 1 < n {
        *sel += 1;
    } else if !down && *sel > 0 {
        *sel -= 1;
    } else {
        return Some(ev::CURSOR_END);
    }
    None
}

/// The palette cursor of the Livery tab: the zone it paints (0..3), the swatch under it, the colour and finish
/// before (Esc puts them back).
#[derive(Clone, Copy)]
pub(crate) struct PaintPick {
    zone: usize,
    cursor: usize,
    old: [u8; 3],
    old_finish: PaintFinish,
}

/// The top left of Chat keys card `i` (design px).
fn key_card(i: usize) -> (f32, f32) {
    (62.0 + 585.0 * (i % 2) as f32, 228.0 + 68.0 * (i / 2) as f32)
}

/// `.sel-rim` on a glass card (19.4): the orange rim with its halo pulsing .3 to .5 (`a`: the card's opacity).
pub(super) fn sel_rim(m: &mut Mesh, d: Glass, (x, y, w, h): (f32, f32, f32, f32), pulse: f32, a: f32) {
    let rim = vec::rounded_rect(x, y, w, h, d.d(12.0));
    m.halo(&rim, d.d(14.0), hex(G_ORANGE, (0.3 + 0.2 * pulse) * a));
    m.stroke(&rim, true, d.w(1.0), Ends::Round, hex(G_ORANGE, a), d.px);
}

/// REMASTER a metallic swatch over its colour (#135, the mockup's `.metal` sheen) [design choice]: a white sheen
/// from the top-left corner, a dark fall to the bottom-right, and fine flake dots placed by a hash of `seed`, so each
/// swatch keeps its own pattern frame to frame. `(x, y, w, h)` canvas px, `r` the corner radius, `a` the card's opacity.
fn metallic(m: &mut Mesh, d: Glass, (x, y, w, h): (f32, f32, f32, f32), r: f32, seed: u32, a: f32) {
    let body = vec::rounded_rect(x, y, w, h, r);
    let diag = |q: [f32; 2]| (((q[0] - x) / w + (q[1] - y) / h) * 0.5).clamp(0.0, 1.0);
    m.fill_with(&body, &|q| [1.0, 1.0, 1.0, 0.55 * a * (1.0 - diag(q) / 0.46).max(0.0)], d.px);
    m.fill_with(&body, &|q| [0.0, 0.0, 0.0, 0.42 * a * ((diag(q) - 0.46) / 0.54).max(0.0)], d.px);
    // flake: about one faint dot per 24 square px, kept off the corners
    let n = ((w * h) / (24.0 * d.d(1.0).powi(2))).round().max(4.0) as u32;
    let dot = d.d(0.6).max(0.75);
    let mut hsh = seed.wrapping_mul(0x9e37_79b9) ^ 0x85eb_ca6b;
    let mut next = || {
        hsh ^= hsh << 13;
        hsh ^= hsh >> 17;
        hsh ^= hsh << 5;
        (hsh & 0xffff) as f32 / 65535.0
    };
    for _ in 0..n {
        let (u, v, l) = (next(), next(), next());
        let (fx, fy) = (x + r * 0.5 + u * (w - r - dot), y + r * 0.5 + v * (h - r - dot));
        m.fill(&[[fx, fy], [fx + dot, fy], [fx + dot, fy + dot], [fx, fy + dot]], [1.0, 1.0, 1.0, (0.08 + 0.22 * l) * a], d.px);
    }
}

/// REMASTER a pearl swatch over its colour (#351) [design choice, no mockup]: the paint turning towards its edge colour
/// (`pearl_edge`) from the top-left to the bottom-right, a soft white sheen in the top-left corner (fainter than
/// Metallic's) and a few very fine flake dots placed by a hash of `seed`. Arguments as `metallic`.
fn pearl(m: &mut Mesh, d: Glass, rgb: [u8; 3], (x, y, w, h): (f32, f32, f32, f32), r: f32, seed: u32, a: f32) {
    let body = vec::rounded_rect(x, y, w, h, r);
    let diag = |q: [f32; 2]| (((q[0] - x) / w + (q[1] - y) / h) * 0.5).clamp(0.0, 1.0);
    let edge = pearl_edge(rgb);
    m.fill_with(&body, &|q| {
        let t = ((diag(q) - 0.2) / 0.8).clamp(0.0, 1.0);
        [edge[0], edge[1], edge[2], 0.9 * a * t * t * (3.0 - 2.0 * t)]
    }, d.px);
    m.fill_with(&body, &|q| [1.0, 1.0, 1.0, 0.32 * a * (1.0 - diag(q) / 0.4).max(0.0)], d.px);
    // flake: about one faint dot per 60 square px, finer and fewer than Metallic's
    let n = ((w * h) / (60.0 * d.d(1.0).powi(2))).round().max(3.0) as u32;
    let dot = d.d(0.5).max(0.7);
    let mut hsh = seed.wrapping_mul(0x9e37_79b9) ^ 0xc2b2_ae35;
    let mut next = || {
        hsh ^= hsh << 13;
        hsh ^= hsh >> 17;
        hsh ^= hsh << 5;
        (hsh & 0xffff) as f32 / 65535.0
    };
    for _ in 0..n {
        let (u, v, l) = (next(), next(), next());
        let (fx, fy) = (x + r * 0.5 + u * (w - r - dot), y + r * 0.5 + v * (h - r - dot));
        m.fill(&[[fx, fy], [fx + dot, fy], [fx + dot, fy + dot], [fx, fy + dot]], [1.0, 1.0, 1.0, (0.06 + 0.16 * l) * a], d.px);
    }
}

/// A pearl paint's edge colour on the swatches: the colour turned about the grey axis by `PEARL_TURN`, greys first given
/// a little warm chroma (`PEARL_SEED`, times the paint's brightness), as paint.wgsl `fin_shade` does with `fin_pearl()`
/// [APPROX: mirrors the preset by hand; keep in step].
fn pearl_edge(rgb: [u8; 3]) -> [f32; 3] {
    const PEARL_TURN: f32 = 0.6;
    const PEARL_SEED: f32 = 1.2;
    const SEED_DIR: [f32; 3] = [0.40, -0.25, -0.15];
    let c = rgb.map(|v| f32::from(v) / 255.0);
    let peak = c.iter().copied().fold(0.0, f32::max);
    let chroma = peak - c.iter().copied().fold(1.0, f32::min);
    let seed = PEARL_SEED * peak.min(1.0) * (1.0 - (chroma * 4.0).clamp(0.0, 1.0));
    let c = [c[0] + SEED_DIR[0] * seed, c[1] + SEED_DIR[1] * seed, c[2] + SEED_DIR[2] * seed];
    let k = 0.577_350_3_f32;
    let (cs, sn) = (PEARL_TURN.cos(), PEARL_TURN.sin());
    let dot = k * (c[0] + c[1] + c[2]);
    let cross = [k * (c[2] - c[1]), k * (c[0] - c[2]), k * (c[1] - c[0])];
    std::array::from_fn(|i| (c[i] * cs + cross[i] * sn + k * dot * (1.0 - cs)).clamp(0.0, 1.0))
}

/// The finish's sheen over a swatch already filled with its colour: Metallic's, Pearl's, nothing for Gloss.
#[allow(clippy::too_many_arguments)]
fn finish_sheen(m: &mut Mesh, d: Glass, finish: PaintFinish, rgb: [u8; 3], rect: (f32, f32, f32, f32), r: f32, seed: u32, a: f32) {
    match finish {
        PaintFinish::Gloss => {}
        PaintFinish::Metallic => metallic(m, d, rect, r, seed, a),
        PaintFinish::Pearl => pearl(m, d, rgb, rect, r, seed, a),
    }
}

pub(super) fn rgb_hex(c: [u8; 3], a: f32) -> [f32; 4] {
    hex((u32::from(c[0]) << 16) | (u32::from(c[1]) << 8) | u32::from(c[2]), a)
}

/// One stat card of the Career tab (`careerHTML`'s `card`): caption, icon, value with its unit, the note under it.
#[derive(Debug, PartialEq)]
struct StatCard {
    cap: &'static str,
    icon: char,
    value: String,
    unit: &'static str,
    sub: String,
    /// the value in 22 px (a circuit's name) instead of 28
    small: bool,
}

/// `n` and its noun, singular for one.
fn counted(n: u32, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

/// The eight cards in the mockup's order (the left column, then the right) from stats.cfg: dashes where there is no
/// data yet (no lap, no top speed, no circuit), numbers for the counts [design choice]. `car_name` = a car's name by
/// its file id.
fn career_cards(st: &crate::stats::Stats, car_name: &dyn Fn(u32) -> String) -> [StatCard; CAREER_CARDS] {
    let dash = || "--".to_string();
    let card = |cap, icon, value: String, unit, sub: String| StatCard { cap, icon, value, unit, sub, small: false };
    let fav = st.favourite();
    let fast = st.fastest();
    let top = st.top_speed.as_ref();
    [
        card("RACES FINISHED", ui_font::icon::FLAG, st.races.to_string(), "", String::new()),
        card("TIME RACING", ui_font::icon::TIMER, fmt_hms(st.total.race_s), "", format!("PLAY TIME {}", fmt_hms(st.play_s))),
        card("DISTANCE", ui_font::icon::ROUTE, format!("{:.1}", st.total.distance_m / 1000.0), "KM", counted(st.total.overtakes, "OVERTAKE", "OVERTAKES")),
        StatCard { small: true, ..card("FAVOURITE", ui_font::icon::FLAG, fav.map_or_else(dash, |(n, _)| n.to_uppercase()), "", fav.map_or_else(String::new, |(_, t)| counted(t.plays, "VISIT", "VISITS"))) },
        card("FASTEST LAP", ui_font::icon::TIMER, fast.map_or_else(|| "--:--.---".to_string(), |(ms, ..)| glass_time(ms)), "", fast.map_or_else(dash, |(_, n, c)| format!("{} · {}", n.to_uppercase(), car_name(c).to_uppercase()))),
        card("TOP SPEED", ui_font::icon::GAUGE, top.map_or_else(dash, |t| format!("{:.0}", t.kmh)), "KM/H", top.map_or_else(String::new, |t| format!("{} · {}", t.track.to_uppercase(), car_name(t.car).to_uppercase()))),
        card("LONGEST DRIFT", ui_font::icon::ROUTE, format!("{:.0}", st.longest_drift_m), "M", format!("{:.1} KM DRIFTED IN ALL", st.total.drift_m / 1000.0)),
        card("LONGEST JUMP", ui_font::icon::ROUTE, format!("{:.2}", st.longest_jump_s), "S", format!("{} · {:.0} S IN THE AIR", counted(st.total.jumps, "JUMP", "JUMPS"), st.total.air_s)),
    ]
}

/// A rate as the mockup's `pctOf`: whole percent, None before the first race.
fn percent(rate: Option<f32>) -> Option<u32> {
    rate.map(|r| (r * 100.0).round() as u32)
}

/// The stat card `i`'s box (design px): x, y, width.
fn stat_card(i: usize) -> (f32, f32, f32) {
    let (x, w) = if i < 4 { (62.0, 270.0) } else { (892.0, 326.0) };
    (x, 228.0 + STAT_STEP * (i % 4) as f32, w)
}

/// The Records table of `recordsHTML` from stats.cfg: the columns and the rows of the eight circuits (`cars` false, in
/// the game's order) or the ten cars, with the share of time as a bar against the most driven row. A circuit or car with
/// no entry yet is a faded "NOT DRIVEN YET" row [design choice: the mockup's data has every circuit]. `tracks` = the
/// circuits' names, `cars` = (file id, name) of the cars, listed by id as the garage numbers them.
fn records_table(st: &crate::stats::Stats, tracks: &[&str], cars: &[(u32, &str)], by_car: bool) -> (Vec<TableCol>, Vec<TableRow>) {
    let head = if by_car { ["#", "CAR", "RACES", "", "BEST LAP", "ON", "DISTANCE", "TIME", "SHARE OF TIME"] } else { ["#", "CIRCUIT", "VISITS", "FINISHED", "BEST LAP", "WITH", "DISTANCE", "TIME", "SHARE OF TIME"] };
    let xs = [22.0, 62.0, 300.0, 390.0, 500.0, 640.0, 790.0, 900.0, 990.0];
    let cols = head.iter().zip(xs).map(|(&title, x)| TableCol { title, x }).collect();
    let by_id = {
        let mut v = cars.to_vec();
        v.sort_by_key(|c| c.0);
        v
    };
    let car_name = |id: u32| cars.iter().find(|c| c.0 == id).map_or(format!("#{id}"), |c| c.1.to_uppercase());
    let km = |m: f64| format!("{:.1} KM", m / 1000.0);
    let lap = |ms: Option<u32>| ms.map_or("--:--.---".to_string(), glass_time);
    let name_or_dash = |n: Option<String>| n.unwrap_or_else(|| "--".to_string());
    let fav = st.favourite().map(|f| f.0.to_lowercase());
    // (name, entry): races or visits, finished races, best lap, where / with what, distance, time
    type Entry = (u32, Option<u32>, Option<u32>, String, f64, f64);
    let items: Vec<(String, Option<Entry>)> = if by_car {
        by_id.iter().map(|&(id, name)| (name.to_uppercase(), st.cars.get(&id).map(|c| (c.races, None, c.best_lap.as_ref().map(|l| l.0), name_or_dash(c.best_lap.as_ref().map(|l| l.1.to_uppercase())), c.distance_m, c.race_s)))).collect()
    } else {
        tracks.iter().map(|&name| (name.to_uppercase(), st.tracks.iter().find(|(n, _)| n.eq_ignore_ascii_case(name)).map(|(_, t)| (t.plays, Some(t.races), t.best_lap.map(|l| l.0), name_or_dash(t.best_lap.map(|l| car_name(l.1))), t.distance_m, t.race_s)))).collect()
    };
    let most = items.iter().filter_map(|(_, e)| e.as_ref()).map(|e| e.5).fold(0.0, f64::max);
    let rows = items
        .into_iter()
        .enumerate()
        .map(|(i, (name, entry))| {
            let n = Cell::Text((i + 1).to_string());
            let Some((count, races, best, with, dist, time)) = entry else {
                return TableRow { cells: vec![n, Cell::Name(name, None), Cell::Text("NOT DRIVEN YET".into())], none: true };
            };
            let tag = (!by_car && fav.as_deref() == Some(name.to_lowercase().as_str())).then_some("FAV");
            let cells = vec![
                n,
                Cell::Name(name, tag),
                Cell::Text(count.to_string()),
                races.map_or(Cell::Empty, |r| Cell::Text(r.to_string())),
                Cell::Text(lap(best)),
                Cell::Note(with),
                Cell::Text(km(dist)),
                Cell::Text(fmt_hms(time)),
                Cell::Bar(if most > 0.0 { (time / most) as f32 } else { 0.0 }),
            ];
            TableRow { cells, none: false }
        })
        .collect();
    (cols, rows)
}

impl Game {
    /// Q / E (LB / RB): the next or previous tab, each keeping its own row.
    pub(crate) fn personal_tab_step(&mut self, next: bool) {
        let at = self.personal_tab;
        let to = if next { (at + 1).min(TABS.len() - 1) } else { at.saturating_sub(1) };
        if to == at {
            return;
        }
        self.personal_sel[at] = self.sel;
        self.personal_tab = to;
        self.sel = self.personal_sel[to];
        self.t = 0.0;
        self.ev_override = Some(ev::SELECT);
    }

    /// The keys of the modern Personal screen (not typing: `Game::key` hands those to `edit_key`).
    pub(crate) fn personal_key(&mut self, up: bool, down: bool, left: bool, right: bool, enter: bool, back: bool) {
        if self.paint_pick.is_some() {
            return self.pick_key(up, down, left, right, enter, back);
        }
        if back {
            self.go(Screen::Options);
            self.sel = 0; // personal options
            self.snap_needle();
            return;
        }
        let tab = self.personal_tab;
        match tab {
            DRIVER => self.driver_key(up, down, left, right, enter),
            LIVERY => self.livery_key(up, down, left, right, enter),
            CHAT => self.chat_key(up, down, left, right, enter),
            RECORDS => self.records_key(up, down, left, right),
            _ => {}
        }
    }

    /// Start typing into `row` (`editing` ids): a pad gets the keyboard sheet over the screen (`Game::osk`).
    fn begin_edit(&mut self, row: usize, old: String) {
        self.editing = Some((row, old));
        self.osk_open();
    }

    fn driver_key(&mut self, up: bool, down: bool, left: bool, right: bool, enter: bool) {
        let rows = DRIVER_ROWS.len();
        self.sel = self.sel.min(rows - 1);
        let mut e = None;
        if up || down {
            e = step_rows(&mut self.sel, rows, down);
        } else {
            let (row, s) = (DRIVER_ROWS[self.sel], &mut self.settings);
            match row {
                // a text row: the original types into it at once, ours starts on Enter [design choice]
                0 | 1 if enter => {
                    let old = if row == 0 { s.player_name.clone() } else { s.team.clone() };
                    self.begin_edit(row, old);
                }
                2 if left || right => e = swatch_step(&self.strip, &mut s.player_colour, right),
                // a choice stops at its ends with Left / Right, Enter steps on and wraps
                17 if left || right => {
                    let to = if right { s.menu_confirmation + 1 } else { s.menu_confirmation.wrapping_sub(1) };
                    if to < CONFIRMATIONS.len() {
                        s.menu_confirmation = to;
                    } else {
                        e = Some(ev::CURSOR_END);
                    }
                }
                17 if enter => Self::cycle(&mut s.menu_confirmation, CONFIRMATIONS.len(), true),
                18 if left || right => {
                    if s.intro == right {
                        e = Some(ev::CURSOR_END);
                    }
                    s.intro = right;
                }
                18 if enter => s.intro ^= true,
                _ => {}
            }
        }
        if e.is_some() {
            self.ev_override = e;
        }
    }

    /// The Livery tab's keys: Up / Down between the cards; Custom paint switches; a paint card steps its colour
    /// and Enter opens the palette cursor on it, both refused while custom paint is off.
    fn livery_key(&mut self, up: bool, down: bool, left: bool, right: bool, enter: bool) {
        self.sel = self.sel.min(LIVERY_CARDS - 1);
        let mut e = None;
        if up || down {
            e = step_rows(&mut self.sel, LIVERY_CARDS, down);
        } else if self.sel == 0 {
            let s = &mut self.settings;
            if left || right {
                if s.custom_car_colours == right {
                    e = Some(ev::CURSOR_END);
                }
                s.custom_car_colours = right;
            } else if enter {
                s.custom_car_colours ^= true;
            }
        } else if left || right || enter {
            let zone = self.sel - 1;
            let c = &mut self.settings.car_colours[zone];
            e = if !self.settings.custom_car_colours {
                Some(ev::SELECT_ERROR)
            } else if enter {
                self.paint_pick = Some(PaintPick { zone, cursor: grid_at(*c), old: *c, old_finish: self.settings.car_finish[zone] });
                Some(ev::SELECT)
            } else {
                grid_step(c, right)
            };
        }
        if e.is_some() {
            self.ev_override = e;
        }
    }

    /// The palette's finish tabs (#135, #351): Q / E (LB / RB) while the cursor is open step the paint to the previous /
    /// next of Gloss, Metallic, Pearl; the cursor stays on its colour. The end sound where it already is.
    pub(crate) fn pick_finish(&mut self, next: bool) {
        let Some(p) = self.paint_pick else { return };
        let f = &mut self.settings.car_finish[p.zone];
        let at = PaintFinish::ALL.iter().position(|x| x == f).unwrap_or(0);
        let to = if next { at + 1 } else { at.wrapping_sub(1) };
        self.ev_override = Some(match PaintFinish::ALL.get(to) {
            Some(&t) => {
                *f = t;
                ev::SELECT
            }
            None => ev::CURSOR_END,
        });
    }

    /// The palette cursor: the arrows move it in the grid and paint at once (off the grid: the end sound), Enter
    /// keeps the colour and finish, Esc puts the old ones back.
    fn pick_key(&mut self, up: bool, down: bool, left: bool, right: bool, enter: bool, back: bool) {
        let Some(mut p) = self.paint_pick else { return };
        let sw = paint_grid();
        let n = sw.len();
        let mut e = None;
        if up || down || left || right {
            // Left / Right run on through the rows, Up / Down keep the column [the mockup clamps them to the ends]
            let to = if left {
                p.cursor.checked_sub(1)
            } else if right {
                Some(p.cursor + 1).filter(|&k| k < n)
            } else if up {
                p.cursor.checked_sub(PAL_COLS)
            } else {
                Some(p.cursor + PAL_COLS).filter(|&k| k < n)
            };
            match to {
                Some(k) => {
                    p.cursor = k;
                    self.settings.car_colours[p.zone] = sw[k].rgb;
                    self.paint_pick = Some(p);
                }
                None => e = Some(ev::CURSOR_END),
            }
        } else if enter || back {
            if back {
                self.settings.car_colours[p.zone] = p.old;
                self.settings.car_finish[p.zone] = p.old_finish;
            }
            self.paint_pick = None;
            e = Some(if back { ev::ESCAPE } else { ev::SELECT });
        }
        if e.is_some() {
            self.ev_override = e;
        }
    }

    /// The Chat keys tab's keys: Left / Right step through the keys, Up / Down a row (the ends stop), Enter types
    /// into the key (`editing` rows 3..=12, as the Classic screen numbers them).
    fn chat_key(&mut self, up: bool, down: bool, left: bool, right: bool, enter: bool) {
        let at = self.sel.min(CHAT_KEYS - 1);
        let step: isize = if left { -1 } else if right { 1 } else if up { -2 } else if down { 2 } else { 0 };
        if step != 0 {
            match at.checked_add_signed(step).filter(|&k| k < CHAT_KEYS) {
                Some(k) => self.sel = k,
                None => self.ev_override = Some(ev::CURSOR_END),
            }
        } else if enter {
            self.sel = at;
            self.begin_edit(3 + at, self.settings.short_keys[at].clone());
        }
    }

    /// The Records tab's keys: Up / Down move the highlighted row (the ends stop), Left / Right switch between the
    /// circuits and the cars (the row stays where it can).
    fn records_key(&mut self, up: bool, down: bool, left: bool, right: bool) {
        let n = if self.records_cars { self.cars.len() } else { self.tracks.len() };
        self.sel = self.sel.min(n.saturating_sub(1));
        if up || down {
            self.ev_override = step_rows(&mut self.sel, n, down);
        } else if left || right {
            if self.records_cars == right {
                self.ev_override = Some(ev::CURSOR_END);
            } else {
                self.records_cars = right;
                let n = if right { self.cars.len() } else { self.tracks.len() };
                self.sel = self.sel.min(n.saturating_sub(1));
                self.ev_override = Some(ev::SELECT);
            }
        }
    }

    /// The Driver tab's rows (captions included) with their values and help.
    fn driver_items(&self) -> Vec<PauseItem> {
        let s = &self.settings;
        let typing = |row: usize| self.osk.is_none() && matches!(&self.editing, Some((r, _)) if *r == row);
        let sw = strip_swatches(&self.strip);
        let at = swatch_at(&self.strip, &sw, s.player_colour);
        let row = |label: &str, help: &str, val: PauseVal| PauseItem::Row(PauseRow { label: label.into(), help: help.into(), val, dim: 1.0 });
        vec![
            PauseItem::Head("Driver".into()),
            row("Name", HELP_NAME, PauseVal::Text { value: s.player_name.clone(), max: NAME_MAX, editing: typing(0) }),
            row("Team", HELP_TEAM, PauseVal::Text { value: s.team.clone(), max: TEAM_MAX, editing: typing(1) }),
            row("Driver colour", HELP_COLOUR, PauseVal::Swatch { rgb: s.player_colour, name: swatch_name(&self.strip, &sw, s.player_colour).to_string(), first: at == 0, last: at + 1 >= sw.len() }),
            PauseItem::Head("Menus".into()),
            row("Confirmations", HELP_CONFIRM, PauseVal::Choice(CONFIRM_NAMES.iter().map(|n| n.to_string()).collect(), s.menu_confirmation)),
            row("Intro movies", HELP_INTRO, PauseVal::Switch(s.intro)),
        ]
    }

    /// The glass cards the hologram is blurred behind (canvas px, with the corner radius): the plate, the
    /// tab bar and the tab's body.
    fn personal_cards(d: Glass, tab: usize) -> Vec<(f32, f32, f32, f32, f32)> {
        let plate = (d.r(1021.0), d.t(GLASS_PLATE_TOP), d.d(234.0), d.d(119.0));
        let body: Vec<(f32, f32, f32, f32)> = match tab {
            DRIVER => vec![DRIVER_CARD, TRACK_CARD],
            LIVERY => (0..LIVERY_CARDS).map(|i| (SLOT.0, SLOT.1 + SLOT_STEP * i as f32, SLOT.2, SLOT.3)).chain([PALETTE_CARD]).collect(),
            CHAT => (0..CHAT_KEYS).map(|i| (key_card(i).0, key_card(i).1, KEY_CARD.0, KEY_CARD.1)).collect(),
            CAREER => (0..CAREER_CARDS).map(|i| (stat_card(i).0, stat_card(i).1, stat_card(i).2, STAT_H)).collect(),
            _ => vec![RECORDS_PANEL],
        };
        let rect = |(x, y, w, h): (f32, f32, f32, f32)| (d.x(x), d.y(y), d.d(w), d.d(h));
        [plate, (d.x(62.0), d.y(160.0), d.d(1156.0), d.d(52.0))]
            .into_iter()
            .chain(body.into_iter().map(rect))
            .map(|(x, y, w, h)| (x, y, w, h, d.d(12.0)))
            .collect()
    }

    /// REMASTER Personal screen (#91): the Glass frame, the tab bar and the tab's body over the hologram.
    pub(crate) fn personal_modern(&self, size: (u32, u32)) -> Vec<Layer> {
        let canvas = Canvas::hud(size);
        let px = 1.0 / canvas.transform(size).0;
        let d = Glass::new(&canvas, px);
        let tab = self.personal_tab.min(TABS.len() - 1);
        let cards = Self::personal_cards(d, tab);
        let back = self.ui_in(Batch::sharp_text(&self.assets, canvas, 1.0), |_, b| Self::glass_backdrop(b, d));
        let front = self.ui_in(Batch::sharp_text(&self.assets, canvas, 1.0), |g, b| {
            let pulse = 0.5 - 0.5 * (g.t * TAU / 1.1).cos();
            Self::glass_overlays(b, d);
            Self::glass_furniture(b, d, g.t, &format!("OPTIONS // PERSONAL // {}", TABS[tab].0), "LIVE", GlassRules::None);
            // "personal" at 62 px is ~630 wide: the mockup sets it at 41 (`.title.small`, 19.3)
            Self::glass_title_sized(b, d, "personal", "88-137:66", Some((41.0, 47.0)));
            Self::glass_plate(b, d, (d.r(1021.0), d.t(GLASS_PLATE_TOP), d.d(234.0), d.d(119.0)), "187-C-66", "PERSONALSCREEN");
            Self::glass_version(b, d);
            g.glass_tabbar(b, d, &TABS, tab);
            match tab {
                DRIVER => g.personal_driver(b, d, pulse),
                LIVERY => g.personal_livery(b, d, pulse),
                CHAT => g.personal_chat(b, d, pulse),
                CAREER => g.personal_career(b, d),
                _ => g.personal_records(b, d, pulse),
            }
            if !g.asking() {
                g.personal_hints(b, d, tab);
            }
        });
        let mut out = vec![back];
        out.extend(self.hologram(size, None, &cards));
        out.push(front);
        // REMASTER the car previews over the cards; the Driver card's name plate goes over its car
        match tab {
            DRIVER => {
                let (c, across) = DRIVER_CAR;
                out.extend(self.car_preview(size, d, c, across, (PREVIEW_ANGLE, PREVIEW_PITCH), self.preview_paint(None)));
                out.push(self.ui_in(Batch::sharp_text(&self.assets, canvas, 1.0), |g, b| g.track_card_plate(b, d)));
            }
            LIVERY => {
                let sel = self.sel.min(LIVERY_CARDS - 1);
                let zone = self.paint_pick.map(|p| p.zone).or((sel > 0).then(|| sel - 1));
                out.extend(self.car_preview(size, d, LIVERY_DISC.0, LIVERY_ACROSS, (PREVIEW_ANGLE + self.clock * LIVERY_TURN, PREVIEW_PITCH), self.preview_paint(zone)));
            }
            _ => {}
        }
        out
    }

    /// REMASTER car preview (#91, 09-remaster 9.39): the selected car of the garage scene alone, its body's bounding
    /// sphere `across` design px wide round `centre`, seen from `angle` (radians off the nose, to its left) and
    /// `pitch` above, in `paint`. The viewport is the sphere's square, so the car never crosses it. None until the
    /// garage scene is up.
    pub(super) fn car_preview(&self, size: (u32, u32), d: Glass, centre: (f32, f32), across: f32, (angle, pitch): (f32, f32), paint: LivePaint) -> Option<Layer> {
        let p = self.preview_pose(size, d, centre, across, (angle, pitch), PREVIEW_ZOOM)?;
        Some(Layer::Preview { cam: p.cam, cars: p.cars, viewport: p.viewport, paint })
    }

    /// The camera, the car's parts and the viewport of `car_preview` at lens `zoom`, with the projection of its model
    /// space onto the canvas (the Car screen's floor and shadow are drawn in it).
    pub(super) fn preview_pose(&self, size: (u32, u32), d: Glass, centre: (f32, f32), across: f32, (angle, pitch): (f32, f32), zoom: f32) -> Option<PreviewPose> {
        if self.loaded != Loaded::Garage {
            return None;
        }
        let k = self.settings.car;
        let (sphere, radius) = self.cars.get(k)?.sphere;
        // parked as on the Car screen: the car's heading on the garage spot, the tyres on the road
        let (spot, dir) = self.spot;
        let ground = self.garage_wheels.get(k).map_or(0.0, |(h, r)| h[0].y - r);
        let model = Mat4::from_translation(spot - Vec3::Y * ground) * Mat4::from_mat3(Mat3::from_rotation_y((-dir.x).atan2(-dir.z)));
        let mut cars = vec![(k * CAR_MESHES, model)];
        if let Some((hubs, _)) = self.garage_wheels.get(k) {
            cars.extend(hubs.iter().enumerate().map(|(w, h)| (k * CAR_MESHES + 1 + w, model * Mat4::from_translation(*h))));
        }
        // the camera: far enough that the sphere's outline just fills the square (tan of its half angle = the
        // lens's half width)
        let at = model.transform_point3(Vec3::from_array(sphere));
        let nose = Vec3::new(dir.x, 0.0, dir.z).normalize_or(Vec3::NEG_Z);
        let left = Vec3::new(nose.z, 0.0, -nose.x);
        let half = (self.settings.view.fov_x.to_radians() * 0.5).tan() / zoom;
        let dist = radius / half.atan().sin();
        let eye = at + ((nose * angle.cos() + left * angle.sin()) * pitch.cos() + Vec3::Y * pitch.sin()) * dist;
        let look = (at - eye).normalize();
        let mut cam = Camera::new(eye, look.x.atan2(-look.z), look.y.asin());
        cam.zoom = zoom;
        let canvas = Canvas::hud(size);
        let a = d.d(across);
        let viewport = canvas.to_window(size, d.x(centre.0) - a * 0.5, d.y(centre.1) - a * 0.5, a, a);
        let view = glam::camera::rh::view::look_to_mat4(cam.pos, cam.forward(), cam.up());
        Some(PreviewPose { cam, cars, viewport, model, ground, view, half, centre: [d.x(centre.0), d.y(centre.1)], across: a })
    }

    /// The previews' paint: yours (colours and finishes, #136) while custom paint is on, else the stock gloss
    /// colours; `zone` pulses (`.zone-lit`).
    pub(super) fn preview_paint(&self, zone: Option<usize>) -> LivePaint {
        let s = &self.settings;
        let gain = ZONE_GAIN * (0.5 - 0.5 * (self.t * TAU / ZONE_PERIOD).cos());
        let custom = s.custom_car_colours;
        LivePaint {
            to: if custom { s.car_colours.map(Some) } else { [None; 3] },
            pulse: zone.map(|z| (z, gain)),
            finish: s.car_finish.map(|f| if custom { f.scene() } else { Default::default() }),
        }
    }

    /// The hints under the body: the tab's own after TABS and before BACK.
    fn personal_hints(&self, b: &mut Batch, d: Glass, tab: usize) {
        let device = self.controls.last;
        let mut hints: Vec<(&[usize], &str)> = Vec::new();
        if self.editing.is_some() {
            // the keyboard types (a pad's sheet draws its own hints, `asking`)
            if device == Device::Keyboard {
                hints.push((&[11, 12], "TYPE"));
            }
            hints.extend([(&[2][..], "KEEP"), (&[3][..], "UNDO")]);
        } else if self.paint_pick.is_some() {
            // the four arrows: the atlas has no one-glyph cross
            hints.extend([(&[4, 5][..], "FINISH"), (&[0, 1, 9, 10][..], "PICK"), (&[2][..], "KEEP"), (&[3][..], "UNDO")]);
        } else {
            hints.push((&[4, 5], "TABS"));
            match tab {
                DRIVER => {
                    hints.push((&[9, 10], "MOVE"));
                    hints.push(if self.sel.min(DRIVER_ROWS.len() - 1) < 2 { (&[2], "EDIT") } else { (&[0, 1], "CHANGE") });
                }
                LIVERY => {
                    hints.extend([(&[9, 10][..], "MOVE"), (&[0, 1][..], "CHANGE")]);
                    if self.sel.min(LIVERY_CARDS - 1) > 0 {
                        hints.push((&[2], "PALETTE"));
                    }
                }
                CHAT => hints.extend([(&[0, 1, 9, 10][..], "MOVE"), (&[2][..], "EDIT")]),
                RECORDS => hints.extend([(&[9, 10][..], "MOVE"), (&[0, 1][..], if self.records_cars { "CIRCUITS" } else { "CARS" })]),
                _ => {}
            }
            hints.push((&[3], "BACK"));
        }
        Self::glass_hints(b, d, device, &hints, 626.0);
    }

    /// The Driver tab: the rows card with its help, and the card of how the field sees you.
    fn personal_driver(&self, b: &mut Batch, d: Glass, pulse: f32) {
        let items = self.driver_items();
        let sel = self.sel.min(DRIVER_ROWS.len() - 1);
        let (cx, cy, _, _) = DRIVER_CARD;
        self.rows_card(b, d, DRIVER_CARD, 540.0, 272.0, &items, sel, pulse);
        // the dashed rule and the selected row's help under the rows
        let mut m = Mesh::default();
        let ry = d.y(cy + 286.0);
        m.dashed([d.x(cx + 24.0), ry], [d.x(cx + 24.0 + 512.0), ry], d.d(6.0), d.d(4.0), d.w(1.0), hex(G_LINE, 0.45), d.px);
        b.mesh(&m);
        let PauseItem::Row(row) = items.iter().filter(|i| matches!(i, PauseItem::Row(_))).nth(sel).expect("a Driver row") else { return };
        let st = d.font(HudFont::GlassMed, 15.0, 0.5, hex(G_TEXT, 1.0));
        for (k, line) in wrap_text(b, HudFont::GlassMed, &st, &row.help, d.d(512.0)).iter().enumerate() {
            b.font_text(HudFont::GlassMed, line, d.x(cx + 24.0), d.cap_top(HudFont::GlassMed, 15.0, d.y(cy + 298.0 + 20.0 * k as f32), Some(20.0)), &st);
        }
        self.personal_track_card(b, d);
    }

    /// "ON TRACK · HOW THE FIELD SEES YOU": the car's shadow, the map with your dot, the standings with your row. The
    /// car (`car_preview`) and your name plate over it (`track_card_plate`) are drawn after the card.
    fn personal_track_card(&self, b: &mut Batch, d: Glass) {
        let (cx, cy, cw, ch) = TRACK_CARD;
        let s = &self.settings;
        let mut m = Mesh::default();
        glass_card(&mut m, d, (d.x(cx), d.y(cy), d.d(cw), d.d(ch)), d.d(12.0), (0.13, 0.04), 0.36, (10.0, 26.0, 0.45), 0.35, None);
        b.mesh(&m);
        let cap = d.font(HudFont::GlassMed, 11.0, 1.5, hex(G_DIM, 1.0));
        let caption = |b: &mut Batch, text: &str, x: f32, y: f32| {
            b.font_text(HudFont::GlassMed, text, d.x(cx + x), d.cap_top(HudFont::GlassMed, 11.0, d.y(cy + y), None), &cap);
        };
        caption(b, "ON TRACK · HOW THE FIELD SEES YOU", 20.0, 12.0);
        // the car's shadow: `radial-gradient(ellipse 300px 90px at 200px 150px, rgba(0,0,0,.45), transparent 70%)`
        // over the 576 x 180 block at (0, 28)
        let mut m = Mesh::default();
        m.ellipse([d.x(cx + 200.0), d.y(cy + 28.0 + 150.0)], [d.d(210.0), d.d(63.0)], [0.0, 0.0, 0.0, 0.45], [0.0; 4], 64);
        b.mesh(&m);
        // the map: the lap of the selected track, rivals grey, you in your colour
        let tr = &self.tracks[s.track.min(self.tracks.len() - 1)];
        let mut dots = Vec::new();
        if let Some(p) = tr.profile.as_ref().filter(|p| !p.outline.is_empty()) {
            let at = |f: f32| Vec3::from_array(p.outline[(f * p.outline.len() as f32) as usize % p.outline.len()]);
            dots.extend([0.08, 0.31, 0.37, 0.55, 0.71].map(|f| MapCar { pos: at(f), colour: [0x8f, 0xb5, 0xc9], me: false }));
            dots.push(MapCar { pos: at(0.44), colour: s.player_colour, me: true });
        }
        Self::glass_map(b, d, tr, &MapDisc::small((cx + 408.0 + 75.0, cy + 40.0 + 75.0), 75.0), &dots);
        // the standings: two rivals of the game's AI profiles round your row
        caption(b, "STANDINGS", 20.0, 214.0);
        let rivals = self.sample_rivals();
        let car = self.cars.get(s.car).map_or(String::new(), |c| c.name.to_uppercase());
        let me = if s.player_name.is_empty() { "PLAYER".to_string() } else { s.player_name.to_uppercase() };
        let rows = [
            ("3", rivals[0].0.as_str(), rivals[0].1, rivals[0].2.as_str(), SAMPLE_GAPS[0], false),
            ("4", me.as_str(), s.player_colour, car.as_str(), SAMPLE_GAPS[1], true),
            ("5", rivals[1].0.as_str(), rivals[1].1, rivals[1].2.as_str(), SAMPLE_GAPS[2], false),
        ];
        for (i, (pos, name, colour, car, gap, you)) in rows.into_iter().enumerate() {
            Self::standing_row(b, d, (d.x(cx + 20.0), d.y(cy + 232.0 + 41.0 * i as f32), d.d(536.0)), pos, name, colour, car, gap, you);
        }
        let note = d.font(HudFont::GlassMed, 11.0, 1.0, NOTE);
        b.font_text(HudFont::GlassMed, "YOUR COLOUR: THE MAP DOT, THE PLATE BAR, THE STANDINGS BAR", d.x(cx + 20.0), d.cap_top(HudFont::GlassMed, 11.0, d.y(cy + 356.0), None), &note);
    }

    /// The Driver card's name plate as the race shows it: NAME (TEAM) with your colour's bar, over the car.
    fn track_card_plate(&self, b: &mut Batch, d: Glass) {
        let (cx, cy, _, _) = TRACK_CARD;
        let s = &self.settings;
        let name = if s.team.is_empty() { s.player_name.clone() } else { format!("{} ({})", s.player_name, s.team) };
        let plate = if name.trim().is_empty() { "PLAYER".to_string() } else { name.to_uppercase() };
        Self::name_plate(b, d, d.x(cx + 181.0), d.y(cy + 28.0 + 16.0), &plate, s.player_colour);
    }

    /// The Livery tab: the four slot cards, the selected car's name over the disc (the car in its paint, drawn after
    /// the cards: `car_preview`), STOCK PAINT under it while custom paint is off, and the palette card.
    fn personal_livery(&self, b: &mut Batch, d: Glass, pulse: f32) {
        let s = &self.settings;
        let sel = self.sel.min(LIVERY_CARDS - 1);
        let car = self.cars.get(s.car);
        let zones = car.map_or([true; 3], |c| c.zones);
        for i in 0..LIVERY_CARDS {
            self.livery_slot(b, d, i, i == sel, zones, pulse);
        }
        let name = car.map_or(String::new(), |c| c.name.to_uppercase());
        let cap = d.font(HudFont::GlassMed, 11.0, 1.5, hex(G_DIM, 1.0));
        b.font_text(HudFont::GlassMed, &format!("LIVERY · {name}"), d.x(352.0), d.cap_top(HudFont::GlassMed, 11.0, d.y(228.0), None), &cap);
        let note = d.font(HudFont::GlassMed, 11.0, 1.0, NOTE);
        b.font_text(HudFont::GlassMed, "THE CAR FROM THE CAR SCREEN", d.x(352.0), d.cap_top(HudFont::GlassMed, 11.0, d.y(244.0), None), &note);
        let ((dx, dy), r) = LIVERY_DISC;
        Self::glass_big_disc(b, d, (dx, dy), r);
        if !s.custom_car_colours {
            let w = Self::tag_width(b, d, "STOCK PAINT");
            Self::glass_tag(b, d, "STOCK PAINT", (d.x(dx) - w * 0.5, d.y(590.0)));
        }
        self.livery_palette(b, d, sel, pulse);
    }

    /// A Livery slot card (`.slot`): the icon, the title and under it the switch or the colour with its name and
    /// finish (NOT ON THIS CAR for a zone the car lacks). The paint cards fade to .4 while custom paint is off; the
    /// card the palette cursor paints keeps a quiet rim (`.parked`).
    fn livery_slot(&self, b: &mut Batch, d: Glass, i: usize, sel: bool, zones: [bool; 3], pulse: f32) {
        let s = &self.settings;
        let (x, y) = (SLOT.0, SLOT.1 + SLOT_STEP * i as f32);
        let rect = (d.x(x), d.y(y), d.d(SLOT.2), d.d(SLOT.3));
        let a = if i > 0 && !s.custom_car_colours { 0.4 } else { 1.0 };
        let (on, parked) = (sel && self.paint_pick.is_none(), sel && self.paint_pick.is_some());
        let mut m = Mesh::default();
        glass_card(&mut m, d, rect, d.d(12.0), (0.13 * a, 0.04 * a), if parked { 0.55 } else { 0.36 * a }, (10.0, 26.0, 0.45 * a), 0.35 * a, None);
        if on {
            sel_rim(&mut m, d, rect, pulse, a);
        }
        b.mesh(&m);
        let lit = hex(if on { G_ORANGE } else { G_DIM }, a);
        Self::glass_icon(b, if i == 0 { ui_font::icon::CAR } else { ui_font::icon::PAINT_BUCKET }, d.x(x + 16.0), d.y(y + 19.0), d.d(26.0), lit);
        let title = if i == 0 { "CUSTOM PAINT".to_string() } else { format!("PAINT {i}") };
        b.font_text(HudFont::Glass, &title, d.x(x + 56.0), d.cap_top(HudFont::Glass, 20.0, d.y(y + 9.0), None), &d.font(HudFont::Glass, 20.0, 1.0, hex(if on { G_ORANGE } else { G_TEXT }, a)));
        // `.s`: a flex row at 36, the box or chip and the text centred on its line
        let lh = line_h(HudFont::GlassMed, 13.0).max(16.0);
        let mid = d.y(y + 36.0 + lh * 0.5);
        let mut m = Mesh::default();
        let (lead, text) = if i == 0 {
            super::pause_settings::check_box(&mut m, d, (d.x(x + 56.0), mid - d.d(8.0)), (16.0, 4.0), s.custom_car_colours, a);
            (16.0, if s.custom_car_colours { "ON · YOUR PAINT" } else { "OFF · STOCK PAINT" }.to_string())
        } else {
            let (c, finish) = (s.car_colours[i - 1], s.car_finish[i - 1]);
            let (cx, cy, w, h) = (d.x(x + 56.0), mid - d.d(6.5), d.d(20.0), d.d(13.0));
            let chip = vec::rounded_rect(cx, cy, w, h, d.d(4.0));
            m.halo(&chip, d.d(8.0), [0.0, 0.0, 0.0, 0.6 * a]);
            m.fill(&chip, rgb_hex(c, a), d.px);
            finish_sheen(&mut m, d, finish, c, (cx, cy, w, h), d.d(4.0), i as u32, a);
            m.stroke(&chip, true, d.w(1.0), Ends::Round, [1.0, 1.0, 1.0, 0.55 * a], d.px);
            let name = format!("{} · {}", grid_name(c), finish.name());
            (20.0, if zones[i - 1] { name } else { format!("{name} · NOT ON THIS CAR") })
        };
        b.mesh(&m);
        let st = d.font(HudFont::GlassMed, 13.0, 0.5, hex(G_DIM, a));
        b.font_text(HudFont::GlassMed, &text, d.x(x + 56.0 + lead + 7.0), d.cap_top(HudFont::GlassMed, 13.0, mid - d.d(lh * 0.5), Some(lh)), &st);
    }

    /// The palette card (#135): its caption and the cursor's place, the Gloss / Metallic tabs with the shoulder keys,
    /// the 4 x 20 grid in the paint's finish (the paint's nearest swatch ringed white, the cursor orange), the paint
    /// big with its name, finish and hex, and the selected card's help. It shows Paint 1 while Custom paint is selected.
    fn livery_palette(&self, b: &mut Batch, d: Glass, sel: usize, pulse: f32) {
        let (px, py, pw, ph) = PALETTE_CARD;
        let pick = self.paint_pick;
        let zone = pick.map_or(sel.max(1) - 1, |p| p.zone);
        let (c, finish) = (self.settings.car_colours[zone], self.settings.car_finish[zone]);
        let rect = (d.x(px), d.y(py), d.d(pw), d.d(ph));
        let mut m = Mesh::default();
        glass_card(&mut m, d, rect, d.d(12.0), (0.13, 0.04), 0.36, (10.0, 26.0, 0.45), 0.35, None);
        if pick.is_some() {
            sel_rim(&mut m, d, rect, pulse, 1.0);
        }
        // `.swgrid` in the #135 state: 3 px corners, the rings 3 px out with 5 px corners
        let grid = paint_grid();
        let at = grid_at(c);
        let size = d.d(PAL_CELL);
        for (k, s) in grid.iter().enumerate() {
            let x = d.x(px + 22.0 + (PAL_CELL + PAL_GAP) * (k % PAL_COLS) as f32);
            let y = d.y(py + 80.0 + (PAL_CELL + PAL_GAP) * (k / PAL_COLS) as f32);
            let r = vec::rounded_rect(x, y, size, size, d.d(3.0));
            m.fill(&r, rgb_hex(s.rgb, 1.0), d.px);
            if finish == PaintFinish::Gloss {
                // APPROX: the 6 px inset glow along the top as a fade over the chip
                m.fill_with(&r, &|q| [1.0, 1.0, 1.0, 0.10 * (1.0 - ((q[1] - y) / size).clamp(0.0, 1.0))], d.px);
            } else {
                finish_sheen(&mut m, d, finish, s.rgb, (x, y, size, size), d.d(3.0), k as u32, 1.0);
            }
            m.stroke(&vec::rounded_rect(x + d.d(0.5), y + d.d(0.5), size - d.d(1.0), size - d.d(1.0), d.d(2.5)), true, d.w(1.0), Ends::Round, [1.0, 1.0, 1.0, 0.22], d.px);
        }
        // the rings over every chip: the current paint's white, then the cursor's orange
        for (k, cursor) in [(at, false), (pick.map_or(usize::MAX, |p| p.cursor), true)] {
            if k >= grid.len() || (!cursor && pick.is_some_and(|p| p.cursor == at)) {
                continue;
            }
            let x = d.x(px + 22.0 + (PAL_CELL + PAL_GAP) * (k % PAL_COLS) as f32);
            let y = d.y(py + 80.0 + (PAL_CELL + PAL_GAP) * (k / PAL_COLS) as f32);
            let ring = vec::rounded_rect(x - d.d(3.0), y - d.d(3.0), size + d.d(6.0), size + d.d(6.0), d.d(5.0));
            if cursor {
                m.halo(&ring, d.d(14.0), hex(G_ORANGE, 0.3 + 0.2 * pulse));
                m.stroke(&ring, true, d.w(2.0), Ends::Round, hex(G_ORANGE, 1.0), d.px);
            } else {
                m.halo(&ring, d.d(10.0), hex(0x8fd0ff, 0.8));
                m.stroke(&ring, true, d.w(2.0), Ends::Round, hex(0xf4fbff, 1.0), d.px);
            }
        }
        m.dashed([d.x(px + 20.0), d.y(py + 166.0)], [d.x(px + 20.0 + 402.0), d.y(py + 166.0)], d.d(6.0), d.d(4.0), d.w(1.0), hex(G_LINE, 0.45), d.px);
        // the paint big: a 64 px chip with its glow, inset ring and light (or the metallic / pearl sheen)
        let (cx, cy, cs) = (d.x(px + 20.0), d.y(py + 180.0), d.d(64.0));
        let chip = vec::rounded_rect(cx, cy, cs, cs, d.d(10.0));
        m.halo(&chip, d.d(16.0), rgb_hex(c, 0.4));
        m.fill(&chip, rgb_hex(c, 1.0), d.px);
        if finish == PaintFinish::Gloss {
            m.fill_with(&chip, &|q| [1.0, 1.0, 1.0, 0.15 * (1.0 - ((q[1] - cy) / cs).clamp(0.0, 1.0))], d.px);
        } else {
            finish_sheen(&mut m, d, finish, c, (cx, cy, cs, cs), d.d(10.0), 99, 1.0);
        }
        m.stroke(&vec::rounded_rect(cx + d.d(0.5), cy + d.d(0.5), cs - d.d(1.0), cs - d.d(1.0), d.d(9.5)), true, d.w(1.0), Ends::Round, [1.0, 1.0, 1.0, 0.35], d.px);
        b.mesh(&m);
        let caption = d.font(HudFont::GlassMed, 11.0, 1.5, hex(if pick.is_some() { G_ORANGE } else { G_DIM }, 1.0));
        b.font_text(HudFont::GlassMed, &format!("PALETTE · PAINT {}", zone + 1), d.x(px + 20.0), d.cap_top(HudFont::GlassMed, 11.0, d.y(py + 14.0), None), &caption);
        // the cursor's place (the open cursor's, else the paint's nearest swatch), right-aligned
        let count = d.font(HudFont::GlassMed, 11.0, 1.5, hex(G_TEXT, 1.0));
        let n = format!("{}/{}", pick.map_or(at, |p| p.cursor) + 1, grid.len());
        let nw = b.font_width(HudFont::GlassMed, &n, &count) - count.tracking;
        b.font_text(HudFont::GlassMed, &n, d.x(px + pw - 20.0) - nw, d.cap_top(HudFont::GlassMed, 11.0, d.y(py + 14.0), None), &count);
        self.finish_tabs(b, d, (px + 20.0, py + 36.0), finish, pick.is_some());
        // the name (26 px line) over the finish and hex caption, the pair centred on the chip
        let top = py + 180.0 + (64.0 - 26.0 - line_h(HudFont::GlassMed, 11.0)) * 0.5;
        let tx = d.x(px + 20.0 + 64.0 + 14.0);
        let name = if finish == PaintFinish::Gloss { grid_name(c).to_string() } else { format!("{} {}", finish.name(), grid_name(c)) };
        b.font_text(HudFont::Glass, &name, tx, d.cap_top(HudFont::Glass, 25.0, d.y(top), Some(26.0)), &d.font(HudFont::Glass, 25.0, 0.5, WHITE));
        let cap = d.font(HudFont::GlassMed, 11.0, 1.5, hex(G_DIM, 1.0));
        b.font_text(HudFont::GlassMed, &format!("{} · #{:02X}{:02X}{:02X}", finish.name(), c[0], c[1], c[2]), tx, d.cap_top(HudFont::GlassMed, 11.0, d.y(top + 26.0), None), &cap);
        let help = if sel == 0 { HELP_CUSTOM } else { HELP_PAINT[sel - 1] };
        let st = d.font(HudFont::GlassMed, 14.0, 0.5, hex(G_TEXT, 1.0));
        let dim = d.font(HudFont::GlassMed, 14.0, 0.5, hex(G_DIM, 1.0));
        let lines: Vec<(String, &FontStyle)> = wrap_text(b, HudFont::GlassMed, &st, help, d.d(402.0)).into_iter().map(|l| (l, &st))
            .chain(wrap_text(b, HudFont::GlassMed, &dim, HELP_FINISH, d.d(402.0)).into_iter().map(|l| (l, &dim)))
            .collect();
        for (k, (line, style)) in lines.iter().enumerate() {
            b.font_text(HudFont::GlassMed, line, d.x(px + 20.0), d.cap_top(HudFont::GlassMed, 14.0, d.y(py + 264.0 + 19.0 * k as f32), Some(19.0)), style);
        }
    }

    /// The palette's finish tabs (`.fintabs`) from (x, y), the row's top (design px): the shoulder key, GLOSS and
    /// METALLIC 22 apart, the other shoulder key; the paint's finish white with the glowing orange underline. The
    /// names dim while the cursor is closed (the keys then switch the screen's tabs) [design choice].
    fn finish_tabs(&self, b: &mut Batch, d: Glass, (x, y): (f32, f32), finish: PaintFinish, open: bool) {
        let mid = d.y(y + 14.0);
        let mut m = Mesh::default();
        let mut runs = Vec::new();
        let mut tx = d.x(x + 26.0 + 22.0);
        for f in PaintFinish::ALL {
            let on = f == finish;
            let st = d.font(HudFont::Glass, 17.0, 1.0, hex(if on { 0xffffff } else { G_DIM }, if open || on { 1.0 } else { 0.6 }));
            let w = b.font_width(HudFont::Glass, f.name(), &st) - st.tracking;
            if on {
                let line = vec::rounded_rect(tx, mid + d.d(13.0), w, d.d(2.0), d.d(1.0));
                m.halo(&line, d.d(8.0), hex(G_ORANGE, if open { 0.5 } else { 0.25 }));
                m.fill(&line, hex(G_ORANGE, if open { 1.0 } else { 0.6 }), d.px);
            }
            runs.push((f.name(), tx, st));
            tx += w + d.d(22.0);
        }
        b.mesh(&m);
        for (name, x, st) in &runs {
            b.font_text(HudFont::Glass, name, *x, d.cap_top(HudFont::Glass, 17.0, mid - d.d(14.0), Some(28.0)), st);
        }
        self.glass_shoulder(b, d, [d.x(x + 13.0), mid], "Q", 4);
        self.glass_shoulder(b, d, [tx + d.d(13.0), mid], "E", 5);
    }

    /// `discHTML`: a big bezel disc (19.3) at `c` with the bezel radius `r` (design px): the tick ring 14 px out
    /// (80 minor and 10 major ticks from 3 o'clock, as the SVG dashes start), the conic bezel 10 px wide, the dark
    /// gap 6 px and the grid face with its gloss and inset shadow (the map disc's face, `glass_map`).
    fn glass_big_disc(b: &mut Batch, d: Glass, c: (f32, f32), r: f32) {
        let cc = [d.x(c.0), d.y(c.1)];
        let (rb, ri, rf) = (d.d(r), d.d(r - 10.0), d.d(r - 16.0));
        let mut m = Mesh::default();
        // the dashes are centred on their circles: r + 14 (7 long), r + 16 (11 long)
        m.ticks(cc, d.d(r + 14.0 - 3.5), 80, d.d(7.0), d.w(1.2), hex(G_TEXT, 0.5), 0.0, d.px);
        m.ticks(cc, d.d(r + 16.0 - 5.5), 10, d.d(11.0), d.w(2.4), hex(0xf4fbff, 0.85), 0.0, d.px);
        let bez = circle(cc, rb);
        m.shadow(&bez, 0.0, d.d(24.0), d.d(54.0), [0.0, 0.0, 0.0, 0.65]);
        m.halo(&bez, d.d(70.0), hex(0x50aaff, 0.28));
        m.ring(cc, ri, rb, 180, &Self::bezel, d.px);
        m.disc(cc, ri, hex(G_DARK, 1.0), hex(G_DARK, 1.0), 96);
        m.disc_at(cc, rf, [cc[0], cc[1] - 0.4 * rf], [120.0 / 255.0, 190.0 / 255.0, 1.0, 0.24], [8.0 / 255.0, 20.0 / 255.0, 40.0 / 255.0, 0.9], 96);
        Self::disc_overlays(&mut m, d, cc, rf, 22.0, hex(G_LINE, 0.12), (0.2, 0.34), (0.0, 0.0), (30.0, 0.6));
        Self::gap_ring(&mut m, d, cc, rf, ri);
        b.mesh(&m);
    }

    /// `.tag-v`'s width (canvas px): 10 px bold text, 7 px padding and the rim.
    pub(super) fn tag_width(b: &Batch, d: Glass, text: &str) -> f32 {
        let st = d.font(HudFont::Glass, 10.0, 1.5, WHITE);
        b.font_width(HudFont::Glass, text, &st) - st.tracking + d.d(16.0)
    }

    /// `.tag-v`: a small orange tag on the dark tag ground with an orange rim, 18 design px high, its top left at
    /// (x, y) in canvas px.
    pub(super) fn glass_tag(b: &mut Batch, d: Glass, text: &str, (x, y): (f32, f32)) {
        let tag = vec::rounded_rect(x, y, Self::tag_width(b, d, text), d.d(18.0), d.d(7.0));
        let mut m = Mesh::default();
        m.fill(&tag, hex(0x0a1626, 1.0), d.px);
        m.stroke(&tag, true, d.w(1.0), Ends::Round, hex(G_ORANGE, 0.6), d.px);
        b.mesh(&m);
        b.font_text(HudFont::Glass, text, x + d.d(8.0), d.cap_top(HudFont::Glass, 10.0, y, Some(18.0)), &d.font(HudFont::Glass, 10.0, 1.5, hex(G_ORANGE, 1.0)));
    }

    /// The Chat keys tab: the ten keys as cards (the key cap and the message's field, typed in place), then the
    /// tag that they are stored only and how the lobby would show the selected one.
    fn personal_chat(&self, b: &mut Batch, d: Glass, pulse: f32) {
        let s = &self.settings;
        let sel = self.sel.min(CHAT_KEYS - 1);
        for i in 0..CHAT_KEYS {
            let (x, y) = key_card(i);
            let typing = self.osk.is_none() && matches!(&self.editing, Some((r, _)) if *r == 3 + i);
            let on = i == sel && self.editing.is_none();
            let rect = (d.x(x), d.y(y), d.d(KEY_CARD.0), d.d(KEY_CARD.1));
            let mut m = Mesh::default();
            glass_card(&mut m, d, rect, d.d(12.0), (0.13, 0.04), 0.36, (10.0, 26.0, 0.45), 0.35, None);
            if on {
                sel_rim(&mut m, d, rect, pulse, 1.0);
            }
            // `.kcap` 32 x 32 with the key's digit, orange on the selected card
            let cap = (d.x(x + 14.0), d.y(y + 14.0), d.d(32.0), d.d(32.0));
            cap_body(&mut m, d, cap, false, 1.0);
            if on {
                m.stroke(&vec::rounded_rect(cap.0, cap.1, cap.2, cap.3, d.d(5.0)), true, d.w(1.0), Ends::Round, hex(G_ORANGE, 1.0), d.px);
            }
            b.mesh(&m);
            let st = d.font(HudFont::Glass, 17.0, 0.5, hex(if on { G_ORANGE } else { 0xffffff }, 1.0));
            let digit = i.to_string();
            let dw = b.font_width(HudFont::Glass, &digit, &st) - st.tracking;
            b.font_text(HudFont::Glass, &digit, cap.0 + (cap.2 - dw) * 0.5, d.cap_top(HudFont::Glass, 17.0, cap.1, Some(32.0)), &st);
            // the 490 px field: no counter (127 characters), its end shown while typed into
            let val = PauseVal::Text { value: s.short_keys[i].clone(), max: SHORT_KEY_MAX, editing: typing };
            self.pause_control(b, d, &val, (d.x(x + 62.0), d.x(x + 62.0 + 490.0)), d.y(y + 16.0 + 13.0), false, 1.0);
        }
        // the lobby line, centred under the cards: the tag, 12 px, then "IN THE LOBBY: NAME: message" (the name in
        // the driver colour, the message as typed)
        let tag = "STORED FOR THE MULTIPLAYER LOBBY · NOT USED YET";
        let me = if s.player_name.is_empty() { "PLAYER".to_string() } else { s.player_name.to_uppercase() };
        let msg = if s.short_keys[sel].is_empty() { "--" } else { s.short_keys[sel].as_str() };
        let parts = [("IN THE LOBBY: ".to_string(), NOTE), (format!("{me}: "), rgb_hex(s.player_colour, 1.0)), (msg.to_string(), WHITE)];
        let fonts: Vec<FontStyle> = parts.iter().map(|(_, c)| d.font(HudFont::GlassMed, 11.0, 1.0, *c)).collect();
        let tw = Self::tag_width(b, d, tag);
        let total = tw + d.d(12.0) + parts.iter().zip(&fonts).map(|((t, _), st)| b.font_width(HudFont::GlassMed, t, st)).sum::<f32>();
        let (mut x, top) = (d.x(640.0) - total * 0.5, d.y(576.0));
        Self::glass_tag(b, d, tag, (x, top));
        x += tw + d.d(12.0);
        for ((text, _), st) in parts.iter().zip(&fonts) {
            x += b.font_text(HudFont::GlassMed, text, x, d.cap_top(HudFont::GlassMed, 11.0, top, Some(18.0)), st);
        }
    }

    /// Two AI drivers for the sample standings: (nick, colour, car) of the first two profiles of the install
    /// [design choice]; dashes when there are none.
    fn sample_rivals(&self) -> [(String, [u8; 3], String); 2] {
        let pick = |i: usize| self.profiles.get(i).map_or_else(|| ("--".to_string(), [0x8f, 0xb5, 0xc9], String::new()), |p| (p.nick.to_uppercase(), p.colour, p.cars[0].to_uppercase()));
        [pick(0), pick(1)]
    }

    /// `.nplate`: a name plate centred on `cx` with its top at `top` (canvas px), a bar in the driver colour first.
    fn name_plate(b: &mut Batch, d: Glass, cx: f32, top: f32, text: &str, colour: [u8; 3]) {
        let st = d.font(HudFont::Glass, 14.0, 1.0, WHITE);
        let tw = b.font_width(HudFont::Glass, text, &st);
        let w = d.d(1.0 + 7.0 + 4.0 + 7.0 + 10.0 + 1.0) + tw;
        let h = d.d(line_h(HudFont::Glass, 14.0) + 6.0 + 2.0);
        let x = cx - w * 0.5;
        let mut m = Mesh::default();
        let r = vec::rounded_rect(x, top, w, h, d.d(6.0));
        m.fill(&r, [4.0 / 255.0, 10.0 / 255.0, 22.0 / 255.0, 0.55], d.px);
        m.stroke(&r, true, d.w(1.0), Ends::Round, hex(G_LINE, 0.3), d.px);
        let bar = vec::rounded_rect(x + d.d(1.0 + 7.0), top + (h - d.d(14.0)) * 0.5, d.d(4.0), d.d(14.0), d.d(2.0));
        m.halo(&bar, d.d(6.0), rgb_hex(colour, 0.6));
        m.fill(&bar, rgb_hex(colour, 1.0), d.px);
        b.mesh(&m);
        b.font_text(HudFont::Glass, text, x + d.d(1.0 + 7.0 + 4.0 + 7.0), d.cap_top(HudFont::Glass, 14.0, top, Some(h / d.d(1.0))), &st);
    }

    /// `.st`: one standings row, 39 design px high, `w` canvas px wide at (x, y): position, your colour's bar, the
    /// name (with the YOU tag on your row), the car and the gap. Your row is lit; its car sits left of the gap.
    #[allow(clippy::too_many_arguments)]
    fn standing_row(b: &mut Batch, d: Glass, (x, y, w): (f32, f32, f32), pos: &str, name: &str, colour: [u8; 3], car: &str, gap: &str, you: bool) {
        let h = d.d(39.0);
        let mut m = Mesh::default();
        if you {
            let row = vec::rounded_rect(x, y, w, h, d.d(9.0));
            m.halo(&row, d.d(14.0), hex(0x50aaff, 0.18));
            m.fill(&row, hex(G_S1, 0.10), d.px);
            m.stroke(&row, true, d.w(1.0), Ends::Round, hex(G_S1, 0.5), d.px);
        }
        let bar = vec::rounded_rect(x + d.d(1.0 + 40.0), y + d.d(8.5), d.d(4.0), d.d(22.0), d.d(2.0));
        m.halo(&bar, d.d(6.0), rgb_hex(colour, 0.6));
        m.fill(&bar, rgb_hex(colour, 1.0), d.px);
        b.mesh(&m);
        let cy = |px: f32| d.cap_top(HudFont::Glass, px, y, Some(39.0));
        let st = d.font(HudFont::Glass, 20.0, 0.0, WHITE);
        let pw = b.font_width(HudFont::Glass, pos, &st) - st.tracking;
        b.font_text(HudFont::Glass, pos, x + d.d(1.0 + 20.0) - pw * 0.5, cy(20.0), &st);
        let nst = d.font(HudFont::Glass, 18.0, 0.5, hex(if you { 0xffffff } else { G_TEXT }, 1.0));
        let nx = x + d.d(1.0 + 40.0 + 4.0 + 12.0);
        let nw = b.font_width(HudFont::Glass, name, &nst) - nst.tracking;
        b.font_text(HudFont::Glass, name, nx, cy(18.0), &nst);
        if you {
            let tst = d.font(HudFont::Glass, 10.0, 1.5, hex(G_S1, 1.0));
            b.font_text(HudFont::Glass, "YOU", nx + nw + d.d(8.0), cy(18.0) + nst.size - tst.size, &tst);
        }
        // the rivals' car column follows the 170 px name column; yours is a 100 px box next to the gap's 90 (80 + 10 padding)
        let car_x = if you { x + w - d.d(1.0 + 90.0 + 100.0) } else { nx + d.d(170.0) };
        b.font_text(HudFont::GlassMed, car, car_x, d.cap_top(HudFont::GlassMed, 14.0, y, Some(39.0)), &d.font(HudFont::GlassMed, 14.0, 1.0, hex(G_DIM, 1.0)));
        let gst = d.font(HudFont::Glass, 18.0, 0.5, WHITE);
        let gw = b.font_width(HudFont::Glass, gap, &gst) - gst.tracking;
        b.font_text(HudFont::Glass, gap, x + w - d.d(1.0 + 10.0) - gw, cy(18.0), &gst);
    }

    /// The Career tab (`careerHTML`): four stat cards each side of the win disc, the orange arc of the win share round
    /// it, the wins in chromatic numbers and the podium and clean-race rings under them.
    fn personal_career(&self, b: &mut Batch, d: Glass) {
        let st = &self.stats;
        let name = |id: u32| self.cars.iter().find(|c| c.id == id).map_or(format!("#{id}"), |c| c.name.clone());
        for (i, card) in career_cards(st, &name).iter().enumerate() {
            Self::glass_stat_card(b, d, stat_card(i), card);
        }
        let (c, r) = CAREER_DISC;
        let cc = [d.x(c.0), d.y(c.1)];
        // the win share's arc from 12 o'clock, clockwise (SVG: round caps, a 5 px glow)
        let win = percent(st.win_rate());
        if let Some(p) = win.filter(|&p| p > 0) {
            let arc = vec::arc(cc, d.d(WIN_ARC_R), -FRAC_PI_2, -FRAC_PI_2 + TAU * (p.min(100) as f32 / 100.0), 120);
            let mut m = Mesh::default();
            m.glow(&arc, false, d.d(3.5), d.d(5.0), hex(G_ORANGE, 0.6), 3);
            m.stroke(&arc, false, d.w(3.5), Ends::Round, hex(G_ORANGE, 1.0), d.px);
            b.mesh(&m);
        }
        Self::glass_big_disc(b, d, c, r);
        // the face's column, centred on the disc: WINS, the count, OF N RACES, the rings 12 px under
        let (cap_f, lh) = (HudFont::GlassMed, line_h(HudFont::GlassMed, 11.0));
        let mut top = c.1 - (lh * 2.0 + 72.0 + 12.0 + 64.0) * 0.5;
        let centred = |b: &mut Batch, f: HudFont, text: &str, st: &FontStyle, cy: f32| {
            let w = b.font_width(f, text, st) - st.tracking;
            b.font_text(f, text, cc[0] - w * 0.5, cy, st);
        };
        centred(b, cap_f, "WINS", &d.font(cap_f, 11.0, 1.5, hex(G_DIM, 1.0)), d.cap_top(cap_f, 11.0, d.y(top), None));
        top += lh;
        Self::chroma_text(b, d, cc[0], d.cap_top(HudFont::Glass, 72.0, d.y(top), Some(72.0)), &st.wins.to_string(), 72.0);
        top += 72.0;
        let of = format!("OF {} · {}", counted(st.races, "RACE", "RACES"), win.map_or("--%".to_string(), |p| format!("{p}%")));
        centred(b, cap_f, &of, &d.font(cap_f, 11.0, 1.5, hex(G_TEXT, 1.0)), d.cap_top(cap_f, 11.0, d.y(top), None));
        top += lh + 12.0;
        for (k, (rate, label)) in [(st.podium_rate(), "PODIUM"), (st.clean_rate(), "CLEAN")].into_iter().enumerate() {
            let x = c.0 - 41.0 + 82.0 * k as f32;
            Self::ring_gauge(b, d, [d.x(x), d.y(top + 32.0)], 64.0, percent(rate), label);
        }
    }

    /// `careerHTML`'s `card`: a 84 px glass card at (x, y) with w design px, the icon, the caption, the value (with its
    /// unit, baseline-aligned) and the note.
    fn glass_stat_card(b: &mut Batch, d: Glass, (x, y, w): (f32, f32, f32), card: &StatCard) {
        let rect = (d.x(x), d.y(y), d.d(w), d.d(STAT_H));
        let mut m = Mesh::default();
        glass_card(&mut m, d, rect, d.d(12.0), (0.13, 0.04), 0.36, (10.0, 26.0, 0.45), 0.35, None);
        b.mesh(&m);
        Self::glass_icon(b, card.icon, d.x(x + 16.0), d.y(y + 29.0), d.d(26.0), hex(G_DIM, 1.0));
        let e = HudFont::GlassMed;
        b.font_text(e, card.cap, d.x(x + 56.0), d.cap_top(e, 11.0, d.y(y + 12.0), None), &d.font(e, 11.0, 1.5, hex(G_DIM, 1.0)));
        // the value's line box 32 high; the 22 px circuit name and the unit share its 28 px baseline
        let base = d.cap_top(HudFont::Glass, 28.0, d.y(y + 26.0), Some(32.0)) + 28.0 * d.s * ui_font::face(HudFont::Glass).cap / ui_font::SRC;
        let px = if card.small { 22.0 } else { 28.0 };
        let st = d.font(HudFont::Glass, px, 1.0, WHITE);
        let vx = d.x(x + 56.0);
        let vw = b.font_width(HudFont::Glass, &card.value, &st);
        b.font_text(HudFont::Glass, &card.value, vx, d.cap_above(HudFont::Glass, px, base), &st);
        if !card.unit.is_empty() {
            b.font_text(e, card.unit, vx + vw + d.d(6.0), d.cap_above(e, 15.0, base), &d.font(e, 15.0, 1.0, hex(G_DIM, 1.0)));
        }
        if !card.sub.is_empty() {
            b.font_text(e, &card.sub, d.x(x + 56.0), d.cap_top(e, 11.0, d.y(y + 60.0), None), &d.font(e, 11.0, 1.0, NOTE));
        }
    }

    /// `.chroma`: `text` centred on canvas x `cx` with the cap top at `top`: the chromatic offsets (red left, cyan
    /// right, 1.5 px) and a soft blue glow [APPROX: the 26 px text glow as a soft pad, as `glass_selector`].
    fn chroma_text(b: &mut Batch, d: Glass, cx: f32, top: f32, text: &str, px: f32) {
        let st = d.font(HudFont::Glass, px, 0.0, hex(0xf4fbff, 1.0));
        let w = b.font_width(HudFont::Glass, text, &st) - st.tracking;
        let x = cx - w * 0.5;
        let mut m = Mesh::default();
        m.soft_pad(&vec::rounded_rect(x + d.d(4.0), top + d.d(4.0), w - d.d(8.0), st.size - d.d(8.0), d.d(10.0)), d.d(26.0), [120.0 / 255.0, 200.0 / 255.0, 1.0, 0.22]);
        b.mesh(&m);
        let off = d.d(1.5);
        b.font_text(HudFont::Glass, text, x - off, top, &FontStyle { color: [1.0, 70.0 / 255.0, 70.0 / 255.0, 0.55], ..st });
        b.font_text(HudFont::Glass, text, x + off, top, &FontStyle { color: [70.0 / 255.0, 200.0 / 255.0, 1.0, 0.55], ..st });
        b.font_text(HudFont::Glass, text, x, top, &st);
    }

    /// `ringSVG`: a ring gauge `dia` design px wide at `c` (canvas px): the track in the line colour, `pct` of the circle
    /// from 12 o'clock in sector-1 blue with a glow (a dot at 0 %, none before data), the value over its label.
    fn ring_gauge(b: &mut Batch, d: Glass, c: [f32; 2], dia: f32, pct: Option<u32>, label: &str) {
        let r = d.d(dia * 0.5 - 5.0);
        let mut m = Mesh::default();
        m.stroke(&circle(c, r), true, d.w(4.0), Ends::Round, hex(G_LINE, 0.14), d.px);
        if let Some(p) = pct {
            // SVG's round cap draws a dot for a zero-length dash
            let sweep = (TAU * p.min(100) as f32 / 100.0).max(0.002);
            let arc = vec::arc(c, r, -FRAC_PI_2, -FRAC_PI_2 + sweep, (sweep * 20.0).ceil().max(2.0) as usize);
            m.glow(&arc, false, d.d(4.0), d.d(4.0), hex(0x4aa8ff, 0.6), 3);
            m.stroke(&arc, false, d.w(4.0), Ends::Round, hex(G_S1, 1.0), d.px);
        }
        b.mesh(&m);
        let text = pct.map_or("--".to_string(), |p| format!("{p}%"));
        let st = d.font(HudFont::Glass, dia * 0.27, 0.0, WHITE);
        let w = b.font_width(HudFont::Glass, &text, &st) - st.tracking;
        b.font_text(HudFont::Glass, &text, c[0] - w * 0.5, d.cap_above(HudFont::Glass, dia * 0.27, c[1] + d.d(2.0)), &st);
        let st = d.font(HudFont::GlassMed, 9.5, 1.5, hex(G_DIM, 1.0));
        let w = b.font_width(HudFont::GlassMed, label, &st) - st.tracking;
        b.font_text(HudFont::GlassMed, label, c[0] - w * 0.5, d.cap_above(HudFont::GlassMed, 9.5, c[1] + d.d(dia * 0.2)), &st);
    }

    /// The Records tab (`recordsHTML`): the subtabs and the table of circuits or cars from stats.cfg.
    fn personal_records(&self, b: &mut Batch, d: Glass, pulse: f32) {
        let cars = self.records_cars;
        let (cols, rows) = records_table(&self.stats, &self.tracks.iter().map(|t| t.name.as_str()).collect::<Vec<_>>(), &self.cars.iter().map(|c| (c.id, c.name.as_str())).collect::<Vec<_>>(), cars);
        let (x, y, _, _) = RECORDS_PANEL;
        let table = GlassTable { rect: RECORDS_PANEL, head_y: 42.0, rows_y: 62.0, row_h: RECORD_ROW_H[cars as usize], cols: &cols, rows: &rows, sel: self.sel };
        Self::glass_table(b, d, &table, pulse);
        Self::glass_subtabs(b, d, (x + 1.0 + 18.0, y + 1.0 + 10.0), &SUBTABS, cars as usize);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// #351: a pearl's swatch edge colour turns a coloured paint (red towards orange: green rises, red falls), gives white and
    /// silver a warm tint to turn, and leaves black and the channel mean of a colour alone.
    #[test]
    fn pearl_edge_turns_colours_and_tints_greys() {
        let red = pearl_edge([237, 29, 29]);
        assert!(red[1] > 29.0 / 255.0 + 0.1 && red[0] < 237.0 / 255.0);
        let white = pearl_edge([238, 238, 238]);
        assert!(white.iter().any(|c| (c - white[0]).abs() > 0.05), "{white:?}");
        assert_eq!(pearl_edge([0, 0, 0]), [0.0; 3]);
        // the turn keeps the mean of the channels (before clamping): a mid colour that stays inside the gamut
        let mid = pearl_edge([120, 100, 140]);
        let mean = |c: [f32; 3]| (c[0] + c[1] + c[2]) / 3.0;
        assert!((mean(mid) - (120.0 + 100.0 + 140.0) / 765.0).abs() < 0.02);
    }

    /// The shipped palette's last cells: 22 black, then #0a0000 and nine colours (Data/Default.ACT).
    fn shipped_strip() -> [[u8; 3]; STRIP_CELLS] {
        let tail = [[0x0a, 0, 0], [0x3c, 0x7b, 0x3c], [0x4e, 0x46, 0x0d], [0x47, 0x0d, 0x0e], [0x0d, 0x0c, 0x8c], [0x3b, 0x8e, 0x47], [0x2a, 0x51, 0xfd], [0xa3, 0xfa, 0xfa], [0xc9, 0xa7, 0x26], [0x62, 0x2e, 0xea]];
        std::array::from_fn(|i| if i >= 22 { tail[i - 22] } else { [0; 3] })
    }

    const STATS_TEXT: &str = "Races 36\nWins 28\nPodiums 31\nClean 0\nPlayTime 22079.66\nDriving distance 726635.99 time 11778.75 overtakes 528 carhits 289 wallhits 1334 drift 58213.4 air 534.18 jumps 881\nLongestDrift 223.368\nLongestJump 2.0000055\nTopSpeed 355.58 car 4 track Goldbridge\nTrack plays 29 races 10 distance 200998.1 time 3157.4 lap 36379 car 10 name Goldbridge\nTrack plays 3 races 2 distance 43842.9 time 574.3 lap 24733 car 4 name OkkunSpeedway\n";

    #[test]
    fn career_cards_from_a_real_stats_file() {
        let st = crate::stats::Stats::parse(STATS_TEXT);
        let c = career_cards(&st, &|id| format!("Car{id}"));
        let text = |i: usize| (c[i].value.as_str(), c[i].unit, c[i].sub.as_str());
        assert_eq!(text(0), ("36", "", ""));
        assert_eq!(text(1), ("3:16:18", "", "PLAY TIME 6:07:59"));
        assert_eq!(text(2), ("726.6", "KM", "528 OVERTAKES"));
        assert_eq!((text(3), c[3].small), (("GOLDBRIDGE", "", "29 VISITS"), true));
        assert_eq!(text(4), ("00:24.733", "", "OKKUNSPEEDWAY · CAR4"));
        assert_eq!(text(5), ("356", "KM/H", "GOLDBRIDGE · CAR4"));
        assert_eq!(text(6), ("223", "M", "58.2 KM DRIFTED IN ALL"));
        assert_eq!(text(7), ("2.00", "S", "881 JUMPS · 534 S IN THE AIR"));
        assert_eq!(percent(st.win_rate()), Some(78));
        assert_eq!(percent(st.podium_rate()), Some(86));
        assert_eq!(percent(st.clean_rate()), Some(0));
    }

    #[test]
    fn career_cards_of_an_empty_stats_are_dashes_or_zero() {
        let st = crate::stats::Stats::default();
        let c = career_cards(&st, &|id| format!("Car{id}"));
        assert_eq!(c[0].value, "0");
        assert_eq!(c[3].value, "--");
        assert_eq!(c[4].value, "--:--.---");
        assert_eq!((c[4].sub.as_str(), c[5].value.as_str(), c[5].sub.as_str()), ("--", "--", ""));
        assert_eq!(percent(st.win_rate()), None);
    }

    #[test]
    fn the_stat_cards_fill_two_columns_of_four() {
        assert_eq!(stat_card(0), (62.0, 228.0, 270.0));
        assert_eq!(stat_card(3), (62.0, 516.0, 270.0));
        assert_eq!(stat_card(4), (892.0, 228.0, 326.0));
        assert_eq!(stat_card(7), (892.0, 516.0, 326.0));
    }

    #[test]
    fn the_strip_offers_ten_named_swatches() {
        let strip = shipped_strip();
        let sw = strip_swatches(&strip);
        let names: Vec<&str> = sw.iter().map(|s| s.name).collect();
        assert_eq!(names, ["Black", "Moss", "Olive", "Oxblood", "Navy", "Green", "Cobalt", "Ice", "Gold", "Violet"]);
        // every swatch's colour finds its own swatch again
        for (i, s) in sw.iter().enumerate() {
            assert_eq!(swatch_at(&strip, &sw, s.rgb), i);
        }
    }

    #[test]
    fn near_black_shows_as_black_and_an_off_strip_colour_as_its_nearest() {
        let strip = shipped_strip();
        let sw = strip_swatches(&strip);
        assert_eq!(swatch_at(&strip, &sw, [0x0a, 0, 0]), 0);
        // the default player colour is not on the strip: it is Cobalt
        assert_eq!(sw[swatch_at(&strip, &sw, [60, 100, 255])].name, "Cobalt");
    }

    #[test]
    fn a_colour_off_the_strip_is_named_custom() {
        let strip = shipped_strip();
        let sw = strip_swatches(&strip);
        assert_eq!(swatch_name(&strip, &sw, [0x2a, 0x51, 0xfd]), "Cobalt");
        assert_eq!(swatch_name(&strip, &sw, [0x0a, 0, 0]), "Black");
        assert_eq!(swatch_name(&strip, &sw, [255, 50, 40]), "Custom");
        assert_eq!(swatch_name(&strip, &sw, [60, 100, 255]), "Custom");
    }

    #[test]
    fn a_swatch_step_stops_at_the_ends() {
        let strip = shipped_strip();
        let mut c = [0, 0, 0];
        assert_eq!(swatch_step(&strip, &mut c, false), Some(ev::CURSOR_END));
        assert_eq!(swatch_step(&strip, &mut c, true), None);
        assert_eq!(c, [0x3c, 0x7b, 0x3c]);
        let mut c = [0x62, 0x2e, 0xea];
        assert_eq!(swatch_step(&strip, &mut c, true), Some(ev::CURSOR_END));
        // an off-strip colour steps from its nearest swatch (Cobalt -> Ice)
        let mut c = [60, 100, 255];
        assert_eq!(swatch_step(&strip, &mut c, true), None);
        assert_eq!(c, [0xa3, 0xfa, 0xfa]);
    }

    #[test]
    fn records_of_a_real_stats_file_by_circuit() {
        let st = crate::stats::Stats::parse(STATS_TEXT);
        let tracks = ["RedRock", "NolbyHills", "NeoCity", "Atlantika", "OkkunSpeedway", "Olympos", "RuhrStadt", "Goldbridge"];
        let cars = [(4, "Serpent"), (10, "Titan")];
        let (cols, rows) = records_table(&st, &tracks, &cars, false);
        assert_eq!(cols.len(), 9);
        assert_eq!(rows.len(), 8);
        // the circuits without an entry fade, the favourite carries the tag, the most driven circuit fills its bar
        assert!(rows[0].none && rows[1].none);
        let gold = &rows[7];
        assert!(!gold.none);
        assert!(matches!(&gold.cells[1], Cell::Name(n, Some("FAV")) if n == "GOLDBRIDGE"));
        assert!(matches!(&gold.cells[4], Cell::Text(t) if t == "00:36.379"));
        assert!(matches!(&gold.cells[5], Cell::Note(n) if n == "TITAN"));
        assert!(matches!(&gold.cells[6], Cell::Text(t) if t == "201.0 KM"));
        assert!(matches!(&gold.cells[8], Cell::Bar(s) if (*s - 1.0).abs() < 1e-6));
        let okk = &rows[4];
        assert!(matches!(&okk.cells[5], Cell::Note(n) if n == "SERPENT"));
        assert!(matches!(&okk.cells[8], Cell::Bar(s) if *s < 0.2));
    }

    #[test]
    fn records_of_a_real_stats_file_by_car() {
        let st = crate::stats::Stats::parse("Races 1\nCar 4 races 2 distance 1000 time 100 lap 24733 track OkkunSpeedway\nCar 8 races 0 distance 300 time 5\n");
        // given out of order: the table lists by id
        let cars = [(8, "Resonic"), (5, "ASC II"), (4, "Serpent")];
        let (cols, rows) = records_table(&st, &[], &cars, true);
        assert_eq!(cols[2].title, "RACES");
        assert_eq!(cols[3].title, "");
        assert!(!rows[0].none && rows[1].none && !rows[2].none);
        assert!(matches!(&rows[0].cells[5], Cell::Note(n) if n == "OKKUNSPEEDWAY"));
        // never finished a lap: dashes in the lap's own style
        assert!(matches!(&rows[2].cells[4], Cell::Text(t) if t == "--:--.---"));
        assert!(matches!(&rows[2].cells[5], Cell::Note(n) if n == "--"));
        assert!(matches!(&rows[2].cells[8], Cell::Bar(s) if (*s - 0.05).abs() < 1e-6));
    }

    #[test]
    fn records_of_empty_stats_are_all_not_driven_yet() {
        let (_, rows) = records_table(&crate::stats::Stats::default(), &["A", "B"], &[(1, "X")], false);
        assert!(rows.len() == 2 && rows.iter().all(|r| r.none));
        let (_, rows) = records_table(&crate::stats::Stats::default(), &[], &[(1, "X")], true);
        assert!(rows.len() == 1 && rows[0].none);
    }
}
