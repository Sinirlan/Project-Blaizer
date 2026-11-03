//! DEBUG World Editor chrome (re-notes/15-motorview-overhaul.md phase 3): the left tool palette
//! (transform tools and place tools, like an RTS map editor) and the tool options strip under
//! the ribbon. Both only push `Cmd`s and set `WorldEdit` state; the editor logic is in world_edit.rs.

use crate::fill_gizmo::Mode;
use crate::icons as ic;
use crate::theme;
use crate::world_edit::{kind_icon, Cmd, Kind, WorldEdit};
use egui::{Button, Ui};

const SIDE: f32 = 34.0;

/// The palette along the left edge (add it before the right dock); its width in points.
pub fn palette(ui: &mut Ui, w: &mut WorldEdit) -> f32 {
    egui::Panel::left("world_tools")
        .resizable(false)
        .exact_size(SIDE)
        .frame(egui::Frame::new().fill(theme::PANEL_DARK).inner_margin(egui::Margin::symmetric(3, 4)))
        .show(ui, |ui| {
            ui.vertical_centered(|ui| {
                let mut cmd = None;
                for (m, icon, tip) in [(Mode::Handles, ic::MOUSE_POINTER, "Select (handles)  [Ctrl+Q]"), (Mode::Move, ic::MOVE, "Move gizmo  [Ctrl+W]"), (Mode::Rotate, ic::ROTATE, "Rotate gizmo  [Ctrl+T]"), (Mode::Scale, ic::SCALE, "Scale gizmo  [Ctrl+B]")] {
                    if ui.add_sized([28.0, 28.0], Button::selectable(w.mode == m && w.armed().is_none(), icon)).on_hover_text(tip).clicked() {
                        cmd = Some(Cmd::Gizmo(m));
                    }
                }
                ui.separator();
                for k in Kind::ALL {
                    let on = w.armed() == Some(k);
                    if ui.add_sized([28.0, 28.0], Button::selectable(on, kind_icon(k))).on_hover_text(format!("place {}  [Ctrl+{}]", k.name(), k as usize + 1)).clicked() {
                        if on {
                            w.disarm();
                        } else {
                            cmd = Some(Cmd::Start(k));
                        }
                    }
                }
                ui.separator();
                let has = w.sel.is_some();
                ui.add_enabled_ui(has, |ui| {
                    if ui.add_sized([28.0, 28.0], Button::new(ic::COPY)).on_hover_text("duplicate  [Ctrl+D]").clicked() {
                        cmd = Some(Cmd::Duplicate);
                    }
                    if ui.add_sized([28.0, 28.0], Button::new(ic::TRASH)).on_hover_text("delete  [Delete]").clicked() {
                        cmd = Some(Cmd::Delete);
                    }
                });
                w.cmds.extend(cmd);
            });
        })
        .response
        .rect
        .width()
}

/// The tool options strip (under the ribbon) while a place tool is armed; its height in points,
/// 0 when nothing is armed.
pub fn options(ui: &mut Ui, w: &mut WorldEdit, textures: &[String]) -> f32 {
    let Some(kind) = w.armed() else { return 0.0 };
    egui::Panel::top("world_options")
        .exact_size(30.0)
        .frame(egui::Frame::new().fill(theme::PANEL).inner_margin(egui::Margin::symmetric(8, 3)))
        .show(ui, |ui| {
            ui.horizontal_centered(|ui| {
                ui.label(egui::RichText::new(format!("{}  {}", kind_icon(kind), kind.name())).strong());
                ui.separator();
                match kind {
                    Kind::Plant => w.brush_ui(ui, textures),
                    Kind::Water => {
                        ui.weak("click the shore points; the water level is the last click's height");
                    }
                    _ => {
                        ui.label(format!("{} points", w.points()));
                    }
                }
                if kind != Kind::Plant {
                    ui.separator();
                    if ui.add_enabled(w.points() > 0, Button::new(format!("{}  Remove last", ic::UNDO))).on_hover_text("Backspace").clicked() {
                        w.cmds.push(Cmd::Back);
                    }
                    if ui.add(Button::new(format!("{}  Finish", ic::CIRCLE_DOT))).on_hover_text("Enter").clicked() {
                        w.cmds.push(Cmd::Finish);
                    }
                }
                if ui.add(Button::new(format!("{}  Cancel", ic::CLOSE))).on_hover_text("Esc").clicked() {
                    w.disarm();
                }
            });
        })
        .response
        .rect
        .height()
}
