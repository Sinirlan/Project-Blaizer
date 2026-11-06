//! The source scanner behind the constants catalog (#398): finds every `const` (Rust and WGSL) and every
//! `tunable!` entry of a source file with its doc comment. motorview's build.rs includes this file (`#[path]`) and
//! writes the catalog (`emit`) to OUT_DIR, so a new constant is listed after the next build. Std only.
//!
//! Line based, which fits this code base: one declaration starts on its own line, the value ends at a `;` that
//! closes a line (multi-line values are joined). The doc is the `///` block above (WGSL: the `//` block), else the
//! comment after the `;`; a constant without either directly under one with a doc shares that doc ("group").

/// One constant found in the source.
#[derive(Debug, Clone, PartialEq)]
pub struct Found {
    pub name: String,
    /// as written; empty for a WGSL `const` without a type
    pub ty: String,
    /// the value expression as written (lines joined)
    pub value: String,
    pub doc: String,
    /// the doc is the one of the constant above
    pub group: bool,
    /// workspace relative, `/` separated
    pub file: String,
    /// 1-based line of the declaration
    pub line: u32,
    /// "Live" (`tunable!`), "Wgsl", "Build" (a number or bool), "Fixed" (anything else)
    pub kind: &'static str,
    /// "CLASSIC", "REMASTER" or "" (from the doc or the line's own comment)
    pub tag: &'static str,
}

const NUMERIC: [&str; 14] = ["f32", "f64", "i8", "i16", "i32", "i64", "isize", "u8", "u16", "u32", "u64", "usize", "bool", "AbstractFloat"];

/// Is `ty` edited with a slider (numbers and bools)?
pub fn numeric(ty: &str) -> bool {
    NUMERIC.contains(&ty)
}

/// Text before a `//` comment that is not inside a string or char literal, and the comment.
fn split_comment(line: &str) -> (&str, Option<&str>) {
    let b = line.as_bytes();
    let (mut i, mut in_str) = (0, false);
    while i < b.len() {
        match b[i] {
            b'\\' if in_str => i += 1,
            b'"' => in_str = !in_str,
            // a char literal: '"' or '/' must not count
            b'\'' if !in_str && i + 2 < b.len() && b[i + 2] == b'\'' => i += 2,
            b'/' if !in_str && b.get(i + 1) == Some(&b'/') => return (&line[..i], Some(line[i + 2..].trim_start_matches('/').trim())),
            _ => {}
        }
        i += 1;
    }
    (line, None)
}

fn ident(s: &str) -> Option<(&str, &str)> {
    let end = s.find(|c: char| !(c.is_ascii_alphanumeric() || c == '_')).unwrap_or(s.len());
    let id = &s[..end];
    (!id.is_empty() && id.starts_with(|c: char| c.is_ascii_uppercase()) && id.chars().all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_')).then_some((id, &s[end..]))
}

/// `pub `, `pub(crate) `, `pub(super) ` and the like.
fn strip_vis(s: &str) -> &str {
    let Some(r) = s.strip_prefix("pub") else { return s };
    let r = if r.starts_with('(') { r.find(')').map_or(r, |i| &r[i + 1..]) } else { r };
    if r.starts_with(char::is_whitespace) {
        r.trim_start()
    } else {
        s
    }
}

/// `NAME: TYPE = rest` (rest = the value's start) or for WGSL also `NAME = rest`.
fn decl(s: &str, wgsl: bool) -> Option<(String, String, String)> {
    let (name, r) = ident(s)?;
    let r = r.trim_start();
    if let Some(r) = r.strip_prefix(':') {
        // the type ends at the first `=` outside brackets
        let (mut depth, mut eq) = (0i32, None);
        for (i, c) in r.char_indices() {
            match c {
                '<' | '(' | '[' => depth += 1,
                '>' | ')' | ']' => depth -= 1,
                '=' if depth == 0 => {
                    eq = Some(i);
                    break;
                }
                _ => {}
            }
        }
        let eq = eq?;
        Some((name.to_string(), r[..eq].trim().to_string(), r[eq + 1..].trim().to_string()))
    } else if wgsl {
        r.strip_prefix('=').map(|v| (name.to_string(), String::new(), v.trim().to_string()))
    } else {
        None
    }
}

fn tag_of(texts: &[&str]) -> &'static str {
    if texts.iter().any(|t| t.contains("REMASTER")) {
        "REMASTER"
    } else if texts.iter().any(|t| t.contains("CLASSIC")) {
        "CLASSIC"
    } else {
        ""
    }
}

