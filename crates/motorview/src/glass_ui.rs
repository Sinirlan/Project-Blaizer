//! DEBUG motorview's Glass widgets (#395, concept A of `reference_materials/ui-overhaul/motorview-overhaul/`):
//! the tool glass card every panel is drawn in, the record of where those cards are for the scene frost
//! behind them (`render::SceneFrost`, the Glass pause menu's pass), and the 19.4 selection pulse. Tokens are
//! in `theme.rs`. The game does not use it.
//!
//! Frame order [design choice]: egui runs after the scene is drawn, so a pass records its glass cards and the
//! viewer frosts them in the next frame's scene (`frost`); a panel that opens or closes is one frame late.
//! Headless shots lay egui out before they draw, so their single frame has the rects.

use crate::render::{SceneFrost, SCENE_FROST_RECTS};
use crate::theme;
use egui::epaint::Shadow;
use egui::{Align2, Color32, Context, FontId, Frame, Galley, Id, Pos2, Rect, Response, RichText, Sense, Stroke, StrokeKind, Ui, Vec2};
use std::f32::consts::TAU;
use std::sync::Arc;

/// The frost's gaussian sigma in points (`.glass` `backdrop-filter: blur(14px)`).
pub const BLUR: f32 = 14.0;
/// The 19.4 selection pulse period, seconds (the dial's 1.1 s cosine).
pub const PULSE: f64 = 1.1;

fn rects_id() -> Id {
    Id::unique("glass_ui::rects")
}

fn solid_id() -> Id {
    Id::unique("glass_ui::solid")
}

/// Start a pass: forget the last pass's cards; `solid` = Solid panels (opaque glass, no frost).
pub fn begin(ctx: &Context, solid: bool) {
    ctx.data_mut(|d| {
        d.insert_temp(rects_id(), Vec::<Rect>::new());
        d.insert_temp(solid_id(), solid);
    });
}

/// Solid panels are on (`begin`).
pub fn solid(ctx: &Context) -> bool {
    ctx.data(|d| d.get_temp(solid_id()).unwrap_or(false))
}

/// The tool glass card (`.glass`): navy glass, the `line` .30 rim, radius 12, the 0 10 26 shadow. Opaque with
/// Solid panels. Callers set the inner margin; `finish` after `show` adds the top highlight and the frost.
pub fn glass_frame(ctx: &Context) -> Frame {
    Frame::new()
        .fill(if solid(ctx) { theme::GLASS_SOLID } else { theme::GLASS })
        .stroke(Stroke::new(1.0, theme::line(0.30)))
        .corner_radius(theme::RADIUS)
        .shadow(theme::SHADOW)
}

/// A glass card was shown as `r`: its 1 pt top highlight (`inset 0 1px 0 rgba(255,255,255,.22)`) and its rect
/// for the scene frost.
pub fn finish(ctx: &Context, r: &Response) {
    let rect = r.rect;
    let k = theme::RADIUS as f32;
    let y = rect.top() + 1.5;
    ctx.layer_painter(r.layer_id).line_segment([egui::pos2(rect.left() + k, y), egui::pos2(rect.right() - k, y)], Stroke::new(1.0, theme::white(0.22)));
    ctx.data_mut(|d| d.get_temp_mut_or_default::<Vec<Rect>>(rects_id()).push(rect));
}

/// The scene frost behind this pass's glass cards (window px), None without any or with Solid panels.
/// The first `SCENE_FROST_RECTS` cards are frosted.
pub fn frost(ctx: &Context) -> Option<SceneFrost> {
    if solid(ctx) {
        return None;
    }
    let rects = ctx.data(|d| d.get_temp::<Vec<Rect>>(rects_id())).unwrap_or_default();
    if rects.is_empty() {
        return None;
    }
    let ppp = ctx.pixels_per_point();
    let mut fr = SceneFrost { rects: Default::default(), n: rects.len().min(SCENE_FROST_RECTS), radius: theme::RADIUS as f32 * ppp, blur: BLUR * ppp };
    for (slot, r) in fr.rects.iter_mut().zip(&rects) {
        *slot = ([r.center().x * ppp, r.center().y * ppp], [r.width() * 0.5 * ppp, r.height() * 0.5 * ppp]);
    }
    Some(fr)
}

/// The 19.4 selection halo's alpha: .3 to .5 on a 1.1 s cosine. Keeps egui repainting while a pulsing item shows.
pub fn pulse(ctx: &Context) -> f32 {
    ctx.request_repaint();
    let t = ctx.input(|i| i.time);
    0.4 - 0.1 * (TAU * (t / PULSE).fract() as f32).cos()
}

// ---------------------------------------------------------------------------------------------
// Widgets (phase 2: top bar, tool shelf, flyout rows). Sizes are the mockup's, in points
// (`reference_materials/ui-overhaul/motorview-overhaul/assets/motorview.css`). egui text has no
// letter-spacing, so captions that the mockup spaces out are drawn plain [APPROX].
// ---------------------------------------------------------------------------------------------

/// A tool (`.tool`): width at least 50, height 54.
pub const TOOL_W: f32 = 50.0;
pub const TOOL_H: f32 = 54.0;
/// A settings row at tool size (`.srow`).
pub const ROW_H: f32 = 32.0;

fn galley(ui: &Ui, s: impl ToString, font: FontId, color: Color32) -> Arc<Galley> {
    ui.painter().layout_no_wrap(s.to_string(), font, color)
}

/// `rgba(143,208,255,a)`: sector-1 at alpha `a` (lit fills, rims).
fn s1(a: f32) -> Color32 {
    Color32::from_rgba_unmultiplied(143, 208, 255, (a * 255.0 + 0.5) as u8)
}

/// Dim an ink while its widget is disabled (`.tool.off` opacity .35).
fn ink(ui: &Ui, c: Color32) -> Color32 {
    if ui.is_enabled() {
        c
    } else {
        c.gamma_multiply(0.35)
    }
}

/// A 1 pt hairline separator `h` tall (`.vsep`).
pub fn vsep(ui: &mut Ui, h: f32) {
    let (r, _) = ui.allocate_exact_size(Vec2::new(1.0, h), Sense::hover());
    ui.painter().rect_filled(r, 0.0, theme::line(0.14));
}

/// One top bar tab (`.tab`): icon 17, label Rajdhani Bold 16, the orange underline with its glow when open. `label`
/// None = icon only (a narrow window). `dot` = the unsaved dot. `h` is the card's height.
pub fn tab(ui: &mut Ui, icon: &str, label: Option<&str>, on: bool, dot: bool, h: f32) -> Response {
    let col = |hot: bool| if on { theme::ORANGE } else if hot { theme::TEXT } else { theme::DIM };
    let ig = galley(ui, icon, FontId::proportional(17.0), Color32::PLACEHOLDER);
    let lg = label.map(|l| galley(ui, l, theme::glass_font(16.0), Color32::PLACEHOLDER));
    let w = 11.0 * 2.0 + ig.size().x + lg.as_ref().map_or(0.0, |g| 7.0 + g.size().x) + if dot { 8.0 } else { 0.0 };
    let (rect, r) = ui.allocate_exact_size(Vec2::new(w, h), Sense::click());
    let c = ink(ui, col(r.hovered()));
    let mut x = rect.left() + 11.0;
    ui.painter().galley(Pos2::new(x, rect.center().y - ig.size().y / 2.0), ig.clone(), c);
    x += ig.size().x;
    if let Some(g) = lg {
        x += 7.0;
        ui.painter().galley(Pos2::new(x, rect.center().y - g.size().y / 2.0), g.clone(), c);
        x += g.size().x;
    }
    if dot {
        ui.painter().circle_filled(Pos2::new(x + 6.0, rect.center().y), 3.0, theme::ORANGE_TEXT);
    }
    if on {
        let line = Rect::from_min_max(Pos2::new(rect.left() + 12.0, rect.bottom() - 2.0), Pos2::new(rect.right() - 12.0, rect.bottom()));
        let glow = Shadow { offset: [0, 0], blur: 14, spread: 0, color: theme::ORANGE.gamma_multiply(0.45) };
        ui.painter().add(glow.as_shape(line, 1.0));
        ui.painter().rect_filled(line, 1.0, theme::ORANGE);
    }
    r
}

/// A tool (`.tool`): the icon over a short caption. `on` = lit (sector-1 wash and rim, blue icon), `armed` = orange.
pub fn tool(ui: &mut Ui, icon: &str, caption: &str, on: bool) -> Response {
    tool_styled(ui, icon, caption, on, false)
}

