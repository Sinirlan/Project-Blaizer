//! DEBUG motorview's status bar (re-notes/15-motorview-overhaul.md phase 2): a thin bottom
//! panel. Left: what the mouse and keys do now. Middle: the latest note. Right: FPS, sim time
//! and the paused state.

use motorview::theme;
use egui::{Ui, RichText};
use std::sync::atomic::{AtomicBool, Ordering};

pub struct StatusBar<'a> {
    pub hint: String,
    pub note: &'a str,
    pub fps: f32,
    /// the sim's state text (`DebugState::status`)
    pub sim: String,
    /// a background job (the Cine render): its text, progress 0..1 (None: not known) and the flag its cancel button sets
    pub job: Option<(String, Option<f32>, &'a AtomicBool)>,
}

impl StatusBar<'_> {
    /// The bar along the bottom edge (add it before the side panels); its height in points.
    pub fn show(&self, ui: &mut Ui) -> f32 {
        egui::Panel::bottom("status")
            .exact_size(22.0)
            .frame(egui::Frame::new().fill(theme::PANEL_DARK).inner_margin(egui::Margin::symmetric(8, 2)))
            .show(ui, |ui| {
                ui.horizontal_centered(|ui| {
                    ui.label(RichText::new(&self.hint).color(theme::TEXT_WEAK));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(RichText::new(format!("{:.0} fps   {}", self.fps, self.sim)).color(theme::TEXT_WEAK));
                        if let Some((text, part, cancel)) = &self.job {
                            ui.separator();
                            if ui.small_button("cancel").clicked() {
                                cancel.store(true, Ordering::Relaxed);
                            }
                            if let Some(p) = part {
                                ui.add(egui::ProgressBar::new(*p).desired_width(120.0).desired_height(10.0));
                            }
                            ui.label(text);
                        }
                        if !self.note.is_empty() {
                            ui.separator();
                            ui.label(self.note);
                        }
                    });
                });
            })
            .response
            .rect
            .height()
    }
}
