//! REMASTER HUD layout editor (#542 phase 2; design choice after WoW's Edit Mode): the race frozen under a light veil,
//! the Race line HUD on it with a sample of every piece that comes and goes (`hud::draw_rects` in `HudMode::Edit`), a
//! dashed box per piece, and a Glass toolbar card docked at the bottom (it moves to the top while the selected piece is
//! under it). Keys and pad: Q / E pick a piece, the arrows move it (faster while held, Shift 10 px), Z / C scale it,
//! H shows or hides it, R puts it back (held: every piece), G switches snapping, Enter keeps the layout, Esc puts everything
//! back. The mouse (#542 phase 3): click a piece to pick it, drag it to move it (snapping to the screen edges, the centre
//! lines and an 8 px grid; Shift drags freely), pull the handle on the selected box's corner to resize it, the wheel
//! resizes the selected piece, and every control of the card is clickable. The layout model is `hud_layout`; the first
//! change freezes the stock fit into the player's own layout (`hud::freeze`).

use super::pause_settings::{glass_panel, PauseVal};
use super::*;
use crate::hud::{self, HudMode};
use winit::window::CursorIcon;
use crate::hud_layout::{Anchor, ElemLayout, HudElem, HudLayout, HudRects, HudSettings, Rect, ELEM_SCALE};

/// One press of +/- scales by this much (percent of the piece's size).
const SCALE_STEP: u32 = 5;
/// Canvas px an arrow moves the piece: a lone press the first, each quick repeat of the same arrow (OS key repeat, the
/// pad's held d-pad) steps along the list every `NUDGE_EVERY` repeats; Shift moves `NUDGE_SHIFT`.
const NUDGE: [i32; 5] = [1, 2, 4, 8, 16];
const NUDGE_EVERY: usize = 3;
const NUDGE_SHIFT: i32 = 10;
/// Seconds within which a press of the same arrow counts as a repeat.
const NUDGE_REPEAT: f32 = 0.25;
/// The toolbar card (design px): width, height, its bottom when docked, its top when it moves up. It carries its own
/// keys (no hint row), so at 4:3 nothing else runs into the splits block.
const BAR_W: f32 = 620.0;
const BAR_H: f32 = 128.0;
const BAR_BOTTOM: f32 = 696.0;
const BAR_TOP_UP: f32 = 96.0;
/// The veil over the race: navy, alpha at the middle and at the edges (a quarter of the pause menu's, so the boxes read
/// and the race still looks as it does in play).
const VEIL: (f32, f32) = (0.12, 0.22);
/// The hint kind of SHOW: H on the keyboard, Y / triangle on a pad (`glyph_for`).
pub(super) const KIND_SHOW: usize = 18;
/// The hint kind of SNAP: G on the keyboard (a pad has no use for it: snapping is for the mouse).
pub(super) const KIND_SNAP: usize = 19;
/// Mouse: a press that moves less than this (canvas px) is a click, not a drag (it must not turn a stock layout custom).
const MOVE_SLOP: f32 = 3.0;
/// Snapping: a box edge or centre this near a screen edge or centre line (canvas px) jumps to it; elsewhere the box's top
/// left corner steps on a `GRID` px grid. Resizing steps `SNAP_SCALE` percent.
const SNAP_PX: f32 = 8.0;
const GRID: f32 = 8.0;
const SNAP_SCALE: u32 = 5;
/// The resize handle (design px): its drawn side and the radius in which it takes the pointer.
const HANDLE: f32 = 9.0;
const HANDLE_HIT: f32 = 11.0;
/// The card's controls (design px: x, y, w, h from the card's top left). The name field (where Q < NAME > E sit) is a
/// fixed width, so the arrows stay where they are whatever the piece is called.
const PREV: [f32; 4] = [14.0, 8.0, 66.0, 40.0];
const NEXT: [f32; 4] = [292.0, 8.0, 74.0, 40.0];
const NAME_FIELD: (f32, f32) = (70.0, 300.0);
const SNAP_BOX: [f32; 4] = [440.0, 8.0, 160.0, 40.0];
const SHOW_BOX: [f32; 4] = [14.0, 64.0, 190.0, 32.0];
/// The Scale slider: the span given to `pause_control` (its track ends 54 design px before the span does) and the box
/// that takes the pointer (the track and a little beyond).
const SLIDER_SPAN: (f32, f32) = (300.0, 478.0);
const SLIDER_BOX: [f32; 4] = [296.0, 64.0, 134.0, 32.0];
/// The buttons' row: top, height, gap.
const BUTTONS: (f32, f32, f32) = (98.0, 24.0, 10.0);

/// What Cancel puts back: the layout and every setting the Show switch can touch.
#[derive(Clone, Copy)]
struct Saved {
    hud: HudSettings,
    info_detail: usize,
    road_map: usize,
    name_plates: usize,
    checkpoint_info: usize,
    checkpoint_delay: u32,
}

impl Saved {
    fn of(s: &Settings) -> Saved {
        Saved { hud: s.hud, info_detail: s.info_detail, road_map: s.road_map, name_plates: s.name_plates, checkpoint_info: s.checkpoint_info, checkpoint_delay: s.checkpoint_delay }
    }

    fn restore(&self, s: &mut Settings) {
        (s.hud, s.info_detail, s.road_map, s.name_plates, s.checkpoint_info, s.checkpoint_delay) = (self.hud, self.info_detail, self.road_map, self.name_plates, self.checkpoint_info, self.checkpoint_delay);
    }
}

/// A control of the toolbar card.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum BarHit {
    Prev,
    Next,
    Snap,
    Show,
    Scale,
    Reset,
    ResetAll,
    Done,
    Cancel,
}

/// What the pointer is over.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Hit {
    Bar(BarHit),
    /// the resize handle of the selected piece
    Handle(HudElem),
    Piece(HudElem),
}

/// A drag the left button holds (latched at the press, as the demo play controls latch theirs, `Playback::mouse`).
#[derive(Clone, Copy, Debug)]
enum Drag {
    /// moving a piece: its placement at the press, where the pointer took it, its box then, and whether the pointer has
    /// left the slop yet (a click leaves the layout alone)
    Move { e: HudElem, start: ElemLayout, grab: [f32; 2], rect: Rect, moved: bool },
    /// resizing: the placement at the press, the point that stays put, the grabbed corner and which corner it is
    /// (right, bottom)
    Size { e: HudElem, start: ElemLayout, fixed: (f32, f32), corner: (f32, f32), side: (bool, bool) },
    /// the Scale slider
    Slider(HudElem),
    /// the RESET ALL button held (it fills as R / X does)
    ResetAll,
}

/// The layout editor's state (`Game::hud_edit`, while `Screen::HudEdit` is up).
#[derive(Clone, Copy)]
pub(crate) struct HudEdit {
    /// the selected piece
    pub(crate) sel: HudElem,
    saved: Saved,
    /// Reset all: R / X held for `HOLD_S` (armed by the press)
    pub(crate) reset_all: Hold,
    /// the arrow last pressed, when (`Game::clock`) and how many quick repeats of it so far
    nudge: Option<(KeyCode, f32, usize)>,
    /// the boxes of the last frame (what the mouse hits), what the pointer is over, the drag in progress and the lines
    /// a snapping move is on (x, y; canvas px)
    rects: HudRects,
    hover: Option<Hit>,
    drag: Option<Drag>,
    guides: [Option<f32>; 2],
    /// a move is under way with snapping on (the grid shows)
    snapping: bool,
}

/// Does box `a` overlap box `b` (canvas px, x y w h)?
fn overlaps(a: [f32; 4], b: [f32; 4]) -> bool {
    a[0] < b[0] + b[2] && b[0] < a[0] + a[2] && a[1] < b[1] + b[3] && b[1] < a[1] + a[3]
}

/// The anchor as the toolbar names it.
fn anchor_name(a: Anchor) -> &'static str {
    ["TOP LEFT", "TOP", "TOP RIGHT", "LEFT", "CENTRE", "RIGHT", "BOTTOM LEFT", "BOTTOM", "BOTTOM RIGHT"][a as usize]
}

