//! DEBUG motorview's ribbon (re-notes/00-debuger.md, look: 15-motorview-overhaul.md): every viewer
//! option, by tab and group, over the Properties pane (`motorview::tune`). An item with a key runs
//! the same `Act` as the key (its shortcut is in the hover text, from `BINDS`); values are edited
//! in place. A group's launcher opens its long grids in the pane. Items are Lucide icon + label
//! (`icons.rs`), the main actions of a group are large icon-over-caption tools. The game does not use it.

use super::{key_name, Act, Ctx, DebugState, BINDS, HELD_FREE, HELD_RACE, TIME_SCALES};
use egui::{Align2, Button, ComboBox, FontId, Response, RichText, Sense, Stroke, StrokeKind, Ui, Vec2};
use motorview::drive::{CamMode, PoseCam, Session};
use motorview::gui::{wheel_drag, wheel_slider};
use motorview::cine_edit::{self, CineEdit, Preview, Req};
use motorview::world_edit::{kind_icon, Cmd, Kind, WorldEdit};
use motorview::ghost::{runs, Follow, Sync, ViewerGhost};
use motorview::icons as ic;
use motorview::inspect::{Passes, Tab};
use motorview::render::{aa_stop, CarShading, DebugShade, GrassLevel, LightShadows, PostAaKind, RemasterView, RenderAaKind, RenderMode, ShadowQuality, Tonemap, View, AA_STOPS, AA_STOP_STRENGTH};
use motorview::theme;
use motorview::wire::WireStyle;
use motorview::tune::{Page, Tune, CAM_MODES};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RTab {
    View,
    Sim,
    Render,
    Remaster,
    Debug,
    World,
    Ghost,
    Cine,
    Help,
}

impl RTab {
    pub const ALL: [RTab; 9] = [RTab::View, RTab::Sim, RTab::Render, RTab::Remaster, RTab::Debug, RTab::World, RTab::Ghost, RTab::Cine, RTab::Help];

    pub fn name(self) -> &'static str {
        match self {
            RTab::View => "View",
            RTab::Sim => "Sim",
            RTab::Render => "Render",
            RTab::Remaster => "Remaster",
            RTab::Debug => "Debug",
            RTab::World => "World",
            RTab::Ghost => "Ghost",
            RTab::Cine => "Cine",
            RTab::Help => "Help",
        }
    }

    fn icon(self) -> &'static str {
        match self {
            RTab::View => ic::EYE,
            RTab::Sim => ic::PLAY,
            RTab::Render => ic::MONITOR,
            RTab::Remaster => ic::SPARKLES,
            RTab::Debug => ic::BUG,
            RTab::World => ic::MOUNTAIN,
            RTab::Ghost => ic::GHOST,
            RTab::Cine => ic::FILM,
            RTab::Help => ic::HELP,
        }
    }

    pub fn parse(s: &str) -> Option<RTab> {
        let s = if s.eq_ignore_ascii_case("fill") { "world" } else { s };
        RTab::ALL.into_iter().find(|t| t.name().eq_ignore_ascii_case(s))
    }
}

/// The ribbon's own state (kept by the viewer between frames).
pub struct RibbonState {
    pub tab: RTab,
    /// only the tab strip shows (double-click a tab)
    pub minimised: bool,
    /// Help > Shortcuts search text
    pub filter: String,
    /// the backdrop colour the picker starts from (gamma space)
    pub backdrop: [u8; 3],
}

impl Default for RibbonState {
    fn default() -> Self {
        RibbonState { tab: RTab::View, minimised: false, filter: String::new(), backdrop: [128; 3] }
    }
}

/// What the ribbon shows and edits.
pub struct Ribbon<'a> {
    pub st: &'a mut RibbonState,
    pub tune: &'a mut Tune,
    pub view: &'a mut View,
    pub dbg: &'a mut DebugState,
    pub sess: Option<&'a mut Session>,
    pub free: bool,
    /// free camera: the arrows drive the followed car (`Act::DriveFree`)
    pub drive_free: bool,
    pub overlay: bool,
    pub cross: bool,
    /// F4 wire style (the combo edits it)
    pub wireframe: &'a mut WireStyle,
    pub mirror: bool,
    pub track: u32,
    pub car: u32,
    /// the followed car drawn with the other model (`Act::SwapMesh`)
    pub mesh_swap: bool,
    /// world editor open
    pub world: Option<&'a mut WorldEdit>,
    /// the loaded ghost, and the ghost window's camera when it is open
    pub ghost: Option<&'a mut ViewerGhost>,
    pub ghost_win: Option<CamMode>,
    /// the camera a ghost window opens with
    pub ghost_cam: CamMode,
    /// what a new ghost window follows, and the window's (when open)
    pub ghost_follow: Follow,
    /// set to -1 / 1 while the Rewind / Forward button is held
    pub scrub: &'a mut i8,
    /// set to a timeline second when the timeline slider is moved (#60)
    pub seek: &'a mut Option<f64>,
    /// the Cine editor (#152)
    pub cine: Option<&'a mut CineEdit>,
    /// clicked items and their choice, run after the pass (`App::act`: a track switch needs
    /// the GPU, which the frame holds)
    pub acts: Vec<(Act, usize)>,
}

/// Width of a slider in a group.
const SLIDER_W: f32 = 80.0;
/// Height of a large tool (icon over caption).
const TOOL_H: f32 = 52.0;

/// m:ss.s of a number of seconds.
pub fn clock(secs: f64) -> String {
    format!("{}:{:04.1}", (secs / 60.0) as u32, secs % 60.0)
}

