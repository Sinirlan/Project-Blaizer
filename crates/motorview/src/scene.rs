//! Turn a track (and one car) from a user's install into GPU-ready vertex data.
//! Coordinates: the game is left-handed (x right, y up, z forward); we negate z.

use glam::Vec3;
use crate::loading::{Progress, Stage};
use crate::{procgen, texpack};
use motor_data::{aidat, cfg::Config, iff, myo, myw, pdi, tga};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

pub const TEX: u32 = 256; // every texture is resampled into a 256x256 array layer
/// 256 was enough before the scene objects; Olympos with a full grid needs a little more
pub const MAX_LAYERS: usize = 512;
/// REMASTER texture pack: layers of its array (colour and normal+roughness maps together)
pub const MAX_PACK_LAYERS: usize = 192;

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct Vertex {
    pub pos: [f32; 3],
    pub uv: [f32; 2],
    pub color: [f32; 4],
    /// texture array layer, or -1 for untextured
    pub layer: f32,
    /// vertex normal (view space) for lit objects; the plane normal for pre-lit world geometry
    /// (only the dynamic lights use it there)
    pub normal: [f32; 3],
    /// 1 = gets the environment-map reflection (car body paint)
    pub env: f32,
}

/// REMASTER (09-remaster 9.14): `Vertex::env` of a retro-reflective road sign face
pub const RETRO_ENV: f32 = 8.0;
/// REMASTER (#136, 09-remaster 9.40/9.41): `Vertex::env` of a car body face painted in a Metallic zone.
/// Inside the paint band (0.5..2.5): Classic `vs_main` draws it as plain paint (1); `vs_remaster`
/// hands it to the paint finish model (fragment code 5).
pub const METALLIC_ENV: f32 = 2.0;
/// REMASTER (#351, 09-remaster 9.41): `Vertex::env` of a car body face painted in a Pearl zone, also inside
/// the paint band (fragment code 5.25). shader.wgsl `tag_finish` reads the three tags (1 gloss, 1.75, 2).
pub const PEARL_ENV: f32 = 1.75;
/// `Vertex::env` of an env-mapped scene-object polygon (Gouraud types 8/9 with flags & 0x60,
/// 05-formats "Env-mapped polygons"): the track's `EnvMapName` texture times the vertex light (CLASSIC: the D3D
/// env polygon, `0x4402f3` with `0x43ecfc`, #459; REMASTER: the software path's even light `0x42d044`)
pub const PROP_ENV: f32 = 9.0;
/// CLASSIC: `Vertex::env` of a lit object's non-Gouraud polygon (types 0..7) with flag 0x10. The D3D converter passes
/// the flag on untextured types only (`0x441146`): a scene object's such face is face colour x ObjectBrightness, unlit
/// (`0x44073d`, shader.wgsl `vs_main`); textured faces and car draws ignore the tag. `vs_remaster` treats it as 0.
pub const SELF_LIT_ENV: f32 = 10.0;
/// CHEAT tribute to tron (#320, `0x48e964`, 08 §8.14): in a Tron scene `Vertex::color.a` of every MYO triangle is
/// `TRON_CODE` plus its code (plain scenes keep alpha 1). Bits 0..2: the edge opposite corner k is an edge of the
/// polygon, outlined (a fan diagonal of a quad is not); `TRON_DARK`: untextured flat polygon, black fill; `TRON_KEYED`:
/// colour-keyed texture, blue x 0.25 and no outline. shader.wgsl `tron_shade` reads it.
pub const TRON_CODE: f32 = 2.0;
/// REMASTER Neon grid (#320, design choice): the track whose sky dome a Tron scene loads (Goldbridge), graded dark cyan
/// (shader `tron_dome`) like the Tron Legacy night sky. Classic keeps the original's black sky.
pub const TRON_SKY_TRACK: u32 = 8;
pub const TRON_DARK: u32 = 8;
pub const TRON_KEYED: u32 = 16;

/// The cheats that change how a race's scene is built (`load_progress`); default = none.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct SceneCheat {
    /// lemmy / ace "Hell mode" (#316, `0x439e84`), ramlosa / h2o (#317): one WorldBrightness and ObjectBrightness for the
    /// whole scene (the world colours, the cars and props, `Scene::object_scale`); `None` = the track's own
    pub brightness: Option<[f32; 3]>,
    /// tribute to tron (#320): the MYO triangles carry their Tron code (`TRON_CODE`)
    pub tron: bool,
}
/// Sign face textures [O: each is the one face polygon of its prop; Pilsg is also in the
/// NEOCITY, ATLANTIKA and RUHRSTADT world meshes]. Tagged by name: the data has no reflectivity
/// [design choice].
const RETRO_TEXTURES: [&str; 30] = [
    "_100m", "_200m", "_300m", "_vpil1", "_vpil2", "_vpil3", "arro2", "arro3", "arroc", "arrod", "krok", "krok2b", "pilsg", "pilsvrt", "lilapil", "_lilapil", "_mgulpil", "fetpil", "greenpil", "_moskylt", "redarow1", "redarow2", "closed2", "skyl3", "sky2b", "skyp", "skyc", "work", "skyl2", "skyl2d",
];

fn is_retro(name: &str) -> bool {
    let stem = name.rsplit_once('.').map_or(name, |(s, _)| s);
    RETRO_TEXTURES.iter().any(|t| stem.eq_ignore_ascii_case(t))
}

/// Per-track lighting, fog and sky (Game/TrackN.trk + the MYW header).
/// Formulas: re-notes/05-formats.md "Rendering colour model".
#[derive(Clone, Copy, Debug)]
pub struct Look {
    /// CueColour / 255
    pub fog_colour: [f32; 3],
    /// CueStart: fog starts at this fraction of the view distance (0x439e84)
    pub fog_start: f32,
    /// ObjectAmbient
    pub object_ambient: [f32; 3],
    /// sun colour * intensity / 255 (MYW 0x44, 0x48)
    pub sun: [f32; 3],
    /// towards the sun, view space
    pub sun_dir: [f32; 3],
    pub specular: f32,
    /// LightIntensity: scales the MYW point lights (0x4353cc)
    pub light_intensity: f32,
    /// EnvMapName texture layer, if any
    pub env_layer: Option<usize>,
    pub sky_top: [f32; 3],
    pub sky_bottom: [f32; 3],
    pub horizon: f32,
    /// REMASTER car lights: 0 on a bright track, 1 at night [design choice]. From the mean
    /// luma of the sky image (TopColour/BottomColour without one) as the player sees it:
    /// 1 - smoothstep(0.08, 0.30, luma). Mean sky lumas [O]: NeoCity 0.11, Goldbridge 0.18,
    /// RuhrStadt 0.20, RedRock 0.21, the others 0.43 and up. CueColour and the MYW sun don't
    /// tell night from sunset (Okkun's fog is near black under a bright sunset sky).
    pub darkness: f32,
    /// REMASTER Enhanced fog (17-fog-rework): the height the haze is thickest at, the 5th percentile of
    /// the world vertices' y [design choice]
    pub ground_y: f32,
    /// REMASTER this track's fog constants (`fog::load`: table + `remaster/fog/trackNN.fog`)
    pub fog: crate::render::FogTune,
    /// REMASTER fog colour (17-fog-rework.md): the mean of the sky band's bottom 15 % of rows
    /// (BottomColour without a band), gamma 0..1
    pub horizon_band: [f32; 3],
}

/// REMASTER: how a generated car's halo follows its lamp's lit pattern (`procgen::lamps`).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum HaloStrip {
    /// an ordinary halo
    #[default]
    None,
    /// one straight run of the pattern, of this half-width (m): drawn as a flat-ended bar along
    /// p0–p1 that cross-fades into the next one at a bend (`join`), no burst, no light, no flare
    Strip { width: f32, join: [bool; 2] },
    /// the record the strips came from: keeps the burst, the spot light and the flare, no glow
    Anchor,
}

/// One light halo (MYO/MYW halo record) ready to draw: endpoints in view space (car-local
/// for car meshes), colour 0..1, texture layer.
#[derive(Clone, Copy, Debug)]
pub struct HaloInst {
    pub p0: Vec3,
    pub p1: Vec3,
    pub rgb: [f32; 3],
    pub layer: f32,
    pub flags: u32,
    /// the record's index into the owner's `Halo` texture list: on cars 0 = headlights, 1 = tail
    /// lights, which the object renderer draws with their own kind (0x439134)
    pub texture: u32,
    /// CLASSIC (0x436763): the direction a directional halo shines in (car-local view space), ZERO =
    /// omnidirectional. Only set by the Classic car-light kinds (render.rs `classic_car_halo`).
    pub dir: Vec3,
    /// dominant colour of the halo texture, brightest channel 1 (tints its lens flare)
    pub tint: [f32; 3],
    /// extra pull towards the camera (metres): scene objects' halos sit inside their own mesh
    /// (the blinkers), so they are drawn in front of it [design choice]
    pub pull: f32,
    pub strip: HaloStrip,
}

/// REMASTER (design choice, 09-remaster 9.38): the headlamps of a scene object whose MYO has no
/// headlight halo. Lamps are drawn and lit like a car's front lamps (`CarLightTune::head`).
#[derive(Clone, Debug, Default)]
pub struct PropLamps {
    /// the way the lamps shine, object-local view space
    pub forward: Vec3,
    /// short segments across each lamp, object-local view space, no strip, no direction
    pub lamps: Vec<HaloInst>,
}

/// REMASTER (design choice): scene objects that get headlamps: MYO file stem, forward axis in
/// game coordinates, lamp centres. `Je_spinn` (track 8's police spinner) [O: its cyan lamp faces,
/// flags 0x70, are MYO polygons 28 and 39, centred (+-1.30, 0.05, -3.66) on the front of the two
/// pods; D: the front is -z, `goldbridge_move.lws` moves it along local -z]. The other vehicle props
/// (track 3 `Hover`, track 8 `Je_cybenv`, track 5 `Je_cyheli2`, `AirShipnn`) have no lamp
/// geometry at the front [O: survey of every shipped MYO], so none is listed.
const PROP_HEADLAMPS: &[HeadlampDef] = &[HeadlampDef { name: "Je_spinn", forward: [0.0, 0.0, -1.0], lamps: &[[1.30, 0.05, -3.70], [-1.30, 0.05, -3.70]] }];

/// One `PROP_HEADLAMPS` entry
struct HeadlampDef {
    name: &'static str,
    forward: [f32; 3],
    lamps: &'static [[f32; 3]],
}
// Half the width (m) of a lamp's halo segment, its colour (cool white, like the cyan faces) and
// the car halo texture it uses
motor_tune::tunable! {
    /// Half the width of a lamp's halo segment.
    /// range: 0.01..1
    /// unit: m
    /// applies: reload
    /// area: Graphics/Lights
    PROP_LAMP_HALF: f32 = 0.1;
}
const PROP_LAMP_RGB: [f32; 3] = [0.85, 1.0, 0.98];
const PROP_LAMP_TEXTURE: &str = "Cars/Textures/halowhit.iff";

/// REMASTER street lamp light [design choice, 11-dynamic-shadows.md]: a world halo turned into a
/// downward cone light. Position in view space, `range` from its height above the road.
#[derive(Clone, Copy, Debug)]
pub struct LampLight {
    pub pos: Vec3,
    pub range: f32,
    /// halo colour times its texture's tint
    pub rgb: [f32; 3],
}

motor_tune::tunable! {
    /// Range of a lamp's light per metre of its height above the road.
    /// range: 0.5..6
    /// unit: m/m
    /// applies: reload
    /// area: Graphics/lamps
    LAMP_RANGE_PER_HEIGHT: f32 = 2.5;
    /// The shortest range a lamp's light reaches, however low the lamp hangs.
    /// range: 0..20
    /// unit: m
    /// applies: reload
    /// area: Graphics/lamps
    LAMP_RANGE_MIN: f32 = 6.0;
    /// The longest range a lamp's light reaches, however high the lamp hangs.
    /// range: 5..60
    /// unit: m
    /// applies: reload
    /// area: Graphics/lamps
    LAMP_RANGE_MAX: f32 = 25.0;
}

/// The lamp lights of a track: each world halo, merged with any other within 1.5 m (the survey,
/// `examples/lampaudit.rs`, found none to merge on the shipped tracks) [O].
fn lamp_lights(halos: &[HaloInst], heights: &Heights) -> Vec<LampLight> {
    let mut out: Vec<LampLight> = Vec::new();
    for h in halos {
        let pos = (h.p0 + h.p1) * 0.5;
        if out.iter().any(|l| l.pos.distance(pos) < 1.5) {
            continue;
        }
        // view space z is the game's negated
        let up = pos.y - heights.lowest(pos.x, -pos.z, 4.0).unwrap_or(pos.y);
        let range = if up > 0.5 { (up * LAMP_RANGE_PER_HEIGHT.get()).clamp(LAMP_RANGE_MIN.get(), LAMP_RANGE_MAX.get().max(LAMP_RANGE_MIN.get())) } else { 10.0 };
        out.push(LampLight { pos, range, rgb: [0, 1, 2].map(|k| h.rgb[k] * h.tint[k]) });
    }
    out
}

pub struct SkyImage {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

pub struct Scene {
    pub tris: Vec<Vertex>,
    /// REMASTER world fill (12-world-fill): generated pieces from `remaster/fill/trackNN.fill`,
    /// world-style vertices drawn after the world. Kept out of `tris` so Classic, `world_objects`
    /// and picking are unchanged.
    pub fill: Vec<Vertex>,
    /// REMASTER water (procgen/water.rs): the surface meshes, per body the packed parameters,
    /// and per world triangle whether the Remaster world draw leaves it out (empty: none)
    pub water: Vec<Vertex>,
    pub water_params: Vec<[[f32; 4]; procgen::water::ROWS]>,
    pub water_hide: Vec<bool>,
    /// REMASTER the generated bed under the water (procgen/water.rs `Mesh::bed`)
    pub water_bed: Vec<Vertex>,
    /// REMASTER tree models (procgen/trees.rs, #389): the pack models standing where the original's tree
    /// cards stood (world-style vertices, view space), and per world triangle whether it is a card they
    /// replace (empty: no trees)
    pub trees: Vec<Vertex>,
    pub tree_hide: Vec<bool>,
    /// REMASTER the original's tree cards (`trees::find`), every one found, swapped or not: the fill's placed
    /// models (#390) are lit and coloured like them
    pub tree_sites: Vec<procgen::trees::Site>,
    /// REMASTER clouds (#470): the fill's `Piece cloud` fog volumes, read by the Remaster fog shaders
    pub clouds: Vec<procgen::fill::Cloud>,
    /// REMASTER (#390): the layer of the tree pack's first image in `layers` (the pack's images follow it), for
    /// the cards' models and the fill's `Shape::Model`; None when the pack does not load or the layers are full
    pub tree_pack_layer: Option<usize>,
    /// REMASTER grass shells (procgen/grass.rs): the tagged world triangles and per body the
    /// packed parameters
    pub grass: Vec<Vertex>,
    pub grass_params: Vec<[[f32; 4]; procgen::grass::ROWS]>,
    /// REMASTER grass: the outline distance per pixel (the edge fade)
    pub grass_edge: Option<procgen::grass::EdgeMap>,
    pub lines: Vec<Vertex>,
    /// RGBA8, TEX x TEX per layer
    pub layers: Vec<Vec<u8>>,
    /// REMASTER the texture pack's layers and their ids per entry of `layers` (texpack.rs)
    pub pack: texpack::Pack,
    pub look: Look,
    pub sky_image: Option<SkyImage>,
    /// REMASTER sky dome HDRI of this track (`remaster/sky/trackNN.exr`, 09-remaster 9.32)
    pub hdri: Option<crate::sky::Hdri>,
    pub world_halos: Vec<HaloInst>,
    /// REMASTER street lamps as lights (from `world_halos`)
    pub lamps: Vec<LampLight>,
    /// tyre smoke, grass, sparks and lens flare textures
    pub fx: FxLayers,
    /// per car mesh (same order as `cars`)
    pub car_halos: Vec<Vec<HaloInst>>,
    /// REMASTER the halos of `remaster_cars`' bodies where they moved (empty: the original's)
    pub remaster_car_halos: Vec<Vec<HaloInst>>,
    /// the track's MYW point lights, lit objects only (05-formats section 8)
    pub point_lights: PointLights,
    pub view_distance: f32,
    pub start_pos: Vec3,
    pub start_dir: Vec3,
    /// meshes: car k is body `k * CAR_MESHES` followed by its 4 wheels (FL, FR, BR, BL),
    /// in car-local view space; wheels are centred on their own hub
    pub cars: Vec<Vec<Vertex>>,
    /// REMASTER generated car parts (09-remaster 9.20), parallel to `cars`: an empty mesh
    /// means "draw the original" (bodies, for now)
    pub remaster_cars: Vec<Vec<Vertex>>,
    /// CLASSIC car detail levels (05-formats "MYO"): per car, body `OBJ1`, `OBJ2` and `OBJ3` (`OBJ2` and
    /// `OBJ3` have the wheels built in; `OBJ3` is for the supercars / grem cheat); an empty mesh = missing, draw `OBJ0`
    pub car_lods: Vec<[Vec<Vertex>; 3]>,
    /// per car: wheel centres at rest (car-local view space, for static display) and radius.
    /// The tyre bottoms stand `hubs.y - radius` above the model origin: the ground at rest
    /// (`CarParams::rest_ground` less `MODEL_OFFSET`, 6.4 cm on the Titan), as a race draws them
    pub wheel_hubs: Vec<([Vec3; 4], f32)>,
    /// per car: tyre half-width of each wheel mesh (m), for skid marks
    pub wheel_half_width: Vec<[f32; 4]>,
    /// REMASTER: the tread texture's layer (colour; normal + roughness follow), which the detailed
    /// skid marks sample; None when the layer array is full
    pub tyre_layer: Option<f32>,
    /// scene object meshes, object-local view space, in `motor_sim::props::Props::meshes` order,
    /// then the three `FINISH_EMBLEMS` (always present, empty when missing)
    pub props: Vec<Vec<Vertex>>,
    /// per `props` mesh: its light halos (object-local view space)
    pub prop_halos: Vec<Vec<HaloInst>>,
    /// REMASTER: per `props` mesh, its headlamps (`PROP_HEADLAMPS`; empty for most)
    pub prop_lamps: Vec<PropLamps>,
    /// Picking metadata (motorview's target overlay). Per MYW object: its triangles in `tris`
    /// (triangle indices) and its file offset, the object's only identifier.
    pub world_objects: Vec<(std::ops::Range<usize>, usize)>,
    /// per world triangle: index into `world_textures` (u32::MAX = untextured)
    pub world_tri_texture: Vec<u32>,
    pub world_textures: Vec<String>,
    /// REMASTER editor lists: `world_textures` sorted case-insensitively, duplicates (by case) dropped
    pub world_textures_sorted: Vec<String>,
    /// per `props` mesh: the .lws object file stem
    pub prop_names: Vec<String>,
    /// per car slot: `car_label` ("01 Kobra X3")
    pub car_names: Vec<String>,
    /// per car slot: the car file's own Colour1..3 (what CustomCarColours replaces)
    pub car_colours: Vec<[[u8; 3]; 3]>,
    /// REMASTER (#478) per car slot: what loading its model file found (None: the car has no file)
    pub car_models: Vec<Option<ModelCheck>>,
    /// the track's ObjectBrightness: the car face colours are emitted times it (`rgba_obj`)
    pub object_scale: [f32; 3],
    pub stats: String,
}

/// One MYW point light in view space. `scale` = LightIntensity * the record's intensity (can be
/// negative: it darkens).
#[derive(Clone, Copy, Debug)]
pub struct ViewLight {
    pub pos: Vec3,
    pub range: f32,
    pub rgb: [f32; 3],
    pub scale: f32,
}

/// The MYW point lights (section 8) and the per-cell lists that select them (0x439134 hands
/// the list of the object's cell to 0x4353cc). The lights of a cell are exactly those whose
/// range reaches its 19 m square [D, checked on all 8 tracks].
#[derive(Default, Clone)]
pub struct PointLights {
    pub lights: Vec<ViewLight>,
    cells: Vec<Vec<u16>>,
    grid: (usize, usize),
    /// bbox min x, z and the cell size (game coordinates)
    origin: [f32; 2],
    cell: [f32; 2],
}

impl PointLights {
    fn new(w: &myw::World, light_intensity: f32) -> Self {
        let lights = w
            .lights
            .iter()
            .map(|l| ViewLight { pos: Vec3::from(to_view(l.pos)), range: l.range, rgb: l.rgb, scale: l.intensity * light_intensity })
            .collect();
        PointLights {
            lights,
            cells: w.cell_lights.clone(),
            grid: (w.grid_w, w.grid_h),
            origin: [w.bbox_min[0], w.bbox_min[2]],
            cell: [w.cell_size[0] as f32, w.cell_size[1] as f32],
        }
    }

