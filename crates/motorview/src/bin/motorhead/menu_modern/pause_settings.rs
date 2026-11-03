//! REMASTER pause menu settings tabs (#300 phase 2): the rows card and the info card of concept C
//! (`reference_materials/pause-overhaul/C-paddock.html`, `settingsHTML` / `valueHTML`, styles in
//! `assets/glass.css`). One drawing path for the five settings tabs (Sound, Controls, Camera and, phase 3,
//! Graphics and HUD, whose info card holds the HUD preview). Design px through `Glass`, the rules are
//! 19-glass-hud-design-language.

use super::*;

/// design px of a row, a section caption, the list window and a row's control column
const ROW_H: f32 = 36.0;
const HEAD_H: f32 = 30.0;
const LIST_H: f32 = 352.0;
const CTL_W: f32 = 210.0;
/// the pause menu's rows card and info card (design px)
pub(super) const ROWS_CARD: (f32, f32, f32, f32) = (62.0, 228.0, 700.0, 372.0);
pub(super) const INFO_CARD: (f32, f32, f32, f32) = (782.0, 228.0, 436.0, 372.0);

/// The help of the three pad tuning sliders (the Controls tab's last rows).
pub(super) const CTL_TUNE_HELP: [&str; 3] = [
    "How far the stick moves before the car steers. Raise it if the car pulls on its own.",
    "How far the triggers move before throttle and brake respond.",
    "Steering per stick travel. Above 100% reaches full lock before the end of the stick.",
];

/// One cap of a binding: a PromptFont glyph (a key cap or button already) or a name on a drawn cap.
pub(super) enum Cap {
    Glyph(char, [f32; 4]),
    Text(String),
}

/// A task's key and pad caps; `waiting` while a binding waits for its control.
pub(super) struct BindCaps {
    pub key: Cap,
    pub pad: Cap,
    pub pad_dim: bool,
    pub waiting: bool,
}

pub(super) enum PauseVal {
    Choice(Vec<String>, usize),
    Switch(bool),
    Slider(f32, String),
    Bind(BindCaps),
    /// a text field (the Personal screen, #91): the value, its length limit (the counter shows up to
    /// 15) and whether it is being typed into
    Text { value: String, max: usize, editing: bool },
    /// a colour swatch and its name between the arrows; `first` / `last` dim the arrow at an end of the list
    Swatch { rgb: [u8; 3], name: String, first: bool, last: bool },
    /// the Sound screen's output choice (#260): a wide `< NAME >` and, in the info card, the list of outputs
    Device(DeviceView),
}

pub(super) struct PauseRow {
    pub label: String,
    pub help: String,
    pub val: PauseVal,
    /// 1.0, or 0.4 for a row that does not apply
    pub dim: f32,
}

pub(super) enum PauseItem {
    /// a section caption with its dashed rule
    Head(String),
    Row(PauseRow),
}

impl PauseRow {
    /// A row of today's option panels (`OptRow`) with its help text.
    pub fn from_opt(row: OptRow, help: &str) -> PauseRow {
        let val = match row.val {
            RowValue::Control(Control::Switch(on)) => PauseVal::Switch(on),
            RowValue::Control(Control::Slider(f, text)) => PauseVal::Slider(f, text),
            // the panels' own check box for an Off / On choice
            RowValue::Choices(list, cur) if list.len() == 2 && list[0] == "Off" && list[1] == "On" => PauseVal::Switch(cur == 1),
            RowValue::Choices(list, cur) => PauseVal::Choice(list.iter().map(|s| s.trim_matches(|c| c == '<' || c == '>' || c == ' ').to_string()).collect(), cur),
        };
        PauseRow { label: row.label.trim_end_matches(':').to_string(), help: help.to_string(), val, dim: row.dim }
    }
}

/// The item index of row number `sel` (`sel` counts rows only, not captions).
fn row_item(items: &[PauseItem], sel: usize) -> usize {
    items.iter().enumerate().filter(|(_, i)| matches!(i, PauseItem::Row(_))).nth(sel).map_or(0, |(i, _)| i)
}

/// First item shown in a list window of `avail` design px: the selected item centred, no blank
/// space at the end, whole rows only (`Batch` cannot clip, so the list steps a row at a time).
pub(super) fn rows_first(heights: &[f32], sel: usize, avail: f32) -> usize {
    if heights.iter().sum::<f32>() <= avail {
        return 0;
    }
    // the first item from which everything to the end still fits
    let (mut tail, mut max_first) = (0.0, heights.len());
    for (i, h) in heights.iter().enumerate().rev() {
        if tail + h > avail {
            break;
        }
        tail += h;
        max_first = i;
    }
    let want = heights[..sel].iter().sum::<f32>() - (avail - heights[sel]) * 0.5;
    let (mut y, mut first) = (0.0, 0);
    while y < want && first < sel {
        y += heights[first];
        first += 1;
    }
    first.min(max_first)
}

fn with_a(c: [f32; 4], a: f32) -> [f32; 4] {
    [c[0], c[1], c[2], c[3] * a]
}

/// A CSS line box with the font's normal line height, in design px.
pub(super) fn line_h(f: HudFont, px: f32) -> f32 {
    let face = ui_font::face(f);
    (face.ascender - face.descender) * px / ui_font::SRC
}

/// The text wrapped to `max_w` canvas px.
pub(super) fn wrap_text(b: &Batch, f: HudFont, st: &FontStyle, text: &str, max_w: f32) -> Vec<String> {
    let (mut lines, mut line) = (Vec::new(), String::new());
    for word in text.split(' ') {
        let next = if line.is_empty() { word.to_string() } else { format!("{line} {word}") };
        if !line.is_empty() && b.font_width(f, &next, st) - st.tracking > max_w {
            lines.push(std::mem::take(&mut line));
            line = word.to_string();
        } else {
            line = next;
        }
    }
    lines.push(line);
    lines
}

/// `.card.panel`: the glass card a little darker under long text (the standings' fill).
pub(super) fn glass_panel(m: &mut Mesh, d: Glass, rect: (f32, f32, f32, f32)) {
    m.fill(&vec::rounded_rect(rect.0, rect.1, rect.2, rect.3, d.d(12.0)), [6.0 / 255.0, 14.0 / 255.0, 28.0 / 255.0, 0.38], d.px);
    glass_card(m, d, rect, d.d(12.0), (0.11, 0.03), 0.36, (10.0, 26.0, 0.45), 0.35, None);
}

