//! REMASTER League › View standings (#92 phase 2, concept A · Paddock of `reference_materials/leaguetable-overhaul/A-paddock.html`,
//! shots `shots/A-*.jpg`): the four divisions as the tab bar (Division 3, 2, 1, the Final: the way up, Q / E or left / right
//! step along it, the ends stop; it opens on yours, marked with the blue dot), the division's standings as a Glass table
//! (zone bars, portraits, car, wins / 2nd / 3rd, the gap to the leader, points; your row lit) and a card with your league:
//! the status line, the ladder with a ring on the division shown, the next race with its map and the season's dots, your
//! place, points and best lap. The numbers are `league.rs` (`League`, `zone`, `season_tracks`, `next_track`); the table and
//! status line are League Load's (`league_load.rs`); keys in front.rs `Screen::LeagueTable`. Design px through `Glass`, 1:1
//! from the mockup's inline styles (08-missing-features 8.8).

use super::league_load::{car_of, date_text, line_box, Cols, Save, Standings, GAIN, LOSS, NOTE, ORANGE_TEXT};
use super::pause_settings::glass_panel;
use super::rankings::rank_time;
use super::results::{txt, txt_r};
use super::*;
use crate::league::{self, League};

/// The standings card and the league card.
const TABLE: (f32, f32, f32, f32) = (62.0, 228.0, 776.0, 372.0);
const CARD: (f32, f32, f32, f32) = (854.0, 228.0, 364.0, 372.0);
/// Their inner widths (the 1 px borders off).
const TABLE_INNER: f32 = 774.0;
const CARD_INNER: f32 = 362.0;
/// The tabs, the way up: a tab's index is `3 - division`.
const NAMES: [&str; 4] = ["THE FINAL", "DIVISION 1", "DIVISION 2", "DIVISION 3"];
/// What each division's table decides, by division (the mockup's `ZONE_RULE`).
const RULES: [&str; 4] = ["ONE RACE \u{b7} THE WINNER TAKES THE LEAGUE", "THE WINNER GOES TO THE FINAL \u{b7} BOTTOM 2 GO DOWN", "TOP 2 GO UP \u{b7} BOTTOM 2 GO DOWN", "TOP 2 GO UP"];
/// The columns of a division's rows and of the Final's two big ones.
const COLS: Cols = Cols { pic_w: 46.0, nm: 106.0, nm_px: 17.0, car: 296.0, w: 430.0, w_step: 50.0, w_w: 40.0, gap: Some(586.0), pts: 650.0, pts_w: 80.0 };
const COLS_FINAL: Cols = Cols { pic_w: 84.0, nm: 152.0, nm_px: 24.0, car: 340.0, w: 470.0, w_step: 50.0, w_w: 40.0, gap: Some(600.0), pts: 650.0, pts_w: 80.0 };

/// The zone mark of place `i` of division `div` (true = goes up): `league::zone`, and the Final's 2nd place goes down: the
/// season end swaps it with the Division 1 winner (0x453ccc; a proposal, `league::zone` marks nothing in the Final).
fn zone_mark(div: usize, i: usize) -> Option<bool> {
    if div == 0 { (i == 1).then_some(false) } else { league::zone(div, i) }
}

/// The name of a division as the header and the cards say it.
fn div_name(div: usize) -> &'static str {
    NAMES[div.min(3)]
}

/// The baseline (canvas px) of text of font `f` at `px` whose CSS line box (`lh` design px, None = normal) starts at
/// canvas `top`.
fn baseline(d: Glass, f: HudFont, px: f32, top: f32, lh: Option<f32>) -> f32 {
    d.cap_top(f, px, top, lh) + px * d.s * ui_font::face(f).cap / ui_font::SRC
}

/// The ascent of font `f` at `px`, design px.
fn ascent(f: HudFont, px: f32) -> f32 {
    ui_font::face(f).ascender * px / ui_font::SRC
}

/// How far an inline box of `f` at `px` hangs under the top of an `abs` block of the page's 16 px text: it sits on the
/// block's baseline (the strut), so its line box starts `ascent(16) - ascent(px)` down.
fn strut(f: HudFont, px: f32) -> f32 {
    ascent(HudFont::GlassMed, 16.0) - ascent(f, px)
}

