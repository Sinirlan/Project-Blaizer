//! DEBUG motorview's status bar (re-notes/15-motorview-overhaul.md, Glass #395): a 28 pt glass card along the
//! bottom. Left: what the mouse and keys do now, as key caps. Then the latest note with its dot. Right: the sim's
//! state (blinking while it runs), its speed, frame time, FPS and the render mode.

use egui::{Align, Layout, RichText, Ui};
use motorview::glass_ui::{self, HintPart};
use motorview::icons;
use motorview::theme;
use std::sync::atomic::{AtomicBool, Ordering};

pub struct StatusBar<'a> {
    pub hint: Vec<HintPart>,
    pub note: &'a str,
    /// a tag after the note (the World Editor's "unsaved · 3 changes")
    pub tag: Option<String>,
    pub fps: f32,
    /// the sim is paused / its speed factor (`DebugState`)
    pub paused: bool,
    pub speed: f32,
    /// the render mode's name (Classic / Remaster)
    pub mode: &'static str,
    /// a background job (the Cine render): its text, progress 0..1 (None: not known) and the flag its cancel button sets
    pub job: Option<(String, Option<f32>, &'a AtomicBool)>,
}

/// The card's height and its margins around it (points).
const H: f32 = 28.0;

impl StatusBar<'_> {
    /// The bar along the bottom edge (add it before the side panels); its height in points, margins included.
    pub fn show(&self, ui: &mut Ui) -> f32 {
        let ctx = ui.ctx().clone();
        let r = egui::Panel::bottom("status")
            .exact_size(H + 20.0)
            .show_separator_line(false)
            .frame(egui::Frame::NONE.inner_margin(egui::Margin { left: 12, right: 12, top: 10, bottom: 10 }))
            .show(ui, |ui| {
                let card = glass_ui::glass_frame(&ctx).corner_radius(9).inner_margin(egui::Margin::symmetric(14, 0)).show(ui, |ui| {
                    ui.set_height(H - 2.0);
                    ui.set_width(ui.available_width());
                    ui.horizontal_centered(|ui| {
                        ui.spacing_mut().item_spacing.x = 6.0;
                        glass_ui::hint_row(ui, &self.hint);
                        if let Some((text, part, cancel)) = &self.job {
                            glass_ui::status_sep(ui);
                            ui.label(RichText::new(text).size(13.0).color(theme::TEXT));
                            if let Some(p) = part {
                                ui.add(egui::ProgressBar::new(*p).desired_width(120.0).desired_height(8.0));
                            }
                            if ui.small_button("cancel").clicked() {
                                cancel.store(true, Ordering::Relaxed);
                            }
                        }
                        if !self.note.is_empty() {
                            glass_ui::status_sep(ui);
                            let (dot, _) = ui.allocate_exact_size(egui::vec2(7.0, 7.0), egui::Sense::hover());
                            ui.painter().circle_filled(dot.center(), 5.5, egui::Color32::from_rgb(0x7c, 0xf0, 0xa8).gamma_multiply(0.25));
                            ui.painter().circle_filled(dot.center(), 3.5, egui::Color32::from_rgb(0x7c, 0xf0, 0xa8));
                            ui.label(RichText::new(self.note).size(13.0).color(theme::TEXT));
                        }
                        if let Some(t) = &self.tag {
                            ui.add_space(6.0);
                            tag(ui, t);
                        }
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            ui.spacing_mut().item_spacing.x = 16.0;
                            ui.label(RichText::new(self.mode.to_uppercase()).font(theme::glass_font(13.0)).color(theme::S1));
                            glass_ui::status_sep(ui);
                            let (fps, ms) = if self.fps > 0.5 { (format!("{:.0}", self.fps), format!("{:.1} ms", 1000.0 / self.fps)) } else { ("-".into(), "-".into()) };
                            metric(ui, "FPS", &fps);
                            metric(ui, "FRAME", &ms);
                            glass_ui::status_sep(ui);
                            metric(ui, "SPEED", &format!("x{}", self.speed));
                            // blinks while the sim runs (`hudBlink` 1.6 s)
                            let (text, icon, glow) = if self.paused { ("SIM PAUSED", icons::PAUSE, 1.0) } else { ("SIM RUNNING", icons::PLAY, 0.65 + 0.35 * (std::f64::consts::TAU * ui.input(|i| i.time) / 1.6).cos() as f32) };
                            if !self.paused {
                                ui.ctx().request_repaint();
                            }
                            ui.scope(|ui| {
                                ui.spacing_mut().item_spacing.x = 5.0;
                                ui.label(RichText::new(text).font(theme::glass_font(11.0)).color(theme::ORANGE));
                                ui.label(RichText::new(icon).size(13.0).color(theme::ORANGE.gamma_multiply(glow)));
                            });
                        });
                    });
                });
                glass_ui::finish(&ctx, &card.response);
            })
            .response;
        r.rect.height()
    }
}

