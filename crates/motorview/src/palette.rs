//! DEBUG World Editor chrome (re-notes/15-motorview-overhaul.md "Glass World Editor"): the tool palette (gizmo
//! modes, place tools, duplicate / delete) and the tool options strip, two floating glass cards under the
//! toolbar (concept A `#palette` and `#opts`). Both only push `Cmd`s and set `WorldEdit` state; the editor
//! logic is in world_edit.rs. They are `Area`s, not panels, so only the cards take the mouse and the dock does
//! not move when the strip opens.

use crate::fill_gizmo::Mode;
use crate::glass_ui;
use crate::icons as ic;
use crate::theme;
use crate::world_edit::{kind_icon, Cmd, Kind, WorldEdit};
use egui::{Area, FontId, Id, Margin, Order, Pos2, RichText, Ui, Vec2};

/// The palette card's width (`#palette`), points.
const W: f32 = 56.0;
/// The gap between the cards and the window edge / each other.
const GAP: f32 = 12.0;

/// The palette at the left edge, `top` points down (under the toolbar) and `bottom` points clear of the window's
/// bottom edge (the status bar); it scrolls when the window is too short for all its tools. Returns its right edge
/// in points (the card and its gap), for the overlay labels to keep clear of it.
pub fn palette(ui: &mut Ui, w: &mut WorldEdit, top: f32, bottom: f32) -> f32 {
    let ctx = ui.ctx().clone();
    // the tools' own height last frame sizes the scroll area (the first frame shows it at the room it has)
    let room = (ctx.content_rect().height() - top - bottom - 12.0 - 12.0).max(120.0);
    let hid = Id::unique("world_palette_h");
    let vis = ctx.data(|d| d.get_temp::<f32>(hid)).unwrap_or(f32::INFINITY).min(room);
    let r = Area::new(Id::unique("world_palette")).order(Order::Middle).fixed_pos(Pos2::new(GAP, top)).constrain(false).show(&ctx, |ui| {
        let fr = glass_ui::glass_frame(&ctx).inner_margin(Margin::symmetric(0, 6)).show(ui, |ui| {
            ui.set_width(W);
            ui.spacing_mut().item_spacing.y = 3.0;
            let mut cmd = None;
            let out = egui::ScrollArea::vertical().max_height(vis).min_scrolled_height(vis).auto_shrink([false, false]).show(ui, |ui| {
                ui.vertical_centered(|ui| {
                    for (m, icon, tip) in [(Mode::Handles, ic::MOUSE_POINTER, "Select (handles)  [Ctrl+Q]"), (Mode::Move, ic::MOVE, "Move gizmo  [Ctrl+W]"), (Mode::Rotate, ic::ROTATE, "Rotate gizmo  [Ctrl+T]"), (Mode::Scale, ic::SCALE, "Scale gizmo  [Ctrl+B]")] {
                        if glass_ui::tool_sm(ui, icon, w.mode == m && w.armed().is_none(), false).on_hover_text(tip).clicked() {
                            cmd = Some(Cmd::Gizmo(m));
                        }
                    }
                    glass_ui::palette_sep(ui);
                    for k in Kind::ALL {
                        let on = w.armed() == Some(k);
                        if glass_ui::tool_sm(ui, kind_icon(k), false, on).on_hover_text(format!("place {}  [Ctrl+{}]", k.name(), k as usize + 1)).clicked() {
                            if on {
                                w.disarm();
                            } else {
                                cmd = Some(Cmd::Start(k));
                            }
                        }
                    }
                    glass_ui::palette_sep(ui);
                    ui.add_enabled_ui(w.sel.is_some(), |ui| {
                        if glass_ui::tool_sm(ui, ic::COPY, false, false).on_hover_text("duplicate  [Ctrl+D]").clicked() {
                            cmd = Some(Cmd::Duplicate);
                        }
                        if glass_ui::tool_sm(ui, ic::TRASH, false, false).on_hover_text("delete  [Delete]").clicked() {
                            cmd = Some(Cmd::Delete);
                        }
                    });
                });
            });
            ctx.data_mut(|d| d.insert_temp(hid, out.content_size.y));
            w.cmds.extend(cmd);
        });
        glass_ui::finish(&ctx, &fr.response);
        fr.response.rect
    });
    r.inner.right() + GAP
}

/// A strip button (`Remove last`, `Finish`, `Cancel`): icon and Rajdhani capitals.
fn strip_button(ui: &mut Ui, icon: &str, text: &str, enabled: bool, tip: &str) -> bool {
    ui.add_enabled(enabled, egui::Button::new(RichText::new(format!("{icon}  {}", text.to_uppercase())).font(theme::glass_font(14.0))).min_size(Vec2::new(0.0, 30.0))).on_hover_text(tip).clicked()
}

/// The tool options strip, `left` points from the window edge and `top` down, while a place tool is armed; at most
/// `max_w` wide (it scrolls sideways beyond). Nothing is drawn when no tool is armed.
pub fn options(ui: &mut Ui, w: &mut WorldEdit, textures: &[String], top: f32, left: f32, max_w: f32) {
    let Some(kind) = w.armed() else { return };
    let ctx = ui.ctx().clone();
    Area::new(Id::unique("world_options")).order(Order::Middle).fixed_pos(Pos2::new(left, top)).show(&ctx, |ui| {
        let fr = glass_ui::glass_frame(&ctx).inner_margin(Margin::symmetric(14, 6)).show(ui, |ui| {
            // the strip's own width last frame sizes the scroll area (it scrolls only when `max_w` is short of it)
            let wid = Id::unique("world_options_w");
            let last: f32 = ctx.data(|d| d.get_temp(wid)).unwrap_or(f32::INFINITY);
            let vis = last.min((max_w - 28.0).max(200.0));
            egui::ScrollArea::horizontal().auto_shrink([false, true]).max_width(vis).min_scrolled_width(vis).show(ui, |ui| {
                let used = ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 10.0;
                    ui.spacing_mut().interact_size.y = 32.0;
                    let title = if kind == Kind::Plant { "plant brush".to_string() } else { kind.name().to_string() };
                    ui.label(RichText::new(format!("{}  {}", kind_icon(kind), title.to_uppercase())).font(theme::glass_font(15.0)).color(theme::ORANGE));
                    glass_ui::vsep(ui, 24.0);
                    match kind {
                        Kind::Plant => w.brush_ui(ui, textures),
                        Kind::Water => {
                            ui.label(RichText::new("click the shore points; the water level is the last click's height").size(13.0).color(theme::DIM));
                        }
                        _ => {
                            glass_ui::cap_label(ui, "points");
                            ui.label(RichText::new(w.points().to_string()).font(FontId::monospace(13.0)).color(theme::WHITE));
                        }
                    }
                    if kind != Kind::Plant {
                        glass_ui::vsep(ui, 24.0);
                        if strip_button(ui, ic::UNDO, "Remove last", w.points() > 0, "Backspace") {
                            w.cmds.push(Cmd::Back);
                        }
                        if strip_button(ui, ic::CIRCLE_DOT, "Finish", true, "Enter") {
                            w.cmds.push(Cmd::Finish);
                        }
                    }
                    if strip_button(ui, ic::CLOSE, "Cancel", true, "Esc") {
                        w.disarm();
                    }
                })
                .response
                .rect
                .width();
                ctx.data_mut(|d| d.insert_temp(wid, used + 4.0));
            });
        });
        glass_ui::finish(&ctx, &fr.response);
    });
}
