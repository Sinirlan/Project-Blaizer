//! In-race HUD from the game's Direct3D HUD set (Data/d3d): speed, gear and rev arc on the
//! left, times/lap/position on the right, countdown, wrong way, lap flashes and the minimap
//! (transform from 0x412c0c). The centre texts follow the InfoDetail / CheckpointInfo /
//! NamePlates settings as the D3D HUD 0x44a2a4 tests them, the map markers RoadMap (0x44b1f0).
//! The corner elements follow the HUD Settings toggles (`hud_layout`) in both HUDs, so F9
//! keeps what shows (InfoDetail's level picks which show, Custom all the toggles; bug #167, #181).
//! REMASTER modern HUD (9.21): the HUD Settings also scale and move the corner elements; the top-left
//! cluster, the top-right stack, the map disc, the splits, the centre messages and the name plates are the
//! Race line look (`hud_glass`, #323), with our countdown art and the proximity arrows (`proximity`).

use crate::front::{fmt_time, Game, RaceState};
use crate::front::menu_modern::{glass_time, Glass};
use crate::hud_glass::{self, Cluster, Msg, PlateStyle, Pos, RowGap, SplitRow, Splits, Stack};
use crate::hud_layout::{Extents, HudSettings, MapMode, Placement};
use glam::Vec3;
use motor_sim::race::Gap;
use motorview::minimap::map_point;
use motorview::ui::{Batch, FontStyle, WHITE};
use motorview::ui_font::HudFont;

const LEFT: f32 = 12.0;
const RIGHT_MARGIN: f32 = 12.0;

/// REMASTER countdown (9.21 phase 7; design choice): the art's size (canvas px at 100 %),
/// centred where the original's 78 px count00..03 are (x centre, y 110 + 39); each number
/// pops in from POP_FROM × over POP_TIME s, GO fades out over 1 s.
const COUNT_PX: f32 = 160.0;
const COUNT_Y: f32 = 149.0;
const POP_FROM: f32 = 1.3;
const POP_TIME: f32 = 0.15;

fn countdown(b: &mut Batch, race: &motor_sim::session::Race, cx: f32, scale: f32) {
    // (image, seconds since it appeared, alpha)
    let (n, since, alpha) = if !race.started() {
        // no lamp during the start fly-by (the digits' 2 s windows, 0x45e3c7..0x45e64f)
        let Some((n, since)) = race.count_digit() else { return };
        (n, since, 1.0)
    } else if race.clock_ms < 1000.0 {
        let t = (race.clock_ms.max(0.0) / 1000.0) as f32;
        (0, t, 1.0 - t)
    } else {
        return;
    };
    let e = (since / POP_TIME).clamp(0.0, 1.0);
    let pop = POP_FROM + (1.0 - POP_FROM) * (1.0 - (1.0 - e) * (1.0 - e));
    let size = COUNT_PX * scale * pop;
    b.image_filtered(&format!("cd:{n}"), cx - size * 0.5, COUNT_Y - size * 0.5, size, size, [1.0, 1.0, 1.0, alpha]);
}

/// REMASTER: text with its digits in equal cells (the font's widest digit), so a changing
/// number doesn't shift sideways; returns the advance.
pub(crate) fn digits(b: &mut Batch, font: HudFont, s: &str, x: f32, y: f32, st: &FontStyle) -> f32 {
    let cell = digit_cell(b, font, st);
    let mut cx = x;
    for c in s.chars() {
        let cs = c.to_string();
        let w = b.font_width(font, &cs, st);
        let adv = if c.is_ascii_digit() { cell } else { w };
        b.font_text(font, &cs, cx + (adv - w) * 0.5, y, st);
        cx += adv;
    }
    cx - x
}

pub(crate) fn digits_width(b: &Batch, font: HudFont, s: &str, st: &FontStyle) -> f32 {
    let cell = digit_cell(b, font, st);
    s.chars().map(|c| if c.is_ascii_digit() { cell } else { b.font_width(font, &c.to_string(), st) }).sum()
}

pub(crate) fn digit_cell(b: &Batch, font: HudFont, st: &FontStyle) -> f32 {
    ('0'..='9').map(|c| b.font_width(font, &c.to_string(), st)).fold(0.0, f32::max)
}

/// Right-aligned row of label images (e.g. ["h:best", "h:lap", "h:time"]).
fn label_right(b: &mut Batch, parts: &[&str], right: f32, y: f32) {
    let w: f32 = parts.iter().map(|p| b.size(p).0).sum::<f32>() + 4.0 * (parts.len() as f32 - 1.0);
    let mut x = right - w;
    for p in parts {
        x += b.image(p, x, y) + 4.0;
    }
}

fn label_centre(b: &mut Batch, parts: &[&str], cx: f32, y: f32) {
    let w: f32 = parts.iter().map(|p| b.size(p).0).sum::<f32>() + 4.0 * (parts.len() as f32 - 1.0);
    label_right(b, parts, cx + w * 0.5, y);
}

fn number_right(b: &mut Batch, s: &str, right: f32, y: f32) {
    let w = b.hud_number_width(s, false);
    b.hud_number(s, right - w, y, false);
}

fn number_centre(b: &mut Batch, s: &str, cx: f32, y: f32) {
    let w = b.hud_number_width(s, false);
    b.hud_number(s, cx - w * 0.5, y, false);
}

