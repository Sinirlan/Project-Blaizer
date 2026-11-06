//! DEBUG motorview's Constants page (F11 dock, #398): every constant of the game's source (motorview, motor-sim,
//! motor-audio; the catalog build.rs writes from the source, `motor_tune::scan`), by area and file, with its doc as
//! the description. A `tunable!` value changes live and is kept in `tune.cfg` (the game loads it too); a WGSL `const`
//! changes when the shaders are rebuilt (`motor_tune::wgsl`, `Gpu::rebuild_shaders`) and is kept there too; a Rust
//! `const` is edited here and written back into its source file, for the next build.

use crate::glass_ui;
use crate::icons as ic;
use crate::theme;
use egui::{Color32, RichText, Ui};
use motor_tune::catalog::{self, Kind};
use motor_tune::Erased;
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

include!(concat!(env!("OUT_DIR"), "/consts_catalog.rs"));

/// `tune.cfg`, beside settings.cfg.
pub fn cfg_path() -> Option<PathBuf> {
    crate::ghost::config_dir().map(|d| d.join("tune.cfg"))
}

/// Apply `tune.cfg` (when there is one) to this program's tunables, and say so on stdout.
pub fn load_overrides() {
    let Some(p) = cfg_path() else { return };
    let Ok(text) = std::fs::read_to_string(&p) else { return };
    let mut l = motor_tune::load_str(&text);
    // a shader value of a constant the shaders don't have (renamed or removed since it was saved)
    for (f, n, v, _) in motor_tune::wgsl::all() {
        if !CATALOG.iter().any(|e| e.kind == Kind::Wgsl && e.name == n && motor_tune::wgsl::file_key(e.file) == f) {
            motor_tune::wgsl::set(&f, &n, None);
            l.applied -= 1;
            l.unknown.push(format!("{f}::{n} {v}"));
        }
    }
    println!("tune: {} tuning override(s) active from {}", l.applied, p.display());
    for u in l.unknown {
        println!("tune: not applied (no such tunable here, or a bad value): {u}");
    }
}

/// Write the modified tunables to `tune.cfg` (none: the file is removed).
fn save_overrides() -> Result<String, String> {
    let p = cfg_path().ok_or("no settings directory")?;
    let n = motor_tune::TUNABLES.iter().filter(|t| t.modified()).count() + motor_tune::wgsl::all().iter().filter(|o| o.3).count();
    if n == 0 {
        if p.exists() {
            std::fs::remove_file(&p).map_err(|e| format!("{}: {e}", p.display()))?;
        }
        return Ok(format!("no overrides: {} removed", p.display()));
    }
    if let Some(d) = p.parent() {
        std::fs::create_dir_all(d).map_err(|e| format!("{}: {e}", d.display()))?;
    }
    std::fs::write(&p, motor_tune::save_string()).map_err(|e| format!("{}: {e}", p.display()))?;
    Ok(format!("{n} override(s) saved to {}", p.display()))
}

/// The workspace this program was built from (write-back target), when it is still there.
fn source_root() -> Option<PathBuf> {
    let r = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    r.join("Cargo.toml").is_file().then_some(r)
}

/// An edit of a compiled-in constant, not written back yet.
#[derive(Clone)]
enum Edit {
    Num(f64),
    Text(String),
}

/// The page's state (in `tune::Tune`).
#[derive(Default)]
pub struct ConstsPage {
    /// built on first show: the live tunable of each catalog entry, its source value as a number, the area tree
    live: Vec<Option<&'static dyn Erased>>,
    nums: Vec<Option<f64>>,
    tree: BTreeMap<String, BTreeMap<String, Vec<usize>>>,
    edits: HashMap<usize, Edit>,
    /// values written back this session: the row's source value until the next build
    written: HashMap<usize, String>,
    live_only: bool,
    modified_only: bool,
    undocumented: bool,
    msg: Option<(String, bool)>,
    /// a live value read when the track loads changed: the Reload button is lit
    reload_due: bool,
    /// Reload was pressed: the viewer reloads the scene (`tune::Tune::reload_req`)
    pub reload_req: bool,
    /// a shader value changed since the shaders were built: the Rebuild button is lit
    shader_due: bool,
    /// Rebuild shaders was pressed: the viewer calls `Gpu::rebuild_shaders` and passes the result to `rebuilt`
    pub shader_req: bool,
}

