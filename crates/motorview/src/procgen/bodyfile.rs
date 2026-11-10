//! REMASTER body recipe files (`assets/cars/carNN.body`, embedded; an install's
//! `remaster/cars/carNN.body` overrides): a `BodyStyle` as text, one record per line,
//! `Record key=value key=value ...`. `#` starts a comment. Numbers are plain, triples `x,y,z`
//! (colours `r,g,b`), point lists `x,y;x,y` or `x,y,z;x,y,z`. A `Led` line belongs to the
//! `Lamp kind=leds` before it, a `Split` line to the `Thicken` before it. Unknown records or
//! keys are errors (a mistyped box key must not vanish silently); every key is required.
//! The format exists so the per-car tables can be tuned without a rebuild (#183).

use super::aero::{Fins, Foil, Splitter, Wing};
use super::body::{BodyStyle, Glass};
use super::cuts::{Chamfer, MirrorCopy, Paint, Panel, Pick, Push, Slice, Thicken, Warp};
use super::diffuser::Diffuser;
use super::exhaust::Exhaust;
use super::lamps::{Lamp, LampKind, LedPath, LedStyle};
use super::smooth::Smooth;
use super::vents::{Vent, VentKind};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

const HEADER: &str = "# Motorhead body v1";

/// The recipe built into the program for `car` (the source of truth, `assets/cars/`).
pub fn embedded(car: u32) -> Option<&'static str> {
    match car {
        1 => Some(include_str!("../../../../assets/cars/car01.body")),
        2 => Some(include_str!("../../../../assets/cars/car02.body")),
        3 => Some(include_str!("../../../../assets/cars/car03.body")),
        4 => Some(include_str!("../../../../assets/cars/car04.body")),
        5 => Some(include_str!("../../../../assets/cars/car05.body")),
        6 => Some(include_str!("../../../../assets/cars/car06.body")),
        7 => Some(include_str!("../../../../assets/cars/car07.body")),
        8 => Some(include_str!("../../../../assets/cars/car08.body")),
        9 => Some(include_str!("../../../../assets/cars/car09.body")),
        10 => Some(include_str!("../../../../assets/cars/car10.body")),
        _ => None,
    }
}

/// `--cars-dir`: the recipes come from this directory (the repo's `assets/cars` while they are
/// being edited) instead of the install's `remaster/cars`.
static DIR: OnceLock<PathBuf> = OnceLock::new();

pub fn set_dir(dir: PathBuf) {
    let _ = DIR.set(dir);
}

/// The file that overrides `car`'s embedded recipe: `--cars-dir`'s if set, else the install's.
/// It need not exist (motorview watches it, so one written later counts).
pub fn path(root: &Path, car: u32) -> PathBuf {
    override_path(root, &format!("car{car:02}.body"))
}

/// `name` (a recipe or `.fix` file) in `--cars-dir` if set, else in the install's `remaster/cars`.
pub fn override_path(root: &Path, name: &str) -> PathBuf {
    match DIR.get() {
        Some(d) => d.join(name),
        None => root.join("remaster/cars").join(name),
    }
}

/// The body style for `car`: the install's file if there is one (a broken file is reported and
/// skipped), else the embedded recipe; None for a car without one (it keeps its MYO body).
pub fn load(root: &Path, car: u32) -> Option<BodyStyle> {
    let p = path(root, car);
    if let Ok(text) = std::fs::read_to_string(&p) {
        match from_text(&text) {
            Ok(style) => {
                eprintln!("car{car:02}: body recipe from {}", p.display());
                return Some(style);
            }
            Err(e) => eprintln!("{}: {e}", p.display()),
        }
    }
    embedded(car).map(|t| from_text(t).unwrap_or_else(|e| panic!("embedded car{car:02}.body: {e}")))
}

/// One record's `key=value` pairs, each read once; `done` reports the ones left over.
struct Kv<'a> {
    line: usize,
    name: &'a str,
    pairs: HashMap<&'a str, &'a str>,
}

