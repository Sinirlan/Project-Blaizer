//! REMASTER Lap Analysis (#429 phase 3/4), concept A · Broadcast of `reference_materials/ui-overhaul/analysis-overhaul/`
//! (`A-broadcast.html`, `assets/analysis.{css,js}`, shots `shots/A-*.jpg`): two laps of one track side by side, like a TV
//! qualifying analysis. Reached from the Ghost screen's ANALYSE (F / pad Y); no Classic counterpart. The data is
//! `analysis_data.rs` (`motor_sim::lap_compare`); keys in `analysis_key_modern`.
//!
//! Design px of 1280 x 720 through `Glass`, 1:1 from the mockup's inline styles:
//!
//! | piece | rect / place |
//! |---|---|
//! | side A card / side B card (mirrored) | (62, 150, 336 x 236) / (882, 150, 336 x 236) |
//! | map card; the map's box | (410, 150, 460 x 236); 204 x 204 at (126, 26) in it |
//! | chart card | (62, 394, 1156 x 214) |
//! | speed plot / delta strip, in the chart card | (70, 62, 1062 x 78) / (70, 162, 1062 x 40) |
//! | hints | y 626, gap 28 |
//! | picker | 600 x 458 at (62, 150) for A, (618, 150) for B; rows 38 tall every 40 from (18, 78), nine in view |
//! | requester | 540 wide or more, centred on (640, 353), one OK button |
//!
//! Phase 4/4 adds the picker's BROWSE FILES row (a dashed 38 px row after the laps), the drop overlay (`.dropzone`: the scrim at
//! .8 and a 760 x 300 dashed gold box in the middle) and the KEEP THIS LAP? requester with its two buttons.
//!
//! Deviations: the map is the HUD map's projection (as on the Ghost screen) and its turns are `TrackProfile`'s; the
//! driver name has no text glow; the picker has the Ghost screen's scrollbar; no mouse, no toast.

use super::analysis_data::{file_name, lap_secs, AnaAsk, SideLap, Want};
use super::league_load::{line_box, GAIN, LOSS};
use super::pause_settings::glass_panel;
use super::personal::sel_rim;
use super::results::tag;
use super::*;
use motor_sim::lap_compare::{Comparison, Kind, LapTrace, Side as LapSide};
use motor_sim::race::TrackProfile;

/// Side colours (`analysis.js` SIDE): A ice (the palette's `--s1`), B gold; the Glass orange stays the selection rim.
const SIDE_COL: [u32; 2] = [0x8fd0ff, 0xffd54a];
const KEY: [&str; 2] = ["A", "B"];
/// a mini-sector within `TIE_MS` is nobody's: white
const TIE: u32 = 0xe6f6ff;
const CARD: [(f32, f32, f32, f32); 2] = [(62.0, 150.0, 336.0, 236.0), (882.0, 150.0, 336.0, 236.0)];
const MAP_CARD: (f32, f32, f32, f32) = (410.0, 150.0, 460.0, 236.0);
const CHART_CARD: (f32, f32, f32, f32) = (62.0, 394.0, 1156.0, 214.0);
const PLOT: (f32, f32, f32, f32) = (70.0, 62.0, 1062.0, 78.0);
const DELTA: (f32, f32, f32, f32) = (70.0, 162.0, 1062.0, 40.0);
/// the map's svg box in its card (x, y, size) and the room kept round the outline in it
const MAP_BOX: (f32, f32, f32) = (126.0, 26.0, 204.0);
const MAP_PAD: f32 = 14.0;
/// the mockup's map units: its outline lives in a 200 box drawn at 204
const MAP_U: f32 = 204.0 / 200.0;
const PICK: (f32, f32) = (600.0, 458.0);
const PICK_ROWS: usize = 9;
/// Q / E move the cursor this many grid points (`moveCursor`)
const CURSOR_STEP: usize = 8;

/// A closed outline drawn dashed: dashes of about `on` and gaps of about `off` (canvas px), the period fitted so the dashes meet
/// round the corner (a CSS `border: dashed`).
fn dashed_outline(m: &mut Mesh, pts: &[[f32; 2]], on: f32, off: f32, width: f32, col: [f32; 4], px: f32) {
    let n = pts.len();
    if n < 3 || on <= 0.0 {
        return;
    }
    let mut cum = vec![0.0f32];
    for i in 0..n {
        let (a, b) = (pts[i], pts[(i + 1) % n]);
        cum.push(cum[i] + (b[0] - a[0]).hypot(b[1] - a[1]));
    }
    let total = cum[n];
    let count = (total / (on + off)).round().max(1.0);
    let period = total / count;
    let on = period * on / (on + off);
    let at = |s: f32| {
        let i = cum.partition_point(|&c| c <= s).clamp(1, n) - 1;
        let (a, b) = (pts[i], pts[(i + 1) % n]);
        let f = if cum[i + 1] > cum[i] { (s - cum[i]) / (cum[i + 1] - cum[i]) } else { 0.0 };
        [a[0] + (b[0] - a[0]) * f, a[1] + (b[1] - a[1]) * f]
    };
    for k in 0..count as usize {
        let (s0, s1) = (k as f32 * period, k as f32 * period + on);
        let mut run = vec![at(s0)];
        run.extend((0..n).filter(|&i| cum[i] > s0 && cum[i] < s1).map(|i| pts[i]));
        run.push(at(s1));
        m.stroke(&run, false, width, Ends::Butt, col, px);
    }
}

/// A piece of the cursor readout: font, size, letter-spacing, colour, text, margin before and after (design px).
type ReadItem = (HudFont, f32, f32, [f32; 4], String, f32, f32);

/// `fmtG`: a signed gap in seconds with the true minus.
fn gap_s(ms: f32) -> String {
    format!("{}{:.3}", if ms < 0.0 { '\u{2212}' } else { '+' }, ms.abs() / 1000.0)
}

fn lap_s(ms: f32) -> String {
    lap_secs(ms.max(0.0).round() as u32)
}

/// Text with its CSS line box top at design (x, top); `al` 0 = left, 0.5 = centred, 1 = right-aligned on x (the trailing
/// letter-spacing counts, as in CSS). Returns the canvas width.
#[allow(clippy::too_many_arguments)]
fn text(b: &mut Batch, d: Glass, f: HudFont, px: f32, ls: f32, col: [f32; 4], s: &str, (x, top): (f32, f32), lh: Option<f32>, al: f32) -> f32 {
    let st = d.font(f, px, ls, col);
    let w = b.font_width(f, s, &st);
    b.font_text(f, s, d.x(x) - w * al, d.cap_top(f, px, d.y(top), lh), &st);
    w
}

/// SVG text: the baseline at canvas (x, base), anchored by `al` as `text`; `ol` = the mockup's dark outline
/// (`paint-order: stroke`, 3 px wide, half of it outside).
#[allow(clippy::too_many_arguments)]
fn svg_text(b: &mut Batch, d: Glass, f: HudFont, px: f32, ls: f32, col: [f32; 4], s: &str, (x, base): (f32, f32), al: f32, ol: bool) -> f32 {
    let mut st = d.font(f, px, ls, col);
    if ol {
        st = st.outline([4.0 / 255.0, 7.0 / 255.0, 13.0 / 255.0], d.d(1.5));
    }
    let w = b.font_width(f, s, &st);
    b.font_text(f, s, x - w * al, d.cap_above(f, px, base), &st);
    w
}

