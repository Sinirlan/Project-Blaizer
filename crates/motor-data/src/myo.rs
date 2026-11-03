//! MYO object meshes ("!OYM" v1 / "2OYM" v2). Loader 0x47aa08, polygon walk 0x47a89c.
//! Polygon records are shared with MYW worlds.

use crate::bytes::{cstr, err, f32_, slice, u16_, u32_, u8_, vec3, vec4, Result};

/// Per-type record layout (size from the loader; fields derived from data statistics).
struct Layout {
    size: usize,
    ncol: usize,
    nv: usize,
    normal: bool,
    nvn: usize,
    uv_tex: Option<(usize, usize)>,
}

fn layout(t: u8) -> Option<Layout> {
    let l = |size, ncol, nv, normal, nvn, uv_tex| Some(Layout { size, ncol, nv, normal, nvn, uv_tex });
    match t {
        0 => l(16, 1, 3, true, 0, None),           // flat tri
        1 => l(20, 1, 4, true, 0, None),           // flat quad
        2 => l(24, 1, 3, true, 0, Some((18, 20))), // textured tri
        3 => l(24, 1, 4, true, 0, Some((18, 20))), // textured quad
        4 => l(24, 3, 3, true, 0, None),           // vertex-colour tri
        5 => l(32, 4, 4, true, 0, None),           // vertex-colour quad
        6 => l(32, 3, 3, true, 0, Some((26, 28))), // vcol textured tri
        7 => l(36, 4, 4, true, 0, Some((30, 32))), // vcol textured quad
        8 => l(20, 1, 3, false, 3, None),          // gouraud tri
        9 => l(24, 1, 4, false, 4, None),          // gouraud quad
        _ => None,
    }
}

#[derive(Debug, Clone)]
pub struct Polygon {
    pub ty: u8,
    pub flags: u8,
    /// index into planes
    pub plane: u16,
    /// one face colour, or one per vertex (types 4-7)
    pub rgba: Vec<[u8; 4]>,
    pub v: Vec<u16>,
    /// flat-shading normal (index into vertex normals)
    pub normal: Option<u16>,
    /// per-vertex normals (gouraud types)
    pub vn: Vec<u16>,
    /// byte offset into uv bytes (2 bytes per vertex)
    pub uv_off: Option<u16>,
    /// byte offset into texture refs (12-byte records)
    pub texref_off: Option<u32>,
}

pub fn parse_polygons(d: &[u8], off: usize, count: usize, size: Option<usize>) -> Result<Vec<Polygon>> {
    let mut polys = Vec::with_capacity(count);
    let mut p = off;
    for _ in 0..count {
        let ty = crate::bytes::u8_(d, p)?;
        let Some(l) = layout(ty) else {
            return err(format!("unknown polygon type {ty} at {p:#x}"));
        };
        slice(d, p, l.size)?;
        let mut q = p + 4;
        let rgba = (0..l.ncol).map(|i| d[q + 4 * i..q + 4 * i + 4].try_into().unwrap()).collect();
        q += 4 * l.ncol;
        let v = (0..l.nv).map(|i| u16_(d, q + 2 * i)).collect::<Result<Vec<_>>>()?;
        q += 2 * l.nv;
        let normal = if l.normal {
            let n = u16_(d, q)?;
            q += 2;
            Some(n)
        } else {
            None
        };
        let vn = (0..l.nvn).map(|i| u16_(d, q + 2 * i)).collect::<Result<Vec<_>>>()?;
        let (uv_off, texref_off) = match l.uv_tex {
            Some((u, t)) => (Some(u16_(d, p + u)?), Some(u32_(d, p + t)?)),
            None => (None, None),
        };
        polys.push(Polygon { ty, flags: d[p + 1], plane: u16_(d, p + 2)?, rgba, v, normal, vn, uv_off, texref_off });
        p += l.size;
    }
    if let Some(sz) = size {
        if p - off != sz {
            return err(format!("polygon section size mismatch {} != {sz}", p - off));
        }
    }
    Ok(polys)
}

#[derive(Debug, Clone)]
pub struct Mesh {
    pub version: u8,
    pub vertices: Vec<[f32; 3]>,
    pub polygons: Vec<Polygon>,
    pub planes: Vec<[f32; 4]>,
    pub vnormals: Vec<[f32; 3]>,
    /// texture-name index per texref record
    pub texrefs: Vec<u32>,
    /// u8 (u, v) texel coordinates
    pub uvbytes: Vec<u8>,
    pub texnames: Vec<String>,
    pub radius: f32,
    pub center: [f32; 3],
    /// light halos (v2 header dwords 23/25; loader 0x47aa08 -> object +0xa0/+0xa4)
    pub halos: Vec<Halo>,
}

