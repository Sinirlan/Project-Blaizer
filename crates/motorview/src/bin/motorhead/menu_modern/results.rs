//! REMASTER Results screen in the Glass HUD (#94, mockup B · Timing sheet of
//! `reference_materials/results-overhaul/B-timing.html`). Phase 2: the frame over the frozen, frosted finish scene,
//! the tab bar, the classification table and the selected driver's card; phase 3 adds the League tab
//! (`results_league.rs`), phase 4 the Laps and Race chart tabs (`results_laps.rs`), phase 5 the Records tab (`results_records.rs`) and the Time attack tab set (Laps + Records). Numbers come from `Game::results` (order, points, times) and `RaceSheet` (laps, grid, laps
//! led). The table is drawn here row by row (8 fixed rows, no scrolling): the same panel, rims and fonts as
//! `table.rs`, with the portrait, colour bar and blue "you" row cells that widget has no cell for. Design px
//! through `Glass`; every offset is the mockup's `clsHTML` inline style.

use super::pause_settings::glass_panel;
use super::*;
use motor_sim::sheet::CarSheet;

use super::results_league::LeagueView;
use super::results_records::records_rects;
use super::results_laps::{laps_geometry, LapsView};

/// the tab bar, the table panel and the driver card (design px)
pub(super) const TABBAR: (f32, f32, f32, f32) = (62.0, 160.0, 1156.0, 52.0);
pub(super) const TABLE: (f32, f32, f32, f32) = (62.0, 228.0, 760.0, 372.0);
pub(super) const CARD: (f32, f32, f32, f32) = (842.0, 228.0, 376.0, 372.0);
/// a row's content box: from the panel's inner left + 11, 736 wide, rows 39 high on a 41 pitch from 34
pub(super) const LAPS_PANEL: (f32, f32, f32, f32) = (62.0, 228.0, 640.0, 372.0);
pub(super) const LAPS_CARD: (f32, f32, f32, f32) = (722.0, 228.0, 496.0, 372.0);
pub(super) const CHART_PANEL: (f32, f32, f32, f32) = (62.0, 228.0, 1156.0, 372.0);
pub(super) const ROW_X: f32 = 11.0;
pub(super) const ROW_W: f32 = 736.0;
pub(super) const ROW_TOP: f32 = 34.0;
pub(super) const ROW_PITCH: f32 = 41.0;
const DASH: &str = "--:--.---";

/// `1ST`, `2ND`, `3RD`, `4TH` ...
pub(super) fn ord(n: usize) -> String {
    match n {
        1 => "1ST".into(),
        2 => "2ND".into(),
        3 => "3RD".into(),
        _ => format!("{n}TH"),
    }
}

/// A gap to the winner as the mockup's `fmtGap`: `+s.mmm`, `+m:ss.mmm` from a minute.
pub(super) fn gap_text(ms: u32) -> String {
    if ms >= 60_000 {
        format!("+{}:{:02}.{:03}", ms / 60_000, (ms / 1000) % 60, ms % 1000)
    } else {
        format!("+{}.{:03}", ms / 1000, ms % 1000)
    }
}

/// A lap without the leading zero minutes (`lapBarsHTML`'s `fmtL`): `36.094`, `1:04.871`.
pub(super) fn lap_text(ms: u32) -> String {
    if ms >= 60_000 {
        format!("{}:{:02}.{:03}", ms / 60_000, (ms / 1000) % 60, ms % 1000)
    } else {
        format!("{}.{:03}", ms / 1000, ms % 1000)
    }
}

/// The tabs of the screen, in order: a race has only those that apply (Classification always; League in a league race).
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum ResTab {
    Classification,
    Laps,
    Chart,
    Records,
    League,
}

impl ResTab {
    fn label(self) -> (&'static str, char) {
        match self {
            ResTab::Classification => ("CLASSIFICATION", ui_font::icon::LIST_ORDERED),
            ResTab::Laps => ("LAPS", ui_font::icon::TIMER),
            ResTab::Chart => ("RACE CHART", ui_font::icon::ACTIVITY),
            ResTab::Records => ("RECORDS", ui_font::icon::BAR_CHART),
            ResTab::League => ("LEAGUE", ui_font::icon::TROPHY),
        }
    }
}

/// One finisher's line, gathered once for the table and the card.
pub(super) struct ResRow<'a> {
    pub(super) pos: usize,
    pub(super) pts: Option<u32>,
    pub(super) time: Option<u32>,
    pub(super) best: Option<u32>,
    pub(super) nick: String,
    pub(super) name: String,
    pub(super) car: String,
    pub(super) colour: u32,
    pub(super) human: bool,
    pub(super) pic: String,
    pub(super) sheet: Option<&'a CarSheet>,
    /// Knockout: the lap the car was knocked out on
    pub(super) out: Option<u32>,
}

impl ResRow<'_> {
    pub(super) fn fastest(&self) -> bool {
        self.sheet.is_some_and(|s| s.fastest)
    }
}

/// Text with its CSS line box starting at canvas `top` (`lh` design px, None = normal) at canvas `x`; returns the advance.
#[allow(clippy::too_many_arguments)]
pub(super) fn txt(b: &mut Batch, d: Glass, f: HudFont, px: f32, ls: f32, col: [f32; 4], s: &str, x: f32, top: f32, lh: Option<f32>) -> f32 {
    let st = d.font(f, px, ls, col);
    b.font_text(f, s, x, d.cap_top(f, px, top, lh), &st)
}