/// Every constant of one source file. `rel` is the workspace relative path (`.wgsl` selects WGSL rules).
pub fn scan_file(rel: &str, text: &str) -> Vec<Found> {
    let wgsl = rel.ends_with(".wgsl");
    let lines: Vec<&str> = text.lines().collect();
    let mut out: Vec<Found> = Vec::new();
    // the brace depth inside a `tunable! { ... }` block (None: outside)
    let mut tun_depth: Option<i32> = None;
    // the line after the previous declaration's end (to tell a group: no gap between them)
    let mut prev_end: Option<usize> = None;
    let mut i = 0;
    while i < lines.len() {
        let raw = lines[i];
        let (code, _) = split_comment(raw);
        // a declaration whose type runs over lines (`(f32,\n f32) = ...`): read on to its `=`
        let mut joined = code.trim().to_string();
        let mut extra = 0;
        if (joined.contains("const ") || tun_depth.is_some()) && joined.contains(':') && !joined.contains('=') && !joined.ends_with('{') {
            while extra < 4 && i + extra + 1 < lines.len() && !joined.contains('=') {
                extra += 1;
                joined.push(' ');
                joined += split_comment(lines[i + extra]).0.trim();
            }
        }
        let t = joined.as_str();
        let mut live = false;
        let found = if let Some(d) = tun_depth.as_mut() {
            *d += code.matches('{').count() as i32 - code.matches('}').count() as i32;
            if *d <= 0 {
                tun_depth = None;
                None
            } else {
                live = true;
                decl(strip_vis(t), false)
            }
        } else if t.contains("tunable!") && !t.starts_with("macro_rules") {
            let d = code.matches('{').count() as i32 - code.matches('}').count() as i32;
            tun_depth = (d > 0).then_some(d);
            None
        } else {
            let r = strip_vis(t);
            let r = if wgsl { r.strip_prefix("const ").or_else(|| r.strip_prefix("override ")) } else { r.strip_prefix("const ") };
            r.filter(|r| !r.starts_with("fn ") && !r.starts_with('_')).and_then(|r| decl(r.trim_start(), wgsl))
        };
        let Some((name, ty, first)) = found else {
            i += 1;
            continue;
        };
        let start = i;
        i += extra;
        // the value up to the `;` that ends a line
        let mut value = first;
        let mut trailing = None;
        loop {
            let (c, com) = split_comment(lines[i]);
            if c.trim_end().ends_with(';') {
                trailing = com;
                break;
            }
            if i + 1 >= lines.len() {
                break;
            }
            i += 1;
            let (c, _) = split_comment(lines[i]);
            value.push(' ');
            value += c.trim();
        }
        let value = value.trim().trim_end_matches(';').trim().to_string();
        // the doc block above (attributes skipped)
        let mut doc_lines = Vec::new();
        let mut j = start;
        while j > 0 {
            let l = lines[j - 1].trim();
            if l.starts_with("#[") {
                j -= 1;
                continue;
            }
            let d = if wgsl { l.strip_prefix("//") } else { l.strip_prefix("///") };
            match d {
                Some(d) if !l.starts_with("////") => {
                    doc_lines.push(d.trim_start_matches('/').trim().to_string());
                    j -= 1;
                }
                _ => break,
            }
        }
        doc_lines.reverse();
        let mut doc = doc_lines.join("\n").trim().to_string();
        if doc.is_empty() {
            doc = trailing.unwrap_or("").to_string();
        }
        let mut group = false;
        if doc.is_empty() && prev_end == Some(start) {
            if let Some(p) = out.last() {
                doc = p.doc.clone();
                group = !doc.is_empty();
            }
        }
        let kind = if live {
            "Live"
        } else if wgsl {
            "Wgsl"
        } else if numeric(&ty) {
            "Build"
        } else {
            "Fixed"
        };
        let tag = tag_of(&[&doc, raw]);
        out.push(Found { name, ty, value, doc, group, file: rel.to_string(), line: start as u32 + 1, kind, tag });
        i += 1;
        prev_end = Some(i);
    }
    out
}

