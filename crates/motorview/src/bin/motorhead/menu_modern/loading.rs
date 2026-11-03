//! REMASTER Loading screen (#90, mockup B "Circuit" of reference_materials/loading-overhaul): the event card, the
//! track's name block (location, length, laps, climb, the best lap and race at this lap count), the load stage with
//! its percentage and the stage bar, ESC CANCEL; READY holds the full bar for `READY_BEAT` before the race. Behind
//! them the lap in 3D on a turning turntable (`loading_turntable`, the mockup's `draw3D` canvas at 568, 104): heights
//! x 5 above the lap's lowest point, painter-sorted 2D mesh (no depth buffer), lit from the start line as the load runs.
//!
//! Deviations from the mockup: the bar's stages are the game's own, in the order the load runs, with their
//! measured weights (`Stage::span`), so the labels of the two short stages (TRACK DATA, AUDIO) are pushed right
//! of their segment to stay readable; best lap / best race are the `.CL` / `.CR` records, the original's loading
//! screen fields (0x427dc0), not the statistics. APPROX: the canvas `shadowBlur` glows of the lit road and the
//! head dot are wider low-alpha strokes and halos (the GPU glow pass would break the painter order).

use super::personal::rgb_hex;
use super::results::{tag, txt, txt_r};
use super::*;
use motor_sim::race::TrackProfile;
use motorview::loading::Stage;

/// The event card and the progress rule (design px).
const CARD: (f32, f32, f32, f32) = (62.0, 228.0, 300.0, 160.0);
const BAR: (f32, f32) = (62.0, 1218.0);
const BAR_TOP: f32 = 654.0;

/// The turntable (`draw3D`): its canvas (design px), the height exaggeration (one value for every track, so climbs
/// compare), the camera pitch (rad), one turn in `TURN_S` seconds from yaw `TURN_YAW0`, and the lap's points.
const VIEW: (f32, f32, f32, f32) = (568.0, 104.0, 680.0, 480.0);
const EX: f32 = 5.0;
const PITCH: f32 = 0.36;
const TURN_S: f32 = 40.0;
const TURN_YAW0: f32 = 0.6;
const LAP_POINTS: usize = 128;
/// the unlit lap's ice, `rgba(190,230,255,a)`
const ICE: u32 = 0xbee6ff;

/// The lowest and highest point of the lap's outline (game y).
fn lap_span(p: &TrackProfile) -> (f32, f32) {
    p.outline.iter().fold((f32::INFINITY, f32::NEG_INFINITY), |(lo, hi), q| (lo.min(q[1]), hi.max(q[1])))
}

/// The climb (m): the outline's highest point over its lowest (the CLIMB cell and the turntable's `+NN M`).
fn climb(p: &TrackProfile) -> f32 {
    let (lo, hi) = lap_span(p);
    if hi >= lo { hi - lo } else { 0.0 }
}

/// The lap for the turntable: `LAP_POINTS` even arc lengths of the outline as (game x, height above the lowest
/// point, game z), x and z about the centre of the lap's bounding box (the turntable's axis, as the mockup's data);
/// z stays the game's (up the screen, never mirrored).
fn lap3d(p: &TrackProfile) -> Vec<[f32; 3]> {
    let n = p.outline.len();
    if n < 2 {
        return Vec::new();
    }
    let (lo, _) = lap_span(p);
    let bx = p.outline.iter().fold([f32::INFINITY, f32::NEG_INFINITY, f32::INFINITY, f32::NEG_INFINITY], |m, q| [m[0].min(q[0]), m[1].max(q[0]), m[2].min(q[2]), m[3].max(q[2])]);
    let (mx, mz) = ((bx[0] + bx[1]) * 0.5, (bx[2] + bx[3]) * 0.5);
    (0..LAP_POINTS)
        .map(|k| {
            // the outline is uniform in arc length, so the index is the arc length's share
            let x = k as f32 / LAP_POINTS as f32 * n as f32;
            let (i, f) = (x.floor() as usize % n, x.fract());
            let (a, b) = (p.outline[i], p.outline[(i + 1) % n]);
            [a[0] + (b[0] - a[0]) * f - mx, a[1] + (b[1] - a[1]) * f - lo, a[2] + (b[2] - a[2]) * f - mz]
        })
        .collect()
}

/// Text in a 2D canvas's `fillText` placement: `align` 0.5 = centred on `x`, 1 = ending at `x`, baseline at `base`
/// (canvas px).
#[allow(clippy::too_many_arguments)]
fn canvas_text(b: &mut Batch, d: Glass, f: HudFont, px: f32, ls: f32, col: [f32; 4], s: &str, x: f32, base: f32, align: f32) {
    let st = d.font(f, px, ls, col);
    let w = b.font_width(f, s, &st) - st.tracking;
    b.font_text(f, s, x - w * align, d.cap_above(f, px, base), &st);
}

/// What the loading screen says about the race being loaded, taken when the screen opens (`Game::go`).
pub(crate) struct LoadFacts {
    pub(crate) track: usize,
    pub(crate) laps: u32,
    /// the header's mode name
    pub(crate) mode: &'static str,
    /// the event card's rows
    pub(crate) rows: Vec<(String, String)>,
    pub(crate) best_lap: Option<u32>,
    pub(crate) best_race: Option<u32>,
    /// cars on the grid (the GRID stage's words)
    pub(crate) field: usize,
    /// the car on the event card: yours, or the recorder's when a recording is watched
    pub(crate) car: String,
}

