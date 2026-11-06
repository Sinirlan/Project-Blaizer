//! DEBUG motorview's top bar and tool shelf (re-notes/00-debuger.md, look: 15-motorview-overhaul.md, concept A ·
//! Paddock of `reference_materials/ui-overhaul/motorview-overhaul/`, #395): every viewer option, by tab and group.
//! The top bar holds the brand, the tabs, the track and car pickers, the Classic / Remaster switch and the dock and
//! shelf buttons; under it the shelf is one fixed-height row of groups. A group's items are tools (on / off), value
//! tiles (choices and sliders, the value read without a click) and buttons; the ▾ of its caption opens a flyout with
//! all of the group's settings (`Look::Fly`), the ⧉ opens its page in the dock. An item with a key runs the same `Act`
//! as the key (its shortcut is in the hover text, from `BINDS`). Each group is one function drawn twice, as a shelf
//! row (`Look::Shelf`) and as the flyout's rows (`Look::Fly`); `It::hide` keeps an item out of the shelf. The game does
//! not use any of it.

use super::{key_name, Act, Ctx, DebugState, BINDS, HELD_FREE, HELD_RACE, TIME_SCALES};
use egui::emath::Numeric;
use egui::{Align, Context, Frame, Id, Layout, Rect, Response, RichText, ScrollArea, Sense, Ui, Vec2};
use motorview::cine_edit::{self, CineEdit, Preview, Req};
use motorview::drive::{CamMode, PoseCam, Session};
use motorview::ghost::{runs, Follow, Sync, ViewerGhost};
use motorview::gui::{wheel_drag, wheel_slider};
use motorview::icons as ic;
use motorview::inspect::{Passes, Tab};
use motorview::render::{aa_stop, CarShading, DebugShade, FogStyle, GrassLevel, LightShadows, PostAaKind, RemasterView, RenderAaKind, RenderMode, ShadowQuality, Tonemap, View, AA_STOPS, AA_STOP_STRENGTH};
use motorview::tune::{Page, Tune, CAM_MODES};
use motorview::wire::WireStyle;
use motorview::world_edit::{kind_icon, Cmd, Kind, WorldEdit};
use motorview::{glass_ui, theme};
use std::ops::RangeInclusive;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RTab {
    View,
    Sim,
    Render,
    Remaster,
    Debug,
    World,
    Ghost,
    Cine,
    Help,
}

impl RTab {
    pub const ALL: [RTab; 9] = [RTab::View, RTab::Sim, RTab::Render, RTab::Remaster, RTab::Debug, RTab::World, RTab::Ghost, RTab::Cine, RTab::Help];

    pub fn name(self) -> &'static str {
        match self {
            RTab::View => "View",
            RTab::Sim => "Sim",
            RTab::Render => "Render",
            RTab::Remaster => "Remaster",
            RTab::Debug => "Debug",
            RTab::World => "World",
            RTab::Ghost => "Ghost",
            RTab::Cine => "Cine",
            RTab::Help => "Help",
        }
    }

    fn icon(self) -> &'static str {
        match self {
            RTab::View => ic::EYE,
            RTab::Sim => ic::PLAY,
            RTab::Render => ic::MONITOR,
            RTab::Remaster => ic::SPARKLES,
            RTab::Debug => ic::BUG,
            RTab::World => ic::MOUNTAIN,
            RTab::Ghost => ic::GHOST,
            RTab::Cine => ic::FILM,
            RTab::Help => ic::HELP,
        }
    }

    pub fn parse(s: &str) -> Option<RTab> {
        let s = if s.eq_ignore_ascii_case("fill") { "world" } else { s };
        RTab::ALL.into_iter().find(|t| t.name().eq_ignore_ascii_case(s))
    }

    /// The tab `d` places on, wrapping.
    fn step(self, d: i32) -> RTab {
        let i = RTab::ALL.iter().position(|t| *t == self).unwrap_or(0) as i32;
        RTab::ALL[(i + d).rem_euclid(RTab::ALL.len() as i32) as usize]
    }
}

/// The game's own track and car names (`TrackName` / `CarName` of `Game/TrackN.trk` / `carNN.car`).
const TRACKS: [&str; 8] = ["RedRock", "NolbyHills", "NeoCity", "Atlantika", "OkkunSpeedway", "Olympos", "RuhrStadt", "Goldbridge"];
const CARS: [&str; 10] = ["Kobra X3", "Adder Mk2", "Python V12", "Serpent", "ASC II", "Avalanche", "ASC", "Resonic", "R-Build 2", "Titan"];

/// The ribbon's own state (kept by the viewer between frames).
pub struct RibbonState {
    pub tab: RTab,
    /// the shelf is hidden (`\`, double-click a tab); the top bar stays
    pub minimised: bool,
    /// Help > Shortcuts search text
    pub filter: String,
    /// the backdrop colour the picker starts from (gamma space)
    pub backdrop: [u8; 3],
    /// the open flyout: the group's index in the tab
    pub fly: Option<usize>,
    /// a top-bar dropdown held open (`--ribbon-pick`, headless shots only)
    pub pick: Option<Pick>,
}

/// One of the top bar's dropdowns.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Pick {
    Car,
    Track,
}

impl Pick {
    /// `--ribbon-pick car|track`.
    pub fn parse(s: &str) -> Option<Pick> {
        match s.to_ascii_lowercase().as_str() {
            "car" => Some(Pick::Car),
            "track" => Some(Pick::Track),
            _ => None,
        }
    }
}

impl Default for RibbonState {
    fn default() -> Self {
        RibbonState { tab: RTab::View, minimised: false, filter: String::new(), backdrop: [128; 3], fly: None, pick: None }
    }
}

impl RibbonState {
    /// The group of `tab` named `name` as the open flyout (`--ribbon-flyout`).
    pub fn open_flyout(&mut self, name: &str) -> bool {
        self.fly = groups(self.tab).iter().position(|g| g.name.eq_ignore_ascii_case(name));
        self.fly.is_some()
    }
}

/// What the ribbon shows and edits.
pub struct Ribbon<'a> {
    pub st: &'a mut RibbonState,
    pub tune: &'a mut Tune,
    pub view: &'a mut View,
    pub dbg: &'a mut DebugState,
    pub sess: Option<&'a mut Session>,
    pub free: bool,
    /// free camera: the arrows drive the followed car (`Act::DriveFree`)
    pub drive_free: bool,
    pub overlay: bool,
    pub cross: bool,
    /// Solid panels: opaque glass, no scene frost behind the panels (#395)
    pub solid_ui: bool,
    /// F4 wire style (the choice edits it)
    pub wireframe: &'a mut WireStyle,
    pub mirror: bool,
    pub track: u32,
    pub car: u32,
    /// the followed car drawn with the other model (`Act::SwapMesh`)
    pub mesh_swap: bool,
    /// world editor open
    pub world: Option<&'a mut WorldEdit>,
    /// the loaded ghost, and the ghost window's camera when it is open
    pub ghost: Option<&'a mut ViewerGhost>,
    pub ghost_win: Option<CamMode>,
    /// the camera a ghost window opens with
    pub ghost_cam: CamMode,
    /// what a new ghost window follows, and the window's (when open)
    pub ghost_follow: Follow,
    /// set to -1 / 1 while the Rewind / Forward button is held
    pub scrub: &'a mut i8,
    /// set to a timeline second when the timeline slider is moved (#60)
    pub seek: &'a mut Option<f64>,
    /// the Cine editor (#152)
    pub cine: Option<&'a mut CineEdit>,
    /// clicked items and their choice, run after the pass (`App::act`: a track switch needs
    /// the GPU, which the frame holds)
    pub acts: Vec<(Act, usize)>,
}

/// m:ss.s of a number of seconds.
pub fn clock(secs: f64) -> String {
    format!("{}:{:04.1}", (secs / 60.0) as u32, secs % 60.0)
}

// ---------------------------------------------------------------------------------------------
// Items: one description, drawn as a shelf tool / tile or as a flyout row
// ---------------------------------------------------------------------------------------------

/// How an item is drawn: on the shelf, or as a row of the group's flyout.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Look {
    Shelf,
    Fly,
}

/// One item: its icon, its full name (flyout label, hover text), its short shelf caption and its key.
struct It<'a> {
    icon: &'a str,
    name: &'a str,
    short: &'a str,
    act: Option<Act>,
    key: &'a str,
    /// the flyout only (`sh: hide`)
    hide: bool,
    /// the shelf tile's value when it is not the choice's name (`MSAA 16×`)
    tile: Option<String>,
}

