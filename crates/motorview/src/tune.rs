//! DEBUG motorview's Properties pane (` or F11; re-notes/00-debuger.md item 6): the long grids
//! the ribbon (`ribbon.rs`) has no room for. Live editing of the followed car's physics
//! parameters, the camera offsets and easing, the fog, sky and lamp tuning. Edits last until
//! the track or car is reloaded; nothing is saved.

use crate::drive::{CamMode, CamTune, Session};
use crate::render::{CarLightTune, FogColour, FogStyle, FogTune, LampTune, LocalLightTune, SkyTune, View};
use crate::icons as ic;
use crate::theme;
use crate::gui::{wheel_drag, wheel_slider};
use egui::{ComboBox, Grid, Ui};
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
}

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
}

impl Page {
    pub const ALL: [Page; 11] = [Page::Sim, Page::Car, Page::Camera, Page::Fog, Page::Sky, Page::Lamps, Page::CarLights, Page::LocalLight, Page::World, Page::Ghost, Page::Cine];

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
        }
    }

    pub fn parse(s: &str) -> Option<Page> {
        let s = if s == "fill" || s == "filleditor" { "world" } else { s };
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
}

pub const CAM_MODES: [(CamMode, &str); 6] = [(CamMode::Chase, "chase"), (CamMode::FarChase, "far chase"), (CamMode::InCar, "in-car"), (CamMode::Bumper, "bumper"), (CamMode::Goal, "goal"), (CamMode::Tv, "tv")];