impl ConstsPage {
    fn init(&mut self) {
        if !self.nums.is_empty() || CATALOG.is_empty() {
            return;
        }
        self.live = CATALOG.iter().map(|e| if e.kind == Kind::Live { motor_tune::find(e.file, e.name) } else { None }).collect();
        self.nums = CATALOG.iter().map(|e| e.number(CATALOG)).collect();
        for (i, e) in CATALOG.iter().enumerate() {
            let (a, s) = e.area();
            self.tree.entry(a).or_default().entry(s).or_default().push(i);
        }
    }

    /// The dock header's caption.
    pub fn caption(&self) -> String {
        let live = motor_tune::TUNABLES.len();
        format!("{} constants · {live} live", CATALOG.len())
    }

    fn modified(&self, i: usize) -> bool {
        let e = &CATALOG[i];
        self.live[i].is_some_and(|t| t.modified()) || self.edits.contains_key(&i) || (e.kind == Kind::Wgsl && motor_tune::wgsl::saved(e.file, e.name))
    }

    /// What `Gpu::rebuild_shaders` said (the viewer calls it when `shader_req` was set).
    pub fn rebuilt(&mut self, r: Result<usize, String>) {
        self.msg = Some(match r {
            Ok(n) => {
                self.shader_due = false;
                (format!("{n} shader sources rebuilt"), false)
            }
            Err(e) => (format!("shaders not rebuilt: {e}"), true),
        });
    }

    /// Set the shader value of row `i` to `text`: the source's value (or the one written back this session) drops the
    /// override, any other value is kept for `tune.cfg`. Applies when the shaders are rebuilt.
    fn wgsl_put(&mut self, i: usize, text: &str) {
        let e = &CATALOG[i];
        let text = text.trim();
        match self.written.get(&i) {
            Some(w) if w.trim() == text => {
                motor_tune::wgsl::set(e.file, e.name, Some(text));
                motor_tune::wgsl::mark_written(e.file, e.name);
            }
            None if e.value.trim() == text => motor_tune::wgsl::set(e.file, e.name, None),
            _ => motor_tune::wgsl::set(e.file, e.name, Some(text)),
        }
        self.shader_due = true;
    }

    fn shown(&self, i: usize, f: &str) -> bool {
        let e = &CATALOG[i];
        (!self.live_only || self.live[i].is_some() || e.kind == Kind::Wgsl)
            && (!self.modified_only || self.modified(i))
            && (!self.undocumented || e.doc.is_empty())
            && (f.is_empty() || e.name.to_lowercase().contains(f) || e.doc.to_lowercase().contains(f) || e.file.contains(f))
    }

    pub fn ui(&mut self, ui: &mut Ui) {
        self.init();
        let f = glass_ui::filter(ui.ctx());
        self.header(ui);
        let filtering = !f.is_empty() || self.live_only || self.modified_only || self.undocumented;
        let tree = std::mem::take(&mut self.tree);
        for (area, subs) in &tree {
            let hits: Vec<(&String, Vec<usize>)> = subs.iter().map(|(s, v)| (s, v.iter().copied().filter(|&i| self.shown(i, &f)).collect::<Vec<_>>())).filter(|(_, v)| !v.is_empty()).collect();
            let n: usize = hits.iter().map(|h| h.1.len()).sum();
            if n == 0 {
                continue;
            }
            let id = ui.make_persistent_id(("consts area", area));
            let mut open = ui.ctx().data_mut(|d| *d.get_temp_mut_or_insert_with(id, || false));
            ui.add_space(4.0);
            glass_ui::section_header(ui, area_icon(area), area, Some(&n.to_string()), &mut open, false);
            ui.ctx().data_mut(|d| d.insert_temp(id, open));
            if !(open || filtering) {
                continue;
            }
            for (sub, rows) in hits {
                let h = egui::CollapsingHeader::new(RichText::new(format!("{sub}  ·  {}", rows.len())).color(theme::DIM)).id_salt(("consts sub", area, sub));
                let h = if filtering { h.open(Some(true)) } else { h };
                h.show(ui, |ui| {
                    for i in rows {
                        self.row(ui, i);
                    }
                });
            }
        }
        self.tree = tree;
    }

