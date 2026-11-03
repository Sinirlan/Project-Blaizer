//! REMASTER Rankings (#93, concept C · Paddock of `reference_materials/rankings-overhaul/C-paddock.html`): four tabs for
//! the table kinds (Q / E), the circuits as cards down the left (your place under the tab's filters; the league tab
//! lists its three division tables), and the top-ten table with the laps and difficulty filters in its header. The
//! tables are the game's `.MHI` files (`Game::rank_table`, `rank_kind`); the filters are `RankView`. Design px through
//! `Glass`, 1:1 from the mockup's inline styles.

use super::pause_settings::glass_panel;
use super::personal::sel_rim;
use super::results::{txt, txt_r};
use super::*;
use crate::hiscore::Entry;

/// The tabs: name, icon and the rule of the table kind (the footer sentence), in `RankView::kind` order.
const KINDS: [(&str, char, &str); 4] = [
    ("BEST LAP", ui_font::icon::TIMER, "Your fastest lap on the circuit from any race, finished or not. Laps and difficulty do not count."),
    ("BEST RACE", ui_font::icon::FLAG, "The fastest finished races over this many laps, in any mode and at any difficulty."),
    ("SINGLE RACE", ui_font::icon::CAR, "Finished single races only, over this many laps at this difficulty."),
    ("LEAGUE", ui_font::icon::TROPHY, "Season results: points first, then the summed race time of the season. Written when a season ends."),
];

/// The difficulty list: ours starts at Beginner and rises (the game's ring starts at Easy) [design choice].
const DIFFICULTIES: [Difficulty; 4] = [Difficulty::Beginner, Difficulty::Easy, Difficulty::Medium, Difficulty::Hard];

/// The league tab's cards: name and what the table holds (`RANK_DIVISIONS` order).
const DIVISION_CARDS: [(&str, &str); 3] = [("DIVISION 3", "SEASONS IN DIVISION 3"), ("DIVISION 2 & 3", "SEASONS IN DIVISION 2 OR 3"), ("TOTAL", "EVERY SEASON, ANY DIVISION")];

/// A circuit's slot card: 300 x 44 at x 62, 46 apart from y 228. The league's tall card: 300 x 120, 126 apart.
const SLOT: (f32, f32, f32, f32) = (62.0, 228.0, 300.0, 44.0);
const SLOT_STEP: f32 = 46.0;
const TALL: (f32, f32, f32, f32) = (62.0, 228.0, 300.0, 120.0);
const TALL_STEP: f32 = 126.0;
/// The table panel, and the first row's top, a row's height, from the panel's inner top.
const PANEL: (f32, f32, f32, f32) = (382.0, 228.0, 836.0, 372.0);
const ROWS_TOP: f32 = 64.0;
const ROW_H: f32 = 29.0;

/// Your standing in one table: whether any run is in it (else it is the built-in rows), your best row with its place,
/// and which rows are built in.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct RankSlot {
    pub real: bool,
    /// 0-based row of your best entry and the entry
    pub best: Option<(usize, Entry)>,
    pub built_in: Vec<bool>,
}

/// `rows` against the built-in `defaults`; `me` tells your name.
pub(crate) fn rank_slot(rows: &[Entry], defaults: &[Entry], me: &dyn Fn(&str) -> bool) -> RankSlot {
    let built_in: Vec<bool> = rows.iter().map(|e| defaults.contains(e)).collect();
    let best = rows.iter().zip(&built_in).position(|(e, &d)| !d && me(&e.name)).map(|i| (i, rows[i].clone()));
    RankSlot { real: built_in.iter().any(|&d| !d), best, built_in }
}

/// `m:ss.mmm`, `h:mm:ss.mmm` from an hour on.
pub(crate) fn rank_time(ms: u32) -> String {
    let (h, m, s) = (ms / 3_600_000, ms / 60_000 % 60, ms / 1000 % 60);
    if h > 0 { format!("{h}:{m:02}:{s:02}.{:03}", ms % 1000) } else { format!("{m}:{s:02}.{:03}", ms % 1000) }
}

/// What a key cap of the filters shows.
#[derive(Clone, Copy)]
enum Cap {
    Left,
    Right,
    /// the letter on the keyboard, the pad glyph kind otherwise
    Key(&'static str, usize),
}

impl Game {
    /// Whether table row `name` is you: the saved name (a headless shot counts only with `--hiscores`).
    fn rank_is_me(&self, name: &str) -> bool {
        (self.persist || self.hiscore_dir.is_some()) && name.eq_ignore_ascii_case(&self.settings.player_name)
    }

    /// Your standing in every circuit's table (every division's, in the league tab) under the current filters.
    pub(crate) fn rank_summaries(&self) -> Vec<RankSlot> {
        let v = self.rank;
        let one = |kind: hiscore::Kind, track: &str| {
            let (t, d) = self.rank_table_of(kind, track, RANK_LAPS[v.laps], v.diff);
            rank_slot(&t.entries, &d.entries, &|n| self.rank_is_me(n))
        };
        if v.kind == RANK_LEAGUE {
            RANK_DIVISIONS.iter().map(|&k| one(k, "")).collect()
        } else {
            let kind = Self::rank_kind(&v);
            self.tracks.iter().map(|t| one(kind, &t.name)).collect()
        }
    }

