//! DEBUG motorview's overlays: the F3 info, sim and render panels, the browser and pinned-target
//! columns, gizmo labels, the crosshair, the tuning Changes card and the exit dialog. Text is egui's JetBrains Mono (theme.rs)
//! on a background layer under the ribbon; only the texture preview stays in the UI batch,
//! its atlas image lives there. The game does not use any of it.

use egui::{Align2, Color32, FontId, Id, LayerId, Order, Pos2, Rect, Vec2};
use glam::Vec3;
use motorview::debug;
use motorview::render::Camera;
use motorview::glass_ui;
use motorview::theme;
use motorview::tune_changes::{Change, Key};
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
    /// labels farther than this (metres, the main view's far distance) are hidden; None: no limit (unlimited distance)
    pub label_far: Option<f32>,
    /// window rectangle (x, y, w, h in pixels) the rear mirror covers: it is a second camera, so no label is drawn over it
    pub label_hide: Option<[f32; 4]>,
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

/// How far (overlay pixels at the 720p scale) past the free area a gizmo label's box can sit and still be pinned to the edge.
const LABEL_PIN: f32 = 60.0;

/// How dark the view outside the Cine preview's frame is (0 = as drawn, 1 = black; #438 design choice, a passepartout).
const PASSEPARTOUT: f32 = 0.6;
/// Points between the Cine preview's frame and a panel beside it.
pub const FRAME_GAP: f32 = 8.0;

/// Slots on egui's background layer for the Cine preview's frame (#438), taken before the panels so they lie over it and
/// filled by `preview_frame` once the panels have said how much of the window they take.
pub struct FrameSlots(Vec<egui::layers::ShapeIdx>);

/// Take the slots (call before the panels): the four dim sides, two letterbox bars and the fade.
pub fn frame_slots(ctx: &egui::Context) -> FrameSlots {
    let painter = ctx.layer_painter(LayerId::background());
    FrameSlots((0..7).map(|_| painter.add(egui::Shape::Noop)).collect())
}

