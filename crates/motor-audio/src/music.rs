//! Streaming music: a decoder thread (symphonia: MP3, Ogg Vorbis, FLAC, WAV) sends stereo
//! chunks through a bounded channel; the mixer resamples them to the output rate.

use std::collections::VecDeque;
use std::fs::File;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{channel, sync_channel, Receiver, Sender, SyncSender, TryRecvError};
use std::sync::Arc;
use symphonia::core::codecs::audio::AudioDecoderOptions;
use symphonia::core::formats::probe::Hint;
use symphonia::core::formats::{FormatOptions, TrackType};
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;

/// Linear resampling over a queue of stereo frames (the music stream's, shared): the frame at the read position is
/// interpolated between the first two queued frames.
#[derive(Default)]
struct Resampler {
    pub buf: VecDeque<[f32; 2]>,
    frac: f64,
}

impl Resampler {
    /// the frame at the read position; None with fewer than two frames queued
    #[inline]
    pub fn frame(&self) -> Option<(f32, f32)> {
        if self.buf.len() < 2 {
            return None;
        }
        let (a, b) = (self.buf[0], self.buf[1]);
        let t = self.frac as f32;
        Some((a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t))
    }

    /// moves the read position on by `step` source frames; the count of frames left behind
    #[inline]
    pub fn advance(&mut self, step: f64) -> u64 {
        self.frac += step;
        let mut n = 0;
        while self.frac >= 1.0 && !self.buf.is_empty() {
            self.frac -= 1.0;
            self.buf.pop_front();
            n += 1;
        }
        n
    }
}

/// Decoded audio: interleaved stereo at `rate`.
struct Chunk {
    rate: u32,
    data: Vec<f32>,
}

pub struct MusicStream {
    rx: Receiver<Chunk>,
    q: Resampler,
    rate: u32,
    finished: bool,
    /// fed by a decoder thread of its own (`open`), which `mix` may wait for; a `pcm` stream's feeder is the caller
    decoder: bool,
    /// source sample frames consumed by the mixer (what has actually been played, no underrun silence)
    played: Arc<AtomicU64>,
    pub volume: f32,
}

/// The sending end of a [`MusicStream::pcm`] stream (a movie's sound track).
pub struct PcmFeed {
    tx: Sender<Chunk>,
    rate: u32,
    played: Arc<AtomicU64>,
}

impl PcmFeed {
    /// Queue interleaved stereo samples; never blocks. Dropping the feed ends the stream.
    pub fn push(&self, samples: &[i16]) {
        let data = samples.iter().map(|&v| v as f32 / 32768.0).collect();
        let _ = self.tx.send(Chunk { rate: self.rate, data });
    }

    /// Sample frames the mixer has consumed so far: the movie's audio clock is this over the rate.
    pub fn played(&self) -> u64 {
        self.played.load(Ordering::Relaxed)
    }
}

/// Chunks queued ahead of the mixer (each is one decoded packet, ~26 ms of MP3).
const QUEUE: usize = 64;

impl MusicStream {
    /// Start decoding `path` on a background thread.
    pub fn open(path: &Path, looping: bool, volume: f32) -> MusicStream {
        let (tx, rx) = sync_channel(QUEUE);
        let path = path.to_path_buf();
        std::thread::Builder::new()
            .name("music".into())
            .spawn(move || decode_loop(&path, looping, &tx))
            .expect("spawn music thread");
        MusicStream { rx, q: Resampler::default(), rate: 44100, finished: false, decoder: true, played: Arc::default(), volume }
    }

    /// A stream fed by the caller with interleaved stereo `i16` at `rate` (no decoder thread).
    pub fn pcm(rate: u32, volume: f32) -> (MusicStream, PcmFeed) {
        let (tx, rx) = channel();
        let played = Arc::new(AtomicU64::new(0));
        let m = MusicStream { rx, q: Resampler::default(), rate, finished: false, decoder: false, played: played.clone(), volume };
        (m, PcmFeed { tx, rate, played })
    }

    pub fn is_finished(&self) -> bool {
        self.finished && self.q.buf.len() < 2
    }

    /// Queue decoded chunks until two frames are ready. `wait` (offline mixing, #490): block for the decoder thread
    /// rather than return with the queue short, which would play silence for as long as the decoder is late.
    fn fill(&mut self, wait: bool) {
        while self.q.buf.len() < 2 && !self.finished {
            let got = if wait && self.decoder { self.rx.recv().map_err(|_| TryRecvError::Disconnected) } else { self.rx.try_recv() };
            match got {
                Ok(c) => {
                    self.rate = c.rate;
                    self.q.buf.extend(c.data.as_chunks::<2>().0.iter().map(|f| [f[0], f[1]]));
                }
                Err(TryRecvError::Empty) => return,
                Err(TryRecvError::Disconnected) => self.finished = true,
            }
        }
    }

    /// Add into interleaved `out` at `out_rate`. `wait`: see `fill`.
    pub fn mix(&mut self, out: &mut [f32], channels: usize, out_rate: u32, master: f32, wait: bool) {
        let step = self.rate as f64 / out_rate as f64;
        let g = self.volume * master;
        for frame in out.chunks_mut(channels.max(1)) {
            self.fill(wait);
            let Some((l, r)) = self.q.frame() else {
                continue; // decoder behind, or the end: silence
            };
            if channels == 1 {
                frame[0] += (l + r) * 0.5 * g;
            } else {
                frame[0] += l * g;
                frame[1] += r * g;
            }
            let n = self.q.advance(step);
            if n > 0 {
                self.played.fetch_add(n, Ordering::Relaxed);
            }
        }
    }
}

