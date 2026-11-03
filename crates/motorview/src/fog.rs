//! REMASTER per-track fog tuning [design choice, 17-fog-rework.md]: the Remaster haze needs hand-set
//! values per track (the track files can't tell a clear track from a foggy one: CueStart is 0.2-0.31
//! on all of them). A table in code gives the defaults; `<install>/remaster/fog/trackNN.fog` overrides
//! it and is written from the motorview F11 window. Distances are metres (the view-distance option
//! does not move the fog; the clip plane is guarded in the shader).

use crate::render::{FogColour, FogTune};
use std::path::{Path, PathBuf};

/// The code defaults per track (index = track number - 1). Start = CueStart x ViewDistance and Full =
/// ViewDistance of the track [O], the rest first guesses, set from shots (phase 4).
const TABLE: [FogTune; 8] = [
    FogTune { start: 36.0, full: 180.0, density: 0.33, height: 40.0, glow: 0.8, lamps: 5.0, ..FogTune::BASE },  // 1 RedRock
    FogTune { start: 56.0, full: 180.0, density: 0.17, height: 40.0, glow: 0.8, lamps: 5.0, ..FogTune::BASE },  // 2 NolbyHills (clear sky)
    FogTune { start: 40.0, full: 180.0, density: 0.44, height: 60.0, brightness: 1.5, glow: 0.8, lamps: 5.0, ..FogTune::BASE },  // 3 NeoCity (night: lifted)
    FogTune { start: 50.0, full: 180.0, density: 0.28, height: 50.0, glow: 0.8, lamps: 5.0, ..FogTune::BASE },  // 4 Atlantika
    FogTune { start: 40.0, full: 180.0, density: 0.44, height: 60.0, glow: 0.8, lamps: 5.0, ..FogTune::BASE },  // 5 OkkunSpeedway (night)
    FogTune { start: 40.0, full: 180.0, density: 0.22, height: 40.0, glow: 0.8, lamps: 5.0, ..FogTune::BASE },  // 6 Olympos (clear sky)
    FogTune { start: 24.0, full: 120.0, density: 0.83, height: 60.0, glow: 0.8, lamps: 5.0, ..FogTune::BASE },  // 7 RuhrStadt
    FogTune { start: 32.0, full: 160.0, density: 0.63, height: 60.0, brightness: 1.3, glow: 0.8, lamps: 5.0, ..FogTune::BASE },  // 8 Goldbridge (night: lifted)
];

/// The table's values for `track` (the global default for an unknown track).
pub fn track_default(track: u32) -> FogTune {
    track.checked_sub(1).and_then(|i| TABLE.get(i as usize)).copied().unwrap_or_default()
}

pub fn path(root: &Path, track: u32) -> PathBuf {
    root.join(format!("remaster/fog/track{track:02}.fog"))
}

/// The fog of `track`: the table, then the keys of its file. Problems are reported.
pub fn load(root: &Path, track: u32) -> FogTune {
    let base = track_default(track);
    let p = path(root, track);
    let Ok(text) = std::fs::read_to_string(&p) else { return base };
    let (f, warnings) = from_text(&text, base);
    for w in warnings {
        eprintln!("{}: {w}", p.display());
    }
    f
}

