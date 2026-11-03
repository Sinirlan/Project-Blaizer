//! REMASTER League › Load (#92 phase 1, concept B · Paddock of `reference_materials/leagueload-overhaul/B-paddock.html`,
//! shots `shots/B-*.jpg`): the saved leagues as slot cards down the left (division badge, race and points, your place),
//! and a panel with the selected save as you left it: the status line, the next race with its map and the season's dots,
//! and your division's standings with the zone bars, portraits and your row. The Final shows its two rows and the
//! division ladder. Numbers from `league.rs` (`League`, `zone`, `season_tracks`, `next_track`); keys in front.rs
//! `Screen::LeagueLoad`. Design px through `Glass`, 1:1 from the mockup's inline styles (08-missing-features 8.8).

use super::pause_settings::glass_panel;
use super::personal::rgb_hex;
use super::results::{ord, tag, txt, txt_r};
use super::*;
use crate::league::{self, League, Place};

/// A slot card: 300 x 44 at x 62, 46 apart from y 228, eight in view.
pub(super) const SLOT: (f32, f32, f32, f32) = (62.0, 228.0, 300.0, 44.0);
pub(super) const STEP: f32 = 46.0;
pub(super) const VIEW: usize = 8;
/// The panel beside them.
pub(super) const PANEL: (f32, f32, f32, f32) = (382.0, 228.0, 836.0, 372.0);
/// The panel's inner width (its 1 px borders off) and the standings' first row from its inner top.
pub(super) const INNER_W: f32 = 834.0;
const ROWS_TOP: f32 = 112.0;
/// The zone bars and the status tones: the gain and loss tokens of doc 19.2.
pub(super) const GAIN: u32 = 0x7cf0a8;
pub(super) const LOSS: u32 = 0xff8a7a;
/// `--orange-text`
pub(super) const ORANGE_TEXT: u32 = 0xffb26a;
/// `.note`: a small caption in the dim blue at .75
pub(super) const NOTE: u32 = 0x8fb5c9;

/// How the status line is coloured (`.status.up` / `.down` / `.final`, plain otherwise).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum Tone {
    Plain,
    Up,
    Down,
    Final,
}

/// A save with everything the screen shows worked out: your division and place, how far the season has gone, the
/// status line.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Save {
    /// 0 = the final
    pub div: usize,
    /// your 0-based place in the division, and the division's size
    pub place: usize,
    pub size: usize,
    pub score: u32,
    /// a race of the season has been raced
    pub started: bool,
    /// the races the season has (`season_tracks`), the ones raced (`RaceNumber`) and the one shown as next
    pub races: usize,
    pub raced: usize,
    pub next: usize,
    pub tone: Tone,
    pub status: String,
    /// points between you and the leader
    pub to_leader: u32,
}

impl Save {
    /// The numbers of `l` on a track list with these `codes`.
    pub(super) fn of(l: &League, codes: &[u32]) -> Save {
        let div = l.human_division();
        let table = l.division(div);
        let place = table.iter().position(|p| p.human).unwrap_or(0);
        let score = table.get(place).map_or(0, |p| p.score);
        let races = league::season_tracks(codes, div).len();
        let raced = l.race_number as usize;
        // `League::next_track`: past the last race it starts over
        let next = if raced < races { raced } else { 0 };
        let started = raced > 0;
        let (tone, status) = if div == 0 {
            (Tone::Final, "ONE RACE FOR THE LEAGUE".to_string())
        } else if !started {
            (Tone::Plain, "SEASON NOT STARTED".to_string())
        } else {
            match league::zone(div, place) {
                Some(true) => (Tone::Up, if div == 1 { "ON COURSE FOR THE FINAL" } else { "IN THE PROMOTION ZONE" }.to_string()),
                Some(false) => (Tone::Down, "IN THE DROP ZONE".to_string()),
                None => {
                    // the last place that goes up, and what is missing to it
                    let last_up = if div == 1 { 0 } else { 1 };
                    let gap = table.get(last_up).map_or(0, |p| p.score).saturating_sub(score);
                    let text = if gap > 0 { format!("{gap} PTS OFF {}", if div == 1 { "THE FINAL" } else { "PROMOTION" }) } else { format!("LEVEL WITH {} PLACE", ord(last_up + 1)) };
                    (Tone::Plain, text)
                }
            }
        };
        let to_leader = table.first().map_or(0, |p| p.score).saturating_sub(score);
        Save { div, place, size: table.len(), score, started, races, raced, next, tone, status, to_leader }
    }

    /// `RACE 2 OF 4`, the final's `THE FINAL RACE`.
    pub(super) fn race_text(&self) -> String {
        if self.div == 0 { "THE FINAL RACE".into() } else { format!("RACE {} OF {}", self.next + 1, self.races) }
    }

    pub(super) fn div_name(&self) -> String {
        if self.div == 0 { "THE FINAL".into() } else { format!("DIVISION {}", self.div) }
    }

    /// The season's dots, one per race: 9 px, the next race 11.
    pub(super) fn dot_sizes(&self) -> Vec<f32> {
        (0..self.races).map(|k| if k == self.next && k >= self.raced { 11.0 } else { 9.0 }).collect()
    }
}

/// Where the cells of a standings table sit, design px from a row's padding box (the headings 1 px left of them).
#[derive(Clone, Copy)]
pub(super) struct Cols {
    pub pic_w: f32,
    pub nm: f32,
    /// the nick's size
    pub nm_px: f32,
    pub car: f32,
    /// the first of WINS / 2ND / 3RD, their step and their width
    pub w: f32,
    pub w_step: f32,
    pub w_w: f32,
    /// the GAP column's left (`GAP_W` wide); the League Load panel has none
    pub gap: Option<f32>,
    pub pts: f32,
    pub pts_w: f32,
}

