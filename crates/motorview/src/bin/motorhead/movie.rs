//! Movie player (#42, #43): plays one movie of `movies/movies.pak` through `motor_data::smk`.
//!
//! The original hands the file to RAD's smackw32 (`0x48ae28` open, `0x48abe0` one frame per call,
//! 02-architecture "Movies"); here the decoder runs on the game thread (0.4-0.9 ms a frame) and
//! the sound goes to the mixer as a PCM stream (`motor_audio::MusicStream::pcm`).
//!
//! The Smacker files carry their sound ahead of the picture: frame 0 holds 26 frames' worth, every
//! later frame one, the last 25 none [O, movies 0-2]. So the player decodes `LEAD` frames ahead of
//! the picture on screen (the pictures wait in a ring) and the audio device's consumed-sample
//! count is the movie's clock, which keeps picture and sound together.

use motor_audio::PcmFeed;
use motor_data::smk::Smacker;
use motor_data::{fs, pak};
use std::collections::VecDeque;
use std::fs::File;
use std::path::Path;

/// Frames decoded ahead of the one on screen: the sound's lead (26 frames in frame 0, 25 ahead of the rest).
const LEAD: u32 = 26;

/// What the original's 640x480 16-bit mode shows the movie in, in canvas pixels.
pub const SCREEN: (u32, u32) = (640, 480);

struct Pic {
    pixels: Vec<u8>,
    palette: Box<[[u8; 3]; 256]>,
}

enum Clock {
    /// advances with the game's frame time (headless shots, no sound device)
    Timer,
    /// attached by the sound output once it has a stream for this movie
    Held,
    /// the sound's consumed samples
    Audio(PcmFeed),
}

pub struct Movie {
    /// `movies.pak` entry
    pub index: usize,
    smk: Smacker<pak::Section<File>>,
    y_double: bool,
    clock: Clock,
    /// decoded pictures, `ring[0]` is on screen (frame `shown`)
    ring: VecDeque<Pic>,
    spare: Vec<Pic>,
    shown: u32,
    seconds: f64,
    /// sample frames given to the sound
    pushed: u64,
    ended: bool,
}