/// REMASTER close minimap (9.21 phase 6; a design choice, to be tuned in playtests): the
/// ground the round window shows across (m: MIN up to SLOW km/h, widening linearly to MAX at FAST,
/// eased over EASE s).
pub const MAP_ZOOM_MIN: f32 = 250.0;
const MAP_ZOOM_MAX: f32 = 500.0;
const MAP_ZOOM_SLOW: f32 = 100.0;
const MAP_ZOOM_FAST: f32 = 250.0;
const MAP_ZOOM_EASE: f32 = 0.5;

/// REMASTER per frame: ease the close map's ground width towards the one for your speed.
pub fn map_zoom_update(r: &mut RaceState, dt: f32) {
    let kmh = r.sess.race.racers[r.sess.focus].telemetry.kmh.abs();
    let f = ((kmh - MAP_ZOOM_SLOW) / (MAP_ZOOM_FAST - MAP_ZOOM_SLOW)).clamp(0.0, 1.0);
    let target = MAP_ZOOM_MIN + (MAP_ZOOM_MAX - MAP_ZOOM_MIN) * f;
    r.map_zoom += (target - r.map_zoom) * (1.0 - (-dt / MAP_ZOOM_EASE).exp());
}

/// REMASTER: scale the quads drawn since `start` by `k` about `anchor`, then move them by
/// `off` (a corner element placed by `HudSettings::place`).
fn place(b: &mut Batch, start: usize, anchor: (f32, f32), k: f32, off: (f32, f32)) {
    // the default layout stays bit-exact (a + (p - a) need not round to p)
    if k == 1.0 && off == (0.0, 0.0) {
        return;
    }
    for v in &mut b.verts[start..] {
        v.pos = [anchor.0 + (v.pos[0] - anchor.0) * k + off.0, anchor.1 + (v.pos[1] - anchor.1) * k + off.1];
    }
}

/// REMASTER: the visible corner elements' sizes at 100 % (the boxes the code below draws in).
fn extents(h: &HudSettings, map: (f32, f32), split_rows: usize) -> Extents {
    let left = if h.cluster { hud_glass::cluster_size() } else { (0.0, 0.0) };
    let right = hud_glass::stack_size(h.clocks.iter().filter(|x| **x).count(), h.pos, h.lap);
    Extents { left, right, map: if h.map { map } else { (0.0, 0.0) }, splits: hud_glass::splits_size(split_rows), top: 8.0, bottom: 10.0 }
}

/// REMASTER: where the HUD Settings put the corner elements on this batch's canvas.
fn placement(g: &Game, r: &RaceState, b: &Batch) -> Placement {
    let h = &g.settings.hud.shown(g.settings.info_detail);
    // the Race line map disc (#323 phase 3) needs the lap's profile
    let map = if g.tracks[r.track].profile.is_some() { hud_glass::map_size() } else { (0.0, 0.0) };
    // the splits block is sized for the most rows the settings can show, so the HUD does not resize as one appears
    let s = &g.settings;
    let rows = if live_enabled(g, r) {
        if s.checkpoint_info == 1 { LIVE_ROWS } else { r.sess.race.racers.len() }
    } else if s.info_detail >= 1 && s.checkpoint_info > 0 && s.checkpoint_delay > 0 {
        if g.game_type == crate::front::GameType::TimeAttack || s.checkpoint_info == 1 { 3 } else { r.sess.race.racers.len() }
    } else {
        0
    };
    h.place(b.canvas.w, &extents(h, map, rows))
}

/// REMASTER live gaps (#302): the slots of the block at "Checkpoint splits: Nearby" (the car ahead, you, the car behind;
/// All cars lists the field).
const LIVE_ROWS: usize = 3;

/// REMASTER live gaps (#302): the block takes the place of the checkpoint list in this race (the Modern HUD only;
/// the caller checks that). Time Attack has no rivals to measure against and keeps its checkpoint rows.
fn live_enabled(g: &Game, r: &RaceState) -> bool {
    g.settings.hud.live_gaps && g.settings.checkpoint_info > 0 && g.settings.info_detail >= 1 && g.game_type != crate::front::GameType::TimeAttack && r.sess.race.racers.len() > 1
}

/// REMASTER: the scale the corner elements get (the asked one, or less where it won't fit).
pub fn effective_scale(g: &Game, r: &RaceState, b: &Batch) -> f32 {
    placement(g, r, b).scale
}