/// A tool of the World palette: armed = orange (`.tool.armed`).
pub fn tool_styled(ui: &mut Ui, icon: &str, caption: &str, on: bool, armed: bool) -> Response {
    let cap = galley(ui, caption, FontId::proportional(11.0), Color32::PLACEHOLDER);
    let w = (cap.size().x + 10.0).max(TOOL_W);
    let (rect, r) = ui.allocate_exact_size(Vec2::new(w, TOOL_H), Sense::click());
    let hot = r.hovered() || r.is_pointer_button_down_on();
    let (fill, rim) = match (armed, on, hot) {
        (true, _, _) => (theme::ORANGE.gamma_multiply(0.08), theme::ORANGE),
        (_, true, _) => (s1(if hot { 0.2 } else { 0.14 }), s1(0.75)),
        (_, _, true) => (theme::white(0.05), theme::line(0.32)),
        _ => (Color32::TRANSPARENT, Color32::TRANSPARENT),
    };
    let (text, icon_ink) = match (armed, on) {
        (true, _) => (theme::ORANGE, theme::ORANGE),
        (_, true) => (theme::NAME, theme::S1),
        _ => (theme::TEXT, theme::TEXT),
    };
    ui.painter().rect(rect, theme::RADIUS_TOOL, fill, Stroke::new(1.0, rim), StrokeKind::Inside);
    // icon 20 over caption 11, a 6 gap, centred: 8.5 from the top
    ui.painter().text(Pos2::new(rect.center().x, rect.top() + 18.5), Align2::CENTER_CENTER, icon, FontId::proportional(20.0), ink(ui, icon_ink));
    ui.painter().galley(Pos2::new(rect.center().x - cap.size().x / 2.0, rect.top() + 34.5), cap, ink(ui, text));
    r
}

/// A value tile (`.vtile`): the caption over its value, a chevron for a choice, a meter for a slider. `open` = its
/// flyout is open (orange rim). Click opens the flyout.
pub fn value_tile(ui: &mut Ui, caption: &str, value: &str, chevron: bool, meter: Option<f32>, open: bool) -> Response {
    let cap = galley(ui, caption.to_uppercase(), FontId::proportional(10.0), theme::DIM);
    let val = galley(ui, value, theme::glass_font(16.0), theme::WHITE);
    let chev = if chevron { 16.0 } else { 0.0 };
    let w = (20.0 + cap.size().x.max(val.size().x + chev)).max(58.0);
    let (rect, r) = ui.allocate_exact_size(Vec2::new(w, TOOL_H), Sense::click_and_drag());
    let rim = if open { theme::ORANGE } else if r.hovered() { theme::line(0.4) } else { theme::line(0.14) };
    if open {
        ui.painter().add(Shadow { offset: [0, 0], blur: 14, spread: 0, color: theme::ORANGE.gamma_multiply(0.35) }.as_shape(rect, theme::RADIUS_TOOL));
    }
    ui.painter().rect(rect, theme::RADIUS_TOOL, theme::TILE, Stroke::new(1.0, rim), StrokeKind::Inside);
    // caption 10 + 6 + value 16 (+ 6 + meter 3), centred
    let h = 10.0 + 6.0 + 16.0 + if meter.is_some() { 9.0 } else { 0.0 };
    let top = rect.center().y - h / 2.0;
    let x = rect.left() + 10.0;
    ui.painter().galley(Pos2::new(x, top - 1.0), cap, ink(ui, theme::DIM));
    let vy = top + 16.0;
    ui.painter().galley(Pos2::new(x, vy - 2.0), val.clone(), ink(ui, theme::WHITE));
    if chevron {
        ui.painter().text(Pos2::new(x + val.size().x + 10.0, vy + 6.0), Align2::CENTER_CENTER, crate::icons::CHEVRON_DOWN, FontId::proportional(11.0), theme::DIM);
    }
    if let Some(f) = meter {
        let bar = Rect::from_min_size(Pos2::new(x, vy + 19.0), Vec2::new(rect.width() - 20.0, 3.0));
        ui.painter().rect_filled(bar, 2.0, theme::line(0.14));
        ui.painter().rect_filled(Rect::from_min_size(bar.min, Vec2::new(bar.width() * f.clamp(0.0, 1.0), 3.0)), 2.0, theme::S1);
    }
    r
}

/// One option of a segmented control.
pub struct SegOpt<'a> {
    pub icon: &'a str,
    pub text: &'a str,
}

/// A segmented control (`.seg`): the lit option has the tool's wash and rim. `square` = fixed option width (the number
/// pickers). Returns the clicked option.
pub fn seg(ui: &mut Ui, opts: &[SegOpt], sel: usize, square: Option<f32>) -> Option<usize> {
    let font = theme::glass_font(13.0);
    let gs: Vec<_> = opts.iter().map(|o| (if o.icon.is_empty() { None } else { Some(galley(ui, o.icon, FontId::proportional(14.0), Color32::PLACEHOLDER)) }, galley(ui, o.text, font.clone(), Color32::PLACEHOLDER))).collect();
    let ws: Vec<f32> = gs.iter().map(|(i, t)| square.unwrap_or_else(|| 22.0 + t.size().x + i.as_ref().map_or(0.0, |i| 6.0 + i.size().x))).collect();
    let total = ws.iter().sum::<f32>() + 2.0 * (opts.len().saturating_sub(1)) as f32 + 6.0;
    let (rect, _) = ui.allocate_exact_size(Vec2::new(total, 32.0), Sense::hover());
    ui.painter().rect(rect, 10.0, Color32::from_rgba_unmultiplied(2, 8, 18, 107), Stroke::new(1.0, theme::line(0.16)), StrokeKind::Inside);
    let mut clicked = None;
    let mut x = rect.left() + 3.0;
    for (i, ((ig, tg), w)) in gs.into_iter().zip(ws).enumerate() {
        let o = Rect::from_min_size(Pos2::new(x, rect.top() + 3.0), Vec2::new(w, 26.0));
        let r = ui.interact(o, ui.scope_id().with(("seg", i, opts[i].text)), Sense::click());
        let lit = i == sel;
        if lit {
            ui.painter().rect(o, 7.0, s1(0.14), Stroke::new(1.0, s1(0.75)), StrokeKind::Inside);
        } else if r.hovered() {
            ui.painter().rect_filled(o, 7.0, theme::white(0.04));
        }
        let c = ink(ui, if lit { theme::NAME } else if r.hovered() { theme::TEXT } else { theme::DIM });
        let content = tg.size().x + ig.as_ref().map_or(0.0, |i| 6.0 + i.size().x);
        let mut cx = o.center().x - content / 2.0;
        if let Some(ig) = ig {
            let iw = ig.size().x;
            ui.painter().galley(Pos2::new(cx, o.center().y - ig.size().y / 2.0), ig, if lit { theme::S1 } else { c });
            cx += iw + 6.0;
        }
        ui.painter().galley(Pos2::new(cx, o.center().y - tg.size().y / 2.0), tg, c);
        if r.clicked() {
            clicked = Some(i);
        }
        x += w + 2.0;
    }
    clicked
}

/// A key cap (`.kc`): `quiet` = the plain outline cap of a settings row, else the lit cap of the status bar.
pub fn kcap(ui: &mut Ui, key: &str, quiet: bool) -> Response {
    let g = galley(ui, key, theme::glass_font(11.0), if quiet { theme::DIM } else { Color32::from_rgb(4, 36, 58) });
    let (rect, r) = ui.allocate_exact_size(Vec2::new((g.size().x + 10.0).max(18.0), 18.0), Sense::hover());
    kcap_at(ui, rect, g, quiet);
    r
}

/// A key cap drawn in `rect` (shared with `kcap`).
pub fn kcap_at(ui: &Ui, rect: Rect, g: Arc<Galley>, quiet: bool) {
    let p = ui.painter();
    if quiet {
        p.rect(rect, 4.0, theme::white(0.06), Stroke::new(1.0, theme::line(0.3)), StrokeKind::Inside);
    } else {
        p.rect_filled(rect.translate(Vec2::new(0.0, 2.0)), 4.0, Color32::from_black_alpha(115));
        p.rect(rect, 4.0, theme::white(0.22), Stroke::new(1.0, theme::white(0.4)), StrokeKind::Inside);
    }
    p.galley(rect.center() - g.size() / 2.0, g, Color32::PLACEHOLDER);
}

/// A small selector triangle (`.trib`), 9x11 drawn in a 20x24 hit box; `off` at the end of a list.
pub fn tri(ui: &mut Ui, left: bool, off: bool) -> Response {
    let (rect, r) = ui.allocate_exact_size(Vec2::new(20.0, 24.0), if off { Sense::hover() } else { Sense::click() });
    tri_at(ui, rect, left, off, r.hovered(), None);
    r
}

/// The triangle drawn in `rect`'s centre; `tint` replaces the rim colour (orange on a selected row).
pub fn tri_at(ui: &Ui, rect: Rect, left: bool, off: bool, hot: bool, tint: Option<Color32>) {
    let c = rect.center();
    let pts: Vec<Pos2> = if left {
        vec![Pos2::new(c.x + 3.5, c.y - 4.5), Pos2::new(c.x - 3.0, c.y), Pos2::new(c.x + 3.5, c.y + 4.5)]
    } else {
        vec![Pos2::new(c.x - 3.5, c.y - 4.5), Pos2::new(c.x + 3.0, c.y), Pos2::new(c.x - 3.5, c.y + 4.5)]
    };
    let a = if off { 0.2 } else { 1.0 };
    let rim = tint.unwrap_or(if hot { theme::WHITE } else { Color32::from_rgb(0xd8, 0xf2, 0xff) });
    ui.painter().add(egui::Shape::convex_polygon(pts, theme::white(0.14 * a), Stroke::new(1.6, rim.gamma_multiply(a))));
}