/// The GAP column's width.
const GAP_W: f32 = 56.0;

/// The League Load panel's columns (10 in from its edges).
const LOAD_COLS: Cols = Cols { pic_w: 46.0, nm: 106.0, nm_px: 17.0, car: 300.0, w: 440.0, w_step: 52.0, w_w: 44.0, gap: None, pts: 726.0, pts_w: 76.0 };

/// A division's standings table to draw: `origin` = the panel's inner top left, `top` the first row's top from it,
/// `zone` the mark of a place (true = goes up, false = goes down) once the season `started`; `dim` = the zero state.
pub(super) struct Standings<'a> {
    pub table: &'a [Place],
    pub div: usize,
    pub started: bool,
    pub dim: bool,
    pub origin: (f32, f32),
    pub top: f32,
    pub row_h: f32,
    pub row_w: f32,
    pub cols: Cols,
    pub zone: fn(usize, usize) -> Option<bool>,
}

/// The first card of the window: the selection on its fourth card, no blank end (the mockup's `first`).
pub(super) fn window_first(sel: usize, n: usize) -> usize {
    sel.saturating_sub(3).min(n.saturating_sub(VIEW))
}

/// A CSS line box of normal height (design px) of font `f` at `px`.
pub(super) fn line_box(f: HudFont, px: f32) -> f32 {
    let face = ui_font::face(f);
    (face.ascender - face.descender) * px / ui_font::SRC
}

/// `yyyy-mm-dd` of a `CreateDate`; None when the file has none.
pub(super) fn date_text(date: u32) -> Option<String> {
    (date > 0).then(|| format!("{}-{:02}-{:02}", date / 10000, date / 100 % 100, date % 100))
}

/// A standings row's car: its division's, else the second car (`cars[div] || cars[1]`).
pub(super) fn car_of(p: &Place, div: usize) -> String {
    let c = &p.p.cars[div.min(3)];
    if c.is_empty() { p.p.cars[1].to_uppercase() } else { c.to_uppercase() }
}

impl Game {
    /// The glass the hologram is frosted behind: the plate, the slot cards, the panel (canvas px, with the radius).
    fn league_load_cards(&self, d: Glass) -> Vec<(f32, f32, f32, f32, f32)> {
        let plate = (d.r(1021.0), d.t(GLASS_PLATE_TOP), d.d(234.0), d.d(119.0));
        let n = self.leagues.len();
        let f = window_first(self.sel.min(n.saturating_sub(1)), n);
        let rect = |(x, y, w, h): (f32, f32, f32, f32)| (d.x(x), d.y(y), d.d(w), d.d(h));
        [plate]
            .into_iter()
            .chain((0..VIEW.min(n - f)).map(|k| rect((SLOT.0, SLOT.1 + STEP * k as f32, SLOT.2, SLOT.3))).chain([rect(PANEL)]))
            .map(|(x, y, w, h)| (x, y, w, h, d.d(12.0)))
            .collect()
    }

    /// REMASTER League › Load (#92): the Glass frame, the save cards and the selected save's panel over the hologram.
    pub(crate) fn league_load_modern(&self, size: (u32, u32)) -> Vec<Layer> {
        let canvas = Canvas::hud(size);
        let px = 1.0 / canvas.transform(size).0;
        let d = Glass::new(&canvas, px);
        let cards = self.league_load_cards(d);
        let back = self.ui_in(Batch::sharp_text(&self.assets, canvas, 1.0), |_, b| Self::glass_backdrop(b, d));
        let front = self.ui_in(Batch::sharp_text(&self.assets, canvas, 1.0), |g, b| {
            let pulse = 0.5 - 0.5 * (g.t * TAU / 1.1).cos();
            // the title and plate codes are the Classic League art's (`frame_common("leagu", "scrc")`)
            Self::glass_options_frame(b, d, g.t, "league", "MAIN // LEAGUE // LOAD", "88-137:59", "LEAGUESCREEN", "187-C-35");
            g.league_load_body(b, d, pulse);
            if !g.asking() {
                Self::glass_hints(b, d, g.controls.last, &[(&[9, 10], "LEAGUE"), (&[2], "LOAD"), (&[16], "DELETE"), (&[3], "BACK")], 626.0);
            }
        });
        let mut out = vec![back];
        out.extend(self.hologram(size, None, &cards));
        out.push(front);
        out
    }

    /// The cards and the panel of the selected save.
    fn league_load_body(&self, b: &mut Batch, d: Glass, pulse: f32) {
        let n = self.leagues.len();
        if n == 0 {
            return;
        }
        let sel = self.sel.min(n - 1);
        let codes = self.track_codes();
        let f = window_first(sel, n);
        for (k, l) in self.leagues.iter().enumerate().skip(f).take(VIEW) {
            self.league_slot(b, d, (k - f, l, &Save::of(l, &codes)), (k == sel, k == self.league), pulse);
        }
        // `.scrollbar`: 3 px at x 370 past eight cards, the thumb the share shown
        if n > VIEW {
            let h = VIEW as f32 * STEP - 2.0;
            let mut m = Mesh::default();
            m.fill(&vec::rounded_rect(d.x(370.0), d.y(SLOT.1), d.d(3.0), d.d(h), d.d(2.0)), hex(G_LINE, 0.12), d.px);
            m.fill(&vec::rounded_rect(d.x(370.0), d.y(SLOT.1 + h * f as f32 / n as f32), d.d(3.0), d.d(h * VIEW as f32 / n as f32), d.d(2.0)), hex(G_LINE, 0.6), d.px);
            b.mesh(&m);
        }
        let l = &self.leagues[sel];
        self.league_panel(b, d, l, &Save::of(l, &codes), sel == self.league);
    }

