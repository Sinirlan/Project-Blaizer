//! DEBUG motorview's overlays: the F3 info, sim and render panels, the browser and pinned-target
//! columns, gizmo labels, the crosshair and the exit dialog. Text is egui's JetBrains Mono (theme.rs)
//! on a background layer under the ribbon; only the texture preview stays in the UI batch,
//! its atlas image lives there. The game does not use any of it.

use egui::{Align2, Color32, FontId, Id, LayerId, Order, Pos2, Rect, Vec2};
use glam::Vec3;
use motorview::debug;
use motorview::render::Camera;
use motorview::glass_ui;
use motorview::theme;
use motorview::wire::Span;
use motorview::ui::{Assets, Batch, Canvas, UiVertex, WHITE};

/// Texture preview size in overlay pixels (at 720 lines).
pub const PREVIEW: f32 = 192.0;

/// One overlay line as runs of text, each plain (white) or in its own colour.
#[derive(Clone, Default)]
pub struct Line(pub Vec<Span>);

impl From<String> for Line {
    fn from(s: String) -> Line {
        Line(vec![(s, None)])
    }
}

impl From<Vec<Span>> for Line {
    fn from(v: Vec<Span>) -> Line {
        Line(v)
    }
}

impl Line {
    /// The text without the colours.
    pub fn text(&self) -> String {
        self.0.iter().map(|s| s.0.as_str()).collect()
    }

    fn all(lines: &[String]) -> Vec<Line> {
        lines.iter().cloned().map(Line::from).collect()
    }
}

/// A telemetry row's value.
#[derive(Clone)]
pub enum Val {
    /// a game-axes position, X / Y / Z tinted; the dim suffix after it (a distance)
    Xyz([f32; 3], Option<String>),
    /// the scene clock: seconds and prop frame
    Time(f64, f32),
    Text(String),
}

/// One row of the telemetry card (the F3 overlay): its caption and value, and the plain line it was in the old
/// text overlay (`Telem::lines`, the pinned target).
#[derive(Clone)]
pub struct Row {
    pub cap: &'static str,
    pub val: Val,
    pub line: String,
}

/// The F3 overlay as a card (concept A `telemetryBody`).
#[derive(Clone, Default)]
pub struct Telem {
    pub fps: Option<f32>,
    pub track: u32,
    pub rows: Vec<Row>,
}

impl Telem {
    /// The overlay lines the card replaced, in their old text form.
    pub fn lines(&self) -> Vec<String> {
        let head = format!("{}   track {}", self.fps.map_or("FPS -".into(), |f| format!("FPS {f:.0}")), self.track);
        std::iter::once(head).chain(self.rows.iter().map(|r| r.line.clone())).collect()
    }
}

impl Telem {
    /// The card as Markdown for the copy button: the FPS line, a caption / value table, then the extra lines as a list.
    pub fn markdown(&self, extra: &[Line]) -> String {
        let xyz = |v: &[f32; 3]| format!("{:.2}, {:.2}, {:.2}", v[0], v[1], v[2]);
        let mut out = format!("**{}** · track {}\n\n| | |\n|---|---|\n", self.fps.map_or("FPS -".into(), |f| format!("FPS {f:.0}")), self.track);
        for r in &self.rows {
            let val = match &r.val {
                Val::Xyz(v, Some(s)) => format!("{} ({s})", xyz(v)),
                Val::Xyz(v, None) => xyz(v),
                Val::Time(t, f) => format!("{t:.3} s, prop frame {f:.1}"),
                Val::Text(s) => s.clone(),
            };
            out += &format!("| {} | {} |\n", r.cap, val.replace('|', "\\|"));
        }
        for l in extra {
            let text: String = l.0.iter().map(|(t, _)| t.as_str()).collect();
            out += &format!("\n- {text}");
        }
        out
    }
}

