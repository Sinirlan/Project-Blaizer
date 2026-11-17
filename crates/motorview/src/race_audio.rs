//! Race sound: the car you hear (engine, tyres, hits, cues), the other cars, the track
//! ambience, heard from a listener (`Ear`). Shared by the game (bin motorhead audio.rs) and the
//! cine render (cine_render.rs). Spec: re-notes/07-audio.md, 20-cine.md "Audio".

use crate::drive::Session;
use crate::loading::{Progress, Stage};
use crate::render::Camera;
use glam::Vec3;
use motor_audio::{wav, Audio, Clip, Params, Tone, VoiceId};
use motor_sim::session::Race;
use motor_sim::world::MatSound;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

motor_tune::tunable! {
    /// Engine pitch slope: pitch = rpm x this + `PITCH_BASE` (0x483efe) [O].
    /// range: 0..0.001
    /// area: Audio/Engine
    PITCH_PER_RPM: f32 = 0.00017;
    /// Engine pitch at 0 rpm (0x483efe) [O].
    /// range: 0..1
    /// area: Audio/Engine
    PITCH_BASE: f32 = 0.31746;
}
/// Load-time volumes of the six engine samples (0x483630..0x48379d: 0x4c490c = 1.5,
/// 0x4c4910 = 1.0, 0x4c4914 = 0.9, the R channels x0.8 where noted).
const ENGINE_SAMPLES: [(&str, f32, f32); 6] = [
    ("gason.wav", 1.5, 0.0),
    ("gasrele.wav", 1.0, 0.0),
    ("gasotl.wav", 0.9, -1.0),
    ("gasotr.wav", 0.8, 1.0),
    ("gasrtl.wav", 0.9, -1.0),
    ("gasrtr.wav", 0.9, 1.0),
];
motor_tune::tunable! {
    /// Loop-sample boost relative to the one-shots (0x4d83a7 = 1.8) [O].
    /// range: 0..4
    /// area: Audio/Engine
    LOOP_GAIN: f32 = 1.8;
    /// Pitch slope of the other cars' engines (0x4d835f; the car you hear: `PITCH_PER_RPM`) [O].
    /// range: 0..0.001
    /// area: Audio/Engine
    AI_PITCH_PER_RPM: f32 = 0.00016512;
}

/// REMASTER: the engine sound (the `EngineSound` setting). The engine-sim Baked / Live engines were tried and
/// removed (#493, re-notes/09-remaster.md 9.19); Enhanced (#494) is the game's own samples, cleaner.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum EngineSound {
    /// the game's engine recordings through the original linear mixer
    #[default]
    Classic,
    /// the same recordings, band-limited resampling and seamless loops (`motor_audio::resample`), smoothed blends, a
    /// character per car and the body / air layers (#494)
    Enhanced,
}

impl EngineSound {
    pub const ALL: [EngineSound; 2] = [EngineSound::Classic, EngineSound::Enhanced];

    /// the settings file's word
    pub fn key(self) -> &'static str {
        match self {
            EngineSound::Classic => "Classic",
            EngineSound::Enhanced => "Enhanced",
        }
    }

    /// the menu's word
    pub fn label(self) -> &'static str {
        self.key()
    }

    /// a settings word or command-line value; the removed `simulated` / `baked` / `live` fall back to Classic
    pub fn parse(v: &str) -> Option<EngineSound> {
        match v.to_ascii_lowercase().as_str() {
            "classic" | "simulated" | "baked" | "live" => Some(EngineSound::Classic),
            "enhanced" => Some(EngineSound::Enhanced),
            _ => None,
        }
    }
}
motor_tune::tunable! {
    /// REMASTER Enhanced engine: length of the crossfade that closes each engine loop's seam [design choice].
    /// range: 0..100
    /// unit: ms
    /// applies: reload
    /// area: Audio/Enhanced
    SEAM_XFADE_MS: f32 = 20.0;
    /// REMASTER Enhanced engine: how far back from a loop's end the seam search looks for the best match with its start [design choice].
    /// range: 0..300
    /// unit: ms
    /// applies: reload
    /// area: Audio/Enhanced
    SEAM_SEARCH_MS: f32 = 60.0;
    /// REMASTER Enhanced engine: time constant with which each engine-family voice glides to a new volume, pan and pitch,
    /// so the per-frame set leaves no steps [design choice].
    /// range: 0..100
    /// unit: ms
    /// applies: reload
    /// area: Audio/Enhanced
    ENGINE_GLIDE_MS: f32 = 10.0;
    /// REMASTER Enhanced engine: how fast the throttle recording (gason) swells in when the pedal goes down [design choice].
    /// range: 1..500
    /// unit: ms
    /// area: Audio/Enhanced
    PEDAL_ATTACK_MS: f32 = 40.0;
    /// REMASTER Enhanced engine: how fast the overrun recordings (gasrele) take over when the pedal is lifted [design choice].
    /// range: 1..1000
    /// unit: ms
    /// area: Audio/Enhanced
    PEDAL_RELEASE_MS: f32 = 120.0;
    /// REMASTER Enhanced engine: smoothing of the echo-zone channel `b` that fades the left/right echo loops in and out [design choice].
    /// range: 1..1000
    /// unit: ms
    /// area: Audio/Enhanced
    ECHO_BLEND_MS: f32 = 150.0;
    /// REMASTER Enhanced engine: how far the engine level dips at a gear change (0 = none, 1 = silent) [design choice].
    /// range: 0..1
    /// area: Audio/Enhanced
    GEAR_DIP: f32 = 0.35;
    /// REMASTER Enhanced engine: how long the gear-change dip takes to fade (time constant) [design choice].
    /// range: 1..500
    /// unit: ms
    /// area: Audio/Enhanced
    GEAR_DIP_MS: f32 = 90.0;
    /// REMASTER Enhanced engine: how much of each car's character (assets/audio/engine_character.cfg) is applied:
    /// 0 = every car as the game's, 1 = the table, 2 = twice its offsets [design choice].
    /// range: 0..2
    /// area: Audio/Enhanced
    CHARACTER: f32 = 1.0;
    /// REMASTER Enhanced engine: corner of the per-car low shelf (the engine's body) [design choice].
    /// range: 40..1000
    /// unit: Hz
    /// area: Audio/Enhanced
    TONE_LOW_HZ: f32 = 250.0;
    /// REMASTER Enhanced engine: corner of the per-car high shelf (the engine's rasp) [design choice].
    /// range: 1000..10000
    /// unit: Hz
    /// area: Audio/Enhanced
    TONE_HIGH_HZ: f32 = 3000.0;
    /// REMASTER Enhanced engine (#494 phase 4): level of the "body" layer, the throttle and overrun recordings played
    /// one octave down under the engine, relative to the recording it doubles (-60 or less = off) [design choice,
    /// chosen by the listening kit: "subtle" ranked first for cars 1 and 8, 4b].
    /// range: -60..12
    /// unit: dB
    /// area: Audio/Enhanced
    BODY_DB: f32 = -9.0;
    /// REMASTER Enhanced engine: the body layer keeps only what lies below this (its low end) [design choice].
    /// range: 60..2000
    /// unit: Hz
    /// area: Audio/Enhanced
    BODY_LP_HZ: f32 = 400.0;
    /// REMASTER Enhanced engine: level of the "air" layer, the throttle recording's own combustion noise (its
    /// harmonics turned to noise, its pulse kept), relative to the noise the recording carries (-60 or less = off)
    /// [design choice, chosen by the listening kit: "subtle" ranked first for cars 1 and 8, 4b].
    /// range: -60..12
    /// unit: dB
    /// area: Audio/Enhanced
    AIR_DB: f32 = -6.0;
    /// REMASTER Enhanced engine: the air layer keeps only what lies above this (its hiss) [design choice].
    /// range: 200..8000
    /// unit: Hz
    /// area: Audio/Enhanced
    AIR_HP_HZ: f32 = 1200.0;
    /// REMASTER Enhanced engine: share of the air layer at 0 rpm; it grows to full at 12 000 rpm [design choice].
    /// range: 0..1
    /// area: Audio/Enhanced
    AIR_IDLE: f32 = 0.3;
    /// REMASTER Enhanced engine: share of the air layer with the pedal released (full with it pressed) [design choice].
    /// range: 0..1
    /// area: Audio/Enhanced
    AIR_OFF: f32 = 0.5;
    /// REMASTER Enhanced engine: what the air layer is built from: the throttle recording above this, in the
    /// recording's own frequencies [design choice].
    /// range: 0..4000
    /// unit: Hz
    /// applies: reload
    /// area: Audio/Enhanced
    AIR_CUT_HZ: f32 = 600.0;
}

/// REMASTER Enhanced engine: the body layer's high-pass, so the octave-down 43 Hz rumble of the recordings does not
/// pump the mix [design choice].
const BODY_HP_HZ: f32 = 30.0;

/// A level in dB as a volume factor; -60 dB or less is silence (the mixer then skips the voice).
fn db_gain(db: f32) -> f32 {
    if db <= -60.0 {
        0.0
    } else {
        10f32.powf(db / 20.0)
    }
}

/// REMASTER Enhanced engine (#494 phase 3): a car's engine character, one line of
/// assets/audio/engine_character.cfg (`carNN pitch low_db high_db level_db body_db air_db`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Character {
    /// factor on the engine's pitch
    pub pitch: f32,
    /// gains of the low / high shelf
    pub low_db: f32,
    pub high_db: f32,
    /// the engine's level
    pub level_db: f32,
    /// added to `BODY_DB` / `AIR_DB` for this car (#496)
    pub body_db: f32,
    pub air_db: f32,
}

impl Character {
    /// the game's engine, unchanged
    pub const NEUTRAL: Character = Character { pitch: 1.0, low_db: 0.0, high_db: 0.0, level_db: 0.0, body_db: 0.0, air_db: 0.0 };

    /// The offsets times `k` (0 = neutral).
    fn scaled(self, k: f32) -> Character {
        Character { pitch: self.pitch.max(0.01).powf(k), low_db: self.low_db * k, high_db: self.high_db * k, level_db: self.level_db * k, body_db: self.body_db * k, air_db: self.air_db * k }
    }

    /// the level as a volume factor
    pub fn level(self) -> f32 {
        10f32.powf(self.level_db / 20.0)
    }

    /// the voice's tone, at the shelf corners of the tunables
    pub fn tone(self) -> Tone {
        Tone { low_hz: TONE_LOW_HZ.get(), low_db: self.low_db, high_hz: TONE_HIGH_HZ.get(), high_db: self.high_db, ..Default::default() }
    }
}

/// The per-car table, built into the game.
const CHARACTER_TABLE: &str = include_str!("../../../assets/audio/engine_character.cfg");