pub fn draw(g: &Game, r: &RaceState, b: &mut Batch, paused: bool, px: f32) {
    let race = &r.sess.race;
    let me = &race.racers[r.sess.focus];
    let w = b.canvas.w;
    let right = w - RIGHT_MARGIN;
    let t = &me.telemetry;
    let s = &g.settings;
    // InfoDetail: Selective (1) shows speed, lap time and lap; All (2) everything
    let (some, all) = (s.info_detail >= 1, s.info_detail >= 2);
    // the HUD Settings toggles rule the corner elements in both HUDs (InfoDetail's level picks which show)
    let modern = b.is_sharp();
    let h = &s.hud.shown(s.info_detail);
    let cluster_on = h.cluster;
    let row_on = [h.clocks[0], h.clocks[1], h.clocks[2], h.lap, h.pos];
    let tr = &g.tracks[r.track];
    let map_key = format!("map:{}", tr.n);
    let map_size = b.size(&map_key);
    let map_on = map_size.0 > 0.0 && h.map;
    let pl = if modern {
        placement(g, r, b)
    } else {
        Placement { scale: 1.0, dx: 0.0, dy: 0.0 }
    };

    // REMASTER: the glass name plates go first, so every HUD group and message lies over them (#323 phase 5)
    if modern && !paused && s.name_plates > 0 {
        let standings = race.standings();
        let list = plate_points(r, w, b.canvas.h, r.sess.wobble())
            .into_iter()
            .map(|(i, x, y, z)| {
                let rc = &race.racers[i];
                hud_glass::Plate { x, y, z, place: standings.iter().position(|&k| k == i).map_or(0, |k| k as u32 + 1), colour: rc.driver.colour, name: rc.driver.nick.clone() }
            })
            .collect();
        hud_glass::plates(b, Glass::hud(w, px), match (s.name_plates, s.hud.plate_frame) {
            (2, true) => PlateStyle::Glass,
            (2, false) => PlateStyle::Bare,
            _ => PlateStyle::Flat,
        }, list);
    }

    // ---- left: speed, gear, rev arc
    let start = b.verts.len();
    if modern && h.proximity {
        crate::proximity::draw(r, b);
    }
    if cluster_on {
        let kmh = format!("{}", t.kmh.abs().round() as u32);
        let gear = if t.gear == 0 { "R".to_string() } else { t.gear.to_string() };
        let frac = ((t.rpm - me.car.p.min_rpm) / (me.car.p.max_rpm - me.car.p.min_rpm)).clamp(0.0, 1.0);
        if modern {
            // REMASTER: the Race line cluster (#323)
            let c = Cluster { kmh: t.kmh.abs().round() as u32, gear: &gear, frac, clock: g.clock };
            hud_glass::cluster(b, Glass::hud(w, px), &c, &s.view.remaster.rev_meter);
        } else {
            let sw = b.hud_number(&kmh, LEFT, 8.0, true);
            b.image("h:tkmh00", LEFT + sw + 4.0, 31.0);
            b.image("h:tgear00", LEFT - 2.0, 52.0);
            b.hud_number(&gear, LEFT + 40.0, 52.0, false);
            b.image("h:meter01", LEFT + 6.0, 72.0);
            b.image_left("h:meter00", LEFT + 6.0, 72.0, frac);
        }
    }
    if modern {
        place(b, start, hud_glass::CLUSTER_ANCHOR, pl.scale, (pl.dx, pl.dy));
    }

    // ---- right: times, lap, position (labels as in the original: TOTAL TIME,
    // TOTAL LAP TIME, BEST LAP TIME, LAP, POS); the shown rows close up from the top
    let p = &me.progress;
    let now = race.clock_ms.max(0.0) as u32;
    let total = p.finished_ms.unwrap_or(now);
    let lap_time = if p.finished_ms.is_some() { p.last_lap_ms.unwrap_or(0) } else { now.saturating_sub(p.lap_start_ms) };
    let standings = race.standings();
    let pos = standings.iter().position(|&i| i == r.sess.focus).unwrap_or(0) + 1;
    let start = b.verts.len();
    // REMASTER: PB = the race's best lap beats your best ever lap here (the statistics are only folded in at
    // the end of a race, so they hold the record from before it)
    let pb = modern && p.best_lap_ms.is_some_and(|ms| g.stats.tracks.get(&tr.name).and_then(|t| t.best_lap).is_none_or(|(rec, _)| ms < rec));
    if modern {
        // REMASTER: the Race line stack (#323 phase 2)
        let st = Stack {
            clocks: [row_on[0].then_some(Some(total)), row_on[1].then_some(Some(lap_time)), row_on[2].then_some(p.best_lap_ms)],
            pb,
            pos: row_on[4].then(|| {
                if p.eliminated_ms.is_some() {
                    Pos::Out
                } else {
                    Pos::Place(pos as u32, race.racers.iter().filter(|x| x.progress.eliminated_ms.is_none()).count() as u32)
                }
            }),
            lap: row_on[3].then_some((p.lap.min(p.laps), p.laps)),
            final_lap: p.finished_ms.is_none() && p.lap >= p.laps,
        };
        hud_glass::stack(b, Glass::hud(w, px), &st);
        place(b, start, (right, 8.0), pl.scale, (-pl.dx, pl.dy));
    } else {
        let rows: [(bool, &[&str], String); 5] = [
            (row_on[0], &["h:total", "h:time"], fmt_time(total)),
            (row_on[1], &["h:total", "h:lap", "h:time"], fmt_time(lap_time)),
            (row_on[2], &["h:best", "h:lap", "h:time"], fmt_time(p.best_lap_ms.unwrap_or(0))),
            (row_on[3], &["h:lap"], format!("{}/{}", p.lap.min(p.laps), p.laps)),
            (row_on[4], &["h:pos"], if p.eliminated_ms.is_some() { "OUT".into() } else { format!("{}/{}", pos, race.racers.iter().filter(|x| x.progress.eliminated_ms.is_none()).count()) }),
        ];
        let mut y = 8.0;
        for (_, labels, value) in rows.iter().filter(|x| x.0) {
            label_right(b, labels, right, y);
            number_right(b, value, right, y + 18.0);
            y += 42.0;
        }
    }

    if !modern && !paused && s.name_plates > 0 {
        nameplates(r, b, s.name_plates == 2);
    }

    // ---- centre: countdown, wrong way, lap flash / split, finish (hidden while paused)
    let cx = w * 0.5;
    if !paused {
        if modern {
            // REMASTER: our countdown art, scaled with the HUD
            countdown(b, race, cx, pl.scale);
        } else if !race.started() {
            if let Some((n, _)) = race.count_digit() {
                b.image(&format!("h:count0{}", 3 - n), cx - 39.0, 110.0);
            }
        } else if race.clock_ms < 2000.0 {
            // CLASSIC: GO shows for its own 2 s window (0x45e61f..0x45e64f)
            b.image("h:count03", cx - 39.0, 110.0);
        }
        // wrong way: All only, blinking every 200 ms (0x44a2a4)
        if all && p.wrong_way && p.finished_ms.is_none() && ((g.clock * 5.0) as u32).is_multiple_of(2) {
            if modern {
                // REMASTER: red Glass text with a warning triangle (#323 phase 4)
                hud_glass::wrong_way(b, Glass::hud(w, px));
            } else {
                let (ww, _) = b.size("h:tww00");
                b.image("h:tww00", cx - ww * 0.5, 160.0);
            }
        }
        // Knockout (our design): you are out, and watch the leader
        let out = race.racers.iter().any(|x| x.human && x.progress.eliminated_ms.is_some());
        if out && !modern {
            let txt = "ELIMINATED";
            let tw = b.text_width(txt, 1.6);
            b.text(txt, cx - tw * 0.5, 96.0, 1.6, crate::front::ORANGE);
        }
        // Ghost mode / Record race (0x45c7c4): the ghost icon while the recorded cars play, the R icon
        // while recording, and the console messages. APPROX: the original's positions [U]
        if modern {
            // REMASTER (#345): the REC / GHOST / DEMO tags top centre (the notice joins the centre messages below)
            let modes: Vec<hud_glass::Mode> = [(r.recording_demo(), hud_glass::Mode::Rec), (r.ghost_playing() && r.watch.is_none(), hud_glass::Mode::Ghost), (r.watch.is_some(), hud_glass::Mode::Demo)]
                .into_iter()
                .filter_map(|(on, m)| on.then_some(m))
                .collect();
            hud_glass::mode_tags(b, Glass::hud(w, px), g.clock, &modes);
        } else {
            // CLASSIC
            let mut ix = 8.0;
            for (on, art) in [(r.watch.is_some(), "h:cam00"), (r.ghost_playing() && r.watch.is_none(), "h:ghost00"), (r.recording_demo(), "h:r00")] {
                if on {
                    let (iw, _) = b.size(art);
                    b.image(art, ix, 92.0);
                    ix += iw + 4.0;
                }
            }
            if let Some((_, msg)) = &r.notice {
                let tw = b.text_width(msg, 0.9);
                b.text(msg, cx - tw * 0.5, 24.0, 0.9, motorview::ui::WHITE);
            }
        }
        // lap and checkpoint info need InfoDetail, CheckpointInfo and CheckpointDelay
        let info = some && s.checkpoint_info > 0 && s.checkpoint_delay > 0;
        let time_attack = g.game_type == crate::front::GameType::TimeAttack;
        // REMASTER: the centre messages stack, the checkpoint block sits bottom left (#323 phase 4)
        let mut msgs: Vec<Msg> = Vec::new();
        let mut splits: Option<Splits> = None;
        // REMASTER (#345): the 4 s notice of Record race / Ghost mode / Demo play leads the stack, in capitals without the "!"
        if let (true, Some((_, msg))) = (modern, &r.notice) {
            msgs.push(Msg::Notice(msg.trim_end_matches('!').to_uppercase()));
        }
        // REMASTER (#302): the live gaps replace the checkpoint list (the chime still marks the checkpoint)
        let live_cfg = modern && live_enabled(g, r);
        let live_on = live_cfg && race.started() && p.finished_ms.is_none() && p.eliminated_ms.is_none();
        if modern && out {
            msgs.push(Msg::Eliminated);
        }
        // the FINAL LAP notice: the first 2 s of the last lap of a race of more than one lap
        if modern && some && p.finished_ms.is_none() && p.laps > 1 && p.lap >= p.laps && race.started() && now.saturating_sub(p.lap_start_ms) < 2000 {
            msgs.push(Msg::FinalLap);
        }
        if !some {
            // InfoDetail None: only the countdown
        } else if let Some(f) = p.finished_ms {
            let last = p.last_lap_ms.unwrap_or(0);
            if modern {
                msgs.push(Msg::Finished { pos: pos as u32, of: race.racers.len() as u32, last, total: f });
            } else {
                // CLASSIC: LAST LAP TIME over RACE TIME from the finish until the results, the
                // second pair 33 px below the first (gameplay videos) [O]
                label_centre(b, &["h:last", "h:lap", "h:time"], cx, 8.0);
                number_centre(b, &fmt_time(last), cx, 26.0);
                label_centre(b, &["h:race", "h:time"], cx, 41.0);
                number_centre(b, &fmt_time(f), cx, 59.0);
            }
        } else if let Some((_, ms, best, delta)) = r.lap_flash.filter(|_| if modern { some && s.hud.lap_delay > 0 } else { info }) {
            if modern {
                msgs.push(Msg::Lap { best, ms, pb: best && pb, delta: delta.filter(|_| s.hud.sector_delay > 0) });
                if time_attack && info {
                    let done = p.finished_ms.map_or(p.lap - 1, |_| p.laps);
                    splits = Some(Splits { caption: format!("LAP {done} · {}", glass_time(ms)), rows: ta_rows(r), live: false });
                }
            } else {
                label_centre(b, &[if best { "h:best" } else { "h:last" }, "h:lap", "h:time"], cx, 8.0);
                number_centre(b, &fmt_time(ms), cx, 26.0);
                if time_attack {
                    ta_gaps(r, b, cx, 48.0);
                }
            }
        } else if let Some((_, idx, lap_ms)) = r.split.filter(|_| info && !live_cfg) {
            // checkpoint: the split at the top and every car's gap at this point, bottom left
            if modern {
                // your split time is on your row (Time Attack: the caption)
                // your time sits on your own row; Time Attack has no such row, so its caption keeps the time
                splits = Some(if time_attack {
                    Splits { caption: format!("CHECKPOINT {} · {}", idx % 4, glass_time(lap_ms)), rows: ta_rows(r), live: false }
                } else {
                    Splits { caption: format!("CHECKPOINT {}", idx % 4), rows: split_block(r, idx, s.checkpoint_info == 1, lap_ms), live: false }
                });
            } else {
                number_centre(b, &fmt_time(lap_ms), cx, 12.0);
                if time_attack {
                    ta_gaps(r, b, cx, 34.0);
                } else {
                    split_list(r, idx, b, s.checkpoint_info == 1);
                }
            }
        }
        // REMASTER (#409, design choice [I]): the delta of the time so far this lap against the best earlier lap at this
        // checkpoint (no time shown), for "Sector delta" seconds, whatever the splits block shows
        if let (true, true, None, Some((_, n, delta))) = (modern, some, p.finished_ms, r.sector_flash) {
            msgs.push(Msg::Sector { n, delta });
        }
        if live_on && splits.is_none() {
            splits = Some(live_block(r, pos, s.checkpoint_info == 2));
        }
        if modern {
            hud_glass::messages(b, Glass::hud(w, px), &msgs);
            // REMASTER (#346): the playback controls of demo play, low in the centre column
            if let (true, Some(pb)) = (r.watch.is_some(), r.pb.as_ref()) {
                let cam = crate::playback::cam_label(r.sess.cam);
                let hints = if pb.shown() { g.playback_hints(pb.ended) } else { Vec::new() };
                crate::hud_transport::transport(b, Glass::hud(w, px), &pb.view(motor_sim::demo::race_ms(race)), cam, &hints, g.clock);
            }
            if let Some(sp) = splits {
                let start = b.verts.len();
                hud_glass::splits(b, Glass::hud(w, px), &sp);
                place(b, start, hud_glass::SPLITS_ANCHOR, pl.scale, (pl.dx, -pl.dy));
            }
        }
    }

    // ---- minimap, bottom right (10 px in from the corner, 0x412c0c): a marker in each
    // driver's colour with their race position above it
    let mine = pos;
    // RoadMap Selective: you and the places directly ahead and behind (0x44b1f0); Knockout: the eliminated are out
    let shown = |i: usize| -> Option<usize> {
        let place = standings.iter().position(|&s| s == i).unwrap_or(0) + 1;
        (race.racers[i].progress.eliminated_ms.is_none() && !(s.road_map == 1 && place.abs_diff(mine) > 1)).then_some(place)
    };
    if modern {
        // REMASTER: the Race line map disc (#323 phase 3), the lap drawn from the centreline
        if let (true, Some(prof)) = (h.map, &tr.profile) {
            let start = b.verts.len();
            let mut dots: Vec<hud_glass::MapDot> = (0..race.racers.len())
                .filter_map(|i| {
                    let place = shown(i)?;
                    let x = &race.racers[i];
                    Some(hud_glass::MapDot { pos: x.car.pos, colour: x.driver.colour, place: (i != r.sess.focus && h.map_numbers).then_some(place as u32), me: i == r.sess.focus, ghost: false, ninja: x.driver.ninja_marker })
                })
                .collect();
            // Time Attack ghost and Ghost mode's recorded cars (design choice): pale, translucent dots without a number
            dots.extend(r.ghost_poses().iter().map(|gp| hud_glass::MapDot { pos: gp.pos, colour: [191, 230, 255], place: None, me: false, ghost: true, ninja: false }));
            let frame = if h.map_mode == MapMode::Close {
                // close map: centred on you, map_zoom metres across (by speed), north up or turned so your car's
                // nose points up (design choice: the nose, not the travel direction, so a spin turns the map)
                let p = me.car.pos;
                let (o, f) = (map_point(tr.map_info, p), map_point(tr.map_info, p + me.car.rot * Vec3::Z));
                let turn = if h.map_rotate { -std::f32::consts::FRAC_PI_2 - (f.1 - o.1).atan2(f.0 - o.0) } else { 0.0 };
                hud_glass::MapFrame::Close { at: p, metres: r.map_zoom, turn }
            } else {
                hud_glass::MapFrame::Circuit
            };
            hud_glass::map(b, Glass::hud(w, px), tr.map_info, prof, frame, &dots);
            place(b, start, (w - hud_glass::MAP_INSET.0, 480.0 - hud_glass::MAP_INSET.1), pl.scale, (-pl.dx, -pl.dy));
        }
    } else if map_on {
        let (mw, mh) = map_size;
        let (mx, my) = (w - mw - 10.0, 480.0 - mh - 10.0);
        let art = |p: Vec3| {
            let (x, y) = map_point(tr.map_info, p);
            (x + tr.map_offset.0, y + tr.map_offset.1)
        };
        b.image(&map_key, mx, my);
        let mut order: Vec<usize> = (0..race.racers.len()).collect();
        order.sort_by_key(|&i| i == r.sess.focus); // draw yourself last, on top
        for i in order {
            let Some(place) = shown(i) else { continue };
            let (ax, ay) = art(race.racers[i].car.pos);
            let (px, py) = (mx + ax, my + ay);
            let c = colour(race.racers[i].driver.colour);
            // CLASSIC 0x412aa4: sprite 0x16 (avenger.tga) for the cheat's racer, else 0x1e plupp for you (the flag
            // [esp+0x5c] is 1 at 0x412d23 for the focus racer) and 0x29 pluppother for the others (0 at 0x412f75);
            // the number sits over whichever it is
            let dot = if race.racers[i].driver.ninja_marker && b.assets.atlas.get("h:avenger").is_some() {
                "h:avenger"
            } else if i != r.sess.focus && b.assets.atlas.get("h:pluppother").is_some() {
                "h:pluppother"
            } else {
                "h:plupp"
            };
            let (pw, ph) = b.size(dot);
            let (x, y) = (px - pw * 0.5, py - ph);
            b.image_tint(dot, x, y, c);
            if h.map_numbers {
                let nr = format!("h:mapnr{place}");
                let (nw, nh) = b.size(&nr);
                b.image(&nr, x + (pw - nw) * 0.5, y - nh - 1.0);
            }
        }
        // Time Attack ghost and Ghost mode's recorded cars (design choice): pale, translucent markers without a number
        for gp in r.ghost_poses() {
            let (ax, ay) = art(gp.pos);
            let (pw, ph) = b.size("h:plupp");
            b.image_tint("h:plupp", mx + ax - pw * 0.5, my + ay - ph, [0.75, 0.9, 1.0, 0.55]);
        }
    }

    // ---- CHEAT tribute to tron (#320, 0x48e964): the scroller's band and greeting over everything else
    if !paused {
        if let Some(sc) = motor_sim::cheats::tron(race.name_cheat).then(|| motor_sim::cheats::tron_scroll(race.since_init_s())).flatten() {
            tron_scroller(b, sc, modern, px);
        }
    }
}