impl<'a> Kv<'a> {
    fn err(&self, msg: String) -> String {
        format!("line {}: {}: {msg}", self.line, self.name)
    }
    fn take(&mut self, key: &str) -> Result<&'a str, String> {
        self.pairs.remove(key).ok_or_else(|| self.err(format!("needs {key}=")))
    }
    fn num<T: std::str::FromStr>(&self, key: &str, s: &str) -> Result<T, String> {
        s.parse().map_err(|_| self.err(format!("bad number '{s}' for {key}")))
    }
    fn f(&mut self, key: &str) -> Result<f32, String> {
        let s = self.take(key)?;
        self.num(key, s)
    }
    /// an optional number: `default` when the key is not given
    fn f_or(&mut self, key: &str, default: f32) -> Result<f32, String> {
        if self.pairs.contains_key(key) { self.f(key) } else { Ok(default) }
    }
    fn n(&mut self, key: &str) -> Result<usize, String> {
        let s = self.take(key)?;
        self.num(key, s)
    }
    fn b(&mut self, key: &str) -> Result<bool, String> {
        match self.take(key)? {
            "0" | "false" | "no" => Ok(false),
            "1" | "true" | "yes" => Ok(true),
            s => Err(self.err(format!("bad flag '{s}' for {key} (0 or 1)"))),
        }
    }
    fn tuple<const N: usize>(&self, key: &str, s: &str) -> Result<[f32; N], String> {
        let v: Vec<f32> = s.split(',').map(|x| self.num(key, x.trim())).collect::<Result<_, _>>()?;
        v.try_into().map_err(|_| self.err(format!("{key} needs {N} numbers")))
    }
    fn v3(&mut self, key: &str) -> Result<[f32; 3], String> {
        let s = self.take(key)?;
        self.tuple(key, s)
    }
    fn v2(&mut self, key: &str) -> Result<[f32; 2], String> {
        let s = self.take(key)?;
        self.tuple(key, s)
    }
    /// a colour tag `r,g,b`; a texture name is reserved for the lamp picking of #171 (G0)
    fn rgb(&mut self, key: &str) -> Result<[u8; 3], String> {
        let s = self.take(key)?;
        if s.chars().next().is_some_and(|c| c.is_ascii_alphabetic()) {
            return Err(self.err(format!("{key}={s}: texture-name tags are not supported yet (#171 G0)")));
        }
        let v: [f32; 3] = self.tuple(key, s)?;
        Ok(v.map(|c| c.clamp(0.0, 255.0) as u8))
    }
    fn list<const N: usize>(&mut self, key: &str) -> Result<Vec<[f32; N]>, String> {
        let s = self.take(key)?;
        s.split(';').map(|p| self.tuple(key, p.trim())).collect()
    }
    fn done(self) -> Result<(), String> {
        match self.pairs.keys().min() {
            Some(k) => Err(self.err(format!("unknown key '{k}'"))),
            None => Ok(()),
        }
    }
}

/// a recess: its tag under `tag` (`rgb` for the glass and vent housings, `from` for a lamp's)
fn glass_rec(kv: &mut Kv, tag: &str) -> Result<Glass, String> {
    Ok(Glass { rgb: kv.rgb(tag)?, frame: kv.f("frame")?, inset: kv.f("inset")?, tint: kv.f("tint")? })
}