/// `carNN pitch low_db high_db level_db body_db air_db` lines by car number; `//` comments, bad lines skipped.
fn parse_characters(text: &str) -> HashMap<u32, Character> {
    text.lines()
        .filter_map(|l| {
            let l = l.split("//").next().unwrap_or("").trim();
            let mut w = l.split_whitespace();
            let car = w.next()?.strip_prefix("car")?.parse().ok()?;
            let v: Vec<f32> = w.map(str::parse).collect::<Result<_, _>>().ok()?;
            let [pitch, low_db, high_db, level_db, body_db, air_db] = v[..] else { return None };
            Some((car, Character { pitch, low_db, high_db, level_db, body_db, air_db }))
        })
        .collect()
}

/// REMASTER Enhanced engine: the character of car `car_id` (the carNN number), scaled by `CHARACTER`.
pub fn character(car_id: u32) -> Character {
    static TABLE: std::sync::OnceLock<HashMap<u32, Character>> = std::sync::OnceLock::new();
    let table = TABLE.get_or_init(|| parse_characters(CHARACTER_TABLE));
    let c = character_overrides().lock().ok().and_then(|o| o.get(&car_id).copied()).or_else(|| table.get(&car_id).copied());
    c.unwrap_or(Character::NEUTRAL).scaled(CHARACTER.get())
}

/// Character lines set from the command line (`--engine-character`), tried before the built-in table.
fn character_overrides() -> &'static std::sync::Mutex<HashMap<u32, Character>> {
    static O: std::sync::OnceLock<std::sync::Mutex<HashMap<u32, Character>>> = std::sync::OnceLock::new();
    O.get_or_init(Default::default)
}

/// Dev tool (`--engine-character "car10 1.08 1.5 3.5 1.5 3 2"`, repeatable): replace cars' character lines for this run,
/// to try a sound without a rebuild (#515). Unlike the built-in table, a line that does not parse is an error. Returns the
/// number of cars set.
pub fn override_characters(text: &str) -> Result<usize, String> {
    let mut n = 0;
    for l in text.lines().map(|l| l.split("//").next().unwrap_or("").trim()).filter(|l| !l.is_empty()) {
        let one = parse_characters(l);
        let Some((car, c)) = one.into_iter().next() else {
            return Err(format!("--engine-character: `carNN pitch low_db high_db level_db body_db air_db`, not `{l}`"));
        };
        character_overrides().lock().map_err(|e| e.to_string())?.insert(car, c);
        n += 1;
    }
    Ok(n)
}

/// slide1.wav's loop (0x483596: start 9952, end 1 = to the sample's end [I]).
const SLIDE1_LOOP: Option<(usize, usize)> = Some((9952, usize::MAX));
motor_tune::tunable! {
    /// World hit impulse from which SndHardImpulse plays instead of SndWeakImpulse (patch 3.0 0x5140da;
    /// the 1998 game had 16500, 0x4d83e7); the weak sound's volume is `volume x impulse / this x HIT_GAIN`
    /// (0x5140e2) [O].
    /// range: 1000..60000
    /// unit: kg m/s
    /// area: Audio/Hits
    HARD_IMPULSE: f32 = 18500.0;
    /// Level factor of the hit sounds (0x4c4900, 0x4c4904) [O].
    /// range: 0..2
    /// area: Audio/Hits
    HIT_GAIN: f32 = 0.7;
    /// Extra gain of the `ObjectEnvSound` voices over the `EnvSound` points [design choice: at the
    /// .esd volumes the passing aircraft were barely audible next to the engines].
    /// range: 0..4
    /// area: Audio/Hits
    OBJECT_GAIN: f32 = 1.0;
    /// Stereo offset of a scrape at the left (hull points 0, 3) or right of the car (0x4844f7) [O].
    /// range: 0..1
    /// area: Audio/Hits
    SCRAPE_PAN: f32 = 0.7;
    /// Overall level: the game's sample volumes go up to 1.8, so keep headroom. Read when the
    /// mixer is made.
    /// range: 0..1.5
    /// applies: restart
    /// area: Audio/Engine
    pub MASTER: f32 = 0.45;
    /// REMASTER cine: the car you hear is at full level up to this distance when the camera is
    /// outside it (the other cars: 10..100) [design choice, tuned by ear].
    /// range: 0..100
    /// unit: m
    /// area: Audio/Cine
    FOCUS_NEAR: f32 = 10.0;
    /// REMASTER cine: the car you hear is silent beyond this distance when the camera is outside it
    /// [design choice, tuned by ear].
    /// range: 1..500
    /// unit: m
    /// area: Audio/Cine
    FOCUS_FAR: f32 = 150.0;
    /// REMASTER cine: the level `place` gives a source straight ahead (spread 0.6 / 0.6), so the car
    /// you hear keeps its game level when it is close in front of the camera.
    /// range: 0.05..1
    /// area: Audio/Cine
    AHEAD_LEVEL: f32 = 0.6;
    /// REMASTER cine Doppler: the speed of sound (game units are metres).
    /// range: 50..1000
    /// unit: m/s
    /// area: Audio/Cine
    SOUND_SPEED: f32 = 343.0;
    /// REMASTER cine Doppler: the smoothing time of the approach speed.
    /// range: 0.001..1
    /// unit: s
    /// area: Audio/Cine
    DOPPLER_SMOOTH: f32 = 0.05;
    /// REMASTER cine Doppler: the lowest pitch factor.
    /// range: 0.1..1
    /// area: Audio/Cine
    DOPPLER_MIN: f32 = 0.5;
    /// REMASTER cine Doppler: the highest pitch factor.
    /// range: 1..4
    /// area: Audio/Cine
    DOPPLER_MAX: f32 = 2.0;
}

/// PATCH30 0x4b0a8c: a material sound's clip as the patch 3.0 DirectSound driver loads it (#503: the tyre, wall-hit
/// and bump sounds follow patch 3.0 whatever the install, 07-audio "Patch 3.0 car sound update"). The record's
/// `loopstart loopend` are frames cut from the sample's head and tail (each below 100 counts as 0, both are
/// clamped to the length; when the tail would reach into the head the two swap; when they cover the sample
/// exactly both become 0), and a looping record loops all that is left. Returns the clip and the tail used,
/// or None when the cuts leave nothing (the driver refuses the record).
fn trim_p30(clip: Clip, (ls, le): (usize, usize)) -> Option<(Clip, usize)> {
    let n = clip.data.len();
    let (mut ls, mut le) = (if ls < 100 { 0 } else { ls.min(n) }, if le < 100 { 0 } else { le.min(n) });
    if n - le < ls {
        std::mem::swap(&mut ls, &mut le);
    }
    if n - le == ls {
        (ls, le) = (0, 0);
    }
    if ls + le >= n {
        return None;
    }
    let data = if (ls, le) == (0, 0) { clip.data.clone() } else { std::sync::Arc::new(clip.data[ls..n - le].to_vec()) };
    Some((Clip { data, loop_points: None, ..clip }, le))
}

pub fn engine_pitch(rpm: f32) -> f32 {
    (rpm * PITCH_PER_RPM.get() + PITCH_BASE.get()).max(0.05)
}

/// The engine's on/off levels (0x483f16, 0x484645): only gason while the driving pedal P+0xf0 is
/// down; released, gason fades into gasrele (x 0.9) from 6000 to 12000 rpm. In-car both x 0.9.
// CLASSIC 0x483d4c
fn engine_levels(pedal: f32, rpm: f32, in_car: bool) -> (f32, f32) {
    let (on, off) = if pedal > 0.0 {
        (1.0, 0.0)
    } else if rpm >= 12000.0 {
        (0.0, 0.9)
    } else {
        let x = ((rpm - 6000.0) / 6000.0).max(0.0);
        (1.0 - x, 0.9 * x)
    };
    let k = if in_car { 0.9 } else { 1.0 };
    (on * k, off * k)
}

/// Volumes of `ENGINE_SAMPLES` from the levels and the loop fade `b` (0x483f85..0x48405c).
// CLASSIC 0x483d4c
fn engine_volumes(on: f32, off: f32, b: f32) -> [f32; 6] {
    let b = b.clamp(0.0, 1.0);
    let (lon, loff) = (b * on * LOOP_GAIN.get(), b * off * LOOP_GAIN.get());
    [(1.0 - b) * on, (1.0 - b) * off, lon, lon, loff, loff]
}

/// REMASTER Enhanced engine: the six engine volumes with the hard edges of `engine_levels` / `engine_volumes`
/// smoothed. Steady, it gives what they give (the pitch law and the end states are the game's); between, the pedal
/// gate and the echo channel glide, the two recordings of a fade mix in equal power (no hole in the middle), and a
/// gear change dips the level for a moment (it hides the rpm jump).
#[derive(Clone, Debug, Default)]
pub struct EngineBlend {
    /// 1 = the driving pedal is down (gason), 0 = released (the overrun mix), smoothed
    gate: f32,
    /// the echo channel `b`, smoothed
    echo: f32,
    /// the gear-change dip, 1 at the shift, decaying
    dip: f32,
    gear: usize,
    started: bool,
    /// what the last `step` mixed, for the layers (`levels`)
    out: BlendLevels,
}

/// REMASTER Enhanced engine: the levels of the last `EngineBlend::step`, before the echo split.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct BlendLevels {
    /// the throttle (gason) and overrun (gasrele) levels, the in-car and gear-dip factors included
    pub on: f32,
    pub off: f32,
    /// the pedal gate (1 pressed, 0 released, glided)
    pub gate: f32,
    /// the in-car and gear-dip factors alone
    pub scale: f32,
}

/// One-pole step of `x` towards `to` over `dt` seconds with time constant `ms`.
fn glide(x: f32, to: f32, ms: f32, dt: f32) -> f32 {
    let tau = ms * 0.001;
    if tau <= 0.0 || dt <= 0.0 { if dt > 0.0 { to } else { x } } else { x + (to - x) * (1.0 - (-dt / tau).exp()) }
}