/// CHEAT tribute to tron (#320, `0x48f810..0x48fc83`): the black band at the bottom (a soft top edge from 0.88 H to
/// 0.90 H, solid below) and the greeting's five lines rising through it. The layout is the original's 512 x 384 one in
/// fractions of the height, on the 480-tall HUD canvas: a 4:3 window matches it, a wider one keeps the left inset.
fn tron_scroller(b: &mut Batch, sc: motor_sim::cheats::TronScroll, modern: bool, px: f32) {
    use motor_sim::cheats::*;
    let (w, h) = (b.canvas.w, b.canvas.h);
    let (y0, y1) = (TRON_BAND_Y0 * h, TRON_BAND_Y1 * h);
    // the extents also cover a letterboxed canvas edge to edge
    b.rect_fade(0.0, y0, w, y1 - y0, [0.0, 0.0, 0.0, 0.0], [0.0, 0.0, 0.0, sc.alpha]);
    b.rect(0.0, y1, w, h - y1, [0.0, 0.0, 0.0, sc.alpha]);
    if !sc.text {
        return;
    }
    let k = h / TRON_REF_H;
    for i in 0..TRON_LINES {
        let Some(line) = TRON_TEXT.get(sc.first + i).filter(|l| !l.is_empty()) else { continue };
        let y = (TRON_TEXT_Y0 + TRON_TEXT_PITCH * (i as f32 + sc.f)) * h;
        if modern {
            hud_glass::tron_line(b, Glass::hud(w, px), line, y, y0, y1);
        } else {
            // CLASSIC: white myfont, cut off where the band starts (the original's depth test)
            b.text_from(line, TRON_TEXT_X * k, y, k, motorview::ui::WHITE, y0);
        }
    }
}

