//! Control layouts (`Game/*.clo`) and the devices behind them: keyboard, mouse, and
//! gamepads / joysticks / wheels through gilrs (02-architecture §Input).
//!
//! A layout binds each task to one control code from the list at the end of every .clo
//! file: a DirectInput key name, `JoyLeft/Right/Up/Down/In/Out`, `JoyButton1..8`,
//! `MouseLeft/Right/Up/Down` or `MouseButtonLeft/Right/Middle`.

use std::collections::HashSet;
use std::path::Path;
use std::time::{Duration, Instant};
use winit::keyboard::KeyCode;

/// The car tasks the controls screen lets you change, then the fixed in-game tasks
/// (the two groups of every .clo file), in file order.
pub const TASKS: [&str; 20] = [
    "TurnLeft",
    "TurnRight",
    "Accelerate",
    "Brake",
    "ShiftGearUp",
    "ShiftGearDown",
    "Horn",
    "HandBrake",
    "RearView",
    // ours: not in any shipped .clo (defaults to R, see `Layout::parse`)
    "Respawn",
    // ours: cycles the camera views (V by default)
    "ChangeCamera",
    "Info",
    "ChatMode",
    "InCarView",
    "OutCarView",
    "CameraView",
    "RearMirror",
    "CyclePlayers",
    "ScreenShot",
    "GameMenu",
];
/// How many of `TASKS` are car tasks (changeable in the menu).
pub const CAR_TASKS: usize = 11;
pub const TASK_LABELS: [&str; CAR_TASKS] = ["Turn left", "Turn right", "Accelerate", "Brake", "Gear up", "Gear down", "Horn", "Hand brake", "Rear view", "Respawn", "Change camera"];

/// Indices into `TASKS`.
pub mod task {
    pub const TURN_LEFT: usize = 0;
    pub const TURN_RIGHT: usize = 1;
    pub const ACCELERATE: usize = 2;
    pub const BRAKE: usize = 3;
    pub const GEAR_UP: usize = 4;
    pub const GEAR_DOWN: usize = 5;
    pub const HORN: usize = 6;
    pub const HANDBRAKE: usize = 7;
    pub const REAR_VIEW: usize = 8;
    pub const RESPAWN: usize = 9;
    pub const CHANGE_CAMERA: usize = 10;
    pub const INFO: usize = 11;
    pub const IN_CAR_VIEW: usize = 13;
    pub const OUT_CAR_VIEW: usize = 14;
    pub const CAMERA_VIEW: usize = 15;
    pub const REAR_MIRROR: usize = 16;
    pub const CYCLE_PLAYERS: usize = 17;
    pub const SCREENSHOT: usize = 18;
    pub const GAME_MENU: usize = 19;
}

/// The layout in use when settings.cfg names none (the shipped Keyboard.clo).
pub const DEFAULT_LAYOUT_NAME: &str = "Keyboard";