/// `.check` (19.4): a check box `size` design px square with corners of `radius` at (bx, by), lit cyan with its tick
/// when on (the tick drawn in the 22 px box's units).
pub(super) fn check_box(m: &mut Mesh, d: Glass, (bx, by): (f32, f32), (size, radius): (f32, f32), lit: bool, a: f32) {
    let (s, k) = (d.d(size), size / 22.0);
    let r = vec::rounded_rect(bx, by, s, s, d.d(radius));
    if lit {
        m.halo(&r, d.d(14.0), hex(0x50c8ff, 0.75 * a));
        m.fill_with(&r, &|q| {
            let t = ((q[1] - by) / s).clamp(0.0, 1.0);
            [(0x6f as f32 + (0x2a as f32 - 0x6f as f32) * t) / 255.0, (0xdc as f32 + (0xa6 as f32 - 0xdc as f32) * t) / 255.0, (0xff as f32 + (0xd4 as f32 - 0xff as f32) * t) / 255.0, a]
        }, d.px);
        m.stroke(&[[bx + d.d(5.0 * k), by + d.d(1.0)], [bx + s - d.d(5.0 * k), by + d.d(1.0)]], false, d.w(1.0), Ends::Butt, [1.0, 1.0, 1.0, 0.6 * a], d.px);
        let tick = [[bx + d.d(5.0 * k), by + d.d(11.5 * k)], [bx + d.d(9.0 * k), by + d.d(15.5 * k)], [bx + d.d(17.0 * k), by + d.d(6.5 * k)]];
        m.stroke(&tick, false, d.w(2.4 * k), Ends::Round, hex(0x04243a, a), d.px);
    } else {
        m.fill(&r, [1.0, 1.0, 1.0, 0.08 * a], d.px);
        m.stroke(&r, true, d.w(1.0), Ends::Round, hex(G_LINE, 0.5 * a), d.px);
    }
}

/// `.kcap`'s body: a key cap (`round`: a pad button's pill) of `w` x `h` canvas px at (x, y).
pub(super) fn cap_body(m: &mut Mesh, d: Glass, (x, y, w, h): (f32, f32, f32, f32), round: bool, a: f32) {
    let body = vec::rounded_rect(x, y, w, h, if round { h * 0.5 } else { d.d(5.0) });
    m.fill(&vec::rounded_rect(x, y + d.d(2.0), w, h, if round { h * 0.5 } else { d.d(5.0) }), [0.0, 0.0, 0.0, 0.45 * a], d.px);
    m.fill_with(&body, &|q| [1.0, 1.0, 1.0, (0.28 - 0.20 * ((q[1] - y) / h).clamp(0.0, 1.0)) * a], d.px);
    m.stroke(&body, true, d.w(1.0), Ends::Round, [1.0, 1.0, 1.0, 0.4 * a], d.px);
}

impl Game {
    /// The Sound tab's rows in the pause order (SFX, CD volume, CD music, ambient, engine sound).
    pub(super) fn pause_sound_rows(&self) -> Vec<(OptRow, &'static str)> {
        // the pause tab's order: SFX, CD volume, CD music, ambient, engine sound
        let mut all: Vec<Option<_>> = self.sound_items().into_iter().map(Some).collect();
        [0, 1, 3, 2, 6].into_iter().map(|i| all[i].take().expect("sound item")).collect()
    }

    /// The Controls tab: the car tasks with their key and pad caps, then the keyboard layout and the
    /// pad's tuning sliders under their own captions.
    fn pause_controls_items(&self) -> Vec<PauseItem> {
        let dev = self.pad_device();
        let layout = self.controls.layout();
        let pad_off = !self.controls.pad_active();
        let mut help = "Press Enter, then the key or button for this task. Esc cancels; Start cancels a pad binding.".to_string();
        if !layout.is_custom() {
            help += &format!(" Binding a key copies the {} layout to Custom first.", layout.name);
        }
        if pad_off {
            help += &format!(" The {} layout drives the pad itself, so the pad bindings are off.", layout.name);
        }
        let mut items: Vec<PauseItem> = (0..CAR_TASKS)
            .map(|t| {
                let key = match layout.bind[t] {
                    Some(c) => code_glyph(c, dev).map_or_else(|| Cap::Text(c.name().to_uppercase()), |(g, tint)| Cap::Glyph(g, tint)),
                    None => Cap::Text("-".into()),
                };
                let pad = self.controls.pad_bind[t].map_or(Cap::Text("-".into()), |p| {
                    let (g, tint) = pad_glyph(p, dev);
                    Cap::Glyph(g, tint)
                });
                let waiting = self.capture.is_some_and(|c| c.task == t && c.any);
                PauseItem::Row(PauseRow { label: TASK_LABELS[t].into(), help: help.clone(), val: PauseVal::Bind(BindCaps { key, pad, pad_dim: pad_off, waiting }), dim: 1.0 })
            })
            .collect();
        items.push(PauseItem::Head("Keyboard".into()));
        let names = self.controls.layouts.iter().map(|l| l.name.clone()).collect();
        items.push(PauseItem::Row(PauseRow {
            label: "Layout".into(),
            help: "The control layout: the presets of the install (Game/*.clo) and Custom. Changing a key edits Custom.".into(),
            val: PauseVal::Choice(names, self.controls.active),
            dim: 1.0,
        }));
        items.push(PauseItem::Head("Controller".into()));
        for (i, label) in CTL_TUNE_LABELS.iter().enumerate() {
            let (lo, hi) = PAD_TUNE_RANGE[i];
            let v = self.settings.pad_tune[i];
            items.push(PauseItem::Row(PauseRow { label: (*label).into(), help: CTL_TUNE_HELP[i].into(), val: PauseVal::Slider((v - lo) as f32 / (hi - lo) as f32, format!("{v}%")), dim: 1.0 }));
        }
        items
    }

    /// The rows of the settings tab on screen, captions included.
    fn settings_items(&self, b: &Batch, r: Option<&RaceState>) -> Vec<PauseItem> {
        match self.screen {
            Screen::Sound => {
                let mut items: Vec<PauseItem> = self.sound_items().into_iter().map(|(r, h)| PauseItem::Row(PauseRow::from_opt(r, h))).collect();
                // #260: the main menu's Sound screen only; the pause tab (`pause_sound_rows`) keeps its five
                items.push(self.device_row());
                items
            }
            Screen::PauseSfx => self.pause_sound_rows().into_iter().map(|(r, h)| PauseItem::Row(PauseRow::from_opt(r, h))).collect(),
            Screen::Graphics | Screen::PauseGfx | Screen::PauseCam | Screen::PauseHud => {
                let e = self.gfx.unwrap_or_else(|| GfxEdit::new(&self.settings));
                let mut items = Vec::new();
                for (name, rows) in modern_sections(self.screen, e.page, &self.settings.view) {
                    // the main menu's Graphics pages are the tabs over the list, not captions in it
                    if !name.is_empty() && self.screen != Screen::Graphics {
                        items.push(PauseItem::Head(name.into()));
                    }
                    for row in rows {
                        let (label, val) = self.gfx_row_view_modern(row, &e);
                        let dim = if row.active_in(&self.settings) { 1.0 } else { 0.4 };
                        let mut pr = PauseRow::from_opt(OptRow { label: label.into(), val, dim }, row.help_modern());
                        // the Modern HUD shrinks to fit a small window: say what it is drawn at
                        if let (GfxRow::HudScale, Some(r), true) = (row, r, dim == 1.0) {
                            let (asked, fit) = (self.settings.hud.scale, hud::effective_scale(self, r, b));
                            if fit + 0.005 < asked as f32 / 100.0 {
                                pr.help += &format!(" Now drawn at {:.0}% to fit.", fit * 100.0);
                            }
                        }
                        items.push(PauseItem::Row(pr));
                    }
                }
                items
            }
            Screen::PauseControls | Screen::Controls => self.pause_controls_items(),
            _ => Vec::new(),
        }
    }

