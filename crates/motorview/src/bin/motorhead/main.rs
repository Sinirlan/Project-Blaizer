//! motorhead — the game: front end (main menu, car and track selection, options), races
//! with the original HUD, pause and results. Reads a user-supplied install.
//!
//!   motorhead <installed_game> [--demo]
//!   motorhead <installed_game> --shot main|single|track|car|options|graphics|sound|controls|personal|statistics|credits|loading|race|pause|pausegfx|pausesfx|pausehud|results|rankings|league|leaguetable|leagueload|leagueresults|leaguemovie out.png
//!             [--size WxH] [--car N] [--track N] [--sim-seconds S] [--frame-hz HZ]
//!
//! Menus: arrows move, Enter selects, Esc goes back (a gamepad's d-pad, A and B too).
//! Race: the control layout's keys (Keyboard.clo: arrows drive, A/S gears, Space horn,
//! Shift handbrake, Ctrl look back, F1 in-car, F2 chase, F4 switch car, F5 rear mirror),
//! plus Respawn and Change camera (our tasks, R and V by default), Esc pause. F9 switches Classic / Remaster graphics, F8 / Pause freezes the race (comparison shots), F11
//! fullscreen, F12 saves a screenshot (menus too) to the settings folder. `--demo` lets the
//! AI drive your car; `--mirror` turns the rear mirror on; `--remaster` shoots in Remaster mode.
//! `--shot loading --menu-style modern --load-progress P` draws the Remaster loading screen's bar at P (1 = READY).

mod audio;
mod cheats;
mod controls;
mod demo;
mod front;
mod hiscore;
mod hud;
mod hud_glass;
mod hud_layout;
mod hud_transport;
mod league;
mod menu_fx;
mod movie;
mod playback;
mod proximity;
mod rumble;
mod slide_fx;
mod stats;