impl EngineBlend {
    /// Volumes of `ENGINE_SAMPLES` this frame (`dt` seconds after the last).
    // REMASTER
    pub fn step(&mut self, pedal: f32, rpm: f32, in_car: bool, echo: f32, gear: usize, dt: f32) -> [f32; 6] {
        let pressed = if pedal > 0.0 { 1.0 } else { 0.0 };
        let echo = echo.clamp(0.0, 1.0);
        if !self.started {
            *self = EngineBlend { gate: pressed, echo, dip: 0.0, gear, started: true, out: BlendLevels::default() };
        }
        let ms = if pressed > self.gate { PEDAL_ATTACK_MS.get() } else { PEDAL_RELEASE_MS.get() };
        self.gate = glide(self.gate, pressed, ms, dt);
        self.echo = glide(self.echo, echo, ECHO_BLEND_MS.get(), dt);
        // a shift between two forward gears dips the level; reverse (0) and the first start do not
        if gear != self.gear {
            if gear > 0 && self.gear > 0 {
                self.dip = 1.0;
            }
            self.gear = gear;
        } else {
            self.dip = glide(self.dip, 0.0, GEAR_DIP_MS.get(), dt);
        }
        // the pressed and the released levels mixed in energy
        let ((on_p, off_p), (on_r, off_r)) = (engine_levels(1.0, rpm, in_car), engine_levels(0.0, rpm, in_car));
        let g = self.gate;
        let mix = |p: f32, r: f32| (g * p * p + (1.0 - g) * r * r).sqrt();
        let (on, off) = (mix(on_p, on_r), mix(off_p, off_r));
        // centre loops against the left/right echo loops in equal power
        let (c, e) = ((self.echo * std::f32::consts::FRAC_PI_2).cos(), (self.echo * std::f32::consts::FRAC_PI_2).sin() * LOOP_GAIN.get());
        let k = 1.0 - GEAR_DIP.get().clamp(0.0, 1.0) * self.dip;
        self.out = BlendLevels { on: on * k, off: off * k, gate: g, scale: k * if in_car { 0.9 } else { 1.0 } };
        [c * on * k, c * off * k, e * on * k, e * on * k, e * off * k, e * off * k]
    }

    /// The levels the last `step` mixed (all 0 before the first).
    pub fn levels(&self) -> BlendLevels {
        self.out
    }
}

/// A 3D record's distance gain: 1 inside `min`, 0 beyond `max`, a straight line between
/// (0x484a6b..0x484bf4).
// CLASSIC 0x484a00
fn falloff(d: f32, min: f32, max: f32) -> f32 {
    if d < min {
        1.0
    } else if d > max || max <= min {
        0.0
    } else {
        1.0 - (d - min) / (max - min)
    }
}

/// Left/right levels of a 3D voice from the source's offset in the listener's frame (`x` right,
/// `z` forward; the height is dropped and the rest normalised, 0x42b610): ahead 0.6 / 0.6,
/// behind 0.36 / 0.36, beside 0.8 near and 0.16 far (0x484aae..0x484af0).
// CLASSIC 0x484a00
fn spread(x: f32, z: f32) -> (f32, f32) {
    let n = (x * x + z * z).sqrt();
    // a source at the listener [I: 0x42b610's zero vector untraced]: as if ahead
    let (x, z) = if n > 1e-4 { (x / n, z / n) } else { (0.0, 1.0) };
    let a = 0.8 + 0.2 * z;
    (a * (0.6 - 0.4 * x), a * (0.6 + 0.4 * x))
}

/// A 2D record's linear pan: the far side fades, the near side stays (0x484bfd..0x484c6d).
// CLASSIC 0x484a00
fn pan_2d(pan: f32) -> (f32, f32) {
    (if pan > 0.0 { 1.0 - pan } else { 1.0 }, if pan < 0.0 { 1.0 + pan } else { 1.0 })
}

/// The original sets each channel (clamped to 0..1, 0x484b24); our mixer takes a volume and an
/// equal-power pan. Same left/right ratio, and L = R = v gives volume v at pan 0.
fn to_mixer(l: f32, r: f32) -> (f32, f32) {
    let (l, r) = (l.clamp(0.0, 1.0), r.clamp(0.0, 1.0));
    let volume = (l * l + r * r).sqrt() * std::f32::consts::FRAC_1_SQRT_2;
    let pan = if volume > 0.0 { r.atan2(l) / std::f32::consts::FRAC_PI_4 - 1.0 } else { 0.0 };
    (volume, pan)
}

/// The sound surface: of the wheels' materials the one whose SndContact priority is the
/// highest, the first wheel on a tie (0x484117..0x484140).
// CLASSIC 0x483d4c
fn sound_surface(wheels: &[Option<usize>; 4], mats: &[motor_sim::world::Material]) -> Option<usize> {
    let prio = |m: usize| mats.get(m).and_then(|m| m.sound("contact")).map_or(0.0, |s| s.priority);
    wheels.iter().flatten().copied().fold(None, |best, m| match best {
        Some(b) if prio(m) <= prio(b) => Some(b),
        _ => Some(m),
    })
}

/// Samples under Sounds/, loaded on first use (case-insensitive paths).
pub struct Bank {
    root: PathBuf,
    clips: HashMap<String, Option<Clip>>,
}

/// A sample's key in the `Bank`: its path under Sounds/, lower case with forward slashes.
fn bank_key(rel: &str) -> String {
    rel.to_ascii_lowercase().replace('\\', "/")
}

/// Decode Sounds/`key` (None, with a message, when it is missing or bad).
fn decode_sound(root: &Path, key: &str) -> Option<Clip> {
    motor_data::fs::read(root, &format!("Sounds/{key}")).ok().and_then(|b| match wav::decode(&b) {
        Ok(c) => Some(c),
        Err(e) => {
            eprintln!("sound {key}: {e}");
            None
        }
    })
}

impl Bank {
    pub fn new(root: &Path) -> Bank {
        Bank { root: root.to_path_buf(), clips: HashMap::new() }
    }

    pub fn get(&mut self, rel: &str) -> Option<Clip> {
        let key = bank_key(rel);
        if let Some(c) = self.clips.get(&key) {
            return c.clone();
        }
        let clip = decode_sound(&self.root, &key);
        self.clips.insert(key, clip.clone());
        clip
    }

    /// REMASTER Enhanced engine: the sample as a seamless loop read with the sinc kernel (built once, kept beside the original).
    pub fn get_enhanced(&mut self, rel: &str) -> Option<Clip> {
        let key = format!("{}#enhanced", bank_key(rel));
        if let Some(c) = self.clips.get(&key) {
            return c.clone();
        }
        let clip = self.get(rel).map(|c| motor_audio::resample::enhanced_loop(&c, SEAM_XFADE_MS.get(), SEAM_SEARCH_MS.get()));
        self.clips.insert(key, clip.clone());
        clip
    }

    /// REMASTER Enhanced engine (#494 phase 4): the air layer of a loop: noise with the colour and pulse of its
    /// enhanced version above `AIR_CUT_HZ`, the same length (so it plays in step with it). Built once.
    pub fn get_air(&mut self, rel: &str) -> Option<Clip> {
        let key = format!("{}#air", bank_key(rel));
        if let Some(c) = self.clips.get(&key) {
            return c.clone();
        }
        let clip = self.get_enhanced(rel).map(|c| Clip { data: std::sync::Arc::new(motor_audio::noise::noise_like(&c.data, c.rate, AIR_CUT_HZ.get(), 1)), ..c });
        self.clips.insert(key, clip.clone());
        clip
    }

    /// Take the samples a loading worker decoded (`race_sounds`); those already here stay.
    fn adopt(&mut self, clips: Vec<(String, Option<Clip>)>) {
        for (key, clip) in clips {
            self.clips.entry(key).or_insert(clip);
        }
    }
}

/// The race's one-shot and loop samples besides the engine's (the names `RaceAudio` plays).
const RACE_SAMPLES: [&str; 14] = [
    "turbo.wav", "inneen.wav", "enemyeng.wav", "eburst1.wav", "eburst2.wav", "gear3.wav", "finish2.wav", "barrel.wav", "crash2.wav", "majcrash.wav", "slide1.wav", "slidee1.wav", "slidee2.wav", "spin.wav",
];

/// #335 a race's sounds, decoded on the loading worker (`race_sounds`); `RaceAudio::adopt`
/// takes them when the race starts, so the first race frame decodes nothing.
pub struct RaceSounds {
    /// bank keys and their clips
    clips: Vec<(String, Option<Clip>)>,
}

/// The track's .esd ambience file (the .trk's EnvSoundName).
/// Music volume of the mixer's music slot (the game's CD music level, a design value shared with motorview).
pub const MUSIC_VOLUME: f32 = 1.15;

/// Index a folder of ripped CD tracks: "04 RedRock.mp3" -> 4. Any format symphonia reads.
pub fn index_music(root: &Path) -> HashMap<u32, PathBuf> {
    let mut map = HashMap::new();
    let Some(dir) = motor_data::fs::find_ci(root, "Music") else { return map };
    for e in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let p = e.path();
        let ok = p.extension().is_some_and(|x| ["mp3", "ogg", "flac", "wav"].iter().any(|k| x.eq_ignore_ascii_case(k)));
        let name = e.file_name().to_string_lossy().to_string();
        let digits: String = name.chars().take_while(|c| c.is_ascii_digit()).collect();
        if let (true, Ok(n)) = (ok, digits.parse::<u32>()) {
            map.insert(n, p);
        }
    }
    map
}

/// The `CDTrack` of .trk `track_n` (the audio CD track its race plays), None when the .trk names none.
pub fn cd_track(root: &Path, track_n: u32) -> Option<u32> {
    let c = motor_data::fs::config(root, &format!("Game/Track{track_n}.trk")).ok()?;
    c.get("CDTrack").and_then(|e| e.f32(0)).map(|n| n as u32).filter(|&n| n > 0)
}

fn track_esd(root: &Path, track_n: u32) -> Option<motor_data::cfg::Config> {
    let esd = motor_data::fs::config(root, &format!("Game/Track{track_n}.trk")).ok().and_then(|c| c.get("EnvSoundName").and_then(|e| e.values.first().cloned()))?;
    motor_data::fs::config(root, &esd).ok()
}

/// #335 decode what a race on .trk `track_n` plays (on the loading worker): the engine (the six original
/// loops), the race samples, the
/// .esd ambience and object sounds, the track materials' sounds. `p` reports the AUDIO stage.
pub fn race_sounds(root: &Path, track_n: u32, materials: &[motor_sim::world::Material], p: &Progress) -> Result<RaceSounds, String> {
    p.set(Stage::Audio, 0.0)?;
    let mut names: Vec<String> = RACE_SAMPLES.iter().map(|s| s.to_string()).collect();
    names.extend(ENGINE_SAMPLES.iter().map(|s| s.0.to_string()));
    if let Some(cfg) = track_esd(root, track_n) {
        names.extend(cfg.all("EnvSound").filter_map(|e| e.values.first().cloned()));
        names.extend(cfg.all("ObjectEnvSound").chain(cfg.all("ObjectColSound")).filter_map(|e| e.values.get(1).cloned()));
    }
    names.extend(materials.iter().flat_map(|m| m.sounds.iter().map(|s| s.1.sample.clone())));
    let mut keys: Vec<String> = names.iter().map(|n| bank_key(n)).collect();
    keys.sort();
    keys.dedup();
    let mut clips = Vec::with_capacity(keys.len());
    for (i, key) in keys.into_iter().enumerate() {
        let clip = decode_sound(root, &key);
        clips.push((key, clip));
        p.set(Stage::Audio, (i + 1) as f32 / clips.capacity() as f32)?;
    }
    Ok(RaceSounds { clips })
}

