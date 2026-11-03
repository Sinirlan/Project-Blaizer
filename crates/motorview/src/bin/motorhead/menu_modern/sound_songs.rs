//! REMASTER Sound screen, Songs tab (#259): the song-to-track table of the Classic Sound screen in the Glass HUD.
//! Mockup `reference_materials/sound-overhaul/sound.html`: the tab bar (Volume / Songs, `glass_tabbar`), a table of the
//! race tracks (number, name, the CD track with its title and `<` `>` arrows on the selected row, length) in the rows
//! card's place, and the info card with the choice, its default and a preview strip. Design px through `Glass`.

use super::pause_settings::{glass_panel, wrap_text, INFO_CARD, ROWS_CARD};
use super::*;

/// The tabs of the Sound screen's tab bar.
pub(super) const SOUND_TABS: [(&str, char); 2] = [("VOLUME", ui_font::icon::VOLUME), ("SONGS", ui_font::icon::MUSIC)];

/// design px: the first row's top from the panel's inner top, the rows' height and pitch
const ROWS_Y: f32 = 34.0;
const ROW_H: f32 = 36.0;
const ROW_STEP: f32 = 37.0;
/// the faded text of a length that is unknown (`rgba(191,227,243,.35)`)
const NONE: u32 = 0xbfe3f3;

/// `m:ss`
fn mmss(secs: u32) -> String {
    format!("{}:{:02}", secs / 60, secs % 60)
}

impl Game {
    /// The Songs tab: the table, the info card and the hints.
    pub(super) fn songs_body(&self, b: &mut Batch, d: Glass, pulse: f32) {
        self.songs_table(b, d, pulse);
        self.songs_info(b, d);
        if self.asking() {
            return;
        }
        let go = if self.cd_preview.is_some() { "STOP" } else { "PLAY" };
        Self::glass_hints(b, d, self.controls.last, &[(&[4, 5], "PAGES"), (&[9, 10], "MOVE"), (&[0, 1], "CD TRACK"), (&[2], go), (&[3], "BACK")], 626.0);
    }

    /// The table's selected row: the list shows every race track (the panel holds eight rows).
    fn songs_sel(&self) -> usize {
        self.sound_list.min(self.tracks.len().saturating_sub(1))
    }

    /// The title of CD track `cd` (empty when `Music/` has no such file).
    fn cd_title(&self, cd: u32) -> String {
        self.cd_names.get(&cd).map(|s| s.to_uppercase()).unwrap_or_default()
    }