    /// The circuits in the mockup's order, TrackN.trk 1 to 8 (the game's own list runs by division): indices into `tracks`.
    fn rank_order(tracks: &[TrackInfo]) -> Vec<usize> {
        let mut order: Vec<usize> = (0..tracks.len()).collect();
        order.sort_by_key(|&i| tracks[i].n);
        order
    }

    /// Q / E (LB / RB): the next or previous table kind; the lists stop at the ends.
    pub(crate) fn rank_tab_step(&mut self, next: bool) {
        let at = self.rank.kind;
        let to = if next { (at + 1).min(RANK_LEAGUE) } else { at.saturating_sub(1) };
        self.rank_changed(to != at, |g| g.rank.kind = to);
    }

    /// Z / C (LT / RT): the next or previous difficulty, where the kind uses it.
    pub(crate) fn rank_diff_step(&mut self, next: bool) {
        if !self.rank_rows().contains(&3) {
            return;
        }
        let at = DIFFICULTIES.iter().position(|&d| d == self.rank.diff).unwrap_or(1);
        let to = if next { (at + 1).min(DIFFICULTIES.len() - 1) } else { at.saturating_sub(1) };
        self.rank_changed(to != at, |g| g.rank.diff = DIFFICULTIES[to]);
    }

    /// Apply a filter change and reload the tables.
    fn rank_changed(&mut self, changed: bool, set: impl FnOnce(&mut Self)) {
        if !changed {
            return;
        }
        set(self);
        self.ev_override = Some(ev::SELECT);
        self.load_rank_table();
    }

    /// The keys of the modern Rankings screen (Q / E and Z / C are taken in `key_inner`): up / down = circuit
    /// (division in the league tab), left / right = laps where the kind uses them, back = the main menu.
    pub(crate) fn rankings_key_modern(&mut self, up: bool, down: bool, left: bool, right: bool, leave: bool) {
        let v = &mut self.rank;
        let before = *v;
        let clamp = |at: usize, n: usize, fwd: bool| if fwd { (at + 1).min(n - 1) } else { at.saturating_sub(1) };
        if up || down {
            if v.kind == RANK_LEAGUE {
                v.division = clamp(v.division, RANK_DIVISIONS.len(), down);
            } else {
                // the cards run in track-file order: up / down walk that list
                let order = Self::rank_order(&self.tracks);
                let at = order.iter().position(|&t| t == v.track).unwrap_or(0);
                v.track = order[clamp(at, order.len(), down)];
            }
        }
        if (left || right) && matches!(v.kind, 1 | 2) {
            v.laps = clamp(v.laps, RANK_LAPS.len(), right);
        }
        let moved = (v.track, v.division, v.laps) != (before.track, before.division, before.laps);
        if moved {
            self.load_rank_table();
        }
        if leave {
            self.go(Screen::Main);
            self.sel = 2; // rankings
            self.snap_needle();
        }
    }

    /// The cards the hologram is frosted behind: the plate, the tab bar, the slots, the table.
    fn rankings_cards(&self, d: Glass) -> Vec<(f32, f32, f32, f32, f32)> {
        let plate = (d.r(1021.0), d.t(GLASS_PLATE_TOP), d.d(234.0), d.d(119.0));
        let league = self.rank.kind == RANK_LEAGUE;
        let slots: Vec<(f32, f32, f32, f32)> = if league {
            (0..RANK_DIVISIONS.len()).map(|k| (TALL.0, TALL.1 + TALL_STEP * k as f32, TALL.2, TALL.3)).collect()
        } else {
            (0..self.tracks.len()).map(|k| (SLOT.0, SLOT.1 + SLOT_STEP * k as f32, SLOT.2, SLOT.3)).collect()
        };
        let rect = |(x, y, w, h): (f32, f32, f32, f32)| (d.x(x), d.y(y), d.d(w), d.d(h));
        [plate, (d.x(62.0), d.y(160.0), d.d(1156.0), d.d(52.0))]
            .into_iter()
            .chain(slots.into_iter().chain([PANEL]).map(rect))
            .map(|(x, y, w, h)| (x, y, w, h, d.d(12.0)))
            .collect()
    }

