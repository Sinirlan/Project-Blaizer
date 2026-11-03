//! REMASTER Results, the Records tab (#94 phase 5, mockup B `recHTML`): the ranking tables the race entered
//! (`Game::results_ranks`, kept by `enter_hiscores`), ten rows each, your new row blue with a NEW tag, the built-in
//! "DI dude" rows dimmed, `#n` / NOT IN THE TOP TEN and PB over the panel. Two panels of 568 design px as the mockup;
//! a Single race enters a third table (its difficulty's), so three panels of 376 (design choice, the mockup has two).

use super::pause_settings::glass_panel;
use super::results::{tag, txt, txt_r};
use super::*;

/// The panels of `n` tables (design px): the mockup's two 568 wide, or three 376 wide on the same 1156 px band.
pub(super) fn records_rects(n: usize) -> Vec<(f32, f32, f32, f32)> {
    match n {
        0 => Vec::new(),
        3 => (0..3).map(|k| (62.0 + 390.0 * k as f32, 228.0, 376.0, 372.0)).collect(),
        _ => (0..n).map(|k| (62.0 + 588.0 * k as f32, 228.0, 568.0, 372.0)).collect(),
    }
}

/// `.tr > span` colour of a built-in row: the text colour at .45
const DEFAULT_ROW: u32 = G_TEXT;

impl Game {
    /// What the table is called over the panel: `BEST RACE · 3 LAPS · NOLBYHILLS`; with three panels the track
    /// (already in the title's code line) is left out so the tags still fit.
    fn record_caption(&self, kind: hiscore::Kind, track: &str, laps: u32, narrow: bool) -> String {
        let what = match kind {
            hiscore::Kind::BestLap => "BEST LAP".to_string(),
            hiscore::Kind::BestRace => format!("BEST RACE \u{b7} {laps} {}", if laps == 1 { "LAP" } else { "LAPS" }),
            _ => format!("{} \u{b7} {}", kind.label().to_uppercase(), difficulty_name(self.settings.difficulty).to_uppercase()),
        };
        if narrow { what } else { format!("{what} \u{b7} {track}") }
    }

    /// The Records tab: one panel per table the race entered.
    pub(super) fn records_tab(&self, b: &mut Batch, d: Glass, track: &str, laps: u32, pulse: f32) {
        let rects = records_rects(self.results_ranks.len());
        for (view, &rect) in self.results_ranks.iter().zip(&rects) {
            self.record_panel(b, d, view, rect, &self.record_caption(view.kind, track, laps, rect.2 < 500.0), pulse);
        }
    }

    fn record_panel(&self, b: &mut Batch, d: Glass, view: &hiscore::RankView, (px, py, pw, ph): (f32, f32, f32, f32), caption: &str, pulse: f32) {
        let narrow = pw < 500.0;
        let mut m = Mesh::default();
        glass_panel(&mut m, d, (d.x(px), d.y(py), d.d(pw), d.d(ph)));
        b.mesh(&m);
        let (ix, iy) = (px + 1.0, py + 1.0);
        let inner_w = pw - 2.0;
        txt(b, d, HudFont::GlassMed, 11.0, 1.5, hex(G_TEXT, 1.0), caption, d.x(ix + 20.0), d.y(iy + 12.0), None);
        // top right (`right:16px; top:9px; gap:6px`): `#n` or NOT IN THE TOP TEN, then PB
        let mut tags: Vec<(String, bool)> = vec![if view.at > 0 { (format!("#{}", view.at), true) } else { ("NOT IN THE TOP TEN".to_string(), false) }];
        if view.pb {
            tags.push(("PB".to_string(), true));
        }
        let widths: Vec<f32> = tags.iter().map(|(t, _)| Self::tag_width(b, d, t)).collect();
        let mut x = d.x(ix + inner_w - 16.0) - widths.iter().sum::<f32>() - d.d(6.0) * (tags.len() - 1) as f32;
        for ((text, orange), w) in tags.iter().zip(&widths) {
            if *orange {
                tag(b, d, text, (x, d.y(iy + 9.0)), G_ORANGE, 0.6);
            } else {
                Self::chip(b, d, text, (x, d.y(iy + 9.0)), *w, G_DIM);
            }
            x += w + d.d(6.0);
        }
        // headings (`.th`, top 34)
        let (car_head, car_at) = if narrow { (166.0, 156.0) } else { (270.0, 260.0) };
        let dim = hex(G_DIM, 1.0);
        let top = d.y(iy + 34.0);
        txt(b, d, HudFont::GlassMed, 11.0, 1.5, dim, "#", d.x(ix + 20.0), top, None);
        txt(b, d, HudFont::GlassMed, 11.0, 1.5, dim, "DRIVER", d.x(ix + 54.0), top, None);
        txt(b, d, HudFont::GlassMed, 11.0, 1.5, dim, "CAR", d.x(ix + car_head), top, None);
        txt_r(b, d, HudFont::GlassMed, 11.0, 1.5, dim, "TIME", d.x(ix + inner_w - 24.0), top, None);
        // rows (top 52 + 31 i, 30 high); a row's content box starts 11 px in (10 + the border) and is `pw - 24` wide
        let new_w = Self::tag_width(b, d, "NEW");
        let name_st = d.font(HudFont::Glass, 16.0, 0.5, WHITE);
        for (i, e) in view.rows.iter().enumerate() {
            let fresh = view.at == i + 1;
            let built_in = view.default.get(i).copied().unwrap_or(false);
            let rtop = d.y(iy + 52.0 + 31.0 * i as f32);
            Self::row_ground(b, d, (ix + 10.0, pw - 22.0, 30.0), rtop, i > 0, (fresh, false, i > 0 && view.at == i), pulse);
            let (ox, oy) = (d.x(ix + 11.0), rtop + d.d(1.0));
            let lh = Some(28.0);
            let tone = |on: u32| if built_in { hex(DEFAULT_ROW, 0.45) } else { hex(on, 1.0) };
            // #: 16 px centred in 36
            let n = (i + 1).to_string();
            let w = b.font_width(HudFont::Glass, &n, &name_st);
            txt(b, d, HudFont::Glass, 16.0, 0.5, tone(0xffffff), &n, ox + d.d(18.0) - w * 0.5, oy, lh);
            // the driver (a long name is cut so it stays clear of the car); the NEW tag only when it fits too (the row is
            // blue and the panel says `#n` anyway)
            let room = d.d(car_at - 44.0 - 8.0);
            let mut name = e.name.to_uppercase();
            while b.font_width(HudFont::Glass, &name, &name_st) > room && name.chars().count() > 1 {
                name.pop();
            }
            let nw = txt(b, d, HudFont::Glass, 16.0, 0.5, if fresh { tone(0xffffff) } else { tone(G_TEXT) }, &name, ox + d.d(44.0), oy, lh);
            if fresh && nw + d.d(10.0) + new_w <= room {
                tag(b, d, "NEW", (ox + d.d(44.0) + nw + d.d(10.0), oy + d.d(5.0)), G_ORANGE, 0.6);
            }
            txt(b, d, HudFont::GlassMed, 14.0, 1.0, tone(G_DIM), &e.car.to_uppercase(), ox + d.d(car_at), oy, lh);
            let ms = if view.lap { e.lap_ms } else { e.race_ms };
            txt_r(b, d, HudFont::Glass, 17.0, 0.5, tone(0xffffff), &glass_time(ms), ox + d.d(pw - 24.0 - 14.0), oy, lh);
        }
    }
}