impl Game {
    /// Q / E (LB / RB) and left / right: the next division along the ladder (`up` = towards the Final); the ends stop
    /// with the end sound.
    pub(crate) fn league_table_step(&mut self, up: bool) {
        let at = self.league_div.min(3);
        let to = if up { at.saturating_sub(1) } else { (at + 1).min(3) };
        self.ev_override = Some(if to == at { ev::CURSOR_END } else { ev::SELECT });
        self.league_div = to;
    }

    /// The glass the hologram is frosted behind: the plate, the tab bar, the two cards (canvas px, with the radius).
    fn league_table_cards(&self, d: Glass) -> Vec<(f32, f32, f32, f32, f32)> {
        let plate = (d.r(1021.0), d.t(GLASS_PLATE_TOP), d.d(234.0), d.d(119.0));
        let rect = |(x, y, w, h): (f32, f32, f32, f32)| (d.x(x), d.y(y), d.d(w), d.d(h));
        [plate, rect((62.0, 160.0, 1156.0, 52.0)), rect(TABLE), rect(CARD)].into_iter().map(|(x, y, w, h)| (x, y, w, h, d.d(12.0))).collect()
    }

    /// REMASTER League › View standings (#92): the Glass frame, the division tabs and the two cards over the hologram.
    pub(crate) fn league_table_modern(&self, size: (u32, u32)) -> Vec<Layer> {
        let canvas = Canvas::hud(size);
        let px = 1.0 / canvas.transform(size).0;
        let d = Glass::new(&canvas, px);
        let cards = self.league_table_cards(d);
        let back = self.ui_in(Batch::sharp_text(&self.assets, canvas, 1.0), |_, b| Self::glass_backdrop(b, d));
        let front = self.ui_in(Batch::sharp_text(&self.assets, canvas, 1.0), |g, b| {
            // the title and plate codes are the Classic League art's (`frame_common("leagu", "scrc")`)
            Self::glass_options_frame(b, d, g.t, "league", &format!("MAIN // LEAGUE // STANDINGS // {}", div_name(g.league_div)), "88-137:59", "LEAGUESCREEN", "187-C-35");
            g.league_table_body(b, d);
            Self::glass_hints(b, d, g.controls.last, &[(&[4, 5], "DIVISION"), (&[3], "BACK")], 626.0);
        });
        let mut out = vec![back];
        out.extend(self.hologram(size, None, &cards));
        out.push(front);
        out
    }

    /// The tab bar and the two cards of the current league.
    fn league_table_body(&self, b: &mut Batch, d: Glass) {
        let Some(l) = self.current_league() else { return };
        let sv = Save::of(l, &self.track_codes());
        let div = self.league_div.min(3);
        // the way up, left to right; the tab of your division carries the blue dot, the Final's a trophy
        let tabs: Vec<(&str, Option<char>, bool)> = [3, 2, 1, 0].iter().map(|&dv| (div_name(dv), (dv == 0).then_some(ui_font::icon::TROPHY), dv == sv.div)).collect();
        self.glass_tabbar_with(b, d, &tabs, 3 - div);
        self.standings_card(b, d, l, &sv, div);
        self.standings_league_card(b, d, l, &sv, div);
    }

    // ------------------------------------------------------------------------------------------ the standings

