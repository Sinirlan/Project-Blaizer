//! DEBUG motorview's Properties pane (` or F11; re-notes/00-debuger.md item 6): the long grids
//! the ribbon (`ribbon.rs`) has no room for. Live editing of the followed car's physics
//! parameters, the camera offsets and easing, the fog, sky and lamp tuning. Edits last until
//! the track or car is reloaded; nothing is saved.

use crate::drive::{CamMode, CamTune, Session};
use crate::render::{CarLightTune, FogColour, FogStyle, FogTune, LampTune, LocalLightTune, SkyTune, View};
use crate::icons as ic;
use crate::glass_ui;
use egui::{ComboBox, Frame, Ui};
use glam::Vec3;
use motor_sim::CarParams;

/// What the pane may edit besides the session.
pub struct Targets<'a> {
    pub view: &'a mut View,
    /// REMASTER: the loaded track and its own fog values (`Look::fog`), for the fog page
    pub track_fog: Option<(u32, FogTune)>,
    /// REMASTER: the loaded track and its HDRI dome (sky.cfg values in `Dome`), for the sky page
    pub track_sky: Option<(u32, crate::sky::Dome)>,
    /// the followed car's paint as drawn and its own (the car file's Colour1..3), for the car page
    pub paint: Option<(Paint, Paint)>,
    /// REMASTER local light: where the probe volume is (`Gpu::local_status`) and the probe
    /// gizmo layer (`debug::Layers::probes`; None: not offered)
    pub local_status: &'static str,
    pub probe_gizmos: Option<&'a mut bool>,
    /// the World Editor page's caption (the fill file and whether it is saved)
    pub world_sub: String,
}

/// the rail card's width with its margins, the dock's default width with its left margin, and the rail slot of the
/// hairline before the World Editor page (`A-paddock` `i === 8`)
const RAIL_W: f32 = 76.0;
const DOCK_W: f32 = 404.0;
const WORLD_AT: usize = 8;

/// CarColour1..3 (CustomCarColours)
pub type Paint = [[u8; 3]; 3];

/// The pane's pages: what a ribbon group's launcher (`↘`) opens.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Page {
    Sim,
    Car,
    Camera,
    Fog,
    Sky,
    Lamps,
    CarLights,
    LocalLight,
    World,
    Ghost,
    Cine,
    Constants,
}

impl Page {
    pub const ALL: [Page; 12] = [Page::Sim, Page::Car, Page::Camera, Page::Fog, Page::Sky, Page::Lamps, Page::CarLights, Page::LocalLight, Page::World, Page::Ghost, Page::Cine, Page::Constants];

    pub fn name(self) -> &'static str {
        match self {
            Page::Sim => "sim",
            Page::Car => "car",
            Page::Camera => "camera",
            Page::Fog => "fog",
            Page::Sky => "sky",
            Page::Lamps => "lamps",
            Page::CarLights => "car lights",
            Page::LocalLight => "local light",
            Page::World => "world editor",
            Page::Ghost => "ghost",
            Page::Cine => "cine",
            Page::Constants => "constants",
        }
    }

    /// The icon of the page's tab in the strip.
    fn icon(self) -> &'static str {
        match self {
            Page::Sim => ic::CLOCK,
            Page::Car => ic::CAR,
            Page::Camera => ic::CAMERA,
            Page::Fog => ic::FOG,
            Page::Sky => ic::SUN,
            Page::Lamps => ic::BULB,
            Page::CarLights => ic::ZAP,
            Page::LocalLight => ic::SHADOW,
            Page::World => ic::MOUNTAIN,
            Page::Ghost => ic::GHOST,
            Page::Cine => ic::FILM,
            Page::Constants => ic::SLIDERS,
        }
    }

    pub fn parse(s: &str) -> Option<Page> {
        let s = if matches!(s, "fill" | "filleditor" | "world") { "world editor" } else { s };
        Page::ALL.into_iter().find(|p| p.name().replace(' ', "") == s || p.name() == s)
    }
}