/// `m:ss.mmm` (the mockup's `fmtLap`), dashes without a time.
fn lap_time(ms: Option<u32>) -> String {
    ms.map_or_else(|| "-:--.---".into(), |ms| format!("{}:{:02}.{:03}", ms / 60_000, (ms / 1000) % 60, ms % 1000))
}

/// The stage line's words after `k OF 7 ·`.
fn stage_detail(s: Stage, f: &LoadFacts, cd: u32) -> String {
    match s {
        Stage::TrackData => "COLLISION \u{b7} RACING LINE \u{b7} DRIVERS".into(),
        Stage::World => "TRACK MESH \u{b7} TEXTURES".into(),
        Stage::Cars if f.field > 1 => format!("{} CARS \u{b7} PAINT \u{b7} WHEELS", f.field),
        Stage::Cars => format!("{} \u{b7} PAINT \u{b7} WHEELS", f.car),
        Stage::Scene => "PROPS \u{b7} LIGHTS \u{b7} SKY".into(),
        Stage::Remaster => "LIGHT PROBES \u{b7} WORLD FILL \u{b7} GRASS".into(),
        Stage::Audio => format!("ENGINES \u{b7} AMBIENCE \u{b7} CD TRACK {cd:02}"),
        Stage::Grid if f.field > 1 => "CARS TO THE GRID".into(),
        Stage::Grid => "CAR TO THE LINE".into(),
    }
}

impl Game {
    /// The facts of the race the loading screen loads: Ghost mode and a watched recording take the recording's
    /// track and laps (as `race_plan` / `watch_plan`), every other mode the selected ones.
    pub(crate) fn load_facts(&self) -> LoadFacts {
        let s = &self.settings;
        let on_off = |b: bool| if b { "ON" } else { "OFF" }.to_string();
        let cars = |n: usize| format!("{n} CARS");
        let laps_row = |n: u32| ("LAPS".to_string(), n.to_string());
        let watch = self.watch_next.as_ref();
        let ghost = self.ghost_demo.as_ref().filter(|_| self.game_type == GameType::GhostMode);
        let rec = watch.or(ghost);
        let track = rec.and_then(|m| self.ghost_track(m).ok()).unwrap_or(s.track);
        let laps = rec.map_or_else(|| self.shown_laps(), |m| m.header.laps);
        // the recording's driver and car (`watch_plan`'s slot)
        let recorder = |m: &motor_data::mde::Mde| {
            let h = &m.header;
            let slots = h.active_slots();
            let slot = if slots.contains(&(h.player as usize)) { h.player as usize } else { slots.first().copied().unwrap_or(0) };
            h.cars.get(slot).map_or_else(|| (String::new(), String::new()), |c| (c.name.to_uppercase(), c.car.to_uppercase()))
        };
        let own = self.cars.get(s.car).map_or_else(String::new, |c| c.name.to_uppercase());
        let car = watch.map_or(own, |m| recorder(m).1);
        let (mode, rows, field): (&'static str, Vec<(String, String)>, usize) = if let Some(m) = watch {
            let who = recorder(m).0;
            ("RECORDING", vec![("MODE".into(), "RECORDED RACE".into()), ("WATCHING".into(), who), laps_row(laps), ("CATCH-UP".into(), on_off(m.header.catchup()))], 1)
        } else {
            match self.game_type {
                GameType::GhostMode => {
                    let who = ghost.map(|m| recorder(m).0).unwrap_or_default();
                    ("GHOST RACE", vec![("MODE".into(), "GHOST RACE".into()), ("GHOST".into(), who), laps_row(laps), ("CATCH-UP".into(), on_off(ghost.is_some_and(|m| m.header.catchup())))], 1)
                }
                GameType::TimeAttack => {
                    let car = self.cars[s.car].id;
                    let have = hiscore::ghost_path(self.persist, &self.tracks[track].name, car).is_some_and(|p| p.exists());
                    let g = if !s.ghost_car { "OFF" } else if have { "YOUR BEST LAP" } else { "NONE YET" };
                    ("TIME ATTACK", vec![("MODE".into(), "TIME ATTACK".into()), ("GHOST".into(), g.into()), laps_row(laps), ("FIELD".into(), "YOU ALONE".into())], 1)
                }
                GameType::League => match self.leagues.get(self.league) {
                    Some(l) => {
                        let d = l.human_division();
                        let n = l.field().len();
                        let races = league::season_tracks(&self.track_codes(), d).len().max(1);
                        let div = if d == 0 { "FINAL".to_string() } else { d.to_string() };
                        let rows = vec![
                            ("MODE".into(), format!("LEAGUE \u{b7} {}", l.name.to_uppercase())),
                            ("DIVISION".into(), div),
                            ("RACE".into(), format!("{} OF {races}", (l.race_number as usize + 1).min(races))),
                            laps_row(laps),
                            ("FIELD".into(), cars(n)),
                        ];
                        ("LEAGUE", rows, n)
                    }
                    None => ("LEAGUE", vec![("MODE".into(), "LEAGUE".into()), laps_row(laps)], GRID_SIZE),
                },
                GameType::Single | GameType::Knockout => {
                    let mode = if self.game_type == GameType::Knockout { "KNOCKOUT" } else { "SINGLE RACE" };
                    let rows = vec![
                        ("MODE".into(), mode.into()),
                        ("DIFFICULTY".into(), difficulty_name(s.difficulty).to_uppercase()),
                        laps_row(laps),
                        ("FIELD".into(), cars(GRID_SIZE)),
                        ("CATCH-UP".into(), on_off(s.catchup)),
                    ];
                    (mode, rows, GRID_SIZE)
                }
            }
        };
        let rec_dir = hiscore::dir(self.persist);
        let records = hiscore::Records::load(rec_dir.as_deref(), &self.tracks[track].name, laps);
        LoadFacts { track, laps, mode, rows, best_lap: records.lap.map(|r| r.lap_ms), best_race: records.race.map(|r| r.race_ms), field, car }
    }

