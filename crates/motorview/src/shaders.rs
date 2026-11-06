//! The WGSL sources (#398): every shader file of motorview, compiled into the program, with the Constants page's
//! overrides of their `const`s put in (`motor_tune::wgsl::patch`). render.rs and volfog.rs build their shader modules
//! from here, so a rebuild (`Gpu::rebuild_shaders`) picks up changed values. Without overrides a source is the file
//! byte for byte.

use std::borrow::Cow;

/// Every shader file, by name.
const FILES: [(&str, &str); 21] = [
    ("aa.wgsl", include_str!("aa.wgsl")),
    ("bloom.wgsl", include_str!("bloom.wgsl")),
    ("blur.wgsl", include_str!("blur.wgsl")),
    ("flarevis.wgsl", include_str!("flarevis.wgsl")),
    ("glow.wgsl", include_str!("glow.wgsl")),
    ("grass.wgsl", include_str!("grass.wgsl")),
    ("holofrost.wgsl", include_str!("holofrost.wgsl")),
    ("paint.wgsl", include_str!("paint.wgsl")),
    ("post.wgsl", include_str!("post.wgsl")),
    ("scenefrost.wgsl", include_str!("scenefrost.wgsl")),
    ("shader.wgsl", include_str!("shader.wgsl")),
    ("smaa.wgsl", include_str!("smaa.wgsl")),
    ("ssao.wgsl", include_str!("ssao.wgsl")),
    ("studioaa.wgsl", include_str!("studioaa.wgsl")),
    ("taa.wgsl", include_str!("taa.wgsl")),
    ("trail.wgsl", include_str!("trail.wgsl")),
    ("ui.wgsl", include_str!("ui.wgsl")),
    ("underwater.wgsl", include_str!("underwater.wgsl")),
    ("vblur.wgsl", include_str!("vblur.wgsl")),
    ("volfog.wgsl", include_str!("volfog.wgsl")),
    ("water.wgsl", include_str!("water.wgsl")),
];

/// The source of the shader file `name` (`"post.wgsl"`), overrides applied.
pub fn src(name: &str) -> Cow<'static, str> {
    let (_, text) = FILES.iter().find(|f| f.0 == name).unwrap_or_else(|| panic!("no shader file {name}"));
    motor_tune::wgsl::patch(name, text)
}

/// The scene shader: shader.wgsl plus the REMASTER paint finish model (paint.wgsl, re-notes/21), which `fs_remaster`
/// calls. Every module built from shader.wgsl (grass, water, holofrost) starts from this, so the `fin_*` names resolve.
pub fn scene() -> String {
    format!("{}\n{}", src("shader.wgsl"), src("paint.wgsl"))
}

#[cfg(test)]
mod tests {
    /// Every .wgsl beside this file is in `FILES` (a new shader file must be added there to be built from it).
    #[test]
    fn lists_every_shader_file() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        for e in std::fs::read_dir(&dir).expect("src").flatten() {
            let n = e.file_name().to_string_lossy().to_string();
            if n.ends_with(".wgsl") {
                assert!(super::FILES.iter().any(|f| f.0 == n), "{n} is not in shaders::FILES");
            }
        }
    }
}
