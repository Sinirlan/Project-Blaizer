//! REMASTER Results, the Laps and Race chart tabs (#94 phase 4, mockup B `lapsHTML` / `chartHTML` /
//! `posChartSVG` of `reference_materials/results-overhaul/B-timing.html`). Laps: the selected driver's laps with
//! sector times, lap time, gap to the best lap and place, the race's best sectors, and a card with the map in its
//! three sectors, the driver's best sectors, the theoretical best and the lap bars. Race chart: every car's place by
//! lap. Numbers from `RaceSheet` (`Game::results_sheet`). Design px through `Glass`; offsets are the mockup's inline
//! styles (a row's `right: N` is measured from its padding box, a heading's from the panel's: the 1 px between them
//! is the mockup's too).

use super::pause_settings::glass_panel;
use super::results::{lap_text, ord, tag, txt, txt_r, ResRow, CHART_PANEL, LAPS_CARD, LAPS_PANEL};
use super::*;

/// What the Laps tab needs from the screen's frame.
pub(super) struct LapsView<'a> {
    pub(super) pulse: f32,
    pub(super) tr: &'a TrackInfo,
}

/// The three sector colours (19.2) in sector order.
const SC: [u32; 3] = [G_S1, G_S2, G_S3];
/// the table's rows start here (panel px) and have this much height in all
const ROWS_TOP: f32 = 60.0;
const ROWS_H: f32 = 222.0;
/// the right edges of the table's columns (the mockup's `x`)
const COL_S: [f32; 3] = [150.0, 250.0, 350.0];
const COL_LAP: f32 = 480.0;
const COL_DELTA: f32 = 572.0;
const COL_POS: f32 = 618.0;

/// The row height and how many rows show for `n` laps (a window with the scroll bar when not all fit).
pub(super) fn laps_geometry(n: usize) -> (f32, usize) {
    let h = ((ROWS_H / n.max(1) as f32).floor() - 1.0).clamp(18.0, 30.0);
    (h, ((ROWS_H / (h + 1.0)).floor() as usize).min(n))
}

/// `fmtS`: seconds and thousandths without the minutes (`22.175`, `75.123`).
fn sec_text(ms: u32) -> String {
    format!("{}.{:03}", ms / 1000, ms % 1000)
}

/// `fmtD`: a difference with its sign (`+0.586`, `-0.190`; `0.000` for none).
fn delta_text(ms: i64) -> String {
    match ms {
        0 => "0.000".into(),
        _ => format!("{}{}", if ms > 0 { '+' } else { '-' }, sec_text(ms.unsigned_abs() as u32)),
    }
}

/// A font's normal line height (canvas px) for a CSS size.
fn line_h(d: Glass, f: HudFont, px: f32) -> f32 {
    let face = ui_font::face(f);
    (face.ascender - face.descender) * px * d.s / ui_font::SRC
}

/// A driver's best sectors: time and the lap (1-based) it was set on.
type BestSectors = [Option<(u32, usize)>; 3];

/// The best of each sector over the laps that have them, and the lap it was set on (1-based).
fn best_sectors(sectors: &[Option<[u32; 3]>]) -> BestSectors {
    let mut best: BestSectors = [None; 3];
    for (k, s) in sectors.iter().enumerate() {
        for (j, &t) in s.iter().flatten().enumerate() {
            if best[j].is_none_or(|(b, _)| t < b) {
                best[j] = Some((t, k + 1));
            }
        }
    }
    best
}

/// A filled rounded swatch (`.sw8`): 8 design px, the sector colour, an optional glow.
fn swatch(m: &mut Mesh, d: Glass, (x, y): (f32, f32), col: u32, glow: bool) {
    let r = vec::rounded_rect(x, y, d.d(8.0), d.d(8.0), d.d(2.0));
    if glow {
        m.halo(&r, d.d(6.0), hex(col, 0.7));
    }
    m.fill(&r, hex(col, 1.0), d.px);
}

