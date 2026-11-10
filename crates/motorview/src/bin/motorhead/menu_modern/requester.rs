//! REMASTER requesters in the Glass HUD (#304 phase 3): concept A's `confirmHTML` of
//! `reference_materials/pause-overhaul/A-rail.html` (`.card.req`, `.btn`, `.scrim` in `assets/glass.css`,
//! shot `shots/A-exit-confirm.jpg`). A near-opaque glass card over a dimmed screen: a warning icon and the
//! question, one line of explanation, two glass buttons with icons (the chosen one with the orange rim and
//! pulse); the hints CHOOSE / CONFIRM / CANCEL replace the screen's own. The text-entry requesters use the
//! same card with a field. Design px through `Glass`, rules in 19-glass-hud-design-language.

use super::pause_settings::{wrap_text, PauseVal};
use super::*;

/// The two-button questions: the Classic lines (the original's strings, 0x4d607a..0x4d60fb) and the
/// Glass text. Keys and results are handled in `Game::key`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Ask {
    Exit,
    DeleteDemo,
    CreateLeague,
    DeleteLeague,
    SaveRace,
    /// the keep-display-settings question with the seconds left before it reverts
    KeepDisplay(u32),
}

impl Ask {
    /// The MENU.SPR requester's two lines.
    pub(crate) fn classic(self) -> [String; 2] {
        let l = |a: &str, b: &str| [a.to_string(), b.to_string()];
        match self {
            Ask::Exit => l("Exit to system.", "Are you sure?"),
            Ask::DeleteDemo => l("Delete demo file.", "Are you sure?"),
            Ask::CreateLeague => l("Create this league.", "Are you sure?"),
            Ask::DeleteLeague => l("Delete this league.", "Are you sure?"),
            Ask::SaveRace => l("Save the recorded race?", ""),
            Ask::KeepDisplay(s) => l("Keep these settings?", &format!("Reverting in {s} s")),
        }
    }

    /// The two buttons (the first is "yes"; Esc picks the second).
    pub(crate) fn buttons(self) -> [&'static str; 2] {
        match self {
            Ask::Exit | Ask::SaveRace => ["Yes, please", "No way!"],
            Ask::DeleteDemo => ["Yes", "NO!"],
            Ask::CreateLeague => ["Yes, keep it!", "No way!"],
            Ask::DeleteLeague => ["Yes, delete", "Keep it!"],
            Ask::KeepDisplay(_) => ["Keep", "Revert"],
        }
    }

    fn title(self) -> &'static str {
        match self {
            Ask::Exit => "Exit to system?",
            Ask::DeleteDemo => "Delete demo file?",
            Ask::CreateLeague => "Create this league?",
            Ask::DeleteLeague => "Delete this league?",
            Ask::SaveRace => "Save the recorded race?",
            Ask::KeepDisplay(_) => "Keep these settings?",
        }
    }

    fn explain(self) -> String {
        match self {
            Ask::Exit => "The game closes and you return to the system.".into(),
            Ask::DeleteDemo => "The recorded race is removed for good.".into(),
            Ask::CreateLeague => "A new league starts under a name of your choice.".into(),
            Ask::DeleteLeague => "The league and its standings are removed for good.".into(),
            Ask::SaveRace => "Keeps a replay of this race that you can watch again.".into(),
            Ask::KeepDisplay(s) => format!("The screen settings changed. They revert in {s} s unless you keep them."),
        }
    }

    /// The icons of the "yes" and "no" buttons.
    fn icons(self) -> [char; 2] {
        let yes = match self {
            Ask::Exit => ui_font::icon::LOG_OUT,
            Ask::DeleteDemo | Ask::DeleteLeague => ui_font::icon::TRASH,
            Ask::SaveRace => ui_font::icon::SAVE,
            Ask::CreateLeague | Ask::KeepDisplay(_) => ui_font::icon::CHECK,
        };
        [yes, if matches!(self, Ask::KeepDisplay(_)) { ui_font::icon::RESTART } else { ui_font::icon::X }]
    }
}