    /// Indices into `lights` for an object at `pos` (view space): its cell's list (empty
    /// outside the grid, as in the original).
    pub fn at(&self, pos: Vec3) -> &[u16] {
        let x = ((pos.x - self.origin[0]) / self.cell[0]).floor();
        let z = ((-pos.z - self.origin[1]) / self.cell[1]).floor();
        if x < 0.0 || z < 0.0 || x as usize >= self.grid.0 || z as usize >= self.grid.1 {
            return &[];
        }
        self.cells.get(z as usize * self.grid.0 + x as usize).map_or(&[], Vec::as_slice)
    }
}

/// CLASSIC podium emblems for places 1–3 (`info_FinishInit` 0x44b620) [O]. They follow the
/// .lws meshes in `Scene::props`, so place p is mesh `Props::meshes.len() + p − 1`
/// (`Session::finish_emblem`).
pub const FINISH_EMBLEMS: [&str; 3] = ["Data/Finish/first.myo", "Data/Finish/second.myo", "Data/Finish/third.myo"];

type Res<T> = Result<T, String>;

fn read(root: &Path, rel: &str) -> Res<Vec<u8>> {
    motor_data::fs::read(root, rel)
}

fn config(root: &Path, rel: &str) -> Res<Config> {
    motor_data::fs::config(root, rel)
}

fn s(c: &Config, k: &str) -> Res<String> {
    c.get(k).and_then(|e| e.values.first().cloned()).ok_or_else(|| format!("config key {k} missing"))
}

fn fv(c: &Config, k: &str) -> Option<Vec<f32>> {
    c.get(k).map(|e| e.values.iter().filter_map(|v| v.parse().ok()).collect())
}

/// Texture library: every IFF in the track's PDI packs, looked up case-insensitively.
struct TexLib<'a> {
    packs: Vec<pdi::Pack<'a>>,
}

impl TexLib<'_> {
    fn decode(&self, name: &str) -> Option<iff::Image> {
        let base = name.rsplit(['\\', '/']).next().unwrap_or(name);
        self.packs.iter().find_map(|p| p.find(base).and_then(|e| p.bytes(e).ok()).and_then(|b| iff::decode(b).ok()))
    }
}

/// Polygon flag bit 2: palette index 0 is see-through. Some worlds have it baked in; the
/// loader also sets it for textures in the track's `ZeroTransparent` list. The IFF BMHD transparent colour is not used by the game (every
/// texture has one, e.g. TRIROAD.IFF says 10).
const ZERO_TRANSPARENT: u8 = 4;

/// Wrapping bilinear resample to TEX x TEX RGBA.
/// `key`: the colour index drawn transparent (alpha 0), if any.
///
/// CLASSIC Begin3D (0x415354) sets linear texture filtering and the D3D texture is sampled at its own size, so a magnified
/// 64x64 texture is smooth; the array layer is TEX x TEX, so the upscale is bilinear here (#450; it was nearest, which
/// left square stair steps on the chevron arrows) and the Linear sampler does the rest. Texels wrap (the sampler repeats).
/// A layer of TEX x TEX is copied unchanged. APPROX [I]: RGB is weighted by alpha so the key colour never bleeds into
/// its neighbours, alpha interpolates, and `fs_main` cuts at 0.5 (the D3D colour-key rule with filtering is not traced).
fn resample(img: &iff::Image, key: Option<u16>) -> Vec<u8> {
    let (w, h) = (img.width as usize, img.height as usize);
    let src: Vec<[f32; 4]> = img
        .indices
        .iter()
        .map(|&i| {
            let c = img.palette.get(i as usize).copied().unwrap_or([255, 0, 255]);
            [c[0] as f32, c[1] as f32, c[2] as f32, if key == Some(i as u16) { 0.0 } else { 255.0 }]
        })
        .collect();
    let t = TEX as usize;
    // per output index: the two source indices and the weight of the second
    let taps = |n: usize| -> Vec<(usize, usize, f32)> {
        (0..t)
            .map(|o| {
                let f = (o as f32 + 0.5) * n as f32 / t as f32 - 0.5;
                let i0 = f.floor();
                ((i0 as isize).rem_euclid(n as isize) as usize, ((i0 as isize + 1).rem_euclid(n as isize)) as usize, f - i0)
            })
            .collect()
    };
    let (tx, ty) = (taps(w), taps(h));
    let mut out = vec![0u8; t * t * 4];
    for (y, &(y0, y1, fy)) in ty.iter().enumerate() {
        for (x, &(x0, x1, fx)) in tx.iter().enumerate() {
            let px = [(y0, x0, (1.0 - fy) * (1.0 - fx)), (y0, x1, (1.0 - fy) * fx), (y1, x0, fy * (1.0 - fx)), (y1, x1, fy * fx)];
            let (mut rgb, mut a) = ([0.0f32; 3], 0.0f32);
            for (sy, sx, wt) in px {
                let s = src[sy * w + sx];
                a += s[3] * wt;
                for k in 0..3 {
                    rgb[k] += s[k] * s[3] * wt;
                }
            }
            let o = (y * t + x) * 4;
            for k in 0..3 {
                // fully keyed neighbourhood (a == 0): the plain mix, so the key texels keep their colour (mip chains)
                out[o + k] = if a > 0.0 { (rgb[k] / a).round() as u8 } else { px.iter().map(|&(sy, sx, wt)| src[sy * w + sx][k] * wt).sum::<f32>().round() as u8 };
            }
            out[o + 3] = a.round() as u8;
        }
    }
    out
}

fn checker() -> Vec<u8> {
    let mut out = Vec::with_capacity((TEX * TEX * 4) as usize);
    for y in 0..TEX {
        for x in 0..TEX {
            let on = ((x / 32) + (y / 32)) % 2 == 0;
            out.extend_from_slice(if on { &[255, 0, 255, 255] } else { &[40, 0, 40, 255] });
        }
    }
    out
}

/// Maps texture names to array layers, decoding each once.
/// (texture name, zero-transparent) -> (layer, source width, source height)
type LayerMap = HashMap<(String, bool), (usize, u32, u32)>;

struct Layers {
    by_name: LayerMap,
    data: Vec<Vec<u8>>,
    missing: usize,
    /// 3DAccBrightness: the texture loader 0x40fec4 scales every texel channel by this
    /// and clamps at 255 (0x4490d4) before upload
    gain: [f32; 3],
    /// lower-cased `ZeroTransparent` texture names from the .trk
    zero_transparent: Vec<String>,
    /// REMASTER texture pack (texpack.rs): the install and track to look in (None: no
    /// `remaster/textures`), the materials found per base layer, files that did not decode
    pack_dir: Option<(std::path::PathBuf, u32)>,
    pending: Vec<(usize, texpack::Material)>,
    pack_notes: Vec<String>,
}

/// A track's `.trk` colour triple `k`, or `d`.
fn trk_rgb(trk: &Config, k: &str, d: [f32; 3]) -> [f32; 3] {
    match fv(trk, k) {
        Some(v) if v.len() >= 3 => [v[0], v[1], v[2]],
        _ => d,
    }
}

impl Layers {
    /// The track's layer set; `pack`: look in the REMASTER texture pack too.
    fn new(root: &Path, trk: &Config, track: u32, pack: bool) -> Layers {
        Layers {
            by_name: HashMap::new(),
            data: Vec::new(),
            missing: 0,
            gain: trk_rgb(trk, "3DAccBrightness", [1.0; 3]),
            zero_transparent: trk.all("ZeroTransparent").filter_map(|e| e.values.first()).map(|v| v.to_ascii_lowercase()).collect(),
            pack_dir: motor_data::fs::find_ci(root, "remaster/textures").filter(|p| pack && p.is_dir()).map(|_| (root.to_path_buf(), track)),
            pending: Vec::new(),
            pack_notes: Vec::new(),
        }
    }

    fn get(&mut self, lib: &TexLib, name: &str, zero_transparent: bool) -> (usize, u32, u32) {
        let lower = name.to_ascii_lowercase();
        let zero_transparent = zero_transparent || self.zero_transparent.contains(&lower);
        let key = (lower, zero_transparent);
        if let Some(&v) = self.by_name.get(&key) {
            return v;
        }
        let v = if self.data.len() >= MAX_LAYERS {
            (0, TEX, TEX)
        } else {
            let (pix, w, h) = match lib.decode(name) {
                Some(img) => {
                    let mut pix = resample(&img, zero_transparent.then_some(0));
                    for px in pix.chunks_mut(4) {
                        for (c, g) in px.iter_mut().zip(self.gain) {
                            *c = (*c as f32 * g).min(255.0) as u8;
                        }
                    }
                    (pix, img.width, img.height)
                }
                None => {
                    self.missing += 1;
                    (checker(), TEX, TEX)
                }
            };
            self.data.push(pix);
            // REMASTER texture pack: the user's maps for this texture, if any
            if let Some((root, track)) = self.pack_dir.clone() {
                let m = texpack::find(&root, track, name, &mut self.pack_notes);
                if !m.is_empty() {
                    self.pending.push((self.data.len() - 1, m));
                }
            }
            (self.data.len() - 1, w, h)
        };
        self.by_name.insert(key, v);
        v
    }

    /// REMASTER texture pack: the found maps as layers of one size (the largest map's, 256..1024)
    /// with the per-layer ids. Colour gets the 3DAccBrightness gain like the original; a colour
    /// without alpha keeps the original's colour-key cut-out. Normals and roughness are plain data.
    fn finish_pack(&mut self) -> texpack::Pack {
        let pending = std::mem::take(&mut self.pending);
        let mut pack = texpack::Pack { notes: std::mem::take(&mut self.pack_notes), ..Default::default() };
        if pending.is_empty() {
            return pack;
        }
        let want: usize = pending.iter().map(|(_, m)| m.colour.is_some() as usize + m.normal.is_some() as usize).sum();
        let largest = pending.iter().flat_map(|(_, m)| [m.colour.as_ref(), m.normal.as_ref()]).flatten().map(|m| m.w.max(m.h)).max().unwrap_or(0);
        pack.size = texpack::layer_size(largest, want.min(MAX_PACK_LAYERS));
        pack.map = vec![[-1, -1]; self.data.len()];
        let size = pack.size;
        for (base, m) in pending {
            let need = m.colour.is_some() as usize + m.normal.is_some() as usize;
            if pack.layers.len() + need > MAX_PACK_LAYERS {
                pack.notes.push("layer limit reached".into());
                break;
            }
            if let Some(c) = &m.colour {
                let mut px = texpack::resize(c, size, size).rgba;
                px.as_chunks_mut::<4>().0.iter_mut().for_each(|p| p[..3].iter_mut().zip(self.gain).for_each(|(ch, g)| *ch = (*ch as f32 * g).min(255.0) as u8));
                if c.rgba.as_chunks::<4>().0.iter().all(|p| p[3] == 255) && self.data[base].as_chunks::<4>().0.iter().any(|p| p[3] != 255) {
                    let orig = texpack::Map { w: TEX, h: TEX, rgba: self.data[base].clone() };
                    let up = texpack::resize(&orig, size, size);
                    px.as_chunks_mut::<4>().0.iter_mut().zip(up.rgba.as_chunks::<4>().0.iter()).for_each(|(d, s)| d[3] = s[3]);
                }
                pack.map[base][0] = pack.layers.len() as i32;
                pack.layers.push(px);
                pack.counts[0] += 1;
            }
            if let Some(n) = &m.normal {
                pack.map[base][1] = pack.layers.len() as i32;
                pack.layers.push(texpack::resize(n, size, size).rgba);
                pack.counts[1] += 1;
                pack.counts[2] += m.has_rough as usize;
            }
        }
        pack
    }

    /// REMASTER the carbon fibre weave (9.22 phase 5, assets/textures/carbon, CC0): its colour
    /// layer, the next one holding its normal (rgb) and roughness (a), as `procgen::CARBON_ENV`
    /// reads them. No 3DAccBrightness gain: not the game's art. None when the array is full.
    fn carbon(&mut self) -> Option<f32> {
        self.asset_pair("carbon", include_bytes!("../../../assets/textures/carbon/colour.png"), include_bytes!("../../../assets/textures/carbon/normal_rough.png"))
    }

    /// REMASTER the tread rubber (9.20, assets/textures/tyre): colour layer, then normal + roughness,
    /// as `procgen::TYRE_ENV` reads them. None when the array is full.
    fn tyre(&mut self) -> Option<f32> {
        self.asset_pair("tyre", include_bytes!("../../../assets/textures/tyre/colour.png"), include_bytes!("../../../assets/textures/tyre/normal_rough.png"))
    }

    /// Two embedded 256² layers (colour, normal + roughness) under `name`; the first layer's index.
    fn asset_pair(&mut self, name: &str, colour: &[u8], normal_rough: &[u8]) -> Option<f32> {
        let key = (format!("asset:{name}"), false);
        if let Some(&(l, _, _)) = self.by_name.get(&key) {
            return Some(l as f32);
        }
        if self.data.len() + 2 > MAX_LAYERS {
            return None;
        }
        for bytes in [colour, normal_rough] {
            let (_, _, px) = crate::ui::decode_rgba_png(bytes).filter(|(w, h, _)| (*w, *h) == (TEX, TEX))?;
            self.data.push(px);
        }
        let l = self.data.len() - 2;
        self.by_name.insert(key, (l, TEX, TEX));
        Some(l as f32)
    }

    /// REMASTER (#478): an imported car model's base colour image as one layer, cached under `key`. No
    /// 3DAccBrightness gain (not the game's art, as `carbon`) [design choice]. None when the array is full.
    fn image(&mut self, key: String, m: &texpack::Map) -> Option<f32> {
        let key = (key, false);
        if let Some(&(l, _, _)) = self.by_name.get(&key) {
            return Some(l as f32);
        }
        if self.data.len() >= MAX_LAYERS || m.w == 0 || m.h == 0 {
            return None;
        }
        self.data.push(texpack::resize(m, TEX, TEX).rgba);
        let l = self.data.len() - 1;
        self.by_name.insert(key, (l, m.w, m.h));
        Some(l as f32)
    }

