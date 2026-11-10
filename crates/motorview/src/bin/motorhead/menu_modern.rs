//! REMASTER modern front end (09-remaster 9.24): the redesigned menu screens, one by one. Drawn
//! only when `View::modern_menu()`; Classic keeps the original sprites. Sharp batch at the
//! window's aspect (`Canvas::hud`: 480 tall, edges anchor to edges). The vector shapes
//! (`ui_vector`, lyon) and the Title font (Motorhead.ttf, our own face) follow the concept
//! `reference_materials/menu-overhaul/main-menu-concept.png`; sizes below are that mockup scaled
//! by 0.384 (its 1250 px height onto the 480 px canvas) [design choice].
//!
//! Shared pieces: `backdrop`, `title`, `plate`, `dial` (+ `footer`); a screen picks its own.
//! Done: Main, Single, Options and League (`dial_modern`: the Glass HUD dial `glass_dial` and its
//! label tables, #295), the requesters (`requester.rs`, #304).

use super::*;
use crate::controls::Dir;
use motorview::ui::FontStyle;
use motorview::ui_font::{self, HudFont};
use motorview::ui_vector::{self as vec, Ends, Mesh};
use std::f32::consts::{FRAC_PI_2, TAU};

#[path = "menu_modern/car.rs"]
mod car;
#[path = "menu_modern/loading.rs"]
mod loading;
pub(crate) use loading::LoadFacts;
#[path = "menu_modern/pause_settings.rs"]
mod pause_settings;
#[path = "menu_modern/personal.rs"]
mod personal;
pub(crate) use personal::PaintPick;
#[path = "menu_modern/requester.rs"]
mod requester;
#[path = "menu_modern/results.rs"]
mod results;
#[path = "menu_modern/results_laps.rs"]
mod results_laps;
#[path = "menu_modern/results_league.rs"]
mod results_league;
#[path = "menu_modern/results_records.rs"]
mod results_records;
#[path = "menu_modern/league_load.rs"]
mod league_load;
#[path = "menu_modern/league_table.rs"]
mod league_table;
#[path = "menu_modern/replays.rs"]
pub(crate) mod replays;
// REMASTER Lap Analysis (#429): the screen and its model
#[path = "menu_modern/analysis.rs"]
mod analysis;
#[path = "menu_modern/analysis_data.rs"]
mod analysis_data;
pub(crate) use analysis_data::{Analysis, Want};
#[path = "menu_modern/rankings.rs"]
mod rankings;
#[path = "menu_modern/sound_device.rs"]
mod sound_device;
#[path = "menu_modern/sound_songs.rs"]
mod sound_songs;
#[path = "menu_modern/table.rs"]
mod table;
pub(crate) use rankings::RankSlot;
pub(crate) use requester::Ask;

const ORANGE_HI: [f32; 4] = [1.0, 0.52, 0.12, 1.0];
const LABEL_SEL: [f32; 4] = [0.78, 0.95, 1.0, 1.0];
/// the grey light blue of the tip labels; the pause menu's inactive items use it too
const TIP_GREY: [f32; 4] = [0.62, 0.8, 0.95, 0.9];

/// the mockup's dial: rim radius, centre offset from the canvas centre, centre height
const DIAL_R: f32 = 154.0;
const DIAL_DX: f32 = 11.0;
const DIAL_CY: f32 = 263.0;
/// the flat bottom chord, below the dial centre (fraction of the radius)
const CHORD: f32 = 0.5125;
/// tick sweep: from the lower-left tick over the top to the lower-right one (radians, y down)
const TICK_FROM: f32 = 146.9 * std::f32::consts::PI / 180.0;
const TICK_TO: f32 = (360.0 + 33.1) * std::f32::consts::PI / 180.0;

// REMASTER needle, tuned in tools/needle-tuner.html (design px at 1125 tall, DIAL_R = 154)
/// kite along the needle, x DIAL_R from the pivot: tip ahead, tail behind, widest point
const NEEDLE_TIP: f32 = 0.845;
const NEEDLE_TAIL: f32 = 0.185;
const NEEDLE_MID: f32 = 0.02;
const NEEDLE_HALF_WIDTH: f32 = 9.2;
/// lit and shaded facet; the tip boost lightens (+) / darkens (-) toward the tip (-1..1)
const NEEDLE_LIT: [f32; 3] = [1.0, 0.62, 0.2];
const NEEDLE_SHADE: [f32; 3] = [1.0, 0.451, 0.078];
const NEEDLE_TIP_BOOST: f32 = 0.0;
/// opacity at the tail, rising linearly to 1.0 at the tip
const NEEDLE_TAIL_ALPHA: f32 = 0.9;
const NEEDLE_EDGE: [f32; 4] = [1.0, 0.451, 0.078, 1.0];
const NEEDLE_EDGE_WIDTH: f32 = 0.8;
/// gloss streak on the lit facet: colour + strength, width (x half width), length (x tip length)
const NEEDLE_GLOSS: [f32; 4] = [1.0, 1.0, 1.0, 0.79];
const NEEDLE_GLOSS_WIDTH: f32 = 0.35;
const NEEDLE_GLOSS_LENGTH: f32 = 0.7;
/// 0 soft .. 1 hard: how abruptly the streak fades in from its tail end
const NEEDLE_GLOSS_SHARP: f32 = 0.5;
/// the needle's glow: one blurred stroke along its axis (Mesh::glow_smooth)
const NEEDLE_GLOW_ON: bool = true;
const NEEDLE_GLOW: [f32; 4] = [1.0, 0.682, 0.42, 0.2];
const NEEDLE_GLOW_RADIUS: f32 = 25.0;
/// the blurred source reads weaker than the tuner's gaussian shadow: peak gain
const NEEDLE_GLOW_GAIN: f32 = 1.5;
/// width of the glow's full-strength core along the axis, x half width
const NEEDLE_GLOW_CORE: f32 = 0.5;

/// the hologram car: view angle off its nose, distance, eye height, sideways aim shift (m)
const HOLO_ANGLE: f32 = 0.6;
const HOLO_DIST: f32 = 3.6;
const HOLO_HEIGHT: f32 = 0.5;
/// the face's frosting (#296): gaussian sigma (px at 1125 tall; the old 3 px ring of 8 taps
/// had a spread of about 0.6 of its radius), the blurred car's light
const BLUR_PX: f32 = 1.8;
const BLUR_GAIN: f32 = 1.0;
const HOLO_SHIFT: (f32, f32) = (0.3, 0.0);
/// REMASTER Glass HUD (#294): the blur behind the glass cards, x BLUR_PX
const CARD_BLUR: f32 = 2.5;
/// the drive's camera: seconds per orbit round the car; the road lines fade out by this distance (m)
const HOLO_ORBIT_PERIOD: f32 = 60.0;
const HOLO_ROAD_FADE: f32 = 30.0;
const HOLO_DRIVE_HEIGHT: f32 = 1.4;

/// The needle's angle for dial item `sel` of `n`: the items sit on every second major tick.
pub(super) fn main_needle(sel: usize, n: usize) -> f32 {
    TICK_FROM + (TICK_TO - TICK_FROM) * sel.min(n - 1) as f32 / (n - 1) as f32
}

const PER: usize = 6;
/// opacity of all text in the modern menus (PAUSE LAYOUT v1)
pub(super) const MENU_TEXT_ALPHA: f32 = 0.75;
/// One row of a modern option list (`PauseRow::from_opt`): label, control, 1.0 or dimmed.
pub(super) struct OptRow {
    pub label: String,
    pub val: RowValue,
    pub dim: f32,
}

/// What a screen puts on the plate in the top-right corner.
pub struct Plate<'a> {
    pub header: &'a str,
    pub code: &'a str,
    pub name: &'a str,
}

/// Where the five dial items sit, relative to the dial centre: lower left, upper left, top,
/// upper right, lower right (the order of the classic dials' items too).
const DIAL_SLOTS: [(f32, f32); 5] = [
    (-212.0, 96.0),
    (-225.0, -85.0),
    (2.0, -172.0),
    (192.0, -85.0),
    (190.0, 89.0),
];

/// The Single dial's six slots (Knockout between time attack and ghost mode): the same ellipse
/// as `DIAL_SLOTS`, at the six needle angles [design choice].
const SINGLE_SLOTS: [(f32, f32); 6] = [
    (-210.0, 96.0),
    (-235.0, -49.0),
    (-100.0, -156.0),
    (96.0, -156.0),
    (213.0, -50.0),
    (190.0, 89.0),
];

/// The dial's items by name, in their slots.
fn dial_labels(names: [&str; 5]) -> Vec<(&str, f32, f32)> {
    (0..5).map(|i| (names[i], DIAL_SLOTS[i].0, DIAL_SLOTS[i].1)).collect()
}