    /// The bar as drawn: the stage, the share of the bar (0..1) and READY (the scene is on the GPU). Without a
    /// load (the shots) `--load-progress` sets the share.
    fn load_shown(&self) -> (Stage, f32, bool) {
        match self.load_progress() {
            Some(_) if self.load_ready() => (Stage::Grid, 1.0, true),
            Some((st, p)) => (st, p, false),
            None => {
                let p = self.load_preview.unwrap_or(0.0).clamp(0.0, 1.0);
                (Stage::at(p), p, p >= 1.0)
            }
        }
    }

    /// REMASTER Loading screen (#90): the Glass layout of mockup B over the navy backdrop.
    pub(crate) fn loading_modern(&self, size: (u32, u32)) -> Vec<Layer> {
        let canvas = Canvas::hud(size);
        let px = 1.0 / canvas.transform(size).0;
        vec![self.ui_in(Batch::sharp_text(&self.assets, canvas, 1.0), |g, b| g.loading_body(b, px))]
    }

    fn loading_body(&self, b: &mut Batch, px: f32) {
        let d = Glass::new(&b.canvas, px);
        let fallback;
        let f = match &self.load_facts {
            Some(f) => f,
            None => {
                fallback = self.load_facts();
                &fallback
            }
        };
        let tr = &self.tracks[f.track.min(self.tracks.len() - 1)];
        let (stage, p, ready) = self.load_shown();
        Self::glass_backdrop(b, d);
        Self::loading_survey(b, d);
        if let Some(prof) = &tr.profile {
            // a shot holds the mockup's `?shot` pose: yaw 0.6, the head dot at full size
            let shot = self.load_preview.is_some();
            let yaw = TURN_YAW0 + if shot { 0.0 } else { self.t / TURN_S * TAU };
            let breathe = if shot { 1.0 } else { 0.75 + 0.25 * (self.t * 1000.0 / 180.0).sin() };
            Self::loading_turntable(b, d, prof, (p, ready), yaw, breathe, self.settings.player_colour);
        }
        let mut m = Mesh::default();
        Self::glass_title_wash(&mut m, d);
        b.mesh(&m);
        let header = format!("{} // LOADING // {}", f.mode, tr.name.to_uppercase());
        Self::glass_furniture(b, d, self.t, &header, if ready { "READY" } else { "LOADING" }, GlassRules::None);
        Self::glass_title_sized(b, d, "loading", "88-137:90", Some((54.0, 39.0)));
        Self::glass_plate(b, d, (d.r(1021.0), d.t(GLASS_PLATE_TOP), d.d(234.0), d.d(119.0)), "187-C-90", "LOADINGSCREEN");
        Self::glass_version(b, d);
        Self::loading_event(b, d, f);
        Self::loading_name(b, d, f, tr, self.tracks.len());
        Self::loading_progress(b, d, f, tr, (stage, p, ready));
        if !ready {
            Self::glass_hints_at(b, d, self.controls.last, &[(&[3], "CANCEL")], (640.0, 670.0), 0.8, 20.0);
        }
    }

    /// `.survey`: the light pool round the lap (760 x 560 at 860, 380) and `::after`, the dark wash from the left
    /// that the name block reads on.
    fn loading_survey(b: &mut Batch, d: Glass) {
        let (w, h) = (b.canvas.w, b.canvas.h);
        let mut m = Mesh::default();
        let (cx, cy, rx, ry) = (d.x(860.0), d.y(380.0), d.d(760.0 * 0.5), d.d(560.0 * 0.5));
        m.shade_rect(cx - rx, cy - ry, rx * 2.0, ry * 2.0, 32, 24, &|q| {
            let k = (((q[0] - cx) / rx).powi(2) + ((q[1] - cy) / ry).powi(2)).sqrt();
            [40.0 / 255.0, 110.0 / 255.0, 200.0 / 255.0, 0.20 * (1.0 - k / 0.7).clamp(0.0, 1.0)]
        });
        // linear-gradient(90deg, .85 at 0 %, .35 at 34 %, 0 at 52 %) of #02050b
        m.shade_rect(0.0, 0.0, w, h, 26, 1, &|q| {
            let u = q[0] / w;
            let a = if u < 0.34 { 0.85 + (0.35 - 0.85) * u / 0.34 } else { (0.35 * (1.0 - (u - 0.34) / 0.18)).max(0.0) };
            [2.0 / 255.0, 5.0 / 255.0, 11.0 / 255.0, a]
        });
        b.mesh(&m);
    }

