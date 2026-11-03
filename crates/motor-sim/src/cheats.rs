//! The original's profile-name cheats (re-notes 08 §8.14, checker `0x46f3b4`): the driver name and
//! the team name switch one mode on. Nothing is saved; the result is rebuilt for every race.
//!
//! This is the test (`NameCheat::of`) and the effects of moon gravity (#315), bouncy wheels (#313) and
//! bouncy cars (#314), all applied by `Race::set_name_cheat`, and the constants of supercars / grem's top-down view (#319) and
//! ignition / uds's high chase camera (#318), read by the game's camera. The other effects live with their own tickets (#316-#321) and read `Race::name_cheat`. `r pettersson / swe` (#310) and `avenger / zx` (#322) are not part of the
//! flag word and have their own functions here.

/// Characters of the driver name that are compared (`0x495a1f`, strnicmp) [D].
pub const NAME_LEN: usize = 16;
/// Characters of the team name that are compared: a team "Gremlin" matches "grem" [D].
pub const TEAM_LEN: usize = 4;

/// The mode a driver/team pair switches on. Doc: flag bit in `0x704918`, ticket.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NameCheat {
    /// 0x01 demon / grem: soft damper, longer and softer springs (#313)
    BouncyWheels,
    /// 0x02 buzz aldrin / nasa: half gravity (#315)
    MoonGravity,
    /// 0x04 g-ride / west: negative damping (#314)
    BouncyCars,
    /// 0x08 lemmy / ace: "Hell mode": red world, black fog, cloud-ceiling sky, lightning (#316)
    Hell,
    /// 0x10 ramlosa / h2o: "Underwater": teal look, ceiling sky, wobbling picture, bubbles, half-speed race (#317)
    Underwater,
    /// 0x20 ignition / uds: "Ignition mode": a high chase camera, OBJ2 cars, black background (#318)
    Ignition,
    /// 0x40 tribute to tron, any team: Tron look, Glide only in the original (#320)
    Tron,
    /// 0x80 supercars / grem: "Supercars mode", a top-down camera (Super Cars was a top-down racer) (#319)
    Supercars,
    /// 0x100 r3tr0 / a500: Amiga scroller, Glide only in the original (#321)
    AmigaScroller,
}

/// The table of `0x4be250..` in the checker's order: (name, team, cheat). A `None` team is not
/// compared (Tron). "grem" is shared by demon and supercars; the name tells them apart.
const PAIRS: [(&str, Option<&str>, NameCheat); 9] = [
    ("demon", Some("grem"), NameCheat::BouncyWheels),
    ("buzz aldrin", Some("nasa"), NameCheat::MoonGravity),
    ("g-ride", Some("west"), NameCheat::BouncyCars),
    ("lemmy", Some("ace"), NameCheat::Hell),
    ("ramlosa", Some("h2o"), NameCheat::Underwater),
    ("ignition", Some("uds"), NameCheat::Ignition),
    ("tribute to tron", None, NameCheat::Tron),
    ("supercars", Some("grem"), NameCheat::Supercars),
    ("r3tr0", Some("a500"), NameCheat::AmigaScroller),
];

/// strnicmp(a, b, n) == 0: the first `n` bytes, ASCII case ignored; both strings end at `n` or
/// earlier, so a shorter string only matches an equal one.
fn eq_n(a: &str, b: &str, n: usize) -> bool {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    a[..a.len().min(n)].eq_ignore_ascii_case(&b[..b.len().min(n)])
}

impl NameCheat {
    /// The cheat of this driver and team name: the first matching pair wins, only one is ever on.
    pub fn of(name: &str, team: &str) -> Option<NameCheat> {
        PAIRS.iter().find(|(n, t, _)| eq_n(name, n, NAME_LEN) && t.is_none_or(|t| eq_n(team, t, TEAM_LEN))).map(|p| p.2)
    }
}

/// buzz aldrin / nasa (`sim_step` `0x481548`): gravity 7.905 (`0x40fcf5c3`) instead of 15.81, half of
/// it, for every car and prop of the race (#315). `GRAVITY` for every other mode.
pub fn gravity(cheat: Option<NameCheat>) -> f32 {
    if cheat == Some(NameCheat::MoonGravity) { crate::world::GRAVITY * 0.5 } else { crate::world::GRAVITY }
}