/// The summary box under the Single and League dials: two columns of label and value.
pub(super) type DialInfo = [[(&'static str, String); 3]; 2];

impl Game {
    /// REMASTER dial screens in the Glass HUD (19-glass-hud-design-language, #295): Main, Single,
    /// Options and League share the dial, the hologram, the plate and the furniture; each brings
    /// its labels. The first three Single / League items are the classic dials' order [design
    /// choice: the same slots].
    pub(super) fn dial_modern(&self, size: (u32, u32)) -> Vec<Layer> {
        let canvas = Canvas::hud(size);
        let px = 1.0 / canvas.transform(size).0;
        let d = Glass::new(&canvas, px);
        // Options' fifth item is the difficulty, shown with its value (Enter cycles it)
        let difficulty = format!(
            "DIFFICULTY: {}",
            difficulty_name(self.settings.difficulty).to_uppercase()
        );
        // title, header line, the plate's screen name, the strip's tag, the items, the summary
        let (title, header, name, tag, labels, info) = match self.screen {
            Screen::Single => (
                "single",
                "SINGLE PLAYER // MODE DIAL 2B",
                "SINGLESCREEN",
                "RACE SETTINGS",
                ["QUICK RACE", "SINGLE RACE", "LEAGUE RACE", "TIME ATTACK", "KNOCKOUT", "GHOST MODE"]
                    .iter()
                    .zip(SINGLE_SLOTS)
                    .map(|(n, (x, y))| (*n, x, y))
                    .collect::<Vec<_>>(),
                Some(self.single_info()),
            ),
            Screen::Options => (
                "options",
                "OPTIONS // SYSTEM DIAL 3C",
                "OPTIONS",
                "",
                dial_labels(["PERSONAL", "GRAPHICS", "CONTROLS", "SOUND", &difficulty]),
                None,
            ),
            Screen::League => (
                "league",
                "LEAGUE // SEASON DIAL 4D",
                "LEAGUESCREEN",
                "LEAGUE",
                dial_labels(["CONTINUE", "VIEW STANDINGS", "LOAD", "NEW", "DELETE"]),
                self.league_info_rows(),
            ),
            _ => (
                "main",
                "MAIN MENU // COMMAND DIAL 1A",
                "MAINSCREEN",
                "",
                dial_labels(["ONE PLAYER", "MULTI PLAYER", "RANKINGS", "OPTIONS", "CREDITS"]),
                None,
            ),
        };
        let centre = [d.x(GLASS_DIAL_C.0), d.y(GLASS_DIAL_C.1)];
        // the glass cards (canvas px) the hologram is blurred behind, and the dial's face
        let plate = (d.r(1021.0), d.t(GLASS_PLATE_TOP), d.d(234.0), d.d(119.0));
        let strip = (d.x(327.0), d.y(615.0), d.d(660.0), d.d(82.0));
        let mut blur = vec![(plate.0, plate.1, plate.2, plate.3, d.d(12.0))];
        if info.is_some() {
            blur.push((strip.0, strip.1, strip.2, strip.3, d.d(12.0)));
        }
        let face = d.d(GLASS_DIAL_R - GLASS_DIAL_BEZEL - GLASS_DIAL_GAP);
        let back = self.ui_in(Batch::sharp_text(&self.assets, canvas, 1.0), |_, b| Self::glass_backdrop(b, d));
        let front = self.ui_in(Batch::sharp_text(&self.assets, canvas, 1.0), |g, b| {
            Self::glass_overlays(b, d);
            Self::glass_furniture(b, d, g.t, header, "LIVE", GlassRules::None);
            Self::glass_title(b, d, title, "88-137:59");
            Self::glass_plate(b, d, plate, "187-C-35", name);
            g.glass_dial(b, d, centre, &labels);
            // a requester over the dial brings its own hints; the strip and the copyright line give way to them
            if !g.asking() {
                Self::glass_tips(b, d, g.controls.last, GLASS_DIAL_TIPS, None);
                match &info {
                    Some(info) => Self::glass_settings(b, d, strip, tag, info),
                    None => Self::glass_copyright(b, d),
                }
            }
            Self::glass_version(b, d);
        });
        let mut out = vec![back];
        out.extend(self.hologram_dial(size, None, &blur, Some((centre, face))));
        out.push(front);
        out
    }

    /// The selected car as a slowly swaying hologram behind the dial (the garage scene's
    /// meshes, `Frame::hologram`); none until the scene is loaded.
    fn hologram(&self, size: (u32, u32), panel: Option<(f32, f32, f32, f32)>, cards: &[(f32, f32, f32, f32, f32)]) -> Vec<Layer> {
        self.hologram_dial(size, panel, cards, None)
    }

    /// `hologram` with the Glass dial's face (canvas px: centre, radius) frosted next to the cards.
    fn hologram_dial(&self, size: (u32, u32), panel: Option<(f32, f32, f32, f32)>, cards: &[(f32, f32, f32, f32, f32)], dial: Option<([f32; 2], f32)>) -> Vec<Layer> {
        // the planar tracking camera round the driving car, or the parked garage car (fallback)
        let (spot, dir, cars, road, a, height) = match (&self.menu_drive, self.loaded) {
            (Some(d), Loaded::MenuDrive) => {
                // the angle off the car's nose drifts round it (one turn per HOLO_ORBIT_PERIOD)
                (d.pos, d.dir, d.sess.models(), HOLO_ROAD_FADE, HOLO_ANGLE + TAU * self.clock / HOLO_ORBIT_PERIOD, HOLO_DRIVE_HEIGHT)
            }
            (_, Loaded::Garage) => {
                let (spot, dir) = self.spot;
                let k = self.settings.car;
                // car meshes face +z in game space, i.e. -z in view space
                let yaw_car = (-dir.x).atan2(-dir.z);
                let model = Mat4::from_translation(spot) * Mat4::from_mat3(Mat3::from_rotation_y(yaw_car));
                let mut cars = vec![(k * CAR_MESHES, model)];
                if let Some((hubs, _)) = self.garage_wheels.get(k) {
                    cars.extend(hubs.iter().enumerate().map(|(w, h)| (k * CAR_MESHES + 1 + w, model * Mat4::from_translation(*h))));
                }
                (spot, dir, cars, 0.0, HOLO_ANGLE + 0.08 * (self.clock * 0.3).sin(), HOLO_HEIGHT)
            }
            _ => return Vec::new(),
        };
        let fwd = Vec3::new(dir.x, 0.0, dir.z).normalize_or(Vec3::Z);
        let side = Vec3::new(fwd.z, 0.0, -fwd.x);
        let eye = spot + (fwd * a.cos() + side * a.sin()) * HOLO_DIST + Vec3::Y * height;
        // aim past the car so it sits left and low, behind the dial's lower left
        let aim_right = Vec3::new((spot - eye).z, 0.0, -(spot - eye).x).normalize_or(Vec3::X);
        let at =
            spot + Vec3::Y * 0.7 + aim_right * if panel.is_some() { 0.0 } else { HOLO_SHIFT.0 };
        let look = (at - eye).normalize();
        let (yaw, pitch) = (look.x.atan2(-look.z), look.y.asin());
        // the dial's face in window pixels: sharp outside, a blurred copy inside (frosted glass)
        let canvas = Canvas::hud(size);
        let (s, ox, oy) = canvas.transform(size);
        let c = match dial {
            Some((dc, _)) => [ox + dc[0] * s, oy + dc[1] * s],
            None => [ox + (canvas.w * 0.5 + DIAL_DX) * s, oy + DIAL_CY * s],
        };
        // the dial's face, or on the Graphics screen the options panel (frosted glass)
        let (pc, rect) = if let Some((x, y, w, h)) = panel {
            (
                [ox + (x + w * 0.5) * s, oy + (y + h * 0.5) * s],
                Some(([w * 0.5 * s, h * 0.5 * s], 10.0 * s)),
            )
        } else {
            (c, None)
        };
        // REMASTER Glass HUD (#294): the cards' rectangles (canvas px -> window px) join the clip; with
        // cards and no panel there is no dial face: an empty circle
        let mut rects = [([0.0; 2], [0.0; 2]); motorview::render::HOLO_RECTS];
        for (i, &(x, y, w, h, _)) in cards.iter().take(rects.len()).enumerate() {
            rects[i] = ([ox + (x + w * 0.5) * s, oy + (y + h * 0.5) * s], [w * 0.5 * s, h * 0.5 * s]);
        }
        let rects_n = cards.len().min(rects.len());
        let rects_r = cards.first().map_or(0.0, |c| c.4 * s);
        let (radius, chord_y) = match dial {
            Some((_, r)) => (r * s, c[1] + CHORD * r * s),
            None if panel.is_none() && !cards.is_empty() => (0.0, -1.0),
            None => (DIAL_R * s, c[1] + CHORD * DIAL_R * s),
        };
        // REMASTER frosted glass (#296): one pass, blurred inside the clip by the renderer
        let blur = BLUR_PX * size.1 as f32 / 1125.0 * if cards.is_empty() { 1.0 } else { CARD_BLUR };
        let clip = motorview::render::HoloClip { centre: pc, radius, chord_y, inside: true, gain: BLUR_GAIN, rect, rects, rects_n, rects_r, blur };
        vec![Layer::Holo { cam: Camera::new(eye, yaw, pitch), cars, clip: Some(Box::new(clip)), road }]
    }

    /// The dark backdrop: a navy gradient, a cool pool of light behind the dial's lower left
    /// and the halftone dots in the top-left corner.
    fn backdrop(b: &mut Batch, centre: [f32; 2], px: f32) {
        let (w, h) = (b.canvas.w, b.canvas.h);
        let mut m = Mesh::default();
        let top = [0.02, 0.03, 0.05, 1.0];
        let bot = [0.015, 0.04, 0.08, 1.0];
        m.quad(0.0, 0.0, w, h, [top, top, bot, bot]);
        m.disc([centre[0] - 150.0, centre[1] + 150.0], 330.0, [0.03, 0.2, 0.55, 0.40], [0.03, 0.2, 0.55, 0.0], 96);
        m.disc([centre[0] + 60.0, centre[1] - 30.0], 380.0, [0.05, 0.14, 0.25, 0.30], [0.05, 0.14, 0.25, 0.0], 96);
        // halftone: a lattice of dots that shrink away from the corner
        let (step, reach) = (22.0, 150.0);
        for j in 0..8 {
            for i in 0..8 {
                let (x, y) = (14.0 + (i as f32 + 0.5 * (j % 2) as f32) * step, 12.0 + j as f32 * step * 0.9);
                let d = (x * x + y * y).sqrt() / reach;
                if d < 1.0 {
                    let r = 9.0 * (1.0 - d).powf(0.8) + 0.8;
                    let a = 0.35 + 0.5 * (1.0 - d);
                    // a feathered fill: smooth round edges at any size
                    m.fill(&vec::arc([x, y], r, 0.0, TAU, 40)[..40], [0.42, 0.46, 0.52, a], px);
                }
            }
        }
        b.mesh(&m);
    }

    /// The screen title in the Motorhead face (lowercase italic, orange) with its shadow, and
    /// the small orange code under it.
    fn title(b: &mut Batch, text: &str, code: &str) {
        // long names (graphics) are set smaller so they stay clear of the plate
        let size = if text.len() > 6 { 28.0 } else { 42.0 };
        let (x, y) = (42.0, if text.len() > 6 { 36.0 } else { 22.0 });
        let style = FontStyle::new(size, ORANGE_HI).tracking(0.5);
        b.font_text(HudFont::Title, text, x + 2.5, y + 3.5, &FontStyle::new(size, [0.0, 0.0, 0.0, 0.55]).tracking(0.5));
        b.font_text(HudFont::Title, text, x, y, &style);
        b.font_text(HudFont::Clock, code, x + 42.0, y + size + 3.0, &FontStyle::new(11.0, [1.0, 0.5, 0.15, 0.95]).tracking(0.5));
    }

    /// The glass plate in the top-right corner: a header block with the screen's name, a
    /// barcode, and the screen's italic name along the bottom.
    fn plate(b: &mut Batch, p: &Plate, px: f32) {
        let (w, h) = (191.0, 80.0);
        let (x, y) = (b.canvas.w - 17.0 - w, 19.0);
        let mut m = Mesh::default();
        let body = vec::rounded_rect(x, y, w, h, 6.0);
        // glass body: lighter at the top
        m.fill_with(&body, &|q| {
            let t = ((q[1] - y) / h).clamp(0.0, 1.0);
            [0.20 - 0.08 * t, 0.23 - 0.08 * t, 0.27 - 0.08 * t, 0.86]
        }, px);
        m.stroke(&body, true, 1.4, Ends::Round, [0.62, 0.68, 0.75, 0.8], px);
        // the header block with the screen's name
        let head = vec::rounded_rect(x + 2.0, y + 2.0, 80.0, 36.0, 4.0);
        m.fill(&head, [0.42, 0.45, 0.48, 0.92], px);
        // barcode
        let mut bx = x + w - 62.0;
        let mut k = 7u32;
        while bx < x + w - 5.0 {
            k = k.wrapping_mul(1_103_515_245).wrapping_add(12_345);
            let bw = 0.9 + (k >> 24) as f32 % 3.0 * 0.7;
            m.quad(bx, y + 3.0, bw, 33.0, [[0.72, 0.76, 0.8, 0.9]; 4]);
            bx += bw + 0.9 + (k >> 20) as f32 % 2.0;
        }
        // lower rail: the dot at the right
        m.fill(&vec::arc([x + w - 11.0, y + h - 14.0], 5.0, 0.0, TAU, 20), [0.62, 0.66, 0.7, 0.85], px);
        b.mesh(&m);
        b.font_text(HudFont::Label, p.header, x + 7.0, y + 6.0, &FontStyle::new(14.0, [0.10, 0.12, 0.15, 1.0]));
        b.font_text(HudFont::Label, p.code, x + 7.0, y + 24.0, &FontStyle::new(8.0, [0.12, 0.14, 0.17, 1.0]).tracking(1.0));
        let tag = FontStyle::new(5.0, [0.7, 0.74, 0.78, 0.9]).tracking(1.0);
        let tw = b.font_width(HudFont::Label, "MOTORHEAD.", &tag);
        b.font_text(HudFont::Label, "MOTORHEAD.", x + w - 6.0 - tw, y + 39.0, &tag);
        let sub = FontStyle::new(8.0, [0.55, 0.6, 0.66, 0.9]).tracking(2.0);
        let sw = b.font_width(HudFont::Label, "TUNNELVISION", &sub);
        b.font_text(HudFont::Label, "TUNNELVISION", x + (w - sw) * 0.5, y + 47.0, &sub);
        b.font_text(HudFont::Title, p.name, x + 9.0, y + h - 19.0, &FontStyle::new(9.0, [0.55, 0.6, 0.65, 0.95]).tracking(0.3));
    }

    /// A row of glyph + label tips (`glyph_for` kinds), centred on `at` or starting at it.
    fn tip_row(b: &mut Batch, device: Device, tips: &[(&str, usize)], at: f32, cy: f32, centred: bool) {
        let text = FontStyle::new(11.0, TIP_GREY).tracking(1.0);
        let (gap, label_gap) = (22.0, 5.0);
        let glyphs: Vec<(char, [f32; 4])> = tips.iter().map(|&(_, k)| Self::glyph_for(device, k)).collect();
        let size = |c: char| FontStyle::new(if matches!(c, '\u{242E}' | '\u{242F}' | '\u{243A}') { 7.5 } else { 8.5 }, WHITE);
        let widths: Vec<f32> = tips.iter().zip(&glyphs).map(|(&(t, _), &(c, _))| b.font_width(HudFont::Glyphs, &c.to_string(), &size(c)) + label_gap + b.font_width(HudFont::Label, t, &text)).collect();
        let total = widths.iter().sum::<f32>() + gap * (tips.len() - 1) as f32;
        let mut x = if centred { at - total * 0.5 } else { at };
        for ((&(label, _), &(c, tint)), w) in tips.iter().zip(&glyphs).zip(widths) {
            let mut st = size(c);
            st.color = tint;
            let gw = b.font_text(HudFont::Glyphs, &c.to_string(), x, cy - st.size * 0.5, &st);
            b.font_text(HudFont::Label, label, x + gw + label_gap, cy - 7.0, &text);
            x += w + gap;
        }
    }

    /// The PromptFont glyph and tint of one tip (0 left, 1 right, 2 select, 3 back).
    fn glyph_for(device: Device, kind: usize) -> (char, [f32; 4]) {
        const CYAN_G: [f32; 4] = [0.7, 0.92, 1.0, 1.0];
        // kinds 4..7: tab left, tab right, apply, restore defaults
        const YELLOW: [f32; 4] = [0.95, 0.8, 0.25, 1.0];
        const BLUE: [f32; 4] = [0.35, 0.6, 1.0, 1.0];
        match (device, kind) {
            (Device::Keyboard, 0) => ('\u{23F4}', CYAN_G),
            (Device::Keyboard, 1) => ('\u{23F5}', CYAN_G),
            (Device::Keyboard, 2) => ('\u{242E}', CYAN_G),
            (Device::Keyboard, 3) => ('\u{242F}', CYAN_G),
            (Device::Keyboard, 4) => ('\u{FF31}', CYAN_G),
            (Device::Keyboard, 5) => ('\u{FF25}', CYAN_G),
            (Device::Keyboard, 6) => ('\u{243A}', CYAN_G),
            // kind 8: the pad's Start (cancels a pad binding); Esc on the keyboard
            (Device::Keyboard, 8) => ('\u{242F}', CYAN_G),
            // kinds 11, 12: the A and Z caps of a typing hint (#91)
            (Device::Keyboard, 11) => ('\u{FF21}', CYAN_G),
            (Device::Keyboard, 12) => ('\u{FF3A}', CYAN_G),
            // kind 13: the X cap (the Results screen's WATCH); the pad's X button is the fall-through below
            (Device::Keyboard, 13) => ('\u{FF38}', CYAN_G),
            // kinds 14, 15: the Rankings' difficulty keys, Z / C and the pad's LT / RT (#93)
            (Device::Keyboard, 14) => ('\u{FF3A}', CYAN_G),
            (Device::Keyboard, 15) => ('\u{FF23}', CYAN_G),
            // kind 16: the Delete key (League Load's DELETE); the pad's X button is the fall-through below
            (Device::Keyboard, 16) => ('\u{2437}', CYAN_G),
            (Device::Xbox, 14) => ('\u{2196}', CYAN_G),
            (Device::Xbox, 15) => ('\u{2197}', CYAN_G),
            (Device::PlayStation, 14) => ('\u{21B2}', CYAN_G),
            (Device::PlayStation, 15) => ('\u{21B3}', CYAN_G),
            // kinds 9, 10: up, down
            (Device::Keyboard, 9) => ('\u{23F6}', CYAN_G),
            (Device::Keyboard, 10) => ('\u{23F7}', CYAN_G),
            (Device::Xbox, 9) => ('\u{227B}', CYAN_G),
            (Device::Xbox, 10) => ('\u{227D}', CYAN_G),
            (Device::PlayStation, 9) => ('\u{219F}', CYAN_G),
            (Device::PlayStation, 10) => ('\u{21A1}', CYAN_G),
            (Device::Xbox, 8) => ('\u{21FB}', CYAN_G),
            (Device::PlayStation, 8) => ('\u{21E8}', CYAN_G),
            // kind 17: the Ghost screen's ANALYSE (#429): the F cap, the pad's Y / triangle (it arrives as F5)
            (Device::Keyboard, 17) => ('\u{FF26}', CYAN_G),
            (Device::Xbox, 17) => ('\u{21D1}', YELLOW),
            (Device::PlayStation, 17) => ('\u{21E1}', [0.4, 0.85, 0.6, 1.0]),
            (Device::Keyboard, _) => ('\u{FF32}', CYAN_G),
            (Device::Xbox, 0) => ('\u{227A}', CYAN_G),
            (Device::Xbox, 1) => ('\u{227C}', CYAN_G),
            (Device::Xbox, 2) => ('\u{21D3}', [0.5, 0.85, 0.35, 1.0]),
            (Device::Xbox, 3) => ('\u{21D2}', [0.95, 0.4, 0.33, 1.0]),
            (Device::Xbox, 4) => ('\u{2198}', CYAN_G),
            (Device::Xbox, 5) => ('\u{2199}', CYAN_G),
            (Device::Xbox, 6) => ('\u{21D1}', YELLOW),
            (Device::Xbox, _) => ('\u{21D0}', BLUE),
            (Device::PlayStation, 0) => ('\u{219E}', CYAN_G),
            (Device::PlayStation, 1) => ('\u{21A0}', CYAN_G),
            (Device::PlayStation, 2) => ('\u{21E3}', [0.55, 0.72, 1.0, 1.0]),
            (Device::PlayStation, 3) => ('\u{21E2}', [1.0, 0.45, 0.45, 1.0]),
            (Device::PlayStation, 4) => ('\u{21B0}', CYAN_G),
            (Device::PlayStation, 5) => ('\u{21B1}', CYAN_G),
            (Device::PlayStation, 6) => ('\u{21E1}', [0.4, 0.85, 0.6, 1.0]),
            (Device::PlayStation, _) => ('\u{21E0}', [0.9, 0.5, 0.8, 1.0]),
        }
    }

    /// Copyright line and version.
    fn footer(b: &mut Batch, legal: bool, px: f32) {
        if legal {
            Self::copyright(b, px);
        }
        let ver = motor_version::DISPLAY;
        let vw = b.font_width(
            HudFont::Label,
            ver,
            &FontStyle::new(10.0, WHITE).tracking(1.0),
        );
        b.font_text(
            HudFont::Label,
            ver,
            b.canvas.w - 17.0 - vw,
            458.0,
            &FontStyle::new(10.0, [0.4, 0.5, 0.6, 0.7]).tracking(1.0),
        );
    }

    fn copyright(b: &mut Batch, px: f32) {
        let (cx, y) = (b.canvas.w * 0.5 + DIAL_DX, 433.0);
        let style = FontStyle::new(11.0, [0.35, 0.45, 0.55, 0.75]).tracking(1.0);
        let text = "digital illusions cs 1997-98";
        let w = b.font_width(HudFont::Label, text, &style);
        let x = cx - w * 0.5 + 6.0;
        let mut m = Mesh::default();
        m.stroke(&vec::arc([x - 14.0, y + 6.0], 7.0, 0.0, TAU, 28), true, 1.2, Ends::Round, [0.35, 0.45, 0.55, 0.7], px);
        b.mesh(&m);
        b.font_text(HudFont::Label, "C", x - 17.0, y + 2.5, &FontStyle::new(7.0, [0.35, 0.45, 0.55, 0.75]));
        b.font_text(HudFont::Label, text, x, y, &style);
    }

    /// The pause menu windows' material [design choice]: frosted glass, a neutral cool grey
    /// instead of the dial's blue, milky towards the top with a pale rim. (The race behind is
    /// not blurred: that would need a backdrop pass.)
    fn frost(m: &mut Mesh, (x, y, w, h): (f32, f32, f32, f32), r: f32, px: f32) {
        let outline = vec::rounded_rect(x, y, w, h, r);
        m.fill_with(&outline, &|q| {
            let t = ((q[1] - y) / h).clamp(0.0, 1.0);
            [0.17 - 0.08 * t, 0.19 - 0.08 * t, 0.22 - 0.08 * t, 0.62 + 0.06 * t]
        }, px);
        // the milky sheen over the upper half
        m.fill_with(&outline, &|q| {
            let t = ((q[1] - y) / h).clamp(0.0, 1.0);
            [0.9, 0.95, 1.0, 0.16 * (1.0 - t * 1.6).max(0.0)]
        }, px);
        m.glow(&outline, true, 1.5, 5.0, [0.55, 0.78, 1.0, 0.24], 5);
        m.stroke(&outline, true, 1.2, Ends::Round, [0.65, 0.84, 0.98, 0.78], px);
        m.stroke(&[[x + 18.0, y + 4.0], [x + w * 0.5, y + 4.0]], false, 2.0, Ends::Round, [1.0, 1.0, 1.0, 0.16], px);
    }

    /// REMASTER Graphics screen in the Glass HUD (#304 phase 1): the pause tab's rows card and info
    /// card (`pause_settings`) under the screen's title and plate, Display / Detail as page tabs
    /// over the list, the hologram blurred behind the cards.
    pub(super) fn graphics_modern(&self, size: (u32, u32)) -> Vec<Layer> {
        let canvas = Canvas::hud(size);
        let px = 1.0 / canvas.transform(size).0;
        let d = Glass::new(&canvas, px);
        let cards = Self::options_cards(d);
        let back = self.ui_in(Batch::sharp_text(&self.assets, canvas, 1.0), |_, b| Self::glass_backdrop(b, d));
        let front = self.ui_in(Batch::sharp_text(&self.assets, canvas, 1.0), |g, b| {
            let pulse = 0.5 - 0.5 * (g.t * TAU / 1.1).cos();
            Self::glass_options_frame(b, d, g.t, "graphics", "OPTIONS // GRAPHICS", "88-137:62", "GRAPHICSCREEN", "187-C-62");
            let names: Vec<&str> = MODERN_GFX_PAGES.iter().map(|p| p.0).collect();
            g.glass_page_tabs(b, d, &names, g.gfx.map_or(0, |e| e.page));
            g.settings_screen_body(b, d, pulse);
        });
        let mut out = vec![back];
        out.extend(self.hologram(size, None, &cards));
        out.push(front);
        out
    }

    /// The glass cards of a main menu settings screen (canvas px, with the corner radius): the plate,
    /// the rows card and the info card. The hologram is blurred behind them.
    fn options_cards(d: Glass) -> Vec<(f32, f32, f32, f32, f32)> {
        let plate = (d.r(1021.0), d.t(GLASS_PLATE_TOP), d.d(234.0), d.d(119.0));
        [plate].into_iter().chain([pause_settings::ROWS_CARD, pause_settings::INFO_CARD].map(|(x, y, w, h)| (d.x(x), d.y(y), d.d(w), d.d(h)))).map(|(x, y, w, h)| (x, y, w, h, d.d(12.0))).collect()
    }

    /// The Glass frame of a main menu settings screen: the overlays, the furniture, the title, the
    /// plate and the version. `code` is the title's code line, `plate_code` the plate's.
    #[allow(clippy::too_many_arguments)]
    fn glass_options_frame(b: &mut Batch, d: Glass, t: f32, title: &str, header: &str, code: &str, plate_name: &str, plate_code: &str) {
        Self::glass_overlays(b, d);
        Self::glass_furniture(b, d, t, header, "LIVE", GlassRules::None);
        Self::glass_title(b, d, title, code);
        Self::glass_plate(b, d, (d.r(1021.0), d.t(GLASS_PLATE_TOP), d.d(234.0), d.d(119.0)), plate_code, plate_name);
        Self::glass_version(b, d);
    }

    /// The page tabs over a settings list (concept A's `.subtab`): the shoulder keys at the ends, the
    /// names between, the current one white with a glowing orange underline [design choice: the
    /// pause mockup has no pages inside a tab].
    fn glass_page_tabs(&self, b: &mut Batch, d: Glass, names: &[&str], page: usize) {
        let mid = d.y(192.0);
        let mut x = d.x(62.0);
        let mut m = Mesh::default();
        let mut runs = Vec::new();
        x += d.d(26.0 + 16.0);
        for (i, name) in names.iter().enumerate() {
            let on = i == page;
            let st = d.font(HudFont::Glass, 18.0, 1.0, hex(if on { 0xffffff } else { G_DIM }, 1.0));
            let name = name.to_uppercase();
            let w = b.font_width(HudFont::Glass, &name, &st) - st.tracking;
            if on {
                let line = vec::rounded_rect(x, mid + d.d(14.0), w, d.d(2.0), d.d(1.0));
                m.halo(&line, d.d(8.0), hex(G_ORANGE, 0.5));
                m.fill(&line, hex(G_ORANGE, 1.0), d.px);
            }
            runs.push((name, x, st));
            x += w + d.d(28.0);
        }
        b.mesh(&m);
        for (name, x, st) in &runs {
            b.font_text(HudFont::Glass, name, *x, d.cap_top(HudFont::Glass, 18.0, mid - d.d(14.0), Some(28.0)), st);
        }
        self.glass_shoulder(b, d, [d.x(62.0 + 13.0), mid], "Q", 4);
        self.glass_shoulder(b, d, [x - d.d(28.0) + d.d(16.0 + 13.0), mid], "E", 5);
    }

    /// A shoulder key at `c`: the Q / E cap on the keyboard, the pad's LB / RB glyph (`kind` 4 / 5) otherwise.
    fn glass_shoulder(&self, b: &mut Batch, d: Glass, c: [f32; 2], letter: &str, kind: usize) {
        if self.controls.last == Device::Keyboard {
            Self::glass_kcap(b, d, c, letter);
        } else {
            let (g, tint) = Self::glyph_for(self.controls.last, kind);
            let gs = FontStyle::new(d.d(18.0), tint);
            let gw = b.font_width(HudFont::Glyphs, &g.to_string(), &gs);
            b.font_text(HudFont::Glyphs, &g.to_string(), c[0] - gw * 0.5, c[1] - gs.size * 0.5, &gs);
        }
    }

    /// REMASTER Credits (14-menu-overhaul #2): the dial screens' backdrop, title and plate, the
    /// staff list on a frosted glass panel over the hologram car [design choice: no scrolling,
    /// the list is 12 lines].
    pub(super) fn credits_modern(&self, size: (u32, u32)) -> Vec<Layer> {
        let canvas = Canvas::hud(size);
        let px = 1.0 / canvas.transform(size).0;
        let rect = Self::credits_rect(canvas.w);
        let back = self.ui_in(
            Batch::sharp_text(&self.assets, canvas, MENU_TEXT_ALPHA),
            |_, b| Self::backdrop(b, [canvas.w * 0.5, 263.0], px),
        );
        let front = self.ui_in(
            Batch::sharp_text(&self.assets, canvas, MENU_TEXT_ALPHA),
            |g, b| {
                Self::title(b, "credits", "88-137:64");
                Self::plate(
                    b,
                    &Plate {
                        header: "SCREEN",
                        code: "187-C-64",
                        name: "creditscreen",
                    },
                    px,
                );
                let (x, y, w, h) = rect;
                let mut m = Mesh::default();
                Self::frost(&mut m, rect, 10.0, px);
                b.mesh(&m);
                // headings (end with ':') in orange, the names lit; the original's 0x99 is an o-umlaut, missing in the Label face: plain o
                let lines: Vec<&String> = g.credits_page().iter().collect();
                let head = FontStyle::new(11.0, ORANGE_HI).tracking(1.0);
                let name = FontStyle::new(9.5, LABEL_SEL).tracking(0.5);
                let gap = 10.0;
                let total: f32 = lines
                    .iter()
                    .enumerate()
                    .map(|(i, l)| 15.0 + if l.ends_with(':') && i > 0 { gap } else { 0.0 })
                    .sum();
                let mut ty = y + (h - total) * 0.5;
                for (i, l) in lines.iter().enumerate() {
                    let heading = l.ends_with(':');
                    if heading && i > 0 {
                        ty += gap;
                    }
                    let t = l.replace('\u{99}', "o").to_uppercase();
                    let st = if heading { &head } else { &name };
                    let tw = b.font_width(HudFont::Label, &t, st) - st.tracking;
                    b.font_text(HudFont::Label, &t, x + (w - tw) * 0.5, ty, st);
                    ty += 15.0;
                }
                Self::tip_row(
                    b,
                    g.controls.last,
                    &[("BACK", 3)],
                    b.canvas.w * 0.5 + DIAL_DX,
                    y + h + 24.0,
                    true,
                );
                Self::footer(b, true, px);
            },
        );
        let mut out = vec![back];
        out.extend(self.hologram(size, Some(rect), &[]));
        out.push(front);
        out
    }

    /// The Credits panel: x, y, width, height.
    fn credits_rect(canvas_w: f32) -> (f32, f32, f32, f32) {
        let w = 440.0;
        ((canvas_w - w) * 0.5, 120.0, w, 250.0)
    }

    /// The Sound items in the modern Sound screen's order, each with its help text: the three
    /// volumes (dials there, sliders in the pause tab), CD music, CD loop, menu CD track, engine sound.
    fn sound_items(&self) -> Vec<(OptRow, &'static str)> {
        let row = |label: &str, val, help| (OptRow { label: label.into(), val, dim: 1.0 }, help);
        let sw = |on| RowValue::Control(Control::Switch(on));
        let pct = |v: u32| RowValue::Control(Control::Slider(v as f32 / 100.0, format!("{v}%")));
        let s = &self.settings;
        vec![
            row("SFX volume", pct(s.vol_sfx), "Engines, tyres, crashes and the other effects."),
            row("CD volume", pct(s.vol_music), "The music from the CD tracks."),
            row("Ambient volume", pct(s.vol_ambient), "The sounds of the track and the crowd."),
            row("CD music", sw(s.music), "Plays the CD soundtrack in menus and races."),
            row("CD loop", sw(s.cd_loop), "Repeats the CD track that is playing."),
            row("Menu CD track", self.menu_track_choice(), "The CD track played in the menus: the tracks found in the Music folder."),
            row("Engine sound", RowValue::Choices(vec!["Classic".into(), "Simulated".into()], s.engine_sound as usize), "Classic: the game's engine recordings. Simulated: an engine simulation (cars with a sound pack in remaster/engines)."),
        ]
    }

    /// The menu CD track as a choice over the tracks found (`step_cd_track` walks the same list).
    fn menu_track_choice(&self) -> RowValue {
        if self.cd_tracks.is_empty() {
            return RowValue::Choices(vec!["No CD tracks".into()], 0);
        }
        let at = self.cd_tracks.iter().position(|&t| t >= self.settings.menu_cd_track).unwrap_or(self.cd_tracks.len() - 1);
        RowValue::Choices(self.cd_tracks.iter().map(|t| format!("Track {t}")).collect(), at)
    }

    /// REMASTER Sound in the Glass HUD (#304 phase 2, #259): under the screen's title and plate a tab bar
    /// with Volume = the seven rows of `sound_items` in the settings body (item `i` is row `i`) and Songs =
    /// the song-to-track table (`songs_body`).
    pub(super) fn sound_modern(&self, size: (u32, u32)) -> Vec<Layer> {
        let canvas = Canvas::hud(size);
        let px = 1.0 / canvas.transform(size).0;
        let d = Glass::new(&canvas, px);
        let mut cards = Self::options_cards(d);
        cards.push((d.x(62.0), d.y(160.0), d.d(1156.0), d.d(52.0), d.d(12.0)));
        let back = self.ui_in(Batch::sharp_text(&self.assets, canvas, 1.0), |_, b| Self::glass_backdrop(b, d));
        let front = self.ui_in(Batch::sharp_text(&self.assets, canvas, 1.0), |g, b| {
            let pulse = 0.5 - 0.5 * (g.t * TAU / 1.1).cos();
            Self::glass_options_frame(b, d, g.t, "sound", "OPTIONS // SOUND", "88-137:63", "SOUNDSCREEN", "187-C-63");
            g.glass_tabbar(b, d, &sound_songs::SOUND_TABS, g.sound_page);
            if g.sound_page == SOUND_SONGS {
                g.songs_body(b, d, pulse);
            } else {
                g.settings_screen_body(b, d, pulse);
            }
        });
        let mut out = vec![back];
        out.extend(self.hologram(size, None, &cards));
        out.push(front);
        out
    }

    /// REMASTER Controls in the Glass HUD (#304 phase 2): the pause tab's one list (car tasks with
    /// key and pad caps, the keyboard layout, the pad's sliders).
    pub(super) fn controls_modern(&self, size: (u32, u32)) -> Vec<Layer> {
        self.settings_screen(size, "controls", "CONTROLS", "88-137:65", "CONTROLSCREEN", "187-C-65")
    }

    /// A main menu settings screen without pages: the Glass frame and the settings body.
    fn settings_screen(&self, size: (u32, u32), title: &str, header: &str, code: &str, plate_name: &str, plate_code: &str) -> Vec<Layer> {
        let canvas = Canvas::hud(size);
        let px = 1.0 / canvas.transform(size).0;
        let d = Glass::new(&canvas, px);
        let cards = Self::options_cards(d);
        let back = self.ui_in(Batch::sharp_text(&self.assets, canvas, 1.0), |_, b| Self::glass_backdrop(b, d));
        let front = self.ui_in(Batch::sharp_text(&self.assets, canvas, 1.0), |g, b| {
            let pulse = 0.5 - 0.5 * (g.t * TAU / 1.1).cos();
            Self::glass_options_frame(b, d, g.t, title, &format!("OPTIONS // {header}"), code, plate_name, plate_code);
            g.settings_screen_body(b, d, pulse);
        });
        let mut out = vec![back];
        out.extend(self.hologram(size, None, &cards));
        out.push(front);
        out
    }

    /// REMASTER pause menu (#300, concept C): the frame (title, furniture, plate, tab bar), the
    /// Race tab and the settings tabs (`pause_settings`). `Screen::Pause` is the Race tab; PauseGfx / PauseSfx / PauseControls /
    /// PauseCam / PauseHud are the others (`PAUSE_TAB_SCREENS`).
    pub(super) fn pause_modern(&self, b: &mut Batch, r: &RaceState, px: f32) {
        let d = Glass::new(&b.canvas, px);
        let tab = PAUSE_TAB_SCREENS.iter().position(|s| *s == self.screen).unwrap_or(0);
        let tr = &self.tracks[r.track];
        let race = &r.sess.race;
        let me = &race.racers[r.sess.focus];
        let pulse = 0.5 - 0.5 * (self.t * TAU / 1.1).cos();
        Self::pause_veil(b, d);
        let mut m = Mesh::default();
        Self::glass_title_wash(&mut m, d);
        // no dot array in the corner on the pause screen (playtest); the other Glass screens keep theirs
        b.mesh(&m);
        // the sector the focus car is in: arc length against the profile's sector starts
        let sector = tr.profile.as_ref().map_or(1, |p| 1 + (me.progress.s >= p.sectors[0]) as usize + (me.progress.s >= p.sectors[1]) as usize);
        Self::glass_furniture(b, d, self.t, &format!("RACE PAUSED // {} SECTOR {sector}", tr.name.to_uppercase()), "PAUSED", GlassRules::Pause);
        Self::glass_title_sized(b, d, "pause", &glass_time(race.clock_ms.max(0.0) as u32), Some((62.0, 31.0)));
        Self::glass_plate(b, d, (d.r(1021.0), d.t(28.0), d.d(234.0), d.d(119.0)), "187-P-08", "PAUSESCREEN");
        Self::glass_version(b, d);
        self.pause_tabbar(b, d, tab);
        if tab == 0 {
            self.pause_race_tab(b, d, r, tr, pulse);
            let go = if self.sel == 0 { "SELECT" } else { "HOLD" };
            if !self.asking() {
                Self::glass_hints(b, d, self.controls.last, &[(&[4, 5], "TABS"), (&[9, 10], "MOVE"), (&[2], go), (&[3], "RESUME")], 626.0);
            }
        } else {
            self.pause_settings_tab(b, d, r, pulse);
        }
    }

    /// The cards of the pause screen the race is frosted behind (window px, `SceneFrost`): the plate,
    /// the tab bar and the tab's body cards.
    pub(super) fn pause_frost(&self, size: (u32, u32)) -> motorview::render::SceneFrost {
        Self::scene_frost(size, |d| {
            let mut cards = vec![(d.r(1021.0), d.t(28.0), d.d(234.0), d.d(119.0)), (d.x(62.0), d.y(160.0), d.d(1156.0), d.d(52.0))];
            if self.screen == Screen::Pause {
                cards.extend([(62.0, 228.0, 270.0, 64.0), (62.0, 304.0, 270.0, 64.0), (62.0, 536.0, 270.0, 64.0), (62.0, 388.0, 270.0, 132.0), (352.0, 228.0, 520.0, 372.0), (946.0, 534.0, 200.0, 66.0)].map(|(x, y, w, h)| (d.x(x), d.y(y), d.d(w), d.d(h))));
            } else {
                cards.extend([pause_settings::ROWS_CARD, pause_settings::INFO_CARD].map(|(x, y, w, h)| (d.x(x), d.y(y), d.d(w), d.d(h))));
            }
            cards
        })
    }

    /// The frost of a Glass screen over the race scene: `cards` lists its glass cards (canvas px: x, y, w, h) for the
    /// canvas of a `size` window; the scene is blurred behind them (up to `SCENE_FROST_RECTS`).
    fn scene_frost(size: (u32, u32), cards: impl FnOnce(Glass) -> Vec<(f32, f32, f32, f32)>) -> motorview::render::SceneFrost {
        let canvas = Canvas::hud(size);
        let (s, ox, oy) = canvas.transform(size);
        let d = Glass::new(&canvas, 1.0 / s);
        let cards = cards(d);
        let mut fr = motorview::render::SceneFrost { rects: Default::default(), n: cards.len().min(motorview::render::SCENE_FROST_RECTS), radius: d.d(12.0) * s, blur: 12.0 * size.1 as f32 / 720.0 };
        for (slot, &(x, y, w, h)) in fr.rects.iter_mut().zip(&cards) {
            *slot = ([ox + (x + w * 0.5) * s, oy + (y + h * 0.5) * s], [w * 0.5 * s, h * 0.5 * s]);
        }
        fr
    }

    /// `.veil`: the orange wash from the left over the navy radial that dims the race (0.42 at the
    /// middle to 0.72 at the edges), so glass and text read and the race stays recognisable.
    fn pause_veil(b: &mut Batch, d: Glass) {
        Self::glass_veil(b, d, (0.42, 0.72), 380.0);
    }

    /// `pause_veil` with the navy's alpha at the middle and the edges and the radial's centre height (design px): the
    /// Results screen's `.veil.res` is a little lighter (.36 to .74 at 400).
    fn glass_veil(b: &mut Batch, d: Glass, (a0, a1): (f32, f32), centre_y: f32) {
        let (w, h) = (b.canvas.w, b.canvas.h);
        let (cx, cy, rx, ry) = (d.x(640.0), d.y(centre_y), d.d(900.0), d.d(560.0));
        let mut m = Mesh::default();
        m.shade_rect(0.0, 0.0, w, h, 32, 18, &|q| {
            let k = (((q[0] - cx) / rx).powi(2) + ((q[1] - cy) / ry).powi(2)).sqrt().min(1.0);
            let a = a0 + (a1 - a0) * k;
            let navy = |c0: f32, c1: f32| (c0 * a0 * (1.0 - k) + c1 * a1 * k) / a / 255.0;
            let navy = [navy(4.0, 3.0), navy(10.0, 7.0), navy(22.0, 14.0)];
            let o = 0.14 * (1.0 - q[0] / (0.24 * w)).max(0.0);
            let out = o + a * (1.0 - o);
            let orange = [1.0, 120.0 / 255.0, 40.0 / 255.0];
            [0, 1, 2].map(|i| (orange[i] * o + navy[i] * a * (1.0 - o)) / out).into_iter().chain([out]).collect::<Vec<_>>().try_into().unwrap()
        });
        b.mesh(&m);
    }

    /// A Lucide icon `size` canvas px tall with its top left at (x, top).
    fn glass_icon(b: &mut Batch, ch: char, x: f32, top: f32, size: f32, color: [f32; 4]) {
        // the font's box is 16 field px high: font_text scales by size / cap
        let sz = size * ui_font::face(HudFont::Icons).cap / ui_font::SRC;
        b.font_text(HudFont::Icons, &ch.to_string(), x, top + size - sz, &FontStyle::new(sz, color));
    }

    /// The mockup's `.kcap` of a letter, centred at `c` (a key cap: 26 x 22 design px).
    fn glass_kcap(b: &mut Batch, d: Glass, c: [f32; 2], letter: &str) {
        let (w, h) = (d.d(26.0), d.d(22.0));
        let (x, y) = (c[0] - w * 0.5, c[1] - h * 0.5);
        let mut m = Mesh::default();
        let body = vec::rounded_rect(x, y, w, h, d.d(5.0));
        m.fill(&vec::rounded_rect(x, y + d.d(2.0), w, h, d.d(5.0)), [0.0, 0.0, 0.0, 0.45], d.px);
        m.fill_with(&body, &|q| [1.0, 1.0, 1.0, 0.28 - 0.20 * ((q[1] - y) / h).clamp(0.0, 1.0)], d.px);
        m.stroke(&body, true, d.w(1.0), Ends::Round, [1.0, 1.0, 1.0, 0.4], d.px);
        b.mesh(&m);
        let st = d.font(HudFont::Glass, 12.0, 0.5, WHITE);
        let lw = b.font_width(HudFont::Glass, letter, &st) - st.tracking;
        b.font_text(HudFont::Glass, letter, c[0] - lw * 0.5, d.cap_top(HudFont::Glass, 12.0, y, Some(22.0 * 1.0)), &st);
    }

    /// The tab bar: a glass card 1156 x 52 at (62, 160) with Q / E at its ends (the device's
    /// shoulder buttons on a pad) and six tabs, the selected one orange with its glowing underline.
    fn pause_tabbar(&self, b: &mut Batch, d: Glass, tab: usize) {
        const TABS: [(&str, char); 6] = [("RACE", ui_font::icon::FLAG), ("GRAPHICS", ui_font::icon::MONITOR), ("SOUND", ui_font::icon::VOLUME), ("CONTROLS", ui_font::icon::GAMEPAD), ("CAMERA", ui_font::icon::CAMERA), ("HUD", ui_font::icon::GAUGE)];
        self.glass_tabbar(b, d, &TABS, tab);
    }

    /// `pause_tabbar` for any list of (name, icon) tabs: equal cells over the 1044 design px between the
    /// shoulder keys (the Personal screen's five, #91).
    fn glass_tabbar(&self, b: &mut Batch, d: Glass, tabs: &[(&str, char)], tab: usize) {
        let tabs: Vec<(&str, Option<char>, bool)> = tabs.iter().map(|&(name, icon)| (name, Some(icon), false)).collect();
        self.glass_tabbar_with(b, d, &tabs, tab);
    }

    /// `glass_tabbar` whose tabs may have no icon and may carry the blue "you" dot after the name (`.cdot`, 10 px on:
    /// the League standings' four divisions, #92).
    fn glass_tabbar_with(&self, b: &mut Batch, d: Glass, tabs: &[(&str, Option<char>, bool)], tab: usize) {
        let (bx, by, bw, bh) = (d.x(62.0), d.y(160.0), d.d(1156.0), d.d(52.0));
        let (x0, cell) = (d.x(62.0 + 56.0), d.d(1044.0 / tabs.len() as f32));
        let mut m = Mesh::default();
        glass_card(&mut m, d, (bx, by, bw, bh), d.d(12.0), (0.13, 0.04), 0.36, (10.0, 26.0, 0.45), 0.35, None);
        for i in 1..tabs.len() {
            let x = x0 + cell * i as f32;
            m.stroke(&[[x, by + d.d(16.0)], [x, by + d.d(36.0)]], false, d.w(1.0), Ends::Butt, hex(G_LINE, 0.14), d.px);
        }
        let line = vec::rounded_rect(x0 + cell * tab as f32 + d.d(30.0), by + d.d(51.0), cell - d.d(60.0), d.d(2.0), d.d(1.0));
        // the selected tab's glow at half the mockup's strength (playtest: too much blur)
        m.halo(&line, d.d(22.0), hex(G_ORANGE, 0.25));
        m.halo(&line, d.d(10.0), hex(G_ORANGE, 0.4));
        m.fill(&line, hex(G_ORANGE, 1.0), d.px);
        b.mesh(&m);
        // the shoulder keys: Q / E caps on the keyboard, the pad's LB / RB glyphs otherwise
        for (x, letter, kind) in [(bx + d.d(16.0 + 13.0), "Q", 4), (bx + bw - d.d(16.0 + 13.0), "E", 5)] {
            self.glass_shoulder(b, d, [x, by + d.d(15.0 + 11.0)], letter, kind);
        }
        for (i, (name, icon, dot)) in tabs.iter().enumerate() {
            let col = hex(if i == tab { G_ORANGE } else { G_DIM }, 1.0);
            let st = d.font(HudFont::Glass, 19.0, 1.5, col);
            let tw = b.font_width(HudFont::Glass, name, &st) - st.tracking;
            // the icon 20 px and 10 px of gap, the name, then 10 px of gap and the 6 px dot (after the name's letter-spacing)
            let lead = if icon.is_some() { d.d(20.0 + 10.0) } else { 0.0 };
            let trail = if *dot { st.tracking + d.d(10.0 + 6.0) } else { 0.0 };
            let x = x0 + cell * (i as f32 + 0.5) - (lead + tw + trail) * 0.5;
            if let Some(icon) = icon {
                Self::glass_icon(b, *icon, x, by + d.d(16.0), d.d(20.0), col);
            }
            b.font_text(HudFont::Glass, name, x + lead, d.cap_top(HudFont::Glass, 19.0, by, Some(52.0)), &st);
            if *dot {
                let c = [x + lead + tw + st.tracking + d.d(10.0 + 3.0), by + bh * 0.5];
                let mut m = Mesh::default();
                m.halo(&circle(c, d.d(3.0)), d.d(6.0), hex(0x4aa8ff, 0.9));
                m.fill(&circle(c, d.d(3.0)), hex(G_S1, 1.0), d.px);
                b.mesh(&m);
            }
        }
    }

    /// The Race tab's body: the three actions, total / best, the standings, the map disc with
    /// its legend and the track card.
    fn pause_race_tab(&self, b: &mut Batch, d: Glass, r: &RaceState, tr: &TrackInfo, pulse: f32) {
        let race = &r.sess.race;
        let me = &race.racers[r.sess.focus];
        let (e, device) = (HudFont::GlassMed, self.controls.last);
        let key = match device {
            Device::Keyboard => "Enter",
            Device::Xbox => "A",
            Device::PlayStation => "Cross",
        };
        // ---- the actions; Restart and Exit fill while Enter is held
        let acts = [("RESUME", ui_font::icon::PLAY, "Back to the race".to_string(), 228.0), ("RESTART RACE", ui_font::icon::RESTART, "From the grid · times are lost".to_string(), 304.0), ("EXIT TO MENU", ui_font::icon::LOG_OUT, "The race is not saved".to_string(), 536.0)];
        for (k, (label, icon, sub, top)) in acts.iter().enumerate() {
            let sel = self.sel == k;
            let hold = if sel && k > 0 { self.pause_hold.v.clamp(0.0, 1.0) } else { 0.0 };
            let (x, y, w, h) = (d.x(62.0), d.y(*top), d.d(270.0), d.d(64.0));
            let mut m = Mesh::default();
            glass_card(&mut m, d, (x, y, w, h), d.d(12.0), (0.13, 0.04), 0.36, (10.0, 26.0, 0.45), 0.35, sel.then_some(0.3 + 0.2 * pulse));
            if hold > 0.0 {
                // the card fills left to right (an orange gradient with an edge line), cut to the card
                let edge = x + w * hold;
                let inner = vec::rounded_rect(x + d.px, y + d.px, w - 2.0 * d.px, h - 2.0 * d.px, d.d(12.0) - d.px);
                let part = clip_half(&inner, &|q| q[0] - edge);
                if part.len() >= 3 {
                    m.fill_with(&part, &|q| hex(G_ORANGE, 0.10 + 0.28 * ((q[0] - x) / (edge - x).max(1.0)).clamp(0.0, 1.0)), d.px);
                    m.stroke(&[[edge - d.d(1.0), y + d.d(2.0)], [edge - d.d(1.0), y + h - d.d(2.0)]], false, d.w(2.0), Ends::Butt, hex(G_ORANGE, 1.0), d.px);
                }
            }
            b.mesh(&m);
            Self::glass_icon(b, *icon, x + d.d(16.0), y + d.d(19.0), d.d(26.0), hex(if sel { G_ORANGE } else { G_DIM }, 1.0));
            b.font_text(HudFont::Glass, label, x + d.d(56.0), d.cap_top(HudFont::Glass, 22.0, y + d.d(9.0), None), &d.font(HudFont::Glass, 22.0, 1.0, hex(if sel { G_ORANGE } else { G_TEXT }, 1.0)));
            let sub = if hold > 0.0 { format!("Hold {key} to {} · {}%", if k == 1 { "restart" } else { "leave the race" }, (hold * 100.0).round() as u32) } else { sub.clone() };
            b.font_text(e, &sub, x + d.d(56.0), d.cap_top(e, 12.0, y + d.d(37.0), None), &d.font(e, 12.0, 0.5, hex(G_DIM, 1.0)));
        }
        // ---- total time and best lap
        let (x, y, w, h) = (d.x(62.0), d.y(388.0), d.d(270.0), d.d(132.0));
        let mut m = Mesh::default();
        glass_card(&mut m, d, (x, y, w, h), d.d(12.0), (0.13, 0.04), 0.36, (10.0, 26.0, 0.45), 0.35, None);
        m.dashed([x + d.d(16.0), y + d.d(66.0)], [x + d.d(254.0), y + d.d(66.0)], d.d(6.0), d.d(4.0), d.w(1.0), hex(G_LINE, 0.45), d.px);
        b.mesh(&m);
        let cap = d.font(e, 11.0, 1.5, hex(G_DIM, 1.0));
        let val = d.font(HudFont::Glass, 28.0, 1.0, WHITE);
        let best = me.progress.best_lap_ms.filter(|&ms| ms > 0);
        b.font_text(e, "TOTAL TIME", x + d.d(16.0), d.cap_top(e, 11.0, y + d.d(10.0), None), &cap);
        b.font_text(HudFont::Glass, &glass_time(race.clock_ms.max(0.0) as u32), x + d.d(16.0), d.cap_top(HudFont::Glass, 28.0, y + d.d(23.0), Some(32.0)), &val);
        b.font_text(e, "BEST LAP", x + d.d(16.0), d.cap_top(e, 11.0, y + d.d(74.0), None), &cap);
        b.font_text(HudFont::Glass, &best.map_or_else(|| "--:--.---".to_string(), glass_time), x + d.d(16.0), d.cap_top(HudFont::Glass, 28.0, y + d.d(87.0), Some(32.0)), &val);
        // PB: at or under the saved best of this track (or there is none yet)
        let saved = self.stats.tracks.get(&tr.name).and_then(|t| t.best_lap).map(|(ms, _)| ms);
        if best.is_some_and(|ms| saved.is_none_or(|s| ms <= s)) {
            let st = d.font(HudFont::Glass, 10.0, 1.5, hex(G_ORANGE, 1.0));
            let pw = b.font_width(HudFont::Glass, "PB", &st) - st.tracking;
            b.font_text(HudFont::Glass, "PB", x + w - d.d(14.0) - pw, d.cap_top(HudFont::Glass, 10.0, y + d.d(76.0), None), &st);
        }
        // ---- the standings
        let (x, y, w, h) = (d.x(352.0), d.y(228.0), d.d(520.0), d.d(372.0));
        let mut m = Mesh::default();
        m.fill(&vec::rounded_rect(x, y, w, h, d.d(12.0)), [6.0 / 255.0, 14.0 / 255.0, 28.0 / 255.0, 0.38], d.px);
        glass_card(&mut m, d, (x, y, w, h), d.d(12.0), (0.11, 0.03), 0.36, (10.0, 26.0, 0.45), 0.35, None);
        b.mesh(&m);
        b.font_text(e, "POS", x + d.d(22.0), d.cap_top(e, 11.0, y + d.d(12.0), None), &cap);
        b.font_text(e, "DRIVER", x + d.d(76.0), d.cap_top(e, 11.0, y + d.d(12.0), None), &cap);
        b.font_text(e, "CAR", x + d.d(246.0), d.cap_top(e, 11.0, y + d.d(12.0), None), &cap);
        let gw = b.font_width(e, "GAP", &cap) - cap.tracking;
        b.font_text(e, "GAP", x + w - d.d(22.0) - gw, d.cap_top(e, 11.0, y + d.d(12.0), None), &cap);
        let order = race.standings();
        for (i, &slot) in order.iter().take(8).enumerate() {
            let rc = &race.racers[slot];
            let ry = y + d.d(34.0 + 41.0 * i as f32);
            let (rx, rw, rh) = (x + d.d(10.0), d.d(500.0), d.d(39.0));
            let col = (u32::from(rc.driver.colour[0]) << 16) | (u32::from(rc.driver.colour[1]) << 8) | u32::from(rc.driver.colour[2]);
            let mut m = Mesh::default();
            if i > 0 {
                m.stroke(&[[rx + d.d(8.0), ry - d.d(2.0)], [rx + rw - d.d(8.0), ry - d.d(2.0)]], false, d.w(1.0), Ends::Butt, hex(G_LINE, 0.08), d.px);
            }
            if rc.human {
                let row = vec::rounded_rect(rx, ry, rw, rh, d.d(9.0));
                m.halo(&row, d.d(14.0), hex(0x50aaff, 0.18));
                m.fill(&row, hex(G_S1, 0.10), d.px);
                m.stroke(&row, true, d.w(1.0), Ends::Round, hex(G_S1, 0.5), d.px);
            }
            let bar = vec::rounded_rect(rx + d.d(1.0 + 40.0), ry + d.d(8.5), d.d(4.0), d.d(22.0), d.d(2.0));
            m.halo(&bar, d.d(6.0), hex(col, 0.6));
            m.fill(&bar, hex(col, 1.0), d.px);
            b.mesh(&m);
            let cy = |px: f32| d.cap_top(HudFont::Glass, px, ry, Some(39.0));
            let pos = (i + 1).to_string();
            let st = d.font(HudFont::Glass, 20.0, 0.0, WHITE);
            let pw = b.font_width(HudFont::Glass, &pos, &st) - st.tracking;
            b.font_text(HudFont::Glass, &pos, rx + d.d(1.0 + 20.0) - pw * 0.5, cy(20.0), &st);
            let nick = rc.driver.nick.to_uppercase();
            let nst = d.font(HudFont::Glass, 18.0, 0.5, hex(if rc.human { 0xffffff } else { G_TEXT }, 1.0));
            let nx = rx + d.d(1.0 + 40.0 + 4.0 + 12.0);
            let nw = b.font_width(HudFont::Glass, &nick, &nst) - nst.tracking;
            b.font_text(HudFont::Glass, &nick, nx, cy(18.0), &nst);
            if rc.human {
                let tst = d.font(HudFont::Glass, 10.0, 1.5, hex(G_S1, 1.0));
                let base = cy(18.0) + nst.size;
                b.font_text(HudFont::Glass, "YOU", nx + nw + d.d(8.0), base - tst.size, &tst);
            }
            let car = self.cars.iter().find(|c| c.id == rc.car_id).map_or(String::new(), |c| c.name.to_uppercase());
            b.font_text(e, &car, rx + d.d(1.0 + 40.0 + 4.0 + 12.0 + 170.0), d.cap_top(e, 14.0, ry, Some(39.0)), &d.font(e, 14.0, 1.0, hex(G_DIM, 1.0)));
            // the gap to the leader (#299): LAP n for the leader, +s.mmm, +1 LAP, dashes without data
            let gap = if i == 0 { format!("LAP {}", rc.progress.lap) } else { pause_gap(race.live_gap(order[0], slot)) };
            let gst = d.font(HudFont::Glass, 18.0, 0.5, WHITE);
            let gw = b.font_width(HudFont::Glass, &gap, &gst) - gst.tracking;
            b.font_text(HudFont::Glass, &gap, rx + rw - d.d(1.0 + 10.0) - gw, cy(18.0), &gst);
        }
        // ---- the map disc, its legend and the track card
        let cars: Vec<MapCar> = race.racers.iter().enumerate().map(|(i, rc)| MapCar { pos: rc.car.pos, colour: rc.driver.colour, me: i == r.sess.focus }).collect();
        Self::glass_map(b, d, tr, &MapDisc::pause(), &cars);
        let st = d.font(HudFont::Glass, 11.0, 1.0, hex(G_TEXT, 1.0));
        let names = [("S1", G_S1), ("S2", G_S2), ("S3", G_S3)];
        let total: f32 = names.iter().map(|(n, _)| d.d(8.0 + 6.0 + 6.0) + b.font_width(HudFont::Glass, n, &st) - st.tracking).sum::<f32>() + d.d(4.0 * 2.0) - d.d(6.0);
        let (mut lx, ly) = (d.x(946.0 + 100.0) - total * 0.5, d.cap_top(HudFont::Glass, 11.0, d.y(508.0), None));
        let mut m = Mesh::default();
        for (i, (name, col)) in names.iter().enumerate() {
            if i > 0 {
                lx += d.d(4.0);
            }
            m.fill(&vec::rounded_rect(lx, ly + st.size * 0.5 - d.d(4.0), d.d(8.0), d.d(8.0), d.d(2.0)), hex(*col, 1.0), d.px);
            lx += d.d(8.0 + 6.0);
            lx += b.font_width(HudFont::Glass, name, &st) - st.tracking + d.d(6.0);
        }
        b.mesh(&m);
        let mut lx = d.x(946.0 + 100.0) - total * 0.5 + d.d(8.0 + 6.0);
        for (i, (name, _)) in names.iter().enumerate() {
            if i > 0 {
                lx += d.d(4.0);
            }
            lx += b.font_text(HudFont::Glass, name, lx, ly, &st) - st.tracking + d.d(6.0 + 8.0 + 6.0);
        }
        let (x, y) = (d.x(946.0), d.y(534.0));
        let mut m = Mesh::default();
        glass_card(&mut m, d, (x, y, d.d(200.0), d.d(66.0)), d.d(12.0), (0.13, 0.04), 0.36, (10.0, 26.0, 0.45), 0.35, None);
        b.mesh(&m);
        b.font_text(e, &format!("TRACK {}", tr.n), x + d.d(12.0), d.cap_top(e, 11.0, y + d.d(6.0), None), &cap);
        b.font_text(HudFont::Glass, &tr.name.to_uppercase(), x + d.d(12.0), d.cap_top(HudFont::Glass, 20.0, y + d.d(20.0), Some(24.0)), &d.font(HudFont::Glass, 20.0, 0.5, WHITE));
        let km = if tr.length_m > 0.0 { tr.length_m } else { tr.profile.as_ref().map_or(0.0, |p| p.length) };
        let line = format!("{:.2} KM · LAP {}/{}", km / 1000.0, me.progress.lap.min(race.laps), race.laps);
        b.font_text(e, &line, x + d.d(12.0), d.cap_top(e, 11.0, y + d.d(48.0), None), &d.font(e, 11.0, 1.5, hex(G_TEXT, 1.0)));
    }

    /// The hint row centred over the body (`glass_tips`' look): each item = its glyphs (the
    /// `glyph_for` kinds) and a label, 28 design px apart.
    fn glass_hints(b: &mut Batch, d: Glass, device: Device, items: &[(&[usize], &str)], top: f32) {
        Self::glass_hints_at(b, d, device, items, (640.0, top), 1.0, 28.0);
    }

    /// `glass_hints` centred on design x `centre`, the whole row `scale` times the size with `gap` design px
    /// between the hints (the pad keyboard sheet sets them at .8 and 20).
    fn glass_hints_at(b: &mut Batch, d: Glass, device: Device, items: &[(&[usize], &str)], at: (f32, f32), scale: f32, gap: f32) {
        Self::glass_hints_off(b, d, device, items, at, (scale, gap), &[]);
    }

    /// `glass_hints_at` with the items in `off` dimmed to .35 (the mockup's `.hint.off`: an action that does nothing now).
    fn glass_hints_off(b: &mut Batch, d: Glass, device: Device, items: &[(&[usize], &str)], (centre, top): (f32, f32), (scale, gap): (f32, f32), off: &[usize]) {
        let st = d.font(HudFont::Glass, 23.0 * scale, 1.0 * scale, hex(0xa9d3e6, 1.0));
        let ty = d.cap_top(HudFont::Glass, 23.0 * scale, d.y(top), None);
        let glyph = |kind: usize| {
            let (c, tint) = Self::glyph_for(device, kind);
            (c.to_string(), FontStyle::new(d.d(15.0 * scale), tint))
        };
        let widths: Vec<f32> = items
            .iter()
            .map(|(kinds, label)| {
                let g: f32 = kinds.iter().map(|&k| { let (c, gs) = glyph(k); b.font_width(HudFont::Glyphs, &c, &gs) + d.d(3.0 * scale) }).sum();
                g - d.d(3.0 * scale) + d.d(7.0 * scale) + b.font_width(HudFont::Glass, label, &st) - st.tracking
            })
            .collect();
        let total = widths.iter().sum::<f32>() + d.d(gap) * (items.len() - 1) as f32;
        let mut x = d.x(centre) - total * 0.5;
        for (i, ((kinds, label), w)) in items.iter().zip(widths).enumerate() {
            let x0 = x;
            let a = if off.contains(&i) { 0.35 } else { 1.0 };
            for (n, &k) in kinds.iter().enumerate() {
                let (c, mut gs) = glyph(k);
                gs.color[3] *= a;
                if n > 0 {
                    x += d.d(3.0 * scale);
                }
                x += b.font_text(HudFont::Glyphs, &c, x, ty + st.size * 0.5 - gs.size * 0.5, &gs);
            }
            let mut ls = st;
            ls.color[3] *= a;
            b.font_text(HudFont::Glass, label, x + d.d(7.0 * scale), ty, &ls);
            x = x0 + w + d.d(gap);
        }
    }

    /// The pad family the Controller page draws: the pad used last, an Xbox pad by default.
    fn pad_device(&self) -> Device {
        if self.controls.last == Device::PlayStation {
            Device::PlayStation
        } else {
            Device::Xbox
        }
    }

    /// The needle of a dial of radius `r` at `angle`, in the main dial's shape.
    fn needle(b: &mut Batch, c: [f32; 2], r: f32, angle: f32, px: f32) {
        // REMASTER the needle (tuned in tools/needle-tuner.html): a straight kite in two facets,
        // translucent toward the tail, with an axis line, a gloss streak and a soft glow
        let (ux, uy) = (angle.cos(), angle.sin());
        let at = |s: f32, t: f32| [c[0] + ux * s * r - uy * t, c[1] + uy * s * r + ux * t];
        let hw = NEEDLE_HALF_WIDTH * (r / DIAL_R);
        let (tip, tail, mid) = (
            at(NEEDLE_TIP, 0.0),
            at(-NEEDLE_TAIL, 0.0),
            at(NEEDLE_MID, 0.0),
        );
        let (mid_l, mid_r) = (at(NEEDLE_MID, -hw), at(NEEDLE_MID, hw));
        // opacity runs linearly from the tail to the tip; s is the position along the axis
        let along = |p: [f32; 2]| {
            ((((p[0] - c[0]) * ux + (p[1] - c[1]) * uy) / r + NEEDLE_TAIL)
                / (NEEDLE_TIP + NEEDLE_TAIL))
                .clamp(0.0, 1.0)
        };
        let body = |col: [f32; 3]| {
            move |p: [f32; 2]| {
                let u = along(p);
                let k = NEEDLE_TIP_BOOST * u;
                let lift = |v: f32| {
                    if k >= 0.0 {
                        v + (1.0 - v) * k
                    } else {
                        v * (1.0 + k)
                    }
                };
                [
                    lift(col[0]),
                    lift(col[1]),
                    lift(col[2]),
                    NEEDLE_TAIL_ALPHA + (1.0 - NEEDLE_TAIL_ALPHA) * u,
                ]
            }
        };
        let mut n = Mesh::default();
        // the glow follows the axis (not the outline) so it stays even right up to the tip
        if NEEDLE_GLOW_ON {
            n.glow_smooth(
                &[tip, tail],
                hw * NEEDLE_GLOW_CORE,
                NEEDLE_GLOW_RADIUS,
                [
                    NEEDLE_GLOW[0],
                    NEEDLE_GLOW[1],
                    NEEDLE_GLOW[2],
                    NEEDLE_GLOW[3] * NEEDLE_GLOW_GAIN,
                ],
                20,
            );
        }
        n.fill_with(&[tip, mid_r, tail, mid], &body(NEEDLE_LIT), px);
        n.fill_with(&[tip, mid, tail, mid_l], &body(NEEDLE_SHADE), px);
        n.stroke(
            &[tip, mid, tail],
            false,
            NEEDLE_EDGE_WIDTH,
            Ends::Butt,
            NEEDLE_EDGE,
            px,
        );
        // the gloss: a streak on the lit facet that fades in from nothing at its tail end
        let (s1, len) = (NEEDLE_TIP - 0.08, NEEDLE_GLOSS_LENGTH * NEEDLE_TIP);
        let s0 = (s1 - len).max(NEEDLE_MID);
        // lit facet half width at s, tapering to the tip
        let w = |s: f32| hw * ((NEEDLE_TIP - s) / (NEEDLE_TIP - NEEDLE_MID)).clamp(0.0, 1.0);
        let gw = NEEDLE_GLOSS_WIDTH;
        let streak = [
            at(s0, w(s0) * (0.5 - gw * 0.5)),
            at(s1, w(s1) * (0.5 - gw * 0.5)),
            at(s1, w(s1) * (0.5 + gw * 0.5)),
            at(s0, w(s0) * (0.5 + gw * 0.5)),
        ];
        // the fade reaches full strength at 1 - 0.5 * (1 - sharp) of the streak: soft = long ramp
        let ramp = (1.0 - 0.5 * (1.0 - NEEDLE_GLOSS_SHARP)).max(0.05);
        n.fill_with(
            &streak,
            &|p| {
                let s = ((p[0] - c[0]) * ux + (p[1] - c[1]) * uy) / r;
                let u = ((s - s0) / ((s1 - s0) * ramp)).clamp(0.0, 1.0);
                [
                    NEEDLE_GLOSS[0],
                    NEEDLE_GLOSS[1],
                    NEEDLE_GLOSS[2],
                    NEEDLE_GLOSS[3] * u,
                ]
            },
            px,
        );
        b.mesh(&n);
    }
}

/// tints of the face buttons: Xbox A B X Y, PlayStation Cross Circle Square Triangle (as the tips)
const FACE_XBOX: [[f32; 4]; 4] = [[0.5, 0.85, 0.35, 1.0], [0.95, 0.4, 0.33, 1.0], [0.35, 0.6, 1.0, 1.0], [0.95, 0.8, 0.25, 1.0]];
const FACE_PS: [[f32; 4]; 4] = [[0.55, 0.72, 1.0, 1.0], [1.0, 0.45, 0.45, 1.0], [0.9, 0.5, 0.8, 1.0], [0.4, 0.85, 0.6, 1.0]];
const GLYPH_TINT: [f32; 4] = [0.7, 0.92, 1.0, 1.0];

/// REMASTER the PromptFont glyph and tint of a pad control (codepoints: PromptFont glyphs.json).
pub(crate) fn pad_glyph(p: PadIn, dev: Device) -> (char, [f32; 4]) {
    use gilrs::Button::*;
    let ps = dev == Device::PlayStation;
    let face = |i: usize| if ps { (['\u{21E3}', '\u{21E2}', '\u{21E0}', '\u{21E1}'][i], FACE_PS[i]) } else { (['\u{21D3}', '\u{21D2}', '\u{21D0}', '\u{21D1}'][i], FACE_XBOX[i]) };
    let c = match p {
        PadIn::Stick(Dir::Left) => '\u{21BC}',
        PadIn::Stick(Dir::Right) => '\u{21C0}',
        PadIn::Stick(Dir::Up) => '\u{21BE}',
        PadIn::Stick(Dir::Down) => '\u{21C2}',
        PadIn::Stick(_) => '\u{21F1}',
        PadIn::Button(South) => return face(0),
        PadIn::Button(East) => return face(1),
        PadIn::Button(West) => return face(2),
        PadIn::Button(North) => return face(3),
        PadIn::Button(b) => match (b, ps) {
            (LeftTrigger, false) => '\u{2198}',
            (RightTrigger, false) => '\u{2199}',
            (LeftTrigger2, false) => '\u{2196}',
            (RightTrigger2, false) => '\u{2197}',
            (LeftTrigger, true) => '\u{21B0}',
            (RightTrigger, true) => '\u{21B1}',
            (LeftTrigger2, true) => '\u{21B2}',
            (RightTrigger2, true) => '\u{21B3}',
            (LeftThumb, false) => '\u{21BA}',
            (RightThumb, false) => '\u{21BB}',
            (LeftThumb, true) => '\u{21EF}',
            (RightThumb, true) => '\u{21F0}',
            (Select, false) => '\u{21FA}',
            (Start, false) => '\u{21FB}',
            (Select, true) => '\u{21E6}',
            (Start, true) => '\u{21E8}',
            (DPadLeft, false) => '\u{227A}',
            (DPadUp, false) => '\u{227B}',
            (DPadRight, false) => '\u{227C}',
            (DPadDown, false) => '\u{227D}',
            (DPadLeft, true) => '\u{219E}',
            (DPadUp, true) => '\u{219F}',
            (DPadRight, true) => '\u{21A0}',
            (DPadDown, true) => '\u{21A1}',
            _ => '\u{21F9}',
        },
    };
    (c, GLYPH_TINT)
}

/// REMASTER the glyph of a layout's control code: keys, mouse, and the .clo pad codes on `dev`.
pub(crate) fn code_glyph(c: Code, dev: Device) -> Option<(char, [f32; 4])> {
    let g = |c: char| Some((c, GLYPH_TINT));
    match c {
        Code::Key(k) => key_glyph(k).and_then(g),
        Code::Either(KeyCode::ShiftLeft, _) => g('\u{2429}'),
        Code::Either(KeyCode::ControlLeft, _) => g('\u{2427}'),
        Code::Either(..) => g('\u{2428}'),
        Code::MouseButton(i) => ['\u{27F5}', '\u{27F6}', '\u{27F7}'].get(i as usize).copied().and_then(g),
        Code::Mouse(Dir::Left) => g('\u{27F8}'),
        Code::Mouse(Dir::Up) => g('\u{27F9}'),
        Code::Mouse(Dir::Right) => g('\u{27FD}'),
        Code::Mouse(_) => g('\u{27FE}'),
        Code::Joy(d) => Some(pad_glyph(PadIn::Stick(d), dev)),
        Code::JoyButton(n) => Some(pad_glyph(PadIn::Button(crate::controls::PAD_BUTTONS[(n as usize).clamp(1, 8) - 1]), dev)),
    }
}

/// REMASTER the PromptFont keycap of a key; None for keys it has no glyph for.
pub(crate) fn key_glyph(k: KeyCode) -> Option<char> {
    use KeyCode::*;
    const LETTERS: [KeyCode; 26] = [KeyA, KeyB, KeyC, KeyD, KeyE, KeyF, KeyG, KeyH, KeyI, KeyJ, KeyK, KeyL, KeyM, KeyN, KeyO, KeyP, KeyQ, KeyR, KeyS, KeyT, KeyU, KeyV, KeyW, KeyX, KeyY, KeyZ];
    const DIGITS: [KeyCode; 10] = [Digit0, Digit1, Digit2, Digit3, Digit4, Digit5, Digit6, Digit7, Digit8, Digit9];
    const FKEYS: [KeyCode; 12] = [F1, F2, F3, F4, F5, F6, F7, F8, F9, F10, F11, F12];
    const NUMPAD: [KeyCode; 9] = [Numpad1, Numpad2, Numpad3, Numpad4, Numpad5, Numpad6, Numpad7, Numpad8, Numpad9];
    let at = |base: u32, i: usize| char::from_u32(base + i as u32);
    if let Some(i) = LETTERS.iter().position(|&x| x == k) {
        return at(0xFF21, i);
    }
    if let Some(i) = DIGITS.iter().position(|&x| x == k) {
        return at(0xFF10, i);
    }
    if let Some(i) = FKEYS.iter().position(|&x| x == k) {
        return at(0x2460, i);
    }
    if let Some(i) = NUMPAD.iter().position(|&x| x == k) {
        return at(0x2474, i);
    }
    Some(match k {
        Quote => '\u{FF07}',
        Comma => '\u{FF0C}',
        Minus => '\u{FF0D}',
        Period => '\u{FF0E}',
        Slash => '\u{FF0F}',
        Semicolon => '\u{FF1B}',
        Equal => '\u{FF1D}',
        BracketLeft => '\u{FF3B}',
        Backslash => '\u{FF3C}',
        BracketRight => '\u{FF3D}',
        Backquote => '\u{FF40}',
        ArrowLeft => '\u{23F4}',
        ArrowRight => '\u{23F5}',
        ArrowUp => '\u{23F6}',
        ArrowDown => '\u{23F7}',
        ControlLeft => '\u{244D}',
        ControlRight => '\u{244E}',
        AltLeft => '\u{244B}',
        AltRight => '\u{244C}',
        ShiftLeft => '\u{244F}',
        ShiftRight => '\u{2450}',
        SuperLeft | SuperRight => '\u{242A}',
        Tab => '\u{242B}',
        CapsLock => '\u{242C}',
        Backspace => '\u{242D}',
        Enter => '\u{242E}',
        Escape => '\u{242F}',
        PrintScreen => '\u{2430}',
        ScrollLock => '\u{2431}',
        Pause => '\u{2432}',
        NumLock => '\u{2433}',
        Insert => '\u{2434}',
        Home => '\u{2435}',
        PageUp => '\u{2436}',
        Delete => '\u{2437}',
        End => '\u{2438}',
        PageDown => '\u{2439}',
        Space => '\u{243A}',
        Numpad0 => '\u{247D}',
        NumpadDecimal => '\u{247E}',
        NumpadEnter => '\u{247F}',
        NumpadSubtract => '\u{2480}',
        NumpadAdd => '\u{2481}',
        NumpadDivide => '\u{2482}',
        NumpadMultiply => '\u{2483}',
        NumpadEqual => '\u{2484}',
        _ => return None,
    })
}

// REMASTER Track screen: the Glass HUD (#294, 14-menu-overhaul 14.7; the style guide for every later
// modern screen is re-notes/19-glass-hud-design-language.md) [design choice; the user's
// mockup `reference_materials/menu-overhaul/Glass HUD/ConceptE.dc.html`, a 1280x720 artboard whose
// inline styles are the spec]. Every length in this section is the mockup's design px; `Glass`
// puts them on the 480-tall canvas (x 2/3 at 16:9, scaled down about the centre on narrower
// canvases, edge pieces keeping their distance from the edge). Deviations the user chose: the
// SCREEN card is lifted clear of the map ring (`GLASS_PLATE_TOP`), the ELEVATION card and the legend are dropped (`GLASS_ELEV_DROP`), the mockup's background art is
// replaced by our hologram car (its overlay layers stay), the title gradient's highlight stops are
// halved, the hints keep the device glyphs, the selected item is marked in orange.

/// the design's size
const GLASS_W: f32 = 1280.0;
const GLASS_H: f32 = 720.0;
/// the SCREEN card's top: 28 in the mockup, lifted so its bottom clears the map's blue arc
const GLASS_PLATE_TOP: f32 = 14.0;
/// the ELEVATION card and the sector legend sit this much lower than in the mockup (design px), clear of the map ring
const GLASS_ELEV_DROP: f32 = 16.0;
/// REMASTER Glass dial (#295, design px): the centre, the bezel's outer radius (the old dial's rim),
/// the bezel's width, the dark gap between it and the face, the tips row's line-box top
const GLASS_DIAL_C: (f32, f32) = (656.5, 394.5);
const GLASS_DIAL_R: f32 = 231.0;
const GLASS_DIAL_BEZEL: f32 = 10.0;
const GLASS_DIAL_GAP: f32 = 6.0;
const GLASS_DIAL_TIPS: f32 = 572.0;
/// the clear distance of every item label from the bezel's edge
const GLASS_DIAL_LABEL_GAP: f32 = 18.0;
/// the mockup's colours
pub(crate) const G_TEXT: u32 = 0xbfe3f3;
pub(crate) const G_DIM: u32 = 0x8fb5c9;
pub(crate) const G_ORANGE: u32 = 0xff9a3c;
/// rgba(190,230,255, a): rims, rulers, ticks
pub(crate) const G_LINE: u32 = 0xbee6ff;
pub(crate) const G_S1: u32 = 0x8fd0ff;
pub(crate) const G_S2: u32 = 0xff9a3c;
pub(crate) const G_S3: u32 = 0xe6f6ff;
const G_PLATE: u32 = 0xaab4c2;
pub(crate) const G_DARK: u32 = 0x04070d;
/// the "track" title's gradient: the mockup's stops with the three highlights at half brightness
const G_TITLE: [(f32, u32); 5] = [(0.0, 0x806d58), (0.30, 0x805027), (0.60, 0xc4601a), (0.80, 0x804d25), (1.0, 0x8a3a0c)];
/// the metal bezels' conic gradient (CSS `from 210deg`)
const G_BEZEL: [(f32, u32); 7] = [(0.0, 0x9db6c8), (0.18, 0x1c2836), (0.33, 0xd8ecf8), (0.48, 0x26384b), (0.66, 0x86a0b4), (0.84, 0x1c2836), (1.0, 0x9db6c8)];

pub(crate) fn hex(h: u32, a: f32) -> [f32; 4] {
    [((h >> 16) & 255) as f32 / 255.0, ((h >> 8) & 255) as f32 / 255.0, (h & 255) as f32 / 255.0, a]
}

/// The colour at `t` (0..1) of a stop list.
fn stops_at(stops: &[(f32, u32)], t: f32) -> [f32; 4] {
    let (mut lo, mut hi) = (stops[0], stops[stops.len() - 1]);
    for w in stops.windows(2) {
        if t >= w[0].0 && t <= w[1].0 {
            (lo, hi) = (w[0], w[1]);
        }
    }
    let u = if hi.0 > lo.0 { ((t - lo.0) / (hi.0 - lo.0)).clamp(0.0, 1.0) } else { 0.0 };
    let (a, b) = (hex(lo.1, 1.0), hex(hi.1, 1.0));
    [a[0] + (b[0] - a[0]) * u, a[1] + (b[1] - a[1]) * u, a[2] + (b[2] - a[2]) * u, 1.0]
}

/// The lap's points `line` (the profile's outline mapped to canvas) split into the three sectors, each run
/// continuing to the next sector's first point so the strokes join (the Track / pause maps and the race HUD's).
pub(crate) fn sector_runs(p: &motor_sim::race::TrackProfile, line: &[[f32; 2]]) -> [Vec<[f32; 2]>; 3] {
    let sector = |s: f32| if s < p.sectors[0] { 0 } else if s < p.sectors[1] { 1 } else { 2 };
    let n = line.len();
    let mut out: [Vec<[f32; 2]>; 3] = Default::default();
    for (sec, run) in out.iter_mut().enumerate() {
        *run = line.iter().zip(&p.outline_s).filter(|(_, &s)| sector(s) == sec).map(|(q, _)| *q).collect();
        if run.is_empty() {
            continue;
        }
        let last = (0..n).rev().find(|&i| sector(p.outline_s[i]) == sec).unwrap_or(0);
        run.push(line[(last + 1) % n]);
    }
    out
}

/// The white tick across the start / finish line of the lap `line`, `half` long on each side.
pub(crate) fn start_tick(m: &mut Mesh, line: &[[f32; 2]], half: f32, width: f32, px: f32) {
    let (a, nx) = (line[0], line[1 % line.len()]);
    let t = [nx[0] - a[0], nx[1] - a[1]];
    let tl = (t[0] * t[0] + t[1] * t[1]).sqrt().max(1e-3);
    let nrm = [-t[1] / tl, t[0] / tl];
    m.stroke(&[[a[0] - nrm[0] * half, a[1] - nrm[1] * half], [a[0] + nrm[0] * half, a[1] + nrm[1] * half]], false, width, Ends::Butt, WHITE, px);
}

/// 96 points round a circle (no repeated end: fills, halos and closed strokes take it as is).
pub(crate) fn circle(c: [f32; 2], r: f32) -> Vec<[f32; 2]> {
    let mut v = vec::arc(c, r, 0.0, TAU, 96);
    v.pop();
    v
}

/// A time as the Glass screens show it: mm:ss.mmm.
pub(crate) fn glass_time(ms: u32) -> String {
    format!("{:02}:{:02}.{:03}", ms / 60000, (ms / 1000) % 60, ms % 1000)
}

/// The standings' gap to the leader (#299): `+s.mmm`, `+m:ss.mmm` from a minute, `+1 LAP`, dashes
/// when there is no gap (the car is not on the leader's lap data yet).
fn pause_gap(gap: Option<motor_sim::race::Gap>) -> String {
    match gap {
        Some(motor_sim::race::Gap::Time(ms)) if ms >= 60_000 => format!("+{}:{:02}.{:03}", ms / 60_000, (ms / 1000) % 60, ms % 1000),
        Some(motor_sim::race::Gap::Time(ms)) => format!("+{}.{:03}", ms / 1000, ms % 1000),
        Some(motor_sim::race::Gap::Laps(1)) => "+1 LAP".to_string(),
        Some(motor_sim::race::Gap::Laps(n)) => format!("+{n} LAPS"),
        None => "--.---".to_string(),
    }
}

/// A rounded rectangle with a radius per corner (tl, tr, br, bl), clockwise.
fn rounded_rect4(x: f32, y: f32, w: f32, h: f32, r: [f32; 4]) -> Vec<[f32; 2]> {
    use std::f32::consts::{FRAC_PI_2, PI};
    let mut v = Vec::new();
    for (i, (cx, cy, a0)) in [(x + w - r[1], y + r[1], -FRAC_PI_2), (x + w - r[2], y + h - r[2], 0.0), (x + r[3], y + h - r[3], FRAC_PI_2), (x + r[0], y + r[0], PI)].into_iter().enumerate() {
        let rr = r[[1, 2, 3, 0][i]];
        v.extend(vec::arc([cx, cy], rr, a0, a0 + FRAC_PI_2, 6));
    }
    v
}

/// Where and how the Glass map disc is drawn (design px): centre, bezel radius, the blue arc
/// (radius, from, to in radians) and whether the turns are numbered.
struct MapDisc {
    c: (f32, f32),
    bezel: f32,
    arc: Option<(f32, f32, f32)>,
    /// the tick ring round the bezel
    ticks: bool,
    turns: bool,
    /// the lap in the three sector colours with its start / finish tick, else one thin line
    sectors: bool,
    /// the car dots' size against the pause disc's
    dot: f32,
}

impl MapDisc {
    /// the Track screen's
    fn track() -> MapDisc {
        MapDisc { c: (1013.0, 288.0), bezel: 126.0, arc: Some((143.0, Game::clock(320.0), Game::clock(435.0))), ticks: true, turns: true, sectors: true, dot: 1.0 }
    }

    /// the pause screen's (C-paddock: bezel 236, arc r 138 from -2.3 to -0.7 rad)
    fn pause() -> MapDisc {
        MapDisc { c: (1046.0, 370.0), bezel: 118.0, arc: Some((138.0, -2.3, -0.7)), ticks: true, turns: false, sectors: true, dot: 1.0 }
    }

    /// the Personal screen's small disc (C-paddock `disc-s`, #91): no tick ring or arc, a plain lap
    fn small(c: (f32, f32), bezel: f32) -> MapDisc {
        MapDisc { c, bezel, arc: None, ticks: false, turns: false, sectors: false, dot: 0.7 }
    }
}

/// One REMASTER stat dial (`glass_stat_dial`): centre and diameter in design px, the arc's fill (0..1, past 1 = full),
/// the previous car's fill for its tick, and the texts.
struct StatDial {
    c: (f32, f32),
    size: f32,
    fill: f32,
    prev: Option<f32>,
    value: String,
    unit: &'static str,
    caption: &'static str,
}

/// A car dot on the map disc: game position, driver colour, the focus car.
struct MapCar {
    pos: Vec3,
    colour: [u8; 3],
    me: bool,
}

/// Which dashed rules the HUD furniture draws at y 287.
#[derive(Clone, Copy)]
enum GlassRules {
    None,
    /// the Track screen's three, behind the photo discs
    Track,
    /// the pause screen's short one at the left edge
    Pause,
    /// the Car screen's two short ones at both edges
    Car,
}

/// The mockup's coordinates on the canvas: centred items follow the canvas centre (`x`, `y`),
/// edge items their edge (`l`, `r`, `t`, `b`); `s` = canvas px per design px.
#[derive(Clone, Copy)]
pub(crate) struct Glass {
    pub(crate) s: f32,
    pub(crate) cw: f32,
    pub(crate) px: f32,
}

impl Glass {
    pub(crate) fn new(canvas: &Canvas, px: f32) -> Glass {
        let k = (canvas.w / (GLASS_W * 480.0 / GLASS_H)).min(1.0);
        Glass { s: 480.0 / GLASS_H * k, cw: canvas.w, px }
    }
    /// REMASTER Race line HUD (#323): the mockup's design px on the HUD canvas (480 tall, so 2/3 canvas px
    /// per design px); the HUD's own Scale setting does the sizing after that.
    pub(crate) fn hud(canvas_w: f32, px: f32) -> Glass {
        Glass { s: 480.0 / GLASS_H, cw: canvas_w, px }
    }
    pub(crate) fn x(self, v: f32) -> f32 {
        self.cw * 0.5 + (v - GLASS_W * 0.5) * self.s
    }
    pub(crate) fn y(self, v: f32) -> f32 {
        240.0 + (v - GLASS_H * 0.5) * self.s
    }
    pub(crate) fn l(self, v: f32) -> f32 {
        v * self.s
    }
    pub(crate) fn r(self, v: f32) -> f32 {
        self.cw - (GLASS_W - v) * self.s
    }
    pub(crate) fn t(self, v: f32) -> f32 {
        v * self.s
    }
    pub(crate) fn b(self, v: f32) -> f32 {
        480.0 - (GLASS_H - v) * self.s
    }
    pub(crate) fn d(self, v: f32) -> f32 {
        v * self.s
    }
    /// a line's width: the design's, never under a window pixel
    pub(crate) fn w(self, v: f32) -> f32 {
        (v * self.s).max(self.px)
    }
    /// a CSS font (size px, letter-spacing px) on the canvas: cap height and tracking
    pub(crate) fn font(self, f: HudFont, px: f32, ls: f32, color: [f32; 4]) -> FontStyle {
        let face = ui_font::face(f);
        FontStyle::new(px * self.s * face.cap / ui_font::SRC, color).tracking(ls / px * ui_font::SRC)
    }
    /// the cap top of text whose CSS line box starts at canvas `top`: `lh` = the line height
    /// (design px), None = the font's normal one
    pub(crate) fn cap_top(self, f: HudFont, px: f32, top: f32, lh: Option<f32>) -> f32 {
        let face = ui_font::face(f);
        let em = px * self.s / ui_font::SRC;
        let content = (face.ascender - face.descender) * em;
        top + lh.map_or(0.0, |lh| (lh * self.s - content) * 0.5) + (face.ascender - face.cap) * em
    }
    /// the cap top of an SVG text whose baseline is at canvas `base`
    pub(crate) fn cap_above(self, f: HudFont, px: f32, base: f32) -> f32 {
        base - px * self.s * ui_font::face(f).cap / ui_font::SRC
    }
}

/// A lap's fit into a circle (`Game::fit_lap`): the HUD map's projection centred on `c`, scaled by `k`.
pub(super) struct LapFit {
    map_info: [f32; 4],
    mid: (f32, f32),
    k: f32,
    c: [f32; 2],
}

impl LapFit {
    /// A game-space point on the fitted map (canvas px).
    pub(super) fn at(&self, q: [f32; 3]) -> [f32; 2] {
        let (x, y) = minimap::map_point(self.map_info, Vec3::from_array(q));
        [self.c[0] + (x - self.mid.0) * self.k, self.c[1] + (y - self.mid.1) * self.k]
    }

    /// The whole lap's points.
    pub(super) fn line(&self, p: &motor_sim::race::TrackProfile) -> Vec<[f32; 2]> {
        p.outline.iter().map(|&q| self.at(q)).collect()
    }
}

/// A glass card of the mockup: `fill` = the white alpha at the top and bottom, `border` the
/// rim's alpha, `shadow` = (dy, blur, alpha), `inset` the top highlight's alpha; `accent` =
/// the selection's orange glow alpha (the rim turns orange).
#[allow(clippy::too_many_arguments)]
fn glass_card(m: &mut Mesh, d: Glass, (x, y, w, h): (f32, f32, f32, f32), r: f32, fill: (f32, f32), border: f32, shadow: (f32, f32, f32), inset: f32, accent: Option<f32>) {
    let body = vec::rounded_rect(x, y, w, h, r);
    m.shadow(&body, 0.0, d.d(shadow.0), d.d(shadow.1), [0.0, 0.0, 0.0, shadow.2]);
    if let Some(a) = accent {
        m.halo(&body, d.d(14.0), hex(G_ORANGE, a));
    }
    m.fill_with(&body, &|q| [1.0, 1.0, 1.0, fill.0 + (fill.1 - fill.0) * ((q[1] - y) / h).clamp(0.0, 1.0)], d.px);
    let rim = if accent.is_some() { hex(G_ORANGE, 0.95) } else { hex(G_LINE, border) };
    m.stroke(&body, true, d.w(1.0), Ends::Round, rim, d.px);
    m.stroke(&[[x + r, y + d.d(1.5)], [x + w - r, y + d.d(1.5)]], false, d.w(1.0), Ends::Butt, [1.0, 1.0, 1.0, inset], d.px);
}

impl Game {
    /// REMASTER Track screen in the Glass HUD. Selection items keep the Classic order: 0 the
    /// selector, 1 laps, 2 catch-up.
    pub(super) fn track_modern(&self, size: (u32, u32)) -> Vec<Layer> {
        let canvas = Canvas::hud(size);
        let px = 1.0 / canvas.transform(size).0;
        let d = Glass::new(&canvas, px);
        // the glass cards (canvas px) in the mockup's places: the hologram is blurred behind them
        let plate = (d.r(1021.0), d.t(GLASS_PLATE_TOP), d.d(234.0), d.d(119.0));
        let cards = [(d.x(115.0), d.y(231.0), d.d(204.0), d.d(45.0)), (d.x(115.0), d.y(321.0), d.d(204.0), d.d(45.0)), (d.x(115.0), d.y(396.0), d.d(204.0), d.d(58.0)), (d.x(913.0), d.y(432.0 + GLASS_ELEV_DROP), d.d(200.0), d.d(72.0))];
        let settings = (d.x(327.0), d.y(615.0), d.d(660.0), d.d(82.0));
        let blur: Vec<(f32, f32, f32, f32, f32)> = cards.iter().chain([&plate, &settings]).map(|&(x, y, w, h)| (x, y, w, h, d.d(12.0))).collect();
        let back = self.ui_in(Batch::sharp_text(&self.assets, canvas, 1.0), |_, b| Self::glass_backdrop(b, d));
        let front = self.ui_in(Batch::sharp_text(&self.assets, canvas, 1.0), |g, b| {
            let tr = &g.tracks[g.settings.track];
            let pulse = 0.5 - 0.5 * (g.t * TAU / 1.1).cos();
            Self::glass_overlays(b, d);
            Self::glass_furniture(b, d, g.t, "TRACK SELECT // SECTOR GRID 4F", "LIVE", GlassRules::Track);
            Self::glass_title(b, d, "track", "88-137:60");
            Self::glass_photo(b, d, g.assets.preview_name(tr.n).map_or_else(|| format!("trk:{}", tr.n), str::to_string));
            Self::glass_map(b, d, tr, &MapDisc::track(), &[]);
            let locked = g.track_locked();
            if locked {
                // browsed but not unlocked: the disc shaded under a blinking tag, SELECT dimmed
                g.glass_blocked(b, d, px, 170.0, 0.6, "NOT YET AVAILABLE");
            }
            // the cards on the left: laps, catch-up (selection items 1 and 2), best lap
            let mut m = Mesh::default();
            let accent = |i: usize| (g.sel == i).then_some(0.3 + 0.2 * pulse);
            for (i, card) in cards.iter().take(2).enumerate() {
                glass_card(&mut m, d, *card, d.d(12.0), (0.14, 0.04), 0.38, (10.0, 26.0, 0.45), 0.38, accent(i + 1));
            }
            glass_card(&mut m, d, cards[2], d.d(12.0), (0.13, 0.04), 0.36, (10.0, 26.0, 0.45), 0.35, None);
            glass_card(&mut m, d, cards[3], d.d(12.0), (0.13, 0.04), 0.36, (10.0, 26.0, 0.45), 0.35, None);
            // the catch-up check box: lit cyan when on, a glass frame when off
            let (cx, cy, cw, _) = cards[1];
            let bx = vec::rounded_rect(cx + cw - d.d(12.0) - d.d(22.0), cy + d.d(10.0), d.d(22.0), d.d(22.0), d.d(6.0));
            let (bx0, by0) = (cx + cw - d.d(12.0) - d.d(22.0), cy + d.d(10.0));
            if g.settings.catchup {
                m.halo(&bx, d.d(14.0), [80.0 / 255.0, 200.0 / 255.0, 1.0, 0.75]);
                m.fill_with(&bx, &|q| stops_at(&[(0.0, 0x6fdcff), (1.0, 0x2aa6d4)], ((q[1] - by0) / d.d(22.0)).clamp(0.0, 1.0)), px);
                m.stroke(&[[bx0 + d.d(6.0), by0 + d.d(1.0)], [bx0 + d.d(16.0), by0 + d.d(1.0)]], false, d.w(1.0), Ends::Butt, [1.0, 1.0, 1.0, 0.6], px);
                let tick: Vec<[f32; 2]> = [[5.0, 11.5], [9.0, 15.5], [17.0, 6.5]].iter().map(|p| [bx0 + d.d(p[0]), by0 + d.d(p[1])]).collect();
                m.stroke(&tick, false, d.w(2.4), Ends::Round, hex(0x04243a, 1.0), px);
            } else {
                m.fill(&bx, [1.0, 1.0, 1.0, 0.08], px);
                m.stroke(&bx, true, d.w(1.0), Ends::Round, hex(G_LINE, 0.5), px);
            }
            b.mesh(&m);
            let lit = |i: usize| if g.sel == i { hex(G_ORANGE, 1.0) } else { hex(G_TEXT, 1.0) };
            for (i, (label, card)) in [("LAPS", cards[0]), ("CATCH-UP", cards[1])].into_iter().enumerate() {
                let st = d.font(HudFont::Glass, 25.0, 0.5, lit(i + 1));
                b.font_text(HudFont::Glass, label, card.0 + d.d(16.0), d.cap_top(HudFont::Glass, 25.0, card.1, Some(43.0)), &st);
            }
            let st = d.font(HudFont::Glass, 25.0, 0.0, WHITE);
            let laps = g.shown_laps().to_string();
            let lw = b.font_width(HudFont::Glass, &laps, &st) - st.tracking;
            b.font_text(HudFont::Glass, &laps, cards[0].0 + cards[0].2 - d.d(16.0) - lw, d.cap_top(HudFont::Glass, 25.0, cards[0].1, Some(43.0)), &st);
            // best lap: the statistics' best on this track (any car), "--:--.---" without one
            let (x, y, w, _) = cards[2];
            b.font_text(HudFont::GlassMed, "BEST LAP", x + d.d(14.0), d.cap_top(HudFont::GlassMed, 11.0, y + d.d(6.0), None), &d.font(HudFont::GlassMed, 11.0, 1.5, hex(G_DIM, 1.0)));
            let best = g.stats.tracks.get(&tr.name).and_then(|t| t.best_lap);
            let time = best.map_or_else(|| "--:--.---".to_string(), |(ms, _)| format!("{:02}:{:02}.{:03}", ms / 60000, (ms / 1000) % 60, ms % 1000));
            b.font_text(HudFont::Glass, &time, x + d.d(14.0), d.cap_top(HudFont::Glass, 28.0, y + d.d(19.0), Some(32.0)), &d.font(HudFont::Glass, 28.0, 1.0, WHITE));
            if best.is_some() {
                let st = d.font(HudFont::Glass, 10.0, 1.5, hex(G_ORANGE, 1.0));
                let pw = b.font_width(HudFont::Glass, "PB", &st) - st.tracking;
                b.font_text(HudFont::Glass, "PB", x + w - d.d(12.0) - pw, d.cap_top(HudFont::Glass, 10.0, y + d.d(8.0), None), &st);
            }
            Self::glass_elevation(b, d, cards[3], tr);
            Self::glass_plate(b, d, plate, "187-C-60", "TRACKSCREEN");
            // the sector legend (the mockup's "03 / 12" counter is left out: it collides with the arrows of long names)
            let st = d.font(HudFont::Glass, 11.0, 1.0, hex(G_TEXT, 1.0));
            let (mut lx, ly) = (d.x(960.0), d.cap_top(HudFont::Glass, 11.0, d.y(521.0 + GLASS_ELEV_DROP), None));
            let mut m = Mesh::default();
            for (i, (name, col)) in [("S1", G_S1), ("S2", G_S2), ("S3", G_S3)].into_iter().enumerate() {
                if i > 0 {
                    lx += d.d(4.0);
                }
                let mid = ly + st.size * 0.5;
                m.fill(&vec::rounded_rect(lx, mid - d.d(4.0), d.d(8.0), d.d(8.0), d.d(2.0)), hex(col, 1.0), px);
                lx += d.d(8.0) + d.d(6.0);
                lx += b.font_text(HudFont::Glass, name, lx, ly, &st) - st.tracking + d.d(6.0);
            }
            b.mesh(&m);
            let browse = g.locks().tracks;
            Self::glass_selector(b, d, &tr.name.to_uppercase(), g.sel == 0, pulse, (g.settings.track == 0, g.settings.track + 1 >= browse));
            Self::glass_tips(b, d, g.controls.last, 561.0, locked.then_some(2));
            Self::glass_settings(b, d, settings, "RACE SETTINGS", &g.track_info());
            Self::glass_version(b, d);
        });
        let mut out = vec![back];
        out.extend(self.hologram(size, None, &blur));
        out.push(front);
        out
    }

    /// Under the hologram: our navy backdrop with its light pools, the mockup's halftone dots in
    /// the top-left corner and its two blueprint grids (32 and 160 px).
    fn glass_backdrop(b: &mut Batch, d: Glass) {
        let (w, h) = (b.canvas.w, b.canvas.h);
        let mut m = Mesh::default();
        let top = [0.02, 0.03, 0.05, 1.0];
        let bot = [0.015, 0.04, 0.08, 1.0];
        m.quad(0.0, 0.0, w, h, [top, top, bot, bot]);
        let centre = [d.x(622.0), d.y(360.0)];
        m.disc([centre[0] - 150.0, centre[1] + 150.0], 330.0, [0.03, 0.2, 0.55, 0.40], [0.03, 0.2, 0.55, 0.0], 96);
        m.disc([centre[0] + 60.0, centre[1] - 30.0], 380.0, [0.05, 0.14, 0.25, 0.30], [0.05, 0.14, 0.25, 0.0], 96);
        for (step, a) in [(32.0, 0.05), (160.0, 0.09)] {
            let col = hex(0x8fd0ff, a);
            let mut x = 0.0;
            while x < w {
                m.stroke(&[[x, 0.0], [x, h]], false, d.w(1.0), Ends::Butt, col, d.px);
                x += d.d(step);
            }
            let mut y = 0.0;
            while y < h {
                m.stroke(&[[0.0, y], [w, y]], false, d.w(1.0), Ends::Butt, col, d.px);
                y += d.d(step);
            }
        }
        Self::glass_dots(&mut m, d);
        b.mesh(&m);
    }

    /// The halftone in the top-left corner: 26 px cells of 14 px dots over 300 x 220, masked out by
    /// 230 px from the corner.
    fn glass_dots(m: &mut Mesh, d: Glass) {
        for j in 0..9 {
            for i in 0..12 {
                let (x, y) = (13.0 + 26.0 * i as f32, 13.0 + 26.0 * j as f32);
                let dist = (x * x + y * y).sqrt();
                let mask = if dist < 120.0 { 1.0 - 0.4 * dist / 120.0 } else { (0.6 * (1.0 - (dist - 120.0) / 110.0)).max(0.0) };
                if mask > 0.0 {
                    m.fill(&circle([d.l(x), d.t(y)], d.d(7.0)), hex(0x8a94a6, 0.4 * mask), d.px);
                }
            }
        }
    }

    /// The orange glow behind the title (the mockup's `.title-wash`).
    fn glass_title_wash(m: &mut Mesh, d: Glass) {
        let (gx, gy, gw, gh) = (d.l(-80.0), d.t(-110.0), d.d(560.0), d.d(360.0));
        m.shade_rect(gx, gy, gw, gh, 28, 18, &|q| {
            let k = (((q[0] - gx - gw * 0.5) / (gw * 0.5)).powi(2) + ((q[1] - gy - gh * 0.5) / (gh * 0.5)).powi(2)).sqrt();
            hex(0xff8228, 0.2 * (1.0 - k).max(0.0))
        });
    }

    /// Over the hologram, under everything else: the vignette, the orange wash from the left,
    /// the orange glow behind the title and the four blurred colour orbs.
    fn glass_overlays(b: &mut Batch, d: Glass) {
        let mut m = Mesh::default();
        let (x, y, w, h) = (d.x(0.0), d.y(188.0), d.d(1195.0), d.d(528.0));
        let (cx, cy, rx, ry) = (x + w * 0.5, y + h * 0.45, w * 0.5 * 2f32.sqrt(), h * 0.55 * 2f32.sqrt());
        m.shade_rect(x, y, w, h, 40, 20, &|q| {
            let k = (((q[0] - cx) / rx).powi(2) + ((q[1] - cy) / ry).powi(2)).sqrt();
            [0.0, 0.0, 0.0, 0.55 * ((k - 0.58) / 0.42).clamp(0.0, 1.0)]
        });
        let cw = b.canvas.w;
        m.shade_rect(0.0, 0.0, cw, 480.0, 16, 1, &|q| hex(0xff7828, 0.18 * (1.0 - q[0] / (0.24 * cw)).max(0.0)));
        Self::glass_title_wash(&mut m, d);
        for (x, y, size, col, a) in [(60.0, 70.0, 100.0, 0xff963c, 0.2), (1130.0, 270.0, 130.0, 0x50aaff, 0.16), (20.0, 560.0, 80.0, 0x50aaff, 0.16), (1100.0, 580.0, 70.0, 0xff963c, 0.16)] {
            let c = [d.x(x + size * 0.5), d.y(y + size * 0.5)];
            m.disc(c, d.d(size * 0.5), hex(col, a), hex(col, 0.0), 48);
        }
        b.mesh(&m);
    }

    /// The HUD furniture: the dashed rules (`rules`), the top and left rulers, the corner brackets, the
    /// header line and the blinking LIVE marker.
    fn glass_furniture(b: &mut Batch, d: Glass, t: f32, header: &str, live: &str, rules: GlassRules) {
        let mut m = Mesh::default();
        let (cw, ch) = (b.canvas.w, b.canvas.h);
        let rule = hex(G_LINE, 0.45);
        let y = d.y(287.0);
        // the dial screens have no photo discs for the rules to run behind: none
        let segments: &[(f32, f32)] = match rules {
            GlassRules::None => &[],
            GlassRules::Track => &[(0.0, d.x(440.0)), (d.x(804.0), d.x(874.0)), (d.x(1160.0), cw)],
            GlassRules::Pause => &[(0.0, d.l(46.0))],
            GlassRules::Car => &[(0.0, d.l(112.0)), (d.r(1250.0), cw)],
        };
        for &(x0, x1) in segments {
            m.dashed([x0, y], [x1, y], d.d(6.0), d.d(4.0), d.w(1.0), rule, d.px);
        }
        // rulers: a tick every 8 px, a long one every 40
        let (minor, major) = (hex(G_LINE, 0.5), hex(0xe6f6ff, 0.85));
        let mut i = 0;
        while d.d(8.0 * i as f32) <= d.d(440.0) {
            let x = d.x(500.0) + d.d(8.0 * i as f32);
            let long = i % 5 == 0;
            m.stroke(&[[x, 0.0], [x, d.d(if long { 15.0 } else { 8.0 })]], false, d.w(1.0), Ends::Butt, if long { major } else { minor }, d.px);
            i += 1;
        }
        let mut i = 0;
        while d.d(8.0 * i as f32) <= d.d(360.0) {
            let y = d.y(200.0) + d.d(8.0 * i as f32);
            let long = i % 5 == 0;
            m.stroke(&[[0.0, y], [d.d(if long { 15.0 } else { 8.0 }), y]], false, d.w(1.0), Ends::Butt, if long { major } else { minor }, d.px);
            i += 1;
        }
        // corner brackets, 8 px in, 2 px lines (the top-right one is 10 px)
        let br = hex(G_LINE, 0.6);
        let (ins, wd) = (d.d(8.0), d.w(2.0));
        for (sx, sy, len) in [(1.0, 1.0, 16.0), (-1.0, 1.0, 10.0), (1.0, -1.0, 16.0), (-1.0, -1.0, 16.0)] {
            let (x, y) = (if sx > 0.0 { ins } else { cw - ins }, if sy > 0.0 { ins } else { ch - ins });
            let l = d.d(len);
            m.stroke(&[[x, y - sy * wd * 0.5], [x, y + sy * l]], false, wd, Ends::Butt, br, d.px);
            m.stroke(&[[x - sx * wd * 0.5, y], [x + sx * l, y]], false, wd, Ends::Butt, br, d.px);
        }
        // "● LIVE" (or PAUSED): the dot blinks (1.6 s, ease-in-out between 1 and .25)
        let u = 1.0 - (2.0 * (t / 1.6).fract() - 1.0).abs();
        let blink = 1.0 - 0.75 * (u * u * (3.0 - 2.0 * u));
        let st = d.font(HudFont::Glass, 10.0, 2.0, hex(G_ORANGE, 1.0));
        let lw = b.font_width(HudFont::Glass, live, &st) - st.tracking;
        let (lx, ly) = (d.x(940.0) - lw, d.cap_top(HudFont::Glass, 10.0, d.t(20.0), None));
        m.fill(&circle([lx - d.d(7.5), ly + st.size * 0.5], d.d(3.2)), hex(G_ORANGE, blink), d.px);
        b.mesh(&m);
        b.font_text(HudFont::Glass, live, lx, ly, &st);
        let st = d.font(HudFont::GlassMed, 10.0, 2.0, hex(G_DIM, 1.0));
        b.font_text(HudFont::GlassMed, header, d.x(500.0), d.cap_top(HudFont::GlassMed, 10.0, d.t(20.0), None), &st);
    }

    /// The screen's title in the Motorhead face: the orange gradient with its shadow and glow,
    /// the code under it and the orange rule. A name wider than the room before the header
    /// line (410 design px) is set smaller [design choice].
    fn glass_title(b: &mut Batch, d: Glass, text: &str, code: &str) {
        Self::glass_title_sized(b, d, text, code, None);
    }

    /// `glass_title` with the mockup's own size and top (design px) where it sets one, e.g. the
    /// pause screen's `.title.small` (62 at 31).
    fn glass_title_sized(b: &mut Batch, d: Glass, text: &str, code: &str, set: Option<(f32, f32)>) {
        let full = d.font(HudFont::Title, 74.0, 0.0, WHITE).skew(14.0);
        let (px_size, top) = set.unwrap_or_else(|| (74.0 * (d.d(410.0) / b.font_width(HudFont::Title, text, &full)).min(1.0), 22.0));
        let st = d.font(HudFont::Title, px_size, 0.0, WHITE).skew(14.0);
        let (x, y) = (d.l(62.0), d.cap_top(HudFont::Title, px_size, d.t(top), Some(px_size)));
        let tw = b.font_width(HudFont::Title, text, &st);
        let mut m = Mesh::default();
        // APPROX: the text's 18 px glow as a soft pad behind its box, its 12 px shadow as an offset copy
        m.soft_pad(&vec::rounded_rect(x + d.d(14.0), y + d.d(10.0), tw - d.d(14.0), st.size - d.d(16.0), d.d(16.0)), d.d(28.0), hex(0xff781e, 0.2));
        let (rx, ry, rw, rh) = (d.l(62.0), d.t(120.0) - d.w(1.0) * 0.5, d.d(380.0), d.w(1.0));
        m.shade_rect(rx, ry, rw, rh, 24, 1, &|q| hex(G_ORANGE, (1.0 - (q[0] - rx) / rw).clamp(0.0, 1.0)));
        b.mesh(&m);
        let shadow = FontStyle { color: [0.0, 0.0, 0.0, 0.6], ..st };
        b.font_text(HudFont::Title, text, x + d.d(1.0), y + d.d(6.0), &shadow);
        let stops: Vec<(f32, [f32; 4])> = G_TITLE.iter().map(|&(t, c)| (t, hex(c, 1.0))).collect();
        b.font_text_graded(HudFont::Title, text, x, y, &st, &stops);
        let st = d.font(HudFont::Glass, 21.0, 1.5, hex(0xffb26a, 1.0));
        b.font_text(HudFont::Glass, code, d.l(127.0), d.cap_top(HudFont::Glass, 21.0, d.t(94.0), None), &st);
    }

    /// The overlays of a disc face: a `step` px grid (lines cut at the rim), a gloss (CSS
    /// linear-gradient 140deg, white `gloss` to 0 at `gloss_to`), a radial vignette (clear to
    /// `vig_from` of the corner radius, black `vig` at it; none at 0) and an inset shadow
    /// `inset` px wide of alpha `inset_a`.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn disc_overlays(m: &mut Mesh, d: Glass, c: [f32; 2], r: f32, step: f32, grid: [f32; 4], gloss: (f32, f32), vig: (f32, f32), inset: (f32, f32)) {
        let (x0, y0, side) = (c[0] - r, c[1] - r, 2.0 * r);
        let mut k = 0.0;
        while k < side && grid[3] > 0.0 {
            let off = x0 + k - c[0];
            let half = (r * r - off * off).max(0.0).sqrt();
            m.stroke(&[[x0 + k, c[1] - half], [x0 + k, c[1] + half]], false, d.w(1.0), Ends::Butt, grid, d.px);
            m.stroke(&[[c[0] - half, y0 + k], [c[0] + half, y0 + k]], false, d.w(1.0), Ends::Butt, grid, d.px);
            k += d.d(step);
        }
        let dir = [140f32.to_radians().sin(), -140f32.to_radians().cos()];
        let len = side * (dir[0].abs() + dir[1].abs());
        m.shade_disc(c, r, 10, 64, &|q| {
            let t = ((q[0] - c[0]) * dir[0] + (q[1] - c[1]) * dir[1]) / len + 0.5;
            [1.0, 1.0, 1.0, gloss.0 * (1.0 - t / gloss.1).max(0.0)]
        });
        if vig.0 > 0.0 {
            let rc = r * 2f32.sqrt();
            m.shade_disc(c, r, 10, 64, &|q| [0.0, 0.0, 0.0, vig.0 * (((((q[0] - c[0]).powi(2) + (q[1] - c[1]).powi(2)).sqrt() / rc) - vig.1) / (1.0 - vig.1)).clamp(0.0, 1.0)]);
        }
        m.annulus(c, r - d.d(inset.0), r, [0.0, 0.0, 0.0, 0.0], [0.0, 0.0, 0.0, inset.1], 96);
    }

    /// The dark gap between a face (radius `r`) and its bezel (inner radius `ri`), drawn over
    /// the face's clipped edge with a feather on both sides (the ellipse clip is pixel-hard),
    /// and the bezel's 1 px inset ring.
    pub(crate) fn gap_ring(m: &mut Mesh, d: Glass, c: [f32; 2], r: f32, ri: f32) {
        let dark = hex(G_DARK, 1.0);
        m.annulus(c, r - d.px, r, hex(G_DARK, 0.0), dark, 96);
        m.annulus(c, r, ri, dark, dark, 96);
        m.annulus(c, ri, ri + d.px, dark, hex(G_DARK, 0.0), 96);
        m.stroke(&circle(c, ri - d.d(0.5)), true, d.w(1.0), Ends::Round, [1.0, 1.0, 1.0, 0.12], d.px);
    }

    /// The bezel's conic gradient at angle `a` (radians from 3 o'clock, clockwise).
    fn bezel(a: f32) -> [f32; 4] {
        stops_at(&G_BEZEL, (a.to_degrees() + 90.0 - 210.0).rem_euclid(360.0) / 360.0)
    }

    /// Degrees clockwise from 12 o'clock (the mockup's arcs) to our radians.
    fn clock(deg: f32) -> f32 {
        (deg - 90.0).to_radians()
    }

    /// The photo disc: the compass ticks round it, the metal bezel with its shadow and glow,
    /// the photo under a grid, a gloss and a vignette, the orange arcs and the bearing labels.
    fn glass_photo(b: &mut Batch, d: Glass, name: String) {
        let c = [d.x(622.0), d.y(288.0)];
        let rp = d.d(170.0);
        Self::glass_disc_back(b, d, 0.0);
        // the photo (cover: the 6:5 picture fills the height), cut to the disc with its overlays
        b.clip_ellipse(c[0], c[1], rp, rp);
        b.image_scaled(&name, c[0] - rp * 1.2, c[1] - rp, rp * 2.4, rp * 2.0, WHITE);
        b.clip_none();
        Self::glass_disc_front(b, d, (hex(G_LINE, 0.1), 44.0, 0.28, 0.45));
        Self::glass_disc_labels(b, d);
    }

    /// The big disc under its face: the compass ticks, the two orange arcs (at 30..70 and 250..290 degrees clockwise
    /// from 12 o'clock, turned `turn` radians clockwise: the Car screen's follow its car's nose), the bezel's shadow,
    /// glow and conic ring and the dark disc inside it.
    fn glass_disc_back(b: &mut Batch, d: Glass, turn: f32) {
        let c = [d.x(622.0), d.y(288.0)];
        let (rb, ri) = (d.d(186.0), d.d(176.0));
        let mut m = Mesh::default();
        m.ticks(c, d.d(192.0), 60, d.d(8.0), d.w(1.5), hex(G_TEXT, 0.55), -FRAC_PI_2, d.px);
        m.ticks(c, d.d(189.0), 10, d.d(14.0), d.w(2.5), hex(0xf4fbff, 0.9), -FRAC_PI_2, d.px);
        for (a0, a1) in [(30.0, 70.0), (250.0, 290.0)] {
            let arc = vec::arc(c, d.d(208.0), Self::clock(a0) + turn, Self::clock(a1) + turn, 24);
            m.glow(&arc, false, d.d(3.0), d.d(5.0), hex(G_ORANGE, 0.6), 3);
            m.stroke(&arc, false, d.w(3.0), Ends::Round, hex(G_ORANGE, 1.0), d.px);
        }
        let bez = circle(c, rb);
        m.shadow(&bez, 0.0, d.d(24.0), d.d(54.0), [0.0, 0.0, 0.0, 0.65]);
        m.halo(&bez, d.d(70.0), hex(0x50aaff, 0.28));
        m.ring(c, ri, rb, 180, &Self::bezel, d.px);
        m.disc(c, ri, hex(G_DARK, 1.0), hex(G_DARK, 1.0), 96);
        b.mesh(&m);
    }

    /// The big disc's overlays over its face: the grid (`grid` colour, step, none at alpha 0), the gloss (alpha `gloss`),
    /// the vignette (black `vig` at the corner radius; Track's .45), the inset shadow and the dark gap to the bezel.
    fn glass_disc_front(b: &mut Batch, d: Glass, (grid_col, grid_step, gloss, vig): ([f32; 4], f32, f32, f32)) {
        let c = [d.x(622.0), d.y(288.0)];
        let (ri, rp) = (d.d(176.0), d.d(170.0));
        let mut m = Mesh::default();
        Self::disc_overlays(&mut m, d, c, rp, grid_step, grid_col, (gloss, 0.36), (vig, 0.62), (36.0, 0.7));
        Self::gap_ring(&mut m, d, c, rp, ri);
        b.mesh(&m);
    }

    /// The bearing labels 000 / 090 / 270 round the big disc.
    fn glass_disc_labels(b: &mut Batch, d: Glass) {
        let st = d.font(HudFont::Glass, 10.0, 1.5, hex(G_TEXT, 1.0));
        let (w0, w2) = (b.font_width(HudFont::Glass, "000", &st) - st.tracking, b.font_width(HudFont::Glass, "270", &st) - st.tracking);
        b.font_text(HudFont::Glass, "000", d.x(622.0) - w0 * 0.5, d.cap_above(HudFont::Glass, 10.0, d.y(82.0)), &st);
        b.font_text(HudFont::Glass, "090", d.x(832.0), d.cap_above(HudFont::Glass, 10.0, d.y(291.0)), &st);
        b.font_text(HudFont::Glass, "270", d.x(412.0) - w2, d.cap_above(HudFont::Glass, 10.0, d.y(291.0)), &st);
    }

    /// The small bezel disc (`.disc-s`, 6 px wide): the shadow 0 / 20 / 44 at .6, the blue glow 50 at .22, the conic
    /// ring and the dark disc inside it. `c`, `rb` in canvas px.
    pub(crate) fn glass_bezel_small(m: &mut Mesh, d: Glass, c: [f32; 2], rb: f32) {
        let ri = rb - d.d(6.0);
        let bez = circle(c, rb);
        m.shadow(&bez, 0.0, d.d(20.0), d.d(44.0), [0.0, 0.0, 0.0, 0.6]);
        m.halo(&bez, d.d(50.0), hex(0x50aaff, 0.22));
        m.ring(c, ri, rb, 160, &Self::bezel, d.px);
        m.disc(c, ri, hex(G_DARK, 1.0), hex(G_DARK, 1.0), 96);
    }

    /// REMASTER stat dial (`statDialHTML`, #95; 19.3 "Stat dial"): a small bezel disc `dial.size` design px across
    /// at `dial.c`, a 270 degree arc from 7:30 o'clock filled to `fill` (a fill past 1 draws full), eleven ticks,
    /// a dim tick for the car shown before, the value and unit in the middle and the caption at the bottom.
    fn glass_stat_dial(b: &mut Batch, d: Glass, dial: &StatDial) {
        let c = [d.x(dial.c.0), d.y(dial.c.1)];
        let rb = d.d(dial.size * 0.5);
        let (ri, rf) = (rb - d.d(6.0), rb - d.d(12.0));
        let mut m = Mesh::default();
        m.ticks(c, rb + d.d(5.5), 80, d.d(5.0), d.w(1.0), hex(G_TEXT, 0.55), -FRAC_PI_2, d.px);
        Self::glass_bezel_small(&mut m, d, c, rb);
        // `.face.dial`: radial rgba(120,190,255,.16) -> rgba(8,20,40,.88) from 50 % / 30 %, a white .10 -> .03 sheen over it, no grid
        m.disc_at(c, rf, [c[0], c[1] - 0.4 * rf], [120.0 / 255.0, 190.0 / 255.0, 1.0, 0.16], [8.0 / 255.0, 20.0 / 255.0, 40.0 / 255.0, 0.88], 96);
        m.shade_disc(c, rf, 8, 64, &|q| [1.0, 1.0, 1.0, 0.10 - 0.07 * ((q[1] - (c[1] - rf)) / (2.0 * rf)).clamp(0.0, 1.0)]);
        Self::disc_overlays(&mut m, d, c, rf, 22.0, hex(G_LINE, 0.0), (0.2, 0.34), (0.0, 0.0), (30.0, 0.6));
        Self::gap_ring(&mut m, d, c, rf, ri);
        // the arc: r = face / 2 - 11, 135 degrees clockwise from 3 o'clock, 270 wide
        let r = rf - d.d(11.0);
        let ang = |t: f32| (135.0 + 270.0 * t).to_radians();
        let at = |t: f32, rr: f32| [c[0] + ang(t).cos() * rr, c[1] + ang(t).sin() * rr];
        m.stroke(&vec::arc(c, r, ang(0.0), ang(1.0), 48), false, d.w(5.0), Ends::Round, [190.0 / 255.0, 230.0 / 255.0, 1.0, 0.14], d.px);
        let fill = dial.fill.min(1.0);
        if fill > 0.005 {
            let arc = vec::arc(c, r, ang(0.0), ang(fill), 48);
            m.glow(&arc, false, d.d(5.0), d.d(6.0), hex(0x4aa8ff, 0.6), 3);
            m.stroke(&arc, false, d.w(5.0), Ends::Round, hex(G_S1, 1.0), d.px);
        }
        for i in 0..=10 {
            let major = i % 5 == 0;
            let t = i as f32 / 10.0;
            let (col, w) = if major { (hex(0xf4fbff, 0.85), 1.6) } else { (hex(G_TEXT, 0.45), 1.0) };
            m.stroke(&[at(t, r - d.d(8.0)), at(t, r - d.d(if major { 14.0 } else { 11.0 }))], false, d.w(w), Ends::Butt, col, d.px);
        }
        if let Some(p) = dial.prev {
            let t = p.min(1.0);
            m.stroke(&[at(t, r - d.d(6.0)), at(t, r + d.d(6.0))], false, d.w(2.0), Ends::Round, [244.0 / 255.0, 251.0 / 255.0, 1.0, 0.7], d.px);
        }
        b.mesh(&m);
        // value (Glass bold, 23 / 26 / 30 by size), unit and caption, centred on the face
        let big = if dial.size >= 140.0 { 30.0 } else if dial.size >= 124.0 { 26.0 } else { 23.0 };
        let top = c[1] - rf;
        let centre = |b: &mut Batch, f: HudFont, text: &str, st: &FontStyle, y: f32| {
            let w = b.font_width(f, text, st);
            b.font_text(f, text, c[0] - w * 0.5, y, st);
        };
        let face_c = rf;
        let st = d.font(HudFont::Glass, big, 1.0, WHITE);
        centre(b, HudFont::Glass, &dial.value, &st, d.cap_top(HudFont::Glass, big, top + face_c - d.d(big * 0.62 + 4.0), Some(big)));
        let cap = |col: u32| d.font(HudFont::GlassMed, 11.0, 1.5, hex(col, 1.0));
        if !dial.unit.is_empty() {
            centre(b, HudFont::GlassMed, dial.unit, &cap(G_DIM), d.cap_top(HudFont::GlassMed, 11.0, top + face_c + d.d(big * 0.42), None));
        }
        centre(b, HudFont::GlassMed, dial.caption, &cap(G_TEXT), d.cap_top(HudFont::GlassMed, 11.0, top + 2.0 * rf - d.d(26.0), None));
    }

    /// The map disc: its tick ring, bezel, dark grid face and blue arc, with the lap drawn from
    /// the centreline in the three sector colours, the start/finish mark and the numbered turns
    /// (oriented like the race HUD's map, `minimap::map_point`).
    /// The lap `p` of `tr` in the HUD map's projection, centred on `c` and scaled so the circle round it has radius `r`
    /// (canvas px): the Track, pause and Rankings maps. `LapFit::at` places any other point of the track the same way.
    pub(super) fn fit_lap(tr: &TrackInfo, p: &motor_sim::race::TrackProfile, c: [f32; 2], r: f32) -> LapFit {
        let map = |q: [f32; 3]| minimap::map_point(tr.map_info, Vec3::from_array(q));
        let (mut lo, mut hi) = ((f32::MAX, f32::MAX), (f32::MIN, f32::MIN));
        for &q in &p.outline {
            let (x, y) = map(q);
            lo = (lo.0.min(x), lo.1.min(y));
            hi = (hi.0.max(x), hi.1.max(y));
        }
        let mid = ((lo.0 + hi.0) * 0.5, (lo.1 + hi.1) * 0.5);
        let half = (((hi.0 - lo.0) * 0.5).powi(2) + ((hi.1 - lo.1) * 0.5).powi(2)).sqrt().max(1e-3);
        LapFit { map_info: tr.map_info, mid, k: r / half, c }
    }

    fn glass_map(b: &mut Batch, d: Glass, tr: &TrackInfo, disc: &MapDisc, cars: &[MapCar]) {
        let c = [d.x(disc.c.0), d.y(disc.c.1)];
        let (rb, ri, rf) = (d.d(disc.bezel), d.d(disc.bezel - 6.0), d.d(disc.bezel - 12.0));
        let mut m = Mesh::default();
        if disc.ticks {
            m.ticks(c, d.d(130.5), 80, d.d(5.0), d.w(1.0), hex(G_TEXT, 0.55), -FRAC_PI_2, d.px);
        }
        Self::glass_bezel_small(&mut m, d, c, rb);
        // the face: radial from 50 % / 30 %, the 22 px grid, the inset shadow and the gloss
        m.disc_at(c, rf, [c[0], c[1] - 0.4 * rf], [120.0 / 255.0, 190.0 / 255.0, 1.0, 0.24], [8.0 / 255.0, 20.0 / 255.0, 40.0 / 255.0, 0.9], 96);
        Self::disc_overlays(&mut m, d, c, rf, 22.0, hex(G_LINE, 0.12), (0.2, 0.34), (0.0, 0.0), (30.0, 0.6));
        Self::gap_ring(&mut m, d, c, rf, ri);
        b.mesh(&m);
        let mut m = Mesh::default();
        if let Some((r, from, to)) = disc.arc {
            let arc = vec::arc(c, d.d(r), from, to, 40);
            m.glow(&arc, false, d.d(3.0), d.d(6.0), hex(0x4aa8ff, 0.6), 3);
            m.stroke(&arc, false, d.w(3.0), Ends::Round, hex(G_S1, 0.85), d.px);
        }
        let Some(p) = &tr.profile else {
            b.mesh(&m);
            return;
        };
        // the lap in the HUD map's projection, fitted into the face
        let fit = Self::fit_lap(tr, p, c, 0.84 * rf);
        let line = fit.line(p);
        // the three sectors: each polyline runs on to the next sector's first point
        if !disc.sectors {
            let mut run = line.clone();
            run.push(line[0]);
            m.stroke(&run, false, d.w(1.7), Ends::Round, hex(G_TEXT, 0.7), d.px);
        }
        let runs = sector_runs(p, &line);
        for (run, col) in runs.iter().zip([G_S1, G_S2, G_S3]).filter(|_| disc.sectors) {
            if run.is_empty() {
                continue;
            }
            m.glow(run, false, d.d(3.5), d.d(5.0), hex(col, 0.55), 3);
            m.stroke(run, false, d.w(3.5), Ends::Round, hex(col, 1.0), d.px);
        }
        b.mesh(&m);
        // start/finish: a white tick across the line (no label: it only cluttered the map)
        if disc.sectors {
            let mut m = Mesh::default();
            start_tick(&mut m, &line, d.d(8.0), d.w(2.0), d.px);
            b.mesh(&m);
        }
        // the cars (the pause screen): the rivals first, the focus car ringed on top
        let mut m = Mesh::default();
        for car in cars.iter().filter(|c| !c.me).chain(cars.iter().filter(|c| c.me)) {
            let at = fit.at(car.pos.to_array());
            let col = (u32::from(car.colour[0]) << 16) | (u32::from(car.colour[1]) << 8) | u32::from(car.colour[2]);
            if car.me {
                let dot = circle(at, d.d(5.4 * disc.dot));
                m.halo(&dot, d.d(10.0 * disc.dot), hex(col, 0.7));
                m.fill(&dot, hex(col, 1.0), d.px);
                m.stroke(&dot, true, d.w(1.8 * disc.dot), Ends::Round, WHITE, d.px);
            } else {
                let dot = circle(at, d.d(3.2 * disc.dot));
                m.fill(&dot, hex(col, 1.0), d.px);
                m.stroke(&dot, true, d.w(1.1 * disc.dot), Ends::Round, hex(G_DARK, 1.0), d.px);
            }
        }
        b.mesh(&m);
        // the turns, numbered outside the bend
        let st = d.font(HudFont::Glass, 9.0, 0.0, hex(G_TEXT, 1.0));
        for (i, turn) in p.turns.iter().enumerate().filter(|_| disc.turns) {
            let apex = fit.at(turn.apex);
            let out = fit.at([turn.apex[0] + turn.outward[0] * 10.0, turn.apex[1], turn.apex[2] + turn.outward[1] * 10.0]);
            let v = [out[0] - apex[0], out[1] - apex[1]];
            let vl = (v[0] * v[0] + v[1] * v[1]).sqrt().max(1e-3);
            let label = (i + 1).to_string();
            let lw = b.font_width(HudFont::Glass, &label, &st);
            let at = [apex[0] + v[0] / vl * d.d(11.0), apex[1] + v[1] / vl * d.d(11.0)];
            b.font_text(HudFont::Glass, &label, at[0] - lw * 0.5, at[1] - st.size * 0.5, &st);
        }
    }

    /// The ELEVATION card's content: the lap's height profile, the length and the turn count.
    fn glass_elevation(b: &mut Batch, d: Glass, (x, y, _, _): (f32, f32, f32, f32), tr: &TrackInfo) {
        b.font_text(HudFont::GlassMed, "ELEVATION", x + d.d(12.0), d.cap_top(HudFont::GlassMed, 11.0, y + d.d(6.0), None), &d.font(HudFont::GlassMed, 11.0, 1.5, hex(G_DIM, 1.0)));
        let (sx, sy, sw, sh) = (x + d.d(12.0), y + d.d(22.0), d.d(176.0), d.d(26.0));
        let mut m = Mesh::default();
        let (length, turns) = match &tr.profile {
            Some(p) => {
                let top = p.elevation.iter().copied().fold(0.0, f32::max).max(1.0);
                let n = p.elevation.len();
                let line: Vec<[f32; 2]> = p.elevation.iter().enumerate().map(|(i, &h)| [sx + sw * i as f32 / (n - 1) as f32, sy + sh - d.d(4.0) - d.d(18.0) * h / top]).collect();
                let mut poly = line.clone();
                poly.push([sx + sw, sy + sh]);
                poly.push([sx, sy + sh]);
                m.fill(&poly, hex(0x8fd0ff, 0.18), d.px);
                m.stroke(&line, false, d.w(1.5), Ends::Round, hex(G_TEXT, 1.0), d.px);
                (if tr.length_m > 0.0 { tr.length_m } else { p.length }, p.turns.len())
            }
            None => (tr.length_m, 0),
        };
        b.mesh(&m);
        let st = d.font(HudFont::Glass, 13.0, 1.0, WHITE);
        let ty = d.cap_top(HudFont::Glass, 13.0, y + d.d(52.0), None);
        let mut tx = x + d.d(12.0);
        tx += b.font_text(HudFont::Glass, &format!("{:.2} KM", length / 1000.0), tx, ty, &st);
        let mut m = Mesh::default();
        m.fill(&circle([tx + d.d(3.0), ty + st.size * 0.55], d.d(1.6)), hex(G_DIM, 1.0), d.px);
        b.mesh(&m);
        b.font_text(HudFont::Glass, &format!("{turns} TURNS"), tx + d.d(10.0), ty, &st);
    }

    /// The SCREEN plate in the top-right corner (the mockup's, lifted clear of the map ring).
    fn glass_plate(b: &mut Batch, d: Glass, (x, y, w, h): (f32, f32, f32, f32), code: &str, name: &str) {
        let mut m = Mesh::default();
        glass_card(&mut m, d, (x, y, w, h), d.d(12.0), (0.10, 0.03), 0.3, (10.0, 30.0, 0.45), 0.3, None);
        m.fill(&rounded_rect4(x + d.d(4.0), y + d.d(4.0), d.d(120.0), d.d(52.0), [d.d(8.0), d.d(8.0), d.d(16.0), d.d(8.0)]), [1.0, 1.0, 1.0, 0.07], d.px);
        let by = y + d.d(6.0);
        m.dashed([x + w - d.d(12.0) - d.d(76.0), by + d.d(25.0)], [x + w - d.d(12.0), by + d.d(25.0)], d.d(2.0), d.d(2.0), d.d(50.0), hex(G_PLATE, 1.0), d.px);
        m.fill(&circle([x + w - d.d(17.0), y + h - d.d(16.0)], d.d(7.0)), hex(0xdfe5ec, 1.0), d.px);
        b.mesh(&m);
        let plate = hex(G_PLATE, 1.0);
        b.font_text(HudFont::Glass, "SCREEN", x + d.d(12.0), d.cap_top(HudFont::Glass, 30.0, y + d.d(6.0), Some(30.0)), &d.font(HudFont::Glass, 30.0, 1.0, plate));
        b.font_text(HudFont::GlassMed, code, x + d.d(12.0), d.cap_top(HudFont::GlassMed, 16.0, y + d.d(36.0), None), &d.font(HudFont::GlassMed, 16.0, 1.0, plate));
        let st = d.font(HudFont::Glass, 10.0, 1.0, plate);
        let mw = b.font_width(HudFont::Glass, "MOTORHEAD.", &st) - st.tracking;
        b.font_text(HudFont::Glass, "MOTORHEAD.", x + w - d.d(12.0) - mw, d.cap_top(HudFont::Glass, 10.0, y + d.d(60.0), None), &st);
        let st = d.font(HudFont::Glass, 14.0, 2.0, plate);
        let tw = b.font_width(HudFont::Glass, "TUNNELVISION", &st) - st.tracking;
        b.font_text(HudFont::Glass, "TUNNELVISION", x + (w - tw) * 0.5, d.cap_top(HudFont::Glass, 14.0, y + d.d(70.0), None), &st);
        let st = d.font(HudFont::Orbitron, 15.0, 0.0, hex(0x9aa4b2, 1.0)).skew(14.0);
        b.font_text(HudFont::Orbitron, name, x + d.d(14.0), d.cap_top(HudFont::Orbitron, 15.0, y + d.d(90.0), None), &st);
    }

    /// The selector under the photo: the track's name with its chromatic shadow and glow, the
    /// two arrows (orange while it is the selected item).
    /// `ends`: the list is at its first / last entry, so that arrow is dimmed like `glass_tri`'s end (the lists don't wrap).
    fn glass_selector(b: &mut Batch, d: Glass, name: &str, sel: bool, pulse: f32, ends: (bool, bool)) {
        let st = d.font(HudFont::Glass, 50.0, 2.0, hex(0xf4fbff, 1.0));
        let nw = b.font_width(HudFont::Glass, name, &st) - st.tracking;
        let (x, y) = (d.x(622.0) - nw * 0.5, d.cap_top(HudFont::Glass, 50.0, d.y(487.0), Some(56.0)));
        let mut m = Mesh::default();
        // APPROX: the 26 px text glow as a soft pad behind the run
        m.soft_pad(&vec::rounded_rect(x + d.d(4.0), y + d.d(4.0), nw - d.d(8.0), st.size - d.d(8.0), d.d(10.0)), d.d(26.0), [120.0 / 255.0, 200.0 / 255.0, 1.0, 0.22]);
        let (col, glow) = if sel { (hex(G_ORANGE, 1.0), hex(G_ORANGE, 0.55 + 0.3 * pulse)) } else { (hex(0xd8f2ff, 1.0), hex(0x6fcbff, 0.6)) };
        // a name wider than the mockup's pushes the arrows out (12 px clear of the run)
        let push = (nw * 0.5 + d.d(12.0) - (d.x(622.0) - d.x(498.0))).max(0.0);
        for (ox, sg, pts, end) in [(474.0, -1.0, [[24.0, 3.0], [5.0, 15.0], [24.0, 27.0]], ends.0), (742.0, 1.0, [[4.0, 3.0], [23.0, 15.0], [4.0, 27.0]], ends.1)] {
            let k = if end { 0.2 } else { 1.0 };
            let tri: Vec<[f32; 2]> = pts.iter().map(|p| [d.x(ox + p[0]) + sg * push, d.y(501.0 + p[1])]).collect();
            if !end {
                m.glow(&tri, true, d.d(2.2), d.d(6.0), glow, 3);
            }
            m.fill(&tri, [1.0, 1.0, 1.0, 0.14 * k], d.px);
            m.stroke(&tri, true, d.w(2.2), Ends::Round, [col[0], col[1], col[2], col[3] * k], d.px);
        }
        b.mesh(&m);
        b.font_text(HudFont::Glass, name, x - d.d(1.5), y, &FontStyle { color: [1.0, 70.0 / 255.0, 70.0 / 255.0, 0.55], ..st });
        b.font_text(HudFont::Glass, name, x + d.d(1.5), y, &FontStyle { color: [70.0 / 255.0, 200.0 / 255.0, 1.0, 0.55], ..st });
        b.font_text(HudFont::Glass, name, x, y, &st);
    }

    /// A disc face that can't be used: shaded under `shade` of black (disc radius `r` design px) with a blinking tag
    /// over it, blinking like the original's (on while the 1024 ms clock is below 512, 0x46d25e / 0x46c6de).
    pub(super) fn glass_blocked(&self, b: &mut Batch, d: Glass, px: f32, r: f32, shade: f32, text: &str) {
        let mut m = Mesh::default();
        m.fill(&circle([d.x(622.0), d.y(288.0)], d.d(r)), [0.0, 0.0, 0.0, shade], px);
        b.mesh(&m);
        if (self.clock * 1000.0) as u32 & 0x3ff < 0x200 {
            let tw = Self::tag_width(b, d, text);
            Self::glass_tag(b, d, text, (d.x(622.0) - tw * 0.5, d.y(368.0)));
        }
    }

    /// The hints LEFT / RIGHT / SELECT / BACK in the mockup's places (line box top `top`, design
    /// px), the device's glyph where the mockup has a blank key cap.
    /// `off`: the hint at that index dimmed to .35 (an action that does nothing now).
    fn glass_tips(b: &mut Batch, d: Glass, device: Device, top: f32, off: Option<usize>) {
        let st = d.font(HudFont::Glass, 23.0, 1.0, hex(0xa9d3e6, 1.0));
        let ty = d.cap_top(HudFont::Glass, 23.0, d.y(top), None);
        for (i, (x, label, kind)) in [(431.0, "LEFT", 0), (543.0, "RIGHT", 1), (667.0, "SELECT", 2), (800.0, "BACK", 3)].into_iter().enumerate() {
            let a = if off == Some(i) { 0.35 } else { 1.0 };
            let (c, tint) = Self::glyph_for(device, kind);
            let mut gs = FontStyle::new(d.d(15.0), tint);
            gs.color = tint;
            gs.color[3] *= a;
            let mut st = st;
            st.color[3] *= a;
            let gw = b.font_text(HudFont::Glyphs, &c.to_string(), d.x(x), ty + st.size * 0.5 - gs.size * 0.5, &gs);
            b.font_text(HudFont::Glass, label, d.x(x) + gw + d.d(7.0), ty, &st);
        }
    }

    /// The RACE SETTINGS panel: the glass strip with its orange bar and tab, two columns of
    /// label / value pairs.
    fn glass_settings(b: &mut Batch, d: Glass, (x, y, w, h): (f32, f32, f32, f32), tag_text: &str, info: &DialInfo) {
        let mut m = Mesh::default();
        glass_card(&mut m, d, (x, y, w, h), d.d(12.0), (0.12, 0.04), 0.36, (14.0, 34.0, 0.5), 0.36, None);
        let bar = vec::rounded_rect(x, y + d.d(14.0), d.d(3.0), d.d(52.0), d.d(2.0));
        m.glow(&bar, true, d.d(1.0), d.d(10.0), hex(G_ORANGE, 0.7), 3);
        m.fill(&bar, hex(G_ORANGE, 1.0), d.px);
        let st = d.font(HudFont::Glass, 10.0, 1.5, hex(G_ORANGE, 1.0));
        let tw = b.font_width(HudFont::Glass, tag_text, &st) - st.tracking;
        let tag = vec::rounded_rect(d.x(343.0), d.y(607.0), tw + d.d(14.0), d.d(14.0), d.d(7.0));
        m.fill(&tag, hex(0x0a1626, 1.0), d.px);
        m.stroke(&tag, true, d.w(1.0), Ends::Round, hex(G_LINE, 0.3), d.px);
        b.mesh(&m);
        b.font_text(HudFont::Glass, tag_text, d.x(343.0) + d.d(7.0), d.cap_top(HudFont::Glass, 10.0, d.y(607.0), Some(14.0)), &st);
        let key = d.font(HudFont::GlassMed, 15.0, 0.5, hex(G_DIM, 1.0));
        let val = d.font(HudFont::GlassMed, 15.0, 0.5, WHITE);
        for (c, col) in info.iter().enumerate() {
            let (kx, label_w) = if c == 0 { (x + d.d(22.0), d.d(62.0)) } else { (x + d.d(352.0), d.d(92.0)) };
            // a longer label (TRANSMISSION:) pushes its values out
            let label_w = col.iter().map(|(k, _)| b.font_width(HudFont::GlassMed, &k.to_uppercase(), &key) - key.tracking + d.d(8.0)).fold(label_w, f32::max);
            for (i, (k, v)) in col.iter().enumerate() {
                let ty = d.cap_top(HudFont::GlassMed, 15.0, y + d.d(10.0) + d.d(20.0) * i as f32, Some(20.0));
                b.font_text(HudFont::GlassMed, &k.to_uppercase(), kx, ty, &key);
                b.font_text(HudFont::GlassMed, &v.to_uppercase(), kx + label_w, ty, &val);
            }
        }
    }
}

impl Game {
    /// The version (`motor_version::DISPLAY`), right-aligned in the bottom right corner.
    fn glass_version(b: &mut Batch, d: Glass) {
        let st = d.font(HudFont::Glass, 22.0, 1.0, hex(0x4a5566, 1.0));
        let ver = motor_version::DISPLAY;
        // ends where the original `rs0.1` ended (it started at 1203 design px)
        let right = d.r(1203.0) + b.font_width(HudFont::Glass, "rs0.1", &st);
        let w = b.font_width(HudFont::Glass, ver, &st);
        b.font_text(HudFont::Glass, ver, right - w, d.cap_top(HudFont::Glass, 22.0, d.b(683.0), None), &st);
    }

    /// The copyright line under the dial (the dial screens without a summary strip).
    fn glass_copyright(b: &mut Batch, d: Glass) {
        let st = d.font(HudFont::GlassMed, 15.0, 1.5, hex(G_DIM, 0.8));
        let text = "(C) DIGITAL ILLUSIONS CS 1997-98";
        let w = b.font_width(HudFont::GlassMed, text, &st) - st.tracking;
        b.font_text(HudFont::GlassMed, text, d.x(GLASS_DIAL_C.0) - w * 0.5, d.cap_top(HudFont::GlassMed, 15.0, d.y(650.0), None), &st);
    }

    /// The dial's flat-bottomed outline: the arc of radius `r` over the top, closed by the chord
    /// `chord` below the centre.
    fn dial_outline(c: [f32; 2], r: f32, chord: f32) -> Vec<[f32; 2]> {
        let corner = (chord / r).asin();
        vec::arc(c, r, std::f32::consts::PI - corner, TAU + corner, 120)
    }

    /// The Glass dial (19.3): a frosted face (white fill, 44 px grid, 140 deg gloss, inset shadow)
    /// in a dark gap and a metal bezel with its shadow and glow, the tick scale, the needle and
    /// the items (selected = orange, pulsing glow).
    fn glass_dial(&self, b: &mut Batch, d: Glass, c: [f32; 2], items: &[(&str, f32, f32)]) {
        let (r_out, bz, gap) = (d.d(GLASS_DIAL_R), d.d(GLASS_DIAL_BEZEL), d.d(GLASS_DIAL_GAP));
        let rf = r_out - bz - gap;
        // the three outlines share one chord that steps down by the gap and the bezel
        let face = Self::dial_outline(c, rf, CHORD * rf);
        let ring = Self::dial_outline(c, rf + gap, CHORD * rf + gap);
        let outer = Self::dial_outline(c, r_out, CHORD * rf + gap + bz);
        let chord_y = c[1] + CHORD * rf;
        let mut m = Mesh::default();
        m.shadow(&outer, 0.0, d.d(24.0), d.d(54.0), [0.0, 0.0, 0.0, 0.65]);
        m.halo(&outer, d.d(70.0), hex(0x50aaff, 0.28));
        m.fill_with(&face, &|q| [1.0, 1.0, 1.0, 0.10 - 0.07 * ((q[1] - (c[1] - rf)) / (chord_y - c[1] + rf)).clamp(0.0, 1.0)], d.px);
        // the 44 px grid, cut at the rim and the chord
        let (step, grid) = (d.d(44.0), hex(G_LINE, 0.08));
        for i in -((rf / step) as i32)..=((rf / step) as i32) {
            let off = i as f32 * step;
            let half = (rf * rf - off * off).max(0.0).sqrt();
            m.stroke(&[[c[0] + off, c[1] - half], [c[0] + off, (c[1] + half).min(chord_y)]], false, d.w(1.0), Ends::Butt, grid, d.px);
            if c[1] + off <= chord_y {
                m.stroke(&[[c[0] - half, c[1] + off], [c[0] + half, c[1] + off]], false, d.w(1.0), Ends::Butt, grid, d.px);
            }
        }
        // the gloss: CSS linear-gradient 140deg, white .16 to 0 at 34 % (the face cut by the line it ends on)
        let dir = [140f32.to_radians().sin(), -140f32.to_radians().cos()];
        let len = 2.0 * rf * (dir[0].abs() + dir[1].abs());
        let t_of = |q: [f32; 2]| ((q[0] - c[0]) * dir[0] + (q[1] - c[1]) * dir[1]) / len + 0.5;
        let gloss = clip_half(&face, &|q| t_of(q) - 0.34);
        if gloss.len() >= 3 {
            m.fill_with(&gloss, &|q| [1.0, 1.0, 1.0, 0.16 * (1.0 - t_of(q) / 0.34).max(0.0)], d.px);
        }
        m.band(&face, c, (-d.d(24.0), 0.0), &|_| [0.0; 4], &|_| [0.0, 0.0, 0.0, 0.45], 0.0);
        // the dark gap, the conic bezel, its 1 px inset line
        let dark = hex(G_DARK, 1.0);
        m.band(&face, c, (0.0, gap), &|_| dark, &|_| dark, d.px);
        m.band(&ring, c, (0.0, bz), &|q| Self::bezel((q[1] - c[1]).atan2(q[0] - c[0])), &|q| Self::bezel((q[1] - c[1]).atan2(q[0] - c[0])), d.px);
        m.stroke(&ring, true, d.w(1.0), Ends::Round, [1.0, 1.0, 1.0, 0.12], d.px);
        // the scale: majors on every second tick of the needle's sweep, six steps between
        let n_major = 2 * (items.len() - 1);
        for i in 0..=(n_major * PER) {
            let major = i % PER == 0;
            if major && (i / PER) % 2 == 1 {
                continue;
            }
            let a = TICK_FROM + (TICK_TO - TICK_FROM) * i as f32 / (n_major * PER) as f32;
            let (r0, r1, w) = if major { (0.80, 0.935, d.d(2.5)) } else { (0.865, 0.935, d.w(1.5)) };
            let (ca, sa) = (a.cos(), a.sin());
            let seg = [[c[0] + ca * rf * r0, c[1] + sa * rf * r0], [c[0] + ca * rf * r1, c[1] + sa * rf * r1]];
            if major {
                m.glow(&seg, false, w, d.d(4.0), hex(0x6fcbff, 0.4), 3);
            }
            m.stroke(&seg, false, w, Ends::Butt, if major { hex(0xf4fbff, 0.9) } else { hex(G_TEXT, 0.55) }, d.px);
        }
        b.mesh(&m);
        // the items: Rajdhani, the selected one orange on an orange glow. Each sits on the ray of
        // its slot from the dial's centre, pushed out until the label is `gap` clear of the
        // bezel's edge (a point set along the outline and the chord): one distance for all
        let mut edge = outer.clone();
        let (z0, z1) = (outer[outer.len() - 1], outer[0]);
        edge.extend((1..32).map(|i| [z0[0] + (z1[0] - z0[0]) * i as f32 / 32.0, z0[1] + (z1[1] - z0[1]) * i as f32 / 32.0]));
        let gap = d.d(GLASS_DIAL_LABEL_GAP);
        let pulse = 0.5 - 0.5 * (self.t * TAU / 1.1).cos();
        for (i, &(name, dx, dy)) in items.iter().enumerate() {
            let sel = i == self.sel;
            let st = d.font(HudFont::Glass, 36.0, 1.5, if sel { hex(G_ORANGE, 1.0) } else { hex(G_TEXT, 0.9) });
            let w = b.font_width(HudFont::Glass, name, &st) - st.tracking;
            let (hw, hh) = (w * 0.5, st.size * 0.5 + d.d(4.0));
            let len = dx.hypot(dy);
            let u = [dx / len, dy / len];
            let clear = |s: f32| {
                let (bx, by) = (c[0] + u[0] * s, c[1] + u[1] * s);
                edge.iter().map(|p| ((p[0] - bx).abs() - hw).max(0.0).hypot(((p[1] - by).abs() - hh).max(0.0))).fold(f32::MAX, f32::min)
            };
            let (mut lo, mut hi) = (0.0, d.d(900.0));
            for _ in 0..24 {
                let mid = 0.5 * (lo + hi);
                if clear(mid) < gap {
                    lo = mid;
                } else {
                    hi = mid;
                }
            }
            // inside the canvas, as far as it allows
            let cx = (c[0] + u[0] * hi).clamp(8.0 + hw, (b.canvas.w - 8.0 - hw).max(8.0 + hw));
            let (x, y) = (cx - hw, c[1] + u[1] * hi - st.size * 0.5);
            if sel {
                // a glow, not a box: a lattice whose alpha is a gaussian of the distance to the
                // text's middle line (flat over the cap height, smooth everywhere: no bar, no edge)
                let (mid, flat, sigma) = (y + st.size * 0.5, st.size * 0.15, d.d(24.0));
                let (x0, x1, reach) = (x + hh, x + w - hh, sigma * 2.6);
                let (gx, gy, gw, gh) = (x0 - reach, mid - reach, x1 - x0 + 2.0 * reach, 2.0 * reach);
                let peak = 0.34 + 0.18 * pulse;
                let mut g = Mesh::default();
                g.shade_rect(gx, gy, gw, gh, (gw / 3.0).ceil() as usize, (gh / 3.0).ceil() as usize, &|q| {
                    let dist = ((x0 - q[0]).max(q[0] - x1).max(0.0)).hypot(q[1] - mid);
                    let e = (dist - flat).max(0.0) / sigma;
                    hex(G_ORANGE, peak * (-e * e).exp() * (1.0 - (dist / reach).powi(4).min(1.0)))
                });
                b.mesh(&g);
            }
            b.font_text(HudFont::Glass, name, x, y, &st);
        }
        Self::needle(b, c, r_out, self.needle, d.px);
    }
}

/// The part of the convex polygon `poly` where `f <= 0` (one Sutherland-Hodgman step).
fn clip_half(poly: &[[f32; 2]], f: &dyn Fn([f32; 2]) -> f32) -> Vec<[f32; 2]> {
    let mut out = Vec::new();
    for i in 0..poly.len() {
        let (a, b) = (poly[i], poly[(i + 1) % poly.len()]);
        let (fa, fb) = (f(a), f(b));
        if fa <= 0.0 {
            out.push(a);
        }
        if (fa < 0.0 && fb > 0.0) || (fa > 0.0 && fb < 0.0) {
            let t = fa / (fa - fb);
            out.push([a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t]);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use motorview::ui_font::GLYPHS;

    /// Every glyph the bindings and tips can show is in the glyph field atlas.
    #[test]
    fn glyphs_in_atlas() {
        let devs = [Device::Keyboard, Device::Xbox, Device::PlayStation];
        let mut want: Vec<char> = devs.iter().flat_map(|&d| (0..18).map(move |k| Game::glyph_for(d, k).0)).collect();
        for d in devs {
            for (_, p) in crate::controls::PAD_NAMES {
                want.push(pad_glyph(p, d).0);
            }
            for c in ["Shift", "Ctrl", "Alt", "MouseButtonLeft", "MouseButtonRight", "MouseButtonMiddle", "MouseLeft", "MouseRight", "MouseUp", "MouseDown", "JoyIn", "JoyButton8"] {
                want.extend(Code::parse(c).and_then(|c| code_glyph(c, d)).map(|g| g.0));
            }
        }
        // every key name of the .clo vocabulary
        for &(_, k) in crate::controls::KEYS {
            want.extend(key_glyph(k));
        }
        want.extend(['\u{243F}', '\u{2441}', '\u{2442}']);
        for c in want {
            assert!(GLYPHS.contains(&c), "U+{:04X} not in ui_font::GLYPHS", c as u32);
        }
    }
}