/// A square icon button (`.ibtn.sq`, 36x36): lit while `on`.
pub fn ibtn(ui: &mut Ui, icon: &str, on: bool) -> Response {
    ibtn_shape(ui, on, |p, rect, c| {
        p.text(rect.center(), Align2::CENTER_CENTER, icon, FontId::proportional(17.0), c);
    })
}

/// A square icon button whose picture `paint` draws in the button's rect with the button's ink.
pub fn ibtn_shape(ui: &mut Ui, on: bool, paint: impl FnOnce(&egui::Painter, Rect, Color32)) -> Response {
    let (rect, r) = ui.allocate_exact_size(Vec2::splat(36.0), Sense::click());
    let (fill, rim) = if on { (s1(0.14), s1(0.75)) } else { (theme::white(0.04), if r.hovered() { theme::line(0.5) } else { theme::line(0.22) }) };
    ui.painter().rect(rect, theme::RADIUS_TOOL, fill, Stroke::new(1.0, rim), StrokeKind::Inside);
    paint(ui.painter(), rect, if on { theme::S1 } else { theme::TEXT });
    r
}

/// The dock icon (a window with its right pane marked): `ibtn_shape`'s picture for the F11 button.
pub fn dock_icon(p: &egui::Painter, rect: Rect, c: Color32) {
    let b = Rect::from_center_size(rect.center(), Vec2::new(16.0, 14.0));
    p.rect_stroke(b, 3.0, Stroke::new(1.5, c), StrokeKind::Inside);
    let x = b.right() - 5.5;
    p.line_segment([Pos2::new(x, b.top() + 1.0), Pos2::new(x, b.bottom() - 1.0)], Stroke::new(1.5, c));
}

/// A layer toggle cell of the World tab (`.layers span`, 26x24) in `rect`: lit while the layer shows.
pub fn layer_cell(ui: &mut Ui, rect: Rect, key: usize, icon: &str, on: bool) -> Response {
    let r = ui.interact(rect, ui.scope_id().with(("layer", key)), Sense::click());
    if on {
        ui.painter().rect(rect, 6.0, s1(0.1), Stroke::new(1.0, s1(0.4)), StrokeKind::Inside);
    } else if r.hovered() {
        ui.painter().rect_filled(rect, 6.0, theme::white(0.05));
    }
    ui.painter().text(rect.center(), Align2::CENTER_CENTER, icon, FontId::proportional(13.0), if on { theme::S1 } else { theme::DIM });
    r
}

/// The Sim tab's timeline tile (`.tl`, 300x54): the clock over the run's bar, the lit part up to `now`, the orange
/// head; dragging the bar returns the second under the pointer. `(start, now, end)` in seconds.
pub fn timeline(ui: &mut Ui, span: (f64, f64, f64), left: &str, tag: &str) -> Option<f64> {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(300.0, TOOL_H), Sense::hover());
    ui.painter().rect(rect, theme::RADIUS_TOOL, theme::TILE, Stroke::new(1.0, theme::line(0.14)), StrokeKind::Inside);
    let l = galley(ui, left, FontId::monospace(12.0), theme::WHITE);
    let t = galley(ui, tag.to_uppercase(), FontId::proportional(10.0), theme::DIM);
    ui.painter().galley(Pos2::new(rect.left() + 12.0, rect.top() + 9.0), l, Color32::PLACEHOLDER);
    ui.painter().galley(Pos2::new(rect.right() - 12.0 - t.size().x, rect.top() + 10.0), t, Color32::PLACEHOLDER);
    let bar = Rect::from_min_size(Pos2::new(rect.left() + 12.0, rect.top() + 9.0 + 12.0 + 8.0), Vec2::new(rect.width() - 24.0, 6.0));
    let (start, now, end) = span;
    let f = if end > start { ((now - start) / (end - start)).clamp(0.0, 1.0) as f32 } else { 0.0 };
    let p = ui.painter();
    p.rect_filled(bar, 3.0, theme::line(0.14));
    p.rect_filled(bar, 3.0, s1(0.25));
    let lit = Rect::from_min_size(bar.min, Vec2::new(bar.width() * f, 6.0));
    p.rect_filled(lit, 3.0, theme::S1);
    let hx = bar.left() + bar.width() * f;
    p.rect_filled(Rect::from_center_size(Pos2::new(hx, bar.center().y), Vec2::new(4.0, 14.0)), 2.0, theme::ORANGE);
    let hit = ui.interact(bar.expand2(Vec2::new(4.0, 8.0)), ui.scope_id().with("timeline"), Sense::click_and_drag());
    if !(hit.drag_started() || hit.dragged() || hit.clicked()) {
        return None;
    }
    let k = ((hit.interact_pointer_pos()?.x - bar.left()) / bar.width()).clamp(0.0, 1.0) as f64;
    Some(start + (end - start) * k)
}
/// The track / car dropdown of the top bar (`.pick`): icon, the current number and name, a ▾. A click opens the
/// list of every entry under the box (`row_item` rows, the current one marked) and a row returns its index; the
/// list closes on a pick, a press outside or Esc. `digits` pads the number (`01` cars, `1` tracks), `label_w` is
/// the room for number and name, `open` forces the list open (a headless shot: `--ribbon-pick`).
pub fn dropdown(ui: &mut Ui, icon: &str, names: &[&str], sel: usize, digits: usize, label_w: f32, open: bool) -> Option<usize> {
    let sel = sel.min(names.len().saturating_sub(1));
    let h = 32.0;
    ui.spacing_mut().item_spacing.x = 0.0;
    let w = 10.0 + 15.0 + 9.0 + label_w + 8.0 + 14.0 + 10.0;
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(w, h), Sense::click());
    let popup_id = egui::Popup::default_response_id(&resp);
    if open {
        egui::Popup::open_id(ui.ctx(), popup_id);
    }
    let lit = resp.hovered() || egui::Popup::is_id_open(ui.ctx(), popup_id);
    let number = format!("{:0digits$}", sel + 1);
    let n = galley(ui, &number, theme::glass_font(15.0), theme::DIM);
    let t = galley(ui, names.get(sel).copied().unwrap_or("").to_uppercase(), theme::glass_font(15.0), theme::WHITE);
    let rim = if lit { Stroke::new(1.0, theme::ORANGE) } else { Stroke::new(1.0, theme::line(0.16)) };
    let p = ui.painter();
    p.rect(rect, theme::RADIUS_TOOL, Color32::from_rgba_unmultiplied(2, 8, 18, 89), rim, StrokeKind::Inside);
    let cy = rect.center().y;
    p.text(Pos2::new(rect.left() + 10.0 + 7.5, cy), Align2::CENTER_CENTER, icon, FontId::proportional(15.0), theme::DIM);
    // number (dim) and name (white) from the left of the label box, clipped to it
    let x = rect.left() + 10.0 + 15.0 + 9.0;
    let clip = Rect::from_min_size(Pos2::new(x, rect.top()), Vec2::new(label_w, h));
    let p = ui.painter().with_clip_rect(clip);
    p.galley(Pos2::new(x, cy - n.size().y / 2.0), n.clone(), Color32::PLACEHOLDER);
    p.galley(Pos2::new(x + n.size().x + 6.0, cy - t.size().y / 2.0), t, Color32::PLACEHOLDER);
    let chev = if lit { theme::ORANGE } else { theme::DIM };
    ui.painter().text(Pos2::new(rect.right() - 10.0 - 7.0, cy), Align2::CENTER_CENTER, crate::icons::CHEVRON_DOWN, FontId::proportional(14.0), chev);

    let mut picked = None;
    egui::Popup::menu(&resp).width(rect.width()).show(|ui| {
        ui.spacing_mut().item_spacing.y = 2.0;
        for (i, name) in names.iter().enumerate() {
            if row_item(ui, "", &format!("{:0digits$}  {name}", i + 1), i == sel) {
                picked = Some(i);
            }
        }
    });
    picked.filter(|&i| i != sel)
}

/// What a shelf group's caption row asked for.
#[derive(Clone, Copy)]
pub struct GroupHit {
    /// the ▾ was clicked (open / close the flyout)
    pub fly: bool,
    /// the ⧉ was clicked (open the dock page)
    pub page: bool,
    /// the group's rect (the flyout hangs under it)
    pub rect: Rect,
}