/// What the debug overlay shows besides the crosshair.
#[derive(Default)]
pub struct OverlayText {
    /// the telemetry card (F3), top left under the toolbar
    pub telem: Option<Telem>,
    /// top left: lines after the card (or the whole F3 block when there is no card)
    pub info: Vec<Line>,
    /// the card as Markdown, what its copy button copies
    pub copy_md: String,
    /// bottom left
    pub panel: Vec<String>,
    /// right column from the bottom up (browser, pinned target, render panel)
    pub right: Vec<Vec<String>>,
    /// draw the texture preview (atlas image "preview") above the right column
    pub preview: bool,
    /// gizmo labels: view-space point, text, colour
    pub labels: Vec<(Vec3, String, [f32; 4])>,
}

impl OverlayText {
    /// Nothing for the egui layer to draw (the preview image is the batch's).
    pub fn is_empty(&self) -> bool {
        self.telem.is_none() && self.info.is_empty() && self.panel.is_empty() && self.labels.is_empty() && self.right.iter().all(|r| r.is_empty())
    }
}

/// The overlay's scale: whole multiples of the 720 line layout.
fn scale(size: (u32, u32)) -> f32 {
    (size.1 as f32 / 720.0).round().max(1.0)
}

/// The texture preview in window pixels (the image is a UI atlas entry), right-aligned above
/// the stacked right column (`t.right`, drawn by `paint`).
pub fn preview(assets: &Assets, size: (u32, u32), t: &OverlayText, bottom: f32) -> (Canvas, Vec<UiVertex>) {
    let canvas = Canvas::window(size);
    let mut b = Batch::new(assets, canvas);
    let sc = scale(size);
    let p = PREVIEW * sc;
    let stack: f32 = t.right.iter().filter(|l| !l.is_empty()).map(|l| l.len() as f32 * 18.0 * sc + 6.0 * sc + 4.0).sum();
    let (x, y) = (size.0 as f32 - 4.0 - p - 8.0 * sc, size.1 as f32 - bottom - 4.0 - stack - p - 8.0 * sc);
    b.rect(x, y, p + 8.0 * sc, p + 8.0 * sc, [0.0, 0.0, 0.0, 0.6]);
    b.image_scaled("preview", x + 4.0 * sc, y + 4.0 * sc, p, p, WHITE);
    (canvas, b.verts)
}

fn shade(a: u8) -> Color32 {
    Color32::from_black_alpha(a)
}

fn colour(c: [f32; 4]) -> Color32 {
    let k = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    Color32::from_rgba_unmultiplied(k(c[0]), k(c[1]), k(c[2]), k(c[3]))
}

struct Ink<'a> {
    p: &'a egui::Painter,
    font: FontId,
    /// line height and one overlay pixel at the scale, in points
    lh: f32,
    u: f32,
}

impl Ink<'_> {
    fn width(&self, s: &str) -> f32 {
        self.p.layout_no_wrap(s.to_string(), self.font.clone(), Color32::WHITE).size().x
    }

    /// The runs of a line as one galley (white for the plain ones).
    fn galley(&self, l: &Line) -> std::sync::Arc<egui::Galley> {
        let mut job = egui::text::LayoutJob::default();
        for (text, c) in &l.0 {
            job.append(text, 0.0, egui::TextFormat { font_id: self.font.clone(), color: c.map_or(Color32::WHITE, colour), ..Default::default() });
        }
        self.p.layout_job(job)
    }

    /// A shaded panel of lines; `at` gets its size and returns its top left.
    fn block(&self, lines: &[Line], at: impl FnOnce(Vec2) -> Pos2) -> Pos2 {
        let galleys: Vec<_> = lines.iter().map(|l| self.galley(l)).collect();
        let w = galleys.iter().map(|g| g.size().x).fold(0.0, f32::max) + 12.0 * self.u;
        let h = lines.len() as f32 * self.lh + 6.0 * self.u;
        let pos = at(Vec2::new(w, h));
        self.p.rect_filled(Rect::from_min_size(pos, Vec2::new(w, h)), 0.0, shade(153));
        for (i, g) in galleys.into_iter().enumerate() {
            self.p.galley(pos + Vec2::new(6.0 * self.u, 3.0 * self.u + i as f32 * self.lh), g, Color32::WHITE);
        }
        pos
    }
}