    /// The standings card: the division's name and rule, how far the round is, the table, the legend; the Final's two
    /// rows are big and carry the trophy block under them.
    fn standings_card(&self, b: &mut Batch, d: Glass, l: &League, sv: &Save, div: usize) {
        let (x, y, w, h) = TABLE;
        let mut m = Mesh::default();
        glass_panel(&mut m, d, (d.x(x), d.y(y), d.d(w), d.d(h)));
        b.mesh(&m);
        let (ix, iy) = (x + 1.0, y + 1.0);
        let at = |px: f32, py: f32| (d.x(ix + px), d.y(iy + py));
        let (fin, mine) = (div == 0, div == sv.div);
        // the name (20 px in a 24 px line) with the rule beside it on the same baseline, 14 px after it
        let (nx, ny) = at(20.0, 12.0);
        let nw = txt(b, d, HudFont::Glass, 20.0, 0.5, hex(0xffffff, 1.0), div_name(div), nx, ny, Some(24.0));
        let base = baseline(d, HudFont::Glass, 20.0, ny, Some(24.0));
        b.font_text(HudFont::GlassMed, RULES[div], nx + nw + d.d(14.0), d.cap_above(HudFont::GlassMed, 11.0, base), &d.font(HudFont::GlassMed, 11.0, 1.5, hex(G_DIM, 1.0)));
        // the round at the right: how far the season is, or where you race when it is not here
        let right = d.x(ix + TABLE_INNER - 20.0);
        let note = d.font(HudFont::GlassMed, 11.0, 1.0, hex(NOTE, 0.75));
        let lb = line_box(HudFont::GlassMed, 11.0);
        if mine {
            let text = if sv.started { format!("ROUND {} OF {} PLAYED", sv.raced, sv.races) } else { "NO ROUNDS PLAYED".to_string() };
            txt_r(b, d, HudFont::GlassMed, 11.0, 1.0, hex(NOTE, 0.75), &text, right, d.y(iy + 16.0 + strut(HudFont::GlassMed, 11.0)), None);
        } else {
            // the dot, 8 px, the words in the blue: an inline-flex centred on its first item's bottom, on the strut's baseline
            let text = format!("YOU RACE IN {}", div_name(sv.div));
            let top = iy + 16.0 + ascent(HudFont::GlassMed, 16.0) - (lb * 0.5 + 3.0);
            let tw = b.font_width(HudFont::GlassMed, &text, &note);
            txt_r(b, d, HudFont::GlassMed, 11.0, 1.0, hex(G_S1, 1.0), &text, right, d.y(top), None);
            Self::you_dot(b, d, [right - tw - d.d(8.0 + 3.0), d.y(top + lb * 0.5)]);
        }
        let s = Standings {
            table: l.division(div),
            div,
            started: sv.started,
            dim: !sv.started,
            origin: (ix, iy),
            top: 66.0,
            row_h: if fin { 64.0 } else { 32.0 },
            row_w: TABLE_INNER - 20.0,
            cols: if fin { COLS_FINAL } else { COLS },
            zone: zone_mark,
        };
        Self::standings(b, d, &s);
        // the legend under the table, and in the Final the trophy block under its rows
        let (lx, top) = at(20.0, if fin { 206.0 } else { 346.0 });
        if fin {
            Self::legend_keys(b, d, (lx, top), &[("2ND MEETS THE DIVISION 1 WINNER", LOSS)]);
            let mut m = Mesh::default();
            let (a, z) = (at(18.0, 228.0), at(TABLE_INNER - 18.0, 228.0));
            m.dashed([a.0, a.1], [z.0, z.1], d.d(6.0), d.d(4.0), d.w(1.0), hex(G_LINE, 0.45), d.px);
            b.mesh(&m);
            self.final_block(b, d, l, sv, mine, (d.x(ix) + d.d(TABLE_INNER) * 0.5, d.y(iy + 244.0)));
        } else {
            let keys: &[(&str, u32)] = match div {
                1 => &[("TO THE FINAL", GAIN), ("GO DOWN", LOSS)],
                2 => &[("GO UP", GAIN), ("GO DOWN", LOSS)],
                _ => &[("GO UP", GAIN)],
            };
            if sv.started {
                Self::legend_keys(b, d, (lx, top), keys);
            } else {
                txt(b, d, HudFont::GlassMed, 11.0, 1.0, hex(NOTE, 0.75), "NO RACES YET \u{b7} THE ZONES SHOW AFTER THE FIRST ROUND", lx, Self::legend_text_top(d, top), None);
            }
            txt_r(b, d, HudFont::GlassMed, 11.0, 1.0, hex(NOTE, 0.75), "W 2ND 3RD = RACE FINISHES \u{b7} GAP = POINTS TO THE LEADER", d.x(ix + TABLE_INNER - 20.0), Self::legend_text_top(d, top), None);
        }
    }

    /// `.cdot`: the blue "you" dot of 6 px with its glow, centred on `c` (canvas px).
    fn you_dot(b: &mut Batch, d: Glass, c: [f32; 2]) {
        let mut m = Mesh::default();
        m.halo(&circle(c, d.d(3.0)), d.d(6.0), hex(0x4aa8ff, 0.9));
        m.fill(&circle(c, d.d(3.0)), hex(G_S1, 1.0), d.px);
        b.mesh(&m);
    }

