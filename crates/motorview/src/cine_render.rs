//! Cine (#32, re-notes/20-cine.md): the headless render of a `.cine` sequence. The all-AI race is
//! stepped at a fixed 1/fps, the `Director` picks the camera, `Gpu::capture_submit` draws the frame and it
//! goes to ffmpeg's stdin as raw RGBA (or to a PNG sequence when ffmpeg is missing or asked not to).
//! A shot's `Speed` scales the race step (slow motion), its fades darken the captured pixels (#151).
//! A shot's `Pitch tape` plays its sound at the Speed too, pitch included (#426).
//! REMASTER sound (#153): the race audio heard by the camera on air is mixed in the same loop,
//! `44100 / fps` samples a frame, faded with the picture, and muxed into the video (or written as
//! `audio.wav` beside a PNG sequence).
//! One frame is in flight (#422): frame n is drawn and submitted before frame n - 1 is read back, faded and
//! written, so the GPU draws while the CPU post-processes and encodes. The pictures are the same as when each
//! frame was read right after its draw.

use crate::cine::{self, fade_at, speed_at, Director, RaceDef, Sequence};
use crate::drive::{CamMode, Session};
use crate::inspect::Passes;
use crate::race_audio::{CineEar, Ear, RaceAudio, MASTER};
use crate::render::{Frame, Gpu, Readback, RenderMode, View};
use crate::scene;
use crate::{device, instance, png};
use motor_sim::replay::Recording;
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

    /// Video frame `n` (1/fps of video time after the last), heard by `ear` (camera `air` on air) at the picture's fade `gain`, `(gain, rate)`; kept when `keep`.
    /// `rate` = the shot's tape speed (#426, `cine::audio_rate_at`): the race sound runs that much faster on the sound clock and the mixer plays it
    /// back at that rate, so 0.5 is an octave lower and twice as long; 1 = normal pitch.
    fn frame(&mut self, sess: &Session, ear: Ear, air: usize, n: usize, (gain, rate): (f32, f32), keep: bool) {
        let (fps, dt) = (self.fps, 1.0 / self.fps as f32);
        let cut = self.heard.is_some_and(|a| a != air);
        self.heard = Some(air);
        self.race.update(sess, &Ear { cine: Some(CineEar { doppler: self.doppler, cut }), ..ear }, dt * rate);
        self.buf.resize(2 * Self::frames_of(n, fps), 0.0);
        self.mixer.with(|m| {
            m.rate = rate;
            m.render(&mut self.buf, 2)
        });
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

/// Cross-dissolve: `px` becomes `k` of itself and `1 - k` of `from` (same size RGBA), alpha untouched. CPU blend of two captures [design choice].
fn dissolve(px: &mut [u8], from: &[u8], k: f32) {
    for (p, q) in px.as_chunks_mut::<4>().0.iter_mut().zip(from.as_chunks::<4>().0) {
        for c in 0..3 {
            p[c] = (p[c] as f32 * k + q[c] as f32 * (1.0 - k) + 0.5) as u8;
        }
    }
}

/// Letterbox: `bar` black rows at the top and at the bottom of a `w` wide RGBA frame.
fn letterbox(px: &mut [u8], w: u32, h: u32, bar: u32) {
    let (row, bar) = (w as usize * 4, bar.min(h / 2) as usize);
    for r in (0..bar).chain(h as usize - bar..h as usize) {
        for p in px[r * row..(r + 1) * row].as_chunks_mut::<4>().0 {
            p[..3].fill(0);
        }
    }
}

/// The scene and the all-AI session of a sequence's race at its time 0 (the "3" of the countdown, or the green light
/// with `Start go`), stepped at 1/`fps`: the render's and the editor's (#152) start. With a `Recording` (#423) the
/// recorded race instead (`Session::from_recording`, played by `Session::play_to`), and `r` takes its track, cars and
/// laps (`RaceDef::from_header`).
pub fn race(root: &Path, r: &mut RaceDef, fps: u32, remaster: bool) -> Result<(scene::Scene, Session), String> {
    if let Some(p) = r.recording.clone() {
        let path = crate::cine::recording_path(&p);
        let rec = Recording::load(&path).map_err(|e| format!("recording {}: {e}", path.display()))?;
        r.from_header(&rec.header);
        let mut sess = Session::from_recording(root, &rec, r.start_go).map_err(|e| format!("recording {}: {e}", path.display()))?;
        let gloss: Vec<_> = sess.driver_paints().into_iter().map(|(i, c)| (i, scene::CarPaint::gloss(c))).collect();
        let s = scene::load(root, r.track, &r.cars, &gloss).map_err(|e| format!("track {}: {e}", r.track))?;
        println!("{}", s.stats);
        let secs = sess.timeline.as_ref().map_or(0.0, |t| t.span().2);
        println!("recording {}: {} {} laps, {} cars, {secs:.1} s from time 0", path.display(), rec.header.track_name, rec.header.laps, rec.header.cars.len());
        sess.set_wheels(&s);
        sess.fx.remaster = remaster;
        return Ok((s, sess));
    }
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

/// A frame that is drawn and submitted but not read back yet (#422).
struct Pending {
    /// the frame number (the sink's) and its time
    n: usize,
    t: f32,
    /// the camera on air (its name goes in the still's log line)
    air: usize,
    px: Readback,
    /// a dissolve's outgoing picture and its weight for the incoming one
    old: Option<(Readback, f32)>,
    /// the fade to black of this frame
    fade: f32,
}

/// What `emit` writes a frame to, and the progress it keeps.
struct Emit<'a> {
    o: &'a RenderOpts,
    seq: &'a Sequence,
    out: &'a Out<'a>,
    sink: Option<Sink>,
    written: usize,
    /// frames to write, None = not known
    total: Option<usize>,
    t0: Instant,
    tick: Instant,
}

impl Emit<'_> {
    /// Read the frame back, dissolve / fade / letterbox it and write it out.
    fn frame(&mut self, gpu: &Gpu, p: Pending) -> Result<(), String> {
        let (w, h) = self.o.size;
        let mut px = gpu.read(p.px);
        if let Some((old, k)) = p.old {
            let old = gpu.read(old);
            dissolve(&mut px, &old, k);
        }
        fade(&mut px, p.fade);
        if let Some(a) = self.seq.letterbox {
            letterbox(&mut px, w, h, cine::letterbox_bar(w, h, a));
        }
        match (&mut self.sink, self.out) {
            (Some(sink), _) => sink.write(p.n, self.o.size, &px)?,
            (None, Out::Still(path)) => {
                png::write(path, w, h, &px).map_err(|e| e.to_string())?;
                println!("wrote {} (t = {:.3} s, camera {})", path.display(), p.t, self.seq.cameras[p.air].name);
            }
            _ => {}
        }
        self.written += 1;
        if let Some(wt) = &self.o.watch {
            wt.frames.store(self.written, Ordering::Relaxed);
        }
        if self.tick.elapsed().as_secs_f32() >= 1.0 {
            self.tick = Instant::now();
            let rate = self.written as f32 / self.t0.elapsed().as_secs_f32();
            match self.total {
                Some(tot) => println!("frame {}/{tot}  {rate:.1} fps  eta {:.0} s", self.written, (tot - self.written) as f32 / rate),
                None => println!("frame {}  t = {:.1} s  {rate:.1} fps", self.written, p.t),
            }
        }
        Ok(())
    }
}