    /// The rows card and the info card of the settings screen on show (pause tab or main menu
    /// screen); true when the selected row is a key binding (for the hints).
    fn settings_cards(&self, b: &mut Batch, d: Glass, r: Option<&RaceState>, pulse: f32) -> bool {
        let items = self.settings_items(b, r);
        let n_rows = items.iter().filter(|i| matches!(i, PauseItem::Row(_))).count();
        if n_rows == 0 {
            return false;
        }
        let sel = self.sel.min(n_rows - 1);
        let (name, icon) = match self.screen {
            Screen::PauseSfx | Screen::Sound => ("Sound", ui_font::icon::VOLUME),
            Screen::PauseControls | Screen::Controls => ("Controls", ui_font::icon::GAMEPAD),
            Screen::PauseGfx => ("Graphics", ui_font::icon::MONITOR),
            Screen::Graphics => ("Graphics", ui_font::icon::MONITOR),
            Screen::PauseHud => ("HUD", ui_font::icon::GAUGE),
            _ => ("Camera", ui_font::icon::CAMERA),
        };
        // the main menu's Graphics names its page in the caption, where the pause tab has a section caption
        let name = match self.screen {
            Screen::Graphics => format!("{name} // {}", MODERN_GFX_PAGES[self.gfx.map_or(0, |e| e.page)].0),
            _ => name.to_string(),
        };
        self.pause_rows_card(b, d, &items, sel, pulse);
        // the HUD tab shows the race screen's pieces instead of the value: the selected row's lit
        let hud = (self.screen == Screen::PauseHud).then(|| {
            let lit = modern_sections(self.screen, 0, &self.settings.view).into_iter().flat_map(|(_, rows)| rows).nth(sel).map_or(&[][..], |row| row.hud_lit());
            (lit, self.hud_pieces_shown())
        });
        self.pause_info_card(b, d, &items, sel, &name, icon, hud, pulse);
        // display changes wait for Apply (and the keep-settings question)
        if matches!(self.screen, Screen::Graphics | Screen::PauseGfx) && self.gfx.is_some_and(|e| e.display != self.settings.display) {
            let (ix, iy, iw, _) = INFO_CARD;
            let st = d.font(HudFont::Glass, 10.0, 1.5, hex(G_ORANGE, 1.0));
            let text = "NOT APPLIED";
            let tw = b.font_width(HudFont::Glass, text, &st) - st.tracking;
            let (w, h) = (tw + d.d(16.0), d.d(18.0));
            let (x, y) = (d.x(ix + iw - 16.0) - w, d.y(iy + 40.0));
            let tag = vec::rounded_rect(x, y, w, h, d.d(7.0));
            let mut m = Mesh::default();
            m.fill(&tag, hex(0x0a1626, 1.0), d.px);
            m.stroke(&tag, true, d.w(1.0), Ends::Round, hex(G_ORANGE, 0.6), d.px);
            b.mesh(&m);
            b.font_text(HudFont::Glass, text, x + d.d(8.0), d.cap_top(HudFont::Glass, 10.0, y, Some(18.0)), &st);
        }
        matches!(&items[row_item(&items, sel)], PauseItem::Row(PauseRow { val: PauseVal::Bind(_), .. }))
    }

    /// The pause menu's Sound, Camera, Controls, Graphics or HUD tab: the cards and the hints.
    pub(super) fn pause_settings_tab(&self, b: &mut Batch, d: Glass, r: &RaceState, pulse: f32) {
        let on_bind = self.settings_cards(b, d, Some(r), pulse);
        if self.asking() {
            return;
        }
        // the hints: the tab's own after TABS / MOVE / CHANGE / RESUME
        if self.capture.is_some() {
            let pad = self.pad_device();
            Self::glass_hints_dev(b, d, &[(&[(Device::Keyboard, 3), (pad, 8)], "CANCEL")], 626.0);
            return;
        }
        let device = self.controls.last;
        let mut hints: Vec<(&[usize], &str)> = vec![(&[4, 5], "TABS"), (&[9, 10], "MOVE"), if on_bind { (&[2], "CHANGE") } else { (&[0, 1], "CHANGE") }, (&[3], "RESUME")];
        match self.screen {
            Screen::PauseCam | Screen::PauseGfx => hints.extend([(&[6][..], "APPLY"), (&[7][..], "DEFAULTS")]),
            Screen::PauseHud => hints.push((&[7], "DEFAULTS")),
            Screen::PauseControls => hints.push((&[7], "DEFAULTS")),
            _ => {}
        }
        Self::glass_hints(b, d, device, &hints, 626.0);
    }

    /// A main menu settings screen (Graphics): the cards and the hints, BACK instead of RESUME.
    pub(super) fn settings_screen_body(&self, b: &mut Batch, d: Glass, pulse: f32) {
        let on_bind = self.settings_cards(b, d, None, pulse);
        if self.asking() {
            return;
        }
        if self.capture.is_some() {
            let pad = self.pad_device();
            Self::glass_hints_dev(b, d, &[(&[(Device::Keyboard, 3), (pad, 8)], "CANCEL")], 626.0);
            return;
        }
        let mut hints: Vec<(&[usize], &str)> = Vec::new();
        if self.screen == Screen::Graphics {
            hints.push((&[4, 5], "PAGE"));
        }
        if self.screen == Screen::Sound {
            hints.push((&[4, 5], "PAGES"));
        }
        hints.extend([(&[9, 10][..], "MOVE"), if on_bind { (&[2][..], "CHANGE") } else { (&[0, 1][..], "CHANGE") }]);
        if self.screen == Screen::Graphics {
            hints.extend([(&[6][..], "APPLY"), (&[7][..], "DEFAULTS")]);
        } else if self.screen == Screen::Controls {
            hints.push((&[7], "DEFAULTS"));
        }
        hints.push((&[3], "BACK"));
        Self::glass_hints(b, d, self.controls.last, &hints, 626.0);
    }

    /// The rows card: the list window round the selected row, the scroll bar at its right edge.
    fn pause_rows_card(&self, b: &mut Batch, d: Glass, items: &[PauseItem], sel: usize, pulse: f32) {
        self.rows_card(b, d, ROWS_CARD, 672.0, LIST_H, items, sel, pulse);
    }