/// `txt` ending at canvas `right` (the trailing letter-spacing counts, as CSS right alignment has it).
#[allow(clippy::too_many_arguments)]
pub(super) fn txt_r(b: &mut Batch, d: Glass, f: HudFont, px: f32, ls: f32, col: [f32; 4], s: &str, right: f32, top: f32, lh: Option<f32>) {
    let w = b.font_width(f, s, &d.font(f, px, ls, col));
    txt(b, d, f, px, ls, col, s, right - w, top, lh);
}

/// `.tag-v` / `.tag-b`: the dark tag ground with a rim and text in `col` (the rim at alpha `rim`), 18 design px high,
/// top left at canvas (x, y); returns its width.
pub(super) fn tag(b: &mut Batch, d: Glass, text: &str, (x, y): (f32, f32), col: u32, rim: f32) -> f32 {
    let w = Game::tag_width(b, d, text);
    let r = vec::rounded_rect(x, y, w, d.d(18.0), d.d(7.0));
    let mut m = Mesh::default();
    m.fill(&r, hex(0x0a1626, 1.0), d.px);
    m.stroke(&r, true, d.w(1.0), Ends::Round, hex(col, rim), d.px);
    b.mesh(&m);
    txt(b, d, HudFont::Glass, 10.0, 1.5, hex(col, 1.0), text, x + d.d(8.0), y, Some(18.0));
    w
}

/// Break `text` into at most two lines no wider than `width` canvas px (the mockup's name box wraps the same way).
fn wrap2(b: &Batch, d: Glass, text: &str, width: f32) -> Vec<String> {
    let st = d.font(HudFont::Glass, 21.0, 0.5, WHITE);
    let mut lines: Vec<String> = vec![String::new()];
    for word in text.split(' ') {
        let cur = lines.last().unwrap();
        let next = if cur.is_empty() { word.to_string() } else { format!("{cur} {word}") };
        if !cur.is_empty() && b.font_width(HudFont::Glass, &next, &st) > width && lines.len() < 2 {
            lines.push(word.to_string());
        } else {
            *lines.last_mut().unwrap() = next;
        }
    }
    lines
}