    /// REMASTER Rankings screen (#93): the Glass frame, the tab bar, the circuit cards and the table over the hologram.
    pub(crate) fn rankings_modern(&self, size: (u32, u32)) -> Vec<Layer> {
        let canvas = Canvas::hud(size);
        let px = 1.0 / canvas.transform(size).0;
        let d = Glass::new(&canvas, px);
        let kind = self.rank.kind.min(KINDS.len() - 1);
        let cards = self.rankings_cards(d);
        let back = self.ui_in(Batch::sharp_text(&self.assets, canvas, 1.0), |_, b| Self::glass_backdrop(b, d));
        let front = self.ui_in(Batch::sharp_text(&self.assets, canvas, 1.0), |g, b| {
            let pulse = 0.5 - 0.5 * (g.t * TAU / 1.1).cos();
            Self::glass_overlays(b, d);
            Self::glass_furniture(b, d, g.t, &format!("MAIN // RANKINGS // {}", KINDS[kind].0), "LIVE", GlassRules::None);
            // "rankings" at 41 px as "personal" (19.3: titles wider than 410 design px are set smaller)
            Self::glass_title_sized(b, d, "rankings", "88-137:59", Some((41.0, 47.0)));
            Self::glass_plate(b, d, (d.r(1021.0), d.t(GLASS_PLATE_TOP), d.d(234.0), d.d(119.0)), "187-C-35", "RANKINGSCREEN");
            Self::glass_version(b, d);
            let tabs: Vec<(&str, char)> = KINDS.iter().map(|k| (k.0, k.1)).collect();
            g.glass_tabbar(b, d, &tabs, kind);
            g.rank_slots_view(b, d, pulse);
            g.rank_table_view(b, d, pulse);
            g.rank_hints(b, d);
        });
        let mut out = vec![back];
        out.extend(self.hologram(size, None, &cards));
        out.push(front);
        out
    }

    /// The hints: the keys of the tab's filters, the ones it does not use dimmed.
    fn rank_hints(&self, b: &mut Batch, d: Glass) {
        let used = self.rank_rows();
        let league = self.rank.kind == RANK_LEAGUE;
        let items: [(&[usize], &str); 5] = [
            (&[4, 5], "TYPE"),
            (&[9, 10], if league { "DIVISION" } else { "TRACK" }),
            (&[0, 1], "LAPS"),
            (&[14, 15], "DIFFICULTY"),
            (&[3], "BACK"),
        ];
        let off: Vec<usize> = [(2, 2), (3, 3)].iter().filter(|(_, row)| !used.contains(row)).map(|(i, _)| *i).collect();
        Self::glass_hints_off(b, d, self.controls.last, &items, (640.0, 626.0), (1.0, 28.0), &off);
    }

    // ------------------------------------------------------------------------------------------ the cards

    /// The circuit cards (the division cards in the league tab).
    fn rank_slots_view(&self, b: &mut Batch, d: Glass, pulse: f32) {
        let v = self.rank;
        if v.kind == RANK_LEAGUE {
            for k in 0..RANK_DIVISIONS.len() {
                self.division_card(b, d, k, k == v.division, self.rank_slots.get(k), pulse);
            }
        } else {
            for (pos, &k) in Self::rank_order(&self.tracks).iter().enumerate() {
                self.circuit_card(b, d, (pos, k), k == v.track, self.rank_slots.get(k), pulse);
            }
        }
    }

    /// One glass card (the tab bar's fill) with the selection's rim.
    pub(super) fn slot_ground(b: &mut Batch, d: Glass, (x, y, w, h): (f32, f32, f32, f32), sel: bool, pulse: f32) {
        let rect = (d.x(x), d.y(y), d.d(w), d.d(h));
        let mut m = Mesh::default();
        glass_card(&mut m, d, rect, d.d(12.0), (0.13, 0.04), 0.36, (10.0, 26.0, 0.45), 0.35, None);
        if sel {
            sel_rim(&mut m, d, rect, pulse, 1.0);
        }
        b.mesh(&m);
    }

    /// `.slot`: the lap in a 36 px square, the name, your best time and car (or why there is none), your place.
    /// `(pos, k)` = its place down the list and the track.
    fn circuit_card(&self, b: &mut Batch, d: Glass, (pos, k): (usize, usize), sel: bool, slot: Option<&RankSlot>, pulse: f32) {
        let (x, y) = (SLOT.0, SLOT.1 + SLOT_STEP * pos as f32);
        Self::slot_ground(b, d, (x, y, SLOT.2, SLOT.3), sel, pulse);
        let tr = &self.tracks[k];
        // the map (`.mini` at 14, 4; 36 px): lit in the sector colours when selected, mono otherwise
        if let Some(p) = tr.profile.as_ref().filter(|p| !p.outline.is_empty()) {
            let line = Self::fit_lap(tr, p, [d.x(x + 1.0 + 14.0 + 18.0), d.y(y + 1.0 + 4.0 + 18.0)], d.d(22.0)).line(p);
            let mut m = Mesh::default();
            m.stroke(&line, true, d.w(3.2), Ends::Round, hex(G_DARK, 0.7), d.px);
            if sel {
                for (run, col) in sector_runs(p, &line).iter().zip([G_S1, G_S2, G_S3]).filter(|(r, _)| !r.is_empty()) {
                    m.stroke(run, false, d.w(1.6), Ends::Round, hex(col, 0.85), d.px);
                }
            } else {
                m.stroke(&line, true, d.w(1.6), Ends::Round, hex(G_TEXT, 0.55), d.px);
            }
            b.mesh(&m);
        }
        let (tx, ox) = (d.x(x + 1.0 + 62.0), d.y(y + 1.0));
        let name_col = hex(if sel { G_ORANGE } else { G_TEXT }, 1.0);
        txt(b, d, HudFont::Glass, 18.0, 1.0, name_col, &tr.name.to_uppercase(), tx, ox + d.d(5.0), None);
        // `.s` at 26: a best time in white, then ` · CAR`
        let sub_top = ox + d.d(26.0);
        let place = slot.and_then(|s| s.best.as_ref());
        match (slot.is_some_and(|s| s.real), place) {
            (false, _) => {
                txt(b, d, HudFont::GlassMed, 13.0, 0.5, hex(G_DIM, 1.0), "NO RUNS YET", tx, sub_top, None);
            }
            (true, None) => {
                txt(b, d, HudFont::GlassMed, 13.0, 0.5, hex(G_DIM, 1.0), "NOT IN THE TOP TEN", tx, sub_top, None);
            }
            (true, Some((_, e))) => {
                let w = txt(b, d, HudFont::Glass, 13.0, 0.5, hex(0xffffff, 1.0), &self.rank_value(e), tx, sub_top, None);
                txt(b, d, HudFont::GlassMed, 13.0, 0.5, hex(G_DIM, 1.0), &format!(" \u{b7} {}", e.car.to_uppercase()), tx + w, sub_top, None);
            }
        }
        // `.pn` (right 14, centred in the card): `#n`, or a dim dash
        let right = d.x(x + SLOT.2 - 1.0 - 14.0);
        let lh = Some(SLOT.3 - 2.0);
        match place {
            Some((i, _)) => txt_r(b, d, HudFont::Glass, 22.0, 0.0, hex(0xffffff, 1.0), &format!("#{}", i + 1), right, ox, lh),
            None => Self::dash(b, d, right, ox + d.d(SLOT.3 * 0.5 - 1.0), 9.0),
        }
    }

