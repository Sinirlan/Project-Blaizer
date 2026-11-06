//! #398 phase 5: the graphics values are `tunable!`s. A value set through the registry (the Constants page, tune.cfg) is
//! what the renderer reads, and it goes to and from `tune.cfg` text under its `crate::module::NAME` key. One test in its own
//! process (the values are `static`s of the program, so a lib test that changed one would move the others running beside it).

use motorview::render::{CarDetail, CarShading, RemasterView, RenderMode, View, CLASSIC_MAX_DISTANCE};

fn view() -> View {
    View {
        brightness: 1.0,
        fog: true,
        show_line: false,
        shadows: true,
        halos: true,
        car_shading: CarShading::Reflection,
        car_detail: CarDetail::High,
        distance: 1.0,
        fov_x: 90.0,
        sky_image: true,
        motion_blur: false,
        lens_flares: true,
        smoke: true,
        sparks: true,
        mode: RenderMode::Classic,
        remaster: RemasterView::default(),
        debug: Default::default(),
    }
}

#[test]
fn a_graphics_value_set_through_the_registry_reaches_the_renderer_and_tune_cfg() {
    let t = motor_tune::find("crates/motorview/src/render.rs", "CLASSIC_MAX_DISTANCE").expect("CLASSIC_MAX_DISTANCE is registered");
    let mut v = view();
    v.distance = 1.4;
    assert_eq!(v.distance(), 1.4, "the source's cap is 1.5");
    assert!(!t.modified());

    // the registry side reaches the renderer
    t.set_value(1.25);
    assert_eq!(CLASSIC_MAX_DISTANCE.get(), 1.25);
    assert_eq!(v.distance(), 1.25);
    assert!(t.modified());

    // and tune.cfg: the value is saved under its key, a fresh start reads it back
    let saved = motor_tune::save_string();
    assert!(saved.lines().any(|l| l == "motorview::render::CLASSIC_MAX_DISTANCE 1.25"), "{saved}");
    t.reset();
    assert_eq!(v.distance(), 1.4);
    let loaded = motor_tune::load_str("motorview::render::CLASSIC_MAX_DISTANCE 0.8\n");
    assert_eq!(loaded.applied, 1);
    assert_eq!(v.distance(), 0.8);
    t.reset();
    assert!(!t.modified());
}