fn colour(c: [u8; 3]) -> [f32; 4] {
    [c[0] as f32 / 255.0, c[1] as f32 / 255.0, c[2] as f32 / 255.0, 1.0]
}

/// Signed gap as -m:ss:cc / +m:ss:cc (the split list of the original).
fn fmt_gap(ms: i64) -> String {
    let a = ms.unsigned_abs();
    let sign = match ms {
        0 => ' ',
        x if x < 0 => '-',
        _ => '+',
    };
    format!("{sign}{}:{:02}:{:02}", a / 60000, (a / 1000) % 60, (a % 1000) / 10)
}

/// Time Attack (`0x45d23b`): "Lap" in white against this race's best lap, "Lap Ever" and
/// "Race Ever" in 0xaaaa00 against the track records (strings at file offset 0xd395a); rows
/// without a reference are left out. Placement and font are ours [I].
fn ta_gaps(r: &RaceState, b: &mut Batch, cx: f32, y: f32) {
    let rows = [("Lap", WHITE), ("Lap Ever", [0.667, 0.667, 0.0, 1.0]), ("Race Ever", [0.667, 0.667, 0.0, 1.0])];
    let mut y = y;
    for ((label, c), gap) in rows.iter().zip(r.ta_gaps) {
        let Some(gap) = gap else { continue };
        let txt = format!("{label} {}", fmt_gap(gap));
        let tw = b.text_width(&txt, 1.0);
        b.text(&txt, cx - tw * 0.5, y, 1.0, *c);
        y += 13.0;
    }
}

