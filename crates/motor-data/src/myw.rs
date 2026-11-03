//! MYW world (track) meshes ("!WYM"): a grid of cells, each holding one serialized
//! MYO-style object. Loader 0x47ad84.

use crate::bytes::{err, f32_, f64_, i32_, slice, u16_, u32_, u8_, vec3, vec4, Result};
use crate::myo::{parse_halos, parse_polygons, read_names, Halo, Polygon};

/// Section order of the 12 (count, bytes, offset) triples at header dword 23.
pub const SECTION_NAMES: [&str; 12] = [
    "objects", "vertices", "objinfo", "polygons", "planes", "vnormals",
    "texrefs", "uvbytes", "instances", "celllinks", "texnames", "halos",
];

#[derive(Debug, Clone, Copy)]
pub struct Section {
    pub count: usize,
    pub bytes: usize,
    pub offset: usize,
}

#[derive(Debug, Clone)]
pub struct Cell {
    /// offset relative to the objects section, -1 = empty
    pub obj: i32,
    /// offset relative to celllinks, -1 = none
    pub links: i32,
    pub n_anim: u32,
    pub anim: i32,
}

#[derive(Debug, Clone)]
pub struct Object {
    pub offset: usize,
    pub center: [f32; 3],
    pub radius: f32,
    pub vertices: Vec<[f32; 3]>,
    pub planes: Vec<[f32; 4]>,
    pub vnormals: Vec<[f32; 3]>,
    pub uvbytes: Vec<u8>,
    pub polygons: Vec<Polygon>,
}

#[derive(Debug, Clone)]
pub struct World {
    pub grid_w: usize,
    pub grid_h: usize,
    pub cell_size: [f64; 2],
    pub bbox_min: [f32; 3],
    pub bbox_max: [f32; 3],
    pub sections: Vec<Section>,
    pub cells: Vec<Cell>,
    /// one per non-empty cell, in cell order
    pub objects: Vec<Object>,
    pub texrefs: Vec<u32>,
    pub texnames: Vec<String>,
    pub light: Light,
    /// light halos (section 11; cells reference them through n_anim/anim)
    pub halos: Vec<Halo>,
    /// point lights (section 8, "instances")
    pub lights: Vec<PointLight>,
    /// per cell (row-major, `z * grid_w + x`): indices into `lights`, from the cell's
    /// `link_rel` list in celllinks (section 9)
    pub cell_lights: Vec<Vec<u16>>,
}

/// A point light of section 8 (196 B records); `k = (1 - d/range)^2 * LightIntensity *
/// intensity` per lit object (0x4353cc, see 05-formats).
#[derive(Debug, Clone, Copy)]
pub struct PointLight {
    /// +4
    pub pos: [f32; 3],
    /// +0x1c, 0..1
    pub rgb: [f32; 3],
    /// +0x28, can be negative
    pub intensity: f32,
    /// +0x2c, metres
    pub range: f32,
}

/// Scene lighting from the header, copied by the loader 0x47ad84 into the world struct
/// (+0x28..+0x40) and read by the per-frame light setup 0x439e84.
#[derive(Debug, Clone, Copy)]
pub struct Light {
    /// 0x38 rgb, 0x3c f64 intensity
    pub ambient: [u8; 3],
    pub ambient_intensity: f32,
    /// 0x44 rgb, 0x48 f32 intensity, 0x4c direction
    pub sun: [u8; 3],
    pub sun_intensity: f32,
    pub sun_dir: [f32; 3],
}

impl World {
    pub fn section(&self, name: &str) -> Section {
        self.sections[SECTION_NAMES.iter().position(|n| *n == name).expect("section name")]
    }

    /// Texture name of a textured polygon (texref offsets are global in MYW).
    pub fn texture_name(&self, p: &Polygon) -> Option<&str> {
        let ti = *self.texrefs.get(p.texref_off? as usize / 12)? as usize;
        self.texnames.get(ti).map(String::as_str)
    }
}

