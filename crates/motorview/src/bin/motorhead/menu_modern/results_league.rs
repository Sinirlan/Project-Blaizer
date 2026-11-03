//! REMASTER Results, the League tab (#94 phase 3, mockup B `leagueHTML` of
//! `reference_materials/results-overhaul/B-timing.html`): the human's division after the race with the movement
//! arrows and the promotion / relegation zones, and a card beside it: the season so far and the next race, or at a
//! season's end the outcome (the old "Final league results" screen). Numbers from `league.rs` (`League`, `Moved`,
//! `Outcome`, `zone`, `new_unlocks`). Design px through `Glass`; every offset is the mockup's inline style.

use super::pause_settings::glass_panel;
use super::results::{ord, txt, txt_r, CARD, ROW_W, ROW_X, TABLE};
use super::*;
use crate::league::{self, League, Moved, Outcome, Place, DIV_SIZE, POINTS};

/// What the tab needs from the screen's frame.
pub(super) struct LeagueView {
    pub(super) pulse: f32,
}

/// Movement and zone colours: the gain and loss tokens of doc 19.2 (the arrows carry the meaning too).
const GAIN: u32 = 0x7cf0a8;
const LOSS: u32 = 0xff8a7a;
/// the card's content width (design px)
const CARD_W: f32 = 332.0;

fn div_name(d: usize) -> String {
    if d == 0 {
        "THE FINAL".into()
    } else {
        format!("DIVISION {d}")
    }
}

/// One place of the division with how it moved in the race.
struct LRow<'a> {
    place: &'a Place,
    moved: Option<&'a Moved>,
}

/// `text` broken at spaces into lines no wider than `width` design px at the given font.
fn wrap_font(b: &Batch, d: Glass, f: HudFont, px: f32, ls: f32, text: &str, width: f32) -> Vec<String> {
    let st = d.font(f, px, ls, WHITE);
    let mut lines: Vec<String> = vec![String::new()];
    for word in text.split(' ') {
        let cur = lines.last().unwrap();
        let next = if cur.is_empty() { word.to_string() } else { format!("{cur} {word}") };
        if !cur.is_empty() && b.font_width(f, &next, &st) - st.tracking > d.d(width) {
            lines.push(word.to_string());
        } else {
            *lines.last_mut().unwrap() = next;
        }
    }
    lines
}

impl Game {
    /// The panel's height: the eight rows fill it, the final's two get a short one (`leagueHTML`).
    pub(super) fn league_table_h(&self) -> f32 {
        let n = self.current_league().map_or(8, |l| DIV_SIZE[l.human_division()]);
        if n > 2 {
            TABLE.3
        } else {
            52.0 + n as f32 * 41.0 + 12.0
        }
    }

    /// The header line: after a season's last race the tab is the League results screen.
    pub(super) fn league_header(&self) -> String {
        let end = self.league_outcome.is_some_and(|o| o != Outcome::Season);
        match self.current_league() {
            Some(l) if end => format!("LEAGUE // FINAL RESULTS // {}", div_name(l.human_division())),
            _ => "LEAGUE // RESULTS // LEAGUE".into(),
        }
    }

    pub(super) fn league_tab(&self, b: &mut Batch, d: Glass, v: LeagueView) {
        let Some(l) = self.current_league() else { return };
        let hd = l.human_division();
        let rows: Vec<LRow> = l.division(hd).iter().map(|place| LRow { place, moved: self.league_moves.iter().find(|m| m.name == place.p.name && m.human == place.human) }).collect();
        self.results_league_table(b, d, &rows, hd, v.pulse);
        match self.league_outcome {
            Some(o) if o != Outcome::Season => self.league_end_card(b, d, &rows, hd, o),
            _ => self.league_season_card(b, d, l, &rows, hd),
        }
    }