/// Everyone through checkpoint `idx` (Progress::splits), ordered, against your time: (place at the
/// checkpoint, gap to you in ms, racer). CheckpointInfo Selective lists 3 rows instead of 8
/// (0x413f88); we take the ones around yours [I].
fn split_rows(r: &RaceState, idx: usize, selective: bool) -> Vec<(u32, i64, usize)> {
    let race = &r.sess.race;
    let Some(mine) = race.racers[r.sess.focus].progress.splits.get(idx).copied().flatten() else { return Vec::new() };
    let mut rows: Vec<(i64, usize)> = race
        .racers
        .iter()
        .enumerate()
        .filter_map(|(i, rc)| rc.progress.splits.get(idx).copied().flatten().map(|t| (t as i64 - mine as i64, i)))
        .collect();
    rows.sort();
    let mut rows: Vec<(u32, i64, usize)> = rows.into_iter().enumerate().map(|(k, (gap, i))| (k as u32 + 1, gap, i)).collect();
    if selective {
        let me = rows.iter().position(|x| x.2 == r.sess.focus).unwrap_or(0);
        let from = me.saturating_sub(1).min(rows.len().saturating_sub(3));
        rows = rows.into_iter().skip(from).take(3).collect();
    }
    rows
}

fn split_list(r: &RaceState, idx: usize, b: &mut Batch, selective: bool) {
    let race = &r.sess.race;
    let rows = split_rows(r, idx, selective);
    let y0 = 480.0 - 10.0 - rows.len() as f32 * 13.0;
    for (k, (_, gap, i)) in rows.iter().enumerate() {
        let c = colour(race.racers[*i].driver.colour);
        let y = y0 + k as f32 * 13.0;
        b.text(&fmt_gap(*gap), 10.0, y, 1.0, c);
        b.text(&race.racers[*i].driver.nick, 104.0, y, 1.0, c);
    }
}