impl Game {
    /// The driver selector of the header (`TRI_L`, colour bar, nick, place and car, YOU, `TRI_R`): `at` = (left or right
    /// edge in canvas px, line centre), `after` = the caption to put after the car (the chart's grid-to-finish).
    /// `right_aligned` places it so it ends at `at.0`. Returns the left edge.
    fn results_selector(b: &mut Batch, d: Glass, row: &ResRow, sel: (usize, usize), (x, mid): (f32, f32), right_aligned: bool, caption: &str) -> f32 {
        let many = sel.1 > 1;
        let gap = d.d(12.0);
        let (nick, cap, you) = (d.font(HudFont::Glass, 17.0, 1.0, WHITE), d.font(HudFont::GlassMed, 11.0, 1.5, hex(G_DIM, 1.0)), d.font(HudFont::Glass, 10.0, 1.5, hex(G_S1, 1.0)));
        let cap_text = if caption.is_empty() { format!("{} \u{b7} {}", ord(row.pos + 1), row.car) } else { caption.to_string() };
        let mut widths = vec![d.d(4.0), b.font_width(HudFont::Glass, &row.nick, &nick), b.font_width(HudFont::GlassMed, &cap_text, &cap)];
        if row.human {
            widths.push(b.font_width(HudFont::Glass, "YOU", &you));
        }
        if many {
            widths.insert(0, d.d(9.0));
            widths.push(d.d(9.0));
        }
        let total: f32 = widths.iter().sum::<f32>() + gap * (widths.len() - 1) as f32;
        let mut x = if right_aligned { x - total } else { x };
        let left = x;
        let top = mid - d.d(11.0);
        let mut m = Mesh::default();
        if many {
            Self::glass_tri(&mut m, d, x, mid, true, false, 1.0, sel.0 == 0);
            x += d.d(9.0) + gap;
        }
        let bar = vec::rounded_rect(x, mid - d.d(8.0), d.d(4.0), d.d(16.0), d.d(2.0));
        m.halo(&bar, d.d(6.0), hex(row.colour, 0.6));
        m.fill(&bar, hex(row.colour, 1.0), d.px);
        b.mesh(&m);
        x += d.d(4.0) + gap;
        let tx = |b: &mut Batch, f: HudFont, px: f32, st: &FontStyle, s: &str, x: f32| b.font_text(f, s, x, d.cap_top(f, px, top, Some(22.0)), st);
        x += tx(b, HudFont::Glass, 17.0, &nick, &row.nick, x) + gap;
        x += tx(b, HudFont::GlassMed, 11.0, &cap, &cap_text, x) + gap;
        if row.human {
            x += tx(b, HudFont::Glass, 10.0, &you, "YOU", x) + gap;
        }
        if many {
            let mut m = Mesh::default();
            Self::glass_tri(&mut m, d, x, mid, false, false, 1.0, sel.0 + 1 == sel.1);
            b.mesh(&m);
        }
        left
    }

