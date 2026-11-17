//! REMASTER (#292): a generated car body as Wavefront OBJ, for hand fixes in Blender.
//!
//! Frame [design choice]: the renderer's car-local view space (game x, y, -z), metres. It is
//! right-handed, so Blender's default OBJ import (forward -Z, up Y) shows the car unmirrored
//! with its nose along +Y: Blender X/Y/Z = game x/z/y. The triangles keep the renderer's
//! winding. Positions that are bit-identical share one `v` (the body's `merge_close` has already
//! snapped its points within 0.5 mm), so a point moved in Blender keeps the mesh closed. No
//! normals: Blender's own normals follow an edit, custom ones would not.
//!
//! One object, one `g` per part (`scene::BodyParts`), so Blender keeps a single mesh with a
//! material slot per material.
//!
//! #540: the materials go into a `.mtl` beside the `.obj` (`mtllib`), the textures the body uses
//! into `<stem>_texK.png`. A material is one (part, colour, texture layer): a part with one is
//! named as the part, a part with several `part`, `part.2`, ... [design choice]. `Kd` = the
//! vertex colour made linear (Blender reads `Kd` as linear, like the blueprint's
//! `baseColorFactor`), `d` = its alpha when below 1. A textured material is `map_Kd` with `Kd`
//! white: the renderer draws a textured car face from the texel and a fixed base, not the vertex
//! colour (shader.wgsl `D3D_TEX_FACE`); `map_d` too when the texture has see-through texels (the
//! renderer drops alpha < 0.5). Only textured triangles get `vt` (v flipped: OBJ's origin is
//! bottom-left), so the weld is unchanged.

use super::carmodel::srgb_to_linear;
use crate::scene::{self, BodyParts, Vertex};
use std::collections::HashMap;
use std::fmt::Write;
use std::path::Path;

/// Car `car`'s generated body (`scene::export_body`, `track`'s textures) written to `out` as OBJ
/// with its `.mtl` and textures beside it (folders made as needed); the summary line, e.g.
/// `car09 'R-Build 2': 41234 tris, ...`.
pub fn export(root: &Path, track: u32, car: u32, out: &Path) -> Result<String, String> {
    let b = scene::export_body(root, track, car)?;
    let name = format!("car{car:02}");
    let stem = out.file_stem().map_or_else(|| name.clone(), |s| s.to_string_lossy().into_owned());
    let title = format!("{name} '{}': generated body (Remaster)", b.name);
    let (obj, st) = to_obj(&name, &stem, &title, &b.verts, &b.parts, |l| b.layers.get(l).is_some_and(|t| see_through(t)));
    let dir = out.parent().filter(|d| !d.as_os_str().is_empty());
    if let Some(d) = dir {
        std::fs::create_dir_all(d).map_err(|e| format!("{}: {e}", d.display()))?;
    }
    let beside = |f: &str| dir.map_or_else(|| f.into(), |d| d.join(f));
    let write = |p: &Path, bytes: &[u8]| std::fs::write(p, bytes).map_err(|e| format!("{}: {e}", p.display()));
    write(out, obj.obj.as_bytes())?;
    write(&beside(&format!("{stem}.mtl")), obj.mtl.as_bytes())?;
    for (k, &l) in obj.layers.iter().enumerate() {
        let rgba = b.layers.get(l).ok_or_else(|| format!("texture layer {l} missing"))?;
        write(&beside(&tex_file(&stem, k)), &super::blueprint::png(rgba, scene::TEX))?;
    }
    let skipped = if st.skipped > 0 { format!(" ({} degenerate left out)", st.skipped) } else { String::new() };
    Ok(format!(
        "{name} '{}': {} tris, {} points, {} parts, {} materials, {} textures{skipped} -> {}",
        b.name,
        st.tris,
        st.points,
        st.groups,
        st.materials,
        st.textures,
        out.display()
    ))
}

/// the file name of texture `k` of the export `stem`
fn tex_file(stem: &str, k: usize) -> String {
    format!("{stem}_tex{k}.png")
}

/// What `to_obj` wrote.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Stats {
    pub tris: usize,
    pub points: usize,
    pub groups: usize,
    /// #540: `newmtl` entries
    pub materials: usize,
    /// #540: texture files (`map_Kd`)
    pub textures: usize,
    /// triangles with two corners on one welded point, left out
    pub skipped: usize,
}

/// The files of one export.
pub struct Obj {
    pub obj: String,
    pub mtl: String,
    /// the texture layers `<stem>_texK.png` hold, by K
    pub layers: Vec<usize>,
}