/// Which corner of a box the resize handle sits on for a piece anchored at `a`: the one away from the anchor (the anchor
/// point stays put when a piece is scaled), (right, bottom). A piece anchored in the middle grows to the right / down.
fn handle_side(a: Anchor) -> (bool, bool) {
    (a as usize % 3 != 2, a as usize / 3 != 2)
}

/// A drag of the handle as a scale in percent: the grabbed `corner` is `fixed` + v, the pointer `p` projected on v gives
/// the factor. Rounded to `step` percent and kept in `ELEM_SCALE`.
fn resize_scale(start: u32, fixed: (f32, f32), corner: (f32, f32), p: (f32, f32), step: u32) -> u32 {
    let v = (corner.0 - fixed.0, corner.1 - fixed.1);
    let n = v.0 * v.0 + v.1 * v.1;
    if n < 1.0 {
        return start;
    }
    let t = ((p.0 - fixed.0) * v.0 + (p.1 - fixed.1) * v.1) / n;
    let steps = (start as f32 * t / step as f32).round().max(0.0) as u32;
    (steps * step).clamp(ELEM_SCALE.0, ELEM_SCALE.1)
}

/// Snap a move of the box `r` (canvas px: x y w h) by `delta` on a `cw` x 480 canvas: an edge or the centre of the box
/// within `SNAP_PX` of a screen edge or centre line jumps onto it (the guide is returned for drawing); an axis with no
/// such line puts the box's top left corner on the `GRID`.
fn snap_move(r: Rect, delta: (f32, f32), cw: f32) -> ((f32, f32), [Option<f32>; 2]) {
    let axis = |lo: f32, len: f32, d: f32, max: f32| -> (f32, Option<f32>) {
        let best = [0.0, len * 0.5, len]
            .into_iter()
            .flat_map(|own| [0.0, max * 0.5, max].into_iter().map(move |line| (line - (lo + own + d), line)))
            .filter(|(gap, _)| gap.abs() <= SNAP_PX)
            .min_by(|a, b| a.0.abs().total_cmp(&b.0.abs()));
        match best {
            Some((gap, line)) => (d + gap, Some(line)),
            None => ((((lo + d) / GRID).round() * GRID) - lo, None),
        }
    };
    let (dx, gx) = axis(r[0], r[2], delta.0, cw);
    let (dy, gy) = axis(r[1], r[3], delta.1, 480.0);
    ((dx, dy), [gx, gy])
}

/// The card's controls in canvas px, for a card whose top is at design y `top`: the one place the drawing and the
/// pointer both read them from.
fn bar_hits(d: Glass, top: f32) -> Vec<(BarHit, [f32; 4])> {
    let x0 = 640.0 - BAR_W * 0.5;
    let at = |b: [f32; 4]| [d.x(x0 + b[0]), d.y(top + b[1]), d.d(b[2]), d.d(b[3])];
    let mut out = vec![(BarHit::Prev, at(PREV)), (BarHit::Next, at(NEXT)), (BarHit::Snap, at(SNAP_BOX)), (BarHit::Show, at(SHOW_BOX)), (BarHit::Scale, at(SLIDER_BOX))];
    let (by, bh, gap) = BUTTONS;
    let bw = (BAR_W - 40.0 - 3.0 * gap) / 4.0;
    for (i, h) in [BarHit::Reset, BarHit::ResetAll, BarHit::Done, BarHit::Cancel].into_iter().enumerate() {
        out.push((h, at([20.0 + i as f32 * (bw + gap), by, bw, bh])));
    }
    out
}

/// The scale in percent the Scale slider shows for the pointer at canvas x `px` on the card whose top is `top`.
fn slider_scale(d: Glass, px: f32, snap: bool) -> u32 {
    let x0 = 640.0 - BAR_W * 0.5;
    let (t0, t1) = (d.x(x0 + SLIDER_SPAN.0), d.x(x0 + SLIDER_SPAN.1) - d.d(12.0 + 42.0));
    let f = ((px - t0) / (t1 - t0)).clamp(0.0, 1.0);
    let v = ELEM_SCALE.0 as f32 + f * (ELEM_SCALE.1 - ELEM_SCALE.0) as f32;
    let step = if snap { SNAP_SCALE as f32 } else { 1.0 };
    ((v / step).round() * step) as u32
}

/// A piece's box as the editor draws it (the drawn box with its 4 design px of air).
fn padded(d: Glass, r: Rect) -> Rect {
    let pad = d.d(4.0);
    [r[0] - pad, r[1] - pad, r[2] + 2.0 * pad, r[3] + 2.0 * pad]
}

/// The piece under `p` (canvas px): of the boxes (with their air) that hold it, the smallest, so a small piece inside a
/// big one can be picked.
fn pick_piece(rects: &HudRects, p: [f32; 2], d: Glass) -> Option<HudElem> {
    HudElem::ALL
        .into_iter()
        .filter_map(|e| rects.get(e).map(|r| (e, padded(d, r))))
        .filter(|(_, r)| p[0] >= r[0] && p[0] < r[0] + r[2] && p[1] >= r[1] && p[1] < r[1] + r[3])
        .min_by(|a, b| (a.1[2] * a.1[3]).total_cmp(&(b.1[2] * b.1[3])))
        .map(|(e, _)| e)
}

/// The corner of `r` the handle `side` names.
fn corner_of(r: Rect, side: (bool, bool)) -> (f32, f32) {
    (r[0] + if side.0 { r[2] } else { 0.0 }, r[1] + if side.1 { r[3] } else { 0.0 })
}

impl Game {
    /// The HUD tab's Edit layout row: the race frozen with the layout editor over it (the Race line HUD only).
    pub fn hud_edit_open(&mut self) {
        if self.race.is_none() || !self.settings.view.modern_hud() {
            return;
        }
        self.hud_edit = Some(HudEdit { sel: HudElem::Cluster, saved: Saved::of(&self.settings), reset_all: Hold::default(), nudge: None, rects: HudRects::default(), hover: None, drag: None, guides: [None; 2], snapping: false });
        self.screen = Screen::HudEdit;
        self.t = 0.0;
    }

    /// Headless shots (`--hud-edit ELEM`): the selected piece.
    pub fn hud_edit_select(&mut self, e: HudElem) {
        if let Some(ed) = &mut self.hud_edit {
            ed.sel = e;
        }
    }

    /// Back to the HUD tab, on the Edit layout row.
    fn hud_edit_leave(&mut self) {
        self.hud_edit = None;
        self.screen = Screen::PauseHud;
        self.t = 0.0;
        self.gfx = Some(GfxEdit::new(&self.settings));
        self.sel = gfx_page_rows(Screen::PauseHud, 0, &self.settings.view).iter().position(|r| *r == GfxRow::HudLayout).unwrap_or(0);
    }

    /// The HUD canvas's width in the window (the layout's offsets are canvas px).
    pub(super) fn hud_edit_w(&self) -> f32 {
        Canvas::hud(self.view_size).w
    }

    /// Change the selected piece's placement with `f`, then keep it as the editor writes it (`ElemLayout::normalise`).
    /// The stock layout is first frozen into the player's own, so nothing else moves.
    fn hud_edit_place(&mut self, e: HudElem, f: impl FnOnce(&mut ElemLayout)) {
        let Some(r) = &self.race else { return };
        let w = self.hud_edit_w();
        let mut lay = if self.settings.hud.layout.custom { self.settings.hud.layout } else { hud::freeze(self, r, w) };
        let home = hud::home_of(self, r, w, e);
        let l = &mut lay.elems[e as usize];
        f(l);
        if e.movable() {
            *l = l.normalise(home, w, self.settings.hud.scale);
        }
        self.settings.hud.layout = lay;
    }