    /// The Laps tab: the lap table with the race's bests under it, and the map / sector bests / lap bars card.
    pub(super) fn laps_tab(&self, b: &mut Batch, d: Glass, rows: &[ResRow], sel: usize, v: LapsView) {
        let row = &rows[sel];
        let (lap_ms, sectors): (&[u32], &[Option<[u32; 3]>]) = row.sheet.map_or((&[], &[]), |s| (&s.lap_ms, &s.sector_ms));
        let (n, many) = (lap_ms.len(), rows.len() > 1);
        let best = lap_ms.iter().copied().min();
        let best_s = best_sectors(sectors);
        let (px, py, pw, ph) = LAPS_PANEL;
        let (ix, iy) = (px + 1.0, py + 1.0);
        let (white, dim, text) = (hex(0xffffff, 1.0), hex(G_DIM, 1.0), hex(G_TEXT, 1.0));
        let note = hex(0x8fb5c9, 0.75);
        let mut m = Mesh::default();
        glass_panel(&mut m, d, (d.x(px), d.y(py), d.d(pw), d.d(ph)));
        b.mesh(&m);
        Self::results_selector(b, d, row, (sel, rows.len()), (d.x(ix + 18.0), d.y(iy + 21.0)), false, "");
        // headings (`.th`, top 40): the sector columns carry their swatch, DELTA its triangle
        let head_top = d.y(iy + 40.0);
        let st = d.font(HudFont::GlassMed, 11.0, 1.5, dim);
        let base = d.cap_top(HudFont::GlassMed, 11.0, head_top, None) + st.size;
        txt(b, d, HudFont::GlassMed, 11.0, 1.5, dim, "LAP", d.x(ix + 22.0), head_top, None);
        let mut m = Mesh::default();
        for (j, name) in ["S1", "S2", "S3"].into_iter().enumerate() {
            let right = d.x(ix + COL_S[j] + 8.0);
            let w = b.font_width(HudFont::GlassMed, name, &st);
            swatch(&mut m, d, (right - w - d.d(14.0), base - d.d(8.0)), SC[j], false);
            txt_r(b, d, HudFont::GlassMed, 11.0, 1.5, dim, name, right, head_top, None);
        }
        let right = d.x(ix + COL_DELTA + 8.0);
        let w = b.font_width(HudFont::GlassMed, "BEST", &st);
        let tri = [[right - w - d.d(11.0), base], [right - w - d.d(7.0), base - d.d(8.0)], [right - w - d.d(3.0), base]];
        m.stroke(&tri, true, d.w(1.2), Ends::Round, dim, d.px);
        b.mesh(&m);
        txt_r(b, d, HudFont::GlassMed, 11.0, 1.5, dim, "BEST", right, head_top, None);
        txt_r(b, d, HudFont::GlassMed, 11.0, 1.5, dim, "LAP TIME", d.x(ix + COL_LAP + 8.0), head_top, None);
        txt_r(b, d, HudFont::GlassMed, 11.0, 1.5, dim, "POS", d.x(ix + COL_POS + 8.0), head_top, None);
        // the rows: height from the lap count, a window when more laps than fit
        let (h, vis) = laps_geometry(n);
        let first = self.results_scroll.min(n - vis);
        if n == 0 {
            txt(b, d, HudFont::GlassMed, 11.0, 1.5, dim, "NO COMPLETED LAP", d.x(ix + 22.0), d.y(iy + ROWS_TOP + 8.0), None);
        }
        for (kk, k) in (first..first + vis).enumerate() {
            let is_best = Some(lap_ms[k]) == best;
            let prev_best = kk > 0 && Some(lap_ms[k - 1]) == best;
            let top = d.y(iy + ROWS_TOP + kk as f32 * (h + 1.0));
            Self::row_ground(b, d, (ix + 10.0, pw - 22.0, h), top, kk > 0, (is_best, false, prev_best), v.pulse);
            let (ox, oy, lh) = (d.x(ix + 11.0), top + d.d(1.0), Some(h - 2.0));
            let mid = oy + d.d((h - 2.0) * 0.5);
            txt(b, d, HudFont::Glass, 16.0, 0.5, text, &(k + 1).to_string(), ox + d.d(12.0), oy, lh);
            for j in 0..3 {
                let edge = d.x(ix + COL_S[j] + 7.0);
                let Some(t) = sectors.get(k).copied().flatten().map(|s| s[j]) else {
                    txt_r(b, d, HudFont::GlassMed, 16.0, 0.5, hex(G_TEXT, 0.35), "--.---", edge, oy, lh);
                    continue;
                };
                if best_s[j].is_some_and(|(bt, _)| bt == t) {
                    // `.sec.best::after`: the 5 px blue dot at the column's edge, the text shifted by it
                    txt_r(b, d, HudFont::GlassMed, 16.0, 0.5, white, &sec_text(t), edge - d.d(11.0), oy, lh);
                    let c = [edge - d.d(2.5), mid];
                    let mut m = Mesh::default();
                    m.halo(&circle(c, d.d(2.5)), d.d(5.0), hex(0x4aa8ff, 0.9));
                    m.fill(&circle(c, d.d(2.5)), hex(G_S1, 1.0), d.px);
                    b.mesh(&m);
                } else {
                    txt_r(b, d, HudFont::GlassMed, 16.0, 0.5, text, &sec_text(t), edge, oy, lh);
                }
            }
            let t = lap_ms[k];
            let edge = d.x(ix + COL_LAP + 7.0);
            let label = lap_text(t);
            txt_r(b, d, HudFont::Glass, 17.0, 0.5, white, &label, edge, oy, lh);
            if is_best {
                let lw = b.font_width(HudFont::Glass, &label, &d.font(HudFont::Glass, 17.0, 0.5, WHITE));
                let tw = Self::tag_width(b, d, "BEST");
                tag(b, d, "BEST", (edge - lw - d.d(8.0) - tw, mid - d.d(9.0)), G_S1, 0.55);
            }
            let edge = d.x(ix + COL_DELTA + 7.0);
            if is_best {
                let mut m = Mesh::default();
                m.stroke(&[[edge - d.d(9.0), mid], [edge - d.d(1.0), mid]], false, d.w(1.6), Ends::Round, hex(G_S1, 1.0), d.px);
                b.mesh(&m);
            } else {
                let delta = delta_text(i64::from(t) - i64::from(best.unwrap_or(t)));
                txt_r(b, d, HudFont::GlassMed, 15.0, 0.5, dim, &delta, edge, oy, lh);
            }
            if let Some(p) = row.sheet.and_then(|s| s.pos_at.get(k + 1)) {
                txt_r(b, d, HudFont::Glass, 16.0, 0.5, white, &p.to_string(), d.x(ix + COL_POS + 7.0), oy, lh);
            }
        }
        if vis < n {
            // `.scrollbar` as table.rs: 3 px at the panel's right, the thumb the share of rows shown
            let (y0, hh) = (iy + ROWS_TOP, vis as f32 * (h + 1.0));
            let mut m = Mesh::default();
            m.fill(&vec::rounded_rect(d.x(px + pw - 4.0), d.y(y0), d.d(3.0), d.d(hh), d.d(2.0)), hex(G_LINE, 0.12), d.px);
            m.fill(&vec::rounded_rect(d.x(px + pw - 4.0), d.y(y0 + hh * first as f32 / n as f32), d.d(3.0), d.d(hh * vis as f32 / n as f32), d.d(2.0)), hex(G_LINE, 0.6), d.px);
            b.mesh(&m);
        }
        if many {
            self.laps_race_best(b, d, rows, row, (ix, iy));
        }
        self.laps_card(b, d, &v, (lap_ms, best, best_s), note);
    }

