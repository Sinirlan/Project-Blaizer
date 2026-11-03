//! Line-based text configs (.car .trk .mat .adp .esd .clo .chf .srf), LightWave
//! motion files (LWMO) and scenes (LWSC). Keys are case-insensitive in the game; `//` starts a comment.

use crate::bytes::{err, latin1, Error, Result};

/// Whitespace as Python's `str.isspace()` sees Latin-1 text (keeps parity with cfg.py).
pub fn is_space(c: char) -> bool {
    matches!(c, '\t' | '\n' | '\x0b' | '\x0c' | '\r' | '\x1c'..='\x1f' | ' ' | '\u{85}' | '\u{a0}')
}

/// Non-empty, comment-stripped lines.
pub fn lines(text: &str) -> Vec<&str> {
    text.split('\n')
        .map(|l| l.split("//").next().unwrap_or("").trim_matches(|c| is_space(c) || c == '\r'))
        .filter(|l| !l.is_empty())
        .collect()
}

#[derive(Debug, Clone)]
pub struct Entry {
    pub key: String,
    pub values: Vec<String>,
}

impl Entry {
    pub fn f32(&self, i: usize) -> Option<f32> {
        self.values.get(i)?.parse().ok()
    }
    pub fn i64(&self, i: usize) -> Option<i64> {
        self.values.get(i)?.parse().ok()
    }
}

/// Parsed key/value config; keep order and duplicates (Halo, EnvMapName, Material...).
#[derive(Debug, Clone, Default)]
pub struct Config {
    pub entries: Vec<Entry>,
}

impl Config {
    pub fn parse(bytes: &[u8]) -> Config {
        let text = latin1(bytes);
        let entries = lines(&text)
            .into_iter()
            .map(|l| {
                let mut it = l.split(is_space).filter(|t| !t.is_empty());
                let key = it.next().unwrap_or("").to_string();
                Entry { key, values: it.map(str::to_string).collect() }
            })
            .collect();
        Config { entries }
    }

    /// First entry whose key matches case-insensitively.
    pub fn get(&self, key: &str) -> Option<&Entry> {
        self.entries.iter().find(|e| e.key.eq_ignore_ascii_case(key))
    }

    pub fn all<'a>(&'a self, key: &'a str) -> impl Iterator<Item = &'a Entry> + 'a {
        self.entries.iter().filter(move |e| e.key.eq_ignore_ascii_case(key))
    }
}

#[derive(Debug, Clone)]
pub struct MotionKey {
    /// x y z heading pitch bank sx sy sz
    pub values: Vec<f32>,
    pub frame: f32,
    pub linear: f32,
    pub tension: f32,
    pub continuity: f32,
    pub bias: f32,
}

#[derive(Debug, Clone)]
pub struct Motion {
    pub version: u32,
    pub channels: u32,
    pub keys: Vec<MotionKey>,
}

fn floats(line: &str) -> Result<Vec<f32>> {
    line.split(is_space)
        .filter(|t| !t.is_empty())
        .map(|t| t.parse::<f32>().map_err(|_| Error(format!("bad number {t:?} in LWMO"))))
        .collect()
}

/// `n` keys of 2 lines each from line `i`; returns them and the next line index.
fn motion_keys(t: &[&str], mut i: usize, n: usize) -> Result<(Vec<MotionKey>, usize)> {
    let mut keys = Vec::with_capacity(n);
    for _ in 0..n {
        let values = floats(t.get(i).ok_or_else(|| Error("LWMO truncated".into()))?)?;
        let s = floats(t.get(i + 1).ok_or_else(|| Error("LWMO truncated".into()))?)?;
        if s.len() < 5 {
            return err("LWMO key spline line too short");
        }
        keys.push(MotionKey { values, frame: s[0], linear: s[1], tension: s[2], continuity: s[3], bias: s[4] });
        i += 2;
    }
    Ok((keys, i))
}