    /// A rows card at `rect` (design px) listing `items` in a window `list_h` high and `rows_w` wide: the
    /// pause menu's, and the Personal screen's Driver card (#91).
    #[allow(clippy::too_many_arguments)]
    pub(super) fn rows_card(&self, b: &mut Batch, d: Glass, rect: (f32, f32, f32, f32), rows_w: f32, list_h: f32, items: &[PauseItem], sel: usize, pulse: f32) {
        let (cx, cy, cw, ch) = rect;
        let mut m = Mesh::default();
        glass_panel(&mut m, d, (d.x(cx), d.y(cy), d.d(cw), d.d(ch)));
        b.mesh(&m);
        let heights: Vec<f32> = items.iter().map(|i| if matches!(i, PauseItem::Head(_)) { HEAD_H } else { ROW_H }).collect();
        let sel_item = row_item(items, sel);
        let first = rows_first(&heights, sel_item, list_h);
        let (rx, rw) = (d.x(cx + 10.0), d.d(rows_w));
        let (left, right) = (rx + d.d(15.0), rx + rw - d.d(15.0));
        let mut y = 0.0;
        for (i, item) in items.iter().enumerate().skip(first) {
            if y + heights[i] > list_h {
                break;
            }
            let top = d.y(cy + 10.0 + y);
            y += heights[i];
            let row = match item {
                PauseItem::Head(name) => {
                    let st = d.font(HudFont::GlassMed, 11.0, 1.5, hex(G_DIM, 1.0));
                    let name = name.to_uppercase();
                    let w = b.font_width(HudFont::GlassMed, &name, &st);
                    b.font_text(HudFont::GlassMed, &name, left, d.cap_top(HudFont::GlassMed, 11.0, top + d.d(8.0), Some(22.0)), &st);
                    let mut m = Mesh::default();
                    let ly = top + d.d(19.0);
                    m.dashed([left + w + d.d(10.0), ly], [right, ly], d.d(6.0), d.d(4.0), d.w(1.0), hex(G_LINE, 0.3), d.px);
                    b.mesh(&m);
                    continue;
                }
                PauseItem::Row(row) => row,
            };
            let on = i == sel_item;
            let a = row.dim;
            let (rh, mid) = (d.d(ROW_H), top + d.d(ROW_H) * 0.5);
            let mut m = Mesh::default();
            if on {
                // 19.4: the orange rim with its halo pulsing .3 to .5, never a filled bar
                let rr = vec::rounded_rect(rx, top, rw, rh, d.d(10.0));
                m.halo(&rr, d.d(10.0), hex(G_ORANGE, 0.3 + 0.2 * pulse));
                m.fill(&rr, hex(G_ORANGE, 0.06 + 0.06 * pulse), d.px);
                m.stroke(&rr, true, d.w(1.0), Ends::Round, hex(G_ORANGE, 1.0), d.px);
            } else if i > first && i - 1 != sel_item && matches!(items[i - 1], PauseItem::Row(_)) {
                m.stroke(&[[left, top], [right, top]], false, d.w(1.0), Ends::Butt, hex(G_LINE, 0.09), d.px);
            }
            b.mesh(&m);
            let label = row.label.to_uppercase();
            let col = hex(if on { G_ORANGE } else { G_TEXT }, a);
            b.font_text(HudFont::Glass, &label, left, d.cap_top(HudFont::Glass, 18.0, top, Some(ROW_H)), &d.font(HudFont::Glass, 18.0, 0.5, col));
            self.pause_control(b, d, &row.val, (right - d.d(CTL_W), right), mid, on, a);
        }
        // the scroll bar: the window's share of the list is the thumb
        let total: f32 = heights.iter().sum();
        if total > list_h {
            let (bx, top, len) = (d.x(cx + cw - 12.0 + 1.5), d.y(cy + 14.0), d.d(ch - 28.0));
            let t0: f32 = heights[..first].iter().sum();
            let (a, z) = (t0 / total, ((t0 + list_h) / total).min(1.0));
            let mut m = Mesh::default();
            m.stroke(&[[bx, top], [bx, top + len]], false, d.w(3.0), Ends::Round, hex(G_LINE, 0.12), d.px);
            m.stroke(&[[bx, top + len * a], [bx, top + len * z]], false, d.w(3.0), Ends::Round, hex(G_LINE, 0.6), d.px);
            b.mesh(&m);
        }
    }

