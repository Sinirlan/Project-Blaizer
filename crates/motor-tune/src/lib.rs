//! DEBUG live tuning values (design choice, #398). A `tunable!` value is a `static` the program reads at run time
//! (`NAME.get()`), so motorview's Constants page (F11 dock) can change it while the game runs. Every `tunable!`
//! registers itself (`TUNABLES`, a link-time list), so a new one shows on the page without more code.
//!
//! The page also lists every plain `const` of the source (the build-time catalog, `catalog` / `scan`): those can
//! only be written back into the source file, since Rust compiles their value in.
//!
//! Doc comments of a tunable may carry tags, each on its own line: `range: 0..10`, `area: Graphics/Lamps`,
//! `applies: frame|reload|restart` (default frame), `unit: m`.

use std::sync::atomic::{AtomicBool, AtomicI32, AtomicU32, AtomicU64, AtomicUsize, Ordering::Relaxed};

pub mod catalog;
pub mod scan;
pub mod wgsl;

#[doc(hidden)]
pub use linkme;

/// Where a tunable was declared and what it says about itself.
#[derive(Debug)]
pub struct Meta {
    pub name: &'static str,
    /// `file!()`: workspace relative, as the catalog has it
    pub file: &'static str,
    pub module: &'static str,
    /// the doc comment, one line per `///` line
    pub doc: &'static str,
}

impl Meta {
    /// The `tune.cfg` key: `crate::module::NAME`.
    pub fn key(&self) -> String {
        format!("{}::{}", self.module, self.name)
    }
}

/// How a value is edited.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Num {
    Float,
    Int,
    Bool,
}

/// A value type a tunable can hold: stored in an atomic, edited as f64.
pub trait Atom: Copy + Send + Sync + 'static {
    type Cell: Send + Sync;
    const NUM: Num;
    fn cell(v: Self) -> Self::Cell;
    fn load(c: &Self::Cell) -> Self;
    fn store(c: &Self::Cell, v: Self);
    fn to_f64(self) -> f64;
    fn from_f64(v: f64) -> Self;
}

macro_rules! atom_int {
    ($t:ty, $cell:ty) => {
        impl Atom for $t {
            type Cell = $cell;
            const NUM: Num = Num::Int;
            fn cell(v: Self) -> $cell {
                <$cell>::new(v)
            }
            fn load(c: &$cell) -> Self {
                c.load(Relaxed)
            }
            fn store(c: &$cell, v: Self) {
                c.store(v, Relaxed)
            }
            fn to_f64(self) -> f64 {
                self as f64
            }
            fn from_f64(v: f64) -> Self {
                v.round() as $t
            }
        }
    };
}
atom_int!(i32, AtomicI32);
atom_int!(u32, AtomicU32);
atom_int!(usize, AtomicUsize);

impl Atom for f32 {
    type Cell = AtomicU32;
    const NUM: Num = Num::Float;
    fn cell(v: Self) -> AtomicU32 {
        AtomicU32::new(v.to_bits())
    }
    fn load(c: &AtomicU32) -> Self {
        f32::from_bits(c.load(Relaxed))
    }
    fn store(c: &AtomicU32, v: Self) {
        c.store(v.to_bits(), Relaxed)
    }
    fn to_f64(self) -> f64 {
        self as f64
    }
    fn from_f64(v: f64) -> Self {
        v as f32
    }
}

impl Atom for f64 {
    type Cell = AtomicU64;
    const NUM: Num = Num::Float;
    fn cell(v: Self) -> AtomicU64 {
        AtomicU64::new(v.to_bits())
    }
    fn load(c: &AtomicU64) -> Self {
        f64::from_bits(c.load(Relaxed))
    }
    fn store(c: &AtomicU64, v: Self) {
        c.store(v.to_bits(), Relaxed)
    }
    fn to_f64(self) -> f64 {
        self
    }
    fn from_f64(v: f64) -> Self {
        v
    }
}

impl Atom for bool {
    type Cell = AtomicBool;
    const NUM: Num = Num::Bool;
    fn cell(v: Self) -> AtomicBool {
        AtomicBool::new(v)
    }
    fn load(c: &AtomicBool) -> Self {
        c.load(Relaxed)
    }
    fn store(c: &AtomicBool, v: Self) {
        c.store(v, Relaxed)
    }
    fn to_f64(self) -> f64 {
        if self {
            1.0
        } else {
            0.0
        }
    }
    fn from_f64(v: f64) -> Self {
        v != 0.0
    }
}