    /// `.slot.tall`: a division's table, the trophy, what the table holds, your best season and place.
    fn division_card(&self, b: &mut Batch, d: Glass, k: usize, sel: bool, slot: Option<&RankSlot>, pulse: f32) {
        let (x, y) = (TALL.0, TALL.1 + TALL_STEP * k as f32);
        Self::slot_ground(b, d, (x, y, TALL.2, TALL.3), sel, pulse);
        let (name, what) = DIVISION_CARDS[k];
        Self::glass_icon(b, ui_font::icon::TROPHY, d.x(x + 1.0 + 12.0), d.y(y + 1.0 + 34.0), d.d(40.0), hex(if sel { G_ORANGE } else { G_DIM }, 1.0));
        let (tx, oy) = (d.x(x + 1.0 + 62.0), d.y(y + 1.0));
        txt(b, d, HudFont::Glass, 22.0, 1.0, hex(if sel { G_ORANGE } else { G_TEXT }, 1.0), name, tx, oy + d.d(14.0), None);
        txt(b, d, HudFont::GlassMed, 13.0, 0.5, hex(G_DIM, 1.0), what, tx, oy + d.d(44.0), None);
        let place = slot.and_then(|s| s.best.as_ref());
        let line = match (slot.is_some_and(|s| s.real), place) {
            (false, _) => "NO SEASONS YET".to_string(),
            (true, None) => "NOT IN THE TOP TEN".to_string(),
            (true, Some((_, e))) => format!("YOUR BEST {} \u{b7} {}", self.rank_value(e), e.car.to_uppercase()),
        };
        txt(b, d, HudFont::GlassMed, 13.0, 0.5, hex(G_TEXT, 1.0), &line, tx, oy + d.d(66.0), None);
        let right = d.x(x + TALL.2 - 1.0 - 14.0);
        match place {
            Some((i, _)) => txt_r(b, d, HudFont::Glass, 30.0, 0.0, hex(0xffffff, 1.0), &format!("#{}", i + 1), right, oy + d.d(12.0), None),
            None => Self::dash(b, d, right, oy + d.d(12.0 + 21.0), 16.0),
        }
    }

    /// The dim dash of a card with no place (the font has no em dash): `w` design px wide, ending at canvas `right`.
    pub(super) fn dash(b: &mut Batch, d: Glass, right: f32, y: f32, w: f32) {
        Self::dash_col(b, d, right, y, w, hex(0xbfe3f3, 0.35));
    }

    /// `dash` in any colour.
    pub(super) fn dash_col(b: &mut Batch, d: Glass, right: f32, y: f32, w: f32, col: [f32; 4]) {
        let mut m = Mesh::default();
        m.fill(&vec::rounded_rect(right - d.d(w), y, d.d(w), d.d(1.5), d.d(0.75)), col, d.px);
        b.mesh(&m);
    }

    /// The value a table ranks by, as words: the lap or race time, or the season's points.
    fn rank_value(&self, e: &Entry) -> String {
        match self.rank.kind {
            0 => rank_time(e.lap_ms),
            RANK_LEAGUE => format!("{} POINTS", e.score),
            _ => rank_time(e.race_ms),
        }
    }

    // ------------------------------------------------------------------------------------------ the table