use front::{Display, Game, Screen};
use motorview::render::{parse_aa_strength, CarDetail, CarShading, Gpu, GrassLevel, LightShadows, PostAa, PostAaKind, RenderAa, RenderAaKind, RemasterView, RenderMode, ShadowQuality, Tonemap};
use motorview::{device, instance, png};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};
use winit::application::ApplicationHandler;
use winit::event::{DeviceEvent, DeviceId, ElementState, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{KeyCode, PhysicalKey};
use winit::monitor::MonitorHandle;
use winit::window::{Fullscreen, Window, WindowId};

/// `KIND[:P]` of `--aa-render` / `--aa-post`: the method and an optional strength in percent.
fn parse_aa<K>(v: &str, kind: fn(&str) -> Option<K>) -> Result<(K, Option<u32>), String> {
    let (k, p) = v.split_once(':').map_or((v, None), |(k, p)| (k, Some(p)));
    let strength = p.map(|p| parse_aa_strength(p).ok_or_else(|| format!("bad anti-aliasing strength {p:?} (2x, 4x, 8x, 16x or a percent)"))).transpose()?;
    Ok((kind(k).ok_or_else(|| format!("unknown anti-aliasing method {k:?}"))?, strength))
}

const USAGE: &str = "usage: motorhead [installed_game] [--demo] [--shot SCREEN out.png [--size WxH] [--attract] [--idle S] [--knockout] [--laps N] [--time-attack [--ghost run.ghl|run.mhr] [--save-ghost out.ghl --autopilot]] [--threaded-load [--cancel-load] [--cancel-upload]] [--league] [--league-div 0-3] [--locked none|TRACK,TRACK] [--league-name TEXT] [--season-end 1-7] [--stats FILE [--stats-page 0-3]] [--hiscores DIR] [--player-name NAME] [--team TEAM] [--sound-devices NAME;NAME;.. [--sound-device NAME]] [--leagues DIR] [--demos DIR] [--compare A,B [--car-a C] [--car-b C] [--lap-a N] [--lap-b N]] [--analysis-overlay working|drop|keep] [--analysis-drop FILE] [--car N] [--track N] [--sim-seconds S] [--place X,Z[,HEADING[,FROM_Y]]] [--club-seen] [--frame-hz HZ] [--mirror] [--cheat casper,gulliver] [--free-look X,Y] [--car-shading flat|gouraud|reflection] [--car-detail low|medium|high] [--remaster [--msaa 1|2|4|8] [--aa-render off|msaa|ssaa[:2x|4x|8x|16x]] [--aa-post off|fxaa|smaa|taa[:2x|4x|8x|16x]] [--colour-grade] [--no-hdr] [--tonemap neutral|aces|agx] [--exposure P] [--bloom P] [--ssao P] [--halos classic|smooth] [--flares classic|smooth] [--hud classic|modern] [--rev-meter G,I,A,S,B,T,D,R] [--hud-scale P] [--hud-margin P] [--hud-map circuit|close[,rotate]] [--hud-hide cluster,clocks,lap,pos,map,proximity,numbers,frame,delta] [--hud-show live|sector|sector-slow] [--name-plates off|flat|see-through] [--shadows classic|real] [--blur classic|velocity] [--skids off|on|detailed] [--car-models original|generated]] [--motion-blur] [--view-distance PERCENT] [--no-fill] [--no-intro] [--trees on|off] [--water on|off] [--water-reflect sky|scene] [--local-light on|off] [--texture-pack on|off] [--grass off|short|full|ultra] [--grass-range P] [--car-lod-distance P] [--fog-strength P] [--car-colours R,G,B/R,G,B/R,G,B] [--car-finish gloss|metallic|pearl[,F,F]]]";

struct Args {
    root: PathBuf,
    shot: Option<(Screen, PathBuf)>,
    size: (u32, u32),
    car: usize,
    track: usize,
    sim_seconds: f32,
    /// `--place X,Z[,HEADING[,FROM_Y]]`: stand the followed car there before the shot's frames (dev tool for #366)
    place: Option<[f32; 4]>,
    /// `--club-seen`: the Credits screen shows the hidden page, as after the RuhrStadt club message
    club_seen: bool,
    /// race shots: the frame rate the race is stepped at (60; #374: other rates draw between physics ticks)
    frame_hz: f32,
    demo: bool,
    /// `--attract`: race shots are the attract demo (#210); `--idle S`: a menu shot after S seconds of the idle timer
    attract: bool,
    track_given: bool,
    idle: Option<f32>,
    /// race shots: play this recording (.mhr) back / drive with the up key held and save the
    /// recording here
    replay: Option<PathBuf>,
    playback: Option<String>,
    save_replay: Option<PathBuf>,
    ghost_demo: Option<PathBuf>,
    save_demo: Option<PathBuf>,
    ghost: Option<PathBuf>,
    save_ghost: Option<PathBuf>,
    autopilot: bool,
    /// DEBUG: start the race through the loading screen and its worker thread (#335), as the window does
    threaded_load: bool,
    /// DEBUG with `--threaded-load`: press Esc once the worker runs, then load again
    cancel_load: bool,
    /// `--cancel-upload`: Esc once the GPU upload has started (#335)
    cancel_upload: bool,
    /// race in Time Attack (alone, with the record gaps)
    time_attack: bool,
    /// `--hud-show sector|sector-slow`: seed a sample sector delta after the race's frames (shots only, #409)
    sector_sample: Option<bool>,
    knockout: bool,
    /// --laps N: the shot's race length (default: the track's)
    laps: Option<u32>,
    /// race shots: the current league's next race (a new league when headless)
    league: bool,
    /// league shots: you start in this division (0 = the final) instead of Division 3
    league_div: Option<usize>,
    /// simulate the unlock flags: `none` or a comma list of the flagged track names
    locked: Option<String>,
    /// league screen shots: the New league name entry with this text typed
    league_name: Option<String>,
    /// league shots: the season ends with this code 1..7 (results screen, `leaguemovie` plays its movie)
    season_end: Option<u32>,
    stats: Option<PathBuf>,
    stats_page: usize,
    /// rear mirror on
    mirror: bool,
    difficulty: Option<motor_sim::Difficulty>,
    /// MotionBlur on (the race shot renders its last frames to build the history)
    motion_blur: bool,
    car_shading: Option<CarShading>,
    car_detail: Option<CarDetail>,
    /// draw in the Remaster graphics mode
    remaster: bool,
    /// REMASTER shot options: MSAA samples, post pass on (50 %), view distance in percent
    msaa: Option<u32>,
    /// `--aa-render KIND[:P]`, `--aa-post KIND[:P]`: method and optional strength
    render_aa: Option<(RenderAaKind, Option<u32>)>,
    post_aa: Option<(PostAaKind, Option<u32>)>,
    grade: bool,
    no_hdr: bool,
    tonemap: Option<Tonemap>,
    smooth_halos: Option<bool>,
    fog_style: Option<motorview::render::FogStyle>,
    /// `--fog-tune key=value,...`: the keys of `remaster/fog/trackNN.fog` over the track's own values
    fog_tune: Option<String>,
    /// --sky classic|hdri, --sky-exposure EV, --sky-yaw DEG (09-remaster 9.32)
    sky_hdri: Option<bool>,
    sky_exposure: Option<f32>,
    sky_yaw: Option<f32>,
    no_sky_sun: bool,
    no_sky_ambient: bool,
    sky_ambient_tint: Option<f32>,
    smooth_flares: Option<bool>,
    retro_signs: Option<bool>,
    car_shadows: Option<bool>,
    velocity_blur: Option<bool>,
    /// --skids off|on|detailed: (marks drawn, detailed)
    skids: Option<(bool, bool)>,
    ssao: Option<u32>,
    generated_cars: Option<bool>,
    no_fill: bool,
    /// --no-intro: start at the main menu
    no_intro: bool,
    /// --shot: start the garage scene's background build on the way to the Car screen (the window's behaviour)
    prefetch: bool,
    light_shadows: Option<LightShadows>,
    /// `--grass`: REMASTER grass level
    grass: Option<GrassLevel>,
    /// `--grass-range`: REMASTER grass range, percent
    grass_range: Option<u32>,
    /// `--car-lod-distance`: REMASTER car detail distance, percent
    car_lod_distance: Option<u32>,
    /// `--fog-strength`: REMASTER fog strength, percent
    fog_strength: Option<u32>,
    /// `--water-reflect`: REMASTER water reflections, true = the scene (SSR), false = the sky only
    water_ssr: Option<bool>,
    local_light: Option<bool>,
    texture_pack: Option<bool>,
    /// `--water`: REMASTER water surfaces on/off (off: the original water textures)
    water: Option<bool>,
    /// `--trees`: REMASTER tree models on/off (off: the original tree cards)
    trees: Option<bool>,
    shadow_quality: Option<ShadowQuality>,
    /// --bench N: after the shot, time N more frames of the same view
    bench: usize,
    /// --bench-move: step the race between the bench frames (camera and car motion for the blur)
    bench_move: bool,
    modern_hud: Option<bool>,
    /// NamePlates for the shot: 0 off, 1 flat, 2 see-through
    name_plates: Option<usize>,
    modern_menu: Option<bool>,
    menu_stretch: Option<bool>,
    /// REMASTER HUD Settings for the shot (--hud-scale/--hud-margin/--hud-map/--hud-hide)
    hud: hud_layout::HudSettings,
    exposure: Option<u32>,
    bloom: Option<u32>,
    view_distance: Option<f32>,
    /// CustomCarColours on with these CarColour1..3
    car_colours: Option<[[u8; 3]; 3]>,
    /// REMASTER CarFinish1..3 (#135): one value for all three paints, or three
    car_finish: Option<[front::PaintFinish; 3]>,
    /// fixed free-look stick during the race shot
    free_look: Option<glam::Vec2>,
    record_audio: Option<PathBuf>,
    /// --engine-sound classic|simulated (with --record-audio)
    engine_sound: bool,
    /// seconds of menu animation before a menu screenshot
    menu_time: f32,
    /// `--leave-time S`: press `--keys` after the menu has settled and run S more seconds (a Classic screen sliding out)
    leave_time: Option<f32>,
    /// REMASTER pause menu shots: the Restart / Exit hold's fill (0..1)
    pause_hold: Option<f32>,
    /// REMASTER loading screen shots (#90): the bar's share (0..1, 1 = READY)
    load_progress: Option<f32>,
    /// --mode-dissolve P: the shot is the F9 dissolve P (0..1) of the way through
    mode_dissolve: Option<f32>,
    /// open the exit requester before the screenshot
    confirm_exit: bool,
    save_requester: Option<String>,
    cheats: Option<String>,
    keep_display: bool,
    /// REMASTER: navigation glyph set of the shot (keyboard, xbox, playstation)
    glyphs: Option<controls::Device>,
    /// REMASTER: write a sample sheet of the modern HUD fonts (9.21) instead of a shot
    font_specimen: Option<PathBuf>,
    /// REMASTER: write the glass rev meter at three fills (9.21 phase 4) instead of a shot
    rev_meter_sheet: Option<PathBuf>,
    /// REMASTER: the rev meter's tune (--rev-meter, 8 percentages as in RevMeterTune)
    rev_meter: Option<motorview::revmeter::RevMeterTune>,
    /// keys pressed before the screenshot (left,right,up,down,enter,esc)
    keys: Vec<KeyCode>,
    /// text typed into the field a `--keys` Enter opened (a name, a team)
    type_text: Option<String>,
    /// `.MHI` tables of the Rankings shot read from this folder (never written), and the name that counts as you
    hiscores: Option<PathBuf>,
    leagues: Option<PathBuf>,
    demos: Option<PathBuf>,
    /// `--shot analysis`: the two laps (`--compare A,B`) with each side's car and lap (#429)
    compare: Option<[PathBuf; 2]>,
    compare_car: [Option<String>; 2],
    compare_lap: [Option<u32>; 2],
    /// `--analysis-overlay working|drop|keep`: the working overlay on the selected side, the drop overlay, or side B kept from outside
    analysis_overlay: Option<String>,
    /// `--analysis-drop FILE`: that file dropped on the screen before the shot (the drop path of #429 phase 4)
    analysis_drop: Option<PathBuf>,
    player_name: Option<String>,
    team: Option<String>,
    /// the Sound screen's output list (`;` between names) instead of the machine's, and the output picked in it (#260)
    sound_devices: Option<Vec<String>>,
    sound_device: Option<String>,
}

fn parse_args() -> Result<Args, String> {
    let mut it = std::env::args().skip(1);
    let mut a = Args { root: PathBuf::new(), shot: None, size: (1280, 800), car: 1, track: 1, sim_seconds: 20.0, place: None, club_seen: false, frame_hz: 60.0, demo: false, attract: false, track_given: false, idle: None, replay: None, playback: None, save_replay: None, ghost_demo: None, save_demo: None, ghost: None, save_ghost: None, autopilot: false, threaded_load: false, cancel_load: false, cancel_upload: false, time_attack: false, sector_sample: None, knockout: false, laps: None, league: false, league_div: None, locked: None, league_name: None, season_end: None, stats: None, stats_page: 0, mirror: false, difficulty: None, motion_blur: false, car_shading: None, car_detail: None, remaster: false, msaa: None, render_aa: None, post_aa: None, grade: false, no_hdr: false, tonemap: None, smooth_halos: None, fog_style: None, fog_tune: None, sky_hdri: None, sky_exposure: None, sky_yaw: None, no_sky_sun: false, no_sky_ambient: false, sky_ambient_tint: None, smooth_flares: None, retro_signs: None, car_shadows: None, velocity_blur: None, skids: None, ssao: None, generated_cars: None, no_fill: false, no_intro: false, prefetch: false, light_shadows: None, grass: None, grass_range: None, car_lod_distance: None, fog_strength: None, water_ssr: None, local_light: None, texture_pack: None, water: None, trees: None, shadow_quality: None, bench: 0, bench_move: false, modern_hud: None, name_plates: None, modern_menu: None, menu_stretch: None, hud: Default::default(), exposure: None, bloom: None, view_distance: None, car_colours: None, car_finish: None, free_look: None, record_audio: None, engine_sound: false, menu_time: 0.5, leave_time: None, pause_hold: None, load_progress: None, mode_dissolve: None, confirm_exit: false, save_requester: None, cheats: None, keep_display: false, glyphs: None, font_specimen: None, rev_meter_sheet: None, rev_meter: None, keys: Vec::new(), type_text: None, hiscores: None, leagues: None, demos: None, compare: None, compare_car: [None, None], compare_lap: [None, None], analysis_overlay: None, analysis_drop: None, player_name: None, team: None, sound_devices: None, sound_device: None };
    let mut root = None;
    while let Some(x) = it.next() {
        let mut val = |n: &str| it.next().ok_or(format!("{n} needs a value"));
        match x.as_str() {
            "--shot" => {
                let s = val("--shot")?;
                let screen = Screen::parse(&s).ok_or(format!("unknown screen {s}"))?;
                a.shot = Some((screen, val("--shot")?.into()));
            }
            "--size" => {
                let v = val("--size")?;
                let (w, h) = v.split_once('x').ok_or("--size WxH")?;
                a.size = (w.parse().map_err(|_| "bad width")?, h.parse().map_err(|_| "bad height")?);
            }
            "--car" => a.car = val("--car")?.parse().map_err(|_| "bad --car")?,
            "--track" => { a.track = val("--track")?.parse().map_err(|_| "bad --track")?; a.track_given = true; }
            "--place" => {
                let v: Vec<f32> = val("--place")?.split(',').map(|s| s.trim().parse()).collect::<Result<_, _>>().map_err(|_| "bad --place X,Z[,HEADING[,FROM_Y]]")?;
                if !(2..=4).contains(&v.len()) {
                    return Err("bad --place X,Z[,HEADING[,FROM_Y]]".into());
                }
                a.place = Some([v[0], v[1], v.get(2).copied().unwrap_or(0.0), v.get(3).copied().unwrap_or(100.0)]);
            }
            "--club-seen" => a.club_seen = true,
            "--sim-seconds" => a.sim_seconds = val("--sim-seconds")?.parse().map_err(|_| "bad --sim-seconds")?,
            "--frame-hz" => a.frame_hz = val("--frame-hz")?.parse::<f32>().ok().filter(|&h| (1.0..=1000.0).contains(&h)).ok_or("bad --frame-hz")?,
            "--demo" => a.demo = true,
            "--attract" => a.attract = true,
            "--idle" => a.idle = Some(val("--idle")?.parse().map_err(|_| "bad --idle")?),
            "--replay" => a.replay = Some(val("--replay")?.into()),
            "--playback" => a.playback = Some(val("--playback")?),
            "--save-replay" => a.save_replay = Some(val("--save-replay")?.into()),
            "--ghost-demo" => a.ghost_demo = Some(val("--ghost-demo")?.into()),
            "--save-demo" => a.save_demo = Some(val("--save-demo")?.into()),
            "--ghost" => a.ghost = Some(val("--ghost")?.into()),
            "--save-ghost" => a.save_ghost = Some(val("--save-ghost")?.into()),
            "--autopilot" => a.autopilot = true,
            "--threaded-load" => a.threaded_load = true,
            "--cancel-load" => a.cancel_load = true,
            "--cancel-upload" => a.cancel_upload = true,
            "--time-attack" => a.time_attack = true,
            "--knockout" => a.knockout = true,
            "--laps" => a.laps = Some(val("--laps")?.parse().ok().filter(|n| (1..=99).contains(n)).ok_or("--laps needs 1-99")?),
            "--league" => a.league = true,
            "--league-div" => a.league_div = Some(val("--league-div")?.parse().ok().filter(|d| *d <= 3).ok_or("--league-div needs 0-3")?),
            "--league-name" => a.league_name = Some(val("--league-name")?),
            "--season-end" => a.season_end = Some(val("--season-end")?.parse().ok().filter(|c| (1..=7).contains(c)).ok_or("--season-end needs 1-7")?),
            "--stats" => a.stats = Some(PathBuf::from(val("--stats")?)),
            "--stats-page" => a.stats_page = val("--stats-page")?.parse().map_err(|_| "--stats-page needs 0-3")?,
            "--locked" => a.locked = Some(val("--locked")?),
            "--mirror" => a.mirror = true,
            "--difficulty" => {
                use motor_sim::Difficulty::*;
                a.difficulty = Some(match val("--difficulty")?.as_str() {
                    "easy" => Easy,
                    "medium" => Medium,
                    "hard" => Hard,
                    "beginner" => Beginner,
                    _ => return Err("--difficulty easy|medium|hard|beginner".into()),
                });
            }
            "--motion-blur" => a.motion_blur = true,
            "--car-detail" => {
                let v = val("--car-detail")?;
                a.car_detail = Some(CarDetail::parse(&v).ok_or(format!("unknown car detail {v}"))?);
            }
            "--car-shading" => {
                let v = val("--car-shading")?;
                a.car_shading = Some(CarShading::parse(&v).ok_or(format!("unknown car shading {v}"))?);
            }
            "--remaster" => a.remaster = true,
            "--no-msaa" => a.msaa = Some(1),
            "--aa-render" => a.render_aa = Some(parse_aa(&val("--aa-render")?, RenderAaKind::parse)?),
            "--aa-post" => a.post_aa = Some(parse_aa(&val("--aa-post")?, PostAaKind::parse)?),
            "--msaa" => a.msaa = Some(val("--msaa")?.parse().map_err(|e: std::num::ParseIntError| e.to_string())?),
            "--colour-grade" => a.grade = true,
            "--no-hdr" => a.no_hdr = true,
            "--tonemap" => {
                let v = val("--tonemap")?;
                a.tonemap = Some(Tonemap::parse(&v).ok_or(format!("unknown tonemapper {v}"))?);
            }
            "--exposure" => a.exposure = Some(val("--exposure")?.parse().map_err(|e: std::num::ParseIntError| e.to_string())?),
            "--fog" => {
                let v = val("--fog")?;
                a.fog_style = Some(motorview::render::FogStyle::parse(&v).ok_or(format!("unknown fog style {v}"))?);
            }
            "--fog-tune" => {
                let v = val("--fog-tune")?;
                if !v.contains('=') {
                    return Err(format!("--fog-tune takes key=value pairs (start=20,full=120,density=0.5,colour=sky; the keys of remaster/fog/trackNN.fog), got {v}"));
                }
                a.fog_tune = Some(v);
            }
            "--sky" => {
                let v = val("--sky")?;
                a.sky_hdri = Some(match v.to_ascii_lowercase().as_str() {
                    "classic" | "band" => false,
                    "hdri" | "dome" => true,
                    _ => return Err(format!("unknown sky {v} (classic or hdri)")),
                });
            }
            "--sky-exposure" => a.sky_exposure = Some(val("--sky-exposure")?.parse().map_err(|e: std::num::ParseFloatError| e.to_string())?),
            "--no-sky-sun" => a.no_sky_sun = true,
            "--no-sky-ambient" => a.no_sky_ambient = true,
            "--sky-ambient-tint" => a.sky_ambient_tint = Some(val("--sky-ambient-tint")?.parse().map_err(|e: std::num::ParseFloatError| e.to_string())?),
            "--sky-yaw" => a.sky_yaw = Some(val("--sky-yaw")?.parse().map_err(|e: std::num::ParseFloatError| e.to_string())?),
            "--halos" => {
                let v = val("--halos")?;
                a.smooth_halos = Some(match v.to_ascii_lowercase().as_str() {
                    "classic" => false,
                    "smooth" => true,
                    _ => return Err(format!("unknown halo style {v}")),
                });
            }
            "--flares" => {
                let v = val("--flares")?;
                a.smooth_flares = Some(match v.to_ascii_lowercase().as_str() {
                    "classic" => false,
                    "smooth" => true,
                    _ => return Err(format!("unknown flare style {v}")),
                });
            }
            "--signs" => {
                let v = val("--signs")?;
                a.retro_signs = Some(match v.to_ascii_lowercase().as_str() {
                    "on" => true,
                    "off" => false,
                    _ => return Err(format!("unknown signs value {v}")),
                });
            }
            "--shadows" => {
                let v = val("--shadows")?;
                a.car_shadows = Some(match v.to_ascii_lowercase().as_str() {
                    "classic" => false,
                    "real" => true,
                    _ => return Err(format!("unknown shadow style {v}")),
                });
            }
            "--blur" => {
                let v = val("--blur")?;
                a.velocity_blur = Some(match v.to_ascii_lowercase().as_str() {
                    "classic" => false,
                    "velocity" => true,
                    _ => return Err(format!("unknown motion blur style {v}")),
                });
            }
            "--skids" => {
                let v = val("--skids")?;
                a.skids = Some(match v.to_ascii_lowercase().as_str() {
                    "off" => (false, false),
                    "on" | "classic" => (true, false),
                    "detailed" => (true, true),
                    _ => return Err(format!("unknown skid marks {v}")),
                });
            }
            "--light-shadows" => {
                let v = val("--light-shadows")?;
                a.light_shadows = Some(LightShadows::parse(&v).ok_or(format!("unknown light shadows {v}"))?);
            }
            "--shadow-quality" => {
                let v = val("--shadow-quality")?;
                a.shadow_quality = Some(ShadowQuality::parse(&v).ok_or(format!("unknown shadow quality {v} (low|medium|high|ultra)"))?);
            }
            "--bench-move" => a.bench_move = true,
            "--bench" => a.bench = val("--bench")?.parse().map_err(|e: std::num::ParseIntError| e.to_string())?,
            "--no-fill" => a.no_fill = true,
            "--no-intro" => a.no_intro = true,
            "--prefetch" => a.prefetch = true,
            "--grass" => {
                let v = val("--grass")?;
                a.grass = Some(GrassLevel::parse(&v).ok_or(format!("unknown grass level {v} (off|short|full|ultra)"))?);
            }
            "--grass-range" => {
                let (lo, hi) = RemasterView::GRASS_RANGE;
                a.grass_range = Some(val("--grass-range")?.parse::<u32>().map_err(|e| e.to_string())?.clamp(lo, hi));
            }
            "--car-lod-distance" => {
                let (lo, hi) = RemasterView::CAR_LOD_DISTANCE;
                a.car_lod_distance = Some(val("--car-lod-distance")?.parse::<u32>().map_err(|e| e.to_string())?.clamp(lo, hi));
            }
            "--fog-strength" => {
                let (lo, hi) = RemasterView::FOG_STRENGTH;
                a.fog_strength = Some(val("--fog-strength")?.parse::<u32>().map_err(|e| e.to_string())?.clamp(lo, hi));
            }
            "--trees" => {
                let v = val("--trees")?;
                a.trees = Some(match v.to_ascii_lowercase().as_str() {
                    "on" => true,
                    "off" => false,
                    _ => return Err(format!("unknown trees {v} (on|off)")),
                });
            }
            "--water" => {
                let v = val("--water")?;
                a.water = Some(match v.to_ascii_lowercase().as_str() {
                    "on" => true,
                    "off" => false,
                    _ => return Err(format!("unknown water {v} (on|off)")),
                });
            }
            "--texture-pack" => {
                let v = val("--texture-pack")?;
                a.texture_pack = Some(match v.to_ascii_lowercase().as_str() {
                    "on" => true,
                    "off" => false,
                    _ => return Err(format!("unknown texture pack {v} (on|off)")),
                });
            }
            "--local-light" => {
                let v = val("--local-light")?;
                a.local_light = Some(match v.to_ascii_lowercase().as_str() {
                    "on" => true,
                    "off" => false,
                    _ => return Err(format!("unknown local light {v} (on|off)")),
                });
            }
            "--water-reflect" => {
                let v = val("--water-reflect")?;
                a.water_ssr = Some(match v.to_ascii_lowercase().as_str() {
                    "sky" => false,
                    "scene" => true,
                    _ => return Err(format!("unknown water reflections {v} (sky|scene)")),
                });
            }
            "--car-models" => {
                let v = val("--car-models")?;
                a.generated_cars = Some(match v.to_ascii_lowercase().as_str() {
                    "original" => false,
                    "generated" => true,
                    _ => return Err(format!("unknown car models {v}")),
                });
            }
            "--font-specimen" => a.font_specimen = Some(val("--font-specimen")?.into()),
            "--rev-meter-sheet" => a.rev_meter_sheet = Some(val("--rev-meter-sheet")?.into()),
            "--rev-meter" => {
                let v: Vec<u32> = val("--rev-meter")?.split(',').map(|x| x.trim().parse::<u32>().map_err(|e| e.to_string())).collect::<Result<_, _>>()?;
                let v = <[u32; motorview::revmeter::RevMeterTune::FIELDS]>::try_from(v.as_slice()).map_err(|_| "--rev-meter wants 8 values: glow,inner,alpha,spec,bloom,thickness,depth,roundness".to_string())?;
                a.rev_meter = Some(motorview::revmeter::RevMeterTune::from_array(v));
            }
            "--menu-style" => {
                let v = val("--menu-style")?;
                a.modern_menu = Some(match v.to_ascii_lowercase().as_str() {
                    "classic" => false,
                    "modern" => true,
                    _ => return Err(format!("unknown menu style {v}")),
                });
            }
            "--menu-picture" => {
                let v = val("--menu-picture")?;
                a.menu_stretch = Some(match v.to_ascii_lowercase().as_str() {
                    "stretch" => true,
                    "keep" => false,
                    _ => return Err(format!("unknown menu picture {v}")),
                });
            }
            "--hud" => {
                let v = val("--hud")?;
                a.modern_hud = Some(match v.to_ascii_lowercase().as_str() {
                    "classic" => false,
                    "modern" => true,
                    _ => return Err(format!("unknown HUD style {v}")),
                });
            }
            "--name-plates" => {
                let v = val("--name-plates")?;
                a.name_plates = Some(match v.to_ascii_lowercase().as_str() {
                    "off" | "none" => 0,
                    "flat" => 1,
                    "see-through" | "transparent" => 2,
                    _ => return Err(format!("unknown name plates {v}")),
                });
            }
            "--hud-scale" => a.hud.scale = val("--hud-scale")?.parse().map_err(|e: std::num::ParseIntError| e.to_string())?,
            "--hud-margin" => a.hud.margin = val("--hud-margin")?.parse::<u32>().map_err(|e| e.to_string())?.min(100),
            "--hud-map" => {
                for m in val("--hud-map")?.split(',') {
                    match m.to_ascii_lowercase().as_str() {
                        "circuit" => a.hud.map_mode = hud_layout::MapMode::Circuit,
                        "close" => a.hud.map_mode = hud_layout::MapMode::Close,
                        "rotate" => a.hud.map_rotate = true,
                        _ => return Err(format!("unknown --hud-map value {m}")),
                    }
                }
            }
            "--hud-hide" => {
                for e in val("--hud-hide")?.split(',') {
                    let h = &mut a.hud;
                    match e.to_ascii_lowercase().as_str() {
                        "cluster" => h.cluster = false,
                        "clocks" => h.clocks = [false; 3],
                        "lap" => h.lap = false,
                        "pos" => h.pos = false,
                        "map" => h.map = false,
                        "proximity" => h.proximity = false,
                        "numbers" => h.map_numbers = false,
                        "frame" => h.plate_frame = false,
                        "delta" => h.sector_delay = 0,
                        _ => return Err(format!("unknown --hud-hide element {e}")),
                    }
                }
            }
            "--hud-show" => {
                for e in val("--hud-show")?.split(',') {
                    match e.to_ascii_lowercase().as_str() {
                        "live" => a.hud.live_gaps = true,
                        "sector" => a.sector_sample = Some(false),
                        "sector-slow" => a.sector_sample = Some(true),
                        _ => return Err(format!("unknown --hud-show element {e}")),
                    }
                }
            }
            "--ssao" => a.ssao = Some(val("--ssao")?.parse().map_err(|e: std::num::ParseIntError| e.to_string())?),
            "--bloom" => a.bloom = Some(val("--bloom")?.parse().map_err(|e: std::num::ParseIntError| e.to_string())?),
            "--view-distance" => a.view_distance = Some(val("--view-distance")?.parse::<f32>().map_err(|e| e.to_string())? / 100.0),
            "--car-colours" => {
                let v = val("--car-colours")?;
                let c: Vec<[u8; 3]> = v
                    .split('/')
                    .map(|c| {
                        let n: Vec<u8> = c.split(',').filter_map(|x| x.trim().parse().ok()).collect();
                        n.try_into().map_err(|_| "--car-colours R,G,B/R,G,B/R,G,B")
                    })
                    .collect::<Result<_, _>>()?;
                a.car_colours = Some(c.try_into().map_err(|_| "--car-colours needs 3 colours")?);
            }
            "--car-finish" => {
                let v = val("--car-finish")?;
                let f: Vec<front::PaintFinish> = v.split(',').map(|x| front::PaintFinish::from_key(x.trim()).ok_or("--car-finish gloss|metallic|pearl[,F,F]")).collect::<Result<_, _>>()?;
                a.car_finish = Some(match f[..] {
                    [one] => [one; 3],
                    [x, y, z] => [x, y, z],
                    _ => return Err("--car-finish needs 1 or 3 finishes".into()),
                });
            }
            "--free-look" => {
                let v = val("--free-look")?;
                let (x, y) = v.split_once(',').ok_or("--free-look X,Y")?;
                a.free_look = Some(glam::Vec2::new(x.trim().parse().map_err(|_| "bad --free-look")?, y.trim().parse().map_err(|_| "bad --free-look")?));
            }
            "--confirm-exit" => a.confirm_exit = true,
            "--save-requester" => a.save_requester = Some(val("--save-requester")?),
            "--cheat" => a.cheats = Some(val("--cheat")?),
            "--keep-display" => a.keep_display = true,
            "--glyphs" => {
                let v = val("--glyphs")?;
                a.glyphs = Some(controls::Device::parse(&v).ok_or_else(|| format!("unknown glyph set {v}"))?);
            }
            "--keys" => {
                for k in val("--keys")?.split(',') {
                    a.keys.push(match k {
                        "left" => KeyCode::ArrowLeft,
                        "right" => KeyCode::ArrowRight,
                        "up" => KeyCode::ArrowUp,
                        "down" => KeyCode::ArrowDown,
                        "enter" => KeyCode::Enter,
                        "esc" => KeyCode::Escape,
                        "tabl" => KeyCode::PageUp,
                        "tabr" => KeyCode::PageDown,
                        // the Remaster Rankings' difficulty keys (Z / C, LT / RT)
                        "difl" => KeyCode::KeyZ,
                        "difr" => KeyCode::KeyC,
                        "apply" => KeyCode::Space,
                        "reset" => KeyCode::KeyR,
                        // the Results screen's WATCH key
                        "watch" => KeyCode::KeyX,
                        "delete" => KeyCode::Delete,
                        "backspace" => KeyCode::Backspace,
                        // the pad's X and Y (the pad keyboard's delete and space)
                        "padx" => KeyCode::F6,
                        "pady" => KeyCode::F5,
                        x => return Err(format!("--keys: unknown key {x}")),
                    });
                }
            }
            "--type" => a.type_text = Some(val("--type")?),
            "--hiscores" => a.hiscores = Some(PathBuf::from(val("--hiscores")?)),
            "--leagues" => a.leagues = Some(PathBuf::from(val("--leagues")?)),
            "--demos" => a.demos = Some(PathBuf::from(val("--demos")?)),
            "--compare" => {
                let v = val("--compare")?;
                let (x, y) = v.split_once(',').ok_or("--compare takes A,B")?;
                a.compare = Some([PathBuf::from(x), PathBuf::from(y)]);
            }
            "--car-a" => a.compare_car[0] = Some(val("--car-a")?),
            "--car-b" => a.compare_car[1] = Some(val("--car-b")?),
            "--lap-a" => a.compare_lap[0] = Some(val("--lap-a")?.parse().map_err(|_| "bad --lap-a")?),
            "--lap-b" => a.compare_lap[1] = Some(val("--lap-b")?.parse().map_err(|_| "bad --lap-b")?),
            "--analysis-overlay" => match val("--analysis-overlay")?.as_str() {
                x @ ("working" | "drop" | "keep") => a.analysis_overlay = Some(x.to_string()),
                x => return Err(format!("--analysis-overlay: unknown overlay {x} (working, drop, keep)")),
            },
            "--analysis-drop" => a.analysis_drop = Some(PathBuf::from(val("--analysis-drop")?)),
            "--player-name" => a.player_name = Some(val("--player-name")?),
            "--team" => a.team = Some(val("--team")?),
            "--sound-devices" => a.sound_devices = Some(val("--sound-devices")?.split(';').filter(|n| !n.is_empty()).map(str::to_string).collect()),
            "--sound-device" => a.sound_device = Some(val("--sound-device")?),
            "--mode-dissolve" => a.mode_dissolve = Some(val("--mode-dissolve")?.parse::<f32>().map_err(|_| "bad --mode-dissolve")?.clamp(0.0, 1.0)),
            "--pause-hold" => a.pause_hold = Some(val("--pause-hold")?.parse::<f32>().map_err(|_| "bad --pause-hold")?.clamp(0.0, 1.0)),
            "--load-progress" => a.load_progress = Some(val("--load-progress")?.parse::<f32>().map_err(|_| "bad --load-progress")?.clamp(0.0, 1.0)),
            "--menu-time" => a.menu_time = val("--menu-time")?.parse().map_err(|_| "bad --menu-time")?,
            "--leave-time" => a.leave_time = Some(val("--leave-time")?.parse().map_err(|_| "bad --leave-time")?),
            "--engine-sound" => {
                a.engine_sound = match val("--engine-sound")?.to_ascii_lowercase().as_str() {
                    "classic" => false,
                    "simulated" => true,
                    v => return Err(format!("--engine-sound: classic or simulated, not {v}")),
                }
            }
            "--record-audio" => a.record_audio = Some(val("--record-audio")?.into()),
            "-h" | "--help" => return Err(USAGE.into()),
            _ if root.is_none() => root = Some(PathBuf::from(x)),
            _ => return Err(format!("unexpected argument {x}\n{USAGE}")),
        }
    }
    a.root = root.or_else(motor_data::fs::default_install_root).ok_or_else(|| format!("{}\n{USAGE}", motor_data::fs::NOT_FOUND))?;
    Ok(a)
}

/// Headless: put the game on `screen` (racing first if needed) and save a PNG.
/// design choice (09-remaster 9.1 rule 2, 9.7): how long the optional F9 dissolve takes
const DISSOLVE_SECS: f32 = 0.3;

/// The old image's share `p` (0..1) of the way through the dissolve: smoothstep, 1 -> 0.
fn dissolve_alpha(p: f32) -> f64 {
    let t = p.clamp(0.0, 1.0);
    (1.0 - t * t * (3.0 - 2.0 * t)) as f64
}

fn shot(a: &Args, screen: Screen, path: &Path) -> Result<(), String> {
    let mut game = Game::new(a.root.clone());
    game.prefetch = a.prefetch;
    // a results shot has no human unless `--autopilot` drives the human car (the YOU row)
    game.demo = a.demo || (matches!(screen, Screen::Results) && !a.autopilot);
    game.settings.cam.mirror = a.mirror;
    if let Some(c) = &a.cheats {
        game.cheats.set_by_names(c)?;
    }
    if a.time_attack {
        game.game_type = front::GameType::TimeAttack;
    }
    if a.knockout {
        game.game_type = front::GameType::Knockout;
    }
    game.free_look = a.free_look;
    if let Some(c) = a.car_colours {
        game.settings.custom_car_colours = true;
        game.settings.car_colours = c;
    }
    if let Some(f) = a.car_finish {
        game.settings.car_finish = f;
    }
    game.settings.car = game.car_index(a.car as u32); // --car N: file carNN.car
    game.settings.track = game.track_index(a.track as u32); // --track N: file TrackN.trk
    game.settings.laps = a.laps.unwrap_or(game.tracks[game.settings.track].laps);
    game.settings.view.motion_blur = a.motion_blur;
    if let Some(d) = a.difficulty {
        game.settings.difficulty = d;
    }
    if let Some(c) = a.car_shading {
        game.settings.view.car_shading = c;
    }
    if let Some(c) = a.car_detail {
        game.settings.view.car_detail = c;
    }
    if a.remaster {
        game.settings.view.mode = RenderMode::Remaster;
    }
    if let Some((on, _)) = a.skids {
        game.settings.skid_marks = on;
    }
    let r = &mut game.settings.view.remaster;
    if let Some((_, detailed)) = a.skids {
        r.skids_detailed = detailed;
    }
    if let Some(n) = a.msaa {
        r.render_aa = RenderAa::msaa(n);
    }
    if let Some((kind, strength)) = a.render_aa {
        r.render_aa = RenderAa { kind, strength: strength.unwrap_or(r.render_aa.strength) };
    }
    if let Some((kind, strength)) = a.post_aa {
        r.post_aa = PostAa { kind, strength: strength.unwrap_or(r.post_aa.strength) };
    }
    if a.grade {
        (r.grade, r.vignette, r.grain) = (50, 50, 50);
    }
    r.hdr &= !a.no_hdr;
    r.tonemap = a.tonemap.unwrap_or(r.tonemap);
    r.smooth_halos = a.smooth_halos.unwrap_or(r.smooth_halos);
    r.fog_style = a.fog_style.unwrap_or(r.fog_style);
    if let Some(t) = &a.fog_tune {
        // the keys over this track's own fog (the league's next race may differ: its track keeps the override too)
        let (f, warnings) = motorview::fog::from_text(&t.replace(',', "\n").replace('=', " "), motorview::fog::load(&a.root, a.track as u32));
        for w in warnings {
            eprintln!("--fog-tune: {w}");
        }
        r.fog_tune = Some(f);
    }
    r.sky_hdri = a.sky_hdri.unwrap_or(r.sky_hdri);
    r.sky_tune.exposure = a.sky_exposure.unwrap_or(r.sky_tune.exposure);
    r.sky_tune.yaw = a.sky_yaw.unwrap_or(r.sky_tune.yaw);
    r.sky_tune.sun &= !a.no_sky_sun;
    r.sky_tune.ambient &= !a.no_sky_ambient;
    r.sky_tune.ambient_tint = a.sky_ambient_tint.unwrap_or(r.sky_tune.ambient_tint);
    r.smooth_flares = a.smooth_flares.unwrap_or(r.smooth_flares);
    r.retro_signs = a.retro_signs.unwrap_or(r.retro_signs);
    r.car_shadows = a.car_shadows.unwrap_or(r.car_shadows);
    r.velocity_blur = a.velocity_blur.unwrap_or(r.velocity_blur);
    r.generated_cars = a.generated_cars.unwrap_or(r.generated_cars);
    r.world_fill = !a.no_fill;
    r.grass = a.grass.unwrap_or(r.grass);
    r.grass_range = a.grass_range.unwrap_or(r.grass_range);
    r.car_lod_distance = a.car_lod_distance.unwrap_or(r.car_lod_distance);
    r.fog_strength = a.fog_strength.unwrap_or(r.fog_strength);
    r.water = a.water.unwrap_or(r.water);
    r.tree_models = a.trees.unwrap_or(r.tree_models);
    r.water_ssr = a.water_ssr.unwrap_or(r.water_ssr);
    r.local_light = a.local_light.unwrap_or(r.local_light);
    r.texture_pack = a.texture_pack.unwrap_or(r.texture_pack);
    r.light_shadows = a.light_shadows.unwrap_or(r.light_shadows);
    r.shadow_quality = a.shadow_quality.unwrap_or(r.shadow_quality);
    r.modern_hud = a.modern_hud.unwrap_or(r.modern_hud);
    r.modern_menu = a.modern_menu.unwrap_or(r.modern_menu);
    r.rev_meter = a.rev_meter.unwrap_or(r.rev_meter);
    game.settings.menu_stretch = a.menu_stretch.unwrap_or(game.settings.menu_stretch);
    let (lo, hi) = hud_layout::SCALE_RANGE;
    game.settings.hud = hud_layout::HudSettings { scale: a.hud.scale.clamp(lo, hi), ..a.hud };
    game.settings.name_plates = a.name_plates.unwrap_or(game.settings.name_plates);
    // the --hud-* toggles are the shot's own set; Custom shows them (the centre texts draw as with All)
    game.settings.info_detail = hud_layout::INFO_CUSTOM;
    r.exposure = a.exposure.unwrap_or(r.exposure);
    r.bloom = a.bloom.unwrap_or(r.bloom);
    r.ssao = a.ssao.unwrap_or(r.ssao);
    if let Some(d) = a.view_distance {
        game.settings.view.distance = d;
    }
    if let Some(l) = &a.locked {
        let on: Vec<&str> = l.split(',').filter(|n| *n != "none").collect();
        game.flagged = Some(game.tracks.iter().map(|t| on.iter().any(|n| n.eq_ignore_ascii_case(&t.name))).collect());
    }
    if let Some(dir) = &a.hiscores {
        game.preview_hiscores(dir, a.player_name.as_deref());
    } else if let Some(n) = &a.player_name {
        game.settings.player_name = n.clone();
    }
    if let Some(t) = &a.team {
        game.settings.team = t.clone();
    }
    game.club_seen = a.club_seen;
    if let Some(dir) = &a.leagues {
        game.preview_leagues(dir);
    }
    if let Some(dir) = &a.demos {
        game.preview_demos(dir);
    }
    if let Some(list) = &a.sound_devices {
        game.preview_sound_devices(list, a.sound_device.as_deref());
    }
    if let Some(f) = &a.stats {
        game.preview_stats(&stats::Stats::load(f), a.stats_page);
    }
    // before the screen opens: the control options pick their page from it, a name requester its keyboard sheet
    if let Some(d) = a.glyphs {
        game.controls.last = d;
    }
    if let Some(n) = &a.league_name {
        game.go(Screen::League);
        game.preview_league_name(n);
    }
    if let Some(c) = a.season_end {
        game.preview_season_end(c);
    }
    if a.league {
        // the league dial's Continue: the next race's track, then the car screen
        game.go(Screen::League);
        if let Some(d) = a.league_div {
            game.preview_league_div(d);
        }
        game.key(KeyCode::Enter);
    } else if let Some(d) = a.league_div {
        // a league screen's shot (View standings, Load) with you in that division
        game.preview_league_div(d);
    }
    let inst = instance();
    let adapter = pollster::block_on(inst.request_adapter(&Default::default())).map_err(|e| format!("no GPU adapter: {e}"))?;
    game.render_device = adapter.get_info().name;
    let (dev, queue) = pollster::block_on(device(&adapter));
    let mut gpu = Gpu::new(&adapter, dev, queue, wgpu::TextureFormat::Rgba8Unorm);
    // REMASTER local light: the whole probe bake in the shot (#83)
    gpu.wait_local_light(true);
    gpu.load_ui(&game.assets.atlas);
    let (w, h) = a.size;
    let mut keys = HashSet::new();
    if let Some(f) = &a.ghost {
        // a saved lap (.ghl), or the first lap of a recording (.mhr)
        game.ghost_override = Some(match f.extension().and_then(|e| e.to_str()) {
            Some("ghl") => motor_sim::replay::GhostTrack::load(f)?.0,
            _ => motor_sim::replay::Recording::load(f)?.ghost(&game.root)?,
        });
    }
    if a.save_replay.is_some() {
        game.save_mhr = true;
    }
    if a.save_replay.is_some() || a.save_demo.is_some() {
        game.settings.record_race = true;
        game.demo_export = a.save_demo.clone();
        keys.insert(KeyCode::ArrowUp);
    }
    // Ghost mode: race the recorded cars of an .MDE
    if let Some(f) = &a.ghost_demo {
        game.ghost_demo = Some(motor_data::mde::Mde::load(f).map_err(|e| e.to_string())?);
        game.game_type = front::GameType::GhostMode;
    }
    let dt = 1.0 / 60.0;
    match screen {
        Screen::Race | Screen::Pause | Screen::PauseGfx | Screen::PauseSfx | Screen::PauseHud | Screen::PauseControls | Screen::PauseCam | Screen::Results => {
            match &a.replay {
                // an .MDE is watched as a demo (GameType 6), an .mhr is replayed from its inputs
                Some(f) if f.extension().is_some_and(|e| e.eq_ignore_ascii_case("mde")) => game.start_watch(motor_data::mde::Mde::load(f).map_err(|e| e.to_string())?)?,
                Some(f) => game.start_playback(motor_sim::replay::Recording::load(f)?)?,
                None if a.threaded_load => {
                    // the window's path: the loading screen starts the worker, the race follows
                    game.go(Screen::Loading);
                    if a.cancel_load {
                        while game.load_progress().is_none() {
                            game.update(dt, &HashSet::new());
                        }
                        std::thread::sleep(Duration::from_millis(100));
                        println!("cancel: at {:?}", game.load_progress());
                        game.key(KeyCode::Escape);
                        println!("cancel: screen {:?}, load running: {}", game.screen, game.load_progress().is_some());
                        game.go(Screen::Loading);
                    }
                    if a.cancel_upload {
                        while !game.load_uploading() && game.screen == Screen::Loading {
                            game.update(dt, &HashSet::new());
                            game.prepare(&mut gpu);
                            std::thread::sleep(Duration::from_millis(2));
                        }
                        println!("cancel: at {:?}, uploading {}", game.load_progress(), game.load_uploading());
                        game.key(KeyCode::Escape);
                        println!("cancel: screen {:?}, load running: {}", game.screen, game.load_progress().is_some());
                        game.go(Screen::Loading);
                    }
                    let t0 = Instant::now();
                    // the window's frame: `update`, then `prepare` (the upload's slices)
                    while game.screen == Screen::Loading {
                        game.update(dt, &HashSet::new());
                        game.prepare(&mut gpu);
                        std::thread::sleep(Duration::from_millis(2));
                    }
                    if game.race.is_none() {
                        return Err("the threaded load failed".into());
                    }
                    println!("threaded load: {:.0} ms until the race screen", t0.elapsed().as_secs_f32() * 1000.0);
                }
                None => {
                    if a.attract {
                        game.begin_attract(a.track_given.then_some(game.settings.track));
                    }
                    game.start_race()?
                }
            }
            if a.autopilot {
                if let Some(r) = &mut game.race {
                    r.sess.race.autopilot = true;
                }
            }
            // REMASTER demo play (#346): the playback controls in a state (the held controls are the shot's held keys)
            if let Some(spec) = &a.playback {
                let (held, ptr, press) = game.set_playback(spec)?;
                keys.extend(held);
                if let Some((x, y)) = ptr {
                    game.pointer_moved(x, y, (w, h));
                }
                if press {
                    game.controls.mouse_button(0, true);
                }
            }
            // the race screen first: `prepare` loads the REMASTER menu-drive scene (Neo City) on a
            // modern-menu screen, over the race's scene
            if let (Some([x, z, hd, fy]), Some(r)) = (a.place, game.race.as_mut()) {
                let slot = r.sess.focus;
                if !r.sess.race.place_at(slot, x, z, hd, fy) {
                    return Err(format!("--place: no ground at {x},{z} below y {fy}"));
                }
            }
            game.screen = Screen::Race;
            game.prepare(&mut gpu);
            let limit = if screen == Screen::Results { 3600.0 } else { a.sim_seconds };
            // the race's frames at `--frame-hz` (60 by default: the loading above stays at 1/60)
            let dt = 1.0 / a.frame_hz;
            let mut t = 0.0;
            while t < limit && game.screen == Screen::Race {
                game.update(dt, &keys);
                t += dt;
                // MotionBlur blends with the previous frames: draw the last few
                // (REMASTER TAA and the volumetric fog's temporal filter accumulate over 16 of them)
                let taa = a.remaster && matches!(a.post_aa, Some((PostAaKind::Taa, _)));
                let volumetric = a.remaster && a.fog_style == Some(motorview::render::FogStyle::Volumetric);
                if (a.motion_blur && t > limit - 10.0 * dt || (taa || volumetric) && t > limit - 16.0 * dt) && t + dt < limit {
                    gpu.offscreen(w, h, |g, enc, target, depth| game.render(g, enc, target, depth, (w, h)));
                }
            }
            // `--hud-show sector`: sector 1 ahead of the best lap with a best-lap message; `sector-slow`: sector 2 behind it
            if let (Some(slow), Some(r)) = (a.sector_sample.filter(|_| a.hud.sector_delay > 0), game.race.as_mut()) {
                (r.sector_flash, r.lap_flash) = if slow { (Some((4.0, 2, 1_234)), None) } else { (Some((4.0, 1, -214)), Some((4.0, 83_120, true, Some(-540)))) };
            }
            if matches!(screen, Screen::Pause | Screen::PauseGfx | Screen::PauseSfx | Screen::PauseHud | Screen::PauseControls | Screen::PauseCam) {
                game.key(KeyCode::Escape);
                // REMASTER: the tab bar; Classic: into GFX Options (3rd item) or SFX Options (4th item)
                let modern = game.settings.view.modern_menu();
                if modern {
                    game.pause_goto(match screen {
                        Screen::PauseGfx => 1,
                        Screen::PauseSfx => 2,
                        Screen::PauseControls => 3,
                        Screen::PauseCam => 4,
                        Screen::PauseHud => 5,
                        _ => 0,
                    });
                }
                let downs = if modern { 0 } else { match screen {
                    Screen::PauseGfx => 2,
                    Screen::PauseSfx => 3,
                    // the modern menu has Controls between Sound and HUD
                    Screen::PauseHud => 4 + 2 * game.settings.view.modern_menu() as usize,
                    Screen::PauseControls => 4,
                    Screen::PauseCam => 5,
                    _ => 0,
                } };
                for _ in 0..downs {
                    game.key(KeyCode::ArrowDown);
                }
                if downs > 0 {
                    game.key(KeyCode::Enter);
                }
            }
            println!("screen after {t:.0} s: {:?}", game.screen);
            if let Some(f) = &a.save_ghost {
                game.export_ghost(f)?;
                println!("wrote {}", f.display());
            }
            if let Some(f) = &a.save_demo {
                game.export_demo(f)?;
                println!("wrote {}", f.display());
            }
            if let Some(f) = &a.save_replay {
                game.export_recording(f)?;
                println!("wrote {}", f.display());
            }
        }
        // REMASTER Lap Analysis (#429): the `--compare` pair, else the Ghost screen's first recording
        Screen::Analysis => {
            let pair = a.compare.clone().map(|[x, y]| {
                let want = |i: usize| front::Want { slot: None, car: a.compare_car[i].clone(), lap: a.compare_lap[i] };
                [(x, want(0)), (y, want(1))]
            });
            game.preview_analysis(pair, a.analysis_overlay.as_deref())?;
            game.go(Screen::Analysis);
            if let Some(f) = a.analysis_drop.clone() {
                game.ana_dropped(f);
            }
        }
        s => {
            game.go(s);
            // #210: the idle timer runs for S seconds, then the demo (if it started) loads and plays a little
            if let Some(secs) = a.idle {
                game.attract_idle = true;
                let mut t = 0.0;
                while t < secs && game.screen == s {
                    game.update(dt, &HashSet::new());
                    t += dt;
                }
                // the main menu slides out before the demo's loading screen
                while game.leaving() {
                    game.update(dt, &HashSet::new());
                }
                while game.screen == Screen::Loading {
                    game.update(dt, &HashSet::new());
                    game.prepare(&mut gpu);
                    std::thread::sleep(Duration::from_millis(2));
                }
                if game.screen == Screen::Race {
                    game.prepare(&mut gpu);
                    for _ in 0..a.sim_seconds.mul_add(60.0, 0.0) as usize {
                        game.update(dt, &HashSet::new());
                    }
                }
                println!("idle {secs} s: screen {:?}, demo {}", game.screen, game.attract);
            }
        }
    }
    if a.prefetch {
        game.update(1.0 / 60.0, &Default::default()); // starts the garage build, as the first frame of the screen does
    }
    if a.leave_time.is_none() {
        for &k in &a.keys {
            game.key(k);
        }
    }
    if let Some(t) = &a.type_text {
        game.type_text(t);
    }
    if a.keep_display {
        game.preview_keep_display();
    }
    if a.confirm_exit {
        game.key(KeyCode::Escape);
    }
    // the Record race requester over the race: `ask` = the question, any other text = the name being typed
    if let Some(n) = &a.save_requester {
        game.preview_demo_save((n != "ask").then_some(n.as_str()));
    }
    for _ in 0..(a.menu_time / dt) as usize {
        // (the loading screen would start its load, and the shot would show the race or a timing-dependent frame)
        if !matches!(game.screen, Screen::Race | Screen::Loading) {
            game.update(dt, &keys); // let highlight animations settle
        }
    }
    // a Classic screen sliding out: the keys come after the settle, then the slide runs for S seconds
    if let Some(secs) = a.leave_time {
        for &k in &a.keys {
            game.key(k);
        }
        for _ in 0..(secs / dt).round() as usize {
            if !matches!(game.screen, Screen::Race | Screen::Loading) {
                game.update(dt, &keys);
            }
        }
    }
    if let Some(v) = a.pause_hold {
        game.pause_hold.v = v;
    }
    game.load_preview = a.load_progress;
    // the water and wind at the sim time, not the wall clock: the shot repeats byte for byte
    gpu.pin_clock(Some(a.sim_seconds));
    game.prepare(&mut gpu);
    // design choice (09-remaster 9.1 rule 2): the old mode as F9 leaves it, faded over this one
    let old = a.mode_dissolve.map(|p| {
        let mode = game.settings.view.mode;
        game.settings.view.mode = mode.toggle();
        game.prepare(&mut gpu);
        let snap = gpu.snapshot_target(w, h);
        let depth = gpu.depth_view(w, h);
        let mut enc = gpu.device.create_command_encoder(&Default::default());
        game.render(&gpu, &mut enc, &snap.view, &depth, (w, h));
        gpu.queue.submit([enc.finish()]);
        game.settings.view.mode = mode;
        game.prepare(&mut gpu);
        (snap, p)
    });
    let px = gpu.offscreen(w, h, |g, enc, target, depth| {
        game.render(g, enc, target, depth, (w, h));
        if let Some((snap, p)) = &old {
            g.draw_dissolve(enc, target, snap, dissolve_alpha(*p));
        }
    });
    png::write(path, w, h, &px).map_err(|e| e.to_string())?;
    println!("wrote {}", path.display());
    if a.bench > 0 {
        // wall time per frame incl. the read-back (equal in every run: compare differences)
        let mut ms: Vec<f64> = (0..a.bench)
            .map(|_| {
                if a.bench_move && game.screen == Screen::Race {
                    game.update(dt, &keys);
                }
                let t = Instant::now();
                gpu.offscreen(w, h, |g, enc, target, depth| game.render(g, enc, target, depth, (w, h)));
                t.elapsed().as_secs_f64() * 1000.0
            })
            .collect();
        ms.sort_by(f64::total_cmp);
        let mean = ms.iter().sum::<f64>() / ms.len() as f64;
        println!("bench {} frames at {w}x{h}: mean {mean:.2} ms, median {:.2} ms, p95 {:.2} ms", ms.len(), ms[ms.len() / 2], ms[ms.len() * 95 / 100]);
    }
    Ok(())
}

/// REMASTER: the modern HUD fonts (9.21) at a few sizes, styles and the fallback, drawn
/// through the sharp HUD batch on the 480-tall HUD canvas.
fn font_specimen(a: &Args, path: &Path) -> Result<(), String> {
    use motorview::ui::{Assets, Batch, Canvas, FontStyle};
    use motorview::ui_font::HudFont;
    let assets = Assets::font_only(&a.root);
    let inst = instance();
    let adapter = pollster::block_on(inst.request_adapter(&Default::default())).map_err(|e| format!("no GPU adapter: {e}"))?;
    let (dev, queue) = pollster::block_on(device(&adapter));
    let mut gpu = Gpu::new(&adapter, dev, queue, wgpu::TextureFormat::Rgba8Unorm);
    gpu.load_ui(&assets.atlas);
    let (w, h) = a.size;
    let canvas = Canvas::hud((w, h));
    let mut b = Batch::sharp(&assets, canvas);
    b.rect(0.0, 0.0, canvas.w, 240.0, [0.25, 0.3, 0.35, 1.0]);
    let yellow = [1.0, 0.86, 0.2, 1.0];
    let white = [1.0; 4];
    let mut y = 8.0;
    for (font, name) in [(HudFont::Clock, "Audiowide"), (HudFont::Speed, "Teko Bold"), (HudFont::Caption, "Rajdhani Medium")] {
        let st = FontStyle::new(14.0, white);
        let x = 8.0 + b.font_text(font, name, 8.0, y, &st) + 12.0;
        b.font_text(font, "ABCDEFGHIJKLMNOPQRSTUVWXYZ abcdefghijklmnopqrstuvwxyz 0123456789 :/-+.%»", x, y, &st);
        y += 24.0;
    }
    for size in [6.0, 9.0, 24.0] {
        let x = 8.0 + b.font_text(HudFont::Caption, "TOTAL TIME", 8.0, y, &FontStyle::new(size, yellow).outline([0.0; 3], size * 0.08)) + 12.0;
        b.font_text(HudFont::Clock, "01:23.45  LAP 2/3  POS 4/8", x, y, &FontStyle::new(size, white).outline([0.0; 3], size * 0.08));
        y += size * 1.6 + 6.0;
    }
    // the speed cluster: Teko with the 12° synthetic italic, over the light and the dark half
    for (x0, y0) in [(8.0, 168.0), (8.0, 300.0)] {
        let st = FontStyle::new(60.0, white).skew(12.0).outline([0.0; 3], 2.0);
        let adv = b.font_text(HudFont::Speed, "287", x0, y0, &st);
        b.font_text(HudFont::Caption, "KM/H", x0 + adv + 6.0, y0 + 44.0, &FontStyle::new(16.0, yellow));
        b.font_text(HudFont::Clock, "5", x0 + adv + 90.0, y0, &FontStyle::new(60.0, yellow).outline([0.0; 3], 2.0));
        b.font_text(HudFont::Caption, "GEAR", x0 + adv + 140.0, y0 + 44.0, &FontStyle::new(16.0, yellow));
    }
    b.font_text(HudFont::Clock, "»", 420.0, 300.0, &FontStyle::new(60.0, [0.2, 1.0, 0.1, 1.0]).outline([0.0; 3], 2.0));
    // the bitmap fields beside them: myfont through the 9.12 path
    b.text("myfont: TOTAL TIME 01:23.45", 8.0, 400.0, 2.0, white);
    // FONT1.FNT (the original front end's font) at 1x and 2x, in the menu palette's orange and white
    b.fnt_text("FONT1.FNT: Name: NeoCity  Best laptime: 1:23,45", 8.0, 432.0, 1.0, [1.0, 100.0 / 255.0, 0.0, 1.0]);
    b.fnt_text("FONT1.FNT: Mode: Quick race  Laps: 3", 8.0, 454.0, 2.0, white);
    let px = gpu.offscreen(w, h, |g, enc, target, _| g.draw_ui(enc, target, (w, h), canvas, &b.verts, Some([0.0, 0.0, 0.0]), Some(assets.sdf())));
    png::write(path, w, h, &px).map_err(|e| e.to_string())?;
    println!("wrote {}", path.display());
    Ok(())
}

/// REMASTER: the glass rev meter at fill 0.18 / 0.55 / 1.0 over the Blender preview's
/// background (RevMeter_750_preview.png has 0.18 / 0.75 / 1.0), plus a row at HUD size.
fn rev_meter_sheet(a: &Args, path: &Path) -> Result<(), String> {
    use motorview::revmeter;
    use motorview::ui::{Assets, Batch, Canvas};
    let assets = Assets::font_only(&a.root);
    let inst = instance();
    let adapter = pollster::block_on(inst.request_adapter(&Default::default())).map_err(|e| format!("no GPU adapter: {e}"))?;
    let (dev, queue) = pollster::block_on(device(&adapter));
    let mut gpu = Gpu::new(&adapter, dev, queue, wgpu::TextureFormat::Rgba8Unorm);
    gpu.load_ui(&assets.atlas);
    let (w, h) = a.size;
    let canvas = Canvas::hud((w, h));
    let mut b = Batch::sharp(&assets, canvas);
    let tune = a.rev_meter.unwrap_or_default();
    b.rect(0.0, 0.0, canvas.w, canvas.h, [0.13, 0.135, 0.145, 1.0]);
    // the big ones: 3 x the HUD size, drawn at 100 % then scaled like `hud::place`
    let k = 3.0;
    let (mw, mh) = revmeter::size();
    for (i, frac) in [0.18, 0.55, 1.0].into_iter().enumerate() {
        let start = b.verts.len();
        let (x, y) = (12.0, 8.0 + i as f32 * (mh * k + 10.0));
        b.rev_meter(0.0, 0.0, frac, &tune);
        for v in &mut b.verts[start..] {
            v.pos = [x + v.pos[0] * k, y + v.pos[1] * k];
        }
        b.rect(x + mw * k + 16.0, y, 1.0, mh * k, [0.3, 0.3, 0.3, 1.0]);
    }
    // HUD size (100 %) over a light and a dark ground
    let x0 = 12.0 + mw * k + 40.0;
    b.rect(x0 - 6.0, 8.0, mw + 12.0, mh * 3.0 + 30.0, [0.55, 0.6, 0.7, 1.0]);
    for (i, frac) in [0.18, 0.55, 1.0].into_iter().enumerate() {
        b.rev_meter(x0, 12.0 + i as f32 * (mh + 8.0), frac, &tune);
        b.rev_meter(x0, 200.0 + i as f32 * (mh + 8.0), frac, &tune);
    }
    let px = gpu.offscreen(w, h, |g, enc, target, _| g.draw_ui(enc, target, (w, h), canvas, &b.verts, Some([0.0, 0.0, 0.0]), Some(assets.sdf())));
    png::write(path, w, h, &px).map_err(|e| e.to_string())?;
    println!("wrote {}", path.display());
    Ok(())
}

struct App {
    game: Game,
    sound: audio::GameAudio,
    window: Option<Arc<Window>>,
    surface: Option<wgpu::Surface<'static>>,
    config: Option<wgpu::SurfaceConfiguration>,
    gpu: Option<Gpu>,
    depth: Option<wgpu::TextureView>,
    keys: HashSet<KeyCode>,
    /// accumulated touchpad wheel pixels
    wheel: f32,
    last: Instant,
    /// design choice: the old mode's last frame and when the F9 dissolve started
    dissolve: Option<(motorview::render::Snapshot, Instant)>,
    /// when the next frame is due (frame limit)
    next_frame: Instant,
    /// connected screens, in `Game::monitors` order
    monitors: Vec<MonitorHandle>,
    /// display settings the window currently has
    applied: Option<Display>,
    /// Wayland session: no exclusive fullscreen, windows can't place themselves
    wayland: bool,
    /// F12 pressed: save the next frame
    screenshot: bool,
    /// the system mouse arrow is shown (`Game::os_pointer`)
    cursor_shown: bool,
    /// Lap Analysis's BROWSE FILES (#429): the file dialog runs on its own thread (the frames go on); its answer comes here
    dialog: Option<std::sync::mpsc::Receiver<Option<PathBuf>>>,
    /// the dialog took the window out of exclusive fullscreen: `apply_display` puts it back when the dialog closes
    dialog_unfullscreened: bool,
}

impl App {
    /// REMASTER Lap Analysis (#429): the system's file dialog for a ghost, race replay or demo, over the window. It runs on a
    /// thread so the game keeps drawing (and a Linux portal dialog never blocks the event loop); exclusive fullscreen
    /// would hide it, so the window goes borderless meanwhile.
    fn open_file_dialog(&mut self) {
        let Some(w) = self.window.clone() else { return self.game.ana_dialog_done(None) };
        if matches!(w.fullscreen(), Some(Fullscreen::Exclusive(_))) {
            w.set_fullscreen(Some(Fullscreen::Borderless(w.current_monitor())));
            self.dialog_unfullscreened = true;
        }
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let p = rfd::FileDialog::new()
                .set_title("Choose a lap to compare")
                .add_filter("Motorhead laps (.ghl .mhr .MDE)", &["ghl", "GHL", "mhr", "MHR", "mde", "MDE"])
                .set_parent(&*w)
                .pick_file();
            let _ = tx.send(p);
        });
        self.dialog = Some(rx);
    }

    /// The file dialog answered (or its thread ended): the window goes back to its mode and the lap loads.
    fn poll_file_dialog(&mut self) {
        let Some(rx) = &self.dialog else { return };
        let p = match rx.try_recv() {
            Ok(p) => p,
            Err(std::sync::mpsc::TryRecvError::Empty) => return,
            Err(std::sync::mpsc::TryRecvError::Disconnected) => None,
        };
        self.dialog = None;
        if std::mem::take(&mut self.dialog_unfullscreened) {
            self.applied = None;
            self.apply_display();
        }
        self.game.ana_dialog_done(p);
    }

    /// Bring the window and surface in line with the graphic options: screen, window or
    /// fullscreen at the chosen resolution, vsync.
    fn apply_display(&mut self) {
        let d = self.game.settings.display;
        if self.applied == Some(d) {
            return;
        }
        let Some(w) = self.window.clone() else { return };
        let mon = self.monitors.get(self.game.monitor()).cloned().or_else(|| w.current_monitor());
        let size = winit::dpi::PhysicalSize::new(d.size.0, d.size.1);
        let moved = self.applied.is_some_and(|a| a.monitor != d.monitor);
        if d.fullscreen {
            // exclusive mode at the resolution when the screen offers it (fastest refresh),
            // else borderless at the screen's own size. Wayland ignores exclusive mode.
            let mode = if self.wayland {
                None
            } else {
                mon.as_ref().and_then(|m| m.video_modes().filter(|v| v.size() == size).max_by_key(|v| v.refresh_rate_millihertz()))
            };
            // compositors keep a fullscreen window where it is: leave fullscreen to change screen
            if moved && w.fullscreen().is_some() {
                w.set_fullscreen(None);
            }
            w.set_fullscreen(Some(match mode {
                Some(m) => Fullscreen::Exclusive(m),
                None => Fullscreen::Borderless(mon.clone()),
            }));
        } else {
            if moved && self.wayland {
                eprintln!("display: Wayland doesn't let a window move itself to another screen; use fullscreen (or move it with the desktop)");
            }
            w.set_fullscreen(None);
            if let Some(s) = w.request_inner_size(size) {
                self.resize(s.width, s.height);
            }
            // centre on the chosen screen (ignored where windows can't be placed)
            if let Some(m) = &mon {
                let (p, ms) = (m.position(), m.size());
                let x = p.x + (ms.width as i32 - size.width as i32).max(0) / 2;
                let y = p.y + (ms.height as i32 - size.height as i32).max(0) / 2;
                w.set_outer_position(winit::dpi::PhysicalPosition::new(x, y));
            }
        }
        let vsync_changed = self.applied.is_none_or(|a| a.vsync != d.vsync);
        if let (true, Some(s), Some(c), Some(g)) = (vsync_changed, &self.surface, &mut self.config, &self.gpu) {
            c.present_mode = present_mode(d.vsync);
            s.configure(&g.device, c);
        }
        let name = self.game.monitors.get(self.game.monitor()).map_or("default screen", |s| s.as_str());
        let mode = if d.fullscreen { "fullscreen" } else { "window" };
        eprintln!("display: {name}, {mode} {}x{}, vsync {}", d.size.0, d.size.1, if d.vsync { "on" } else { "off" });
        self.applied = Some(d);
    }

    fn resize(&mut self, w: u32, h: u32) {
        let (Some(s), Some(c), Some(g)) = (&self.surface, &mut self.config, &self.gpu) else { return };
        c.width = w.max(1);
        c.height = h.max(1);
        self.game.pointer_left();
        s.configure(&g.device, c);
        self.depth = Some(g.depth_view(c.width, c.height));
    }

    /// F9 with the Dissolve option: render the old mode's frame into a snapshot first.
    fn capture_dissolve(&mut self) {
        self.dissolve = None;
        let (Some(gpu), Some(cfg), Some(depth)) = (&mut self.gpu, &self.config, &self.depth) else { return };
        if !self.game.settings.mode_dissolve {
            return;
        }
        self.game.prepare(gpu);
        let snap = gpu.snapshot_target(cfg.width, cfg.height);
        let mut enc = gpu.device.create_command_encoder(&Default::default());
        self.game.render(gpu, &mut enc, &snap.view, depth, (cfg.width, cfg.height));
        gpu.queue.submit([enc.finish()]);
        self.dissolve = Some((snap, Instant::now()));
    }

    fn frame(&mut self) {
        let (Some(surface), Some(gpu), Some(cfg)) = (&self.surface, &mut self.gpu, &self.config) else { return };
        self.game.prepare(gpu);
        let Some(depth) = &self.depth else { return };
        let frame = match surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(t) | wgpu::CurrentSurfaceTexture::Suboptimal(t) => t,
            wgpu::CurrentSurfaceTexture::Outdated | wgpu::CurrentSurfaceTexture::Lost => {
                surface.configure(&gpu.device, cfg);
                return;
            }
            _ => return,
        };
        let view = frame.texture.create_view(&Default::default());
        // REMASTER: the screenshot draws this frame again, with the same velocity blur
        let held = self.screenshot.then(|| gpu.motion_history());
        let mut enc = gpu.device.create_command_encoder(&Default::default());
        self.game.render(gpu, &mut enc, &view, depth, (cfg.width, cfg.height));
        // design choice (09-remaster 9.1 rule 2): the F9 dissolve over the finished frame
        if let Some((snap, start)) = &self.dissolve {
            let p = start.elapsed().as_secs_f32() / DISSOLVE_SECS;
            if p < 1.0 && snap.size == (cfg.width, cfg.height) {
                gpu.draw_dissolve(&mut enc, &view, snap, dissolve_alpha(p));
            } else {
                self.dissolve = None;
            }
        }
        gpu.queue.submit([enc.finish()]);
        gpu.queue.present(frame);
        if std::mem::take(&mut self.screenshot) {
            // the frame again, offscreen at the window size (the swapchain can't be read back)
            let (w, h) = (cfg.width, cfg.height);
            if let Some(m) = held {
                gpu.restore_motion_history(m);
            }
            let mut px = gpu.offscreen(w, h, |g, enc, target, depth| self.game.render(g, enc, target, depth, (w, h)));
            save_screenshot(gpu.color_format, w, h, &mut px);
        }
    }
}