/// demon / grem (car init `0x40ced0`, #313): (spring length factor, spring rate factor, damper `SUSP`).
/// 3.0 (`0x4cf41b`), 0.16 (`0x4cf42b`), 21666 (`0x40d1b4`) for `BouncyWheels`; the loaded values (1, 1,
/// `SUSP_DAMP` = 65000) for every other mode. g-ride / west (`BouncyCars`, #314): length x1.2 (`0x4cf413`),
/// rate x0.5 (`0x4cf423`) and a negative damper -50000 (`0x40d3fe`) that pumps energy in.
pub fn suspension(cheat: Option<NameCheat>) -> (f32, f32, f32) {
    match cheat {
        Some(NameCheat::BouncyWheels) => (3.0, 0.16, 21666.0),
        Some(NameCheat::BouncyCars) => (1.2, 0.5, -50000.0),
        _ => (1.0, 1.0, crate::car::SUSP_DAMP),
    }
}

/// supercars / grem (`0x439e84` at `0x439f7a`, #319): the followed car is seen from straight above, this many
/// metres up (`0x41c80000`) and this far ahead along its heading (`0x41400000`) [O].
pub const TOPDOWN_UP: f32 = 25.0;
pub const TOPDOWN_AHEAD: f32 = 12.0;
/// The body level every car takes under supercars / grem: OBJ3, 42 polygons with the wheels built in
/// (`0x4392b4`, `0x4392e4`); ignition / uds takes level 2 [O]. Classic only (a design choice for Remaster).
pub const TOPDOWN_LEVEL: usize = 3;

/// Whether the mode is supercars / grem, the top-down view.
pub fn top_down(cheat: Option<NameCheat>) -> bool {
    cheat == Some(NameCheat::Supercars)
}

/// ignition / uds (`0x42c944`, reached from the chase cameras `0x42c540` / `0x42c710`, #318) [O]: the chase eye's target is
/// `IGNITION_BACK` behind the car along its forward x / z (`0x4d2db5`, f64 -12.0) and `IGNITION_UP` above it (`0x41880000`),
/// lowered by `IGNITION_PITCH` x forward.y (`0x4d2dbd`); the camera looks at the car origin + rot (0, 0, `IGNITION_AHEAD`)
/// (`0x4b02b8`).
pub const IGNITION_UP: f32 = 17.0;
pub const IGNITION_BACK: f32 = 12.0;
pub const IGNITION_PITCH: f32 = 3.0;
pub const IGNITION_AHEAD: f32 = 2.0;
/// Its easing per world axis x, y, z (`0x4b02ac`, clamped to 0.05..1.0 by `0x42b780`): applied once per drawn frame, no dt
/// term, so x lags and z snaps [O]. Ours applies them per 1/60 s (design choice).
pub const IGNITION_K: [f32; 3] = [0.05, 0.9, 1.0];
/// The body level every car takes under ignition / uds (`0x43957c` -> `0x4392c2`) [O]. Classic only.
pub const IGNITION_LEVEL: usize = 2;

/// Whether the mode is ignition / uds, the high chase camera.
pub fn ignition(cheat: Option<NameCheat>) -> bool {
    cheat == Some(NameCheat::Ignition)
}

/// The lowest body level of every car and the black background instead of the sky (`0x443a99`, `0x443b3d`), CLASSIC:
/// `TOPDOWN_LEVEL` for supercars / grem, `IGNITION_LEVEL` for ignition / uds, `None` for every other mode.
pub fn car_level(cheat: Option<NameCheat>) -> Option<usize> {
    match cheat {
        Some(NameCheat::Supercars) => Some(TOPDOWN_LEVEL),
        Some(NameCheat::Ignition) => Some(IGNITION_LEVEL),
        _ => None,
    }
}