    fn header(&mut self, ui: &mut Ui) {
        let modified = (0..CATALOG.len()).filter(|&i| self.modified(i)).count();
        let undoc = CATALOG.iter().filter(|e| e.doc.is_empty()).count();
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = egui::vec2(6.0, 6.0);
            glass_ui::chip(ui, &CATALOG.len().to_string(), "constants");
            glass_ui::chip(ui, &motor_tune::TUNABLES.len().to_string(), "live");
            glass_ui::chip(ui, &modified.to_string(), "modified");
            glass_ui::chip(ui, &undoc.to_string(), "undocumented");
        });
        ui.add_space(4.0);
        ui.horizontal_wrapped(|ui| {
            if ui.button(format!("{} Save tune.cfg", ic::SAVE)).on_hover_text("keep the live values that differ from the source (motorview and the game load them at start)").clicked() {
                self.msg = Some(match save_overrides() {
                    Ok(m) => (m, false),
                    Err(e) => (e, true),
                });
            }
            if ui.button(format!("{} Revert live", ic::UNDO)).on_hover_text("every live and shader value back to its source value").clicked() {
                for t in motor_tune::TUNABLES.iter() {
                    t.reset();
                }
                self.reload_due = true;
                for (f, n, _, save) in motor_tune::wgsl::all() {
                    if save {
                        motor_tune::wgsl::set(&f, &n, None);
                        self.shader_due = true;
                    }
                }
            }
            let reload = egui::Button::new(RichText::new(format!("{} Reload track", ic::RELOAD)).color(if self.reload_due { theme::ORANGE_TEXT } else { theme::TEXT }));
            if ui.add(reload).on_hover_text("values marked 'reload' are read when the track loads").clicked() {
                self.reload_req = true;
                self.reload_due = false;
            }
            let rebuild = egui::Button::new(RichText::new(format!("{} Rebuild shaders", ic::BOLT)).color(if self.shader_due { theme::ORANGE_TEXT } else { theme::TEXT }));
            if ui.add(rebuild).on_hover_text("compile the shaders with the shader values set here (a value that does not compile keeps the old ones)").clicked() {
                self.shader_req = true;
            }
        });
        ui.horizontal_wrapped(|ui| {
            ui.checkbox(&mut self.live_only, "live only");
            ui.checkbox(&mut self.modified_only, "modified");
            ui.checkbox(&mut self.undocumented, "undocumented");
        });
        if let Some((m, err)) = &self.msg {
            ui.label(RichText::new(m).small().color(if *err { theme::ORANGE_TEXT } else { theme::DIM }));
        }
        ui.add_space(2.0);
    }

    fn row(&mut self, ui: &mut Ui, i: usize) {
        let e = &CATALOG[i];
        let live = self.live[i];
        let modified = self.modified(i);
        ui.add_space(3.0);
        egui::Frame::NONE.fill(theme::white(0.025)).corner_radius(6.0).inner_margin(egui::Margin::symmetric(8, 5)).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal_wrapped(|ui| {
                if modified {
                    ui.label(RichText::new("●").color(theme::ORANGE_TEXT));
                }
                let name = ui.label(RichText::new(e.name).monospace().color(theme::NAME));
                name.on_hover_text(format!("{}:{}\ntype {}\nsource value: {}", e.file, e.line, if e.ty.is_empty() { "(inferred)" } else { e.ty }, e.value));
                let (badge, c) = match (e.kind, live) {
                    (Kind::Live, Some(_)) => ("live", Color32::from_rgb(0x7c, 0xf0, 0xa8)),
                    (Kind::Live, None) => ("live in the game", theme::DIM),
                    (Kind::Wgsl, _) => ("shader", theme::S1),
                    (Kind::Build, _) => ("build", theme::DIM),
                    (Kind::Fixed, _) => ("text", theme::DIM),
                };
                ui.label(RichText::new(badge).small().color(c));
                if !e.tag.is_empty() {
                    ui.label(RichText::new(e.tag.to_lowercase()).small().color(theme::ORANGE));
                }
                if let Some(a) = motor_tune::doc_tag(e.doc, "applies").filter(|a| *a != "frame") {
                    ui.label(RichText::new(format!("applies: {a}")).small().color(theme::DIM));
                }
            });
            let text = e.text();
            if text.is_empty() {
                ui.label(RichText::new("no description").small().italics().color(theme::VERSION));
            } else {
                ui.label(RichText::new(if e.group { format!("{text} (shared)") } else { text }).small().color(theme::DIM));
            }
            ui.horizontal(|ui| self.control(ui, i));
        });
    }

    /// The row's editor: a slider with a typed field (numbers), a box (bools) or a text field, then reset and
    /// write-back.
    fn control(&mut self, ui: &mut Ui, i: usize) {
        let e = &CATALOG[i];
        let live = self.live[i];
        let unit = motor_tune::doc_tag(e.doc, "unit").unwrap_or("");
        // the value the source holds now (written back this session, else the build's)
        let base_num = match (live, self.written.get(&i)) {
            (Some(t), _) => Some(t.default_f64()),
            (None, Some(w)) => catalog::eval(w, &|_| None).or(match w.as_str() {
                "true" => Some(1.0),
                "false" => Some(0.0),
                _ => None,
            }),
            (None, None) => self.nums[i],
        };
        let base_text = self.written.get(&i).cloned().unwrap_or_else(|| e.value.to_string());
        // a shader value: the override in place of `edits`
        let shader = (e.kind == Kind::Wgsl).then(|| motor_tune::wgsl::get(e.file, e.name)).flatten();
        let shader_num = shader.as_deref().and_then(|t| catalog::eval(t, &|_| None).or(match t {
            "true" => Some(1.0),
            "false" => Some(0.0),
            _ => None,
        }));
        let room = (ui.available_width() - 64.0).max(80.0);
        let mut new_text = None;
        match (base_num, e.slider(), e.is_bool()) {
            (Some(b), _, true) => {
                let mut on = match (live, self.edits.get(&i), shader_num) {
                    (Some(t), _, _) => t.value() != 0.0,
                    (None, _, Some(v)) => v != 0.0,
                    (None, Some(Edit::Num(v)), _) => *v != 0.0,
                    _ => b != 0.0,
                };
                if ui.checkbox(&mut on, "").changed() {
                    self.set_num(i, if on { 1.0 } else { 0.0 }, b);
                }
            }
            (Some(b), Some(int), _) => {
                let mut v = match (live, self.edits.get(&i), shader_num) {
                    (Some(t), _, _) => t.value(),
                    (None, _, Some(v)) => v,
                    (None, Some(Edit::Num(v)), _) => *v,
                    _ => b,
                };
                let (lo, hi) = motor_tune::doc_range(e.doc).unwrap_or_else(|| auto_range(b, int));
                ui.spacing_mut().slider_width = (room - 90.0).max(60.0);
                let mut s = egui::Slider::new(&mut v, lo..=hi).clamping(egui::SliderClamping::Never).suffix(if unit.is_empty() { String::new() } else { format!(" {unit}") });
                s = if int { s.integer() } else { s.max_decimals(5) };
                if ui.add(s).changed() {
                    self.set_num(i, v, b);
                }
            }
            _ => {
                let mut t = match (self.edits.get(&i), &shader) {
                    (_, Some(t)) => t.clone(),
                    (Some(Edit::Text(t)), _) => t.clone(),
                    _ => base_text.clone(),
                };
                if ui.add(egui::TextEdit::singleline(&mut t).code_editor().desired_width(room)).changed() {
                    if e.kind == Kind::Wgsl {
                        self.wgsl_put(i, &t);
                    } else if t == base_text {
                        self.edits.remove(&i);
                    } else {
                        self.edits.insert(i, Edit::Text(t));
                    }
                }
            }
        }
        // reset
        let modified = self.modified(i);
        if ui.add_enabled(modified, egui::Button::new(ic::RESET).small()).on_hover_text("back to the source value").clicked() {
            match live {
                Some(t) => t.reset(),
                None if e.kind == Kind::Wgsl => self.wgsl_put(i, &base_text),
                None => {
                    self.edits.remove(&i);
                }
            }
        }
        // write back
        let root = source_root();
        let what = match (live, self.edits.get(&i)) {
            (Some(t), _) if t.modified() => Some(catalog::literal(e, t.value())),
            (None, _) if modified && e.kind == Kind::Wgsl => shader.clone(),
            (None, Some(Edit::Num(v))) => Some(catalog::literal(e, *v)),
            (None, Some(Edit::Text(t))) => Some(t.clone()),
            _ => None,
        };
        let tip = match &root {
            Some(_) => format!("write this value into {}:{}", e.file, e.line),
            None => "the source tree this program was built from is not here".to_string(),
        };
        if ui.add_enabled(what.is_some() && root.is_some(), egui::Button::new(ic::UPLOAD).small()).on_hover_text(tip).clicked() {
            if let (Some(v), Some(root)) = (what, root) {
                new_text = Some((v, root));
            }
        }
        if let Some((v, root)) = new_text {
            self.msg = Some(match catalog::write_back(&root, e, &v) {
                Ok(()) => {
                    match live {
                        Some(t) => t.rebase(),
                        // the shader keeps the value through the override (this build has the old one), not saved
                        None if e.kind == Kind::Wgsl => {
                            motor_tune::wgsl::mark_written(e.file, e.name);
                            self.written.insert(i, v.clone());
                        }
                        None => {
                            self.edits.remove(&i);
                            self.written.insert(i, v.clone());
                        }
                    }
                    let then = match (live, e.kind) {
                        (Some(_), _) => "",
                        (None, Kind::Wgsl) => " (live through the shader rebuild until the next build)",
                        _ => " (rebuild to apply)",
                    };
                    (format!("{} = {v} written to {}:{}{then}", e.name, e.file, e.line), false)
                }
                Err(err) => (err, true),
            });
        }
    }

    fn set_num(&mut self, i: usize, v: f64, base: f64) {
        let e = &CATALOG[i];
        let v = if e.slider() == Some(true) { v.round() } else { v };
        match self.live[i] {
            Some(t) => {
                t.set_value(v);
                self.reload_due |= motor_tune::doc_tag(e.doc, "applies") == Some("reload");
            }
            None if e.kind == Kind::Wgsl => {
                let text = if v == base { self.written.get(&i).cloned().unwrap_or_else(|| e.value.to_string()) } else { catalog::literal(e, v) };
                self.wgsl_put(i, &text);
            }
            None if v == base => {
                self.edits.remove(&i);
            }
            None => {
                self.edits.insert(i, Edit::Num(v));
            }
        }
    }
}

