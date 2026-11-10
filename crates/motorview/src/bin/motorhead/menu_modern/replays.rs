//! REMASTER Single > Ghost mode (#92 phase 3, concept B · Telemetry of `reference_materials/ghost-overhaul/B-telemetry.html`,
//! shots `shots/B-*.jpg`): the recordings of `Demos/*.MDE` as slot cards down the left and a telemetry panel for the selected
//! one: its map with the ghost running the best lap (live lap time and speed), the race time, best lap, top speed and your
//! best lap against it (Ghost race) or the finish (Replay race), the speed trace of the best lap or the finish order, and the
//! lap bars. The mode is two chips flipped with left / right. The numbers are worked out once when the list is read
//! (`motor_sim::demo_stats`, [D]). Keys in `replays_key_modern`; Classic keeps `Game::replays_screen`
//! (CLASSIC 0x462bc8). Design px through `Glass`, 1:1 from the mockup's inline styles (08-missing-features 8.9).

use super::league_load::{line_box, window_first, GAIN, INNER_W, LOSS, NOTE, PANEL, SLOT, STEP, VIEW};
use super::pause_settings::glass_panel;
use super::results::{gap_text, lap_text, txt, txt_r};
use super::*;
use motor_sim::demo_stats::{self, Stats};

/// What the screen shows of a recording beyond the file: worked out when the list is read.
#[derive(Clone, Debug, Default)]
pub(crate) struct RecInfo {
    /// the laps, times and speeds (None when the track is not installed or the file has too little in it)
    pub stats: Option<Stats>,
    /// the file came with the game (the install's `Demos/`)
    pub install: bool,
    /// when the file was written, `yyyy-mm-dd`
    pub date: Option<String>,
    /// your best lap on that track (HiScores `.CL`), ms
    pub your_best: Option<u32>,
}

/// `.tr` of the mockup's `speedCol`: deep blue (slow) to ice white (fast).
fn speed_col(f: f32) -> [f32; 4] {
    let (a, b) = ([61.0, 139.0, 255.0], [230.0, 246.0, 255.0]);
    let c = |i: usize| (a[i] + (b[i] - a[i]) * f.clamp(0.0, 1.0)) / 255.0;
    [c(0), c(1), c(2), 1.0]
}

/// A race time: `m:ss.mmm`, `h:mm:ss.mmm` past an hour.
pub(super) fn race_text(ms: u32) -> String {
    let (h, m, s, f) = (ms / 3_600_000, ms / 60_000 % 60, ms / 1000 % 60, ms % 1000);
    if h > 0 { format!("{h}:{m:02}:{s:02}.{f:03}") } else { format!("{m}:{s:02}.{f:03}") }
}

/// `yyyy-mm-dd` of a day count since 1970-01-01 (the civil calendar).
pub(super) fn ymd(days: u64) -> String {
    let z = days as i64 + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    format!("{}-{:02}-{:02}", yoe + era * 400 + i64::from(month <= 2), month, day)
}