    /// `leagueHTML`'s table: the division in its new order, a zone bar at the left edge, the movement, the race's place
    /// and points, the score.
    fn results_league_table(&self, b: &mut Batch, d: Glass, rows: &[LRow], hd: usize, pulse: f32) {
        let (px, py, pw, _) = TABLE;
        let mut m = Mesh::default();
        glass_panel(&mut m, d, (d.x(px), d.y(py), d.d(pw), d.d(self.league_table_h())));
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
        for (t, x) in [("POS", 22.0), ("DRIVER", 140.0), ("CAR", 300.0)] {
            head(b, t, x, false);
        }
        for (t, x) in [("W", 456.0), ("2ND", 504.0), ("3RD", 552.0), ("RACE", 650.0), ("SCORE", 738.0)] {
            head(b, t, x, true);
        }
        let white = hex(0xffffff, 1.0);
        let lh = Some(37.0);
        for (i, row) in rows.iter().enumerate() {
            let human = row.place.human;
            let top = Self::row_chrome(b, d, (ix, iy), i, (human, false, i > 0 && rows[i - 1].place.human), pulse);
            let (ox, oy) = (d.x(ix + ROW_X), top + d.d(1.0));
            let at = |v: f32| ox + d.d(v);
            let cy = oy + d.d(18.5);
            // the zone bar: places that go up (green) or down (red) at the season's end
            if let Some(up) = league::zone(hd, i) {
                let col = if up { GAIN } else { LOSS };
                let bar = vec::rounded_rect(at(-11.0), oy + d.d(6.0), d.d(3.0), d.d(25.0), d.d(1.5));
                let mut m = Mesh::default();
                m.halo(&bar, d.d(6.0), hex(col, 0.7));
                m.fill(&bar, hex(col, 1.0), d.px);
                b.mesh(&m);
            }
            // POS, centred in 40
            let pos = (i + 1).to_string();
            let w = b.font_width(HudFont::Glass, &pos, &d.font(HudFont::Glass, 20.0, 0.5, white));
            txt(b, d, HudFont::Glass, 20.0, 0.5, white, &pos, at(20.0) - w * 0.5, oy, lh);
            // the movement: a chevron and the places gained or lost, a dash for none
            let mv = row.moved.map_or(0, |m| m.prev as i32 - (i as i32 + 1));
            if mv != 0 {
                let col = hex(if mv > 0 { GAIN } else { LOSS }, 1.0);
                let num = mv.abs().to_string();
                let nw = b.font_width(HudFont::Glass, &num, &d.font(HudFont::Glass, 13.0, 0.5, col));
                let x = at(58.0) - (d.d(16.0) + nw) * 0.5;
                Self::glass_icon(b, if mv > 0 { ui_font::icon::CHEVRON_UP } else { ui_font::icon::CHEVRON_DOWN }, x, cy - d.d(7.0), d.d(14.0), col);
                txt(b, d, HudFont::Glass, 13.0, 0.5, col, &num, x + d.d(16.0), oy, lh);
            } else {
                let mut m = Mesh::default();
                m.stroke(&[[at(53.0), cy], [at(63.0), cy]], false, d.w(1.5), Ends::Round, [0.749, 0.89, 0.953, 0.35], d.px);
                b.mesh(&m);
            }
            Self::results_portrait(b, d, &motorview::ui::duo_key(&row.place.p.picture), &row.place.p.nick.to_uppercase(), (at(80.0), cy - d.d(14.5), d.d(46.0), d.d(29.0)));
            let nick = row.place.p.nick.to_uppercase();
            let nw = txt(b, d, HudFont::Glass, 18.0, 0.5, if human { white } else { hex(G_TEXT, 1.0) }, &nick, at(136.0), oy, lh);
            if human {
                txt(b, d, HudFont::Glass, 10.0, 1.5, hex(G_S1, 1.0), "YOU", at(136.0) + nw + d.d(8.0), oy, lh);
            }
            txt(b, d, HudFont::GlassMed, 14.0, 1.0, dim, &row.place.p.cars[hd.min(3)].to_uppercase(), at(290.0), oy, lh);
            for (v, x) in [(row.place.wins, 442.0), (row.place.seconds, 490.0), (row.place.thirds, 538.0)] {
                txt_r(b, d, HudFont::Glass, 18.0, 0.5, white, &v.to_string(), at(x), oy, lh);
            }
            // RACE: this race's place and the points it gave
            match row.moved.map(|m| m.finish).filter(|&f| f > 0) {
                Some(f) => {
                    let gain = format!("+{}", POINTS.get(f as usize - 1).copied().unwrap_or(0));
                    let gw = b.font_width(HudFont::GlassMed, &gain, &d.font(HudFont::GlassMed, 15.0, 0.5, dim));
                    txt_r(b, d, HudFont::GlassMed, 15.0, 0.5, dim, &gain, at(636.0), oy, lh);
                    txt_r(b, d, HudFont::GlassMed, 15.0, 0.5, hex(G_TEXT, 1.0), &ord(f as usize), at(636.0) - gw - d.d(6.0), oy, lh);
                }
                None => txt_r(b, d, HudFont::GlassMed, 15.0, 0.5, dim, "--", at(636.0), oy, lh),
            }
            txt_r(b, d, HudFont::Glass, 20.0, 0.5, white, &row.place.score.to_string(), at(ROW_W - 12.0), oy, lh);
        }
    }

