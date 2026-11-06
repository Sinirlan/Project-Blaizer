//! The constants catalog (#398): every constant of the game's source, as motorview's build.rs found it (`scan`),
//! with its area for the Constants page, its value as a number when it has one, and the write-back into the source.

use std::path::Path;

/// How a constant can be changed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// `tunable!`: live, through `crate::find`
    Live,
    /// a WGSL `const`: live when the shaders are rebuilt (`crate::wgsl`), and written back to the shader source
    Wgsl,
    /// a Rust number or bool `const`: compiled in, written back to the source
    Build,
    /// any other Rust `const` (strings, arrays, tuples): its expression is written back as text
    Fixed,
}

/// One constant of the source.
#[derive(Debug)]
pub struct Entry {
    pub name: &'static str,
    pub ty: &'static str,
    /// the value expression as written
    pub value: &'static str,
    pub doc: &'static str,
    /// the doc is shared with the constant above
    pub group: bool,
    /// workspace relative
    pub file: &'static str,
    pub line: u32,
    pub kind: Kind,
    /// "CLASSIC", "REMASTER" or ""
    pub tag: &'static str,
}

impl Entry {
    /// The page's area and sub-area: the doc's `area: A/B` tag, else from the file.
    pub fn area(&self) -> (String, String) {
        if let Some(a) = crate::doc_tag(self.doc, "area") {
            let (a, b) = a.split_once('/').unwrap_or((a, self.stem()));
            return (a.trim().to_string(), b.trim().to_string());
        }
        (area_of(self.file).to_string(), self.stem().to_string())
    }

    /// The file name (`scene.rs`), with its folder for the game's own files (`motorhead/hud.rs`).
    pub fn stem(&self) -> &'static str {
        let f = self.file;
        let cut = f.rfind('/').map_or(0, |i| i + 1);
        match f.find("/bin/") {
            Some(b) if b + 5 < cut => &f[b + 5..],
            _ => &f[cut..],
        }
    }

    /// The type a slider edits: `Some(int)` for numbers (true = whole numbers), None for text.
    pub fn slider(&self) -> Option<bool> {
        match self.ty {
            "f32" | "f64" | "AbstractFloat" => Some(false),
            "bool" => None,
            t if crate::scan::numeric(t) => Some(true),
            // a WGSL const without a type: a literal with a point is a float
            "" if self.kind == Kind::Wgsl => {
                let v = self.value.trim();
                parse_num(v).map(|_| !(v.contains('.') || v.ends_with('f') || v.contains('e')))
            }
            _ => None,
        }
    }

    pub fn is_bool(&self) -> bool {
        self.ty == "bool"
    }

    /// The source's value as a number (simple arithmetic over literals and the catalog's other constants).
    pub fn number(&self, all: &[Entry]) -> Option<f64> {
        if self.is_bool() {
            return match self.value.trim() {
                "true" => Some(1.0),
                "false" => Some(0.0),
                _ => None,
            };
        }
        self.slider()?;
        eval(self.value, &|name| all.iter().filter(|e| e.name == name && e.file == self.file).chain(all.iter().filter(|e| e.name == name)).find(|e| e.slider().is_some()).and_then(|e| if e.name == self.name { None } else { eval(e.value, &|_| None) }))
    }

    /// The doc without its tag lines (`range:` ...).
    pub fn text(&self) -> String {
        crate::doc_text(self.doc)
    }
}