impl Movie {
    /// Open `index` of the install's `movies/movies.pak`; the clock is the game's frame time until
    /// `hold` / `attach_audio` change it.
    pub fn open(root: &Path, index: usize) -> Result<Movie, String> {
        let path = fs::find_ci(root, "movies/movies.pak").ok_or("no movies/movies.pak")?;
        let mut file = File::open(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        let e = pak::read_dir(&mut file).map_err(|e| e.to_string())?.into_iter().nth(index).ok_or_else(|| format!("movies.pak has no entry {index}"))?;
        let smk = Smacker::open(pak::Section::new(file, &e).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
        Ok(Movie { index, smk, y_double: e.y_double(), clock: Clock::Timer, ring: VecDeque::new(), spare: Vec::new(), shown: 0, seconds: 0.0, pushed: 0, ended: false })
    }

    /// Wait for the sound output to attach the audio clock (nothing advances until then).
    pub fn hold(&mut self) {
        self.clock = Clock::Held;
    }

    pub fn is_held(&self) -> bool {
        matches!(self.clock, Clock::Held)
    }

    /// Sample rate of the sound track, None when the movie is silent.
    pub fn audio_rate(&self) -> Option<u32> {
        self.smk.info().audio[0].map(|a| a.rate)
    }

    /// Give the sound to `feed` and use its played samples as the clock.
    pub fn attach_audio(&mut self, feed: PcmFeed) {
        self.clock = Clock::Audio(feed);
    }

    /// Fall back to the game's frame time (no sound device).
    pub fn use_timer(&mut self) {
        self.clock = Clock::Timer;
    }

    pub fn size(&self) -> (u32, u32) {
        (self.smk.info().width, self.smk.info().height)
    }

    /// Where the picture sits on the 640x480 screen: centred, every line doubled when the
    /// original's open flags say so (0x48ae28 centres by the buffer size; SMACKW32 0x10004b69 doubles the height).
    pub fn rect(&self) -> (f32, f32, f32, f32) {
        let (w, h) = self.size();
        let h = if self.y_double { h * 2 } else { h };
        (((SCREEN.0 - w) / 2) as f32, ((SCREEN.1.saturating_sub(h)) / 2) as f32, w as f32, h as f32)
    }

    fn frame_secs(&self) -> f64 {
        self.smk.frame_duration().as_secs_f64()
    }

    /// The frame on screen.
    pub fn shown(&self) -> u32 {
        self.shown
    }

    /// Movie time in seconds.
    pub fn time(&self) -> f64 {
        match &self.clock {
            Clock::Audio(f) => f.played() as f64 / self.audio_rate().unwrap_or(44100) as f64,
            _ => self.seconds,
        }
    }

    pub fn finished(&self) -> bool {
        self.ended && self.ring.len() <= 1 && {
            match &self.clock {
                // the mixer keeps the last sample frame back
                Clock::Audio(f) => f.played() + 2 >= self.pushed,
                _ => self.seconds >= self.smk.info().frames as f64 * self.frame_secs(),
            }
        }
    }

    /// Advance the clock by `dt` seconds (the Timer clock) and decode what is due.
    pub fn update(&mut self, dt: f32) {
        match self.clock {
            Clock::Held => return,
            Clock::Timer => self.seconds += dt as f64,
            Clock::Audio(_) => self.seconds = self.time(),
        }
        let want = (self.seconds / self.frame_secs()) as u32;
        let frames = self.smk.info().frames;
        let target = (want + LEAD).min(frames.saturating_sub(1));
        while !self.ended && self.smk.frame_index() <= target {
            match self.smk.next_frame() {
                Ok(true) => {
                    if let Clock::Audio(f) = &self.clock {
                        let a = self.smk.audio(0);
                        f.push(a);
                        self.pushed += a.len() as u64 / 2;
                    }
                    let mut pic = self.spare.pop().unwrap_or_else(|| Pic { pixels: Vec::new(), palette: Box::new([[0; 3]; 256]) });
                    pic.pixels.clear();
                    pic.pixels.extend_from_slice(self.smk.pixels());
                    *pic.palette = *self.smk.palette();
                    self.ring.push_back(pic);
                }
                Ok(false) => self.ended = true,
                Err(e) => {
                    eprintln!("movie {}: {e}", self.index);
                    self.ended = true;
                }
            }
        }
        if self.smk.frame_index() >= frames {
            self.ended = true;
        }
        while self.shown < want && self.ring.len() > 1 {
            self.spare.extend(self.ring.pop_front());
            self.shown += 1;
        }
    }

    /// The picture on screen as RGBA into `out`; returns its size. CLASSIC = what the original's
    /// 16-bit display shows (colours cut to 5-6-5 bits [I], lines doubled), REMASTER = the decoded
    /// picture in full colour (the screen stretches it).
    pub fn picture(&self, classic: bool, out: &mut Vec<u8>) -> (u32, u32) {
        let (w, h) = self.size();
        out.clear();
        let Some(pic) = self.ring.front() else { return (0, 0) };
        let lut: Vec<[u8; 4]> = pic.palette.iter().map(|&[r, g, b]| if classic { [expand(r, 5), expand(g, 6), expand(b, 5), 255] } else { [r, g, b, 255] }).collect();
        let times = if classic && self.y_double { 2 } else { 1 };
        out.reserve((w * h * times * 4) as usize);
        for row in pic.pixels.chunks_exact(w as usize) {
            for _ in 0..times {
                out.extend(row.iter().flat_map(|&p| lut[p as usize]));
            }
        }
        (w, h * times)
    }
}

/// An 8-bit channel cut to `bits` bits and expanded back the way a 16-bit surface shows it.
fn expand(v: u8, bits: u32) -> u8 {
    let c = (v >> (8 - bits)) as u32;
    ((c << (8 - bits)) | (c >> (2 * bits - 8))) as u8
}

#[cfg(test)]
mod tests {
    use super::{expand, Movie};
    use motor_audio::{Mixer, MusicStream};
    use std::path::PathBuf;

    #[test]
    fn sixteen_bit_channels_reach_full_range() {
        for bits in [5, 6] {
            assert_eq!(expand(0, bits), 0);
            assert_eq!(expand(255, bits), 255);
        }
        assert_eq!(expand(0x80, 5), 0x84); // 10000 -> 10000100
        assert_eq!(expand(0x80, 6), 0x82); // 100000 -> 10000010
    }

    /// Movie 1 (126 frames, 5.04 s) driven by a mixer without a device: the picture follows the
    /// consumed samples and the movie ends at its real length. Skipped without the install.
    #[test]
    fn audio_clock_keeps_picture_and_sound_together() {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../installed_game");
        let Ok(mut m) = Movie::open(&root, 1) else {
            eprintln!("skipped: no installed_game next to the workspace");
            return;
        };
        let (stream, feed) = MusicStream::pcm(44100, 1.0);
        let mut mixer = Mixer::new(44100);
        mixer.set_music(Some(stream));
        m.hold();
        m.attach_audio(feed);
        let mut out = vec![0.0f32; 2 * 735]; // 1/60 s
        let mut steps = 0;
        while !m.finished() && steps < 1000 {
            mixer.render(&mut out, 2);
            m.update(1.0 / 60.0);
            steps += 1;
            // never more than a frame off the sound that has been played
            let due = (m.time() / 0.04) as u32;
            assert!(m.shown() == due.min(125) || m.shown() + 1 == due.min(125), "step {steps}: shown {} due {due}", m.shown());
        }
        assert!(m.finished(), "movie did not finish");
        let secs = steps as f64 / 60.0;
        assert!((secs - 5.04).abs() < 0.1, "ran {secs} s");
        assert_eq!(m.shown(), 125);
    }
}
