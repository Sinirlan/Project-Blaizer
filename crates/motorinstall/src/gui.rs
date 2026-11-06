//! The installer window (no arguments): pick the disc image and the target folder, install with
//! a progress bar. The job is install::install on a worker thread.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

use eframe::egui;

use crate::disc::Source;
use crate::install::{self, Options, Summary};

const EXTS: [&str; 4] = ["cue", "iso", "bin", "img"];
const OK: egui::Color32 = egui::Color32::from_rgb(110, 200, 120);
const BAD: egui::Color32 = egui::Color32::from_rgb(235, 100, 90);

/// Headless check (`--shot`): open the window, optionally run the install, save a PNG, quit.
pub struct Shot {
    pub path: PathBuf,
    pub image: Option<PathBuf>,
    pub dest: Option<PathBuf>,
    pub install: bool,
}

struct ShotState {
    shot: Shot,
    frames: u32,
    requested: bool,
    saved: Arc<AtomicBool>,
}

/// The installer's icon (assets/icon/tools/motorinstall); none if it cannot be decoded.
fn icon() -> egui::IconData {
    let decode = || {
        let mut r = png::Decoder::new(include_bytes!("../../../assets/icon/tools/motorinstall/motorinstall-256.png").as_slice()).read_info().ok()?;
        let mut rgba = vec![0; r.output_buffer_size()];
        let i = r.next_frame(&mut rgba).ok()?;
        Some(egui::IconData { rgba, width: i.width, height: i.height })
    };
    decode().unwrap_or_default()
}

pub fn run(shot: Option<Shot>) -> eframe::Result {
    let opts = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Motorhead installer")
            .with_app_id("motorhead-installer")
            .with_icon(icon())
            .with_inner_size([600.0, 300.0])
            .with_min_inner_size([460.0, 260.0]),
        ..Default::default()
    };
    eframe::run_native("Motorhead installer", opts, Box::new(|_| Ok(Box::new(App::new(shot)))))
}

#[derive(Default)]
struct Progress {
    done: u64,
    total: u64,
    item: String,
}

struct Job {
    cancel: Arc<AtomicBool>,
    progress: Arc<Mutex<Progress>>,
    handle: JoinHandle<Result<Summary, String>>,
    dest: PathBuf,
}

/// What the picked image holds: Ok(audio track count) or why it cannot be installed.
type Probe = Result<usize, String>;

struct App {
    image: String,
    dest: String,
    /// the target follows the image until the player edits it
    dest_auto: bool,
    music: bool,
    probe: Option<(String, Probe)>,
    job: Option<Job>,
    /// last result and the folder it went to
    result: Option<(Result<Summary, String>, PathBuf)>,
    confirm: bool,
    shot: Option<ShotState>,
}

impl App {
    fn new(shot: Option<Shot>) -> App {
        let mut app = App { image: String::new(), dest: String::new(), dest_auto: true, music: true, probe: None, job: None, result: None, confirm: false, shot: None };
        if let Some(p) = shot.as_ref().and_then(|s| s.image.clone()).or_else(find_image) {
            app.set_image(p);
        }
        if let Some(d) = shot.as_ref().and_then(|s| s.dest.as_ref()) {
            app.dest = d.display().to_string();
            app.dest_auto = false;
        }
        app.shot = shot.map(|shot| ShotState { shot, frames: 0, requested: false, saved: Arc::new(AtomicBool::new(false)) });
        app
    }

    /// `--shot`: start the install on the second frame if asked, capture once the window is
    /// settled (and the install finished), close after the PNG is written.
    fn shot_step(&mut self, ctx: &egui::Context) {
        let Some(st) = &mut self.shot else { return };
        st.frames += 1;
        ctx.request_repaint_after(Duration::from_millis(30));
        if st.saved.load(Ordering::Relaxed) {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            return;
        }
        let install = st.shot.install;
        if install && st.frames == 2 {
            self.press_install();
            return;
        }
        let st = self.shot.as_mut().unwrap();
        let waiting = install && !self.confirm && (self.job.is_some() || self.result.is_none());
        if st.requested || st.frames < 20 || waiting {
            return;
        }
        st.requested = true;
        let (path, saved, c) = (st.shot.path.clone(), st.saved.clone(), ctx.clone());
        ctx.request_screenshot(move |img| {
            match save_png(&path, &img) {
                Ok(()) => println!("wrote {}", path.display()),
                Err(e) => eprintln!("motorinstall: {}: {e}", path.display()),
            }
            saved.store(true, Ordering::Relaxed);
            c.request_repaint();
        });
    }

    fn set_image(&mut self, p: PathBuf) {
        self.image = p.display().to_string();
        if self.dest_auto {
            self.dest = default_dest(&p).display().to_string();
        }
    }

