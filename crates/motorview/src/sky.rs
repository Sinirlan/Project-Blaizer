//! REMASTER sky dome (09-remaster 9.32) [design choice]: an equirectangular HDRI (Poly Haven
//! style `.exr`, user-supplied in `<install>/remaster/sky/trackNN.exr`) drawn behind the world
//! in place of the original's flat Top/BottomColour. Decoded here, uploaded by `Gpu::load`.
//! A missing or unreadable file is not an error: the track keeps the Classic sky.
//!
//! Optional `<install>/remaster/sky/sky.cfg`, one line per track, `Key value` pairs after the name:
//! `Track06 File sunset.exr Yaw 20 Exposure -0.5 Sun off`. `File` replaces `trackNN.exr`; `Yaw` (degrees)
//! and `Exposure` (stops) add to the F11 sky dome values; `Sun off` keeps the track's own sun for the
//! light and shadows of cars and props (`SkyTune::sun`). The F11 Sky page's "save for track"
//! writes that line (`save`).

use std::path::{Path, PathBuf};

/// Largest value kept from the file (f16 tops out at 65504; the sun disc of a clear-sky HDRI
/// is far above it and would turn into inf in the mips).
const MAX_RADIANCE: f32 = 60000.0;

/// Columns of the mip level the irradiance is projected from (the sky is smooth at that size).
const SH_WIDTH: usize = 512;

/// Texels brighter than this many times the upper half's median luminance are scaled down to it
/// before the irradiance projection: the sun is `Globals::sun`, not ambient light.
const SH_SUN_CLIP: f32 = 8.0;

pub struct Hdri {
    pub width: u32,
    pub height: u32,
    pub mip_levels: u32,
    /// RGBA16F, all mip levels one after the other (mip 0 first)
    pub data: Vec<u8>,
    /// azimuth of the sun in the image, in the shader's convention (`atan2(x, -z)`, radians);
    /// the dome is turned by (track sun azimuth - this) so both suns agree
    pub sun_azimuth: f32,
    /// elevation of the sun above the horizon in the image (radians); the light of cars and
    /// props takes it when `SkyTune::sun` is on, so their shadows fall away from the visible sun
    pub sun_elevation: f32,
    /// REMASTER fog colour (17-fog-rework.md): mean radiance of the ring just above the horizon
    /// (elevation 0..5 degrees), linear, before the exposure
    pub horizon: [f32; 3],
    /// REMASTER sky ambient (09-remaster 9.32): `irradiance_sh`, linear, before the exposure
    pub sh: [[f32; 3]; 9],
    /// sky.cfg: extra turn (degrees) and exposure (stops), and whether the light follows the image's sun
    pub yaw: f32,
    pub exposure: f32,
    pub follow_sun: bool,
}

/// What the renderer keeps of an `Hdri` besides its texture.
#[derive(Clone, Copy, Debug)]
pub struct Dome {
    pub sun_azimuth: f32,
    pub sun_elevation: f32,
    /// `Hdri::horizon`
    pub horizon: [f32; 3],
    /// `Hdri::sh`
    pub sh: [[f32; 3]; 9],
    pub yaw: f32,
    pub exposure: f32,
    pub follow_sun: bool,
}

impl Hdri {
    pub fn dome(&self) -> Dome {
        Dome { sun_azimuth: self.sun_azimuth, sun_elevation: self.sun_elevation, horizon: self.horizon, sh: self.sh, yaw: self.yaw, exposure: self.exposure, follow_sun: self.follow_sun }
    }
}

/// Does this sky.cfg line name `track` (`Track06 ...`)?
fn names_track(name: &str, track: u32) -> bool {
    name.get(..5).is_some_and(|p| p.eq_ignore_ascii_case("track")) && name[5..].parse::<u32>() == Ok(track)
}

/// One track's line of sky.cfg.
struct Entry {
    file: Option<String>,
    yaw: f32,
    exposure: f32,
    follow_sun: bool,
}