/// The page's area of a source file.
pub fn area_of(file: &str) -> &'static str {
    let stem = file.rsplit('/').next().unwrap_or(file);
    let stem = stem.split('.').next().unwrap_or(stem);
    if file.contains("motor-audio/") {
        return "Audio";
    }
    if file.contains("motor-sim/") {
        return match stem {
            "ai" | "autopilot" => "AI",
            "car" | "body" | "world" | "props" => "Physics",
            "startcam" | "tvcam" => "Camera",
            "cheats" => "Gameplay",
            _ => "Race",
        };
    }
    if file.contains("/bin/motorhead/") {
        return match stem {
            s if s.starts_with("hud") => "HUD",
            "audio" => "Audio",
            "controls" | "rumble" => "Input",
            "front" | "league" | "hiscore" | "stats" | "movie" | "demo" | "playback" => "Menus",
            s if s.starts_with("menu") => "Menus",
            _ => "Game",
        };
    }
    if file.ends_with(".wgsl") {
        return "Graphics";
    }
    if file.contains("/procgen") {
        return "Procedural cars";
    }
    match stem {
        "render" | "scene" | "fog" | "sky" | "probes" | "volfog" | "fx" | "texpack" | "previews" | "loading" | "sky_image" => "Graphics",
        "drive" | "ghost" => "Camera & drive",
        "cine" | "cine_render" => "Cinematics",
        "race_audio" | "engine_sim" => "Audio",
        "ui" | "ui_sdf" | "ui_vector" | "ui_font" | "minimap" | "revmeter" => "HUD",
        _ => "Tools UI",
    }
}

/// A number literal as Rust or WGSL writes it (`1_000.0`, `2.5f32`, `16u`, `0x10`).
pub fn parse_num(s: &str) -> Option<f64> {
    let s = s.trim().replace('_', "");
    if let Some(h) = s.strip_prefix("0x") {
        let h = h.trim_end_matches(|c: char| c.is_ascii_alphabetic() && !c.is_ascii_hexdigit());
        return u64::from_str_radix(h, 16).ok().map(|v| v as f64);
    }
    let mut t = s.as_str();
    for suf in ["f32", "f64", "i32", "u32", "i64", "u64", "usize", "isize", "u8", "u16", "i8", "i16", "f", "u", "i", "h"] {
        if let Some(r) = t.strip_suffix(suf) {
            if r.ends_with(|c: char| c.is_ascii_digit() || c == '.') {
                t = r;
                break;
            }
        }
    }
    if t.is_empty() || !t.starts_with(|c: char| c.is_ascii_digit() || c == '.') {
        return None;
    }
    t.parse().ok()
}

/// Evaluate `+ - * /`, brackets, `as T` casts, number literals, `std::f32::consts::PI` / `TAU` and names `look` knows.
pub fn eval(expr: &str, look: &dyn Fn(&str) -> Option<f64>) -> Option<f64> {
    let toks = tokens(expr)?;
    let mut p = Parser { t: &toks, i: 0, look };
    let v = p.sum()?;
    (p.i == toks.len() && v.is_finite()).then_some(v)
}

#[derive(Debug, Clone, PartialEq)]
enum Tok {
    Num(f64),
    Name(String),
    Op(char),
}

fn tokens(s: &str) -> Option<Vec<Tok>> {
    let b: Vec<char> = s.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        let c = b[i];
        if c.is_whitespace() {
            i += 1;
        } else if "+-*/()".contains(c) {
            out.push(Tok::Op(c));
            i += 1;
        } else if c.is_ascii_digit() || c == '.' {
            let st = i;
            while i < b.len() && (b[i].is_ascii_alphanumeric() || b[i] == '.' || b[i] == '_' || ((b[i] == '-' || b[i] == '+') && matches!(b[i - 1], 'e' | 'E') && !b[st..i].contains(&'x'))) {
                i += 1;
            }
            out.push(Tok::Num(parse_num(&b[st..i].iter().collect::<String>())?));
        } else if c.is_ascii_alphabetic() || c == '_' {
            let st = i;
            while i < b.len() && (b[i].is_ascii_alphanumeric() || b[i] == '_' || (b[i] == ':' && b.get(i + 1) == Some(&':'))) {
                i += if b[i] == ':' { 2 } else { 1 };
            }
            out.push(Tok::Name(b[st..i].iter().collect()));
        } else {
            return None;
        }
    }
    Some(out)
}

struct Parser<'a> {
    t: &'a [Tok],
    i: usize,
    look: &'a dyn Fn(&str) -> Option<f64>,
}