    /// Layer for an IFF file outside the PDI packs (halo textures), or None if unreadable.
    fn file(&mut self, root: &Path, rel: &str) -> Option<usize> {
        let key = (format!("file:{}", rel.to_ascii_lowercase()), false);
        if let Some(&(l, _, _)) = self.by_name.get(&key) {
            return Some(l);
        }
        if self.data.len() >= MAX_LAYERS {
            return None;
        }
        let img = iff::decode(&read(root, rel).ok()?).ok()?;
        // the file's own colour key (masking 2: smoke frames, some halos)
        self.data.push(resample(&img, img.transparent));
        let l = self.data.len() - 1;
        self.by_name.insert(key, (l, img.width, img.height));
        Some(l)
    }

    /// Layers for a config's `Halo` lines, in order (halo records index this list).
    fn halo_list(&mut self, root: &Path, c: &Config) -> Vec<Option<usize>> {
        c.all("Halo").filter_map(|e| e.values.first()).map(|f| self.file(root, f)).collect()
    }
}

/// Texture layers of the particle and flare effects. The D3D build loads its own sets
/// (gfx_InitAnims 0x445c8c..0x445d69): `d3d\smoke\smoke%.2d.iff` x9, `d3d\grass\gras%.2d.iff`
/// x9 [O: path 0x4d39da], `d3d\sparks\spark%.2d.iff` x6 (11 files ship), and race load
/// 0x45e87c reads `data\hex.iff`, the flares of the software halo drawer (mode 1, 0x435bd2 [O];
/// REMASTER's hex flares). CLASSIC: the D3D GeometryEngineInit (0x43a963..0x43ab8d) builds the 256²
/// halo page from `data\halofix.iff` at (0, 0) and `data\hwhex.iff` at (128, 0) (halofix's palette,
/// which hwhex's own equals) [O]; every halo of the D3D list samples halofix (`halo`), its lens flares
/// hwhex (`hwhex`, 0x43d864).
#[derive(Clone, Debug, Default)]
pub struct FxLayers {
    pub smoke: Vec<usize>,
    pub grass: Vec<usize>,
    pub sparks: Vec<usize>,
    pub flare: Option<usize>,
    pub halo: Option<usize>,
    pub hwhex: Option<usize>,
}

impl FxLayers {
    fn load(root: &Path, layers: &mut Layers) -> FxLayers {
        let mut set = |fmt: &dyn Fn(usize) -> String, n: usize| -> Vec<usize> { (1..=n).filter_map(|i| layers.file(root, &fmt(i))).collect() };
        let smoke = set(&|i| format!("Data/d3d/smoke/smoke{i:02}.iff"), 9);
        let grass = set(&|i| format!("Data/d3d/grass/gras{i:02}.iff"), 9);
        let sparks = set(&|i| format!("Data/d3d/sparks/spark{i:02}.iff"), 6);
        let flare = layers.file(root, "Data/hex.iff");
        let (halo, hwhex) = (layers.file(root, "Data/halofix.iff"), layers.file(root, "Data/hwhex.iff"));
        FxLayers { smoke, grass, sparks, flare, halo, hwhex }
    }
}

/// Brightness-weighted mean colour of a texture, scaled so its brightest channel is 1.
fn dominant(px: &[u8]) -> [f32; 3] {
    let mut sum = [0.0f32; 3];
    for p in px.chunks(4) {
        let w = p[0].max(p[1]).max(p[2]) as f32;
        for c in 0..3 {
            sum[c] += p[c] as f32 * w;
        }
    }
    let m = sum[0].max(sum[1]).max(sum[2]);
    if m > 0.0 { sum.map(|c| c / m) } else { [1.0; 3] }
}

fn halo_insts(halos: &[myo::Halo], textures: &[Option<usize>]) -> Vec<HaloInst> {
    halos
        .iter()
        .filter(|h| h.flags & 2 != 0)
        .filter_map(|h| {
            let layer = (*textures.get(h.texture as usize)?)?;
            Some(HaloInst {
                p0: Vec3::from(to_view(h.p0)),
                p1: Vec3::from(to_view(h.p1)),
                rgb: h.rgb.map(|c| c as f32 / 255.0),
                layer: layer as f32,
                flags: h.flags,
                texture: h.texture,
                dir: Vec3::ZERO,
                tint: [1.0; 3],
                pull: 0.0,
                strip: HaloStrip::None,
            })
        })
        .collect()
}

/// Colour byte times a per-channel brightness, as the D3D vertex colour (0..1, clamped).
pub(crate) fn rgba_f(c: [u8; 4], scale: [f32; 3]) -> [f32; 4] {
    let ch = |i: usize| (c[i] as f32 * scale[i] / 255.0).min(1.0);
    [ch(0), ch(1), ch(2), 1.0]
}

/// A lit object's face colour byte times ObjectBrightness, not clamped: the shaders clamp the final colour after the
/// object light, as `0x4302f8` does (face x ObjectBrightness x light). CLASSIC textured faces do not use it (shader.wgsl
/// `TEX_FACE`); REMASTER keeps it for every face. Every track ships ObjectBrightness 1, so it only
/// matters for ramlosa / h2o's (0.5, 1.0, 1.2) (#317).
pub(crate) fn rgba_obj(c: [u8; 4], scale: [f32; 3]) -> [f32; 4] {
    let ch = |i: usize| c[i] as f32 * scale[i] / 255.0;
    [ch(0), ch(1), ch(2), 1.0]
}

/// How a mesh is coloured: pre-lit world (vertex colour * WorldBrightness, 0x431708) or a
/// lit object (face colour * ObjectBrightness * per-vertex light, 0x4302f8).
type Normals<'a> = (&'a [[f32; 3]], &'a [[f32; 4]]);

struct Shade<'a> {
    scale: [f32; 3],
    /// vertex normals and planes, for lit objects
    normals: Option<Normals<'a>>,
    /// CHEAT tribute to tron (#320): write each triangle's Tron code into its alpha (`TRON_CODE`)
    tron: bool,
}

/// Append one polygon (triangle or quad, fan-triangulated).
#[allow(clippy::too_many_arguments)]
fn emit(
    out: &mut Vec<Vertex>,
    p: &myo::Polygon,
    verts: &[[f32; 3]],
    planes: &[[f32; 4]],
    uvbytes: &[u8],
    tex: Option<(&str, &mut Layers, &TexLib)>,
    xf: &dyn Fn([f32; 3]) -> [f32; 3],
    shade: &Shade,
) {
    let colour = if shade.normals.is_some() { rgba_obj } else { rgba_f };
    let cols: Vec<[f32; 4]> = (0..p.v.len()).map(|k| colour(*p.rgba.get(k).unwrap_or(&p.rgba[0]), shade.scale)).collect();
    let normal = |k: usize| -> [f32; 3] {
        // pre-lit world: the flat plane normal, used only by the dynamic lights
        let Some((vn, planes)) = shade.normals else { return planes.get(p.plane as usize).map_or([0.0; 3], |q| to_view([q[0], q[1], q[2]])) };
        let n = p.vn.get(k).and_then(|&i| vn.get(i as usize)).copied();
        let n = n.or_else(|| planes.get(p.plane as usize).map(|q| [q[0], q[1], q[2]])).unwrap_or([0.0, 1.0, 0.0]);
        to_view(n)
    };
    let (layer, tw, th) = match tex {
        Some((name, layers, lib)) => {
            let (l, w, h) = layers.get(lib, name, p.flags & ZERO_TRANSPARENT != 0);
            (l as f32, w as f32, h as f32)
        }
        None => (-1.0, 1.0, 1.0),
    };
    let uv = |k: usize| -> [f32; 2] {
        match p.uv_off {
            Some(o) if layer >= 0.0 => {
                let o = o as usize + 2 * k;
                let (u, v) = (*uvbytes.get(o).unwrap_or(&0) as f32, *uvbytes.get(o + 1).unwrap_or(&0) as f32);
                [(u + 0.5) / tw, (v + 0.5) / th]
            }
            _ => [0.0, 0.0],
        }
    };
    // Reflection mode (CarShading Reflection): car paint = the Gouraud polygons (types 8/9) with flags 0x60, which the
    // mesh loop sends to the env-map builders (0x4351e2); every car Gouraud polygon has 0x60 [O: all 10 cars]
    let lit = shade.normals.is_some();
    let env = if lit && matches!(p.ty, 8 | 9) {
        1.0
    } else if lit && p.flags & 0x10 != 0 {
        SELF_LIT_ENV
    } else {
        0.0
    };
    let vtx = |k: usize| Vertex { pos: xf(verts[p.v[k] as usize]), uv: uv(k), color: cols[k], layer, normal: normal(k), env };
    // The original skips a polygon unless the eye is on the front of its plane (0x405b28 from
    // the mesh loop 0x434ffc). Each triangle is wound counter-clockwise as seen from the
    // plane's front, so GPU back-face culling does the same test.
    let facing = planes.get(p.plane as usize).map(|q| Vec3::from(to_view([q[0], q[1], q[2]])));
    let n = p.v.len();
    for k in 1..n - 1 {
        let (a, b, c) = (vtx(0), vtx(k), vtx(k + 1));
        let (pa, pb, pc) = (Vec3::from(a.pos), Vec3::from(b.pos), Vec3::from(c.pos));
        let flip = facing.is_some_and(|n| (pb - pa).cross(pc - pa).dot(n) < 0.0);
        let mut tri = if flip { [a, c, b] } else { [a, b, c] };
        if shade.tron {
            let code = tron_code(p, layer >= 0.0, k, n, flip);
            tri.iter_mut().for_each(|v| v.color[3] = TRON_CODE + code as f32);
        }
        out.extend(tri);
    }
}

/// CHEAT tribute to tron (#320): the code (`TRON_CODE`) of fan triangle (v0, vk, vk+1) of an `n`-gon, wound
/// (v0, vk+1, vk) when `flip`. Its edge vk-vk+1 is always the polygon's; v0-vk only for the first triangle and
/// vk+1-v0 only for the last (the others are fan diagonals: `0x48c088` draws a quad's 4 sides, no diagonal).
/// Bit k = the edge opposite corner k. The list the original puts the polygon in decides the rest (08 §8.14):
/// untextured flat types 0/1/4/5 are filled black [I], colour-keyed textures are dimmed and not outlined [D].
fn tron_code(p: &myo::Polygon, textured: bool, k: usize, n: usize, flip: bool) -> u32 {
    if textured && p.flags & ZERO_TRANSPARENT != 0 {
        return TRON_KEYED;
    }
    let (first, last) = ((k == 1) as u32, (k + 1 == n - 1) as u32);
    let edges = 1 | if flip { first << 1 | last << 2 } else { last << 1 | first << 2 };
    let dark = if !textured && matches!(p.ty, 0 | 1 | 4 | 5) { TRON_DARK } else { 0 };
    edges | dark
}

fn to_view(v: [f32; 3]) -> [f32; 3] {
    [v[0], v[1], -v[2]]
}

/// Height lookup for points on the road: nearest world vertex in a 4 m grid.
struct Heights(HashMap<(i32, i32), Vec<[f32; 3]>>);

impl Heights {
    fn new(objs: &[myw::Object]) -> Self {
        let mut m: HashMap<(i32, i32), Vec<[f32; 3]>> = HashMap::new();
        for v in objs.iter().flat_map(|o| &o.vertices) {
            m.entry(((v[0] / 4.0).floor() as i32, (v[2] / 4.0).floor() as i32)).or_default().push(*v);
        }
        Heights(m)
    }
    /// Lowest vertex within `r` metres (at most 8) horizontally of (x, z): the ground under a
    /// lamp, where `at` would find the lamp's own mesh.
    fn lowest(&self, x: f32, z: f32, r: f32) -> Option<f32> {
        let (cx, cz) = ((x / 4.0).floor() as i32, (z / 4.0).floor() as i32);
        let n = (r / 4.0).ceil() as i32;
        let mut lo: Option<f32> = None;
        for dx in -n..=n {
            for dz in -n..=n {
                for v in self.0.get(&(cx + dx, cz + dz)).into_iter().flatten() {
                    if (v[0] - x).powi(2) + (v[2] - z).powi(2) <= r * r {
                        lo = Some(lo.map_or(v[1], |l: f32| l.min(v[1])));
                    }
                }
            }
        }
        lo
    }
    fn at(&self, x: f32, z: f32) -> f32 {
        let (cx, cz) = ((x / 4.0).floor() as i32, (z / 4.0).floor() as i32);
        let mut best = (f32::MAX, 0.0);
        for dx in -2..=2 {
            for dz in -2..=2 {
                for v in self.0.get(&(cx + dx, cz + dz)).into_iter().flatten() {
                    let d = (v[0] - x).powi(2) + (v[2] - z).powi(2);
                    if d < best.0 {
                        best = (d, v[1]);
                    }
                }
            }
        }
        best.1
    }
}

/// REMASTER (#136, #351) a paint zone's finish: Gloss is the original's paint, Metallic and Pearl are drawn by
/// Remaster only (shader.wgsl fragment code 5 / 5.25; Classic draws all three as plain paint).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Finish {
    #[default]
    Gloss,
    Metallic,
    Pearl,
}

impl Finish {
    /// The `Vertex::env` tag of a face painted in this finish; None for gloss (the paint's own tag 1).
    pub fn tag(self) -> Option<f32> {
        match self {
            Finish::Gloss => None,
            Finish::Metallic => Some(METALLIC_ENV),
            Finish::Pearl => Some(PEARL_ENV),
        }
    }
}

/// CustomCarColours of one car: CarColour1..3 and, REMASTER (#136, #351), each zone's finish.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CarPaint {
    pub colours: [[u8; 3]; 3],
    pub finish: [Finish; 3],
}

impl CarPaint {
    /// The original's paint: all three zones gloss.
    pub fn gloss(colours: [[u8; 3]; 3]) -> CarPaint {
        CarPaint { colours, finish: [Finish::Gloss; 3] }
    }
}

/// `paint`: CustomCarColours per car, as (index into `car_ids`, its paint).
pub fn load(root: &Path, track: u32, car_ids: &[u32], paint: &[(usize, CarPaint)]) -> Res<Scene> {
    load_parts(root, track, car_ids, paint, Parts::Full, SceneCheat::default(), None)
}

/// `load` on a worker thread (#335): reports the stage it is in to `progress` and stops with
/// `loading::CANCELLED` when the load is cancelled.
///
/// `cheat`: the name cheats that change how the scene is built (`SceneCheat`).
pub fn load_progress(root: &Path, track: u32, car_ids: &[u32], paint: &[(usize, CarPaint)], cheat: SceneCheat, progress: &Progress) -> Res<Scene> {
    load_parts(root, track, car_ids, paint, Parts::Full, cheat, Some(progress))
}

/// REMASTER (front end Main drive, 09-remaster 9.24): only what the hologram draws and its drive
/// uses: the world triangles (for `road_lines`, untextured) and the cars with their wheel hubs.
/// No world textures, halos, props, sky, lights or generated world (fill, water, grass); the
/// scene is not for drawing a world.
pub fn load_hologram(root: &Path, track: u32, car_ids: &[u32]) -> Res<Scene> {
    load_parts(root, track, car_ids, &[], Parts::Hologram, SceneCheat::default(), None)
}

/// What `load_parts` builds.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Parts {
    Full,
    Hologram,
}

