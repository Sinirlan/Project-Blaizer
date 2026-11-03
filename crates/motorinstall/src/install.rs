//! The install job: what the original InstallShield setup put on the disk, from the CD.
//! [O] on the user's disc and install (re-notes/01-inventory.md "Install media"): every valid
//! file of MOTOR/DISK1/DATA1.CAB under its cab directory, plus movies/movies.pak copied from
//! the disc. The CD audio tracks become Music/NN Name.flac, the rip motorhead streams.

use std::io::{self, Read, Write};
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use flacenc::component::BitRepr;
use flacenc::error::{SourceError, Verify};
use flacenc::source::{Fill, Source as FlacSource};

use crate::disc::{AudioTrack, Source};
use crate::iscab::Cab;

const CAB: &str = "MOTOR/DISK1/DATA1.CAB";
const MOVIES: &str = "movies/movies.pak";
/// [O] motor.exe refuses any other size (01-inventory, the CD check).
const MOVIES_SIZE: u64 = 0x6f6_fc38;
/// CD track -> name, as in the rip motorhead was tested with (07-audio: 2 results, 3 menu,
/// then each race track's CDTrack).
const MUSIC: [(u32, &str); 10] = [
    (2, "Results"),
    (3, "Menu"),
    (4, "RedRock"),
    (5, "NolbyHills"),
    (6, "NeoCity"),
    (7, "Atlantika"),
    (8, "RuhrStadt"),
    (9, "Olympos"),
    (10, "OkkunSpeedway"),
    (11, "Goldbridge"),
];

pub struct Options {
    pub music: bool,
}

#[derive(Default)]
pub struct Summary {
    pub files: usize,
    pub bytes: u64,
    pub tracks: usize,
    /// things the player should know (no audio on the image, odd movie size)
    pub notes: Vec<String>,
}

/// Progress: bytes done, bytes in total, what is being written.
pub type Report<'a> = dyn FnMut(u64, u64, &str) + 'a;

pub fn music_name(track: u32) -> String {
    let name = MUSIC.iter().find(|m| m.0 == track).map_or("Track", |m| m.1);
    format!("{track:02} {name}.flac")
}

/// Turn a disc path ('\\' separated) into a path under `dest`, refusing anything that could
/// leave it (the image is untrusted input).
fn target(dest: &Path, rel: &str) -> Result<PathBuf, String> {
    let rel = Path::new(&rel.replace('\\', "/")).to_path_buf();
    if rel.components().any(|c| !matches!(c, Component::Normal(_))) {
        return Err(format!("refusing unsafe path {}", rel.display()));
    }
    Ok(dest.join(rel))
}

fn write_file(path: &Path, data: &[u8]) -> Result<(), String> {
    if let Some(p) = path.parent() {
        std::fs::create_dir_all(p).map_err(|e| format!("{}: {e}", p.display()))?;
    }
    std::fs::write(path, data).map_err(|e| format!("{}: {e}", path.display()))
}

fn cancelled(cancel: &AtomicBool) -> Result<(), String> {
    if cancel.load(Ordering::Relaxed) {
        return Err("cancelled".into());
    }
    Ok(())
}

/// Is this the Motorhead CD?
pub fn check(src: &Source) -> Result<(), String> {
    if !src.medium.exists(CAB) {
        return Err(format!("{CAB} not found: this is not the Motorhead CD"));
    }
    Ok(())
}

pub fn install(src: &Source, dest: &Path, opts: &Options, cancel: &AtomicBool, report: &mut Report) -> Result<Summary, String> {
    check(src)?;
    let (mut r, len) = src.medium.open(CAB)?;
    let mut cab_data = Vec::with_capacity(len as usize);
    r.read_to_end(&mut cab_data).map_err(|e| format!("{CAB}: {e}"))?;
    let cab = Cab::parse(&cab_data).map_err(|e| format!("{CAB}: {e}"))?;

    let audio: Vec<&AudioTrack> = if opts.music { src.audio.iter().collect() } else { Vec::new() };
    let movies = src.medium.open(MOVIES).ok();
    let total = cab.files.iter().filter(|f| f.valid()).map(|f| f.size).sum::<u64>()
        + movies.as_ref().map_or(0, |m| m.1)
        + audio.iter().map(|t| t.len).sum::<u64>();
    let mut sum = Summary::default();
    let mut done = 0u64;

    // directories first: some exist only for the game to write into (HiScores, AddressBook)
    for d in &cab.dirs {
        let p = target(dest, d)?;
        std::fs::create_dir_all(&p).map_err(|e| format!("{}: {e}", p.display()))?;
    }
    for f in cab.files.iter().filter(|f| f.valid()) {
        cancelled(cancel)?;
        let rel = cab.path(f);
        report(done, total, &rel);
        let data = cab.extract(f)?;
        write_file(&target(dest, &rel)?, &data)?;
        done += f.size;
        sum.files += 1;
        sum.bytes += f.size;
    }

    match movies {
        Some((mut r, len)) => {
            if len != MOVIES_SIZE {
                sum.notes.push(format!("{MOVIES} is {len} bytes, the game expects {MOVIES_SIZE}"));
            }
            let path = target(dest, MOVIES)?;
            write_file(&path, &[])?;
            let mut out = std::fs::File::create(&path).map_err(|e| format!("{}: {e}", path.display()))?;
            let mut buf = vec![0u8; 1 << 20];
            loop {
                cancelled(cancel)?;
                report(done, total, MOVIES);
                let n = r.read(&mut buf).map_err(|e| format!("{MOVIES}: {e}"))?;
                if n == 0 {
                    break;
                }
                out.write_all(&buf[..n]).map_err(|e| format!("{}: {e}", path.display()))?;
                done += n as u64;
                sum.bytes += n as u64;
            }
            sum.files += 1;
        }
        None => sum.notes.push(format!("{MOVIES} is not on the disc: the game runs without its movies")),
    }

    if opts.music && src.audio.is_empty() {
        sum.notes.push("no CD audio tracks in this source (the CD music needs a .cue/.bin image): no Music/ written".into());
    }
    for t in audio {
        cancelled(cancel)?;
        let name = music_name(t.number);
        let path = target(dest, &format!("Music/{name}"))?;
        let start = done;
        let flac = encode(t, cancel, &mut |n| report(start + n, total, &format!("Music/{name}")))?;
        write_file(&path, &flac)?;
        done = start + t.len;
        sum.bytes += flac.len() as u64;
        sum.tracks += 1;
    }
    report(total, total, "");
    Ok(sum)
}

