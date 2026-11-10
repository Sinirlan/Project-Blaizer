//! dof.wgsl (REMASTER cine depth of field, #421) validates with both depth kinds `DEPTH_TEX` stands for.
use wgpu::naga;

#[test]
fn dof_wgsl_validates() {
    for ty in ["texture_depth_2d", "texture_depth_multisampled_2d"] {
        let src = include_str!("../src/dof.wgsl").replace("DEPTH_TEX", ty);
        let module = naga::front::wgsl::parse_str(&src).unwrap_or_else(|e| panic!("dof.wgsl ({ty}): {}", e.emit_to_string(&src)));
        naga::valid::Validator::new(naga::valid::ValidationFlags::all(), naga::valid::Capabilities::all())
            .validate(&module)
            .unwrap_or_else(|e| panic!("dof.wgsl ({ty}): {e:?}"));
    }
}
