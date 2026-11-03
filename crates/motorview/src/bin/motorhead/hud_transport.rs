//! REMASTER playback controls of demo play (#346; mockup B · Telemetry, `reference_materials/playback-overhaul/B-telemetry.html`,
//! style `re-notes/19-glass-hud-design-language.md`): a Glass card low in the free centre column with the transport buttons,
//! the clock and the speed / camera / telemetry chips over a progress bar (laps ticked, the best lap framed); TELEMETRY unfolds
//! the speed trace of the whole recording above the bar, which is the diagram's time axis. Idle the card goes and a thin
//! progress line (folded) or a bare ribbon of the trace (unfolded) stays. Design px of the 1280 x 720 artboard (`Glass::hud`),
//! like `hud_glass`; every path here is REMASTER.

use crate::front::menu_modern::{circle, hex, Glass, G_DIM, G_LINE, G_ORANGE, G_S1, G_TEXT};
use crate::hud::{digits, digits_width};
use crate::playback::{Target, View, HOLD_RATE};
use motor_sim::demo::GO_MS;
use motorview::ui::{text_width, Batch, FontStyle, WHITE};
use motorview::ui_font::{self, HudFont};
use motorview::ui_vector::{self as vec, Ends, Mesh};
use std::f32::consts::TAU;

/// One hint of the row under the card: the glyphs of its keys or buttons and what they do.
pub struct Hint {
    pub caps: Vec<(char, [f32; 4])>,
    pub label: &'static str,
}

/// The card: left, bottom, width, the folded and the unfolded height, the padding (`.bcard`, `.card` border 1).
const CARD_X: f32 = 290.0;
const CARD_BOTTOM: f32 = 686.0;
const CARD_W: f32 = 740.0;
const CARD_H: f32 = 82.0;
/// What the diagram adds when unfolded: its 76 px and the 6 px gap (`.up`).
const UP_H: f32 = 82.0;
const GRAPH_H: f32 = 76.0;
/// The content box: x and width (the card's border and 16 px padding).
const IN_X: f32 = CARD_X + 17.0;
const IN_W: f32 = CARD_W - 34.0;
/// The speed the diagram's top stands for (km/h), or more when the recording goes faster.
const VMAX: f32 = 330.0;
/// The lap labels' and the top mark's top inside the diagram.
const LABEL_Y: f32 = 4.0;

fn clock_text(ms: f64) -> String {
    let r = (ms - GO_MS) / 1000.0;
    let a = r.abs();
    let (m, s) = ((a / 60.0).floor(), a % 60.0);
    format!("{}{}:{:04.1}", if r < -0.05 { "-" } else { "" }, m as u32, s)
}

fn lap_text(ms: f64) -> String {
    let (m, s) = ((ms / 60_000.0).floor(), ms % 60_000.0 / 1000.0);
    format!("{}:{:06.3}", m as u32, s)
}

fn rate_text(r: f32) -> String {
    if r < 1.0 { format!("\u{d7}{r}") } else { format!("\u{d7}{}", r as u32) }
}

/// A line of text with the HUD's 0 1px 2px shadow.
fn shadowed(b: &mut Batch, font: HudFont, s: &str, x: f32, y: f32, st: &FontStyle, off: f32) -> f32 {
    b.font_text(font, s, x, y + off, &FontStyle { color: [0.0, 0.0, 0.0, 0.45 * st.color[3]], ..*st });
    b.font_text(font, s, x, y, st)
}

/// `.tb`: a glass chip, `big` a circle; `rim` orange (the focus), `held` lit orange.
#[allow(clippy::too_many_arguments)]
fn button(m: &mut Mesh, d: Glass, (x, y, s): (f32, f32, f32), round: bool, rim: Option<f32>, held: bool, hover: bool, a: f32) {
    let r = if round { s * 0.5 } else { d.d(10.0) };
    let body = vec::rounded_rect(x, y, s, s, r);
    m.shadow(&body, 0.0, d.d(2.0), d.d(6.0), [0.0, 0.0, 0.0, 0.4 * a]);
    if held {
        m.halo(&body, d.d(16.0), hex(G_ORANGE, 0.6 * a));
        m.fill_with(&body, &|q| {
            let t = ((q[1] - y) / s).clamp(0.0, 1.0);
            [1.0, (0xbe as f32 + (0x9a as f32 - 0xbe as f32) * t) / 255.0, (0x7a as f32 + (0x3c as f32 - 0x7a as f32) * t) / 255.0, a]
        }, d.px);
    } else {
        m.fill(&body, [6.0 / 255.0, 14.0 / 255.0, 28.0 / 255.0, 0.35 * a], d.px);
        m.fill_with(&body, &|q| [1.0, 1.0, 1.0, (0.16 - 0.12 * ((q[1] - y) / s).clamp(0.0, 1.0)) * a], d.px);
    }
    if let Some(p) = rim {
        m.halo(&body, d.d(14.0), hex(G_ORANGE, (0.3 + 0.2 * p) * a));
    }
    let edge = if rim.is_some() || held || hover { hex(G_ORANGE, a) } else if round { hex(G_LINE, 0.6 * a) } else { hex(G_LINE, 0.36 * a) };
    m.stroke(&body, true, d.w(1.0), Ends::Round, edge, d.px);
    m.stroke(&[[x + r, y + d.d(1.5)], [x + s - r, y + d.d(1.5)]], false, d.w(1.0), Ends::Butt, [1.0, 1.0, 1.0, 0.35 * a], d.px);
}

/// Lucide's filled transport shapes (the mockup's `.ico.fill`): the 24 px grid scaled to `size` about (cx, cy), the fill and
/// the 2 px round-joined outline in `col`.
#[derive(Clone, Copy, PartialEq)]
enum Glyph {
    Play,
    Pause,
    Rewind,
    Forward,
    Back,
    Next,
}

