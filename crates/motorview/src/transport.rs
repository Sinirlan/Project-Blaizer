//! DEBUG motorview's transport strip (#435): a slim glass card above the status bar on every tab (and with the shelf
//! hidden) while the run has a timeline: rewind, pause, step, forward, the scrubber with `m:ss.s / m:ss.s`, the speed
//! dropdown. With the Cine editor open it shows the Cine clock instead (Start, previous / next shot, the Cine
//! timeline): never two timelines. `Shift+\` shows / hides it (`RibbonState::no_transport`).
//!
//! The shelf's Sim > Playback and Cine > Clock groups keep their tools; this strip is the same controls with the
//! same held-button logic (`scrub`, `seek`) in a place that does not depend on the tab.

use super::ribbon::clock;
use super::{Act, DebugState, TIME_SCALES};
use egui::{Align2, Color32, FontId, Margin, Response, Sense, Stroke, StrokeKind, Ui, Vec2};
use motorview::cine_edit::CineEdit;
use motorview::drive::Session;
use motorview::icons as ic;
use motorview::{glass_ui, theme};

/// The card's height and the room around the strip (points): 4 above, none below (the status bar has its own 10 on top).
const H: f32 = 36.0;
const TOP: f32 = 4.0;

/// What the strip works on. `acts` and `scrub` / `seek` are the ribbon's: the app applies them after the frame.
pub struct Transport<'a> {
    pub dbg: &'a mut DebugState,
    pub sess: Option<&'a Session>,
    pub cine: Option<&'a mut CineEdit>,
    pub scrub: &'a mut i8,
    pub seek: &'a mut Option<f64>,
    pub acts: &'a mut Vec<(Act, usize)>,
}

impl Transport<'_> {
    /// Does the strip have a timeline to show (the Cine clock, or the run's recording)?
    pub fn wanted(sess: Option<&Session>, cine: Option<&CineEdit>) -> bool {
        cine.is_some_and(CineEdit::active) || sess.is_some_and(|s| s.timeline.is_some())
    }

    /// The strip above the status bar (add it right after the status bar). Its height in points, margins included.
    pub fn show(mut self, ui: &mut Ui) -> f32 {
        let ctx = ui.ctx().clone();
        let cine = self.cine.as_ref().is_some_and(|c| c.active());
        let r = egui::Panel::bottom("transport")
            .exact_size(H + TOP)
            .show_separator_line(false)
            .frame(egui::Frame::NONE.inner_margin(Margin { left: 12, right: 12, top: TOP as i8, bottom: 0 }))
            .show(ui, |ui| {
                let card = glass_ui::glass_frame(&ctx).corner_radius(9).inner_margin(Margin::symmetric(10, 0)).show(ui, |ui| {
                    ui.set_height(H - 2.0);
                    ui.set_width(ui.available_width());
                    ui.horizontal_centered(|ui| {
                        ui.spacing_mut().item_spacing.x = 4.0;
                        if cine {
                            self.cine_clock(ui);
                        } else {
                            self.run(ui);
                        }
                    });
                });
                glass_ui::finish(&ctx, &card.response);
            })
            .response;
        r.rect.height()
    }

    /// The pause button: the action's icon and the state's word, lit while paused (Pause / Paused).
    fn pause(&mut self, ui: &mut Ui) {
        let paused = self.dbg.paused;
        let tip = if paused { "resume" } else { "pause" };
        let r = button(ui, if paused { ic::PLAY } else { ic::PAUSE }, Some(if paused { "Paused" } else { "Pause" }), paused);
        if r.on_hover_text(format!("{tip}  [P]")).clicked() {
            self.acts.push((Act::Pause, 0));
        }
    }

    /// The run's controls: rewind and forward act while held (the sim then returns to what it was doing).
    fn run(&mut self, ui: &mut Ui) {
        let back = button(ui, ic::REWIND, None, false).on_hover_text("hold: run the race backwards  [, key]");
        self.pause(ui);
        if button(ui, ic::STEP, None, false).on_hover_text("step one frame  [.]").clicked() {
            self.acts.push((Act::Step, 0));
        }
        let fwd = button(ui, ic::FORWARD, None, false).on_hover_text("hold: run the race ahead at x4  [/ key]");
        if back.is_pointer_button_down_on() {
            *self.scrub = -1;
        } else if fwd.is_pointer_button_down_on() {
            *self.scrub = 1;
        }
        sep(ui);
        // the speed sits at the right edge: lay it out first, right to left, so the timeline takes the rest
        let mut scale = None;
        let sess = self.sess;
        let seek = &mut *self.seek;
        let cur = self.dbg.scale;
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            scale = speed(ui, cur);
            ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                if let Some(span) = sess.and_then(|x| x.timeline.as_ref()).map(|t| t.span()) {
                    let left = format!("{} / {}", clock(span.1), clock(span.2));
                    if let Some(t) = glass_ui::timeline_strip(ui, span, &left, "rec") {
                        *seek = Some(t);
                    }
                }
            });
        });
        if let Some(n) = scale {
            self.dbg.scale = n;
        }
    }

    /// The Cine editor's clock (the shelf's Cine > Clock group): Start, previous / next shot, the timeline.
    fn cine_clock(&mut self, ui: &mut Ui) {
        self.pause(ui);
        let Some(ed) = self.cine.as_deref_mut() else { return };
        if button(ui, ic::RESTART, None, false).on_hover_text("Start (the clock to 0)").clicked() {
            ed.seek(0.0);
        }
        if button(ui, ic::REWIND, None, false).on_hover_text("Previous shot").clicked() {
            ed.shot_step(-1);
        }
        if button(ui, ic::FORWARD, None, false).on_hover_text("Next shot").clicked() {
            ed.shot_step(1);
        }
        sep(ui);
        let end = ed.slider_end(self.sess);
        let race = ed.race_secs();
        let behind = if ed.behind { " · ahead" } else { "" };
        let cur = self.dbg.scale;
        let t = ed.t;
        let mut scale = None;
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            scale = speed(ui, cur);
            ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                let left = format!("{} / {}", clock(t as f64), clock(end as f64));
                if let Some(s) = glass_ui::timeline_strip(ui, (0.0, t as f64, end.max(0.01) as f64), &left, &format!("race {}{behind}", clock(race as f64))) {
                    ed.seek(s as f32);
                }
            });
        });
        if let Some(n) = scale {
            self.dbg.scale = n;
        }
    }
}