    /// The side card's glass (376 x 372 at 842, 228) and its inner origin in canvas px.
    fn league_card(b: &mut Batch, d: Glass) -> (f32, f32) {
        let mut m = Mesh::default();
        glass_card(&mut m, d, (d.x(CARD.0), d.y(CARD.1), d.d(CARD.2), d.d(CARD.3)), d.d(12.0), (0.11, 0.03), 0.36, (10.0, 26.0, 0.45), 0.35, None);
        b.mesh(&m);
        (d.x(CARD.0 + 1.0), d.y(CARD.1 + 1.0))
    }

    /// `.cap`: a small dim caption, its line box starting at canvas (x, top).
    fn league_cap(b: &mut Batch, d: Glass, text: &str, x: f32, top: f32) -> f32 {
        txt(b, d, HudFont::GlassMed, 11.0, 1.5, hex(G_DIM, 1.0), text, x, top, None)
    }

    /// Mid-season: where you stand, the season's races, the next one on its map and the zones.
    fn league_season_card(&self, b: &mut Batch, d: Glass, l: &League, rows: &[LRow], hd: usize) {
        let (cx, cy) = Self::league_card(b, d);
        let at = |x: f32, y: f32| (cx + d.d(x), cy + d.d(y));
        let codes = self.track_codes();
        let races = league::season_tracks(&codes, hd);
        let done = l.race_number as usize;
        let white = hex(0xffffff, 1.0);
        Self::league_cap(b, d, &format!("{} \u{b7} RACE {} OF {}", div_name(hd), done.min(races.len()), races.len()), at(22.0, 16.0).0, at(22.0, 16.0).1);
        // your place and the gap to the leader (or to the one behind)
        let me = rows.iter().position(|r| r.place.human).unwrap_or(0);
        let score = rows.get(me).map_or(0, |r| r.place.score);
        let (x, y) = at(22.0, 34.0);
        let top = y;
        let w = txt(b, d, HudFont::Glass, 40.0, 1.0, white, &ord(me + 1), x, top, Some(46.0));
        let base = d.cap_top(HudFont::Glass, 40.0, top, Some(46.0)) + d.font(HudFont::Glass, 40.0, 1.0, WHITE).size;
        let unit = d.font(HudFont::GlassMed, 15.0, 1.0, hex(G_DIM, 1.0));
        b.font_text(HudFont::GlassMed, "IN THE DIVISION", x + w + d.d(6.0), d.cap_above(HudFont::GlassMed, 15.0, base), &unit);
        let nick = |i: usize| rows[i].place.p.nick.to_uppercase();
        let gap = if rows.len() < 2 {
            String::new()
        } else if me == 0 {
            match score - rows[1].place.score {
                0 => format!(" \u{b7} LEVEL WITH {}", nick(1)),
                g => format!(" \u{b7} {g} AHEAD OF {}", nick(1)),
            }
        } else {
            match rows[0].place.score - score {
                0 => format!(" \u{b7} LEVEL WITH {}", nick(0)),
                g => format!(" \u{b7} {g} BEHIND {}", nick(0)),
            }
        };
        let (x, y) = at(22.0, 84.0);
        txt(b, d, HudFont::GlassMed, 15.0, 0.5, hex(G_TEXT, 1.0), &format!("{score} PTS{gap}"), x, y, None);
        // the season's races: dots on a line, raced ones lit, the next one orange
        let (ax, ay) = at(22.0, 118.0);
        let n = races.len();
        let mut m = Mesh::default();
        m.fill(&vec::rounded_rect(ax + d.d(30.0), ay + d.d(6.0), d.d(272.0), d.d(2.0), d.d(1.0)), hex(G_LINE, 0.25), d.px);
        b.mesh(&m);
        for (k, &t) in races.iter().enumerate() {
            let c = [ax + d.d(30.0 + 272.0 * k as f32 / (n.max(2) - 1) as f32), ay + d.d(7.0)];
            let mut m = Mesh::default();
            let (fill, rim, glow) = if k < done {
                (G_S1, 0xffffff, Some(hex(0x4aa8ff, 0.9)))
            } else if k == done {
                (0x04070d, G_ORANGE, Some(hex(G_ORANGE, 0.7)))
            } else {
                (0x04070d, G_LINE, None)
            };
            if let Some(g) = glow {
                m.halo(&circle(c, d.d(7.0)), d.d(8.0), g);
            }
            m.fill(&circle(c, d.d(7.0)), hex(fill, 1.0), d.px);
            m.stroke(&circle(c, d.d(6.0)), true, d.w(2.0), Ends::Round, hex(rim, if rim == G_LINE { 0.6 } else { 1.0 }), d.px);
            b.mesh(&m);
            // a long season (Division 1) shows numbers: its names would not fit
            let name = if n <= 3 { self.tracks[t].name.to_uppercase() } else { (k + 1).to_string() };
            let nw = b.font_width(HudFont::Glass, &name, &d.font(HudFont::Glass, 11.0, 1.0, WHITE));
            txt(b, d, HudFont::Glass, 11.0, 1.0, hex(G_TEXT, 1.0), &name, c[0] - nw * 0.5, ay + d.d(20.0), None);
            let sub = if k < done { "RACED" } else if k == done { "NEXT" } else { "" };
            let sw = b.font_width(HudFont::GlassMed, sub, &d.font(HudFont::GlassMed, 10.0, 1.0, WHITE));
            txt(b, d, HudFont::GlassMed, 10.0, 1.0, hex(G_DIM, 1.0), sub, c[0] - sw * 0.5, ay + d.d(39.0), None);
        }
        let mut m = Mesh::default();
        let (a, z) = (at(22.0, 184.0), at(22.0 + CARD_W, 184.0));
        m.dashed([a.0, a.1], [z.0, z.1], d.d(6.0), d.d(4.0), d.w(1.0), hex(G_LINE, 0.45), d.px);
        b.mesh(&m);
        // the next race: its map disc, name, length and the zones of the division
        let (x, y) = at(22.0, 196.0);
        Self::league_cap(b, d, "NEXT RACE", x, y);
        let Some(t) = l.next_track(&codes) else { return };
        let tr = &self.tracks[t];
        let disc = MapDisc::small((CARD.0 + 1.0 + 22.0 + 62.0, CARD.1 + 1.0 + 216.0 + 62.0), 62.0);
        Self::glass_map(b, d, tr, &disc, &[]);
        let (x, y) = at(160.0, 234.0);
        txt(b, d, HudFont::Glass, 20.0, 0.5, white, &tr.name.to_uppercase(), x, y, Some(24.0));
        let km = if tr.length_m > 0.0 { tr.length_m } else { tr.profile.as_ref().map_or(0.0, |p| p.length) };
        let note = hex(0x8fb5c9, 0.75);
        let laps = self.tracks[t].laps;
        txt(b, d, HudFont::GlassMed, 11.0, 1.0, note, &format!("TRACK {} \u{b7} {:.2} KM \u{b7} {} {}", tr.n, km / 1000.0, laps, if laps == 1 { "LAP" } else { "LAPS" }), x, y + d.d(26.0), None);
        let zones: &[(&str, u32)] = match hd {
            3 => &[("PLACES 1-2 GO UP", GAIN)],
            2 => &[("PLACES 1-2 GO UP", GAIN), ("PLACES 7-8 GO DOWN", LOSS)],
            1 => &[("PLACE 1 GOES TO THE FINAL", GAIN), ("PLACES 7-8 GO DOWN", LOSS)],
            _ => &[],
        };
        let (x, mut y) = at(160.0, 286.0);
        for &(text, col) in zones {
            let bar = vec::rounded_rect(x, y + d.d(0.5), d.d(3.0), d.d(12.0), d.d(1.5));
            let mut m = Mesh::default();
            m.halo(&bar, d.d(6.0), hex(col, 0.7));
            m.fill(&bar, hex(col, 1.0), d.px);
            b.mesh(&m);
            txt(b, d, HudFont::GlassMed, 11.0, 1.0, note, text, x + d.d(11.0), y, None);
            y += d.d(19.0);
        }
    }