fn glyph(m: &mut Mesh, d: Glass, g: Glyph, (cx, cy, size): (f32, f32, f32), col: [f32; 4]) {
    let k = size / 24.0;
    let at = |p: [f32; 2]| [cx + (p[0] - 12.0) * k, cy + (p[1] - 12.0) * k];
    let tri = |m: &mut Mesh, p: [[f32; 2]; 3]| {
        let pts: Vec<[f32; 2]> = p.iter().map(|&q| at(q)).collect();
        m.fill(&pts, col, d.px);
        m.stroke(&pts, true, 2.0 * k, Ends::Round, col, d.px);
    };
    let bar = |m: &mut Mesh, x: f32, y0: f32, y1: f32, w: f32| {
        let pts = vec::rounded_rect(cx + (x - 12.0) * k - w * 0.5 * k, cy + (y0 - 12.0) * k, w * k, (y1 - y0) * k, k);
        m.fill(&pts, col, d.px);
        m.stroke(&pts, true, 2.0 * k, Ends::Round, col, d.px);
    };
    match g {
        Glyph::Play => tri(m, [[6.0, 3.0], [20.0, 12.0], [6.0, 21.0]]),
        Glyph::Pause => {
            bar(m, 16.0, 4.0, 20.0, 4.0);
            bar(m, 8.0, 4.0, 20.0, 4.0);
        }
        Glyph::Rewind => {
            tri(m, [[11.0, 19.0], [2.0, 12.0], [11.0, 5.0]]);
            tri(m, [[22.0, 19.0], [13.0, 12.0], [22.0, 5.0]]);
        }
        Glyph::Forward => {
            tri(m, [[13.0, 19.0], [22.0, 12.0], [13.0, 5.0]]);
            tri(m, [[2.0, 19.0], [11.0, 12.0], [2.0, 5.0]]);
        }
        Glyph::Back => {
            tri(m, [[19.0, 20.0], [9.0, 12.0], [19.0, 4.0]]);
            m.stroke(&[at([5.0, 19.0]), at([5.0, 5.0])], false, 2.0 * k, Ends::Round, col, d.px);
        }
        Glyph::Next => {
            tri(m, [[5.0, 4.0], [15.0, 12.0], [5.0, 20.0]]);
            m.stroke(&[at([19.0, 5.0]), at([19.0, 19.0])], false, 2.0 * k, Ends::Round, col, d.px);
        }
    }
}

/// An outlined Lucide icon from the icon font, `size` design px, its box's left top at (x, y).
fn icon(b: &mut Batch, d: Glass, c: char, (x, y): (f32, f32), size: f32, col: [f32; 4]) {
    let s = d.d(size) * ui_font::face(HudFont::Icons).cap / ui_font::SRC;
    b.font_text(HudFont::Icons, &c.to_string(), x, y + (d.d(size) - s) * 0.5, &FontStyle::new(s, col));
}

/// A chip's width in canvas px (`chip` draws it, `targets` lays out its hit rectangle).
fn chip_width(d: Glass, icon: f32, label: &str, num: bool, pads: (f32, f32)) -> f32 {
    let st = d.font(HudFont::Glass, if num { 13.0 } else { 14.0 }, if num { 0.0 } else { 1.5 }, WHITE);
    d.d(pads.0 + icon + 6.0 + pads.1 + 2.0) + text_width(HudFont::Glass, label, &st) - if num { 0.0 } else { st.tracking }
}

/// `.pc`: a chip; returns its width in canvas px. `lit` = the orange rim of a value that is not the default.
#[allow(clippy::too_many_arguments)]
fn chip(b: &mut Batch, d: Glass, x_right: f32, top: f32, ic: (char, f32), label: &str, num: bool, lit: bool, pads: (f32, f32), a: f32) -> f32 {
    let st = d.font(HudFont::Glass, if num { 13.0 } else { 14.0 }, if num { 0.0 } else { 1.5 }, hex(if lit { G_ORANGE } else { G_TEXT }, a));
    let font = HudFont::Glass;
    let w = chip_width(d, ic.1, label, num, pads);
    let x = x_right - w;
    let h = d.d(26.0);
    let body = vec::rounded_rect(x, top, w, h, h * 0.5);
    let mut m = Mesh::default();
    m.fill(&body, [6.0 / 255.0, 14.0 / 255.0, 28.0 / 255.0, 0.35 * a], d.px);
    m.fill_with(&body, &|q| [1.0, 1.0, 1.0, (0.13 - 0.09 * ((q[1] - top) / h).clamp(0.0, 1.0)) * a], d.px);
    m.stroke(&body, true, d.w(1.0), Ends::Round, if lit { hex(G_ORANGE, 0.8 * a) } else { hex(G_LINE, 0.36 * a) }, d.px);
    m.stroke(&[[x + h * 0.5, top + d.d(1.5)], [x + w - h * 0.5, top + d.d(1.5)]], false, d.w(1.0), Ends::Butt, [1.0, 1.0, 1.0, 0.3 * a], d.px);
    b.mesh(&m);
    let ix = x + d.d(1.0 + pads.0);
    icon(b, d, ic.0, (ix, top + (h - d.d(ic.1)) * 0.5), ic.1, st.color);
    let tx = ix + d.d(ic.1 + 6.0);
    b.font_text(font, label, tx, d.cap_top(font, if num { 13.0 } else { 14.0 }, top, Some(26.0)) + 0.0, &st);
    w
}

/// The progress bar and, over it, the diagram: file ms to x.
struct Axis {
    x: f32,
    w: f32,
    len: f64,
}

impl Axis {
    fn at(&self, ms: f64) -> f32 {
        self.x + self.w * (ms / self.len).clamp(0.0, 1.0) as f32
    }
}

/// The hatch of the countdown before the green light (`repeating-linear-gradient(135deg ...)`): 45-degree strokes 6 px apart.
fn hatch(m: &mut Mesh, d: Glass, (x, y, w, h): (f32, f32, f32, f32), a: f32) {
    let step = d.d(6.0) * std::f32::consts::SQRT_2;
    let mut k = -h;
    while k < w {
        let (x0, x1) = ((x + k).max(x), (x + k + h).min(x + w));
        if x1 > x0 {
            let (y0, y1) = (y + h - (x0 - (x + k)), y + h - (x1 - (x + k)));
            m.stroke(&[[x0, y0], [x1, y1]], false, d.d(1.5), Ends::Butt, hex(G_LINE, 0.12 * a), d.px);
        }
        k += step;
    }
}

/// The recording's speed at file ms `ms` from the trace.
fn speed_at(v: &View, ms: f64) -> f32 {
    if v.trace.is_empty() {
        return 0.0;
    }
    let f = (ms / v.len_ms).clamp(0.0, 1.0) as f32 * (v.trace.len() - 1) as f32;
    let (i, t) = (f.floor() as usize, f.fract());
    v.trace[i] + (v.trace[(i + 1).min(v.trace.len() - 1)] - v.trace[i]) * t
}

/// The card's sizes: `d` shrunk on a canvas narrower than 16:9 (it stays clear of the splits and the map), the card's
/// rectangle (it grows upwards with the unfold), the top of the control row and its middle line.
struct Geo {
    d: Glass,
    tele: f32,
    card: (f32, f32, f32, f32),
    row_top: f32,
    mid: f32,
    axis: Axis,
}

fn geo(d0: Glass, v: &View) -> Geo {
    let d = Glass { s: d0.s * (d0.cw / (1280.0 * d0.s)).min(1.0), ..d0 };
    let tele = v.unfold.clamp(0.0, 1.0);
    let card_h = CARD_H + UP_H * tele;
    let card = (d.x(CARD_X), d.b(CARD_BOTTOM - card_h), d.d(CARD_W), d.d(card_h));
    let row_top = card.1 + d.d(11.0);
    Geo { d, tele, card, row_top, mid: row_top + d.d(17.0), axis: Axis { x: d.x(IN_X), w: d.d(IN_W), len: v.len_ms.max(1.0) } }
}