fn load_parts(root: &Path, track: u32, car_ids: &[u32], paint: &[(usize, CarPaint)], parts: Parts, cheat: SceneCheat, progress: Option<&Progress>) -> Res<Scene> {
    let holo = parts == Parts::Hologram;
    Progress::report(progress, Stage::World, 0.0)?;
    let trk = config(root, &format!("Game/Track{track}.trk"))?;
    let base = s(&trk, "BasePath")?;
    let file = s(&trk, "FileName")?;
    let world_bytes = read(root, &format!("{base}/Hardware/{file}")).or_else(|_| read(root, &format!("{base}/{file}")))?;
    let world = myw::parse(&world_bytes).map_err(|e| format!("{file}: {e}"))?;

    let pack_bytes: Vec<Vec<u8>> = ["tex1.pdi", "tex2.pdi"].iter().filter_map(|n| read(root, &format!("{base}/Packed/{n}")).ok()).collect();
    let lib = TexLib { packs: pack_bytes.iter().filter_map(|b| pdi::parse(b).ok()).collect() };
    let rgb = |k: &str, d: [f32; 3]| trk_rgb(&trk, k, d);
    let mut layers = Layers::new(root, &trk, track, true);

    let mut tris = Vec::new();
    let mut world_objects = Vec::new();
    let mut world_tri_texture = Vec::new();
    let world_shade = Shade { scale: cheat.brightness.unwrap_or_else(|| rgb("WorldBrightness", [1.0; 3])), normals: None, tron: cheat.tron };
    for o in &world.objects {
        let first = tris.len() / 3;
        for p in &o.polygons {
            let tex = world.texture_name(p).filter(|_| !holo).map(|n| (n, &mut layers, &lib));
            let start = tris.len();
            let retro = tex.as_ref().is_some_and(|t| is_retro(t.0));
            emit(&mut tris, p, &o.vertices, &o.planes, &o.uvbytes, tex, &to_view, &world_shade);
            if retro {
                tris[start..].iter_mut().for_each(|v| v.env = RETRO_ENV);
            }
            let ti = p.texref_off.and_then(|t| world.texrefs.get(t as usize / 12)).map_or(u32::MAX, |&t| t);
            world_tri_texture.resize(tris.len() / 3, ti);
        }
        world_objects.push((first..tris.len() / 3, o.offset));
    }
    let world_tris = tris.len() / 3;
    let track_halos = if holo { Vec::new() } else { layers.halo_list(root, &trk) };
    let env_layer = s(&trk, "EnvMapName").ok().filter(|_| !holo).and_then(|n| layers.file(root, &n));
    let mut world_halos = if holo { Vec::new() } else { halo_insts(&world.halos, &track_halos) };
    for h in &mut world_halos {
        h.tint = layers.data.get(h.layer as usize).map_or([1.0; 3], |px| dominant(px));
    }
    let fx = if holo { FxLayers::default() } else { FxLayers::load(root, &mut layers) };

    // AI racing line, lifted onto the road surface.
    let heights = if holo { Heights(HashMap::new()) } else { Heights::new(&world.objects) };
    let lamps = lamp_lights(&world_halos, &heights);
    let ai_rel = format!("{}/AI/ai.dat", s(&trk, "AITrackName").unwrap_or(base.clone()));
    let line = read(root, &ai_rel).ok().filter(|_| !holo).and_then(|b| aidat::parse(&b).ok());
    let mut lines = Vec::new();
    let (mut start_pos, mut start_dir) = (Vec3::ZERO, Vec3::NEG_Z);
    if let Some(l) = &line {
        let pts: Vec<[f32; 3]> = l.nodes.iter().map(|n| to_view([n.x, heights.at(n.x, n.z) + 0.3, n.z])).collect();
        for i in 0..pts.len() {
            let c = [1.0, 0.15, 0.1, 1.0];
            lines.push(Vertex { pos: pts[i], uv: [0.0; 2], color: c, layer: -1.0, normal: [0.0; 3], env: 0.0 });
            lines.push(Vertex { pos: pts[(i + 1) % pts.len()], uv: [0.0; 2], color: c, layer: -1.0, normal: [0.0; 3], env: 0.0 });
        }
        if let Some(n) = l.nodes.first() {
            start_pos = Vec3::from(pts[0]);
            start_dir = Vec3::from(to_view(n.dir)).normalize_or(Vec3::NEG_Z);
        }
    }

    // Cars: body LOD0 and 4 wheels as separate meshes, each drawn with its own transform.
    let mut cars = Vec::new();
    let mut remaster_cars = Vec::new();
    let mut car_lods = Vec::new();
    let mut car_halos = Vec::new();
    let mut remaster_car_halos = Vec::new();
    let mut wheel_hubs = Vec::new();
    let mut wheel_half_width = Vec::new();
    let mut notes = Vec::new();
    let mut car_names = Vec::new();
    let mut car_models: Vec<Option<ModelCheck>> = Vec::new();
    let mut car_colours = Vec::new();
    // REMASTER: (car index, index of its note, job) of the generated bodies, run after the loop
    let mut body_jobs: Vec<(usize, usize, BodyJob)> = Vec::new();
    let object_scale = cheat.brightness.unwrap_or_else(|| rgb("ObjectBrightness", [1.0; 3]));
    for (k, &id) in car_ids.iter().enumerate() {
        Progress::report(progress, Stage::Cars, k as f32 / car_ids.len().max(1) as f32)?;
        let mut halos = Vec::new();
        // REMASTER (#478): the halos of an imported car's lamps (empty: the original's)
        let mut imported_halos = Vec::new();
        let colours = paint.iter().find(|p| p.0 == k).map(|p| p.1);
        let cfg = config(root, &format!("Game/car{id:02}.car")).ok();
        car_colours.push(cfg.as_ref().map_or([[0; 3]; 3], own_colours));
        let label = car_label(id, &cfg.as_ref().and_then(|c| c.get("CarName").map(|e| e.values.join(" "))).unwrap_or_default());
        match build_car(root, id, &mut halos, &mut layers, &lib, object_scale, colours, cheat.tron) {
            Ok((meshes, mut generated, hubs, mut n, job, lods)) => {
                // REMASTER (#478): a model file's body wins over the recipe, its wheels over the
                // generated ones corner by corner, its lamps carry the halos
                let livery = colours.unwrap_or_else(|| CarPaint::gloss(cfg.as_ref().map_or([[0; 3]; 3], own_colours)));
                let (imp, check) = imported_car(root, id, &meshes[0], hubs, livery, &halos, &mut layers, object_scale);
                car_models.push(check);
                match imp {
                    Some(imp) => {
                        let from_file: Vec<&str> = imp.wheels.iter().zip(procgen::carmodel::WHEEL_NODES).filter(|(w, _)| w.is_some()).map(|(_, n)| &n["wheel_".len()..]).collect();
                        generated[0] = imp.body;
                        for (g, w) in generated[1..].iter_mut().zip(imp.wheels) {
                            if let Some(w) = w {
                                *g = w;
                            }
                        }
                        let wheel_tris: usize = generated[1..].iter().map(|w| w.len() / 3).sum();
                        let file = if from_file.is_empty() { String::new() } else { format!(" ({} from the file)", from_file.join(" ")) };
                        let mut note = imp.note;
                        if (generated[0].len() + 3 * wheel_tris) / 3 > procgen::carmodel::CAR_TRI_BUDGET {
                            note += &format!(", over the {} triangle budget", procgen::carmodel::CAR_TRI_BUDGET);
                        }
                        n = format!("{label} (imported body {} tris, wheels {wheel_tris}{file}{note})", generated[0].len() / 3);
                        imported_halos = imp.halos;
                    }
                    // the generated body's budget goes into the note once the body is built
                    None => {
                        // REMASTER (#478): `--car-source original` draws the MYO model: no recipe job
                        if let Some(job) = job.filter(|_| procgen::carmodel::source() != procgen::carmodel::CarSource::Original) {
                            body_jobs.push((k, notes.len(), job));
                        }
                    }
                }
                car_names.push(label);
                notes.push(n);
                cars.extend(meshes);
                remaster_cars.extend(generated);
                car_lods.push(lods);
                wheel_hubs.push((hubs.0, hubs.1));
                wheel_half_width.push(hubs.2);
            }
            Err(e) => {
                notes.push(format!("car{id:02} not shown: {e}"));
                car_models.push(None);
                car_names.push(label);
                cars.extend((0..CAR_MESHES).map(|_| Vec::new()));
                remaster_cars.extend((0..CAR_MESHES).map(|_| Vec::new()));
                car_lods.push([Vec::new(), Vec::new(), Vec::new()]);
                wheel_hubs.push(([Vec3::ZERO; 4], 0.35));
                wheel_half_width.push([0.15; 4]);
            }
        }
        car_halos.push(halos);
        car_halos.extend((1..CAR_MESHES).map(|_| Vec::new()));
        remaster_car_halos.push(imported_halos);
        remaster_car_halos.extend((1..CAR_MESHES).map(|_| Vec::new()));
    }
    // REMASTER: the generated bodies, several cars at once (09-remaster 9.22)
    let owners: Vec<(usize, usize)> = body_jobs.iter().map(|j| (j.0, j.1)).collect();
    let jobs: Vec<BodyJob> = body_jobs.into_iter().map(|j| j.2).collect();
    for ((k, note), (job, verts, strips)) in owners.into_iter().zip(run_bodies(jobs)) {
        let at = k * CAR_MESHES;
        // REMASTER: the generated body's budget in the stats line
        let body_tris = verts.len() / 3;
        let wheel_tris: usize = remaster_cars[at + 1..at + CAR_MESHES].iter().map(|w| w.len() / 3).sum();
        if body_tris > 0 {
            // REMASTER (#292): the hand fixes, and the ones that no longer match
            let fixes = match &job.fixed {
                Some(r) if r.total > 0 => format!(", {}", r.summary()),
                _ => String::new(),
            };
            for f in job.fixed.iter().flat_map(|r| &r.stale) {
                eprintln!("{}: stale body fix {f}", car_names[k]);
            }
            notes[note] = format!("{} (generated body {body_tris} tris, wheels {wheel_tris}{fixes})", car_names[k]);
        }
        remaster_car_halos[at] = job.halos(&car_halos[at], &strips);
        remaster_cars[at] = verts;
    }

    // Scene objects (SceneName): lit like cars [I: the object renderer 0x4302f8 is assumed];
    // their env-mapped polygons (flags 0x60) take the EnvMapName layer (`PROP_ENV`).
    Progress::report(progress, Stage::Scene, 0.0)?;
    let prop_list = if holo {
        Default::default()
    } else {
        motor_sim::props::Props::load(root, &trk).unwrap_or_else(|e| {
            notes.push(format!("scene not loaded: {e}"));
            Default::default()
        })
    };
    let (mut props, mut prop_halos, mut prop_lamps) = (Vec::new(), Vec::new(), Vec::new());
    let prop_files = prop_list.meshes.iter().map(|pm| format!("{base}/Objects/{}.myo", pm.name));
    for file in prop_files.chain(FINISH_EMBLEMS.map(String::from)).filter(|_| !holo) {
        let (mut tris, mut halos) = (Vec::new(), Vec::new());
        if let Ok(m) = read(root, &file).and_then(|b| myo::parse(&b).map_err(|e| e.to_string())) {
            let shade = Shade { scale: object_scale, normals: Some((&m.vnormals, &m.planes)), tron: cheat.tron };
            for p in &m.polygons {
                let tex = m.texture(p).map(|(n, _)| (n, &mut layers, &lib));
                let start = tris.len();
                let retro = tex.as_ref().is_some_and(|t| is_retro(t.0));
                emit(&mut tris, p, &m.vertices, &m.planes, &m.uvbytes, tex, &to_view, &shade);
                if retro {
                    tris[start..].iter_mut().for_each(|v| v.env = RETRO_ENV);
                } else if matches!(p.ty, 8 | 9) && p.flags & 0x60 != 0 {
                    // CLASSIC: scene objects take the env-map path whatever the car shading
                    // (`0x4351a3`: object +0xac is 0, 0x47ab49)
                    tris[start..].iter_mut().for_each(|v| v.env = PROP_ENV);
                }
            }
            // the blinkers' glow: a halo record indexing the track's `Halo` list, like the
            // world's (blinkers0 → Haloyell, blinkers3 → Halored on track 8)
            halos = halo_insts(&m.halos, &track_halos);
            for h in &mut halos {
                h.pull = m.radius;
                // the dynamic light's colour
                h.tint = layers.data.get(h.layer as usize).map_or([1.0; 3], |px| dominant(px));
            }
        }
        tris.iter_mut().filter(|v| v.env != RETRO_ENV && v.env != PROP_ENV && v.env != SELF_LIT_ENV).for_each(|v| v.env = 0.0);
        // REMASTER headlamps
        let stem = Path::new(&file).file_stem().and_then(|s| s.to_str()).unwrap_or_default();
        let lamps = PROP_HEADLAMPS.iter().find(|d| d.name.eq_ignore_ascii_case(stem)).and_then(|d| {
            let layer = layers.file(root, PROP_LAMP_TEXTURE)?;
            let tint = layers.data.get(layer).map_or([1.0; 3], |px| dominant(px));
            let lamps = d
                .lamps
                .iter()
                .map(|c| HaloInst {
                    p0: Vec3::from(to_view([c[0] - PROP_LAMP_HALF.get(), c[1], c[2]])),
                    p1: Vec3::from(to_view([c[0] + PROP_LAMP_HALF.get(), c[1], c[2]])),
                    rgb: PROP_LAMP_RGB,
                    layer: layer as f32,
                    flags: 2,
                    texture: 0,
                    dir: Vec3::ZERO,
                    tint,
                    pull: 0.2,
                    strip: HaloStrip::None,
                })
                .collect();
            Some(PropLamps { forward: Vec3::from(to_view(d.forward)).normalize_or_zero(), lamps })
        });
        prop_lamps.push(lamps.unwrap_or_default());
        props.push(tris);
        prop_halos.push(halos);
    }
    let car_note = notes.join(", ");

    let byte3 = |v: [f32; 3]| v.map(|x| (x / 255.0).clamp(0.0, 1.0));
    let l = world.light;
    let mut look = Look {
        fog_colour: byte3(rgb("CueColour", [128.0; 3])),
        fog_start: fv(&trk, "CueStart").and_then(|v| v.first().copied()).unwrap_or(0.2),
        object_ambient: rgb("ObjectAmbient", [0.5; 3]),
        sun: l.sun.map(|c| c as f32 * l.sun_intensity / 255.0),
        sun_dir: to_view(Vec3::from(l.sun_dir).normalize_or(Vec3::Y).into()),
        specular: fv(&trk, "SpecularFactor").and_then(|v| v.first().copied()).unwrap_or(2.0),
        light_intensity: fv(&trk, "LightIntensity").and_then(|v| v.first().copied()).unwrap_or(1.0),
        env_layer,
        sky_top: byte3(rgb("TopColour", [64.0, 96.0, 160.0])),
        sky_bottom: byte3(rgb("BottomColour", [160.0; 3])),
        horizon: fv(&trk, "Horizon").and_then(|v| v.first().copied()).unwrap_or(0.25),
        darkness: 0.0,
        ground_y: 0.0,
        fog: crate::fog::load(root, track),
        horizon_band: [0.0; 3],
    };
    let sky_image = s(&trk, "BackgroundName").ok().filter(|_| !holo).and_then(|n| read(root, &n).ok()).and_then(|b| tga::decode(&b).ok()).map(|i| SkyImage {
        width: i.width,
        height: i.height,
        rgba: i.rgba,
    });
    let luma = |c: [f32; 3]| 0.299 * c[0] + 0.587 * c[1] + 0.114 * c[2];
    let sky_luma = match &sky_image {
        Some(i) if !i.rgba.is_empty() => i.rgba.as_chunks::<4>().0.iter().map(|p| luma([p[0], p[1], p[2]].map(|c| c as f32 / 255.0))).sum::<f32>() / (i.rgba.len() / 4) as f32,
        _ => (luma(look.sky_top) + luma(look.sky_bottom)) / 2.0,
    };
    let t = ((sky_luma - 0.08) / (0.30 - 0.08)).clamp(0.0, 1.0);
    look.darkness = 1.0 - t * t * (3.0 - 2.0 * t);
    look.horizon_band = match &sky_image {
        Some(i) if i.height > 0 && !i.rgba.is_empty() => {
            let rows = (i.height as usize * 85 / 100).min(i.height as usize - 1)..i.height as usize;
            let px = &i.rgba[rows.start * i.width as usize * 4..rows.end * i.width as usize * 4];
            let n = (px.len() / 4).max(1) as f32;
            let mut s = [0f32; 3];
            for p in px.as_chunks::<4>().0 {
                for k in 0..3 {
                    s[k] += p[k] as f32 / 255.0;
                }
            }
            s.map(|c| c / n)
        }
        _ => look.sky_bottom,
    };
    if !tris.is_empty() {
        let mut ys: Vec<f32> = tris.iter().map(|v| v.pos[1]).collect();
        let k = ys.len() / 20;
        look.ground_y = *ys.select_nth_unstable_by(k, f32::total_cmp).1;
    }
    let name = trk.get("TrackName").map(|e| e.values.join(" ")).unwrap_or_default();
    let pack = layers.finish_pack();
    let stats = format!(
        "track {track} '{name}': {} cells, {world_tris} world tris, {} scene objects, {} textures ({} missing), {} AI nodes; {car_note}",
        world.objects.len(),
        prop_list.props.len(),
        layers.data.len(),
        layers.missing,
        line.as_ref().map_or(0, |l| l.nodes.len()),
    ) + &pack.stats();
    if layers.data.is_empty() {
        layers.data.push(checker());
    }
    let tyre_layer = layers.tyre();
    let mut scene = Scene {
        tris,
        fill: Vec::new(),
        clouds: Vec::new(),
        water: Vec::new(),
        water_params: Vec::new(),
        water_hide: Vec::new(),
        trees: Vec::new(),
        tree_hide: Vec::new(),
        tree_sites: Vec::new(),
        tree_pack_layer: None,
        water_bed: Vec::new(),
        grass: Vec::new(),
        grass_params: Vec::new(),
        grass_edge: None,
        lines,
        layers: layers.data,
        pack,
        look,
        sky_image,
        // CHEAT tribute to tron, REMASTER Neon grid (#320): Goldbridge's night sky, graded cyan by the dome shader
        hdri: if holo { None } else { crate::sky::load(root, if cheat.tron { TRON_SKY_TRACK } else { track }) },
        world_halos,
        lamps,
        fx,
        car_halos,
        remaster_car_halos,
        point_lights: if holo { PointLights::default() } else { PointLights::new(&world, look.light_intensity) },
        view_distance: fv(&trk, "ViewDistance").and_then(|v| v.first().copied()).unwrap_or(180.0),
        start_pos,
        start_dir,
        cars,
        remaster_cars,
        car_lods,
        wheel_hubs,
        wheel_half_width,
        tyre_layer,
        props,
        prop_halos,
        prop_lamps,
        world_objects,
        world_tri_texture,
        world_textures: world.texnames.clone(),
        world_textures_sorted: sorted_names(&world.texnames),
        prop_names: prop_list.meshes.iter().map(|m| m.name.clone()).chain(["first", "second", "third"].map(String::from)).collect(),
        car_names,
        car_colours,
        car_models,
        object_scale,
        stats,
    };
    if let Some(h) = &scene.hdri {
        let sd = scene.look.sun_dir;
        scene.stats.push_str(&format!(
            "; sky HDRI {}x{}, its sun az {:.0} el {:.0} deg, the track's sun az {:.0} el {:.0} deg",
            h.width,
            h.height,
            h.sun_azimuth.to_degrees(),
            h.sun_elevation.to_degrees(),
            sd[0].atan2(-sd[2]).to_degrees(),
            sd[1].clamp(-1.0, 1.0).asin().to_degrees()
        ));
    }
    if holo {
        scene.stats.push_str("; hologram scene");
        return Ok(scene);
    }
    Progress::report(progress, Stage::Remaster, 0.0)?;
    let fill = procgen::fill::load(root, track);
    if !fill.water.is_empty() {
        let w = procgen::water::build(&fill.water, &scene);
        scene.stats.push_str(&format!("; water {} bodies, {} tris, bed {} tris", w.params.len(), w.verts.len() / 3, w.bed.len() / 3));
        (scene.water, scene.water_params, scene.water_hide, scene.water_bed) = (w.verts, w.params, w.hide, w.bed);
    }
    Progress::report(progress, Stage::Remaster, 0.4)?;
    // REMASTER (#390): the tree pack's textures, on every track, so the fill and the World Editor can place
    // models anywhere (layers are uploaded once; Classic never samples them)
    let pack_layers = procgen::trees::pack_layers();
    if !pack_layers.is_empty() && scene.layers.len() + pack_layers.len() <= MAX_LAYERS {
        scene.tree_pack_layer = Some(scene.layers.len());
        scene.layers.extend(pack_layers);
    }
    // REMASTER tree models (#389): the cards of the world's trees (before the fill: its models match them)
    let swap = procgen::trees::find(&scene.tris, &scene.world_objects, &scene.world_tri_texture, &scene.world_textures);
    scene.tree_sites = swap.sites.clone();
    scene.clouds = procgen::fill::clouds(&fill);
    if !scene.clouds.is_empty() {
        scene.stats.push_str(&format!("; {} clouds", scene.clouds.len()));
    }
    // REMASTER tree models (#389): the models in place of the cards
    if let (false, Some(first)) = (swap.sites.is_empty(), scene.tree_pack_layer) {
        let m = procgen::trees::mesh(&swap.sites, &scene.look, &scene.layers, first);
        if m.verts.is_empty() {
            scene.stats.push_str(&format!("; {} tree cards kept (no models)", swap.sites.len()));
        } else {
            scene.stats.push_str(&format!("; trees {} as models ({} tris)", swap.sites.len(), m.verts.len() / 3));
            (scene.trees, scene.tree_hide) = (m.verts, swap.hide);
        }
    } else if !swap.sites.is_empty() {
        scene.stats.push_str(&format!("; {} tree cards kept (no models)", swap.sites.len()));
    }
    // after the tree models: a fill's stamps (#466) leave out the cards they replace (`tree_hide`)
    if !fill.pieces.is_empty() {
        scene.fill = procgen::fill::mesh(&fill, &scene);
        scene.stats.push_str(&format!("; world fill {} pieces, {} tris", fill.pieces.len(), scene.fill.len() / 3));
    }
    // REMASTER grass: the fill's bodies, else the built-in rule (`Grass none` turns it off);
    // after the fill so its ground pieces can join
    Progress::report(progress, Stage::Remaster, 0.7)?;
    let grass_bodies = procgen::grass::track_bodies(&fill, &scene.world_textures);
    if !grass_bodies.is_empty() {
        let m = procgen::grass::build(&grass_bodies, &scene, &scene.fill);
        let per: Vec<String> = m.counts.iter().map(|(n, c)| format!("{n} {c}")).collect();
        let edge = m.edge.as_ref().map_or(String::new(), |e| format!(", edge map {}x{} at {} m", e.size[0], e.size[1], e.texel));
        scene.stats.push_str(&format!("; grass {} bodies ({} tris, {} drawn{edge})", m.params.len(), per.join(", "), m.verts.len() / 3));
        (scene.grass, scene.grass_params, scene.grass_edge) = (m.verts, m.params, m.edge);
    }
    Ok(scene)
}