/// The shipped Keyboard.clo, used when the install has no layouts.
const DEFAULT_LAYOUT: &str = "LayoutName Keyboard\nTurnLeft LeftArrow\nTurnRight RightArrow\nAccelerate UpArrow\nBrake DownArrow\n\
    ShiftGearUp S\nShiftGearDown A\nHorn Space\nHandBrake Shift\nRearView Ctrl\nInfo Tab\nChatMode T\nInCarView F1\n\
    OutCarView F2\nCameraView F3\nRearMirror F5\nCyclePlayers F4\nScreenShot F12\nGameMenu Esc\n";

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Dir {
    Left,
    Right,
    Up,
    Down,
    In,
    Out,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Code {
    Key(KeyCode),
    /// "Shift", "Ctrl", "Alt" in the shipped layouts: either side
    Either(KeyCode, KeyCode),
    /// 1..8
    JoyButton(u8),
    Joy(Dir),
    Mouse(Dir),
    /// 0 left, 1 right, 2 middle
    MouseButton(u8),
}

/// DirectInput key names (the "Valid control codes" list in the .clo files, plus the keys
/// it leaves out such as U and UpArrow) and their keys.
pub const KEYS: &[(&str, KeyCode)] = {
    use KeyCode::*;
    &[
        ("Escape", Escape), ("1", Digit1), ("2", Digit2), ("3", Digit3), ("4", Digit4), ("5", Digit5),
        ("6", Digit6), ("7", Digit7), ("8", Digit8), ("9", Digit9), ("0", Digit0), ("Minus", Minus),
        ("Equals", Equal), ("Back", Backspace), ("Tab", Tab), ("Q", KeyQ), ("W", KeyW), ("E", KeyE),
        ("R", KeyR), ("T", KeyT), ("Y", KeyY), ("U", KeyU), ("I", KeyI), ("O", KeyO), ("P", KeyP),
        ("LeftBracket", BracketLeft), ("RightBracket", BracketRight), ("Return", Enter),
        ("LeftControl", ControlLeft), ("A", KeyA), ("S", KeyS), ("D", KeyD), ("F", KeyF), ("G", KeyG),
        ("H", KeyH), ("J", KeyJ), ("K", KeyK), ("L", KeyL), ("SemiColon", Semicolon),
        ("Apostrophe", Quote), ("Grave", Backquote), ("LeftShift", ShiftLeft), ("BackSlash", Backslash),
        ("Z", KeyZ), ("X", KeyX), ("C", KeyC), ("V", KeyV), ("B", KeyB), ("N", KeyN), ("M", KeyM),
        ("Comma", Comma), ("Period", Period), ("Slash", Slash), ("RightShift", ShiftRight),
        ("NumMultiply", NumpadMultiply), ("LeftAlt", AltLeft), ("Space", Space), ("CapsLock", CapsLock),
        ("F1", F1), ("F2", F2), ("F3", F3), ("F4", F4), ("F5", F5), ("F6", F6), ("F7", F7), ("F8", F8),
        ("F9", F9), ("F10", F10), ("NumLock", NumLock), ("ScrollLock", ScrollLock), ("Num7", Numpad7),
        ("Num8", Numpad8), ("Num9", Numpad9), ("NumSubtract", NumpadSubtract), ("Num4", Numpad4),
        ("Num5", Numpad5), ("Num6", Numpad6), ("NumAdd", NumpadAdd), ("Num1", Numpad1), ("Num2", Numpad2),
        ("Num3", Numpad3), ("Num0", Numpad0), ("NumDecimal", NumpadDecimal), ("Greater", IntlBackslash),
        ("F11", F11), ("F12", F12), ("F13", F13), ("F14", F14), ("F15", F15), ("Kana", KanaMode),
        ("Convert", Convert), ("NoConvert", NonConvert), ("Yen", IntlYen), ("NumEquals", NumpadEqual),
        ("NumEnter", NumpadEnter), ("RightControl", ControlRight), ("NumComma", NumpadComma),
        ("NumDivide", NumpadDivide), ("SysRequest", PrintScreen), ("RightAlt", AltRight),
        ("Home", Home), ("UpArrow", ArrowUp), ("PageUp", PageUp), ("LeftArrow", ArrowLeft),
        ("RightArrow", ArrowRight), ("End", End), ("DownArrow", ArrowDown), ("PageDown", PageDown),
        ("Insert", Insert), ("Delete", Delete), ("LeftWindows", SuperLeft), ("RightWindows", SuperRight),
        ("AppMenu", ContextMenu), ("Esc", Escape),
    ]
};
const EITHER: [(&str, KeyCode, KeyCode); 3] =
    [("Shift", KeyCode::ShiftLeft, KeyCode::ShiftRight), ("Ctrl", KeyCode::ControlLeft, KeyCode::ControlRight), ("Alt", KeyCode::AltLeft, KeyCode::AltRight)];
const DIRS: [(&str, Dir); 6] = [("Left", Dir::Left), ("Right", Dir::Right), ("Up", Dir::Up), ("Down", Dir::Down), ("In", Dir::In), ("Out", Dir::Out)];
const MOUSE_BUTTONS: [&str; 3] = ["Left", "Right", "Middle"];

impl Code {
    /// A control code name, case-insensitively.
    pub fn parse(s: &str) -> Option<Code> {
        let eq = |a: &str| a.eq_ignore_ascii_case(s);
        if let Some(&(_, k)) = KEYS.iter().find(|(n, _)| eq(n)) {
            return Some(Code::Key(k));
        }
        if let Some(&(_, a, b)) = EITHER.iter().find(|(n, ..)| eq(n)) {
            return Some(Code::Either(a, b));
        }
        let lower = s.to_ascii_lowercase();
        let dir = |rest: &str| DIRS.iter().find(|(n, _)| n.eq_ignore_ascii_case(rest)).map(|d| d.1);
        if let Some(n) = lower.strip_prefix("joybutton") {
            return n.parse().ok().filter(|n| (1..=8).contains(n)).map(Code::JoyButton);
        }
        if let Some(n) = lower.strip_prefix("mousebutton") {
            return MOUSE_BUTTONS.iter().position(|b| b.eq_ignore_ascii_case(n)).map(|i| Code::MouseButton(i as u8));
        }
        if let Some(d) = lower.strip_prefix("joy").and_then(dir) {
            return Some(Code::Joy(d));
        }
        lower.strip_prefix("mouse").and_then(dir).filter(|d| !matches!(d, Dir::In | Dir::Out)).map(Code::Mouse)
    }

    pub fn name(&self) -> String {
        let dir = |d: Dir| DIRS.iter().find(|x| x.1 == d).map_or("?", |x| x.0);
        match *self {
            Code::Key(k) => KEYS.iter().find(|x| x.1 == k).map_or("?", |x| x.0).to_string(),
            Code::Either(a, _) => EITHER.iter().find(|x| x.1 == a).map_or("?", |x| x.0).to_string(),
            Code::JoyButton(n) => format!("JoyButton{n}"),
            Code::Joy(d) => format!("Joy{}", dir(d)),
            Code::Mouse(d) => format!("Mouse{}", dir(d)),
            Code::MouseButton(i) => format!("MouseButton{}", MOUSE_BUTTONS.get(i as usize).unwrap_or(&"?")),
        }
    }

    /// Axes give a proportional value; keys and buttons are on or off.
    pub fn analog(&self) -> bool {
        matches!(self, Code::Joy(_) | Code::Mouse(_))
    }
}

#[derive(Clone, Debug)]
pub struct Layout {
    /// LayoutName (shown in the menu)
    pub name: String,
    /// MouseSpeedH / MouseSpeedV
    pub mouse_speed: (f32, f32),
    /// the bindings in use: `file` with the player's changes on top (front.rs `sync_controls`)
    pub bind: [Option<Code>; TASKS.len()],
    /// the bindings as the .clo file has them
    pub file: [Option<Code>; TASKS.len()],
}

impl Layout {
    pub fn parse(bytes: &[u8]) -> Layout {
        let c = motor_data::cfg::Config::parse(bytes);
        let val = |k: &str| c.get(k).and_then(|e| e.values.first()).cloned();
        let speed = |k: &str| c.get(k).and_then(|e| e.f32(0)).unwrap_or(1.0);
        let mut bind = TASKS.map(|t| val(t).and_then(|v| Code::parse(&v)));
        // Respawn is our task: R unless the layout has it or uses R for something else
        let r = Some(Code::Key(KeyCode::KeyR));
        if bind[task::RESPAWN].is_none() && !bind.contains(&r) {
            bind[task::RESPAWN] = r;
        }
        // ChangeCamera is ours too: V unless the layout has it or uses V for something else
        let v = Some(Code::Key(KeyCode::KeyV));
        if bind[task::CHANGE_CAMERA].is_none() && !bind.contains(&v) {
            bind[task::CHANGE_CAMERA] = v;
        }
        Layout { name: val("LayoutName").unwrap_or_else(|| "Custom".into()), mouse_speed: (speed("MouseSpeedH"), speed("MouseSpeedV")), bind, file: bind }
    }

    pub fn is_custom(&self) -> bool {
        self.name.eq_ignore_ascii_case("custom")
    }
}

/// Every Game/*.clo of the install, in the order of the file names (the original's layout list:
/// Custom, Joystick, Keyboard, Mouse, SteeringWheel [O reference video; the order of the directory
/// scan 0x419cf8 is [I] the file names']); the built-in keyboard layout when there are none
/// ("Couldn't find any control config files!!").
pub fn load_layouts(root: &Path) -> Vec<Layout> {
    let mut files: Vec<(String, Layout)> = motor_data::fs::find_ci(root, "Game")
        .and_then(|d| std::fs::read_dir(d).ok())
        .map(|d| {
            d.flatten()
                .map(|e| e.path())
                .filter(|p| p.extension().is_some_and(|x| x.eq_ignore_ascii_case("clo")))
                .filter_map(|p| Some((p.file_name()?.to_string_lossy().to_ascii_lowercase(), Layout::parse(&std::fs::read(&p).ok()?))))
                .collect()
        })
        .unwrap_or_default();
    files.sort_by(|a, b| a.0.cmp(&b.0));
    let mut out: Vec<Layout> = files.into_iter().map(|f| f.1).collect();
    if out.is_empty() {
        out.push(Layout::parse(DEFAULT_LAYOUT.as_bytes()));
    }
    out
}

/// Stick and trigger dead zones and the steering sensitivity, set on the modern Controller
/// page [design choice]. The original's stick axis code has no dead zone or sensitivity of its own
/// (`(x - centre) / (end - centre)`, above 0.98 snapped to 1: 0x426c3a, 0x4d2738 [O static]; the
/// 5 % seen live comes from the DirectInput / Wine layer [I]); `K(0x4d2740)` is its mouse path.
/// Ours steers in proportion to the travel right up to full lock and skips the original's
/// full-lock key ramp (half lock, then +0.6 / s, 0x45bf34) [design choice, #377].
/// Defaults: 5 % stick dead zone (the original's live value), 15 % triggers, sensitivity 1.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct PadTune {
    pub stick_dead: f32,
    pub trigger_dead: f32,
    pub steer_sens: f32,
}