/// The transport buttons' sizes (design px), left to right: previous lap, rewind, play, forward, next lap.
const BTN: [f32; 5] = [32.0, 32.0, 40.0, 32.0, 32.0];
/// Gap between buttons.
const BTN_GAP: f32 = 8.0;

/// What a click can reach, as canvas-px rectangles `[x, y, w, h]` (the pointer's coordinates): the end choice's pills
/// whenever the recording has ended, and, while the card shows, its buttons, chips, the diagram (unfolded) and the bar.
pub fn targets(d0: Glass, v: &View, cam: &str) -> Vec<(Target, [f32; 4])> {
    let mut out = Vec::new();
    if v.ended {
        let w = pill_widths(d0);
        let mut x = d0.x(640.0) - (w[0] + w[1] + d0.d(PILL_GAP)) * 0.5;
        for (i, w) in w.into_iter().enumerate() {
            out.push((Target::Pick(i), [x, d0.y(PILL_TOP), w, d0.d(PILL_H)]));
            x += w + d0.d(PILL_GAP);
        }
    }
    if v.show <= 0.5 {
        return out;
    }
    let g = geo(d0, v);
    let d = g.d;
    let mut x = d.x(IN_X);
    for (t, size) in [Target::Back, Target::Rew, Target::Play, Target::Fwd, Target::Next].into_iter().zip(BTN) {
        let s = d.d(size);
        out.push((t, [x, g.mid - s * 0.5, s, s]));
        x += d.d(size + BTN_GAP);
    }
    let (top, h) = (g.mid - d.d(13.0), d.d(26.0));
    let mut right = d.x(IN_X) + d.d(IN_W);
    for (t, w) in [
        (Target::Tele, chip_width(d, 16.0, "TELEMETRY", false, (10.0, 12.0))),
        (Target::Camera, chip_width(d, 18.0, cam, false, (11.0, 11.0))),
        (Target::Speed, chip_width(d, 16.0, &rate_text(v.rate), true, (11.0, 11.0))),
    ] {
        out.push((t, [right - w, top, w, h]));
        right -= w + d.d(8.0);
    }
    if g.tele > 0.5 {
        out.push((Target::Graph, [g.axis.x, g.row_top + d.d(34.0 + 6.0), g.axis.w, d.d(GRAPH_H)]));
    }
    // the bar's box is 20 design px; a little more to grab
    out.push((Target::Bar, [g.axis.x, d.b(CARD_BOTTOM - 33.0) - d.d(2.0), g.axis.w, d.d(24.0)]));
    out
}

/// The end choice's pills: widths, top, height and gap (design px).
const PILL_TOP: f32 = 196.0;
const PILL_H: f32 = 40.0;
const PILL_GAP: f32 = 16.0;
const PILL_LABELS: [&str; 2] = ["WATCH AGAIN", "BACK TO THE LIST"];

fn pill_widths(d: Glass) -> [f32; 2] {
    let st = d.font(HudFont::Glass, 17.0, 2.0, WHITE);
    PILL_LABELS.map(|l| d.d(22.0 * 2.0 + 20.0 + 9.0) + text_width(HudFont::Glass, l, &st) - st.tracking)
}