/// A live value: the current one and the source's one (`def`; moves when the page writes the value back).
pub struct Tun<T: Atom> {
    cur: T::Cell,
    def: T::Cell,
    pub meta: Meta,
}

impl<T: Atom> Tun<T> {
    /// The value now (one relaxed atomic load).
    pub fn get(&self) -> T {
        T::load(&self.cur)
    }
    pub fn set(&self, v: T) {
        T::store(&self.cur, v)
    }
    pub fn default_value(&self) -> T {
        T::load(&self.def)
    }
}

// `Tun::new` must be const for a `static`, and trait methods cannot be: one constructor per type.
macro_rules! tun_new {
    ($t:ty, $cell:ty, $conv:expr) => {
        impl Tun<$t> {
            #[doc(hidden)]
            pub const fn new(v: $t, meta: Meta) -> Self {
                Tun { cur: <$cell>::new($conv(v)), def: <$cell>::new($conv(v)), meta }
            }
        }
    };
}
const fn id<T>(v: T) -> T {
    v
}
tun_new!(f32, AtomicU32, f32::to_bits);
tun_new!(f64, AtomicU64, f64::to_bits);
tun_new!(i32, AtomicI32, id);
tun_new!(u32, AtomicU32, id);
tun_new!(usize, AtomicUsize, id);
tun_new!(bool, AtomicBool, id);

/// A tunable without its type, as the page and `tune.cfg` see it.
pub trait Erased: Sync {
    fn meta(&self) -> &Meta;
    fn num(&self) -> Num;
    fn value(&self) -> f64;
    fn set_value(&self, v: f64);
    fn default_f64(&self) -> f64;
    /// The source now holds the current value (written back).
    fn rebase(&self);
    fn modified(&self) -> bool {
        self.value() != self.default_f64()
    }
    fn reset(&self) {
        self.set_value(self.default_f64())
    }
}

impl<T: Atom> Erased for Tun<T>
where
    Tun<T>: Sync,
{
    fn meta(&self) -> &Meta {
        &self.meta
    }
    fn num(&self) -> Num {
        T::NUM
    }
    fn value(&self) -> f64 {
        self.get().to_f64()
    }
    fn set_value(&self, v: f64) {
        self.set(T::from_f64(v))
    }
    fn default_f64(&self) -> f64 {
        self.default_value().to_f64()
    }
    fn rebase(&self) {
        T::store(&self.def, self.get())
    }
}

