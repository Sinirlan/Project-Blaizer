//! motorview — viewer and driving test for Motorhead tracks and cars (user-supplied install).
//!
//!   motorview <installed_game> [--track N] [--car NN] [--free]
//!   motorview <installed_game> --shot out.png [--view chase|side|near|hood|quarter|top] [--sim-seconds S] [--size WxH]
//!   motorview <installed_game> --cine FILE.cine --render out.mp4 [--from S --to S] [--fps N] [--size WxH] [--codec h264|hevc|nvenc|png] [--no-audio]
//!   motorview <installed_game> --cine FILE.cine --shot out.png --at-time S   (cine.rs: planned camera sequences, re-notes/20-cine.md)
//!   motorview <installed_game> --cine FILE.cine --cine-edit [--shot out.png [--at-time S] [--cine-preview air|selected] [--cine-sel N] --menu cine --tune cine]   (the Cine editor, cine_edit.rs)
//!
//! Race (default): you start at the back of an 8-car grid against bots.
//! Up/W throttle, Down/S brake/reverse, Left/A Right/D steer, R respawn, Backspace restart race.
//! Free camera (Tab toggles): WASD move, Q/E down/up, Shift fast, Shift+wheel speed, arrows or right-drag look.
//! Free camera grabs the mouse (mouse look); Esc releases it, left click grabs it again.
//! The race is paused in free camera; P runs it.
//! Both: 1-8 track, C next car, L racing line, V camera, B look back, F fog, H shadows, M car shading, G halos, [ ] brightness,
//! F9 Classic/Remaster graphics, K Enhanced car paint, J dynamic lights,
//! U MSAA and O colour grade (Remaster),
//! F3 debug overlay (FPS, camera, crosshair target), F4 wire style (lines, solid, check; Shift+F4 back), Esc exit (asks first).
//! Menu bar along the top (\ hides it): every option by topic, with its key.
//! Left click copies the crosshair target (object position, label, surface) to the clipboard.
//! Debug (see `BINDS`): P pause, . single step, - / = time scale, N focus car, I inspect target,
//! F6 sim panel, F7 car gizmos, F8 track gizmos, F2 collision faces, F12 screenshot.
//! Render debug: F10 shading view (normals, UV checker, texture, lighting, depth), F1 pass
//! panel (then 1-9 hide passes, 0 unlimited view distance; off when the game's settings.cfg is read, see below), Z bounding
//! volumes, X light ranges. Data: T browser (textures, scene objects, cars; PgUp/PgDn,
//! Home/End, Enter jumps to the next object), I pins the target's full data (texture preview).
//! ` (or F11) tuning window (egui): car parameters, camera offsets, render options, debug layers.
//! Ctrl+E world-fill editor (re-notes/12-world-fill.md 12.5): Ctrl / Ctrl+Shift keys, Help > all shortcuts lists them.

use glam::Vec3;
use motor_sim::replay::{Header, Recording};
use motor_sim::{AiRow, Difficulty, Input, Rules, SteerKeys};
use motorview::debug::{self, ColCache, Gizmos, Layers, TrackCache};
use motorview::drive::{self, CamMode, FrameParts, PoseCam, Session};
use motorview::ghost::{self, Follow, Sync as GhostSync, ViewerGhost};
use motorview::inspect::{self, Browser, Catalog, Passes, Tab};
use motorview::pick::{Hit, Picker, Target};
use motorview::wire::WireStyle;
use motorview::render::{CarDetail, CarShading, Camera, DebugShade, DebugView, Frame, Gpu, GrassLevel, RemasterView, RenderAa, RenderMode, View};
use motorview::scene::TEX;
use motorview::timeline::Timeline;
use motorview::ui::{Assets, Rect};
use overlay::{Line, OverlayText};
use motorview::palette;
use motorview::cine_edit::{self, CineEdit, Preview, Req};
use motorview::world_edit::{self, Cmd, Kind, WorldEdit};
use motorview::fill_gizmo;
use motorview::gui::Gui;
use motorview::tune::{self, Page, Targets, Tune};
use ribbon::{RTab, Ribbon, RibbonState};
use motorview::{device, instance, png, scene};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};
use winit::application::ApplicationHandler;
use winit::event::{DeviceEvent, DeviceId, ElementState, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{KeyCode, PhysicalKey};
use winit::window::{CursorGrabMode, Window, WindowId};

mod overlay;
mod ribbon;
mod status;

const USAGE: &str = "usage: motorview [installed_game] [--track N] [--car NN] [--free [--drive-free]] [--difficulty easy|medium|hard|beginner] \
                     [--shot out.png [--view chase|side|near|hood|quarter|top] [--cam chase|far|incar|bumper] [--sim-seconds S [--seek S]] [--replay FILE.mhr] [--size WxH] [--eye X,Y,Z --at X,Y,Z] [--overlay] [--wireframe [lines|solid|check]] [--backdrop R,G,B] [--confirm-exit] [--debug car,all,track,col,bounds,lights,probes] [--debug-view normals|uv|texture|lighting|depth|lampshadow] [--hide world,sky,props,bodies,wheels,sprites,skids,shadows,blades,shafts] [--limited-distance|--unlimited-distance] [--car-sheet sheet.png] [--env-cube cube.png] [--paint R,G,B/R,G,B/R,G,B [--car-finish gloss|metallic|pearl[,F,F]]] [--swap-mesh] [--no-settings] [--cars-dir DIR] [--classic] [--inspect] [--ghost FILE.ghl|.mhr [--ghost-sync lap|loop] [--ghost-window] [--ghost-follow ghost|SLOT] [--ghost-shot out.png [--ghost-cam chase|far|incar|bumper|tv]]] [--tune [car|camera|fog|sky|lamps|carlights|world|ghost|cine|sim]] [--menu [view|sim|track|render|remaster|debug|world|ghost|cine|help]] [--browse textures|props|cars[:N]] [--panel] [--focus N] [--dump] [--car-shading flat|gouraud|reflection] [--car-detail low|medium|high] [--remaster [--no-enhanced-paint] [--no-dynamic-lights] [--no-msaa] [--no-hdr] [--grass off|short|full|ultra] [--grass-range P] [--car-lod-distance P] [--fog-strength P] [--fog-tune key=value,...] [--shadows classic|real] [--skids off|on|detailed] [--water on|off] [--water-reflect sky|scene] [--local-light on|off [--local-tune OCCLUSION,BOUNCE]] [--texture-pack on|off] [--colour-grade] [--brake-preview 0..1]]] \
                     [--cine FILE.cine [--render out.mp4 [--from S] [--to S] [--fps N] [--codec h264|hevc|nvenc|png] [--no-audio] | --shot out.png --at-time S | --cine-edit [--shot out.png [--at-time S] [--cine-preview air|selected] [--cine-sel N]]] [--size WxH] [--classic|--remaster ...]] \
                     [--fill-auto [--seed S] [--force]] [--probes-stats [--track N|all]] [--world-edit [--world-gizmo move|rotate[:N]] [--world-tool KIND]]";

struct Args {
    root: PathBuf,
    /// `--fog-tune key=value,...` over the track's own fog (applied once the track is known)
    fog_tune: Option<String>,
    track: u32,
    car: u32,
    free: bool,
    /// start with hand driving on in the free camera (`Act::DriveFree`)
    drive_free: bool,
    shot: Option<PathBuf>,
    /// the focus car from four sides in one image (2 x 2 tiles of `size`), for a body recipe check
    car_sheet: Option<PathBuf>,
    /// DEBUG the Enhanced paint's env cube after the shot, six faces as a vertical strip (#196)
    env_cube: Option<PathBuf>,
    /// CustomCarColours CarColour1..3 for the followed car (shots); Swap mesh on from the start
    paint: Option<[[u8; 3]; 3]>,
    /// REMASTER --car-finish (#136, #351): the finish of each zone of every repainted car (--paint, Properties > Car > Paint)
    car_finish: [scene::Finish; 3],
    swap_mesh: bool,
    view: String,
    /// REMASTER body recipes read from here instead of the install's `remaster/cars` (#183)
    cars_dir: Option<PathBuf>,
    cam: CamMode,
    sim_seconds: f32,
    /// `--seek S`: after `--sim-seconds`, jump back to timeline second S (#60)
    seek: Option<f32>,
    /// `--replay FILE`: a recorded run (`.mhr`) opened instead of a fresh race (#60 phase 2)
    replay: Option<PathBuf>,
    size: (u32, u32),
    difficulty: Difficulty,
    /// shot camera eye and target, game coordinates
    eye: Option<Vec3>,
    at: Option<Vec3>,
    /// shot: debug overlay and crosshair, wireframe, exit requester
    overlay: bool,
    wireframe: WireStyle,
    /// DEBUG: a solid colour in place of the sky (`--backdrop R,G,B`)
    backdrop: Option<[u8; 3]>,
    confirm_exit: bool,
    /// shot: debug gizmo layers, sim panel, followed car, print the sim panels
    layers: Layers,
    panel: bool,
    focus: Option<usize>,
    dump: bool,
    car_shading: CarShading,
    car_detail: CarDetail,
    mode: RenderMode,
    remaster: RemasterView,
    /// the graphics options the rest of `view()` comes from: the game's settings.cfg, else the defaults below
    base: View,
    /// shot: render debug view, hidden passes, the track's own far plane, pinned target, browser
    shade: DebugShade,
    hide: Passes,
    limited: bool,
    inspect: bool,
    browse: Option<(Tab, usize)>,
    /// shot: the Properties pane open on this page (`--tune [PAGE]`)
    tune: Option<Page>,
    /// shot: the ribbon on this tab (`--menu [TAB]`)
    menu: Option<RTab>,
    /// `--world-edit` (`--fill-edit`): the World Editor open in the shot (forces Remaster, 400 % distance)
    world_edit: bool,
    /// `--fill-gizmo move|rotate[:N]`: the transform gizmo on piece N (default 0) in the shot
    fill_gizmo: Option<(fill_gizmo::Mode, usize)>,
    /// `--world-tool KIND`: a place tool armed in the shot (`ground`, `block`, `plant` ...)
    world_tool: Option<Kind>,
    /// `--track all` (every track: `--fill-auto`, `--probes-stats`)
    all_tracks: bool,
    /// REMASTER: generate `remaster/fill/trackNN.fill` (12-world-fill 12.4) and exit
    fill_auto: bool,
    fill_seed: u32,
    /// `--force`: overwrite an existing .fill
    force: bool,
    /// REMASTER: bake the irradiance probes (probes.rs), print counts and times, exit
    probes_stats: bool,
    /// `--ghost FILE`: a .ghl or .mhr loaded at start (in the shot: drawn in the main view)
    ghost: Option<PathBuf>,
    ghost_sync: GhostSync,
    /// `--ghost-shot out.png`: a second shot from the ghost car's camera (`--ghost-cam`)
    ghost_shot: Option<PathBuf>,
    ghost_cam: CamMode,
    /// `--ghost-window`: open the ghost view window at start
    ghost_window: bool,
    /// `--ghost-follow ghost|N`: what the ghost view follows (a grid slot N)
    ghost_follow: Follow,
    /// `--cine FILE`: a camera sequence (cine.rs); with `--render OUT` it is rendered to a video, with `--shot` + `--at-time` one frame
    cine: Option<PathBuf>,
    render: Option<PathBuf>,
    /// `--from S` / `--to S` / `--fps N` / `--codec C` / `--at-time S` of the cine render
    from: Option<f32>,
    to: Option<f32>,
    fps: Option<u32>,
    codec: motorview::cine_render::Codec,
    /// `--no-audio`: the cine render stays silent (#153)
    no_audio: bool,
    at_time: Option<f32>,
    /// `--size` was given (else the cine file's `Output Size` applies)
    size_set: bool,
    /// `--cine-edit`: open the `--cine` file in the Cine editor (#152) instead of rendering it; `--cine-preview`: the
    /// shot looks through a camera of it
    cine_edit: bool,
    cine_preview: Preview,
    /// `--cine-sel N`: the camera selected in the editor's shot (its gizmo)
    cine_sel: usize,
}

impl Args {
    fn view(&self) -> View {
        View { car_shading: self.car_shading, car_detail: self.car_detail, mode: self.mode, remaster: self.remaster, debug: DebugView { shade: self.shade, unlimited: !self.limited, backdrop: self.backdrop, ..DebugView::default() }, ..self.base }
    }
}

fn parse_args() -> Result<Args, String> {
    let mut it = std::env::args().skip(1).peekable();
    // the game's graphics options (settings.cfg, everything but the window) are the baseline, so what is
    // tuned here looks the same in the game; the flags below override them, `--no-settings` skips the file
    let mut base = View { brightness: 1.0, fog: true, show_line: false, shadows: true, halos: true, car_shading: CarShading::Reflection, car_detail: CarDetail::High, distance: 1.0, fov_x: motorview::render::FOV_X, sky_image: true, motion_blur: false, lens_flares: true, smoke: true, sparks: true, mode: RenderMode::Classic, remaster: RemasterView::default(), debug: DebugView::default() };
    let loaded = if std::env::args().any(|x| x == "--no-settings") { None } else { motorview::view_settings::load(&mut base) };
    if let Some(l) = &loaded {
        println!("settings: {}", l.path.display());
    }
    let mut a = Args {
        root: PathBuf::new(),
        fog_tune: None,
        track: 1,
        car: 1,
        free: false,
        drive_free: false,
        shot: None,
        car_sheet: None,
        env_cube: None,
        paint: None,
        car_finish: [scene::Finish::Gloss; 3],
        swap_mesh: false,
        view: "chase".into(),
        cars_dir: None,
        cam: CamMode::Chase,
        sim_seconds: 0.0,
        seek: None,
        replay: None,
        size: (1280, 720),
        difficulty: Difficulty::Medium,
        eye: None,
        at: None,
        overlay: false,
        wireframe: WireStyle::Off,
        backdrop: None,
        car_shading: base.car_shading,
        car_detail: base.car_detail,
        mode: base.mode,
        remaster: base.remaster,
        base,
        confirm_exit: false,
        layers: Layers::default(),
        panel: false,
        focus: None,
        dump: false,
        all_tracks: false,
        fill_auto: false,
        fill_seed: 1,
        force: false,
        probes_stats: false,
        shade: DebugShade::Off,
        hide: Passes { skids: loaded.as_ref().is_none_or(|l| l.skid_marks), ..Passes::default() },
        // the game clips at its view distance (the fog's clip guard sits there); without settings the viewer is unlimited
        limited: loaded.is_some(),
        inspect: false,
        browse: None,
        tune: None,
        menu: None,
        ghost: None,
        ghost_sync: GhostSync::Lap,
        ghost_shot: None,
        ghost_cam: CamMode::Chase,
        ghost_window: false,
        ghost_follow: Follow::Ghost,
        cine: None,
        render: None,
        from: None,
        to: None,
        fps: None,
        codec: motorview::cine_render::Codec::H264,
        no_audio: false,
        at_time: None,
        cine_edit: false,
        cine_preview: Preview::Off,
        cine_sel: 0,
        size_set: false,
        world_edit: false,
        fill_gizmo: None,
        world_tool: None,
    };
    let mut root = None;
    while let Some(x) = it.next() {
        let mut val = |n: &str| it.next().ok_or(format!("{n} needs a value"));
        match x.as_str() {
            "--track" => match val("--track")?.as_str() {
                "all" => a.all_tracks = true,
                v => a.track = v.parse().map_err(|_| "bad --track")?,
            },
            "--fill-auto" => a.fill_auto = true,
            "--probes-stats" => a.probes_stats = true,
            "--cars-dir" => a.cars_dir = Some(val("--cars-dir")?.into()),
            "--seed" => a.fill_seed = val("--seed")?.parse().map_err(|_| "bad --seed")?,
            "--force" => a.force = true,
            "--car" => a.car = val("--car")?.parse().map_err(|_| "bad --car")?,
            "--free" => a.free = true,
            "--drive-free" => a.drive_free = true,
            "--overlay" => a.overlay = true,
            "--wireframe" => {
                let style = it.next_if(|v| !v.starts_with("--")).map(|v| WireStyle::parse(&v).filter(|w| w.on()).ok_or(format!("unknown wire style {v}"))).transpose()?;
                a.wireframe = style.unwrap_or(WireStyle::Lines);
            }
            "--backdrop" => {
                let v = val("--backdrop")?;
                let c: Vec<u8> = v.split(',').map(|x| x.trim().parse()).collect::<Result<_, _>>().map_err(|_| format!("bad --backdrop {v} (R,G,B 0..255)"))?;
                a.backdrop = Some(<[u8; 3]>::try_from(c).map_err(|_| format!("bad --backdrop {v} (R,G,B 0..255)"))?);
            }
            "--car-shading" => {
                let v = val("--car-shading")?;
                a.car_shading = CarShading::parse(&v).ok_or(format!("unknown car shading {v}"))?;
            }
            "--car-detail" => {
                let v = val("--car-detail")?;
                a.car_detail = CarDetail::parse(&v).ok_or(format!("unknown car detail {v}"))?;
            }
            "--confirm-exit" => a.confirm_exit = true,
            "--debug" => a.layers = Layers::parse(&val("--debug")?)?,
            "--panel" => a.panel = true,
            "--debug-view" => {
                let v = val("--debug-view")?;
                a.shade = DebugShade::parse(&v).ok_or(format!("unknown debug view {v}"))?;
            }
            "--hide" => a.hide = Passes::hide(&val("--hide")?)?,
            "--limited-distance" => a.limited = true,
            "--unlimited-distance" => a.limited = false,
            "--no-settings" => {}
            "--inspect" => a.inspect = true,
            "--tune" => {
                let page = it.next_if(|v| !v.starts_with("--")).map(|v| Page::parse(&v).ok_or(format!("unknown pane page {v}"))).transpose()?;
                a.tune = Some(page.unwrap_or(Page::Car));
            }
            "--menu" => {
                let tab = it.next_if(|v| !v.starts_with("--")).map(|v| RTab::parse(&v).ok_or(format!("unknown ribbon tab {v}"))).transpose()?;
                a.menu = Some(tab.unwrap_or(RTab::View));
            }
            "--world-edit" | "--fill-edit" => a.world_edit = true,
            "--ghost" => a.ghost = Some(val("--ghost")?.into()),
            "--ghost-window" => a.ghost_window = true,
            "--ghost-follow" => {
                a.ghost_follow = match val("--ghost-follow")?.as_str() {
                    "ghost" => Follow::Ghost,
                    v => Follow::Car(v.parse().map_err(|_| "--ghost-follow ghost|SLOT")?),
                }
            }
            "--ghost-shot" => a.ghost_shot = Some(val("--ghost-shot")?.into()),
            "--ghost-cam" => {
                let v = val("--ghost-cam")?;
                a.ghost_cam = CamMode::parse(&v).filter(|m| PoseCam::MODES.contains(m)).ok_or(format!("unknown ghost camera {v}"))?;
            }
            "--ghost-sync" => {
                a.ghost_sync = match val("--ghost-sync")?.as_str() {
                    "lap" => GhostSync::Lap,
                    "loop" => GhostSync::Loop,
                    v => return Err(format!("unknown ghost sync {v}")),
                }
            }
            "--world-tool" => {
                let v = val("--world-tool")?;
                a.world_tool = Some(Kind::ALL.into_iter().find(|k| k.name() == v).ok_or("--world-tool ground|block|wall|hill|patch|skirt|water|plant")?);
                a.world_edit = true;
            }
            "--world-gizmo" | "--fill-gizmo" => {
                let v = val("--world-gizmo")?;
                let (k, n) = v.split_once(':').unwrap_or((&v, "0"));
                let mode = match k {
                    "move" => fill_gizmo::Mode::Move,
                    "rotate" => fill_gizmo::Mode::Rotate,
                    "scale" => fill_gizmo::Mode::Scale,
                    _ => return Err("--world-gizmo move|rotate|scale[:N]".into()),
                };
                a.fill_gizmo = Some((mode, n.parse().map_err(|_| "bad --world-gizmo piece")?));
                a.world_edit = true;
            }
            "--browse" => {
                let v = val("--browse")?;
                let (t, n) = v.split_once(':').unwrap_or((&v, "0"));
                a.browse = Some((Tab::parse(t).ok_or("--browse textures|props|cars[:N]")?, n.parse().map_err(|_| "bad --browse index")?));
            }
            "--dump" => a.dump = true,
            "--focus" => a.focus = Some(val("--focus")?.parse().map_err(|_| "bad --focus")?),
            "--remaster" => a.mode = RenderMode::Remaster,
            "--classic" => a.mode = RenderMode::Classic,
            "--no-enhanced-paint" => a.remaster.enhanced_paint = false,
            "--no-dynamic-lights" => a.remaster.dynamic_lights = false,
            "--shadows" => {
                a.remaster.car_shadows = match val("--shadows")?.to_ascii_lowercase().as_str() {
                    "classic" => false,
                    "real" => true,
                    _ => return Err("bad --shadows (classic|real)".into()),
                }
            }
            "--skids" => match val("--skids")?.to_ascii_lowercase().as_str() {
                "off" => a.hide.skids = false,
                "on" | "classic" => (a.hide.skids, a.remaster.skids_detailed) = (true, false),
                "detailed" => (a.hide.skids, a.remaster.skids_detailed) = (true, true),
                _ => return Err("bad --skids (off|on|detailed)".into()),
            },
            "--no-msaa" => a.remaster.render_aa = RenderAa::msaa(1),
            "--no-hdr" => a.remaster.hdr = false,
            "--grass" => a.remaster.grass = GrassLevel::parse(&val("--grass")?).ok_or("bad --grass (off|short|full|ultra)")?,
            "--grass-range" => {
                let (lo, hi) = RemasterView::GRASS_RANGE;
                a.remaster.grass_range = val("--grass-range")?.parse::<u32>().map_err(|e| e.to_string())?.clamp(lo, hi);
            }
            "--car-lod-distance" => {
                let (lo, hi) = RemasterView::CAR_LOD_DISTANCE;
                a.remaster.car_lod_distance = val("--car-lod-distance")?.parse::<u32>().map_err(|e| e.to_string())?.clamp(lo, hi);
            }
            "--fog-tune" => a.fog_tune = Some(val("--fog-tune")?),
            "--fog-strength" => {
                let (lo, hi) = RemasterView::FOG_STRENGTH;
                a.remaster.fog_strength = val("--fog-strength")?.parse::<u32>().map_err(|e| e.to_string())?.clamp(lo, hi);
            }
            "--water" => a.remaster.water = match val("--water")?.to_ascii_lowercase().as_str() {
                "on" => true,
                "off" => false,
                _ => return Err("bad --water (on|off)".into()),
            },
            "--texture-pack" => a.remaster.texture_pack = match val("--texture-pack")?.to_ascii_lowercase().as_str() {
                "on" => true,
                "off" => false,
                _ => return Err("bad --texture-pack (on|off)".into()),
            },
            "--local-light" => a.remaster.local_light = match val("--local-light")?.to_ascii_lowercase().as_str() {
                "on" => true,
                "off" => false,
                _ => return Err("bad --local-light (on|off)".into()),
            },
            "--local-tune" => {
                let v: Vec<f32> = val("--local-tune")?.split(',').map(|x| x.trim().parse()).collect::<Result<_, _>>().map_err(|_| "bad --local-tune (occlusion,bounce)")?;
                let [occlusion, bounce] = v[..] else { return Err("bad --local-tune (occlusion,bounce)".into()) };
                a.remaster.local_tune = motorview::render::LocalLightTune { occlusion, bounce };
            }
            "--water-reflect" => a.remaster.water_ssr = match val("--water-reflect")?.to_ascii_lowercase().as_str() {
                "sky" => false,
                "scene" => true,
                _ => return Err("bad --water-reflect (sky|scene)".into()),
            },
            "--ssao" => a.remaster.ssao = val("--ssao")?.parse().map_err(|_| "bad --ssao")?,
            "--brake-preview" => a.remaster.car_lights.brake_preview = val("--brake-preview")?.parse().map_err(|_| "bad --brake-preview")?,
            "--lamp-gain" => a.remaster.lamp_lights.gain = val("--lamp-gain")?.parse().map_err(|_| "bad --lamp-gain")?,
            "--colour-grade" => (a.remaster.grade, a.remaster.vignette, a.remaster.grain) = (50, 50, 50),
            "--difficulty" => {
                a.difficulty = match val("--difficulty")?.as_str() {
                    "easy" => Difficulty::Easy,
                    "beginner" => Difficulty::Beginner,
                    "hard" => Difficulty::Hard,
                    _ => Difficulty::Medium,
                }
            }
            "--shot" => a.shot = Some(val("--shot")?.into()),
            "--car-sheet" => a.car_sheet = Some(val("--car-sheet")?.into()),
            "--env-cube" => a.env_cube = Some(val("--env-cube")?.into()),
            "--paint" => {
                let v = val("--paint")?;
                let c: Vec<[u8; 3]> = v
                    .split('/')
                    .map(|c| {
                        let n: Vec<u8> = c.split(',').filter_map(|x| x.trim().parse().ok()).collect();
                        n.try_into().map_err(|_| "--paint R,G,B/R,G,B/R,G,B")
                    })
                    .collect::<Result<_, _>>()?;
                a.paint = Some(c.try_into().map_err(|_| "--paint needs 3 colours")?);
            }
            "--car-finish" => {
                let v = val("--car-finish")?;
                let f: Vec<scene::Finish> = v
                    .split(',')
                    .map(|x| match x.trim() {
                        "gloss" => Ok(scene::Finish::Gloss),
                        "metallic" => Ok(scene::Finish::Metallic),
                        "pearl" => Ok(scene::Finish::Pearl),
                        _ => Err("--car-finish gloss|metallic|pearl[,F,F]"),
                    })
                    .collect::<Result<_, _>>()?;
                a.car_finish = match f[..] {
                    [x] => [x; 3],
                    [x, y, z] => [x, y, z],
                    _ => return Err("--car-finish needs 1 or 3 finishes".into()),
                };
            }
            "--swap-mesh" => a.swap_mesh = true,
            "--view" => a.view = val("--view")?,
            "--cam" => a.cam = CamMode::parse(&val("--cam")?).ok_or("--cam: chase|far|incar|bumper|goal|tv")?,
            "--eye" | "--at" => {
                let v: Vec<f32> = val(&x)?.split(',').filter_map(|c| c.trim().parse().ok()).collect();
                let p = (v.len() == 3).then(|| Vec3::new(v[0], v[1], v[2])).ok_or(format!("{x} X,Y,Z"))?;
                if x == "--eye" { a.eye = Some(p) } else { a.at = Some(p) }
            }
            "--sim-seconds" => a.sim_seconds = val("--sim-seconds")?.parse().map_err(|_| "bad --sim-seconds")?,
            "--replay" => a.replay = Some(val("--replay")?.into()),
            "--seek" => a.seek = Some(val("--seek")?.parse().map_err(|_| "bad --seek")?),
            "--size" => {
                let v = val("--size")?;
                let (w, h) = v.split_once('x').ok_or("--size WxH")?;
                a.size = (w.parse().map_err(|_| "bad width")?, h.parse().map_err(|_| "bad height")?);
                a.size_set = true;
            }
            "--cine" => a.cine = Some(val("--cine")?.into()),
            "--render" => a.render = Some(val("--render")?.into()),
            "--from" => a.from = Some(val("--from")?.parse().map_err(|_| "bad --from")?),
            "--to" => a.to = Some(val("--to")?.parse().map_err(|_| "bad --to")?),
            "--fps" => a.fps = Some(val("--fps")?.parse().map_err(|_| "bad --fps")?),
            "--at-time" => a.at_time = Some(val("--at-time")?.parse().map_err(|_| "bad --at-time")?),
            "--cine-edit" => a.cine_edit = true,
            "--cine-preview" => a.cine_preview = Preview::parse(&val("--cine-preview")?).ok_or("--cine-preview air|selected|off")?,
            "--cine-sel" => a.cine_sel = val("--cine-sel")?.parse().map_err(|_| "--cine-sel N")?,
            "--no-audio" => a.no_audio = true,
            "--codec" => a.codec = motorview::cine_render::Codec::parse(&val("--codec")?).ok_or("--codec h264|hevc|nvenc|png")?,
            "-h" | "--help" => return Err(USAGE.into()),
            _ if root.is_none() => root = Some(PathBuf::from(x)),
            _ => return Err(format!("unexpected argument {x}\n{USAGE}")),
        }
    }
    a.root = root.or_else(motor_data::fs::default_install_root).ok_or_else(|| format!("{}\n{USAGE}", motor_data::fs::NOT_FOUND))?;
    if let Some(t) = &a.fog_tune {
        let (f, warnings) = motorview::fog::from_text(&t.replace(',', "\n").replace('=', " "), motorview::fog::load(&a.root, a.track));
        warnings.iter().for_each(|w| eprintln!("--fog-tune: {w}"));
        a.remaster.fog_tune = Some(f);
    }
    Ok(a)
}

/// Load the render scene and the race session; `player` = drive the last slot yourself, `paint`
/// = CustomCarColours per car slot.
fn load(root: &Path, track: u32, car: u32, player: bool, difficulty: Difficulty, paint: &[(usize, scene::CarPaint)]) -> Option<(scene::Scene, Option<Session>)> {
    let cars = drive::lineup(car);
    let s = match scene::load(root, track, &cars, paint) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("track {track}: {e}");
            return None;
        }
    };
    println!("{}", s.stats);
    let sess = match Session::new(root, track, &cars, player.then_some(cars.len() - 1), difficulty) {
        Ok(mut x) => {
            x.set_wheels(&s);
            // the viewer keeps its own cameras: no start fly-by
            x.fly_by = false;
            x.setup = Some(run_setup(root, track, &cars, &x, difficulty));
            Some(x)
        }
        Err(e) => {
            eprintln!("physics unavailable: {e}");
            None
        }
    };
    Some((s, sess))
}