/// A shelf group: its items in a 54 pt row, the caption under them with `▾` (`fly`: Some(open)) and `⧉` (`page`),
/// a hairline before it unless it is the first.
pub fn shelf_group(ui: &mut Ui, title: &str, first: bool, fly: Option<bool>, page: bool, body: impl FnOnce(&mut Ui)) -> GroupHit {
    let mut hit = GroupHit { fly: false, page: false, rect: Rect::NOTHING };
    let cap = galley(ui, title.to_uppercase(), FontId::proportional(10.0), theme::DIM);
    let cap_w = cap.size().x + fly.map_or(0.0, |_| 16.0) + if page { 16.0 } else { 0.0 };
    let r = ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing = Vec2::new(4.0, 0.0);
        ui.add_space(3.0);
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing = Vec2::new(4.0, 0.0);
            ui.add_space(7.0);
            ui.set_min_width(cap_w);
            let body_r = ui.horizontal(|ui| {
                ui.set_height(TOOL_H);
                body(ui);
            });
            let (row, _) = ui.allocate_exact_size(Vec2::new(body_r.response.rect.width().max(cap_w), 22.0), Sense::hover());
            let mut x = row.center().x - cap_w / 2.0;
            ui.painter().galley(Pos2::new(x, row.center().y - cap.size().y / 2.0 - 0.5), cap.clone(), theme::DIM);
            x += cap.size().x;
            let mut icon = |ui: &mut Ui, glyph: &str, on: bool, tip: String| {
                x += 6.0;
                let c = Rect::from_center_size(Pos2::new(x + 5.0, row.center().y), Vec2::splat(18.0));
                let r = ui.interact(c, ui.scope_id().with((glyph, "g")), Sense::click()).on_hover_text(tip);
                let col = if on || r.hovered() { theme::ORANGE } else { theme::DIM.gamma_multiply(0.8) };
                ui.painter().text(c.center(), Align2::CENTER_CENTER, glyph, FontId::proportional(10.0), col);
                x += 10.0;
                r.clicked()
            };
            if let Some(open) = fly {
                hit.fly = icon(ui, crate::icons::CHEVRON_DOWN, open, format!("all {title} settings"));
            }
            if page {
                hit.page = icon(ui, crate::icons::LAUNCH, false, "open the page in the dock".into());
            }
        });
        ui.add_space(3.0);
    });
    hit.rect = r.response.rect;
    if !first {
        let (x, top, bottom) = (hit.rect.left(), hit.rect.top(), hit.rect.bottom());
        ui.painter().line_segment([Pos2::new(x, top + 12.0), Pos2::new(x, bottom - 12.0)], Stroke::new(1.0, theme::line(0.12)));
    }
    hit
}

// ---------------------------------------------------------------------------------------------
// Flyout and its settings rows (`.flyout`, `.srow`)
// ---------------------------------------------------------------------------------------------

/// The flyout's width.
pub const FLYOUT_W: f32 = 380.0;

/// The shelf group's flyout: a glass card under the group with a nub over it, the group's rows inside. `group` = the
/// group's rect, `top` = where the card starts (under the shelf). Returns (card rect, ✕ clicked, ⧉ clicked).
pub fn flyout(ctx: &Context, group: Rect, top: f32, title: &str, icon: &str, page: bool, body: impl FnOnce(&mut Ui)) -> (Rect, bool, bool) {
    let screen = ctx.content_rect();
    let x = group.left().min(screen.right() - FLYOUT_W - 12.0).max(12.0);
    let (mut close, mut open_page) = (false, false);
    let r = egui::Area::new(Id::unique("glass_ui::flyout")).order(egui::Order::Foreground).fixed_pos(Pos2::new(x, top)).show(ctx, |ui| {
        let fr = glass_frame(ctx).inner_margin(egui::Margin { left: 10, right: 10, top: 12, bottom: 10 }).show(ui, |ui| {
            ui.set_width(FLYOUT_W - 20.0);
            ui.spacing_mut().item_spacing = Vec2::new(6.0, 0.0);
            ui.horizontal(|ui| {
                ui.add_space(6.0);
                ui.label(RichText::new(icon).size(16.0).color(theme::DIM));
                ui.label(RichText::new(title.to_uppercase()).font(theme::glass_font(16.0)).color(theme::WHITE));
                ui.label(RichText::new("ALL SETTINGS").size(10.0).color(theme::DIM));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    close = ui.add(egui::Button::new(RichText::new(crate::icons::CLOSE).size(14.0).color(theme::DIM)).frame(false)).on_hover_text("close  [Esc]").clicked();
                    if page {
                        open_page = ui.add(egui::Button::new(RichText::new(format!("{}  PAGE", crate::icons::LAUNCH)).size(10.0).color(theme::DIM)).small()).on_hover_text("open the page in the dock").clicked();
                    }
                });
            });
            ui.add_space(8.0);
            body(ui);
        });
        let rect = fr.response.rect;
        // the nub: a 12 pt square turned 45 degrees, half of it over the card's top edge
        let nx = (group.center().x).clamp(rect.left() + 20.0, rect.right() - 20.0);
        let tip = Pos2::new(nx, rect.top() - 7.0);
        let nub = vec![tip, Pos2::new(nx + 8.0, rect.top() + 1.0), Pos2::new(nx - 8.0, rect.top() + 1.0)];
        let p = ui.painter();
        p.add(egui::Shape::convex_polygon(nub, Color32::from_rgba_unmultiplied(30, 44, 62, 242), Stroke::NONE));
        p.line_segment([tip, Pos2::new(nx + 8.0, rect.top() + 1.0)], Stroke::new(1.0, theme::line(0.3)));
        p.line_segment([tip, Pos2::new(nx - 8.0, rect.top() + 1.0)], Stroke::new(1.0, theme::line(0.3)));
        finish(ctx, &fr.response);
        rect
    });
    (r.inner, close, open_page)
}

/// A settings row's frame: the label (icon, name, key cap) on the left, the control's rect on the right.
fn row_shell(ui: &mut Ui, icon: &str, name: &str, key: &str, sense: Sense, ctl_w: f32) -> (Rect, Rect, Response) {
    let (rect, r) = ui.allocate_exact_size(Vec2::new(ui.available_width(), ROW_H), sense);
    let hot = r.hovered() && ui.is_enabled();
    if hot {
        ui.painter().rect(rect, theme::RADIUS_TOOL, theme::ORANGE.gamma_multiply(0.05), Stroke::new(1.0, theme::ORANGE), StrokeKind::Inside);
    }
    let cy = rect.center().y;
    let (label, glyph) = if hot { (theme::ORANGE, theme::ORANGE) } else { (theme::TEXT, theme::DIM) };
    let mut x = rect.left() + 12.0;
    if !icon.is_empty() {
        ui.painter().text(Pos2::new(x + 8.0, cy), Align2::CENTER_CENTER, icon, FontId::proportional(15.0), ink(ui, glyph));
        x += 16.0 + 9.0;
    }
    let g = galley(ui, name, theme::glass_font(15.0), ink(ui, label));
    ui.painter().galley(Pos2::new(x, cy - g.size().y / 2.0), g.clone(), Color32::PLACEHOLDER);
    if !key.is_empty() {
        let kg = galley(ui, key, theme::glass_font(11.0), theme::DIM);
        let kr = Rect::from_min_size(Pos2::new(x + g.size().x + 8.0, cy - 9.0), Vec2::new((kg.size().x + 10.0).max(18.0), 18.0));
        kcap_at(ui, kr, kg, true);
    }
    let ctl = Rect::from_min_max(Pos2::new(rect.right() - 10.0 - ctl_w, rect.top()), Pos2::new(rect.right() - 10.0, rect.bottom()));
    (rect, ctl, r)
}

/// A toggle row: the check box on the right (`.check`). True when clicked.
pub fn row_check(ui: &mut Ui, icon: &str, name: &str, key: &str, on: bool) -> bool {
    let (_, ctl, r) = row_shell(ui, icon, name, key, Sense::click(), 22.0);
    let b = Rect::from_center_size(Pos2::new(ctl.right() - 10.0, ctl.center().y), Vec2::splat(20.0));
    if on {
        ui.painter().rect(b, 5.0, Color32::from_rgb(0x4d, 0xc1, 0xe9), Stroke::NONE, StrokeKind::Inside);
        let c = b.center();
        ui.painter().add(egui::Shape::line(vec![c + Vec2::new(-4.5, 0.5), c + Vec2::new(-1.0, 4.0), c + Vec2::new(4.5, -4.0)], Stroke::new(2.2, Color32::from_rgb(4, 36, 58))));
    } else {
        ui.painter().rect(b, 5.0, theme::white(0.08), Stroke::new(1.0, theme::line(0.5)), StrokeKind::Inside);
    }
    r.clicked()
}

/// An action row: its key cap and a play mark on the right. True when clicked.
pub fn row_action(ui: &mut Ui, icon: &str, name: &str, key: &str) -> bool {
    let (rect, ctl, r) = row_shell(ui, icon, name, "", Sense::click(), 120.0);
    let cy = rect.center().y;
    ui.painter().text(Pos2::new(ctl.right() - 6.0, cy), Align2::CENTER_CENTER, crate::icons::FORWARD, FontId::proportional(11.0), ink(ui, theme::DIM));
    if !key.is_empty() {
        let kg = galley(ui, key, theme::glass_font(11.0), theme::DIM);
        let w = (kg.size().x + 10.0).max(18.0);
        kcap_at(ui, Rect::from_min_size(Pos2::new(ctl.right() - 20.0 - w, cy - 9.0), Vec2::new(w, 18.0)), kg, true);
    }
    r.clicked()
}