impl ApplicationHandler for App {
    /// The event loop is over (window closed or the exit requester): save the statistics.
    fn exiting(&mut self, _: &ActiveEventLoop) {
        self.game.shutdown();
    }

    fn resumed(&mut self, el: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }
        #[cfg(target_os = "linux")]
        {
            use winit::platform::wayland::ActiveEventLoopExtWayland;
            self.wayland = el.is_wayland();
        }
        self.game.set_wayland(self.wayland);
        self.monitors = el.available_monitors().collect();
        self.game.monitors = self.monitors.iter().enumerate().map(|(i, m)| m.name().unwrap_or_else(|| format!("Screen {}", i + 1))).collect();
        let (w, h) = self.game.settings.display.size;
        let window = Arc::new(
            el.create_window(motorview::app_icon::apply(Window::default_attributes()).with_title(format!("Motorhead {}", motor_version::DISPLAY)).with_inner_size(winit::dpi::PhysicalSize::new(w, h)))
                .expect("create window"),
        );
        let inst = instance();
        let surface = inst.create_surface(window.clone()).expect("create surface");
        let adapter = pollster::block_on(inst.request_adapter(&wgpu::RequestAdapterOptions { compatible_surface: Some(&surface), ..Default::default() }))
            .expect("no GPU adapter");
        let (dev, queue) = pollster::block_on(device(&adapter));
        let size = window.inner_size();
        let mut cfg = surface.get_default_config(&adapter, size.width.max(1), size.height.max(1)).expect("surface config");
        // gamma-space output, like the original
        if let Some(f) = surface.get_capabilities(&adapter).formats.into_iter().find(|f| !f.is_srgb()) {
            cfg.format = f;
        }
        cfg.present_mode = present_mode(self.game.settings.display.vsync);
        // MotionBlur copies each frame into its history texture
        if surface.get_capabilities(&adapter).usages.contains(wgpu::TextureUsages::COPY_SRC) {
            cfg.usage |= wgpu::TextureUsages::COPY_SRC;
        }
        surface.configure(&dev, &cfg);
        self.game.render_device = adapter.get_info().name;
        let mut gpu = Gpu::new(&adapter, dev, queue, cfg.format);
        gpu.load_ui(&self.game.assets.atlas);
        self.depth = Some(gpu.depth_view(cfg.width, cfg.height));
        self.gpu = Some(gpu);
        self.config = Some(cfg);
        self.surface = Some(surface);
        self.window = Some(window);
        self.apply_display();
        self.last = Instant::now();
    }

    fn window_event(&mut self, el: &ActiveEventLoop, _: WindowId, ev: WindowEvent) {
        match ev {
            WindowEvent::CloseRequested => el.exit(),
            WindowEvent::Resized(s) => self.resize(s.width, s.height),
            WindowEvent::KeyboardInput { event, .. } => {
                let PhysicalKey::Code(code) = event.physical_key else { return };
                if event.state == ElementState::Released {
                    self.keys.remove(&code);
                    return;
                }
                let typing = self.game.editing_text();
                if event.repeat && !typing && !matches!(code, KeyCode::ArrowUp | KeyCode::ArrowDown | KeyCode::ArrowLeft | KeyCode::ArrowRight) {
                    return;
                }
                if let Some(t) = &event.text {
                    if typing {
                        self.game.type_text(t);
                    } else {
                        self.game.cheat_text(t);
                    }
                }
                self.keys.insert(code);
                self.game.controls.used_keyboard();
                if self.game.is_screenshot_key(code) {
                    self.screenshot = true;
                } else if code == KeyCode::F11 {
                    self.game.toggle_fullscreen();
                } else if self.game.is_render_mode_key(code) {
                    self.capture_dissolve();
                    self.game.toggle_render_mode();
                } else {
                    self.game.key(code);
                }
                self.apply_display();
                if self.game.quit {
                    el.exit();
                }
            }
            // the mouse pointer for the demo-play controls (#352)
            WindowEvent::CursorMoved { position, .. } => {
                if let Some(c) = &self.config {
                    self.game.pointer_moved(position.x as f32, position.y as f32, (c.width, c.height));
                }
            }
            WindowEvent::CursorLeft { .. } => self.game.pointer_left(),
            // Lap Analysis (#429): a file dragged onto the window (Windows and X11; winit 0.30 sends none on Wayland)
            WindowEvent::HoveredFile(_) => self.game.ana_hover(true),
            WindowEvent::HoveredFileCancelled => self.game.ana_hover(false),
            WindowEvent::DroppedFile(p) => self.game.ana_dropped(p),
            WindowEvent::MouseInput { state, button, .. } => {
                let i = match button {
                    MouseButton::Left => 0,
                    MouseButton::Right => 1,
                    MouseButton::Middle => 2,
                    _ => return,
                };
                self.game.controls.mouse_button(i, state == ElementState::Pressed);
                if state == ElementState::Pressed {
                    self.game.controls.used_keyboard();
                    self.game.menu_mouse(if i == 0 { KeyCode::Enter } else if i == 1 { KeyCode::Escape } else { return });
                    self.apply_display();
                    if self.game.quit {
                        el.exit();
                    }
                }
            }
            // the wheel steps the menus: up = left / up, down = right / down
            WindowEvent::MouseWheel { delta, .. } => {
                let y = match delta {
                    MouseScrollDelta::LineDelta(_, y) => y,
                    MouseScrollDelta::PixelDelta(p) => {
                        self.wheel += p.y as f32 / 40.0;
                        let n = self.wheel.trunc();
                        self.wheel -= n;
                        n
                    }
                };
                if y != 0.0 {
                    self.game.controls.used_keyboard();
                    self.game.menu_mouse(if y > 0.0 { KeyCode::ArrowLeft } else { KeyCode::ArrowRight });
                }
            }
            WindowEvent::RedrawRequested => {
                let now = Instant::now();
                let dt = (now - self.last).as_secs_f32().min(0.1);
                self.last = now;
                self.game.poll_pads();
                if self.game.quit {
                    el.exit();
                }
                self.poll_file_dialog();
                self.game.update(dt, &self.keys);
                if self.dialog.is_none() && self.game.ana_take_browse() {
                    self.open_file_dialog();
                }
                self.apply_display(); // the keep-settings countdown may revert
                // the system's mouse arrow shows in the menus only; a race has none (demo play draws its own)
                let shown = self.game.os_pointer();
                if shown != self.cursor_shown {
                    if let Some(w) = &self.window {
                        w.set_cursor_visible(shown);
                    }
                    self.cursor_shown = shown;
                }
                self.sound.sync(&mut self.game, dt);
                self.frame();
            }
            _ => {}
        }
    }

    fn device_event(&mut self, _: &ActiveEventLoop, _: DeviceId, ev: DeviceEvent) {
        if let DeviceEvent::MouseMotion { delta } = ev {
            self.game.controls.mouse_motion(delta.0, delta.1);
        }
    }

    /// Frame pacing: redraw now, or wait until the next frame slot of the graphic-options
    /// frame limit (presentation runs without vsync so 120/240 aren't capped at the monitor).
    fn about_to_wait(&mut self, el: &ActiveEventLoop) {
        let Some(w) = &self.window else { return };
        let limit = self.game.settings.fps_limit;
        if limit == 0 {
            el.set_control_flow(ControlFlow::Poll);
            w.request_redraw();
            return;
        }
        let period = Duration::from_secs_f64(1.0 / limit as f64);
        let now = Instant::now();
        if now >= self.next_frame {
            // keep a steady cadence; after a stall start counting again from now
            self.next_frame = if self.next_frame + period < now { now + period } else { self.next_frame + period };
            w.request_redraw();
        }
        el.set_control_flow(ControlFlow::WaitUntil(self.next_frame));
    }
}