/// The repro header of a fresh viewer race (#60 phase 2): the setup `Session::new` builds, the fixed
/// line-up of drivers (MHR1 style: no driver list).
fn run_setup(root: &Path, track: u32, cars: &[u32], sess: &Session, difficulty: Difficulty) -> Header {
    let player = sess.race.racers.iter().position(|r| r.human);
    let me = player.map(|p| &sess.race.racers[p]);
    Header {
        track,
        laps: sess.race.laps,
        catchup: sess.race.ai_vector[10] != 0.0,
        rules: Rules::Normal,
        manual: me.is_some_and(|r| r.car.manual),
        row: AiRow::Difficulty(difficulty),
        player,
        cars: cars.to_vec(),
        track_name: motorview::ghost::track_name(root, track).unwrap_or_default(),
        nick: me.map_or_else(String::new, |r| r.driver.nick.clone()),
        car_name: me.map_or_else(String::new, |r| r.car.p.name.clone()),
        drivers: Vec::new(),
        race_ms: None,
        best_lap_ms: None,
    }
}

/// A recorded run (#60 phase 2): its scene and a session in the recorded setup with the timeline of
/// all its frames, at 0:00; `paint` = the drivers' own car colours per slot.
#[allow(clippy::type_complexity)]
fn load_run(root: &Path, rec: &Recording) -> Result<(scene::Scene, Session, Vec<(usize, [[u8; 3]; 3])>), String> {
    let h = &rec.header;
    let mut sess = Session::for_recording(root, rec)?;
    let paint: Vec<_> = sess.race.racers.iter().enumerate().filter(|(_, r)| !r.human).filter_map(|(i, r)| r.driver.car_colours.map(|c| (i, c))).collect();
    let gloss: Vec<_> = paint.iter().map(|&(i, c)| (i, scene::CarPaint::gloss(c))).collect();
    let s = scene::load(root, h.track, &h.cars, &gloss).map_err(|e| format!("track {}: {e}", h.track))?;
    println!("{}", s.stats);
    sess.set_wheels(&s);
    sess.fly_by = false;
    sess.setup = Some(h.clone());
    sess.timeline = Some(Timeline::from_frames(&mut sess.race, &rec.frames));
    Ok((s, sess, paint))
}

/// The car of the recording's player (the last of the line-up without one).
fn run_car(rec: &Recording) -> u32 {
    let h = &rec.header;
    h.player.and_then(|p| h.cars.get(p)).or(h.cars.last()).copied().unwrap_or(1)
}

/// Chase camera behind the car (view space).
fn chase(sess: &Session) -> Camera {
    sess.camera()
}

/// Game coordinates (left-handed, z forward) of a view-space point or direction.
fn game(v: Vec3) -> Vec3 {
    v * Vec3::new(1.0, 1.0, -1.0)
}

/// Frame limit (the viewer presents without vsync).
const FPS_LIMIT: f64 = 90.0;
/// Free-camera speed multiplier range (Shift + mouse wheel, x1.25 per notch).
const SPEED_RANGE: (f32, f32) = (0.05, 20.0);

/// Picking range of the crosshair (m).
const PICK_RANGE: f32 = 2000.0;
/// Sim time multipliers (- / =) and the single-step length (.).
const TIME_SCALES: [f32; 7] = [0.05, 0.1, 0.25, 0.5, 1.0, 2.0, 4.0];
const STEP_DT: f32 = 1.0 / 60.0;
/// Forward runs this many sim steps per frame while held.
const FORWARD_STEPS: usize = 4;
/// Collision faces are drawn within this distance of the camera (m).
const COL_RANGE: f32 = 40.0;
/// Light gizmos (X) within this distance of the camera (m).
const LIGHT_RANGE: f32 = 250.0;
/// Probe gizmos: radius round the camera, metres.
const PROBE_RANGE: f32 = 40.0;
/// the probe debug layer rebakes when the view's sun turns more than this (cosine of 0.5°, as the GPU volume, #198)
const PROBE_SUN_TURN: f32 = 0.999_962;

/// Debug state: gizmo layers, sim panel, time control, lazily built gizmo caches.
struct DebugState {
    layers: Layers,
    /// F6 sim panel: 0 off, 1 followed car, 2 followed car + every racer
    panel: u8,
    paused: bool,
    /// index into `TIME_SCALES`
    scale: usize,
    /// sim steps (`STEP_DT`) still to run while paused
    steps: u32,
    track: Option<TrackCache>,
    col: Option<ColCache>,
    /// REMASTER irradiance probes, baked the first time their gizmos are on
    probes: Option<([f32; 3], motorview::probes::ProbeVolume)>,
    /// I: inspected target, frozen (overlay lines), and its texture layer
    pinned: Option<Vec<String>>,
    pinned_layer: Option<usize>,
    /// F1 render panel open (digits toggle passes)
    render_panel: bool,
    passes: Passes,
    /// T data browser and its per-track lists
    browser: Browser,
    catalog: Option<Catalog>,
}

impl DebugState {
    fn new(layers: Layers, panel: bool) -> DebugState {
        DebugState {
            layers,
            panel: panel as u8,
            paused: false,
            scale: 4,
            steps: 0,
            track: None,
            col: None,
            probes: None,
            pinned: None,
            pinned_layer: None,
            render_panel: false,
            passes: Passes::default(),
            browser: Browser::default(),
            catalog: None,
        }
    }

    /// What the F11 tuning window edits besides the session.
    /// Drop the per-track caches (track switch).
    fn reset(&mut self) {
        (self.track, self.col, self.pinned, self.pinned_layer, self.catalog, self.probes) = (None, None, None, None, None, None);
        (self.browser.sel, self.browser.jumps) = (0, 0);
    }

    fn time_scale(&self) -> f32 {
        TIME_SCALES[self.scale]
    }

    /// This frame's sim step for a wall-clock `dt`, or None when paused.
    fn sim_dt(&mut self, dt: f32) -> Option<f32> {
        if self.steps > 0 {
            self.steps -= 1;
            return Some(STEP_DT);
        }
        (!self.paused).then(|| dt * self.time_scale())
    }

    fn status(&self) -> String {
        let state = if self.paused { "paused (P run, . step)" } else { "running" };
        format!("sim: {state}  x{}", self.time_scale())
    }

    /// F1 render panel lines.
    fn render_lines(&self, view: &View) -> Vec<String> {
        if !self.render_panel {
            return Vec::new();
        }
        let on = |b: bool| if b { "[x]" } else { "[ ]" };
        let mut p = self.passes;
        let mut out = vec!["render (F1 close)".to_string(), format!("F10 shading: {}", view.debug.shade.name())];
        for (i, help) in Passes::HELP.iter().enumerate() {
            out.push(format!("{}  {} {help}", i + 1, on(*p.get(i).unwrap())));
        }
        out.push(format!("0  {} unlimited view distance", on(view.debug.unlimited)));
        out
    }

    /// The gizmos for the current layers.
    #[allow(clippy::too_many_arguments)]
    fn gizmos(&mut self, sess: &Session, view: &View, cam: &Camera, face: Option<usize>, scene: Option<(&scene::Scene, &Picker)>, parts: &FrameParts, hit: Option<Hit>) -> Gizmos {
        let mut g = Gizmos::default();
        let l = self.layers;
        if let (true, Some((_, p))) = (l.bounds, scene) {
            let object = match hit.map(|h| h.target) {
                Some(Target::World { object, .. }) => Some(object),
                _ => None,
            };
            debug::bounds(&mut g, p, game(cam.pos), &parts.cars, &parts.props, object);
        }
        if let (true, Some((s, _))) = (l.lights, scene) {
            debug::lights(&mut g, s, game(cam.pos), &parts.cars, &parts.props, LIGHT_RANGE);
        }
        if let (true, Some((s, _))) = (l.probes, scene) {
            // the sun the GPU volume is baked for (#198); rebaked when it turns (F11 sky turn, HDRI option)
            let sun = motorview::render::view_look(view, s).sun_dir;
            if self.probes.as_ref().is_none_or(|(b, _)| glam::Vec3::from(*b).dot(glam::Vec3::from(sun)) < PROBE_SUN_TURN) {
                let v = motorview::probes::ProbeVolume::bake_scene(s, &sess.race.world, glam::Vec3::from(sun));
                println!("probes: {} bricks, sun {:.2},{:.2},{:.2}, {}", v.bricks.len(), sun[0], sun[1], sun[2], v.stats);
                self.probes = Some((sun, v));
            }
            if let Some((_, vol)) = &self.probes {
                debug::probes(&mut g, vol, s.look.object_ambient, sun, game(cam.pos), PROBE_RANGE);
            }
        }
        if l.all_cars {
            for i in 0..sess.race.racers.len() {
                debug::car(&mut g, sess, i, true);
            }
        } else if l.car {
            debug::car(&mut g, sess, sess.focus, false);
        }
        if l.track {
            let cache = self.track.get_or_insert_with(|| TrackCache::new(&sess.race));
            debug::track(&mut g, &sess.race, cache, game(cam.pos));
        }
        if l.collision {
            let cache = self.col.get_or_insert_with(|| ColCache::new(&sess.race.world));
            debug::collision(&mut g, &sess.race.world, cache, game(cam.pos), COL_RANGE, face);
        }
        g
    }

    /// Sim panel lines (bottom left).
    fn panel(&self, sess: Option<&Session>) -> Vec<String> {
        let Some(s) = sess.filter(|_| self.panel > 0) else { return Vec::new() };
        let mut out = debug::car_panel(s, s.focus);
        if self.panel > 1 {
            out.push(String::new());
            out.extend(debug::racers_panel(s));
        }
        out
    }
}

/// The COL face behind a crosshair hit on the visual world mesh (the collision mesh is
/// separate from the visual one: its surface around the hit).
fn col_hit(sess: &Session, cam: &Camera, hit: Option<Hit>) -> Option<motor_data::col::Hit> {
    let h = hit.filter(|h| matches!(h.target, Target::World { .. }))?;
    let (p, d) = (game(h.point), game(h.point - cam.pos).normalize_or_zero());
    sess.race.world.col.raycast((p - d * 0.5).to_array(), (p + d * 0.5).to_array())
}

/// The overlay lines of the wire style: its name and, in Check, the followed car's mesh counts
/// (each class in its mesh colour).
fn wire_lines(style: WireStyle, gpu: &Gpu, focus: Option<usize>, generated: bool) -> Vec<Line> {
    let mut out = vec![format!("wire style: {} (F4)", style.name()).into()];
    if let (WireStyle::Check, Some(slot)) = (style, focus) {
        if let Some(st) = gpu.wire_stats(slot * scene::CAR_MESHES, generated) {
            out.push(st.spans().into());
        }
    }
    out
}

/// The debug overlay's lines: frame rate, camera, and what the crosshair points at
/// (positions in game coordinates, as `--eye`/`--at` take them).
fn info_lines(fps: Option<f32>, track: u32, cam: &Camera, picker: &Picker, hit: Option<Hit>, sess: Option<&Session>) -> Vec<String> {
    let xyz = |v: Vec3| {
        let g = game(v);
        format!("{:.2}, {:.2}, {:.2}", g.x, g.y, g.z)
    };
    let mut out = vec![format!("{}   track {track}", fps.map_or("FPS -".into(), |f| format!("FPS {f:.0}"))), format!("camera: {}", xyz(cam.pos))];
    // the scene objects' clock: `--sim-seconds` takes the seconds to shoot the same moment
    if let Some(s) = sess {
        let (t, frame) = s.race.props.clock();
        out.push(format!("time: {t:.3} s  prop frame {frame:.1}"));
    }
    let Some(h) = hit else {
        out.push("target: -".into());
        return out;
    };
    out.push(format!("target: {}  ({:.1} m)", xyz(h.point), h.t));
    out.push(format!("object: {}", xyz(h.origin)));
    let mut label = picker.label(h.target);
    if let (Target::Car { slot, .. }, Some(s)) = (h.target, sess) {
        if let Some(r) = s.race.racers.get(slot) {
            label += &format!(", driver {}", r.driver.name);
        }
    }
    out.push(format!("label: {label}"));
    if let Some((c, s)) = sess.and_then(|s| col_hit(s, cam, hit).map(|c| (c, s))) {
        let w = &s.race.world;
        out.push(format!("surface: {}", w.materials.get(w.material_of(c.material)).map_or("?", |m| m.name.as_str())));
        out.push(format!("col face {} normal {:.2}, {:.2}, {:.2}", c.face, c.normal[0], c.normal[1], c.normal[2]));
    }
    out
}

/// Where a key binding applies.
#[derive(Clone, Copy, PartialEq)]
pub(crate) enum Ctx {
    Any,
    /// race camera (driving)
    Race,
    /// free camera
    Free,
    /// world editor open (checked before the others)
    World,
}