/// A choice row: ◀ value ▶ on the right. -1 / 1 when a triangle was clicked.
pub fn row_choice(ui: &mut Ui, icon: &str, name: &str, key: &str, shown: &str, can_prev: bool, can_next: bool) -> i8 {
    let (_, ctl, r) = row_shell(ui, icon, name, key, Sense::hover(), 168.0);
    let mut out = 0;
    let cy = ctl.center().y;
    let (l, rr) = (Rect::from_center_size(Pos2::new(ctl.left() + 10.0, cy), Vec2::new(20.0, 24.0)), Rect::from_center_size(Pos2::new(ctl.right() - 10.0, cy), Vec2::new(20.0, 24.0)));
    let hot = r.hovered();
    let tint = hot.then_some(theme::ORANGE);
    for (rc, left, on, d) in [(l, true, can_prev, -1), (rr, false, can_next, 1)] {
        let tr = ui.interact(rc, r.id.with(("tri", left)), if on { Sense::click() } else { Sense::hover() });
        tri_at(ui, rc, left, !on, tr.hovered(), tint);
        if tr.clicked() {
            out = d;
        }
    }
    let g = galley(ui, shown, theme::glass_font(15.0), ink(ui, theme::WHITE));
    ui.painter().galley(Pos2::new(ctl.center().x - g.size().x / 2.0, cy - g.size().y / 2.0), g, Color32::PLACEHOLDER);
    out
}

/// A slider row: the slider and its value text on the right. `edit` draws the slider in the control's rect.
pub fn row_ctl(ui: &mut Ui, icon: &str, name: &str, key: &str, ctl_w: f32, edit: impl FnOnce(&mut Ui)) {
    let (_, ctl, _) = row_shell(ui, icon, name, key, Sense::hover(), ctl_w);
    let mut c = ui.new_child(egui::UiBuilder::new().max_rect(ctl).layout(egui::Layout::right_to_left(egui::Align::Center)));
    edit(&mut c);
}

/// A list entry row (`lit` = the current one, blue). True when clicked.
pub fn row_item(ui: &mut Ui, icon: &str, name: &str, lit: bool) -> bool {
    let (rect, ctl, r) = row_shell(ui, icon, name, "", Sense::click(), 20.0);
    if lit {
        ui.painter().circle_filled(Pos2::new(ctl.right() - 6.0, rect.center().y), 4.0, theme::S1);
    }
    r.clicked()
}

// ---------------------------------------------------------------------------------------------
// Dock (phase 3): page rail, header, property search, sections, property rows and fields
// (`.rail`, `.insp-head`, `.search`, `.sect-h`, `.prow`, `.num`, `.vec`, `.swatch`, `.readout`)
// ---------------------------------------------------------------------------------------------

fn filter_id() -> Id {
    Id::unique("glass_ui::filter")
}

fn sect_hit_id() -> Id {
    Id::unique("glass_ui::section_hit")
}

fn reset_all_id() -> Id {
    Id::unique("glass_ui::reset_all")
}

/// The dock's property search text (kept lower case, trimmed); empty = every row shows.
pub fn set_filter(ctx: &Context, text: &str) {
    ctx.data_mut(|d| d.insert_temp(filter_id(), text.trim().to_lowercase()));
}

/// The search text set by `set_filter`.
pub fn filter(ctx: &Context) -> String {
    ctx.data(|d| d.get_temp::<String>(filter_id())).unwrap_or_default()
}

/// The page header's reset was clicked this pass: every section that has a reset button resets.
pub fn set_reset_all(ctx: &Context, on: bool) {
    ctx.data_mut(|d| d.insert_temp(reset_all_id(), on));
}

pub fn reset_all(ctx: &Context) -> bool {
    ctx.data(|d| d.get_temp(reset_all_id()).unwrap_or(false))
}

/// A section starts: with a search text, its rows all show when the section's own name matches.
pub fn section_enter(ctx: &Context, title: &str) {
    let f = filter(ctx);
    ctx.data_mut(|d| d.insert_temp(sect_hit_id(), !f.is_empty() && title.to_lowercase().contains(&f)));
}

/// The section that started last (`section_enter`) matched the search by its own name.
pub fn section_matched(ctx: &Context) -> bool {
    ctx.data(|d| d.get_temp(sect_hit_id()).unwrap_or(false))
}

/// A row labelled `label` passes the property search (no search, its section matched, or the label matches).
pub fn row_shown(ctx: &Context, label: &str) -> bool {
    let f = filter(ctx);
    f.is_empty() || ctx.data(|d| d.get_temp(sect_hit_id()).unwrap_or(false)) || label.to_lowercase().contains(&f)
}

/// The dock's icon rail item (`.rail .ri`, 36x36): the open page is orange with a bar on the card's edge.
pub fn rail_item(ui: &mut Ui, icon: &str, on: bool) -> Response {
    let (rect, r) = ui.allocate_exact_size(Vec2::splat(36.0), Sense::click());
    let p = ui.painter();
    if on {
        p.rect_filled(rect, theme::RADIUS_TOOL as f32, theme::ORANGE.gamma_multiply(0.08));
        let x = rect.left() - 6.0;
        p.rect_filled(Rect::from_min_max(Pos2::new(x - 4.0, rect.top() + 8.0), Pos2::new(x + 2.0, rect.bottom() - 8.0)), 1.0, theme::ORANGE.gamma_multiply(0.18));
        p.rect_filled(Rect::from_min_max(Pos2::new(x, rect.top() + 8.0), Pos2::new(x + 2.0, rect.bottom() - 8.0)), 1.0, theme::ORANGE);
    } else if r.hovered() {
        p.rect_filled(rect, theme::RADIUS_TOOL as f32, theme::white(0.05));
    }
    let c = if on { theme::ORANGE } else if r.hovered() { theme::TEXT } else { theme::DIM };
    p.text(rect.center(), Align2::CENTER_CENTER, icon, FontId::proportional(18.0), c);
    r
}

/// A hairline between two groups of rail items (`.rail .rs`).
pub fn rail_sep(ui: &mut Ui) {
    ui.add_space(4.0);
    let (r, _) = ui.allocate_exact_size(Vec2::new(24.0, 1.0), Sense::hover());
    ui.painter().rect_filled(r, 0.0, theme::line(0.14));
    ui.add_space(4.0);
}

/// A small square header tool (`.insp-head .tools .lc`, 28x28); `on` = blue.
fn head_tool(ui: &mut Ui, icon: &str, tip: &str) -> Response {
    let (rect, r) = ui.allocate_exact_size(Vec2::splat(28.0), Sense::click());
    let hot = r.hovered();
    if hot {
        ui.painter().rect_filled(rect, 7.0, theme::white(0.06));
    }
    ui.painter().text(rect.center(), Align2::CENTER_CENTER, icon, FontId::proportional(15.0), if hot { theme::TEXT } else { theme::DIM });
    r.on_hover_text(tip)
}

/// The dock header (`.insp-head`): the page icon (24), its name (Rajdhani Bold 22), a caption under it, and the
/// reset / close tools. Returns (reset page, close).
pub fn dock_header(ui: &mut Ui, icon: &str, name: &str, sub: &str) -> (bool, bool) {
    let (mut reset, mut close) = (false, false);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing = Vec2::new(12.0, 0.0);
        ui.add_space(6.0);
        ui.label(RichText::new(icon).size(24.0).color(theme::DIM));
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 3.0;
            ui.label(RichText::new(name.to_uppercase()).font(theme::glass_font(22.0)).color(theme::WHITE));
            if !sub.is_empty() {
                ui.label(RichText::new(sub.to_uppercase()).size(11.0).color(theme::DIM));
            }
        });
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            close = head_tool(ui, crate::icons::CLOSE, "close  [F11]").clicked();
            reset = head_tool(ui, crate::icons::RESET, "reset every section of this page").clicked();
        });
    });
    (reset, close)
}

/// The property search field (`.search`): the magnifier, the text, an ✕ to clear it; orange rim while it has the keys.
pub fn search(ui: &mut Ui, text: &mut String, hint: &str) -> Response {
    search_with(ui, text, hint, crate::icons::SEARCH)
}

/// `search` with another icon (the outliner's filter funnel).
pub fn search_with(ui: &mut Ui, text: &mut String, hint: &str, icon: &str) -> Response {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 30.0), Sense::hover());
    let bg = ui.painter().add(egui::Shape::Noop);
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(rect.shrink2(Vec2::new(10.0, 0.0))).layout(egui::Layout::left_to_right(egui::Align::Center)));
    child.spacing_mut().item_spacing.x = 8.0;
    child.label(RichText::new(icon).size(14.0).color(theme::DIM));
    let w = (child.available_width() - 24.0).max(20.0);
    let edit = egui::TextEdit::singleline(text).frame(egui::Frame::NONE).hint_text(RichText::new(hint).color(theme::TEXT.gamma_multiply(0.45))).desired_width(w).font(FontId::proportional(14.0));
    let r = child.add(edit);
    if !text.is_empty() && child.add(egui::Button::new(RichText::new(crate::icons::CLOSE).size(12.0).color(theme::DIM)).frame(false)).on_hover_text("clear").clicked() {
        text.clear();
    }
    let (fill, rim) = if r.has_focus() { (theme::FIELD, theme::ORANGE) } else { (theme::FIELD, theme::line(0.18)) };
    let mut shapes = vec![egui::Shape::rect_filled(rect, 8.0, fill), egui::Shape::rect_stroke(rect, 8.0, Stroke::new(1.0, rim), StrokeKind::Inside)];
    if r.has_focus() {
        shapes.push(egui::Shape::rect_stroke(rect.expand(2.0), 10.0, Stroke::new(2.0, theme::ORANGE.gamma_multiply(0.25)), StrokeKind::Outside));
    }
    ui.painter().set(bg, egui::Shape::Vec(shapes));
    r
}

