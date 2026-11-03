//! Stamps `MOTOR_VERSION` (the display string) for `src/lib.rs` (#338).
//!
//! The version and the stage come from the workspace `version` in the root
//! `Cargo.toml` (SemVer: `0.9.0-alpha`, `1.0.0-beta.1`, `1.0.0-rc.2`, `1.0.0`).
//! The short git commit is appended iff the version has a pre-release part.
//! `MOTOR_GIT_HASH` overrides the lookup (CI, source tarballs).

use std::process::Command;

fn git(args: &[&str]) -> Option<String> {
    let out = Command::new("git").args(args).output().ok()?;
    if !out.status.success() {
        return None;
    }
    let s = String::from_utf8(out.stdout).ok()?.trim().to_string();
    (!s.is_empty()).then_some(s)
}

fn main() {
    println!("cargo:rerun-if-env-changed=MOTOR_GIT_HASH");
    println!("cargo:rerun-if-changed=build.rs");

    let version = std::env::var("CARGO_PKG_VERSION").expect("cargo sets the version");
    let prerelease = version.contains('-');

    // The commit moves when HEAD or the branch ref it points to changes
    // (`--git-path` also works in worktrees, where `.git` is a file).
    if let Some(head) = git(&["rev-parse", "--git-path", "HEAD"]) {
        println!("cargo:rerun-if-changed={head}");
    }
    if let Some(r) = git(&["symbolic-ref", "-q", "HEAD"]) {
        if let Some(p) = git(&["rev-parse", "--git-path", &r]) {
            println!("cargo:rerun-if-changed={p}");
        }
    }

    let commit = std::env::var("MOTOR_GIT_HASH")
        .ok()
        .filter(|s| !s.is_empty())
        .or_else(|| git(&["rev-parse", "--short=7", "HEAD"]))
        .unwrap_or_else(|| "nogit".to_string());

    let display = if prerelease { format!("{version} {commit}") } else { version };
    println!("cargo:rustc-env=MOTOR_VERSION={display}");
    println!("cargo:rustc-env=MOTOR_COMMIT={commit}");
}