/// Track ambience line: `EnvSound sample priority volume pitch mindistance maxdistance x y z`,
/// or one object of an `ObjectEnvSound n …` line (`prop` set: the sound follows that scene
/// object, 0x425642 registers one per object with that id).
struct EnvSound {
    voice: VoiceId,
    pos: Vec3,
    /// index into `Props::props` for an object sound
    prop: Option<usize>,
    pitch: f32,
    min: f32,
    max: f32,
    volume: f32,
}

/// A material sound that loops while its hit lasts and plays out afterwards.
struct HitVoice {
    v: VoiceId,
    sample: String,
    p: Params,
    /// PATCH30 0x4b1921: seconds a released loop keeps looping before it is stopped (the cut tail's
    /// length at the voice's rate), and the countdown once released
    tail: f32,
    stop_in: Option<f32>,
}

/// The race's voices (built by `RaceAudio::start`).
struct Voices {
    engine: Vec<Option<VoiceId>>,
    /// REMASTER Enhanced engine: the smoothed pedal / echo / gear state of the volumes
    blend: EngineBlend,
    turbo: Option<VoiceId>,
    inside: Option<VoiceId>,
    ai: Vec<Option<VoiceId>>,
    /// REMASTER Enhanced engine layers (#494 phase 4): gason and gasrele an octave down, the air noise (Classic: none)
    body: Vec<Option<VoiceId>>,
    air: Option<VoiceId>,
    env: Vec<EnvSound>,
    /// the other cars' slide loops (P+0xa88 of 0x483b41), by racer
    ai_slide: Vec<Option<VoiceId>>,
    /// the tyre sound surface (a material index) and its SndSpin / SndSlide / SndContact voices
    /// (0x757078 / 0x7577a8 / 0x757a08)
    surface: Option<usize>,
    spin: Option<VoiceId>,
    slide: Option<VoiceId>,
    contact: Option<VoiceId>,
    /// slide1.wav (0x757010) and the slidee1/2 tail (0x7576e0 / 0x757744)
    slide1: Option<VoiceId>,
    tail: Option<VoiceId>,
    /// eburst1/2 (0x757280 / 0x7572e4); REMASTER the sides of a pack's pop
    burst: Vec<VoiceId>,
    /// SndScratch and SndWeakImpulse: shared looping records of 0x483d4c
    scrape: Option<HitVoice>,
    weak: Option<HitVoice>,
    /// the SndHardImpulse voice
    hard: Option<VoiceId>,
    last_gear: usize,
    last_count: i32,
    started: bool,
    last_cp: Option<(u32, u32)>,
    finished: bool,
    /// `ObjectColSound n sample priority volume pitch` of the track's .esd (0x48394c), by n
    body_hits: Vec<Option<(String, f32)>>,
    /// the shared voice of a body hit and the id it plays (0x757b40 / 0x757be8)
    body_hit: Option<(VoiceId, u16)>,
    /// 0x757bec: the alternation of 0x483390 (car-car hit, slide tail, exhaust burst)
    alt: u32,
    /// REMASTER cine Doppler: per source (racers, then `env`) the last distance and the smoothed
    /// approach speed (m/s, positive = moving away); None after a cut
    doppler: Vec<Option<(f32, f32)>>,
}

impl Voices {
    /// 0x483390: the counter steps by 0x139; its low bit picks the second of two samples.
    fn alternate(&mut self) -> bool {
        self.alt = self.alt.wrapping_add(0x139);
        self.alt & 1 == 1
    }
}

/// Where the race is heard from, per frame.
#[derive(Clone, Copy, Debug)]
pub struct Ear {
    /// the listener: the camera (view space, game z negated)
    pub cam: Camera,
    /// the car you hear (its engine, tyres, hits and cues)
    pub focus: usize,
    /// the in-car camera (0x756ce0): the car you hear is inside it
    pub in_car: bool,
    /// REMASTER: heard by a cine camera (None: the game)
    pub cine: Option<CineEar>,
}

/// REMASTER: a cine camera's ear (20-cine "Audio"). The car you hear is placed like the other
/// cars unless the camera is inside it, the player's cues are dropped, and Doppler is optional.
#[derive(Clone, Copy, Debug)]
pub struct CineEar {
    pub doppler: bool,
    /// the camera changed this frame (the Doppler history restarts)
    pub cut: bool,
}

/// The pitch factor of a source approaching at `-v` m/s (`v` > 0: moving away).
pub fn doppler_factor(v: f32) -> f32 {
    // faster than sound towards the ear (a jump): the top of the range, not a negative pitch
    let (lo, hi) = (DOPPLER_MIN.get(), DOPPLER_MAX.get());
    (SOUND_SPEED.get() / (SOUND_SPEED.get() + v).max(1.0)).clamp(lo.min(hi), hi.max(lo))
}

/// REMASTER cine: how the car you hear sounds from the camera, applied to its voices.
#[derive(Clone, Copy, Debug)]
struct FocusMix {
    gain: f32,
    pan: f32,
    pitch: f32,
}

/// The engine-family voices `RaceAudio::engine_family` starts.
struct EngineFamily {
    engine: Vec<Option<VoiceId>>,
    turbo: Option<VoiceId>,
    inside: Option<VoiceId>,
    ai: Vec<Option<VoiceId>>,
    /// REMASTER Enhanced: the body (gason, gasrele) and air layers; none in Classic
    body: Vec<Option<VoiceId>>,
    air: Option<VoiceId>,
}

/// REMASTER/CLASSIC race sound on a mixer: the game plays it live, the cine render offline.
pub struct RaceAudio {
    audio: Audio,
    bank: Bank,
    voices: Option<Voices>,
    install: PathBuf,
    /// REMASTER cine: the car you hear, placed (None in the game: heard as the player)
    focus_mix: Option<FocusMix>,
    /// REMASTER: which engine sound the next `start` builds (the setting; Classic in the cine render)
    pub engine_sound: EngineSound,
    /// user volumes 0..1: effects, track ambience
    pub sfx: f32,
    pub amb: f32,
}

impl RaceAudio {
    pub fn new(root: &Path, audio: Audio) -> RaceAudio {
        RaceAudio { audio, bank: Bank::new(root), voices: None, install: root.to_path_buf(), focus_mix: None, engine_sound: EngineSound::Classic, sfx: 0.8, amb: 0.8 }
    }

    /// Take the sounds a loading worker decoded (`race_sounds`), before `start`.
    pub fn adopt(&mut self, pre: RaceSounds) {
        self.bank.adopt(pre.clips);
    }

    /// The race's voices exist (between `start` and `stop`).
    pub fn active(&self) -> bool {
        self.voices.is_some()
    }

    fn play(&mut self, rel: &str, p: Params) -> Option<VoiceId> {
        let clip = self.bank.get(rel)?;
        Some(self.audio.play(&clip, p))
    }

    /// An engine-family loop (the six engine samples, turbo, interior hum, the other cars' engine): the original
    /// clip (CLASSIC) or, with `EngineSound::Enhanced`, its seamless sinc-read version (REMASTER).
    fn play_engine(&mut self, rel: &str, p: Params) -> Option<VoiceId> {
        let clip = match self.engine_sound {
            EngineSound::Classic => self.bank.get(rel)?,
            // REMASTER: the voice glides to its per-frame volume and pitch
            EngineSound::Enhanced => Clip { glide: ENGINE_GLIDE_MS.get() * 0.001, ..self.bank.get_enhanced(rel)? },
        };
        Some(self.audio.play(&clip, p))
    }

    /// A voice of the car you hear as the camera hears it (unchanged in the game).
    fn focus(&self, p: Params) -> Params {
        match self.focus_mix {
            None => p,
            Some(m) => Params { volume: p.volume * m.gain, pan: (p.pan + m.pan).clamp(-1.0, 1.0), pitch: p.pitch * m.pitch, ..p },
        }
    }

    /// `focus` for a (volume, pan) pair.
    fn focus_vp(&self, (volume, pan): (f32, f32)) -> (f32, f32) {
        let p = self.focus(Params { volume, pan, ..Default::default() });
        (p.volume, p.pan)
    }

    /// Keep a material sound looping at `want` = (volume, pan) while its hit lasts; without one
    /// the loop bit is cleared (`and 0xfe` on the record's flags) and the sample plays out.
    /// `looping` false = a one-shot started when not playing, its level followed while it plays
    /// (PATCH30 0x4b6f2b: the weak impulse). PATCH30 0x4b1921: a released loop goes on looping for
    /// its cut tail's length, then stops.
    fn drive_hit_loop(&mut self, slot: &mut Option<HitVoice>, snd: Option<&MatSound>, want: Option<(f32, f32)>, looping: bool, dt: f32) {
        let playing = slot.as_ref().is_some_and(|h| self.audio.with(|m| m.is_playing(h.v)));
        if !playing {
            *slot = None;
        }
        match (want, snd) {
            (Some((volume, pan)), Some(snd)) => {
                let p = Params { volume: volume.min(1.0), pitch: snd.pitch.max(0.1), pan, looping };
                // another material's sample replaces the record's (stopped first, 0x484d78)
                if slot.as_ref().is_some_and(|h| h.sample != snd.sample) {
                    if let Some(h) = slot.take() {
                        self.audio.stop(h.v);
                    }
                }
                if let Some(h) = slot {
                    h.p = p;
                    h.stop_in = None;
                    self.audio.set(h.v, Params { volume: p.volume * self.sfx, ..p });
                } else if let Some((clip, tail)) = self.mat_clip(snd) {
                    let tail = tail as f32 / (clip.rate.max(1) as f32 * p.pitch);
                    let v = self.audio.play(&clip, Params { volume: p.volume * self.sfx, ..p });
                    *slot = Some(HitVoice { v, sample: snd.sample.clone(), p, tail, stop_in: None });
                }
            }
            _ => {
                if let Some(h) = slot {
                    if h.p.looping {
                        let left = h.stop_in.get_or_insert(h.tail);
                        *left -= dt;
                        if *left <= 0.0 {
                            self.audio.stop(h.v);
                            *slot = None;
                        }
                    }
                }
            }
        }
    }

    /// A material record's clip, cut as the patch 3.0 driver loads it (`trim_p30`); the second value is the
    /// cut tail in frames.
    fn mat_clip(&mut self, snd: &MatSound) -> Option<(Clip, usize)> {
        trim_p30(self.bank.get(&snd.sample)?, snd.trim)
    }

    /// Keep a looping record going at `p` (0x484cf0: started when it does not play, else updated),
    /// at the effects volume; volumes above 1 are clamped like the voice update (0x484b24).
    fn keep(&mut self, slot: &mut Option<VoiceId>, rel: &str, loop_points: Option<(usize, usize)>, p: Params) {
        self.keep_with(slot, |a| a.bank.get(rel).map(|clip| Clip { loop_points, ..clip }), p);
    }