#[derive(Clone, Copy, PartialEq)]
enum Act {
    Escape,
    FreeCam,
    Overlay,
    Wireframe,
    WireframeBack,
    Mirror,
    Pause,
    Step,
    Slower,
    Faster,
    Focus,
    Inspect,
    Panel,
    CarGizmos,
    TrackGizmos,
    ColGizmos,
    BoundGizmos,
    LightGizmos,
    ProbeGizmos,
    Shade,
    RenderPanel,
    Browse,
    BrowseUp,
    BrowseDown,
    BrowseFirst,
    BrowseLast,
    Jump,
    Screenshot,
    Cross,
    Tune,
    RacingLine,
    Fog,
    Shadows,
    CarShading,
    Halos,
    Camera,
    Mode,
    Paint,
    Lights,
    Msaa,
    Grade,
    Respawn,
    /// free camera: the arrow keys drive the followed car (#60)
    DriveFree,
    ResetCamera,
    Restart,
    Darker,
    Brighter,
    NextCar,
    /// track = position of the key in `keys` + 1
    Track,
    /// car = index + 1 (menu only)
    Car,
    MenuBar,
    /// ghost view window open / closed, next camera, camera `PoseCam::MODES[i]`, clear the ghost
    GhostWindow,
    GhostCam,
    GhostCamSet,
    /// what the ghost view follows: 0 the ghost, n the car in grid slot n - 1
    GhostFollow,
    GhostClear,
    /// the recorded run saved as a repro file / the `.mhr` of `Replays` number i opened (#60)
    SaveRun,
    OpenRun,
    /// world editor on / off
    WorldToggle,
    /// world editor command
    World(Cmd),
    /// start a new fill piece: `Kind::ALL[i]`
    WorldNew,
    /// REMASTER the cars rebuilt from their body recipes (#183)
    ReloadCars,
    /// the followed car drawn with the other model, original <-> generated
    SwapMesh,
    /// Cine editor (#152): a static camera / a path key at the free camera, the preview's next mode, a shot of the
    /// selected camera at the clock
    CineAddCam,
    CineAddKey,
    CinePreview,
    CineCut,
}

/// One key binding: the dispatch and the menu bar's shortcut texts both come from `BINDS`.
struct Bind {
    keys: &'static [KeyCode],
    name: &'static str,
    ctx: Ctx,
    act: Act,
    /// modifiers that must be held (`CTRL`, `SHIFT`, or neither)
    mods: u8,
}

impl Bind {
    /// What the key does, for the Help menu's full list.
    fn label(&self) -> &'static str {
        match self.act {
            Act::Escape => "release mouse / exit",
            Act::FreeCam => "free camera",
            Act::Overlay => "overlay",
            Act::Cross => "target cross",
            Act::Wireframe => "wire style",
            Act::WireframeBack => "wire style back",
            Act::Mirror => "rear mirror",
            Act::Pause => "pause",
            Act::Step => "single step",
            Act::Slower => "slower",
            Act::Faster => "faster",
            Act::Focus => "follow next car",
            Act::Inspect => "inspect / pin target",
            Act::Panel => "sim panel",
            Act::CarGizmos => "car gizmos",
            Act::TrackGizmos => "track gizmos",
            Act::ColGizmos => "collision faces",
            Act::BoundGizmos => "bounding volumes",
            Act::LightGizmos => "light ranges",
            Act::ProbeGizmos => "irradiance probes",
            Act::Shade => "shading view",
            Act::RenderPanel => "render panel",
            Act::Browse => "data browser",
            Act::BrowseUp => "browser previous",
            Act::BrowseDown => "browser next",
            Act::BrowseFirst => "browser first",
            Act::BrowseLast => "browser last",
            Act::Jump => "jump to object",
            Act::Screenshot => "screenshot",
            Act::Tune => "properties pane",
            Act::RacingLine => "racing line",
            Act::Fog => "fog",
            Act::Shadows => "shadows",
            Act::CarShading => "car shading",
            Act::Halos => "halos",
            Act::Camera => "race camera",
            Act::Mode => "Classic / Remaster",
            Act::Paint => "Enhanced paint",
            Act::Lights => "dynamic lights",
            Act::Msaa => "MSAA",
            Act::Grade => "colour grade",
            Act::Respawn => "respawn",
            Act::DriveFree => "drive the followed car (arrows)",
            Act::ResetCamera => "reset free camera",
            Act::Restart => "restart race",
            Act::Darker => "darker",
            Act::Brighter => "brighter",
            Act::NextCar => "next car",
            Act::Track => "track",
            Act::Car => "car",
            Act::MenuBar => "ribbon",
            Act::GhostWindow => "ghost view window",
            Act::GhostCam => "ghost view: next camera",
            Act::GhostCamSet => "ghost view: camera",
            Act::GhostFollow => "ghost view: follow",
            Act::GhostClear => "clear ghost",
            Act::CineAddCam => "cine: camera here",
            Act::CineAddKey => "cine: path key here",
            Act::CinePreview => "cine: preview off / on air / selected",
            Act::CineCut => "cine: cut to the selected camera",
            Act::SaveRun => "save run",
            Act::OpenRun => "open run",
            Act::WorldToggle => "world editor",
            Act::ReloadCars => "reload car recipes",
            Act::SwapMesh => "swap mesh (original / generated)",
            Act::WorldNew => "new world piece",
            Act::World(c) => match c {
                Cmd::Save => "world: save",
                Cmd::Reload => "world: reload",
                Cmd::Undo => "world: undo",
                Cmd::Redo => "world: redo",
                Cmd::Delete => "world: delete piece",
                Cmd::Duplicate => "world: duplicate piece",
                Cmd::Generate => "world: generate",
                Cmd::Holes => "world: hole finder",
                Cmd::Step(d) if d < 0 => "world: previous piece",
                Cmd::Step(_) => "world: next piece",
                Cmd::Start(_) => "world: new piece",
                Cmd::Finish => "world: finish piece",
                Cmd::Back => "world: remove last point",
                Cmd::Gizmo(fill_gizmo::Mode::Handles) => "world: handles only",
                Cmd::Gizmo(fill_gizmo::Mode::Move) => "world: move gizmo",
                Cmd::Gizmo(fill_gizmo::Mode::Rotate) => "world: rotate gizmo",
                Cmd::Gizmo(fill_gizmo::Mode::Scale) => "world: scale gizmo",
                Cmd::Level => "world: level selected piece",
            },
        }
    }
}

/// Modifier bits of a binding (Alt is never bound: Alt+F4, AltGr layouts).
const CTRL: u8 = 1;
const SHIFT: u8 = 2;

const fn bind(keys: &'static [KeyCode], name: &'static str, ctx: Ctx, act: Act) -> Bind {
    Bind { keys, name, ctx, act, mods: 0 }
}

/// A binding that needs `mods` held; `name` includes them ("Ctrl+Z").
const fn bind_mod(mods: u8, keys: &'static [KeyCode], name: &'static str, ctx: Ctx, act: Act) -> Bind {
    Bind { keys, name, ctx, act, mods }
}

/// The binding for `code` with `mods` held in `ctx`. Exact modifiers win; Shift alone also
/// runs a plain key (Shift is the free camera's "fast", so it must not block other keys).
fn find_bind(code: KeyCode, mods: u8, ctx: Ctx, fill: bool) -> Option<&'static Bind> {
    let of = |c: Ctx, m: u8| BINDS.iter().find(|b| b.mods == m && b.ctx == c && b.keys.contains(&code));
    let exact = |m: u8| fill.then(|| of(Ctx::World, m)).flatten().or_else(|| of(ctx, m)).or_else(|| of(Ctx::Any, m));
    exact(mods).or_else(|| if mods == SHIFT { exact(0) } else { None })
}

use KeyCode as K;
const BINDS: &[Bind] = &[
    bind(&[K::KeyR], "R", Ctx::Race, Act::Respawn),
    bind(&[K::Backspace], "Backspace", Ctx::Race, Act::Restart),
    bind(&[K::KeyV], "V", Ctx::Race, Act::Camera),
    bind(&[K::KeyR], "R", Ctx::Free, Act::ResetCamera),
    bind(&[K::KeyB], "B", Ctx::Free, Act::DriveFree),
    bind(&[K::Tab], "Tab", Ctx::Any, Act::FreeCam),
    bind(&[K::KeyP], "P", Ctx::Any, Act::Pause),
    bind(&[K::Period], ".", Ctx::Any, Act::Step),
    bind(&[K::Minus], "-", Ctx::Any, Act::Slower),
    bind(&[K::Equal], "=", Ctx::Any, Act::Faster),
    bind(&[K::KeyN], "N", Ctx::Any, Act::Focus),
    bind(&[K::KeyI], "I", Ctx::Any, Act::Inspect),
    bind(&[K::F6], "F6", Ctx::Any, Act::Panel),
    bind(&[K::F7], "F7", Ctx::Any, Act::CarGizmos),
    bind(&[K::F8], "F8", Ctx::Any, Act::TrackGizmos),
    bind(&[K::F2], "F2", Ctx::Any, Act::ColGizmos),
    bind(&[K::KeyZ], "Z", Ctx::Any, Act::BoundGizmos),
    bind(&[K::KeyX], "X", Ctx::Any, Act::LightGizmos),
    bind(&[K::F10], "F10", Ctx::Any, Act::Shade),
    bind(&[K::F1], "F1", Ctx::Any, Act::RenderPanel),
    bind(&[K::KeyT], "T", Ctx::Any, Act::Browse),
    bind(&[K::PageUp], "PgUp", Ctx::Any, Act::BrowseUp),
    bind(&[K::PageDown], "PgDn", Ctx::Any, Act::BrowseDown),
    bind(&[K::Home], "Home", Ctx::Any, Act::BrowseFirst),
    bind(&[K::End], "End", Ctx::Any, Act::BrowseLast),
    bind(&[K::Enter, K::NumpadEnter], "Enter", Ctx::Any, Act::Jump),
    bind(&[K::F12], "F12", Ctx::Any, Act::Screenshot),
    bind(&[K::Backquote, K::F11], "` / F11", Ctx::Any, Act::Tune),
    bind(&[K::Digit1, K::Digit2, K::Digit3, K::Digit4, K::Digit5, K::Digit6, K::Digit7, K::Digit8], "1-8", Ctx::Any, Act::Track),
    bind(&[K::KeyC], "C", Ctx::Any, Act::NextCar),
    bind(&[K::KeyL], "L", Ctx::Any, Act::RacingLine),
    bind(&[K::KeyF], "F", Ctx::Any, Act::Fog),
    bind(&[K::KeyH], "H", Ctx::Any, Act::Shadows),
    bind(&[K::KeyM], "M", Ctx::Any, Act::CarShading),
    bind(&[K::KeyG], "G", Ctx::Any, Act::Halos),
    bind(&[K::F9], "F9", Ctx::Any, Act::Mode),
    bind(&[K::KeyK], "K", Ctx::Any, Act::Paint),
    bind(&[K::KeyJ], "J", Ctx::Any, Act::Lights),
    bind(&[K::KeyU], "U", Ctx::Any, Act::Msaa),
    bind(&[K::KeyO], "O", Ctx::Any, Act::Grade),
    bind(&[K::BracketLeft], "[", Ctx::Any, Act::Darker),
    bind(&[K::BracketRight], "]", Ctx::Any, Act::Brighter),
    bind(&[K::F3], "F3", Ctx::Any, Act::Overlay),
    bind(&[K::F4], "F4", Ctx::Any, Act::Wireframe),
    bind_mod(SHIFT, &[K::F4], "Shift+F4", Ctx::Any, Act::WireframeBack),
    bind(&[K::F5], "F5", Ctx::Any, Act::Mirror),
    bind(&[K::Backslash], "\\", Ctx::Any, Act::MenuBar),
    bind(&[K::KeyY], "Y", Ctx::Any, Act::GhostWindow),
    bind_mod(SHIFT, &[K::KeyY], "Shift+Y", Ctx::Any, Act::GhostCam),
    bind(&[K::Escape], "Esc", Ctx::Any, Act::Escape),
    // fill editor (12-world-fill 12.5): Ctrl / Ctrl+Shift keep the single keys free
    bind_mod(CTRL, &[K::KeyE], "Ctrl+E", Ctx::Any, Act::WorldToggle),
    bind_mod(CTRL | SHIFT, &[K::KeyR], "Ctrl+Shift+R", Ctx::Any, Act::ReloadCars),
    bind_mod(CTRL | SHIFT, &[K::KeyG], "Ctrl+Shift+G", Ctx::Any, Act::SwapMesh),
    bind_mod(CTRL, &[K::KeyK], "Ctrl+K", Ctx::Any, Act::CineAddCam),
    bind_mod(CTRL | SHIFT, &[K::KeyK], "Ctrl+Shift+K", Ctx::Any, Act::CineAddKey),
    bind_mod(CTRL, &[K::KeyP], "Ctrl+P", Ctx::Any, Act::CinePreview),
    bind_mod(CTRL, &[K::KeyS], "Ctrl+S", Ctx::World, Act::World(Cmd::Save)),
    bind_mod(CTRL, &[K::KeyR], "Ctrl+R", Ctx::World, Act::World(Cmd::Reload)),
    bind_mod(CTRL, &[K::KeyZ], "Ctrl+Z", Ctx::World, Act::World(Cmd::Undo)),
    bind_mod(CTRL, &[K::KeyY], "Ctrl+Y", Ctx::World, Act::World(Cmd::Redo)),
    bind_mod(CTRL | SHIFT, &[K::KeyZ], "Ctrl+Shift+Z", Ctx::World, Act::World(Cmd::Redo)),
    bind_mod(CTRL, &[K::KeyD], "Ctrl+D", Ctx::World, Act::World(Cmd::Duplicate)),
    bind(&[K::Delete], "Delete", Ctx::World, Act::World(Cmd::Delete)),
    bind_mod(CTRL, &[K::KeyG], "Ctrl+G", Ctx::World, Act::World(Cmd::Generate)),
    bind_mod(CTRL, &[K::KeyH], "Ctrl+H", Ctx::World, Act::World(Cmd::Holes)),
    bind_mod(CTRL, &[K::KeyQ], "Ctrl+Q", Ctx::World, Act::World(Cmd::Gizmo(fill_gizmo::Mode::Handles))),
    bind_mod(CTRL, &[K::KeyW], "Ctrl+W", Ctx::World, Act::World(Cmd::Gizmo(fill_gizmo::Mode::Move))),
    bind_mod(CTRL, &[K::KeyT], "Ctrl+T", Ctx::World, Act::World(Cmd::Gizmo(fill_gizmo::Mode::Rotate))),
    bind_mod(CTRL, &[K::KeyB], "Ctrl+B", Ctx::World, Act::World(Cmd::Gizmo(fill_gizmo::Mode::Scale))),
    bind_mod(CTRL, &[K::Digit1, K::Digit2, K::Digit3, K::Digit4, K::Digit5, K::Digit6], "Ctrl+1-6", Ctx::World, Act::WorldNew),
    bind(&[K::Enter, K::NumpadEnter], "Enter", Ctx::World, Act::World(Cmd::Finish)),
    bind(&[K::Backspace], "Backspace", Ctx::World, Act::World(Cmd::Back)),
    bind_mod(CTRL, &[K::PageUp], "Ctrl+PgUp", Ctx::World, Act::World(Cmd::Step(-1))),
    bind_mod(CTRL, &[K::PageDown], "Ctrl+PgDn", Ctx::World, Act::World(Cmd::Step(1))),
];

/// An action's key in `BINDS` (menu shortcut text).
fn mtime(p: &Path) -> Option<std::time::SystemTime> {
    std::fs::metadata(p).ok().and_then(|m| m.modified().ok())
}

fn key_name(act: Act) -> &'static str {
    BINDS.iter().find(|b| b.act == act).map_or("", |b| b.name)
}

/// Held keys (not in `BINDS`), for the Help menu.
const HELD_FREE: &[(&str, &str)] =
    &[("WASD", "move"), ("Q / E", "down / up"), ("Shift", "fast"), ("Shift+wheel", "speed"), ("mouse, arrows", "look"), (", /", "rewind / forward (held)"), ("left click", "copy target")];
const HELD_RACE: &[(&str, &str)] = &[("Up/W Down/S", "throttle / brake"), ("Left/A Right/D", "steer"), ("B", "look back"), (", /", "rewind / forward (held)"), ("left click", "copy target")];

/// The ghost window's view of the scene: every racer drawn except the one the camera sits in,
/// and the ghost unless the camera is inside it.
fn ghost_scene(parts: &mut FrameParts, sess: &Session, g: Option<&ViewerGhost>, mode: CamMode, follow: Follow) {
    let inside = mode.hides_car();
    parts.cars = sess.models_hiding(match follow {
        Follow::Car(i) if inside => Some(i),
        _ => None,
    });
    parts.hidden.clear();
    parts.ghost = g.filter(|_| !(inside && follow == Follow::Ghost)).map_or_else(Vec::new, |g| g.models(sess));
}

/// The Ghost page of the Properties pane: the saved ghosts and recordings, a path field.
fn ghost_page(ui: &mut egui::Ui, path: &mut String, msg: &str, files: &mut Option<Vec<ghost::Entry>>, cur: Option<&ViewerGhost>, load: &mut Option<PathBuf>) {
    ui.label("A Time Attack lap (.ghl) or a race recording (.mhr), drawn as a ghost car. The session switches to its track and car.");
    ui.horizontal(|ui| {
        ui.text_edit_singleline(path);
        if ui.add_enabled(!path.trim().is_empty(), egui::Button::new("load")).clicked() {
            *load = Some(PathBuf::from(path.trim()));
        }
    });
    if !msg.is_empty() {
        ui.weak(msg);
    }
    if let Some(g) = cur {
        ui.separator();
        ui.label(format!("{}: car {}, track {}, lap {:.2} s, {} poses", g.name, g.car, g.track_no.map_or("?".into(), |t| t.to_string()), g.track.lap_ms as f32 / 1000.0, g.track.poses.len()));
    }
    ui.separator();
    ui.horizontal(|ui| {
        ui.strong("saved");
        if ui.small_button("refresh").clicked() {
            *files = None;
        }
    });
    match files.as_deref() {
        Some([]) => {
            ui.weak("none in the Ghosts and Replays folders of motorhead's settings directory");
        }
        Some(list) => {
            for e in list {
                if ui.button(&e.name).clicked() {
                    *path = e.path.display().to_string();
                    *load = Some(e.path.clone());
                }
            }
        }
        None => {}
    }
}

/// Copy texture `layer` into the atlas preview slot and upload it.
fn upload_preview(gpu: &Gpu, assets: &mut Assets, slot: Rect, s: &scene::Scene, layer: usize) {
    if let Some(px) = s.layers.get(layer) {
        if let Some(r) = assets.atlas.put(slot, "preview", TEX, TEX, px) {
            gpu.update_ui(&assets.atlas, r);
        }
    }
}

