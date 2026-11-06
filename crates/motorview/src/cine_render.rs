//! Cine (#32, re-notes/20-cine.md): the headless render of a `.cine` sequence. The all-AI race is
//! stepped at a fixed 1/fps, the `Director` picks the camera, `Gpu::capture` draws the frame and it
//! goes to ffmpeg's stdin as raw RGBA (or to a PNG sequence when ffmpeg is missing or asked not to).
//! A shot's `Speed` scales the race step (slow motion), its fades darken the captured pixels (#151).
//! REMASTER sound (#153): the race audio heard by the camera on air is mixed in the same loop,
//! `44100 / fps` samples a frame, faded with the picture, and muxed into the video (or written as
//! `audio.wav` beside a PNG sequence).

use crate::cine::{fade_at, on_air, speed_at, Director, RaceDef, Sequence};
use crate::drive::{CamMode, Session};
use crate::inspect::Passes;
use crate::race_audio::{CineEar, Ear, RaceAudio, MASTER};
use crate::render::{Gpu, RenderMode, View};
use crate::scene;
use crate::{device, instance, png};
use motor_sim::Input;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Instant;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Codec {
    H264,
    Hevc,
    Nvenc,
    Png,
}

impl Codec {
    pub fn parse(s: &str) -> Option<Codec> {
        Some(match s {
            "h264" => Codec::H264,
            "hevc" => Codec::Hevc,
            "nvenc" => Codec::Nvenc,
            "png" => Codec::Png,
            _ => return None,
        })
    }

    fn args(self) -> &'static [&'static str] {
        match self {
            Codec::H264 | Codec::Png => &["-c:v", "libx264", "-preset", "medium", "-crf", "18"],
            Codec::Hevc => &["-c:v", "libx265", "-crf", "20", "-tag:v", "hvc1"],
            Codec::Nvenc => &["-c:v", "h264_nvenc", "-cq", "19"],
        }
    }
}

pub struct RenderOpts {
    /// the graphics options (Classic or Remaster), without the racing line
    pub view: View,
    pub passes: Passes,
    pub size: (u32, u32),
    pub fps: u32,
    /// seconds; frames before `from` are simulated, not drawn
    pub from: f32,
    /// seconds; None = the sequence's `End`, else every car finished + 2 s
    pub to: Option<f32>,
    pub codec: Codec,
    /// a render on a worker thread (the editor's Render button): progress out, cancel in
    pub watch: Option<Watch>,
    /// mix the race's sound (when the sequence's `Audio` is on too); `--no-audio` clears it
    pub audio: bool,
}

/// The sample rate of the rendered sound (the mixer's).
const AUDIO_RATE: u32 = 44100;

/// A render's progress, read by another thread, and its cancel flag.
#[derive(Clone, Default)]
pub struct Watch {
    /// frames written
    pub frames: Arc<AtomicUsize>,
    /// frames to write, 0 = not known (no `End`: until every car finished + 2 s)
    pub total: Arc<AtomicUsize>,
    /// set to stop: the frames so far are kept (ffmpeg closes the video)
    pub cancel: Arc<AtomicBool>,
}

/// The error of a cancelled render.
pub const CANCELLED: &str = "cancelled";