    /// The panel with its headings and one row per race track.
    fn songs_table(&self, b: &mut Batch, d: Glass, pulse: f32) {
        let (px, py, pw, ph) = ROWS_CARD;
        let mut m = Mesh::default();
        glass_panel(&mut m, d, (d.x(px), d.y(py), d.d(pw), d.d(ph)));
        b.mesh(&m);
        // the panel's inner box (the card's 1 px border)
        let (ix, iy) = (px + 1.0, py + 1.0);
        let th = d.font(HudFont::GlassMed, 11.0, 1.5, hex(G_DIM, 1.0));
        let head_y = d.cap_top(HudFont::GlassMed, 11.0, d.y(iy + 12.0), None);
        b.font_text(HudFont::GlassMed, "NO.", d.x(ix + 34.0), head_y, &th);
        b.font_text(HudFont::GlassMed, "RACE TRACK", d.x(ix + 74.0), head_y, &th);
        b.font_text(HudFont::GlassMed, "CD TRACK", d.x(ix + 344.0), head_y, &th);
        let len_w = b.font_width(HudFont::GlassMed, "LENGTH", &th) - th.tracking;
        b.font_text(HudFont::GlassMed, "LENGTH", d.x(ix + pw - 2.0 - 34.0) - len_w, head_y, &th);

        let sel = self.songs_sel();
        let fit = ((ph - 2.0 - ROWS_Y + 1.0) / ROW_STEP) as usize;
        let (row_x, row_w) = (ix + 10.0, pw - 2.0 - 20.0);
        for (k, tr) in self.tracks.iter().enumerate().take(fit) {
            let top = iy + ROWS_Y + k as f32 * ROW_STEP;
            let on = k == sel;
            let rect = (d.x(row_x), d.y(top), d.d(row_w), d.d(ROW_H));
            let mut m = Mesh::default();
            // a rule above the row, hidden next to the selected one
            if k > 0 && !on && k - 1 != sel {
                m.fill(&vec::rounded_rect(d.x(row_x + 9.0), d.y(top - 2.0), d.d(row_w - 18.0), d.w(1.0), 0.0), hex(G_LINE, 0.08), d.px);
            }
            if on {
                let rim = vec::rounded_rect(rect.0, rect.1, rect.2, rect.3, d.d(9.0));
                m.halo(&rim, d.d(14.0), hex(G_ORANGE, 0.3 + 0.2 * pulse));
                m.fill(&rim, hex(G_ORANGE, 0.06), d.px);
                m.stroke(&rim, true, d.w(1.0), Ends::Round, hex(G_ORANGE, 1.0), d.px);
            }
            b.mesh(&m);
            // the cells, from the row's inner box (its 1 px border)
            let (cx, cy) = (row_x + 1.0, top + 1.0);
            let mid = |f: HudFont, px: f32| d.cap_top(f, px, d.y(cy), Some(ROW_H - 2.0));
            let cd = self.race_cd_track(k);
            let default = tr.cd_track;
            b.font_text(HudFont::Glass, &format!("{:02}", tr.n), d.x(cx + 12.0), mid(HudFont::Glass, 16.0), &d.font(HudFont::Glass, 16.0, 0.5, hex(G_DIM, 1.0)));
            let name = tr.name.to_uppercase();
            let st = d.font(HudFont::Glass, 17.0, 0.5, hex(if on { G_ORANGE } else { G_TEXT }, 1.0));
            let nw = b.font_width(HudFont::Glass, &name, &st);
            b.font_text(HudFont::Glass, &name, d.x(cx + 52.0), mid(HudFont::Glass, 17.0), &st);
            if cd != default {
                Self::glass_tag(b, d, "CHANGED", (d.x(cx + 52.0) + nw + d.d(10.0), d.y(cy + (ROW_H - 2.0) * 0.5) - d.d(9.0)));
            }
            // the CD track: `<` the number, its title `>`; the arrows only on the selected row, dimmed at the ends
            let x0 = cx + 300.0;
            let n = format!("{cd:02}");
            let ns = d.font(HudFont::Glass, 17.0, 0.5, WHITE);
            let nw = b.font_width(HudFont::Glass, &n, &ns) - ns.tracking;
            b.font_text(HudFont::Glass, &n, d.x(x0 + 45.0) - nw, mid(HudFont::Glass, 17.0), &ns);
            b.font_text(HudFont::GlassMed, &self.cd_title(cd), d.x(x0 + 55.0), mid(HudFont::GlassMed, 13.0), &d.font(HudFont::GlassMed, 13.0, 1.0, hex(G_DIM, 1.0)));
            if on {
                let (first, last) = match self.cd_tracks.as_slice() {
                    [] => (cd <= 2, cd >= CD_LAST_TRACK),
                    t => (cd <= t[0], cd >= t[t.len() - 1]),
                };
                let (cmid, mut m) = (d.y(cy + (ROW_H - 2.0) * 0.5), Mesh::default());
                Self::glass_tri(&mut m, d, d.x(x0), cmid, true, true, 1.0, first);
                Self::glass_tri(&mut m, d, d.x(x0 + 250.0 - 9.0), cmid, false, true, 1.0, last);
                b.mesh(&m);
            }
            // the length at the right, the equaliser left of it while this row plays
            let playing = on && self.cd_preview.is_some();
            let right = d.x(row_x + row_w - 1.0 - 24.0);
            let (text, col) = match self.cd_lengths.get(&cd) {
                Some(&s) => (mmss(s), hex(0xffffff, 1.0)),
                None => ("--:--".to_string(), hex(NONE, 0.35)),
            };
            let ls = d.font(HudFont::Glass, 17.0, 0.5, col);
            let lw = b.font_width(HudFont::Glass, &text, &ls) - ls.tracking;
            b.font_text(HudFont::Glass, &text, right - lw, mid(HudFont::Glass, 17.0), &ls);
            if playing {
                self.equaliser(b, d, right - lw - d.d(10.0 + 14.0), d.y(cy + (ROW_H - 2.0) * 0.5));
            }
        }
    }

    /// `.eq`: three 14 px bars moving at different phases, the left edge at canvas `x`, centred on `mid`.
    fn equaliser(&self, b: &mut Batch, d: Glass, x: f32, mid: f32) {
        let mut m = Mesh::default();
        let bar = d.d((14.0 - 4.0) / 3.0);
        for i in 0..3 {
            let phase = i as f32 * 0.25;
            let k = 0.25 + 0.75 * (0.5 + 0.5 * (std::f32::consts::TAU * (self.t / 1.4 - phase)).sin());
            let h = d.d(14.0) * k;
            let r = vec::rounded_rect(x + (bar + d.d(2.0)) * i as f32, mid + d.d(7.0) - h, bar, h, d.d(1.0));
            m.halo(&r, d.d(6.0), hex(G_ORANGE, 0.5));
            m.fill(&r, hex(G_ORANGE, 1.0), d.px);
        }
        b.mesh(&m);
    }