    /// Under the table: the race's best sectors and lap with who set them, and the sentence about the shown driver's best.
    fn laps_race_best(&self, b: &mut Batch, d: Glass, rows: &[ResRow], row: &ResRow, (ix, iy): (f32, f32)) {
        let (white, dim) = (hex(0xffffff, 1.0), hex(G_DIM, 1.0));
        let note = hex(0x8fb5c9, 0.75);
        let mut m = Mesh::default();
        let (a, z) = ((d.x(ix + 20.0), d.y(iy + 290.0)), (d.x(ix + 620.0), d.y(iy + 290.0)));
        m.dashed([a.0, a.1], [z.0, z.1], d.d(6.0), d.d(4.0), d.w(1.0), hex(G_LINE, 0.45), d.px);
        b.mesh(&m);
        txt(b, d, HudFont::GlassMed, 11.0, 1.5, dim, "RACE BEST", d.x(ix + 22.0), d.y(iy + 300.0), None);
        // who set what: the first of the finishing order on a tie
        let mut who: [Option<(u32, usize)>; 3] = [None; 3];
        for (i, r) in rows.iter().enumerate() {
            let best = r.sheet.map_or([None; 3], |s| best_sectors(&s.sector_ms));
            for j in 0..3 {
                if let Some((t, _)) = best[j] {
                    if who[j].is_none_or(|(bt, _)| t < bt) {
                        who[j] = Some((t, i));
                    }
                }
            }
        }
        let note_top = d.y(iy + 296.0) + line_h(d, HudFont::Glass, 16.0);
        let me = rows.iter().position(|r| r.pic == row.pic).unwrap_or(0);
        for (j, w) in who.iter().enumerate() {
            let Some((t, i)) = *w else { continue };
            let right = d.x(ix + COL_S[j] + 8.0);
            txt_r(b, d, HudFont::Glass, 16.0, 0.0, white, &sec_text(t), right, d.y(iy + 296.0), None);
            let col = if i == me { hex(G_S1, 1.0) } else { note };
            txt_r(b, d, HudFont::GlassMed, 11.0, 1.0, col, &rows[i].nick, right, note_top, None);
        }
        let fastest = rows.iter().position(|r| r.fastest());
        let Some(f) = fastest else { return };
        let right = d.x(ix + COL_LAP + 8.0);
        let ft = rows[f].best.unwrap_or(0);
        txt_r(b, d, HudFont::Glass, 17.0, 0.0, white, &lap_text(ft), right, d.y(iy + 296.0), None);
        let note_top = d.y(iy + 296.0) + line_h(d, HudFont::Glass, 17.0);
        txt_r(b, d, HudFont::GlassMed, 11.0, 1.0, if f == me { hex(G_S1, 1.0) } else { note }, &rows[f].nick, right, note_top, None);
        let sentence = match (f == me, row.best) {
            (true, _) => format!("{} THE FASTEST LAP OF THE RACE", if row.human { "YOU HOLD".to_string() } else { format!("{} HOLDS", row.nick) }),
            (false, Some(t)) => format!("{} BEST LAP: {} ON THE FASTEST", if row.human { "YOUR".to_string() } else { format!("{}'S", row.nick) }, delta_text(i64::from(t) - i64::from(ft))),
            (false, None) => String::new(),
        };
        txt(b, d, HudFont::GlassMed, 11.0, 1.5, dim, &sentence, d.x(ix + 22.0), d.y(iy + 336.0), None);
    }

