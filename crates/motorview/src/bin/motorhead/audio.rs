//! Game sound: the race sound (motorview::race_audio, heard from the game camera), horns,
//! menu sounds, CD music and movie sound. Spec: re-notes/07-audio.md.

use crate::front::{Game, Screen, Sfx};
use motor_audio::{Audio, MusicStream, Params, VoiceId};
use motorview::drive::CamMode;
use motorview::race_audio::{Bank, Ear, RaceAudio, MASTER};
pub use motorview::race_audio::{race_sounds, RaceSounds};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// The audio CD's track layout: 1 data, 2 results, 3 menu (the MenuCDTrack default),
/// 4.. per-track music (CDTrack).
const CD_RESULTS: u32 = 2;
const MUSIC_VOLUME: f32 = 1.15;

pub struct GameAudio {
    audio: Audio,
    bank: Bank,
    /// menusound.cfg: event -> sample under Sounds/Menu
    menu: HashMap<u32, String>,
    /// Menyloop.wav when no CD rip is present
    music: Option<VoiceId>,
    /// the race sound (its own sample bank, the same mixer)
    race: RaceAudio,
    /// Music/: CD track number -> file (leading digits of the file name)
    cd: HashMap<u32, PathBuf>,
    /// CD track now streaming
    cd_playing: Option<u32>,
    /// a movie's sound sits in the music slot (`movie`)
    movie_sound: bool,
    /// user volumes 0..1 (Settings vol_*): effects, music, track ambience
    sfx: f32,
    mus: f32,
    amb: f32,
    /// REMASTER Car screen: the horn being tried (a new one cuts it)
    horn_try: Option<VoiceId>,
    /// the id of the output the sound is on (#260): what the player picked on the Sound screen, None = the system default
    device: Option<String>,
}