impl Default for PadTune {
    fn default() -> PadTune {
        PadTune { stick_dead: 0.05, trigger_dead: 0.15, steer_sens: 1.0 }
    }
}

impl PadTune {
    /// A raw axis (0..1 in the wanted direction) past the dead zone `dz`, rescaled to 0..1.
    pub fn dead(a: f32, dz: f32) -> f32 {
        ((a - dz) / (1.0 - dz).max(1e-3)).clamp(0.0, 1.0)
    }

    /// A stick direction: dead zone, then the steering sensitivity on left / right.
    pub fn stick(&self, a: f32, d: Dir) -> f32 {
        let v = Self::dead(a, self.stick_dead);
        if matches!(d, Dir::Left | Dir::Right) {
            (v * self.steer_sens).min(1.0)
        } else {
            v
        }
    }
}
/// Mouse counts for full lock at MouseSpeed 1 (design choice).
const MOUSE_RANGE: f32 = 400.0;
/// Stick deflection that counts as a press (menus, binding capture).
const PRESS: f32 = 0.6;
/// Menu navigation by a held d-pad or stick [design choice, #419]: the first repeat comes after this pause,
/// the next few are slow, then it speeds up.
const NAV_DELAY: Duration = Duration::from_millis(400);
const NAV_SLOW: Duration = Duration::from_millis(180);
const NAV_FAST: Duration = Duration::from_millis(70);
/// Repeats that keep the slow pace before the fast one.
const NAV_SLOW_STEPS: u32 = 3;

/// The gap before repeat `n + 1`, after `n` repeats were sent.
fn nav_gap(n: u32) -> Duration {
    if n <= NAV_SLOW_STEPS { NAV_SLOW } else { NAV_FAST }
}

/// A d-pad button or stick direction held in a menu, repeating its arrow key.
struct NavHold {
    key: KeyCode,
    /// the d-pad button (`None` = the left stick)
    button: Option<gilrs::Button>,
    next: Instant,
    n: u32,
}

/// gilrs buttons for JoyButton1..8.
pub const PAD_BUTTONS: [gilrs::Button; 8] = {
    use gilrs::Button::*;
    [South, East, West, North, LeftTrigger, RightTrigger, LeftTrigger2, RightTrigger2]
};

/// The device the player last used, for the navigation glyphs [REMASTER].
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Device {
    #[default]
    Keyboard,
    Xbox,
    PlayStation,
}