/// F12: write `MotorheadNNNN.png` (first free number) next to the settings file, on a
/// worker thread. Design choice: PNG in the settings folder; the original writes numbered
/// TGAs (`MenuSnapShot0000.TGA`, file offset 0xbc590) [O], in-race name and folder [U].
fn save_screenshot(format: wgpu::TextureFormat, w: u32, h: u32, px: &mut [u8]) {
    use wgpu::TextureFormat as F;
    match format {
        F::Rgba8Unorm | F::Rgba8UnormSrgb => {}
        F::Bgra8Unorm | F::Bgra8UnormSrgb => px.as_chunks_mut::<4>().0.iter_mut().for_each(|p| p.swap(0, 2)),
        // packed little-endian u32: r bits 0..10, g 10..20, b 20..30, a 30..32
        F::Rgb10a2Unorm => px.as_chunks_mut::<4>().0.iter_mut().for_each(|p| {
            let v = u32::from_le_bytes([p[0], p[1], p[2], p[3]]);
            let c = |s: u32| (((v >> s) & 0x3ff) >> 2) as u8;
            p.copy_from_slice(&[c(0), c(10), c(20), 255]);
        }),
        f => {
            eprintln!("screenshot: surface format {f:?} not supported");
            return;
        }
    }
    let Some(dir) = front::settings_path().and_then(|p| p.parent().map(Path::to_path_buf)) else { return };
    let Some(path) = (0..10000).map(|n| dir.join(format!("Motorhead{n:04}.png"))).find(|p| !p.exists()) else { return };
    let px = px.to_vec();
    std::thread::spawn(move || {
        let r = std::fs::create_dir_all(&dir).and_then(|_| png::write(&path, w, h, &px));
        match r {
            Ok(()) => println!("screenshot: {}", path.display()),
            Err(e) => eprintln!("screenshot: {}: {e}", path.display()),
        }
    });
}

