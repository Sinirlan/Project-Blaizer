//! Sample playback for the Motorhead reimplementation: a WAV reader, a software mixer with
//! per-voice pitch, volume, pan, looping and loop points (the game's DirectSound usage, see
//! re-notes/07-audio.md), and a cpal output stream. The mixer works without a sound device,
//! so a missing or busy device only silences the game.

use std::sync::{Arc, Mutex};

pub mod music;
pub mod noise;
pub mod resample;
pub mod tone;
pub mod wav;

pub use music::{MusicStream, PcmFeed};
pub use tone::Tone;

/// How the mixer reads a clip between its samples.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Interp {
    /// the game's own linear interpolation (CLASSIC)
    #[default]
    Linear,
    /// REMASTER: band-limited windowed sinc (`resample`), no imaging when pitched down, no aliasing when pitched up
    Sinc,
}

/// A decoded sound: mono samples in -1..1 at `rate` Hz.
#[derive(Clone)]
pub struct Clip {
    pub rate: u32,
    pub data: Arc<Vec<f32>>,
    /// loop region in samples (start, end); the whole clip when None
    pub loop_points: Option<(usize, usize)>,
    /// how a voice of this clip is read (the game's linear path by default)
    pub interp: Interp,
    /// REMASTER: time constant in seconds with which a voice of this clip glides to a new volume, pan and
    /// pitch (one pole, no zipper at the frame rate); 0 = the game's instant set (CLASSIC)
    pub glide: f32,
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
    /// the glided (left gain, right gain, step) of a clip with `glide`; None until its first render
    sm: Option<(f32, f32, f64)>,
    /// REMASTER: the voice's tone (`set_tone`); None = not filtered (CLASSIC)
    tone: Option<Tone>,
    eq: tone::Shelves,
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
    /// Headless rendering (#490): the music waits for its decoder thread instead of playing silence while it is behind,
    /// so a recording does not depend on thread timing. Off for a sound device, which must never block.
    pub offline: bool,
}

impl Mixer {
    pub fn new(out_rate: u32) -> Mixer {
        Mixer { voices: Vec::new(), out_rate, master: 1.0, rate: 1.0, generations: 0, music: None, offline: false }
    }