#[derive(Default)]
pub struct Tune {
    /// the open page; None = the pane is closed
    pub page: Option<Page>,
    /// the page to come back to (F11)
    last: Option<Page>,
    /// per racer: the parameters as loaded, and the track's gravity (Reset buttons)
    orig: Vec<CarParams>,
    gravity: f32,
    /// set when 'save for track' was pressed: the caller writes `remaster/fog/trackNN.fog` (`fog::save`)
    pub fog_saved: Option<(u32, FogTune)>,
    /// set when the sky page's 'save for track' was pressed: track, yaw, exposure, sun follows; the
    /// caller writes `remaster/sky/sky.cfg` (`sky::save`) and resets `View::remaster.sky_tune`
    pub sky_saved: Option<(u32, f32, f32, bool)>,
    /// set when the car page's paint was edited: car slot, CarColour1..3; the caller reloads the
    /// scene with it (CustomCarColours, `scene::load`) once no mouse button is down
    pub paint_req: Option<(usize, Paint)>,
    /// the dock's property search text
    search: String,
    /// the Constants page (`consts.rs`); its `reload_req` asks the viewer to reload the scene
    pub consts: crate::consts::ConstsPage,
}

pub const CAM_MODES: [(CamMode, &str); 6] = [(CamMode::Chase, "chase"), (CamMode::FarChase, "far chase"), (CamMode::InCar, "in-car"), (CamMode::Bumper, "bumper"), (CamMode::Goal, "goal"), (CamMode::Tv, "tv")];

impl Tune {
    /// A pane open on `page` (shots).
    pub fn shown(page: Page) -> Tune {
        Tune { page: Some(page), ..Default::default() }
    }

    /// The dock's property search text (shots).
    pub fn set_search(&mut self, text: &str) {
        self.search = text.to_string();
    }

    /// F11: close the pane, or open it on the page it showed last.
    pub fn toggle(&mut self) {
        match self.page.take() {
            Some(p) => self.last = Some(p),
            None => self.page = Some(self.last.unwrap_or(Page::Car)),
        }
    }

    /// Remember the loaded values (after a track or car switch).
    pub fn loaded(&mut self, sess: Option<&Session>) {
        self.orig = sess.map(|s| s.race.racers.iter().map(|r| r.car.p.clone()).collect()).unwrap_or_default();
        self.gravity = sess.map_or(0.0, |s| s.race.world.gravity);
    }