/// lemmy / ace, "Hell mode" (`0x439e84`, #316): WorldBrightness and ObjectBrightness (`0x4b00e8..f0`, `0x4b0108..110`)
/// are forced to this, replacing the track's own and the user's Brightness every frame [O].
pub const HELL_BRIGHTNESS: [f32; 3] = [0.8, 0.4, 0.4];
/// The lightning frame (`0x439f1b..`): both brightnesses 3.0, CueColour white (255), sky top 0x7f7f60ff, bottom white.
pub const HELL_FLASH_BRIGHTNESS: f32 = 3.0;
/// Sky gradient colours (`0x4ebea8` top, `0x4ebea4` bottom; 0..255 rgb): normal 0x7fff0000 / 0x7f3f006f, flash 0x7f7f60ff / 0x7fffffff.
pub const HELL_SKY: ([f32; 3], [f32; 3]) = ([255.0, 0.0, 0.0], [63.0, 0.0, 111.0]);
pub const HELL_SKY_FLASH: ([f32; 3], [f32; 3]) = ([127.0, 96.0, 255.0], [255.0, 255.0, 255.0]);
/// Cloud-ceiling scroll per metre of eye position (`0x443570`: 0.005; underwater uses 0.001) [O].
pub const HELL_SCROLL: f32 = 0.005;
/// Lightning rate: the original rolls `rand()` (0..32767) once per SetCamera and flashes when `20000 < r < 20400`.
const FLASH_LO: u32 = 20000;
const FLASH_HI: u32 = 20400;
/// Ours rolls on a fixed 30 Hz tick of the race clock, not per drawn frame (design choice, user).
pub const HELL_FLASH_HZ: f32 = 30.0;

/// ramlosa / h2o, "Underwater" (`0x43a54b`, #317): WorldBrightness and ObjectBrightness forced to this every frame,
/// as in Hell mode [O].
pub const UNDERWATER_BRIGHTNESS: [f32; 3] = [0.5, 1.0, 1.2];
/// Its CueColour (`0x4ebf8c..94` = 0, 100.0, 64.0; 0..255): the fog. The packed copy `0x4ec124` is rebuilt from it (`0x43a048`) [O].
pub const UNDERWATER_FOG: [f32; 3] = [0.0, 100.0, 64.0];
/// Sky gradient colours (`0x4ebea8` top 0x7f60a0ff, `0x4ebea4` bottom 0x7f00a080; 0..255 rgb) of the cloud ceiling [O].
pub const UNDERWATER_SKY: ([f32; 3], [f32; 3]) = ([96.0, 160.0, 255.0], [0.0, 160.0, 128.0]);
/// Cloud-ceiling scroll per metre of eye position under water (`0x443a56`: 0x3a83126f) [O].
pub const UNDERWATER_SCROLL: f32 = 0.001;
/// Once the race clock runs, the race frame calls `sim_step` with dt x 0.5 (`0x45c2d6`, `0x4d4ee6`); the clock itself
/// keeps real time [O]. Before GO the step is the normal one.
pub const UNDERWATER_TIME_SCALE: f32 = 0.5;
/// The picture's wobble (`0x4419b4`, run once per drawn frame on the projected polygons) [O]: the phase steps by this
/// (`0x4d377d`) and wraps at 2 pi; ours steps it on a 30 Hz clock, as `HELL_FLASH_HZ` (design choice).
pub const WOBBLE_STEP: f32 = 0.05;
/// Zoom s = `WOBBLE_ZOOM` + `WOBBLE_BREATH` cos(phase) (`0x4d3795`, `0x4d378d`) [O]; ours zooms around the centre, the
/// original around the bottom-right corner (design choice, user).
pub const WOBBLE_ZOOM: f32 = 1.2;
pub const WOBBLE_BREATH: f32 = 0.05;
/// Sideways ripple: `WOBBLE_AMP_PX` sin((y - H) * `WOBBLE_WAVES` / (H / 2) + phase) px (`0x4d37a5`, `0x4d379d`) [O]; the
/// original's pixels are taken at 480 lines (design choice).
pub const WOBBLE_AMP_PX: f32 = 10.0;
pub const WOBBLE_WAVES: f32 = 25.0;

/// Whether the mode is ramlosa / h2o, Underwater.
pub fn underwater(cheat: Option<NameCheat>) -> bool {
    cheat == Some(NameCheat::Underwater)
}

/// The share of the race clock's time the simulation steps once the race runs: 0.5 under water, else 1.
pub fn time_scale(cheat: Option<NameCheat>) -> f32 {
    if underwater(cheat) { UNDERWATER_TIME_SCALE } else { 1.0 }
}

/// The wobble's phase (radians, 0..2 pi) at `t` seconds of race time: `WOBBLE_STEP` per 1/30 s tick, continuous.
pub fn wobble_phase(t: f32) -> f32 {
    (t.max(0.0) * HELL_FLASH_HZ * WOBBLE_STEP).rem_euclid(std::f32::consts::TAU)
}