    /// A row's control in its 210 design px column (`c0`..`c1`), centred on `mid`.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn pause_control(&self, b: &mut Batch, d: Glass, val: &PauseVal, (c0, c1): (f32, f32), mid: f32, on: bool, a: f32) {
        let cc = (c0 + c1) * 0.5;
        if let PauseVal::Device(v) = val {
            self.device_control(b, d, v, c1, mid, on, a);
            return;
        }
        let mut m = Mesh::default();
        match val {
            PauseVal::Choice(list, cur) => {
                // < VALUE >: the selector's glass triangles, dimmed at the ends (no wrap)
                Self::choice_arrows(&mut m, d, (c0, c1), mid, on, a, (*cur == 0, *cur + 1 >= list.len()));
                b.mesh(&m);
                let text = list[(*cur).min(list.len() - 1)].to_uppercase();
                let st = d.font(HudFont::GlassMed, 17.0, 0.5, hex(0xffffff, a));
                let w = b.font_width(HudFont::GlassMed, &text, &st) - st.tracking;
                b.font_text(HudFont::GlassMed, &text, cc - w * 0.5, d.cap_top(HudFont::GlassMed, 17.0, mid - d.d(10.0), Some(20.0)), &st);
            }
            PauseVal::Swatch { rgb, name, first, last } => {
                Self::choice_arrows(&mut m, d, (c0, c1), mid, on, a, (*first, *last));
                // `.chip`: 26 x 16 with a white ring, then the name, the pair centred
                let st = d.font(HudFont::GlassMed, 17.0, 0.5, hex(0xffffff, a));
                let text = name.to_uppercase();
                let tw = b.font_width(HudFont::GlassMed, &text, &st) - st.tracking;
                let (cw, ch) = (d.d(26.0), d.d(16.0));
                let x = cc - (cw + d.d(8.0) + tw) * 0.5;
                let chip = vec::rounded_rect(x, mid - ch * 0.5, cw, ch, d.d(4.0));
                m.halo(&chip, d.d(8.0), [0.0, 0.0, 0.0, 0.6 * a]);
                m.fill(&chip, [rgb[0] as f32 / 255.0, rgb[1] as f32 / 255.0, rgb[2] as f32 / 255.0, a], d.px);
                m.stroke(&chip, true, d.w(1.0), Ends::Round, [1.0, 1.0, 1.0, 0.55 * a], d.px);
                b.mesh(&m);
                b.font_text(HudFont::GlassMed, &text, x + cw + d.d(8.0), d.cap_top(HudFont::GlassMed, 17.0, mid - d.d(10.0), Some(20.0)), &st);
            }
            PauseVal::Text { value, max, editing } => {
                // `.tfield`: a 26 px field, white value or `--`, the counter, the caret and the orange rim while typed into
                let (x, h) = (c0, d.d(26.0));
                let r = vec::rounded_rect(x, mid - h * 0.5, c1 - c0, h, d.d(7.0));
                if *editing {
                    m.halo(&r, d.d(10.0), hex(G_ORANGE, 0.35 * a));
                    m.fill(&r, hex(G_ORANGE, 0.08 * a), d.px);
                } else {
                    m.fill(&r, [2.0 / 255.0, 8.0 / 255.0, 18.0 / 255.0, 0.35 * a], d.px);
                }
                m.stroke(&r, true, d.w(1.0), Ends::Round, if *editing { hex(G_ORANGE, a) } else { hex(G_LINE, 0.18 * a) }, d.px);
                // the inset shadow along the top edge
                m.stroke(&[[x + d.d(6.0), mid - h * 0.5 + d.d(1.5)], [c1 - d.d(6.0), mid - h * 0.5 + d.d(1.5)]], false, d.w(2.0), Ends::Butt, [0.0, 0.0, 0.0, 0.3 * a], d.px);
                b.mesh(&m);
                let (pad, gap) = (d.d(10.0), d.d(8.0));
                let cnt = d.font(HudFont::GlassMed, 11.0, 1.0, hex(G_DIM, a));
                let count = if *max <= 15 { format!("{}/{}", value.chars().count(), max) } else { String::new() };
                let cw = if count.is_empty() { 0.0 } else { b.font_width(HudFont::GlassMed, &count, &cnt) - cnt.tracking };
                if !count.is_empty() {
                    b.font_text(HudFont::GlassMed, &count, c1 - pad - cw, d.cap_top(HudFont::GlassMed, 11.0, mid - h * 0.5, Some(26.0)), &cnt);
                }
                let room = c1 - c0 - 2.0 * pad - if count.is_empty() { 0.0 } else { cw + gap } - if *editing { d.d(4.0) } else { 0.0 };
                let st = d.font(HudFont::GlassMed, 16.0, 1.0, hex(0xffffff, a));
                let ty = d.cap_top(HudFont::GlassMed, 16.0, mid - h * 0.5, Some(26.0));
                let shown = if value.is_empty() && !*editing { "--".to_string() } else { fit_text(b, HudFont::GlassMed, &st, &value.to_uppercase(), room, *editing) };
                let st = if value.is_empty() { FontStyle { color: [191.0 / 255.0, 227.0 / 255.0, 243.0 / 255.0, 0.35 * a], ..st } } else { st };
                let tw = b.font_text(HudFont::GlassMed, &shown, x + pad, ty, &st);
                if *editing && ((self.t * 1.0).fract() < 0.5) {
                    // `.caret`: 2 x 17, orange with a glow, blinking once a second
                    let caret = vec::rounded_rect(x + pad + tw + d.d(2.0), mid - d.d(8.5), d.d(2.0), d.d(17.0), d.d(1.0));
                    let mut m = Mesh::default();
                    m.halo(&caret, d.d(6.0), hex(G_ORANGE, 0.9));
                    m.fill(&caret, hex(G_ORANGE, 1.0), d.px);
                    b.mesh(&m);
                }
            }
            PauseVal::Switch(lit) => {
                // 19.4: the check box, lit cyan when on; ON / OFF beside it
                let (bx, by) = (cc - d.d(11.0), mid - d.d(11.0));
                check_box(&mut m, d, (bx, by), (22.0, 6.0), *lit, a);
                b.mesh(&m);
                let st = d.font(HudFont::GlassMed, 15.0, 0.5, hex(G_DIM, a));
                b.font_text(HudFont::GlassMed, if *lit { "ON" } else { "OFF" }, bx - d.d(10.0 + 34.0), d.cap_top(HudFont::GlassMed, 15.0, mid - d.d(10.0), Some(20.0)), &st);
            }
            PauseVal::Slider(f, text) => {
                // blue fill with glow and a knob, orange on the selected row; the value after it
                let (t0, t1) = (c0, c1 - d.d(12.0 + 42.0));
                let kx = t0 + (t1 - t0) * f.clamp(0.0, 1.0);
                let track = vec::rounded_rect(t0, mid - d.d(2.0), t1 - t0, d.d(4.0), d.d(2.0));
                m.fill(&track, hex(G_LINE, 0.18 * a), d.px);
                if kx > t0 + d.d(1.0) {
                    let fill = vec::rounded_rect(t0, mid - d.d(2.0), kx - t0, d.d(4.0), d.d(2.0));
                    m.halo(&fill, d.d(6.0), hex(0x4aa8ff, 0.8 * a));
                    m.fill(&fill, hex(G_S1, a), d.px);
                }
                let knob = vec::arc([kx, mid], d.d(6.0), 0.0, TAU, 24);
                let ring = vec::arc([kx, mid], d.d(8.0), 0.0, TAU, 24);
                m.halo(&ring, d.d(8.0), if on { hex(G_ORANGE, 0.9 * a) } else { hex(0x6fcbff, 0.8 * a) });
                m.fill(&ring, hex(G_DARK, 0.8 * a), d.px);
                m.fill(&knob, if on { hex(G_ORANGE, a) } else { hex(0xf4fbff, a) }, d.px);
                b.mesh(&m);
                let st = d.font(HudFont::GlassMed, 15.0, 0.0, hex(0xffffff, a));
                let w = b.font_width(HudFont::GlassMed, text, &st) - st.tracking;
                b.font_text(HudFont::GlassMed, text, c1 - w, d.cap_top(HudFont::GlassMed, 15.0, mid - d.d(10.0), Some(20.0)), &st);
            }
            PauseVal::Device(_) => {}
            PauseVal::Bind(caps) => {
                let h = d.d(22.0);
                let (kw, pw) = (self.cap_width(b, d, &caps.key, h, caps.waiting), self.cap_width(b, d, &caps.pad, h, caps.waiting));
                let x = cc - (kw + pw + d.d(10.0)) * 0.5;
                self.draw_cap(b, d, &caps.key, (x, mid, kw, h), false, caps.waiting, a, on);
                self.draw_cap(b, d, &caps.pad, (x + kw + d.d(10.0), mid, pw, h), true, caps.waiting, if caps.pad_dim { a * 0.35 } else { a }, on);
            }
        }
    }

    /// The selector's glass triangles round a control column (`c0`..`c1`): `ends` = (first, last) dims
    /// the arrow that has nowhere to go (no wrap).
    #[allow(clippy::too_many_arguments)]
    pub(super) fn choice_arrows(m: &mut Mesh, d: Glass, (c0, c1): (f32, f32), mid: f32, on: bool, a: f32, ends: (bool, bool)) {
        Self::glass_tri(m, d, c0, mid, true, on, a, ends.0);
        Self::glass_tri(m, d, c1 - d.d(9.0), mid, false, on, a, ends.1);
    }

    /// One of the selector's glass triangles (`.tri`, 9 x 11 design px) with its left edge at canvas `x`, centred on
    /// `mid`; `on` = the selected row's orange with a glow, `end` = dimmed to .2 (nowhere to go).
    #[allow(clippy::too_many_arguments)]
    pub(super) fn glass_tri(m: &mut Mesh, d: Glass, x: f32, mid: f32, left: bool, on: bool, a: f32, end: bool) {
        let accent = hex(if on { G_ORANGE } else { 0xd8f2ff }, a);
        let th = d.d(11.0);
        let p = |u: f32, v: f32| [x + d.d(u), mid - th * 0.5 + d.d(v)];
        let tri = if left { vec![p(8.0, 1.0), p(1.5, 5.5), p(8.0, 10.0)] } else { vec![p(1.0, 1.0), p(7.5, 5.5), p(1.0, 10.0)] };
        let k = if end { 0.2 } else { 1.0 };
        if on && !end {
            m.halo(&tri, d.d(4.0), hex(G_ORANGE, 0.8 * a));
        }
        m.fill(&tri, [1.0, 1.0, 1.0, 0.14 * a * k], d.px);
        m.stroke(&tri, true, d.w(1.6), Ends::Round, with_a(accent, k), d.px);
    }

    /// The width of a cap: the glyph's, or the text's on a drawn cap (26 design px at least).
    fn cap_width(&self, b: &Batch, d: Glass, cap: &Cap, h: f32, waiting: bool) -> f32 {
        if waiting {
            return d.d(34.0).max(h);
        }
        match cap {
            Cap::Glyph(c, _) => b.font_width(HudFont::Glyphs, &c.to_string(), &FontStyle::new(h * 0.95, WHITE)).max(h),
            Cap::Text(t) => (b.font_width(HudFont::Glass, t, &d.font(HudFont::Glass, cap_font(d, h), 0.5, WHITE)) + d.d(14.0)).max(d.d(26.0)),
        }
    }

    /// One cap centred on `mid` at `x`: the glyph, or the name on a drawn cap, or three dots while
    /// the binding waits. `pill`: the pad's round cap.
    #[allow(clippy::too_many_arguments)]
    fn draw_cap(&self, b: &mut Batch, d: Glass, cap: &Cap, (x, mid, w, h): (f32, f32, f32, f32), pill: bool, waiting: bool, a: f32, on: bool) {
        let mut m = Mesh::default();
        if waiting || matches!(cap, Cap::Text(_)) {
            cap_body(&mut m, d, (x, mid - h * 0.5, w, h), pill, a);
            if on {
                m.stroke(&vec::rounded_rect(x, mid - h * 0.5, w, h, if pill { h * 0.5 } else { d.d(5.0) }), true, d.w(1.0), Ends::Round, hex(G_ORANGE, a), d.px);
            }
        }
        if waiting {
            for k in [-1.0, 0.0, 1.0] {
                m.fill(&vec::arc([x + w * 0.5 + k * h * 0.22, mid], h * 0.07, 0.0, TAU, 12), [1.0, 1.0, 1.0, a], d.px);
            }
            b.mesh(&m);
            return;
        }
        b.mesh(&m);
        match cap {
            Cap::Glyph(c, tint) => {
                let st = FontStyle::new(h * 0.95, with_a(*tint, a));
                let gw = b.font_width(HudFont::Glyphs, &c.to_string(), &st);
                b.font_text(HudFont::Glyphs, &c.to_string(), x + (w - gw) * 0.5, mid - st.size * 0.5, &st);
            }
            Cap::Text(t) => {
                let px = cap_font(d, h);
                let st = d.font(HudFont::Glass, px, 0.5, hex(0xffffff, a));
                let tw = b.font_width(HudFont::Glass, t, &st) - st.tracking;
                b.font_text(HudFont::Glass, t, x + (w - tw) * 0.5, d.cap_top(HudFont::Glass, px, mid - h * 0.5, Some(h / d.d(1.0))), &st);
            }
        }
    }

    /// The info card: the tab's icon, the row's name and its place in the list, the help text and
    /// the row's value.
    #[allow(clippy::too_many_arguments)]
    fn pause_info_card(&self, b: &mut Batch, d: Glass, items: &[PauseItem], sel: usize, tab: &str, icon: char, hud: Option<(&[usize], [bool; 11])>, pulse: f32) {
        let (ix, iy, iw, ih) = INFO_CARD;
        let (x, y) = (d.x(ix), d.y(iy));
        let mut m = Mesh::default();
        glass_card(&mut m, d, (x, y, d.d(iw), d.d(ih)), d.d(12.0), (0.13, 0.04), 0.36, (10.0, 26.0, 0.45), 0.35, None);
        b.mesh(&m);
        let at = row_item(items, sel);
        let PauseItem::Row(row) = &items[at] else { return };
        let head = items[..at].iter().rev().find_map(|i| if let PauseItem::Head(n) = i { Some(n.as_str()) } else { None });
        let n_rows = items.iter().filter(|i| matches!(i, PauseItem::Row(_))).count();
        // the title block beside the icon, centred on each other
        Self::glass_icon(b, icon, x + d.d(20.0), y + d.d(22.5), d.d(30.0), WHITE);
        let title = row.label.to_uppercase();
        b.font_text(HudFont::Glass, &title, x + d.d(62.0), d.cap_top(HudFont::Glass, 25.0, y + d.d(18.0), Some(26.0)), &d.font(HudFont::Glass, 25.0, 0.5, WHITE));
        let caption = format!("{}{} · {} OF {}", tab.to_uppercase(), head.map_or(String::new(), |h| format!(" // {}", h.to_uppercase())), sel + 1, n_rows);
        let cap = d.font(HudFont::GlassMed, 11.0, 1.5, hex(G_DIM, 1.0));
        b.font_text(HudFont::GlassMed, &caption, x + d.d(62.0), d.cap_top(HudFont::GlassMed, 11.0, y + d.d(44.0), None), &cap);
        // the help text, wrapped to 396 design px
        let st = d.font(HudFont::GlassMed, 15.0, 0.5, hex(G_TEXT, 1.0));
        let lines = wrap_text(b, HudFont::GlassMed, &st, &row.help, d.d(396.0));
        for (k, line) in lines.iter().enumerate() {
            b.font_text(HudFont::GlassMed, line, x + d.d(20.0), d.cap_top(HudFont::GlassMed, 15.0, y + d.d(78.0 + 20.0 * k as f32), Some(20.0)), &st);
        }
        // the rule under the help (further down when the help is long), then the value
        let device = matches!(row.val, PauseVal::Device(_));
        let rule = (78.0 + 20.0 * lines.len() as f32 + 6.0).max(if hud.is_some() { 124.0 } else if device { sound_device::RULE_MIN } else { 160.0 });
        let mut m = Mesh::default();
        m.dashed([x + d.d(20.0), y + d.d(rule)], [x + d.d(416.0), y + d.d(rule)], d.d(6.0), d.d(4.0), d.w(1.0), hex(G_LINE, 0.45), d.px);
        b.mesh(&m);
        if let Some((lit, shown)) = hud {
            b.font_text(HudFont::GlassMed, "PREVIEW · LIT: THIS ROW · DASHED: HIDDEN", x + d.d(20.0), d.cap_top(HudFont::GlassMed, 11.0, y + d.d(rule + 8.0), None), &cap);
            // 16:9, 372 design px wide where it fits under a long help text
            let top = rule + 26.0;
            let h = (ih - top - 13.0).min(209.0);
            let w = h * 16.0 / 9.0;
            self.pause_hud_preview(b, d, (x + d.d(32.0 + (372.0 - w) * 0.5), y + d.d(top), d.d(w)), lit, shown);
            return;
        }
        if let PauseVal::Device(v) = &row.val {
            self.device_info(b, d, v, rule);
            return;
        }
        let heading = if matches!(row.val, PauseVal::Bind(_)) { "BOUND TO" } else { "VALUE" };
        b.font_text(HudFont::GlassMed, heading, x + d.d(20.0), d.cap_top(HudFont::GlassMed, 11.0, y + d.d(rule + 10.0), None), &cap);
        let (vx, vy) = (x + d.d(20.0), y + d.d(rule + 32.0));
        match &row.val {
            PauseVal::Choice(list, cur) => {
                let (mut cx, mut cy) = (0.0, 0.0);
                for (k, text) in list.iter().enumerate() {
                    let w = self.chip_width(b, d, text);
                    if cx > 0.0 && cx + w > d.d(396.0) {
                        (cx, cy) = (0.0, cy + d.d(line_h(HudFont::Glass, 15.0) + 10.0 + 8.0));
                    }
                    self.draw_chip(b, d, text, (vx + cx, vy + cy), k == *cur);
                    cx += w + d.d(8.0);
                }
            }
            PauseVal::Switch(on) => {
                let w = self.chip_width(b, d, "OFF");
                self.draw_chip(b, d, "OFF", (vx, vy), !on);
                self.draw_chip(b, d, "ON", (vx + w + d.d(8.0), vy), *on);
            }
            PauseVal::Slider(_, text) => {
                b.font_text(HudFont::Glass, text, vx, d.cap_top(HudFont::Glass, 40.0, vy, Some(44.0)), &d.font(HudFont::Glass, 40.0, 1.0, WHITE));
            }
            PauseVal::Text { .. } | PauseVal::Swatch { .. } | PauseVal::Device(_) => {}
            PauseVal::Bind(caps) => {
                let h = d.d(30.0);
                let mid = vy + h * 0.5;
                let (kw, pw) = (self.cap_width(b, d, &caps.key, h, caps.waiting), self.cap_width(b, d, &caps.pad, h, caps.waiting));
                let mut cx = vx;
                for (label, cap, w, pill, a) in [("KEY", &caps.key, kw, false, 1.0), ("PAD", &caps.pad, pw, true, if caps.pad_dim { 0.35 } else { 1.0 })] {
                    let lw = b.font_width(HudFont::GlassMed, label, &cap_style(d));
                    b.font_text(HudFont::GlassMed, label, cx, d.cap_top(HudFont::GlassMed, 11.0, mid - d.d(7.0), Some(14.0)), &cap_style(d));
                    cx += lw + d.d(12.0);
                    self.draw_cap(b, d, cap, (cx, mid, w, h), pill, caps.waiting, a, false);
                    cx += w + d.d(22.0);
                }
                if caps.waiting {
                    let st = d.font(HudFont::Glass, 15.0, 1.0, hex(G_ORANGE, 0.55 + 0.45 * pulse));
                    b.font_text(HudFont::Glass, "PRESS A KEY OR A BUTTON", vx, d.cap_top(HudFont::Glass, 15.0, vy + d.d(46.0), None), &st);
                }
            }
        }
    }

    /// Which pieces of the race screen the HUD shows with these settings, in `HUD_PIECES` order.
    fn hud_pieces_shown(&self) -> [bool; 11] {
        let s = &self.settings;
        let h = s.hud.shown(s.info_detail);
        [h.cluster, h.clocks[0], h.clocks[1], h.clocks[2], h.lap, h.pos, map_row(s) > 0, h.proximity && s.view.modern_hud(), s.name_plates > 0, s.checkpoint_info > 0 && s.checkpoint_delay > 0 || s.hud.live_gaps && s.checkpoint_info > 0 && s.view.modern_hud(), s.hud.lap_delay > 0 && s.view.modern_hud()]
    }

    /// `hudPreviewHTML`: a 16:9 thumbnail of the race screen, `w` canvas px wide at (x, y), with a box per
    /// HUD piece: orange when `lit` names it (the selected row changes it), solid when shown, dashed when hidden.
    fn pause_hud_preview(&self, b: &mut Batch, d: Glass, (x, y, w): (f32, f32, f32), lit: &[usize], shown: [bool; 11]) {
        let (h, u) = (w * 9.0 / 16.0, w / 1280.0);
        let mut m = Mesh::default();
        let frame = vec::rounded_rect(x, y, w, h, d.d(8.0));
        m.fill(&frame, [4.0 / 255.0, 9.0 / 255.0, 18.0 / 255.0, 0.55], d.px);
        m.stroke(&frame, true, d.w(1.0), Ends::Round, [190.0 / 255.0, 230.0 / 255.0, 1.0, 0.3], d.px);
        b.mesh(&m);
        for (k, &(bx, by, bw, bh, label)) in HUD_PIECES.iter().enumerate() {
            let (hot, on) = (lit.contains(&k), shown[k]);
            let (rx, ry, rw, rh) = (x + bx * u, y + by * u, bw * u, bh * u);
            let r = vec::rounded_rect(rx, ry, rw, rh, d.d(4.0));
            let mut m = Mesh::default();
            if hot {
                m.halo(&r, d.d(10.0), hex(0xff9a3c, 0.5));
                m.fill(&r, hex(0xff9a3c, 0.16), d.px);
            } else if on {
                m.fill(&r, [190.0 / 255.0, 230.0 / 255.0, 1.0, 0.10], d.px);
            }
            let edge = if hot { hex(G_ORANGE, 1.0) } else { [190.0 / 255.0, 230.0 / 255.0, 1.0, if on { 0.55 } else { 0.25 }] };
            if on {
                m.stroke(&r, true, d.w(1.0), Ends::Round, edge, d.px);
            } else {
                let (dash, gap) = (d.d(3.0), d.d(2.0));
                for (a, z) in [([rx, ry], [rx + rw, ry]), ([rx + rw, ry], [rx + rw, ry + rh]), ([rx + rw, ry + rh], [rx, ry + rh]), ([rx, ry + rh], [rx, ry])] {
                    m.dashed(a, z, dash, gap, d.w(1.0), edge, d.px);
                }
            }
            b.mesh(&m);
            let size = (11.0 * u * 2.0 / d.d(1.0)).max(8.0);
            let col = if hot { hex(G_ORANGE, 1.0) } else if on { hex(G_TEXT, 1.0) } else { [191.0 / 255.0, 227.0 / 255.0, 243.0 / 255.0, 0.35] };
            let st = d.font(HudFont::Glass, size, 1.0, col);
            let tw = b.font_width(HudFont::Glass, label, &st) - st.tracking;
            b.font_text(HudFont::Glass, label, rx + (rw - tw) * 0.5, d.cap_top(HudFont::Glass, size, ry + (rh - d.d(size)) * 0.5, None), &st);
        }
    }

    /// `.vchip`'s width (canvas px): the text and the padding and border round it.
    fn chip_width(&self, b: &Batch, d: Glass, text: &str) -> f32 {
        b.font_width(HudFont::Glass, &text.to_uppercase(), &d.font(HudFont::Glass, 15.0, 1.0, WHITE)) + d.d(22.0)
    }

    /// A value chip at its top left; the current one lit sector-1 blue.
    fn draw_chip(&self, b: &mut Batch, d: Glass, text: &str, (x, y): (f32, f32), on: bool) {
        let (w, h) = (self.chip_width(b, d, text), d.d(line_h(HudFont::Glass, 15.0) + 10.0));
        let r = vec::rounded_rect(x, y, w, h, d.d(7.0));
        let mut m = Mesh::default();
        if on {
            m.halo(&r, d.d(10.0), hex(0x50aaff, 0.35));
            m.fill(&r, hex(0x8fd0ff, 0.14), d.px);
        }
        m.stroke(&r, true, d.w(1.0), Ends::Round, if on { hex(0x8fd0ff, 0.8) } else { hex(G_LINE, 0.18) }, d.px);
        b.mesh(&m);
        let st = d.font(HudFont::Glass, 15.0, 1.0, hex(if on { 0xffffff } else { G_DIM }, 1.0));
        b.font_text(HudFont::Glass, &text.to_uppercase(), x + d.d(11.0), d.cap_top(HudFont::Glass, 15.0, y + d.d(5.0), None), &st);
    }

    /// `glass_hints` with each glyph's own device (a Cancel hint shows Esc and the pad's Start).
    pub(super) fn glass_hints_dev(b: &mut Batch, d: Glass, items: &[(&[(Device, usize)], &str)], top: f32) {
        let st = d.font(HudFont::Glass, 23.0, 1.0, hex(0xa9d3e6, 1.0));
        let ty = d.cap_top(HudFont::Glass, 23.0, d.y(top), None);
        let glyph = |(device, kind): (Device, usize)| {
            let (c, tint) = Self::glyph_for(device, kind);
            (c.to_string(), FontStyle::new(d.d(15.0), tint))
        };
        let widths: Vec<f32> = items
            .iter()
            .map(|(kinds, label)| {
                let g: f32 = kinds.iter().map(|&k| { let (c, gs) = glyph(k); b.font_width(HudFont::Glyphs, &c, &gs) + d.d(3.0) }).sum();
                g - d.d(3.0) + d.d(7.0) + b.font_width(HudFont::Glass, label, &st) - st.tracking
            })
            .collect();
        let total = widths.iter().sum::<f32>() + d.d(28.0) * (items.len() - 1) as f32;
        let mut x = d.x(640.0) - total * 0.5;
        for ((kinds, label), w) in items.iter().zip(widths) {
            let x0 = x;
            for (n, &k) in kinds.iter().enumerate() {
                let (c, gs) = glyph(k);
                if n > 0 {
                    x += d.d(3.0);
                }
                x += b.font_text(HudFont::Glyphs, &c, x, ty + st.size * 0.5 - gs.size * 0.5, &gs);
            }
            b.font_text(HudFont::Glass, label, x + d.d(7.0), ty, &st);
            x = x0 + w + d.d(28.0);
        }
    }
}