    /// `.slot`: the division badge, the name (with the dot of the current league), the race and points, your place.
    /// `pos` = its place in the window, `flags` = (selected, current).
    fn league_slot(&self, b: &mut Batch, d: Glass, (pos, l, sv): (usize, &League, &Save), (sel, current): (bool, bool), pulse: f32) {
        let (x, y) = (SLOT.0, SLOT.1 + STEP * pos as f32);
        Self::slot_ground(b, d, (x, y, SLOT.2, SLOT.3), sel, pulse);
        // the padding box: the card's 1 px border in
        let (ix, iy) = (x + 1.0, y + 1.0);
        Self::division_badge(b, d, (ix + 10.0, iy + 4.0), 36.0, sv.div, sel);
        let right = d.x(ix + SLOT.2 - 2.0 - 14.0);
        // `.pn`: `#n` and `/8`, or the dash of a season not started
        let lh = Some(SLOT.3 - 2.0);
        let top = d.y(iy);
        let mut place_w = d.d(10.0);
        if sv.started {
            let of = format!("/{}", sv.size);
            let small = d.font(HudFont::GlassMed, 12.0, 0.0, WHITE);
            let ow = b.font_width(HudFont::GlassMed, &of, &small);
            txt_r(b, d, HudFont::GlassMed, 12.0, 0.0, hex(G_DIM, 1.0), &of, right, top, lh);
            let num = format!("#{}", sv.place + 1);
            let nw = b.font_width(HudFont::Glass, &num, &d.font(HudFont::Glass, 22.0, 0.0, WHITE));
            txt_r(b, d, HudFont::Glass, 22.0, 0.0, hex(0xffffff, 1.0), &num, right - ow - d.d(3.0), top, lh);
            place_w = ow + d.d(3.0) + nw;
        } else {
            Self::dash(b, d, right, top + d.d(SLOT.3 * 0.5 - 1.0), 9.0);
        }
        // `.t`: the name (cut to stay clear of the place), then the current league's dot after 8 px
        let tx = d.x(ix + 62.0);
        let st = d.font(HudFont::Glass, 18.0, 1.0, WHITE);
        let room = right - place_w - d.d(12.0) - tx - if current { d.d(14.0) } else { 0.0 };
        let mut name = l.name.to_uppercase();
        while b.font_width(HudFont::Glass, &name, &st) > room && name.chars().count() > 1 {
            name.pop();
        }
        let name_top = d.y(iy + 5.0);
        let nw = txt(b, d, HudFont::Glass, 18.0, 1.0, hex(if sel { G_ORANGE } else { G_TEXT }, 1.0), &name, tx, name_top, None);
        if current {
            let c = [tx + nw + d.d(8.0 + 3.0), name_top + d.d(line_box(HudFont::Glass, 18.0)) * 0.5];
            let mut m = Mesh::default();
            m.halo(&circle(c, d.d(3.0)), d.d(6.0), hex(0x4aa8ff, 0.9));
            m.fill(&circle(c, d.d(3.0)), hex(G_S1, 1.0), d.px);
            b.mesh(&m);
        }
        // `.s`: the race and the points (the points, and the Final's name, in white)
        let parts: Vec<(String, bool)> = if sv.div == 0 {
            vec![("THE FINAL".into(), true), (" \u{b7} ONE RACE".into(), false)]
        } else if sv.started {
            vec![(format!("{} \u{b7} ", sv.race_text()), false), (sv.score.to_string(), true), (" PTS".into(), false)]
        } else {
            vec![(format!("{} \u{b7} NOT STARTED", sv.race_text()), false)]
        };
        let mut sx = tx;
        let sub_top = d.y(iy + 26.0);
        for (s, bold) in parts {
            sx += if bold {
                txt(b, d, HudFont::Glass, 13.0, 0.5, hex(0xffffff, 1.0), &s, sx, sub_top, None)
            } else {
                txt(b, d, HudFont::GlassMed, 13.0, 0.5, hex(G_DIM, 1.0), &s, sx, sub_top, None)
            };
        }
    }

    /// `.dbadge`: the division's number over `DIV` in a glass square of `s` design px with its top left at (x, y); the
    /// Final shows the trophy.
    fn division_badge(b: &mut Batch, d: Glass, (x, y): (f32, f32), s: f32, div: usize, sel: bool) {
        let r = vec::rounded_rect(d.x(x), d.y(y), d.d(s), d.d(s), d.d(8.0));
        let mut m = Mesh::default();
        m.fill(&r, hex(0x040a16, 0.45), d.px);
        let top = d.y(y);
        m.fill_with(&r, &|q| [1.0, 1.0, 1.0, 0.10 - 0.08 * ((q[1] - top) / d.d(s)).clamp(0.0, 1.0)], d.px);
        m.stroke(&r, true, d.w(1.0), Ends::Round, hex(if sel { G_ORANGE } else { G_LINE }, if sel { 0.6 } else { 0.3 }), d.px);
        b.mesh(&m);
        if div == 0 {
            let icon = (s * 0.5).round();
            Self::glass_icon(b, ui_font::icon::TROPHY, d.x(x + (s - icon) * 0.5), d.y(y + (s - icon) * 0.5), d.d(icon), hex(ORANGE_TEXT, 1.0));
            return;
        }
        // the number (half the badge) and the 8 px `DIV` 2 px under it, centred as a block in the inner box
        let big = (s * 0.5).round();
        let block = big + 2.0 + 8.0;
        let top = y + 1.0 + (s - 2.0 - block) * 0.5;
        let cx = d.x(x) + d.d(s) * 0.5;
        let num = div.to_string();
        let nw = b.font_width(HudFont::Glass, &num, &d.font(HudFont::Glass, big, 0.0, WHITE));
        txt(b, d, HudFont::Glass, big, 0.0, hex(if sel { G_ORANGE } else { 0xffffff }, 1.0), &num, cx - nw * 0.5, d.y(top), Some(big));
        let dw = b.font_width(HudFont::GlassMed, "DIV", &d.font(HudFont::GlassMed, 8.0, 1.2, WHITE));
        txt(b, d, HudFont::GlassMed, 8.0, 1.2, hex(G_DIM, 1.0), "DIV", cx - dw * 0.5, d.y(top + big + 2.0), Some(8.0));
    }