    /// `keep` for a material record (`mat_clip`).
    fn keep_mat(&mut self, slot: &mut Option<VoiceId>, snd: &MatSound, p: Params) {
        self.keep_with(slot, |a| a.mat_clip(snd).map(|(clip, _)| clip), p);
    }

    fn keep_with(&mut self, slot: &mut Option<VoiceId>, clip: impl FnOnce(&mut Self) -> Option<Clip>, p: Params) {
        let p = Params { volume: p.volume.min(1.0) * self.sfx, ..p };
        match *slot {
            Some(v) if self.audio.with(|m| m.is_playing(v)) => self.audio.set(v, p),
            _ => *slot = clip(self).map(|clip| self.audio.play(&clip, p)),
        }
    }

    /// Stop a record's voice (0x484d78).
    fn stop_slot(&self, slot: &mut Option<VoiceId>) {
        if let Some(v) = slot.take() {
            self.audio.stop(v);
        }
    }

    /// A sound effect, at the effects volume.
    fn one_shot(&mut self, rel: &str, volume: f32) {
        let volume = volume * self.sfx;
        self.play(rel, Params { volume, ..Default::default() });
    }

    /// Update a sound-effect voice, at the effects volume.
    fn set(&self, v: Option<VoiceId>, p: Params) {
        if let Some(v) = v {
            self.audio.set(v, Params { volume: p.volume * self.sfx, ..p });
        }
    }
    /// The engine's voices: the original six loops (CLASSIC, 0x483630..).
    fn engine_voices(&mut self) -> Vec<Option<VoiceId>> {
        let looped = |pan: f32| Params { volume: 0.0, pitch: 1.0, pan, looping: true };
        ENGINE_SAMPLES.iter().map(|&(n, _, pan)| self.play_engine(n, looped(pan))).collect()
    }

    /// The engine-family voices: the engine's six, turbo, interior hum, an `enemyeng` for each of `n` cars, and with
    /// Enhanced the layers (#494 phase 4).
    fn engine_family(&mut self, n: usize) -> EngineFamily {
        let looped = |pan: f32| Params { volume: 0.0, pitch: 1.0, pan, looping: true };
        let engine = self.engine_voices();
        let turbo = self.play_engine("turbo.wav", looped(0.0));
        let inside = self.play_engine("inneen.wav", looped(0.0));
        // one for every car: the followed one is muted, and F4 changes which that is
        let ai = (0..n).map(|_| self.play_engine("enemyeng.wav", looped(0.0))).collect();
        // REMASTER Enhanced: gason and gasrele again for the octave-down body, and gason's air, started with the
        // engine so the air pulses in step with gason (the same length, start and pitch)
        let (body, air) = match self.engine_sound {
            EngineSound::Classic => (Vec::new(), None),
            EngineSound::Enhanced => {
                let body = ["gason.wav", "gasrele.wav"].iter().map(|n| self.play_engine(n, looped(0.0))).collect();
                let air = self.bank.get_air("gason.wav").map(|clip| self.audio.play(&Clip { glide: ENGINE_GLIDE_MS.get() * 0.001, ..clip }, looped(0.0)));
                (body, air)
            }
        };
        EngineFamily { engine, turbo, inside, ai, body, air }
    }

    /// REMASTER: switch the engine sound (the menu row, F9). During a race the engine-family voices are played
    /// again from the other clips; the race and every other voice go on.
    pub fn set_engine_sound(&mut self, e: EngineSound) {
        if e == self.engine_sound {
            return;
        }
        self.engine_sound = e;
        let Some(mut ra) = self.voices.take() else { return };
        for v in ra.engine.iter().chain(ra.ai.iter()).chain(ra.body.iter()).chain([&ra.turbo, &ra.inside, &ra.air]).flatten() {
            self.audio.stop(*v);
        }
        let f = self.engine_family(ra.ai.len());
        (ra.engine, ra.turbo, ra.inside, ra.ai, ra.body, ra.air) = (f.engine, f.turbo, f.inside, f.ai, f.body, f.air);
        ra.blend = EngineBlend::default();
        self.voices = Some(ra);
    }

    /// Start the race's voices for .trk `track_n`.
    pub fn start(&mut self, track_n: u32, race: &Race) {
        let n = race.racers.len();
        let EngineFamily { engine, turbo, inside, ai, body, air } = self.engine_family(n);
        // track ambience from the .trk's EnvSoundName file
        let mut env = Vec::new();
        let mut body_hits = vec![None; 16];
        if let Some(cfg) = track_esd(&self.install, track_n) {
            for e in cfg.all("EnvSound") {
                let f = |i: usize| e.f32(i).unwrap_or(0.0);
                let Some(sample) = e.values.first().cloned() else { continue };
                // pitch 0 in the .esd files = the sample's own rate [I: DirectSound frequency 0]
                let pitch = if f(3) > 0.0 { f(3) } else { 1.0 };
                if let Some(voice) = self.play(&sample, Params { volume: 0.0, pitch, pan: 0.0, looping: true }) {
                    env.push(EnvSound { voice, pos: Vec3::new(f(6), f(7), f(8)), prop: None, pitch, min: f(4), max: f(5).max(f(4) + 1.0), volume: f(2) });
                }
            }
            // ObjectEnvSound n sample priority volume pitch mindistance maxdistance: one voice
            // for every scene object whose ObjEdgeColor id is n (aircraft, trains, helicopters)
            let props = &race.props.props[..];
            for e in cfg.all("ObjectEnvSound") {
                let f = |i: usize| e.f32(i).unwrap_or(0.0);
                let (Some(id), Some(sample)) = (e.f32(0), e.values.get(1)) else { continue };
                let pitch = if f(4) > 0.0 { f(4) } else { 1.0 };
                for (i, _) in props.iter().enumerate().filter(|(_, p)| p.env_sound != 0 && p.env_sound as f32 == id) {
                    if let Some(voice) = self.play(sample, Params { volume: 0.0, pitch, pan: 0.0, looping: true }) {
                        env.push(EnvSound { voice, pos: Vec3::ZERO, prop: Some(i), pitch, min: f(5), max: f(6).max(f(5) + 1.0), volume: f(3) });
                    }
                }
            }
            for e in cfg.all("ObjectColSound") {
                let slot = e.f32(0).map_or(16, |n| n as usize);
                if let (Some(sample), Some(entry)) = (e.values.get(1), body_hits.get_mut(slot)) {
                    *entry = Some((sample.clone(), e.f32(4).unwrap_or(0.0)));
                }
            }
        }
        self.voices = Some(Voices {
            engine,
            blend: EngineBlend::default(),
            turbo,
            inside,
            ai,
            body,
            air,
            env,
            ai_slide: vec![None; n],
            surface: None,
            spin: None,
            slide: None,
            contact: None,
            slide1: None,
            tail: None,
            burst: Vec::new(),
            scrape: None,
            weak: None,
            hard: None,
            last_gear: 1,
            last_count: 4,
            started: false,
            last_cp: None,
            finished: false,
            body_hits,
            body_hit: None,
            alt: 0,
            doppler: Vec::new(),
        });
    }

    /// Stop every voice of the mixer (music too) and forget the race.
    pub fn stop(&mut self) {
        self.audio.with(|m| m.stop_all());
        self.voices = None;
    }

    /// Mute the race (paused, frozen) without losing its voices.
    pub fn silence(&self) {
        let Some(ra) = &self.voices else { return };
        let mute = Params { volume: 0.0, looping: true, ..Default::default() };
        for v in ra.engine.iter().chain(ra.ai.iter()).chain(ra.body.iter()).chain([&ra.turbo, &ra.inside, &ra.air, &ra.scrape.as_ref().map(|h| h.v), &ra.weak.as_ref().map(|h| h.v), &ra.hard]).flatten() {
            self.audio.set(*v, mute);
        }
        for e in &ra.env {
            self.audio.set(e.voice, mute);
        }
        let tyres = [ra.spin, ra.slide, ra.contact, ra.slide1, ra.tail];
        for v in ra.ai_slide.iter().chain(tyres.iter()).flatten().chain(&ra.burst) {
            self.audio.set(*v, mute);
        }
    }

