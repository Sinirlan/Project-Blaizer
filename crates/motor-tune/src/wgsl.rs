//! DEBUG shader values (#398): overrides of WGSL `const`s, applied when motorview builds its shader modules
//! (`motorview::shaders`). A WGSL constant is compiled into the shader, so a changed value needs the shaders rebuilt
//! (`Gpu::rebuild_shaders`); the game picks up the overrides of `tune.cfg` when it builds them at start.
//!
//! Keyed by the shader's file name (`shader.wgsl`, unique in motorview) and the constant's name; `tune.cfg` writes
//! them as `shader.wgsl::SHADOW_WORLD 0.5`.

use std::borrow::Cow;
use std::collections::BTreeMap;
use std::sync::Mutex;

/// One override: the value expression, and whether `tune.cfg` keeps it (false once written back to the source:
/// it stays applied only because this build still holds the old value).
#[derive(Clone, Debug, PartialEq)]
struct Over {
    value: String,
    save: bool,
}

static OVER: Mutex<BTreeMap<(String, String), Over>> = Mutex::new(BTreeMap::new());

fn store() -> std::sync::MutexGuard<'static, BTreeMap<(String, String), Over>> {
    OVER.lock().unwrap_or_else(|e| e.into_inner())
}

/// `crates/motorview/src/shader.wgsl` (or `\` paths) -> `shader.wgsl`.
pub fn file_key(file: &str) -> &str {
    file.rsplit(['/', '\\']).next().unwrap_or(file)
}

/// Override the value of `name` in `file` (None: the source's value again). `tune.cfg` keeps it.
pub fn set(file: &str, name: &str, value: Option<&str>) {
    let k = (file_key(file).to_string(), name.to_string());
    match value {
        Some(v) => {
            store().insert(k, Over { value: v.trim().to_string(), save: true });
        }
        None => {
            store().remove(&k);
        }
    }
}

/// The override has been written into the source: it stays applied (this build still holds the old value) but
/// is no longer saved to `tune.cfg`.
pub fn mark_written(file: &str, name: &str) {
    if let Some(o) = store().get_mut(&(file_key(file).to_string(), name.to_string())) {
        o.save = false;
    }
}

/// The overridden value of `name` in `file`.
pub fn get(file: &str, name: &str) -> Option<String> {
    store().get(&(file_key(file).to_string(), name.to_string())).map(|o| o.value.clone())
}

/// Whether `tune.cfg` keeps the override (it differs from the source).
pub fn saved(file: &str, name: &str) -> bool {
    store().get(&(file_key(file).to_string(), name.to_string())).is_some_and(|o| o.save)
}

/// Every override: (file name, constant, value, saved).
pub fn all() -> Vec<(String, String, String, bool)> {
    store().iter().map(|((f, n), o)| (f.clone(), n.clone(), o.value.clone(), o.save)).collect()
}

/// Drop every override (the source's values again).
pub fn clear() {
    store().clear();
}

/// `tune.cfg` lines of the saved overrides (`file.wgsl::NAME value`).
pub(crate) fn save_lines() -> Vec<String> {
    store().iter().filter(|(_, o)| o.save).map(|((f, n), o)| format!("{f}::{n} {}", o.value)).collect()
}

/// A `tune.cfg` key naming a shader constant: (file name, constant).
pub(crate) fn parse_key(key: &str) -> Option<(&str, &str)> {
    let (f, n) = key.split_once("::")?;
    (f.ends_with(".wgsl") && !n.is_empty()).then_some((f, n))
}

/// `src` (the shader `file`) with the overridden constants' values put in: the expression between the `=` and the
/// `;` of the first `const NAME` (or `override NAME`) declaration that starts a line. Borrowed when nothing of this
/// file is overridden, so the default shaders are the source byte for byte.
pub fn patch<'a>(file: &str, src: &'a str) -> Cow<'a, str> {
    let key = file_key(file);
    let overs: Vec<(String, String)> = store().iter().filter(|((f, _), _)| f == key).map(|((_, n), o)| (n.clone(), o.value.clone())).collect();
    if overs.is_empty() {
        return Cow::Borrowed(src);
    }
    let mut out = src.to_string();
    for (name, value) in overs {
        if let Some((a, b)) = value_span(&out, &name) {
            out.replace_range(a..b, &format!(" {value}"));
        }
    }
    Cow::Owned(out)
}

/// The byte range of the value of `name`'s declaration in `src`: from after its `=` to its `;`.
fn value_span(src: &str, name: &str) -> Option<(usize, usize)> {
    let mut at = 0;
    for line in src.split_inclusive('\n') {
        let t = line.trim_start();
        let rest = t.strip_prefix("const ").or_else(|| t.strip_prefix("override ")).map(str::trim_start);
        if let Some(r) = rest.and_then(|r| r.strip_prefix(name)) {
            if r.starts_with([':', '=', ' ', '\t']) {
                let start = at + (line.len() - t.len()) + (t.len() - r.len());
                let eq = start + src[start..].find('=')?;
                let semi = eq + src[eq..].find(';')?;
                return Some((eq + 1, semi));
            }
        }
        at += line.len();
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    const SRC: &str = "// const SHADOW_WORLD: f32 = 9.0;\nconst SHADOW_WORLDX: f32 = 2.0;\nconst SHADOW_WORLD: f32 = 0.6;\n  const N = vec3<f32>(1.0,\n    2.0, 3.0);\nfn f() {}\n";

    #[test]
    fn patches_only_the_named_const() {
        clear();
        assert!(matches!(patch("crates/motorview/src/t1.wgsl", SRC), Cow::Borrowed(_)));
        set("crates/motorview/src/t1.wgsl", "SHADOW_WORLD", Some("0.25"));
        set("t1.wgsl", "N", Some("vec3<f32>(0.0)"));
        let p = patch("t1.wgsl", SRC);
        assert!(p.contains("const SHADOW_WORLD: f32 = 0.25;\n"), "{p}");
        assert!(p.contains("const SHADOW_WORLDX: f32 = 2.0;") && p.contains("// const SHADOW_WORLD: f32 = 9.0;"));
        assert!(p.contains("  const N = vec3<f32>(0.0);\nfn f()"), "{p}");
        // another file's overrides do not apply
        assert!(matches!(patch("t2.wgsl", SRC), Cow::Borrowed(_)));
        assert_eq!(get("x/t1.wgsl", "N").as_deref(), Some("vec3<f32>(0.0)"));
        mark_written("t1.wgsl", "N");
        assert!(!saved("t1.wgsl", "N") && saved("t1.wgsl", "SHADOW_WORLD"));
        assert_eq!(save_lines(), vec!["t1.wgsl::SHADOW_WORLD 0.25".to_string()]);
        set("t1.wgsl", "SHADOW_WORLD", None);
        set("t1.wgsl", "N", None);
        assert!(matches!(patch("t1.wgsl", SRC), Cow::Borrowed(_)));
        assert_eq!(parse_key("shader.wgsl::A"), Some(("shader.wgsl", "A")));
        assert_eq!(parse_key("motor_sim::ai::A"), None);
        // tune.cfg: the rest of the line is the expression
        let l = crate::load_str("t1.wgsl::N vec3<f32>(1.0, 0.5, 0.0)\n");
        assert_eq!((l.applied, get("t1.wgsl", "N").as_deref()), (1, Some("vec3<f32>(1.0, 0.5, 0.0)")));
        assert!(crate::save_string().contains("t1.wgsl::N vec3<f32>(1.0, 0.5, 0.0)\n"));
        set("t1.wgsl", "N", None);
    }
}