pub fn from_text(text: &str) -> Result<BodyStyle, String> {
    let mut body: Option<BodyStyle> = None;
    let mut mirror_glass = None;
    let mut glass = None;
    let mut flat_glass = Vec::new();
    let mut lamps: Vec<Lamp> = Vec::new();
    let mut vents = Vec::new();
    let mut chamfers = Vec::new();
    let mut panels = Vec::new();
    let mut splitter = None;
    let mut wing = None;
    let mut fins = None;
    let mut foils = Vec::new();
    let mut paints = Vec::new();
    let mut mirror_copies = Vec::new();
    let mut pushes = Vec::new();
    let mut slices = Vec::new();
    let mut thickens: Vec<Thicken> = Vec::new();
    let mut carbon = Vec::new();
    let mut exhaust = None;
    let mut diffusers = Vec::new();
    let mut smooths = Vec::new();
    let mut warps = Vec::new();
    let mut refine = 0.0;
    let mut refine_seams = false;
    for (n, raw) in text.lines().enumerate() {
        let l = raw.split('#').next().unwrap_or("").trim();
        let mut it = l.split_whitespace();
        let Some(name) = it.next() else { continue };
        let line = n + 1;
        let mut pairs = HashMap::new();
        for tok in it {
            let Some((k, v)) = tok.split_once('=') else { return Err(format!("line {line}: {name}: '{tok}' is not key=value")) };
            if pairs.insert(k, v).is_some() {
                return Err(format!("line {line}: {name}: {k} given twice"));
            }
        }
        let mut kv = Kv { line, name, pairs };
        match name {
            "Body" => {
                if body.is_some() {
                    return Err(kv.err("given twice".into()));
                }
                body = Some(BodyStyle {
                    bevel: kv.f("bevel")?,
                    segments: kv.n("segments")?,
                    bevel_tol: kv.f("bevel_tol")?,
                    smooth_deg: kv.f("smooth_deg")?,
                    arch_step_deg: kv.f("arch_step_deg")?,
                    arch_clearance: kv.f("arch_clearance")?,
                    arch_lip: kv.f("arch_lip")?,
                    drop: kv.f("drop")?,
                    centre: kv.f_or("centre", 0.0)?,
                    glass: None,
                    flat_glass: Vec::new(),
                    lamps: Vec::new(),
                    vents: Vec::new(),
                    chamfers: Vec::new(),
                    panels: Vec::new(),
                    splitter: None,
                    wing: None,
                    fins: None,
                    foils: Vec::new(),
                    paints: Vec::new(),
                    mirror_copies: Vec::new(),
                    pushes: Vec::new(),
                    slices: Vec::new(),
                    thickens: Vec::new(),
                    mirror_glass: None,
                    carbon: Vec::new(),
                    exhaust: None,
                    diffusers: Vec::new(),
                    smooths: Vec::new(),
                    warps: Vec::new(),
                    refine: 0.0,
                    refine_seams: false,
                });
            }
            "MirrorGlass" => mirror_glass = Some(kv.rgb("rgb")?),
            "Glass" => glass = Some(glass_rec(&mut kv, "rgb")?),
            "FlatGlass" => flat_glass.push((kv.v3("lo")?, kv.v3("hi")?)),
            "Lamp" => {
                let housing = glass_rec(&mut kv, "from")?;
                let kind = match kv.take("kind")? {
                    "projectors" => LampKind::Projectors { count: kv.n("count")?, radius: kv.f("radius")?, rows: kv.n("rows")?, span: kv.f("span")?, floor: kv.rgb("floor")?, paths: Vec::new(), inset: kv.f("led_inset")? },
                    "reflector" => LampKind::Reflector { count: kv.n("count")?, rows: kv.n("rows")?, pitch: kv.f("pitch")?, bulb: kv.f("bulb")?, paths: Vec::new(), inset: kv.f("led_inset")? },
                    "leds" => LampKind::Leds { paths: Vec::new(), inset: kv.f("led_inset")? },
                    k => return Err(kv.err(format!("unknown kind '{k}' (projectors, reflector, leds)"))),
                };
                lamps.push(Lamp { housing, kind });
            }
            "Led" => {
                let Some(LampKind::Leds { paths, .. } | LampKind::Projectors { paths, .. } | LampKind::Reflector { paths, .. }) = lamps.last_mut().map(|l| &mut l.kind) else {
                    return Err(kv.err("needs a Lamp line before it".into()));
                };
                let pts = kv.list::<2>("pts")?;
                let repeat = kv.n("repeat")?;
                let step = kv.v2("step")?;
                let style = match kv.take("style")? {
                    "dots" => LedStyle::Dots { dot: kv.f("dot")?, pitch: kv.f("pitch")? },
                    "bar" => LedStyle::Bar { width: kv.f("width")? },
                    "rim" => LedStyle::Rim { width: kv.f("width")?, gap: kv.f("gap")?, closed: kv.b("closed")? },
                    k => return Err(kv.err(format!("unknown style '{k}' (dots, bar, rim)"))),
                };
                paths.push(LedPath { pts, repeat, step, style, margin: kv.f("margin")?, clip: kv.f("clip")?, halo: kv.b("halo")? });
            }
            "Vent" => {
                let from = kv.rgb("from")?;
                let (lo, hi) = (kv.v3("lo")?, kv.v3("hi")?);
                let housing = glass_rec(&mut kv, "rgb")?;
                let kind = match kv.take("kind")? {
                    "plain" => VentKind::Plain,
                    "slats" => VentKind::Slats { pitch: kv.f("pitch")?, width: kv.f("width")?, blades: kv.n("blades")?, spine: kv.f("spine")?, tilt: kv.f("tilt")?, level: kv.f_or("level", 0.0)? != 0.0 },
                    "hex" => VentKind::Hex { radius: kv.f("radius")?, wall: kv.f("wall")?, blades: kv.n("blades")? },
                    "louvres" => VentKind::Louvres { count: kv.n("count")?, ribs: kv.n("ribs")?, width: kv.f("width")?, low: kv.f("low")? },
                    k => return Err(kv.err(format!("unknown kind '{k}' (plain, slats, hex, louvres)"))),
                };
                vents.push(Vent { from, lo, hi, housing, kind });
            }
            "Chamfer" => chamfers.push(Chamfer { from: kv.rgb("from")?, corner: kv.v3("corner")?, across: kv.f("across")?, along: kv.f("along")? }),
            "Panel" => panels.push(Panel { from: kv.rgb("from")?, lo: kv.v3("lo")?, hi: kv.v3("hi")?, frame: kv.f("frame")?, rgb: kv.rgb("rgb")? }),
            "Splitter" => splitter = Some(Splitter { edge: kv.list::<3>("edge")?, reach: kv.f("reach")?, depth: kv.f("depth")?, thickness: kv.f("thickness")?, nose: kv.n("nose")?, taper: kv.f("taper")?, carbon: kv.b("carbon")? }),
            "Wing" => {
                wing = Some(Wing {
                    span: kv.f("span")?,
                    chord: kv.f("chord")?,
                    thickness: kv.f("thickness")?,
                    y: kv.f("y")?,
                    z: kv.f("z")?,
                    angle_deg: kv.f("angle_deg")?,
                    plate: kv.f("plate")?,
                    pylon_x: kv.f("pylon_x")?,
                    pylon_width: kv.f("pylon_width")?,
                    pylon_thickness: kv.f("pylon_thickness")?,
                    carbon: kv.b("carbon")?,
                })
            }
            "Foil" => foils.push(Foil {
                // one tag, or a `;` list (#177 phase 3: a wing in two colours)
                from: kv.list::<3>("from")?.into_iter().map(|c| c.map(|x| x as u8)).collect(),
                lo: kv.v3("lo")?,
                hi: kv.v3("hi")?,
                le: kv.list::<3>("le")?,
                te: kv.list::<3>("te")?,
                thickness: kv.f("thickness")?,
                camber: kv.f("camber")?,
                step: kv.f("step")?,
                struts: kv.list::<2>("struts")?,
                strut_chord: kv.f("strut_chord")?,
                strut_width: kv.f("strut_width")?,
            }),
            "Fins" => fins = Some(Fins { xs: kv.list::<1>("xs")?.into_iter().map(|x| x[0]).collect(), z0: kv.f("z0")?, z1: kv.f("z1")?, depth: kv.f("depth")?, thickness: kv.f("thickness")?, carbon: kv.b("carbon")? }),
            "Paint" => paints.push(Paint { from: kv.rgb("from")?, lo: kv.v3("lo")?, hi: kv.v3("hi")?, like: kv.rgb("like")? }),
            "Mirror" => mirror_copies.push(MirrorCopy { lo: kv.v3("lo")?, hi: kv.v3("hi")? }),
            "Push" => pushes.push(Push { lo: kv.v3("lo")?, hi: kv.v3("hi")?, by: kv.v3("by")? }),
            "Slice" => {
                let (lo, hi) = (kv.v3("lo")?, kv.v3("hi")?);
                let axis = match kv.take("axis")? {
                    "x" => 0,
                    "y" => 1,
                    "z" => 2,
                    a => return Err(kv.err(format!("bad axis '{a}' (x, y or z)"))),
                };
                slices.push(Slice { lo, hi, axis, at: kv.f("at")? });
            }
            "Thicken" => thickens.push(Thicken { lo: kv.v3("lo")?, hi: kv.v3("hi")?, thickness: kv.f("thickness")?, splits: Vec::new(), legs: kv.b("legs")? }),
            "Split" => {
                let Some(t) = thickens.last_mut() else { return Err(kv.err("needs a Thicken line before it".into())) };
                t.splits.push((kv.v3("a")?, kv.v3("b")?, kv.v3("c")?));
            }
            "Carbon" => carbon.push(Pick { from: kv.rgb("from")?, lo: kv.v3("lo")?, hi: kv.v3("hi")? }),
            "Exhaust" => {
                exhaust = Some(Exhaust {
                    x: kv.f("x")?,
                    y: kv.f("y")?,
                    z: kv.f("z")?,
                    width: kv.f("width")?,
                    height: kv.f("height")?,
                    chamfer: kv.f("chamfer")?,
                    slant: kv.f("slant")?,
                    round: kv.f("round")?,
                    lip: kv.f("lip")?,
                    pipe: kv.f("pipe")?,
                    depth: kv.f("depth")?,
                    // optional (#388): one tip on the centre line
                    centre: kv.f_or("centre", 0.0)? != 0.0,
                })
            }
            "Diffuser" => {
                diffusers.push(Diffuser {
                    lo: kv.v3("lo")?,
                    hi: kv.v3("hi")?,
                    width: kv.f("width")?,
                    wall: kv.f("wall")?,
                    roof: kv.list::<2>("roof")?,
                    keel: kv.list::<2>("keel")?,
                    keel_bottom: kv.f("keel_bottom")?,
                    keel_top: kv.f("keel_top")?,
                    keel_round: kv.f("keel_round")?,
                    step: kv.f("step")?,
                })
            }
            "Smooth" => {
                // `lo`/`hi`: one box, or a `;` list of boxes (the zone is their union)
                let (lo, hi) = (kv.list::<3>("lo")?, kv.list::<3>("hi")?);
                if lo.len() != hi.len() {
                    return Err(kv.err(format!("Smooth: lo lists {} boxes, hi {}", lo.len(), hi.len())));
                }
                // `round`: one value, or one per box
                let round: Vec<f32> = kv.list::<1>("round")?.into_iter().map(|[r]| r).collect();
                if round.len() != 1 && round.len() != lo.len() {
                    return Err(kv.err(format!("Smooth: round lists {} values for {} boxes", round.len(), lo.len())));
                }
                let tags = |l: Vec<[f32; 3]>| l.into_iter().map(|c| c.map(|x| x as u8)).collect::<Vec<[u8; 3]>>();
                // `max_deg`: one value, or one per box (#465)
                let degs: Vec<f32> = kv.list::<1>("max_deg")?.into_iter().map(|[d]| d).collect();
                if degs.len() != 1 && degs.len() != lo.len() {
                    return Err(kv.err(format!("Smooth: max_deg lists {} values for {} boxes", degs.len(), lo.len())));
                }
                let max_degs = if degs.len() > 1 { degs.clone() } else { Vec::new() };
                // `level`: optional tags whose level edges keep their height (#464)
                let level = if kv.pairs.contains_key("level") { tags(kv.list::<3>("level")?) } else { Vec::new() };
                smooths.push(Smooth { from: tags(kv.list::<3>("from")?), lo, hi, max_deg: degs[0], step: kv.f("step")?, loose: kv.b("loose")?, round, level, max_degs })
            }
            "Warp" => {
                // `y` and `z` are optional: a missing table weighs 1
                let by = kv.v3("by")?;
                let x = kv.list::<2>("x")?;
                let y = if kv.pairs.contains_key("y") { kv.list::<2>("y")? } else { Vec::new() };
                let z = if kv.pairs.contains_key("z") { kv.list::<2>("z")? } else { Vec::new() };
                for (k, t) in [("x", &x), ("y", &y), ("z", &z)] {
                    if t.windows(2).any(|w| w[1][0] <= w[0][0]) {
                        return Err(kv.err(format!("{k}= needs rising coordinates")));
                    }
                }
                warps.push(Warp { by, x, y, z });
            }
            "Refine" => {
                refine = kv.f("step")?;
                refine_seams = kv.f_or("seams", 0.0)? != 0.0;
            }
            _ => return Err(kv.err("unknown record".into())),
        }
        kv.done()?;
    }
    let Some(mut st) = body else { return Err("no Body line".into()) };
    st.glass = glass;
    st.flat_glass = flat_glass;
    st.mirror_glass = mirror_glass;
    st.lamps = lamps;
    st.vents = vents;
    st.chamfers = chamfers;
    st.panels = panels;
    st.splitter = splitter;
    st.wing = wing;
    st.fins = fins;
    st.foils = foils;
    st.paints = paints;
    st.mirror_copies = mirror_copies;
    st.pushes = pushes;
    st.slices = slices;
    st.thickens = thickens;
    st.carbon = carbon;
    st.exhaust = exhaust;
    st.diffusers = diffusers;
    st.smooths = smooths;
    st.warps = warps;
    st.refine = refine;
    st.refine_seams = refine_seams;
    Ok(st)
}