/// `text` cut to `max_w` canvas px: its head (with `..`) or, while typed into, its tail.
fn fit_text(b: &Batch, f: HudFont, st: &FontStyle, text: &str, max_w: f32, tail: bool) -> String {
    let width = |s: &str| b.font_width(f, s, st) - st.tracking;
    if width(text) <= max_w {
        return text.to_string();
    }
    let chars: Vec<char> = text.chars().collect();
    for n in (0..chars.len()).rev() {
        let s: String = if tail { chars[chars.len() - n..].iter().collect() } else { chars[..n].iter().chain(['.', '.'].iter()).collect() };
        if width(&s) <= max_w {
            return s;
        }
    }
    String::new()
}

/// The font px (design) of a drawn cap `h` canvas px high: 12 in a row (22), 15 in the info card (30).
fn cap_font(d: Glass, h: f32) -> f32 {
    if h > d.d(26.0) { 15.0 } else { 12.0 }
}

/// The pieces of the HUD preview (1280 x 720 units, `hudPreviewHTML`): x, y, width, height, label.
/// The order `GfxRow::hud_lit` and `Game::hud_pieces_shown` use (LAP before POS, though POSITION sits
/// above LAP on screen). REMASTER (#323 phase 6): the boxes follow the Race line HUD's placement, from the
/// constants of `hud_glass.rs`: the cluster's speed, swoosh and dashed rule (x 36..336, y 22..223); the clocks
/// (right edge 1244, 236 wide, from y 22), POSITION (top 134) and LAP (top 208) 120 wide, 58 high; the map disc's tick
/// ring (r 90 round (1143, 585)); the splits' caption and three rows (x 36..286, down to y 684). Design choice:
/// the clock rows (30 high on screen) are drawn 38 high with 4 between, and POSITION and LAP are 54 high a little lower,
/// so the 8 px labels fit the thumbnail's boxes. The arrows and the names are 3D / world labels:
/// their boxes are where they usually are.
const HUD_PIECES: [(f32, f32, f32, f32, &str); 11] = [
    (28.0, 18.0, 316.0, 212.0, "SPEED"),
    (1008.0, 22.0, 236.0, 38.0, "TOTAL"),
    (1008.0, 64.0, 236.0, 38.0, "LAP TIME"),
    (1008.0, 106.0, 236.0, 38.0, "BEST"),
    (1124.0, 216.0, 120.0, 54.0, "LAP"),
    (1124.0, 154.0, 120.0, 54.0, "POS"),
    (1051.0, 495.0, 184.0, 180.0, "MAP"),
    (520.0, 470.0, 240.0, 120.0, "ARROWS"),
    (540.0, 236.0, 200.0, 34.0, "NAMES"),
    (36.0, 590.0, 250.0, 94.0, "SPLITS"),
    (540.0, 40.0, 200.0, 56.0, "LAP TIMES"),
];