    /// The panel: the save's name, caption and status, the next race, the standings and the legend; the Final's two rows
    /// with the ladder and the help line.
    fn league_panel(&self, b: &mut Batch, d: Glass, l: &League, sv: &Save, current: bool) {
        let (px, py, pw, ph) = PANEL;
        let mut m = Mesh::default();
        glass_panel(&mut m, d, (d.x(px), d.y(py), d.d(pw), d.d(ph)));
        b.mesh(&m);
        // the panel's padding box origin (design px) and a point in it (canvas px)
        let (ix, iy) = (px + 1.0, py + 1.0);
        let at = |x: f32, y: f32| (d.x(ix + x), d.y(iy + y));
        let fin = sv.div == 0;
        // the name 26 px in a 32 px line, the CURRENT tag after it
        let (x, y) = at(20.0, 10.0);
        let nw = txt(b, d, HudFont::Glass, 26.0, 1.0, hex(0xffffff, 1.0), &l.name.to_uppercase(), x, y, Some(32.0));
        if current {
            tag(b, d, "CURRENT", (x + nw + d.d(12.0), y + d.d(7.0)), G_S1, 0.55);
        }
        // `.cap`: the division, when it was made, how many races it has seen
        let cap = [Some(sv.div_name()), date_text(l.create_date).map(|t| format!("CREATED {t}")), Some(format!("{} {} IN ALL", l.races_done, if l.races_done == 1 { "RACE" } else { "RACES" }))];
        let cap = cap.into_iter().flatten().collect::<Vec<_>>().join(" \u{b7} ");
        let (x, y) = at(20.0, 46.0);
        txt(b, d, HudFont::GlassMed, 11.0, 1.5, hex(G_DIM, 1.0), &cap, x, y, None);
        // the mockup's inline-flex sits on its parent's line (14 px strut): 2.5 px under its `top: 64`
        self.league_status(b, d, sv, at(20.0, 66.5));
        self.league_next(b, d, l, sv, (ix, iy));
        // the dashed rules (6 on, 4 off) at 86, and under the Final's rows
        let rule = |b: &mut Batch, y: f32| {
            let (a, z) = (at(18.0, y), at(INNER_W - 18.0, y));
            let mut m = Mesh::default();
            m.dashed([a.0, a.1], [z.0, z.1], d.d(6.0), d.d(4.0), d.w(1.0), hex(G_LINE, 0.45), d.px);
            b.mesh(&m);
        };
        rule(b, 86.0);
        let row_h = if fin { 52.0 } else { 28.0 };
        self.league_standings(b, d, l, sv, (ix, iy), row_h);
        // the legend line: the zone keys left, how far you are from the lead right
        let note = hex(NOTE, 0.75);
        let (lx, top) = at(20.0, if fin { 230.0 } else { 350.0 });
        let text_top = Self::legend_text_top(d, top);
        if fin {
            txt(b, d, HudFont::GlassMed, 11.0, 1.0, note, "ONE RACE \u{b7} THE WINNER TAKES THE LEAGUE", lx, text_top, None);
        } else {
            let keys: &[(&str, u32)] = match sv.div {
                1 => &[("TO THE FINAL", GAIN), ("GO DOWN", LOSS)],
                2 => &[("GO UP", GAIN), ("GO DOWN", LOSS)],
                _ => &[("GO UP", GAIN)],
            };
            Self::legend_keys(b, d, (lx, top), keys);
        }
        let far = if !sv.started {
            "NO RACES YET THIS SEASON".to_string()
        } else if sv.place > 0 {
            format!("{} PTS TO THE LEADER", sv.to_leader)
        } else {
            "YOU LEAD THE DIVISION".to_string()
        };
        txt_r(b, d, HudFont::GlassMed, 11.0, 1.0, note, &far, d.x(ix + INNER_W - 20.0), text_top, None);
        if fin {
            rule(b, 256.0);
            let (lx, ly) = at(168.0, 276.0);
            Self::league_load_ladder(b, d, (lx, ly), 500.0, 0, None);
            self.league_final_help(b, d, l, sv, at(20.0, 330.0));
        }
    }

    /// The text top of a legend row: its flex row is 20 high (the body's line height), the text and bars centred in it.
    pub(super) fn legend_text_top(d: Glass, top: f32) -> f32 {
        top + d.d((20.0 - line_box(HudFont::GlassMed, 11.0)) * 0.5)
    }

    /// A legend row's zone keys from canvas `(x, top)`: a 3 x 12 bar, 6 px, the label, 14 px to the next key.
    pub(super) fn legend_keys(b: &mut Batch, d: Glass, (mut lx, top): (f32, f32), keys: &[(&str, u32)]) {
        let note = hex(NOTE, 0.75);
        let text_top = Self::legend_text_top(d, top);
        for (k, (label, col)) in keys.iter().enumerate() {
            if k > 0 {
                lx += d.d(14.0);
            }
            Self::zone_bar(b, d, [lx, top + d.d((20.0 - 12.0) * 0.5)], 12.0, *col);
            lx += d.d(3.0 + 6.0);
            lx += txt(b, d, HudFont::GlassMed, 11.0, 1.0, note, label, lx, text_top, None);
        }
    }