    /// The lap in 3D (`draw3D` of mockup B): a disc at the lap's lowest point with a 100 m grid and a tick ring, the
    /// lap's shadow, then the segments far to near (the curtain from the road down to the base, a post every 4th
    /// point, the road: lit in its sector's colour up to `p`), the high point and the base marked, S/F, the head dot
    /// in the driver's colour while loading, the caption. `yaw` turns the table; `breathe` scales the head's pool.
    fn loading_turntable(b: &mut Batch, d: Glass, prof: &TrackProfile, (p, ready): (f32, bool), yaw: f32, breathe: f32, colour: [u8; 3]) {
        let pts = lap3d(prof);
        let n = pts.len();
        if n < 3 {
            return;
        }
        // canvas px -> window canvas
        let (ox, oy, u) = (d.x(VIEW.0), d.y(VIEW.1), d.d(1.0));
        let at = |q: [f32; 2]| [ox + q[0] * u, oy + q[1] * u];
        let r = pts.iter().map(|q| q[0].hypot(q[2])).fold(0.0, f32::max) * 1.08;
        let (cs, sn, cp, sp, ff) = (yaw.cos(), yaw.sin(), PITCH.cos(), PITCH.sin(), r * 5.0);
        // fit: the tick ring (1.08 R; a circle, so the same at every yaw) and the highest road over it, 12 px margin;
        // 36 px over it for the high point's label, 34 under it for the caption; centred in what is left
        let top0 = pts.iter().map(|q| q[1]).fold(0.0, f32::max) * EX * cp;
        let (kn, kf) = (ff / (ff - r * 1.08 * cp), ff / (ff + r * 1.08 * cp));
        let room = VIEW.3 - 36.0 - 34.0;
        let (up, down) = (top0 * kf * 1.15 + r * 1.08 * sp * kf, r * 1.08 * sp * kn);
        let sc = ((VIEW.2 - 24.0) / (2.0 * r * 1.08 * kn)).min(room / (up + down));
        let (cx, cy) = (VIEW.2 * 0.5, 36.0 + (room - (up + down) * sc) * 0.5 + up * sc);
        // world (x, h, z) -> window [x, y] and the depth (growing away from the viewer)
        let pr = |x: f32, h: f32, z: f32| -> ([f32; 2], f32) {
            let (xr, dd) = (x * cs - z * sn, x * sn + z * cs);
            let k = ff / (ff + dd * cp);
            (at([cx + xr * sc * k, cy - (dd * sp + h * EX * cp) * sc * k]), dd)
        };
        let flat = |x: f32, z: f32| pr(x, 0.0, z).0;
        let ice = |a: f32| hex(ICE, a);
        let mut m = Mesh::default();

        // the turntable: the disc's radial gradient (rgba(60,140,255,.16) at the centre to rgba(10,30,60,.05) at
        // R·sc) as a lattice of projected rings, so every vertex carries its own share
        let (c0, gr) = (at([cx, cy]), r * sc * u);
        let disc_col = |q: [f32; 2]| {
            let t = ((q[0] - c0[0]).hypot(q[1] - c0[1]) / gr).min(1.0);
            let mix = |a: f32, b: f32| (a + (b - a) * t) / 255.0;
            [mix(60.0, 10.0), mix(140.0, 30.0), mix(255.0, 60.0), 0.16 + (0.05 - 0.16) * t]
        };
        let (rings, segs) = (8usize, 96usize);
        let base_v = m.verts.len() as u32;
        let ctr = flat(0.0, 0.0);
        m.verts.push(vec::V { pos: ctr, color: disc_col(ctr) });
        for k in 1..=rings {
            let rr = r * k as f32 / rings as f32;
            for i in 0..segs {
                let a = i as f32 / segs as f32 * TAU;
                let q = flat(a.cos() * rr, a.sin() * rr);
                m.verts.push(vec::V { pos: q, color: disc_col(q) });
            }
        }
        let ring = |k: usize, i: usize| if k == 0 { base_v } else { base_v + 1 + ((k - 1) * segs + i % segs) as u32 };
        for k in 1..=rings {
            for i in 0..segs {
                let (a, bq) = (ring(k, i), ring(k, i + 1));
                if k == 1 {
                    m.idx.extend([base_v, a, bq]);
                } else {
                    m.idx.extend([ring(k - 1, i), a, bq, ring(k - 1, i), bq, ring(k - 1, i + 1)]);
                }
            }
        }
        // the 100 m grid, clipped to the disc in the world (straight lines stay straight in perspective)
        let mut v = -(r / 100.0).floor() * 100.0;
        while v <= r {
            let half = (r * r - v * v).max(0.0).sqrt();
            m.stroke(&[flat(v, -half), flat(v, half)], false, d.w(1.0), Ends::Butt, ice(0.07), d.px);
            m.stroke(&[flat(-half, v), flat(half, v)], false, d.w(1.0), Ends::Butt, ice(0.07), d.px);
            v += 100.0;
        }
        let rim: Vec<[f32; 2]> = (0..96).map(|i| i as f32 / 96.0 * TAU).map(|a| flat(a.cos() * r, a.sin() * r)).collect();
        m.stroke(&rim, true, d.w(1.0), Ends::Butt, ice(0.3), d.px);
        for i in 0..80 {
            let a = i as f32 / 80.0 * TAU;
            let major = i % 8 == 0;
            let (r0, r1) = (r * 1.02, r * if major { 1.07 } else { 1.045 });
            let col = if major { hex(0xf4fbff, 0.7) } else { hex(G_TEXT, 0.35) };
            m.stroke(&[flat(a.cos() * r0, a.sin() * r0), flat(a.cos() * r1, a.sin() * r1)], false, d.w(if major { 1.6 } else { 1.0 }), Ends::Butt, col, d.px);
        }

        // the lap's shadow on the base
        let top: Vec<([f32; 2], f32)> = pts.iter().map(|q| pr(q[0], q[1], q[2])).collect();
        let base: Vec<[f32; 2]> = pts.iter().map(|q| flat(q[0], q[2])).collect();
        m.stroke(&base, true, d.w(5.0), Ends::Round, [2.0 / 255.0, 6.0 / 255.0, 14.0 / 255.0, 0.55], d.px);
        m.stroke(&base, true, d.w(1.2), Ends::Round, ice(0.14), d.px);

        // the segments far to near: the curtain from the road down to the base, its post, then the road
        let mut order: Vec<usize> = (0..n).collect();
        order.sort_by(|&a, &b| {
            let depth = |i: usize| top[i].1 + top[(i + 1) % n].1;
            depth(b).total_cmp(&depth(a))
        });
        let sector_col = |f: f32| {
            let s = f * prof.length;
            if s < prof.sectors[0] { G_S1 } else if s < prof.sectors[1] { G_S2 } else { G_S3 }
        };
        for &i in &order {
            let j = (i + 1) % n;
            let f = (i as f32 + 0.5) / n as f32;
            let lit = ready || f < p;
            let col = sector_col(f);
            let (a, bq, bb, ba) = (top[i].0, top[j].0, base[j], base[i]);
            if pts[i][1] + pts[j][1] > 0.4 {
                let (y0, y1) = (a[1].min(bq[1]), ba[1].max(bb[1]));
                let (a0, a1, c) = if lit { (0.34, 0.03, col) } else { (0.2, 0.02, ICE) };
                m.fill_with(
                    &[a, bq, bb, ba],
                    &|q| {
                        let t = if y1 > y0 { ((q[1] - y0) / (y1 - y0)).clamp(0.0, 1.0) } else { 0.0 };
                        hex(c, a0 + (a1 - a0) * t)
                    },
                    d.px * 0.5,
                );
                if i % 4 == 0 && pts[i][1] > 1.0 {
                    m.stroke(&[a, ba], false, d.w(1.0), Ends::Butt, if lit { hex(col, 0.35) } else { ice(0.14) }, d.px);
                }
            }
            if lit {
                // APPROX: `shadowBlur 8` in the sector colour as two wide faint strokes under the line
                m.stroke(&[a, bq], false, d.d(3.5 + 12.0), Ends::Round, hex(col, 0.06), d.px);
                m.stroke(&[a, bq], false, d.d(3.5 + 6.0), Ends::Round, hex(col, 0.14), d.px);
                m.stroke(&[a, bq], false, d.w(3.5), Ends::Round, hex(col, 1.0), d.px);
            } else {
                m.stroke(&[a, bq], false, d.w(2.5), Ends::Round, ice(0.42), d.px);
            }
        }
        // the high point: a dashed post down to the base
        // the first point of the top plateau and of the bottom one (the mockup's data is rounded to 0.1 m, so its first
        // maximum and minimum; heights are above the lowest point, so the bottom is 0)
        let top_h = pts.iter().map(|q| q[1]).fold(0.0, f32::max);
        let hi = (0..n).find(|&i| pts[i][1] >= top_h - 0.05).unwrap_or(0);
        let lo = (0..n).find(|&i| pts[i][1] <= 0.05).unwrap_or(0);
        let (ht, hb) = (top[hi].0, base[hi]);
        m.dashed(ht, hb, d.d(3.0), d.d(3.0), d.w(1.0), hex(0xf4fbff, 0.6), d.px);
        b.mesh(&m);
        let dim = hex(G_DIM, 1.0);
        canvas_text(b, d, HudFont::Glass, 15.0, 0.0, WHITE, &format!("+{} M", climb(prof).round() as u32), ht[0], ht[1] - d.d(12.0), 0.5);
        canvas_text(b, d, HudFont::GlassMed, 10.0, 0.0, dim, "HIGH POINT", ht[0], ht[1] - d.d(27.0), 0.5);
        // the low point is the base (0 M)
        let lb = base[lo];
        let mut m = Mesh::default();
        m.fill(&circle(lb, d.d(3.0)), hex(G_TEXT, 1.0), d.px);
        b.mesh(&m);
        canvas_text(b, d, HudFont::GlassMed, 10.0, 0.0, dim, "BASE 0 M", lb[0], lb[1] + d.d(16.0), 0.5);
        // start / finish
        let sf = top[0].0;
        let mut m = Mesh::default();
        m.fill(&circle(sf, d.d(5.0)), hex(0x04070d, 1.0), d.px);
        m.stroke(&circle(sf, d.d(5.0)), true, d.w(2.0), Ends::Round, WHITE, d.px);
        b.mesh(&m);
        canvas_text(b, d, HudFont::Glass, 10.0, 0.0, hex(G_TEXT, 1.0), "S/F", sf[0] - d.d(10.0), sf[1] + d.d(4.0), 1.0);
        // the progress head: where the load has got to along the lap
        if !ready {
            let x = p.clamp(0.0, 1.0) * n as f32;
            let (i, k) = (x.floor() as usize % n, x.fract());
            let (q0, q1) = (pts[i], pts[(i + 1) % n]);
            let h = pr(q0[0] + (q1[0] - q0[0]) * k, q0[1] + (q1[1] - q0[1]) * k, q0[2] + (q1[2] - q0[2]) * k).0;
            let mut m = Mesh::default();
            let (pool, dot) = (circle(h, d.d(14.0)), circle(h, d.d(7.0)));
            // APPROX: `shadowBlur 12 #6f8dff` as halos round the pool and the dot
            m.halo(&pool, d.d(12.0), [111.0 / 255.0, 141.0 / 255.0, 1.0, 0.25 * breathe * 0.6]);
            m.fill(&pool, rgb_hex(colour, 0.25 * breathe), d.px);
            m.halo(&dot, d.d(12.0), [111.0 / 255.0, 141.0 / 255.0, 1.0, 0.6]);
            m.fill(&dot, rgb_hex(colour, 1.0), d.px);
            m.stroke(&dot, true, d.w(2.4), Ends::Round, WHITE, d.px);
            b.mesh(&m);
        }
        // X: the Glass font has no multiplication sign
        let cap = format!("HEIGHTS X {EX:.0} \u{b7} BASE = LOWEST POINT OF THE LAP \u{b7} {:.2} KM", prof.length / 1000.0);
        let foot = at([cx, VIEW.3 - 6.0]);
        canvas_text(b, d, HudFont::GlassMed, 11.0, 1.5, dim, &cap, foot[0], foot[1], 0.5);
    }

