//! The game's version, stage and git commit (#338).
//!
//! One source of truth: the workspace `version` in the root `Cargo.toml`. Its SemVer
//! pre-release field is the stage (`0.9.0-alpha`, `1.0.0-beta.1`, `1.0.0-rc.2`; none = release).
//! `build.rs` appends the short git commit to [`DISPLAY`] while the version is a pre-release.

/// Cargo version, e.g. `0.9.0-alpha` or `1.0.0`.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
/// Short git commit of the build (`nogit` outside a checkout).
pub const COMMIT: &str = env!("MOTOR_COMMIT");
/// What the menus, the window title and `--version` show: `0.9.0-alpha ee390bd`, or `1.0.0` at release.
pub const DISPLAY: &str = env!("MOTOR_VERSION");

/// The stage of a version: `"alpha"`, `"beta"`, `"rc"` (the pre-release name before any `.N`),
/// or `""` for a release.
pub fn stage(version: &str) -> &str {
    match version.split_once('-') {
        Some((_, pre)) => pre.split('.').next().unwrap_or(""),
        None => "",
    }
}

/// [`stage`] of this build.
pub fn current_stage() -> &'static str {
    stage(VERSION)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stages() {
        assert_eq!(stage("0.9.0-alpha"), "alpha");
        assert_eq!(stage("1.0.0-beta.2"), "beta");
        assert_eq!(stage("1.0.0-rc.1"), "rc");
        assert_eq!(stage("1.0.0"), "");
    }

    #[test]
    fn display_follows_the_rule() {
        if VERSION.contains('-') {
            assert_eq!(DISPLAY, format!("{VERSION} {COMMIT}"));
        } else {
            assert_eq!(DISPLAY, VERSION);
        }
    }
}