    /// A season's end: the outcome in big letters, your move on the division ladder, what is new in Single race and
    /// the movie that follows (the old "Final league results" screen).
    fn league_end_card(&self, b: &mut Batch, d: Glass, rows: &[LRow], hd: usize, o: Outcome) {
        let (cx, cy) = Self::league_card(b, d);
        let at = |x: f32, y: f32| (cx + d.d(x), cy + d.d(y));
        let white = hex(0xffffff, 1.0);
        // `radial-gradient(closest-side, rgba(255,140,50,.2), transparent)` at (-80, -60), 420 x 260, cut to the card
        let shape = vec::rounded_rect(d.x(CARD.0), d.y(CARD.1), d.d(CARD.2), d.d(CARD.3), d.d(12.0));
        let c = at(130.0, 70.0);
        let mut m = Mesh::default();
        m.fill_with(&shape, &|q| {
            let k = (((q[0] - c.0) / d.d(210.0)).powi(2) + ((q[1] - c.1) / d.d(130.0)).powi(2)).sqrt();
            [1.0, 0.549, 0.196, 0.2 * (1.0 - k).max(0.0)]
        }, 0.0);
        b.mesh(&m);
        let (from, to) = o.ladder(hd);
        let (x, y) = at(22.0, 16.0);
        Self::league_cap(b, d, &format!("{} \u{b7} SEASON OVER", div_name(hd)), x, y);
        let icon = match o {
            Outcome::Promoted(_) => ui_font::icon::TRENDING_UP,
            Outcome::Relegated | Outcome::LostFinal => ui_font::icon::TRENDING_DOWN,
            Outcome::Won => ui_font::icon::TROPHY,
            _ => ui_font::icon::FLAG,
        };
        let (x, y) = at(22.0, 46.0);
        Self::glass_icon(b, icon, x, y, d.d(30.0), hex(0xffb26a, 1.0));
        Self::league_word(b, d, o.word(), at(64.0, 36.0), 290.0);
        // your place and the move, in words
        let me = rows.iter().position(|r| r.place.human).unwrap_or(0);
        let score = rows.get(me).map_or(0, |r| r.place.score);
        let line = match o {
            Outcome::Won => "YOU WON THE LEAGUE".to_string(),
            Outcome::Stays => format!("YOU STAY IN {}", div_name(hd)),
            _ => format!("{} \u{bb} {}", div_name(from.max(0) as usize), div_name(to.max(0) as usize)),
        };
        let lines = wrap_font(b, d, HudFont::GlassMed, 15.0, 0.5, &format!("{line} \u{b7} {} WITH {score} PTS", ord(me + 1)), CARD_W);
        let (x, y) = at(22.0, 88.0);
        for (k, t) in lines.iter().enumerate() {
            txt(b, d, HudFont::GlassMed, 15.0, 0.5, hex(G_TEXT, 1.0), t, x, y + d.d(20.0 * k as f32), Some(20.0));
        }
        Self::league_ladder(b, d, at(22.0, 124.0), from, to);
        let rule = |b: &mut Batch, y: f32| {
            let (a, z) = (at(22.0, y), at(22.0 + CARD_W, y));
            let mut m = Mesh::default();
            m.dashed([a.0, a.1], [z.0, z.1], d.d(6.0), d.d(4.0), d.w(1.0), hex(G_LINE, 0.45), d.px);
            b.mesh(&m);
        };
        rule(b, 184.0);
        // what the promotion opens in Single race (the Track and Car lists' new entries)
        let codes = self.track_codes();
        let on = from.max(0) as usize;
        let unlock: Vec<usize> = if matches!(o, Outcome::Promoted(_) | Outcome::Won) { league::season_tracks(&codes, on).into_iter().chain(league::bonus_tracks(&codes, on)).collect() } else { Vec::new() };
        let flagged = self.unlock_flags().unwrap_or_else(|| vec![true; self.tracks.len()]);
        let (tracks, cars) = league::new_unlocks(&codes, &flagged, &self.car_codes(), &unlock);
        let names: Vec<String> = tracks.filter_map(|i| self.tracks.get(i)).map(|t| t.name.to_uppercase()).chain(cars.filter_map(|i| self.cars.get(i)).map(|c| c.name.to_uppercase())).collect();
        let (x, y) = at(22.0, 196.0);
        if !names.is_empty() {
            Self::glass_icon(b, ui_font::icon::LOCK_OPEN, x, d.cap_top(HudFont::GlassMed, 11.0, y, None) - d.d(1.0), d.d(13.0), hex(G_DIM, 1.0));
            Self::league_cap(b, d, "NEW IN SINGLE RACE", x + d.d(19.0), y);
            let (x0, mut cy2) = at(22.0, 216.0);
            let mut tx = x0;
            for name in &names {
                let w = Self::tag_width(b, d, name);
                if tx + w > x0 + d.d(CARD_W) {
                    tx = x0;
                    cy2 += d.d(24.0);
                }
                Self::chip(b, d, name, (tx, cy2), w, 0xffffff);
                tx += w + d.d(6.0);
            }
        } else {
            let (title, text) = if o == Outcome::Won {
                ("THE LEAGUE IS OVER", "The league's file is deleted. A new league starts in Division 3.")
            } else {
                ("NOTHING NEW IN SINGLE RACE", "This season opens no new tracks or cars.")
            };
            Self::glass_icon(b, if o == Outcome::Won { ui_font::icon::FLAG } else { ui_font::icon::LOCK_OPEN }, x, d.cap_top(HudFont::GlassMed, 11.0, y, None) - d.d(1.0), d.d(13.0), hex(G_DIM, 1.0));
            Self::league_cap(b, d, title, x + d.d(19.0), y);
            let (x, y) = at(22.0, 216.0);
            for (k, t) in wrap_font(b, d, HudFont::GlassMed, 14.0, 0.5, text, CARD_W).iter().enumerate() {
                txt(b, d, HudFont::GlassMed, 14.0, 0.5, hex(G_TEXT, 1.0), t, x, y + d.d(19.0 * k as f32), Some(19.0));
            }
        }
        rule(b, 284.0);
        // what follows: the season's movie, else the League menu
        let movie = self.has_movies && o.movie().is_some();
        let (x, y) = at(22.0, 296.0);
        Self::glass_icon(b, if movie { ui_font::icon::FILM } else { ui_font::icon::LOG_OUT }, x, y + d.d(6.0), d.d(20.0), hex(G_DIM, 1.0));
        Self::league_cap(b, d, "NEXT", x + d.d(30.0), y);
        let what = if movie { format!("THE SEASON'S MOVIE \u{b7} {}", o.movie_name()) } else { "THE LEAGUE MENU".into() };
        txt(b, d, HudFont::Glass, 16.0, 0.5, white, &what, x + d.d(30.0), y + d.d(14.0), Some(19.0));
    }

