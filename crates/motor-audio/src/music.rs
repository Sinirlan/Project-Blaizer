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

/// Decoded audio: interleaved stereo at `rate`.
struct Chunk {
    rate: u32,
    data: Vec<f32>,
}

pub struct MusicStream {
    rx: Receiver<Chunk>,
    buf: VecDeque<[f32; 2]>,
    rate: u32,
    frac: f64,
    finished: bool,
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
        MusicStream { rx, buf: VecDeque::new(), rate: 44100, frac: 0.0, finished: false, played: Arc::default(), volume }
    }

    /// A stream fed by the caller with interleaved stereo `i16` at `rate` (no decoder thread).
    pub fn pcm(rate: u32, volume: f32) -> (MusicStream, PcmFeed) {
        let (tx, rx) = channel();
        let played = Arc::new(AtomicU64::new(0));
        let m = MusicStream { rx, buf: VecDeque::new(), rate, frac: 0.0, finished: false, played: played.clone(), volume };
        (m, PcmFeed { tx, rate, played })
    }

    pub fn is_finished(&self) -> bool {
        self.finished && self.buf.len() < 2
    }

    fn fill(&mut self) {
        while self.buf.len() < 2 && !self.finished {
            match self.rx.try_recv() {
                Ok(c) => {
                    self.rate = c.rate;
                    self.buf.extend(c.data.as_chunks::<2>().0.iter().map(|f| [f[0], f[1]]));
                }
                Err(TryRecvError::Empty) => return,
                Err(TryRecvError::Disconnected) => self.finished = true,
            }
        }
    }

    /// Add into interleaved `out` at `out_rate`.
    pub fn mix(&mut self, out: &mut [f32], channels: usize, out_rate: u32, master: f32) {
        let step = self.rate as f64 / out_rate as f64;
        let g = self.volume * master;
        for frame in out.chunks_mut(channels.max(1)) {
            self.fill();
            if self.buf.len() < 2 {
                continue; // decoder behind, or the end: silence
            }
            let (a, b) = (self.buf[0], self.buf[1]);
            let t = self.frac as f32;
            let (l, r) = (a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t);
            if channels == 1 {
                frame[0] += (l + r) * 0.5 * g;
            } else {
                frame[0] += l * g;
                frame[1] += r * g;
            }
            self.frac += step;
            while self.frac >= 1.0 && !self.buf.is_empty() {
                self.frac -= 1.0;
                self.buf.pop_front();
                self.played.fetch_add(1, Ordering::Relaxed);
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
        m.mix(&mut out, 2, 1000, 1.0);
        assert_eq!(feed.played(), 3);
        assert!((out[0] - 0.5).abs() < 1e-6 && (out[1] + 0.5).abs() < 1e-6);
        // underrun: only the last frame is left and it is never consumed, so nothing more counts
        let mut out = [0.0f32; 4];
        m.mix(&mut out, 2, 1000, 1.0);
        assert_eq!(feed.played(), 3);
        drop(feed);
        m.mix(&mut out, 2, 1000, 1.0);
        assert!(m.is_finished());
    }
}