fn entry(root: &Path, track: u32) -> Entry {
    let mut e = Entry { file: None, yaw: 0.0, exposure: 0.0, follow_sun: true };
    let Ok(text) = std::fs::read_to_string(root.join("remaster/sky/sky.cfg")) else { return e };
    for line in text.lines() {
        let line = line.split("//").next().unwrap_or("");
        let mut t = line.split_whitespace();
        let Some(name) = t.next() else { continue };
        if !names_track(name, track) {
            continue;
        }
        while let (Some(k), Some(v)) = (t.next(), t.next()) {
            match k.to_ascii_lowercase().as_str() {
                "file" => e.file = Some(v.to_string()),
                "yaw" => e.yaw = v.parse().unwrap_or(e.yaw),
                "exposure" => e.exposure = v.parse().unwrap_or(e.exposure),
                "sun" => e.follow_sun = !matches!(v.to_ascii_lowercase().as_str(), "off" | "0" | "no" | "false"),
                _ => eprintln!("sky.cfg: unknown key {k}"),
            }
        }
    }
    e
}

/// Write `track`'s line of `<root>/remaster/sky/sky.cfg` (its `File` is kept, other tracks' lines
/// and comments stay as they are) and return the path.
pub fn save(root: &Path, track: u32, yaw: f32, exposure: f32, follow_sun: bool) -> std::io::Result<PathBuf> {
    let path = root.join("remaster/sky/sky.cfg");
    let old = std::fs::read_to_string(&path).unwrap_or_default();
    let round = |x: f32| (x * 100.0).round() / 100.0;
    let new_line = |file: Option<&str>| {
        let file = file.map_or(String::new(), |f| format!(" File {f}"));
        let sun = if follow_sun { "" } else { " Sun off" };
        format!("Track{track:02}{file} Yaw {} Exposure {}{sun}", round(yaw), round(exposure))
    };
    let mut done = false;
    let mut lines: Vec<String> = old
        .lines()
        .map(|l| {
            let mut t = l.split("//").next().unwrap_or("").split_whitespace();
            if t.next().is_none_or(|n| !names_track(n, track)) {
                return l.to_string();
            }
            let file = std::iter::from_fn(|| Some((t.next()?, t.next()?))).find(|(k, _)| k.eq_ignore_ascii_case("file")).map(|(_, v)| v);
            done = true;
            new_line(file)
        })
        .collect();
    if !done {
        lines.push(new_line(None));
    }
    if let Some(d) = path.parent() {
        std::fs::create_dir_all(d)?;
    }
    std::fs::write(&path, lines.join("\n") + "\n")?;
    Ok(path)
}

/// `<root>/remaster/sky/track{NN}.exr`, if it is there and decodes.
pub fn load(root: &Path, track: u32) -> Option<Hdri> {
    let e = entry(root, track);
    let path = root.join("remaster/sky").join(e.file.clone().unwrap_or_else(|| format!("track{track:02}.exr")));
    if !path.is_file() {
        return None;
    }
    match decode(&path) {
        Ok(h) => Some(Hdri { yaw: e.yaw, exposure: e.exposure, follow_sun: e.follow_sun, ..h }),
        Err(e) => {
            eprintln!("sky: {}: {e}", path.display());
            None
        }
    }
}

