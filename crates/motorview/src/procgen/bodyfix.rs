//! REMASTER (#292): hand fixes of a generated car body, saved from Blender
//! (`tools/blender/motorhead_body.py`) as `assets/cars/carNN.fix` (embedded; an install's
//! `remaster/cars/carNN.fix`, or `--cars-dir`'s, overrides it). One fix per line,
//! `at=x,y,z by=dx,dy,dz`, in game car-local metres before the ride-height drop (the frame of
//! `BodyJob::run_parts` just before the drop line, and of the OBJ export). `#` starts a comment;
//! `# stale at=.. by=..` lines are fixes that matched no point when Blender last looked, and
//! are tried again here, as the add-on does on each import.
//!
//! A fix moves the nearest generated point within `MATCH_RADIUS`, and every triangle corner at
//! that point with it (welded copies stay welded). Fixes that match nothing after a recipe
//! change are `stale`: the caller reports them so they cannot go unnoticed.

use crate::scene::Vertex;
use glam::Vec3;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// m: a fix moves the nearest generated point within 2 mm [design choice, the add-on's value]
pub const MATCH_RADIUS: f32 = 2e-3;

/// One fix, both vectors in game space.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Fix {
    pub at: [f32; 3],
    pub by: [f32; 3],
}

impl std::fmt::Display for Fix {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let (a, b) = (self.at, self.by);
        write!(f, "at={:.6},{:.6},{:.6} by={:.6},{:.6},{:.6}", a[0], a[1], a[2], b[0], b[1], b[2])
    }
}

/// What `apply` did.
#[derive(Default, Debug)]
pub struct Report {
    /// fixes in the file
    pub total: usize,
    /// fixes that found a point
    pub applied: usize,
    /// the ones that did not
    pub stale: Vec<Fix>,
}

impl Report {
    /// the stats line's part for a car with fixes, e.g. `fixes 3 of 4, 1 stale`
    pub fn summary(&self) -> String {
        match self.stale.len() {
            0 => format!("fixes {} of {}", self.applied, self.total),
            n => format!("fixes {} of {}, {n} stale", self.applied, self.total),
        }
    }
}

/// The fixes built into the program for `car` (`assets/cars/`).
pub fn embedded(car: u32) -> Option<&'static str> {
    match car {
        1 => Some(include_str!("../../../../assets/cars/car01.fix")),
        2 => Some(include_str!("../../../../assets/cars/car02.fix")),
        3 => Some(include_str!("../../../../assets/cars/car03.fix")),
        4 => Some(include_str!("../../../../assets/cars/car04.fix")),
        5 => Some(include_str!("../../../../assets/cars/car05.fix")),
        6 => Some(include_str!("../../../../assets/cars/car06.fix")),
        7 => Some(include_str!("../../../../assets/cars/car07.fix")),
        8 => Some(include_str!("../../../../assets/cars/car08.fix")),
        9 => Some(include_str!("../../../../assets/cars/car09.fix")),
        10 => Some(include_str!("../../../../assets/cars/car10.fix")),
        _ => None,
    }
}

/// The file that overrides `car`'s embedded fixes, next to its recipe override
/// (`bodyfile::path`). It need not exist (motorview watches it, so one written later counts).
pub fn path(root: &Path, car: u32) -> PathBuf {
    super::bodyfile::override_path(root, &format!("car{car:02}.fix"))
}

/// `car`'s fixes: the override file if there is one (a broken file is reported and skipped),
/// else the embedded ones.
pub fn load(root: &Path, car: u32) -> Vec<Fix> {
    let p = path(root, car);
    if let Ok(text) = std::fs::read_to_string(&p) {
        match parse(&text) {
            Ok(fixes) => {
                if !fixes.is_empty() {
                    eprintln!("car{car:02}: {} body fixes from {}", fixes.len(), p.display());
                }
                return fixes;
            }
            Err(e) => eprintln!("{}: {e}", p.display()),
        }
    }
    embedded(car).map(|t| parse(t).unwrap_or_else(|e| panic!("embedded car{car:02}.fix: {e}"))).unwrap_or_default()
}

/// The fixes of a `.fix` file's text; a line that cannot be read is an error naming it.
pub fn parse(text: &str) -> Result<Vec<Fix>, String> {
    let mut out = Vec::new();
    for (n, raw) in text.lines().enumerate() {
        let line = raw.trim();
        let body = if let Some(c) = line.strip_prefix('#') {
            // only `# stale at=..` is a fix; any other comment is skipped
            match c.trim().strip_prefix("stale").map(str::trim) {
                Some(b) if b.starts_with("at=") => b,
                _ => continue,
            }
        } else if line.is_empty() {
            continue;
        } else {
            line
        };
        let bad = || format!("line {}: cannot read {raw:?} (want at=x,y,z by=dx,dy,dz)", n + 1);
        let mut it = body.split_whitespace();
        let (at, by) = (it.next().and_then(|t| t.strip_prefix("at=")), it.next().and_then(|t| t.strip_prefix("by=")));
        if it.next().is_some() {
            return Err(bad());
        }
        let triple = |s: Option<&str>| -> Option<[f32; 3]> {
            let v: Vec<f32> = s?.split(',').map(|x| x.parse().ok().filter(|f: &f32| f.is_finite())).collect::<Option<_>>()?;
            v.try_into().ok()
        };
        match (triple(at), triple(by)) {
            (Some(at), Some(by)) => out.push(Fix { at, by }),
            _ => return Err(bad()),
        }
    }
    Ok(out)
}