    /// Per frame, after the race stepped `dt`.
    pub fn update(&mut self, sess: &Session, ear: &Ear, dt: f32) {
        let Some(mut ra) = self.voices.take() else { return };
        let race = &sess.race;
        let me = &race.racers[ear.focus];
        let t = &me.telemetry;
        // listener: the camera (view space; game positions convert with z negated, 0x485168)
        let (lp, lr) = (ear.cam.pos, ear.cam.right());
        let lf = {
            let f = ear.cam.forward();
            Vec3::new(f.x, 0.0, f.z).normalize_or(Vec3::NEG_Z)
        };
        // CLASSIC 0x484a00: a 3D record's (volume, pan) for our mixer
        let place = |game_pos: Vec3, min: f32, max: f32, volume: f32| -> (f32, f32) {
            let p = Vec3::new(game_pos.x, game_pos.y, -game_pos.z) - lp;
            let g = falloff(p.length(), min, max);
            let (l, r) = spread(p.dot(lr), p.dot(lf));
            to_mixer(volume * g * l, volume * g * r)
        };
        // 0x756ce0: the in-car camera (0x4b7b10, set with the listener at 0x483b20)
        let in_car = ear.in_car;
        // REMASTER cine: Doppler factors per source (racers, then env) from the change of distance
        let n_src = race.racers.len() + ra.env.len();
        let dop: Vec<f32> = match ear.cine {
            Some(c) if c.doppler => {
                if c.cut || ra.doppler.len() != n_src {
                    ra.doppler = vec![None; n_src];
                }
                let k = 1.0 - (-dt.max(1e-6) / DOPPLER_SMOOTH.get().max(1e-4)).exp();
                let racers = race.racers.iter().map(|rc| rc.car.pos);
                let env = ra.env.iter().map(|e| e.prop.and_then(|i| race.props.props.get(i)).map_or(e.pos, |p| p.pose.pos));
                racers.chain(env).zip(ra.doppler.iter_mut()).map(|(pos, h)| {
                    let d = (Vec3::new(pos.x, pos.y, -pos.z) - lp).length();
                    let v = match *h {
                        Some((d0, v0)) if dt > 0.0 => v0 + ((d - d0) / dt - v0) * k,
                        Some((_, v0)) => v0,
                        None => 0.0,
                    };
                    *h = Some((d, v));
                    doppler_factor(v)
                }).collect()
            }
            _ => vec![1.0; n_src],
        };
        self.focus_mix = match ear.cine {
            None => None,
            Some(_) => {
                let (gain, pan) = if in_car { (1.0, 0.0) } else { place(me.car.pos, FOCUS_NEAR.get(), FOCUS_FAR.get(), 1.0) };
                let gain = if in_car { 1.0 } else { gain / AHEAD_LEVEL.get().max(0.01) };
                Some(FocusMix { gain, pan, pitch: if in_car { 1.0 } else { dop[ear.focus] } })
            }
        };

        // ---- engine: six samples at one rpm pitch (0x483efe..0x48405c) [CLASSIC]
        // `b` = the echo channel of the centreline at the heard car's arc (0x425bab -> P+0xc28, 07-audio 7.2) [O]
        let echo = race.line.echo_at(me.progress.s);
        let pitch = engine_pitch(t.rpm);
        let vols = match self.engine_sound {
            EngineSound::Classic => {
                let (on, off) = engine_levels(me.car.pedal, t.rpm, in_car);
                engine_volumes(on, off, echo)
            }
            // REMASTER: the same levels, glided (#494 phase 2)
            EngineSound::Enhanced => ra.blend.step(me.car.pedal, t.rpm, in_car, echo, t.gear, dt),
        };
        // REMASTER Enhanced: the heard car's character (pitch, level, tone); CLASSIC: x 1 changes nothing
        let ch = match self.engine_sound {
            EngineSound::Classic => None,
            EngineSound::Enhanced => Some(character(me.car_id)),
        };
        let (cp, cl) = ch.map_or((1.0, 1.0), |c| (c.pitch, c.level()));
        for (k, v) in ra.engine.iter().enumerate() {
            let (_, base, pan) = ENGINE_SAMPLES[k];
            self.set(*v, self.focus(Params { volume: vols[k] * base * 0.5 * cl, pitch: pitch * cp, pan, looping: true }));
            if let (Some(c), Some(v)) = (ch, v) {
                self.audio.set_tone(*v, Some(c.tone()));
            }
        }
        // REMASTER Enhanced (#494 phase 4): the layers under the engine of the car you hear, with its character. The
        // body is the throttle / overrun recording an octave down at its level (before the echo split, so it stays in
        // the echo zones); the air is gason's noise at gason's pitch, louder with the revs and the pedal
        if let Some(c) = ch {
            let lv = ra.blend.levels();
            let ep = pitch * cp;
            let (bg, body_tone) = (db_gain(BODY_DB.get() + c.body_db), Tone { low_pass: BODY_LP_HZ.get(), high_pass: BODY_HP_HZ, ..c.tone() });
            for (k, v) in ra.body.iter().enumerate() {
                let level = if k == 0 { lv.on } else { lv.off };
                self.set(*v, self.focus(Params { volume: level * ENGINE_SAMPLES[k].1 * 0.5 * cl * bg, pitch: ep * 0.5, pan: 0.0, looping: true }));
                if let Some(v) = v {
                    self.audio.set_tone(*v, Some(body_tone));
                }
            }
            let (idle, off) = (AIR_IDLE.get().clamp(0.0, 1.0), AIR_OFF.get().clamp(0.0, 1.0));
            let revs = idle + (1.0 - idle) * (t.rpm / 12000.0).clamp(0.0, 1.0);
            let pedal = off + (1.0 - off) * lv.gate;
            let volume = db_gain(AIR_DB.get() + c.air_db) * revs * pedal * lv.scale * ENGINE_SAMPLES[0].1 * 0.5 * cl;
            self.set(ra.air, self.focus(Params { volume, pitch: ep, pan: 0.0, looping: true }));
            if let Some(v) = ra.air {
                // the car's rasp shelf colours its air, its body shelf does not
                self.audio.set_tone(v, Some(Tone { high_pass: AIR_HP_HZ.get(), low_db: 0.0, ..c.tone() }));
            }
        }
        let rpm = t.rpm;
        // turbo whine: volume rpm*5.3e-5 - 0.3, pitch rpm*1.1e-4 + 0.1 (0x484063)
        self.set(ra.turbo, self.focus(Params { volume: (rpm * 5.3e-5 - 0.3).clamp(0.0, 1.0) * 0.6, pitch: rpm * 1.1e-4 + 0.1, pan: 0.0, looping: true }));
        // interior hum, in-car only: volume 0.25 * (1 - 0.3 b), pitch rpm * 2.628e-5 + 0.9 (0x4840b0)
        let hum = if in_car { 0.25 * (1.0 - 0.3 * echo) } else { 0.0 };
        self.set(ra.inside, Params { volume: hum, pitch: rpm * 2.628e-5 + 0.9, pan: 0.0, looping: true });
        // overrun backfire: one of eburst1/2 (0.9, 0x4837b0) while P+0xb0 is set and neither plays (0x4840f6)
        if me.car.overrun && !ra.burst.iter().any(|&v| self.audio.with(|m| m.is_playing(v))) {
            let name = if ra.alternate() { "eburst2.wav" } else { "eburst1.wav" };
            ra.burst = self.play(name, self.focus(Params { volume: 0.9 * self.sfx, ..Default::default() })).into_iter().collect();
        }

        // ---- other cars (0x4845e5): enemyeng and the slide loop, both 3D records of 0x483b41
        // CLASSIC: remote cars are silent (0x483dd1, 07-audio 7.2); ghost cars are not racers here, so they stay out of this loop
        for (i, rc) in race.racers.iter().enumerate() {
            let silent = i == ear.focus || rc.progress.eliminated_ms.is_some();
            let (volume, pan) = if silent { (0.0, 0.0) } else { place(rc.car.pos, 10.0, 100.0, 0.6) };
            if let Some(v) = ra.ai.get(i).copied().flatten() {
                // REMASTER Enhanced: each car's own character
                let ch = (self.engine_sound == EngineSound::Enhanced).then(|| character(rc.car_id));
                let (cp, cl) = ch.map_or((1.0, 1.0), |c| (c.pitch, c.level()));
                self.audio.set(v, Params { volume: volume * self.sfx * cl, pitch: (rc.telemetry.rpm * AI_PITCH_PER_RPM.get() + PITCH_BASE.get()).max(0.05) * dop[i] * cp, pan, looping: true });
                if let Some(c) = ch {
                    self.audio.set_tone(v, Some(c.tone()));
                }
            }
            // spin.wav from 36102, pitch 0.93, (slide * 0.15 + 0.1) * 0.7 while sliding (0x4845fd)
            let c = &rc.car;
            let want = (!silent && c.sliding && !c.wheelspin && !c.brake_locked).then(|| place(c.pos, 10.0, 100.0, (c.slide * 0.15 + 0.1) * 0.7));
            let mut slot = ra.ai_slide.get_mut(i).and_then(Option::take);
            match want {
                Some((volume, pan)) => self.keep(&mut slot, "spin.wav", Some((36102, usize::MAX)), Params { volume, pitch: 0.93 * dop[i], pan, looping: true }),
                None => self.stop_slot(&mut slot),
            }
            if let Some(s) = ra.ai_slide.get_mut(i) {
                *s = slot;
            }
        }

        // ---- ambience
        for (k, e) in ra.env.iter().enumerate() {
            let pos = e.prop.and_then(|i| race.props.props.get(i)).map_or(e.pos, |p| p.pose.pos);
            let boost = if e.prop.is_some() { OBJECT_GAIN.get() } else { 1.0 };
            let (volume, pan) = place(pos, e.min, e.max, e.volume * 0.5 * boost);
            self.audio.set(e.voice, Params { volume: volume * self.amb, pitch: e.pitch * dop[race.racers.len() + k], pan, looping: true });
        }

        // ---- tyres (0x484117..0x484339): the surface is the wheel material with the highest
        // SndContact priority; a new surface stops its three records (they are reloaded from it)
        let car = &me.car;
        let surface = sound_surface(&car.wheel_mat, &race.world.materials);
        if surface != ra.surface {
            ra.surface = surface;
            for s in [&mut ra.spin, &mut ra.slide, &mut ra.contact] {
                self.stop_slot(s);
            }
        }
        match surface.and_then(|m| race.world.materials.get(m)) {
            None => {
                // no wheel on a material (0x4848ae)
                for s in [&mut ra.spin, &mut ra.slide, &mut ra.slide1, &mut ra.contact] {
                    self.stop_slot(s);
                }
            }
            Some(mat) => {
                // SndSpin: locked brakes at pitch 1.08, wheelspin at 1.0 (0x48420d, 0x4847b3)
                let spin = if car.brake_locked { Some(1.08) } else if car.wheelspin { Some(1.0) } else { None };
                match (spin, mat.sound("spin")) {
                    (Some(pitch), Some(snd)) => {
                        // PATCH30 0x4b6b94: x 1.0 (0x501a54; 1998 x 0.4)
                        let volume = car.slide * 0.3 * snd.volume;
                        self.keep_mat(&mut ra.spin, snd, self.focus(Params { volume, pitch, pan: 0.0, looping: snd.looping }));
                    }
                    _ => self.stop_slot(&mut ra.spin),
                }
                // the slide (0x484263, 0x484809): slide1 on a SlideType 1 surface, else SndSlide
                if car.sliding && !car.wheelspin && !car.brake_locked {
                    if mat.slide_type == 1 {
                        for s in [&mut ra.slide, &mut ra.tail] {
                            self.stop_slot(s);
                        }
                        // PATCH30 0x4b6c7a: (slide x 0.9 + 0.4) x 0.9 (1998 (slide x 0.35 + 0.3) x 0.7)
                        let volume = (car.slide * 0.9 + 0.4) * 0.9;
                        self.keep(&mut ra.slide1, "slide1.wav", SLIDE1_LOOP, self.focus(Params { volume, pitch: 1.0, pan: 0.0, looping: true }));
                    } else if let Some(snd) = mat.sound("slide") {
                        self.stop_slot(&mut ra.slide1);
                        // PATCH30 0x4b6cc1: x 0.9 (0x501a50; 1998 x 0.7)
                        let volume = (car.slide * 0.035 + 0.3) * 0.9;
                        self.keep_mat(&mut ra.slide, snd, self.focus(Params { volume, pitch: snd.pitch.max(0.1), pan: 0.0, looping: snd.looping }));
                    }
                } else {
                    // the squeal's tail: slidee1 or slidee2 at 0.5 (0x484282)
                    if mat.slide_type == 1 && ra.slide1.is_some_and(|v| self.audio.with(|m| m.is_playing(v))) {
                        self.stop_slot(&mut ra.slide1);
                        let name = if ra.alternate() { "slidee2.wav" } else { "slidee1.wav" };
                        ra.tail = self.play(name, self.focus(Params { volume: 0.5 * self.sfx, ..Default::default() }));
                    }
                    self.stop_slot(&mut ra.slide);
                }
                // SndContact: the rolling tyres, by the forward speed rb+0x50 (0x4842cc)
                if let Some(snd) = mat.sound("contact") {
                    let k = if in_car { 1.0 } else { 1.2 };
                    let volume = (car.vel.z.abs() * 0.2 * k * 0.1).min(1.0) * snd.volume;
                    self.keep_mat(&mut ra.contact, snd, self.focus(Params { volume, pitch: snd.pitch.max(0.1), pan: 0.0, looping: snd.looping }));
                }
            }
        }

        // ---- hits against the world (0x483d4c at 0x484339..0x484511): the frame's strongest hull
        // hit (car+0xd0) picks SndWeakImpulse (looping, louder with the impulse) or, from
        // HARD_IMPULSE, SndHardImpulse (one shot) of the hit's material (car+0x154); SndScratch
        // loops while that hit's material throws sparks (car+0x168), scaled by how fast the car
        // rubs along it (car+0x178)
        let mat = me.car.hit_material.and_then(|m| race.world.materials.get(m));
        let hit = me.car.world_hits[0];
        let impulse = hit.map_or(0.0, |h| h.impulse);
        // PATCH30 0x4b6eab, 0x4b6f17: 18500 and 1 / 18500 (1998: 16500)
        let hard_from = HARD_IMPULSE.get();
        let weak = (impulse > 0.0 && impulse < hard_from).then(|| mat.and_then(|m| m.sound("weakimpulse")).map(|s| ((s.volume * impulse / hard_from.max(1.0) * HIT_GAIN.get()).min(1.0), 0.0)));
        let weak = weak.flatten().map(|w| self.focus_vp(w));
        // PATCH30 0x4b6f2b: the weak impulse's loop bit is cleared, a one-shot (1998 0x48447d sets it)
        self.drive_hit_loop(&mut ra.weak, mat.and_then(|m| m.sound("weakimpulse")), weak, false, dt);
        // #374: a one-shot fires on a frame that ran a tick only (between ticks the flags are the last tick's)
        let fresh = race.ticks_last_frame() > 0;
        if fresh && impulse >= hard_from {
            // 0x484d78 stops the previous one, then it starts again at volume * 0.7 (0x484906)
            if let Some(v) = ra.hard.take() {
                self.audio.stop(v);
            }
            if let Some(snd) = mat.and_then(|m| m.sound("hardimpulse")) {
                let p = self.focus(Params { volume: (snd.volume * HIT_GAIN.get()).min(1.0) * self.sfx, pitch: snd.pitch.max(0.1), ..Default::default() });
                // PATCH30 0x4b6f53: the same stop and restart, the sample cut by the 3.0 driver
                ra.hard = self.mat_clip(snd).map(|(clip, _)| self.audio.play(&clip, p));
            }
        }
        let scratch = hit.filter(|h| h.spark_anim(&race.world.materials) != 0).and_then(|h| {
            let vol = mat.and_then(|m| m.sound("scratch"))?.volume;
            let pan = if matches!(h.point, 0 | 3) { -SCRAPE_PAN.get() } else { SCRAPE_PAN.get() };
            let (l, r) = pan_2d(pan);
            // PATCH30 0x4b6fa2: x 1.2
            let vol = vol * 1.2;
            Some(to_mixer(h.strength() * vol * l, h.strength() * vol * r))
        });
        let scratch = scratch.map(|w| self.focus_vp(w));
        self.drive_hit_loop(&mut ra.scrape, mat.and_then(|m| m.sound("scratch")), scratch, true, dt);

        // ---- hits against props and cars (0x483d4c at 0x48451e): one shared voice
        if let Some(b) = me.car.bump.filter(|_| fresh) {
            let playing = ra.body_hit.is_some_and(|(v, id)| id == b.id && self.audio.with(|m| m.is_playing(v)));
            if !playing {
                // PATCH30 0x4b7129: v / 30 * 0.9 + 0.1 (1998 0.5 + 0.5 * v / 30); the .esd volume is not used
                let volume = (b.impact / 30.0 * 0.9 + 0.1).min(1.0);
                let (sample, pitch, volume) = match ra.body_hits.get(b.id as usize).and_then(Option::as_ref) {
                    Some((sample, pitch)) if b.id != 0 => (Some(sample.clone()), *pitch, volume),
                    None if b.id != 0 => (None, 0.0, volume),
                    // a car (id 0): majcrash from 30 m/s, else barrel / crash2 (pitch 0.8) alternating
                    _ => {
                        match (b.impact >= 30.0, ra.alternate()) {
                            (true, _) => (Some("majcrash.wav".to_string()), 0.0, 1.0),
                            // PATCH30 0x4b5f4d: barrel at pitch 0.8 too
                            (false, false) => (Some("barrel.wav".to_string()), 0.8, volume),
                            (false, true) => (Some("crash2.wav".to_string()), 0.8, volume),
                        }
                    }
                };
                if let Some((v, _)) = ra.body_hit.take() {
                    self.audio.stop(v);
                }
                // pitch 0 in the .esd files = the sample's own rate [I: DirectSound frequency 0]
                let pitch = if pitch > 0.0 { pitch } else { 1.0 };
                let p = self.focus(Params { volume: volume * self.sfx, pitch, ..Default::default() });
                ra.body_hit = sample.and_then(|s| self.play(&s, p)).map(|v| (v, b.id));
            }
        }

        // ---- gearbox: gear3 (0.7, 0x483601) on every gear change, manual gearbox only (0x48410d)
        if t.gear != ra.last_gear {
            if me.car.manual {
                self.one_shot("gear3.wav", 0.7);
            }
            ra.last_gear = t.gear;
        }

        // ---- race cues (starten\*.wav, finish2.wav)
        if !race.started() {
            // one beep as each digit lights (2 s windows from the "3" at +6 s, 0x45e3c7..0x45e64f)
            let n = race.count_digit().map_or(0, |d| d.0 as i32);
            if n != ra.last_count && (1..=3).contains(&n) {
                self.one_shot("starten/!32.wav", 0.9);
            }
            ra.last_count = n;
        } else if !ra.started {
            ra.started = true;
            self.one_shot("starten/go!.wav", 1.0);
        }
        let p = &me.progress;
        // REMASTER cine: the checkpoint and finish cues are the player's, not the camera's
        let cues = ear.cine.is_none();
        if p.last_checkpoint != ra.last_cp {
            if p.last_checkpoint.is_some() && cues {
                self.one_shot("starten/checkpoint.wav", 0.9);
            }
            ra.last_cp = p.last_checkpoint;
        }
        if p.finished_ms.is_some() && !ra.finished {
            ra.finished = true;
            if cues {
                self.one_shot("finish2.wav", 1.0);
            }
        }
        self.voices = Some(ra);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-5
    }