/// `.card.req`: the near-opaque glass of a requester over busy content.
pub(super) fn req_card(m: &mut Mesh, d: Glass, (x, y, w, h): (f32, f32, f32, f32)) {
    let body = vec::rounded_rect(x, y, w, h, d.d(12.0));
    m.shadow(&body, 0.0, d.d(18.0), d.d(48.0), [0.0, 0.0, 0.0, 0.6]);
    m.halo(&body, d.d(40.0), hex(0x50aaff, 0.18));
    m.fill_with(&body, &|q| {
        let t = ((q[1] - y) / h).clamp(0.0, 1.0);
        let (a, b) = ([24.0, 40.0, 62.0], [8.0, 15.0, 28.0]);
        [(a[0] + (b[0] - a[0]) * t) / 255.0, (a[1] + (b[1] - a[1]) * t) / 255.0, (a[2] + (b[2] - a[2]) * t) / 255.0, 0.97]
    }, d.px);
    m.stroke(&body, true, d.w(1.0), Ends::Round, hex(G_LINE, 0.5), d.px);
    m.stroke(&[[x + d.d(12.0), y + d.d(1.5)], [x + w - d.d(12.0), y + d.d(1.5)]], false, d.w(1.0), Ends::Butt, [1.0, 1.0, 1.0, 0.3], d.px);
}

impl Game {
    /// The scrim over the whole screen (`.scrim`).
    pub(super) fn req_scrim(b: &mut Batch) {
        let mut m = Mesh::default();
        m.quad(0.0, 0.0, b.canvas.w, b.canvas.h, [[2.0 / 255.0, 5.0 / 255.0, 10.0 / 255.0, 0.55]; 4]);
        b.mesh(&m);
    }

    /// The question's title row: the icon (the warning triangle for a question, `Some(char)` for another) and the title,
    /// Glass 28 / 1.
    fn req_title(b: &mut Batch, d: Glass, (x, y): (f32, f32), title: &str, icon: Option<char>) {
        let mut tx = x + d.d(22.0);
        if let Some(icon) = icon {
            Self::glass_icon(b, icon, tx, y + d.d(21.0), d.d(26.0), hex(0xffb26a, 1.0));
            tx += d.d(26.0 + 12.0);
        }
        b.font_text(HudFont::Glass, &title.to_uppercase(), tx, d.cap_top(HudFont::Glass, 28.0, y + d.d(18.0), Some(32.0)), &d.font(HudFont::Glass, 28.0, 1.0, WHITE));
    }

    /// The title's width in canvas px.
    fn req_title_w(b: &Batch, d: Glass, title: &str, icon: bool) -> f32 {
        d.d(22.0 + 22.0 + if icon { 26.0 + 12.0 } else { 0.0 }) + b.font_width(HudFont::Glass, &title.to_uppercase(), &d.font(HudFont::Glass, 28.0, 1.0, WHITE)) - d.d(1.0)
    }

    /// REMASTER two-button requester over any screen: the scrim, the card, the question, its
    /// explanation, the two buttons (`sel` is the chosen one) and the hints.
    pub(crate) fn requester_modern(&self, b: &mut Batch, ask: Ask, sel: usize, px: f32) {
        let d = Glass::new(&b.canvas, px);
        let buttons: Vec<(char, String)> = ask.icons().into_iter().zip(ask.buttons()).map(|(i, n)| (i, n.to_string())).collect();
        self.req_dialog(b, d, (ui_font::icon::ALERT, ask.title()), &[(ask.explain(), false)], &buttons, sel, 460.0, &[(&[0, 1], "CHOOSE"), (&[2], "CONFIRM"), (&[3], "CANCEL")]);
    }