/// The one WorldBrightness and ObjectBrightness the cheat forces on the whole scene (`0x439e84`): Hell's red (#316) or
/// the underwater teal (#317); `None` = the track's own and the user's Brightness.
pub fn brightness(cheat: Option<NameCheat>) -> Option<[f32; 3]> {
    match cheat {
        Some(NameCheat::Hell) => Some(HELL_BRIGHTNESS),
        Some(NameCheat::Underwater) => Some(UNDERWATER_BRIGHTNESS),
        _ => None,
    }
}

/// Whether the mode is lemmy / ace, Hell mode.
pub fn hell(cheat: Option<NameCheat>) -> bool {
    cheat == Some(NameCheat::Hell)
}

/// Whether the 1/30 s tick holding `t` seconds of race time is a lightning flash: a hashed 15-bit value of the
/// tick in the original's window, 399 of 32768 (1.22 %). Deterministic, so shots and replays agree.
pub fn hell_flash(t: f32) -> bool {
    let tick = (t.max(0.0) * HELL_FLASH_HZ).floor() as u32;
    let mut h = tick.wrapping_mul(0x41c6_4e6d).wrapping_add(0x3039);
    h ^= h >> 15;
    h = h.wrapping_mul(0x2c1b_3c6d);
    h ^= h >> 12;
    h = h.wrapping_mul(0x297a_2d39);
    h ^= h >> 15;
    let r = h & 0x7fff;
    r > FLASH_LO && r < FLASH_HI
}

/// tribute to tron (#320, 08 §8.14): the Glide-only Tron look (`0x48e964`). The lit colour of a polygon becomes pure
/// blue, b = (R + G + B) · `TRON_FILL` (`0x4d8f26`).
pub const TRON_FILL: f32 = 0.33;
/// The colour-keyed (alpha-tested) textures' fill is dimmed once more (`0x4d8f66`) and gets no outline.
pub const TRON_KEYED: f32 = 0.25;
/// The scroller's text (`0x4cacc8`, 15 lines of 0x3c bytes): five empty lines lead in.
pub const TRON_TEXT: [&str; 15] = [
    "",
    "",
    "",
    "",
    "",
    "Hello there",
    "Here is something for all the oldskool people out there:",
    "A tribute to the classic movie TRON!",
    "Enjoy the look and feel of the very early 80's --",
    "when hidden-lines were 'in'",
    "and never forget your roots!",
    "",
    "Regards,",
    "all the former demosceners at Digital Illusions",
    "",
];

/// The scroller's band starts at this race-init time (s, `0x48f810`) [O].
pub const TRON_BAND_START: f32 = 3.0;
/// Seconds per scrolled line; the block moves up one line in the first fifth of it (`0x4d8f4e`) [O].
pub const TRON_STEP: f32 = 4.0;
/// Lines on screen at once (`first .. first + 5`).
pub const TRON_LINES: usize = 5;
/// The band's soft top edge (alpha 0) and where it turns solid, as fractions of the screen height (`0x4d8f36`, `0x4d8f3e`).
pub const TRON_BAND_Y0: f32 = 0.88;
pub const TRON_BAND_Y1: f32 = 0.90;
/// First text line's top and the line pitch, fractions of the screen height (`0x4d8f6e`, `0x4d8f56`) in the original's 512 x 384 layout.
pub const TRON_TEXT_Y0: f32 = 0.77;
pub const TRON_TEXT_PITCH: f32 = 0.05;
/// The text's left edge and size in that layout: x = 25 px of 384 lines high, myfont's 12 px cell.
pub const TRON_TEXT_X: f32 = 25.0;
pub const TRON_REF_H: f32 = 384.0;

/// What the scroller shows at race-init time `t` (`0x48f810..0x48fc83`).
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct TronScroll {
    /// The band's alpha 0..1
    pub alpha: f32,
    /// Whether the text shows (4 < t < 52)
    pub text: bool,
    /// The first of the `TRON_LINES` lines (of `TRON_TEXT`; lines past its end are empty)
    pub first: usize,
    /// How far the block still has to rise (1 just after a step, 0 when it rests): lines sit `f` pitches lower
    pub f: f32,
}

/// The scroller at `t` seconds after the race init, `None` outside the band's 3 .. 53 s.
pub fn tron_scroll(t: f32) -> Option<TronScroll> {
    let end = (TRON_TEXT.len() - 3) as f32 * TRON_STEP + 5.0;
    if t <= TRON_BAND_START || t >= end {
        return None;
    }
    let alpha = if t < TRON_BAND_START + 1.0 { t - TRON_BAND_START } else if t > end - 1.0 { end - t } else { 1.0 };
    let text = t > TRON_BAND_START + 1.0 && t < end - 1.0;
    let p = ((t - (TRON_BAND_START + 1.0)) / TRON_STEP).max(0.0);
    let first = p.floor() as usize;
    let f = (1.0 - 5.0 * p.fract()).max(0.0);
    Some(TronScroll { alpha: alpha.clamp(0.0, 1.0), text, first, f })
}