    /// Show or hide a piece through the HUD's own settings (one source with the HUD tab). Hiding a corner piece forks
    /// InfoDetail into Custom (bug #181); showing a piece that another setting keeps off turns that on too (the splits
    /// need Checkpoint splits, the plates Name plates, WRONG WAY InfoDetail All).
    fn hud_set_shown(&mut self, e: HudElem, on: bool) {
        let s = &mut self.settings;
        match e {
            HudElem::Cluster => {
                fork_hud(s);
                s.hud.cluster = on;
            }
            HudElem::Stack => {
                fork_hud(s);
                (s.hud.clocks, s.hud.lap, s.hud.pos) = ([on; 3], on, on);
            }
            HudElem::Map => set_map_row(s, if !on { 0 } else if s.road_map == 0 { 2 } else { s.road_map }),
            HudElem::Splits => {
                s.hud.splits = on;
                if on {
                    if s.info_detail == 0 {
                        fork_hud(s);
                    }
                    if s.checkpoint_info == 0 {
                        s.checkpoint_info = 1;
                    }
                    if s.checkpoint_delay == 0 {
                        s.checkpoint_delay = Settings::default().checkpoint_delay;
                    }
                }
            }
            HudElem::Countdown => s.hud.countdown = on,
            HudElem::Messages => s.hud.messages = on,
            HudElem::WrongWay => {
                if on && s.info_detail < 2 {
                    fork_hud(s);
                }
                s.hud.wrong_way = on;
            }
            HudElem::ModeTags => s.hud.mode_tags = on,
            HudElem::Proximity => s.hud.proximity = on,
            HudElem::Plates => {
                s.hud.plates = on;
                if on && s.name_plates == 0 {
                    s.name_plates = 2;
                }
            }
        }
    }

    /// The editor's keys (the pad's arrive as these keys, `controls::poll`): Q / E (LB / RB, Tab) the piece, the
    /// arrows move it, Z / C (LT / RT, - / +) scale it, H (Y) shows or hides it, R (X) puts it back (held: all), Enter (A)
    /// keeps the layout, Esc (B, Start) puts back everything as it was on entry.
    pub(in crate::front) fn hud_edit_key(&mut self, k: KeyCode) {
        let Some(mut ed) = self.hud_edit else { return };
        let e = ed.sel;
        let mut pick = |fwd: bool| {
            let mut i = e as usize;
            Self::cycle(&mut i, HudElem::ALL.len(), fwd);
            ed.sel = HudElem::ALL[i];
            Some(if fwd { ev::CURSOR_DOWN } else { ev::CURSOR_UP })
        };
        let sound = match k {
            KeyCode::KeyQ | KeyCode::PageUp => pick(false),
            KeyCode::KeyE | KeyCode::PageDown => pick(true),
            KeyCode::Tab => pick(!self.shift),
            KeyCode::ArrowLeft | KeyCode::ArrowRight | KeyCode::ArrowUp | KeyCode::ArrowDown => {
                let n = match ed.nudge {
                    Some((key, at, n)) if key == k && self.clock - at <= NUDGE_REPEAT => n + 1,
                    _ => 0,
                };
                ed.nudge = Some((k, self.clock, n));
                let step = if self.shift { NUDGE_SHIFT } else { NUDGE[(n / NUDGE_EVERY).min(NUDGE.len() - 1)] };
                let (dx, dy) = match k {
                    KeyCode::ArrowLeft => (-step, 0),
                    KeyCode::ArrowRight => (step, 0),
                    KeyCode::ArrowUp => (0, -step),
                    _ => (0, step),
                };
                if e.movable() {
                    self.hud_edit_place(e, |l| l.off = (l.off.0 + dx, l.off.1 + dy));
                    None
                } else {
                    Some(ev::SELECT_ERROR)
                }
            }
            KeyCode::KeyZ | KeyCode::F7 | KeyCode::Minus | KeyCode::NumpadSubtract | KeyCode::KeyC | KeyCode::F8 | KeyCode::Equal | KeyCode::NumpadAdd => {
                let up = matches!(k, KeyCode::KeyC | KeyCode::F8 | KeyCode::Equal | KeyCode::NumpadAdd);
                self.hud_edit_scale(e, up);
                Some(if up { ev::ARROW_RIGHT } else { ev::ARROW_LEFT })
            }
            KeyCode::KeyH | KeyCode::F5 => self.hud_edit_toggle_show(e),
            KeyCode::KeyG => {
                self.settings.hud.snap ^= true;
                Some(if self.settings.hud.snap { ev::SWITCH_ON } else { ev::SWITCH_OFF })
            }
            KeyCode::KeyR | KeyCode::F6 => {
                self.hud_edit_reset_piece(e);
                ed.reset_all.armed = true;
                Some(ev::SELECT)
            }
            KeyCode::Enter | KeyCode::NumpadEnter => {
                self.hud_edit_done();
                return;
            }
            KeyCode::Escape | KeyCode::Backspace => {
                self.hud_edit_cancel();
                return;
            }
            _ => None,
        };
        self.hud_edit = Some(ed);
        if let Some(s) = sound {
            self.ev_override = Some(s);
        }
    }

    /// Scale the piece one step up or down (the keys, the wheel).
    fn hud_edit_scale(&mut self, e: HudElem, up: bool) {
        let step = if up { SCALE_STEP as i32 } else { -(SCALE_STEP as i32) };
        self.hud_edit_place(e, |l| l.scale = (l.scale as i32 + step).clamp(ELEM_SCALE.0 as i32, ELEM_SCALE.1 as i32) as u32);
    }

    /// Show or hide the piece (H, the SHOW switch); the sound that goes with it.
    fn hud_edit_toggle_show(&mut self, e: HudElem) -> Option<u32> {
        let r = self.race.as_ref()?;
        let on = !hud::elem_shown(self, r, e);
        self.hud_set_shown(e, on);
        Some(if on { ev::SWITCH_ON } else { ev::SWITCH_OFF })
    }

    /// The stock place and size of this piece (its Show stays); R held, `hud_edit_update` resets them all.
    fn hud_edit_reset_piece(&mut self, e: HudElem) {
        if self.settings.hud.layout.custom {
            if let Some(r) = &self.race {
                let stock = hud::freeze(self, r, self.hud_edit_w()).elems[e as usize];
                self.settings.hud.layout.elems[e as usize] = stock;
            }
        }
    }

    /// Done: keep the layout and go back to the HUD tab.
    fn hud_edit_done(&mut self) {
        self.save_settings();
        self.hud_edit_leave();
    }

    /// Cancel: put back everything as it was on entry.
    fn hud_edit_cancel(&mut self) {
        if let Some(ed) = self.hud_edit {
            ed.saved.restore(&mut self.settings);
        }
        self.hud_edit_leave();
    }

    /// Per frame: R / X (or the RESET ALL button) held long enough puts back the stock layout (Reset all), and the
    /// mouse: the boxes of this frame, what the pointer is over, the drag in progress.
    pub(in crate::front) fn hud_edit_update(&mut self, dt: f32, keys: &HashSet<KeyCode>) {
        let Some(mut ed) = self.hud_edit else { return };
        self.hud_edit_mouse(&mut ed, keys);
        let down = keys.contains(&KeyCode::KeyR) || keys.contains(&KeyCode::F6) || self.controls.pad_down(gilrs::Button::West) || matches!(ed.drag, Some(Drag::ResetAll));
        if ed.reset_all.step(dt, down) {
            self.settings.hud.layout = HudLayout::stock();
            self.sfx.push(Sfx::Menu(ev::SELECT));
        }
        self.hud_edit = Some(ed);
    }

    /// The boxes the HUD draws this frame (what the mouse hits).
    fn hud_edit_rects(&self) -> HudRects {
        let mut rects = HudRects::default();
        if let Some(r) = &self.race {
            let canvas = Canvas::hud(self.view_size);
            let px = 1.0 / canvas.transform(self.view_size).0;
            let _ = self.ui_in(Batch::new_for(&self.assets, canvas, true), |g, b| rects = hud::draw_rects(g, r, b, HudMode::Edit, px));
        }
        rects
    }

    /// A piece's placement as the editor changes it: the player's own, or the stock fit frozen into one.
    fn hud_edit_elem(&self, e: HudElem) -> ElemLayout {
        let lay = &self.settings.hud.layout;
        match &self.race {
            Some(r) if !lay.custom => hud::freeze(self, r, self.hud_edit_w()).elems[e as usize],
            _ => *lay.get(e),
        }
    }