impl<'a> It<'a> {
    fn new(icon: &'a str, name: &'a str, short: &'a str) -> It<'a> {
        It { icon, name, short, act: None, key: "", hide: false, tile: None }
    }

    fn act(mut self, a: Act) -> Self {
        self.act = Some(a);
        self
    }

    fn key(mut self, k: &'a str) -> Self {
        self.key = k;
        self
    }

    fn hide(mut self) -> Self {
        self.hide = true;
        self
    }

    fn tile(mut self, t: String) -> Self {
        self.tile = Some(t);
        self
    }

    /// The key text: the bound key of the item's `Act`, else the literal one.
    fn key_text(&self) -> &str {
        self.act.map(key_name).filter(|k| !k.is_empty()).unwrap_or(self.key)
    }
}

/// Hover text: the item's name and its key.
fn tip(it: &It, r: Response) -> Response {
    match it.key_text() {
        "" => r.on_hover_text(it.name),
        k => r.on_hover_text(format!("{}  [{k}]", it.name)),
    }
}

fn open_id() -> Id {
    Id::unique("ribbon::open")
}

fn flyopen_id() -> Id {
    Id::unique("ribbon::flyopen")
}

/// A value tile was clicked: open (or close) its group's flyout.
fn request_open(ui: &Ui) {
    ui.ctx().data_mut(|d| d.insert_temp(open_id(), true));
}

/// The group being drawn has its flyout open (its tiles get the orange rim).
fn fly_is_open(ui: &Ui) -> bool {
    ui.ctx().data(|d| d.get_temp::<bool>(flyopen_id())).unwrap_or(false)
}

/// A toggle: a lit tool on the shelf, a check row in the flyout. True when clicked.
fn tog(ui: &mut Ui, l: Look, it: &It, on: bool) -> bool {
    match l {
        Look::Shelf if it.hide => false,
        Look::Shelf => tip(it, glass_ui::tool(ui, it.icon, it.short, on)).clicked(),
        Look::Fly => glass_ui::row_check(ui, it.icon, it.name, it.key_text(), on),
    }
}

/// A toggle on a `bool` the ribbon edits directly.
fn flip(ui: &mut Ui, l: Look, it: &It, v: &mut bool) {
    if tog(ui, l, it, *v) {
        *v ^= true;
    }
}

/// A button: a tool on the shelf, an action row in the flyout. True when clicked.
fn press(ui: &mut Ui, l: Look, it: &It, enabled: bool) -> bool {
    match l {
        Look::Shelf if it.hide => false,
        Look::Shelf => ui.add_enabled_ui(enabled, |ui| tip(it, glass_ui::tool(ui, it.icon, it.short, false)).clicked()).inner,
        Look::Fly => ui.add_enabled_ui(enabled, |ui| glass_ui::row_action(ui, it.icon, it.name, it.key_text())).inner,
    }
}

/// A choice among `names`: a value tile on the shelf (click opens the flyout), ◀ value ▶ in the flyout. The new index
/// when a triangle was clicked.
fn pick_idx(ui: &mut Ui, l: Look, it: &It, names: &[&str], i: usize) -> Option<usize> {
    let i = i.min(names.len().saturating_sub(1));
    match l {
        Look::Shelf if it.hide => None,
        Look::Shelf => {
            let shown = it.tile.as_deref().unwrap_or(names[i]);
            if glass_ui::value_tile(ui, it.short, shown, true, None, fly_is_open(ui)).clicked() {
                request_open(ui);
            }
            None
        }
        Look::Fly => match glass_ui::row_choice(ui, it.icon, it.name, it.key_text(), names[i], i > 0, i + 1 < names.len()) {
            -1 => Some(i - 1),
            1 => Some(i + 1),
            _ => None,
        },
    }
}

/// A choice over `all` on a value the ribbon edits directly (`name` = the entry's text).
fn pick<T: Copy + PartialEq>(ui: &mut Ui, l: Look, it: &It, v: &mut T, all: &[T], name: impl Fn(T) -> String) {
    let names: Vec<String> = all.iter().map(|x| name(*x)).collect();
    let refs: Vec<&str> = names.iter().map(String::as_str).collect();
    let i = all.iter().position(|x| x == v).unwrap_or(0);
    if let Some(n) = pick_idx(ui, l, it, &refs, i) {
        *v = all[n];
    }
}

/// A slider on `v`: a tile with a meter on the shelf (click opens the flyout), a slider row in the flyout.
fn fslider<N: Numeric>(ui: &mut Ui, l: Look, it: &It, v: &mut N, range: RangeInclusive<N>, fmt: impl Fn(f64) -> String) {
    let (lo, hi) = (range.start().to_f64(), range.end().to_f64());
    match l {
        Look::Shelf if it.hide => {}
        Look::Shelf => {
            let f = if hi > lo { ((v.to_f64() - lo) / (hi - lo)) as f32 } else { 0.0 };
            if glass_ui::value_tile(ui, it.short, &fmt(v.to_f64()), false, Some(f), fly_is_open(ui)).clicked() {
                request_open(ui);
            }
        }
        Look::Fly => {
            let text = fmt(v.to_f64());
            glass_ui::row_ctl(ui, it.icon, it.name, it.key_text(), 190.0, |ui| {
                ui.add_sized([44.0, 20.0], egui::Label::new(RichText::new(text).font(theme::glass_font(14.0)).color(theme::WHITE)));
                ui.spacing_mut().slider_width = 120.0;
                wheel_slider(ui, v, range, |s| s.show_value(false));
            });
        }
    }
}

/// A number: a tile you drag sideways on the shelf, a drag field in the flyout.
fn fnum(ui: &mut Ui, l: Look, it: &It, v: &mut f32, speed: f64, fmt: impl Fn(f32) -> String) {
    match l {
        Look::Shelf if it.hide => {}
        Look::Shelf => {
            let r = glass_ui::value_tile(ui, it.short, &fmt(*v), false, None, false).on_hover_text(format!("{}: drag sideways", it.name)).on_hover_cursor(egui::CursorIcon::ResizeHorizontal);
            if r.dragged() {
                *v += r.drag_delta().x * speed as f32;
            }
        }
        Look::Fly => glass_ui::row_ctl(ui, it.icon, it.name, it.key_text(), 110.0, |ui| {
            ui.set_width(100.0);
            wheel_drag(ui, v, None, |d| d.speed(speed));
        }),
    }
}

/// A list of runtime entries (cameras, runs): the current one on a tile on the shelf; true in the flyout, where the
/// caller draws the entries as `glass_ui::row_item`s.
fn list_tile(ui: &mut Ui, l: Look, it: &It, shown: &str) -> bool {
    match l {
        Look::Shelf => {
            if !it.hide && glass_ui::value_tile(ui, it.short, shown, true, None, fly_is_open(ui)).clicked() {
                request_open(ui);
            }
            false
        }
        Look::Fly => true,
    }
}

/// One shelf group.
struct Group {
    name: &'static str,
    /// the dock page its ⧉ opens
    page: Option<Page>,
    /// has a ▾ flyout (a choice, a slider or hidden items)
    fly: bool,
    /// the group is on the shelf now
    shown: fn(&Ribbon<'_>) -> bool,
    body: fn(&mut Ribbon<'_>, &mut Ui, Look),
}

fn always(_: &Ribbon<'_>) -> bool {
    true
}

const fn g(name: &'static str, page: Option<Page>, fly: bool, body: fn(&mut Ribbon<'_>, &mut Ui, Look)) -> Group {
    Group { name, page, fly, shown: always, body }
}

const fn gif(name: &'static str, page: Option<Page>, fly: bool, shown: fn(&Ribbon<'_>) -> bool, body: fn(&mut Ribbon<'_>, &mut Ui, Look)) -> Group {
    Group { name, page, fly, shown, body }
}

fn world_is_open(s: &Ribbon<'_>) -> bool {
    s.world_open()
}

fn ghost_loaded(s: &Ribbon<'_>) -> bool {
    s.ghost.is_some()
}

fn cine_is_open(s: &Ribbon<'_>) -> bool {
    s.cine.as_ref().is_some_and(|c| c.active())
}

/// The groups of a tab (`TAB_GROUPS` of the mockup's `mv.js`).
/// The group bodies as plain functions (a `fn` pointer in `Group` must not be tied to one `Ribbon` lifetime).
macro_rules! bodies {
    ($($n:ident),* $(,)?) => { $( fn $n(s: &mut Ribbon<'_>, ui: &mut Ui, l: Look) { s.$n(ui, l) } )* };
}
bodies!(cine_cameras, cine_clock, cine_file, cine_preview, cine_render, dbg_gizmos, dbg_inspect, ghost_ghost, ghost_show, ghost_window, help_keys, render_debug, render_effects, render_light, rm_aa, rm_car, rm_fog, rm_grass, rm_hdr, rm_post, rm_shadows, rm_sky, rm_surface, sim_car, sim_physics, sim_playback, sim_race, sim_run_file, view_camera, view_capture, view_show, world_editor, world_file, world_generate, world_history, world_select, world_view);

fn groups(tab: RTab) -> &'static [Group] {
    const VIEW: [Group; 3] = [g("Camera", Some(Page::Camera), true, view_camera), g("Show", None, false, view_show), g("Capture", None, false, view_capture)];
    const SIM: [Group; 5] = [
        g("Playback", Some(Page::Sim), true, sim_playback),
        g("Race", None, true, sim_race),
        g("Run file", None, true, sim_run_file),
        g("Car", Some(Page::Car), true, sim_car),
        g("Physics", Some(Page::Car), false, sim_physics),
    ];
    const RENDER: [Group; 3] = [g("Effects", None, true, render_effects), g("Light", None, true, render_light), g("Debug view", None, true, render_debug)];
    const REMASTER: [Group; 9] = [
        g("Car", Some(Page::CarLights), true, rm_car),
        g("Surface", None, false, rm_surface),
        g("Anti-aliasing", None, true, rm_aa),
        g("HDR", None, true, rm_hdr),
        g("Post", None, true, rm_post),
        g("Fog", Some(Page::Fog), true, rm_fog),
        g("Sky & water", Some(Page::Sky), false, rm_sky),
        g("Grass", None, true, rm_grass),
        g("Shadows", Some(Page::Lamps), true, rm_shadows),
    ];
    const DEBUG: [Group; 2] = [g("Gizmos", None, true, dbg_gizmos), g("Inspect", None, true, dbg_inspect)];
    const WORLD: [Group; 6] = [
        g("Editor", Some(Page::World), false, world_editor),
        gif("File", None, false, world_is_open, world_file),
        gif("History", None, false, world_is_open, world_history),
        gif("Generate", None, false, world_is_open, world_generate),
        gif("Select", None, false, world_is_open, world_select),
        gif("View", None, false, world_is_open, world_view),
    ];
    const GHOST: [Group; 3] = [g("Ghost", Some(Page::Ghost), false, ghost_ghost), gif("Show", None, true, ghost_loaded, ghost_show), g("Ghost window", None, true, ghost_window)];
    const CINE: [Group; 5] = [
        g("File", Some(Page::Cine), false, cine_file),
        gif("Cameras", None, true, cine_is_open, cine_cameras),
        gif("Preview", None, false, cine_is_open, cine_preview),
        gif("Render", None, false, cine_is_open, cine_render),
        gif("Clock", Some(Page::Cine), false, cine_is_open, cine_clock),
    ];
    const HELP: [Group; 1] = [g("Keys", None, true, help_keys)];
    match tab {
        RTab::View => &VIEW,
        RTab::Sim => &SIM,
        RTab::Render => &RENDER,
        RTab::Remaster => &REMASTER,
        RTab::Debug => &DEBUG,
        RTab::World => &WORLD,
        RTab::Ghost => &GHOST,
        RTab::Cine => &CINE,
        RTab::Help => &HELP,
    }
}

impl Ribbon<'_> {
    /// The top bar and the tool shelf along the top edge; their height in points.
    pub fn bar(&mut self, ui: &mut Ui) -> f32 {
        let ctx = ui.ctx().clone();
        let width = ui.available_width();
        // a narrow window: only the open tab keeps its label, then the key cap goes
        let (compact, tiny) = (width < 1500.0, width < 1300.0);
        let mut h = 0.0;
        let top = egui::Panel::top("ribbon_bar").frame(Frame::NONE).show(ui, |ui| {
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing = Vec2::ZERO;
                ui.add_space(12.0);
                let inner = ui.available_width() - 12.0 - 2.0;
                let card = glass_ui::glass_frame(&ctx).show(ui, |ui| {
                    // a fixed 46 pt row, its items centred on it
                    let rect = Rect::from_min_size(ui.cursor().min, Vec2::new(inner, 46.0));
                    ui.scope_builder(egui::UiBuilder::new().max_rect(rect).layout(Layout::left_to_right(Align::Center)), |ui| {
                        ui.set_min_size(rect.size());
                        self.top_bar(ui, compact, tiny);
                    });
                });
                glass_ui::finish(&ctx, &card.response);
            });
            ui.add_space(if self.st.minimised { 10.0 } else { 8.0 });
        });
        h += top.response.rect.height();
        let mut region = top.response.rect;
        let tab = self.st.tab;
        let defs = groups(tab);
        let mut rects: Vec<Option<Rect>> = vec![None; defs.len()];
        let mut card_rect = Rect::NOTHING;
        if !self.st.minimised {
            let before = self.st.fly;
            let shelf = egui::Panel::top("ribbon_shelf").frame(Frame::NONE).show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing = Vec2::ZERO;
                    ui.add_space(12.0);
                    let inner = ui.available_width() - 12.0 - 2.0;
                    let card = glass_ui::glass_frame(&ctx).show(ui, |ui| {
                        ui.set_height(82.0);
                        ScrollArea::horizontal().id_salt(("shelf", tab as u8)).max_width(inner).auto_shrink([true, false]).show(ui, |ui| {
                            ui.horizontal_top(|ui| {
                                ui.spacing_mut().item_spacing = Vec2::ZERO;
                                let mut first = true;
                                for (gi, d) in defs.iter().enumerate() {
                                    if !(d.shown)(self) {
                                        continue;
                                    }
                                    let open = self.st.fly == Some(gi);
                                    ctx.data_mut(|dt| dt.insert_temp(flyopen_id(), open));
                                    let hit = glass_ui::shelf_group(ui, d.name, first, d.fly.then_some(open), d.page.is_some(), |ui| (d.body)(self, ui, Look::Shelf));
                                    first = false;
                                    rects[gi] = Some(hit.rect);
                                    let asked = ctx.data_mut(|dt| dt.remove_temp::<bool>(open_id())).unwrap_or(false);
                                    if hit.fly || asked {
                                        self.st.fly = if open { None } else { Some(gi) };
                                    }
                                    if hit.page {
                                        self.launch(d.page);
                                    }
                                }
                            });
                        });
                    });
                    glass_ui::finish(&ctx, &card.response);
                    card_rect = card.response.rect;
                });
                ui.add_space(10.0);
            });
            h += shelf.response.rect.height();
            region = region.union(shelf.response.rect);
            // a press on the view (outside the flyout and the shelf) closes the flyout
            if self.st.fly == before && before.is_some() {
                self.fly_outside_press(&ctx, card_rect);
            }
        }
        if let Some(gi) = self.st.fly {
            match (defs.get(gi), rects.get(gi).copied().flatten()) {
                (Some(d), Some(group)) => {
                    let (rect, close, page) = glass_ui::flyout(&ctx, group, card_rect.bottom() + 10.0, d.name, ic::SLIDERS, d.page.is_some(), |ui| (d.body)(self, ui, Look::Fly));
                    FLY_RECT.with(|c| c.set(rect));
                    if close {
                        self.st.fly = None;
                    }
                    if page {
                        self.launch(d.page);
                    }
                }
                _ => self.st.fly = None,
            }
        } else {
            FLY_RECT.with(|c| c.set(Rect::NOTHING));
        }
        // ← → switch tabs while the pointer is over the bar and no text field has the keys [design choice: the
        // arrows also look in the free camera and steer the car, so they are the bar's only while it is hovered]
        let hovering = ctx.input(|i| i.pointer.hover_pos()).is_some_and(|p| region.contains(p));
        if hovering && !self.drive_free && !ctx.egui_wants_keyboard_input() {
            let d = ctx.input(|i| i32::from(i.key_pressed(egui::Key::ArrowRight)) - i32::from(i.key_pressed(egui::Key::ArrowLeft)));
            if d != 0 {
                self.st.tab = self.st.tab.step(d);
                self.st.fly = None;
            }
        }
        h
    }

    /// A press outside the flyout and the shelf closes the flyout.
    fn fly_outside_press(&mut self, ctx: &Context, shelf: Rect) {
        let fly = FLY_RECT.with(std::cell::Cell::get);
        let pressed = ctx.input(|i| i.pointer.primary_pressed().then(|| i.pointer.interact_pos()).flatten());
        if pressed.is_some_and(|p| !fly.contains(p) && !shelf.contains(p)) {
            self.st.fly = None;
        }
    }

    /// Open a dock page from a group's ⧉.
    fn launch(&mut self, page: Option<Page>) {
        if let Some(p) = page {
            self.tune.page = Some(p);
            if p == Page::World && !self.world_open() {
                self.acts.push((Act::WorldToggle, 0));
            }
        }
    }

    /// The top bar's content: brand, tabs, pickers, mode switch, dock and shelf buttons.
    fn top_bar(&mut self, ui: &mut Ui, compact: bool, tiny: bool) {
        let unsaved = self.world.as_ref().is_some_and(|w| w.open && w.unsaved());
        {
            ui.spacing_mut().item_spacing = Vec2::ZERO;
            ui.add_space(16.0);
            // brand: Orbitron, the M orange (`.brand`; the mockup's 14 degree skew and letter-spacing are not drawn [APPROX])
            let orbitron = egui::FontId::new(15.0, egui::FontFamily::Name(theme::BRAND_FAMILY.into()));
            ui.label(RichText::new("M").font(orbitron.clone()).color(theme::ORANGE));
            ui.label(RichText::new("OTORVIEW").font(orbitron).color(theme::PLATE));
            if !tiny {
                ui.add_space(8.0);
                ui.label(RichText::new(motor_version::VERSION).font(theme::glass_font(11.0)).color(theme::VERSION));
            }
            ui.add_space(14.0);
            glass_ui::vsep(ui, 28.0);
            ui.add_space(2.0);
            for t in RTab::ALL {
                let on = self.st.tab == t;
                let label = (!compact || on).then(|| t.name());
                let r = glass_ui::tab(ui, t.icon(), label, on, t == RTab::World && unsaved, 46.0).on_hover_text(if self.st.minimised && on { "double-click: show the tools" } else { t.name() });
                if r.double_clicked() {
                    self.st.minimised ^= true;
                } else if r.clicked() {
                    if self.st.tab != t {
                        self.st.fly = None;
                    }
                    self.st.tab = t;
                    self.st.minimised = false;
                }
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                ui.add_space(8.0);
                let shelf_tip = format!("{} the tools  [{}]", if self.st.minimised { "show" } else { "hide" }, key_name(Act::MenuBar));
                if glass_ui::ibtn(ui, if self.st.minimised { ic::CHEVRON_DOWN } else { ic::CHEVRON_UP }, false).on_hover_text(shelf_tip).clicked() {
                    self.acts.push((Act::MenuBar, 0));
                }
                let dock_on = self.tune.page.is_some();
                if glass_ui::ibtn_shape(ui, dock_on, glass_ui::dock_icon).on_hover_text(format!("properties dock  [{}]", key_name(Act::Tune))).clicked() {
                    self.acts.push((Act::Tune, 0));
                }
                glass_ui::vsep(ui, 28.0);
                if !tiny {
                    glass_ui::kcap(ui, "F9", true);
                }
                let remaster = self.view.mode == RenderMode::Remaster;
                let opts = if tiny {
                    [glass_ui::SegOpt { icon: ic::IMAGE, text: "" }, glass_ui::SegOpt { icon: ic::SPARKLES, text: "" }]
                } else {
                    [glass_ui::SegOpt { icon: ic::IMAGE, text: "CLASSIC" }, glass_ui::SegOpt { icon: ic::SPARKLES, text: "REMASTER" }]
                };
                if let Some(i) = glass_ui::seg(ui, &opts, usize::from(remaster), None) {
                    if (i == 1) != remaster {
                        self.acts.push((Act::Mode, 0));
                    }
                }
                glass_ui::vsep(ui, 28.0);
                let car = (self.car.clamp(1, 10) - 1) as usize;
                if let Some(i) = glass_ui::dropdown(ui, ic::CAR, &CARS, car, 2, if tiny { 96.0 } else { 118.0 }, self.st.pick == Some(Pick::Car)) {
                    self.acts.push((Act::Car, i));
                }
                let track = (self.track.clamp(1, 8) - 1) as usize;
                if let Some(i) = glass_ui::dropdown(ui, ic::ROUTE, &TRACKS, track, 1, if tiny { 112.0 } else { 134.0 }, self.st.pick == Some(Pick::Track)) {
                    self.acts.push((Act::Track, i));
                }
            });
        }
    }

    fn world_open(&self) -> bool {
        self.world.as_ref().is_some_and(|w| w.open)
    }

    /// A toggle that runs `it.act` (its state is read from the viewer).
    fn tog_act(&mut self, ui: &mut Ui, l: Look, it: It, on: bool) {
        if tog(ui, l, &it, on) {
            self.acts.extend(it.act.map(|a| (a, 0)));
        }
    }

    /// A button that runs `it.act`.
    fn press_act(&mut self, ui: &mut Ui, l: Look, it: It, enabled: bool) {
        if press(ui, l, &it, enabled) {
            self.acts.extend(it.act.map(|a| (a, 0)));
        }
    }

    // ----- View -----

    fn view_camera(&mut self, ui: &mut Ui, l: Look) {
        self.tog_act(ui, l, It::new(ic::FREE_CAM, "Free cam", "Free cam").act(Act::FreeCam), self.free);
        self.press_act(ui, l, It::new(ic::RESET, "Reset camera", "Reset").act(Act::ResetCamera), true);
        self.press_act(ui, l, It::new(ic::NEXT_CAMERA, "Next camera", "Next cam").act(Act::Camera), true);
        self.tog_act(ui, l, It::new(ic::MIRROR, "Mirror", "Mirror").act(Act::Mirror), self.mirror);
        if let Some(ss) = self.sess.as_deref_mut() {
            let names: Vec<&str> = CAM_MODES.iter().map(|m| m.1).collect();
            let i = CAM_MODES.iter().position(|m| m.0 == ss.cam).unwrap_or(0);
            if let Some(n) = pick_idx(ui, l, &It::new(ic::VIDEO, "Race camera", "Race cam"), &names, i) {
                ss.cam = CAM_MODES[n].0;
            }
        }
    }

    fn view_show(&mut self, ui: &mut Ui, l: Look) {
        self.tog_act(ui, l, It::new(ic::GAUGE, "Telemetry", "Telemetry").act(Act::Overlay), self.overlay);
        self.tog_act(ui, l, It::new(ic::CROSSHAIR, "Target", "Target").act(Act::Cross), self.cross);
        self.tog_act(ui, l, It::new(ic::PANEL_TOP, "Toolbar", "Toolbar").act(Act::MenuBar), !self.st.minimised);
        self.tog_act(ui, l, It::new(ic::BLEND, "Solid panels (no blur behind the panels)", "Solid").act(Act::SolidUi), self.solid_ui);
    }

    fn view_capture(&mut self, ui: &mut Ui, l: Look) {
        self.press_act(ui, l, It::new(ic::CAMERA, "Screenshot", "Shot").act(Act::Screenshot), true);
        self.press_act(ui, l, It::new(ic::EXIT, "Exit", "Exit").act(Act::Escape), true);
    }

    // ----- Sim -----

    fn sim_playback(&mut self, ui: &mut Ui, l: Look) {
        let paused = self.dbg.paused;
        if l == Look::Shelf {
            // timeline (#60): the whole recorded run, drag to scrub; driving in the past takes over
            if let Some(span) = self.sess.as_ref().and_then(|x| x.timeline.as_ref()).map(|t| t.span()) {
                let left = format!("{} / {}", clock(span.1), clock(span.2));
                if let Some(t) = glass_ui::timeline(ui, span, &left, "rec") {
                    *self.seek = Some(t);
                }
            }
            // held buttons: the sim goes back / runs ahead while the button is down, then returns
            // to what it was doing (playing or paused)
            let back = glass_ui::tool(ui, ic::REWIND, "Rewind", false).on_hover_text("hold: run the race backwards  [, key]");
            self.tog_act(ui, l, It::new(if paused { ic::PLAY } else { ic::PAUSE }, "Pause", if paused { "Paused" } else { "Playing" }).act(Act::Pause), paused);
            self.press_act(ui, l, It::new(ic::STEP, "Step one frame", "Step").act(Act::Step), true);
            let fwd = glass_ui::tool(ui, ic::FORWARD, "Forward", false).on_hover_text("hold: run the race ahead at x4  [/ key]");
            if back.is_pointer_button_down_on() {
                *self.scrub = -1;
            } else if fwd.is_pointer_button_down_on() {
                *self.scrub = 1;
            }
        }
        let names: Vec<String> = TIME_SCALES.iter().map(|x| format!("x{x}")).collect();
        let refs: Vec<&str> = names.iter().map(String::as_str).collect();
        if let Some(n) = pick_idx(ui, l, &It::new(ic::CLOCK, "Time scale", "Speed").key("- ="), &refs, self.dbg.scale) {
            self.dbg.scale = n;
        }
        if l == Look::Fly {
            self.press_act(ui, l, It::new(ic::SLOWER, "Slower", "Slower").act(Act::Slower).hide(), true);
            self.press_act(ui, l, It::new(ic::FASTER, "Faster", "Faster").act(Act::Faster).hide(), true);
        }
    }

    fn sim_race(&mut self, ui: &mut Ui, l: Look) {
        self.press_act(ui, l, It::new(ic::RESTART, "Restart race", "Restart").act(Act::Restart), true);
        self.press_act(ui, l, It::new(ic::RESPAWN, "Respawn", "Respawn").act(Act::Respawn), true);
        self.press_act(ui, l, It::new(ic::FOCUS, "Follow next car", "Follow").act(Act::Focus), true);
        self.tog_act(ui, l, It::new(ic::CAR, "Drive in free cam", "Drive free").act(Act::DriveFree), self.drive_free);
        let names = ["Off", "Followed car", "Car + race"];
        if let Some(n) = pick_idx(ui, l, &It::new(ic::CHART, "Sim panel", "Sim panel").act(Act::Panel), &names, self.dbg.panel as usize % 3) {
            self.dbg.panel = n as u8;
        }
    }

    /// Repro files (#60): the recorded run saved as .mhr, or a game / viewer recording opened.
    fn sim_run_file(&mut self, ui: &mut Ui, l: Look) {
        let can = self.sess.as_ref().is_some_and(|x| x.setup.is_some() && x.timeline.as_ref().is_some_and(|t| t.whole()));
        self.press_act(ui, l, It::new(ic::SAVE, "Save run (.mhr to the Replays folder)", "Save run").act(Act::SaveRun), can);
        if list_tile(ui, l, &It::new(ic::UPLOAD, "Open run", "Open run"), "Replays…") {
            let runs = runs();
            if runs.is_empty() {
                ui.weak("no .mhr in the Replays folder of motorhead's settings directory");
            }
            for (i, e) in runs.iter().enumerate() {
                if glass_ui::row_item(ui, ic::FILM, &e.name, false) {
                    self.acts.push((Act::OpenRun, i));
                }
            }
        }
    }

    fn sim_car(&mut self, ui: &mut Ui, l: Look) {
        self.press_act(ui, l, It::new(ic::RELOAD, "Reload car recipes", "Reload").act(Act::ReloadCars), true);
        self.tog_act(ui, l, It::new(ic::BOX, "Swap mesh (original / generated)", "Swap mesh").act(Act::SwapMesh), self.mesh_swap);
        if l == Look::Fly {
            let opts: Vec<String> = (1..=10).map(|c| c.to_string()).collect();
            let opts: Vec<glass_ui::SegOpt> = opts.iter().map(|t| glass_ui::SegOpt { icon: "", text: t }).collect();
            let sel = (self.car.clamp(1, 10) - 1) as usize;
            let mut picked = None;
            glass_ui::row_ctl(ui, ic::CAR, "Car", key_name(Act::NextCar), 260.0, |ui| picked = glass_ui::seg(ui, &opts, sel, Some(22.0)));
            if let Some(i) = picked {
                self.acts.push((Act::Car, i));
            }
        }
    }

    fn sim_physics(&mut self, ui: &mut Ui, l: Look) {
        if let Some(ss) = self.sess.as_deref_mut() {
            fnum(ui, l, &It::new(ic::ORBIT, "Gravity", "Gravity"), &mut ss.race.world.gravity, 0.05, |v| format!("{v:.2}"));
        }
    }

    // ----- Render -----

    fn render_effects(&mut self, ui: &mut Ui, l: Look) {
        let v = &*self.view;
        let (fog, shadows, halos, line, torch) = (v.fog, v.shadows, v.halos, v.show_line, v.debug.flashlight);
        self.tog_act(ui, l, It::new(ic::FOG, "Fog", "Fog").act(Act::Fog), fog);
        self.tog_act(ui, l, It::new(ic::SHADOW, "Shadows", "Shadows").act(Act::Shadows), shadows);
        self.tog_act(ui, l, It::new(ic::SUN, "Halos", "Halos").act(Act::Halos), halos);
        self.tog_act(ui, l, It::new(ic::ROUTE, "Racing line", "Line").act(Act::RacingLine), line);
        self.tog_act(ui, l, It::new(ic::BULB, "Torch", "Torch").act(Act::Flashlight), torch);
        let v = &mut *self.view;
        flip(ui, l, &It::new(ic::ZAP, "Lens flares", "Flares"), &mut v.lens_flares);
        flip(ui, l, &It::new(ic::WIND, "Smoke", "Smoke"), &mut v.smoke);
        flip(ui, l, &It::new(ic::FLAME, "Sparks", "Sparks"), &mut v.sparks);
        flip(ui, l, &It::new(ic::CLOUD, "Sky image", "Sky"), &mut v.sky_image);
        flip(ui, l, &It::new(ic::BLEND, "Motion blur", "Blur").hide(), &mut v.motion_blur);
    }

    fn render_light(&mut self, ui: &mut Ui, l: Look) {
        fslider(ui, l, &It::new(ic::BULB, "Brightness", "Bright").key("[ ]"), &mut self.view.brightness, 0.1..=3.0, |v| format!("{v:.2}"));
        fslider(ui, l, &It::new(ic::BINOCULARS, "View distance", "Distance"), &mut self.view.distance, 0.25..=View::MAX_DISTANCE, |v| format!("{:.0}%", v * 100.0));
        fslider(ui, l, &It::new(ic::APERTURE, "FOV (horizontal °)", "FOV"), &mut self.view.fov_x, 40.0..=140.0, |v| format!("{v:.0}°"));
        pick(ui, l, &It::new(ic::CAR, "Car shading", "Car shade").act(Act::CarShading), &mut self.view.car_shading, &CarShading::ALL, |c| c.name().to_string());
    }

    fn render_debug(&mut self, ui: &mut Ui, l: Look) {
        pick(ui, l, &It::new(ic::GRID, "Wire style", "Wire").act(Act::Wireframe), self.wireframe, &WireStyle::ALL, |w| w.name().to_string());
        // a solid colour in place of the sky; the picker keeps its last colour while off
        let mut on = self.view.debug.backdrop.is_some();
        let it = It::new(ic::IMAGE, "Backdrop colour", "Backdrop");
        flip(ui, l, &it, &mut on);
        if on != self.view.debug.backdrop.is_some() {
            self.view.debug.backdrop = on.then_some(self.st.backdrop);
        }
        if let (Some(c), Look::Fly) = (&mut self.view.debug.backdrop, l) {
            glass_ui::row_ctl(ui, "", "Backdrop colour", "", 60.0, |ui| {
                egui::color_picker::color_edit_button_srgb(ui, c);
            });
            self.st.backdrop = *c;
        }
        self.tog_act(ui, l, It::new(ic::LAYERS, "Passes panel", "Passes").act(Act::RenderPanel), self.dbg.render_panel);
        pick(ui, l, &It::new(ic::PALETTE, "Shading view", "Shading").act(Act::Shade), &mut self.view.debug.shade, &DebugShade::ALL, |s| s.name().to_string());
        if l == Look::Fly {
            flip(ui, l, &It::new(ic::DISC, "Wheels", "Wheels"), &mut self.dbg.passes.wheels);
            for (i, name) in Passes::HELP.iter().enumerate() {
                if let Some(b) = self.dbg.passes.get(i) {
                    flip(ui, l, &It::new(ic::LIST, name, name), b);
                }
            }
            flip(ui, l, &It::new(ic::BINOCULARS, "Unlimited view distance", "Unlimited"), &mut self.view.debug.unlimited);
        }
    }

    // ----- Remaster -----

    fn rm_car(&mut self, ui: &mut Ui, l: Look) {
        let r = &self.view.remaster;
        let (paint, lights) = (r.enhanced_paint, r.dynamic_lights);
        self.tog_act(ui, l, It::new(ic::PALETTE, "Enhanced paint", "Paint").act(Act::Paint), paint);
        self.tog_act(ui, l, It::new(ic::BULB, "Dynamic lights", "Lights").act(Act::Lights), lights);
        let r = &mut self.view.remaster;
        flip(ui, l, &It::new(ic::SPARKLES, "Clear coat", "Coat"), &mut r.clear_coat);
        flip(ui, l, &It::new(ic::SHADOW, "Car shadows", "Shadows"), &mut r.car_shadows);
        flip(ui, l, &It::new(ic::CAR, "Generated cars", "Gen. cars").hide(), &mut r.generated_cars);
        let (lo, hi) = RemasterView::CAR_LOD_DISTANCE;
        ui.add_enabled_ui(r.generated_cars, |ui| fslider(ui, l, &It::new(ic::BINOCULARS, "Car detail distance", "Detail").hide(), &mut r.car_lod_distance, lo..=hi, pct));
    }

    fn rm_surface(&mut self, ui: &mut Ui, l: Look) {
        let r = &mut self.view.remaster;
        flip(ui, l, &It::new(ic::BULB, "Pixel light", "Px light"), &mut r.pixel_light);
        flip(ui, l, &It::new(ic::FOG, "Pixel fog", "Px fog"), &mut r.pixel_fog);
        flip(ui, l, &It::new(ic::FLAG, "Reflective signs", "Signs"), &mut r.retro_signs);
        flip(ui, l, &It::new(ic::SUN, "Smooth halos", "Halos"), &mut r.smooth_halos);
        flip(ui, l, &It::new(ic::ZAP, "Smooth flares", "Flares"), &mut r.smooth_flares);
    }

    fn rm_aa(&mut self, ui: &mut Ui, l: Look) {
        let r = &mut self.view.remaster;
        let tile = |name: &str, on: bool, strength: u32| if on { format!("{name} {}", AA_STOPS[aa_stop(strength)]) } else { name.to_string() };
        let t = tile(r.render_aa.kind.name(), r.render_aa.kind != RenderAaKind::Off, r.render_aa.strength);
        pick(ui, l, &It::new(ic::SCAN, "Render AA", "Render").tile(t), &mut r.render_aa.kind, &RenderAaKind::AVAILABLE, |k| k.name().to_string());
        aa_stop_pick(ui, l, "Render AA strength", &mut r.render_aa.strength, r.render_aa.kind != RenderAaKind::Off);
        let t = tile(r.post_aa.kind.name(), r.post_aa.kind != PostAaKind::Off, r.post_aa.strength);
        pick(ui, l, &It::new(ic::SCAN, "Post AA", "Post").act(Act::Msaa).tile(t), &mut r.post_aa.kind, &PostAaKind::AVAILABLE, |k| k.name().to_string());
        aa_stop_pick(ui, l, "Post AA strength", &mut r.post_aa.strength, r.post_aa.kind != PostAaKind::Off);
        pick(ui, l, &It::new(ic::IMAGE, "Anisotropy", "Aniso"), &mut r.anisotropy, &RemasterView::ANISOTROPY_LEVELS, |n| format!("{n}×"));
    }

    fn rm_hdr(&mut self, ui: &mut Ui, l: Look) {
        let r = &mut self.view.remaster;
        flip(ui, l, &It::new(ic::SUN, "HDR", "HDR"), &mut r.hdr);
        pick(ui, l, &It::new(ic::PALETTE, "Tonemap", "Tonemap"), &mut r.tonemap, &Tonemap::ALL, |m| m.name().to_string());
        let (lo, hi) = RemasterView::EXPOSURE_RANGE;
        fslider(ui, l, &It::new(ic::APERTURE, "Exposure", "Exposure"), &mut r.exposure, lo..=hi, pct);
        fslider(ui, l, &It::new(ic::SPARKLES, "Bloom", "Bloom"), &mut r.bloom, 0..=100, pct);
    }

    fn rm_post(&mut self, ui: &mut Ui, l: Look) {
        let grade = self.view.remaster.post();
        self.tog_act(ui, l, It::new(ic::PALETTE, "Colour grade", "Grade").act(Act::Grade), grade);
        let r = &mut self.view.remaster;
        flip(ui, l, &It::new(ic::BLEND, "Velocity blur", "Vel. blur"), &mut r.velocity_blur);
        fslider(ui, l, &It::new(ic::CIRCLE_DOT, "Ambient occlusion", "AO"), &mut r.ssao, 0..=100, pct);
        fslider(ui, l, &It::new(ic::PALETTE, "Grade strength", "Grade").hide(), &mut r.grade, 0..=100, pct);
        fslider(ui, l, &It::new(ic::APERTURE, "Vignette", "Vignette").hide(), &mut r.vignette, 0..=100, pct);
        fslider(ui, l, &It::new(ic::GRID, "Grain", "Grain").hide(), &mut r.grain, 0..=100, pct);
    }

    fn rm_fog(&mut self, ui: &mut Ui, l: Look) {
        pick(ui, l, &It::new(ic::FOG, "Fog style", "Style"), &mut self.view.remaster.fog_style, &FogStyle::ALL, |s| s.name().to_string());
    }

    fn rm_sky(&mut self, ui: &mut Ui, l: Look) {
        let r = &mut self.view.remaster;
        flip(ui, l, &It::new(ic::CLOUD, "HDRI sky", "HDRI"), &mut r.sky_hdri);
        flip(ui, l, &It::new(ic::WAVES, "Water", "Water"), &mut r.water);
        flip(ui, l, &It::new(ic::WAVES, "Water reflections (SSR)", "SSR"), &mut r.water_ssr);
        flip(ui, l, &It::new(ic::SUN, "Local light", "Local"), &mut r.local_light);
    }

    fn rm_grass(&mut self, ui: &mut Ui, l: Look) {
        let r = &mut self.view.remaster;
        pick(ui, l, &It::new(ic::TREE, "Grass", "Grass"), &mut r.grass, &GrassLevel::ALL, |g| g.name().to_string());
        let (lo, hi) = RemasterView::GRASS_RANGE;
        ui.add_enabled_ui(r.grass != GrassLevel::Off, |ui| fslider(ui, l, &It::new(ic::BINOCULARS, "Grass range", "Range").hide(), &mut r.grass_range, lo..=hi, pct));
    }

    fn rm_shadows(&mut self, ui: &mut Ui, l: Look) {
        let r = &mut self.view.remaster;
        pick(ui, l, &It::new(ic::SHADOW, "Light shadows", "Lights"), &mut r.light_shadows, &LightShadows::ALL, |m| m.name().to_string());
        pick(ui, l, &It::new(ic::SLIDERS, "Shadow quality", "Quality"), &mut r.shadow_quality, &ShadowQuality::ALL, |q| q.name().to_string());
    }

    // ----- Debug -----

    fn dbg_gizmos(&mut self, ui: &mut Ui, l: Look) {
        let ly = self.dbg.layers;
        self.tog_act(ui, l, It::new(ic::ROUTE, "Track", "Track").act(Act::TrackGizmos), ly.track);
        self.tog_act(ui, l, It::new(ic::GRID, "Collision", "Collision").act(Act::ColGizmos), ly.collision);
        self.tog_act(ui, l, It::new(ic::BOX, "Bounds", "Bounds").act(Act::BoundGizmos), ly.bounds);
        self.tog_act(ui, l, It::new(ic::BULB, "Lights", "Lights").act(Act::LightGizmos), ly.lights);
        self.tog_act(ui, l, It::new(ic::CIRCLE_DOT, "Probes", "Probes").act(Act::ProbeGizmos), ly.probes);
        let ly = &mut self.dbg.layers;
        let states = [(false, false), (true, false), (true, true)];
        let names = ["No car", "Followed car", "All cars"];
        let i = states.iter().position(|s| *s == (ly.car, ly.all_cars)).unwrap_or(0);
        if let Some(n) = pick_idx(ui, l, &It::new(ic::CAR, "Car gizmos", "Car").act(Act::CarGizmos), &names, i) {
            (ly.car, ly.all_cars) = states[n];
        }
    }

    fn dbg_inspect(&mut self, ui: &mut Ui, l: Look) {
        self.press_act(ui, l, It::new(ic::SEARCH, "Inspect target", "Inspect").act(Act::Inspect), true);
        self.press_act(ui, l, It::new(ic::RESPAWN, "Jump to", "Jump to").act(Act::Jump), true);
        let b = &mut self.dbg.browser;
        let tabs = [(None, "Closed"), (Some(Tab::Textures), "World textures"), (Some(Tab::Props), "Scene objects"), (Some(Tab::Cars), "Cars")];
        let names: Vec<&str> = tabs.iter().map(|t| t.1).collect();
        let i = tabs.iter().position(|t| t.0 == b.tab).unwrap_or(0);
        if let Some(n) = pick_idx(ui, l, &It::new(ic::LIST, "Data browser", "Browser").act(Act::Browse), &names, i) {
            b.tab = tabs[n].0;
            (b.sel, b.jumps) = (0, 0);
        }
        self.press_act(ui, l, It::new(ic::REWIND, "Browser: first", "First").act(Act::BrowseFirst).hide(), true);
        self.press_act(ui, l, It::new(ic::CHEVRON_UP, "Browser: previous", "Previous").act(Act::BrowseUp).hide(), true);
        self.press_act(ui, l, It::new(ic::CHEVRON_DOWN, "Browser: next", "Next").act(Act::BrowseDown).hide(), true);
        self.press_act(ui, l, It::new(ic::FORWARD, "Browser: last", "Last").act(Act::BrowseLast).hide(), true);
    }

    // ----- World -----

    fn world_editor(&mut self, ui: &mut Ui, l: Look) {
        let open = self.world_open();
        self.tog_act(ui, l, It::new(ic::MOUNTAIN, "World editor", "Editor").act(Act::WorldToggle), open);
    }

    fn world_file(&mut self, ui: &mut Ui, l: Look) {
        let unsaved = self.world.as_ref().is_some_and(|w| w.unsaved());
        self.press_act(ui, l, It::new(ic::SAVE, "Save", if unsaved { "Save *" } else { "Save" }).act(Act::World(Cmd::Save)), true);
        self.press_act(ui, l, It::new(ic::RELOAD, "Reload", "Reload").act(Act::World(Cmd::Reload)), true);
    }

    fn world_history(&mut self, ui: &mut Ui, l: Look) {
        let (undo, redo) = self.world.as_ref().map_or((false, false), |w| (w.can_undo(), w.can_redo()));
        self.press_act(ui, l, It::new(ic::UNDO, "Undo", "Undo").act(Act::World(Cmd::Undo)), undo);
        self.press_act(ui, l, It::new(ic::REDO, "Redo", "Redo").act(Act::World(Cmd::Redo)), redo);
    }

    fn world_generate(&mut self, ui: &mut Ui, l: Look) {
        self.press_act(ui, l, It::new(ic::WAND, "Auto fill", "Auto fill").act(Act::World(Cmd::Generate)), true);
    }

    fn world_select(&mut self, ui: &mut Ui, l: Look) {
        self.press_act(ui, l, It::new(ic::REWIND, "Previous piece", "Previous").act(Act::World(Cmd::Step(-1))), true);
        self.press_act(ui, l, It::new(ic::FORWARD, "Next piece", "Next").act(Act::World(Cmd::Step(1))), true);
    }

    fn world_view(&mut self, ui: &mut Ui, l: Look) {
        let holes = self.world.as_ref().is_some_and(|w| w.holes);
        self.tog_act(ui, l, It::new(ic::SEARCH, "Holes", "Holes").act(Act::World(Cmd::Holes)), holes);
        let Some(w) = self.world.as_deref_mut() else { return };
        fnum(ui, l, &It::new(ic::MOVE, "Work height", "Height"), &mut w.work_y, 0.5, |v| format!("{v:.1}"));
        // layer visibility: one cell per kind, the last shows or hides all
        match l {
            Look::Shelf => {
                ui.add_space(4.0);
                let (area, _) = ui.allocate_exact_size(Vec2::new(4.0 * 26.0 + 3.0 * 3.0, 2.0 * 24.0 + 3.0), Sense::hover());
                let kinds: Vec<Kind> = Kind::ALL.into_iter().filter(|k| *k != Kind::Water).collect();
                let cell = |i: usize| Rect::from_min_size(area.min + Vec2::new((i % 4) as f32 * 29.0, (i / 4) as f32 * 27.0), Vec2::new(26.0, 24.0));
                for (i, k) in kinds.iter().enumerate() {
                    let on = !w.hidden[*k as usize];
                    if glass_ui::layer_cell(ui, cell(i), i, kind_icon(*k), on).on_hover_text(format!("{} layer", k.name())).clicked() {
                        w.hidden[*k as usize] = on;
                    }
                }
                let all = kinds.iter().all(|k| !w.hidden[*k as usize]);
                if glass_ui::layer_cell(ui, cell(kinds.len()), kinds.len(), ic::LAYERS, all).on_hover_text("all layers").clicked() {
                    for k in &kinds {
                        w.hidden[*k as usize] = all;
                    }
                }
            }
            Look::Fly => {}
        }
    }

    // ----- Ghost -----

    fn ghost_ghost(&mut self, ui: &mut Ui, l: Look) {
        let has = self.ghost.is_some();
        if press(ui, l, &It::new(ic::UPLOAD, "Load ghost… (pick a .ghl or .mhr in the dock)", "Load"), true) {
            self.tune.page = Some(Page::Ghost);
        }
        self.press_act(ui, l, It::new(ic::TRASH, "Clear", "Clear").act(Act::GhostClear), has);
        let open = self.ghost_win.is_some();
        self.tog_act(ui, l, It::new(ic::APP, if open { "Close ghost window" } else { "Open ghost window" }, "Window").act(Act::GhostWindow), open);
        if l == Look::Shelf {
            let text = self.ghost.as_deref().map_or("no ghost".to_string(), |g| format!("{} (car {})", g.name, g.car));
            glass_ui::value_tile(ui, "ghost", &text, false, None, false);
        }
    }

    fn ghost_show(&mut self, ui: &mut Ui, l: Look) {
        if let Some(g) = self.ghost.as_deref_mut() {
            flip(ui, l, &It::new(ic::EYE, "In the main view", "Main view"), &mut g.show_main);
            pick(ui, l, &It::new(ic::CLOCK, "Clock", "Clock"), &mut g.sync, &Sync::ALL, |s| s.name().to_string());
        }
    }

    fn ghost_window(&mut self, ui: &mut Ui, l: Look) {
        let has_ghost = self.ghost.is_some();
        let cur = self.ghost_follow;
        let name = |f: Follow, sess: Option<&Session>| match f {
            Follow::Ghost => "the ghost".to_string(),
            Follow::Car(i) => sess.and_then(|ss| ss.race.racers.get(i)).map_or("?".to_string(), |r| format!("#{i} {}", r.driver.name)),
        };
        let shown = name(cur, self.sess.as_deref());
        if list_tile(ui, l, &It::new(ic::FOCUS, "Follow", "Follow"), &shown) {
            if glass_ui::row_item(ui, ic::GHOST, "the ghost", cur == Follow::Ghost) && has_ghost {
                self.acts.push((Act::GhostFollow, 0));
            }
            for i in 0..self.sess.as_deref().map_or(0, |ss| ss.race.racers.len()) {
                if glass_ui::row_item(ui, ic::CAR, &name(Follow::Car(i), self.sess.as_deref()), cur == Follow::Car(i)) {
                    self.acts.push((Act::GhostFollow, i + 1));
                }
            }
        }
        let open = self.ghost_win.is_some();
        let cur = self.ghost_win.unwrap_or(self.ghost_cam);
        let names: Vec<&str> = PoseCam::MODES.iter().map(|m| CAM_MODES.iter().find(|c| c.0 == *m).map_or("?", |c| c.1)).collect();
        let i = PoseCam::MODES.iter().position(|m| *m == cur).unwrap_or(0);
        if let Some(n) = pick_idx(ui, l, &It::new(ic::VIDEO, "Camera", "Camera"), &names, i) {
            self.acts.push((Act::GhostCamSet, n));
        }
        self.press_act(ui, l, It::new(ic::NEXT_CAMERA, "Next camera", "Next cam").act(Act::GhostCam), open);
    }

    // ----- Cine (cine_edit.rs, #152) -----

    fn cine_file(&mut self, ui: &mut Ui, l: Look) {
        let Some(ed) = self.cine.as_deref_mut() else { return };
        let open = ed.active();
        if press(ui, l, &It::new(ic::FILM, "New sequence (on this track, its first camera where the free camera is; the race becomes all AI)", "New"), true) {
            ed.reqs.push(Req::New);
        }
        if press(ui, l, &It::new(ic::UPLOAD, "Open… (pick a .cine in the dock)", "Open…"), true) {
            self.tune.page = Some(Page::Cine);
        }
        if press(ui, l, &It::new(ic::SAVE, "Save", "Save"), open) {
            ed.msg = match ed.save(None) {
                Ok(p) => format!("saved {}", p.display()),
                Err(e) => e,
            };
        }
        if press(ui, l, &It::new(ic::CLOSE, "Close (leave the editor, the race stays)", "Close"), open) {
            ed.close();
        }
        if l == Look::Shelf {
            let name = match &ed.path {
                Some(p) => format!("{}{}", p.file_name().map_or_else(String::new, |n| n.to_string_lossy().into_owned()), if ed.dirty { " *" } else { "" }),
                None if open => (if ed.dirty { "unsaved *" } else { "unsaved" }).to_string(),
                None => "no sequence".to_string(),
            };
            glass_ui::value_tile(ui, "file", &name, false, None, false);
        }
    }

    fn cine_cameras(&mut self, ui: &mut Ui, l: Look) {
        let Some(ed) = self.cine.as_deref_mut() else { return };
        let ncars = ed.seq.as_ref().map_or(0, |q| q.race.cars.len());
        if press(ui, l, &It::new(ic::CAMERA, "Camera here", "Add here").act(Act::CineAddCam), true) {
            self.acts.push((Act::CineAddCam, 0));
        }
        if press(ui, l, &It::new(ic::CAR, "Mounted camera (on the followed car, where the free camera is)", "Mounted"), true) {
            ed.reqs.push(Req::AddMounted);
        }
        if press(ui, l, &It::new(ic::SPLINE, "Path camera (its first key at the free camera)", "Path"), true) {
            ed.reqs.push(Req::AddPath);
        }
        if press(ui, l, &It::new(ic::ROUTE, "Path key here", "Add key").act(Act::CineAddKey), true) {
            self.acts.push((Act::CineAddKey, 0));
        }
        let many = ed.seq.as_ref().is_some_and(|q| q.cameras.len() > 1);
        if press(ui, l, &It::new(ic::TRASH, "Remove the selected camera and its shots", "Remove"), many) {
            ed.remove_selected();
        }
        if let Some(seq) = ed.seq.as_ref() {
            let sel = ed.sel.min(seq.cameras.len() - 1);
            let mut pick = sel;
            if list_tile(ui, l, &It::new(ic::VIDEO, "Camera", "Camera"), &seq.cameras[sel].name) {
                for (i, c) in seq.cameras.iter().enumerate() {
                    if glass_ui::row_item(ui, ic::VIDEO, &c.name, i == sel) {
                        pick = i;
                    }
                }
            }
            ed.sel = pick;
        }
        if l == Look::Fly {
            let names = cine_edit::names(self.sess.as_deref());
            glass_ui::row_ctl(ui, ic::FOCUS, "New cameras follow", "", 190.0, |ui| {
                cine_edit::target_combo(ui, "cine-new-target", &mut ed.target, true, ncars, &names);
            });
        }
    }

    fn cine_preview(&mut self, ui: &mut Ui, l: Look) {
        let Some(ed) = self.cine.as_deref_mut() else { return };
        if press(ui, l, &It::new(ic::VIDEO, "Cut here", "Cut").act(Act::CineCut), true) {
            ed.reqs.push(Req::Cut);
        }
        let names: Vec<&str> = Preview::ALL.iter().map(|p| p.name()).collect();
        let i = Preview::ALL.iter().position(|p| *p == ed.preview).unwrap_or(0);
        if l == Look::Shelf {
            let opts: Vec<glass_ui::SegOpt> = names.iter().map(|t| glass_ui::SegOpt { icon: "", text: t }).collect();
            if let Some(n) = glass_ui::seg(ui, &opts, i, None) {
                ed.preview = Preview::ALL[n];
            }
            if let (Some(seq), Some(air)) = (ed.seq.as_ref(), ed.on_air()) {
                glass_ui::value_tile(ui, "on air", &seq.cameras[air].name, false, None, false);
            }
        }
    }

    fn cine_render(&mut self, ui: &mut Ui, l: Look) {
        let Some(ed) = self.cine.as_deref_mut() else { return };
        match (ed.job.as_ref(), ed.render_path()) {
            (None, Some(out)) => {
                let name = out.file_name().map_or_else(String::new, |n| n.to_string_lossy().into_owned());
                if press(ui, l, &It::new(ic::FILM, "Render the whole sequence as it is here, in the background, with these graphics options (the status bar shows the progress)", "Render"), true) {
                    ed.reqs.push(Req::Render);
                }
                if l == Look::Shelf {
                    glass_ui::value_tile(ui, "to", &name, false, None, false);
                }
            }
            (None, None) => {
                if l == Look::Shelf {
                    glass_ui::value_tile(ui, "render", "no file", false, None, false);
                }
            }
            (Some(j), _) => {
                if press(ui, l, &It::new(ic::CLOSE, "Cancel the render (the frames so far stay in the video)", "Cancel"), true) {
                    j.watch.cancel.store(true, std::sync::atomic::Ordering::Relaxed);
                }
                if let (Some((text, _, _)), Look::Shelf) = (ed.job_status(), l) {
                    glass_ui::value_tile(ui, "render", &text, false, None, false);
                }
            }
        }
    }

    fn cine_clock(&mut self, ui: &mut Ui, l: Look) {
        let paused = self.dbg.paused;
        self.tog_act(ui, l, It::new(if paused { ic::PLAY } else { ic::PAUSE }, "Pause", if paused { "Paused" } else { "Playing" }).act(Act::Pause), paused);
        let Some(ed) = self.cine.as_deref_mut() else { return };
        if press(ui, l, &It::new(ic::RESTART, "Start (the clock to 0)", "Start"), true) {
            ed.seek(0.0);
        }
        if press(ui, l, &It::new(ic::REWIND, "Previous shot", "Prev shot"), true) {
            ed.shot_step(-1);
        }
        if press(ui, l, &It::new(ic::FORWARD, "Next shot", "Next shot"), true) {
            ed.shot_step(1);
        }
        if l == Look::Shelf {
            let end = ed.slider_end(self.sess.as_deref());
            let race = ed.race_secs();
            let behind = if ed.behind { " · ahead" } else { "" };
            let left = format!("{} / {}", clock(ed.t as f64), clock(end as f64));
            if let Some(t) = glass_ui::timeline(ui, (0.0, ed.t as f64, end.max(0.01) as f64), &left, &format!("race {}{behind}", clock(race as f64))) {
                ed.seek(t as f32);
            }
        }
    }

    // ----- Help -----

    fn help_keys(&mut self, ui: &mut Ui, l: Look) {
        match l {
            Look::Shelf => {
                if glass_ui::tool(ui, ic::KEYBOARD, "Shortcuts", false).on_hover_text("every key and its action").clicked() {
                    request_open(ui);
                }
            }
            Look::Fly => {
                for (title, keys) in [("free camera", HELD_FREE), ("race camera", HELD_RACE)] {
                    ui.add_space(4.0);
                    ui.label(RichText::new(title.to_uppercase()).size(10.0).color(theme::DIM));
                    for (k, what) in keys {
                        key_line(ui, k, what);
                    }
                }
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    ui.label(RichText::new(ic::SEARCH).color(theme::DIM));
                    ui.add(egui::TextEdit::singleline(&mut self.st.filter).hint_text("filter keys and actions").desired_width(250.0));
                    if ui.small_button(ic::CLOSE).clicked() {
                        self.st.filter.clear();
                    }
                });
                let f = self.st.filter.to_lowercase();
                ScrollArea::vertical().max_height(300.0).show(ui, |ui| {
                    for (title, ctx) in [("any", Ctx::Any), ("race camera", Ctx::Race), ("free camera", Ctx::Free), ("world editor", Ctx::World)] {
                        let mut head = false;
                        for b in BINDS.iter().filter(|b| b.ctx == ctx && (f.is_empty() || b.label().to_lowercase().contains(&f) || b.name.to_lowercase().contains(&f) || title.contains(&f))) {
                            if !head {
                                ui.add_space(6.0);
                                ui.label(RichText::new(title.to_uppercase()).size(10.0).color(theme::DIM));
                                head = true;
                            }
                            key_line(ui, b.name, b.label());
                        }
                    }
                });
            }
        }
    }
}