/// CustomCarColours (`0x47b9c4`, called once per car at race load `0x45f36e`) [O]: Gouraud
/// polygons (types 8, 9) with flags & 0x60 whose face RGB equals the car file's ColourK get
/// CarColourK. The three tests run in order on the updated colour, as in the original. The
/// original walks the body object chain (+0xd4); the wheels are not painted [I].
pub fn repaint(m: &mut myo::Mesh, from: [[u8; 3]; 3], to: [[u8; 3]; 3]) {
    for p in m.polygons.iter_mut().filter(|p| matches!(p.ty, 8 | 9) && p.flags & 0x60 != 0) {
        for (f, t) in from.iter().zip(&to) {
            if p.rgba[0][..3] == f[..] {
                p.rgba[0][..3].copy_from_slice(t);
            }
        }
    }
}

/// REMASTER (#136, #351): per polygon, the finish of the zone of `to` whose colour `repaint` gives it: the zone of the
/// last of its in-order tests that matched (Gloss for a face outside every zone).
fn finish_faces(m: &myo::Mesh, from: [[u8; 3]; 3], to: CarPaint) -> Vec<Finish> {
    m.polygons
        .iter()
        .map(|p| {
            if !matches!(p.ty, 8 | 9) || p.flags & 0x60 == 0 {
                return Finish::Gloss;
            }
            let mut rgb = [p.rgba[0][0], p.rgba[0][1], p.rgba[0][2]];
            let mut zone = None;
            for (k, (f, t)) in from.iter().zip(&to.colours).enumerate() {
                if rgb == *f {
                    (rgb, zone) = (*t, Some(k));
                }
            }
            zone.map_or(Finish::Gloss, |k| to.finish[k])
        })
        .collect()
}

/// The paint zones `repaint` finds on a body: whether any face it tests has the colour ColourK.
fn zones_of(m: &myo::Mesh, own: [[u8; 3]; 3]) -> [bool; 3] {
    let faces: Vec<&[u8]> = m.polygons.iter().filter(|p| matches!(p.ty, 8 | 9) && p.flags & 0x60 != 0).map(|p| &p.rgba[0][..3]).collect();
    own.map(|c| faces.iter().any(|f| *f == c))
}

/// The paint zones car `id` has (08-missing-features 8.5 [O]: on the shipped cars only the Kobra X3,
/// Adder Mk2 and ASC II have a third); all three when the body cannot be read.
pub fn paint_zones(root: &Path, id: u32) -> [bool; 3] {
    body_myo(root, id).map_or([true; 3], |(m, c)| zones_of(&m, own_colours(&c)))
}

/// The bounding sphere of car `id`'s body (the MYO header's centre, in mesh space, and radius; m),
/// to frame the car in a menu preview; a 2.4 m sphere round the origin when the body cannot be read.
pub fn body_sphere(root: &Path, id: u32) -> ([f32; 3], f32) {
    body_myo(root, id).map_or(([0.0, 0.7, 0.0], 2.4), |(m, _)| (to_view(m.center), m.radius))
}

/// REMASTER menu car preview: the body's footprint on the ground plane, view space: (centre x, centre z, half
/// extent x, half extent z) of its vertices' box (the Car screen's floor and contact shadow).
pub fn body_footprint(root: &Path, id: u32) -> [f32; 4] {
    let Ok((m, _)) = body_myo(root, id) else { return [0.0, 0.0, 1.0, 2.2] };
    let (mut lo, mut hi) = ([f32::MAX; 2], [f32::MIN; 2]);
    for v in &m.vertices {
        let q = to_view(*v);
        for (k, a) in [q[0], q[2]].into_iter().enumerate() {
            lo[k] = lo[k].min(a);
            hi[k] = hi[k].max(a);
        }
    }
    if lo[0] > hi[0] {
        return [0.0, 0.0, 1.0, 2.2];
    }
    [(lo[0] + hi[0]) * 0.5, (lo[1] + hi[1]) * 0.5, (hi[0] - lo[0]) * 0.5, (hi[1] - lo[1]) * 0.5]
}

/// Car `id`'s body (`{FileName}0.MYO`) and its car file.
fn body_myo(root: &Path, id: u32) -> Res<(myo::Mesh, Config)> {
    let c = config(root, &format!("Game/car{id:02}.car"))?;
    let m = myo::parse(&read(root, &format!("{}0.MYO", s(&c, "FileName")?))?).map_err(|e| e.to_string())?;
    Ok((m, c))
}

/// The car file's Colour1..3 of car `id` (the stock paint), as `own_colours`; black when the file is missing.
pub fn car_colours(root: &Path, id: u32) -> [[u8; 3]; 3] {
    config(root, &format!("Game/car{id:02}.car")).map_or([[0; 3]; 3], |c| own_colours(&c))
}

/// The car file's Colour1..3: the face colours CustomCarColours replaces.
fn own_colours(c: &Config) -> [[u8; 3]; 3] {
    ["Colour1", "Colour2", "Colour3"].map(|k| match fv(c, k) {
        Some(v) if v.len() >= 3 => [v[0] as u8, v[1] as u8, v[2] as u8],
        _ => [0; 3],
    })
}

/// Meshes per car: body + 4 wheels.
pub const CAR_MESHES: usize = 5;
/// The wheel objects sit at PhyWheel x times this (0x40cb59, 0x4cf3a3 = 1.25).
pub const WHEEL_X_SCALE: f32 = 1.25;

/// Body LOD0 and its 4 wheels (FL, FR, BR, BL) in car-local view space; wheel meshes are
/// left centred on their own origin. Returns the meshes, the REMASTER generated ones in the
/// same order (empty: none), the wheel hubs and radius.
///
/// Rest placement for static display (the garage): the game's wheel position
/// (0x40cb46: PhyWheel x * 1.25, y and z from PhyWheel, plus suspension travel) with the hub
/// at the wheel radius above the model's ground plane. In races `drive::Session` places the
/// wheels from the live suspension.
type CarMeshes = (Vec<Vec<Vertex>>, Vec<Vec<Vertex>>, ([Vec3; 4], f32, [f32; 4]), String, Option<BodyJob>, [Vec<Vertex>; 3]);

#[allow(clippy::too_many_arguments)]
fn build_car(
    root: &Path,
    car: u32,
    halos: &mut Vec<HaloInst>,
    layers: &mut Layers,
    lib: &TexLib,
    scale: [f32; 3],
    paint: Option<CarPaint>,
    tron: bool,
) -> Res<CarMeshes> {
    let c = config(root, &format!("Game/car{car:02}.car"))?;
    let body = s(&c, "FileName")?;
    let halo_tex = layers.halo_list(root, &c);
    let mut body_mesh = myo::parse(&read(root, &format!("{body}0.MYO"))?).map_err(|e| format!("{body}: {e}"))?;
    // REMASTER: the part tags the generator keys on are the original face colours, before
    // CustomCarColours repaints them (the Titan's trim, Colour2, is what its intakes and carbon
    // bumper are cut from)
    let tags: Vec<[u8; 3]> = body_mesh.polygons.iter().map(|p| [p.rgba[0][0], p.rgba[0][1], p.rgba[0][2]]).collect();
    // REMASTER (#136, #351): the finish of each painted face, found before the repaint
    let finish = |m: &myo::Mesh| paint.map_or(Vec::new(), |p| finish_faces(m, own_colours(&c), p));
    let body_finish = finish(&body_mesh);
    if let Some(to) = paint {
        repaint(&mut body_mesh, own_colours(&c), to.colours);
    }
    *halos = halo_insts(&body_mesh.halos, &halo_tex);
    // `finish`: per polygon, its paint finish (empty: all gloss)
    let emit_mesh = |m: &myo::Mesh, finish: &[Finish], layers: &mut Layers| -> Vec<Vertex> {
        let mut tris = Vec::new();
        let shade = Shade { scale, normals: Some((&m.vnormals, &m.planes)), tron };
        for (i, p) in m.polygons.iter().enumerate() {
            let tex = m.texture(p).map(|(n, _)| (n, &mut *layers, lib));
            let start = tris.len();
            emit(&mut tris, p, &m.vertices, &m.planes, &m.uvbytes, tex, &to_view, &shade);
            if let Some(tag) = finish.get(i).and_then(|f| f.tag()) {
                tris[start..].iter_mut().for_each(|v| v.env = tag);
            }
        }
        tris
    };
    let mut meshes = vec![emit_mesh(&body_mesh, &body_finish, layers)];
    // REMASTER: the body refined from the MYO cage (09-remaster 9.22), after the repaint; built
    // once the wheel hubs are known (the arches are rounded round them)
    let mut generated = vec![Vec::new()];
    let mut hubs = [Vec3::ZERO; 4];
    let mut half_widths = [0.15; 4];
    let mut radius = 0.35;
    // the ground at rest, above the model origin: the wheels hang from the physics origin
    // (`MODEL_OFFSET` below the model's), so the tyres stand this far up the model's y
    let ground_y = motor_sim::car::CarParams::from_config(&c).map(|p| p.rest_ground(motor_sim::world::GRAVITY.get()) - motor_sim::car::MODEL_OFFSET.get()).unwrap_or(0.0);
    let wheel_keys = ["PhyWheelFL", "PhyWheelFR", "PhyWheelBR", "PhyWheelBL"];
    for (i, key) in wheel_keys.iter().enumerate() {
        let (Some(m), Some(p)) = (wheel_myo(root, &c, i), fv(&c, key).filter(|p| p.len() >= 3)) else {
            meshes.push(Vec::new());
            generated.push(Vec::new());
            continue;
        };
        let (r, half_w) = wheel_size(&m);
        radius = r;
        half_widths[i] = half_w;
        hubs[i] = Vec3::from(to_view([p[0] * WHEEL_X_SCALE, r + ground_y, p[2]]));
        meshes.push(emit_mesh(&m, &[], layers));
        // REMASTER: the generated wheel at the measured size, with this wheel's alloy
        let (face, side) = wheel_face(&m, layers, lib, scale, i);
        let tyre = layers.tyre();
        // REMASTER: a rim file from `rimshop` (<install>/remaster/rims/carNN.rim) or the built-in
        let rim = procgen::rimfile::load(root, car).zip(face).map(|(style, f)| {
            let px = &layers.data[f.layer as usize];
            let look = procgen::analyse(r, &|y, z| texel(px, f.fit, y, z), Some(style.spokes));
            procgen::Rim { style, look }
        });
        generated.push(procgen::wheel(&procgen::WheelSpec { radius: r, half_width: half_w, side, face, rim, scale, tyre }));
    }
    // REMASTER: the body recipe's job; `load_parts` builds the cars' bodies side by side (`run_bodies`)
    // CLASSIC detail levels OBJ1, OBJ2 (0x40b888): painted like OBJ0 (the original's painter walks the
    // chain, 0x47b9c4); halos come from OBJ0, the records are identical on all four levels [O]
    let lods = [1, 2, 3].map(|i| {
        let mut m = read(root, &format!("{body}{i}.MYO")).ok().and_then(|b| myo::parse(&b).ok())?;
        let lod_finish = finish(&m);
        if let Some(to) = paint {
            repaint(&mut m, own_colours(&c), to.colours);
        }
        Some(emit_mesh(&m, &lod_finish, layers))
    });
    let lods = lods.map(Option::unwrap_or_default);
    let job = procgen::bodyfile::load(root, car).map(|st| BodyJob::prepare(&body_mesh, &tags, &body_finish, st, procgen::bodyfix::load(root, car), &hubs, radius, layers, lib, scale));
    let name = format!("car{car:02} '{}'", c.get("CarName").map(|e| e.values.join(" ")).unwrap_or_default());
    Ok((meshes, generated, (hubs, radius, half_widths), name, job, lods))
}

/// REMASTER (#478) what a car's model file gives: its body, the wheels it has, its lamps' halos.
struct ImportedCar {
    body: Vec<Vertex>,
    /// FL, FR, BR, BL centred on their hubs as `build_car`'s; None: the generated wheel stays
    wheels: [Option<Vec<Vertex>>; 4],
    /// for `Scene::remaster_car_halos` (empty: the original's)
    halos: Vec<HaloInst>,
    /// for the stats line: `, hub error 0.4 mm` and the warnings
    note: String,
}

/// REMASTER (#478): what loading a car's model file found, for motorview's Model file section and
/// `--check-model`. Made whenever the file exists, also when `carmodel::source` leaves it unused.
#[derive(Clone, Debug, Default)]
pub struct ModelCheck {
    pub path: PathBuf,
    /// the file did not load or fit (the recipe takes over); everything below is then empty
    pub error: Option<String>,
    pub body_tris: usize,
    pub wheel_tris: usize,
    /// model units to metres
    pub scale: f32,
    /// the largest miss of a wheel node from its hub, mm; None: fitted to the original body's box
    pub hub_err_mm: Option<f32>,
    /// FL, FR, BR, BL: the file has a wheel mesh there
    pub wheels_from_file: [bool; 4],
    /// lamp spots found (`carmodel::lamps`)
    pub lamps: usize,
    /// material names no rule knows (not `rubber` or `trim_*`, which the contract names): they keep their base colour
    pub plain_materials: Vec<String>,
    /// the fit's, the wheels', the loader's and the lamps' warnings, and the triangle budget's
    pub warnings: Vec<String>,
}