fn shot(a: &Args, path: Option<&Path>) -> Result<(), String> {
    // a ghost brings its own track and car along (its meshes draw it)
    let mut ghost = a.ghost.as_ref().map(|p| ViewerGhost::load(&a.root, p)).transpose()?;
    let run = a.replay.as_ref().map(|p| Recording::load(p)).transpose()?;
    // --cine FILE --cine-edit (#152): the sequence's race in the editor, its clock at --at-time
    let cine_file = if a.cine_edit {
        let p = cine_edit::resolve(&a.root, a.cine.as_deref().ok_or("--cine-edit needs --cine FILE")?);
        let (q, warns) = cine_edit::open(&p)?;
        for w in &warns {
            eprintln!("{}: {w}", p.display());
        }
        Some((q, p))
    } else {
        None
    };
    let (track, car) = match (&run, &ghost, &cine_file) {
        (_, _, Some((q, _))) => (q.race.track, q.race.cars.last().copied().unwrap_or(1)),
        (Some(r), _, _) => (r.header.track, run_car(r)),
        (_, Some(g), _) => (g.track_no.unwrap_or(a.track), g.car),
        _ => (a.track, a.car),
    };
    // --paint: the followed car's slot (the last without --focus, `Session::with_field`)
    let paint = a.paint.map(|c| (a.focus.unwrap_or(drive::lineup(car).len() - 1), scene::CarPaint { colours: c, finish: a.car_finish }));
    let (s, mut sess) = match (&run, &cine_file) {
        (_, Some((q, _))) => {
            let (s, ss) = motorview::cine_render::race(&a.root, &q.race, q.fps, a.view().mode == RenderMode::Remaster)?;
            (s, Some(ss))
        }
        (Some(r), _) => {
            let (s, ss, _) = load_run(&a.root, r)?;
            (s, Some(ss))
        }
        (None, None) => load(&a.root, track, car, false, a.difficulty, paint.as_slice()).ok_or("could not load track")?,
    };
    // a recorded run shows `--seek S` (else `--sim-seconds`); a fresh race is stepped that long
    let at = if run.is_some() { a.seek.unwrap_or(a.sim_seconds) } else { a.sim_seconds };
    // the editor: its race second at the clock (the water and wind clock)
    let at = cine_file.as_ref().map_or(at, |(q, _)| motorview::cine::race_secs(q, a.at_time.unwrap_or(0.0)));
    let mut cine = cine_file.map(|(q, p)| {
        let mut ed = CineEdit::default();
        let race = q.race.clone();
        ed.set(q, Some(p));
        ed.restart(race);
        ed.preview = a.cine_preview;
        ed.sel = a.cine_sel.min(ed.seq.as_ref().map_or(0, |q| q.cameras.len().saturating_sub(1)));
        ed
    });
    if let Some(ss) = sess.as_mut() {
        ss.cam = a.cam;
        if let Some(f) = a.focus.filter(|&f| f < ss.race.racers.len()) {
            ss.focus = f;
        }
        ss.fx.remaster = a.view().mode == motorview::render::RenderMode::Remaster;
        let dt = 1.0 / 60.0;
        if let Some(ed) = cine.as_mut() {
            // frame by frame from 0, as the render: the director's cameras have the same history
            ss.record_timeline();
            let step = 1.0 / ed.seq.as_ref().map_or(60, |q| q.fps.max(1)) as f32;
            // the editor's clock stops at the sequence's End
            let to = a.at_time.unwrap_or(0.0).min(ed.seq.as_ref().and_then(|q| q.end).unwrap_or(f32::MAX));
            ed.frame(ss, 0.0);
            while ed.t + step * 0.5 < to {
                ed.frame(ss, step);
            }
            if let Some((_, slot, hide)) = ed.preview_cam() {
                ss.focus = slot;
                ss.cam = if hide { CamMode::InCar } else { CamMode::Chase };
            }
        } else if run.is_some() {
            ss.seek(at as f64);
        } else {
            if a.seek.is_some() {
                ss.record_timeline();
            }
            for _ in 0..(a.sim_seconds / dt) as usize {
                ss.step(dt, Input::default());
            }
            if let Some(to) = a.seek {
                ss.seek(to as f64);
            }
        }
        println!("{}", ss.hud());
        if let Some(g) = ghost.as_mut() {
            g.sync = a.ghost_sync;
            g.clock_ms = at as f64 * 1000.0;
            if g.slot(ss).is_none() {
                eprintln!("ghost: car {} is not on the grid", g.car);
            }
        }
        if a.dump {
            println!("{}", debug::car_panel(ss, ss.focus).join("\n"));
            println!("{}", debug::racers_panel(ss).join("\n"));
            // where the marks are (game coordinates, view z negated): aim --eye/--at here
            let marks = ss.skid_marks(true);
            let spots: Vec<String> = marks.chunks(6).step_by((marks.len() / 6 / 8).max(1)).map(|q| format!("kind {} ({:.0}, {:.1}, {:.0})", q[0].env, q[0].pos[0], q[0].pos[1], -q[0].pos[2])).collect();
            println!("skid marks: {} segments, e.g. {}", marks.len() / 6, spots.join(" "));
        }
    }
    let inst = instance();
    let adapter = pollster::block_on(inst.request_adapter(&Default::default())).map_err(|e| format!("no GPU adapter: {e}"))?;
    let (dev, queue) = pollster::block_on(device(&adapter));
    let mut gpu = Gpu::new(&adapter, dev, queue, wgpu::TextureFormat::Rgba8Unorm);
    // the water and wind at the sim time, not the wall clock: the shot repeats byte for byte
    gpu.pin_clock(Some(at));
    // REMASTER local light: the whole probe bake in the shot (#83)
    gpu.wait_local_light(true);
    gpu.load(&s);
    gpu.set_local_source(sess.as_ref().map(|ss| motorview::probes::BakeSource::new(&s, &ss.race.world)));
    if a.wireframe.on() {
        gpu.load_wireframe(&s);
    }
    gpu.set_car_swap(sess.as_ref().filter(|_| a.swap_mesh).map(|ss| ss.focus));
    let (pos, fwd) = match &sess {
        Some(ss) => ss.pose_view(),
        None => (s.start_pos, s.start_dir),
    };
    let side = fwd.cross(Vec3::Y);
    let cam = match (a.eye, a.at) {
        // game -> view space: negate z
        (Some(e), Some(t)) => {
            let d = ((t - e) * Vec3::new(1.0, 1.0, -1.0)).normalize_or(Vec3::NEG_Z);
            Camera::new(e * Vec3::new(1.0, 1.0, -1.0), d.x.atan2(-d.z), d.y.asin())
        }
        _ => match (a.view.as_str(), &sess) {
        ("side", _) => Camera::new(pos + side * 5.0 + Vec3::Y * 1.0, (-side).x.atan2(side.z), -0.08),
        // close-ups of the followed car's body: broadside, and from the front three-quarter
        ("near", _) => Camera::new(pos + side * 3.4 + Vec3::Y * 0.9, (-side).x.atan2(side.z), -0.12),
        // the hood and nose from in front and above (checking faired panels, #171)
        ("hood", _) => Camera::new(pos + fwd * 2.6 + Vec3::Y * 2.2, (-fwd).x.atan2(fwd.z), -0.8),
        ("quarter", _) => {
            let d = (fwd * 0.75 + side * 0.66).normalize();
            Camera::new(pos + d * 4.2 + Vec3::Y * 1.3, (-d).x.atan2(d.z), -0.2)
        }
        ("top", _) => Camera::new(pos + Vec3::Y * 60.0, fwd.x.atan2(-fwd.z), -1.5),
        (_, Some(ss)) => chase(ss),
        _ => Camera::new(pos - fwd * 9.0 + Vec3::Y * 3.5, fwd.x.atan2(-fwd.z), -0.15),
        },
    };
    // --cine-preview: through the editor's camera
    let preview = cine.as_ref().and_then(CineEdit::preview_cam);
    let cam = preview.map_or(cam, |p| p.0);
    let mut dbg = DebugState::new(a.layers, a.panel);
    dbg.passes = a.hide;
    // --menu / --tune: the menu bar and the F11 window over the shot (laid out before the
    // frame borrows the session)
    let mut tune = a.tune.or(a.world_edit.then_some(Page::World)).map_or_else(Tune::default, Tune::shown);
    tune.loaded(sess.as_ref());
    let mut world = WorldEdit::default();
    if a.world_edit {
        world.begin(&a.root, track);
        world.sel = (!world.file.pieces.is_empty()).then_some(0);
        if let Some((mode, n)) = a.fill_gizmo {
            world.mode = mode;
            world.sel = (!world.file.pieces.is_empty()).then(|| n.min(world.file.pieces.len() - 1));
        }
        world.skirt_base = motorview::procgen::fill::mesh_pieces(&world.file, &s).skirt.map(|(c, min_y)| (c * world_edit::FLIP, min_y));
        let centre = sess.as_ref().map(|ss| TrackCache::new(&ss.race).centre).unwrap_or_default();
        world.run(Cmd::Holes, &s, &centre);
        if let Some(k) = a.world_tool {
            world.run(Cmd::Start(k), &s, &centre);
        }
    }
    let mut base = a.view();
    if a.world_edit {
        (base.mode, base.distance) = (RenderMode::Remaster, View::MAX_DISTANCE);
    }
    let view = dbg.passes.view(&View { show_line: preview.is_none(), ..base });
    gpu.update_local_light(&view);
    let local_status = gpu.local_status();
    let mut parts = dbg.passes.parts(sess.as_ref(), &cam, &view);
    if let (Some(g), Some(ss)) = (&ghost, &sess) {
        parts.ghost = g.models(ss);
    }
    let picker = Picker::new(&s);
    let hit = picker.cast(cam.pos, cam.forward(), PICK_RANGE, &parts.cars, &parts.props);
    let face = sess.as_ref().and_then(|ss| col_hit(ss, &cam, hit)).map(|c| c.face as usize);
    let mut g = sess.as_ref().map(|ss| dbg.gizmos(ss, &view, &cam, face, Some((&s, &picker)), &parts, hit)).unwrap_or_default();
    if a.world_edit {
        world.gizmos(&mut g, cam.pos * world_edit::FLIP);
    }
    if let Some(ed) = cine.as_ref() {
        ed.gizmos(&mut g, cam.pos * world_edit::FLIP, view.fov_x, sess.as_ref().map(|s| &s.race));
    }
    let mut text = OverlayText {
        info: if a.overlay { info_lines(None, track, &cam, &picker, hit, sess.as_ref()).into_iter().map(Line::from).collect() } else { Vec::new() },
        panel: dbg.panel(sess.as_ref()),
        labels: g.labels,
        ..Default::default()
    };
    if a.wireframe.on() {
        let lines = wire_lines(a.wireframe, &gpu, sess.as_ref().map(|s| s.focus), view.generated_cars());
        // the counts also go to stdout, so a run checks the mesh without reading the picture
        if a.wireframe == WireStyle::Check {
            println!("{}", lines.last().map_or(String::new(), Line::text));
        }
        if a.overlay {
            text.info.extend(lines);
        }
    }
    let mut layer = None;
    if let Some((tab, sel)) = a.browse {
        let cat = Catalog::new(&s, sess.as_ref());
        dbg.browser = Browser { tab: Some(tab), sel, jumps: 0 };
        dbg.browser.step(&s, 0);
        text.right.push(dbg.browser.lines(&s, &cat, sess.as_ref()));
        layer = dbg.browser.preview(&cat);
    }
    if let Some(h) = hit.filter(|_| a.inspect) {
        let (mut lines, l) = inspect::target_details(&s, &picker, sess.as_ref(), &h);
        lines.insert(0, format!("pinned: {}", picker.label(h.target)));
        println!("{}", lines.join("\n"));
        text.right.push(lines);
        layer = layer.or(l);
    }
    let mut gui = (a.tune.is_some() || a.menu.is_some() || a.world_edit || a.confirm_exit || !text.is_empty()).then(|| Gui::new(&gpu.device, wgpu::TextureFormat::Rgba8Unorm, None));
    let mut tview = a.view();
    let mut bar_h = 0.0;
    let mut pane_w = 0.0;
    let mut left_w = 0.0;
    let mut status_h = 0.0;
    let mut shot_wire = a.wireframe;
    let painted = gui.as_mut().map(|g| {
        g.run_headless(a.size, |ui| {
            if a.tune.is_some() || a.menu.is_some() || a.world_edit {
            let mut st = RibbonState::default();
            if let Some(tab) = a.menu {
                st.tab = tab;
                let mut m = Ribbon { st: &mut st, tune: &mut tune, view: &mut tview, dbg: &mut dbg, sess: sess.as_mut(), free: false, drive_free: a.drive_free, overlay: a.overlay, cross: true, wireframe: &mut shot_wire, mirror: false, track, car, mesh_swap: a.swap_mesh, world: a.world_edit.then_some(&mut world), ghost: ghost.as_mut(), ghost_win: a.ghost_shot.as_ref().map(|_| a.ghost_cam), ghost_cam: a.ghost_cam, ghost_follow: a.ghost_follow, scrub: &mut 0, seek: &mut None, cine: cine.as_mut(), acts: Vec::new() };
                bar_h = m.bar(ui) * ui.ctx().pixels_per_point();
            }
            let textures = &s.world_textures_sorted;
            if a.world_edit {
                bar_h += palette::options(ui, &mut world, textures) * ui.ctx().pixels_per_point();
            }
            let sb = status::StatusBar { hint: world.hint(a.world_edit, false, false, false), note: &world.note, fps: 0.0, sim: dbg.status(), job: None };
            status_h = sb.show(ui) * ui.ctx().pixels_per_point();
            if a.world_edit {
                left_w = palette::palette(ui, &mut world) * ui.ctx().pixels_per_point();
            }
            let mut ghost_files = Some(Vec::new());
            let mut path = String::new();
            let mut none = None;
            let names = cine_edit::names(sess.as_ref());
            let mut page = |p: Page, ui: &mut egui::Ui| match p {
                Page::World => world.panel_body(ui, textures),
                Page::Ghost => ghost_page(ui, &mut path, "", &mut ghost_files, ghost.as_ref(), &mut none),
                Page::Cine => {
                    if let Some(ed) = cine.as_mut() {
                        cine_edit::page(ui, ed, &a.root, &names);
                    }
                }
                _ => {}
            };
            let focus = sess.as_ref().map(|ss| ss.focus);
            let paint = focus.and_then(|f| s.car_colours.get(f)).map(|&own| (paint.filter(|p| Some(p.0) == focus).map_or(own, |p| p.1.colours), own));
            let w = tune.pane(ui, sess.as_mut(), Targets { view: &mut tview, track_fog: Some((track, s.look.fog)), track_sky: s.hdri.as_ref().map(|h| (track, h.dome())), paint, local_status, probe_gizmos: None }, &mut page);
            pane_w = w * ui.ctx().pixels_per_point();
            }
            overlay::paint(ui.ctx(), a.size, &cam, view.fov_x, &text, a.overlay, (bar_h, pane_w, status_h, left_w));
            if a.confirm_exit {
                overlay::exit_dialog(ui.ctx());
            }
        })
    });
    let mut f = parts.frame();
    f.wire = a.wireframe;
    f.debug_lines = &g.lines;
    f.debug_xray = &g.xray;
    f.debug_xray_tris = &g.xray_tris;
    let px = if painted.is_some() || layer.is_some() {
        let mut assets = Assets::font_only(&a.root);
        if let (Some(l), Some(slot)) = (layer, assets.atlas.reserve("preview", TEX, TEX)) {
            text.preview = s.layers.get(l).and_then(|px| assets.atlas.put(slot, "preview", TEX, TEX, px)).is_some();
        }
        gpu.load_ui(&assets.atlas);
        let ui = text.preview.then(|| overlay::preview(&assets, a.size, &text, status_h));
        gpu.offscreen(a.size.0, a.size.1, |g, enc, target, depth| {
            g.draw(enc, target, depth, a.size, &cam, &view, &f);
            if let Some((canvas, verts)) = &ui {
                g.draw_ui(enc, target, a.size, *canvas, verts, None, None);
            }
            if let (Some(gui), Some(p)) = (gui.as_mut(), painted) {
                gui.paint(&g.device, &g.queue, enc, target, a.size, p);
            }
        })
    } else {
        gpu.capture(a.size.0, a.size.1, &cam, &view, &f)
    };
    if let Some(path) = path {
        png::write(path, a.size.0, a.size.1, &px).map_err(|e| e.to_string())?;
        println!("wrote {}", path.display());
    }
    // DEBUG --env-cube: the Enhanced paint's env cube of that frame (#196)
    if let Some(out) = &a.env_cube {
        if !view.enhanced() || sess.is_none() {
            eprintln!("--env-cube: no env cube is drawn without --remaster (Enhanced paint) and a race");
        }
        let (w, h, px) = gpu.env_cube();
        png::write(out, w, h, &px).map_err(|e| e.to_string())?;
        println!("wrote {} ({w}x{h}, faces +X -X +Y -Y +Z -Z top to bottom)", out.display());
    }
    // --car-sheet: the focus car from four sides in one image, 2 x 2 tiles of `size` (#183):
    // broadside, front three-quarter, the nose and the tail close up. Each tile renders as its
    // own view (fresh TAA and blur histories), so the tiles do not smear into each other.
    if let Some(out) = &a.car_sheet {
        let look = |e: Vec3, t: Vec3| {
            let d = (t - e).normalize_or(Vec3::NEG_Z);
            Camera::new(e, d.x.atan2(-d.z), d.y.asin())
        };
        let centre = pos + Vec3::Y * 0.5;
        let diag = (fwd * 0.75 + side * 0.66).normalize();
        let cams = [
            Camera::new(pos + side * 5.0 + Vec3::Y * 1.0, (-side).x.atan2(side.z), -0.08),
            Camera::new(pos + diag * 4.2 + Vec3::Y * 1.3, (-diag).x.atan2(diag.z), -0.2),
            look(pos + fwd * 3.2 + side * 1.2 + Vec3::Y * 0.9, centre),
            look(pos - fwd * 3.2 - side * 1.2 + Vec3::Y * 0.9, centre),
        ];
        let (w, h) = a.size;
        let row = (w * 4) as usize;
        let mut sheet = vec![0u8; row * 2 * (h * 2) as usize];
        for (i, cam) in cams.iter().enumerate() {
            let sparts = dbg.passes.parts(sess.as_ref(), cam, &view);
            let mut sf = sparts.frame();
            sf.wire = a.wireframe;
            gpu.select_view(2 + i);
            let tile = gpu.capture(w, h, cam, &view, &sf);
            let (ox, oy) = ((i as u32 % 2) * w, (i as u32 / 2) * h);
            for y in 0..h {
                let dst = ((oy + y) as usize * 2 * w as usize + ox as usize) * 4;
                sheet[dst..dst + row].copy_from_slice(&tile[(y as usize) * row..(y as usize + 1) * row]);
            }
        }
        png::write(out, w * 2, h * 2, &sheet).map_err(|e| e.to_string())?;
        println!("wrote {}", out.display());
    }
    // --ghost-shot: the scene from the ghost car, or from `--ghost-follow SLOT` (its own view state,
    // as in the window)
    if let (Some(out), Some(ss)) = (&a.ghost_shot, &sess) {
        let at = match a.ghost_follow {
            Follow::Ghost => ghost.as_ref().and_then(|g| g.pose(ss)).map(|p| (p.pos, p.rot)).ok_or("the ghost has no pose at this time (try --sim-seconds)")?,
            Follow::Car(i) => ss.race.racers.get(i).map(|r| (r.car.pos, r.car.rot)).ok_or("no such grid slot for --ghost-follow")?,
        };
        let gcam = PoseCam::new(a.ghost_cam).camera(&ss.race, &ss.tune, at.0, at.1, 1.0 / 60.0);
        let gview = dbg.passes.view(&a.view());
        let mut gparts = dbg.passes.parts(Some(ss), &gcam, &gview);
        ghost_scene(&mut gparts, ss, ghost.as_ref(), a.ghost_cam, a.ghost_follow);
        let gf = gparts.frame();
        gpu.select_view(1);
        let gpx = gpu.capture(a.size.0, a.size.1, &gcam, &gview, &gf);
        png::write(out, a.size.0, a.size.1, &gpx).map_err(|e| e.to_string())?;
        println!("wrote {}", out.display());
    }
    Ok(())
}

/// DEBUG the ghost view window: a second OS window drawing the scene from the ghost car (view 1
/// of the `Gpu`).
struct GhostWin {
    window: Arc<Window>,
    surface: wgpu::Surface<'static>,
    config: wgpu::SurfaceConfiguration,
    depth: wgpu::TextureView,
    cam: PoseCam,
    follow: Follow,
    last: Instant,
    title_at: Instant,
    /// the camera of the last frame with a pose (held while the ghost is between laps)
    held: Option<Camera>,
}

struct App {
    args: Args,
    track: u32,
    car: u32,
    /// the loaded line-up (a recorded run brings its own)
    cars: Vec<u32>,
    window: Option<Arc<Window>>,
    surface: Option<wgpu::Surface<'static>>,
    config: Option<wgpu::SurfaceConfiguration>,
    gpu: Option<Gpu>,
    depth: Option<wgpu::TextureView>,
    sess: Option<Session>,
    free: bool,
    /// free camera: the arrow keys drive the followed car (`Act::DriveFree`, #60)
    drive_free: bool,
    cam: Camera,
    start: (Vec3, Vec3),
    view: View,
    keys: HashSet<KeyCode>,
    steer_keys: SteerKeys,
    looking: bool,
    /// right-drag look holds the cursor: where it was when the button went down (restored on release)
    look_at: Option<(f32, f32)>,
    last: Instant,
    title_at: Instant,
    /// debug overlay font, crosshair picking
    assets: Option<Assets>,
    picker: Option<Picker>,
    overlay: bool,
    /// the target cross is drawn (View > Show)
    cross: bool,
    /// F4: the wire style (wire.rs)
    wireframe: WireStyle,
    /// F5 rear-view mirror strip (as in the game)
    mirror: bool,
    /// free camera owns the mouse (hidden, locked or confined to the window)
    grabbed: bool,
    confirm_exit: bool,
    /// the exit dialog's Exit button was clicked
    exit_now: bool,
    /// this frame's overlay text and crosshair, painted by the egui pass
    ov: Option<(OverlayText, bool)>,
    /// (frames per second, frames counted, counting since)
    fps: (f32, u32, Instant),
    /// when the next frame is due (FPS_LIMIT)
    next_frame: Instant,
    /// free-camera speed multiplier
    speed: f32,
    dbg: DebugState,
    /// F12: render the next frame to a PNG as well
    screenshot: bool,
    /// rendering an F12 screenshot: no overlay text, crosshair or panels
    clean: bool,
    /// kept alive: on X11 the clipboard contents live in this process
    clipboard: Option<arboard::Clipboard>,
    /// short message under the overlay info (text, since)
    note: Option<(String, Instant)>,
    /// the loaded track (inspector, browser, light gizmos, texture preview)
    scene: Option<scene::Scene>,
    /// atlas slot of the texture preview and the layer in it
    preview_slot: Option<Rect>,
    preview: Option<usize>,
    /// F11 tuning window (egui) and its host
    tune: Tune,
    ribbon: RibbonState,
    inst: Option<wgpu::Instance>,
    adapter: Option<wgpu::Adapter>,
    ghost: Option<ViewerGhost>,
    ghost_win: Option<GhostWin>,
    /// open the window when the event loop is next at hand (`about_to_wait`)
    ghost_open: bool,
    /// the camera a new ghost window starts with
    ghost_cam: CamMode,
    ghost_follow: Follow,
    /// Rewind (-1) / Forward (1) held in the ribbon this frame
    scrub: i8,
    /// timeline slider position asked for this frame (sim seconds)
    seek: Option<f64>,
    /// the Ghost page: path field, last result, listed files, load request (run after the frame)
    ghost_path: String,
    ghost_msg: String,
    ghost_files: Option<Vec<ghost::Entry>>,
    ghost_load: Option<PathBuf>,
    gui: Option<Gui>,
    /// ribbon shown (\), its height and the Properties pane's width in pixels (last frame; 0 when hidden)
    menu_bar: bool,
    bar_h: f32,
    /// the Properties dock (right) and the status bar (bottom), in pixels
    pane_w: f32,
    /// width of the World Editor's tool palette (left edge), pixels
    left_w: f32,
    status_h: f32,
    /// menu items clicked in the egui pass, run after the frame
    pending: Vec<(Act, usize)>,
    /// held modifiers (`CTRL` | `SHIFT`)
    mods: u8,
    /// world-fill editor (Ctrl+E) and the mouse position in window pixels
    world: WorldEdit,
    cursor: (f32, f32),
    last_mesh: Instant,
    /// REMASTER the body recipe file of every car on the grid and when it last changed (#183)
    body_watch: Vec<(PathBuf, Option<std::time::SystemTime>)>,
    body_checked: Instant,
    /// the water bodies as last uploaded (`set_water`)
    last_water: Vec<motorview::procgen::water::Body>,
    /// CustomCarColours per car slot (Properties > Car > Paint), until the line-up changes
    paints: HashMap<usize, [[u8; 3]; 3]>,
    /// the followed car drawn with the other model, original <-> generated (Ctrl+Shift+G)
    mesh_swap: bool,
    /// a paint edit waits for the end of the frame (`repaint` needs the GPU back)
    repaint_due: bool,
    /// REMASTER local light: `Gpu::local_status` of the last frame (the F11 page; the GPU is
    /// taken out of the app while egui runs)
    local_status: &'static str,
    /// the Cine editor (#152): while a sequence is open its clock drives the race
    cine: CineEdit,
}