/// `.cap`: the dim caption, 11 px.
fn cap(b: &mut Batch, d: Glass, s: &str, at: (f32, f32), al: f32) -> f32 {
    text(b, d, HudFont::GlassMed, 11.0, 1.5, hex(G_DIM, 1.0), s, at, None, al)
}

/// `.note`: 11 px, the dim colour at .75.
fn note(b: &mut Batch, d: Glass, s: &str, at: (f32, f32), al: f32) -> f32 {
    text(b, d, HudFont::GlassMed, 11.0, 1.0, hex(G_DIM, 0.75), s, at, None, al)
}

/// `.sidetag`: the side's 22 x 22 chip (r 6) with its letter in Orbitron 12, dark on the colour with its glow; `dim` (the
/// other side's mark in the picker) only the colour's rim.
fn side_tag(b: &mut Batch, d: Glass, i: usize, (x, y): (f32, f32), dim: bool) {
    let col = SIDE_COL[i];
    let r = vec::rounded_rect(d.x(x), d.y(y), d.d(22.0), d.d(22.0), d.d(6.0));
    let mut m = Mesh::default();
    if dim {
        m.stroke(&r, true, d.w(1.0), Ends::Round, hex(col, 0.6), d.px);
    } else {
        m.halo(&r, d.d(10.0), hex(col, 0.55));
        m.fill(&r, hex(col, 1.0), d.px);
    }
    b.mesh(&m);
    let ink = if dim { hex(col, 1.0) } else { hex(G_DARK, 1.0) };
    text(b, d, HudFont::Orbitron, 12.0, 0.0, ink, KEY[i], (x + 11.0, y), Some(22.0), 0.5);
}

/// The source tag of a lap (`srcTag`): GHOST (`.tag-b`), RACE REPLAY (`.tag-v`), DEMO · Ln (`.tag-d`); top left at
/// canvas (x, y), returns its width.
fn src_tag(b: &mut Batch, d: Glass, l: &LapTrace, (x, y): (f32, f32)) -> f32 {
    match l.kind {
        Kind::Ghost => tag(b, d, "GHOST", (x, y), G_S1, 0.55),
        Kind::Replay => tag(b, d, "RACE REPLAY", (x, y), G_ORANGE, 0.6),
        Kind::Demo => Game::tag_dim(b, d, &format!("DEMO · L{}", l.lap.unwrap_or(0)), (x, y)),
    }
}

fn src_tag_text(l: &LapTrace) -> String {
    match l.kind {
        Kind::Ghost => "GHOST".into(),
        Kind::Replay => "RACE REPLAY".into(),
        Kind::Demo => format!("DEMO · L{}", l.lap.unwrap_or(0)),
    }
}

/// A point of the profile's outline at arc length `s` (between its 10 m points).
fn outline_at(p: &TrackProfile, s: f32) -> [f32; 3] {
    let n = p.outline.len();
    let s = s.clamp(0.0, p.length);
    let i = p.outline_s.partition_point(|&v| v <= s).saturating_sub(1).min(n - 1);
    let (s0, s1) = (p.outline_s[i], if i + 1 < n { p.outline_s[i + 1] } else { p.length });
    let (a, b) = (p.outline[i], p.outline[(i + 1) % n]);
    let f = if s1 > s0 { (s - s0) / (s1 - s0) } else { 0.0 };
    [a[0] + (b[0] - a[0]) * f, a[1] + (b[1] - a[1]) * f, a[2] + (b[2] - a[2]) * f]
}

/// The outline from arc `a` to `b` (its ends between points, so neighbouring stretches meet).
fn outline_run(p: &TrackProfile, a: f32, b: f32) -> Vec<[f32; 3]> {
    let mut v = vec![outline_at(p, a)];
    v.extend(p.outline.iter().zip(&p.outline_s).filter(|(_, &s)| s > a && s < b).map(|(q, _)| *q));
    v.push(outline_at(p, b));
    v
}

impl Game {
    /// The glass the hologram is frosted behind: the plate and the four cards (canvas px, with the radius).
    fn analysis_cards(&self, d: Glass) -> Vec<(f32, f32, f32, f32, f32)> {
        let plate = (d.r(1021.0), d.t(GLASS_PLATE_TOP), d.d(234.0), d.d(119.0));
        let rect = |(x, y, w, h): (f32, f32, f32, f32)| (d.x(x), d.y(y), d.d(w), d.d(h));
        [plate, rect(CARD[0]), rect(MAP_CARD), rect(CARD[1]), rect(CHART_CARD)].into_iter().map(|(x, y, w, h)| (x, y, w, h, d.d(12.0))).collect()
    }

    /// REMASTER Lap Analysis (#429): the Glass frame, the two driver cards, the map card, the chart card and the hints; over
    /// them the picker or a requester.
    pub(crate) fn analysis_modern(&self, size: (u32, u32)) -> Vec<Layer> {
        let canvas = Canvas::hud(size);
        let px = 1.0 / canvas.transform(size).0;
        let d = Glass::new(&canvas, px);
        let cards = self.analysis_cards(d);
        let back = self.ui_in(Batch::sharp_text(&self.assets, canvas, 1.0), |_, b| Self::glass_backdrop(b, d));
        let front = self.ui_in(Batch::sharp_text(&self.assets, canvas, 1.0), |g, b| {
            let pulse = 0.5 - 0.5 * (g.t * TAU / 1.1).cos();
            Self::glass_options_frame(b, d, g.t, "analysis", "MAIN // SINGLE // GHOST MODE // LAP ANALYSIS", "88-142:61", "LAPSCREEN", "187-D-61");
            let a = &g.analysis;
            let prof = g.ana_profile();
            let tr = a.track.and_then(|t| g.tracks.get(t));
            for i in 0..2 {
                g.ana_driver_card(b, d, i, pulse);
            }
            if let (Some(c), Some(p), Some(tr)) = (&a.cmp, prof, tr) {
                g.ana_map_card(b, d, c, p, tr);
                g.ana_chart_card(b, d, c, p);
            } else {
                for r in [MAP_CARD, CHART_CARD] {
                    let mut m = Mesh::default();
                    glass_panel(&mut m, d, (d.x(r.0), d.y(r.1), d.d(r.2), d.d(r.3)));
                    b.mesh(&m);
                }
            }
            if a.picker.is_some() {
                g.ana_picker(b, d, pulse);
            }
            let dev = g.controls.last;
            if let Some(ask) = &a.ask {
                let hints: &[(&[usize], &str)] = if matches!(ask, AnaAsk::Keep { .. }) { &[(&[0, 1], "CHOOSE"), (&[2], "CONFIRM")] } else { &[(&[2], "OK")] };
                g.req_dialog(b, d, (ask.icon(), ask.title()), &ask.runs(), &ask.buttons(), a.ask_btn, 540.0, hints);
            } else if let Some(sel) = a.picker {
                let pick = if sel >= a.list.len() { "BROWSE" } else { "PICK" };
                Self::glass_hints_off(b, d, dev, &[(&[9, 10], "LAP"), (&[2], pick), (&[3], "CANCEL")], (640.0, 626.0), (1.0, 28.0), &[]);
            } else {
                Self::glass_hints_off(b, d, dev, &[(&[0, 1], "SIDE"), (&[2], "CHANGE LAP"), (&[4, 5], "CURSOR"), (&[3], "BACK")], (640.0, 626.0), (1.0, 28.0), &[]);
            }
            if a.hover && a.ask.is_none() {
                g.ana_dropzone(b, d);
            }
        });
        let mut out = vec![back];
        out.extend(self.hologram(size, None, &cards));
        out.push(front);
        out
    }