impl Parser<'_> {
    fn sum(&mut self) -> Option<f64> {
        let mut v = self.product()?;
        while let Some(Tok::Op(c @ ('+' | '-'))) = self.t.get(self.i) {
            self.i += 1;
            let r = self.product()?;
            v = if *c == '+' { v + r } else { v - r };
        }
        Some(v)
    }
    fn product(&mut self) -> Option<f64> {
        let mut v = self.unary()?;
        while let Some(Tok::Op(c @ ('*' | '/'))) = self.t.get(self.i) {
            self.i += 1;
            let r = self.unary()?;
            v = if *c == '*' { v * r } else { v / r };
        }
        Some(v)
    }
    fn unary(&mut self) -> Option<f64> {
        if self.t.get(self.i) == Some(&Tok::Op('-')) {
            self.i += 1;
            return Some(-self.unary()?);
        }
        let mut v = self.atom()?;
        // `x as f32`
        while let Some(Tok::Name(n)) = self.t.get(self.i) {
            if n != "as" {
                break;
            }
            self.i += 2;
            v = match self.t.get(self.i - 1) {
                Some(Tok::Name(t)) if t.starts_with('u') || t.starts_with('i') => v.trunc(),
                Some(Tok::Name(_)) => v,
                _ => return None,
            };
        }
        Some(v)
    }
    fn atom(&mut self) -> Option<f64> {
        let t = self.t.get(self.i)?.clone();
        self.i += 1;
        match t {
            Tok::Num(v) => Some(v),
            Tok::Op('(') => {
                let v = self.sum()?;
                (self.t.get(self.i) == Some(&Tok::Op(')'))).then(|| self.i += 1)?;
                Some(v)
            }
            Tok::Name(n) => {
                let last = n.rsplit("::").next().unwrap_or(&n);
                match last {
                    "PI" => Some(std::f64::consts::PI),
                    "TAU" => Some(std::f64::consts::TAU),
                    "FRAC_PI_2" => Some(std::f64::consts::FRAC_PI_2),
                    "FRAC_PI_4" => Some(std::f64::consts::FRAC_PI_4),
                    _ if n.contains("::") => None,
                    _ => (self.look)(&n),
                }
            }
            Tok::Op(_) => None,
        }
    }
}

/// A number as the source writes it for `e` (floats keep a point: `3.0`).
pub fn literal(e: &Entry, v: f64) -> String {
    if e.is_bool() {
        return (v != 0.0).to_string();
    }
    match e.slider() {
        Some(true) => format!("{}", v.round() as i64),
        _ => {
            let s = if e.ty == "f32" { format!("{:?}", v as f32) } else { format!("{v:?}") };
            if s.contains('.') || s.contains('e') || s.contains("inf") || s.contains("NaN") {
                s
            } else {
                format!("{s}.0")
            }
        }
    }
}