/// Vsync on: wait for the screen (the frame limit still caps below its rate); off: the frame
/// limit alone paces the game, so 120/240 aren't capped at the monitor.
fn present_mode(vsync: bool) -> wgpu::PresentMode {
    if vsync {
        wgpu::PresentMode::AutoVsync
    } else {
        wgpu::PresentMode::AutoNoVsync
    }
}

/// Headless: race with the AI driving for `sim_seconds` and write the game's sound to a WAV
/// (44.1 kHz stereo 16-bit) through the same mixer the window uses.
fn record_audio(a: &Args, path: &Path) -> Result<(), String> {
    let mut game = Game::new(a.root.clone());
    game.demo = true;
    game.settings.car = game.car_index(a.car as u32); // --car N: file carNN.car
    game.settings.engine_sound = a.engine_sound;
    game.settings.track = game.track_index(a.track as u32); // --track N: file TrackN.trk
    let audio = motor_audio::Audio::silent();
    let mixer = audio.mixer.clone();
    let mut sound = audio::GameAudio::new(&a.root, audio, None);
    game.start_race()?;
    game.go(Screen::Race);
    let (dt, rate) = (1.0 / 60.0, 44100usize);
    let keys = HashSet::new();
    let mut pcm: Vec<i16> = Vec::new();
    let mut buf = vec![0.0f32; 2 * rate / 60];
    for _ in 0..(a.sim_seconds / dt) as usize {
        game.update(dt, &keys);
        sound.sync(&mut game, dt);
        mixer.lock().map_err(|e| e.to_string())?.render(&mut buf, 2);
        pcm.extend(buf.iter().map(|&s| motor_audio::wav::to_pcm16(s)));
    }
    std::fs::write(path, motor_audio::wav::encode_pcm16(rate as u32, 2, &pcm)).map_err(|e| e.to_string())?;
    if let Some(r) = &game.race {
        println!("skid-mark segments on the track: {}", r.sess.skid_marks(false).len() / 6);
        let [smoke, grass, sparks] = r.sess.fx.spawned;
        println!("particles spawned: {smoke} smoke, {grass} grass, {sparks} sparks");
    }
    println!("wrote {}", path.display());
    Ok(())
}