/// A CD audio track as a FLAC source: 16-bit little-endian stereo PCM, read in blocks.
struct CdAudio<'a> {
    pcm: io::BufReader<io::Take<std::fs::File>>,
    frames: usize,
    read: u64,
    buf: Vec<u8>,
    samples: Vec<i32>,
    cancel: &'a AtomicBool,
    progress: &'a mut dyn FnMut(u64),
}

impl FlacSource for CdAudio<'_> {
    fn channels(&self) -> usize {
        2
    }
    fn bits_per_sample(&self) -> usize {
        16
    }
    fn sample_rate(&self) -> usize {
        44_100
    }
    fn read_samples<F: Fill>(&mut self, block_size: usize, dest: &mut F) -> Result<usize, SourceError> {
        if self.cancel.load(Ordering::Relaxed) {
            return Err(SourceError::from_io_error(io::Error::new(io::ErrorKind::Interrupted, "cancelled")));
        }
        self.buf.resize(block_size * 4, 0);
        let mut n = 0;
        while n < self.buf.len() {
            match self.pcm.read(&mut self.buf[n..]).map_err(SourceError::from_io_error)? {
                0 => break,
                k => n += k,
            }
        }
        let n = n / 4 * 4;
        self.samples.clear();
        self.samples.extend(self.buf[..n].as_chunks::<2>().0.iter().map(|&s| i16::from_le_bytes(s) as i32));
        dest.fill_interleaved(&self.samples)?;
        self.read += n as u64;
        (self.progress)(self.read);
        Ok(n / 4)
    }
    fn len_hint(&self) -> Option<usize> {
        Some(self.frames)
    }
}

fn encode(t: &AudioTrack, cancel: &AtomicBool, progress: &mut dyn FnMut(u64)) -> Result<Vec<u8>, String> {
    let err = |e: io::Error| format!("{}: {e}", t.file.display());
    let mut f = std::fs::File::open(&t.file).map_err(err)?;
    io::Seek::seek(&mut f, io::SeekFrom::Start(t.start)).map_err(err)?;
    let len = t.len / 4 * 4;
    let src = CdAudio {
        pcm: io::BufReader::with_capacity(1 << 20, f.take(len)),
        frames: (len / 4) as usize,
        read: 0,
        buf: Vec::new(),
        samples: Vec::new(),
        cancel,
        progress,
    };
    let config = flacenc::config::Encoder::default().into_verified().map_err(|e| format!("FLAC config: {:?}", e.1))?;
    let mut stream = flacenc::encode_with_fixed_block_size(&config, src, config.block_size).map_err(|e| {
        if cancel.load(Ordering::Relaxed) {
            "cancelled".to_string()
        } else {
            format!("track {}: FLAC encoding failed: {e}", t.number)
        }
    })?;
    // flacenc lowers the minimum block size to the short last frame; the spec leaves the last
    // block out, and libFLAC then maps frame numbers to wrong sample positions (seeking).
    let bs = config.block_size;
    stream.stream_info_mut().set_block_sizes(bs, bs).map_err(|e| format!("FLAC block size: {e:?}"))?;
    let mut sink = flacenc::bitsink::ByteSink::new();
    stream.write(&mut sink).map_err(|e| format!("track {}: FLAC write failed: {e:?}", t.number))?;
    Ok(sink.as_slice().to_vec())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_and_paths() {
        assert_eq!(music_name(4), "04 RedRock.flac");
        assert_eq!(music_name(12), "12 Track.flac");
        assert_eq!(target(Path::new("/i"), "Cars\\car01\\data\\x.cfg").unwrap(), Path::new("/i/Cars/car01/data/x.cfg"));
        assert!(target(Path::new("/i"), "..\\evil").is_err());
        assert!(target(Path::new("/i"), "/etc/passwd").is_err());
    }
}
