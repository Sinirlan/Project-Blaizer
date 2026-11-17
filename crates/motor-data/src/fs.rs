//! Locate original files: game paths use '\' and arbitrary case; Linux is case-sensitive.

use std::path::{Path, PathBuf};

/// Resolve a game path ("Tracks\\Track1\\x.myw") case-insensitively under `root`.
pub fn find_ci(root: &Path, rel: &str) -> Option<PathBuf> {
    let mut cur = root.to_path_buf();
    for part in rel.split(['\\', '/']).filter(|p| !p.is_empty()) {
        let exact = cur.join(part);
        if exact.exists() {
            cur = exact;
            continue;
        }
        let entry = std::fs::read_dir(&cur)
            .ok()?
            .flatten()
            .find(|e| e.file_name().to_string_lossy().eq_ignore_ascii_case(part))?;
        cur = entry.path();
    }
    Some(cur)
}

/// Read a game file; `Err` carries a readable message.
pub fn read(root: &Path, rel: &str) -> Result<Vec<u8>, String> {
    let p = find_ci(root, rel).ok_or_else(|| format!("missing {rel}"))?;
    std::fs::read(&p).map_err(|e| format!("{}: {e}", p.display()))
}

/// Read a (possibly TBL-encrypted) text config.
pub fn config(root: &Path, rel: &str) -> Result<crate::cfg::Config, String> {
    let raw = read(root, rel)?;
    let t = crate::tbl::load(&raw, None).map_err(|e| format!("{rel}: {e}"))?;
    Ok(crate::cfg::Config::parse(&t.data))
}

/// The car numbers NN of every `Game/carNN.car` in the install, ascending (the original globs `game\*.car`, list builder 0x4613ae
/// [O]; patch 3.0 adds car11..14, so the count is whatever the install holds).
pub fn car_ids(root: &Path) -> Vec<u32> {
    let Some(dir) = find_ci(root, "Game") else { return Vec::new() };
    let Ok(rd) = std::fs::read_dir(dir) else { return Vec::new() };
    let mut ids: Vec<u32> = rd
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().to_ascii_lowercase();
            let n = name.strip_prefix("car")?.strip_suffix(".car")?;
            (n.len() == 2 && n.bytes().all(|b| b.is_ascii_digit())).then(|| n.parse().ok()).flatten()
        })
        .collect();
    ids.sort_unstable();
    ids.dedup();
    ids
}

/// Shown when no install folder was given and none is found beside the program.
pub const NOT_FOUND: &str = "no game found here: put the program next to motor.exe, or pass the install folder as an argument";

/// The install folder when none is given: the first of the program's own folder and the working
/// folder that holds `motor.exe` (any case). An explicit folder argument always wins over this.
pub fn default_install_root() -> Option<PathBuf> {
    let exe_dir = std::env::current_exe().ok().and_then(|e| e.parent().map(Path::to_path_buf));
    [exe_dir, std::env::current_dir().ok()].into_iter().flatten().find(|d| has_motor_exe(d))
}

fn has_motor_exe(dir: &Path) -> bool {
    find_ci(dir, "motor.exe").is_some_and(|p| p.is_file())
}

/// Install folder for the tools that take it as their first argument: that argument unless it is a
/// `--flag` (or missing), then [`default_install_root`].
pub fn root_from_first_arg(first: Option<&str>) -> Result<PathBuf, String> {
    match first {
        Some(a) if !a.starts_with("--") => Ok(PathBuf::from(a)),
        _ => default_install_root().ok_or_else(|| NOT_FOUND.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn motor_exe_detected_in_any_case() {
        let d = std::env::temp_dir().join(format!("motor_fs_test_{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        assert!(!has_motor_exe(&d));
        std::fs::write(d.join("MOTOR.EXE"), b"").unwrap();
        assert!(has_motor_exe(&d));
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn car_ids_lists_the_car_files() {
        let d = std::env::temp_dir().join(format!("motor_car_ids_test_{}", std::process::id()));
        std::fs::create_dir_all(d.join("game")).unwrap();
        assert!(car_ids(&d).is_empty());
        for f in ["car02.car", "CAR11.CAR", "car01.car", "car1.car", "carxx.car", "car03.txt", "Track1.trk"] {
            std::fs::write(d.join("game").join(f), b"").unwrap();
        }
        assert_eq!(car_ids(&d), [1, 2, 11]);
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn explicit_argument_wins() {
        assert_eq!(root_from_first_arg(Some("somewhere")).unwrap(), PathBuf::from("somewhere"));
    }
}