impl Device {
    pub fn parse(s: &str) -> Option<Device> {
        match s.to_ascii_lowercase().as_str() {
            "keyboard" => Some(Device::Keyboard),
            "xbox" => Some(Device::Xbox),
            "playstation" | "ps" => Some(Device::PlayStation),
            _ => None,
        }
    }
}

/// Xbox or PlayStation family from the USB vendor (Sony 0x054c) or the name; everything else
/// is drawn as an Xbox pad [design choice].
fn pad_kind(p: &gilrs::Gamepad) -> Device {
    let name = p.name().to_ascii_lowercase();
    let sony = p.vendor_id() == Some(0x054c) || ["playstation", "dualshock", "dualsense", "ps3", "ps4", "ps5", "wireless controller"].iter().any(|n| name.contains(n));
    if sony {
        Device::PlayStation
    } else {
        Device::Xbox
    }
}

/// What a gamepad does to a task alongside the layout's keys [design choice: the original
/// drives from the one selected layout only]: both devices drive at once. Skipped while the
/// layout binds pad codes itself. The car tasks are set on the modern Controller page.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PadIn {
    /// the left stick
    Stick(Dir),
    Button(gilrs::Button),
}

/// Settings names of the pad controls.
pub const PAD_NAMES: [(&str, PadIn); 21] = {
    use gilrs::Button::*;
    use PadIn::*;
    [
        ("StickLeft", Stick(Dir::Left)), ("StickRight", Stick(Dir::Right)), ("StickUp", Stick(Dir::Up)), ("StickDown", Stick(Dir::Down)),
        ("South", Button(South)), ("East", Button(East)), ("West", Button(West)), ("North", Button(North)),
        ("LeftTrigger", Button(LeftTrigger)), ("RightTrigger", Button(RightTrigger)), ("LeftTrigger2", Button(LeftTrigger2)), ("RightTrigger2", Button(RightTrigger2)),
        ("Select", Button(Select)), ("Start", Button(Start)), ("Mode", Button(Mode)), ("LeftThumb", Button(LeftThumb)), ("RightThumb", Button(RightThumb)),
        ("DPadUp", Button(DPadUp)), ("DPadDown", Button(DPadDown)), ("DPadLeft", Button(DPadLeft)), ("DPadRight", Button(DPadRight)),
    ]
};

impl PadIn {
    pub fn parse(s: &str) -> Option<PadIn> {
        PAD_NAMES.iter().find(|(n, _)| n.eq_ignore_ascii_case(s)).map(|x| x.1)
    }

    pub fn name(&self) -> &'static str {
        PAD_NAMES.iter().find(|x| x.1 == *self).map_or("?", |x| x.0)
    }
}

pub const DEFAULT_PAD_BIND: [Option<PadIn>; TASKS.len()] = {
    use gilrs::Button::*;
    use PadIn::*;
    let mut b = [None; TASKS.len()];
    b[task::TURN_LEFT] = Some(Stick(Dir::Left));
    b[task::TURN_RIGHT] = Some(Stick(Dir::Right));
    b[task::ACCELERATE] = Some(Button(RightTrigger2));
    b[task::BRAKE] = Some(Button(LeftTrigger2));
    b[task::GEAR_UP] = Some(Button(RightTrigger));
    b[task::GEAR_DOWN] = Some(Button(LeftTrigger));
    b[task::HORN] = Some(Button(LeftThumb));
    b[task::HANDBRAKE] = Some(Button(West));
    b[task::REAR_VIEW] = Some(Button(North));
    b[task::RESPAWN] = Some(Button(Select));
    b[task::CHANGE_CAMERA] = Some(Button(East));
    b[task::INFO] = Some(Button(RightThumb));
    b[task::OUT_CAR_VIEW] = Some(Button(DPadUp));
    b[task::IN_CAR_VIEW] = Some(Button(DPadDown));
    b[task::CAMERA_VIEW] = Some(Button(DPadRight));
    b[task::REAR_MIRROR] = Some(Button(DPadLeft));
    b
};

/// Keep the set of held pad buttons up to date from one gilrs event [design choice].
/// gilrs' cached `button_data` is found only through the pad mapping, so a hat D-Pad or a
/// stick click reads 0 there although its events arrive; the events are the reliable source.
fn track_held<I: std::hash::Hash + Eq + Copy>(held: &mut HashSet<(I, gilrs::Button)>, id: I, ev: &gilrs::EventType) {
    use gilrs::EventType;
    match *ev {
        EventType::ButtonPressed(b, _) => set_held(held, id, b, true),
        EventType::ButtonReleased(b, _) => set_held(held, id, b, false),
        EventType::Disconnected => held.retain(|(i, _)| *i != id),
        _ => {}
    }
}

fn set_held<I: std::hash::Hash + Eq + Copy>(held: &mut HashSet<(I, gilrs::Button)>, id: I, b: gilrs::Button, down: bool) {
    if b == gilrs::Button::Unknown {
        return;
    }
    if down {
        held.insert((id, b));
    } else {
        held.remove(&(id, b));
    }
}

/// A key the gamepad gives the menus (and whether it also works while racing).
pub struct PadKey {
    pub key: KeyCode,
    pub in_race: bool,
    /// the pad's Start / Menu button (an Escape that the pad keyboard reads as "keep")
    pub start: bool,
}

