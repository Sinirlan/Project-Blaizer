//! DEBUG motorview's command search (#436, Blender F3 / VS Code Ctrl+Shift+P): Space opens one centred glass box that
//! finds every key action (`BINDS`, with its key cap), the held keys, the tracks and cars, the tool shelf's groups, the
//! dock pages and the Constants page's entries; and, harvested the pass the box opens (`harvest`), every toolbar item
//! (Enter opens its tab and flyout) and every dock section and property (Enter opens its page searched for it). Typing ranks the rows (`score`), the arrows pick, Enter runs the
//! picked row (`Run`, applied by main.rs `App::command_run`), Esc closes. The picks of this session come first. Help >
//! Shortcuts opens the same box. The game does not use it.

use super::ribbon::{groups, RTab, CARS, TRACKS};
use super::{Act, Ctx, BINDS, HELD_FREE, HELD_RACE};
use egui::{Color32, Context, Id, Key, Modifiers, RichText, Vec2};
use motorview::consts::CATALOG;
use motorview::icons as ic;
use motorview::tune::Page;
use motorview::world_edit::Kind;
use motorview::glass_ui::{self, Harvested};
use motorview::theme;
use std::collections::VecDeque;

/// What a row does when it is run.
#[derive(Clone, PartialEq)]
pub(crate) enum Run {
    /// an action of the key table (and its key's position in the bind, or the choice)
    Act(Act, usize),
    /// a held key (WASD, ...): listed, nothing to run
    Held,
    /// a group of the tool shelf: its tab, its index in `groups(tab)`
    Toolbar(RTab, usize),
    /// a dock page, the dock search set to the text
    Dock(Page, String),
    /// an entry of the Constants page (the page opens filtered to its name)
    Const(&'static str),
}

/// One row of the box.
pub(crate) struct Entry {
    pub(crate) icon: &'static str,
    pub(crate) name: String,
    /// dim text after the name: where it applies, what kind of row it is
    pub(crate) hint: String,
    /// the key cap ("" = none)
    pub(crate) key: String,
    /// the key table context the action needs (`Ctx::Any` for every other row)
    pub(crate) ctx: Ctx,
    pub(crate) run: Run,
    /// the name, hint and key in lower case (what `score` reads)
    low: (String, String, String),
}

impl Entry {
    fn new(icon: &'static str, name: impl Into<String>, hint: impl Into<String>, key: impl Into<String>, ctx: Ctx, run: Run) -> Entry {
        let (name, hint, key) = (name.into(), hint.into(), key.into());
        let low = (name.to_lowercase(), hint.to_lowercase(), key.to_lowercase());
        Entry { icon, name, hint, key, ctx, run, low }
    }
}

fn ctx_name(c: Ctx) -> &'static str {
    match c {
        Ctx::Any => "",
        Ctx::Race => "race camera",
        Ctx::Free => "free camera",
        Ctx::World => "world editor",
    }
}

/// Every row, in the order an empty search lists them: key actions, held keys, tracks, cars, the shelf's groups, the
/// dock pages, the constants (`CONST_FROM` on, listed only when searched).
pub(crate) fn entries() -> Vec<Entry> {
    let mut v = Vec::new();
    for b in BINDS.iter().filter(|b| b.act != Act::Command) {
        let hint = ctx_name(b.ctx);
        match b.act {
            // one row per key of a range bind
            Act::Track => {
                for (i, t) in TRACKS.iter().enumerate() {
                    v.push(Entry::new(ic::ROUTE, format!("track {} {t}", i + 1), "switch track", DIGITS[i], b.ctx, Run::Act(Act::Track, i)));
                }
            }
            Act::WorldNew => {
                for (i, k) in Kind::ALL.iter().enumerate() {
                    v.push(Entry::new(ic::MOUNTAIN, format!("world: new {} piece", k.name()), hint, CTRL_DIGITS[i], b.ctx, Run::Act(Act::WorldNew, i)));
                }
            }
            a => v.push(Entry::new(ic::KEYBOARD, b.label(), hint, b.name, b.ctx, Run::Act(a, 0))),
        }
    }
    for (title, keys) in [("free camera, held", HELD_FREE), ("race camera, held", HELD_RACE)] {
        for &(k, what) in keys {
            v.push(Entry::new(ic::KEYBOARD, what, title, k, Ctx::Any, Run::Held));
        }
    }
    for (i, c) in CARS.iter().enumerate() {
        v.push(Entry::new(ic::CAR, format!("car {:02} {c}", i + 1), "switch car", "", Ctx::Any, Run::Act(Act::Car, i)));
    }
    for tab in RTab::ALL {
        for (gi, g) in groups(tab).iter().enumerate() {
            v.push(Entry::new(tab.icon(), g.name, format!("toolbar · {} tab", tab.name()), "", Ctx::Any, Run::Toolbar(tab, gi)));
        }
    }
    for p in Page::ALL {
        v.push(Entry::new(p.icon(), format!("{} page", p.name()), "dock", "", Ctx::Any, Run::Dock(p, String::new())));
    }
    for e in CATALOG {
        let (a, s) = e.area();
        v.push(Entry::new(ic::SLIDERS, e.name, format!("constant · {a} / {s}"), "", Ctx::Any, Run::Const(e.name)));
    }
    v
}