fn decode(path: &Path) -> Result<Hdri, String> {
    let img = exr::prelude::read_first_rgba_layer_from_file(
        path,
        |res, _| (res.width(), res.height(), vec![[0f32; 4]; res.width() * res.height()]),
        |(w, _, px): &mut (usize, usize, Vec<[f32; 4]>), pos, (r, g, b, a): (f32, f32, f32, f32)| px[pos.y() * *w + pos.x()] = [r, g, b, a],
    )
    .map_err(|e| e.to_string())?;
    let (w, h, mut px) = img.layer_data.channel_data.pixels;
    if w < 2 || h < 2 {
        return Err("image too small".into());
    }
    for p in &mut px {
        for c in &mut p[..3] {
            *c = if c.is_finite() { c.clamp(0.0, MAX_RADIANCE) } else { 0.0 };
        }
        p[3] = 1.0;
    }
    let (sun_azimuth, sun_elevation) = sun(&px, w, h);
    let horizon = horizon(&px, w, h);
    let mip_levels = (w.max(h).ilog2() + 1).min(10);
    let mut sh = [[0.0; 3]; 9];
    let mut have_sh = false;
    let mut data = Vec::with_capacity(w * h * 8 * 4 / 3);
    let (mut lw, mut lh) = (w, h);
    let mut level = px;
    for l in 0..mip_levels {
        // the irradiance needs no more than 512 columns
        if lw <= SH_WIDTH && !have_sh {
            sh = irradiance_sh(&level, lw, lh);
            have_sh = true;
        }
        for p in &level {
            for c in p {
                data.extend_from_slice(&half::f16::from_f32(*c).to_bits().to_le_bytes());
            }
        }
        if l + 1 < mip_levels {
            (level, lw, lh) = downsample(&level, lw, lh);
        }
    }
    Ok(Hdri { width: w as u32, height: h as u32, mip_levels, data, sun_azimuth, sun_elevation, horizon, sh, yaw: 0.0, exposure: 0.0, follow_sun: true })
}

/// 2x2 box filter (the u axis wraps by itself, v is clamped).
fn downsample(src: &[[f32; 4]], w: usize, h: usize) -> (Vec<[f32; 4]>, usize, usize) {
    let (nw, nh) = ((w / 2).max(1), (h / 2).max(1));
    let mut out = vec![[0f32; 4]; nw * nh];
    for y in 0..nh {
        for x in 0..nw {
            let mut s = [0f32; 4];
            for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                let p = src[(y * 2 + dy).min(h - 1) * w + (x * 2 + dx).min(w - 1)];
                for k in 0..4 {
                    s[k] += p[k] * 0.25;
                }
            }
            out[y * nw + x] = s;
        }
    }
    (out, nw, nh)
}

/// Mean radiance of the rows from the horizon up to 5 degrees above it (the fog colour's source).
/// Texels brighter than 4x the ring's median luminance (the sun disc and its glare on a sunset
/// image) are left out, so the haze takes the sky's colour, not the sun's.
fn horizon(px: &[[f32; 4]], w: usize, h: usize) -> [f32; 3] {
    let rows = (h / 2).saturating_sub((h as f32 * 5.0 / 180.0).ceil() as usize)..h / 2;
    let ring = &px[rows.start * w..rows.end * w];
    let luma = |p: &[f32; 4]| 0.2126 * p[0] + 0.7152 * p[1] + 0.0722 * p[2];
    let mut lumas: Vec<f32> = ring.iter().map(luma).collect();
    if lumas.is_empty() {
        return [0.5; 3];
    }
    let mid = lumas.len() / 2;
    let limit = 4.0 * *lumas.select_nth_unstable_by(mid, f32::total_cmp).1;
    let mut s = [0f64; 3];
    let mut n = 0usize;
    for p in ring.iter().filter(|p| luma(p) <= limit) {
        for k in 0..3 {
            s[k] += p[k] as f64;
        }
        n += 1;
    }
    s.map(|c| if n > 0 { (c / n as f64) as f32 } else { 0.5 })
}

/// Azimuth and elevation of the brightest region: the luminance-weighted mean direction of the
/// texels within half of the peak luminance (the sun disc, or the glow of the brightest
/// cloud or lamp).
fn sun(px: &[[f32; 4]], w: usize, h: usize) -> (f32, f32) {
    let luma = |p: &[f32; 4]| 0.2126 * p[0] + 0.7152 * p[1] + 0.0722 * p[2];
    let peak = px.iter().map(luma).fold(0.0, f32::max);
    if peak <= 0.0 {
        return (0.0, std::f32::consts::FRAC_PI_4);
    }
    let (mut sx, mut sy, mut sup, mut sk) = (0.0f64, 0.0f64, 0.0f64, 0.0f64);
    for y in 0..h {
        // solid angle of the row
        let area = ((y as f32 + 0.5) / h as f32 * std::f32::consts::PI).sin() as f64;
        for x in 0..w {
            let l = luma(&px[y * w + x]);
            if l >= 0.5 * peak {
                let a = ((x as f32 + 0.5) / w as f32 - 0.5) * std::f32::consts::TAU;
                let k = l as f64 * area;
                sx += k * a.sin() as f64;
                sy += k * a.cos() as f64;
                sup += k * ((y as f32 + 0.5) / h as f32 * std::f32::consts::PI).cos() as f64;
                sk += k;
            }
        }
    }
    (sx.atan2(sy) as f32, (sup / sk).clamp(-1.0, 1.0).asin() as f32)
}