/// The controls: the card (or the idle ribbon), the held / paused mark over the picture, the end choice and the hint row.
/// `cam` names the camera, `hints` the row under the card, `clock` drives the focus pulse.
pub fn transport(b: &mut Batch, d0: Glass, v: &View, cam: &str, hints: &[Hint], clock: f32) {
    // a canvas narrower than 16:9 shrinks the card about its bottom centre, so it stays clear of the splits and the map
    let Geo { d, tele, card: (cx0, cy0, cw, ch), row_top, mid, axis } = geo(d0, v);
    let pulse = 0.5 - 0.5 * (clock * TAU / 1.1).cos();
    let a = v.show.clamp(0.0, 1.0);
    let ribbon = 1.0 - a;
    let (t, hot) = (v.t_ms, v.hold != 0 || v.scrub);
    if a > 0.01 {
        // the card
        let body = vec::rounded_rect(cx0, cy0, cw, ch, d.d(12.0));
        let mut m = Mesh::default();
        m.shadow(&body, 0.0, d.d(10.0), d.d(26.0), [0.0, 0.0, 0.0, 0.45 * a]);
        m.fill(&body, [6.0 / 255.0, 14.0 / 255.0, 28.0 / 255.0, 0.42 * a], d.px);
        m.fill_with(&body, &|q| [1.0, 1.0, 1.0, (0.11 - 0.08 * ((q[1] - cy0) / ch).clamp(0.0, 1.0)) * a], d.px);
        m.stroke(&body, true, d.w(1.0), Ends::Round, hex(G_LINE, 0.36 * a), d.px);
        m.stroke(&[[cx0 + d.d(12.0), cy0 + d.d(1.5)], [cx0 + cw - d.d(12.0), cy0 + d.d(1.5)]], false, d.w(1.0), Ends::Butt, [1.0, 1.0, 1.0, 0.35 * a], d.px);
        b.mesh(&m);
        // the control row: buttons, clock, chips
        let white = [1.0, 1.0, 1.0, a];
        let text = hex(G_TEXT, a);
        let mut m = Mesh::default();
        let mut x = d.x(IN_X);
        let focus = !v.playing && !v.ended;
        let step = |s: f32| d.d(s + BTN_GAP);
        let play = if v.ended { Glyph::Play } else if v.playing { Glyph::Pause } else { Glyph::Play };
        for (((g, held, big), size), target) in [(Glyph::Back, false, false), (Glyph::Rewind, v.hold < 0, false), (play, false, true), (Glyph::Forward, v.hold > 0, false), (Glyph::Next, false, false)]
            .into_iter()
            .zip(BTN)
            .zip([Target::Back, Target::Rew, Target::Play, Target::Fwd, Target::Next])
        {
            let s = d.d(size);
            let hover = v.hover == Some(target);
            button(&mut m, d, (x, mid - s * 0.5, s), big, (big && focus || big && v.ended).then_some(pulse), held, hover, a);
            let col = if held { [26.0 / 255.0, 13.0 / 255.0, 0.0, a] } else if hover || big && (focus || v.ended) { hex(G_ORANGE, a) } else if big { white } else { text };
            if big && v.ended {
                // the play button turned into replay: the arrow of the icon font below
            } else {
                glyph(&mut m, d, g, (x + s * 0.5, mid, d.d(if big { 22.0 } else { 17.0 })), col);
            }
            x += step(size);
        }
        b.mesh(&m);
        if v.ended {
            let s = d.d(40.0);
            let bx = d.x(IN_X) + step(BTN[0]) + step(BTN[1]);
            icon(b, d, ui_font::icon::RESTART, (bx + (s - d.d(22.0)) * 0.5, mid - d.d(11.0)), 22.0, hex(G_ORANGE, a));
        }
        // chips, right to left: TELEMETRY, the camera, the speed
        let mut right = d.x(IN_X) + d.d(IN_W);
        let top = mid - d.d(13.0);
        let w = chip(b, d, right, top, (if v.tele { ui_font::icon::CHEVRON_DOWN } else { ui_font::icon::CHEVRON_UP }, 16.0), "TELEMETRY", false, v.tele || v.hover == Some(Target::Tele), (10.0, 12.0), a);
        right -= w + d.d(8.0);
        let w = chip(b, d, right, top, (ui_font::icon::CAMERA, 18.0), cam, false, v.hover == Some(Target::Camera), (11.0, 11.0), a);
        right -= w + d.d(8.0);
        let w = chip(b, d, right, top, (ui_font::icon::GAUGE, 16.0), &rate_text(v.rate), true, (v.rate - 1.0).abs() > 1e-3 || v.hover == Some(Target::Speed), (11.0, 11.0), a);
        let chips_left = right - w;
        // the clock: m:ss.s in fixed cells, then "/ total"
        let clk = d.font(HudFont::Clock, 22.0, 0.0, [1.0, 1.0, 1.0, a]);
        let cx = x + d.d(14.0);
        let cy = d.cap_top(HudFont::Clock, 22.0, mid - d.d(15.0), Some(30.0));
        let now = clock_text(t.max(0.0));
        digits(b, HudFont::Clock, &now, cx, cy + d.d(1.0), &FontStyle { color: [0.0, 0.0, 0.0, 0.45 * a], ..clk });
        let adv = digits(b, HudFont::Clock, &now, cx, cy, &clk);
        let of = d.font(HudFont::Clock, 15.0, 0.0, text);
        let of_y = cy + (d.d(22.0) - d.d(15.0)) * 0.55;
        // "/ total" goes when a long recording would run it into the chips
        let total = format!("/ {}", clock_text(v.len_ms));
        if cx + adv + d.d(8.0) + b.font_width(HudFont::Clock, &total, &of) < chips_left - d.d(10.0) {
            shadowed(b, HudFont::Clock, &total, cx + adv + d.d(8.0), of_y, &of, d.d(1.0));
        }
        // the diagram, over the bar
        if tele > 0.02 {
            graph(b, d, v, &axis, (d.x(IN_X), row_top + d.d(34.0 + 6.0), d.d(IN_W), d.d(GRAPH_H)), a * tele, hot);
        }
        // the progress bar at the card's foot
        let bar_y = d.b(CARD_BOTTOM - 33.0);
        bar(b, d, v, &axis, bar_y, a, hot);
        if !hints.is_empty() {
            hint_row(b, d, hints, a);
        }
        // the bubble over the bar: the time, the lap and the speed under the pointer (or at the dragged-to moment)
        if let Some(ms) = v.tip {
            tip(b, d, v, &axis, cy0, ms, a);
        }
    }
    // idle: the thin line (folded) or the bare ribbon of the trace (unfolded) stays
    if ribbon > 0.01 {
        idle(b, d, v, &axis, ribbon, tele, hot);
    }
    // over the picture: the held control, PAUSED, the end choice
    over(b, d0, v);
    if let (Some(p), true) = (v.pointer, v.show > 0.01) {
        let mut m = Mesh::default();
        pointer(&mut m, d0, p, v.show.clamp(0.0, 1.0));
        b.mesh(&m);
    }
}

/// The Glass pointer (REMASTER demo play, #352): an arrow with its tip at `p` (canvas px), the OS arrow being hidden.
fn pointer(m: &mut Mesh, d: Glass, p: [f32; 2], a: f32) {
    let k = d.d(1.0);
    let pts: Vec<[f32; 2]> = [[0.0, 0.0], [0.0, 17.0], [4.2, 13.2], [7.2, 19.6], [9.8, 18.4], [6.9, 12.2], [12.0, 12.2]].iter().map(|q| [p[0] + q[0] * k, p[1] + q[1] * k]).collect();
    m.shadow(&pts, 0.0, d.d(1.5), d.d(3.0), [0.0, 0.0, 0.0, 0.5 * a]);
    m.fill(&pts, [244.0 / 255.0, 251.0 / 255.0, 1.0, a], d.px);
    m.stroke(&pts, true, d.w(1.2), Ends::Round, [4.0 / 255.0, 7.0 / 255.0, 13.0 / 255.0, 0.9 * a], d.px);
}

/// `.tip`: the bubble 56 design px above the card, centred on file ms `ms`: the clock there, and COUNTDOWN, FINISHED or
/// the lap and the speed of the trace.
fn tip(b: &mut Batch, d: Glass, v: &View, ax: &Axis, card_top: f32, ms: f64, a: f32) {
    let ms = ms.clamp(0.0, v.len_ms);
    let line2 = if ms < GO_MS {
        "COUNTDOWN".to_string()
    } else if v.laps.flag.is_some_and(|f| ms > f) {
        "FINISHED".to_string()
    } else {
        let lap = v.laps.starts.iter().rposition(|&s| s <= ms).unwrap_or(0) + 1;
        format!("LAP {lap} \u{b7} {} KM/H", speed_at(v, ms).round() as u32)
    };
    let s1 = d.font(HudFont::Clock, 15.0, 0.0, [1.0, 1.0, 1.0, a]);
    let s2 = d.font(HudFont::Glass, 11.0, 1.2, hex(G_DIM, a));
    let now = clock_text(ms);
    let w = d.d(22.0) + digits_width(b, HudFont::Clock, &now, &s1).max(b.font_width(HudFont::Glass, &line2, &s2) - s2.tracking);
    let h = d.d(1.0 + 5.0 + 18.0 + 13.0 + 6.0 + 1.0);
    let x = (ax.at(ms) - w * 0.5).clamp(d.d(4.0), (d.cw - w - d.d(4.0)).max(d.d(4.0)));
    let y = card_top - d.d(56.0);
    let body = vec::rounded_rect(x, y, w, h, d.d(9.0));
    let mut m = Mesh::default();
    m.shadow(&body, 0.0, d.d(8.0), d.d(18.0), [0.0, 0.0, 0.0, 0.45 * a]);
    m.fill(&body, [6.0 / 255.0, 14.0 / 255.0, 28.0 / 255.0, 0.6 * a], d.px);
    m.fill_with(&body, &|q| [1.0, 1.0, 1.0, (0.18 - 0.13 * ((q[1] - y) / h).clamp(0.0, 1.0)) * a], d.px);
    m.stroke(&body, true, d.w(1.0), Ends::Round, [190.0 / 255.0, 230.0 / 255.0, 1.0, 0.45 * a], d.px);
    b.mesh(&m);
    let tw = digits_width(b, HudFont::Clock, &now, &s1);
    digits(b, HudFont::Clock, &now, x + (w - tw) * 0.5, d.cap_top(HudFont::Clock, 15.0, y + d.d(6.0), Some(18.0)), &s1);
    let lw = b.font_width(HudFont::Glass, &line2, &s2) - s2.tracking;
    b.font_text(HudFont::Glass, &line2, x + (w - lw) * 0.5, d.cap_top(HudFont::Glass, 11.0, y + d.d(24.0), Some(13.0)), &s2);
}

