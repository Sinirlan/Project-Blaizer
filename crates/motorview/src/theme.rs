//! DEBUG motorview's egui look (re-notes/15-motorview-overhaul.md): a Blender-like dark theme,
//! the Lucide icon font (`icons.rs`) as a fallback of both families, and the colours the
//! ribbon draws with. All the numbers of the look are here. The game does not use it.

use egui::{epaint::CornerRadius, Color32, Context, FontData, FontDefinitions, FontFamily, Stroke, Vec2};
use std::sync::Arc;

pub const PANEL: Color32 = Color32::from_rgb(0x2b, 0x2b, 0x2b);
pub const PANEL_DARK: Color32 = Color32::from_rgb(0x1f, 0x1f, 0x1f);
pub const WIDGET: Color32 = Color32::from_rgb(0x3d, 0x3d, 0x3d);
pub const HOVER: Color32 = Color32::from_rgb(0x4b, 0x4b, 0x4b);
/// Blender's blue: active tool, selected tab, toggled-on chip
pub const ACCENT: Color32 = Color32::from_rgb(0x47, 0x72, 0xb3);
pub const ACCENT_HOT: Color32 = Color32::from_rgb(0x5a, 0x88, 0xcc);
pub const TEXT: Color32 = Color32::from_rgb(0xd8, 0xd8, 0xd8);
pub const TEXT_WEAK: Color32 = Color32::from_rgb(0x8c, 0x8c, 0x8c);
pub const STROKE: Color32 = Color32::from_rgb(0x14, 0x14, 0x14);

const RADIUS: u8 = 4;

pub fn apply(ctx: &Context) {
    let mut fonts = FontDefinitions::default();
    fonts.font_data.insert("lucide".into(), Arc::new(FontData::from_static(include_bytes!("../../../assets/fonts/lucide/lucide.ttf"))));
    // overlays and number fields: JetBrains Mono (OFL, assets/fonts/LICENSES.md) leads the monospace family
    fonts.font_data.insert("jetbrains".into(), Arc::new(FontData::from_static(include_bytes!("../../../assets/fonts/JetBrainsMono-Regular.ttf"))));
    fonts.families.entry(FontFamily::Monospace).or_default().insert(0, "jetbrains".into());
    for fam in [FontFamily::Proportional, FontFamily::Monospace] {
        fonts.families.entry(fam).or_default().push("lucide".into());
    }
    ctx.set_fonts(fonts);

    ctx.global_style_mut(|s| {
        let v = &mut s.visuals;
        *v = egui::Visuals::dark();
        v.panel_fill = PANEL;
        v.window_fill = PANEL;
        v.extreme_bg_color = PANEL_DARK;
        v.faint_bg_color = Color32::from_rgb(0x32, 0x32, 0x32);
        v.override_text_color = Some(TEXT);
        v.selection.bg_fill = ACCENT;
        v.selection.stroke = Stroke::new(1.0, Color32::WHITE);
        v.hyperlink_color = ACCENT_HOT;
        let r = CornerRadius::same(RADIUS);
        v.window_corner_radius = CornerRadius::same(RADIUS + 2);
        v.menu_corner_radius = r;
        let w = &mut v.widgets;
        for (st, fill) in [(&mut w.inactive, WIDGET), (&mut w.hovered, HOVER), (&mut w.active, ACCENT_HOT), (&mut w.open, HOVER)] {
            st.corner_radius = r;
            st.weak_bg_fill = fill;
            st.bg_fill = fill;
            st.bg_stroke = Stroke::new(1.0, STROKE);
            st.fg_stroke = Stroke::new(1.0, TEXT);
        }
        w.noninteractive.corner_radius = r;
        w.noninteractive.bg_stroke = Stroke::new(1.0, STROKE);
        w.noninteractive.fg_stroke = Stroke::new(1.0, TEXT);
        w.hovered.fg_stroke = Stroke::new(1.0, Color32::WHITE);

        s.spacing.item_spacing = Vec2::new(4.0, 3.0);
        s.spacing.button_padding = Vec2::new(6.0, 3.0);
        s.spacing.interact_size.y = 22.0;
        s.spacing.slider_width = 90.0;
    });
}