    /// The Properties dock (when a page is open) along the right edge: the page rail as its own glass card on the
    /// outer edge, then the dock card (header, property search, the page's sections). Returns both widths in
    /// points. `extra` draws the pages whose state lives in the viewer (`Page::World`, `Page::Ghost`, `Page::Cine`).
    pub fn pane(&mut self, ui: &mut Ui, sess: Option<&mut Session>, t: Targets, extra: &mut dyn FnMut(Page, &mut Ui)) -> f32 {
        let Some(mut page) = self.page else { return 0.0 };
        let ctx = ui.ctx().clone();
        glass_ui::set_filter(&ctx, &self.search);
        glass_ui::set_reset_all(&ctx, false);
        let (mut close, mut reset_page) = (false, false);
        let sub = match (page, sess.as_deref()) {
            (Page::Car, Some(s)) => format!("car #{} · {} · followed", s.focus, s.race.racers[s.focus].car.p.name),
            (Page::Camera, Some(s)) => format!("{} · followed car", CAM_MODES.iter().find(|m| m.0 == s.cam).map_or("?", |m| m.1)),
            (Page::World, _) => t.world_sub.clone(),
            (Page::Constants, _) => self.consts.caption(),
            _ => String::new(),
        };
        // added first = outermost
        let rail = egui::Panel::right("props_tabs")
            .resizable(false)
            .exact_size(RAIL_W)
            .show_separator_line(false)
            .frame(Frame::NONE.inner_margin(egui::Margin { left: 8, right: 12, top: 0, bottom: 10 }))
            .show(ui, |ui| {
                let card = glass_ui::glass_frame(&ctx).inner_margin(egui::Margin::symmetric(10, 8)).show(ui, |ui| {
                    ui.spacing_mut().item_spacing.y = 4.0;
                    for (i, p) in Page::ALL.into_iter().enumerate() {
                        if i == WORLD_AT {
                            glass_ui::rail_sep(ui);
                        }
                        if glass_ui::rail_item(ui, p.icon(), page == p).on_hover_text(p.name()).clicked() {
                            page = p;
                        }
                    }
                });
                glass_ui::finish(&ctx, &card.response);
            })
            .response;
        let rail = rail.rect.width();
        let dock = egui::Panel::right("props")
            .resizable(true)
            .default_size(DOCK_W)
            .size_range(320.0..=760.0)
            .show_separator_line(false)
            .frame(Frame::NONE.inner_margin(egui::Margin { left: 12, right: 0, top: 0, bottom: 10 }))
            .show(ui, |ui| {
                let card = glass_ui::glass_frame(&ctx).inner_margin(egui::Margin::symmetric(10, 10)).show(ui, |ui| {
                    ui.set_min_size(ui.available_size());
                    (reset_page, close) = glass_ui::dock_header(ui, page.icon(), page.name(), &sub);
                    ui.add_space(8.0);
                    glass_ui::search(ui, &mut self.search, "find a property");
                    ui.add_space(4.0);
                    glass_ui::set_reset_all(&ctx, reset_page);
                    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| match page {
                        Page::Sim => sim(ui, sess.as_deref()),
                        Page::Car => match sess {
                            Some(s) => {
                                self.paint(ui, s.focus, t.paint);
                                self.car(ui, s);
                            }
                            None => {
                                ui.weak("no race loaded");
                            }
                        },
                        Page::Camera => match sess {
                            Some(s) => camera(ui, s),
                            None => {
                                ui.weak("no race loaded");
                            }
                        },
                        Page::Fog => {
                            let (track_fog, saved) = (t.track_fog, &mut self.fog_saved);
                            fog(ui, t.view, track_fog, saved);
                        }
                        Page::Sky => sky(ui, t.view, t.track_sky, &mut self.sky_saved),
                        Page::Lamps => lamps(ui, t.view),
                        Page::CarLights => car_lights(ui, &mut t.view.remaster.car_lights),
                        Page::LocalLight => local_light(ui, t.view, t.local_status, t.probe_gizmos),
                        Page::World | Page::Ghost | Page::Cine => extra(page, ui),
                        Page::Constants => self.consts.ui(ui),
                    });
                });
                glass_ui::finish(&ctx, &card.response);
            })
            .response;
        self.page = if close {
            self.last = Some(page);
            None
        } else {
            Some(page)
        };
        dock.rect.width() + rail
    }

    /// The followed car's paint, CarColour1..3 (`paint`: as drawn, its own). An edit goes to
    /// `paint_req` and shows from there until the caller has reloaded the scene.
    fn paint(&mut self, ui: &mut Ui, focus: usize, paint: Option<(Paint, Paint)>) {
        let Some((drawn, own)) = paint else { return };
        let mut c = self.paint_req.filter(|p| p.0 == focus).map_or(drawn, |p| p.1);
        let mut ch = false;
        let reset = section(ui, ic::PALETTE, "Paint", true, |ui| {
            for (k, rgb) in c.iter_mut().enumerate() {
                let modified = *rgb != own[k];
                glass_ui::prow(ui, &format!("Colour {}", k + 1), modified, None, |ui| ch |= glass_ui::swatch_field(ui, rgb));
            }
        });
        if reset {
            (c, ch) = (own, true);
        }
        if ch {
            self.paint_req = Some((focus, c));
        }
    }

    fn car(&self, ui: &mut Ui, s: &mut Session) {
        let focus = s.focus;
        let mut ch = false;
        let tl = s.race.racers[focus].telemetry;
        let g0 = self.gravity;
        let g = &mut s.race.world.gravity;
        let reset_gravity = section(ui, ic::WIND, "World", true, |ui| {
            ch |= num(ui, "Gravity (track)", "m/s²", g, 0.05, Some(g0));
            if glass_ui::filter(ui.ctx()).is_empty() {
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing = egui::vec2(6.0, 6.0);
                    ui.add_space(22.0);
                    glass_ui::chip(ui, &format!("{:.0}", tl.kmh.round() + 0.0), "km/h");
                    glass_ui::chip(ui, &tl.gear.to_string(), "gear");
                    glass_ui::chip(ui, &format!("{:.0}", tl.rpm), "rpm");
                    glass_ui::chip(ui, &tl.contacts.to_string(), "wheels down");
                    if tl.wheelspin || tl.sliding {
                        glass_ui::chip(ui, if tl.wheelspin { "wheelspin" } else { "sliding" }, "");
                    }
                });
            }
        });
        if reset_gravity {
            s.race.world.gravity = self.gravity;
        }
        let r = &mut s.race.racers[focus];
        let before = r.car.p.clone();
        let orig = self.orig.get(focus).cloned();
        let o = orig.as_ref();
        let p = &mut r.car.p;
        let mut reset = false;
        reset |= section(ui, ic::CAR, "Physics", orig.is_some(), |ui| {
            ch |= num(ui, "Mass", "kg", &mut p.mass, 5.0, o.map(|o| o.mass));
            ch |= num(ui, "Acc force", "", &mut p.acc_force, 10.0, o.map(|o| o.acc_force));
            ch |= num(ui, "Brake force", "", &mut p.brake_force, 10.0, o.map(|o| o.brake_force));
            ch |= num(ui, "Turn force", "", &mut p.turn_force, 50.0, o.map(|o| o.turn_force));
            ch |= num(ui, "Min rpm", "rpm", &mut p.min_rpm, 10.0, o.map(|o| o.min_rpm));
            ch |= num(ui, "Max rpm", "rpm", &mut p.max_rpm, 10.0, o.map(|o| o.max_rpm));
            ch |= num(ui, "Spring length", "m", &mut p.spring_len, 0.005, o.map(|o| o.spring_len));
            ch |= num(ui, "Spring strength", "", &mut p.spring_k, 10.0, o.map(|o| o.spring_k));
            ch |= vec3(ui, "Centre of mass", &mut p.com, 0.01, o.map(|o| o.com));
        });
        section(ui, ic::SLIDERS, "Grip", false, |ui| {
            for (i, (g, name)) in p.grip.iter_mut().zip(["Slide speed", "Slide yaw", "Lateral", "Yaw"]).enumerate() {
                ch |= num(ui, name, "", g, 0.005, o.map(|o| o.grip[i]));
            }
        });
        section_open(ui, ic::GAUGE, "Gears", false, false, |ui| {
            for (i, g) in p.gear_top.iter_mut().enumerate() {
                let name = if i == 0 { "Reverse top".to_string() } else { format!("Gear {i} top") };
                ch |= num(ui, &name, "km/h", g, 0.5, o.and_then(|o| o.gear_top.get(i).copied()));
            }
        });
        if let (true, Some(o)) = (reset, orig) {
            r.car.p = o;
            ch = true;
        }
        if ch {
            // keep the edit through the rubber band (AI cars)
            r.rebase(&before);
        }
    }
}