pub struct Controls {
    pub layouts: Vec<Layout>,
    pub active: usize,
    gilrs: Option<gilrs::Gilrs>,
    mouse_buttons: [bool; 3],
    /// mouse position as a virtual stick, -1..1 per axis
    mouse: (f32, f32),
    /// left stick direction last reported to the menus
    stick_dir: Option<Dir>,
    /// the d-pad or stick arrow being held (repeats in `poll`)
    nav: Option<NavHold>,
    /// a gamepad or mouse control pressed since the last `take_pressed`
    pressed: Option<Code>,
    held: [bool; TASKS.len()],
    /// the device last used, for the navigation glyphs
    pub last: Device,
    /// a pad control pressed since the last `take_pad_pressed` (pad binding capture)
    pad_pressed: Option<PadIn>,
    /// what the pad does to each task (`DEFAULT_PAD_BIND`; car tasks from the settings)
    pub pad_bind: [Option<PadIn>; TASKS.len()],
    pub tune: PadTune,
    /// digital pad buttons held right now, from the events (see `track_held`)
    pad_held: HashSet<(gilrs::GamepadId, gilrs::Button)>,
    /// rumble effects on the pads that support force feedback: strong (low frequency) and weak
    /// motor, played for good with the gain set per frame (#383)
    ff: Vec<gilrs::ff::Effect>,
    /// the (strong, weak) gains last sent
    ff_gain: (f32, f32),
}

/// One frame of the active layout: every task's value (0..1) and the tasks that went down.
pub struct State {
    pub value: [f32; TASKS.len()],
    pub analog: [bool; TASKS.len()],
    pub went_down: [bool; TASKS.len()],
}

impl State {
    pub fn held(&self, t: usize) -> bool {
        self.value[t] > 0.5
    }
}

impl Controls {
    pub fn new(root: &Path) -> Controls {
        let layouts = load_layouts(root);
        let active = layouts.iter().position(|l| l.name.eq_ignore_ascii_case(DEFAULT_LAYOUT_NAME)).unwrap_or(0);
        Controls { layouts, active, gilrs: None, mouse_buttons: [false; 3], mouse: (0.0, 0.0), stick_dir: None, nav: None, pressed: None, held: [false; TASKS.len()], last: Device::Keyboard, pad_pressed: None, pad_bind: DEFAULT_PAD_BIND, tune: PadTune::default(), pad_held: HashSet::new(), ff: Vec::new(), ff_gain: (0.0, 0.0) }
    }

    /// Start reading gamepads, joysticks and wheels (interactive runs only).
    pub fn open_pads(&mut self) {
        match gilrs::Gilrs::new() {
            Ok(g) => {
                for (_, p) in g.gamepads() {
                    println!("gamepad: {}", p.name());
                }
                self.gilrs = Some(g);
                self.rebuild_ff();
            }
            Err(e) => eprintln!("gamepads: {e}"),
        }
    }

    /// Rumble effects (the strong and the weak motor) on every connected pad that supports force
    /// feedback; called at start and when a pad comes or goes. REMASTER-era feature, design choice
    /// (the original has none).
    fn rebuild_ff(&mut self) {
        use gilrs::ff::{BaseEffect, BaseEffectType, EffectBuilder, Replay, Ticks};
        self.ff.clear();
        self.ff_gain = (0.0, 0.0);
        let Some(g) = &mut self.gilrs else { return };
        let ids: Vec<gilrs::GamepadId> = g.gamepads().filter(|(_, p)| p.is_ff_supported()).map(|(id, _)| id).collect();
        if ids.is_empty() {
            return;
        }
        let replay = Replay { after: Ticks::from_ms(0), play_for: Ticks::from_ms(1000), with_delay: Ticks::from_ms(0) };
        for kind in [BaseEffectType::Strong { magnitude: u16::MAX }, BaseEffectType::Weak { magnitude: u16::MAX }] {
            let built = EffectBuilder::new().add_effect(BaseEffect { kind, scheduling: replay, ..Default::default() }).gamepads(&ids).gain(0.0).finish(g);
            match built {
                Ok(e) => {
                    if let Err(e) = e.play() {
                        eprintln!("rumble: {e}");
                    }
                    self.ff.push(e);
                }
                Err(e) => {
                    eprintln!("rumble: {e}");
                    self.ff.clear();
                    return;
                }
            }
        }
        println!("rumble: {} pad(s)", ids.len());
    }

    /// Set the rumble motors, 0..1 each (0, 0 stops them). Cheap: gilrs hears of a change only
    /// when a motor moved by more than 2 %.
    pub fn set_rumble(&mut self, strong: f32, weak: f32) {
        let moved = |a: f32, b: f32| (a - b).abs() > 0.02 || (a != b && (a == 0.0 || b == 0.0));
        if self.ff.len() != 2 || !(moved(strong, self.ff_gain.0) || moved(weak, self.ff_gain.1)) {
            return;
        }
        self.ff_gain = (strong, weak);
        for (e, v) in self.ff.iter().zip([strong, weak]) {
            let _ = e.set_gain(v.clamp(0.0, 1.0));
        }
    }

    pub fn layout(&self) -> &Layout {
        &self.layouts[self.active.min(self.layouts.len() - 1)]
    }

    pub fn custom_mut(&mut self) -> Option<&mut Layout> {
        self.layouts.iter_mut().find(|l| l.is_custom())
    }

    /// Whether task `t` of the active layout is bound to key `k`.
    pub fn is_key(&self, t: usize, k: KeyCode) -> bool {
        match self.layout().bind[t] {
            Some(Code::Key(x)) => x == k,
            Some(Code::Either(a, b)) => a == k || b == k,
            _ => false,
        }
    }

    /// Index of a layout by name (settings file).
    pub fn find(&self, name: &str) -> Option<usize> {
        self.layouts.iter().position(|l| l.name.eq_ignore_ascii_case(name))
    }