    /// `.zone`: a 3 px bar of `h` design px with its top left at canvas `p`, glowing 6 px.
    pub(super) fn zone_bar(b: &mut Batch, d: Glass, p: [f32; 2], h: f32, col: u32) {
        let bar = vec::rounded_rect(p[0], p[1], d.d(3.0), d.d(h), d.d(1.5));
        let mut m = Mesh::default();
        m.halo(&bar, d.d(6.0), hex(col, 0.7));
        m.fill(&bar, hex(col, 1.0), d.px);
        b.mesh(&m);
    }

    /// `.status`: the dot and the words, where you stand in your division; `(x, top)` = the line box in canvas px.
    pub(super) fn league_status(&self, b: &mut Batch, d: Glass, sv: &Save, (x, top): (f32, f32)) {
        let (text, dot, glow) = match sv.tone {
            Tone::Up => (hex(GAIN, 1.0), hex(GAIN, 1.0), Some(hex(GAIN, 0.8))),
            Tone::Down => (hex(LOSS, 1.0), hex(LOSS, 1.0), Some(hex(LOSS, 0.8))),
            Tone::Final => (hex(ORANGE_TEXT, 1.0), hex(G_ORANGE, 1.0), Some(hex(G_ORANGE, 1.0))),
            Tone::Plain => (hex(G_TEXT, 1.0), hex(G_LINE, 0.5), None),
        };
        let c = [x + d.d(4.0), top + d.d(line_box(HudFont::Glass, 13.0)) * 0.5];
        let mut m = Mesh::default();
        if let Some(g) = glow {
            m.halo(&circle(c, d.d(4.0)), d.d(6.0), g);
        }
        m.fill(&circle(c, d.d(4.0)), dot, d.px);
        b.mesh(&m);
        txt(b, d, HudFont::Glass, 13.0, 1.5, text, &sv.status, x + d.d(16.0), top, None);
    }

    /// The next race, right-aligned 84 px in: its number, the track, the laps and your car with the season's dots; the
    /// track's lap beside it, 56 px at 18 px in.
    fn league_next(&self, b: &mut Batch, d: Glass, l: &League, sv: &Save, (ix, iy): (f32, f32)) {
        let codes = self.track_codes();
        let Some(t) = l.next_track(&codes) else { return };
        let tr = &self.tracks[t];
        let right = d.x(ix + INNER_W - 84.0);
        let cap = format!("NEXT \u{b7} {}", sv.race_text());
        txt_r(b, d, HudFont::GlassMed, 11.0, 1.5, hex(G_DIM, 1.0), &cap, right, d.y(iy + 12.0), None);
        // `.val-s` 2 px under the caption's line, then the note row 6 px under its 24 px line
        let name_top = iy + 12.0 + line_box(HudFont::GlassMed, 11.0) + 2.0;
        txt_r(b, d, HudFont::Glass, 20.0, 0.5, hex(0xffffff, 1.0), &tr.name.to_uppercase(), right, d.y(name_top), Some(24.0));
        let row_top = name_top + 24.0 + 6.0;
        let row_h = line_box(HudFont::GlassMed, 11.0).max(11.0);
        // the season's dots, 6 px apart, ending at `right`
        let sizes = sv.dot_sizes();
        let dots_w = sizes.iter().sum::<f32>() + 6.0 * sv.races.saturating_sub(1) as f32;
        Self::season_dots(b, d, sv, right - d.d(dots_w), d.y(row_top + row_h * 0.5));
        // `{laps} LAPS · CAR`
        let me = &l.division(sv.div)[sv.place.min(l.division(sv.div).len() - 1)];
        let laps = tr.laps;
        let note = format!("{laps} {} \u{b7} {}", if laps == 1 { "LAP" } else { "LAPS" }, car_of(me, sv.div));
        txt_r(b, d, HudFont::GlassMed, 11.0, 1.0, hex(NOTE, 0.75), &note, right - d.d(dots_w + 10.0), d.y(row_top + (row_h - line_box(HudFont::GlassMed, 11.0)) * 0.5), None);
        Self::lap_outline(b, d, tr, (ix + INNER_W - 18.0 - 28.0, iy + 12.0 + 28.0), 34.0, 1.8);
    }

    /// The season's dots from canvas `x`, centred on canvas `cy`: raced lit blue with a glow, the next one 11 px orange with
    /// a white rim, the rest dark (`.sdots`, 6 px apart).
    pub(super) fn season_dots(b: &mut Batch, d: Glass, sv: &Save, x: f32, cy: f32) {
        let mut dx = x;
        let mut m = Mesh::default();
        for (k, s) in sv.dot_sizes().iter().enumerate() {
            let c = [dx + d.d(s * 0.5), cy];
            let (fill, rim, glow) = if k < sv.raced {
                (hex(G_S1, 1.0), hex(G_S1, 1.0), Some((hex(0x4aa8ff, 0.9), 6.0)))
            } else if k == sv.next {
                (hex(G_ORANGE, 1.0), hex(0xffffff, 1.0), Some((hex(G_ORANGE, 1.0), 8.0)))
            } else {
                (hex(0x040a16, 0.6), hex(G_LINE, 0.45), None)
            };
            if let Some((g, blur)) = glow {
                m.halo(&circle(c, d.d(s * 0.5)), d.d(blur), g);
            }
            m.fill(&circle(c, d.d(s * 0.5)), fill, d.px);
            m.stroke(&circle(c, d.d(s * 0.5 - 0.5)), true, d.w(1.0), Ends::Round, rim, d.px);
            dx += d.d(s + 6.0);
        }
        b.mesh(&m);
    }

