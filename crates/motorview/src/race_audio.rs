//! Race sound: the car you hear (engine, tyres, hits, cues), the other cars, the track
//! ambience, heard from a listener (`Ear`). Shared by the game (bin motorhead audio.rs) and the
//! cine render (cine_render.rs). Spec: re-notes/07-audio.md, 20-cine.md "Audio".

use crate::drive::Session;
use crate::engine_sim::SimEngine;
use crate::loading::{Progress, Stage};
use crate::render::Camera;
use glam::Vec3;
use motor_audio::{wav, Audio, Clip, Params, VoiceId};
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
    /// `b` of the engine update (P+0xc28, 0x483d7b): fades the four L/R loops in. Only the loader
    /// writes it (0 at 0x483cbd), so the loops never sound (07-audio 7.2) [D].
    /// range: 0..1
    /// area: Audio/Engine
    LOOP_FADE: f32 = 0.0;
    /// Pitch slope of the other cars' engines (0x4d835f; the car you hear: `PITCH_PER_RPM`) [O].
    /// range: 0..0.001
    /// area: Audio/Engine
    AI_PITCH_PER_RPM: f32 = 0.00016512;
    /// Level of the simulated engine's loops (baked to a 0.9 peak) [design choice, tuned by ear].
    /// range: 0..3
    /// area: Audio/Engine
    SIM_VOLUME: f32 = 1.0;
}
/// slide1.wav's loop (0x483596: start 9952, end 1 = to the sample's end [I]).
const SLIDE1_LOOP: Option<(usize, usize)> = Some((9952, usize::MAX));
motor_tune::tunable! {
    /// World hit impulse from which SndHardImpulse plays instead of SndWeakImpulse (0x4d83e7); the
    /// weak sound's volume is `volume x impulse / this x HIT_GAIN` (0x4d83ef) [O].
    /// range: 1000..60000
    /// unit: kg m/s
    /// area: Audio/Hits
    HARD_IMPULSE: f32 = 16500.0;
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
    /// REMASTER the engine pack, for (car id) when the simulated engine is asked for
    sim: Option<(u32, SimEngine)>,
}

/// The track's .esd ambience file (the .trk's EnvSoundName).
fn track_esd(root: &Path, track_n: u32) -> Option<motor_data::cfg::Config> {
    let esd = motor_data::fs::config(root, &format!("Game/Track{track_n}.trk")).ok().and_then(|c| c.get("EnvSoundName").and_then(|e| e.values.first().cloned()))?;
    motor_data::fs::config(root, &esd).ok()
}

