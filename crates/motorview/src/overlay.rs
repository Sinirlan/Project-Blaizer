//! DEBUG motorview's overlays: the F3 info, sim and render panels, the browser and pinned-target
//! columns, gizmo labels, the crosshair and the exit dialog. Text is egui's JetBrains Mono (theme.rs)
//! on a background layer under the ribbon; only the texture preview stays in the UI batch,
//! its atlas image lives there. The game does not use any of it.

use egui::{Align2, Color32, FontId, Id, LayerId, Order, Pos2, Rect, Vec2};
use glam::Vec3;
use motorview::debug;
use motorview::render::Camera;
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

/// What the debug overlay shows besides the crosshair.
#[derive(Default)]
pub struct OverlayText {
    /// top left
    pub info: Vec<Line>,
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
        self.info.is_empty() && self.panel.is_empty() && self.labels.is_empty() && self.right.iter().all(|r| r.is_empty())
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
/// right, bottom, left) the egui panels' pixels.
pub fn paint(ctx: &egui::Context, size: (u32, u32), cam: &Camera, fov_x: f32, t: &OverlayText, crosshair: bool, (top, right, bottom, left): (f32, f32, f32, f32)) {
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
    if !t.info.is_empty() {
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
}

/// The exit requester: Some(true) = exit clicked, Some(false) = cancelled (Cancel, a click
/// outside or Esc), None = still asking. Y / Enter / N keys are the viewer's own.
pub fn exit_dialog(ctx: &egui::Context) -> Option<bool> {
    let mut exit = false;
    let m = egui::Modal::new(Id::unique("motorview-exit")).show(ctx, |ui| {
        ui.set_width(300.0);
        ui.heading("Exit motorview?");
        ui.add_space(4.0);
        ui.label(egui::RichText::new("Enter / Y: exit    Esc / N: cancel").color(theme::TEXT_WEAK));
        ui.add_space(10.0);
        ui.horizontal(|ui| {
            exit = ui.add(egui::Button::selectable(true, "Exit").min_size(Vec2::new(90.0, 26.0))).clicked();
            if ui.add(egui::Button::new("Cancel").min_size(Vec2::new(90.0, 26.0))).clicked() {
                ui.close();
            }
        });
    });
    if exit {
        Some(true)
    } else {
        m.should_close().then_some(false)
    }
}