    pub fn mouse_motion(&mut self, dx: f64, dy: f64) {
        let (sh, sv) = self.layout().mouse_speed;
        self.mouse.0 = (self.mouse.0 + dx as f32 * sh / MOUSE_RANGE).clamp(-1.0, 1.0);
        self.mouse.1 = (self.mouse.1 + dy as f32 * sv / MOUSE_RANGE).clamp(-1.0, 1.0);
    }

    /// Is mouse button `i` (0 left) held?
    pub fn mouse_down(&self, i: u8) -> bool {
        self.mouse_buttons.get(i as usize).copied().unwrap_or(false)
    }

    pub fn mouse_button(&mut self, i: u8, down: bool) {
        if let Some(b) = self.mouse_buttons.get_mut(i as usize) {
            if down && !*b {
                self.pressed = Some(Code::MouseButton(i));
            }
            *b = down;
        }
    }

    /// Centre the mouse "wheel" and forget held tasks (race start).
    pub fn reset(&mut self) {
        self.mouse = (0.0, 0.0);
        self.held = [true; TASKS.len()]; // no edges from keys still held from the menu
    }

    /// The last gamepad or mouse control pressed, for binding capture.
    pub fn take_pressed(&mut self) -> Option<Code> {
        self.pressed.take()
    }

    /// The last pad button or left-stick direction pressed, for pad binding capture.
    pub fn take_pad_pressed(&mut self) -> Option<PadIn> {
        self.pad_pressed.take()
    }

    /// Whether the pad bindings drive: the layout has no pad codes of its own.
    pub fn pad_active(&self) -> bool {
        !self.layout_uses_pad()
    }

    /// The active layout binds joystick codes itself (Joystick / SteeringWheel .clo).
    pub fn layout_uses_pad(&self) -> bool {
        self.layout().bind.iter().flatten().any(|c| matches!(c, Code::Joy(_) | Code::JoyButton(_)))
    }

    /// A pad button is held down right now (the pause menu's hold to confirm).
    pub fn pad_down(&self, b: gilrs::Button) -> bool {
        self.pad_held.iter().any(|&(_, x)| x == b)
    }

    /// A (South) is bound to a driving task, by the layout or a pad binding.
    pub fn a_bound(&self) -> bool {
        self.layout_uses_pad() || self.pad_bind.contains(&Some(PadIn::Button(gilrs::Button::South)))
    }

    /// Drain gamepad events: returns menu keys (d-pad / left stick = arrows, A = Enter,
    /// B = Esc, Start = Esc also in races) and records presses for binding capture.
    pub fn poll(&mut self) -> Vec<PadKey> {
        let mut out = Vec::new();
        let Some(g) = &mut self.gilrs else { return out };
        let mut reconnect = false;
        while let Some(ev) = g.next_event() {
            use gilrs::{Axis, Button, EventType};
            track_held(&mut self.pad_held, ev.id, &ev.event);
            let menu = |key| PadKey { key, in_race: false, start: false };
            match ev.event {
                EventType::Connected => reconnect = true,
                EventType::Disconnected => {
                    reconnect = true;
                    self.nav = None;
                }
                EventType::ButtonReleased(b, _) => {
                    if self.nav.as_ref().is_some_and(|h| h.button == Some(b)) {
                        self.nav = None;
                    }
                }
                EventType::ButtonPressed(b, _) => {
                    self.last = pad_kind(&g.gamepad(ev.id));
                    if b != Button::Unknown {
                        self.pad_pressed = Some(PadIn::Button(b));
                    }
                    if let Some(i) = PAD_BUTTONS.iter().position(|&x| x == b) {
                        self.pressed = Some(Code::JoyButton(i as u8 + 1));
                    }
                    let arrow = match b {
                        Button::DPadUp => Some(KeyCode::ArrowUp),
                        Button::DPadDown => Some(KeyCode::ArrowDown),
                        Button::DPadLeft => Some(KeyCode::ArrowLeft),
                        Button::DPadRight => Some(KeyCode::ArrowRight),
                        _ => None,
                    };
                    if let Some(key) = arrow {
                        out.push(menu(key));
                        self.nav = Some(NavHold { key, button: Some(b), next: Instant::now() + NAV_DELAY, n: 0 });
                    }
                    match b {
                        Button::South => out.push(PadKey { key: KeyCode::Enter, in_race: true, start: false }),
                        Button::East => out.push(menu(KeyCode::Escape)),
                        // the modern graphic options: LB / RB switch tab, Y applies, X restores defaults
                        Button::LeftTrigger => out.push(menu(KeyCode::PageUp)),
                        Button::RightTrigger => out.push(menu(KeyCode::PageDown)),
                        Button::North => out.push(menu(KeyCode::F5)),
                        Button::West => out.push(menu(KeyCode::F6)),
                        // the modern Rankings' difficulty (Z / C): menu only, in a race the triggers are accelerate / brake
                        Button::LeftTrigger2 => out.push(menu(KeyCode::F7)),
                        Button::RightTrigger2 => out.push(menu(KeyCode::F8)),
                        Button::Start => out.push(PadKey { key: KeyCode::Escape, in_race: true, start: true }),
                        _ => {}
                    }
                }
                EventType::AxisChanged(Axis::LeftStickX | Axis::LeftStickY | Axis::LeftZ, ..) => {
                    let pad = g.gamepad(ev.id);
                    let (x, y, z) = (pad.value(Axis::LeftStickX), pad.value(Axis::LeftStickY), pad.value(Axis::LeftZ));
                    let dir = if x < -PRESS {
                        Some(Dir::Left)
                    } else if x > PRESS {
                        Some(Dir::Right)
                    } else if y > PRESS {
                        Some(Dir::Up)
                    } else if y < -PRESS {
                        Some(Dir::Down)
                    } else if z < -PRESS {
                        Some(Dir::In)
                    } else if z > PRESS {
                        Some(Dir::Out)
                    } else {
                        None
                    };
                    if dir != self.stick_dir {
                        if self.nav.as_ref().is_some_and(|h| h.button.is_none()) {
                            self.nav = None;
                        }
                        if let Some(d) = dir {
                            self.last = pad_kind(&pad);
                            self.pressed = Some(Code::Joy(d));
                            if !matches!(d, Dir::In | Dir::Out) {
                                self.pad_pressed = Some(PadIn::Stick(d));
                            }
                            let key = match d {
                                Dir::Left => Some(KeyCode::ArrowLeft),
                                Dir::Right => Some(KeyCode::ArrowRight),
                                Dir::Up => Some(KeyCode::ArrowUp),
                                Dir::Down => Some(KeyCode::ArrowDown),
                                _ => None,
                            };
                            out.extend(key.map(menu));
                            if let Some(key) = key {
                                self.nav = Some(NavHold { key, button: None, next: Instant::now() + NAV_DELAY, n: 0 });
                            }
                        }
                        self.stick_dir = dir;
                    }
                }
                // any other clear stick or trigger movement also means the pad is in use (glyphs)
                EventType::AxisChanged(_, v, _) | EventType::ButtonChanged(_, v, _) if v.abs() > PRESS => {
                    self.last = pad_kind(&g.gamepad(ev.id));
                }
                _ => {}
            }
        }
        // a held arrow repeats: a pause, a few slow steps, then fast (menu keys: the race ignores them, front.rs `poll_pads`)
        if let Some(h) = &mut self.nav {
            let now = Instant::now();
            if now >= h.next {
                out.push(PadKey { key: h.key, in_race: false, start: false });
                h.n += 1;
                h.next = now + nav_gap(h.n);
            }
        }
        if reconnect {
            self.rebuild_ff();
        }
        out
    }