    /// The Final's block under its two rows: the trophy 34 px, a line in 20 px and the help text, centred on canvas `cx`
    /// from canvas `top`, 6 px between them; the text wraps in 560 design px.
    fn final_block(&self, b: &mut Batch, d: Glass, l: &League, sv: &Save, mine: bool, (cx, top): (f32, f32)) {
        Self::glass_icon(b, ui_font::icon::TROPHY, cx - d.d(17.0), top, d.d(34.0), hex(ORANGE_TEXT, 1.0));
        let head = if mine { "WIN THE FINAL, WIN THE LEAGUE" } else { "THE CHAMPION'S SEAT" };
        let hw = b.font_width(HudFont::Glass, head, &d.font(HudFont::Glass, 20.0, 0.5, WHITE));
        let head_top = top + d.d(34.0 + 6.0);
        txt(b, d, HudFont::Glass, 20.0, 0.5, hex(0xffffff, 1.0), head, cx - hw * 0.5, head_top, Some(24.0));
        let codes = self.track_codes();
        let runs: Vec<(String, bool)> = if mine {
            let rival = l.division(0).iter().find(|p| !p.human).map_or(String::new(), |p| p.p.nick.to_uppercase());
            let (track, laps) = l.next_track(&codes).map_or((String::new(), 0), |t| (self.tracks[t].name.to_uppercase(), self.tracks[t].laps));
            vec![
                ("One race against".into(), false),
                (rival, true),
                (format!("on {track} over {laps} {}. Lose it and you go back to Division 1.", if laps == 1 { "lap" } else { "laps" }), false),
            ]
        } else {
            let way = if sv.div == 1 { ": that is your way in" } else { "" };
            vec![(format!("The leader holds the Final. At the season end the winner of Division 1 takes the 2nd seat{way}."), false)]
        };
        let lines = Self::wrap_runs(b, d, &runs, d.d(560.0));
        let mut ty = head_top + d.d(24.0 + 6.0);
        let space = b.font_width(HudFont::GlassMed, " ", &d.font(HudFont::GlassMed, 15.0, 0.5, WHITE));
        for line in lines {
            let width = |b: &Batch, s: &str, bold: bool| b.font_width(if bold { HudFont::Glass } else { HudFont::GlassMed }, s, &d.font(HudFont::Glass, 15.0, 0.5, WHITE));
            let total: f32 = line.iter().map(|(s, bold)| width(b, s, *bold)).sum::<f32>() + space * (line.len() - 1) as f32;
            let mut px = cx - total * 0.5;
            for (s, bold) in &line {
                let adv = if *bold {
                    txt(b, d, HudFont::Glass, 15.0, 0.5, hex(0xffffff, 1.0), s, px, ty, Some(20.0))
                } else {
                    txt(b, d, HudFont::GlassMed, 15.0, 0.5, hex(G_TEXT, 1.0), s, px, ty, Some(20.0))
                };
                px += adv + space;
            }
            ty += d.d(20.0);
        }
    }

    /// `runs` (a word run and whether it is bold: Glass white against GlassMed) set in lines no wider than `max` canvas
    /// px, one space between the runs and between their words, the way a browser breaks them. The text is 15 px.
    fn wrap_runs(b: &Batch, d: Glass, runs: &[(String, bool)], max: f32) -> Vec<Vec<(String, bool)>> {
        let width = |s: &str, bold: bool| b.font_width(if bold { HudFont::Glass } else { HudFont::GlassMed }, s, &d.font(HudFont::Glass, 15.0, 0.5, WHITE));
        let space = width(" ", false);
        let mut lines: Vec<Vec<(String, bool)>> = vec![Vec::new()];
        let mut at = 0.0;
        for (text, bold) in runs {
            // a bold run stays whole (a name); a plain one breaks at its spaces
            let words: Vec<&str> = if *bold { vec![text.as_str()] } else { text.split(' ').filter(|w| !w.is_empty()).collect() };
            for word in words {
                let w = width(word, *bold);
                let fresh = lines.last().is_some_and(|l| l.is_empty());
                if !fresh && at + space + w > max {
                    lines.push(Vec::new());
                    at = 0.0;
                } else if !fresh {
                    at += space;
                }
                at += w;
                let line = lines.last_mut().unwrap();
                // words of one plain run share a piece, so they are drawn as one string
                match line.last_mut() {
                    Some((s, b2)) if *b2 == *bold && !*bold => {
                        s.push(' ');
                        s.push_str(word);
                    }
                    _ => line.push((word.to_string(), *bold)),
                }
            }
        }
        lines
    }