/// Every `tunable!` of the program (all linked crates).
#[linkme::distributed_slice]
pub static TUNABLES: [&'static dyn Erased];

/// Declare live values. Each entry reads like a `const` without the keyword; uses read it with `NAME.get()`:
/// ```ignore
/// motor_tune::tunable! {
///     /// Range of a lamp per metre of height above the road.
///     /// range: 1..5
///     /// applies: reload
///     pub(crate) LAMP_RANGE_PER_HEIGHT: f32 = 2.5;
/// }
/// ```
/// Types: f32, f64, i32, u32, usize, bool. The value must be a constant expression.
#[macro_export]
macro_rules! tunable {
    ($( $(#[doc = $doc:literal])* $vis:vis $name:ident : $ty:ty = $val:expr ; )*) => {$(
        $(#[doc = $doc])*
        $vis static $name: $crate::Tun<$ty> = $crate::Tun::<$ty>::new($val, $crate::Meta {
            name: stringify!($name),
            file: file!(),
            module: module_path!(),
            doc: concat!($($doc, "\n",)*),
        });
        const _: () = {
            #[$crate::linkme::distributed_slice($crate::TUNABLES)]
            #[linkme(crate = $crate::linkme)]
            static ENTRY: &'static dyn $crate::Erased = &$name;
        };
    )*};
}

/// The tunable declared as `name` in `file` (workspace relative), else the only one named `name`.
pub fn find(file: &str, name: &str) -> Option<&'static dyn Erased> {
    let all: Vec<&'static dyn Erased> = TUNABLES.iter().copied().filter(|t| t.meta().name == name).collect();
    all.iter().copied().find(|t| same_file(t.meta().file, file)).or(if all.len() == 1 { Some(all[0]) } else { None })
}

/// `file!()` uses `\` on Windows.
fn same_file(a: &str, b: &str) -> bool {
    a.replace('\\', "/") == b.replace('\\', "/")
}

/// The tunables that differ from the source, as `tune.cfg` text (`crate::module::NAME value` lines), then the
/// shader values (`file.wgsl::NAME value`, `wgsl`).
pub fn save_string() -> String {
    let mut lines: Vec<String> = TUNABLES.iter().filter(|t| t.modified()).map(|t| format!("{} {}", t.meta().key(), fmt(t.value(), t.num()))).collect();
    lines.sort();
    lines.extend(wgsl::save_lines());
    let mut s = String::from("# motorview Constants page (F11): live values that differ from the source\n");
    for l in lines {
        s += &l;
        s.push('\n');
    }
    s
}

/// A value as `tune.cfg` and the source write it.
pub fn fmt(v: f64, num: Num) -> String {
    match num {
        Num::Bool => (v != 0.0).to_string(),
        Num::Int => format!("{}", v.round() as i64),
        // an f32 tunable holds its value widened to f64: print the f32's own shortest form (0.7, not 0.699999988079071);
        // reading it back gives the same f32
        Num::Float if (v as f32) as f64 == v => format!("{:?}", v as f32),
        Num::Float => format!("{v:?}"),
    }
}

/// What loading `tune.cfg` did.
#[derive(Debug, Default)]
pub struct Loaded {
    pub applied: usize,
    /// lines naming no tunable of this program, or with a value that does not parse
    pub unknown: Vec<String>,
}

/// Apply `tune.cfg` text: `key value` lines, `#` comments. A shader value (`file.wgsl::NAME`) takes the rest of the
/// line as its expression; whether the shader has that constant is for the caller to check (`wgsl::all`).
pub fn load_str(text: &str) -> Loaded {
    let mut out = Loaded::default();
    for line in text.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with('#')) {
        let Some((key, v)) = line.split_once(char::is_whitespace).map(|(k, v)| (k, v.trim())).filter(|(_, v)| !v.is_empty()) else {
            out.unknown.push(line.to_string());
            continue;
        };
        if let Some((f, n)) = wgsl::parse_key(key) {
            wgsl::set(f, n, Some(v));
            out.applied += 1;
            continue;
        }
        let v = match v.split_whitespace().next().unwrap_or(v) {
            "true" => Some(1.0),
            "false" => Some(0.0),
            v => v.parse::<f64>().ok(),
        };
        match (TUNABLES.iter().find(|t| t.meta().key() == key), v) {
            (Some(t), Some(v)) => {
                t.set_value(v);
                out.applied += 1;
            }
            _ => out.unknown.push(line.to_string()),
        }
    }
    out
}

/// The tags of a doc comment (`range:`, `area:`, `applies:`, `unit:`) and the text without them.
pub fn doc_tag<'a>(doc: &'a str, tag: &str) -> Option<&'a str> {
    doc.lines().map(str::trim).find_map(|l| l.strip_prefix(tag).and_then(|r| r.strip_prefix(':')).map(str::trim))
}

/// The doc comment without its tag lines.
pub fn doc_text(doc: &str) -> String {
    doc.lines().map(str::trim).filter(|l| !["range:", "area:", "applies:", "unit:"].iter().any(|t| l.starts_with(t))).collect::<Vec<_>>().join(" ").trim().to_string()
}

/// `range: a..b` (also `a..=b`).
pub fn doc_range(doc: &str) -> Option<(f64, f64)> {
    let r = doc_tag(doc, "range")?;
    let (a, b) = r.split_once("..")?;
    let b = b.trim_start_matches('=');
    Some((a.trim().parse().ok()?, b.trim().parse().ok()?))
}

#[cfg(test)]
mod tests {
    use super::*;

    tunable! {
        /// a test value
        /// range: 0..4
        pub(crate) TEST_F: f32 = 1.5;
        /// a count
        TEST_N: u32 = 3;
        TEST_B: bool = true;
    }

    #[test]
    fn registry_set_get_and_cfg() {
        let t = find(file!(), "TEST_F").expect("registered");
        assert_eq!(TEST_F.get(), 1.5);
        t.set_value(2.25);
        assert_eq!(TEST_F.get(), 2.25);
        assert!(save_string().contains("motor_tune::tests::TEST_F 2.25"));
        t.reset();
        assert_eq!(TEST_F.get(), 1.5);
        let l = load_str("motor_tune::tests::TEST_N 7\nmotor_tune::tests::TEST_B false\nnope::X 1\n");
        assert_eq!((l.applied, l.unknown.len()), (2, 1));
        assert_eq!((TEST_N.get(), TEST_B.get()), (7, false));
        assert_eq!(doc_range(TEST_F.meta.doc), Some((0.0, 4.0)));
        assert_eq!(doc_text(TEST_F.meta.doc), "a test value");
    }
}