    /// The table panel: the filters in its header, the headings, ten rows and the footer.
    fn rank_table_view(&self, b: &mut Batch, d: Glass, pulse: f32) {
        let v = self.rank;
        let (px, py, pw, ph) = PANEL;
        let mut m = Mesh::default();
        glass_panel(&mut m, d, (d.x(px), d.y(py), d.d(pw), d.d(ph)));
        b.mesh(&m);
        let (ix, iy) = (px + 1.0, py + 1.0);
        // the filters (`.fgroup` at y 8): laps on left / right, difficulty on Z / C; the unused ones dimmed with ANY
        let used = self.rank_rows();
        let laps: Vec<String> = RANK_LAPS.iter().map(u32::to_string).collect();
        let diffs: Vec<String> = DIFFICULTIES.iter().map(|&x| difficulty_name(x).to_uppercase()).collect();
        let diff_at = DIFFICULTIES.iter().position(|&x| x == v.diff).unwrap_or(1);
        self.filter_group(b, d, (ix + 18.0, iy + 8.0), "LAPS", (Cap::Left, Cap::Right), &laps, v.laps, used.contains(&2));
        // 24 px left of the mockup's 396, so the ANY of an unused group stays inside the panel [design choice]
        self.filter_group(b, d, (ix + 372.0, iy + 8.0), "DIFFICULTY", (Cap::Key("Z", 14), Cap::Key("C", 15)), &diffs, diff_at, used.contains(&3));
        let mut m = Mesh::default();
        let mut x = ix + 18.0;
        while x < ix + pw - 2.0 - 18.0 {
            m.fill(&vec::rounded_rect(d.x(x), d.y(iy + 40.0), d.d(6.0).min(d.d(ix + pw - 20.0 - x)), d.w(1.0), 0.0), hex(G_LINE, 0.45), d.px);
            x += 10.0;
        }
        b.mesh(&m);
        let Some(t) = &self.rank_table else { return };
        let league = v.kind == RANK_LEAGUE;
        let lap_tab = v.kind == 0;
        let slot = self.rank_slots.get(if league { v.division } else { v.track });
        let cols = Self::rank_columns(league, lap_tab);
        // headings (`.th` at 48): left = column x + 10 from the panel's inner left
        let head = |s: &str, c: &Col, key: bool| {
            let col = hex(if key { G_TEXT } else { G_DIM }, 1.0);
            let top = d.y(iy + 48.0);
            let st = d.font(HudFont::GlassMed, 11.0, 1.5, col);
            let w = b.font_width(HudFont::GlassMed, s, &st);
            let left = d.x(ix + 10.0 + c.x);
            let at = match c.align {
                Side::Left => left,
                Side::Right => left + d.d(c.w) - w,
                Side::Centre => left + (d.d(c.w) - w) * 0.5,
            };
            (s.to_string(), at, top, st)
        };
        let heads: Vec<(String, f32, f32, FontStyle)> = cols.iter().map(|c| head(c.head, c, c.key)).collect();
        for (s, at, top, st) in &heads {
            b.font_text(HudFont::GlassMed, s, *at, d.cap_top(HudFont::GlassMed, 11.0, *top, None), st);
        }
        let best = slot.and_then(|s| s.best.as_ref()).map(|(i, _)| *i);
        let name_st = d.font(HudFont::Glass, 17.0, 0.5, WHITE);
        let you_w = Self::tag_width(b, d, "YOU") - d.d(16.0) + d.d(8.0);
        let top_row = t.entries.first();
        for (i, e) in t.entries.iter().enumerate() {
            let rtop = d.y(iy + ROWS_TOP + (ROW_H + 1.0) * i as f32);
            let built_in = slot.and_then(|s| s.built_in.get(i)).copied().unwrap_or(false);
            // your rows: any of your entries that is not built in
            let mine = !built_in && self.rank_is_me(&e.name);
            let is_best = best == Some(i);
            Self::row_ground(b, d, (ix + 10.0, pw - 22.0, ROW_H), rtop, i > 0, (is_best, false, best.is_some_and(|x| x + 1 == i)), pulse);
            if mine && !is_best {
                // `.tr.you::after`: a sector-1 bar at the left edge
                let bar = vec::rounded_rect(d.x(ix + 10.0 + 1.0 + 3.0), rtop + d.d(1.0 + 7.0), d.d(2.0), d.d(ROW_H - 2.0 - 14.0), d.d(1.0));
                let mut m = Mesh::default();
                m.halo(&bar, d.d(6.0), hex(0x4aa8ff, 0.5));
                m.fill(&bar, hex(G_S1, 1.0), d.px);
                b.mesh(&m);
            }
            let lh = Some(ROW_H - 2.0);
            let oy = rtop + d.d(1.0);
            let cx = |c: &Col| d.x(ix + 10.0 + 1.0 + c.x);
            let tone = |on: u32, a: f32| if built_in { hex(G_TEXT, 0.45) } else { hex(on, a) };
            // # (centred in 40)
            let n = (i + 1).to_string();
            let nw = b.font_width(HudFont::Glass, &n, &d.font(HudFont::Glass, 16.0, 0.5, WHITE));
            txt(b, d, HudFont::Glass, 16.0, 0.5, tone(G_TEXT, 1.0), &n, cx(&cols[0]) + (d.d(cols[0].w) - nw) * 0.5, oy, lh);
            // the driver, cut to stay clear of the car; YOU after your best row's name
            let mut name = e.name.to_uppercase();
            let room = d.d(cols[1].w) - if is_best { you_w } else { 0.0 };
            while b.font_width(HudFont::Glass, &name, &name_st) > room && name.chars().count() > 1 {
                name.pop();
            }
            let nw = txt(b, d, HudFont::Glass, 17.0, 0.5, tone(if mine { 0xffffff } else { G_TEXT }, 1.0), &name, cx(&cols[1]), oy, lh);
            if is_best {
                txt(b, d, HudFont::Glass, 10.0, 1.5, hex(G_S1, 1.0), "YOU", cx(&cols[1]) + nw + d.d(8.0), oy, lh);
            }
            txt(b, d, HudFont::GlassMed, 14.0, 1.0, tone(G_DIM, 1.0), &e.car.to_uppercase(), cx(&cols[2]), oy, lh);
            let right = |c: &Col| cx(c) + d.d(c.w);
            // the ranking value (right, 18 px white); the league's points carry a small PTS
            let mc = &cols[3];
            if league {
                let unit = d.font(HudFont::Glass, 12.0, 1.0, tone(G_DIM, 1.0));
                let uw = b.font_width(HudFont::Glass, "PTS", &unit);
                txt_r(b, d, HudFont::Glass, 12.0, 1.0, tone(G_DIM, 1.0), "PTS", right(mc), oy, lh);
                txt_r(b, d, HudFont::Glass, 18.0, 0.5, tone(0xffffff, 1.0), &e.score.to_string(), right(mc) - uw - d.d(6.0), oy, lh);
            } else {
                let ms = if lap_tab { e.lap_ms } else { e.race_ms };
                txt_r(b, d, HudFont::Glass, 18.0, 0.5, tone(0xffffff, 1.0), &rank_time(ms), right(mc), oy, lh);
            }
            if !lap_tab {
                let second = if league { e.race_ms } else { e.lap_ms };
                txt_r(b, d, HudFont::Glass, 16.0, 0.5, tone(0xffffff, 0.78), &rank_time(second), right(&cols[4]), oy, lh);
            }
            // the gap to the first row: RECORD, TIED, or how far behind
            let gc = cols.last().expect("the gap column");
            let lead = |b: &mut Batch, s: &str| txt_r(b, d, HudFont::GlassMed, 11.0, 1.5, tone(G_DIM, 1.0), s, right(gc), oy, lh);
            let first = top_row.unwrap_or(e);
            if i == 0 {
                lead(b, "RECORD");
            } else if league {
                if first.score == e.score {
                    lead(b, "TIED");
                } else {
                    txt_r(b, d, HudFont::GlassMed, 15.0, 0.5, tone(G_DIM, 1.0), &format!("-{}", first.score.saturating_sub(e.score)), right(gc), oy, lh);
                }
            } else {
                let (a, z) = if lap_tab { (e.lap_ms, first.lap_ms) } else { (e.race_ms, first.race_ms) };
                txt_r(b, d, HudFont::GlassMed, 15.0, 0.5, tone(G_DIM, 1.0), &crate::hud_glass::glass_gap(i64::from(a) - i64::from(z)), right(gc), oy, lh);
            }
        }
        // the footer under the panel (right, y 604): the kind's rule, and BUILT-IN TIMES when no run is in the table
        let mut right = d.x(px + pw);
        if !slot.is_some_and(|s| s.real) {
            let w = Self::tag_width(b, d, "BUILT-IN TIMES");
            Self::chip(b, d, "BUILT-IN TIMES", (right - w, d.y(604.0)), w, G_DIM);
            right -= w + d.d(10.0);
        }
        txt_r(b, d, HudFont::GlassMed, 11.0, 1.0, hex(G_DIM, 0.75), &KINDS[v.kind.min(KINDS.len() - 1)].2.to_uppercase(), right, d.y(604.0), Some(18.0));
    }