impl ModelCheck {
    /// One fact per line, for `--check-model`.
    pub fn lines(&self) -> Vec<String> {
        let mut out = vec![format!("file: {}", self.path.display())];
        if let Some(e) = &self.error {
            out.push(format!("error: {e}"));
            return out;
        }
        out.push(format!("triangles: body {}, wheels {}", self.body_tris, self.wheel_tris));
        out.push(format!("scale: {:.5} (model units to metres)", self.scale));
        out.push(match self.hub_err_mm {
            Some(e) => format!("fit: wheel nodes, hub error {e:.1} mm"),
            None => "fit: the original body's box (no usable wheel nodes)".into(),
        });
        let have: Vec<&str> = self.wheels_from_file.iter().zip(procgen::carmodel::WHEEL_NODES).filter(|(h, _)| **h).map(|(_, n)| &n["wheel_".len()..]).collect();
        out.push(format!("wheels from the file: {}", if have.is_empty() { "none (generated ones)".to_string() } else { have.join(" ") }));
        out.push(format!("lamps: {}", self.lamps));
        if !self.plain_materials.is_empty() {
            out.push(format!("plain materials (base colour only): {}", self.plain_materials.join(", ")));
        }
        out.extend(self.warnings.iter().map(|w| format!("warning: {w}")));
        out
    }
}

/// REMASTER (#478): car `car` from its model file (`procgen::carmodel`), fitted to the wheel hubs of
/// `wheels` (`build_car`'s hubs, radius, half-widths; else to `body`, the original OBJ0 mesh's box),
/// painted in `livery`, its lamps' halos made from the car's own `halos`.
/// - no file: (None, None)
/// - it does not load (reported; the recipe takes over): (None, the check with its error)
/// - `carmodel::source` is not Imported: (None, the check): the file is read for the check only
/// - else: (the car, the check)
#[allow(clippy::too_many_arguments)]
fn imported_car(root: &Path, car: u32, body: &[Vertex], wheels: ([Vec3; 4], f32, [f32; 4]), livery: CarPaint, halos: &[HaloInst], layers: &mut Layers, scale: [f32; 3]) -> (Option<ImportedCar>, Option<ModelCheck>) {
    use procgen::carmodel;
    let (hubs, radius, half_widths) = wheels;
    let Some(path) = carmodel::path(root, car) else { return (None, None) };
    let failed = |e: String| {
        eprintln!("{}: {e}", path.display());
        (None, Some(ModelCheck { path: path.clone(), error: Some(e), ..Default::default() }))
    };
    let model = match carmodel::load(&path) {
        Ok(m) => m,
        Err(e) => return failed(e),
    };
    let body_box = body.iter().map(|v| Vec3::from(v.pos)).fold(None, |b: Option<(Vec3, Vec3)>, p| Some(b.map_or((p, p), |(lo, hi)| (lo.min(p), hi.max(p)))));
    let fit = match carmodel::fit(&model, &hubs, body_box) {
        Ok(f) => f,
        Err(e) => return failed(e),
    };
    let use_it = carmodel::source() == carmodel::CarSource::Imported;
    let paint = carmodel::Paint { colours: livery.colours.map(|c| rgba_obj([c[0], c[1], c[2], 255], scale)), env: livery.finish.map(|f| f.tag().unwrap_or(1.0)) };
    // not used: the layers stay as without the file
    let layer_of: Vec<Option<f32>> = if use_it { model.images.iter().enumerate().map(|(i, m)| layers.image(format!("glb:car{car:02}:{i}"), m)).collect() } else { Vec::new() };
    let layer = |i: usize| layer_of.get(i).copied().flatten();
    let verts = carmodel::vertices(&model, &fit, &paint, scale, &layer);
    let mut warnings = fit.warnings.clone();
    // a corner the car has no wheel at keeps nothing
    let wheels: [Option<Vec<Vertex>>; 4] = std::array::from_fn(|i| {
        let sim = carmodel::SimWheel { radius, half_width: half_widths[i] };
        let (v, w) = carmodel::wheel(&model, &fit, i, sim, &paint, scale, &layer).filter(|_| hubs[i] != Vec3::ZERO)?;
        warnings.extend(w);
        Some(v)
    });
    let spots = carmodel::lamps(&model, &fit);
    let (halos, lamp_notes) = imported_halos(&spots, halos);
    let mut note = fit.hub_err.map_or_else(|| ", fitted to the original body's box".to_string(), |e| format!(", hub error {:.1} mm", e * 1000.0));
    warnings.extend(model.notes.iter().chain(&lamp_notes).cloned());
    for w in &warnings {
        eprintln!("{}: {w}", path.display());
        note += &format!(", {w}");
    }
    let wheel_tris: usize = wheels.iter().flatten().map(|w| w.len() / 3).sum();
    let body_tris = verts.len() / 3;
    let mut check_warnings = warnings;
    if body_tris + wheel_tris > carmodel::CAR_TRI_BUDGET {
        check_warnings.push(format!("over the {} triangle budget", carmodel::CAR_TRI_BUDGET));
    }
    let check = ModelCheck {
        path: path.clone(),
        error: None,
        body_tris,
        wheel_tris,
        scale: fit.scale,
        hub_err_mm: fit.hub_err.map(|e| e * 1000.0),
        wheels_from_file: std::array::from_fn(|i| wheels[i].is_some()),
        lamps: spots.len(),
        plain_materials: model.plain_materials.iter().filter(|n| !carmodel::expected_plain(n)).cloned().collect(),
        warnings: check_warnings,
    };
    if !use_it {
        return (None, Some(check));
    }
    eprintln!("car{car:02}: model from {}", path.display());
    (Some(ImportedCar { body: verts, wheels, halos, note }), Some(check))
}

/// REMASTER (#478): the halos of an imported car: per group (front, rear: view z of the middle, as the
/// renderer splits them), one at each of its lamps when the file has some, else the car's own. A lamp's
/// halo is the nearest own halo of its group, moved onto the lamp's centre and laid along x (keeps its
/// texture, colour and length, so it looks as before) [design choice]. Empty when no lamp moved one.
fn imported_halos(lamps: &[procgen::carmodel::LampSpot], own: &[HaloInst]) -> (Vec<HaloInst>, Vec<String>) {
    use procgen::carmodel::Lamp;
    let mid = |h: &HaloInst| (h.p0 + h.p1) * 0.5;
    let (mut out, mut notes, mut any) = (Vec::new(), Vec::new(), false);
    for (rear, name) in [(false, "front"), (true, "rear")] {
        let group: Vec<&HaloInst> = own.iter().filter(|h| (mid(h).z > 0.0) == rear).collect();
        let spots: Vec<_> = lamps.iter().filter(|l| (l.kind != Lamp::Front) == rear).collect();
        let moved: Vec<HaloInst> = spots
            .iter()
            .filter_map(|l| {
                let t = group.iter().min_by(|a, b| mid(a).distance(l.centre).total_cmp(&mid(b).distance(l.centre)))?;
                let half = Vec3::X * (t.p1 - t.p0).length() * 0.5;
                Some(HaloInst { p0: l.centre - half, p1: l.centre + half, strip: HaloStrip::None, ..**t })
            })
            .collect();
        if !spots.is_empty() && moved.is_empty() {
            notes.push(format!("{name} lamps without a {name} halo of the car's to copy: the car's own stay"));
        }
        if moved.is_empty() {
            out.extend(group.into_iter().copied());
        } else {
            any = true;
            out.extend(moved);
        }
    }
    if !any {
        out.clear();
    }
    (out, notes)
}

/// Wheel `i`'s (FL, FR, BR, BL) LOD0 mesh of a car file.
fn wheel_myo(root: &Path, c: &Config, i: usize) -> Option<myo::Mesh> {
    let prefix = c.get(&format!("Wheel{}", i + 1)).and_then(|w| w.values.first().cloned())?;
    read(root, &format!("{prefix}0.MYO")).ok().and_then(|b| myo::parse(&b).ok())
}

/// A wheel mesh's radius (largest y) and tyre half-width (largest |x|).
fn wheel_size(m: &myo::Mesh) -> (f32, f32) {
    (m.vertices.iter().map(|v| v[1]).fold(0.0, f32::max), m.vertices.iter().map(|v| v[0].abs()).fold(0.0, f32::max))
}

/// REMASTER rimshop's car preview (#337 phase 5): one car without its track's world, its four
/// wheels kept as inputs so a rim style can be fitted to them again and again (`wheels`).
pub struct ShowroomCar {
    /// the car file's CarName
    pub name: String,
    /// the original body (OBJ0), car-local view space, unpainted
    pub body: Vec<Vertex>,
    /// the generated body (`remaster/cars/carNN.body`), lowered as in a race; empty without a recipe
    pub generated: Vec<Vertex>,
    /// RGBA8, TEX x TEX per layer (`Vertex::layer` indexes it)
    pub layers: Vec<Vec<u8>>,
    /// wheel centres at rest, as `Scene::wheel_hubs`
    pub hubs: [Vec3; 4],
    /// FL, FR, BR, BL; None where the car has no such wheel
    pub wheels: [Option<ShowroomWheel>; 4],
    /// the original body's light halos (car-local view space)
    pub halos: Vec<HaloInst>,
    /// REMASTER (#478): what loading the car's model file found (None: no file)
    pub model: Option<ModelCheck>,
}

/// What `build_car` feeds `procgen::wheel` for one wheel, less the rim style.
#[derive(Clone, Copy, Debug)]
pub struct ShowroomWheel {
    pub radius: f32,
    pub half_width: f32,
    pub side: f32,
    pub face: Option<procgen::WheelFace>,
    pub tyre: Option<f32>,
    pub scale: [f32; 3],
}

impl ShowroomCar {
    /// The four generated wheels with `style`, each moved to its hub. As `build_car`: the spoke angle
    /// and the colours are read off the car's alloy texture; a wheel without a face gets no rim.
    pub fn wheels(&self, style: &procgen::RimStyle) -> Vec<Vertex> {
        let mut out = Vec::new();
        for (w, hub) in self.wheels.iter().zip(self.hubs) {
            let Some(w) = w else { continue };
            let rim = w.face.map(|f| {
                let px = &self.layers[f.layer as usize];
                let look = procgen::analyse(w.radius, &|y, z| texel(px, f.fit, y, z), Some(style.spokes));
                procgen::Rim { style: style.clone(), look }
            });
            let spec = procgen::WheelSpec { radius: w.radius, half_width: w.half_width, side: w.side, face: w.face, rim, scale: w.scale, tyre: w.tyre };
            out.extend(procgen::wheel(&spec).into_iter().map(|mut v| {
                v.pos = (Vec3::from(v.pos) + hub).to_array();
                v
            }));
        }
        out
    }
}

/// The one way a car is named in motorview (#433): its number and the game's own name, `01 Kobra X3` (the number alone
/// when the file has no name). The top bar's car list shows the same.
pub fn car_label(id: u32, name: &str) -> String {
    if name.is_empty() {
        format!("{id:02}")
    } else {
        format!("{id:02} {name}")
    }
}

/// A car file's CarName ("carNN" when unreadable).
pub fn car_name(root: &Path, car: u32) -> String {
    config(root, &format!("Game/car{car:02}.car")).ok().and_then(|c| c.get("CarName").map(|e| e.values.join(" "))).unwrap_or_else(|| format!("car{car:02}"))
}

/// REMASTER rimshop's car preview (#337 phase 5): car `car` with the textures of `track`'s packs
/// (the cars' art ships in every track's packs; the game's Car screen uses track 1), its
/// ObjectBrightness and 3DAccBrightness, no world. Original textures only (no texture pack).
pub fn load_showroom(root: &Path, track: u32, car: u32) -> Res<ShowroomCar> {
    with_car_textures(root, track, |lib, layers, scale| {
        let mut halos = Vec::new();
        let (mut meshes, _, (hubs, radius, half_widths), _, job, _) = build_car(root, car, &mut halos, layers, lib, scale, None, false)?;
        let c = config(root, &format!("Game/car{car:02}.car"))?;
        // REMASTER (#478): a model file's body wins over the recipe (rimshop edits the generated wheels: those stay)
        let (imp, model) = imported_car(root, car, &meshes[0], (hubs, radius, half_widths), CarPaint::gloss(own_colours(&c)), &halos, layers, scale);
        let generated = match imp {
            Some(imp) => imp.body,
            None => job.filter(|_| procgen::carmodel::source() != procgen::carmodel::CarSource::Original).map(|j| run_bodies(vec![j]).swap_remove(0).1).unwrap_or_default(),
        };
        let wheels = std::array::from_fn(|i| {
            let m = wheel_myo(root, &c, i).filter(|_| hubs[i] != Vec3::ZERO)?;
            let (radius, half_width) = wheel_size(&m);
            // the layers are cached by name: the same indices as build_car's
            let (face, side) = wheel_face(&m, layers, lib, scale, i);
            Some(ShowroomWheel { radius, half_width, side, face, tyre: layers.tyre(), scale })
        });
        if layers.data.is_empty() {
            layers.data.push(checker());
        }
        Ok(ShowroomCar { name: car_name(root, car), body: std::mem::take(&mut meshes[0]), generated, layers: std::mem::take(&mut layers.data), hubs, wheels, halos, model })
    })
}

/// `f` with the texture packs of `track` (the cars' art ships in every track's packs), their
/// layers and the track's ObjectBrightness: a car built without its world.
fn with_car_textures<R>(root: &Path, track: u32, f: impl FnOnce(&TexLib, &mut Layers, [f32; 3]) -> Res<R>) -> Res<R> {
    let trk = config(root, &format!("Game/Track{track}.trk"))?;
    let base = s(&trk, "BasePath")?;
    let pack_bytes: Vec<Vec<u8>> = ["tex1.pdi", "tex2.pdi"].iter().filter_map(|n| read(root, &format!("{base}/Packed/{n}")).ok()).collect();
    let lib = TexLib { packs: pack_bytes.iter().filter_map(|b| pdi::parse(b).ok()).collect() };
    let mut layers = Layers::new(root, &trk, track, false);
    let scale = trk_rgb(&trk, "ObjectBrightness", [1.0; 3]);
    f(&lib, &mut layers, scale)
}

/// REMASTER (#292): a car's generated body for the OBJ export (`procgen::bodyobj`).
pub struct BodyExport {
    /// the car file's CarName
    pub name: String,
    /// car-local view space at the MYO body's height: the recipe's ride-height `drop` is left
    /// out, so a hand fix keyed to a position survives a changed drop
    pub verts: Vec<Vertex>,
    pub parts: BodyParts,
}

/// REMASTER (#292): the parts of a generated body, one per triangle.
#[derive(Default)]
pub struct BodyParts {
    /// `<class>_<r>_<g>_<b>` for the refined cage's faces (`part_name`), else the added part
    /// (`lamps`, `vents`, `splitter`, `wing`, `fins`, `exhaust`)
    pub names: Vec<String>,
    /// per triangle, an index into `names`
    pub tri: Vec<u16>,
}

impl BodyParts {
    fn id(&mut self, name: String) -> u16 {
        match self.names.iter().position(|n| *n == name) {
            Some(i) => i as u16,
            None => {
                self.names.push(name);
                (self.names.len() - 1) as u16
            }
        }
    }

    /// the next `verts / 3` triangles are part `name`
    fn add(&mut self, name: &str, verts: usize) {
        if verts > 0 {
            let i = self.id(name.to_string());
            self.tri.extend(std::iter::repeat_n(i, verts / 3));
        }
    }
}

/// REMASTER (#292): the part of a refined cage face, by what the recipe made of it, and its
/// colour tag as the recipes write it (`r,g,b` before the repaint).
fn part_name(f: &procgen::body::Face, st: &procgen::body::BodyStyle) -> String {
    let class = if f.env == procgen::CARBON_ENV {
        "carbon"
    } else if st.glass.as_ref().is_some_and(|g| g.rgb == f.rgb) {
        "glass"
    } else if f.rgb == procgen::body::RUBBER_RGB {
        "rubber"
    } else if st.mirror_glass == Some(f.rgb) {
        "mirror"
    } else if st.lamps.iter().any(|l| l.housing.rgb == f.rgb) {
        "lamp"
    } else if st.vents.iter().any(|v| v.housing.rgb == f.rgb) {
        "vent"
    } else if (0.5..2.5).contains(&f.env) {
        // the paint band of `Vertex::env` (gloss 1, pearl 1.75, metallic 2)
        "paint"
    } else {
        "trim"
    };
    format!("{class}_{}_{}_{}", f.rgb[0], f.rgb[1], f.rgb[2])
}

/// REMASTER (#292): car `car`'s generated body and its parts, built as the game builds it (the
/// recipe from `bodyfile`, live or `--cars-dir` included) with `track`'s textures, which leave
/// the shape alone. An error if the car has no recipe.
pub fn export_body(root: &Path, track: u32, car: u32) -> Res<BodyExport> {
    with_car_textures(root, track, |lib, layers, scale| {
        let (.., job, _) = build_car(root, car, &mut Vec::new(), layers, lib, scale, None, false)?;
        let mut job = job.ok_or_else(|| format!("car {car} has no body recipe (remaster/cars/car{car:02}.body)"))?;
        let (verts, _, parts) = job.run_parts(false);
        Ok(BodyExport { name: car_name(root, car), verts, parts })
    })
}