    /// The requester's card centred over the screen (`.card.req`): the scrim, the `title` with its icon, the text
    /// (`runs` of (text, bold): bold is `.rx b`, Rajdhani Bold in white) wrapped to the card, the buttons centred under it
    /// (`sel` with the orange rim and pulse; one button or more) and `hints` in place of the screen's. The card is `min_w`
    /// design px wide or wider for a long title or label. Ask's questions and Lap Analysis's refusals (#429) draw here.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn req_dialog(&self, b: &mut Batch, d: Glass, (title_icon, title): (char, &str), runs: &[(String, bool)], buttons: &[(char, String)], sel: usize, min_w: f32, hints: &[(&[usize], &str)]) {
        let pulse = 0.5 - 0.5 * (self.t * TAU / 1.1).cos();
        let names: Vec<String> = buttons.iter().map(|(_, n)| n.to_uppercase()).collect();
        let nb = names.len().max(1) as f32;
        let bst = d.font(HudFont::Glass, 22.0, 1.0, WHITE);
        // the buttons: 200 design px, wider for a long label (icon 20, 10, text, 18 each side)
        let tw: Vec<f32> = names.iter().map(|n| b.font_width(HudFont::Glass, n, &bst) - bst.tracking).collect();
        let bw = d.d(200.0).max(tw.iter().cloned().fold(0.0, f32::max) + d.d(20.0 + 10.0 + 36.0));
        let gap = d.d(16.0);
        let w = d.d(min_w).max(Self::req_title_w(b, d, title, true)).max(nb * bw + (nb - 1.0) * gap + d.d(44.0));
        let st = d.font(HudFont::GlassMed, 15.0, 0.5, hex(G_TEXT, 1.0));
        let bold = d.font(HudFont::Glass, 15.0, 0.5, WHITE);
        let lines: Vec<Vec<(String, bool)>> = if runs.iter().all(|r| !r.1) {
            let text: String = runs.iter().map(|r| r.0.as_str()).collect();
            wrap_text(b, HudFont::GlassMed, &st, &text, w - d.d(44.0)).into_iter().map(|l| vec![(l, false)]).collect()
        } else {
            wrap_runs(b, (&st, &bold), runs, w - d.d(44.0))
        };
        let n = lines.len() as f32;
        let h = d.d(60.0 + 20.0 * n + 26.0 + 44.0 + 24.0);
        let (x, y) = (d.x(640.0) - w * 0.5, d.y(353.0) - h * 0.5);
        Self::req_scrim(b);
        let mut m = Mesh::default();
        req_card(&mut m, d, (x, y, w, h));
        b.mesh(&m);
        Self::req_title(b, d, (x, y), title, Some(title_icon));
        for (k, line) in lines.iter().enumerate() {
            let top = y + d.d(60.0 + 20.0 * k as f32);
            let mut lx = x + d.d(22.0);
            for (s, strong) in line {
                let (f, sty) = if *strong { (HudFont::Glass, &bold) } else { (HudFont::GlassMed, &st) };
                lx += b.font_text(f, s, lx, d.cap_top(f, 15.0, top, Some(20.0)), sty);
            }
        }
        // the buttons, centred under the text
        let (by, bh) = (y + d.d(60.0 + 20.0 * n + 26.0), d.d(44.0));
        let bx0 = x + (w - nb * bw - (nb - 1.0) * gap) * 0.5;
        for (i, (name, (icon, _))) in names.iter().zip(buttons).enumerate() {
            let on = i == sel;
            let bx = bx0 + i as f32 * (bw + gap);
            let r = vec::rounded_rect(bx, by, bw, bh, d.d(12.0));
            let mut m = Mesh::default();
            if on {
                m.halo(&r, d.d(14.0), hex(G_ORANGE, 0.3 + 0.2 * pulse));
            }
            m.fill_with(&r, &|q| [1.0, 1.0, 1.0, 0.14 - 0.10 * ((q[1] - by) / bh).clamp(0.0, 1.0)], d.px);
            if on {
                m.fill(&r, hex(G_ORANGE, 0.06 + 0.06 * pulse), d.px);
            }
            m.stroke(&r, true, d.w(1.0), Ends::Round, if on { hex(G_ORANGE, 1.0) } else { hex(G_LINE, 0.38) }, d.px);
            m.stroke(&[[bx + d.d(12.0), by + d.d(1.5)], [bx + bw - d.d(12.0), by + d.d(1.5)]], false, d.w(1.0), Ends::Butt, [1.0, 1.0, 1.0, 0.38], d.px);
            b.mesh(&m);
            let col = hex(if on { G_ORANGE } else { G_TEXT }, 1.0);
            let ix = bx + (bw - (d.d(30.0) + tw[i])) * 0.5;
            Self::glass_icon(b, *icon, ix, by + d.d(12.0), d.d(20.0), col);
            b.font_text(HudFont::Glass, name, ix + d.d(30.0), d.cap_top(HudFont::Glass, 22.0, by, Some(44.0)), &d.font(HudFont::Glass, 22.0, 1.0, col));
        }
        Self::glass_hints(b, d, self.controls.last, hints, 626.0);
    }

    /// REMASTER text entry requester (league name, recorded race name): the requester's card with a
    /// field holding the text as typed, a blinking caret and `hints` (glyph kinds and labels) below.
    pub(crate) fn requester_input_modern(&self, b: &mut Batch, title: &str, text: &str, hints: &[(&[usize], &str)], px: f32) {
        let d = Glass::new(&b.canvas, px);
        let w = d.d(460.0).max(Self::req_title_w(b, d, title, false));
        let h = d.d(66.0 + 40.0 + 24.0);
        let (x, y) = (d.x(640.0) - w * 0.5, d.y(353.0) - h * 0.5);
        Self::req_scrim(b);
        let mut m = Mesh::default();
        req_card(&mut m, d, (x, y, w, h));
        let (fx, fy, fw, fh) = (x + d.d(22.0), y + d.d(66.0), w - d.d(44.0), d.d(40.0));
        let field = vec::rounded_rect(fx, fy, fw, fh, d.d(8.0));
        m.fill(&field, [1.0, 1.0, 1.0, 0.08], d.px);
        m.stroke(&field, true, d.w(1.0), Ends::Round, hex(G_LINE, 0.5), d.px);
        b.mesh(&m);
        Self::req_title(b, d, (x, y), title, None);
        let st = d.font(HudFont::Glass, 20.0, 0.5, WHITE);
        let tx = fx + d.d(14.0);
        b.font_text(HudFont::Glass, text, tx, d.cap_top(HudFont::Glass, 20.0, fy, Some(40.0)), &st);
        // the caret blinks at 1 Hz after the text
        if ((self.clock * 2.0) as u32).is_multiple_of(2) {
            let tw = if text.is_empty() { 0.0 } else { b.font_width(HudFont::Glass, text, &st) - st.tracking };
            let cx = tx + tw + d.d(3.0);
            let mut m = Mesh::default();
            m.stroke(&[[cx, fy + d.d(9.0)], [cx, fy + fh - d.d(9.0)]], false, d.w(2.0), Ends::Butt, hex(G_ORANGE, 1.0), d.px);
            b.mesh(&m);
        }
        Self::glass_hints(b, d, self.controls.last, hints, 626.0);
    }
}