/// The Cine preview's frame (#438) in `slots`: `rect` = `[x, y, w, h]` window pixels (`cine::preview_frame`), the view outside
/// it dimmed, the `letterbox` aspect's black bars inside it (rows as the render's, `letterbox_bar`) and black at `1 - fade` over it
/// (the render multiplies the frame by `fade` in gamma space; a black overlay of that alpha is the same blend).
pub fn preview_frame(ctx: &egui::Context, slots: &FrameSlots, size: (u32, u32), rect: [f32; 4], letterbox: Option<f32>, fade: Option<f32>) {
    let ppp = ctx.pixels_per_point();
    let (w, h) = (size.0 as f32 / ppp, size.1 as f32 / ppp);
    let [x0, y0, rw, rh] = rect.map(|v| v / ppp);
    let (x1, y1) = (x0 + rw, y0 + rh);
    let black = |a: f32| Color32::from_black_alpha((a.clamp(0.0, 1.0) * 255.0).round() as u8);
    let r = |a: Pos2, b: Pos2, c: Color32| egui::Shape::rect_filled(Rect::from_min_max(a, b), 0.0, c);
    let dim = black(PASSEPARTOUT);
    let mut shapes = vec![
        r(Pos2::new(0.0, 0.0), Pos2::new(w, y0), dim),
        r(Pos2::new(0.0, y1), Pos2::new(w, h), dim),
        r(Pos2::new(0.0, y0), Pos2::new(x0, y1), dim),
        r(Pos2::new(x1, y0), Pos2::new(w, y1), dim),
    ];
    let bar = letterbox.map_or(0, |a| motorview::cine::letterbox_bar(rect[2].round() as u32, rect[3].round() as u32, a)) as f32 / ppp;
    if bar > 0.0 {
        shapes.push(r(Pos2::new(x0, y0), Pos2::new(x1, y0 + bar), Color32::BLACK));
        shapes.push(r(Pos2::new(x0, y1 - bar), Pos2::new(x1, y1), Color32::BLACK));
    }
    if let Some(f) = fade.filter(|&f| f < 1.0) {
        shapes.push(r(Pos2::new(x0, y0), Pos2::new(x1, y1), black(1.0 - f)));
    }
    let painter = ctx.layer_painter(LayerId::background());
    for (i, idx) in slots.0.iter().enumerate() {
        painter.set(*idx, shapes.get(i).cloned().unwrap_or(egui::Shape::Noop));
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
        // CLASSIC and REMASTER alike: the main view's camera (`debug::project_any`)
        let Some((x, y, depth)) = debug::project_any(cam, size, fov_x, *p) else { continue };
        if t.label_far.is_some_and(|far| depth > far) || t.label_hide.is_some_and(|r| (r[0]..=r[0] + r[2]).contains(&x) && (r[1]..=r[1] + r[3]).contains(&y)) {
            continue;
        }
        let (x, y) = (x / ppp, y / ppp);
        let tw = ink.width(text);
        // the label's centre may sit this far (half its box) off the free area before it is dropped; inside that it is
        // pinned to the edge and dimmed [design choice: farther off, a ring of pinned labels would fill the edges]
        let (hw, hh) = (tw / 2.0 + 2.0 * u, ink.lh / 2.0);
        let free = Rect::from_min_max(Pos2::new(left + hw, top + hh), Pos2::new((w - right - hw).max(left + hw), (h - bottom - hh).max(top + hh)));
        let reach = Vec2::new(hw + LABEL_PIN * u, hh + LABEL_PIN * u);
        if !free.expand2(reach).contains(Pos2::new(x, y)) {
            continue;
        }
        let pinned = !free.contains(Pos2::new(x, y));
        let (x, y) = (x.clamp(free.min.x, free.max.x), y.clamp(free.min.y, free.max.y));
        let c = if pinned { [c[0], c[1], c[2], c[3] * 0.6] } else { *c };
        painter.rect_filled(Rect::from_min_size(Pos2::new(x - tw / 2.0 - 2.0 * u, y - ink.lh / 2.0), Vec2::new(tw + 4.0 * u, ink.lh)), 0.0, shade(115));
        painter.text(Pos2::new(x - tw / 2.0, y - ink.lh / 2.0 + u), Align2::LEFT_TOP, text, ink.font.clone(), colour(c));
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
        // on the view axis: the window's centre, or the Cine preview frame's (the lens shift, #441)
        let (c, r, th) = (Pos2::new((1.0 + cam.shift.x) * w / 2.0, (1.0 - cam.shift.y) * h / 2.0), 8.0 * u, 2.0 * u);
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

/// What the Changes card asked for.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum CardAct {
    /// Copy as Markdown (`tune_changes::markdown`)
    Copy,
    /// a row's reset button
    Reset(Key),
    Close,
}

/// The Changes card's outer width, points.
const CHANGES_W: f32 = 460.0;

/// The tuning Changes card (#434), opened by the status bar's "N CHANGED" chip: its bottom left at `anchor`. One
/// group per car / World / Camera, each row the property, original -> now and a reset button; camera values that
/// tune.cfg holds come last, dimmed (an exit keeps them). Copy as Markdown and close in the header.
pub fn changes_card(ctx: &egui::Context, list: &[Change], anchor: Pos2) -> Option<CardAct> {
    let mut out = None;
    let lost = list.iter().filter(|c| !c.saved).count();
    egui::Area::new(Id::unique("motorview-changes")).order(Order::Foreground).pivot(Align2::LEFT_BOTTOM).fixed_pos(anchor).show(ctx, |ui| {
        let fr = glass_ui::glass_frame(ctx).inner_margin(egui::Margin { left: 14, right: 14, top: 10, bottom: 12 }).show(ui, |ui| {
            ui.set_width(CHANGES_W - 28.0);
            ui.spacing_mut().item_spacing = Vec2::ZERO;
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new("CHANGES").font(theme::glass_font(22.0)).color(theme::WHITE));
                ui.add_space(8.0);
                ui.label(egui::RichText::new(format!("{lost} unsaved")).size(11.0).color(theme::DIM));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if glass_ui::ibtn(ui, motorview::icons::CLOSE, false).on_hover_text("Close").clicked() {
                        out = Some(CardAct::Close);
                    }
                    ui.add_space(6.0);
                    if glass_ui::ibtn(ui, motorview::icons::COPY, false).on_hover_text("Copy as Markdown").clicked() {
                        out = Some(CardAct::Copy);
                    }
                });
            });
            ui.add_space(2.0);
            ui.label(egui::RichText::new("a car or track switch loses the car and world values; an exit, all but what tune.cfg holds").size(11.0).color(theme::DIM));
            ui.add_space(4.0);
            let mono = FontId::monospace(12.0);
            egui::ScrollArea::vertical().max_height(380.0).auto_shrink([false, true]).show(ui, |ui| {
                let order = list.iter().filter(|c| !c.saved).chain(list.iter().filter(|c| c.saved));
                let mut group: Option<(&str, bool)> = None;
                for c in order {
                    if group != Some((c.group.as_str(), c.saved)) {
                        group = Some((c.group.as_str(), c.saved));
                        ui.add_space(8.0);
                        let head = if c.saved { format!("{} · in tune.cfg", c.group) } else { c.group.clone() };
                        ui.label(egui::RichText::new(head.to_uppercase()).font(theme::glass_font(11.0)).color(if c.saved { theme::DIM } else { theme::S1 }));
                        ui.add_space(2.0);
                    }
                    let (val, now) = if c.saved { (theme::DIM, theme::TEXT) } else { (theme::DIM, theme::ORANGE_TEXT) };
                    let (l, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 1.0), egui::Sense::hover());
                    ui.painter().rect_filled(l, 0.0, theme::line(0.07));
                    ui.horizontal(|ui| {
                        ui.set_height(24.0);
                        let (cap, _) = ui.allocate_exact_size(Vec2::new(140.0, 24.0), egui::Sense::hover());
                        ui.painter().with_clip_rect(cap).text(Pos2::new(cap.left(), cap.center().y), Align2::LEFT_CENTER, &c.label, FontId::proportional(13.0), if c.saved { theme::DIM } else { theme::TEXT });
                        ui.add(egui::Label::new(egui::RichText::new(&c.was).font(mono.clone()).color(val)).truncate());
                        ui.label(egui::RichText::new("  →  ").font(mono.clone()).color(theme::DIM));
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if reset_btn(ui).on_hover_text("back to the original").clicked() {
                                out = Some(CardAct::Reset(c.key));
                            }
                            ui.add_space(8.0);
                            ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                                ui.add(egui::Label::new(egui::RichText::new(&c.now).font(mono.clone()).color(now)).truncate());
                            });
                        });
                    });
                }
            });
        });
        glass_ui::finish(ctx, &fr.response);
    });
    out
}

