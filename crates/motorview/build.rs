// Windows: each .exe carries its own icon (the game's assets/icon/motorhead.ico; the tools' assets/icon/tools/<bin>/, #407) and its
// name in the file properties.
// Other targets: nothing to do (the window icon is set at run time, see motorview::app_icon).
// Every target: the constants catalog of the Constants page (#398, `motor_tune::scan`): every `const` and `tunable!`
// of the game's source, rescanned whenever a source file changes.

#[allow(dead_code)]
#[path = "../motor-tune/src/scan.rs"]
mod scan;

use std::path::Path;

/// The crates whose constants the page lists (workspace relative).
const SCANNED: [&str; 3] = ["crates/motorview/src", "crates/motor-sim/src", "crates/motor-audio/src"];

fn walk(root: &Path, dir: &Path, out: &mut Vec<scan::Found>) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    let mut items: Vec<_> = rd.flatten().map(|e| e.path()).collect();
    items.sort();
    for p in items {
        if p.is_dir() {
            walk(root, &p, out);
        } else if p.extension().is_some_and(|x| x == "rs" || x == "wgsl") {
            let Ok(text) = std::fs::read_to_string(&p) else { continue };
            let rel = p.strip_prefix(root).unwrap_or(&p).to_string_lossy().replace('\\', "/");
            out.extend(scan::scan_file(&rel, &text));
        }
    }
}

fn main() {
    println!("cargo:rerun-if-changed=../../assets/icon");
    let root = Path::new("../..");
    let mut found = Vec::new();
    for d in SCANNED {
        println!("cargo:rerun-if-changed=../../{d}");
        walk(root, &root.join(d), &mut found);
    }
    let out = Path::new(&std::env::var("OUT_DIR").expect("OUT_DIR")).join("consts_catalog.rs");
    let text = scan::emit(&found);
    if std::fs::read_to_string(&out).ok().as_deref() != Some(text.as_str()) {
        std::fs::write(&out, text).expect("write the constants catalog");
    }
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        // (binary, .ico, name): compile_for gives each binary its own resource object
        for (bin, ico, name) in [
            ("motorhead", "assets/icon/motorhead.ico", "Motorhead"),
            ("motorview", "assets/icon/tools/motorview/motorview.ico", "Motorhead motorview"),
            ("revshop", "assets/icon/tools/revshop/revshop.ico", "Motorhead Rev Workshop"),
        ] {
            let mut res = winresource::WindowsResource::new();
            res.set_icon(&format!("../../{ico}"));
            res.set("ProductName", name);
            res.set("FileDescription", name);
            if let Err(e) = res.compile_for(bin) {
                println!("cargo:warning=app icon of {bin} not embedded: {e}");
            }
        }
    }
}