    /// `driverCard`: position, driver, car, the side chip, source tag and date, LAP TIME and GAP, the three shares; the right
    /// card mirrored. A side being re-simulated shows the working overlay over it.
    fn ana_driver_card(&self, b: &mut Batch, d: Glass, i: usize, pulse: f32) {
        let (x, y, w, h) = CARD[i];
        let (r, col) = (i == 1, SIDE_COL[i]);
        let a = &self.analysis;
        let rect = (d.x(x), d.y(y), d.d(w), d.d(h));
        let mut m = Mesh::default();
        glass_panel(&mut m, d, rect);
        if a.side == i && a.picker.is_none() {
            sel_rim(&mut m, d, rect, pulse, 1.0);
        }
        let bar = vec::rounded_rect(d.x(if r { x + w - 17.0 } else { x + 14.0 }), d.y(y + 14.0), d.d(3.0), d.d(h - 28.0), d.d(2.0));
        m.halo(&bar, d.d(10.0), hex(col, 0.7));
        m.fill(&bar, hex(col, 1.0), d.px);
        b.mesh(&m);
        if let Some(s) = &a.sides[i] {
            self.ana_card_body(b, d, i, s);
        }
        // `.work`: a race replay of this side being re-simulated
        let work = a.job.as_ref().filter(|j| j.side == i).map(|j| (j.frac(), file_name(&j.path))).or_else(|| {
            a.fake_work.filter(|_| a.side == i).map(|f| (f, a.sides[i].as_ref().map_or_else(String::new, |s| file_name(&s.path))))
        });
        if let Some((f, file)) = work {
            let mut m = Mesh::default();
            m.fill(&vec::rounded_rect(rect.0, rect.1, rect.2, rect.3, d.d(12.0)), hex(0x050b16, 0.94), d.px);
            b.mesh(&m);
            let (lh, lp) = (line_box(HudFont::Glass, 18.0), line_box(HudFont::GlassMed, 12.0));
            let top = y + (h - (lh + 10.0 + 4.0 + 10.0 + lp)) * 0.5;
            let cx = x + w * 0.5;
            text(b, d, HudFont::Glass, 18.0, 2.0, WHITE, "RE-SIMULATING THE RACE", (cx, top), None, 0.5);
            let (bx, by) = (cx - 100.0, top + lh + 10.0);
            let mut m = Mesh::default();
            m.fill(&vec::rounded_rect(d.x(bx), d.y(by), d.d(200.0), d.d(4.0), d.d(2.0)), hex(G_LINE, 0.15), d.px);
            if f > 0.0 {
                let fill = vec::rounded_rect(d.x(bx), d.y(by), d.d(200.0 * f), d.d(4.0), d.d(2.0));
                m.halo(&fill, d.d(8.0), hex(col, 0.8));
                m.fill(&fill, hex(col, 1.0), d.px);
            }
            b.mesh(&m);
            let p = format!("{} · {} %", file.to_uppercase(), (f * 100.0).round() as u32);
            text(b, d, HudFont::GlassMed, 12.0, 1.0, hex(G_DIM, 1.0), &p, (cx, by + 14.0), None, 0.5);
        }
    }

    /// The card's text and bars for side `i`'s lap.
    fn ana_card_body(&self, b: &mut Batch, d: Glass, i: usize, s: &SideLap) {
        let (x, y, w, _) = CARD[i];
        let (r, col) = (i == 1, SIDE_COL[i]);
        // an offset from the card's own side (left for A, right for B) and the alignment that goes with it
        let ax = |o: f32| if r { x + w - o } else { x + o };
        let al = if r { 1.0 } else { 0.0 };
        let c = self.analysis.cmp.as_ref();
        // `.pos`: 1 for the faster side
        let gap = c.map(|c| if i == 0 { -c.lap_gap_ms } else { c.lap_gap_ms });
        let pos = if gap.is_none_or(|g| g <= 0.0) { "1" } else { "2" };
        text(b, d, HudFont::Orbitron, 40.0, 0.0, WHITE, pos, (ax(28.0), y + 10.0), Some(44.0), al);
        // `.who`: DRIVER, the name in the side's colour, the car
        let l11 = line_box(HudFont::GlassMed, 11.0);
        cap(b, d, "DRIVER", (ax(72.0), y + 12.0), al);
        text(b, d, HudFont::Glass, 30.0, 1.0, hex(col, 1.0), &s.driver.to_uppercase(), (ax(72.0), y + 12.0 + l11), Some(32.0), al);
        text(b, d, HudFont::Glass, 14.0, 1.0, hex(G_TEXT, 1.0), &s.car.to_uppercase(), (ax(72.0), y + 12.0 + l11 + 32.0), None, al);
        // `.meta` at 88: the side chip, the source tag, the date (reversed on the right card)
        let my = y + 88.0;
        let tag_txt = src_tag_text(&s.trace);
        let tw = Self::tag_width(b, d, &tag_txt) / d.s;
        let nw = b.font_width(HudFont::GlassMed, &s.note, &d.font(HudFont::GlassMed, 11.0, 1.0, WHITE)) / d.s;
        let (chip_x, tag_x, note_x) = if r {
            let e = ax(28.0);
            (e - 22.0, e - 22.0 - 6.0 - tw, e - 22.0 - 6.0 - tw - 6.0 - nw)
        } else {
            let e = ax(28.0);
            (e, e + 22.0 + 6.0, e + 22.0 + 6.0 + tw + 6.0)
        };
        side_tag(b, d, i, (chip_x, my), false);
        src_tag(b, d, &s.trace, (d.x(tag_x), d.y(my + 2.0)));
        note(b, d, &s.note, (note_x, my + (22.0 - l11) * 0.5), 0.0);
        // `.num`: LAP TIME, GAP (green when faster) with its unit
        cap(b, d, "LAP TIME", (ax(28.0), y + 114.0), al);
        text(b, d, HudFont::Glass, 26.0, 0.5, WHITE, &lap_s(s.trace.lap_ms), (ax(28.0), y + 114.0 + l11), Some(30.0), al);
        if let Some(g) = gap {
            cap(b, d, "GAP", (ax(178.0), y + 114.0), al);
            let vt = gap_s(g);
            let vs = d.font(HudFont::Glass, 26.0, 0.5, hex(if g <= 0.0 { GAIN } else { LOSS }, 1.0));
            let us = d.font(HudFont::GlassMed, 15.0, 1.0, hex(G_DIM, 1.0));
            let (vw, uw) = (b.font_width(HudFont::Glass, &vt, &vs), b.font_width(HudFont::GlassMed, "S", &us));
            let top = d.y(y + 114.0 + l11);
            let base = d.cap_top(HudFont::Glass, 26.0, top, Some(30.0)) + vs.size;
            let vx = if r { d.x(ax(178.0)) - uw - d.d(6.0) - vw } else { d.x(ax(178.0)) };
            b.font_text(HudFont::Glass, &vt, vx, base - vs.size, &vs);
            b.font_text(HudFont::GlassMed, "S", vx + vw + d.d(6.0), base - us.size, &us);
        }
        // the three shares (`sharesHTML`): name, value with %, a 4 px bar in the side's colour
        let Some(c) = c else { return };
        let sh = c.sides[i].shares;
        let (row_x, row_w) = (if r { x + w - 28.0 - 280.0 } else { x + 28.0 }, 280.0);
        for (k, (name, v)) in [("FULL THROTTLE", sh.throttle), ("HEAVY BRAKING", sh.braking), ("CORNERING", sh.cornering)].into_iter().enumerate() {
            let ry = y + 166.0 + 22.0 * k as f32;
            text(b, d, HudFont::GlassMed, 11.0, 1.5, hex(G_DIM, 1.0), name, (if r { row_x + row_w } else { row_x }, ry), None, al);
            let vs = d.font(HudFont::Glass, 15.0, 0.0, WHITE);
            let ps = d.font(HudFont::GlassMed, 10.0, 0.0, hex(G_DIM, 1.0));
            let vt = v.to_string();
            let (vw, pw) = (b.font_width(HudFont::Glass, &vt, &vs), b.font_width(HudFont::GlassMed, "%", &ps));
            let base = d.cap_top(HudFont::Glass, 15.0, d.y(ry - 2.0), None) + vs.size;
            let vx = if r { d.x(row_x) } else { d.x(row_x + row_w) - pw - d.d(1.0) - vw };
            b.font_text(HudFont::Glass, &vt, vx, base - vs.size, &vs);
            b.font_text(HudFont::GlassMed, "%", vx + vw + d.d(1.0), base - ps.size, &ps);
            let mut m = Mesh::default();
            m.fill(&vec::rounded_rect(d.x(row_x), d.y(ry + 15.0), d.d(row_w), d.d(4.0), d.d(2.0)), hex(G_LINE, 0.12), d.px);
            let fw = row_w * f32::from(v.min(100)) / 100.0;
            if fw > 0.0 {
                let fx = if r { row_x + row_w - fw } else { row_x };
                let fill = vec::rounded_rect(d.x(fx), d.y(ry + 15.0), d.d(fw), d.d(4.0), d.d(2.0));
                m.halo(&fill, d.d(8.0), hex(col, 0.6));
                m.fill(&fill, hex(col, 1.0), d.px);
            }
            b.mesh(&m);
        }
    }