/// A section title without its live count (`Race (8 cars, 3 laps)` -> `Race`).
fn bare(title: &str) -> &str {
    title.split_once(" (").map_or(title, |(t, _)| t).trim()
}

/// The rows of a harvest (`Ribbon::harvest`, `Tune::harvest`): the toolbar items, the dock sections and properties.
/// A row seen twice (a page's lamps share their labels) is listed once, as the first one.
pub(crate) fn harvested(bar: &[(RTab, usize, Harvested)], dock: &[(Page, Harvested)]) -> Vec<Entry> {
    let mut seen = std::collections::HashSet::new();
    let mut v = Vec::new();
    for (tab, gi, h) in bar {
        let hint = format!("toolbar · {} / {}", tab.name(), groups(*tab)[*gi].name);
        if seen.insert((h.label.clone(), hint.clone())) {
            v.push(Entry::new(tab.icon(), h.label.clone(), hint, h.key.clone(), Ctx::Any, Run::Toolbar(*tab, *gi)));
        }
    }
    for (page, h) in dock {
        let sect = bare(&h.section);
        let (name, hint) = match (h.label.as_str(), sect) {
            ("", s) => (s.to_string(), format!("dock · {} section", page.name())),
            (l, "") => (l.to_string(), format!("dock · {}", page.name())),
            (l, s) => (l.to_string(), format!("dock · {} / {s}", page.name())),
        };
        if !name.is_empty() && seen.insert((name.clone(), hint.clone())) {
            v.push(Entry::new(page.icon(), name.clone(), hint, h.key.clone(), Ctx::Any, Run::Dock(*page, name)));
        }
    }
    v
}

const DIGITS: [&str; 8] = ["1", "2", "3", "4", "5", "6", "7", "8"];
const CTRL_DIGITS: [&str; 10] = ["Ctrl+1", "Ctrl+2", "Ctrl+3", "Ctrl+4", "Ctrl+5", "Ctrl+6", "Ctrl+7", "Ctrl+8", "Ctrl+9", "Ctrl+0"];

/// `t` is at the start of a word of `s` (after a space or punctuation).
fn word_start(s: &str, t: &str) -> bool {
    s.match_indices(t).any(|(i, _)| s[..i].chars().next_back().is_none_or(|c| !c.is_alphanumeric()))
}

/// The letters of `t` appear in `s` in order.
fn subsequence(s: &str, t: &str) -> bool {
    let mut it = s.chars();
    t.chars().all(|c| it.any(|x| x == c))
}