    /// The anchor the handle is placed by: a piece that only scales has none (its handle is at the bottom right).
    fn hud_edit_anchor(&self, e: HudElem) -> Anchor {
        if e.movable() { self.settings.hud.layout.get(e).anchor } else { Anchor::TopLeft }
    }

    /// What the pointer `p` (canvas px) is over: a control of the card (the card takes every click on it), the handle of
    /// the selected piece, else the smallest box that holds it (so a small piece inside a big one can be picked).
    fn hud_edit_hit(&self, ed: &HudEdit, p: [f32; 2]) -> Option<Hit> {
        let d = Glass::new(&Canvas::hud(self.view_size), 1.0);
        let inside = |r: Rect| p[0] >= r[0] && p[0] < r[0] + r[2] && p[1] >= r[1] && p[1] < r[1] + r[3];
        let top = Self::hud_edit_bar_top(d, ed.rects.get(ed.sel));
        if inside([d.x(640.0 - BAR_W * 0.5), d.y(top), d.d(BAR_W), d.d(BAR_H)]) {
            return bar_hits(d, top).into_iter().find(|(_, r)| inside(*r)).map(|(h, _)| Hit::Bar(h));
        }
        if let Some(rc) = ed.rects.get(ed.sel) {
            let side = match ed.drag {
                Some(Drag::Size { side, .. }) => side,
                _ => handle_side(self.hud_edit_anchor(ed.sel)),
            };
            let c = corner_of(padded(d, rc), side);
            if (p[0] - c.0).hypot(p[1] - c.1) <= d.d(HANDLE_HIT) {
                return Some(Hit::Handle(ed.sel));
            }
        }
        pick_piece(&ed.rects, p, d).map(Hit::Piece)
    }

    /// A left press (`menu_mouse`): a control of the card does its job, the handle starts a resize, a piece is picked
    /// and, when it can move, taken. The drag then follows the pointer every frame (`hud_edit_mouse`).
    pub(in crate::front) fn hud_edit_press(&mut self) {
        let Some(mut ed) = self.hud_edit else { return };
        let Some((p, _)) = self.pointer else { return };
        ed.rects = self.hud_edit_rects();
        let d = Glass::new(&Canvas::hud(self.view_size), 1.0);
        let e = ed.sel;
        let sound = match self.hud_edit_hit(&ed, p) {
            None => None,
            Some(Hit::Bar(h @ (BarHit::Prev | BarHit::Next))) => {
                let mut i = e as usize;
                Self::cycle(&mut i, HudElem::ALL.len(), h == BarHit::Next);
                ed.sel = HudElem::ALL[i];
                Some(if h == BarHit::Next { ev::CURSOR_DOWN } else { ev::CURSOR_UP })
            }
            Some(Hit::Bar(BarHit::Snap)) => {
                self.settings.hud.snap ^= true;
                Some(if self.settings.hud.snap { ev::SWITCH_ON } else { ev::SWITCH_OFF })
            }
            Some(Hit::Bar(BarHit::Show)) => self.hud_edit_toggle_show(e),
            Some(Hit::Bar(BarHit::Scale)) => {
                ed.drag = Some(Drag::Slider(e));
                self.hud_edit_slider(e, p[0], d);
                Some(ev::SELECT)
            }
            Some(Hit::Bar(BarHit::Reset)) => {
                self.hud_edit_reset_piece(e);
                Some(ev::SELECT)
            }
            Some(Hit::Bar(BarHit::ResetAll)) => {
                ed.reset_all.armed = true;
                ed.drag = Some(Drag::ResetAll);
                Some(ev::SELECT)
            }
            Some(Hit::Bar(BarHit::Done)) => return self.hud_edit_done(),
            Some(Hit::Bar(BarHit::Cancel)) => return self.hud_edit_cancel(),
            Some(Hit::Handle(e)) => {
                if let Some(rc) = ed.rects.get(e) {
                    let start = self.hud_edit_elem(e);
                    let side = if e.movable() { handle_side(start.anchor) } else { (true, true) };
                    let corner = corner_of(padded(d, rc), side);
                    let fixed = match &self.race {
                        Some(r) if e.movable() => {
                            let w = self.hud_edit_w();
                            let put = start.put(hud::home_of(self, r, w, e), w, self.settings.hud.scale);
                            (put.anchor.0 + put.off.0, put.anchor.1 + put.off.1)
                        }
                        _ => (rc[0] + rc[2] * 0.5, rc[1] + rc[3] * 0.5),
                    };
                    ed.drag = Some(Drag::Size { e, start, fixed, corner, side });
                }
                None
            }
            Some(Hit::Piece(e)) => {
                if ed.sel != e {
                    ed.sel = e;
                }
                if let (true, Some(rect)) = (e.movable(), ed.rects.get(e)) {
                    ed.drag = Some(Drag::Move { e, start: self.hud_edit_elem(e), grab: p, rect, moved: false });
                }
                Some(ev::CURSOR_DOWN)
            }
        };
        if let Some(s) = sound {
            self.sfx.push(Sfx::Menu(s));
        }
        self.hud_edit = Some(ed);
    }

    /// The wheel (`menu_mouse`): up makes the selected piece bigger, down smaller.
    pub(in crate::front) fn hud_edit_wheel(&mut self, up: bool) {
        if let Some(ed) = self.hud_edit {
            self.hud_edit_scale(ed.sel, up);
            self.sfx.push(Sfx::Menu(if up { ev::ARROW_RIGHT } else { ev::ARROW_LEFT }));
        }
    }

    /// The Scale slider at pointer x `px`.
    fn hud_edit_slider(&mut self, e: HudElem, px: f32, d: Glass) {
        let scale = slider_scale(d, px, self.settings.hud.snap);
        self.hud_edit_place(e, |l| l.scale = scale.clamp(ELEM_SCALE.0, ELEM_SCALE.1));
    }

    /// Per frame, the mouse: the boxes, what the pointer is over, and the drag the left button holds.
    fn hud_edit_mouse(&mut self, ed: &mut HudEdit, keys: &HashSet<KeyCode>) {
        ed.rects = self.hud_edit_rects();
        let p = self.pointer.map(|p| p.0);
        if !self.controls.mouse_down(0) {
            ed.drag = None;
            ed.guides = [None; 2];
            ed.snapping = false;
        }
        ed.hover = match (ed.drag, p) {
            (None, Some(p)) => self.hud_edit_hit(ed, p),
            _ => ed.hover,
        };
        let (Some(drag), Some(p)) = (ed.drag, p) else { return };
        let d = Glass::new(&Canvas::hud(self.view_size), 1.0);
        let free = keys.contains(&KeyCode::ShiftLeft) || keys.contains(&KeyCode::ShiftRight);
        let snap = self.settings.hud.snap && !free;
        match drag {
            Drag::Move { e, start, grab, rect, moved } => {
                let (mut dx, mut dy) = (p[0] - grab[0], p[1] - grab[1]);
                let moved = moved || dx.hypot(dy) >= MOVE_SLOP;
                ed.drag = Some(Drag::Move { e, start, grab, rect, moved });
                ed.snapping = moved && snap;
                if moved {
                    ed.guides = [None; 2];
                    if snap {
                        ((dx, dy), ed.guides) = snap_move(rect, (dx, dy), self.hud_edit_w());
                    }
                    let off = (start.off.0 + dx.round() as i32, start.off.1 + dy.round() as i32);
                    self.hud_edit_place(e, |l| *l = ElemLayout { off, ..start });
                }
            }
            Drag::Size { e, start, fixed, corner, .. } => {
                let scale = resize_scale(start.scale, fixed, corner, (p[0], p[1]), if snap { SNAP_SCALE } else { 1 });
                if scale != start.scale || self.settings.hud.layout.custom {
                    self.hud_edit_place(e, |l| *l = ElemLayout { scale, ..start });
                }
            }
            Drag::Slider(e) => self.hud_edit_slider(e, p[0], d),
            Drag::ResetAll => {}
        }
    }