fn parse_object(d: &[u8], s: &[Section], rel: usize) -> Result<Object> {
    let o = s[0].offset + rel;
    slice(d, o, 0xe4)?;
    let n_verts = u16_(d, o + 0x80)? as usize;
    let n_vnormals = u16_(d, o + 0x82)? as usize;
    let n_planes = u16_(d, o + 0x84)? as usize;
    let n_polys = u16_(d, o + 0x86)? as usize;
    let n_uv = u32_(d, o + 0x88)? as usize;
    let at = |field: usize, sec: usize| -> Result<usize> { Ok(s[sec].offset + u32_(d, o + field)? as usize) };
    let (vo, po, no, uo, yo) = (at(0x8c, 1)?, at(0x90, 4)?, at(0x94, 5)?, at(0x98, 7)?, at(0x9c, 3)?);
    Ok(Object {
        offset: o,
        center: vec3(d, o + 0x70)?,
        radius: crate::bytes::f32_(d, o + 0x7c)?,
        vertices: (0..n_verts).map(|i| vec3(d, vo + 12 * i)).collect::<Result<_>>()?,
        planes: (0..n_planes).map(|i| vec4(d, po + 16 * i)).collect::<Result<_>>()?,
        vnormals: (0..n_vnormals).map(|i| vec3(d, no + 12 * i)).collect::<Result<_>>()?,
        uvbytes: slice(d, uo, n_uv)?.to_vec(),
        polygons: parse_polygons(d, yo, n_polys, None)?,
    })
}

pub fn parse(d: &[u8]) -> Result<World> {
    if d.get(..4) != Some(b"!WYM") {
        return err("not a MYW world");
    }
    let h = |i: usize| u32_(d, 4 * i).map(|v| v as usize);
    let sections = (0..12)
        .map(|k| Ok(Section { count: h(23 + 3 * k)?, bytes: h(24 + 3 * k)?, offset: h(25 + 3 * k)? }))
        .collect::<Result<Vec<_>>>()?;
    let (gw, gh, cells_off) = (h(2)?, h(3)?, h(22)?);
    let mut cells = Vec::with_capacity(gw * gh);
    for i in 0..gw * gh {
        let c = cells_off + 24 * i;
        cells.push(Cell { obj: i32_(d, c)?, links: i32_(d, c + 8)?, n_anim: u32_(d, c + 16)?, anim: i32_(d, c + 20)? });
    }
    let objects = cells
        .iter()
        .filter(|c| c.obj != -1)
        .map(|c| parse_object(d, &sections, c.obj as usize))
        .collect::<Result<_>>()?;
    let tr = sections[6];
    let texrefs = (0..tr.count).map(|i| u32_(d, tr.offset + 12 * i)).collect::<Result<_>>()?;
    let (inst, links) = (sections[8], sections[9]);
    let lights = (0..inst.count)
        .map(|i| {
            let r = inst.offset + 196 * i;
            Ok(PointLight { pos: vec3(d, r + 4)?, rgb: vec3(d, r + 0x1c)?, intensity: f32_(d, r + 0x28)?, range: f32_(d, r + 0x2c)? })
        })
        .collect::<Result<Vec<_>>>()?;
    // the cell's list: {u32 instance_rel, i32 next_rel} nodes, instance_rel in bytes
    let cell_lights = cells
        .iter()
        .map(|c| {
            let (mut out, mut at) = (Vec::new(), c.links);
            while at >= 0 && out.len() < 64 {
                let n = links.offset + at as usize;
                out.push((u32_(d, n)? / 196) as u16);
                at = i32_(d, n + 4)?;
            }
            Ok(out)
        })
        .collect::<Result<Vec<_>>>()?;
    let halos = parse_halos(d, sections[11].offset, sections[11].count)?;
    let tn = sections[10];
    Ok(World {
        grid_w: gw,
        grid_h: gh,
        cell_size: [f64_(d, 16)?, f64_(d, 24)?],
        bbox_min: vec3(d, 32)?,
        bbox_max: vec3(d, 44)?,
        sections,
        cells,
        objects,
        texrefs,
        halos,
        lights,
        cell_lights,
        light: Light {
            ambient: [u8_(d, 0x38)?, u8_(d, 0x39)?, u8_(d, 0x3a)?],
            ambient_intensity: f64_(d, 0x3c)? as f32,
            sun: [u8_(d, 0x44)?, u8_(d, 0x45)?, u8_(d, 0x46)?],
            sun_intensity: f32_(d, 0x48)?,
            sun_dir: vec3(d, 0x4c)?,
        },
        texnames: read_names(d, tn.offset, tn.count)?,
    })
}
