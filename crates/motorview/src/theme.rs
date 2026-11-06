//! DEBUG motorview's egui look: the Glass HUD tokens (re-notes/19-glass-hud-design-language.md 19.2) at tool
//! size, after the concept A mockup (`reference_materials/ui-overhaul/motorview-overhaul/assets/motorview.css`,
//! #395). Fonts: Rajdhani SemiBold leads the proportional family (Rajdhani Bold `GLASS`, Orbitron `BRAND`
//! as named families), JetBrains Mono the monospace one, egui's own fonts after them for the glyphs
//! Rajdhani lacks and the Lucide icon font (`icons.rs`) last. The glass widgets are in `glass_ui.rs`.
//! Units are egui points (a 1920x1080 window at pixels_per_point 1). The game does not use it.
//!
//! Colour rule (19.4 for a tool): orange = where you are (open tab, hovered / selected item, armed tool);
//! sector-1 blue = a state that is on; white = values; `TEXT` / `DIM` = labels.

use egui::{epaint::CornerRadius, Color32, Context, FontData, FontDefinitions, FontFamily, FontId, Shadow, Stroke, TextStyle, Vec2};
use std::sync::Arc;

/// labels (`--text`)
pub const TEXT: Color32 = Color32::from_rgb(0xbf, 0xe3, 0xf3);
/// captions, units, weak text (`--dim`)
pub const DIM: Color32 = Color32::from_rgb(0x8f, 0xb5, 0xc9);
/// values
pub const WHITE: Color32 = Color32::WHITE;
/// where you are: open tab, hovered / selected item, armed tool (`--orange`)
pub const ORANGE: Color32 = Color32::from_rgb(0xff, 0x9a, 0x3c);
/// orange text and the modified dot (`--orange-text`)
pub const ORANGE_TEXT: Color32 = Color32::from_rgb(0xff, 0xb2, 0x6a);
/// a state that is on: lit tool icon, slider fill, current choice (`--s1`)
pub const S1: Color32 = Color32::from_rgb(0x8f, 0xd0, 0xff);
/// the sector-1 glow (`#4aa8ff`)
pub const S1_GLOW: Color32 = Color32::from_rgb(0x4a, 0xa8, 0xff);
/// the text on a lit item (`--name`)
pub const NAME: Color32 = Color32::from_rgb(0xf4, 0xfb, 0xff);
/// the brand plate grey and the version text (`--plate`, `--version`)
pub const PLATE: Color32 = Color32::from_rgb(0xaa, 0xb4, 0xc2);
pub const VERSION: Color32 = Color32::from_rgb(0x4a, 0x55, 0x66);
/// X / Y / Z field tints (`.ax.x` / `.ax.y` / `.ax.z`)
pub const AXIS: [Color32; 3] = [Color32::from_rgb(0xff, 0x8a, 0x7a), Color32::from_rgb(0x7c, 0xf0, 0xa8), S1];

/// `rgba(--line, a)`: the glass rim and hairline colour `#bee6ff` at alpha `a`.
pub const fn line(a: f32) -> Color32 {
    Color32::from_rgba_unmultiplied_const(190, 230, 255, (a * 255.0 + 0.5) as u8)
}

/// white at alpha `a` (hover washes, the glass top highlight)
pub const fn white(a: f32) -> Color32 {
    Color32::from_rgba_unmultiplied_const(255, 255, 255, (a * 255.0 + 0.5) as u8)
}

/// Tool glass fill: `.glass` navy `rgba(6,14,28,.52)` under the white .09 to .025 gradient, flattened to its
/// middle (.0575) [APPROX: egui frames have one fill; the gradient's top end is the 1 pt highlight].
pub const GLASS: Color32 = Color32::from_rgba_unmultiplied_const(32, 39, 52, 140);
/// The same glass, opaque, over a dark scene: Solid panels (no frost behind them).
pub const GLASS_SOLID: Color32 = Color32::from_rgb(21, 27, 38);
/// A value tile / group bed inside the glass (`.vtile` `rgba(2,8,18,.32)`).
pub const TILE: Color32 = Color32::from_rgba_unmultiplied_const(2, 8, 18, 82);
/// A number or text field (`.num` `rgba(2,8,18,.55)`).
pub const FIELD: Color32 = Color32::from_rgba_unmultiplied_const(2, 8, 18, 140);
/// Tooltips, menus, popups and the requester: the `.tip` card, nearly opaque (no frost behind them).
pub const POPUP: Color32 = Color32::from_rgba_unmultiplied_const(14, 24, 40, 240);

/// Glass card corner radius (`.glass` 12), tool / tile radius (9), field radius (6).
pub const RADIUS: u8 = 12;
pub const RADIUS_TOOL: u8 = 9;
pub const RADIUS_FIELD: u8 = 6;

/// Rajdhani Bold: titles, tabs and values (`font: 700 … 'Rajdhani'`).
pub const GLASS_FAMILY: &str = "glass";
/// Orbitron ExtraBold: the MOTORVIEW brand.
pub const BRAND_FAMILY: &str = "brand";

/// A Rajdhani Bold font id.
pub fn glass_font(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name(GLASS_FAMILY.into()))
}