    /// Re-open the source when its path changed (cheap: the cue sheet and the volume descriptors).
    fn probe(&mut self) -> Option<&Probe> {
        if self.image.trim().is_empty() {
            return None;
        }
        if self.probe.as_ref().is_none_or(|(p, _)| *p != self.image) {
            let r = Source::open(Path::new(self.image.trim())).and_then(|s| install::check(&s).map(|()| s.audio.len()));
            self.probe = Some((self.image.clone(), r));
        }
        self.probe.as_ref().map(|p| &p.1)
    }

    /// Install, after asking when the target folder already has files.
    fn press_install(&mut self) {
        if dir_has_files(Path::new(self.dest.trim())) {
            self.confirm = true;
        } else {
            self.start();
        }
    }

    fn start(&mut self) {
        let (image, dest) = (PathBuf::from(self.image.trim()), PathBuf::from(self.dest.trim()));
        let cancel = Arc::new(AtomicBool::new(false));
        let progress = Arc::new(Mutex::new(Progress::default()));
        let opts = Options { music: self.music };
        let (c, p, d) = (cancel.clone(), progress.clone(), dest.clone());
        let handle = std::thread::spawn(move || {
            let src = Source::open(&image)?;
            install::install(&src, &d, &opts, &c, &mut |done, total, item| {
                if let Ok(mut p) = p.lock() {
                    (p.done, p.total) = (done, total);
                    if p.item != item {
                        p.item = item.to_string();
                    }
                }
            })
        });
        self.result = None;
        self.job = Some(Job { cancel, progress, handle, dest });
    }

    fn rows(&mut self, ui: &mut egui::Ui) {
        let busy = self.job.is_some();
        // label | text field filling the row | buttons on the right
        let row = |ui: &mut egui::Ui, label: &str, add: &mut dyn FnMut(&mut egui::Ui)| {
            ui.horizontal(|ui| {
                ui.add_sized([72.0, 20.0], egui::Label::new(label));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| add(ui));
            });
        };
        row(ui, "Disc image", &mut |ui| {
            if ui.add_enabled(!busy, egui::Button::new("Folder…")).on_hover_text("a mounted disc or a CD copied to disk").clicked() {
                if let Some(p) = rfd::FileDialog::new().set_title("Motorhead CD folder").pick_folder() {
                    self.set_image(p);
                }
            }
            if ui.add_enabled(!busy, egui::Button::new("Browse…")).clicked() {
                let start = Path::new(self.image.trim()).parent().map(Path::to_path_buf);
                let mut d = rfd::FileDialog::new().set_title("Motorhead CD image").add_filter("CD image (.cue .iso .bin .img)", &EXTS);
                if let Some(s) = start.filter(|s| s.is_dir()) {
                    d = d.set_directory(s);
                }
                if let Some(p) = d.pick_file() {
                    self.set_image(p);
                }
            }
            let edit = egui::TextEdit::singleline(&mut self.image).desired_width(f32::INFINITY).hint_text(".cue, .iso, .bin or a disc folder");
            ui.add_enabled(!busy, edit);
        });
        row(ui, "Install to", &mut |ui| {
            if ui.add_enabled(!busy, egui::Button::new("Browse…")).clicked() {
                if let Some(p) = rfd::FileDialog::new().set_title("Install Motorhead to").pick_folder() {
                    self.dest = p.display().to_string();
                    self.dest_auto = false;
                }
            }
            if ui.add_enabled(!busy, egui::TextEdit::singleline(&mut self.dest).desired_width(f32::INFINITY)).changed() {
                self.dest_auto = false;
            }
        });
    }

    fn status(&mut self, ui: &mut egui::Ui) -> bool {
        let probe = self.probe().cloned();
        let audio = matches!(probe, Some(Ok(n)) if n > 0);
        match &probe {
            Some(Ok(0)) => ui.label("Motorhead CD found. No CD audio in this image (the music needs the .cue/.bin)."),
            Some(Ok(n)) => ui.label(format!("Motorhead CD found, {n} CD audio tracks.")),
            Some(Err(e)) => ui.colored_label(BAD, e),
            None => ui.label("Pick your Motorhead disc image."),
        };
        let busy = self.job.is_some();
        ui.add_enabled_ui(!busy && audio, |ui| {
            let mut on = self.music && audio;
            if ui.checkbox(&mut on, "CD music (lossless FLAC, about 300 MB)").changed() {
                self.music = on;
            }
        });
        matches!(probe, Some(Ok(_))) && !self.dest.trim().is_empty()
    }

    fn progress(&mut self, ui: &mut egui::Ui) {
        if let Some(job) = &self.job {
            let p = job.progress.lock().map(|p| (p.done, p.total, p.item.clone())).unwrap_or_default();
            let frac = if p.1 > 0 { p.0 as f32 / p.1 as f32 } else { 0.0 };
            ui.add(egui::ProgressBar::new(frac).show_percentage());
            ui.label(egui::RichText::new(p.2.replace('\\', "/")).weak());
            ui.ctx().request_repaint_after(Duration::from_millis(50));
            if job.handle.is_finished() {
                let job = self.job.take().unwrap();
                let r = job.handle.join().unwrap_or_else(|_| Err("the installer crashed".into()));
                self.result = Some((r, job.dest));
            }
        }
        match &self.result {
            Some((Ok(sum), dest)) => {
                ui.colored_label(OK, format!("Installed {} files and {} music tracks.", sum.files, sum.tracks));
                for n in &sum.notes {
                    ui.label(format!("Note: {n}"));
                }
                let cmd = format!("motorhead \"{}\"", dest.display());
                ui.horizontal(|ui| {
                    ui.label("Start the game with:");
                    if ui.small_button("Copy").clicked() {
                        ui.ctx().copy_text(cmd.clone());
                    }
                });
                ui.add(egui::Label::new(egui::RichText::new(&cmd).monospace()).selectable(true).wrap_mode(egui::TextWrapMode::Wrap));
            }
            Some((Err(e), _)) if e == "cancelled" => {
                ui.colored_label(BAD, "Cancelled. The folder holds a partial install; install again to finish it.");
            }
            Some((Err(e), _)) => {
                ui.colored_label(BAD, format!("Install failed: {e}"));
            }
            None => {}
        }
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.shot_step(&ctx);
        egui::CentralPanel::default().show(ui, |ui| {
            ui.heading("Install Motorhead from your CD image");
            ui.add_space(8.0);
            self.rows(ui);
            ui.add_space(6.0);
            let ready = self.status(ui);
            ui.add_space(6.0);
            ui.horizontal(|ui| match &self.job {
                Some(job) => {
                    let stopping = job.cancel.load(Ordering::Relaxed);
                    if ui.add_enabled(!stopping, egui::Button::new("Cancel")).clicked() {
                        job.cancel.store(true, Ordering::Relaxed);
                    }
                }
                None => {
                    if ui.add_enabled(ready, egui::Button::new("Install")).clicked() {
                        self.press_install();
                    }
                }
            });
            ui.add_space(6.0);
            self.progress(ui);
        });

        if self.confirm {
            let m = egui::Modal::new(egui::Id::unique("overwrite")).show(&ctx, |ui| {
                ui.set_width(380.0);
                ui.label(format!("{} is not empty. Files of the game there will be overwritten; your own files stay.", self.dest.trim()));
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui.button("Overwrite").clicked() {
                        self.confirm = false;
                        self.start();
                    }
                    if ui.button("Cancel").clicked() {
                        self.confirm = false;
                    }
                });
            });
            if m.should_close() {
                self.confirm = false;
            }
        }
    }
}