/// A section header (`.sect-h`, 32 tall): chevron, icon, name (Rajdhani Bold 15), an optional count, the dashed
/// rule and a reset button. `open` is the section's state (a click on the header flips it). True when reset was clicked.
pub fn section_header(ui: &mut Ui, icon: &str, title: &str, count: Option<&str>, open: &mut bool, reset: bool) -> bool {
    let (rect, r) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 32.0), Sense::click());
    let p = ui.painter();
    if r.hovered() {
        p.rect_filled(rect, 7.0, theme::white(0.04));
    }
    if r.clicked() {
        *open = !*open;
    }
    let cy = rect.center().y;
    let c = Pos2::new(rect.left() + 12.0, cy);
    let pts = if *open { vec![c + Vec2::new(-4.0, -2.0), c + Vec2::new(0.0, 2.0), c + Vec2::new(4.0, -2.0)] } else { vec![c + Vec2::new(-2.0, -4.0), c + Vec2::new(2.0, 0.0), c + Vec2::new(-2.0, 4.0)] };
    p.add(egui::Shape::line(pts, Stroke::new(1.6, theme::DIM)));
    p.text(Pos2::new(rect.left() + 34.0, cy), Align2::CENTER_CENTER, icon, FontId::proportional(15.0), theme::DIM);
    let name = galley(ui, title.to_uppercase(), theme::glass_font(15.0), theme::TEXT);
    let mut x = rect.left() + 50.0;
    p.galley(Pos2::new(x, cy - name.size().y / 2.0), name.clone(), Color32::PLACEHOLDER);
    x += name.size().x + 8.0;
    if let Some(n) = count {
        let g = galley(ui, n, FontId::proportional(11.0), theme::DIM);
        p.galley(Pos2::new(x, cy - g.size().y / 2.0), g.clone(), Color32::PLACEHOLDER);
        x += g.size().x + 8.0;
    }
    let end = rect.right() - if reset { 30.0 } else { 6.0 };
    if end > x + 8.0 {
        p.add(egui::Shape::dashed_line(&[Pos2::new(x + 4.0, cy), Pos2::new(end, cy)], Stroke::new(1.0, theme::line(0.28)), 6.0, 4.0));
    }
    let mut clicked = false;
    if reset {
        let b = Rect::from_center_size(Pos2::new(rect.right() - 16.0, cy), Vec2::splat(24.0));
        let br = ui.interact(b, r.id.with("reset"), Sense::click()).on_hover_text("reset this section");
        let hot = br.hovered();
        if hot {
            ui.painter().rect_filled(b, 6.0, theme::ORANGE.gamma_multiply(0.08));
        }
        ui.painter().text(b.center(), Align2::CENTER_CENTER, crate::icons::RESET, FontId::proportional(13.0), if hot { theme::ORANGE } else { theme::DIM });
        clicked = br.clicked();
    }
    clicked
}

/// A small caption over a group of rows (the sub-headings of a page without sections), dim, capitals.
pub fn caption(ui: &mut Ui, text: &str) {
    if !filter(ui.ctx()).is_empty() {
        return;
    }
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        ui.add_space(6.0);
        ui.label(RichText::new(text.to_uppercase()).size(11.0).color(theme::DIM));
    });
    ui.add_space(2.0);
}

/// A property row's height (`.prow`).
pub const PROW_H: f32 = 28.0;
/// The number field's size (`.num`).
pub const NUM_W: f32 = 92.0;
pub const NUM_H: f32 = 22.0;

fn rows_id() -> Id {
    Id::unique("glass_ui::rows")
}

/// Count the property rows drawn from now (`rows_drawn` reads it): a section tells whether the search left it any.
pub fn rows_begin(ctx: &Context) {
    ctx.data_mut(|d| d.insert_temp(rows_id(), 0usize));
}

pub fn rows_drawn(ctx: &Context) -> usize {
    ctx.data(|d| d.get_temp(rows_id()).unwrap_or(0))
}

fn last_row_id() -> Id {
    Id::unique("glass_ui::last_row")
}

/// The line under a property row whose label took its own line (a long label over its fields): no label, drawn
/// only when that row was.
pub fn prow_cont(ui: &mut Ui, unit: Option<&str>, ctl: impl FnOnce(&mut Ui)) {
    if ui.ctx().data(|d| d.get_temp(last_row_id()).unwrap_or(true)) {
        prow(ui, "", false, unit, ctl);
        ui.ctx().data_mut(|d| d.insert_temp(last_row_id(), true));
    }
}

/// The room a property row leaves for its label before `ctl_w` points of controls (the unit column included).
pub fn label_room(ui: &Ui, ctl_w: f32) -> f32 {
    ui.available_width() - 28.0 - 6.0 - ctl_w - 8.0
}

/// A property row (`.prow`, 28 tall): the label on the left with the orange "changed from the file" dot before it,
/// the control right-aligned, then the `unit` column (None = no unit column, as the swatch rows). A row the search
/// text does not match is not drawn.
pub fn prow(ui: &mut Ui, label: &str, modified: bool, unit: Option<&str>, ctl: impl FnOnce(&mut Ui)) {
    let shown = row_shown(ui.ctx(), label);
    ui.ctx().data_mut(|d| {
        d.insert_temp(last_row_id(), shown);
        if shown {
            *d.get_temp_mut_or_insert_with(rows_id(), || 0usize) += 1;
        }
    });
    if !shown {
        return;
    }
    let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), PROW_H), Sense::hover());
    let p = ui.painter();
    if ui.rect_contains_pointer(rect) {
        p.rect_filled(rect, 7.0, theme::white(0.035));
    }
    let cy = rect.center().y;
    if modified {
        p.circle_filled(Pos2::new(rect.left() + 17.0, cy), 3.0, theme::ORANGE_TEXT);
        p.circle_filled(Pos2::new(rect.left() + 17.0, cy), 5.0, theme::ORANGE.gamma_multiply(0.22));
    }
    let g = galley(ui, label, FontId::proportional(14.0), theme::TEXT);
    p.galley(Pos2::new(rect.left() + 28.0, cy - g.size().y / 2.0), g, Color32::PLACEHOLDER);
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(rect.shrink2(Vec2::new(6.0, 0.0))).layout(egui::Layout::right_to_left(egui::Align::Center)));
    child.spacing_mut().item_spacing.x = 4.0;
    if let Some(u) = unit {
        let (ur, _) = child.allocate_exact_size(Vec2::new(30.0, PROW_H), Sense::hover());
        child.painter().text(Pos2::new(ur.left() + 6.0, cy), Align2::LEFT_CENTER, u, FontId::proportional(11.0), theme::DIM);
    }
    ctl(&mut child);
}

/// A number field (`.num`, 92x22): mono value, a 2 pt range bar along the bottom when `range` is given, an orange
/// rim when `modified`. Drags and steps with the wheel like `gui::wheel_drag`.
pub fn num_field(ui: &mut Ui, v: &mut f32, speed: f32, range: Option<std::ops::RangeInclusive<f32>>, modified: bool) -> Response {
    num_at(ui, NUM_W, v, speed, range, modified, None)
}

fn num_at(ui: &mut Ui, w: f32, v: &mut f32, speed: f32, range: Option<std::ops::RangeInclusive<f32>>, modified: bool, axis: Option<(&str, Color32)>) -> Response {
    num_sfx(ui, w, v, speed, range, modified, axis, "")
}

/// `num_at` with a suffix after the value (`4.0 m`).
#[allow(clippy::too_many_arguments)]
fn num_sfx(ui: &mut Ui, w: f32, v: &mut f32, speed: f32, range: Option<std::ops::RangeInclusive<f32>>, modified: bool, axis: Option<(&str, Color32)>, suffix: &str) -> Response {
    let bar = range.clone();
    let r = ui
        .scope(|ui| {
            ui.style_mut().drag_value_text_style = egui::TextStyle::Monospace;
            ui.spacing_mut().interact_size = Vec2::new(w, NUM_H);
            crate::gui::wheel_drag(ui, v, range, |d| d.speed(speed).max_decimals(3).suffix(suffix))
        })
        .inner;
    let rect = r.rect;
    let p = ui.painter();
    if let Some(rg) = bar {
        let (lo, hi) = (*rg.start(), *rg.end());
        if hi > lo {
            let f = ((*v - lo) / (hi - lo)).clamp(0.0, 1.0);
            p.rect_filled(Rect::from_min_max(Pos2::new(rect.left() + 3.0, rect.bottom() - 3.0), Pos2::new(rect.left() + 3.0 + (rect.width() - 6.0) * f, rect.bottom() - 1.0)), 1.0, s1(0.55));
        }
    }
    if modified {
        p.rect_stroke(rect, theme::RADIUS_FIELD as f32, Stroke::new(1.0, theme::ORANGE_TEXT.gamma_multiply(0.45)), StrokeKind::Inside);
    }
    if let Some((letter, c)) = axis {
        p.text(Pos2::new(rect.left() + 8.0, rect.center().y), Align2::LEFT_CENTER, letter, theme::glass_font(11.0), c);
    }
    r
}