impl Tune {
    /// A pane open on `page` (shots).
    pub fn shown(page: Page) -> Tune {
        Tune { page: Some(page), ..Default::default() }
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

    /// The Properties dock (when a page is open) along the right edge, Blender style: the page
    /// icons in a strip on the outer edge, the page's name as the header. Returns its width in
    /// points. `extra` draws the pages whose state lives in the viewer (`Page::World`, `Page::Ghost`, `Page::Cine`).
    pub fn pane(&mut self, ui: &mut Ui, sess: Option<&mut Session>, t: Targets, extra: &mut dyn FnMut(Page, &mut Ui)) -> f32 {
        let Some(mut page) = self.page else { return 0.0 };
        let mut close = false;
        // added first = outermost
        let strip = egui::Panel::right("props_tabs")
            .resizable(false)
            .exact_size(34.0)
            .frame(egui::Frame::new().fill(theme::PANEL_DARK).inner_margin(egui::Margin::symmetric(3, 4)))
            .show(ui, |ui| {
                ui.vertical_centered(|ui| {
                    for p in Page::ALL {
                        if ui.add_sized([28.0, 28.0], egui::Button::selectable(page == p, p.icon())).on_hover_text(p.name()).clicked() {
                            page = p;
                        }
                    }
                    ui.add_space(6.0);
                    close = ui.add_sized([28.0, 28.0], egui::Button::new(ic::CLOSE)).on_hover_text("close (F11)").clicked();
                });
            })
            .response
            .rect
            .width();
        let w = egui::Panel::right("props")
            .resizable(true)
            .default_size(330.0)
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new(format!("{}  {}", page.icon(), page.name())).strong().size(15.0));
                });
                ui.separator();
                egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| match page {
                    Page::Sim => sim(ui, sess.as_deref()),
                    Page::Car => match sess {
                        Some(s) => {
                            ui.weak(format!("car #{}: {}", s.focus, s.race.racers[s.focus].car.p.name));
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
                });
            })
            .response
            .rect
            .width();
        self.page = if close {
            self.last = Some(page);
            None
        } else {
            Some(page)
        };
        w + strip
    }

    /// The followed car's paint, CarColour1..3 (`paint`: as drawn, its own). An edit goes to
    /// `paint_req` and shows from there until the caller has reloaded the scene.
    fn paint(&mut self, ui: &mut Ui, focus: usize, paint: Option<(Paint, Paint)>) {
        let Some((drawn, own)) = paint else { return };
        let mut c = self.paint_req.filter(|p| p.0 == focus).map_or(drawn, |p| p.1);
        let mut ch = false;
        let reset = section(ui, ic::PALETTE, "Paint", true, |ui| {
            Grid::new("paint").num_columns(2).show(ui, |ui| {
                for (k, rgb) in c.iter_mut().enumerate() {
                    ui.label(format!("colour {}", k + 1));
                    ch |= egui::color_picker::color_edit_button_srgb(ui, rgb).changed();
                    ui.end_row();
                }
            });
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
        let mut reset_gravity = false;
        let w = &mut s.race.world;
        reset_gravity |= section(ui, ic::WIND, "World", true, |ui| {
            Grid::new("world").num_columns(2).show(ui, |ui| {
                ch |= drag(ui, "gravity (track)", &mut w.gravity, 0.05);
            });
        });
        if reset_gravity {
            s.race.world.gravity = self.gravity;
        }
        let r = &mut s.race.racers[focus];
        let tl = r.telemetry;
        ui.label(format!("{:.0} km/h  gear {}  {:.0} rpm  {} wheels down{}", tl.kmh, tl.gear, tl.rpm, tl.contacts, if tl.wheelspin { "  wheelspin" } else if tl.sliding { "  sliding" } else { "" }));
        let before = r.car.p.clone();
        let orig = self.orig.get(focus).cloned();
        let p = &mut r.car.p;
        let mut reset = false;
        reset |= section(ui, ic::CAR, "Physics", orig.is_some(), |ui| {
            Grid::new("car").num_columns(2).striped(true).show(ui, |ui| {
                ch |= drag(ui, "mass", &mut p.mass, 5.0);
                ch |= drag(ui, "acc force", &mut p.acc_force, 10.0);
                ch |= drag(ui, "brake force", &mut p.brake_force, 10.0);
                ch |= drag(ui, "turn force", &mut p.turn_force, 50.0);
                ch |= drag(ui, "min rpm", &mut p.min_rpm, 10.0);
                ch |= drag(ui, "max rpm", &mut p.max_rpm, 10.0);
                ch |= drag(ui, "spring length", &mut p.spring_len, 0.005);
                ch |= drag(ui, "spring strength", &mut p.spring_k, 10.0);
                ch |= vec3(ui, "centre of mass", &mut p.com, 0.01);
            });
        });
        section(ui, ic::SLIDERS, "Grip", false, |ui| {
            Grid::new("grip").num_columns(2).striped(true).show(ui, |ui| {
                for (g, name) in p.grip.iter_mut().zip(["slide speed", "slide yaw", "lateral", "yaw"]) {
                    ch |= drag(ui, name, g, 0.005);
                }
            });
        });
        section(ui, ic::GAUGE, "Gears", false, |ui| {
            Grid::new("gears").num_columns(2).striped(true).show(ui, |ui| {
                for (i, g) in p.gear_top.iter_mut().enumerate() {
                    let name = if i == 0 { "reverse top km/h".to_string() } else { format!("gear {i} top km/h") };
                    ch |= drag(ui, &name, g, 0.5);
                }
            });
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
    ui.weak("time control: ribbon Sim tab");
}

fn camera(ui: &mut Ui, s: &mut Session) {
    section(ui, ic::VIDEO, "View", false, |ui| {
        ComboBox::from_label("view").selected_text(CAM_MODES.iter().find(|m| m.0 == s.cam).map_or("?", |m| m.1)).show_ui(ui, |ui| {
            for (m, name) in CAM_MODES {
                ui.selectable_value(&mut s.cam, m, name);
            }
        });
    });
    let t = &mut s.tune;
    let mut reset = false;
    reset |= section(ui, ic::MOVE, "Offsets", true, |ui| {
        Grid::new("cam").num_columns(2).striped(true).show(ui, |ui| {
            vec3(ui, "chase eye", &mut t.chase_eye, 0.02);
            vec3(ui, "chase look", &mut t.chase_look, 0.02);
            vec3(ui, "far eye", &mut t.far_eye, 0.02);
            vec3(ui, "far look", &mut t.far_look, 0.02);
            vec3(ui, "in-car eye", &mut t.incar_eye, 0.01);
            vec3(ui, "bumper eye", &mut t.bumper_eye, 0.01);
        });
    });
    reset |= section(ui, ic::SLIDERS, "Easing and walls", true, |ui| {
        Grid::new("cam_ease").num_columns(2).striped(true).show(ui, |ui| {
            drag_in(ui, "easing xz", &mut t.k_xz, 0.005, 0.0..=1.0);
            drag_in(ui, "easing y", &mut t.k_y, 0.005, 0.0..=1.0);
            drag_in(ui, "max lag (rad)", &mut t.max_lag, 0.01, 0.0..=std::f32::consts::PI);
            drag_in(ui, "wall margin", &mut t.margin, 0.01, 0.0..=5.0);
            drag_in(ui, "min distance", &mut t.min, 0.01, 0.0..=10.0);
            drag_in(ui, "ease-out rate", &mut t.out_rate, 0.05, 0.0..=50.0);
        });
    });
    if reset {
        *t = CamTune::default();
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
        ui.label("distance fog (metres; the clip plane is always fully fogged)");
        ch |= wheel_slider(ui, &mut shown.start, 0.0..=400.0, |s| s.text("start m")).changed();
        ch |= wheel_slider(ui, &mut shown.full, 10.0..=800.0, |s| s.text("full m (reaches opacity)")).changed();
        ch |= wheel_slider(ui, &mut shown.opacity, 0.0..=1.0, |s| s.text("opacity")).changed();
        ch |= wheel_slider(ui, &mut shown.curve, 0.5..=3.0, |s| s.text("curve (1 linear, >1 soft start)")).changed();
        ui.label("height haze");
        ch |= wheel_slider(ui, &mut shown.density, 0.0..=3.0, |s| s.text("density (optical depth per 100 m at ground)")).changed();
        ch |= wheel_slider(ui, &mut shown.height, 5.0..=300.0, |s| s.text("height scale m")).changed();
        ui.label("colour");
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
        ch |= wheel_slider(ui, &mut shown.brightness, 0.0..=3.0, |s| s.text("brightness")).changed();
        ui.horizontal(|ui| {
            ui.label("tint");
            ch |= ui.color_edit_button_rgb(&mut shown.tint).changed();
            if ui.small_button("white").clicked() {
                shown.tint = [1.0; 3];
                ch = true;
            }
        });
        ui.label("sun and sky");
        ch |= wheel_slider(ui, &mut shown.glow, 0.0..=2.0, |s| s.text("sun glow")).changed();
        ch |= wheel_slider(ui, &mut shown.anisotropy, 0.0..=0.95, |s| s.text("glow anisotropy (HG g)")).changed();
        ch |= wheel_slider(ui, &mut shown.sky_haze, 0.0..=1.0, |s| s.text("sky haze (how far up the sky)")).changed();
        ch |= wheel_slider(ui, &mut shown.lamps, 0.0..=40.0, |s| s.text("lamp scatter (volumetric)")).changed();
        ch |= wheel_slider(ui, &mut shown.shafts, 0.0..=1.0, |s| s.text("sun shafts: haze darkening in shadow (volumetric)")).changed();
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
        wheel_slider(ui, &mut t.exposure, -6.0..=4.0, |s| s.text("exposure (stops)"));
        wheel_slider(ui, &mut t.yaw, -180.0..=180.0, |s| s.text("turn °"));
        ui.checkbox(&mut t.sun, "sun of cars and shadows follows the HDRI's sun");
        ui.checkbox(&mut t.ambient, "ambient light of cars and props from the HDRI (SH)");
        wheel_slider(ui, &mut t.ambient_tint, 0.0..=1.0, |s| s.text("ambient tint"));
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
        wheel_slider(ui, &mut l.gain, 0.0..=6.0, |s| s.text("gain"));
        wheel_slider(ui, &mut l.range, 0.3..=3.0, |s| s.text("range ×"));
        wheel_slider(ui, &mut l.inner, 0.0..=80.0, |s| s.text("cone inner °"));
        wheel_slider(ui, &mut l.outer, 10.0..=89.0, |s| s.text("cone outer °"));
        wheel_slider(ui, &mut l.shadow, 0.0..=3.0, |s| s.text("shadow on world"));
        wheel_slider(ui, &mut l.bias, 0.0..=0.5, |s| s.text("shadow bias m"));
        wheel_slider(ui, &mut l.distance, 20.0..=500.0, |s| s.text("shadow distance m"));
        wheel_slider(ui, &mut l.reach, 1.0..=8.0, |s| s.text("shadow reach ×"));
        wheel_slider(ui, &mut l.shadow_cone, 30.0..=84.0, |s| s.text("shadow cone °"));
        wheel_slider(ui, &mut l.head_reach, 20.0..=140.0, |s| s.text("headlight shadow reach m"));
        wheel_slider(ui, &mut l.head_fov_x, 20.0..=80.0, |s| s.text("headlight map half width °"));
        wheel_slider(ui, &mut l.head_fov_y, 10.0..=60.0, |s| s.text("headlight map half height °"));
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
        wheel_slider(ui, &mut t.occlusion, 0.0..=1.0, |s| s.text("sky occlusion"));
        wheel_slider(ui, &mut t.bounce, 0.0..=4.0, |s| s.text("bounce gain"));
    });
    if reset {
        *t = LocalLightTune::default();
    }
    if let Some(g) = gizmos {
        ui.checkbox(g, "probe gizmos (an upward face's ambient per probe; magenta = inside geometry)");
    }
}

fn car_lights(ui: &mut Ui, c: &mut CarLightTune) {
    wheel_slider(ui, &mut c.brake_preview, 0.0..=1.0, |s| s.text("brake preview"));
    for (name, l) in [("head lights", &mut c.head), ("tail lights", &mut c.tail)] {
        section(ui, ic::BULB, name, false, |ui| {
            Grid::new(name).num_columns(2).striped(true).show(ui, |ui| lamp(ui, l));
        });
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
        drag_in(ui, name, v, speed, 0.0..=50.0);
    }
}

/// A labelled number in a two-column grid; true when it changed.
fn drag(ui: &mut Ui, name: &str, v: &mut f32, speed: f32) -> bool {
    ui.label(name);
    let ch = wheel_drag(ui, v, None, |d| d.speed(speed)).changed();
    ui.end_row();
    ch
}

fn drag_in(ui: &mut Ui, name: &str, v: &mut f32, speed: f32, range: std::ops::RangeInclusive<f32>) -> bool {
    ui.label(name);
    let ch = wheel_drag(ui, v, Some(range), |d| d.speed(speed)).changed();
    ui.end_row();
    ch
}

/// A labelled vector (game axes) in a two-column grid; true when it changed.
fn vec3(ui: &mut Ui, name: &str, v: &mut Vec3, speed: f32) -> bool {
    ui.label(name);
    let ch = ui
        .horizontal(|ui| {
            let mut ch = false;
            for c in [&mut v.x, &mut v.y, &mut v.z] {
                ch |= wheel_drag(ui, c, None, |d| d.speed(speed).max_decimals(3)).changed();
            }
            ch
        })
        .inner;
    ui.end_row();
    ch
}

/// A collapsing section with an icon; `reset` adds a reset button at the header's right end
/// (true when it was clicked).
pub(crate) fn section(ui: &mut Ui, icon: &str, title: &str, reset: bool, body: impl FnOnce(&mut Ui)) -> bool {
    let id = ui.make_persistent_id(title);
    let mut clicked = false;
    egui::collapsing_header::CollapsingState::load_with_default_open(ui.ctx(), id, true)
        .show_header(ui, |ui| {
            ui.label(egui::RichText::new(format!("{icon}  {title}")).strong());
            if reset {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    clicked = ui.small_button(ic::RESET).on_hover_text("reset this section").clicked();
                });
            }
        })
        .body(body);
    clicked
}