    /// The map's projection into its box: the HUD map's (`minimap::map_point`) fitted, centred, into the 204 box.
    fn ana_map_fit(tr: &TrackInfo, p: &TrackProfile, d: Glass) -> LapFit {
        let (bx, by, bs) = MAP_BOX;
        let (cx, cy) = (MAP_CARD.0 + bx + bs * 0.5, MAP_CARD.1 + by + bs * 0.5);
        let map = |q: [f32; 3]| minimap::map_point(tr.map_info, Vec3::from_array(q));
        let (mut lo, mut hi) = ((f32::MAX, f32::MAX), (f32::MIN, f32::MIN));
        for &q in &p.outline {
            let (x, y) = map(q);
            lo = (lo.0.min(x), lo.1.min(y));
            hi = (hi.0.max(x), hi.1.max(y));
        }
        let span = (hi.0 - lo.0).max(hi.1 - lo.1).max(1e-3);
        LapFit { map_info: tr.map_info, mid: ((lo.0 + hi.0) * 0.5, (lo.1 + hi.1) * 0.5), k: d.d(bs - 2.0 * MAP_PAD) / span, c: [d.x(cx), d.y(cy)] }
    }

    /// `mapCard`: the caption, the lap with each mini-sector in its winner's colour, the start bar, turn numbers and speed
    /// classes, the cursor's two dots, the win counts and the top speeds.
    fn ana_map_card(&self, b: &mut Batch, d: Glass, c: &Comparison, p: &TrackProfile, tr: &TrackInfo) {
        let (x, y, w, h) = MAP_CARD;
        let mut m = Mesh::default();
        glass_panel(&mut m, d, (d.x(x), d.y(y), d.d(w), d.d(h)));
        b.mesh(&m);
        cap(b, d, &format!("{} · {:.2} KM · {} TURNS", tr.name.to_uppercase(), p.length / 1000.0, p.turns.len()), (x + 18.0, y + 12.0), 0.0);
        let fit = Self::ana_map_fit(tr, p, d);
        let line = fit.line(p);
        let n = line.len().max(1) as f32;
        let cen = line.iter().fold([0.0, 0.0], |a, q| [a[0] + q[0] / n, a[1] + q[1] / n]);
        let sw = 4.6;
        let mut m = Mesh::default();
        m.stroke(&line, true, d.w(sw * 2.3), Ends::Round, hex(G_DARK, 0.75), d.px);
        for ms in &c.minis {
            let col = ms.winner.map_or(TIE, |s| SIDE_COL[usize::from(s == LapSide::B)]);
            let run: Vec<[f32; 2]> = outline_run(p, ms.a, ms.b).into_iter().map(|q| fit.at(q)).collect();
            m.glow(&run, false, d.d(sw), d.d(3.0), hex(col, 0.6), 3);
            m.stroke(&run, false, d.w(sw), Ends::Butt, hex(col, 1.0), d.px);
        }
        // the start / finish bar across the line, 7 units each side
        if line.len() > 2 {
            let (p0, p1) = (line[0], line[1]);
            let (dx, dy) = (p1[0] - p0[0], p1[1] - p0[1]);
            let l = dx.hypot(dy).max(1e-6);
            let (nx, ny) = (-dy / l * d.d(7.0 * MAP_U), dx / l * d.d(7.0 * MAP_U));
            m.stroke(&[[p0[0] + nx, p0[1] + ny], [p0[0] - nx, p0[1] - ny]], false, d.w(2.5), Ends::Butt, WHITE, d.px);
        }
        // the cursor: B where it was when A passed the cursor, then A
        let a = &self.analysis;
        let mut dots = Vec::new();
        if let (Some(k), [Some(sa), Some(sb)]) = (a.cursor, &a.sides) {
            let k = k.min(sa.trace.xz.len().saturating_sub(1));
            if let Some(&qa) = sa.trace.xz.get(k) {
                let qb = sb.trace.xz_at_ms(sa.trace.t_ms[k]);
                dots = vec![(fit.at([qb[0], 0.0, qb[1]]), SIDE_COL[1]), (fit.at([qa[0], 0.0, qa[1]]), SIDE_COL[0])];
            }
        }
        for (q, col) in &dots {
            let ring = circle(*q, d.d(5.0));
            m.fill(&ring, hex(*col, 1.0), d.px);
            m.stroke(&ring, true, d.w(1.5), Ends::Round, hex(G_DARK, 1.0), d.px);
        }
        b.mesh(&m);
        // turn numbers, pushed out from the map's middle
        let away = |q: [f32; 2], push: (f32, f32)| {
            let (du, dv) = (q[0] - cen[0], q[1] - cen[1]);
            let l = du.hypot(dv).max(1e-6);
            ([q[0] + du / l * push.0, q[1] + dv / l * push.1], du)
        };
        for (j, t) in p.turns.iter().enumerate() {
            let (q, _) = away(fit.at(t.apex), (d.d(12.0 * MAP_U), d.d(12.0 * MAP_U)));
            svg_text(b, d, HudFont::Glass, 12.0, 0.0, WHITE, &(j + 1).to_string(), (q[0], q[1] + d.d(4.0 * MAP_U)), 0.5, true);
        }
        // the speed class once per run of equal classes, further out, at the run's first group; one that would cover a label
        // already drawn is left out (design choice: the mockup's declutter rule for dense tracks, taken one step further)
        let mut drawn: Vec<[f32; 4]> = Vec::new();
        for (j, ms) in c.minis.iter().enumerate() {
            if (j > 0 && c.minis[j - 1].class == ms.class) || ms.turns.is_empty() {
                continue;
            }
            let pts: Vec<[f32; 2]> = ms.turns.iter().filter_map(|&no| p.turns.get(no.saturating_sub(1))).map(|t| fit.at(t.apex)).collect();
            let k = pts.len().max(1) as f32;
            let g = pts.iter().fold([0.0, 0.0], |a, q| [a[0] + q[0] / k, a[1] + q[1] / k]);
            let (q, du) = away(g, (d.d(30.0 * MAP_U), d.d(26.0 * MAP_U)));
            let al = if du > d.d(25.0 * MAP_U) { 0.0 } else if du < -d.d(25.0 * MAP_U) { 1.0 } else { 0.5 };
            // kept under the caption and inside the card (deviation: a tall lap pushes a label into the caption)
            let base = q[1].clamp(d.y(y + 40.0), d.y(y + h - 8.0));
            let st = d.font(HudFont::GlassMed, 10.0, 1.2, WHITE);
            let lw = b.font_width(HudFont::GlassMed, ms.class.label(), &st);
            let r = [q[0] - lw * al - d.d(2.0), base - st.size - d.d(2.0), q[0] - lw * al + lw + d.d(2.0), base + d.d(2.0)];
            if drawn.iter().any(|o| r[0] < o[2] && o[0] < r[2] && r[1] < o[3] && o[1] < r[3]) {
                continue;
            }
            drawn.push(r);
            svg_text(b, d, HudFont::GlassMed, 10.0, 1.2, hex(G_TEXT, 0.7), ms.class.label(), (q[0], base), al, true);
        }
        // the win counts at the bottom left, the top speeds at the bottom right
        let l11 = line_box(HudFont::GlassMed, 11.0);
        for i in 0..2 {
            let ry = y + h - 12.0 - 22.0 - 28.0 * (1 - i) as f32;
            side_tag(b, d, i, (x + 18.0, ry), false);
            let won = c.minis.iter().filter(|ms| ms.winner == Some([LapSide::A, LapSide::B][i])).count();
            note(b, d, &format!("FASTER IN {won}"), (x + 18.0 + 28.0, ry + (22.0 - l11) * 0.5), 0.0);
        }
        let tops = format!("TOP {} / {}", c.sides[0].top_kmh.round(), c.sides[1].top_kmh.round());
        note(b, d, &tops, (x + w - 18.0, y + h - 12.0 - 2.0 * l11), 1.0);
        note(b, d, "KM/H", (x + w - 18.0, y + h - 12.0 - l11), 1.0);
    }