    /// The info card: the selected race track and its song, the default, the preview strip and its note.
    fn songs_info(&self, b: &mut Batch, d: Glass) {
        let (ix, iy, iw, ih) = INFO_CARD;
        let (x, y) = (d.x(ix), d.y(iy));
        let mut m = Mesh::default();
        glass_card(&mut m, d, (x, y, d.d(iw), d.d(ih)), d.d(12.0), (0.13, 0.04), 0.36, (10.0, 26.0, 0.45), 0.35, None);
        b.mesh(&m);
        let Some(tr) = self.tracks.get(self.songs_sel()) else { return };
        let sel = self.songs_sel();
        let cd = self.race_cd_track(sel);
        let (default, changed) = (tr.cd_track, self.race_cd_track(sel) != tr.cd_track);
        let len = self.cd_lengths.get(&cd).copied();
        let playing = self.cd_preview.is_some();
        // the title block beside the icon
        Self::glass_icon(b, ui_font::icon::MUSIC, x + d.d(20.0), y + d.d(22.5), d.d(30.0), WHITE);
        b.font_text(HudFont::Glass, &tr.name.to_uppercase(), x + d.d(62.0), d.cap_top(HudFont::Glass, 25.0, y + d.d(18.0), Some(26.0)), &d.font(HudFont::Glass, 25.0, 0.5, WHITE));
        let cap = d.font(HudFont::GlassMed, 11.0, 1.5, hex(G_DIM, 1.0));
        let caption = format!("SONGS · TRACK {:02} · {} OF {}", tr.n, sel + 1, self.tracks.len());
        b.font_text(HudFont::GlassMed, &caption, x + d.d(62.0), d.cap_top(HudFont::GlassMed, 11.0, y + d.d(44.0), None), &cap);
        // the help, wrapped to 396 design px
        let st = d.font(HudFont::GlassMed, 15.0, 0.5, hex(G_TEXT, 1.0));
        let lines = wrap_text(b, HudFont::GlassMed, &st, "The CD track this race plays. Left and Right pick one of the tracks found in the Music folder; Enter plays it here.", d.d(396.0));
        for (k, line) in lines.iter().enumerate() {
            b.font_text(HudFont::GlassMed, line, x + d.d(20.0), d.cap_top(HudFont::GlassMed, 15.0, y + d.d(78.0 + 20.0 * k as f32), Some(20.0)), &st);
        }
        let dashed = |b: &mut Batch, at: f32| {
            let mut m = Mesh::default();
            m.dashed([x + d.d(20.0), y + d.d(at)], [x + d.d(416.0), y + d.d(at)], d.d(6.0), d.d(4.0), d.w(1.0), hex(G_LINE, 0.45), d.px);
            b.mesh(&m);
        };
        let rule = (78.0 + 20.0 * lines.len() as f32 + 6.0).max(140.0);
        dashed(b, rule);
        // the choice: its number big, its title after it, the default under it
        let cap_at = |b: &mut Batch, text: &str, top: f32| b.font_text(HudFont::GlassMed, text, x + d.d(20.0), d.cap_top(HudFont::GlassMed, 11.0, y + d.d(top), None), &cap);
        cap_at(b, "CD TRACK", rule + 10.0);
        if changed {
            let w = Self::tag_width(b, d, "CHANGED");
            Self::glass_tag(b, d, "CHANGED", (x + d.d(416.0) - w, y + d.d(rule + 10.0 + 6.0) - d.d(9.0)));
        }
        let num = format!("{cd:02}");
        let ns = d.font(HudFont::Glass, 40.0, 1.0, WHITE);
        let top40 = d.cap_top(HudFont::Glass, 40.0, y + d.d(rule + 30.0), Some(44.0));
        b.font_text(HudFont::Glass, &num, x + d.d(20.0), top40, &ns);
        let title = self.cd_title(cd);
        if !title.is_empty() {
            let nw = b.font_width(HudFont::Glass, &num, &ns);
            // the title shares the number's baseline
            let top20 = top40 + d.s * 20.0 * ui_font::face(HudFont::Glass).cap / ui_font::SRC;
            b.font_text(HudFont::Glass, &title, x + d.d(20.0) + nw + d.d(14.0), top20, &d.font(HudFont::Glass, 20.0, 1.0, hex(G_TEXT, 1.0)));
        }
        let note = d.font(HudFont::GlassMed, 12.0, 1.0, hex(G_DIM, 0.85));
        let own = if changed { format!("DEFAULT: {default:02} {}", self.cd_title(default)).trim_end().to_string() } else { "THE TRACK'S OWN SONG".to_string() };
        b.font_text(HudFont::GlassMed, &own, x + d.d(20.0), d.cap_top(HudFont::GlassMed, 12.0, y + d.d(rule + 82.0), None), &note);
        dashed(b, rule + 112.0);
        cap_at(b, "PREVIEW", rule + 122.0);
        // the strip: the play / stop button, the progress, the time
        let (bx, by) = (x + d.d(20.0), y + d.d(rule + 146.0));
        let (r, mid) = (d.d(17.0), by + d.d(17.0));
        let tint = if playing { G_ORANGE } else { 0x8fd0ff };
        let disc = circle([bx + r, mid], r);
        let mut m = Mesh::default();
        if playing {
            m.halo(&disc, d.d(12.0), hex(G_ORANGE, 0.45));
        }
        m.fill(&disc, hex(tint, 0.12), d.px);
        m.stroke(&disc, true, d.w(1.0), Ends::Round, hex(tint, if playing { 1.0 } else { 0.6 }), d.px);
        b.mesh(&m);
        let glyph = if playing { ui_font::icon::SQUARE } else { ui_font::icon::PLAY };
        Self::glass_icon(b, glyph, bx + r - d.d(8.0), mid - d.d(8.0), d.d(16.0), hex(if playing { G_ORANGE } else { 0xffffff }, 1.0));
        let time_w = d.d(92.0);
        let (bar_x, bar_end) = (bx + d.d(34.0 + 12.0), x + d.d(416.0) - time_w - d.d(12.0));
        let frac = match (playing, len) {
            (true, Some(l)) if l > 0 => (self.preview_t / l as f32).clamp(0.0, 1.0),
            _ => 0.0,
        };
        let track = vec::rounded_rect(bar_x, mid - d.d(3.0), bar_end - bar_x, d.d(6.0), d.d(3.0));
        let mut m = Mesh::default();
        m.fill(&track, hex(G_LINE, 0.12), d.px);
        if frac > 0.0 {
            let fill = vec::rounded_rect(bar_x, mid - d.d(3.0), (bar_end - bar_x) * frac, d.d(6.0), d.d(3.0));
            m.halo(&fill, d.d(8.0), hex(G_ORANGE, 0.5));
            m.fill(&fill, hex(G_ORANGE, 1.0), d.px);
        }
        b.mesh(&m);
        let (done, total) = match len {
            Some(l) => (mmss(if playing { self.preview_t as u32 } else { 0 }), format!("/ {}", mmss(l))),
            None => ("--:--".to_string(), "/ --:--".to_string()),
        };
        let ts = d.font(HudFont::Glass, 16.0, 1.0, if len.is_some() { WHITE } else { hex(G_DIM, 1.0) });
        let ds = d.font(HudFont::Glass, 16.0, 1.0, hex(G_DIM, 1.0));
        let end = x + d.d(416.0);
        let tw = b.font_width(HudFont::Glass, &total, &ds) - ds.tracking;
        let dw = b.font_width(HudFont::Glass, &done, &ts) - ts.tracking;
        let ty = d.cap_top(HudFont::Glass, 16.0, mid - d.d(10.0), Some(20.0));
        b.font_text(HudFont::Glass, &total, end - tw, ty, &ds);
        b.font_text(HudFont::Glass, &done, end - tw - d.d(6.0) - dw, ty, &ts);
        // the note under the strip
        let text = if self.cd_tracks.is_empty() {
            "NO CD TRACKS FOUND IN THE MUSIC FOLDER · LENGTHS UNKNOWN"
        } else if playing {
            "PLAYING · ENTER STOPS · LEAVING THE TAB STOPS IT"
        } else if !self.settings.music {
            "CD MUSIC IS OFF · TURN IT ON IN VOLUME"
        } else if self.settings.vol_music == 0 {
            "THE CD VOLUME IS 0% · RAISE IT IN VOLUME"
        } else {
            "PLAYS AT THE CD VOLUME · SILENT WHEN IT IS 0%"
        };
        b.font_text(HudFont::GlassMed, text, x + d.d(20.0), d.cap_top(HudFont::GlassMed, 12.0, y + d.d(rule + 192.0), None), &note);
    }
}