fn sim(ui: &mut Ui, sess: Option<&Session>) {
    match sess {
        Some(s) => ui.label(s.hud()),
        None => ui.weak("no race loaded"),
    };
    ui.weak("time control: toolbar Sim tab");
}

fn camera(ui: &mut Ui, s: &mut Session) {
    section(ui, ic::VIDEO, "View", false, |ui| {
        glass_ui::prow(ui, "View", false, None, |ui| {
            ComboBox::from_id_salt("cam_view").selected_text(CAM_MODES.iter().find(|m| m.0 == s.cam).map_or("?", |m| m.1)).show_ui(ui, |ui| {
                for (m, name) in CAM_MODES {
                    ui.selectable_value(&mut s.cam, m, name);
                }
            });
        });
    });
    // the page edits a copy of the live values and stores a change back: the Constants page and tune.cfg see the same numbers
    let before = CamTune::live();
    let mut t = before;
    let d = CamTune::default();
    let mut reset = false;
    reset |= section(ui, ic::MOVE, "Offsets", true, |ui| {
        vec3(ui, "Chase eye", &mut t.chase_eye, 0.02, Some(d.chase_eye));
        vec3(ui, "Chase look", &mut t.chase_look, 0.02, Some(d.chase_look));
        vec3(ui, "Far eye", &mut t.far_eye, 0.02, Some(d.far_eye));
        vec3(ui, "Far look", &mut t.far_look, 0.02, Some(d.far_look));
        vec3(ui, "In-car eye", &mut t.incar_eye, 0.01, Some(d.incar_eye));
        vec3(ui, "Bumper eye", &mut t.bumper_eye, 0.01, Some(d.bumper_eye));
    });
    reset |= section(ui, ic::SLIDERS, "Easing and walls", true, |ui| {
        num_in(ui, "Easing xz", "", &mut t.k_xz, 0.005, 0.0..=1.0, Some(d.k_xz));
        num_in(ui, "Easing y", "", &mut t.k_y, 0.005, 0.0..=1.0, Some(d.k_y));
        num_in(ui, "Max lag", "rad", &mut t.max_lag, 0.01, 0.0..=std::f32::consts::PI, Some(d.max_lag));
        num_in(ui, "Wall margin", "m", &mut t.margin, 0.01, 0.0..=5.0, Some(d.margin));
        num_in(ui, "Min distance", "m", &mut t.min, 0.01, 0.0..=10.0, Some(d.min));
        num_in(ui, "Ease-out rate", "", &mut t.out_rate, 0.05, 0.0..=50.0, Some(d.out_rate));
    });
    if reset {
        t = d;
    }
    if t != before {
        t.store();
    }
}