    // ------------------------------------------------------------------------------------------ the league card

    /// Your league: its name and status, the ladder (a ring on the division shown), the next race with its map and the
    /// season's dots, your place, points and best lap, and what the league has seen.
    fn standings_league_card(&self, b: &mut Batch, d: Glass, l: &League, sv: &Save, div: usize) {
        let (x, y, w, h) = CARD;
        let mut m = Mesh::default();
        glass_panel(&mut m, d, (d.x(x), d.y(y), d.d(w), d.d(h)));
        b.mesh(&m);
        let (ix, iy) = (x + 1.0, y + 1.0);
        let at = |px: f32, py: f32| (d.x(ix + px), d.y(iy + py));
        let cap = |b: &mut Batch, text: &str, (px, py): (f32, f32)| {
            txt(b, d, HudFont::GlassMed, 11.0, 1.5, hex(G_DIM, 1.0), text, px, py, None);
        };
        let (px, py) = at(20.0, 14.0);
        cap(b, "YOUR LEAGUE", (px, py));
        let (px, py) = at(20.0, 30.0);
        txt(b, d, HudFont::Glass, 26.0, 1.0, hex(0xffffff, 1.0), &l.name.to_uppercase(), px, py, Some(32.0));
        let line = [Some(sv.div_name()), date_text(l.create_date).map(|t| format!("CREATED {t}"))].into_iter().flatten().collect::<Vec<_>>().join(" \u{b7} ");
        cap(b, &line, at(20.0, 64.0));
        // the mockup's inline-flex sits on its parent's line: 2.5 px under its `top`
        self.league_status(b, d, sv, at(20.0, 86.5));
        Self::league_load_ladder(b, d, at(16.0, 114.0), 332.0, sv.div, Some(div));
        let mut m = Mesh::default();
        for ry in [170.0, 276.0] {
            let (a, z) = (at(18.0, ry), at(CARD_INNER - 18.0, ry));
            m.dashed([a.0, a.1], [z.0, z.1], d.d(6.0), d.d(4.0), d.w(1.0), hex(G_LINE, 0.45), d.px);
        }
        b.mesh(&m);
        self.next_race(b, d, l, sv, (ix, iy));
        // place, points, best lap: a caption over a 26 px value
        let lb = line_box(HudFont::GlassMed, 11.0);
        let stat = |b: &mut Batch, left: f32, name: &str| -> (f32, f32) {
            let (px, py) = at(left, 290.0);
            cap(b, name, (px, py));
            (px, py + d.d(lb + 2.0))
        };
        let white = hex(0xffffff, 1.0);
        let (px, vy) = stat(b, 20.0, "PLACE");
        if sv.started {
            let nw = txt(b, d, HudFont::Glass, 26.0, 0.5, white, &format!("#{}", sv.place + 1), px, vy, Some(30.0));
            let base = baseline(d, HudFont::Glass, 26.0, vy, Some(30.0));
            let small = format!("/{}", sv.size);
            b.font_text(HudFont::GlassMed, &small, px + nw + d.d(3.0), d.cap_above(HudFont::GlassMed, 13.0, base), &d.font(HudFont::GlassMed, 13.0, 1.0, hex(G_DIM, 1.0)));
        } else {
            Self::dash_col(b, d, px + d.d(15.0), vy + d.d(15.0), 15.0, white);
        }
        let (px, vy) = stat(b, 118.0, "POINTS");
        txt(b, d, HudFont::Glass, 26.0, 0.5, white, &sv.score.to_string(), px, vy, Some(30.0));
        let (px, vy) = stat(b, 204.0, "BEST LAP");
        if l.season_lap_ms > 0 {
            txt(b, d, HudFont::Glass, 26.0, 0.5, white, &rank_time(l.season_lap_ms), px, vy, Some(30.0));
        } else {
            Self::dash_col(b, d, px + d.d(15.0), vy + d.d(15.0), 15.0, white);
        }
        // what the league has seen, and the season's time
        let mut tail = format!("{} {} \u{b7} {} {} IN ALL", l.races_done, if l.races_done == 1 { "RACE" } else { "RACES" }, l.laps_done, if l.laps_done == 1 { "LAP" } else { "LAPS" });
        if l.season_ms > 0 {
            tail += &format!(" \u{b7} SEASON TIME {}", rank_time(l.season_ms));
        }
        let (px, py) = at(20.0, 344.0);
        txt(b, d, HudFont::GlassMed, 11.0, 1.0, hex(NOTE, 0.75), &tail, px, py, None);
    }