/// The catalog as Rust source: `pub static CATALOG: &[motor_tune::catalog::Entry]`.
pub fn emit(found: &[Found]) -> String {
    let mut s = String::from("// generated by build.rs from the source (motor_tune::scan); do not edit\npub static CATALOG: &[motor_tune::catalog::Entry] = &[\n");
    for f in found {
        s += &format!(
            "    motor_tune::catalog::Entry {{ name: {:?}, ty: {:?}, value: {:?}, doc: {:?}, group: {}, file: {:?}, line: {}, kind: motor_tune::catalog::Kind::{}, tag: {:?} }},\n",
            f.name, f.ty, f.value, f.doc, f.group, f.file, f.line, f.kind, f.tag
        );
    }
    s += "];\n";
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    const RS: &str = r#"
/// Range of a lamp per metre of height above the road, and its limits (metres).
const LAMP_RANGE_PER_HEIGHT: f32 = 2.5;
const LAMP_RANGE_MIN: f32 = 6.0;

const G_AI: f32 = 9.82; // 0x4cf020
pub(crate) const NAMES: [&str; 2] = ["a // b", "c"];
/// CLASSIC: wrapped
pub const LONG: (f32,
    f32) = (1.0,
    2.0);
const fn helper() -> u32 { 3 }
motor_tune::tunable! {
    /// live one
    /// range: 0..4
    pub(crate) LIVE_A: f32 = 1.5;
    LIVE_B: u32 = 3;
}
const AFTER: usize = 4;
"#;

    #[test]
    fn rust_consts() {
        let f = scan_file("crates/x/src/a.rs", RS);
        let names: Vec<_> = f.iter().map(|f| f.name.as_str()).collect();
        assert_eq!(names, ["LAMP_RANGE_PER_HEIGHT", "LAMP_RANGE_MIN", "G_AI", "NAMES", "LONG", "LIVE_A", "LIVE_B", "AFTER"]);
        assert_eq!((f[0].line, f[0].kind, f[0].value.as_str()), (3, "Build", "2.5"));
        assert!(f[1].group && f[1].doc.starts_with("Range of a lamp"));
        assert_eq!(f[2].doc, "0x4cf020");
        assert!(!f[2].group);
        assert_eq!((f[3].kind, f[3].value.as_str(), f[3].ty.as_str()), ("Fixed", "[\"a // b\", \"c\"]", "[&str; 2]"));
        assert_eq!((f[4].value.as_str(), f[4].tag), ("(1.0, 2.0)", "CLASSIC"));
        assert_eq!((f[5].kind, f[5].doc.as_str()), ("Live", "live one\nrange: 0..4"));
        assert_eq!((f[6].kind, f[6].ty.as_str()), ("Live", "u32"));
        assert_eq!((f[7].kind, f[7].value.as_str()), ("Build", "4"));
    }

    #[test]
    fn wgsl_consts() {
        let src = "// world light [design choice]\nconst PACK_SPEC: f32 = 0.35;\nconst TAPS = 16;\n\n// REMASTER only\nconst TINT: vec3<f32> = vec3<f32>(1.0, 0.5, 0.0);\n";
        let f = scan_file("crates/x/src/s.wgsl", src);
        assert_eq!(f.len(), 3);
        assert_eq!((f[0].kind, f[0].doc.as_str()), ("Wgsl", "world light [design choice]"));
        assert_eq!((f[1].ty.as_str(), f[1].value.as_str(), f[1].group), ("", "16", true));
        assert_eq!((f[2].ty.as_str(), f[2].tag), ("vec3<f32>", "REMASTER"));
    }
}