    /// The card beside the table: the map in its sectors, the best three sectors, the theoretical best, the lap bars.
    fn laps_card(&self, b: &mut Batch, d: Glass, v: &LapsView, (lap_ms, best, best_s): (&[u32], Option<u32>, BestSectors), note: [f32; 4]) {
        let (cx, cy) = (LAPS_CARD.0 + 1.0, LAPS_CARD.1 + 1.0);
        let at = |x: f32, y: f32| (d.x(cx + x), d.y(cy + y));
        let (white, dim) = (hex(0xffffff, 1.0), hex(G_DIM, 1.0));
        let mut m = Mesh::default();
        glass_card(&mut m, d, (d.x(LAPS_CARD.0), d.y(LAPS_CARD.1), d.d(LAPS_CARD.2), d.d(LAPS_CARD.3)), d.d(12.0), (0.11, 0.03), 0.36, (10.0, 26.0, 0.45), 0.35, None);
        b.mesh(&m);
        Self::glass_map(b, d, v.tr, &MapDisc { sectors: true, ..MapDisc::small((cx + 18.0 + 84.0, cy + 18.0 + 84.0), 84.0) }, &[]);
        let (x, y) = at(206.0, 16.0);
        txt(b, d, HudFont::GlassMed, 11.0, 1.5, dim, &format!("SECTOR BESTS \u{b7} {}", v.tr.name.to_uppercase()), x, y, None);
        for (j, bs) in best_s.iter().enumerate() {
            let (x, y) = at(206.0, 38.0 + j as f32 * 30.0);
            let mid = y + d.d(12.0);
            let mut m = Mesh::default();
            swatch(&mut m, d, (x, mid - d.d(4.0)), SC[j], true);
            b.mesh(&m);
            txt(b, d, HudFont::GlassMed, 11.0, 1.5, dim, &format!("S{}", j + 1), x + d.d(18.0), y, Some(24.0));
            let (time, lap) = bs.map_or(("--.---".to_string(), String::new()), |(t, l)| (sec_text(t), format!("LAP {l}")));
            txt(b, d, HudFont::Glass, 20.0, 0.5, white, &time, x + d.d(18.0 + 24.0 + 10.0), y, Some(24.0));
            txt_r(b, d, HudFont::GlassMed, 11.0, 1.0, note, &lap, x + d.d(272.0), y, Some(24.0));
        }
        let rule = |b: &mut Batch, (x0, y0): (f32, f32), w: f32| {
            let (a, z) = (at(x0, y0), at(x0 + w, y0));
            let mut m = Mesh::default();
            m.dashed([a.0, a.1], [z.0, z.1], d.d(6.0), d.d(4.0), d.w(1.0), hex(G_LINE, 0.45), d.px);
            b.mesh(&m);
        };
        rule(b, (206.0, 134.0), 272.0);
        let (x, y) = at(206.0, 144.0);
        txt(b, d, HudFont::GlassMed, 11.0, 1.5, dim, "THEORETICAL BEST", x, y, None);
        let theory = best_s.iter().map(|s| s.map(|(t, _)| t)).sum::<Option<u32>>();
        let (x, y) = at(206.0, 158.0);
        txt(b, d, HudFont::Glass, 28.0, 1.0, white, &theory.map_or("--:--.---".to_string(), glass_time), x, y, Some(32.0));
        if let (Some(t), Some(bl)) = (theory, best) {
            let (x, y) = at(206.0, 192.0);
            txt(b, d, HudFont::GlassMed, 11.0, 1.0, note, &format!("{} ON THE BEST LAP \u{b7} THE THREE BEST SECTORS", delta_text(i64::from(t) - i64::from(bl))), x, y, None);
        }
        if lap_ms.is_empty() {
            return;
        }
        rule(b, (18.0, 214.0), 460.0);
        let avg = (lap_ms.iter().map(|&t| u64::from(t)).sum::<u64>() as f64 / lap_ms.len() as f64).round() as u32;
        let (x, y) = at(18.0, 222.0);
        txt(b, d, HudFont::GlassMed, 11.0, 1.5, dim, &format!("LAP TIMES \u{b7} AVERAGE {}", lap_text(avg)), x, y, None);
        Self::lap_bars(b, d, lap_ms, at(18.0, 240.0), (460.0, 118.0));
    }