/// REMASTER: the checkpoint rows of the Race line splits block.
fn split_block(r: &RaceState, idx: usize, selective: bool, lap_ms: u32) -> Vec<Option<SplitRow>> {
    let race = &r.sess.race;
    split_rows(r, idx, selective)
        .into_iter()
        .map(|(place, gap, i)| Some(SplitRow { place: Some(place), colour: race.racers[i].driver.colour, name: race.racers[i].driver.nick.clone(), gap: (i != r.sess.focus).then_some(RowGap::Ms(gap)), time: (i == r.sess.focus).then_some(lap_ms), you: i == r.sess.focus }))
        .collect()
}

/// REMASTER live gaps (#302): the car before and the car after `focus` in `standings`; a knocked-out car behind
/// counts as none (cars ahead of a running car are never out).
fn neighbours(standings: &[usize], focus: usize, out: impl Fn(usize) -> bool) -> (Option<usize>, Option<usize>) {
    let Some(k) = standings.iter().position(|&i| i == focus) else { return (None, None) };
    (k.checked_sub(1).map(|j| standings[j]), standings.get(k + 1).copied().filter(|&i| !out(i)))
}

/// REMASTER live gaps (#302): the splits block, `place` = yours. "Checkpoint splits: Nearby" lists the car ahead, you
/// and the car behind; `all` ("All cars") lists the field in race order. The gap to a car ahead reads negative (red),
/// to one behind positive (green); `None` shows dashes. In the three-slot block a slot with no car (you lead, or run
/// last) stays empty so your row does not jump when you overtake.
fn live_block(r: &RaceState, place: usize, all: bool) -> Splits {
    let race = &r.sess.race;
    let focus = r.sess.focus;
    let standings = race.standings();
    let row = |i: usize, place: usize, gap: Option<RowGap>| {
        let d = &race.racers[i].driver;
        Some(SplitRow { place: Some(place as u32), colour: d.colour, name: d.nick.clone(), gap, time: None, you: i == focus })
    };
    let signed = |g: Option<Gap>, k: i64| match g {
        Some(Gap::Time(ms)) => RowGap::Ms(k * i64::from(ms)),
        Some(Gap::Laps(n)) => RowGap::Laps((k * i64::from(n)) as i32),
        None => RowGap::Dashes,
    };
    let ahead_of_you = |i: usize, place: usize| row(i, place, Some(signed(race.live_gap(i, focus), -1)));
    let behind_you = |i: usize, place: usize| row(i, place, Some(signed(race.live_gap(focus, i), 1)));
    let rows = if all {
        let me = place - 1;
        standings.iter().enumerate().map(|(k, &i)| if k < me { ahead_of_you(i, k + 1) } else if k == me { row(i, k + 1, None) } else { behind_you(i, k + 1) }).collect()
    } else {
        let (ahead, behind) = neighbours(&standings, focus, |i| race.racers[i].progress.eliminated_ms.is_some());
        vec![ahead.and_then(|i| ahead_of_you(i, place - 1)), row(focus, place, None), behind.and_then(|i| behind_you(i, place + 1))]
    };
    Splits { caption: "LIVE GAP".to_string(), live: true, rows }
}

/// REMASTER: Time Attack's rows of the splits block (the references `ta_gaps` has; same colours as the Classic text).
fn ta_rows(r: &RaceState) -> Vec<Option<SplitRow>> {
    [("LAP", [255, 255, 255]), ("LAP EVER", [170, 170, 0]), ("RACE EVER", [170, 170, 0])]
        .into_iter()
        .zip(r.ta_gaps)
        .filter_map(|((name, colour), gap)| Some(Some(SplitRow { place: None, colour, name: name.to_string(), gap: Some(RowGap::Ms(gap?)), time: None, you: false })))
        .collect()
}