/// `.bar`: the track with the countdown hatch, the best lap framed, the fill to the playhead, the lap ticks and the knob.
fn bar(b: &mut Batch, d: Glass, v: &View, ax: &Axis, y: f32, a: f32, hot: bool) {
    let mut m = Mesh::default();
    let (x0, w) = (ax.x, ax.w);
    let (ty, th) = (y + d.d(7.0), d.d(6.0));
    m.fill(&vec::rounded_rect(x0, ty, w, th, th * 0.5), hex(G_LINE, 0.2 * a), d.px);
    let pre = ax.at(GO_MS) - x0;
    hatch(&mut m, d, (x0, ty, pre, th), a);
    if let Some(k) = v.laps.best {
        let (s, e) = v.laps.span(k, v.len_ms);
        let (bx, bw) = (ax.at(s) + d.d(2.0), ax.at(e) - ax.at(s) - d.d(4.0));
        if bw > 0.0 {
            let r = vec::rounded_rect(bx, y + d.d(5.0), bw, d.d(10.0), d.d(3.0));
            m.fill(&r, hex(G_ORANGE, 0.12 * a), d.px);
            m.stroke(&r, true, d.w(1.0), Ends::Round, hex(G_ORANGE, 0.7 * a), d.px);
        }
    }
    let px = ax.at(v.t_ms);
    if px > x0 + 0.5 {
        let fill = vec::rounded_rect(x0, ty, px - x0, th, th * 0.5);
        m.halo(&fill, d.d(8.0), hex(0x4aa8ff, 0.5 * a));
        m.fill(&fill, hex(G_S1, a), d.px);
    }
    let marks = v.laps.starts.iter().copied().chain(v.laps.flag);
    let last = v.laps.flag.is_some();
    let n = v.laps.starts.len() + last as usize;
    for (i, ms) in marks.enumerate() {
        let col = if last && i + 1 == n { [1.0, 1.0, 1.0, a] } else { [230.0 / 255.0, 246.0 / 255.0, 1.0, 0.75 * a] };
        m.fill(&vec::rounded_rect(ax.at(ms) - d.d(1.0), y + d.d(3.0), d.d(2.0), d.d(14.0), d.d(1.0)), col, d.px);
    }
    knob(&mut m, d, [px, y + d.d(10.0)], 7.0, a, hot);
    b.mesh(&m);
}

/// The playhead's knob or dot: a `r` design px disc with the dark ring and the cyan (orange when `hot`) glow.
fn knob(m: &mut Mesh, d: Glass, c: [f32; 2], r: f32, a: f32, hot: bool) {
    let col = if hot { hex(G_ORANGE, a) } else { [244.0 / 255.0, 251.0 / 255.0, 1.0, a] };
    let glow = if hot { hex(G_ORANGE, 0.9 * a) } else { [111.0 / 255.0, 203.0 / 255.0, 1.0, 0.9 * a] };
    m.halo(&circle(c, d.d(r + 2.0)), d.d(10.0), [glow[0], glow[1], glow[2], 0.45 * a]);
    m.fill(&circle(c, d.d(r + 2.0)), [4.0 / 255.0, 7.0 / 255.0, 13.0 / 255.0, 0.8 * a], d.px);
    m.fill(&circle(c, d.d(r)), col, d.px);
}