    /// The outcome word in the Glass chromatic style (`.chroma`), shrunk to fit `max_w` design px; `(x, top)` = its line
    /// box (46 high) in canvas px.
    fn league_word(b: &mut Batch, d: Glass, word: &str, (x, top): (f32, f32), max_w: f32) {
        let name = hex(0xf4fbff, 1.0);
        let w = b.font_width(HudFont::Glass, word, &d.font(HudFont::Glass, 40.0, 2.0, name));
        let k = (d.d(max_w) / w).min(1.0);
        let (px, ls) = (40.0 * k, 2.0 * k);
        let st = d.font(HudFont::Glass, px, ls, name);
        let y = d.cap_top(HudFont::Glass, px, top, Some(46.0));
        let nw = b.font_width(HudFont::Glass, word, &st) - st.tracking;
        let mut m = Mesh::default();
        // APPROX: the 26 px text glow as a soft pad behind the run, as the Track selector
        m.soft_pad(&vec::rounded_rect(x + d.d(4.0), y + d.d(4.0), nw - d.d(8.0), st.size - d.d(8.0), d.d(10.0)), d.d(26.0), [120.0 / 255.0, 200.0 / 255.0, 1.0, 0.22]);
        b.mesh(&m);
        b.font_text(HudFont::Glass, word, x - d.d(1.5), y, &FontStyle { color: [1.0, 70.0 / 255.0, 70.0 / 255.0, 0.55], ..st });
        b.font_text(HudFont::Glass, word, x + d.d(1.5), y, &FontStyle { color: [70.0 / 255.0, 200.0 / 255.0, 1.0, 0.55], ..st });
        b.font_text(HudFont::Glass, word, x, y, &st);
    }