impl App {
    fn switch(&mut self, track: u32, car: u32) {
        if self.world.open && track != self.track && self.world.unsaved() {
            self.note("fill editor: Ctrl+S saves, Ctrl+R discards, before switching track");
            return;
        }
        if (track, car) != (self.track, self.car) {
            self.paints.clear();
        }
        let paint: Vec<_> = self.paints.iter().map(|(&k, &c)| (k, scene::CarPaint { colours: c, finish: self.args.car_finish })).collect();
        let Some((s, sess)) = load(&self.args.root, track, car, true, self.args.difficulty, &paint) else { return };
        self.install(track, car, drive::lineup(car), s, sess);
    }

    /// Put a loaded scene and session in place (the race restarts, the camera resets).
    fn install(&mut self, track: u32, car: u32, cars: Vec<u32>, s: scene::Scene, sess: Option<Session>) {
        (self.track, self.car, self.cars) = (track, car, cars);
        self.start = (s.start_pos, s.start_dir);
        self.sess = sess;
        if let Some(ss) = self.sess.as_mut().filter(|ss| ss.timeline.is_none()) {
            ss.record_timeline();
        }
        self.dbg.reset();
        self.reset_camera();
        self.picker = Some(Picker::new(&s));
        if let Some(g) = &mut self.gpu {
            g.load(&s);
            g.set_local_source(self.sess.as_ref().map(|ss| motorview::probes::BakeSource::new(&s, &ss.race.world)));
            g.load_wireframe(&s);
        }
        self.dbg.catalog = Some(Catalog::new(&s, self.sess.as_ref()));
        self.tune.loaded(self.sess.as_ref());
        (self.scene, self.preview) = (Some(s), None);
        self.body_watch = self
            .cars
            .iter()
            .map(|&c| {
                let p = motorview::procgen::bodyfile::path(&self.args.root, c);
                let m = mtime(&p);
                (p, m)
            })
            .collect();
        if self.world.open {
            self.world.begin(&self.args.root, track);
            self.world.scene_changed();
        }
    }

    /// Sim > Car, C: `switch` to another car on the same track with the camera kept: the free
    /// camera and the pause as they are, the race camera's mode, tuning and followed slot too.
    /// The race restarts, the new car on the grid.
    fn switch_car(&mut self, car: u32) {
        let keep = (self.cam, self.free, self.dbg.paused);
        let old = self.sess.as_ref().map(|ss| (ss.cam, ss.tune, ss.focus));
        self.switch(self.track, car);
        (self.cam, self.free, self.dbg.paused) = keep;
        if let (Some((cam, tune, focus)), Some(ss)) = (old, self.sess.as_mut()) {
            (ss.cam, ss.tune) = (cam, tune);
            if focus < ss.race.racers.len() {
                ss.focus = focus;
            }
        }
    }

    /// Run the Cine editor's requests (#152) after the frame: a sequence opened or started loads its race, the
    /// rest edit it from the free camera.
    fn cine_apply(&mut self) {
        if let Some(n) = self.cine.poll_job() {
            self.note(n);
        }
        let mut load = false;
        for req in std::mem::take(&mut self.cine.reqs) {
            match req {
                Req::Open(p) => match cine_edit::open(&p) {
                    Ok((seq, warns)) => {
                        for w in &warns {
                            eprintln!("{}: {w}", p.display());
                        }
                        self.cine.msg = if warns.is_empty() { format!("opened {}", p.display()) } else { format!("{} warnings: {}", warns.len(), warns.join("; ")) };
                        self.cine.set(seq, Some(p));
                        load = true;
                    }
                    Err(e) => {
                        self.note(e.clone());
                        self.cine.msg = e;
                    }
                },
                Req::New => {
                    let cam = self.cine.saved.map_or(self.cam, |s| s.2);
                    let p = cine_edit::new_path(&self.args.root, self.track);
                    self.cine.set(cine_edit::new_seq(self.track, &cam), Some(p));
                    self.cine.dirty = true;
                    self.cine.msg = "new sequence: Ctrl+K adds a camera where you stand".into();
                    load = true;
                }
                Req::ApplyRace => load = true,
                Req::Render => self.cine.render(&self.args.root, self.view, self.dbg.passes),
                r => self.cine.edit(r, &self.cam, self.sess.as_ref()),
            }
        }
        if load {
            self.cine_race();
        }
    }

    /// The open sequence's race becomes the viewer's (all AI, the render's start: `cine_render::race`); the clock
    /// goes back to 0, paused, free camera. The camera stays where it is on the same track.
    fn cine_race(&mut self) {
        let Some(seq) = self.cine.seq.as_ref() else { return };
        let (r, fps) = (seq.race.clone(), seq.fps);
        if self.world.open && r.track != self.track && self.world.unsaved() {
            return self.note("fill editor: Ctrl+S saves, Ctrl+R discards, before switching track");
        }
        let remaster = self.view.mode == motorview::render::RenderMode::Remaster;
        match motorview::cine_render::race(&self.args.root, &r, fps, remaster) {
            Ok((s, mut sess)) => {
                sess.setup = Some(run_setup(&self.args.root, r.track, &r.cars, &sess, r.difficulty));
                let keep = (r.track == self.track).then(|| self.cine.saved.map_or(self.cam, |s| s.2));
                self.paints.clear();
                self.install(r.track, r.cars.last().copied().unwrap_or(1), r.cars.clone(), s, Some(sess));
                if let Some(c) = keep {
                    self.cam = c;
                }
                (self.free, self.dbg.paused) = (true, true);
                self.cine.restart(r);
            }
            Err(e) => {
                self.note(format!("cine race: {e}"));
                self.cine.msg = e;
            }
        }
    }

    /// The Cine editor's frame (#152): its clock drives the race (the Sim tab's timeline slider moves the clock), and
    /// the view looks through the previewed camera. True when the free camera flies.
    fn cine_update(&mut self, dt: f32) -> bool {
        if let Some(to) = self.seek.take() {
            let v = self.cine.video_secs(to as f32);
            self.cine.seek(v);
        }
        self.scrub = 0;
        let play = if self.dbg.paused { 0.0 } else { dt * self.dbg.time_scale() };
        let remaster = self.view.mode == motorview::render::RenderMode::Remaster;
        let Some(sess) = self.sess.as_mut() else { return true };
        sess.fx.remaster = remaster;
        self.cine.frame(sess, play);
        match self.cine.preview_cam() {
            Some((cam, slot, hide)) => {
                self.cine.saved.get_or_insert((sess.cam, sess.focus, self.cam));
                // as the render: the shown car is followed, a camera inside it leaves its body out
                sess.focus = slot;
                sess.cam = if hide { CamMode::InCar } else { CamMode::Chase };
                self.cam = cam;
                false
            }
            None => {
                self.cine_unpreview();
                if self.free {
                    return true;
                }
                if let Some(sess) = self.sess.as_ref() {
                    self.cam = chase(sess);
                }
                false
            }
        }
    }

    /// A Cine preview ended: the session's camera mode and followed car and the viewer's camera as they were.
    fn cine_unpreview(&mut self) {
        let Some((mode, focus, cam)) = self.cine.saved.take() else { return };
        if let Some(sess) = self.sess.as_mut() {
            sess.cam = mode;
            sess.focus = focus.min(sess.race.racers.len() - 1);
        }
        self.cam = cam;
    }

    /// Sim > Playback > Open run (#60 phase 2): a recorded `.mhr` becomes the race, its whole run on
    /// the timeline at 0:00. False when it could not be opened (nothing changes).
    fn open_run(&mut self, path: &Path) -> bool {
        let rec = match Recording::load(path) {
            Ok(r) => r,
            Err(e) => {
                self.note(format!("{}: {e}", path.display()));
                return false;
            }
        };
        if self.world.open && rec.header.track != self.track && self.world.unsaved() {
            self.note("fill editor: Ctrl+S saves, Ctrl+R discards, before switching track");
            return false;
        }
        let (s, sess, paint) = match load_run(&self.args.root, &rec) {
            Ok(x) => x,
            Err(e) => {
                self.note(format!("{}: {e}", path.display()));
                return false;
            }
        };
        self.paints = paint.into_iter().collect();
        self.install(rec.header.track, run_car(&rec), rec.header.cars.clone(), s, Some(sess));
        let secs = self.sess.as_ref().and_then(|ss| ss.timeline.as_ref()).map_or(0.0, |t| t.span().2);
        self.note(format!("{}: {} frames, {}", path.file_name().map_or_else(String::new, |n| n.to_string_lossy().into_owned()), rec.frames.len(), ribbon::clock(secs)));
        true
    }

    /// Sim > Playback > Save run: the recorded run as `Replays/<Track>-<time>.mhr`, the game's folder.
    fn save_run(&mut self) {
        let Some(dir) = ghost::replay_dir() else { return self.note("no settings folder to save into") };
        let rec = match self.sess.as_mut().map(Session::recording) {
            Some(Ok(r)) => r,
            Some(Err(e)) => return self.note(format!("run not saved: {e}")),
            None => return self.note("no race loaded"),
        };
        let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
        let name: String = rec.header.track_name.chars().filter(char::is_ascii_alphanumeric).collect();
        let path = dir.join(format!("{name}-{secs}.mhr"));
        match rec.save(&path) {
            Ok(()) => {
                self.note(format!("run saved: {} ({} frames)", path.display(), rec.frames.len()));
                self.ghost_files = None;
            }
            Err(e) => self.note(format!("run not saved: {e}")),
        }
    }

    /// Put the camera in front of a view-space point, `r` its size, looking down at it from
    /// the current heading (free camera, race paused).
    fn look_at(&mut self, c: Vec3, r: f32) {
        let f = self.cam.forward();
        let dir = (Vec3::new(f.x, 0.0, f.z).normalize_or(Vec3::NEG_Z) - Vec3::Y * 0.5).normalize();
        let eye = c - dir * (r * 2.0 + 4.0);
        let d = (c - eye).normalize();
        self.cam = Camera::new(eye, d.x.atan2(-d.z), d.y.asin());
        if !self.free {
            self.free = true;
            self.dbg.paused = true;
        }
    }

    /// Ctrl+E: the fill editor forces Remaster, the longest view distance and the paused free
    /// camera; closing puts the view back (in `fill_update`).
    fn fill_toggle(&mut self) {
        if self.world.open {
            self.world.open = false;
            return;
        }
        if self.scene.is_none() {
            return;
        }
        self.world.prev = Some((self.view.mode, self.view.distance));
        self.view.mode = RenderMode::Remaster;
        self.view.distance = View::MAX_DISTANCE;
        self.view.remaster.world_fill = true;
        self.view.remaster.water = true;
        self.free = true;
        self.dbg.paused = true;
        self.grab(false);
        self.world.begin(&self.args.root, self.track);
    }

    /// The Sky page's 'save for track': write sky.cfg, then fold the F11 values into the loaded
    /// dome so the picture does not change.
    fn sky_update(&mut self) {
        let Some((n, yaw, exposure, sun)) = self.tune.sky_saved.take() else { return };
        match motorview::sky::save(&self.args.root, n, yaw, exposure, sun) {
            Ok(p) => {
                eprintln!("sky saved: {}", p.display());
                self.view.remaster.sky_tune = Default::default();
                if let Some(h) = self.scene.as_mut().and_then(|s| s.hdri.as_mut()) {
                    (h.yaw, h.exposure, h.follow_sun) = (yaw, exposure, sun);
                }
                if let Some(g) = self.gpu.as_mut() {
                    g.set_dome_cfg(yaw, exposure, sun);
                }
            }
            Err(e) => eprintln!("sky not saved: {e}"),
        }
    }

    /// Ctrl+Shift+R or a changed recipe file: the cars rebuilt from their body recipes
    /// (`bodyfile`, #183) by reloading the scene (0.6 s), the camera staying where it is. A
    /// recipe that does not parse is reported in the status bar and nothing changes.
    fn reload_cars(&mut self, manual: bool) {
        for (p, _) in &self.body_watch {
            let Ok(text) = std::fs::read_to_string(p) else { continue };
            if let Err(e) = motorview::procgen::bodyfile::from_text(&text) {
                let name = p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                self.note(format!("{name}: {e}"));
                return;
            }
        }
        self.switch_car(self.car);
        self.note(if manual { "car recipes reloaded" } else { "car recipe changed: cars reloaded" });
    }

    /// Properties > Car > Paint: the render scene reloaded with `paints` (0.6 s); unlike `switch`
    /// the race, the camera and the followed car stay.
    fn repaint(&mut self) {
        let paint: Vec<_> = self.paints.iter().map(|(&k, &c)| (k, scene::CarPaint { colours: c, finish: self.args.car_finish })).collect();
        let s = match scene::load(&self.args.root, self.track, &self.cars, &paint) {
            Ok(s) => s,
            Err(e) => return self.note(format!("paint: {e}")),
        };
        if let Some(ss) = self.sess.as_mut() {
            ss.set_wheels(&s);
        }
        self.picker = Some(Picker::new(&s));
        if let Some(g) = &mut self.gpu {
            g.load(&s);
            g.set_local_source(self.sess.as_ref().map(|ss| motorview::probes::BakeSource::new(&s, &ss.race.world)));
            g.load_wireframe(&s);
        }
        self.dbg.catalog = Some(Catalog::new(&s, self.sess.as_ref()));
        (self.scene, self.preview) = (Some(s), None);
        if self.world.open {
            self.world.scene_changed();
        }
    }

    /// Swap mesh: which model the followed car shows now.
    fn mesh_note(&self) -> String {
        let (Some(s), Some(ss)) = (&self.scene, &self.sess) else { return "no race loaded".into() };
        let name = s.car_names.get(ss.focus).map_or("car", String::as_str);
        let has = s.remaster_cars.get(ss.focus * scene::CAR_MESHES).is_some_and(|m| !m.is_empty());
        let generated = self.view.generated_cars() != self.mesh_swap;
        let swapped = if self.mesh_swap { " (swapped)" } else { "" };
        match (has, generated) {
            (false, _) => format!("{name}: no generated mesh"),
            (true, true) => format!("{name}: generated mesh{swapped}"),
            (true, false) => format!("{name}: original mesh{swapped}"),
        }
    }

    /// Twice a second: a body recipe file that changed (or appeared) reloads the cars.
    fn body_poll(&mut self) {
        if self.body_checked.elapsed().as_millis() < 500 {
            return;
        }
        self.body_checked = Instant::now();
        let mut changed = false;
        for (p, m) in &mut self.body_watch {
            let now = mtime(p);
            if now != *m {
                (*m, changed) = (now, true);
            }
        }
        if changed {
            self.reload_cars(false);
        }
    }

    /// Run the editor's commands, restore the view when it closed, upload the re-meshed fill.
    fn fill_update(&mut self) {
        if !self.world.open {
            if let Some((mode, distance)) = self.world.prev.take() {
                (self.view.mode, self.view.distance) = (mode, distance);
            }
            return;
        }
        let cmds = std::mem::take(&mut self.world.cmds);
        if let Some(s) = &self.scene {
            // the road's centreline keeps the hole finder to the interior
            let centre = if cmds.contains(&Cmd::Holes) { self.sess.as_ref().map(|s| TrackCache::new(&s.race).centre).unwrap_or_default() } else { Vec::new() };
            for c in cmds {
                self.world.run(c, s, &centre);
            }
        }
        // meshing walks every world vertex: while dragging, at most ~7 times a second
        let ready = (self.world.drag.is_none() && !self.world.piece_dragging() && !self.world.painting()) || self.last_mesh.elapsed().as_millis() > 150;
        if let (true, true, Some(g), Some(s)) = (self.world.dirty_mesh, ready, self.gpu.as_mut(), &self.scene) {
            let fill = motorview::procgen::fill::mesh_pieces(&self.world.file, s);
            g.set_fill(&fill.verts);
            // the viewport click picks the fill's pieces; the skirt gizmo needs the skirt's centre
            if let Some(p) = &mut self.picker {
                p.set_fill(&fill.verts, &fill.pieces);
            }
            self.world.skirt_base = fill.skirt.map(|(c, min_y)| (c * world_edit::FLIP, min_y));
            if self.last_water != self.world.file.water {
                g.set_water(s, &motorview::procgen::water::build(&self.world.file.water, s));
                self.last_water = self.world.file.water.clone();
            }
            // the grass follows the fill's ground pieces too, so every fill edit re-meshes it
            g.set_grass(&motorview::procgen::grass::build(&motorview::procgen::grass::track_bodies(&self.world.file, &s.world_textures), s, &fill.verts));
            self.world.dirty_mesh = false;
            self.last_mesh = Instant::now();
        }
    }

    /// The point under the mouse in game space: the world surface (Shift: its nearest triangle
    /// vertex), else the working-height plane.
    fn fill_cast(&self) -> Option<Vec3> {
        let c = self.config.as_ref()?;
        let dir = debug::cursor_ray(&self.cam, (c.width, c.height), self.view.fov_x, self.cursor);
        let pk = self.picker.as_ref()?;
        let world = pk.cast(self.cam.pos, dir, 3000.0, &[], &[]);
        // ground the user added (fill pieces) counts as well as the original world
        let skip = self.world.ground_skip(None);
        let fill = pk.cast_fill(self.cam.pos, dir, 3000.0, &|i| skip.get(i).copied().unwrap_or(true)).map(|(_, t)| t);
        if let Some(t) = fill.filter(|&t| world.is_none_or(|h| t < h.t)) {
            return Some((self.cam.pos + dir * t) * world_edit::FLIP);
        }
        if let Some(h) = world {
            let mut p = h.point;
            if let (true, Target::World { .. }, Some(s)) = (self.mods & SHIFT != 0, h.target, &self.scene) {
                if let Some(t) = s.tris.get(h.tri * 3..h.tri * 3 + 3) {
                    p = t.iter().map(|v| Vec3::from(v.pos)).min_by(|a, b| a.distance(h.point).total_cmp(&b.distance(h.point))).unwrap_or(p);
                }
            }
            return Some(p * world_edit::FLIP);
        }
        let t = (self.world.work_y - self.cam.pos.y) / dir.y;
        (dir.y.abs() > 1e-4 && (0.0..3000.0).contains(&t)).then(|| (self.cam.pos + dir * t) * world_edit::FLIP)
    }

    /// LMB in the editor: add a point while drawing, else grab a handle of the selected piece.
    fn fill_press(&mut self) {
        if self.world.drawing() {
            if let (Some(p), Some(s), Some(pk)) = (self.fill_cast(), &self.scene, &self.picker) {
                let skip = self.world.ground_skip(None);
                self.world.add_point(p, s, &|q| ground_below(pk, q, &skip));
            }
            return;
        }
        if self.fill_gizmo_press() {
            return;
        }
        let Some(c) = &self.config else { return };
        let size = (c.width, c.height);
        let near = |(i, h): (usize, Vec3)| {
            let (x, y) = debug::project(&self.cam, size, self.view.fov_x, h * world_edit::FLIP)?;
            let d = (x - self.cursor.0).hypot(y - self.cursor.1);
            (d <= world_edit::PICK_PX).then_some((i, d))
        };
        if let Some((i, _)) = self.world.sel_handles().into_iter().enumerate().filter_map(near).min_by(|a, b| a.1.total_cmp(&b.1)) {
            self.world.begin_drag(i);
            return;
        }
        // a click on a fill piece selects it (and a drag moves it); on bare world or sky the piece stays
        let (Some(ray), Some(pk)) = (self.fill_ray(), self.picker.as_ref()) else { return };
        let dir = ray.1 * world_edit::FLIP;
        let world_t = pk.cast(self.cam.pos, dir, 3000.0, &[], &[]).map_or(f32::MAX, |h| h.t);
        // pieces that lie on the world (draped ground, patches) win over it within half a metre
        let piece = pk.cast_fill(self.cam.pos, dir, 3000.0, &|i| !self.world.pickable(i)).filter(|&(_, t)| t <= world_t + 0.5);
        if let Some((i, t)) = piece {
            self.world.piece_press(i, (self.cam.pos + dir * t) * world_edit::FLIP, self.cursor);
        } else {
            self.world.clear_vertex();
        }
    }

