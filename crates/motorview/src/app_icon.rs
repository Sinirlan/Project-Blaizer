//! The application icons (Glass M, Smoke variant; `assets/icon/`, source and packs in
//! `reference_materials/ui-overhaul/icon-overhaul`): the window icon of every program that opens a window, and the
//! app id a Linux desktop entry (`assets/icon/motorhead.desktop`) matches. The game's icon is built in (`apply`, `rgba`);
//! the tools (`assets/icon/tools/<bin>/`, #407) pass their own 256 px PNG to `apply_with` / `decode`. The Windows `.exe`
//! icon is a resource added by each crate's build.rs.

use winit::window::{Icon, WindowAttributes};

/// Linux app id / WM class: `StartupWMClass` and the icon name of `motorhead.desktop`.
pub const APP_ID: &str = "motorhead";

const PNG_256: &[u8] = include_bytes!("../../../assets/icon/motorhead-256.png");

/// The game's 256 px icon as RGBA, for any toolkit (`egui::IconData`).
pub fn rgba() -> Option<(u32, u32, Vec<u8>)> {
    decode(PNG_256)
}

/// An 8-bit RGBA PNG as (width, height, pixels); `None` if it is anything else.
pub fn decode(png: &[u8]) -> Option<(u32, u32, Vec<u8>)> {
    let mut reader = ::png::Decoder::new(png).read_info().ok()?;
    let mut buf = vec![0; reader.output_buffer_size()];
    let info = reader.next_frame(&mut buf).ok()?;
    (info.color_type == ::png::ColorType::Rgba && info.bit_depth == ::png::BitDepth::Eight).then_some((info.width, info.height, buf))
}

pub fn window_icon() -> Option<Icon> {
    icon_of(PNG_256)
}

fn icon_of(png: &[u8]) -> Option<Icon> {
    let (w, h, px) = decode(png)?;
    Icon::from_rgba(px, w, h).ok()
}

/// `attrs` with the game's icon and app id (Wayland `app_id`, X11 class) set.
pub fn apply(attrs: WindowAttributes) -> WindowAttributes {
    apply_with(attrs, APP_ID, PNG_256)
}

/// `attrs` with a tool's icon (`png` = its `assets/icon/tools/<bin>/<bin>-256.png`) and its own app id set.
pub fn apply_with(attrs: WindowAttributes, app_id: &str, png: &[u8]) -> WindowAttributes {
    let attrs = attrs.with_window_icon(icon_of(png));
    #[cfg(all(unix, not(target_os = "macos")))]
    let attrs = {
        use winit::platform::{wayland::WindowAttributesExtWayland, x11::WindowAttributesExtX11};
        let attrs = WindowAttributesExtWayland::with_name(attrs, app_id, app_id);
        WindowAttributesExtX11::with_name(attrs, app_id, app_id)
    };
    #[cfg(not(all(unix, not(target_os = "macos"))))]
    let _ = app_id;
    attrs
}