    /// Value of one control, 0..1.
    pub fn value(&self, code: Code, keys: &HashSet<KeyCode>) -> f32 {
        let on = |b: bool| b as u8 as f32;
        match code {
            Code::Key(k) => on(keys.contains(&k)),
            Code::Either(a, b) => on(keys.contains(&a) || keys.contains(&b)),
            Code::MouseButton(i) => on(self.mouse_buttons.get(i as usize).copied().unwrap_or(false)),
            Code::Mouse(d) => match d {
                Dir::Left => (-self.mouse.0).max(0.0),
                Dir::Right => self.mouse.0.max(0.0),
                Dir::Up => (-self.mouse.1).max(0.0),
                Dir::Down => self.mouse.1.max(0.0),
                _ => 0.0,
            },
            // the strongest of all connected pads
            Code::JoyButton(n) => self.button_value(PAD_BUTTONS[(n as usize - 1).min(7)]),
            Code::Joy(d) => self.pads(|p| {
                use gilrs::Axis;
                self.tune.stick(match d {
                    Dir::Left => -p.value(Axis::LeftStickX),
                    Dir::Right => p.value(Axis::LeftStickX),
                    Dir::Up => p.value(Axis::LeftStickY),
                    Dir::Down => -p.value(Axis::LeftStickY),
                    // APPROX: the joystick Z axis (throttle lever) as in/out
                    Dir::In => -p.value(Axis::LeftZ),
                    Dir::Out => p.value(Axis::LeftZ),
                }, d)
            }),
        }
    }

    /// Right stick of the most deflected pad, -1..1 (x right, y up) past a radial dead
    /// zone: the chase camera's free look [design choice, not in the original].
    pub fn right_stick(&self) -> glam::Vec2 {
        use gilrs::Axis;
        let Some(g) = &self.gilrs else { return glam::Vec2::ZERO };
        let v = g.gamepads().map(|(_, p)| glam::Vec2::new(p.value(Axis::RightStickX), p.value(Axis::RightStickY))).max_by(|a, b| a.length_squared().total_cmp(&b.length_squared())).unwrap_or_default();
        let len = v.length();
        if len <= self.tune.stick_dead {
            return glam::Vec2::ZERO;
        }
        v / len * PadTune::dead(len, self.tune.stick_dead)
    }

    /// A pad control of the fixed overlay, 0..1.
    fn pad_value(&self, c: PadIn) -> f32 {
        let dead = |a: f32| PadTune::dead(a, self.tune.trigger_dead);
        match c {
            PadIn::Stick(d) => self.value(Code::Joy(d), &HashSet::new()),
            PadIn::Button(b @ (gilrs::Button::LeftTrigger2 | gilrs::Button::RightTrigger2)) => dead(self.button_value(b)),
            PadIn::Button(b) => self.button_value(b),
        }
    }

    /// One pad button, 0..1, strongest of all pads. The triggers are analog (cached value);
    /// every other button is read from the events, because `button_data` finds nothing for a
    /// hat D-Pad or some stick clicks (gilrs 0.11 maps through the pad mapping only).
    fn button_value(&self, b: gilrs::Button) -> f32 {
        if matches!(b, gilrs::Button::LeftTrigger2 | gilrs::Button::RightTrigger2) {
            return self.pads(|p| p.button_data(b).map_or(0.0, |d| d.value()));
        }
        self.pad_held.iter().any(|&(_, x)| x == b) as u8 as f32
    }

    /// The pad bindings drive beside the layout's keys, unless the layout binds pad codes itself.
    fn overlay(&self) -> bool {
        self.gilrs.is_some() && self.pad_active()
    }