fn v3(p: [f32; 3]) -> String {
    format!("{},{},{}", p[0], p[1], p[2])
}
fn rgb(c: [u8; 3]) -> String {
    format!("{},{},{}", c[0], c[1], c[2])
}
fn glass_text(g: &Glass, tag: &str) -> String {
    format!("{tag}={} frame={} inset={} tint={}", rgb(g.rgb), g.frame, g.inset, g.tint)
}

/// The style as a recipe (no comments; the embedded files carry the annotations).
pub fn to_text(st: &BodyStyle) -> String {
    let mut t = format!("{HEADER}\n");
    // `centre` is optional and written only when set, so the other recipes keep their lines
    let centre = if st.centre != 0.0 { format!(" centre={}", st.centre) } else { String::new() };
    t += &format!(
        "Body bevel={} segments={} bevel_tol={} smooth_deg={} arch_step_deg={} arch_clearance={} arch_lip={} drop={}{centre}\n",
        st.bevel, st.segments, st.bevel_tol, st.smooth_deg, st.arch_step_deg, st.arch_clearance, st.arch_lip, st.drop
    );
    if let Some(c) = st.mirror_glass {
        t += &format!("MirrorGlass rgb={}\n", rgb(c));
    }
    if let Some(g) = &st.glass {
        t += &format!("Glass {}\n", glass_text(g, "rgb"));
    }
    for (lo, hi) in &st.flat_glass {
        t += &format!("FlatGlass lo={} hi={}\n", v3(*lo), v3(*hi));
    }
    for l in &st.lamps {
        t += &format!("Lamp {} ", glass_text(&l.housing, "from"));
        match &l.kind {
            LampKind::Projectors { count, radius, rows, span, floor, paths, inset } => {
                t += &format!("kind=projectors count={count} radius={radius} rows={rows} span={span} floor={} led_inset={inset}\n", rgb(*floor));
                led_lines(&mut t, paths);
            }
            LampKind::Reflector { count, rows, pitch, bulb, paths, inset } => {
                t += &format!("kind=reflector count={count} rows={rows} pitch={pitch} bulb={bulb} led_inset={inset}\n");
                led_lines(&mut t, paths);
            }
            LampKind::Leds { paths, inset } => {
                t += &format!("kind=leds led_inset={inset}\n");
                led_lines(&mut t, paths);
            }
        }
    }
    vents_text(&mut t, st);
    t
}

