//! The install source: a CD image (.cue sheet, .iso, raw .bin/.img) or a folder (a mounted disc
//! or a CD copied to disk). Gives the data track's files and the CD audio tracks.
//! Layout of the Motorhead disc: re-notes/01-inventory.md "Install media".

use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use crate::iso9660::Iso;

/// Bytes of one raw CD sector (2352) and of the user data in a data sector (2048).
pub const RAW: u64 = 2352;
pub const USER: u64 = 2048;
/// CD sectors per second (MSF frames).
const FPS: u64 = 75;

/// Where the data track's 2048-byte sectors lie in a file.
#[derive(Clone, Debug, PartialEq)]
pub struct DataTrack {
    pub file: PathBuf,
    /// byte offset of sector 0 in the file
    pub base: u64,
    /// bytes per stored sector (2048, 2336 or 2352)
    pub sector: u64,
    /// bytes before the user data in a stored sector (0, 8, 16 or 24)
    pub skip: u64,
}

impl DataTrack {
    /// Read `n` user-data sectors from `lba` (short at the end of the file).
    pub fn read(&self, f: &mut File, lba: u64, n: u64) -> io::Result<Vec<u8>> {
        f.seek(SeekFrom::Start(self.base + lba * self.sector))?;
        let mut raw = Vec::with_capacity((n * self.sector) as usize);
        f.take(n * self.sector).read_to_end(&mut raw)?;
        if self.sector == USER {
            return Ok(raw);
        }
        let mut out = Vec::with_capacity((n * USER) as usize);
        for s in raw.chunks(self.sector as usize) {
            let a = (self.skip as usize).min(s.len());
            out.extend_from_slice(&s[a..(a + USER as usize).min(s.len())]);
        }
        Ok(out)
    }
}

/// A CD audio track: 16-bit little-endian stereo PCM at 44.1 kHz.
#[derive(Clone, Debug, PartialEq)]
pub struct AudioTrack {
    pub number: u32,
    pub file: PathBuf,
    pub start: u64,
    pub len: u64,
}

/// A readable file tree: a disc's data track or a folder.
pub enum Medium {
    Iso(Iso),
    Folder(PathBuf),
}

impl Medium {
    /// Open a file by its path on the disc ('/' or '\\', any case) with its size.
    pub fn open(&self, path: &str) -> Result<(Box<dyn Read>, u64), String> {
        match self {
            Medium::Iso(iso) => {
                let e = iso.find(path)?.ok_or_else(|| format!("{path} is not on the disc"))?;
                Ok((Box::new(iso.reader(&e)?), e.size))
            }
            Medium::Folder(root) => {
                let p = motor_data::fs::find_ci(root, path)
                    .filter(|p| p.is_file())
                    .ok_or_else(|| format!("{path} is not in {}", root.display()))?;
                let f = File::open(&p).map_err(|e| format!("{}: {e}", p.display()))?;
                let len = f.metadata().map_err(|e| format!("{}: {e}", p.display()))?.len();
                Ok((Box::new(f), len))
            }
        }
    }

    pub fn exists(&self, path: &str) -> bool {
        match self {
            Medium::Iso(iso) => matches!(iso.find(path), Ok(Some(_))),
            Medium::Folder(root) => motor_data::fs::find_ci(root, path).is_some(),
        }
    }
}

pub struct Source {
    pub medium: Medium,
    pub audio: Vec<AudioTrack>,
}

impl Source {
    /// Open a .cue sheet, a disc image or a folder.
    pub fn open(path: &Path) -> Result<Source, String> {
        if path.is_dir() {
            return Ok(Source { medium: Medium::Folder(path.to_path_buf()), audio: Vec::new() });
        }
        let ext = path.extension().map(|e| e.to_string_lossy().to_ascii_lowercase());
        if matches!(ext.as_deref(), Some("bin" | "img")) {
            // a .bin picked instead of its sheet: the sheet knows the audio tracks
            if let Some(cue) = sibling_cue(path) {
                return Source::open(&cue);
            }
        }
        let (data, audio) = if ext.as_deref() == Some("cue") {
            let text = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
            let dir = path.parent().unwrap_or(Path::new("."));
            let tracks = parse_cue(&String::from_utf8_lossy(&text))?;
            layout(&tracks, dir, |p| std::fs::metadata(p).map(|m| m.len()).map_err(|e| format!("{}: {e}", p.display())))?
        } else {
            (sniff(path)?, Vec::new())
        };
        Ok(Source { medium: Medium::Iso(Iso::open(data)?), audio })
    }
}

/// "x.cue" (any case) next to "x.bin".
fn sibling_cue(bin: &Path) -> Option<PathBuf> {
    let stem = bin.file_stem()?.to_string_lossy().to_string();
    let dir = bin.parent().filter(|d| !d.as_os_str().is_empty()).unwrap_or(Path::new("."));
    std::fs::read_dir(dir).ok()?.flatten().map(|e| e.path()).find(|p| {
        p.file_stem().is_some_and(|s| s.to_string_lossy() == stem) && p.extension().is_some_and(|e| e.eq_ignore_ascii_case("cue"))
    })
}