    /// The Race chart tab: position by lap, one line per car in its colour, the shown driver's lit.
    pub(super) fn chart_tab(&self, b: &mut Batch, d: Glass, rows: &[ResRow], sel: usize, tr: &TrackInfo, laps: u32) {
        let (px, py, pw, ph) = CHART_PANEL;
        let (ix, iy) = (px + 1.0, py + 1.0);
        let (dim, white) = (hex(G_DIM, 1.0), hex(0xffffff, 1.0));
        let mut m = Mesh::default();
        glass_panel(&mut m, d, (d.x(px), d.y(py), d.d(pw), d.d(ph)));
        b.mesh(&m);
        txt(b, d, HudFont::GlassMed, 11.0, 1.5, dim, &format!("POSITION BY LAP \u{b7} {} \u{b7} {} {}", tr.name.to_uppercase(), laps, if laps == 1 { "LAP" } else { "LAPS" }), d.x(ix + 20.0), d.y(iy + 14.0), None);
        let row = &rows[sel];
        let sheet = row.sheet;
        let grid = sheet.map_or(0, |s| s.grid as usize);
        let end = match row.out {
            Some(l) => format!("OUT ON LAP {l}"),
            None => ord(row.pos + 1),
        };
        let led = sheet.map_or(0, |s| s.led);
        let caption = format!("GRID {} \u{bb} {}{}", if grid == 0 { "--".to_string() } else { ord(grid) }, end, if led > 0 { format!(" \u{b7} LED {led}") } else { String::new() });
        Self::results_selector(b, d, row, (sel, rows.len()), (d.x(ix + 1156.0 - 2.0 - 20.0), d.y(iy + 9.0 + 12.0)), true, &caption);
        // the chart box (`posChartSVG`, 1116 x 316 at 20, 44): grid on the left, the nicks on the right
        let (w, h, l, t, r, bt) = (1116.0f32, 316.0f32, 34.0f32, 12.0f32, 92.0f32, 26.0f32);
        let (ox, oy) = (d.x(ix + 20.0), d.y(iy + 44.0));
        let n = rows.len();
        // the laps the race got to: all of them, fewer when a Knockout ended early
        let raced = rows.iter().filter_map(|r| r.sheet).map(|s| s.pos_at.len().saturating_sub(1)).max().unwrap_or(0);
        let laps = (raced as u32).clamp(1, laps.max(1));
        let lap_n = laps as f32;
        let xk = |k: usize| ox + d.d(l + (w - l - r) * k as f32 / lap_n);
        let yp = |p: usize| oy + d.d(t + (h - t - bt) * (p as f32 - 1.0) / (n.max(2) - 1) as f32);
        let st = d.font(HudFont::GlassMed, 11.0, 0.0, dim);
        let mut m = Mesh::default();
        for p in 1..=n {
            m.stroke(&[[xk(0), yp(p)], [ox + d.d(w - r), yp(p)]], false, d.w(1.0), Ends::Butt, hex(G_LINE, 0.08), d.px);
        }
        b.mesh(&m);
        for p in 1..=n {
            let label = p.to_string();
            let lw = b.font_width(HudFont::GlassMed, &label, &st);
            b.font_text(HudFont::GlassMed, &label, ox + d.d(l - 12.0) - lw, d.cap_above(HudFont::GlassMed, 11.0, yp(p) + d.d(4.0)), &st);
        }
        // lap labels, thinned when the laps would crowd them (28 design px apart at least)
        let step = ((lap_n * 28.0 / (w - l - r)).ceil() as usize).max(1);
        let lst = d.font(HudFont::GlassMed, 11.0, 1.0, dim);
        for k in (0..=laps as usize).filter(|k| k % step == 0 || *k == laps as usize) {
            let label = if k == 0 { "GRID".to_string() } else { k.to_string() };
            let lw = b.font_width(HudFont::GlassMed, &label, &lst) - lst.tracking;
            b.font_text(HudFont::GlassMed, &label, xk(k) - lw * 0.5, d.cap_above(HudFont::GlassMed, 11.0, oy + d.d(h - 6.0)), &lst);
        }
        // the lines: the others, your own, the shown driver last (on top)
        let mut order: Vec<usize> = (0..n).collect();
        order.sort_by_key(|&i| (i == sel, rows[i].human));
        for &i in &order {
            let c = &rows[i];
            let Some(s) = c.sheet else { continue };
            let on = i == sel;
            let pts: Vec<[f32; 2]> = s.pos_at.iter().enumerate().map(|(k, &p)| [xk(k), yp(usize::from(p))]).collect();
            let col = c.colour;
            let mut m = Mesh::default();
            if on {
                m.glow(&pts, false, d.d(3.5), d.d(6.0), hex(col, 0.6), 3);
            }
            m.stroke(&pts, false, d.w(if on { 3.5 } else { 2.0 }), Ends::Round, hex(col, if on { 1.0 } else { 0.42 }), d.px);
            b.mesh(&m);
            let Some(&last) = pts.last() else { continue };
            let mut m = Mesh::default();
            let dot = circle(last, d.d(if on { 4.5 } else { 3.0 }));
            m.fill(&dot, hex(col, 1.0), d.px);
            m.stroke(&dot, true, d.w(1.2), Ends::Round, hex(G_DARK, 1.0), d.px);
            b.mesh(&m);
            if c.out.is_some() {
                let ost = d.font(HudFont::Glass, 9.0, 1.0, dim);
                b.font_text(HudFont::Glass, "OUT", last[0] + d.d(6.0), d.cap_above(HudFont::Glass, 9.0, last[1] - d.d(6.0)), &ost);
            }
        }
        // the nicks at the finishing places
        for (i, c) in rows.iter().enumerate() {
            let on = i == sel;
            let col = if on { white } else { hex(G_TEXT, 0.75) };
            let nst = d.font(HudFont::Glass, 12.0, 0.5, col);
            b.font_text(HudFont::Glass, &c.nick, ox + d.d(w - r + 14.0), d.cap_above(HudFont::Glass, 12.0, yp(c.pos + 1) + d.d(4.0)), &nst);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn texts_and_windows() {
        assert_eq!(sec_text(22_175), "22.175");
        assert_eq!(sec_text(75_123), "75.123");
        assert_eq!(delta_text(586), "+0.586");
        assert_eq!(delta_text(-190), "-0.190");
        assert_eq!(delta_text(0), "0.000");
        // 3 laps: full height rows; 10 laps all fit; 25 laps a window of 11 rows at the minimum height
        assert_eq!(laps_geometry(3), (30.0, 3));
        assert_eq!(laps_geometry(10), (21.0, 10));
        assert_eq!(laps_geometry(25), (18.0, 11));
        assert_eq!(laps_geometry(0).1, 0);
    }

    #[test]
    fn best_sectors_skip_missing_laps() {
        let s = [Some([10, 20, 30]), None, Some([9, 21, 29]), Some([9, 19, 31])];
        assert_eq!(best_sectors(&s), [Some((9, 3)), Some((19, 4)), Some((29, 3))]);
        assert_eq!(best_sectors(&[None]), [None; 3]);
    }
}