/// the `Led` lines of a lamp
fn led_lines(t: &mut String, paths: &[LedPath]) {
    for p in paths {
        let pts: Vec<String> = p.pts.iter().map(|q| format!("{},{}", q[0], q[1])).collect();
        *t += &format!("Led pts={} repeat={} step={},{} ", pts.join(";"), p.repeat, p.step[0], p.step[1]);
        match p.style {
            LedStyle::Dots { dot, pitch } => *t += &format!("style=dots dot={dot} pitch={pitch}"),
            LedStyle::Bar { width } => *t += &format!("style=bar width={width}"),
            LedStyle::Rim { width, gap, closed } => *t += &format!("style=rim width={width} gap={gap} closed={}", closed as u8),
        }
        *t += &format!(" margin={} clip={} halo={}\n", p.margin, p.clip, p.halo as u8);
    }
}

/// everything after the lamps
fn vents_text(t: &mut String, st: &BodyStyle) {
    for v in &st.vents {
        *t += &format!("Vent from={} lo={} hi={} {} ", rgb(v.from), v3(v.lo), v3(v.hi), glass_text(&v.housing, "rgb"));
        match v.kind {
            VentKind::Plain => *t += "kind=plain\n",
            VentKind::Slats { pitch, width, blades, spine, tilt, level } => *t += &format!("kind=slats pitch={pitch} width={width} blades={blades} spine={spine} tilt={tilt}{}\n", if level { " level=1" } else { "" }),
            VentKind::Hex { radius, wall, blades } => *t += &format!("kind=hex radius={radius} wall={wall} blades={blades}\n"),
            VentKind::Louvres { count, ribs, width, low } => *t += &format!("kind=louvres count={count} ribs={ribs} width={width} low={low}\n"),
        }
    }
    for c in &st.chamfers {
        *t += &format!("Chamfer from={} corner={} across={} along={}\n", rgb(c.from), v3(c.corner), c.across, c.along);
    }
    for p in &st.panels {
        *t += &format!("Panel from={} lo={} hi={} frame={} rgb={}\n", rgb(p.from), v3(p.lo), v3(p.hi), p.frame, rgb(p.rgb));
    }
    if let Some(s) = &st.splitter {
        let edge: Vec<String> = s.edge.iter().map(|&p| v3(p)).collect();
        *t += &format!("Splitter edge={} reach={} depth={} thickness={} nose={} taper={} carbon={}\n", edge.join(";"), s.reach, s.depth, s.thickness, s.nose, s.taper, s.carbon as u8);
    }
    if let Some(w) = &st.wing {
        *t += &format!(
            "Wing span={} chord={} thickness={} y={} z={} angle_deg={} plate={} pylon_x={} pylon_width={} pylon_thickness={} carbon={}\n",
            w.span, w.chord, w.thickness, w.y, w.z, w.angle_deg, w.plate, w.pylon_x, w.pylon_width, w.pylon_thickness, w.carbon as u8
        );
    }
    for f in &st.foils {
        let pts = |l: &[[f32; 3]]| l.iter().map(|&p| v3(p)).collect::<Vec<_>>().join(";");
        let struts: Vec<String> = f.struts.iter().map(|s| format!("{},{}", s[0], s[1])).collect();
        *t += &format!(
            "Foil from={} lo={} hi={} le={} te={} thickness={} camber={} step={} struts={} strut_chord={} strut_width={}\n",
            f.from.iter().map(|&c| rgb(c)).collect::<Vec<_>>().join(";"), v3(f.lo), v3(f.hi), pts(&f.le), pts(&f.te), f.thickness, f.camber, f.step, struts.join(";"), f.strut_chord, f.strut_width
        );
    }
    if let Some(f) = &st.fins {
        let xs: Vec<String> = f.xs.iter().map(|x| x.to_string()).collect();
        *t += &format!("Fins xs={} z0={} z1={} depth={} thickness={} carbon={}\n", xs.join(";"), f.z0, f.z1, f.depth, f.thickness, f.carbon as u8);
    }
    for p in &st.paints {
        *t += &format!("Paint from={} lo={} hi={} like={}\n", rgb(p.from), v3(p.lo), v3(p.hi), rgb(p.like));
    }
    for m in &st.mirror_copies {
        *t += &format!("Mirror lo={} hi={}\n", v3(m.lo), v3(m.hi));
    }
    for p in &st.pushes {
        *t += &format!("Push lo={} hi={} by={}\n", v3(p.lo), v3(p.hi), v3(p.by));
    }
    for s in &st.slices {
        *t += &format!("Slice lo={} hi={} axis={} at={}\n", v3(s.lo), v3(s.hi), ["x", "y", "z"][s.axis.min(2)], s.at);
    }
    for th in &st.thickens {
        *t += &format!("Thicken lo={} hi={} thickness={} legs={}\n", v3(th.lo), v3(th.hi), th.thickness, th.legs as u8);
        for (a, b, c) in &th.splits {
            *t += &format!("Split a={} b={} c={}\n", v3(*a), v3(*b), v3(*c));
        }
    }
    for p in &st.carbon {
        *t += &format!("Carbon from={} lo={} hi={}\n", rgb(p.from), v3(p.lo), v3(p.hi));
    }
    if let Some(e) = &st.exhaust {
        *t += &format!(
            "Exhaust x={} y={} z={} width={} height={} chamfer={} slant={} round={} lip={} pipe={} depth={}{}\n",
            e.x, e.y, e.z, e.width, e.height, e.chamfer, e.slant, e.round, e.lip, e.pipe, e.depth, if e.centre { " centre=1" } else { "" }
        );
    }
    for d in &st.diffusers {
        let pts = |l: &[[f32; 2]]| l.iter().map(|p| format!("{},{}", p[0], p[1])).collect::<Vec<_>>().join(";");
        *t += &format!(
            "Diffuser lo={} hi={} width={} wall={} roof={} keel={} keel_bottom={} keel_top={} keel_round={} step={}\n",
            v3(d.lo), v3(d.hi), d.width, d.wall, pts(&d.roof), pts(&d.keel), d.keel_bottom, d.keel_top, d.keel_round, d.step
        );
    }
    for s in &st.smooths {
        let boxes = |b: &[[f32; 3]]| b.iter().map(|&p| v3(p)).collect::<Vec<_>>().join(";");
        let tags = |l: &[[u8; 3]]| l.iter().map(|&c| rgb(c)).collect::<Vec<_>>().join(";");
        let level = if s.level.is_empty() { String::new() } else { format!(" level={}", tags(&s.level)) };
        *t += &format!("Smooth from={} lo={} hi={} max_deg={} step={} loose={} round={}{level}\n", tags(&s.from), boxes(&s.lo), boxes(&s.hi), if s.max_degs.is_empty() { s.max_deg.to_string() } else { s.max_degs.iter().map(|d| d.to_string()).collect::<Vec<_>>().join(";") }, s.step, s.loose as u8, s.round.iter().map(|r| r.to_string()).collect::<Vec<_>>().join(";"));
    }
    for w in &st.warps {
        let table = |name: &str, t: &[[f32; 2]]| if t.is_empty() { String::new() } else { format!(" {name}={}", t.iter().map(|p| format!("{},{}", p[0], p[1])).collect::<Vec<_>>().join(";")) };
        *t += &format!("Warp by={}{}{}{}\n", v3(w.by), table("x", &w.x), table("y", &w.y), table("z", &w.z));
    }
    if st.refine > 0.0 {
        // `seams` is optional and written only when set (#177)
        *t += &format!("Refine step={}{}\n", st.refine, if st.refine_seams { " seams=1" } else { "" });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `centre` is optional: 0 when not given and not written then; [O] the Avalanche's is +3 mm
    #[test]
    fn centre_is_optional() {
        let line = "Body bevel=0.015 segments=3 bevel_tol=0.001 smooth_deg=10 arch_step_deg=7.5 arch_clearance=0.035 arch_lip=0.02 drop=0";
        let st = from_text(line).unwrap();
        assert_eq!(st.centre, 0.0);
        assert!(!to_text(&st).contains("centre"));
        assert_eq!(from_text(embedded(6).unwrap()).unwrap().centre, 0.003);
    }

    /// `Refine seams=` is optional: off when not given and not written then (#177)
    #[test]
    fn refine_seams_is_optional() {
        let body = "Body bevel=0.015 segments=3 bevel_tol=0.001 smooth_deg=10 arch_step_deg=7.5 arch_clearance=0.035 arch_lip=0.02 drop=0\n";
        let st = from_text(&format!("{body}Refine step=0.04\n")).unwrap();
        assert!(!st.refine_seams && !to_text(&st).contains("seams"));
        let st = from_text(&format!("{body}Refine step=0.04 seams=1\n")).unwrap();
        assert!(st.refine_seams && to_text(&st).contains("Refine step=0.04 seams=1"));
        // the cars whose seams the mesh check showed closing without a new defect (#177, #387)
        for n in [1, 3, 4, 5, 6] {
            assert!(from_text(embedded(n).unwrap()).unwrap().refine_seams, "car {n}");
        }
    }

    /// every embedded recipe parses and survives a round trip through `to_text`; all ten cars
    /// have one since the Kobra X3's (#179)
    #[test]
    fn embedded_round_trip() {
        for car in 1..=10 {
            let st = from_text(embedded(car).unwrap()).unwrap_or_else(|e| panic!("car{car:02}: {e}"));
            let again = from_text(&to_text(&st)).unwrap_or_else(|e| panic!("car{car:02} round trip: {e}"));
            assert_eq!(st, again, "car{car:02}");
        }
        let titan = from_text(embedded(10).unwrap()).unwrap();
        assert_eq!((titan.lamps.len(), titan.vents.len(), titan.smooths.len(), titan.thickens[0].splits.len()), (2, 6, 2, 1));
        assert!(titan.uses_carbon());
        assert!(embedded(0).is_none() && embedded(11).is_none());
    }

    #[test]
    fn errors_name_the_line() {
        let base = "Body bevel=0.02 segments=3 bevel_tol=0.001 smooth_deg=12 arch_step_deg=7.5 arch_clearance=0.04 arch_lip=0.02 drop=0.016\n";
        assert!(from_text("# nothing\n").unwrap_err().contains("no Body"));
        let e = from_text(&format!("{base}Vent from=1,1,1 lo=0,0,0 hi=1,1,1 rgb=3,3,3 frame=0.01 inset=0.03 tint=0.5 kind=plain lp=1\n")).unwrap_err();
        assert!(e.starts_with("line 2: Vent: unknown key 'lp'"), "{e}");
        let e = from_text(&format!("{base}Mirror lo=0,0 hi=1,1,1\n")).unwrap_err();
        assert!(e.contains("lo needs 3 numbers"), "{e}");
        let e = from_text(&format!("{base}Led pts=0,0;1,0 repeat=1 step=0,0 style=bar width=0.02 margin=0 clip=0 halo=1\n")).unwrap_err();
        assert!(e.contains("Lamp line"), "{e}");
        let e = from_text(&format!("{base}Carbon from=Grill1 lo=0,0,0 hi=1,1,1\n")).unwrap_err();
        assert!(e.contains("texture-name"), "{e}");
        let e = from_text(&format!("{base}Spoiler lo=0,0,0\n")).unwrap_err();
        assert!(e.contains("unknown record"), "{e}");
    }
}