    /// The table's columns for a kind: `#`, DRIVER, CAR, the ranking value, the second value (not in the lap tab), the
    /// gap (`columnsFor`: the panel's 802 px of row, columns 14 apart, the driver column taking what is left).
    fn rank_columns(league: bool, lap_tab: bool) -> Vec<Col> {
        let (car, m, s, g) = (140.0, if league { 100.0 } else { 118.0 }, 118.0, 104.0);
        let place = 40.0;
        let fixed = car + m + if lap_tab { 0.0 } else { s } + g + 14.0 * 3.0 + if lap_tab { 0.0 } else { 14.0 };
        let name_w = PANEL.2 - 20.0 - 14.0 - (place + 6.0) - fixed;
        let mut cols = vec![Col { head: "#", x: 0.0, w: place, align: Side::Centre, key: false }];
        let mut x = place + 6.0;
        cols.push(Col { head: "DRIVER", x, w: name_w, align: Side::Left, key: false });
        x += name_w + 14.0;
        cols.push(Col { head: "CAR", x, w: car, align: Side::Left, key: false });
        x += car + 14.0;
        let head = if lap_tab { "BEST LAP" } else if league { "POINTS" } else { "RACE TIME" };
        cols.push(Col { head, x, w: m, align: Side::Right, key: true });
        x += m + 14.0;
        if !lap_tab {
            cols.push(Col { head: if league { "SEASON TIME" } else { "BEST LAP" }, x, w: s, align: Side::Right, key: false });
            x += s + 14.0;
        }
        cols.push(Col { head: if league { "BEHIND #1" } else { "GAP TO #1" }, x, w: g, align: Side::Right, key: false });
        cols
    }