/// `Key value` lines over `base`; `#` starts a comment. `Colour` takes `sky`, `track` or `R G B`
/// (0..1), `Tint` takes `R G B`, every other key one number. A `# Motorhead fog v1` file is migrated:
/// its `Density` was per view distance (`base.full`), `GlowPower` has no equivalent and is dropped.
pub fn from_text(text: &str, base: FogTune) -> (FogTune, Vec<String>) {
    let (mut f, mut warnings) = (base, Vec::new());
    let v1 = text.lines().next().is_some_and(|l| l.contains("fog v1"));
    for (n, line) in text.lines().enumerate() {
        let line = line.split('#').next().unwrap_or("").trim();
        if line.is_empty() {
            continue;
        }
        let mut it = line.split_whitespace();
        let key = it.next().unwrap_or("").to_ascii_lowercase();
        let vals: Vec<f32> = it.clone().filter_map(|v| v.parse::<f32>().ok()).filter(|v| v.is_finite()).collect();
        let one = vals.first().copied();
        let three = (vals.len() >= 3).then(|| [vals[0], vals[1], vals[2]]);
        let mut need = |what: &str| warnings.push(format!("line {}: '{line}' needs {what}", n + 1));
        match key.as_str() {
            "colour" | "color" => match (it.next().map(str::to_ascii_lowercase).as_deref(), three) {
                (Some("sky"), _) => f.colour = FogColour::Sky,
                (Some("track"), _) => f.colour = FogColour::Track,
                (_, Some(c)) => f.colour = FogColour::Rgb(c),
                _ => need("sky, track or R G B"),
            },
            "tint" => match three {
                Some(c) => f.tint = c,
                None => need("R G B"),
            },
            "glowpower" => warnings.push(format!("line {}: GlowPower (fog v1) is dropped: the glow lobe is Anisotropy now", n + 1)),
            _ => {
                let Some(v) = one else {
                    need("a number");
                    continue;
                };
                match key.as_str() {
                    "start" => f.start = v,
                    "full" | "end" => f.full = v,
                    "opacity" => f.opacity = v,
                    "curve" => f.curve = v,
                    "density" => f.density = if v1 { v * 100.0 / base.full.max(1.0) } else { v },
                    "height" => f.height = v,
                    "brightness" => f.brightness = v,
                    "glow" => f.glow = v,
                    "anisotropy" => f.anisotropy = v,
                    "skyhaze" => f.sky_haze = v,
                    "lamps" => f.lamps = v,
                    "shafts" => f.shafts = v,
                    _ => warnings.push(format!("line {}: unknown key {key}", n + 1)),
                }
            }
        }
    }
    (f, warnings)
}

pub fn to_text(f: &FogTune) -> String {
    let colour = match f.colour {
        FogColour::Sky => "sky".to_string(),
        FogColour::Track => "track".to_string(),
        FogColour::Rgb([r, g, b]) => format!("{r} {g} {b}"),
    };
    format!(
        "# Motorhead fog v2 (17-fog-rework.md; metres, colours 0..1)\nStart {}\nFull {}\nOpacity {}\nCurve {}\nDensity {}\nHeight {}\nColour {colour}\nBrightness {}\nTint {} {} {}\nGlow {}\nAnisotropy {}\nSkyHaze {}\nLamps {}\nShafts {}\n",
        f.start, f.full, f.opacity, f.curve, f.density, f.height, f.brightness, f.tint[0], f.tint[1], f.tint[2], f.glow, f.anisotropy, f.sky_haze, f.lamps, f.shafts
    )
}

pub fn save(root: &Path, track: u32, f: &FogTune) -> std::io::Result<PathBuf> {
    let p = path(root, track);
    if let Some(d) = p.parent() {
        std::fs::create_dir_all(d)?;
    }
    std::fs::write(&p, to_text(f))?;
    Ok(p)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        let f = FogTune { start: 20.0, full: 150.0, opacity: 0.9, curve: 1.5, density: 0.35, height: 42.5, colour: FogColour::Rgb([0.5, 0.6, 0.7]), brightness: 1.2, tint: [1.0, 0.9, 0.8], glow: 0.2, anisotropy: 0.6, sky_haze: 0.4, lamps: 12.0, shafts: 0.7 };
        let (g, w) = from_text(&to_text(&f), FogTune::default());
        assert!(w.is_empty(), "{w:?}");
        assert_eq!(f, g);
        let (g, _) = from_text("Colour sky\n", f);
        assert_eq!(g.colour, FogColour::Sky);
    }

    #[test]
    fn partial_file_keeps_table() {
        let (g, w) = from_text("Density 0 # none\nBogus 1\nTint 1\n", track_default(2));
        assert_eq!(g.density, 0.0);
        assert_eq!(g.height, track_default(2).height);
        assert_eq!(w.len(), 2);
    }

    #[test]
    fn v1_migrates() {
        let (g, w) = from_text("# Motorhead fog v1\nDensity 0.9\nHeight 280\nGlow 0.8\nGlowPower 6\nLamps 1\n", track_default(3));
        assert!((g.density - 0.5).abs() < 1e-6);
        assert_eq!(g.height, 280.0);
        assert_eq!(g.lamps, 1.0);
        assert_eq!(w.len(), 1);
    }
}
