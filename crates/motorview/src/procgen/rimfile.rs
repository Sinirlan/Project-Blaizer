//! REMASTER rim files (`remaster/rims/carNN.rim` in the install, written by `rimshop`): a
//! `RimStyle` as plain `Key value ...` lines like settings.cfg. `#` starts a comment, unknown
//! keys are ignored, missing ones keep the Titan defaults, list keys (`DishPoint`, `Group`,
//! `GroupPoint`) repeat.
//!
//! v2 (#337) writes `Version 2` and the spokes as groups: `Group multiple phase mirror section
//! depth smooth` (mirror 0 none, 1 twin, 2 cross; section 0 flat, 1 round, 2 ridge, 3 facet;
//! smooth 0/1), then that group's `GroupPoint radius sweep offset width height` lines. A v2 file
//! has exactly the groups it lists (none: no spokes). v1 files (no `Version`) carry one spoke
//! kind, `Bar` or `SpokePoint` / `SpokeTwin`, with its depth as the second `Depth` value; it
//! loads as one group that builds the same mesh.
//!
//! Details (#337 phase 3, any version): `DishPoint t depth [corner]` (corner 1 = sharp there; two
//! at one t = a vertical step, a stepped lip), a 7th `Group` value `onto_lip` (0/1), `Lugs count
//! radius size sides height phase`, `Rivets count radius size`, `CapShape sides dome`; all are
//! optional and written only when used.
//!
//! Window plates (#337 phase 2, any version): `Window r0 r1 depth lift count phase corner
//! groove`, then that plate's window outline as `WindowPoint radius angle` lines
//! (`rimplate::WindowPlate`). A file without them has no plates.

use super::{Lugs, Mirror, Rivets, RimStyle, Section, SpokeGroup, WindowPlate};
use std::path::Path;

const HEADER: &str = "# Motorhead rim v2 (rimshop)";

/// The style for `car`: its file under `<root>/remaster/rims/` if there is one, else the
/// built-in (`rim_style`). A broken file is reported and skipped.
pub fn load(root: &Path, car: u32) -> Option<RimStyle> {
    let path = root.join(format!("remaster/rims/car{car:02}.rim"));
    if let Ok(text) = std::fs::read_to_string(&path) {
        match from_text(&text) {
            Ok(style) => return Some(style),
            Err(e) => eprintln!("{}: {e}", path.display()),
        }
    }
    super::rim_style(car)
}

pub fn to_text(s: &RimStyle) -> String {
    let b = &s.barrel;
    let mut t = format!("{HEADER}\n");
    let mut line = |k: &str, v: &[f32]| {
        t.push_str(k);
        for x in v {
            t.push_str(&format!(" {x}"));
        }
        t.push('\n');
    };
    line("Version", &[2.0]);
    line("Spokes", &[s.spokes as f32]);
    line("Cap", &[s.cap]);
    line("Star", &s.star);
    line("StarDepth", &[s.star_depth]);
    line("Flange", &[s.flange]);
    line("Caliper", &s.caliper.map(|c| c as f32));
    line("CaliperAt", &[s.caliper_at]);
    line("CaliperSpan", &[s.caliper_span]);
    line("Rim", &[b.rim]);
    line("Dish", &[b.dish]);
    line("FaceInset", &[b.face_inset]);
    line("Bead", &[b.bead]);
    line("TyreShape", &[b.tyre_shape]);
    line("InnerInset", &[b.inner_inset]);
    line("LipRing", &[b.lip_ring]);
    line("Chamfer", &[b.chamfer]);
    line("Floor", &[b.floor]);
    line("Tyre", &b.tyre_rgb.map(|c| c as f32));
    for p in b.dish_curve.iter().flatten() {
        line("DishPoint", if p[2] != 0.0 { &p[..] } else { &p[..2] });
    }
    for g in &s.groups {
        let mirror = Mirror::ALL.iter().position(|&m| m == g.mirror).unwrap_or(0);
        let section = Section::ALL.iter().position(|&m| m == g.section).unwrap_or(0);
        line("Group", &[g.multiple as f32, g.phase, mirror as f32, section as f32, g.depth, g.smooth as u8 as f32, g.onto_lip as u8 as f32]);
        for p in &g.points {
            line("GroupPoint", p);
        }
    }
    if s.cap_sides != 0 || s.cap_dome != 0.0 {
        line("CapShape", &[s.cap_sides as f32, s.cap_dome]);
    }
    if let Some(l) = &s.lugs {
        line("Lugs", &[l.count as f32, l.radius, l.size, l.sides as f32, l.height, l.phase]);
    }
    if let Some(r) = &s.rivets {
        line("Rivets", &[r.count as f32, r.radius, r.size]);
    }
    for w in &s.windows {
        line("Window", &[w.r0, w.r1, w.depth, w.lift, w.count as f32, w.phase, w.corner, w.groove]);
        for p in &w.outline {
            line("WindowPoint", p);
        }
    }
    t
}