    /// The event card: `EVENT · <car>` over the mode's rows (`.kv`, 25 px lines, a hairline under each but the last).
    fn loading_event(b: &mut Batch, d: Glass, f: &LoadFacts) {
        let (x, y, w, h) = (d.l(CARD.0), d.y(CARD.1), d.d(CARD.2), d.d(CARD.3));
        let mut m = Mesh::default();
        glass_card(&mut m, d, (x, y, w, h), d.d(12.0), (0.13, 0.04), 0.36, (10.0, 26.0, 0.45), 0.35, None);
        b.mesh(&m);
        let dim = hex(G_DIM, 1.0);
        txt(b, d, HudFont::GlassMed, 11.0, 1.5, dim, &format!("EVENT \u{b7} {}", f.car), x + d.d(16.0), y + d.d(12.0), None);
        let st = d.font(HudFont::GlassMed, 15.0, 0.5, WHITE);
        let kcol = f.rows.iter().map(|(k, _)| b.font_width(HudFont::GlassMed, k, &st) - st.tracking).fold(0.0, f32::max);
        let (left, right) = (x + d.d(16.0), x + d.d(16.0 + 268.0));
        let mut m = Mesh::default();
        for (i, (k, v)) in f.rows.iter().enumerate() {
            let top = y + d.d(28.0 + 25.0 * i as f32);
            txt(b, d, HudFont::GlassMed, 15.0, 0.5, dim, k, left, top, Some(25.0));
            txt_r(b, d, HudFont::GlassMed, 15.0, 0.5, WHITE, v, right, top, Some(25.0));
            if i + 1 < f.rows.len() {
                let ly = top + d.d(25.0) - d.w(1.0) * 0.5;
                for (a, z) in [(left, left + kcol), (left + kcol + d.d(14.0), right)] {
                    m.stroke(&[[a, ly], [z, ly]], false, d.w(1.0), Ends::Butt, hex(G_LINE, 0.08), d.px);
                }
            }
        }
        b.mesh(&m);
    }