fn fog(ui: &mut Ui, v: &mut View, track_fog: Option<(u32, FogTune)>, saved: &mut Option<(u32, FogTune)>) {
    let r = &mut v.remaster;
    ui.horizontal(|ui| {
        ui.label("fog");
        for st in FogStyle::ALL {
            ui.selectable_value(&mut r.fog_style, st, st.name());
        }
    });
    // per track (fog.rs): the track's own values until overridden here; saving writes them for the track
    let own = track_fog.map(|t| t.1).unwrap_or_default();
    let mut over = r.fog_tune.is_some();
    if ui.checkbox(&mut over, "override this track's fog").changed() {
        r.fog_tune = over.then_some(own);
    }
    let mut shown = r.fog_tune.unwrap_or(own);
    let mut ch = false;
    ui.add_enabled_ui(over, |ui| {
        glass_ui::caption(ui, "distance fog (metres; the clip plane is always fully fogged)");
        ch |= glass_ui::slider_row(ui, "start m", &mut shown.start, 0.0..=400.0);
        ch |= glass_ui::slider_row(ui, "full m (reaches opacity)", &mut shown.full, 10.0..=800.0);
        ch |= glass_ui::slider_row(ui, "opacity", &mut shown.opacity, 0.0..=1.0);
        ch |= glass_ui::slider_row(ui, "curve (1 linear, >1 soft start)", &mut shown.curve, 0.5..=3.0);
        glass_ui::caption(ui, "height haze");
        ch |= glass_ui::slider_row(ui, "density (optical depth per 100 m at ground)", &mut shown.density, 0.0..=3.0);
        ch |= glass_ui::slider_row(ui, "height scale m", &mut shown.height, 5.0..=300.0);
        glass_ui::caption(ui, "colour");
        ui.horizontal(|ui| {
            let was = shown.colour;
            ui.selectable_value(&mut shown.colour, FogColour::Sky, "sky horizon");
            ui.selectable_value(&mut shown.colour, FogColour::Track, "track CueColour");
            let mut rgb = matches!(shown.colour, FogColour::Rgb(_));
            if ui.selectable_label(rgb, "custom").clicked() && !rgb {
                shown.colour = FogColour::Rgb([0.5; 3]);
                rgb = true;
            }
            if let (true, FogColour::Rgb(c)) = (rgb, &mut shown.colour) {
                ch |= ui.color_edit_button_rgb(c).changed();
            }
            ch |= shown.colour != was;
        });
        ch |= glass_ui::slider_row(ui, "brightness", &mut shown.brightness, 0.0..=3.0);
        ui.horizontal(|ui| {
            ui.label("tint");
            ch |= ui.color_edit_button_rgb(&mut shown.tint).changed();
            if ui.small_button("white").clicked() {
                shown.tint = [1.0; 3];
                ch = true;
            }
        });
        glass_ui::caption(ui, "sun and sky");
        ch |= glass_ui::slider_row(ui, "sun glow", &mut shown.glow, 0.0..=2.0);
        ch |= glass_ui::slider_row(ui, "glow anisotropy (HG g)", &mut shown.anisotropy, 0.0..=0.95);
        ch |= glass_ui::slider_row(ui, "sky haze (how far up the sky)", &mut shown.sky_haze, 0.0..=1.0);
        ch |= glass_ui::slider_row(ui, "lamp scatter (volumetric)", &mut shown.lamps, 0.0..=40.0);
        ch |= glass_ui::slider_row(ui, "sun shafts (haze darkening in shadow, volumetric)", &mut shown.shafts, 0.0..=1.0);
    });
    if ch {
        r.fog_tune = Some(shown);
    }
    ui.horizontal(|ui| {
        if ui.button("reset to track").clicked() {
            r.fog_tune = None;
        }
        if let Some((n, _)) = track_fog {
            if ui.add_enabled(over, egui::Button::new(format!("save for track {n}"))).clicked() {
                *saved = Some((n, shown));
            }
        }
    });
}