/// An X / Y / Z field row body (`.vec`): three 76 wide fields with the axis letters tinted. `orig` marks the
/// components that differ from the file. True when one changed. Add inside `prow` (right to left).
pub fn vec3_field(ui: &mut Ui, v: &mut glam::Vec3, speed: f32, orig: Option<glam::Vec3>) -> bool {
    let mut ch = false;
    let (x, y, z) = (&mut v.x, &mut v.y, &mut v.z);
    // right to left: Z first
    for (i, c, letter) in [(2, z, "Z"), (1, y, "Y"), (0, x, "X")] {
        let modified = orig.is_some_and(|o| [o.x, o.y, o.z][i] != *c);
        ch |= num_at(ui, 76.0, c, speed, None, modified, Some((letter, theme::AXIS[i]))).changed();
    }
    ch
}

/// A colour swatch row body (`.swatch` 44x20 and its hex value). True when the colour changed.
pub fn swatch_field(ui: &mut Ui, rgb: &mut [u8; 3]) -> bool {
    let ch = ui
        .scope(|ui| {
            ui.spacing_mut().interact_size = Vec2::new(44.0, 20.0);
            egui::color_picker::color_edit_button_srgb(ui, rgb).changed()
        })
        .inner;
    ui.label(RichText::new(format!("#{:02x}{:02x}{:02x}", rgb[0], rgb[1], rgb[2])).font(FontId::monospace(12.0)).color(theme::DIM));
    ch
}

/// A slider row: the label and its slider with the value field (`.prow` + the sector-1 rail). A label's "(…)" part
/// is the hover text. True when changed.
pub fn slider_row<N: egui::emath::Numeric>(ui: &mut Ui, label: &str, v: &mut N, range: std::ops::RangeInclusive<N>) -> bool {
    let (name, tip) = match label.split_once(" (") {
        Some((n, t)) => (n, Some(t.trim_end_matches(')'))),
        None => (label, None),
    };
    let mut ch = false;
    prow(ui, &cap_first(name), false, None, |ui| {
        ui.spacing_mut().slider_width = 96.0;
        let r = crate::gui::wheel_slider(ui, v, range, |s| s);
        ch = r.changed();
        if let Some(t) = tip {
            r.on_hover_text(t);
        }
    });
    ch
}

/// `abc` -> `Abc`.
pub fn cap_first(s: &str) -> String {
    let mut c = s.chars();
    c.next().map_or_else(String::new, |f| f.to_uppercase().chain(c).collect())
}

/// A live readout chip (`.readout .r`): the value in mono over nothing, its caption after it.
pub fn chip(ui: &mut Ui, value: &str, caption: &str) -> Response {
    let v = galley(ui, value, FontId::monospace(13.0), theme::WHITE);
    let c = galley(ui, caption.to_uppercase(), FontId::proportional(10.0), theme::DIM);
    let (rect, r) = ui.allocate_exact_size(Vec2::new(v.size().x + c.size().x + 5.0 + 18.0, 24.0), Sense::hover());
    let p = ui.painter();
    p.rect(rect, 7.0, theme::TILE, Stroke::new(1.0, theme::line(0.12)), StrokeKind::Inside);
    p.galley(Pos2::new(rect.left() + 9.0, rect.center().y - v.size().y / 2.0), v.clone(), Color32::PLACEHOLDER);
    p.galley(Pos2::new(rect.left() + 9.0 + v.size().x + 5.0, rect.center().y - c.size().y / 2.0 + 0.5), c, Color32::PLACEHOLDER);
    r
}

// ---------------------------------------------------------------------------------------------
// Status bar parts (`.status`)
// ---------------------------------------------------------------------------------------------

/// One part of the status bar's hint: a key cap, plain text, or the bold mode name.
#[derive(Clone, Debug, PartialEq)]
pub enum HintPart {
    Key(String),
    Text(String),
    Head(String),
}

/// A hint written as `*MODE* [LMB] place [Shift] snap`: `[…]` is a key cap, `*…*` the bold mode name, the rest text.
pub fn hint(s: &str) -> Vec<HintPart> {
    let mut out = Vec::new();
    let mut rest = s;
    while !rest.is_empty() {
        let next = rest.find(['[', '*']).unwrap_or(rest.len());
        if next > 0 {
            let t = rest[..next].trim();
            if !t.is_empty() {
                out.push(HintPart::Text(t.to_string()));
            }
            rest = &rest[next..];
            continue;
        }
        let close = if rest.starts_with('[') { ']' } else { '*' };
        match rest[1..].find(close) {
            Some(e) => {
                let inner = rest[1..1 + e].to_string();
                out.push(if close == ']' { HintPart::Key(inner) } else { HintPart::Head(inner) });
                rest = &rest[e + 2..];
            }
            None => {
                out.push(HintPart::Text(rest.to_string()));
                break;
            }
        }
    }
    out
}

/// The hint's parts as plain text (tests, logs).
pub fn hint_text(parts: &[HintPart]) -> String {
    parts.iter().map(|p| match p {
        HintPart::Key(k) => format!("[{k}]"),
        HintPart::Text(t) | HintPart::Head(t) => t.clone(),
    }).collect::<Vec<_>>().join(" ")
}

/// Draw a hint left to right: key caps, text, the bold mode name.
pub fn hint_row(ui: &mut Ui, parts: &[HintPart]) {
    let mut prev_text = false;
    for part in parts {
        match part {
            HintPart::Key(k) => {
                if prev_text {
                    ui.add_space(10.0);
                }
                kcap(ui, k, false);
                prev_text = false;
            }
            HintPart::Text(t) => {
                ui.label(RichText::new(t).font(FontId::proportional(13.0)).color(theme::DIM));
                prev_text = true;
            }
            HintPart::Head(t) => {
                ui.label(RichText::new(t.to_uppercase()).font(theme::glass_font(13.0)).color(theme::TEXT));
                prev_text = false;
            }
        }
    }
}

/// A 1 x 14 pt separator of the status bar (`.status .sep`).
pub fn status_sep(ui: &mut Ui) {
    let (r, _) = ui.allocate_exact_size(Vec2::new(1.0, 14.0), Sense::hover());
    ui.painter().rect_filled(r, 0.0, theme::line(0.18));
}

// ---------------------------------------------------------------------------------------------
// World Editor and requester (phase 4): palette tools, the options strip's pieces, outliner rows, requester buttons
// ---------------------------------------------------------------------------------------------

/// A tool of the World palette (`#palette .tool.sm`, 42x40): the icon alone. `on` = lit blue (the gizmo mode), `armed`
/// = orange with a glow (the place tool in use).
pub fn tool_sm(ui: &mut Ui, icon: &str, on: bool, armed: bool) -> Response {
    let (rect, r) = ui.allocate_exact_size(Vec2::new(42.0, 40.0), Sense::click());
    let hot = (r.hovered() || r.is_pointer_button_down_on()) && ui.is_enabled();
    let (fill, rim) = match (armed, on, hot) {
        (true, _, _) => (theme::ORANGE.gamma_multiply(0.08), theme::ORANGE),
        (_, true, _) => (s1(if hot { 0.2 } else { 0.14 }), s1(0.75)),
        (_, _, true) => (theme::white(0.05), theme::line(0.32)),
        _ => (Color32::TRANSPARENT, Color32::TRANSPARENT),
    };
    let c = if armed { theme::ORANGE } else if on { theme::S1 } else { theme::TEXT };
    ui.painter().rect(rect, theme::RADIUS_TOOL, fill, Stroke::new(1.0, rim), StrokeKind::Inside);
    if armed {
        // `text-shadow: 0 0 8px` of the armed icon
        ui.painter().circle_filled(rect.center(), 9.0, theme::ORANGE.gamma_multiply(0.12));
    }
    ui.painter().text(rect.center(), Align2::CENTER_CENTER, icon, FontId::proportional(18.0), ink(ui, c));
    r
}

/// The hairline between the palette's groups (30 x 1 with 5 pt above and below).
pub fn palette_sep(ui: &mut Ui) {
    ui.add_space(5.0);
    let (r, _) = ui.allocate_exact_size(Vec2::new(30.0, 1.0), Sense::hover());
    ui.painter().rect_filled(r, 0.0, theme::line(0.14));
    ui.add_space(5.0);
}