/// Replace the value of `e` in its source file (under `root`, the workspace) by `value`. The declaration must still
/// be at its line (else the source changed since the build: an error, nothing is written).
pub fn write_back(root: &Path, e: &Entry, value: &str) -> Result<(), String> {
    let path = root.join(e.file);
    let src = std::fs::read_to_string(&path).map_err(|err| format!("{}: {err}", path.display()))?;
    // byte offset of the declaration's line
    let mut at = 0;
    for (n, l) in src.split_inclusive('\n').enumerate() {
        if n + 1 == e.line as usize {
            break;
        }
        at += l.len();
    }
    let rest = &src[at..];
    let line_end = rest.find('\n').unwrap_or(rest.len());
    let word = |c: Option<char>| c.is_some_and(|c| c.is_ascii_alphanumeric() || c == '_');
    let line = &rest[..line_end];
    let Some(name_at) = line.match_indices(e.name).map(|(i, _)| i).find(|&i| !word(line[..i].chars().next_back()) && !word(line[i + e.name.len()..].chars().next())) else {
        return Err(format!("{}:{} no longer declares {} (rebuild to refresh the catalog)", e.file, e.line, e.name));
    };
    // the `=` after the name and its type, then the `;` that ends a line
    let after = name_at + e.name.len();
    let mut depth = 0i32;
    let mut eq = None;
    for (i, c) in rest[after..].char_indices() {
        match c {
            '<' | '(' | '[' => depth += 1,
            '>' | ')' | ']' => depth -= 1,
            '=' if depth == 0 => {
                eq = Some(after + i);
                break;
            }
            _ => {}
        }
    }
    let eq = eq.ok_or_else(|| format!("{}:{} has no `=` after {}", e.file, e.line, e.name))?;
    let mut end = None;
    let mut off = eq + 1;
    for l in rest[eq + 1..].split_inclusive('\n') {
        let code = l.split("//").next().unwrap_or(l);
        if let Some(p) = code.trim_end().strip_suffix(';').map(|c| c.len()) {
            end = Some(off + p);
            break;
        }
        off += l.len();
    }
    let end = end.ok_or_else(|| format!("{}:{}: no `;` after {}", e.file, e.line, e.name))?;
    let new = format!("{}{} {}{}", &src[..at + eq], "=", value.trim(), &src[at + end..]);
    std::fs::write(&path, new).map_err(|err| format!("{}: {err}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(file: &'static str, name: &'static str, ty: &'static str, value: &'static str, line: u32) -> Entry {
        Entry { name, ty, value, doc: "", group: false, file, line, kind: Kind::Build, tag: "" }
    }

    #[test]
    fn numbers() {
        let none = |_: &str| None;
        assert_eq!(eval("2.5", &none), Some(2.5));
        assert_eq!(eval("1.0 / 30.0", &none), Some(1.0 / 30.0));
        assert_eq!(eval("-(2.0 + 1_000.0) * 2", &none), Some(-2004.0));
        assert_eq!(eval("std::f32::consts::PI / 180.0", &none), Some(std::f64::consts::PI / 180.0));
        assert_eq!(eval("12 as f32 * 0.5", &none), Some(6.0));
        assert_eq!(eval("1.5e-3", &none), Some(0.0015));
        assert_eq!(eval("0x4cf0", &none), Some(19696.0));
        assert_eq!(eval("OTHER * 2.0", &|n| (n == "OTHER").then_some(3.0)), Some(6.0));
        assert_eq!(eval("[1.0, 2.0]", &none), None);
        assert_eq!(eval("Vec3::new(1.0, 0.0, 0.0)", &none), None);
        let all = [entry("a.rs", "A", "f32", "2.0", 1), entry("a.rs", "B", "f32", "A / 4.0", 2)];
        assert_eq!(all[1].number(&all), Some(0.5));
        assert_eq!(literal(&all[0], 3.0), "3.0");
        assert_eq!(literal(&entry("a.rs", "N", "usize", "4", 1), 6.6), "7");
    }

    #[test]
    fn writes_back_in_place() {
        let dir = std::env::temp_dir().join(format!("motor-tune-wb-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("a.rs"), "/// doc\nconst A: f32 = 2.5; // note\nconst B: (f32,\n    f32) = (1.0,\n    2.0);\nconst C: u8 = 1;\n").unwrap();
        write_back(&dir, &entry("a.rs", "A", "f32", "2.5", 2), "3.25").unwrap();
        write_back(&dir, &entry("a.rs", "B", "(f32, f32)", "(1.0, 2.0)", 3), "(4.0, 5.0)").unwrap();
        assert!(write_back(&dir, &entry("a.rs", "C", "u8", "1", 2), "2").is_err());
        let got = std::fs::read_to_string(dir.join("a.rs")).unwrap();
        assert_eq!(got, "/// doc\nconst A: f32 = 3.25; // note\nconst B: (f32,\n    f32) = (4.0, 5.0);\nconst C: u8 = 1;\n");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn areas() {
        assert_eq!(area_of("crates/motor-sim/src/ai.rs"), "AI");
        assert_eq!(area_of("crates/motorview/src/bin/motorhead/hud_glass.rs"), "HUD");
        assert_eq!(area_of("crates/motorview/src/shader.wgsl"), "Graphics");
        assert_eq!(entry("crates/motorview/src/bin/motorhead/front.rs", "X", "f32", "1.0", 1).stem(), "motorhead/front.rs");
        assert_eq!(entry("crates/motorview/src/scene.rs", "X", "f32", "1.0", 1).stem(), "scene.rs");
    }
}