/// whether texture layer `rgba` has texels the renderer drops (alpha < 0.5)
fn see_through(rgba: &[u8]) -> bool {
    rgba.as_chunks::<4>().0.iter().any(|p| p[3] < 128)
}

/// A material: colour bytes RGBA and texture layer (-1 = none).
type Key = ([u8; 4], i32);

/// A part's material: its key, the colour of its first corner, its triangles as 1-based (point,
/// uv) indices (uv 0 = none).
type Mat = (Key, [f32; 4], Vec<[(usize, usize); 3]>);

/// The OBJ and MTL text of triangle list `verts` (car-local view space) with its `parts`, object
/// `name` (e.g. `car09`); `stem` names the `.mtl` and texture files, `title` goes into the headers;
/// `cutout(layer)`: the layer has see-through texels.
pub fn to_obj(name: &str, stem: &str, title: &str, verts: &[Vertex], parts: &BodyParts, cutout: impl Fn(usize) -> bool) -> (Obj, Stats) {
    let mut st = Stats::default();
    let mut index: HashMap<[u32; 3], usize> = HashMap::new();
    let mut points: Vec<[f32; 3]> = Vec::new();
    let mut uv_index: HashMap<[u32; 2], usize> = HashMap::new();
    let mut uvs: Vec<[f32; 2]> = Vec::new();
    // per part, its materials in first-use order
    let mut groups: Vec<Vec<Mat>> = vec![Vec::new(); parts.names.len().max(1)];
    let mut layers: Vec<usize> = Vec::new();
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
        let layer = t[0].layer as i32;
        let uv = [0, 1, 2].map(|k| {
            if layer < 0 {
                return 0;
            }
            let q = [t[k].uv[0], 1.0 - t[k].uv[1]];
            *uv_index.entry(q.map(f32::to_bits)).or_insert_with(|| {
                uvs.push(q);
                uvs.len()
            })
        });
        if layer >= 0 && !layers.contains(&(layer as usize)) {
            layers.push(layer as usize);
        }
        // a textured face: the texel alone (see the module doc)
        let c = if layer >= 0 { [1.0; 4] } else { t[0].color };
        let key: Key = (c.map(|x| (x.clamp(0.0, 1.0) * 255.0).round() as u8), layer);
        let g = parts.tri.get(i).map_or(0, |&g| g as usize).min(groups.len() - 1);
        let tri = [0, 1, 2].map(|k| (ids[k], uv[k]));
        match groups[g].iter_mut().find(|m| m.0 == key) {
            Some(m) => m.2.push(tri),
            None => groups[g].push((key, c, vec![tri])),
        }
    }
    let (mut s, mut m) = (String::new(), String::new());
    let _ = writeln!(s, "# {title}");
    let _ = writeln!(s, "# motorview {} --export-body (#292, materials #540)", motor_version::VERSION);
    let _ = writeln!(s, "# frame: car-local view space = game x, y, -z (metres); Blender X/Y/Z = game x/z/y after the default import");
    let _ = writeln!(s, "mtllib {stem}.mtl");
    let _ = writeln!(m, "# {title}: materials of {stem}.obj (#540)");
    let _ = writeln!(s, "o {name}");
    for p in &points {
        let _ = writeln!(s, "v {:.6} {:.6} {:.6}", p[0], p[1], p[2]);
    }
    for q in &uvs {
        let _ = writeln!(s, "vt {:.6} {:.6}", q[0], q[1]);
    }
    for (g, mats) in groups.iter().enumerate() {
        if mats.is_empty() {
            continue;
        }
        let part = parts.names.get(g).map_or("body", String::as_str);
        let _ = writeln!(s, "g {part}");
        for (k, ((_, layer), colour, tris)) in mats.iter().enumerate() {
            let mat = if k == 0 { part.to_string() } else { format!("{part}.{}", k + 1) };
            let _ = writeln!(s, "usemtl {mat}");
            for t in tris {
                let c = t.map(|(v, vt)| if vt > 0 { format!("{v}/{vt}") } else { v.to_string() });
                let _ = writeln!(s, "f {} {} {}", c[0], c[1], c[2]);
            }
            st.tris += tris.len();
            let kd = [0, 1, 2].map(|i| srgb_to_linear(colour[i]));
            let _ = writeln!(m, "\nnewmtl {mat}");
            let _ = writeln!(m, "Kd {:.6} {:.6} {:.6}", kd[0], kd[1], kd[2]);
            if colour[3] < 1.0 {
                let _ = writeln!(m, "d {:.6}", colour[3].max(0.0));
            }
            if *layer >= 0 {
                let t = layers.iter().position(|&l| l == *layer as usize).unwrap_or(0);
                let _ = writeln!(m, "map_Kd {}", tex_file(stem, t));
                if cutout(*layer as usize) {
                    let _ = writeln!(m, "map_d {}", tex_file(stem, t));
                }
            }
            st.materials += 1;
        }
        st.groups += 1;
    }
    st.points = points.len();
    st.textures = layers.len();
    (Obj { obj: s, mtl: m, layers }, st)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(p: [f32; 3]) -> Vertex {
        Vertex { pos: p, uv: [0.0; 2], color: [1.0; 4], layer: -1.0, normal: [0.0, 0.0, 1.0], env: 1.0 }
    }

    fn body(s: &str) -> Vec<&str> {
        s.lines().filter(|l| !l.starts_with('#') && !l.is_empty()).collect()
    }

    /// two triangles on a shared edge weld to 4 points, each in its part, the positions as given
    /// (view space) and 1-based; a triangle with a repeated corner is left out; each part's
    /// material in the `.mtl`, white `Kd`, no texture
    #[test]
    fn welds_groups_and_skips() {
        let (a, b, c, d) = ([0.0, 0.0, -1.0], [1.0, 0.0, -1.0], [1.0, 1.0, -1.0], [0.0, 1.0, -1.0]);
        let verts = [a, b, c, a, c, d, a, a, b].map(v);
        let parts = BodyParts { names: vec!["paint_200_0_0".into(), "glass_55_72_89".into()], tri: vec![0, 1, 1] };
        let (o, st) = to_obj("car09", "car09", "test", &verts, &parts, |_| false);
        assert_eq!(st, Stats { tris: 2, points: 4, groups: 2, materials: 2, textures: 0, skipped: 1 });
        assert_eq!(
            body(&o.obj),
            [
                "mtllib car09.mtl",
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
        assert_eq!(body(&o.mtl), ["newmtl paint_200_0_0", "Kd 1.000000 1.000000 1.000000", "newmtl glass_55_72_89", "Kd 1.000000 1.000000 1.000000"]);
        assert!(o.layers.is_empty());
    }

    /// a part with two colours and a textured triangle: three materials (`lamps`, `lamps.2`,
    /// `lamps.3`), the textured one white with `vt` (v flipped), `map_Kd` and `map_d` (its vertex
    /// colour left out), see-through alpha as `d`; the plain triangles keep `f v`
    #[test]
    fn colours_textures_and_split_parts() {
        let (a, b, c, d) = ([0.0, 0.0, -1.0], [1.0, 0.0, -1.0], [1.0, 1.0, -1.0], [0.0, 1.0, -1.0]);
        let mut verts = [a, b, c, a, c, d, b, c, d].map(v).to_vec();
        for x in &mut verts[3..6] {
            x.color = [0.0, 0.0, 0.0, 0.5];
        }
        for (x, uv) in verts[6..9].iter_mut().zip([[0.0, 0.0], [1.0, 0.0], [1.0, 0.25]]) {
            x.layer = 7.0;
            x.color = [0.0, 0.0, 0.0, 1.0];
            x.uv = uv;
        }
        let parts = BodyParts { names: vec!["lamps".into()], tri: vec![0, 0, 0] };
        let (o, st) = to_obj("car01", "out", "test", &verts, &parts, |l| l == 7);
        assert_eq!(st, Stats { tris: 3, points: 4, groups: 1, materials: 3, textures: 1, skipped: 0 });
        let obj = body(&o.obj);
        assert_eq!(obj[0], "mtllib out.mtl");
        assert_eq!(
            obj[6..],
            [
                "vt 0.000000 1.000000",
                "vt 1.000000 1.000000",
                "vt 1.000000 0.750000",
                "g lamps",
                "usemtl lamps",
                "f 1 2 3",
                "usemtl lamps.2",
                "f 1 3 4",
                "usemtl lamps.3",
                "f 2/1 3/2 4/3",
            ]
        );
        assert_eq!(
            body(&o.mtl),
            [
                "newmtl lamps",
                "Kd 1.000000 1.000000 1.000000",
                "newmtl lamps.2",
                "Kd 0.000000 0.000000 0.000000",
                "d 0.500000",
                "newmtl lamps.3",
                "Kd 1.000000 1.000000 1.000000",
                "map_Kd out_tex0.png",
                "map_d out_tex0.png",
            ]
        );
        assert_eq!(o.layers, [7]);
    }
}