/// How well `e` matches the lower-case search `q`: every word of it must match the name, the hint or the key (the
/// name itself ranks first, then a name that starts with it, then a word of the name starting with it, the name holding it, the key equal to it,
/// the hint or key holding it, the name's letters in order). None = no match.
pub(crate) fn score(e: &Entry, q: &str) -> Option<i32> {
    let (name, hint, key) = (&e.low.0, &e.low.1, &e.low.2);
    let mut total = 0;
    for t in q.split_whitespace() {
        total += if name == t {
            120
        } else if name.starts_with(t) {
            100
        } else if key == t {
            90
        } else if word_start(name, t) {
            70
        } else if name.contains(t) {
            50
        } else if key.contains(t) || word_start(hint, t) {
            30
        } else if hint.contains(t) {
            20
        } else if t.len() >= 2 && subsequence(name, t) {
            5
        } else {
            return None;
        };
    }
    Some(total)
}

/// What the viewer is doing now: which key table contexts run.
#[derive(Clone, Copy)]
pub(crate) struct Now {
    pub free: bool,
    pub world: bool,
}

impl Now {
    /// A row of context `c` runs now; else the note why not.
    pub(crate) fn runs(self, c: Ctx) -> Result<(), &'static str> {
        match c {
            Ctx::Race if self.free => Err("needs the race camera (Tab)"),
            Ctx::Free if !self.free => Err("needs the free camera (Tab)"),
            Ctx::World if !self.world => Err("needs the world editor (Ctrl+E)"),
            _ => Ok(()),
        }
    }
}

/// Most rows a search lists.
const MAX_HITS: usize = 80;
/// Rows of the list before it scrolls.
const LIST_ROWS: f32 = 12.0;
/// Picks remembered (this session).
const RECENT: usize = 8;

/// The box's state (kept by the viewer).
#[derive(Default)]
pub(crate) struct CommandBox {
    pub(crate) open: bool,
    pub(crate) text: String,
    /// the picked row of the list
    sel: usize,
    /// the static rows (built on first open), then the harvested ones (`harvest`, dropped when the box closes)
    all: Vec<Entry>,
    statics: usize,
    harvested: bool,
    /// the picks of the session by name and hint (a harvest may list them elsewhere), the last first
    recent: VecDeque<(String, String)>,
    /// the text the list was ranked for, and the ranked indices
    ranked: Option<(String, Vec<usize>)>,
}

impl CommandBox {
    /// Open the box with an empty search (or close it).
    pub(crate) fn toggle(&mut self) {
        if self.open {
            self.close();
        } else {
            self.open = true;
        }
        self.text.clear();
        self.sel = 0;
    }

    /// The box is open without the harvested rows: the caller harvests the toolbar and the dock this pass.
    pub(crate) fn wants_harvest(&self) -> bool {
        self.open && !self.harvested
    }

    /// Add the harvested rows (`harvested`) after the static ones.
    pub(crate) fn harvest(&mut self, bar: &[(RTab, usize, Harvested)], dock: &[(Page, Harvested)]) {
        self.statics();
        self.all.truncate(self.statics);
        self.all.extend(harvested(bar, dock));
        self.harvested = true;
        self.ranked = None;
    }

    fn statics(&mut self) {
        if self.all.is_empty() {
            self.all = entries();
            self.statics = self.all.len();
        }
    }

    fn is_recent(&self, e: &Entry) -> bool {
        self.recent.iter().any(|(n, h)| *n == e.name && *h == e.hint)
    }

    /// The box closed (also by Esc in the viewer): the harvested rows go, the next open harvests afresh.
    pub(crate) fn close(&mut self) {
        self.open = false;
        if self.harvested {
            self.all.truncate(self.statics);
            self.harvested = false;
            self.ranked = None;
        }
    }

    /// The box open on `text` (`--command`).
    pub(crate) fn shot(text: &str) -> CommandBox {
        CommandBox { open: true, text: text.to_string(), ..Default::default() }
    }

    /// The rows the search lists, best first (`recent` true for the picks of the session).
    fn hits(&mut self) -> &[usize] {
        self.statics();
        let q = self.text.trim().to_lowercase();
        if self.ranked.as_ref().is_none_or(|r| r.0 != q) {
            let list = if q.is_empty() {
                // the picks first, then the static rows (the harvested and the constants are listed when searched)
                let recent = self.recent_rows();
                let rest = (0..self.statics).filter(|i| !recent.contains(i) && !matches!(self.all[*i].run, Run::Const(_)));
                recent.iter().copied().chain(rest).collect()
            } else {
                let mut s: Vec<(i32, usize)> =
                    self.all.iter().enumerate().filter_map(|(i, e)| score(e, &q).map(|s| (s + if self.is_recent(e) { 25 } else { 0 } - i32::from(matches!(e.run, Run::Const(_))), i))).collect();
                s.sort_by_key(|&(s, i)| (-s, i));
                s.into_iter().take(MAX_HITS).map(|(_, i)| i).collect()
            };
            self.sel = 0;
            self.ranked = Some((q, list));
        }
        &self.ranked.as_ref().expect("ranked above").1
    }