    #[test]
    fn doppler_rises_coming_and_drops_going() {
        assert_eq!(doppler_factor(0.0), 1.0);
        assert!(close(doppler_factor(-30.0), 343.0 / 313.0));
        assert!(close(doppler_factor(48.8), 343.0 / 391.8));
        // a jump (a cut not caught) stays within the range
        assert_eq!(doppler_factor(-1.0e4), DOPPLER_MAX.get());
        assert_eq!(doppler_factor(1.0e4), DOPPLER_MIN.get());
    }

    #[test]
    fn falloff_is_a_line_from_min_to_max() {
        assert_eq!(falloff(5.0, 10.0, 100.0), 1.0);
        assert!(close(falloff(55.0, 10.0, 100.0), 0.5));
        assert_eq!(falloff(101.0, 10.0, 100.0), 0.0);
    }

    #[test]
    fn spread_quieter_behind_and_on_the_far_side() {
        let (l, r) = spread(0.0, 5.0);
        assert!(close(l, 0.6) && close(r, 0.6));
        let (l, r) = spread(0.0, -5.0);
        assert!(close(l, 0.36) && close(r, 0.36));
        let (l, r) = spread(3.0, 0.0);
        assert!(close(l, 0.16) && close(r, 0.8));
    }

    #[test]
    fn mixer_keeps_ratio_and_centre_level() {
        let (v, p) = to_mixer(0.7, 0.7);
        assert!(close(v, 0.7) && close(p, 0.0));
        // equal-power gains of the mixer: cos / sin of (pan + 1) * pi / 4, times the volume
        let (v, p) = to_mixer(0.2, 0.8);
        let a = (p + 1.0) * std::f32::consts::FRAC_PI_4;
        assert!(close(a.cos() * v, 0.2 * std::f32::consts::FRAC_1_SQRT_2));
        assert!(close(a.sin() * v, 0.8 * std::f32::consts::FRAC_1_SQRT_2));
        let (l, r) = pan_2d(-0.7);
        assert!(close(l, 1.0) && close(r, 0.3));
    }

    #[test]
    fn engine_levels_follow_pedal_and_rpm() {
        assert_eq!(engine_levels(0.5, 9000.0, false), (1.0, 0.0));
        assert_eq!(engine_levels(0.0, 5000.0, false), (1.0, 0.0));
        let (on, off) = engine_levels(0.0, 9000.0, false);
        assert!(close(on, 0.5) && close(off, 0.45));
        assert_eq!(engine_levels(0.0, 13000.0, false), (0.0, 0.9));
        // b = 0: the four L/R loops stay silent
        assert_eq!(&engine_volumes(1.0, 0.0, 0.0)[2..], &[0.0; 4]);
        // b = 0.5: gason halves, the left/right on-throttle loops come in at b * on * 1.8
        let v = engine_volumes(1.0, 0.0, 0.5);
        assert!(close(v[0], 0.5) && close(v[2], 0.5 * LOOP_GAIN.get()) && close(v[2], v[3]) && v[4] == 0.0);
    }

    /// run `n` frames of 1/60 s of the blend with fixed inputs
    fn run(b: &mut EngineBlend, n: usize, pedal: f32, rpm: f32, in_car: bool, echo: f32, gear: usize) -> [f32; 6] {
        let mut v = [0.0; 6];
        for _ in 0..n {
            v = b.step(pedal, rpm, in_car, echo, gear, 1.0 / 60.0);
        }
        v
    }

    #[test]
    fn engine_blend_settles_to_the_game_levels() {
        for &(pedal, rpm, in_car, echo) in &[(1.0f32, 4000.0f32, false, 0.0f32), (0.0, 9000.0, false, 0.0), (0.0, 13000.0, true, 1.0), (0.7, 9000.0, true, 1.0), (0.0, 5000.0, false, 0.0)] {
            let mut b = EngineBlend::default();
            // start from the opposite pedal so there is something to settle
            run(&mut b, 3, 1.0 - pedal.min(1.0), rpm, in_car, 1.0 - echo, 3);
            let got = run(&mut b, 240, pedal, rpm, in_car, echo, 3);
            let (on, off) = engine_levels(pedal, rpm, in_car);
            let want = engine_volumes(on, off, echo);
            for k in 0..6 {
                assert!(close(got[k], want[k]), "pedal {pedal} rpm {rpm} echo {echo} #{k}: {} vs {}", got[k], want[k]);
            }
        }
    }