/// Draw the overlay text and the crosshair; `size` is the window in pixels, `fov_x` the view's field (labels), `margins` (top,
/// right, bottom, left) the egui panels' pixels. True when the F3 card's copy button was clicked.
pub fn paint(ctx: &egui::Context, size: (u32, u32), cam: &Camera, fov_x: f32, t: &OverlayText, crosshair: bool, (top, right, bottom, left): (f32, f32, f32, f32)) -> bool {
    let ppp = ctx.pixels_per_point();
    let u = scale(size) / ppp;
    let painter = ctx.layer_painter(LayerId::new(Order::Background, Id::unique("motorview-overlay")));
    let ink = Ink { p: &painter, font: FontId::monospace(13.0 * u), lh: 18.0 * u, u };
    let (w, h) = (size.0 as f32 / ppp, size.1 as f32 / ppp);
    let (top, right, bottom, left) = (top / ppp, right / ppp, bottom / ppp, left / ppp);
    for (p, text, c) in &t.labels {
        let Some((x, y)) = debug::project(cam, size, fov_x, *p) else { continue };
        let (x, y) = (x / ppp, y / ppp);
        // a point under the ribbon, the panes or the status bar has no label
        if x < left || x > w - right || y < top || y > h - bottom {
            continue;
        }
        let tw = ink.width(text);
        painter.rect_filled(Rect::from_min_size(Pos2::new(x - tw / 2.0 - 2.0 * u, y - ink.lh / 2.0), Vec2::new(tw + 4.0 * u, ink.lh)), 0.0, shade(115));
        painter.text(Pos2::new(x - tw / 2.0, y - ink.lh / 2.0 + u), Align2::LEFT_TOP, text, ink.font.clone(), colour(*c));
    }
    let mut copy = false;
    if let Some(tl) = &t.telem {
        copy = card(ctx, tl, &t.info, Pos2::new(left + 12.0, top + 2.0));
    } else if !t.info.is_empty() {
        ink.block(&t.info, |_| Pos2::new(left + 4.0, top + 4.0));
    }
    if !t.panel.is_empty() {
        ink.block(&Line::all(&t.panel), |s| Pos2::new(left + 4.0, h - bottom - 4.0 - s.y));
    }
    // the right column, stacked from the bottom
    let mut bottom_y = h - bottom - 4.0;
    for lines in t.right.iter().filter(|l| !l.is_empty()) {
        let y = ink.block(&Line::all(lines), |s| Pos2::new(w - right - 4.0 - s.x, bottom_y - s.y)).y;
        bottom_y = y - 4.0;
    }
    if crosshair {
        let (c, r, th) = (Pos2::new(w / 2.0, h / 2.0), 8.0 * u, 2.0 * u);
        for (col, o) in [(shade(153), u), (Color32::WHITE, 0.0)] {
            painter.rect_filled(Rect::from_center_size(c, Vec2::new(2.0 * (r + o), th + 2.0 * o)), 0.0, col);
            painter.rect_filled(Rect::from_center_size(c, Vec2::new(th + 2.0 * o, 2.0 * (r + o))), 0.0, col);
        }
    }
    copy
}