    /// `chartCard`: the speed plot over the lap with the mini-sector bands, class brackets, turn numbers and gains; the
    /// delta strip; the vertical captions; the cursor's line and readout.
    fn ana_chart_card(&self, b: &mut Batch, d: Glass, c: &Comparison, p: &TrackProfile) {
        let (x, y, w, h) = CHART_CARD;
        let a = &self.analysis;
        let [Some(sa), Some(sb)] = &a.sides else { return };
        let mut m = Mesh::default();
        glass_panel(&mut m, d, (d.x(x), d.y(y), d.d(w), d.d(h)));
        b.mesh(&m);
        cap(b, d, "SPEED · KM/H", (x + 70.0, y + 8.0), 0.0);
        let n = c.gap.len().min(sa.trace.kmh.len()).min(sb.trace.kmh.len());
        match a.cursor.filter(|_| n > 1) {
            Some(k) => self.ana_readout(b, d, c, (sa, sb), k.min(n - 1), (x + w - 24.0, y + 4.0)),
            None => {
                note(b, d, "Q / E MOVE THE CURSOR ALONG THE LAP", (x + w - 24.0, y + 4.0), 1.0);
            }
        }
        for (s, ty) in [("SPEED", 136.0), ("DELTA", 206.0)] {
            let from = b.vert_len();
            cap(b, d, s, (x + 22.0, y + ty), 0.0);
            b.rotate_from(from, [d.x(x + 22.0), d.y(y + ty)], -std::f32::consts::FRAC_PI_2);
        }
        if n < 2 || p.length <= 0.0 {
            return;
        }
        // the speed plot
        let (px0, py0, pw, ph) = (x + PLOT.0, y + PLOT.1, PLOT.2, PLOT.3);
        let st = &c.sides;
        let lo = (st[0].low_kmh.min(st[1].low_kmh) / 10.0).floor() * 10.0 - 10.0;
        let hi = (st[0].top_kmh.max(st[1].top_kmh) / 10.0).ceil() * 10.0 + 10.0;
        let sx = |s: f32| d.x(px0 + s / p.length * pw);
        let kx = |k: usize| d.x(px0 + k as f32 / (n - 1) as f32 * pw);
        let vy = |v: f32| d.y(py0 + ph - (v - lo) / (hi - lo).max(1.0) * ph);
        let py = |v: f32| d.y(py0 + v);
        let mut m = Mesh::default();
        for ms in &c.minis {
            let (x0, x1) = (sx(ms.a), sx(ms.b));
            let tint = ms.winner.map_or(hex(TIE, 0.05), |s| hex(SIDE_COL[usize::from(s == LapSide::B)], 0.09));
            m.fill(&[[x0, py(0.0)], [x1, py(0.0)], [x1, py(ph)], [x0, py(ph)]], tint, d.px);
            m.stroke(&[[x0, py(-30.0)], [x0, py(ph + 18.0)]], false, d.w(1.0), Ends::Butt, hex(G_LINE, 0.16), d.px);
            let (b0, b1) = (x0 + d.d(6.0), x1 - d.d(6.0));
            if b1 > b0 {
                m.stroke(&[[b0, py(-22.0)], [b0, py(-27.0)], [b1, py(-27.0)], [b1, py(-22.0)]], false, d.w(1.0), Ends::Butt, hex(G_LINE, 0.35), d.px);
            }
        }
        for v in [lo + 10.0, hi - 10.0] {
            m.dashed([d.x(px0), vy(v)], [d.x(px0 + pw), vy(v)], d.d(3.0), d.d(4.0), d.w(1.0), hex(G_LINE, 0.12), d.px);
        }
        for (i, sl) in [(1, sb), (0, sa)] {
            let pts: Vec<[f32; 2]> = sl.trace.kmh.iter().take(n).enumerate().map(|(k, &v)| [kx(k), vy(v)]).collect();
            m.glow(&pts, false, d.d(1.6), d.d(3.0), hex(SIDE_COL[i], 0.5), 3);
            m.stroke(&pts, false, d.w(1.6), Ends::Round, hex(SIDE_COL[i], 1.0), d.px);
        }
        if let Some(k) = a.cursor {
            let cx = kx(k.min(n - 1));
            m.stroke(&[[cx, py(-2.0)], [cx, py(ph + 2.0)]], false, d.w(1.0), Ends::Butt, [1.0, 1.0, 1.0, 0.8], d.px);
        }
        b.mesh(&m);
        for ms in &c.minis {
            let mid = (sx(ms.a) + sx(ms.b)) * 0.5;
            svg_text(b, d, HudFont::GlassMed, 9.5, 1.2, hex(G_TEXT, 0.6), ms.class.label(), (mid, py(-31.0)), 0.5, false);
            let (col, g) = match ms.winner {
                None => (TIE, "\u{b1}0.000".to_string()),
                Some(s) => (SIDE_COL[usize::from(s == LapSide::B)], gap_s(-ms.delta_ms.abs())),
            };
            svg_text(b, d, HudFont::Glass, 12.5, 0.5, hex(col, 1.0), &g, (mid, py(ph + 15.0)), 0.5, false);
        }
        // turn numbers: one closer than 11 px to the last drawn is skipped; TURN 1 only when turn 2 leaves room
        let mut last = f32::MIN;
        for (j, t) in p.turns.iter().enumerate() {
            let tx = t.s / p.length * pw;
            let nx = p.turns.get(j + 1).map_or(f32::MAX, |u| u.s / p.length * pw);
            if tx - last < 11.0 {
                continue;
            }
            last = tx;
            let label = if j == 0 && nx - tx > 60.0 { "TURN 1".to_string() } else { (j + 1).to_string() };
            svg_text(b, d, HudFont::Glass, 13.0, 0.5, WHITE, &label, (d.x(px0 + tx), py(-8.0)), 0.5, false);
        }
        for v in [hi - 10.0, lo + 10.0] {
            svg_text(b, d, HudFont::Glass, 11.0, 0.0, hex(G_DIM, 1.0), &format!("{v:.0}"), (d.x(px0 - 8.0), vy(v) + d.d(4.0)), 1.0, false);
        }
        // the delta strip: B flat, A's line up where A is ahead
        let (dx0, dy0, dw, dh) = (x + DELTA.0, y + DELTA.1, DELTA.2, DELTA.3);
        let mid = dh * 0.5;
        let kk = (mid - 4.0) / c.max_gap_ms.max(1.0);
        let dy = |v: f32| d.y(dy0 + v);
        let mut m = Mesh::default();
        let band = |y0: f32, y1: f32| [[d.x(dx0), dy(y0)], [d.x(dx0 + dw), dy(y0)], [d.x(dx0 + dw), dy(y1)], [d.x(dx0), dy(y1)]];
        m.fill(&band(0.0, mid), hex(GAIN, 0.05), d.px);
        m.fill(&band(mid, dh), hex(LOSS, 0.05), d.px);
        m.stroke(&[[d.x(dx0), dy(mid)], [d.x(dx0 + dw), dy(mid)]], false, d.w(1.4), Ends::Butt, hex(SIDE_COL[1], 1.0), d.px);
        let pts: Vec<[f32; 2]> = c.gap.iter().take(n).enumerate().map(|(k, &g)| [d.x(dx0 + k as f32 / (n - 1) as f32 * dw), dy(mid - g * kk)]).collect();
        m.glow(&pts, false, d.d(1.8), d.d(3.0), hex(SIDE_COL[0], 0.6), 3);
        m.stroke(&pts, false, d.w(1.8), Ends::Round, hex(SIDE_COL[0], 1.0), d.px);
        if let Some(k) = a.cursor {
            let cx = d.x(dx0 + k.min(n - 1) as f32 / (n - 1) as f32 * dw);
            m.stroke(&[[cx, dy(0.0)], [cx, dy(dh)]], false, d.w(1.0), Ends::Butt, [1.0, 1.0, 1.0, 0.8], d.px);
        }
        b.mesh(&m);
        let lab = hex(G_TEXT, 0.32);
        svg_text(b, d, HudFont::Orbitron, 10.0, 1.5, lab, "A FASTER", (d.x(dx0 + 8.0), dy(14.0)), 0.0, false);
        svg_text(b, d, HudFont::Orbitron, 10.0, 1.5, lab, "A SLOWER", (d.x(dx0 + 8.0), dy(dh - 5.0)), 0.0, false);
        svg_text(b, d, HudFont::Orbitron, 10.0, 1.5, lab, &format!("\u{b1}{:.2} S", c.max_gap_ms / 1000.0), (d.x(dx0 + dw - 6.0), dy(14.0)), 1.0, false);
    }

