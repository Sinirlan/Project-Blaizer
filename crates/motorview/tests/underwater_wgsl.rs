//! underwater.wgsl (CHEAT ramlosa / h2o, #317, the wobble image pass) validates on its own.
use wgpu::naga;

#[test]
fn underwater_wgsl_validates() {
    let src = include_str!("../src/underwater.wgsl");
    let module = naga::front::wgsl::parse_str(src).unwrap_or_else(|e| panic!("underwater.wgsl: {}", e.emit_to_string(src)));
    naga::valid::Validator::new(naga::valid::ValidationFlags::all(), naga::valid::Capabilities::all())
        .validate(&module)
        .unwrap_or_else(|e| panic!("underwater.wgsl: {e:?}"));
}