thread_local! {
    /// the flyout's rect of the last pass (a press outside it closes it)
    static FLY_RECT: std::cell::Cell<Rect> = const { std::cell::Cell::new(Rect::NOTHING) };
}

/// A key and what it does, one line of the shortcut list.
fn key_line(ui: &mut Ui, key: &str, what: &str) {
    ui.horizontal(|ui| {
        ui.set_height(22.0);
        ui.add_sized([110.0, 18.0], egui::Label::new(RichText::new(key).font(egui::FontId::monospace(12.0)).color(theme::WHITE)).sense(Sense::hover()));
        ui.label(RichText::new(what).color(theme::TEXT));
    });
}

/// A percent.
fn pct<N: Numeric>(v: N) -> String {
    format!("{}%", v.to_f64().round())
}

/// the strength of an AA group as a stop of the shared 2x .. 16x scale; hidden on the shelf, and while its method is Off
fn aa_stop_pick(ui: &mut Ui, l: Look, name: &str, strength: &mut u32, on: bool) {
    if on {
        let i = aa_stop(*strength);
        if let Some(n) = pick_idx(ui, l, &It::new(ic::SCAN, name, "Strength").hide(), &AA_STOPS, i) {
            *strength = AA_STOP_STRENGTH[n];
        }
    }
}

/// Where the tool shelf's flyout is this pass (Rect::NOTHING when none is open): the telemetry card keeps out of it.
pub fn fly_rect() -> Rect {
    FLY_RECT.with(std::cell::Cell::get)
}