/// Whether the mode is tribute to tron, the Tron look.
pub fn tron(cheat: Option<NameCheat>) -> bool {
    cheat == Some(NameCheat::Tron)
}

/// r pettersson / swe (`0x461698`): all tracks and cars unlocked in memory, nothing written (#310).
pub fn unlocks_all(name: &str, team: &str) -> bool {
    eq_n(name, "r pettersson", NAME_LEN) && eq_n(team, "swe", TEAM_LEN)
}

/// avenger / zx (`0x412aa4`, compare `0x49235e`): the racer's minimap dot is a ninja (#322). Not
/// part of the flag word; the whole string, case ignored [I].
pub fn ninja_marker(name: &str, team: &str) -> bool {
    name.eq_ignore_ascii_case("avenger") && team.eq_ignore_ascii_case("zx")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hell_flash_rate_matches_the_original() {
        let n = 200_000;
        let hits = (0..n).filter(|&i| hell_flash(i as f32 / HELL_FLASH_HZ + 0.001)).count();
        let rate = hits as f32 / n as f32;
        assert!((rate - 399.0 / 32768.0).abs() < 0.003, "rate {rate}");
        // deterministic, and constant within a tick
        assert_eq!(hell_flash(5.01), hell_flash(5.02));
        assert!(hell(NameCheat::of("lemmy", "ace")) && !hell(None));
    }

    #[test]
    fn underwater_slows_the_race_and_wobbles() {
        let c = NameCheat::of("Ramlosa", "H2O");
        assert!(underwater(c) && !underwater(None) && !hell(c));
        assert_eq!(time_scale(c), 0.5);
        assert_eq!(time_scale(None), 1.0);
        assert_eq!(time_scale(NameCheat::of("lemmy", "ace")), 1.0);
        assert_eq!(brightness(c), Some(UNDERWATER_BRIGHTNESS));
        assert_eq!(brightness(NameCheat::of("lemmy", "ace")), Some(HELL_BRIGHTNESS));
        assert_eq!(brightness(None), None);
        // 1.5 rad/s, wrapping at 2 pi
        assert!((wobble_phase(1.0) - 1.5).abs() < 1e-5);
        assert!(wobble_phase(100.0) < std::f32::consts::TAU);
    }

    #[test]
    fn every_pair_finds_its_cheat() {
        for (n, t, c) in PAIRS {
            assert_eq!(NameCheat::of(n, t.unwrap_or("")), Some(c), "{n}");
        }
    }

    #[test]
    fn case_is_ignored() {
        assert_eq!(NameCheat::of("Buzz Aldrin", "NASA"), Some(NameCheat::MoonGravity));
        assert_eq!(NameCheat::of("G-RIDE", "West"), Some(NameCheat::BouncyCars));
    }

    #[test]
    fn only_the_first_four_of_the_team_count() {
        assert_eq!(NameCheat::of("demon", "Gremlin"), Some(NameCheat::BouncyWheels));
        assert_eq!(NameCheat::of("buzz aldrin", "nasa team"), Some(NameCheat::MoonGravity));
        assert_eq!(NameCheat::of("demon", "gre"), None);
    }

    #[test]
    fn a_longer_or_shorter_name_does_not_match() {
        assert_eq!(NameCheat::of("demons", "grem"), None);
        assert_eq!(NameCheat::of("demo", "grem"), None);
    }

    #[test]
    fn grem_is_told_apart_by_the_name() {
        assert_eq!(NameCheat::of("demon", "grem"), Some(NameCheat::BouncyWheels));
        assert_eq!(NameCheat::of("supercars", "grem"), Some(NameCheat::Supercars));
        assert_eq!(NameCheat::of("supercars", "west"), None);
    }

    #[test]
    fn tron_needs_no_team() {
        assert_eq!(NameCheat::of("tribute to tron", ""), Some(NameCheat::Tron));
        assert_eq!(NameCheat::of("Tribute To Tron", "anything"), Some(NameCheat::Tron));
    }

    #[test]
    fn no_match_and_empty() {
        assert_eq!(NameCheat::of("", ""), None);
        assert_eq!(NameCheat::of("Player", "DICE"), None);
        assert_eq!(NameCheat::of("demon", "west"), None);
    }

    #[test]
    fn the_name_is_cut_at_sixteen() {
        // "tribute to tron" is 15 characters; a 16th one still differs, a longer tail never counts
        assert_eq!(NameCheat::of("tribute to trone", ""), None);
    }

    #[test]
    fn ignition_camera_and_levels() {
        assert!(ignition(NameCheat::of("IGNITION", "UDS")));
        assert!(!ignition(NameCheat::of("ignition", "ace")));
        assert!(!ignition(None));
        assert_eq!(car_level(NameCheat::of("ignition", "uds")), Some(2));
        assert_eq!(car_level(NameCheat::of("supercars", "grem")), Some(3));
        assert_eq!(car_level(NameCheat::of("demon", "grem")), None);
        assert_eq!(car_level(None), None);
    }

    #[test]
    fn only_supercars_is_top_down() {
        assert!(top_down(NameCheat::of("Supercars", "GREM")));
        assert!(!top_down(NameCheat::of("demon", "grem")));
        assert!(!top_down(None));
    }

    #[test]
    fn moon_gravity_is_half_and_only_for_nasa() {
        assert_eq!(gravity(NameCheat::of("buzz aldrin", "nasa")), 7.905);
        assert_eq!(gravity(NameCheat::of("demon", "grem")), crate::world::GRAVITY);
        assert_eq!(gravity(None), crate::world::GRAVITY);
    }

    #[test]
    fn bouncy_wheels_soften_the_suspension_only_for_demon_and_gride() {
        assert_eq!(suspension(NameCheat::of("Demon", "Gremlin")), (3.0, 0.16, 21666.0));
        assert_eq!(suspension(NameCheat::of("G-Ride", "Western")), (1.2, 0.5, -50000.0));
        assert_eq!(suspension(NameCheat::of("supercars", "grem")), (1.0, 1.0, 65000.0));
        assert_eq!(suspension(NameCheat::of("buzz aldrin", "nasa")), (1.0, 1.0, 65000.0));
        assert_eq!(suspension(None), (1.0, 1.0, 65000.0));
    }

    #[test]
    fn tron_flag_and_text() {
        assert!(tron(NameCheat::of("Tribute To TRON", "")));
        assert!(tron(NameCheat::of("tribute to tron", "any team")));
        assert!(!tron(NameCheat::of("tribute to", "tron")));
        assert!(!tron(NameCheat::of("lemmy", "ace")));
        assert_eq!(TRON_TEXT.len(), 15);
        assert!(TRON_TEXT.iter().all(|l| l.len() < 0x3c));
    }

    #[test]
    fn tron_scroller_timeline() {
        assert_eq!(tron_scroll(2.9), None);
        assert_eq!(tron_scroll(53.1), None);
        let s = tron_scroll(3.5).unwrap();
        assert!((s.alpha - 0.5).abs() < 1e-5 && !s.text);
        let s = tron_scroll(4.4).unwrap();
        assert_eq!((s.alpha, s.text, s.first), (1.0, true, 0));
        assert!((s.f - (1.0 - 5.0 * 0.1)).abs() < 1e-5);
        let s = tron_scroll(8.0).unwrap(); // a step has just happened
        assert_eq!(s.first, 1);
        assert!((s.f - 1.0).abs() < 1e-5);
        let s = tron_scroll(9.0).unwrap(); // at rest
        assert_eq!((s.first, s.f), (1, 0.0));
        let s = tron_scroll(51.9).unwrap();
        assert!(s.text && s.first == 11);
        let s = tron_scroll(52.5).unwrap();
        assert!((s.alpha - 0.5).abs() < 1e-5 && !s.text);
    }

    #[test]
    fn pettersson_and_avenger_have_their_own_checks() {
        assert!(unlocks_all("R Pettersson", "SWE"));
        assert!(!unlocks_all("r pettersson", "Sweden")); // 4 characters compared against the 3-letter "swe"
        assert!(!unlocks_all("r pettersson", "nasa"));
        assert_eq!(NameCheat::of("r pettersson", "swe"), None);
        assert!(ninja_marker("Avenger", "ZX"));
        assert!(!ninja_marker("avenger", "zxy"));
        assert!(!ninja_marker("avenger", ""));
        assert_eq!(NameCheat::of("avenger", "zx"), None);
    }
}