    /// A held piece follows the mouse (`WorldEdit::piece_drag`); blocks and plants find the ground under
    /// them with a ray straight down the world [APPROX: the first surface below, a roof if the piece is under one].
    fn fill_piece_move(&mut self) {
        let (Some(ray), Some(pk)) = (self.fill_ray(), self.picker.as_ref()) else { return };
        let skip = self.world.ground_skip(self.world.dragged());
        self.world.piece_drag(ray, self.cursor, &|p| ground_below(pk, p, &skip));
    }

    /// The plant brush stroke follows the held mouse, stamping on the ground under the path.
    fn fill_paint_move(&mut self) {
        let (Some(p), Some(pk), Some(s)) = (self.fill_cast(), self.picker.as_ref(), self.scene.as_ref()) else { return };
        self.world.cursor_ground = Some(p);
        let skip = self.world.ground_skip(None);
        self.world.stroke_to(p, s, &|q| ground_below(pk, q, &skip));
    }

    /// The mouse ray in game space: origin, direction.
    fn fill_ray(&self) -> Option<(Vec3, Vec3)> {
        let c = self.config.as_ref()?;
        Some((self.cam.pos * world_edit::FLIP, debug::cursor_ray(&self.cam, (c.width, c.height), self.view.fov_x, self.cursor) * world_edit::FLIP))
    }

    /// Hover highlight of the transform gizmo's axes, or the held axis following the mouse.
    fn fill_gizmo_move(&mut self) {
        let (Some(c), Some(ray)) = (&self.config, self.fill_ray()) else { return };
        if self.world.gizmo_dragging() {
            self.world.gizmo_drag(ray);
            return;
        }
        let (cam, size, fov) = (&self.cam, (c.width, c.height), self.view.fov_x);
        self.world.gizmo_hover(ray.0, &|p| debug::project(cam, size, fov, p * world_edit::FLIP), self.cursor);
    }

    /// LMB on a gizmo axis starts a move or rotation.
    fn fill_gizmo_press(&mut self) -> bool {
        let (Some(c), Some(ray)) = (&self.config, self.fill_ray()) else { return false };
        let (cam, size, fov) = (&self.cam, (c.width, c.height), self.view.fov_x);
        self.world.gizmo_press(ray.0, &|p| debug::project(cam, size, fov, p * world_edit::FLIP), self.cursor, ray)
    }

    /// The Cine editor has the left mouse button (#152): a sequence open, its ribbon tab up, the World editor closed
    /// and the free camera not holding the mouse.
    fn cine_mouse(&self) -> bool {
        self.cine.active() && self.ribbon.tab == RTab::Cine && !self.world.open && !self.grabbed && self.sess.is_some()
    }

    /// The Cine editor's lines are drawn: a sequence open and its tab or page shown.
    fn cine_gizmos(&self) -> bool {
        self.cine.active() && !self.world.open && (self.ribbon.tab == RTab::Cine || self.tune.page == Some(Page::Cine))
    }

    /// LMB in the Cine editor: a gizmo arrow, else pick a camera, key or aim point.
    fn cine_press(&mut self) {
        let (Some(c), Some(ray)) = (&self.config, self.fill_ray()) else { return };
        let (cam, size, fov) = (&self.cam, (c.width, c.height), self.view.fov_x);
        self.cine.press(&|p| debug::project(cam, size, fov, p * world_edit::FLIP), self.cursor, ray, self.sess.as_ref().map(|s| &s.race));
    }

    /// The mouse moved in the Cine editor: the held point follows it, else the arrows light up under it.
    fn cine_move(&mut self) {
        let (Some(c), Some(ray)) = (&self.config, self.fill_ray()) else { return };
        let (cam, size, fov) = (&self.cam, (c.width, c.height), self.view.fov_x);
        self.cine.mouse_move(&|p| debug::project(cam, size, fov, p * world_edit::FLIP), self.cursor, ray, self.sess.as_ref().map(|s| &s.race));
    }

    /// Take or release the mouse for free-camera look.
    fn grab(&mut self, on: bool) {
        let Some(w) = &self.window else { return };
        let ok = if on {
            // Locked is not available on X11 / Windows; Confined still gives raw motion
            w.set_cursor_grab(CursorGrabMode::Locked).or_else(|_| w.set_cursor_grab(CursorGrabMode::Confined)).is_ok()
        } else {
            let _ = w.set_cursor_grab(CursorGrabMode::None);
            false
        };
        w.set_cursor_visible(!ok);
        self.grabbed = ok;
    }

    /// Right-drag look: while the button is down the cursor is hidden and held in the window, so the
    /// drag can't run off it; on release it comes back where the drag began. `grab` (the free camera's
    /// own grab, which keeps the mouse until Esc) is not involved and needs none of this.
    fn look_grab(&mut self, on: bool) {
        self.looking = on;
        let Some(w) = &self.window else { return };
        if on {
            if self.free && !self.grabbed && self.look_at.is_none() {
                // Locked is not available on X11 / Windows; Confined still gives raw motion
                if w.set_cursor_grab(CursorGrabMode::Locked).or_else(|_| w.set_cursor_grab(CursorGrabMode::Confined)).is_ok() {
                    w.set_cursor_visible(false);
                    self.look_at = Some(self.cursor);
                }
            }
        } else if let Some(at) = self.look_at.take() {
            // put the cursor back before letting go (Wayland only honours it while locked)
            let _ = w.set_cursor_position(winit::dpi::PhysicalPosition::new(at.0 as f64, at.1 as f64));
            let _ = w.set_cursor_grab(CursorGrabMode::None);
            w.set_cursor_visible(true);
            self.cursor = at;
        }
    }

    fn reset_camera(&mut self) {
        let fwd = Vec3::new(self.start.1.x, 0.0, self.start.1.z).normalize_or(Vec3::NEG_Z);
        self.cam = Camera::looking(self.start.0 - fwd * 9.0 + Vec3::Y * 3.5, fwd);
        self.cam.pitch = -0.15;
    }

    fn resize(&mut self, w: u32, h: u32) {
        let (Some(s), Some(c), Some(g)) = (&self.surface, &mut self.config, &self.gpu) else { return };
        c.width = w.max(1);
        c.height = h.max(1);
        s.configure(&g.device, c);
        self.depth = Some(g.depth_view(c.width, c.height));
    }

    fn key(&self, codes: &[KeyCode]) -> bool {
        codes.iter().any(|c| self.keys.contains(c))
    }

    fn note(&mut self, msg: impl Into<String>) {
        self.note = Some((msg.into(), Instant::now()));
    }

    /// What the crosshair points at.
    fn target(&self) -> Option<Hit> {
        let p = self.picker.as_ref()?;
        let cars = self.sess.as_ref().map(Session::models).unwrap_or_default();
        let props = self.sess.as_ref().map(Session::prop_models).unwrap_or_default();
        p.cast(self.cam.pos, self.cam.forward(), PICK_RANGE, &cars, &props)
    }

    /// The crosshair target's overlay lines from "object:" on, or None.
    fn target_lines(&self) -> Option<Vec<String>> {
        let p = self.picker.as_ref()?;
        let lines = info_lines(None, self.track, &self.cam, p, self.target(), self.sess.as_ref());
        let i = lines.iter().position(|l| l.starts_with("object:"))?;
        Some(lines[i..].to_vec())
    }

    /// Copy what the crosshair points at (object position, label, surface) to the clipboard.
    fn copy_target(&mut self) {
        let Some(lines) = self.target_lines() else {
            self.note("nothing targeted");
            return;
        };
        let text = lines.join("\n");
        println!("{text}");
        if self.clipboard.is_none() {
            self.clipboard = arboard::Clipboard::new().map_err(|e| eprintln!("clipboard: {e}")).ok();
        }
        let ok = self.clipboard.as_mut().is_some_and(|c| c.set_text(text).map_err(|e| eprintln!("clipboard: {e}")).is_ok());
        self.note(if ok { "copied to clipboard" } else { "clipboard unavailable (printed to stdout)" });
    }

    /// I: a targeted car becomes the followed car (with the sim panel on); anything else is
    /// pinned to the overlay. With nothing targeted, unpin.
    fn inspect(&mut self) {
        if let (Some(Hit { target: Target::Car { slot, .. }, .. }), Some(s)) = (self.target(), self.sess.as_mut()) {
            s.focus = slot;
            self.dbg.panel = self.dbg.panel.max(1);
            self.note(format!("following car #{slot}"));
            return;
        }
        let hit = self.target();
        let details = match (hit, &self.picker, &self.scene) {
            (Some(h), Some(p), Some(s)) => Some(inspect::target_details(s, p, self.sess.as_ref(), &h)),
            _ => None,
        };
        self.dbg.pinned_layer = details.as_ref().and_then(|d| d.1);
        self.dbg.pinned = self.target_lines().map(|l| [vec!["pinned (I: again / unpin):".to_string()], l, details.map(|d| d.0).unwrap_or_default()].concat());
        if self.dbg.pinned.is_none() {
            self.note("unpinned");
        }
    }

    /// Load a .ghl or .mhr into the viewer. The ghost's track and car must be loaded to draw
    /// it (its own meshes): the session switches to them when they are not.
    fn load_ghost(&mut self, path: &Path) {
        match ViewerGhost::load(&self.args.root, path) {
            Ok(mut g) => {
                g.sync = self.args.ghost_sync;
                let track = g.track_no.unwrap_or(self.track);
                if track != self.track || !self.cars.contains(&g.car) {
                    self.switch(track, g.car);
                }
                self.ghost_msg = format!("loaded {} ({:.2} s, car {}{})", g.name, g.track.lap_ms as f32 / 1000.0, g.car, g.track_no.map_or(", track unknown".into(), |t| format!(", track {t}")));
                self.ghost = Some(g);
            }
            Err(e) => self.ghost_msg = format!("{}: {e}", path.display()),
        }
        eprintln!("ghost: {}", self.ghost_msg);
    }

    /// The ghost view window: a second surface on the same device and format.
    fn open_ghost_window(&mut self, el: &ActiveEventLoop) {
        let (Some(inst), Some(adapter), Some(gpu), Some(main)) = (&self.inst, &self.adapter, &self.gpu, &self.config) else { return };
        let window = match el.create_window(Window::default_attributes().with_title("motorview — ghost").with_inner_size(winit::dpi::LogicalSize::new(960, 540))) {
            Ok(w) => Arc::new(w),
            Err(e) => return eprintln!("ghost window: {e}"),
        };
        let Ok(surface) = inst.create_surface(window.clone()) else { return eprintln!("ghost window: no surface") };
        let size = window.inner_size();
        let Some(mut config) = surface.get_default_config(adapter, size.width.max(1), size.height.max(1)) else { return eprintln!("ghost window: no surface config") };
        // the pipelines are built for the main surface's format
        if !surface.get_capabilities(adapter).formats.contains(&main.format) {
            return eprintln!("ghost window: the surface does not support {:?}", main.format);
        }
        config.format = main.format;
        config.present_mode = wgpu::PresentMode::AutoNoVsync;
        surface.configure(&gpu.device, &config);
        let depth = gpu.depth_view(config.width, config.height);
        let now = Instant::now();
        let follow = match (self.ghost_follow, &self.ghost, &self.sess) {
            (Follow::Ghost, None, Some(s)) => Follow::Car(s.focus),
            (f, ..) => f,
        };
        self.ghost_win = Some(GhostWin { window, surface, config, depth, cam: PoseCam::new(self.ghost_cam), follow, last: now, title_at: now, held: None });
    }

    fn ghost_event(&mut self, ev: WindowEvent) {
        match ev {
            WindowEvent::CloseRequested => self.ghost_win = None,
            WindowEvent::Resized(s) => {
                if let (Some(gw), Some(g)) = (self.ghost_win.as_mut(), &self.gpu) {
                    gw.config.width = s.width.max(1);
                    gw.config.height = s.height.max(1);
                    gw.surface.configure(&g.device, &gw.config);
                    gw.depth = g.depth_view(gw.config.width, gw.config.height);
                }
            }
            WindowEvent::KeyboardInput { event, .. } if event.state == ElementState::Pressed && !event.repeat => {
                if let PhysicalKey::Code(code) = event.physical_key {
                    match code {
                        KeyCode::KeyC | KeyCode::Tab | KeyCode::KeyY => self.act(Act::GhostCam, 0),
                        KeyCode::Escape => self.ghost_win = None,
                        _ => {}
                    }
                }
            }
            _ => {}
        }
    }

    /// Draw the ghost window after the main one (own targets: `Gpu::select_view`).
    fn ghost_frame(&mut self) {
        let Some(mut gw) = self.ghost_win.take() else { return };
        self.draw_ghost_window(&mut gw);
        // kept while there is a race to show
        if self.sess.is_some() {
            self.ghost_win = Some(gw);
        }
    }