/// REMASTER sky ambient (09-remaster 9.32) [design choice]: the cosine-convolved irradiance of the
/// image as nine L2 spherical-harmonic terms (Ramamoorthi & Hanrahan), linear radiance, texels
/// weighted by solid angle. The cosine-lobe factors (pi, 2pi/3, pi/4) and the basis constants are
/// folded in, so with d in the image frame (x right, y up, z, the shader's `-z` forward)
///   E(d) = c0 + c1 y + c2 z + c3 x + c4 x y + c5 y z + c6 (3 z^2 - 1) + c7 x z + c8 (x^2 - y^2)
/// and a constant sky of radiance L gives E = pi L. The direction of a texel is the dome's
/// (`dome_radiance`): azimuth `a = (u - 0.5) 2pi`, polar angle `v pi`, d = (sin t sin a, cos t, -sin t cos a).
/// The sun is clipped first (`SH_SUN_CLIP`).
fn irradiance_sh(px: &[[f32; 4]], w: usize, h: usize) -> [[f32; 3]; 9] {
    let luma = |p: &[f32; 4]| 0.2126 * p[0] + 0.7152 * p[1] + 0.0722 * p[2];
    let mut upper: Vec<f32> = px[..w * h / 2].iter().map(luma).collect();
    let mid = upper.len() / 2;
    let limit = SH_SUN_CLIP * upper.select_nth_unstable_by(mid, f32::total_cmp).1.max(1e-4);
    let mut sh = [[0f64; 3]; 9];
    for y in 0..h {
        let t = (y as f32 + 0.5) / h as f32 * std::f32::consts::PI;
        let area = (t.sin() * std::f32::consts::TAU / w as f32 * std::f32::consts::PI / h as f32) as f64;
        for x in 0..w {
            let a = ((x as f32 + 0.5) / w as f32 - 0.5) * std::f32::consts::TAU;
            let (dx, dy, dz) = ((t.sin() * a.sin()) as f64, t.cos() as f64, (-t.sin() * a.cos()) as f64);
            let p = &px[y * w + x];
            let k = (limit / luma(p).max(1e-9)).min(1.0);
            let basis = [
                0.282095,
                0.488603 * dy,
                0.488603 * dz,
                0.488603 * dx,
                1.092548 * dx * dy,
                1.092548 * dy * dz,
                0.315392 * (3.0 * dz * dz - 1.0),
                1.092548 * dx * dz,
                0.546274 * (dx * dx - dy * dy),
            ];
            for (b, s) in basis.iter().zip(sh.iter_mut()) {
                for c in 0..3 {
                    s[c] += (p[c] * k) as f64 * b * area;
                }
            }
        }
    }
    // band factor * the basis constant the shader no longer has
    let pi = std::f64::consts::PI;
    let scale = [pi, 2.0 * pi / 3.0, 2.0 * pi / 3.0, 2.0 * pi / 3.0, pi / 4.0, pi / 4.0, pi / 4.0, pi / 4.0, pi / 4.0];
    let basis = [0.282095, 0.488603, 0.488603, 0.488603, 1.092548, 1.092548, 0.315392, 1.092548, 0.546274];
    std::array::from_fn(|i| sh[i].map(|c| (c * scale[i] * basis[i]) as f32))
}