/// LightWave motion ("LWMO"): version, channel count, key count, then 2 lines per key.
pub fn parse_lwmo(bytes: &[u8]) -> Result<Motion> {
    let text = latin1(bytes).replace('\r', "");
    let t: Vec<&str> = text.split('\n').collect();
    if t.first().map(|s| s.trim()) != Some("LWMO") || t.len() < 4 {
        return err("not an LWMO motion");
    }
    let num = |s: &str| s.trim().parse::<u32>().map_err(|_| Error(format!("bad LWMO header {s:?}")));
    let (version, channels, nkeys) = (num(t[1])?, num(t[2])?, num(t[3])? as usize);
    let (keys, _) = motion_keys(&t, 4, nkeys)?;
    Ok(Motion { version, channels, keys })
}

/// One `LoadObject` of a LightWave scene, as the scene loader 0x455390 reads it [O].
#[derive(Debug, Clone)]
pub struct SceneObject {
    /// file stem of the `.lwo` path; the game loads `<track>/Objects/<stem>.myo`
    pub name: String,
    /// inline motion (`ObjectMotion (unnamed)`); `Err(name)` for a motion file the game would
    /// load separately (0x456be4; no shipped scene has one)
    pub motion: std::result::Result<Motion, String>,
    /// EndBehavior: bit 1 set = repeat, else hold the last key (0x45571f)
    pub end_behavior: u32,
    /// UnseenByRays: drawn only; without it the object gets a rigid body and its `.COL`
    pub unseen_by_rays: bool,
    /// PolygonSize, read as mass / 100 (0x4556e8)
    pub polygon_size: Option<f32>,
    /// ObjEdgeColor, 2nd value (obj+0xce): the `ObjectColSound` id of a hit (0x48451e) [O]
    pub col_sound: u16,
    /// ObjEdgeColor, 1st value (obj+0xcc): the `ObjectEnvSound` id; every object with id n gets
    /// the .esd's `ObjectEnvSound n` sound (loader 0x425642) [O]
    pub env_sound: u16,
}

/// LightWave scene (`LWSC`, track `SceneName`). Only the keys the game reads are kept.
#[derive(Debug, Clone)]
pub struct Scene {
    /// FramesPerSecond (default 30, 0x4553a4)
    pub fps: f32,
    pub objects: Vec<SceneObject>,
}

