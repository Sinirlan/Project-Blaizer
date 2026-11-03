//! REMASTER Car screen in the Glass HUD (#95, mockup A · Turntable of
//! `reference_materials/car-overhaul/A-turntable.html`): the Track screen's twin. Phase 1: the frame, the selector,
//! the setup cards of the left column and the RACE SETTINGS strip. Phase 2: the disc holds the garage car, turning
//! on a grid floor (the car's picture until the garage is loaded). Phase 3: the three stat dials down the right (GRIP, ACC, SPEED, the
//! Classic dials' numbers) and the SPEC card. Keys are the Classic arm's
//! (`Screen::Car` in front.rs): 0 the car, 1 transmission, 2 car horn, 3 record race, 4 ghost car (Time Attack).

use super::pause_settings::check_box;
use super::personal::{PREVIEW_PITCH, LIVERY_TURN, PreviewPose};
use super::*;

/// The disc's face (design px): its centre and radius, and the sphere the car is fitted into (6 px inside the face).
const FACE: ((f32, f32), f32) = ((622.0, 288.0), 170.0);
const CAR_ACROSS: f32 = 328.0;
/// The mockup's lens (12 m away at 68 px per metre) and the car's still pose: 0.64 rad (`TT.still` 2.5 from the
/// tail) off the nose, to its left, turning on at the Livery car's rate.
const CAR_ZOOM: f32 = 4.8;
const CAR_STILL: f32 = 0.64;
/// The floor disc's radius in metres (`ref` 5.6 x 0.62) and the grid's colour (`rgb(143,208,255)`).
const FLOOR_R: f32 = 5.6 * 0.62;
const FLOOR_RGB: u32 = 0x8fd0ff;

/// the left column's cards (design px): x, width and the tops / heights of Transmission, Car horn, Record race, Ghost car
const CARD_X: f32 = 115.0;
const CARD_W: f32 = 204.0;
const CHOICE_H: f32 = 58.0;
const SWITCH_H: f32 = 45.0;
const CARD_TOPS: [f32; 4] = [214.0, 300.0, 372.0, 431.0];

/// the stat dials (design px): centre and diameter, GRIP, ACC, SPEED (the mockup's cascade down the right)
const DIALS: [((f32, f32), f32); 3] = [((1150.0, 212.0), 108.0), ((1053.5, 326.6), 128.0), ((942.5, 458.4), 150.0)];
/// the SPEC card (design px): x, top, width, height
const SPEC: (f32, f32, f32, f32) = (1050.0, 418.0, 200.0, 116.0);

impl Game {
    /// The card of selection item `i` (1..=4) in canvas px.
    fn car_card(d: Glass, i: usize) -> (f32, f32, f32, f32) {
        let h = if i <= 2 { CHOICE_H } else { SWITCH_H };
        (d.x(CARD_X), d.y(CARD_TOPS[i - 1]), d.d(CARD_W), d.d(h))
    }

