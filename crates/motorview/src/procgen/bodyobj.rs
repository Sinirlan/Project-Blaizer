//! REMASTER (#292): a generated car body as Wavefront OBJ, for hand fixes in Blender.
//!
//! Frame [design choice]: the renderer's car-local view space (game x, y, -z), metres. It is
//! right-handed, so Blender's default OBJ import (forward -Z, up Y) shows the car unmirrored
//! with its nose along +Y: Blender X/Y/Z = game x/z/y. The triangles keep the renderer's
//! winding. Positions that are bit-identical share one `v` (the body's `merge_close` has already
//! snapped its points within 0.5 mm), so a point moved in Blender keeps the mesh closed. No
//! normals or uvs: Blender's own normals follow an edit, custom ones would not.
//!
//! One object, one `g` + `usemtl` per part (`scene::BodyParts`), so Blender keeps a single mesh
//! with a material slot per part.

use crate::scene::{self, BodyParts, Vertex};
use std::collections::HashMap;
use std::fmt::Write;
use std::path::Path;

/// Car `car`'s generated body (`scene::export_body`, `track`'s textures) written to `out` as OBJ
/// (folders made as needed); the summary line, e.g. `car09 'R-Build 2': 41234 tris, ...`.
pub fn export(root: &Path, track: u32, car: u32, out: &Path) -> Result<String, String> {
    let b = scene::export_body(root, track, car)?;
    let name = format!("car{car:02}");
    let (text, st) = to_obj(&name, &format!("{name} '{}': generated body (Remaster)", b.name), &b.verts, &b.parts);
    if let Some(d) = out.parent().filter(|d| !d.as_os_str().is_empty()) {
        std::fs::create_dir_all(d).map_err(|e| format!("{}: {e}", d.display()))?;
    }
    std::fs::write(out, text).map_err(|e| format!("{}: {e}", out.display()))?;
    let skipped = if st.skipped > 0 { format!(" ({} degenerate left out)", st.skipped) } else { String::new() };
    Ok(format!("{name} '{}': {} tris, {} points, {} parts{skipped} -> {}", b.name, st.tris, st.points, st.groups, out.display()))
}

/// What `to_obj` wrote.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Stats {
    pub tris: usize,
    pub points: usize,
    pub groups: usize,
    /// triangles with two corners on one welded point, left out
    pub skipped: usize,
}

/// The OBJ text of triangle list `verts` (car-local view space) with its `parts`, object `name`
/// (e.g. `car09`); `title` goes into the header comment.
pub fn to_obj(name: &str, title: &str, verts: &[Vertex], parts: &BodyParts) -> (String, Stats) {
    let mut st = Stats::default();
    let mut index: HashMap<[u32; 3], usize> = HashMap::new();
    let mut points: Vec<[f32; 3]> = Vec::new();
    // per part, its triangles as 1-based point indices
    let mut faces: Vec<Vec<[usize; 3]>> = vec![Vec::new(); parts.names.len().max(1)];
    for (i, t) in verts.as_chunks::<3>().0.iter().enumerate() {
        let ids = [0, 1, 2].map(|k| {
            let p = t[k].pos;
            *index.entry(p.map(f32::to_bits)).or_insert_with(|| {
                points.push(p);
                points.len()
            })
        });
        if ids[0] == ids[1] || ids[1] == ids[2] || ids[2] == ids[0] {
            st.skipped += 1;
            continue;
        }
        let g = parts.tri.get(i).map_or(0, |&g| g as usize).min(faces.len() - 1);
        faces[g].push(ids);
    }
    let mut s = String::new();
    let _ = writeln!(s, "# {title}");
    let _ = writeln!(s, "# motorview {} --export-body (#292)", motor_version::VERSION);
    let _ = writeln!(s, "# frame: car-local view space = game x, y, -z (metres); Blender X/Y/Z = game x/z/y after the default import");
    let _ = writeln!(s, "o {name}");
    for p in &points {
        let _ = writeln!(s, "v {:.6} {:.6} {:.6}", p[0], p[1], p[2]);
    }
    for (g, tris) in faces.iter().enumerate() {
        if tris.is_empty() {
            continue;
        }
        let part = parts.names.get(g).map_or("body", String::as_str);
        let _ = writeln!(s, "g {part}");
        let _ = writeln!(s, "usemtl {part}");
        for t in tris {
            let _ = writeln!(s, "f {} {} {}", t[0], t[1], t[2]);
        }
        st.groups += 1;
        st.tris += tris.len();
    }
    st.points = points.len();
    (s, st)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(p: [f32; 3]) -> Vertex {
        Vertex { pos: p, uv: [0.0; 2], color: [1.0; 4], layer: -1.0, normal: [0.0, 0.0, 1.0], env: 1.0 }
    }

    /// two triangles on a shared edge weld to 4 points, each in its part, the positions as given
    /// (view space) and 1-based; a triangle with a repeated corner is left out
    #[test]
    fn welds_groups_and_skips() {
        let (a, b, c, d) = ([0.0, 0.0, -1.0], [1.0, 0.0, -1.0], [1.0, 1.0, -1.0], [0.0, 1.0, -1.0]);
        let verts = [a, b, c, a, c, d, a, a, b].map(v);
        let parts = BodyParts { names: vec!["paint_200_0_0".into(), "glass_55_72_89".into()], tri: vec![0, 1, 1] };
        let (s, st) = to_obj("car09", "test", &verts, &parts);
        assert_eq!(st, Stats { tris: 2, points: 4, groups: 2, skipped: 1 });
        let lines: Vec<&str> = s.lines().filter(|l| !l.starts_with('#')).collect();
        assert_eq!(
            lines,
            [
                "o car09",
                "v 0.000000 0.000000 -1.000000",
                "v 1.000000 0.000000 -1.000000",
                "v 1.000000 1.000000 -1.000000",
                "v 0.000000 1.000000 -1.000000",
                "g paint_200_0_0",
                "usemtl paint_200_0_0",
                "f 1 2 3",
                "g glass_55_72_89",
                "usemtl glass_55_72_89",
                "f 1 3 4",
            ]
        );
    }
}