/// game space -> the renderer's view space (z negated)
fn view(p: [f32; 3]) -> Vec3 {
    Vec3::new(p[0], p[1], -p[2])
}

/// Move `verts` (triangle list, view space) by `fixes`. Each fix takes the nearest distinct point
/// within `MATCH_RADIUS` (477 point pairs of R-Build 2 are closer than 2 mm: nearest, not
/// first); two fixes on a point add up. Every corner at a moved point moves with it.
/// The normals follow, APPROX [design choice]: per smoothing group (corners at one position with
/// one normal, so creases stay creases) the normal tilts by the mean change of its triangles'
/// face normals, first order, which is right for the small moves fixes are for. Nothing is
/// touched without a matching fix.
pub fn apply(verts: &mut [Vertex], fixes: &[Fix]) -> Report {
    let mut rep = Report { total: fixes.len(), ..Report::default() };
    if fixes.is_empty() {
        return rep;
    }
    // the distinct points, hashed in cells of the match radius
    let cell = |p: Vec3| (p / MATCH_RADIUS).floor().as_ivec3().to_array();
    let mut ids: HashMap<[u32; 3], usize> = HashMap::new();
    let mut pts: Vec<Vec3> = Vec::new();
    let mut grid: HashMap<[i32; 3], Vec<usize>> = HashMap::new();
    for v in verts.iter() {
        ids.entry(v.pos.map(f32::to_bits)).or_insert_with(|| {
            let p = Vec3::from(v.pos);
            pts.push(p);
            grid.entry(cell(p)).or_default().push(pts.len() - 1);
            pts.len() - 1
        });
    }
    let mut delta: HashMap<usize, Vec3> = HashMap::new();
    for fx in fixes {
        let at = view(fx.at);
        let c = cell(at);
        let mut best: Option<(f32, usize)> = None;
        for dx in -1..=1 {
            for dy in -1..=1 {
                for dz in -1..=1 {
                    for &i in grid.get(&[c[0] + dx, c[1] + dy, c[2] + dz]).into_iter().flatten() {
                        let d = pts[i].distance(at);
                        if d <= MATCH_RADIUS && best.is_none_or(|(b, _)| d < b) {
                            best = Some((d, i));
                        }
                    }
                }
            }
        }
        match best {
            Some((_, i)) => {
                *delta.entry(i).or_default() += view(fx.by);
                rep.applied += 1;
            }
            None => rep.stale.push(*fx),
        }
    }
    if delta.is_empty() {
        return rep;
    }
    // face normals before and after, and the change per smoothing group (sum, corners)
    let unit = |p: [Vec3; 3]| (p[1] - p[0]).cross(p[2] - p[0]).normalize_or_zero();
    let mut group: HashMap<([u32; 3], [u32; 3]), (Vec3, f32)> = HashMap::new();
    for t in verts.as_chunks::<3>().0 {
        let old = [0, 1, 2].map(|k| Vec3::from(t[k].pos));
        let new = [0, 1, 2].map(|k| old[k] + delta.get(&ids[&t[k].pos.map(f32::to_bits)]).copied().unwrap_or(Vec3::ZERO));
        let d = unit(new) - unit(old);
        for v in t {
            let g = group.entry((v.pos.map(f32::to_bits), v.normal.map(f32::to_bits))).or_default();
            g.0 += d;
            g.1 += 1.0;
        }
    }
    let new_normals: Vec<[f32; 3]> = verts
        .iter()
        .map(|v| match group[&(v.pos.map(f32::to_bits), v.normal.map(f32::to_bits))] {
            (s, n) if s != Vec3::ZERO => (Vec3::from(v.normal) + s / n).normalize_or(Vec3::from(v.normal)).to_array(),
            _ => v.normal,
        })
        .collect();
    for (v, n) in verts.iter_mut().zip(new_normals) {
        if let Some(d) = delta.get(&ids[&v.pos.map(f32::to_bits)]) {
            v.pos = (Vec3::from(v.pos) + *d).to_array();
        }
        v.normal = n;
    }
    rep
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vx(pos: [f32; 3], normal: [f32; 3]) -> Vertex {
        Vertex { pos, uv: [0.0; 2], color: [1.0; 4], layer: -1.0, normal, env: 1.0 }
    }

    /// two triangles of a flat square (normal +Y) sharing the diagonal
    fn square() -> Vec<Vertex> {
        let n = [0.0, 1.0, 0.0];
        let p = [[0.0, 0.0, 0.0], [0.0, 0.0, 0.1], [0.1, 0.0, 0.1], [0.1, 0.0, 0.0]];
        [[0, 1, 2], [0, 2, 3]].iter().flatten().map(|&i| vx(p[i], n)).collect()
    }

    #[test]
    fn parse_reads_fixes_comments_and_stale_lines() {
        let t = "# header\n# a fix moves the nearest point; '# stale' lines\n\nat=1,2,3 by=0.01,0,-0.5\n# stale at=0,0,0 by=1,1,1\n";
        let f = parse(t).unwrap();
        assert_eq!(f, vec![Fix { at: [1.0, 2.0, 3.0], by: [0.01, 0.0, -0.5] }, Fix { at: [0.0; 3], by: [1.0; 3] }]);
        let e = parse("at=1,2,3 by=1,2\n").unwrap_err();
        assert!(e.starts_with("line 1:"), "{e}");
        assert!(parse("at=1,2,3\n").is_err() && parse("garbage\n").is_err() && parse("at=1,2,3 by=nan,0,0\n").is_err());
    }

    #[test]
    fn embedded_files_parse() {
        for car in 1..=10 {
            parse(embedded(car).unwrap()).unwrap_or_else(|e| panic!("car{car:02}.fix: {e}"));
        }
    }

    #[test]
    fn no_fixes_touch_nothing() {
        let mut v = square();
        let before: Vec<_> = v.iter().map(|x| (x.pos, x.normal)).collect();
        assert_eq!(apply(&mut v, &[]).total, 0);
        assert_eq!(before, v.iter().map(|x| (x.pos, x.normal)).collect::<Vec<_>>());
    }

    #[test]
    fn a_fix_moves_all_welded_copies_and_tilts_the_normals() {
        let mut v = square();
        // game z is the view z negated: the point at view (0.1, 0, 0.1) is game (0.1, 0, -0.1)
        let r = apply(&mut v, &[Fix { at: [0.1, 0.0, -0.1], by: [0.0, 0.01, 0.0] }]);
        assert_eq!((r.applied, r.stale.len()), (1, 0));
        let moved: Vec<_> = v.iter().filter(|x| x.pos[1] != 0.0).collect();
        assert_eq!(moved.len(), 2, "both triangles' copies of the corner");
        assert!(moved.iter().all(|x| x.pos == [0.1, 0.01, 0.1]));
        // the raised corner tilts the surface: the normals near it lean away from +Y and stay unit
        assert!(v.iter().all(|x| (Vec3::from(x.normal).length() - 1.0).abs() < 1e-5));
        assert!(v.iter().any(|x| x.normal[0] < -1e-3 || x.normal[2] < -1e-3));
    }

    #[test]
    fn nearest_point_wins_and_far_fixes_are_stale() {
        let n = [0.0, 1.0, 0.0];
        // two points 1 mm apart, a third triangle corner far away
        let mut v = vec![vx([0.0, 0.0, 0.0], n), vx([0.001, 0.0, 0.0], n), vx([0.0, 0.0, 0.1], n)];
        let r = apply(&mut v, &[Fix { at: [0.0009, 0.0, 0.0], by: [0.0, 0.5, 0.0] }, Fix { at: [0.0045, 0.0, 0.0], by: [0.0, 0.5, 0.0] }]);
        assert_eq!((r.applied, r.stale.len()), (1, 1));
        assert_eq!(v[0].pos, [0.0, 0.0, 0.0]);
        assert_eq!(v[1].pos, [0.001, 0.5, 0.0], "the nearer of the two");
        assert_eq!(r.summary(), "fixes 1 of 2, 1 stale");
    }

    #[test]
    fn a_crease_keeps_its_own_normal() {
        // two corners at one position with different normals (a crease): only the one whose
        // triangle changed shape tilts
        let up = [0.0, 1.0, 0.0];
        let side = [1.0, 0.0, 0.0];
        let mut v = vec![
            vx([0.0, 0.0, 0.0], up), vx([0.0, 0.0, 0.1], up), vx([0.1, 0.0, 0.0], up),
            vx([0.0, 0.0, 0.0], side), vx([0.0, -0.1, 0.0], side), vx([0.0, 0.0, -0.1], side),
        ];
        // raise the far corner of the first triangle: its normal changes, the second triangle is untouched
        apply(&mut v, &[Fix { at: [0.1, 0.0, 0.0], by: [0.0, 0.02, 0.0] }]);
        assert!(v[0].normal != up, "the moved triangle's corner tilts");
        assert_eq!(v[3].normal, side, "the crease's other side keeps its normal");
    }
}