/// A dim caption of the options strip (`.cap`: 11 pt, capitals).
pub fn cap_label(ui: &mut Ui, text: &str) -> Response {
    ui.label(RichText::new(text.to_uppercase()).font(FontId::proportional(11.0)).color(theme::DIM))
}

/// A number field of any width with its unit inside the strip (`.num` 60-76 wide): a range gives the bar.
pub fn num_w(ui: &mut Ui, w: f32, v: &mut f32, speed: f32, range: Option<std::ops::RangeInclusive<f32>>, suffix: &str) -> Response {
    num_sfx(ui, w, v, speed, range, false, None, suffix)
}

/// `num_w` for a count: whole numbers with the range bar.
pub fn num_count(ui: &mut Ui, w: f32, v: &mut usize, range: std::ops::RangeInclusive<usize>) -> Response {
    let (lo, hi) = (*range.start(), *range.end());
    let r = ui
        .scope(|ui| {
            ui.style_mut().drag_value_text_style = egui::TextStyle::Monospace;
            ui.spacing_mut().interact_size = Vec2::new(w, NUM_H);
            crate::gui::wheel_drag(ui, v, Some(range), |d| d)
        })
        .inner;
    if hi > lo {
        let f = ((*v - lo) as f32 / (hi - lo) as f32).clamp(0.0, 1.0);
        let rect = r.rect;
        ui.painter().rect_filled(Rect::from_min_max(Pos2::new(rect.left() + 3.0, rect.bottom() - 3.0), Pos2::new(rect.left() + 3.0 + (rect.width() - 6.0) * f, rect.bottom() - 1.0)), 1.0, s1(0.55));
    }
    r
}

/// A choice box (`.choice`): a triangle, the value centred in Rajdhani 17, a triangle. -1 / 1 = a triangle was clicked.
pub fn choice_box(ui: &mut Ui, w: f32, shown: &str, can_prev: bool, can_next: bool) -> i8 {
    let (rect, r) = ui.allocate_exact_size(Vec2::new(w, 30.0), Sense::hover());
    let cy = rect.center().y;
    let mut out = 0;
    for (left, on, d) in [(true, can_prev, -1), (false, can_next, 1)] {
        let rc = Rect::from_center_size(Pos2::new(if left { rect.left() + 10.0 } else { rect.right() - 10.0 }, cy), Vec2::new(20.0, 24.0));
        let tr = ui.interact(rc, r.id.with(("tri", left)), if on { Sense::click() } else { Sense::hover() });
        tri_at(ui, rc, left, !on, tr.hovered(), None);
        if tr.clicked() {
            out = d;
        }
    }
    let g = ui.painter().layout(shown.to_string(), theme::glass_font(16.0), ink(ui, theme::WHITE), w - 44.0);
    ui.painter().galley(Pos2::new(rect.center().x - g.size().x / 2.0, cy - g.size().y / 2.0), g, Color32::PLACEHOLDER);
    out
}

/// An outliner kind header (`.ol-kind`, 28 tall): chevron, icon, name in capitals, the count. Returns its response
/// (a click flips the group); `open` draws the chevron down.
pub fn kind_header(ui: &mut Ui, open: bool, icon: &str, name: &str, count: usize) -> Response {
    let (rect, r) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 28.0), Sense::click());
    if r.hovered() {
        ui.painter().rect_filled(rect, 7.0, theme::white(0.04));
    }
    let cy = rect.center().y;
    let c = Pos2::new(rect.left() + 14.0, cy);
    let pts = if open { vec![c + Vec2::new(-4.0, -2.0), c + Vec2::new(0.0, 2.0), c + Vec2::new(4.0, -2.0)] } else { vec![c + Vec2::new(-2.0, -4.0), c + Vec2::new(2.0, 0.0), c + Vec2::new(-2.0, 4.0)] };
    ui.painter().add(egui::Shape::line(pts, Stroke::new(1.6, theme::DIM)));
    ui.painter().text(Pos2::new(rect.left() + 34.0, cy), Align2::CENTER_CENTER, icon, FontId::proportional(14.0), theme::DIM);
    let g = galley(ui, name.to_uppercase(), theme::glass_font(13.0), theme::TEXT);
    ui.painter().galley(Pos2::new(rect.left() + 50.0, cy - g.size().y / 2.0), g.clone(), Color32::PLACEHOLDER);
    let n = galley(ui, count.to_string(), FontId::proportional(12.0), theme::DIM);
    ui.painter().galley(Pos2::new(rect.left() + 50.0 + g.size().x + 8.0, cy - n.size().y / 2.0), n, Color32::PLACEHOLDER);
    r
}

/// An outliner piece row (`.ol-row`, 26 tall): the eye, the name, the size in mono on the right. `sel` = the orange rim,
/// `hidden` = dimmed. Returns (the row, the eye was clicked).
pub fn piece_row(ui: &mut Ui, hidden: bool, sel: bool, name: &str, meta: &str) -> (Response, bool) {
    let (rect, r) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 26.0), Sense::click());
    let fade = if hidden { 0.45 } else { 1.0 };
    if sel {
        ui.painter().rect(rect, 7.0, theme::ORANGE.gamma_multiply(0.06), Stroke::new(1.0, theme::ORANGE), StrokeKind::Inside);
    } else if r.hovered() {
        ui.painter().rect_filled(rect, 7.0, theme::white(0.04));
    }
    let cy = rect.center().y;
    let eye_r = Rect::from_center_size(Pos2::new(rect.left() + 43.0, cy), Vec2::new(22.0, 24.0));
    let eye = ui.interact(eye_r, r.id.with("eye"), Sense::click()).on_hover_text("hide while editing (not saved)");
    ui.painter().text(eye_r.center(), Align2::CENTER_CENTER, if hidden { crate::icons::EYE_OFF } else { crate::icons::EYE }, FontId::proportional(13.0), if eye.hovered() { theme::TEXT } else { theme::DIM }.gamma_multiply(fade));
    let c = if sel { theme::ORANGE } else { theme::TEXT }.gamma_multiply(fade);
    let g = galley(ui, name, theme::glass_font(14.0), c);
    ui.painter().galley(Pos2::new(rect.left() + 60.0, cy - g.size().y / 2.0), g, Color32::PLACEHOLDER);
    if !meta.is_empty() {
        let m = galley(ui, meta, FontId::monospace(11.5), theme::DIM.gamma_multiply(fade));
        ui.painter().galley(Pos2::new(rect.right() - 8.0 - m.size().x, cy - m.size().y / 2.0), m, Color32::PLACEHOLDER);
    }
    (r, eye.clicked())
}

/// A requester button (`.btn`, 184 x 44): icon 20 and Rajdhani Bold 22 capitals. `sel` = the default / keyboard choice:
/// orange text and rim with the 19.4 pulse halo.
pub fn req_button(ui: &mut Ui, icon: &str, text: &str, sel: bool) -> Response {
    let (rect, r) = ui.allocate_exact_size(Vec2::new(184.0, 44.0), Sense::click());
    let hot = r.hovered() || r.is_pointer_button_down_on();
    let k = 12.0;
    if sel {
        let a = pulse(ui.ctx());
        ui.painter().add(Shadow { offset: [0, 0], blur: 14, spread: 1, color: theme::ORANGE.gamma_multiply(a) }.as_shape(rect, k));
    }
    // `linear-gradient(rgba(255,255,255,.14), rgba(255,255,255,.04))` approximated by its middle
    ui.painter().rect(rect, k, theme::white(if hot { 0.14 } else { 0.09 }), Stroke::new(1.0, if sel { theme::ORANGE } else { theme::line(if hot { 0.6 } else { 0.38 }) }), StrokeKind::Inside);
    ui.painter().line_segment([Pos2::new(rect.left() + k, rect.top() + 1.5), Pos2::new(rect.right() - k, rect.top() + 1.5)], Stroke::new(1.0, theme::white(0.38)));
    let c = if sel { theme::ORANGE } else { theme::TEXT };
    let g = galley(ui, text, theme::glass_font(22.0), c);
    let total = 20.0 + 10.0 + g.size().x;
    let x = rect.center().x - total / 2.0;
    ui.painter().text(Pos2::new(x + 10.0, rect.center().y), Align2::CENTER_CENTER, icon, FontId::proportional(20.0), c);
    ui.painter().galley(Pos2::new(x + 30.0, rect.center().y - g.size().y / 2.0), g, Color32::PLACEHOLDER);
    r
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hints_parse() {
        let h = hint("*PLANT* [LMB] place [Shift] snap [Esc] cancel");
        assert_eq!(h[0], HintPart::Head("PLANT".into()));
        assert_eq!(h[1], HintPart::Key("LMB".into()));
        assert_eq!(h[2], HintPart::Text("place".into()));
        assert_eq!(h.len(), 7);
        assert_eq!(hint_text(&h), "PLANT [LMB] place [Shift] snap [Esc] cancel");
        assert_eq!(hint("WASD move"), vec![HintPart::Text("WASD move".into())]);
        assert!(hint("").is_empty());
    }
}