/// Index a folder of ripped CD tracks: "04 RedRock.mp3" -> 4. Any format symphonia reads.
fn index_music(root: &Path) -> HashMap<u32, PathBuf> {
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

/// The CD track numbers present in Music/, ascending.
pub fn cd_track_numbers(root: &Path) -> Vec<u32> {
    let mut v: Vec<u32> = index_music(root).into_keys().collect();
    v.sort_unstable();
    v
}

/// The title of each CD track present in Music/, from its file name: "04 RedRock.mp3" -> "RedRock" (the
/// Remaster Songs tab, #259).
pub fn cd_track_names(root: &Path) -> HashMap<u32, String> {
    index_music(root)
        .into_iter()
        .filter_map(|(n, p)| {
            let stem = p.file_stem()?.to_string_lossy().to_string();
            let name = stem.trim_start_matches(|c: char| c.is_ascii_digit()).trim_matches(|c: char| c == ' ' || c == '-' || c == '_' || c == '.').to_string();
            Some((n, name))
        })
        .collect()
}

/// The length in seconds of each CD track present in Music/ (the song-to-track table's Length
/// column; the original reads it from the disc's table of contents, 0x469668). Tracks whose
/// length cannot be read are left out.
pub fn cd_track_lengths(root: &Path) -> HashMap<u32, u32> {
    index_music(root).into_iter().filter_map(|(n, p)| Some((n, motor_audio::music::duration_secs(&p)?))).collect()
}

/// Parse Sounds/Menu/menusound.cfg: SampleName lines, each followed by SampleEvent numbers.
/// The first sample listed for an event wins.
fn menu_events(root: &Path) -> HashMap<u32, String> {
    let mut map = HashMap::new();
    let Ok(raw) = motor_data::fs::read(root, "Sounds/Menu/menusound.cfg") else { return map };
    let c = motor_data::cfg::Config::parse(&raw);
    let mut sample = None;
    for e in &c.entries {
        match e.key.to_ascii_lowercase().as_str() {
            "samplename" => sample = e.values.first().cloned(),
            "sampleevent" => {
                if let Some(s) = &sample {
                    for v in &e.values {
                        if let Ok(n) = v.parse::<u32>() {
                            map.entry(n).or_insert_with(|| format!("Menu/{s}"));
                        }
                    }
                }
            }
            _ => {}
        }
    }
    map
}

impl GameAudio {
    pub fn new(root: &Path, audio: Audio, device: Option<String>) -> GameAudio {
        audio.with(|m| m.master = MASTER);
        let cd = index_music(root);
        if !cd.is_empty() {
            println!("music: {} CD tracks in Music/", cd.len());
        }
        let race = RaceAudio::new(root, audio.share());
        GameAudio { audio, bank: Bank::new(root), menu: menu_events(root), music: None, race, cd, cd_playing: None, movie_sound: false, sfx: 0.8, mus: 0.7, amb: 0.8, horn_try: None, device }
    }


    fn play(&mut self, rel: &str, p: Params) -> Option<VoiceId> {
        let clip = self.bank.get(rel)?;
        Some(self.audio.play(&clip, p))
    }

    /// Stop a voice (the horn being tried).
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


    /// Per frame, after the game has updated.
    pub fn sync(&mut self, game: &mut Game, dt: f32) {
        // #260: the output the player picked on the Sound screen. Moved first, so the click of the step that picked it
        // (queued below) is heard on the new one; the mixer, with the music and the voices, plays on.
        let want = game.settings.sound_device.as_ref().map(|d| d.id.as_str());
        if want != self.device.as_deref() {
            self.audio.switch(want);
            self.device = want.map(str::to_string);
        }
        game.sound_device_failed = self.audio.fallback().is_some();
        game.sound_silent = !self.audio.has_device();
        let pct = |v: u32| v.min(100) as f32 / 100.0;
        (self.sfx, self.mus, self.amb) = (pct(game.settings.vol_sfx), pct(game.settings.vol_music), pct(game.settings.vol_ambient));
        (self.race.sfx, self.race.amb) = (self.sfx, self.amb);
        self.race.sim_wanted = game.settings.engine_sound;
        self.race.car_id = game.cars.get(game.settings.car).map_or(1, |c| c.id);
        for s in std::mem::take(&mut game.sfx) {
            match s {
                Sfx::Menu(e) => {
                    if let Some(rel) = self.menu.get(&e).cloned() {
                        self.one_shot(&rel, 0.8);
                    }
                }
                Sfx::Horn | Sfx::Honk(_) | Sfx::HornTry => {
                    let i = if let Sfx::Honk(i) = s { i } else { game.settings.horn };
                    if s == Sfx::HornTry {
                        let mut v = self.horn_try.take();
                        self.stop_slot(&mut v);
                    }
                    if let Some(h) = game.horns.get(i).cloned() {
                        let p = Params { volume: 0.9 * (1.0 + h.volume_change * 0.01) * self.sfx, pitch: 1.0 + h.pitch_change * 0.01, ..Default::default() };
                        let v = self.play(&h.sample, p);
                        if s == Sfx::HornTry {
                            self.horn_try = v;
                        }
                    }
                }
            }
        }
        // race sounds live while racing or paused; results and menus stop them
        let racing = matches!(game.screen, Screen::Race | Screen::Pause | Screen::PauseGfx | Screen::PauseSfx | Screen::PauseHud | Screen::PauseControls | Screen::PauseCam);
        if self.race.active() && (!racing || game.race.is_none()) {
            self.race.stop();
            self.music = None;
        }
        self.movie(game);
        if !game.screen.is_movie() {
            self.music(game);
        }
        if let (Some(r), false, Screen::Race) = (&mut game.race, self.race.active(), game.screen) {
            // #335 the sounds the loading worker decoded
            if let Some(pre) = r.sounds.take() {
                self.race.adopt(pre);
            }
            self.race.start(game.tracks[r.track].n, &r.sess.race);
        }
        if let Some(r) = &game.race {
            if game.screen == Screen::Race && !game.frozen && !r.pb.as_ref().is_some_and(|p| r.watch.is_some() && game.settings.view.modern_hud() && p.silent()) {
                let ear = Ear { cam: if r.sess.top_down() { r.sess.base_camera() } else { r.cam }, focus: r.sess.focus, in_car: r.sess.cam == CamMode::InCar, cine: None };
                self.race.update(&r.sess, &ear, dt);
            } else {
                self.race.silence();
            }
        }
    }

    /// A movie's sound (#42, #43): a PCM stream in the music slot at the CD music volume. It also
    /// gives the movie its clock, the samples the mixer has played; without a sound device the
    /// movie runs on frame time. The CD track comes back after the last movie.
    fn movie(&mut self, game: &mut Game) {
        match (&mut game.movie, game.screen) {
            (Some(m), scr) if scr.is_movie() => {
                if m.is_held() {
                    match m.audio_rate().filter(|_| self.audio.has_device()) {
                        Some(rate) => {
                            let (stream, feed) = MusicStream::pcm(rate, 0.0);
                            self.audio.with(|x| x.set_music(Some(stream)));
                            m.attach_audio(feed);
                            self.movie_sound = true;
                            // the original stops the CD before a result movie (0x40f3f4); the menu loop must not play under it
                            if let Some(v) = self.music.take() {
                                self.audio.stop(v);
                            }
                        }
                        None => m.use_timer(),
                    }
                }
                let v = if game.settings.music { MUSIC_VOLUME * self.mus } else { 0.0 };
                self.audio.with(|x| x.set_music_volume(v));
            }
            _ if self.movie_sound => {
                self.audio.with(|x| x.set_music(None));
                self.movie_sound = false;
                self.cd_playing = None; // `music` picks the screen's track again
            }
            _ => {}
        }
    }

    /// The CD track this screen wants: the race's CDTrack, results, or the menu track.
    /// Loading keeps whatever plays.
    fn wanted_cd(game: &Game) -> Option<Option<u32>> {
        if !game.settings.music {
            return Some(None);
        }
        Some(match game.screen {
            Screen::Loading => return None,
            Screen::Race | Screen::Pause | Screen::PauseGfx | Screen::PauseSfx | Screen::PauseHud | Screen::PauseControls | Screen::PauseCam => game.race.as_ref().map(|r| game.race_cd_track(r.track)),
            Screen::Results => Some(CD_RESULTS),
            // the song-to-track table's Enter plays the picked song until the screen is left
            Screen::Sound if game.cd_preview.is_some() => game.cd_preview,
            _ => Some(game.settings.menu_cd_track),
        })
    }

    /// Stream ripped CD music from Music/, or loop Menyloop.wav in the menus without it.
    fn music(&mut self, game: &Game) {
        let Some(want) = Self::wanted_cd(game) else { return };
        let want = want.filter(|n| self.cd.contains_key(n));
        if want != self.cd_playing {
            let stream = want.and_then(|n| self.cd.get(&n)).map(|p| MusicStream::open(p, game.settings.cd_loop, MUSIC_VOLUME));
            self.audio.with(|m| m.set_music(stream));
            self.cd_playing = want;
        }
        // Menyloop.wav (loaded at race init, 0x45880c) when there is no menu track to stream
        let in_menus = !matches!(game.screen, Screen::Race | Screen::Pause | Screen::PauseGfx | Screen::PauseSfx | Screen::PauseHud | Screen::PauseControls | Screen::PauseCam | Screen::Loading);
        let fallback = in_menus && game.settings.music && self.cd_playing.is_none();
        match (fallback, self.music) {
            (true, None) => self.music = self.play("menyloop.wav", Params { volume: 0.7, looping: true, ..Default::default() }),
            (false, Some(v)) => {
                self.audio.stop(v);
                self.music = None;
            }
            _ => {}
        }
        // music volume, quieter under the in-race menus
        let paused = matches!(game.screen, Screen::Pause | Screen::PauseGfx | Screen::PauseSfx | Screen::PauseHud | Screen::PauseControls | Screen::PauseCam);
        let v = MUSIC_VOLUME * self.mus * if paused { 0.4 } else { 1.0 };
        self.audio.with(|m| m.set_music_volume(v));
        if let Some(m) = self.music {
            self.audio.set(m, Params { volume: 0.7 * self.mus, looping: true, ..Default::default() });
        }
    }
}