    /// The name block at (62, 400), 560 wide: `TRACK n OF 8 · CD TRACK nn` (+ BONUS TRACK), the name in the chromatic
    /// style shrunk to fit, the location, then the cells LENGTH / LAPS / CLIMB / BEST LAP / BEST RACE.
    fn loading_name(b: &mut Batch, d: Glass, f: &LoadFacts, tr: &TrackInfo, count: usize) {
        let (x, mut top) = (d.l(62.0), d.y(400.0));
        let dim = hex(G_DIM, 1.0);
        let cap = format!("TRACK {} OF {count} \u{b7} CD TRACK {:02}", tr.n, tr.cd_track);
        let cw = txt(b, d, HudFont::GlassMed, 11.0, 1.5, dim, &cap, x, top, Some(14.0));
        if tr.division % 100 >= 10 {
            tag(b, d, "BONUS TRACK", (x + cw + d.d(6.0), top - d.d(2.0)), G_ORANGE, 0.6);
        }
        top += d.d(14.0 + 4.0);
        // bigName: 58 px, smaller when the name is wider than 560; line height 1.1 of the size
        let name = tr.name.to_uppercase();
        let full = b.font_width(HudFont::Glass, &name, &d.font(HudFont::Glass, 58.0, 2.0, WHITE));
        let k = (d.d(560.0) / full).min(1.0);
        let size = (58.0 * k).floor();
        let lh = (size * 1.1).round();
        Self::loading_big_name(b, d, &name, (x, top), size, lh);
        top += d.d(lh + 6.0);
        // the location line (17 px, the pin icon in the dim colour, 8 px gap)
        Self::glass_icon(b, ui_font::icon::MAP_PIN, x, top + d.d(1.5), d.d(17.0), dim);
        txt(b, d, HudFont::GlassMed, 17.0, 1.5, hex(G_TEXT, 1.0), &tr.location.to_uppercase(), x + d.d(25.0), top, Some(20.0));
        top += d.d(20.0 + 12.0);
        // the cells: caption over value, 18 px padding, a hairline between
        let climb = tr.profile.as_ref().map(|p| climb(p).round() as u32);
        let km = if tr.length_m > 0.0 { tr.length_m } else { tr.profile.as_ref().map_or(0.0, |p| p.length) } / 1000.0;
        let cells: [(&str, String, Option<&str>, bool); 5] = [
            ("LENGTH", format!("{km:.2}"), Some("KM"), true),
            ("LAPS", f.laps.to_string(), None, true),
            ("CLIMB", climb.map_or_else(|| "--".into(), |c| c.to_string()), climb.map(|_| "M"), climb.is_some()),
            ("BEST LAP", lap_time(f.best_lap), None, f.best_lap.is_some()),
            ("BEST RACE", lap_time(f.best_race), None, f.best_race.is_some()),
        ];
        let mut cx = x;
        let mut m = Mesh::default();
        for (i, (label, value, unit, have)) in cells.iter().enumerate() {
            if i > 0 {
                cx += d.d(18.0);
                m.stroke(&[[cx, top], [cx, top + d.d(14.0 + 24.0)]], false, d.w(1.0), Ends::Butt, hex(G_LINE, 0.14), d.px);
                cx += d.d(18.0);
            }
            let mut wcap = txt(b, d, HudFont::GlassMed, 11.0, 1.5, dim, label, cx, top, Some(14.0));
            if i == 3 && f.best_lap.is_some() {
                // `.tag`: PB in the small orange caps beside its caption
                wcap += d.d(4.0) + txt(b, d, HudFont::Glass, 10.0, 1.5, hex(G_ORANGE, 1.0), "PB", cx + wcap + d.d(4.0), top, Some(14.0));
            }
            let col = if *have { WHITE } else { [191.0 / 255.0, 227.0 / 255.0, 243.0 / 255.0, 0.35] };
            let mut wv = txt(b, d, HudFont::Glass, 20.0, 0.5, col, value, cx, top + d.d(14.0), Some(24.0));
            if let Some(u) = unit {
                let base = d.cap_top(HudFont::Glass, 20.0, top + d.d(14.0), Some(24.0)) + d.font(HudFont::Glass, 20.0, 0.0, WHITE).size;
                let st = d.font(HudFont::GlassMed, 12.0, 1.0, dim);
                wv += d.d(4.0) + b.font_text(HudFont::GlassMed, u, cx + wv + d.d(4.0), d.cap_above(HudFont::GlassMed, 12.0, base), &st);
            }
            cx += wcap.max(wv) - d.font(HudFont::Glass, 20.0, 0.5, WHITE).tracking;
        }
        b.mesh(&m);
    }