    /// `cursorHTML`: AT n M, both speeds, the gap there; right-aligned at design `right`, its box top at `top`, all on
    /// one baseline.
    fn ana_readout(&self, b: &mut Batch, d: Glass, c: &Comparison, (sa, sb): (&SideLap, &SideLap), k: usize, (right, top): (f32, f32)) {
        let g = c.gap[k];
        let gap = if g >= 0.0 { format!("A {}", gap_s(-g)) } else { format!("B {}", gap_s(g)) };
        let m_at = (k as f32 * c.step).round();
        let cap_c = hex(G_DIM, 1.0);
        // (font, size, letter-spacing, colour, text, margin before, margin after)
        let items: [ReadItem; 7] = [
            (HudFont::GlassMed, 11.0, 1.5, cap_c, format!("AT {m_at} M"), 4.0, 0.0),
            (HudFont::Glass, 16.0, 0.0, hex(SIDE_COL[0], 1.0), format!("{:.0}", sa.trace.kmh[k]), 0.0, 0.0),
            (HudFont::GlassMed, 10.0, 0.0, cap_c, "KM/H".into(), 0.0, 8.0),
            (HudFont::Glass, 16.0, 0.0, hex(SIDE_COL[1], 1.0), format!("{:.0}", sb.trace.kmh[k]), 0.0, 0.0),
            (HudFont::GlassMed, 10.0, 0.0, cap_c, "KM/H".into(), 0.0, 8.0),
            (HudFont::GlassMed, 11.0, 1.5, cap_c, "GAP".into(), 4.0, 0.0),
            (HudFont::Glass, 16.0, 0.0, WHITE, gap, 0.0, 0.0),
        ];
        let styles: Vec<FontStyle> = items.iter().map(|it| d.font(it.0, it.1, it.2, it.3)).collect();
        let widths: Vec<f32> = items.iter().zip(&styles).map(|(it, st)| b.font_width(it.0, &it.4, st) + d.d(it.5 + it.6)).collect();
        let total = widths.iter().sum::<f32>() + d.d(6.0) * (items.len() - 1) as f32;
        let base = d.cap_top(HudFont::Glass, 16.0, d.y(top), None) + d.font(HudFont::Glass, 16.0, 0.0, WHITE).size;
        let mut cx = d.x(right) - total;
        for ((it, st), w) in items.iter().zip(&styles).zip(&widths) {
            b.font_text(it.0, &it.4, cx + d.d(it.5), base - st.size, st);
            cx += w + d.d(6.0);
        }
    }