    /// A track's lap in its three sector colours (the HUD map's projection), no disc: centred on design (cx, cy), the
    /// circle round it `r` design px; the start line a small dot (`mapSVG`).
    pub(super) fn lap_outline(b: &mut Batch, d: Glass, tr: &TrackInfo, (cx, cy): (f32, f32), r: f32, sw: f32) {
        let Some(p) = tr.profile.as_ref().filter(|p| !p.outline.is_empty()) else { return };
        let line = Self::fit_lap(tr, p, [d.x(cx), d.y(cy)], d.d(r)).line(p);
        let mut m = Mesh::default();
        m.stroke(&line, true, d.w(2.0 * sw), Ends::Round, hex(G_DARK, 0.7), d.px);
        for (run, col) in sector_runs(p, &line).iter().zip([G_S1, G_S2, G_S3]).filter(|(r, _)| !r.is_empty()) {
            m.stroke(run, false, d.w(sw), Ends::Round, hex(col, 0.85), d.px);
        }
        m.fill(&circle(line[0], d.d(2.0)), hex(G_DARK, 1.0), d.px);
        m.stroke(&circle(line[0], d.d(2.0)), true, d.w(0.8), Ends::Round, hex(0xffffff, 1.0), d.px);
        b.mesh(&m);
    }

    /// The standings of the division as the save left them: the headings at 92, the rows from 112, `row_h` high, one
    /// pixel apart, ten px in from the panel's edges; your row has the blue fill, the zone bars show once the season has
    /// started.
    fn league_standings(&self, b: &mut Batch, d: Glass, l: &League, sv: &Save, (ix, iy): (f32, f32), row_h: f32) {
        let s = Standings { table: l.division(sv.div), div: sv.div, started: sv.started, dim: false, origin: (ix, iy), top: ROWS_TOP, row_h, row_w: INNER_W - 20.0, cols: LOAD_COLS, zone: league::zone };
        Self::standings(b, d, &s);
    }

    /// A division's standings table (19.3 Table, Standing row): the headings `s.top - 20` from the panel's inner top, the
    /// rows from `s.top`; the League Load panel's and the View standings card's.
    pub(super) fn standings(b: &mut Batch, d: Glass, s: &Standings) {
        let (ix, iy) = s.origin;
        let (dim, white) = (hex(G_DIM, 1.0), hex(0xffffff, 1.0));
        let c = &s.cols;
        // `.th`: left = the column's x + 10 from the panel; the column's width where it aligns right or centre
        let mut heads: Vec<(&str, f32, f32, i8)> = vec![("#", 0.0, 36.0, 0), ("DRIVER", c.nm, 120.0, -1), ("CAR", c.car, 120.0, -1)];
        for (k, t) in ["WINS", "2ND", "3RD"].into_iter().enumerate() {
            heads.push((t, c.w + c.w_step * k as f32, c.w_w, 0));
        }
        if let Some(g) = c.gap {
            heads.push(("GAP", g, GAP_W, 1));
        }
        heads.push(("POINTS", c.pts, c.pts_w, 1));
        for (t, x, w, align) in heads {
            let st = d.font(HudFont::GlassMed, 11.0, 1.5, dim);
            let tw = b.font_width(HudFont::GlassMed, t, &st);
            let left = d.x(ix + x + 10.0);
            let at = match align {
                -1 => left,
                0 => left + (d.d(w) - tw) * 0.5,
                _ => left + d.d(w) - tw,
            };
            b.font_text(HudFont::GlassMed, t, at, d.cap_top(HudFont::GlassMed, 11.0, d.y(iy + s.top - 20.0), None), &st);
        }
        let row_h = s.row_h;
        let lh = Some(row_h - 2.0);
        // the stats and the gap read dim in a season not started (`.tr.zero`)
        let stat = if s.dim { hex(0xbfe3f3, 0.35) } else { hex(0xffffff, 0.78) };
        for (i, p) in s.table.iter().enumerate() {
            let rtop = d.y(iy + s.top + (row_h + 1.0) * i as f32);
            Self::row_ground(b, d, (ix + 10.0, s.row_w, row_h), rtop, i > 0, (p.human, false, i > 0 && s.table[i - 1].human), 0.0);
            // the row's padding box
            let (ox, oy) = (d.x(ix + 11.0), rtop + d.d(1.0));
            let cx = |v: f32| ox + d.d(v);
            let cy = oy + d.d((row_h - 2.0) * 0.5);
            if s.started {
                if let Some(up) = (s.zone)(s.div, i) {
                    Self::zone_bar(b, d, [cx(4.0), oy + d.d(6.0)], row_h - 2.0 - 12.0, if up { GAIN } else { LOSS });
                }
            }
            // the place, 16 px centred in 36
            let n = (i + 1).to_string();
            let nw = b.font_width(HudFont::Glass, &n, &d.font(HudFont::Glass, 16.0, 0.5, WHITE));
            txt(b, d, HudFont::Glass, 16.0, 0.5, hex(G_TEXT, 1.0), &n, cx(18.0) - nw * 0.5, oy, lh);
            // the driver's colour (4 x 18, at most the row less 10) and the portrait 8 px after it
            let bar_h = 22f32.min(row_h - 10.0);
            let bar = vec::rounded_rect(cx(44.0), cy - d.d(bar_h * 0.5), d.d(4.0), d.d(bar_h), d.d(2.0));
            let col = p.p.colour;
            let mut m = Mesh::default();
            m.halo(&bar, d.d(6.0), rgb_hex(col, 1.0));
            m.fill(&bar, rgb_hex(col, 1.0), d.px);
            b.mesh(&m);
            let nick = p.p.nick.to_uppercase();
            let pic_h = (c.pic_w * 56.0 / 90.0).round();
            Self::results_portrait(b, d, &motorview::ui::duo_key(&p.p.picture), &nick, (cx(56.0), cy - d.d(pic_h * 0.5), d.d(c.pic_w), d.d(pic_h)));
            // the nick, YOU after yours
            let nw = txt(b, d, HudFont::Glass, c.nm_px, 0.5, if p.human { white } else { hex(G_TEXT, 1.0) }, &nick, cx(c.nm), oy, lh);
            if p.human {
                txt(b, d, HudFont::Glass, 10.0, 1.5, hex(G_S1, 1.0), "YOU", cx(c.nm) + nw + d.d(8.0), oy, lh);
            }
            txt(b, d, HudFont::GlassMed, 14.0, 1.0, dim, &car_of(p, s.div), cx(c.car), oy, lh);
            // wins, seconds and thirds: 16 px at .78, centred in their column
            for (k, v) in [p.wins, p.seconds, p.thirds].into_iter().enumerate() {
                let t = v.to_string();
                let w = b.font_width(HudFont::Glass, &t, &d.font(HudFont::Glass, 16.0, 0.5, WHITE));
                txt(b, d, HudFont::Glass, 16.0, 0.5, stat, &t, cx(c.w + c.w_step * k as f32 + c.w_w * 0.5) - w * 0.5, oy, lh);
            }
            // the points to the leader: LEVEL on a tie, a dash for the leader and before the first round
            if let Some(g) = c.gap {
                let right = cx(g + GAP_W);
                let behind = s.table[0].score.saturating_sub(p.score);
                if !s.started || i == 0 {
                    Self::dash_col(b, d, right, cy + d.d(0.5), 8.0, if s.dim { stat } else { dim });
                } else {
                    // ASCII hyphen for the mockup's minus sign: the menu font atlas has no U+2212
                    let text = if behind == 0 { "LEVEL".to_string() } else { format!("-{behind}") };
                    txt_r(b, d, HudFont::GlassMed, 15.0, 0.5, if s.dim { stat } else { dim }, &text, right, oy, lh);
                }
            }
            // the points right-aligned in their column, `PTS` 4 px after
            let right = cx(c.pts + c.pts_w);
            txt_r(b, d, HudFont::GlassMed, 11.0, 1.0, dim, "PTS", right, oy, lh);
            let pw = b.font_width(HudFont::GlassMed, "PTS", &d.font(HudFont::GlassMed, 11.0, 1.0, WHITE));
            txt_r(b, d, HudFont::Glass, 17.0, 0.5, white, &p.score.to_string(), right - pw - d.d(4.0), oy, lh);
        }
    }