    fn draw_ghost_window(&self, gw: &mut GhostWin) {
        let (Some(gpu), Some(sess)) = (&self.gpu, &self.sess) else { return };
        let g = self.ghost.as_ref();
        let now = Instant::now();
        let dt = (now - gw.last).as_secs_f32().min(0.1);
        gw.last = now;
        let frame = match gw.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(t) | wgpu::CurrentSurfaceTexture::Suboptimal(t) => t,
            wgpu::CurrentSurfaceTexture::Outdated | wgpu::CurrentSurfaceTexture::Lost => return gw.surface.configure(&gpu.device, &gw.config),
            _ => return,
        };
        let at = match gw.follow {
            Follow::Ghost => g.and_then(|g| g.pose(sess)).map(|p| (p.pos, p.rot)),
            Follow::Car(i) => sess.race.racers.get(i).map(|r| (r.car.pos, r.car.rot)),
        };
        let cam = match at {
            Some((pos, rot)) => {
                let c = gw.cam.camera(&sess.race, &sess.tune, pos, rot, dt);
                gw.held = Some(c);
                c
            }
            None => gw.held.unwrap_or_else(|| sess.camera()),
        };
        if gw.title_at.elapsed().as_millis() > 250 {
            gw.title_at = now;
            let mode = tune::CAM_MODES.iter().find(|m| m.0 == gw.cam.mode).map_or("?", |m| m.1);
            let who = match gw.follow {
                Follow::Ghost => g.map_or("ghost".to_string(), |g| g.name.clone()),
                Follow::Car(i) => sess.race.racers.get(i).map_or("?".to_string(), |r| format!("#{i} {}", r.driver.name)),
            };
            let state = if at.is_some() { String::new() } else { " (waiting for the ghost's lap)".into() };
            gw.window.set_title(&format!("motorview — {who} — {mode} camera{state}"));
        }
        let size = (gw.config.width, gw.config.height);
        let target = frame.texture.create_view(&Default::default());
        let view = self.dbg.passes.view(&self.view);
        let mut parts = self.dbg.passes.parts(Some(sess), &cam, &view);
        ghost_scene(&mut parts, sess, g, gw.cam.mode, gw.follow);
        let f = parts.frame();
        gpu.select_view(1);
        let mut enc = gpu.device.create_command_encoder(&Default::default());
        gpu.draw(&mut enc, &target, &gw.depth, size, &cam, &view, &f);
        gpu.queue.submit([enc.finish()]);
        gpu.queue.present(frame);
        gpu.select_view(0);
    }

    /// The held driving keys as a car input: the arrows, and WASD too when `wasd` (the race camera).
    /// Digital steering with the game's half-lock-then-ramp keyboard behaviour.
    fn key_input(&mut self, wasd: bool, dt: f32) -> Input {
        let ctrl = self.mods & CTRL != 0;
        let held = |arrow: KeyCode, letter: KeyCode| !ctrl && (self.keys.contains(&arrow) || wasd && self.keys.contains(&letter));
        let (left, right) = (held(KeyCode::ArrowLeft, KeyCode::KeyA), held(KeyCode::ArrowRight, KeyCode::KeyD));
        let (throttle, brake) = (held(KeyCode::ArrowUp, KeyCode::KeyW), held(KeyCode::ArrowDown, KeyCode::KeyS));
        Input { throttle: throttle as u8 as f32, brake: brake as u8 as f32, steer: self.steer_keys.update(left, right, dt), ..Default::default() }
    }

    fn update(&mut self, dt: f32) {
        self.body_poll();
        self.fill_update();
        self.sky_update();
        if self.cine.active() && self.sess.is_some() {
            if self.cine_update(dt) {
                self.fly(dt);
            }
            return;
        }
        // the editor closed while previewing: the viewer's camera back
        self.cine_unpreview();
        // Rewind / Forward act only while held (ribbon buttons, `,` and `/`); the sim then goes back
        // to what it was doing, playing or paused
        let held = std::mem::take(&mut self.scrub);
        let scrub = match held {
            0 if self.key(&[KeyCode::Comma]) => -1,
            0 if self.key(&[KeyCode::Slash]) => 1,
            h => h,
        };
        let scale = self.dbg.time_scale();
        let mut sim_dt = self.dbg.sim_dt(dt);
        let mut reps = 1;
        match scrub {
            -1 => {
                let back = dt * scale;
                if self.sess.as_mut().is_some_and(|s| s.rewind(back)) {
                    if let Some(g) = self.ghost.as_mut() {
                        g.advance(-back);
                    }
                }
                sim_dt = None;
            }
            1 => {
                sim_dt = Some(sim_dt.unwrap_or(dt * scale));
                reps = FORWARD_STEPS;
            }
            _ => {}
        }
        if let (Some(g), Some(dt)) = (self.ghost.as_mut(), sim_dt) {
            g.advance(dt * reps as f32);
        }
        // the timeline slider
        if let Some(to) = self.seek.take() {
            if let Some(moved) = self.sess.as_mut().and_then(|s| s.seek(to)) {
                if let Some(g) = self.ghost.as_mut() {
                    g.advance(moved as f32);
                }
            }
        }
        if self.free {
            // #60: with hand driving the arrows steer, accelerate and brake the followed car (the
            // WASD keys keep flying the camera)
            let driving = self.drive_free && self.sess.is_some();
            let input = if driving { self.key_input(false, sim_dt.unwrap_or(0.0)) } else { Input::default() };
            if let (Some(sess), Some(dt)) = (self.sess.as_mut(), sim_dt) {
                sess.fx.remaster = self.view.mode == motorview::render::RenderMode::Remaster;
                let hand = if driving { sess.hand_slot() } else { None };
                for _ in 0..reps {
                    sess.step_as(dt, input, hand);
                }
            }
        } else if self.sess.is_some() {
            let input = self.key_input(true, sim_dt.unwrap_or(0.0));
            let look_back = self.keys.contains(&KeyCode::KeyB);
            let Some(sess) = self.sess.as_mut() else { return };
            sess.look_back = look_back;
            sess.fx.remaster = self.view.mode == motorview::render::RenderMode::Remaster;
            if let Some(dt) = sim_dt {
                for _ in 0..reps {
                    sess.step(dt, input);
                }
            }
            self.cam = chase(sess);
            if self.title_at.elapsed().as_millis() > 150 {
                self.title_at = Instant::now();
                if let Some(w) = &self.window {
                    w.set_title(&format!("motorview — {}", sess.hud()));
                }
            }
            return;
        }
        self.fly(dt);
    }

    /// The free camera flies: WASD / QE, Shift fast, the arrows turn (not while they drive the car).
    fn fly(&mut self, dt: f32) {
        let speed = if self.key(&[KeyCode::ShiftLeft, KeyCode::ShiftRight]) { 60.0 } else { 15.0 } * self.speed * dt;
        let (f, r) = (self.cam.forward(), self.cam.right());
        let dirs = [(KeyCode::KeyW, f), (KeyCode::KeyS, -f), (KeyCode::KeyD, r), (KeyCode::KeyA, -r), (KeyCode::KeyE, Vec3::Y), (KeyCode::KeyQ, -Vec3::Y)];
        // Ctrl+key is a shortcut (Ctrl+S saves), not movement
        let m: Vec3 = if self.mods & CTRL != 0 { Vec3::ZERO } else { dirs.iter().filter(|(k, _)| self.keys.contains(k)).map(|(_, d)| *d).sum() };
        self.cam.pos += m * speed;
        let turn = if self.drive_free { 0.0 } else { 1.5 * dt };
        if self.key(&[KeyCode::ArrowLeft]) {
            self.cam.yaw -= turn;
        }
        if self.key(&[KeyCode::ArrowRight]) {
            self.cam.yaw += turn;
        }
        if self.key(&[KeyCode::ArrowUp]) {
            self.cam.pitch += turn;
        }
        if self.key(&[KeyCode::ArrowDown]) {
            self.cam.pitch -= turn;
        }
        self.cam.pitch = self.cam.pitch.clamp(-1.5, 1.5);
    }

    /// Draw the scene, rear mirror and overlay of this frame into `target`.
    fn render(&mut self, gpu: &Gpu, enc: &mut wgpu::CommandEncoder, target: &wgpu::TextureView, depth: &wgpu::TextureView, size: (u32, u32)) {
        let sess = self.sess.as_ref();
        let mut view = self.dbg.passes.view(&self.view);
        // looking through a cine camera: the video's picture, no AI racing line (as `cine_render`)
        if self.cine.preview_cam().is_some() {
            view.show_line = false;
        }
        let mut parts = self.dbg.passes.parts(sess, &self.cam, &view);
        if let (Some(g), Some(s)) = (self.ghost.as_ref().filter(|g| g.show_main), sess) {
            parts.ghost = g.models(s);
        }
        let hit = self.picker.as_ref().and_then(|p| p.cast(self.cam.pos, self.cam.forward(), PICK_RANGE, &parts.cars, &parts.props));
        let face = sess.and_then(|s| col_hit(s, &self.cam, hit)).map(|c| c.face as usize);
        let mut g = sess.map(|s| self.dbg.gizmos(s, &view, &self.cam, face, self.scene.as_ref().zip(self.picker.as_ref()), &parts, hit)).unwrap_or_default();
        if self.world.open {
            self.world.gizmos(&mut g, self.cam.pos * world_edit::FLIP);
        }
        if self.cine_gizmos() {
            self.cine.gizmos(&mut g, self.cam.pos * world_edit::FLIP, self.view.fov_x, sess.map(|s| &s.race));
        }
        let mut f = parts.frame();
        f.wire = self.wireframe;
        f.debug_lines = &g.lines;
        f.debug_xray = &g.xray;
        f.debug_xray_tris = &g.xray_tris;
        gpu.draw(enc, target, depth, size, &self.cam, &view, &f);
        if let (true, Some(ss)) = (self.mirror, sess.filter(|s| !s.look_back)) {
            // the mirror is a second scene pass: the shared uniforms take their values at submit
            gpu.queue.submit([std::mem::replace(enc, gpu.device.create_command_encoder(&Default::default())).finish()]);
            let (w, h, top) = (size.0 as f32 * 0.25, size.1 as f32 * 0.08, 8.0 + self.bar_h);
            let x = (size.0 as f32 - w) * 0.5;
            let (eye, at) = ss.mirror_view();
            let look = (at - eye).normalize_or(Vec3::Z);
            let cam = Camera::new(eye, look.x.atan2(-look.z), look.y.asin());
            let mcars = ss.models_hiding(Some(ss.focus));
            let mhidden = ss.hidden_bodies(Some(ss.focus));
            let mut mf = Frame::new(&mcars, &[]);
            mf.props = &parts.props;
            mf.prop_glow = &parts.glows;
            mf.car_brake = &parts.brakes;
            mf.car_spin = &parts.spins;
            mf.light_cars = &mhidden;
            mf.sprites = &parts.sprites;
            mf.mirror = true;
            mf.viewport = Some([x, top, w, h]);
            gpu.draw(enc, target, depth, size, &cam, &view, &mf);
        }
        let crosshair = !self.clean && self.cross && ((self.free && !self.world.open) || self.overlay);
        // texture preview: the browsed texture, else the pinned target's
        let want = self.dbg.catalog.as_ref().and_then(|c| self.dbg.browser.preview(c)).or(self.dbg.pinned_layer);
        if want != self.preview {
            if let (Some(l), Some(a), Some(slot), Some(s)) = (want, self.assets.as_mut(), self.preview_slot, self.scene.as_ref()) {
                upload_preview(gpu, a, slot, s, l);
            }
            self.preview = want;
        }
        let sess = self.sess.as_ref();
        let mut text = OverlayText { panel: self.dbg.panel(sess), labels: g.labels, preview: want.is_some(), ..Default::default() };
        if let (Some(s), Some(c)) = (&self.scene, &self.dbg.catalog) {
            text.right.push(self.dbg.browser.lines(s, c, sess));
        }
        text.right.extend(self.dbg.pinned.clone());
        text.right.push(self.dbg.render_lines(&self.view));
        if let (Some(p), true) = (&self.picker, self.overlay) {
            text.info = info_lines(Some(self.fps.0), self.track, &self.cam, p, hit, sess).into_iter().map(Line::from).collect();
            if self.free {
                text.info.insert(2, format!("camera speed: x{:.2} ({:.1} m/s)", self.speed, 15.0 * self.speed).into());
            }
        }
        if !self.clean && (self.overlay || self.dbg.paused || self.dbg.time_scale() != 1.0) {
            text.info.push(self.dbg.status().into());
        }
        if self.view.debug.shade != DebugShade::Off {
            text.info.push(format!("shading view: {} (F10)", self.view.debug.shade.name()).into());
        }
        // `gpu` is the parameter: `self.gpu` is taken out while a frame renders
        if self.wireframe.on() {
            text.info.extend(wire_lines(self.wireframe, gpu, self.sess.as_ref().map(|s| s.focus), self.view.generated_cars()));
        }
        if let Some((n, _)) = self.note.as_ref().filter(|(_, t)| !self.clean && t.elapsed().as_secs_f32() < 2.0) {
            text.info.push(n.clone().into());
        }
        if self.clean {
            self.ov = None;
            return;
        }
        // the egui pass paints the text (`egui`); the texture preview is a UI atlas image
        if let (Some(assets), true) = (&self.assets, text.preview) {
            let (canvas, verts) = overlay::preview(assets, size, &text, self.status_h);
            gpu.draw_ui(enc, target, size, canvas, &verts, None, None);
        }
        self.ov = (crosshair || !text.is_empty()).then_some((text, crosshair));
    }

    fn frame(&mut self) {
        let (Some(surface), Some(mut gpu), Some(depth), Some(cfg)) = (self.surface.take(), self.gpu.take(), self.depth.take(), self.config.clone()) else { return };
        gpu.set_car_swap(self.sess.as_ref().filter(|_| self.mesh_swap).map(|s| s.focus));
        gpu.update_local_light(&self.view);
        self.local_status = gpu.local_status();
        self.frame_with(&surface, &gpu, &depth, &cfg);
        (self.surface, self.gpu, self.depth) = (Some(surface), Some(gpu), Some(depth));
        for (act, i) in std::mem::take(&mut self.pending) {
            self.act(act, i);
        }
        self.cine_apply();
        if std::mem::take(&mut self.repaint_due) {
            self.repaint();
        }
        if let Some(p) = self.ghost_load.take() {
            self.load_ghost(&p);
        }
        self.ghost_frame();
    }

    /// The egui pass: the ribbon and the Properties pane.
    fn egui(&mut self, ui: &mut egui::Ui) {
        self.bar_h = 0.0;
        self.status_h = 0.0;
        self.pane_w = 0.0;
        self.left_w = 0.0;
        if self.egui_shown() {
            self.panels(ui);
        }
        if let Some((text, crosshair)) = &self.ov {
            let size = self.config.as_ref().map_or((1, 1), |c| (c.width, c.height));
            overlay::paint(ui.ctx(), size, &self.cam, self.view.fov_x, text, *crosshair, (self.bar_h, self.pane_w, self.status_h, self.left_w));
        }
        if self.confirm_exit {
            match overlay::exit_dialog(ui.ctx()) {
                Some(true) => self.exit_now = true,
                Some(false) => self.confirm_exit = false,
                None => {}
            }
        }
    }

    /// The ribbon, status bar, Properties pane and World Editor panels.
    fn panels(&mut self, ui: &mut egui::Ui) {
        // outliner double-click: frame the piece
        if let Some((c, r)) = self.world.focus.take() {
            self.look_at(c * world_edit::FLIP, r);
        }
        let ppp = ui.ctx().pixels_per_point();
        if self.menu_bar {
            let mut m = Ribbon {
                st: &mut self.ribbon,
                tune: &mut self.tune,
                view: &mut self.view,
                dbg: &mut self.dbg,
                sess: self.sess.as_mut(),
                free: self.free,
                drive_free: self.drive_free,
                overlay: self.overlay,
                cross: self.cross,
                wireframe: &mut self.wireframe,
                mirror: self.mirror,
                track: self.track,
                car: self.car,
                mesh_swap: self.mesh_swap,
                world: Some(&mut self.world),
                ghost: self.ghost.as_mut(),
                ghost_win: self.ghost_win.as_ref().map(|g| g.cam.mode),
                ghost_cam: self.ghost_cam,
                ghost_follow: self.ghost_win.as_ref().map_or(self.ghost_follow, |g| g.follow),
                scrub: &mut self.scrub, seek: &mut self.seek,
                cine: Some(&mut self.cine),
                acts: Vec::new(),
            };
            self.bar_h = m.bar(ui) * ppp;
            self.pending.extend(m.acts);
        }
        let textures: &[String] = self.scene.as_ref().map_or(&[], |s| &s.world_textures_sorted);
        if self.world.open {
            self.bar_h += palette::options(ui, &mut self.world, textures) * ppp;
        }
        let note = self.note.as_ref().filter(|(_, t)| t.elapsed().as_secs_f32() < 4.0).map_or("", |(n, _)| n.as_str());
        let note = if note.is_empty() { self.world.note.as_str() } else { note };
        let hint = if self.cine_mouse() { cine_edit::HINT.to_string() } else { self.world.hint(self.world.open, self.free, self.sess.is_some(), self.mods & SHIFT != 0) };
        let sb = status::StatusBar { hint, note, fps: self.fps.0, sim: self.dbg.status(), job: self.cine.job_status() };
        self.status_h = sb.show(ui) * ppp;
        self.left_w = if self.world.open { palette::palette(ui, &mut self.world) * ppp } else { 0.0 };
        let track_fog = self.scene.as_ref().map(|s| (self.track, s.look.fog));
        let track_sky = self.scene.as_ref().and_then(|s| s.hdri.as_ref()).map(|h| (self.track, h.dome()));
        if self.tune.page == Some(Page::Ghost) {
            self.ghost_files.get_or_insert_with(ghost::list);
        }
        let world = &mut self.world;
        let (cine, root, names) = (&mut self.cine, &self.args.root, cine_edit::names(self.sess.as_ref()));
        let mut page = |p: Page, ui: &mut egui::Ui| match p {
            Page::World => world.panel_body(ui, textures),
            Page::Ghost => ghost_page(ui, &mut self.ghost_path, &self.ghost_msg, &mut self.ghost_files, self.ghost.as_ref(), &mut self.ghost_load),
            Page::Cine => cine_edit::page(ui, cine, root, &names),
            _ => {}
        };
        let paint = self.sess.as_ref().and_then(|ss| {
            let own = *self.scene.as_ref()?.car_colours.get(ss.focus)?;
            Some((self.paints.get(&ss.focus).copied().unwrap_or(own), own))
        });
        self.pane_w = self.tune.pane(ui, self.sess.as_mut(), Targets { view: &mut self.view, track_fog, track_sky, paint, local_status: self.local_status, probe_gizmos: Some(&mut self.dbg.layers.probes) }, &mut page) * ppp;
        // a picker drag reloads the scene once, when the button is let go
        if self.tune.paint_req.is_some() && !ui.ctx().input(|i| i.pointer.any_down()) {
            if let Some((slot, c)) = self.tune.paint_req.take() {
                let own = self.scene.as_ref().and_then(|s| s.car_colours.get(slot)).copied();
                if own == Some(c) {
                    self.paints.remove(&slot);
                } else {
                    self.paints.insert(slot, c);
                }
                // after the frame: the frame holds the GPU the new scene is uploaded to
                self.repaint_due = true;
            }
        }
        if let Some((n, f)) = self.tune.fog_saved.take() {
            match motorview::fog::save(&self.args.root, n, &f) {
                Ok(p) => {
                    eprintln!("fog saved: {}", p.display());
                    if let Some(s) = self.scene.as_mut() {
                        s.look.fog = f;
                    }
                }
                Err(e) => eprintln!("fog not saved: {e}"),
            }
        }
    }

    /// The ribbon or the Properties pane is up (egui gets the window input).
    fn egui_shown(&self) -> bool {
        self.menu_bar || self.tune.page.is_some() || self.confirm_exit
    }

    /// egui has something to draw: panels, the exit dialog or the overlay text.
    fn egui_paints(&self) -> bool {
        self.egui_shown() || self.ov.is_some()
    }

    fn frame_with(&mut self, surface: &wgpu::Surface<'static>, gpu: &Gpu, depth: &wgpu::TextureView, cfg: &wgpu::SurfaceConfiguration) {
        let frame = match surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(t) | wgpu::CurrentSurfaceTexture::Suboptimal(t) => t,
            wgpu::CurrentSurfaceTexture::Outdated | wgpu::CurrentSurfaceTexture::Lost => {
                surface.configure(&gpu.device, cfg);
                return;
            }
            _ => return,
        };
        let view = frame.texture.create_view(&Default::default());
        let size = (cfg.width, cfg.height);
        let mut enc = gpu.device.create_command_encoder(&Default::default());
        self.render(gpu, &mut enc, &view, depth, size);
        // only take the egui host while it shows something (a tuple would take it every frame)
        let shown = self.egui_paints();
        if let Some((w, mut gui)) = self.window.clone().filter(|_| shown).zip(self.gui.take_if(|_| shown)) {
            if let Some(p) = gui.run(&w, |ui| self.egui(ui)) {
                gui.paint(&gpu.device, &gpu.queue, &mut enc, &view, size, p);
            }
            self.gui = Some(gui);
        }
        gpu.queue.submit([enc.finish()]);
        gpu.queue.present(frame);
        if std::mem::take(&mut self.screenshot) {
            // no menu bar in the shot: the overlay and mirror go back to the top
            let bar_h = std::mem::take(&mut self.bar_h);
            self.clean = true;
            let mut px = gpu.offscreen(size.0, size.1, |g, enc, target, depth| self.render(g, enc, target, depth, size));
            self.bar_h = bar_h;
            self.clean = false;
            self.save_screenshot(cfg.format, size, &mut px);
        }
    }

    /// F12: `motorview-NNNN.png` (first free number) in the working directory.
    fn save_screenshot(&mut self, format: wgpu::TextureFormat, size: (u32, u32), px: &mut [u8]) {
        match format {
            wgpu::TextureFormat::Bgra8Unorm | wgpu::TextureFormat::Bgra8UnormSrgb => px.as_chunks_mut::<4>().0.iter_mut().for_each(|p| p.swap(0, 2)),
            // packed 10:10:10:2 (r in the low bits): the surface can pick it when it is the first non-sRGB format
            wgpu::TextureFormat::Rgb10a2Unorm => px.as_chunks_mut::<4>().0.iter_mut().for_each(|p| {
                let v = u32::from_le_bytes(*p);
                *p = [(v & 0x3ff) >> 2, (v >> 10 & 0x3ff) >> 2, (v >> 20 & 0x3ff) >> 2, 255].map(|c| c as u8);
            }),
            _ => {}
        }
        let Some(path) = (0..10000).map(|n| PathBuf::from(format!("motorview-{n:04}.png"))).find(|p| !p.exists()) else { return };
        let msg = match png::write(&path, size.0, size.1, px) {
            Ok(()) => format!("screenshot: {}", path.display()),
            Err(e) => format!("screenshot: {}: {e}", path.display()),
        };
        println!("{msg}");
        self.note(msg);
    }

    /// A key press (not a repeat) outside the exit requester.
    fn press(&mut self, code: KeyCode) {
        // F1 render panel: the digits toggle passes (1-9) and the unlimited distance (0)
        if self.dbg.render_panel && self.mods == 0 {
            let digits = [K::Digit1, K::Digit2, K::Digit3, K::Digit4, K::Digit5, K::Digit6, K::Digit7, K::Digit8, K::Digit9, K::Digit0];
            if let Some(i) = digits.iter().position(|&k| k == code) {
                match self.dbg.passes.get(i) {
                    Some(p) => *p ^= true,
                    None => self.view.debug.unlimited ^= true,
                }
                return;
            }
        }
        let ctx = if self.free { Ctx::Free } else { Ctx::Race };
        let Some(b) = find_bind(code, self.mods, ctx, self.world.open) else { return };
        self.act(b.act, b.keys.iter().position(|&k| k == code).unwrap_or(0));
    }

    /// Run a key's or menu item's action; `i` is the key's position in its bind (track) or the
    /// menu's choice (track, car).
    fn act(&mut self, act: Act, i: usize) {
        match act {
            Act::Escape if self.grabbed => self.grab(false),
            Act::Escape if self.world.open && self.world.cancel() => self.note("piece cancelled"),
            Act::Escape => self.confirm_exit = true,
            Act::FreeCam => {
                self.free ^= true;
                // the race pauses in free camera and runs again in the race camera
                self.dbg.paused = self.free;
                self.grab(self.free);
            }
            Act::Overlay => self.overlay ^= true,
            Act::Cross => self.cross ^= true,
            Act::Wireframe => self.wireframe = self.wireframe.next(),
            Act::WireframeBack => self.wireframe = self.wireframe.prev(),
            Act::Mirror => self.mirror ^= true,
            Act::Pause => self.dbg.paused ^= true,
            Act::Step => {
                self.dbg.paused = true;
                self.dbg.steps += 1;
            }
            Act::Slower => self.dbg.scale = self.dbg.scale.saturating_sub(1),
            Act::Faster => self.dbg.scale = (self.dbg.scale + 1).min(TIME_SCALES.len() - 1),
            Act::Focus => {
                if let Some(s) = self.sess.as_mut() {
                    s.cycle_focus();
                    let msg = format!("following car #{}", s.focus);
                    self.note(msg);
                }
            }
            Act::Inspect => self.inspect(),
            Act::Panel => self.dbg.panel = (self.dbg.panel + 1) % 3,
            Act::CarGizmos => {
                let l = &mut self.dbg.layers;
                (l.car, l.all_cars) = match (l.car, l.all_cars) {
                    (false, _) => (true, false),
                    (true, false) => (true, true),
                    _ => (false, false),
                };
            }
            Act::TrackGizmos => self.dbg.layers.track ^= true,
            Act::ColGizmos => self.dbg.layers.collision ^= true,
            Act::BoundGizmos => self.dbg.layers.bounds ^= true,
            Act::LightGizmos => self.dbg.layers.lights ^= true,
            Act::ProbeGizmos => self.dbg.layers.probes ^= true,
            Act::Shade => self.view.debug.shade = self.view.debug.shade.next(),
            Act::RenderPanel => self.dbg.render_panel ^= true,
            Act::Browse => self.dbg.browser.cycle(),
            Act::BrowseUp | Act::BrowseDown | Act::BrowseFirst | Act::BrowseLast => {
                let Some(s) = &self.scene else { return };
                let d = match act {
                    Act::BrowseUp => -1,
                    Act::BrowseDown => 1,
                    Act::BrowseFirst => isize::MIN / 2,
                    _ => isize::MAX / 2,
                };
                self.dbg.browser.step(s, d);
            }
            Act::Jump => {
                let (Some(p), Some(c)) = (&self.picker, &self.dbg.catalog) else { return };
                match self.dbg.browser.jump(p, c, self.sess.as_mut()) {
                    Some((at, r, what)) => {
                        self.look_at(at, r);
                        self.note(format!("jumped to {what}"));
                    }
                    None => self.note("nothing to jump to"),
                }
            }
            Act::Screenshot => self.screenshot = true,
            Act::ReloadCars => self.reload_cars(true),
            Act::CineAddCam | Act::CineAddKey | Act::CinePreview | Act::CineCut if !self.cine.active() => self.note("Cine: open or start a sequence first (ribbon Cine tab)"),
            Act::CineAddCam => self.cine.reqs.push(Req::AddStatic),
            Act::CineAddKey => self.cine.reqs.push(Req::AddKey),
            Act::CineCut => self.cine.reqs.push(Req::Cut),
            Act::CinePreview => {
                self.cine.preview = self.cine.preview.next();
                self.note(format!("cine preview: {}", self.cine.preview.name()));
            }
            Act::SwapMesh => {
                self.mesh_swap ^= true;
                self.note(self.mesh_note());
            }
            Act::Tune => {
                self.tune.toggle();
                // the pane needs the mouse
                if self.tune.page.is_some() && self.grabbed {
                    self.grab(false);
                }
            }
            Act::RacingLine => self.view.show_line ^= true,
            Act::Fog => self.view.fog ^= true,
            Act::Shadows => self.view.shadows ^= true,
            Act::CarShading => self.view.car_shading = self.view.car_shading.cycle(true),
            Act::Halos => self.view.halos ^= true,
            Act::Camera => {
                if let Some(ss) = self.sess.as_mut() {
                    ss.cam = ss.cam.next();
                }
            }
            Act::Mode => self.view.mode = self.view.mode.toggle(),
            Act::Paint => self.view.remaster.enhanced_paint ^= true,
            Act::Lights => self.view.remaster.dynamic_lights ^= true,
            Act::Msaa => {
                let l = RemasterView::MSAA_LEVELS;
                let i = l.iter().position(|&n| n == self.view.remaster.msaa()).unwrap_or(0);
                self.view.remaster.render_aa = RenderAa::msaa(l[(i + 1) % l.len()]);
            }
            Act::Grade => {
                let r = &mut self.view.remaster;
                let p = if r.post() { 0 } else { 50 };
                (r.grade, r.vignette, r.grain) = (p, p, p);
            }
            Act::Respawn => {
                if let Some(s) = &mut self.sess {
                    s.respawn_focus();
                }
            }
            Act::DriveFree => {
                self.drive_free ^= true;
                if self.drive_free {
                    // the race runs while the arrows drive
                    self.dbg.paused = false;
                    let msg = format!("driving car #{} by hand (arrows)", self.sess.as_ref().map_or(0, |s| s.focus));
                    self.note(msg);
                } else {
                    self.note("hand driving off");
                }
            }
            Act::ResetCamera => self.reset_camera(),
            Act::Restart => {
                if let Some(s) = &mut self.sess {
                    s.restart();
                }
            }
            Act::Darker => self.view.brightness = (self.view.brightness - 0.1).max(0.1),
            Act::Brighter => self.view.brightness += 0.1,
            Act::NextCar => self.switch_car(self.car % 10 + 1),
            Act::Track => self.switch(i as u32 + 1, self.car),
            Act::Car => self.switch_car(i as u32 + 1),
            Act::GhostWindow if self.ghost_win.is_some() => self.ghost_win = None,
            Act::GhostWindow if self.sess.is_some() => self.ghost_open = true,
            Act::GhostWindow => self.note("no race loaded"),
            Act::GhostFollow => {
                let follow = if i == 0 { Follow::Ghost } else { Follow::Car(i - 1) };
                if follow == Follow::Ghost && self.ghost.is_none() {
                    self.note("load a ghost first (ribbon Ghost tab)");
                } else {
                    self.ghost_follow = follow;
                    if let Some(gw) = self.ghost_win.as_mut() {
                        (gw.follow, gw.held) = (follow, None);
                    }
                }
            }
            Act::GhostCam => {
                if let Some(gw) = self.ghost_win.as_mut() {
                    gw.cam.next();
                    self.ghost_cam = gw.cam.mode;
                }
            }
            Act::GhostCamSet => {
                self.ghost_cam = PoseCam::MODES[i.min(PoseCam::MODES.len() - 1)];
                if let Some(gw) = self.ghost_win.as_mut() {
                    gw.cam = PoseCam::new(self.ghost_cam);
                }
            }
            Act::SaveRun => self.save_run(),
            Act::OpenRun => {
                if let Some(e) = ghost::runs().get(i) {
                    self.open_run(&e.path);
                }
            }
            Act::GhostClear => {
                self.ghost = None;
                // a window that followed the ghost follows the car under the camera instead
                let focus = self.sess.as_ref().map_or(0, |s| s.focus);
                if self.ghost_follow == Follow::Ghost {
                    self.ghost_follow = Follow::Car(focus);
                }
                if let Some(gw) = self.ghost_win.as_mut().filter(|g| g.follow == Follow::Ghost) {
                    (gw.follow, gw.held) = (Follow::Car(focus), None);
                }
            }
            Act::WorldToggle => {
                self.fill_toggle();
                // the editor's controls are the pane's World page
                if self.world.open {
                    self.tune.page = Some(Page::World);
                } else if self.tune.page == Some(Page::World) {
                    self.tune.page = None;
                }
            }
            Act::World(c) if self.world.open => self.world.cmds.push(c),
            Act::WorldNew if self.world.open => self.world.cmds.push(Cmd::Start(Kind::ALL[i.min(5)])),
            Act::World(_) | Act::WorldNew => {}
            Act::MenuBar => {
                self.menu_bar ^= true;
                if !self.menu_bar {
                    self.bar_h = 0.0;
                }
                self.ribbon.minimised = false;
            }
        }
    }
}