/// REMASTER (09-remaster 9.22): a car's body refined from the MYO cage after the repaint, its
/// lamps, vents, splitter and exhausts, at the lowered ride height. Built once the wheel hubs are
/// known: the arches are rounded round them. `prepare` takes what needs the texture layers (in
/// the car's turn, so the layer order stays); `run` is pure geometry and takes most of a car's
/// load time, so `run_bodies` does several cars at once.
struct BodyJob {
    faces: Vec<procgen::body::Face>,
    st: procgen::body::BodyStyle,
    game_hubs: Vec<[f32; 3]>,
    radius: f32,
    carbon: Option<f32>,
    segs: Vec<procgen::lamps::HaloSeg>,
    /// the cage's halo records' ends (game space), which the lit strips refer to
    halo_src: Vec<([f32; 3], [f32; 3])>,
    /// REMASTER (#292): the car's hand fixes (`procgen::bodyfix`), applied by `run_parts` as drawn
    fixes: Vec<procgen::bodyfix::Fix>,
    /// what the fixes did at the last `run_parts` as drawn
    fixed: Option<procgen::bodyfix::Report>,
}

impl BodyJob {
    /// `tags`: the cage polygons' original RGBs (before CustomCarColours), which mark the parts;
    /// `finish`: per polygon, its paint finish (#136, #351; empty: all gloss).
    #[allow(clippy::too_many_arguments)]
    fn prepare(body_mesh: &myo::Mesh, tags: &[[u8; 3]], finish: &[Finish], st: procgen::body::BodyStyle, fixes: Vec<procgen::bodyfix::Fix>, hubs: &[Vec3; 4], radius: f32, layers: &mut Layers, lib: &TexLib, scale: [f32; 3]) -> BodyJob {
        let mut faces = cage_faces(body_mesh, tags, layers, lib, scale);
        faces.iter_mut().zip(finish).for_each(|(f, fin)| {
            if let Some(tag) = fin.tag() {
                f.env = tag;
            }
        });
        procgen::body::drop_repeats(&mut faces);
        // REMASTER (#177): the generator works with the cage's mirror plane on x = 0 (`centre`,
        // [O] the Avalanche's is x +3 mm); `run` moves the result back. `halo_src` stays where it
        // was: `halos` finds the car's records by it.
        let left = |mut p: [f32; 3]| {
            p[0] -= st.centre;
            p
        };
        faces.iter_mut().for_each(|f| f.pts.iter_mut().for_each(|p| *p = left(*p)));
        let game_hubs = hubs.iter().filter(|h| **h != Vec3::ZERO).map(|h| left(to_view(h.to_array()))).collect();
        let carbon = if st.uses_carbon() { layers.carbon() } else { None };
        let segs = body_mesh.halos.iter().map(|h| procgen::lamps::HaloSeg { p0: left(h.p0), p1: left(h.p1), rgb: h.rgb }).collect();
        let halo_src = body_mesh.halos.iter().map(|h| (h.p0, h.p1)).collect();
        BodyJob { faces, st, game_hubs, radius, carbon, segs, halo_src, fixes, fixed: None }
    }

    /// The body's triangles and the lit patterns' strips.
    fn run(&mut self) -> (Vec<Vertex>, Vec<procgen::lamps::LampStrip>) {
        let (verts, strips, _) = self.run_parts(true);
        (verts, strips)
    }

    /// `run`, and each triangle's part (#292); `as_drawn`: at the recipe's ride height and with
    /// the car's hand fixes (`procgen::bodyfix`), else at the MYO body's height and unfixed (the
    /// OBJ export: Blender's Basis is the raw shape, the add-on applies the `.fix` itself).
    fn run_parts(&mut self, as_drawn: bool) -> (Vec<Vertex>, Vec<procgen::lamps::LampStrip>, BodyParts) {
        let (faces, st, carbon) = (&mut self.faces, &self.st, self.carbon);
        procgen::cuts::mirror_copies(faces, &st.mirror_copies);
        // the slices before the pushes: a face cut first and moved then bends only at the cut (the
        // ASC's tail: the red corners' inner strips become the side walls of the set-back centre)
        procgen::cuts::slice(faces, &st.slices);
        procgen::cuts::push(faces, &st.pushes);
        if let Some(rgb) = st.mirror_glass {
            faces.iter_mut().filter(|f| f.rgb == rgb).for_each(|f| f.env = procgen::METAL_ENV);
        }
        procgen::body::round_arches(faces, &self.game_hubs, self.radius, st);
        // REMASTER (#174 phase 4): before the curved zones, which leave its faces alone (they
        // carry their own `smooth_deg`), and with the deck still flat under the struts. After
        // the arches (#177 phase 3): [O] the Avalanche's wing sits 8-11 cm over its deck, its
        // loft's undersides 0.63 m from the rear hubs and facing them, and its 244 faces
        // outnumbered the arch tunnel's 6 black ones, so the arch took the wing for its tunnel
        // and stayed angular.
        procgen::aero::foils(faces, &st.foils);
        // REMASTER (#388): after the arches (the channel stays inside the tunnels' walls), before
        // the curved zones, which then round the cut rear bumper like the rest of it
        procgen::diffuser::diffusers(faces, &st.diffusers);
        procgen::smooth::smooth(faces, &self.game_hubs, self.radius, st);
        // after the arches: the plates' undersides over a wheel would pass for arch ceilings
        procgen::cuts::thicken(faces, &st.thickens);
        procgen::cuts::chamfer(faces, &st.chamfers);
        procgen::cuts::panels(faces, &st.panels);
        procgen::cuts::paint(faces, &st.paints);
        if let Some(l) = carbon {
            procgen::cuts::carbon(faces, &st.carbon, l);
        }
        if let Some(g) = &st.glass {
            procgen::body::inset_glass(faces, g, &st.flat_glass);
        }
        for l in &st.lamps {
            procgen::body::inset_glass(faces, &l.housing, &[]);
            procgen::lamps::housing_floor(faces, l);
        }
        let recessed = procgen::vents::recess(faces, &st.vents);
        let faces: &[procgen::body::Face] = faces;
        let mut parts = BodyParts::default();
        // per triangle the part of its face, named once per face
        let mut label = |faces: &[procgen::body::Face], src: Vec<usize>| {
            let mut ids: HashMap<usize, u16> = HashMap::new();
            let tri = src.into_iter().map(|f| *ids.entry(f).or_insert_with(|| parts.id(part_name(&faces[f], st)))).collect();
            parts.tri = tri;
        };
        let mut out = if st.refine > 0.0 || !st.warps.is_empty() {
            // REMASTER (#388): the bulges carry the refine's cut points, so they come after it
            let mut cage = if st.refine > 0.0 { procgen::smooth::refine(faces, st.refine, st.refine_seams) } else { faces.to_vec() };
            procgen::cuts::warp(&mut cage, &st.warps);
            let (out, src) = procgen::body::body_parts(&cage, st);
            label(&cage, src);
            out
        } else {
            let (out, src) = procgen::body::body_parts(faces, st);
            label(faces, src);
            out
        };
        let body_len = out.len();
        let mut extra = |out: &mut Vec<Vertex>, name: &str, v: Vec<Vertex>| {
            parts.add(name, v.len());
            out.extend(v);
        };
        let (lit, mut strips) = procgen::lamps::lamps(faces, &self.segs, &st.lamps);
        extra(&mut out, "lamps", lit);
        extra(&mut out, "vents", procgen::vents::vents(faces, &st.vents, &recessed));
        if let Some(s) = &st.splitter {
            extra(&mut out, "splitter", procgen::aero::splitter(s, carbon));
        }
        if let Some(w) = &st.wing {
            extra(&mut out, "wing", procgen::aero::wing(w, faces, carbon));
        }
        if let Some(f) = &st.fins {
            extra(&mut out, "fins", procgen::aero::fins(f, faces, carbon));
        }
        if let Some(e) = &st.exhaust {
            extra(&mut out, "exhaust", procgen::exhaust::exhausts(e));
        }
        // REMASTER: the added parts and the lit strips were built from the unwarped cage; they
        // follow the body through the warps, a pure function of position (`cuts::warp_point`;
        // the Serpent's tail lamps sit in its rear bulge). View space here, game z negated.
        if !st.warps.is_empty() {
            for v in &mut out[body_len..] {
                let g = procgen::cuts::warp_point([v.pos[0], v.pos[1], -v.pos[2]], &st.warps);
                v.pos = [g[0], g[1], -g[2]];
            }
            for ls in &mut strips {
                ls.p0 = procgen::cuts::warp_point(ls.p0, &st.warps);
                ls.p1 = procgen::cuts::warp_point(ls.p1, &st.warps);
            }
        }
        // the lower ride height: the body and its lights, not the wheels; and the mirror plane
        // back where the cage has it (`prepare`)
        for v in &mut out {
            v.pos[0] += self.st.centre;
        }
        // REMASTER (#292): the hand fixes, in the frame the Blender add-on saw: the cage's mirror
        // plane back, the ride-height drop not yet taken
        if as_drawn {
            self.fixed = Some(procgen::bodyfix::apply(&mut out, &self.fixes));
        }
        let down = if as_drawn { self.st.drop } else { 0.0 };
        for v in &mut out {
            v.pos[1] -= down;
        }
        let strips = strips
            .into_iter()
            .map(|mut ls| {
                ls.p0[0] += self.st.centre;
                ls.p1[0] += self.st.centre;
                ls
            })
            .collect();
        (out, strips, parts)
    }

    /// The halos of the generated body: the car's, lowered with it, and the lit patterns' strips,
    /// which replace the record they came from as the glow [design choice].
    fn halos(&self, halos: &[HaloInst], strips: &[procgen::lamps::LampStrip]) -> Vec<HaloInst> {
        let down = |p: Vec3| p - Vec3::Y * self.st.drop;
        let mut out: Vec<HaloInst> = halos.iter().map(|h| HaloInst { p0: down(h.p0), p1: down(h.p1), ..*h }).collect();
        let n_car = out.len();
        for ls in strips {
            let src = self.halo_src[ls.halo];
            let (a, b) = (Vec3::from(to_view(src.0)), Vec3::from(to_view(src.1)));
            let Some(h) = out.iter_mut().find(|h| h.p0 == down(a) && h.p1 == down(b)) else { continue };
            h.strip = HaloStrip::Anchor;
            let strip = HaloInst { p0: down(Vec3::from(to_view(ls.p0))), p1: down(Vec3::from(to_view(ls.p1))), strip: HaloStrip::Strip { width: ls.width, join: ls.join }, ..*h };
            out.push(strip);
        }
        // REMASTER: the car's own halos follow the body through the warps like the lit parts
        // (`run_parts`); the cage is warped with its mirror plane on x = 0 (`centre`). The
        // strips were warped there already.
        if !self.st.warps.is_empty() {
            let bend = |p: Vec3| {
                let g = procgen::cuts::warp_point([p.x - self.st.centre, p.y, -p.z], &self.st.warps);
                down(Vec3::new(g[0] + self.st.centre, g[1], -g[2]))
            };
            for (h, raw) in out[..n_car].iter_mut().zip(halos) {
                h.p0 = bend(raw.p0);
                h.p1 = bend(raw.p1);
            }
        }
        out
    }
}

/// `BodyJob::run` for every job, on all cores but one (the thread that asked keeps working).
/// The results come back in the jobs' order whatever the thread timing.
fn run_bodies(jobs: Vec<BodyJob>) -> Vec<(BodyJob, Vec<Vertex>, Vec<procgen::lamps::LampStrip>)> {
    use std::sync::{atomic::{AtomicUsize, Ordering}, Mutex};
    type Slot = (BodyJob, Option<(Vec<Vertex>, Vec<procgen::lamps::LampStrip>)>);
    let slots: Vec<Mutex<Slot>> = jobs.into_iter().map(|j| Mutex::new((j, None))).collect();
    let threads = std::thread::available_parallelism().map_or(4, |n| n.get()).saturating_sub(1).clamp(1, slots.len().max(1));
    let next = AtomicUsize::new(0);
    let work = || {
        while let Some(slot) = slots.get(next.fetch_add(1, Ordering::Relaxed)) {
            let mut g = slot.lock().unwrap();
            let out = g.0.run();
            g.1 = Some(out);
        }
    };
    if threads == 1 {
        work();
    } else {
        std::thread::scope(|sc| {
            for _ in 0..threads {
                sc.spawn(work);
            }
        });
    }
    slots
        .into_iter()
        .map(|m| {
            let (job, out) = m.into_inner().unwrap();
            let (verts, strips) = out.expect("every body job ran");
            (job, verts, strips)
        })
        .collect()
}

/// REMASTER (09-remaster 9.22): the MYO body's polygons as `procgen::body` faces, coloured,
/// textured and env-tagged like `emit` draws them. `tags` are the polygons' original RGBs (before
/// CustomCarColours), which mark the parts; the colours are the painted ones.
fn cage_faces(m: &myo::Mesh, tags: &[[u8; 3]], layers: &mut Layers, lib: &TexLib, scale: [f32; 3]) -> Vec<procgen::body::Face> {
    m.polygons
        .iter()
        .zip(tags)
        .map(|(p, &tag)| {
            let (layer, tw, th) = match m.texture(p) {
                Some((name, _)) => {
                    let (l, w, h) = layers.get(lib, name, p.flags & ZERO_TRANSPARENT != 0);
                    (l as f32, w as f32, h as f32)
                }
                None => (-1.0, 1.0, 1.0),
            };
            let uv = (0..p.v.len())
                .map(|k| match p.uv_off {
                    Some(o) if layer >= 0.0 => {
                        let o = o as usize + 2 * k;
                        let (u, v) = (*m.uvbytes.get(o).unwrap_or(&0) as f32, *m.uvbytes.get(o + 1).unwrap_or(&0) as f32);
                        [(u + 0.5) / tw, (v + 0.5) / th]
                    }
                    _ => [0.0, 0.0],
                })
                .collect();
            procgen::body::Face {
                pts: p.v.iter().map(|&i| m.vertices[i as usize]).collect(),
                normal: m.planes.get(p.plane as usize).map_or([0.0, 1.0, 0.0], |q| [q[0], q[1], q[2]]),
                colour: (0..p.v.len()).map(|k| rgba_obj(*p.rgba.get(k).unwrap_or(&p.rgba[0]), scale)).collect(),
                uv,
                layer,
                env: if matches!(p.ty, 8 | 9) { 1.0 } else { 0.0 },
                rgb: tag,
                smooth_deg: 0.0,
                remesh: 0.0,
            }
        })
        .collect()
}

/// REMASTER: the colour (0..1) of texture layer `px` at point (y, z) of an original wheel's
/// face, through the face's uv fit (nearest texel of the resampled layer).
fn texel(px: &[u8], fit: [[f32; 3]; 2], y: f32, z: f32) -> [f32; 3] {
    let at = |f: [f32; 3]| ((f[0] * y + f[1] * z + f[2]).clamp(0.0, 1.0) * (TEX - 1) as f32).round() as usize;
    let o = (at(fit[1]) * TEX as usize + at(fit[0])) * 4;
    [0, 1, 2].map(|i| px[o + i] as f32 / 255.0)
}

/// REMASTER (09-remaster 9.20): the original wheel's rim face for `procgen::wheel`. [O] Each
/// MYO wheel is an octagonal prism whose rim side is a shallow cone of 8 textured triangles
/// (the alloy, e.g. 1alg0.iff, face colour 94) and whose other faces are flat (1, 1, 1). The
/// rim is on the textured side: -x on the left wheels (vf, vb), +x on the right (hf, hb).
/// Returns the face (texture layer, uv fit, colours) and the rim side.
fn wheel_face(m: &myo::Mesh, layers: &mut Layers, lib: &TexLib, scale: [f32; 3], wheel: usize) -> (Option<procgen::WheelFace>, f32) {
    let mut samples = Vec::new();
    let (mut side, mut found) = (0.0, None);
    for p in &m.polygons {
        let Some((name, uv)) = m.texture(p) else { continue };
        let (layer, tw, th) = layers.get(lib, name, p.flags & ZERO_TRANSPARENT != 0);
        found.get_or_insert((layer, p.rgba[0]));
        let mut poly = Vec::new();
        for (&vi, t) in p.v.iter().zip(&uv) {
            let v = m.vertices[vi as usize];
            side += v[0];
            poly.push(([v[1], v[2]], [t[0] / tw as f32, t[1] / th as f32]));
        }
        samples.push(poly);
    }
    // FL, BL are on the left
    let side = if side != 0.0 { side.signum() } else if matches!(wheel, 0 | 3) { -1.0 } else { 1.0 };
    // one map the polygons agree on: some wheels have half their face mapped mirrored (#356)
    let (Some((layer, face_rgba)), Some(fit)) = (found, procgen::fit_face_consensus(&samples)) else { return (None, side) };
    let colour = rgba_obj(face_rgba, scale);
    // the lip: the texture's mean around the rim's bright outer ring (0.85 of the wheel radius)
    let r = m.vertices.iter().map(|v| v[1]).fold(0.0, f32::max);
    let px = &layers.data[layer];
    let mut sum = [0.0f32; 3];
    const N: usize = 64;
    for k in 0..N {
        let (s, c) = (k as f32 / N as f32 * std::f32::consts::TAU).sin_cos();
        for (s, b) in sum.iter_mut().zip(texel(px, fit, 0.85 * r * s, 0.85 * r * c)) {
            *s += b / N as f32;
        }
    }
    let lip = [colour[0] * sum[0], colour[1] * sum[1], colour[2] * sum[2], 1.0];
    (Some(procgen::WheelFace { layer: layer as f32, fit, colour, lip }), side)
}