impl Game {
    /// Work a recording out for the Glass screen (`load_replays`).
    pub(crate) fn rec_info(&self, path: &std::path::Path, m: &Mde) -> RecInfo {
        let tr = self.tracks.iter().find(|t| t.name.eq_ignore_ascii_case(&m.header.track));
        let stats = tr.and_then(|t| t.profile.as_ref()).and_then(|p| demo_stats::analyse(m, p));
        let date = std::fs::metadata(path)
            .and_then(|x| x.modified())
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| ymd(d.as_secs() / 86_400));
        let dir = self.hiscore_dir.clone().or_else(|| hiscore::dir(self.persist));
        let your_best = tr.and_then(|t| hiscore::Records::load(dir.as_deref(), &t.name, m.header.laps).lap).map(|l| l.lap_ms).filter(|&v| v > 0);
        RecInfo { stats, install: path.starts_with(self.root.join("Demos")), date, your_best }
    }

    /// Why Enter cannot start a recording, as the tag on it: the track is not installed or not open yet.
    fn rec_locked(&self, m: &Mde) -> Option<&'static str> {
        self.ghost_track(m).err().map(|e| if e.contains("not found") { "TRACK NOT FOUND" } else { "NOT YET AVAILABLE" })
    }

    /// Keys of the Glass screen (`Screen::Replays`): up / down the recordings (the ends stop with their sound), left / right
    /// and Q / E the mode, Enter races or watches, Del / the pad's X deletes behind the requester (NO! preselected), F / the
    /// pad's Y opens Lap Analysis (#429).
    pub(crate) fn replays_key_modern(&mut self, k: KeyCode) {
        let n = self.replays.len();
        let mut e = None;
        match k {
            KeyCode::ArrowUp | KeyCode::ArrowDown if n > 0 => {
                let to = if k == KeyCode::ArrowDown { (self.sel + 1).min(n - 1) } else { self.sel.saturating_sub(1) };
                e = Some(if to == self.sel {
                    ev::CURSOR_END
                } else if k == KeyCode::ArrowUp {
                    ev::CURSOR_UP
                } else {
                    ev::CURSOR_DOWN
                });
                self.sel = to;
            }
            KeyCode::ArrowLeft | KeyCode::ArrowRight | KeyCode::PageUp | KeyCode::PageDown | KeyCode::KeyQ | KeyCode::KeyE => {
                let to = usize::from(matches!(k, KeyCode::ArrowRight | KeyCode::PageDown | KeyCode::KeyE));
                e = Some(if to == self.ghost_mode { ev::CURSOR_END } else { ev::SELECT });
                self.ghost_mode = to;
            }
            KeyCode::Delete | KeyCode::F6 if self.sel < n => {
                self.ghost_del = Some(1);
                e = Some(ev::REQUESTER);
            }
            KeyCode::Enter | KeyCode::NumpadEnter | KeyCode::Space => e = Some(self.ghost_pick()),
            // ANALYSE (#429): Lap Analysis of this recording; F, the pad's Y (F5)
            KeyCode::KeyF | KeyCode::F5 if self.sel < n => {
                e = Some(if self.ana_from_ghost(self.sel) {
                    self.go(Screen::Analysis);
                    ev::SELECT
                } else {
                    ev::SELECT_ERROR
                });
            }
            KeyCode::Escape | KeyCode::Backspace => self.back_to_dial(Screen::Single, 5),
            _ => {}
        }
        self.ev_override = e;
    }

    /// The glass the hologram is frosted behind: the plate, the slot cards, the panel (canvas px, with the radius).
    fn replays_cards(&self, d: Glass) -> Vec<(f32, f32, f32, f32, f32)> {
        let plate = (d.r(1021.0), d.t(GLASS_PLATE_TOP), d.d(234.0), d.d(119.0));
        let rect = |(x, y, w, h): (f32, f32, f32, f32)| (d.x(x), d.y(y), d.d(w), d.d(h));
        let n = self.replays.len();
        let mut cards = vec![plate];
        if n == 0 {
            cards.push(rect((62.0, 228.0, 1156.0, 372.0)));
        } else {
            cards.extend((0..VIEW.min(n)).map(|k| rect((SLOT.0, SLOT.1 + STEP * k as f32, SLOT.2, SLOT.3))));
            cards.push(rect(PANEL));
        }
        cards.into_iter().map(|(x, y, w, h)| (x, y, w, h, d.d(12.0))).collect()
    }

    /// REMASTER Single > Ghost mode (#92): the Glass frame, the recording cards and the selected recording's panel.
    pub(crate) fn replays_modern(&self, size: (u32, u32)) -> Vec<Layer> {
        let canvas = Canvas::hud(size);
        let px = 1.0 / canvas.transform(size).0;
        let d = Glass::new(&canvas, px);
        let cards = self.replays_cards(d);
        let back = self.ui_in(Batch::sharp_text(&self.assets, canvas, 1.0), |_, b| Self::glass_backdrop(b, d));
        let front = self.ui_in(Batch::sharp_text(&self.assets, canvas, 1.0), |g, b| {
            let pulse = 0.5 - 0.5 * (g.t * TAU / 1.1).cos();
            let header = format!("MAIN // SINGLE // GHOST MODE // {}", ["GHOST RACE", "REPLAY RACE"][g.ghost_mode.min(1)]);
            // new codes in the series of the others
            Self::glass_options_frame(b, d, g.t, "ghost", &header, "88-137:61", "GHOSTSCREEN", "187-C-61");
            if g.replays.is_empty() {
                g.replays_empty(b, d);
            } else {
                g.replays_body(b, d, pulse);
            }
            if !g.asking() {
                let verb = ["RACE", "WATCH"][g.ghost_mode.min(1)];
                let dev = g.controls.last;
                if g.replays.is_empty() {
                    Self::glass_hints(b, d, dev, &[(&[3], "BACK")], 626.0);
                } else {
                    let locked = g.replays.get(g.sel).is_some_and(|(_, m)| g.rec_locked(m).is_some());
                    // RACE / WATCH dimmed on a locked track, ANALYSE (#429) without a lap to compare
                    let off: Vec<usize> = [(2, locked), (4, !g.ana_possible(g.sel))].iter().filter(|x| x.1).map(|x| x.0).collect();
                    let items: [(&[usize], &str); 6] = [(&[9, 10], "RECORDING"), (&[0, 1], "MODE"), (&[2], verb), (&[16], "DELETE"), (&[17], "ANALYSE"), (&[3], "BACK")];
                    Self::glass_hints_off(b, d, dev, &items, (640.0, 626.0), (1.0, 28.0), &off);
                }
            }
        });
        let mut out = vec![back];
        out.extend(self.hologram(size, None, &cards));
        out.push(front);
        out
    }

    /// No recordings: how to make one.
    fn replays_empty(&self, b: &mut Batch, d: Glass) {
        let (x, y, w, h) = (62.0, 228.0, 1156.0, 372.0);
        let mut m = Mesh::default();
        glass_panel(&mut m, d, (d.x(x), d.y(y), d.d(w), d.d(h)));
        b.mesh(&m);
        // icon 54, 8, heading 30 (25 px), 8, three lines of 20: centred in the card
        let top = y + (h - (54.0 + 8.0 + 30.0 + 8.0 + 60.0)) * 0.5;
        let cx = d.x(x + w * 0.5);
        Self::glass_icon(b, ui_font::icon::GHOST, cx - d.d(27.0), d.y(top), d.d(54.0), hex(G_DIM, 0.7));
        let head = "NO RECORDINGS YET";
        let hw = b.font_width(HudFont::Glass, head, &d.font(HudFont::Glass, 25.0, 1.0, WHITE));
        txt(b, d, HudFont::Glass, 25.0, 1.0, hex(0xffffff, 1.0), head, cx - hw * 0.5, d.y(top + 62.0), Some(30.0));
        let lines: [&[(&str, bool)]; 3] = [
            &[("Switch on ", false), ("RECORD RACE", true), (" on the Car screen.", false)],
            &[("When the race ends you name it, and it waits", false)],
            &[("here to be raced or watched.", false)],
        ];
        for (i, line) in lines.iter().enumerate() {
            let font = |bold: bool| if bold { HudFont::Glass } else { HudFont::GlassMed };
            let width: f32 = line.iter().map(|&(s, bold)| b.font_width(font(bold), s, &d.font(font(bold), 15.0, 0.5, WHITE))).sum();
            let mut lx = cx - width * 0.5;
            for &(s, bold) in line.iter() {
                lx += txt(b, d, font(bold), 15.0, 0.5, hex(if bold { 0xffffff } else { G_TEXT }, 1.0), s, lx, d.y(top + 100.0 + 20.0 * i as f32), Some(20.0));
            }
        }
    }

    /// The cards and the panel of the selected recording.
    fn replays_body(&self, b: &mut Batch, d: Glass, pulse: f32) {
        let n = self.replays.len();
        let sel = self.sel.min(n - 1);
        let f = window_first(sel, n);
        for (k, (path, m)) in self.replays.iter().enumerate().skip(f).take(VIEW) {
            let info = self.replay_info.get(k);
            self.replay_slot(b, d, (k - f, path, m, info), k == sel, pulse);
        }
        // `.scrollbar`: 3 px at x 370 past eight cards, the thumb the share shown
        if n > VIEW {
            let h = VIEW as f32 * STEP - 2.0;
            let mut m = Mesh::default();
            m.fill(&vec::rounded_rect(d.x(370.0), d.y(SLOT.1), d.d(3.0), d.d(h), d.d(2.0)), hex(G_LINE, 0.12), d.px);
            m.fill(&vec::rounded_rect(d.x(370.0), d.y(SLOT.1 + h * f as f32 / n as f32), d.d(3.0), d.d(h * VIEW as f32 / n as f32), d.d(2.0)), hex(G_LINE, 0.6), d.px);
            b.mesh(&m);
        }
        let (path, m) = &self.replays[sel];
        if let Some(info) = self.replay_info.get(sel) {
            self.replay_panel(b, d, path, m, info);
        }
    }

    /// `.slot`: the lap in a 36 px square, the name (the install's own files with a disc), the track, laps and time, and how
    /// many cars it holds (a lock on a track you cannot race yet).
    fn replay_slot(&self, b: &mut Batch, d: Glass, (pos, path, m, info): (usize, &std::path::Path, &Mde, Option<&RecInfo>), sel: bool, pulse: f32) {
        let (x, y) = (SLOT.0, SLOT.1 + STEP * pos as f32);
        Self::slot_ground(b, d, (x, y, SLOT.2, SLOT.3), sel, pulse);
        let locked = self.rec_locked(m).is_some();
        let fade = if locked { 0.55 } else { 1.0 };
        let (ix, iy) = (x + 1.0, y + 1.0);
        let tr = self.tracks.iter().find(|t| t.name.eq_ignore_ascii_case(&m.header.track));
        // the map (`.lap` at 14, 4; 36 px): lit in the sector colours on the selected card (not on a locked track), mono otherwise
        if let Some((tr, p)) = tr.and_then(|t| t.profile.as_ref().filter(|p| !p.outline.is_empty()).map(|p| (t, p))) {
            let line = Self::fit_lap(tr, p, [d.x(ix + 14.0 + 18.0), d.y(iy + 4.0 + 18.0)], d.d(22.0)).line(p);
            let mut ms = Mesh::default();
            ms.stroke(&line, true, d.w(3.2), Ends::Round, hex(G_DARK, 0.7), d.px);
            if sel && !locked {
                for (run, col) in sector_runs(p, &line).iter().zip([G_S1, G_S2, G_S3]).filter(|(r, _)| !r.is_empty()) {
                    ms.stroke(run, false, d.w(1.6), Ends::Round, hex(col, 0.85), d.px);
                }
            } else {
                ms.stroke(&line, true, d.w(1.6), Ends::Round, hex(G_TEXT, 0.55), d.px);
            }
            b.mesh(&ms);
        }
        // `.pn.none`: the lock, SOLO, or the users icon and the car count (13 px bold, dim)
        let right = d.x(ix + SLOT.2 - 2.0 - 14.0);
        let top = d.y(iy);
        let cars = m.header.active_slots().len();
        let dim = hex(0xbfe3f3, 0.45);
        let right_w = if locked {
            Self::glass_icon(b, ui_font::icon::LOCK, right - d.d(15.0), top + d.d((SLOT.3 - 2.0 - 15.0) * 0.5), d.d(15.0), hex(G_DIM, 1.0));
            d.d(15.0)
        } else if cars <= 1 {
            let w = b.font_width(HudFont::Glass, "SOLO", &d.font(HudFont::Glass, 13.0, 1.0, WHITE));
            txt_r(b, d, HudFont::Glass, 13.0, 1.0, dim, "SOLO", right, top, Some(SLOT.3 - 2.0));
            w
        } else {
            let num = cars.to_string();
            let w = b.font_width(HudFont::Glass, &num, &d.font(HudFont::Glass, 13.0, 1.0, WHITE));
            txt_r(b, d, HudFont::Glass, 13.0, 1.0, dim, &num, right, top, Some(SLOT.3 - 2.0));
            Self::glass_icon(b, ui_font::icon::USERS, right - w - d.d(5.0 + 15.0), top + d.d((SLOT.3 - 2.0 - 15.0) * 0.5), d.d(15.0), hex(G_DIM, 1.0));
            w + d.d(20.0)
        };
        // `.t`: the name, at most 186 px (cut with dots), the install's disc 8 px after it
        let tx = d.x(ix + 62.0);
        let st = d.font(HudFont::Glass, 18.0, 1.0, WHITE);
        let mut name = demo_name(path).to_uppercase();
        let room = d.d(186.0).min(right - right_w - d.d(12.0) - tx);
        if b.font_width(HudFont::Glass, &name, &st) > room {
            while name.chars().count() > 1 && b.font_width(HudFont::Glass, &format!("{name}..."), &st) > room {
                name.pop();
            }
            name.push_str("...");
        }
        let name_top = d.y(iy + 5.0);
        let nw = txt(b, d, HudFont::Glass, 18.0, 1.0, hex(if sel { G_ORANGE } else { G_TEXT }, fade), &name, tx, name_top, None);
        if info.is_some_and(|i| i.install) {
            Self::glass_icon(b, ui_font::icon::DISC, tx + nw + d.d(8.0), name_top + (d.d(line_box(HudFont::Glass, 18.0)) - d.d(14.0)) * 0.5, d.d(14.0), hex(G_DIM, fade));
        }
        // `.s`: `TRACK · n LAPS · ` then the time in white (the whole race, or where a quit one stopped)
        let laps = m.header.laps;
        let lead = format!("{} \u{b7} {} {} \u{b7} ", m.header.track.to_uppercase(), laps, if laps == 1 { "LAP" } else { "LAPS" });
        let time = match info.and_then(|i| i.stats.as_ref()) {
            Some(s) => match (s.race_ms(), s.quit_lap) {
                (Some(ms), _) => race_text(ms),
                (None, Some(l)) => format!("QUIT L{l}"),
                _ => race_text((m.seconds() * 1000.0) as u32),
            },
            None => race_text((m.seconds() * 1000.0) as u32),
        };
        let sub_top = d.y(iy + 26.0);
        let lw = txt(b, d, HudFont::GlassMed, 13.0, 0.5, hex(G_DIM, fade), &lead, tx, sub_top, None);
        txt(b, d, HudFont::Glass, 13.0, 0.5, hex(0xffffff, fade), &time, tx + lw, sub_top, None);
    }

    /// `.tag-d`: a dim tag (no accent), 18 design px high, its top left at canvas (x, y); returns its width.
    pub(super) fn tag_dim(b: &mut Batch, d: Glass, text: &str, (x, y): (f32, f32)) -> f32 {
        let w = Self::tag_width(b, d, text);
        let r = vec::rounded_rect(x, y, w, d.d(18.0), d.d(7.0));
        let mut m = Mesh::default();
        m.fill(&r, hex(0x040a16, 0.5), d.px);
        m.stroke(&r, true, d.w(1.0), Ends::Round, hex(G_LINE, 0.25), d.px);
        b.mesh(&m);
        txt(b, d, HudFont::Glass, 10.0, 1.5, hex(G_DIM, 1.0), text, x + d.d(8.0), y, Some(18.0));
        w
    }

    /// The panel of the selected recording.
    fn replay_panel(&self, b: &mut Batch, d: Glass, path: &std::path::Path, m: &Mde, info: &RecInfo) {
        let (px, py, pw, ph) = PANEL;
        let mut mesh = Mesh::default();
        glass_panel(&mut mesh, d, (d.x(px), d.y(py), d.d(pw), d.d(ph)));
        b.mesh(&mesh);
        let (ix, iy) = (px + 1.0, py + 1.0);
        let at = |x: f32, y: f32| (d.x(ix + x), d.y(iy + y));
        let right_edge = d.x(ix + INNER_W - 20.0);
        let h = &m.header;
        let stats = info.stats.as_ref();
        let locked = self.rec_locked(m);
        let mode = self.ghost_mode.min(1);
        let dim = hex(G_DIM, 1.0);
        let white = hex(0xffffff, 1.0);
        let note = hex(NOTE, 0.75);
        let cap = |b: &mut Batch, s: &str, x: f32, y: f32| {
            txt(b, d, HudFont::GlassMed, 11.0, 1.5, dim, s, x, y, None);
        };
        // the name 26 px in a 32 px line, then the tags 10 px apart
        let (x, y) = at(20.0, 10.0);
        let mut nm = demo_name(path).to_uppercase();
        let name_room = d.d(400.0);
        while nm.chars().count() > 1 && b.font_width(HudFont::Glass, &nm, &d.font(HudFont::Glass, 26.0, 1.0, WHITE)) > name_room {
            nm.pop();
        }
        let nw = txt(b, d, HudFont::Glass, 26.0, 1.0, white, &nm, x, y, Some(32.0));
        let mut tx = x + nw + d.d(10.0);
        let ty = y + d.d(7.0);
        if let Some(why) = locked {
            // 19.4: the tag blinks at the original's clock (on 512 ms of 1024)
            if self.t.rem_euclid(1.024) < 0.512 {
                tx += super::results::tag(b, d, why, (tx, ty), G_ORANGE, 0.6);
            } else {
                tx += Self::tag_width(b, d, why);
            }
            tx += d.d(10.0);
        }
        if info.install {
            tx += Self::tag_dim(b, d, "INSTALL", (tx, ty)) + d.d(10.0);
        }
        if let Some(l) = stats.and_then(|s| s.quit_lap) {
            Self::tag_dim(b, d, &format!("QUIT ON LAP {l} OF {}", h.laps), (tx, ty));
        }
        // `.cap`: the track, laps, catch-up and where the file came from
        let when = if info.install { "SHIPPED WITH THE GAME".to_string() } else { info.date.as_ref().map_or_else(|| "RECORDED".into(), |t| format!("RECORDED {t}")) };
        let caption = format!("{} \u{b7} {} {} \u{b7} CATCH-UP {} \u{b7} {when}", h.track.to_uppercase(), h.laps, if h.laps == 1 { "LAP" } else { "LAPS" }, if h.catchup() { "ON" } else { "OFF" });
        let (x, y) = at(20.0, 46.0);
        cap(b, &caption, x, y);
        self.replay_modes(b, d, (ix, iy), mode);
        let help = ["YOU RACE ALONE \u{b7} THE RECORDED CARS RUN AS GHOSTS", "RACE CAMERAS, HUD AND SOUND, AS RECORDED"][mode];
        txt_r(b, d, HudFont::GlassMed, 11.0, 1.0, hex(G_TEXT, 1.0), help, right_edge, d.y(iy + 50.0), None);
        // the dashed rule at 74
        let (a, z) = (at(18.0, 74.0), at(INNER_W - 18.0, 74.0));
        let mut mesh = Mesh::default();
        mesh.dashed([a.0, a.1], [z.0, z.1], d.d(6.0), d.d(4.0), d.w(1.0), hex(G_LINE, 0.45), d.px);
        b.mesh(&mesh);
        // the map with the ghost, its live readouts under it
        let trace: &[[f32; 4]] = stats.map_or(&[], |s| &s.trace);
        let best_ms = stats.and_then(|s| s.best).map_or(0, |(_, ms)| ms);
        let phase = if best_ms > 0 { ((self.t * 4000.0) % best_ms as f32) / best_ms as f32 } else { 0.0 };
        let tr = self.tracks.iter().find(|t| t.name.eq_ignore_ascii_case(&h.track));
        self.replay_disc(b, d, tr, [ix + 20.0 + 110.0, iy + 88.0 + 110.0], trace, phase, locked.is_some());
        let (lap_now, kmh) = if trace.is_empty() {
            (None, None)
        } else {
            let f = phase * trace.len() as f32;
            let (i, k) = (f.floor() as usize % trace.len(), f.fract());
            let (a, z) = (trace[i], trace[(i + 1) % trace.len()]);
            (Some((phase * best_ms as f32) as u32), Some((a[3] + (z[3] - a[3]) * k).round() as u32))
        };
        self.replay_live(b, d, (ix + 20.0, iy + 318.0), lap_now.zip(Some(best_ms)), kmh);
        // the four stats at 86
        let stat = |b: &mut Batch, x: f32, title: &str, value: &str, small: &str| {
            let (sx, sy) = at(x, 86.0);
            cap(b, title, sx, sy);
            let vy = sy + d.d(line_box(HudFont::GlassMed, 11.0) + 2.0);
            let w = txt(b, d, HudFont::Glass, 26.0, 0.5, white, value, sx, vy, Some(30.0));
            if !small.is_empty() {
                txt(b, d, HudFont::GlassMed, 13.0, 1.0, dim, small, sx + w + d.d(4.0), vy, Some(30.0));
            }
        };
        let quit = stats.and_then(|s| s.quit_lap);
        match (quit, stats.and_then(|s| s.race_ms())) {
            (Some(l), _) => stat(b, 262.0, "STOPPED AT", &format!("LAP {l}"), &format!("OF {}", h.laps)),
            (None, Some(ms)) => stat(b, 262.0, "RACE TIME", &race_text(ms), ""),
            _ => stat(b, 262.0, "RACE TIME", &race_text((m.seconds() * 1000.0) as u32), "FILE"),
        }
        match stats.and_then(|s| s.best) {
            Some((no, ms)) => stat(b, 420.0, "BEST LAP", &lap_text(ms), &format!("L{no}")),
            None => stat(b, 420.0, "BEST LAP", "--", ""),
        }
        match stats {
            Some(s) if s.top_kmh > 0 => stat(b, 548.0, "TOP SPEED", &s.top_kmh.to_string(), "KM/H"),
            _ => stat(b, 548.0, "TOP SPEED", "--", ""),
        }
        let (fx, fy) = at(650.0, 86.0);
        let vy = fy + d.d(line_box(HudFont::GlassMed, 11.0) + 2.0);
        let sub_y = vy + d.d(30.0 + 3.0);
        let sub = |b: &mut Batch, s: &str, col: u32| {
            txt(b, d, HudFont::Glass, 12.0, 1.0, hex(col, 1.0), s, fx, sub_y, None);
        };
        let me_info = stats.and_then(|s| h.cars.get(s.cars[s.me].slot)).or_else(|| h.cars.get(h.player as usize));
        if mode == 0 {
            cap(b, "YOUR BEST LAP", fx, fy);
            match (info.your_best, stats.and_then(|s| s.best)) {
                (Some(yours), best) => {
                    let w = txt(b, d, HudFont::Glass, 26.0, 0.5, white, &lap_text(yours), fx, vy, Some(30.0));
                    let _ = w;
                    match best.map(|(_, g)| g as i64 - yours as i64) {
                        Some(0) => sub(b, "LEVEL WITH YOUR BEST", 0xbfe3f3),
                        Some(dl) if dl > 0 => sub(b, &format!("YOU ARE {} FASTER", lap_text(dl as u32)), GAIN),
                        Some(dl) => sub(b, &format!("THE GHOST IS {} FASTER", lap_text((-dl) as u32)), LOSS),
                        None => {}
                    }
                }
                (None, _) => {
                    Self::dash_col(b, d, fx + d.d(18.0), vy + d.d(15.0), 18.0, white);
                    sub(b, "NO LAP OF YOURS HERE YET", G_DIM);
                }
            }
        } else {
            let solo = stats.is_none_or(|s| s.cars.len() <= 1);
            cap(b, if quit.is_some() { "ORDER AT THE QUIT" } else { "FINISH" }, fx, fy);
            if solo {
                txt(b, d, HudFont::Glass, 26.0, 0.5, white, "SOLO", fx, vy, Some(30.0));
            } else if let Some(s) = stats {
                let place = s.order.iter().position(|&i| i == s.me).map_or(0, |p| p + 1);
                let w = txt(b, d, HudFont::Glass, 26.0, 0.5, white, &format!("P{place}"), fx, vy, Some(30.0));
                txt(b, d, HudFont::GlassMed, 13.0, 1.0, dim, &format!("/{}", s.cars.len()), fx + w + d.d(4.0), vy, Some(30.0));
            }
            if let Some(c) = me_info {
                sub(b, &format!("{} \u{b7} {}", c.name.to_uppercase(), c.car.to_uppercase()), G_DIM);
            }
        }
        // the middle: the speed trace of a solo run, or the field
        let many = stats.is_some_and(|s| s.cars.len() > 1);
        let (mx, my) = at(262.0, 158.0);
        let mid_right = right_edge;
        if let (Some(s), false) = (stats, many) {
            let (lo, hi) = trace.iter().fold((f32::MAX, 0.0f32), |(lo, hi), p| (lo.min(p[3]), hi.max(p[3])));
            let (no, _) = s.best.unwrap_or((0, 0));
            if trace.is_empty() {
                cap(b, "NO LAP COMPLETED", mx, my);
            } else {
                cap(b, &format!("SPEED \u{b7} BEST LAP (L{no})"), mx, my);
                txt_r(b, d, HudFont::GlassMed, 11.0, 1.0, note, &format!("{} - {} KM/H", lo.round() as u32, hi.round() as u32), mid_right, my, None);
                let (sx, sy) = at(262.0, 176.0);
                Self::replay_spark(b, d, trace, (sx, sy), (554.0, 56.0), phase);
            }
        } else if let Some(s) = stats {
            let title = if mode == 0 { "THE FIELD \u{b7} EVERY CAR RUNS AS A GHOST".to_string() } else if let Some(l) = quit { format!("RUNNING ORDER \u{b7} LAP {l}") } else { "THE FINISH".to_string() };
            cap(b, &title, mx, my);
            let right = match (quit, s.order.first().and_then(|&i| s.cars[i].finish_ms)) {
                (Some(_), _) => "QUIT BEFORE THE FLAG".to_string(),
                (None, Some(ms)) => format!("WINNER {}", race_text(ms)),
                _ => String::new(),
            };
            txt_r(b, d, HudFont::GlassMed, 11.0, 1.0, note, &right, mid_right, my, None);
            self.replay_field(b, d, m, s, (quit.is_some(), info.install), at(268.0, 176.0));
        }
        // the laps, under it
        let solo_mid = !many;
        let bars_top = if solo_mid { 252.0 } else { 274.0 };
        let (bx, by) = at(262.0, bars_top);
        let nick = me_info.map_or(String::new(), |c| c.name.to_uppercase());
        cap(b, &format!("LAPS \u{b7} {nick}"), bx, by);
        if let Some(s) = stats {
            let c = &s.cars[s.me];
            let right = match (quit, s.race_ms()) {
                (Some(_), _) => format!("{} OF {} DRIVEN", c.laps_ms.len(), h.laps),
                (None, Some(ms)) => format!("AVERAGE {}", lap_text(ms / h.laps.max(1))),
                _ => String::new(),
            };
            txt_r(b, d, HudFont::GlassMed, 11.0, 1.0, note, &right, mid_right, by, None);
            let (lx, ly) = at(262.0, bars_top + 16.0);
            Self::lap_bars_of(b, d, &c.laps_ms, h.laps as usize, (lx, ly), (554.0, 356.0 - bars_top - 16.0));
        }
    }

    /// The two mode chips at the panel's top right, a triangle at each end: left / right flip them.
    fn replay_modes(&self, b: &mut Batch, d: Glass, (ix, iy): (f32, f32), mode: usize) {
        let names = [("GHOST RACE", ui_font::icon::GHOST), ("REPLAY RACE", ui_font::icon::EYE)];
        let st = d.font(HudFont::Glass, 15.0, 1.0, WHITE);
        // a chip: border 1, padding 12, icon 16, 7, the text (its trailing tracking included, as CSS has it)
        let widths: Vec<f32> = names.iter().map(|(n, _)| d.d(2.0 + 24.0 + 16.0 + 7.0) + b.font_width(HudFont::Glass, n, &st)).collect();
        let tri = (d.d(9.0), d.d(11.0));
        let gap = d.d(8.0);
        let total = tri.0 * 2.0 + gap * 3.0 + widths.iter().sum::<f32>();
        let mut x = d.x(ix + INNER_W - 20.0) - total;
        let top = d.y(iy + 12.0);
        let cy = top + d.d(14.0);
        let tri_at = |m: &mut Mesh, left: bool, x0: f32, off: bool| {
            let p = |u: f32, v: f32| [x0 + d.d(u), cy - tri.1 * 0.5 + d.d(v)];
            let pts = if left { vec![p(8.0, 1.0), p(1.5, 5.5), p(8.0, 10.0)] } else { vec![p(1.0, 1.0), p(7.5, 5.5), p(1.0, 10.0)] };
            let a = if off { 0.2 } else { 1.0 };
            m.fill(&pts, [1.0, 1.0, 1.0, 0.14 * a], d.px);
            m.stroke(&pts, true, d.w(1.6), Ends::Round, [0.847, 0.949, 1.0, a], d.px);
        };
        let mut m = Mesh::default();
        tri_at(&mut m, true, x, mode == 0);
        b.mesh(&m);
        x += tri.0 + gap;
        for (i, ((name, icon), w)) in names.iter().zip(&widths).enumerate() {
            let on = i == mode;
            let r = vec::rounded_rect(x, top, *w, d.d(28.0), d.d(8.0));
            let mut m = Mesh::default();
            if on {
                m.halo(&r, d.d(12.0), hex(0x50aaff, 0.35));
                m.fill(&r, hex(G_S1, 0.14), d.px);
                m.stroke(&r, true, d.w(1.0), Ends::Round, hex(G_S1, 0.8), d.px);
            } else {
                m.stroke(&r, true, d.w(1.0), Ends::Round, hex(G_LINE, 0.18), d.px);
            }
            b.mesh(&m);
            Self::glass_icon(b, *icon, x + d.d(13.0), top + d.d(6.0), d.d(16.0), hex(if on { G_S1 } else { G_DIM }, 1.0));
            txt(b, d, HudFont::Glass, 15.0, 1.0, hex(if on { 0xffffff } else { G_DIM }, 1.0), name, x + d.d(13.0 + 16.0 + 7.0), top, Some(28.0));
            x += w + gap;
        }
        let mut m = Mesh::default();
        tri_at(&mut m, false, x, mode == 1);
        b.mesh(&m);
    }

    /// The small map disc (`.disc-s`, 220 px) with the track's lap, the best lap's line coloured by speed, the start dot and
    /// the ghost running it (`phase` 0..1, a dot with a tail). `c` = its centre in design px.
    #[allow(clippy::too_many_arguments)]
    fn replay_disc(&self, b: &mut Batch, d: Glass, tr: Option<&TrackInfo>, c: [f32; 2], trace: &[[f32; 4]], phase: f32, locked: bool) {
        let cc = [d.x(c[0]), d.y(c[1])];
        let rb = d.d(110.0);
        let (ri, rf) = (rb - d.d(6.0), rb - d.d(12.0));
        let mut m = Mesh::default();
        Self::glass_bezel_small(&mut m, d, cc, rb);
        m.disc_at(cc, rf, [cc[0], cc[1] - 0.4 * rf], [120.0 / 255.0, 190.0 / 255.0, 1.0, 0.24], [8.0 / 255.0, 20.0 / 255.0, 40.0 / 255.0, 0.9], 96);
        Self::disc_overlays(&mut m, d, cc, rf, 22.0, hex(G_LINE, 0.12), (0.2, 0.34), (0.0, 0.0), (30.0, 0.6));
        Self::gap_ring(&mut m, d, cc, rf, ri);
        b.mesh(&m);
        let Some((tr, p)) = tr.and_then(|t| t.profile.as_ref().filter(|p| !p.outline.is_empty()).map(|p| (t, p))) else { return };
        let fit = Self::fit_lap(tr, p, cc, 0.84 * rf);
        let line = fit.line(p);
        // the unit of the mockup's 200 x 200 map in canvas px: its 157 px square over 200
        let u = d.d(157.0) / 200.0;
        let sw = 3.6 * u;
        let mut m = Mesh::default();
        m.stroke(&line, true, 2.0 * sw, Ends::Round, hex(G_DARK, 0.7), d.px);
        m.stroke(&line, true, sw, Ends::Round, hex(0xbfe3f3, 0.28), d.px);
        let pts: Vec<[f32; 2]> = trace.iter().map(|q| fit.at([q[0], q[1], q[2]])).collect();
        if pts.len() > 2 {
            let (lo, hi) = trace.iter().fold((f32::MAX, 0.0f32), |(lo, hi), q| (lo.min(q[3]), hi.max(q[3])));
            let mut closed = pts.clone();
            closed.push(pts[0]);
            m.glow(&closed, false, sw * 0.62, d.d(4.0), hex(0x4aa8ff, 0.9), 3);
            // the colour in 24 steps, a run of one step one stroke (a stroke per segment would show its joints)
            let step = |i: usize| (((trace[i][3] - lo) / (hi - lo).max(1.0)) * 23.0).round() as usize;
            let (mut from, n) = (0, pts.len());
            for i in 1..=n {
                if i == n || step(i) != step(from) {
                    let run: Vec<[f32; 2]> = (from..=i).map(|k| pts[k % n]).collect();
                    m.stroke(&run, false, sw * 0.62, Ends::Round, speed_col(step(from) as f32 / 23.0), d.px);
                    from = i;
                }
            }
        }
        // the start line: a dark dot with a white rim
        m.fill(&circle(line[0], 1.3 * sw), hex(G_DARK, 1.0), d.px);
        m.stroke(&circle(line[0], 1.3 * sw), true, 0.55 * sw, Ends::Round, hex(0xffffff, 1.0), d.px);
        if pts.len() > 2 {
            // the ghost: ten points of tail and the dot
            let f = phase * pts.len() as f32;
            let (i, k) = (f.floor() as usize % pts.len(), f.fract());
            let (a, z) = (pts[i], pts[(i + 1) % pts.len()]);
            let now = [a[0] + (z[0] - a[0]) * k, a[1] + (z[1] - a[1]) * k];
            let mut tail: Vec<[f32; 2]> = (1..=10).rev().map(|j| pts[(i + pts.len() - j) % pts.len()]).collect();
            tail.push(now);
            m.stroke(&tail, false, 1.1 * sw, Ends::Round, [1.0, 1.0, 1.0, 0.55], d.px);
            m.halo(&circle(now, 1.5 * sw), d.d(7.0), hex(G_S1, 0.6));
            m.halo(&circle(now, 1.5 * sw), d.d(3.0), [1.0, 1.0, 1.0, 0.6]);
            m.fill(&circle(now, 1.5 * sw), [1.0, 1.0, 1.0, 1.0], d.px);
        }
        if locked {
            m.disc(cc, rf, [0.0, 0.0, 0.0, 0.6], [0.0, 0.0, 0.0, 0.6], 64);
        }
        b.mesh(&m);
    }

    /// `.live2`: the ghost's lap time and speed under the disc, spread over its 220 px (`space-around`); `at` = the box's top
    /// left in design px.
    fn replay_live(&self, b: &mut Batch, d: Glass, at: (f32, f32), lap: Option<(u32, u32)>, kmh: Option<u32>) {
        let bb: &Batch = b;
        let cap = |s: &str| (s.to_string(), bb.font_width(HudFont::GlassMed, s, &d.font(HudFont::GlassMed, 11.0, 1.5, WHITE)));
        let big = |s: &str| bb.font_width(HudFont::Glass, s, &d.font(HudFont::Glass, 22.0, 0.5, WHITE));
        let small = |s: &str| bb.font_width(HudFont::GlassMed, s, &d.font(HudFont::GlassMed, 12.0, 0.0, WHITE));
        let (t_main, t_small) = lap.map_or(("--".to_string(), String::new()), |(now, best)| (lap_text(now), format!("/ {}", lap_text(best))));
        let (s_main, s_small) = kmh.map_or(("--".to_string(), String::new()), |k| (k.to_string(), "KM/H".to_string()));
        let w_of = |c: &(String, f32), main: &str, sm: &str| c.1.max(big(main) + if sm.is_empty() { 0.0 } else { small(sm) + d.d(4.0) });
        let (c1, c2) = (cap("GHOST LAP"), cap("SPEED"));
        let (w1, w2) = (w_of(&c1, &t_main, &t_small), w_of(&c2, &s_main, &s_small));
        let free = d.d(220.0) - w1 - w2;
        let x1 = d.x(at.0) + free * 0.25;
        let x2 = x1 + w1 + free * 0.5;
        let top = d.y(at.1);
        let vtop = top + d.d(line_box(HudFont::GlassMed, 11.0));
        let vws = [&t_main, &s_main].map(|m| big(m));
        let sws = [&t_small, &s_small].map(|m| if m.is_empty() { 0.0 } else { small(m) + d.d(4.0) });
        for (n, (x, w, cp, main, sm)) in [(x1, w1, &c1, &t_main, &t_small), (x2, w2, &c2, &s_main, &s_small)].into_iter().enumerate() {
            txt(b, d, HudFont::GlassMed, 11.0, 1.5, hex(G_DIM, 1.0), &cp.0, x + (w - cp.1) * 0.5, top, None);
            let vw = vws[n] + sws[n];
            let vx = x + (w - vw) * 0.5;
            let mw = txt(b, d, HudFont::Glass, 22.0, 0.5, hex(0xffffff, 1.0), main, vx, vtop, Some(26.0));
            if !sm.is_empty() {
                txt(b, d, HudFont::GlassMed, 12.0, 0.0, hex(G_DIM, 1.0), sm, vx + mw + d.d(4.0), vtop, Some(26.0));
            }
        }
    }

    /// `sparkHTML`: the speed over the best lap as an area and a line, with a cursor at the ghost's place. `(x, y)` = canvas.
    fn replay_spark(b: &mut Batch, d: Glass, trace: &[[f32; 4]], (x, y): (f32, f32), (w, h): (f32, f32), phase: f32) {
        let (lo, hi) = trace.iter().fold((f32::MAX, 0.0f32), |(lo, hi), p| (lo.min(p[3]), hi.max(p[3])));
        let yv = |v: f32| y + d.d(h - 4.0 - ((v - lo) / (hi - lo).max(1.0)) * (h - 8.0));
        let n = trace.len();
        let line: Vec<[f32; 2]> = trace.iter().enumerate().map(|(i, p)| [x + d.d(w) * i as f32 / (n - 1) as f32, yv(p[3])]).collect();
        let mut area = vec![[x, y + d.d(h)]];
        area.extend(&line);
        area.push([x + d.d(w), y + d.d(h)]);
        let mut m = Mesh::default();
        m.stroke(&[[x, y + d.d(h - 0.5)], [x + d.d(w), y + d.d(h - 0.5)]], false, d.w(1.0), Ends::Butt, hex(0xbee6ff, 0.18), d.px);
        m.fill(&area, hex(G_S1, 0.18), d.px);
        m.stroke(&line, false, d.w(1.5), Ends::Round, hex(0xbfe3f3, 1.0), d.px);
        let cx = x + d.d(w) * phase;
        m.stroke(&[[cx, y], [cx, y + d.d(h)]], false, d.w(1.0), Ends::Butt, [1.0, 1.0, 1.0, 0.7], d.px);
        b.mesh(&m);
    }

    /// `fieldHTML`: the finish (or the running order of a quit recording) in two columns of four: place, the car's colour bar,
    /// the name (YOU on the recorder's row of your own recordings), the time or gap. `(x, y)` = the box's top left in canvas px.
    fn replay_field(&self, b: &mut Batch, d: Glass, m: &Mde, s: &Stats, (quit, install): (bool, bool), (x, y): (f32, f32)) {
        let rows = s.order.len().min(8);
        let (cols, per) = (2usize, rows.div_ceil(2).max(1));
        let cw = (542.0 - 24.0) / cols as f32;
        let winner = s.order.first().and_then(|&i| s.cars[i].finish_ms);
        for (i, &ci) in s.order.iter().take(8).enumerate() {
            let (col, row) = (i / per, i % per);
            let c = &s.cars[ci];
            let info = &m.header.cars[c.slot];
            let you = ci == s.me && !install;
            let rx = x + d.d((cw + 24.0) * col as f32);
            let ry = y + d.d(22.0 * row as f32);
            if you {
                let r = vec::rounded_rect(rx - d.d(6.0), ry, d.d(cw + 12.0), d.d(21.0), d.d(6.0));
                let mut mm = Mesh::default();
                mm.fill(&r, hex(G_S1, 0.10), d.px);
                mm.stroke(&r, true, d.w(1.0), Ends::Round, hex(G_S1, 0.5), d.px);
                b.mesh(&mm);
            }
            let rgb = info.colours[0];
            let colour = [((rgb >> 16) & 255) as f32 / 255.0, ((rgb >> 8) & 255) as f32 / 255.0, (rgb & 255) as f32 / 255.0, 1.0];
            let place = (i + 1).to_string();
            let pw = b.font_width(HudFont::Glass, &place, &d.font(HudFont::Glass, 13.0, 0.5, WHITE));
            txt(b, d, HudFont::Glass, 13.0, 0.5, hex(G_DIM, 1.0), &place, rx + (d.d(16.0) - pw) * 0.5, ry, Some(21.0));
            let bar = vec::rounded_rect(rx + d.d(24.0), ry + d.d(3.5), d.d(4.0), d.d(14.0), d.d(2.0));
            let mut mm = Mesh::default();
            mm.halo(&bar, d.d(6.0), [colour[0], colour[1], colour[2], 0.7]);
            mm.fill(&bar, colour, d.px);
            b.mesh(&mm);
            let nx = rx + d.d(36.0);
            let white = hex(0xffffff, 1.0);
            let nw = txt(b, d, HudFont::Glass, 14.0, 0.5, if you { white } else { hex(G_TEXT, 1.0) }, &info.name.to_uppercase(), nx, ry, Some(21.0));
            if you {
                txt(b, d, HudFont::Glass, 10.0, 1.5, hex(G_S1, 1.0), "YOU", nx + nw + d.d(4.0), ry, Some(21.0));
            }
            // the time: the winner's whole race, the others' gap; a car still racing when the file ends shows its lap
            let text = match (c.finish_ms, winner) {
                (Some(t), Some(w)) if !quit => {
                    if i == 0 { race_text(t) } else { gap_text(t.saturating_sub(w)) }
                }
                _ => format!("LAP {}", c.lap_at_end),
            };
            txt_r(b, d, HudFont::GlassMed, 13.0, 0.0, if you { white } else { hex(G_DIM, 1.0) }, &text, rx + d.d(cw), ry, Some(21.0));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_race_reads_minutes_and_hours() {
        assert_eq!(race_text(597_232), "9:57.232");
        assert_eq!(race_text(3_723_004), "1:02:03.004");
    }

    #[test]
    fn a_day_count_is_a_date() {
        assert_eq!(ymd(0), "1970-01-01");
        assert_eq!(ymd(20_447), "2025-12-25");
        assert_eq!(ymd(20_759), "2026-11-02");
        assert_eq!(ymd(19_837), "2024-04-24");
    }

    #[test]
    fn the_speed_colour_runs_from_deep_blue_to_ice() {
        assert_eq!(speed_col(0.0), [61.0 / 255.0, 139.0 / 255.0, 1.0, 1.0]);
        assert_eq!(speed_col(2.0), speed_col(1.0));
    }
}