impl ApplicationHandler for App {
    fn resumed(&mut self, el: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }
        let window = Arc::new(
            el.create_window(Window::default_attributes().with_title("motorview").with_inner_size(winit::dpi::LogicalSize::new(1280, 720)))
                .expect("create window"),
        );
        let inst = instance();
        let surface = inst.create_surface(window.clone()).expect("create surface");
        let adapter = pollster::block_on(inst.request_adapter(&wgpu::RequestAdapterOptions {
            compatible_surface: Some(&surface),
            ..Default::default()
        }))
        .expect("no GPU adapter");
        println!("adapter: {}", adapter.get_info().name);
        let (dev, queue) = pollster::block_on(device(&adapter));
        let size = window.inner_size();
        let mut cfg = surface.get_default_config(&adapter, size.width.max(1), size.height.max(1)).expect("surface config");
        // no vsync: FPS_LIMIT paces the frames
        cfg.present_mode = wgpu::PresentMode::AutoNoVsync;
        // gamma-space output, like the original: no sRGB encode on write
        if let Some(f) = surface.get_capabilities(&adapter).formats.into_iter().find(|f| !f.is_srgb()) {
            cfg.format = f;
        }
        surface.configure(&dev, &cfg);
        let mut gpu = Gpu::new(&adapter, dev, queue, cfg.format);
        (self.inst, self.adapter) = (Some(inst), Some(adapter));
        let mut assets = Assets::font_only(&self.args.root);
        self.preview_slot = assets.atlas.reserve("preview", TEX, TEX);
        gpu.load_ui(&assets.atlas);
        self.assets = Some(assets);
        self.gui = Some(Gui::new(&gpu.device, cfg.format, Some(&window)));
        self.gpu = Some(gpu);
        self.depth = Some(self.gpu.as_ref().unwrap().depth_view(cfg.width, cfg.height));
        self.config = Some(cfg);
        self.surface = Some(surface);
        self.window = Some(window);
        if let Some(p) = self.args.cine.clone().filter(|_| self.args.cine_edit) {
            self.cine.reqs.push(Req::Open(cine_edit::resolve(&self.args.root, &p)));
            self.cine_apply();
        }
        if !self.cine.active() && !self.args.replay.clone().is_some_and(|p| self.open_run(&p)) {
            self.switch(self.track, self.car);
        }
        if let Some(p) = self.args.ghost.clone() {
            self.load_ghost(&p);
        }
        self.ghost_open = self.args.ghost_window;
        if self.free {
            self.grab(true);
        }
        self.last = Instant::now();
    }

    fn window_event(&mut self, el: &ActiveEventLoop, id: WindowId, ev: WindowEvent) {
        if self.ghost_win.as_ref().is_some_and(|g| g.window.id() == id) {
            self.ghost_event(ev);
            return;
        }
        // the menu bar and tuning window take clicks, wheel and typing over them; key releases
        // always pass, and so does the mouse while the free camera holds it
        if let (true, Some(g), Some(w)) = (self.egui_shown(), self.gui.as_mut(), self.window.as_ref()) {
            let used = g.on_event(w, &ev);
            let take = match &ev {
                WindowEvent::KeyboardInput { event, .. } => event.state == ElementState::Pressed && (used || g.wants_keys()),
                WindowEvent::MouseInput { state: ElementState::Pressed, .. } | WindowEvent::MouseWheel { .. } => !self.grabbed && (used || g.wants_pointer()),
                _ => false,
            };
            if take {
                return;
            }
        }
        match ev {
            WindowEvent::CloseRequested => el.exit(),
            WindowEvent::Resized(s) => self.resize(s.width, s.height),
            WindowEvent::MouseInput { state, button: MouseButton::Right, .. } => self.look_grab(state == ElementState::Pressed),
            WindowEvent::CursorMoved { position, .. } => {
                self.cursor = (position.x as f32, position.y as f32);
                if self.world.drag.is_some() {
                    if let Some(p) = self.fill_cast() {
                        self.world.drag_to(p);
                    }
                } else if self.world.piece_dragging() {
                    self.fill_piece_move();
                } else if self.world.painting() {
                    self.fill_paint_move();
                } else if self.world.open {
                    self.fill_gizmo_move();
                    // the plant brush ring follows the cursor
                    self.world.cursor_ground = if self.world.armed() == Some(Kind::Plant) { self.fill_cast() } else { None };
                } else if self.cine_mouse() || self.cine.dragging() {
                    self.cine_move();
                }
            }
            WindowEvent::MouseInput { state, button: MouseButton::Left, .. } if self.world.open && !self.confirm_exit => {
                if state == ElementState::Pressed {
                    self.fill_press();
                } else {
                    self.world.drag = None;
                    self.world.stroke_end();
                    self.world.gizmo_release();
                }
            }
            // the Cine tab: LMB picks and drags (a miss does nothing: RMB drag looks, as in the World editor)
            WindowEvent::MouseInput { state, button: MouseButton::Left, .. } if (self.cine_mouse() || self.cine.dragging()) && !self.confirm_exit => {
                if state == ElementState::Pressed {
                    self.cine_press();
                } else {
                    self.cine.release(self.sess.as_ref().map(|s| &s.race));
                }
            }
            WindowEvent::MouseInput { state: ElementState::Pressed, button: MouseButton::Left, .. } if !self.confirm_exit => {
                if self.free && !self.grabbed {
                    self.grab(true);
                } else if self.free || self.overlay {
                    self.copy_target();
                }
            }
            WindowEvent::ModifiersChanged(m) => {
                let m = m.state();
                self.mods = if m.control_key() { CTRL } else { 0 } | if m.shift_key() { SHIFT } else { 0 };
            }
            WindowEvent::Focused(false) => {
                self.mods = 0;
                self.keys.clear();
                self.look_grab(false);
                if self.grabbed {
                    self.grab(false);
                }
            }
            WindowEvent::MouseWheel { delta, .. } if self.free && self.key(&[KeyCode::ShiftLeft, KeyCode::ShiftRight]) => {
                let notches = match delta {
                    MouseScrollDelta::LineDelta(_, y) => y,
                    MouseScrollDelta::PixelDelta(p) => p.y as f32 / 40.0,
                };
                self.speed = (self.speed * 1.25f32.powf(notches)).clamp(SPEED_RANGE.0, SPEED_RANGE.1);
            }
            WindowEvent::KeyboardInput { event, .. } => {
                let PhysicalKey::Code(code) = event.physical_key else { return };
                if event.state == ElementState::Released {
                    self.keys.remove(&code);
                    return;
                }
                if event.repeat {
                    // held . steps the sim frame by frame, held PgUp / PgDn scroll the browser
                    if code == KeyCode::Period && !self.confirm_exit {
                        self.dbg.steps += 1;
                    }
                    if matches!(code, KeyCode::PageUp | KeyCode::PageDown) && !self.confirm_exit {
                        self.press(code);
                    }
                    return;
                }
                self.keys.insert(code);
                if self.confirm_exit {
                    match code {
                        KeyCode::KeyY | KeyCode::Enter | KeyCode::NumpadEnter => el.exit(),
                        KeyCode::KeyN | KeyCode::Escape => self.confirm_exit = false,
                        _ => {}
                    }
                    return;
                }
                self.press(code);
            }
            WindowEvent::RedrawRequested => {
                let now = Instant::now();
                let dt = (now - self.last).as_secs_f32().min(0.1);
                self.last = now;
                self.fps.1 += 1;
                let counted = (now - self.fps.2).as_secs_f32();
                if counted >= 0.5 {
                    self.fps = (self.fps.1 as f32 / counted, 0, now);
                }
                self.update(dt);
                self.frame();
                if self.exit_now {
                    el.exit();
                }
            }
            _ => {}
        }
    }

    /// Frame pacing: redraw now, or wait for the next FPS_LIMIT slot (as the game does).
    fn about_to_wait(&mut self, el: &ActiveEventLoop) {
        if std::mem::take(&mut self.ghost_open) {
            self.open_ghost_window(el);
        }
        let Some(w) = &self.window else { return };
        let period = Duration::from_secs_f64(1.0 / FPS_LIMIT);
        let now = Instant::now();
        if now >= self.next_frame {
            // keep a steady cadence; after a stall start counting again from now
            self.next_frame = if self.next_frame + period < now { now + period } else { self.next_frame + period };
            w.request_redraw();
        }
        el.set_control_flow(ControlFlow::WaitUntil(self.next_frame));
    }

    fn device_event(&mut self, _: &ActiveEventLoop, _: DeviceId, ev: DeviceEvent) {
        if let (DeviceEvent::MouseMotion { delta }, true, true) = (ev, self.looking || self.grabbed, self.free && !self.confirm_exit) {
            self.cam.yaw += delta.0 as f32 * 0.003;
            self.cam.pitch = (self.cam.pitch - delta.1 as f32 * 0.003).clamp(-1.5, 1.5);
        }
    }
}

/// The ground height under a game-space point: a ray straight down from 30 m above it, hitting the
/// original world or a fill piece (not those `skip`ped) [APPROX: the first surface below, a roof if the
/// point is under one].
fn ground_below(pk: &Picker, p: Vec3, skip: &[bool]) -> Option<f32> {
    let o = Vec3::new(p.x, p.y + 30.0, -p.z);
    let t = pk.ground_t(o, Vec3::NEG_Y, 600.0, &|i| skip.get(i).copied().unwrap_or(true))?;
    Some(o.y - t)
}

/// `--cine FILE` (cine.rs, re-notes/20-cine.md): render the camera sequence to a video (`--render`) or one frame (`--shot` + `--at-time`).
fn cine(a: &Args) -> Result<(), String> {
    use motorview::cine_render::{self, Out, RenderOpts};
    let arg = a.cine.as_deref().ok_or("--cine FILE")?;
    // a bare name is looked for in the install's remaster/cine folder
    let path = if arg.exists() { arg.to_path_buf() } else { a.root.join("remaster/cine").join(arg) };
    let text = std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    let (seq, warnings) = motorview::cine::parse(&text).map_err(|e| format!("{}: {e}", path.display()))?;
    for w in &warnings {
        eprintln!("{}: {w}", path.display());
    }
    // CLASSIC or REMASTER as in `--shot`; the AI racing line is never drawn
    let view = View { show_line: false, ..a.view() };
    let opts = RenderOpts { view, passes: a.hide, size: if a.size_set { a.size } else { seq.size }, fps: a.fps.unwrap_or(seq.fps), from: a.from.or(a.at_time).unwrap_or(0.0), to: a.to, codec: a.codec, watch: None, audio: !a.no_audio };
    let out = match (&a.render, &a.shot) {
        (Some(p), None) => Out::Video(p),
        (None, Some(p)) => {
            if a.at_time.is_none() {
                return Err("--cine with --shot needs --at-time S".into());
            }
            Out::Still(p)
        }
        _ => return Err("--cine FILE needs --render OUT, or --shot OUT --at-time S".into()),
    };
    cine_render::render(&a.root, &opts, &seq, out)
}

fn main() {
    if std::env::args().any(|a| a == "--version") {
        println!("motorview {}", motor_version::DISPLAY);
        return;
    }
    let args = match parse_args() {
        Ok(a) => a,
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(2);
        }
    };
    if let Some(d) = &args.cars_dir {
        motorview::procgen::bodyfile::set_dir(d.clone());
    }
    if args.fill_auto {
        // REMASTER world fill, automatic pass (procgen/fill.rs `auto`); keeps hand-edited files unless --force
        let tracks: Vec<u32> = if args.all_tracks { (1..=8).collect() } else { vec![args.track] };
        let mut failed = false;
        for t in tracks {
            let path = args.root.join(format!("remaster/fill/track{t:02}.fill"));
            if path.exists() && !args.force {
                println!("track {t}: {} exists (--force to overwrite)", path.display());
                continue;
            }
            let res = scene::load(&args.root, t, &[], &[]).map_err(|e| e.to_string()).and_then(|s| {
                let mut f = motorview::procgen::fill::auto(&s, args.fill_seed);
                // the water and grass lines are hand-made: they survive a regenerated fill
                let old = motorview::procgen::fill::load(&args.root, t);
                (f.water, f.grass, f.grass_none) = (old.water, old.grass, old.grass_none);
                if let Some(d) = path.parent() {
                    std::fs::create_dir_all(d).map_err(|e| e.to_string())?;
                }
                std::fs::write(&path, motorview::procgen::fill::to_text(&f)).map_err(|e| e.to_string())?;
                Ok(format!("track {t}: {} pieces -> {}", f.pieces.len(), path.display()))
            });
            match res {
                Ok(m) => println!("{m}"),
                Err(e) => {
                    eprintln!("track {t}: {e}");
                    failed = true;
                }
            }
        }
        std::process::exit(failed as i32);
    }
    if args.probes_stats {
        // REMASTER irradiance probes (probes.rs, #83): bake the road's bricks, print what it took
        let tracks: Vec<u32> = if args.all_tracks { (1..=8).collect() } else { vec![args.track] };
        let mut failed = false;
        for t in tracks {
            let res = scene::load(&args.root, t, &[], &[]).map_err(|e| e.to_string()).and_then(|s| {
                let w = motor_sim::world::World::load(&args.root, t)?;
                // the sun the GPU volume bakes for in Remaster (#198)
                let sun = motorview::render::view_look(&View { mode: RenderMode::Remaster, ..args.view() }, &s).sun_dir;
                let v = motorview::probes::ProbeVolume::bake_scene(&s, &w, glam::Vec3::from(sun));
                // the most enclosed probe (least sky overhead), game coordinates: a place to aim a shot at
                let dark = v.points(|_| true).filter(|(_, p)| p.valid()).min_by(|a, b| a.1.eval(glam::Vec3::Y).0.total_cmp(&b.1.eval(glam::Vec3::Y).0));
                let dark = dark.map_or(String::new(), |(p, pr)| format!("; least sky {:.2} at {:.1},{:.1},{:.1}", pr.eval(glam::Vec3::Y).0, p.x, p.y, -p.z));
                Ok(format!("track {t}: {} bricks, sun {:.2},{:.2},{:.2} (MYW {:.2},{:.2},{:.2}), {}{dark}", v.bricks.len(), sun[0], sun[1], sun[2], s.look.sun_dir[0], s.look.sun_dir[1], s.look.sun_dir[2], v.stats))
            });
            match res {
                Ok(m) => println!("{m}"),
                Err(e) => {
                    eprintln!("track {t}: {e}");
                    failed = true;
                }
            }
        }
        std::process::exit(failed as i32);
    }
    if args.cine.is_some() && !args.cine_edit {
        if let Err(e) = cine(&args) {
            eprintln!("{e}");
            std::process::exit(1);
        }
        return;
    }
    if args.shot.is_some() || args.car_sheet.is_some() {
        if let Err(e) = shot(&args, args.shot.as_deref()) {
            eprintln!("{e}");
            std::process::exit(1);
        }
        return;
    }
    let view = args.view();
    let el = EventLoop::new().expect("event loop");
    let mut dbg = DebugState::new(args.layers, args.panel);
    // the race is paused in free camera (P runs it)
    dbg.paused = args.free;
    let swap_mesh = args.swap_mesh;
    let mut app = App {
        track: args.track,
        car: args.car,
        cars: drive::lineup(args.car),
        free: args.free,
        drive_free: args.drive_free,
        args,
        window: None,
        surface: None,
        config: None,
        gpu: None,
        depth: None,
        sess: None,
        cam: Camera::looking(Vec3::ZERO, Vec3::NEG_Z),
        start: (Vec3::ZERO, Vec3::NEG_Z),
        view,
        keys: HashSet::new(),
        steer_keys: SteerKeys::default(),
        looking: false,
        look_at: None,
        last: Instant::now(),
        title_at: Instant::now(),
        assets: None,
        picker: None,
        overlay: false,
        wireframe: WireStyle::Off,
        mirror: false,
        cross: true,
        grabbed: false,
        confirm_exit: false,
        exit_now: false,
        ov: None,
        fps: (0.0, 0, Instant::now()),
        next_frame: Instant::now(),
        speed: 1.0,
        dbg,
        screenshot: false,
        clean: false,
        clipboard: None,
        note: None,
        scene: None,
        preview_slot: None,
        preview: None,
        tune: Tune::default(),
        ribbon: RibbonState::default(),
        inst: None,
        adapter: None,
        ghost: None,
        ghost_win: None,
        ghost_open: false,
        ghost_cam: CamMode::Chase,
        ghost_follow: Follow::Ghost,
        scrub: 0,
        seek: None,
        ghost_path: String::new(),
        ghost_msg: String::new(),
        ghost_files: None,
        ghost_load: None,
        pane_w: 0.0,
        left_w: 0.0,
        status_h: 0.0,
        gui: None,
        menu_bar: true,
        bar_h: 0.0,
        pending: Vec::new(),
        mods: 0,
        world: WorldEdit::default(),
        cursor: (0.0, 0.0),
        last_mesh: Instant::now(),
        body_watch: Vec::new(),
        body_checked: Instant::now(),
        last_water: Vec::new(),
        paints: HashMap::new(),
        mesh_swap: swap_mesh,
        repaint_due: false,
        local_status: "",
        cine: CineEdit::default(),
    };
    el.run_app(&mut app).expect("event loop");
}

#[cfg(test)]
mod tests {
    use super::*;

    /// No two bindings may fire for the same key + modifiers in overlapping contexts.
    #[test]
    fn binds_do_not_conflict() {
        for (i, a) in BINDS.iter().enumerate() {
            for b in &BINDS[i + 1..] {
                // a World bind deliberately shadows the same key in the other contexts
                let overlap = a.ctx == b.ctx || (a.ctx != Ctx::World && b.ctx != Ctx::World && (a.ctx == Ctx::Any || b.ctx == Ctx::Any));
                let shared = a.keys.iter().any(|k| b.keys.contains(k));
                assert!(!(overlap && shared && a.mods == b.mods), "conflict: {} and {}", a.name, b.name);
            }
        }
    }
}