    /// `.fgroup`: a key cap, the label, the values (the current one white with the orange underline), a key cap, ANY
    /// when the kind does not use it. The whole group at .35 when unused. `(x, y)` = its top left, design px; 28 high,
    /// 16 apart.
    #[allow(clippy::too_many_arguments)]
    fn filter_group(&self, b: &mut Batch, d: Glass, (x, y): (f32, f32), label: &str, caps: (Cap, Cap), values: &[String], cur: usize, on: bool) {
        let a = if on { 1.0 } else { 0.35 };
        let mid = d.y(y + 14.0);
        let gap = d.d(16.0);
        let (wl, wr) = (self.rank_cap_w(b, d, caps.0), self.rank_cap_w(b, d, caps.1));
        let lab = d.font(HudFont::GlassMed, 11.0, 1.5, hex(G_DIM, a));
        let val = |i: usize| d.font(HudFont::Glass, 15.0, 1.0, hex(if on && i == cur { 0xffffff } else { G_DIM }, a));
        let mut cx = d.x(x);
        let left = cx;
        cx += wl + gap + d.d(2.0);
        let lab_x = cx;
        cx += b.font_width(HudFont::GlassMed, label, &lab) + d.d(2.0) + gap;
        let mut runs = Vec::new();
        for (i, s) in values.iter().enumerate() {
            let w = b.font_width(HudFont::Glass, s, &val(i));
            runs.push((cx, w));
            cx += w + gap;
        }
        let right = cx;
        let mut m = Mesh::default();
        if on {
            if let Some(&(ux, uw)) = runs.get(cur) {
                let line = vec::rounded_rect(ux, mid + d.d(12.0), uw, d.d(2.0), d.d(1.0));
                m.halo(&line, d.d(8.0), hex(G_ORANGE, 0.5));
                m.fill(&line, hex(G_ORANGE, 1.0), d.px);
            }
        }
        b.mesh(&m);
        for (i, (s, &(vx, _))) in values.iter().zip(&runs).enumerate() {
            b.font_text(HudFont::Glass, s, vx, d.cap_top(HudFont::Glass, 15.0, mid - d.d(13.0), Some(26.0)), &val(i));
        }
        b.font_text(HudFont::GlassMed, label, lab_x, d.cap_top(HudFont::GlassMed, 11.0, mid - d.d(14.0), Some(28.0)), &lab);
        self.rank_cap(b, d, caps.0, [left + wl * 0.5, mid], a);
        self.rank_cap(b, d, caps.1, [right + wr * 0.5, mid], a);
        if !on {
            let at = right + wr + gap;
            b.font_text(HudFont::GlassMed, "ANY", at, d.cap_top(HudFont::GlassMed, 11.0, mid - d.d(14.0), Some(28.0)), &d.font(HudFont::GlassMed, 11.0, 1.0, hex(G_DIM, 0.75)));
        }
    }

    /// The pad glyph of a cap (`glyph_for`), or the width of the arrows' (no glyph on the pad: its D-pad kinds 0 / 1).
    fn rank_cap_glyph(&self, c: Cap) -> (char, [f32; 4]) {
        Self::glyph_for(self.controls.last, match c {
            Cap::Left => 0,
            Cap::Right => 1,
            Cap::Key(_, kind) => kind,
        })
    }

    /// A filter key's width: the cap's 24 design px, or the pad glyph's.
    fn rank_cap_w(&self, b: &Batch, d: Glass, c: Cap) -> f32 {
        if self.controls.last == Device::Keyboard {
            return d.d(24.0);
        }
        let (g, _) = self.rank_cap_glyph(c);
        b.font_width(HudFont::Glyphs, &g.to_string(), &FontStyle::new(d.d(18.0), WHITE)).max(d.d(24.0))
    }