/// The telemetry card (`.telem`, F3): the FPS in Rajdhani Bold 30, then one row per value with its dim caption; `extra`
/// lines (shading view, wire counts, the note) follow in mono. At `pos`, moved under the toolbar's open flyout when
/// that would cover it. True when its copy button (top right) was clicked.
fn card(ctx: &egui::Context, t: &Telem, extra: &[Line], mut pos: Pos2) -> bool {
    let fly = crate::ribbon::fly_rect();
    if fly != Rect::NOTHING && fly.left() < pos.x + CARD_W && fly.right() > pos.x {
        pos.y = pos.y.max(fly.bottom() + 12.0);
    }
    let mut copy = false;
    egui::Area::new(Id::unique("motorview-telemetry")).order(Order::Middle).interactable(true).fixed_pos(pos).show(ctx, |ui| {
        let fr = glass_ui::glass_frame(ctx).inner_margin(egui::Margin { left: 14, right: 14, top: 10, bottom: 10 }).show(ui, |ui| {
            ui.set_width(CARD_W - 28.0);
            ui.spacing_mut().item_spacing = Vec2::ZERO;
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new(t.fps.map_or("-".into(), |f| format!("{f:.0}"))).font(theme::glass_font(30.0)).color(theme::WHITE));
                ui.add_space(4.0);
                ui.label(egui::RichText::new("FPS").size(11.0).color(theme::DIM));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    copy = glass_ui::ibtn(ui, motorview::icons::COPY, false).on_hover_text("Copy as Markdown").clicked();
                    ui.add_space(6.0);
                    let ms = t.fps.filter(|f| *f > 0.5).map_or(String::new(), |f| format!("{:.1} MS · ", 1000.0 / f));
                    ui.label(egui::RichText::new(format!("{ms}TRACK {}", t.track)).size(11.0).color(theme::DIM));
                });
            });
            ui.add_space(6.0);
            let mono = FontId::monospace(12.5);
            for (i, r) in t.rows.iter().enumerate() {
                if i > 0 {
                    let (l, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 1.0), egui::Sense::hover());
                    ui.painter().rect_filled(l, 0.0, theme::line(0.07));
                }
                ui.horizontal(|ui| {
                    ui.set_height(21.0);
                    let (c, _) = ui.allocate_exact_size(Vec2::new(62.0, 21.0), egui::Sense::hover());
                    ui.painter().text(Pos2::new(c.left(), c.center().y), Align2::LEFT_CENTER, r.cap.to_uppercase(), FontId::proportional(10.0), theme::DIM);
                    match &r.val {
                        Val::Xyz(v, suffix) => {
                            for (k, x) in v.iter().enumerate() {
                                ui.label(egui::RichText::new(["X", "Y", "Z"][k]).font(theme::glass_font(10.0)).color(theme::AXIS[k]));
                                ui.add_space(3.0);
                                ui.label(egui::RichText::new(format!("{x:.2}")).font(mono.clone()).color(theme::WHITE));
                                ui.add_space(10.0);
                            }
                            if let Some(s) = suffix {
                                ui.label(egui::RichText::new(s).font(mono.clone()).color(theme::DIM));
                            }
                        }
                        Val::Time(t, f) => {
                            ui.label(egui::RichText::new(format!("{t:.3} s")).font(mono.clone()).color(theme::WHITE));
                            ui.add_space(10.0);
                            ui.label(egui::RichText::new("prop frame").font(mono.clone()).color(theme::DIM));
                            ui.add_space(6.0);
                            ui.label(egui::RichText::new(format!("{f:.1}")).font(mono.clone()).color(theme::WHITE));
                        }
                        Val::Text(s) => {
                            ui.add(egui::Label::new(egui::RichText::new(s).size(13.0).color(theme::TEXT)).truncate());
                        }
                    }
                });
            }
            for l in extra {
                ui.add_space(4.0);
                let mut job = egui::text::LayoutJob::default();
                for (text, c) in &l.0 {
                    job.append(text, 0.0, egui::TextFormat { font_id: FontId::monospace(11.5), color: c.map_or(theme::DIM, colour), ..Default::default() });
                }
                ui.add(egui::Label::new(job).wrap());
            }
        });
        glass_ui::finish(ctx, &fr.response);
    });
    copy
}

/// The telemetry card's outer width, points (`.telem` in a 340 pt card).
const CARD_W: f32 = 340.0;

/// What the exit requester decided.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Exit {
    /// save the World Editor's fill, then exit (only offered with unsaved edits)
    Save,
    /// exit; unsaved edits are lost
    Discard,
    Cancel,
}