pub fn from_text(text: &str) -> Result<RimStyle, String> {
    let mut s = RimStyle::default();
    let mut version = 1;
    let mut dish: Vec<[f32; 3]> = Vec::new();
    let mut groups: Vec<SpokeGroup> = Vec::new();
    // v1: one spoke kind, its depth the second `Depth` value (the Titan's when missing)
    let mut bar = [0.24, 0.93, 0.09, 0.05, 0.11];
    let mut bar_depth = 0.12;
    let mut spoke: Vec<[f32; 3]> = Vec::new();
    let mut twin = None;
    for (n, raw) in text.lines().enumerate() {
        let l = raw.split('#').next().unwrap_or("").trim();
        let mut it = l.split_whitespace();
        let Some(key) = it.next() else { continue };
        let v: Vec<f32> = it.map(|x| x.parse::<f32>().map_err(|_| format!("line {}: bad number '{x}'", n + 1))).collect::<Result<_, _>>()?;
        let need = |k: usize| if v.len() >= k { Ok(()) } else { Err(format!("line {}: {key} needs {k} values", n + 1)) };
        let b = &mut s.barrel;
        match key {
            "Version" => { need(1)?; version = v[0] as u32; }
            "Spokes" => { need(1)?; s.spokes = (v[0] as usize).clamp(2, 24); }
            "Cap" => { need(1)?; s.cap = v[0]; }
            "Star" => { need(3)?; s.star = [v[0], v[1], v[2]]; }
            "StarDepth" => { need(1)?; s.star_depth = v[0]; }
            "Flange" => { need(1)?; s.flange = v[0]; }
            "Caliper" => { need(3)?; s.caliper = [v[0], v[1], v[2]].map(|c| c.clamp(0.0, 255.0) as u8); }
            "CaliperAt" => { need(1)?; s.caliper_at = v[0]; }
            "CaliperSpan" => { need(1)?; s.caliper_span = v[0]; }
            "Rim" => { need(1)?; b.rim = v[0]; }
            "Dish" => { need(1)?; b.dish = v[0]; }
            "FaceInset" => { need(1)?; b.face_inset = v[0]; }
            "Bead" => { need(1)?; b.bead = v[0]; }
            "TyreShape" => { need(1)?; b.tyre_shape = v[0]; }
            "InnerInset" => { need(1)?; b.inner_inset = v[0]; }
            "LipRing" => { need(1)?; b.lip_ring = v[0]; }
            "Chamfer" => { need(1)?; b.chamfer = v[0]; }
            "Floor" => { need(1)?; b.floor = v[0]; }
            "Tyre" => { need(3)?; b.tyre_rgb = [v[0], v[1], v[2]].map(|c| c.clamp(0.0, 255.0) as u8); }
            "DishPoint" => { need(2)?; dish.push([v[0], v[1], v.get(2).map_or(0.0, |&c| (c != 0.0) as u8 as f32)]); }
            "CapShape" => { need(2)?; s.cap_sides = (v[0].max(0.0) as usize).min(12); s.cap_dome = v[1]; }
            "Lugs" => {
                need(6)?;
                s.lugs = Some(Lugs { count: (v[0].max(0.0) as usize).min(24), radius: v[1], size: v[2], sides: (v[3].max(0.0) as usize).min(12), height: v[4], phase: v[5] });
            }
            "Rivets" => {
                need(3)?;
                s.rivets = Some(Rivets { count: (v[0].max(0.0) as usize).min(64), radius: v[1], size: v[2] });
            }
            "Group" => {
                need(5)?;
                let pick = |x: f32, n: usize| (x.max(0.0) as usize).min(n - 1);
                groups.push(SpokeGroup {
                    multiple: (v[0] as usize).clamp(1, 8),
                    phase: v[1],
                    points: Vec::new(),
                    mirror: Mirror::ALL[pick(v[2], Mirror::ALL.len())],
                    section: Section::ALL[pick(v[3], Section::ALL.len())],
                    depth: v[4],
                    smooth: v.get(5).is_some_and(|&x| x != 0.0),
                    onto_lip: v.get(6).is_some_and(|&x| x != 0.0),
                });
            }
            "GroupPoint" => {
                need(5)?;
                groups.last_mut().ok_or(format!("line {}: GroupPoint before any Group", n + 1))?.points.push([v[0], v[1], v[2], v[3], v[4]]);
            }
            "Window" => {
                need(8)?;
                s.windows.push(WindowPlate { r0: v[0], r1: v[1], depth: v[2], lift: v[3], count: (v[4].max(0.0) as usize).min(64), phase: v[5], outline: Vec::new(), corner: v[6], groove: v[7] });
            }
            "WindowPoint" => {
                need(2)?;
                s.windows.last_mut().ok_or(format!("line {}: WindowPoint before any Window", n + 1))?.outline.push([v[0], v[1]]);
            }
            // v1
            "Depth" => { need(2)?; s.star_depth = v[0]; bar_depth = v[1]; }
            "Bar" => { need(5)?; bar = [v[0], v[1], v[2], v[3], v[4]]; }
            "SpokePoint" => { need(3)?; spoke.push([v[0], v[1], v[2]]); }
            "SpokeTwin" => { need(1)?; twin = Some(v[0] != 0.0); }
            _ => {}
        }
    }
    if dish.len() >= 2 {
        dish.sort_by(|a, b| a[0].total_cmp(&b[0]));
        s.barrel.dish_curve = Some(dish);
    }
    s.groups = if version >= 2 || !groups.is_empty() {
        for g in &mut groups {
            g.points.sort_by(|a, b| a[0].total_cmp(&b[0]));
        }
        groups
    } else if spoke.len() >= 2 {
        spoke.sort_by(|a, b| a[0].total_cmp(&b[0]));
        vec![SpokeGroup::from_curve(&spoke, twin.unwrap_or(false), bar_depth)]
    } else {
        vec![SpokeGroup::from_bar(bar, bar_depth)]
    };
    Ok(s)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_round_trips() {
        let s = RimStyle::default();
        assert_eq!(from_text(&to_text(&s)).unwrap(), s);
    }

    /// every mirror and section kind, several groups, a dish curve
    #[test]
    fn groups_round_trip() {
        let mut s = RimStyle { spokes: 7, ..Default::default() };
        s.barrel.dish_curve = Some(vec![[0.0, 0.3, 0.0], [0.5, 0.12, 0.0], [1.0, 0.0, 0.0]]);
        s.groups = Mirror::ALL.iter().zip(Section::ALL.iter().cycle()).enumerate().map(|(i, (&mirror, &section))| SpokeGroup {
            multiple: i + 1,
            phase: 0.25 * i as f32,
            points: vec![[0.2, -0.1, 0.0, 0.1, 0.0], [0.6, 0.3, 0.04, 0.07, -0.05], [0.93, 0.9, 0.1, 0.09, 0.02]],
            mirror,
            section,
            depth: 0.1 + 0.01 * i as f32,
            smooth: i % 2 == 0,
            onto_lip: i == 1,
        }).collect();
        s.groups.push(SpokeGroup { section: Section::Facet, ..s.groups[0].clone() });
        assert_eq!(from_text(&to_text(&s)).unwrap(), s);
        // a v2 file without groups has no spokes
        let none = RimStyle { groups: Vec::new(), ..Default::default() };
        assert_eq!(from_text(&to_text(&none)).unwrap(), none);
    }

    /// the phase 3 details: corners and a step in the dish, lugs, rivets, a shaped cap; files
    /// without them load as before and write nothing for them
    #[test]
    fn details_round_trip() {
        let mut s = RimStyle { cap_sides: 6, cap_dome: 0.2, lugs: Some(Lugs { count: 6, radius: 0.15, size: 0.025, sides: 0, height: 0.07, phase: 0.5 }), rivets: Some(Rivets { count: 12, radius: 0.96, size: 0.012 }), ..Default::default() };
        s.barrel.dish_curve = Some(vec![[0.0, 0.4, 0.0], [0.5, 0.2, 1.0], [0.8, 0.2, 1.0], [0.8, 0.05, 1.0], [1.0, 0.0, 0.0]]);
        s.groups[0].onto_lip = true;
        let t = to_text(&s);
        assert_eq!(from_text(&t).unwrap(), s);
        assert!(t.contains("DishPoint 0.5 0.2 1\n") && t.contains("DishPoint 0 0.4\n"));
        let plain = to_text(&RimStyle::default());
        assert!(!plain.contains("Lugs") && !plain.contains("Rivets") && !plain.contains("CapShape"));
        assert!(from_text("Lugs 5 0.1").is_err() && from_text("CapShape 6").is_err());
        // a v2 Group line without the 7th value (phase 1/2 files) loads with the spokes under the ring
        assert!(!from_text("Version 2\nGroup 1 0 0 0 0.1 0\nGroupPoint 0.2 0 0 0.1 0\nGroupPoint 0.9 0 0 0.1 0\n").unwrap().groups[0].onto_lip);
    }

    /// window plates of every kind, beside groups; a cover without windows
    #[test]
    fn windows_round_trip() {
        use super::super::rimplate::{circle, teardrop, trapezoid};
        let s = RimStyle { windows: vec![
            WindowPlate::default(),
            WindowPlate { r0: 0.1, r1: 0.8, depth: 0.1, lift: -0.02, count: 6, phase: 0.25, outline: teardrop(0.3, 0.75, 0.0, 0.2), corner: 0.015, groove: 0.0 },
            WindowPlate { count: 8, outline: trapezoid(0.3, 0.8, 0.12, 0.2), groove: 0.04, ..Default::default() },
            WindowPlate { count: 0, outline: Vec::new(), ..Default::default() },
            WindowPlate { outline: circle(0.5, 0.1, 7), ..Default::default() },
        ], ..Default::default() };
        assert_eq!(from_text(&to_text(&s)).unwrap(), s);
        assert!(from_text("WindowPoint 0.5 0").is_err());
        assert!(from_text("Window 0 0.8 0.1 0 5 0.5 0").is_err());
    }

    /// v1 bars and curves load as one group with the bar depth, the star depth first
    #[test]
    fn v1_files_load_as_one_group() {
        let s = from_text("Spokes 7\nBar 0.2 0.9 0.1 0 0\nDepth 0.2 0.3\n").unwrap();
        assert_eq!((s.spokes, s.star_depth), (7, 0.2));
        assert_eq!(s.groups, vec![SpokeGroup::from_bar([0.2, 0.9, 0.1, 0.0, 0.0], 0.3)]);
        let s = from_text("SpokeTwin 1\nSpokePoint 0.93 0.1 0.09\nSpokePoint 0.2 0 0.1\n").unwrap();
        assert_eq!(s.groups, vec![SpokeGroup::from_curve(&[[0.2, 0.0, 0.1], [0.93, 0.1, 0.09]], true, 0.12)]);
        assert_eq!(s.groups[0].mirror, Mirror::Twin);
    }

    #[test]
    fn bad_input_is_an_error_and_unknown_keys_pass() {
        assert!(from_text("Spokes five").is_err());
        assert!(from_text("Star 1 2").is_err());
        assert!(from_text("GroupPoint 0.2 0 0 0.1 0").is_err());
        assert_eq!(from_text("# hi\nWhatever 3\n").unwrap(), RimStyle::default());
    }
}
