//! REMASTER Glass table (#91 phase 6, #92): the list / table component of the Glass HUD (19-glass-hud-design-language
//! 19.3). A panel with column headings and rows of cells; the selected row has the orange rim, rows past the window
//! scroll (`rows_first`), and the subtab row (`glass_subtabs`) switches between lists. Taken 1:1 from `recordsHTML` /
//! `.th` / `.tr` of `reference_materials/personal-overhaul/C-paddock.html`. Design px through `Glass`.

use super::pause_settings::{glass_panel, rows_first};
use super::*;

/// A column: its heading and the mockup's `x` (design px from the panel's inner left; the heading sits 9 px right of it,
/// the cells 1 px).
pub(super) struct TableCol {
    pub title: &'static str,
    pub x: f32,
}

/// One cell of a row. `Text` = `.tr > span` (17 px bold white), `Name` = `.tn` (the row's name, orange when selected,
/// with an optional orange tag after it), `Note` = `.tm` (14 px dim), `Bar` = `.bar` (a share 0..1 in a 150 px track).
pub(super) enum Cell {
    Empty,
    Text(String),
    Name(String, Option<&'static str>),
    Note(String),
    Bar(f32),
}

/// A row: one cell per column (a short row leaves the rest empty); `none` = `.tr.none`, every cell faded.
pub(super) struct TableRow {
    pub cells: Vec<Cell>,
    pub none: bool,
}

/// The table over a glass panel: `rect` = the panel (design px), `head_y` / `rows_y` = the heading's and the first row's
/// top from the panel's inner top, rows `row_h` high with a pixel between.
pub(super) struct GlassTable<'a> {
    pub rect: (f32, f32, f32, f32),
    pub head_y: f32,
    pub rows_y: f32,
    pub row_h: f32,
    pub cols: &'a [TableCol],
    pub rows: &'a [TableRow],
    pub sel: usize,
}

/// The track of `Cell::Bar` (design px wide, high).
const BAR_W: f32 = 150.0;
const BAR_H: f32 = 6.0;
/// The faded text of a row that has no data (`rgba(191,227,243,.35)`).
const NONE: u32 = 0xbfe3f3;

impl GlassTable<'_> {
    /// The first row shown and how many rows fit from the panel's inner top.
    fn window(&self) -> (usize, usize) {
        let heights = vec![self.row_h + 1.0; self.rows.len()];
        // down to the inner bottom edge, the last row needing no gap after it
        let avail = self.rect.3 - 1.0 - self.rows_y + 1.0;
        let first = if self.rows.is_empty() { 0 } else { rows_first(&heights, self.sel.min(self.rows.len() - 1), avail) };
        let mut used = 0.0;
        let shown = heights[first..].iter().take_while(|h| { used += **h; used <= avail + 0.01 }).count();
        (first, shown)
    }
}

impl Game {
    /// The table: panel, headings, rows (the window around the selection), the selected row's rim pulsing with
    /// `pulse`, and a scroll bar when not every row is shown.
    pub(super) fn glass_table(b: &mut Batch, d: Glass, t: &GlassTable, pulse: f32) {
        let (px, py, pw, ph) = t.rect;
        let mut m = Mesh::default();
        glass_panel(&mut m, d, (d.x(px), d.y(py), d.d(pw), d.d(ph)));
        b.mesh(&m);
        // the panel's inner box (the card's 1 px border)
        let (ix, iy) = (px + 1.0, py + 1.0);
        let th = d.font(HudFont::GlassMed, 11.0, 1.5, hex(G_DIM, 1.0));
        for c in t.cols.iter().filter(|c| !c.title.is_empty()) {
            b.font_text(HudFont::GlassMed, c.title, d.x(ix + c.x + 10.0), d.cap_top(HudFont::GlassMed, 11.0, d.y(iy + t.head_y), None), &th);
        }
        let (first, shown) = t.window();
        let sel = t.sel.min(t.rows.len().saturating_sub(1));
        let row_x = ix + 10.0;
        let row_w = pw - 2.0 - 20.0;
        for (k, row) in t.rows.iter().enumerate().skip(first).take(shown) {
            let top = iy + t.rows_y + (k - first) as f32 * (t.row_h + 1.0);
            let on = k == sel;
            let rect = (d.x(row_x), d.y(top), d.d(row_w), d.d(t.row_h));
            let mut m = Mesh::default();
            // `.tr + .tr::before`: a rule 2 px above the row, hidden next to the selected row
            if k > first && !on && k - 1 != sel {
                m.fill(&vec::rounded_rect(d.x(row_x + 9.0), d.y(top - 1.0), d.d(row_w - 18.0), d.w(1.0), 0.0), hex(G_LINE, 0.08), d.px);
            }
            if on {
                let rim = vec::rounded_rect(rect.0, rect.1, rect.2, rect.3, d.d(9.0));
                m.halo(&rim, d.d(14.0), hex(G_ORANGE, 0.3 + 0.2 * pulse));
                m.stroke(&rim, true, d.w(1.0), Ends::Round, hex(G_ORANGE, 1.0), d.px);
            }
            b.mesh(&m);
            Self::table_cells(b, d, t, row, (row_x + 1.0, top + 1.0), on);
        }
        // `.scrollbar`: 3 px at the panel's right, the thumb the share of rows shown
        if shown < t.rows.len() {
            let n = t.rows.len() as f32;
            let (y0, h) = (iy + t.rows_y, shown as f32 * (t.row_h + 1.0));
            let mut m = Mesh::default();
            m.fill(&vec::rounded_rect(d.x(px + pw - 4.0), d.y(y0), d.d(3.0), d.d(h), d.d(2.0)), hex(G_LINE, 0.12), d.px);
            m.fill(&vec::rounded_rect(d.x(px + pw - 4.0), d.y(y0 + h * first as f32 / n), d.d(3.0), d.d(h * shown as f32 / n), d.d(2.0)), hex(G_LINE, 0.6), d.px);
            b.mesh(&m);
        }
    }