    /// REMASTER Car screen in the Glass HUD. Selection items keep the Classic order: 0 the selector, 1
    /// transmission, 2 car horn, 3 record race, 4 ghost car (Time Attack only).
    pub(crate) fn car_modern(&self, size: (u32, u32)) -> Vec<Layer> {
        let canvas = Canvas::hud(size);
        let px = 1.0 / canvas.transform(size).0;
        let d = Glass::new(&canvas, px);
        let rows = if self.game_type == GameType::TimeAttack { 4 } else { 3 };
        let plate = (d.r(1021.0), d.t(GLASS_PLATE_TOP), d.d(234.0), d.d(119.0));
        let settings = (d.x(327.0), d.y(615.0), d.d(660.0), d.d(82.0));
        let spec = (d.x(SPEC.0), d.y(SPEC.1), d.d(SPEC.2), d.d(SPEC.3));
        let blur: Vec<(f32, f32, f32, f32, f32)> = (1..=rows).map(|i| Self::car_card(d, i)).chain([plate, settings, spec]).map(|(x, y, w, h)| (x, y, w, h, d.d(12.0))).collect();
        let back = self.ui_in(Batch::sharp_text(&self.assets, canvas, 1.0), |_, b| Self::glass_backdrop(b, d));
        // REMASTER the garage car in the disc (None until the garage scene is up: the photo stands in)
        let turn = self.clock * LIVERY_TURN;
        let pose = self.preview_pose(size, d, FACE.0, CAR_ACROSS, (CAR_STILL + turn, PREVIEW_PITCH), CAR_ZOOM);
        let front = self.ui_in(Batch::sharp_text(&self.assets, canvas, 1.0), |g, b| {
            let car = &g.cars[g.settings.car];
            let pulse = 0.5 - 0.5 * (g.t * TAU / 1.1).cos();
            let allowed = g.car_allowed() && !g.car_locked();
            Self::glass_overlays(b, d);
            Self::glass_furniture(b, d, g.t, "CAR SELECT // GARAGE BAY 5E", "LIVE", GlassRules::Car);
            Self::glass_title(b, d, "car", "88-137:59");
            if let Some(p) = &pose {
                Self::glass_disc_back(b, d, turn);
                g.car_floor(b, d, p, car.footprint);
            } else {
                Self::glass_photo(b, d, format!("car:{}", car.id));
                if !allowed {
                    // a car the league's division may not drive: the face dimmed, the tag over it
                    g.car_blocked(b, d, px, 0.55);
                }
            }
            Self::glass_plate(b, d, plate, "187-C-35", "CARSCREEN");
            Self::glass_selector(b, d, &car.name.to_uppercase(), g.sel == 0, pulse, (g.settings.car == 0, g.settings.car + 1 >= g.locks().cars));
            let step = if g.sel == 0 { "CAR" } else { "CHANGE" };
            // RACE is dimmed for a car the league's division may not drive (`.hint.off`)
            let off: &[usize] = if allowed || g.sel != 0 { &[] } else { &[2] };
            // Enter starts the race on the car row only; on a setup row it changes the row
            let enter = if g.sel == 0 { "RACE" } else { "CHANGE" };
            Self::glass_hints_off(b, d, g.controls.last, &[(&[0, 1], step), (&[9, 10], "MOVE"), (&[2], enter), (&[3], "BACK")], (622.0, 561.0), (1.0, 28.0), off);
            g.car_cards(b, d, rows, pulse);
            g.car_dials(b, d);
            g.car_spec(b, d, spec);
            Self::glass_settings(b, d, settings, "RACE SETTINGS", &g.car_info());
            Self::glass_version(b, d);
        });
        let mut out = vec![back];
        out.extend(self.hologram(size, None, &blur));
        out.push(front);
        if let Some(p) = pose {
            // the car over the floor, then the face's gloss, vignette and gap over the car
            let paint = self.preview_paint(None);
            out.push(Layer::Preview { cam: p.cam, cars: p.cars, viewport: p.viewport, paint });
            let allowed = self.car_allowed() && !self.car_locked();
            out.push(self.ui_in(Batch::sharp_text(&self.assets, canvas, 1.0), |g, b| {
                // the vignette at half the Track disc's (.225 instead of .45) so the car stays clear to the rim [design choice, user request]
                Self::glass_disc_front(b, d, (hex(G_LINE, 0.0), 44.0, 0.24, 0.225));
                Self::glass_disc_labels(b, d);
                if !allowed {
                    g.car_blocked(b, d, px, 0.7);
                }
            }));
        }
        out
    }

    /// A car that can't be raced: the face under `shade` of black and the blinking tag. Not unlocked yet comes
    /// first (0x46d266), then the league division's rule.
    fn car_blocked(&self, b: &mut Batch, d: Glass, px: f32, shade: f32) {
        if self.car_locked() {
            self.glass_blocked(b, d, px, FACE.1, shade, "NOT YET AVAILABLE");
        } else {
            let division = self.current_league().map_or(3, |l| l.human_division());
            self.glass_blocked(b, d, px, FACE.1, shade, &format!("NOT AVAILABLE IN {}", Self::car_class(4 - division as u32).to_uppercase()));
        }
    }