/// Render `seq` (see the module docs). `root` is the install.
pub fn render(root: &Path, o: &RenderOpts, seq: &Sequence, out: Out) -> Result<(), String> {
    let (w, h) = o.size;
    let (fps, dt) = (o.fps, 1.0 / o.fps as f32);
    if matches!(out, Out::Video(p) if o.codec != Codec::Png && p.extension().is_some()) && (w % 2 != 0 || h % 2 != 0) {
        return Err(format!("size {w}x{h}: video needs even sides"));
    }
    let base = o.view.mode;
    let remaster = base == RenderMode::Remaster;
    // a recording's header sets the race (#423): the sound and the director read the recorded track and cars
    let mut seq = seq.clone();
    let (s, mut sess) = race(root, &mut seq.race, fps, remaster)?;
    let seq = &seq;
    // #423: a recording ends where its frames do (video seconds)
    let rec_end = sess.replay.then(|| sess.timeline.as_ref().map_or(0.0, |t| cine::video_secs(seq, t.span().2 as f32)));

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
    // REMASTER (#427): a shot may switch to Remaster inside a Classic render, so the probes are baked up front for it too
    // (a Remaster render: the same call as before)
    gpu.update_local_light(&if remaster || !cine::uses_mode(seq, RenderMode::Remaster) { view } else { View { mode: RenderMode::Remaster, ..view } });

    let (first, last) = match out {
        Out::Still(_) => {
            let f = (o.from * fps as f32).round() as usize;
            (f, Some(f + 1))
        }
        Out::Video(_) => ((o.from * fps as f32).round() as usize, o.to.or(seq.end).map(|e| (e * fps as f32).round() as usize)),
    };
    let mut sound = (o.audio && seq.audio.on && matches!(out, Out::Video(_))).then(|| Sound::new(root, seq, &sess, fps));
    let sink = match out {
        Out::Video(p) => Some(Sink::open(p, o, sound.is_some())?),
        Out::Still(_) => None,
    };
    let mut dir = Director::new(seq);
    let mut done_at = None::<f32>;
    let total = last.map(|l| l.saturating_sub(first));
    if let (Some(w), Some(tot)) = (&o.watch, total) {
        w.total.store(tot, Ordering::Relaxed);
    }
    let now = Instant::now();
    let mut emit = Emit { o, seq, out: &out, sink, written: 0, total, t0: now, tick: now };
    // the frame drawn last, read back after the next one is submitted (#422)
    let mut pending = None::<Pending>;
    let mut n = 0usize;
    // race seconds minus video seconds (0 unless a shot runs at another Speed); the camera drawn last
    let (mut race_lag, mut drawn_air) = (0.0f32, None::<usize>);
    // #427: the graphics mode drawn last, and the one the dissolve's outgoing camera is drawn in (the mode before the cut)
    let (mut drawn_mode, mut dis_mode) = (None::<RenderMode>, base);
    // the camera the last dissolve frame came from (its view slot 3 keeps its own TAA and blur history)
    let mut dis_from = None::<usize>;
    loop {
        let t = n as f32 / fps as f32;
        // #427: the shot's graphics mode; the session's particles and wheel roll follow it as with F9
        let mode = cine::mode_at(seq, t, base);
        sess.fx.remaster = mode == RenderMode::Remaster;
        let view_m = View { mode, ..view };
        // frame n shows the race one step of (shot speed x 1/fps) after frame n - 1
        if n > 0 {
            let speed = speed_at(seq, t);
            race_lag += (speed - 1.0) * dt;
            if sess.replay {
                // #423: the recorded frames up to the frame's race time, the cars drawn at it (pose resampling)
                sess.play_to((t + race_lag) as f64);
            } else {
                sess.step(speed * dt, Input::default());
            }
        }
        if last.is_some_and(|l| n >= l) {
            break;
        }
        if o.watch.as_ref().is_some_and(|w| w.cancel.load(Ordering::Relaxed)) {
            // the frame in flight is kept with the others
            if let Some(p) = pending.take() {
                emit.frame(&gpu, p)?;
            }
            if let Some(sink) = emit.sink.take() {
                sink.finish(None)?;
            }
            return Err(CANCELLED.into());
        }
        // no `End`: until every car is over the line, then two seconds more (20 minutes at most)
        if seq.end.is_none() {
            if sess.race.all_finished() {
                done_at.get_or_insert(t);
            }
            if last.is_none() && (done_at.is_some_and(|d| t >= d + 2.0) || rec_end.is_some_and(|e| t > e) || t > 1200.0) {
                break;
            }
        }
        let cam = dir.camera(seq, &sess.race, t, dt);
        // the last shot fades out to the sequence's end (`End`, else every car finished + 2 s or the recording's end),
        // not to `--to`
        let end = seq.end.or(match (done_at.map(|d| d + 2.0), rec_end) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        });
        if let Some(snd) = &mut sound {
            // every frame from 0, so the voices before `--from` have played out as they would have
            let ear = Ear { cam, focus: dir.focus(), in_car: dir.hidden().is_some(), cine: None };
            snd.frame(&sess, ear, dir.air(), n, (fade_at(seq, t, end), cine::audio_rate_at(seq, t)), n >= first);
        }
        if n >= first {
            sess.focus = dir.focus();
            // an in-car camera sits inside its car: the session leaves that body out (and the lights stay)
            sess.cam = if dir.hidden().is_some() { CamMode::InCar } else { CamMode::Chase };
            // a new camera on air is a cut: no blur or TAA history across it
            let air = dir.air();
            // (a switch of the graphics mode is one too, #427: the Remaster history is not the Classic picture)
            let mode_cut = drawn_mode.is_some_and(|m| m != mode);
            if drawn_air.is_some_and(|a| a != air) || mode_cut {
                gpu.cut();
                // the cut log (the auto director's cuts are checked from it, #154)
                if matches!(out, Out::Video(_)) {
                    if mode_cut {
                        println!("cut {t:.3} s -> {} ({})", seq.cameras[air].name, mode.name().to_ascii_lowercase());
                    } else {
                        println!("cut {t:.3} s -> {}", seq.cameras[air].name);
                    }
                }
            }
            drawn_air = Some(air);
            gpu.pin_clock(Some(t + race_lag));
            let parts = o.passes.parts(Some(&sess), &cam, &view_m);
            // the camera's lens (#421; Remaster only, the renderer ignores it in Classic)
            let f = Frame { dof: dir.dof(air), ..parts.frame() };
            let px = gpu.capture_submit(w, h, &cam, &view_m, &f);
            let mut old = None;
            sess.cam = CamMode::Chase;
            // a dissolve (#154): the camera cut away from is drawn too, in the other history slot, and blended under this one.
            // The volumetric fog keeps one history, so it restarts at each switch [APPROX]; the sound is the incoming camera's.
            match dir.dissolve().and_then(|(from, k)| dir.view(from).map(|v| (from, k, v))) {
                Some((from, k, (fcam, slot, hide))) => {
                    gpu.select_view(3);
                    if dis_from != Some(from) {
                        gpu.cut();
                        // #427: the picture cut away from was drawn in the mode of the frame before this one
                        dis_mode = drawn_mode.unwrap_or(mode);
                    }
                    let view_o = View { mode: dis_mode, ..view };
                    // the sim particles are the incoming mode's [APPROX]; the outgoing camera's meshes and passes are its own mode's
                    (sess.focus, sess.cam) = (slot, if hide { CamMode::InCar } else { CamMode::Chase });
                    let parts = o.passes.parts(Some(&sess), &fcam, &view_o);
                    let f = Frame { dof: dir.dof(from), ..parts.frame() };
                    old = Some((gpu.capture_submit(w, h, &fcam, &view_o, &f), k));
                    gpu.select_view(2);
                    (sess.focus, sess.cam) = (dir.focus(), CamMode::Chase);
                    dis_from = Some(from);
                }
                None => dis_from = None,
            }
            // this frame is in flight: the one before it is read back and written while the GPU draws it
            drawn_mode = Some(mode);
            let cur = Pending { n, t, air, px, old, fade: fade_at(seq, t, end) };
            if let Some(prev) = pending.replace(cur) {
                emit.frame(&gpu, prev)?;
            }
        }
        n += 1;
    }
    if let Some(p) = pending.take() {
        emit.frame(&gpu, p)?;
    }
    let written = emit.written;
    if let Some(sink) = emit.sink.take() {
        sink.finish(sound.as_ref().map(|s| &s.pcm[..]))?;
    }
    if let Out::Video(p) = out {
        println!("rendered {written} frames ({:.1} s) at {:.1} fps -> {}", written as f32 / fps as f32, written as f32 / emit.t0.elapsed().as_secs_f32().max(1e-3), p.display());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dissolve_blends_rgb_and_keeps_alpha() {
        let mut a = [200u8, 100, 0, 255, 0, 0, 0, 255];
        let b = [0u8, 100, 200, 7, 100, 100, 100, 7];
        dissolve(&mut a, &b, 1.0);
        assert_eq!(a, [200, 100, 0, 255, 0, 0, 0, 255], "the incoming picture alone at 1");
        dissolve(&mut a, &b, 0.0);
        assert_eq!(a, [0, 100, 200, 255, 100, 100, 100, 255], "the outgoing one at 0, alpha untouched");
        let mut c = [200u8, 0, 0, 255];
        dissolve(&mut c, &[0, 0, 100, 255], 0.5);
        assert_eq!(c, [100, 0, 50, 255]);
    }

    #[test]
    fn letterbox_blacks_the_bars_only() {
        let mut px = vec![9u8; 4 * 2 * 6];
        letterbox(&mut px, 2, 6, 1);
        let row = |r: usize| px[r * 8..(r + 1) * 8].to_vec();
        assert_eq!(row(0), [0, 0, 0, 9, 0, 0, 0, 9]);
        assert_eq!(row(5), [0, 0, 0, 9, 0, 0, 0, 9]);
        assert!((1..5).all(|r| row(r) == [9; 8]));
        // bars over half the height never overrun
        let mut px = vec![9u8; 4 * 2 * 2];
        letterbox(&mut px, 2, 2, 5);
        assert!(px.chunks(4).all(|p| p[..3] == [0, 0, 0]));
    }
}