    /// Headless shots (`--hud-mouse SPEC`): drive the mouse. SPEC is a comma list, run in order, the pointer in canvas px
    /// (the HUD canvas, 480 tall, as the layout's offsets): `at=X:Y`, `at=ELEM` (the middle of that piece's box, the
    /// names of --hud-place), `at=handle` (the selected piece's handle) or `at=bar:NAME` (prev next snap show scale reset
    /// resetall done cancel) put the pointer there; `by=DX:DY` and `to=X:Y` move it (a drag, if the button is down);
    /// `down` / `up` the left button; `wheel=1|-1`; `shift` holds Shift from here on; `wait=S` seconds pass.
    /// Prints the boxes first, so the places can be picked.
    pub fn hud_edit_mouse_shot(&mut self, spec: &str, dt: f32) -> Result<(), String> {
        let size = self.view_size;
        let canvas = Canvas::hud(size);
        let (sx, sy, ox, oy) = canvas.axes(size);
        let mut keys: HashSet<KeyCode> = HashSet::new();
        self.update(dt, &keys);
        let Some(ed) = self.hud_edit else { return Err("--hud-mouse: not in the layout editor".into()) };
        let names: Vec<String> = HudElem::ALL.into_iter().filter_map(|e| ed.rects.get(e).map(|r| format!("{} [{:.0} {:.0} {:.0} {:.0}]", e.key(), r[0], r[1], r[2], r[3]))).collect();
        println!("hud boxes (canvas px x y w h): {}", names.join(", "));
        let mut at = [0.0f32, 0.0];
        for part in spec.split(',').filter(|p| !p.is_empty()) {
            let (key, val) = part.split_once('=').unwrap_or((part, ""));
            let xy = || -> Result<[f32; 2], String> {
                let (x, y) = val.split_once(':').ok_or(format!("--hud-mouse {part}: X:Y"))?;
                Ok([x.parse().map_err(|_| format!("--hud-mouse {part}: a number"))?, y.parse().map_err(|_| format!("--hud-mouse {part}: a number"))?])
            };
            let go = |g: &mut Game, p: [f32; 2], frames: usize, keys: &HashSet<KeyCode>| {
                g.pointer_moved(p[0] * sx + ox, p[1] * sy + oy, size);
                for _ in 0..frames {
                    g.update(dt, keys);
                }
            };
            match key {
                "at" | "to" => {
                    let ed = self.hud_edit.ok_or("--hud-mouse: the editor closed")?;
                    let d = Glass::new(&canvas, 1.0);
                    let rect_mid = |r: Rect| [r[0] + r[2] * 0.5, r[1] + r[3] * 0.5];
                    at = if let Some(e) = HudElem::from_key(val) {
                        ed.rects.get(e).map(rect_mid).ok_or(format!("--hud-mouse {part}: no box"))?
                    } else if val == "handle" {
                        let c = corner_of(padded(d, ed.rects.get(ed.sel).ok_or("--hud-mouse: no box")?), handle_side(self.hud_edit_anchor(ed.sel)));
                        [c.0, c.1]
                    } else if let Some(n) = val.strip_prefix("bar:") {
                        let top = Self::hud_edit_bar_top(d, ed.rects.get(ed.sel));
                        let want = ["prev", "next", "snap", "show", "scale", "reset", "resetall", "done", "cancel"].iter().position(|x| *x == n).ok_or(format!("--hud-mouse {part}: unknown control"))?;
                        let hits = bar_hits(d, top);
                        rect_mid(hits.iter().find(|(h, _)| *h as usize == want).map(|h| h.1).ok_or("--hud-mouse: no such control")?)
                    } else {
                        xy()?
                    };
                    go(self, at, 2, &keys);
                }
                "by" => {
                    let v = xy()?;
                    at = [at[0] + v[0], at[1] + v[1]];
                    go(self, at, 2, &keys);
                }
                "down" => {
                    self.controls.mouse_button(0, true);
                    self.menu_mouse(KeyCode::Enter);
                    go(self, at, 1, &keys);
                }
                "up" => {
                    self.controls.mouse_button(0, false);
                    go(self, at, 2, &keys);
                }
                "wheel" => self.menu_mouse(if val.starts_with('-') { KeyCode::ArrowRight } else { KeyCode::ArrowLeft }),
                "shift" => {
                    keys.insert(KeyCode::ShiftLeft);
                }
                "wait" => go(self, at, (val.parse::<f32>().map_err(|_| format!("--hud-mouse {part}: seconds"))? / dt) as usize, &keys),
                _ => return Err(format!("--hud-mouse: unknown step {part}")),
            }
        }
        Ok(())
    }

    /// The mouse pointer's shape: the editor's own while it is up (a hand over a piece, the diagonal arrows on the
    /// handle, a closed hand while moving, a finger on the card's controls).
    pub fn cursor_icon(&self) -> CursorIcon {
        let Some(ed) = self.hud_edit.filter(|_| self.screen == Screen::HudEdit) else { return CursorIcon::Default };
        let diagonal = |side: (bool, bool)| if side.0 == side.1 { CursorIcon::NwseResize } else { CursorIcon::NeswResize };
        match (ed.drag, ed.hover) {
            (Some(Drag::Move { .. }), _) => CursorIcon::Grabbing,
            (Some(Drag::Size { side, .. }), _) => diagonal(side),
            (Some(_), _) | (None, Some(Hit::Bar(_))) => CursorIcon::Pointer,
            (None, Some(Hit::Handle(e))) => diagonal(handle_side(self.hud_edit_anchor(e))),
            (None, Some(Hit::Piece(e))) if e.movable() => CursorIcon::Grab,
            (None, Some(Hit::Piece(_))) => CursorIcon::Pointer,
            (None, None) => CursorIcon::Default,
        }
    }

    /// The toolbar card's top (design px): docked at the bottom, up at the top while the selected piece is under it.
    fn hud_edit_bar_top(d: Glass, sel: Option<[f32; 4]>) -> f32 {
        let docked = BAR_BOTTOM - BAR_H;
        let card = [d.x(640.0 - BAR_W * 0.5), d.y(docked), d.d(BAR_W), d.d(BAR_H)];
        if sel.is_some_and(|r| overlaps(r, card)) { BAR_TOP_UP } else { docked }
    }

    /// The editor's layers: the race frosted behind the toolbar card, the veil, the HUD in edit mode, the boxes and the
    /// card over it.
    pub(in crate::front) fn hud_edit_layers(&self, size: (u32, u32)) -> Vec<Layer> {
        let (Some(r), Some(ed)) = (&self.race, self.hud_edit) else { return Vec::new() };
        let canvas = Canvas::hud(size);
        let px = 1.0 / canvas.transform(size).0;
        let mut rects = HudRects::default();
        let hud = self.ui_in(Batch::new_for(&self.assets, canvas, true), |g, b| rects = hud::draw_rects(g, r, b, HudMode::Edit, px));
        let d = Glass::new(&canvas, px);
        let top = Self::hud_edit_bar_top(d, rects.get(ed.sel));
        let mut parts = self.race_parts(r);
        parts.frost = Some(Self::scene_frost(size, |d| vec![(d.x(640.0 - BAR_W * 0.5), d.y(top), d.d(BAR_W), d.d(BAR_H))]));
        let veil = self.ui_in(Batch::sharp_text(&self.assets, canvas, 1.0), |_, b| {
            let (w, h) = (b.canvas.w, b.canvas.h);
            let mut m = Mesh::default();
            m.shade_rect(0.0, 0.0, w, h, 16, 9, &|q| {
                let k = (((q[0] - w * 0.5) / (w * 0.6)).powi(2) + ((q[1] - h * 0.5) / (h * 0.7)).powi(2)).sqrt().min(1.0);
                [3.0 / 255.0, 8.0 / 255.0, 18.0 / 255.0, VEIL.0 + (VEIL.1 - VEIL.0) * k]
            });
            b.mesh(&m);
        });
        let over = self.ui_in(Batch::sharp_text(&self.assets, canvas, 1.0), |g, b| g.hud_edit_overlay(b, d, r, &ed, &rects, top));
        vec![Layer::Scene { cam: r.cam, parts: Box::new(parts), viewport: None, mirror: false }, veil, hud, over]
    }