/// The slider's range when the doc gives none: 0..2v around a positive value, 2v..0 around a negative one.
fn auto_range(v: f64, int: bool) -> (f64, f64) {
    let (lo, hi) = if v > 0.0 {
        (0.0, 2.0 * v)
    } else if v < 0.0 {
        (2.0 * v, 0.0)
    } else {
        (-1.0, 1.0)
    };
    if int {
        (lo.floor().min(0.0), hi.ceil().max(10.0))
    } else {
        (lo, hi)
    }
}

fn area_icon(area: &str) -> &'static str {
    match area {
        "Physics" => ic::CAR,
        "AI" => ic::ROUTE,
        "Race" => ic::FLAG,
        "Camera" | "Camera & drive" => ic::CAMERA,
        "Gameplay" => ic::SPARKLES,
        "Graphics" => ic::SUN,
        "Audio" => ic::WAVES,
        "HUD" => ic::GAUGE,
        "Menus" => ic::LIST,
        "Input" => ic::KEYBOARD,
        "Cinematics" => ic::FILM,
        "Procedural cars" => ic::WRENCH,
        _ => ic::SLIDERS,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The catalog was built from this source tree and lists the constants it should.
    #[test]
    fn catalog_lists_the_source() {
        assert!(CATALOG.len() > 1500, "catalog has {} entries", CATALOG.len());
        assert!(CATALOG.iter().any(|e| e.name == "PACK_SPEC" && e.kind == Kind::Wgsl));
        assert!(CATALOG.iter().any(|e| e.file.starts_with("crates/motor-sim/")));
        // every live entry of motorview's own code is linked into it
        for e in CATALOG.iter().filter(|e| e.kind == Kind::Live && !e.file.contains("/bin/")) {
            assert!(motor_tune::find(e.file, e.name).is_some(), "{} ({}) not registered", e.name, e.file);
        }
    }

    /// Every tunable explains itself (the page shows the doc as its description).
    #[test]
    fn tunables_documented() {
        let bare: Vec<_> = motor_tune::TUNABLES.iter().filter(|t| motor_tune::doc_text(t.meta().doc).is_empty()).map(|t| t.meta().key()).collect();
        assert!(bare.is_empty(), "tunables without a doc comment: {bare:?}");
    }

    /// A tunable's source value lies inside its `range:` tag (else the slider starts off its track).
    #[test]
    fn tunable_defaults_inside_range() {
        let bad: Vec<_> = motor_tune::TUNABLES
            .iter()
            .filter_map(|t| {
                let (lo, hi) = motor_tune::doc_range(t.meta().doc)?;
                let d = t.default_f64();
                (d < lo || d > hi).then(|| format!("{} = {d} outside {lo}..{hi}", t.meta().key()))
            })
            .collect();
        assert!(bad.is_empty(), "defaults outside their range: {bad:?}");
    }
}