    /// The next race: its number, the track, the laps and your car, the season's dots, and the track's lap on a small disc.
    fn next_race(&self, b: &mut Batch, d: Glass, l: &League, sv: &Save, (ix, iy): (f32, f32)) {
        let codes = self.track_codes();
        let Some(t) = l.next_track(&codes) else { return };
        let tr = &self.tracks[t];
        let lb = line_box(HudFont::GlassMed, 11.0);
        let (x, y) = (d.x(ix + 20.0), iy + 184.0);
        txt(b, d, HudFont::GlassMed, 11.0, 1.5, hex(G_DIM, 1.0), &format!("NEXT \u{b7} {}", sv.race_text()), x, d.y(y), None);
        let name_top = y + lb + 3.0;
        txt(b, d, HudFont::Glass, 20.0, 0.5, hex(0xffffff, 1.0), &tr.name.to_uppercase(), x, d.y(name_top), Some(24.0));
        let note_top = name_top + 24.0 + 4.0;
        let me = &l.division(sv.div)[sv.place.min(l.division(sv.div).len() - 1)];
        let laps = tr.laps;
        let note = format!("{laps} {} \u{b7} {}", if laps == 1 { "LAP" } else { "LAPS" }, car_of(me, sv.div));
        txt(b, d, HudFont::GlassMed, 11.0, 1.0, hex(NOTE, 0.75), &note, x, d.y(note_top), None);
        // the dots sit in a line of the page's 16 px text: the flex box on its first dot's bottom edge, on the baseline
        let sizes = sv.dot_sizes();
        let tall = sizes.iter().cloned().fold(0.0, f32::max);
        let first = sizes.first().copied().unwrap_or(9.0);
        let line_top = note_top + lb + 9.0;
        let cy = line_top + ascent(HudFont::GlassMed, 16.0) - ((tall - first) * 0.5 + first) + tall * 0.5;
        Self::season_dots(b, d, sv, x, d.y(cy));
        // the map disc: 84 px at (262, 180), the lap in the three sector colours on its face
        let (cx, cy) = (ix + 262.0 + 42.0, iy + 180.0 + 42.0);
        let c = [d.x(cx), d.y(cy)];
        let (rb, rf) = (d.d(42.0), d.d(30.0));
        let mut m = Mesh::default();
        Self::glass_bezel_small(&mut m, d, c, rb);
        m.disc_at(c, rf, [c[0], c[1] - 0.4 * rf], [120.0 / 255.0, 190.0 / 255.0, 1.0, 0.24], [8.0 / 255.0, 20.0 / 255.0, 40.0 / 255.0, 0.9], 96);
        Self::disc_overlays(&mut m, d, c, rf, 22.0, hex(G_LINE, 0.12), (0.2, 0.34), (0.0, 0.0), (30.0, 0.6));
        Self::gap_ring(&mut m, d, c, rf, d.d(36.0));
        b.mesh(&m);
        Self::lap_outline(b, d, tr, (cx, cy), 28.0, 1.4);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_finals_second_place_goes_down_and_the_divisions_keep_the_games_zones() {
        assert_eq!(zone_mark(0, 0), None);
        assert_eq!(zone_mark(0, 1), Some(false));
        assert_eq!(zone_mark(3, 1), Some(true));
        assert_eq!(zone_mark(3, 2), None);
        assert_eq!(zone_mark(1, 0), Some(true));
        assert_eq!(zone_mark(2, 7), Some(false));
    }

    #[test]
    fn a_tab_is_a_division_counted_from_the_way_up() {
        assert_eq!((0..4).map(|dv| 3 - dv).collect::<Vec<_>>(), [3, 2, 1, 0]);
        assert_eq!((div_name(3), div_name(0)), ("DIVISION 3", "THE FINAL"));
    }
}