fn decode_loop(path: &Path, looping: bool, tx: &SyncSender<Chunk>) {
    loop {
        match decode_file(path, tx) {
            Ok(true) if looping => continue,
            Ok(_) => return,
            Err(e) => {
                eprintln!("music {}: {e}", path.display());
                return;
            }
        }
    }
}

/// Decode one pass; Ok(true) at the end of the file, Ok(false) when the player went away.
fn decode_file(path: &Path, tx: &SyncSender<Chunk>) -> Result<bool, String> {
    let file = File::open(path).map_err(|e| e.to_string())?;
    let mss = MediaSourceStream::new(Box::new(file), Default::default());
    let mut hint = Hint::new();
    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
        hint.with_extension(ext);
    }
    let mut format = symphonia::default::get_probe()
        .probe(&hint, mss, FormatOptions::default(), MetadataOptions::default())
        .map_err(|e| e.to_string())?;
    let track = format.default_track(TrackType::Audio).ok_or("no audio track")?;
    let params = track.codec_params.as_ref().and_then(|p| p.audio()).ok_or("not audio")?;
    let mut decoder = symphonia::default::get_codecs().make_audio_decoder(params, &AudioDecoderOptions::default()).map_err(|e| e.to_string())?;
    let track_id = track.id;
    let mut samples: Vec<f32> = Vec::new();
    loop {
        let packet = match format.next_packet() {
            Ok(Some(p)) => p,
            Ok(None) => return Ok(true),
            Err(symphonia::core::errors::Error::IoError(_)) => return Ok(true),
            Err(e) => return Err(e.to_string()),
        };
        if packet.track_id != track_id {
            continue;
        }
        let buf = match decoder.decode(&packet) {
            Ok(b) => b,
            Err(symphonia::core::errors::Error::DecodeError(_)) => continue,
            Err(e) => return Err(e.to_string()),
        };
        let (rate, ch) = (buf.spec().rate(), buf.spec().channels().count().max(1));
        samples.resize(buf.samples_interleaved(), 0.0);
        buf.copy_to_slice_interleaved(&mut samples);
        let data: Vec<f32> = samples.chunks_exact(ch).flat_map(|f| [f[0], f[if ch > 1 { 1 } else { 0 }]]).collect();
        if tx.send(Chunk { rate, data }).is_err() {
            return Ok(false);
        }
    }
}

/// Length of a music file in whole seconds, from the container (no decoding); `None` when the
/// file has no frame count or cannot be probed. The front end's song-to-track table shows it.
pub fn duration_secs(path: &Path) -> Option<u32> {
    let file = File::open(path).ok()?;
    let mss = MediaSourceStream::new(Box::new(file), Default::default());
    let mut hint = Hint::new();
    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
        hint.with_extension(ext);
    }
    let format = symphonia::default::get_probe().probe(&hint, mss, FormatOptions::default(), MetadataOptions::default()).ok()?;
    let track = format.default_track(TrackType::Audio)?;
    let rate = track.codec_params.as_ref()?.audio()?.sample_rate?;
    let frames = track.num_frames?;
    (rate > 0).then(|| (frames as f64 / rate as f64).round() as u32)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pcm_feed_counts_consumed_frames() {
        let (mut m, feed) = MusicStream::pcm(1000, 1.0);
        feed.push(&[16384, -16384, 0, 0, 8192, 8192, 0, 0]); // 4 stereo frames
        let mut out = [0.0f32; 6]; // 3 frames at the stream's rate
        m.mix(&mut out, 2, 1000, 1.0, false);
        assert_eq!(feed.played(), 3);
        assert!((out[0] - 0.5).abs() < 1e-6 && (out[1] + 0.5).abs() < 1e-6);
        // underrun: only the last frame is left and it is never consumed, so nothing more counts
        let mut out = [0.0f32; 4];
        m.mix(&mut out, 2, 1000, 1.0, false);
        assert_eq!(feed.played(), 3);
        drop(feed);
        m.mix(&mut out, 2, 1000, 1.0, false);
        assert!(m.is_finished());
    }

    /// #490: an offline mix waits for the decoder thread, so the first frames are the file's, not silence, every time.
    #[test]
    fn offline_mix_waits_for_the_decoder() {
        let dir = std::env::temp_dir().join(format!("motor_music_test_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("t.wav");
        let pcm: Vec<i16> = (0..2 * 4000).map(|i| ((i % 200) as i16 - 100) * 100).collect();
        std::fs::write(&path, crate::wav::encode_pcm16(44100, 2, &pcm)).unwrap();
        let render = || {
            let mut m = MusicStream::open(&path, false, 1.0);
            let mut out = vec![0.0f32; 2 * 1000];
            m.mix(&mut out, 2, 44100, 1.0, true);
            out
        };
        let (a, b) = (render(), render());
        assert_eq!(a, b);
        assert!(a.iter().any(|&s| s != 0.0), "the decoder's first frames were played");
        std::fs::remove_dir_all(&dir).ok();
    }
}