impl Game {
    /// The race track's mode name for the header (`MODE_NAMES`).
    fn results_mode(&self) -> &'static str {
        match self.game_type {
            GameType::Single => "SINGLE RACE",
            GameType::League => "LEAGUE",
            GameType::TimeAttack => "TIME ATTACK",
            GameType::Knockout => "KNOCKOUT",
            GameType::GhostMode => "GHOST RACE",
        }
    }

    /// The finishers in order with everything the screen prints about them.
    pub(super) fn results_rows(&self, r: &RaceState) -> Vec<ResRow<'_>> {
        let race = &r.sess.race;
        let time_attack = self.game_type == GameType::TimeAttack;
        self.results
            .iter()
            .take(8)
            .enumerate()
            .map(|(pos, &(slot, pts, time, best))| {
                let rc = &race.racers[slot];
                let sheet = self.results_sheet.as_ref().and_then(|s| s.cars.get(slot));
                let c = rc.driver.colour;
                ResRow {
                    pos,
                    pts: (!time_attack).then_some(pts),
                    time,
                    best,
                    nick: rc.driver.nick.to_uppercase(),
                    name: rc.driver.name.to_uppercase(),
                    car: self.cars.iter().find(|x| x.id == rc.car_id).map_or(String::new(), |x| x.name.to_uppercase()),
                    colour: (u32::from(c[0]) << 16) | (u32::from(c[1]) << 8) | u32::from(c[2]),
                    human: rc.human,
                    pic: motorview::ui::duo_key(&rc.driver.picture),
                    sheet,
                    out: rc.progress.eliminated_ms.map(|_| sheet.and_then(|s| s.out_lap).unwrap_or(rc.progress.lap)),
                }
            })
            .collect()
    }

    /// The race time of the winner, `+s.mmm` for the rest, dashes for a car without a time (Knockout, Time Attack; Single and League cars still racing get a made-up one, #286).
    pub(super) fn results_time(rows: &[ResRow], row: &ResRow) -> String {
        match (row.time, rows.first().and_then(|w| w.time)) {
            (Some(t), _) if row.pos == 0 => glass_time(t),
            (Some(t), Some(w)) => gap_text(t.saturating_sub(w)),
            (Some(t), None) => glass_time(t),
            _ => DASH.into(),
        }
    }

    /// The cards the finish scene is frosted behind (window px): the plate, the tab bar, the table and the driver card.
    pub(super) fn results_frost(&self, size: (u32, u32)) -> motorview::render::SceneFrost {
        Self::scene_frost(size, |d| {
            let mut cards = vec![(d.r(1021.0), d.t(GLASS_PLATE_TOP), d.d(234.0), d.d(119.0))];
            let table = if self.results_league_tab() { (TABLE.0, TABLE.1, TABLE.2, self.league_table_h()) } else { TABLE };
            let body: Vec<(f32, f32, f32, f32)> = match self.results_tab() {
                ResTab::Laps => vec![LAPS_PANEL, LAPS_CARD],
                ResTab::Chart => vec![CHART_PANEL],
                ResTab::Records => records_rects(self.results_ranks.len()),
                _ => vec![table, CARD],
            };
            cards.extend(std::iter::once(TABBAR).chain(body).map(|(x, y, w, h)| (d.x(x), d.y(y), d.d(w), d.d(h))));
            cards
        })
    }

    /// REMASTER Results (#94): the race scene as it stood when the results opened, frosted behind the glass, and the
    /// Classification screen over it. X (`results_watch`) takes the glass away to look at the finish.
    pub(crate) fn results_modern(&self, size: (u32, u32)) -> Vec<Layer> {
        let Some(r) = &self.race else { return Vec::new() };
        let mut parts = self.race_parts(r);
        parts.frost = (!self.results_watch).then(|| self.results_frost(size));
        let mut out = vec![Layer::Scene { cam: r.cam, parts: Box::new(parts), viewport: None, mirror: false }];
        if !self.results_watch {
            let canvas = Canvas::hud(size);
            let px = 1.0 / canvas.transform(size).0;
            out.push(self.ui_in(Batch::sharp_text(&self.assets, canvas, 1.0), |g, b| g.results_body(b, r, px)));
        }
        out
    }

    fn results_body(&self, b: &mut Batch, r: &RaceState, px: f32) {
        let d = Glass::new(&b.canvas, px);
        let pulse = 0.5 - 0.5 * (self.t * TAU / 1.1).cos();
        let tr = &self.tracks[r.track];
        let laps = r.sess.race.laps;
        // `.veil.res`: the pause veil a little lighter in the middle
        Self::glass_veil(b, d, (0.36, 0.74), 400.0);
        let mut m = Mesh::default();
        Self::glass_title_wash(&mut m, d);
        b.mesh(&m);
        let tab = self.results_tab();
        let header = self.results_header(tab);
        Self::glass_furniture(b, d, self.t, &header, "FINISH", GlassRules::None);
        let code = format!("{} \u{b7} {} {} \u{b7} {}", tr.name.to_uppercase(), laps, if laps == 1 { "LAP" } else { "LAPS" }, difficulty_name(self.settings.difficulty).to_uppercase());
        Self::glass_title(b, d, "results", &code);
        Self::glass_plate(b, d, (d.r(1021.0), d.t(GLASS_PLATE_TOP), d.d(234.0), d.d(119.0)), "187-R-12", "RESULTSCREEN");
        Self::glass_version(b, d);
        let tabs = self.results_tabs();
        let labels: Vec<(&str, char)> = tabs.iter().map(|t| t.label()).collect();
        self.glass_tabbar(b, d, &labels, tabs.iter().position(|&t| t == tab).unwrap_or(0));
        let rows = self.results_rows(r);
        match tab {
            ResTab::Classification if !rows.is_empty() => {
                let sel = self.sel.min(rows.len() - 1);
                self.results_table(b, d, &rows, sel, pulse);
                self.results_card(b, d, &rows, sel, laps);
            }
            ResTab::Classification => {}
            ResTab::Laps if !rows.is_empty() => self.laps_tab(b, d, &rows, self.sel.min(rows.len() - 1), LapsView { pulse, tr }),
            ResTab::Chart if !rows.is_empty() => self.chart_tab(b, d, &rows, self.sel.min(rows.len() - 1), tr, laps),
            ResTab::Laps | ResTab::Chart => {}
            ResTab::Records => self.records_tab(b, d, &tr.name.to_uppercase(), laps, pulse),
            ResTab::League => self.league_tab(b, d, LeagueView { pulse }),
        }
        if !self.asking() {
            let mut hints: Vec<(&[usize], &str)> = Vec::new();
            if tabs.len() > 1 {
                hints.push((&[4, 5], "TABS"));
            }
            match tab {
                ResTab::Classification => hints.push((&[9, 10], "DRIVER")),
                ResTab::Laps | ResTab::Chart if rows.len() > 1 => hints.push((&[0, 1], "DRIVER")),
                _ => {}
            }
            hints.push((&[2], if self.results_has_league() && tab != ResTab::League { "STANDINGS" } else { "CONTINUE" }));
            hints.push((&[13], "WATCH"));
            Self::glass_hints(b, d, self.controls.last, &hints, 626.0);
        }
    }

    /// The tabs the race has: Classification and Race chart need a field (a Time attack has none), Laps the race sheet,
    /// Records the tables the race entered (Knockout enters none), League a league race's standings.
    fn results_tabs(&self) -> Vec<ResTab> {
        let ta = self.game_type == GameType::TimeAttack;
        let mut tabs = Vec::new();
        if !ta {
            tabs.push(ResTab::Classification);
        }
        if self.results_sheet.is_some() {
            tabs.push(ResTab::Laps);
            if !ta && self.results.len() > 1 {
                tabs.push(ResTab::Chart);
            }
        }
        if !self.results_ranks.is_empty() {
            tabs.push(ResTab::Records);
        }
        if self.results_has_league() {
            tabs.push(ResTab::League);
        }
        if tabs.is_empty() {
            tabs.push(ResTab::Classification);
        }
        tabs
    }

    /// A league race with its standings to show: the League tab exists.
    pub(crate) fn results_has_league(&self) -> bool {
        self.game_type == GameType::League && self.league_outcome.is_some() && self.current_league().is_some()
    }

    /// The tab on show; the League results screen (`Screen::LeagueResults`) is always the League tab.
    pub(super) fn results_tab(&self) -> ResTab {
        let tabs = self.results_tabs();
        if self.screen == Screen::LeagueResults && tabs.contains(&ResTab::League) {
            return ResTab::League;
        }
        tabs[self.results_tab.min(tabs.len() - 1)]
    }

    pub(crate) fn results_league_tab(&self) -> bool {
        self.results_tab() == ResTab::League
    }

    /// Q / E (LB / RB): the next or previous tab, no wrap.
    pub(crate) fn results_tab_step(&mut self, next: bool) {
        let n = self.results_tabs().len();
        let at = self.results_tab.min(n - 1);
        let to = if next { (at + 1).min(n - 1) } else { at.saturating_sub(1) };
        if to != at {
            self.results_tab = to;
            self.results_scroll = 0;
            self.t = 0.0;
            self.ev_override = Some(ev::SELECT);
        }
    }

    /// Up / Down: the driver on Classification, the lap window on Laps (nothing on the other tabs).
    pub(crate) fn results_vertical(&mut self, down: bool) {
        let n = self.results.len().min(8);
        match self.results_tab() {
            ResTab::Classification => self.sel = if down { (self.sel + 1).min(n.saturating_sub(1)) } else { self.sel.saturating_sub(1) },
            ResTab::Laps => {
                let laps = self.results.get(self.sel).and_then(|r| self.results_sheet.as_ref()?.cars.get(r.0)).map_or(0, |c| c.lap_ms.len());
                let max = laps - laps_geometry(laps).1;
                self.results_scroll = if down { (self.results_scroll + 1).min(max) } else { self.results_scroll.saturating_sub(1) };
            }
            _ => {}
        }
    }

    /// Left / Right: the other driver on Laps and Race chart, no wrap.
    pub(crate) fn results_horizontal(&mut self, right: bool) {
        if matches!(self.results_tab(), ResTab::Laps | ResTab::Chart) {
            let n = self.results.len().min(8);
            let to = if right { (self.sel + 1).min(n.saturating_sub(1)) } else { self.sel.saturating_sub(1) };
            if to != self.sel {
                self.sel = to;
                self.results_scroll = 0;
                self.ev_override = Some(ev::SELECT);
            }
        }
    }

    /// Enter on another tab of a league race: on to the standings.
    pub(crate) fn results_goto_league(&mut self) {
        if let Some(i) = self.results_tabs().iter().position(|&t| t == ResTab::League) {
            self.results_tab = i;
            self.t = 0.0;
            self.ev_override = Some(ev::SELECT);
        }
    }

    /// The header line over the title (`MODE_NAMES`); the League tab of a season's end is the League results screen.
    fn results_header(&self, tab: ResTab) -> String {
        match tab {
            ResTab::Classification => format!("{} // RESULTS // CLASSIFICATION", self.results_mode()),
            ResTab::Laps => format!("{} // RESULTS // LAPS", self.results_mode()),
            ResTab::Chart => format!("{} // RESULTS // RACE CHART", self.results_mode()),
            ResTab::Records => format!("{} // RESULTS // RECORDS", self.results_mode()),
            ResTab::League => self.league_header(),
        }
    }

    /// `clsHTML`'s table: headings, the eight rows (your row blue, the selected one with the orange rim).
    fn results_table(&self, b: &mut Batch, d: Glass, rows: &[ResRow], sel: usize, pulse: f32) {
        let (px, py, pw, ph) = TABLE;
        let mut m = Mesh::default();
        glass_panel(&mut m, d, (d.x(px), d.y(py), d.d(pw), d.d(ph)));
        b.mesh(&m);
        let (ix, iy) = (px + 1.0, py + 1.0);
        let dim = hex(G_DIM, 1.0);
        let head = |b: &mut Batch, t: &str, x: f32, right: bool| {
            let top = d.y(iy + 12.0);
            if right {
                txt_r(b, d, HudFont::GlassMed, 11.0, 1.5, dim, t, d.x(ix + x), top, None);
            } else {
                txt(b, d, HudFont::GlassMed, 11.0, 1.5, dim, t, d.x(ix + x), top, None);
            }
        };
        head(b, "POS", 22.0, false);
        head(b, "DRIVER", 126.0, false);
        head(b, "CAR", 280.0, false);
        head(b, "TIME", 530.0, true);
        head(b, "BEST LAP", 650.0, true);
        if rows[0].pts.is_some() {
            head(b, "PTS", 738.0, true);
        }
        for (i, row) in rows.iter().enumerate() {
            let on = i == sel;
            let prev_special = i > 0 && (i - 1 == sel || rows[i - 1].human);
            let top = Self::row_chrome(b, d, (ix, iy), i, (row.human, on, prev_special), pulse);
            self.results_row(b, d, rows, row, (d.x(ix + ROW_X), top + d.d(1.0)), on);
        }
    }

    /// A table row's ground (`.tr`): the rule above it (hidden next to your row or the selected one: `quiet_above`), your
    /// row's blue fill, the selected row's orange rim. `(ix, iy)` = the panel's inner top left (design px); returns the
    /// row's top (canvas px). `flags` = (your row, selected, the row above is special).
    pub(super) fn row_chrome(b: &mut Batch, d: Glass, (ix, iy): (f32, f32), i: usize, flags: (bool, bool, bool), pulse: f32) -> f32 {
        let top = d.y(iy + ROW_TOP + ROW_PITCH * i as f32);
        Self::row_ground(b, d, (ix + 10.0, 738.0, 39.0), top, i > 0, flags, pulse);
        top
    }

    /// The ground of any row (`.tr`): `(x, w, h)` = the row's left edge, width and height (design px), `top` its top
    /// (canvas px), `rule` = a row above it (the rule between rows); `flags` as `row_chrome`.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn row_ground(b: &mut Batch, d: Glass, (x, w, h): (f32, f32, f32), top: f32, rule: bool, flags: (bool, bool, bool), pulse: f32) {
        let (human, on, quiet_above) = flags;
        let rect = (d.x(x), top, d.d(w), d.d(h));
        let mut m = Mesh::default();
        // `.tr + .tr::before`: the rule between rows
        if rule && !on && !human && !quiet_above {
            m.fill(&vec::rounded_rect(d.x(x + 9.0), top - d.d(1.0), d.d(w - 18.0), d.w(1.0), 0.0), hex(G_LINE, 0.08), d.px);
        }
        let shape = vec::rounded_rect(rect.0, rect.1, rect.2, rect.3, d.d(9.0));
        if human {
            m.halo(&shape, d.d(14.0), hex(0x50aaff, 0.18));
            m.fill(&shape, hex(G_S1, 0.10), d.px);
            m.stroke(&shape, true, d.w(1.0), Ends::Round, hex(G_S1, 0.5), d.px);
        }
        if on {
            m.halo(&shape, d.d(14.0), hex(G_ORANGE, 0.3 + 0.2 * pulse));
            m.stroke(&shape, true, d.w(1.0), Ends::Round, hex(G_ORANGE, 1.0), d.px);
        }
        b.mesh(&m);
    }

    /// One row's cells; `(ox, oy)` = the row's padding box (canvas px), 736 x 37 design px.
    fn results_row(&self, b: &mut Batch, d: Glass, rows: &[ResRow], row: &ResRow, (ox, oy): (f32, f32), on: bool) {
        let lh = Some(37.0);
        let right = |v: f32| ox + d.d(v);
        let white = hex(0xffffff, 1.0);
        // POS: 20 px, centred in 40
        let pos = (row.pos + 1).to_string();
        let w = b.font_width(HudFont::Glass, &pos, &d.font(HudFont::Glass, 20.0, 0.5, white));
        txt(b, d, HudFont::Glass, 20.0, 0.5, white, &pos, right(20.0) - w * 0.5, oy, lh);
        // the driver's colour bar and the portrait
        let cy = oy + d.d(18.5);
        let mut m = Mesh::default();
        let bar = vec::rounded_rect(right(44.0), cy - d.d(11.0), d.d(4.0), d.d(22.0), d.d(2.0));
        m.halo(&bar, d.d(6.0), hex(row.colour, 0.6));
        m.fill(&bar, hex(row.colour, 1.0), d.px);
        b.mesh(&m);
        Self::results_portrait(b, d, &row.pic, &row.nick, (right(58.0), cy - d.d(14.5), d.d(46.0), d.d(29.0)));
        // the nick (orange when selected) with the YOU tag
        let nick_col = if on { hex(G_ORANGE, 1.0) } else if row.human { white } else { hex(G_TEXT, 1.0) };
        let nw = txt(b, d, HudFont::Glass, 18.0, 0.5, nick_col, &row.nick, right(116.0), oy, lh);
        if row.human {
            txt(b, d, HudFont::Glass, 10.0, 1.5, hex(G_S1, 1.0), "YOU", right(116.0) + nw + d.d(8.0), oy, lh);
        }
        txt(b, d, HudFont::GlassMed, 14.0, 1.0, hex(G_DIM, 1.0), &row.car, right(270.0), oy, lh);
        // TIME: the winner's time or the gap; `OUT . LAP n` (Knockout) small and dim
        if let Some(lap) = row.out {
            txt_r(b, d, HudFont::Glass, 15.0, 0.5, hex(G_DIM, 1.0), &format!("OUT \u{b7} LAP {lap}"), right(ROW_W - 220.0), oy, lh);
        } else {
            txt_r(b, d, HudFont::Glass, 18.0, 0.5, white, &Self::results_time(rows, row), right(ROW_W - 220.0), oy, lh);
        }
        // BEST LAP, the race's fastest with the blue dot after it
        let best = row.best.map_or(DASH.to_string(), glass_time);
        let fast = row.fastest();
        txt_r(b, d, HudFont::GlassMed, 16.0, 0.5, if fast { white } else { hex(G_TEXT, 1.0) }, &best, right(ROW_W - 100.0), oy, lh);
        if fast {
            let c = [right(ROW_W - 100.0 + 8.5), cy];
            let mut m = Mesh::default();
            m.halo(&circle(c, d.d(2.5)), d.d(5.0), hex(0x4aa8ff, 0.9));
            m.fill(&circle(c, d.d(2.5)), hex(G_S1, 1.0), d.px);
            b.mesh(&m);
        }
        if let Some(p) = row.pts {
            txt_r(b, d, HudFont::Glass, 18.0, 0.5, white, &format!("+{p}"), right(ROW_W - 12.0), oy, lh);
        }
    }

    /// A portrait in its rounded frame (`.pic`): the duotone photo with the rim and a gloss, a dark tile with the nick's
    /// first letter when the picture is missing. `rect` in canvas px. APPROX: the mockup's edge vignette is left out.
    pub(super) fn results_portrait(b: &mut Batch, d: Glass, pic: &str, nick: &str, (x, y, w, h): (f32, f32, f32, f32)) {
        let frame = vec::rounded_rect(x, y, w, h, d.d(6.0));
        let mut m = Mesh::default();
        m.shadow(&frame, 0.0, d.d(4.0), d.d(10.0), [0.0, 0.0, 0.0, 0.45]);
        b.mesh(&m);
        let have = b.size(pic).0 > 0.0;
        if have {
            b.image_filtered(pic, x, y, w, h, WHITE);
        } else {
            let mut m = Mesh::default();
            m.fill(&frame, hex(0x0a1626, 1.0), d.px);
            b.mesh(&m);
            let letter = nick.chars().next().map_or(String::new(), |c| c.to_string());
            let lw = b.font_width(HudFont::Glass, &letter, &d.font(HudFont::Glass, 18.0, 0.0, WHITE));
            txt(b, d, HudFont::Glass, 18.0, 0.0, hex(G_DIM, 1.0), &letter, x + (w - lw) * 0.5, y, Some(29.0));
        }
        let mut m = Mesh::default();
        // the gloss: white .22 at the top left fading out by 40 % of a 140 degree gradient
        let dir = [140f32.to_radians().sin(), -140f32.to_radians().cos()];
        let len = w * dir[0].abs() + h * dir[1].abs();
        let c = [x + w * 0.5, y + h * 0.5];
        m.fill_with(&frame, &|q| {
            let t = ((q[0] - c[0]) * dir[0] + (q[1] - c[1]) * dir[1]) / len + 0.5;
            [1.0, 1.0, 1.0, 0.22 * (1.0 - t / 0.4).max(0.0)]
        }, 0.0);
        m.stroke(&frame, true, d.w(1.0), Ends::Round, hex(G_LINE, 0.35), d.px);
        b.mesh(&m);
    }

    /// The selected driver's card (`clsHTML`'s `det`): photo disc, caption, name, tags, four cells and the lap bars.
    fn results_card(&self, b: &mut Batch, d: Glass, rows: &[ResRow], sel: usize, laps: u32) {
        let row = &rows[sel];
        let (cx, cy) = (CARD.0 + 1.0, CARD.1 + 1.0);
        let at = |x: f32, y: f32| (d.x(cx + x), d.y(cy + y));
        let mut m = Mesh::default();
        glass_card(&mut m, d, (d.x(CARD.0), d.y(CARD.1), d.d(CARD.2), d.d(CARD.3)), d.d(12.0), (0.11, 0.03), 0.36, (10.0, 26.0, 0.45), 0.35, None);
        // the photo disc (r 52 at 80, 84): ticks, bezel, the picture cut to the face, its overlays, the gap
        let c = [at(80.0, 84.0).0, at(80.0, 84.0).1];
        m.ticks(c, d.d(61.0), 80, d.d(5.0), d.w(1.0), hex(G_TEXT, 0.55), -FRAC_PI_2, d.px);
        Self::glass_bezel_small(&mut m, d, c, d.d(52.0));
        b.mesh(&m);
        let (rp, f) = (d.d(40.0), d.d(80.0));
        b.clip_ellipse(c[0], c[1], rp, rp);
        // `.face img`: 160 % x 108 % of the face, shifted -30 % / -4 %, covering at 50 % 30 %
        b.image_scaled(&row.pic, c[0] - f * 0.5 - 0.3679 * f, c[1] - f * 0.5 - 0.04 * f, 1.7357 * f, 1.08 * f, WHITE);
        b.clip_none();
        let mut m = Mesh::default();
        Self::disc_overlays(&mut m, d, c, rp, 44.0, hex(G_LINE, 0.1), (0.28, 0.36), (0.45, 0.62), (36.0, 0.7));
        Self::gap_ring(&mut m, d, c, rp, d.d(46.0));
        b.mesh(&m);
        // caption, name, nick and car, the tags
        let two = row.name.chars().count() > 15;
        let (x0, w) = (d.x(cx + 156.0), d.d(206.0));
        let place = format!("{} OF {}{}", ord(row.pos + 1), rows.len(), row.pts.map_or(String::new(), |p| format!(" \u{b7} +{p} PTS")));
        txt(b, d, HudFont::GlassMed, 11.0, 1.5, hex(G_DIM, 1.0), &place, x0, d.y(cy + 30.0), None);
        for (k, line) in wrap2(b, d, &row.name, w).iter().enumerate() {
            txt(b, d, HudFont::Glass, 21.0, 0.5, hex(0xffffff, 1.0), line, x0, d.y(cy + 44.0 + 24.0 * k as f32), Some(24.0));
        }
        txt(b, d, HudFont::GlassMed, 11.0, 1.5, hex(G_DIM, 1.0), &format!("{} \u{b7} {}", row.nick, row.car), x0, d.y(cy + if two { 96.0 } else { 72.0 }), None);
        let ty = d.y(cy + if two { 114.0 } else { 92.0 });
        let mut tx = x0;
        let winner = row.pos == 0 && row.out.is_none();
        for (show, text, col, rim) in [(winner, "WINNER", G_ORANGE, 0.6), (row.human, "YOU", G_S1, 0.55), (row.fastest(), "FASTEST LAP", G_S1, 0.55)] {
            if show {
                tx += tag(b, d, text, (tx, ty), col, rim) + d.d(6.0);
            }
        }
        // the cells: gap or race time, best lap, grid to finish, laps led
        let mut m = Mesh::default();
        for y in [152.0, 256.0] {
            let (a, z) = (at(20.0, y), at(356.0, y));
            m.dashed([a.0, a.1], [z.0, z.1], d.d(6.0), d.d(4.0), d.w(1.0), hex(G_LINE, 0.45), d.px);
        }
        b.mesh(&m);
        let cell = |b: &mut Batch, x: f32, y: f32, cap: &str, val: &str| -> (f32, f32) {
            let (px, py) = at(x, y);
            txt(b, d, HudFont::GlassMed, 11.0, 1.5, hex(G_DIM, 1.0), cap, px, py, Some(14.0));
            let top = py + d.d(16.0);
            let w = txt(b, d, HudFont::Glass, 20.0, 0.5, hex(0xffffff, 1.0), val, px, top, Some(24.0));
            (px + w, d.cap_top(HudFont::Glass, 20.0, top, Some(24.0)) + d.font(HudFont::Glass, 20.0, 0.5, WHITE).size)
        };
        let (cap, val) = match (row.out, row.pos) {
            (Some(lap), _) => ("KNOCKED OUT", format!("LAP {lap} OF {laps}")),
            (None, 0) => ("RACE TIME", Self::results_time(rows, row)),
            _ => ("GAP TO WINNER", Self::results_time(rows, row)),
        };
        cell(b, 20.0, 162.0, cap, &val);
        cell(b, 196.0, 162.0, "BEST LAP", &row.best.map_or(DASH.to_string(), glass_time));
        let grid = row.sheet.map_or(0, |s| s.grid as usize);
        cell(b, 20.0, 208.0, "GRID \u{bb} FINISH", "");
        self.results_grid_finish(b, d, at(20.0, 208.0), grid, row.pos + 1);
        let led = row.sheet.map_or(0, |s| s.led);
        let (ux, base) = cell(b, 196.0, 208.0, "LAPS LED", &led.to_string());
        let unit = d.font(HudFont::GlassMed, 12.0, 1.0, hex(G_DIM, 1.0));
        b.font_text(HudFont::GlassMed, &format!("OF {laps}"), ux + d.d(6.0), d.cap_above(HudFont::GlassMed, 12.0, base), &unit);
        // the lap bars
        let (lx, ly) = at(20.0, 264.0);
        txt(b, d, HudFont::GlassMed, 11.0, 1.5, hex(G_DIM, 1.0), "LAPS", lx, ly, None);
        if let Some(s) = row.sheet {
            Self::lap_bars(b, d, &s.lap_ms, at(20.0, 280.0), (336.0, 80.0));
        }
    }

    /// `6TH > 4TH` of the grid-to-finish cell: the font has no arrow, so it is drawn.
    fn results_grid_finish(&self, b: &mut Batch, d: Glass, (x, y): (f32, f32), grid: usize, finish: usize) {
        let top = y + d.d(16.0);
        let white = hex(0xffffff, 1.0);
        let from = if grid == 0 { "--".to_string() } else { ord(grid) };
        let w = txt(b, d, HudFont::Glass, 20.0, 0.5, white, &from, x, top, Some(24.0));
        let mid = d.cap_top(HudFont::Glass, 20.0, top, Some(24.0)) + d.font(HudFont::Glass, 20.0, 0.5, WHITE).size * 0.5;
        let (a, z) = (x + w + d.d(8.0), x + w + d.d(22.0));
        let mut m = Mesh::default();
        let arrow = hex(G_TEXT, 1.0);
        m.stroke(&[[a, mid], [z, mid]], false, d.w(1.8), Ends::Round, arrow, d.px);
        m.stroke(&[[z - d.d(4.0), mid - d.d(4.0)], [z, mid], [z - d.d(4.0), mid + d.d(4.0)]], false, d.w(1.8), Ends::Round, arrow, d.px);
        b.mesh(&m);
        txt(b, d, HudFont::Glass, 20.0, 0.5, white, &ord(finish), z + d.d(8.0), top, Some(24.0));
    }

    /// `lapBarsHTML`: one bar per lap, its height the lap time against the slowest, the best lap in sector 1 blue, the
    /// time over it and `Ln` under it. `(x, y)` = the box's top left (canvas px), `(w, h)` design px; bars start at the left.
    pub(super) fn lap_bars(b: &mut Batch, d: Glass, laps: &[u32], at: (f32, f32), size: (f32, f32)) {
        Self::lap_bars_of(b, d, laps, laps.len(), at, size);
    }

    /// `lap_bars` for a race of `total` laps of which only `laps` were driven (a recording quit midway): the laps not driven
    /// are dashed stubs (`.lb.cut`).
    pub(super) fn lap_bars_of(b: &mut Batch, d: Glass, laps: &[u32], total: usize, (x, y): (f32, f32), (w, h): (f32, f32)) {
        let best = laps.iter().min().copied().unwrap_or(1);
        let hi = laps.iter().max().copied().unwrap_or(best);
        let lo = best as f32 * 0.985;
        let n = total.max(1) as f32;
        // many laps (past 12) get a hairline gap, the time over the best bar only and every few `Ln` labels
        let dense = total > 12;
        let gap = if dense { 2.0 } else if total > 6 { 6.0 } else { 10.0 };
        let every = total.div_ceil(10).max(1);
        let bw = (64.0f32).min((w - gap * (n - 1.0)) / n);
        for k in 0..total {
            let Some(&t) = laps.get(k) else {
                let bx = x + d.d((bw + gap) * k as f32);
                let (bh, bottom) = ((0.3 * (h - 34.0)).round(), y + d.d(h - 16.0));
                let mut m = Mesh::default();
                let mut dy = 0.0;
                while dy < bh {
                    let seg = 3.0f32.min(bh - dy);
                    m.fill(&vec::rounded_rect(bx, bottom - d.d(dy + seg), d.d(bw), d.d(seg), d.d(0.5)), [0.745, 0.902, 1.0, 0.18], d.px);
                    dy += 6.0;
                }
                b.mesh(&m);
                if k % every == 0 {
                    let ln = format!("L{}", k + 1);
                    let nw = b.font_width(HudFont::GlassMed, &ln, &d.font(HudFont::GlassMed, 10.0, 1.0, WHITE));
                    txt(b, d, HudFont::GlassMed, 10.0, 1.0, hex(G_DIM, 1.0), &ln, bx + d.d(bw) * 0.5 - nw * 0.5, bottom + d.d(4.0), Some(12.0));
                }
                continue;
            };
            let on = t == best;
            let bx = x + d.d((bw + gap) * k as f32);
            let f = 0.3 + 0.7 * (t as f32 - lo) / ((hi as f32 - lo).max(1.0));
            let bh = (f * (h - 34.0)).round();
            let (top, bottom) = (y + d.d(h - 16.0 - bh), y + d.d(h - 16.0));
            let bar = vec::rounded_rect(bx, top, d.d(bw), bottom - top, d.d(3.0));
            let mut m = Mesh::default();
            if on {
                m.halo(&bar, d.d(10.0), hex(0x4aa8ff, 0.6));
                m.fill_with(&bar, &|q| {
                    let u = ((q[1] - top) / (bottom - top).max(1.0)).clamp(0.0, 1.0);
                    let (a, z) = (hex(G_S1, 1.0), hex(G_S1, 0.25));
                    [a[0] + (z[0] - a[0]) * u, a[1] + (z[1] - a[1]) * u, a[2] + (z[2] - a[2]) * u, a[3] + (z[3] - a[3]) * u]
                }, d.px);
            } else {
                m.fill_with(&bar, &|q| {
                    let u = ((q[1] - top) / (bottom - top).max(1.0)).clamp(0.0, 1.0);
                    [0.745, 0.902, 1.0, 0.35 + (0.08 - 0.35) * u]
                }, d.px);
            }
            m.stroke(&[[bx + d.d(2.0), top + d.w(0.5)], [bx + d.d(bw - 2.0), top + d.w(0.5)]], false, d.w(1.0), Ends::Butt, [1.0, 1.0, 1.0, if on { 0.5 } else { 0.3 }], d.px);
            b.mesh(&m);
            let centre = bx + d.d(bw) * 0.5;
            if on || !dense {
                let label = lap_text(t);
                let lw = b.font_width(HudFont::Glass, &label, &d.font(HudFont::Glass, 12.0, 0.5, WHITE));
                txt(b, d, HudFont::Glass, 12.0, 0.5, hex(if on { 0xffffff } else { G_TEXT }, 1.0), &label, centre - lw * 0.5, top - d.d(18.0), Some(14.0));
            }
            if k % every == 0 || on {
                let ln = format!("L{}", k + 1);
                let nw = b.font_width(HudFont::GlassMed, &ln, &d.font(HudFont::GlassMed, 10.0, 1.0, WHITE));
                txt(b, d, HudFont::GlassMed, 10.0, 1.0, hex(G_DIM, 1.0), &ln, centre - nw * 0.5, bottom + d.d(4.0), Some(12.0));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gap_and_lap_text() {
        assert_eq!(gap_text(3418), "+3.418");
        assert_eq!(gap_text(62_345), "+1:02.345");
        assert_eq!(lap_text(36_094), "36.094");
        assert_eq!(lap_text(64_871), "1:04.871");
        assert_eq!(ord(4), "4TH");
        assert_eq!(ord(2), "2ND");
    }
}