    /// `ladderHTML`: DIV 3, DIV 2, DIV 1, FINAL with your move drawn (`from` to `to`; -1 = the league won). `(ox, oy)` =
    /// the box's top left in canvas px, 332 design px wide.
    fn league_ladder(b: &mut Batch, d: Glass, (ox, oy): (f32, f32), from: i32, to: i32) {
        let x = |dv: i32| ox + d.d(14.0 + (CARD_W - 28.0) * (3 - dv) as f32 / 3.0);
        let y = oy + d.d(14.0);
        let won = to < 0;
        let to_x = if won { x(0) } else { x(to) };
        let mut m = Mesh::default();
        m.dashed([x(3), y], [x(0), y], d.d(4.0), d.d(4.0), d.w(2.0), hex(G_LINE, 0.25), d.px);
        if (x(from) - to_x).abs() > 0.5 {
            let seg = [[x(from), y], [to_x, y]];
            m.glow(&seg, false, d.d(1.5), d.d(5.0), hex(G_ORANGE, 0.6), 3);
            m.stroke(&seg, false, d.w(3.0), Ends::Round, hex(G_ORANGE, 1.0), d.px);
        }
        for dv in [3, 2, 1, 0] {
            let on = if won { dv == 0 } else { dv == to };
            let was = dv == from && !won;
            let c = [x(dv), y];
            let r = d.d(if on { 8.0 } else { 5.0 });
            if on {
                m.halo(&circle(c, r), d.d(8.0), hex(G_ORANGE, 0.8));
            }
            m.fill(&circle(c, r), hex(if on { G_ORANGE } else if was { G_S1 } else { 0x04070d }, 1.0), d.px);
            let rim = if on { hex(0xffffff, 1.0) } else { hex(G_LINE, 0.6) };
            m.stroke(&circle(c, r), true, d.w(if on { 2.4 } else { 1.5 }), Ends::Round, rim, d.px);
        }
        b.mesh(&m);
        for dv in [3, 2, 1, 0] {
            let on = if won { dv == 0 } else { dv == to };
            let label = if dv == 0 { "FINAL".to_string() } else { format!("DIV {dv}") };
            let st = d.font(HudFont::Glass, 10.0, 1.5, WHITE);
            let w = b.font_width(HudFont::Glass, &label, &st) - st.tracking;
            txt(b, d, HudFont::Glass, 10.0, 1.5, hex(if on { G_ORANGE } else { G_DIM }, 1.0), &label, x(dv) - w * 0.5, oy + d.d(28.0), None);
        }
    }

    /// `.tag-d`: a label in `col` (white for the unlock chips, the dim blue of `.tag-d`'s own colour for a quiet note)
    /// on the dark tag ground with a faint rim, 18 design px high, top left at (x, y), `w` canvas px wide.
    pub(super) fn chip(b: &mut Batch, d: Glass, text: &str, (x, y): (f32, f32), w: f32, col: u32) {
        let r = vec::rounded_rect(x, y, w, d.d(18.0), d.d(7.0));
        let mut m = Mesh::default();
        m.fill(&r, hex(0x040a16, 0.5), d.px);
        m.stroke(&r, true, d.w(1.0), Ends::Round, hex(G_LINE, 0.25), d.px);
        b.mesh(&m);
        txt(b, d, HudFont::Glass, 10.0, 1.5, hex(col, 1.0), text, x + d.d(8.0), y, Some(18.0));
    }
}