/// Light halo record, 120 bytes (drawn by 0x4354fc). Bytes past 0x38 are runtime fill.
#[derive(Debug, Clone, Copy)]
pub struct Halo {
    /// bit 1 draw; bit 3 directional; bits 4/5 select the distance curve
    pub flags: u32,
    /// index into the owner's `Halo` texture list (.car or .trk)
    pub texture: u32,
    /// segment endpoints; the halo is drawn at the midpoint, stretched along the segment
    pub p0: [f32; 3],
    pub p1: [f32; 3],
    pub rgb: [u8; 3],
}

pub const HALO_SIZE: usize = 120;

pub(crate) fn parse_halos(d: &[u8], off: usize, n: usize) -> Result<Vec<Halo>> {
    (0..n)
        .map(|i| {
            let o = off + HALO_SIZE * i;
            Ok(Halo {
                flags: u32_(d, o)?,
                texture: u32_(d, o + 4)?,
                p0: vec3(d, o + 8)?,
                p1: vec3(d, o + 0x14)?,
                rgb: [u8_(d, o + 0x30)?, u8_(d, o + 0x31)?, u8_(d, o + 0x32)?],
            })
        })
        .collect()
}

impl Mesh {
    /// Texture name and texel UVs (+0.5 like the loader) of a textured polygon.
    pub fn texture(&self, p: &Polygon) -> Option<(&str, Vec<[f32; 2]>)> {
        let ti = *self.texrefs.get(p.texref_off? as usize / 12)? as usize;
        let name = self.texnames.get(ti)?;
        let uo = p.uv_off? as usize;
        let uv = (0..p.v.len())
            .map(|k| Some([*self.uvbytes.get(uo + 2 * k)? as f32 + 0.5, *self.uvbytes.get(uo + 2 * k + 1)? as f32 + 0.5]))
            .collect::<Option<Vec<_>>>()?;
        Some((name, uv))
    }
}

pub(crate) fn read_names(d: &[u8], mut p: usize, n: usize) -> Result<Vec<String>> {
    let mut names = Vec::with_capacity(n);
    for _ in 0..n {
        let s = cstr(d, p, None)?;
        p += s.len() + 1;
        names.push(s);
    }
    Ok(names)
}

pub fn parse(d: &[u8]) -> Result<Mesh> {
    let version = match d.get(..4) {
        Some(b"!OYM") => 1,
        Some(b"2OYM") => 2,
        _ => return err("not a MYO mesh"),
    };
    let h = |i: usize| u32_(d, 4 * i).map(|v| v as usize);
    let sec = |i: usize| -> Result<(usize, usize, usize)> { Ok((h(1 + 3 * i)?, h(2 + 3 * i)?, h(3 + 3 * i)?)) };
    let (n, _, o) = sec(0)?;
    let vertices = (0..n).map(|i| vec3(d, o + 12 * i)).collect::<Result<_>>()?;
    let (n, sz, o) = sec(1)?;
    let polygons = parse_polygons(d, o, n, Some(sz))?;
    let (n, _, o) = sec(2)?;
    let planes = (0..n).map(|i| vec4(d, o + 16 * i)).collect::<Result<_>>()?;
    let (n, _, o) = sec(3)?;
    let vnormals = (0..n).map(|i| vec3(d, o + 12 * i)).collect::<Result<_>>()?;
    let (n, _, o) = sec(4)?;
    let texrefs = (0..n).map(|i| u32_(d, o + 12 * i)).collect::<Result<_>>()?;
    let (_, sz, o) = sec(5)?;
    let uvbytes = slice(d, o, sz)?.to_vec();
    let (n, _, o) = sec(6)?;
    let texnames = read_names(d, o, n)?;
    let halos = if version == 2 { parse_halos(d, h(25)?, h(23)?)? } else { Vec::new() };
    Ok(Mesh {
        version,
        vertices,
        polygons,
        planes,
        vnormals,
        texrefs,
        uvbytes,
        texnames,
        radius: f32_(d, 26 * 4)?,
        center: vec3(d, 27 * 4)?,
        halos,
    })
}