/// The tool glass shadow (`0 10px 26px rgba(0,0,0,.45)`).
pub const SHADOW: Shadow = Shadow { offset: [0, 10], blur: 26, spread: 0, color: Color32::from_black_alpha(115) };
/// Tooltip / popup shadow (`.tip` `0 8px 22px rgba(0,0,0,.5)`).
const POPUP_SHADOW: Shadow = Shadow { offset: [0, 8], blur: 22, spread: 0, color: Color32::from_black_alpha(128) };

fn font(name: &str, bytes: &'static [u8]) -> (String, Arc<FontData>) {
    (name.into(), Arc::new(FontData::from_static(bytes)))
}

pub fn apply(ctx: &Context) {
    let mut fonts = FontDefinitions::default();
    let defaults = fonts.families.get(&FontFamily::Proportional).cloned().unwrap_or_default();
    // OFL fonts, assets/fonts/LICENSES.md; Lucide (ISC) is the icon font
    fonts.font_data.extend([
        font("rajdhani", include_bytes!("../../../assets/fonts/Rajdhani-SemiBold.ttf")),
        font("rajdhani-bold", include_bytes!("../../../assets/fonts/Rajdhani-Bold.ttf")),
        font("orbitron", include_bytes!("../../../assets/fonts/Orbitron-ExtraBold.ttf")),
        font("jetbrains", include_bytes!("../../../assets/fonts/JetBrainsMono-Regular.ttf")),
        font("lucide", include_bytes!("../../../assets/fonts/lucide/lucide.ttf")),
    ]);
    fonts.families.entry(FontFamily::Proportional).or_default().insert(0, "rajdhani".into());
    fonts.families.entry(FontFamily::Monospace).or_default().insert(0, "jetbrains".into());
    for (name, lead) in [(GLASS_FAMILY, "rajdhani-bold"), (BRAND_FAMILY, "orbitron")] {
        fonts.families.insert(FontFamily::Name(name.into()), std::iter::once(lead.to_string()).chain(defaults.iter().cloned()).collect());
    }
    for fam in [FontFamily::Proportional, FontFamily::Monospace, FontFamily::Name(GLASS_FAMILY.into()), FontFamily::Name(BRAND_FAMILY.into())] {
        fonts.families.entry(fam).or_default().push("lucide".into());
    }
    ctx.set_fonts(fonts);

    ctx.global_style_mut(|s| {
        use FontFamily::{Monospace, Proportional};
        // Rajdhani runs small for its size: the mockup's 14 pt labels, 11 pt captions
        s.text_styles = [
            (TextStyle::Small, FontId::new(11.0, Proportional)),
            (TextStyle::Body, FontId::new(14.0, Proportional)),
            (TextStyle::Button, FontId::new(14.0, Proportional)),
            (TextStyle::Heading, glass_font(20.0)),
            (TextStyle::Monospace, FontId::new(12.5, Monospace)),
        ]
        .into();

        let v = &mut s.visuals;
        *v = egui::Visuals::dark();
        v.override_text_color = None;
        v.weak_text_color = Some(DIM);
        v.panel_fill = GLASS;
        v.window_fill = POPUP;
        v.window_stroke = Stroke::new(1.0, line(0.35));
        v.window_shadow = SHADOW;
        v.popup_shadow = POPUP_SHADOW;
        v.extreme_bg_color = FIELD;
        v.text_edit_bg_color = Some(FIELD);
        v.code_bg_color = FIELD;
        v.faint_bg_color = white(0.035);
        // a lit choice: sector-1 at .30 (egui's selected button has no rim of its own) [APPROX of `.on`]
        v.selection.bg_fill = Color32::from_rgba_unmultiplied_const(143, 208, 255, 77);
        v.selection.stroke = Stroke::new(1.0, NAME);
        v.hyperlink_color = S1;
        v.slider_trailing_fill = true;
        v.window_corner_radius = CornerRadius::same(RADIUS);
        v.menu_corner_radius = CornerRadius::same(RADIUS_TOOL);
        let r = CornerRadius::same(RADIUS_FIELD);
        let w = &mut v.widgets;
        // (state, button wash, field / rail fill, rim, ink)
        for (st, wash, fill, rim, ink) in [
            (&mut w.noninteractive, Color32::TRANSPARENT, FIELD, Stroke::new(1.0, line(0.14)), TEXT),
            (&mut w.inactive, white(0.05), FIELD, Stroke::new(1.0, line(0.22)), TEXT),
            (&mut w.hovered, white(0.09), FIELD, Stroke::new(1.0, line(0.45)), WHITE),
            (&mut w.active, Color32::from_rgba_unmultiplied_const(255, 154, 60, 31), FIELD, Stroke::new(1.0, ORANGE), WHITE),
            (&mut w.open, white(0.07), FIELD, Stroke::new(1.0, ORANGE), WHITE),
        ] {
            st.corner_radius = r;
            st.weak_bg_fill = wash;
            st.bg_fill = fill;
            st.bg_stroke = rim;
            st.fg_stroke = Stroke::new(1.0, ink);
            st.expansion = 0.0;
        }

        s.spacing.item_spacing = Vec2::new(6.0, 4.0);
        s.spacing.button_padding = Vec2::new(8.0, 3.0);
        s.spacing.interact_size.y = 22.0;
        s.spacing.slider_width = 90.0;
    });
}