// ---------------------------------------------------------------- the pad keyboard (#91 phase 4)
// `OSK` / `oskKey` / `oskHTML` of `reference_materials/personal-overhaul/assets/personal.js`, `.osk` in
// `personal.css`, shot `shots/C-name-pad.jpg`: a requester card with the field on top and the keys below.

/// The 40 characters of the grid, 10 to a row.
const OSK_CHARS: &str = "ABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789-.!?";
const OSK_COLS: usize = 10;
/// The keys after the characters: SPACE, DEL and OK fill the fifth row (3, 3 and 4 columns wide).
const OSK_SPACE: usize = 40;
const OSK_DEL: usize = 41;
const OSK_OK: usize = 42;
const OSK_KEYS: usize = 43;

/// The first column, the width in columns and the row of key `k`.
fn osk_cell(k: usize) -> (usize, usize, usize) {
    match k {
        OSK_SPACE => (0, 3, 4),
        OSK_DEL => (3, 3, 4),
        OSK_OK => (6, 4, 4),
        _ => (k % OSK_COLS, 1, k / OSK_COLS),
    }
}

/// Where an arrow takes the cursor from key `k`; None at the edge. Left / Right run on through the list; Up / Down
/// keep the column, and the wide keys of the last row are entered by the column they sit under and left by their
/// first column [design choice: the mockup steps ±10 through the flat list, which puts OK under `4` and `7` alike].
fn osk_move(k: usize, key: KeyCode) -> Option<usize> {
    let (col, _, row) = osk_cell(k);
    match key {
        KeyCode::ArrowLeft => k.checked_sub(1),
        KeyCode::ArrowRight => Some(k + 1).filter(|&n| n < OSK_KEYS),
        KeyCode::ArrowUp => match row {
            0 => None,
            _ => Some((row - 1) * OSK_COLS + col),
        },
        KeyCode::ArrowDown => match row {
            0..=2 => Some(k + OSK_COLS),
            3 => Some(OSK_SPACE + [0, 0, 0, 1, 1, 1, 2, 2, 2, 2][col]),
            _ => None,
        },
        _ => None,
    }
}

/// What the sheet types into: its title and icon, the text so far, the limit, and the labels of B and Start
/// (the Personal rows undo / keep; the league name falls back to a random name; the recorded race is dropped / saved).
struct OskTarget {
    title: String,
    icon: char,
    value: String,
    max: usize,
    undo: &'static str,
    keep: &'static str,
}