/// `FPS 144`: the caption in dim, the value in white mono, right to left.
fn metric(ui: &mut Ui, caption: &str, value: &str) {
    ui.scope(|ui| {
        ui.spacing_mut().item_spacing.x = 5.0;
        ui.label(RichText::new(value).font(egui::FontId::monospace(12.0)).color(theme::WHITE));
        ui.label(RichText::new(caption).size(13.0).color(theme::DIM));
    });
}

/// The unsaved tag (`.tagv`): an orange outlined pill.
fn tag(ui: &mut Ui, text: &str) {
    let g = ui.painter().layout_no_wrap(text.to_uppercase(), theme::glass_font(10.0), theme::ORANGE_TEXT);
    let (rect, _) = ui.allocate_exact_size(egui::vec2(g.size().x + 16.0, 18.0), egui::Sense::hover());
    ui.painter().rect(rect, 7.0, theme::ORANGE.gamma_multiply(0.08), egui::Stroke::new(1.0, theme::ORANGE.gamma_multiply(0.6)), egui::StrokeKind::Inside);
    ui.painter().galley(rect.center() - g.size() / 2.0, g, egui::Color32::PLACEHOLDER);
}

/// How long the sim-state toast lives (seconds): fade in, hold, fade out.
pub const TOAST_SECS: f32 = 1.4;

/// The sim-state toast: a glass pill with the play / pause icon and "SIM RUNNING" / "SIM PAUSED", centred in
/// `x_range` (the viewport between the docks) at `top` (points). `age` is seconds since the change. True while it is
/// still showing.
pub fn sim_toast(ctx: &egui::Context, paused: bool, age: f32, x_range: (f32, f32), top: f32) -> bool {
    if age >= TOAST_SECS {
        return false;
    }
    // .12 s in, .35 s out
    let alpha = (age / 0.12).min(1.0).min((TOAST_SECS - age) / 0.35).clamp(0.0, 1.0);
    let (text, icon) = if paused { ("SIM PAUSED", icons::PAUSE) } else { ("SIM RUNNING", icons::PLAY) };
    let r = egui::Area::new(egui::Id::unique("motorview-sim-toast"))
        .order(egui::Order::Foreground)
        .interactable(false)
        .pivot(egui::Align2::CENTER_TOP)
        .fixed_pos(egui::pos2((x_range.0 + x_range.1) * 0.5, top))
        .show(ctx, |ui| {
            ui.set_opacity(alpha);
            let card = glass_ui::glass_frame(ctx).corner_radius(10).inner_margin(egui::Margin::symmetric(18, 9)).show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 10.0;
                    ui.label(RichText::new(icon).size(18.0).color(theme::ORANGE));
                    ui.label(RichText::new(text).font(theme::glass_font(16.0)).color(theme::ORANGE));
                });
            });
            card.response
        });
    glass_ui::finish(ctx, &r.inner);
    ctx.request_repaint();
    true
}