    /// The Final's help line, centred: `Beat NICK on TRACK over n laps to win the league.`; `(x, top)` = the 794 px line.
    fn league_final_help(&self, b: &mut Batch, d: Glass, l: &League, sv: &Save, (x, top): (f32, f32)) {
        let codes = self.track_codes();
        let Some(t) = l.next_track(&codes) else { return };
        let tr = &self.tracks[t];
        let rival = l.division(sv.div).iter().find(|p| !p.human).map_or(String::new(), |p| p.p.nick.to_uppercase());
        let laps = tr.laps;
        let parts = [
            ("Beat ".to_string(), false),
            (rival, true),
            (format!(" on {} over {laps} {} to win the league.", tr.name.to_uppercase(), if laps == 1 { "lap" } else { "laps" }), false),
        ];
        let width = |b: &Batch, s: &str, bold: bool| {
            if bold { b.font_width(HudFont::Glass, s, &d.font(HudFont::Glass, 15.0, 0.5, WHITE)) } else { b.font_width(HudFont::GlassMed, s, &d.font(HudFont::GlassMed, 15.0, 0.5, WHITE)) }
        };
        let total: f32 = parts.iter().map(|(s, bold)| width(b, s, *bold)).sum();
        let mut px = x + (d.d(794.0) - total) * 0.5;
        for (s, bold) in &parts {
            px += if *bold {
                txt(b, d, HudFont::Glass, 15.0, 0.5, hex(0xffffff, 1.0), s, px, top, Some(20.0))
            } else {
                txt(b, d, HudFont::GlassMed, 15.0, 0.5, hex(G_TEXT, 1.0), s, px, top, Some(20.0))
            };
        }
    }