/// The hairline between the buttons and the timeline.
fn sep(ui: &mut Ui) {
    ui.add_space(4.0);
    glass_ui::status_sep(ui);
    ui.add_space(4.0);
}

/// A strip button (26 pt high): an icon, with a word after it when given; `on` = lit blue like a shelf tool.
fn button(ui: &mut Ui, icon: &str, word: Option<&str>, on: bool) -> Response {
    let font = FontId::proportional(12.0);
    let w = word.map(|w| ui.painter().layout_no_wrap(w.to_string(), font.clone(), theme::TEXT));
    let width = 30.0 + w.as_ref().map_or(0.0, |g| g.size().x + 4.0);
    let (rect, r) = ui.allocate_exact_size(Vec2::new(width, 26.0), Sense::click());
    let hot = r.hovered() || r.is_pointer_button_down_on();
    let (fill, rim) = match (on, hot) {
        (true, _) => (glass_ui::s1(if hot { 0.2 } else { 0.14 }), glass_ui::s1(0.75)),
        (_, true) => (theme::white(0.05), theme::line(0.32)),
        _ => (Color32::TRANSPARENT, Color32::TRANSPARENT),
    };
    let ink = if on { theme::S1 } else { theme::TEXT };
    ui.painter().rect(rect, theme::RADIUS_TOOL, fill, Stroke::new(1.0, rim), StrokeKind::Inside);
    let icon_x = if w.is_some() { rect.left() + 15.0 } else { rect.center().x };
    ui.painter().text(egui::pos2(icon_x, rect.center().y), Align2::CENTER_CENTER, icon, FontId::proportional(15.0), ink);
    if let Some(g) = w {
        let pos = egui::pos2(rect.left() + 28.0, rect.center().y - g.size().y / 2.0);
        ui.painter().galley(pos, g, ink);
    }
    r
}

/// The speed dropdown (`TIME_SCALES`, 7 options): a box with the clock icon and `x1`; returns the picked index.
fn speed(ui: &mut Ui, cur: usize) -> Option<usize> {
    let names: Vec<String> = TIME_SCALES.iter().map(|x| format!("x{x}")).collect();
    let refs: Vec<&str> = names.iter().map(String::as_str).collect();
    let cur = cur.min(refs.len() - 1);
    let (rect, r) = ui.allocate_exact_size(Vec2::new(86.0, 26.0), Sense::click());
    let open = glass_ui::list_is_open(ui.ctx(), &r);
    let lit = r.hovered() || open;
    let rim = if lit { Stroke::new(1.0, theme::ORANGE) } else { Stroke::new(1.0, theme::line(0.16)) };
    ui.painter().rect(rect, theme::RADIUS_TOOL, Color32::from_rgba_unmultiplied(2, 8, 18, 89), rim, StrokeKind::Inside);
    let cy = rect.center().y;
    ui.painter().text(egui::pos2(rect.left() + 15.0, cy), Align2::CENTER_CENTER, ic::CLOCK, FontId::proportional(14.0), theme::DIM);
    ui.painter().text(egui::pos2(rect.left() + 30.0, cy), Align2::LEFT_CENTER, refs[cur], FontId::monospace(12.0), theme::WHITE);
    ui.painter().text(egui::pos2(rect.right() - 12.0, cy), Align2::CENTER_CENTER, ic::CHEVRON_DOWN, FontId::proportional(13.0), if lit { theme::ORANGE } else { theme::DIM });
    let r = r.on_hover_text("time scale  [- =]");
    let hold = glass_ui::is_held(ui.ctx(), &["Speed", "Time scale"]);
    glass_ui::pick_list(ui, &r, &refs, cur, glass_ui::ListOpts { wheel: true, hold, width: Some(120.0), ..Default::default() })
}