    /// `pickerCard`: the lap picker of the selected side over the screen (its side's half): the title with the side chip, the
    /// caption, the rows (source icon, driver and car, source and date or OUTDATED, lap time, the sides' marks), nine in view.
    fn ana_picker(&self, b: &mut Batch, d: Glass, pulse: f32) {
        let a = &self.analysis;
        let i = a.side.min(1);
        let (w, h) = PICK;
        let (x, y) = (if i == 1 { 1218.0 - w } else { 62.0 }, 150.0);
        Self::req_scrim(b);
        let mut m = Mesh::default();
        super::requester::req_card(&mut m, d, (d.x(x), d.y(y), d.d(w), d.d(h)));
        m.dashed([d.x(x + 18.0), d.y(y + 66.0)], [d.x(x + w - 18.0), d.y(y + 66.0)], d.d(6.0), d.d(4.0), d.w(1.0), hex(G_LINE, 0.45), d.px);
        b.mesh(&m);
        side_tag(b, d, i, (x + 20.0, y + 15.0), false);
        text(b, d, HudFont::Glass, 20.0, 0.5, WHITE, &format!("CHOOSE A LAP FOR SIDE {}", KEY[i]), (x + 52.0, y + 14.0), Some(24.0), 0.0);
        let track = a.track.and_then(|t| self.tracks.get(t)).map(|t| t.name.to_uppercase()).unwrap_or_default();
        note(b, d, &format!("{track} · EACH CAR'S BEST LAP · GHOSTS, RACE REPLAYS, DEMOS"), (x + 20.0, y + 44.0), 0.0);
        let list = &a.list;
        let sel = a.picker.unwrap_or(0);
        // the rows are the laps and, last, BROWSE FILES
        let total = list.len() + 1;
        let first = sel.saturating_sub(3).min(total.saturating_sub(PICK_ROWS));
        let rw = w - 36.0;
        let other = 1 - i;
        if list.is_empty() {
            note(b, d, "NO OTHER LAPS OF THIS TRACK", (x + 20.0, y + 78.0 + 46.0), 0.0);
        }
        for (r, k) in (first..total.min(first + PICK_ROWS)).enumerate() {
            let (rx, ry) = (x + 18.0, y + 78.0 + 40.0 * r as f32);
            let on = k == sel;
            let Some(row) = list.get(k) else {
                self.ana_browse_row(b, d, (rx, ry, rw), on, pulse);
                continue;
            };
            if on {
                let rr = vec::rounded_rect(d.x(rx), d.y(ry), d.d(rw), d.d(38.0), d.d(9.0));
                let mut m = Mesh::default();
                m.halo(&rr, d.d(12.0), hex(G_ORANGE, 0.3 + 0.1 * pulse));
                m.fill(&rr, hex(G_ORANGE, 0.08), d.px);
                m.stroke(&rr, true, d.w(1.0), Ends::Round, hex(G_ORANGE, 1.0), d.px);
                b.mesh(&m);
            }
            let fade = if row.outdated { 0.5 } else { 1.0 };
            let icon = match row.kind {
                Kind::Ghost => ui_font::icon::GHOST,
                Kind::Replay => ui_font::icon::FILM,
                Kind::Demo => ui_font::icon::DISC,
            };
            Self::glass_icon(b, icon, d.x(rx + 10.0), d.y(ry + 10.0), d.d(18.0), hex(G_DIM, 1.0));
            // the driver and, smaller, the car on one baseline
            let ns = d.font(HudFont::Glass, 16.0, 0.5, hex(if on { G_ORANGE } else { G_TEXT }, fade));
            let cs = d.font(HudFont::GlassMed, 12.0, 1.0, hex(G_DIM, fade));
            let base = d.cap_top(HudFont::Glass, 16.0, d.y(ry), Some(38.0)) + ns.size;
            let nx = d.x(rx + 38.0);
            let nw = b.font_text(HudFont::Glass, &row.driver.to_uppercase(), nx, base - ns.size, &ns);
            b.font_text(HudFont::GlassMed, &row.car.to_uppercase(), nx + nw + d.d(4.0), base - cs.size, &cs);
            // from the right: the side mark, the lap time (56 wide), the source and date
            let mine = |s: &Option<SideLap>| s.as_ref().is_some_and(|s| s.path == row.path && row.slot.is_none_or(|sl| sl == s.trace.slot));
            let mut right = rx + rw - 10.0;
            if mine(&a.sides[i]) {
                side_tag(b, d, i, (right - 22.0, ry + 8.0), false);
                right -= 32.0;
            } else if mine(&a.sides[other]) {
                side_tag(b, d, other, (right - 22.0, ry + 8.0), true);
                right -= 32.0;
            }
            let lt = row.lap_ms.map_or_else(|| "-".to_string(), lap_s);
            text(b, d, HudFont::Glass, 16.0, 0.0, hex(0xffffff, fade), &lt, (right, ry), Some(38.0), 1.0);
            right -= 56.0 + 10.0;
            if row.outdated {
                let tw = Self::tag_width(b, d, "OUTDATED") / d.s;
                tag(b, d, "OUTDATED", (d.x(right - tw), d.y(ry + 10.0)), LOSS, 0.5);
            } else {
                let src = match row.kind {
                    Kind::Ghost => format!("GHOST · {}", row.date),
                    Kind::Replay => format!("RACE REPLAY · {}", row.date),
                    Kind::Demo => format!("DEMO · {} L{}", row.stem.to_uppercase(), row.lap.unwrap_or(0)),
                };
                text(b, d, HudFont::GlassMed, 11.0, 1.0, hex(G_DIM, 0.75), &src, (right, ry), Some(38.0), 1.0);
            }
        }
        // the Ghost screen's scrollbar, past the rows (deviation: the mockup's list has none)
        let n = total;
        if n > PICK_ROWS {
            let (sx, sy, sh) = (x + w - 12.0, y + 78.0, PICK_ROWS as f32 * 40.0 - 2.0);
            let mut m = Mesh::default();
            m.fill(&vec::rounded_rect(d.x(sx), d.y(sy), d.d(3.0), d.d(sh), d.d(2.0)), hex(G_LINE, 0.12), d.px);
            m.fill(&vec::rounded_rect(d.x(sx), d.y(sy + sh * first as f32 / n as f32), d.d(3.0), d.d(sh * PICK_ROWS as f32 / n as f32), d.d(2.0)), hex(G_LINE, 0.6), d.px);
            b.mesh(&m);
        }
    }

    /// `.prow.browse`: the picker's last row, a dashed 38 px row: the folder icon, BROWSE FILES… and (not on Wayland, where
    /// winit delivers no dropped files) OR DROP A FILE ON THE WINDOW.
    fn ana_browse_row(&self, b: &mut Batch, d: Glass, (rx, ry, rw): (f32, f32, f32), on: bool, pulse: f32) {
        let rr = vec::rounded_rect(d.x(rx), d.y(ry), d.d(rw), d.d(38.0), d.d(9.0));
        let mut m = Mesh::default();
        if on {
            m.halo(&rr, d.d(12.0), hex(G_ORANGE, 0.3 + 0.1 * pulse));
            m.fill(&rr, hex(G_ORANGE, 0.08), d.px);
        }
        dashed_outline(&mut m, &rr, d.d(5.0), d.d(3.0), d.w(1.0), if on { hex(G_ORANGE, 1.0) } else { hex(G_LINE, 0.3) }, d.px);
        b.mesh(&m);
        Self::glass_icon(b, ui_font::icon::FOLDER_OPEN, d.x(rx + 10.0), d.y(ry + 10.0), d.d(18.0), hex(G_DIM, 1.0));
        text(b, d, HudFont::Glass, 16.0, 0.5, hex(if on { G_ORANGE } else { G_TEXT }, 1.0), "BROWSE FILES\u{2026}", (rx + 38.0, ry), Some(38.0), 0.0);
        if !self.analysis.wayland {
            text(b, d, HudFont::GlassMed, 11.0, 1.0, hex(G_DIM, 0.75), "OR DROP A FILE ON THE WINDOW", (rx + rw - 10.0, ry), Some(38.0), 1.0);
        }
    }