    /// `ladderHTML(div, w)`: DIV 3, DIV 2, DIV 1, FINAL on a line, the divisions behind you blue with the line lit up to
    /// yours, yours orange; `(ox, oy)` = the box's top left in canvas px, `w` design px wide.
    pub(super) fn league_load_ladder(b: &mut Batch, d: Glass, (ox, oy): (f32, f32), w: f32, div: usize, look: Option<usize>) {
        let x = |dv: usize| ox + d.d(14.0 + (w - 28.0) * (3 - dv) as f32 / 3.0);
        let y = oy + d.d(14.0);
        let mut m = Mesh::default();
        m.dashed([x(3), y], [x(0), y], d.d(4.0), d.d(4.0), d.w(2.0), hex(G_LINE, 0.25), d.px);
        if div < 3 {
            let seg = [[x(3), y], [x(div), y]];
            m.glow(&seg, false, d.d(2.0), d.d(4.0), hex(0x4aa8ff, 0.8), 3);
            m.stroke(&seg, false, d.w(2.0), Ends::Round, hex(G_S1, 1.0), d.px);
        }
        // the division the screen shows when it is not yours: a dashed ring round its step
        if let Some(lk) = look.filter(|&lk| lk != div) {
            let (c, r) = ([x(lk), y], d.d(10.0));
            let (period, on) = (d.d(4.5) / r, d.d(2.0) / r);
            let mut a = 0.0;
            while a < TAU - 1e-3 {
                m.stroke(&vec::arc(c, r, a, (a + on).min(TAU), 3), false, d.w(1.5), Ends::Butt, hex(0xffffff, 0.75), d.px);
                a += period;
            }
        }
        for dv in [3, 2, 1, 0] {
            let (on, was) = (dv == div, dv > div);
            let (c, r) = ([x(dv), y], d.d(if on { 8.0 } else { 5.0 }));
            if on {
                m.halo(&circle(c, r), d.d(8.0), hex(G_ORANGE, 0.8));
            }
            m.fill(&circle(c, r), hex(if on { G_ORANGE } else if was { G_S1 } else { 0x04070d }, 1.0), d.px);
            m.stroke(&circle(c, r), true, d.w(if on { 2.4 } else { 1.5 }), Ends::Round, if on { hex(0xffffff, 1.0) } else { hex(G_LINE, 0.6) }, d.px);
        }
        b.mesh(&m);
        for dv in [3, 2, 1, 0] {
            let label = if dv == 0 { "FINAL".to_string() } else { format!("DIV {dv}") };
            let st = d.font(HudFont::Glass, 10.0, 1.5, WHITE);
            let lw = b.font_width(HudFont::Glass, &label, &st) - st.tracking;
            let col = if dv == div { G_ORANGE } else if dv > div { G_S1 } else { G_DIM };
            txt(b, d, HudFont::Glass, 10.0, 1.5, hex(col, 1.0), &label, x(dv) - lw * 0.5, oy + d.d(28.0), None);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A league whose division `div` holds you at `place`, with these scores down the division.
    fn league(div: usize, place: usize, scores: &[u32], race_number: u32) -> League {
        let mut places = vec![Place::default(); league::PLACES];
        for (k, &s) in scores.iter().enumerate() {
            places[league::DIV_START[div] + k].score = s;
        }
        places[league::DIV_START[div] + place].human = true;
        League { name: "T".into(), file: "T".into(), create_date: 20261102, race_number, races_done: 0, laps_done: 0, season_ms: 0, season_lap_ms: 0, places }
    }

    /// Track codes in track-list order: Goldbridge 100, RedRock 101, NeoCity 200, Atlantika 201, RuhrStadt 300, Olympos 301,
    /// NolbyHills 310 (a bonus track), OkkunSpeedway 400.
    const CODES: [u32; 8] = [100, 101, 200, 201, 300, 301, 310, 400];

    #[test]
    fn the_window_puts_the_selection_on_the_fourth_card_and_never_leaves_a_blank_end() {
        assert_eq!(window_first(0, 12), 0);
        assert_eq!(window_first(3, 12), 0);
        assert_eq!(window_first(5, 12), 2);
        assert_eq!(window_first(11, 12), 4);
        assert_eq!(window_first(2, 5), 0);
        assert_eq!(window_first(0, 0), 0);
    }

    #[test]
    fn a_season_not_started_says_so_and_has_no_place_to_show() {
        let s = Save::of(&league(3, 5, &[0; 8], 0), &CODES);
        assert_eq!((s.div, s.started, s.tone), (3, false, Tone::Plain));
        assert_eq!(s.status, "SEASON NOT STARTED");
        assert_eq!((s.races, s.next), (2, 0));
    }

    #[test]
    fn the_zones_colour_the_status() {
        let up = Save::of(&league(3, 1, &[12, 10, 8, 6, 4, 3, 2, 1], 1), &CODES);
        assert_eq!((up.tone, up.status.as_str()), (Tone::Up, "IN THE PROMOTION ZONE"));
        let one = Save::of(&league(1, 0, &[12, 10, 8, 6, 4, 3, 2, 1], 3), &CODES);
        assert_eq!((one.tone, one.status.as_str()), (Tone::Up, "ON COURSE FOR THE FINAL"));
        let down = Save::of(&league(2, 7, &[12, 10, 8, 6, 4, 3, 2, 1], 2), &CODES);
        assert_eq!((down.tone, down.status.as_str(), down.to_leader), (Tone::Down, "IN THE DROP ZONE", 11));
    }

    #[test]
    fn a_place_between_the_zones_counts_the_points_to_the_last_one_that_goes_up() {
        let s = Save::of(&league(3, 2, &[12, 10, 8, 6, 4, 3, 2, 1], 1), &CODES);
        assert_eq!((s.tone, s.status.as_str()), (Tone::Plain, "2 PTS OFF PROMOTION"));
        let d1 = Save::of(&league(1, 3, &[12, 10, 8, 6, 4, 3, 2, 1], 1), &CODES);
        assert_eq!(d1.status, "6 PTS OFF THE FINAL");
        let level = Save::of(&league(3, 2, &[10, 10, 10, 6, 4, 3, 2, 1], 1), &CODES);
        assert_eq!(level.status, "LEVEL WITH 2ND PLACE");
    }

    #[test]
    fn the_final_is_one_race() {
        let s = Save::of(&league(0, 1, &[0, 0], 0), &CODES);
        assert_eq!((s.div, s.size, s.races, s.tone), (0, 2, 1, Tone::Final));
        assert_eq!(s.status, "ONE RACE FOR THE LEAGUE");
        assert_eq!((s.race_text().as_str(), s.div_name().as_str()), ("THE FINAL RACE", "THE FINAL"));
    }

    #[test]
    fn the_next_race_wraps_as_the_game_does_and_the_race_text_counts_from_one() {
        let s = Save::of(&league(2, 0, &[12, 10, 8, 6, 4, 3, 2, 1], 2), &CODES);
        assert_eq!((s.races, s.raced, s.next, s.race_text().as_str()), (4, 2, 2, "RACE 3 OF 4"));
        // a file with more races raced than the season has starts over, like `League::next_track`
        let over = Save::of(&league(3, 0, &[0; 8], 9), &CODES);
        assert_eq!((over.next, over.race_text().as_str()), (0, "RACE 1 OF 2"));
    }

    #[test]
    fn a_date_reads_year_month_day_and_a_file_without_one_has_none() {
        assert_eq!(date_text(20261102).as_deref(), Some("2026-11-02"));
        assert_eq!(date_text(0), None);
    }
}