    /// `.bigname.chroma`: the name with its red / cyan offsets and the soft blue glow, `size` px on a `lh` line box
    /// whose top left is (x, top) in canvas px.
    fn loading_big_name(b: &mut Batch, d: Glass, name: &str, (x, top): (f32, f32), size: f32, lh: f32) {
        let st = d.font(HudFont::Glass, size, 2.0, hex(0xf4fbff, 1.0));
        let y = d.cap_top(HudFont::Glass, size, top, Some(lh));
        let nw = b.font_width(HudFont::Glass, name, &st) - st.tracking;
        let mut m = Mesh::default();
        // APPROX: the 26 px text glow as a soft pad behind the run (as `league_word`)
        m.soft_pad(&vec::rounded_rect(x + d.d(4.0), y + d.d(4.0), nw - d.d(8.0), st.size - d.d(8.0), d.d(10.0)), d.d(26.0), [120.0 / 255.0, 200.0 / 255.0, 1.0, 0.22]);
        b.mesh(&m);
        b.font_text(HudFont::Glass, name, x - d.d(1.5), y, &FontStyle { color: [1.0, 70.0 / 255.0, 70.0 / 255.0, 0.55], ..st });
        b.font_text(HudFont::Glass, name, x + d.d(1.5), y, &FontStyle { color: [70.0 / 255.0, 200.0 / 255.0, 1.0, 0.55], ..st });
        b.font_text(HudFont::Glass, name, x, y, &st);
    }