/// The F11 values add to the track's sky.cfg line (`sky::Dome`); saving writes the sum as the new line
/// and the caller then resets the F11 values, so the picture stays the same.
fn sky(ui: &mut Ui, v: &mut View, track_sky: Option<(u32, crate::sky::Dome)>, saved: &mut Option<(u32, f32, f32, bool)>) {
    let r = &mut v.remaster;
    ui.checkbox(&mut r.sky_hdri, "HDRI sky (needs remaster/sky/trackNN.exr)");
    let t = &mut r.sky_tune;
    let reset = section(ui, ic::SUN, "Sky tuning", true, |ui| {
        glass_ui::slider_row(ui, "exposure (stops)", &mut t.exposure, -6.0..=4.0);
        glass_ui::slider_row(ui, "turn °", &mut t.yaw, -180.0..=180.0);
        ui.checkbox(&mut t.sun, "sun of cars and shadows follows the HDRI's sun");
        ui.checkbox(&mut t.ambient, "ambient light of cars and props from the HDRI (SH)");
        glass_ui::slider_row(ui, "ambient tint", &mut t.ambient_tint, 0.0..=1.0);
    });
    if reset {
        *t = Default::default();
    }
    ui.horizontal(|ui| {
        let n = track_sky.map(|s| s.0);
        let label = n.map_or("save for track".to_string(), |n| format!("save for track {n}"));
        if ui.add_enabled(n.is_some(), egui::Button::new(label)).on_hover_text("write the sum of these values and the track's sky.cfg line to remaster/sky/sky.cfg").clicked() {
            if let Some((n, d)) = track_sky {
                *saved = Some((n, t.yaw + d.yaw, t.exposure - SkyTune::default().exposure + d.exposure, t.sun && d.follow_sun));
            }
        }
        if track_sky.is_none() {
            ui.weak("this track has no HDRI");
        }
    });
}