/// Recognise a bare image by its sectors: the CD sync pattern at sector 16 means raw 2352-byte
/// sectors (user data after the 16-byte header, or 24 bytes in mode 2), "CD001" at byte
/// 16·2048 + 1 means plain 2048-byte sectors.
fn sniff(path: &Path) -> Result<DataTrack, String> {
    let err = |e: io::Error| format!("{}: {e}", path.display());
    let mut f = File::open(path).map_err(err)?;
    let mut head = vec![0u8; (17 * RAW) as usize];
    let n = read_full(&mut f, &mut head).map_err(err)?;
    head.truncate(n);
    let file = path.to_path_buf();
    if let Some(skip) = raw_skip(head.get((16 * RAW) as usize..).unwrap_or(&[])) {
        return Ok(DataTrack { file, base: 0, sector: RAW, skip });
    }
    if head.get((16 * USER + 1) as usize..(16 * USER + 6) as usize) == Some(b"CD001") {
        return Ok(DataTrack { file, base: 0, sector: USER, skip: 0 });
    }
    Err(format!("{} is not a CD image (no ISO 9660 volume found)", path.display()))
}

/// User-data offset of a raw sector, from its sync pattern and mode byte.
fn raw_skip(s: &[u8]) -> Option<u64> {
    const SYNC: [u8; 12] = [0, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0];
    if s.len() < 16 || s[..12] != SYNC {
        return None;
    }
    Some(if s[15] == 2 { 24 } else { 16 })
}

fn read_full(f: &mut File, buf: &mut [u8]) -> io::Result<usize> {
    let mut n = 0;
    while n < buf.len() {
        match f.read(&mut buf[n..])? {
            0 => break,
            k => n += k,
        }
    }
    Ok(n)
}

/// One TRACK of a cue sheet.
#[derive(Clone, Debug, PartialEq)]
pub struct CueTrack {
    pub file: String,
    pub number: u32,
    pub mode: String,
    /// first index in the file (INDEX 00 if present, else INDEX 01), sectors
    pub first: u64,
    /// INDEX 01, sectors
    pub index1: u64,
}

impl CueTrack {
    fn audio(&self) -> bool {
        self.mode == "AUDIO"
    }
    /// (stored sector size, user-data offset) of the track's mode.
    fn sector(&self) -> Result<(u64, u64), String> {
        Ok(match self.mode.as_str() {
            "AUDIO" | "MODE1/2352" => (RAW, 16),
            "MODE2/2352" => (RAW, 24),
            "MODE2/2336" => (2336, 8),
            "MODE1/2048" => (USER, 0),
            m => return Err(format!("track {}: unsupported mode {m}", self.number)),
        })
    }
}

/// Parse a cue sheet's FILE / TRACK / INDEX lines. PREGAP and POSTGAP are silence that is not in
/// the file, so they do not move anything.
pub fn parse_cue(text: &str) -> Result<Vec<CueTrack>, String> {
    let mut file: Option<String> = None;
    let mut tracks: Vec<CueTrack> = Vec::new();
    let mut idx0: Option<u64> = None;
    for (no, line) in text.lines().enumerate() {
        let line = line.trim();
        let err = |m: &str| format!("cue line {}: {m}", no + 1);
        let (kw, rest) = line.split_once(char::is_whitespace).unwrap_or((line, ""));
        match kw.to_ascii_uppercase().as_str() {
            "FILE" => {
                let rest = rest.trim();
                let (name, kind) = match rest.strip_prefix('"') {
                    Some(r) => r.split_once('"').ok_or_else(|| err("unclosed quote"))?,
                    None => rest.rsplit_once(char::is_whitespace).unwrap_or((rest, "BINARY")),
                };
                let kind = kind.trim().to_ascii_uppercase();
                if kind != "BINARY" {
                    return Err(err(&format!("FILE type {kind} is not supported (only BINARY)")));
                }
                file = Some(name.to_string());
            }
            "TRACK" => {
                let mut it = rest.split_whitespace();
                let number = it.next().and_then(|n| n.parse().ok()).ok_or_else(|| err("bad track number"))?;
                let mode = it.next().ok_or_else(|| err("missing track mode"))?.to_ascii_uppercase();
                let file = file.clone().ok_or_else(|| err("TRACK before FILE"))?;
                tracks.push(CueTrack { file, number, mode, first: u64::MAX, index1: u64::MAX });
                idx0 = None;
            }
            "INDEX" => {
                let mut it = rest.split_whitespace();
                let n: u32 = it.next().and_then(|n| n.parse().ok()).ok_or_else(|| err("bad index"))?;
                let at = msf(it.next().unwrap_or("")).ok_or_else(|| err("bad index time"))?;
                let t = tracks.last_mut().ok_or_else(|| err("INDEX before TRACK"))?;
                match n {
                    0 => idx0 = Some(at),
                    1 => {
                        t.index1 = at;
                        t.first = idx0.unwrap_or(at);
                    }
                    _ => {}
                }
            }
            _ => {}
        }
    }
    if let Some(t) = tracks.iter().find(|t| t.index1 == u64::MAX) {
        return Err(format!("cue: track {} has no INDEX 01", t.number));
    }
    if tracks.is_empty() {
        return Err("cue: no tracks".into());
    }
    Ok(tracks)
}