    /// The centre lines, a box and a name tag per piece (the selected one orange, the hidden ones dim and marked), the
    /// toolbar card and the hints.
    fn hud_edit_overlay(&self, b: &mut Batch, d: Glass, r: &RaceState, ed: &HudEdit, rects: &HudRects, top: f32) {
        let pulse = 0.5 - 0.5 * (self.t * TAU / 1.1).cos();
        let (w, h) = (b.canvas.w, b.canvas.h);
        let mut m = Mesh::default();
        let faint = hex(G_LINE, 0.16);
        m.dashed([w * 0.5, 0.0], [w * 0.5, h], d.d(6.0), d.d(6.0), d.w(1.0), faint, d.px);
        m.dashed([0.0, h * 0.5], [w, h * 0.5], d.d(6.0), d.d(6.0), d.w(1.0), faint, d.px);
        // a move with snapping on: the grid it steps on, and the lines it has caught
        if ed.snapping {
            let grid = hex(G_LINE, 0.07);
            let mut x = GRID;
            while x < w {
                m.stroke(&[[x, 0.0], [x, h]], false, d.w(0.5), Ends::Butt, grid, d.px);
                x += GRID;
            }
            let mut y = GRID;
            while y < h {
                m.stroke(&[[0.0, y], [w, y]], false, d.w(0.5), Ends::Butt, grid, d.px);
                y += GRID;
            }
        }
        if let Some(gx) = ed.guides[0] {
            m.stroke(&[[gx, 0.0], [gx, h]], false, d.w(1.5), Ends::Butt, hex(G_S1, 0.9), d.px);
        }
        if let Some(gy) = ed.guides[1] {
            m.stroke(&[[0.0, gy], [w, gy]], false, d.w(1.5), Ends::Butt, hex(G_S1, 0.9), d.px);
        }
        b.mesh(&m);
        // the others first, the selected piece on top
        let order = HudElem::ALL.into_iter().filter(|e| *e != ed.sel).chain([ed.sel]);
        for e in order {
            let Some(rc) = rects.get(e) else { continue };
            let (shown, sel) = (hud::elem_shown(self, r, e), e == ed.sel);
            let hot = !sel && ed.hover == Some(Hit::Piece(e));
            let pad = d.d(4.0);
            let (x, y, bw, bh) = (rc[0] - pad, rc[1] - pad, rc[2] + 2.0 * pad, rc[3] + 2.0 * pad);
            let rr = vec::rounded_rect(x, y, bw, bh, d.d(6.0));
            let mut m = Mesh::default();
            if !shown {
                m.fill(&rr, [2.0 / 255.0, 6.0 / 255.0, 14.0 / 255.0, 0.3], d.px);
            }
            if sel {
                m.halo(&rr, d.d(10.0), hex(G_ORANGE, 0.3 + 0.2 * pulse));
                m.fill(&rr, hex(G_ORANGE, 0.05 + 0.04 * pulse), d.px);
                m.stroke(&rr, true, d.w(1.5), Ends::Round, hex(G_ORANGE, 1.0), d.px);
            } else {
                let col = hex(G_LINE, if hot { 0.95 } else if shown { 0.55 } else { 0.3 });
                for (a, z) in [([x, y], [x + bw, y]), ([x + bw, y], [x + bw, y + bh]), ([x + bw, y + bh], [x, y + bh]), ([x, y + bh], [x, y])] {
                    m.dashed(a, z, d.d(5.0), d.d(4.0), d.w(1.0), col, d.px);
                }
            }
            b.mesh(&m);
            // the name on a dark pill over the box's top left; at the top of the screen beside the box (right, else
            // left, else under it), so it never covers a piece
            let name = if shown { e.name().to_string() } else { format!("{} · HIDDEN", e.name()) };
            let st = d.font(HudFont::GlassMed, 11.0, 1.5, hex(if sel { G_ORANGE } else if shown { G_TEXT } else { G_DIM }, 1.0));
            let (tw, th, gap) = (b.font_width(HudFont::GlassMed, &name, &st) - st.tracking + d.d(12.0), d.d(18.0), d.d(3.0));
            let (tx, ty) = if y - th - gap >= 0.0 {
                (x.clamp(0.0, (w - tw).max(0.0)), y - th - gap)
            } else if x + bw + gap + tw <= w {
                (x + bw + gap, y.max(0.0))
            } else if x - gap - tw >= 0.0 {
                (x - gap - tw, y.max(0.0))
            } else {
                (x.clamp(0.0, (w - tw).max(0.0)), y + bh + gap)
            };
            let mut m = Mesh::default();
            m.fill(&vec::rounded_rect(tx, ty, tw, th, d.d(5.0)), [2.0 / 255.0, 6.0 / 255.0, 14.0 / 255.0, 0.72], d.px);
            b.mesh(&m);
            b.font_text(HudFont::GlassMed, &name, tx + d.d(6.0), d.cap_top(HudFont::GlassMed, 11.0, ty, Some(18.0)), &st);
        }
        // the handle on the selected box's corner: pull it to resize
        if let Some(rc) = rects.get(ed.sel) {
            let side = match ed.drag {
                Some(Drag::Size { side, .. }) => side,
                _ => handle_side(self.hud_edit_anchor(ed.sel)),
            };
            let c = corner_of(padded(d, rc), side);
            let lit = matches!(ed.hover, Some(Hit::Handle(_))) || matches!(ed.drag, Some(Drag::Size { .. }));
            let sq = vec::rounded_rect(c.0 - d.d(HANDLE) * 0.5, c.1 - d.d(HANDLE) * 0.5, d.d(HANDLE), d.d(HANDLE), d.d(2.5));
            let mut m = Mesh::default();
            if lit {
                m.halo(&sq, d.d(8.0), hex(G_ORANGE, 0.7));
            }
            m.fill(&sq, if lit { hex(G_ORANGE, 1.0) } else { [4.0 / 255.0, 10.0 / 255.0, 20.0 / 255.0, 0.9] }, d.px);
            m.stroke(&sq, true, d.w(1.5), Ends::Round, hex(G_ORANGE, 1.0), d.px);
            b.mesh(&m);
        }
        self.hud_edit_bar(b, d, r, ed, top);
    }

    /// A key's glyph (`glyph_for` kind) for the device in use, 15 design px, its left edge at `x` and centred on `mid`;
    /// returns its width.
    fn hud_edit_glyph(&self, b: &mut Batch, d: Glass, kind: usize, x: f32, mid: f32) -> f32 {
        let (g, tint) = Self::glyph_for(self.controls.last, kind);
        let gs = FontStyle::new(d.d(15.0), tint);
        b.font_text(HudFont::Glyphs, &g.to_string(), x, mid - gs.size * 0.5, &gs)
    }

