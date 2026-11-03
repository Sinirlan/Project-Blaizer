//! DEBUG egui host for motorview's menu bar and tuning window (re-notes/00-debuger.md): window
//! input through egui-winit, one pass per frame, drawn last over the frame by egui-wgpu.
//! The game does not use it, so Classic and Remaster output is unchanged.

use egui::emath::Numeric;
use egui_wgpu::{Renderer, RendererOptions, ScreenDescriptor};
use std::ops::RangeInclusive;
use winit::event::WindowEvent;
use winit::window::Window;

/// One egui pass, tessellated and ready to draw.
pub struct Painted {
    prims: Vec<egui::ClippedPrimitive>,
    textures: egui::TexturesDelta,
    ppp: f32,
}

pub struct Gui {
    pub ctx: egui::Context,
    /// window input; None for headless shots
    state: Option<egui_winit::State>,
    renderer: Renderer,
}

impl Gui {
    pub fn new(device: &wgpu::Device, format: wgpu::TextureFormat, window: Option<&Window>) -> Gui {
        let ctx = egui::Context::default();
        crate::theme::apply(&ctx);
        let max_tex = Some(device.limits().max_texture_dimension_2d as usize);
        let state = window.map(|w| egui_winit::State::new(ctx.clone(), egui::ViewportId::ROOT, w, Some(w.scale_factor() as f32), None, max_tex));
        Gui { ctx, state, renderer: Renderer::new(device, format, RendererOptions::default()) }
    }

    /// Feed a window event; true when egui took it (a click on a widget, typing in a field),
    /// so the viewer should not act on it.
    pub fn on_event(&mut self, window: &Window, ev: &WindowEvent) -> bool {
        self.state.as_mut().is_some_and(|s| s.on_window_event(window, ev).consumed)
    }

    /// egui has the keyboard (a text or number field is being edited).
    pub fn wants_keys(&self) -> bool {
        self.ctx.egui_wants_keyboard_input()
    }

    /// egui is under the mouse or dragging a widget.
    pub fn wants_pointer(&self) -> bool {
        self.ctx.egui_wants_pointer_input() || self.ctx.is_pointer_over_egui()
    }

    /// One pass with the window's input (a root `Ui`: panels and windows).
    pub fn run(&mut self, window: &Window, ui: impl FnMut(&mut egui::Ui)) -> Option<Painted> {
        let s = self.state.as_mut()?;
        let out = self.ctx.run_ui(s.take_egui_input(window), ui);
        s.handle_platform_output(window, out.platform_output);
        Some(Painted { prims: self.ctx.tessellate(out.shapes, out.pixels_per_point), textures: out.textures_delta, ppp: out.pixels_per_point })
    }

    /// Headless pass for a `size` pixel shot. Two passes: the first lays the windows out.
    pub fn run_headless(&mut self, size: (u32, u32), mut ui: impl FnMut(&mut egui::Ui)) -> Painted {
        // a second apart, so the windows' fade-in has finished
        let input = |time| egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(size.0 as f32, size.1 as f32))), time: Some(time), ..Default::default() };
        let first = self.ctx.run_ui(input(0.0), &mut ui);
        let out = self.ctx.run_ui(input(1.0), &mut ui);
        // the first pass's delta holds the font atlas
        let mut textures = first.textures_delta;
        textures.append(out.textures_delta);
        Painted { prims: self.ctx.tessellate(out.shapes, out.pixels_per_point), textures, ppp: out.pixels_per_point }
    }

    /// Draw a pass over `target` (loaded, no depth).
    pub fn paint(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, enc: &mut wgpu::CommandEncoder, target: &wgpu::TextureView, size: (u32, u32), p: Painted) {
        for (id, deltas) in &p.textures.set {
            for d in deltas {
                self.renderer.update_texture(device, queue, *id, d);
            }
        }
        let sd = ScreenDescriptor { size_in_pixels: [size.0, size.1], pixels_per_point: p.ppp };
        let extra = self.renderer.update_buffers(device, queue, enc, &p.prims, &sd);
        if !extra.is_empty() {
            queue.submit(extra);
        }
        let mut pass = enc
            .begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("egui"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment { view: target, depth_slice: None, resolve_target: None, ops: wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Store } })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            })
            .forget_lifetime();
        self.renderer.render(&mut pass, &p.prims, &sd);
        drop(pass);
        for id in &p.textures.free {
            self.renderer.free_texture(id);
        }
    }
}

