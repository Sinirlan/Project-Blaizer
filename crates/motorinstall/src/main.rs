//! motorinstall: install Motorhead from the user's CD image, no CD drive needed.
//!   motorinstall                          the installer window (gui.rs)
//!   motorinstall IMAGE DEST [--no-music]  the same from the command line
//!   motorinstall --shot out.png [--image X] [--dest D] [--install]   the window, captured
//!     (--install presses Install: the capture shows the result, or the overwrite question)
//! IMAGE = a .cue sheet (data + CD music), an .iso / raw .bin / .img (data only) or a folder
//! (a mounted disc or a CD copied to disk). DEST becomes the install folder the game takes.

// no console window behind the installer window on Windows; the CLI attaches to its terminal
#![cfg_attr(windows, windows_subsystem = "windows")]

mod disc;
mod gui;
mod install;
mod iscab;
mod iso9660;

use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::time::Instant;

const USAGE: &str = "usage: motorinstall IMAGE DEST [--no-music]\n  IMAGE: motorhead.cue (with CD music), an .iso/.bin/.img, or a disc folder";

fn main() {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    if argv.is_empty() || argv[0] == "--shot" {
        let shot = (!argv.is_empty()).then(|| shot_args(&argv));
        if let Err(e) = gui::run(shot) {
            fail(&format!("cannot open the window: {e}"));
        }
        return;
    }
    #[cfg(windows)]
    // SAFETY: plain Win32 call without pointers; fails harmlessly without a parent console
    unsafe {
        windows_sys::Win32::System::Console::AttachConsole(windows_sys::Win32::System::Console::ATTACH_PARENT_PROCESS);
    }
    let mut args = std::env::args().skip(1);
    let mut paths = Vec::new();
    let mut music = true;
    for a in args.by_ref() {
        match a.as_str() {
            "--no-music" => music = false,
            "-h" | "--help" => return println!("{USAGE}"),
            _ if a.starts_with("--") => fail(&format!("unknown option {a}\n{USAGE}")),
            _ => paths.push(PathBuf::from(a)),
        }
    }
    let [image, dest] = <[PathBuf; 2]>::try_from(paths).unwrap_or_else(|_| fail(USAGE));
    let src = disc::Source::open(&image).unwrap_or_else(|e| fail(&e));
    println!("{}: {} CD audio tracks", image.display(), src.audio.len());
    let t0 = Instant::now();
    let mut last = (u64::MAX, String::new());
    let mut report = |done: u64, total: u64, item: &str| {
        let pct = done * 100 / total.max(1);
        // one line per percent step or per music track
        let group = if item.starts_with("Music/") { item } else { "" };
        if pct != last.0 && (pct.is_multiple_of(5) || group != last.1) {
            println!("{pct:3}% {item}");
            last = (pct, group.to_string());
        }
    };
    let sum = install::install(&src, &dest, &install::Options { music }, &AtomicBool::new(false), &mut report).unwrap_or_else(|e| fail(&e));
    for n in &sum.notes {
        println!("note: {n}");
    }
    println!(
        "installed {} files and {} music tracks ({} MB) in {:.1} s\nplay: motorhead \"{}\"",
        sum.files,
        sum.tracks,
        sum.bytes >> 20,
        t0.elapsed().as_secs_f32(),
        dest.display()
    );
}

/// `--shot out.png [--image X] [--dest D] [--install]`: the window, captured.
fn shot_args(argv: &[String]) -> gui::Shot {
    let path = argv.get(1).map(PathBuf::from).unwrap_or_else(|| fail("--shot needs an output .png"));
    let mut shot = gui::Shot { path, image: None, dest: None, install: false };
    let mut it = argv[2..].iter();
    while let Some(a) = it.next() {
        let mut val = || it.next().map(PathBuf::from).unwrap_or_else(|| fail(&format!("{a} needs a path")));
        match a.as_str() {
            "--image" => shot.image = Some(val()),
            "--dest" => shot.dest = Some(val()),
            "--install" => shot.install = true,
            _ => fail(&format!("unknown --shot option {a}")),
        }
    }
    shot
}

fn fail(msg: &str) -> ! {
    eprintln!("motorinstall: {msg}");
    std::process::exit(1)
}