/// The light shadows' tuning (modes and quality: ribbon Remaster > Shadows).
fn lamps(ui: &mut Ui, v: &mut View) {
    let r = &mut v.remaster;
    let l = &mut r.lamp_lights;
    let reset = section(ui, ic::BULB, "Light shadows", true, |ui| {
        glass_ui::slider_row(ui, "gain", &mut l.gain, 0.0..=6.0);
        glass_ui::slider_row(ui, "range ×", &mut l.range, 0.3..=3.0);
        glass_ui::slider_row(ui, "cone inner °", &mut l.inner, 0.0..=80.0);
        glass_ui::slider_row(ui, "cone outer °", &mut l.outer, 10.0..=89.0);
        glass_ui::slider_row(ui, "shadow on world", &mut l.shadow, 0.0..=3.0);
        glass_ui::slider_row(ui, "shadow bias m", &mut l.bias, 0.0..=0.5);
        glass_ui::slider_row(ui, "shadow distance m", &mut l.distance, 20.0..=500.0);
        glass_ui::slider_row(ui, "shadow reach ×", &mut l.reach, 1.0..=8.0);
        glass_ui::slider_row(ui, "shadow cone °", &mut l.shadow_cone, 30.0..=84.0);
        glass_ui::slider_row(ui, "headlight shadow reach m", &mut l.head_reach, 20.0..=140.0);
        glass_ui::slider_row(ui, "headlight map half width °", &mut l.head_fov_x, 20.0..=80.0);
        glass_ui::slider_row(ui, "headlight map half height °", &mut l.head_fov_y, 10.0..=60.0);
    });
    if reset {
        *l = Default::default();
    }
}

/// REMASTER local light (#83): the option, the probes' strength and the probe gizmos.
fn local_light(ui: &mut Ui, v: &mut View, status: &str, gizmos: Option<&mut bool>) {
    ui.checkbox(&mut v.remaster.local_light, "local light (Detail row; Remaster only)");
    ui.weak(format!("probe volume: {status}"));
    let t = &mut v.remaster.local_tune;
    let reset = section(ui, ic::SHADOW, "Strength", true, |ui| {
        glass_ui::slider_row(ui, "sky occlusion", &mut t.occlusion, 0.0..=1.0);
        glass_ui::slider_row(ui, "bounce gain", &mut t.bounce, 0.0..=4.0);
    });
    if reset {
        *t = LocalLightTune::default();
    }
    if let Some(g) = gizmos {
        ui.checkbox(g, "probe gizmos (an upward face's ambient per probe; magenta = inside geometry)");
    }
}

fn car_lights(ui: &mut Ui, c: &mut CarLightTune) {
    glass_ui::slider_row(ui, "brake preview", &mut c.brake_preview, 0.0..=1.0);
    for (name, l) in [("head lights", &mut c.head), ("tail lights", &mut c.tail)] {
        section(ui, ic::BULB, name, false, |ui| lamp(ui, l));
    }
    if ui.button("reset car lights").clicked() {
        *c = CarLightTune { brake_preview: c.brake_preview, ..Default::default() };
    }
}

fn lamp(ui: &mut Ui, l: &mut LampTune) {
    let rows: [(&str, &mut f32, f32); 20] = [
        ("halo", &mut l.halo, 0.01),
        ("halo night", &mut l.night, 0.01),
        ("glare", &mut l.glare, 0.05),
        ("glare size", &mut l.glare_size, 0.01),
        ("size", &mut l.size, 0.01),
        ("burst size", &mut l.burst_size, 0.01),
        ("burst gain", &mut l.burst_gain, 0.01),
        ("scale", &mut l.scale, 0.01),
        ("spot", &mut l.spot, 0.02),
        ("flare", &mut l.flare, 0.01),
        ("flare day", &mut l.flare_day, 0.01),
        ("flare night", &mut l.flare_night, 0.01),
        ("brake: halo x", &mut l.brake_halo, 0.02),
        ("brake: size", &mut l.brake_size, 0.01),
        ("brake: burst size", &mut l.brake_burst_size, 0.01),
        ("brake: burst gain", &mut l.brake_burst_gain, 0.01),
        ("brake: spot +", &mut l.brake_spot, 0.02),
        ("brake: flare +", &mut l.brake_flare, 0.01),
        ("strip gain", &mut l.strip, 0.01),
        ("strip width", &mut l.strip_width, 0.01),
    ];
    for (name, v, speed) in rows {
        num_in(ui, &glass_ui::cap_first(name), "", v, speed, 0.0..=50.0, None);
    }
}

/// A property row with a number field (`unit` shown after it); `orig` = the value in the file: the row gets the
/// modified dot when it differs. True when it changed.
fn num(ui: &mut Ui, name: &str, unit: &str, v: &mut f32, speed: f32, orig: Option<f32>) -> bool {
    let modified = orig.is_some_and(|o| o != *v);
    let mut ch = false;
    glass_ui::prow(ui, name, modified, Some(unit), |ui| ch = glass_ui::num_field(ui, v, speed, None, modified).changed());
    ch
}