/// #335 decode what a race on .trk `track_n` plays (on the loading worker): the engine (the
/// REMASTER pack of `car_id` when `sim`, else the six original loops), the race samples, the
/// .esd ambience and object sounds, the track materials' sounds. `p` reports the AUDIO stage.
pub fn race_sounds(root: &Path, track_n: u32, materials: &[motor_sim::world::Material], car_id: u32, sim: bool, p: &Progress) -> Result<RaceSounds, String> {
    p.set(Stage::Audio, 0.0)?;
    let sim = sim.then(|| SimEngine::load(root, car_id).map(|e| (car_id, e))).flatten();
    let mut names: Vec<String> = RACE_SAMPLES.iter().map(|s| s.to_string()).collect();
    if sim.is_none() {
        names.extend(ENGINE_SAMPLES.iter().map(|s| s.0.to_string()));
    }
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
    Ok(RaceSounds { clips, sim })
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

/// REMASTER simulated engine: one looping voice per grid loop of the pack (engine_sim.rs).
struct SimVoices {
    engine: SimEngine,
    voices: Vec<VoiceId>,
}

/// A material sound that loops while its hit lasts and plays out afterwards.
struct HitVoice {
    v: VoiceId,
    sample: String,
    p: Params,
}

/// The race's voices (built by `RaceAudio::start`).
struct Voices {
    engine: Vec<Option<VoiceId>>,
    /// replaces `engine` when the pack exists and the setting asks for it
    sim: Option<SimVoices>,
    /// the setting the engine voices were built for (it can change from the pause menu)
    sim_asked: bool,
    turbo: Option<VoiceId>,
    inside: Option<VoiceId>,
    ai: Vec<Option<VoiceId>>,
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
    /// eburst1/2 (0x757280 / 0x7572e4)
    burst: Option<VoiceId>,
    /// SndScratch and SndWeakImpulse: shared looping records of 0x483d4c
    scrape: Option<HitVoice>,
    weak: Option<HitVoice>,
    /// the SndHardImpulse voice
    hard: Option<VoiceId>,
    throttle: f32,
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

/// REMASTER/CLASSIC race sound on a mixer: the game plays it live, the cine render offline.
pub struct RaceAudio {
    audio: Audio,
    bank: Bank,
    voices: Option<Voices>,
    /// #335 the engine pack the loading worker read (car id, pack), taken by `engine_voices`
    pre_sim: Option<(u32, SimEngine)>,
    install: PathBuf,
    /// REMASTER cine: the car you hear, placed (None in the game: heard as the player)
    focus_mix: Option<FocusMix>,
    /// user volumes 0..1: effects, track ambience
    pub sfx: f32,
    pub amb: f32,
    /// simulated engine sound wanted, and the car whose pack it would use
    pub sim_wanted: bool,
    pub car_id: u32,
}

impl RaceAudio {
    pub fn new(root: &Path, audio: Audio) -> RaceAudio {
        RaceAudio { audio, bank: Bank::new(root), voices: None, pre_sim: None, install: root.to_path_buf(), focus_mix: None, sfx: 0.8, amb: 0.8, sim_wanted: false, car_id: 1 }
    }

    /// Take the sounds a loading worker decoded (`race_sounds`), before `start`.
    pub fn adopt(&mut self, pre: RaceSounds) {
        self.bank.adopt(pre.clips);
        self.pre_sim = pre.sim;
    }

    /// The race's voices exist (between `start` and `stop`).
    pub fn active(&self) -> bool {
        self.voices.is_some()
    }

    fn play(&mut self, rel: &str, p: Params) -> Option<VoiceId> {
        let clip = self.bank.get(rel)?;
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
    fn drive_hit_loop(&mut self, slot: &mut Option<HitVoice>, snd: Option<&MatSound>, want: Option<(f32, f32)>) {
        let playing = slot.as_ref().is_some_and(|h| self.audio.with(|m| m.is_playing(h.v)));
        if !playing {
            *slot = None;
        }
        match (want, snd) {
            (Some((volume, pan)), Some(snd)) => {
                let p = Params { volume: volume.min(1.0), pitch: snd.pitch.max(0.1), pan, looping: true };
                // another material's sample replaces the record's (stopped first, 0x484d78)
                if slot.as_ref().is_some_and(|h| h.sample != snd.sample) {
                    if let Some(h) = slot.take() {
                        self.audio.stop(h.v);
                    }
                }
                if let Some(h) = slot {
                    h.p = p;
                    self.audio.set(h.v, Params { volume: p.volume * self.sfx, ..p });
                } else if let Some(mut clip) = self.bank.get(&snd.sample) {
                    clip.loop_points = snd.loop_points;
                    let v = self.audio.play(&clip, Params { volume: p.volume * self.sfx, ..p });
                    *slot = Some(HitVoice { v, sample: snd.sample.clone(), p });
                }
            }
            _ => {
                if let Some(h) = slot {
                    if h.p.looping {
                        h.p.looping = false;
                        self.audio.set(h.v, Params { volume: h.p.volume * self.sfx, ..h.p });
                    }
                }
            }
        }
    }

    /// Keep a looping record going at `p` (0x484cf0: started when it does not play, else updated),
    /// at the effects volume; volumes above 1 are clamped like the voice update (0x484b24).
    fn keep(&mut self, slot: &mut Option<VoiceId>, rel: &str, loop_points: Option<(usize, usize)>, p: Params) {
        let p = Params { volume: p.volume.min(1.0) * self.sfx, ..p };
        match *slot {
            Some(v) if self.audio.with(|m| m.is_playing(v)) => self.audio.set(v, p),
            _ => {
                *slot = self.bank.get(rel).map(|mut clip| {
                    clip.loop_points = loop_points;
                    self.audio.play(&clip, p)
                });
            }
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
    /// The engine's voices: the baked simulation when asked for and the car has a pack
    /// (REMASTER), else the original six loops (CLASSIC, 0x483630..).
    fn engine_voices(&mut self) -> (Vec<Option<VoiceId>>, Option<SimVoices>) {
        let looped = |pan: f32| Params { volume: 0.0, pitch: 1.0, pan, looping: true };
        if self.sim_wanted {
            let pre = self.pre_sim.take().filter(|p| p.0 == self.car_id).map(|p| p.1);
            if let Some(engine) = pre.or_else(|| SimEngine::load(&self.install, self.car_id)) {
                let voices = engine.loops.iter().map(|l| self.audio.play(&l.clip, looped(0.0))).collect();
                return (Vec::new(), Some(SimVoices { engine, voices }));
            }
            eprintln!("audio: no engine pack for car {:02} (remaster/engines), using the original engine", self.car_id);
        }
        (ENGINE_SAMPLES.iter().map(|&(n, _, pan)| self.play(n, looped(pan))).collect(), None)
    }

    /// Start the race's voices for .trk `track_n` (the adopted pack, if any, is used up).
    pub fn start(&mut self, track_n: u32, race: &Race) {
        let looped = |pan: f32| Params { volume: 0.0, pitch: 1.0, pan, looping: true };
        let (engine, sim) = self.engine_voices();
        let turbo = self.play("turbo.wav", looped(0.0));
        let inside = self.play("inneen.wav", looped(0.0));
        let n = race.racers.len();
        // one for every car: the followed one is muted, and F4 changes which that is
        let ai = (0..n).map(|_| self.play("enemyeng.wav", looped(0.0))).collect();
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
        self.pre_sim = None;
        self.voices = Some(Voices {
            engine,
            sim,
            sim_asked: self.sim_wanted,
            turbo,
            inside,
            ai,
            env,
            ai_slide: vec![None; n],
            surface: None,
            spin: None,
            slide: None,
            contact: None,
            slide1: None,
            tail: None,
            burst: None,
            scrape: None,
            weak: None,
            hard: None,
            throttle: 0.0,
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
        for v in ra.engine.iter().chain(ra.ai.iter()).chain([&ra.turbo, &ra.inside, &ra.scrape.as_ref().map(|h| h.v), &ra.weak.as_ref().map(|h| h.v), &ra.hard]).flatten() {
            self.audio.set(*v, mute);
        }
        for v in ra.sim.iter().flat_map(|s| &s.voices) {
            self.audio.set(*v, mute);
        }
        for e in &ra.env {
            self.audio.set(e.voice, mute);
        }
        let tyres = [ra.spin, ra.slide, ra.contact, ra.slide1, ra.tail, ra.burst];
        for v in ra.ai_slide.iter().chain(tyres.iter()).flatten() {
            self.audio.set(*v, mute);
        }
    }

    /// Per frame, after the race stepped `dt`.
    pub fn update(&mut self, sess: &Session, ear: &Ear, dt: f32) {
        let Some(mut ra) = self.voices.take() else { return };
        if ra.sim_asked != self.sim_wanted {
            // the setting changed in the pause menu: swap the engine voices
            for v in ra.engine.drain(..).flatten().chain(ra.sim.take().into_iter().flat_map(|s| s.voices)) {
                self.audio.stop(v);
            }
            (ra.engine, ra.sim) = self.engine_voices();
            ra.sim_asked = self.sim_wanted;
        }
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

        // ---- engine: six samples at one rpm pitch (0x483efe..0x48405c)
        let pedal = if t.gear == 0 { 0.0 } else { (t.drive_force > 0.0) as u8 as f32 };
        ra.throttle += (pedal - ra.throttle) * (1.0 - (-12.0 * dt).exp());
        let pitch = engine_pitch(t.rpm);
        if let Some(sim) = &ra.sim {
            // REMASTER: the baked loops around the wanted rpm and pedal, each pitched to that rpm
            let mapped = sim.engine.map_rpm(t.rpm, me.car.p.min_rpm, me.car.p.max_rpm);
            let mut mix = vec![(0.0f32, 1.0f32); sim.voices.len()];
            for tap in sim.engine.taps(mapped, ra.throttle) {
                mix[tap.index] = (tap.gain, tap.pitch);
            }
            for (v, (gain, pitch)) in sim.voices.iter().zip(mix) {
                self.audio.set(*v, self.focus(Params { volume: gain * SIM_VOLUME.get() * self.sfx, pitch, pan: 0.0, looping: true }));
            }
        } else {
            // CLASSIC
            let (on, off) = engine_levels(me.car.pedal, t.rpm, in_car);
            let vols = engine_volumes(on, off, LOOP_FADE.get());
            for (k, v) in ra.engine.iter().enumerate() {
                let (_, base, pan) = ENGINE_SAMPLES[k];
                self.set(*v, self.focus(Params { volume: vols[k] * base * 0.5, pitch, pan, looping: true }));
            }
        }
        // turbo whine: volume rpm*5.3e-5 - 0.3, pitch rpm*1.1e-4 + 0.1 (0x484063)
        self.set(ra.turbo, self.focus(Params { volume: (t.rpm * 5.3e-5 - 0.3).clamp(0.0, 1.0) * 0.6, pitch: t.rpm * 1.1e-4 + 0.1, pan: 0.0, looping: true }));
        // interior hum, in-car only: volume 0.25 * (1 - 0.3 b), pitch rpm * 2.628e-5 + 0.9 (0x4840b0)
        let hum = if in_car { 0.25 * (1.0 - 0.3 * LOOP_FADE.get()) } else { 0.0 };
        self.set(ra.inside, Params { volume: hum, pitch: t.rpm * 2.628e-5 + 0.9, pan: 0.0, looping: true });
        // overrun backfire: one of eburst1/2 (0.9, 0x4837b0) while P+0xb0 is set and neither plays (0x4840f6)
        if me.car.overrun && !ra.burst.is_some_and(|v| self.audio.with(|m| m.is_playing(v))) {
            let name = if ra.alternate() { "eburst2.wav" } else { "eburst1.wav" };
            ra.burst = self.play(name, self.focus(Params { volume: 0.9 * self.sfx, ..Default::default() }));
        }

        // ---- other cars (0x4845e5): enemyeng and the slide loop, both 3D records of 0x483b41
        // CLASSIC: remote cars are silent (0x483dd1, 07-audio 7.2); ghost cars are not racers here, so they stay out of this loop
        for (i, rc) in race.racers.iter().enumerate() {
            let silent = i == ear.focus || rc.progress.eliminated_ms.is_some();
            let (volume, pan) = if silent { (0.0, 0.0) } else { place(rc.car.pos, 10.0, 100.0, 0.6) };
            if let Some(v) = ra.ai.get(i).copied().flatten() {
                self.audio.set(v, Params { volume: volume * self.sfx, pitch: (rc.telemetry.rpm * AI_PITCH_PER_RPM.get() + PITCH_BASE.get()).max(0.05) * dop[i], pan, looping: true });
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
                        let volume = car.slide * 0.3 * 0.4 * snd.volume;
                        self.keep(&mut ra.spin, &snd.sample, snd.loop_points, self.focus(Params { volume, pitch, pan: 0.0, looping: snd.looping }));
                    }
                    _ => self.stop_slot(&mut ra.spin),
                }
                // the slide (0x484263, 0x484809): slide1 on a SlideType 1 surface, else SndSlide
                if car.sliding && !car.wheelspin && !car.brake_locked {
                    if mat.slide_type == 1 {
                        for s in [&mut ra.slide, &mut ra.tail] {
                            self.stop_slot(s);
                        }
                        let volume = (car.slide * 0.35 + 0.3) * 0.7;
                        self.keep(&mut ra.slide1, "slide1.wav", SLIDE1_LOOP, self.focus(Params { volume, pitch: 1.0, pan: 0.0, looping: true }));
                    } else if let Some(snd) = mat.sound("slide") {
                        self.stop_slot(&mut ra.slide1);
                        let volume = (car.slide * 0.035 + 0.3) * 0.7;
                        self.keep(&mut ra.slide, &snd.sample, snd.loop_points, self.focus(Params { volume, pitch: snd.pitch.max(0.1), pan: 0.0, looping: snd.looping }));
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
                    self.keep(&mut ra.contact, &snd.sample, snd.loop_points, self.focus(Params { volume, pitch: snd.pitch.max(0.1), pan: 0.0, looping: snd.looping }));
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
        let weak = (impulse > 0.0 && impulse < HARD_IMPULSE.get()).then(|| mat.and_then(|m| m.sound("weakimpulse")).map(|s| ((s.volume * impulse / HARD_IMPULSE.get().max(1.0) * HIT_GAIN.get()).min(1.0), 0.0)));
        let weak = weak.flatten().map(|w| self.focus_vp(w));
        self.drive_hit_loop(&mut ra.weak, mat.and_then(|m| m.sound("weakimpulse")), weak);
        // #374: a one-shot fires on a frame that ran a tick only (between ticks the flags are the last tick's)
        let fresh = race.ticks_last_frame() > 0;
        if fresh && impulse >= HARD_IMPULSE.get() {
            // 0x484d78 stops the previous one, then it starts again at volume * 0.7 (0x484906)
            if let Some(v) = ra.hard.take() {
                self.audio.stop(v);
            }
            if let Some(snd) = mat.and_then(|m| m.sound("hardimpulse")) {
                ra.hard = self.play(&snd.sample, self.focus(Params { volume: (snd.volume * HIT_GAIN.get()).min(1.0) * self.sfx, pitch: snd.pitch.max(0.1), ..Default::default() }));
            }
        }
        let scratch = hit.filter(|h| h.spark_anim(&race.world.materials) != 0).and_then(|h| {
            let vol = mat.and_then(|m| m.sound("scratch"))?.volume;
            let pan = if matches!(h.point, 0 | 3) { -SCRAPE_PAN.get() } else { SCRAPE_PAN.get() };
            let (l, r) = pan_2d(pan);
            Some(to_mixer(h.strength() * vol * l, h.strength() * vol * r))
        });
        let scratch = scratch.map(|w| self.focus_vp(w));
        self.drive_hit_loop(&mut ra.scrape, mat.and_then(|m| m.sound("scratch")), scratch);

        // ---- hits against props and cars (0x483d4c at 0x48451e): one shared voice
        if let Some(b) = me.car.bump.filter(|_| fresh) {
            let playing = ra.body_hit.is_some_and(|(v, id)| id == b.id && self.audio.with(|m| m.is_playing(v)));
            if !playing {
                // volume 0.5 + 0.5 * v / 30 (0x4d83f7, 0x4d83ff); the .esd volume is not used
                let volume = (0.5 + b.impact / 30.0 * 0.5).min(1.0);
                let (sample, pitch, volume) = match ra.body_hits.get(b.id as usize).and_then(Option::as_ref) {
                    Some((sample, pitch)) if b.id != 0 => (Some(sample.clone()), *pitch, volume),
                    None if b.id != 0 => (None, 0.0, volume),
                    // a car (id 0): majcrash from 30 m/s, else barrel / crash2 (pitch 0.8) alternating
                    _ => {
                        match (b.impact >= 30.0, ra.alternate()) {
                            (true, _) => (Some("majcrash.wav".to_string()), 0.0, 1.0),
                            (false, false) => (Some("barrel.wav".to_string()), 0.0, volume),
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
        assert_eq!(&engine_volumes(1.0, 0.0, LOOP_FADE.get())[2..], &[0.0; 4]);
    }
}
