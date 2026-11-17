//! Motorview's own sound (#495): the speaker button's Sound and Music switches. Sound is the game's race sound
//! (`race_audio::RaceAudio`) heard from the camera on screen, Music the race track's CD track from `Music/`; both at the
//! volumes of the game's `settings.cfg`. Both start off (`view_settings::Switches`). The Cine editor has its own sound
//! (`cine_edit::Listen`): while it is open this one is hushed. Never used by the headless `--shot` paths.

use crate::drive::Session;
use crate::race_audio::{cd_track, index_music, Ear, EngineSound, RaceAudio, MASTER, MUSIC_VOLUME};
use crate::render::{Camera, RenderMode};
use crate::view_settings::{SoundPrefs, Switches};
use motor_audio::{Audio, MusicStream};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// A forward step of the race clock larger than this (seconds) is a jump (seek, restart): the voices start again [design choice].
const JUMP: f64 = 2.0;

/// What the viewer hands over each frame.
pub struct Frame<'a> {
    pub sess: &'a Session,
    /// the camera on screen
    pub cam: Camera,
    pub in_car: bool,
    /// .trk number and car number of the race (a change rebuilds the voices)
    pub track: u32,
    pub car: u32,
    pub mode: RenderMode,
    /// wall-clock seconds when the sim ran forward at normal pace this frame; None paused, rewinding or running ahead
    pub play: Option<f32>,
}

/// The output stream and what plays on it; exists only while a switch is on.
struct Out {
    audio: Audio,
    race: RaceAudio,
    /// CD track number -> file, `Music/`
    cd: HashMap<u32, PathBuf>,
    /// the race the voices were built for: (track, car, racers)
    key: Option<(u32, u32, usize)>,
    /// the race timeline position (s) of the last frame heard
    heard: Option<f64>,
    /// the CD track streaming now
    music: Option<u32>,
}

pub struct ViewSound {
    pub switches: Switches,
    pub prefs: SoundPrefs,
    root: PathBuf,
    /// the saved output id (None: the system default)
    device: Option<String>,
    out: Option<Out>,
}

impl ViewSound {
    pub fn new(root: &Path, device: Option<String>, prefs: SoundPrefs) -> ViewSound {
        ViewSound { switches: Switches::load(), prefs, root: root.to_path_buf(), device, out: None }
    }

    /// Flip the Sound switch and keep it.
    pub fn toggle_sound(&mut self) {
        self.switches.sound ^= true;
        self.switches.save();
    }

    /// Flip the Music switch and keep it.
    pub fn toggle_music(&mut self) {
        self.switches.music ^= true;
        self.switches.save();
    }

    /// The engine sound picked on the speaker button, else the game's setting.
    pub fn engine(&self) -> EngineSound {
        self.switches.engine.unwrap_or(self.prefs.engine_sound)
    }

    /// Classic <-> Enhanced from the speaker button, kept.
    pub fn toggle_engine(&mut self) {
        self.switches.engine = Some(match self.engine() {
            EngineSound::Classic => EngineSound::Enhanced,
            EngineSound::Enhanced => EngineSound::Classic,
        });
        self.switches.save();
    }

    /// A switch is on but the machine has no output to play on.
    pub fn no_device(&self) -> bool {
        self.out.as_ref().is_some_and(|o| !o.audio.has_device())
    }

    /// Close the output (the Cine editor is open, no race, both switches off).
    pub fn hush(&mut self) {
        self.out = None;
    }

    /// Once per viewer frame, after the sim stepped.
    pub fn update(&mut self, f: &Frame) {
        if !(self.switches.sound || self.switches.music) {
            return self.hush();
        }
        if self.out.is_none() {
            let audio = Audio::open_on(self.device.as_deref());
            audio.with(|m| m.master = MASTER.get());
            let race = RaceAudio::new(&self.root, audio.share());
            self.out = Some(Out { audio, race, cd: index_music(&self.root), key: None, heard: None, music: None });
        }
        let Some(out) = self.out.as_mut() else { return };
        let pct = |v: u32| v.min(100) as f32 / 100.0;
        let engine = self.switches.engine.unwrap_or(self.prefs.engine_sound);
        if self.switches.sound {
            out.sound(f, &self.prefs, engine, pct);
        } else if out.race.active() {
            out.race.stop();
            (out.key, out.heard) = (None, None);
        }
        let want = self.switches.music.then(|| self.prefs.track_cd.get(&f.track).copied().or_else(|| cd_track(&self.root, f.track))).flatten().filter(|n| out.cd.contains_key(n));
        if want != out.music {
            let stream = want.and_then(|n| out.cd.get(&n)).map(|p| MusicStream::open(p, true, MUSIC_VOLUME));
            out.audio.with(|m| m.set_music(stream));
            out.music = want;
        }
        let v = MUSIC_VOLUME * pct(self.prefs.vol_music);
        out.audio.with(|m| m.set_music_volume(v));
    }
}

impl Out {
    fn sound(&mut self, f: &Frame, prefs: &SoundPrefs, engine: EngineSound, pct: impl Fn(u32) -> f32) {
        let race = &f.sess.race;
        let key = (f.track, f.car, race.racers.len());
        // the race timeline: the countdown, then the clock since GO (restart, rewind and seek move it back or far)
        let p = race.clock_ms / 1000.0 - race.countdown as f64;
        let fresh = !self.race.active() || self.key != Some(key) || self.heard.is_none_or(|h| p < h - 1e-6 || p - h > JUMP);
        // REMASTER: the setting's engine sound, the game's own in Classic (as the game, audio.rs `effective_engine_sound`)
        self.race.set_engine_sound(if f.mode == RenderMode::Remaster { engine } else { EngineSound::Classic });
        if fresh {
            self.race.stop();
            self.race.start(f.track, race);
            (self.key, self.heard) = (Some(key), None);
        }
        (self.race.sfx, self.race.amb) = (pct(prefs.vol_sfx), pct(prefs.vol_ambient));
        match f.play {
            Some(dt) => {
                let ear = Ear { cam: f.cam, focus: f.sess.focus, in_car: f.in_car, cine: None };
                self.race.update(f.sess, &ear, dt);
                self.heard = Some(p);
            }
            None => self.race.silence(),
        }
    }
}