// Hologram line strength of the road surface (alpha; the car's edges are 0.20)
motor_tune::tunable! {
    /// Hologram line strength of the road surface (alpha; the car's edges are 0.20).
    /// range: 0..1
    /// applies: reload
    /// area: Graphics/View
    HOLO_ROAD_LINE: f32 = 0.35;
}
// the road's triangles are cut down to this edge length (m) so the lines show the car's speed
motor_tune::tunable! {
    /// The hologram road's triangles are cut down to this edge length so the lines show the car's speed.
    /// range: 0.5..20
    /// unit: m
    /// applies: reload
    /// area: Graphics/View
    HOLO_ROAD_CELL: f32 = 3.0;
}

/// REMASTER (front end Main drive, 09-remaster 9.24): the road surface's outline as a line
/// list: the upward-facing world triangles whose COL surface is `asphalt` (no kerbs, walls,
/// grass), cut into cells of at most `HOLO_ROAD_CELL` by bisecting the longest edge (the
/// original strips are long, a few lines would not show motion). Shared edges are drawn once,
/// and the diagonal of a quad (the longest edge of both coplanar triangles) is dropped
/// [design choice]. `tris` are view-space triangles (game z negated).
/// Per COL material (`World::materials`): is it the road surface (`asphalt`).
pub fn asphalt_materials(world: &motor_sim::world::World) -> Vec<bool> {
    world.materials.iter().map(|m| m.name.eq_ignore_ascii_case("asphalt")).collect()
}

pub fn road_lines(tris: &[Vertex], world: &motor_sim::world::World) -> Vec<Vertex> {
    use std::collections::HashMap;
    struct Edge {
        ends: [Vec3; 2],
        uses: u32,
        /// uses as a triangle's longest edge
        longest: u32,
        /// normal of the first triangle
        normal: Vec3,
    }
    let flip = Vec3::new(1.0, 1.0, -1.0);
    let asphalt = asphalt_materials(world);
    // quantised endpoints (1 cm) -> the edge's key
    let q = |p: Vec3| [(p.x * 100.0).round() as i32, (p.y * 100.0).round() as i32, (p.z * 100.0).round() as i32];
    let mut edges: HashMap<([i32; 3], [i32; 3]), Edge> = HashMap::new();
    let mut add = |p: [Vec3; 3], n: Vec3| {
        let len: [f32; 3] = std::array::from_fn(|k| p[k].distance(p[(k + 1) % 3]));
        let longest = (0..3).max_by(|&a, &b| len[a].total_cmp(&len[b])).unwrap();
        for k in 0..3 {
            let (a, b) = (q(p[k]), q(p[(k + 1) % 3]));
            let key = if a < b { (a, b) } else { (b, a) };
            let e = edges.entry(key).or_insert(Edge { ends: [p[k], p[(k + 1) % 3]], uses: 0, longest: 0, normal: n });
            e.uses += 1;
            e.longest += (k == longest) as u32;
            if e.uses == 2 && e.normal.dot(n) < 0.99 {
                e.longest = 0; // a fold: a real edge
            }
        }
    };
    for t in tris.as_chunks::<3>().0 {
        let p: [Vec3; 3] = std::array::from_fn(|k| Vec3::from(t[k].pos) * flip);
        let n = (p[1] - p[0]).cross(p[2] - p[0]).normalize_or_zero();
        if n.y.abs() < 0.7 {
            continue;
        }
        // the road under the centroid: asphalt within 0.3 m of the triangle
        let c = (p[0] + p[1] + p[2]) / 3.0;
        let hit = world.raycast(c + Vec3::Y * 0.5, c - Vec3::Y * 0.5);
        if !hit.is_some_and(|h| asphalt.get(h.material).copied().unwrap_or(false) && (h.point.y - c.y).abs() < 0.3) {
            continue;
        }
        let mut todo = vec![(p, 0u32)];
        while let Some((p, depth)) = todo.pop() {
            let len: [f32; 3] = std::array::from_fn(|k| p[k].distance(p[(k + 1) % 3]));
            let l = (0..3).max_by(|&a, &b| len[a].total_cmp(&len[b])).unwrap();
            if len[l] <= HOLO_ROAD_CELL.get() || depth >= 14 {
                add(p, n);
            } else {
                // bisect the longest edge a-b at m, c the opposite corner
                let (a, b, c) = (p[l], p[(l + 1) % 3], p[(l + 2) % 3]);
                let m = (a + b) * 0.5;
                todo.extend([([a, m, c], depth + 1), ([m, b, c], depth + 1)]);
            }
        }
    }
    let colour = [1.0, 1.0, 1.0, HOLO_ROAD_LINE.get()];
    let vertex = |p: Vec3| Vertex { pos: (p * flip).to_array(), uv: [0.0; 2], color: colour, layer: -1.0, normal: [0.0; 3], env: 0.0 };
    let mut out = Vec::with_capacity(edges.len() * 2);
    for e in edges.into_values() {
        if e.uses == 2 && e.longest == 2 {
            continue;
        }
        out.extend(e.ends.map(vertex));
    }
    out
}

/// Texture names sorted case-insensitively, one per name (the MYW lists repeat names in other case).
pub fn sorted_names(names: &[String]) -> Vec<String> {
    let mut v: Vec<String> = names.to_vec();
    v.sort_by_key(|n| n.to_lowercase());
    v.dedup_by_key(|n| n.to_lowercase());
    v
}

#[cfg(test)]
mod sort_tests {
    #[test]
    fn sorted_names_ignore_case_and_dedupe() {
        let n = ["b.iff", "_Rost.IFF", "A.iff", "_rost.iff", "a.IFF"].map(String::from);
        assert_eq!(super::sorted_names(&n), ["_Rost.IFF", "A.iff", "b.iff"]);
    }

    /// 08-missing-features 8.5: only the Kobra X3, Adder Mk2 and ASC II have a third paint zone. Skipped
    /// without the install.
    #[test]
    fn third_zone_on_three_cars() {
        let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../installed_game");
        if !root.join("Game").exists() {
            eprintln!("skipped: no installed_game next to the workspace");
            return;
        }
        let third: Vec<u32> = (1..=10).filter(|&id| super::paint_zones(&root, id)[2]).collect();
        assert_eq!(third, [1, 2, 5]);
        assert!((1..=10).all(|id| super::paint_zones(&root, id)[..2] == [true, true]));
    }

    /// #136, #351: a Metallic or Pearl zone tags only the faces `repaint` gives that zone's colour, the last of its
    /// in-order tests deciding; gloss tags none. Skipped without the install.
    #[test]
    fn metallic_faces_follow_the_repaint() {
        use super::Finish::{Gloss, Metallic, Pearl};
        let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../installed_game");
        if !root.join("Game").exists() {
            eprintln!("skipped: no installed_game next to the workspace");
            return;
        }
        let (m, c) = super::body_myo(&root, 1).unwrap();
        let own = super::own_colours(&c);
        let painted = |k: usize| m.polygons.iter().filter(|p| matches!(p.ty, 8 | 9) && p.flags & 0x60 != 0 && p.rgba[0][..3] == own[k][..]).count();
        let tagged = |paint: super::CarPaint, f: super::Finish| super::finish_faces(&m, own, paint).iter().filter(|&&t| t == f).count();
        let to = [[10, 20, 30], [40, 50, 60], [70, 80, 90]];
        assert_eq!(tagged(super::CarPaint::gloss(to), Metallic) + tagged(super::CarPaint::gloss(to), Pearl), 0);
        assert!(painted(0) > 0 && painted(1) > 0);
        assert_eq!(tagged(super::CarPaint { colours: to, finish: [Metallic, Gloss, Gloss] }, Metallic), painted(0));
        assert_eq!(tagged(super::CarPaint { colours: to, finish: [Gloss, Pearl, Gloss] }, Pearl), painted(1));
        // zone 1 painted in zone 2's stock colour: the second test repaints those faces too, so they take zone 2's finish
        let chain = super::CarPaint { colours: [own[1], to[1], to[2]], finish: [Gloss, Metallic, Gloss] };
        assert_eq!(tagged(chain, Metallic), painted(0) + painted(1));
        // mixed zones: each face carries its own zone's finish, and the three tags are distinct and inside the paint band
        let mixed = super::CarPaint { colours: to, finish: [Metallic, Pearl, Gloss] };
        assert_eq!((tagged(mixed, Metallic), tagged(mixed, Pearl)), (painted(0), painted(1)));
        let tags = [Gloss.tag(), Metallic.tag(), Pearl.tag()];
        assert_eq!(tags, [None, Some(super::METALLIC_ENV), Some(super::PEARL_ENV)]);
        assert!(tags.iter().flatten().all(|t| (0.5..2.5).contains(t)) && super::PEARL_ENV != super::METALLIC_ENV);
    }

    /// rimshop's car preview (#337 phase 5): every car loads without its world, and each fitted wheel
    /// keeps the original's size round its hub. Skipped without the install.
    #[test]
    fn showroom_cars_load() {
        use glam::Vec3;
        let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../installed_game");
        if !root.join("Game").exists() {
            eprintln!("skipped: no installed_game next to the workspace");
            return;
        }
        for id in 1..=10 {
            let car = super::load_showroom(&root, 1, id).unwrap();
            assert!(!car.body.is_empty() && !car.name.is_empty(), "car {id}");
            assert!(car.body.iter().chain(&car.generated).all(|v| v.layer < car.layers.len() as f32), "car {id}: layer out of range");
            let style = crate::procgen::RimStyle::default();
            for (i, (w, hub)) in car.wheels.iter().zip(car.hubs).enumerate() {
                let w = w.as_ref().unwrap_or_else(|| panic!("car {id} wheel {i}"));
                let alone = super::ShowroomCar { wheels: std::array::from_fn(|k| (k == i).then_some(*w)), name: String::new(), body: Vec::new(), generated: Vec::new(), layers: car.layers.clone(), hubs: car.hubs, halos: Vec::new(), model: None };
                let v = alone.wheels(&style);
                let (lo, hi) = v.iter().fold((Vec3::splat(f32::MAX), Vec3::splat(f32::MIN)), |(lo, hi), v| (lo.min(v.pos.into()), hi.max(v.pos.into())));
                let want = Vec3::new(w.half_width, w.radius, w.radius);
                assert!((hi - hub - want).abs().max_element() < 1e-3 && (lo - hub + want).abs().max_element() < 1e-3, "car {id} wheel {i}: {lo}..{hi} round {hub}");
            }
        }
    }
}

#[cfg(test)]
mod tron_tests {
    use super::*;

    fn poly(ty: u8, n: usize, flags: u8) -> myo::Polygon {
        myo::Polygon { ty, flags, plane: 0, rgba: vec![[255, 255, 255, 255]], v: (0..n as u16).collect(), normal: None, vn: Vec::new(), uv_off: None, texref_off: None }
    }

    /// The outlined edges of `p`'s triangles, as sorted pairs of polygon vertex indices.
    fn outlined(p: &myo::Polygon, verts: &[[f32; 3]], plane: [f32; 4]) -> (Vec<(usize, usize)>, Vec<u32>) {
        let mut out = Vec::new();
        let shade = Shade { scale: [1.0; 3], normals: None, tron: true };
        emit(&mut out, p, verts, &[plane], &[], None, &to_view, &shade);
        let of = |v: &Vertex| verts.iter().position(|q| to_view(*q) == v.pos).unwrap();
        let (mut edges, mut codes) = (Vec::new(), Vec::new());
        for t in out.as_chunks::<3>().0 {
            let code = (t[0].color[3] - TRON_CODE) as u32;
            assert!(t.iter().all(|v| v.color[3] == t[0].color[3]), "one code per triangle");
            codes.push(code);
            for k in 0..3 {
                if code & 1 << k != 0 {
                    let (a, b) = (of(&t[(k + 1) % 3]), of(&t[(k + 2) % 3]));
                    edges.push((a.min(b), a.max(b)));
                }
            }
        }
        edges.sort();
        (edges, codes)
    }

    #[test]
    fn quads_are_outlined_without_their_diagonal() {
        let quad = [[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [1.0, 0.0, 1.0], [0.0, 0.0, 1.0]];
        // both windings against the plane (the second triangle order is flipped by `emit`)
        for plane in [[0.0, 1.0, 0.0, 0.0], [0.0, -1.0, 0.0, 0.0]] {
            let (edges, codes) = outlined(&poly(3, 4, 0), &quad, plane);
            assert_eq!(edges, vec![(0, 1), (0, 3), (1, 2), (2, 3)], "plane {plane:?}");
            assert!(codes.iter().all(|c| c & TRON_DARK == 0));
        }
        let (edges, _) = outlined(&poly(0, 3, 0), &quad[..3], [0.0, 1.0, 0.0, 0.0]);
        assert_eq!(edges, vec![(0, 1), (0, 2), (1, 2)]);
    }

    #[test]
    fn flat_untextured_is_dark_and_plain_scenes_keep_alpha() {
        let quad = [[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [1.0, 0.0, 1.0], [0.0, 0.0, 1.0]];
        let (_, codes) = outlined(&poly(1, 4, 0), &quad, [0.0, 1.0, 0.0, 0.0]);
        assert!(codes.iter().all(|c| c & TRON_DARK != 0));
        let (_, codes) = outlined(&poly(9, 4, 0), &quad, [0.0, 1.0, 0.0, 0.0]);
        assert!(codes.iter().all(|c| c & TRON_DARK == 0), "Gouraud polygons are lit blue");
        let mut out = Vec::new();
        emit(&mut out, &poly(1, 4, 0), &quad, &[[0.0, 1.0, 0.0, 0.0]], &[], None, &to_view, &Shade { scale: [1.0; 3], normals: None, tron: false });
        assert!(out.iter().all(|v| v.color[3] == 1.0));
    }

    /// #459: a lit non-Gouraud polygon with flag 0x10 is tagged `SELF_LIT_ENV` (untextured too: the D3D converter
    /// draws a scene object's such face unlit); a Gouraud one keeps the paint tag
    #[test]
    fn flag_0x10_tags_lit_flat_polygons() {
        let quad = [[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [1.0, 0.0, 1.0], [0.0, 0.0, 1.0]];
        let planes = [[0.0, 1.0, 0.0, 0.0]];
        let lit = Shade { scale: [1.0; 3], normals: Some((&[], &planes)), tron: false };
        let env = |ty, flags, shade: &Shade| {
            let mut out = Vec::new();
            emit(&mut out, &poly(ty, 4, flags), &quad, &planes, &[], None, &to_view, shade);
            out[0].env
        };
        assert_eq!(env(1, 0x10, &lit), SELF_LIT_ENV);
        assert_eq!(env(1, 0, &lit), 0.0);
        assert_eq!(env(9, 0x10, &lit), 1.0);
        assert_eq!(env(1, 0x10, &Shade { scale: [1.0; 3], normals: None, tron: false }), 0.0, "the pre-lit world is untagged");
    }
}

#[cfg(test)]
mod resample_tests {
    use super::*;

    fn img(w: u32, h: u32, indices: Vec<u8>, palette: Vec<[u8; 3]>) -> iff::Image {
        iff::Image { width: w, height: h, planes: 8, indices, palette, transparent: None }
    }

    #[test]
    fn full_size_texture_is_copied_unchanged() {
        let n = (TEX * TEX) as usize;
        let palette: Vec<[u8; 3]> = (0..=255u8).map(|i| [i, 255 - i, i / 2]).collect();
        let indices: Vec<u8> = (0..n).map(|i| (i * 7 % 256) as u8).collect();
        let out = resample(&img(TEX, TEX, indices.clone(), palette.clone()), None);
        for (i, &ix) in indices.iter().enumerate() {
            let c = palette[ix as usize];
            assert_eq!(out[i * 4..i * 4 + 4], [c[0], c[1], c[2], 255], "texel {i}");
        }
    }

    #[test]
    fn upscale_is_a_smooth_ramp_that_wraps() {
        // 2x2: black | white on both rows; 256 wide -> each source texel covers 128 columns
        let out = resample(&img(2, 2, vec![0, 1, 0, 1], vec![[0, 0, 0], [255, 255, 255]]), None);
        let at = |x: usize| out[x * 4];
        assert!(at(64) <= 2, "centre of the black texel: {}", at(64));
        assert!(at(192) >= 253, "centre of the white texel: {}", at(192));
        assert!((at(128) as i32 - 128).abs() <= 2, "midway between: {}", at(128));
        assert!(at(100) < at(110) && at(110) < at(120), "monotonic");
        // wrap: column 0 blends white (left neighbour across the edge) with black
        assert!(at(0) > 0 && at(255) > at(0), "wraps: {} {}", at(0), at(255));
    }

    #[test]
    fn key_colour_does_not_bleed_and_alpha_crosses_half_midway() {
        let mut im = img(2, 2, vec![0, 1, 0, 1], vec![[255, 0, 255], [10, 20, 30]]);
        im.transparent = Some(0);
        let out = resample(&im, Some(0));
        let px = |x: usize| &out[x * 4..x * 4 + 4];
        assert!(px(64)[3] <= 2, "keyed centre {}", px(64)[3]);
        assert!(px(192)[3] >= 253 && px(192)[..3] == [10, 20, 30], "opaque centre {:?}", px(192));
        // between the two texels the visible colour stays the opaque texel's, never magenta
        assert_eq!(px(150)[..3], [10, 20, 30]);
        assert!(px(100)[3] < 128 && px(156)[3] > 128, "alpha {} {}", px(100)[3], px(156)[3]);
        // a fully keyed texel keeps the key colour (its alpha is 0 only well inside the keyed block; here 2x2 never has one)
        assert!(px(10)[3] < 128);
    }
}