impl Ribbon<'_> {
    /// The ribbon along the top edge; its height in points.
    pub fn bar(&mut self, ui: &mut Ui) -> f32 {
        egui::Panel::top("ribbon")
            .frame(egui::Frame::new().fill(theme::PANEL).inner_margin(egui::Margin::symmetric(6, 4)))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    for t in RTab::ALL {
                        let on = self.st.tab == t && !self.st.minimised;
                        let r = ui.add(Button::selectable(on, format!("{}  {}", t.icon(), t.name())));
                        if r.double_clicked() {
                            self.st.minimised ^= true;
                        } else if r.clicked() {
                            (self.st.tab, self.st.minimised) = (t, false);
                        }
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let remaster = self.view.mode == RenderMode::Remaster;
                        let (icon, name) = if remaster { (ic::SPARKLES, "Remaster") } else { (ic::MONITOR, "Classic") };
                        self.toggle(ui, Act::Mode, icon, name, remaster);
                        ui.weak(format!("{}  track {}   {}  car {}", ic::ROUTE, self.track, ic::CAR, self.car));
                    });
                });
                if self.st.minimised {
                    return;
                }
                ui.add_space(2.0);
                egui::ScrollArea::horizontal().auto_shrink([false, true]).show(ui, |ui| {
                    ui.horizontal_top(|ui| match self.st.tab {
                        RTab::View => self.view_tab(ui),
                        RTab::Sim => self.sim_tab(ui),
                        RTab::Render => self.render_tab(ui),
                        RTab::Remaster => self.remaster_tab(ui),
                        RTab::Debug => self.debug_tab(ui),
                        RTab::World => self.world_tab(ui),
                        RTab::Ghost => self.ghost_tab(ui),
                        RTab::Cine => self.cine_tab(ui),
                        RTab::Help => self.help_tab(ui),
                    });
                });
            })
            .response
            .rect
            .height()
    }

    /// A framed group: controls wrapped at `width`, the caption centred under them, the launcher
    /// (opens the group's page in the pane) in the caption's corner.
    fn group(&mut self, ui: &mut Ui, title: &str, launch: Option<Page>, width: f32, body: impl FnOnce(&mut Self, &mut Ui)) {
        self.framed(ui, title, launch, Some(width), body);
    }

    /// A framed group as wide as its body: the body lays out its own columns (`ui.vertical`
    /// stacks), nothing wraps. Used where a fixed width would clip or waste the row.
    fn group_auto(&mut self, ui: &mut Ui, title: &str, launch: Option<Page>, body: impl FnOnce(&mut Self, &mut Ui)) {
        self.framed(ui, title, launch, None, body);
    }

    fn framed(&mut self, ui: &mut Ui, title: &str, launch: Option<Page>, width: Option<f32>, body: impl FnOnce(&mut Self, &mut Ui)) {
        egui::Frame::new().fill(theme::PANEL_DARK).stroke(Stroke::new(1.0, theme::STROKE)).corner_radius(4).inner_margin(egui::Margin::symmetric(6, 4)).show(ui, |ui| {
            ui.vertical(|ui| {
                ui.set_min_height(TOOL_H + 22.0);
                let width = match width {
                    Some(w) => {
                        ui.set_width(w);
                        ui.horizontal_wrapped(|ui| body(self, ui));
                        w
                    }
                    None => {
                        ui.set_min_width(54.0);
                        ui.horizontal_top(|ui| body(self, ui));
                        ui.min_rect().width()
                    }
                };
                ui.add_space(2.0);
                let (row, _) = ui.allocate_exact_size(Vec2::new(width, 16.0), Sense::hover());
                ui.painter().text(row.center(), Align2::CENTER_CENTER, title, FontId::proportional(11.0), theme::TEXT_WEAK);
                if let Some(p) = launch {
                    let hit = egui::Rect::from_min_size(row.right_top() - Vec2::new(16.0, 0.0), Vec2::splat(16.0));
                    let r = ui.put(hit, Button::new(RichText::new(ic::LAUNCH).size(10.0)).frame(false)).on_hover_text(format!("{} in the properties pane", p.name()));
                    if r.clicked() {
                        self.tune.page = Some(p);
                        if p == Page::World && !self.world_open() {
                            self.acts.push((Act::WorldToggle, 0));
                        }
                    }
                }
            });
        });
    }

    /// Hover text: the item's name and its key.
    fn tip(act: Act, text: &str, r: Response) -> Response {
        match key_name(act) {
            "" => r.on_hover_text(text),
            k => r.on_hover_text(format!("{text}  [{k}]")),
        }
    }

    /// A large tool: the icon over a short caption; `on` = active (accent).
    fn tool_button(ui: &mut Ui, icon: &str, text: &str, on: bool) -> Response {
        let font = FontId::proportional(11.0);
        let enabled = ui.is_enabled();
        let galley = ui.painter().layout_no_wrap(text.to_string(), font, theme::TEXT);
        let (rect, r) = ui.allocate_exact_size(Vec2::new((galley.size().x + 12.0).max(46.0), TOOL_H), Sense::click());
        let fill = match (on, r.hovered() || r.is_pointer_button_down_on()) {
            (true, true) => theme::ACCENT_HOT,
            (true, false) => theme::ACCENT,
            (false, true) => theme::HOVER,
            (false, false) => theme::WIDGET,
        };
        let ink = if enabled { theme::TEXT } else { theme::TEXT_WEAK };
        ui.painter().rect(rect, 4, fill, Stroke::new(1.0, theme::STROKE), StrokeKind::Inside);
        ui.painter().text(rect.center_top() + Vec2::new(0.0, 17.0), Align2::CENTER_CENTER, icon, FontId::proportional(20.0), ink);
        ui.painter().galley(rect.center_bottom() - Vec2::new(galley.size().x / 2.0, galley.size().y + 4.0), galley, ink);
        r
    }

    /// A large tool that runs `act`.
    fn tool(&mut self, ui: &mut Ui, act: Act, icon: &str, text: &str) {
        if Self::tip(act, text, Self::tool_button(ui, icon, text, false)).clicked() {
            self.acts.push((act, 0));
        }
    }

    fn tool_toggle(&mut self, ui: &mut Ui, act: Act, icon: &str, text: &str, on: bool) {
        if Self::tip(act, text, Self::tool_button(ui, icon, text, on)).clicked() {
            self.acts.push((act, 0));
        }
    }

    /// A compact icon + label button that runs `act`.
    fn item(&mut self, ui: &mut Ui, act: Act, icon: &str, text: &str) {
        if Self::tip(act, text, ui.add(Button::new(format!("{icon}  {text}")))).clicked() {
            self.acts.push((act, 0));
        }
    }

    /// A compact icon + label toggle for `act`.
    fn toggle(&mut self, ui: &mut Ui, act: Act, icon: &str, text: &str, on: bool) {
        if Self::tip(act, text, ui.add(Button::selectable(on, format!("{icon}  {text}")))).clicked() {
            self.acts.push((act, 0));
        }
    }

    fn view_tab(&mut self, ui: &mut Ui) {
        self.group(ui, "Camera", Some(Page::Camera), 470.0, |s, ui| {
            s.tool_toggle(ui, Act::FreeCam, ic::FREE_CAM, "Free cam", s.free);
            s.tool(ui, Act::ResetCamera, ic::RESET, "Reset");
            s.tool(ui, Act::Camera, ic::NEXT_CAMERA, "Next cam");
            s.tool_toggle(ui, Act::Mirror, ic::MIRROR, "Mirror", s.mirror);
            if let Some(ss) = s.sess.as_deref_mut() {
                combo(ui, ic::VIDEO, "race camera", CAM_MODES.iter().find(|m| m.0 == ss.cam).map_or("?", |m| m.1), |ui| {
                    for (m, name) in CAM_MODES {
                        ui.selectable_value(&mut ss.cam, m, name);
                    }
                });
            }
        });
        self.group(ui, "Show", None, 190.0, |s, ui| {
            s.tool_toggle(ui, Act::Overlay, ic::GAUGE, "Overlay", s.overlay);
            s.tool_toggle(ui, Act::Cross, ic::CROSSHAIR, "Target", s.cross);
            s.tool_toggle(ui, Act::MenuBar, ic::PANEL_TOP, "Ribbon", true);
        });
        self.group(ui, "Capture", None, 180.0, |s, ui| {
            s.tool(ui, Act::Screenshot, ic::CAMERA, "Screenshot");
            s.tool(ui, Act::Escape, ic::EXIT, "Exit");
        });
    }

    fn sim_tab(&mut self, ui: &mut Ui) {
        self.group(ui, "Playback", Some(Page::Sim), 800.0, |s, ui| {
            // held buttons: the sim goes back / runs ahead while the button is down, then returns
            // to what it was doing (playing or paused)
            let back = Self::tool_button(ui, ic::REWIND, "Rewind", false).on_hover_text("hold: run the race backwards  [, key]");
            let paused = s.dbg.paused;
            s.tool_toggle(ui, Act::Pause, if paused { ic::PLAY } else { ic::PAUSE }, if paused { "Paused" } else { "Playing" }, paused);
            s.tool(ui, Act::Step, ic::STEP, "Step");
            let fwd = Self::tool_button(ui, ic::FORWARD, "Forward", false).on_hover_text("hold: run the race ahead at x4  [/ key]");
            if back.is_pointer_button_down_on() {
                *s.scrub = -1;
            } else if fwd.is_pointer_button_down_on() {
                *s.scrub = 1;
            }
            // timeline (#60): the whole recorded run, drag to scrub; driving in the past takes over
            if let Some((start, now, end)) = s.sess.as_ref().and_then(|x| x.timeline.as_ref()).map(|t| t.span()) {
                let mut at = now;
                let slider = ui.add_sized([SLIDER_W * 2.0, 24.0], egui::Slider::new(&mut at, start..=end.max(start + 0.01)).show_value(false)).on_hover_text("timeline: drag to go back and forward in the recorded run");
                if slider.changed() {
                    *s.seek = Some(at);
                }
                ui.label(format!("{} / {}", clock(now), clock(end)));
            }
            s.item(ui, Act::Slower, ic::SLOWER, "Slower");
            s.item(ui, Act::Faster, ic::FASTER, "Faster");
            combo(ui, ic::CLOCK, "time scale", &format!("x{}", TIME_SCALES[s.dbg.scale]), |ui| {
                for (i, x) in TIME_SCALES.iter().enumerate() {
                    ui.selectable_value(&mut s.dbg.scale, i, format!("x{x}"));
                }
            });
        });
        self.group(ui, "Race", None, 540.0, |s, ui| {
            s.tool(ui, Act::Restart, ic::RESTART, "Restart");
            s.tool(ui, Act::Respawn, ic::RESPAWN, "Respawn");
            s.tool(ui, Act::Focus, ic::FOCUS, "Follow next");
            let on = s.drive_free;
            s.tool_toggle(ui, Act::DriveFree, ic::CAR, "Drive in free cam", on);
            let names = ["off", "followed car", "followed car + race"];
            combo(ui, ic::CHART, "sim panel", names[s.dbg.panel as usize % 3], |ui| {
                for (i, name) in names.into_iter().enumerate() {
                    ui.selectable_value(&mut s.dbg.panel, i as u8, name);
                }
            });
        });
        // repro files (#60): the recorded run saved as .mhr, or a game / viewer recording opened
        self.group(ui, "Run file", None, 330.0, |s, ui| {
            let can = s.sess.as_ref().is_some_and(|x| x.setup.is_some() && x.timeline.as_ref().is_some_and(|t| t.whole()));
            ui.add_enabled_ui(can, |ui| {
                if Self::tool_button(ui, ic::SAVE, "Save run", false).on_hover_text("write the whole recorded run to the game's Replays folder as a repro .mhr").clicked() {
                    s.acts.push((Act::SaveRun, 0));
                }
            });
            combo(ui, ic::UPLOAD, "open run", "Replays", |ui| {
                let runs = runs();
                if runs.is_empty() {
                    ui.weak("no .mhr in the Replays folder of motorhead's settings directory");
                }
                for (i, e) in runs.iter().enumerate() {
                    if ui.selectable_label(false, &e.name).clicked() {
                        s.acts.push((Act::OpenRun, i));
                    }
                }
            });
        });
        self.group(ui, "Track", None, 270.0, |s, ui| {
            ui.label(ic::ROUTE);
            for t in 1..=8u32 {
                if ui.add(Button::selectable(s.track == t, t.to_string()).min_size(Vec2::splat(24.0))).on_hover_text(format!("track {t}")).clicked() {
                    s.acts.push((Act::Track, t as usize - 1));
                }
            }
        });
        self.group(ui, "Car", None, 440.0, |s, ui| {
            ui.label(ic::CAR);
            for c in 1..=10u32 {
                if ui.add(Button::selectable(s.car == c, c.to_string()).min_size(Vec2::splat(24.0))).on_hover_text(format!("car {c}")).clicked() {
                    s.acts.push((Act::Car, c as usize - 1));
                }
            }
            s.item(ui, Act::NextCar, ic::NEXT_CAMERA, "Next car");
            s.item(ui, Act::ReloadCars, ic::RELOAD, "Reload recipes");
            if Self::tip(Act::SwapMesh, "the followed car: original <-> generated model", ui.add(Button::selectable(s.mesh_swap, format!("{}  Swap mesh", ic::BOX)))).clicked() {
                s.acts.push((Act::SwapMesh, 0));
            }
        });
        self.group(ui, "Physics", Some(Page::Car), 150.0, |s, ui| {
            if let Some(ss) = s.sess.as_deref_mut() {
                ui.label(format!("{}  gravity", ic::ORBIT));
                wheel_drag(ui, &mut ss.race.world.gravity, None, |d| d.speed(0.05));
            }
        });
    }

    fn render_tab(&mut self, ui: &mut Ui) {
        self.group(ui, "Effects", None, 560.0, |s, ui| {
            let v = &*s.view;
            let (fog, shadows, halos, line) = (v.fog, v.shadows, v.halos, v.show_line);
            s.tool_toggle(ui, Act::Fog, ic::FOG, "Fog", fog);
            s.tool_toggle(ui, Act::Shadows, ic::SHADOW, "Shadows", shadows);
            s.tool_toggle(ui, Act::Halos, ic::SUN, "Halos", halos);
            s.tool_toggle(ui, Act::RacingLine, ic::ROUTE, "Line", line);
            let v = &mut *s.view;
            for (b, icon, name) in [(&mut v.lens_flares, ic::ZAP, "lens flares"), (&mut v.smoke, ic::WIND, "smoke"), (&mut v.sparks, ic::FLAME, "sparks"), (&mut v.sky_image, ic::CLOUD, "sky image"), (&mut v.motion_blur, ic::BLEND, "motion blur")] {
                chip(ui, b, icon, name);
            }
        });
        self.group(ui, "Light", None, 300.0, |s, ui| {
            wheel_slider(ui, &mut s.view.brightness, 0.1..=3.0, |s| s.prefix(format!("{}  ", ic::BULB)).text("brightness"));
            wheel_slider(ui, &mut s.view.distance, 0.25..=View::MAX_DISTANCE, |s| s.prefix(format!("{}  ", ic::BINOCULARS)).text("view distance"));
            wheel_slider(ui, &mut s.view.fov_x, 40.0..=140.0, |s| s.prefix(format!("{}  ", ic::APERTURE)).text("FOV (horizontal °)"));
            combo(ui, ic::CAR, "car shading", s.view.car_shading.name(), |ui| {
                for c in CarShading::ALL {
                    ui.selectable_value(&mut s.view.car_shading, c, c.name());
                }
            });
        });
        self.group(ui, "Debug view", None, 440.0, |s, ui| {
            combo(ui, ic::GRID, "wire style", s.wireframe.name(), |ui| {
                for w in WireStyle::ALL {
                    ui.selectable_value(&mut *s.wireframe, w, w.name());
                }
            });
            // a solid colour in place of the sky; the picker keeps its last colour while off
            let mut on = s.view.debug.backdrop.is_some();
            chip(ui, &mut on, ic::IMAGE, "backdrop");
            if on != s.view.debug.backdrop.is_some() {
                s.view.debug.backdrop = on.then_some(s.st.backdrop);
            }
            if let Some(c) = &mut s.view.debug.backdrop {
                egui::color_picker::color_edit_button_srgb(ui, c);
                s.st.backdrop = *c;
            }
            s.tool_toggle(ui, Act::RenderPanel, ic::LAYERS, "Passes", s.dbg.render_panel);
            chip(ui, &mut s.dbg.passes.wheels, ic::DISC, "wheels");
            combo(ui, ic::PALETTE, "shading view", s.view.debug.shade.name(), |ui| {
                for sh in DebugShade::ALL {
                    ui.selectable_value(&mut s.view.debug.shade, sh, sh.name());
                }
            });
            ui.menu_button(format!("{}  pass list", ic::LIST), |ui| {
                for (i, name) in Passes::HELP.iter().enumerate() {
                    if let Some(b) = s.dbg.passes.get(i) {
                        ui.checkbox(b, *name);
                    }
                }
                ui.checkbox(&mut s.view.debug.unlimited, "unlimited view distance");
            });
        });
    }

    /// The Remaster tab: toggles and sliders stacked in columns (three rows is the group height),
    /// so the whole tab fits 1440p without scrolling sideways.
    fn remaster_tab(&mut self, ui: &mut Ui) {
        self.group_auto(ui, "Car", Some(Page::CarLights), |s, ui| {
            let r = &s.view.remaster;
            let (paint, lights) = (r.enhanced_paint, r.dynamic_lights);
            s.tool_toggle(ui, Act::Paint, ic::PALETTE, "Paint", paint);
            s.tool_toggle(ui, Act::Lights, ic::BULB, "Lights", lights);
            let r = &mut s.view.remaster;
            ui.vertical(|ui| {
                for (b, icon, name) in [(&mut r.clear_coat, ic::SPARKLES, "clear coat"), (&mut r.car_shadows, ic::SHADOW, "car shadows"), (&mut r.generated_cars, ic::CAR, "generated cars")] {
                    chip(ui, b, icon, name);
                }
                ui.add_enabled_ui(r.generated_cars, |ui| {
                    let (lo, hi) = RemasterView::CAR_LOD_DISTANCE;
                    ui.spacing_mut().slider_width = SLIDER_W;
                    wheel_slider(ui, &mut r.car_lod_distance, lo..=hi, |s| s.text("car detail distance %"));
                });
            });
        });
        self.group_auto(ui, "Surface", None, |s, ui| {
            let r = &mut s.view.remaster;
            ui.vertical(|ui| {
                for (b, icon, name) in [(&mut r.pixel_light, ic::BULB, "pixel light"), (&mut r.pixel_fog, ic::FOG, "pixel fog"), (&mut r.retro_signs, ic::FLAG, "reflective signs")] {
                    chip(ui, b, icon, name);
                }
            });
            ui.vertical(|ui| {
                for (b, icon, name) in [(&mut r.smooth_halos, ic::SUN, "smooth halos"), (&mut r.smooth_flares, ic::ZAP, "smooth flares")] {
                    chip(ui, b, icon, name);
                }
            });
        });
        self.group(ui, "Anti-aliasing", None, 300.0, |s, ui| {
            let r = &mut s.view.remaster;
            combo(ui, ic::SCAN, "render AA", r.render_aa.kind.name(), |ui| {
                for k in RenderAaKind::AVAILABLE {
                    ui.selectable_value(&mut r.render_aa.kind, k, k.name());
                }
            });
            aa_stop_combo(ui, &mut r.render_aa.strength, r.render_aa.kind != RenderAaKind::Off);
            combo(ui, ic::SCAN, "post AA", r.post_aa.kind.name(), |ui| {
                for k in PostAaKind::AVAILABLE {
                    ui.selectable_value(&mut r.post_aa.kind, k, k.name());
                }
            });
            aa_stop_combo(ui, &mut r.post_aa.strength, r.post_aa.kind != PostAaKind::Off);
            combo(ui, ic::IMAGE, "anisotropy", &format!("{}x", r.anisotropy), |ui| {
                for n in RemasterView::ANISOTROPY_LEVELS {
                    ui.selectable_value(&mut r.anisotropy, n, format!("{n}x"));
                }
            });
        });
        self.group_auto(ui, "HDR", None, |s, ui| {
            let r = &mut s.view.remaster;
            ui.vertical(|ui| {
                chip(ui, &mut r.hdr, ic::SUN, "HDR");
                combo(ui, ic::PALETTE, "tonemap", r.tonemap.name(), |ui| {
                    for m in Tonemap::ALL {
                        ui.selectable_value(&mut r.tonemap, m, m.name());
                    }
                });
            });
            ui.vertical(|ui| {
                let (lo, hi) = RemasterView::EXPOSURE_RANGE;
                ui.spacing_mut().slider_width = SLIDER_W;
                wheel_slider(ui, &mut r.exposure, lo..=hi, |s| s.prefix(format!("{}  ", ic::APERTURE)).text("exposure %"));
                slider(ui, &mut r.bloom, "bloom %");
            });
        });
        self.group_auto(ui, "Post", None, |s, ui| {
            let grade = s.view.remaster.post();
            s.tool_toggle(ui, Act::Grade, ic::PALETTE, "Grade", grade);
            let r = &mut s.view.remaster;
            ui.vertical(|ui| {
                chip(ui, &mut r.velocity_blur, ic::BLEND, "velocity blur");
                slider(ui, &mut r.ssao, "AO %").on_hover_text("ambient occlusion");
                slider(ui, &mut r.grade, "grade %");
            });
            ui.vertical(|ui| {
                slider(ui, &mut r.vignette, "vignette %");
                slider(ui, &mut r.grain, "grain %");
            });
        });
        self.group_auto(ui, "Fog", Some(Page::Fog), |s, ui| {
            let r = &mut s.view.remaster;
            ui.vertical(|ui| {
                for st in motorview::render::FogStyle::ALL {
                    ui.selectable_value(&mut r.fog_style, st, st.name());
                }
            });
        });
        self.group_auto(ui, "Sky", Some(Page::Sky), |s, ui| {
            ui.vertical(|ui| {
                chip(ui, &mut s.view.remaster.sky_hdri, ic::CLOUD, "HDRI sky");
                chip(ui, &mut s.view.remaster.water, ic::WAVES, "water");
                chip(ui, &mut s.view.remaster.water_ssr, ic::WAVES, "water SSR");
                chip(ui, &mut s.view.remaster.local_light, ic::SUN, "local light");
            });
        });
        self.group_auto(ui, "Grass", None, |s, ui| {
            let r = &mut s.view.remaster;
            ui.vertical(|ui| {
                combo(ui, ic::TREE, "grass", r.grass.name(), |ui| {
                    for l in GrassLevel::ALL {
                        ui.selectable_value(&mut r.grass, l, l.name());
                    }
                });
                ui.add_enabled_ui(r.grass != GrassLevel::Off, |ui| {
                    let (lo, hi) = RemasterView::GRASS_RANGE;
                    ui.spacing_mut().slider_width = SLIDER_W;
                    wheel_slider(ui, &mut r.grass_range, lo..=hi, |s| s.text("range %"));
                });
            });
        });
        self.group_auto(ui, "Shadows", Some(Page::Lamps), |s, ui| {
            let r = &mut s.view.remaster;
            ui.vertical(|ui| {
                combo(ui, ic::SHADOW, "light shadows", r.light_shadows.name(), |ui| {
                    for m in LightShadows::ALL {
                        ui.selectable_value(&mut r.light_shadows, m, m.name());
                    }
                });
                combo(ui, ic::SLIDERS, "quality", r.shadow_quality.name(), |ui| {
                    for q in ShadowQuality::ALL {
                        ui.selectable_value(&mut r.shadow_quality, q, q.name());
                    }
                });
            });
        });
    }

    fn debug_tab(&mut self, ui: &mut Ui) {
        self.group(ui, "Gizmos", None, 400.0, |s, ui| {
            let l = s.dbg.layers;
            s.tool_toggle(ui, Act::TrackGizmos, ic::ROUTE, "Track", l.track);
            s.tool_toggle(ui, Act::ColGizmos, ic::GRID, "Collision", l.collision);
            s.tool_toggle(ui, Act::BoundGizmos, ic::BOX, "Bounds", l.bounds);
            s.tool_toggle(ui, Act::LightGizmos, ic::BULB, "Lights", l.lights);
            s.tool_toggle(ui, Act::ProbeGizmos, ic::CIRCLE_DOT, "Probes", l.probes);
            let l = &mut s.dbg.layers;
            let names = [(false, false, "no car"), (true, false, "followed car"), (true, true, "all cars")];
            let cur = names.iter().find(|n| (n.0, n.1) == (l.car, l.all_cars)).map_or("car", |n| n.2);
            combo(ui, ic::CAR, "car gizmos", cur, |ui| {
                for (car, all, name) in names {
                    if ui.add(Button::selectable((l.car, l.all_cars) == (car, all), name)).clicked() {
                        (l.car, l.all_cars) = (car, all);
                    }
                }
            });
        });
        self.group(ui, "Inspect", None, 420.0, |s, ui| {
            s.tool(ui, Act::Inspect, ic::SEARCH, "Inspect");
            s.tool(ui, Act::Jump, ic::RESPAWN, "Jump to");
            let b = &mut s.dbg.browser;
            let tabs = [(None, "closed"), (Some(Tab::Textures), "world textures"), (Some(Tab::Props), "scene objects"), (Some(Tab::Cars), "cars")];
            combo(ui, ic::LIST, "data browser", tabs.iter().find(|t| t.0 == b.tab).map_or("?", |t| t.1), |ui| {
                for (tab, name) in tabs {
                    if ui.selectable_value(&mut b.tab, tab, name).changed() {
                        (b.sel, b.jumps) = (0, 0);
                    }
                }
            });
            s.item(ui, Act::BrowseFirst, ic::REWIND, "first");
            s.item(ui, Act::BrowseUp, ic::CHEVRON_UP, "previous");
            s.item(ui, Act::BrowseDown, ic::CHEVRON_DOWN, "next");
            s.item(ui, Act::BrowseLast, ic::FORWARD, "last");
        });
    }

    fn world_open(&self) -> bool {
        self.world.as_ref().is_some_and(|w| w.open)
    }

    fn world_tab(&mut self, ui: &mut Ui) {
        let open = self.world_open();
        self.group(ui, "Editor", Some(Page::World), 100.0, |s, ui| s.tool_toggle(ui, Act::WorldToggle, ic::MOUNTAIN, "World editor", open));
        let Some(w) = self.world.as_deref().filter(|w| w.open) else { return };
        let (unsaved, undo, redo, holes) = (w.unsaved(), w.can_undo(), w.can_redo(), w.holes);
        let mut acts = Vec::new();
        let mut cmd = |ui: &mut Ui, on: bool, c: Cmd, icon: &str, text: &str| {
            if ui.add_enabled(on, Button::new(format!("{icon}  {text}"))).on_hover_text(format!("{text}  [{}]", key_name(Act::World(c)))).clicked() {
                acts.push((Act::World(c), 0));
            }
        };
        self.group_with(ui, "File", 130.0, |ui| {
            cmd(ui, true, Cmd::Save, ic::SAVE, if unsaved { "Save *" } else { "Save" });
            cmd(ui, true, Cmd::Reload, ic::RELOAD, "Reload");
        });
        self.group_with(ui, "History", 130.0, |ui| {
            cmd(ui, undo, Cmd::Undo, ic::UNDO, "Undo");
            cmd(ui, redo, Cmd::Redo, ic::REDO, "Redo");
        });
        self.group_with(ui, "Generate", 130.0, |ui| cmd(ui, true, Cmd::Generate, ic::WAND, "Auto fill"));
        self.group_with(ui, "Select", 130.0, |ui| {
            cmd(ui, true, Cmd::Step(-1), ic::REWIND, "Previous");
            cmd(ui, true, Cmd::Step(1), ic::FORWARD, "Next");
        });
        self.acts.extend(acts);
        let mut toggle_holes = false;
        self.group(ui, "View", None, 470.0, |s, ui| {
            let Some(w) = s.world.as_deref_mut() else { return };
            if ui.add(Button::selectable(holes, format!("{}  Holes", ic::SEARCH))).on_hover_text("open world edges near the road  [Ctrl+H]").clicked() {
                toggle_holes = true;
            }
            ui.label("work height");
            wheel_drag(ui, &mut w.work_y, None, |d| d.speed(0.5));
            ui.separator();
            for k in Kind::ALL.into_iter().filter(|k| *k != Kind::Water) {
                let on = !w.hidden[k as usize];
                if ui.add(Button::selectable(on, kind_icon(k))).on_hover_text(format!("{} layer", k.name())).clicked() {
                    w.hidden[k as usize] = on;
                }
            }
        });
        if toggle_holes {
            self.acts.push((Act::World(Cmd::Holes), 0));
        }
    }

    /// A group with no launcher whose body only needs the `Ui` (not the ribbon).
    fn group_with(&mut self, ui: &mut Ui, title: &str, width: f32, body: impl FnOnce(&mut Ui)) {
        self.group(ui, title, None, width, |_, ui| body(ui));
    }

    /// The Cine editor (#152, cine_edit.rs): file, cameras, shots, preview, the sequence's clock.
    fn cine_tab(&mut self, ui: &mut Ui) {
        let Some(ed) = self.cine.take() else { return };
        let open = ed.active();
        self.group(ui, "File", Some(Page::Cine), 330.0, |s, ui| {
            if Self::tool_button(ui, ic::FILM, "New", false).on_hover_text("a new sequence on this track, its first camera where the free camera is (the race becomes all AI)").clicked() {
                ed.reqs.push(Req::New);
            }
            if Self::tool_button(ui, ic::UPLOAD, "Open…", false).on_hover_text("pick a .cine in the pane").clicked() {
                s.tune.page = Some(Page::Cine);
            }
            ui.add_enabled_ui(open, |ui| {
                if Self::tool_button(ui, ic::SAVE, "Save", false).clicked() {
                    ed.msg = match ed.save(None) {
                        Ok(p) => format!("saved {}", p.display()),
                        Err(e) => e,
                    };
                }
                if Self::tool_button(ui, ic::CLOSE, "Close", false).on_hover_text("leave the editor (the race stays)").clicked() {
                    ed.close();
                }
            });
            match &ed.path {
                Some(p) => ui.label(format!("{}{}", p.file_name().map_or_else(String::new, |n| n.to_string_lossy().into_owned()), if ed.dirty { " *" } else { "" })),
                None if open => ui.label(if ed.dirty { "unsaved *" } else { "unsaved" }),
                None => ui.weak("no sequence"),
            };
        });
        if !open {
            self.cine = Some(ed);
            return;
        }
        let ncars = ed.seq.as_ref().map_or(0, |q| q.race.cars.len());
        self.group(ui, "Cameras", Some(Page::Cine), 600.0, |s, ui| {
            s.tool(ui, Act::CineAddCam, ic::CAMERA, "Add here");
            if Self::tool_button(ui, ic::CAR, "Mounted", false).on_hover_text("a camera on the followed car, where the free camera is").clicked() {
                ed.reqs.push(Req::AddMounted);
            }
            if Self::tool_button(ui, ic::SPLINE, "Path", false).on_hover_text("a path camera, its first key at the free camera").clicked() {
                ed.reqs.push(Req::AddPath);
            }
            s.tool(ui, Act::CineAddKey, ic::ROUTE, "Add key");
            ui.add_enabled_ui(ed.seq.as_ref().is_some_and(|q| q.cameras.len() > 1), |ui| {
                if Self::tool_button(ui, ic::TRASH, "Remove", false).on_hover_text("remove the selected camera and its shots").clicked() {
                    ed.remove_selected();
                }
            });
            ui.vertical(|ui| {
                if let Some(seq) = ed.seq.as_ref() {
                    let sel = ed.sel.min(seq.cameras.len() - 1);
                    let mut pick = sel;
                    ComboBox::from_label(format!("{} camera", ic::VIDEO)).selected_text(&seq.cameras[sel].name).show_ui(ui, |ui| {
                        for (i, c) in seq.cameras.iter().enumerate() {
                            ui.selectable_value(&mut pick, i, &c.name);
                        }
                    });
                    ed.sel = pick;
                }
                ui.horizontal(|ui| {
                    cine_edit::target_combo(ui, "cine-new-target", &mut ed.target, true, ncars, &cine_edit::names(s.sess.as_deref()));
                    ui.label(format!("{} new cameras follow", ic::FOCUS));
                });
            });
        });
        self.group(ui, "Preview", None, 250.0, |_, ui| {
            if Self::tip(Act::CineCut, "Cut here", Self::tool_button(ui, ic::VIDEO, "Cut here", false)).clicked() {
                ed.reqs.push(Req::Cut);
            }
            ui.vertical(|ui| {
                ui.horizontal(|ui| {
                    for p in Preview::ALL {
                        if Self::tip(Act::CinePreview, p.name(), ui.add(Button::selectable(ed.preview == p, p.name()))).clicked() {
                            ed.preview = p;
                        }
                    }
                });
                if let (Some(seq), Some(air)) = (ed.seq.as_ref(), ed.on_air()) {
                    ui.label(format!("{} on air: {}", ic::VIDEO, seq.cameras[air].name));
                }
            });
        });
        self.group(ui, "Render", None, 230.0, |_, ui| {
            match (ed.job.as_ref(), ed.render_path()) {
                (None, Some(out)) => {
                    let name = out.file_name().map_or_else(String::new, |n| n.to_string_lossy().into_owned());
                    if Self::tool_button(ui, ic::FILM, "Render", false).on_hover_text(format!("the whole sequence as it is here to {} in the background, with these graphics options (the status bar shows the progress)", out.display())).clicked() {
                        ed.reqs.push(Req::Render);
                    }
                    ui.weak(name);
                }
                (None, None) => {
                    ui.weak("no file to render next to");
                }
                (Some(j), _) => {
                    if Self::tool_button(ui, ic::CLOSE, "Cancel", false).on_hover_text("stop the render (the frames so far stay in the video)").clicked() {
                        j.watch.cancel.store(true, std::sync::atomic::Ordering::Relaxed);
                    }
                    if let Some((text, _, _)) = ed.job_status() {
                        ui.label(text);
                    }
                }
            }
        });
        self.group(ui, "Clock", Some(Page::Cine), 520.0, |s, ui| {
            let paused = s.dbg.paused;
            s.tool_toggle(ui, Act::Pause, if paused { ic::PLAY } else { ic::PAUSE }, if paused { "Paused" } else { "Playing" }, paused);
            if Self::tool_button(ui, ic::RESTART, "Start", false).on_hover_text("the clock to 0").clicked() {
                ed.seek(0.0);
            }
            if Self::tool_button(ui, ic::REWIND, "Prev shot", false).clicked() {
                ed.shot_step(-1);
            }
            if Self::tool_button(ui, ic::FORWARD, "Next shot", false).clicked() {
                ed.shot_step(1);
            }
            ui.vertical(|ui| {
                let end = ed.slider_end(s.sess.as_deref());
                let mut at = ed.t;
                let slider = ui.add_sized([SLIDER_W * 2.5, 24.0], egui::Slider::new(&mut at, 0.0..=end.max(0.01)).show_value(false)).on_hover_text("the sequence's clock (video seconds): drag to scrub, past the recorded run the race runs ahead");
                if slider.changed() {
                    ed.seek(at);
                }
                let race = ed.race_secs();
                let behind = if ed.behind { "  running ahead…" } else { "" };
                ui.label(format!("{} / {}   race {}{behind}", clock(ed.t as f64), clock(end as f64), clock(race as f64)));
            });
        });
        self.cine = Some(ed);
    }

    fn ghost_tab(&mut self, ui: &mut Ui) {
        self.group(ui, "Ghost", Some(Page::Ghost), 390.0, |s, ui| {
            if Self::tool_button(ui, ic::UPLOAD, "Load…", false).on_hover_text("pick a .ghl or .mhr in the pane").clicked() {
                s.tune.page = Some(Page::Ghost);
            }
            ui.add_enabled_ui(s.ghost.is_some(), |ui| {
                if Self::tool_button(ui, ic::TRASH, "Clear", false).clicked() {
                    s.acts.push((Act::GhostClear, 0));
                }
            });
            let open = s.ghost_win.is_some();
            s.tool_toggle(ui, Act::GhostWindow, ic::APP, if open { "Close window" } else { "Open window" }, open);
            match s.ghost.as_deref() {
                Some(g) => ui.label(format!("{} (car {})", g.name, g.car)),
                None => ui.weak("no ghost loaded"),
            };
        });
        self.group(ui, "Show", None, 330.0, |s, ui| {
            if let Some(g) = s.ghost.as_deref_mut() {
                chip(ui, &mut g.show_main, ic::EYE, "in the main view");
                combo(ui, ic::CLOCK, "clock", g.sync.name(), |ui| {
                    for m in Sync::ALL {
                        ui.selectable_value(&mut g.sync, m, m.name());
                    }
                });
            }
        });
        self.group(ui, "Ghost window", None, 420.0, |s, ui| {
            let has_ghost = s.ghost.is_some();
            let cur = s.ghost_follow;
            let name = |f: Follow, sess: Option<&Session>| match f {
                Follow::Ghost => "the ghost".to_string(),
                Follow::Car(i) => sess.and_then(|ss| ss.race.racers.get(i)).map_or("?".to_string(), |r| format!("#{i} {}", r.driver.name)),
            };
            let shown = name(cur, s.sess.as_deref());
            let mut picked = None;
            combo(ui, ic::FOCUS, "follow", &shown, |ui| {
                if ui.add_enabled(has_ghost, Button::selectable(cur == Follow::Ghost, "the ghost")).clicked() {
                    picked = Some(0);
                }
                for i in 0..s.sess.as_deref().map_or(0, |ss| ss.race.racers.len()) {
                    if ui.add(Button::selectable(cur == Follow::Car(i), name(Follow::Car(i), s.sess.as_deref()))).clicked() {
                        picked = Some(i + 1);
                    }
                }
            });
            if let Some(i) = picked {
                s.acts.push((Act::GhostFollow, i));
            }
            let open = s.ghost_win.is_some();
            let cur = s.ghost_win.unwrap_or(s.ghost_cam);
            combo(ui, ic::VIDEO, "camera", CAM_MODES.iter().find(|m| m.0 == cur).map_or("?", |m| m.1), |ui| {
                for (i, m) in PoseCam::MODES.iter().enumerate() {
                    if ui.add(Button::selectable(*m == cur, CAM_MODES.iter().find(|c| c.0 == *m).map_or("?", |c| c.1))).clicked() {
                        s.acts.push((Act::GhostCamSet, i));
                    }
                }
            });
            ui.add_enabled_ui(open, |ui| s.item(ui, Act::GhostCam, ic::NEXT_CAMERA, "next camera"));
        });
    }

    fn help_tab(&mut self, ui: &mut Ui) {
        self.group(ui, "Keys", None, 330.0, |_, ui| {
            for (title, keys) in [("free camera", HELD_FREE), ("race camera", HELD_RACE)] {
                ui.menu_button(format!("{}  {title}", ic::KEYBOARD), |ui| {
                    for (k, what) in keys {
                        ui.add_enabled(false, Button::new(*what).shortcut_text(*k));
                    }
                });
            }
        });
        self.group(ui, "Shortcuts", None, 460.0, |s, ui| {
            ui.horizontal(|ui| {
                ui.label(ic::SEARCH);
                ui.add(egui::TextEdit::singleline(&mut s.st.filter).hint_text("filter keys and actions").desired_width(200.0));
                if ui.small_button(ic::CLOSE).clicked() {
                    s.st.filter.clear();
                }
            });
            let f = s.st.filter.to_lowercase();
            egui::ScrollArea::vertical().max_height(TOOL_H + 18.0).show(ui, |ui| ui.horizontal_wrapped(|ui| {
                for (title, ctx) in [("any", Ctx::Any), ("race camera", Ctx::Race), ("free camera", Ctx::Free), ("world editor", Ctx::World)] {
                    for b in BINDS.iter().filter(|b| b.ctx == ctx && (f.is_empty() || b.label().to_lowercase().contains(&f) || b.name.to_lowercase().contains(&f) || title.contains(&f))) {
                        ui.weak(format!("{}  [{}]", b.label(), b.name)).on_hover_text(title);
                    }
                }
            }));
        });
    }
}