/// The speed trace over `rect` (canvas px): the area and line, the part up to the playhead lit, the lap bands, labels and
/// the top-speed mark. `H`-high diagram of the mockup's `bands` and `traceSVG`.
fn graph(b: &mut Batch, d: Glass, v: &View, ax: &Axis, rect: (f32, f32, f32, f32), a: f32, hot: bool) {
    let (gx, gy, gw, gh) = rect;
    let vmax = v.trace.iter().copied().fold(VMAX, f32::max).max(VMAX);
    let y_of = |s: f32| gy + gh - d.d(2.0) - s / vmax * gh * 0.74;
    let mut m = Mesh::default();
    // the bands: the best lap tinted with its top edge, the countdown hatched, a divider at every lap start and the flag
    if let Some(k) = v.laps.best {
        let (s, e) = v.laps.span(k, v.len_ms);
        let r = vec::rounded_rect(ax.at(s), gy, ax.at(e) - ax.at(s), gh, 0.0);
        m.fill_with(&r, &|q| [1.0, 154.0 / 255.0, 60.0 / 255.0, (0.20 - 0.15 * ((q[1] - gy) / gh).clamp(0.0, 1.0)) * a], d.px);
        m.stroke(&[[ax.at(s), gy + d.w(1.0) * 0.5], [ax.at(e), gy + d.w(1.0) * 0.5]], false, d.w(1.0), Ends::Butt, hex(G_ORANGE, 0.7 * a), d.px);
    }
    hatch(&mut m, d, (gx, gy, ax.at(GO_MS) - gx, gh), a * 0.75);
    let marks: Vec<f64> = v.laps.starts.iter().copied().chain(v.laps.flag).collect();
    for (i, &ms) in marks.iter().enumerate() {
        let end = v.laps.flag.is_some() && i + 1 == marks.len();
        m.fill(&vec::rounded_rect(ax.at(ms), gy, d.w(1.0), gh, 0.0), if end { [1.0, 1.0, 1.0, 0.7 * a] } else { hex(G_LINE, 0.5 * a) }, d.px);
    }
    // the trace: a point per `TRACE_POINTS` step across the file
    let n = v.trace.len();
    if n > 1 {
        let pts: Vec<[f32; 2]> = (0..n).map(|i| [gx + gw * i as f32 / (n - 1) as f32, y_of(v.trace[i])]).collect();
        let head = ((v.t_ms / v.len_ms).clamp(0.0, 1.0) as f32 * gw).min(gw);
        let split = |lit: bool| -> Vec<[f32; 2]> {
            let mut out: Vec<[f32; 2]> = pts.iter().copied().filter(|p| (p[0] - gx <= head) == lit).collect();
            if lit && head > 0.0 {
                out.push([gx + head, y_of(speed_at(v, v.t_ms))]);
            }
            out
        };
        let area = |pts: &[[f32; 2]], top: f32, bottom: f32, m: &mut Mesh| {
            if pts.len() < 2 {
                return;
            }
            let mut poly = pts.to_vec();
            poly.push([pts[pts.len() - 1][0], gy + gh]);
            poly.push([pts[0][0], gy + gh]);
            m.fill_with(&poly, &|q| [143.0 / 255.0, 208.0 / 255.0, 1.0, (top + (bottom - top) * ((q[1] - gy) / gh).clamp(0.0, 1.0)) * a], d.px);
        };
        let dim = split(false);
        let lit = split(true);
        area(&dim, 0.26, 0.03, &mut m);
        if dim.len() > 1 {
            m.stroke(&dim, false, d.w(1.2), Ends::Round, [190.0 / 255.0, 230.0 / 255.0, 1.0, 0.5 * a], d.px);
        }
        area(&lit, 0.62, 0.08, &mut m);
        if lit.len() > 1 {
            m.stroke(&lit, false, d.w(4.5), Ends::Round, [111.0 / 255.0, 203.0 / 255.0, 1.0, 0.22 * a], d.px);
            m.stroke(&lit, false, d.w(1.8), Ends::Round, [230.0 / 255.0, 246.0 / 255.0, 1.0, a], d.px);
        }
        // the playhead's line and the dot on the curve
        let px = gx + head;
        let col = if hot { hex(G_ORANGE, a) } else { [244.0 / 255.0, 251.0 / 255.0, 1.0, a] };
        m.fill(&vec::rounded_rect(px - d.d(1.0), gy, d.d(2.0), gh + d.d(6.0), 0.0), col, d.px);
        knob(&mut m, d, [px, y_of(speed_at(v, v.t_ms))], 4.0, a, hot);
    }
    b.mesh(&m);
    // labels: LAP n · m:ss.mmm over each lap (the best one orange with its BEST tag), the top speed
    let lab = |c: u32| d.font(HudFont::Glass, 10.0, 1.4, hex(c, a));
    for (k, &s) in v.laps.starts.iter().enumerate() {
        let (s0, e0) = v.laps.span(k, v.len_ms);
        if v.laps.flag.is_none() && k + 1 == v.laps.starts.len() {
            break;
        }
        let best = v.laps.best == Some(k);
        let st = lab(if best { G_ORANGE } else { G_TEXT });
        let tag = d.font(HudFont::Glass, 9.0, 1.0, hex(G_ORANGE, a));
        let tag_w = b.font_width(HudFont::Glass, "BEST", &tag) + d.d(10.0 + 6.0);
        // the label as long as the lap is wide: "LAP 2 \u{b7} 1:04.871" and the BEST tag, else "LAP 2", else "2"
        let room = ax.at(e0) - ax.at(s0) - d.d(12.0);
        let full = format!("LAP {} \u{b7} {}", k + 1, lap_text(e0 - s0));
        let width = |t: &str| b.font_width(HudFont::Glass, t, &st) - st.tracking;
        let (txt, with_tag) = if width(&full) + if best { tag_w } else { 0.0 } <= room {
            (full, best)
        } else if width(&full) <= room {
            (full, false)
        } else if width(&format!("LAP {}", k + 1)) <= room {
            (format!("LAP {}", k + 1), false)
        } else if width(&(k + 1).to_string()) <= room {
            ((k + 1).to_string(), false)
        } else {
            continue;
        };
        let x = ax.at(s) + d.d(6.0);
        let y = d.cap_top(HudFont::Glass, 10.0, gy + d.d(LABEL_Y), None);
        shadowed(b, HudFont::Glass, &txt, x, y, &st, d.d(1.0));
        if with_tag {
            let tw = b.font_width(HudFont::Glass, &txt, &st);
            let bw = tag_w - d.d(6.0);
            let bx = x + tw + d.d(6.0);
            let mut m = Mesh::default();
            let r = vec::rounded_rect(bx, y - d.d(3.0), bw, d.d(14.0), d.d(5.0));
            m.fill(&r, [10.0 / 255.0, 22.0 / 255.0, 38.0 / 255.0, 0.9 * a], d.px);
            m.stroke(&r, true, d.w(1.0), Ends::Round, hex(G_ORANGE, 0.6 * a), d.px);
            b.mesh(&m);
            b.font_text(HudFont::Glass, "BEST", bx + d.d(5.0), d.cap_top(HudFont::Glass, 9.0, y - d.d(3.0), Some(14.0)), &tag);
        }
    }
    if let Some((kmh, at)) = v.laps.top {
        let st = d.font(HudFont::Glass, 10.0, 1.2, [1.0, 1.0, 1.0, a]);
        let txt = format!("TOP {kmh}");
        let tw = b.font_width(HudFont::Glass, &txt, &st) - st.tracking;
        let ty = (y_of(speed_at(v, at)) - d.d(19.0)).max(gy + d.d(15.0));
        let (cx, tri) = (ax.at(at), d.d(3.5));
        let text_top = d.cap_top(HudFont::Glass, 10.0, ty, None);
        // the mark is a small triangle pointing down at the peak, left of the figure
        let mut m = Mesh::default();
        let mid = text_top + st.size * 0.5;
        let x0 = cx - (tw + tri * 2.0 + d.d(4.0)) * 0.5;
        m.fill(&[[x0, mid - tri], [x0 + tri * 2.0, mid - tri], [x0 + tri, mid + tri]], [1.0, 1.0, 1.0, a], d.px);
        b.mesh(&m);
        shadowed(b, HudFont::Glass, &txt, x0 + tri * 2.0 + d.d(4.0), text_top, &st, d.d(1.0));
    }
}