    /// The disc's studio face and the car's floor under it: a soft top light (`.face.studio`), the grid disc, its rim ticks
    /// and the car's contact shadow (`TT.floor`, `TT.draw`), all in the car's model space so they turn with it, cut to
    /// the face. `foot` is the body's footprint (centre x / z, half x / z).
    fn car_floor(&self, b: &mut Batch, d: Glass, p: &PreviewPose, foot: [f32; 4]) {
        let c = [d.x(FACE.0 .0), d.y(FACE.0 .1)];
        let r = d.d(FACE.1);
        let mut m = Mesh::default();
        // radial-gradient(ellipse 70% 55% at 50% 36%, rgba(120,190,255,.30), rgba(14,34,64,.92) 62%, rgba(4,9,18,.98))
        let (fc, rx, ry) = ([c[0], c[1] - 0.28 * r], 1.4 * r, 1.1 * r);
        let stops: [(f32, [f32; 4]); 3] = [(0.0, [120.0 / 255.0, 190.0 / 255.0, 1.0, 0.30]), (0.62, [14.0 / 255.0, 34.0 / 255.0, 64.0 / 255.0, 0.92]), (1.0, [4.0 / 255.0, 9.0 / 255.0, 18.0 / 255.0, 0.98])];
        m.shade_disc(c, r, 24, 96, &|q| {
            let t = (((q[0] - fc[0]) / rx).powi(2) + ((q[1] - fc[1]) / ry).powi(2)).sqrt().min(1.0);
            let k = if t < stops[1].0 { 0 } else { 1 };
            let u = (t - stops[k].0) / (stops[k + 1].0 - stops[k].0);
            std::array::from_fn(|i| stops[k].1[i] + (stops[k + 1].1[i] - stops[k].1[i]) * u)
        });
        b.mesh(&m);
        let [fx, fz, hx, hz] = foot;
        let at = |x: f32, z: f32| p.project(Vec3::new(fx + x, p.ground, fz + z));
        let line = hex(FLOOR_RGB, 1.0);
        let mut m = Mesh::default();
        for k in 1..=3 {
            let rr = FLOOR_R * k as f32 / 3.0;
            let ring: Vec<[f32; 2]> = (0..64).map(|a| at(rr * (a as f32 / 64.0 * TAU).cos(), rr * (a as f32 / 64.0 * TAU).sin())).collect();
            m.stroke(&ring, true, d.w(1.0), Ends::Butt, [line[0], line[1], line[2], if k == 3 { 0.35 } else { 0.14 }], d.px);
        }
        for i in -4..=4 {
            let u = i as f32 / 4.0 * FLOOR_R;
            let w = (FLOOR_R * FLOOR_R - u * u).max(0.0).sqrt();
            for (a, e) in [(at(u, -w), at(u, w)), (at(-w, u), at(w, u))] {
                m.stroke(&[a, e], false, d.w(1.0), Ends::Butt, [line[0], line[1], line[2], 0.09], d.px);
            }
        }
        for k in 0..48 {
            let t = k as f32 / 48.0 * TAU;
            let (cs, sn) = (t.cos(), t.sin());
            let major = k % 12 == 0;
            let r1 = FLOOR_R * if major { 1.12 } else { 1.06 };
            let col = if major { hex(0xf4fbff, 0.8) } else { hex(0xbfe3f3, 0.4) };
            m.stroke(&[at(cs * FLOOR_R * 1.02, sn * FLOOR_R * 1.02), at(cs * r1, sn * r1)], false, d.w(if major { 1.6 } else { 1.0 }), Ends::Butt, col, d.px);
        }
        // the footprint, softened by five stacked copies about its centre
        let corners = [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)].map(|(sx, sz)| at(sx * hx * 1.02, sz * hz * 0.95));
        let c0 = at(0.0, 0.0);
        for k in [1.26, 1.17, 1.09, 1.02, 0.94] {
            let poly = corners.map(|q| [c0[0] + (q[0] - c0[0]) * k, c0[1] + (q[1] - c0[1]) * k]);
            m.fill(&poly, [0.0, 0.0, 0.0, 0.15], d.px);
        }
        b.clip_ellipse(c[0], c[1], r, r);
        b.mesh(&m);
        b.clip_none();
    }

    /// The three stat dials: the arcs ease with `Game::gauges` (the Classic dials' easing, 0x46d2d6) and map through
    /// `gauge_fills`; the numbers are the car's own. The tick marks the car the selector just left.
    fn car_dials(&self, b: &mut Batch, d: Glass) {
        let car = &self.cars[self.settings.car];
        let fills = gauge_fills(self.gauges);
        let prev = self.prev_car.filter(|&p| p != self.settings.car).and_then(|p| self.cars.get(p)).map(|p| gauge_fills(gauge_stats(p)));
        let texts = [
            (format!("{:.1}", car.handling), "", "GRIP"),
            (format!("{:.2}", car.accel), "S", "ACC"),
            (format!("{:.0}", car.top_speed), "KM/H", "SPEED"),
        ];
        for (k, (value, unit, caption)) in texts.into_iter().enumerate() {
            // DIALS runs GRIP, ACC, SPEED; the fills are SPEED, ACC, GRIP
            let j = 2 - k;
            Self::glass_stat_dial(b, d, &StatDial { c: DIALS[k].0, size: DIALS[k].1, fill: fills[j], prev: prev.map(|p| p[j]), value, unit, caption });
        }
    }

    /// The SPEC card (`specCardHTML`): the class, gears and weight, the size, and this car's best lap on the Track
    /// screen's circuit. The statistics keep one best lap per track with the car that set it, so the lap shows only
    /// when this car holds the record, else dashes [design choice, #95].
    fn car_spec(&self, b: &mut Batch, d: Glass, (x, y, w, h): (f32, f32, f32, f32)) {
        let car = &self.cars[self.settings.car];
        let mut m = Mesh::default();
        glass_card(&mut m, d, (x, y, w, h), d.d(12.0), (0.14, 0.04), 0.38, (10.0, 26.0, 0.45), 0.38, None);
        m.dashed([x + d.d(14.0), y + d.d(66.0)], [x + d.d(186.0), y + d.d(66.0)], d.d(6.0), d.d(4.0), d.w(1.0), hex(G_LINE, 0.45), d.px);
        b.mesh(&m);
        let track = self.car_info()[0][1].1.clone();
        let lap = self.stats.tracks.get(&track).and_then(|t| t.best_lap).filter(|&(_, id)| id == car.id).map_or_else(|| "--:--.---".to_string(), |(ms, _)| glass_time(ms));
        let cap = d.font(HudFont::GlassMed, 11.0, 1.5, hex(G_DIM, 1.0));
        let [len, wid, hgt] = car.size;
        b.font_text(HudFont::GlassMed, &format!("SPEC · {}", Self::car_class(league::tier(car.division)).to_uppercase()), x + d.d(14.0), d.cap_top(HudFont::GlassMed, 11.0, y + d.d(6.0), None), &cap);
        let val = d.font(HudFont::Glass, 17.0, 0.5, WHITE);
        b.font_text(HudFont::Glass, &format!("{} GEARS · {:.0} KG", car.gears, car.weight), x + d.d(14.0), d.cap_top(HudFont::Glass, 17.0, y + d.d(21.0), Some(22.0)), &val);
        b.font_text(HudFont::GlassMed, &format!("{len:.2} x {wid:.2} x {hgt:.2} M"), x + d.d(14.0), d.cap_top(HudFont::GlassMed, 13.0, y + d.d(43.0), None), &d.font(HudFont::GlassMed, 13.0, 1.0, hex(G_DIM, 1.0)));
        b.font_text(HudFont::GlassMed, &format!("YOUR BEST · {}", track.to_uppercase()), x + d.d(14.0), d.cap_top(HudFont::GlassMed, 11.0, y + d.d(73.0), None), &cap);
        b.font_text(HudFont::Glass, &lap, x + d.d(14.0), d.cap_top(HudFont::Glass, 17.0, y + d.d(86.0), Some(22.0)), &val);
    }

    /// The Car horn card's speaker icon (16 design px at `x`, `y`). It is dim; for 0.9 s after a horn is tried it is
    /// light blue (`--s1`) and pulses twice, 0.5 s each: `honk .5s ease-out 2`, 50 % = scale 1.25 and a blue glow.
    fn car_speaker(&self, b: &mut Batch, d: Glass, x: f32, y: f32) {
        let size = d.d(16.0);
        let e = self.honk_at.map_or(f32::MAX, |t| self.clock - t);
        if e >= 0.9 {
            return Self::glass_icon(b, ui_font::icon::VOLUME, x, y, size, hex(G_DIM, 1.0));
        }
        // the keyframe peaks at the middle of each 0.5 s cycle, eased out on the way in and the way back
        let u = (e % 0.5) / 0.5;
        let bump = 1.0 - (2.0 * u - 1.0).abs();
        let bump = bump * (2.0 - bump);
        let k = 1.0 + 0.25 * bump;
        let (cx, cy) = (x + size * 0.5, y + size * 0.5);
        let mut m = Mesh::default();
        m.glow_fill(&circle([cx, cy], size * 0.35), d.d(6.0), hex(0x4aa8ff, 0.6 * bump), 0);
        b.mesh(&m);
        let s = size * k;
        Self::glass_icon(b, ui_font::icon::VOLUME, cx - s * 0.5, cy - s * 0.5, s, hex(0x8fd0ff, 1.0));
    }

    /// The class name of a car tier (the mockup's `DIV_NAME`): tier 1 races in division 3, tier 4 is the final.
    pub(crate) fn car_class(tier: u32) -> &'static str {
        ["", "Division 3", "Division 2", "Division 1", "Final"][tier.min(4) as usize]
    }

    /// The setup cards: Transmission and Car horn as caption + choice, Record race and Ghost car as label + check box.
    fn car_cards(&self, b: &mut Batch, d: Glass, rows: usize, pulse: f32) {
        let s = &self.settings;
        let mut m = Mesh::default();
        for i in 1..=rows {
            let card = Self::car_card(d, i);
            let accent = (self.sel == i).then_some(0.3 + 0.2 * pulse);
            glass_card(&mut m, d, card, d.d(12.0), (0.14, 0.04), 0.38, (10.0, 26.0, 0.45), 0.38, accent);
            let (x, y, w, _) = card;
            if i <= 2 {
                // the glass triangles: Transmission dims the end it is at, the horn wraps
                let (first, last) = if i == 1 { (!s.manual, s.manual) } else { (false, false) };
                let mid = y + d.d(37.0);
                Self::glass_tri(&mut m, d, x + d.d(10.0), mid, true, self.sel == i, 1.0, first);
                Self::glass_tri(&mut m, d, x + w - d.d(10.0) - d.d(9.0), mid, false, self.sel == i, 1.0, last);
            } else {
                let on = if i == 3 { s.record_race } else { s.ghost_car };
                check_box(&mut m, d, (x + w - d.d(12.0) - d.d(22.0), y + d.d(10.0)), (22.0, 6.0), on, 1.0);
            }
        }
        b.mesh(&m);
        for i in 1..=rows {
            let (x, y, w, _) = Self::car_card(d, i);
            let lit = if self.sel == i { hex(G_ORANGE, 1.0) } else { hex(G_TEXT, 1.0) };
            if i <= 2 {
                let (cap, value) = if i == 1 {
                    ("TRANSMISSION".to_string(), if s.manual { "MANUAL" } else { "AUTOMATIC" }.to_string())
                } else {
                    let name = self.horns.get(s.horn).map_or("HORN".to_string(), |h| h.name.to_uppercase());
                    (format!("CAR HORN · {:02} / {}", s.horn + 1, self.horns.len().max(1)), name)
                };
                let cap_col = if self.sel == i { hex(G_ORANGE, 1.0) } else { hex(G_DIM, 1.0) };
                b.font_text(HudFont::GlassMed, &cap, x + d.d(14.0), d.cap_top(HudFont::GlassMed, 11.0, y + d.d(6.0), None), &d.font(HudFont::GlassMed, 11.0, 1.5, cap_col));
                if i == 2 {
                    self.car_speaker(b, d, x + w - d.d(12.0) - d.d(16.0), y + d.d(6.0));
                }
                let st = d.font(HudFont::Glass, 22.0, 1.0, WHITE);
                let vw = b.font_width(HudFont::Glass, &value, &st) - st.tracking;
                b.font_text(HudFont::Glass, &value, x + (w - vw) * 0.5, d.cap_top(HudFont::Glass, 22.0, y + d.d(22.0), Some(30.0)), &st);
            } else {
                let label = if i == 3 { "RECORD RACE" } else { "GHOST CAR" };
                b.font_text(HudFont::Glass, label, x + d.d(16.0), d.cap_top(HudFont::Glass, 25.0, y, Some(43.0)), &d.font(HudFont::Glass, 25.0, 0.5, lit));
            }
        }
    }
}