/// A combo box with its icon and label before it.
fn combo(ui: &mut Ui, icon: &str, label: &str, shown: &str, body: impl FnOnce(&mut Ui)) {
    ComboBox::from_label(format!("{icon} {label}")).selected_text(shown).show_ui(ui, body);
}

/// A toggle chip on a bool (icon + label, accent when on).
fn chip(ui: &mut Ui, v: &mut bool, icon: &str, text: &str) {
    if ui.add(Button::selectable(*v, format!("{icon}  {text}"))).clicked() {
        *v ^= true;
    }
}

/// A 0..=100 slider.
/// the strength of an AA group as a stop of the shared 2x .. 16x scale; hidden while its method is Off
fn aa_stop_combo(ui: &mut Ui, strength: &mut u32, on: bool) {
    if on {
        combo(ui, ic::SCAN, "strength", AA_STOPS[aa_stop(*strength)], |ui| {
            for (i, n) in AA_STOPS.iter().enumerate() {
                if ui.selectable_label(aa_stop(*strength) == i, *n).clicked() {
                    *strength = AA_STOP_STRENGTH[i];
                }
            }
        });
    }
}

fn slider(ui: &mut Ui, v: &mut u32, name: &str) -> Response {
    ui.spacing_mut().slider_width = SLIDER_W;
    wheel_slider(ui, v, 0..=100, |s| s.text(name))
}