    /// The toolbar card with its top at design y `top`: the piece and its place, Show, Scale, stock or custom, and the
    /// four buttons with their keys.
    fn hud_edit_bar(&self, b: &mut Batch, d: Glass, r: &RaceState, ed: &HudEdit, top: f32) {
        let (x0, y0) = (640.0 - BAR_W * 0.5, top);
        let e = ed.sel;
        let mut m = Mesh::default();
        // a tool card: darker than the menu cards, so the HUD under it does not show through
        m.fill(&vec::rounded_rect(d.x(x0), d.y(y0), d.d(BAR_W), d.d(BAR_H), d.d(12.0)), [4.0 / 255.0, 10.0 / 255.0, 20.0 / 255.0, 0.72], d.px);
        glass_panel(&mut m, d, (d.x(x0), d.y(y0), d.d(BAR_W), d.d(BAR_H)));
        b.mesh(&m);
        // row 1: Q < NAME > E in a field of fixed width, its number, SNAP; then where the piece is anchored
        let hov = |h: BarHit| ed.hover == Some(Hit::Bar(h));
        let mid = d.y(y0 + 28.0);
        self.glass_shoulder(b, d, [d.x(x0 + 30.0), mid], "Q", 4);
        let st = d.font(HudFont::Glass, 22.0, 1.0, hex(G_ORANGE, 1.0));
        let nw = b.font_width(HudFont::Glass, e.name(), &st) - st.tracking;
        let (f0, f1) = NAME_FIELD;
        let nx = d.x(x0 + f0) + (d.d(f1 - f0) - nw).max(0.0) * 0.5;
        let mut m = Mesh::default();
        Self::glass_tri(&mut m, d, d.x(x0 + f0) - d.d(18.0), mid, true, hov(BarHit::Prev), 1.0, false);
        Self::glass_tri(&mut m, d, d.x(x0 + f1) + d.d(9.0), mid, false, hov(BarHit::Next), 1.0, false);
        b.mesh(&m);
        b.font_text(HudFont::Glass, e.name(), nx, d.cap_top(HudFont::Glass, 22.0, mid - d.d(14.0), Some(28.0)), &st);
        self.glass_shoulder(b, d, [d.x(x0 + f1 + 18.0 + 9.0 + 13.0), mid], "E", 5);
        let cap = d.font(HudFont::GlassMed, 11.0, 1.5, hex(G_DIM, 1.0));
        let count = format!("{} / {}", e as usize + 1, HudElem::ALL.len());
        b.font_text(HudFont::GlassMed, &count, d.x(x0 + 372.0), d.cap_top(HudFont::GlassMed, 11.0, mid - d.d(9.0), Some(18.0)), &cap);
        // SNAP: the G key (keyboard), the label and the switch; the mouse's drag snaps while it is on
        let label = d.font(HudFont::GlassMed, 13.0, 1.5, hex(if hov(BarHit::Snap) { G_TEXT } else { G_DIM }, 1.0));
        let sx = d.x(x0 + SNAP_BOX[0] + 8.0);
        let gw = if self.controls.last == Device::Keyboard { self.hud_edit_glyph(b, d, KIND_SNAP, sx, mid) + d.d(6.0) } else { 0.0 };
        b.font_text(HudFont::GlassMed, "SNAP", sx + gw, d.cap_top(HudFont::GlassMed, 13.0, mid - d.d(9.0), Some(18.0)), &label);
        self.pause_control(b, d, &PauseVal::Switch(self.settings.hud.snap), (d.x(x0 + 540.0), d.x(x0 + 600.0)), mid, true, 1.0);
        let lay = &self.settings.hud.layout;
        let place = if !e.movable() {
            "FOLLOWS THE CARS · SCALE AND SHOW ONLY".to_string()
        } else if lay.custom {
            let l = lay.get(e);
            format!("ANCHOR {} · {:+}, {:+}", anchor_name(l.anchor), l.off.0, l.off.1)
        } else {
            "STOCK PLACE · FITTED TO THE SCREEN".to_string()
        };
        b.font_text(HudFont::GlassMed, &place, d.x(x0 + 20.0), d.cap_top(HudFont::GlassMed, 11.0, d.y(y0 + 46.0), None), &cap);
        // the arrows on the right of that line: they move it (not the pieces that follow the cars)
        if e.movable() {
            let cmid = d.y(y0 + 52.0);
            let mw = b.font_width(HudFont::GlassMed, "MOVE", &cap) - cap.tracking;
            let mut x = d.x(x0 + BAR_W - 20.0) - mw - d.d(6.0 + 4.0 * 15.0 + 3.0 * 3.0);
            for kind in [0, 1, 9, 10] {
                x += self.hud_edit_glyph(b, d, kind, x, cmid) + d.d(3.0);
            }
            b.font_text(HudFont::GlassMed, "MOVE", x + d.d(3.0), d.cap_top(HudFont::GlassMed, 11.0, d.y(y0 + 46.0), None), &cap);
        }
        // row 2: H Show, Z Scale C, STOCK / CUSTOM
        let mid = d.y(y0 + 80.0);
        let label = d.font(HudFont::GlassMed, 13.0, 1.5, hex(G_DIM, 1.0));
        let lt = d.cap_top(HudFont::GlassMed, 13.0, mid - d.d(9.0), Some(18.0));
        let gw = self.hud_edit_glyph(b, d, KIND_SHOW, d.x(x0 + 20.0), mid);
        b.font_text(HudFont::GlassMed, "SHOW", d.x(x0 + 20.0) + gw + d.d(6.0), lt, &label);
        self.pause_control(b, d, &PauseVal::Switch(hud::elem_shown(self, r, e)), (d.x(x0 + 104.0), d.x(x0 + 196.0)), mid, true, 1.0);
        let gw = self.hud_edit_glyph(b, d, 14, d.x(x0 + 212.0), mid);
        self.hud_edit_glyph(b, d, 15, d.x(x0 + 486.0), mid);
        b.font_text(HudFont::GlassMed, "SCALE", d.x(x0 + 212.0) + gw + d.d(6.0), lt, &label);
        let scale = lay.get(e).scale;
        let f = (scale - ELEM_SCALE.0) as f32 / (ELEM_SCALE.1 - ELEM_SCALE.0) as f32;
        // a stock layout shows its fit's size: 100 % until the first change freezes it
        let text = if lay.custom { format!("{scale}%") } else { "AUTO".to_string() };
        self.pause_control(b, d, &PauseVal::Slider(if lay.custom { f } else { (100 - ELEM_SCALE.0) as f32 / (ELEM_SCALE.1 - ELEM_SCALE.0) as f32 }, text), (d.x(x0 + 300.0), d.x(x0 + 478.0)), mid, true, 1.0);
        let tag = if lay.custom { "CUSTOM" } else { "STOCK" };
        let ts = d.font(HudFont::Glass, 13.0, 1.5, hex(if lay.custom { G_S1 } else { G_DIM }, 1.0));
        let tw = b.font_width(HudFont::Glass, tag, &ts) - ts.tracking;
        let (tx, th) = (d.x(x0 + BAR_W - 20.0) - tw - d.d(16.0), d.d(22.0));
        let mut m = Mesh::default();
        let pill = vec::rounded_rect(tx, mid - th * 0.5, tw + d.d(16.0), th, d.d(7.0));
        if lay.custom {
            m.halo(&pill, d.d(8.0), hex(0x50aaff, 0.3));
            m.fill(&pill, hex(G_S1, 0.12), d.px);
        }
        m.stroke(&pill, true, d.w(1.0), Ends::Round, hex(if lay.custom { G_S1 } else { G_LINE }, if lay.custom { 0.8 } else { 0.25 }), d.px);
        b.mesh(&m);
        b.font_text(HudFont::Glass, tag, tx + d.d(8.0), d.cap_top(HudFont::Glass, 13.0, mid - th * 0.5, Some(22.0)), &ts);
        // row 3: the buttons, each with its key (Reset all fills while R / X is held)
        let (by, bh, gap) = (y0 + BUTTONS.0, BUTTONS.1, BUTTONS.2);
        let bw = (BAR_W - 40.0 - 3.0 * gap) / 4.0;
        let buttons = [("RESET", 7, None, BarHit::Reset), ("RESET ALL", 7, Some(ed.reset_all.v), BarHit::ResetAll), ("DONE", 2, None, BarHit::Done), ("CANCEL", 3, None, BarHit::Cancel)];
        for (i, (text, kind, hold, hit)) in buttons.into_iter().enumerate() {
            let x = d.x(x0 + 20.0 + i as f32 * (bw + gap));
            let (y, w, h) = (d.y(by), d.d(bw), d.d(bh));
            let body = vec::rounded_rect(x, y, w, h, d.d(7.0));
            let mut m = Mesh::default();
            m.fill(&body, [1.0, 1.0, 1.0, if hov(hit) { 0.14 } else { 0.05 }], d.px);
            if let Some(v) = hold.filter(|v| *v > 0.0) {
                m.fill(&vec::rounded_rect(x, y, w * v.min(1.0), h, d.d(7.0)), hex(G_ORANGE, 0.35), d.px);
            }
            m.stroke(&body, true, d.w(1.0), Ends::Round, hex(G_LINE, 0.25), d.px);
            b.mesh(&m);
            let (g, tint) = Self::glyph_for(self.controls.last, kind);
            let gw = b.font_width(HudFont::Glyphs, &g.to_string(), &FontStyle::new(d.d(15.0), tint));
            let st = d.font(HudFont::GlassMed, 12.0, 1.0, hex(G_TEXT, 1.0));
            // Reset all is the same key held: HOLD before its glyph
            let pre = d.font(HudFont::GlassMed, 10.0, 1.0, hex(G_DIM, 1.0));
            let pw = if hold.is_some() { b.font_width(HudFont::GlassMed, "HOLD", &pre) - pre.tracking + d.d(5.0) } else { 0.0 };
            let lw = b.font_width(HudFont::GlassMed, text, &st) - st.tracking;
            let lx = x + (w - pw - gw - d.d(6.0) - lw) * 0.5;
            if hold.is_some() {
                b.font_text(HudFont::GlassMed, "HOLD", lx, d.cap_top(HudFont::GlassMed, 10.0, y, Some(bh)), &pre);
            }
            self.hud_edit_glyph(b, d, kind, lx + pw, y + h * 0.5);
            b.font_text(HudFont::GlassMed, text, lx + pw + gw + d.d(6.0), d.cap_top(HudFont::GlassMed, 12.0, y, Some(bh)), &st);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn boxes_overlap_only_when_they_share_area() {
        assert!(overlaps([0.0, 0.0, 10.0, 10.0], [5.0, 5.0, 10.0, 10.0]));
        assert!(!overlaps([0.0, 0.0, 10.0, 10.0], [10.0, 0.0, 5.0, 5.0]));
        assert!(!overlaps([0.0, 0.0, 10.0, 10.0], [0.0, 20.0, 5.0, 5.0]));
    }

    fn glass() -> Glass {
        Glass::new(&Canvas::hud((1280, 720)), 1.0)
    }

    #[test]
    fn snapping_catches_edges_and_centre_lines_else_the_grid() {
        // the left edge 3 px off the screen edge: it jumps onto it, the guide is x = 0
        let ((dx, _), g) = snap_move([3.0, 100.0, 50.0, 20.0], (0.0, 0.0), 853.0);
        assert_eq!((dx, g[0]), (-3.0, Some(0.0)));
        // the centre 6 px from the centre line
        let ((dx, _), g) = snap_move([370.0, 100.0, 113.0, 20.0], (0.0, 0.0), 853.0);
        assert!((370.0 + 56.5 + dx - 426.5).abs() < 1e-3 && g[0] == Some(426.5), "{dx} {g:?}");
        // the bottom edge on the bottom of the screen
        let ((_, dy), g) = snap_move([200.0, 300.0, 50.0, 175.0], (0.0, 3.0), 853.0);
        assert!((300.0 + 175.0 + dy - 480.0).abs() < 1e-3 && g[1] == Some(480.0), "{dy} {g:?}");
        // away from every line: the top left corner on the 8 px grid, no guide
        let ((dx, dy), g) = snap_move([101.0, 133.0, 40.0, 30.0], (0.0, 0.0), 853.0);
        assert_eq!(((101.0 + dx) % GRID, (133.0 + dy) % GRID, g), (0.0, 0.0, [None, None]));
    }

    #[test]
    fn the_handle_is_the_corner_away_from_the_anchor() {
        assert_eq!(handle_side(Anchor::TopLeft), (true, true));
        assert_eq!(handle_side(Anchor::TopRight), (false, true));
        assert_eq!(handle_side(Anchor::BottomLeft), (true, false));
        assert_eq!(handle_side(Anchor::BottomRight), (false, false));
        // a piece anchored in the middle grows right and down
        assert_eq!(handle_side(Anchor::Centre), (true, true));
        assert_eq!(corner_of([10.0, 20.0, 100.0, 50.0], (false, true)), (10.0, 70.0));
    }

    #[test]
    fn pulling_the_handle_scales_about_the_fixed_point() {
        // the corner at (100, 50) from the fixed point (0, 0): twice as far = double, back at the start = unchanged
        assert_eq!(resize_scale(100, (0.0, 0.0), (100.0, 50.0), (200.0, 100.0), 1), 200);
        assert_eq!(resize_scale(100, (0.0, 0.0), (100.0, 50.0), (100.0, 50.0), 1), 100);
        // only the way along the diagonal counts; steps of 5; the range holds
        assert_eq!(resize_scale(100, (0.0, 0.0), (100.0, 0.0), (113.0, 400.0), 5), 115);
        assert_eq!(resize_scale(100, (0.0, 0.0), (100.0, 50.0), (-500.0, -500.0), 1), ELEM_SCALE.0);
        assert_eq!(resize_scale(100, (0.0, 0.0), (100.0, 50.0), (9000.0, 4500.0), 1), ELEM_SCALE.1);
        // a box with no size cannot be pulled
        assert_eq!(resize_scale(80, (5.0, 5.0), (5.0, 5.0), (50.0, 50.0), 1), 80);
    }

    #[test]
    fn the_smallest_box_under_the_pointer_is_picked() {
        let d = glass();
        let mut r = HudRects::default();
        r.rects[HudElem::Proximity as usize] = Some([100.0, 100.0, 300.0, 100.0]);
        r.rects[HudElem::Messages as usize] = Some([200.0, 120.0, 100.0, 20.0]);
        assert_eq!(pick_piece(&r, [250.0, 130.0], d), Some(HudElem::Messages));
        assert_eq!(pick_piece(&r, [120.0, 180.0], d), Some(HudElem::Proximity));
        assert_eq!(pick_piece(&r, [10.0, 10.0], d), None);
    }

    #[test]
    fn the_cards_controls_lie_inside_it_and_apart() {
        let d = glass();
        for top in [BAR_BOTTOM - BAR_H, BAR_TOP_UP] {
            let card = [d.x(640.0 - BAR_W * 0.5), d.y(top), d.d(BAR_W), d.d(BAR_H)];
            let hits = bar_hits(d, top);
            assert_eq!(hits.len(), 9);
            for (i, (h, r)) in hits.iter().enumerate() {
                assert!(r[0] >= card[0] - 1e-3 && r[1] >= card[1] - 1e-3 && r[0] + r[2] <= card[0] + card[2] + 1e-3 && r[1] + r[3] <= card[1] + card[3] + 1e-3, "{h:?} leaves the card");
                for (g, q) in &hits[i + 1..] {
                    assert!(!overlaps(*r, *q), "{h:?} and {g:?} overlap");
                }
            }
        }
    }

    #[test]
    fn the_slider_maps_the_track_to_the_scale_range() {
        let d = glass();
        let x0 = 640.0 - BAR_W * 0.5;
        let (t0, t1) = (d.x(x0 + SLIDER_SPAN.0), d.x(x0 + SLIDER_SPAN.1) - d.d(54.0));
        assert_eq!(slider_scale(d, t0 - 50.0, true), ELEM_SCALE.0);
        assert_eq!(slider_scale(d, t1 + 50.0, true), ELEM_SCALE.1);
        assert_eq!(slider_scale(d, (t0 + t1) * 0.5, false), (ELEM_SCALE.0 + ELEM_SCALE.1) / 2 + (ELEM_SCALE.0 + ELEM_SCALE.1) % 2);
        assert_eq!(slider_scale(d, (t0 + t1) * 0.5, true) % SNAP_SCALE, 0);
    }

    #[test]
    fn cancel_puts_back_what_the_editor_touches() {
        let mut s = Settings::default();
        let saved = Saved::of(&s);
        // the toggles the Show switch writes and the layout
        s.hud.cluster = false;
        s.hud.layout.custom = true;
        s.checkpoint_info = 0;
        s.name_plates = 0;
        saved.restore(&mut s);
        assert_eq!(s.hud, Settings::default().hud);
        assert_eq!((s.checkpoint_info, s.name_plates), (Settings::default().checkpoint_info, Settings::default().name_plates));
    }
}