/// `num` with a range: the field's range bar shows where the value sits in it.
fn num_in(ui: &mut Ui, name: &str, unit: &str, v: &mut f32, speed: f32, range: std::ops::RangeInclusive<f32>, orig: Option<f32>) -> bool {
    let modified = orig.is_some_and(|o| o != *v);
    let mut ch = false;
    glass_ui::prow(ui, name, modified, Some(unit), |ui| ch = glass_ui::num_field(ui, v, speed, Some(range), modified).changed());
    ch
}

/// A property row with a vector (game axes); true when it changed.
fn vec3(ui: &mut Ui, name: &str, v: &mut Vec3, speed: f32, orig: Option<Vec3>) -> bool {
    let modified = orig.is_some_and(|o| o != *v);
    let mut ch = false;
    // 3 fields of 76, 2 gaps of 4 and the unit column
    let wide = ui.painter().layout_no_wrap(name.to_string(), egui::FontId::proportional(14.0), egui::Color32::WHITE).size().x > glass_ui::label_room(ui, 3.0 * 76.0 + 8.0);
    if wide {
        // the label takes its own line, the fields the next
        glass_ui::prow(ui, name, modified, None, |_| {});
        glass_ui::prow_cont(ui, None, |ui| ch = glass_ui::vec3_field(ui, v, speed, orig));
    } else {
        glass_ui::prow(ui, name, modified, None, |ui| ch = glass_ui::vec3_field(ui, v, speed, orig));
    }
    ch
}

/// A collapsible section (`.sect`): glass header with chevron, icon and dashed rule, `reset` adds a reset button at
/// its right end (true when it was clicked, or the page header's reset was). While the property search has text
/// every section is open and its rows are filtered.
pub(crate) fn section(ui: &mut Ui, icon: &str, title: &str, reset: bool, body: impl FnOnce(&mut Ui)) -> bool {
    section_open(ui, icon, title, reset, true, body)
}

/// `section` with its first state: `open` false = shut until the user opens it.
pub(crate) fn section_open(ui: &mut Ui, icon: &str, title: &str, reset: bool, open: bool, body: impl FnOnce(&mut Ui)) -> bool {
    section_counted(ui, icon, title, None, reset, open, body)
}

/// `section_open` with a count after the name (the outliner's piece count).
#[allow(clippy::too_many_arguments)]
pub(crate) fn section_counted(ui: &mut Ui, icon: &str, title: &str, count: Option<&str>, reset: bool, open: bool, body: impl FnOnce(&mut Ui)) -> bool {
    let ctx = ui.ctx().clone();
    let id = ui.make_persistent_id(("glass section", title));
    let mut open = ctx.data_mut(|d| *d.get_temp_mut_or_insert_with(id, || open));
    let filtering = !glass_ui::filter(&ctx).is_empty();
    // the rows the search left in this section last pass: none = the section is not shown (its body still runs,
    // unseen, to count them again)
    let rows_id = id.with("rows");
    let hide = filtering && ctx.data(|d| d.get_temp::<usize>(rows_id)).is_some_and(|n| n == 0);
    glass_ui::section_enter(&ctx, title);
    glass_ui::rows_begin(&ctx);
    let mut clicked = false;
    if hide {
        let mut ghost = ui.new_child(egui::UiBuilder::new().max_rect(ui.available_rect_before_wrap()).invisible());
        body(&mut ghost);
    } else {
        ui.add_space(4.0);
        clicked = glass_ui::section_header(ui, icon, title, count, &mut open, reset);
        ctx.data_mut(|d| d.insert_temp(id, open));
        if open || filtering {
            body(ui);
        }
    }
    // a section whose own name matches the search stays even without a row of it
    let n = glass_ui::rows_drawn(&ctx) + usize::from(glass_ui::section_matched(&ctx));
    ctx.data_mut(|d| d.insert_temp(rows_id, n));
    clicked |= reset && glass_ui::reset_all(&ctx);
    clicked
}