/// The rivals' plate points on the canvas: (racer, x, y, distance along the view) of every other running car
/// 4..140 m ahead of the camera, the point 1.8 m over the car (CHEAT gulliver: scaled with the car).
/// `wobble` = the phase of CHEAT ramlosa / h2o's picture warp (#355): the points go through it, so the plates follow the cars.
fn plate_points(r: &RaceState, w: f32, h: f32, wobble: Option<f32>) -> Vec<(usize, f32, f32, f32)> {
    let cam = &r.cam;
    let (f, rt) = (cam.forward(), cam.right());
    let up = rt.cross(f);
    // 90 degree horizontal field of view: focal length = half the width
    let focal = w * 0.5;
    let mut out = Vec::new();
    for (i, rc) in r.sess.race.racers.iter().enumerate() {
        if i == r.sess.focus || rc.progress.eliminated_ms.is_some() {
            continue;
        }
        // #374: over the car where it is drawn
        let at = r.sess.race.view_pose(i).pos;
        let p = glam::Vec3::new(at.x, at.y + 1.8 * rc.car.p.scale, -at.z);
        let d = p - cam.pos;
        let z = d.dot(f);
        if !(4.0..140.0).contains(&z) {
            continue;
        }
        let (x, y) = (w * 0.5 + d.dot(rt) / z * focal, h * 0.5 - d.dot(up) / z * focal);
        let (x, y) = wobble.map_or((x, y), |phi| wobble_point(x, y, w, h, phi));
        out.push((i, x, y, z));
    }
    out
}

/// CHEAT ramlosa / h2o (#355): where the picture warp (`underwater.wgsl`, `Gpu::draw_underwater`) puts the canvas point
/// (x, y): zoom around the centre plus the sideways ripple of the point's own row. The canvas spans the whole 3D view, and
/// the amplitude scales with its height as in the pass (10 px at 480 lines).
fn wobble_point(x: f32, y: f32, w: f32, h: f32, phi: f32) -> (f32, f32) {
    use motor_sim::cheats::{WOBBLE_AMP_PX, WOBBLE_BREATH, WOBBLE_WAVES, WOBBLE_ZOOM};
    let s = WOBBLE_ZOOM + WOBBLE_BREATH * phi.cos();
    let ripple = WOBBLE_AMP_PX * h / 480.0 * ((y - h) * 2.0 * WOBBLE_WAVES / h + phi).sin();
    (w * 0.5 + (x - w * 0.5) * s + ripple, h * 0.5 + (y - h * 0.5) * s)
}

/// Opponents' nicks floating over their cars, in their colour; NamePlates Transparent draws
/// them see-through [I: the alpha is ours]. (The Remaster HUD draws `hud_glass::plates` instead.)
fn nameplates(r: &RaceState, b: &mut Batch, transparent: bool) {
    let (w, h) = (b.canvas.w, b.canvas.h);
    for (i, sx, sy, _) in plate_points(r, w, h, r.sess.wobble()) {
        let nick = &r.sess.race.racers[i].driver.nick;
        let tw = b.text_width(nick, 0.75);
        if sx < -tw || sx > w + tw || !(0.0..h).contains(&sy) {
            continue;
        }
        let mut c = colour(r.sess.race.racers[i].driver.colour);
        if transparent {
            c[3] = 0.55;
        }
        b.text(nick, sx - tw * 0.5, sy - 10.0, 0.75, c);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The shader's mapping (`underwater.wgsl` `fs_warp`): from a warped pixel back to the picture it samples.
    fn shader_inverse(x: f32, y: f32, w: f32, h: f32, phi: f32) -> (f32, f32) {
        use motor_sim::cheats::{WOBBLE_AMP_PX, WOBBLE_BREATH, WOBBLE_WAVES, WOBBLE_ZOOM};
        let s = WOBBLE_ZOOM + WOBBLE_BREATH * phi.cos();
        let sy = h * 0.5 + (y - h * 0.5) / s;
        let ripple = WOBBLE_AMP_PX * h / 480.0 * ((sy - h) * 2.0 * WOBBLE_WAVES / h + phi).sin();
        (w * 0.5 + (x - w * 0.5 - ripple) / s, sy)
    }

    #[test]
    fn wobble_point_is_the_inverse_of_the_shader_sampling() {
        for (w, h) in [(640.0, 480.0), (853.0, 480.0)] {
            for phi in [0.0, 0.7, 2.0, std::f32::consts::PI, 5.5] {
                for (x, y) in [(0.0, 0.0), (w * 0.5, h * 0.5), (100.0, 400.0), (w - 20.0, 60.0), (w * 0.3, h * 0.8)] {
                    let (wx, wy) = wobble_point(x, y, w, h, phi);
                    let (bx, by) = shader_inverse(wx, wy, w, h, phi);
                    assert!((bx - x).abs() < 1e-2 && (by - y).abs() < 1e-3, "({x},{y}) phi {phi}: back at ({bx},{by})");
                }
            }
        }
        // the zoom keeps the centre row where it is and spreads the others away from it
        assert!((wobble_point(320.0, 240.0, 640.0, 480.0, 1.0).1 - 240.0).abs() < 1e-3);
        assert!(wobble_point(320.0, 100.0, 640.0, 480.0, 1.0).1 < 100.0);
    }

    #[test]
    fn neighbours_are_the_places_either_side() {
        let order = [3, 0, 5, 1];
        let none = |_: usize| false;
        // the leader has nobody ahead, last place nobody behind, the others both
        assert_eq!(neighbours(&order, 3, none), (None, Some(0)));
        assert_eq!(neighbours(&order, 5, none), (Some(0), Some(1)));
        assert_eq!(neighbours(&order, 1, none), (Some(5), None));
        // a knocked-out car behind is none; one that is not in the order gives nothing
        assert_eq!(neighbours(&order, 5, |i| i == 1), (Some(0), None));
        assert_eq!(neighbours(&order, 9, none), (None, None));
        assert_eq!(neighbours(&[2], 2, none), (None, None));
    }
}