/// `.ribbon`: the card gone, what stays of the bar.
fn idle(b: &mut Batch, d: Glass, v: &View, ax: &Axis, a: f32, tele: f32, hot: bool) {
    let bottom = d.b(CARD_BOTTOM - 2.0);
    let mut m = Mesh::default();
    // the dark bed behind it keeps it readable on a bright road
    let h = d.d(34.0 * tele + 6.0);
    m.soft_pad(&vec::rounded_rect(ax.x - d.d(20.0), bottom - h - d.d(22.0 * tele + 6.0), ax.w + d.d(40.0), h + d.d(22.0 * tele + 18.0), d.d(30.0)), d.d(24.0), [3.0 / 255.0, 8.0 / 255.0, 16.0 / 255.0, 0.5 * a]);
    // the progress line
    let la = a * (1.0 - tele);
    if la > 0.01 {
        let ly = bottom - d.d(3.0);
        m.fill(&vec::rounded_rect(ax.x, ly - d.d(1.5), ax.w, d.d(3.0), d.d(1.5)), hex(G_LINE, 0.4 * la), d.px);
        let px = ax.at(v.t_ms);
        if px > ax.x {
            let fill = vec::rounded_rect(ax.x, ly - d.d(1.5), px - ax.x, d.d(3.0), d.d(1.5));
            m.halo(&fill, d.d(8.0), hex(0x4aa8ff, 0.5 * la));
            m.fill(&fill, hex(G_S1, la), d.px);
        }
        for ms in v.laps.starts.iter().copied().chain(v.laps.flag) {
            m.fill(&vec::rounded_rect(ax.at(ms) - d.d(1.0), ly - d.d(6.0), d.d(2.0), d.d(12.0), 0.0), [230.0 / 255.0, 246.0 / 255.0, 1.0, 0.85 * la], d.px);
        }
        knob(&mut m, d, [px, ly], 4.0, la, hot);
    }
    b.mesh(&m);
    // the ribbon of the trace
    let ta = a * tele;
    if ta > 0.01 && v.trace.len() > 1 {
        let (gx, gy, gw, gh) = (ax.x, bottom - d.d(34.0), ax.w, d.d(34.0));
        let vmax = v.trace.iter().copied().fold(VMAX, f32::max);
        let y_of = |s: f32| gy + gh - d.d(2.0) - s / vmax * gh * 0.74;
        let n = v.trace.len();
        let pts: Vec<[f32; 2]> = (0..n).map(|i| [gx + gw * i as f32 / (n - 1) as f32, y_of(v.trace[i])]).collect();
        let head = ((v.t_ms / v.len_ms).clamp(0.0, 1.0) as f32 * gw).min(gw);
        let mut m = Mesh::default();
        let part = |lit: bool| -> Vec<[f32; 2]> {
            let mut out: Vec<[f32; 2]> = pts.iter().copied().filter(|p| (p[0] - gx <= head) == lit).collect();
            if lit && head > 0.0 {
                out.push([gx + head, y_of(speed_at(v, v.t_ms))]);
            }
            out
        };
        for (lit, (top, bot)) in [(false, (0.26, 0.03)), (true, (0.62, 0.08))] {
            let p = part(lit);
            if p.len() > 1 {
                let mut poly = p.clone();
                poly.push([p[p.len() - 1][0], gy + gh]);
                poly.push([p[0][0], gy + gh]);
                m.fill_with(&poly, &|q| [143.0 / 255.0, 208.0 / 255.0, 1.0, (top + (bot - top) * ((q[1] - gy) / gh).clamp(0.0, 1.0)) * ta], d.px);
                m.stroke(&p, false, d.w(if lit { 1.8 } else { 1.2 }), Ends::Round, if lit { [230.0 / 255.0, 246.0 / 255.0, 1.0, ta] } else { [190.0 / 255.0, 230.0 / 255.0, 1.0, 0.5 * ta] }, d.px);
            }
        }
        for ms in v.laps.starts.iter().copied().chain(v.laps.flag) {
            m.fill(&vec::rounded_rect(ax.at(ms), gy, d.w(1.0), gh, 0.0), hex(G_LINE, 0.4 * ta), d.px);
        }
        m.fill(&vec::rounded_rect(gx + head - d.d(1.0), gy - d.d(2.0), d.d(2.0), gh + d.d(4.0), 0.0), [244.0 / 255.0, 251.0 / 255.0, 1.0, ta], d.px);
        b.mesh(&m);
        let st = d.font(HudFont::Clock, 14.0, 0.0, [1.0, 1.0, 1.0, ta]);
        let dim = d.font(HudFont::Clock, 14.0, 0.0, hex(G_DIM, ta));
        let ty = d.cap_top(HudFont::Clock, 14.0, gy - d.d(22.0), None);
        digits(b, HudFont::Clock, &clock_text(v.t_ms.max(0.0)), gx, ty, &st);
        let total = clock_text(v.len_ms);
        b.font_text(HudFont::Clock, &total, gx + gw - b.font_width(HudFont::Clock, &total, &dim), ty, &dim);
    }
}

/// Over the picture: the held control's big mark and speed, the PAUSED mark, and the end choice with its notice's place left to the stack.
fn over(b: &mut Batch, d: Glass, v: &View) {
    let cx = d.x(640.0);
    if v.ended {
        let labels = PILL_LABELS;
        let st = |on: bool| d.font(HudFont::Glass, 17.0, 2.0, hex(if on { G_ORANGE } else { G_TEXT }, 1.0));
        let icons = [ui_font::icon::RESTART, ui_font::icon::MUSIC];
        let w = pill_widths(d);
        let gap = d.d(PILL_GAP);
        let mut x = cx - (w[0] + w[1] + gap) * 0.5;
        let top = d.y(PILL_TOP);
        for i in 0..2 {
            let on = v.picked == i;
            let h = d.d(PILL_H);
            let r = vec::rounded_rect(x, top, w[i], h, h * 0.5);
            let mut m = Mesh::default();
            m.shadow(&r, 0.0, d.d(10.0), d.d(26.0), [0.0, 0.0, 0.0, 0.45]);
            if on {
                m.halo(&r, d.d(14.0), hex(G_ORANGE, 0.4));
            }
            m.fill(&r, [6.0 / 255.0, 14.0 / 255.0, 28.0 / 255.0, 0.5], d.px);
            m.fill_with(&r, &|q| [1.0, 1.0, 1.0, 0.16 - 0.11 * ((q[1] - top) / h).clamp(0.0, 1.0)], d.px);
            m.stroke(&r, true, d.w(1.0), Ends::Round, if on { hex(G_ORANGE, 1.0) } else { hex(G_LINE, 0.4) }, d.px);
            b.mesh(&m);
            let col = hex(if on { G_ORANGE } else { G_TEXT }, 1.0);
            icon(b, d, icons[i], (x + d.d(22.0), top + (h - d.d(20.0)) * 0.5), 20.0, col);
            b.font_text(HudFont::Glass, labels[i], x + d.d(22.0 + 20.0 + 9.0), d.cap_top(HudFont::Glass, 17.0, top, Some(40.0)), &st(on));
            x += w[i] + gap;
        }
        return;
    }
    let (mark, text) = if v.hold != 0 {
        (Some(if v.hold < 0 { Glyph::Rewind } else { Glyph::Forward }), format!("\u{d7}{}", HOLD_RATE as u32))
    } else if !v.playing && !v.scrub {
        (Some(Glyph::Pause), "PAUSED".to_string())
    } else {
        (None, String::new())
    };
    if let Some(g) = mark {
        let paused = v.hold == 0;
        let (size, top) = if paused { (52.0, 318.0) } else { (64.0, 300.0) };
        let col = [244.0 / 255.0, 251.0 / 255.0, 1.0, 1.0];
        let mut m = Mesh::default();
        let c = [cx, d.y(top + size * 0.5)];
        m.soft_pad(&vec::rounded_rect(c[0] - d.d(size * 0.8), c[1] - d.d(size * 0.8), d.d(size * 1.6), d.d(size * 1.6), d.d(size * 0.6)), d.d(22.0), [120.0 / 255.0, 200.0 / 255.0, 1.0, 0.18]);
        glyph(&mut m, d, g, (c[0], c[1], d.d(size)), col);
        b.mesh(&m);
        let st = d.font(HudFont::Glass, 22.0, 4.0, col);
        let tw = b.font_width(HudFont::Glass, &text, &st) - st.tracking;
        shadowed(b, HudFont::Glass, &text, cx - tw * 0.5, d.cap_top(HudFont::Glass, 22.0, d.y(top + size + 4.0), None), &st, d.d(2.0));
    }
}