impl OskTarget {
    fn new(title: impl Into<String>, icon: char, value: &str, max: usize) -> Self {
        OskTarget { title: title.into(), icon, value: value.to_string(), max, undo: "UNDO", keep: "KEEP" }
    }
}

impl Game {
    /// Open the sheet for the field just opened, when a pad (not the keyboard) was the last device used.
    pub(crate) fn osk_open(&mut self) {
        self.osk = (self.modern_requester() && self.controls.last != Device::Keyboard).then_some(0);
    }

    /// The field the sheet types into: the recorded race's name, the league name, or a Personal row.
    fn osk_target(&self) -> Option<OskTarget> {
        if let Some(DemoSave { name: Some(n), .. }) = &self.demo_save {
            return Some(OskTarget { undo: "DON'T SAVE", keep: "SAVE", ..OskTarget::new("Recorded race", ui_font::icon::FILM, n, DEMO_NAME_MAX) });
        }
        let (row, _) = self.editing.as_ref()?;
        let s = &self.settings;
        Some(match *row {
            0 => OskTarget::new("Name", ui_font::icon::USER, &s.player_name, NAME_MAX),
            1 => OskTarget::new("Team", ui_font::icon::FLAG, &s.team, TEAM_MAX),
            2 => OskTarget { undo: "RANDOM", keep: "OK", ..OskTarget::new("League name", ui_font::icon::TROPHY, &self.league_name, LEAGUE_NAME_MAX) },
            r @ 3..=12 => OskTarget::new(format!("Key {}", r - 3), ui_font::icon::MESSAGE_SQUARE, &s.short_keys[r - 3], SHORT_KEY_MAX),
            _ => return None,
        })
    }

    /// An editing key for whichever field the sheet types into.
    fn osk_edit(&mut self, k: KeyCode) {
        if self.demo_save.as_ref().is_some_and(|d| d.name.is_some()) {
            self.demo_name_key(k);
        } else {
            self.edit_key(k);
        }
    }

    /// The sheet over the screen while a pad types into a row of the Personal screen.
    pub(crate) fn osk_layer(&self, size: (u32, u32)) -> Option<Layer> {
        let k = self.osk?;
        let target = self.osk_target()?;
        let canvas = Canvas::hud(size);
        let px = 1.0 / canvas.transform(size).0;
        Some(self.ui_in(Batch::sharp_text(&self.assets, canvas, 1.0), |g, b| g.osk_modern(b, &target, k, px)))
    }

    /// The characters in the field being typed into.
    fn osk_len(&self) -> usize {
        self.osk_target().map_or(0, |t| t.value.chars().count())
    }

    /// A key while the sheet is open: the arrows move the cursor, A types its key, X deletes, Y adds a space,
    /// B undoes (Start keeps, `osk_keep`).
    pub(crate) fn osk_key(&mut self, k: KeyCode) {
        let Some(at) = self.osk else { return };
        match k {
            KeyCode::ArrowLeft | KeyCode::ArrowRight | KeyCode::ArrowUp | KeyCode::ArrowDown => match osk_move(at, k) {
                Some(to) => {
                    self.osk = Some(to);
                    self.sfx.push(Sfx::Menu(if matches!(k, KeyCode::ArrowUp | KeyCode::ArrowLeft) { ev::CURSOR_UP } else { ev::CURSOR_DOWN }));
                }
                None => self.sfx.push(Sfx::Menu(ev::CURSOR_END)),
            },
            KeyCode::Enter | KeyCode::NumpadEnter => self.osk_press(at),
            KeyCode::F6 => self.osk_press(OSK_DEL),
            KeyCode::F5 => self.osk_press(OSK_SPACE),
            KeyCode::Escape => self.osk_edit(KeyCode::Escape),
            _ => {}
        }
    }

    /// Press key `at`: a character, a space or a delete changes the field (nothing to change = the error sound);
    /// OK keeps the text.
    fn osk_press(&mut self, at: usize) {
        if at == OSK_OK {
            return self.osk_edit(KeyCode::Enter);
        }
        let before = self.osk_len();
        match at {
            OSK_DEL => self.osk_edit(KeyCode::Backspace),
            OSK_SPACE => self.type_text(" "),
            c => self.type_text(&OSK_CHARS[c..=c]),
        }
        self.sfx.push(Sfx::Menu(if self.osk_len() == before { ev::SELECT_ERROR } else { ev::SELECT }));
    }