    /// The cells of one row; `(x, top)` = the row's inner box (design px).
    fn table_cells(b: &mut Batch, d: Glass, t: &GlassTable, row: &TableRow, (x, top): (f32, f32), on: bool) {
        let h = t.row_h - 2.0;
        let fade = |c: u32, a: f32| if row.none { hex(NONE, 0.35) } else { hex(c, a) };
        let mid = |f: HudFont, px: f32| d.cap_top(f, px, d.y(top), Some(h));
        for (col, cell) in t.cols.iter().zip(&row.cells) {
            let cx = d.x(x + col.x - 10.0);
            match cell {
                Cell::Empty => {}
                Cell::Text(s) => {
                    b.font_text(HudFont::Glass, s, cx, mid(HudFont::Glass, 17.0), &d.font(HudFont::Glass, 17.0, 0.5, fade(0xffffff, 1.0)));
                }
                Cell::Name(s, tag) => {
                    // the selected name is orange even on a faded row (`.tr.sel > .tn`)
                    let c = if on { hex(G_ORANGE, 1.0) } else { fade(G_TEXT, 1.0) };
                    let st = d.font(HudFont::Glass, 17.0, 0.5, c);
                    let w = b.font_width(HudFont::Glass, s, &st);
                    b.font_text(HudFont::Glass, s, cx, mid(HudFont::Glass, 17.0), &st);
                    if let Some(tag) = tag {
                        b.font_text(HudFont::Glass, tag, cx + w + d.d(8.0), mid(HudFont::Glass, 10.0), &d.font(HudFont::Glass, 10.0, 1.5, fade(G_ORANGE, 1.0)));
                    }
                }
                Cell::Note(s) => {
                    b.font_text(HudFont::GlassMed, s, cx, mid(HudFont::GlassMed, 14.0), &d.font(HudFont::GlassMed, 14.0, 1.0, fade(G_DIM, 1.0)));
                }
                Cell::Bar(share) => {
                    let by = d.y(top + (h - BAR_H) * 0.5);
                    let mut m = Mesh::default();
                    m.fill(&vec::rounded_rect(cx, by, d.d(BAR_W), d.d(BAR_H), d.d(3.0)), hex(G_LINE, 0.12), d.px);
                    // a sliver stays visible (the track clips the fill: `overflow: hidden`, so no glow outside)
                    if *share > 0.0 {
                        let w = (BAR_W * share.min(1.0)).max(2.0);
                        m.fill(&vec::rounded_rect(cx, by, d.d(w), d.d(BAR_H), d.d(3.0)), hex(if on { G_ORANGE } else { G_S1 }, 1.0), d.px);
                    }
                    b.mesh(&m);
                }
            }
        }
    }

    /// `.subtabs`: the lists a table switches between, from (x, y) (design px, the row's top): a triangle, the names,
    /// a triangle, 14 px apart; the shown list white with the glowing orange underline, a triangle with nowhere to go
    /// dimmed. Returns nothing: Left / Right switch the list.
    pub(super) fn glass_subtabs(b: &mut Batch, d: Glass, (x, y): (f32, f32), names: &[&str], on: usize) {
        let mid = d.y(y + 14.0);
        let mut m = Mesh::default();
        let mut x = d.x(x);
        Self::glass_tri(&mut m, d, x, mid, true, false, 1.0, on == 0);
        x += d.d(9.0 + 14.0);
        let mut runs = Vec::new();
        for (i, name) in names.iter().enumerate() {
            let st = d.font(HudFont::Glass, 16.0, 1.0, hex(if i == on { 0xffffff } else { G_DIM }, 1.0));
            let w = b.font_width(HudFont::Glass, name, &st);
            if i == on {
                let line = vec::rounded_rect(x, mid + d.d(13.0), w, d.d(2.0), d.d(1.0));
                m.halo(&line, d.d(8.0), hex(G_ORANGE, 0.5));
                m.fill(&line, hex(G_ORANGE, 1.0), d.px);
            }
            runs.push((*name, x, st));
            x += w + d.d(14.0);
        }
        Self::glass_tri(&mut m, d, x, mid, false, false, 1.0, on + 1 >= names.len());
        b.mesh(&m);
        for (name, x, st) in &runs {
            b.font_text(HudFont::Glass, name, *x, d.cap_top(HudFont::Glass, 16.0, mid - d.d(14.0), Some(28.0)), st);
        }
    }
}