/// "mm:ss:ff" -> sectors.
fn msf(s: &str) -> Option<u64> {
    let mut it = s.split(':').map(|p| p.parse::<u64>().ok());
    let (m, sec, f) = (it.next()??, it.next()??, it.next()??);
    (sec < 60 && f < FPS && it.next().is_none()).then_some((m * 60 + sec) * FPS + f)
}

/// Byte ranges of the cue's tracks: the first data track and every audio track. A track runs
/// from its INDEX 01 to the next track's first index in the same file, or to the file's end.
type Layout = (DataTrack, Vec<AudioTrack>);
fn layout(tracks: &[CueTrack], dir: &Path, size: impl Fn(&Path) -> Result<u64, String>) -> Result<Layout, String> {
    let mut data = None;
    let mut audio = Vec::new();
    for (i, t) in tracks.iter().enumerate() {
        let (sector, skip) = t.sector()?;
        let file = dir.join(&t.file);
        let start = t.index1 * sector;
        if !t.audio() {
            if data.is_none() {
                data = Some(DataTrack { file, base: start, sector, skip });
            }
            continue;
        }
        let end = match tracks.get(i + 1).filter(|n| n.file == t.file) {
            Some(n) => n.first * sector,
            None => size(&file)?,
        };
        audio.push(AudioTrack { number: t.number, file, start, len: end.saturating_sub(start) });
    }
    let data = data.ok_or("the cue sheet has no data track")?;
    Ok((data, audio))
}

#[cfg(test)]
mod tests {
    use super::*;

    const CUE: &str = r#"FILE "motorhead.bin" BINARY
  TRACK 01 MODE1/2352
    INDEX 01 00:00:00
  TRACK 02 AUDIO
    PREGAP 00:02:00
    INDEX 01 27:11:35
  TRACK 03 AUDIO
    INDEX 00 27:55:62
    INDEX 01 27:57:62
"#;

    #[test]
    fn msf_maths() {
        assert_eq!(msf("00:02:00"), Some(150));
        assert_eq!(msf("27:11:35"), Some((27 * 60 + 11) * 75 + 35));
        assert_eq!(msf("00:60:00"), None);
        assert_eq!(msf("1:2"), None);
    }

    #[test]
    fn single_file_cue() {
        let t = parse_cue(CUE).unwrap();
        assert_eq!(t.len(), 3);
        assert_eq!((t[1].first, t[1].index1), (122_360, 122_360)); // PREGAP is not in the file
        assert_eq!(t[2].first, msf("27:55:62").unwrap());
        let (d, a) = layout(&t, Path::new("/x"), |_| Ok(200_000 * RAW)).unwrap();
        assert_eq!(d, DataTrack { file: "/x/motorhead.bin".into(), base: 0, sector: RAW, skip: 16 });
        assert_eq!(a[0].start, 122_360 * RAW);
        assert_eq!(a[0].len, (msf("27:55:62").unwrap() - 122_360) * RAW); // ends at track 3's INDEX 00
        assert_eq!(a[1].start, msf("27:57:62").unwrap() * RAW);
        assert_eq!(a[1].start + a[1].len, 200_000 * RAW);
    }

    #[test]
    fn multi_file_cue() {
        let cue = "FILE \"Motorhead (Track 1).bin\" BINARY\n TRACK 01 MODE1/2352\n  INDEX 01 00:00:00\n\
                   FILE \"Motorhead (Track 2).bin\" BINARY\n TRACK 02 AUDIO\n  INDEX 00 00:00:00\n  INDEX 01 00:02:00\n";
        let t = parse_cue(cue).unwrap();
        let (d, a) = layout(&t, Path::new("d"), |_| Ok(1000 * RAW)).unwrap();
        assert_eq!(d.file, Path::new("d/Motorhead (Track 1).bin"));
        assert_eq!(a[0].file, Path::new("d/Motorhead (Track 2).bin"));
        assert_eq!((a[0].start, a[0].len), (150 * RAW, 850 * RAW));
    }

    #[test]
    fn cue_errors() {
        assert!(parse_cue("TRACK 01 MODE1/2352\n").is_err());
        assert!(parse_cue("FILE \"a.wav\" WAVE\n").is_err());
        assert!(parse_cue("FILE a.bin BINARY\nTRACK 01 AUDIO\n").is_err());
    }

    #[test]
    fn raw_sector_sniff() {
        let mut s = vec![0u8; RAW as usize];
        assert_eq!(raw_skip(&s), None);
        s[1..11].fill(0xff);
        s[15] = 1;
        assert_eq!(raw_skip(&s), Some(16));
        s[15] = 2;
        assert_eq!(raw_skip(&s), Some(24));
    }
}