    /// Start: keep the text and close the sheet.
    pub(crate) fn osk_keep(&mut self) {
        self.osk_edit(KeyCode::Enter);
        if !self.editing_text() {
            self.osk = None;
        }
    }

    /// REMASTER the pad keyboard sheet: the scrim, a 520 x 330 requester card at (380, 206) with the row's icon and
    /// title, its field (the caret, the counter) and the 43 keys, the one under the cursor ringed in orange, then the
    /// pad's hints at four fifths of their size.
    fn osk_modern(&self, b: &mut Batch, t: &OskTarget, at: usize, px: f32) {
        let OskTarget { title, icon, value, max, undo, keep } = t;
        let (icon, max) = (*icon, *max);
        let d = Glass::new(&b.canvas, px);
        let pulse = 0.5 - 0.5 * (self.t * TAU / 1.1).cos();
        let (cx, cy) = (380.0, 206.0);
        Self::req_scrim(b);
        let mut m = Mesh::default();
        req_card(&mut m, d, (d.x(cx), d.y(cy), d.d(520.0), d.d(330.0)));
        b.mesh(&m);
        // `.lbl25` beside a 24 px icon, both white
        Self::glass_icon(b, icon, d.x(cx + 22.0), d.y(cy + 17.0), d.d(24.0), WHITE);
        b.font_text(HudFont::Glass, &title.to_uppercase(), d.x(cx + 22.0 + 24.0 + 12.0), d.cap_top(HudFont::Glass, 25.0, d.y(cy + 16.0), Some(26.0)), &d.font(HudFont::Glass, 25.0, 0.5, WHITE));
        // the field: the Driver tab's `.tfield` while typed into, 476 wide
        let val = PauseVal::Text { value: value.clone(), max, editing: true };
        self.pause_control(b, d, &val, (d.x(cx + 22.0), d.x(cx + 22.0 + 476.0)), d.y(cy + 58.0 + 13.0), false, 1.0);
        // the keys: 10 columns 6 apart, 30 high
        let (gap, kw) = (6.0, (476.0 - 9.0 * 6.0) / 10.0);
        for k in 0..OSK_KEYS {
            let (col, span, row) = osk_cell(k);
            let on = k == at;
            let (x, y) = (d.x(cx + 22.0 + col as f32 * (kw + gap)), d.y(cy + 106.0 + row as f32 * 36.0));
            let (w, h) = (d.d(span as f32 * kw + (span - 1) as f32 * gap), d.d(30.0));
            let r = vec::rounded_rect(x, y, w, h, d.d(7.0));
            let mut m = Mesh::default();
            if on {
                m.halo(&r, d.d(14.0), hex(G_ORANGE, 0.3 + 0.2 * pulse));
            }
            m.fill_with(&r, &|q| [1.0, 1.0, 1.0, 0.12 - 0.09 * ((q[1] - y) / h).clamp(0.0, 1.0)], d.px);
            m.stroke(&r, true, d.w(1.0), Ends::Round, if on { hex(G_ORANGE, 1.0) } else { hex(G_LINE, 0.3) }, d.px);
            b.mesh(&m);
            let col = hex(if on { G_ORANGE } else { G_TEXT }, 1.0);
            if k < OSK_SPACE {
                let st = d.font(HudFont::Glass, 16.0, 0.5, col);
                let s = &OSK_CHARS[k..=k];
                let sw = b.font_width(HudFont::Glass, s, &st) - st.tracking;
                b.font_text(HudFont::Glass, s, x + (w - sw) * 0.5, d.cap_top(HudFont::Glass, 16.0, y, Some(30.0)), &st);
            } else {
                // SPACE, DEL (with its icon) and OK: 14 px, letter-spaced
                let st = d.font(HudFont::Glass, 14.0, 1.5, col);
                let label = ["SPACE", "DEL", "OK"][k - OSK_SPACE];
                let lw = b.font_width(HudFont::Glass, label, &st) - st.tracking;
                let ic = if k == OSK_DEL { d.d(16.0 + 6.0) } else { 0.0 };
                let lx = x + (w - lw - ic) * 0.5;
                if k == OSK_DEL {
                    Self::glass_icon(b, ui_font::icon::DELETE, lx, y + d.d(7.0), d.d(16.0), col);
                }
                b.font_text(HudFont::Glass, label, lx + ic, d.cap_top(HudFont::Glass, 14.0, y, Some(30.0)), &st);
            }
        }
        // A type, X delete, Y space, B undo (or the field's own), Menu keep
        Self::glass_hints_at(b, d, self.pad_device(), &[(&[2], "TYPE"), (&[7], "DELETE"), (&[6], "SPACE"), (&[3], *undo), (&[8], *keep)], (640.0, cy + 296.0), 0.8, 20.0);
    }
}