    /// `.dropzone`: while a file is dragged over the window: the scrim at .8 over the screen and, in the middle (640, 360), the
    /// 760 x 300 dashed gold box with the folder icon, the headline for the selected side and the accepted files.
    fn ana_dropzone(&self, b: &mut Batch, d: Glass) {
        let gold = SIDE_COL[1];
        let mut m = Mesh::default();
        m.quad(0.0, 0.0, b.canvas.w, b.canvas.h, [[2.0 / 255.0, 6.0 / 255.0, 14.0 / 255.0, 0.8]; 4]);
        b.mesh(&m);
        let r = vec::rounded_rect(d.x(640.0 - 380.0), d.y(360.0 - 150.0), d.d(760.0), d.d(300.0), d.d(18.0));
        let mut m = Mesh::default();
        m.halo(&r, d.d(40.0), hex(gold, 0.18));
        m.fill(&r, hex(gold, 0.06), d.px);
        dashed_outline(&mut m, &r, d.d(9.0), d.d(6.0), d.w(2.0), hex(gold, 0.8), d.px);
        b.mesh(&m);
        let (lh, lp) = (line_box(HudFont::Glass, 30.0), line_box(HudFont::GlassMed, 13.0));
        let top = 360.0 - (46.0 + 12.0 + lh + 12.0 + lp) * 0.5;
        Self::glass_icon(b, ui_font::icon::FOLDER_OPEN, d.x(640.0 - 23.0), d.y(top), d.d(46.0), hex(gold, 1.0));
        let head = format!("DROP TO COMPARE ON SIDE {}", KEY[self.analysis.side.min(1)]);
        text(b, d, HudFont::Glass, 30.0, 2.0, WHITE, &head, (640.0, top + 46.0 + 12.0), None, 0.5);
        text(b, d, HudFont::GlassMed, 13.0, 1.5, hex(G_DIM, 1.0), "GHOST (.GHL) \u{b7} RACE REPLAY (.MHR) \u{b7} DEMO (.MDE) \u{b7} THE SAME TRACK", (640.0, top + 46.0 + 12.0 + lh + 12.0), None, 0.5);
    }

    /// Keys of Lap Analysis (`Screen::Analysis`): a requester takes Enter / Esc (KEEP THIS LAP? also left / right); the open picker up / down, Enter (pick) and
    /// Esc; else left / right choose the side, Enter opens its picker, Q / E (LB / RB) move the cursor, Esc clears the cursor
    /// and then goes back to the Ghost screen.
    pub(crate) fn analysis_key_modern(&mut self, k: KeyCode) {
        let enter = matches!(k, KeyCode::Enter | KeyCode::NumpadEnter | KeyCode::Space);
        let back = matches!(k, KeyCode::Escape | KeyCode::Backspace);
        let mut e = None;
        // the system file dialog is open: it has the keyboard
        if self.analysis.dialog {
            self.ev_override = None;
            return;
        }
        if let Some(AnaAsk::Keep { path, .. }) = self.analysis.ask.clone() {
            // KEEP THIS LAP?: left / right choose, Enter answers, Esc is NOT NOW
            let (r, ev) = requester_key(k, self.analysis.ask_btn);
            match r {
                Req::Move(to) => self.analysis.ask_btn = to,
                Req::Pick(i) => {
                    self.analysis.ask = None;
                    if i == 0 {
                        self.ana_keep(&path);
                    } else {
                        self.ana_decline(&path);
                    }
                }
                Req::None => {}
            }
            e = ev.map(|v| if self.analysis.ask.is_some() && v == ev::SELECT { ev::REQUESTER } else { v });
            self.ev_override = e;
            return;
        }
        if self.analysis.ask.is_some() {
            if enter || back {
                self.analysis.ask = None;
                e = Some(ev::SELECT);
            }
            self.ev_override = e;
            return;
        }
        let side = self.analysis.side.min(1);
        if let Some(sel) = self.analysis.picker {
            // the laps and, last, BROWSE FILES
            let n = self.analysis.list.len() + 1;
            match k {
                KeyCode::ArrowUp | KeyCode::ArrowDown if n > 0 => {
                    let to = if k == KeyCode::ArrowDown { (sel + 1).min(n - 1) } else { sel.saturating_sub(1) };
                    e = Some(if to == sel {
                        ev::CURSOR_END
                    } else if k == KeyCode::ArrowUp {
                        ev::CURSOR_UP
                    } else {
                        ev::CURSOR_DOWN
                    });
                    self.analysis.picker = Some(to);
                }
                _ if enter && sel >= self.analysis.list.len() => {
                    // the picker stays open under the dialog; a chosen file closes it, a cancel leaves it
                    self.ana_browse();
                    e = Some(ev::SELECT);
                }
                _ if enter => {
                    self.analysis.picker = None;
                    if let Some(row) = self.analysis.list.get(sel).cloned() {
                        let want = Want { slot: row.slot, car: None, lap: row.slot.and(row.lap) };
                        self.ana_load(side, &row.path, want);
                        e = Some(if self.analysis.ask.is_some() { ev::REQUESTER } else { ev::SELECT });
                    } else {
                        e = Some(ev::LIST_OUT);
                    }
                }
                _ if back => {
                    self.analysis.picker = None;
                    e = Some(ev::LIST_OUT);
                }
                _ => {}
            }
            self.ev_override = e;
            return;
        }
        match k {
            KeyCode::ArrowLeft | KeyCode::ArrowRight => {
                let to = usize::from(k == KeyCode::ArrowRight);
                e = Some(if to == side { ev::CURSOR_END } else { ev::SELECT });
                self.analysis.side = to;
            }
            KeyCode::PageUp | KeyCode::PageDown | KeyCode::KeyQ | KeyCode::KeyE => {
                let fwd = matches!(k, KeyCode::PageDown | KeyCode::KeyE);
                let n = self.analysis.cmp.as_ref().map_or(0, |c| c.gap.len());
                if n > 1 {
                    let to = match self.analysis.cursor {
                        None => if fwd { 0 } else { n - 1 },
                        Some(c) if fwd => (c + CURSOR_STEP).min(n - 1),
                        Some(c) => c.saturating_sub(CURSOR_STEP),
                    };
                    e = Some(if Some(to) == self.analysis.cursor { ev::CURSOR_END } else if fwd { ev::CURSOR_DOWN } else { ev::CURSOR_UP });
                    self.analysis.cursor = Some(to);
                }
            }
            _ if enter => {
                let cur = self.analysis.sides[side].as_ref();
                let at = cur.and_then(|s| self.analysis.list.iter().position(|r| r.path == s.path && r.slot.is_none_or(|sl| sl == s.trace.slot)));
                self.analysis.picker = Some(at.unwrap_or(0));
                e = Some(ev::LIST_IN);
            }
            _ if back => {
                if self.analysis.cursor.take().is_none() {
                    self.go(Screen::Replays);
                }
                e = Some(ev::ESCAPE);
            }
            _ => {}
        }
        self.ev_override = e;
    }
}
