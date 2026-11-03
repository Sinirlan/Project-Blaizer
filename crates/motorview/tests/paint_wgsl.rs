//! paint.wgsl (re-notes/21-paint-finishes.md 21.10) validates on its own and appended to shader.wgsl,
//! the way it will be built into the Remaster car pipeline (no name clashes).
use wgpu::naga;

fn validate(label: &str, src: &str) {
    let module = naga::front::wgsl::parse_str(src).unwrap_or_else(|e| panic!("{label}: {}", e.emit_to_string(src)));
    naga::valid::Validator::new(naga::valid::ValidationFlags::all(), naga::valid::Capabilities::all())
        .validate(&module)
        .unwrap_or_else(|e| panic!("{label}: {e:?}"));
}

#[test]
fn paint_wgsl_validates_alone_and_appended() {
    let paint = include_str!("../src/paint.wgsl");
    validate("paint.wgsl", paint);
    validate("shader.wgsl + paint.wgsl", &format!("{}\n{}", include_str!("../src/shader.wgsl"), paint));
}