    /// The picks of the session that are rows now, the last first.
    fn recent_rows(&self) -> Vec<usize> {
        self.recent.iter().filter_map(|(n, h)| self.all.iter().position(|e| e.name == *n && e.hint == *h)).collect()
    }

    /// Draw the box when it is open; the row run this pass (the box closes), with the note when it does not run now.
    pub(crate) fn show(&mut self, ctx: &Context, now: Now) -> Option<(Run, Option<&'static str>)> {
        if !self.open {
            return None;
        }
        let hits = self.hits().to_vec();
        let mut run = None;
        let n = hits.len();
        // the list keys, taken before the text field sees them
        let (down, up, pg_down, pg_up, enter) = ctx.input_mut(|i| {
            (
                i.consume_key(Modifiers::NONE, Key::ArrowDown),
                i.consume_key(Modifiers::NONE, Key::ArrowUp),
                i.consume_key(Modifiers::NONE, Key::PageDown),
                i.consume_key(Modifiers::NONE, Key::PageUp),
                i.consume_key(Modifiers::NONE, Key::Enter),
            )
        });
        let moved = down || up || pg_down || pg_up;
        if n > 0 {
            let page = LIST_ROWS as usize - 1;
            self.sel = match (down, up, pg_down, pg_up) {
                (true, ..) => (self.sel + 1) % n,
                (_, true, ..) => (self.sel + n - 1) % n,
                (_, _, true, _) => (self.sel + page).min(n - 1),
                (.., true) => self.sel.saturating_sub(page),
                _ => self.sel.min(n - 1),
            };
            if enter {
                run = Some(hits[self.sel]);
            }
        }
        let frame = egui::Frame::new()
            .fill(Color32::from_rgba_unmultiplied(16, 28, 45, 247))
            .stroke(egui::Stroke::new(1.0, theme::line(0.5)))
            .corner_radius(14)
            .shadow(egui::epaint::Shadow { offset: [0, 18], blur: 48, spread: 0, color: Color32::from_black_alpha(153) })
            .inner_margin(egui::Margin::same(12));
        let id = Id::unique("motorview-command");
        let top = ctx.content_rect().height() * 0.16;
        let area = egui::Modal::default_area(id).anchor(egui::Align2::CENTER_TOP, Vec2::new(0.0, top));
        let width = (ctx.content_rect().width() - 48.0).clamp(320.0, 640.0);
        let m = egui::Modal::new(id).area(area).frame(frame).backdrop_color(Color32::from_rgba_unmultiplied(2, 5, 10, 110)).show(ctx, |ui| {
            ui.set_width(width);
            let r = glass_ui::search(ui, &mut self.text, "search actions, keys, toolbar, dock pages and constants");
            r.request_focus();
            ui.add_space(8.0);
            if n == 0 {
                ui.label(RichText::new("nothing found").size(13.0).color(theme::DIM));
                return;
            }
            let n_recent = if self.text.trim().is_empty() { self.recent_rows().len() } else { 0 };
            let recent = n_recent > 0;
            egui::ScrollArea::vertical().max_height(glass_ui::ROW_H * LIST_ROWS + 30.0).auto_shrink([false, true]).show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 0.0;
                for (k, &i) in hits.iter().enumerate() {
                    if recent && (k == 0 || k == n_recent) {
                        let cap = if k == 0 { "RECENT" } else { "ALL" };
                        ui.add_space(if k == 0 { 0.0 } else { 6.0 });
                        ui.label(RichText::new(cap).size(10.0).color(theme::DIM));
                        ui.add_space(2.0);
                    }
                    let e = &self.all[i];
                    let dim = now.runs(e.ctx).is_err() || e.run == Run::Held;
                    let r = glass_ui::cmd_row(ui, e.icon, &e.name, &e.hint, &e.key, k == self.sel, dim);
                    if k == self.sel && moved {
                        r.scroll_to_me(None);
                    }
                    if r.clicked() {
                        run = Some(i);
                    }
                }
            });
            ui.add_space(6.0);
            ui.label(RichText::new("Up / Down pick · Enter run · Esc close").size(11.0).color(theme::DIM));
        });
        let picked = run.map(|i| {
            let e = &self.all[i];
            let id = (e.name.clone(), e.hint.clone());
            self.recent.retain(|r| *r != id);
            self.recent.push_front(id);
            self.recent.truncate(RECENT);
            self.ranked = None;
            let e = &self.all[i];
            (e.run.clone(), now.runs(e.ctx).err().or((e.run == Run::Held).then_some("a held key: hold it in the view")))
        });
        if picked.is_some() || m.should_close() {
            self.open = false;
        }
        if !self.open {
            self.close();
        }
        picked
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn find<'a>(all: &'a [Entry], q: &str) -> Vec<&'a str> {
        let mut s: Vec<(i32, usize)> = all.iter().enumerate().filter_map(|(i, e)| score(e, q).map(|s| (s, i))).collect();
        s.sort_by_key(|&(s, i)| (-s, i));
        s.into_iter().map(|(_, i)| all[i].name.as_str()).collect()
    }

    #[test]
    fn ranking() {
        let all = entries();
        // a name that starts with the word wins over one that holds it
        assert_eq!(find(&all, "fog")[0], "fog");
        // a key cap finds its action
        assert_eq!(find(&all, "f9")[0], "Classic / Remaster");
        // every word must match
        assert_eq!(find(&all, "track 4")[0], "track 4 Atlantika");
        assert!(find(&all, "zzzz qqqq").is_empty());
        // pages and toolbar groups are found by name
        assert!(find(&all, "constants page").contains(&"constants page"));
        assert!(find(&all, "anti").contains(&"Anti-aliasing"));
        // the exact name beats a longer one that starts with it
        assert_eq!(find(&all, "gravity")[0], "GRAVITY");
    }

    #[test]
    fn harvest_rows() {
        let h = |section: &str, label: &str, key: &str| Harvested { section: section.into(), label: label.into(), key: key.into() };
        let bar = [(RTab::Remaster, 3, h("", "Bloom", "")), (RTab::Remaster, 3, h("", "Bloom", "B"))];
        let dock = [(Page::Sim, h("Race (8 cars, 3 laps)", "", "")), (Page::Sim, h("Race (8 cars, 3 laps)", "Race clock", "")), (Page::Lamps, h("Head", "gain", "")), (Page::Lamps, h("Tail", "gain", ""))];
        let v = harvested(&bar, &dock);
        let names: Vec<(&str, &str)> = v.iter().map(|e| (e.name.as_str(), e.hint.as_str())).collect();
        // a row seen twice is listed once (the first); a section loses its live count
        assert_eq!(names, [("Bloom", "toolbar · Remaster / HDR"), ("Race", "dock · sim section"), ("Race clock", "dock · sim / Race"), ("gain", "dock · lamps / Head"), ("gain", "dock · lamps / Tail")]);
        assert!(v[1].run == Run::Dock(Page::Sim, "Race".into()) && v[0].run == Run::Toolbar(RTab::Remaster, 3));
    }

    #[test]
    fn contexts() {
        let race = Now { free: false, world: false };
        assert!(race.runs(Ctx::Any).is_ok() && race.runs(Ctx::Race).is_ok());
        assert!(race.runs(Ctx::Free).is_err() && race.runs(Ctx::World).is_err());
        assert!(Now { free: true, world: true }.runs(Ctx::World).is_ok());
    }

    #[test]
    fn helpers() {
        assert!(word_start("world: new hill piece", "hill") && !word_start("chill", "hill"));
        assert!(subsequence("colour grade", "clgr") && !subsequence("fog", "gof"));
    }
}
