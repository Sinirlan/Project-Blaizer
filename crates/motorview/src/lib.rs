//! Rendering and session code shared by `motorview` (track/car viewer and driving test) and
//! `motorhead` (the game: front end, HUD and races).

pub mod cine;
pub mod cine_render;
pub mod cine_edit;
pub mod consts;
pub mod debug;
pub mod drive;
pub mod engine_sim;
pub mod world_edit;
pub mod fill_gizmo;
pub mod fog;
pub mod fx;
pub mod ghost;
pub mod loading;
pub mod glass_ui;
pub mod gui;
pub mod icons;
pub mod inspect;
pub mod minimap;
pub mod palette;
pub mod pick;
pub mod png;
pub mod previews;
pub mod probes;
pub mod race_audio;
pub mod procgen;
pub mod render;
pub mod revmeter;
pub mod scene;
pub mod shaders;
pub mod sky;
pub mod texpack;
pub mod theme;
pub mod timeline;
pub mod tune;
pub mod tune_changes;
pub mod ui;
pub mod app_icon;
pub mod ui_font;
pub mod ui_sdf;
pub mod ui_vector;
pub mod view_settings;
pub mod wire;
pub mod volfog;

pub fn instance() -> wgpu::Instance {
    wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env())
}

pub async fn device(adapter: &wgpu::Adapter) -> (wgpu::Device, wgpu::Queue) {
    adapter
        .request_device(&wgpu::DeviceDescriptor {
            label: Some("motorview"),
            // REMASTER: MSAA counts other than 1 and 4 (8x) need the adapter's format features
            required_features: adapter.features() & wgpu::Features::TEXTURE_ADAPTER_SPECIFIC_FORMAT_FEATURES,
            required_limits: wgpu::Limits {
                max_texture_array_layers: adapter.limits().max_texture_array_layers.clamp(256, scene::MAX_LAYERS as u32),
                ..wgpu::Limits::default().using_resolution(adapter.limits())
            },
            ..Default::default()
        })
        .await
        .expect("request device")
}