/// A row's reset: the undo arrow in a 22 pt square, lit on hover.
fn reset_btn(ui: &mut egui::Ui) -> egui::Response {
    let (rect, r) = ui.allocate_exact_size(Vec2::splat(22.0), egui::Sense::click());
    if r.hovered() {
        ui.painter().rect_filled(rect, 5.0, theme::white(0.06));
    }
    ui.painter().text(rect.center(), Align2::CENTER_CENTER, motorview::icons::UNDO, FontId::proportional(14.0), if r.hovered() { theme::WHITE } else { theme::DIM });
    r
}

/// What the requester decided.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Exit {
    /// save the World Editor's fill (and copy the tuning changes, if any), then go on (only offered with unsaved edits)
    Save,
    /// copy the tuning changes as Markdown (the Changes card's button), then go on
    Copy,
    /// go on; unsaved edits and tuning changes are lost
    Discard,
    Cancel,
}

/// What the requester asks about.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum AskKind {
    Exit,
    /// switch to this track
    Track(u32),
    /// switch to this car
    Car(u32),
    /// close the World Editor while it holds unsaved edits (#473)
    WorldClose,
    /// Cine: close the open sequence (#438)
    CineClose,
    /// Cine: start a new sequence
    CineNew,
    /// Cine: open this file in place of the sequence
    CineOpen(std::path::PathBuf),
}

impl AskKind {
    /// The Cine editor's steps: they lose only the sequence, never the tuning changes.
    pub fn is_cine(&self) -> bool {
        matches!(self, AskKind::CineClose | AskKind::CineNew | AskKind::CineOpen(_))
    }
}