fn main() {
    if std::env::args().any(|a| a == "--version") {
        println!("motorhead {}", motor_version::DISPLAY);
        return;
    }
    let args = match parse_args() {
        Ok(a) => a,
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(2);
        }
    };
    if let Some(path) = &args.record_audio {
        if let Err(e) = record_audio(&args, path) {
            eprintln!("{e}");
            std::process::exit(1);
        }
        return;
    }
    if let Some(path) = &args.rev_meter_sheet {
        if let Err(e) = rev_meter_sheet(&args, path) {
            eprintln!("{e}");
            std::process::exit(1);
        }
        return;
    }
    if let Some(path) = &args.font_specimen {
        if let Err(e) = font_specimen(&args, path) {
            eprintln!("{e}");
            std::process::exit(1);
        }
        return;
    }
    if let Some((screen, path)) = &args.shot {
        if let Err(e) = shot(&args, *screen, path) {
            eprintln!("{e}");
            std::process::exit(1);
        }
        return;
    }
    // #398: live values tuned on motorview's Constants page (none: the source's values)
    motorview::consts::load_overrides();
    let mut game = Game::new(args.root.clone());
    game.demo = args.demo;
    game.attract_idle = true; // #210: the idle main menu plays the attract demo
    game.prefetch = true;
    game.load_settings();
    game.controls.open_pads();
    let el = EventLoop::new().expect("event loop");
    // #260: the output picked on the Sound screen, else the system default
    let device = game.settings.sound_device.as_ref().map(|d| d.id.clone());
    let audio = motor_audio::Audio::open_on(device.as_deref());
    // the intro movies (#42): the sound output keeps their clock when a device runs
    game.movie_hold = audio.has_device();
    if game.settings.intro && !args.no_intro && game.has_movies() {
        game.go(Screen::Intro);
    }
    let sound = audio::GameAudio::new(&args.root, audio, device);
    let mut app = App { game, sound, window: None, surface: None, config: None, gpu: None, depth: None, keys: HashSet::new(), wheel: 0.0, last: Instant::now(), dissolve: None, next_frame: Instant::now(), monitors: Vec::new(), applied: None, wayland: false, screenshot: false, cursor_shown: true, dialog: None, dialog_unfullscreened: false };
    el.run_app(&mut app).expect("event loop");
}