    #[test]
    fn engine_blend_lift_off_is_not_a_step() {
        let mut b = EngineBlend::default();
        run(&mut b, 120, 1.0, 9000.0, false, 0.0, 3);
        let mut last = 1.0;
        let mut biggest: f32 = 0.0;
        for _ in 0..120 {
            let v = b.step(0.0, 9000.0, false, 0.0, 3, 1.0 / 60.0);
            biggest = biggest.max((v[0] - last).abs());
            last = v[0];
        }
        // gason goes 1 -> 0.5 (the game's level at 9000 rpm); the game does it in one frame, here no frame moves it by 0.2
        assert!(biggest < 0.2 && close(last, 0.5), "step {biggest}, end {last}");
        // and the pedal comes in as fast as it was set: gason within 3 % after 5 attack times
        let up = run(&mut b, (5.0 * PEDAL_ATTACK_MS.get() / 1000.0 * 60.0).ceil() as usize + 1, 1.0, 9000.0, false, 0.0, 3);
        assert!(up[0] > 0.97, "gason {}", up[0]);
    }

    #[test]
    fn engine_blend_echo_fade_keeps_its_level() {
        // the original's linear fade of two different recordings dips to -3 dB at b = 0.5; the blend does not
        let (c, e) = (|v: &[f32; 6]| v[0], |v: &[f32; 6]| v[2] / LOOP_GAIN.get());
        for echo in [0.1f32, 0.3, 0.5, 0.7, 0.9] {
            let mut b = EngineBlend::default();
            run(&mut b, 1, 1.0, 6000.0, false, echo, 2);
            let v = run(&mut b, 600, 1.0, 6000.0, false, echo, 2);
            let energy = c(&v).powi(2) + e(&v).powi(2);
            assert!((energy - 1.0).abs() < 1e-3, "b {echo}: energy {energy}");
        }
    }

    #[test]
    fn engine_blend_dips_at_a_shift_but_not_into_reverse() {
        let mut b = EngineBlend::default();
        let steady = run(&mut b, 120, 1.0, 7000.0, false, 0.0, 2)[0];
        let dip = b.step(1.0, 5000.0, false, 0.0, 3, 1.0 / 60.0)[0];
        assert!(close(dip, steady * (1.0 - GEAR_DIP.get())), "dip {dip}");
        let after = run(&mut b, (5.0 * GEAR_DIP_MS.get() / 1000.0 * 60.0).ceil() as usize, 1.0, 5000.0, false, 0.0, 3)[0];
        assert!(after > 0.98 * steady, "after {after}");
        // 1 -> 0 (reverse) and the start itself: no dip
        let mut b = EngineBlend::default();
        run(&mut b, 120, 1.0, 3000.0, false, 0.0, 1);
        assert!(close(b.step(1.0, 3000.0, false, 0.0, 0, 1.0 / 60.0)[0], steady.min(1.0)));
    }

    #[test]
    fn engine_blend_levels_feed_the_layers() {
        for &(pedal, in_car) in &[(1.0f32, false), (0.0, false), (1.0, true), (0.0, true)] {
            let mut b = EngineBlend::default();
            let lv = { run(&mut b, 240, pedal, 9000.0, in_car, 0.6, 3); b.levels() };
            let (on, off) = engine_levels(pedal, 9000.0, in_car);
            assert!(close(lv.on, on) && close(lv.off, off), "pedal {pedal} in-car {in_car}: {lv:?}");
            assert!(close(lv.gate, pedal.min(1.0)) && close(lv.scale, if in_car { 0.9 } else { 1.0 }), "{lv:?}");
        }
        assert_eq!(db_gain(-60.0), 0.0);
        assert_eq!(db_gain(0.0), 1.0);
        assert!(close(db_gain(-6.0), 0.501_187));
    }

    #[test]
    fn layers_only_with_enhanced() {
        let mut ra = RaceAudio::new(Path::new("/nonexistent"), Audio::silent());
        let f = ra.engine_family(3);
        assert!(f.body.is_empty() && f.air.is_none() && f.engine.len() == 6 && f.ai.len() == 3);
        ra.engine_sound = EngineSound::Enhanced;
        let f = ra.engine_family(3);
        // no samples here: the slots exist, empty
        assert_eq!(f.body.len(), 2);
        assert!(f.air.is_none());
    }

    #[test]
    fn engine_sound_words() {
        assert_eq!(EngineSound::parse("Enhanced"), Some(EngineSound::Enhanced));
        for v in ["baked", "Live", "simulated", "classic"] {
            assert_eq!(EngineSound::parse(v), Some(EngineSound::Classic), "{v}");
        }
        assert_eq!(EngineSound::parse("nonsense"), None);
        for e in EngineSound::ALL {
            assert_eq!(EngineSound::parse(e.key()), Some(e));
        }
    }

    #[test]
    fn character_table() {
        let t = parse_characters(CHARACTER_TABLE);
        assert_eq!(t.len(), 14, "one line per car, the ten of the original and the four of patch 3.0");
        for (car, c) in &t {
            assert!((1..=14).contains(car));
            assert!((0.85..=1.15).contains(&c.pitch), "car {car} pitch {}", c.pitch);
            assert!(c.low_db.abs() <= 6.0 && c.high_db.abs() <= 6.0 && c.level_db.abs() <= 3.0, "car {car} {c:?}");
            assert!((-15.0..=6.0).contains(&c.body_db) && (-8.0..=6.0).contains(&c.air_db), "car {car} {c:?}");
        }
        // the weakest engine (Resonic) is quieter, thinner and has less body than the Titan and the Serpent (#496)
        for strong in [10, 4] {
            assert!(t[&8].level_db < t[&strong].level_db && t[&8].low_db < t[&strong].low_db && t[&8].body_db < t[&strong].body_db);
        }
        // the heavy cars are lower than the screamers; the Titan is a high-revving V12 since #515
        assert!(t[&6].pitch < t[&3].pitch && t[&14].pitch < t[&4].pitch && t[&10].pitch > t[&3].pitch);
        assert_eq!(character(99), Character::NEUTRAL);
        assert_eq!(t[&6].scaled(0.0), Character::NEUTRAL);
        assert_eq!(parse_characters("car01 1 2 3 4\ncar01 1 2 3\n// car02 1 1 1 1\ncarx 1 1 1 1\n").len(), 0);
    }

    #[test]
    fn character_override() {
        // car 13's line is overridden here; no other test reads car 13 through `character()` with a different expectation
        let before = character(13);
        assert!(override_characters("car13 1.234 1 2 3 4 5 // a comment\n").is_ok());
        let c = character(13);
        assert!((c.pitch - 1.234).abs() < 1e-3 && c.air_db == 5.0 * CHARACTER.get(), "{c:?}");
        assert_ne!(c, before);
        assert_eq!(character(14), parse_characters(CHARACTER_TABLE)[&14].scaled(CHARACTER.get()), "other cars stay on the table");
        assert!(override_characters("car13 1 2 3").is_err() && override_characters("hello").is_err());
    }

    #[test]
    fn engine_sound_switch_without_a_race() {
        let mut ra = RaceAudio::new(Path::new("/nonexistent"), Audio::silent());
        ra.set_engine_sound(EngineSound::Enhanced);
        assert_eq!(ra.engine_sound, EngineSound::Enhanced);
        assert!(!ra.active());
    }

    /// the Enhanced engine's loops wrap without a click, on the install's own samples (skipped without one)
    #[test]
    fn enhanced_engine_loops_wrap_smoothly() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../installed_game");
        if !root.join("motor.exe").exists() {
            return;
        }
        let mut bank = Bank::new(&root);
        for n in ["gason", "gasrele", "gasotl", "gasotr", "gasrtl", "gasrtr", "enemyeng", "turbo", "inneen"] {
            let rel = format!("{n}.wav");
            // the 99th-percentile step of the recording itself: a wrap inside it is an ordinary sample step, not a click
            let step = |d: &[f32]| {
                let mut v: Vec<f32> = d.windows(2).map(|w| (w[1] - w[0]).abs()).collect();
                v.sort_by(|a, b| a.total_cmp(b));
                v[v.len() * 99 / 100]
            };
            let jump = |d: &[f32]| (d[d.len() - 1] - d[0]).abs();
            let (orig, enh) = (bank.get(&rel).unwrap(), bank.get_enhanced(&rel).unwrap());
            println!("{n:9} wrap jump {:6.0} -> {:6.0}  (p99 step {:6.0}), length {} -> {}", jump(&orig.data) * 32768.0, jump(&enh.data) * 32768.0, step(&enh.data) * 32768.0, orig.data.len(), enh.data.len());
            // the wrap is no bigger than the steps the recording itself takes
            assert!(jump(&enh.data) <= step(&enh.data), "{n}");
            assert!(enh.data.len() >= orig.data.len() * 9 / 10, "{n}: the seam search ate the loop");
        }
    }

    /// the air layer is noise of gason's length that loops, not a copy of gason (skipped without an install)
    #[test]
    fn air_layer_loops_and_is_noise() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../installed_game");
        if !root.join("motor.exe").exists() {
            return;
        }
        let mut bank = Bank::new(&root);
        let (enh, air) = (bank.get_enhanced("gason.wav").unwrap(), bank.get_air("gason.wav").unwrap());
        assert_eq!(air.data.len(), enh.data.len(), "same length: it plays in step with gason");
        assert_eq!(air.interp, motor_audio::Interp::Sinc);
        let mut st: Vec<f32> = air.data.windows(2).map(|w| (w[1] - w[0]).abs()).collect();
        st.sort_by(|a, b| a.total_cmp(b));
        let wrap = (air.data[air.data.len() - 1] - air.data[0]).abs();
        assert!(wrap <= st[st.len() * 99 / 100], "wrap {wrap}");
        let dot = |a: &[f32], b: &[f32]| a.iter().zip(b).map(|(x, y)| *x as f64 * *y as f64).sum::<f64>();
        let corr = dot(&air.data, &enh.data) / (dot(&air.data, &air.data) * dot(&enh.data, &enh.data)).sqrt();
        let rms = |x: &[f32]| (dot(x, x) / x.len() as f64).sqrt();
        println!("air: rms {:.4} (gason {:.4}), correlation with gason {corr:.3}", rms(&air.data), rms(&enh.data));
        assert!(corr.abs() < 0.2, "correlation {corr}");
        assert!(rms(&air.data) > 0.01);
    }

    /// #495: a race track's CD track from its .trk, and the CD tracks of `Music/` (both skip without the install)
    #[test]
    fn cd_track_of_a_race_track() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../installed_game");
        if !root.join("motor.exe").exists() {
            return;
        }
        let n = cd_track(&root, 2).expect("Track2.trk names a CDTrack");
        assert!(n >= 4, "race tracks play from CD track 4 on, got {n}");
        if motor_data::fs::find_ci(&root, "Music").is_some() {
            assert!(index_music(&root).contains_key(&n), "Music/ holds CD track {n}");
        }
    }
}
