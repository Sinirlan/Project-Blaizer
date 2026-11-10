//! Game flow and front-end screens, laid out after the original 640x400 menus
//! (reference_materials/menu and the menu video) with the game's own MENU.SPR art.
//!
//! Flow as in the original: main -> one player ("single" dial) -> track -> car -> race.

use crate::controls::{task, Code, Controls, Device, DEFAULT_LAYOUT_NAME, PadIn, PadTune, CAR_TASKS, DEFAULT_PAD_BIND, TASKS, TASK_LABELS};
use crate::league::{self, League, Locks, Outcome, Profile};
use crate::hud_layout::{HudSettings, MapMode, INFO_CUSTOM, MAP_MODES, MARGIN_STEP, SCALE_RANGE, SCALE_STEP};
use self::menu_modern::{Ask, PaintPick};
pub(crate) use self::menu_modern::Want;
use crate::movie::Movie;
use crate::slide_fx;
use crate::{hiscore, hud, stats};
use glam::{Mat3, Mat4, Vec2, Vec3};
use motor_sim::{Difficulty, Input, Pedals, Rules, SteerKeys};
use motor_data::mde::{self, Mde};
use motor_data::sprpos::{Slid, SprPos};
use motor_sim::demo::DemoRecorder;
use motor_sim::startcam;
use motor_sim::replay::{GhostTrack, Header, LapRecorder, Recording};
use crate::demo::DemoCars;
use crate::playback::{self, Playback};
use motor_sim::AiRow;
use motorview::drive::{lineup, CamMode, FrameParts, Session};
use motorview::loading::{Progress, Stage, CANCELLED};
use motorview::minimap;
use motorview::render::{aa_stop, SceneLoad, SceneUpload, CarDetail, CarShading, Camera, FogStyle, Frame, Gpu, GrassLevel, LightShadows, LivePaint, PostAaKind, RemasterView, RenderAaKind, RenderMode, ShadowQuality, Tonemap, View, AA_STOPS, AA_STOP_STRENGTH};
use motorview::scene::{self, Vertex, CAR_MESHES};
use motorview::view_settings::{self, pick, BLUR_STYLES, CAR_MODELS, HUD_STYLES, SHADOW_STYLES, SKID_STYLES};
use motorview::ui::{pic_key, Assets, Batch, Canvas, UiVertex, WHITE};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use winit::keyboard::KeyCode;

pub const ORANGE: [f32; 4] = [1.0, 0.45, 0.05, 1.0];
/// CLASSIC menu text colours: FONT1 colour index 1..=5, filled by 0x45ff85 into the menu
/// palette 0x74fe98 [O]: orange (values), white, blue (labels), lavender, black (on the bar).
const MENU_ORANGE: usize = 1;
const MENU_WHITE: usize = 2;
const MENU_BLUE: usize = 3;
const MENU_BLACK: usize = 5;
const MENU_LAVENDER: usize = 4;
const MENU_PALETTE: [[u8; 3]; 5] = [[255, 100, 0], [255, 255, 255], [96, 96, 192], [186, 186, 248], [0, 0, 0]];
/// FONT1 chars 0x87 / 0x88: the blinking arrows beside a name that can be stepped (0x46c5f3, 0x46d170).
const ARROW_LEFT: &str = "\u{87}";
const ARROW_RIGHT: &str = "\u{88}";

/// 0x489c58: where a text's x sits.
#[derive(Clone, Copy, PartialEq)]
enum Align {
    Left,
    Centre,
    Right,
}

/// CLASSIC 0x489c60: FONT1 text at the original's pixel position (`y` = the top of the glyph
/// box), `colour` = menu palette index 1..=5; centred = x minus half the advance (0x489c87).
fn menu_text(b: &mut Batch, text: &str, x: f32, y: f32, colour: usize, align: Align) {
    menu_text_a(b, text, x, y, colour, align, 1.0);
}

/// `menu_text` faded to `alpha`: a row that has no effect right now [design choice, the original
/// has no such state].
fn menu_text_a(b: &mut Batch, text: &str, x: f32, y: f32, colour: usize, align: Align, alpha: f32) {
    let w = b.fnt_width(text, 1.0);
    let x = match align {
        Align::Left => x,
        Align::Centre => x - (w / 2.0).floor(),
        Align::Right => x - w,
    };
    let [r, g, bl] = MENU_PALETTE[colour - 1];
    b.fnt_text(text, x, y, 1.0, [r as f32 / 255.0, g as f32 / 255.0, bl as f32 / 255.0, alpha]);
}

/// CLASSIC 0x461e74: the selection bar behind a list item, 12 rows from y-1, x-4, text width + 8.
fn menu_bar(b: &mut Batch, x: f32, y: f32, text_w: f32, colour: usize) {
    menu_bar_a(b, x, y, text_w, colour, 1.0);
}

fn menu_bar_a(b: &mut Batch, x: f32, y: f32, text_w: f32, colour: usize, alpha: f32) {
    let [r, g, bl] = MENU_PALETTE[colour - 1];
    b.rect(x - 4.0, y - 1.0, text_w + 8.0, 12.0, [r as f32 / 255.0, g as f32 / 255.0, bl as f32 / 255.0, alpha]);
}

/// How a row of an option page is shown (CLASSIC 0x467855, 0x466bf3: labels colour 3, the
/// selected row's colour 2; a row's values colour 1, the selected row's colour 2).
#[derive(Clone, Copy)]
struct MenuRow {
    sel: bool,
    /// false: no effect right now, drawn faded
    on: bool,
}

impl MenuRow {
    fn alpha(self) -> f32 {
        if self.on { 1.0 } else { 0.45 }
    }
}

/// CLASSIC 0x467855: a row's label.
fn menu_label(b: &mut Batch, text: &str, x: f32, y: f32, row: MenuRow) {
    menu_text_a(b, text, x, y, if row.sel { 2 } else { 3 }, Align::Left, row.alpha());
}

/// CLASSIC 0x461ec0: the choices of a row side by side (width + 16 apart), the current one black
/// on a bar (white on the selected row, orange elsewhere), the others in the row's value colour.
/// `cur` out of range = no current one (plain text). Returns the x after the last choice.
fn menu_choices(b: &mut Batch, choices: &[&str], cur: usize, x: f32, y: f32, row: MenuRow) -> f32 {
    let colour = if row.sel { 2 } else { 1 };
    let mut x = x;
    for (k, c) in choices.iter().enumerate() {
        let w = b.fnt_width(c, 1.0);
        if k == cur {
            menu_bar_a(b, x, y, w, colour, row.alpha());
            menu_text_a(b, c, x, y, MENU_BLACK, Align::Left, row.alpha());
        } else {
            menu_text_a(b, c, x, y, colour, Align::Left, row.alpha());
        }
        x += w + 16.0;
    }
    x
}
/// `text` cut to `max` px of FONT1: its head, or (`tail`) its end.
fn clip_text(b: &Batch, text: &str, max: f32, tail: bool) -> String {
    let mut t = text.to_string();
    while !t.is_empty() && b.fnt_width(&t, 1.0) > max {
        if tail {
            t.remove(0);
        } else {
            t.pop();
        }
    }
    t
}

/// CLASSIC 0x45f634: a colour row's swatch (a frame in the row's colour, 32 x 10, round the exact
/// colour) and the strip of 32 cells 44 px to its right; the cell nearest to the colour has a
/// frame, black inside, round it.
fn colour_strip(b: &mut Batch, strip: &[[u8; 3]; STRIP_CELLS], c: [u8; 3], x: f32, y: f32, sel: bool) {
    let rect = |b: &mut Batch, x: f32, y: f32, w: f32, h: f32, c: [u8; 3]| b.rect(x, y, w, h, [c[0] as f32 / 255.0, c[1] as f32 / 255.0, c[2] as f32 / 255.0, 1.0]);
    let frame = MENU_PALETTE[if sel { 1 } else { 0 }];
    rect(b, x, y - 1.0, 32.0, 10.0, frame);
    rect(b, x + 1.0, y, 30.0, 8.0, c);
    let cur = strip_index(strip, c);
    for (i, cell) in strip.iter().enumerate() {
        let cx = x + 44.0 + 12.0 * i as f32;
        if i == cur {
            rect(b, cx - 2.0, y - 2.0, 14.0, 12.0, frame);
            rect(b, cx - 1.0, y - 1.0, 12.0, 10.0, [0, 0, 0]);
        }
        rect(b, cx, y, 10.0, 8.0, *cell);
    }
}

/// CLASSIC 0x461fc8: a vertical list, `n` items 15 px apart round the current one (`cur` out of
/// range: none current, plain text), the current one black on a bar. Colour 1 (orange), 2 (white)
/// on the selected list.
fn menu_list(b: &mut Batch, items: &[&str], cur: usize, x: f32, y: f32, n: usize, sel: bool) {
    let colour = if sel { 2 } else { 1 };
    let first = cur.min(items.len()).saturating_sub(n / 2);
    for (k, item) in items.iter().enumerate().skip(first).take(n) {
        let ly = y + (k - first) as f32 * 15.0;
        if k == cur {
            menu_bar(b, x, ly, b.fnt_width(item, 1.0), colour);
            menu_text(b, item, x, ly, MENU_BLACK, Align::Left);
        } else {
            menu_text(b, item, x, ly, colour, Align::Left);
        }
    }
}

/// CLASSIC 0x460468: a time as three right-aligned parts, "m:" ends at x + 0x50, "ss," at x + 0x68
/// and "cc" at x + 0x7c.
fn menu_time(b: &mut Batch, ms: u32, x: f32, y: f32, colour: usize) {
    menu_text(b, &format!("{}:", ms / 60000), x + 80.0, y, colour, Align::Right);
    menu_text(b, &format!("{:02},", (ms / 1000) % 60), x + 104.0, y, colour, Align::Right);
    menu_text(b, &format!("{:02}", (ms % 1000) / 10), x + 124.0, y, colour, Align::Right);
}
/// CLASSIC 0x481d58: a volume dial's needle at `cx, cy`, `angle` in 4096ths of a turn: a wedge from
/// the tip (0, -40) down to (+-3, -5) and a short tail to (0, 10), squashed to 5/6 in y like the
/// dial's face. `scale` is 1.0 on the Sound screen; the main dials call 0x481e64 with 2.6.
fn dial_needle(b: &mut Batch, cx: f32, cy: f32, angle: f32, scale: f32, colour: [f32; 4]) {
    let (s, c) = (angle * std::f32::consts::TAU / 4096.0).sin_cos();
    let (s, c) = (s * scale, c * scale);
    let p = |tx: f32, ty: f32| [cx + tx * s + ty * c, cy + 0.8333 * (tx * c - ty * s)];
    b.triangle([p(-3.0, -5.0), p(0.0, -40.0), p(3.0, -5.0)], colour);
    b.triangle([p(-3.0, -5.0), p(3.0, -5.0), p(0.0, 10.0)], colour);
}

/// CLASSIC 0x46052c: a two-digit flip counter at `x, y`. `pos` = each digit's position on the digit
/// strip (units first, 0..10): 21 rows of `numa` from `pos * 20` and of the tick ruler `numb` from
/// `-pos * 30` (as an unsigned number mod 20), digits 24 px apart.
fn flip_counter(b: &mut Batch, x: f32, y: f32, pos: &[f32]) {
    let (aw, _) = b.size("m:numa:0");
    let (bw, _) = b.size("m:numb:0");
    for (k, p) in pos.iter().enumerate() {
        let dx = x + 24.0 * (pos.len() - 1 - k) as f32;
        b.image_sub("m:numa:0", 0.0, ((p * 20.0) as i32 % 200) as f32, aw, 21.0, dx + 6.0, y);
        b.image_sub("m:numb:0", 0.0, ((-p * 30.0) as i32 as u32 % 20) as f32, bw, 21.0, dx, y);
    }
}
const DIM: [f32; 4] = [0.45, 0.45, 0.6, 1.0];
/// The dials' lavender (MENU.SPR palette 120 = menu colour 4 of the original's fill mask).
const LAVENDER: [f32; 4] = [190.0 / 255.0, 191.0 / 255.0, 248.0 / 255.0, 1.0];
/// CLASSIC 0x46d2d6: the stat values ease by frame ms * 0.005 per frame (about 5/s at 60 fps);
/// ours is the exponential form of it.
const GAUGE_RATE: f32 = 5.0;
/// CLASSIC 0x46d082 / 0x46c37c: the Car and Track screens' picture fades 0.003 per ms, out and in.
const PIC_FADE_RATE: f32 = 3.0;
/// Points per finishing position (results screen of the original: 12, 10, 8, 6, 4, 3, 2, 1).
const POINTS: [u32; 8] = [12, 10, 8, 6, 4, 3, 2, 1];
/// Seconds to keep racing after you finish before the results (AI cars finish meanwhile).
const RESULTS_AFTER: f32 = 12.0;

/// CLASSIC attract demo (GameType 5): the main menu idles this long before an AI race starts
/// (the original waits 30 000 ms, `0x46fb8d`, compared with the last input `0x46fd02`; design choice: 120 s)
const ATTRACT_IDLE_S: f32 = 120.0;
/// after a demo that ran to its end the next one follows after this long (10 000 ms, `0x470184`)
const ATTRACT_REARM_S: f32 = 10.0;
/// the demo race's laps (`0x4580af`: 25 laps, Hard, CatchUp off)
const ATTRACT_LAPS: u32 = 25;

/// The tracks the attract demo draws from (`0x4580d1..0x458118`): a regular track (Division n < 10) of tier
/// 3 or below (so not OkkunSpeedway), and not the track raced last unless it is the only one.
fn attract_tracks(divisions: &[u32], last: Option<usize>) -> Vec<usize> {
    let all: Vec<usize> = (0..divisions.len()).filter(|&i| divisions[i] / 100 <= 3 && divisions[i] % 100 < 10).collect();
    let fresh: Vec<usize> = all.iter().copied().filter(|&i| Some(i) != last).collect();
    if fresh.is_empty() { all } else { fresh }
}

/// The settings the attract demo overrides while it runs (the original forces InfoDetail, CheckpointInfo
/// and RoadMap to None, `0x4580aa..0x4581d6`); put back when it ends.
struct AttractSaved {
    game_type: GameType,
    hud: (usize, usize, usize, HudSettings),
}
/// Backdrop track for the car preview.
const GARAGE_TRACK: u32 = 1;
/// Needle sweep rate towards the selected item (1/s, exponential).
const NEEDLE_RATE: f32 = 10.0;
/// CLASSIC needle easing (1/s): 32 / 3000 per millisecond [O 0x481e64].
const CLASSIC_NEEDLE_RATE: f32 = 32.0 / 3.0;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Screen {
    Main,
    Single,
    Track,
    Car,
    Options,
    Graphics,
    /// volume knobs (options dial -> sound options)
    Sound,
    /// control layout and the Custom layout's bindings (options dial -> control options)
    Controls,
    /// name, team, colours and the race info options (options dial -> personal options)
    Personal,
    /// personal options -> statistics (stats.rs): overview, driving, by circuit, by car
    Statistics,
    /// in-race GFX Options / SFX Options, text lists like the original's
    PauseGfx,
    PauseSfx,
    /// REMASTER in-race HUD Settings (modern HUD layout, hud_layout.rs; 9.21)
    PauseHud,
    /// REMASTER in-race Control options (the modern pause menu's tab)
    PauseControls,
    /// REMASTER in-race Camera options (the modern pause menu's tab; GfxEdit with the camera rows)
    PauseCam,
    Credits,
    /// the original's intro movies (movie.rs, #42): `movies.pak` #0, #1, #2 before the main menu
    Intro,
    /// the season's result movie after the league results (0x42aa18, #43)
    LeagueMovie,
    Loading,
    Race,
    Pause,
    Results,
    /// main dial -> rankings: the .MHI top-ten tables (hiscore.rs)
    Rankings,
    /// single dial -> league: continue, view standings, load, new, delete (league.rs)
    League,
    /// the divisions of the current league
    LeagueTable,
    /// the saved leagues
    LeagueLoad,
    /// after a league race: your division on LResults.TGA (0x42a2b4)
    LeagueResults,
    /// single dial -> ghost mode: the "Ghost" screen (CLASSIC 0x462bc8, sprpos set 19), ghost race / replay race over the
    /// list of `Demos/*.MDE` recordings (demo.rs)
    Replays,
    /// REMASTER Ghost screen -> ANALYSE: Lap Analysis, two laps of one track compared (#429; no Classic counterpart)
    Analysis,
    /// main dial -> multi player: the original's first page (descriptor 6, sprpos set 9, 0x463f84); the lobby is story #7
    Multi,
}

/// GameType (config +0x7f4, parser 0x417b5f): Single 0, League 1, TimeAttack 2, GhostMode 3.
/// We have them all (#45), plus our own Knockout.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum GameType {
    Single,
    /// the next race of the current league: your division as the field, AID<division>
    League,
    /// alone on the track (0x458568 clears every other driver slot), with the gaps to the
    /// best lap and the track records at each split (0x45d23b)
    TimeAttack,
    /// Knockout (our design, not in the original): 8 cars, laps = cars - 1, the last car over the
    /// line at the end of each lap is out (motor_sim `Rules::Elimination`)
    Knockout,
    /// the recorded cars of an `.MDE` race run beside you (0x41bd18); the track, laps and CatchUp
    /// come from the file (demo.rs)
    GhostMode,
}

/// Record race: a finished recording waiting for "Save the recorded race?" (0x4d5715) and then
/// "Name the recorded race:" (0x4d572e) before it is written to `Demos/`.
pub struct DemoSave {
    mde: Mde,
    /// the requester's button
    sel: usize,
    /// None while asking, the typed name once the player said yes
    name: Option<String>,
}

/// Longest typed recording name.
const DEMO_NAME_MAX: usize = 24;

/// Knockout runs one lap fewer than the grid has cars (`Race::set_rules`); shown fixed on the Track screen.
const KNOCKOUT_LAPS: u32 = 7;

/// CLASSIC 0x46284c: a screen being left. The old screen keeps drawing with its clock `ms` counting down (no keys),
/// then the switch to `to` happens (`sel`: the item the new screen starts on).
#[derive(Clone, Copy)]
struct Leave {
    to: Screen,
    sel: Option<usize>,
    ms: f32,
}

impl Screen {
    /// A Classic screen whose draw goes through the sliding sprpos lookups (`Game::sp`): it slides in and out.
    fn slides(self) -> bool {
        matches!(
            self,
            Screen::Main | Screen::Single | Screen::Options | Screen::League | Screen::LeagueTable | Screen::LeagueLoad | Screen::Track | Screen::Car | Screen::Graphics | Screen::Sound | Screen::Controls | Screen::Personal | Screen::Statistics | Screen::Rankings | Screen::Credits | Screen::Replays | Screen::Multi
        )
    }

    /// A screen that plays a movie (movie.rs).
    pub fn is_movie(self) -> bool {
        matches!(self, Screen::Intro | Screen::LeagueMovie)
    }

    pub fn parse(s: &str) -> Option<Screen> {
        Some(match s {
            "main" => Screen::Main,
            "single" => Screen::Single,
            "track" => Screen::Track,
            "car" => Screen::Car,
            "options" => Screen::Options,
            "graphics" => Screen::Graphics,
            "sound" => Screen::Sound,
            "controls" => Screen::Controls,
            "personal" => Screen::Personal,
            "statistics" => Screen::Statistics,
            "pausesfx" => Screen::PauseSfx,
            "pausehud" => Screen::PauseHud,
            "pausecontrols" => Screen::PauseControls,
            "pausecam" => Screen::PauseCam,
            "pausegfx" => Screen::PauseGfx,
            "credits" => Screen::Credits,
            "intro" => Screen::Intro,
            "leaguemovie" => Screen::LeagueMovie,
            "loading" => Screen::Loading,
            "race" => Screen::Race,
            "pause" => Screen::Pause,
            "results" => Screen::Results,
            "rankings" => Screen::Rankings,
            "league" => Screen::League,
            "leaguetable" => Screen::LeagueTable,
            "leagueload" => Screen::LeagueLoad,
            "leagueresults" => Screen::LeagueResults,
            "replays" => Screen::Replays,
            "analysis" => Screen::Analysis,
            "multi" => Screen::Multi,
            _ => return None,
        })
    }
}

pub struct CarInfo {
    pub id: u32,
    pub name: String,
    /// Division: tier·100 + n (CarDef +0x100); league divisions limit the tier
    pub division: u32,
    pub top_speed: f32,
    pub accel: f32,
    /// the menu's `Handling` stat (CarDef +0x122c), what the GRIP dial shows
    pub handling: f32,
    /// REMASTER Car screen SPEC card: `Gears`, `Weight` (kg) and `Length` / `Width` / `Height` (m) of the .car
    pub gears: u32,
    pub weight: f32,
    pub size: [f32; 3],
    /// the paint zones the body has (`scene::paint_zones`): CarColour1..3 change something
    pub zones: [bool; 3],
    /// REMASTER menu car preview: the body's bounding sphere (mesh-space centre, radius; `scene::body_sphere`)
    pub sphere: ([f32; 3], f32),
    /// REMASTER Car screen floor: the body's footprint (`scene::body_footprint`)
    pub footprint: [f32; 4],
}

/// Cars on a single-race grid (the original draws 8 profiles, 0x46e110).
const GRID_SIZE: usize = 8;

/// Where settings live: %APPDATA%\Motorhead on Windows, $XDG_CONFIG_HOME/motorhead or
/// ~/.config/motorhead elsewhere.
pub fn settings_path() -> Option<PathBuf> {
    let dir = if cfg!(windows) {
        PathBuf::from(std::env::var_os("APPDATA")?).join("Motorhead")
    } else if let Some(x) = std::env::var_os("XDG_CONFIG_HOME").filter(|x| !x.is_empty()) {
        PathBuf::from(x).join("motorhead")
    } else {
        PathBuf::from(std::env::var_os("HOME")?).join(".config").join("motorhead")
    };
    Some(dir.join("settings.cfg"))
}

impl Settings {
    /// `cars`: the file number of each entry of the car list (`Car` is saved by file number).
    fn to_text(&self, cars: &[u32], tracks: &[u32]) -> String {
        let v = &self.view;
        let diff = match self.difficulty {
            Difficulty::Easy => "easy",
            Difficulty::Medium => "medium",
            Difficulty::Hard => "hard",
            Difficulty::Beginner => "beginner",
        };
        let b = |x: bool| x as u8;
        let c3 = |c: [u8; 3]| format!("{} {} {}", c[0], c[1], c[2]);
        let bright = (v.brightness * 100.0).round() as u32;
        format!(
            "// Motorhead settings (written by the game)\n\
             Car {}\nTrack {}\nLaps {}\nCatchup {}\nDifficulty {diff}\nHorn {}\n\
             CDMusic {}\nFrameLimit {}\nSFXVolume {}\nMusicVolume {}\nAmbientVolume {}\nEngineSound {}\n{}\
             Shadows {}\nHalos {}\nFog {}\nCarShading {}\nViewDistance {}\n\
             Monitor {}\nFullscreen {}\nScreenRatio {}\nMenuPicture {}\nResolution {}x{}\nVSync {}\n\
             Transmission {}\nRecordRace {}\nGhostCar {}\nControl {}\n\
             PlayerName {}\nTeamName {}\nPlayerColour {}\nCustomCarColours {}\nCarColour1 {}\nCarColour2 {}\nCarColour3 {}\nCarFinish1 {}\nCarFinish2 {}\nCarFinish3 {}\n\
             Brightness {bright} {bright} {bright}\nInfoDetail {}\nCheckpointInfo {}\nCheckpointDelay {}\nRoadMap {}\n\
             DetailMode {}\nSkidMarks {}\nNamePlates {}\nSkyImage {}\nMotionBlur {}\nLensFlares {}\nSparks {}\nSmoke {}\nTrackDetail {}\nCarDetail {}\nPerspectiveOn {}\nPerspective {}\n\
             GraphicsMode {}\nGraphicsModeSwitch {}\nRemasterCarPaint {}\nRemasterDynamicLights {}\nRemasterRenderAA {}\nRemasterRenderAAStrength {}\nRemasterPostAA {}\nRemasterPostAAStrength {}\nRemasterAnisotropy {}\nRemasterPixelLight {}\nRemasterPixelFog {}\nRemasterFogStyle {}\nRemasterFogStrength {}\nRemasterSkyHDRI {}\nRemasterClearCoat {}\nRemasterSigns {}\nRemasterWorldFill {}\nRemasterTreeModels {}\nRemasterWater {}\nRemasterWaterReflections {}\nRemasterLocalLight {}\nRemasterTexturePack {}\nRemasterGrass {}\nRemasterGrassRange {}\nRemasterColourGrade {}\nRemasterVignette {}\nRemasterGrain {}\nRemasterHDR {}\nRemasterTonemap {}\nRemasterExposure {}\nRemasterBloom {}\nRemasterSSAO {}\nRemasterHalos {}\nRemasterFlares {}\nRemasterShadows {}\nRemasterHud {}\nRemasterMenu {}\nRemasterMotionBlur {}\nRemasterSkidMarks {}\nRemasterCarModels {}\nRemasterCarLodDistance {}\nRemasterLightShadows {}\nRemasterShadowQuality {}\nRemasterRevMeter {}\n\
             HudCluster {}\nHudClocks {} {} {}\nHudLap {}\nHudPos {}\nHudMap {}\nHudProximity {}\nHudScale {}\nHudMargin {}\nHudMapMode {}\nHudMapRotate {}\nHudMapNumbers {}\nHudPlateFrame {}\nHudLapDelay {}\nHudLiveGaps {}\nHudSectorDelay {}\n\
             Intro {}\nCDLoop {}\nMenuConfirmation {}\nMenuCDTrack {}\nCameraStart {}\nCameraMirror {}\nCameraFreeLook {}\nCameraPullIn {}\nCameraFollow {}\n{}\
             PadStickDeadZone {}\nPadTriggerDeadZone {}\nPadSteerSensitivity {}\nPadVibration {}\n{}{}{}",
            cars.get(self.car).copied().unwrap_or(self.car as u32 + 1),
            // the track's file number (TrackN.trk), not its place in the Division-sorted list
            tracks.get(self.track).copied().unwrap_or(self.track as u32 + 1),
            self.laps,
            b(self.catchup),
            self.horn,
            b(self.music),
            self.fps_limit,
            self.vol_sfx,
            self.vol_music,
            self.vol_ambient,
            if self.engine_sound { "Simulated" } else { "Classic" },
            // the id keeps the pick, the name is what the menu shows while the output is not connected
            self.sound_device.as_ref().map_or(String::new(), |d| format!("SoundDevice {}\nSoundDeviceName {}\n", d.id, d.name)),
            b(v.shadows),
            b(v.halos),
            b(v.fog),
            v.car_shading.name(),
            (v.distance * 100.0).round() as u32,
            self.display.monitor + 1,
            b(self.display.fullscreen),
            if self.display.wide { "16:9" } else { "4:3" },
            if self.menu_stretch { "stretch" } else { "keep" },
            self.display.size.0,
            self.display.size.1,
            b(self.display.vsync),
            if self.manual { "manual" } else { "auto" },
            b(self.record_race),
            b(self.ghost_car),
            self.layout,
            self.player_name,
            self.team,
            c3(self.player_colour),
            b(self.custom_car_colours),
            c3(self.car_colours[0]),
            c3(self.car_colours[1]),
            c3(self.car_colours[2]),
            self.car_finish[0].key(),
            self.car_finish[1].key(),
            self.car_finish[2].key(),
            INFO_LEVELS[self.info_detail],
            LEVELS[self.checkpoint_info],
            self.checkpoint_delay,
            LEVELS[self.road_map],
            DETAIL_MODES[self.detail_mode],
            b(self.skid_marks),
            PLATES[self.name_plates],
            b(v.sky_image),
            b(v.motion_blur),
            b(v.lens_flares),
            b(v.sparks),
            b(v.smoke),
            TRACK_DETAILS[self.track_detail],
            v.car_detail.name(),
            PERSPECTIVE_ON[self.perspective_on],
            PERSPECTIVES[self.perspective],
            v.mode.name(),
            if self.mode_dissolve { "Dissolve" } else { "Instant" },
            b(v.remaster.enhanced_paint),
            b(v.remaster.dynamic_lights),
            v.remaster.render_aa.kind.name(),
            v.remaster.render_aa.strength,
            v.remaster.post_aa.kind.name(),
            v.remaster.post_aa.strength,
            level(v.remaster.anisotropy as u32),
            b(v.remaster.pixel_light),
            b(v.remaster.pixel_fog),
            v.remaster.fog_style.name(),
            v.remaster.fog_strength,
            b(v.remaster.sky_hdri),
            b(v.remaster.clear_coat),
            b(v.remaster.retro_signs),
            b(v.remaster.world_fill),
            b(v.remaster.tree_models),
            b(v.remaster.water),
            b(v.remaster.water_ssr),
            b(v.remaster.local_light),
            b(v.remaster.texture_pack),
            v.remaster.grass.name(),
            v.remaster.grass_range,
            v.remaster.grade,
            v.remaster.vignette,
            v.remaster.grain,
            b(v.remaster.hdr),
            v.remaster.tonemap.name(),
            v.remaster.exposure,
            v.remaster.bloom,
            v.remaster.ssao,
            if v.remaster.smooth_halos { "Smooth" } else { "Classic" },
            if v.remaster.smooth_flares { "Smooth" } else { "Classic" },
            SHADOW_STYLES[v.remaster.car_shadows as usize],
            HUD_STYLES[v.remaster.modern_hud as usize],
            HUD_STYLES[v.remaster.modern_menu as usize],
            BLUR_STYLES[v.remaster.velocity_blur as usize],
            SKID_STYLES[v.remaster.skids_detailed as usize],
            CAR_MODELS[v.remaster.generated_cars as usize],
            v.remaster.car_lod_distance,
            v.remaster.light_shadows.name(),
            v.remaster.shadow_quality.name(),
            v.remaster.rev_meter.to_array().map(|x| x.to_string()).join(" "),
            b(self.hud.cluster),
            b(self.hud.clocks[0]),
            b(self.hud.clocks[1]),
            b(self.hud.clocks[2]),
            b(self.hud.lap),
            b(self.hud.pos),
            b(self.hud.map),
            b(self.hud.proximity),
            self.hud.scale,
            self.hud.margin,
            MAP_MODES[self.hud.map_mode as usize],
            b(self.hud.map_rotate),
            b(self.hud.map_numbers),
            b(self.hud.plate_frame),
            self.hud.lap_delay,
            b(self.hud.live_gaps),
            self.hud.sector_delay,
            b(self.intro),
            b(self.cd_loop),
            CONFIRMATIONS[self.menu_confirmation],
            self.menu_cd_track,
            CAM_STARTS[self.cam.start],
            b(self.cam.mirror),
            b(self.cam.free_look),
            b(self.cam.pull_in),
            CAM_FOLLOWS[self.cam.follow],
            // the player's changes to the layouts, as the original rewrites the .clo files [design choice: kept here]
            self.binds.iter().flat_map(|(l, b)| b.iter().zip(TASKS).filter_map(move |(c, t)| Some(format!("Bind {l} {t} {}\n", c.as_ref()?.name())))).collect::<String>(),
            self.pad_tune[0],
            self.pad_tune[1],
            self.pad_tune[2],
            self.pad_tune[3],
            // the modern Controller page's car tasks
            self.pad.iter().zip(TASKS).filter_map(|(c, t)| Some(format!("Pad{t} {}\n", c.as_ref()?.name()))).collect::<String>(),
            self.short_keys.iter().enumerate().filter(|(_, k)| !k.is_empty()).map(|(i, k)| format!("ShortKey{i} {k}\n")).collect::<String>(),
            self.track_cd.iter().map(|(t, c)| format!("TrackCDTrack {t} {c}\n")).collect::<String>(),
        )
    }

    /// Apply a saved file; unknown keys and bad values are ignored, ranges are clamped.
    fn apply(&mut self, text: &str, cars: &[u32], tracks: &[u32], horns: usize) {
        for line in text.lines() {
            let mut it = line.split_whitespace();
            let (Some(k), Some(v)) = (it.next(), it.next()) else { continue };
            // the graphics keys are shared with motorview
            if view_settings::apply(&mut self.view, k, v, &mut it) {
                continue;
            }
            let n = v.parse::<i64>().ok();
            let u = |max: i64| n.map(|x| x.clamp(0, max) as u32);
            let on = n.map(|x| x != 0);
            match k.to_ascii_lowercase().as_str() {
                "car" => self.car = n.and_then(|x| cars.iter().position(|&id| id as i64 == x)).unwrap_or(self.car),
                "track" => self.track = n.and_then(|x| tracks.iter().position(|&id| id as i64 == x)).unwrap_or(self.track),
                "laps" => self.laps = u(99).unwrap_or(self.laps).max(1),
                "catchup" => self.catchup = on.unwrap_or(self.catchup),
                "difficulty" => {
                    self.difficulty = match v {
                        "easy" => Difficulty::Easy,
                        "hard" => Difficulty::Hard,
                        "beginner" => Difficulty::Beginner,
                        _ => Difficulty::Medium,
                    }
                }
                "horn" => self.horn = n.map_or(self.horn, |x| x.clamp(0, horns.max(1) as i64 - 1) as usize),
                "cdmusic" => self.music = on.unwrap_or(self.music),
                "menupicture" => self.menu_stretch = v != "keep",
                "framelimit" => self.fps_limit = n.map(|x| x as u32).filter(|x| FRAME_LIMITS.contains(x)).unwrap_or(self.fps_limit),
                "sfxvolume" => self.vol_sfx = u(100).unwrap_or(self.vol_sfx),
                "musicvolume" => self.vol_music = u(100).unwrap_or(self.vol_music),
                "ambientvolume" => self.vol_ambient = u(100).unwrap_or(self.vol_ambient),
                "enginesound" => self.engine_sound = v.eq_ignore_ascii_case("simulated"),
                // the rest of the line: ids and names hold spaces and commas (#260)
                "sounddevice" => {
                    let id = line.trim()[k.len()..].trim();
                    self.sound_device = (!id.is_empty() && id != "default").then(|| motor_audio::OutputDevice { id: id.to_string(), name: id.to_string() });
                }
                "sounddevicename" => {
                    let name = line.trim()[k.len()..].trim();
                    if let (Some(d), false) = (&mut self.sound_device, name.is_empty()) {
                        d.name = name.to_string();
                    }
                }
                "monitor" => self.display.monitor = n.map_or(self.display.monitor, |x| (x - 1).clamp(0, 15) as usize),
                "fullscreen" => self.display.fullscreen = on.unwrap_or(self.display.fullscreen),
                "screenratio" => self.display.wide = v != "4:3",
                "resolution" => {
                    let wh = v.split_once('x').and_then(|(w, h)| Some((w.parse().ok()?, h.parse().ok()?)));
                    if let Some(s) = wh {
                        self.display.size = s;
                    }
                }
                "vsync" => self.display.vsync = on.unwrap_or(self.display.vsync),
                "transmission" => self.manual = v.eq_ignore_ascii_case("manual"),
                "recordrace" => self.record_race = on.unwrap_or(self.record_race),
                "ghostcar" => self.ghost_car = on.unwrap_or(self.ghost_car),
                "control" => self.layout = v.to_string(),
                // names may hold spaces: the rest of the line
                "playername" => self.player_name = line.trim()[k.len()..].trim().chars().take(NAME_MAX).collect(),
                // the rest of the line, like the name: the keys hold spaces
                key if key.len() == 9 && key.starts_with("shortkey") && key.as_bytes()[8].is_ascii_digit() => {
                    let i = (key.as_bytes()[8] - b'0') as usize;
                    self.short_keys[i] = line.trim()[k.len()..].trim().chars().filter(|c| c.is_ascii_graphic() || *c == ' ').take(SHORT_KEY_MAX).collect();
                }
                "teamname" => self.team = v.chars().take(TEAM_MAX).collect(),
                "playercolour" => self.player_colour = rgb(&mut it, v).unwrap_or(self.player_colour),
                "customcarcolours" => self.custom_car_colours = on.unwrap_or(self.custom_car_colours),
                "carcolour1" | "carcolour2" | "carcolour3" => {
                    let i = (k.as_bytes()[9] - b'1') as usize;
                    self.car_colours[i] = rgb(&mut it, v).unwrap_or(self.car_colours[i]);
                }
                "carfinish1" | "carfinish2" | "carfinish3" => {
                    let i = (k.as_bytes()[9] - b'1') as usize;
                    self.car_finish[i] = PaintFinish::from_key(v).unwrap_or(self.car_finish[i]);
                }
                "infodetail" => self.info_detail = pick(v, &INFO_LEVELS).unwrap_or(self.info_detail),
                "checkpointinfo" => self.checkpoint_info = pick(v, &LEVELS).unwrap_or(self.checkpoint_info),
                "checkpointdelay" => self.checkpoint_delay = u(DELAY_MAX as i64).unwrap_or(self.checkpoint_delay),
                "roadmap" => self.road_map = pick(v, &LEVELS).unwrap_or(self.road_map),
                "detailmode" => self.detail_mode = pick(v, &DETAIL_MODES).unwrap_or(self.detail_mode),
                "skidmarks" => self.skid_marks = on.unwrap_or(self.skid_marks),
                "nameplates" => self.name_plates = pick(v, &PLATES).unwrap_or(self.name_plates),
                "graphicsmodeswitch" => self.mode_dissolve = v.eq_ignore_ascii_case("dissolve"),
                "hudcluster" => self.hud.cluster = on.unwrap_or(self.hud.cluster),
                // total, lap and best lap time
                "hudclocks" => {
                    let rest: Vec<bool> = it.by_ref().take(2).filter_map(|x| x.parse::<i64>().ok()).map(|x| x != 0).collect();
                    if let (Some(a), [b, c]) = (on, rest.as_slice()) {
                        self.hud.clocks = [a, *b, *c];
                    }
                }
                "hudlap" => self.hud.lap = on.unwrap_or(self.hud.lap),
                "hudpos" => self.hud.pos = on.unwrap_or(self.hud.pos),
                "hudmap" => self.hud.map = on.unwrap_or(self.hud.map),
                "hudproximity" => self.hud.proximity = on.unwrap_or(self.hud.proximity),
                "hudscale" => {
                    let (lo, hi) = SCALE_RANGE;
                    self.hud.scale = n.map_or(self.hud.scale, |x| (x.clamp(lo as i64, hi as i64) as u32) / SCALE_STEP * SCALE_STEP);
                }
                "hudmargin" => self.hud.margin = u(100).unwrap_or(self.hud.margin),
                "hudmapmode" => self.hud.map_mode = pick(v, &MAP_MODES).map_or(self.hud.map_mode, |i| if i == 1 { MapMode::Close } else { MapMode::Circuit }),
                "hudmaprotate" => self.hud.map_rotate = on.unwrap_or(self.hud.map_rotate),
                "hudmapnumbers" => self.hud.map_numbers = on.unwrap_or(self.hud.map_numbers),
                "hudplateframe" => self.hud.plate_frame = on.unwrap_or(self.hud.plate_frame),
                "hudlivegaps" => self.hud.live_gaps = on.unwrap_or(self.hud.live_gaps),
                "hudsectordelay" => self.hud.sector_delay = u(crate::hud_layout::LAP_DELAY_MAX as i64).unwrap_or(self.hud.sector_delay),
                "hudlapdelay" => self.hud.lap_delay = u(crate::hud_layout::LAP_DELAY_MAX as i64).unwrap_or(self.hud.lap_delay),
                "trackdetail" => self.track_detail = pick(v, &TRACK_DETAILS).unwrap_or(self.track_detail),
                "perspectiveon" => self.perspective_on = pick(v, &PERSPECTIVE_ON).unwrap_or(self.perspective_on),
                "perspective" => self.perspective = pick(v, &PERSPECTIVES).unwrap_or(self.perspective),
                "intro" => self.intro = on.unwrap_or(self.intro),
                "cdloop" => self.cd_loop = on.unwrap_or(self.cd_loop),
                "menuconfirmation" => self.menu_confirmation = pick(v, &CONFIRMATIONS).unwrap_or(self.menu_confirmation),
                "camerastart" => self.cam.start = pick(v, &CAM_STARTS).unwrap_or(self.cam.start),
                "cameramirror" => self.cam.mirror = on.unwrap_or(self.cam.mirror),
                "camerafreelook" => self.cam.free_look = on.unwrap_or(self.cam.free_look),
                "camerapullin" => self.cam.pull_in = on.unwrap_or(self.cam.pull_in),
                "camerafollow" => self.cam.follow = pick(v, &CAM_FOLLOWS).unwrap_or(self.cam.follow),
                "menucdtrack" => self.menu_cd_track = n.map_or(self.menu_cd_track, |x| x.clamp(2, 99) as u32),
                "trackcdtrack" => {
                    if let (Some(t), Some(c)) = (n, it.next().and_then(|x| x.parse::<i64>().ok())) {
                        self.track_cd.insert(t.clamp(1, 99) as u32, c.clamp(2, 99) as u32);
                    }
                }
                "bind" => {
                    let t = it.next().and_then(|t| TASKS.iter().position(|x| x.eq_ignore_ascii_case(t)));
                    if let (Some(i), Some(c)) = (t, it.next().and_then(Code::parse)) {
                        self.binds.entry(v.to_ascii_lowercase()).or_insert([None; TASKS.len()])[i] = Some(c);
                    }
                }
                "padstickdeadzone" | "padtriggerdeadzone" | "padsteersensitivity" | "padvibration" => {
                    let i = ["padstickdeadzone", "padtriggerdeadzone", "padsteersensitivity", "padvibration"].iter().position(|x| k.eq_ignore_ascii_case(x)).unwrap_or(0);
                    let (lo, hi) = PAD_TUNE_RANGE[i];
                    self.pad_tune[i] = n.map_or(self.pad_tune[i], |x| x.clamp(lo as i64, hi as i64) as u32);
                }
                _ if k.get(..3).is_some_and(|p| p.eq_ignore_ascii_case("pad")) => {
                    let t = k.get(3..).and_then(|t| TASKS[..CAR_TASKS].iter().position(|x| x.eq_ignore_ascii_case(t)));
                    if let (Some(i), Some(c)) = (t, PadIn::parse(v)) {
                        self.pad[i] = Some(c);
                    }
                }
                _ => {
                    let t = k.get(6..).filter(|_| k.get(..6).is_some_and(|p| p.eq_ignore_ascii_case("custom")));
                    // the files before layouts were all editable: Custom<Task> Code
                    if let (Some(i), Some(c)) = (t.and_then(|t| TASKS[..CAR_TASKS].iter().position(|x| x.eq_ignore_ascii_case(t))), Code::parse(v)) {
                        self.binds.entry("custom".into()).or_insert([None; TASKS.len()])[i] = Some(c);
                    }
                }
            }
        }
        if self.player_name.is_empty() {
            self.player_name = "Player".into();
        }
        // keep the resolution one of the ratio's list
        let list = resolutions(self.display.wide);
        if !list.contains(&self.display.size) {
            self.display.size = list[0];
        }
    }
}

/// Sound events raised by the game (menu events are menusound.cfg numbers).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sfx {
    Menu(u32),
    Horn,
    /// a cheat code was accepted: the horn at this index
    Honk(usize),
    /// REMASTER Car screen: the horn just chosen, tried once (cuts the one before)
    HornTry,
}

/// menusound.cfg event numbers (the list at the end of that file).
pub mod ev {
    pub const ESCAPE: u32 = 1;
    pub const ARROW_LEFT: u32 = 4;
    pub const ARROW_RIGHT: u32 = 5;
    pub const CURSOR_UP: u32 = 7;
    pub const CURSOR_DOWN: u32 = 8;
    pub const SELECT: u32 = 11;
    /// a requester opens (pagein.wav, 0x4626f0)
    pub const REQUESTER: u32 = 2;
    /// a list takes / gives back the focus (return.wav)
    pub const LIST_IN: u32 = 12;
    pub const LIST_OUT: u32 = 13;
    pub const SELECT_ERROR: u32 = 15;
    /// Enter on a locked car / track (0x46d8bb / 0x46ca5b): the original's events 36 and 37 have no sample in menusound.cfg, so silence
    pub const LOCKED_CAR: u32 = 36;
    pub const LOCKED_TRACK: u32 = 37;
    // the Controls screen's own events (0x4691e4)
    pub const SELECT_CONTROL: u32 = 14;
    pub const COLUMN_LEFT: u32 = 26;
    pub const COLUMN_RIGHT: u32 = 27;
    pub const COLUMN_ERROR: u32 = 28;
    pub const CONTROL_OK: u32 = 29;
    pub const CONTROL_SWITCH: u32 = 30;
    pub const CONTROL_ERROR: u32 = 31;
    pub const SWITCH_ON: u32 = 19;
    pub const SWITCH_OFF: u32 = 20;
    // the Sound screen's own events (0x469c18)
    pub const CURSOR_END: u32 = 9;
    pub const VOLUME_UP: u32 = 16;
    pub const VOLUME_DOWN: u32 = 17;
    pub const VOLUME_LIMIT: u32 = 18;
    pub const SWITCH_SAME: u32 = 21;
    pub const SONG_UP: u32 = 22;
    pub const SONG_UP_LIMIT: u32 = 23;
    pub const SONG_DOWN: u32 = 24;
    pub const SONG_DOWN_LIMIT: u32 = 25;
}

/// Line-object wireframe: vertices and edges.
pub type Wire = (Vec<[f32; 3]>, Vec<[u32; 2]>);

pub struct TrackInfo {
    pub n: u32,
    pub name: String,
    /// Division: tier·100 + n, n >= 10 a bonus track (TrackDef +0); picks the league seasons
    pub division: u32,
    pub laps: u32,
    /// CDTrack: the audio CD track played during the race
    pub cd_track: u32,
    /// TrackMapInfo: x0, camera y, z0, angle (deg)
    pub map_info: [f32; 4],
    /// marker correction for two 48-pixel-wide maps, hard-coded in the original
    pub map_offset: (f32, f32),
    /// LineObjectName wireframe (game space), for the track screen
    pub wire: Option<Wire>,
    /// CLASSIC: the wireframe's vertices centred and scaled with the one factor of the first
    /// track of the original's list (0x461718), so the outlines keep their sizes relative to each other
    pub outline: Vec<[f32; 3]>,
    /// REMASTER Glass HUD track screen (#294): the lap from the centreline (elevation, outline,
    /// sectors, turns); None when the spline is missing
    pub profile: Option<motor_sim::race::TrackProfile>,
    /// the .trk `Length` (m) [O], the game's own figure; the centreline length without it
    pub length_m: f32,
    /// the .trk `Location` [O] ("Arizona, USA"), on the Remaster loading screen
    pub location: String,
    /// the .trk `Length` as written ("4725 meters") [O], drawn as is by the Classic loading screen (TrackDef +0x308)
    pub length_text: String,
}

/// REMASTER a paint's finish (#135, CarFinish1..3): Gloss is the original's paint; Metallic is metal paint and Pearl a
/// colour-shifting pearl paint in Remaster (#136, #351, `scene::CarPaint`, 09-remaster 9.41) [design choice: the finish
/// belongs to each paint zone, picked on the palette's tabs].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PaintFinish {
    #[default]
    Gloss,
    Metallic,
    Pearl,
}

impl PaintFinish {
    pub const ALL: [PaintFinish; 3] = [PaintFinish::Gloss, PaintFinish::Metallic, PaintFinish::Pearl];
    pub fn key(self) -> &'static str {
        match self {
            PaintFinish::Gloss => "gloss",
            PaintFinish::Metallic => "metallic",
            PaintFinish::Pearl => "pearl",
        }
    }
    /// The renderer's finish (`scene::CarPaint`, `LivePaint`).
    pub fn scene(self) -> scene::Finish {
        match self {
            PaintFinish::Gloss => scene::Finish::Gloss,
            PaintFinish::Metallic => scene::Finish::Metallic,
            PaintFinish::Pearl => scene::Finish::Pearl,
        }
    }
    pub fn from_key(v: &str) -> Option<PaintFinish> {
        PaintFinish::ALL.into_iter().find(|f| v.eq_ignore_ascii_case(f.key()))
    }
    pub fn name(self) -> &'static str {
        match self {
            PaintFinish::Gloss => "GLOSS",
            PaintFinish::Metallic => "METALLIC",
            PaintFinish::Pearl => "PEARL",
        }
    }
}

#[derive(Clone)]
pub struct Settings {
    pub car: usize,
    pub track: usize,
    pub laps: u32,
    pub catchup: bool,
    pub difficulty: Difficulty,
    pub horn: usize,
    /// play the CD soundtrack from the Music folder
    pub music: bool,
    /// frames per second cap, 0 = unlimited (one of FRAME_LIMITS)
    pub fps_limit: u32,
    /// CLASSIC menus fill the window (the original's fullscreen mode does), else keep their 16:10 shape
    /// with black bars (Display row "Menu picture", key `MenuPicture`)
    pub menu_stretch: bool,
    /// volumes in percent (the original keeps 0..254 and shows value*100/254, 0x428986)
    pub vol_sfx: u32,
    pub vol_music: u32,
    pub vol_ambient: u32,
    /// REMASTER: the engine sound baked from engine-sim (engine_sim.rs) instead of the original loops
    pub engine_sound: bool,
    /// the audio output the player picked on the Sound screen (#260); None = the system default
    pub sound_device: Option<motor_audio::OutputDevice>,
    pub view: View,
    pub display: Display,
    /// manual gearbox (car screen "Transmission")
    pub manual: bool,
    /// car screen "Record race": record every race as an `.MDE` (demo.rs) and offer it for saving
    /// (0x46f93b); default Off
    pub record_race: bool,
    /// car screen "Ghost car" (Time Attack only): race your best run on this track and car
    /// (our own feature, 08 §8.9)
    pub ghost_car: bool,
    /// LayoutName of the control layout in use
    pub layout: String,
    /// the player's changes to the layouts' bindings, by lowercase layout name (the original rewrites
    /// the layout's Game/*.clo; we keep them here and leave the install alone)
    pub binds: BTreeMap<String, [Option<Code>; TASKS.len()]>,
    /// REMASTER the pad's car tasks (modern Controller page; `DEFAULT_PAD_BIND`)
    pub pad: [Option<PadIn>; CAR_TASKS],
    /// pad stick dead zone, trigger dead zone (percent of travel), steering sensitivity (percent)
    pub pad_tune: [u32; 4],
    /// Personal: PlayerName, TeamName (< 5 chars), PlayerColour
    pub player_name: String,
    pub team: String,
    pub player_colour: [u8; 3],
    /// CustomCarColours, CarColour1..3: repaint the player's car in races (scene::repaint)
    pub custom_car_colours: bool,
    pub car_colours: [[u8; 3]; 3],
    /// REMASTER CarFinish1..3 (#135): each paint's finish, picked on the Livery tab's palette; drawn in Remaster (#136)
    pub car_finish: [PaintFinish; 3],
    /// ShortKey0..9 (the Personal screen's "Key N:" rows, config +0x2dc, 127 characters each): stored
    /// only. The retail build never reads them after the menu [O static, 08 §8.5], so they are
    /// probably chat macros of the multiplayer lobby [I]
    pub short_keys: [String; 10],
    /// InfoDetail (an index into INFO_LEVELS), CheckpointInfo, RoadMap (LEVELS) (config +0x994/+0x998/+0x9a0)
    pub info_detail: usize,
    pub checkpoint_info: usize,
    pub road_map: usize,
    /// CheckpointDelay in seconds, 0..`DELAY_MAX` (+0x99c, kept in ms by the original, which offers 0..10: `DELAY_CLASSIC_MAX`)
    pub checkpoint_delay: u32,
    /// DetailMode: an index into DETAIL_MODES (+0x9a4); the presets are `apply_detail`
    pub detail_mode: usize,
    /// design choice (09-remaster 9.1 rule 2): F9 fades between the modes over 0.3 s
    pub mode_dissolve: bool,
    /// TrackDetail (0 Medium, 1 High): kept for the presets, no effect yet (#272). CarDetail is
    /// `view.car_detail`, which the car draw reads (Classic body levels)
    pub track_detail: usize,
    /// PerspectiveOn (0 Selective, 1 All; +0x9d4) and Perspective (0 Normal, 1 Good; +0x9d8): the
    /// original's software renderer's texture perspective correction. Kept for the presets and
    /// the file, no effect: the hardware paths are always perspective-correct (08 §8.5)
    pub perspective_on: usize,
    pub perspective: usize,
    pub skid_marks: bool,
    /// the Camera tab
    pub cam: CamOpts,
    /// NamePlates: an index into PLATES (+0x9c0)
    pub name_plates: usize,
    /// Intro: play the intro movies at start-up (our key, the original always plays them: 0x457ecc)
    pub intro: bool,
    /// CDLoop: repeat the CD track
    pub cd_loop: bool,
    /// MenuConfirmation: an index into CONFIRMATIONS
    pub menu_confirmation: usize,
    /// MenuCDTrack: the CD track played in the menus
    pub menu_cd_track: u32,
    /// TrackCDTrack N M: the CD track the player assigned to race track N (file number) on the
    /// Sound screen's song-to-track table; only the tracks that differ from their .trk CDTrack
    /// [design choice: the original keeps the assignment until the game is closed, [I]]
    pub track_cd: BTreeMap<u32, u32>,
    /// REMASTER HUD Settings (the modern HUD's elements, scale and margins)
    pub hud: HudSettings,
}

/// The three-level HUD options (string table at file offset 0xacfb0).
pub const LEVELS: [&str; 3] = ["None", "Selective", "All"];
/// InfoDetail's values: the three levels and Custom, the HUD Settings toggles as the player
/// set them [design choice, bug #181; the original dial has three].
pub const INFO_LEVELS: [&str; 4] = ["None", "Selective", "All", "Custom"];
pub const PLATES: [&str; 3] = ["None", "Flat", "Transparent"];
pub const DETAIL_MODES: [&str; 5] = ["Low", "Medium", "High", "Max", "Custom"];
const CUSTOM_DETAIL: usize = 4;
const CONFIRMATIONS: [&str; 3] = ["Off", "On", "NoExit"];
const TRACK_DETAILS: [&str; 2] = ["Medium", "High"];
/// The Remaster Carshading row's index (Gouraud, Reflection, Enhanced); Classic's Flat shows as Gouraud
fn remaster_shading(c: CarShading, enhanced: bool) -> usize {
    if enhanced {
        2
    } else {
        usize::from(c == CarShading::Reflection)
    }
}
const PERSPECTIVE_ON: [&str; 2] = ["Selective", "All"];
const PERSPECTIVES: [&str; 2] = ["Normal", "Good"];
/// CLASSIC 0x4bdba8: the colour strip's cells as the exe holds them. The game replaces them at
/// start-up with entries 255..224 of `Data/Default.ACT` (0x45f480, `parse_strip`); this table is
/// what it shows when the file is missing, 15 colours then black.
const STRIP_EXE: [[u8; 3]; 15] = [
    [0x80, 0x80, 0x80],
    [0xff, 0x80, 0x80],
    [0x80, 0xff, 0x80],
    [0xff, 0xff, 0x80],
    [0x80, 0x80, 0xff],
    [0xff, 0x80, 0xff],
    [0x80, 0xff, 0xff],
    [0xff, 0xff, 0xff],
    [0x80, 0x60, 0x40],
    [0x40, 0x60, 0x80],
    [0x30, 0x30, 0x30],
    [0x30, 0x30, 0x50],
    [0x50, 0x30, 0x00],
    [0x80, 0x60, 0x30],
    [0xa0, 0x80, 0x50],
];
/// The strip's cell count and the cell geometry (0x45f634): swatch 32 x 10, cells 12 apart from
/// 44 px right of the swatch, 10 x 8 each.
const STRIP_CELLS: usize = 32;

/// CLASSIC 0x45f480: cell i is entry 255 - i of the palette file (R G B bytes at 0x2fd - 3i).
fn parse_strip(act: &[u8]) -> Option<[[u8; 3]; STRIP_CELLS]> {
    let mut cells = [[0u8; 3]; STRIP_CELLS];
    for (i, c) in cells.iter_mut().enumerate() {
        let o = 3 * (255 - i);
        *c = <[u8; 3]>::try_from(act.get(o..o + 3)?).ok()?;
    }
    Some(cells)
}

/// The strip of an install: `Data/Default.ACT`, else the exe's own table.
fn load_strip(root: &Path) -> [[u8; 3]; STRIP_CELLS] {
    motor_data::fs::read(root, "Data/Default.ACT")
        .ok()
        .and_then(|d| parse_strip(&d))
        .unwrap_or_else(|| std::array::from_fn(|i| STRIP_EXE.get(i).copied().unwrap_or([0; 3])))
}

/// CLASSIC 0x45f54c: the cell nearest to `c` (squared RGB distance, the first of equals wins:
/// with several black cells only the first one can be reached).
fn strip_index(strip: &[[u8; 3]; STRIP_CELLS], c: [u8; 3]) -> usize {
    let dist = |t: &[u8; 3]| (0..3).map(|k| (t[k] as i32 - c[k] as i32).pow(2)).sum::<i32>();
    let mut best = (i32::MAX, 0);
    for (i, t) in strip.iter().enumerate() {
        let d = dist(t);
        if d < best.0 {
            best = (d, i);
        }
    }
    best.1
}

/// CLASSIC 0x45f634: Left / Right move to the previous / next cell from the nearest one, round
/// the ends, and set the colour to that cell.
fn strip_step(strip: &[[u8; 3]; STRIP_CELLS], c: [u8; 3], fwd: bool) -> [u8; 3] {
    let i = strip_index(strip, c);
    strip[if fwd { (i + 1) % STRIP_CELLS } else { (i + STRIP_CELLS - 1) % STRIP_CELLS }]
}
const NAME_MAX: usize = 15;
/// ShortKey length (128-byte fields, 0x46733a)
const SHORT_KEY_MAX: usize = 127;
/// LeagueName length (the .LGF key, 0x450204)
const LEAGUE_NAME_MAX: usize = 15;
/// TeamName is "< 5 chars" (config template comment)
const TEAM_MAX: usize = 4;

impl Default for Settings {
    fn default() -> Self {
        Settings {
            car: 0,
            track: 0,
            laps: 3,
            catchup: true,
            difficulty: Difficulty::Medium,
            horn: 2,
            music: true,
            fps_limit: 60,
            menu_stretch: true,
            vol_sfx: 80,
            vol_music: 70,
            vol_ambient: 80,
            engine_sound: false,
            sound_device: None,
            // Classic by default, to stay faithful (09-remaster 9.7)
            view: View {
                brightness: 1.0,
                fog: true,
                show_line: false,
                shadows: true,
                halos: true,
                car_shading: CarShading::Reflection,
                car_detail: CarDetail::High,
                distance: 1.0,
                fov_x: motorview::render::FOV_X.get(),
                sky_image: true,
                motion_blur: false,
                lens_flares: true,
                smoke: true,
                sparks: true,
                mode: RenderMode::Classic,
                remaster: RemasterView::default(),
                debug: Default::default(),
            },
            display: Display::default(),
            manual: false,
            record_race: false,
            ghost_car: true,
            layout: String::new(),
            binds: BTreeMap::new(),
            pad: DEFAULT_PAD,
            pad_tune: PAD_TUNE_DEFAULT,
            player_name: "Player".into(),
            team: String::new(),
            player_colour: [60, 100, 255],
            custom_car_colours: false,
            car_colours: [[255, 50, 40], [255, 255, 255], [40, 40, 40]],
            car_finish: [PaintFinish::Gloss; 3],
            short_keys: Default::default(),
            // the original's defaults (0x417454): CheckpointInfo All, 4000 ms, RoadMap All;
            // InfoDetail All [I]
            info_detail: 2,
            checkpoint_info: 2,
            road_map: 2,
            checkpoint_delay: 4,
            // Custom keeps our rows as they are (design choice)
            detail_mode: CUSTOM_DETAIL,
            mode_dissolve: false,
            track_detail: 1,
            perspective_on: 0,
            perspective: 0,
            skid_marks: true,
            cam: CamOpts::default(),
            name_plates: 1,
            intro: true,
            cd_loop: true,
            menu_confirmation: 1,
            menu_cd_track: 3,
            track_cd: BTreeMap::new(),
            hud: HudSettings::default(),
        }
    }
}

/// Index of `v` in `names`, ignoring case.
/// A Remaster level (MSAA samples, anisotropy) in the settings file: "Off" for 1, else "Nx".
fn level(n: u32) -> String {
    if n <= 1 { "Off".into() } else { format!("{n}x") }
}

fn rgb<'a>(it: &mut std::str::SplitWhitespace<'a>, first: &'a str) -> Option<[u8; 3]> {
    let mut c = [0u8; 3];
    for (i, v) in std::iter::once(first).chain(it.by_ref().take(2)).enumerate() {
        c[i] = v.parse::<i64>().ok()?.clamp(0, 255) as u8;
    }
    Some(c)
}

impl Settings {
    /// DetailMode presets, 0x417020 (perspective correction is set there too: stored, no effect).
    /// Every preset turns MotionBlur off. Low's Flat car shading is Classic only (Remaster draws
    /// it as Gouraud). The Remaster options are not part of the presets.
    pub fn apply_detail(&mut self) {
        // Shadow / Smoke / Sparks: Low off/off/off, Medium on/off/on, High and Max all on
        let (refl, flat) = (CarShading::Reflection, CarShading::Flat);
        let (shadows, smoke, sparks, shading, plates, track, car, distance) = match self.detail_mode {
            0 => (false, false, false, flat, 1, 0, 0, 0.5),
            1 => (true, false, true, refl, 1, 1, 1, 0.6),
            2 => (true, true, true, refl, 2, 1, 2, 0.8),
            3 => (true, true, true, refl, 2, 1, 2, 1.0),
            _ => return,
        };
        let v = &mut self.view;
        (v.smoke, v.sparks, v.lens_flares) = (smoke, sparks, true);
        (v.shadows, v.halos, v.car_shading, v.sky_image, v.motion_blur, v.distance) = (shadows, true, shading, true, false, distance);
        (self.skid_marks, self.name_plates, self.track_detail, v.car_detail) = (true, plates, track, CarDetail::ALL[car]);
        // Max: all polygons corrected, accuracy Good
        (self.perspective_on, self.perspective) = (usize::from(self.detail_mode == 3), usize::from(self.detail_mode == 3));
    }
}

/// Window / screen settings (graphic options), applied by the window layer in main.rs.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Display {
    /// index into `Game::monitors` (clamped when used)
    pub monitor: usize,
    pub fullscreen: bool,
    /// 16:9 resolutions, else 4:3
    pub wide: bool,
    /// physical pixels, one of `resolutions(wide)`
    pub size: (u32, u32),
    pub vsync: bool,
}

impl Default for Display {
    fn default() -> Self {
        Display { monitor: 0, fullscreen: false, wide: true, size: (1280, 720), vsync: false }
    }
}

/// The common resolutions of each screen ratio, small to large (design choice).
pub fn resolutions(wide: bool) -> &'static [(u32, u32)] {
    if wide {
        &[(1280, 720), (1366, 768), (1600, 900), (1920, 1080), (2560, 1440), (3840, 2160)]
    } else {
        &[(640, 480), (800, 600), (1024, 768), (1152, 864), (1280, 960), (1400, 1050), (1600, 1200)]
    }
}

pub struct RaceState {
    pub sess: Session,
    /// #335 the race's sounds decoded on the loading worker, taken by the audio at the start
    pub sounds: Option<crate::audio::RaceSounds>,
    pub track: usize,
    pub finished_for: Option<f32>,
    /// (seconds left, lap time ms, best?, delta ms against the best earlier lap) after a lap is completed
    pub lap_flash: Option<(f32, u32, bool, Option<i64>)>,
    /// checkpoint split display: (seconds left, splits index, lap time at the split)
    pub split: Option<(f32, usize, u32)>,
    /// REMASTER sector delta (#409): (seconds left, sector 1 / 2, delta ms against the best earlier lap at this checkpoint)
    pub sector_flash: Option<(f32, u32, i64)>,
    last_lap: u32,
    last_cp: Option<(u32, u32)>,
    pub cam: Camera,
    /// the track's best lap and best race over these laps (.CL / .CR)
    records: hiscore::Records,
    /// Time Attack gaps (Lap, Lap Ever, Race Ever) at the last split or lap, taken before
    /// the records take the new times
    pub ta_gaps: [Option<i64>; 3],
    /// REMASTER proximity arrows (`proximity`): per racer, the arrow's fade 0..1
    pub prox: Vec<f32>,
    /// REMASTER close minimap: the ground it shows across (m), eased by speed (`hud::map_zoom_update`)
    pub map_zoom: f32,
    /// the map easter eggs (RuhrStadt club message, Atlantika roof scroller; #366, `eggs.rs`)
    pub eggs: motor_sim::eggs::MapEggs,
    /// the player's laps already offered to the records, and the race
    rec_laps: u32,
    rec_finished: bool,
    /// a recording being played back: it and the next frame (the sim takes its inputs and
    /// frame lengths from it)
    playback: Option<(Recording, usize)>,
    /// the race being recorded (Record race on, a driven non-league race); taken when saved
    recording: Option<Recording>,
    /// Ghost mode: the recorded cars beside the player (demo.rs)
    pub demo: Option<DemoCars>,
    /// Record race: the `.MDE` being written (taken when saved)
    demo_rec: Option<DemoRecorder>,
    /// Ghost mode: the recording's last record has passed
    demo_over: bool,
    /// demo play: the block of the recorded player, posed as the (remote) racer
    pub watch: Option<usize>,
    /// demo play: the playback controls (REMASTER Race line HUD only, `playback.rs`, #346); kept in Classic too, unused
    pub pb: Option<Playback>,
    /// a console message of the original (0x41d27e: "Ghost play started!", "End of ghost play!",
    /// "Race recording started!"): seconds left and the text
    pub notice: Option<(f32, String)>,
    /// a respawn done since the last stepped frame, for the recording
    pending_respawn: Option<usize>,
    /// the player skipped the start fly-by since the last recorded frame (#216)
    pending_skip: bool,
    /// Time Attack: the best run of this track and car, as poses by race clock (replay.rs)
    /// Time Attack: the best lap so far, replayed beside the live car, and the recorder of the lap
    /// being driven (every faster lap replaces the ghost at once and is saved)
    ghost: Option<GhostTrack>,
    lap_rec: Option<LapRecorder>,
    /// laps of the player already offered to the ghost
    ghost_laps: u32,
    /// the statistics of this race so far (stats.rs; committed at the results, a restart or a quit)
    run: stats::Run,
}

/// (mesh index, model matrix) pairs, as the renderer takes them.
type Models = Vec<(usize, Mat4)>;

impl RaceState {
    /// What the player's car does to the pad's rumble this frame (#383); `None` outside the race
    /// proper (before the green light, after the finish).
    fn rumble_in(&self, player: usize) -> Option<crate::rumble::RumbleIn> {
        let race = &self.sess.race;
        let me = &race.racers[player];
        if !race.started() || me.progress.done() {
            return None;
        }
        let car = &me.car;
        let mats = &race.world.materials;
        let mut i = crate::rumble::RumbleIn { speed: car.vel.length(), wheelspin: car.wheelspin, brake_locked: car.brake_locked, ..Default::default() };
        for w in 0..4 {
            if let Some(m) = car.wheel_mat[w].filter(|_| car.wheel_contact[w]) {
                i.bumpy[w] = Some(mats[m].bumpy);
                i.kerb[w] = mats[m].name.eq_ignore_ascii_case("kerbing");
            }
        }
        // #374: a hit kicks once, on the frame whose tick had it (between ticks the flags are the last tick's)
        if race.ticks_last_frame() > 0 {
            i.hit = car.impact.max(car.bump.map_or(0.0, |b| b.impact.abs()));
        }
        Some(i)
    }

    /// Statistics: this frame of the player's car goes into the run (nothing before the green
    /// light or after the finish).
    fn count_run(&mut self, dt: f32, player: usize) {
        let race = &self.sess.race;
        let me = &race.racers[player];
        if !race.started() || me.progress.done() {
            return;
        }
        let order = race.standings();
        let pos = order.iter().position(|&s| s == player).unwrap_or(0);
        let racing = |s: &usize| !race.racers[*s].progress.done();
        let ahead: Vec<usize> = order[..pos].iter().copied().filter(racing).collect();
        let behind: Vec<usize> = order[(pos + 1).min(order.len())..].iter().copied().filter(racing).collect();
        let car = &me.car;
        // #374: a hit counts once, on the frame whose tick had it
        let fresh = race.ticks_last_frame() > 0;
        let sample = stats::FrameSample {
            running: true,
            speed: car.vel.length(),
            kmh: me.telemetry.kmh,
            drifting: self.sess.drifting(player),
            airborne: !car.wheel_contact.iter().any(|&c| c),
            car_impact: car.bump.filter(|b| b.id == 0 && fresh).map_or(0.0, |b| b.impact),
            world_impact: if fresh { car.impact } else { 0.0 },
            ahead: &ahead,
            behind: &behind,
        };
        self.run.frame(dt, &sample);
    }

    /// The ghost car's pose at the time into the player's lap (`None` without a ghost, once the
    /// ghost's lap is over until the next lap starts, or once the player finished).
    pub fn ghost_pose(&self) -> Option<motor_sim::replay::GhostPose> {
        let p = &self.sess.race.racers.iter().find(|x| x.human)?.progress;
        if p.finished_ms.is_some() {
            return None;
        }
        // #374: at the view time, so the ghost moves with the interpolated cars
        let now = self.sess.race.clock_ms - self.sess.race.view_lag() as f64 * 1000.0;
        // #419: the view lags the sim clock by up to one tick, so on the grid (clock 0) and just after a lap starts
        // the lap time is slightly negative (or before the first saved pose): show the ghost's first pose then,
        // not nothing (it flickered)
        let ghost = self.ghost.as_ref()?;
        ghost.pose_at((now - p.lap_start_ms as f64).max(ghost.start_ms()))
    }

    /// REMASTER demo play (#346): the race was put at another moment. The lap and checkpoint the HUD last saw are
    /// the new moment's, and the messages of the old one (lap time, split, gaps) go.
    fn resync(&mut self) {
        let p = &self.sess.race.racers[self.sess.focus].progress;
        (self.last_lap, self.last_cp) = (p.lap, p.last_checkpoint);
        (self.lap_flash, self.split, self.sector_flash, self.ta_gaps, self.finished_for) = (None, None, None, [None; 3], None);
    }

    /// Watching a recording (an `.mhr` replay or an `.MDE` demo): no input, no records, no results.
    pub fn watching(&self) -> bool {
        self.playback.is_some() || self.watch.is_some()
    }

    /// The ghost icon of the HUD: the recorded cars are playing.
    pub fn ghost_playing(&self) -> bool {
        self.demo.is_some() && !self.demo_over
    }

    /// The R icon of the HUD: Record race is writing a `.MDE`.
    pub fn recording_demo(&self) -> bool {
        self.demo_rec.is_some()
    }

    /// Every ghost on the minimap: the Time Attack ghost and the recorded cars of Ghost mode.
    pub fn ghost_poses(&self) -> Vec<motor_sim::replay::GhostPose> {
        let mut out: Vec<_> = self.ghost_pose().into_iter().collect();
        if let Some(d) = &self.demo {
            out.extend(d.poses(motor_sim::demo::view_ms(&self.sess.race)).into_iter().map(|(_, p)| p));
        }
        out
    }

    /// The ghost car's meshes at the live race clock (empty when there is none or it finished).
    pub fn ghost_models(&self) -> Vec<(usize, Mat4)> {
        let Some(slot) = self.sess.race.racers.iter().position(|x| x.human) else { return Vec::new() };
        self.ghost_pose().map(|p| self.sess.ghost_models(slot, &p)).unwrap_or_default()
    }

    /// The recorded cars' meshes at the race clock (Ghost mode).
    pub fn demo_models(&self) -> Models {
        self.demo.as_ref().map(|d| d.models(motor_sim::demo::view_ms(&self.sess.race), self.sess.race.car_scale.0)).unwrap_or_default()
    }

    /// The player's grid slot while the casper cheat is on (the car is drawn as a ghost).
    pub fn casper_slot(&self, casper: bool) -> Option<usize> {
        self.sess.race.racers.iter().position(|x| x.human).filter(|_| casper)
    }

    /// CHEAT casper: the player's meshes of a view's `cars` move to the ghost list, drawn with the
    /// ghost pipeline; the body still lights the scene (`hidden`). Without the cheat (or in a view
    /// that leaves the car out) the lists come back as they are. (cars, hidden, ghost)
    pub fn cheat_cars(&self, casper: bool, mut cars: Models, mut hidden: Models) -> (Models, Models, Models) {
        let mut ghost = self.ghost_models();
        // Ghost mode: the recorded cars are remote cars, drawn by the ghost pipeline (CLASSIC: the original's remote
        // look, 0x439533 / 0x43fca4, #253; REMASTER: the hologram), with no halos (0x43939b)
        ghost.extend(self.demo_models());
        if let Some(slot) = self.casper_slot(casper) {
            let own = slot * CAR_MESHES..(slot + 1) * CAR_MESHES;
            let (mine, rest): (Vec<_>, Vec<_>) = cars.into_iter().partition(|(m, _)| own.contains(m));
            cars = rest;
            hidden.extend(mine.iter().copied().filter(|(m, _)| *m == own.start));
            ghost.extend(mine);
        }
        (cars, hidden, ghost)
    }
}

/// Rankings screen filters: table kind (0 best laptime, 1 best racetime, 2 single race, 3 the
/// league view), track, division (league view), laps (an index into RANK_LAPS), difficulty.
#[derive(Clone, Copy)]
struct RankView {
    kind: usize,
    track: usize,
    division: usize,
    laps: usize,
    diff: Difficulty,
}

/// The league view's kind in `RankView::kind`: CLASSIC the original draws it (0x4bdcd0, the Track
/// label becomes "Division") but its init (0x466016/0x466026) always leaves it for kind 1 or 2 and
/// the Type list stops at 2, so it is unreachable there [D]. Ours opens it with right past
/// "Single race" [design choice].
const RANK_LEAGUE: usize = 3;
/// The league view's list in the Track column (strings at 0x4bdcd0), in `RankView::division` order.
const RANK_DIVISIONS: [hiscore::Kind; 3] = [hiscore::Kind::LeagueDiv3, hiscore::Kind::LeagueDiv23, hiscore::Kind::LeagueTotal];

/// The laps the rankings offer (" 1 3 5 10 15 25" at file offset 0xbc6e9).
const RANK_LAPS: [u32; 6] = [1, 3, 5, 10, 15, 25];

/// One drawable layer, painted in order.
pub enum Layer {
    /// a UI batch; `true`: a REMASTER sharp batch (quads from the field atlas); then its glow
    /// sources (`Batch::glow`), blurred and drawn under it
    Ui(Canvas, Vec<UiVertex>, bool, Vec<UiVertex>),
    /// REMASTER front end: the cars as a hologram (`Frame::hologram`) over what is drawn before
    /// (the clip boxed: its card list makes it the largest variant)
    Holo { cam: Camera, cars: Vec<(usize, Mat4)>, clip: Option<Box<motorview::render::HoloClip>>, road: f32 },
    /// a 3D view of the race (or a menu preview); `parts` as motorview builds them
    /// (`Session::frame_parts`), so the REMASTER grass gets the car motion and the trail too
    Scene { cam: Camera, parts: Box<FrameParts>, viewport: Option<[f32; 4]>, mirror: bool },
    /// REMASTER menu car preview (#91): the cars alone over what is drawn before, in `viewport` (window px),
    /// their paint recoloured live (`Frame::cars_only`, `Frame::paint`)
    Preview { cam: Camera, cars: Vec<(usize, Mat4)>, viewport: [f32; 4], paint: LivePaint },
}

impl Layer {
    /// A layer the renderer draws as a 3D scene: they share the uniform buffers.
    fn is_3d(&self) -> bool {
        !matches!(self, Layer::Ui(..))
    }
}

/// CLASSIC 0x46d02c (Car) / 0x46c32e (Track): the picture of the screen's selection, with its
/// brightness. A new selection fades the shown picture out, swaps it, and fades the new one in;
/// the screen opens at 0, so it fades in.
#[derive(Clone, Copy, Default)]
struct PicFade {
    shown: usize,
    b: f32,
}

impl PicFade {
    fn step(&mut self, selected: usize, dt: f32) {
        if self.b <= 0.0 {
            self.shown = selected;
        }
        self.b = if self.shown == selected { (self.b + dt * PIC_FADE_RATE).min(1.0) } else { (self.b - dt * PIC_FADE_RATE).max(0.0) };
    }

    /// The picture's tint: the brightness as grey.
    fn tint(&self) -> [f32; 4] {
        [self.b, self.b, self.b, 1.0]
    }

    /// The tint of a locked entry's picture: the brightness times `dim` (0.5 tracks, 0.3 cars).
    fn tint_dim(&self, dim: f32) -> [f32; 4] {
        let v = self.b * dim;
        [v, v, v, 1.0]
    }
}

#[derive(PartialEq, Eq, Clone, Copy)]
enum Loaded {
    None,
    Garage,
    /// REMASTER modern Main: NeoCity with the one car of `Game::menu_drive`
    MenuDrive,
    Race,
}

/// #335 what the loading worker needs to build a race: owned, so it can leave the main thread.
struct LoadPlan {
    root: PathBuf,
    /// the .trk number
    track: u32,
    /// the session's field (the player last, or the recorder's car for a demo)
    ids: Vec<u32>,
    player: Option<usize>,
    row: AiRow,
    laps: u32,
    catchup: bool,
    drivers: Option<Vec<motor_sim::Driver>>,
    /// paint on top of the AI drivers': the player's own colours and finishes, the recorder's car
    paint: Vec<(usize, scene::CarPaint)>,
    /// the AI drivers' records paint their cars (0x47b9c4)
    driver_paint: bool,
    demo: Option<DemoPlan>,
    /// the audio's car and engine setting (`race_sounds`)
    car_id: u32,
    engine_sound: bool,
    /// the name cheats the scene is built with: Hell's / underwater's one brightness (#316, #317), Tron's polygon codes (#320)
    cheat: scene::SceneCheat,
}

/// The recorded cars beside the racers (Ghost mode, demo play).
struct DemoPlan {
    mde: Mde,
    first_mesh: usize,
    skip: Option<usize>,
    /// car names -> ids, for the recorded cars; `fallback` for an unknown name
    cars: Vec<(String, u32)>,
    fallback: u32,
}

/// What the worker hands back.
struct RaceLoad {
    sess: Session,
    sc: scene::Scene,
    demo: Option<DemoCars>,
    /// REMASTER local light's source (a BVH of the world), built here so `prepare` need not
    bake: Option<motorview::probes::BakeSource>,
    /// the CPU half of the scene's GPU upload (mip chains, edges)
    upload: Option<SceneUpload>,
    sounds: Option<crate::audio::RaceSounds>,
}

/// What the main thread keeps while the worker runs, to finish the race with.
enum Finish {
    Race(RaceFinish),
    Watch(WatchFinish),
}

struct RaceFinish {
    track_idx: usize,
    track_n: u32,
    track_name: String,
    car: u32,
    ids: Vec<u32>,
    player: Option<usize>,
    human_slot: Option<usize>,
    is_league: bool,
    row: AiRow,
    catchup: bool,
    rules: Rules,
}

struct WatchFinish {
    track_idx: usize,
    track_name: String,
    block: usize,
    name: String,
    laps: u32,
}

/// A race loading on a worker thread (`Game::poll_load`).
struct LoadJob {
    rx: mpsc::Receiver<Result<RaceLoad, String>>,
    progress: std::sync::Arc<Progress>,
    finish: Finish,
    started: std::time::Instant,
    /// frames the screen kept updating during the load (the log line)
    frames: u32,
    /// the worker's result, kept while `prepare` uploads its scene
    done: Option<RaceLoad>,
    worker_ms: f32,
    upload: Option<SceneLoad>,
    /// the scene is on the GPU: the next `poll_load` starts the race
    uploaded: bool,
    /// the upload's frames and its longest step (ms), for the log line
    up_frames: u32,
    up_longest: f32,
    /// REMASTER loading screen (#90): when READY was first shown (for `READY_BEAT`) and the stage timings then
    ready_at: Option<(std::time::Instant, String)>,
}

/// REMASTER loading screen (#90): how long READY and the full bar stay up before the race (s) [design choice]
const READY_BEAT: f32 = 0.5;

/// The GPU upload's time per frame behind the loading screen (#335).
const UPLOAD_BUDGET: std::time::Duration = std::time::Duration::from_millis(6);

impl LoadPlan {
    /// The worker's part (#335): the session, the recorded cars, the scene, the light probe source.
    fn run(self, p: &Progress) -> Result<RaceLoad, String> {
        p.set(Stage::TrackData, 0.0)?;
        let mut sess = Session::with_field(&self.root, self.track, &self.ids, self.player, self.row, Some(self.laps), self.catchup, self.drivers.as_deref())?;
        // The painter 0x47b9c4 runs for every driver at race load (0x45f1e7..0x45f36e): Colour1..3 of the
        // car file become the driver record's CarColour1..3. Only the local player is skipped, when
        // CustomCarColours is off (0x45f1f6); with it on, your CarColour1..3 are your record's.
        let mut paint: Vec<(usize, scene::CarPaint)> = if self.driver_paint {
            sess.race.racers.iter().enumerate().filter(|(_, r)| !r.human).filter_map(|(i, r)| r.driver.car_colours.map(|c| (i, scene::CarPaint::gloss(c)))).collect()
        } else {
            Vec::new()
        };
        paint.extend(self.paint);
        // Ghost mode / demo play: the recorded cars' meshes follow the racers' in the scene, in their own colours
        let mut scene_ids = self.ids.clone();
        let mut demo = None;
        if let Some(dp) = self.demo {
            let (names, fallback) = (dp.cars, dp.fallback);
            let (d, gids) = DemoCars::new(&self.root, dp.mde, dp.first_mesh, dp.skip, |n| names.iter().find(|c| c.0.eq_ignore_ascii_case(n)).map_or(fallback, |c| c.1))?;
            for c in &d.cars {
                let rgb = |v: u32| [(v >> 16) as u8, (v >> 8) as u8, v as u8];
                if c.info.colours[..3].iter().any(|&v| v != 0) {
                    paint.push((c.mesh, scene::CarPaint::gloss([rgb(c.info.colours[0]), rgb(c.info.colours[1]), rgb(c.info.colours[2])])));
                }
            }
            scene_ids.extend(gids);
            demo = Some(d);
        }
        p.set(Stage::TrackData, 1.0)?;
        let sc = scene::load_progress(&self.root, self.track, &scene_ids, &paint, self.cheat, p)?;
        if let Some(d) = &mut demo {
            d.set_radii(&sc);
        }
        sess.set_wheels(&sc);
        p.set(Stage::Remaster, 0.9)?;
        let bake = motorview::probes::BakeSource::new(&sc, &sess.race.world);
        let sounds = crate::audio::race_sounds(&self.root, self.track, &sess.race.world.materials, self.car_id, self.engine_sound, p)?;
        p.set(Stage::Grid, 0.0)?;
        let upload = SceneUpload::new(&sc);
        Ok(RaceLoad { sess, sc, demo, bake: Some(bake), upload: Some(upload), sounds: Some(sounds) })
    }
}

/// REMASTER modern Main screen (09-remaster 9.24): the selected car driven round NeoCity by the
/// game's AI, shown as a hologram over the road's outline. The camera is planar: it follows the
/// car's position and (damped) heading, never its pitch or roll.
struct MenuDrive {
    sess: Session,
    /// `settings.car` this session was built for
    car: usize,
    /// the asphalt outline (`scene::road_lines`), re-uploaded with the scene
    road: Vec<Vertex>,
    /// the camera's planar frame: the car's position and a damped heading (view space)
    pos: Vec3,
    dir: Vec3,
}

/// sim seconds run before the first frame, the camera heading's damping rate (1/s)
const MENU_DRIVE_WARMUP: f32 = 12.0;
/// the car's speed limit (km/h, the AI's target speed cap)
const MENU_DRIVE_SPEED: f32 = 100.0;
const MENU_DRIVE_TURN_RATE: f32 = 2.5;

#[path = "menu_modern.rs"]
pub(crate) mod menu_modern;

/// CLASSIC: where one dial screen's sprites sit in `Data/sprpos.dta` (the init functions 0x463428
/// main, 0x46a1dc single, 0x46a9a0 league, 0x466784 options list the sprites in slot order: dots,
/// the two header boxes, msel, [an empty slot], the items, then the info brackets). The needle's
/// centre is slot 0x1e and its five angles (8 x + y, 4096 to the turn) are slots 0x18..0x1c [O].
struct DialLayout {
    set: usize,
    title: usize,
    screen_box: usize,
    msel: usize,
    /// slot of the first item; the items follow in order
    first: usize,
    /// slot of the `info1` sprite (the bracket and connector beside the info text); `info2` follows
    info: Option<usize>,
}
const MAIN_DIAL: DialLayout = DialLayout { set: 0, title: 5, screen_box: 2, msel: 3, first: 6, info: None };
const SINGLE_DIAL: DialLayout = DialLayout { set: SINGLE_SET, title: 4, screen_box: 2, msel: 3, first: 5, info: Some(0xa) };
const LEAGUE_DIAL: DialLayout = DialLayout { set: LEAGUE_SET, title: 5, screen_box: 2, msel: 3, first: 6, info: Some(0xb) };
const OPTIONS_DIAL: DialLayout = DialLayout { set: OPTIONS_SET, title: 2, screen_box: 3, msel: 4, first: 5, info: None };
/// Main menu dial in the original's item order.
const MAIN_ITEMS: [&str; 5] = ["tone", "tmul", "tran", "topt", "tcre"];
/// "Single" dial: quick race, single race, league race, time attack, ghost mode.
/// KNOCKOUT (ours) sits between time attack and ghost mode; the original art has no sprite for it,
/// so `dial` draws its label as text (design choice).
const SINGLE_ITEMS: [&str; 6] = ["tquic", "tsing", "tleag", "tatta", KNOCKOUT_LABEL, "tghos"];
const KNOCKOUT_LABEL: &str = "tknoc";
/// League dial (title leagu): continue, view standings, load, new, delete.
const LEAGUE_ITEMS: [&str; 5] = ["tlcon", "tlvst", "tlloa", "tlnew", "tldel"];
/// Options dial: personal, graphic, control, sound options; difficulty is drawn by `options_menu`.
const OPTION_ITEMS: [&str; 4] = ["tpers", "tgro", "tcon", "tso"];
/// CLASSIC Controls screen (0x468f10): the tree's rows, as tasks (the original's ten; Respawn and
/// Change camera are ours and only in the modern menu).
const CLASSIC_TASKS: [usize; 10] = [0, 1, 2, 3, 4, 5, 6, 7, 8, task::GAME_MENU];
/// the task labels' sprites, in the same order
const CLASSIC_LABELS: [&str; 10] = ["tlef", "trig", "tacc", "tbre", "tgup", "tgdo", "thorn", "thbr", "tsht", "tmenu"];
pub(super) const CTL_TUNE_LABELS: [&str; 4] = ["Steering dead zone", "Trigger dead zone", "Steering sensitivity", "Vibration"];
/// Rows of the pause menu's Controls tab: the car tasks, the keyboard layout, the pad sliders.
pub(super) const PAUSE_CTL_ROWS: usize = CAR_TASKS + 1 + 4;
/// `Settings::pad_tune`: defaults and ranges in percent, the step of one ←/→
const PAD_TUNE_DEFAULT: [u32; 4] = [5, 15, 100, 70];
pub(super) const PAD_TUNE_RANGE: [(u32, u32); 4] = [(0, 50), (0, 50), (50, 200), (0, 100)];
const PAD_TUNE_STEP: u32 = 5;
const DEFAULT_PAD: [Option<PadIn>; CAR_TASKS] = {
    let mut b = [None; CAR_TASKS];
    let mut i = 0;
    while i < CAR_TASKS {
        b[i] = DEFAULT_PAD_BIND[i];
        i += 1;
    }
    b
};

/// A binding waiting for its control: the car task, and whether it is the pad's (Controller page).
/// `any` (REMASTER pause menu's Controls tab): the first key, mouse button or pad button pressed is
/// bound, to the keyboard or to the pad.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) struct Capture {
    pub task: usize,
    pub pad: bool,
    pub any: bool,
}
/// `Data/sprpos.dta` sets of the screens drawn in FONT1 (the screen descriptors' +0x18 at 0x4bdd98..).
const SINGLE_SET: usize = 10;
const TRACK_SET: usize = 2;
const CAR_SET: usize = 3;
const CREDITS_SET: usize = 18;
const GRAPHICS_SET: usize = 5;
const CONTROLS_SET: usize = 8;
const PERSONAL_SET: usize = 17;
const RANKINGS_SET: usize = 11;
/// The league screen's info box under the dial.
const LEAGUE_SET: usize = 12;
const STANDINGS_SET: usize = 16;
/// The league load list's set (descriptor 11, init 0x46b9a4; New is set 13, Delete set 15).
const LEAGUE_LOAD_SET: usize = 14;
const REPLAYS_SET: usize = 19;
const MULTI_SET: usize = 9;
/// The options dial's set (init 0x466784, draw 0x4668a8): slots 9 / 10 / 11 = `tdif`, `difin`, `emh`.
const OPTIONS_SET: usize = 7;
/// The original's menu frames per second [I: 30; the menu video shows no more than that in its glitches and rolls].
/// Everything the original counts in menu frames (glitches, turn overs, value rolls) runs at this rate.
const MENU_FPS: f32 = 30.0;
/// `emh` frames per second of the difficulty word's roll, one per original menu frame.
const DIFF_ROLL_RATE: f32 = MENU_FPS;
/// CLASSIC 0x46c446 / 0x46ce3a / 0x46ce62: the rolling `onoff` / `tman` sprites, one frame (0 .. 7) a menu frame
/// towards 0 (manual / on) or 7 (auto / off): indices into `Game::rolls`.
const ROLL_MANUAL: usize = 0;
const ROLL_RECORD: usize = 1;
const ROLL_CATCHUP: usize = 2;

/// The camera options of the graphic options' Camera tab [design choice]: kept in the settings,
/// applied when a race starts and live from the pause menu.
#[derive(Clone, Copy, PartialEq)]
pub struct CamOpts {
    /// the camera a race starts in: an index into CAM_STARTS
    pub start: usize,
    pub mirror: bool,
    /// the right stick orbits the chase camera
    pub free_look: bool,
    /// the chase eye comes closer when a wall is in the way
    pub pull_in: bool,
    /// chase camera easing: an index into CAM_FOLLOWS
    pub follow: usize,
}

impl Default for CamOpts {
    fn default() -> Self {
        CamOpts { start: 0, mirror: false, free_look: true, pull_in: true, follow: 1 }
    }
}

const CAM_STARTS: [&str; 5] = ["Chase", "Far chase", "In-car", "Bumper", "TV"];
const CAM_FOLLOWS: [&str; 3] = ["Loose", "Normal", "Tight"];

impl CamOpts {
    /// the camera mode of `start` (the TV camera needs the track's camera stations)
    fn mode(&self, has_tv: bool) -> CamMode {
        match self.start {
            1 => CamMode::FarChase,
            2 => CamMode::InCar,
            3 => CamMode::Bumper,
            4 if has_tv => CamMode::Tv,
            _ => CamMode::Chase,
        }
    }

    /// the `start` index that picks `mode` (the inverse of `mode`; the goal camera is no choice)
    fn start_of(mode: CamMode) -> Option<usize> {
        match mode {
            CamMode::Chase => Some(0),
            CamMode::FarChase => Some(1),
            CamMode::InCar => Some(2),
            CamMode::Bumper => Some(3),
            CamMode::Tv => Some(4),
            CamMode::Goal => None,
        }
    }
}

/// Frame-rate caps offered in graphic options (0 = unlimited).
pub const FRAME_LIMITS: [u32; 6] = [30, 60, 90, 120, 240, 0];
/// Setting rows of the graphic options screen.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum GfxRow {
    RenderDevice,
    Monitor,
    ScreenMode,
    Ratio,
    MenuPicture,
    Resolution,
    VSync,
    FrameLimit,
    Brightness,
    CdMusic,
    DetailMode,
    Shadows,
    Halos,
    Fog,
    CarShading,
    TrackDetailRow,
    CarDetailRow,
    SkidMarks,
    NamePlates,
    SkyImage,
    MotionBlur,
    Smoke,
    Sparks,
    LensFlares,
    Mode,
    ModeSwitch,
    DynamicLights,
    LocalLight,
    TexturePack,
    RenderAa,
    RenderAaStrength,
    PostAa,
    PostAaStrength,
    Anisotropic,
    PixelLight,
    PixelFog,
    FogStrength,
    ClearCoat,
    Signs,
    WorldFill,
    TreeModels,
    Water,
    WaterReflect,
    Grass,
    GrassRange,
    ViewDistance,
    ColourGrade,
    Vignette,
    Grain,
    Hdr,
    Tonemap,
    Exposure,
    Bloom,
    Ssao,
    HudStyle,
    MenuStyle,
    CarModels,
    CarLodDistance,
    LightShadows,
    ShadowQuality,
    InfoDetail,
    CheckpointInfo,
    CheckpointDelay,
    RoadMap,
    CamStart,
    CamMirror,
    CamFreeLook,
    CamPullIn,
    CamFollow,
    // the race HUD's rows (the pause menu's HUD tab, #300 phase 3)
    HudScale,
    HudMargin,
    HudCluster,
    HudTotal,
    HudLapTime,
    HudBest,
    HudLap,
    HudPos,
    /// the map's Off / Nearby / All cars: RoadMap and the map toggle in one (`map_row`)
    HudMap,
    MapView,
    MapTurns,
    /// the place numbers over the cars on the map
    MapNumbers,
    Proximity,
    /// the glass frame of the See-through name plates
    PlateFrame,
    /// how long BEST LAP / LAST LAP stays at the top (Modern HUD)
    LapDelay,
    /// the live gap to the car ahead and behind in the bottom-left block (Modern HUD, #302)
    LiveGaps,
    /// the delta against the best lap at each checkpoint and the line (Modern HUD, #409)
    SectorDelta,
}

/// The map row of the modern HUD page, as the HUD draws it: Off when the map toggle is off, Nearby when RoadMap
/// is Selective (it filters the markers, hud.rs), else All cars (also a file with the toggle on and RoadMap None:
/// the HUD shows the whole field then) [design choice, #300].
pub(super) fn map_row(s: &Settings) -> usize {
    if !s.hud.map { 0 } else if s.road_map == 1 { 1 } else { 2 }
}

/// Write the map row: RoadMap and the map toggle together.
pub(super) fn set_map_row(s: &mut Settings, i: usize) {
    s.road_map = i;
    s.hud.apply_road_map(i);
}

/// Touching a corner piece forks the shown set into Custom (bug #181): the pieces InfoDetail shows
/// become the saved toggles, so the preset's look stays and only the touched piece changes. The
/// toggles of an earlier Custom set are kept until then.
pub(super) fn fork_hud(s: &mut Settings) {
    if s.info_detail != INFO_CUSTOM {
        s.hud = s.hud.shown(s.info_detail);
        s.info_detail = INFO_CUSTOM;
    }
}

/// How long the checkpoint splits can stay on screen, seconds: the original's menu offers 0..10 (04-track-car-data,
/// the Classic row keeps that); the Remaster HUD tab's "Splits on screen" slider goes to 25 [design choice, #323].
pub(super) const DELAY_CLASSIC_MAX: u32 = 10;
pub(super) const DELAY_MAX: u32 = 25;

/// Map row, checkpoint info and name plates levels on the modern HUD page.
pub(super) const MAP_LEVELS: [&str; 3] = ["Off", "Nearby", "All cars"];

impl GfxRow {
    /// the DetailMode presets set this row: touching it makes the mode Custom [I]
    fn preset(self) -> bool {
        use GfxRow::*;
        matches!(self, Shadows | Halos | CarShading | TrackDetailRow | CarDetailRow | SkidMarks | NamePlates | SkyImage | MotionBlur | Smoke | Sparks | LensFlares)
    }
    /// the row has an effect with these settings; an inactive row is dimmed and does not change
    pub(super) fn active(self, v: &View) -> bool {
        match self {
            GfxRow::GrassRange => v.remaster.grass != GrassLevel::Off,
            GfxRow::CarLodDistance => v.remaster.generated_cars,
            GfxRow::FogStrength => v.fog && v.remaster.fog_style != FogStyle::Classic,
            GfxRow::WaterReflect => v.remaster.water,
            _ => true,
        }
    }
    /// the row is left out of the list: an AA strength with its method Off (a slider with nothing
    /// to strengthen) [design choice]
    pub(super) fn hidden(self, v: &View) -> bool {
        match self {
            GfxRow::RenderAaStrength => v.remaster.render_aa.kind == RenderAaKind::Off,
            GfxRow::PostAaStrength => v.remaster.post_aa.kind == PostAaKind::Off,
            _ => false,
        }
    }
    /// `active` for the modern pages, which also dim the HUD rows that do nothing with these settings
    /// (Scale, Margin and Proximity arrows only work in the Modern HUD; the map's rows need the map)
    pub(super) fn active_in(self, s: &Settings) -> bool {
        match self {
            GfxRow::HudScale | GfxRow::HudMargin | GfxRow::Proximity | GfxRow::LapDelay | GfxRow::LiveGaps | GfxRow::SectorDelta => s.view.modern_hud(),
            GfxRow::MapView | GfxRow::MapNumbers => map_row(s) > 0,
            GfxRow::PlateFrame => s.view.modern_hud() && s.name_plates == 2,
            GfxRow::MapTurns => map_row(s) > 0 && s.hud.map_mode == MapMode::Close,
            GfxRow::CheckpointDelay => s.checkpoint_info > 0,
            _ => self.active(&s.view),
        }
    }
    /// The HUD preview's pieces (`menu_modern::HUD_PIECES` order) that this row changes
    pub(super) fn hud_lit(self) -> &'static [usize] {
        use GfxRow::*;
        match self {
            HudCluster => &[0],
            HudTotal => &[1],
            HudLapTime => &[2],
            HudBest => &[3],
            HudLap => &[4],
            HudPos => &[5],
            InfoDetail => &[0, 1, 2, 3, 4, 5],
            HudMap | MapView | MapTurns | MapNumbers => &[6],
            Proximity => &[7],
            NamePlates | PlateFrame => &[8],
            LapDelay | SectorDelta => &[10],
            CheckpointInfo | CheckpointDelay | LiveGaps => &[9],
            _ => &[0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10],
        }
    }
    /// a Remaster-only option: hidden in Classic mode (a Remaster style of an original option, like
    /// the shadows, is a value of that option's row instead)
    pub(super) fn remaster(self) -> bool {
        use GfxRow::*;
        matches!(self, CamPullIn | CamFollow | DynamicLights | LocalLight | TexturePack | RenderAa | RenderAaStrength | PostAa | PostAaStrength | Anisotropic | PixelLight | PixelFog | ClearCoat | Signs | WorldFill | TreeModels | Water | WaterReflect | Grass | GrassRange | FogStrength | ColourGrade | Vignette | Grain | Hdr | Tonemap | Exposure | Bloom | Ssao | HudStyle | MenuStyle | CarModels | CarLodDistance | LightShadows | ShadowQuality | HudScale | HudMargin | HudCluster | HudTotal | HudLapTime | HudBest | HudLap | HudPos | HudMap | MapView | MapTurns | MapNumbers | Proximity | PlateFrame | LapDelay | LiveGaps | SectorDelta)
    }
    /// Restore this row to the default: the display rows on the draft `d`, the others live.
    /// Mode and MenuStyle are left: resetting them would drop the user out of the modern menu.
    fn reset(self, s: &mut Settings, d: &mut Display, def: &Settings) {
        use GfxRow::*;
        let (v, w) = (&mut s.view, &def.view);
        let (r, q) = (&mut v.remaster, &w.remaster);
        match self {
            RenderDevice => {}
            Monitor => d.monitor = def.display.monitor,
            ScreenMode => d.fullscreen = def.display.fullscreen,
            Ratio | Resolution => (d.wide, d.size) = (def.display.wide, def.display.size),
            MenuPicture => s.menu_stretch = def.menu_stretch,
            VSync => d.vsync = def.display.vsync,
            FrameLimit => s.fps_limit = def.fps_limit,
            Brightness => v.brightness = w.brightness,
            CdMusic => s.music = def.music,
            DetailMode => {
                s.detail_mode = def.detail_mode;
                s.apply_detail();
            }
            Shadows => (v.shadows, r.car_shadows) = (w.shadows, q.car_shadows),
            Halos => (v.halos, r.smooth_halos) = (w.halos, q.smooth_halos),
            Fog => (v.fog, r.fog_style) = (w.fog, q.fog_style),
            CarShading => (v.car_shading, r.enhanced_paint) = (w.car_shading, q.enhanced_paint),
            TrackDetailRow => s.track_detail = def.track_detail,
            CarDetailRow => v.car_detail = w.car_detail,
            SkidMarks => (s.skid_marks, r.skids_detailed) = (def.skid_marks, q.skids_detailed),
            NamePlates => s.name_plates = def.name_plates,
            SkyImage => (v.sky_image, r.sky_hdri) = (w.sky_image, q.sky_hdri),
            MotionBlur => (v.motion_blur, r.velocity_blur) = (w.motion_blur, q.velocity_blur),
            Smoke => v.smoke = w.smoke,
            Sparks => v.sparks = w.sparks,
            LensFlares => (v.lens_flares, r.smooth_flares) = (w.lens_flares, q.smooth_flares),
            InfoDetail => s.info_detail = def.info_detail,
            CheckpointInfo => s.checkpoint_info = def.checkpoint_info,
            CheckpointDelay => s.checkpoint_delay = def.checkpoint_delay,
            LapDelay => s.hud.lap_delay = def.hud.lap_delay,
            RoadMap => {
                s.road_map = def.road_map;
                s.hud.apply_road_map(s.road_map);
            }
            CamStart => s.cam.start = def.cam.start,
            CamMirror => s.cam.mirror = def.cam.mirror,
            CamFreeLook => s.cam.free_look = def.cam.free_look,
            CamPullIn => s.cam.pull_in = def.cam.pull_in,
            CamFollow => s.cam.follow = def.cam.follow,
            HudScale => s.hud.scale = def.hud.scale,
            HudMargin => s.hud.margin = def.hud.margin,
            HudCluster => s.hud.cluster = def.hud.cluster,
            HudTotal => s.hud.clocks[0] = def.hud.clocks[0],
            HudLapTime => s.hud.clocks[1] = def.hud.clocks[1],
            HudBest => s.hud.clocks[2] = def.hud.clocks[2],
            HudLap => s.hud.lap = def.hud.lap,
            HudPos => s.hud.pos = def.hud.pos,
            HudMap => set_map_row(s, map_row(def)),
            MapView => s.hud.map_mode = def.hud.map_mode,
            MapTurns => s.hud.map_rotate = def.hud.map_rotate,
            MapNumbers => s.hud.map_numbers = def.hud.map_numbers,
            PlateFrame => s.hud.plate_frame = def.hud.plate_frame,
            LiveGaps => s.hud.live_gaps = def.hud.live_gaps,
            SectorDelta => s.hud.sector_delay = def.hud.sector_delay,
            Proximity => s.hud.proximity = def.hud.proximity,
            Mode | MenuStyle => {}
            ModeSwitch => s.mode_dissolve = def.mode_dissolve,
            DynamicLights => r.dynamic_lights = q.dynamic_lights,
            LocalLight => r.local_light = q.local_light,
            TexturePack => r.texture_pack = q.texture_pack,
            RenderAa => r.render_aa.kind = q.render_aa.kind,
            RenderAaStrength => r.render_aa.strength = q.render_aa.strength,
            PostAa => r.post_aa.kind = q.post_aa.kind,
            PostAaStrength => r.post_aa.strength = q.post_aa.strength,
            Anisotropic => r.anisotropy = q.anisotropy,
            PixelLight => r.pixel_light = q.pixel_light,
            PixelFog => r.pixel_fog = q.pixel_fog,
            ClearCoat => r.clear_coat = q.clear_coat,
            Signs => r.retro_signs = q.retro_signs,
            WorldFill => r.world_fill = q.world_fill,
            TreeModels => r.tree_models = q.tree_models,
            Water => r.water = q.water,
            WaterReflect => r.water_ssr = q.water_ssr,
            Grass => r.grass = q.grass,
            GrassRange => r.grass_range = q.grass_range,
            FogStrength => r.fog_strength = q.fog_strength,
            ViewDistance => v.distance = w.distance,
            ColourGrade => r.grade = q.grade,
            Vignette => r.vignette = q.vignette,
            Grain => r.grain = q.grain,
            Hdr => r.hdr = q.hdr,
            Tonemap => r.tonemap = q.tonemap,
            Exposure => r.exposure = q.exposure,
            Bloom => r.bloom = q.bloom,
            Ssao => r.ssao = q.ssao,
            HudStyle => r.modern_hud = q.modern_hud,
            CarModels => r.generated_cars = q.generated_cars,
            CarLodDistance => r.car_lod_distance = q.car_lod_distance,
            LightShadows => r.light_shadows = q.light_shadows,
            ShadowQuality => r.shadow_quality = q.shadow_quality,
        }
    }
    /// the tooltip under the list while the row is selected [design choice]
    pub(super) fn help(self) -> &'static str {
        use GfxRow::*;
        match self {
            RenderDevice => "The graphics card the game draws with. Read only.",
            Monitor => "The screen the game opens on in fullscreen. Takes effect on Apply or OK.",
            ScreenMode => "Play in a window or fullscreen. Takes effect on Apply or OK.",
            Ratio => "4:3 like the original, or 16:9 widescreen. Takes effect on Apply or OK.",
            MenuPicture => "The classic menus: Stretch fills the window like the original on its monitor, Keep shape keeps the 16:10 picture with black bars.",
            Resolution => "The size of the window or fullscreen mode. Takes effect on Apply or OK.",
            VSync => "Waits for the monitor's refresh: no tearing, but more input lag. Takes effect on Apply or OK.",
            FrameLimit => "Caps the frame rate to save power and heat. The race simulation does not depend on it.",
            Brightness => "Overall brightness of the 3D view.",
            CdMusic => "Plays the CD soundtrack in menus and races.",
            DetailMode => "Presets for the options below. Changing any of them makes the mode Custom.",
            Shadows => "Shadows of the cars on the track. Classic: the original's projected shadow. Real (Remaster): sun shadows from a shadow map, on the world and the cars.",
            Halos => "Glows around lamps, car lights and track lights. Classic: the original halo sprites. Smooth (Remaster): computed glows with a lens starburst; with bloom they glow like other lights.",
            Fog => "Distance fog in the track's colour; hides the far clipping plane. Enhanced (Remaster): fog in the sky's horizon colour with a start and full distance set per track, a height haze in low ground, a glow towards the sun, and the sky fogged to match. Volumetric (Remaster, HDR): the same as a 3D volume that the street lamps light up.",
            CarShading => "Flat (Classic): every polygon one even colour, no sun shading. Gouraud: plain lit paint. Reflection: the sky and track mirrored in the paint. Enhanced (Remaster): cube-mapped reflections of the real surroundings.",
            TrackDetailRow => "Medium leaves out the objects along the track (signs, props, moving scenery) as the original did on slow machines. Only stored for now.",
            CarDetailRow => "How detailed the car bodies are: Low and Medium switch to simpler meshes sooner as the cars get farther away.",
            SkidMarks => "Tyre marks left on the road when the cars slide. Classic: the original flat marks. Detailed (Remaster): soft marks with tread streaks that fade at the ends, a tread imprint on grass and gravel.",
            NamePlates => "Driver names above the other cars: none, flat, or see-through.",
            SkyImage => "The sky picture. Off draws a plain colour.",
            MotionBlur => "Blurs the view at speed. Classic: the original's frame blend. Velocity (Remaster): per-pixel blur from the camera and car motion.",
            Smoke => "Tyre smoke when the cars slide or spin their wheels.",
            Sparks => "Sparks when cars scrape walls or each other.",
            LensFlares => "Lens flares when looking into the sun. Classic: the original flare textures. Smooth (Remaster): computed flares from lamps, car lights and blinkers, with streaks and ghosts.",
            ModeSwitch => "Instant swaps the graphics mode at once. Dissolve fades the old look out over the new one for 0.3 s when F9 is pressed.",
            Mode => "Classic draws like the 1998 original. Remaster adds modern options to this page and a modern style to some of the original ones.",
            DynamicLights => "Blinkers and flashing cones light up the cars and the road around them.",
            TexturePack => "Your own upscaled textures, normal maps and roughness maps from remaster/textures in the game folder, drawn over the original art where you have them. Off: the original textures. Does nothing when the folder has no files.",
            LocalLight => "The cars and roadside objects take their fill light from the track around them: darker in tunnels and under bridges, tinted by lit walls and grass nearby. Off: the same sky light everywhere. The first race on a track works it out in the background: it fades in after a few seconds.",
            RenderAa => "Anti-aliasing while the scene is drawn: MSAA smooths polygon edges, SSAA draws the whole scene at a higher resolution and shrinks it (cleanest, very costly). Combines with the post-processing method.",
            RenderAaStrength => "How many samples per pixel the render-stage method takes: 2x, 4x, 8x or 16x. MSAA uses the most the graphics card supports; SSAA draws that many times the pixels (16x is four times the width and height, kept within a memory budget). Higher costs more GPU time.",
            PostAa => "Anti-aliasing of the finished image. FXAA is the cheapest and softest, SMAA is sharper, TAA is the most stable in motion but can smear fast detail. Combines with the render-stage method.",
            PostAaStrength => "Quality in four steps, 2x to 16x. FXAA: how much sub-pixel detail is smoothed. SMAA: preset Low to Ultra. TAA: how much of the previous frame is kept.",
            Anisotropic => "Keeps textures sharp at shallow angles, like the road ahead.",
            PixelLight => "Car lighting computed per pixel instead of per vertex: smoother highlights.",
            FogStrength => "How thick the Enhanced and Volumetric fog is. Higher: the haze in low ground gets denser and the distance fog starts and fills in closer; lower: clearer air. The far edge of the view stays hidden.",
            PixelFog => "Fog computed per pixel instead of per vertex: no banding on large polygons.",
            ClearCoat => "A glossy top layer on the car paint with a sharp sun highlight.",
            Signs => "Road signs reflect the headlights back at night and show a faint sheen of the sky by day.",
            WorldFill => "Generated scenery that fills the void past the edge of the map and the holes inside it, so a long view distance shows no gaps.",
            TreeModels => "3D trees and bushes in place of the original's flat cut-out pictures, so they look solid from every side. Off: the original pictures.",
            Water => "Animated water in place of the original's flat water texture: waves, depth you can see through, foam at the shore. Off: the original texture.",
            WaterReflect => "The water mirrors the banks, buildings and cars around it. Off: it mirrors only the sky. Needs Water. Costs GPU time where water fills the screen.",
            Grass => "Turf with depth and a light wind sway over the grass verges, fading back to the plain texture in the distance. Short draws half the layers to half the distance. Ultra grows real blades near the car.",
            GrassRange => "How far out the grass reaches before it fades to the flat texture. At Ultra the real blades reach farther too; high values cost a lot of GPU time with Ultra.",
            ViewDistance => "How far the track is drawn. Over 100% shows more than the original did.",
            ColourGrade => "A film-style colour grade over the image.",
            Vignette => "Darkens the corners of the screen.",
            Grain => "Adds film grain.",
            Hdr => "Lights in linear high dynamic range, then tonemaps to the screen. Needed for bloom.",
            Tonemap => "Maps HDR to the screen: Neutral keeps colours, ACES is filmic, AgX softens bright colours.",
            Exposure => "Brightness of the HDR image before tonemapping.",
            Bloom => "Bright lights and highlights glow into their surroundings.",
            Ssao => "Screen-space ambient occlusion: darkens corners and contact points: wheel arches, kerbs, posts and walls.",
            HudStyle => "Classic: the original HUD pictures scaled up. Modern: new fonts at the screen's resolution, set up in the race's HUD Settings.",
            MenuStyle => "Classic: the original menu pictures. Modern: the redesigned front end, screen by screen (Main so far).",
            LightShadows => "Night shadows of cars from their headlights and the street lamps. Headlights: only the cars' own beams; All: the lamps too.",
            ShadowQuality => "Sharpness of the night shadows. Higher uses larger shadow maps (more video memory) and softer filtering at Ultra.",
            CarModels => "Original: the 1998 car meshes. Generated: smooth high-polygon wheels built at the original size.",
            CarLodDistance => "How far away other cars switch from the detailed Generated body to the original model. Higher keeps the detail out to a greater distance; lower runs faster with many cars on screen. Needs Car models: Generated.",
            InfoDetail => "How much race information the HUD shows: laps, positions and times.",
            CheckpointInfo => "The split times shown when passing a checkpoint.",
            CheckpointDelay => "How long the checkpoint splits stay on screen.",
            RoadMap => "The track map in the corner of the screen.",
            CamStart => "The camera a race starts in. During a race the camera keys switch between them; the TV camera needs a track with camera stations. The camera you pick in a race is kept here.",
            CamMirror => "The rear view mirror at the top of the screen. The mirror key switches it during a race.",
            CamFreeLook => "The right stick of a gamepad orbits the chase camera round the car; letting go brings it back behind.",
            CamPullIn => "The chase camera moves closer to the car when a wall is in the way, instead of going through it.",
            CamFollow => "How closely the chase camera follows the car: Loose lags more in corners, Tight stays behind it.",
            HudScale => "The size of the HUD. It shrinks to fit a small window. Modern HUD only.",
            HudMargin => "The gap between the HUD and the window's edges. Modern HUD only.",
            HudCluster => "The speed, the gear and the rev meter. Modern HUD: a big speed over a swoosh that turns red at the redline.",
            HudTotal => "The race clock.",
            HudLapTime => "The clock of the lap in progress.",
            HudBest => "The best lap time so far.",
            HudLap => "The lap counter.",
            HudPos => "Your place in the race.",
            HudMap => "The track map in the corner. Nearby marks you and the cars just ahead and behind; All cars marks the whole field.",
            MapView => "Circuit: the whole track. Close: the stretch around your car.",
            MapTurns => "The close map and its ring turn with the car. N marks north (best guess).",
            MapNumbers => "The place number over each car on the map.",
            Proximity => "Arrows round your car point at rivals close by. Modern HUD only.",
            PlateFrame => "The glass frame round each See-through name plate. Off leaves only the colour bar and the name. Modern HUD only.",
            LapDelay => "How long BEST LAP and LAST LAP stay at the top of the screen after each lap. Modern HUD only.",
            SectorDelta => "How long the time difference stays on screen at each checkpoint and on BEST / LAP / LAST LAP, against your best earlier lap of this race: green when you are ahead of it, red when behind. From the second lap on. Off hides it. Modern HUD only.",
            LiveGaps => "The gap to the car ahead and the car behind, bottom left, all race, as the Checkpoint splits row says: Nearby = the cars either side of you, All cars = the whole field, Off = none. It replaces the checkpoint list in races (a chime still marks each checkpoint); Time Attack keeps its list. Modern HUD only.",
        }
    }
    /// `help` as the modern pages word it (the HUD tab's rows are named after the mockup)
    pub(super) fn help_modern(self) -> &'static str {
        use GfxRow::*;
        match self {
            HudStyle => "Classic: the original HUD pictures scaled up. Modern: new fonts at the screen's resolution; Scale, Margin and Proximity arrows work with it.",
            InfoDetail => "How much race information shows. None, Selective and All set the pieces below; changing a piece makes it Custom.",
            NamePlates => "Driver names above the other cars. Flat is plain text; See-through draws small glass plates (Modern HUD).",
            CheckpointInfo => "The split list at each checkpoint: Nearby shows the three places round yours, All cars the whole field.",
            CheckpointDelay => "How long the checkpoint splits stay on screen.",
            _ => self.help(),
        }
    }
}

/// Pages of the graphic options screen [design choice]: "Screen" is the original's list (CLASSIC
/// 0x4676d4: Render device .. Motion blur, its order; the rows after Graphic detail only while
/// that is Custom, `gfx_rows`; our HUD rows are in it, the original kept them there too), then the
/// window, our detail options and the Remaster additions (Remaster-only rows are hidden in
/// Classic) and the race camera. The screen's rows are the page tabs, the page's rows, then the
/// buttons (not on Screen: the original has none). The in-race GFX Options show the same pages.
pub(super) const GFX_PAGES: [(&str, &[GfxRow]); 4] = {
    use GfxRow::*;
    [
        (
            "Screen",
            &[
                RenderDevice, Resolution, Brightness, InfoDetail, RoadMap, CheckpointInfo, CheckpointDelay, DetailMode, LensFlares, Sparks, Smoke, Halos, Shadows, SkidMarks, NamePlates, SkyImage, TrackDetailRow, CarDetailRow, CarShading, ViewDistance,
                MotionBlur,
            ],
        ),
        ("Display", &[Monitor, ScreenMode, Ratio, MenuPicture, VSync, FrameLimit, RenderAa, RenderAaStrength, PostAa, PostAaStrength, Anisotropic, CdMusic]),
        // Shadows, Halos, Lens flares, Motion blur and Carshading have a Remaster style as an extra value
        (
            "Detail",
            &[
                Mode, ModeSwitch, Fog, CarModels, CarLodDistance, ClearCoat, PixelLight, DynamicLights, LocalLight, TexturePack, LightShadows, ShadowQuality, PixelFog, FogStrength, Signs, WorldFill, TreeModels, Water, WaterReflect, Grass, GrassRange, Ssao, Hdr, Tonemap, Exposure, Bloom,
                ColourGrade, Vignette, Grain, HudStyle, MenuStyle,
            ],
        ),
        // the race camera: where it starts and how it behaves (the keys are in the controls)
        ("Camera", &[CamStart, CamMirror, CamFreeLook, CamPullIn, CamFollow]),
    ]
};
/// The Screen page of GFX_PAGES (the original's list: no buttons) and the Detail page.
const SCREEN_PAGE: usize = 0;
const DETAIL_PAGE: usize = 2;
/// Index in the Screen page's rows of Graphic detail: the rows after it need Custom.
const SCREEN_CUSTOM_AFTER: usize = 8;
/// The rows of a page for these settings: Classic hides the Remaster-only rows, a strength row
/// goes with its Off method (`GfxRow::hidden`), and the Screen page stops after Graphic detail
/// unless that is Custom (CLASSIC 0x467f89: the other rows are neither drawn nor reachable).
pub(super) fn gfx_rows(page: usize, s: &Settings) -> Vec<GfxRow> {
    let v = &s.view;
    let rows = GFX_PAGES[page].1.iter().copied().filter(|r| !(v.mode == RenderMode::Classic && r.remaster()) && !r.hidden(v));
    if page == SCREEN_PAGE && s.detail_mode != CUSTOM_DETAIL {
        rows.take(SCREEN_CUSTOM_AFTER).collect()
    } else {
        rows.collect()
    }
}
/// REMASTER pages of the modern graphic options [design choice, PAUSE LAYOUT v1]: Texture filter
/// sits on Display, CD music is on the pause menu's Sound tab, the Camera rows are their own
/// pause tab (`CAMERA_ROWS`, `Screen::PauseCam`) and the race HUD's rows the HUD tab
/// (`HUD_SECTIONS`). Menu style is a Display row. The Classic list keeps `GFX_PAGES`.
pub(super) const MODERN_GFX_PAGES: [(&str, &[GfxRow]); 2] = {
    use GfxRow::*;
    [
        ("Display", &[Monitor, ScreenMode, Ratio, MenuPicture, MenuStyle, Resolution, VSync, FrameLimit, RenderAa, RenderAaStrength, PostAa, PostAaStrength, Brightness, Anisotropic]),
        (
            "Detail",
            &[
                Mode, ModeSwitch, DetailMode, ViewDistance, CarModels, CarLodDistance, CarShading, ClearCoat, PixelLight, DynamicLights, LocalLight, TexturePack, Shadows, LightShadows, ShadowQuality, Halos, LensFlares, Fog, PixelFog, FogStrength, SkyImage, SkidMarks, Smoke, Sparks, Signs, WorldFill, TreeModels, Water, WaterReflect, Grass, GrassRange,
                MotionBlur, Ssao, Hdr, Tonemap, Exposure, Bloom, ColourGrade, Vignette, Grain,
            ],
        ),
    ]
};
/// The modern pause menu's CAMERA tab.
pub(super) const CAMERA_ROWS: &[GfxRow] = &[GfxRow::CamStart, GfxRow::CamMirror, GfxRow::CamFreeLook, GfxRow::CamPullIn, GfxRow::CamFollow];
/// The modern pause menu's HUD tab: every option of the race HUD in four groups (concept C, #300).
pub(super) const HUD_SECTIONS: [(&str, &[GfxRow]); 4] = {
    use GfxRow::*;
    [
        ("Look", &[HudStyle, HudScale, HudMargin]),
        ("On screen", &[InfoDetail, HudCluster, HudTotal, HudLapTime, HudBest, HudLap, HudPos, LapDelay, SectorDelta]),
        ("Map", &[HudMap, MapView, MapTurns, MapNumbers]),
        ("Rivals", &[NamePlates, PlateFrame, Proximity, LiveGaps, CheckpointInfo, CheckpointDelay]),
    ]
};
/// The rows of a modern screen in groups for this view: the main menu's Graphics screen shows `page`
/// of `MODERN_GFX_PAGES`; the pause tabs are one list, Graphics = both pages with their names as
/// captions, HUD = `HUD_SECTIONS`, Camera = its rows without a caption (see `gfx_rows`).
pub(super) fn modern_sections(screen: Screen, page: usize, v: &View) -> Vec<(&'static str, Vec<GfxRow>)> {
    let keep = |rows: &[GfxRow]| -> Vec<GfxRow> { rows.iter().copied().filter(|r| !(v.mode == RenderMode::Classic && r.remaster()) && !r.hidden(v)).collect() };
    match screen {
        Screen::PauseCam => vec![("", CAMERA_ROWS.to_vec())],
        Screen::PauseHud => HUD_SECTIONS.iter().map(|(n, rows)| (*n, keep(rows))).collect(),
        Screen::PauseGfx => MODERN_GFX_PAGES.iter().map(|(n, rows)| (*n, keep(rows))).collect(),
        _ => vec![(MODERN_GFX_PAGES[page].0, keep(MODERN_GFX_PAGES[page].1))],
    }
}
/// The rows of a modern screen as one list (`modern_sections` flattened).
pub(super) fn gfx_page_rows(screen: Screen, page: usize, v: &View) -> Vec<GfxRow> {
    modern_sections(screen, page, v).into_iter().flat_map(|(_, rows)| rows).collect()
}
/// Rows of the Classic graphic options list shown at once.
const GFX_LIST_ROWS: usize = 15;
/// Rows of the personal screen, Back included (the Statistics row is the one before Back).
/// CLASSIC 0x466b70: 0 Name, 1 Team, 2 Colour, 3..=12 Key 0..9, 13 Car colours, 14..=16 Car colour 1..3
/// (only while Car colours is On); ours after them: 17 Confirmations, 18 Intro movies, 19 Statistics, 20 Back.
const PERSONAL_ROWS: usize = 21;
/// The rows the cursor can reach (0x466f44: the car colour rows only with Car colours On).
/// Rows of the personal screen shown at once (the menu canvas is 400 px tall, 17 px apart from y 96).
const PERSONAL_SLOTS: usize = 18;
fn personal_rows(custom: bool) -> Vec<usize> {
    (0..PERSONAL_ROWS).filter(|&r| custom || !(14..=16).contains(&r)).collect()
}
/// The statistics screen's pages, and the list rows shown at once.
const STATS_PAGES: [&str; 4] = ["Overview", "Driving", "By circuit", "By car"];
const STATS_ROWS: usize = 14;
/// What the Sound device choice shows (#260, `Game::sound_device_view`): the value text, which arrows are at an end,
/// whether the pick is unavailable (not connected, or it would not open) or there is no output at all, and the list
/// of outputs for the Remaster info card.
pub(super) struct DeviceView {
    pub text: String,
    pub left_end: bool,
    pub right_end: bool,
    pub unavailable: bool,
    pub none: bool,
    pub entries: Vec<DeviceEntry>,
}

/// One output in that list: `sub` = what "System default" plays on; `lit` = the choice.
pub(super) struct DeviceEntry {
    pub name: String,
    pub sub: Option<String>,
    pub lit: bool,
    pub tag: Option<DeviceTag>,
}

#[derive(Clone, Copy, PartialEq)]
pub(super) enum DeviceTag {
    /// the output that actually plays while the pick is unavailable
    InUse,
    Unavailable,
}

/// The start-up movies, `movies.pak` entries played in this order (0x457ecc, 0x457f1f, 0x457f53) [O].
const INTRO: [usize; 3] = [0, 1, 2];
/// CLASSIC 0x469668, sprpos set 6: the Sound screen's items are the volume dials (0 sound effects,
/// 1 cd music), cd loop and the song-to-track table.
const SOUND_SET: usize = 6;
const SOUND_LOOP: usize = 2;
const SOUND_TABLE: usize = 3;
/// The sound device line (#260): a fifth item drawn above the first, so Up from the sound effects dial reaches it and
/// Down leaves it (the other items keep their numbers).
const SOUND_DEVICE: usize = 4;
/// REMASTER Sound screen, Volume tab: the sound device is the eighth row (#260), after the seven of `sound_items`
pub(super) const SOUND_DEVICE_ROW: usize = 7;
/// REMASTER Sound screen tabs (#259)
pub(super) const SOUND_VOLUME: usize = 0;
pub(super) const SOUND_SONGS: usize = 1;
/// CLASSIC 0x45fdac / 0x469cec: a held Left / Right turns a dial by frame ms * 0.07 * 3 of 255 per
/// frame (x2 with Shift, x4 with Ctrl), i.e. percent per second: 1000 * 0.21 * 100 / 255.
const DIAL_TURN_RATE: f32 = 82.4;
/// CLASSIC 0x46052c: a flip counter's digit closes on its value by frame ms * 0.2 * 0.02857, per second.
const COUNTER_RATE: f32 = 5.714;
/// The disc's last CD track (07-audio: 1 data, 2 results, 3 menu, 4..11 the races); the range of the
/// song-to-track table when Music/ has no rips to choose from.
const CD_LAST_TRACK: u32 = 11;
const GFX_BUTTONS: [&str; 3] = ["Apply", "OK", "Cancel"];
/// Seconds before an unconfirmed display change reverts.
const KEEP_SECONDS: f32 = 15.0;

/// The graphic options screen's edit state: the display draft, the live rows as of the
/// last Apply (for Cancel), and the selected button.
#[derive(Clone, Copy)]
pub(super) struct GfxEdit {
    pub(super) display: Display,
    backup: GfxBackup,
    button: usize,
    /// shown page of GFX_PAGES
    pub(super) page: usize,
}

impl GfxEdit {
    pub(super) fn new(s: &Settings) -> GfxEdit {
        GfxEdit { display: s.display, backup: GfxBackup::of(s), button: 0, page: 0 }
    }
    /// after Apply: the new baseline, on the same page
    fn rebase(self, s: &Settings) -> GfxEdit {
        GfxEdit { page: self.page, ..GfxEdit::new(s) }
    }
}

/// The graphic options that take effect at once.
#[derive(Clone, Copy)]
struct GfxBackup {
    fps_limit: u32,
    menu_stretch: bool,
    view: View,
    music: bool,
    skid_marks: bool,
    checkpoint_delay: u32,
    cam: CamOpts,
    /// the HUD Settings toggles
    hud: HudSettings,
    /// detail mode, name plates, track detail, car detail, info detail, checkpoint info, road map
    levels: [usize; 8],
}

impl GfxBackup {
    fn of(s: &Settings) -> GfxBackup {
        GfxBackup {
            fps_limit: s.fps_limit,
            menu_stretch: s.menu_stretch,
            view: s.view,
            music: s.music,
            skid_marks: s.skid_marks,
            checkpoint_delay: s.checkpoint_delay,
            cam: s.cam,
            hud: s.hud,
            levels: [s.detail_mode, s.name_plates, s.track_detail, s.info_detail, s.checkpoint_info, s.road_map, s.perspective_on, s.perspective],
        }
    }

    fn restore(&self, s: &mut Settings) {
        (s.fps_limit, s.menu_stretch, s.view, s.music, s.skid_marks, s.checkpoint_delay) = (self.fps_limit, self.menu_stretch, self.view, self.music, self.skid_marks, self.checkpoint_delay);
        s.cam = self.cam;
        s.hud = self.hud;
        [s.detail_mode, s.name_plates, s.track_detail, s.info_detail, s.checkpoint_info, s.road_map, s.perspective_on, s.perspective] = self.levels;
    }
}

#[derive(Clone, Copy)]
struct KeepDisplay {
    /// selected button: 0 keep, 1 revert
    sel: usize,
    /// seconds until it reverts by itself
    left: f32,
    old: Display,
    /// OK was pressed: leave the screen once kept
    leave: bool,
}

/// A key in a two-button requester.
enum Req {
    None,
    Move(usize),
    /// closed with this button (Esc picks the second)
    Pick(usize),
}

fn requester_key(k: KeyCode, sel: usize) -> (Req, Option<u32>) {
    match k {
        KeyCode::ArrowLeft | KeyCode::ArrowRight | KeyCode::ArrowUp | KeyCode::ArrowDown => {
            (Req::Move(1 - sel), Some(if matches!(k, KeyCode::ArrowLeft | KeyCode::ArrowUp) { ev::ARROW_LEFT } else { ev::ARROW_RIGHT }))
        }
        KeyCode::Enter | KeyCode::NumpadEnter | KeyCode::Space => (Req::Pick(sel), Some(ev::SELECT)),
        KeyCode::Escape | KeyCode::Backspace => (Req::Pick(1), Some(ev::ESCAPE)),
        _ => (Req::None, None),
    }
}
/// The in-race menu (string table at 0x4ae838; "Retire race"/"Abort race" in other modes).
/// "HUD Settings" is ours (REMASTER, 9.21).
/// REMASTER pause menu, Race tab: the actions (Resume, Restart, Exit)
const PAUSE_ACTIONS: usize = 3;
/// REMASTER pause menu: seconds Enter is held to restart or exit (concept B / C, #300)
pub const HOLD_S: f32 = 0.8;

/// The pause menu's tabs left to right, as the screen each one is (concept C, #300)
const PAUSE_TAB_SCREENS: [Screen; 6] = [Screen::Pause, Screen::PauseGfx, Screen::PauseSfx, Screen::PauseControls, Screen::PauseCam, Screen::PauseHud];

/// REMASTER hold to confirm: `v` fills 0..1 over `HOLD_S` while the confirm key is down on an armed
/// action, drains twice as fast when it is let go, and the action runs once at 1.
#[derive(Clone, Copy, Default, Debug)]
pub struct Hold {
    pub v: f32,
    pub armed: bool,
}

impl Hold {
    /// One frame: true once when the fill reaches 1.
    pub fn step(&mut self, dt: f32, down: bool) -> bool {
        if self.armed && down {
            self.v += dt / HOLD_S;
            if self.v >= 1.0 {
                *self = Hold::default();
                return true;
            }
        } else {
            self.armed = false;
            self.v = (self.v - 2.0 * dt / HOLD_S).max(0.0);
        }
        false
    }

    /// Moving, leaving or switching tab drops the hold at once.
    pub fn clear(&mut self) {
        *self = Hold::default();
    }
}

const PAUSE_ITEMS: [&str; 6] = ["Continue race", "Restart race", "GFX Options", "SFX Options", "HUD Settings", "Quit to menu"];
/// Rows of the HUD Settings screen, Back included.
const HUD_ROWS: usize = 14;

/// What `--playback` leaves for `main` to apply: the held keys, the pointer (window px) and the left button.
pub type ShotPlayback = (Vec<KeyCode>, Option<(f32, f32)>, bool);

pub struct Game {
    pub root: PathBuf,
    pub assets: Assets,
    pub cars: Vec<CarInfo>,
    pub tracks: Vec<TrackInfo>,
    pub horns: Vec<Horn>,
    /// sound events for the audio layer, drained each frame
    pub sfx: Vec<Sfx>,
    /// cheat codes typed in the pause menu (cheats.rs)
    pub cheats: crate::cheats::Cheats,
    credits: Vec<String>,
    /// CLASSIC 0x46dabb: the hidden credits page ("Code:" ... "Organizing:"), shown once the club message has appeared in
    /// this session (`club_seen`, `[0x750828]`) or while Left Ctrl + Left Alt + Right Alt are held (`0x46dd82`)
    credits_alt: Vec<String>,
    pub club_seen: bool,
    credits_keys: bool,
    /// `Data/d3d/tullinge.iff`: the Atlantika roof scrolltext (plain text, `cha_InfoInit` `0x41212f`)
    pub egg_text: Vec<u8>,
    /// `Data/sprpos.dta`: where the original puts the front end's text (motor_data::sprpos)
    sprpos: SprPos,
    /// the Personal screen's colour strip (`load_strip`)
    strip: [[u8; 3]; STRIP_CELLS],
    /// CD track numbers found in Music/ (the menu CD track choices), ascending
    cd_tracks: Vec<u32>,
    /// seconds of each CD track found in Music/ (the song-to-track table's Length column)
    cd_lengths: HashMap<u32, u32>,
    /// the CD tracks' titles (file names), for the Remaster Songs tab
    cd_names: HashMap<u32, String>,
    /// Sound screen: the table's current row (an index of `tracks`), the digit counters' positions
    /// (rows previous / current / next x columns CD track / minutes / seconds x units, tens),
    /// whether they jump (the list scrolled), the `onoff` frame, the preview song, and the held
    /// Left / Right keys with the dial's fractional volume
    sound_list: usize,
    sound_roll: [f32; 18],
    sound_snap: bool,
    loop_frame: f32,
    /// Car / Track screens: the rolling frames of `tman`, the record race `onoff` and the catch-up `onoff` (`ROLL_*`)
    rolls: [f32; 3],
    /// Classic menus: the settled slot drawer's counters (glitch, title turn over), stepped while drawing
    settle: std::cell::RefCell<slide_fx::Settle>,
    pub cd_preview: Option<u32>,
    /// REMASTER Sound screen (#259): the tab shown (`SOUND_VOLUME` / `SOUND_SONGS`) and the seconds the preview has played
    sound_page: usize,
    preview_t: f32,
    /// Sound screen, sound device (#260): the outputs found when the screen opened (`refresh_sound_devices`), the name of what
    /// "System default" plays on, a fixed list for headless shots (`--sound-devices`), and whether the picked output could not
    /// be opened (set by `GameAudio::sync`, which plays on the default then)
    sound_devices: Vec<motor_audio::OutputDevice>,
    sound_default: Option<String>,
    sound_devices_fixed: Option<Vec<motor_audio::OutputDevice>>,
    pub sound_device_failed: bool,
    /// no sound output is playing at all (set by `GameAudio::sync`; a headless shot with an empty `--sound-devices`)
    pub sound_silent: bool,
    arrows_held: (bool, bool),
    /// the start camera changed in a race (#419): the settings are written on the next input frame
    cam_dirty: bool,
    /// REMASTER pause menu: the Restart / Exit hold
    pub pause_hold: Hold,
    dial_carry: f32,
    /// the menu sound a Sound screen key asked for, instead of the generic one `key` derives
    ev_override: Option<u32>,
    pub settings: Settings,
    /// names of the connected screens (filled by the window layer; empty when headless)
    pub monitors: Vec<String>,
    /// the graphics adapter's name (filled by the window layer), the Graphics screen's Render device row
    pub render_device: String,
    pub screen: Screen,
    /// seconds on this screen, and overall
    pub t: f32,
    /// CLASSIC 0x46284c leaving state (`[0x7511c0]`): the old screen slides out before the switch to `to`
    leave: Option<Leave>,
    pub clock: f32,
    /// CLASSIC the Track screen's laps counter digits (ones, tens), see `roll_laps`
    laps_roll: [f32; 2],
    /// CLASSIC menu background rings and flying line objects
    menu_fx: crate::menu_fx::MenuFx,
    sel: usize,
    /// REMASTER modern dial: needle angle (radians) of the current dial
    needle: f32,
    /// CLASSIC dial: needle angle in 4096ths of a turn (0x756ad4)
    needle_q: f32,
    /// `emh` frame of the Options difficulty word (0x7519c8): 0 easy, 8 medium, 16 hard, between = roll.
    diff_roll: f32,
    /// car screen stats as drawn (top speed, acceleration, handling: the car's own fields), easing to
    /// `gauge_stats`; `gauge_fills` maps them to the dials. Kept across screens like the original's
    /// globals (0x752a20..28), so the dials glide from the last car shown
    gauges: [f32; 3],
    /// REMASTER Car screen: the car (index into `cars`) the selector just left, for the dials' previous-car tick;
    /// any other key and every entry to the screen clear it
    prev_car: Option<usize>,
    /// REMASTER Car screen: `clock` when the horn was last tried (the speaker icon pulses for a second); cleared on entry
    honk_at: Option<f32>,
    pub race: Option<RaceState>,
    pending: Option<scene::Scene>,
    /// the light probe source built with `pending`'s race scene (`LoadPlan::run`)
    pending_bake: Option<motorview::probes::BakeSource>,
    /// the CPU half of `pending`'s upload, built with it on the worker
    pending_upload: Option<SceneUpload>,
    loaded: Loaded,
    /// the race loading on a worker (#335)
    loading: Option<LoadJob>,
    /// where the loading screen was entered from (screen, selected item): Esc goes back
    load_from: (Screen, usize),
    /// REMASTER loading screen (#90): the race's facts, taken as the screen opens
    load_facts: Option<menu_modern::LoadFacts>,
    /// `--load-progress`: the bar a shot of the loading screen shows (no load runs in a shot)
    pub load_preview: Option<f32>,
    /// garage: car position (view space, on the road) and heading
    spot: (Vec3, Vec3),
    garage_wheels: Vec<([Vec3; 4], f32)>,
    /// REMASTER: the garage scene, kept after its upload so the next visit only uploads it again
    garage_scene: Option<scene::Scene>,
    /// REMASTER: the garage scene being built on a worker while a screen before the Car screen is open
    garage_job: Option<mpsc::Receiver<Result<scene::Scene, String>>>,
    /// start `garage_job` in the background (the window, not the headless shots)
    pub prefetch: bool,
    /// the Car and Track screens' picture
    pic: PicFade,
    menu_drive: Option<MenuDrive>,
    steer: SteerKeys,
    /// CLASSIC (#376): the key / button brake ramp and the x1.1 analog pedals
    pedals: Pedals,
    pub quit: bool,
    /// the movie on screen (`Screen::Intro`, `Screen::LeagueMovie`) and the movies after it (#42, #43)
    pub movie: Option<Movie>,
    movie_queue: Vec<usize>,
    /// the install has `movies/movies.pak`
    has_movies: bool,
    /// bumped for every movie opened: the sound output attaches its stream once per movie
    pub movie_serial: u32,
    /// the sound output keeps the movie's clock (a sound device runs), see `Movie::hold`
    pub movie_hold: bool,
    /// (movie, frame, classic) of the picture in the atlas slot, and a scratch buffer for it
    movie_shown: Option<(usize, u32, bool)>,
    movie_rgba: Vec<u8>,
    /// the exit requester is open: selected button (0 "Yes, please", 1 "No way!")
    pub confirm_exit: Option<usize>,
    /// graphic options being edited (display rows wait for Apply/OK)
    gfx: Option<GfxEdit>,
    /// "Keep these settings?" after a display change was applied
    keep_display: Option<KeepDisplay>,
    /// the AI drives your car too
    pub demo: bool,
    /// the attract demo (#210, GameType 5): the next or running race is the idle main menu's AI race
    pub attract: bool,
    /// the idle timer runs on the main menu (the window and `--idle`; off for every other headless shot)
    pub attract_idle: bool,
    /// seconds since the last input on the main menu, and the limit (120 s, 10 s after a demo that ran to its end)
    idle_s: f32,
    idle_limit: f32,
    /// the track of the last race (the demo never repeats it) and the demo's pick
    last_track: Option<usize>,
    attract_track: usize,
    attract_saved: Option<AttractSaved>,
    /// save settings to `settings_path()` whenever they change (interactive runs)
    persist: bool,
    /// the tracks with an unlock flag (`League/<Track>.FLG`); None = nothing is locked (headless
    /// shots and tests; `--locked` simulates an install)
    pub flagged: Option<Vec<bool>>,
    pub controls: Controls,
    /// gamepad rumble (#383): the mixer, its Vibration setting (0..1) and this frame's motors
    rumble: crate::rumble::Rumble,
    rumble_gain: f32,
    rumble_cmd: (f32, f32),
    /// control options: the car task waiting for a key or button
    pub(super) capture: Option<Capture>,
    /// CLASSIC Controls screen: the focus is on the task tree, not the layout column (0x7529d8)
    ctl_tasks: bool,
    /// results computed when the race ends: (slot, points, time ms or None, best lap)
    results: Vec<(usize, u32, Option<u32>, Option<u32>)>,
    /// REMASTER (#94): the numbers of the modern Results screen (laps, sectors, grid, laps led), built with `results`
    results_sheet: Option<motor_sim::sheet::RaceSheet>,
    /// REMASTER Results (#94): the ranking tables the race entered (the Records tab), best lap first
    results_ranks: Vec<hiscore::RankView>,
    /// REMASTER Results: X hides the glass to look at the finish scene, any key brings it back (`self.sel` = the driver)
    results_watch: bool,
    /// REMASTER Results: the tab on show (an index into `results_tabs`)
    results_tab: usize,
    /// the Laps tab's first row shown (a long race scrolls)
    results_scroll: usize,
    /// REMASTER Results: the human's division as it stood before the league race (the League tab's movement arrows)
    league_moves: Vec<league::Moved>,
    /// shots: the season code `--season-end` forces after the race (`finish_league_race`)
    preview_outcome: Option<Outcome>,
    /// F8 (or Pause) photo freeze: the race stands still with no pause overlay, so F9 / F12 give
    /// Classic vs Remaster comparison shots of the same frame (ours, not saved)
    pub frozen: bool,
    /// the mouse pointer on the HUD canvas (px) and that canvas' width, from the window's last cursor move (#352)
    pub pointer: Option<([f32; 2], f32)>,
    /// fixed free-look stick instead of the pads (headless shots)
    pub free_look: Option<Vec2>,
    /// personal screen: the text row being typed into, and its text before editing
    editing: Option<(usize, String)>,
    /// REMASTER Personal screen (#91): the tab shown (Driver, Livery, Chat keys, Career, Records) and each
    /// tab's selected row (`sel` is the shown tab's)
    personal_tab: usize,
    personal_sel: [usize; 5],
    /// REMASTER Personal, Records tab (#91 phase 6): the table lists the cars, not the circuits
    records_cars: bool,
    /// REMASTER Personal, Livery tab (#91): the palette cursor open on a paint zone
    paint_pick: Option<PaintPick>,
    /// REMASTER Personal (#91 phase 4): the pad keyboard sheet is open on the text row of `editing`, with its
    /// cursor on this key (`menu_modern/requester.rs` `osk_move`)
    osk: Option<usize>,
    pub game_type: GameType,
    /// the player's statistics (stats.cfg), and the statistics screen's page and list scroll
    pub(crate) stats: stats::Stats,
    stats_page: usize,
    stats_scroll: usize,
    rank: RankView,
    /// the table the rankings screen shows (reloaded when a filter changes)
    rank_table: Option<hiscore::Table>,
    /// REMASTER Rankings (#93): your standing in every circuit's (division's) table for the current filters
    rank_slots: Vec<menu_modern::RankSlot>,
    /// headless shots only: read the `.MHI` tables from this folder, and count `settings.player_name` as you
    hiscore_dir: Option<PathBuf>,
    /// League/Profiles/*.adp
    pub profiles: Vec<Profile>,
    /// the saved leagues (loaded on entering the league dial) and the current one
    leagues: Vec<League>,
    league: usize,
    /// the New / Delete requester: (delete, selected button)
    league_req: Option<(bool, usize)>,
    /// CLASSIC: the season movie being played is the final's; the Main menu follows it (not the League dial)
    final_won: bool,
    /// CLASSIC: the standings opened after a league race, so Enter goes on to the car screen
    standings_next: bool,
    /// the name typed for a new league (`editing` row 2)
    league_name: String,
    /// the running league race: (place, grid slot) of each car
    league_field: Vec<(usize, usize)>,
    /// the Ghost screen's list of `Demos/*.MDE` recordings, by name (`sel` = the current one)
    replays: Vec<(PathBuf, Mde)>,
    /// REMASTER: what the Glass Ghost screen shows of each (`replays`' order; `load_replays`)
    replay_info: Vec<menu_modern::replays::RecInfo>,
    /// headless shots: the folder of recordings read instead of the settings folder's `Demos/` (`--demos`)
    demo_preview: Option<PathBuf>,
    /// headless `--save-demo`: where the finished recording is written
    pub demo_export: Option<PathBuf>,
    /// the Ghost screen's focus (0x7511b0 +0x14): 0 ghost race, 1 replay race, 2 the list
    ghost_row: usize,
    /// the mode the list acts on: the last row 0 / 1 that had the focus (the original's GameType 3 / 6)
    ghost_mode: usize,
    /// the recording picked last (lower-case name), selected again when the screen reopens (0x462b1c: `DemoFileName`)
    ghost_last: Option<String>,
    /// REMASTER Lap Analysis (#429): its sides, picker, requester, re-simulation and the session's caches
    pub(crate) analysis: menu_modern::Analysis,
    /// the Ghost screen's "Delete demo file." requester: the selected button
    ghost_del: Option<usize>,
    /// the Multi page's refusal line
    multi_msg: Option<String>,
    /// Ghost mode: the chosen `.MDE` (loaded in `start_race`)
    pub ghost_demo: Option<Mde>,
    /// demo play: the `.MDE` to watch (loaded by the loading screen, `start_watch`)
    pub watch_next: Option<Mde>,
    /// Record race: the recording being offered for saving (`save_demo`)
    demo_save: Option<DemoSave>,
    /// the Ghost mode list's last refusal ("Ghostmode error! ...")
    replay_msg: Option<String>,
    /// headless `--save-replay`: also write the `.mhr` input recording
    pub save_mhr: bool,
    /// headless `--ghost`: the Time Attack ghost instead of the saved best lap
    pub ghost_override: Option<GhostTrack>,
    /// what the last league race meant, until its standings screen is left
    league_outcome: Option<Outcome>,
    /// the division the standings screen shows
    league_div: usize,
    rng: league::Rng,
}

fn f(c: &motor_data::cfg::Config, k: &str) -> f32 {
    c.get(k).and_then(|e| e.f32(0)).unwrap_or(0.0)
}

/// A volume one step (5 %) up (`d` > 0) or down, within 0..=100.
fn step_volume(v: u32, d: i32) -> u32 {
    (v as i32 + d.signum() * 5).clamp(0, 100) as u32
}

/// The next CD track present in `tracks` (ascending) up (`d` > 0) or down from `cur`, stopping
/// at the ends; `cur` itself when no track files exist. A `cur` not in the list snaps to its neighbour.
/// One step of the Sound device choice (#260) over "System default" (0) and `n` outputs (1..=n), clamped at both ends. A pick
/// that is not connected (`None`) sits before "System default": a step up leaves it for the default, a step down stays.
fn step_device_choice(cur: Option<usize>, d: i32, n: usize) -> Option<usize> {
    match cur {
        None if d < 0 => None,
        None => Some(0),
        Some(c) => Some((c as i32 + d).clamp(0, n as i32) as usize),
    }
}

fn step_cd_track(cur: u32, tracks: &[u32], d: i32) -> u32 {
    let Some(last) = tracks.len().checked_sub(1) else { return cur };
    let i = tracks.iter().position(|&t| t >= cur).unwrap_or(last);
    let j = if tracks[i] == cur {
        (i as i32 + d).clamp(0, last as i32) as usize
    } else if d > 0 {
        i
    } else {
        i.saturating_sub(1)
    };
    tracks[j]
}

/// The staff list stored in motor.exe ("Programming:" ... the line after "Produced by:").
fn read_credits(root: &std::path::Path) -> Vec<String> {
    let Ok(exe) = motor_data::fs::read(root, "motor.exe") else { return Vec::new() };
    let Some(at) = exe.windows(12).position(|w| w == b"Programming:") else { return Vec::new() };
    let mut out = Vec::new();
    let mut cur = String::new();
    for &b in &exe[at..] {
        if b == 0 {
            if !cur.is_empty() {
                let done = out.last().is_some_and(|l: &String| l == "Produced by:");
                out.push(std::mem::take(&mut cur));
                if done {
                    break;
                }
            }
        } else {
            cur.push(b as char); // Latin-1
        }
    }
    out
}

/// The hidden credits page of `0x46dabb`: the 12 strings from "Code:" to the last name before the default page's
/// "Programming:" (`0x4d6298..0x4d638c`).
fn read_credits_alt(root: &std::path::Path) -> Vec<String> {
    let Ok(exe) = motor_data::fs::read(root, "motor.exe") else { return Vec::new() };
    let Some(at) = exe.windows(15).position(|w| w == b"Code:\0Atlan/CRG") else { return Vec::new() };
    exe[at..].split(|&b| b == 0).take(12).map(|s| s.iter().map(|&b| b as char).collect()).collect()
}

/// A car horn from Sounds/CarHorns/*.chf.
#[derive(Clone, Debug)]
pub struct Horn {
    /// "Name" (shown in the car menu)
    pub name: String,
    /// "SampleName", relative to Sounds/
    pub sample: String,
    /// "VolumeChange" / "PitchChange", percent
    pub volume_change: f32,
    pub pitch_change: f32,
}

/// Horns from Sounds/CarHorns/*.chf, sorted by number.
fn read_horns(root: &std::path::Path) -> Vec<Horn> {
    let Some(dir) = motor_data::fs::find_ci(root, "Sounds/CarHorns") else { return Vec::new() };
    let mut files: Vec<_> = std::fs::read_dir(dir)
        .map(|d| d.flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|x| x.eq_ignore_ascii_case("chf"))).collect())
        .unwrap_or_default();
    files.sort_by_key(|p| {
        let s = p.file_stem().map(|s| s.to_string_lossy().to_ascii_lowercase()).unwrap_or_default();
        s.trim_start_matches("horn").parse::<u32>().unwrap_or(0)
    });
    files
        .iter()
        .filter_map(|p| {
            let c = motor_data::cfg::Config::parse(&std::fs::read(p).ok()?);
            let v = |k: &str| c.get(k).and_then(|e| e.f32(0)).unwrap_or(0.0);
            Some(Horn {
                name: c.get("Name")?.values.first()?.clone(),
                sample: c.get("SampleName")?.values.first()?.replace('\\', "/"),
                volume_change: v("VolumeChange"),
                pitch_change: v("PitchChange"),
            })
        })
        .collect()
}

fn read_wire(root: &std::path::Path, trk: &motor_data::cfg::Config) -> Option<Wire> {
    let rel = trk.get("LineObjectName")?.values.first()?.clone();
    let raw = motor_data::fs::read(root, &rel).ok()?;
    let data = motor_data::tbl::load(&raw, None).ok()?.data;
    let l = motor_data::lob::parse(&data).ok()?;
    Some((l.vertices, l.edges))
}

impl MenuDrive {
    /// Follow the car in the ground plane: its position, its heading damped over `dt`
    /// (infinite: snap).
    fn pose(&mut self, dt: f32) {
        let car = &self.sess.race.racers[0].car;
        let flip = Vec3::new(1.0, 1.0, -1.0);
        self.pos = car.pos * flip;
        let fwd = car.rot * Vec3::Z * flip;
        let fwd = Vec3::new(fwd.x, 0.0, fwd.z).normalize_or(self.dir);
        let k = 1.0 - (-MENU_DRIVE_TURN_RATE * dt).exp();
        self.dir = if dt.is_finite() { self.dir.lerp(fwd, k).normalize_or(fwd) } else { fwd };
    }
}

impl Game {
    pub fn new(root: PathBuf) -> Game {
        let assets = Assets::load(&root);
        let has_movies = motor_data::fs::find_ci(&root, "movies/movies.pak").is_some();
        let menu_fx = crate::menu_fx::MenuFx::load(&root);
        let mut cars = (1..=10)
            .filter_map(|id| {
                let c = motor_data::fs::config(&root, &format!("Game/car{id:02}.car")).ok()?;
                Some(CarInfo {
                    id,
                    name: c.get("CarName").map(|e| e.values.join(" ")).unwrap_or_else(|| format!("car {id}")),
                    division: f(&c, "Division") as u32,
                    top_speed: f(&c, "TopSpeed"),
                    accel: f(&c, "Acceleration"),
                    handling: f(&c, "Handling"),
                    gears: f(&c, "Gears") as u32,
                    weight: f(&c, "Weight"),
                    size: [f(&c, "Length"), f(&c, "Width"), f(&c, "Height")],
                    zones: scene::paint_zones(&root, id),
                    sphere: scene::body_sphere(&root, id),
                    footprint: scene::body_footprint(&root, id),
                })
            })
            .collect::<Vec<CarInfo>>();
        // CLASSIC: the car list is sorted by Division, ascending and stable (0x4613f9 calls the
        // bubble sort 0x45fa2c), so it runs in unlock order: tier 1 cars first.
        cars.sort_by_key(|c| c.division);
        let tracks = (1..=8)
            .filter_map(|n| {
                let c = motor_data::fs::config(&root, &format!("Game/Track{n}.trk")).ok()?;
                let mi = minimap::map_info(&c);
                Some(TrackInfo {
                    n,
                    name: c.get("TrackName").map(|e| e.values.join(" ")).unwrap_or_else(|| format!("track {n}")),
                    division: f(&c, "Division") as u32,
                    laps: c.get("Laps").and_then(|e| e.f32(0)).unwrap_or(3.0) as u32,
                    cd_track: c.get("CDTrack").and_then(|e| e.f32(0)).unwrap_or(0.0) as u32,
                    map_info: mi,
                    map_offset: minimap::map_offset(c.get("TrackMapName").and_then(|e| e.values.first()).map_or("", |s| s.as_str())),
                    wire: read_wire(&root, &c),
                    outline: Vec::new(),
                    profile: motor_sim::race::load_centreline(&root, &c).ok().map(|l| l.profile()),
                    length_m: c.get("Length").and_then(|e| e.f32(0)).unwrap_or(0.0),
                    location: c.get("Location").map(|e| e.values.join(" ")).unwrap_or_default(),
                    length_text: c.get("Length").map(|e| e.values.join(" ")).unwrap_or_default(),
                })
            })
            .collect::<Vec<_>>();
        let mut tracks = tracks;
        // CLASSIC: the track list is sorted by Division too, ascending and stable (0x461761 calls
        // the bubble sort 0x45fa2c at front-end init); single race, Time Attack, Ghost mode and the
        // league's season list (0x4534b8) all walk this order.
        tracks.sort_by_key(|t| t.division);
        // CLASSIC 0x4618da..0x461919: every LOB is centred, the first track of the (Division-sorted)
        // list is fitted to 170 and the others take its factor
        let k = tracks
            .iter()
            .min_by_key(|t| t.division)
            .and_then(|t| t.wire.as_ref())
            .map_or(1.0, |(v, _)| {
                let mut v = v.clone();
                crate::menu_fx::centre(&mut v);
                crate::menu_fx::fit(&mut v, TRACK_OUTLINE_SIZE)
            });
        for t in &mut tracks {
            if let Some((v, _)) = &t.wire {
                let mut v = v.clone();
                crate::menu_fx::centre(&mut v);
                crate::menu_fx::scale(&mut v, k);
                t.outline = v;
            }
        }
        let laps = tracks.first().map_or(3, |t| t.laps);
        let horns = read_horns(&root);
        let credits = read_credits(&root);
        let credits_alt = read_credits_alt(&root);
        let egg_text = motor_data::fs::read(&root, "Data/d3d/tullinge.iff").unwrap_or_default();
        let strip = load_strip(&root);
        let sprpos = motor_data::fs::read(&root, "Data/sprpos.dta").ok().and_then(|d| SprPos::parse(&d).ok()).unwrap_or_default();
        let cd_tracks = crate::audio::cd_track_numbers(&root);
        let cd_lengths = crate::audio::cd_track_lengths(&root);
        let cd_names = crate::audio::cd_track_names(&root);
        let controls = Controls::new(&root);
        let layout = controls.layout().name.clone();
        let mut g = Game {
            root,
            assets,
            cars,
            tracks,
            horns,
            sfx: Vec::new(),
            cheats: Default::default(),
            credits,
            credits_alt,
            club_seen: false,
            credits_keys: false,
            egg_text,
            sprpos,
            strip,
            cd_tracks,
            cd_lengths,
            cd_names,
            sound_list: 0,
            sound_roll: [0.0; 18],
            sound_snap: true,
            loop_frame: 0.0,
            rolls: [0.0; 3],
            settle: std::cell::RefCell::new(slide_fx::Settle::new(0x1998)),
            cd_preview: None,
            sound_page: SOUND_VOLUME,
            preview_t: 0.0,
            sound_devices: Vec::new(),
            sound_default: None,
            sound_devices_fixed: None,
            sound_device_failed: false,
            sound_silent: false,
            arrows_held: (false, false),
            cam_dirty: false,
            pause_hold: Hold::default(),
            dial_carry: 0.0,
            ev_override: None,
            settings: Settings { laps, layout, ..Settings::default() },
            monitors: Vec::new(),
            render_device: String::new(),
            screen: Screen::Main,
            t: 0.0,
            leave: None,
            clock: 0.0,
            laps_roll: [0.0; 2],
            menu_fx,
            sel: 0,
            needle: 0.0,
            needle_q: 0.0,
            diff_roll: 0.0,
            gauges: [0.0; 3],
            prev_car: None,
            honk_at: None,
            race: None,
            pending: None,
            pending_bake: None,
            pending_upload: None,
            loaded: Loaded::None,
            loading: None,
            load_facts: None,
            load_preview: None,
            load_from: (Screen::Main, 0),
            spot: (Vec3::ZERO, Vec3::NEG_Z),
            garage_wheels: Vec::new(),
            garage_scene: None,
            garage_job: None,
            prefetch: false,
            pic: PicFade::default(),
            menu_drive: None,
            steer: SteerKeys::default(),
            pedals: Pedals::default(),
            quit: false,
            movie: None,
            movie_queue: Vec::new(),
            has_movies,
            movie_serial: 0,
            movie_hold: false,
            movie_shown: None,
            movie_rgba: Vec::new(),
            confirm_exit: None,
            gfx: None,
            keep_display: None,
            demo: false,
            attract: false,
            attract_idle: false,
            idle_s: 0.0,
            idle_limit: ATTRACT_IDLE_S,
            last_track: None,
            attract_track: 0,
            attract_saved: None,
            persist: false,
            flagged: None,
            controls,
            rumble: Default::default(),
            rumble_gain: PAD_TUNE_DEFAULT[3] as f32 / 100.0,
            rumble_cmd: (0.0, 0.0),
            capture: None,
            ctl_tasks: false,
            results: Vec::new(),
            results_sheet: None,
            results_ranks: Vec::new(),
            results_watch: false,
            results_tab: 0,
            results_scroll: 0,
            league_moves: Vec::new(),
            preview_outcome: None,
            frozen: false,
            pointer: None,
            free_look: None,
            editing: None,
            personal_tab: 0,
            personal_sel: [0; 5],
            records_cars: false,
            paint_pick: None,
            osk: None,
            game_type: GameType::Single,
            stats: stats::Stats::default(),
            stats_page: 0,
            stats_scroll: 0,
            rank: RankView { kind: 2, track: 0, division: 0, laps: 1, diff: Difficulty::Medium },
            rank_table: None,
            rank_slots: Vec::new(),
            hiscore_dir: None,
            profiles: Vec::new(),
            leagues: Vec::new(),
            league: 0,
            league_req: None,
            final_won: false,
            standings_next: false,
            league_name: String::new(),
            league_field: Vec::new(),
            replays: Vec::new(),
            replay_info: Vec::new(),
            demo_preview: None,
            demo_export: None,
            ghost_row: 0,
            ghost_mode: 0,
            ghost_last: None,
            analysis: Default::default(),
            ghost_del: None,
            multi_msg: None,
            ghost_demo: None,
            watch_next: None,
            demo_save: None,
            replay_msg: None,
            save_mhr: false,
            ghost_override: None,
            league_outcome: None,
            league_div: 3,
            rng: league::Rng(0x1998),
        };
        let names: Vec<String> = g.cars.iter().map(|c| c.name.clone()).collect();
        g.profiles = league::load_profiles(&g.root, &names);
        g.sync_controls();
        g
    }

    /// Point the controls at the settings: active layout, and the Custom layout's bindings
    /// (made from the first layout when the install has no custom.clo).
    fn sync_controls(&mut self) {
        let c = &mut self.controls;
        if c.custom_mut().is_none() {
            let mut l = c.layouts[c.find(DEFAULT_LAYOUT_NAME).unwrap_or(0)].clone();
            l.name = "Custom".into();
            c.layouts.insert(0, l);
        }
        // the layouts in use: the file's bindings with the player's on top; one that is the file's again is dropped
        for l in &mut c.layouts {
            let o = self.settings.binds.entry(l.name.to_ascii_lowercase()).or_insert([None; TASKS.len()]);
            for ((b, f), o) in l.bind.iter_mut().zip(l.file).zip(o.iter_mut()) {
                if *o == f {
                    *o = None;
                }
                *b = o.or(f);
            }
        }
        self.settings.binds.retain(|_, o| o.iter().any(Option::is_some));
        c.pad_bind[..CAR_TASKS].copy_from_slice(&self.settings.pad);
        // the fixed pad controls (Info, the camera views) give way to a car task bound to the same
        // control, so one press does one thing (bug #181: Change camera on R3 also stepped Info)
        for (b, d) in c.pad_bind.iter_mut().zip(DEFAULT_PAD_BIND).skip(CAR_TASKS) {
            *b = d.filter(|p| !self.settings.pad.contains(&Some(*p)));
        }
        let t = self.settings.pad_tune.map(|x| x as f32 / 100.0);
        c.tune = PadTune { stick_dead: t[0], trigger_dead: t[1], steer_sens: t[2] };
        self.rumble_gain = t[3];
        c.active = c.find(&self.settings.layout).or_else(|| c.find(DEFAULT_LAYOUT_NAME)).unwrap_or(0);
        self.settings.layout = c.layout().name.clone();
    }

    /// Enter / pad A (design choice, #216): before the "3" skip the start fly-by to the "3"; from the
    /// "3" on `end_camera` ends the fly-by camera (not for a pad A bound to a driving task).
    fn skip_fly_by(&mut self, end_camera: bool) {
        if let Some(r) = self.race.as_mut().filter(|r| !r.watching()) {
            if r.sess.race.pre_race_s() < startcam::HOLD_S {
                r.sess.skip_intro();
                r.pending_skip = true;
            } else if end_camera {
                r.sess.fly_by = false;
            }
        }
    }

    /// Feed gamepad buttons to the menus (and Start to the race as Esc); while a binding
    /// waits for input, the first pad or mouse control pressed is bound.
    pub fn poll_pads(&mut self) {
        let keys = self.controls.poll();
        if let Some(cap) = self.capture {
            if cap.pad || cap.any {
                // Start cancels, so it never gets bound
                match self.controls.take_pad_pressed() {
                    Some(PadIn::Button(gilrs::Button::Start)) => {
                        self.capture = None;
                        self.sfx.push(Sfx::Menu(ev::ESCAPE));
                    }
                    Some(p) => self.bind_pad(cap.task, p),
                    None => {}
                }
            }
            if !cap.pad && self.capture.is_some() {
                if let Some(c) = self.controls.take_pressed() {
                    // the modern Keyboard page takes keys and mouse buttons only
                    if !(self.settings.view.modern_menu() && matches!(c, Code::Joy(_) | Code::JoyButton(_))) {
                        self.bind(cap.task, c);
                    }
                }
            }
            return;
        }
        for k in keys {
            if self.input_seen() {
                break;
            }
            // Start keeps the text on the pad keyboard (Esc undoes it, as on the keyboard)
            if k.start && self.osk.is_some() {
                self.osk_keep();
                continue;
            }
            // A bound to a driving task still skips the fly-by: those controls do nothing before the "3"
            if k.key == KeyCode::Enter && self.screen == Screen::Race && self.controls.a_bound() {
                self.skip_fly_by(false);
            } else if k.in_race || !matches!(self.screen, Screen::Race | Screen::Loading) || (self.screen == Screen::Loading && k.key == KeyCode::Escape) {
                self.key(k.key);
            }
        }
    }

    /// A mouse wheel notch or button as a menu key (not while racing, binding or typing);
    /// the button for the pause menu stays Esc/Start. REMASTER: the modern pause menu takes no
    /// mouse input [design choice].
    pub fn menu_mouse(&mut self, k: KeyCode) {
        if self.input_seen() {
            return;
        }
        let modern_pause = matches!(self.screen, Screen::Pause | Screen::PauseGfx | Screen::PauseSfx | Screen::PauseHud | Screen::PauseControls | Screen::PauseCam) && self.settings.view.modern_menu();
        if modern_pause || self.capture.is_some() || self.editing.is_some() || matches!(self.screen, Screen::Race | Screen::Loading) {
            return;
        }
        self.key(k);
    }

    /// Give a car task of the Custom layout a control and finish capturing.
    /// CLASSIC 0x4691e4: a control a fixed task (Info, chat, the camera keys, screen shot) holds is
    /// refused (CONTROL_ERROR, the capture goes on); one another task holds swaps with this task's
    /// (CONTROL_SWITCH); otherwise CONTROL_OK. The layout in use takes it.
    fn bind(&mut self, t: usize, c: Code) {
        let mut cur = self.controls.layout().bind;
        if (CAR_TASKS..task::GAME_MENU).any(|f| cur[f] == Some(c)) {
            return self.sfx.push(Sfx::Menu(ev::CONTROL_ERROR));
        }
        // REMASTER: the pause menu's Controls tab waits for a key or a pad button, so the preset
        // becomes Custom when a key is bound, not when the wait starts
        if self.settings.view.modern_menu() && matches!(self.screen, Screen::PauseControls | Screen::Controls) && !self.controls.layout().is_custom() {
            self.copy_to_custom();
            cur = self.controls.layout().bind;
        }
        let name = self.controls.layout().name.to_ascii_lowercase();
        let o = self.settings.binds.entry(name).or_insert([None; TASKS.len()]);
        let other = (0..CAR_TASKS).chain([task::GAME_MENU]).find(|&u| u != t && cur[u] == Some(c));
        if let Some(u) = other {
            o[u] = cur[t];
        }
        o[t] = Some(c);
        self.capture = None;
        self.sync_controls();
        self.save_settings();
        self.sfx.push(Sfx::Menu(if other.is_some() { ev::CONTROL_SWITCH } else { ev::CONTROL_OK }));
    }

    /// REMASTER: the modern menu edits the Custom layout only; a preset is copied into it first [design choice].
    fn copy_to_custom(&mut self) {
        if self.controls.layout().is_custom() {
            return;
        }
        let bind = self.controls.layout().bind;
        let o = self.settings.binds.entry("custom".into()).or_insert([None; TASKS.len()]);
        o[..CAR_TASKS].copy_from_slice(&bind[..CAR_TASKS]);
        self.settings.layout = "Custom".into();
        self.sync_controls();
    }

    /// REMASTER: give a car task of the pad a control and finish capturing.
    fn bind_pad(&mut self, t: usize, p: PadIn) {
        self.settings.pad[t] = Some(p);
        self.capture = None;
        self.sync_controls();
        self.save_settings();
        self.sfx.push(Sfx::Menu(ev::SELECT));
    }

    /// The screen shows the hologram car (the modern dials, Graphics, Controls, League Load and View standings).
    fn holo_screen(&self) -> bool {
        (self.modern_dial()
            || matches!(
                self.screen,
                Screen::Graphics | Screen::Controls | Screen::Credits | Screen::Sound | Screen::Track | Screen::LeagueLoad | Screen::LeagueTable | Screen::Replays | Screen::Analysis
            ))
            && self.settings.view.modern_menu()
    }

    /// REMASTER: a dial screen drawn by `dial_modern` (Main, Single, Options, League).
    fn modern_dial(&self) -> bool {
        matches!(
            self.screen,
            Screen::Main | Screen::Single | Screen::Options | Screen::League
        ) && self.settings.view.modern_menu()
    }

    pub fn go(&mut self, s: Screen) {
        self.go_sel(s, None);
    }

    /// Whether a Classic screen is sliding out (keys are ignored).
    pub fn leaving(&self) -> bool {
        self.leave.is_some()
    }

    /// CLASSIC 0x46284c: go to `s` (starting on item `sel`). A Classic menu slides out first for as long as it had
    /// slid in (up to 150 ms); a later request while leaving replaces the target. APPROX: a screen at clock 0 switches at
    /// once (the original draws one frame at slide 256 first).
    fn go_sel(&mut self, s: Screen, sel: Option<usize>) {
        if let Some(l) = &mut self.leave {
            (l.to, l.sel) = (s, sel);
            return;
        }
        let ms = (self.t * 1000.0).min(motor_data::sprpos::SLIDE_MS as f32);
        if self.screen.slides() && !self.settings.view.modern_menu() && ms > 0.0 {
            self.leave = Some(Leave { to: s, sel, ms });
            return;
        }
        self.switch(s, sel);
    }

    /// The screen change itself (what `go` was before the slide-out).
    fn switch(&mut self, s: Screen, sel: Option<usize>) {
        self.leave = None;
        if self.screen == Screen::Graphics {
            self.gfx = None;
        }
        // REMASTER Lap Analysis (#429): only in the Glass menus and with a pair set up (`ana_from_ghost`), else the Ghost screen
        let s = if s == Screen::Analysis && (!self.settings.view.modern_menu() || self.analysis.track.is_none()) { Screen::Replays } else { s };
        if s != Screen::Analysis {
            self.analysis.job = None;
        }
        if s == Screen::Loading && self.screen != Screen::Loading {
            self.load_from = (self.screen, self.sel);
        }
        self.screen = s;
        self.t = 0.0;
        // CLASSIC: every screen init clears the slot drawer's counters (0x750770, flags 0x7507f0)
        *self.settle.borrow_mut() = slide_fx::Settle::new(self.rng.next() << 15 | self.rng.next());
        self.rolls = self.roll_targets();
        if s == Screen::Loading {
            self.frozen = false;
            self.load_facts = Some(self.load_facts());
        }
        self.sel = 0;
        self.prev_car = None;
        self.honk_at = None;
        self.snap_needle();
        if s != Screen::Sound {
            self.cd_preview = None;
        } else {
            // the counters and the cd loop switch start settled
            self.sound_page = SOUND_VOLUME;
            self.refresh_sound_devices();
            self.sound_snap = true;
            self.loop_frame = if self.settings.cd_loop { 0.0 } else { 7.0 };
        }
        if matches!(s, Screen::Track | Screen::Car) {
            self.fix_locked();
        }
        if s == Screen::Graphics {
            self.gfx = Some(GfxEdit::new(&self.settings));
        }
        // REMASTER Personal (#91): from Options on the Driver tab
        if s == Screen::Personal && self.settings.view.modern_menu() {
            (self.personal_tab, self.personal_sel, self.records_cars) = (0, [0; 5], false);
            self.paint_pick = None;
            self.sel = self.personal_sel[self.personal_tab];
        }
        if s == Screen::Controls {
            self.ctl_tasks = false;
        }
        // REMASTER Results (#94): the glass is up and the selection starts on your row
        if matches!(s, Screen::Results | Screen::LeagueResults) {
            self.results_watch = false;
        }
        if s == Screen::Results {
            self.results_tab = 0;
            self.results_scroll = 0;
            self.sel = self.race.as_ref().and_then(|r| self.results.iter().position(|x| r.sess.race.racers[x.0].human)).unwrap_or(0);
        }
        if s == Screen::Rankings {
            self.rank_defaults();
            self.load_rank_table();
        }
        if matches!(s, Screen::League | Screen::LeagueTable | Screen::LeagueLoad) {
            self.open_leagues();
        }
        if s == Screen::Replays {
            // 0x462a10: the focus starts on the first row
            self.ghost_row = 0;
            self.ghost_mode = 0;
            self.ghost_del = None;
            self.replay_msg = None;
            self.load_replays();
        }
        if s == Screen::Multi {
            self.multi_msg = None;
        }
        if s == Screen::LeagueTable {
            self.league_div = self.current_league().map_or(3, |l| l.human_division());
        }
        if s == Screen::LeagueLoad {
            self.sel = self.league;
        }
        if matches!(s, Screen::Car | Screen::Track) {
            self.pic.b = 0.0;
        }
        // CLASSIC shows the car's photo; the garage scene is for the Remaster Car screen
        // (and the modern Personal screen: the parked car behind the cards, #91)
        if s == Screen::Car || (s == Screen::Personal && self.settings.view.modern_menu()) {
            if self.settings.view.mode == RenderMode::Remaster {
                self.ensure_garage();
            }
        } else if self.holo_screen() {
            self.ensure_menu_drive();
        }
        if s == Screen::Intro {
            self.movie_queue = INTRO.iter().rev().copied().collect();
        }
        if s.is_movie() {
            self.next_movie();
        }
        if let Some(i) = sel {
            self.sel = i;
            self.snap_needle();
        }
    }

    /// REMASTER: NeoCity with the selected car driving it (`MenuDrive`), uploaded unless it is
    /// there; the lap goes on while the car screen shows the garage. Falls back to the parked
    /// garage car when the track or the drive cannot be set up.
    fn ensure_menu_drive(&mut self) {
        if self.loaded == Loaded::MenuDrive && self.menu_drive.as_ref().is_some_and(|d| d.car == self.settings.car) {
            return;
        }
        if let Err(e) = self.load_menu_drive() {
            eprintln!("menu drive: {e}");
            self.menu_drive = None;
            self.ensure_garage();
        }
    }

    fn load_menu_drive(&mut self) -> Result<(), String> {
        let neo = self.tracks.iter().find(|t| t.name.eq_ignore_ascii_case("neocity")).map(|t| t.n).ok_or("no NeoCity")?;
        let car = self.settings.car;
        let id = self.cars.get(car).ok_or("no car")?.id;
        let sc = scene::load_hologram(&self.root, neo, &[id])?;
        if !self.menu_drive.as_ref().is_some_and(|d| d.car == car) {
            // all AI (no player), no rubber band, an endless lap; at speed from the first frame
            let mut sess = Session::with_field(&self.root, neo, &[id], None, AiRow::Difficulty(Difficulty::Hard), Some(999), false, None)?;
            sess.set_wheels(&sc);
            sess.race.countdown = 0.0;
            sess.race.racers[0].ai.speed_cap = MENU_DRIVE_SPEED / 3.6;
            for _ in 0..(MENU_DRIVE_WARMUP * 60.0) as usize {
                sess.step(1.0 / 60.0, Input::default());
            }
            let road = scene::road_lines(&sc.tris, &sess.race.world);
            let mut d = MenuDrive { sess, car, road, pos: Vec3::ZERO, dir: Vec3::Z };
            d.pose(f32::INFINITY);
            self.menu_drive = Some(d);
        }
        self.pending = Some(sc);
        self.loaded = Loaded::MenuDrive;
        Ok(())
    }

    /// Load the garage scene (the track the Remaster car screen shows the cars on) unless it is:
    /// the one kept from the last visit, else the prefetched one (waiting for its worker), else
    /// built here.
    fn ensure_garage(&mut self) {
        if self.loaded == Loaded::Garage {
            return;
        }
        self.collect_garage(true);
        let loaded = match self.garage_scene.take() {
            Some(sc) => Ok(sc),
            None => {
                let ids: Vec<u32> = self.cars.iter().map(|c| c.id).collect();
                scene::load(&self.root, GARAGE_TRACK, &ids, &[])
            }
        };
        match loaded {
            Ok(sc) => {
                // the AI line start is lifted 0.3 m above the road (scene.rs)
                self.spot = (sc.start_pos - Vec3::Y * 0.3, sc.start_dir);
                self.garage_wheels = sc.wheel_hubs.clone();
                self.pending = Some(sc);
                self.loaded = Loaded::Garage;
            }
            Err(e) => eprintln!("garage: {e}"),
        }
    }

    /// REMASTER: build the garage scene on a worker while the player is on a screen that leads to
    /// the Car screen, so opening it does not stall (the generated car bodies take over a second).
    fn prefetch_garage(&mut self) {
        if self.garage_scene.is_some() || self.garage_job.is_some() || self.loaded == Loaded::Garage {
            return;
        }
        let (root, ids) = (self.root.clone(), self.cars.iter().map(|c| c.id).collect::<Vec<u32>>());
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let _ = tx.send(scene::load(&root, GARAGE_TRACK, &ids, &[]));
        });
        self.garage_job = Some(rx);
    }

    /// Take the prefetched garage scene once its worker is done (`wait`: block until it is).
    fn collect_garage(&mut self, wait: bool) {
        let Some(rx) = &self.garage_job else { return };
        let got = if wait { rx.recv().map_err(|_| mpsc::TryRecvError::Disconnected) } else { rx.try_recv() };
        match got {
            Ok(Ok(sc)) => {
                self.garage_scene = Some(sc);
                self.garage_job = None;
            }
            Ok(Err(e)) => {
                eprintln!("garage: {e}");
                self.garage_job = None;
            }
            Err(mpsc::TryRecvError::Empty) => {}
            Err(mpsc::TryRecvError::Disconnected) => self.garage_job = None,
        }
    }

    /// Upload whatever scene the current screen needs.
    pub fn prepare(&mut self, gpu: &mut Gpu) {
        // the modern Main, Graphics and Controls screens show the car from the first frame
        if self.holo_screen() {
            self.ensure_menu_drive();
        }
        // F9 to Remaster on the Car screen: the 3D car replaces the photo
        if (self.screen == Screen::Car || (self.screen == Screen::Personal && self.settings.view.modern_menu())) && self.settings.view.mode == RenderMode::Remaster {
            self.ensure_garage();
        }
        // #335 the loaded race's scene, a slice a frame behind the loading screen
        if let Some(job) = self.loading.as_mut().filter(|j| !j.uploaded) {
            if let Some(done) = &mut job.done {
                let t0 = std::time::Instant::now();
                let l = job.upload.get_or_insert_with(|| gpu.begin_load(&done.sc, done.upload.take().unwrap_or_else(|| SceneUpload::new(&done.sc))));
                let swapped = gpu.load_step(l, &done.sc, Some(UPLOAD_BUDGET));
                let _ = job.progress.set(Stage::Grid, if swapped { 1.0 } else { l.frac() });
                job.up_frames += 1;
                job.up_longest = job.up_longest.max(t0.elapsed().as_secs_f32() * 1000.0);
                if swapped {
                    job.upload = None;
                    job.uploaded = true;
                    let src = done.bake.take().unwrap_or_else(|| motorview::probes::BakeSource::new(&done.sc, &done.sess.race.world));
                    gpu.set_local_source(Some(src));
                    self.loaded = Loaded::Race;
                }
            }
        }
        if let Some(s) = self.pending.take() {
            match self.pending_upload.take() {
                Some(up) => gpu.load_prepared(&s, up),
                None => gpu.load(&s),
            }
            if let (Loaded::MenuDrive, Some(d)) = (self.loaded, &self.menu_drive) {
                gpu.load_holo_road(&d.road);
            }
            // REMASTER local light (#83): races keep what their probe volume is baked from
            if let (Loaded::Race, Some(r)) = (self.loaded, &self.race) {
                let src = self.pending_bake.take().unwrap_or_else(|| motorview::probes::BakeSource::new(&s, &r.sess.race.world));
                gpu.set_local_source(Some(src));
            }
            // REMASTER: keep the garage for the next visit
            if self.loaded == Loaded::Garage {
                self.garage_scene = Some(s);
            }
        }
        gpu.update_local_light(&self.settings.view);
        // REMASTER: the Track screen's photo into its atlas slot (14-menu-overhaul #10)
        if self.screen == Screen::Track && self.settings.view.modern_menu() {
            if let Some(rect) = self.tracks.get(self.settings.track).and_then(|t| self.assets.show_preview(t.n)) {
                gpu.update_ui(&self.assets.atlas, rect);
            }
        }
        // the movie's picture into its atlas slot (movie.rs; CLASSIC shows the 16-bit, line-doubled picture)
        if let (true, Some(m)) = (self.screen.is_movie(), &self.movie) {
            let classic = self.settings.view.mode == RenderMode::Classic;
            let key = (m.index, m.shown(), classic);
            if self.movie_shown != Some(key) {
                let (w, h) = m.picture(classic, &mut self.movie_rgba);
                if let Some(rect) = self.assets.show_movie(w, h, &self.movie_rgba) {
                    gpu.update_ui(&self.assets.atlas, rect);
                    self.movie_shown = Some(key);
                }
            }
        }
    }

    // ---------------------------------------------------------------- movies (#42, #43)

    /// Open the next movie of the queue. One that does not open is skipped (the original goes on
    /// after result 3, 0x457eef); the end of the queue is the main menu (intro) or the League dial
    /// (the season's movie, back where the results screen would have gone).
    fn next_movie(&mut self) {
        self.movie = None;
        self.movie_shown = None;
        while let Some(i) = self.movie_queue.pop() {
            match Movie::open(&self.root, i) {
                Ok(mut m) => {
                    if self.movie_hold {
                        m.hold();
                    }
                    self.movie_serial += 1;
                    self.movie = Some(m);
                    return;
                }
                Err(e) => eprintln!("movie {i}: {e}"),
            }
        }
        if self.screen == Screen::LeagueMovie {
            let to_main = std::mem::take(&mut self.final_won);
            self.league_race_over(to_main);
        } else {
            self.go(Screen::Main);
        }
    }

    /// The install has the movies.
    pub fn has_movies(&self) -> bool {
        self.has_movies
    }

    /// Shots: the season ends with `code` (1..7): the results screen with its movie queued (`--season-end`).
    pub fn preview_season_end(&mut self, code: u32) {
        self.league_outcome = Outcome::from_code(code);
        self.preview_outcome = self.league_outcome;
        self.movie_queue = self.league_outcome.and_then(Outcome::movie).into_iter().collect();
    }

    /// Shots: you start the league in division `div` (0 = the final) instead of Division 3 (`--league-div`); call it on the
    /// League screen, before the Continue.
    pub fn preview_league_div(&mut self, div: usize) {
        self.open_leagues();
        let Some(l) = self.leagues.get_mut(self.league) else { return };
        let Some(h) = l.human() else { return };
        let to = league::DIV_START[div.min(3)] + usize::from(div == 0);
        l.places.swap(h, to);
    }

    // ---------------------------------------------------------------- league

    /// You as a league driver: PlayerName, PlayerColour and the selected car in every division.
    fn me_profile(&self) -> Profile {
        let s = &self.settings;
        let name: String = s.player_name.chars().take(15).collect();
        let car = self.cars.get(s.car).map_or(String::new(), |c| c.name.clone());
        Profile { nick: name.clone(), name, picture: "Game/player.tga".into(), colour: s.player_colour, car_colours: s.car_colours, cars: std::array::from_fn(|_| car.clone()), ..Profile::default() }
    }

    /// Re-read the unlock flags (interactive runs).
    pub fn refresh_locks(&mut self) {
        if let Some(dir) = league::dir(self.persist) {
            let names: Vec<&str> = self.tracks.iter().map(|t| t.name.as_str()).collect();
            self.flagged = Some(league::flagged(&dir, &names));
        }
    }

    /// The unlock flags the lock rules read: the `.FLG` files, or every track flagged while the profile is
    /// R Pettersson / SWE (CLASSIC `0x461698`: each track's `+0x1d98` = 1 in memory, no `.FLG` written, #310).
    /// `None` = no flag source (not an interactive run), nothing locked. [I] Design choice: checked live
    /// instead of once at front-end init, so a name typed on the Personal screen counts at once and
    /// clearing it locks again.
    pub fn unlock_flags(&self) -> Option<Vec<bool>> {
        if crate::cheats::unlocks_all(&self.settings.player_name, &self.settings.team) {
            return Some(vec![true; self.tracks.len()]);
        }
        self.flagged.clone()
    }

    /// What the Track and Car screens browse and what can be picked (`league::single_locks`). A league
    /// picks its own track, so its tracks are never locked here; its car list leaves out the Serpent
    /// (`league::league_cars`) but keeps the single-race pick count (the original's is global).
    fn locks(&self) -> Locks {
        let cars = self.car_codes();
        let tracks = self.tracks.len();
        match (self.unlock_flags(), self.game_type) {
            (Some(f), GameType::League) => {
                let browse = league::league_cars(&cars);
                Locks { tracks, tracks_open: tracks, cars: browse, cars_open: league::single_locks(&self.track_codes(), &f, &cars).cars_open.min(browse) }
            }
            (_, GameType::League) => Locks::all(tracks, league::league_cars(&cars)),
            (Some(f), _) => league::single_locks(&self.track_codes(), &f, &cars),
            _ => Locks::all(tracks, cars.len()),
        }
    }

    /// The selected track is browsed but not yet unlocked.
    fn track_locked(&self) -> bool {
        self.settings.track >= self.locks().tracks_open
    }

    /// The selected car is browsed but not yet unlocked.
    fn car_locked(&self) -> bool {
        self.settings.car >= self.locks().cars_open
    }

    /// Step the selected track (`d` = +-1) over the browsed ones; the ends don't wrap (0x46c94d), and
    /// the end click plays there.
    fn step_track(&mut self, d: i32) {
        let n = self.locks().tracks.max(1);
        let t = (self.settings.track as i32 + d).clamp(0, n as i32 - 1) as usize;
        if t == self.settings.track {
            self.ev_override = Some(ev::SELECT_ERROR);
        }
        self.settings.track = t;
        self.settings.laps = self.tracks[t].laps;
    }

    /// Step the selected car over the browsed ones, clamped (0x46d6a2, 0x46d77d).
    fn step_car(&mut self, d: i32) {
        let n = self.locks().cars.max(1);
        self.prev_car = Some(self.settings.car);
        let c = (self.settings.car as i32 + d).clamp(0, n as i32 - 1) as usize;
        if c == self.settings.car {
            self.ev_override = Some(ev::SELECT_ERROR);
        }
        self.settings.car = c;
    }

    /// Leaving the Track / Car screen (or loading the settings) puts a locked choice back on the last
    /// unlocked one (0x46ca8f, 0x46d84b).
    fn fix_locked(&mut self) {
        let l = self.locks();
        if self.settings.track >= l.tracks_open && l.tracks_open > 0 {
            self.settings.track = l.tracks_open - 1;
            self.settings.laps = self.tracks[self.settings.track].laps;
        }
        if self.settings.car >= l.cars_open && l.cars_open > 0 {
            self.settings.car = l.cars_open - 1;
        }
    }

    fn track_codes(&self) -> Vec<u32> {
        self.tracks.iter().map(|t| t.division).collect()
    }

    fn car_codes(&self) -> Vec<u32> {
        self.cars.iter().map(|c| c.division).collect()
    }

    pub fn current_league(&self) -> Option<&League> {
        self.leagues.get(self.league)
    }

    /// Read the saved leagues once; with none there is always a new one (0x450204 -> 0x44f8d0).
    fn open_leagues(&mut self) {
        if self.leagues.is_empty() {
            if let Some(dir) = league::dir(self.persist) {
                self.leagues = league::load_all(&dir, &self.profiles, &self.me_profile());
            }
            // resume the league played last, not the first by file name
            if let Some(last) = league::load_last(self.persist) {
                self.league = self.leagues.iter().position(|l| l.file.eq_ignore_ascii_case(&last)).unwrap_or(self.league);
            }
        }
        if self.leagues.is_empty() {
            self.new_league(None);
        }
        self.league = self.league.min(self.leagues.len() - 1);
    }

    /// CLASSIC New league screen (0x46bb44): the name field opens with "League" typed in (0x46bc60), the league
    /// itself is made after the question that follows Enter. Nothing exists before that, so Esc leaves no trace.
    fn open_league_name(&mut self) {
        self.league_name = "League".to_string();
        self.editing = Some((2, String::new()));
        self.osk_open();
    }

    fn new_league(&mut self, name: Option<&str>) {
        // a new league starts in the ASC, the car every division allows (design choice)
        if let Some(i) = self.cars.iter().position(|c| c.name.eq_ignore_ascii_case("ASC")) {
            self.settings.car = i;
        }
        let mut l = League::create(name, &self.profiles, &self.me_profile(), &mut self.rng, league::today());
        let taken: Vec<String> = self.leagues.iter().map(|l| l.file.clone()).collect();
        l.file = league::file_stem(&l.name, &taken);
        self.leagues.push(l);
        self.league = self.leagues.len() - 1;
        self.save_league();
    }

    /// Delete league `at` (the current one from the League dial, the selected one from the REMASTER Load screen); the
    /// current league stays the same save when an earlier one goes, and the last one is replaced by a new league (0x44fff4).
    fn delete_league_at(&mut self, at: usize) {
        if at >= self.leagues.len() {
            return;
        }
        let l = self.leagues.remove(at);
        if let Some(dir) = league::dir(self.persist) {
            let _ = std::fs::remove_file(League::path(&dir, &l.file));
        }
        self.league = league::index_after_delete(self.league, at, self.leagues.len());
        if self.leagues.is_empty() {
            self.new_league(None);
        }
    }

    fn save_league(&self) {
        let (Some(dir), Some(l)) = (league::dir(self.persist), self.current_league()) else { return };
        if let Err(e) = l.save(&dir) {
            eprintln!("league {}: {e}", dir.display());
        }
        league::save_last(self.persist, &l.file);
    }

    /// The car you picked may be driven in your division (0x46d595).
    fn car_allowed(&self) -> bool {
        self.game_type != GameType::League || self.current_league().is_none_or(|l| league::car_allowed(self.cars[self.settings.car].division, l.human_division()))
    }

    /// Continue: the next race's track and laps, then the car screen.
    fn continue_league(&mut self) {
        let codes = self.track_codes();
        let Some(t) = self.current_league().and_then(|l| l.next_track(&codes)) else { return };
        // preselect the car you last drove in this league (stored per division in the .LGF)
        let last = self.current_league().and_then(|l| l.places.iter().find(|p| p.human).map(|p| p.p.cars[l.human_division()].clone()));
        if let Some(i) = last.and_then(|n| self.cars.iter().position(|c| c.name == n)) {
            self.settings.car = i;
        }
        self.game_type = GameType::League;
        self.settings.track = t;
        self.settings.laps = self.tracks[t].laps;
        self.go(Screen::Car);
    }

    /// The league race is over: points, sorting, the season result (0x453940), and save.
    fn finish_league_race(&mut self) {
        let Some(r) = &self.race else { return };
        let order = r.sess.race.standings();
        let finish: Vec<(usize, u32)> = self
            .league_field
            .iter()
            .filter_map(|&(place, slot)| order.iter().position(|&o| o == slot).map(|k| (place, k as u32 + 1)))
            .collect();
        let laps = r.sess.race.laps;
        let times = r.sess.race.racers.iter().find(|x| x.human).map(|x| (x.progress.finished_ms, x.progress.best_lap_ms));
        let codes = self.track_codes();
        let Some(l) = self.leagues.get_mut(self.league) else { return };
        if let Some((fin, lap)) = times {
            l.add_times(fin, lap);
        }
        // REMASTER Results: the division's order before the race re-sorts it
        self.league_moves = l.moved_before(&finish);
        self.league_outcome = Some(l.finish_race(&finish, laps, &codes, &mut self.rng));
        if self.preview_outcome.is_some() {
            self.league_outcome = self.preview_outcome;
        }
        self.save_league();
    }

    /// Leaving the standings: the season's movie after them (0x42aa18), else the League dial. The season end and the save
    /// stay here, not after the movie like 0x453ccc, so quitting during the 4.5 min one loses nothing [design choice].
    fn leave_league_results(&mut self) {
        self.race = None;
        // CLASSIC 0x46de12: after the final is won the front end opens on the Main menu, after any other league race on
        // the standings; REMASTER keeps the League dial [design choice]
        let to_main = self.league_outcome == Some(Outcome::Won) && !self.modern_requester();
        self.movie_queue = self.league_outcome.and_then(Outcome::movie).filter(|_| self.has_movies).into_iter().collect();
        self.close_league_results();
        if self.movie_queue.is_empty() {
            self.league_race_over(to_main);
        } else {
            self.final_won = to_main;
            self.go(Screen::LeagueMovie);
        }
    }

    /// Where the front end opens after a league race (and its season movie). CLASSIC 0x46ddf4: the standings (0x4bded8),
    /// whose Enter goes on to the car screen (0x46b5ec -> 0x4bde38) and Esc to the League dial; the Main menu after the
    /// final is won. Confirmed in the original (2026-11-07, Division 3 race 1). REMASTER goes to the League dial [design choice].
    fn league_race_over(&mut self, to_main: bool) {
        if to_main {
            self.go(Screen::Main);
        } else if self.modern_requester() {
            self.back_to_dial(Screen::League, 0);
        } else {
            self.go(Screen::LeagueTable);
            self.standings_next = true;
        }
    }

    /// A season is over: the human's score goes into the L*.MHI tables for their division
    /// (3, 2&3, Total). Design choice: the original never writes these tables
    /// (0x447028 only reads them), so they would show the defaults forever.
    fn enter_league_hiscores(&self) {
        let (Some(dir), Some(l)) = (hiscore::dir(self.persist), self.current_league()) else { return };
        let Some(me) = l.human().map(|i| &l.places[i]) else { return };
        let entry = hiscore::Entry { name: me.p.name.clone(), car: self.cars[self.settings.car].name.clone(), score: me.score, lap_ms: l.season_lap_ms, race_ms: l.season_ms, ..hiscore::Entry::default() };
        let mut kinds = vec![hiscore::Kind::LeagueTotal];
        match l.human_division() {
            3 => kinds.extend([hiscore::Kind::LeagueDiv3, hiscore::Kind::LeagueDiv23]),
            2 => kinds.push(hiscore::Kind::LeagueDiv23),
            _ => {}
        }
        for kind in kinds {
            let name = hiscore::file_name(kind, "", 0, self.settings.difficulty, false);
            let mut table = hiscore::Table::load(&self.root, Some(&dir), &name, 100);
            table.insert(table.place_score(me.score), entry.clone());
            if let Err(e) = table.save(&dir, &name) {
                eprintln!("hiscores: {e}");
            }
        }
    }

    /// Leaving the standings: at the end of a season, unlocks and the moves between divisions
    /// (0x453ccc); a won league is deleted.
    fn close_league_results(&mut self) {
        let Some(o) = self.league_outcome.take() else { return };
        if o == Outcome::Season {
            return;
        }
        let codes = self.track_codes();
        self.enter_league_hiscores();
        let Some(l) = self.leagues.get_mut(self.league) else { return };
        let unlock = l.end_season(o, &codes);
        if let Some(dir) = league::dir(self.persist) {
            for t in unlock {
                if let Err(e) = league::unlock(&dir, &self.tracks[t].name) {
                    eprintln!("league unlock: {e}");
                }
            }
        }
        self.refresh_locks();
        if o == Outcome::Won {
            // 0x453ccc deletes the file; the front end's reload then makes current the last league of the same name,
            // else the first file (0x450f3b..0x450f87), and a "Default" one when none is left (0x450e32)
            let won = self.leagues[self.league].name.clone();
            self.delete_league_at(self.league);
            // the reload lists the files in name order, as the next start does
            self.leagues.sort_by(|a, b| a.file.cmp(&b.file));
            let names: Vec<&str> = self.leagues.iter().map(|l| l.name.as_str()).collect();
            self.league = league::current_after_win(&names, &won);
            if let Some(l) = self.current_league() {
                league::save_last(self.persist, &l.file);
            }
        } else {
            self.save_league();
        }
    }

    /// #335 the loading screen's frame: the first starts the worker (the screen has been drawn once),
    /// the later ones wait for it; the race starts when it is done.
    fn poll_load(&mut self) {
        let beat = self.settings.view.modern_menu();
        let Some(job) = &mut self.loading else {
            let planned = match self.watch_next.take() {
                Some(m) => self.watch_plan(m),
                None => self.race_plan(),
            };
            match planned {
                Ok((plan, finish)) => {
                    let progress = std::sync::Arc::new(Progress::default());
                    let (tx, rx) = mpsc::channel();
                    let worker = progress.clone();
                    std::thread::spawn(move || {
                        let _ = tx.send(plan.run(&worker));
                    });
                    self.loading = Some(LoadJob { rx, progress, finish, started: std::time::Instant::now(), frames: 0, done: None, worker_ms: 0.0, upload: None, uploaded: false, up_frames: 0, up_longest: 0.0, ready_at: None });
                }
                Err(e) => {
                    eprintln!("race: {e}");
                    if self.attract {
                        self.end_attract(false);
                    }
                    self.go(Screen::Main);
                }
            }
            return;
        };
        job.frames += 1;
        if job.done.is_none() {
            let got = match job.rx.try_recv() {
                Err(mpsc::TryRecvError::Empty) => return,
                Err(mpsc::TryRecvError::Disconnected) => Err("the loading thread stopped".to_string()),
                Ok(r) => r,
            };
            match got {
                // `prepare` uploads the scene from the next frame on
                Ok(load) => {
                    job.done = Some(load);
                    job.worker_ms = job.started.elapsed().as_secs_f32() * 1000.0;
                }
                Err(e) => {
                    self.loading = None;
                    eprintln!("race: {e}");
                    if self.attract {
                        self.end_attract(false);
                    }
                    self.go(Screen::Main);
                }
            }
            return;
        }
        if !job.uploaded {
            return;
        }
        // REMASTER: the finished bar and READY stay up a moment (the load itself is about a second)
        if beat && job.ready_at.get_or_insert_with(|| (std::time::Instant::now(), job.progress.timings())).0.elapsed().as_secs_f32() < READY_BEAT {
            return;
        }
        let Some(mut job) = self.loading.take() else { return };
        let Some(load) = job.done.take() else { return };
        self.finish_race(job.finish, load, true);
        println!(
            "load: {:.0} ms on the worker, upload {} frames (longest {:.1} ms), {:.0} ms in all, {} frames drawn meanwhile; {}{}",
            job.worker_ms,
            job.up_frames,
            job.up_longest,
            job.started.elapsed().as_secs_f32() * 1000.0,
            job.frames,
            job.ready_at.as_ref().map_or_else(|| job.progress.timings(), |r| r.1.clone()),
            job.ready_at.as_ref().map_or_else(String::new, |r| format!(", READY {:.0} ms", r.0.elapsed().as_secs_f32() * 1000.0))
        );
        self.go(Screen::Race);
    }

    /// Esc on the loading screen: drop the load (the worker stops at its next stage report) and go
    /// back to where it was started from. Nothing of the race has been put into the game yet.
    fn cancel_load(&mut self) {
        if let Some(job) = self.loading.take() {
            job.progress.cancel();
            println!("load: {CANCELLED} after {:.0} ms", job.started.elapsed().as_secs_f32() * 1000.0);
        }
        self.watch_next = None;
        let (screen, sel) = self.load_from;
        self.go(screen);
        self.sel = sel;
        self.snap_needle();
        self.sfx.push(Sfx::Menu(ev::ESCAPE));
    }

    /// The loading screen's stage and the share of the bar done (0..1), None when no load runs.
    pub fn load_progress(&self) -> Option<(Stage, f32)> {
        self.loading.as_ref().map(|j| (j.progress.stage(), j.progress.share()))
    }

    /// The race is loaded and on the GPU: the loading screen shows READY until it starts.
    pub fn load_ready(&self) -> bool {
        self.loading.as_ref().is_some_and(|j| j.uploaded)
    }

    /// #335 the race's scene is being uploaded (the headless `--cancel-upload` check).
    pub fn load_uploading(&self) -> bool {
        self.loading.as_ref().is_some_and(|j| j.upload.is_some())
    }

    /// Load the race (blocking; the headless shots): the plan, its worker's part run here, the finish.
    pub fn start_race(&mut self) -> Result<(), String> {
        let (plan, fin) = self.race_plan()?;
        let load = plan.run(&Progress::default())?;
        self.finish_race(fin, load, false);
        Ok(())
    }

    /// The cars' names and ids, for a worker that maps a recording's names (`DemoPlan`).
    fn car_table(&self) -> Vec<(String, u32)> {
        self.cars.iter().map(|c| (c.name.clone(), c.id)).collect()
    }

    /// #335 the main thread's part of loading a race: the field (league, single race draw), the
    /// player's slot and paint. Everything the worker needs goes into the `LoadPlan`, what the
    /// finish needs into the `Finish`.
    fn race_plan(&mut self) -> Result<(LoadPlan, Finish), String> {
        let car = self.cars[self.settings.car].id;
        // Ghost mode (0x41bd18): the track, laps and CatchUp are the recording's; a track that is not
        // installed or not unlocked yet is refused with the original's messages
        let ghost_demo = self.ghost_demo.clone().filter(|_| self.game_type == GameType::GhostMode);
        let (track_idx, laps_set, catchup) = match &ghost_demo {
            Some(m) => (self.ghost_track(m)?, m.header.laps, m.header.catchup()),
            // CLASSIC 0x4580aa: 25 laps, CatchUp off, on the drawn track
            None if self.attract => (self.attract_track, ATTRACT_LAPS, false),
            None => (self.settings.track, self.settings.laps, self.settings.catchup),
        };
        let tr = &self.tracks[track_idx];
        // League (0x45193c): your division in grid order, each AI in its car for the division
        self.league_field.clear();
        let mut drivers = None;
        // the demo runs on Hard (0x4581a9: difficulty 2)
        let mut row = AiRow::Difficulty(if self.attract { Difficulty::Hard } else { self.settings.difficulty });
        let mut human_slot = None;
        if let (GameType::League, Some(l)) = (self.game_type, self.leagues.get_mut(self.league)) {
            // your place drives the car you picked (the original keeps the one from creation)
            if let Some(h) = l.human() {
                let d = l.human_division();
                l.places[h].p.cars[d] = self.cars[self.settings.car].name.clone();
            }
        }
        if let (GameType::League, Some(l)) = (self.game_type, self.leagues.get(self.league)) {
            let d = l.human_division();
            self.league_field = l.field();
            let n = self.league_field.len();
            let mut ids = vec![car; n];
            let mut ds = vec![motor_sim::Driver::default(); n];
            for &(place, slot) in &self.league_field {
                let pl = &l.places[place];
                if pl.human {
                    human_slot = Some(slot);
                } else {
                    ids[slot] = self.cars.iter().find(|c| c.name.eq_ignore_ascii_case(&pl.p.cars[d])).map_or(car, |c| c.id);
                }
                ds[slot] = pl.p.driver();
            }
            drivers = Some((ids, ds));
            row = AiRow::Division(d as u32);
        }
        let is_league = drivers.is_some();
        // Single race (0x46e110): 7 random League drivers in the cars of your tier, you last. Time
        // Attack: you alone (0x458568). A restart keeps the field (it never reloads), a new race draws.
        if drivers.is_none() && !matches!(self.game_type, GameType::TimeAttack | GameType::GhostMode) && !self.profiles.is_empty() {
            let d = league::single_car_division(league::tier(self.cars[self.settings.car].division));
            let pick = league::single_field(self.profiles.len(), GRID_SIZE, &mut self.rng);
            let mut ids: Vec<u32> = pick.iter().map(|&p| self.cars.iter().find(|c| c.name.eq_ignore_ascii_case(&self.profiles[p].cars[d])).map_or(car, |c| c.id)).collect();
            let mut ds: Vec<_> = pick.iter().map(|&p| self.profiles[p].driver()).collect();
            ids.push(car);
            ds.push(motor_sim::Driver::default());
            drivers = Some((ids, ds));
        }
        let ids = match &drivers {
            Some((ids, _)) => ids.clone(),
            None if matches!(self.game_type, GameType::TimeAttack | GameType::GhostMode) => vec![car],
            None => lineup(car),
        };
        let player = if self.ai_demo() { None } else { Some(human_slot.unwrap_or(ids.len() - 1)) };
        let mut paint = Vec::new();
        // REMASTER (#136, #351): your finishes go with your colours (the AI's and the recordings' paint is gloss)
        let mine = scene::CarPaint { colours: self.settings.car_colours, finish: self.settings.car_finish.map(PaintFinish::scene) };
        paint.extend(player.filter(|_| self.settings.custom_car_colours).map(|p| (p, mine)));
        let rules = if self.game_type == GameType::Knockout { Rules::Elimination } else { Rules::Normal };
        // CHEAT lemmy / ace (#316), ramlosa / h2o (#317) (0x439e84): the whole scene is baked with the cheat's brightness; tribute to
        // tron (#320, 0x48e964): its triangles carry their outline codes. Your own race only; Watch shows the normal look
        let name_cheat = player.and_then(|_| crate::cheats::NameCheat::of(&self.settings.player_name, &self.settings.team));
        let cheat = scene::SceneCheat { brightness: motor_sim::cheats::brightness(name_cheat), tron: motor_sim::cheats::tron(name_cheat) };
        let demo = ghost_demo.map(|mde| DemoPlan { mde, first_mesh: ids.len(), skip: None, cars: self.car_table(), fallback: car });
        let plan = LoadPlan { root: self.root.clone(), track: tr.n, ids: ids.clone(), player, row, laps: laps_set, catchup, drivers: drivers.map(|d| d.1), paint, driver_paint: true, demo, car_id: self.cars.get(self.settings.car).map_or(1, |c| c.id), engine_sound: self.settings.engine_sound, cheat };
        let fin = Finish::Race(RaceFinish { track_idx, track_n: tr.n, track_name: tr.name.clone(), car, ids, player, human_slot, is_league, row, catchup, rules });
        Ok((plan, fin))
    }

    /// The main thread's part after the load: the player's settings, recorders, ghost, records, `RaceState`.
    /// `on_gpu`: `prepare` has uploaded the scene already (the threaded load), else it does next.
    fn finish_race(&mut self, fin: Finish, load: RaceLoad, on_gpu: bool) {
        let RaceLoad { mut sess, sc, demo, bake, upload, mut sounds } = load;
        if !on_gpu {
            self.pending = Some(sc);
            self.pending_bake = bake;
            self.pending_upload = upload;
        }
        self.loaded = Loaded::Race;
        self.last_track = Some(match &fin {
            Finish::Race(f) => f.track_idx,
            Finish::Watch(f) => f.track_idx,
        });
        match fin {
            Finish::Race(RaceFinish { track_idx, track_n, track_name, car, ids, player, human_slot, is_league, row, catchup, rules }) => {
                if rules != Rules::Normal {
                    sess.race.set_rules(rules);
                }
                if let Some(h) = human_slot.filter(|_| player.is_none()) {
                    sess.focus = h; // demo: watch the car of your place
                }
                let cam = Camera::looking(Vec3::ZERO, Vec3::NEG_Z);
                if let Some(p) = player {
                    let s = &self.settings;
                    // the original's name cheat, worked out once per race like 0x46f774 (08 §8.14); buzz aldrin / nasa also sets the gravity (#315)
                    sess.race.set_name_cheat(crate::cheats::NameCheat::of(&s.player_name, &s.team));
                    if let Some(c) = sess.race.name_cheat {
                        eprintln!("name cheat {c:?}");
                    }
                    let r = &mut sess.race.racers[p];
                    r.car.manual = s.manual;
                    // the personal options: nick = PlayerName, the team after the name (design choice)
                    r.driver.nick = s.player_name.clone();
                    r.driver.name = if s.team.is_empty() { s.player_name.clone() } else { format!("{} ({})", s.player_name, s.team) };
                    r.driver.colour = s.player_colour;
                    // CHEAT avenger / zx (0x412aa4, #322): not in the flag word, the minimap dot only
                    r.driver.ninja_marker = crate::cheats::ninja_marker(&s.player_name, &s.team);
                    if r.driver.ninja_marker {
                        eprintln!("ninja marker");
                    }
                }
                if player.is_some() {
                    sess.cam = self.settings.cam.mode(!sess.race.tv_camera.is_empty());
                }
                let laps = sess.race.racers[0].progress.laps;
                // Record race: the setup, then one record per stepped frame (drive); leagues are left
                // out because their field comes from the league's profiles
                let ta_ghost = self.game_type == GameType::TimeAttack && self.settings.ghost_car && !is_league && player.is_some();
                // design choice (#374): the fixed physics tick; the recording names it (`tick_hz`), so its frames replay to the same race
                sess.race.set_fixed_tick(Some(motor_sim::session::PHYSICS_HZ.get()));
                let recording = player.filter(|_| self.save_mhr && self.settings.record_race && !is_league).map(|p| {
                    Recording::new(Header {
                        track: track_n,
                        laps,
                        catchup,
                        rules,
                        manual: self.settings.manual,
                        row,
                        player: Some(p),
                        cars: ids.clone(),
                        track_name: track_name.clone(),
                        nick: self.settings.player_name.clone(),
                        car_name: self.cars[self.settings.car].name.clone(),
                        drivers: if is_league || ids.len() < 2 { Vec::new() } else { sess.race.racers.iter().map(|r| if r.human { String::new() } else { r.driver.nick.clone() }).collect() },
                        tick_hz: sess.race.fixed_hz(),
                        race_ms: None,
                        best_lap_ms: None,
                    })
                });
                // Record race writes the original's `.MDE` (demo.rs): the whole field, every 62.5 ms of race time
                let demo_rec = player.filter(|_| self.settings.record_race && !is_league).map(|p| DemoRecorder::new(self.demo_header(&sess, &track_name, laps, catchup, p)));
                // Time Attack ghost (design choice): the best lap on this track and car, saved as it is driven
                let ghost = ta_ghost.then(|| self.ghost_override.clone()).flatten().or_else(|| {
                    let path = hiscore::ghost_path(self.persist, &track_name, car).filter(|_| ta_ghost)?;
                    let (g, id) = GhostTrack::load(&path).map_err(|e| eprintln!("ghost {}: {e}", path.display())).ok()?;
                    (id == car).then_some(g)
                });
                let records = hiscore::Records::load(hiscore::dir(self.persist).as_deref(), &track_name, laps);
                self.race = Some(RaceState { sess, sounds: sounds.take(), track: track_idx, finished_for: None, lap_flash: None, split: None, sector_flash: None, last_lap: 1, last_cp: None, cam, records, ta_gaps: [None; 3], prox: Vec::new(), map_zoom: crate::hud::MAP_ZOOM_MIN, eggs: Default::default(), rec_laps: 0, rec_finished: false, recording, notice: demo.is_some().then_some("Ghost play started!").or(demo_rec.is_some().then_some("Race recording started!")).map(|s| (4.0, s.to_string())), demo, demo_rec, demo_over: false, watch: None, pb: None, pending_respawn: None, pending_skip: false, playback: None, ghost, lap_rec: ta_ghost.then(LapRecorder::default), ghost_laps: 0, run: stats::Run::default() });
                self.steer = SteerKeys::default();
        self.pedals = Pedals::default();
                self.controls.reset();
            }
            Finish::Watch(WatchFinish { track_idx, track_name, block, name, laps }) => {
                {
                    let r = &mut sess.race.racers[0];
                    r.remote = true;
                    r.driver.nick = name.clone();
                    r.driver.name = name;
                }
                sess.cam = self.settings.cam.mode(!sess.race.tv_camera.is_empty());
                self.game_type = GameType::GhostMode;
                let cam = Camera::looking(Vec3::ZERO, Vec3::NEG_Z);
                let records = hiscore::Records::load(None, &track_name, laps);
                // REMASTER (#346): the playback controls of the Race line HUD
                let pb = demo.as_ref().map(|d| Playback::new(&d.mde, self.tracks[track_idx].profile.as_ref()));
                // design choice (#374): demo play on the fixed physics tick (the recorded car is posed at the view time)
                sess.race.set_fixed_tick(Some(motor_sim::session::PHYSICS_HZ.get()));
                self.race = Some(RaceState { sess, sounds: sounds.take(), track: track_idx, finished_for: None, lap_flash: None, split: None, sector_flash: None, last_lap: 1, last_cp: None, cam, records, ta_gaps: [None; 3], prox: Vec::new(), map_zoom: crate::hud::MAP_ZOOM_MIN, eggs: Default::default(), rec_laps: 0, rec_finished: false, playback: None, recording: None, demo, demo_rec: None, demo_over: false, watch: Some(block), pb, notice: Some((4.0, "Demo play started!".into())), pending_respawn: None, pending_skip: false, ghost: None, lap_rec: None, ghost_laps: 0, run: stats::Run::default() });
                self.steer = SteerKeys::default();
        self.pedals = Pedals::default();
                self.controls.reset();
            }
        }
    }


    /// Back to a dial with this item selected.
    fn back_to_dial(&mut self, s: Screen, item: usize) {
        self.go_sel(s, Some(item));
    }

    /// Items on the current dial: six on Single (with Knockout), five on the others.
    fn dial_stops(&self) -> usize {
        if self.screen == Screen::Single { SINGLE_ITEMS.len() } else { 5 }
    }

    /// Laps the Track screen shows: Knockout's are fixed.
    fn shown_laps(&self) -> u32 {
        if self.game_type == GameType::Knockout { KNOCKOUT_LAPS } else { self.settings.laps }
    }

    /// REMASTER modern dial: angle (radians) of the selected item's stop.
    fn needle_target(&self) -> f32 {
        menu_modern::main_needle(self.sel, self.dial_stops())
    }

    /// The dial screen the current screen draws its sprites from (the Main one elsewhere).
    fn dial_layout(&self) -> &'static DialLayout {
        match self.screen {
            Screen::Single => &SINGLE_DIAL,
            Screen::League => &LEAGUE_DIAL,
            Screen::Options => &OPTIONS_DIAL,
            _ => &MAIN_DIAL,
        }
    }

    /// CLASSIC 0x463538 / 0x46a300 / 0x46aacc / 0x4668a8: the needle angle (4096ths of a turn) of the
    /// selected item, `8 x + y` of sprpos slot 0x18 + item, read through the sliding accessors
    /// (0x4635a8), so while a screen slides in the needle chases a moving target [O]. Single's sixth
    /// stop (Knockout, ours) is halfway between time attack's and ghost mode's.
    fn needle_target_q(&self) -> f32 {
        let (set, sp) = (self.dial_layout().set, self.sp());
        let at = |k: usize| (sp.x(set, 0x18 + k) * 8 + sp.y(set, 0x18 + k)) as f32;
        let stop = if self.screen == Screen::Single { [0.0, 1.0, 2.0, 3.0, 3.5, 4.0][self.sel.min(5)] } else { self.sel.min(4) as f32 };
        let i = stop.floor() as usize;
        at(i) + (at((i + 1).min(4)) - at(i)) * stop.fract()
    }

    /// CLASSIC 0x4627e0: the slide factor (256 = the start positions, 0 = settled) from the screen's
    /// clock `t` (0 on entry, `go`). The Glass menus do not slide (0).
    fn slide(&self) -> i32 {
        if self.settings.view.modern_menu() {
            return 0;
        }
        motor_data::sprpos::slide_factor(self.leave.map_or(self.t * 1000.0, |l| l.ms) as u32)
    }

    /// `Data/sprpos.dta` as the Classic screens draw it this frame (`slide`).
    fn sp(&self) -> Slid<'_> {
        self.sprpos.slid(self.slide())
    }

    /// Put the needle on the selected item at once (screen entry; 0x481d40 does the same).
    fn snap_needle(&mut self) {
        self.needle = self.needle_target();
        self.needle_q = self.needle_target_q();
    }

    /// The CD track race track `i` plays: the player's assignment on the Sound screen, else the
    /// .trk file's CDTrack.
    pub fn race_cd_track(&self, i: usize) -> u32 {
        let t = &self.tracks[i];
        self.settings.track_cd.get(&t.n).copied().unwrap_or(t.cd_track)
    }

    /// The dial's volume (percent) of a Sound screen item: sound effects, cd music.
    fn sound_volume(&mut self, item: usize) -> &mut u32 {
        if item == 0 { &mut self.settings.vol_sfx } else { &mut self.settings.vol_music }
    }

    /// CLASSIC 0x469c18: the Sound screen's keys. Up / Down stop at the ends and scroll the table
    /// before leaving it; Left / Right turn the dials (a tap = 5 %, a held key = `sound_hold`),
    /// switch cd loop (Left off, Right on, Enter toggles) and pick the CD track of the table's
    /// race track (2 .. the last), Enter plays it. Esc leaves; Enter elsewhere is an error click.
    fn sound_key(&mut self, up: bool, down: bool, left: bool, right: bool, enter: bool, back: bool) {
        if up {
            if self.sel == SOUND_DEVICE {
                self.ev_override = Some(ev::CURSOR_END);
            } else if self.sel == SOUND_TABLE && self.sound_list > 0 {
                self.sound_list -= 1;
                self.sound_snap = true;
                self.ev_override = Some(ev::CURSOR_UP);
            } else if self.sel > 0 {
                self.sel -= 1;
                self.ev_override = Some(ev::CURSOR_UP);
            } else {
                // #260: above the first dial, the sound device line
                self.sel = SOUND_DEVICE;
                self.ev_override = Some(ev::CURSOR_UP);
            }
        }
        if down {
            if self.sel == SOUND_DEVICE {
                self.sel = 0;
                self.ev_override = Some(ev::CURSOR_DOWN);
            } else if self.sel == SOUND_TABLE && self.sound_list + 1 < self.tracks.len() {
                self.sound_list += 1;
                self.sound_snap = true;
                self.ev_override = Some(ev::CURSOR_DOWN);
            } else if self.sel < SOUND_TABLE {
                self.sel += 1;
                self.ev_override = Some(ev::CURSOR_DOWN);
            } else {
                self.ev_override = Some(ev::CURSOR_END);
            }
        }
        let d: i32 = if left { -1 } else { 1 };
        match self.sel {
            // a key repeat while the key is held is `sound_hold`'s
            0..=1 if (left || right) && !(if left { self.arrows_held.0 } else { self.arrows_held.1 }) => {
                self.turn_dial(self.sel, d as f32 * 5.0);
            }
            SOUND_LOOP if left || right || enter => {
                let want = if enter { !self.settings.cd_loop } else { right };
                self.ev_override = Some(if want == self.settings.cd_loop { ev::SWITCH_SAME } else if want { ev::SWITCH_ON } else { ev::SWITCH_OFF });
                self.settings.cd_loop = want;
            }
            SOUND_DEVICE if left || right => self.step_sound_device(left),
            SOUND_TABLE if left || right => self.sound_pick_track(left),
            SOUND_TABLE if enter => {
                // 0x469ee0..: plays the song unless the CD volume is 0
                self.ev_override = Some(ev::SELECT);
                if self.settings.vol_music > 0 {
                    self.cd_preview = Some(self.race_cd_track(self.sound_list));
                }
            }
            _ => {}
        }
        if back {
            self.cd_preview = None;
            self.go_sel(Screen::Options, Some(3)); // sound options
            self.ev_override = None;
        }
    }

    /// The Sound screen table's Left / Right: the CD track of the table's race track one step down / up
    /// (2 .. the last, or the tracks found in `Music/`), saved in `track_cd` only while it differs from the
    /// .trk's own. Sets the cursor sound of the step or of its limit. Shared by the Classic table and the
    /// Remaster Songs tab.
    fn sound_pick_track(&mut self, left: bool) {
        let Some(t) = self.tracks.get(self.sound_list) else { return };
        let (n, default) = (t.n, t.cd_track);
        let d: i32 = if left { -1 } else { 1 };
        let cur = self.race_cd_track(self.sound_list);
        let new = if self.cd_tracks.is_empty() { (cur as i32 + d).clamp(2, CD_LAST_TRACK as i32) as u32 } else { step_cd_track(cur, &self.cd_tracks, d) };
        self.ev_override = Some(match (new == cur, left) {
            (false, true) => ev::SONG_DOWN,
            (false, false) => ev::SONG_UP,
            (true, true) => ev::SONG_DOWN_LIMIT,
            (true, false) => ev::SONG_UP_LIMIT,
        });
        if new == default {
            self.settings.track_cd.remove(&n);
        } else {
            self.settings.track_cd.insert(n, new);
        }
    }

    /// REMASTER Sound screen keys of the Songs tab (#259): Up / Down move along the table and stop at its ends
    /// (a preview of another row ends), Left / Right pick the row's CD track (a running preview follows it),
    /// Enter starts or stops the preview, Esc leaves.
    fn sound_songs_key(&mut self, up: bool, down: bool, left: bool, right: bool, enter: bool, back: bool) {
        let last = self.tracks.len().saturating_sub(1);
        if up || down {
            let to = if up { self.sound_list.saturating_sub(1) } else { (self.sound_list + 1).min(last) };
            self.ev_override = Some(if to == self.sound_list { ev::CURSOR_END } else if up { ev::CURSOR_UP } else { ev::CURSOR_DOWN });
            if to != self.sound_list {
                self.sound_list = to;
                self.sound_stop_preview();
            }
        } else if left || right {
            self.sound_pick_track(left);
            if self.cd_preview.is_some() {
                self.sound_start_preview();
            }
        } else if enter {
            if self.cd_preview.is_some() {
                self.ev_override = Some(ev::SELECT);
                self.sound_stop_preview();
            } else if self.sound_start_preview() {
                self.ev_override = Some(ev::SELECT);
            } else {
                self.ev_override = Some(ev::SELECT_ERROR);
            }
        }
        if back {
            self.sound_stop_preview();
            self.go_sel(Screen::Options, Some(3)); // sound options
            self.ev_override = None;
        }
    }

    /// Play the song of the table's row (silent CD music or a CD volume of 0 plays nothing: false).
    fn sound_start_preview(&mut self) -> bool {
        if !self.settings.music || self.settings.vol_music == 0 || self.tracks.is_empty() {
            self.cd_preview = None;
            return false;
        }
        self.cd_preview = Some(self.race_cd_track(self.sound_list));
        self.preview_t = 0.0;
        true
    }

    fn sound_stop_preview(&mut self) {
        self.cd_preview = None;
        self.preview_t = 0.0;
    }

    /// REMASTER Sound screen: Q / E (LB / RB) switch the tab; a preview ends with the Songs tab.
    fn sound_tab_step(&mut self, next: bool) {
        let to = if next { SOUND_SONGS } else { SOUND_VOLUME };
        if to == self.sound_page {
            return;
        }
        self.sound_page = to;
        if to == SOUND_VOLUME {
            self.refresh_sound_devices();
        }
        self.sound_stop_preview();
        self.ev_override = Some(ev::SELECT);
    }

    /// Read the audio outputs again (#260): each time the Sound screen or its Volume tab opens, not while it is
    /// open (no hot-plug watching). A headless shot's fixed list wins; its first entry stands for the default.
    fn refresh_sound_devices(&mut self) {
        match &self.sound_devices_fixed {
            Some(list) => {
                self.sound_devices = list.clone();
                self.sound_default = list.first().map(|d| d.name.clone());
            }
            None => {
                self.sound_devices = motor_audio::output_devices();
                self.sound_default = motor_audio::default_output_name();
            }
        }
    }

    /// Headless shots (`--sound-devices`): this list of outputs (id = name) instead of the machine's.
    pub fn preview_sound_devices(&mut self, names: &[String], pick: Option<&str>) {
        let list: Vec<_> = names.iter().map(|n| motor_audio::OutputDevice { id: n.clone(), name: n.clone() }).collect();
        self.sound_silent = list.is_empty();
        self.sound_devices_fixed = Some(list);
        self.refresh_sound_devices();
        self.settings.sound_device = pick.map(|n| motor_audio::OutputDevice { id: n.to_string(), name: n.to_string() });
    }

    /// The Sound device row's choice (#260): 0 = "System default", 1.. = `sound_devices` in order; None while the
    /// picked output is not in the list (not connected) and there is a list to pick from.
    fn sound_device_choice(&self) -> Option<usize> {
        match &self.settings.sound_device {
            None => Some(0),
            Some(d) => self.sound_devices.iter().position(|o| o.id == d.id).map(|i| i + 1),
        }
    }

    /// The picked output cannot be used now: gone from the list, or listed but it would not open (the default plays).
    fn sound_device_unavailable(&self) -> bool {
        self.settings.sound_device.is_some() && (self.sound_device_choice().is_none() || self.sound_device_failed)
    }

    /// What the Sound device choice shows (#260): the Classic line and the Remaster row and info card.
    fn sound_device_view(&self) -> DeviceView {
        let list = &self.sound_devices;
        let (choice, unavailable) = (self.sound_device_choice(), self.sound_device_unavailable());
        let none = list.is_empty() && self.sound_silent;
        let pick = self.settings.sound_device.as_ref();
        let text = match (pick, choice) {
            _ if none => "No output device".to_string(),
            (None, _) => "System default".to_string(),
            (Some(d), None) => d.name.clone(),
            (Some(_), Some(c)) => list[c - 1].name.clone(),
        };
        let (left_end, right_end) = match (none, choice) {
            (true, _) => (true, true),
            (false, None) => (true, false),
            (false, Some(c)) => (c == 0, c >= list.len()),
        };
        let mut entries = Vec::new();
        if !none {
            let tag = |lit: bool| if lit && unavailable { Some(DeviceTag::Unavailable) } else { None };
            let default_tag = if unavailable { Some(DeviceTag::InUse) } else { None };
            entries.push(DeviceEntry { name: "System default".into(), sub: self.sound_default.clone(), lit: choice == Some(0), tag: default_tag });
            for (i, o) in list.iter().enumerate() {
                entries.push(DeviceEntry { name: o.name.clone(), sub: None, lit: choice == Some(i + 1), tag: tag(choice == Some(i + 1)) });
            }
            if let (Some(d), None) = (pick, choice) {
                entries.push(DeviceEntry { name: d.name.clone(), sub: None, lit: true, tag: Some(DeviceTag::Unavailable) });
            }
        }
        DeviceView { text, left_end, right_end, unavailable: unavailable && !none, none, entries }
    }

    /// Left / Right on the Sound device row: one step along "System default" and the outputs found (no wrap). A pick that
    /// is not connected sits before "System default": Right leaves it, Left has nowhere to go. The audio follows on the
    /// next frame (`GameAudio::sync`), so the click of this step is heard on the new output. Sounds: the Classic line
    /// uses the song table's step and limit events, the Remaster row the generic arrow click and the cursor's end sound.
    fn step_sound_device(&mut self, left: bool) {
        let classic = !self.settings.view.modern_menu();
        let cur = self.sound_device_choice();
        let to = step_device_choice(cur, if left { -1 } else { 1 }, self.sound_devices.len());
        if to == cur {
            self.ev_override = Some(match (classic, left) {
                (false, _) => ev::CURSOR_END,
                (true, true) => ev::SONG_DOWN_LIMIT,
                (true, false) => ev::SONG_UP_LIMIT,
            });
            return;
        }
        if classic {
            self.ev_override = Some(if left { ev::SONG_DOWN } else { ev::SONG_UP });
        }
        self.settings.sound_device = to.filter(|&c| c > 0).map(|c| self.sound_devices[c - 1].clone());
    }

    /// Turn a Sound screen dial by `pct` percent (limits 0 and 100), with the original's tick.
    fn turn_dial(&mut self, item: usize, pct: f32) {
        let v = self.sound_volume(item);
        let old = *v;
        *v = (old as f32 + pct).round().clamp(0.0, 100.0) as u32;
        let new = *v;
        self.ev_override = Some(match new.cmp(&old) {
            std::cmp::Ordering::Greater => ev::VOLUME_UP,
            std::cmp::Ordering::Less => ev::VOLUME_DOWN,
            std::cmp::Ordering::Equal => ev::VOLUME_LIMIT,
        });
    }

    /// CLASSIC 0x469c18 per frame: Left / Right held on a dial turn it smoothly; the settings are
    /// saved when the key is let go. Remembers the held keys for `sound_key`'s repeats.
    fn sound_hold(&mut self, dt: f32, keys: &HashSet<KeyCode>) {
        let held = (keys.contains(&KeyCode::ArrowLeft), keys.contains(&KeyCode::ArrowRight));
        let (l, r) = held;
        let was = self.arrows_held;
        self.arrows_held = held;
        if self.sel < SOUND_LOOP && l != r {
            let shift = keys.contains(&KeyCode::ShiftLeft) || keys.contains(&KeyCode::ShiftRight);
            let ctrl = keys.contains(&KeyCode::ControlLeft) || keys.contains(&KeyCode::ControlRight);
            let mult = if shift { 2.0 } else if ctrl { 4.0 } else { 1.0 };
            self.dial_carry += if r { 1.0 } else { -1.0 } * DIAL_TURN_RATE * mult * dt;
            let whole = self.dial_carry.trunc();
            if whole != 0.0 {
                self.dial_carry -= whole;
                let before = *self.sound_volume(self.sel);
                self.turn_dial(self.sel, whole);
                // the original ticks every frame; a tick per 5 % is about as many
                if *self.sound_volume(self.sel) / 5 != before / 5 {
                    if let Some(e) = self.ev_override.take() {
                        self.sfx.push(Sfx::Menu(e));
                    }
                } else {
                    self.ev_override = None;
                }
            }
        } else {
            self.dial_carry = 0.0;
        }
        if was != held && (was.0 || was.1) && !(l || r) {
            self.save_settings();
        }
    }

    /// CLASSIC 0x469668 / 0x46052c per frame: the `onoff` frame of cd loop steps 30/s towards on
    /// (0) or off (7), and the table's digit counters close on their values (all at once after
    /// the list scrolled).
    fn sound_step(&mut self, dt: f32) {
        let want = if self.settings.cd_loop { 0.0 } else { 7.0 };
        let step = DIFF_ROLL_RATE * dt;
        self.loop_frame += (want - self.loop_frame).clamp(-step, step);
        self.sound_list = self.sound_list.min(self.tracks.len().saturating_sub(1));
        for role in 0..3 {
            let Some(i) = (self.sound_list + role).checked_sub(1).filter(|&i| i < self.tracks.len()) else { continue };
            for (col, value) in self.sound_values(i).into_iter().enumerate() {
                for k in 0..2 {
                    let target = (value / 10u32.pow(k as u32) % 10) as f32;
                    let pos = &mut self.sound_roll[(role * 3 + col) * 2 + k];
                    let mut d = target - pos.rem_euclid(10.0);
                    if d > 5.0 {
                        d -= 10.0;
                    } else if d < -5.0 {
                        d += 10.0;
                    }
                    *pos = if self.sound_snap || d.abs() < 0.1 { target } else { (*pos + d * (dt * COUNTER_RATE).min(1.0)).rem_euclid(10.0) };
                }
            }
        }
        self.sound_snap = false;
    }

    /// CD track, minutes and seconds the table shows for race track `i` (0x469668).
    fn sound_values(&self, i: usize) -> [u32; 3] {
        let cd = self.race_cd_track(i);
        let len = self.cd_lengths.get(&cd).copied().unwrap_or(0);
        [cd % 100, len / 60 % 100, len % 60]
    }

    fn cycle(sel: &mut usize, n: usize, fwd: bool) {
        *sel = if fwd { (*sel + 1) % n } else { (*sel + n - 1) % n };
    }

    /// Menu state compared before/after a key to pick its sound (and whether to save).
    /// The display part is the graphic options draft while that screen is open.
    fn snapshot(&self) -> (Screen, usize, Vec<u32>, Vec<bool>) {
        let s = &self.settings;
        let v = &s.view;
        let d = &self.gfx.map_or(s.display, |e| e.display);
        let rgb = |c: [u8; 3]| u32::from_be_bytes([0, c[0], c[1], c[2]]);
        (
            self.leave.map_or(self.screen, |l| l.to),
            ((self.sel * 4 + self.gfx.map_or(0, |e| e.button)) * 8 + self.gfx.map_or(0, |e| e.page)),
            vec![
                s.car as u32,
                s.cam.start as u32,
                s.cam.follow as u32,
                s.track as u32,
                s.laps,
                s.horn as u32,
                s.difficulty as u32,
                s.vol_sfx,
                s.vol_music,
                s.vol_ambient,
                (v.distance * 1000.0) as u32,
                (v.brightness * 1000.0) as u32,
                s.fps_limit,
                d.monitor as u32,
                d.size.0 * 10_000 + d.size.1,
                self.controls.active as u32,
                rgb(s.player_colour),
                rgb(s.car_colours[0]),
                rgb(s.car_colours[1]),
                rgb(s.car_colours[2]),
                s.info_detail as u32,
                s.checkpoint_info as u32,
                s.checkpoint_delay,
                s.hud.lap_delay,
                s.hud.sector_delay,
                s.road_map as u32,
                s.detail_mode as u32,
                s.name_plates as u32,
                s.menu_confirmation as u32,
                s.menu_cd_track,
                v.car_shading as u32,
                v.remaster.render_aa.kind as u32,
                v.remaster.render_aa.strength,
                v.remaster.post_aa.kind as u32,
                v.remaster.post_aa.strength,
                v.remaster.anisotropy as u32,
                v.remaster.grade,
                v.remaster.vignette,
                v.remaster.grain,
                v.remaster.tonemap as u32,
                v.remaster.exposure,
                v.remaster.bloom,
                v.remaster.ssao,
                s.hud.scale,
                s.hud.margin,
                s.hud.map_mode as u32,
                s.track_detail as u32,
                v.car_detail as u32,
                s.perspective_on as u32,
                s.perspective as u32,
            ],
            vec![
                s.catchup,
                v.shadows,
                v.halos,
                v.fog,
                s.music,
                s.fps_limit == 0,
                d.fullscreen,
                d.wide,
                s.menu_stretch,
                d.vsync,
                s.manual,
                s.record_race,
                s.ghost_car,
                s.skid_marks,
                s.cam.mirror,
                s.cam.free_look,
                s.cam.pull_in,
                v.sky_image,
                v.motion_blur,
                v.lens_flares,
                v.sparks,
                v.smoke,
                v.mode == RenderMode::Remaster,
                v.remaster.enhanced_paint,
                v.remaster.dynamic_lights,
                v.remaster.pixel_light,
                v.remaster.pixel_fog,
                v.remaster.clear_coat,
                v.remaster.hdr,
                v.remaster.smooth_halos,
                v.remaster.smooth_flares,
                v.remaster.car_shadows,
                v.remaster.modern_hud,
                v.remaster.modern_menu,
                v.remaster.generated_cars,
                s.cd_loop,
                s.engine_sound,
                s.custom_car_colours,
                s.hud.cluster,
                s.hud.clocks[0],
                s.hud.clocks[1],
                s.hud.clocks[2],
                s.hud.lap,
                s.hud.pos,
                s.hud.map,
                s.hud.map_rotate,
                s.hud.map_numbers,
                s.hud.plate_frame,
                s.hud.live_gaps,
                s.hud.proximity,
                v.remaster.velocity_blur,
                v.remaster.skids_detailed,
                v.remaster.fog_style != FogStyle::Classic,
            ],
        )
    }

    /// F11: flip between window and fullscreen (saved like the graphic option).
    pub fn toggle_fullscreen(&mut self) {
        self.settings.display.fullscreen ^= true;
        if let Some(e) = &mut self.gfx {
            e.display.fullscreen = self.settings.display.fullscreen;
        }
        self.save_settings();
    }

    /// F9: flip between the Classic and Remaster graphics (saved like the graphic option;
    /// works in menus and races). A key the control layout uses is left to the layout.
    pub fn toggle_render_mode(&mut self) {
        self.settings.view.mode = self.settings.view.mode.toggle();
        if let Some(e) = &mut self.gfx {
            e.backup.view.mode = self.settings.view.mode;
        }
        // the options screens list other rows now
        if matches!(self.screen, Screen::Graphics | Screen::PauseGfx | Screen::PauseHud) {
            self.sel = 0;
        }
        if self.holo_screen() {
            self.ensure_menu_drive();
        }
        self.save_settings();
    }

    /// The graphics-mode key (F9) is free: not a task of the control layout in use, and no
    /// binding or text entry is waiting for a key.
    pub fn is_render_mode_key(&self, k: KeyCode) -> bool {
        k == KeyCode::F9 && self.capture.is_none() && self.editing.is_none() && !self.controls.layout().bind.contains(&Some(Code::Key(k)))
    }

    /// Graphic options Apply (`leave` = OK): display changes go to the window and wait for
    /// "Keep these settings?"; the other rows are already live and become the new baseline.
    fn apply_graphics(&mut self, leave: bool) {
        let Some(e) = self.gfx else { return };
        if e.display != self.settings.display {
            let old = self.settings.display;
            self.settings.display = e.display;
            self.keep_display = Some(KeepDisplay { sel: 1, left: KEEP_SECONDS, old, leave });
        } else {
            self.gfx = Some(e.rebase(&self.settings));
            if leave {
                self.leave_graphics();
            }
        }
    }

    /// Answer to "Keep these settings?" (a timeout counts as no).
    fn finish_keep(&mut self, keep: bool) {
        let Some(kd) = self.keep_display.take() else { return };
        if !keep {
            self.settings.display = kd.old;
            return;
        }
        self.save_settings();
        self.gfx = self.gfx.map(|e| e.rebase(&self.settings));
        if kd.leave {
            self.leave_graphics();
        }
    }

    /// Cancel / Esc: restore what Apply last committed and go back to the options dial.
    fn cancel_graphics(&mut self) {
        if let Some(e) = self.gfx {
            e.backup.restore(&mut self.settings);
        }
        self.leave_graphics();
    }

    /// Back to where the graphic options were opened: the options dial or the in-race menu.
    fn leave_graphics(&mut self) {
        // the Graphics screen needs its state while it slides out (`switch` drops it)
        if self.screen != Screen::Graphics {
            self.gfx = None;
        }
        if matches!(self.screen, Screen::PauseGfx | Screen::PauseCam | Screen::PauseHud) {
            // REMASTER: Esc on the tab resumes the race; Classic: the GFX Options item
            if self.settings.view.modern_menu() {
                self.pause_resume();
            } else {
                self.sel = 2;
                self.screen = Screen::Pause;
                self.t = 0.0;
            }
            return;
        }
        self.go_sel(Screen::Options, Some(1)); // graphic options
    }

    /// The selected screen, clamped to the connected ones.
    pub fn monitor(&self) -> usize {
        self.settings.display.monitor.min(self.monitors.len().saturating_sub(1))
    }

    /// Load saved settings and keep saving them from now on.
    pub fn load_settings(&mut self) {
        self.persist = true;
        if let Some(p) = stats::path(true) {
            self.stats = stats::Stats::load(&p);
        }
        self.refresh_locks();
        self.rng = league::Rng(std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(1, |d| d.subsec_nanos() ^ d.as_secs() as u32));
        let Some(path) = settings_path() else { return };
        if let Ok(text) = std::fs::read_to_string(&path) {
            let (c, t, h) = (self.car_ids(), self.track_ids(), self.horns.len());
            self.settings.apply(&text, &c, &t, h);
            println!("settings: {}", path.display());
            // a locked choice saved while browsing goes back to the last unlocked one
            self.fix_locked();
        }
        self.sync_controls();
    }

    /// Headless: write the running race's recording to `path`.
    pub fn export_recording(&mut self, path: &std::path::Path) -> Result<(), String> {
        let r = self.race.as_mut().ok_or("no race")?;
        let mut rec = r.recording.take().ok_or("not recording")?;
        if let Some(h) = r.sess.race.racers.iter().find(|x| x.human) {
            rec.header.race_ms = h.progress.finished_ms;
            rec.header.best_lap_ms = h.progress.best_lap_ms;
        }
        rec.save(path).map_err(|e| e.to_string())
    }

    /// Headless: write the race's ghost (the best lap driven) to `path`.
    pub fn export_ghost(&mut self, path: &std::path::Path) -> Result<(), String> {
        let r = self.race.as_ref().ok_or("no race")?;
        let g = r.ghost.as_ref().ok_or("no lap completed")?;
        println!("ghost lap {} ms, {} poses", g.lap_ms, g.poses.len());
        g.save(path, self.cars[self.settings.car].id, &self.tracks[r.track].name).map_err(|e| e.to_string())
    }

    /// The track of a Ghost mode recording, by name; refused as the original does (0x41bed8).
    fn ghost_track(&self, m: &Mde) -> Result<usize, String> {
        let i = self.tracks.iter().position(|t| t.name.eq_ignore_ascii_case(&m.header.track)).ok_or("Ghostmode error! Track not found.")?;
        if i >= self.locks().tracks_open {
            return Err("Ghostmode error! Track not yet available.".into());
        }
        Ok(i)
    }

    /// The `.MDE` header of the race being started: one entry per racer (0x41d3c4). Colours are
    /// 0x00RRGGBB, the control type is 3 for the keyboard player [I], 0 for the AI.
    fn demo_header(&self, sess: &Session, track: &str, laps: u32, catchup: bool, player: usize) -> mde::Header {
        let mut cars: [mde::CarInfo; mde::CAR_ENTRIES] = Default::default();
        for (i, r) in sess.race.racers.iter().enumerate().take(mde::CAR_ENTRIES) {
            let own = scene::car_colours(&self.root, r.car_id);
            // the record's CarColour1..3 as painted (0x41d54c); yours only with CustomCarColours
            let colours = if r.human { if self.settings.custom_car_colours { self.settings.car_colours } else { own } } else { r.driver.car_colours.unwrap_or(own) };
            let pack = |c: [u8; 3]| (c[0] as u32) << 16 | (c[1] as u32) << 8 | c[2] as u32;
            let pc = self.settings.player_colour;
            cars[i] = mde::CarInfo {
                active: true,
                name: r.driver.nick.clone(),
                team: if r.human { self.settings.team.clone() } else { "CPU".into() },
                car: self.cars.iter().find(|c| c.id == r.car_id).map_or_else(String::new, |c| c.name.clone()),
                colours: [pack(colours[0]), pack(colours[1]), pack(colours[2]), if r.human { pack(pc) } else { 0 }],
                w39: 0,
                control: if r.human { 3 } else { 0 },
            };
        }
        mde::Header::new(track, laps, catchup, player as u32, cars)
    }

    /// Enter on the Ghost list (0x462d4c): ghost race goes on to the Car screen with the recording's track, laps and
    /// CatchUp; replay race watches it (GameType 6). A track that is missing or not unlocked is refused with the
    /// original's message. Returns the menu sound.
    fn ghost_pick(&mut self) -> u32 {
        let Some((path, m)) = self.replays.get(self.sel) else { return ev::SELECT_ERROR };
        if let Err(e) = self.ghost_track(m) {
            self.replay_msg = Some(e);
            return ev::SELECT_ERROR;
        }
        let m = m.clone();
        self.ghost_last = Some(demo_name(path).to_ascii_lowercase());
        if self.ghost_mode == 0 {
            self.ghost_demo = Some(m);
            self.game_type = GameType::GhostMode;
            self.go(Screen::Car);
        } else {
            self.watch_next = Some(m);
            self.go(Screen::Loading);
        }
        ev::SELECT
    }

    /// CLASSIC 0x462b1c: the recordings of `demos\*.mde` by name (case-insensitive [I: the directory's order]). The
    /// settings folder's `Demos/`, where Record race writes, is ours: a file there hides an install file of the same
    /// name. The list keeps the recording picked last selected (`DemoFileName`).
    fn load_replays(&mut self) {
        self.replays.clear();
        self.replay_info.clear();
        let mut found: Vec<PathBuf> = Vec::new();
        let dirs = [self.root.join("Demos"), self.demo_preview.clone().or_else(|| hiscore::demo_dir(self.persist)).unwrap_or_default()];
        for dir in dirs.iter().filter(|d| !d.as_os_str().is_empty()) {
            let Ok(rd) = std::fs::read_dir(dir) else { continue };
            for e in rd.flatten() {
                let p = e.path();
                if p.extension().and_then(|x| x.to_str()).is_some_and(|x| x.eq_ignore_ascii_case("mde")) {
                    let name = demo_name(&p).to_ascii_lowercase();
                    found.retain(|f| demo_name(f).to_ascii_lowercase() != name);
                    found.push(p);
                }
            }
        }
        found.sort_by_key(|f| demo_name(f).to_ascii_lowercase());
        for path in found {
            match Mde::load(&path) {
                Ok(m) => {
                    let info = self.rec_info(&path, &m);
                    self.replay_info.push(info);
                    self.replays.push((path, m));
                }
                Err(e) => eprintln!("replay {}: {e}", path.display()),
            }
        }
        let last = self.ghost_last.as_deref();
        self.sel = self.replays.iter().position(|(p, _)| Some(demo_name(p).to_ascii_lowercase().as_str()) == last).unwrap_or(0);
    }

    /// Replay (`.mhr`): rebuild the recorded race and watch it. The sim takes the
    /// recording's inputs in `drive`, so it plays out as it did (replay.rs); `for_recording` puts the sim on the
    /// physics the file was made on (the fixed tick of an MHR3, #374; the per-frame path of an older file).
    pub fn start_playback(&mut self, rec: Recording) -> Result<(), String> {
        let h = &rec.header;
        let ti = self.tracks.iter().position(|t| t.n == h.track).ok_or("the recording's track is not installed")?;
        let mut sess = Session::for_recording(&self.root, &rec)?;
        let paint: Vec<_> = sess.race.racers.iter().enumerate().filter(|(_, r)| !r.human).filter_map(|(i, r)| r.driver.car_colours.map(|c| (i, scene::CarPaint::gloss(c)))).collect();
        let sc = scene::load(&self.root, h.track, &h.cars, &paint)?;
        sess.set_wheels(&sc);
        self.pending = Some(sc);
        self.loaded = Loaded::Race;
        self.game_type = GameType::Single;
        let cam = Camera::looking(Vec3::ZERO, Vec3::NEG_Z);
        let records = hiscore::Records::load(None, &self.tracks[ti].name, h.laps);
        self.race = Some(RaceState { sess, sounds: None, track: ti, finished_for: None, lap_flash: None, split: None, sector_flash: None, last_lap: 1, last_cp: None, cam, records, ta_gaps: [None; 3], prox: Vec::new(), map_zoom: crate::hud::MAP_ZOOM_MIN, eggs: Default::default(), rec_laps: 0, rec_finished: false, playback: Some((rec, 0)), recording: None, demo: None, demo_rec: None, demo_over: false, watch: None, pb: None, notice: None, pending_respawn: None, pending_skip: false, ghost: None, lap_rec: None, ghost_laps: 0, run: stats::Run::default() });
        self.steer = SteerKeys::default();
        self.pedals = Pedals::default();
        self.controls.reset();
        Ok(())
    }

    /// Demo play (GameType 6): watch an `.MDE`. The recorder's car is the only racer, remote and posed
    /// from the stream so the cameras, HUD and sound follow it; the other recorded cars are drawn
    /// beside it (`DemoCars`). The track must be installed and unlocked as for Ghost mode.
    /// Blocking, like `start_race`.
    pub fn start_watch(&mut self, m: Mde) -> Result<(), String> {
        let (plan, fin) = self.watch_plan(m)?;
        let load = plan.run(&Progress::default())?;
        self.finish_race(fin, load, false);
        Ok(())
    }

    /// The window's cursor moved to (`x`, `y`) window px of a `size` window (#352): kept on the HUD canvas for the demo-play
    /// controls, which a move also shows for a while.
    pub fn pointer_moved(&mut self, x: f32, y: f32, size: (u32, u32)) {
        let c = Canvas::hud(size);
        let (sx, sy, ox, oy) = c.axes(size);
        self.pointer = Some(([(x - ox) / sx, (y - oy) / sy], c.w));
        if self.screen == Screen::Race && self.settings.view.modern_hud() {
            if let Some(pb) = self.race.as_mut().and_then(|r| r.pb.as_mut()) {
                pb.touch();
            }
        }
    }

    /// The cursor left the window (or the window was resized: the canvas px no longer hold).
    pub fn pointer_left(&mut self) {
        self.pointer = None;
    }

    /// Whether the system's mouse arrow shows: not over a race (the picture has none; REMASTER demo play draws its own
    /// Glass pointer), yes in every menu, the pause menu and the loading screen.
    pub fn os_pointer(&self) -> bool {
        !(self.screen == Screen::Race && self.race.is_some())
    }

    /// Headless shots (`--playback SPEC`): put the demo play controls in a state. `SPEC` is a comma list of `at=S` (the
    /// race clock, s; the "3" is -6), `paused`, `rate=R`, `tele`, `idle`, `ended`, `cam=chase|far|incar|bumper|tv`,
    /// `hold=-1|1` (returned as the held key the shot keeps down), `ptr=X:Y` (the pointer, window px of the shot) and
    /// `press` (the left button held). Returns the held keys, the pointer and the button for `main`.
    pub fn set_playback(&mut self, spec: &str) -> Result<ShotPlayback, String> {
        let Some(r) = &mut self.race else { return Err("--playback: no race".into()) };
        let (Some(block), Some(d), Some(pb)) = (r.watch, r.demo.as_ref(), r.pb.as_mut()) else { return Err("--playback: not demo play".into()) };
        let player = r.sess.race.racers.iter().position(|x| x.human).unwrap_or(r.sess.focus);
        let (mut held, mut jumped, mut cam, mut ptr, mut press) = (Vec::new(), false, None, None, false);
        for part in spec.split(',').filter(|p| !p.is_empty()) {
            let (key, val) = part.split_once('=').unwrap_or((part, ""));
            let num = || val.parse::<f64>().map_err(|_| format!("--playback {part}: a number"));
            match key {
                "at" => {
                    pb.seek(&mut r.sess, d, block, player, motor_sim::demo::GO_MS + num()? * 1000.0);
                    jumped = true;
                }
                "ended" => {
                    pb.seek(&mut r.sess, d, block, player, pb.len_ms);
                    jumped = true;
                }
                "paused" => pb.playing = false,
                "rate" => pb.rate = playback::RATES.iter().position(|&x| (x as f64 - num().unwrap_or(1.0)).abs() < 1e-3).ok_or("--playback rate: 0.25 0.5 1 2 4")?,
                "tele" => {
                    pb.tele = true;
                    pb.ensure_trace(d, block);
                    pb.snap();
                }
                "idle" => pb.hide(),
                "cam" => cam = Some(CamMode::parse(val).ok_or("--playback cam: chase far incar bumper tv")?),
                "hold" => held.push(if num()? < 0.0 { KeyCode::ArrowLeft } else { KeyCode::ArrowRight }),
                "ptr" => {
                    let xy = val.split_once(':').and_then(|(x, y)| Some((x.parse::<f32>().ok()?, y.parse::<f32>().ok()?)));
                    ptr = Some(xy.ok_or("--playback ptr=X:Y: window px")?);
                }
                "press" => press = true,
                _ => return Err(format!("--playback {part}: at=S paused rate=R tele idle ended cam=NAME hold=-1|1 ptr=X:Y press")),
            }
        }
        if jumped {
            r.resync();
        }
        if let Some(c) = cam {
            r.sess.cam = c;
        }
        Ok((held, ptr, press))
    }

    /// The main thread's part of `start_watch` (see `race_plan`).
    fn watch_plan(&mut self, m: Mde) -> Result<(LoadPlan, Finish), String> {
        let ti = self.ghost_track(&m)?;
        let tr = &self.tracks[ti];
        let h = &m.header;
        let slots = h.active_slots();
        let own = self.cars[self.settings.car].id;
        let car_of = |n: &str| self.cars.iter().find(|c| c.name.eq_ignore_ascii_case(n)).map_or(own, |c| c.id);
        let slot = if slots.contains(&(h.player as usize)) { h.player as usize } else { *slots.first().ok_or("No started race recorded.")? };
        let block = slots.iter().position(|&s| s == slot).unwrap_or(0);
        let info = h.cars[slot].clone();
        let (pid, laps, catchup) = (car_of(&info.car), h.laps, h.catchup());
        let rgb = |v: u32| [(v >> 16) as u8, (v >> 8) as u8, v as u8];
        let paint: Vec<(usize, scene::CarPaint)> = Some(&info).filter(|i| i.colours[..3].iter().any(|&v| v != 0)).map(|i| (0, scene::CarPaint::gloss([rgb(i.colours[0]), rgb(i.colours[1]), rgb(i.colours[2])]))).into_iter().collect();
        let demo = DemoPlan { mde: m, first_mesh: 1, skip: Some(slot), cars: self.car_table(), fallback: own };
        let plan = LoadPlan { root: self.root.clone(), track: tr.n, ids: vec![pid], player: Some(0), row: AiRow::Difficulty(self.settings.difficulty), laps, catchup, drivers: None, paint, driver_paint: false, demo: Some(demo), car_id: self.cars.get(self.settings.car).map_or(1, |c| c.id), engine_sound: self.settings.engine_sound, cheat: scene::SceneCheat::default() };
        let fin = Finish::Watch(WatchFinish { track_idx: ti, track_name: tr.name.clone(), block, name: info.name.clone(), laps });
        Ok((plan, fin))
    }

    /// Write the race recording (if any) to `Replays/` next to settings.cfg, named by
    /// track and date, once: the finished race, or the one left through the pause menu.
    fn save_recording(&mut self) {
        self.save_demo();
        let Some(r) = &mut self.race else { return };
        let Some(mut rec) = r.recording.take().filter(|x| x.frames.len() > 1) else { return };
        if let Some(h) = r.sess.race.racers.iter().find(|x| x.human) {
            rec.header.race_ms = h.progress.finished_ms;
            rec.header.best_lap_ms = h.progress.best_lap_ms;
        }
        if !self.settings.record_race {
            return;
        }
        let Some(dir) = crate::hiscore::replay_dir(self.persist) else { return };
        let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
        let name: String = rec.header.track_name.chars().filter(char::is_ascii_alphanumeric).collect();
        if let Err(e) = rec.save(&dir.join(format!("{name}-{secs}.mhr"))) {
            eprintln!("replay: {e}");
        }
    }

    /// Record race: the finished recording goes to the "Save the recorded race?" requester; the
    /// original asks at the end of the race (0x4632c6). Nothing is offered without a settings folder
    /// (headless runs).
    fn save_demo(&mut self) {
        let Some(r) = &mut self.race else { return };
        let Some(rec) = r.demo_rec.take().filter(|x| x.len() > 1) else { return };
        // headless `--save-demo`: the finished recording goes straight to the file
        if let Some(path) = &self.demo_export {
            match rec.finish().save(path) {
                Ok(()) => println!("wrote {}", path.display()),
                Err(e) => eprintln!("{}: {e}", path.display()),
            }
            return;
        }
        if hiscore::demo_dir(self.persist).is_some() {
            self.demo_save = Some(DemoSave { mde: rec.finish(), sel: 0, name: None });
        }
    }

    /// Write the offered recording to `Demos/<name>.MDE` (an empty name becomes the track and the date).
    fn write_demo(&mut self, ds: DemoSave) {
        let Some(dir) = hiscore::demo_dir(self.persist) else { return };
        let mut name: String = ds.name.unwrap_or_default().trim().chars().filter(|c| c.is_ascii_alphanumeric() || matches!(c, ' ' | '-' | '_')).collect();
        if name.is_empty() {
            let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
            let track: String = ds.mde.header.track.chars().filter(char::is_ascii_alphanumeric).collect();
            name = format!("{track}-{secs}");
        }
        if let Err(e) = ds.mde.save(&dir.join(format!("{name}.MDE"))) {
            eprintln!("demo: {e}");
        }
    }

    /// Headless `--save-requester`: the "Save the recorded race?" requester (`None`) or the name entry
    /// with `name` typed, over the running race.
    pub fn preview_demo_save(&mut self, name: Option<&str>) {
        let Some(mde) = self.race.as_ref().and_then(|r| r.demo_rec.as_ref()).map(|d| d.snapshot()) else { return };
        self.demo_save = Some(DemoSave { mde, sel: 0, name: name.map(str::to_string) });
        if name.is_some() {
            self.osk_open();
        }
    }

    /// Headless: write the running race's `.MDE` recording to `path`.
    pub fn export_demo(&mut self, path: &std::path::Path) -> Result<(), String> {
        let exported = self.demo_export.is_some();
        let r = self.race.as_mut().ok_or("no race")?;
        // a race that finished has been written already (`save_demo`)
        let Some(rec) = r.demo_rec.as_ref() else { return if exported { Ok(()) } else { Err("not recording".into()) } };
        let mde = rec.snapshot();
        println!("demo: {} records, {:.1} s", mde.records.len(), mde.seconds());
        mde.save(path).map_err(|e| e.to_string())
    }

    /// The file number (`GameN/carNN.car`) of each car, in list order.
    pub fn car_ids(&self) -> Vec<u32> {
        self.cars.iter().map(|c| c.id).collect()
    }

    /// The file number (`TrackN.trk`) of each track, in list order (sorted by Division).
    pub fn track_ids(&self) -> Vec<u32> {
        self.tracks.iter().map(|t| t.n).collect()
    }

    /// The list index of track file `n` (`--track N`), or the first track.
    pub fn track_index(&self, n: u32) -> usize {
        self.tracks.iter().position(|t| t.n == n).unwrap_or(0)
    }

    /// The list index of car file `id`, or the first car.
    pub fn car_index(&self, id: u32) -> usize {
        self.cars.iter().position(|c| c.id == id).unwrap_or(0)
    }

    fn save_settings(&self) {
        let Some(path) = settings_path().filter(|_| self.persist) else { return };
        // the attract demo's HUD overrides are not the player's settings
        let mut own = None;
        if let Some(sv) = &self.attract_saved {
            let mut c = self.settings.clone();
            (c.info_detail, c.checkpoint_info, c.road_map, c.hud) = sv.hud;
            own = Some(c);
        }
        let settings = own.as_ref().unwrap_or(&self.settings);
        let saved = path.parent().map_or(Ok(()), std::fs::create_dir_all).and_then(|_| std::fs::write(&path, settings.to_text(&self.car_ids(), &self.track_ids())));
        if let Err(e) = saved {
            eprintln!("settings {}: {e}", path.display());
        }
    }

    /// The recorded race's name being typed: Enter saves it, Esc drops the recording, Backspace deletes.
    pub(crate) fn demo_name_key(&mut self, k: KeyCode) {
        let Some(mut ds) = self.demo_save.take() else { return };
        match k {
            KeyCode::Enter | KeyCode::NumpadEnter => {
                self.sfx.push(Sfx::Menu(ev::SELECT));
                return self.write_demo(ds);
            }
            KeyCode::Escape => return self.sfx.push(Sfx::Menu(ev::ESCAPE)),
            KeyCode::Backspace => {
                if let Some(n) = &mut ds.name {
                    n.pop();
                }
            }
            _ => {}
        }
        self.demo_save = Some(ds);
    }

    /// Handle a key press and queue the menu sound the original would play for it
    /// (menusound.cfg events).
    pub fn key(&mut self, k: KeyCode) {
        // the attract demo ends on any key (design choice: the original stops on Enter / Esc only, 0x4286fd)
        if self.input_seen() {
            return;
        }
        // CLASSIC 0x46284c: no key handler while a screen slides out
        if self.leave.is_some() {
            return;
        }
        // requesters take all keys while they are open
        if let Some(kd) = self.keep_display {
            let (r, ev) = requester_key(k, kd.sel);
            match r {
                Req::Move(sel) => self.keep_display = Some(KeepDisplay { sel, ..kd }),
                Req::Pick(i) => self.finish_keep(i == 0),
                Req::None => {}
            }
            if let Some(e) = ev {
                self.sfx.push(Sfx::Menu(e));
            }
            return;
        }
        if let Some(sel) = self.confirm_exit {
            let (r, ev) = requester_key(k, sel);
            match r {
                Req::Move(sel) => self.confirm_exit = Some(sel),
                Req::Pick(i) => {
                    self.confirm_exit = None;
                    self.quit = i == 0;
                }
                Req::None => {}
            }
            if let Some(e) = ev {
                self.sfx.push(Sfx::Menu(e));
            }
            return;
        }
        // "Delete demo file." / "Are you sure?" (0x4d56ca, 0x4d56c0), Yes / NO!. The original asks only when
        // it runs under Windows (config +0x238 = 1, as the exit requester); we always ask.
        if let Some(sel) = self.ghost_del {
            let (r, ev) = requester_key(k, sel);
            match r {
                Req::Move(sel) => self.ghost_del = Some(sel),
                Req::Pick(i) => {
                    self.ghost_del = None;
                    if i == 0 && self.sel < self.replays.len() {
                        // headless shots keep the files (like the league saves)
                        if self.persist {
                            let _ = std::fs::remove_file(&self.replays[self.sel].0);
                        }
                        self.replays.remove(self.sel);
                        if self.sel < self.replay_info.len() {
                            self.replay_info.remove(self.sel);
                        }
                        self.sel = self.sel.min(self.replays.len().saturating_sub(1));
                        if self.replays.is_empty() {
                            self.ghost_row = 0;
                        }
                    }
                }
                Req::None => {}
            }
            if let Some(e) = ev {
                self.sfx.push(Sfx::Menu(e));
            }
            return;
        }
        if let Some((delete, sel)) = self.league_req {
            let (r, ev) = requester_key(k, sel);
            match r {
                Req::Move(sel) => self.league_req = Some((delete, sel)),
                Req::Pick(i) => {
                    self.league_req = None;
                    match (delete, i) {
                        // CLASSIC 0x46bff0: the question comes after the name; Yes keeps the league (0x44f8d0 made it
                        // when the screen opened), "No way!" deletes it again (0x44fff4)
                        (false, 0) if !self.modern_requester() => {
                            let name = std::mem::take(&mut self.league_name);
                            self.new_league(Some(&name));
                        }
                        (false, _) if !self.modern_requester() => self.league_name.clear(),
                        // REMASTER keeps its own order: the question, then a name (design choice)
                        (false, 0) => {
                            self.league_name.clear();
                            self.editing = Some((2, String::new()));
                            self.osk_open();
                        }
                        // the League dial deletes the current league, the Load screen the selected one (REMASTER)
                        (true, 0) => {
                            let at = if self.screen == Screen::LeagueLoad { self.sel } else { self.league };
                            self.delete_league_at(at);
                            self.sel = self.sel.min(self.leagues.len().saturating_sub(1));
                        }
                        _ => {}
                    }
                }
                Req::None => {}
            }
            if let Some(e) = ev {
                self.sfx.push(Sfx::Menu(e));
            }
            return;
        }
        // the recorded race's name: the pad's sheet when it is open, else the keyboard
        if self.demo_save.as_ref().is_some_and(|d| d.name.is_some()) {
            if self.osk.is_some() && self.controls.last != Device::Keyboard {
                self.osk_key(k);
            } else {
                self.demo_name_key(k);
            }
            if !self.editing_text() {
                self.osk = None;
            }
            return;
        }
        if let Some(mut ds) = self.demo_save.take() {
            let (r, ev) = requester_key(k, ds.sel);
            match r {
                Req::Move(sel) => ds.sel = sel,
                Req::Pick(0) => {
                    ds.name = Some(String::new());
                    self.osk_open();
                }
                Req::Pick(_) => return self.sfx.push(Sfx::Menu(ev.unwrap_or(ev::ESCAPE))),
                Req::None => {}
            }
            if let Some(e) = ev {
                self.sfx.push(Sfx::Menu(e));
            }
            self.demo_save = Some(ds);
            return;
        }
        if let Some(cap) = self.capture {
            // the Classic game menu row can take Esc itself (the original's capture has no way out)
            let esc_binds = self.screen == Screen::Controls && !self.settings.view.modern_menu() && cap.task == task::GAME_MENU;
            if k == KeyCode::Escape && !esc_binds {
                self.capture = None;
                self.sfx.push(Sfx::Menu(ev::ESCAPE));
            } else if !cap.pad {
                self.bind(cap.task, Code::Key(k));
            }
            return;
        }
        if self.editing.is_some() {
            // the sheet reads the pad; a keyboard (or mouse) used since types into the field as always
            if self.osk.is_some() && self.controls.last != Device::Keyboard {
                self.osk_key(k);
            } else {
                self.edit_key(k);
            }
            if self.editing.is_none() {
                self.osk = None;
            }
            return;
        }
        if self.screen == Screen::Loading {
            if matches!(k, KeyCode::Escape | KeyCode::Backspace) {
                self.cancel_load();
            }
            return;
        }
        let before = self.snapshot();
        let saved_text = self.settings.to_text(&self.car_ids(), &self.track_ids());
        let in_menu = !matches!(self.screen, Screen::Race | Screen::Loading);
        self.key_inner(k);
        // save whatever the key changed, so no setting depends on the snapshot lists
        if self.settings.to_text(&self.car_ids(), &self.track_ids()) != saved_text {
            self.save_settings();
        }
        if !in_menu {
            return;
        }
        if self.editing.is_some() {
            self.sfx.push(Sfx::Menu(ev::SELECT));
            return;
        }
        if self.capture.is_some() {
            let classic = self.screen == Screen::Controls && !self.settings.view.modern_menu();
            self.sfx.push(Sfx::Menu(if classic { ev::SELECT_CONTROL } else { ev::SELECT }));
            return;
        }
        let after = self.snapshot();
        let ev = if after.0 != before.0 {
            Some(if matches!(k, KeyCode::Escape | KeyCode::Backspace) { ev::ESCAPE } else { ev::SELECT })
        } else if after.3 != before.3 {
            let turned_on = after.3.iter().zip(before.3.iter()).any(|(a, b)| *a && !*b);
            Some(if turned_on { ev::SWITCH_ON } else { ev::SWITCH_OFF })
        } else if after.2 != before.2 {
            Some(if k == KeyCode::ArrowLeft { ev::ARROW_LEFT } else { ev::ARROW_RIGHT })
        } else if after.1 != before.1 {
            Some(if matches!(k, KeyCode::ArrowUp | KeyCode::ArrowLeft) { ev::CURSOR_UP } else { ev::CURSOR_DOWN })
        } else if matches!(k, KeyCode::Enter | KeyCode::NumpadEnter | KeyCode::Space) {
            Some(ev::SELECT_ERROR) // an item this build doesn't have
        } else {
            None
        };
        if let Some(e) = self.ev_override.take().or(ev) {
            self.sfx.push(Sfx::Menu(e));
        }
    }

    /// Headless shots: the "Keep these settings?" question.
    pub fn preview_keep_display(&mut self) {
        self.keep_display = Some(KeepDisplay { sel: 1, left: KEEP_SECONDS, old: self.settings.display, leave: false });
    }

    /// Headless shots: the New league name entry with `text` typed.
    /// Headless: show these statistics (never saved) on this page of the statistics screen.
    /// Headless Rankings shots over real runs: the tables of `dir` (read only), `name` counted as you.
    pub fn preview_hiscores(&mut self, dir: &Path, name: Option<&str>) {
        self.hiscore_dir = Some(dir.to_path_buf());
        if let Some(n) = name {
            self.settings.player_name = n.to_string();
        }
    }

    /// Headless League Load shots over saved leagues: the `.LGF` files of `dir` (read only: nothing is written or deleted
    /// without `persist`), the current league `LastLeague` of its `league.cfg`, else the first.
    /// Headless Ghost screen shots over a folder of recordings (read only).
    pub fn preview_demos(&mut self, dir: &Path) {
        self.demo_preview = Some(dir.to_path_buf());
        self.load_replays();
    }

    pub fn preview_leagues(&mut self, dir: &Path) {
        self.leagues = league::load_all(dir, &self.profiles, &self.me_profile());
        let last = league::read_last(&dir.join("league.cfg"));
        self.league = last.and_then(|f| self.leagues.iter().position(|l| l.file.eq_ignore_ascii_case(&f))).unwrap_or(0);
    }

    pub fn preview_stats(&mut self, stats: &stats::Stats, page: usize) {
        self.stats = stats.clone();
        self.stats_page = page.min(STATS_PAGES.len() - 1);
    }


    pub fn preview_league_name(&mut self, text: &str) {
        if self.modern_requester() {
            self.editing = Some((2, String::new()));
            self.league_name.clear();
            self.osk_open();
        } else {
            self.open_league_name();
        }
        self.type_text(text);
    }

    /// Characters typed on the pause tab list: a cheat code toggles its cheat and honks
    /// (a random "Honker" horn, any horn without one) [design choice].
    pub fn cheat_text(&mut self, t: &str) {
        if self.screen != Screen::Pause {
            return;
        }
        let Some((cheat, on)) = self.cheats.type_text(t) else { return };
        eprintln!("cheat {cheat:?} {}", if on { "on" } else { "off" });
        // the frozen race behind the pause menu shows the change at once
        if let Some(r) = &mut self.race {
            let (scale, feel) = self.cheats.car_size();
            r.sess.race.set_car_scale(scale, feel);
        }
        let honkers: Vec<usize> = self.horns.iter().enumerate().filter(|(_, h)| h.name.starts_with("Honker")).map(|(i, _)| i).collect();
        let pool = if honkers.is_empty() { (0..self.horns.len()).collect() } else { honkers };
        if !pool.is_empty() {
            self.sfx.push(Sfx::Honk(pool[self.rng.next() as usize % pool.len()]));
        }
    }

    /// Whether a text row of the personal screen takes typed characters.
    pub fn editing_text(&self) -> bool {
        self.editing.is_some() || self.demo_save.as_ref().is_some_and(|d| d.name.is_some())
    }

    /// Characters typed into the row being edited (printable ASCII: the font has no more).
    pub fn type_text(&mut self, t: &str) {
        if self.leave.is_some() {
            return;
        }
        if let Some(DemoSave { name: Some(n), .. }) = &mut self.demo_save {
            for c in t.chars().filter(|c| c.is_ascii_alphanumeric() || matches!(c, ' ' | '-' | '_')) {
                if n.chars().count() < DEMO_NAME_MAX {
                    n.push(c);
                }
            }
            return;
        }
        let Some((row, _)) = &self.editing else { return };
        if *row == 2 {
            for c in t.chars().filter(|c| c.is_ascii_graphic() || *c == ' ') {
                if self.league_name.chars().count() < LEAGUE_NAME_MAX {
                    self.league_name.push(c);
                }
            }
            return;
        }
        let team = *row == 1;
        let (field, max) = match *row {
            3..=12 => (&mut self.settings.short_keys[*row - 3], SHORT_KEY_MAX),
            1 => (&mut self.settings.team, TEAM_MAX),
            _ => (&mut self.settings.player_name, NAME_MAX),
        };
        for c in t.chars().filter(|c| c.is_ascii_graphic() || (*c == ' ' && !team)) {
            if field.chars().count() < max {
                field.push(c);
            }
        }
    }

    /// Editing keys: Backspace deletes, Enter keeps the text, Esc restores it.
    fn edit_key(&mut self, k: KeyCode) {
        let Some((row, old)) = self.editing.take() else { return };
        if row == 2 {
            match k {
                // CLASSIC: Enter asks "Create this league" (Esc cancels [I: 0x46bff0 deletes the league made at 0x46bc60])
                KeyCode::Enter | KeyCode::NumpadEnter | KeyCode::Escape if !self.modern_requester() => {
                    if k == KeyCode::Escape {
                        self.league_name.clear();
                    } else {
                        self.league_req = Some((false, 1));
                    }
                    self.sfx.push(Sfx::Menu(if k == KeyCode::Escape { ev::ESCAPE } else { ev::SELECT }));
                }
                // REMASTER: an empty name or Esc is a random "League??" [design choice]
                KeyCode::Enter | KeyCode::NumpadEnter | KeyCode::Escape => {
                    let name = std::mem::take(&mut self.league_name);
                    let typed = (k != KeyCode::Escape && !name.trim().is_empty()).then_some(name);
                    let name = typed.unwrap_or_else(|| League::random_name(&mut self.rng));
                    self.new_league(Some(&name));
                    self.sfx.push(Sfx::Menu(if k == KeyCode::Escape { ev::ESCAPE } else { ev::SELECT }));
                }
                _ => {
                    if k == KeyCode::Backspace {
                        self.league_name.pop();
                    }
                    self.editing = Some((row, old));
                }
            }
            return;
        }
        let field = match row {
            3..=12 => &mut self.settings.short_keys[row - 3],
            1 => &mut self.settings.team,
            _ => &mut self.settings.player_name,
        };
        match k {
            KeyCode::Enter | KeyCode::NumpadEnter => {
                *field = field.trim().to_string();
                if field.is_empty() && row == 0 {
                    *field = old;
                }
                self.save_settings();
                self.sfx.push(Sfx::Menu(ev::SELECT));
            }
            KeyCode::Escape => {
                *field = old;
                self.sfx.push(Sfx::Menu(ev::ESCAPE));
            }
            _ => {
                if k == KeyCode::Backspace {
                    field.pop();
                }
                self.editing = Some((row, old));
            }
        }
    }

    /// Step a row of an original on/off option with a Remaster style: Off, Classic, then the
    /// Remaster style (`modern`, only in Remaster mode; Classic mode leaves it alone).
    fn cycle_style(on: &mut bool, modern: Option<&mut bool>, fwd: bool) {
        match modern {
            None => *on ^= true,
            Some(m) => {
                let mut i = if !*on { 0 } else if *m { 2 } else { 1 };
                Self::cycle(&mut i, 3, fwd);
                (*on, *m) = (i > 0, i == 2);
            }
        }
    }

    /// Change one graphic-options row: left / right step it, enter toggles or cycles it.
    fn change_gfx_row(&mut self, e: &mut GfxEdit, row: GfxRow, left: bool, enter: bool) {
        let monitors = self.monitors.len().max(1);
        let active = row.active_in(&self.settings);
        let s = &mut self.settings;
        let (v, d) = (&mut s.view, &mut e.display);
        let classic = v.mode == RenderMode::Classic;
        match row {
            GfxRow::RenderDevice => {}
            GfxRow::TrackDetailRow => Self::cycle(&mut s.track_detail, TRACK_DETAILS.len(), !left),
            GfxRow::CarDetailRow => {
                let mut i = v.car_detail as usize;
                Self::cycle(&mut i, CarDetail::ALL.len(), !left);
                v.car_detail = CarDetail::ALL[i];
            }
            GfxRow::Monitor => {
                let mut m = d.monitor.min(monitors - 1);
                Self::cycle(&mut m, monitors, !left);
                d.monitor = m;
            }
            GfxRow::ScreenMode => d.fullscreen ^= true,
            GfxRow::Ratio => {
                // keep about the same size in the other ratio: same height or less
                d.wide ^= true;
                let list = resolutions(d.wide);
                d.size = list.iter().rev().find(|r| r.1 <= d.size.1).copied().unwrap_or(list[0]);
            }
            GfxRow::Resolution => {
                let list = resolutions(d.wide);
                let mut i = list.iter().position(|&r| r == d.size).unwrap_or(0);
                Self::cycle(&mut i, list.len(), !left);
                d.size = list[i];
            }
            GfxRow::MenuPicture => s.menu_stretch ^= true,
            GfxRow::VSync => d.vsync ^= true,
            GfxRow::FrameLimit => {
                let mut i = FRAME_LIMITS.iter().position(|&f| f == s.fps_limit).unwrap_or(1);
                Self::cycle(&mut i, FRAME_LIMITS.len(), !left);
                s.fps_limit = FRAME_LIMITS[i];
            }
            GfxRow::Brightness => v.brightness = (v.brightness + if left { -0.1 } else { 0.1 }).clamp(0.5, 2.0),
            GfxRow::CdMusic => s.music ^= true,
            GfxRow::DetailMode => {
                Self::cycle(&mut s.detail_mode, DETAIL_MODES.len(), !left);
                s.apply_detail();
            }
            GfxRow::Shadows => Self::cycle_style(&mut v.shadows, (!classic).then_some(&mut v.remaster.car_shadows), !left),
            GfxRow::Halos => Self::cycle_style(&mut v.halos, (!classic).then_some(&mut v.remaster.smooth_halos), !left),
            GfxRow::Fog if !classic => {
                // Off, Classic, Enhanced (the style rides on the row, like Halos)
                let mut i = if !v.fog { 0 } else { 1 + v.remaster.fog_style as usize };
                Self::cycle(&mut i, 4, !left);
                (v.fog, v.remaster.fog_style) = (i > 0, FogStyle::ALL[i.saturating_sub(1).min(2)]);
            }
            GfxRow::Fog => v.fog ^= true,
            GfxRow::CarShading => {
                // Classic: Flat, Gouraud, Reflection. Remaster: Gouraud, Reflection, Enhanced (Reflection
                // with the cube-mapped paint); a stored Flat shows as Gouraud there
                let r = &mut v.remaster;
                if classic {
                    v.car_shading = v.car_shading.cycle(!left);
                    r.enhanced_paint &= v.car_shading == CarShading::Reflection;
                } else {
                    let mut i = remaster_shading(v.car_shading, r.enhanced_paint);
                    Self::cycle(&mut i, 3, !left);
                    (v.car_shading, r.enhanced_paint) = (if i == 0 { CarShading::Gouraud } else { CarShading::Reflection }, i == 2);
                }
            }
            GfxRow::SkidMarks => Self::cycle_style(&mut s.skid_marks, (!classic).then_some(&mut v.remaster.skids_detailed), !left),
            GfxRow::NamePlates => Self::cycle(&mut s.name_plates, PLATES.len(), !left),
            GfxRow::SkyImage => Self::cycle_style(&mut v.sky_image, (!classic).then_some(&mut v.remaster.sky_hdri), !left),
            GfxRow::MotionBlur => Self::cycle_style(&mut v.motion_blur, (!classic).then_some(&mut v.remaster.velocity_blur), !left),
            GfxRow::Smoke => v.smoke ^= true,
            GfxRow::Sparks => v.sparks ^= true,
            GfxRow::LensFlares => Self::cycle_style(&mut v.lens_flares, (!classic).then_some(&mut v.remaster.smooth_flares), !left),
            GfxRow::InfoDetail => Self::cycle(&mut s.info_detail, INFO_LEVELS.len(), !left),
            GfxRow::CheckpointInfo => Self::cycle(&mut s.checkpoint_info, LEVELS.len(), !left),
            GfxRow::CheckpointDelay => s.checkpoint_delay = (s.checkpoint_delay as i32 + if left { -1 } else { 1 }).clamp(0, if classic { DELAY_CLASSIC_MAX } else { DELAY_MAX } as i32) as u32,
            GfxRow::LapDelay => s.hud.lap_delay = (s.hud.lap_delay as i32 + if left { -1 } else { 1 }).clamp(0, crate::hud_layout::LAP_DELAY_MAX as i32) as u32,
            GfxRow::SectorDelta => s.hud.sector_delay = (s.hud.sector_delay as i32 + if left { -1 } else { 1 }).clamp(0, crate::hud_layout::LAP_DELAY_MAX as i32) as u32,
            GfxRow::RoadMap => {
                Self::cycle(&mut s.road_map, LEVELS.len(), !left);
                s.hud.apply_road_map(s.road_map);
            }
            GfxRow::CamStart => {
                Self::cycle(&mut s.cam.start, CAM_STARTS.len(), !left);
                // in the pause menu the race switches at once (not on the goal camera)
                let start = s.cam;
                if let Some(r) = self.race.as_mut().filter(|r| r.sess.cam != CamMode::Goal) {
                    r.sess.cam = start.mode(!r.sess.race.tv_camera.is_empty());
                }
            }
            GfxRow::CamMirror => s.cam.mirror ^= true,
            GfxRow::CamFreeLook => s.cam.free_look ^= true,
            GfxRow::CamPullIn => s.cam.pull_in ^= true,
            GfxRow::CamFollow => Self::cycle(&mut s.cam.follow, CAM_FOLLOWS.len(), !left),
            GfxRow::Mode => v.mode = v.mode.toggle(),
            GfxRow::ModeSwitch => s.mode_dissolve ^= true,
            GfxRow::DynamicLights => v.remaster.dynamic_lights ^= true,
            GfxRow::RenderAa => {
                let kinds = RenderAaKind::AVAILABLE;
                let mut i = kinds.iter().position(|&k| k == v.remaster.render_aa.kind).unwrap_or(0);
                Self::cycle(&mut i, kinds.len(), !left);
                v.remaster.render_aa.kind = kinds[i];
            }
            GfxRow::PostAa => {
                let kinds = PostAaKind::AVAILABLE;
                let mut i = kinds.iter().position(|&k| k == v.remaster.post_aa.kind).unwrap_or(0);
                Self::cycle(&mut i, kinds.len(), !left);
                v.remaster.post_aa.kind = kinds[i];
            }
            GfxRow::RenderAaStrength | GfxRow::PostAaStrength if !enter => {
                // one stop of the shared 2x .. 16x scale per press, no wrap
                let p = if row == GfxRow::RenderAaStrength { &mut v.remaster.render_aa.strength } else { &mut v.remaster.post_aa.strength };
                let i = aa_stop(*p);
                *p = AA_STOP_STRENGTH[if left { i.saturating_sub(1) } else { (i + 1).min(AA_STOPS.len() - 1) }];
            }
            GfxRow::RenderAaStrength | GfxRow::PostAaStrength => {}
            GfxRow::Anisotropic => {
                let levels = RemasterView::ANISOTROPY_LEVELS;
                let mut i = levels.iter().position(|&n| n == v.remaster.anisotropy).unwrap_or(levels.len() - 1);
                Self::cycle(&mut i, levels.len(), !left);
                v.remaster.anisotropy = levels[i];
            }
            GfxRow::PixelLight => v.remaster.pixel_light ^= true,
            GfxRow::PixelFog => v.remaster.pixel_fog ^= true,
            GfxRow::ClearCoat => v.remaster.clear_coat ^= true,
            GfxRow::Signs => v.remaster.retro_signs ^= true,
            GfxRow::WorldFill => v.remaster.world_fill ^= true,
            GfxRow::TreeModels => v.remaster.tree_models ^= true,
            GfxRow::Water => v.remaster.water ^= true,
            GfxRow::WaterReflect => v.remaster.water_ssr ^= true,
            GfxRow::LocalLight => v.remaster.local_light ^= true,
            GfxRow::TexturePack => v.remaster.texture_pack ^= true,
            GfxRow::Grass => {
                let mut i = v.remaster.grass as usize;
                Self::cycle(&mut i, GrassLevel::ALL.len(), !left);
                v.remaster.grass = GrassLevel::ALL[i];
            }
            GfxRow::Hdr => v.remaster.hdr ^= true,
            GfxRow::HudStyle => v.remaster.modern_hud ^= true,
            GfxRow::MenuStyle => v.remaster.modern_menu ^= true,
            GfxRow::CarModels => v.remaster.generated_cars ^= true,
            GfxRow::LightShadows => {
                let mut i = v.remaster.light_shadows as usize;
                Self::cycle(&mut i, LightShadows::ALL.len(), !left);
                v.remaster.light_shadows = LightShadows::ALL[i];
            }
            GfxRow::ShadowQuality => {
                let mut i = v.remaster.shadow_quality as usize;
                Self::cycle(&mut i, ShadowQuality::ALL.len(), !left);
                v.remaster.shadow_quality = ShadowQuality::ALL[i];
            }
            GfxRow::Tonemap => {
                let mut i = v.remaster.tonemap as usize;
                Self::cycle(&mut i, Tonemap::ALL.len(), !left);
                v.remaster.tonemap = Tonemap::ALL[i];
            }
            GfxRow::FogStrength if !enter && row.active(v) => {
                let (lo, hi) = RemasterView::FOG_STRENGTH;
                let p = &mut v.remaster.fog_strength;
                *p = if left { p.saturating_sub(10).max(lo) } else { (*p + 10).min(hi) };
            }
            GfxRow::CarLodDistance if !enter && v.remaster.generated_cars => {
                let (lo, hi) = RemasterView::CAR_LOD_DISTANCE;
                let p = &mut v.remaster.car_lod_distance;
                *p = if left { p.saturating_sub(10).max(lo) } else { (*p + 10).min(hi) };
            }
            GfxRow::GrassRange if !enter && v.remaster.grass != GrassLevel::Off => {
                let (lo, hi) = RemasterView::GRASS_RANGE;
                let p = &mut v.remaster.grass_range;
                *p = if left { p.saturating_sub(10).max(lo) } else { (*p + 10).min(hi) };
            }
            GfxRow::Exposure if !enter => {
                let (lo, hi) = RemasterView::EXPOSURE_RANGE;
                let e = &mut v.remaster.exposure;
                *e = if left { e.saturating_sub(10).max(lo) } else { (*e + 10).min(hi) };
            }
            // sliders move with left / right only. View distance: the original clamps at
            // 30..150 % (0x428cfc / 0x428e88); REMASTER: up to 200 %, Classic draws at most
            // 150 % (View::distance) [design choice]
            GfxRow::ViewDistance if !enter => {
                v.distance = ((v.distance + if left { -0.1 } else { 0.1 }).clamp(0.3, View::MAX_DISTANCE) * 10.0).round() / 10.0;
            }
            GfxRow::ColourGrade | GfxRow::Vignette | GfxRow::Grain | GfxRow::Bloom | GfxRow::Ssao if !enter => {
                let r = &mut v.remaster;
                let p = match row {
                    GfxRow::ColourGrade => &mut r.grade,
                    GfxRow::Vignette => &mut r.vignette,
                    GfxRow::Bloom => &mut r.bloom,
                    GfxRow::Ssao => &mut r.ssao,
                    _ => &mut r.grain,
                };
                *p = if left { p.saturating_sub(10) } else { (*p + 10).min(100) };
            }
            // the race HUD's rows (the HUD tab): a row that does nothing with these settings does not change
            GfxRow::HudScale | GfxRow::HudMargin if !enter && active => {
                let (h, d) = (&mut s.hud, if left { -1 } else { 1 });
                if row == GfxRow::HudScale {
                    let (lo, hi) = SCALE_RANGE;
                    h.scale = (h.scale as i32 + d * SCALE_STEP as i32).clamp(lo as i32, hi as i32) as u32;
                } else {
                    h.margin = (h.margin as i32 + d * MARGIN_STEP as i32).clamp(0, 100) as u32;
                }
            }
            GfxRow::HudCluster | GfxRow::HudTotal | GfxRow::HudLapTime | GfxRow::HudBest | GfxRow::HudLap | GfxRow::HudPos => {
                fork_hud(s);
                let h = &mut s.hud;
                match row {
                    GfxRow::HudCluster => h.cluster ^= true,
                    GfxRow::HudTotal => h.clocks[0] ^= true,
                    GfxRow::HudLapTime => h.clocks[1] ^= true,
                    GfxRow::HudBest => h.clocks[2] ^= true,
                    GfxRow::HudLap => h.lap ^= true,
                    _ => h.pos ^= true,
                }
            }
            GfxRow::HudMap => {
                let i = map_row(s);
                let i = if enter { (i + 1) % MAP_LEVELS.len() } else if left { i.saturating_sub(1) } else { (i + 1).min(MAP_LEVELS.len() - 1) };
                set_map_row(s, i);
            }
            GfxRow::MapView if active => s.hud.map_mode = if s.hud.map_mode == MapMode::Close { MapMode::Circuit } else { MapMode::Close },
            GfxRow::MapTurns if active => s.hud.map_rotate ^= true,
            GfxRow::MapNumbers if active => s.hud.map_numbers ^= true,
            GfxRow::PlateFrame if active => s.hud.plate_frame ^= true,
            GfxRow::Proximity if active => s.hud.proximity ^= true,
            GfxRow::LiveGaps if active => s.hud.live_gaps ^= true,
            GfxRow::HudScale | GfxRow::HudMargin | GfxRow::MapView | GfxRow::MapTurns | GfxRow::MapNumbers | GfxRow::PlateFrame | GfxRow::Proximity | GfxRow::LiveGaps => {}
            GfxRow::ViewDistance | GfxRow::GrassRange | GfxRow::CarLodDistance | GfxRow::FogStrength | GfxRow::ColourGrade | GfxRow::Vignette | GfxRow::Grain | GfxRow::Exposure | GfxRow::Bloom | GfxRow::Ssao => {}
        }
        if row.preset() {
            s.detail_mode = CUSTOM_DETAIL;
        }
    }

    /// REMASTER modern graphic options keys [design choice]: no tab or button rows, `sel` is the
    /// row of the page. LB / RB (PageUp / PageDown, Q / E) switch the tab, Space or Y applies,
    /// R or X restores the page's defaults, Esc or B is Back (applies and leaves).
    fn graphics_key_modern(&mut self, k: KeyCode) {
        let mut e = self.gfx.unwrap_or_else(|| GfxEdit::new(&self.settings));
        // the pause tabs are one list each (Q / E change the tab, `key_inner`); the main menu pages
        let pause = self.screen != Screen::Graphics;
        let step = |next| if next { (e.page + 1).min(MODERN_GFX_PAGES.len() - 1) } else { e.page.saturating_sub(1) };
        let tab = match k {
            _ if pause => None,
            KeyCode::PageUp | KeyCode::KeyQ => Some(step(false)),
            KeyCode::PageDown | KeyCode::KeyE => Some(step(true)),
            _ => None,
        };
        let rows = gfx_page_rows(self.screen, e.page, &self.settings.view);
        let (up, down) = (k == KeyCode::ArrowUp, k == KeyCode::ArrowDown);
        let (left, right) = (k == KeyCode::ArrowLeft, k == KeyCode::ArrowRight);
        if let Some(page) = tab {
            e.page = page;
            self.sel = self.sel.min(gfx_page_rows(self.screen, page, &self.settings.view).len() - 1);
        } else if up || down {
            Self::cycle(&mut self.sel, rows.len(), down);
        } else if left || right || k == KeyCode::Enter || k == KeyCode::NumpadEnter {
            let row = rows[self.sel.min(rows.len() - 1)];
            self.change_gfx_row(&mut e, row, left, !(left || right));
        } else if matches!(k, KeyCode::KeyR | KeyCode::F6) {
            let def = Settings::default();
            for &row in &rows {
                row.reset(&mut self.settings, &mut e.display, &def);
            }
        }
        self.gfx = Some(e);
        match k {
            KeyCode::Space | KeyCode::F5 => self.apply_graphics(false),
            KeyCode::Escape | KeyCode::Backspace => self.apply_graphics(true),
            _ => {}
        }
    }

    /// REMASTER Controls (the pause menu's tab and, #304 phase 2, the main menu's screen) [design choice]: one list of the car
    /// tasks (Enter, then the next key, mouse button or pad button is bound, Esc / Start cancel), the
    /// keyboard layout and the four pad sliders; left / right turn the layout or a slider, R or F6
    /// restores the defaults, Esc resumes the race.
    fn controls_key_list(&mut self, k: KeyCode) {
        let (up, down) = (k == KeyCode::ArrowUp, k == KeyCode::ArrowDown);
        let (left, right) = (k == KeyCode::ArrowLeft, k == KeyCode::ArrowRight);
        let enter = matches!(k, KeyCode::Enter | KeyCode::NumpadEnter);
        if up || down {
            Self::cycle(&mut self.sel, PAUSE_CTL_ROWS, down);
        } else if self.sel < CAR_TASKS && enter {
            self.capture = Some(Capture { task: self.sel, pad: false, any: true });
            self.controls.take_pressed();
            self.controls.take_pad_pressed();
        } else if self.sel == CAR_TASKS && (left || right) {
            let mut i = self.controls.active;
            Self::cycle(&mut i, self.controls.layouts.len(), right);
            self.settings.layout = self.controls.layouts[i].name.clone();
            self.sync_controls();
        } else if self.sel > CAR_TASKS && (left || right) {
            let i = (self.sel - CAR_TASKS - 1).min(3);
            let (lo, hi) = PAD_TUNE_RANGE[i];
            let v = self.settings.pad_tune[i];
            self.settings.pad_tune[i] = if right { (v + PAD_TUNE_STEP).min(hi) } else { v.saturating_sub(PAD_TUNE_STEP).max(lo) };
            self.sync_controls();
        } else if matches!(k, KeyCode::KeyR | KeyCode::F6) {
            self.settings.pad = DEFAULT_PAD;
            self.settings.pad_tune = PAD_TUNE_DEFAULT;
            self.settings.layout = DEFAULT_LAYOUT_NAME.into();
            self.sync_controls();
        } else if matches!(k, KeyCode::Escape | KeyCode::Backspace) {
            if self.screen == Screen::Controls {
                self.go_sel(Screen::Options, Some(2)); // control options
            } else {
                self.pause_resume();
            }
        }
    }

    /// REMASTER pause menu: Esc on a settings tab goes back to the race.
    fn pause_resume(&mut self) {
        self.pause_hold.clear();
        self.gfx = None;
        self.screen = Screen::Race;
    }

    /// Graphic options keys, in the menus and in the race [design choice: the in-race GFX
    /// Options are the same screen]: the page tabs, the page's rows, then Apply / OK / Cancel.
    fn graphics_key(&mut self, up: bool, down: bool, left: bool, right: bool, enter: bool, back: bool) {
        let mut e = self.gfx.unwrap_or_else(|| GfxEdit::new(&self.settings));
        let rows = gfx_rows(e.page, &self.settings);
        // the Screen page is the original's list: no buttons
        let buttons_row = (e.page != SCREEN_PAGE).then_some(rows.len() + 1);
        if up || down {
            Self::cycle(&mut self.sel, rows.len() + 1 + usize::from(buttons_row.is_some()), down);
        }
        let buttons = Some(self.sel) == buttons_row;
        if self.sel == 0 && (left || right) {
            e.page = if right { (e.page + 1).min(GFX_PAGES.len() - 1) } else { e.page.saturating_sub(1) };
        } else if buttons && (left || right) {
            e.button = if right { (e.button + 1).min(GFX_BUTTONS.len() - 1) } else { e.button.saturating_sub(1) };
        } else if self.sel > 0 && !buttons && (left || right || enter) {
            let row = rows[self.sel - 1];
            self.change_gfx_row(&mut e, row, left, enter);
            // Graphic detail Custom <-> a preset shows or hides the rows below it
            self.sel = self.sel.min(gfx_rows(e.page, &self.settings).len());
        }
        self.gfx = Some(e);
        // CLASSIC: Esc leaves and keeps the changes (0x468378 writes the config); Cancel undoes them
        if back {
            self.apply_graphics(true);
        } else if buttons && enter && e.button == 2 {
            self.cancel_graphics();
        } else if buttons && enter {
            self.apply_graphics(e.button == 1);
        }
    }

    /// Restart the race from the grid (the pause menu's Restart race / hold).
    fn restart_race(&mut self) {
        self.commit_stats(false);
        if let Some(r) = &mut self.race {
            r.sess.race.restart();
            r.sess.fly_by = true;
            if matches!(r.sess.cam, CamMode::Goal | CamMode::Tv) {
                r.sess.cam = CamMode::Chase;
            }
            r.finished_for = None;
            r.lap_flash = None;
            r.split = None;
            r.sector_flash = None;
            r.last_cp = None;
            r.last_lap = 1;
            // demo play starts over: playing, the end choice forgotten (#346)
            if let Some(p) = &mut r.pb {
                p.restart();
                (r.demo_over, r.notice) = (false, None);
            }
            (r.ta_gaps, r.rec_laps, r.rec_finished) = ([None; 3], 0, false);
            if let Some(d) = &mut r.demo_rec {
                d.clear();
            }
            // the ghost keeps the best lap; the lap in progress starts over
            (r.ghost_laps, r.lap_rec) = (0, r.lap_rec.as_ref().map(|_| LapRecorder::default()));
            // the recording starts over with the race
            if let Some(rec) = &mut r.recording {
                rec.frames.clear();
            }
            (r.pending_respawn, r.pending_skip) = (None, false);
            if let Some(p) = &mut r.playback {
                p.1 = 0;
            }
        }
        self.pause_hold.clear();
        self.screen = Screen::Race;
    }

    /// Leave the race for the main menu (or the replay list when watching one).
    fn exit_race(&mut self) {
        self.save_recording();
        self.commit_stats(false);
        let watching = self.race.as_ref().is_some_and(|r| r.watching());
        self.race = None;
        self.pause_hold.clear();
        self.go(if watching { Screen::Replays } else { Screen::Main });
    }

    /// Any input: restart the main menu's idle count. An attract demo that is running (or loading) ends instead and
    /// the input is used up (returns true); the menu is back with the full 120 s.
    fn input_seen(&mut self) -> bool {
        self.idle_s = 0.0;
        if self.attract {
            self.end_attract(false);
            return true;
        }
        false
    }

    /// The attract demo (#210, GameType 5): a random regular track (`attract_tracks`; `track` picks one, the headless
    /// `--track`), the HUD info off, the game type Single. `race_plan` builds the race from it.
    pub fn begin_attract(&mut self, track: Option<usize>) {
        let divs: Vec<u32> = self.tracks.iter().map(|t| t.division).collect();
        let pool = attract_tracks(&divs, self.last_track);
        self.attract_track = match track {
            Some(t) => t,
            None if pool.is_empty() => self.settings.track,
            None => pool[self.rng.below(pool.len())],
        };
        let s = &mut self.settings;
        self.attract_saved = Some(AttractSaved { game_type: self.game_type, hud: (s.info_detail, s.checkpoint_info, s.road_map, s.hud) });
        (s.info_detail, s.checkpoint_info, s.road_map) = (0, 0, 0);
        s.hud.apply_road_map(0);
        self.game_type = GameType::Single;
        self.attract = true;
    }

    /// The idle main menu starts the demo through the loading screen.
    fn start_attract(&mut self) {
        self.begin_attract(None);
        self.go(Screen::Loading);
    }

    /// Leave the attract demo for the main menu: `finished` = it ran to its end, so the next one follows after 10 s
    /// (`0x470184`), else after the full 120 s. No results, records, statistics or recording (the original clears
    /// Record race for GameType 5, `0x41db24`).
    fn end_attract(&mut self, finished: bool) {
        if let Some(sv) = self.attract_saved.take() {
            self.game_type = sv.game_type;
            let s = &mut self.settings;
            (s.info_detail, s.checkpoint_info, s.road_map, s.hud) = sv.hud;
        }
        self.attract = false;
        self.idle_s = 0.0;
        self.idle_limit = if finished { ATTRACT_REARM_S } else { ATTRACT_IDLE_S };
        if self.loading.is_some() {
            self.cancel_load();
        } else {
            self.race = None;
            self.pause_hold.clear();
            self.go(Screen::Main);
        }
    }

    /// The AI drives the race by itself (the headless `--demo`, the attract demo).
    fn ai_demo(&self) -> bool {
        self.demo || self.attract
    }

    /// Headless shots: the pause menu on tab `i` of `PAUSE_TAB_SCREENS`.
    pub fn pause_goto(&mut self, i: usize) {
        for _ in 0..i {
            self.pause_tab(true);
        }
    }

    /// REMASTER pause menu: step to the next / previous tab (Q / E, LB / RB), concept C's order.
    /// Leaving Graphics or Camera applies its pending display change.
    fn pause_tab(&mut self, next: bool) {
        let Some(at) = PAUSE_TAB_SCREENS.iter().position(|s| *s == self.screen) else { return };
        let to = if next { (at + 1).min(PAUSE_TAB_SCREENS.len() - 1) } else { at.saturating_sub(1) };
        if to == at {
            return;
        }
        if matches!(self.screen, Screen::PauseGfx | Screen::PauseCam | Screen::PauseHud) {
            self.apply_graphics(false);
        }
        self.pause_hold.clear();
        self.screen = PAUSE_TAB_SCREENS[to];
        self.t = 0.0;
        self.sel = 0;
        self.gfx = matches!(self.screen, Screen::PauseGfx | Screen::PauseCam | Screen::PauseHud).then(|| GfxEdit::new(&self.settings));
    }

    /// Car screen: Left / Right (and Enter, on a setup row) change the selected row.
    fn car_row_change(&mut self, right: bool) {
        match self.sel {
            0 => {
                // the cars past the browse count are not in the list; the ends don't wrap
                self.step_car(if right { 1 } else { -1 });
            }
            2 if !self.horns.is_empty() => {
                let n = self.horns.len();
                Self::cycle(&mut self.settings.horn, n, right);
                // REMASTER (modern menu): the new horn plays once, so the names mean something [design choice, #95]
                if self.settings.view.modern_menu() {
                    self.sfx.push(Sfx::HornTry);
                    self.honk_at = Some(self.clock);
                }
            }
            1 => self.settings.manual ^= true,
            3 => self.settings.record_race ^= true,
            _ => self.settings.ghost_car ^= true,
        }
    }

    fn key_inner(&mut self, k: KeyCode) {
        let up = matches!(k, KeyCode::ArrowUp);
        let down = matches!(k, KeyCode::ArrowDown);
        let left = matches!(k, KeyCode::ArrowLeft);
        let right = matches!(k, KeyCode::ArrowRight);
        let enter = matches!(k, KeyCode::Enter | KeyCode::NumpadEnter | KeyCode::Space);
        let back = matches!(k, KeyCode::Escape | KeyCode::Backspace);
        // REMASTER pause menu: Q / E (LB / RB) switch the tab from every tab
        if self.settings.view.modern_menu() && PAUSE_TAB_SCREENS.contains(&self.screen) {
            let step = match k {
                KeyCode::PageUp | KeyCode::KeyQ => Some(false),
                KeyCode::PageDown | KeyCode::KeyE => Some(true),
                _ => None,
            };
            // typing "casper" in the Race tab: its E belongs to the code
            let typing = k == KeyCode::KeyE && self.screen == Screen::Pause && self.cheats.mid_code();
            if let Some(next) = step.filter(|_| !typing) {
                self.pause_tab(next);
                return;
            }
        }
        // REMASTER Personal (#91): Q / E (LB / RB) switch the tab; a text field being typed into takes its own keys
        // first, and the open palette cursor holds the tab: there they pick Gloss / Metallic (#135)
        if self.settings.view.modern_menu() && self.screen == Screen::Personal {
            match k {
                KeyCode::PageUp | KeyCode::KeyQ if self.paint_pick.is_some() => return self.pick_finish(false),
                KeyCode::PageDown | KeyCode::KeyE if self.paint_pick.is_some() => return self.pick_finish(true),
                KeyCode::PageUp | KeyCode::KeyQ => return self.personal_tab_step(false),
                KeyCode::PageDown | KeyCode::KeyE => return self.personal_tab_step(true),
                _ => {}
            }
        }
        // REMASTER Sound (#259): Q / E (LB / RB) switch between Volume and Songs
        if self.settings.view.modern_menu() && self.screen == Screen::Sound {
            match k {
                KeyCode::PageUp | KeyCode::KeyQ => return self.sound_tab_step(false),
                KeyCode::PageDown | KeyCode::KeyE => return self.sound_tab_step(true),
                _ => {}
            }
        }
        // REMASTER Results (#94): Q / E (LB / RB) switch the tab; with the glass hidden any key just brings it back
        if self.settings.view.modern_menu() && self.screen == Screen::Results && !self.results_watch {
            match k {
                KeyCode::PageUp | KeyCode::KeyQ => return self.results_tab_step(false),
                KeyCode::PageDown | KeyCode::KeyE => return self.results_tab_step(true),
                _ => {}
            }
        }
        // REMASTER Rankings (#93): Q / E (LB / RB) switch the table kind, Z / C (LT / RT) the difficulty
        if self.settings.view.modern_menu() && self.screen == Screen::Rankings {
            match k {
                KeyCode::PageUp | KeyCode::KeyQ => return self.rank_tab_step(false),
                KeyCode::PageDown | KeyCode::KeyE => return self.rank_tab_step(true),
                KeyCode::KeyZ | KeyCode::F7 => return self.rank_diff_step(false),
                KeyCode::KeyC | KeyCode::F8 => return self.rank_diff_step(true),
                _ => {}
            }
        }
        // REMASTER League standings (#92): Q / E (LB / RB) step the division along the ladder, towards the Final with E
        if self.settings.view.modern_menu() && self.screen == Screen::LeagueTable {
            match k {
                KeyCode::PageUp | KeyCode::KeyQ => return self.league_table_step(false),
                KeyCode::PageDown | KeyCode::KeyE => return self.league_table_step(true),
                _ => {}
            }
        }
        match self.screen {
            Screen::Main | Screen::Single | Screen::Options | Screen::League => {
                // five stops, one per notch (Single: six with Knockout; options: four items plus difficulty); right/down
                // turns clockwise, left/up anticlockwise, and the ends don't wrap
                if right || down {
                    self.sel = (self.sel + 1).min(self.dial_stops() - 1);
                }
                if left || up {
                    self.sel = self.sel.saturating_sub(1);
                }
                if enter {
                    match (self.screen, self.sel) {
                        (Screen::Main, 0) => {
                            self.go_sel(Screen::Single, Some(1)); // single race
                        }
                        (Screen::Main, 3) => {
                            self.go_sel(Screen::Options, Some(1)); // graphic options
                        }
                        (Screen::Main, 1) => self.go(Screen::Multi),
                        (Screen::Main, 2) => self.go(Screen::Rankings),
                        (Screen::Main, 4) => self.go(Screen::Credits),
                        (Screen::Single, 1) => {
                            self.game_type = GameType::Single;
                            self.go(Screen::Track);
                        }
                        (Screen::Single, 3) => {
                            self.game_type = GameType::TimeAttack;
                            self.go(Screen::Track);
                        }
                        (Screen::Single, 4) => {
                            self.game_type = GameType::Knockout;
                            self.go(Screen::Track);
                        }
                        (Screen::Single, 0) => {
                            // quick race: a single race with the current settings
                            self.game_type = GameType::Single;
                            self.go(Screen::Loading);
                        }
                        (Screen::Single, 2) => self.go(Screen::League),
                        (Screen::Single, 5) => {
                            self.game_type = GameType::GhostMode;
                            self.go(Screen::Replays);
                        }
                        (Screen::League, 0) => self.continue_league(),
                        (Screen::League, 1) => self.go(Screen::LeagueTable),
                        (Screen::League, 2) => self.go(Screen::LeagueLoad),
                        (Screen::League, 3) if !self.modern_requester() => self.open_league_name(),
                        (Screen::League, 3) => self.league_req = Some((false, 1)),
                        (Screen::League, 4) => self.league_req = Some((true, 1)),
                        (Screen::Options, 3) => self.go(Screen::Sound),
                        (Screen::Options, 1) => self.go(Screen::Graphics),
                        (Screen::Options, 2) => self.go(Screen::Controls),
                        (Screen::Options, 0) => self.go(Screen::Personal),
                        (Screen::Options, 4) => self.settings.difficulty = next_difficulty(self.settings.difficulty, true, self.settings.view.modern_menu()),
                        _ => {} // not implemented in this build
                    }
                }
                if back {
                    match self.screen {
                        // MenuConfirmation Off / NoExit: no exit requester [I: NoExit read
                        // as "every requester but the exit one"]
                        Screen::Main if matches!(k, KeyCode::Escape) => {
                            if self.settings.menu_confirmation == 1 {
                                self.confirm_exit = Some(1);
                            } else {
                                self.quit = true;
                            }
                        }
                        Screen::Main => {}
                        Screen::Single | Screen::Options => {
                            let from = self.screen;
                            self.go_sel(Screen::Main, Some(if from == Screen::Single { 0 } else { 3 }));
                        }
                        Screen::League => self.back_to_dial(Screen::Single, 2),
                        _ => {}
                    }
                }
            }
            Screen::Track => {
                if up || down {
                    Self::cycle(&mut self.sel, 3, down);
                }
                if left || right {
                    let d = if right { 1 } else { -1 };
                    match self.sel {
                        0 => self.step_track(d),
                        1 if self.game_type == GameType::Knockout => {} // fixed: cars - 1
                        1 => self.settings.laps = (self.settings.laps as i32 + d).clamp(1, 99) as u32,
                        _ => self.settings.catchup ^= true,
                    }
                }
                if enter {
                    // a locked track is refused (0x46ca5b)
                    if self.track_locked() {
                        self.ev_override = Some(ev::LOCKED_TRACK);
                    } else {
                        self.go(Screen::Car);
                    }
                }
                if back {
                    self.fix_locked();
                    let sel = match self.game_type {
                        GameType::TimeAttack => 3,
                        GameType::Knockout => 4,
                        _ => 1,
                    };
                    self.go_sel(Screen::Single, Some(sel));
                }
            }
            Screen::Car => {
                // Time Attack adds the Ghost car row (ours: the original has no such row)
                let rows = if self.game_type == GameType::TimeAttack { 5 } else { 4 };
                if up || down {
                    Self::cycle(&mut self.sel, rows, down);
                }
                if !(self.sel == 0 && (left || right)) {
                    self.prev_car = None;
                }
                if left || right {
                    self.car_row_change(right);
                }
                // a league allows the cars of your division only (0x46d595)
                if enter {
                    if self.sel != 0 {
                        // design choice: Enter / A on a setup row changes it like Right does; only the
                        // car row starts the race (the original's Enter on these rows is [U])
                        self.car_row_change(true);
                    } else if self.car_locked() {
                        // refused, and silent (0x46d8bb)
                        self.ev_override = Some(ev::LOCKED_CAR);
                    } else if self.car_allowed() {
                        self.go(Screen::Loading);
                    }
                }
                if back {
                    self.fix_locked();
                    if self.game_type == GameType::League {
                        self.back_to_dial(Screen::League, 0);
                    } else if self.game_type == GameType::GhostMode {
                        self.go(Screen::Replays);
                    } else {
                        self.go(Screen::Track);
                    }
                }
            }
            Screen::PauseCam => self.graphics_key_modern(k),
            Screen::PauseHud if self.settings.view.modern_menu() => self.graphics_key_modern(k),
            Screen::Graphics | Screen::PauseGfx if self.settings.view.modern_menu() => self.graphics_key_modern(k),
            Screen::Graphics | Screen::PauseGfx => self.graphics_key(up, down, left, right, enter, back),
            // Esc or Enter ends the whole chain (0x44bda8 on DIK 0x01 / 0x1c, then result 2 at 0x457eef)
            Screen::Intro | Screen::LeagueMovie => {
                if enter || back {
                    self.movie_queue.clear();
                    self.next_movie();
                }
            }
            Screen::Credits => {
                if enter || back {
                    self.go_sel(Screen::Main, Some(4));
                }
            }
            // the layout's tasks are read in drive(); these are fixed extras of ours
            Screen::Race => match k {
                KeyCode::Escape => self.go(Screen::Pause),
                // design choice (#216): Enter / pad A skips the start fly-by to the "3" (the original
                // has no skip); not where the layout binds it, nor in replays and demo play
                KeyCode::Enter | KeyCode::NumpadEnter if !self.controls.layout().bind.contains(&Some(Code::Key(k))) => {
                    self.skip_fly_by(true);
                }
                KeyCode::F8 | KeyCode::Pause if !self.controls.layout().bind.contains(&Some(Code::Key(k))) => self.frozen ^= true,
                _ => {}
            },
            // REMASTER Race tab (concept C): Resume / Restart / Exit; Restart and Exit run when
            // Enter is held for `HOLD_S` (`Game::update` fills `pause_hold`)
            Screen::Pause if self.settings.view.modern_menu() => {
                if up || down {
                    self.sel = if down { (self.sel + 1).min(PAUSE_ACTIONS - 1) } else { self.sel.saturating_sub(1) };
                    self.pause_hold.clear();
                }
                if back {
                    self.pause_hold.clear();
                    self.screen = Screen::Race;
                } else if enter && self.sel == 0 {
                    self.screen = Screen::Race;
                } else if matches!(k, KeyCode::Enter | KeyCode::NumpadEnter) && self.sel > 0 {
                    self.pause_hold.armed = true;
                    self.ev_override = Some(ev::SELECT);
                }
            }
            Screen::Pause => {
                self.sel = self.sel.min(PAUSE_ITEMS.len() - 1);
                if up || down {
                    Self::cycle(&mut self.sel, PAUSE_ITEMS.len(), down);
                }
                if matches!(k, KeyCode::Escape) {
                    self.screen = Screen::Race;
                }
                if enter {
                    match self.sel {
                        0 => self.screen = Screen::Race,
                        1 => self.restart_race(),
                        2..=4 => {
                            self.screen = [Screen::PauseGfx, Screen::PauseSfx, Screen::PauseHud][self.sel - 2];
                            self.t = 0.0;
                            self.sel = 0;
                            if self.screen == Screen::PauseGfx {
                                self.gfx = Some(GfxEdit::new(&self.settings));
                            }
                        }
                        _ => self.exit_race(),
                    }
                }
            }
            Screen::PauseHud => {
                // REMASTER: the tabbed panel has no Back row (Esc returns to the tabs)
                let modern = self.settings.view.modern_menu();
                if up || down {
                    Self::cycle(&mut self.sel, if modern { HUD_ROWS - 1 } else { HUD_ROWS }, down);
                }
                let back_row = !modern && self.sel == HUD_ROWS - 1;
                if left || right || (enter && !back_row) {
                    let s = &mut self.settings;
                    // editing a corner toggle forks the shown set into Custom (#181)
                    if self.sel <= 5 && s.info_detail != INFO_CUSTOM {
                        s.hud = s.hud.shown(s.info_detail);
                        s.info_detail = INFO_CUSTOM;
                    }
                    let h = &mut s.hud;
                    let d: i32 = if left { -1 } else { 1 };
                    match self.sel {
                        0 => h.cluster ^= true,
                        1..=3 => h.clocks[self.sel - 1] ^= true,
                        4 => h.lap ^= true,
                        5 => h.pos ^= true,
                        6 => h.map ^= true,
                        7 => h.map_mode = if h.map_mode == MapMode::Close { MapMode::Circuit } else { MapMode::Close },
                        8 => h.map_rotate ^= true,
                        9 => h.map_numbers ^= true,
                        10 => h.proximity ^= true,
                        11 => {
                            let (lo, hi) = SCALE_RANGE;
                            h.scale = (h.scale as i32 + d * SCALE_STEP as i32).clamp(lo as i32, hi as i32) as u32;
                        }
                        12 => h.margin = (h.margin as i32 + d * MARGIN_STEP as i32).clamp(0, 100) as u32,
                        _ => {}
                    }
                }
                if modern && back {
                    self.pause_resume();
                } else if back || (enter && back_row) {
                    self.screen = Screen::Pause;
                    self.t = 0.0;
                    self.sel = 4; // HUD Settings
                }
            }
            Screen::PauseSfx => {
                // the rows plus "Back" (REMASTER: the tabbed panel has no Back row)
                let modern = self.settings.view.modern_menu();
                let n = if modern { 5 } else { 4 }; // modern: SFX, CD volume, CD music, ambient, engine sound
                if up || down {
                    Self::cycle(&mut self.sel, n, down);
                }
                let back_row = !modern && self.sel == n - 1;
                if left || right || (enter && !back_row) {
                    let d: i32 = if left { -1 } else { 1 };
                    let s = &mut self.settings;
                    let step = |v: &mut u32| *v = step_volume(*v, d);
                    match self.sel {
                        0 => step(&mut s.vol_sfx),
                        1 => step(&mut s.vol_music),
                        2 if modern => s.music ^= true,
                        2 => step(&mut s.vol_ambient),
                        3 if modern => step(&mut s.vol_ambient),
                        4 if modern => s.engine_sound ^= true,
                        _ => {}
                    }
                }
                if modern && back {
                    self.pause_resume();
                } else if back || (enter && back_row) {
                    self.screen = Screen::Pause;
                    self.t = 0.0;
                    self.sel = 3; // SFX Options
                }
            }
            Screen::Sound if self.settings.view.modern_menu() && self.sound_page == SOUND_SONGS => self.sound_songs_key(up, down, left, right, enter, back),
            Screen::Sound if self.settings.view.modern_menu() => {
                // REMASTER: the list's eight rows: three volumes, CD music, CD loop, menu CD track, engine sound, sound device (#260)
                if up || down {
                    Self::cycle(&mut self.sel, 8, down);
                }
                if self.sel == SOUND_DEVICE_ROW && (left || right) {
                    self.step_sound_device(left);
                }
                let s = &mut self.settings;
                let toggle = matches!(self.sel, 3 | 4 | 6);
                if self.sel != SOUND_DEVICE_ROW && (left || right || (enter && toggle)) {
                    let d: i32 = if left { -1 } else { 1 };
                    match self.sel {
                        3 => s.music ^= true,
                        4 => s.cd_loop ^= true,
                        5 => s.menu_cd_track = step_cd_track(s.menu_cd_track, &self.cd_tracks, d),
                        6 => s.engine_sound ^= true,
                        0 => s.vol_sfx = step_volume(s.vol_sfx, d),
                        1 => s.vol_music = step_volume(s.vol_music, d),
                        _ => s.vol_ambient = step_volume(s.vol_ambient, d),
                    }
                }
                if back {
                    self.go_sel(Screen::Options, Some(3)); // sound options
                }
            }
            Screen::Sound => self.sound_key(up, down, left, right, enter, back),
            Screen::Controls if self.settings.view.modern_menu() => self.controls_key_list(k),
            Screen::PauseControls => self.controls_key_list(k),
            Screen::Controls => {
                // CLASSIC 0x4691e4: Left / Right move the focus between the layout column and the
                // task tree, Up / Down stop at the ends, Enter on the column leaves, on a task binds
                let list_end = if self.ctl_tasks { CLASSIC_TASKS.len() } else { self.controls.layouts.len() };
                if left || right {
                    self.ev_override = Some(if self.ctl_tasks == right {
                        ev::COLUMN_ERROR
                    } else if right {
                        ev::COLUMN_RIGHT
                    } else {
                        ev::COLUMN_LEFT
                    });
                    self.ctl_tasks = right;
                } else if up || down {
                    let at = if self.ctl_tasks { self.sel } else { self.controls.active };
                    let to = if down { Some(at + 1).filter(|&i| i < list_end) } else { at.checked_sub(1) };
                    self.ev_override = Some(match to {
                        None => ev::CURSOR_END,
                        Some(i) => {
                            if self.ctl_tasks {
                                self.sel = i;
                            } else {
                                self.settings.layout = self.controls.layouts[i].name.clone();
                                self.sync_controls();
                            }
                            if down { ev::CURSOR_DOWN } else { ev::CURSOR_UP }
                        }
                    });
                } else if enter && self.ctl_tasks {
                    self.capture = Some(Capture { task: CLASSIC_TASKS[self.sel], pad: false, any: false });
                    self.controls.take_pressed();
                } else if back || enter {
                    self.go_sel(Screen::Options, Some(2)); // control options
                }
            }
            Screen::Personal if self.settings.view.modern_menu() => self.personal_key(up, down, left, right, enter, back),
            Screen::Personal => {
                // CLASSIC 0x466f44: Up / Down stop at the ends (end sound), Left / Right step the colour
                // strips and switch Car colours (list sounds), a refused move is the error sound
                let mut e = None;
                if up || down {
                    let rows = personal_rows(self.settings.custom_car_colours);
                    let at = rows.iter().position(|&r| r == self.sel).unwrap_or(0);
                    let to = if down { Some(at + 1).filter(|&i| i < rows.len()) } else { at.checked_sub(1) };
                    e = Some(match to {
                        Some(i) => {
                            self.sel = rows[i];
                            if down { ev::CURSOR_DOWN } else { ev::CURSOR_UP }
                        }
                        None => ev::CURSOR_END,
                    });
                }
                let back_row = self.sel == PERSONAL_ROWS - 1;
                let s = &mut self.settings;
                if self.sel == PERSONAL_ROWS - 2 && enter {
                    self.go(Screen::Statistics);
                    self.stats_page = 0;
                    self.stats_scroll = 0;
                } else if enter && matches!(self.sel, 0 | 1 | 3..=12) {
                    // the original types into the selected row at once; ours starts on Enter [design choice]
                    let old = match self.sel {
                        0 => s.player_name.clone(),
                        1 => s.team.clone(),
                        r => s.short_keys[r - 3].clone(),
                    };
                    self.editing = Some((self.sel, old));
                } else if left || right {
                    match self.sel {
                        2 | 14..=16 => {
                            let c = if self.sel == 2 { &mut s.player_colour } else { &mut s.car_colours[self.sel - 14] };
                            *c = strip_step(&self.strip, *c, right);
                            e = Some(if left { ev::LIST_IN } else { ev::LIST_OUT });
                        }
                        13 if s.custom_car_colours != right => {
                            s.custom_car_colours = right;
                            e = Some(if left { ev::LIST_IN } else { ev::LIST_OUT });
                        }
                        17 => Self::cycle(&mut s.menu_confirmation, CONFIRMATIONS.len(), right),
                        18 => s.intro ^= true,
                        19 | 20 => {}
                        _ => e = Some(ev::SELECT_ERROR),
                    }
                } else if enter {
                    match self.sel {
                        17 => Self::cycle(&mut s.menu_confirmation, CONFIRMATIONS.len(), true),
                        18 => s.intro ^= true,
                        _ => {}
                    }
                }
                if e.is_some() {
                    self.ev_override = e;
                }
                if back || (enter && back_row) {
                    self.go_sel(Screen::Options, Some(0)); // personal options
                }
            }
            Screen::Statistics => {
                if left || right {
                    self.stats_page = (self.stats_page + if right { 1 } else { STATS_PAGES.len() - 1 }) % STATS_PAGES.len();
                    self.stats_scroll = 0;
                }
                if up || down {
                    let max = self.stats_list_len().saturating_sub(STATS_ROWS);
                    self.stats_scroll = if down { (self.stats_scroll + 1).min(max) } else { self.stats_scroll.saturating_sub(1) };
                }
                if enter || back {
                    // the modern screen comes back on the tab it left (`go`)
                    self.go_sel(Screen::Personal, (!self.settings.view.modern_menu()).then_some(PERSONAL_ROWS - 2));
                }
            }
            Screen::Results => {
                let modern = self.settings.view.modern_menu();
                // REMASTER: the glass is hidden to watch the finish: any key brings it back; X (the pad's X) hides it
                if modern && self.results_watch {
                    self.results_watch = false;
                } else if modern && matches!(k, KeyCode::KeyX | KeyCode::F6) {
                    self.results_watch = true;
                } else if modern && (up || down) {
                    self.results_vertical(down);
                } else if modern && (left || right) {
                    self.results_horizontal(right);
                } else if enter || back {
                    if modern && self.results_has_league() && !self.results_league_tab() {
                        // REMASTER: the standings are the League tab of this screen
                        self.results_goto_league();
                    } else if modern && self.results_has_league() {
                        self.leave_league_results();
                    } else if self.game_type == GameType::League && self.league_outcome == Some(Outcome::Season) {
                        // CLASSIC 0x42a2b4 returns at once for code 0: no standings screen mid-season (#225)
                        self.leave_league_results();
                    } else if self.game_type == GameType::League && self.league_outcome.is_some() {
                        self.go(Screen::LeagueResults);
                    } else {
                        self.race = None;
                        self.go(Screen::Main);
                    }
                }
            }
            Screen::LeagueResults => {
                if enter || back {
                    self.leave_league_results();
                }
            }
            Screen::LeagueTable => {
                // left / right: the other divisions (design choice); REMASTER runs them as its tabs do, right = one up
                // towards the Final, the ends stopping with their sound
                if left || right {
                    if self.settings.view.modern_menu() {
                        self.league_table_step(right);
                    } else {
                        self.league_div = if right { (self.league_div + 1).min(3) } else { self.league_div.saturating_sub(1) };
                    }
                }
                if enter || back {
                    if std::mem::take(&mut self.standings_next) {
                        // after a league race: Enter = the next race's car screen, Esc = the dial (0x46b5ec)
                        if enter {
                            self.continue_league();
                        } else {
                            self.back_to_dial(Screen::League, 0);
                        }
                    } else {
                        self.back_to_dial(Screen::League, 1);
                    }
                }
            }
            Screen::LeagueLoad => {
                let n = self.leagues.len().max(1);
                let modern = self.settings.view.modern_menu();
                if up || down {
                    if modern {
                        // REMASTER: the list stops at the ends (19.4), with the end sound
                        let to = if down { (self.sel + 1).min(n - 1) } else { self.sel.saturating_sub(1) };
                        if to == self.sel {
                            self.ev_override = Some(ev::CURSOR_END);
                        }
                        self.sel = to;
                    } else {
                        Self::cycle(&mut self.sel, n, down);
                    }
                }
                // REMASTER: Del / the pad's X delete the selected save, behind the requester (KEEP IT! preselected)
                if modern && matches!(k, KeyCode::Delete | KeyCode::F6) && self.sel < self.leagues.len() {
                    self.league_req = Some((true, 1));
                    self.ev_override = Some(ev::REQUESTER);
                    return;
                }
                if enter {
                    self.league = self.sel.min(n - 1);
                    if let Some(l) = self.current_league() {
                        league::save_last(self.persist, &l.file);
                    }
                }
                if enter || back {
                    self.back_to_dial(Screen::League, 2);
                }
            }
            Screen::Replays if self.settings.view.modern_menu() => self.replays_key_modern(k),
            Screen::Analysis => self.analysis_key_modern(k),
            Screen::Replays => {
                // CLASSIC 0x462d4c: row 0 ghost race, 1 replay race, 2 the list. The ends stop with the
                // end sound (9), Left / Right and Enter move the focus between the rows and the list
                // (12 / 13), a refused move is the error sound (15).
                self.replay_msg = None;
                let n = self.replays.len();
                let mut e = None;
                if self.ghost_row == 2 {
                    if up || down {
                        let to = if up { self.sel.checked_sub(1) } else { Some(self.sel + 1).filter(|&i| i < n) };
                        e = Some(match to {
                            Some(i) => {
                                self.sel = i;
                                if up { ev::CURSOR_UP } else { ev::CURSOR_DOWN }
                            }
                            None => ev::CURSOR_END,
                        });
                    } else if left || back {
                        self.ghost_row = 0;
                        e = Some(ev::LIST_OUT);
                    } else if right {
                        e = Some(ev::SELECT_ERROR);
                    } else if k == KeyCode::Delete && self.sel < n {
                        self.ghost_del = Some(1); // starts on "NO!" (0x46278c)
                        e = Some(ev::REQUESTER);
                    } else if enter {
                        e = Some(self.ghost_pick());
                    }
                } else if up || down {
                    let to = if up { self.ghost_row.checked_sub(1) } else { Some(self.ghost_row + 1).filter(|&r| r < 2) };
                    e = Some(match to {
                        Some(r) => {
                            self.ghost_row = r;
                            if up { ev::CURSOR_UP } else { ev::CURSOR_DOWN }
                        }
                        None => ev::CURSOR_END,
                    });
                } else if left {
                    e = Some(ev::SELECT_ERROR);
                } else if right || enter {
                    e = Some(if n > 0 {
                        self.ghost_row = 2;
                        ev::LIST_IN
                    } else {
                        ev::SELECT_ERROR
                    });
                } else if back {
                    self.back_to_dial(Screen::Single, 5);
                }
                if self.ghost_row < 2 {
                    self.ghost_mode = self.ghost_row;
                }
                self.ev_override = e;
            }
            Screen::Multi => {
                // CLASSIC 0x463e04 / 0x463f84, the first page: join an existing game, create session.
                // The lobby behind them is story #7; Enter says so [design choice].
                if up || down {
                    let to = if up { self.sel.checked_sub(1) } else { Some(self.sel + 1).filter(|&i| i < 2) };
                    self.ev_override = Some(match to {
                        Some(i) => {
                            self.sel = i;
                            if up { ev::CURSOR_UP } else { ev::CURSOR_DOWN }
                        }
                        None => ev::CURSOR_END,
                    });
                } else if enter {
                    self.multi_msg = Some("Multi player is not available yet.".into());
                    self.ev_override = Some(ev::SELECT_ERROR);
                } else if back {
                    self.back_to_dial(Screen::Main, 1);
                }
            }
            Screen::Loading => {}
            Screen::Rankings if self.settings.view.modern_menu() => self.rankings_key_modern(up, down, left, right, enter || back),
            Screen::Rankings => self.rankings_key(up, down, left, right, enter || back),
        }
    }

    /// The rows the current table kind uses (0x4664fe..0x4665d0): 0 type, 1 track (division in the
    /// league view), 2 laps, 3 difficulty. Best laptime has no laps and difficulty, best racetime no
    /// difficulty.
    fn rank_rows(&self) -> Vec<usize> {
        match self.rank.kind {
            0 => vec![0, 1],
            1 => vec![0, 1, 2],
            2 => vec![0, 1, 2, 3],
            _ => vec![0, 1, 3],
        }
    }

    /// The table the filters select.
    fn rank_kind(v: &RankView) -> hiscore::Kind {
        match v.kind {
            0 => hiscore::Kind::BestLap,
            1 => hiscore::Kind::BestRace,
            2 => hiscore::Kind::SingleRace,
            _ => RANK_DIVISIONS[v.division],
        }
    }

    /// CLASSIC 0x465fef..0x466050: the filters a visit starts from. A single or league race starts on
    /// "Single race", the other modes on "Best racetime"; outside a league the track and difficulty are
    /// the current ones; the laps are the first of the list that is not below the current laps (0x45f994).
    fn rank_defaults(&mut self) {
        let league = self.game_type == GameType::League;
        self.rank.kind = if matches!(self.game_type, GameType::Single | GameType::League) { 2 } else { 1 };
        if !league {
            self.rank.track = self.settings.track.min(self.tracks.len().saturating_sub(1));
            self.rank.diff = self.settings.difficulty;
            // CLASSIC lists the original three levels; Beginner is ours
            if self.rank.diff == Difficulty::Beginner && !self.settings.view.modern_menu() {
                self.rank.diff = Difficulty::Easy;
            }
        }
        self.rank.laps = RANK_LAPS.iter().position(|&l| self.settings.laps <= l).unwrap_or(RANK_LAPS.len() - 1);
        // REMASTER the league tab opens on Total (the mockup's entry state)
        if self.settings.view.modern_menu() {
            self.rank.division = RANK_DIVISIONS.len() - 1;
        }
    }

    fn rankings_key(&mut self, up: bool, down: bool, left: bool, right: bool, leave: bool) {
        let rows = self.rank_rows();
        if up || down {
            Self::cycle(&mut self.sel, rows.len(), down);
        }
        if left || right {
            let v = &mut self.rank;
            match rows[self.sel.min(rows.len() - 1)] {
                // CLASSIC the list is clamped (0x4664df); right of "Single race" is the league view
                0 => v.kind = if right { (v.kind + 1).min(RANK_LEAGUE) } else { v.kind.saturating_sub(1) },
                1 if v.kind == RANK_LEAGUE => Self::cycle(&mut v.division, RANK_DIVISIONS.len(), right),
                1 => Self::cycle(&mut v.track, self.tracks.len(), right),
                2 => Self::cycle(&mut v.laps, RANK_LAPS.len(), right),
                _ => v.diff = next_difficulty(v.diff, right, self.settings.view.modern_menu()),
            }
            self.sel = self.sel.min(self.rank_rows().len() - 1);
            self.load_rank_table();
        }
        if leave {
            self.go_sel(Screen::Main, Some(2)); // rankings
        }
    }

    fn load_rank_table(&mut self) {
        let v = self.rank;
        let Some(tr) = self.tracks.get(v.track) else { return };
        self.rank_table = Some(self.rank_table_of(Self::rank_kind(&v), &tr.name, RANK_LAPS[v.laps], v.diff).0);
        // REMASTER the circuit cards: the same filters over every track (every division in the league tab)
        self.rank_slots = if self.settings.view.modern_menu() { self.rank_summaries() } else { Vec::new() };
    }

    /// The table of `kind` for a track, laps and difficulty, and the built-in rows it starts from.
    fn rank_table_of(&self, kind: hiscore::Kind, track: &str, laps: u32, diff: Difficulty) -> (hiscore::Table, hiscore::Table) {
        let name = hiscore::file_name(kind, track, laps, diff, false);
        let scale = match kind {
            hiscore::Kind::BestLap => 1,
            k if k.is_league() => 100, // 0x4470d8
            _ => laps,
        };
        let dir = self.hiscore_dir.clone().or_else(|| hiscore::dir(self.persist));
        (hiscore::Table::load(&self.root, dir.as_deref(), &name, scale), hiscore::Table::defaults(&self.root, scale))
    }

    /// Statistics: fold the race so far into `stats.cfg`. `counted`: the race was played to
    /// the results against rivals (races, wins, podiums, clean); otherwise only what was
    /// driven counts. Does nothing for a watched replay.
    fn commit_stats(&mut self, counted: bool) {
        let Some(r) = &mut self.race else { return };
        let Some(h) = r.sess.race.racers.iter().position(|x| x.human) else { return };
        if r.watching() || r.run.d.race_s <= 0.0 {
            return;
        }
        let mut run = std::mem::take(&mut r.run);
        run.best_lap_ms = r.sess.race.racers[h].progress.best_lap_ms;
        let place = counted.then(|| r.sess.race.standings().iter().position(|&s| s == h)).flatten();
        let track = self.tracks[r.track].name.clone();
        let car = self.cars.get(self.settings.car).map_or(0, |c| c.id);
        self.stats.add(&run, &track, car, place);
        self.save_stats();
    }

    fn save_stats(&self) {
        let Some(path) = stats::path(self.persist) else { return };
        if let Err(e) = self.stats.save(&path) {
            eprintln!("stats {}: {e}", path.display());
        }
    }

    /// The game is closing: the race in progress counts as driven, then the totals are written.
    pub fn shutdown(&mut self) {
        self.commit_stats(false);
        self.save_stats();
    }

    /// The race is over (`0x46e66a`): a finished race goes into BR (and SR in a single race),
    /// the best lap into BL, each when it makes the top ten.
    fn enter_hiscores(&mut self) {
        self.results_ranks.clear();
        // Knockout (our mode) has no tables
        if self.game_type == GameType::Knockout {
            return;
        }
        let Some(r) = &self.race else { return };
        let Some(me) = r.sess.race.racers.iter().find(|x| x.human) else { return };
        let p = &me.progress;
        let tr = &self.tracks[r.track];
        let s = &self.settings;
        // without a HiScores folder (a headless shot) the tables are only worked out for the Results screen
        let dir = hiscore::dir(self.persist);
        let entry = hiscore::Entry { name: s.player_name.clone(), car: self.cars[s.car].name.clone(), lap_ms: p.best_lap_ms.unwrap_or(0), race_ms: p.finished_ms.unwrap_or(0), ..hiscore::Entry::default() };
        let mut todo = Vec::new();
        if let Some(t) = p.finished_ms {
            if self.game_type == GameType::Single {
                todo.push((hiscore::Kind::SingleRace, t, false));
            }
            todo.push((hiscore::Kind::BestRace, t, false));
        }
        if let Some(t) = p.best_lap_ms {
            todo.push((hiscore::Kind::BestLap, t, true));
        }
        let mut ranks = Vec::new();
        for (kind, t, lap) in todo {
            let name = hiscore::file_name(kind, &tr.name, p.laps, s.difficulty, false);
            let scale = if lap { 1 } else { p.laps };
            let before = hiscore::Table::load(&self.root, dir.as_deref(), &name, scale);
            let mut table = before.clone();
            let at = table.place(t, lap);
            table.insert(at, entry.clone());
            if let Some(dir) = &dir {
                if let Err(e) = table.save(dir, &name) {
                    eprintln!("hiscores: {e}");
                }
            }
            ranks.push(hiscore::RankView::new(kind, lap, at, &before, &table, &hiscore::Table::defaults(&self.root, scale)));
        }
        // REMASTER Results: the Records tab lists the tables best lap first (the order the mockup has)
        ranks.sort_by_key(|v| match v.kind {
            hiscore::Kind::BestLap => 0,
            hiscore::Kind::BestRace => 1,
            _ => 2,
        });
        self.results_ranks = ranks;
    }

    pub fn update(&mut self, dt: f32, keys: &HashSet<KeyCode>) {
        self.t += dt;
        // CLASSIC 0x46284c: the clock counts down while leaving; the frame after it reaches 0 does the switch
        if let Some(l) = &mut self.leave {
            if l.ms <= 0.0 {
                let l = *l;
                self.switch(l.to, l.sel);
            } else {
                l.ms = (l.ms - dt * 1000.0).max(0.0);
            }
        }
        self.credits_keys = [KeyCode::ControlLeft, KeyCode::AltLeft, KeyCode::AltRight].iter().all(|k| keys.contains(k));
        self.stats.play_s += dt as f64;
        self.clock += dt;
        // the attract demo (#210): the main menu left alone starts an AI race. Every other screen puts the count and the
        // 120 s limit back (0x46fb8d sets the timeout on entering the main dial, 1e9 elsewhere).
        if self.screen == Screen::Main && !self.attract {
            if self.attract_idle && self.confirm_exit.is_none() && self.keep_display.is_none() && self.loading.is_none() {
                self.idle_s += dt;
                if self.idle_s >= self.idle_limit {
                    self.start_attract();
                }
            } else {
                self.idle_s = 0.0;
            }
        } else if !self.attract {
            (self.idle_s, self.idle_limit) = (0.0, ATTRACT_IDLE_S);
        }
        let holo = self.holo_screen();
        if let (true, Loaded::MenuDrive, Some(d)) = (holo, self.loaded, &mut self.menu_drive) {
            d.sess.step(dt.min(0.05), Input::default());
            d.pose(dt);
        }
        if !matches!(self.screen, Screen::Race | Screen::Pause | Screen::PauseGfx | Screen::PauseSfx | Screen::PauseHud | Screen::PauseControls | Screen::PauseCam) {
            self.menu_fx.update(dt);
        }
        // CLASSIC 0x4668ba: the difficulty word rolls one `emh` frame per menu frame towards its value
        if self.screen == Screen::Options {
            // Beginner has no word: the Easy frame
            let want = (self.settings.difficulty as usize % 3 * 8) as f32;
            let step = DIFF_ROLL_RATE * dt;
            self.diff_roll += (want - self.diff_roll).clamp(-step, step);
        }
        // CLASSIC 0x46c446 / 0x46ce3a: the transmission and on / off words roll one frame per menu frame
        if matches!(self.screen, Screen::Car | Screen::Track) {
            let step = MENU_FPS * dt;
            let wants = self.roll_targets();
            for (r, want) in self.rolls.iter_mut().zip(wants) {
                *r += (want - *r).clamp(-step, step);
            }
        }
        if self.screen == Screen::Sound && !self.settings.view.modern_menu() && self.leave.is_none() {
            self.sound_hold(dt, keys);
            self.sound_step(dt);
        }
        // REMASTER Songs tab: the preview's clock; a song that is not looped ends the preview
        if self.screen == Screen::Sound && self.settings.view.modern_menu() {
            if let Some(cd) = self.cd_preview {
                self.preview_t += dt;
                let len = self.cd_lengths.get(&cd).copied().unwrap_or(0) as f32;
                if len > 0.0 && self.preview_t >= len {
                    if self.settings.cd_loop {
                        self.preview_t -= len;
                    } else {
                        self.sound_stop_preview();
                    }
                }
            }
        }
        // REMASTER pause menu: Restart / Exit run when Enter (A on a pad) has been held for HOLD_S
        if self.screen == Screen::Pause && self.settings.view.modern_menu() {
            let down = keys.contains(&KeyCode::Enter) || keys.contains(&KeyCode::NumpadEnter) || self.controls.pad_down(gilrs::Button::South);
            if self.pause_hold.step(dt, down) {
                self.sfx.push(Sfx::Menu(ev::SELECT));
                if self.sel == 1 {
                    self.restart_race();
                } else {
                    self.exit_race();
                }
            }
        } else {
            self.pause_hold.clear();
        }
        // sweep the dial needle along the shorter way round
        let target = self.needle_target();
        let mut d = target - self.needle;
        while d > std::f32::consts::PI {
            d -= std::f32::consts::TAU;
        }
        while d < -std::f32::consts::PI {
            d += std::f32::consts::TAU;
        }
        self.needle += d * (1.0 - (-NEEDLE_RATE * dt).exp());
        // CLASSIC 0x481e64: a += (target - a) * frame ms * 32 / 3000, no wrap
        self.needle_q += (self.needle_target_q() - self.needle_q) * (1.0 - (-CLASSIC_NEEDLE_RATE * dt).exp());
        match self.screen {
            Screen::Car => self.pic.step(self.settings.car, dt),
            Screen::Track => {
                self.pic.step(self.settings.track, dt);
                self.roll_laps(dt);
            }
            _ => {}
        }
        // REMASTER: the Car screen's garage is built ahead, on the screens that lead to it
        if self.prefetch && self.settings.view.mode == RenderMode::Remaster {
            self.collect_garage(false);
            if matches!(self.screen, Screen::Single | Screen::Track | Screen::League | Screen::Replays | Screen::Options) {
                self.prefetch_garage();
            }
        }
        if self.screen == Screen::Car {
            let k = 1.0 - (-GAUGE_RATE * dt).exp();
            let target = gauge_stats(&self.cars[self.settings.car]);
            for (g, t) in self.gauges.iter_mut().zip(target) {
                *g += (t - *g) * k;
            }
        }
        if let Some(kd) = &mut self.keep_display {
            kd.left -= dt;
            if kd.left <= 0.0 {
                self.finish_keep(false);
            }
        }
        // pad rumble (#383): `drive` fills it in during a race; every other frame stops the motors
        self.rumble_cmd = (0.0, 0.0);
        match self.screen {
            Screen::Loading if self.t > 0.05 => self.poll_load(),
            Screen::Analysis => self.ana_poll(),
            Screen::Race => self.drive(dt, keys),
            Screen::Intro | Screen::LeagueMovie => {
                let done = self.movie.as_mut().is_none_or(|m| {
                    m.update(dt);
                    m.finished()
                });
                if done {
                    self.next_movie();
                }
            }
            _ => {}
        }
        if self.screen != Screen::Race {
            self.rumble.reset();
        }
        self.controls.set_rumble(self.rumble_cmd.0, self.rumble_cmd.1);
    }

    fn drive(&mut self, dt: f32, keys: &HashSet<KeyCode>) {
        if self.race.is_none() {
            return;
        }
        // REMASTER (#346): demo play has the playback controls on the Race line HUD; Classic keeps the original's demo play
        let pb_ui = self.settings.view.modern_hud() && self.race.as_ref().is_some_and(|r| r.watch.is_some() && r.pb.is_some());
        // demo play: the list comes back once "End of demo play!" has been on screen (REMASTER: when BACK TO THE LIST is chosen)
        if self.race.as_ref().is_some_and(|r| r.watch.is_some() && if pb_ui { r.pb.as_ref().is_some_and(|p| p.leave) } else { r.demo_over && r.notice.is_none() }) {
            self.race = None;
            self.go(Screen::Replays);
            return;
        }
        let mut c = self.controls.state(keys);
        // REMASTER demo play (#352): a click on the controls is not also a mouse-bound task (Change camera, ...)
        if pb_ui && self.race.as_ref().and_then(|r| r.pb.as_ref()).is_some_and(|p| p.mouse.hover.is_some() || p.mouse.press.is_some()) {
            let lay = self.controls.layout().bind;
            for (t, bound) in lay.iter().enumerate() {
                if matches!(bound, Some(Code::MouseButton(_))) {
                    c.went_down[t] = false;
                }
            }
        }
        // the pad's D-pad up / down change the speed of the playback: they are not camera views then
        if pb_ui && (self.controls.pad_down(gilrs::Button::DPadUp) || self.controls.pad_down(gilrs::Button::DPadDown)) {
            for t in [task::IN_CAR_VIEW, task::OUT_CAR_VIEW, task::CAMERA_VIEW] {
                c.went_down[t] = false;
            }
        }
        // Tab (Info) steps InfoDetail None -> Selective -> All [I: the binding's handler is
        // not traced; InfoDetail is the setting it names]; Custom is our fourth step (#181)
        if c.went_down[task::INFO] {
            Self::cycle(&mut self.settings.info_detail, INFO_LEVELS.len(), true);
            self.save_settings();
        }
        // the layout's Game menu control (Esc is handled as a key); bound elsewhere it opens the pause menu too
        if c.went_down[task::GAME_MENU] && self.controls.layout().bind[task::GAME_MENU] != Some(Code::Key(KeyCode::Escape)) && self.screen == Screen::Race {
            self.go(Screen::Pause);
            return;
        }
        if std::mem::take(&mut self.cam_dirty) {
            self.save_settings();
        }
        let free_look = self.free_look.unwrap_or_else(|| self.controls.right_stick());
        let Some(r) = &mut self.race else { return };
        // sticks and the mouse steer proportionally; keys use the game's half-lock ramp. The original also
        // ramps a stick at full lock like a key (0x45bf34); not ported [design choice, #377]
        let (l, rt) = (c.value[task::TURN_LEFT], c.value[task::TURN_RIGHT]);
        let steer = if c.analog[task::TURN_LEFT] || c.analog[task::TURN_RIGHT] { rt - l } else { self.steer.update(l > 0.5, rt > 0.5, dt) };
        // CLASSIC 0x45a6e8 / 0x45be6a (#376): the player's pedals (throttle x1.1, key brake ramp, analog brake x1.1); the AI cars do not go through it
        let (throttle, brake) = self.pedals.update(c.value[task::ACCELERATE], c.value[task::BRAKE], r.sess.race.started(), dt);
        let input = Input {
            throttle,
            brake,
            steer,
            shift_up: c.held(task::GEAR_UP),
            shift_down: c.held(task::GEAR_DOWN),
            handbrake: c.held(task::HANDBRAKE),
            hold: false,
        };
        r.sess.look_back = c.held(task::REAR_VIEW);
        r.sess.free_look = if self.settings.cam.free_look { free_look } else { Vec2::ZERO };
        // CLASSIC: the original's camera has no wall pull-in and one easing (0x42c540);
        // REMASTER: both are options (Wall pull-in, Chase follow rows)
        let remaster = self.settings.view.mode == RenderMode::Remaster;
        r.sess.pull_in = remaster && self.settings.cam.pull_in;
        // chase easing: Normal is the original's, Loose and Tight scale it [design choice]
        let follow = if remaster { self.settings.cam.follow.min(2) } else { 1 };
        let (ease, lag) = [(0.5, 0.5), (1.0, 0.3), (1.8, 0.15)][follow];
        // on top of the tuned easing and lag (`CHASE_K_XZ`, `CHASE_K_Y`, `CHASE_MAX_LAG`; the Constants page, tune.cfg)
        r.sess.follow = motorview::drive::Follow { ease, lag: (follow != 1).then_some(lag) };
        if c.went_down[task::HORN] && !pb_ui {
            self.sfx.push(Sfx::Horn);
        }
        // F1 in-car, F2 outside (Remaster flips between our two chase distances), F3 the TV camera
        // (a track without camera stations keeps the view); on the goal camera the player-cycle
        // key switches to the TV camera on the next car (0x45aa45)
        // the camera keys are read only from the "3" on (0x45a868) and then end the start fly-by (0x45a8d7)
        let cam_keys = r.sess.race.pre_race_s() >= startcam::HOLD_S || !r.sess.flying();
        let cam_before = r.sess.cam;
        if cam_keys && [task::CAMERA_VIEW, task::CHANGE_CAMERA, task::IN_CAR_VIEW, task::OUT_CAR_VIEW].iter().any(|&t| c.went_down[t]) {
            r.sess.fly_by = false;
        }
        if c.went_down[task::CAMERA_VIEW] && !r.sess.race.tv_camera.is_empty() && cam_keys {
            r.sess.cam = CamMode::Tv;
        }
        if c.went_down[task::CYCLE_PLAYERS] && r.sess.cam == CamMode::Goal && !r.sess.race.tv_camera.is_empty() {
            r.sess.cam = CamMode::Tv;
        }
        // our camera change key / button (V, pad B by default): the next view in the cycle
        if c.went_down[task::CHANGE_CAMERA] && cam_keys {
            // REMASTER demo play (#346): the cycle goes on to the TV camera, on a track that has one
            r.sess.cam = if pb_ui { playback::next_camera(r.sess.cam, !r.sess.race.tv_camera.is_empty()) } else { r.sess.cam.next() };
        }
        if c.went_down[task::IN_CAR_VIEW] && cam_keys {
            r.sess.cam = CamMode::InCar;
        }
        if c.went_down[task::OUT_CAR_VIEW] && cam_keys {
            // CLASSIC [O, #274 trace]: F2 always selects the chase view, a second press changes nothing;
            // REMASTER: a second press flips to our far chase (the original has only one distance)
            r.sess.cam = if remaster && r.sess.cam == CamMode::Chase { CamMode::FarChase } else { CamMode::Chase };
        }
        // design choice (#419): the camera picked in a race is the start camera from then on (kept in the settings)
        if r.sess.cam != cam_before && !pb_ui && !r.watching() {
            if let Some(i) = CamOpts::start_of(r.sess.cam).filter(|&i| i != self.settings.cam.start) {
                self.settings.cam.start = i;
                // written next frame (`r` borrows the race here)
                self.cam_dirty = true;
            }
        }
        if c.went_down[task::REAR_MIRROR] {
            // saved with the next settings save
            self.settings.cam.mirror = !self.settings.cam.mirror;
        }
        // our rescue: back on the racing line a few nodes on (bindable, R by default)
        if c.went_down[task::RESPAWN] && !r.watching() {
            // not before the start (#301): then nothing is recorded either
            let slot = r.sess.focus;
            if r.sess.respawn_focus() {
                r.pending_respawn = Some(slot);
            }
        }
        let delay = self.settings.checkpoint_delay as f32;
        if c.went_down[task::CYCLE_PLAYERS] {
            r.sess.cycle_focus();
            let p = &r.sess.race.racers[r.sess.focus].progress;
            (r.last_lap, r.last_cp, r.lap_flash, r.split, r.sector_flash) = (p.lap, p.last_checkpoint, None, None, None);
        }
        (r.sess.fx.smoke, r.sess.fx.sparks) = (self.settings.view.smoke, self.settings.view.sparks);
        let (scale, feel) = self.cheats.car_size();
        r.sess.race.set_car_scale(scale, feel); // CHEAT gulliver
        r.sess.fx.remaster = self.settings.view.mode == RenderMode::Remaster;
        // playback: the recorded frame replaces the keys and the frame length; past the last
        // frame the race is over and the list comes back
        // F8 freeze: no step; the camera keys (V, F1, F2, look back) and free look still work
        if self.frozen {
            r.sess.step_camera(dt);
            r.cam = r.sess.camera();
            return;
        }
        let (dt, input, hand) = match &mut r.playback {
            Some((rec, idx)) => match rec.frames.get(*idx) {
                Some(f) => {
                    *idx += 1;
                    if let Some(slot) = f.respawn {
                        r.sess.race.respawn(slot);
                    }
                    if f.skip {
                        r.sess.skip_intro();
                    }
                    (f.dt, f.input, f.hand)
                }
                None => {
                    self.race = None;
                    self.go(Screen::Replays);
                    return;
                }
            },
            // demo play: no keys drive the posed car
            None if r.watch.is_some() => (dt, Input::default(), None),
            None => (dt, input, None),
        };
        if let Some(rec) = &mut r.recording {
            rec.push(dt, input, r.pending_respawn.take(), std::mem::take(&mut r.pending_skip));
        }
        let player = r.sess.race.racers.iter().position(|x| x.human).unwrap_or(r.sess.focus);
        let was_finished = r.sess.race.racers[player].progress.finished_ms.is_some();
        // demo play: the recorded player is a remote racer, posed from the stream (the camera, HUD and
        // sound follow it as any car)
        if let (true, Some(block), Some(d), Some(pb)) = (pb_ui, r.watch, r.demo.as_ref(), r.pb.as_mut()) {
            // REMASTER (#346): the playback controls move the file's clock (forward, held, a seek) and pose the car
            let mut held = playback::held_acts(keys, &|b| self.controls.pad_down(b));
            // the pointer: its press on a control is that control's act, a drag on the bar or diagram a seek (#352)
            let ms = motor_sim::demo::race_ms(&r.sess.race);
            let hits = match self.pointer {
                Some((_, cw)) => crate::hud_transport::targets(menu_modern::Glass::hud(cw, 1.0), &pb.view(ms), playback::cam_label(r.sess.cam)),
                None => Vec::new(),
            };
            let scrub = pb.mouse(&hits, self.pointer.map(|p| p.0), self.controls.mouse_down(0), &mut held);
            let seek = scrub.or(pb.input(&held, dt, ms));
            let jumped = pb.frame(&mut r.sess, d, block, player, dt, seek);
            pb.ease(dt, true);
            if std::mem::take(&mut pb.mouse.camera) && cam_keys {
                r.sess.fly_by = false;
                r.sess.cam = playback::next_camera(r.sess.cam, !r.sess.race.tv_camera.is_empty());
            }
            if jumped {
                r.resync();
                // the goal camera belongs to the finish: a seek to before it gives the chosen camera back
                if r.sess.cam == CamMode::Goal && r.sess.race.racers[player].progress.finished_ms.is_none() {
                    r.sess.cam = self.settings.cam.mode(!r.sess.race.tv_camera.is_empty());
                }
            }
        } else {
            r.sess.step_as(dt, input, hand);
            if let (Some(block), Some(d)) = (r.watch, &r.demo) {
                // #374: the recorded car is posed at the view time (the other cars are drawn up to a tick behind the clock)
                if let Some(s) = d.state(block, motor_sim::demo::view_ms(&r.sess.race)) {
                    motor_sim::demo::drive_remote(&mut r.sess.race.racers[player], &s, dt, &r.sess.race.world.materials);
                }
            }
        }
        if let Some(n) = &mut r.notice {
            n.0 -= dt;
            if n.0 <= 0.0 {
                r.notice = None;
            }
        }
        if let (true, Some(pb)) = (pb_ui, r.pb.as_ref()) {
            // REMASTER (#346): the notice stays up until the choice at the end is made; a seek back takes it off
            if pb.ended {
                r.demo_over = true;
                r.notice = Some((4.0, "End of demo play!".into()));
            } else if r.demo_over {
                (r.demo_over, r.notice) = (false, None);
            }
        } else if let Some(d) = &r.demo {
            let ms = motor_sim::demo::race_ms(&r.sess.race);
            if !r.demo_over && d.mde.records.last().is_none_or(|l| ms > l.ms as f64) {
                r.demo_over = true;
                r.notice = Some((4.0, if r.watch.is_some() { "End of demo play!" } else { "End of ghost play!" }.into()));
            }
        }
        if let Some(rec) = &mut r.demo_rec {
            let cars: Vec<_> = r.sess.race.racers.iter().map(|x| (&x.car, x.brake)).collect();
            rec.push(motor_sim::demo::race_ms(&r.sess.race), &cars, &r.sess.race.world.materials);
        }
        if !r.watching() {
            r.count_run(dt, player);
            // rumble (#383): the pad follows the player's own car only
            if let Some(i) = r.rumble_in(player) {
                self.rumble_cmd = self.rumble.mix(dt, &i, self.rumble_gain);
            }
        }
        // Time Attack ghost: sample the lap; a completed lap that beats the ghost replaces it and
        // is saved at once (our own feature, design choice)
        if let Some(rec) = &mut r.lap_rec {
            let me = &r.sess.race.racers[player];
            let p = &me.progress;
            let done = p.finished_ms.map_or(p.lap - 1, |_| p.laps);
            if done > r.ghost_laps {
                r.ghost_laps = done;
                if let Some(ms) = p.last_lap_ms {
                    let lap = rec.finish(ms, dt, &me.car);
                    if r.ghost.as_ref().is_none_or(|g| ms < g.lap_ms) {
                        let (car, name) = (self.cars[self.settings.car].id, &self.tracks[r.track].name);
                        if let Some(path) = hiscore::ghost_path(self.persist, name, car) {
                            if let Err(e) = lap.save(&path, car, name) {
                                eprintln!("ghost: {e}");
                            }
                        }
                        r.ghost = Some(lap);
                    }
                }
            } else if p.finished_ms.is_none() {
                rec.push(r.sess.race.clock_ms - p.lap_start_ms as f64, dt, &me.car);
            }
        }
        // CLASSIC: demo races alternate 10 s in-car and 10 s TV camera from the start
        // (period 20 000 ms, 0x45a4f1..0x45bd5c)
        if (self.demo || self.attract) && !r.sess.race.tv_camera.is_empty() && r.sess.cam != CamMode::Goal {
            let (now, was) = (r.sess.race.clock_ms, r.sess.race.clock_ms - dt as f64 * 1000.0);
            if (now / 10_000.0).floor() != (was / 10_000.0).floor() && now >= 0.0 {
                r.sess.cam = if (now / 10_000.0).floor() as i64 % 2 == 0 { CamMode::InCar } else { CamMode::Tv };
            }
        }
        // CLASSIC 0x45a490: the attract demo posts "DEMO" while the clock's ms % 500 > 250 (blinks at 2 Hz)
        if self.attract {
            r.notice = (r.sess.race.clock_ms.rem_euclid(500.0) > 250.0).then(|| (1.0, "DEMO".to_string()));
        }
        // CLASSIC: the player's finish cuts to the goal camera until the results (gameplay
        // videos [O]; the camera keys still switch away from it)
        if !was_finished && r.sess.race.racers[player].progress.finished_ms.is_some() {
            r.sess.cam = CamMode::Goal;
        }
        // Knockout: knocked out, you watch the leader
        if r.sess.race.racers[player].progress.eliminated_ms.is_some() && r.sess.focus == player && !(self.demo || self.attract) {
            r.sess.focus = r.sess.race.standings()[0];
        }
        r.cam = r.sess.camera();
        let me = &r.sess.race.racers[r.sess.focus];
        let p = &me.progress;
        let mut new_lap = false;
        if p.lap != r.last_lap || p.finished_ms.is_some() && r.finished_for.is_none() {
            if let Some(l) = p.last_lap_ms {
                // REMASTER: the Modern HUD's lap message has its own time (HudLapDelay); CLASSIC keeps CheckpointDelay.
                // The split just before the line goes, or it would show again when the lap message ends
                let (secs, modern) = if self.settings.view.modern_hud() { (self.settings.hud.lap_delay as f32, true) } else { (delay, false) };
                // REMASTER sector delta: the lap against the best earlier lap, taken before the records change
                let delta = (p.lap > 1 || p.finished_ms.is_some()).then(|| r.records.gaps(&lap_splits(&p.splits), p.finished_ms.map_or(p.lap - 1, |_| p.laps) as usize * 4)[0]).flatten();
                r.lap_flash = Some((secs, l, p.best_lap_ms == Some(l), delta));
                new_lap = true;
                if modern {
                    r.split = None;
                    r.sector_flash = None;
                }
            }
            r.last_lap = p.lap;
        }
        // checkpoint passed: show the split and the gaps to everyone through it (0x425b44
        // records the times; the display follows the gameplay video) for CheckpointDelay
        if p.last_checkpoint != r.last_cp {
            if let Some((cp, at)) = p.last_checkpoint {
                let idx = p.lap as usize * 4 + cp as usize + 1;
                r.ta_gaps = r.records.gaps(&lap_splits(&p.splits), idx - 4);
                r.split = Some((delay, idx, at.saturating_sub(p.lap_start_ms)));
                // REMASTER sector delta (#409, design choice [I]): slots lap*4 + 2 / + 3 = checkpoints 1 / 2 = sectors 1 / 2
                if let (2..=3, Some(d)) = (idx % 4, r.ta_gaps[0]) {
                    r.sector_flash = Some((self.settings.hud.sector_delay as f32, (idx % 4) as u32 - 1, d));
                }
            }
            r.last_cp = p.last_checkpoint;
        }
        // the line: the lap's gaps (index of the next lap's start = this lap's end)
        let done = p.finished_ms.map_or(p.lap - 1, |_| p.laps);
        if new_lap && done > 0 {
            r.ta_gaps = r.records.gaps(&lap_splits(&p.splits), done as usize * 4);
        }
        // the player's laps and race go into the track records (0x425b44, every mode)
        if let Some(h) = r.sess.race.racers.iter().find(|x| x.human).filter(|_| !r.watching() && self.game_type != GameType::Knockout) {
            let hp = &h.progress;
            let splits = lap_splits(&hp.splits);
            let done = hp.finished_ms.map_or(hp.lap - 1, |_| hp.laps);
            let mut changed = false;
            while r.rec_laps < done {
                changed |= r.records.lap_done(&splits, r.rec_laps as usize);
                r.rec_laps += 1;
            }
            if let (Some(t), false) = (hp.finished_ms, r.rec_finished) {
                r.rec_finished = true;
                changed |= r.records.race_done(&splits, hp.laps, t);
            }
            if let Some(dir) = hiscore::dir(self.persist).filter(|_| changed) {
                if let Err(e) = r.records.save(&dir, &self.tracks[r.track].name, hp.laps) {
                    eprintln!("records: {e}");
                }
            }
        }
        for timer in [r.lap_flash.as_mut().map(|x| &mut x.0), r.split.as_mut().map(|x| &mut x.0), r.sector_flash.as_mut().map(|x| &mut x.0)].into_iter().flatten() {
            *timer -= dt;
        }
        if r.lap_flash.is_some_and(|x| x.0 <= 0.0) {
            r.lap_flash = None;
        }
        if r.split.is_some_and(|x| x.0 <= 0.0) {
            r.split = None;
        }
        if r.sector_flash.is_some_and(|x| x.0 <= 0.0) {
            r.sector_flash = None;
        }
        crate::proximity::update(r, dt, self.settings.display.size.0 as f32 / self.settings.display.size.1.max(1) as f32);
        crate::hud::map_zoom_update(r, dt);
        // CLASSIC 0x412c0c: the map eggs test the followed car, only while the minimap is on (RoadMap not None)
        let (me, d) = (&r.sess.race.racers[r.sess.focus], self.settings.display.size);
        r.eggs.update(&self.tracks[r.track].name, me.car.pos, dt, self.settings.road_map != 0, self.egg_text.len(), 480.0 * d.0 as f32 / d.1.max(1) as f32);
        self.club_seen |= r.eggs.active == motor_sim::eggs::Egg::Club;
        if !r.watching() && r.sess.race.racers[player].progress.done() {
            let w = r.finished_for.get_or_insert(0.0);
            *w += dt;
            if *w > RESULTS_AFTER || r.sess.race.all_finished() {
                // the attract demo ran to its end: back to the menu, no results (the next one in 10 s)
                if self.attract {
                    return self.end_attract(true);
                }
                self.results = compute_results(&r.sess, self.game_type);
                self.results_sheet = Some(motor_sim::sheet::RaceSheet::build(&r.sess.race));
                self.save_recording();
                self.enter_hiscores();
                self.commit_stats(self.game_type != GameType::TimeAttack);
                if self.game_type == GameType::League {
                    self.finish_league_race();
                }
                self.go(Screen::Results);
            }
        }
    }

    /// Everything to paint this frame.
    pub fn layers(&self, size: (u32, u32)) -> Vec<Layer> {
        let mut out = self.screen_layers(size);
        out.extend(self.osk_layer(size));
        if let Some(sel) = self.confirm_exit {
            out.push(self.requester_layer(size, Ask::Exit, sel));
        }
        if let Some(sel) = self.ghost_del {
            out.push(self.requester_layer(size, Ask::DeleteDemo, sel));
        }
        // the original's strings (0x4d607a..0x4d60fb); New's second button is ours
        match self.league_req {
            Some((false, sel)) => out.push(self.requester_layer(size, Ask::CreateLeague, sel)),
            Some((true, sel)) => out.push(self.requester_layer(size, Ask::DeleteLeague, sel)),
            None => {}
        }
        if matches!(self.editing, Some((2, _))) {
            // CLASSIC 0x46bc68: a text field in the New league screen; ours is a requester over the dial [design choice]
            let cursor = if ((self.clock * 2.0) as u32).is_multiple_of(2) { "_" } else { " " };
            if self.modern_requester() {
                // REMASTER: a text field with a caret (a pad types on the keyboard sheet, `osk_layer`)
                if self.osk.is_none() {
                    let canvas = Canvas::hud(size);
                    let px = 1.0 / canvas.transform(size).0;
                    out.push(self.ui_in(Batch::sharp_text(&self.assets, canvas, 1.0), |g, b| {
                        g.requester_input_modern(b, "Name your league:", &self.league_name, &[(&[2], "OK"), (&[3], "RANDOM")], px)
                    }));
                }
            } else {
                let line = format!("{}{cursor}", self.league_name);
                out.push(self.text_requester_layer(["Name your league:", &line], ["Enter: ok", "Esc: cancel"], 0));
            }
        }
        if let Some(ds) = &self.demo_save {
            let cursor = if ((self.clock * 2.0) as u32).is_multiple_of(2) { "_" } else { " " };
            match &ds.name {
                None => out.push(self.requester_layer(size, Ask::SaveRace, ds.sel)),
                Some(n) if self.modern_requester() => {
                    if self.osk.is_none() {
                        let canvas = Canvas::hud(size);
                        let px = 1.0 / canvas.transform(size).0;
                        out.push(self.ui_in(Batch::sharp_text(&self.assets, canvas, 1.0), |g, b| {
                            g.requester_input_modern(b, "Name the recorded race:", n, &[(&[2], "SAVE"), (&[3], "DON'T SAVE")], px)
                        }));
                    }
                }
                Some(n) => {
                    let line = format!("{n}{cursor}");
                    out.push(self.text_requester_layer(["Name the recorded race:", &line], ["Enter: save", "Esc: cancel"], 0));
                }
            }
        }
        if let Some(kd) = self.keep_display {
            out.push(self.requester_layer(size, Ask::KeepDisplay(kd.left.ceil() as u32), kd.sel));
        }
        out
    }

    /// Whether a requester is open (the Glass screens under it leave their hint rows to its own).
    pub(super) fn asking(&self) -> bool {
        self.keep_display.is_some() || self.confirm_exit.is_some() || self.ghost_del.is_some() || self.league_req.is_some() || self.demo_save.is_some() || matches!(self.editing, Some((2, _))) || self.osk.is_some()
    }

    /// Whether requesters are drawn in the Glass HUD (REMASTER menu style, on every screen).
    fn modern_requester(&self) -> bool {
        self.settings.view.modern_menu()
    }

    /// A two-button question over the screen: the Glass requester (REMASTER menu style), the
    /// MENU.SPR one otherwise.
    fn requester_layer(&self, size: (u32, u32), ask: Ask, sel: usize) -> Layer {
        if self.modern_requester() {
            let canvas = Canvas::hud(size);
            let px = 1.0 / canvas.transform(size).0;
            self.ui_in(Batch::sharp_text(&self.assets, canvas, 1.0), |g, b| g.requester_modern(b, ask, sel, px))
        } else {
            let lines = ask.classic();
            self.text_requester_layer([&lines[0], &lines[1]], ask.buttons(), sel)
        }
    }

    /// The MENU.SPR requester with these lines and buttons (the Classic look; also the text entry's).
    fn text_requester_layer(&self, lines: [&str; 2], buttons: [&str; 2], sel: usize) -> Layer {
        self.ui(self.menu_canvas(), |_, b| Self::requester(b, lines, buttons, sel))
    }

    /// The quit requester (MENU.SPR `req`: warning sign, message panel, "options" panel)
    /// with the original's texts at 0x4cf8.. ("Exit to windows." becomes "Exit to system.").
    /// The keep-display-settings question reuses it (design choice).
    fn requester(b: &mut Batch, lines: [&str; 2], buttons: [&str; 2], sel: usize) {
        b.rect(0.0, 0.0, 640.0, 400.0, [0.0, 0.0, 0.05, 0.45]);
        let (x, y) = (160.0, 150.0); // 0x462454: the sprite at (slid-in x, 150)
        // the sprite only draws the frame: fill its two panels (and the drop shadow) first
        let navy = [0.04, 0.04, 0.2, 1.0];
        b.rect(x + 20.0, y + 20.0, 300.0, 118.0, [0.0, 0.0, 0.0, 0.9]);
        b.rect(x + 8.0, y + 8.0, 288.0, 102.0, navy);
        b.menu("req", 0, x, y);
        // CLASSIC 0x462454: the message in colour 1 centred at x + 180 from y 166, 12 px per string; the
        // original's text is "line", " ", "line", so a blank row sits between the two (24 px pitch).
        // The list 0x461ec0 (flag 0x800000: colour 1 + 1) at y + 84: each item advances width + 16,
        // the row is centred as a whole (the last gap included); the chosen one is colour 5 (black)
        // on a colour 2 (white) bar, the others colour 2. The second button reads " No way!" [O 0x4d57d0].
        for (i, t) in lines.iter().enumerate() {
            menu_text(b, t, x + 180.0, 166.0 + i as f32 * 24.0, MENU_ORANGE, Align::Centre);
        }
        let buttons = buttons.map(|t| if t == "No way!" { " No way!" } else { t });
        let total: f32 = buttons.iter().map(|t| b.fnt_width(t, 1.0) + 16.0).sum();
        let mut bx = x + 180.0 - (total / 2.0).floor();
        for (i, t) in buttons.iter().enumerate() {
            let w = b.fnt_width(t, 1.0);
            let by = y + 84.0;
            if i == sel {
                menu_bar(b, bx, by, w, MENU_WHITE);
            }
            menu_text(b, t, bx, by, if i == sel { MENU_BLACK } else { MENU_WHITE }, Align::Left);
            bx += w + 16.0;
        }
    }

    fn screen_layers(&self, size: (u32, u32)) -> Vec<Layer> {
        match self.screen {
            Screen::Main | Screen::Single | Screen::Options | Screen::League
                if self.settings.view.modern_menu() =>
            {
                self.dial_modern(size)
            }
            Screen::Main => vec![self.ui(self.menu_canvas(), |g, b| g.main_menu(b))],
            Screen::Single => vec![self.ui(self.menu_canvas(), |g, b| g.single_menu(b))],
            Screen::Track if self.settings.view.modern_menu() => self.track_modern(size),
            Screen::Track => vec![self.ui(self.menu_canvas(), |g, b| g.track_screen(b))],
            Screen::Car if self.settings.view.modern_menu() => self.car_modern(size),
            Screen::Car => self.car_screen(size),
            Screen::Options => vec![self.ui(self.menu_canvas(), |g, b| g.options_menu(b))],
            Screen::Graphics if self.settings.view.modern_menu() => self.graphics_modern(size),
            Screen::Graphics => vec![self.ui(self.menu_canvas(), |g, b| g.graphics_screen(b))],
            Screen::Credits if self.settings.view.modern_menu() => self.credits_modern(size),
            Screen::Credits => vec![self.ui(self.menu_canvas(), |g, b| g.credits_screen(b))],
            Screen::Intro | Screen::LeagueMovie => vec![self.ui(Canvas::MOVIE, |g, b| {
                if let Some((x, y, w, h)) = g.movie.as_ref().filter(|_| g.movie_shown.is_some()).map(Movie::rect) {
                    b.image_scaled("movie", x, y, w, h, WHITE);
                }
            })],
            Screen::Loading if self.settings.view.modern_menu() => self.loading_modern(size),
            Screen::Loading => vec![self.ui(self.menu_canvas(), |g, b| g.loading_screen(b))],
            Screen::Sound if self.settings.view.modern_menu() => self.sound_modern(size),
            Screen::Sound => vec![self.ui(self.menu_canvas(), |g, b| g.sound_screen(b))],
            Screen::Controls if self.settings.view.modern_menu() => self.controls_modern(size),
            Screen::Controls => vec![self.ui(self.menu_canvas(), |g, b| g.controls_screen(b))],
            Screen::Personal if self.settings.view.modern_menu() => self.personal_modern(size),
            Screen::Personal => vec![self.ui(self.menu_canvas(), |g, b| g.personal_screen(b))],
            Screen::Statistics => vec![self.ui(self.menu_canvas(), |g, b| g.statistics_screen(b))],
            Screen::Race | Screen::Pause | Screen::PauseGfx | Screen::PauseSfx | Screen::PauseHud | Screen::PauseControls | Screen::PauseCam => {
                let Some(r) = &self.race else { return Vec::new() };
                let mut parts = self.race_parts(r);
                // REMASTER: the Glass pause menu frosts the race behind its cards (#300)
                parts.frost = (self.screen != Screen::Race && self.settings.view.modern_menu()).then(|| self.pause_frost(size));
                let mut out = vec![Layer::Scene { cam: r.cam, parts: Box::new(parts), viewport: None, mirror: false }];
                let canvas = Canvas::hud(size);
                let paused = self.screen != Screen::Race;
                // REMASTER: the Glass pause menu shows the numbers itself, so the HUD and the mirror strip are hidden
                let glass_pause = paused && self.settings.view.modern_menu();
                if self.settings.cam.mirror && !r.sess.look_back && !r.sess.flying() && !glass_pause {
                    out.extend(self.mirror_layers(r, canvas, size));
                }
                if !glass_pause {
                    let px = 1.0 / canvas.transform(size).0;
                    out.push(self.ui_in(Batch::new_for(&self.assets, canvas, self.settings.view.modern_hud()), |g, b| hud::draw(g, r, b, paused, px)));
                }
                if glass_pause {
                    // REMASTER: the tabbed glass pause menu over the race (#300)
                    let px = 1.0 / canvas.transform(size).0;
                    out.push(self.ui_in(Batch::sharp_text(&self.assets, canvas, 1.0), |g, b| g.pause_modern(b, r, px)));
                } else if self.screen == Screen::PauseGfx {
                    out.push(self.ui(canvas, |_, b| b.rect(0.0, 0.0, b.canvas.w, 480.0, [0.0, 0.0, 0.08, 0.3])));
                    out.push(self.ui(self.menu_canvas(), |g, b| g.pause_graphics(b)));
                } else if self.screen == Screen::PauseHud {
                    out.push(self.ui(canvas, |g, b| g.pause_hud(b, r)));
                } else if paused {
                    out.push(self.ui(canvas, |g, b| g.pause_overlay(b)));
                }
                out
            }
            Screen::Results if self.settings.view.modern_menu() => self.results_modern(size),
            Screen::Results => vec![self.ui(self.menu_canvas(), |g, b| g.results_screen(b))],
            Screen::Rankings if self.settings.view.modern_menu() => self.rankings_modern(size),
            Screen::Rankings => vec![self.ui(self.menu_canvas(), |g, b| g.rankings_screen(b))],
            Screen::League => vec![self.ui(self.menu_canvas(), |g, b| g.league_menu(b))],
            Screen::LeagueTable if self.settings.view.modern_menu() => self.league_table_modern(size),
            Screen::LeagueTable => vec![self.ui(self.menu_canvas(), |g, b| g.league_table(b))],
            Screen::LeagueLoad if self.settings.view.modern_menu() => self.league_load_modern(size),
            Screen::LeagueLoad => vec![self.ui(self.menu_canvas(), |g, b| g.league_load(b))],
            Screen::LeagueResults if self.settings.view.modern_menu() && self.race.is_some() => self.results_modern(size),
            Screen::LeagueResults => vec![self.ui(self.menu_canvas(), |g, b| g.league_results(b))],
            Screen::Replays if self.settings.view.modern_menu() => self.replays_modern(size),
            Screen::Analysis => self.analysis_modern(size),
            Screen::Replays => vec![self.ui(self.menu_canvas(), |g, b| g.replays_screen(b))],
            Screen::Multi => vec![self.ui(self.menu_canvas(), |g, b| g.multi_screen(b))],
        }
    }

    /// The race scene's parts as the race, the pause menu and the Remaster Results screen draw it.
    pub(super) fn race_parts(&self, r: &RaceState) -> FrameParts {
        // the podium emblem rides on this camera (`frame_parts`), so the mirror does not get it
        let mut parts = r.sess.frame_parts(&r.cam, false, self.settings.skid_marks, self.settings.view.skids_detailed());
        if self.settings.view.shadows && !self.settings.view.car_shadows() {
            parts.shadows = r.sess.shadows_except(&r.casper_slot(self.cheats.casper).into_iter().collect::<Vec<_>>());
            // CLASSIC: a remote car casts no shadow (0x4391ad skips 0x438308) [O, #253]; REMASTER keeps them
            if let Some(d) = r.demo.as_ref().filter(|_| self.settings.view.mode != RenderMode::Classic) {
                d.shadows(&r.sess, motor_sim::demo::view_ms(&r.sess.race), &mut parts.shadows);
            }
        }
        (parts.cars, parts.hidden, parts.ghost) = r.cheat_cars(self.cheats.casper, std::mem::take(&mut parts.cars), std::mem::take(&mut parts.hidden));
        parts
    }

    /// the classic menus' canvas: 640x400, filling the window when the Menu picture option says so
    /// CLASSIC 0x427dc0: the loading picture and the track's facts in FONT1, white, left aligned. The original puts
    /// the labels at x = 370 sx - 60 and the values at 500 sx - 40, the rows at 10 / 30 / 40 / 60 / 70 sy, with
    /// sx, sy = screen / (640, 400) (constants 0x4d29c4..0x4d29e4) [O]. APPROX: the -60 / +20 are unscaled screen px
    /// there; here they follow the canvas (the same at 640 x 400).
    fn loading_screen(&self, b: &mut Batch) {
        b.image("bg:loading", 0.0, 0.0);
        let facts = self.load_facts.as_ref();
        let track = facts.map_or(self.settings.track, |f| f.track);
        let Some(tr) = self.tracks.get(track) else { return };
        let ms = |t: Option<Option<u32>>| fmt_time_cs(t.flatten().unwrap_or(0));
        let rows = [
            ("Name:", tr.name.clone(), 10.0),
            ("Length:", tr.length_text.clone(), 30.0),
            ("Location:", tr.location.clone(), 40.0),
            ("Best laptime:", ms(facts.map(|f| f.best_lap)), 60.0),
            ("Best racetime:", ms(facts.map(|f| f.best_race)), 70.0),
        ];
        for (label, value, y) in rows {
            menu_text(b, label, 310.0, y, 2, Align::Left);
            menu_text(b, &value, 460.0, y, 2, Align::Left);
        }
    }

    fn menu_canvas(&self) -> Canvas {
        Canvas::menu(self.settings.menu_stretch)
    }

    fn ui(&self, canvas: Canvas, draw: impl FnOnce(&Self, &mut Batch)) -> Layer {
        self.ui_in(Batch::new(&self.assets, canvas), draw)
    }

    fn ui_in(&self, mut b: Batch, draw: impl FnOnce(&Self, &mut Batch)) -> Layer {
        draw(self, &mut b);
        let sharp = b.is_sharp();
        Layer::Ui(b.canvas, b.verts, sharp, b.glow)
    }

    /// Paint all layers into `target`.
    pub fn render(&self, gpu: &Gpu, enc: &mut wgpu::CommandEncoder, target: &wgpu::TextureView, depth: &wgpu::TextureView, size: (u32, u32)) {
        let layers = self.layers(size);
        for (i, l) in layers.iter().enumerate() {
            match l {
                Layer::Ui(canvas, verts, sharp, glow) => {
                    let clear = (i == 0).then_some([0.0, 0.0, 0.0]);
                    // REMASTER: the smooth glow goes under the batch's crisp shapes and text
                    let glowing = !glow.is_empty();
                    if glowing {
                        if clear.is_some() {
                            gpu.draw_ui(enc, target, size, *canvas, &[], clear, None);
                        }
                        gpu.draw_glow(enc, target, size, *canvas, glow, self.assets.sdf());
                    }
                    gpu.draw_ui(enc, target, size, *canvas, verts, clear.filter(|_| !glowing), sharp.then(|| self.assets.sdf()));
                }
                Layer::Holo { cam, cars, clip, road } => {
                    let mut f = Frame::new(cars, &[]);
                    (f.overlay, f.hologram, f.holo_road) = (true, true, *road);
                    f.holo_clip = clip.as_deref().copied();
                    gpu.draw(enc, target, depth, size, cam, &self.settings.view, &f);
                }
                Layer::Scene { cam, parts, viewport, mirror } => {
                    let mut f = parts.frame();
                    f.viewport = *viewport;
                    f.mirror = *mirror;
                    gpu.draw(enc, target, depth, size, cam, &self.settings.view, &f);
                }
                Layer::Preview { cam, cars, viewport, paint } => {
                    let mut f = Frame::new(cars, &[]);
                    (f.overlay, f.cars_only, f.viewport, f.paint, f.studio) = (true, true, Some(*viewport), Some(*paint), true);
                    gpu.draw(enc, target, depth, size, cam, &self.settings.view, &f);
                }
            }
            // scenes share the uniform buffers, which take their new values at submit
            if l.is_3d() && layers[i + 1..].iter().any(Layer::is_3d) {
                let done = std::mem::replace(enc, gpu.device.create_command_encoder(&Default::default()));
                gpu.queue.submit([done.finish()]);
            }
        }
    }

    /// F5 rear mirror: the view behind the car, mirrored, in a framed strip at the top centre
    /// of the HUD (size and placement are a design choice; the original's are [U]).
    fn mirror_layers(&self, r: &RaceState, canvas: Canvas, size: (u32, u32)) -> [Layer; 2] {
        const W: f32 = 200.0;
        const H: f32 = 56.0;
        const TOP: f32 = 6.0;
        const BORDER: f32 = 2.0;
        let x = (canvas.w - W) * 0.5;
        let (eye, at) = r.sess.mirror_view();
        let look = (at - eye).normalize_or(Vec3::Z);
        let cam = Camera::new(eye, look.x.atan2(-look.z), look.y.asin());
        let (cars, hidden, ghost) = r.cheat_cars(self.cheats.casper, r.sess.models_hiding(Some(r.sess.focus)), r.sess.hidden_bodies(Some(r.sess.focus)));
        let viewport = Some(canvas.to_window(size, x, TOP, W, H));
        let frame = self.ui(canvas, |_, b| {
            let c = [0.05, 0.05, 0.05, 1.0];
            b.rect(x - BORDER, TOP - BORDER, W + 2.0 * BORDER, BORDER, c);
            b.rect(x - BORDER, TOP + H, W + 2.0 * BORDER, BORDER, c);
            b.rect(x - BORDER, TOP, BORDER, H, c);
            b.rect(x + W, TOP, BORDER, H, c);
        });
        // the env cube and the grass trail are the race view's (no `env_probe`, `trail`)
        let parts = FrameParts { cars, props: r.sess.prop_models(), glows: r.sess.prop_glows(), brakes: r.sess.car_brakes(), spins: r.sess.wheel_spins().to_vec(), motion: r.sess.car_motions(), hidden, ghost, sprites: r.sess.fx.sprites(), tron: r.sess.tron(), light_walls: r.sess.light_walls(), ..Default::default() };
        [Layer::Scene { cam, parts: Box::new(parts), viewport, mirror: true }, frame]
    }

    /// Whether `k` takes a screenshot (the layout's ScreenShot key; not while rebinding).
    pub fn is_screenshot_key(&self, k: KeyCode) -> bool {
        self.capture.is_none() && self.editing.is_none() && self.controls.is_key(task::SCREENSHOT, k)
    }

    // ---------------------------------------------------------------- pieces

    /// Highlight frame of a selected label: the 8 frames brighten in, then hold.
    /// Label frame. Unselected labels use frame 0 (dim, sharp). The selected one pulses
    /// as in the menu video: frame 1 (bright, sharp) for ~0.2 s, a quick blur through frames
    /// 2..6 to 7 (~0.05 s), frame 7 held for ~0.27 s, then quickly sharp again; a cycle is
    /// ~32 frames of the 60 fps capture (0.53 s).
    fn glow(&self, selected: bool) -> usize {
        if !selected {
            return 0;
        }
        const SHARP: f32 = 0.2;
        const RAMP: f32 = 0.05;
        const BLUR: f32 = 0.23;
        let t = self.t % (SHARP + RAMP + BLUR + RAMP);
        let ramp = |u: f32| 1 + (u / RAMP * 6.0).clamp(0.0, 6.0) as usize;
        if t < SHARP {
            1
        } else if t < SHARP + RAMP {
            ramp(t - SHARP)
        } else if t < SHARP + RAMP + BLUR {
            7
        } else {
            8 - ramp(t - SHARP - RAMP - BLUR)
        }
    }

    /// The frames the rolling words rest at: 0 = manual / on, 7 = auto / off.
    fn roll_targets(&self) -> [f32; 3] {
        let f = |on: bool| if on { 0.0 } else { 7.0 };
        [f(self.settings.manual), f(self.settings.record_race), f(self.settings.catchup)]
    }

    /// A label sprite drawn at slot `slot` of sprpos set `set`: its slide effect while the screen
    /// slides, else its pulse (selected), with the drawer's random glitch.
    #[allow(clippy::too_many_arguments)]
    fn label(&self, b: &mut Batch, (set, slot): (usize, usize), name: &str, x: f32, y: f32, selected: bool) {
        let frame = self.glow(selected);
        if !self.slide_sprite(b, (set, slot), name, frame, x, y) {
            b.menu(name, frame, x, y);
        }
    }

    /// CLASSIC 0x460880 while the screen slides (slide != 0): sprite `name` (frame `frame`) of slot `slot`
    /// of sprpos set `set`, at (x, y), with the slot's slide effect (`slide_fx`). False when the screen has
    /// settled: the caller draws the settled look.
    #[allow(clippy::too_many_arguments)]
    fn slide_sprite(&self, b: &mut Batch, (set, slot): (usize, usize), name: &str, frame: usize, x: f32, y: f32) -> bool {
        let slide = self.slide();
        if slide == 0 && self.settings.view.modern_menu() {
            return false;
        }
        let key = format!("m:{name}:{}", frame.min(b.assets.frames(name).max(1) - 1));
        let (w, h) = b.size(&key);
        if slide == 0 {
            // CLASSIC 0x460880 settled: the title / box turn over now and then, any slot glitches at random
            let tick = (self.t * MENU_FPS) as u64;
            let s = self.settle.borrow_mut().frame(slot, tick, h as u32);
            let passes = slide_fx::settled_passes(s, slot, h as u32);
            if passes.is_empty() {
                return false;
            }
            slide_fx::blit(b, &key, x as i32, y as i32, w as i32, &passes);
            return true;
        }
        let (effect, value) = self.sprpos.effect(set, slot);
        // APPROX: the original's rand() and row seed (0x4addf4) run on from draw to draw; ours start from
        // the screen clock and the slot, so a `--shot` is repeatable
        let ms = (self.t * 1000.0) as u32;
        let mut rng = league::Rng(ms.wrapping_mul(0x9e37_79b9) ^ ((set * 32 + slot) as u32).wrapping_mul(0x85eb_ca6b));
        let mut seed = rng.next() << 15 | rng.next();
        let passes = slide_fx::passes(effect, value, slide, slot, h as u32, &mut rng, &mut seed);
        slide_fx::blit(b, &key, x as i32, y as i32, w as i32, &passes);
        true
    }

    /// A sprite the original draws at a sprpos slot (0x460880): sliding with its effect, else plain.
    #[allow(clippy::too_many_arguments)]
    fn slot_menu(&self, b: &mut Batch, at: (usize, usize), name: &str, frame: usize, x: f32, y: f32) {
        if !self.slide_sprite(b, at, name, frame, x, y) {
            b.menu(name, frame, x, y);
        }
    }

    /// How far a slot is from its resting place this frame (art the original places by a slot without
    /// the slot drawer: the Track and Car pictures).
    fn slot_shift(&self, set: usize, slot: usize) -> (f32, f32) {
        let sp = self.sp();
        ((sp.x(set, slot) - self.sprpos.x(set, slot)) as f32, (sp.y(set, slot) - self.sprpos.y(set, slot)) as f32)
    }

    /// `slot_menu` at the slot's own position.
    fn slot_sprite(&self, b: &mut Batch, (set, slot): (usize, usize), name: &str, frame: usize) {
        let sp = self.sp();
        self.slot_menu(b, (set, slot), name, frame, sp.x(set, slot) as f32, sp.y(set, slot) as f32);
    }

    /// CLASSIC: the frame of a screen whose title and SCREEN box sit at two slots of its sprpos set
    /// (the inits 0x46d9e0, 0x46c21c, 0x46cc5e, 0x465f1c, 0x466b0c, 0x4674ac, 0x468d8c, 0x469510,
    /// 0x46ae2e, 0x46b987 list `dots`, then the two sprites, in slots 1, 2, 3; which of the two is
    /// first varies per screen) [O].
    fn frame_set(&self, b: &mut Batch, set: usize, title: &str, title_slot: usize, screen_box: &str, box_slot: usize) {
        self.frame_at(b, set, (title, title_slot), (screen_box, box_slot));
    }

    /// The background, rings, the halftone panel (every init loads `dots` at slot 1 of the screen's sprpos
    /// set: (0, 0) at rest, sliding in from the left), the screen title and its box at the given positions.
    fn frame_at(&self, b: &mut Batch, set: usize, (title, title_slot): (&str, usize), (screen_box, box_slot): (&str, usize)) {
        b.image("bg:menu", 0.0, 0.0);
        // CLASSIC 0x46f060: the rings and line objects are skipped on the track and car screens
        if !matches!(title, "track" | "car") {
            self.menu_fx.draw(b);
        }
        // CLASSIC 0x7507f2 / 0x7507f3 etc.: the init flags the title and the box to turn over now and then
        self.settle.borrow_mut().flag(&[title_slot, box_slot]);
        self.slot_sprite(b, (set, 1), "dots", 0);
        self.slot_sprite(b, (set, title_slot), title, 0);
        self.slot_sprite(b, (set, box_slot), screen_box, 0);
        // the version ends where the original's `rs0.1` ended (x = 602 + its width, 640 px layout) [design choice, #338]
        let ver = motor_version::DISPLAY;
        let right = 602.0 + b.text_width("rs0.1", 0.75);
        b.text(ver, right - b.text_width(ver, 0.75), 384.0, 0.75, [0.75, 0.75, 0.9, 1.0]);
    }

    /// CLASSIC: the frame of a dial screen, title and box at their sprpos slots.
    fn dial_frame(&self, b: &mut Batch, title: &str, screen_box: &str) {
        let l = self.dial_layout();
        self.frame_at(b, l.set, (title, l.title), (screen_box, l.screen_box));
    }

    /// CLASSIC 0x463538 etc.: the ring `msel`, the item labels, the info brackets, all at their sprpos
    /// slots, then the needle (0x481e64: scale 2.6, width 3, colour 1) at slot 0x1e.
    fn dial(&self, b: &mut Batch, items: &[&str]) {
        let (l, sp) = (self.dial_layout(), self.sp());
        let at = |slot: usize| (sp.x(l.set, slot) as f32, sp.y(l.set, slot) as f32);
        self.slot_sprite(b, (l.set, l.msel), "msel", 0);
        // the original draws unavailable items like the others (no dimming)
        for (i, &name) in items.iter().enumerate() {
            if name == KNOCKOUT_LABEL {
                // design choice: no sprite in the original; text in the Track screen's style
                // (orange when selected), pulsing like the sprite labels, between time attack's
                // and ghost mode's labels
                let (ax, ay) = at(l.first + 3);
                let (x, y) = (ax + 20.0, ay + 56.0);
                let c = if i != self.sel { [0.75, 0.75, 0.9, 1.0] } else if self.glow(true) >= 4 { ORANGE } else { WHITE };
                b.text("knockout", x, y, 0.95, c);
                continue;
            }
            let slot = l.first + i - usize::from(l.set == SINGLE_SET && i > 4);
            let (x, y) = at(slot);
            self.label(b, (l.set, slot), name, x, y, i == self.sel);
        }
        if let Some(info) = l.info {
            self.slot_sprite(b, (l.set, info), "info1", 0);
            self.slot_sprite(b, (l.set, info + 1), "info2", 0);
        }
        let (cx, cy) = at(0x1e);
        let [r, g, bl] = MENU_PALETTE[0];
        dial_needle(b, cx, cy, self.needle_q, 2.6, [r as f32 / 255.0, g as f32 / 255.0, bl as f32 / 255.0, 1.0]);
    }

    /// Label sprite inside a bracket at its slot (glows when selected).
    fn bracket_label(&self, b: &mut Batch, (set, slot): (usize, usize), label: &str, selected: bool) {
        let sp = self.sp();
        self.label(b, (set, slot), label, sp.x(set, slot) as f32, sp.y(set, slot) as f32, selected);
    }

    /// CLASSIC 0x46052c per frame: the laps counter's digits (ones first) close on the value, the
    /// short way round, like the Sound screen's counters (the original's slots are global and not
    /// reset on entry).
    fn roll_laps(&mut self, dt: f32) {
        let mut n = self.shown_laps();
        for pos in &mut self.laps_roll {
            let target = (n % 10) as f32;
            let mut d = target - pos.rem_euclid(10.0);
            if d > 5.0 {
                d -= 10.0;
            } else if d < -5.0 {
                d += 10.0;
            }
            *pos = if d.abs() < 0.1 { target } else { (*pos + d * (dt * COUNTER_RATE).min(1.0)).rem_euclid(10.0) };
            n /= 10;
        }
    }

    /// The `onoff` sprite at its slot, rolling (`rolls[roll]`) between frame 0 "on" and 7 "off" (0x46c450).
    fn onoff(&self, b: &mut Batch, at: (usize, usize), roll: usize) {
        self.slot_sprite(b, at, "onoff", self.rolls[roll].round() as usize);
    }

    // ---------------------------------------------------------------- screens

    fn main_menu(&self, b: &mut Batch) {
        self.dial_frame(b, "main", "scr0");
        self.dial(b, &MAIN_ITEMS);
    }

    fn single_menu(&self, b: &mut Batch) {
        self.dial_frame(b, "singl", "scr7");
        self.dial(b, &SINGLE_ITEMS);
        // CLASSIC 0x46a300: FONT1 rows from sprpos.dta set 10 (base = slots 10 + 20, row pitch =
        // slot 0x15's y, right column = its x, value columns = slot 0x16's x and y); labels in
        // colour 3, values in colour 1
        let sp = self.sp();
        let (ax, ay) = (|slot| sp.x(SINGLE_SET, slot) as f32, |slot| sp.y(SINGLE_SET, slot) as f32);
        let (x0, y0) = (ax(10) + ax(20), ay(10) + ay(20));
        let (col, pitch) = (ax(0x15), ay(0x15));
        let (left_v, right_v) = (x0 + ax(0x16), x0 + col + ay(0x16));
        let [left, right] = self.single_info();
        for (i, ((lk, lv), (rk, rv))) in left.iter().zip(right.iter()).enumerate() {
            let y = y0 + i as f32 * pitch;
            menu_text(b, lk, x0, y, MENU_BLUE, Align::Left);
            menu_text(b, lv, left_v, y, MENU_ORANGE, Align::Left);
            menu_text(b, rk, x0 + col, y, MENU_BLUE, Align::Left);
            menu_text(b, rv, right_v, y, MENU_ORANGE, Align::Left);
        }
    }

    /// The summary under the Single dial: the selected mode and the settings a race starts with.
    fn single_info(&self) -> menu_modern::DialInfo {
        let s = &self.settings;
        let mode = match self.sel {
            0 => "Quick race",
            2 => "League race",
            3 => "Time attack",
            4 => "Knockout",
            5 => "Ghostmode",
            _ => "Single race",
        };
        [
            [
                ("Mode:", mode.to_string()),
                ("Car:", self.cars[s.car].name.clone()),
                ("Laps:", if self.sel == 4 { KNOCKOUT_LAPS } else { s.laps }.to_string()),
            ],
            [
                ("Difficulty:", difficulty_name(s.difficulty).to_string()),
                (
                    "Transmission:",
                    if s.manual { "Manual" } else { "Automatic" }.into(),
                ),
                ("Track:", self.tracks[s.track].name.clone()),
            ],
        ]
    }

    fn options_menu(&self, b: &mut Batch) {
        self.dial_frame(b, "opt", "scr1");
        self.dial(b, &OPTION_ITEMS);
        // CLASSIC 0x4668a8 via 0x460dd0: sprites 9..11 at their set-7 slots: the plain label (pulses
        // when selected), the line down to the box, and the value word in it
        let sp = self.sp();
        let at = |slot: usize| (sp.x(OPTIONS_SET, slot) as f32, sp.y(OPTIONS_SET, slot) as f32);
        let (lx, ly) = at(9);
        self.label(b, (OPTIONS_SET, 9), "tdif", lx, ly, self.sel == 4);
        self.slot_sprite(b, (OPTIONS_SET, 10), "difin", 0);
        self.slot_sprite(b, (OPTIONS_SET, 11), "emh", self.diff_roll.round() as usize);
    }

    /// Graphic options in the original's text-list style: label, then the choices with the
    /// active one boxed in orange.
    fn graphics_screen(&self, b: &mut Batch) {
        // CLASSIC 0x4674f5: the Graphics screen's tag is scr4 ("Screen")
        self.frame_set(b, GRAPHICS_SET, "opt", 2, "scr4", 3);
        self.graphics_panel_fnt(b);
    }

    /// A graphic-options row as shown: its label and value (shared by the Classic list and the
    /// modern panel). A single-choice value (screens, resolution, percent) is the current text.
    pub(super) fn gfx_row_view(&self, row: GfxRow, e: &GfxEdit) -> (&'static str, RowValue) {
        let v = &self.settings.view;
        let d = &e.display;
        let fps = FRAME_LIMITS.iter().position(|&f| f == self.settings.fps_limit).unwrap_or(1);
        // long lists (screens, resolutions) show only the current value, between arrows
        let mi = d.monitor.min(self.monitors.len().saturating_sub(1));
        let monitor = format!("< {} >", self.monitors.get(mi).map_or("Default", |m| m.as_str()));
        let res = format!("< {}x{} >", d.size.0, d.size.1);
        let bright = format!("< {}% >", (v.brightness * 100.0).round() as u32);
        let s = &self.settings;
        let delay = if s.checkpoint_delay == 0 { "< off >".to_string() } else { format!("< {} s >", s.checkpoint_delay) };
        let render_aa_i = RenderAaKind::AVAILABLE.iter().position(|&k| k == v.remaster.render_aa.kind).unwrap_or(0);
        let post_aa_i = PostAaKind::AVAILABLE.iter().position(|&k| k == v.remaster.post_aa.kind).unwrap_or(0);
        let aniso_i = RemasterView::ANISOTROPY_LEVELS.iter().position(|&n| n == v.remaster.anisotropy).unwrap_or(0);
        let classic = v.mode == RenderMode::Classic;
        // an original on/off option with a Remaster style: Off, Classic, the style
        let style = |on: bool, modern: bool| if !on { 0 } else if modern { 2 } else { 1 };
        // the switches and sliders
            let pct = |p: u32| (p as f32 / 100.0, if p == 0 { "Off".to_string() } else { format!("{p}%") });
            let control = match row {
                GfxRow::DynamicLights => Some(("Dynamic lights:", Control::Switch(v.remaster.dynamic_lights))),
                GfxRow::PixelLight => Some(("Per-pixel car light:", Control::Switch(v.remaster.pixel_light))),
                GfxRow::PixelFog => Some(("Per-pixel fog:", Control::Switch(v.remaster.pixel_fog))),
                GfxRow::ClearCoat => Some(("Clear coat:", Control::Switch(v.remaster.clear_coat))),
                GfxRow::Signs => Some(("Reflective signs:", Control::Switch(v.remaster.retro_signs))),
                GfxRow::WorldFill => Some(("World fill:", Control::Switch(v.remaster.world_fill))),
                GfxRow::TreeModels => Some(("Tree models:", Control::Switch(v.remaster.tree_models))),
                GfxRow::Water => Some(("Water:", Control::Switch(v.remaster.water))),
                GfxRow::WaterReflect => Some(("Water reflections:", Control::Switch(v.remaster.water_ssr))),
                GfxRow::LocalLight => Some(("Local light:", Control::Switch(v.remaster.local_light))),
                GfxRow::TexturePack => Some(("Texture pack:", Control::Switch(v.remaster.texture_pack))),
                GfxRow::ViewDistance => {
                    let d = v.distance;
                    Some(("View distance:", Control::Slider((d - 0.3) / (View::MAX_DISTANCE - 0.3), format!("{}%", (d * 100.0).round() as u32))))
                }
                GfxRow::ColourGrade => Some(("Colour grade:", Control::Slider(pct(v.remaster.grade).0, pct(v.remaster.grade).1))),
                GfxRow::Vignette => Some(("Vignette:", Control::Slider(pct(v.remaster.vignette).0, pct(v.remaster.vignette).1))),
                GfxRow::Grain => Some(("Film grain:", Control::Slider(pct(v.remaster.grain).0, pct(v.remaster.grain).1))),
                GfxRow::Hdr => Some(("HDR lighting:", Control::Switch(v.remaster.hdr))),
                GfxRow::FogStrength => {
                    let (lo, hi) = RemasterView::FOG_STRENGTH;
                    let p = v.remaster.fog_strength;
                    Some(("Fog strength:", Control::Slider((p - lo) as f32 / (hi - lo) as f32, format!("{p}%"))))
                }
                GfxRow::CarLodDistance => {
                    let (lo, hi) = RemasterView::CAR_LOD_DISTANCE;
                    let p = v.remaster.car_lod_distance;
                    Some(("Car detail distance:", Control::Slider((p - lo) as f32 / (hi - lo) as f32, format!("{p}%"))))
                }
                GfxRow::GrassRange => {
                    let (lo, hi) = RemasterView::GRASS_RANGE;
                    let p = v.remaster.grass_range;
                    Some(("Grass range:", Control::Slider((p - lo) as f32 / (hi - lo) as f32, format!("{p}%"))))
                }
                GfxRow::Exposure => {
                    let (lo, hi) = RemasterView::EXPOSURE_RANGE;
                    let e = v.remaster.exposure;
                    Some(("Exposure:", Control::Slider((e - lo) as f32 / (hi - lo) as f32, format!("{e}%"))))
                }
                GfxRow::RenderAaStrength => Some(("Render AA strength:", Control::Slider(aa_stop(v.remaster.render_aa.strength) as f32 / 3.0, v.remaster.render_aa.strength_text()))),
                GfxRow::PostAaStrength => Some(("Post AA strength:", Control::Slider(aa_stop(v.remaster.post_aa.strength) as f32 / 3.0, v.remaster.post_aa.strength_text()))),
                GfxRow::Bloom => Some(("Bloom:", Control::Slider(pct(v.remaster.bloom).0, pct(v.remaster.bloom).1))),
                GfxRow::Ssao => Some(("Ambient occlusion (SSAO):", Control::Slider(pct(v.remaster.ssao).0, pct(v.remaster.ssao).1))),
                _ => None,
            };
        if let Some((label, c)) = control {
            return (label, RowValue::Control(c));
        }
            let (label, choices, cur): (&'static str, &[&str], usize) = match row {
                GfxRow::RenderDevice => ("Render device:", &[self.render_device.as_str()], 0),
                GfxRow::Monitor => ("Primary screen:", &[monitor.as_str()], 0),
                GfxRow::ScreenMode => ("Screen mode:", &["Window", "Fullscreen"], d.fullscreen as usize),
                GfxRow::Ratio => ("Screen ratio:", &["4:3", "16:9"], d.wide as usize),
                GfxRow::MenuPicture => ("Menu picture:", &["Keep shape", "Stretch"], self.settings.menu_stretch as usize),
                GfxRow::Resolution => ("Resolution:", &[res.as_str()], 0),
                GfxRow::VSync => ("VSync:", &["Off", "On"], d.vsync as usize),
                GfxRow::FrameLimit => ("Frame limit:", &["30", "60", "90", "120", "240", "Unlimited"], fps),
                GfxRow::Brightness => ("Brightness:", &[bright.as_str()], 0),
                GfxRow::CdMusic => ("CD music:", &["Off", "On"], self.settings.music as usize),
                GfxRow::DetailMode => ("Detail mode:", &DETAIL_MODES, self.settings.detail_mode),
                GfxRow::Shadows if !classic => ("Shadows:", &["Off", "Classic", "Real"], style(v.shadows, v.remaster.car_shadows)),
                GfxRow::Shadows => ("Shadows:", &["Off", "On"], v.shadows as usize),
                GfxRow::Halos if !classic => ("Halos:", &["Off", "Classic", "Smooth"], style(v.halos, v.remaster.smooth_halos)),
                GfxRow::Halos => ("Halos:", &["Off", "On"], v.halos as usize),
                GfxRow::Fog if !classic => ("Fog:", &["Off", "Classic", "Enhanced", "Volumetric"], if v.fog { 1 + v.remaster.fog_style as usize } else { 0 }),
                GfxRow::Fog => ("Fog:", &["Off", "On"], v.fog as usize),
                GfxRow::CarShading if !classic => ("Carshading:", &["Gouraud", "Reflection", "Enhanced"], remaster_shading(v.car_shading, v.remaster.enhanced_paint)),
                GfxRow::CarShading => ("Carshading:", &CarShading::NAMES, v.car_shading as usize),
                GfxRow::TrackDetailRow => ("Track detail:", &TRACK_DETAILS, s.track_detail),
                GfxRow::CarDetailRow => ("Car detail:", &CarDetail::NAMES, v.car_detail as usize),
                GfxRow::SkidMarks if !classic => ("Skid marks:", &["Off", "Classic", "Detailed"], style(self.settings.skid_marks, v.remaster.skids_detailed)),
                GfxRow::SkidMarks => ("Skid marks:", &["Off", "On"], self.settings.skid_marks as usize),
                GfxRow::NamePlates => ("Name plates:", &PLATES, self.settings.name_plates),
                GfxRow::SkyImage if !classic => ("Sky image:", &["Off", "Classic", "HDRI"], style(v.sky_image, v.remaster.sky_hdri)),
                GfxRow::SkyImage => ("Sky image:", &["Off", "On"], v.sky_image as usize),
                GfxRow::MotionBlur if !classic => ("Motion blur:", &["Off", "Classic", "Velocity"], style(v.motion_blur, v.remaster.velocity_blur)),
                GfxRow::MotionBlur => ("Motion blur:", &["Off", "On"], v.motion_blur as usize),
                GfxRow::Smoke => ("Smoke:", &["Off", "On"], v.smoke as usize),
                GfxRow::Sparks => ("Sparks:", &["Off", "On"], v.sparks as usize),
                GfxRow::LensFlares if !classic => ("Lens flares:", &["Off", "Classic", "Smooth"], style(v.lens_flares, v.remaster.smooth_flares)),
                GfxRow::LensFlares => ("Lens flares:", &["Off", "On"], v.lens_flares as usize),
                GfxRow::InfoDetail => ("Info detail:", &INFO_LEVELS, s.info_detail),
                GfxRow::CheckpointInfo => ("Checkpoint info:", &LEVELS, s.checkpoint_info),
                GfxRow::CheckpointDelay => ("Checkpoint delay:", &[delay.as_str()], 0),
                GfxRow::RoadMap => ("Road map:", &LEVELS, s.road_map),
                GfxRow::CamStart => ("Start camera:", &CAM_STARTS, s.cam.start),
                GfxRow::CamMirror => ("Rear mirror:", &["Off", "On"], s.cam.mirror as usize),
                GfxRow::CamFreeLook => ("Free look:", &["Off", "On"], s.cam.free_look as usize),
                GfxRow::CamPullIn => ("Wall pull-in:", &["Off", "On"], s.cam.pull_in as usize),
                GfxRow::CamFollow => ("Chase follow:", &CAM_FOLLOWS, s.cam.follow),
                GfxRow::Mode => ("Graphics mode:", &RenderMode::NAMES, v.mode as usize),
                GfxRow::ModeSwitch => ("Mode switch:", &["Instant", "Dissolve"], s.mode_dissolve as usize),
                GfxRow::RenderAa => ("Render AA:", &RenderAaKind::NAMES, render_aa_i),
                GfxRow::PostAa => ("Post AA:", &PostAaKind::NAMES, post_aa_i),
                GfxRow::Anisotropic => ("Texture filter:", &["Trilinear", "Aniso 2x", "4x", "8x", "16x"], aniso_i),
                GfxRow::Tonemap => ("Tonemapper:", &Tonemap::NAMES, v.remaster.tonemap as usize),
                GfxRow::HudStyle => ("HUD style:", &HUD_STYLES, v.remaster.modern_hud as usize),
                GfxRow::MenuStyle => ("Menu style:", &HUD_STYLES, v.remaster.modern_menu as usize),
                GfxRow::LightShadows => ("Light shadows:", &LightShadows::NAMES, v.remaster.light_shadows as usize),
                GfxRow::ShadowQuality => ("Shadow quality:", &ShadowQuality::NAMES, v.remaster.shadow_quality as usize),
                GfxRow::Grass => ("Grass:", &GrassLevel::NAMES, v.remaster.grass as usize),
                GfxRow::CarModels => ("Car models:", &CAR_MODELS, v.remaster.generated_cars as usize),
                _ => return ("", RowValue::Choices(Vec::new(), 0)),
            };
        (label, RowValue::Choices(choices.iter().map(|c| c.to_string()).collect(), cur))
    }

    /// `gfx_row_view` for the modern pause tabs: the HUD tab's rows and their wording (Name plates Off / Flat /
    /// See-through, Checkpoint splits, Splits on screen), the lists of the monitors and resolutions, Brightness as a slider.
    pub(super) fn gfx_row_view_modern(&self, row: GfxRow, e: &GfxEdit) -> (&'static str, RowValue) {
        let s = &self.settings;
        let h = s.hud.shown(s.info_detail);
        let sw = |on| RowValue::Control(Control::Switch(on));
        let slider = |f: f32, t: String| RowValue::Control(Control::Slider(f, t));
        let names = |l: &[&str]| RowValue::Choices(l.iter().map(|c| c.to_string()).collect(), 0);
        let with = |v: RowValue, i: usize| match v {
            RowValue::Choices(l, _) => RowValue::Choices(l, i),
            other => other,
        };
        let (lo, hi) = SCALE_RANGE;
        match row {
            GfxRow::HudScale => ("Scale:", slider((h.scale - lo) as f32 / (hi - lo) as f32, format!("{}%", h.scale))),
            GfxRow::HudMargin => ("Margin:", slider(h.margin as f32 / 100.0, format!("{}%", h.margin))),
            GfxRow::HudCluster => ("Speed and revs:", sw(h.cluster)),
            GfxRow::HudTotal => ("Total time:", sw(h.clocks[0])),
            GfxRow::HudLapTime => ("Lap time:", sw(h.clocks[1])),
            GfxRow::HudBest => ("Best lap:", sw(h.clocks[2])),
            GfxRow::HudLap => ("Lap:", sw(h.lap)),
            GfxRow::HudPos => ("Position:", sw(h.pos)),
            GfxRow::HudMap => ("Map:", with(names(&MAP_LEVELS), map_row(s))),
            GfxRow::MapView => ("Map view:", with(names(&MAP_MODES), h.map_mode as usize)),
            GfxRow::MapTurns => ("Map turns:", sw(h.map_rotate)),
            GfxRow::MapNumbers => ("Map numbers:", sw(h.map_numbers)),
            GfxRow::PlateFrame => ("Plate frame:", sw(h.plate_frame)),
            GfxRow::Proximity => ("Proximity arrows:", sw(h.proximity)),
            GfxRow::LiveGaps => ("Live gaps:", sw(h.live_gaps)),
            GfxRow::SectorDelta => {
                let d = h.sector_delay;
                ("Sector delta:", slider(d as f32 / crate::hud_layout::LAP_DELAY_MAX as f32, if d == 0 { "Off".into() } else { format!("{d} s") }))
            }
            GfxRow::NamePlates => ("Name plates:", with(names(&["Off", "Flat", "See-through"]), s.name_plates)),
            GfxRow::CheckpointInfo => ("Checkpoint splits:", with(names(&MAP_LEVELS), s.checkpoint_info)),
            GfxRow::LapDelay => {
                let d = h.lap_delay;
                ("Lap times on screen:", slider(d as f32 / crate::hud_layout::LAP_DELAY_MAX as f32, if d == 0 { "Off".into() } else { format!("{d} s") }))
            }
            GfxRow::CheckpointDelay => {
                let d = s.checkpoint_delay;
                ("Splits on screen:", slider(d as f32 / DELAY_MAX as f32, if d == 0 { "Off".into() } else { format!("{d} s") }))
            }
            GfxRow::Brightness => {
                let br = s.view.brightness;
                ("Brightness:", slider((br - 0.5) / 1.5, format!("{}%", (br * 100.0).round() as u32)))
            }
            GfxRow::Monitor => {
                let list: Vec<String> = if self.monitors.is_empty() { vec!["Default".into()] } else { self.monitors.clone() };
                let at = e.display.monitor.min(list.len() - 1);
                ("Primary screen:", RowValue::Choices(list, at))
            }
            GfxRow::Resolution => {
                let list = resolutions(e.display.wide);
                let at = list.iter().position(|&r| r == e.display.size).unwrap_or(0);
                ("Resolution:", RowValue::Choices(list.iter().map(|r| format!("{}x{}", r.0, r.1)).collect(), at))
            }
            _ => self.gfx_row_view(row, e),
        }
    }

    /// `gfx_row_view` for the Classic lists: the original's labels and value names (CLASSIC
    /// 0x4d5bc8.., 0x4be150..: "Screen size", "Graphic detail", "Lensflares", "Skidmarks", "Nameplates"
    /// Off/Flat/Transparent, "Background" Blank/Picture, "Info map", "Checkpoint displaytime").
    fn gfx_row_view_classic(&self, row: GfxRow, e: &GfxEdit) -> (&'static str, RowValue) {
        let (label, val) = self.gfx_row_view(row, e);
        let label = match row {
            GfxRow::Resolution => "Screen size:",
            GfxRow::DetailMode => "Graphic detail:",
            GfxRow::LensFlares => "Lensflares:",
            GfxRow::SkidMarks => "Skidmarks:",
            GfxRow::NamePlates => "Nameplates:",
            GfxRow::SkyImage => "Background:",
            GfxRow::TrackDetailRow => "Trackdetail:",
            GfxRow::CarDetailRow => "Cardetail:",
            GfxRow::CheckpointDelay => "Checkpoint displaytime:",
            GfxRow::RoadMap => "Info map:",
            _ => label,
        };
        let val = match (row, val) {
            (GfxRow::NamePlates, RowValue::Choices(mut l, c)) => {
                l[0] = "Off".to_string();
                RowValue::Choices(l, c)
            }
            (GfxRow::SkyImage, RowValue::Choices(mut l, c)) => {
                l[0] = "Blank".to_string();
                l[1] = "Picture".to_string();
                RowValue::Choices(l, c)
            }
            (_, val) => val,
        };
        (label, val)
    }

    /// The graphic options' tooltip [design choice]: what the page is for on the tabs row, else
    /// the selected row or button; wrapped for the FONT1 list or the myfont one.
    fn gfx_tip(&self, b: &Batch, e: &GfxEdit, rows: &[GfxRow], fnt: bool) -> Vec<String> {
        let classic = self.settings.view.mode == RenderMode::Classic;
        let mut tip: Vec<String> = Vec::new();
        if self.sel == 0 {
            tip.push(
                match e.page {
                    SCREEN_PAGE => "The original's options; Esc keeps them.",
                    1 => "Window and frame rate: changes wait for Apply or OK.",
                    DETAIL_PAGE if classic => "Our graphics mode switch. Switch to Remaster for more.",
                    DETAIL_PAGE => "The Remaster additions.",
                    _ => "The race camera: where it starts and how the chase view behaves.",
                }
                .to_string(),
            );
            if e.page == DETAIL_PAGE {
                tip.push("F9 switches the graphics mode at any time, in menus and races.".to_string());
            }
        } else if let Some(&row) = rows.get(self.sel - 1) {
            // at most two lines in the myfont list: the Detail page leaves room for no more
            tip = if fnt { wrap_fnt(b, row.help(), 470.0) } else { wrap_text(b, row.help(), 0.75, 470.0) };
        } else {
            tip.push(
                ["Apply: use the changes and stay here.", "OK: use the changes and go back.", "Cancel: undo the changes since the last Apply and go back."][e.button]
                    .to_string(),
            );
        }
        tip
    }

    /// CLASSIC 0x4676d4: the graphic options as the original's FONT1 list: labels at sprpos set 5
    /// slot 4, the values `slot 0x18 x` to their right, one row `slot 0x18 y` below the other;
    /// colours as in `MenuRow`. The tabs, tooltip and buttons are ours (the original is one page)
    /// and use the same colours.
    fn graphics_panel_fnt(&self, b: &mut Batch) {
        let v = &self.settings.view;
        let e = self.gfx.unwrap_or_else(|| GfxEdit::new(&self.settings));
        let d = &e.display;
        let sp = self.sp();
        let (x0, y0) = (sp.x(GRAPHICS_SET, 4) as f32, sp.y(GRAPHICS_SET, 4) as f32);
        let (col, pitch) = (x0 + sp.x(GRAPHICS_SET, 0x18) as f32, sp.y(GRAPHICS_SET, 0x18) as f32);
        // page tabs, above the first row
        let ty = y0 - 20.0;
        let mut x = x0;
        for (k, (name, _)) in GFX_PAGES.iter().enumerate() {
            let w = b.fnt_width(name, 1.0);
            if k == e.page {
                menu_bar(b, x, ty, w, if self.sel == 0 { 2 } else { 1 });
                menu_text(b, name, x, ty, MENU_BLACK, Align::Left);
            } else {
                menu_text(b, name, x, ty, if self.sel == 0 { 4 } else { MENU_BLUE }, Align::Left);
            }
            x += w + 30.0;
        }
        let rows = gfx_rows(e.page, &self.settings);
        // a long page (Detail in Remaster) scrolls: a window of GFX_LIST_ROWS round the selection;
        // the Screen page is the original's list, all rows at once
        let cap = if e.page == SCREEN_PAGE { rows.len() } else { GFX_LIST_ROWS };
        let anchor = match self.sel {
            0 => 0,
            n => (n - 1).min(rows.len() - 1),
        };
        let first = anchor.saturating_sub(cap / 2).min(rows.len().saturating_sub(cap));
        let shown = rows.len().min(cap);
        for (i, &row) in rows.iter().enumerate().skip(first).take(cap) {
            let y = y0 + (i - first) as f32 * pitch;
            let mr = MenuRow { sel: i + 1 == self.sel, on: row.active(v) };
            let (label, val) = self.gfx_row_view_classic(row, &e);
            menu_label(b, label, x0, y, mr);
            match val {
                RowValue::Choices(list, cur) => {
                    let refs: Vec<&str> = list.iter().map(String::as_str).collect();
                    menu_choices(b, &refs, cur, col, y, mr);
                }
                // [design choice] the original has only text choices
                RowValue::Control(Control::Switch(on)) => {
                    menu_choices(b, &["Off", "On"], on as usize, col, y, mr);
                }
                RowValue::Control(Control::Slider(f, text)) => {
                    const W: f32 = 120.0;
                    let f = f.clamp(0.0, 1.0);
                    let [r, g, bl] = MENU_PALETTE[if mr.sel { 1 } else { 0 }];
                    let c = [r as f32 / 255.0, g as f32 / 255.0, bl as f32 / 255.0, mr.alpha()];
                    b.rect(col, y + 4.0, W, 3.0, [c[0], c[1], c[2], 0.35 * c[3]]);
                    b.rect(col, y + 4.0, W * f, 3.0, c);
                    b.rect(col + W * f - 2.0, y, 4.0, 11.0, c);
                    menu_text_a(b, &text, col + W + 12.0, y, if mr.sel { 2 } else { 1 }, Align::Left, mr.alpha());
                }
            }
        }
        if first > 0 {
            menu_text(b, "^", x0 - 18.0, y0, MENU_BLUE, Align::Left);
        }
        if first + shown < rows.len() {
            menu_text(b, "v", x0 - 18.0, y0 + (shown - 1) as f32 * pitch, MENU_BLUE, Align::Left);
        }
        let ny = y0 + shown as f32 * pitch + 6.0;
        let tip = self.gfx_tip(b, &e, &rows, true);
        // the Custom list leaves no room under it for the tooltip: it is left out then
        let tip_h = tip.len() as f32 * 12.0;
        let tip_room = e.page != SCREEN_PAGE || ny + tip_h + 14.0 <= 392.0;
        if tip_room {
            for (k, l) in tip.iter().enumerate() {
                menu_text(b, l, x0, ny + k as f32 * 12.0, MENU_BLUE, Align::Left);
            }
        }
        if e.page == SCREEN_PAGE {
            // CLASSIC: no Apply / OK / Cancel; Esc keeps the changes and leaves
            let hy = if tip_room { ny + tip_h + 4.0 } else { ny + 2.0 };
            menu_text(b, "arrows: change    esc: back", x0, hy, MENU_BLUE, Align::Left);
            return;
        }
        // Apply / OK / Cancel, under the longest page and its tooltip
        let y = 346.0;
        let mut x = x0;
        for (k, t) in GFX_BUTTONS.iter().enumerate() {
            let w = b.fnt_width(t, 1.0);
            if self.sel == rows.len() + 1 && k == e.button {
                menu_bar(b, x, y, w, 2);
                menu_text(b, t, x, y, MENU_BLACK, Align::Left);
            } else {
                menu_text(b, t, x, y, 4, Align::Left);
            }
            x += w + 30.0;
        }
        if *d != self.settings.display {
            menu_text(b, "(not applied yet)", x + 10.0, y, MENU_ORANGE, Align::Left);
        }
        menu_text(b, "arrows: change    enter: select    esc: save, back", x0, y + 20.0, MENU_BLUE, Align::Left);
    }

    /// The graphic options' tabs, rows, tooltip and buttons on the menu canvas: drawn by the
    /// options screen and, over the race, by the in-race GFX Options.
    fn graphics_panel(&self, b: &mut Batch) {
        let v = &self.settings.view;
        let e = self.gfx.unwrap_or_else(|| GfxEdit::new(&self.settings));
        let d = &e.display;
        // page tabs
        let mut x = 130.0;
        for (k, (name, _)) in GFX_PAGES.iter().enumerate() {
            let w = b.text_width(name, 0.85);
            if k == e.page {
                b.rect(x - 4.0, 82.0, w + 8.0, 14.0, if self.sel == 0 { ORANGE } else { [0.45, 0.3, 0.1, 1.0] });
                b.text(name, x, 84.0, 0.85, [0.1, 0.05, 0.0, 1.0]);
            } else {
                b.text(name, x, 84.0, 0.85, if self.sel == 0 { WHITE } else { [0.75, 0.75, 0.95, 1.0] });
            }
            x += w + 30.0;
        }
        let rows = gfx_rows(e.page, &self.settings);
        // a long page (Detail in Remaster) scrolls: a window of GFX_LIST_ROWS round the selection;
        // the Screen page is the original's list, all rows at once and a little closer
        let screen = e.page == SCREEN_PAGE;
        let (cap, pitch) = if screen { (rows.len(), 13.0) } else { (GFX_LIST_ROWS, 14.0) };
        let anchor = match self.sel {
            0 => 0,
            n => (n - 1).min(rows.len() - 1),
        };
        let first = anchor.saturating_sub(cap / 2).min(rows.len().saturating_sub(cap));
        let shown = rows.len().min(cap);
        for (i, &row) in rows.iter().enumerate().skip(first).take(cap) {
            let y = 106.0 + (i - first) as f32 * pitch;
            let sel = i + 1 == self.sel;
            let (label, val) = self.gfx_row_view_classic(row, &e);
            let on = row.active(v);
            match val {
                RowValue::Control(c) => control_row(b, label, c, y, sel, on),
                RowValue::Choices(list, cur) => {
                    let refs: Vec<&str> = list.iter().map(String::as_str).collect();
                    choice_row_in(b, label, &refs, cur, y, sel, on);
                }
            }
        }
        if first > 0 {
            b.text("^", 116.0, 106.0, 0.85, DIM);
        }
        if first + shown < rows.len() {
            b.text("v", 116.0, 106.0 + (shown - 1) as f32 * pitch, 0.85, DIM);
        }
        let tip = self.gfx_tip(b, &e, &rows, false);
        let ny = 110.0 + shown as f32 * pitch;
        let tip_h = tip.len() as f32 * 13.0;
        // the Custom list of the Screen page leaves no room under it for the tooltip
        let tip_room = !screen || ny + tip_h + 14.0 <= 396.0;
        if tip_room {
            for (k, l) in tip.iter().enumerate() {
                b.text(l, 130.0, ny + k as f32 * 13.0, 0.75, DIM);
            }
        }
        if screen {
            // CLASSIC: no Apply / OK / Cancel; Esc keeps the changes and leaves
            let hy = if tip_room { ny + tip_h + 2.0 } else { ny };
            b.text("arrows: change    esc: back", 130.0, hy, 0.75, DIM);
            return;
        }
        // Apply / OK / Cancel, under the longest page and its tooltip, inside the 400-line canvas
        let y = 364.0;
        let mut x = 130.0;
        for (k, t) in GFX_BUTTONS.iter().enumerate() {
            let w = b.text_width(t, 0.85);
            if self.sel == rows.len() + 1 && k == e.button {
                b.rect(x - 4.0, y - 2.0, w + 8.0, 14.0, ORANGE);
                b.text(t, x, y, 0.85, [0.1, 0.05, 0.0, 1.0]);
            } else {
                b.text(t, x, y, 0.85, [0.75, 0.75, 0.95, 1.0]);
            }
            x += w + 30.0;
        }
        if *d != self.settings.display {
            b.text("(not applied yet)", x + 10.0, y + 1.0, 0.75, ORANGE);
        }
        b.text("arrows: change    enter: select    esc: save, back", 130.0, y + 18.0, 0.75, DIM);
    }

    /// The credits page to show: the hidden one after the club message or with the three keys held (`0x46dc15`, `0x46dd82`).
    pub(super) fn credits_page(&self) -> &[String] {
        if (self.club_seen || self.credits_keys) && !self.credits_alt.is_empty() {
            &self.credits_alt
        } else {
            &self.credits
        }
    }

    fn credits_screen(&self, b: &mut Batch) {
        self.frame_set(b, CREDITS_SET, "credi", 3, "scr9", 2);
        // CLASSIC 0x46da44: centred at slot 0x18, one line = slot 0x19's y / 8; a heading (colour 3)
        // is followed by a blank line and its names (colour 1), a blank line before the next heading
        let sp = self.sp();
        let (cx, y0) = (sp.x(CREDITS_SET, 0x18) as f32, sp.y(CREDITS_SET, 0x18) as f32);
        let pitch = (sp.y(CREDITS_SET, 0x19) / 8) as f32;
        let mut line = 0.0;
        for (i, l) in self.credits_page().iter().enumerate() {
            let heading = l.ends_with(':');
            if heading {
                line += if i > 0 { 1.0 } else { 0.0 };
            }
            menu_text(b, l, cx, y0 + line * pitch, if heading { MENU_BLUE } else { MENU_ORANGE }, Align::Centre);
            line += if heading { 2.0 } else { 1.0 };
        }
    }

    fn track_screen(&self, b: &mut Batch) {
        self.frame_set(b, TRACK_SET, "track", 2, "scra", 3);
        let tr = &self.tracks[self.settings.track];
        // the picture (and so its window) slides with its slot 0x18
        let (dx, dy) = self.slot_shift(TRACK_SET, 0x18);
        let (cx, cy, rx, ry) = (TRACK_WINDOW.0 + dx, TRACK_WINDOW.1 + dy, TRACK_WINDOW.2, TRACK_WINDOW.3);
        b.clip_ellipse(cx, cy, rx, ry);
        // CLASSIC 0x46c3a7: faded by `PicFade`
        let shown = self.tracks.get(self.pic.shown).map_or(tr.n, |t| t.n);
        // a browsed-but-locked track is drawn at half brightness (0x46c753)
        let tint = if self.track_locked() { self.pic.tint_dim(0.5) } else { self.pic.tint() };
        b.image_scaled(&format!("trk:{shown}"), cx - rx, cy - ry, rx * 2.0, ry * 2.0, tint);
        b.clip_none();
        // CLASSIC 0x46c5a2: the art is slots 4..0xa of set 2 (init 0x46c21c): trsel carop ttrac tlaps tcat onoff caro2
        self.slot_sprite(b, (TRACK_SET, 4), "trsel", 0);
        self.track_outline(b, tr);
        // CLASSIC 0x46c5f3: the name centred at slot 0x13, the arrows alternate on a 256 ms clock
        let (nx, ny) = (self.sp().x(TRACK_SET, 0x13) as f32, self.sp().y(TRACK_SET, 0x13) as f32);
        menu_text(b, &tr.name, nx, ny, MENU_ORANGE, Align::Centre);
        self.name_arrows(b, nx, ny, b.fnt_width(&tr.name, 1.0), self.settings.track, self.locks().tracks);
        // CLASSIC 0x46c6de: "Not yet available" blinks (on while the 1024 ms clock is below 512) at slot 0x18, 5 px up
        if self.track_locked() && (self.clock * 1000.0) as u32 & 0x3ff < 0x200 {
            menu_text(b, "Not yet available", self.sp().x(TRACK_SET, 0x18) as f32, self.sp().y(TRACK_SET, 0x18) as f32 - 5.0, MENU_ORANGE, Align::Centre);
        }
        // rows: track + laps on one bracket pair, catch-up on the step down to the long line
        self.slot_sprite(b, (TRACK_SET, 5), "carop", 0);
        self.slot_sprite(b, (TRACK_SET, 0xa), "caro2", 0);
        self.bracket_label(b, (TRACK_SET, 6), "ttrac", self.sel == 0);
        self.bracket_label(b, (TRACK_SET, 7), "tlaps", self.sel == 1);
        self.bracket_label(b, (TRACK_SET, 8), "tcat", self.sel == 2);
        // CLASSIC 0x46c72e: the laps counter at slot 0x12
        flip_counter(b, self.sp().x(TRACK_SET, 0x12) as f32, self.sp().y(TRACK_SET, 0x12) as f32, &self.laps_roll);
        self.onoff(b, (TRACK_SET, 9), ROLL_CATCHUP);
    }

    /// The blinking arrows beside the stepped name (0x46c5f3 / 0x46d170): the left one in the first
    /// half of each 256 ms, the right one in the second, each only while there is a neighbour.
    fn name_arrows(&self, b: &mut Batch, x: f32, y: f32, name_w: f32, cur: usize, count: usize) {
        self.name_arrows_at(b, x, y, name_w, cur > 0, cur + 1 < count);
    }

    /// `name_arrows` with the two neighbours given: a left one, a right one.
    fn name_arrows_at(&self, b: &mut Batch, x: f32, y: f32, name_w: f32, left: bool, right: bool) {
        let phase = (self.clock * 1000.0) as u32 & 0xff;
        let half = (name_w / 2.0).floor() + 8.0;
        if left && phase < 0x80 {
            menu_text(b, ARROW_LEFT, x - half, y, MENU_ORANGE, Align::Centre);
        }
        if right && phase > 0x80 {
            menu_text(b, ARROW_RIGHT, x + half, y, MENU_ORANGE, Align::Centre);
        }
    }

    /// The selected track's line object over the picture (CLASSIC 0x46c4a2..0x46c581): seen from
    /// above, turning once in ~5 s while it rocks +-22 deg, and flying away into the distance as
    /// the picture fades out (distance = (1 - brightness) * 3000 + 475).
    fn track_outline(&self, b: &mut Batch, tr: &TrackInfo) {
        let Some((_, edges)) = &tr.wire else { return };
        // the menu clock in ms (0x702d28); 13/16 of a table step per ms
        let t = (self.clock as f64 * 1000.0) as u32;
        let yaw = t as f32 * 13.0 / 16.0;
        let pitch = (250.0 * (((yaw as u32 + 1024) & 0xfff) as f32 * std::f32::consts::TAU / 4096.0).sin()) as i32;
        let (cx, cy) = (self.sp().x(TRACK_SET, 0x1e) as f32, self.sp().y(TRACK_SET, 0x1e) as f32);
        let z = (1.0 - self.pic.b) * 3000.0 + 475.0;
        let pts = crate::menu_fx::project(&tr.outline, [pitch as u32, yaw as u32, 0], [0.0, 105.0, z], (cx, cy));
        for e in edges {
            if let (Some(Some(p)), Some(Some(q))) = (pts.get(e[0] as usize), pts.get(e[1] as usize)) {
                b.line(p.0, p.1, q.0, q.1, 1.0, [1.0, 1.0, 1.0, 1.0]);
            }
        }
    }

    /// REMASTER: the Track screen's info strip (same layout as Single's).
    fn track_info(&self) -> menu_modern::DialInfo {
        let s = &self.settings;
        // Ghost mode: the recording's track, laps and CatchUp
        let ghost = self.ghost_demo.as_ref().filter(|_| self.game_type == GameType::GhostMode);
        let tr = &self.tracks[ghost.and_then(|m| self.ghost_track(m).ok()).unwrap_or(s.track)];
        let mode = match self.game_type {
            GameType::Single => "Single race",
            GameType::League => "League race",
            GameType::TimeAttack => "Time attack",
            GameType::Knockout => "Knockout",
            GameType::GhostMode => "Ghostmode",
        };
        let class = if tr.division % 100 >= 10 { "Bonus".to_string() } else { format!("Division {}", league::tier(tr.division)) };
        [
            [("Mode:", mode.to_string()), ("Track:", tr.name.clone()), ("Class:", class)],
            [
                ("Laps:", ghost.map_or_else(|| self.shown_laps(), |m| m.header.laps).to_string()),
                ("Difficulty:", difficulty_name(s.difficulty).to_string()),
                ("Catch-up:", if ghost.map_or(s.catchup, |m| m.header.catchup()) { "On" } else { "Off" }.into()),
            ],
        ]
    }

    /// REMASTER: the Car screen's RACE SETTINGS strip, the Track screen's with the car's class and the transmission.
    fn car_info(&self) -> menu_modern::DialInfo {
        let mut info = self.track_info();
        info[0][2] = ("Class:", Game::car_class(league::tier(self.cars[self.settings.car].division)).to_string());
        info[1][2] = ("Transmission:", if self.settings.manual { "Manual" } else { "Automatic" }.to_string());
        info
    }

    /// Car select: the 3D car shows through a window in the menu layer.
    fn car_screen(&self, size: (u32, u32)) -> Vec<Layer> {
        // the photo (and so its window) slides with its slot 0x1e
        let (dx, dy) = self.slot_shift(CAR_SET, 0x1e);
        let (cx, cy, rx, ry) = (CAR_WINDOW.0 + dx, CAR_WINDOW.1 + dy, CAR_WINDOW.2, CAR_WINDOW.3);
        let mut out = Vec::new();
        // REMASTER: the 3D car; CLASSIC (and a garage that did not load) shows the car's photo
        let garage = self.settings.view.mode == RenderMode::Remaster && self.loaded == Loaded::Garage;
        if garage {
            // circle the selected car parked on the garage track, close in like the original
            let (spot, dir) = self.spot;
            let a = self.clock * 0.35;
            let eye = spot + Vec3::new(a.sin() * 4.6, 1.4, a.cos() * 4.6);
            let look = (spot + Vec3::Y * 0.5 - eye).normalize();
            let cam = Camera::new(eye, look.x.atan2(-look.z), look.y.asin());
            // car meshes face +z in game space, i.e. -z in view space
            let yaw = (-dir.x).atan2(-dir.z);
            // the tyres stand on the road: the model origin sits below it by the rest ground
            // height (`Scene::wheel_hubs`, 6.4 cm on the Titan), as in a race
            let ground = self.garage_wheels.get(self.settings.car).map_or(0.0, |(h, r)| h[0].y - r);
            let model = Mat4::from_translation(spot - Vec3::Y * ground) * Mat4::from_mat3(Mat3::from_rotation_y(yaw));
            let vp = self.menu_canvas().to_window(size, cx - rx, cy - ry, rx * 2.0, ry * 2.0);
            let k = self.settings.car;
            let mut cars = vec![(k * CAR_MESHES, model)];
            if let Some((hubs, _)) = self.garage_wheels.get(k) {
                cars.extend(hubs.iter().enumerate().map(|(w, h)| (k * CAR_MESHES + 1 + w, model * Mat4::from_translation(*h))));
            }
            out.push(Layer::Scene { cam, parts: Box::new(FrameParts { cars, ..Default::default() }), viewport: Some(vp), mirror: false });
        }
        out.push(self.ui(self.menu_canvas(), |g, b| {
            // the garage scene cleared the whole window to the track's colour: black bars hide it round a kept 16:10 picture
            if garage {
                b.bars(size, [0.0, 0.0, 0.0, 1.0]);
            }
            // CLASSIC 0x46d551: the car's photo (`_menuimg.TGA`, 324 x 270), centred at slot 0x1e of
            // set 3 and faded by `PicFade`, inside the ring
            if let Some(car) = g.cars.get(g.pic.shown).filter(|_| !garage) {
                let (px, py) = (g.sp().x(CAR_SET, 0x1e) as f32, g.sp().y(CAR_SET, 0x1e) as f32);
                b.clip_ellipse(cx, cy, rx, ry);
                // a browsed-but-locked car is drawn at 30 % (0x46d03d)
                let tint = if g.car_locked() { g.pic.tint_dim(0.3) } else { g.pic.tint() };
                b.image_scaled(&format!("car:{}", car.id), px - CAR_PHOTO.0 / 2.0, py - CAR_PHOTO.1 / 2.0, CAR_PHOTO.0, CAR_PHOTO.1, tint);
                b.clip_none();
            }
            // a car you may not drive in this division is shown dimmed (design choice)
            // (the garage's 3D car has no picture to dim, so a locked car is shaded the same way)
            if !g.car_allowed() || (garage && g.car_locked()) {
                b.clip_ellipse(cx, cy, rx, ry);
                b.rect(cx - rx, cy - ry, rx * 2.0, ry * 2.0, [0.0, 0.0, 0.0, if g.car_allowed() { 0.7 } else { 0.65 }]);
            }
            b.clip_outside(cx, cy, rx, ry);
            g.frame_set(b, CAR_SET, "car", 2, "scr8", 3);
            b.clip_none();
            // CLASSIC 0x46cc84..: the ring (slot 4) and the thick rim arc (slot 5) at their sprpos slots
            for slot in [4, 5] {
                g.slot_sprite(b, (CAR_SET, slot), if slot == 4 { "carc" } else { "cars" }, 0);
            }
            let car = &g.cars[g.settings.car];
            // CLASSIC 0x46d170: the name centred at slot 0x18 of set 3, with the blinking arrows
            let (nx, ny) = (g.sp().x(CAR_SET, 0x18) as f32, g.sp().y(CAR_SET, 0x18) as f32);
            menu_text(b, &car.name, nx, ny, MENU_ORANGE, Align::Centre);
            g.name_arrows(b, nx, ny, b.fnt_width(&car.name, 1.0), g.settings.car, g.locks().cars);
            // rows: car + transmission, car horn (step line), record race (tall drop); CLASSIC 0x46d106..0x46d12d:
            // slots 1..0xd, 0x11..0x12, 0x1b, 0x1f of set 3 (init 0x46cc5e)
            g.slot_sprite(b, (CAR_SET, 9), "carop", 0);
            g.slot_sprite(b, (CAR_SET, 0x1f), "caro2", 0);
            g.slot_sprite(b, (CAR_SET, 0x1b), "card", 0);
            g.bracket_label(b, (CAR_SET, 0xa), "tcar", g.sel == 0);
            g.bracket_label(b, (CAR_SET, 0xb), "ttran", g.sel == 1);
            g.bracket_label(b, (CAR_SET, 0xc), "thorn", g.sel == 2);
            g.bracket_label(b, (CAR_SET, 0xd), "trr", g.sel == 3);
            // CLASSIC slot 0x11 `tman`: frame 0 "manual", 7 "auto", rolling through the blurred frames between (0x752a18)
            g.slot_sprite(b, (CAR_SET, 0x11), "tman", g.rolls[ROLL_MANUAL].round() as usize);
            let horn = g.horns.get(g.settings.horn).map_or("horn", |h| h.name.as_str());
            menu_text(b, horn, g.sp().x(CAR_SET, 0x13) as f32, g.sp().y(CAR_SET, 0x13) as f32, MENU_ORANGE, Align::Left);
            g.onoff(b, (CAR_SET, 0x12), ROLL_RECORD);
            if g.game_type == GameType::TimeAttack {
                // DESIGN CHOICE: a text row under Record race (no original art for it)
                let c = if g.sel == 4 { ORANGE } else { WHITE };
                b.text("ghost car", 34.0, 340.0, 0.95, c);
                b.text(if g.settings.ghost_car { "on" } else { "off" }, 150.0, 340.0, 0.95, ORANGE);
            }
            // blinks like the original's (on while the 1024 ms clock is below 512, 0x46d25e)
            // "Not yet available" comes first (0x46d266 / 0x46d5e5), then the league's division rule; the label sits 5 px up (0x46d281)
            let label = if g.car_locked() { Some("Not yet available") } else if !g.car_allowed() { Some("Not available in division") } else { None };
            if let Some(text) = label.filter(|_| (g.clock * 1000.0) as u32 & 0x3ff < 0x200) {
                menu_text(b, text, g.sp().x(CAR_SET, 0x1e) as f32, g.sp().y(CAR_SET, 0x1e) as f32 - 5.0, MENU_ORANGE, Align::Centre);
            }
            // CLASSIC 0x46d2d6..0x46d4ea: the three dials with their fill masks, then the labels
            let fills = gauge_fills(g.gauges);
            for (gauge, v) in GAUGES.iter().zip(fills) {
                gauge.draw(g, b, v);
            }
            for gauge in &GAUGES {
                g.slot_sprite(b, (CAR_SET, gauge.label_slot), gauge.label, 0);
            }
        }));
        out
    }

    /// The in-race menu: centred text lines, "label value" like the original's "%s %s"
    /// (0x45d062), over a dimmed frame.
    fn pause_overlay(&self, b: &mut Batch) {
        let s = &self.settings;
        let pct = |v: u32| format!("{v}%");
        let lines: Vec<String> = match self.screen {
            Screen::PauseSfx => vec![
                format!("SFX Volume: {}", pct(s.vol_sfx)),
                format!("CD Volume: {}", pct(s.vol_music)),
                format!("Ambient Volume: {}", pct(s.vol_ambient)),
                "Back".into(),
            ],
            _ => PAUSE_ITEMS.iter().map(|x| x.to_string()).collect(),
        };
        let w = b.canvas.w;
        b.rect(0.0, 0.0, w, 480.0, [0.0, 0.0, 0.08, 0.6]);
        let scale = 1.25;
        let y0 = 240.0 - lines.len() as f32 * 13.0;
        for (i, l) in lines.iter().enumerate() {
            let sel = i == self.sel;
            let tw = b.text_width(l, scale);
            let y = y0 + i as f32 * 26.0;
            b.text(l, w * 0.5 - tw * 0.5, y, scale, if sel { ORANGE } else { WHITE });
        }
    }

    /// REMASTER in-race HUD Settings [design choice]: a short list in the centre column (the
    /// one the corner elements keep free), lightly dimmed so the HUD behind shows the changes.
    fn pause_hud(&self, b: &mut Batch, r: &RaceState) {
        let h = &self.settings.hud.shown(self.settings.info_detail);
        let onoff = |x: bool| if x { "On" } else { "Off" };
        let fit = hud::effective_scale(self, r, b);
        let scale = if fit + 0.005 < h.scale as f32 / 100.0 { format!("{}% (fits {:.0}%)", h.scale, fit * 100.0) } else { format!("{}%", h.scale) };
        let lines = [
            format!("Speed and revs: {}", onoff(h.cluster)),
            format!("Total time: {}", onoff(h.clocks[0])),
            format!("Lap time: {}", onoff(h.clocks[1])),
            format!("Best lap: {}", onoff(h.clocks[2])),
            format!("Lap: {}", onoff(h.lap)),
            format!("Position: {}", onoff(h.pos)),
            format!("Map: {}", onoff(h.map)),
            format!("Map view: {}", MAP_MODES[h.map_mode as usize]),
            format!("Map turns: {}", onoff(h.map_rotate)),
            format!("Map numbers: {}", onoff(h.map_numbers)),
            format!("Proximity arrows: {}", onoff(h.proximity)),
            format!("Scale: {scale}"),
            format!("Margin: {}%", h.margin),
            "Back".into(),
        ];
        let w = b.canvas.w;
        let (pw, ph) = (220.0_f32.max(w * 0.3), 26.0 + HUD_ROWS as f32 * 18.0 + 22.0);
        let (px, py) = (w * 0.5 - pw * 0.5, 240.0 - ph * 0.5);
        b.rect(px, py, pw, ph, [0.0, 0.0, 0.08, 0.85]);
        let title = "HUD Settings";
        b.text(title, w * 0.5 - b.text_width(title, 1.0) * 0.5, py + 6.0, 1.0, ORANGE);
        for (i, l) in lines.iter().enumerate() {
            let tw = b.text_width(l, 1.0);
            b.text(l, w * 0.5 - tw * 0.5, py + 26.0 + i as f32 * 18.0, 1.0, if i == self.sel { ORANGE } else { WHITE });
        }
        // the settings shape the Remaster HUD only
        if !self.settings.view.modern_hud() {
            let note = "Shown with the Remaster Modern HUD";
            b.text(note, w * 0.5 - b.text_width(note, 0.75) * 0.5, py + ph - 16.0, 0.75, [0.6, 0.6, 0.6, 1.0]);
        }
    }

    /// The in-race GFX Options [design choice]: the graphic options screen's panel over the
    /// race, so the changes show live around it. The original's in-race menu was a short
    /// centred list (0x45d062; it named the sky "Blank"/"Picture", 0xbcb7c).
    fn pause_graphics(&self, b: &mut Batch) {
        b.rect(112.0, 58.0, 500.0, 340.0, [0.02, 0.02, 0.12, 0.85]);
        b.text("GFX Options", 130.0, 64.0, 0.85, ORANGE);
        self.graphics_panel(b);
    }

    /// Options -> sound options, CLASSIC 0x469668 (sprpos set 6): the "volume control" frame with
    /// two dials, cd loop, and the "assign song to track" table: the race tracks, their CD track and
    /// its length, rows 32 px apart with the current one in the boxes. The ambient volume has no dial
    /// here (it is in the pause menu's SFX options and the Remaster Sound screen).
    fn sound_screen(&self, b: &mut Batch) {
        self.frame_set(b, SOUND_SET, "opt", 2, "scr2", 3);
        self.sound_device_line(b);
        let sp = self.sp();
        let at = |slot: usize| (sp.x(SOUND_SET, slot) as f32, sp.y(SOUND_SET, slot) as f32);
        // CLASSIC 0x46973d: slots 1..0x15 through 0x460880 (init 0x4695b4 copies `sndm` into slot 9 too)
        for (name, slot) in [("snd1", 4), ("snd4", 5), ("snd3", 6), ("sndm", 8), ("sndm", 9), ("sndt", 0xc), ("snda", 0xd), ("sndc", 0xe)] {
            self.slot_sprite(b, (SOUND_SET, slot), name, 0);
        }
        self.slot_sprite(b, (SOUND_SET, 0xa), "onoff", self.loop_frame.round() as usize);
        // the labels pulse by item: tsfx, tcdm, tcdl, sndb (slots 0xf..0x12)
        let s = &self.settings;
        for (name, slot) in [("tsfx", 0xf), ("tcdm", 0x10), ("tcdl", 0x11), ("sndb", 0x12)] {
            let (x, y) = at(slot);
            self.label(b, (SOUND_SET, slot), name, x, y, self.sel == slot - 0xf);
        }
        let on_table = self.sel == SOUND_TABLE;
        for (name, slot) in [("sndak", 0x13), ("sndck", 0x14), ("sndtk", 0x15)] {
            self.slot_sprite(b, (SOUND_SET, slot), name, on_table as usize);
        }
        // the needles (0x481d58), the CD music dial left (slot 0x19), the effects dial right (0x18)
        let colour = |item: usize| {
            let [r, g, bl] = MENU_PALETTE[if self.sel == item { 1 } else { 0 }];
            [r as f32 / 255.0, g as f32 / 255.0, bl as f32 / 255.0, 1.0]
        };
        for (pct, centre, item) in [(s.vol_music, at(0x19), 1), (s.vol_sfx, at(0x18), 0)] {
            dial_needle(b, centre.0, centre.1, 4912.0 - 33.84 * pct as f32, 1.0, colour(item));
        }
        // the table
        let (tx, ty) = at(0xb);
        let (x0, y0) = (tx + 53.0, ty + 93.0);
        let ink = if on_table { 2 } else { 1 };
        for role in 0..3 {
            let Some(i) = (self.sound_list + role).checked_sub(1).filter(|&i| i < self.tracks.len()) else { continue };
            let (name_dy, count_dy) = [(-28.0, -32.0), (2.0, -2.0), (32.0, 28.0)][role];
            menu_text(b, &self.tracks[i].name, x0 + 20.0, y0 + name_dy, ink, Align::Left);
            for (col, cx) in [x0 + 183.0, x0 + 304.0, x0 + 360.0].into_iter().enumerate() {
                let at = (role * 3 + col) * 2;
                flip_counter(b, cx, y0 + count_dy, &self.sound_roll[at..at + 2]);
            }
        }
    }

    /// The sound device line under the screen title (#260): "Sound device:" and the output's name, as the p3.0 preview
    /// video shows it (`Sound device: < Direct Sound Driver >`; retail has no such string, 05-formats). Label colours 3 / 2
    /// and the chosen name black on a white bar when the line is the selected item, as the lists (0x461e74); a pick that
    /// is not available reads lavender with a note. Position is a design choice: the video's layout is not the retail one.
    fn sound_device_line(&self, b: &mut Batch) {
        const LABEL: &str = "Sound device:";
        let (x, y) = (6.0, 66.0);
        let (sel, v) = (self.sel == SOUND_DEVICE, self.sound_device_view());
        menu_text(b, LABEL, x, y, if sel { 2 } else { 3 }, Align::Left);
        let x0 = x + b.fnt_width(LABEL, 1.0) + 24.0;
        let text = if v.unavailable { format!("{} (not available)", v.text) } else { v.text.clone() };
        // FONT1 has no middle dot (the names use it as a separator) and stops at Latin-1
        let text: String = text.chars().map(|c| if c == '·' { '-' } else if (c as u32) > 0xff { '?' } else { c }).collect();
        let text = clip_text(b, &text, 560.0 - x0, false);
        let w = b.fnt_width(&text, 1.0);
        if sel && !v.unavailable {
            menu_bar(b, x0, y, w, 2);
            menu_text(b, &text, x0, y, MENU_BLACK, Align::Left);
        } else {
            menu_text(b, &text, x0, y, if v.unavailable { MENU_LAVENDER } else if sel { 2 } else { 1 }, Align::Left);
        }
        if sel {
            // the blinking arrows of a stepped name (0x46c5f3), none at an end of the list
            self.name_arrows_at(b, x0 + (w / 2.0).floor(), y, w, !v.left_end, !v.right_end);
        }
    }

    /// CLASSIC 0x468f10, sprpos set 8: the layouts as a column on the left (the one in use white, in
    /// the `selco` bracket), the ten tasks as a bracket tree (`ctrla` `[` and `ctrlb` `}` per row, the
    /// task's sprite, the control's name in FONT1). Left / Right choose which of the two the cursor is in.
    fn controls_screen(&self, b: &mut Batch) {
        self.frame_set(b, CONTROLS_SET, "opt", 2, "scr3", 3);
        let sp = self.sp();
        let at = |slot: usize| (sp.x(CONTROLS_SET, slot) as f32, sp.y(CONTROLS_SET, slot) as f32);
        let l = self.controls.layout();
        // selco: frame 0 = the column has the cursor, 1 = the tree
        let (sx, sy) = at(4);
        self.slot_menu(b, (CONTROLS_SET, 4), "selco", usize::from(self.ctl_tasks), sx, sy);
        // the column: the name in use at slot 4 + slot 0x15, the others slot 0x16 y apart, 4 each way
        let (nx, ny) = (sx + at(0x15).0, sy + at(0x15).1);
        let (pitch, window) = (at(0x16).1, (at(0x16).0 / 8.0) as usize);
        let cur = self.controls.active;
        for (i, layout) in self.controls.layouts.iter().enumerate() {
            if i.abs_diff(cur) <= window {
                let y = ny + (i as f32 - cur as f32) * pitch;
                menu_text(b, &layout.name, nx, y, if i == cur { MENU_WHITE } else { MENU_ORANGE }, Align::Left);
            }
        }
        // the tree: rows slot 0x1e y apart from slot 0x18 y; the key names at slot 0x19 x + 0x2c
        let (bx, by) = at(0x18);
        let (cx, _) = at(0x19);
        let (ox, row_pitch) = (bx + at(0x1d).0, at(0x1e).1);
        for (i, (&task, name)) in CLASSIC_TASKS.iter().zip(CLASSIC_LABELS).enumerate() {
            let y = by + i as f32 * row_pitch;
            // CLASSIC 0x468fd7 / 0x469023: the row's two bars are slots 0x18 / 0x19, the task sprite slot 5 + row
            self.slot_menu(b, (CONTROLS_SET, 0x18), "ctrla", 0, bx, y);
            self.slot_menu(b, (CONTROLS_SET, 0x19), "ctrlb", 0, cx, y);
            let on = self.ctl_tasks && self.sel == i;
            // the selected task's sprite pulses through its 8 frames, 512 ms a cycle (0x468f33)
            let frame = if on { (3.5 + 3.5 * (self.clock * std::f32::consts::TAU / 0.512).sin()) as usize } else { 0 };
            self.slot_menu(b, (CONTROLS_SET, 5 + i), name, frame.min(7), ox, y);
            let key = match self.capture {
                Some(c) if c.task == task => if ((self.clock * 3.0) as u32).is_multiple_of(2) { "?".to_string() } else { String::new() },
                _ => l.bind[task].map_or("-".into(), |c| c.name()),
            };
            menu_text(b, &key, cx + 44.0, y + 4.0, if self.capture.is_some_and(|c| c.task == task) { MENU_WHITE } else { MENU_ORANGE }, Align::Left);
        }
    }

    /// Personal options: name, team, colours and the ten Key strings (config "Personal" block),
    /// then our own rows, in the graphic options' text-list style.
    fn personal_screen(&self, b: &mut Batch) {
        self.frame_set(b, PERSONAL_SET, "opt", 2, "scrd", 3);
        let s = &self.settings;
        // CLASSIC 0x466b70, sprpos set 17: labels at slot 0x14 x, the first row at its y, a row
        // `slot 0x15 y` below the other, the values at slot 0x15 x. Rows 0..=16 are the original's;
        // ours (Confirmations, Intro movies, Statistics, Back) follow in a column wide enough for
        // their labels. The menu canvas is 400 px tall, so only `PERSONAL_SLOTS` rows show at once and
        // the list scrolls with the cursor, as the graphic options' list does [design choice].
        let sp = self.sp();
        let (lx, y0) = (sp.x(PERSONAL_SET, 0x14) as f32, sp.y(PERSONAL_SET, 0x14) as f32);
        let pitch = sp.y(PERSONAL_SET, 0x15) as f32;
        let vx = sp.x(PERSONAL_SET, 0x15) as f32;
        let vx_own = vx.max(lx + b.fnt_width("Confirmations:", 1.0) + 12.0);
        let custom = s.custom_car_colours;
        let cursor = if ((self.clock * 3.0) as u32).is_multiple_of(2) { "_" } else { "" };
        let mr = |row: usize| MenuRow { sel: self.sel == row, on: true };
        // a text row: an empty one shows nothing (0x466c28); a long Key string shows its head, or
        // its tail with the caret while typed into, within the field's 440 px (0x46733a)
        let text = |b: &Batch, row: usize, v: &str| match &self.editing {
            Some((r, _)) if *r == row => format!("{}{cursor}", clip_text(b, v, 440.0 - b.fnt_width("_", 1.0), true)),
            _ => clip_text(b, v, 440.0, false),
        };
        let rows = personal_rows(custom);
        let at = rows.iter().position(|&r| r == self.sel).unwrap_or(0);
        let scroll = at.saturating_sub(PERSONAL_SLOTS - 1);
        for (slot, &row) in rows.iter().enumerate().skip(scroll).take(PERSONAL_SLOTS) {
            let y = y0 + (slot - scroll) as f32 * pitch;
            let line = |b: &mut Batch, x: f32, label: &str, choices: &[&str], cur: usize| {
                menu_label(b, label, lx, y, mr(row));
                menu_choices(b, choices, cur, x, y, mr(row));
            };
            match row {
                0 => {
                    let v = text(b, row, &s.player_name);
                    line(b, vx, "Name:", &[&v], usize::MAX);
                }
                1 => {
                    let v = text(b, row, &s.team);
                    line(b, vx, "Team:", &[&v], usize::MAX);
                }
                2 => {
                    menu_label(b, "Colour:", lx, y, mr(row));
                    colour_strip(b, &self.strip, s.player_colour, vx, y, self.sel == row);
                }
                3..=12 => {
                    let v = text(b, row, &s.short_keys[row - 3]);
                    line(b, vx, &format!("Key {}:", row - 3), &[&v], usize::MAX);
                }
                // 0x466cd8: only the current word, the cursor turns it with Left / Right
                13 => line(b, vx, "Car colours:", &[if custom { "On" } else { "Off" }], usize::MAX),
                14..=16 => {
                    menu_label(b, &format!("Car colour {}:", row - 13), lx, y, mr(row));
                    colour_strip(b, &self.strip, s.car_colours[row - 14], vx, y, self.sel == row);
                }
                // the HUD rows (info detail, checkpoints, road map) are in the graphic options' HUD page
                17 => line(b, vx_own, "Confirmations:", &CONFIRMATIONS, s.menu_confirmation),
                18 => line(b, vx_own, "Intro movies:", &["Off", "On"], s.intro as usize),
                19 => line(b, vx_own, "Statistics:", &["Open"], usize::MAX),
                _ => {
                    if self.sel == row {
                        let w = b.fnt_width("Back", 1.0);
                        menu_bar(b, lx, y, w, 2);
                        menu_text(b, "Back", lx, y, MENU_BLACK, Align::Left);
                    } else {
                        menu_text(b, "Back", lx, y, MENU_BLUE, Align::Left);
                    }
                }
            }
        }
        // between the title and the first row
        if self.editing.is_some() {
            menu_text(b, "type, then enter: keep   esc: undo", lx, y0 - 20.0, MENU_BLUE, Align::Left);
        }
    }

    /// Rows of the current statistics page when it is a list (circuits, cars).
    fn stats_list_len(&self) -> usize {
        match self.stats_page {
            2 => self.stats.tracks.len(),
            3 => self.stats.cars.len(),
            _ => 0,
        }
    }

    /// Statistics (stats.rs): four pages in the personal options' text-list style.
    fn statistics_screen(&self, b: &mut Batch) {
        // design choice: no original screen; Personal's title and box slots, the Options box (`scr1`)
        self.frame_set(b, PERSONAL_SET, "opt", 2, "scr1", 3);
        let st = &self.stats;
        // CLASSIC: FONT1 in the menu colours on our own layout (the original has no such screen):
        // the page row as the personal options' rows, labels colour 3, values colour 1
        menu_label(b, "Statistics:", 54.0, 78.0, MenuRow { sel: true, on: true });
        menu_choices(b, &STATS_PAGES, self.stats_page, 180.0, 78.0, MenuRow { sel: true, on: true });
        let pct = |v: Option<f32>| v.map_or("-".to_string(), |v| format!("{:.0} %", v * 100.0));
        let km = |m: f64| format!("{:.1} km", m / 1000.0);
        let lap = |ms: u32| fmt_rank_time(ms);
        let car_name = |id: u32| self.cars.iter().find(|c| c.id == id).map_or(format!("#{id}"), |c| c.name.clone());
        let rows: Vec<(&str, String)> = match self.stats_page {
            0 => vec![
                ("Races:", st.races.to_string()),
                ("Wins:", format!("{} ({})", st.wins, pct(st.win_rate()))),
                ("Podiums:", format!("{} ({})", st.podiums, pct(st.podium_rate()))),
                ("Clean races:", format!("{} ({})", st.clean, pct(st.clean_rate()))),
                ("Time racing:", fmt_hms(st.total.race_s)),
                ("Play time:", fmt_hms(st.play_s)),
                ("Favourite:", st.favourite().map_or("-".into(), |(n, t)| format!("{n} ({}x)", t.plays))),
                ("Fastest lap:", st.fastest().map_or("-".into(), |(ms, n, c)| format!("{} {n}, {}", lap(ms), car_name(c)))),
                ("Top speed:", st.top_speed.as_ref().map_or("-".into(), |t| format!("{:.0} km/h {}, {}", t.kmh, t.track, car_name(t.car)))),
            ],
            1 => vec![
                ("Distance:", km(st.total.distance_m)),
                ("Overtakes:", st.total.overtakes.to_string()),
                ("Car collisions:", st.total.car_hits.to_string()),
                ("Wall collisions:", st.total.wall_hits.to_string()),
                ("Drift distance:", km(st.total.drift_m)),
                ("Longest drift:", format!("{:.0} m", st.longest_drift_m)),
                ("Air time:", format!("{:.1} s", st.total.air_s)),
                ("Jumps:", st.total.jumps.to_string()),
                ("Longest jump:", format!("{:.2} s", st.longest_jump_s)),
            ],
            _ => Vec::new(),
        };
        let mut y = 104.0;
        for (label, value) in rows {
            menu_text(b, label, 54.0, y, MENU_BLUE, Align::Left);
            // a long value (the fastest lap, the top speed) wraps under the labels' column
            for (k, line) in wrap_fnt(b, &value, 640.0 - 54.0 - 190.0).iter().enumerate() {
                menu_text(b, line, 190.0, y + k as f32 * 15.0, MENU_ORANGE, Align::Left);
                if k > 0 {
                    y += 15.0;
                }
            }
            y += 17.0;
        }
        if self.stats_page >= 2 {
            let cols = [54.0, 230.0, 290.0, 410.0, 520.0];
            let head = if self.stats_page == 2 { "Circuit" } else { "Car" };
            for (x, h) in cols.iter().zip([head, "Raced", "Best lap", "Distance", "Time"]) {
                menu_text(b, h, *x, y, MENU_BLUE, Align::Left);
            }
            let favourite = st.favourite().map(|f| f.0);
            // (name, raced, best lap, distance, time, highlighted)
            let lines: Vec<(String, u32, u32, f64, f64, bool)> = if self.stats_page == 2 {
                st.tracks.iter().map(|(n, t)| (n.clone(), t.plays, t.best_lap.map_or(0, |l| l.0), t.distance_m, t.race_s, favourite == Some(n.as_str()))).collect()
            } else {
                st.cars.iter().map(|(&id, c)| (car_name(id), c.races, c.best_lap.as_ref().map_or(0, |l| l.0), c.distance_m, c.race_s, false)).collect()
            };
            if lines.is_empty() {
                menu_text(b, "-", cols[0], y + 17.0, MENU_LAVENDER, Align::Left);
            }
            for (k, l) in lines.iter().skip(self.stats_scroll).take(STATS_ROWS).enumerate() {
                let ly = y + 17.0 + k as f32 * 15.0;
                let c = if l.5 { MENU_ORANGE } else { 2 };
                menu_text(b, &l.0, cols[0], ly, c, Align::Left);
                menu_text(b, &l.1.to_string(), cols[1], ly, c, Align::Left);
                menu_text(b, &lap(l.2), cols[2], ly, c, Align::Left);
                menu_text(b, &km(l.3), cols[3], ly, c, Align::Left);
                menu_text(b, &fmt_hms(l.4), cols[4], ly, c, Align::Left);
            }
        }
        menu_text(b, "left/right: page    up/down: scroll    esc: back", 54.0, 420.0, MENU_LAVENDER, Align::Left);
    }

    /// CLASSIC 0x466080, sprpos set 11: the four filters are vertical lists under their label
    /// sprites (sprites at slots 4..7 = ranka, rankb, rankd, ranke, list columns at slots 0x18..0x1b, first
    /// item at slot 0x18's y, 15 px apart; the track and laps lists show 4 items, the others 10); a filter the
    /// table kind does not use shows "----" (0x4bdca0). The table (0x4662a0) takes its columns from slots
    /// 0x10..0x13: name, car, a time by 0x460468, a second time (best laptime: "----" at slot 0x13's x + 0x50);
    /// the first row at slot 0x10's y, 15 px apart, no row numbers. The shipped game has no Mode (mirror) column
    /// (slot 8 is left empty at 0x465fc1). The league view (kind 3, Track label = "Division" = rankb frame + 2,
    /// car column "----") is dead code in the original and opened by ours [design choice, RANK_LEAGUE].
    fn rankings_screen(&self, b: &mut Batch) {
        // CLASSIC 0x465f51: the Rankings init loads rank + scr6 ("Ranking" tag)
        self.frame_set(b, RANKINGS_SET, "rank", 2, "scr6", 3);
        let sp = self.sp();
        let at = |slot: usize| (sp.x(RANKINGS_SET, slot) as f32, sp.y(RANKINGS_SET, slot) as f32);
        let v = self.rank;
        let league = v.kind == RANK_LEAGUE;
        let kinds = [hiscore::Kind::BestLap, hiscore::Kind::BestRace, hiscore::Kind::SingleRace].map(hiscore::Kind::label);
        let places: Vec<&str> = if league {
            RANK_DIVISIONS.iter().map(|k| k.label()).collect()
        } else {
            self.tracks.iter().map(|t| t.name.as_str()).collect()
        };
        let laps: Vec<String> = RANK_LAPS.iter().map(|l| l.to_string()).collect();
        let laps: Vec<&str> = laps.iter().map(String::as_str).collect();
        let diff = [Difficulty::Easy, Difficulty::Medium, Difficulty::Hard, Difficulty::Beginner].iter().position(|&d| d == v.diff).unwrap_or(1);
        let mut levels = vec!["Easy", "Medium", "Hard"];
        if self.settings.view.modern_menu() {
            levels.push("Beginner"); // ours
        }
        // (label sprite, sprite frame offset, items, current, window)
        let rows: [(&str, usize, Vec<&str>, usize, usize); 4] = [
            ("ranka", 0, kinds.to_vec(), v.kind, 10),
            ("rankb", if league { 2 } else { 0 }, places, if league { v.division } else { v.track }, 4),
            ("rankd", 0, laps, v.laps, 4),
            ("ranke", 0, levels, diff, 10),
        ];
        let used = self.rank_rows();
        let sel = used.get(self.sel.min(used.len() - 1)).copied();
        for (row, (sprite, frame, items, cur, n)) in rows.iter().enumerate() {
            let selected = sel == Some(row);
            self.slot_sprite(b, (RANKINGS_SET, 4 + row), sprite, frame + if selected { 0 } else { 1 });
            let (x, y) = (at(0x18 + row).0, at(0x18).1);
            if used.contains(&row) {
                menu_list(b, items, *cur, x, y, *n, selected);
            } else {
                menu_text(b, "----", x, y, MENU_ORANGE, Align::Left);
            }
        }
        self.slot_sprite(b, (RANKINGS_SET, 9), "rankz", 0);
        let Some(t) = &self.rank_table else { return };
        let me = &self.settings.player_name;
        let lap_only = v.kind == 0;
        let ((nx, y0), (cx, pitch), (t1, _), (t2, _)) = (at(0x10), at(0x11), at(0x12), at(0x13));
        for (i, e) in t.entries.iter().enumerate() {
            let y = y0 + i as f32 * pitch;
            // our rows: the player's own entries white [design choice]
            let c = if self.persist && &e.name == me { 2 } else { MENU_ORANGE };
            menu_text(b, &e.name, nx, y, c, Align::Left);
            menu_text(b, if league { "----" } else { &e.car }, cx, y, c, Align::Left);
            menu_time(b, e.lap_ms, t1, y, c);
            if lap_only {
                menu_text(b, "----", t2 + 80.0, y, MENU_ORANGE, Align::Left);
            } else {
                menu_time(b, e.race_ms, t2, y, c);
            }
        }
    }

    /// The league info box under the dial: League, Name, Division, Difficulty, Score, Track.
    fn league_info_rows(&self) -> Option<menu_modern::DialInfo> {
        let l = self.current_league()?;
        let me = l.human().map(|i| &l.places[i]);
        let codes = self.track_codes();
        let track = l
            .next_track(&codes)
            .map_or("-".to_string(), |t| self.tracks[t].name.clone());
        Some([
            [
                ("League:", l.name.clone()),
                ("Name:", me.map_or(String::new(), |p| p.p.name.clone())),
                ("Division:", l.human_division().to_string()),
            ],
            [
                (
                    "Difficulty:",
                    difficulty_name(self.settings.difficulty).to_string(),
                ),
                ("Score:", me.map_or(0, |p| p.score).to_string()),
                ("Track:", track),
            ],
        ])
    }

    /// CLASSIC 0x46aacc, sprpos set 12: the info box under the dial. Origin = slots 0xb + 0x14,
    /// rows slot 0x15 y apart, the right labels slot 0x15 x on, the values slot 0x16 x after their
    /// label column (left) / slot 0x16 y after it (right). Labels colour 3, values colour 1.
    /// (The original shows "-" for the difficulty [O]; ours shows it.)
    fn league_info(&self, b: &mut Batch) {
        let Some([left, right]) = self.league_info_rows() else {
            return;
        };
        let sp = self.sp();
        let (gx, gy) = (|slot| sp.x(LEAGUE_SET, slot) as f32, |slot| sp.y(LEAGUE_SET, slot) as f32);
        let (x0, y0) = (gx(0xb) + gx(0x14), gy(0xb) + gy(0x14));
        let pitch = gy(0x15);
        let (rx, lv) = (x0 + gx(0x15), x0 + gx(0x16));
        let rv = rx + gy(0x16);
        for (i, ((lk, lval), (rk, rval))) in left.iter().zip(right.iter()).enumerate() {
            let y = y0 + i as f32 * pitch;
            menu_text(b, lk, x0, y, MENU_BLUE, Align::Left);
            menu_text(b, rk, rx, y, MENU_BLUE, Align::Left);
            menu_text(b, lval, lv, y, MENU_ORANGE, Align::Left);
            menu_text(b, rval, rv, y, MENU_ORANGE, Align::Left);
        }
    }

    fn league_menu(&self, b: &mut Batch) {
        self.dial_frame(b, "leagu", "scrc");
        self.dial(b, &LEAGUE_ITEMS);
        self.league_info(b);
    }

    /// CLASSIC 0x46ae94, sprpos set 16: the league header (labels colour 3 at slot 0x10, rows slot
    /// 0x11 y apart, the right column slot 0x11 x on, values at slot 0x12 x) and the division's
    /// table: headers at slot 0x13 (Pos, Nick at +slot 0x14 x, Score at +4 columns, 1st / 2nd / 3rd
    /// at +5, 6, 7), the rows 14 px apart from y + 6 + 14. The original shows the nick only and
    /// colours a row by the player's car colour; ours: the human white [design choice].
    fn league_table(&self, b: &mut Batch) {
        self.frame_set(b, STANDINGS_SET, "leagu", 3, "scrc", 2);
        let Some(l) = self.current_league() else { return };
        let sp = self.sp();
        let (gx, gy) = (|slot| sp.x(STANDINGS_SET, slot) as f32, |slot| sp.y(STANDINGS_SET, slot) as f32);
        let codes = self.track_codes();
        let d = self.league_div;
        let next = l.next_track(&codes).map_or("-".to_string(), |t| self.tracks[t].name.clone());
        let total = league::season_tracks(&codes, l.human_division()).len();
        let me = l.human().map(|i| &l.places[i]);
        let (x0, y0, dx, dy) = (gx(0x10), gy(0x10), gx(0x11), gy(0x11));
        let (vx, vx2) = (gx(0x12), gx(0x12) + dx);
        for (label, x, row) in [("League name:", x0, 0.0), ("Races done:", x0 + dx, 0.0), ("Division:", x0, 1.0), ("Next track:", x0 + dx, 1.0), ("Player name:", x0, 2.0), ("Car:", x0 + dx, 2.0)] {
            menu_text(b, label, x, y0 + row * dy, MENU_BLUE, Align::Left);
        }
        let y = |row: f32| y0 + row * dy;
        menu_text(b, &l.name, vx, y(0.0), MENU_ORANGE, Align::Left);
        menu_text(b, &format!("{} / {total}", l.races_done), vx2, y(0.0), MENU_ORANGE, Align::Left);
        menu_text(b, &d.to_string(), vx, y(1.0), MENU_ORANGE, Align::Left);
        menu_text(b, &next, vx2, y(1.0), MENU_ORANGE, Align::Left);
        menu_text(b, me.map_or("", |p| p.p.name.as_str()), vx, y(2.0), MENU_ORANGE, Align::Left);
        menu_text(b, me.map_or("", |p| p.p.cars[d].as_str()), vx2, y(2.0), MENU_ORANGE, Align::Left);
        // the division shown steps with left / right [design choice, no original]
        menu_text(b, &format!("< Division {d} >"), 19.0, gy(0x13) - 24.0, MENU_LAVENDER, Align::Left);
        let (tx, ty, col) = (gx(0x13), gy(0x13), gx(0x14));
        for (h, k) in [("Pos:", 0.0), ("Nick:", 1.0), ("Score:", 4.0), ("1", 5.0), ("2", 6.0), ("3", 7.0)] {
            menu_text(b, h, tx + col * k, ty, MENU_BLUE, Align::Left);
        }
        for (k, pl) in l.division(d).iter().enumerate() {
            let y = ty + 6.0 + (k + 1) as f32 * dy;
            let c = if pl.human { 2 } else { MENU_ORANGE };
            let cells = [(format!("{}", k + 1), 0.0), (pl.p.nick.clone(), 1.0), (pl.score.to_string(), 4.0), (pl.wins.to_string(), 5.0), (pl.seconds.to_string(), 6.0), (pl.thirds.to_string(), 7.0)];
            for (t, cx) in cells {
                menu_text(b, &t, tx + col * cx, y, c, Align::Left);
            }
        }
    }

    /// The Ghost screen, CLASSIC 0x462bc8 (sprpos set 19): title `ghost` at slot 3, the SCREEN box `scre` at 2, the
    /// modes `tgr` "ghost race" / `trpr` "replay race" at 4 / 5, the list tab `fghos` at 6, the names of the recordings
    /// at slot 0x18 (16 rows, `mupp` bar at x - 7 on each) and the hint at 0x15. The focused row pulses; while the
    /// list has the focus the mode it acts on shows frame 4, the tab and the bar frame 0 (frame 1 otherwise) and the
    /// list is white (orange otherwise). The recording's details in the left column and the messages are ours.
    fn replays_screen(&self, b: &mut Batch) {
        let sp = self.sp();
        let at = |slot: usize| (sp.x(REPLAYS_SET, slot) as f32, sp.y(REPLAYS_SET, slot) as f32);
        self.frame_at(b, REPLAYS_SET, ("ghost", 3), ("scre", 2));
        let listed = self.ghost_row == 2;
        for (i, name) in ["tgr", "trpr"].into_iter().enumerate() {
            let (x, y) = at(4 + i);
            if listed && i == self.ghost_mode {
                self.slot_menu(b, (REPLAYS_SET, 4 + i), name, 4, x, y);
            } else {
                self.label(b, (REPLAYS_SET, 4 + i), name, x, y, self.ghost_row == i);
            }
        }
        self.slot_sprite(b, (REPLAYS_SET, 6), "fghos", usize::from(!listed));
        let (lx, ly) = at(0x18);
        let names: Vec<String> = self.replays.iter().map(|(p, _)| demo_name(p)).collect();
        let items: Vec<&str> = names.iter().map(String::as_str).collect();
        let first = self.sel.min(items.len()).saturating_sub(8);
        for k in 0..items.len().saturating_sub(first).min(16) {
            b.menu("mupp", usize::from(!listed), lx - 7.0, ly - 1.0 + k as f32 * 15.0);
        }
        menu_list(b, &items, self.sel, lx, ly, 16, listed);
        let (hx, hy) = at(0x15);
        menu_text(b, "Press: [Delete] To remove race file.", hx, hy, MENU_ORANGE, Align::Left);
        // design choice (#246): the selected recording's details, the refusals and the empty list's advice fill the
        // left column under the modes
        let (cx, cy) = (at(4).0, 175.0);
        if let Some(m) = &self.replay_msg {
            for (i, line) in wrap_fnt(b, m, 190.0).iter().enumerate() {
                menu_text(b, line, cx, cy + i as f32 * 13.0, MENU_ORANGE, Align::Left);
            }
        } else if let Some((_, m)) = self.replays.get(self.sel) {
            let h = &m.header;
            let car = &h.cars[(h.player as usize).min(mde::CAR_ENTRIES - 1)];
            let rows = [
                ("Track:", h.track.clone()),
                ("Car:", car.car.clone()),
                ("Time:", fmt_rank_time((m.seconds() * 1000.0) as u32)),
                ("Laps:", h.laps.to_string()),
            ];
            for (i, (k, v)) in rows.iter().enumerate() {
                menu_text(b, k, cx, cy + i as f32 * 28.0, MENU_BLUE, Align::Left);
                menu_text(b, v, cx, cy + i as f32 * 28.0 + 13.0, MENU_ORANGE, Align::Left);
            }
        } else {
            for (i, line) in wrap_fnt(b, "No recordings yet. Switch on Record race on the car screen.", 190.0).iter().enumerate() {
                menu_text(b, line, cx, cy + i as f32 * 13.0, MENU_ORANGE, Align::Left);
            }
        }
    }

    /// The Multi page, CLASSIC 0x463e04 (sprpos set 9): title `multi` at slot 3, the SCREEN box `scrb` at 2, the rows
    /// `tjoin` "join an existing game" and `tcrea` "create session" at 4 / 5. The refusal line is ours (story #7).
    fn multi_screen(&self, b: &mut Batch) {
        let sp = self.sp();
        let at = |slot: usize| (sp.x(MULTI_SET, slot) as f32, sp.y(MULTI_SET, slot) as f32);
        self.frame_at(b, MULTI_SET, ("multi", 3), ("scrb", 2));
        for (i, name) in ["tjoin", "tcrea"].into_iter().enumerate() {
            let (x, y) = at(4 + i);
            self.label(b, (MULTI_SET, 4 + i), name, x, y, self.sel == i);
        }
        if let Some(m) = &self.multi_msg {
            menu_text(b, m, at(4).0, at(5).1 + 44.0, MENU_ORANGE, Align::Left);
        }
    }

    /// Load: the saved leagues, the selected one's details at the right. FONT1 on our layout (the
    /// original's screen, set 14 (0x46b9a4), is a list of sprites [O])).
    fn league_load(&self, b: &mut Batch) {
        self.frame_set(b, LEAGUE_LOAD_SET, "leagu", 3, "scrc", 2);
        let codes = self.track_codes();
        for (i, l) in self.leagues.iter().enumerate().take(12) {
            let y = 92.0 + i as f32 * 18.0;
            let c = if i == self.league { MENU_ORANGE } else { 2 };
            if i == self.sel {
                menu_bar(b, 44.0, y, b.fnt_width(&l.name, 1.0), c);
                menu_text(b, &l.name, 44.0, y, MENU_BLACK, Align::Left);
            } else {
                menu_text(b, &l.name, 44.0, y, c, Align::Left);
            }
        }
        let Some(l) = self.leagues.get(self.sel) else { return };
        let me = l.human().map(|i| &l.places[i]);
        let date = l.create_date;
        let rows = [
            ("League:", l.name.clone()),
            ("Date:", format!("{}-{:02}-{:02}", date / 10000, date / 100 % 100, date % 100)),
            ("Name:", me.map_or(String::new(), |p| p.p.name.clone())),
            ("Division:", l.human_division().to_string()),
            ("Next Track:", l.next_track(&codes).map_or("-".to_string(), |t| self.tracks[t].name.clone())),
            ("Score:", me.map_or(0, |p| p.score).to_string()),
        ];
        for (i, (k, v)) in rows.iter().enumerate() {
            let y = 92.0 + i as f32 * 18.0;
            menu_text(b, k, 330.0, y, MENU_BLUE, Align::Left);
            menu_text(b, v, 460.0, y, MENU_ORANGE, Align::Left);
        }
    }

    /// After a league race (0x42a2b4): your division on LResults.TGA, places in its portrait
    /// boxes (even places left, odd right, 80 px apart) with Nick and Score; the season's
    /// result under it when the season is over. Text positions as the original: the name at
    /// x 60 / 375 (y 85 / 125 + 80 per pair), labels at 125 / 430 (Nick y 30 / 70, Score 40 / 80),
    /// values at 200 / 500. Its colour is palette index 0xff, white [I]; the human is orange (ours).
    fn league_results(&self, b: &mut Batch) {
        b.image("bg:lresults", 0.0, 0.0);
        let Some(l) = self.current_league() else { return };
        let d = l.human_division();
        for (pos, pl) in l.division(d).iter().enumerate() {
            let (px, py) = if pos % 2 == 0 { (18.0, 22.0 + 40.0 * pos as f32) } else { (328.0, 62.0 + 40.0 * (pos - 1) as f32) };
            b.image_scaled(&pic_key(&pl.p.picture), px, py, 90.0, 56.0, WHITE);
            let c = if pl.human { MENU_ORANGE } else { 2 };
            let (name_x, label_x, value_x) = if pos % 2 == 0 { (60.0, 125.0, 200.0) } else { (375.0, 430.0, 500.0) };
            let base = 80.0 * (pos / 2) as f32 + if pos % 2 == 0 { 0.0 } else { 40.0 };
            menu_text(b, &pl.p.name, name_x, 85.0 + base, c, Align::Left);
            for (i, (k, v)) in [("Nick:", pl.p.nick.clone()), ("Score:", pl.score.to_string())].iter().enumerate() {
                let y = 30.0 + base + i as f32 * 10.0;
                menu_text(b, k, label_x, y, 2, Align::Left);
                menu_text(b, v, value_x, y, c, Align::Left);
            }
        }
        // the original plays a movie after this screen (0x42aa18) and shows no text; without movies.pak
        // the outcome is a line of text [design choice]
        let msg = match self.league_outcome.filter(|_| !self.has_movies) {
            Some(Outcome::Promoted(3)) => "Promoted to Division 2!",
            Some(Outcome::Promoted(2)) => "Promoted to Division 1!",
            Some(Outcome::Promoted(_)) => "On to the final!",
            Some(Outcome::Won) => "You won the league!",
            Some(Outcome::Relegated) => "Relegated.",
            Some(Outcome::LostFinal) => "You lost the final.",
            Some(Outcome::Stays) => "Season over.",
            _ => "",
        };
        // centred on a dark band over the portraits so it can't be missed
        if !msg.is_empty() {
            b.rect(0.0, 188.0, 640.0, 24.0, [0.0, 0.0, 0.0, 0.75]);
            menu_text(b, msg, 320.0, 192.0, MENU_ORANGE, Align::Centre);
        }
    }

    /// CLASSIC 0x429700: the places on Results.TGA, odd ones left, even right, 80 px apart (the
    /// right ones 40 lower). Text at the original's positions: the name at x 60 / 375 (y 85 / 125,
    /// then 80 per pair), the labels at x 125 / 430 (Nick at y 30 / 70, then every 10 px), the
    /// values at x 210 / 510, the nick and score one plus sign further because the times start
    /// with a plus or a space. The colour is palette index 0xff, white [I].
    fn results_screen(&self, b: &mut Batch) {
        b.image("bg:results", 0.0, 0.0);
        let Some(r) = &self.race else { return };
        let winner = self.results.first().and_then(|x| x.2);
        let plus = b.fnt_width("+", 1.0);
        for (pos, &(slot, pts, time, best)) in self.results.iter().enumerate().take(8) {
            let racer = &r.sess.race.racers[slot];
            // portrait boxes in Results.TGA: odd positions left, even right, 80 px apart
            let (px, py) = if pos % 2 == 0 { (18.0, 22.0 + 40.0 * pos as f32) } else { (328.0, 62.0 + 40.0 * (pos - 1) as f32) };
            b.image_scaled(&pic_key(&racer.driver.picture), px, py, 90.0, 56.0, WHITE);
            let race_s = match (time, winner) {
                (Some(t), Some(w)) if pos > 0 => format!("+{}", fmt_rank_time(t.saturating_sub(w))),
                (Some(t), _) => format!(" {}", fmt_rank_time(t)),
                (None, _) if racer.progress.eliminated_ms.is_some() => format!("OUT lap {}", racer.progress.lap), // Knockout
                (None, _) => "--".into(),
            };
            let rows = [
                ("Nick:", racer.driver.nick.clone()),
                ("Score:", pts.to_string()),
                ("Race:", race_s),
                ("Best Lap:", best.map_or("--".into(), |t| format!(" {}", fmt_rank_time(t)))),
            ];
            let (name_x, label_x, value_x) = if pos % 2 == 0 { (60.0, 125.0, 210.0) } else { (375.0, 430.0, 510.0) };
            let base = 80.0 * (pos / 2) as f32 + if pos % 2 == 0 { 0.0 } else { 40.0 };
            for (i, (k, v)) in rows.iter().enumerate() {
                let y = 30.0 + base + i as f32 * 10.0;
                menu_text(b, k, label_x, y, 2, Align::Left);
                menu_text(b, v, value_x + if i < 2 { plus } else { 0.0 }, y, 2, Align::Left);
            }
            menu_text(b, &racer.driver.name, name_x, 85.0 + base, 2, Align::Left);
        }
    }
}

/// Break `text` at spaces into lines no wider than `width` at `scale`.
fn wrap_text(b: &Batch, text: &str, scale: f32, width: f32) -> Vec<String> {
    wrap_with(text, width, |t| b.text_width(t, scale))
}

/// `wrap_text` in FONT1.
/// A recording's name as the Ghost list shows it: the file name without its extension.
fn demo_name(p: &std::path::Path) -> String {
    p.file_stem().and_then(|s| s.to_str()).unwrap_or_default().to_string()
}

fn wrap_fnt(b: &Batch, text: &str, width: f32) -> Vec<String> {
    wrap_with(text, width, |t| b.fnt_width(t, 1.0))
}

fn wrap_with(text: &str, width: f32, measure: impl Fn(&str) -> f32) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();
    let mut line = String::new();
    for word in text.split(' ') {
        let next = if line.is_empty() { word.to_string() } else { format!("{line} {word}") };
        if !line.is_empty() && measure(&next) > width {
            lines.push(std::mem::replace(&mut line, word.to_string()));
        } else {
            line = next;
        }
    }
    lines.push(line);
    lines
}

/// Text scale of a row label: 0.85, smaller for one too wide for the column before the values.
fn label_scale(b: &Batch, label: &str) -> f32 {
    (0.85f32).min(0.85 * 152.0 / b.text_width(label, 0.85).max(1.0))
}

/// A choice row; an inactive one (`on` false: it has no effect right now) is dimmed.
fn choice_row_in(b: &mut Batch, label: &str, choices: &[&str], cur: usize, y: f32, sel: bool, on: bool) {
    let label_colour = match (sel, on) {
        (true, _) => WHITE,
        (false, true) => [0.75, 0.75, 0.95, 1.0],
        (false, false) => DIM,
    };
    b.text(label, 130.0, y, label_scale(b, label), label_colour);
    let mut x = 290.0;
    for (k, c) in choices.iter().enumerate() {
        let w = b.text_width(c, 0.85);
        if k == cur {
            b.rect(x - 2.0, y - 1.0, w + 4.0, 12.0, ORANGE);
            b.text(c, x, y, 0.85, [0.1, 0.05, 0.0, 1.0]);
        } else {
            b.text(c, x, y, 0.85, if sel { WHITE } else if on { ORANGE } else { DIM });
        }
        x += w + 12.0;
    }
}

/// A settings control other than a list of choices.
pub(super) enum Control {
    /// on / off
    Switch(bool),
    /// fill 0..1 and the value shown after the bar
    Slider(f32, String),
}

/// What a graphic-options row shows: the choices with the current one, or a switch / slider.
pub(super) enum RowValue {
    Choices(Vec<String>, usize),
    Control(Control),
}

/// A setting row drawn as a switch or a slider (graphic options, Remaster page) [design
/// choice: the original has only text choices].
fn control_row(b: &mut Batch, label: &str, c: Control, y: f32, sel: bool, on: bool) {
    let label_colour = match (sel, on) {
        (true, _) => WHITE,
        (false, true) => [0.75, 0.75, 0.95, 1.0],
        (false, false) => DIM,
    };
    b.text(label, 130.0, y, label_scale(b, label), label_colour);
    let x = 290.0;
    let accent = if on { ORANGE } else { DIM };
    let track = [0.2, 0.2, 0.35, 1.0];
    match c {
        Control::Switch(state) => {
            // a pill: the knob sits right and the track lights up when on
            b.rect(x, y, 26.0, 10.0, if state { accent } else { track });
            b.rect(if state { x + 15.0 } else { x + 1.0 }, y + 1.0, 10.0, 8.0, if sel { WHITE } else { [0.8, 0.8, 0.9, 1.0] });
            b.text(if state { "On" } else { "Off" }, x + 34.0, y, 0.85, if sel { WHITE } else { label_colour });
        }
        Control::Slider(f, text) => {
            const W: f32 = 150.0;
            let f = f.clamp(0.0, 1.0);
            b.rect(x, y + 3.0, W, 4.0, track);
            b.rect(x, y + 3.0, W * f, 4.0, accent);
            b.rect(x + W * f - 2.0, y - 1.0, 5.0, 12.0, if sel { WHITE } else { [0.8, 0.8, 0.9, 1.0] });
            b.text(&text, x + W + 10.0, y, 0.85, if sel { WHITE } else { label_colour });
        }
    }
}

/// Car preview window: the ring sprite `carc` (slot 4 of sprpos set 3 = (264, 26), 326 x 270) is an ellipse centred
/// (427, 161) with radii 163 x 135 and a 7 px rim [O, measured from the sprite]; the window is that outline minus 2 px, so
/// the rim covers its edge and no gap shows between ring and picture.
const CAR_WINDOW: (f32, f32, f32, f32) = (427.0, 161.0, 161.0, 133.0);
/// The size of a car's `_menuimg.TGA` photo.
const CAR_PHOTO: (f32, f32) = (324.0, 270.0);
/// CLASSIC 0x4618eb: the largest |coordinate| of the first track's outline (the others share its factor).
const TRACK_OUTLINE_SIZE: f32 = 170.0;
const TRACK_WINDOW: (f32, f32, f32, f32) = (448.0, 167.0, 156.0, 131.0);

/// A car screen stat dial [O 0x46cc60 init, 0x46d2d6 draw]: the MENU.SPR dial at its `sprpos`
/// set 3 slot, the fill mask centred at the offset below, and the SPEED / ACC / GRIP tag sprite at
/// its own slot.
struct Gauge {
    sprite: &'static str,
    slot: usize,
    label: &'static str,
    label_slot: usize,
    /// mask centre from the dial's slot position (0x481fbc x / y arguments)
    centre: (f32, f32),
    /// mask radius (x; y is 5/6 of it)
    radius: f32,
}

const GAUGES: [Gauge; 3] = [
    Gauge { sprite: "carx", slot: 6, label: "cas", label_slot: 0xe, centre: (49.0, 44.0), radius: 41.5 },
    Gauge { sprite: "cary", slot: 7, label: "caa", label_slot: 0xf, centre: (42.0, 35.0), radius: 35.5 },
    Gauge { sprite: "carz", slot: 8, label: "cag", label_slot: 0x10, centre: (32.0, 30.0), radius: 25.5 },
];

/// The menu stats the dials show, raw (they ease before the mapping, 0x46d2d6).
fn gauge_stats(car: &CarInfo) -> [f32; 3] {
    [car.top_speed, car.accel, car.handling]
}

/// CLASSIC 0x46d309..0x46d4a0: stat -> dial fill, `(v - low) / (high - low)` with `v - low` floored at
/// 0 and no upper limit: TopSpeed 250..320, Acceleration 3.2 (empty) .. 2.2 (full), Handling 3.5..8.5.
fn gauge_fills(stats: [f32; 3]) -> [f32; 3] {
    [
        (stats[0] - 250.0).max(0.0) / 70.0,
        1.0 - (stats[1] - 2.2).max(0.0) / 1.0,
        (stats[2] - 3.5).max(0.0) / 5.0,
    ]
}

/// 0x481fbc: angles are 4096 to the turn, 0 = +x, increasing clockwise on the screen.
const GAUGE_TURN: f32 = 4096.0;
/// The mask's 8 pie segments, 467.5 units each (0x4d7fea), the whole mask `3740` for an empty dial.
const GAUGE_SEG: f32 = 467.5;

impl Gauge {
    /// CLASSIC 0x481fbc: the sprite's ring is printed fully coloured; the original covers the unfilled
    /// part of it with menu-colour-4 pie wedges from the centre. `(1 - v) * 3740` units are covered,
    /// from the ring's end back towards 12 o'clock: whole segments first (segment i starts at
    /// `468 * (7 - i) + 3250` and spans 469 units), then one partial wedge ending at its segment's end.
    fn draw(&self, g: &Game, b: &mut Batch, v: f32) {
        let sp = g.sp();
        let (x, y) = (sp.x(CAR_SET, self.slot) as f32, sp.y(CAR_SET, self.slot) as f32);
        g.slot_sprite(b, (CAR_SET, self.slot), self.sprite, 0);
        let (cx, cy) = (x + self.centre.0, y + self.centre.1);
        let pt = |k: i32| {
            let t = (k & 0xfff) as f32 * std::f32::consts::TAU / GAUGE_TURN;
            [cx + t.cos() * self.radius, cy + t.sin() * self.radius * 0.8333333]
        };
        let mut a = (1.0 - v) * 3740.0;
        for i in 0..8 {
            let base = 468 * (7 - i) + 3250;
            // 0x482032 compares the float's bits as an int, so a negative `a` (a full dial) is partial too
            if a < 468.0 {
                let start = ((base & 0xfff) as f32 + GAUGE_SEG - a).round() as i32 & 0xfff;
                let end = (start as f32 + a).round() as i32;
                b.triangle([[cx, cy], pt(start), pt(end)], LAVENDER);
                break;
            }
            b.triangle([[cx, cy], pt(base), pt(base + 0x11d5)], LAVENDER);
            a -= GAUGE_SEG;
        }
    }
}

fn difficulty_name(d: Difficulty) -> &'static str {
    match d {
        Difficulty::Easy => "Easy",
        Difficulty::Medium => "Medium",
        Difficulty::Hard => "Hard",
        Difficulty::Beginner => "Beginner",
    }
}

/// The next level. REMASTER menu: Easy, Medium, Hard, Beginner (ours, no sprite word for it in
/// the original); CLASSIC: the original three only, Beginner counts as Easy.
fn next_difficulty(d: Difficulty, fwd: bool, modern: bool) -> Difficulty {
    use Difficulty::*;
    let ring: &[Difficulty] = if modern { &[Easy, Medium, Hard, Beginner] } else { &[Easy, Medium, Hard] };
    let at = ring.iter().position(|&x| x == d).unwrap_or(0);
    let n = ring.len();
    ring[if fwd { (at + 1) % n } else { (at + n - 1) % n }]
}

/// Progress::splits from lap 1 (it starts at index 4), lap 1 starting at 0 ms: the 0-based
/// lap*4 + k layout of the records (hiscore.rs).
fn lap_splits(splits: &[Option<u32>]) -> Vec<Option<u32>> {
    let mut v = splits.get(4..).unwrap_or_default().to_vec();
    if let Some(first) = v.first_mut() {
        first.get_or_insert(0);
    }
    v
}

/// "%d:%02d,%02d" (file offset 0xd1360); 0 = no time.
fn fmt_rank_time(ms: u32) -> String {
    if ms == 0 {
        return "-".into();
    }
    fmt_time_cs(ms)
}

/// "%d:%02d,%02d" of a time in ms, a zero too (the loading screen 0x4282b3 prints 0:00,00 without a record).
fn fmt_time_cs(ms: u32) -> String {
    format!("{}:{:02},{:02}", ms / 60000, (ms / 1000) % 60, (ms % 1000) / 10)
}

/// Seconds as h:mm:ss (statistics).
fn fmt_hms(s: f64) -> String {
    let s = s as u64;
    format!("{}:{:02}:{:02}", s / 3600, (s / 60) % 60, s % 60)
}

pub fn fmt_time(ms: u32) -> String {
    format!("{:02}:{:02}:{:02}", ms / 60000, (ms / 1000) % 60, (ms % 1000) / 10)
}

fn compute_results(s: &Session, game_type: GameType) -> Vec<(usize, u32, Option<u32>, Option<u32>)> {
    let order = s.race.standings();
    // CLASSIC 0x429290: single and league games give the cars still racing a made-up race time (#286)
    let times = if matches!(game_type, GameType::Single | GameType::League) {
        s.race.results_times()
    } else {
        order.iter().map(|&slot| s.race.racers[slot].progress.finished_ms).collect()
    };
    order
        .iter()
        .enumerate()
        .map(|(i, &slot)| {
            let p = &s.race.racers[slot].progress;
            (slot, POINTS.get(i).copied().unwrap_or(0), times[i], p.best_lap_ms)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn camera_start_round_trips() {
        for i in 0..CAM_STARTS.len() {
            let o = CamOpts { start: i, ..CamOpts::default() };
            assert_eq!(CamOpts::start_of(o.mode(true)), Some(i));
        }
        assert_eq!(CamOpts::start_of(CamMode::Goal), None);
    }
    use motorview::render::{RenderAa, CLASSIC_MAX_DISTANCE, PostAa};
    use motorview::revmeter::RevMeterTune;

    /// #210: the attract demo draws from the six regular tracks (Divisions 100 .. 301), never the bonus track (310) or
    /// OkkunSpeedway (400), and never the track raced last unless it is the only one.
    #[test]
    fn attract_track_pool() {
        // the shipped install in `Division` order: Goldbridge, RedRock, NeoCity, Atlantika, RuhrStadt, Olympos, NolbyHills, Okkun
        let divs = [100, 101, 200, 201, 300, 301, 310, 400];
        assert_eq!(attract_tracks(&divs, None), vec![0, 1, 2, 3, 4, 5]);
        assert_eq!(attract_tracks(&divs, Some(2)), vec![0, 1, 3, 4, 5]);
        // the last track was a bonus track: nothing to leave out
        assert_eq!(attract_tracks(&divs, Some(6)), vec![0, 1, 2, 3, 4, 5]);
        assert_eq!(attract_tracks(&[100], Some(0)), vec![0]);
        assert!(attract_tracks(&[310, 400], None).is_empty());
    }

    /// The settings' pad defaults and `PadTune::default()` are the same numbers (stick dead zone 5 %, #377).
    #[test]
    fn pad_tune_defaults_agree() {
        let t = PadTune::default();
        let d = PAD_TUNE_DEFAULT.map(|x| x as f32 / 100.0);
        assert_eq!((t.stick_dead, t.trigger_dead, t.steer_sens), (d[0], d[1], d[2]));
        assert_eq!(PAD_TUNE_DEFAULT[0], 5);
    }

    /// A volume steps 5 % at a time and stops at 0 and 100.
    #[test]
    fn volume_steps_five_and_clamps() {
        assert_eq!(step_volume(50, 1), 55);
        assert_eq!(step_volume(50, -1), 45);
        assert_eq!(step_volume(98, 1), 100);
        assert_eq!(step_volume(100, 1), 100);
        assert_eq!(step_volume(3, -1), 0);
        assert_eq!(step_volume(0, -1), 0);
    }

    /// The pause menu's hold: fills in HOLD_S, drains at twice the speed, runs once.
    #[test]
    fn hold_fills_drains_and_fires_once() {
        let dt = 1.0 / 60.0;
        let mut h = Hold { armed: true, ..Hold::default() };
        let mut fired = 0;
        let mut t = 0.0;
        // still down long after it fired: runs exactly once, in the frame the fill reaches 1
        while t < 2.0 {
            if h.step(dt, true) {
                fired += 1;
                assert!((t - HOLD_S).abs() < 2.0 * dt, "fired at {t}");
            }
            t += dt;
        }
        assert_eq!(fired, 1);
        // let go half way: drains in half the time it took to fill
        let mut h = Hold { armed: true, ..Hold::default() };
        for _ in 0..(0.4 / dt) as usize {
            assert!(!h.step(dt, true));
        }
        assert!((h.v - 0.5).abs() < 0.03);
        let mut n = 0;
        while h.v > 0.0 {
            h.step(dt, false);
            n += 1;
        }
        assert!((n as f32 * dt - 0.2).abs() < 2.0 * dt);
        // released: pressing again without arming does nothing
        assert!(!h.step(10.0, true));
        h.clear();
        assert_eq!(h.v, 0.0);
    }

    fn settings() -> Settings {
        Settings {
            car: 3,
            track: 5,
            laps: 7,
            catchup: false,
            difficulty: Difficulty::Hard,
            horn: 4,
            music: false,
            fps_limit: 120,
            menu_stretch: false,
            vol_sfx: 35,
            vol_music: 60,
            vol_ambient: 5,
            engine_sound: true,
            sound_device: Some(motor_audio::OutputDevice { id: "alsa:sysdefault:CARD=PCH".into(), name: "HDA Intel PCH, ALC3246 Analog".into() }),
            view: View {
                brightness: 1.4,
                fog: false,
                show_line: false,
                shadows: false,
                halos: true,
                car_shading: CarShading::Gouraud,
                car_detail: CarDetail::Medium,
                distance: 1.3,
                fov_x: motorview::render::FOV_X.get(),
                sky_image: false,
                motion_blur: false,
                lens_flares: true,
                smoke: true,
                sparks: true,
                mode: RenderMode::Remaster,
                remaster: RemasterView { enhanced_paint: false, dynamic_lights: true, render_aa: RenderAa::msaa(2), post_aa: PostAa { kind: PostAaKind::Off, strength: 50 }, anisotropy: 4, pixel_light: false, pixel_fog: true, fog_style: FogStyle::Classic, fog_tune: Default::default(), clear_coat: false, grade: 30, vignette: 70, grain: 10, hdr: false, tonemap: Tonemap::Agx, exposure: 130, bloom: 20, ssao: 0, smooth_halos: false, smooth_flares: false, retro_signs: false, modern_hud: false, modern_menu: false, car_shadows: false, velocity_blur: false, skids_detailed: false, generated_cars: false, world_fill: true, tree_models: true, water: true, water_ssr: false, local_light: false, texture_pack: true, local_tune: Default::default(), sky_hdri: false, sky_tune: Default::default(), rev_meter: RevMeterTune { glow: 120, inner_glow: 80, glass_alpha: 60, spec: 150, bloom: 0, thickness: 90, depth: 55, roundness: 300 }, car_lights: Default::default(), lamp_lights: Default::default(), light_shadows: LightShadows::Headlights, shadow_quality: ShadowQuality::Low, grass: motorview::render::GrassLevel::Full, grass_range: 130, car_lod_distance: 100, fog_strength: 100 },
                debug: Default::default(),
            },
            display: Display { monitor: 1, fullscreen: true, wide: false, size: (1024, 768), vsync: true },
            manual: true,
            record_race: true,
            ghost_car: true,
            layout: "Joystick".into(),
            binds: {
                let mut o = [None; TASKS.len()];
                (o[0], o[1], o[6], o[8], o[task::GAME_MENU]) = (Some(Code::Key(KeyCode::KeyJ)), Some(Code::Joy(crate::controls::Dir::Right)), Some(Code::JoyButton(4)), Some(Code::MouseButton(2)), Some(Code::Key(KeyCode::KeyP)));
                BTreeMap::from([("joystick".to_string(), o)])
            },
            pad: {
                let mut p = DEFAULT_PAD;
                p[0] = Some(PadIn::Button(gilrs::Button::DPadUp));
                p[1] = Some(PadIn::Stick(crate::controls::Dir::Down));
                p[9] = Some(PadIn::Button(gilrs::Button::Mode));
                p[10] = Some(PadIn::Button(gilrs::Button::West));
                p
            },
            pad_tune: [5, 30, 180, 70],
            player_name: "Max Power".into(),
            team: "DICE".into(),
            player_colour: [1, 2, 3],
            custom_car_colours: true,
            car_colours: [[4, 5, 6], [7, 8, 9], [250, 251, 252]],
            car_finish: [PaintFinish::Metallic, PaintFinish::Gloss, PaintFinish::Pearl],
            short_keys: std::array::from_fn(|i| if i == 3 { "hello  world".into() } else { String::new() }),
            info_detail: INFO_CUSTOM,
            checkpoint_info: 0,
            road_map: 1,
            checkpoint_delay: 7,
            detail_mode: 1,
            mode_dissolve: true,
            track_detail: 0,
            perspective_on: 0,
            perspective: 0,
            skid_marks: false,
            cam: CamOpts { start: 3, mirror: true, free_look: false, pull_in: false, follow: 2 },
            name_plates: 2,
            cd_loop: false,
            menu_confirmation: 2,
            intro: false,
            menu_cd_track: 5,
            track_cd: BTreeMap::from([(3, 7)]),
            hud: HudSettings { cluster: false, clocks: [true, false, true], lap: false, pos: true, map: false, proximity: false, scale: 150, margin: 40, map_mode: MapMode::Close, map_rotate: true, map_numbers: false, plate_frame: false, lap_delay: 7, live_gaps: true, sector_delay: 9 },
        }
    }

    /// car file numbers in the list order of the original data (sorted by Division)
    const IDS: [u32; 10] = [2, 7, 8, 3, 5, 9, 1, 6, 10, 4];
    /// track file numbers in the list order of the original data (sorted by Division)
    const TRACK_IDS: [u32; 8] = [8, 1, 3, 4, 7, 6, 2, 5];

    /// `Track` in settings.cfg is the TrackN.trk number: it follows the track through the Division sort (#307).
    #[test]
    fn settings_track_is_the_file_number() {
        let mut s = Settings::default();
        s.apply("Track 2\n", &IDS, &TRACK_IDS, 30);
        assert_eq!(s.track, 6, "NolbyHills (Track2.trk) is the 7th of the sorted list");
        assert!(s.to_text(&IDS, &TRACK_IDS).contains("Track 2\n"));
        s.apply("Track 99\n", &IDS, &TRACK_IDS, 30);
        assert_eq!(s.track, 6, "an unknown number is ignored");
    }

    /// The Sound device pick is saved with its name, a missing pick (system default) writes nothing, and `default` clears it (#260).
    #[test]
    fn sound_device_setting_round_trips() {
        let mut a = Settings::default();
        assert!(!a.to_text(&IDS, &TRACK_IDS).contains("SoundDevice"));
        a.sound_device = Some(motor_audio::OutputDevice { id: "alsa:hdmi:CARD=HDMI,DEV=0".into(), name: "HDA ATI HDMI, LC27G7xT".into() });
        let mut b = Settings::default();
        b.apply(&a.to_text(&IDS, &TRACK_IDS), &IDS, &TRACK_IDS, 30);
        assert_eq!(b.sound_device, a.sound_device);
        b.apply("SoundDevice default\n", &IDS, &TRACK_IDS, 30);
        assert_eq!(b.sound_device, None);
    }

    /// Left / Right on the Sound device row: no wrap, and a pick that is not connected is left only to the right.
    #[test]
    fn sound_device_choice_steps_and_clamps() {
        assert_eq!(step_device_choice(Some(0), -1, 3), Some(0));
        assert_eq!(step_device_choice(Some(0), 1, 3), Some(1));
        assert_eq!(step_device_choice(Some(3), 1, 3), Some(3));
        assert_eq!(step_device_choice(Some(2), -1, 3), Some(1));
        assert_eq!(step_device_choice(Some(0), 1, 0), Some(0), "no outputs: only the default");
        assert_eq!(step_device_choice(None, -1, 3), None);
        assert_eq!(step_device_choice(None, 1, 3), Some(0));
    }

    #[test]
    fn settings_round_trip() {
        let a = settings();
        let mut b = Settings { layout: "Keyboard".into(), ..Settings::default() };
        b.apply(&a.to_text(&IDS, &TRACK_IDS), &IDS, &TRACK_IDS, 30);
        assert_eq!(b.to_text(&IDS, &TRACK_IDS), a.to_text(&IDS, &TRACK_IDS));
    }

    /// The modern HUD page's Map row is a view over RoadMap and the map toggle (#300 phase 3).
    #[test]
    fn map_row_is_a_view_over_road_map_and_the_toggle() {
        let mut s = Settings::default();
        // (map toggle, RoadMap) of old files -> what the HUD draws: Off, Nearby (Selective) or All cars
        for ((map, road), row) in [((false, 0), 0), ((false, 1), 0), ((false, 2), 0), ((true, 1), 1), ((true, 2), 2), ((true, 0), 2)] {
            (s.hud.map, s.road_map) = (map, road);
            assert_eq!(map_row(&s), row, "map {map} RoadMap {road}");
        }
        // writing the row sets both, and the Classic RoadMap row reads the same
        for i in 0..MAP_LEVELS.len() {
            set_map_row(&mut s, i);
            assert_eq!((map_row(&s), s.road_map, s.hud.map), (i, i, i > 0));
        }
    }

    /// Touching a corner piece copies the shown set into the saved toggles and makes it Custom; the presets
    /// and an untouched Custom leave the saved toggles alone.
    #[test]
    fn touching_a_piece_forks_the_shown_set_into_custom() {
        let mut s = Settings { info_detail: 1, ..Settings::default() };
        s.hud = HudSettings { cluster: false, clocks: [true, false, false], lap: false, pos: true, ..HudSettings::default() };
        let own = s.hud;
        // Selective shows speed, lap time and lap; the saved set is untouched
        assert_eq!(s.hud, own);
        let shown = s.hud.shown(1);
        fork_hud(&mut s);
        assert_eq!((s.info_detail, s.hud), (INFO_CUSTOM, shown));
        // already Custom: nothing more is copied
        s.hud.pos = !s.hud.pos;
        let edited = s.hud;
        fork_hud(&mut s);
        assert_eq!(s.hud, edited);
    }

    /// A settings file with the HUD page's options as the old pause menu wrote them loads and saves unchanged.
    #[test]
    fn hud_page_settings_round_trip() {
        let mut a = settings();
        // Custom pieces, the map toggle off with RoadMap Selective, a checkpoint delay
        a.hud.map = false;
        a.road_map = 1;
        let mut b = Settings::default();
        b.apply(&a.to_text(&IDS, &TRACK_IDS), &IDS, &TRACK_IDS, 30);
        assert_eq!((b.hud, b.road_map, b.info_detail, b.checkpoint_info, b.checkpoint_delay), (a.hud, 1, INFO_CUSTOM, 0, 7));
        assert_eq!(map_row(&b), 0);
        assert_eq!(b.to_text(&IDS, &TRACK_IDS), a.to_text(&IDS, &TRACK_IDS));
    }

    /// "Splits on screen" goes to `DELAY_MAX` seconds in the file; a larger value is cut to it.
    #[test]
    fn checkpoint_delay_range() {
        let mut a = settings();
        a.checkpoint_delay = DELAY_MAX;
        let mut b = Settings::default();
        b.apply(&a.to_text(&IDS, &TRACK_IDS), &IDS, &TRACK_IDS, 30);
        assert_eq!(b.checkpoint_delay, DELAY_MAX);
        a.checkpoint_delay = 99;
        b.apply(&a.to_text(&IDS, &TRACK_IDS), &IDS, &TRACK_IDS, 30);
        assert_eq!(b.checkpoint_delay, DELAY_MAX);
    }

    /// "Lap times on screen" goes to `LAP_DELAY_MAX` seconds, an old file without the key keeps the 4 s default.
    #[test]
    fn lap_delay_range() {
        let mut a = settings();
        a.hud.lap_delay = 99;
        let mut b = Settings::default();
        b.apply(&a.to_text(&IDS, &TRACK_IDS), &IDS, &TRACK_IDS, 30);
        assert_eq!(b.hud.lap_delay, crate::hud_layout::LAP_DELAY_MAX);
        let old: String = a.to_text(&IDS, &TRACK_IDS).lines().filter(|l| !l.starts_with("HudLapDelay")).collect::<Vec<_>>().join("\n");
        let mut c = Settings::default();
        c.apply(&old, &IDS, &TRACK_IDS, 30);
        assert_eq!(c.hud.lap_delay, crate::hud_layout::LAP_DELAY_DEFAULT);
    }

    /// "Live gaps" (#302) round-trips through the file; an old file without the key has it off.
    #[test]
    fn live_gaps_default_off() {
        assert!(!HudSettings::default().live_gaps);
        let a = settings();
        assert!(a.hud.live_gaps);
        let mut b = Settings::default();
        b.apply(&a.to_text(&IDS, &TRACK_IDS), &IDS, &TRACK_IDS, 30);
        assert!(b.hud.live_gaps);
        let old: String = a.to_text(&IDS, &TRACK_IDS).lines().filter(|l| !l.starts_with("HudLiveGaps")).collect::<Vec<_>>().join("\n");
        let mut c = Settings::default();
        c.apply(&old, &IDS, &TRACK_IDS, 30);
        assert!(!c.hud.live_gaps);
    }

    /// "Sector delta" (#409) goes to `LAP_DELAY_MAX` seconds like the lap times; an old file without the key keeps the 4 s default.
    #[test]
    fn sector_delay_range() {
        assert_eq!(HudSettings::default().sector_delay, crate::hud_layout::LAP_DELAY_DEFAULT);
        let mut a = settings();
        a.hud.sector_delay = 99;
        let mut b = Settings::default();
        b.apply(&a.to_text(&IDS, &TRACK_IDS), &IDS, &TRACK_IDS, 30);
        assert_eq!(b.hud.sector_delay, crate::hud_layout::LAP_DELAY_MAX);
        let old: String = a.to_text(&IDS, &TRACK_IDS).lines().filter(|l| !l.starts_with("HudSectorDelay")).collect::<Vec<_>>().join("\n");
        let mut c = Settings::default();
        c.apply(&old, &IDS, &TRACK_IDS, 30);
        assert_eq!(c.hud.sector_delay, crate::hud_layout::LAP_DELAY_DEFAULT);
    }

    #[test]
    fn layout_binds_load_and_keep_the_old_custom_lines() {
        let mut s = Settings::default();
        s.apply("CustomTurnLeft J\nBind Joystick GameMenu P\nBind Mouse Nope Q\n", &IDS, &TRACK_IDS, 30);
        assert_eq!(s.binds["custom"][task::TURN_LEFT], Some(Code::Key(KeyCode::KeyJ)));
        assert_eq!(s.binds["joystick"][task::GAME_MENU], Some(Code::Key(KeyCode::KeyP)));
        assert!(!s.binds.contains_key("mouse"));
        let t = s.to_text(&IDS, &TRACK_IDS);
        assert!(t.contains("Bind custom TurnLeft J\n") && t.contains("Bind joystick GameMenu P\n"));
    }

    #[test]
    fn short_keys_keep_their_spaces() {
        let mut s = Settings::default();
        s.apply("ShortKey3 hello  world\nShortKey9 x\nShortKey10 no\nShortKey4\n", &IDS, &TRACK_IDS, 30);
        assert_eq!((s.short_keys[3].as_str(), s.short_keys[9].as_str(), s.short_keys[4].as_str()), ("hello  world", "x", ""));
        assert!(s.to_text(&IDS, &TRACK_IDS).contains("ShortKey3 hello  world\n"));
        assert!(!s.to_text(&IDS, &TRACK_IDS).contains("ShortKey4"));
    }

    #[test]
    fn colour_strip_as_the_original() {
        // a palette file with 22 black cells at its top (entries 255..234), then two colours
        let mut act = vec![0u8; 768];
        act[3 * 233..3 * 233 + 3].copy_from_slice(&[10, 0, 0]);
        act[3 * 232..3 * 232 + 3].copy_from_slice(&[60, 123, 60]);
        let strip = parse_strip(&act).unwrap();
        assert_eq!((strip[0], strip[22], strip[23]), ([0, 0, 0], [10, 0, 0], [60, 123, 60]));
        assert!(parse_strip(&act[..700]).is_none());
        // the first black wins the match: Right from black stays on a black cell, Left wraps to the last cell
        assert_eq!(strip_index(&strip, [0, 0, 0]), 0);
        assert_eq!(strip_step(&strip, [0, 0, 0], true), [0, 0, 0]);
        assert_eq!(strip_step(&strip, [0, 0, 0], false), strip[31]);
        assert_eq!(strip_step(&strip, [10, 0, 0], false), [0, 0, 0]);
        assert_eq!(strip_step(&strip, [60, 120, 60], true), strip[24]);
        // an exe-table fallback: 15 colours then black
        let exe: [[u8; 3]; STRIP_CELLS] = std::array::from_fn(|i| STRIP_EXE.get(i).copied().unwrap_or([0; 3]));
        assert_eq!((exe[0], exe[14], exe[15], exe[31]), ([0x80; 3], [0xa0, 0x80, 0x50], [0; 3], [0; 3]));
    }

    #[test]
    fn flat_shading_and_perspective_keys() {
        let mut s = Settings::default();
        s.apply("CarShading Flat\nPerspectiveOn All\nPerspective Good\n", &IDS, &TRACK_IDS, 30);
        assert_eq!((s.view.car_shading, s.perspective_on, s.perspective), (CarShading::Flat, 1, 1));
        let mut b = Settings::default();
        b.apply(&s.to_text(&IDS, &TRACK_IDS), &IDS, &TRACK_IDS, 30);
        assert_eq!((b.view.car_shading, b.perspective_on, b.perspective), (CarShading::Flat, 1, 1));
        // the presets: Low is Flat, Max corrects everything
        for (mode, shading, persp) in [(0, CarShading::Flat, 0), (1, CarShading::Reflection, 0), (3, CarShading::Reflection, 1)] {
            b.detail_mode = mode;
            b.apply_detail();
            assert_eq!((b.view.car_shading, b.perspective_on, b.perspective), (shading, persp, persp));
        }
        // Remaster's row has no Flat: it reads as Gouraud
        assert_eq!(remaster_shading(CarShading::Flat, false), 0);
        assert_eq!(remaster_shading(CarShading::Reflection, true), 2);
    }

    #[test]
    fn info_levels_keep_the_custom_hud() {
        let mut s = settings();
        let own = s.hud;
        // Tab / R3 through the levels leaves the saved selection alone
        for _ in 0..INFO_LEVELS.len() {
            Game::cycle(&mut s.info_detail, INFO_LEVELS.len(), true);
        }
        assert_eq!((s.info_detail, s.hud), (INFO_CUSTOM, own));
        // a preset shows its elements but does not save them
        s.info_detail = 1;
        assert_ne!(s.hud.shown(s.info_detail), own);
        let mut b = Settings::default();
        b.apply(&s.to_text(&IDS, &TRACK_IDS), &IDS, &TRACK_IDS, 30);
        assert_eq!((b.info_detail, b.hud), (1, own));
        // an old file without Custom still reads
        b.apply("InfoDetail Selective\n", &IDS, &TRACK_IDS, 30);
        assert_eq!(b.info_detail, 1);
    }

    #[test]
    fn settings_clamp() {
        let mut s = settings();
        let car = s.car;
        s.apply("Car 99\nSFXVolume 400\nViewDistance 5\nLaps 0\nBogus 1\n", &IDS, &TRACK_IDS, 30);
        assert_eq!((s.car, s.vol_sfx, s.laps), (car, 100, 1));
        // `Car` is a file number: car01 is the 7th entry of the sorted list
        s.apply("Car 1\n", &IDS, &TRACK_IDS, 30);
        assert_eq!(s.car, 6);
        assert!((s.view.distance - 0.3).abs() < 1e-6);
        // Remaster keeps 250 % and clamps at its 400 %; Classic draws it at 150 %
        s.apply("ViewDistance 250\n", &IDS, &TRACK_IDS, 30);
        assert!((s.view.distance - 2.5).abs() < 1e-6);
        s.apply("ViewDistance 900\n", &IDS, &TRACK_IDS, 30);
        assert!((s.view.distance - View::MAX_DISTANCE).abs() < 1e-6);
        s.view.mode = RenderMode::Classic;
        assert!((s.view.distance() - CLASSIC_MAX_DISTANCE.get()).abs() < 1e-6);
    }

    #[test]
    fn settings_track_cd() {
        // the song-to-track table's assignments: one line per changed track, clamped, junk ignored
        let mut s = Settings::default();
        s.apply("TrackCDTrack 3 7\nTrackCDTrack 9 1\nTrackCDTrack 4\nTrackCDTrack x 5\n", &IDS, &TRACK_IDS, 30);
        assert_eq!(s.track_cd, BTreeMap::from([(3, 7), (9, 2)]));
        let text = s.to_text(&IDS, &TRACK_IDS);
        assert!(text.contains("TrackCDTrack 3 7\nTrackCDTrack 9 2\n"));
        assert!(!Settings::default().to_text(&IDS, &TRACK_IDS).contains("TrackCDTrack"));
    }

    #[test]
    fn settings_remaster_levels() {
        let mut s = Settings::default();
        s.apply("RemasterMSAA Off\nRemasterAnisotropy 4x\n", &IDS, &TRACK_IDS, 30);
        assert_eq!((s.view.remaster.msaa(), s.view.remaster.anisotropy), (1, 4));
        // the first version saved MSAA as a switch: 1 = on
        s.apply("RemasterMSAA 1\n", &IDS, &TRACK_IDS, 30);
        assert_eq!(s.view.remaster.msaa(), RemasterView::default().msaa());
        s.apply("RemasterMSAA 0\n", &IDS, &TRACK_IDS, 30);
        assert_eq!(s.view.remaster.msaa(), 1);
        // the two-stage options
        // the strength is a stop of the 2x .. 16x scale, written as "4x" or as a percent
        s.apply("RemasterRenderAA MSAA\nRemasterRenderAAStrength 4x\nRemasterPostAA Off\n", &IDS, &TRACK_IDS, 30);
        assert_eq!((s.view.remaster.render_aa.kind, s.view.remaster.msaa()), (RenderAaKind::Msaa, 4));
        s.apply("RemasterRenderAAStrength 100\n", &IDS, &TRACK_IDS, 30);
        assert_eq!(s.view.remaster.msaa(), 16);
        s.apply("RemasterRenderAAStrength 24\n", &IDS, &TRACK_IDS, 30);
        assert_eq!(s.view.remaster.msaa(), 2);
    }

    #[test]
    fn settings_remaster_migration() {
        // Enhanced and DynamicLights moved to the Remaster options
        let mut s = Settings::default();
        s.apply("CarShading Enhanced\nDynamicLights 0\n", &IDS, &TRACK_IDS, 30);
        let v = s.view;
        assert_eq!((v.car_shading, v.mode, v.remaster), (CarShading::Reflection, RenderMode::Classic, RemasterView { dynamic_lights: false, ..RemasterView::default() }));
    }
}