/// DEBUG a slider that also steps with the mouse wheel while hovered (Shift = finer, `wheel_step`).
pub fn wheel_slider<N: Numeric>(ui: &mut egui::Ui, v: &mut N, range: RangeInclusive<N>, f: impl for<'b> FnOnce(egui::Slider<'b>) -> egui::Slider<'b>) -> egui::Response {
    let (lo, hi) = (range.start().to_f64(), range.end().to_f64());
    let mut r = ui.add(f(egui::Slider::new(v, range)));
    apply_wheel(ui, &mut r, v, lo, hi);
    r
}

/// DEBUG a number field that also steps with the mouse wheel while hovered; `range` None = unbounded.
pub fn wheel_drag<N: Numeric>(ui: &mut egui::Ui, v: &mut N, range: Option<RangeInclusive<N>>, f: impl for<'b> FnOnce(egui::DragValue<'b>) -> egui::DragValue<'b>) -> egui::Response {
    let range = range.unwrap_or(N::MIN..=N::MAX);
    let (lo, hi) = (range.start().to_f64(), range.end().to_f64());
    let mut r = ui.add(f(egui::DragValue::new(v).range(range)));
    apply_wheel(ui, &mut r, v, lo, hi);
    r
}

/// Wheel notches over a hovered widget step `v`. Reads the raw wheel events: egui's own scroll
/// delta turns Shift+wheel into horizontal scrolling. Consumes the scroll so a panel's
/// `ScrollArea` stays put.
fn apply_wheel<N: Numeric>(ui: &mut egui::Ui, r: &mut egui::Response, v: &mut N, lo: f64, hi: f64) {
    if !r.hovered() || !r.enabled() || r.dragged() {
        return;
    }
    let (mut amount, mut fine) = (0.0, false);
    ui.input_mut(|i| {
        for e in &i.events {
            if let egui::Event::MouseWheel { unit, delta, modifiers, .. } = e {
                // points (touchpads) to notches
                let k = if *unit == egui::MouseWheelUnit::Point { 1.0 / 40.0 } else { 1.0 };
                amount += (delta.x + delta.y) * k;
                fine |= modifiers.shift;
            }
        }
        i.smooth_scroll_delta = egui::Vec2::ZERO;
    });
    if amount == 0.0 {
        return;
    }
    let id = r.id.with("wheel");
    let total = amount + ui.data_mut(|d| d.get_temp::<f32>(id)).unwrap_or(0.0);
    let notches = total.trunc();
    ui.data_mut(|d| d.insert_temp(id, total - notches));
    if notches != 0.0 {
        let x = wheel_step(v.to_f64(), lo, hi, notches as i32, fine, N::INTEGRAL);
        if x != v.to_f64() {
            *v = N::from_f64(x);
            r.mark_changed();
        }
    }
}

/// The wheel step: 1 (Shift 0.1); on a range narrower than 10, range/100 rounded to a power of
/// ten (Shift a tenth of it); integers always 1. Lands on the step grid, clamped to the range.
fn wheel_step(v: f64, lo: f64, hi: f64, notches: i32, fine: bool, integral: bool) -> f64 {
    let w = hi - lo;
    let base = if integral || w >= 10.0 { 1.0 } else { 10f64.powf((w / 100.0).log10().round()) };
    let step = if fine && !integral { base / 10.0 } else { base };
    // the next grid point in the wheel's direction, so 0.537 goes to 0.54 / 0.53
    let g = v / step;
    let g = if notches > 0 { (g + 1e-6).floor() } else { (g - 1e-6).ceil() };
    ((g + notches as f64) * step).clamp(lo, hi)
}

#[cfg(test)]
mod tests {
    use super::wheel_step;

    fn near(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-9
    }

    #[test]
    fn steps() {
        assert!(near(wheel_step(0.5, 0.0, 1.0, 1, false, false), 0.51));
        assert!(near(wheel_step(0.5, 0.0, 1.0, -1, true, false), 0.499));
        assert!(near(wheel_step(50.0, 0.0, 100.0, 1, false, false), 51.0));
        assert!(near(wheel_step(50.0, 0.0, 100.0, -1, true, false), 49.9));
        assert!(near(wheel_step(5.0, 0.0, 100.0, 2, true, true), 7.0));
        assert!(near(wheel_step(0.537, 0.0, 1.0, 1, false, false), 0.54));
        assert!(near(wheel_step(0.537, 0.0, 1.0, -1, false, false), 0.53));
        assert!(near(wheel_step(0.995, 0.0, 1.0, 3, false, false), 1.0));
        assert!(near(wheel_step(3.0, f64::MIN, f64::MAX, -1, false, false), 2.0));
        // 0.02..=2: range/100 = 0.0198 -> 0.01
        assert!(near(wheel_step(1.0, 0.02, 2.0, 1, false, false), 1.01));
    }
}