/// One question of the requester: what is about to happen and what it would lose (#434).
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Ask {
    pub kind: AskKind,
    /// the World Editor's fill file name while it holds edits that are not saved (exit and closing the editor)
    pub fill: Option<String>,
    /// the Cine sequence's file name while it holds edits that are not saved (#438)
    pub seq: Option<String>,
    /// tuning changes (physics, gravity, paint, camera values) that the step would lose
    pub changes: usize,
}

impl Ask {
    /// Something would be lost: the requester offers Discard.
    pub fn has_loss(&self) -> bool {
        self.fill.is_some() || self.seq.is_some() || self.changes > 0
    }

    /// The answer of Enter / Y and of the highlighted button.
    pub fn default_choice(&self) -> Exit {
        if self.fill.is_some() || self.seq.is_some() {
            Exit::Save
        } else if self.changes > 0 {
            Exit::Copy
        } else {
            Exit::Discard
        }
    }

    fn title(&self) -> &'static str {
        match self.kind {
            AskKind::Exit => "EXIT MOTORVIEW?",
            AskKind::Track(_) => "SWITCH TRACK?",
            AskKind::Car(_) => "SWITCH CAR?",
            AskKind::WorldClose => "CLOSE WORLD EDITOR?",
            AskKind::CineClose => "CLOSE SEQUENCE?",
            AskKind::CineNew => "NEW SEQUENCE?",
            AskKind::CineOpen(_) => "OPEN ANOTHER SEQUENCE?",
        }
    }

    /// The step in the button and the hint: "exit", "switch", "close", "start the new one", "open".
    fn go(&self) -> &'static str {
        match self.kind {
            AskKind::Exit => "exit",
            AskKind::Track(_) | AskKind::Car(_) => "switch",
            AskKind::WorldClose | AskKind::CineClose => "close",
            AskKind::CineNew => "continue",
            AskKind::CineOpen(_) => "open",
        }
    }

    fn primary(&self) -> (&'static str, String) {
        let go = self.go().to_uppercase();
        match self.default_choice() {
            Exit::Save => (motorview::icons::SAVE, format!("SAVE & {go}")),
            Exit::Copy => (motorview::icons::COPY, format!("COPY & {go}")),
            _ => (motorview::icons::EXIT, go),
        }
    }

    fn text(&self) -> String {
        let n = self.changes;
        let tuning = format!("{n} tuning change{}", if n == 1 { "" } else { "s" });
        let go = self.go();
        // what is unsaved, as one phrase (the fill and the sequence can both be)
        let mut lost: Vec<String> = Vec::new();
        if let Some(f) = &self.fill {
            lost.push(format!("the world editor has unsaved changes in {f}"));
        }
        if let Some(q) = &self.seq {
            lost.push(format!("the Cine sequence {q} has unsaved changes"));
        }
        if lost.is_empty() {
            return match (n > 0, &self.kind) {
                (true, AskKind::Exit) => format!("{tuning} would be lost. Enter / Y: copy them as Markdown and exit, D: discard, Esc / N: cancel."),
                (true, _) => format!("Switching restarts the race and drops {tuning}. Enter / Y: copy them as Markdown and switch, D: discard, Esc / N: cancel."),
                (false, _) => format!("Enter / Y: {go}, Esc / N: cancel."),
            };
        }
        // the World Editor alone keeps its #434 wording
        if let (Some(f), None, AskKind::Exit | AskKind::Track(_) | AskKind::Car(_)) = (&self.fill, &self.seq, &self.kind) {
            return if n > 0 {
                format!("The world editor has unsaved changes in {f}, and {tuning} would be lost. Enter / Y: save the fill, copy the changes and exit, D: discard, Esc / N: cancel.")
            } else {
                format!("The world editor has unsaved changes in {f}. Save them before the viewer closes? Enter / Y: save and exit, D: discard, Esc / N: cancel.")
            };
        }
        let mut head = lost.join(", and ");
        head.replace_range(..1, &head[..1].to_uppercase());
        if n > 0 {
            format!("{head}, and {tuning} would be lost. Enter / Y: save, copy the changes and {go}, D: discard, Esc / N: cancel.")
        } else {
            format!("{head}. Enter / Y: save and {go}, D: discard, Esc / N: cancel.")
        }
    }
}