/// Evaluates what `irradiance_sh` returns towards `d` (the shader's `sky_ambient` in Rust).
#[cfg(test)]
fn eval_sh(sh: &[[f32; 3]; 9], d: [f32; 3]) -> [f32; 3] {
    let [x, y, z] = d;
    let t = [1.0, y, z, x, x * y, y * z, 3.0 * z * z - 1.0, x * z, x * x - y * y];
    std::array::from_fn(|c| sh.iter().zip(t).map(|(s, t)| s[c] * t).sum())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn image(w: usize, h: usize, f: impl Fn(usize, usize) -> [f32; 3]) -> Vec<[f32; 4]> {
        (0..w * h).map(|i| { let c = f(i % w, i / w); [c[0], c[1], c[2], 1.0] }).collect()
    }

    #[test]
    fn sh_constant_sky_is_pi_l() {
        let sh = irradiance_sh(&image(64, 32, |_, _| [0.5, 1.0, 2.0]), 64, 32);
        for d in [[1.0, 0.0, 0.0], [-1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, -1.0, 0.0], [0.0, 0.0, 1.0], [0.0, 0.0, -1.0]] {
            let e = eval_sh(&sh, d);
            for (e, l) in e.iter().zip([0.5, 1.0, 2.0]) {
                assert!((e - std::f32::consts::PI * l).abs() < 0.02 * l, "{d:?}: {e} vs {}", std::f32::consts::PI * l);
            }
        }
    }

    #[test]
    fn sh_bright_top_half() {
        // white above the horizon, black below: E(up) = pi, E(down) = 0, E(side) = pi / 2 (L2 rings a little)
        let sh = irradiance_sh(&image(128, 64, |_, y| if y < 32 { [1.0; 3] } else { [0.0; 3] }), 128, 64);
        let up = eval_sh(&sh, [0.0, 1.0, 0.0])[0];
        let down = eval_sh(&sh, [0.0, -1.0, 0.0])[0];
        let side = eval_sh(&sh, [1.0, 0.0, 0.0])[0];
        assert!((up - std::f32::consts::PI).abs() < 0.3 && down.abs() < 0.3 && (side - std::f32::consts::FRAC_PI_2).abs() < 0.1, "{up} {down} {side}");
    }

    #[test]
    fn sh_dome_azimuth_convention() {
        // radiance only in the half of the image around u = 0.5 (azimuth 0 = -z): it lights normals facing -z
        let sh = irradiance_sh(&image(128, 64, |x, _| if (32..96).contains(&x) { [1.0; 3] } else { [0.0; 3] }), 128, 64);
        assert!(eval_sh(&sh, [0.0, 0.0, -1.0])[0] > eval_sh(&sh, [0.0, 0.0, 1.0])[0] + 0.5);
    }

    #[test]
    fn sh_clips_the_sun() {
        let plain = image(64, 32, |_, _| [1.0; 3]);
        let mut sunny = plain.clone();
        sunny[5 * 64 + 10] = [60000.0; 4];
        let (a, b) = (irradiance_sh(&plain, 64, 32), irradiance_sh(&sunny, 64, 32));
        assert!((a[0][0] - b[0][0]).abs() < 0.05 * a[0][0], "{} vs {}", a[0][0], b[0][0]);
    }

    #[test]
    fn save_rewrites_one_line() {
        let root = std::env::temp_dir().join(format!("motorhead-sky-test-{}", std::process::id()));
        std::fs::create_dir_all(root.join("remaster/sky")).unwrap();
        std::fs::write(root.join("remaster/sky/sky.cfg"), "// my skies\nTrack02 File a.exr Yaw 1\nTrack06 File sunset.exr Yaw 20 Exposure -0.5 // old\nTrack07 Sun off\n").unwrap();
        save(&root, 6, 35.25, -1.5, false).unwrap();
        save(&root, 3, 0.0, 0.0, true).unwrap();
        let text = std::fs::read_to_string(root.join("remaster/sky/sky.cfg")).unwrap();
        assert_eq!(
            text,
            "// my skies\nTrack02 File a.exr Yaw 1\nTrack06 File sunset.exr Yaw 35.25 Exposure -1.5 Sun off\nTrack07 Sun off\nTrack03 Yaw 0 Exposure 0\n"
        );
        let e = entry(&root, 6);
        assert_eq!((e.file.as_deref(), e.yaw, e.exposure, e.follow_sun), (Some("sunset.exr"), 35.25, -1.5, false));
        std::fs::remove_dir_all(&root).unwrap();
    }
}