pub enum Out<'a> {
    /// an mp4 (or a PNG sequence next to it)
    Video(&'a Path),
    /// one PNG: the frame at `RenderOpts::from`
    Still(&'a Path),
}

/// Where the frames of `out` go as a PNG sequence: `out` itself without an extension, else `<stem>_frames`.
fn frames_dir(out: &Path) -> PathBuf {
    if out.extension().is_some() {
        out.with_file_name(format!("{}_frames", out.file_stem().map_or("cine".into(), |s| s.to_string_lossy().into_owned())))
    } else {
        out.to_path_buf()
    }
}

enum Sink {
    /// ffmpeg writing the video to `tmp` (when sound is muxed in at the end) or straight to `out`
    Ffmpeg { child: Child, tmp: PathBuf, out: PathBuf },
    Png(PathBuf),
}

/// Where the silent video goes before the mux: `<stem>.noaudio.<ext>` beside `out`.
fn silent_path(out: &Path) -> PathBuf {
    let stem = out.file_stem().map_or("cine".into(), |s| s.to_string_lossy().into_owned());
    let ext = out.extension().map_or("mp4".into(), |s| s.to_string_lossy().into_owned());
    out.with_file_name(format!("{stem}.noaudio.{ext}"))
}

impl Sink {
    /// `audio`: the sound is muxed in at `finish`, so the video goes to a temporary file first.
    fn open(out: &Path, o: &RenderOpts, audio: bool) -> Result<Sink, String> {
        let png = |why: &str| -> Result<Sink, String> {
            let dir = frames_dir(out);
            std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
            eprintln!("cine: {why}: writing a PNG sequence to {}", dir.display());
            Ok(Sink::Png(dir))
        };
        if o.codec == Codec::Png || out.extension().is_none() {
            return png("PNG output");
        }
        if let Some(d) = out.parent().filter(|d| !d.as_os_str().is_empty()) {
            std::fs::create_dir_all(d).map_err(|e| format!("{}: {e}", d.display()))?;
        }
        let (w, h) = o.size;
        let tmp = if audio { silent_path(out) } else { out.to_path_buf() };
        let mut cmd = Command::new("ffmpeg");
        cmd.args(["-y", "-loglevel", "error", "-f", "rawvideo", "-pix_fmt", "rgba", "-s", &format!("{w}x{h}"), "-r", &o.fps.to_string(), "-i", "-"])
            .args(o.codec.args())
            .args(["-pix_fmt", "yuv420p", "-movflags", "+faststart"])
            .arg(&tmp)
            .stdin(Stdio::piped());
        match cmd.spawn() {
            Ok(child) => Ok(Sink::Ffmpeg { child, tmp, out: out.to_path_buf() }),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => png("ffmpeg is not on PATH"),
            Err(e) => Err(format!("ffmpeg: {e}")),
        }
    }

    fn write(&mut self, n: usize, size: (u32, u32), px: &[u8]) -> Result<(), String> {
        match self {
            Sink::Ffmpeg { child: c, .. } => c.stdin.as_mut().ok_or("ffmpeg stdin closed")?.write_all(px).map_err(|e| format!("ffmpeg stopped: {e}")),
            Sink::Png(dir) => png::write(&dir.join(format!("{n:06}.png")), size.0, size.1, px).map_err(|e| e.to_string()),
        }
    }

    /// Close the video and add the sound: `pcm` (16-bit stereo at `AUDIO_RATE`) is muxed into the
    /// video, or written as `audio.wav` beside a PNG sequence; None leaves the video silent.
    fn finish(self, pcm: Option<&[i16]>) -> Result<(), String> {
        match self {
            Sink::Ffmpeg { child: mut c, tmp, out } => {
                drop(c.stdin.take());
                let st = c.wait().map_err(|e| e.to_string())?;
                if !st.success() {
                    return Err(format!("ffmpeg failed ({st})"));
                }
                if tmp == out {
                    return Ok(());
                }
                let Some(pcm) = pcm else {
                    return std::fs::rename(&tmp, &out).map_err(|e| format!("{}: {e}", out.display()));
                };
                let wav = tmp.with_extension("wav");
                std::fs::write(&wav, motor_audio::wav::encode_pcm16(AUDIO_RATE, 2, pcm)).map_err(|e| format!("{}: {e}", wav.display()))?;
                let st = Command::new("ffmpeg")
                    .args(["-y", "-loglevel", "error", "-i"])
                    .arg(&tmp)
                    .arg("-i")
                    .arg(&wav)
                    .args(["-map", "0:v", "-map", "1:a", "-c:v", "copy", "-c:a", "aac", "-b:a", "192k", "-shortest", "-movflags", "+faststart"])
                    .arg(&out)
                    .status()
                    .map_err(|e| format!("ffmpeg: {e}"))?;
                let _ = std::fs::remove_file(&wav);
                if !st.success() {
                    return Err(format!("ffmpeg mux failed ({st}); the silent video is {}", tmp.display()));
                }
                let _ = std::fs::remove_file(&tmp);
                Ok(())
            }
            Sink::Png(dir) => match pcm {
                Some(pcm) => std::fs::write(dir.join("audio.wav"), motor_audio::wav::encode_pcm16(AUDIO_RATE, 2, pcm)).map_err(|e| e.to_string()),
                None => Ok(()),
            },
        }
    }
}

/// REMASTER (#153): the race heard by the camera on air, mixed offline frame by frame.
struct Sound {
    mixer: motor_audio::Audio,
    race: RaceAudio,
    doppler: bool,
    /// 16-bit stereo of the drawn frames
    pcm: Vec<i16>,
    buf: Vec<f32>,
    /// the camera heard last (a change restarts the Doppler history)
    heard: Option<usize>,
    fps: u32,
}

impl Sound {
    fn new(root: &Path, seq: &Sequence, sess: &Session, fps: u32) -> Sound {
        let mixer = motor_audio::Audio::silent();
        mixer.with(|m| m.master = MASTER.get());
        let mut race = RaceAudio::new(root, mixer.share());
        race.start(seq.race.track, &sess.race);
        Sound { mixer, race, doppler: seq.audio.doppler, pcm: Vec::new(), buf: Vec::new(), heard: None, fps }
    }

    /// The mixer frames of video frame `n`: the samples up to frame n + 1 minus those up to n,
    /// rounded, so the sound never drifts from the picture at any fps.
    fn frames_of(n: usize, fps: u32) -> usize {
        let at = |n: usize| (n as u64 * AUDIO_RATE as u64 + fps as u64 / 2) / fps as u64;
        (at(n + 1) - at(n)) as usize
    }

    /// Video frame `n` (1/fps of video time after the last), heard by `ear` (camera `air` on air) at the picture's fade `gain`; kept when `keep`.
    fn frame(&mut self, sess: &Session, ear: Ear, air: usize, n: usize, gain: f32, keep: bool) {
        let (fps, dt) = (self.fps, 1.0 / self.fps as f32);
        let cut = self.heard.is_some_and(|a| a != air);
        self.heard = Some(air);
        self.race.update(sess, &Ear { cine: Some(CineEar { doppler: self.doppler, cut }), ..ear }, dt);
        self.buf.resize(2 * Self::frames_of(n, fps), 0.0);
        self.mixer.with(|m| m.render(&mut self.buf, 2));
        if keep {
            self.pcm.extend(self.buf.iter().map(|&s| motor_audio::wav::to_pcm16(s * gain)));
        }
    }
}

/// Darken RGBA pixels to `k` (0 = black) for a fade; alpha untouched.
fn fade(px: &mut [u8], k: f32) {
    if k >= 1.0 {
        return;
    }
    for p in px.as_chunks_mut::<4>().0 {
        for c in &mut p[..3] {
            *c = (*c as f32 * k + 0.5) as u8;
        }
    }
}

/// The scene and the all-AI session of a sequence's race at its time 0 (the "3" of the countdown, or the green light
/// with `Start go`), stepped at 1/`fps`: the render's and the editor's (#152) start.
pub fn race(root: &Path, r: &RaceDef, fps: u32, remaster: bool) -> Result<(scene::Scene, Session), String> {
    let s = scene::load(root, r.track, &r.cars, &[]).map_err(|e| format!("track {}: {e}", r.track))?;
    println!("{}", s.stats);
    let mut sess = Session::with_options(root, r.track, &r.cars, None, r.difficulty, r.laps, r.catchup)?;
    sess.set_wheels(&s);
    // the director owns the cameras: no start fly-by (time 0 is the "3" of the countdown), no session camera
    sess.skip_intro();
    sess.fx.remaster = remaster;
    if r.start_go {
        let dt = 1.0 / fps as f32;
        let mut guard = 0;
        while !sess.race.started() && guard < 60 * fps {
            sess.step(dt, Input::default());
            guard += 1;
        }
    }
    Ok((s, sess))
}

/// Render `seq` (see the module docs). `root` is the install.
pub fn render(root: &Path, o: &RenderOpts, seq: &Sequence, out: Out) -> Result<(), String> {
    let (w, h) = o.size;
    let (fps, dt) = (o.fps, 1.0 / o.fps as f32);
    if matches!(out, Out::Video(p) if o.codec != Codec::Png && p.extension().is_some()) && (w % 2 != 0 || h % 2 != 0) {
        return Err(format!("size {w}x{h}: video needs even sides"));
    }
    let remaster = o.view.mode == RenderMode::Remaster;
    let (s, mut sess) = race(root, &seq.race, fps, remaster)?;

    let inst = instance();
    let adapter = pollster::block_on(inst.request_adapter(&Default::default())).map_err(|e| format!("no GPU adapter: {e}"))?;
    let (dev, queue) = pollster::block_on(device(&adapter));
    let mut gpu = Gpu::new(&adapter, dev, queue, wgpu::TextureFormat::Rgba8Unorm);
    // REMASTER local light: the whole probe bake before the first frame, as in `--shot` (#83)
    gpu.wait_local_light(true);
    gpu.load(&s);
    gpu.set_local_source(Some(crate::probes::BakeSource::new(&s, &sess.race.world)));
    // its own targets and TAA / blur history
    gpu.select_view(2);
    let view = o.passes.view(&o.view);
    gpu.update_local_light(&view);

    let (first, last) = match out {
        Out::Still(_) => {
            let f = (o.from * fps as f32).round() as usize;
            (f, Some(f + 1))
        }
        Out::Video(_) => ((o.from * fps as f32).round() as usize, o.to.or(seq.end).map(|e| (e * fps as f32).round() as usize)),
    };
    let mut sound = (o.audio && seq.audio.on && matches!(out, Out::Video(_))).then(|| Sound::new(root, seq, &sess, fps));
    let mut sink = match out {
        Out::Video(p) => Some(Sink::open(p, o, sound.is_some())?),
        Out::Still(_) => None,
    };
    let mut dir = Director::new(seq);
    let (mut written, mut done_at) = (0usize, None::<f32>);
    let (t0, mut tick) = (Instant::now(), Instant::now());
    let total = last.map(|l| l.saturating_sub(first));
    if let (Some(w), Some(tot)) = (&o.watch, total) {
        w.total.store(tot, Ordering::Relaxed);
    }
    let mut n = 0usize;
    // race seconds minus video seconds (0 unless a shot runs at another Speed); the camera drawn last
    let (mut race_lag, mut drawn_air) = (0.0f32, None::<usize>);
    loop {
        let t = n as f32 / fps as f32;
        // frame n shows the race one step of (shot speed x 1/fps) after frame n - 1
        if n > 0 {
            let speed = speed_at(seq, t);
            sess.step(speed * dt, Input::default());
            race_lag += (speed - 1.0) * dt;
        }
        if last.is_some_and(|l| n >= l) {
            break;
        }
        if o.watch.as_ref().is_some_and(|w| w.cancel.load(Ordering::Relaxed)) {
            if let Some(sink) = sink {
                sink.finish(None)?;
            }
            return Err(CANCELLED.into());
        }
        // no `End`: until every car is over the line, then two seconds more (20 minutes at most)
        if seq.end.is_none() {
            if sess.race.all_finished() {
                done_at.get_or_insert(t);
            }
            if last.is_none() && (done_at.is_some_and(|d| t >= d + 2.0) || t > 1200.0) {
                break;
            }
        }
        let cam = dir.camera(seq, &sess.race, t, dt);
        // the last shot fades out to the sequence's end (`End`, else every car finished + 2 s), not to `--to`
        let end = seq.end.or(done_at.map(|d| d + 2.0));
        if let Some(snd) = &mut sound {
            // every frame from 0, so the voices before `--from` have played out as they would have
            let ear = Ear { cam, focus: dir.focus(), in_car: dir.hidden().is_some(), cine: None };
            snd.frame(&sess, ear, on_air(seq, t), n, fade_at(seq, t, end), n >= first);
        }
        if n >= first {
            sess.focus = dir.focus();
            // an in-car camera sits inside its car: the session leaves that body out (and the lights stay)
            sess.cam = if dir.hidden().is_some() { CamMode::InCar } else { CamMode::Chase };
            // a new camera on air is a cut: no blur or TAA history across it
            let air = on_air(seq, t);
            if drawn_air.is_some_and(|a| a != air) {
                gpu.cut();
            }
            drawn_air = Some(air);
            gpu.pin_clock(Some(t + race_lag));
            let parts = o.passes.parts(Some(&sess), &cam, &view);
            let f = parts.frame();
            let mut px = gpu.capture(w, h, &cam, &view, &f);
            sess.cam = CamMode::Chase;
            fade(&mut px, fade_at(seq, t, end));
            match (&mut sink, &out) {
                (Some(sink), _) => sink.write(n, o.size, &px)?,
                (None, Out::Still(p)) => {
                    png::write(p, w, h, &px).map_err(|e| e.to_string())?;
                    println!("wrote {} (t = {t:.3} s, camera {})", p.display(), seq.cameras[air].name);
                }
                _ => {}
            }
            written += 1;
            if let Some(w) = &o.watch {
                w.frames.store(written, Ordering::Relaxed);
            }
            if tick.elapsed().as_secs_f32() >= 1.0 {
                tick = Instant::now();
                let rate = written as f32 / t0.elapsed().as_secs_f32();
                match total {
                    Some(tot) => println!("frame {written}/{tot}  {rate:.1} fps  eta {:.0} s", (tot - written) as f32 / rate),
                    None => println!("frame {written}  t = {t:.1} s  {rate:.1} fps"),
                }
            }
        }
        n += 1;
    }
    if let Some(sink) = sink {
        sink.finish(sound.as_ref().map(|s| &s.pcm[..]))?;
    }
    if let Out::Video(p) = out {
        println!("rendered {written} frames ({:.1} s) at {:.1} fps -> {}", written as f32 / fps as f32, written as f32 / t0.elapsed().as_secs_f32().max(1e-3), p.display());
    }
    Ok(())
}