/// The 11 px caption style (`.cap`).
fn cap_style(d: Glass) -> FontStyle {
    d.font(HudFont::GlassMed, 11.0, 1.5, hex(G_DIM, 1.0))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_list_starts_at_the_top() {
        assert_eq!(rows_first(&[ROW_H; 9], 8, LIST_H), 0);
    }

    #[test]
    fn long_list_centres_the_selection_and_ends_full() {
        let h = [ROW_H; 15];
        // the first rows need no scrolling
        assert_eq!(rows_first(&h, 0, LIST_H), 0);
        assert_eq!(rows_first(&h, 4, LIST_H), 0);
        // in the middle the selected row is about centred
        assert_eq!(rows_first(&h, 7, LIST_H), 3);
        // the end: 9 whole rows fit, so the window stops at row 6
        assert_eq!(rows_first(&h, 14, LIST_H), 6);
    }

    #[test]
    fn captions_count_in_the_window() {
        let mut h = vec![ROW_H; 11];
        h.extend([HEAD_H, ROW_H, HEAD_H, ROW_H, ROW_H, ROW_H]);
        let first = rows_first(&h, h.len() - 1, LIST_H);
        // the window from `first` reaches the end
        assert!(h[first..].iter().sum::<f32>() <= LIST_H);
        assert!(first > 0 && h[first - 1..].iter().sum::<f32>() > LIST_H);
    }

    #[test]
    fn selection_is_always_in_the_window() {
        let mut h = vec![HEAD_H];
        h.extend([ROW_H; 13]);
        h.push(HEAD_H);
        h.extend([ROW_H; 14]);
        for sel in 0..h.len() {
            let first = rows_first(&h, sel, LIST_H);
            assert!(first <= sel, "first {first} past the selection {sel}");
            assert!(h[first..=sel].iter().sum::<f32>() <= LIST_H, "selection {sel} below the window");
        }
    }
}