/// The requester (19.3, `requesterHTML`): a near-opaque card over a dark scrim, the title (exit or a car / track
/// switch), and three buttons when something would be lost (the default: Save & exit with an unsaved fill, else Copy &
/// exit / switch; then Discard, Cancel), else Exit (the default) and Cancel. None = still asking. Keys (Enter / Y, D,
/// Esc / N) are the viewer's own.
pub fn requester(ctx: &egui::Context, ask: &Ask) -> Option<Exit> {
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
            ui.label(egui::RichText::new(ask.title()).font(theme::glass_font(28.0)).color(theme::WHITE));
        });
        ui.add_space(14.0);
        ui.add(egui::Label::new(egui::RichText::new(ask.text()).size(15.0).color(theme::TEXT)));
        ui.add_space(24.0);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 16.0;
            let (icon, label) = ask.primary();
            if glass_ui::req_button(ui, icon, &label, true).clicked() {
                out = Some(ask.default_choice());
            }
            if ask.has_loss() && glass_ui::req_button(ui, motorview::icons::EXIT, "DISCARD", false).clicked() {
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

    fn ask(kind: AskKind, fill: bool, seq: bool, changes: usize) -> Ask {
        Ask { kind, fill: fill.then(|| "track 2's fill".to_string()), seq: seq.then(|| "track01.cine".to_string()), changes }
    }

    #[test]
    fn cine_ask_saves_by_default() {
        let a = ask(AskKind::CineClose, false, true, 0);
        assert!(a.has_loss() && a.default_choice() == Exit::Save);
        assert_eq!(a.primary().1, "SAVE & CLOSE");
        assert_eq!(a.text(), "The Cine sequence track01.cine has unsaved changes. Enter / Y: save and close, D: discard, Esc / N: cancel.");
        assert_eq!(ask(AskKind::CineOpen("a.cine".into()), false, true, 0).primary().1, "SAVE & OPEN");
        assert_eq!(ask(AskKind::CineNew, false, true, 0).title(), "NEW SEQUENCE?");
    }

    #[test]
    fn world_close_ask_saves_by_default() {
        let a = ask(AskKind::WorldClose, true, false, 0);
        assert!(a.has_loss() && a.default_choice() == Exit::Save);
        assert_eq!(a.title(), "CLOSE WORLD EDITOR?");
        assert_eq!(a.primary().1, "SAVE & CLOSE");
        assert_eq!(a.text(), "The world editor has unsaved changes in track 2's fill. Enter / Y: save and close, D: discard, Esc / N: cancel.");
    }

    #[test]
    fn exit_ask_names_everything_lost() {
        let a = ask(AskKind::Exit, true, true, 2);
        assert_eq!(
            a.text(),
            "The world editor has unsaved changes in track 2's fill, and the Cine sequence track01.cine has unsaved changes, and 2 tuning changes would be lost. Enter / Y: save, copy the changes and exit, D: discard, Esc / N: cancel."
        );
        // the old wordings are kept
        assert_eq!(ask(AskKind::Exit, true, false, 0).text(), "The world editor has unsaved changes in track 2's fill. Save them before the viewer closes? Enter / Y: save and exit, D: discard, Esc / N: cancel.");
        assert_eq!(ask(AskKind::Exit, false, false, 0).text(), "Enter / Y: exit, Esc / N: cancel.");
        assert_eq!(ask(AskKind::Exit, false, false, 1).text(), "1 tuning change would be lost. Enter / Y: copy them as Markdown and exit, D: discard, Esc / N: cancel.");
        assert_eq!(ask(AskKind::Exit, true, false, 3).text(), "The world editor has unsaved changes in track 2's fill, and 3 tuning changes would be lost. Enter / Y: save the fill, copy the changes and exit, D: discard, Esc / N: cancel.");
        assert_eq!(ask(AskKind::Car(2), false, false, 2).text(), "Switching restarts the race and drops 2 tuning changes. Enter / Y: copy them as Markdown and switch, D: discard, Esc / N: cancel.");
        assert_eq!(ask(AskKind::Car(2), false, false, 2).primary().1, "COPY & SWITCH");
        assert_eq!(ask(AskKind::Exit, false, false, 2).primary().1, "COPY & EXIT");
        assert_eq!(ask(AskKind::Exit, true, false, 0).primary().1, "SAVE & EXIT");
        assert!(!ask(AskKind::Exit, false, false, 0).has_loss());
    }
}