/// The exit requester (19.3, `requesterHTML`): a near-opaque card over a dark scrim, "EXIT MOTORVIEW?", and three
/// buttons when the World Editor has unsaved edits (`unsaved` = its file name: Save & exit, the default, Discard,
/// Cancel), else Exit (the default) and Cancel. None = still asking. Keys (Enter / Y, D, Esc / N) are the viewer's own.
pub fn exit_dialog(ctx: &egui::Context, unsaved: Option<&str>) -> Option<Exit> {
    let mut out = None;
    let frame = egui::Frame::new()
        .fill(Color32::from_rgba_unmultiplied(16, 28, 45, 247))
        .stroke(egui::Stroke::new(1.0, theme::line(0.5)))
        .corner_radius(14)
        .shadow(egui::epaint::Shadow { offset: [0, 18], blur: 48, spread: 0, color: Color32::from_black_alpha(153) })
        .inner_margin(egui::Margin { left: 22, right: 22, top: 18, bottom: 22 });
    let m = egui::Modal::new(Id::unique("motorview-exit")).frame(frame).backdrop_color(Color32::from_rgba_unmultiplied(2, 5, 10, 140)).show(ctx, |ui| {
        ui.set_width(584.0);
        ui.spacing_mut().item_spacing = Vec2::ZERO;
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new(motorview::icons::WARN).size(26.0).color(theme::ORANGE_TEXT));
            ui.add_space(12.0);
            ui.label(egui::RichText::new("EXIT MOTORVIEW?").font(theme::glass_font(28.0)).color(theme::WHITE));
        });
        ui.add_space(14.0);
        let text = match unsaved {
            Some(f) => format!("The world editor has unsaved changes in {f}. Save them before the viewer closes? Enter / Y: save and exit, D: discard, Esc / N: cancel."),
            None => "Enter / Y: exit, Esc / N: cancel.".to_string(),
        };
        ui.add(egui::Label::new(egui::RichText::new(text).size(15.0).color(theme::TEXT)));
        ui.add_space(24.0);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 16.0;
            if unsaved.is_some() {
                if glass_ui::req_button(ui, motorview::icons::SAVE, "SAVE & EXIT", true).clicked() {
                    out = Some(Exit::Save);
                }
                if glass_ui::req_button(ui, motorview::icons::EXIT, "DISCARD", false).clicked() {
                    out = Some(Exit::Discard);
                }
            } else if glass_ui::req_button(ui, motorview::icons::EXIT, "EXIT", true).clicked() {
                out = Some(Exit::Discard);
            }
            if glass_ui::req_button(ui, motorview::icons::CLOSE, "CANCEL", false).clicked() {
                out = Some(Exit::Cancel);
            }
        });
    });
    out.or_else(|| m.should_close().then_some(Exit::Cancel))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn markdown_card() {
        let t = Telem {
            fps: Some(59.6),
            track: 2,
            rows: vec![
                Row { cap: "camera", val: Val::Xyz([1.0, 2.5, -3.0], None), line: String::new() },
                Row { cap: "speed", val: Val::Text("x1.00 · 15.0 m/s".into()), line: String::new() },
                Row { cap: "target", val: Val::Xyz([0.0, 0.0, 0.0], Some("12.3 m".into())), line: String::new() },
                Row { cap: "time", val: Val::Time(1.5, 3.0), line: String::new() },
            ],
        };
        let md = t.markdown(&[Line::from("shading view: normals (F10)".to_string())]);
        assert_eq!(
            md,
            "**FPS 60** · track 2\n\n| | |\n|---|---|\n| camera | 1.00, 2.50, -3.00 |\n| speed | x1.00 · 15.0 m/s |\n| target | 0.00, 0.00, 0.00 (12.3 m) |\n| time | 1.500 s, prop frame 3.0 |\n\n- shading view: normals (F10)"
        );
    }
}