    /// The stage line (name, `k OF 7 · detail`), the percentage, the stage labels and the bar with its stage ticks.
    fn loading_progress(b: &mut Batch, d: Glass, f: &LoadFacts, tr: &TrackInfo, (stage, p, ready): (Stage, f32, bool)) {
        let dim = hex(G_DIM, 1.0);
        // the stage line, both runs on one baseline
        let (x, top) = (d.l(62.0), d.y(596.0));
        let name_top = d.cap_top(HudFont::Glass, 25.0, top, None);
        let base = name_top + d.font(HudFont::Glass, 25.0, 0.0, WHITE).size;
        let (word, col) = if ready { ("READY", hex(G_ORANGE, 1.0)) } else { (stage.name(), WHITE) };
        let w = b.font_text(HudFont::Glass, word, x, name_top, &d.font(HudFont::Glass, 25.0, 1.0, col));
        let k = Stage::ALL.iter().position(|&s| s == stage).unwrap_or(0);
        let detail = if ready {
            if f.field > 1 { "TO THE GRID".to_string() } else { "TO THE LINE".to_string() }
        } else {
            format!("{} OF {} \u{b7} {}", k + 1, Stage::ALL.len(), stage_detail(stage, f, tr.cd_track))
        };
        let st = d.font(HudFont::GlassMed, 13.0, 1.0, dim);
        b.font_text(HudFont::GlassMed, &detail, x + w + d.d(14.0), d.cap_above(HudFont::GlassMed, 13.0, base), &st);
        // the percentage: 40 px with an 18 px % in the dim colour, its right edge at the bar's
        let pc = ((p * 100.0).floor() as u32).min(100).to_string();
        let right = d.r(BAR.1);
        let big = d.font(HudFont::Glass, 40.0, 1.0, WHITE);
        let small = d.font(HudFont::Glass, 18.0, 1.0, dim);
        let pw = b.font_width(HudFont::Glass, "%", &small) - small.tracking;
        let nw = b.font_width(HudFont::Glass, &pc, &big);
        let ntop = d.cap_top(HudFont::Glass, 40.0, d.y(588.0), Some(44.0));
        let nbase = ntop + big.size;
        b.font_text(HudFont::Glass, &pc, right - pw - d.d(2.0) - nw, ntop, &big);
        b.font_text(HudFont::Glass, "%", right - pw, d.cap_above(HudFont::Glass, 18.0, nbase), &small);
        // the stage labels over their segments: done dim, the current orange, the rest faint; a label never starts
        // before the previous one ends (the short stages' segments are narrower than their names)
        let (x0, x1) = (d.l(BAR.0), d.r(BAR.1));
        let bw = x1 - x0;
        let mut after = x0;
        for (i, s) in Stage::ALL.iter().enumerate() {
            let col = if ready || i < k { dim } else if i == k { hex(G_ORANGE, 1.0) } else { [191.0 / 255.0, 227.0 / 255.0, 243.0 / 255.0, 0.35] };
            let lx = (x0 + s.span().0 * bw).max(after);
            after = lx + txt(b, d, HudFont::Glass, 10.0, 1.5, col, s.name(), lx, d.y(636.0), None) + d.d(10.0);
        }
        // the bar: the line, the orange fill with its glow and the white head, dark ticks where the stages start
        let (y, h) = (d.y(BAR_TOP), d.d(4.0));
        let mut m = Mesh::default();
        m.fill(&vec::rounded_rect(x0, y, bw, h, h * 0.5), hex(G_LINE, 0.14), d.px);
        let fw = (bw * p).max(h);
        let fill = vec::rounded_rect(x0, y, fw, h, h * 0.5);
        m.halo(&fill, d.d(18.0), [1.0, 154.0 / 255.0, 60.0 / 255.0, 0.3]);
        m.halo(&fill, d.d(8.0), [1.0, 154.0 / 255.0, 60.0 / 255.0, 0.6]);
        m.fill(&fill, hex(G_ORANGE, 1.0), d.px);
        for s in &Stage::ALL[1..] {
            let tx = x0 + s.span().0 * bw;
            m.fill(&[[tx - d.d(1.0), y - d.d(3.0)], [tx + d.d(1.0), y - d.d(3.0)], [tx + d.d(1.0), y + h + d.d(3.0)], [tx - d.d(1.0), y + h + d.d(3.0)]], hex(0x04070d, 1.0), d.px);
        }
        if !ready {
            let head = circle([x0 + fw + d.d(2.0), y + h * 0.5], d.d(5.0));
            m.halo(&head, d.d(20.0), [1.0, 154.0 / 255.0, 60.0 / 255.0, 0.45]);
            m.halo(&head, d.d(10.0), [1.0, 154.0 / 255.0, 60.0 / 255.0, 0.8]);
            m.fill(&head, hex(0xfff3e6, 1.0), d.px);
        }
        b.mesh(&m);
    }
}