    /// Note keyboard or mouse use (glyphs follow the last device).
    pub fn used_keyboard(&mut self) {
        self.last = Device::Keyboard;
    }

    fn pads(&self, f: impl Fn(&gilrs::Gamepad) -> f32) -> f32 {
        let Some(g) = &self.gilrs else { return 0.0 };
        g.gamepads().map(|(_, p)| f(&p)).fold(0.0, f32::max)
    }

    /// The active layout this frame.
    pub fn state(&mut self, keys: &HashSet<KeyCode>) -> State {
        let bind = self.layout().bind;
        let mut value = bind.map(|c| c.map_or(0.0, |c| self.value(c, keys)));
        let mut analog = bind.map(|c| c.is_some_and(|c| c.analog()));
        if self.overlay() {
            for (t, p) in self.pad_bind.iter().enumerate() {
                let Some(p) = *p else { continue };
                let v = self.pad_value(p);
                if v > value[t] {
                    value[t] = v;
                    analog[t] = matches!(p, PadIn::Stick(_)) || matches!(p, PadIn::Button(gilrs::Button::LeftTrigger2 | gilrs::Button::RightTrigger2));
                }
            }
        }
        let went_down = std::array::from_fn(|t| value[t] > 0.5 && !self.held[t]);
        self.held = value.map(|v| v > 0.5);
        State { value, analog, went_down }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nav_repeat_speeds_up() {
        assert!(NAV_DELAY > NAV_SLOW && NAV_SLOW > NAV_FAST);
        assert_eq!(nav_gap(1), NAV_SLOW);
        assert_eq!(nav_gap(NAV_SLOW_STEPS), NAV_SLOW);
        assert_eq!(nav_gap(NAV_SLOW_STEPS + 1), NAV_FAST);
    }

    #[test]
    fn codes_round_trip() {
        for n in ["LeftArrow", "S", "Shift", "Ctrl", "JoyButton3", "JoyLeft", "JoyOut", "MouseRight", "MouseButtonMiddle", "NumEnter", "F4"] {
            let c = Code::parse(n).unwrap_or_else(|| panic!("{n}"));
            assert_eq!(c.name(), n);
        }
        assert_eq!(Code::parse("esc"), Some(Code::Key(KeyCode::Escape)));
        assert_eq!(Code::parse("JoyButton9"), None);
        assert_eq!(Code::parse("MouseIn"), None);
    }

    #[test]
    fn default_layout() {
        let l = Layout::parse(DEFAULT_LAYOUT.as_bytes());
        assert_eq!(l.name, "Keyboard");
        assert_eq!(l.bind[task::GEAR_UP], Some(Code::Key(KeyCode::KeyS)));
        assert_eq!(l.bind[task::REAR_VIEW], Some(Code::Either(KeyCode::ControlLeft, KeyCode::ControlRight)));
        assert_eq!(l.bind[task::RESPAWN], Some(Code::Key(KeyCode::KeyR)));
        assert_eq!(l.bind[task::CHANGE_CAMERA], Some(Code::Key(KeyCode::KeyV)));
        assert_eq!(TASKS[task::INFO], "Info");
        assert_eq!(TASKS[task::SCREENSHOT], "ScreenShot");
        assert!(l.bind.iter().all(|b| b.is_some()));
    }

    #[test]
    fn pad_overlay_skipped_for_pad_layouts() {
        let mut c = Controls::new(Path::new("/nonexistent"));
        assert!(!c.overlay()); // no pads open
        let st = c.state(&HashSet::new());
        assert!(st.value.iter().all(|v| *v == 0.0));
        let l = Layout::parse(b"LayoutName X\nTurnLeft JoyLeft\n");
        assert!(l.bind.iter().flatten().any(|c| matches!(c, Code::Joy(_))));
    }

    #[test]
    fn pad_names_and_tune() {
        for (n, p) in PAD_NAMES {
            assert_eq!(PadIn::parse(n), Some(p));
            assert_eq!(p.name(), n);
        }
        assert!(DEFAULT_PAD_BIND.iter().flatten().all(|p| PadIn::parse(p.name()).is_some()));
        let t = PadTune { stick_dead: 0.2, trigger_dead: 0.1, steer_sens: 2.0 };
        assert_eq!(t.stick(0.1, Dir::Left), 0.0);
        assert!((t.stick(0.6, Dir::Up) - 0.5).abs() < 1e-6);
        assert!((t.stick(0.4, Dir::Right) - 0.5).abs() < 1e-6);
        assert_eq!(t.stick(0.9, Dir::Left), 1.0);
    }

    #[test]
    fn held_buttons_follow_events() {
        use gilrs::Button;
        let mut h = HashSet::new();
        set_held(&mut h, 1u8, Button::DPadDown, true);
        set_held(&mut h, 2u8, Button::LeftThumb, true);
        set_held(&mut h, 1u8, Button::Unknown, true);
        assert!(h.contains(&(1, Button::DPadDown)) && h.contains(&(2, Button::LeftThumb)) && h.len() == 2);
        set_held(&mut h, 1u8, Button::DPadDown, false);
        assert!(!h.contains(&(1, Button::DPadDown)));
        track_held(&mut h, 2u8, &gilrs::EventType::Disconnected);
        assert!(h.is_empty());
    }

    #[test]
    fn device_names() {
        assert_eq!(Device::parse("PlayStation"), Some(Device::PlayStation));
        assert_eq!(Device::parse("xbox"), Some(Device::Xbox));
        assert_eq!(Device::parse("tv"), None);
    }
}