/// `runs` of (text, bold) wrapped to `max_w` canvas px (`wrap_text` with the `<b>` parts of a requester's text in their
/// own face): the lines as runs; a word that spans runs ("36.820" then ",") stays whole.
fn wrap_runs(b: &Batch, (st, bold): (&FontStyle, &FontStyle), runs: &[(String, bool)], max_w: f32) -> Vec<Vec<(String, bool)>> {
    let font = |strong: bool| if strong { (HudFont::Glass, bold) } else { (HudFont::GlassMed, st) };
    let width = |segs: &[(String, bool)]| segs.iter().map(|(s, strong)| { let (f, sty) = font(*strong); b.font_width(f, s, sty) }).sum::<f32>() - st.tracking;
    // words as segments of runs, split at spaces
    let mut words: Vec<Vec<(String, bool)>> = vec![Vec::new()];
    for (text, strong) in runs {
        for (i, part) in text.split(' ').enumerate() {
            if i > 0 {
                words.push(Vec::new());
            }
            if !part.is_empty() {
                words.last_mut().unwrap().push((part.to_string(), *strong));
            }
        }
    }
    words.retain(|w| !w.is_empty());
    let mut lines: Vec<Vec<(String, bool)>> = vec![Vec::new()];
    for word in words {
        let line = lines.last_mut().unwrap();
        let mut next = line.clone();
        let mut word = word;
        if !next.is_empty() {
            word[0].0.insert(0, ' ');
        }
        next.extend(word.iter().cloned());
        if !line.is_empty() && width(&next) > max_w {
            word[0].0.remove(0);
            lines.push(word);
        } else {
            *line = next;
        }
    }
    // neighbours in the same face as one run
    lines.into_iter().map(|l| l.into_iter().fold(Vec::<(String, bool)>::new(), |mut acc, (s, strong)| {
        match acc.last_mut() {
            Some(last) if last.1 == strong => last.0.push_str(&s),
            _ => acc.push((s, strong)),
        }
        acc
    })).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The arrows walk the 4 x 10 grid and the three wide keys under it; the edges stop.
    #[test]
    fn osk_cursor_moves_over_the_grid_and_the_wide_keys() {
        assert_eq!(osk_move(0, KeyCode::ArrowLeft), None);
        assert_eq!(osk_move(0, KeyCode::ArrowUp), None);
        assert_eq!(osk_move(9, KeyCode::ArrowRight), Some(10), "Right runs on into the next row");
        assert_eq!(osk_move(18, KeyCode::ArrowDown), Some(28));
        assert_eq!(osk_move(12, KeyCode::ArrowUp), Some(2));
        // the last character row meets SPACE (columns 0-2), DEL (3-5) and OK (6-9)
        let under = |c: usize| osk_move(30 + c, KeyCode::ArrowDown);
        assert_eq!([under(0), under(2), under(3), under(5), under(6), under(9)], [Some(40), Some(40), Some(41), Some(41), Some(42), Some(42)]);
        // and leaves them by their first column
        assert_eq!([40, 41, 42].map(|k| osk_move(k, KeyCode::ArrowUp)), [Some(30), Some(33), Some(36)]);
        assert_eq!(osk_move(41, KeyCode::ArrowDown), None);
        assert_eq!(osk_move(40, KeyCode::ArrowRight), Some(41));
        assert_eq!(osk_move(OSK_OK, KeyCode::ArrowRight), None);
        assert_eq!(osk_move(39, KeyCode::ArrowRight), Some(40));
    }

    /// Every key has a cell inside the 10 columns and 5 rows.
    #[test]
    fn osk_cells_fill_the_sheet() {
        for k in 0..OSK_KEYS {
            let (col, span, row) = osk_cell(k);
            assert!(col + span <= OSK_COLS && row < 5, "key {k}");
        }
        assert_eq!(OSK_CHARS.len(), OSK_SPACE);
    }
}