    /// A filter key at `c` (canvas px): a 24 x 20 cap with the letter or the arrow (`.fgroup .kcap`) on the keyboard,
    /// the pad's glyph otherwise.
    fn rank_cap(&self, b: &mut Batch, d: Glass, cap: Cap, c: [f32; 2], a: f32) {
        if self.controls.last != Device::Keyboard {
            let (g, mut tint) = self.rank_cap_glyph(cap);
            tint[3] *= a;
            let gs = FontStyle::new(d.d(18.0), tint);
            let gw = b.font_width(HudFont::Glyphs, &g.to_string(), &gs);
            b.font_text(HudFont::Glyphs, &g.to_string(), c[0] - gw * 0.5, c[1] - gs.size * 0.5, &gs);
            return;
        }
        let (w, h) = (d.d(24.0), d.d(20.0));
        let (x, y) = (c[0] - w * 0.5, c[1] - h * 0.5);
        let mut m = Mesh::default();
        let body = vec::rounded_rect(x, y, w, h, d.d(5.0));
        m.fill(&vec::rounded_rect(x, y + d.d(2.0), w, h, d.d(5.0)), [0.0, 0.0, 0.0, 0.45 * a], d.px);
        m.fill_with(&body, &|q| [1.0, 1.0, 1.0, (0.28 - 0.20 * ((q[1] - y) / h).clamp(0.0, 1.0)) * a], d.px);
        m.stroke(&body, true, d.w(1.0), Ends::Round, [1.0, 1.0, 1.0, 0.4 * a], d.px);
        // the arrows: the 12 px viewBox of the mockup's svg at 11 px, stroke 2.4
        let p = |u: f32, v: f32| [c[0] + (u - 6.0) * d.d(11.0 / 12.0), c[1] + (v - 6.0) * d.d(11.0 / 12.0)];
        match cap {
            Cap::Left => m.stroke(&[p(7.5, 2.5), p(4.0, 6.0), p(7.5, 9.5)], false, d.w(2.4 * 11.0 / 12.0), Ends::Round, [1.0, 1.0, 1.0, a], d.px),
            Cap::Right => m.stroke(&[p(4.5, 2.5), p(8.0, 6.0), p(4.5, 9.5)], false, d.w(2.4 * 11.0 / 12.0), Ends::Round, [1.0, 1.0, 1.0, a], d.px),
            Cap::Key(..) => {}
        }
        b.mesh(&m);
        if let Cap::Key(letter, _) = cap {
            let st = d.font(HudFont::Glass, 11.0, 0.5, [1.0, 1.0, 1.0, a]);
            let lw = b.font_width(HudFont::Glass, letter, &st) - st.tracking;
            b.font_text(HudFont::Glass, letter, c[0] - lw * 0.5, d.cap_top(HudFont::Glass, 11.0, y, Some(20.0)), &st);
        }
    }
}

/// How a column's cells line up.
#[derive(Clone, Copy)]
enum Side {
    Left,
    Right,
    Centre,
}

/// One column of the table: heading, x from the row's left, width, alignment; `key` = the heading in the lighter tone.
struct Col {
    head: &'static str,
    x: f32,
    w: f32,
    align: Side,
    key: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn e(name: &str, ms: u32) -> Entry {
        Entry { name: name.into(), car: "TITAN".into(), lap_ms: ms, race_ms: ms, ..Entry::default() }
    }

    #[test]
    fn a_table_of_built_in_rows_is_not_real_and_has_no_place() {
        let rows = [e("DI dude 1", 240_000), e("DI dude 2", 241_000)];
        let s = rank_slot(&rows, &rows, &|n| n == "SIN");
        assert!(!s.real);
        assert_eq!(s.best, None);
        assert_eq!(s.built_in, [true, true]);
    }

    #[test]
    fn your_best_row_is_the_first_of_yours_and_built_in_rows_do_not_count() {
        let defaults = [e("DI dude 1", 240_000), e("DI dude 2", 241_000), e("DI dude 3", 242_000)];
        let rows = [e("RIVAL", 60_000), e("SIN", 61_000), e("SIN", 62_000), e("DI dude 1", 240_000)];
        let s = rank_slot(&rows, &defaults, &|n| n == "SIN");
        assert!(s.real);
        assert_eq!(s.best.map(|(i, x)| (i, x.lap_ms)), Some((1, 61_000)));
        assert_eq!(s.built_in, [false, false, false, true]);
    }

    #[test]
    fn a_table_with_runs_that_are_not_yours_has_no_place() {
        let defaults = [e("DI dude 1", 240_000)];
        let s = rank_slot(&[e("RIVAL", 60_000), e("DI dude 1", 240_000)], &defaults, &|n| n == "SIN");
        assert!(s.real);
        assert_eq!(s.best, None);
    }

    #[test]
    fn times_read_minutes_seconds_milliseconds() {
        assert_eq!(rank_time(191_044), "3:11.044");
        assert_eq!(rank_time(35_522), "0:35.522");
        assert_eq!(rank_time(3_661_005), "1:01:01.005");
    }

    #[test]
    fn the_columns_fill_the_row_and_end_where_the_mockup_does() {
        for (league, lap) in [(false, false), (false, true), (true, false)] {
            let cols = Game::rank_columns(league, lap);
            let last = cols.last().unwrap();
            assert_eq!(last.x + last.w, 802.0, "gap column ends at the row's inner right ({league} {lap})");
            // the ranking value ends at 552 in every kind
            assert!(lap || cols[3].x + cols[3].w == 552.0 || cols[3].x + cols[3].w == 684.0);
        }
        let single = Game::rank_columns(false, false);
        assert_eq!((single[1].w, single[2].x, single[3].x), (220.0, 280.0, 434.0));
    }
}