    pub fn play(&mut self, clip: &Clip, p: Params) -> VoiceId {
        self.generations = self.generations.wrapping_add(1);
        let v = Voice { clip: clip.clone(), pos: 0.0, p, generation: self.generations, alive: true, sm: None, tone: None, eq: tone::Shelves::default() };
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

    /// REMASTER: filter a voice through `tone` (None: unfiltered). Coefficients change only when the tone does;
    /// a change while it plays crossfades from the old filter, so it does not click.
    pub fn set_tone(&mut self, id: VoiceId, tone: Option<Tone>) {
        if let Some(v) = self.get(id) {
            v.tone = tone;
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

    /// Stop every voice (the music plays on).
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
            let tstep = v.p.pitch.max(0.0) as f64 * self.rate.max(0.0) as f64 * v.clip.rate as f64 / self.out_rate as f64;
            let (ls, le) = v.clip.loop_points.unwrap_or((0, data.len()));
            let le = le.min(data.len()).max(ls + 1);
            // equal-power pan
            let a = (v.p.pan.clamp(-1.0, 1.0) + 1.0) * std::f32::consts::FRAC_PI_4;
            let (tgl, tgr) = (a.cos() * v.p.volume * self.master, a.sin() * v.p.volume * self.master);
            // REMASTER glide: the one-pole smoothing of volume, pan and pitch; 0 leaves the targets as they are (CLASSIC)
            let k = if v.clip.glide > 0.0 { 1.0 - (-1.0 / (v.clip.glide as f64 * self.out_rate as f64)).exp() } else { 1.0 };
            let (mut gl, mut gr, mut step) = if k < 1.0 { v.sm.unwrap_or((tgl, tgr, tstep)) } else { (tgl, tgr, tstep) };
            if let Some(t) = v.tone {
                v.eq.retune(t, self.out_rate);
            }
            for f in 0..frames {
                if k < 1.0 {
                    gl += (tgl - gl) * k as f32;
                    gr += (tgr - gr) * k as f32;
                    step += (tstep - step) * k;
                    // settled at silence: stop paying for the kernel
                    if tgl == 0.0 && gl.abs() < 1e-6 {
                        gl = 0.0;
                    }
                    if tgr == 0.0 && gr.abs() < 1e-6 {
                        gr = 0.0;
                    }
                }
                let silent = gl == 0.0 && gr == 0.0;
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
                let s = match v.clip.interp {
                    Interp::Linear => {
                        // linear interpolation; the sample after the loop end is the loop start
                        let next = if v.p.looping && i + 1 >= le { ls } else { (i + 1).min(data.len() - 1) };
                        let t = (v.pos - i as f64) as f32;
                        data[i] * (1.0 - t) + data[next] * t
                    }
                    // REMASTER: a silent voice costs nothing
                    Interp::Sinc if silent => 0.0,
                    Interp::Sinc => resample::sinc_at(data, v.pos, resample::cutoff(step), v.p.looping.then_some((ls, le))),
                };
                // REMASTER tone after the resampling
                let s = if v.tone.is_some() { v.eq.process(s) } else { s };
                if channels == 1 {
                    out[f] += s * (gl + gr) * std::f32::consts::FRAC_1_SQRT_2;
                } else {
                    out[f * channels] += s * gl;
                    out[f * channels + 1] += s * gr;
                }
                v.pos += step;
            }
            if k < 1.0 {
                v.sm = Some((gl, gr, step));
            }
        }
        if let Some(m) = &mut self.music {
            m.mix(out, channels, self.out_rate, self.master, self.offline);
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
    pub fn set_tone(&self, id: VoiceId, tone: Option<Tone>) {
        self.with(|m| m.set_tone(id, tone))
    }
}

mod output;

pub use output::{default_output_name, output_devices, OutputDevice};

#[cfg(test)]
mod tests {
    use super::*;

    fn clip(n: usize) -> Clip {
        Clip { rate: 100, data: Arc::new((0..n).map(|i| i as f32 / n as f32).collect()), loop_points: None, interp: Interp::Linear, glide: 0.0 }
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

    /// amplitude of `freq` in `x` (Hann-windowed single-bin DFT)
    fn tone(x: &[f32], freq: f64, rate: f64) -> f64 {
        let n = x.len();
        let (mut re, mut im, mut ws) = (0.0, 0.0, 0.0);
        for (i, &v) in x.iter().enumerate() {
            let w = 0.5 - 0.5 * (2.0 * std::f64::consts::PI * i as f64 / n as f64).cos();
            let a = 2.0 * std::f64::consts::PI * freq * i as f64 / rate;
            re += w * v as f64 * a.cos();
            im -= w * v as f64 * a.sin();
            ws += w;
        }
        2.0 * (re * re + im * im).sqrt() / ws
    }

    /// a 22 050 Hz sine clip of `secs`, amplitude 0.5, played at `pitch` into 44.1 kHz; the middle of the output
    fn play_sine(freq: f64, pitch: f32, interp: Interp) -> Vec<f32> {
        let data: Vec<f32> = (0..22050 * 2).map(|i| 0.5 * (2.0 * std::f64::consts::PI * freq * i as f64 / 22050.0).sin() as f32).collect();
        let c = Clip { rate: 22050, data: Arc::new(data), loop_points: None, interp, glide: 0.0 };
        let mut m = Mixer::new(44100);
        m.play(&c, Params { pitch, pan: 0.0, ..Default::default() });
        let mut out = vec![0.0; 2 * 40000];
        m.render(&mut out, 2);
        // left channel at pan 0 is cos(pi/4) of the voice; undo it for the amplitudes
        out.iter().step_by(2).skip(4000).take(32768).map(|s| s * std::f32::consts::SQRT_2).collect()
    }

    #[test]
    fn sinc_has_no_image_when_pitched_down() {
        // 5 kHz at pitch 0.5 plays as 2.5 kHz; the images sit at 11.025 +- 2.5 kHz
        let (lin, sinc) = (play_sine(5000.0, 0.5, Interp::Linear), play_sine(5000.0, 0.5, Interp::Sinc));
        let main = tone(&sinc, 2500.0, 44100.0);
        assert!((main - 0.5).abs() < 0.01, "main tone {main}");
        let db = |x: &[f32]| 20.0 * (tone(x, 8525.0, 44100.0) / main).log10();
        assert!(db(&sinc) < -60.0, "sinc image {} dB", db(&sinc));
        assert!(db(&lin) > -35.0, "linear image {} dB", db(&lin));
    }

    #[test]
    fn sinc_does_not_alias_when_pitched_up() {
        // 10.5 kHz at pitch 2.4 would be 25.2 kHz: past the output's Nyquist, it folds to 18.9 kHz with linear
        let (lin, sinc) = (play_sine(10500.0, 2.4, Interp::Linear), play_sine(10500.0, 2.4, Interp::Sinc));
        let (a_lin, a_sinc) = (tone(&lin, 18900.0, 44100.0), tone(&sinc, 18900.0, 44100.0));
        assert!(a_sinc < 0.5 * 0.01, "sinc alias {} dB", 20.0*(a_sinc/0.5).log10());
        assert!(a_lin > 10.0 * a_sinc, "linear alias {a_lin}");
        // an in-band tone passes at full level: 5 kHz at 2.4 is 12 kHz
        assert!((tone(&play_sine(5000.0, 2.4, Interp::Sinc), 12000.0, 44100.0) - 0.5).abs() < 0.02);
    }

    #[test]
    fn sinc_loops_and_ends_like_linear() {
        let sinc = |c: Clip| Clip { interp: Interp::Sinc, ..c };
        let mut m = Mixer::new(100);
        let id = m.play(&sinc(clip(10)), Params::default());
        let mut out = vec![0.0; 2 * 20];
        m.render(&mut out, 2);
        assert!(!m.is_playing(id));
        assert!(out[2 * 15] == 0.0);
        let id = m.play(&sinc(clip(10)), Params { looping: true, ..Default::default() });
        m.render(&mut out, 2);
        m.render(&mut out, 2);
        assert!(m.is_playing(id));
    }

    #[test]
    fn seamless_loop_has_no_jump_at_the_wrap() {
        // a noisy tone whose length is not a whole period: the plain wrap jumps, the seam does not
        let mut seed = 1u32;
        let mut noise = || {
            seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
            (seed >> 16) as f32 / 32768.0 - 1.0
        };
        let n = 20000;
        let data: Vec<f32> = (0..n).map(|i| 0.6 * (i as f32 * 0.0313).sin() + 0.05 * noise()).collect();
        let body_step = data.windows(2).map(|w| (w[1] - w[0]).abs()).fold(0.0, f32::max);
        assert!((data[n - 1] - data[0]).abs() > 0.3 * body_step, "test clip should not loop by chance");
        let out = resample::seamless_loop(&data, 441, 1323);
        assert!(out.len() <= n && out.len() >= n - 1323 - 441);
        let wrap = (out[out.len() - 1] - out[0]).abs();
        let out_step = out.windows(2).map(|w| (w[1] - w[0]).abs()).fold(0.0, f32::max);
        assert!(wrap <= out_step, "wrap {wrap} vs largest step {out_step}");
        // the body past the fade is untouched
        assert_eq!(out[441..], data[441..out.len()]);
        // no click: the wrap is as smooth as the tone itself
        assert!(out_step < 1.5 * body_step);
    }

    /// a looping 1 kHz sine clip (22 050 Hz) with `glide`, Sinc or Linear
    fn glide_clip(interp: Interp, glide: f32) -> Clip {
        let data: Vec<f32> = (0..22050).map(|i| (2.0 * std::f64::consts::PI * 1000.0 * i as f64 / 22050.0).sin() as f32).collect();
        Clip { rate: 22050, data: Arc::new(data), loop_points: None, interp, glide }
    }

    #[test]
    fn glide_zero_sets_at_once() {
        let mut a = Mixer::new(44100);
        let mut b = Mixer::new(44100);
        let (ia, ib) = (a.play(&glide_clip(Interp::Linear, 0.0), Params { volume: 0.0, looping: true, ..Default::default() }), b.play(&glide_clip(Interp::Linear, 0.0), Params { volume: 0.0, looping: true, ..Default::default() }));
        let (mut oa, mut ob) = (vec![0.0; 2 * 441], vec![0.0; 2 * 441]);
        a.render(&mut oa, 2);
        b.render(&mut ob, 2);
        a.set(ia, Params { volume: 1.0, pitch: 1.3, looping: true, ..Default::default() });
        b.set(ib, Params { volume: 1.0, pitch: 1.3, looping: true, ..Default::default() });
        a.render(&mut oa, 2);
        b.render(&mut ob, 2);
        assert_eq!(oa, ob);
        // the new volume is there from the first frame
        assert!(oa[..40].iter().any(|s| s.abs() > 0.3));
    }

    #[test]
    fn glide_ramps_volume() {
        for interp in [Interp::Linear, Interp::Sinc] {
            let mut m = Mixer::new(44100);
            let id = m.play(&glide_clip(interp, 0.010), Params { volume: 0.0, looping: true, ..Default::default() });
            let mut out = vec![0.0; 2 * 441];
            m.render(&mut out, 2);
            m.set(id, Params { volume: 1.0, looping: true, ..Default::default() });
            let mut out = vec![0.0; 2 * 4410];
            m.render(&mut out, 2);
            let peak = |r: std::ops::Range<usize>| out[r].iter().step_by(2).fold(0.0f32, |a, s| a.max(s.abs()));
            let full = std::f32::consts::FRAC_1_SQRT_2;
            // the first 0.5 ms is far below the target, 5 time constants (50 ms) in, it is there
            assert!(peak(0..44) < 0.05 * full, "{interp:?} start {}", peak(0..44));
            assert!(peak(2 * 3500..2 * 4410) > 0.98 * full, "{interp:?} end {}", peak(2 * 3500..2 * 4410));
        }
    }

    #[test]
    fn glide_makes_a_pitch_step_continuous() {
        for interp in [Interp::Linear, Interp::Sinc] {
            let step_max = |glide: f32| {
                let mut m = Mixer::new(44100);
                let id = m.play(&glide_clip(interp, glide), Params { looping: true, ..Default::default() });
                let mut out = vec![0.0; 2 * 4410];
                m.render(&mut out, 2);
                m.set(id, Params { pitch: 1.5, looping: true, ..Default::default() });
                m.render(&mut out, 2);
                let l: Vec<f32> = out.iter().step_by(2).copied().collect();
                l.windows(2).map(|w| (w[1] - w[0]).abs()).fold(0.0f32, f32::max)
            };
            // a 1 kHz sine's own largest step is 2 pi f / rate = 0.142 x the gain 0.707; pitched 1.5 up, 1.5 x that
            let own = 1.5 * 2.0 * std::f32::consts::PI * 1000.0 / 44100.0 * 0.5 * 1.0;
            assert!(step_max(0.020) < own * 1.5, "{interp:?} glided {}", step_max(0.020));
        }
    }

    /// a looping sine at the output rate through a voice with `t`, the amplitude after 0.5 s of settling
    fn shelved(freq: f64, t: Option<Tone>) -> f64 {
        let n = 44100;
        let data: Vec<f32> = (0..n).map(|i| (2.0 * std::f64::consts::PI * freq * i as f64 / n as f64).sin() as f32 * 0.25).collect();
        let c = Clip { rate: 44100, data: Arc::new(data), loop_points: None, interp: Interp::Linear, glide: 0.0 };
        let mut m = Mixer::new(44100);
        let id = m.play(&c, Params { looping: true, ..Default::default() });
        m.set_tone(id, t);
        let mut out = vec![0.0; 2 * 22050];
        m.render(&mut out, 2);
        m.render(&mut out, 2);
        let l: Vec<f32> = out.iter().step_by(2).copied().collect();
        tone(&l, freq, 44100.0)
    }

    #[test]
    fn tone_shelves_lift_their_band() {
        let db = |freq: f64, t: Tone| 20.0 * (shelved(freq, Some(t)) / shelved(freq, None)).log10();
        let low = Tone { low_hz: 250.0, low_db: 6.0, high_hz: 3000.0, high_db: 0.0, ..Default::default() };
        assert!((db(30.0, low) - 6.0).abs() < 0.5, "low shelf at 30 Hz {}", db(30.0, low));
        assert!(db(10000.0, low).abs() < 0.3, "low shelf at 10 kHz {}", db(10000.0, low));
        let high = Tone { low_hz: 250.0, low_db: 0.0, high_hz: 3000.0, high_db: -6.0, ..Default::default() };
        assert!((db(15000.0, high) + 6.0).abs() < 0.6, "high shelf at 15 kHz {}", db(15000.0, high));
        assert!(db(50.0, high).abs() < 0.3, "high shelf at 50 Hz {}", db(50.0, high));
        // a flat tone passes the signal unchanged
        assert!(db(1000.0, Tone { low_hz: 250.0, high_hz: 3000.0, ..Default::default() }).abs() < 0.01);
    }

    #[test]
    fn tone_passes_cut_their_side() {
        let db = |freq: f64, t: Tone| 20.0 * (shelved(freq, Some(t)) / shelved(freq, None)).log10();
        let flat = Tone { low_hz: 250.0, high_hz: 3000.0, ..Default::default() };
        // Butterworth: -3 dB at the corner, -24 dB two octaves past it, flat two octaves before it
        let lp = Tone { low_pass: 400.0, ..flat };
        assert!((db(400.0, lp) + 3.0).abs() < 1.0, "low-pass at 400 Hz {}", db(400.0, lp));
        assert!(db(1600.0, lp) < -22.0, "low-pass at 1600 Hz {}", db(1600.0, lp));
        assert!(db(50.0, lp).abs() < 0.3, "low-pass at 50 Hz {}", db(50.0, lp));
        let hp = Tone { high_pass: 1200.0, ..flat };
        assert!((db(1200.0, hp) + 3.0).abs() < 1.0, "high-pass at 1200 Hz {}", db(1200.0, hp));
        assert!(db(300.0, hp) < -22.0, "high-pass at 300 Hz {}", db(300.0, hp));
        assert!(db(10000.0, hp).abs() < 0.3, "high-pass at 10 kHz {}", db(10000.0, hp));
    }

    #[test]
    fn tone_change_mid_voice_does_not_click() {
        let data: Vec<f32> = (0..22050).map(|i| (2.0 * std::f64::consts::PI * 200.0 * i as f64 / 22050.0).sin() as f32 * 0.5).collect();
        let c = Clip { rate: 22050, data: Arc::new(data), loop_points: None, interp: Interp::Sinc, glide: 0.0 };
        let mut m = Mixer::new(44100);
        let id = m.play(&c, Params { looping: true, ..Default::default() });
        m.set_tone(id, Some(Tone { low_hz: 250.0, low_db: -3.0, high_hz: 3000.0, high_db: 3.0, ..Default::default() }));
        let mut a = vec![0.0; 2 * 4410];
        m.render(&mut a, 2);
        m.set_tone(id, Some(Tone { low_hz: 250.0, low_db: 4.0, high_hz: 3000.0, high_db: -2.0, low_pass: 300.0, high_pass: 40.0 }));
        let mut b = vec![0.0; 2 * 4410];
        m.render(&mut b, 2);
        let l: Vec<f32> = a.iter().chain(b.iter()).step_by(2).copied().collect();
        let step = l.windows(2).map(|w| (w[1] - w[0]).abs()).fold(0.0f32, f32::max);
        // a 200 Hz sine of peak 0.5 x 0.707 x at most +4 dB moves at most 2 pi f / rate x that per sample
        let own = 2.0 * std::f32::consts::PI * 200.0 / 44100.0 * 0.5 * 0.7072 * 1.6;
        assert!(step < own * 1.2, "step {step} vs {own}");
    }
}
