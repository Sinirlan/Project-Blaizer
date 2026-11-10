//! Sample playback for the Motorhead reimplementation: a WAV reader, a software mixer with
//! per-voice pitch, volume, pan, looping and loop points (the game's DirectSound usage, see
//! re-notes/07-audio.md), and a cpal output stream. The mixer works without a sound device,
//! so a missing or busy device only silences the game.

use std::sync::{Arc, Mutex};

pub mod music;
pub mod wav;

pub use music::{MusicStream, PcmFeed};

/// A decoded sound: mono samples in -1..1 at `rate` Hz.
#[derive(Clone)]
pub struct Clip {
    pub rate: u32,
    pub data: Arc<Vec<f32>>,
    /// loop region in samples (start, end); the whole clip when None
    pub loop_points: Option<(usize, usize)>,
}

impl Clip {
    pub fn seconds(&self) -> f32 {
        self.data.len() as f32 / self.rate.max(1) as f32
    }
}

/// Playback parameters, changeable while a voice plays.
#[derive(Clone, Copy, Debug)]
pub struct Params {
    pub volume: f32,
    /// playback rate multiplier (1 = the clip's own rate)
    pub pitch: f32,
    /// -1 left .. +1 right
    pub pan: f32,
    pub looping: bool,
}

impl Default for Params {
    fn default() -> Self {
        Params { volume: 1.0, pitch: 1.0, pan: 0.0, looping: false }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct VoiceId {
    slot: u32,
    generation: u32,
}

struct Voice {
    clip: Clip,
    pos: f64,
    p: Params,
    generation: u32,
    alive: bool,
}

/// Software mixer. `render` fills interleaved output frames.
pub struct Mixer {
    voices: Vec<Voice>,
    pub out_rate: u32,
    pub master: f32,
    /// tape speed (#426): every voice's step is multiplied by it (1 = off; the music stream is not affected)
    pub rate: f32,
    generations: u32,
    music: Option<MusicStream>,
}

impl Mixer {
    pub fn new(out_rate: u32) -> Mixer {
        Mixer { voices: Vec::new(), out_rate, master: 1.0, rate: 1.0, generations: 0, music: None }
    }

    pub fn play(&mut self, clip: &Clip, p: Params) -> VoiceId {
        self.generations = self.generations.wrapping_add(1);
        let v = Voice { clip: clip.clone(), pos: 0.0, p, generation: self.generations, alive: true };
        let slot = match self.voices.iter().position(|v| !v.alive) {
            Some(i) => {
                self.voices[i] = v;
                i
            }
            None => {
                self.voices.push(v);
                self.voices.len() - 1
            }
        };
        VoiceId { slot: slot as u32, generation: self.generations }
    }

    fn get(&mut self, id: VoiceId) -> Option<&mut Voice> {
        self.voices.get_mut(id.slot as usize).filter(|v| v.alive && v.generation == id.generation)
    }

    pub fn is_playing(&self, id: VoiceId) -> bool {
        self.voices.get(id.slot as usize).is_some_and(|v| v.alive && v.generation == id.generation)
    }

    pub fn set(&mut self, id: VoiceId, p: Params) {
        if let Some(v) = self.get(id) {
            v.p = p;
        }
    }

    pub fn stop(&mut self, id: VoiceId) {
        if let Some(v) = self.get(id) {
            v.alive = false;
        }
    }

    /// Replace the music stream (None stops it).
    pub fn set_music(&mut self, m: Option<MusicStream>) {
        self.music = m;
    }

    pub fn set_music_volume(&mut self, v: f32) {
        if let Some(m) = &mut self.music {
            m.volume = v;
        }
    }

    pub fn music_finished(&self) -> bool {
        self.music.as_ref().is_none_or(|m| m.is_finished())
    }

    pub fn stop_all(&mut self) {
        self.voices.iter_mut().for_each(|v| v.alive = false);
    }

    pub fn voices_playing(&self) -> usize {
        self.voices.iter().filter(|v| v.alive).count()
    }

    /// Mix into `out` (interleaved, `channels` per frame), overwriting it.
    pub fn render(&mut self, out: &mut [f32], channels: usize) {
        out.iter_mut().for_each(|s| *s = 0.0);
        let channels = channels.max(1);
        let frames = out.len() / channels;
        for v in self.voices.iter_mut().filter(|v| v.alive) {
            let data = &v.clip.data;
            if data.is_empty() {
                v.alive = false;
                continue;
            }
            let step = v.p.pitch.max(0.0) as f64 * self.rate.max(0.0) as f64 * v.clip.rate as f64 / self.out_rate as f64;
            let (ls, le) = v.clip.loop_points.unwrap_or((0, data.len()));
            let le = le.min(data.len()).max(ls + 1);
            // equal-power pan
            let a = (v.p.pan.clamp(-1.0, 1.0) + 1.0) * std::f32::consts::FRAC_PI_4;
            let (gl, gr) = (a.cos() * v.p.volume * self.master, a.sin() * v.p.volume * self.master);
            for f in 0..frames {
                if v.p.looping {
                    while v.pos >= le as f64 {
                        v.pos -= (le - ls) as f64;
                    }
                }
                let i = v.pos as usize;
                if i >= data.len() {
                    v.alive = false;
                    break;
                }
                // linear interpolation; the sample after the loop end is the loop start
                let next = if v.p.looping && i + 1 >= le { ls } else { (i + 1).min(data.len() - 1) };
                let t = (v.pos - i as f64) as f32;
                let s = data[i] * (1.0 - t) + data[next] * t;
                if channels == 1 {
                    out[f] += s * (gl + gr) * std::f32::consts::FRAC_1_SQRT_2;
                } else {
                    out[f * channels] += s * gl;
                    out[f * channels + 1] += s * gr;
                }
                v.pos += step;
            }
        }
        if let Some(m) = &mut self.music {
            m.mix(out, channels, self.out_rate, self.master);
        }
        for s in out.iter_mut() {
            *s = s.clamp(-1.0, 1.0);
        }
    }
}

/// The sound device plus a shared mixer. Without a device the mixer still runs (silently).
pub struct Audio {
    pub mixer: Arc<Mutex<Mixer>>,
    _stream: Option<output::Stream>,
    /// why the output that was asked for could not be used (the system default plays instead); None while
    /// the pick, or the default when none was picked, plays
    fallback: Option<String>,
}

impl Audio {
    /// Open the system's default output; on failure, report it and continue silently.
    pub fn open() -> Audio {
        Audio::open_on(None)
    }

    /// Open the output with this saved id (`output_devices`), or the default for `None`. A pick that is
    /// gone or will not open falls back to the default, `fallback` says why.
    pub fn open_on(device: Option<&str>) -> Audio {
        let mut a = Audio { mixer: Arc::new(Mutex::new(Mixer::new(44100))), _stream: None, fallback: None };
        a.switch(device);
        a
    }

    /// Move the sound to another output while the mixer keeps playing (music, voices): the old stream
    /// stops first (some drivers allow one at a time), then the new one starts; the mixer's rate follows.
    /// Fallback as `open_on`.
    pub fn switch(&mut self, device: Option<&str>) {
        self._stream = None;
        self.fallback = None;
        match output::start(device, &self.mixer) {
            Ok(s) => self._stream = Some(s),
            Err(e) => {
                if let Some(id) = device {
                    eprintln!("audio: output {id}: {e} (using the system default)");
                    self.fallback = Some(e);
                    match output::start(None, &self.mixer) {
                        Ok(s) => self._stream = Some(s),
                        Err(e) => eprintln!("audio: {e} (continuing without sound)"),
                    }
                } else {
                    eprintln!("audio: {e} (continuing without sound)");
                }
            }
        }
    }

    /// Why the output that was asked for is not the one playing (see `open_on`).
    pub fn fallback(&self) -> Option<&str> {
        self.fallback.as_deref()
    }

    /// A sound device is playing (the mixer's clock runs).
    pub fn has_device(&self) -> bool {
        self._stream.is_some()
    }

    /// A mixer with no device (tests, headless screenshots).
    pub fn silent() -> Audio {
        Audio { mixer: Arc::new(Mutex::new(Mixer::new(44100))), _stream: None, fallback: None }
    }

    /// Another handle on the same mixer, without the device (the device stays with `self`).
    pub fn share(&self) -> Audio {
        Audio { mixer: self.mixer.clone(), _stream: None, fallback: None }
    }

    pub fn with<R>(&self, f: impl FnOnce(&mut Mixer) -> R) -> R {
        let mut m = self.mixer.lock().unwrap_or_else(|e| e.into_inner());
        f(&mut m)
    }

    pub fn play(&self, clip: &Clip, p: Params) -> VoiceId {
        self.with(|m| m.play(clip, p))
    }
    pub fn set(&self, id: VoiceId, p: Params) {
        self.with(|m| m.set(id, p))
    }
    pub fn stop(&self, id: VoiceId) {
        self.with(|m| m.stop(id))
    }
}

mod output;

pub use output::{default_output_name, output_devices, OutputDevice};

#[cfg(test)]
mod tests {
    use super::*;

    fn clip(n: usize) -> Clip {
        Clip { rate: 100, data: Arc::new((0..n).map(|i| i as f32 / n as f32).collect()), loop_points: None }
    }

    #[test]
    fn one_shot_ends() {
        let mut m = Mixer::new(100);
        let id = m.play(&clip(10), Params::default());
        let mut out = vec![0.0; 2 * 20];
        m.render(&mut out, 2);
        assert!(!m.is_playing(id));
        assert!(out[2 * 5] > 0.0 && out[2 * 15] == 0.0);
    }

    #[test]
    fn loops_and_pitch() {
        let mut m = Mixer::new(100);
        let id = m.play(&clip(10), Params { looping: true, pitch: 2.0, ..Default::default() });
        let mut out = vec![0.0; 2 * 50];
        m.render(&mut out, 2);
        assert!(m.is_playing(id));
        // pitch 2 plays the 10-sample clip in 5 frames: frame 5 wraps to the start
        assert!(out[2 * 5].abs() < 1e-3);
    }

    #[test]
    fn tape_rate() {
        let mut m = Mixer::new(100);
        m.rate = 0.5;
        let id = m.play(&clip(10), Params::default());
        let mut out = vec![0.0; 2 * 15];
        m.render(&mut out, 2);
        // rate 0.5 plays the 10-sample clip in 20 frames: still going after 15
        assert!(m.is_playing(id));
        m.render(&mut out, 2);
        assert!(!m.is_playing(id));
    }

    #[test]
    fn pan_right() {
        let mut m = Mixer::new(100);
        m.play(&clip(10), Params { pan: 1.0, ..Default::default() });
        let mut out = vec![0.0; 2 * 8];
        m.render(&mut out, 2);
        assert!(out[2 * 5].abs() < 1e-6 && out[2 * 5 + 1] > 0.0);
    }
}