fn save_png(path: &Path, img: &egui::ColorImage) -> Result<(), String> {
    let f = std::fs::File::create(path).map_err(|e| e.to_string())?;
    let mut enc = png::Encoder::new(std::io::BufWriter::new(f), img.size[0] as u32, img.size[1] as u32);
    enc.set_color(png::ColorType::Rgba);
    let rgba: Vec<u8> = img.pixels.iter().flat_map(|p| p.to_array()).collect();
    enc.write_header().and_then(|mut w| w.write_image_data(&rgba)).map_err(|e| e.to_string())
}

fn dir_has_files(p: &Path) -> bool {
    std::fs::read_dir(p).is_ok_and(|mut d| d.next().is_some())
}

/// A disc image next to the program or in the working directory: the first .cue, else .iso.
fn find_image() -> Option<PathBuf> {
    let exe_dir = std::env::current_exe().ok().and_then(|e| e.parent().map(Path::to_path_buf));
    let dirs = [exe_dir, std::env::current_dir().ok()];
    for ext in ["cue", "iso"] {
        for d in dirs.iter().flatten() {
            let mut hits: Vec<PathBuf> = std::fs::read_dir(d)
                .into_iter()
                .flatten()
                .flatten()
                .map(|e| e.path())
                .filter(|p| p.is_file() && p.extension().is_some_and(|e| e.eq_ignore_ascii_case(ext)))
                .collect();
            hits.sort();
            if let Some(p) = hits.into_iter().next() {
                return Some(p);
            }
        }
    }
    None
}

/// "Motorhead" next to the image (beside the folder for a disc folder).
fn default_dest(image: &Path) -> PathBuf {
    let base = image.parent().filter(|p| !p.as_os_str().is_empty()).unwrap_or(Path::new("."));
    let base = std::path::absolute(base).unwrap_or_else(|_| base.to_path_buf());
    base.join("Motorhead")
}