/// The hint row under the card (`.pbhints`: 14 px, 22 px apart), the glyphs of the keys or buttons in use.
fn hint_row(b: &mut Batch, d: Glass, hints: &[Hint], a: f32) {
    let st = d.font(HudFont::Glass, 14.0, 1.0, hex(0xa9d3e6, a));
    let ty = d.cap_top(HudFont::Glass, 14.0, d.b(696.0), None);
    let gsize = d.d(11.0);
    let glyph_w = |b: &Batch, c: char| b.font_width(HudFont::Glyphs, &c.to_string(), &FontStyle::new(gsize, WHITE));
    let widths: Vec<f32> = hints
        .iter()
        .map(|h| h.caps.iter().map(|&(c, _)| glyph_w(b, c) + d.d(2.0)).sum::<f32>() - d.d(2.0) + d.d(6.0) + b.font_width(HudFont::Glass, h.label, &st) - st.tracking)
        .collect();
    let gap = d.d(22.0);
    let mut x = d.x(640.0) - (widths.iter().sum::<f32>() + gap * (hints.len().max(1) - 1) as f32) * 0.5;
    for (h, w) in hints.iter().zip(widths) {
        let x0 = x;
        for (n, &(c, tint)) in h.caps.iter().enumerate() {
            if n > 0 {
                x += d.d(2.0);
            }
            let gs = FontStyle::new(gsize, [tint[0], tint[1], tint[2], tint[3] * a]);
            x += b.font_text(HudFont::Glyphs, &c.to_string(), x, ty + st.size * 0.5 - gsize * 0.5, &gs);
        }
        shadowed(b, HudFont::Glass, h.label, x + d.d(6.0), ty, &st, d.d(1.0));
        x = x0 + w + gap;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::playback::Laps;

    fn view<'a>(laps: &'a Laps, trace: &'a [f32]) -> View<'a> {
        View { t_ms: 0.0, len_ms: 200_000.0, laps, playing: true, hold: 0, rate: 1.0, tele: false, trace, show: 1.0, unfold: 0.0, ended: false, picked: 0, hover: None, tip: None, pointer: None, scrub: false }
    }

    fn rect(t: &[(Target, [f32; 4])], k: Target) -> [f32; 4] {
        t.iter().find(|(h, _)| *h == k).unwrap_or_else(|| panic!("{k:?} missing")).1
    }

    /// 1280 x 720 (16:9) and 1280 x 800 (the narrow canvas shrinks the card).
    fn canvases() -> [Glass; 2] {
        [Glass::hud(480.0 * 1280.0 / 720.0, 1.0), Glass::hud(480.0 * 1280.0 / 800.0, 1.0)]
    }

    #[test]
    fn the_buttons_run_left_to_right_inside_the_card_with_the_chips_after_them() {
        let (laps, trace) = (Laps::default(), Vec::<f32>::new());
        for d in canvases() {
            let v = view(&laps, &trace);
            let t = targets(d, &v, "CHASE");
            let g = geo(d, &v);
            let order = [Target::Back, Target::Rew, Target::Play, Target::Fwd, Target::Next];
            for w in order.windows(2) {
                let (a, b) = (rect(&t, w[0]), rect(&t, w[1]));
                assert!(a[0] + a[2] < b[0], "{w:?} {a:?} {b:?}");
            }
            let next = rect(&t, Target::Next);
            let (speed, cam, tele) = (rect(&t, Target::Speed), rect(&t, Target::Camera), rect(&t, Target::Tele));
            assert!(next[0] + next[2] < speed[0] && speed[0] + speed[2] < cam[0] && cam[0] + cam[2] < tele[0]);
            let card_right = g.card.0 + g.card.2;
            assert!(tele[0] + tele[2] < card_right && rect(&t, Target::Back)[0] > g.card.0);
            // every control sits in the row, the bar under it, in the card
            for (h, r) in &t {
                assert!(r[1] >= g.card.1 && r[1] + r[3] <= g.card.1 + g.card.3 + d.d(4.0), "{h:?} {r:?}");
            }
            // the centre of each button maps to itself
            for k in order {
                let r = rect(&t, k);
                let c = [r[0] + r[2] * 0.5, r[1] + r[3] * 0.5];
                let hit = t.iter().find(|(_, q)| c[0] >= q[0] && c[0] < q[0] + q[2] && c[1] >= q[1] && c[1] < q[1] + q[3]).map(|h| h.0);
                assert_eq!(hit, Some(k));
            }
        }
    }

    #[test]
    fn the_bar_spans_the_axis_and_the_diagram_exists_only_unfolded() {
        let (laps, trace) = (Laps::default(), Vec::<f32>::new());
        let d = canvases()[0];
        let mut v = view(&laps, &trace);
        let g = geo(d, &v);
        let bar = rect(&targets(d, &v, "CHASE"), Target::Bar);
        assert_eq!((bar[0], bar[2]), (g.axis.x, g.axis.w));
        assert!(targets(d, &v, "CHASE").iter().all(|(t, _)| *t != Target::Graph));
        v.unfold = 1.0;
        let t = targets(d, &v, "CHASE");
        let (graph, bar) = (rect(&t, Target::Graph), rect(&t, Target::Bar));
        assert_eq!((graph[0], graph[2]), (g.axis.x, g.axis.w));
        assert!(graph[1] + graph[3] <= bar[1] + 0.01, "{graph:?} {bar:?}");
    }

    #[test]
    fn the_pills_come_with_the_end_and_the_card_goes_with_the_fade() {
        let (laps, trace) = (Laps::default(), Vec::<f32>::new());
        let d = canvases()[1];
        let mut v = view(&laps, &trace);
        assert!(targets(d, &v, "CHASE").iter().all(|(t, _)| !matches!(t, Target::Pick(_))));
        v.show = 0.2;
        assert!(targets(d, &v, "CHASE").is_empty());
        v.ended = true;
        let t = targets(d, &v, "CHASE");
        assert_eq!(t.len(), 2);
        let (a, b) = (rect(&t, Target::Pick(0)), rect(&t, Target::Pick(1)));
        assert!(a[0] + a[2] < b[0] && ((a[0] + b[0] + b[2]) * 0.5 - d.x(640.0)).abs() < 0.01);
    }
}