pub fn parse_lwsc(bytes: &[u8]) -> Result<Scene> {
    let text = latin1(bytes).replace('\r', "");
    let t: Vec<&str> = text.split('\n').collect();
    if t.first().map(|s| s.trim()) != Some("LWSC") {
        return err("not an LWSC scene");
    }
    let mut scene = Scene { fps: 30.0, objects: Vec::new() };
    let mut i = 1;
    let mut in_object = false;
    while i < t.len() {
        let mut it = t[i].split(is_space).filter(|w| !w.is_empty());
        let (key, arg) = (it.next().unwrap_or(""), it.next());
        i += 1;
        // `ShadowOptions` ends the object (04 §4.9); the light and the camera after the last one
        // have their own `EndBehavior` that must not land on it (track 5's rotor lost its repeat)
        let obj = if in_object { scene.objects.last_mut() } else { None };
        match key {
            "LoadObject" => in_object = true,
            "ShadowOptions" => in_object = false,
            _ => {}
        }
        match (key, obj) {
            ("FramesPerSecond", _) => scene.fps = arg.and_then(|a| a.parse().ok()).unwrap_or(30.0),
            ("LoadObject", _) => {
                let path = arg.unwrap_or("");
                let file = path.rsplit('\\').next().unwrap_or(path);
                let name = file.split('.').next().unwrap_or(file).to_string();
                scene.objects.push(SceneObject { name, motion: Err(String::new()), end_behavior: 1, unseen_by_rays: false, polygon_size: None, col_sound: 0, env_sound: 0 });
            }
            ("ObjectMotion", Some(o)) => {
                if arg == Some("(unnamed)") {
                    let num = |s: Option<&&str>| s.and_then(|s| s.trim().parse::<usize>().ok()).ok_or_else(|| Error("bad ObjectMotion header".into()));
                    let (channels, n) = (num(t.get(i))?, num(t.get(i + 1))?);
                    let (keys, next) = motion_keys(&t, i + 2, n)?;
                    o.motion = Ok(Motion { version: 1, channels: channels as u32, keys });
                    i = next;
                } else {
                    o.motion = Err(arg.unwrap_or("").to_string());
                }
            }
            ("EndBehavior", Some(o)) => o.end_behavior = arg.and_then(|a| a.parse().ok()).unwrap_or(1),
            ("UnseenByRays", Some(o)) => o.unseen_by_rays = true,
            ("PolygonSize", Some(o)) => o.polygon_size = arg.and_then(|a| a.parse().ok()),
            ("ObjEdgeColor", Some(o)) => {
                o.env_sound = arg.and_then(|a| a.parse().ok()).unwrap_or(0);
                o.col_sound = it.next().and_then(|a| a.parse().ok()).unwrap_or(0);
            }
            _ => {}
        }
    }
    Ok(scene)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_values_and_comments() {
        let c = Config::parse(b"// header\r\nCarName\tKobra X3\r\n\r\nGears 6 // six\r\nhalo a\r\nHalo b\r\n");
        assert_eq!(c.entries.len(), 4);
        assert_eq!(c.get("carname").unwrap().values, vec!["Kobra", "X3"]);
        assert_eq!(c.get("GEARS").unwrap().i64(0), Some(6));
        assert_eq!(c.all("Halo").count(), 2);
    }

    #[test]
    fn lwmo() {
        let m = parse_lwmo(b"LWMO\n1\n9\n1\n1 2 3 0 0 0 1 1 1\n0 0 0 0 0\n").unwrap();
        assert_eq!((m.channels, m.keys.len()), (9, 1));
        assert_eq!(m.keys[0].values[2], 3.0);
    }

    #[test]
    fn lwsc_light_and_camera_keep_off_the_last_object() {
        let text = b"LWSC\n1\n\nLoadObject S:\\x\\rot.lwo\nObjectMotion (unnamed)\n  9\n  1\n  0 0 0 0 0 0 1 1 1\n  0 0 0 0 0\nEndBehavior 2\nShadowOptions 0\n\nAddLight\nLightMotion (unnamed)\n  9\n  1\n  0 0 0 0 0 0 1 1 1\n  0 0 0 0 0\nEndBehavior 1\n\nShowCamera 1 7\nCameraMotion (unnamed)\n  9\n  1\n  0 0 0 0 0 0 1 1 1\n  0 0 0 0 0\nEndBehavior 1\nPolygonSize 9\n";
        let s = parse_lwsc(text).unwrap();
        let o = &s.objects[0];
        assert_eq!((o.end_behavior, o.polygon_size, o.motion.as_ref().map(|m| m.keys.len())), (2, None, Ok(1)));
    }

    #[test]
    fn lwsc() {
        let text = b"LWSC\n1\n\nFramesPerSecond 24.0\n\nLoadObject S:\\x\\objects\\je_con1.lwo\nObjectMotion (unnamed)\n  9\n  2\n  1 2 3 90 0 0 1 1 1\n  0 0 0 0 0\n  1 2 4 90 0 0 1 1 1\n  30 0 0 0 0\nEndBehavior 2\nPolygonSize 0.05\nObjEdgeColor 0 7 0\n\nLoadObject S:\\x\\pelare.lwo\nUnseenByRays 1\nObjEdgeColor 12 0 0\n";
        let s = parse_lwsc(text).unwrap();
        assert_eq!((s.fps, s.objects.len()), (24.0, 2));
        let o = &s.objects[0];
        assert_eq!((o.name.as_str(), o.end_behavior, o.unseen_by_rays, o.polygon_size), ("je_con1", 2, false, Some(0.05)));
        assert_eq!(o.motion.as_ref().unwrap().keys[1].frame, 30.0);
        assert!(s.objects[1].unseen_by_rays);
        assert_eq!((o.col_sound, s.objects[1].col_sound), (7, 0));
        assert_eq!((o.env_sound, s.objects[1].env_sound), (0, 12));
    }
}
