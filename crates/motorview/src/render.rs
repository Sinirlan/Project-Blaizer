//! wgpu renderer: sky pass, one textured triangle pipeline, one line pipeline, a 2D texture
//! array. Colours are kept in gamma space end to end (non-sRGB textures and targets), as the
//! original's D3D5/Glide output did.

use crate::pick::edges;
use crate::wire::{self, WireStats, WireStyle};
use crate::fx::Sprite;
use crate::scene::{HaloInst, HaloStrip, LampLight, Look, PointLights, PropLamps, Scene, Vertex, CAR_MESHES, TEX};
use crate::ui::{Atlas, Canvas, Rect, UiVertex, ATLAS};
use crate::ui_sdf::SdfAtlas;
use glam::{Mat4, Vec3, Vec4};
use std::cell::{Cell, OnceCell, RefCell};
use wgpu::util::DeviceExt;

pub const DEPTH: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;
/// REMASTER: the HDR scene and bloom targets (linear light, values above 1 kept)
const HDR: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;
/// REMASTER: `Globals.env.w` bit of a draw into the HDR target: the shaders output linear,
/// unclamped colour
const LINEAR_FLAG: u32 = 8;
/// REMASTER: `Globals.env.w` bit of an HDR view with bloom and smooth halos: halos draw a hot
/// core over the bloom threshold and a faint tail (fs_halo_remaster, 09-remaster 9.11)
const HALO_BLOOM_FLAG: u32 = 32;
/// REMASTER: `Globals.env.w` bit of a view that samples the car shadow map (set per view,
/// fs_remaster)
const SHADOW_FLAG: u32 = 128;
/// REMASTER car shadows [design choice]: map size, cars farther than `SHADOW_RANGE` from the
/// camera are left out of the fit (they cast into the map only if inside it), the margin
/// around the cars' centres, the depth bias (metres) and the largest sun slant from vertical
/// the map is set up for.
const SHADOW_SIZE: u32 = 2048;
const SHADOW_RANGE: f32 = 120.0;
const SHADOW_MARGIN: f32 = 4.5;
const SHADOW_MIN_RADIUS: f32 = 8.0;
const SHADOW_BIAS: f32 = 0.08;
/// REMASTER overhead car shadow [design choice]: on tracks darker than `OVERHEAD_DARKNESS`
/// (`Look::darkness`: 1, 3, 7, 8) the map looks straight down instead of along the MYW sun, which
/// cannot tell dusk from night (9.15). The shadow gets a soft edge of `OVERHEAD_SOFT` metres and
/// the shader's fixed `OVERHEAD_WORLD` strength; lit objects are not shadowed.
const OVERHEAD_DARKNESS: f32 = 0.3;
const OVERHEAD_SOFT: f32 = 1.0;
const OVERHEAD_WORLD: f32 = 0.55;
/// REMASTER lamp shadows [design choice, 11-dynamic-shadows.md phase 2]: layers of the atlas
/// (the first `LAMP_LAYERS` for street lamps, the rest for the headlights of phase 3), map size,
/// the `Globals.env.w` bit of a view that samples it, the fade time of a layer, and how far
/// from the camera a lamp's cone must hold a car to get a map.
const SPOT_LAYERS: usize = 6;
/// edges of the light shadow atlases (`ShadowQuality::atlas`)
const SPOT_SIZES: [u32; 3] = [512, 1024, 2048];
/// bound of a car body as a shadow caster, metres [APPROX: one sphere for every car]
const CASTER_RADIUS: f32 = 2.6;
const LAMP_LAYERS: usize = 4;
/// layers of the atlas after the lamps': the headlights of the cars (phase 3)
const HEAD_LAYERS: usize = SPOT_LAYERS - LAMP_LAYERS;
const SPOT_FLAG: u32 = 512;
/// `Globals.env.w` bit of a view whose fog is applied by the volume pass (`VolFog`): the surfaces
/// draw no fog of their own (shader `fog_mix`)
const VOLFOG_FLAG: u32 = 2048;
/// CLASSIC: `Globals.env.w` bit of a view with Flat car shading: lit objects take one even light
/// per polygon (`flat_light`, shader.wgsl)
const FLAT_FLAG: u32 = 8192;
/// CHEAT tribute to tron (#320, `0x48e964`): `Globals.env.w` bit of a Tron view: the MYO triangles are drawn as blue
/// light and outlined (shader `tron_shade`, the codes of `scene::TRON_CODE`)
const TRON_FLAG: u32 = 16384;
/// REMASTER Neon grid (#320 phase 3, design choice): the glowing polygon sides are this wide at a 1080-line render,
/// in proportion at other heights (so SSAA and 4K keep the same look), half on each triangle of a shared side
const NEON_EDGE_PX: f32 = 1.5;
const NEON_REF_H: f32 = 1080.0;
/// REMASTER Neon grid: the edge width in the 128 px env probe faces (the reflections of the grid)
const NEON_PROBE_EDGE_PX: f32 = 1.5;
/// REMASTER fog: the fog colour's ceiling in linear light (`fog_colour`)
const FOG_COLOUR_MAX: f32 = 4.0;
/// the shaders' gamma (shader.wgsl GAMMA)
const GAMMA: f32 = 2.2;
const SPOT_FADE: f32 = 0.3;
/// REMASTER bloom: mip levels of the chain below the half-size first level (fewer on small
/// targets) and the soft threshold, in linear light (knee: the width of the soft part)
/// [design choice]
const BLOOM_LEVELS: u32 = 6;
const BLOOM_THRESHOLD: f32 = 1.0;
const BLOOM_KNEE: f32 = 0.25;
/// the brightest light the bloom takes (linear): above the halo cores (shader HALO_CORE_GAIN
/// 2.5), below sun glints on the cars, which bloomed into blobs driving towards the sun
/// (Atlantika) [design choice]
const BLOOM_MAX: f32 = 3.0;
/// bloom added at the option's 100 %
const BLOOM_STRENGTH: f32 = 4.0;
/// REMASTER ambient occlusion (ssao.wgsl, 09-remaster 9.28) [design choice]: the horizon search
/// radius and the distance where it fades out (metres), the darkening at the option's 100 %, and
/// the format of the half-size (visibility, linear depth) targets
const SSAO_RADIUS: f32 = 1.5;
const SSAO_FADE: [f32; 2] = [60.0, 140.0];
const SSAO_MAX: f32 = 0.7;
/// REMASTER local light: seconds a volume baked in the background takes to fade in [design choice]
const LOCAL_FADE: f32 = 1.0;
/// REMASTER local light (#192): the probes' sun term is rebaked when the view's sun turns more
/// than this (cosine of 0.5°) and has then stayed put for `LOCAL_SUN_SETTLE` seconds (the F11
/// sky turn slider) [design choice].
const LOCAL_SUN_TURN: f32 = 0.999_962;
const LOCAL_SUN_SETTLE: f32 = 0.5;
const SSAO_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rg16Float;
/// Car meshes drawn per frame (one uniform buffer each): 8 cars x (body + 4 wheels), plus
/// headroom.
pub const MAX_CARS: usize = 48;

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Globals {
    view_proj: [[f32; 4]; 4],
    view: [[f32; 4]; 4],
    model: [[f32; 4]; 4],
    /// env-probe faces: inverse view-projection for the sky directions
    sky_inv: [[f32; 4]; 4],
    /// w: screen height / width (the env-mapped scene objects' screen term, `vs_main`)
    cam_pos: [f32; 4],
    params: [f32; 4],
    fog: [f32; 4],
    ambient: [f32; 4],
    sun: [f32; 4],
    sun_dir: [f32; 4],
    sky_top: [f32; 4],
    sky_bottom: [f32; 4],
    sky_geom: [f32; 4],
    /// x: car shading (0 none, 1 Reflection, 2 Enhanced), y: env-map layer, z: dynamic lights
    env: [f32; 4],
    /// x: how many `spots` are in use
    spot_n: [f32; 4],
    /// dynamic lights: (position, range), (rgb, 0) per light; `env.z` of them are in use
    lights: [[f32; 4]; 2 * MAX_LIGHTS],
    /// REMASTER car lights: (position, range), (rgb, kind: 0 headlight, 1 tail light), (beam
    /// axis, 0), (car up, 0)
    spots: [[f32; 4]; 4 * MAX_SPOTS],
    /// MYW point lights of the lit object drawn with this uniform: (position, range), (rgb,
    /// LightIntensity * intensity) per light
    myw: [[f32; 4]; 2 * MAX_MYW],
    /// x: lights in use, y: 1 = the light origin is the model's translation (cars, wheels), 0 =
    /// the vertex (scene objects, drawn in world space)
    myw_n: [f32; 4],
    /// REMASTER street lamps: (position, range), (rgb, 0) per lamp; `lamp_n.x` of them in use
    lamps: [[f32; 4]; 2 * MAX_LAMPS],
    /// x: lamps in use, y/z: cosine of the cone's inner and outer half angle
    lamp_n: [f32; 4],
    /// REMASTER water (procgen/water.rs `Params::packed`): 4 vec4 per body, `MAX_WATER` bodies
    water: [[f32; 4]; WATER_ROWS * MAX_WATER],
    /// x: animation time in seconds; y: 0 no scene copy, 1 copy, 2 copy + reflections; z/w: near, far
    water_t: [f32; 4],
    /// REMASTER fog (`FogTune`): x density per metre at the ground, y ground height,
    /// z 1 / height scale, w sun glow strength (already times the track's daylight)
    fog2: [f32; 4],
    /// x: sun glow anisotropy (HG g); y/z: the sky dome's turn, exposure; w: the fog volume's far distance in a volumetric view (0 otherwise)
    fog3: [f32; 4],
    /// REMASTER fog: rgb the fog colour in linear light (`FogTune::colour` x brightness x tint), w sky haze
    fog4: [f32; 4],
    /// REMASTER fog: start, full (m), opacity, curve
    fog5: [f32; 4],
    /// REMASTER grass (procgen/grass.rs `Params::packed`): `GRASS_ROWS` vec4 per body, `MAX_GRASS` bodies
    grass: [[f32; 4]; GRASS_ROWS * MAX_GRASS],
    /// REMASTER grass: the cars pressing the blades (`grass_cars`), 3 vec4 per car: (position,
    /// hull half length), (forward xz, hull half width, 0), (velocity xz, 0, 0); `grass_n.x` in use
    grass_cars: [[f32; 4]; 3 * MAX_GRASS_CARS],
    /// x: cars pressing; y: the edge map's metres at 1 (0: no map, the vertex fade is used)
    grass_n: [f32; 4],
    /// REMASTER grass: the edge map's transform (`EdgeMap::transform`)
    grass_map: [f32; 4],
    /// REMASTER sky ambient (09-remaster 9.32, `sky_sh`): the HDRI's irradiance as 9 L2 terms, rgb
    /// in linear light; `[0].w` = 1 when it replaces `ambient` on lit objects
    sky_sh: [[f32; 4]; 9],
    /// REMASTER local light (#83, probes.rs): x the probe volume's weight (0 = not bound, under 1
    /// while it fades in), y bounce gain, z sky occlusion strength (0..1)
    local: [f32; 4],
    /// REMASTER local light: xyz the brick coordinates of the volume's low corner
    local_box: [f32; 4],
    /// REMASTER hologram clip (`HoloClip::rects`): centre xy, half extents zw per rectangle
    holo_rects: [[f32; 4]; HOLO_RECTS],
    /// x: the listed rectangles' corner radius, y: how many are in use
    holo_n: [f32; 4],
    /// REMASTER live paint (`LivePaint`, car bodies only): rows 0..2 = stock colour k as emitted, w = 1 gloss, 2 Metallic
    /// (#136), 3 Pearl (#351) for zone k, 0 = off; rows 3..5 = its new colour, w = brightness gain
    paint: [[f32; 4]; 6],
    /// rgb: the user's Brightness per channel (times the lightning gain in Hell mode, #316)
    gain: [f32; 4],
    /// CHEAT lemmy / ace Hell mode: the cloud-ceiling sky's 8 vertices (`hell_sky`), 2 vec4 each
    hell: [[f32; 4]; 16],
    /// CHEAT tribute to tron, REMASTER Neon grid (#320): x = half the edge width in render pixels (`NEON_EDGE_PX`)
    tron: [f32; 4],
}

/// The scene shader: shader.wgsl plus the REMASTER paint finish model (paint.wgsl, re-notes/21), which `fs_remaster`
/// calls. Every module built from shader.wgsl (grass, water, holofrost) starts from this, so the `fin_*` names resolve.
const SHADER_SRC: &str = concat!(include_str!("shader.wgsl"), "\n", include_str!("paint.wgsl"));

/// Water bodies per track (the parameters ride in the globals)
const MAX_WATER: usize = crate::procgen::water::MAX_BODIES;
const WATER_ROWS: usize = crate::procgen::water::ROWS;
/// Grass bodies per track (the parameters ride in the globals)
const MAX_GRASS: usize = crate::procgen::grass::MAX_BODIES;
const GRASS_ROWS: usize = crate::procgen::grass::ROWS;
/// Cars pressing the grass per frame (a race has at most 8)
const MAX_GRASS_CARS: usize = 8;
/// A car's footprint when the frame carries no `CarMotion` (hull half length, half width, m)
const GRASS_CAR_HALF: [f32; 2] = [2.2, 1.0];

/// Street lamps per frame: the ones nearest the camera (the shipped tracks have at most 43 world
/// halos [O: examples/lampaudit.rs], most of them out of range at a time).
pub(crate) const MAX_LAMPS: usize = 16;

/// REMASTER fog style (17-fog-rework.md) [design choice]: Classic is the original's linear distance
/// fog; Enhanced keeps that curve as its far envelope and adds a height haze and a sun glow.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum FogStyle {
    Classic,
    Enhanced,
    /// the Enhanced haze as a froxel volume applied from the depth buffer (`VolFog`, 17-fog-rework.md appendix)
    Volumetric,
}

impl FogStyle {
    pub const ALL: [FogStyle; 3] = [FogStyle::Classic, FogStyle::Enhanced, FogStyle::Volumetric];
    pub const NAMES: [&'static str; 3] = ["Classic", "Enhanced", "Volumetric"];
    pub fn name(self) -> &'static str {
        Self::NAMES[self as usize]
    }
    pub fn parse(s: &str) -> Option<Self> {
        Self::NAMES.iter().position(|n| n.eq_ignore_ascii_case(s)).map(|i| Self::ALL[i])
    }
}

/// REMASTER sky dome constants (F11 window, 09-remaster 9.32) [design choice]
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct SkyTune {
    /// exposure of the HDRI in stops (the file is scene-referred radiance, the world is not)
    pub exposure: f32,
    /// turn of the dome in degrees on top of the automatic sun alignment
    pub yaw: f32,
    /// the sun of cars and props (light and shadow direction) follows the HDRI's sun: its azimuth
    /// is the track's turned by `yaw`, its elevation the image's; off: the track's own sun
    pub sun: bool,
    /// the ambient light of cars and props is the HDRI's irradiance (SH), scaled to the track's
    /// ObjectAmbient in luminance; off: the flat ObjectAmbient
    pub ambient: bool,
    /// how much of the HDRI's colour the sky ambient keeps (0 = grey with the image's up/down
    /// contrast, 1 = the full tint); a saturated sky (track 1's red) would otherwise repaint the cars
    pub ambient_tint: f32,
}

impl Default for SkyTune {
    fn default() -> Self {
        SkyTune { exposure: -1.0, yaw: 0.0, sun: true, ambient: true, ambient_tint: 0.5 }
    }
}

/// REMASTER fog colour source (`FogTune::colour`, 17-fog-rework.md) [design choice]
#[derive(Clone, Copy, PartialEq, Debug, Default)]
pub enum FogColour {
    /// the sky's colour at the horizon (`Look::horizon_band`, or the HDRI dome's horizon ring)
    #[default]
    Sky,
    /// the track's CueColour
    Track,
    /// an explicit colour (0..1, gamma)
    Rgb([f32; 3]),
}

/// REMASTER fog constants (17-fog-rework.md, F11 window) [design choice]: a distance ramp from
/// `start` to `full` reaching `opacity` (shape `t^curve`), an exponential height haze
/// `rho(y) = rho0 * exp(-(y - ground) / height)` integrated along the view ray in closed form, and a
/// guard that reaches 100 % at the clip plane; the in-scatter colour is `colour` plus a
/// Henyey-Greenstein sun glow. Distances in metres. Shader: `fog_remaster`, `fog_sky`.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct FogTune {
    /// distance fog starts (m)
    pub start: f32,
    /// distance where the distance fog reaches `opacity` (m)
    pub full: f32,
    /// the distance fog's maximum (0..1; the clip guard still reaches 1 at the clip plane)
    pub opacity: f32,
    /// ramp shape exponent (1 = linear like the original, higher = soft start)
    pub curve: f32,
    /// optical depth of the haze per 100 m at ground level (0 = no haze)
    pub density: f32,
    /// height scale (m): the haze thins to 1/e this far above the ground
    pub height: f32,
    pub colour: FogColour,
    /// scale of the fog colour (lifts dark night fog)
    pub brightness: f32,
    /// multiplier on the fog colour
    pub tint: [f32; 3],
    /// sun glow strength (times the track's daylight; 0 = none)
    pub glow: f32,
    /// Henyey-Greenstein g of the glow lobe (0 = round, 0.9 = tight around the sun)
    pub anisotropy: f32,
    /// how far above the horizon the fog climbs on the sky (0 = none, 1 = up to 30 degrees)
    pub sky_haze: f32,
    /// Volumetric: how much the street lamps' cones scatter in the haze (0 = none)
    pub lamps: f32,
    /// Volumetric: how much the haze darkens in the sun's shadow (0 = only the sun glow is shadowed, 1 = the whole haze)
    pub shafts: f32,
}

impl FogTune {
    /// the values every track starts from (`fog::TABLE` sets the rest)
    pub const BASE: FogTune = FogTune { start: 36.0, full: 180.0, opacity: 1.0, curve: 1.0, density: 0.3, height: 50.0, colour: FogColour::Sky, brightness: 1.0, tint: [1.0; 3], glow: 0.8, anisotropy: 0.5, sky_haze: 0.6, lamps: 5.0, shafts: 0.5 };
}

impl Default for FogTune {
    fn default() -> Self {
        Self::BASE
    }
}

/// REMASTER street lamp light [design choice, 11-dynamic-shadows.md phase 1]: world halos shine
/// down as cones onto cars and scene objects at night. The pre-lit world is not lit again (the
/// bake already holds its lamp pools). The F11 window edits this.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct LampLightTune {
    /// multiplies the halo colour (times the track's darkness)
    pub gain: f32,
    /// multiplies the range taken from the lamp's height
    pub range: f32,
    /// half angle of the cone's full-strength part and of its edge, degrees
    pub inner: f32,
    pub outer: f32,
    /// how much a car's lamp shadow darkens the baked world (0: no lamp shadows at all)
    pub shadow: f32,
    /// depth bias of the lamp shadow maps, metres towards the lamp
    pub bias: f32,
    /// a lamp gets a shadow map only while a car under it is this close to the camera, metres
    pub distance: f32,
    /// how far a lamp's shadow reaches, times the lamp's light range (a shadow is longer than
    /// the pool of light it sits in: the car passes the lamp at speed)
    pub reach: f32,
    /// half angle from straight down within which a lamp casts shadows, degrees
    pub shadow_cone: f32,
    /// how far a car's headlight shadow reaches, metres (a car further ahead does not shadow the road)
    pub head_reach: f32,
    /// half angles of a headlight shadow map (horizontal, vertical), degrees; wider than the beam
    pub head_fov_x: f32,
    pub head_fov_y: f32,
}

impl LampLightTune {
    /// range of a lamp's shadow, metres
    fn shadow_range(&self, l: &LampLight) -> f32 {
        l.range * self.range * self.reach
    }
}

impl Default for LampLightTune {
    fn default() -> Self {
        LampLightTune { gain: 1.5, range: 1.0, inner: 30.0, outer: 65.0, shadow: 1.5, bias: 0.05, distance: 150.0, reach: 3.0, shadow_cone: 80.0, head_reach: 60.0, head_fov_x: 45.0, head_fov_y: 25.0 }
    }
}

/// REMASTER local light (#83, probes.rs) [design choice]: how the irradiance probes light cars
/// and props, `lin(sky) * mix(1, vis, occlusion) + bounce * gain` (shader.wgsl `local_ambient`).
/// The F11 window edits this; the bake does not depend on it.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct LocalLightTune {
    /// gain of the light bounced off the pre-lit world
    pub bounce: f32,
    /// how much of the sky occlusion is applied, 0..1 (0: the sky ambient everywhere)
    pub occlusion: f32,
}

impl Default for LocalLightTune {
    fn default() -> Self {
        LocalLightTune { bounce: 1.0, occlusion: 1.0 }
    }
}

/// MYW point lights per lit object: a cell holds at most 8 on the shipped tracks [O].
const MAX_MYW: usize = 16;

/// The two rows of a MYW point light in the shader: (position, range), (rgb, LightIntensity * intensity).
fn myw_rows(l: &crate::scene::ViewLight) -> [[f32; 4]; 2] {
    [[l.pos.x, l.pos.y, l.pos.z, l.range], [l.rgb[0], l.rgb[1], l.rgb[2], l.scale]]
}

/// CLASSIC scene objects light themselves from their own cell's point lights (0x439134), one record
/// per object in `Gpu::prop_lights` (shader.wgsl `prop_myw`): `PROP_REC` vec4 = (origin, count) and
/// `PROP_REC_LIGHTS` lights of two rows. A cell holds at most 8 on the shipped tracks [O].
const PROP_REC_LIGHTS: usize = 8;
const PROP_REC: usize = 1 + 2 * PROP_REC_LIGHTS;
/// capacity of the records and of the per-vertex record index (`Gpu::prop_of`); a frame beyond it
/// falls back to the merged list
const PROP_RECS: usize = 4096;
const PROP_VERTS: usize = 1 << 20;

/// Fill `g` with the MYW point lights `idx` (indices into `pl.lights`, nearest first when
/// trimmed); `by_model`: the light origin is the model's translation (cars), else the vertex.
fn set_myw(g: &mut Globals, pl: &PointLights, idx: &[u16], by_model: bool) {
    let idx = &idx[..idx.len().min(MAX_MYW)];
    for (i, &k) in idx.iter().enumerate() {
        [g.myw[2 * i], g.myw[2 * i + 1]] = myw_rows(&pl.lights[k as usize]);
    }
    g.myw_n = [idx.len() as f32, by_model as u32 as f32, 0.0, 0.0];
}

/// Dynamic lights per frame: the ones nearest the camera.
const MAX_LIGHTS: usize = 32;
/// REMASTER dynamic lights [design choice, not in the original]: each scene-object halo (blinkers,
/// flashing cones) lights the road, walls and cars within this range (metres), in its colour
/// times its texture's tint (`HaloInst::tint`) times the object's animated scale, so the
/// light blinks with the halo.
pub const PROP_LIGHT_RANGE: f32 = 7.0;
const PROP_LIGHT_GAIN: f32 = 2.0;
/// Car light spots per frame: two headlights and two tail lights per car for 8 cars.
pub(crate) const MAX_SPOTS: usize = 32;
/// REMASTER car lights [design choice, not in the original: its car halos light nothing]. The
/// front halos of a car (all 10 cars: two warm-white headlights at game z > 0, two red tail
/// lights at z < 0 [O: MYO halo records]) become spot lights with a procedural low-beam
/// cookie (`headlight` in shader.wgsl), the rear ones short red spots facing backwards with a
/// wide cone, so they light the road and the car behind but not their own body (a point
/// light there turned the whole rear red). Both are scaled by the track's darkness (`Look::darkness`), so they show
/// at night and fade out on daylight tracks. Braking raises the tail lights (light and halo)
/// by `BRAKE_GAIN`.
pub const HEADLIGHT_RANGE: f32 = 40.0;
pub const TAIL_LIGHT_RANGE: f32 = 4.0;

/// REMASTER look of one kind of car lamp, headlights and tail lights tuned apart and apart from
/// the world and scene-object halos [design choice]. A lamp's halo gain is
/// (`halo` + night × (`night` + `glare` × facing)) × (1 + (`brake_halo` - 1) × brake), with night
/// = the track's darkness and facing = (beam axis · direction to the eye)^4 (looking into the
/// beam: oncoming cars, the mirror). Size, burst and spot go from the resting value to the
/// `brake_*` one with the car's brake input. Everything is multiplied into the shared pipeline
/// values, so 1.0 is "as a world halo".
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct LampTune {
    /// halo gain (× `CAR_HALO_BRIGHTNESS`), by day
    pub halo: f32,
    /// halo gain added at full darkness, and the extra gain / quad growth looking into the beam
    pub night: f32,
    pub glare: f32,
    pub glare_size: f32,
    /// halo quad size, and the starburst's size and strength (× the halo's colour), at rest
    pub size: f32,
    pub burst_size: f32,
    pub burst_gain: f32,
    /// colour scale of the lamp's halos and bursts (the car's `REMASTER_HALO_SCALE`)
    pub scale: f32,
    /// spot light gain on the road and the cars behind
    pub spot: f32,
    /// lens flare strength × (`flare_day` + `flare_night` × night + `brake_flare` × brake)
    pub flare: f32,
    pub flare_day: f32,
    pub flare_night: f32,
    /// at full brake: halo gain multiplier, quad size, burst size and strength, spot gain
    /// (adds `brake_spot` × brake to 1) and flare strength
    pub brake_halo: f32,
    pub brake_size: f32,
    pub brake_burst_size: f32,
    pub brake_burst_gain: f32,
    pub brake_spot: f32,
    pub brake_flare: f32,
    /// generated cars: gain and width of the halo strips along the lamp's lit pattern
    /// (`HaloStrip`; the anchor record keeps the burst, spot and flare)
    pub strip: f32,
    pub strip_width: f32,
}

impl CarLightTune {
    /// the brake input of car `mesh` (a car mesh index) for its lights
    fn brake(&self, f: &Frame, mesh: usize) -> f32 {
        f.car_brake.get(mesh / CAR_MESHES).copied().unwrap_or(0.0).max(self.brake_preview)
    }
}

fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

impl LampTune {
    pub fn halo_gain(&self, night: f32, facing: f32, brake: f32) -> f32 {
        (self.halo + night * (self.night + self.glare * facing)) * (1.0 + (self.brake_halo - 1.0) * brake)
    }
    pub fn halo_size(&self, night: f32, facing: f32, brake: f32) -> f32 {
        lerp(self.size, self.brake_size, brake) * (1.0 + night * self.glare_size * facing)
    }
    pub fn burst_size(&self, brake: f32) -> f32 {
        lerp(self.burst_size, self.brake_burst_size, brake)
    }
    pub fn burst_gain(&self, brake: f32) -> f32 {
        lerp(self.burst_gain, self.brake_burst_gain, brake)
    }
    pub fn spot_gain(&self, brake: f32) -> f32 {
        self.spot * (1.0 + self.brake_spot * brake)
    }
    pub fn flare_strength(&self, night: f32, brake: f32) -> f32 {
        self.flare * (self.flare_day + self.flare_night * night + self.brake_flare * brake)
    }
}

/// REMASTER car lights [design choice, not in the original: its car halos light nothing]: the
/// front halos of a car (all 10 cars: two warm-white headlights at game z > 0, two red tail
/// lights at z < 0 [O: MYO halo records]) become spot lights with a procedural low-beam
/// cookie (`headlight` in shader.wgsl), the rear ones short red spots facing backwards with a
/// wide cone, so they light the road and the car behind but not their own body (a point
/// light there turned the whole rear red). Both are scaled by the track's darkness
/// (`Look::darkness`), so they show at night and fade out on daylight tracks. Braking raises
/// the tail lights (light, halo and burst, `LampTune`). The F11 window edits this.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct CarLightTune {
    pub head: LampTune,
    pub tail: LampTune,
    /// DEBUG: every car brakes at least this much (0 = off), to tune the brake look standing
    pub brake_preview: f32,
}

impl Default for CarLightTune {
    fn default() -> Self {
        CarLightTune {
            brake_preview: 0.0,
            head: LampTune {
                halo: 1.0,
                night: 2.0,
                glare: 8.0,
                glare_size: 0.3,
                size: 1.0,
                burst_size: 1.0,
                burst_gain: 1.0,
                scale: 0.5,
                spot: 4.0,
                flare: 0.8,
                flare_day: 0.25,
                flare_night: 1.0,
                brake_halo: 1.0,
                brake_size: 1.0,
                brake_burst_size: 1.0,
                brake_burst_gain: 1.0,
                brake_spot: 0.0,
                brake_flare: 0.0,
                strip: 1.0,
                strip_width: 1.0,
            },
            tail: LampTune {
                halo: 0.5,
                night: 0.5,
                glare: 0.0,
                glare_size: 0.0,
                size: 1.0,
                burst_size: 0.5,
                burst_gain: 0.4,
                scale: 0.5,
                spot: 0.6,
                flare: 0.2,
                flare_day: 0.125,
                flare_night: 0.5,
                brake_halo: 5.0,
                brake_size: 1.15,
                brake_burst_size: 1.2,
                brake_burst_gain: 0.8,
                brake_spot: 1.5,
                brake_flare: 1.0,
                strip: 1.2,
                strip_width: 1.0,
            },
        }
    }
}

/// Horizontal field of view: camera zoom 1.0 set at race init (0x458988) gives
/// atan(1/zoom) * 360/pi = 90 degrees (0x40b380) [I: horizontal].
pub const FOV_X: f32 = 90.0;
/// The sky image band is sized for a 480-line screen (0x443a68 works in screen pixels).
const SKY_REF_LINES: f32 = 480.0;
/// CLASSIC sky band: U advances by (heading step in degrees) / 100 per frame (0x443caf = 100.0,
/// 0x4d3865 = 180.0, 0x443e10..0x443e43): one image per 100 degrees of turn [O].
const SKY_U_PER_RAD: f32 = 180.0 / (std::f32::consts::PI * 100.0);

#[derive(Clone, Copy, Debug)]
pub struct Camera {
    pub pos: Vec3,
    pub yaw: f32,
    pub pitch: f32,
    /// bank about the view axis, radians (the TV camera's car-mounted shots)
    pub roll: f32,
    /// 1.0 = the normal horizontal field of view (`FOV_X`); the TV camera zooms (0x40b498)
    pub zoom: f32,
    /// multiplier of the far plane (the TV camera, 0x40b564)
    pub far_k: f32,
}

impl Camera {
    pub fn new(pos: Vec3, yaw: f32, pitch: f32) -> Self {
        Camera { pos, yaw, pitch, roll: 0.0, zoom: 1.0, far_k: 1.0 }
    }
    /// From a game-space orientation (columns right, up, forward; the view space negates z).
    pub fn from_rot(eye_game: Vec3, rot: glam::Mat3, zoom: f32, far_k: f32) -> Self {
        let flip = Vec3::new(1.0, 1.0, -1.0);
        let (f, u) = (rot.z_axis * flip, rot.y_axis * flip);
        let mut c = Camera::new(eye_game * flip, f.x.atan2(-f.z), f.y.clamp(-1.0, 1.0).asin());
        let (r0, u0) = (c.right(), c.right().cross(f));
        c.roll = u.dot(r0).atan2(u.dot(u0));
        (c.zoom, c.far_k) = (zoom, far_k);
        c
    }
    /// The up vector with the roll applied.
    pub fn up(&self) -> Vec3 {
        let (r, f) = (self.right(), self.forward());
        r.cross(f) * self.roll.cos() + r * self.roll.sin()
    }
    pub fn looking(pos: Vec3, dir: Vec3) -> Self {
        Camera::new(pos, dir.x.atan2(-dir.z), 0.0)
    }
    pub fn forward(&self) -> Vec3 {
        Vec3::new(self.yaw.sin() * self.pitch.cos(), self.pitch.sin(), -self.yaw.cos() * self.pitch.cos())
    }
    pub fn right(&self) -> Vec3 {
        Vec3::new(self.yaw.cos(), 0.0, self.yaw.sin())
    }
}

/// CLASSIC car shading (the original's 0 Flat, 1 Gouraud, 2 Reflection, in this order as in its
/// config; globals 0x4ebfd8 / 0x4b014c, 05-formats "Flat car shading"). Flat is Classic only: Remaster
/// draws it as Gouraud. Remaster's Enhanced paint is `RemasterView::enhanced_paint`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum CarShading {
    Flat,
    Gouraud,
    Reflection,
}

impl CarShading {
    pub const ALL: [CarShading; 3] = [CarShading::Flat, CarShading::Gouraud, CarShading::Reflection];
    pub const NAMES: [&'static str; 3] = ["Flat", "Gouraud", "Reflection"];
    pub fn name(self) -> &'static str {
        Self::NAMES[self as usize]
    }
    pub fn parse(s: &str) -> Option<Self> {
        Self::NAMES.iter().position(|n| n.eq_ignore_ascii_case(s)).map(|i| Self::ALL[i])
    }
    /// the next (or previous) mode, wrapping
    pub fn cycle(self, fwd: bool) -> Self {
        let n = Self::ALL.len();
        Self::ALL[if fwd { (self as usize + 1) % n } else { (self as usize + n - 1) % n }]
    }
}

/// CLASSIC car detail (the original's `CarDetail`, 0 Low, 1 Medium, 2 High; config `+0x9cc`). It
/// sets the first body mesh level (`0x45effe` passes 2 - CarDetail to `0x40b888`, which loads
/// `OBJ[max(i, 2 - CarDetail)]` for level i): High OBJ0 then OBJ1, Medium OBJ1, Low OBJ2, whose
/// wheels are built in (Low attaches no wheel objects, `0x45f436`). The D3D and Glide renderers draw
/// only levels 0 and 1 (`0x4ebedc` stays 1 in the software renderer alone), 05-formats "MYO". Classic
/// only; Remaster always draws the full model.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum CarDetail {
    Low,
    Medium,
    High,
}

/// CLASSIC the near/far switch of the body levels, metres at zoom 1 on a 512 px wide screen
/// (`0x4cf243` = 20.0 times level + 1, `0x4d2fd5` = 1/512, `0x4395a8..0x4395d4`).
const LOD_SWITCH: f32 = 20.0;
const LOD_WIDTH: f32 = 512.0;

/// REMASTER (design choice): a generated car is drawn with its original model (OBJ0 and the original
/// wheels) once it is this many pixels long on screen at "Car detail distance" 100 %; the percent
/// scales the switch distance (`RemasterView::CAR_LOD_DISTANCE`). The length is the cars' about 4.5 m.
const GEN_SWITCH_PX: f32 = 30.0;
const GEN_CAR_LENGTH: f32 = 4.5;
/// the switch distance holds a band of this fraction either side, so a car on the line does not flicker
const GEN_SWITCH_BAND: f32 = 0.15;

/// REMASTER: the distance in metres past which a generated car is drawn with the original model, at
/// `percent` of the default, for a screen `width` pixels wide and camera `zoom`.
fn gen_switch_distance(percent: u32, zoom: f32, width: f32) -> f32 {
    let px_per_m = zoom * width * 0.5 / (FOV_X.to_radians() * 0.5).tan();
    GEN_CAR_LENGTH * px_per_m / GEN_SWITCH_PX * percent as f32 / 100.0
}

/// REMASTER: whether a car at `dist` metres takes the original model: past the switch distance, with the
/// band either side keeping the `was_far` choice of the last frame (None: the plain threshold).
fn gen_far(dist: f32, switch: f32, was_far: Option<bool>) -> bool {
    match was_far {
        Some(was) if dist > switch * (1.0 - GEN_SWITCH_BAND) && dist < switch * (1.0 + GEN_SWITCH_BAND) => was,
        _ => dist > switch,
    }
}

impl CarDetail {
    pub const ALL: [CarDetail; 3] = [CarDetail::Low, CarDetail::Medium, CarDetail::High];
    pub const NAMES: [&'static str; 3] = ["Low", "Medium", "High"];
    pub fn name(self) -> &'static str {
        Self::NAMES[self as usize]
    }
    pub fn parse(s: &str) -> Option<Self> {
        Self::NAMES.iter().position(|n| n.eq_ignore_ascii_case(s)).map(|i| Self::ALL[i])
    }
    /// The body mesh (0 = OBJ0, 1 = OBJ1, 2 = OBJ2) at `dist` metres from the camera: level 0 while
    /// `20 * zoom * width / 512 >= dist`, else level 1, both raised to the detail's first level.
    pub fn body_level(self, dist: f32, zoom: f32, width: f32) -> usize {
        let far = LOD_SWITCH * zoom * width / LOD_WIDTH < dist;
        (far as usize).max(2 - self as usize)
    }
}

/// Graphics mode (09-remaster): Classic draws like the original; Remaster adds our modern
/// features on top of the same scene. One key (F9) flips it at any time.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum RenderMode {
    Classic,
    Remaster,
}

impl RenderMode {
    pub const NAMES: [&'static str; 2] = ["Classic", "Remaster"];
    pub fn name(self) -> &'static str {
        Self::NAMES[self as usize]
    }
    pub fn parse(s: &str) -> Option<Self> {
        Self::NAMES.iter().position(|n| n.eq_ignore_ascii_case(s)).map(|i| [RenderMode::Classic, RenderMode::Remaster][i])
    }
    pub fn toggle(self) -> Self {
        match self {
            RenderMode::Classic => RenderMode::Remaster,
            RenderMode::Remaster => RenderMode::Classic,
        }
    }
}

/// REMASTER options, used only in Remaster mode (none are in the original). Every per-mode
/// choice goes here (09-remaster 9.4 "cheap wins" and 9.9).
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct RemasterView {
    /// Enhanced car paint: per-pixel clear coat over a cube map of the real surroundings
    /// captured around the followed car each frame [design choice]
    pub enhanced_paint: bool,
    /// Dynamic lights: blinkers and flashing cones light their surroundings [design choice]
    pub dynamic_lights: bool,
    /// anti-aliasing of the render stage (MSAA / SSAA, `msaa()`); MSAA also gives alpha to
    /// coverage on colour-keyed textures [design choice]
    pub render_aa: RenderAa,
    /// anti-aliasing of the post-processing stage (FXAA / SMAA / TAA); combines with `render_aa`
    pub post_aa: PostAa,
    /// texture anisotropy (one of `ANISOTROPY_LEVELS`; 1 = trilinear only)
    pub anisotropy: u16,
    /// cars and scene objects lit per pixel (off: per vertex like Classic)
    pub pixel_light: bool,
    /// fog per pixel (off: per vertex)
    pub pixel_fog: bool,
    /// fog curve (Enhanced needs `pixel_fog`) and its constants
    pub fog_style: FogStyle,
    /// None = the track's own values (`Look::fog`); Some = an override (F11, `--fog-tune`)
    pub fog_tune: Option<FogTune>,
    /// clear coat (Fresnel) on Reflection paint
    pub clear_coat: bool,
    /// post pass strengths in percent (post.wgsl): colour grade, vignette, film grain;
    /// the pass runs when any is above 0
    pub grade: u32,
    pub vignette: u32,
    pub grain: u32,
    /// HDR: the full-screen view is composited in linear light into a float target, then
    /// tonemapped (09-remaster 9.10); off: gamma space like Classic
    pub hdr: bool,
    pub tonemap: Tonemap,
    /// HDR exposure in percent (`EXPOSURE_RANGE`)
    pub exposure: u32,
    /// HDR bloom strength in percent (0 = off)
    pub bloom: u32,
    /// ambient occlusion in percent (0 = off): contact darkening from the depth buffer
    /// (ssao.wgsl, 09-remaster 9.28)
    pub ssao: u32,
    /// halos drawn with the analytic shape (`fs_halo_remaster`, 09-remaster 9.11); off: the
    /// original halo sprites
    pub smooth_halos: bool,
    /// lens flares computed in the shader (`fs_halo_remaster`, 09-remaster 9.11 step 4), from
    /// lamps, car lights and blinkers; off: the hex.iff flares of Classic
    pub smooth_flares: bool,
    /// retro-reflective road signs: the headlights flare off them at night, a faint sheen by day
    /// (`retro_light`, 09-remaster 9.14)
    pub retro_signs: bool,
    /// the modern race HUD (09-remaster 9.21): TrueType fonts and distance fields at the
    /// window's resolution; off: the bitmaps scaled up like Classic
    pub modern_hud: bool,
    /// the modern front end (09-remaster 9.24): vector dial, glass plate and Motorhead title
    /// font at the window's aspect; off: the original menu art. Screens migrate one by one.
    pub modern_menu: bool,
    /// cars cast real sun shadows from a shadow map onto the world and each other (09-remaster
    /// 9.15); off: the original's projected shadow
    pub car_shadows: bool,
    /// with the MotionBlur option on: per-pixel velocity blur (camera from depth, cars from
    /// their own motion; 09-remaster 9.17) in place of the original's frame blend
    pub velocity_blur: bool,
    /// with the SkidMarks option on: tread-textured marks that fade in and out, darker where the
    /// slide is harder (`fs_skid`), in place of the original's flat strips
    pub skids_detailed: bool,
    /// car models generated with more polygons and smooth normals in place of the MYO meshes
    /// (09-remaster 9.20; the wheels so far); off: the original meshes
    pub generated_cars: bool,
    /// generated world fill pieces (12-world-fill) drawn after the world
    pub world_fill: bool,
    /// the procedural water surfaces (16-water) in place of the original's flat water textures;
    /// off: the textures. The water bodies are read from the fill file whatever World fill says
    pub water: bool,
    /// the water mirrors the scene around it (screen-space reflections from the water pass's
    /// scene copy, 16-water "SSR"); off: the sky reflection only
    pub water_ssr: bool,
    /// lit objects (cars, props) take their ambient light from the irradiance probes along the
    /// road (#83, probes.rs): darker in tunnels, tinted by the lit walls around them
    pub local_light: bool,
    /// the user's texture pack (#211, texpack.rs): upscaled colour, normal and roughness maps over
    /// the original textures; off: the originals. No effect when the install has no pack
    pub texture_pack: bool,
    pub local_tune: LocalLightTune,
    /// the modern HUD's glass rev meter (09-remaster 9.21 phase 4): glow, glass, halo and
    /// cross-section
    pub rev_meter: crate::revmeter::RevMeterTune,
    /// the sky dome from the track's HDRI (`remaster/sky/trackNN.exr`, 09-remaster 9.32) in place
    /// of the sky band; tracks without a file keep the band
    pub sky_hdri: bool,
    pub sky_tune: SkyTune,
    /// head and tail light halos, bursts, spots and flares (F11 window)
    pub car_lights: CarLightTune,
    /// street lamps as downward lights on cars (F11 window)
    pub lamp_lights: LampLightTune,
    /// which street lamp and headlight shadow maps are drawn (11-dynamic-shadows phase 4)
    pub light_shadows: LightShadows,
    /// map size and filtering of the lamp and headlight shadows
    pub shadow_quality: ShadowQuality,
    /// shell grass over the world's grass textures (procgen/grass.rs, 18-grass.md): off, or the
    /// shell count and reach of the bodies scaled by the level
    pub grass: GrassLevel,
    /// how far out the grass reaches, percent (`GRASS_RANGE`): scales the shells' reach and, at
    /// Ultra, the blades' (18-grass.md "Option")
    pub grass_range: u32,
    /// REMASTER: how far away a generated car switches to its original model, percent of the default
    /// (`CAR_LOD_DISTANCE`, `gen_switch_distance`); 400 % is about 7 px, in practice never
    pub car_lod_distance: u32,
    /// how thick the Enhanced / Volumetric fog is, percent (`FOG_STRENGTH`): scales the height haze's
    /// density and pulls the distance fog's start and full distance in or out (17-fog-rework.md "Fog strength")
    pub fog_strength: u32,
}

impl Default for RemasterView {
    fn default() -> Self {
        RemasterView {
            enhanced_paint: true,
            dynamic_lights: true,
            render_aa: RenderAa { kind: RenderAaKind::Msaa, strength: AA_STOP_STRENGTH[2] },
            post_aa: PostAa { kind: PostAaKind::Off, strength: AA_STOP_STRENGTH[2] },
            anisotropy: 16,
            pixel_light: true,
            pixel_fog: true,
            fog_style: FogStyle::Enhanced,
            fog_tune: None,
            clear_coat: true,
            grade: 0,
            vignette: 0,
            grain: 0,
            hdr: true,
            tonemap: Tonemap::Neutral,
            exposure: 100,
            bloom: 50,
            ssao: 50,
            smooth_halos: true,
            smooth_flares: true,
            retro_signs: true,
            modern_hud: true,
            modern_menu: true,
            car_shadows: true,
            velocity_blur: true,
            skids_detailed: true,
            generated_cars: true,
            world_fill: true,
            water: true,
            water_ssr: true,
            // [design choice, #83 phase 4]: on; costs about 0.1 ms a frame at 1080p and 0.1 s at
            // load from the cache (the first run of a track bakes in the background)
            local_light: true,
            texture_pack: true,
            local_tune: Default::default(),
            sky_hdri: true,
            sky_tune: Default::default(),
            rev_meter: Default::default(),
            car_lights: Default::default(),
            lamp_lights: Default::default(),
            light_shadows: LightShadows::All,
            shadow_quality: ShadowQuality::High,
            grass: GrassLevel::Full,
            grass_range: 100,
            car_lod_distance: 100,
            fog_strength: 100,
        }
    }
}

/// REMASTER grass level (18-grass.md) [design choice]: Short halves the bodies' shells and
/// reach (8 shells, 40 m at the defaults), Full keeps them (16, 80 m), Ultra adds the blade
/// geometry near the camera (Gitea #31; `Gpu::blade_pass`).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum GrassLevel {
    Off,
    Short,
    Full,
    Ultra,
}

impl GrassLevel {
    pub const ALL: [GrassLevel; 4] = [GrassLevel::Off, GrassLevel::Short, GrassLevel::Full, GrassLevel::Ultra];
    pub const NAMES: [&'static str; 4] = ["Off", "Short", "Full", "Ultra"];
    pub fn name(self) -> &'static str {
        Self::NAMES[self as usize]
    }
    pub fn parse(s: &str) -> Option<Self> {
        Self::NAMES.iter().position(|n| n.eq_ignore_ascii_case(s)).map(|i| Self::ALL[i])
    }
    /// the scale of the bodies' shell counts and reach
    pub fn factor(self) -> f32 {
        match self {
            GrassLevel::Off => 0.0,
            GrassLevel::Short => 0.5,
            GrassLevel::Full | GrassLevel::Ultra => 1.0,
        }
    }
    /// the Ultra blades are placed and drawn
    pub fn blades(self) -> bool {
        self == GrassLevel::Ultra
    }
}

/// REMASTER which light shadows are drawn at night (11-dynamic-shadows phase 4): none, only
/// the headlights of cars, or the street lamps too [design choice].
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum LightShadows {
    Off,
    Headlights,
    All,
}

impl LightShadows {
    pub const ALL: [LightShadows; 3] = [LightShadows::Off, LightShadows::Headlights, LightShadows::All];
    pub const NAMES: [&'static str; 3] = ["Off", "Headlights", "All"];
    pub fn name(self) -> &'static str {
        Self::NAMES[self as usize]
    }
    /// a name (`head` for the CLI) or the setting's text
    pub fn parse(s: &str) -> Option<Self> {
        if s.eq_ignore_ascii_case("head") {
            return Some(LightShadows::Headlights);
        }
        Self::NAMES.iter().position(|n| n.eq_ignore_ascii_case(s)).map(|i| Self::ALL[i])
    }
}

/// REMASTER quality of the light shadows (11-dynamic-shadows.md 11.9): the edge of the maps
/// and the width of the PCF filter [design choice]. Each atlas size is built on first use.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ShadowQuality {
    Low,
    Medium,
    High,
    Ultra,
}

impl ShadowQuality {
    pub const ALL: [ShadowQuality; 4] = [ShadowQuality::Low, ShadowQuality::Medium, ShadowQuality::High, ShadowQuality::Ultra];
    pub const NAMES: [&'static str; 4] = ["Low", "Medium", "High", "Ultra"];
    pub fn name(self) -> &'static str {
        Self::NAMES[self as usize]
    }
    pub fn parse(s: &str) -> Option<Self> {
        Self::NAMES.iter().position(|n| n.eq_ignore_ascii_case(s)).map(|i| Self::ALL[i])
    }
    /// the old `RemasterLightShadowSize` setting: the nearest quality with a map of that edge
    pub fn from_size(n: u32) -> Self {
        if n <= 512 { ShadowQuality::Low } else if n <= 1024 { ShadowQuality::Medium } else { ShadowQuality::High }
    }
    /// index in `SPOT_SIZES` of this quality's atlas
    fn atlas(self) -> usize {
        [0, 1, 2, 2][self as usize]
    }
    /// PCF radius in texels (1: 3x3, 2: 5x5)
    fn pcf(self) -> u32 {
        if self == ShadowQuality::Ultra { 2 } else { 1 }
    }
}

/// REMASTER tonemapper of the HDR view (post.wgsl). Neutral (Khronos PBR Neutral) leaves
/// colours below about 0.8 as they are, so the tracks keep their look; ACES and AgX are
/// filmic and change contrast and saturation [design choice].
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Tonemap {
    Neutral,
    Aces,
    Agx,
}

impl Tonemap {
    pub const ALL: [Tonemap; 3] = [Tonemap::Neutral, Tonemap::Aces, Tonemap::Agx];
    pub const NAMES: [&'static str; 3] = ["Neutral", "ACES", "AgX"];
    pub fn name(self) -> &'static str {
        Self::NAMES[self as usize]
    }
    pub fn parse(s: &str) -> Option<Self> {
        Self::NAMES.iter().position(|n| n.eq_ignore_ascii_case(s)).map(|i| Self::ALL[i])
    }
}

/// REMASTER render-stage anti-aliasing: what is done while the scene is drawn.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum RenderAaKind {
    Off,
    /// multisampling, 2 / 4 / 8 samples
    Msaa,
    /// supersampling: the scene is drawn at 1.25-2x the resolution and filtered down
    /// (DSR is the same thing, so it has no entry of its own)
    Ssaa,
}

impl RenderAaKind {
    /// the kinds the options offer (grows as the methods are implemented)
    pub const AVAILABLE: [RenderAaKind; 3] = [RenderAaKind::Off, RenderAaKind::Msaa, RenderAaKind::Ssaa];
    pub const NAMES: [&'static str; 3] = ["Off", "MSAA", "SSAA"];
    pub fn name(self) -> &'static str {
        Self::NAMES[Self::AVAILABLE.iter().position(|&k| k == self).unwrap_or(0)]
    }
    pub fn parse(s: &str) -> Option<Self> {
        Self::NAMES.iter().position(|n| n.eq_ignore_ascii_case(s)).map(|i| Self::AVAILABLE[i])
    }
}

/// REMASTER the strength scale both AA groups share: four stops, 2x / 4x / 8x / 16x (Off is the
/// type dropdown's value and hides the slider). The strength stays a percent in the settings
/// file; a quarter of it is one stop, the slider writes `AA_STOP_STRENGTH` [design choice].
pub const AA_STOPS: [&str; 4] = ["2x", "4x", "8x", "16x"];
/// the strength a slider stop stores
pub const AA_STOP_STRENGTH: [u32; 4] = [12, 37, 62, 87];
/// the stop (0..=3) a strength percent falls in
pub fn aa_stop(strength: u32) -> usize {
    (strength.min(100) / 25).min(3) as usize
}
/// a strength from the settings file or the command line: a stop ("8x") or a percent ("62")
pub fn parse_aa_strength(s: &str) -> Option<u32> {
    match AA_STOPS.iter().position(|n| n.eq_ignore_ascii_case(s)) {
        Some(i) => Some(AA_STOP_STRENGTH[i]),
        None => s.parse::<u32>().ok().map(|p| p.min(100)),
    }
}

/// A render-stage method with its strength slider (a stop of `AA_STOPS`).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct RenderAa {
    pub kind: RenderAaKind,
    pub strength: u32,
}

impl RenderAa {
    /// the sample count the stop stands for: 2 / 4 / 8 / 16 (`Gpu::msaa_samples` clamps it to what
    /// the GPU supports); SSAA shades that many samples per pixel too, see `scale`
    pub fn stop_samples(self) -> u32 {
        2 << aa_stop(self.strength)
    }
    /// MSAA sample count (1 for the other kinds)
    pub fn samples(self) -> u32 {
        match self.kind {
            RenderAaKind::Msaa => self.stop_samples(),
            _ => 1,
        }
    }
    /// SSAA render scale per axis: the stop's sample count is the pixel count, so 2x is 1.41 per
    /// side, 4x is 2 (the usual 2x2 grid), 16x is 4 [design choice]; 1 for the other kinds. The
    /// scale is one of four so the target sizes stay put while the slider moves; the cost is
    /// `scale²` in pixels, see `SSAA_MAX_PIXELS`.
    pub fn scale(self) -> f32 {
        match self.kind {
            RenderAaKind::Ssaa => (self.stop_samples() as f32).sqrt(),
            _ => 1.0,
        }
    }
    /// the strength that gives `n` MSAA samples (a settings file from before the two-stage options)
    pub fn msaa(n: u32) -> Self {
        match n {
            0 | 1 => RenderAa { kind: RenderAaKind::Off, strength: AA_STOP_STRENGTH[2] },
            2 | 3 => RenderAa { kind: RenderAaKind::Msaa, strength: AA_STOP_STRENGTH[0] },
            4..=7 => RenderAa { kind: RenderAaKind::Msaa, strength: AA_STOP_STRENGTH[1] },
            8..=15 => RenderAa { kind: RenderAaKind::Msaa, strength: AA_STOP_STRENGTH[2] },
            _ => RenderAa { kind: RenderAaKind::Msaa, strength: AA_STOP_STRENGTH[3] },
        }
    }
    /// the strength slider's text
    pub fn strength_text(self) -> String {
        match self.kind {
            RenderAaKind::Off => "n/a".into(),
            _ => AA_STOPS[aa_stop(self.strength)].into(),
        }
    }
}

/// REMASTER post-processing anti-aliasing: what is done to the finished image.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PostAaKind {
    Off,
    Fxaa,
    Smaa,
    Taa,
}

impl PostAaKind {
    /// the kinds the options offer (grows as the methods are implemented)
    pub const AVAILABLE: [PostAaKind; 4] = [PostAaKind::Off, PostAaKind::Fxaa, PostAaKind::Smaa, PostAaKind::Taa];
    pub const NAMES: [&'static str; 4] = ["Off", "FXAA", "SMAA", "TAA"];
    pub fn name(self) -> &'static str {
        Self::NAMES[Self::AVAILABLE.iter().position(|&k| k == self).unwrap_or(0)]
    }
    pub fn parse(s: &str) -> Option<Self> {
        Self::NAMES.iter().position(|n| n.eq_ignore_ascii_case(s)).map(|i| Self::AVAILABLE[i])
    }
}

/// REMASTER TAA: strength of the unsharp mask after the resolve [design choice].
const TAA_SHARPEN: f32 = 0.2;
/// REMASTER TAA: the sub-pixel offsets of the eight-frame Halton (2, 3) sequence, in pixels
/// from the pixel centre (-0.5..0.5).
const TAA_JITTER: [[f32; 2]; 8] = [[0.0, -0.1667], [-0.25, 0.1667], [0.25, -0.3889], [-0.375, -0.0556], [0.125, 0.2778], [-0.125, -0.2778], [0.375, 0.0556], [-0.4375, 0.3889]];

/// A post-processing method with its strength slider (a stop of `AA_STOPS`).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct PostAa {
    pub kind: PostAaKind,
    pub strength: u32,
}

impl PostAa {
    /// a post method is on
    pub fn on(self) -> bool {
        self.kind != PostAaKind::Off
    }
    /// the strength's stop, 0..=3 (2x .. 16x)
    pub fn stop(self) -> usize {
        aa_stop(self.strength)
    }
    /// FXAA subpixel quality over the stop: 0.25 / 0.5 / 0.75 / 1.0; the edge thresholds stay at
    /// the quality preset [design choice]
    pub fn params(self) -> f32 {
        0.25 * (self.stop() + 1) as f32
    }
    /// SMAA preset over the stop: Low, Medium, High, Ultra
    /// (edge threshold, longest line searched in pixels, corner rounding cut); the reference
    /// presets' thresholds and search lengths (2 px per step) [design choice]. Corner rounding is
    /// on for High and Ultra like the reference (SMAA_CORNER_ROUNDING 25: a corner keeps 25 % of
    /// its blend, so the cut is 0.75)
    pub fn smaa_preset(self) -> (&'static str, [f32; 3]) {
        match self.stop() {
            0 => ("Low", [0.15, 8.0, 0.0]),
            1 => ("Medium", [0.10, 16.0, 0.0]),
            2 => ("High", [0.10, 32.0, 0.75]),
            _ => ("Ultra", [0.05, 64.0, 0.75]),
        }
    }
    /// TAA over the stop: the history's share of the blend, 0.85 to 0.95, and the sharpen that
    /// undoes the resolve's softening [design choice]
    pub fn taa_params(self) -> [f32; 2] {
        [0.85 + 0.10 * self.stop() as f32 / 3.0, TAA_SHARPEN]
    }
    pub fn strength_text(self) -> String {
        match self.kind {
            PostAaKind::Off => "n/a".into(),
            _ => AA_STOPS[self.stop()].into(),
        }
    }
}

impl RemasterView {
    /// MSAA sample count of the full-screen views (1 = off). Drawn with the largest count the
    /// GPU supports (`msaa_samples`).
    pub fn msaa(&self) -> u32 {
        self.render_aa.samples()
    }
    /// SSAA render scale per axis (1 = off), clamped to the device by `Gpu::render_size`
    pub fn render_scale(&self) -> f32 {
        self.render_aa.scale()
    }
}

impl RemasterView {
    pub const MSAA_LEVELS: [u32; 5] = [1, 2, 4, 8, 16];
    pub const ANISOTROPY_LEVELS: [u16; 5] = [1, 2, 4, 8, 16];
    /// exposure option range and step, percent
    pub const EXPOSURE_RANGE: (u32, u32) = (50, 200);
    /// grass range option range, percent (10 % steps)
    pub const GRASS_RANGE: (u32, u32) = (50, 250);
    /// car detail distance option range, percent (10 % steps)
    pub const CAR_LOD_DISTANCE: (u32, u32) = (25, 400);
    /// fog strength option range, percent (10 % steps)
    pub const FOG_STRENGTH: (u32, u32) = (20, 200);
    /// the grade / vignette / grain part of the post pass is in use
    pub fn post(&self) -> bool {
        self.grade + self.vignette + self.grain > 0
    }
    /// the main view goes through the post pass (HDR tonemap or the grade)
    fn post_pass(&self) -> bool {
        self.hdr || self.post() || self.post_aa.on()
    }
    /// `env.w` of the shader globals: 1 per-pixel light, 2 per-pixel fog, 4 clear coat,
    /// 16 smooth halos, 64 smooth flares, 256 reflective signs, 1024 Enhanced fog, 4096 texture pack (8, 32, 2048 and 8192
    /// are set per view: `LINEAR_FLAG`, `HALO_BLOOM_FLAG`, `VOLFOG_FLAG`, `FLAT_FLAG`)
    fn shader_flags(&self) -> f32 {
        (self.pixel_light as u32 | (self.pixel_fog as u32) << 1 | (self.clear_coat as u32) << 2 | (self.smooth_halos as u32) << 4 | (self.smooth_flares as u32) << 6 | (self.retro_signs as u32) << 8 | (self.enhanced_fog() as u32) << 10 | (self.texture_pack as u32) << 12) as f32
    }
    /// the Enhanced fog curve is in use (it is per pixel; Volumetric keeps it for the particles after the volume)
    pub fn enhanced_fog(&self) -> bool {
        self.fog_style != FogStyle::Classic && self.pixel_fog
    }
    /// smooth halos in an HDR view with bloom: a small hot core, the glow comes from the bloom
    fn halo_bloom(&self) -> bool {
        self.smooth_halos && self.bloom > 0
    }
}

/// The view-distance option's top in Classic (0x428cfc / 0x428e88 clamp at 150 %); Remaster
/// goes up to `View::MAX_DISTANCE` [design choice].
pub const CLASSIC_MAX_DISTANCE: f32 = 1.5;

#[derive(Clone, Copy)]
pub struct View {
    pub brightness: f32,
    pub fog: bool,
    pub show_line: bool,
    pub shadows: bool,
    pub halos: bool,
    pub car_shading: CarShading,
    /// CLASSIC body mesh levels of the cars
    pub car_detail: CarDetail,
    /// view distance factor (the in-race GFX option, 30..150 %, Remaster to 200 %; read it
    /// through `distance()`)
    pub distance: f32,
    /// DEBUG: horizontal field of view in degrees at camera zoom 1 (`FOV_X` = the original 90)
    pub fov_x: f32,
    /// SkyImage: draw the BackgroundName band (off: the Top/BottomColour gradient only)
    pub sky_image: bool,
    /// MotionBlur: the sky is blended over the previous frame instead of replacing it
    pub motion_blur: bool,
    /// LensFlares: flare chains from the world halos near the screen centre
    pub lens_flares: bool,
    /// Smoke / Sparks: tyre smoke and wall sparks (spawned by `fx::Effects`)
    pub smoke: bool,
    pub sparks: bool,
    pub mode: RenderMode,
    pub remaster: RemasterView,
    /// DEBUG: motorview's render debug views (the game keeps the default: no change)
    pub debug: DebugView,
}

/// DEBUG: what a mesh pixel shows instead of its colour (shader.wgsl `debug_view`, `spot_n.y`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum DebugShade {
    #[default]
    Off,
    /// shading normal in game axes (x red, y green, z blue)
    Normals,
    /// 8x8 checker per texture, tinted by u (red) and v (green)
    UvChecker,
    /// the texture alone: no vertex colour, light or fog
    Texture,
    /// the vertex colour and light alone: no texture or fog
    Lighting,
    /// distance from the eye, log scale (white near, black at 2 km)
    Depth,
    /// REMASTER: each lamp shadow layer's visibility as a colour (white = lit by all)
    LampShadow,
}

impl DebugShade {
    pub const ALL: [DebugShade; 7] = [DebugShade::Off, DebugShade::Normals, DebugShade::UvChecker, DebugShade::Texture, DebugShade::Lighting, DebugShade::Depth, DebugShade::LampShadow];
    pub fn name(self) -> &'static str {
        ["off", "normals", "uv", "texture", "lighting", "depth", "lampshadow"][self as usize]
    }
    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|m| m.name() == s)
    }
    pub fn next(self) -> Self {
        Self::ALL[(self as usize + 1) % Self::ALL.len()]
    }
}

/// DEBUG: motorview's render options. The default draws exactly what the game draws.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct DebugView {
    pub shade: DebugShade,
    /// far plane at `UNLIMITED_FAR` (the fog keeps the track's distances)
    pub unlimited: bool,
    pub hide_sky: bool,
    pub hide_world: bool,
    /// REMASTER: the Ultra grass blades left out (the shells stay)
    pub hide_blades: bool,
    /// REMASTER: no sun shafts in the volumetric fog (the fog shadow map is not drawn)
    pub hide_shafts: bool,
    /// DEBUG: the sky replaced by this solid colour (gamma-space 0..255; the env probe keeps the real sky)
    pub backdrop: Option<[u8; 3]>,
}

/// DEBUG: the far plane of `DebugView::unlimited` (metres). The depth precision at a distance
/// depends on the near plane, not on this.
pub const UNLIMITED_FAR: f32 = 100_000.0;

impl View {
    /// REMASTER: the view-distance option reaches 400 % (the fog moves out with it; the world
    /// fill, 12-world-fill, covers what lies past the original map)
    pub const MAX_DISTANCE: f32 = 4.0;
    /// the view distance factor in use: Classic caps it at 150 %
    pub fn distance(&self) -> f32 {
        match self.mode {
            RenderMode::Classic => self.distance.min(CLASSIC_MAX_DISTANCE),
            RenderMode::Remaster => self.distance,
        }
    }
    /// REMASTER: Enhanced car paint is in use
    pub fn enhanced(&self) -> bool {
        self.mode == RenderMode::Remaster && self.remaster.enhanced_paint
    }
    /// REMASTER: cars cast real sun shadows (shadow map) in place of the projected ones
    pub fn car_shadows(&self) -> bool {
        self.mode == RenderMode::Remaster && self.remaster.car_shadows && self.shadows
    }
    /// REMASTER: MotionBlur is the velocity blur (`VelBlur`) instead of the frame blend
    pub fn velocity_blur(&self) -> bool {
        self.mode == RenderMode::Remaster && self.remaster.velocity_blur && self.motion_blur
    }
    /// REMASTER: skid marks are the detailed ones (`Pass::Skid`)
    pub fn skids_detailed(&self) -> bool {
        self.mode == RenderMode::Remaster && self.remaster.skids_detailed
    }
    /// REMASTER volumetric fog (`VolFog`) wanted on the main view; it also needs the HDR target
    pub fn fog_volume(&self) -> bool {
        self.mode == RenderMode::Remaster && self.fog && self.remaster.fog_style == FogStyle::Volumetric
    }
    /// REMASTER ambient occlusion on the main view (`Ssao`)
    pub fn ssao(&self) -> bool {
        self.mode == RenderMode::Remaster && self.remaster.ssao > 0
    }
    /// REMASTER generated car models (`Scene::remaster_cars`) in place of the original meshes
    pub fn generated_cars(&self) -> bool {
        self.mode == RenderMode::Remaster && self.remaster.generated_cars
    }
    /// REMASTER: the world fill pieces are drawn
    pub fn world_fill(&self) -> bool {
        self.mode == RenderMode::Remaster && self.remaster.world_fill
    }
    /// REMASTER: the procedural water surfaces replace the water textures (Detail > Water,
    /// `--water`; the bodies are in the world fill file, which is loaded either way)
    pub fn water(&self) -> bool {
        self.mode == RenderMode::Remaster && self.remaster.water
    }
    /// REMASTER: lit objects take their ambient from the probe volume (#83, Detail > Local light)
    pub fn local_light(&self) -> bool {
        self.mode == RenderMode::Remaster && self.remaster.local_light
    }
    /// REMASTER: the water's screen-space reflections (water.wgsl `water_ssr`)
    pub fn water_ssr(&self) -> bool {
        self.water() && self.remaster.water_ssr
    }
    /// REMASTER: the grass shells (`Pass::Grass`), Off in Classic
    pub fn grass(&self) -> GrassLevel {
        match self.mode {
            RenderMode::Remaster => self.remaster.grass,
            RenderMode::Classic => GrassLevel::Off,
        }
    }
    /// REMASTER: the Ultra grass blades are placed and drawn (`Gpu::blade_pass`)
    pub fn grass_blades(&self) -> bool {
        self.grass().blades() && !self.debug.hide_blades
    }
    /// REMASTER: dynamic lights are in use
    pub fn dynamic_lights(&self) -> bool {
        self.mode == RenderMode::Remaster && self.remaster.dynamic_lights
    }
    /// REMASTER: the modern race HUD (drawn in a `Batch::sharp`)
    pub fn modern_hud(&self) -> bool {
        self.mode == RenderMode::Remaster && self.remaster.modern_hud
    }

    /// REMASTER front end (`RemasterView::modern_menu`)
    pub fn modern_menu(&self) -> bool {
        self.mode == RenderMode::Remaster && self.remaster.modern_menu
    }
}

/// MotionBlur (driver +0x284, set from config +0x9dc at 0x458503): the frame is not cleared and
/// the sky is drawn with constant alpha 0x80 over the old picture (Glide 0x48c8d3:
/// grConstantColorValue(0x80000000), blend ONE_MINUS_SRC_ALPHA / SRC_ALPHA; D3D 0x482b94 skips
/// the viewport clear and blends SRCALPHA / INVSRCALPHA). The world is drawn opaque on top, so
/// the old frame shows through only where the sky is visible now.
const MOTION_BLUR_ALPHA: f64 = 0.5;

/// Near plane of the views (the env probe has its own).
const NEAR: f32 = 0.2;

/// REMASTER velocity blur [design choice]: the blur length in frames of motion (1.5: stylised,
/// three times a 180 degree film shutter, which was too subtle to notice; per frame, so the
/// blur gets shorter at higher frame rates), the longest blur as a share of the target width, and the camera or car jump
/// (metres, cos of the turn) that counts as a cut: no blur that frame.
const VBLUR_SHUTTER: f32 = 1.5;
const VBLUR_MAX: f32 = 0.1;
pub(crate) const CUT_DISTANCE: f32 = 15.0;
pub(crate) const CUT_COS: f32 = 0.8;

/// The previous frame's scene for MotionBlur (the original reuses its back buffer).
struct History {
    tex: wgpu::Texture,
    bind: wgpu::BindGroup,
    /// holds a frame of the current scene
    valid: bool,
}

#[derive(Clone, Copy, PartialEq)]
enum Pass {
    Opaque,
    /// world, scene objects and cars: back faces culled (0x405b28), wound for a mirrored view
    /// when `true`
    Mesh(bool),
    Sky,
    /// the sky in an env-probe face (by direction)
    SkyCube,
    /// REMASTER sky dome: the HDRI by direction (`fs_sky_dome`)
    SkyDome,
    /// REMASTER sky dome in an env-probe face (`fs_sky_cube_dome`)
    SkyCubeDome,
    /// CHEAT the cloud-ceiling sky of lemmy / ace (#316) and ramlosa / h2o (#317) (`vs_hell_sky` / `fs_hell_sky`), 12 vertices
    HellSky,
    Shadow,
    /// REMASTER detailed skid marks (`fs_skid`): blended like the shadows, tread-textured
    Skid,
    /// CLASSIC skid marks (`fs_skid_flat`, also Remaster "On"): flat quads that multiply the frame
    /// (0x442911: SRCBLEND ZERO, DESTBLEND SRCCOLOR, no fog, no z write, z bias)
    SkidFlat,
    Halo,
    /// REMASTER: lit, alpha-blended smoke and dust (09-remaster 9.16)
    Smoke,
    /// REMASTER: procedural additive spark streaks (09-remaster 9.16)
    Spark,
    /// CHEAT tribute to tron, REMASTER Neon grid (#320): the cars' light walls (`vs_skid` / `fs_light_wall`), additive,
    /// depth tested, not written, both faces
    LightWall,
    /// DEBUG: lines drawn over everything (no depth test or write; motorview gizmos)
    Xray,
    /// REMASTER procedural water surface: alpha-blended, depth tested, not written
    Water,
    /// REMASTER grass shells (grass.wgsl): opaque, depth tested and written, both faces, alpha
    /// to coverage with MSAA
    Grass,
    /// REMASTER hologram edges / shell: additive, no depth (`Frame::hologram`)
    Holo,
    HoloFill,
    /// CLASSIC ghost car (Time Attack, design choice): the car pipeline blended with a constant
    /// alpha (`GHOST_ALPHA`), depth tested, not written
    Ghost,
    /// DEBUG wire styles (`vs_flat` / `fs_flat`, wire.rs): edges, depth tested
    Wire,
    /// DEBUG wire styles: the fill under the edges, pushed back so the edges win the depth test
    WireFill,
    /// DEBUG wire styles: edges seen through the mesh (non-manifold edges)
    WireXray,
}

/// Halo size: the object renderer sets record +0x38 = 0.5 and +0x2c = 0.6 (0x439134); the
/// base half-size is size * screen width * 0.05 pixels (0x4354fc, 0x4d2e6d).
/// CLASSIC Time Attack ghost car opacity (design choice).
const GHOST_ALPHA: f64 = 0.4;
const HALO_SIZE: f32 = 0.5;
/// The object renderer's halo gain for scene objects (blinkers, cones) and the base of the
/// Remaster brightness below (0x439134: +0x2c = 0.6).
const HALO_GAIN: f32 = 0.6;
/// REMASTER: extra halo brightness on top of HALO_GAIN for the world and scene objects (lamps,
/// blinkers), matched by eye against the original's Goldbridge footage [design choice]. Classic
/// uses the original's kinds below instead.
const WORLD_HALO_BRIGHTNESS: f32 = 2.0;
/// REMASTER: the same for car lights (head/tail lights) [design choice].
const CAR_HALO_BRIGHTNESS: f32 = 0.2;
/// CLASSIC halo colour byte (100, 100, 100) the object renderer writes into world and headlight
/// halos before it draws them (0x43908c, 0x4396b1): the glow's colour comes from its texture only.
const CLASSIC_GREY: f32 = 100.0 / 255.0;
/// CLASSIC world halos (0x439077): gain 1.0, size 1.0, flags |= 0x24 (the u + 0.75 curve and the
/// lens flare), flare intensity 0.65.
const WORLD_HALO_SIZE: f32 = 1.0;
const WORLD_FLARE: f32 = 0.65;
/// CLASSIC head lights (car halos of texture 0, 0x4396aa): gain 0.4, size 1.0, flags 0xe (drawn,
/// directional, lens flare, default curve), flare intensity 0.5, pointing along
/// (+-sin(pi/16), 0, cos(pi/16)) in game coordinates, outwards by the side of the car (0x4393dd).
const HEAD_HALO_GAIN: f32 = 0.4;
const HEAD_HALO_SIZE: f32 = 1.0;
const HEAD_FLARE: f32 = 0.5;
const HEAD_HALO_TOE: f32 = std::f32::consts::PI / 16.0;
/// CLASSIC tail lights (texture 1, 0x439733): colour (201, 0, 0), gain 0.5 and size 0.3, x 1.5
/// and x 1.75 while braking (0x4d2fdd, 0x4d2fe5), flags 0x1a (drawn, directional, the
/// (1-u)^4 curve), pointing backwards.
const TAIL_HALO_RGB: f32 = 201.0 / 255.0;
const TAIL_HALO_GAIN: f32 = 0.5;
const TAIL_HALO_SIZE: f32 = 0.3;
const TAIL_BRAKE_GAIN: f32 = 1.5;
const TAIL_BRAKE_SIZE: f32 = 1.75;
/// CLASSIC any other car halo (0x43989c): the file colour, gain 1.0, size 0.5.
const OTHER_HALO_SIZE: f32 = 0.5;
/// REMASTER: halo and lens-flare colour scale in the main view, the one brightness control of
/// Remaster halos (the HDR core gains in shader.wgsl apply on top of it) [design choice]
pub const REMASTER_HALO_SCALE: f32 = 0.5;
/// REMASTER: halo quad size with bloom (`View::halo_bloom`), in place of `HALO_SIZE`: the core
/// is small and the bloom makes the wide glow [design choice]
const REMASTER_HALO_SIZE: f32 = 0.8 * HALO_SIZE;
/// REMASTER: half-size of a smooth halo's starburst quad, relative to the halo quad's short
/// half-axis (main view only: a lens effect, not in reflections or the mirror) [design choice]
const HALO_BURST_SIZE: f32 = 2.5;
/// REMASTER: the starburst turns by `HALO_SPIN` rad per unit of the halo's horizontal NDC, the
/// same position that places its flare chain, so spikes and ghosts swing together as a lamp
/// passes (about 20 degrees at the screen edge) [design choice, stateless]
const HALO_SPIN: f32 = 0.65;
/// REMASTER: the burst also turns as the lamp nears, by `HALO_SPIN` × `HALO_SPIN_NEAR` at the
/// eye, falling to 0 at `HALO_SPIN_RANGE` metres, so it turns all along an approach even for
/// lamps near the centre line [design choice]
const HALO_SPIN_NEAR: f32 = 1.5;
const HALO_SPIN_RANGE: f32 = 80.0;
/// REMASTER: how much of the pixel-sized base (the halo's minimum glow) a lamp strip's width
/// takes, so its core stays a thin line [design choice]
const STRIP_BASE: f32 = 0.5;
/// REMASTER: the most (m) a lamp strip's glow reaches past its dots' edge, plus a pixel allowance
/// that grows with distance, so it stays on the lens
const STRIP_REACH: f32 = 0.02;
/// REMASTER: how far (in strip half-widths) two strips of a bend overlap and cross-fade
const STRIP_JOIN: f32 = 2.0;

/// The halo distance curve with u = 1 - z / far: (1-t)^3 x 2.5 + 0.2 by default, (1-t)^4 x 1.4 + 0.2
/// with flag 0x10, (1-t) + 0.75 with flag 0x20 (0x43681b, 0x43684f, 0x4357a8) [O].
fn halo_curve(flags: u32, u: f32) -> f32 {
    if flags & 0x20 != 0 {
        u + 0.75
    } else if flags & 0x10 != 0 {
        u * u * u * u * 1.4 + 0.2
    } else {
        u * u * u * 2.5 + 0.2
    }
}

/// The directional factor of a halo (0x436763): -dot(unit vector eye -> halo, direction rotated by
/// the object matrix), 1 for omnidirectional halos (`HaloInst::dir` ZERO). Zero or below = it
/// shines away from the eye and is not drawn.
fn halo_facing(h: &HaloInst, model: Mat4, mid: Vec3, eye: Vec3) -> f32 {
    if h.dir == Vec3::ZERO {
        return 1.0;
    }
    -(mid - eye).normalize_or_zero().dot(model.transform_vector3(h.dir).normalize_or_zero())
}

/// One halo as the Classic object renderer draws it: the record with its colour, flags and
/// direction set, the gain (+0x2c) and the size (+0x38) [O: 0x439134 car halos, 0x439077 world
/// halos]. The lens flare intensity (+0x3c) is `WORLD_FLARE` / `HEAD_FLARE`.
struct ClassicHalo {
    halo: HaloInst,
    gain: f32,
    size: f32,
}

/// CLASSIC world halo (0x439077): grey, flags |= 0x24.
fn classic_world_halo(h: &HaloInst) -> ClassicHalo {
    ClassicHalo { halo: HaloInst { rgb: [CLASSIC_GREY; 3], flags: h.flags | 0x24, ..*h }, gain: 1.0, size: WORLD_HALO_SIZE }
}

/// CLASSIC car halo by texture index (0x4393dd..0x43989c): 0 head light, 1 tail light (`brake`: the
/// brake lights are on), else the file's own. Directions are in view space (z negated).
fn classic_car_halo(h: &HaloInst, brake: bool) -> ClassicHalo {
    match h.texture {
        0 => {
            let side = if h.p0.x >= 0.0 { 1.0 } else { -1.0 };
            let dir = Vec3::new(side * HEAD_HALO_TOE.sin(), 0.0, -HEAD_HALO_TOE.cos());
            ClassicHalo { halo: HaloInst { rgb: [CLASSIC_GREY; 3], flags: 0xe, dir, ..*h }, gain: HEAD_HALO_GAIN, size: HEAD_HALO_SIZE }
        }
        1 => {
            let (gain, size) = if brake { (TAIL_HALO_GAIN * TAIL_BRAKE_GAIN, TAIL_HALO_SIZE * TAIL_BRAKE_SIZE) } else { (TAIL_HALO_GAIN, TAIL_HALO_SIZE) };
            ClassicHalo { halo: HaloInst { rgb: [TAIL_HALO_RGB, 0.0, 0.0], flags: 0x1a, dir: Vec3::Z, ..*h }, gain, size }
        }
        _ => ClassicHalo { halo: *h, gain: 1.0, size: OTHER_HALO_SIZE },
    }
}

/// Halo quads (0x4354fc), world/view space: centred on the segment midpoint, stretched
/// along the segment as seen from the camera (half-length 1.5 * L + base, half-width
/// 0.15 * L + base), faded with distance. APPROX: built as camera-facing world quads; the
/// original works in screen space and rotates the texture by 45 degrees. `glow` multiplies
/// the colour per channel (times a scene object's scale, 0x4354fc), `gain` is the record's gain
/// (+0x2c: Classic per kind, Remaster `HALO_GAIN`).
/// `size` is the record's size (CLASSIC: `HALO_SIZE`); the whole quad scales with it. The
/// vertex normal carries the texture's tint for fs_halo_remaster (fs_halo ignores it).
/// REMASTER: `burst` > 0 adds a square screen-aligned quad of half-size `burst` × the short
/// half-axis per halo, colour alpha 2: the lens starburst and ring of fs_halo_remaster, its
/// strength `burst_gain` × the halo's.
/// REMASTER `strips` = Some([gain, width scale]) draws a generated car's `HaloStrip::Strip`
/// halos as bars along their segment, flat-ended so that neighbours add up evenly (half-width
/// `STRIP_BASE` × the pixel base + the dot radius, × width scale; colour alpha 4,
/// `fs_halo_remaster`), × gain, and
/// the `Anchor` they came from as its burst only; None draws strips nowhere and anchors whole.
#[allow(clippy::too_many_arguments)]
fn halo_quads(out: &mut Vec<Vertex>, halos: &[HaloInst], model: Mat4, glow: Vec3, gain: f32, cam: &Camera, fov_x: f32, far: f32, fade_start: f32, fade_end: f32, size: f32, burst: f32, burst_gain: f32, spin: f32, strips: Option<[f32; 2]>) {
    let f = cam.forward();
    let (right, up) = (cam.right(), cam.right().cross(f));
    let tan_x = (fov_x / 2.0).tan();
    // world size of one pixel at depth z is 2 z tan(fov_x/2) / W; base = size * W * 0.05 px
    // CHEAT gulliver: a shrunk car's model matrix scales its halos' pixel base too (1.0 for every plain matrix)
    let model_scale = model.x_axis.truncate().length();
    let model_scale = if (model_scale - 1.0).abs() < 1e-3 { 1.0 } else { model_scale };
    let base_k = size * 0.05 * 2.0 * (fov_x / 2.0).tan() * model_scale;
    let k = size / HALO_SIZE;
    for h in halos {
        let (p0, p1) = (model.transform_point3(h.p0), model.transform_point3(h.p1));
        let mid = (p0 + p1) * 0.5;
        let z = (mid - cam.pos).dot(f);
        if z <= 0.3 || z >= far {
            continue;
        }
        let seg = p1 - p0;
        let seg_s = seg - f * seg.dot(f);
        let len = seg_s.length() * z / (mid - cam.pos).length().max(1e-3);
        let a = if len > 1e-4 { seg_s.normalize() } else { cam.right() };
        let b = f.cross(a).normalize_or(Vec3::Y);
        let base = base_k * z;
        let (mut ha, mut hb) = (1.5 * k * len + base, 0.15 * k * len + base);
        let (mut strip_gain, mut bar) = (1.0, None);
        match (h.strip, strips) {
            (HaloStrip::Strip { .. }, None) => continue,
            (HaloStrip::Strip { width: w, join: j }, Some([g, ws])) => {
                // the glow's pixel-sized base grows with distance and would spill over the
                // lens: capped at `STRIP_REACH` + 1.5 px (a 1280 px screen) beyond the dots
                let reach = STRIP_REACH + 1.5 * 2.0 * tan_x * z / 1280.0;
                hb = ws * (w + (STRIP_BASE * base).min(reach));
                ha = (0.5 * len).max(hb);
                (strip_gain, bar) = (g, Some(j));
            }
            _ => {}
        }
        // distance fade from the CueStart-derived start to the view distance, then the
        // default curve (1-u)^3 * 2.5 + 0.2 (flags 0x10 / 0x20 pick other curves)
        let fade = 1.0 - ((z - fade_start) / (fade_end - fade_start)).clamp(0.0, 1.0);
        let u = 1.0 - z / far;
        let curve = halo_curve(h.flags, u);
        // CLASSIC directional halos (0x436763): fade x -dot(eye->halo, direction), none behind
        let facing = halo_facing(h, model, mid, cam.pos);
        if facing <= 0.0 {
            continue;
        }
        let m = Vec3::from(h.rgb) * glow * (fade * curve * gain * strip_gain * facing);
        if m.max_element() <= 0.0 {
            continue;
        }
        // pull towards the camera so the glow isn't buried in the lamp geometry
        let c = mid - (mid - cam.pos).normalize() * (0.3 + h.pull);
        let color = [m.x, m.y, m.z, bar.map_or(1.0, |_| 4.0)];
        let v = |sa: f32, sb: f32, uv: [f32; 2]| Vertex { pos: (c + a * ha * sa + b * hb * sb).into(), uv, color, layer: h.layer, normal: h.tint, env: 0.0 };
        if let Some(join) = bar {
            // a strip meeting the next at a bend reaches `STRIP_JOIN` widths past it and fades
            // out over that stretch while the next fades in: the sum stays even (colour is
            // interpolated along the bar)
            let e = join.map(|j| if j { (STRIP_JOIN * hb).min(0.5 * len) } else { 0.0 });
            let xs = [-ha - e[0], -ha + e[0], ha - e[1], ha + e[1]];
            let wt = [if join[0] { 0.0 } else { 1.0 }, 1.0, 1.0, if join[1] { 0.0 } else { 1.0 }];
            let vtx = |k: usize, sb: f32| Vertex { pos: (c + a * xs[k] + b * hb * sb).into(), uv: [0.0, (sb + 1.0) / 2.0], color: [m.x * wt[k], m.y * wt[k], m.z * wt[k], 4.0], layer: h.layer, normal: h.tint, env: 0.0 };
            for k in 0..3 {
                let (q0, q1, q2, q3) = (vtx(k, -1.0), vtx(k + 1, -1.0), vtx(k + 1, 1.0), vtx(k, 1.0));
                out.extend([q0, q1, q2, q0, q2, q3]);
            }
        } else if h.strip != HaloStrip::Anchor || strips.is_none() {
            let (q0, q1, q2, q3) = (v(-1.0, -1.0, [0.0, 0.0]), v(1.0, -1.0, [1.0, 0.0]), v(1.0, 1.0, [1.0, 1.0]), v(-1.0, 1.0, [0.0, 1.0]));
            out.extend([q0, q1, q2, q0, q2, q3]);
        }
        if burst > 0.0 && bar.is_none() {
            let rb = burst * hb;
            let color = [m.x * burst_gain, m.y * burst_gain, m.z * burst_gain, 2.0];
            // REMASTER: turn the burst with the lamp's horizontal screen position
            let ndc_x = (mid - cam.pos).dot(right) / (z * tan_x);
            let near = (1.0 - z / HALO_SPIN_RANGE).clamp(0.0, 1.0);
            let (sn, cs) = (spin * (ndc_x.clamp(-1.5, 1.5) + HALO_SPIN_NEAR * near)).sin_cos();
            let (bx, by) = (right * cs + up * sn, up * cs - right * sn);
            let v = |sa: f32, sb: f32, uv: [f32; 2]| Vertex { pos: (c + bx * rb * sa + by * rb * sb).into(), uv, color, layer: h.layer, normal: h.tint, env: 0.0 };
            let (q0, q1, q2, q3) = (v(-1.0, -1.0, [0.0, 0.0]), v(1.0, -1.0, [1.0, 0.0]), v(1.0, 1.0, [1.0, 1.0]), v(-1.0, 1.0, [0.0, 1.0]));
            out.extend([q0, q1, q2, q0, q2, q3]);
        }
    }
}

/// Lens flares (0x435bd2..0x4368cf, D3D halo drawer). Every halo with flag 0x4 flares: the
/// world-halo pass (0x439077) sets flags |= 0x24, grey and intensity 0.65 on every world halo, and
/// the object renderer sets flags 0xe and intensity 0.5 on the car head lights (0x4396aa) [O]. The
/// flare colour is the halo's m (distance fade x curve x directional factor) x intensity x its
/// colour byte, and it shows while the halo's screen point is within r = half the screen width of
/// the centre, scaled by sin(pi/2 * (r - d) / r) [O]. The 7 elements are hex.iff quads on the line
/// from the centre through the light, at centre + k * (light - centre), half-size s * W / 512
/// px, colour = that x the element's factors [O: 0x435d95..0x436750, in draw order].
const FLARES: [(f32, f32, [f32; 3]); 7] = [
    (2.0, 5.0, [0.6, 0.2, 0.6]),
    (1.45, 8.0, [0.6, 0.6, 0.35]),
    (0.75, 9.0, [1.0, 0.75, 0.75]),
    (0.25, 14.0, [0.45, 0.45, 0.85]),
    (-0.5, 8.0, [0.85, 0.25, 0.25]),
    (-0.8, 8.0, [0.5, 0.5, 0.5]),
    (-2.0, 5.0, [0.15, 0.45, 0.15]),
];
/// REMASTER: the grey flare colour of the first Remaster flares, (100, 100, 100) x 0.65, times a gain
/// so the dim hex texture (peak 64/255) shows [design choice]. Classic flares use the halo's own
/// colour and intensity (`FLARES`), without this gain.
const FLARE_RGB: f32 = 100.0 / 255.0 * 0.65;
const FLARE_GAIN: f32 = 2.0;
/// Flares are quads this far in front of the eye (near plane 0.2), so nothing hides them.
const FLARE_DEPTH: f32 = 0.25;

/// CLASSIC flare quads for halos `halos` drawn with `model`, `intensity` = the halo's +0x3c (see
/// `FLARES`), and per light its `FlareLight`: `flare_vis_pass` then scales the quads by how much of
/// the light the drawn scene shows [design choice: the original only tests the halo's cell].
#[allow(clippy::too_many_arguments)]
fn flare_quads(out: &mut Vec<Vertex>, lights: &mut Vec<FlareLight>, halos: &[HaloInst], model: Mat4, intensity: f32, layer: f32, cam: &Camera, view_proj: Mat4, vp: [f32; 4], target: (f32, f32), fov: (f32, f32), far: f32, fade_start: f32) {
    let (f, right) = (cam.forward(), cam.right());
    let up = right.cross(f);
    let (tx, ty) = ((fov.0 / 2.0).tan(), (fov.1 / 2.0).tan());
    let w = vp[2];
    // one pixel at FLARE_DEPTH, world units
    let px = 2.0 * FLARE_DEPTH * tx / w;
    for hl in halos {
        let p = model.transform_point3((hl.p0 + hl.p1) * 0.5);
        let Some((ndc, fs)) = flare_screen_with(1.0, p, cam, view_proj, (vp[2], vp[3]), far, fade_start) else { continue };
        // m of the halo (curve and directional factor, 0x435bd2) x the intensity
        let facing = halo_facing(hl, model, p, cam.pos);
        let m = fs * intensity * halo_curve(hl.flags, 1.0 - (p - cam.pos).dot(f) / far) * facing;
        if m <= 0.0 {
            continue;
        }
        let first = out.len();
        for &(k, s, rgb) in &FLARES {
            let (ex, ey) = (ndc.x * k, ndc.y * k);
            let centre = cam.pos + (f + right * ex * tx + up * ey * ty) * FLARE_DEPTH;
            let half = s * w / 512.0 * px;
            let t = hl.tint;
            // alpha 0: a flare, not a light (fs_halo); the colour byte is at most 255
            let color = [0, 1, 2].map(|c| (rgb[c] * t[c] * hl.rgb[c] * m).min(1.0));
            let color = [color[0], color[1], color[2], 0.0];
            let v = |a: f32, b: f32, uv: [f32; 2]| Vertex { pos: (centre + right * half * a + up * half * b).into(), uv, color, layer, normal: [0.0; 3], env: 0.0 };
            let (q0, q1, q2, q3) = (v(-1.0, -1.0, [0.0, 1.0]), v(1.0, -1.0, [1.0, 1.0]), v(1.0, 1.0, [1.0, 0.0]), v(-1.0, 1.0, [0.0, 0.0]));
            out.extend([q0, q1, q2, q0, q2, q3]);
        }
        lights.push(FlareLight::new(ndc, (p - cam.pos).dot(f), first..out.len(), vp, target, tx));
    }
}

/// A flaring light for `flare_vis_pass` (flarevis.wgsl `Light`): its point in the target (uv,
/// y down), view depth and tap half-size (uv x units), then its quads in the flare vertices.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct FlareLight {
    p: [f32; 4],
    v: [u32; 4],
}

impl FlareLight {
    /// `ndc` in the view `vp` (x, y, w, h pixels) of a `target`-sized image; `tx` = tan(fov_x / 2).
    fn new(ndc: glam::Vec2, depth: f32, verts: std::ops::Range<usize>, vp: [f32; 4], target: (f32, f32), tx: f32) -> FlareLight {
        let x = (vp[0] + (ndc.x * 0.5 + 0.5) * vp[2]) / target.0;
        let y = (vp[1] + (0.5 - ndc.y * 0.5) * vp[3]) / target.1;
        // FLARE_TAP_SPREAD metres across, in pixels at this depth, within FLARE_TAP_PX
        let r = (FLARE_TAP_SPREAD * vp[2] * 0.5 / (depth.max(0.01) * tx)).clamp(FLARE_TAP_PX.0, FLARE_TAP_PX.1) / target.0;
        FlareLight { p: [x, y, depth, r], v: [verts.start as u32, verts.len() as u32, 0, 0] }
    }
}

/// Flare visibility (`flare_vis_pass`) [design choice]: the half-size of the square of depth
/// taps round a light, metres at its depth within (min, max) pixels, and how far in front of
/// the light (m) the drawn scene may be and still count as the light's own lamp housing.
const FLARE_TAP_SPREAD: f32 = 0.3;
const FLARE_TAP_PX: (f32, f32) = (2.0, 12.0);
const FLARE_CLEARANCE: f32 = 1.0;

/// Where a light's flare chain starts: its NDC point and strength `m` (the sin(pi/2 (r - d)
/// / r) screen-centre falloff times the distance fade), or None when it is behind the eye,
/// beyond `far`, off screen or farther than half the screen width from the centre [O, see
/// `FLARES`]. Shared by the Classic and Remaster flares.
pub fn flare_screen(p: Vec3, cam: &Camera, view_proj: Mat4, size: (f32, f32), far: f32, fade_start: f32) -> Option<(glam::Vec2, f32)> {
    flare_screen_with(FLARE_RGB * FLARE_GAIN, p, cam, view_proj, size, far, fade_start)
}

/// `flare_screen` with the strength `k` in place of the grey flare colour (Classic passes 1: its
/// colour is the halo's).
fn flare_screen_with(k: f32, p: Vec3, cam: &Camera, view_proj: Mat4, size: (f32, f32), far: f32, fade_start: f32) -> Option<(glam::Vec2, f32)> {
    let z = (p - cam.pos).dot(cam.forward());
    if z <= 0.3 || z >= far {
        return None;
    }
    let (w, h) = size;
    let r = w / 2.0;
    let c = view_proj * p.extend(1.0);
    let ndc = c.truncate().truncate() / c.w;
    let (sx, sy) = (ndc.x * w / 2.0, ndc.y * h / 2.0);
    let d = (sx * sx + sy * sy).sqrt();
    if d >= r || ndc.y.abs() > 1.0 {
        return None;
    }
    let fade = 1.0 - ((z - fade_start) / (far - fade_start)).clamp(0.0, 1.0);
    Some((ndc, k * fade * (std::f32::consts::FRAC_PI_2 * (r - d) / r).sin()))
}

/// REMASTER: a light that casts a computed lens flare; `strength` scales the chain.
struct FlareSource {
    p: Vec3,
    tint: [f32; 3],
    strength: f32,
}

/// REMASTER computed flare elements `(k, half-size, aspect, kind, rgb)` [design choice]: `k` as in
/// `FLARES` (position on the line through the screen centre), half-size in `W / 512` pixels,
/// `aspect` = quad height / width. The kinds are drawn by `fs_halo_remaster`: 0 soft disc,
/// 1 hexagonal aperture ghost, 2 ring, 3 veil, 4 anamorphic streak.
const FLARES_PROC: [(f32, f32, f32, f32, [f32; 3]); 9] = [
    (1.0, 30.0, 0.05, 4.0, [0.35, 0.5, 1.0]),
    (1.0, 40.0, 1.0, 3.0, [0.25, 0.25, 0.25]),
    (0.55, 10.0, 1.0, 1.0, [0.5, 0.9, 0.5]),
    (0.3, 18.0, 1.0, 2.0, [0.4, 0.6, 1.0]),
    (-0.35, 7.0, 1.0, 0.0, [1.0, 0.8, 0.4]),
    (-0.7, 13.0, 1.0, 1.0, [0.9, 0.4, 0.8]),
    (-1.1, 22.0, 1.0, 2.0, [0.9, 0.6, 0.3]),
    (1.6, 8.0, 1.0, 0.0, [0.4, 0.9, 0.9]),
    (2.3, 14.0, 1.0, 1.0, [0.6, 0.4, 0.9]),
];
/// REMASTER computed flares: gain on the Classic strength, as the shapes peak at 1 where the
/// hex.iff ghosts peak at 64/255 [design choice]
pub const FLARE_PROC_GAIN: f32 = 0.3;
/// car-light flares fade in between these distances (m)
const CAR_FLARE_NEAR: (f32, f32) = (6.0, 20.0);
/// REMASTER flare range, metres from the eye [design choice]: a light's flare fades in from
/// 0 at `.0` to full at `.1`, and out from full at `.2` to 0 at `.3` (lamps, blinkers, car lights)
pub const FLARE_RANGE: (f32, f32, f32, f32) = (16.25, 32.5, 75.0, 150.0);
/// the most flaring lights per frame (nearest first)
pub const MAX_FLARES: usize = 24;

/// REMASTER flare quads of computed shapes, at full strength, and per light its `FlareLight`
/// (see `flare_quads`). Alpha `-(kind + 1)` marks the quad for `fs_halo_remaster`; the tint
/// rides in the normal like the halos'.
#[allow(clippy::too_many_arguments)]
fn flare_quads_proc(out: &mut Vec<Vertex>, lights: &mut Vec<FlareLight>, srcs: &[FlareSource], cam: &Camera, view_proj: Mat4, vp: [f32; 4], target: (f32, f32), fov: (f32, f32), far: f32, fade_start: f32) {
    let (f, right) = (cam.forward(), cam.right());
    let up = right.cross(f);
    let (tx, ty) = ((fov.0 / 2.0).tan(), (fov.1 / 2.0).tan());
    let w = vp[2];
    let px = 2.0 * FLARE_DEPTH * tx / w;
    let mut seen: Vec<(f32, &FlareSource, glam::Vec2, f32)> = srcs
        .iter()
        .filter_map(|s| flare_screen(s.p, cam, view_proj, (vp[2], vp[3]), far, fade_start).map(|(ndc, m)| ((s.p - cam.pos).dot(f), s, ndc, m * s.strength)))
        .filter(|a| a.3 > 0.001)
        .collect();
    seen.sort_by(|a, b| a.0.total_cmp(&b.0));
    seen.truncate(MAX_FLARES);
    for (z, s, ndc, m) in seen {
        let m = m * FLARE_PROC_GAIN;
        if m < 0.001 {
            continue;
        }
        let first = out.len();
        for &(k, sz, aspect, kind, rgb) in &FLARES_PROC {
            let (ex, ey) = (ndc.x * k, ndc.y * k);
            let centre = cam.pos + (f + right * ex * tx + up * ey * ty) * FLARE_DEPTH;
            let half = sz * w / 512.0 * px;
            let t = s.tint;
            let color = [rgb[0] * m, rgb[1] * m, rgb[2] * m, -(kind + 1.0)];
            let v = |a: f32, b: f32, uv: [f32; 2]| Vertex { pos: (centre + right * half * a + up * half * aspect * b).into(), uv, color, layer: 0.0, normal: t, env: 0.0 };
            let (q0, q1, q2, q3) = (v(-1.0, -1.0, [0.0, 1.0]), v(1.0, -1.0, [1.0, 1.0]), v(1.0, 1.0, [1.0, 0.0]), v(-1.0, 1.0, [0.0, 0.0]));
            out.extend([q0, q1, q2, q0, q2, q3]);
        }
        lights.push(FlareLight::new(ndc, z, first..out.len(), vp, target, tx));
    }
}

/// REMASTER dynamic lights: the scene-object halos (`PROP_LIGHT_RANGE`) as point lights, the
/// `MAX_LIGHTS` nearest `eye` that can reach the view, and the car lights as spots, the
/// `MAX_SPOTS` nearest. Fills `g.lights`,
/// `g.spots` and their counts.
/// REMASTER the spin angle generated calipers turn back by (shader.wgsl `unspin`): the car's
/// wheel spin for its wheel meshes, 0 for bodies.
fn wheel_spin(f: &Frame, mesh: usize) -> f32 {
    if mesh.is_multiple_of(CAR_MESHES) {
        return 0.0;
    }
    // meshes 1, 2 are the front wheels, 3, 4 the rear ones (a pair shares one angle, 0x40ca90)
    f.car_spin.get(mesh / CAR_MESHES).map_or(0.0, |s| s[(mesh % CAR_MESHES - 1) / 2])
}

/// CLASSIC (0x43957c..0x439665): the body level of each car mesh in `list` at the camera, with the screen
/// `width` in pixels; None for a wheel the level leaves out. Levels 0 and 1 carry the separate wheels,
/// the built-in wheels of OBJ2 (Low detail) leave none to draw (0x45aed2, 0x45f436). A wheel takes the
/// level of its body entry (the list can be reordered), or its own distance when the body is not in it.
/// `floor`: the lowest level any body takes (supercars / grem: 3, 0x4392b4; 0 = none).
fn car_levels(list: &[(usize, Mat4)], cam: &Camera, detail: CarDetail, width: f32, floor: usize) -> Vec<Option<usize>> {
    let level = |m: &Mat4| detail.body_level(cam.pos.distance(m.w_axis.truncate()), cam.zoom, width).max(floor);
    list.iter()
        .map(|(mesh, m)| {
            if mesh.is_multiple_of(CAR_MESHES) {
                return Some(level(m));
            }
            let body = list.iter().find(|e| e.0 == mesh / CAR_MESHES * CAR_MESHES).map_or_else(|| level(m), |e| level(&e.1));
            (body < 2).then_some(0)
        })
        .collect()
}

#[allow(clippy::too_many_arguments)]
fn dynamic_lights(g: &mut Globals, s: &GpuScene, f: &Frame, eye: Vec3, far: f32, generated: bool, t: &CarLightTune, head: &SpotAlloc) {
    // (distance, position, range, rgb)
    let mut points: Vec<(f32, Vec3, f32, Vec3)> = Vec::new();
    for (i, &(mesh, m)) in f.props.iter().enumerate() {
        let Some(halos) = s.prop_halos.get(mesh).filter(|h| !h.is_empty()) else { continue };
        let glow = f.prop_glow.get(i).copied().unwrap_or(Vec3::ONE);
        for h in halos {
            let rgb = Vec3::from(h.rgb) * Vec3::from(h.tint) * glow * PROP_LIGHT_GAIN;
            let pos = m.transform_point3((h.p0 + h.p1) * 0.5);
            points.push((pos.distance(eye), pos, PROP_LIGHT_RANGE, rgb));
        }
    }
    // (distance, position, range, rgb, kind, beam axis, up, shadow layer)
    type Spot = (f32, Vec3, f32, Vec3, f32, Vec3, Vec3, f32);
    let mut spots: Vec<Spot> = Vec::new();
    let night = s.look.darkness;
    if night > 0.0 {
        for &(mesh, m) in f.cars.iter().take(MAX_CARS).chain(f.light_cars) {
            let Some(halos) = s.halos(mesh, generated).filter(|h| !h.is_empty()) else { continue };
            let brake = t.brake(f, mesh);
            // REMASTER headlight shadows: the atlas layer of this car's map (-1: none)
            let layer = if mesh.is_multiple_of(CAR_MESHES) { head.layer_of(mesh / CAR_MESHES).map_or(-1.0, |n| (LAMP_LAYERS + n) as f32) } else { -1.0 };
            // view space: forward is -z
            let (fwd, up) = (m.transform_vector3(Vec3::NEG_Z).normalize_or_zero(), m.transform_vector3(Vec3::Y).normalize_or_zero());
            for h in halos.iter().filter(|h| !matches!(h.strip, HaloStrip::Strip { .. })) {
                let mid = (h.p0 + h.p1) * 0.5;
                let pos = m.transform_point3(mid);
                let rgb = Vec3::from(h.rgb) * Vec3::from(h.tint) * night;
                let light = if mid.z < 0.0 {
                    (HEADLIGHT_RANGE, rgb * t.head.spot_gain(brake), 0.0, fwd)
                } else {
                    (TAIL_LIGHT_RANGE, rgb * t.tail.spot_gain(brake), 1.0, -fwd)
                };
                let (range, rgb, kind, axis) = light;
                let layer = if kind < 0.5 { layer } else { -1.0 };
                // the beam's far half decides whether it reaches the view
                spots.push(((pos + axis * range * 0.5).distance(eye) - range * 0.5, pos, range, rgb, kind, axis, up, layer));
            }
        }
        // REMASTER scene-object headlamps: head spots like a car's, without a shadow map (APPROX)
        for &(mesh, m) in f.props {
            let Some(pl) = s.prop_lamps.get(mesh).filter(|p| !p.lamps.is_empty()) else { continue };
            let (fwd, up) = (m.transform_vector3(pl.forward).normalize_or_zero(), m.transform_vector3(Vec3::Y).normalize_or_zero());
            for h in &pl.lamps {
                let pos = m.transform_point3((h.p0 + h.p1) * 0.5);
                let rgb = Vec3::from(h.rgb) * Vec3::from(h.tint) * night * t.head.spot_gain(0.0);
                spots.push(((pos + fwd * HEADLIGHT_RANGE * 0.5).distance(eye) - HEADLIGHT_RANGE * 0.5, pos, HEADLIGHT_RANGE, rgb, 0.0, fwd, up, -1.0));
            }
        }
    }
    points.retain(|p| p.3.max_element() > 0.01 && p.0 < far + p.2);
    points.sort_by(|a, b| a.0.total_cmp(&b.0));
    for (k, (_, pos, range, rgb)) in points.iter().take(MAX_LIGHTS).enumerate() {
        g.lights[2 * k] = pos.extend(*range).into();
        g.lights[2 * k + 1] = rgb.extend(0.0).into();
    }
    g.env[2] = points.len().min(MAX_LIGHTS) as f32;
    spots.retain(|p| p.0 < far);
    spots.sort_by(|a, b| a.0.total_cmp(&b.0));
    for (k, (_, pos, range, rgb, kind, axis, up, layer)) in spots.iter().take(MAX_SPOTS).enumerate() {
        g.spots[4 * k] = pos.extend(*range).into();
        g.spots[4 * k + 1] = rgb.extend(*kind).into();
        g.spots[4 * k + 2] = axis.extend(0.0).into();
        g.spots[4 * k + 3] = up.extend(*layer).into();
    }
    g.spot_n[0] = spots.len().min(MAX_SPOTS) as f32;
}

/// REMASTER street lamps (`LampLight`) as cone lights: the `MAX_LAMPS` nearest `eye` that can
/// reach the view, scaled by the track's darkness. Fills `g.lamps` and `g.lamp_n`.
fn street_lamps(g: &mut Globals, s: &GpuScene, eye: Vec3, far: f32, t: &LampLightTune, alloc: &SpotAlloc) {
    let night = s.look.darkness;
    if night <= 0.0 {
        return;
    }
    let mut near: Vec<(f32, usize, &LampLight)> = s.lamps.iter().enumerate().map(|(i, l)| (l.pos.distance(eye), i, l)).filter(|(d, _, l)| *d < far + l.range * t.range).collect();
    near.sort_by(|a, b| a.0.total_cmp(&b.0));
    for (k, (_, i, l)) in near.iter().take(MAX_LAMPS).enumerate() {
        g.lamps[2 * k] = l.pos.extend(l.range * t.range).into();
        // w: the lamp's shadow layer (-1: none)
        let layer = alloc.layer_of(*i).map_or(-1.0, |n| n as f32);
        g.lamps[2 * k + 1] = (Vec3::from(l.rgb) * t.gain * night).extend(layer).into();
    }
    // w: how much a lamp shadow darkens the baked world (no night factor: the lamps are night-only already)
    g.lamp_n = [near.len().min(MAX_LAMPS) as f32, t.inner.to_radians().cos(), t.outer.to_radians().cos(), t.shadow];
}

/// One layer of the shadow atlas: the light it serves (a lamp index, or a car's grid slot for the
/// headlight layers) and its fade (0..1, 1 = full shadow).
#[derive(Clone, Copy)]
struct SpotLayer {
    key: usize,
    fade: f32,
    on: bool,
}

/// REMASTER light shadows: which light holds which atlas layer (one allocator for the lamps, one
/// for the headlights). Layers fade in and out over `SPOT_FADE` so a light that gains or loses
/// its map does not pop.
struct SpotAlloc {
    layers: Vec<Option<SpotLayer>>,
    /// a lamp has been served before: new layers fade in instead of starting full
    warm: bool,
    at: Option<std::time::Instant>,
    /// the pinned clock (`Gpu::pin_clock`) at the last step: headless renders fade by it, not by the wall clock
    pinned: Option<f32>,
}

impl Default for SpotAlloc {
    fn default() -> Self {
        Self::with_layers(LAMP_LAYERS)
    }
}

impl SpotAlloc {
    fn with_layers(n: usize) -> Self {
        SpotAlloc { layers: vec![None; n], warm: false, at: None, pinned: None }
    }

    fn layer_of(&self, key: usize) -> Option<usize> {
        self.layers.iter().position(|l| l.is_some_and(|l| l.key == key))
    }

    /// Advance by `dt` seconds towards the keys in `wanted` (best first).
    fn step(&mut self, wanted: &[usize], dt: f32) {
        for slot in self.layers.iter_mut() {
            if let Some(l) = slot {
                l.on = wanted.contains(&l.key);
                l.fade = (l.fade + if l.on { dt } else { -dt } / SPOT_FADE).clamp(0.0, 1.0);
                if !l.on && l.fade <= 0.0 {
                    *slot = None;
                }
            }
        }
        for &key in wanted {
            if self.layer_of(key).is_none() {
                if let Some(slot) = self.layers.iter_mut().find(|l| l.is_none()) {
                    *slot = Some(SpotLayer { key, fade: if self.warm { 0.0 } else { 1.0 }, on: true });
                }
            }
        }
        self.warm |= !wanted.is_empty();
    }
}

/// The lamps that deserve a shadow map this frame (at most `LAMP_LAYERS`, best first): night
/// lamps whose cone holds a car body, scored by brightness over distance to the camera, a lamp
/// that already has a layer in `held` scoring more (hysteresis).
fn spot_select(lamps: &[LampLight], held: &SpotAlloc, cars: &[Vec3], cam: Vec3, t: &LampLightTune) -> Vec<usize> {
    let outer = t.shadow_cone.to_radians().cos();
    let mut best: Vec<(f32, usize)> = Vec::new();
    for (i, l) in lamps.iter().enumerate() {
        let range = t.shadow_range(l);
        let hit = cars.iter().any(|&c| {
            let d = c - l.pos;
            d.length() < range && -d.y > outer * d.length() && c.distance(cam) < t.distance
        });
        if !hit {
            continue;
        }
        let luma = 0.3 * l.rgb[0] + 0.59 * l.rgb[1] + 0.11 * l.rgb[2];
        let hold = if held.layer_of(i).is_some() { 1.5 } else { 1.0 };
        best.push((luma * hold / l.pos.distance(cam).max(1.0), i));
    }
    best.sort_by(|a, b| b.0.total_cmp(&a.0));
    best.into_iter().take(LAMP_LAYERS).map(|(_, i)| i).collect()
}

/// REMASTER headlight shadows: where a car's lights shine from. `key` is its grid slot
/// (car mesh index / `CAR_MESHES`).
#[derive(Clone, Copy, Debug)]
struct HeadSrc {
    key: usize,
    pos: Vec3,
    fwd: Vec3,
    up: Vec3,
}

/// The headlights of car body `mesh` seen as one source: the mean of its front halos, the beam
/// axis and up (forward is -z in `m`'s space). `None` for a mesh without front lights.
fn headlight_source(s: &GpuScene, mesh: usize, m: &Mat4, generated: bool) -> Option<HeadSrc> {
    if !mesh.is_multiple_of(CAR_MESHES) {
        return None;
    }
    let front: Vec<Vec3> = s.halos(mesh, generated)?.iter().filter(|h| !matches!(h.strip, HaloStrip::Strip { .. })).map(|h| (h.p0 + h.p1) * 0.5).filter(|p| p.z < 0.0).collect();
    if front.is_empty() {
        return None;
    }
    let mid = front.iter().sum::<Vec3>() / front.len() as f32;
    let (fwd, up) = (m.transform_vector3(Vec3::NEG_Z).normalize_or_zero(), m.transform_vector3(Vec3::Y).normalize_or_zero());
    Some(HeadSrc { key: mesh / CAR_MESHES, pos: m.transform_point3(mid), fwd, up })
}

/// The cars whose headlights deserve a shadow map this frame (at most `HEAD_LAYERS`, best first,
/// as slots): near the camera and with another car body (`bodies`: slot, centre) in the map's
/// frustum within `head_reach`. A car that already holds a layer in `held` scores more.
fn head_select(sources: &[HeadSrc], bodies: &[(usize, Vec3)], held: &SpotAlloc, cam: Vec3, t: &LampLightTune) -> Vec<usize> {
    let (tx, ty) = (t.head_fov_x.to_radians().tan(), t.head_fov_y.to_radians().tan());
    let mut best: Vec<(f32, usize)> = Vec::new();
    for s in sources {
        let dist = s.pos.distance(cam);
        if dist >= t.distance {
            continue;
        }
        let right = s.fwd.cross(s.up);
        // a body is a point here: widen the frustum by its half size (~2 m)
        let hit = bodies.iter().any(|&(k, c)| {
            let d = c - s.pos;
            let z = d.dot(s.fwd);
            k != s.key && z > 0.5 && z < t.head_reach && (d.dot(right) / z).abs() < tx + 2.0 / z && (d.dot(s.up) / z).abs() < ty + 2.0 / z
        });
        if hit {
            let hold = if held.layer_of(s.key).is_some() { 1.5 } else { 1.0 };
            best.push((hold / dist.max(1.0), s.key));
        }
    }
    best.sort_by(|a, b| b.0.total_cmp(&a.0));
    best.into_iter().take(HEAD_LAYERS).map(|(_, k)| k).collect()
}

/// World -> clip space of a shadow map from `view`: a perspective with the given half angles
/// (as tan), depth 0..1 over near 0.3 m to `range`.
#[allow(dead_code)]
fn perspective_shadow(view: Mat4, tan_x: f32, tan_y: f32, range: f32) -> Mat4 {
    perspective_bounds(view, [-tan_x, tan_x, -tan_y, tan_y], range)
}

/// Like `perspective_shadow` for an off-centre window: `[left, right, bottom, top]` as tan.
fn perspective_bounds(view: Mat4, b: [f32; 4], range: f32) -> Mat4 {
    let [l, r, bo, t] = b;
    let (near, far) = (0.3, range.max(1.0));
    let proj = Mat4::from_cols(
        glam::Vec4::new(2.0 / (r - l), 0.0, 0.0, 0.0),
        glam::Vec4::new(0.0, 2.0 / (t - bo), 0.0, 0.0),
        glam::Vec4::new((r + l) / (r - l), (t + bo) / (t - bo), far / (near - far), -1.0),
        glam::Vec4::new(0.0, 0.0, near * far / (near - far), 0.0),
    );
    proj * view
}

/// A lamp's shadow view: straight down, and the widest window it may use (the shadow cone plus a
/// margin) as `[left, right, bottom, top]` tan.
fn lamp_shadow_view(pos: Vec3, outer_deg: f32) -> (Mat4, [f32; 4]) {
    let view = glam::camera::rh::view::look_to_mat4(pos, Vec3::NEG_Y, Vec3::Z);
    let t = (outer_deg + 4.0).min(84.0).to_radians().tan();
    (view, [-t, t, -t, t])
}

/// A car's headlight shadow view: along the beam, `head_fov_x` by `head_fov_y`, and its window.
fn head_shadow_view(s: &HeadSrc, t: &LampLightTune) -> (Mat4, [f32; 4]) {
    let view = glam::camera::rh::view::look_to_mat4(s.pos, s.fwd, s.up);
    let (tx, ty) = (t.head_fov_x.to_radians().tan(), t.head_fov_y.to_radians().tan());
    (view, [-tx, tx, -ty, ty])
}

/// REMASTER: the window of a light's shadow map fitted to the cars that cast into it. Only
/// directions with a car in them can hold a shadow (a shadow lies on its caster's ray from the
/// light), so the map covers just those, plus the PCF margin: 10x the texels per metre of the
/// full cone. Each car is a sphere of `CASTER_RADIUS`. The window's size steps in 2^(1/8) and its
/// centre snaps to texels, so a parked car's shadow does not shimmer. Falls back to `max` (the
/// whole cone) when a car is almost at the light or none is left in the cone.
fn fit_window(view: Mat4, max: [f32; 4], casters: &[Vec3], size: u32, pcf: u32) -> [f32; 4] {
    let mut w = [f32::MAX, f32::MIN, f32::MAX, f32::MIN];
    for &c in casters {
        let v = view.transform_point3(c);
        let d = -v.z;
        if d - CASTER_RADIUS < 0.5 {
            return max;
        }
        for (a, (lo, hi)) in [(v.x, (0, 1)), (v.y, (2, 3))] {
            let (near, far) = (d - CASTER_RADIUS, d + CASTER_RADIUS);
            w[lo] = w[lo].min(((a - CASTER_RADIUS) / near).min((a - CASTER_RADIUS) / far));
            w[hi] = w[hi].max(((a + CASTER_RADIUS) / near).max((a + CASTER_RADIUS) / far));
        }
    }
    let mut out = [0.0; 4];
    for (lo, hi) in [(0, 1), (2, 3)] {
        let half = (w[hi] - w[lo]) * 0.5 * (1.0 + 2.0 * (pcf + 2) as f32 / size as f32);
        let half = 2f32.powf((half.max(0.03).log2() * 8.0).ceil() / 8.0);
        let step = 2.0 * half / size as f32;
        let mid = ((w[lo] + w[hi]) * 0.5 / step).round() * step;
        out[lo] = (mid - half).max(max[lo]);
        out[hi] = (mid + half).min(max[hi]);
        if out[hi] - out[lo] < 0.01 {
            return max;
        }
    }
    out
}

/// A light's shadow matrix and tan per texel for the casters inside its range.
fn fit_shadow(view: Mat4, max: [f32; 4], casters: &[Vec3], range: f32, size: u32, pcf: u32) -> (Mat4, f32) {
    let b = if casters.is_empty() { max } else { fit_window(view, max, casters, size, pcf) };
    (perspective_bounds(view, b, range), ((b[1] - b[0]).max(b[3] - b[2])) / size as f32)
}

/// Camera-facing quads for particles: (colour-keyed, additive, REMASTER lit smoke back to
/// front, REMASTER spark streaks). Without `remaster` the lit smoke is drawn like the
/// original's (a particle can outlive a switch back to Classic) and sparks like its frames.
/// The particle quads as (additive, REMASTER smoke, REMASTER sparks). `tan_x` = tan(fov_x / 2).
fn sprite_quads(sprites: &[Sprite], cam: &Camera, remaster: bool, tan_x: f32) -> (Vec<Vertex>, Vec<Vertex>, Vec<Vertex>) {
    let right = cam.right();
    let up = right.cross(cam.forward());
    let (mut add, mut smoke, mut sparks) = (Vec::new(), Vec::new(), Vec::new());
    let mut order: Vec<&Sprite> = sprites.iter().collect();
    order.sort_by(|a, b| b.pos.distance_squared(cam.pos).total_cmp(&a.pos.distance_squared(cam.pos)));
    for s in order {
        let (u0, u1) = if s.mirror { (1.0, 0.0) } else { (0.0, 1.0) };
        // axes of the quad: the camera's, turned by `rot`; a streak lies along its velocity
        // projected on the screen
        let (ax, ay) = match s.streak.filter(|_| remaster) {
            Some(vel) => {
                let flat = vel - cam.forward() * vel.dot(cam.forward());
                let a = flat.try_normalize().unwrap_or(right);
                (a, cam.forward().cross(a))
            }
            None if s.rot != 0.0 && remaster => {
                let (sn, cs) = s.rot.sin_cos();
                (right * cs + up * sn, up * cs - right * sn)
            }
            None => (right, up),
        };
        let lit = s.lit && remaster;
        let normal = if lit { [s.ground, 0.0, 0.0] } else { [0.0; 3] };
        // CLASSIC particles (0x43b398): the half size is capped at half the screen's half width, a
        // spark's quad runs from the point to one side, and every vertex moves `pull` metres nearer
        // along its eye ray (screen position kept, so the puff doesn't sink into the road)
        let fwd = cam.forward();
        let mut half = s.half;
        if s.pull > 0.0 {
            let cap = 0.5 * (s.pos - cam.pos).dot(fwd).max(NEAR) * tan_x;
            half = half.map(|h| h.min(cap));
        }
        let pull = |p: Vec3| {
            let z = (p - cam.pos).dot(fwd);
            if s.pull > 0.0 && z > NEAR {
                cam.pos + (p - cam.pos) * ((z - s.pull).max(NEAR) / z)
            } else {
                p
            }
        };
        let centre = s.pos + ax * half[0] * s.side;
        let v = |a: f32, b: f32, uv: [f32; 2]| Vertex { pos: pull(centre + ax * half[0] * a + ay * half[1] * b).into(), uv, color: s.color, layer: s.layer, normal, env: 0.0 };
        let (q0, q1, q2, q3) = (v(-1.0, -1.0, [u0, 1.0]), v(1.0, -1.0, [u1, 1.0]), v(1.0, 1.0, [u1, 0.0]), v(-1.0, 1.0, [u0, 0.0]));
        let spark = s.streak.is_some() && remaster;
        let dst = if lit { &mut smoke } else if spark { &mut sparks } else { &mut add };
        dst.extend([q0, q1, q2, q0, q2, q3]);
    }
    (add, smoke, sparks)
}

/// REMASTER hologram clip (front end): keep the hologram inside or outside the main dial's
/// face (window pixels), scaled by `gain`; lets the face show a blurred copy (several jittered
/// draws) while the rest stays sharp.
#[derive(Clone, Copy, Debug)]
pub struct HoloClip {
    pub centre: [f32; 2],
    pub radius: f32,
    /// the face's flat bottom: window y
    pub chord_y: f32,
    pub inside: bool,
    pub gain: f32,
    /// a rounded rectangle round `centre` instead of the circle: half extents, corner radius (px)
    pub rect: Option<([f32; 2], f32)>,
    /// REMASTER glass cards (#294): more rounded rectangles (centre, half extents, window px),
    /// `rects_n` of them in use, all with the corner radius `rects_r`; the clip region is the union
    pub rects: [([f32; 2], [f32; 2]); HOLO_RECTS],
    pub rects_n: usize,
    pub rects_r: f32,
    /// REMASTER frosted glass (#296): > 0 = one unclipped pass, blurred with this gaussian sigma
    /// (window px) and composited sharp outside / blurred inside (`Gpu::draw_hologram_frosted`);
    /// 0 = the hologram clipped per fragment (`inside`)
    pub blur: f32,
}

impl HoloClip {
    /// `myw_n.z` for shader.wgsl `holo_gain`: 1 keep inside, 2 keep outside, 3 unclipped (frosted)
    fn mode(&self) -> f32 {
        if self.blur > 0.0 {
            3.0
        } else if self.inside {
            1.0
        } else {
            2.0
        }
    }
}

/// REMASTER live paint: the `Globals::paint` rows of a car body with the stock colours `own`, in
/// the space `scene::emit` wrote them (gamma, times ObjectBrightness), so the shader can find them
fn paint_rows(p: &LivePaint, own: [[u8; 3]; 3], scale: [f32; 3]) -> [[f32; 4]; 6] {
    let emitted = |c: [u8; 3], w: f32| {
        let v = crate::scene::rgba_obj([c[0], c[1], c[2], 255], scale);
        [v[0], v[1], v[2], w]
    };
    let mut rows = [[0.0; 4]; 6];
    for k in 0..3 {
        rows[k] = emitted(own[k], match p.finish[k] {
            crate::scene::Finish::Gloss => 1.0,
            crate::scene::Finish::Metallic => 2.0,
            crate::scene::Finish::Pearl => 3.0,
        });
        rows[k + 3] = emitted(p.to[k].unwrap_or(own[k]), p.pulse.filter(|z| z.0 == k).map_or(0.0, |z| z.1));
    }
    rows
}

/// REMASTER hologram clip: rounded rectangles a `HoloClip` can list (`Globals::holo_rects`)
pub const HOLO_RECTS: usize = 16;

/// REMASTER frosted scene (#300): the race drawn blurred inside the Glass pause menu's cards
/// (`Frame::frost`, `Gpu::draw_scene_frosted`).
#[derive(Clone, Copy, Debug)]
pub struct SceneFrost {
    /// the cards: centre and half extents, window px; `n` of them in use
    pub rects: [([f32; 2], [f32; 2]); SCENE_FROST_RECTS],
    pub n: usize,
    /// the cards' corner radius, window px
    pub radius: f32,
    /// the gaussian's sigma, window px
    pub blur: f32,
}

/// the most cards a `SceneFrost` lists (scenefrost.wgsl)
pub const SCENE_FROST_RECTS: usize = 12;

/// REMASTER menu car preview (#91, 09-remaster 9.39): the body's paint zones recoloured in the
/// vertex shader, as CustomCarColours repaints the faces at race load (`scene::repaint`), with
/// no scene reload.
#[derive(Clone, Copy, Debug, Default)]
pub struct LivePaint {
    /// the new colour per zone; None = the stock one
    pub to: [Option<[u8; 3]>; 3],
    /// a zone that shows brighter: its index and the gain (colour x (1 + gain))
    pub pulse: Option<(usize, f32)>,
    /// REMASTER (#136, #351): each zone's finish
    pub finish: [crate::scene::Finish; 3],
}

/// Per-frame dynamic geometry.
/// REMASTER grass: how a racer presses the blades (`Frame::car_motion`): its velocity in view
/// space (m/s) and the hull's half length and half width (m, `CarParams::corners`).
#[derive(Clone, Copy, Debug, Default)]
pub struct CarMotion {
    pub vel: Vec3,
    pub half: [f32; 2],
}

/// REMASTER grass trail (18-grass.md, Gitea #158): one footprint of a car on the grass,
/// recorded by `Session::step` (view space). `time` is the sim clock at the stamp, `tail` the
/// seconds since that car's previous stamp (the stamp pass fills the gap back along `vel`).
/// Per-instance vertex of trail.wgsl `vs_stamp` (locations 0..4).
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, bytemuck::Pod, bytemuck::Zeroable)]
pub struct TrailStamp {
    pub pos: [f32; 2],
    pub fwd: [f32; 2],
    pub half: [f32; 2],
    pub vel: [f32; 2],
    pub time: f32,
    pub tail: f32,
    pub _pad: [f32; 2],
}

/// REMASTER grass trail: seconds a flattened trail takes to spring back up (sim time)
pub const TRAIL_RELAX: f32 = 5.0;
/// REMASTER grass trail: texels per edge-map texel per axis (0.25 m at the map's 0.5 m), and the
/// most texels a side (16 MB of R8)
const TRAIL_TEXEL_DIV: u32 = 2;
const TRAIL_MAX: u32 = 4096;

#[derive(Clone)]
pub struct Frame<'a> {
    /// REMASTER hologram: where it shows (`HoloClip`)
    pub holo_clip: Option<HoloClip>,
    /// REMASTER: the scene blurred behind these cards (the Glass pause menu)
    pub frost: Option<SceneFrost>,
    /// CLASSIC, CHEAT supercars / grem (#319), ignition / uds (#318), `cheats::car_level`: every car at least this body
    /// level (OBJ3 / OBJ2) on a black background instead of the sky (0x4392b4, 0x43957c, 0x443a99); Remaster ignores it
    pub cheat_level: Option<usize>,
    /// CHEAT lemmy / ace "Hell mode" (#316, `0x439e84`): `Some(lightning)`. Red brightness is baked into the scene; the frame
    /// has the black (white in a flash) fog, the cloud-ceiling sky (`0x443570`) and the flash gain, in both render modes.
    pub hell: Option<bool>,
    /// CHEAT ramlosa / h2o "Underwater" (#317, `0x43a54b`): `Some(wobble phase)`. The teal brightness is baked into the
    /// scene; the frame has the teal fog and the same cloud-ceiling sky as Hell mode (blue to teal, slower scroll), and the
    /// main view is warped by the wobble pass (`Gpu::draw_underwater`, `0x4419b4`), in both render modes.
    pub underwater: Option<f32>,
    /// CHEAT tribute to tron (#320, `0x48e964`): the Glide Tron look. The scene carries the triangles' outline codes
    /// (`scene::SceneCheat::tron`); CLASSIC draws blue light and outlines, REMASTER the Neon grid (`neon_shade`,
    /// 09-remaster 9.42); both with black sky and fog and none of the lists the original clears unseen (shadows, skid
    /// marks, halos, flares, smoke and sparks).
    pub tron: bool,
    /// CHEAT tribute to tron, REMASTER Neon grid (#320): the cars' light walls, view space (`Session::light_walls`);
    /// drawn in Remaster only
    pub light_walls: &'a [Vertex],
    /// REMASTER grass trail: the footprints of the last `TRAIL_RELAX` seconds, oldest first
    /// (`Session::trail`); the renderer stamps the ones newer than its last frame
    pub trail: &'a [TrailStamp],
    /// the sim clock the trail decays by (seconds; a step back clears and replays the trail)
    pub sim_time: f32,
    /// (car mesh index, model matrix)
    pub cars: &'a [(usize, Mat4)],
    /// car shadow triangles, view space
    pub shadows: &'a [Vertex],
    /// skid mark triangles, view space (`Session::skid_marks`: the detailed ones when
    /// `View::skids_detailed`), drawn right after the shadows
    pub skids: &'a [Vertex],
    /// (scene object mesh index, model matrix)
    pub props: &'a [(usize, Mat4)],
    /// per `props` entry: halo colour multiplier (the object's animated scale; 1 if missing)
    pub prop_glow: &'a [Vec3],
    /// draw into this window-pixel rectangle (x, y, w, h) instead of the whole target
    pub viewport: Option<[f32; 4]>,
    /// draw over what is already in the target: no clear, no sky (menu car preview)
    pub overlay: bool,
    /// REMASTER menu car preview: the cars only, no world, fill, grass or water (with `overlay`)
    pub cars_only: bool,
    /// REMASTER menu car preview: the car bodies' paint, recoloured live
    pub paint: Option<LivePaint>,
    /// REMASTER menu car preview: lit by the studio's key light and neutral cube (`STUDIO_*`) instead of the
    /// scene's own light (with `overlay`)
    pub studio: bool,
    /// mirror image (rear-view mirror): x flipped, drawn over the target inside `viewport`
    pub mirror: bool,
    /// DEBUG triangle edges instead of filled triangles, in this style (needs `Gpu::load_wireframe`)
    pub wire: WireStyle,
    /// REMASTER front end (09-remaster 9.24): the cars as a glowing hologram (additive edges and
    /// a fresnel shell) over the target, nothing else; with `overlay`
    pub hologram: bool,
    /// REMASTER hologram: draw the loaded road lines too, fading out by this distance (m);
    /// 0 = none (uniform `spot_n.w`, shader `fs_holo`)
    pub holo_road: f32,
    /// smoke and sparks
    pub sprites: &'a [Sprite],
    /// per car in grid-slot order (car mesh index / `CAR_MESHES`): brake input 0..1 (REMASTER
    /// brake lights; 0 if missing)
    pub car_brake: &'a [f32],
    /// per car in grid-slot order: wheel spin angle (radians), REMASTER generated calipers turn
    /// back by it (`spot_n.z` of the wheels' uniforms; 0 if missing)
    pub car_spin: &'a [[f32; 2]],
    /// REMASTER grass (18-grass.md): per racer (indexed like `car_brake`) the motion the blades
    /// bend to; empty = still cars with a default footprint
    pub car_motion: &'a [CarMotion],
    /// REMASTER car lights: car bodies that are not drawn but still light the scene (the
    /// player's car in the in-car views and the mirror)
    pub light_cars: &'a [(usize, Mat4)],
    /// Time Attack ghost (design choice): the player's car meshes at a recorded pose, drawn
    /// translucent (CLASSIC) or as a hologram (REMASTER); no shadow, lights or reflections
    pub ghost_cars: &'a [(usize, Mat4)],
    /// REMASTER, Enhanced car paint: capture the env cube here (view space), leaving out the car
    /// meshes in the range (the followed car)
    pub env_probe: Option<(Vec3, std::ops::Range<usize>)>,
    /// DEBUG: line list (view space) drawn after the scene, depth tested (motorview gizmos)
    pub debug_lines: &'a [Vertex],
    /// DEBUG: line list (view space) drawn over everything
    pub debug_xray: &'a [Vertex],
    /// DEBUG: solid triangles without depth test (the world editor's gizmos)
    pub debug_xray_tris: &'a [Vertex],
}

impl<'a> Frame<'a> {
    pub fn new(cars: &'a [(usize, Mat4)], shadows: &'a [Vertex]) -> Frame<'a> {
        Frame { holo_clip: None, frost: None, cheat_level: None, hell: None, underwater: None, tron: false, light_walls: &[], trail: &[], sim_time: 0.0, cars, shadows, skids: &[], props: &[], prop_glow: &[], car_brake: &[], car_spin: &[], car_motion: &[], light_cars: &[], ghost_cars: &[], viewport: None, overlay: false, cars_only: false, paint: None, studio: false, mirror: false, wire: WireStyle::Off, hologram: false, holo_road: 0.0, sprites: &[], env_probe: None, debug_lines: &[], debug_xray: &[], debug_xray_tris: &[] }
    }
}

pub struct Gpu {
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
    pub color_format: wgpu::TextureFormat,
    src: PipeSrc,
    /// CLASSIC: the original's pipeline
    classic: Pipes,
    /// REMASTER: built on first use and kept, per target format (the swapchain's, `HDR`) and
    /// `RemasterView::MSAA_LEVELS` entry (single-sampled: overlays, mirror, env probe)
    remaster: [[OnceCell<Pipes>; 5]; 2],
    /// REMASTER: the MSAA sample counts this GPU can draw (colour and depth)
    msaa_counts: Vec<u32>,
    /// REMASTER: the device's largest 2D texture side (the SSAA render size is clamped to it)
    max_dim: u32,
    /// REMASTER: bind group 1, the texture sampler per `RemasterView::ANISOTROPY_LEVELS` entry
    aniso: Vec<wgpu::BindGroup>,
    /// REMASTER car shadows: the sun's depth map, built on first use
    shadow: OnceCell<ShadowMap>,
    /// REMASTER volumetric fog sun shafts: the world's depth map seen from the sun, built on first use
    shaft: OnceCell<ShaftMap>,
    shadow_layouts: (wgpu::BindGroupLayout, wgpu::BindGroupLayout),
    /// REMASTER lamp shadows: the atlas (built when a lamp first gets a map), which lamp holds
    /// which layer, and whether this frame's atlas holds valid maps (the mirror reuses them)
    spot: [OnceCell<SpotShadows>; 3],
    /// index in `SPOT_SIZES` of this frame's atlas
    spot_size: Cell<usize>,
    spot_alloc: RefCell<SpotAlloc>,
    /// the same for the headlight layers (`HEAD_LAYERS`, atlas layer `LAMP_LAYERS + n`)
    head_alloc: RefCell<SpotAlloc>,
    spot_ready: Cell<bool>,
    /// the shadow map holds this frame's cars (the rear mirror reuses it)
    shadow_ready: Cell<bool>,
    /// REMASTER: bit k set = car k (mesh / CAR_MESHES) is drawn with its original model in the view being
    /// drawn (`far_cars`); `far_memory` is the main view's choice of the last frame (the hysteresis band)
    far_mask: Cell<u64>,
    far_memory: Cell<u64>,
    /// REMASTER: multisample, HDR, bloom and grade targets of the main view
    targets: RefCell<Option<Targets>>,
    post: OnceCell<Post>,
    bloom: OnceCell<Bloom>,
    aa: OnceCell<Aa>,
    smaa: OnceCell<Smaa>,
    taa: OnceCell<Taa>,
    ssao: OnceCell<Ssao>,
    /// REMASTER volumetric fog (`volfog.rs`), built on first use
    volfog: OnceCell<crate::volfog::VolFog>,
    /// REMASTER water (water.wgsl): shader modules and detail maps, built on first use
    water: OnceCell<WaterGpu>,
    /// REMASTER grass (grass.wgsl): its shader module, built on first use
    grass: OnceCell<wgpu::ShaderModule>,
    /// REMASTER grass trail (trail.wgsl): its pipelines, built on first use
    trail: OnceCell<TrailGpu>,
    /// REMASTER Ultra grass blades: their pipelines and buffers, built on first use
    blades: OnceCell<BladeGpu>,
    frame_no: Cell<u32>,
    /// REMASTER TAA: main frames drawn with it (the jitter's index)
    taa_frame: Cell<u32>,
    /// CLASSIC sky band, per window (`view_now`): the U offset and the heading it was last advanced
    /// at (0x6fc308 / 0x6fc30c); the offset is accumulated frame by frame like the original's
    sky_scroll: RefCell<Vec<Option<(f32, f32)>>>,
    probe: EnvProbe,
    blur_layout: wgpu::BindGroupLayout,
    history: RefCell<Option<History>>,
    /// REMASTER velocity blur: pipelines (built on first use) and the last main view's
    /// matrices
    vel_blur: OnceCell<VelBlur>,
    dissolve: OnceCell<wgpu::RenderPipeline>,
    prev_view: RefCell<Option<PrevView>>,
    /// DEBUG second view (motorview's ghost window): the fields above hold the current view's
    /// state; the other views' wait here (`select_view`)
    views: RefCell<Vec<ViewState>>,
    view_now: Cell<usize>,
    /// lens-flare visibility from the depth (`flare_vis_pass`), per depth kind (MSAA or not)
    flare_vis: [OnceCell<(wgpu::BindGroupLayout, wgpu::ComputePipeline)>; 2],
    /// REMASTER water animation clock, and a pinned time in its place (headless shots, so the
    /// water and the grass wind are the same run to run)
    water_clock: std::time::Instant,
    clock_pin: Cell<Option<f32>>,
    layout: wgpu::BindGroupLayout,
    uniforms: wgpu::Buffer,
    car_uniforms: Vec<wgpu::Buffer>,
    /// lit objects already in world space (scene objects)
    prop_uniform: wgpu::Buffer,
    /// CLASSIC per-object light records of the scene objects and the record of each vertex of the
    /// props buffer (shader.wgsl `prop_myw`, `prop_of`; `PROP_REC`)
    prop_lights: wgpu::Buffer,
    prop_of: wgpu::Buffer,
    sampler: wgpu::Sampler,
    scene: Option<GpuScene>,
    /// REMASTER local light (#83): the loaded scene's bake input (kept: a new sun rebakes, #192)
    local_src: Option<std::sync::Arc<crate::probes::BakeSource>>,
    /// REMASTER local light: the bake running on a worker thread (dropped by `load`: cancelled),
    /// and whether its volume fades in (the first one) or replaces the shown one at once
    local_bake: Option<(crate::probes::Baking, bool)>,
    /// REMASTER local light: the sun of the volume shown or baking (None: none started)
    local_sun: Option<[f32; 3]>,
    /// REMASTER local light: the sun the view last wanted, and since when
    local_want: ([f32; 3], std::time::Instant),
    /// REMASTER local light: bake on the calling thread and show the volume at once (headless shots)
    local_wait: bool,
    ui_pipe: wgpu::RenderPipeline,
    ui_layout: wgpu::BindGroupLayout,
    ui_uniform: wgpu::Buffer,
    /// REMASTER: the rev meter's look constants (`revmeter::RevLook`), binding 4 of the sharp UI
    rev_look: wgpu::Buffer,
    ui_bind: Option<wgpu::BindGroup>,
    /// the UI atlas and its sampler (also bound by the sharp HUD)
    ui_atlas: Option<(wgpu::TextureView, wgpu::Sampler)>,
    /// the UI atlas texture (`update_ui` rewrites parts of it)
    ui_tex: Option<wgpu::Texture>,
    /// REMASTER sharp HUD: pipeline + bind group with the field atlas, built on first use
    ui_sharp: OnceCell<UiSharp>,
    glow: OnceCell<Glow>,
    /// REMASTER frosted hologram (`draw_hologram_frosted`); `frosting` while its sharp pass draws
    holo_frost: OnceCell<HoloFrost>,
    /// REMASTER frosted scene (`draw_scene_frosted`)
    scene_frost: OnceCell<SceneFrostGpu>,
    frosting: Cell<bool>,
    /// CHEAT ramlosa / h2o (#317): the wobble pass (`draw_underwater`); `warping` while its sharp copy draws
    underwater: OnceCell<UnderwaterGpu>,
    warping: Cell<bool>,
    /// REMASTER menu car preview anti-aliasing (`draw_studio_aa`): its offscreen MSAA frame, the pipelines that
    /// also write alpha, and the composite; `studio_aa` while the offscreen frame draws
    studio: OnceCell<StudioAa>,
    studio_pipes: OnceCell<Pipes>,
    studio_aa: Cell<bool>,
}

/// What the scene pipelines are built from (kept for the lazily built Remaster sets).
struct PipeSrc {
    shader: wgpu::ShaderModule,
    layout: wgpu::PipelineLayout,
    /// REMASTER: `layout` plus the anisotropic sampler group
    remaster_layout: wgpu::PipelineLayout,
    /// REMASTER car shadows: the caster pass layout (group 0, the sampler group, the map's
    /// uniform as group 2)
    shadow_layout: wgpu::PipelineLayout,
    blur_shader: wgpu::ShaderModule,
    blur_layout: wgpu::PipelineLayout,
    /// REMASTER water: group 3 (scene copy, depth, detail maps; [single, multisampled depth])
    /// and the pipeline layout with it after the Remaster groups
    water_groups: [wgpu::BindGroupLayout; 2],
    water_layouts: [wgpu::PipelineLayout; 2],
    /// REMASTER grass: group 3 (the edge map and its sampler) and the pipeline layout with it
    grass_group: wgpu::BindGroupLayout,
    grass_layout: wgpu::PipelineLayout,
    /// REMASTER Ultra grass blades (`BladeGpu`): the placement pass's group 1 and layout
    /// (globals, this, the grass group as 3), the draw's group 3 (the records) and its layout
    blade_group: wgpu::BindGroupLayout,
    blade_read_group: wgpu::BindGroupLayout,
    blade_compute_layout: wgpu::PipelineLayout,
    blade_draw_layout: wgpu::PipelineLayout,
}

/// REMASTER water (water.wgsl, re-notes/16-water.md): the shader per depth kind (shader.wgsl plus
/// water.wgsl), tuxalin's detail maps, the scene sampler and the group 3 of the views that have
/// no scene copy (mirror, viewports; `Globals::water_t` y = 0 so it isn't read).
struct WaterGpu {
    modules: [wgpu::ShaderModule; 2],
    normal: wgpu::TextureView,
    foam: wgpu::TextureView,
    sampler: wgpu::Sampler,
    /// a 1x1x1 stand-in for the fog volume where there is none (never sampled: only a volumetric view reads it)
    no_fog: wgpu::TextureView,
    dummy: wgpu::BindGroup,
}

/// REMASTER water: the copy of the opaque scene the water pass reads, and its group 3
struct WaterTargets {
    src: wgpu::Texture,
    bind: wgpu::BindGroup,
}

/// REMASTER grass trail: when the trail texture was last relaxed (sim seconds; None = never,
/// the texture is clear) and the fraction of an R8 step not yet applied.
#[derive(Default)]
struct TrailState {
    time: Option<f32>,
    carry: f32,
}

/// REMASTER grass trail (trail.wgsl): the relax (full-texture subtract) and stamp (footprint
/// quads, max blend) pipelines into the R8 trail, and their uniform (the map transform, the step).
struct TrailGpu {
    relax: wgpu::RenderPipeline,
    stamp: wgpu::RenderPipeline,
    uniform: wgpu::Buffer,
    bind: wgpu::BindGroup,
}

/// REMASTER Ultra grass blades (grass.wgsl, 18-grass.md "Ultra", Gitea #31): the placement
/// compute pipeline, the draw pipelines per target ([hdr][sample count], built on first use),
/// the blade records (`ULTRA_MAX_BLADES` x 32 bytes), the indirect draw args the placement
/// counts into, and the draw's group 3 over the records. Built once, on the first Ultra frame.
struct BladeGpu {
    compute: wgpu::ComputePipeline,
    draw: [[OnceCell<wgpu::RenderPipeline>; 5]; 2],
    records: wgpu::Buffer,
    args: wgpu::Buffer,
    read_bind: wgpu::BindGroup,
}

/// The scene pipelines of one mode and sample count (09-remaster I3).
struct Pipes {
    tri: wgpu::RenderPipeline,
    /// REMASTER water surface (`vs_water` / `fs_water`); None in Classic
    water: Option<wgpu::RenderPipeline>,
    /// REMASTER grass shells (`vs_grass` / `fs_grass`); None in Classic
    grass: Option<wgpu::RenderPipeline>,
    tri_mirror: wgpu::RenderPipeline,
    line: wgpu::RenderPipeline,
    /// DEBUG wire styles (`vs_flat` / `fs_flat`)
    wire: wgpu::RenderPipeline,
    wire_fill: wgpu::RenderPipeline,
    wire_xray: wgpu::RenderPipeline,
    sky: wgpu::RenderPipeline,
    sky_cube: wgpu::RenderPipeline,
    /// REMASTER only
    sky_dome: Option<wgpu::RenderPipeline>,
    sky_cube_dome: Option<wgpu::RenderPipeline>,
    /// CHEAT the cloud-ceiling sky of lemmy / ace (#316) and ramlosa / h2o (#317), both modes
    hell_sky: wgpu::RenderPipeline,
    shadow: wgpu::RenderPipeline,
    /// REMASTER detailed skid marks (`vs_skid` / `fs_skid`); None in Classic
    skid: Option<wgpu::RenderPipeline>,
    /// CLASSIC flat skid marks (`fs_skid_flat`)
    skid_flat: wgpu::RenderPipeline,
    /// CHEAT tribute to tron, REMASTER Neon grid (#320): the light walls; None in Classic
    light_wall: Option<wgpu::RenderPipeline>,
    halo: wgpu::RenderPipeline,
    smoke: wgpu::RenderPipeline,
    spark: wgpu::RenderPipeline,
    /// DEBUG: line list without depth test (`Frame::debug_xray`)
    xray: wgpu::RenderPipeline,
    xray_solid: wgpu::RenderPipeline,
    holo: wgpu::RenderPipeline,
    holo_fill: wgpu::RenderPipeline,
    ghost: Option<wgpu::RenderPipeline>,
    blur: wgpu::RenderPipeline,
}

/// REMASTER render targets of the main view, rebuilt when the size or options change.
struct Targets {
    /// output size, render size (SSAA), samples, post pass, HDR, velocity blur, ambient occlusion,
    /// volumetric fog, FXAA, water
    key: (wgpu::Extent3d, wgpu::Extent3d, u32, bool, bool, bool, bool, bool, PostAaKind, bool),
    /// multisampled colour and depth (MSAA on; the depth can be sampled with velocity blur)
    msaa: Option<(wgpu::TextureView, wgpu::TextureView)>,
    /// velocity blur without MSAA: a depth that can be sampled (the caller's can't)
    depth: Option<wgpu::TextureView>,
    vblur: Option<VelTargets>,
    ssao: Option<SsaoTargets>,
    /// REMASTER volumetric fog: the apply pass's bind group over the scene's depth
    vol: Option<wgpu::BindGroup>,
    /// the resolved scene before the post pass (`HDR` or the swapchain format): texture,
    /// view, post bind group
    scene: Option<(wgpu::Texture, wgpu::TextureView, wgpu::BindGroup)>,
    /// SSAA: the scene at the render size, with the bind group of the downsample into `scene`
    hi: Option<(wgpu::Texture, wgpu::TextureView, wgpu::BindGroup)>,
    /// post AA: the tonemapped image the post pass writes instead of the surface, with the bind
    /// group of the AA pass that reads it (13.3 A3)
    ldr: Option<(wgpu::Texture, wgpu::TextureView, wgpu::BindGroup)>,
    /// SMAA: the edge and weight targets with the bind groups of its three passes
    smaa: Option<SmaaTargets>,
    /// TAA: the two history images with their passes' bind groups
    taa: Option<TaaTargets>,
    /// HDR bloom chain: one view per mip, and per mip the bind groups of its downsample
    /// (reads the scene or the mip above) and upsample (reads the mip below) passes
    bloom: Option<(Vec<wgpu::TextureView>, Vec<wgpu::BindGroup>, Vec<wgpu::BindGroup>)>,
    /// REMASTER water: the scene copy and the water pass's group 3
    water: Option<WaterTargets>,
}

/// What a view keeps between frames: its size-keyed targets (TAA history included), the motion
/// blur history and the last view matrices, and the TAA jitter index. One per window.
#[derive(Default)]
struct ViewState {
    targets: Option<Targets>,
    history: Option<History>,
    prev_view: Option<PrevView>,
    taa_frame: u32,
}

/// REMASTER TAA targets (taa.wgsl): two history images at the output size in the scene's format,
/// ping-ponged; `read` is the newest.
struct TaaTargets {
    hist: [wgpu::TextureView; 2],
    /// per history image: the resolve reading it (and the scene), the sharpen reading it
    resolve: [wgpu::BindGroup; 2],
    sharpen: [wgpu::BindGroup; 2],
    read: Cell<usize>,
    /// the history holds the previous frame of this view (cleared on a cut or when TAA was off)
    valid: Cell<bool>,
}

/// REMASTER TAA (taa.wgsl): the resolve and sharpen pipelines per scene format (swapchain, `HDR`).
struct Taa {
    layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    uniform: wgpu::Buffer,
    resolve: [wgpu::RenderPipeline; 2],
    sharpen: [wgpu::RenderPipeline; 2],
}

/// REMASTER SMAA targets (smaa.wgsl): edges (RG8) and blend weights (RGBA8) at the output size.
struct SmaaTargets {
    edges: wgpu::TextureView,
    weights: wgpu::TextureView,
    /// each pass reads the tonemapped image, plus the target before it; the other bindings
    /// of the shared layout get a texture that is not written in that pass
    edges_bind: wgpu::BindGroup,
    weights_bind: wgpu::BindGroup,
    blend_bind: wgpu::BindGroup,
}

/// REMASTER velocity blur targets: a copy of the scene to read from, the cars' velocity and
/// its depth, the blur pass's bind group.
struct VelTargets {
    src: wgpu::Texture,
    vel: wgpu::TextureView,
    vel_depth: wgpu::TextureView,
    bind: wgpu::BindGroup,
    /// every pixel's blur vector, depth and shadow factor (`fs_motion`), and the bind group of
    /// the pass that writes it (a dummy where `bind` has the target)
    motion: wgpu::TextureView,
    pre: wgpu::BindGroup,
}

/// REMASTER velocity motion blur (vblur.wgsl, 09-remaster 9.17) [design choice].
struct VelBlur {
    /// the cars' velocity pass and its (current, previous) matrices per car slot
    vel_pipe: wgpu::RenderPipeline,
    vel_bind: wgpu::BindGroup,
    cars: wgpu::Buffer,
    /// per depth kind (single-sampled, MSAA)
    shaders: [wgpu::ShaderModule; 2],
    layouts: [wgpu::BindGroupLayout; 2],
    /// per depth kind and output format (swapchain, `HDR`)
    pipes: [[OnceCell<wgpu::RenderPipeline>; 2]; 2],
    /// the motion pass (`fs_motion`, always into `HDR`) per depth kind, and the 1x1 view that
    /// stands in for its target in its own bind group
    motion_pipes: [OnceCell<wgpu::RenderPipeline>; 2],
    motion_dummy: wgpu::TextureView,
    /// the sparks drawn over the blurred scene, same indices
    spark_pipes: [[OnceCell<wgpu::RenderPipeline>; 2]; 2],
    /// the Time Attack hologram drawn over the blurred scene (shell, edges) per output format
    holo_pipes: [OnceCell<[wgpu::RenderPipeline; 2]>; 2],
    uniform: wgpu::Buffer,
    sampler: wgpu::Sampler,
}

/// REMASTER velocity blur: the previous main view (camera, view-projection, car matrices by mesh).
#[derive(Clone)]
struct PrevView {
    view_proj: Mat4,
    pos: Vec3,
    fwd: Vec3,
    cars: std::collections::HashMap<usize, Mat4>,
    /// `Frame::props` as drawn (mesh, model), by index
    props: Vec<(usize, Mat4)>,
}

/// REMASTER velocity blur: a saved `PrevView` (`Gpu::motion_history`).
pub struct MotionHistory(Option<PrevView>);

/// REMASTER velocity blur: how the car shadows move with their cars.
#[derive(Clone, Copy)]
enum VelShadow {
    None,
    /// real shadows from the shadow map, darkening the world by up to this share
    Map(f32),
    /// the original's projected shadows
    Projected,
}

/// REMASTER velocity blur: the matrix that drops a point straight down onto the plane
/// y = a x + b z + c fitted (least squares) through the wheel hubs lowered by `VEL_WHEEL_R`.
/// `None` without three hubs.
fn ground_flatten(hubs: &[Vec3]) -> Option<Mat4> {
    if hubs.len() < 3 {
        return None;
    }
    let mid = hubs.iter().sum::<Vec3>() / hubs.len() as f32;
    let (mut xx, mut xz, mut zz, mut xy, mut zy) = (0.0, 0.0, 0.0, 0.0, 0.0);
    for p in hubs {
        let d = *p - mid;
        (xx, xz, zz, xy, zy) = (xx + d.x * d.x, xz + d.x * d.z, zz + d.z * d.z, xy + d.x * d.y, zy + d.z * d.y);
    }
    let det = xx * zz - xz * xz;
    let (a, b) = if det.abs() > 1e-4 { ((xy * zz - zy * xz) / det, (zy * xx - xy * xz) / det) } else { (0.0, 0.0) };
    let c = mid.y - VEL_WHEEL_R - a * mid.x - b * mid.z;
    Some(Mat4::from_cols(Vec4::new(1.0, a, 0.0, 0.0), Vec4::ZERO, Vec4::new(0.0, b, 1.0, 0.0), Vec4::new(0.0, c, 0.0, 1.0)))
}

/// REMASTER velocity blur: draws in the velocity pass (cars, projected shadows, moving props).
const VEL_MAX: usize = 128;
/// Added to the ghost's mesh indices in the velocity pass (a multiple of `CAR_MESHES`, above any mesh).
const VEL_GHOST_KEY: usize = CAR_MESHES * 1_000_000;
/// Wheel radius under the wheel hubs for the shadow footprint's ground plane, m [APPROX].
const VEL_WHEEL_R: f32 = 0.33;

/// REMASTER post pass (post.wgsl): HDR tonemap and bloom composite, colour grade.
struct Post {
    pipe: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    uniform: wgpu::Buffer,
    /// bound as the bloom texture when there is none
    no_bloom: wgpu::TextureView,
    sampler: wgpu::Sampler,
}

/// REMASTER SSAA: the most pixels the scene is drawn with (16x of a 1280x800 window asks for
/// 16.4 M, of a 4K window for 133 M, which clamps; the colour, depth and velocity targets take
/// about 1.2 GB at 80 M) [design choice].
const SSAA_MAX_PIXELS: f64 = 80e6;

/// REMASTER anti-aliasing passes (aa.wgsl, 13-antialiasing.md): the SSAA downsample and FXAA.
struct Aa {
    layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    /// the output size in pixels
    uniform: wgpu::Buffer,
    /// downsample into the swapchain format or `HDR`
    down: [wgpu::RenderPipeline; 2],
    /// FXAA 3.11 (swapchain format in and out)
    fxaa: wgpu::RenderPipeline,
}

/// REMASTER SMAA 1x (smaa.wgsl): the edge, weight and blend pipelines.
struct Smaa {
    layout: wgpu::BindGroupLayout,
    /// edge threshold and search length
    uniform: wgpu::Buffer,
    edges: wgpu::RenderPipeline,
    weights: wgpu::RenderPipeline,
    blend: wgpu::RenderPipeline,
}

/// REMASTER: the sharp HUD's pipeline and bind group (UI atlas + field atlas), the same
/// pipeline for the glow target, and the field atlas for other bind groups
struct UiSharp {
    pipe: wgpu::RenderPipeline,
    bind: wgpu::BindGroup,
    glow_pipe: wgpu::RenderPipeline,
    fields: wgpu::TextureView,
}

/// REMASTER smooth glow (glow.wgsl): sources at half size, a gaussian blur, added over the
/// target. The two half-size targets are made on first use and kept.
struct Glow {
    blur: wgpu::RenderPipeline,
    comp: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    targets: RefCell<Option<GlowTargets>>,
}

/// REMASTER frosted hologram (#296, `Gpu::draw_hologram_frosted`): the composite pipeline
/// (holofrost.wgsl), its group 3, and the window-size sharp copy plus two half-size blur targets
/// for one window size (remade when it changes).
struct HoloFrost {
    pipe: wgpu::RenderPipeline,
    group: wgpu::BindGroupLayout,
    targets: RefCell<Option<FrostTargets>>,
}

/// the frosted hologram's window size, its sharp target (colour format), two half-size `HDR`
/// targets, and the composite's group 3 (sharp, the blurred one, the glow's sampler)
type FrostTargets = ((u32, u32), wgpu::TextureView, [wgpu::TextureView; 2], wgpu::BindGroup);

/// REMASTER frosted scene (#300, `Gpu::draw_scene_frosted`): the composite pipeline
/// (scenefrost.wgsl) and, for one window size, the sharp copy and two half-size blur targets.
/// REMASTER menu car preview: the multisample count of its offscreen frame [design choice]
const STUDIO_MSAA: u32 = 4;

/// REMASTER menu car preview: the premultiplied composite of its resolved frame over the window, and that
/// frame's texture view and bind group (window size of the rectangle it covers).
struct StudioAa {
    pipe: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    frame: RefCell<Option<StudioFrame>>,
}

/// the offscreen frame's size, view and bind group
type StudioFrame = ((u32, u32), wgpu::TextureView, wgpu::BindGroup);

/// CHEAT ramlosa / h2o (#317, `Gpu::draw_underwater`): the wobble pass's pipeline (underwater.wgsl) and, for one window
/// size, the copy the scene is drawn into.
struct UnderwaterGpu {
    pipe: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    copy: RefCell<Option<((u32, u32), wgpu::TextureView)>>,
}

struct SceneFrostGpu {
    pipe: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    targets: RefCell<Option<SceneFrostTargets>>,
}

/// the frosted scene's window size, its sharp copy (colour format) and two half-size `HDR` blur targets
type SceneFrostTargets = ((u32, u32), wgpu::TextureView, [wgpu::TextureView; 2]);

/// the glow's half-size size and its two ping-pong targets
type GlowTargets = ((u32, u32), [wgpu::TextureView; 2]);

/// the glow's blur radius (gaussian sigma, canvas px) and the gain of the added light
const GLOW_SIGMA: f32 = 4.0;
const GLOW_COMPOSITE: f32 = 1.0;

/// REMASTER bloom (bloom.wgsl): a dual-filter chain, 13-tap downsamples (the first with a
/// soft threshold and Karis average) then additive tent upsamples [design choice].
struct Bloom {
    first: wgpu::RenderPipeline,
    down: wgpu::RenderPipeline,
    up: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    uniform: wgpu::Buffer,
}

/// REMASTER ambient occlusion (ssao.wgsl, 09-remaster 9.28) [design choice]: GTAO at half size,
/// a depth-aware blur and a multiply over the scene.
struct Ssao {
    /// one shader per depth kind (single-sampled, MSAA)
    shaders: [wgpu::ShaderModule; 2],
    gtao_layouts: [wgpu::BindGroupLayout; 2],
    gtao: [wgpu::RenderPipeline; 2],
    blur_layout: wgpu::BindGroupLayout,
    blur_h: wgpu::RenderPipeline,
    blur_v: wgpu::RenderPipeline,
    apply_layout: wgpu::BindGroupLayout,
    /// per sample count (log2) and output (swapchain, `HDR`)
    apply: [[OnceCell<wgpu::RenderPipeline>; 2]; 5],
    uniform: wgpu::Buffer,
    sampler: wgpu::Sampler,
}

/// REMASTER ambient occlusion targets of the main view: two half-size (visibility, linear depth)
/// textures (A: GTAO, then the vertical blur; B: the horizontal blur) and each pass's bind group.
struct SsaoTargets {
    a: wgpu::TextureView,
    b: wgpu::TextureView,
    gtao: wgpu::BindGroup,
    blur_h: wgpu::BindGroup,
    blur_v: wgpu::BindGroup,
    apply: wgpu::BindGroup,
}

/// REMASTER, Enhanced car paint: a small cube map of the surroundings, redrawn every frame around the
/// followed car [design choice]. Linear HDR light (`HDR`, drawn with `LINEAR_FLAG`): the sun and the
/// lamps keep their real brightness, the paint shaders decode it (`env_gamma`).
struct EnvProbe {
    /// one render view per cube face (+X -X +Y -Y +Z -Z)
    faces: Vec<wgpu::TextureView>,
    cube: wgpu::TextureView,
    depth: wgpu::TextureView,
    /// 1x1 stand-in bound while the probe itself is the render target
    dummy: wgpu::TextureView,
    /// the cube itself (DEBUG `Gpu::env_cube` reads it back)
    tex: wgpu::Texture,
}

/// Probe face size in pixels and its far plane in metres [design choice].
const PROBE_SIZE: u32 = 128;
const PROBE_FAR: f32 = 400.0;

impl EnvProbe {
    fn new(d: &wgpu::Device) -> Self {
        let cube_tex = |size, label| {
            d.create_texture(&wgpu::TextureDescriptor {
                label: Some(label),
                size: wgpu::Extent3d { width: size, height: size, depth_or_array_layers: 6 },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: HDR,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
                view_formats: &[],
            })
        };
        let cube_view = |t: &wgpu::Texture| t.create_view(&wgpu::TextureViewDescriptor { dimension: Some(wgpu::TextureViewDimension::Cube), ..Default::default() });
        let tex = cube_tex(PROBE_SIZE, "env probe");
        let faces = (0..6)
            .map(|i| {
                tex.create_view(&wgpu::TextureViewDescriptor {
                    dimension: Some(wgpu::TextureViewDimension::D2),
                    base_array_layer: i,
                    array_layer_count: Some(1),
                    ..Default::default()
                })
            })
            .collect();
        let depth = d
            .create_texture(&wgpu::TextureDescriptor {
                label: Some("env probe depth"),
                size: wgpu::Extent3d { width: PROBE_SIZE, height: PROBE_SIZE, depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: DEPTH,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                view_formats: &[],
            })
            .create_view(&Default::default());
        EnvProbe { faces, cube: cube_view(&tex), depth, dummy: cube_view(&cube_tex(1, "env dummy")), tex }
    }

    /// Face cameras in wgpu's cube order. A cube lookup is left-handed, so the faces are drawn
    /// with x flipped (the mirrored mesh pipeline); the up vectors make the face images line
    /// up with `textureSample(cube, r)` for the world-space direction r.
    fn face_views(centre: Vec3) -> [(Mat4, Vec3); 6] {
        let (x, y, z) = (Vec3::X, Vec3::Y, Vec3::Z);
        [(x, y), (-x, y), (y, -z), (-y, z), (z, y), (-z, y)].map(|(f, up)| (glam::camera::rh::view::look_to_mat4(centre, f, up), f))
    }
}

/// The group 0 bind groups of a scene (`Gpu::global_binds`).
struct GlobalBinds {
    bind: wgpu::BindGroup,
    car_binds: Vec<wgpu::BindGroup>,
    prop_bind: wgpu::BindGroup,
    probe_bind: wgpu::BindGroup,
    probe_car_binds: Vec<wgpu::BindGroup>,
    probe_prop_bind: wgpu::BindGroup,
}

/// REMASTER local light (#83): the probe volume on the GPU (probes.rs `Packed`, shader.wgsl
/// `local_ambient`).
struct LocalLightGpu {
    index: wgpu::TextureView,
    atlas: wgpu::TextureView,
    /// the sun's share per probe (#192), the atlas layout of one channel
    sun: wgpu::TextureView,
    /// the brick coordinates of the index's low corner; None = the 1x1x1 dummies
    origin: Option<[i32; 3]>,
    /// when a volume baked in the background was uploaded (it fades in, `LOCAL_FADE`); None =
    /// shown at full strength
    shown: Option<std::time::Instant>,
}

struct GpuScene {
    tris: wgpu::Buffer,
    n_tris: u32,
    /// the world's axis-aligned box (min, max): the sun shafts' caster depth range
    world_box: (Vec3, Vec3),
    /// REMASTER world fill (`Scene::fill`); None when there is none
    fill: Option<(wgpu::Buffer, u32)>,
    /// REMASTER water surface (`Scene::water`) and the world without the triangles tagged as water
    water: Option<(wgpu::Buffer, u32)>,
    tris_dry: Option<(wgpu::Buffer, u32)>,
    water_params: Vec<[[f32; 4]; crate::procgen::water::ROWS]>,
    /// REMASTER grass shells (`Scene::grass`) and their bodies' parameters
    grass: Option<(wgpu::Buffer, u32)>,
    grass_params: Vec<[[f32; 4]; crate::procgen::grass::ROWS]>,
    /// the edge map's group 3 (a 1x1 zero without one) and, with one, its transform and cap
    grass_bind: wgpu::BindGroup,
    grass_edge: Option<([f32; 4], f32)>,
    /// REMASTER grass trail (#158): the R8 "pressed how recently" map over the edge map's
    /// rectangle (binding 2 of `grass_bind`; None without grass) and the decay state
    grass_trail: Option<(wgpu::Texture, wgpu::TextureView)>,
    trail: RefCell<TrailState>,
    /// REMASTER Ultra grass blades: the placement pass's group 1 over this scene's grass
    /// vertices (built on the first Ultra frame, `Gpu::blade_pass`; None again after `set_grass`)
    blade_bind: RefCell<Option<wgpu::BindGroup>>,
    lines: wgpu::Buffer,
    n_lines: u32,
    bind: wgpu::BindGroup,
    /// (vertex buffer, vertex count) per car mesh
    cars: Vec<(wgpu::Buffer, u32)>,
    /// REMASTER the same for `Scene::remaster_cars` (None: draw the original); read through `car`
    remaster_cars: Vec<Option<(wgpu::Buffer, u32)>>,
    /// CLASSIC body levels OBJ1, OBJ2, OBJ3 per car (`Scene::car_lods`; None: draw OBJ0)
    car_lods: Vec<[Option<(wgpu::Buffer, u32)>; 3]>,
    car_binds: Vec<wgpu::BindGroup>,
    prop_bind: wgpu::BindGroup,
    /// the same with the dummy env cube, for drawing into the probe
    probe_bind: wgpu::BindGroup,
    probe_car_binds: Vec<wgpu::BindGroup>,
    probe_prop_bind: wgpu::BindGroup,
    /// the textures of the group 0 bind groups (layers, sky band, sky dome, texture pack): kept to rebuild
    /// them when the local light is baked
    views: [wgpu::TextureView; 4],
    /// REMASTER texture pack: the layer ids per base layer (`pack_ids` in the shader)
    pack_map: wgpu::Buffer,
    /// REMASTER local light (#83): the probe volume (dummies until baked)
    local: LocalLightGpu,
    /// scene object meshes (object-local view space), skinned on the CPU each frame
    props: Vec<Vec<Vertex>>,
    look: Look,
    point_lights: PointLights,
    world_halos: Vec<HaloInst>,
    lamps: Vec<LampLight>,
    car_halos: Vec<Vec<HaloInst>>,
    /// REMASTER the halos of the generated bodies (empty: the original's); read through `halos`
    remaster_car_halos: Vec<Vec<HaloInst>>,
    prop_halos: Vec<Vec<HaloInst>>,
    /// REMASTER the scene objects' headlamps (`PROP_HEADLAMPS`)
    prop_lamps: Vec<PropLamps>,
    fx: crate::scene::FxLayers,
    /// sky image size in pixels (0 x 0 when the track has none)
    sky_size: (u32, u32),
    /// REMASTER sky dome: the sun's azimuth and elevation in the track's HDRI (None: the track has none)
    dome: Option<crate::sky::Dome>,
    view_distance: f32,
    /// wire buffers: world, then per car mesh (empty until `load_wireframe`)
    wire: Option<GpuWire>,
    car_wires: Vec<GpuWire>,
    /// the same for the generated bodies and wheels (`Scene::remaster_cars`); None where there is none
    car_wires_gen: Vec<Option<GpuWire>>,
    /// REMASTER hologram: the car meshes' edges (`Frame::hologram`)
    holo_wires: Vec<(wgpu::Buffer, u32)>,
    /// the same for the generated bodies and wheels (`Car models: Generated`); none where there is none
    holo_wires_gen: Vec<Option<(wgpu::Buffer, u32)>>,
    /// REMASTER hologram: the road surface's edges (front end Main drive, `Gpu::load_holo_road`)
    holo_road: Option<(wgpu::Buffer, u32)>,
    /// DEBUG (motorview Swap mesh): the car slot drawn with the other model than `generated`
    /// asks for, original <-> generated (`Gpu::set_car_swap`); None in the game
    car_swap: Option<usize>,
    /// REMASTER live paint (`Frame::paint`): per car slot the stock Colour1..3 (`Scene::car_colours`)
    /// and the brightness they were emitted with (`Scene::object_scale`)
    car_colours: Vec<[[u8; 3]; 3]>,
    object_scale: [f32; 3],
}

impl GpuScene {
    /// The world triangles to draw: REMASTER without the ones the water surface replaces (every
    /// world draw goes through here, so the texture never shows under or beside the water).
    fn world(&self, v: &View) -> (&wgpu::Buffer, u32) {
        match self.tris_dry.as_ref().filter(|_| v.water()) {
            Some((b, n)) => (b, *n),
            None => (&self.tris, self.n_tris),
        }
    }
    /// Whether car mesh `mesh` draws its generated model: `generated`, turned over for the
    /// swapped car (DEBUG `car_swap`; in Classic that shows the generated body through the
    /// Classic shading, whatever it makes of the generated parts' env tags).
    fn gen_for(&self, mesh: usize, generated: bool) -> bool {
        generated != (self.car_swap == Some(mesh / CAR_MESHES))
    }

    /// Car mesh `mesh`: REMASTER the generated one when `generated` and there is one, else the
    /// original.
    fn car(&self, mesh: usize, generated: bool) -> Option<&(wgpu::Buffer, u32)> {
        let generated = self.gen_for(mesh, generated);
        generated.then(|| self.remaster_cars.get(mesh)).flatten().and_then(Option::as_ref).or_else(|| self.cars.get(mesh))
    }

    /// REMASTER: car mesh `mesh` as `car`, or its original model (OBJ0 and the original wheels) when `far`
    /// and the generated one would be drawn. The halos stay those of the generated set.
    fn car_lod(&self, mesh: usize, generated: bool, far: bool) -> Option<&(wgpu::Buffer, u32)> {
        if far && self.gen_for(mesh, generated) {
            self.cars.get(mesh)
        } else {
            self.car(mesh, generated)
        }
    }

    /// CLASSIC car mesh `mesh` at body `level` (`CarDetail::body_level`): OBJ1 or OBJ2 of its car for a
    /// body, else (level 0, a wheel, a missing level) `car`.
    fn car_level(&self, mesh: usize, level: usize, generated: bool) -> Option<&(wgpu::Buffer, u32)> {
        let lod = (level > 0 && mesh.is_multiple_of(CAR_MESHES) && !self.gen_for(mesh, generated)).then(|| self.car_lods.get(mesh / CAR_MESHES)).flatten();
        lod.and_then(|l| l[level - 1].as_ref()).or_else(|| self.car(mesh, generated))
    }

    /// The halos of car mesh `mesh`: REMASTER the generated body's when `generated` and they moved.
    fn halos(&self, mesh: usize, generated: bool) -> Option<&Vec<HaloInst>> {
        let generated = self.gen_for(mesh, generated);
        generated.then(|| self.remaster_car_halos.get(mesh)).flatten().filter(|h| !h.is_empty()).or_else(|| self.car_halos.get(mesh))
    }

    /// The wireframe edges of car mesh `mesh`, like `car`.
    fn car_wire(&self, mesh: usize, generated: bool) -> Option<&GpuWire> {
        let generated = self.gen_for(mesh, generated);
        generated.then(|| self.car_wires_gen.get(mesh)).flatten().and_then(Option::as_ref).or_else(|| self.car_wires.get(mesh))
    }

    /// The hologram edges of car mesh `mesh`, like `car`.
    fn holo_wires(&self, mesh: usize, generated: bool) -> Option<&(wgpu::Buffer, u32)> {
        let generated = self.gen_for(mesh, generated);
        generated.then(|| self.holo_wires_gen.get(mesh)).flatten().and_then(Option::as_ref).or_else(|| self.holo_wires.get(mesh))
    }
}

/// DEBUG wire styles: one mesh's vertex lists on the GPU (wire.rs).
struct WireBufs {
    lines: (wgpu::Buffer, u32),
    xray: (wgpu::Buffer, u32),
    fill: (wgpu::Buffer, u32),
}

/// `plain` = Lines and Solid; `check` = the analysed Check style (cars only).
struct GpuWire {
    plain: WireBufs,
    check: Option<WireBufs>,
    stats: WireStats,
}

impl GpuWire {
    fn bufs(&self, style: WireStyle) -> &WireBufs {
        if style == WireStyle::Check { self.check.as_ref().unwrap_or(&self.plain) } else { &self.plain }
    }
}

/// Wireframe line colours (debug view).
const WIRE_WORLD: [f32; 4] = [0.2, 0.9, 0.3, 1.0];
const WIRE_CAR: [f32; 4] = [1.0, 0.85, 0.2, 1.0];
const WIRE_PROP: [f32; 4] = [0.3, 0.8, 1.0, 1.0];
/// hologram edge colour; alpha = the line's strength (ui.wgsl `fs_holo` keeps the hue)
const HOLO_LINE: [f32; 4] = [1.0, 1.0, 1.0, 0.20];
/// the generated bodies have many more, smaller triangles: fainter edges
const HOLO_LINE_GEN: [f32; 4] = [1.0, 1.0, 1.0, 0.04];

/// #335 the CPU half of `Gpu::load`, built on the race loading worker (`Gpu::load_prepared`,
/// `Gpu::begin_load` with the same `Scene`): the texture layers' mip chains, the world without
/// its water triangles, the hologram edges.
pub struct SceneUpload {
    /// levels 1.. of each base layer (level 0 is `Scene::layers`)
    mips: Vec<Vec<Vec<u8>>>,
    /// the same for the texture pack's layers (none for the 1x1 dummy without a pack)
    pack_mips: Vec<Vec<Vec<u8>>>,
    dry: Option<Vec<Vertex>>,
    holo: Vec<Vec<Vertex>>,
    holo_gen: Vec<Option<Vec<Vertex>>>,
    world_box: (Vec3, Vec3),
}

impl SceneUpload {
    pub fn new(s: &Scene) -> Self {
        let below = |layer: &[u8], size: u32| mips_of(layer, size).split_off(1);
        SceneUpload {
            mips: s.layers.iter().map(|l| below(l, TEX)).collect(),
            pack_mips: s.pack.layers.iter().map(|l| below(l, s.pack.size)).collect(),
            dry: dry_world(&s.tris, &s.water_hide, &s.water_bed),
            holo: s.cars.iter().map(|c| edges(c, HOLO_LINE)).collect(),
            holo_gen: s.remaster_cars.iter().map(|c| (!c.is_empty()).then(|| edges(c, HOLO_LINE_GEN))).collect(),
            world_box: s.tris.iter().fold((Vec3::splat(f32::MAX), Vec3::splat(f32::MIN)), |(lo, hi), v| (lo.min(Vec3::from(v.pos)), hi.max(Vec3::from(v.pos)))),
        }
    }
}

/// #335 vertex buffers above this many bytes are written a slice a step (about 1 ms each)
const UPLOAD_CHUNK: usize = 4 << 20;

/// #335 a scene upload in progress (`Gpu::begin_load`, `Gpu::load_step`).
pub struct SceneLoad {
    up: SceneUpload,
    tex: wgpu::Texture,
    layers: u32,
    pack_tex: wgpu::Texture,
    pack_layers: u32,
    pack_size: u32,
    sky: Option<wgpu::Texture>,
    hdri: wgpu::Texture,
    bufs: std::collections::HashMap<VSrc, (wgpu::Buffer, u32)>,
    /// the big buffer being written and the bytes written so far
    partial: Option<(wgpu::Buffer, usize)>,
    jobs: Vec<UploadStep>,
    next: usize,
}

impl SceneLoad {
    /// The share of the steps done, 0..1.
    pub fn frac(&self) -> f32 {
        self.next as f32 / self.jobs.len().max(1) as f32
    }

    /// The base texture layers this upload writes (the device's limit applied).
    pub fn layers(&self) -> u32 {
        self.layers
    }
}

#[derive(Clone, Copy)]
enum UploadStep {
    /// a base texture layer with its mips
    Layer(u32),
    PackLayer(u32),
    Sky,
    /// a level of the HDRI dome
    Hdri(u32),
    Buf(VSrc),
    /// the bind groups and the swap
    Finish,
}

/// The vertex lists of a scene upload.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum VSrc {
    Tris,
    Fill,
    Water,
    Dry,
    Grass,
    Lines,
    Car(usize),
    RemasterCar(usize),
    Lod(usize, usize),
    Holo(usize),
    HoloGen(usize),
}

impl VSrc {
    fn verts<'a>(self, s: &'a Scene, up: &'a SceneUpload) -> &'a [Vertex] {
        match self {
            VSrc::Tris => &s.tris,
            VSrc::Fill => &s.fill,
            VSrc::Water => &s.water,
            VSrc::Dry => up.dry.as_deref().unwrap_or_default(),
            VSrc::Grass => &s.grass,
            VSrc::Lines => &s.lines,
            VSrc::Car(i) => &s.cars[i],
            VSrc::RemasterCar(i) => &s.remaster_cars[i],
            VSrc::Lod(i, k) => &s.car_lods[i][k],
            VSrc::Holo(i) => &up.holo[i],
            VSrc::HoloGen(i) => up.holo_gen[i].as_deref().unwrap_or_default(),
        }
    }

    fn label(self) -> &'static str {
        match self {
            VSrc::Tris => "tris",
            VSrc::Fill => "world fill",
            VSrc::Water => "water",
            VSrc::Dry => "world without water",
            VSrc::Grass => "grass",
            VSrc::Lines => "lines",
            VSrc::Car(_) => "car",
            VSrc::RemasterCar(_) => "remaster car",
            VSrc::Lod(..) => "car level",
            VSrc::Holo(_) => "hologram",
            VSrc::HoloGen(_) => "hologram generated",
        }
    }
}


/// The box-filtered mip chain of a square RGBA8 image of `size` (a power of two).
fn mips_of(layer: &[u8], size: u32) -> Vec<Vec<u8>> {
    let mut out = vec![layer.to_vec()];
    let mut size = size as usize;
    while size > 1 {
        let prev = out.last().unwrap();
        let half = size / 2;
        let mut next = vec![0u8; half * half * 4];
        for (y, row) in next.chunks_exact_mut(half * 4).enumerate() {
            // the two source rows of this row, a pixel pair (8 bytes) per output pixel
            let (r0, r1) = (&prev[2 * y * size * 4..][..size * 4], &prev[(2 * y + 1) * size * 4..][..size * 4]);
            for ((px, a), b) in row.as_chunks_mut::<4>().0.iter_mut().zip(r0.as_chunks::<8>().0).zip(r1.as_chunks::<8>().0) {
                for c in 0..4 {
                    px[c] = ((a[c] as u32 + a[c + 4] as u32 + b[c] as u32 + b[c + 4] as u32) / 4) as u8;
                }
            }
        }
        out.push(next);
        size = half;
    }
    out
}

impl Gpu {
    /// `adapter` tells which MSAA counts above 4 the device can use (`crate::device` asks for
    /// the adapter's format features).
    pub fn new(adapter: &wgpu::Adapter, device: wgpu::Device, queue: wgpu::Queue, color_format: wgpu::TextureFormat) -> Self {
        let device_limit = device.limits().max_texture_dimension_2d;
        let specific = device.features().contains(wgpu::Features::TEXTURE_ADAPTER_SPECIFIC_FORMAT_FEATURES);
        let msaa_counts = RemasterView::MSAA_LEVELS
            .into_iter()
            .filter(|&n| match n {
                1 | 4 => true,
                _ => specific && [color_format, HDR, DEPTH].iter().all(|&f| adapter.get_texture_format_features(f).flags.sample_count_supported(n)),
            })
            .collect();
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("shader"),
            source: wgpu::ShaderSource::Wgsl(SHADER_SRC.into()),
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("globals"),
            entries: &[
                // (COMPUTE: the REMASTER Ultra grass blade placement, `Gpu::blade_pass`, reads the
                // globals and samples the world textures)
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT | wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT | wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2Array,
                        multisampled: false,
                    },
                    count: None,
                },
                // (VERTEX: the local light's atlas in `vs_remaster`)
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT | wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 4,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::Cube,
                        multisampled: false,
                    },
                    count: None,
                },
                // REMASTER sky dome HDRI (equirectangular, RGBA16F; a 1x1 black one when the track has none)
                wgpu::BindGroupLayoutEntry {
                    binding: 5,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                // REMASTER local light (#83): the probe volume's brick index (R32Uint) and atlas
                // (RGBA16F), `LocalLightGpu`; read per vertex too (per-pixel light off)
                wgpu::BindGroupLayoutEntry {
                    binding: 6,
                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                    ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Uint, view_dimension: wgpu::TextureViewDimension::D3, multisampled: false },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 7,
                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                    ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: true }, view_dimension: wgpu::TextureViewDimension::D3, multisampled: false },
                    count: None,
                },
                // and the probes' sun term (R8, #192)
                wgpu::BindGroupLayoutEntry {
                    binding: 8,
                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                    ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: true }, view_dimension: wgpu::TextureViewDimension::D3, multisampled: false },
                    count: None,
                },
                // REMASTER texture pack (#211, texpack.rs): its layers (a 1x1 dummy without one) and,
                // per layer of binding 1, the pack's colour and normal layer ids
                wgpu::BindGroupLayoutEntry {
                    binding: 9,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: true }, view_dimension: wgpu::TextureViewDimension::D2Array, multisampled: false },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 10,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None },
                    count: None,
                },
                // CLASSIC scene objects' own point lights (`prop_myw`) and the record of each vertex (`prop_of`)
                wgpu::BindGroupLayoutEntry {
                    binding: 11,
                    visibility: wgpu::ShaderStages::VERTEX,
                    ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Storage { read_only: true }, has_dynamic_offset: false, min_binding_size: None },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 12,
                    visibility: wgpu::ShaderStages::VERTEX,
                    ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Storage { read_only: true }, has_dynamic_offset: false, min_binding_size: None },
                    count: None,
                },
            ],
        });
        let pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("layout"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let aniso_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("anisotropic sampler"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                count: None,
            }],
        });
        let blur_layout = blur_bind_layout(&device);
        // REMASTER car shadows: group 2 samples the map (read) or holds its uniform (caster pass)
        let uniform_entry_at = |binding, visibility| wgpu::BindGroupLayoutEntry {
            binding,
            visibility,
            ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None },
            count: None,
        };
        let uniform_entry = |visibility| uniform_entry_at(0, visibility);
        let shadow_read = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("shadow map"),
            entries: &[
                uniform_entry(wgpu::ShaderStages::FRAGMENT),
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Depth, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Comparison),
                    count: None,
                },
                // REMASTER lamp shadows: the atlas and its matrices
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Depth, view_dimension: wgpu::TextureViewDimension::D2Array, multisampled: false },
                    count: None,
                },
                uniform_entry_at(4, wgpu::ShaderStages::FRAGMENT),
            ],
        });
        let shadow_write = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("shadow caster"),
            entries: &[uniform_entry(wgpu::ShaderStages::VERTEX)],
        });
        let water_groups = [false, true].map(|msaa| {
            let depth = wgpu::BindGroupLayoutEntry {
                binding: 1,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Depth, view_dimension: wgpu::TextureViewDimension::D2, multisampled: msaa },
                count: None,
            };
            // REMASTER binding 5: the volumetric fog volume (a 3D texture), read by the surface in a volumetric view
            let fog_volume = wgpu::BindGroupLayoutEntry {
                binding: 5,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: true }, view_dimension: wgpu::TextureViewDimension::D3, multisampled: false },
                count: None,
            };
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor { label: Some("water"), entries: &[tex_entry(0, true), depth, tex_entry(2, true), tex_entry(3, true), sampler_entry(4), fog_volume] })
        });
        // (the grass group is read by the Ultra blade placement compute pass too)
        let compute_too = |mut e: wgpu::BindGroupLayoutEntry| {
            e.visibility |= wgpu::ShaderStages::COMPUTE;
            e
        };
        let grass_group = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor { label: Some("grass edge"), entries: &[compute_too(tex_entry(0, true)), compute_too(sampler_entry(1)), compute_too(tex_entry(2, true))] });
        // REMASTER Ultra grass blades (grass.wgsl `cs_blades`, `vs_blade`): group 1 of the compute
        // pass = the grass vertices, the blade records and the indirect args (bindings 1..3:
        // binding 0 of group 1 is the sampler of the draw layouts); group 3 of the draw = the
        // records, read-only, at binding 3 (0..2 are the edge map's)
        let storage = |binding, read_only, visibility| wgpu::BindGroupLayoutEntry {
            binding,
            visibility,
            ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Storage { read_only }, has_dynamic_offset: false, min_binding_size: None },
            count: None,
        };
        let blade_group = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("grass blades"),
            entries: &[storage(1, true, wgpu::ShaderStages::COMPUTE), storage(2, false, wgpu::ShaderStages::COMPUTE), storage(3, false, wgpu::ShaderStages::COMPUTE)],
        });
        let blade_read_group = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor { label: Some("grass blades read"), entries: &[storage(3, true, wgpu::ShaderStages::VERTEX)] });
        let src = PipeSrc {
            shader,
            layout: pl,
            remaster_layout: device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("remaster layout"),
                bind_group_layouts: &[Some(&layout), Some(&aniso_layout), Some(&shadow_read)],
                immediate_size: 0,
            }),
            shadow_layout: device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("shadow caster layout"),
                bind_group_layouts: &[Some(&layout), Some(&aniso_layout), Some(&shadow_write)],
                immediate_size: 0,
            }),
            blur_shader: device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("motion blur shader"),
                source: wgpu::ShaderSource::Wgsl(include_str!("blur.wgsl").into()),
            }),
            blur_layout: device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("motion blur"), bind_group_layouts: &[Some(&blur_layout)], immediate_size: 0 }),
            water_layouts: water_groups.each_ref().map(|wg| {
                device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                    label: Some("water layout"),
                    bind_group_layouts: &[Some(&layout), Some(&aniso_layout), Some(&shadow_read), Some(wg)],
                    immediate_size: 0,
                })
            }),
            water_groups,
            grass_layout: device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("grass layout"),
                bind_group_layouts: &[Some(&layout), Some(&aniso_layout), Some(&shadow_read), Some(&grass_group)],
                immediate_size: 0,
            }),
            // (no group 2: the placement needs no shadow map)
            blade_compute_layout: device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("grass blade placement layout"),
                bind_group_layouts: &[Some(&layout), Some(&blade_group), None, Some(&grass_group)],
                immediate_size: 0,
            }),
            blade_draw_layout: device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("grass blade layout"),
                bind_group_layouts: &[Some(&layout), Some(&aniso_layout), Some(&shadow_read), Some(&blade_read_group)],
                immediate_size: 0,
            }),
            grass_group,
            blade_group,
            blade_read_group,
        };
        let classic = scene_pipes(&device, &src, color_format, 1, false, None, None, false);
        let ubuf = |label| {
            device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(label),
                size: std::mem::size_of::<Globals>() as u64,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            })
        };
        let uniforms = ubuf("globals");
        let car_uniforms = (0..MAX_CARS).map(|_| ubuf("car globals")).collect();
        let prop_uniform = ubuf("prop globals");
        let sbuf = |label, size: usize| {
            device.create_buffer(&wgpu::BufferDescriptor { label: Some(label), size: size as u64, usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false })
        };
        let prop_lights = sbuf("prop lights", PROP_RECS * PROP_REC * 16);
        let prop_of = sbuf("prop light records", PROP_VERTS * 4);
        let filtering = wgpu::SamplerDescriptor {
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::Repeat,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Linear,
            ..Default::default()
        };
        let sampler = device.create_sampler(&filtering);
        let aniso = RemasterView::ANISOTROPY_LEVELS
            .iter()
            .map(|&a| {
                let s = device.create_sampler(&wgpu::SamplerDescriptor { anisotropy_clamp: a, ..filtering.clone() });
                device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("anisotropic sampler"),
                    layout: &aniso_layout,
                    entries: &[wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::Sampler(&s) }],
                })
            })
            .collect();
        let (ui_pipe, ui_layout) = ui_pipeline(&device, color_format, false);
        let ui_uniform = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("ui globals"),
            size: 32,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let rev_look = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("rev look"),
            contents: bytemuck::cast_slice(&crate::revmeter::RevLook::default().pack()),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });
        let probe = EnvProbe::new(&device);
        Gpu {
            src,
            classic,
            remaster: Default::default(),
            msaa_counts,
            max_dim: device_limit,
            aniso,
            shadow: OnceCell::new(),
            shaft: OnceCell::new(),
            shadow_layouts: (shadow_read, shadow_write),
            shadow_ready: Cell::new(false),
            far_mask: Cell::new(0),
            far_memory: Cell::new(0),
            spot: [OnceCell::new(), OnceCell::new(), OnceCell::new()],
            spot_size: Cell::new(0),
            spot_alloc: RefCell::new(SpotAlloc::default()),
            head_alloc: RefCell::new(SpotAlloc::with_layers(HEAD_LAYERS)),
            spot_ready: Cell::new(false),
            targets: RefCell::new(None),
            post: OnceCell::new(),
            bloom: OnceCell::new(),
            aa: OnceCell::new(),
            smaa: OnceCell::new(),
            taa: OnceCell::new(),
            ssao: OnceCell::new(),
            volfog: OnceCell::new(),
            water: OnceCell::new(),
            grass: OnceCell::new(),
            trail: OnceCell::new(),
            blades: OnceCell::new(),
            frame_no: Cell::new(0),
            taa_frame: Cell::new(0),
            sky_scroll: RefCell::new(Vec::new()),
            probe,
            ui_pipe,
            ui_layout,
            ui_uniform,
            rev_look,
            ui_bind: None,
            ui_atlas: None,
            ui_tex: None,
            ui_sharp: OnceCell::new(),
            glow: OnceCell::new(),
            holo_frost: OnceCell::new(),
            scene_frost: OnceCell::new(),
            frosting: Cell::new(false),
            underwater: OnceCell::new(),
            warping: Cell::new(false),
            studio: OnceCell::new(),
            studio_pipes: OnceCell::new(),
            studio_aa: Cell::new(false),
            device,
            queue,
            color_format,
            blur_layout,
            history: RefCell::new(None),
            vel_blur: OnceCell::new(),
            dissolve: OnceCell::new(),
            prev_view: RefCell::new(None),
            views: RefCell::new(Vec::new()),
            view_now: Cell::new(0),
            flare_vis: Default::default(),
            water_clock: std::time::Instant::now(),
            clock_pin: Cell::new(None),
            layout,
            uniforms,
            car_uniforms,
            prop_uniform,
            prop_lights,
            prop_of,
            sampler,
            scene: None,
            local_src: None,
            local_bake: None,
            local_sun: None,
            local_want: ([0.0; 3], std::time::Instant::now()),
            local_wait: false,
        }
    }

    /// REMASTER pipelines for a sample count (one of `msaa_counts`) and target (`hdr`: the HDR
    /// scene target, else the swapchain format), built on first use.
    fn remaster_pipes(&self, samples: u32, hdr: bool) -> &Pipes {
        let format = if hdr { HDR } else { self.color_format };
        self.remaster[hdr as usize][samples.ilog2() as usize].get_or_init(|| {
            let ms = (samples > 1) as usize;
            scene_pipes(&self.device, &self.src, format, samples, true, Some((&self.water_gpu().modules[ms], &self.src.water_layouts[ms])), Some(self.grass_shader()), false)
        })
    }

    /// REMASTER menu car preview: pipelines for its offscreen frame, which keep the coverage alpha (no water or
    /// grass: the preview draws cars only)
    fn studio_pipes(&self, samples: u32) -> &Pipes {
        self.studio_pipes.get_or_init(|| scene_pipes(&self.device, &self.src, self.color_format, samples, true, None, None, true))
    }

    /// Pin the animation clock (water, grass wind) at `t` seconds, or let it run again (None):
    /// headless shots pin it at their sim time so a shot repeats byte for byte.
    pub fn pin_clock(&self, t: Option<f32>) {
        self.clock_pin.set(t);
    }

    /// REMASTER: the largest supported MSAA count up to `want`
    pub fn msaa_samples(&self, want: u32) -> u32 {
        self.msaa_counts.iter().copied().filter(|&n| n <= want).max().unwrap_or(1)
    }

    /// REMASTER: the sampler group for an anisotropy (the nearest level at or below it)
    fn aniso_bind(&self, a: u16) -> &wgpu::BindGroup {
        &self.aniso[RemasterView::ANISOTROPY_LEVELS.iter().rposition(|&l| l <= a).unwrap_or(0)]
    }

    /// REMASTER main-view targets for this frame (rebuilt on a size or option change).
    /// `size` is the output, `render` what the scene is drawn at (larger with SSAA, `render_size`).
    #[allow(clippy::too_many_arguments)]
    fn targets(&self, size: wgpu::Extent3d, render: wgpu::Extent3d, samples: u32, post: bool, hdr: bool, vblur: bool, ssao: bool, fogvol: bool, post_aa: PostAaKind, water: bool) -> std::cell::Ref<'_, Targets> {
        let key = (size, render, samples, post, hdr, vblur, ssao, fogvol, post_aa, water);
        if self.targets.borrow().as_ref().is_none_or(|t| t.key != key) {
            let d = &self.device;
            let format = if hdr { HDR } else { self.color_format };
            let tex = |label, size, mips, samples, format, usage| {
                d.create_texture(&wgpu::TextureDescriptor { label: Some(label), size, mip_level_count: mips, sample_count: samples, dimension: wgpu::TextureDimension::D2, format, usage, view_formats: &[] })
            };
            let attach = wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING;
            let scaled = render != size;
            let msaa = (samples > 1).then(|| {
                let c = tex("msaa colour", render, 1, samples, format, wgpu::TextureUsages::RENDER_ATTACHMENT);
                // (sampled by the passes that read the depth, and the lens flares' `flare_vis_pass`)
                let z = tex("msaa depth", render, 1, samples, DEPTH, attach);
                (c.create_view(&Default::default()), z.create_view(&Default::default()))
            });
            // (SSAA: the caller's depth is the output size, so the scene gets its own)
            let depth = ((vblur || ssao || fogvol || water || scaled) && samples == 1).then(|| tex("blur depth", render, 1, 1, DEPTH, attach).create_view(&Default::default()));
            // the chain starts at half size and stops above 8 pixels
            let half = wgpu::Extent3d { width: (size.width / 2).max(1), height: (size.height / 2).max(1), depth_or_array_layers: 1 };
            let levels = (half.width.min(half.height) / 8).max(1).ilog2().clamp(1, BLOOM_LEVELS);
            let bloom_tex = hdr.then(|| tex("bloom", half, levels, 1, HDR, attach));
            let mips: Vec<wgpu::TextureView> = bloom_tex.iter().flat_map(|t| (0..levels).map(|m| t.create_view(&wgpu::TextureViewDescriptor { base_mip_level: m, mip_level_count: Some(1), ..Default::default() }))).collect();
            let scene = post.then(|| {
                let t = tex("post scene", size, 1, 1, format, attach | wgpu::TextureUsages::COPY_SRC);
                let view = t.create_view(&Default::default());
                let p = self.post();
                let bind = d.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("post"),
                    layout: &p.layout,
                    entries: &[
                        wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&view) },
                        wgpu::BindGroupEntry { binding: 1, resource: p.uniform.as_entire_binding() },
                        wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::TextureView(mips.first().unwrap_or(&p.no_bloom)) },
                        wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::Sampler(&p.sampler) },
                    ],
                });
                (t, view, bind)
            });
            // REMASTER SSAA: the scene at the render size and the filter down to `scene`
            let hi = scaled.then(|| {
                let a = self.aa_pipes();
                let t = tex("ssaa scene", render, 1, 1, format, attach | wgpu::TextureUsages::COPY_SRC);
                let view = t.create_view(&Default::default());
                let bind = d.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("ssaa down"),
                    layout: &a.layout,
                    entries: &[
                        wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&view) },
                        wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&a.sampler) },
                        wgpu::BindGroupEntry { binding: 2, resource: a.uniform.as_entire_binding() },
                    ],
                });
                (t, view, bind)
            });
            // REMASTER post AA: the post pass's output, in the swapchain format (13.3 A3)
            let ldr = matches!(post_aa, PostAaKind::Fxaa | PostAaKind::Smaa).then(|| {
                let a = self.aa_pipes();
                let t = tex("post aa input", size, 1, 1, self.color_format, attach);
                let view = t.create_view(&Default::default());
                let bind = d.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("fxaa"),
                    layout: &a.layout,
                    entries: &[
                        wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&view) },
                        wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&a.sampler) },
                        wgpu::BindGroupEntry { binding: 2, resource: a.uniform.as_entire_binding() },
                    ],
                });
                (t, view, bind)
            });
            // REMASTER SMAA: edge and weight targets (13.4 phase 4)
            let smaa = ldr.as_ref().filter(|_| post_aa == PostAaKind::Smaa).map(|l| {
                let s = self.smaa_pipes();
                let edges = tex("smaa edges", size, 1, 1, wgpu::TextureFormat::Rg8Unorm, attach).create_view(&Default::default());
                let weights = tex("smaa weights", size, 1, 1, wgpu::TextureFormat::Rgba8Unorm, attach).create_view(&Default::default());
                let bind = |label, e: &wgpu::TextureView, w: &wgpu::TextureView| {
                    d.create_bind_group(&wgpu::BindGroupDescriptor {
                        label: Some(label),
                        layout: &s.layout,
                        entries: &[
                            wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&l.1) },
                            wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(e) },
                            wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::TextureView(w) },
                            wgpu::BindGroupEntry { binding: 3, resource: s.uniform.as_entire_binding() },
                        ],
                    })
                };
                SmaaTargets { edges_bind: bind("smaa edges", &l.1, &l.1), weights_bind: bind("smaa weights", &edges, &l.1), blend_bind: bind("smaa blend", &l.1, &weights), edges, weights }
            });
            let bloom = scene.as_ref().filter(|_| hdr).map(|(_, view, _)| {
                let b = self.bloom_pipes();
                let group = |src: &wgpu::TextureView| {
                    d.create_bind_group(&wgpu::BindGroupDescriptor {
                        label: Some("bloom"),
                        layout: &b.layout,
                        entries: &[
                            wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(src) },
                            wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&b.sampler) },
                            wgpu::BindGroupEntry { binding: 2, resource: b.uniform.as_entire_binding() },
                        ],
                    })
                };
                let down = (0..mips.len()).map(|m| group(if m == 0 { view } else { &mips[m - 1] })).collect();
                let up = (0..mips.len().saturating_sub(1)).map(|m| group(&mips[m + 1])).collect();
                (mips, down, up)
            });
            let vblur = scene.as_ref().filter(|_| vblur).map(|_| {
                let b = self.vel_blur();
                // the car shadows are taken out of the blur (the map is built here if unused)
                let sm = self.shadow_map();
                let src = tex("blur source", render, 1, 1, format, wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST);
                let vel = tex("car velocity", render, 1, 1, HDR, attach).create_view(&Default::default());
                let vel_depth = tex("car velocity depth", render, 1, 1, DEPTH, wgpu::TextureUsages::RENDER_ATTACHMENT).create_view(&Default::default());
                let z = msaa.as_ref().map(|m| &m.1).or(depth.as_ref()).expect("velocity blur depth");
                let motion = tex("blur motion", render, 1, 1, HDR, attach).create_view(&Default::default());
                let src_view = src.create_view(&Default::default());
                let group = |label, m: &wgpu::TextureView| d.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some(label),
                    layout: &b.layouts[(samples > 1) as usize],
                    entries: &[
                        wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&src_view) },
                        wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&b.sampler) },
                        wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::TextureView(z) },
                        wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::TextureView(&vel) },
                        wgpu::BindGroupEntry { binding: 4, resource: b.uniform.as_entire_binding() },
                        wgpu::BindGroupEntry { binding: 5, resource: sm.uniform.as_entire_binding() },
                        wgpu::BindGroupEntry { binding: 6, resource: wgpu::BindingResource::TextureView(&sm.view) },
                        wgpu::BindGroupEntry { binding: 7, resource: wgpu::BindingResource::Sampler(&sm.sampler) },
                        wgpu::BindGroupEntry { binding: 8, resource: wgpu::BindingResource::TextureView(m) },
                    ],
                });
                let bind = group("velocity blur", &motion);
                let pre = group("velocity motion", &b.motion_dummy);
                VelTargets { src, vel, vel_depth, bind, motion, pre }
            });
            // REMASTER ambient occlusion: reads the depth (sample 0 when multisampled)
            let ssao = ssao.then(|| {
                let s = self.ssao_pipes();
                let half = wgpu::Extent3d { width: (render.width / 2).max(1), height: (render.height / 2).max(1), depth_or_array_layers: 1 };
                let a = tex("ssao a", half, 1, 1, SSAO_FORMAT, attach).create_view(&Default::default());
                let b = tex("ssao b", half, 1, 1, SSAO_FORMAT, attach).create_view(&Default::default());
                let z = msaa.as_ref().map(|m| &m.1).or(depth.as_ref()).expect("ssao depth");
                let entry = |binding, resource| wgpu::BindGroupEntry { binding, resource };
                let bind = |layout: &wgpu::BindGroupLayout, entries: &[wgpu::BindGroupEntry]| d.create_bind_group(&wgpu::BindGroupDescriptor { label: Some("ssao"), layout, entries });
                SsaoTargets {
                    gtao: bind(&s.gtao_layouts[(samples > 1) as usize], &[entry(0, s.uniform.as_entire_binding()), entry(1, wgpu::BindingResource::TextureView(z))]),
                    blur_h: bind(&s.blur_layout, &[entry(2, wgpu::BindingResource::TextureView(&a))]),
                    blur_v: bind(&s.blur_layout, &[entry(2, wgpu::BindingResource::TextureView(&b))]),
                    apply: bind(&s.apply_layout, &[entry(0, s.uniform.as_entire_binding()), entry(2, wgpu::BindingResource::TextureView(&a)), entry(3, wgpu::BindingResource::Sampler(&s.sampler))]),
                    a,
                    b,
                }
            });
            // REMASTER volumetric fog: reads the depth (sample 0 when multisampled)
            let vol = fogvol.then(|| {
                let z = msaa.as_ref().map(|m| &m.1).or(depth.as_ref()).expect("fog depth");
                self.volfog().apply_bind(d, z, samples)
            });
            // REMASTER TAA: history images and the passes' bind groups (the motion is the velocity
            // blur's target, built here when only TAA wants it)
            let taa = scene.as_ref().zip(vblur.as_ref()).filter(|_| post_aa == PostAaKind::Taa).map(|(sc, vt)| {
                let t = self.taa_pipes();
                let hist = [0, 1].map(|_| tex("taa history", size, 1, 1, format, attach).create_view(&Default::default()));
                let group = |label, cur: &wgpu::TextureView, h: &wgpu::TextureView| d.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some(label),
                    layout: &t.layout,
                    entries: &[
                        wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(cur) },
                        wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(h) },
                        wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::TextureView(&vt.motion) },
                        wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::Sampler(&t.sampler) },
                        wgpu::BindGroupEntry { binding: 4, resource: t.uniform.as_entire_binding() },
                    ],
                });
                TaaTargets {
                    resolve: [group("taa resolve", &sc.1, &hist[0]), group("taa resolve", &sc.1, &hist[1])],
                    sharpen: [group("taa sharpen", &hist[0], &hist[0]), group("taa sharpen", &hist[1], &hist[1])],
                    hist,
                    read: Cell::new(0),
                    valid: Cell::new(false),
                }
            });
            // REMASTER water: a copy of the opaque scene at the render size, with the depth
            let water = water.then(|| {
                let w = self.water_gpu();
                let src = tex("water scene copy", render, 1, 1, format, wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST);
                let z = msaa.as_ref().map(|m| &m.1).or(depth.as_ref()).expect("water depth");
                // REMASTER volumetric fog: the surface, drawn after the volume's apply pass, looks the volume up itself (#189)
                let fog = if fogvol { self.volfog().volume() } else { &w.no_fog };
                let bind = water_bind(d, &self.src.water_groups[(samples > 1) as usize], &src.create_view(&Default::default()), z, &w.normal, &w.foam, &w.sampler, fog);
                WaterTargets { src, bind }
            });
            *self.targets.borrow_mut() = Some(Targets { key, msaa, depth, vblur, ssao, vol, scene, hi, ldr, smaa, taa, bloom, water });
        }
        std::cell::Ref::map(self.targets.borrow(), |t| t.as_ref().unwrap())
    }

    fn post(&self) -> &Post {
        self.post.get_or_init(|| post_pipeline(&self.device, self.color_format))
    }

    fn smaa_pipes(&self) -> &Smaa {
        self.smaa.get_or_init(|| smaa_pipeline(&self.device, self.color_format))
    }

    fn taa_pipes(&self) -> &Taa {
        self.taa.get_or_init(|| taa_pipeline(&self.device, self.color_format))
    }

    /// REMASTER TAA: the history is stale (TAA was not drawn this frame)
    fn taa_reset(&self) {
        if let Some(t) = self.targets.borrow().as_ref().and_then(|t| t.taa.as_ref()) {
            t.valid.set(false);
        }
    }

    /// REMASTER: the next frame is a camera cut (the cine render's shot changes, #151). No velocity blur, TAA or
    /// volumetric-fog history carries over from the frame before, even when the two cameras stand close together
    /// (the automatic check, `CUT_DISTANCE` / `CUT_COS`, misses those).
    pub fn cut(&self) {
        *self.prev_view.borrow_mut() = None;
        self.taa_reset();
        if let Some(vf) = self.volfog.get() {
            vf.reset();
        }
    }

    fn aa_pipes(&self) -> &Aa {
        self.aa.get_or_init(|| aa_pipeline(&self.device, self.color_format))
    }

    /// REMASTER SSAA: the size the scene is drawn at, `scale` times the output, never above the
    /// device's texture limit (a 4K output may not get the full scale).
    fn render_size(&self, out: wgpu::Extent3d, scale: f32) -> wgpu::Extent3d {
        let budget = (SSAA_MAX_PIXELS / (out.width.max(1) as f64 * out.height.max(1) as f64)).sqrt() as f32;
        let k = scale.min(budget).min(self.max_dim as f32 / out.width.max(out.height).max(1) as f32).max(1.0);
        let px = |n: u32| ((n as f32 * k).round() as u32).clamp(1, self.max_dim);
        wgpu::Extent3d { width: px(out.width), height: px(out.height), depth_or_array_layers: 1 }
    }

    fn bloom_pipes(&self) -> &Bloom {
        self.bloom.get_or_init(|| bloom_pipeline(&self.device))
    }

    /// REMASTER grass shader (shader.wgsl plus grass.wgsl; the Remaster layout, no bindings of
    /// its own), built on first use
    fn grass_shader(&self) -> &wgpu::ShaderModule {
        self.grass.get_or_init(|| {
            let src = format!("{}\n{}", SHADER_SRC, include_str!("grass.wgsl"));
            self.device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("grass shader"), source: wgpu::ShaderSource::Wgsl(src.into()) })
        })
    }

    /// REMASTER water resources (`WaterGpu`), built on first use
    fn water_gpu(&self) -> &WaterGpu {
        self.water.get_or_init(|| {
            let d = &self.device;
            let modules = [false, true].map(|msaa| {
                let ty = if msaa { "texture_depth_multisampled_2d" } else { "texture_depth_2d" };
                let src = format!("{}\n{}", SHADER_SRC, include_str!("water.wgsl").replace("DEPTH_TEX", ty));
                d.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("water shader"), source: wgpu::ShaderSource::Wgsl(src.into()) })
            });
            // tuxalin's maps (assets/textures/water, MIT): 512 squared RGBA with mips
            let detail = |label, bytes: &[u8]| {
                let (w, h, px) = crate::ui::decode_rgba_png(bytes).filter(|(w, h, _)| w == h && w.is_power_of_two()).expect("water detail map");
                let chain = mips_of(&px, w);
                let tex = d.create_texture(&wgpu::TextureDescriptor {
                    label: Some(label),
                    size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
                    mip_level_count: chain.len() as u32,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: wgpu::TextureFormat::Rgba8Unorm,
                    usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                    view_formats: &[],
                });
                for (lvl, data) in chain.iter().enumerate() {
                    let size = (w >> lvl).max(1);
                    self.queue.write_texture(
                        wgpu::TexelCopyTextureInfo { texture: &tex, mip_level: lvl as u32, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
                        data,
                        wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(size * 4), rows_per_image: Some(size) },
                        wgpu::Extent3d { width: size, height: size, depth_or_array_layers: 1 },
                    );
                }
                tex.create_view(&Default::default())
            };
            let normal = detail("water normal", include_bytes!("../../../assets/textures/water/normal.png"));
            let foam = detail("water foam", include_bytes!("../../../assets/textures/water/foam.png"));
            let sampler = clamp_sampler(d, "water scene");
            let one = wgpu::Extent3d { width: 1, height: 1, depth_or_array_layers: 1 };
            let tex = |label, format| {
                d.create_texture(&wgpu::TextureDescriptor { label: Some(label), size: one, mip_level_count: 1, sample_count: 1, dimension: wgpu::TextureDimension::D2, format, usage: wgpu::TextureUsages::TEXTURE_BINDING, view_formats: &[] }).create_view(&Default::default())
            };
            let no_fog = d
                .create_texture(&wgpu::TextureDescriptor {
                    label: Some("water no fog volume"),
                    size: one,
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D3,
                    format: wgpu::TextureFormat::Rgba16Float,
                    usage: wgpu::TextureUsages::TEXTURE_BINDING,
                    view_formats: &[],
                })
                .create_view(&Default::default());
            let dummy = water_bind(d, &self.src.water_groups[0], &tex("water no scene", wgpu::TextureFormat::Rgba8Unorm), &tex("water no depth", DEPTH), &normal, &foam, &sampler, &no_fog);
            WaterGpu { modules, normal, foam, sampler, no_fog, dummy }
        })
    }

    /// REMASTER grass trail (#158): the pipelines of trail.wgsl into the R8 trail texture.
    /// `relax` subtracts the frame's step from the whole texture (blend dst - src), `stamp`
    /// writes the car footprints with max blending; both read one small uniform
    /// (`TrailUniform`: the edge map's transform, the step, the sim time).
    fn trail_gpu(&self) -> &TrailGpu {
        self.trail.get_or_init(|| {
            let d = &self.device;
            let shader = d.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("grass trail"), source: wgpu::ShaderSource::Wgsl(include_str!("trail.wgsl").into()) });
            let group = d.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("grass trail"),
                entries: &[wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                    ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None },
                    count: None,
                }],
            });
            let layout = d.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("grass trail"), bind_group_layouts: &[Some(&group)], immediate_size: 0 });
            let uniform = d.create_buffer(&wgpu::BufferDescriptor { label: Some("grass trail"), size: 32, usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
            let bind = d.create_bind_group(&wgpu::BindGroupDescriptor { label: Some("grass trail"), layout: &group, entries: &[wgpu::BindGroupEntry { binding: 0, resource: uniform.as_entire_binding() }] });
            let pipe = |label, vs, fs, op: wgpu::BlendOperation, buffers: &[Option<wgpu::VertexBufferLayout>]| {
                let blend = wgpu::BlendComponent { src_factor: wgpu::BlendFactor::One, dst_factor: wgpu::BlendFactor::One, operation: op };
                d.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                    label: Some(label),
                    layout: Some(&layout),
                    vertex: wgpu::VertexState { module: &shader, entry_point: Some(vs), buffers, compilation_options: Default::default() },
                    fragment: Some(wgpu::FragmentState {
                        module: &shader,
                        entry_point: Some(fs),
                        targets: &[Some(wgpu::ColorTargetState { format: wgpu::TextureFormat::R8Unorm, blend: Some(wgpu::BlendState { color: blend, alpha: blend }), write_mask: wgpu::ColorWrites::RED })],
                        compilation_options: Default::default(),
                    }),
                    primitive: wgpu::PrimitiveState { topology: wgpu::PrimitiveTopology::TriangleList, cull_mode: None, ..Default::default() },
                    depth_stencil: None,
                    multisample: wgpu::MultisampleState::default(),
                    multiview_mask: None,
                    cache: None,
                })
            };
            let stamp_layout = wgpu::VertexBufferLayout {
                array_stride: std::mem::size_of::<TrailStamp>() as u64,
                step_mode: wgpu::VertexStepMode::Instance,
                attributes: &wgpu::vertex_attr_array![0 => Float32x2, 1 => Float32x2, 2 => Float32x2, 3 => Float32x2, 4 => Float32x2],
            };
            TrailGpu {
                relax: pipe("grass trail relax", "vs_relax", "fs_relax", wgpu::BlendOperation::ReverseSubtract, &[]),
                stamp: pipe("grass trail stamp", "vs_stamp", "fs_stamp", wgpu::BlendOperation::Max, &[Some(stamp_layout)]),
                uniform,
                bind,
            }
        })
    }

    /// REMASTER grass trail (#158): bring the trail texture to the frame's sim time. The whole
    /// texture decays by `(now - last) / TRAIL_RELAX` (in R8 steps, the remainder carried), then
    /// the footprints newer than the last frame are stamped. A clock that went back (rewind,
    /// restart) or a fresh texture clears it and replays every footprint of `f.trail`
    /// (pre-aged by the shader), so a headless shot after seconds of sim has its trails too.
    fn trail_pass(&self, enc: &mut wgpu::CommandEncoder, s: &GpuScene, f: &Frame) {
        let Some((_, view)) = &s.grass_trail else { return };
        let Some((map, _)) = s.grass_edge else { return };
        let mut st = s.trail.borrow_mut();
        let now = f.sim_time;
        let (clear, since) = match st.time {
            Some(t) if now >= t => (false, t),
            _ => (true, f32::NEG_INFINITY),
        };
        let step = if clear { 0.0 } else { ((now - since) / TRAIL_RELAX + st.carry).min(1.0) };
        let quantised = (step * 255.0).floor() / 255.0;
        st.carry = if clear { 0.0 } else { step - quantised };
        st.time = Some(now);
        drop(st);
        let first = f.trail.partition_point(|t| t.time <= since);
        let stamps = &f.trail[first..];
        if !clear && quantised <= 0.0 && stamps.is_empty() {
            return;
        }
        let t = self.trail_gpu();
        self.queue.write_buffer(&t.uniform, 0, bytemuck::cast_slice(&[map, [quantised, now, 1.0 / TRAIL_RELAX, 0.0]]));
        let buf = (!stamps.is_empty()).then(|| self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some("grass trail stamps"), contents: bytemuck::cast_slice(stamps), usage: wgpu::BufferUsages::VERTEX }));
        let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("grass trail"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations { load: if clear { wgpu::LoadOp::Clear(wgpu::Color::BLACK) } else { wgpu::LoadOp::Load }, store: wgpu::StoreOp::Store },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_bind_group(0, &t.bind, &[]);
        if !clear && quantised > 0.0 {
            pass.set_pipeline(&t.relax);
            pass.draw(0..3, 0..1);
        }
        if let Some(b) = &buf {
            pass.set_pipeline(&t.stamp);
            pass.set_vertex_buffer(0, b.slice(..));
            pass.draw(0..6, 0..stamps.len() as u32);
        }
    }

    /// REMASTER Ultra grass blades: the placement pipeline, the record and indirect-args buffers
    /// and the draw's record group, built on first use (64 MB of records, kept alive).
    fn blade_gpu(&self) -> &BladeGpu {
        self.blades.get_or_init(|| {
            use crate::procgen::grass::{BLADE_SEGMENTS, ULTRA_MAX_BLADES};
            let d = &self.device;
            let compute = d.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some("grass blade placement"),
                layout: Some(&self.src.blade_compute_layout),
                module: self.grass_shader(),
                entry_point: Some("cs_blades"),
                compilation_options: wgpu::PipelineCompilationOptions { constants: &blade_constants(), ..Default::default() },
                cache: None,
            });
            let records = d.create_buffer(&wgpu::BufferDescriptor { label: Some("grass blade records"), size: ULTRA_MAX_BLADES as u64 * 32, usage: wgpu::BufferUsages::STORAGE, mapped_at_creation: false });
            let args = wgpu::util::DrawIndirectArgs { vertex_count: 2 * (BLADE_SEGMENTS + 1), instance_count: 0, first_vertex: 0, first_instance: 0 };
            let args = d.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some("grass blade args"), contents: args.as_bytes(), usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::INDIRECT | wgpu::BufferUsages::COPY_DST });
            let read_bind = d.create_bind_group(&wgpu::BindGroupDescriptor { label: Some("grass blades read"), layout: &self.src.blade_read_group, entries: &[wgpu::BindGroupEntry { binding: 3, resource: records.as_entire_binding() }] });
            BladeGpu { compute, draw: Default::default(), records, args, read_bind }
        })
    }

    /// REMASTER Ultra grass blades: the draw pipeline for a target (strips pulled from the
    /// records, depth tested and written, both faces, no blend).
    fn blade_pipe(&self, samples: u32, hdr: bool) -> &wgpu::RenderPipeline {
        let bg = self.blade_gpu();
        bg.draw[hdr as usize][samples.ilog2() as usize].get_or_init(|| {
            let format = if hdr { HDR } else { self.color_format };
            let constants = blade_constants();
            self.device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("grass blades"),
                layout: Some(&self.src.blade_draw_layout),
                vertex: wgpu::VertexState { module: self.grass_shader(), entry_point: Some("vs_blade"), compilation_options: wgpu::PipelineCompilationOptions { constants: &constants, ..Default::default() }, buffers: &[] },
                primitive: wgpu::PrimitiveState { topology: wgpu::PrimitiveTopology::TriangleStrip, cull_mode: None, ..Default::default() },
                depth_stencil: Some(wgpu::DepthStencilState { format: DEPTH, depth_write_enabled: Some(true), depth_compare: Some(wgpu::CompareFunction::LessEqual), stencil: Default::default(), bias: Default::default() }),
                multisample: wgpu::MultisampleState { count: samples, mask: !0, alpha_to_coverage_enabled: false },
                fragment: Some(wgpu::FragmentState {
                    module: self.grass_shader(),
                    entry_point: Some("fs_blade"),
                    compilation_options: wgpu::PipelineCompilationOptions { constants: &constants, ..Default::default() },
                    targets: &[Some(wgpu::ColorTargetState { format, blend: None, write_mask: wgpu::ColorWrites::ALL })],
                }),
                multiview_mask: None,
                cache: None,
            })
        })
    }

    /// REMASTER Ultra grass blades (18-grass.md "Ultra"): place this frame's blades. The args'
    /// instance count is zeroed in the encoder, then `cs_blades` walks the grass triangles (one
    /// workgroup row per triangle, `BLADE_SLICES` workgroups each) and appends a record per
    /// blade cell it keeps, counting into the args that the main pass draws indirectly.
    fn blade_pass(&self, enc: &mut wgpu::CommandEncoder, s: &GpuScene) {
        let Some((verts, n)) = &s.grass else { return };
        let bg = self.blade_gpu();
        let mut bind = s.blade_bind.borrow_mut();
        let bind = bind.get_or_insert_with(|| {
            self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("grass blades"),
                layout: &self.src.blade_group,
                entries: &[
                    wgpu::BindGroupEntry { binding: 1, resource: verts.as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 2, resource: bg.records.as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 3, resource: bg.args.as_entire_binding() },
                ],
            })
        });
        // instance_count = 0 (the second u32 of the args)
        enc.clear_buffer(&bg.args, 4, Some(4));
        let mut pass = enc.begin_compute_pass(&wgpu::ComputePassDescriptor { label: Some("grass blade placement"), timestamp_writes: None });
        pass.set_pipeline(&bg.compute);
        pass.set_bind_group(0, &s.bind, &[]);
        pass.set_bind_group(1, &*bind, &[]);
        pass.set_bind_group(3, &s.grass_bind, &[]);
        let tris = n / 3;
        pass.dispatch_workgroups(tris.min(BLADE_DISPATCH_W), tris.div_ceil(BLADE_DISPATCH_W), BLADE_SLICES);
    }

    fn volfog(&self) -> &crate::volfog::VolFog {
        self.volfog.get_or_init(|| {
            let m = self.shaft_map();
            crate::volfog::VolFog::new(&self.device, &m.view, &m.sampler)
        })
    }

    fn ssao_pipes(&self) -> &Ssao {
        self.ssao.get_or_init(|| ssao_pipeline(&self.device))
    }

    /// Lens-flare visibility (flarevis.wgsl, both modes; #123): after the opaque scene, each
    /// light's flare quads in `verts` are scaled by the share of depth taps round the light that
    /// the drawn scene leaves open [design choice: the original only tests the halo's cell]. The
    /// depth, not the COL: collision walls stand taller than the drawn ones and hid lamps in plain
    /// view; cars and colour-keyed texels now hide a lamp as drawn.
    fn flare_vis_pass(&self, enc: &mut wgpu::CommandEncoder, depth: &wgpu::TextureView, msaa: bool, verts: &wgpu::Buffer, lights: &[FlareLight], far: f32) {
        let (layout, pipe) = self.flare_vis[msaa as usize].get_or_init(|| {
            let d = &self.device;
            let ty = if msaa { "texture_depth_multisampled_2d" } else { "texture_depth_2d" };
            let module = d.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("flare visibility"), source: wgpu::ShaderSource::Wgsl(include_str!("flarevis.wgsl").replace("DEPTH_TEX", ty).into()) });
            let entry = |binding, ty| wgpu::BindGroupLayoutEntry { binding, visibility: wgpu::ShaderStages::COMPUTE, ty, count: None };
            let storage = |read_only| wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Storage { read_only }, has_dynamic_offset: false, min_binding_size: None };
            let layout = d.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("flare visibility"),
                entries: &[
                    entry(0, wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Depth, view_dimension: wgpu::TextureViewDimension::D2, multisampled: msaa }),
                    entry(1, wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None }),
                    entry(2, storage(true)),
                    entry(3, storage(false)),
                ],
            });
            let pl = d.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("flare visibility"), bind_group_layouts: &[Some(&layout)], immediate_size: 0 });
            let pipe = d.create_compute_pipeline(&wgpu::ComputePipelineDescriptor { label: Some("flare visibility"), layout: Some(&pl), module: &module, entry_point: Some("cs_flare_vis"), compilation_options: Default::default(), cache: None });
            (layout, pipe)
        });
        let d = &self.device;
        let params = d.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some("flare visibility"), contents: bytemuck::cast_slice(&[NEAR, far, FLARE_CLEARANCE, 0.0]), usage: wgpu::BufferUsages::UNIFORM });
        let lights_buf = d.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some("flare lights"), contents: bytemuck::cast_slice(lights), usage: wgpu::BufferUsages::STORAGE });
        let bind = d.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("flare visibility"),
            layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(depth) },
                wgpu::BindGroupEntry { binding: 1, resource: params.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 2, resource: lights_buf.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 3, resource: verts.as_entire_binding() },
            ],
        });
        let mut pass = enc.begin_compute_pass(&wgpu::ComputePassDescriptor { label: Some("flare visibility"), timestamp_writes: None });
        pass.set_pipeline(pipe);
        pass.set_bind_group(0, &bind, &[]);
        pass.dispatch_workgroups((lights.len() as u32).div_ceil(64), 1, 1);
    }

    /// REMASTER: the pipeline that multiplies the scene by the occlusion, for a sample count and
    /// output (the pass has the scene's depth attached, so the pipeline declares it, unused)
    fn ssao_apply(&self, samples: u32, hdr: bool) -> &wgpu::RenderPipeline {
        let s = self.ssao_pipes();
        s.apply[samples.ilog2() as usize][hdr as usize].get_or_init(|| {
            let format = if hdr { HDR } else { self.color_format };
            let pl = self.device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("ssao apply"), bind_group_layouts: &[Some(&s.apply_layout)], immediate_size: 0 });
            // scene * visibility
            let multiply = wgpu::BlendState {
                color: wgpu::BlendComponent { src_factor: wgpu::BlendFactor::Zero, dst_factor: wgpu::BlendFactor::Src, operation: wgpu::BlendOperation::Add },
                alpha: wgpu::BlendComponent { src_factor: wgpu::BlendFactor::Zero, dst_factor: wgpu::BlendFactor::One, operation: wgpu::BlendOperation::Add },
            };
            self.device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("ssao apply"),
                layout: Some(&pl),
                vertex: wgpu::VertexState { module: &s.shaders[0], entry_point: Some("vs_full"), compilation_options: Default::default(), buffers: &[] },
                primitive: wgpu::PrimitiveState { cull_mode: None, ..Default::default() },
                depth_stencil: Some(wgpu::DepthStencilState { format: DEPTH, depth_write_enabled: Some(false), depth_compare: Some(wgpu::CompareFunction::Always), stencil: Default::default(), bias: Default::default() }),
                multisample: wgpu::MultisampleState { count: samples, ..Default::default() },
                fragment: Some(wgpu::FragmentState {
                    module: &s.shaders[0],
                    entry_point: Some("fs_apply"),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState { format, blend: Some(multiply), write_mask: wgpu::ColorWrites::ALL })],
                }),
                multiview_mask: None,
                cache: None,
            })
        })
    }

    fn vel_blur(&self) -> &VelBlur {
        self.vel_blur.get_or_init(|| vel_blur_pipeline(&self.device))
    }
}

/// REMASTER sky dome (09-remaster 9.32): the sun's azimuth and elevation in the HDRI when this view
/// draws the dome (Remaster, the option on, and the track has `remaster/sky/trackNN.exr`).
fn dome_sun(v: &View, s: &GpuScene) -> Option<crate::sky::Dome> {
    s.dome.filter(|_| v.mode == RenderMode::Remaster && v.remaster.sky_hdri)
}

/// REMASTER menu car preview (`Frame::studio`) [design choice]: the Car screen mockup's turntable light
/// (`reference_materials/car-overhaul/assets/car.js` `TT.draw`): a neutral white key light fixed to the
/// camera, towards the light (-0.42, 0.78, -0.46) in the view's right / up / forward (upper left, on the
/// camera's side of the car), with its diffuse gain and a grey fill. Gamma-space gains like the Classic
/// light model. The env cube is linear light: grey sides, a bright top, a dark floor.
const STUDIO_KEY_DIR: [f32; 3] = [-0.42, 0.78, -0.46];
const STUDIO_KEY: f32 = 0.85;
const STUDIO_FILL: f32 = 0.52;
const STUDIO_SPEC: f32 = 1.0;
const STUDIO_SIDE: f64 = 0.16;
const STUDIO_TOP: f64 = 1.1;
const STUDIO_FLOOR: f64 = 0.03;

/// `look` with the studio's light for a camera: neutral key and fill, no fog.
fn studio_look(look: &mut crate::scene::Look, cam: &Camera) {
    let [x, y, z] = STUDIO_KEY_DIR;
    look.sun_dir = (cam.right() * x + cam.up() * y + cam.forward() * z).normalize().to_array();
    (look.sun, look.object_ambient, look.specular) = ([STUDIO_KEY; 3], [STUDIO_FILL; 3], STUDIO_SPEC);
}

/// The track's lighting; with a dome and `SkyTune::sun` the sun direction is the one of the
/// HDRI's sun: the track's azimuth (plus the dome's turn, which carries the visible sun) at the
/// image's elevation [design choice]. The pre-lit world keeps its baked light.
fn sky_look(v: &View, s: &GpuScene) -> crate::scene::Look {
    look_for_dome(v, dome_sun(v, s), s.look)
}

/// `sky_look` for a scene that is not on the GPU (motorview's probe debug layer and
/// `--probes-stats`, which must bake for the sun the GPU volume is baked for, #198).
pub fn view_look(v: &View, s: &crate::scene::Scene) -> crate::scene::Look {
    let dome = s.hdri.as_ref().map(|h| h.dome()).filter(|_| v.mode == RenderMode::Remaster && v.remaster.sky_hdri);
    look_for_dome(v, dome, s.look)
}

fn look_for_dome(v: &View, dome: Option<crate::sky::Dome>, mut l: crate::scene::Look) -> crate::scene::Look {
    if let Some(d) = dome.filter(|d| d.follow_sun && v.remaster.sky_tune.sun) {
        let (el, az) = (d.sun_elevation, l.sun_dir[0].atan2(-l.sun_dir[2]) + (v.remaster.sky_tune.yaw + d.yaw).to_radians());
        l.sun_dir = [el.cos() * az.sin(), el.sin(), -el.cos() * az.cos()];
    }
    l
}

/// REMASTER sky ambient (09-remaster 9.32) [design choice]: the HDRI's irradiance terms
/// (`sky::Hdri::sh`) scaled so their mean over the sphere has the luminance of the track's
/// ObjectAmbient in linear light: the tint (kept `SkyTune::ambient_tint`, the rest is mixed towards
/// grey) and the up/down contrast come from the image, the brightness stays as tuned. All zero (the shader keeps `ambient`) without a dome or with the
/// option off.
fn sky_sh(v: &View, s: &GpuScene, l: &crate::scene::Look) -> [[f32; 4]; 9] {
    let mut out = [[0.0; 4]; 9];
    let Some(d) = dome_sun(v, s).filter(|_| v.remaster.sky_tune.ambient) else {
        return out;
    };
    let luma = |c: [f32; 3]| 0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2];
    let target = luma(l.object_ambient.map(|c| c.max(0.0).powf(GAMMA)));
    let mean = luma(d.sh[0]);
    if mean <= 1e-6 || target <= 0.0 {
        return out;
    }
    let k = target / mean;
    let tint = v.remaster.sky_tune.ambient_tint.clamp(0.0, 1.0);
    for (o, c) in out.iter_mut().zip(d.sh) {
        // the luminance of each term is kept, so the mean and the contrast stay
        let grey = luma(c);
        let m = |i: usize| (grey + tint * (c[i] - grey)) * k;
        *o = [m(0), m(1), m(2), 0.0];
    }
    out[0][3] = 1.0;
    out
}

/// REMASTER fog colour in linear light (17-fog-rework.md): the sky's horizon (the dome's ring at its
/// exposure when this view draws the dome, else the band's bottom rows), CueColour or an explicit
/// colour, times brightness and tint. Capped so a bright HDRI horizon does not wash the frame.
fn fog_colour(v: &View, s: &GpuScene, l: &crate::scene::Look, t: &FogTune) -> [f32; 3] {
    let lin = |c: [f32; 3]| c.map(|x| x.max(0.0).powf(GAMMA));
    let base = match t.colour {
        FogColour::Sky => match dome_sun(v, s) {
            Some(d) => d.horizon.map(|c| c * (v.remaster.sky_tune.exposure + d.exposure).exp2()),
            None => lin(l.horizon_band),
        },
        FogColour::Track => lin(l.fog_colour),
        FogColour::Rgb(c) => lin(c),
    };
    std::array::from_fn(|i| (base[i] * t.brightness * t.tint[i]).clamp(0.0, FOG_COLOUR_MAX))
}

/// CHEAT the cloud-ceiling looks (`0x439e84`, `0x443570`): lemmy / ace "Hell mode" (#316) and ramlosa / h2o "Underwater"
/// (#317). Both force the fog, the sky gradient and the brightness every frame and draw the same ceiling sky.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Ceiling {
    /// the red frame or a lightning one
    Hell { flash: bool },
    Underwater,
}

impl Ceiling {
    fn of(f: &Frame) -> Option<Ceiling> {
        f.hell.map(|flash| Ceiling::Hell { flash }).or(f.underwater.map(|_| Ceiling::Underwater))
    }

    /// sky top and bottom, 0..255
    fn sky(self) -> ([f32; 3], [f32; 3]) {
        use motor_sim::cheats::{HELL_SKY, HELL_SKY_FLASH, UNDERWATER_SKY};
        match self {
            Ceiling::Hell { flash: false } => HELL_SKY,
            Ceiling::Hell { flash: true } => HELL_SKY_FLASH,
            Ceiling::Underwater => UNDERWATER_SKY,
        }
    }

    /// CueColour, 0..1: black, white in a flash, teal under water
    fn fog(self) -> [f32; 3] {
        match self {
            Ceiling::Hell { flash } => [if flash { 1.0 } else { 0.0 }; 3],
            Ceiling::Underwater => motor_sim::cheats::UNDERWATER_FOG.map(|c| c / 255.0),
        }
    }

    /// the ceiling texture's scroll per metre of eye position (0.005, 0.001 under water: `0x443a49`)
    fn scroll(self) -> f32 {
        match self {
            Ceiling::Hell { .. } => motor_sim::cheats::HELL_SCROLL,
            Ceiling::Underwater => motor_sim::cheats::UNDERWATER_SCROLL,
        }
    }

    /// the colour gain over the baked brightness: `3 / brightness` on a lightning frame (both brightnesses 3.0), else 1
    fn gain(self) -> [f32; 4] {
        use motor_sim::cheats::{HELL_BRIGHTNESS, HELL_FLASH_BRIGHTNESS};
        match self {
            Ceiling::Hell { flash: true } => [HELL_FLASH_BRIGHTNESS / HELL_BRIGHTNESS[0], HELL_FLASH_BRIGHTNESS / HELL_BRIGHTNESS[1], HELL_FLASH_BRIGHTNESS / HELL_BRIGHTNESS[2], 0.0],
            _ => [1.0, 1.0, 1.0, 0.0],
        }
    }
}

/// CHEAT the ceiling looks: the fog and sky gradient are the cheat's, so everything that reads the look (the env
/// probe's sky, Remaster fog) follows.
fn ceiling_look(look: &mut crate::scene::Look, c: Ceiling) {
    let (top, bottom) = c.sky();
    look.sky_top = top.map(|c| c / 255.0);
    look.sky_bottom = bottom.map(|c| c / 255.0);
    look.fog_colour = c.fog();
    look.horizon_band = c.fog();
}

/// CHEAT the ceiling looks (`0x439e84`, `0x443570`): the frame's uniforms. The scene was baked with the cheat's
/// brightness (`scene::load_progress`), so the gain is `Ceiling::gain` and the user's Brightness option does not count.
/// The fog is the cheat's without the sun glow, the HDRI ambient is off, and the sky is the two quads of `0x443570`: the
/// env-map texture on a gradient above 0.6 of the height, plain below. Positions are the original's screen-space ones
/// (rhw 10 at the top, 1.11 at the bottom of the textured quad, so the texture reads as a ceiling plane); the texture
/// coordinates scroll with the eye (`Ceiling::scroll` per metre).
fn ceiling_globals(g: &mut Globals, c: Ceiling, cam: &Camera, v: &View, mirror: bool) {
    g.gain = c.gain();
    g.params[2] = 1.0;
    let fog = c.fog();
    g.fog = [fog[0], fog[1], fog[2], 1.0];
    // the Remaster fog in linear light, without the fog tune's brightness and tint
    let lin = fog.map(|x| x.powf(GAMMA));
    g.fog4 = [lin[0], lin[1], lin[2], g.fog4[3]];
    g.fog2[3] = 0.0;
    g.sky_sh = [[0.0; 4]; 9];
    // the original works in game coordinates (z forward); ours are z negated
    let fov = 2.0 * ((v.fov_x.to_radians() / 2.0).tan() / cam.zoom).atan();
    let fwd = cam.forward();
    let d = glam::Vec2::new(fwd.x, -fwd.z).normalize_or_zero();
    let rot = |a: f32| {
        let (sin, cos) = a.sin_cos();
        glam::Vec2::new(cos * d.x + sin * d.y, -sin * d.x + cos * d.y) * 1.5
    };
    let (left, right) = (rot(-fov / 2.0), rot(fov / 2.0));
    let (ox, oz) = ((cam.pos.x * c.scroll()) % 1.0, (-cam.pos.z * c.scroll()) % 1.0);
    let (top_v, bottom_v) = (oz + 0.1 * left.y, oz + 0.9 * right.y);
    let x = if mirror { -1.0 } else { 1.0 };
    // the textured quad over 0..0.6 of the height (ndc y 1 .. -0.2), then the plain one (-0.2 .. -1)
    let (near, far) = (1.0 / 1.111_111_2, 1.0 / 10.0);
    let rows = [
        ([-x, 1.0, far, 1.0], [ox + 0.1 * left.x, top_v, 0.0, 0.0]),
        ([x, 1.0, far, 1.0], [ox + 0.1 * right.x, top_v, 0.0, 0.0]),
        ([x, -0.2, near, 1.0], [ox + 0.9 * right.x, bottom_v, 1.0, 0.0]),
        ([-x, -0.2, near, 1.0], [ox + 0.9 * left.x, bottom_v, 1.0, 0.0]),
        ([-x, -0.2, 1.0, 0.0], [0.0, 0.0, 1.0, 0.0]),
        ([x, -0.2, 1.0, 0.0], [0.0, 0.0, 1.0, 0.0]),
        ([x, -1.0, 1.0, 0.0], [0.0, 0.0, 1.0, 0.0]),
        ([-x, -1.0, 1.0, 0.0], [0.0, 0.0, 1.0, 0.0]),
    ];
    for (k, (a, b)) in rows.into_iter().enumerate() {
        (g.hell[2 * k], g.hell[2 * k + 1]) = (a, b);
    }
}

/// The scene pipelines (sky, meshes, shadows, halos, particles, lines, motion blur) for one
/// sample count. `remaster` picks the REMASTER shader entry points; its multisampled mesh
/// pipelines turn colour keys into coverage.
/// The world without the triangles tagged as water (`Scene::water_hide`), or None when none is.
/// The water's generated bed (`bed`) goes in their place (and an area's bed, which hides nothing,
/// is added to the whole world).
fn dry_world(tris: &[Vertex], hide: &[bool], bed: &[Vertex]) -> Option<Vec<Vertex>> {
    if hide.is_empty() && bed.is_empty() {
        return None;
    }
    Some(tris.chunks(3).enumerate().filter(|(i, _)| !hide.get(*i).copied().unwrap_or(false)).flat_map(|(_, t)| t.iter().copied()).chain(bed.iter().copied()).collect())
}

/// REMASTER Ultra grass blades: workgroups per grass triangle (grass.wgsl `BLADE_SLICES`, each
/// takes every n-th blade cell of the triangle, so a big meadow polygon spreads over the GPU)
/// and the widest dispatch row (the per-dimension limit)
const BLADE_SLICES: u32 = 16;
const BLADE_GROUP: u32 = 64;
const BLADE_DISPATCH_W: u32 = 65535;

/// REMASTER Ultra grass blades: the pipeline constants of grass.wgsl's blade entry points
fn blade_constants() -> [(&'static str, f64); 3] {
    [("BLADE_MAX", crate::procgen::grass::ULTRA_MAX_BLADES as f64), ("BLADE_SLICES", BLADE_SLICES as f64), ("BLADE_GROUP", BLADE_GROUP as f64)]
}

/// REMASTER grass: the shells' vertex buffer, a storage buffer too (the Ultra blade placement
/// walks its triangles)
fn grass_buffer(device: &wgpu::Device, verts: &[Vertex]) -> wgpu::Buffer {
    device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some("grass"), contents: bytemuck::cast_slice(verts), usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::STORAGE })
}

/// REMASTER grass: a body's shell count at a level (0 = none drawn).
fn grass_shells(shells: f32, level: GrassLevel) -> u32 {
    if level == GrassLevel::Off {
        return 0;
    }
    ((shells * level.factor()).round() as u32).max(1)
}

/// The `Vertex` attributes of the scene pipelines (shader.wgsl's vertex inputs).
const SCENE_ATTRS: [wgpu::VertexAttribute; 6] = wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x2, 2 => Float32x4, 3 => Float32, 4 => Float32x3, 5 => Float32];

/// `water`: REMASTER the water shader and layout for this sample count (the water pipeline's own).
/// `grass`: REMASTER the grass shader (the Remaster layout plus the edge map group).
#[allow(clippy::too_many_arguments)]
fn scene_pipes(device: &wgpu::Device, src: &PipeSrc, color_format: wgpu::TextureFormat, samples: u32, remaster: bool, water: Option<(&wgpu::ShaderModule, &wgpu::PipelineLayout)>, grass: Option<&wgpu::ShaderModule>, alpha_out: bool) -> Pipes {
    let (shader, pl) = (&src.shader, if remaster { &src.remaster_layout } else { &src.layout });
    let vbuf = wgpu::VertexBufferLayout { array_stride: std::mem::size_of::<Vertex>() as u64, step_mode: wgpu::VertexStepMode::Vertex, attributes: &SCENE_ATTRS };
    // REMASTER menu car preview (`alpha_out`): its offscreen frame's alpha is the premultiplied composite's coverage, so
    // the additive passes (halos, sparks, hologram) add light and leave it alone
    let additive_alpha = if alpha_out { wgpu::BlendComponent { src_factor: wgpu::BlendFactor::Zero, dst_factor: wgpu::BlendFactor::One, operation: wgpu::BlendOperation::Add } } else { wgpu::BlendComponent::OVER };
    let pipe = |topology, label, kind: Pass| {
        let buffers = [Some(vbuf.clone())];
        let sky = matches!(kind, Pass::Sky | Pass::SkyCube | Pass::SkyDome | Pass::SkyCubeDome | Pass::HellSky);
        let overlay = matches!(kind, Pass::Shadow | Pass::Skid | Pass::SkidFlat | Pass::Halo | Pass::Smoke | Pass::Spark | Pass::LightWall | Pass::Xray | Pass::WireXray | Pass::Holo | Pass::HoloFill | Pass::Ghost | Pass::Water);
        // (the menu car preview's set keeps the plain colour key: its alpha is the premultiplied composite's, not a coverage)
        let a2c = remaster && samples > 1 && !alpha_out && matches!(kind, Pass::Mesh(_) | Pass::Grass);
        let wire = matches!(kind, Pass::Wire | Pass::WireFill | Pass::WireXray);
        let modern = remaster && !sky && !wire && !matches!(kind, Pass::Halo | Pass::Smoke | Pass::Spark | Pass::LightWall | Pass::Holo | Pass::HoloFill | Pass::Water);
        let constants = [("ALPHA_TO_COVERAGE", a2c as u32 as f64)];
        let (shader, pl) = match (water, grass) {
            (Some(w), _) if kind == Pass::Water => w,
            (_, Some(gs)) if kind == Pass::Grass => (gs, &src.grass_layout),
            _ => (shader, pl),
        };
        device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some(label),
            layout: Some(pl),
            vertex: wgpu::VertexState {
                module: shader,
                entry_point: Some(if kind == Pass::HellSky { "vs_hell_sky" } else if sky { "vs_sky" } else if wire { "vs_flat" } else if matches!(kind, Pass::Skid | Pass::SkidFlat | Pass::LightWall) { "vs_skid" } else if kind == Pass::Water { "vs_water" } else if kind == Pass::Grass { "vs_grass" } else if remaster { "vs_remaster" } else { "vs_main" }),
                compilation_options: Default::default(),
                buffers: if sky { &[] } else { &buffers },
            },
            primitive: match kind {
                Pass::Ghost => wgpu::PrimitiveState { topology, front_face: wgpu::FrontFace::Ccw, cull_mode: Some(wgpu::Face::Back), ..Default::default() },
                Pass::Mesh(mirror) => wgpu::PrimitiveState {
                    topology,
                    front_face: if mirror { wgpu::FrontFace::Cw } else { wgpu::FrontFace::Ccw },
                    cull_mode: Some(wgpu::Face::Back),
                    ..Default::default()
                },
                _ => wgpu::PrimitiveState { topology, cull_mode: None, ..Default::default() },
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: DEPTH,
                depth_write_enabled: Some(!sky && !overlay),
                depth_compare: Some(if sky || matches!(kind, Pass::Xray | Pass::WireXray | Pass::Holo | Pass::HoloFill) { wgpu::CompareFunction::Always } else { wgpu::CompareFunction::LessEqual }),
                stencil: Default::default(),
                // shadows are drawn with a z bias (0x438308 swaps in 0x4ec080/84)
                bias: if matches!(kind, Pass::Shadow | Pass::Skid | Pass::SkidFlat) { wgpu::DepthBiasState { constant: -8, slope_scale: -1.0, clamp: 0.0 } } else if kind == Pass::WireFill { wgpu::DepthBiasState { constant: 2, slope_scale: 1.0, clamp: 0.0 } } else { Default::default() },
            }),
            multisample: wgpu::MultisampleState { count: samples, mask: !0, alpha_to_coverage_enabled: a2c },
            fragment: Some(wgpu::FragmentState {
                module: shader,
                entry_point: Some(match kind {
                    Pass::Sky => "fs_sky",
                    Pass::SkyCube => "fs_sky_cube",
                    Pass::SkyDome => "fs_sky_dome",
                    Pass::SkyCubeDome => "fs_sky_cube_dome",
                    Pass::HellSky => "fs_hell_sky",
                    // REMASTER: the analytic halo shape (09-remaster 9.11)
                    Pass::Halo if remaster => "fs_halo_remaster",
                    Pass::Halo => "fs_halo",
                    Pass::Smoke => "fs_smoke",
                    Pass::Spark => "fs_spark",
                    Pass::LightWall => "fs_light_wall",
                    Pass::Holo => "fs_holo",
                    Pass::Water => "fs_water",
                    Pass::Grass => "fs_grass",
                    Pass::Skid => "fs_skid",
                    Pass::SkidFlat => "fs_skid_flat",
                    Pass::HoloFill => "fs_holo_fill",
                    Pass::Wire | Pass::WireFill | Pass::WireXray => "fs_flat",
                    _ if modern => "fs_remaster",
                    _ => "fs_main",
                }),
                compilation_options: wgpu::PipelineCompilationOptions { constants: if modern { &constants } else { &[] }, ..Default::default() },
                targets: &[Some(wgpu::ColorTargetState {
                    format: color_format,
                    blend: match kind {
                        Pass::Shadow | Pass::Skid | Pass::Water => Some(wgpu::BlendState::ALPHA_BLENDING),
                        // CLASSIC marks multiply the frame by their colour (0x442911)
                        Pass::SkidFlat => Some(wgpu::BlendState {
                            color: wgpu::BlendComponent { src_factor: wgpu::BlendFactor::Zero, dst_factor: wgpu::BlendFactor::Src, operation: wgpu::BlendOperation::Add },
                            alpha: wgpu::BlendComponent { src_factor: wgpu::BlendFactor::Zero, dst_factor: wgpu::BlendFactor::One, operation: wgpu::BlendOperation::Add },
                        }),
                        // REMASTER smoke: ordinary alpha blending, back to front
                        Pass::Smoke => Some(wgpu::BlendState::ALPHA_BLENDING),
                        Pass::Ghost => Some(wgpu::BlendState {
                            color: wgpu::BlendComponent { src_factor: wgpu::BlendFactor::Constant, dst_factor: wgpu::BlendFactor::OneMinusConstant, operation: wgpu::BlendOperation::Add },
                            alpha: wgpu::BlendComponent::OVER,
                        }),
                        // the hologram only adds light
                        Pass::Holo | Pass::HoloFill => Some(wgpu::BlendState {
                            color: wgpu::BlendComponent { src_factor: wgpu::BlendFactor::One, dst_factor: wgpu::BlendFactor::One, operation: wgpu::BlendOperation::Add },
                            alpha: additive_alpha,
                        }),
                        // CLASSIC halos add light: textured ONE / ONE, flat, no fog, no z write (0x4427af..0x44283e) [O]; so do sparks [I]
                        Pass::Halo | Pass::Spark | Pass::LightWall => Some(wgpu::BlendState {
                            color: wgpu::BlendComponent { src_factor: wgpu::BlendFactor::One, dst_factor: wgpu::BlendFactor::One, operation: wgpu::BlendOperation::Add },
                            alpha: additive_alpha,
                        }),
                        _ => None,
                    },
                    // the coverage alpha is not a colour: the target keeps its cleared alpha
                    write_mask: if a2c { wgpu::ColorWrites::COLOR } else { wgpu::ColorWrites::ALL },
                })],
            }),
            multiview_mask: None,
            cache: None,
        })
    };
    Pipes {
        water: water.map(|_| pipe(wgpu::PrimitiveTopology::TriangleList, "water", Pass::Water)),
        grass: grass.map(|_| pipe(wgpu::PrimitiveTopology::TriangleList, "grass", Pass::Grass)),
        tri: pipe(wgpu::PrimitiveTopology::TriangleList, "tris", Pass::Mesh(false)),
        tri_mirror: pipe(wgpu::PrimitiveTopology::TriangleList, "tris mirrored", Pass::Mesh(true)),
        line: pipe(wgpu::PrimitiveTopology::LineList, "lines", Pass::Opaque),
        sky: pipe(wgpu::PrimitiveTopology::TriangleList, "sky", Pass::Sky),
        hell_sky: pipe(wgpu::PrimitiveTopology::TriangleList, "hell sky", Pass::HellSky),
        sky_cube: pipe(wgpu::PrimitiveTopology::TriangleList, "sky cube", Pass::SkyCube),
        sky_dome: remaster.then(|| pipe(wgpu::PrimitiveTopology::TriangleList, "sky dome", Pass::SkyDome)),
        sky_cube_dome: remaster.then(|| pipe(wgpu::PrimitiveTopology::TriangleList, "sky cube dome", Pass::SkyCubeDome)),
        shadow: pipe(wgpu::PrimitiveTopology::TriangleList, "shadows", Pass::Shadow),
        skid_flat: pipe(wgpu::PrimitiveTopology::TriangleList, "skid marks flat", Pass::SkidFlat),
        skid: remaster.then(|| pipe(wgpu::PrimitiveTopology::TriangleList, "skid marks", Pass::Skid)),
        light_wall: remaster.then(|| pipe(wgpu::PrimitiveTopology::TriangleList, "light walls", Pass::LightWall)),
        halo: pipe(wgpu::PrimitiveTopology::TriangleList, "halos", Pass::Halo),
        smoke: pipe(wgpu::PrimitiveTopology::TriangleList, "smoke", Pass::Smoke),
        spark: pipe(wgpu::PrimitiveTopology::TriangleList, "sparks", Pass::Spark),
        xray: pipe(wgpu::PrimitiveTopology::LineList, "debug x-ray lines", Pass::Xray),
        xray_solid: pipe(wgpu::PrimitiveTopology::TriangleList, "debug x-ray solids", Pass::Xray),
        wire: pipe(wgpu::PrimitiveTopology::LineList, "wire edges", Pass::Wire),
        wire_fill: pipe(wgpu::PrimitiveTopology::TriangleList, "wire fill", Pass::WireFill),
        wire_xray: pipe(wgpu::PrimitiveTopology::LineList, "wire x-ray edges", Pass::WireXray),
        holo: pipe(wgpu::PrimitiveTopology::LineList, "hologram edges", Pass::Holo),
        holo_fill: pipe(wgpu::PrimitiveTopology::TriangleList, "hologram shell", Pass::HoloFill),
        // REMASTER draws the ghost as a hologram instead
        ghost: (!remaster).then(|| pipe(wgpu::PrimitiveTopology::TriangleList, "ghost car", Pass::Ghost)),
        blur: blur_pipeline(device, src, color_format, samples),
    }
}

/// REMASTER car shadows: a depth map of the cars seen from the sun (09-remaster 9.15).
struct ShadowMap {
    view: wgpu::TextureView,
    uniform: wgpu::Buffer,
    /// group 2 of the Remaster pipelines (reads the map)
    read: wgpu::BindGroup,
    /// group 2 of the caster pass (the uniform only: the map is its depth target)
    write: wgpu::BindGroup,
    pipe: wgpu::RenderPipeline,
    /// the comparison sampler of `read` (the velocity blur's shadow lookups too)
    sampler: wgpu::Sampler,
}

/// REMASTER lamp shadows: the depth atlas of the cars seen from the street lamps.
struct SpotShadows {
    uniform: wgpu::Buffer,
    /// group 2 of the Remaster pipelines: sun map + this atlas
    read: wgpu::BindGroup,
    /// per layer: depth target, its `ShadowU` buffer and bind group (group 2 of the caster pass)
    casters: Vec<(wgpu::TextureView, wgpu::Buffer, wgpu::BindGroup)>,
    pipe: wgpu::RenderPipeline,
}

/// shader.wgsl `SpotU`: per layer the world -> clip matrix, (lamp position, range) and
/// (fade, bias in metres, 1 / map size, cosine of the shadow cone's half angle) and (tan per texel, PCF radius);
/// `light.w` is the shadow range.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct SpotU {
    vp: [[f32; 16]; SPOT_LAYERS],
    light: [[f32; 4]; SPOT_LAYERS],
    p: [[f32; 4]; SPOT_LAYERS],
    q: [[f32; 4]; SPOT_LAYERS],
}

/// REMASTER volumetric fog sun shafts [design choice, 17-fog-rework.md "Sun shafts"]: a depth map of the
/// world, the cars and the scene objects seen from the real sun, fitted to the view frustum. Not the cars'
/// map (`ShadowMap`): that one is small, fitted to the cars and looks straight down on dim tracks.
struct ShaftMap {
    view: wgpu::TextureView,
    uniform: wgpu::Buffer,
    /// group 2 of the caster pass
    write: wgpu::BindGroup,
    pipe: wgpu::RenderPipeline,
    /// comparison sampler of the froxel injection
    sampler: wgpu::Sampler,
}

/// Edge of the fog shadow map (texels).
const SHAFT_SIZE: u32 = 1024;
/// How far from the camera the shafts reach (m); further froxels are lit.
const SHAFT_RANGE: f32 = 150.0;
/// Depth bias of the fog shadow lookup (m).
const SHAFT_BIAS: f32 = 0.6;
/// The map's window grows in steps of this many metres, so it does not breathe as the camera turns.
const SHAFT_STEP: f32 = 8.0;

/// The sun's orthographic view of the view frustum between `NEAR` and `range` (`corners`: its 8 points) and of the
/// casters up to `world` (min, max): world -> light clip space and the depth range in metres. `sun` points towards the sun.
/// The window snaps to whole texels in light space. `None` for a sun under the horizon.
fn shaft_matrix(corners: &[Vec3; 8], world: (Vec3, Vec3), sun: Vec3) -> Option<(Mat4, f32)> {
    if sun.y < 0.05 {
        return None;
    }
    let sun = sun.normalize();
    let up = if sun.y.abs() > 0.95 { Vec3::Z } else { Vec3::Y };
    let v0 = glam::camera::rh::view::look_to_mat4(Vec3::ZERO, -sun, up);
    let bounds = |pts: &mut dyn Iterator<Item = Vec3>| pts.map(|p| v0.transform_point3(p)).fold((Vec3::splat(f32::MAX), Vec3::splat(f32::MIN)), |(lo, hi), p| (lo.min(p), hi.max(p)));
    let (lo, hi) = bounds(&mut corners.iter().copied());
    let (w_lo, w_hi) = world;
    let (_, box_hi) = bounds(&mut (0..8).map(|i| Vec3::new(if i & 1 == 0 { w_lo.x } else { w_hi.x }, if i & 2 == 0 { w_lo.y } else { w_hi.y }, if i & 4 == 0 { w_lo.z } else { w_hi.z })));
    // x, y: the frustum's box, a square of whole steps; z: from the casters nearest the sun to the frustum's far side
    let half = (((hi.x - lo.x).max(hi.y - lo.y) * 0.5 + 1.0) / SHAFT_STEP).ceil() * SHAFT_STEP;
    let texel = 2.0 * half / SHAFT_SIZE as f32;
    let (cx, cy) = (((lo.x + hi.x) * 0.5 / texel).round() * texel, ((lo.y + hi.y) * 0.5 / texel).round() * texel);
    // view space looks down -z: a larger z is nearer the sun
    let near_z = box_hi.z.max(hi.z) + 1.0;
    let depth = near_z - lo.z + 1.0;
    let view = Mat4::from_translation(Vec3::new(-cx, -cy, -near_z)) * v0;
    let ortho = Mat4::from_cols(
        glam::Vec4::new(1.0 / half, 0.0, 0.0, 0.0),
        glam::Vec4::new(0.0, 1.0 / half, 0.0, 0.0),
        glam::Vec4::new(0.0, 0.0, -1.0 / depth, 0.0),
        glam::Vec4::new(0.0, 0.0, 0.0, 1.0),
    );
    Some((ortho * view, depth))
}

/// The sun's orthographic view of the cars near the camera: world -> light clip space, the
/// depth range in metres. `None` without cars in range. The window is snapped to whole texels
/// in light space so the edges do not shimmer as the cars move.
fn shadow_matrix(cars: &[(usize, Mat4)], cam: Vec3, sun: Vec3) -> Option<(Mat4, f32)> {
    let pts: Vec<Vec3> = cars
        .iter()
        .filter(|(mesh, _)| mesh % CAR_MESHES == 0)
        .map(|(_, m)| m.w_axis.truncate())
        .filter(|p| p.distance(cam) < SHADOW_RANGE)
        .collect();
    if pts.is_empty() || sun.y < 0.05 {
        return None;
    }
    let (lo, hi) = pts.iter().fold((Vec3::splat(f32::MAX), Vec3::splat(f32::MIN)), |(lo, hi), &p| (lo.min(p), hi.max(p)));
    let centre = (lo + hi) * 0.5;
    let r = ((hi - lo).length() * 0.5 + SHADOW_MARGIN).max(SHADOW_MIN_RADIUS);
    let sun = sun.normalize();
    let up = if sun.y.abs() > 0.95 { Vec3::Z } else { Vec3::Y };
    let v0 = glam::camera::rh::view::look_to_mat4(Vec3::ZERO, -sun, up);
    let texel = 2.0 * r / SHADOW_SIZE as f32;
    let c = v0.transform_point3(centre);
    let depth = 2.0 * r + 16.0;
    // light space: x, y snapped to the texel grid, z = 0 at the light's near plane
    let offset = Vec3::new((c.x / texel).round() * texel, (c.y / texel).round() * texel, c.z + r + 8.0);
    let view = Mat4::from_translation(-offset) * v0;
    let ortho = Mat4::from_cols(
        glam::Vec4::new(1.0 / r, 0.0, 0.0, 0.0),
        glam::Vec4::new(0.0, 1.0 / r, 0.0, 0.0),
        glam::Vec4::new(0.0, 0.0, -1.0 / depth, 0.0),
        glam::Vec4::new(0.0, 0.0, 0.0, 1.0),
    );
    Some((ortho * view, depth))
}

impl Gpu {
    /// REMASTER car shadows: the map, built on first use.
    fn shadow_map(&self) -> &ShadowMap {
        self.shadow.get_or_init(|| {
            let d = &self.device;
            let tex = d.create_texture(&wgpu::TextureDescriptor {
                label: Some("car shadow map"),
                size: wgpu::Extent3d { width: SHADOW_SIZE, height: SHADOW_SIZE, depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: DEPTH,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            });
            let view = tex.create_view(&Default::default());
            let uniform = d.create_buffer(&wgpu::BufferDescriptor {
                label: Some("car shadow uniform"),
                size: 80,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            let sampler = d.create_sampler(&wgpu::SamplerDescriptor {
                mag_filter: wgpu::FilterMode::Linear,
                min_filter: wgpu::FilterMode::Linear,
                compare: Some(wgpu::CompareFunction::LessEqual),
                ..Default::default()
            });
            let entry = |binding, resource| wgpu::BindGroupEntry { binding, resource };
            // the lamp atlas as the Remaster layouts see it until a lamp gets a map: 1 texel
            let (dummy, dummy_u) = self.spot_dummy();
            let read = self.shadow_read_group("car shadow map", &uniform, &view, &sampler, &dummy, &dummy_u);
            let write = d.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("car shadow caster"),
                layout: &self.shadow_layouts.1,
                entries: &[entry(0, uniform.as_entire_binding())],
            });
            let pipe = self.caster_pipe("car shadow caster", wgpu::DepthBiasState { constant: 2, slope_scale: 2.0, clamp: 0.0 });
            ShadowMap { view, uniform, read, write, pipe, sampler }
        })
    }

    /// REMASTER volumetric fog sun shafts: the map, built on first use.
    fn shaft_map(&self) -> &ShaftMap {
        self.shaft.get_or_init(|| {
            let d = &self.device;
            let tex = d.create_texture(&wgpu::TextureDescriptor {
                label: Some("fog shadow map"),
                size: wgpu::Extent3d { width: SHAFT_SIZE, height: SHAFT_SIZE, depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: DEPTH,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            });
            let view = tex.create_view(&Default::default());
            let uniform = d.create_buffer(&wgpu::BufferDescriptor { label: Some("fog shadow uniform"), size: 80, usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
            let sampler = d.create_sampler(&wgpu::SamplerDescriptor {
                label: Some("fog shadow"),
                mag_filter: wgpu::FilterMode::Linear,
                min_filter: wgpu::FilterMode::Linear,
                compare: Some(wgpu::CompareFunction::LessEqual),
                ..Default::default()
            });
            let write = d.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("fog shadow caster"),
                layout: &self.shadow_layouts.1,
                entries: &[wgpu::BindGroupEntry { binding: 0, resource: uniform.as_entire_binding() }],
            });
            let pipe = self.caster_pipe("fog shadow caster", wgpu::DepthBiasState { constant: 2, slope_scale: 2.0, clamp: 0.0 });
            ShaftMap { view, uniform, write, pipe, sampler }
        })
    }

    /// REMASTER volumetric fog sun shafts: render the world (with its fill), the scene objects and the cars into the fog
    /// shadow map. Returns the map's world -> clip matrix and depth range, or `None` with the sun under the horizon.
    #[allow(clippy::too_many_arguments)]
    fn draw_shaft_map(&self, enc: &mut wgpu::CommandEncoder, s: &GpuScene, v: &View, cars: &[(usize, Mat4)], props: Option<(&wgpu::Buffer, u32)>, inv_vp: Mat4, fwd: Vec3, cam: Vec3, range: f32, sun: Vec3, generated: bool) -> Option<(Mat4, f32)> {
        // the frustum between the near plane and `range` (view depth): rays through the screen corners
        let mut corners = [Vec3::ZERO; 8];
        for (i, c) in corners.iter_mut().enumerate() {
            let ndc = glam::Vec2::new(if i & 1 == 0 { -1.0 } else { 1.0 }, if i & 2 == 0 { -1.0 } else { 1.0 });
            let (a, b) = (inv_vp * ndc.extend(0.0).extend(1.0), inv_vp * ndc.extend(1.0).extend(1.0));
            let dir = (b.truncate() / b.w - a.truncate() / a.w).normalize();
            let z = if i & 4 == 0 { crate::volfog::NEAR } else { range };
            *c = cam + dir * (z / dir.dot(fwd).max(0.05));
        }
        let (vp, depth) = shaft_matrix(&corners, s.world_box, sun)?;
        let m = self.shaft_map();
        let mut u = [0.0f32; 20];
        u[..16].copy_from_slice(&vp.to_cols_array());
        u[16..].copy_from_slice(&[depth, 1.0 / SHAFT_SIZE as f32, 0.0, 0.0]);
        self.queue.write_buffer(&m.uniform, 0, bytemuck::cast_slice(&u));
        let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("fog shadow map"),
            color_attachments: &[],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: &m.view,
                depth_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Clear(1.0), store: wgpu::StoreOp::Store }),
                stencil_ops: None,
            }),
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_pipeline(&m.pipe);
        pass.set_bind_group(1, self.aniso_bind(1), &[]);
        pass.set_bind_group(2, &m.write, &[]);
        // the world whole (the water's ground too), then the fill pieces
        pass.set_bind_group(0, &s.bind, &[]);
        pass.set_vertex_buffer(0, s.tris.slice(..));
        pass.draw(0..s.n_tris, 0..1);
        if let Some((b, n)) = s.fill.as_ref().filter(|_| v.world_fill()) {
            pass.set_vertex_buffer(0, b.slice(..));
            pass.draw(0..*n, 0..1);
        }
        if let Some((b, n)) = props {
            pass.set_bind_group(0, &s.prop_bind, &[]);
            pass.set_vertex_buffer(0, b.slice(..));
            pass.draw(0..n, 0..1);
        }
        for (slot, &(mesh, _)) in cars.iter().enumerate().take(MAX_CARS) {
            let Some((vb, n)) = s.car_lod(mesh, generated, self.is_far(mesh)) else { continue };
            pass.set_bind_group(0, &s.car_binds[slot], &[]);
            pass.set_vertex_buffer(0, vb.slice(..));
            pass.draw(0..*n, 0..1);
        }
        Some((vp, depth))
    }

    /// The caster pipeline of the sun's map and the lamp atlas (`vs_shadow`, depth only).
    fn caster_pipe(&self, label: &str, bias: wgpu::DepthBiasState) -> wgpu::RenderPipeline {
        let vbuf = wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<Vertex>() as u64,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x2, 2 => Float32x4, 3 => Float32, 4 => Float32x3, 5 => Float32],
        };
        self.device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some(label),
            layout: Some(&self.src.shadow_layout),
            vertex: wgpu::VertexState { module: &self.src.shader, entry_point: Some("vs_shadow"), compilation_options: Default::default(), buffers: &[Some(vbuf)] },
            primitive: wgpu::PrimitiveState { cull_mode: None, ..Default::default() },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: DEPTH,
                depth_write_enabled: Some(true),
                depth_compare: Some(wgpu::CompareFunction::Less),
                stencil: Default::default(),
                bias,
            }),
            multisample: Default::default(),
            fragment: None,
            multiview_mask: None,
            cache: None,
        })
    }

    /// Group 2 of the Remaster pipelines: the sun's map and uniform, the lamp atlas and its uniform.
    fn shadow_read_group(&self, label: &str, uniform: &wgpu::Buffer, view: &wgpu::TextureView, sampler: &wgpu::Sampler, atlas: &wgpu::TextureView, spot_u: &wgpu::Buffer) -> wgpu::BindGroup {
        let entry = |binding, resource| wgpu::BindGroupEntry { binding, resource };
        self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some(label),
            layout: &self.shadow_layouts.0,
            entries: &[
                entry(0, uniform.as_entire_binding()),
                entry(1, wgpu::BindingResource::TextureView(view)),
                entry(2, wgpu::BindingResource::Sampler(sampler)),
                entry(3, wgpu::BindingResource::TextureView(atlas)),
                entry(4, spot_u.as_entire_binding()),
            ],
        })
    }

    /// A depth array texture of `layers` layers of `size`², and its array view.
    fn depth_array(&self, label: &str, size: u32, layers: u32) -> (wgpu::Texture, wgpu::TextureView) {
        let tex = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some(label),
            size: wgpu::Extent3d { width: size, height: size, depth_or_array_layers: layers },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: DEPTH,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let view = tex.create_view(&wgpu::TextureViewDescriptor { dimension: Some(wgpu::TextureViewDimension::D2Array), ..Default::default() });
        (tex, view)
    }

    /// The 1-texel atlas and zeroed uniform that stand in until lamps get shadow maps.
    fn spot_dummy(&self) -> (wgpu::TextureView, wgpu::Buffer) {
        let (_, view) = self.depth_array("lamp shadow dummy", 1, 1);
        (view, self.spot_uniform())
    }

    fn spot_uniform(&self) -> wgpu::Buffer {
        self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("lamp shadow uniform"),
            size: std::mem::size_of::<SpotU>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        })
    }

    /// REMASTER lamp shadows: the atlas, built on first use.
    fn spot_shadows(&self, size: usize) -> &SpotShadows {
        self.spot[size].get_or_init(|| {
            let d = &self.device;
            let (tex, atlas) = self.depth_array("lamp shadow atlas", SPOT_SIZES[size], SPOT_LAYERS as u32);
            let uniform = self.spot_uniform();
            let sun = self.shadow_map();
            let read = self.shadow_read_group("lamp shadows", &sun.uniform, &sun.view, &sun.sampler, &atlas, &uniform);
            let casters = (0..SPOT_LAYERS)
                .map(|n| {
                    let view = tex.create_view(&wgpu::TextureViewDescriptor { dimension: Some(wgpu::TextureViewDimension::D2), base_array_layer: n as u32, array_layer_count: Some(1), ..Default::default() });
                    let buf = d.create_buffer(&wgpu::BufferDescriptor { label: Some("lamp shadow caster"), size: 80, usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
                    let bind = d.create_bind_group(&wgpu::BindGroupDescriptor {
                        label: Some("lamp shadow caster"),
                        layout: &self.shadow_layouts.1,
                        entries: &[wgpu::BindGroupEntry { binding: 0, resource: buf.as_entire_binding() }],
                    });
                    (view, buf, bind)
                })
                .collect();
            // perspective depth is steep near the lamp: the bias is applied in metres by the receiver
            let pipe = self.caster_pipe("lamp shadow caster", wgpu::DepthBiasState { constant: 1, slope_scale: 1.5, clamp: 0.0 });
            SpotShadows { uniform, read, casters, pipe }
        })
    }

    /// Group 2 for the Remaster pass: the lamp atlas when this frame has valid maps.
    fn shadow_read(&self) -> &wgpu::BindGroup {
        match self.spot[self.spot_size.get()].get() {
            Some(s) if self.spot_ready.get() => &s.read,
            _ => &self.shadow_map().read,
        }
    }

    /// The headlight sources of this frame's cars and of the hidden ones (the player's car in the
    /// in-car views).
    fn head_sources(&self, s: &GpuScene, f: &Frame, generated: bool) -> Vec<HeadSrc> {
        f.cars.iter().take(MAX_CARS).chain(f.light_cars).filter_map(|(mesh, m)| headlight_source(s, *mesh, m, generated)).collect()
    }

    /// REMASTER light shadows: pick this frame's lamps (see `spot_select`) and cars (`head_select`)
    /// and fade the layers.
    fn spot_update(&self, s: &GpuScene, f: &Frame, cam: Vec3, generated: bool, t: &LampLightTune, mode: LightShadows) {
        let bodies: Vec<(usize, Vec3)> = f.cars.iter().take(MAX_CARS).filter(|(m, _)| m.is_multiple_of(CAR_MESHES)).map(|(m, x)| (m / CAR_MESHES, x.w_axis.truncate())).collect();
        let centres: Vec<Vec3> = bodies.iter().map(|b| b.1).collect();
        let on = s.look.darkness > 0.0 && mode != LightShadows::Off;
        let mut a = self.spot_alloc.borrow_mut();
        let mut h = self.head_alloc.borrow_mut();
        let wanted = if on && mode == LightShadows::All { spot_select(&s.lamps, &a, &centres, cam, t) } else { Vec::new() };
        let heads = if on { head_select(&self.head_sources(s, f, generated), &bodies, &h, cam, t) } else { Vec::new() };
        // a pinned clock (headless shots and cine renders) steps the fades by sim time, so a render repeats exactly
        let dt = match self.clock_pin.get() {
            Some(t) => {
                let d = a.pinned.map_or(0.0, |p| (t - p).clamp(0.0, 0.1));
                a.pinned = Some(t);
                d
            }
            None => {
                let now = std::time::Instant::now();
                let d = a.at.map_or(0.0, |p| now.duration_since(p).as_secs_f32().min(0.1));
                a.at = Some(now);
                d
            }
        };
        a.step(&wanted, dt);
        h.step(&heads, dt);
    }

    /// REMASTER light shadows: render the cars into each held layer (the lamps', then the
    /// headlights', which leave out their own car). False without any layer.
    fn draw_spot_shadows(&self, enc: &mut wgpu::CommandEncoder, s: &GpuScene, f: &Frame, generated: bool, t: &LampLightTune, quality: ShadowQuality) -> bool {
        let (alloc, head) = (self.spot_alloc.borrow(), self.head_alloc.borrow());
        // (atlas layer, fade, view and widest window, (position, range), car slot left out)
        type ShadowJob = (usize, f32, (Mat4, [f32; 4]), Vec4, Option<usize>);
        let mut jobs: Vec<ShadowJob> = Vec::new();
        for (n, slot) in alloc.layers.iter().enumerate() {
            let Some(l) = slot else { continue };
            let lamp = &s.lamps[l.key];
            let range = t.shadow_range(lamp);
            jobs.push((n, l.fade, lamp_shadow_view(lamp.pos, t.shadow_cone), lamp.pos.extend(range), None));
        }
        let sources = self.head_sources(s, f, generated);
        for (n, slot) in head.layers.iter().enumerate() {
            let Some(l) = slot else { continue };
            let Some(src) = sources.iter().find(|x| x.key == l.key) else { continue };
            jobs.push((LAMP_LAYERS + n, l.fade, head_shadow_view(src, t), src.pos.extend(t.head_reach), Some(src.key)));
        }
        if jobs.is_empty() {
            return false;
        }
        let size = quality.atlas();
        let edge = SPOT_SIZES[size];
        let sp = self.spot_shadows(size);
        let mut u = <SpotU as bytemuck::Zeroable>::zeroed();
        for (n, fade, (view, max), light, skip) in jobs {
            // REMASTER: the window follows the cars that can cast into this light
            let casters: Vec<Vec3> = f
                .cars
                .iter()
                .take(MAX_CARS)
                .filter(|(mesh, m)| mesh.is_multiple_of(CAR_MESHES) && skip != Some(mesh / CAR_MESHES) && m.w_axis.truncate().distance(light.truncate()) <= light.w + 6.0)
                .map(|(_, m)| m.w_axis.truncate())
                .collect();
            let (vp, texel) = fit_shadow(view, max, &casters, light.w, edge, quality.pcf());
            u.vp[n] = vp.to_cols_array();
            u.light[n] = light.into();
            u.p[n] = [fade, t.bias, 1.0 / edge as f32, t.shadow_cone.to_radians().cos()];
            u.q[n] = [texel, quality.pcf() as f32, 0.0, 0.0];
            let (view, buf, bind) = &sp.casters[n];
            let mut c = [0.0f32; 20];
            c[..16].copy_from_slice(&u.vp[n]);
            self.queue.write_buffer(buf, 0, bytemuck::cast_slice(&c));
            let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("light shadow map"),
                color_attachments: &[],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view,
                    depth_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Clear(1.0), store: wgpu::StoreOp::Store }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&sp.pipe);
            pass.set_bind_group(1, self.aniso_bind(1), &[]);
            pass.set_bind_group(2, bind, &[]);
            for (slot, &(mesh, m)) in f.cars.iter().enumerate().take(MAX_CARS) {
                if skip == Some(mesh / CAR_MESHES) || m.w_axis.truncate().distance(light.truncate()) > light.w + 6.0 {
                    continue;
                }
                let Some((vb, n)) = s.car_lod(mesh, generated, self.is_far(mesh)) else { continue };
                pass.set_bind_group(0, &s.car_binds[slot], &[]);
                pass.set_vertex_buffer(0, vb.slice(..));
                pass.draw(0..*n, 0..1);
            }
        }
        self.queue.write_buffer(&sp.uniform, 0, bytemuck::bytes_of(&u));
        true
    }

    /// REMASTER car shadows: render the cars into the sun's depth map. False without cars in
    /// range or a sun above the horizon.
    /// REMASTER: decide which cars of `cars` take their original model in this view (`far_mask`). The main
    /// view keeps its choice for the next frame (the band); the mirror and the rest use the plain threshold.
    /// Nothing is far in Classic, the wireframe, overlays, holograms, the garage studio and sub-viewports.
    fn far_cars(&self, cars: &[(usize, Mat4)], cam: &Camera, v: &View, f: &Frame, width: f32) {
        let on = v.mode == RenderMode::Remaster && v.generated_cars() && f.wire == WireStyle::Off && !f.overlay && !f.hologram && !f.studio && f.viewport.is_none();
        let (mut mask, mut memory) = (0u64, self.far_memory.get());
        if on {
            let main = !f.mirror;
            let switch = gen_switch_distance(v.remaster.car_lod_distance, cam.zoom, width);
            for &(mesh, m) in cars.iter().take(MAX_CARS).filter(|(mesh, _)| mesh.is_multiple_of(CAR_MESHES)) {
                let bit = 1u64 << (mesh / CAR_MESHES % 64);
                let was = main.then_some(memory & bit != 0);
                let far = gen_far(cam.pos.distance(m.w_axis.truncate()), switch, was);
                if far {
                    mask |= bit;
                }
                if main {
                    memory = (memory & !bit) | (mask & bit);
                }
            }
        }
        self.far_mask.set(mask);
        if on && !f.mirror {
            self.far_memory.set(memory);
        }
    }

    /// REMASTER: car mesh `mesh` draws its original model in the view `far_cars` last decided
    fn is_far(&self, mesh: usize) -> bool {
        mesh < VEL_GHOST_KEY && self.far_mask.get() >> (mesh / CAR_MESHES % 64) & 1 != 0
    }

    fn draw_shadow_map(&self, enc: &mut wgpu::CommandEncoder, s: &GpuScene, cars: &[(usize, Mat4)], cam: Vec3, (sun, overhead): (Vec3, bool), generated: bool) -> bool {
        let Some((vp, depth)) = shadow_matrix(cars, cam, sun) else { return false };
        let m = self.shadow_map();
        let mut u = [0.0f32; 20];
        u[..16].copy_from_slice(&vp.to_cols_array());
        // w: soft radius in map uv (overhead mode only; `shadow_matrix` sets depth = 2 r + 16)
        let soft = if overhead { OVERHEAD_SOFT / (depth - 16.0) } else { 0.0 };
        u[16..].copy_from_slice(&[depth, 1.0 / SHADOW_SIZE as f32, SHADOW_BIAS / depth, soft]);
        self.queue.write_buffer(&m.uniform, 0, bytemuck::cast_slice(&u));
        let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("car shadow map"),
            color_attachments: &[],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: &m.view,
                depth_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Clear(1.0), store: wgpu::StoreOp::Store }),
                stencil_ops: None,
            }),
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_pipeline(&m.pipe);
        pass.set_bind_group(1, self.aniso_bind(1), &[]);
        pass.set_bind_group(2, &m.write, &[]);
        for (slot, &(mesh, _)) in cars.iter().enumerate().take(MAX_CARS) {
            let Some((vb, n)) = s.car_lod(mesh, generated, self.is_far(mesh)) else { continue };
            pass.set_bind_group(0, &s.car_binds[slot], &[]);
            pass.set_vertex_buffer(0, vb.slice(..));
            pass.draw(0..*n, 0..1);
        }
        true
    }

    /// Upload a scene at once (garage, menu drive, motorview, headless shots).
    pub fn load(&mut self, s: &Scene) {
        self.load_prepared(s, SceneUpload::new(s));
    }

    /// `load` with the CPU half already built (on the race loading worker, #335).
    pub fn load_prepared(&mut self, s: &Scene, up: SceneUpload) {
        let mut l = self.begin_load(s, up);
        while !self.load_step(&mut l, s, None) {}
    }

    /// #335 start uploading `s` (with its `SceneUpload`): the textures are created here, their
    /// layers and the vertex buffers are written by `load_step`. The current scene stays drawn
    /// until the last step swaps the new one in; dropping the `SceneLoad` cancels.
    pub fn begin_load(&self, s: &Scene, up: SceneUpload) -> SceneLoad {
        let d = &self.device;
        // layers past the device limit are dropped (they sample a clamped layer)
        let layers = (s.layers.len() as u32).min(d.limits().max_texture_array_layers);
        let tex = d.create_texture(&wgpu::TextureDescriptor {
            label: Some("textures"),
            size: wgpu::Extent3d { width: TEX, height: TEX, depth_or_array_layers: layers },
            mip_level_count: TEX.ilog2() + 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        // REMASTER texture pack (#211): the pack's layers as one array with a mip chain (a 1x1 dummy without a pack)
        let pack_layers = (s.pack.layers.len() as u32).min(d.limits().max_texture_array_layers).max(1);
        let pack_size = if s.pack.layers.is_empty() { 1 } else { s.pack.size };
        let pack_tex = d.create_texture(&wgpu::TextureDescriptor {
            label: Some("texture pack"),
            size: wgpu::Extent3d { width: pack_size, height: pack_size, depth_or_array_layers: pack_layers },
            mip_level_count: pack_size.ilog2() + 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        // REMASTER sky dome (09-remaster 9.32)
        let (hw, hh, hmips) = s.hdri.as_ref().map_or((1, 1, 1), |h| (h.width, h.height, h.mip_levels));
        let hdri = d.create_texture(&wgpu::TextureDescriptor {
            label: Some("sky hdri"),
            size: wgpu::Extent3d { width: hw, height: hh, depth_or_array_layers: 1 },
            mip_level_count: hmips,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba16Float,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let mut jobs: Vec<UploadStep> = (0..layers).map(UploadStep::Layer).collect();
        jobs.extend((0..pack_layers).map(UploadStep::PackLayer));
        jobs.push(UploadStep::Sky);
        jobs.extend((0..hmips).map(UploadStep::Hdri));
        let mut buf = |k: VSrc, keep: bool| {
            if keep {
                jobs.push(UploadStep::Buf(k));
            }
        };
        buf(VSrc::Tris, true);
        buf(VSrc::Fill, !s.fill.is_empty());
        buf(VSrc::Water, !s.water.is_empty());
        buf(VSrc::Dry, up.dry.is_some());
        buf(VSrc::Grass, !s.grass.is_empty());
        buf(VSrc::Lines, true);
        for i in 0..s.cars.len() {
            buf(VSrc::Car(i), true);
            buf(VSrc::Holo(i), true);
        }
        for (i, c) in s.remaster_cars.iter().enumerate() {
            buf(VSrc::RemasterCar(i), !c.is_empty());
            buf(VSrc::HoloGen(i), up.holo_gen.get(i).is_some_and(|h| h.is_some()));
        }
        for (i, l) in s.car_lods.iter().enumerate() {
            for (k, m) in l.iter().enumerate() {
                buf(VSrc::Lod(i, k), !m.is_empty());
            }
        }
        jobs.push(UploadStep::Finish);
        SceneLoad { up, tex, layers, pack_tex, pack_layers, pack_size, sky: None, hdri, bufs: std::collections::HashMap::new(), partial: None, jobs, next: 0 }
    }

    /// #335 write the next steps of `l` (`s` = the scene `begin_load` was given) for about
    /// `budget` (at least one step; None: all of them). True once the scene is swapped in.
    pub fn load_step(&mut self, l: &mut SceneLoad, s: &Scene, budget: Option<std::time::Duration>) -> bool {
        let t0 = std::time::Instant::now();
        while let Some(&job) = l.jobs.get(l.next) {
            l.next += 1;
            match job {
                UploadStep::Layer(i) => {
                    let base = &s.layers[i as usize];
                    for (lvl, data) in std::iter::once(base).chain(&l.up.mips[i as usize]).enumerate() {
                        let size = (TEX >> lvl).max(1);
                        self.write_layer(&l.tex, lvl as u32, i, size, data);
                    }
                }
                UploadStep::PackLayer(i) => {
                    let dummy = vec![128u8, 128, 255, 255];
                    let base = s.pack.layers.get(i as usize).unwrap_or(&dummy);
                    for (lvl, data) in std::iter::once(base).chain(l.up.pack_mips.get(i as usize).into_iter().flatten()).enumerate() {
                        let edge = (l.pack_size >> lvl).max(1);
                        self.write_layer(&l.pack_tex, lvl as u32, i, edge, data);
                    }
                }
                UploadStep::Sky => {
                    let (sw, sh, spix) = match &s.sky_image {
                        Some(i) => (i.width, i.height, i.rgba.as_slice()),
                        None => (1, 1, &[0u8, 0, 0, 255][..]),
                    };
                    l.sky = Some(self.device.create_texture_with_data(
                        &self.queue,
                        &wgpu::TextureDescriptor {
                            label: Some("sky"),
                            size: wgpu::Extent3d { width: sw, height: sh, depth_or_array_layers: 1 },
                            mip_level_count: 1,
                            sample_count: 1,
                            dimension: wgpu::TextureDimension::D2,
                            format: wgpu::TextureFormat::Rgba8Unorm,
                            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                            view_formats: &[],
                        },
                        wgpu::util::TextureDataOrder::LayerMajor,
                        spix,
                    ));
                }
                UploadStep::Hdri(lvl) => {
                    // all levels one after the other, mip 0 first, RGBA16F (8 bytes a texel)
                    let (hw, hh, data) = match &s.hdri {
                        Some(h) => (h.width, h.height, h.data.as_slice()),
                        None => (1, 1, &[0u8; 8][..]),
                    };
                    let dims = |k: u32| ((hw >> k).max(1), (hh >> k).max(1));
                    let offset: usize = (0..lvl).map(|k| { let (w, h) = dims(k); (w * h * 8) as usize }).sum();
                    let (w, h) = dims(lvl);
                    self.queue.write_texture(
                        wgpu::TexelCopyTextureInfo { texture: &l.hdri, mip_level: lvl, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
                        &data[offset..offset + (w * h * 8) as usize],
                        wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(w * 8), rows_per_image: Some(h) },
                        wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
                    );
                }
                UploadStep::Buf(k) if std::mem::size_of_val(k.verts(s, &l.up)) > UPLOAD_CHUNK => {
                    // a big buffer (the generated cars and their hologram edges): a slice a step
                    let bytes: &[u8] = bytemuck::cast_slice(k.verts(s, &l.up));
                    let (b, at) = l.partial.take().unwrap_or_else(|| {
                        let usage = wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST | if k == VSrc::Grass { wgpu::BufferUsages::STORAGE } else { wgpu::BufferUsages::empty() };
                        (self.device.create_buffer(&wgpu::BufferDescriptor { label: Some(k.label()), size: bytes.len() as u64, usage, mapped_at_creation: false }), 0)
                    });
                    let end = (at + UPLOAD_CHUNK).min(bytes.len());
                    self.queue.write_buffer(&b, at as u64, &bytes[at..end]);
                    if end < bytes.len() {
                        l.partial = Some((b, end));
                        l.next -= 1;
                    } else {
                        l.bufs.insert(k, (b, (bytes.len() / std::mem::size_of::<Vertex>()) as u32));
                    }
                }
                UploadStep::Buf(k) => {
                    let v = k.verts(s, &l.up);
                    let b = if k == VSrc::Grass {
                        grass_buffer(&self.device, v)
                    } else {
                        let dummy = [Vertex { pos: [0.0; 3], uv: [0.0; 2], color: [0.0; 4], layer: -1.0, normal: [0.0; 3], env: 0.0 }];
                        self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                            label: Some(k.label()),
                            contents: bytemuck::cast_slice(if v.is_empty() { &dummy[..] } else { v }),
                            usage: wgpu::BufferUsages::VERTEX,
                        })
                    };
                    l.bufs.insert(k, (b, v.len() as u32));
                }
                UploadStep::Finish => {
                    self.finish_load(l, s);
                    return true;
                }
            }
            if budget.is_some_and(|b| t0.elapsed() >= b) {
                return false;
            }
        }
        true
    }

/// One layer and level of a square RGBA8 texture array.
    fn write_layer(&self, tex: &wgpu::Texture, lvl: u32, layer: u32, edge: u32, data: &[u8]) {
        self.queue.write_texture(
            wgpu::TexelCopyTextureInfo { texture: tex, mip_level: lvl, origin: wgpu::Origin3d { x: 0, y: 0, z: layer }, aspect: wgpu::TextureAspect::All },
            data,
            wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(edge * 4), rows_per_image: Some(edge) },
            wgpu::Extent3d { width: edge, height: edge, depth_or_array_layers: 1 },
        );
    }

    /// The last step of a `SceneLoad`: the bind groups and the `GpuScene`, swapped in for the old
    /// one, and the per-scene state of the old scene dropped.
    fn finish_load(&mut self, l: &mut SceneLoad, s: &Scene) {
        self.local_src = None;
        self.local_bake = None;
        self.local_sun = None;
        self.history = RefCell::new(None);
        self.prev_view = RefCell::new(None);
        self.sky_scroll.borrow_mut().clear();
        if let Some(vf) = self.volfog.get() {
            vf.reset();
        }
        // REMASTER light shadows: the held layers name the old track's lamps and cars
        self.spot_alloc = RefCell::new(SpotAlloc::default());
        self.head_alloc = RefCell::new(SpotAlloc::with_layers(HEAD_LAYERS));
        let array = |t: &wgpu::Texture| t.create_view(&wgpu::TextureViewDescriptor { dimension: Some(wgpu::TextureViewDimension::D2Array), ..Default::default() });
        let sky_view = l.sky.take().expect("the sky step runs before the last").create_view(&Default::default());
        let views = [array(&l.tex), sky_view, l.hdri.create_view(&Default::default()), array(&l.pack_tex)];
        let pack_map = self.pack_map(&s.pack, l.pack_layers);
        let local = self.local_textures(None);
        let GlobalBinds { bind, car_binds, prop_bind, probe_bind, probe_car_binds, probe_prop_bind } = self.global_binds(&views, &pack_map, &local);
        let (grass_bind, grass_trail) = self.grass_bind(s.grass_edge.as_ref());
        let mut take = |k: VSrc| l.bufs.remove(&k);
        let (tris, n_tris) = take(VSrc::Tris).expect("the world buffer is always uploaded");
        let (lines, n_lines) = take(VSrc::Lines).expect("the line buffer is always uploaded");
        self.scene = Some(GpuScene {
            tris,
            n_tris,
            world_box: l.up.world_box,
            fill: take(VSrc::Fill),
            water: take(VSrc::Water),
            tris_dry: take(VSrc::Dry),
            water_params: s.water_params.clone(),
            grass: take(VSrc::Grass),
            grass_params: s.grass_params.clone(),
            grass_bind,
            grass_edge: s.grass_edge.as_ref().map(|e| (e.transform(), e.cap)),
            grass_trail,
            trail: RefCell::new(TrailState::default()),
            blade_bind: RefCell::new(None),
            lines,
            n_lines,
            bind,
            cars: (0..s.cars.len()).filter_map(|i| take(VSrc::Car(i))).collect(),
            // small (a few thousand triangles per wheel), so uploaded with the scene: F9 then
            // switches without a stall
            remaster_cars: (0..s.remaster_cars.len()).map(|i| take(VSrc::RemasterCar(i))).collect(),
            car_lods: (0..s.car_lods.len()).map(|i| [take(VSrc::Lod(i, 0)), take(VSrc::Lod(i, 1)), take(VSrc::Lod(i, 2))]).collect(),
            car_binds,
            prop_bind,
            probe_bind,
            probe_car_binds,
            probe_prop_bind,
            views,
            pack_map,
            local,
            props: s.props.clone(),
            look: s.look,
            point_lights: s.point_lights.clone(),
            world_halos: s.world_halos.clone(),
            lamps: s.lamps.clone(),
            car_halos: s.car_halos.clone(),
            remaster_car_halos: s.remaster_car_halos.clone(),
            prop_halos: s.prop_halos.clone(),
            prop_lamps: s.prop_lamps.clone(),
            fx: s.fx.clone(),
            sky_size: s.sky_image.as_ref().map_or((0, 0), |i| (i.width, i.height)),
            dome: s.hdri.as_ref().map(|h| h.dome()),
            view_distance: s.view_distance,
            wire: None,
            car_wires: Vec::new(),
            car_wires_gen: Vec::new(),
            holo_wires_gen: (0..s.remaster_cars.len()).map(|i| take(VSrc::HoloGen(i))).collect(),
            holo_wires: (0..s.cars.len()).filter_map(|i| take(VSrc::Holo(i))).collect(),
            holo_road: None,
            car_swap: None,
            car_colours: s.car_colours.clone(),
            object_scale: s.object_scale,
        });
    }

    /// The group 0 bind groups of the scene: the globals of the world, the cars and the props,
    /// with the env cube and (for drawing into the env probe) without it.
    fn global_binds(&self, views: &[wgpu::TextureView; 4], pack_map: &wgpu::Buffer, local: &LocalLightGpu) -> GlobalBinds {
        let group_env = |u: &wgpu::Buffer, env: &wgpu::TextureView| {
            self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("globals"),
                layout: &self.layout,
                entries: &[
                    wgpu::BindGroupEntry { binding: 0, resource: u.as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(&views[0]) },
                    wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::Sampler(&self.sampler) },
                    wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::TextureView(&views[1]) },
                    wgpu::BindGroupEntry { binding: 4, resource: wgpu::BindingResource::TextureView(env) },
                    wgpu::BindGroupEntry { binding: 5, resource: wgpu::BindingResource::TextureView(&views[2]) },
                    wgpu::BindGroupEntry { binding: 6, resource: wgpu::BindingResource::TextureView(&local.index) },
                    wgpu::BindGroupEntry { binding: 7, resource: wgpu::BindingResource::TextureView(&local.atlas) },
                    wgpu::BindGroupEntry { binding: 8, resource: wgpu::BindingResource::TextureView(&local.sun) },
                    wgpu::BindGroupEntry { binding: 9, resource: wgpu::BindingResource::TextureView(&views[3]) },
                    wgpu::BindGroupEntry { binding: 10, resource: pack_map.as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 11, resource: self.prop_lights.as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 12, resource: self.prop_of.as_entire_binding() },
                ],
            })
        };
        let group = |u: &wgpu::Buffer| group_env(u, &self.probe.cube);
        let probe_group = |u: &wgpu::Buffer| group_env(u, &self.probe.dummy);
        GlobalBinds {
            bind: group(&self.uniforms),
            car_binds: self.car_uniforms.iter().map(group).collect(),
            prop_bind: group(&self.prop_uniform),
            probe_bind: probe_group(&self.uniforms),
            probe_car_binds: self.car_uniforms.iter().map(probe_group).collect(),
            probe_prop_bind: probe_group(&self.prop_uniform),
        }
    }

    /// REMASTER texture pack (#211): the uniform of pack layer ids per base layer, two to a
    /// `vec4<i32>` (-1: the original; `n` = the pack array's layers). Normal layers keep their
    /// plain box mips (`SceneUpload`): the averaged vectors get shorter, which flattens the bumps
    /// with distance (`map_normal` normalises the result).
    fn pack_map(&self, pack: &crate::texpack::Pack, n: u32) -> wgpu::Buffer {
        let mut ids = vec![-1i32; crate::scene::MAX_LAYERS * 2];
        for (k, m) in pack.map.iter().enumerate().take(crate::scene::MAX_LAYERS) {
            let ok = |v: i32| if (v as u32) < n { v } else { -1 };
            ids[2 * k] = ok(m[0]);
            ids[2 * k + 1] = ok(m[1]);
        }
        self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some("texture pack ids"), contents: bytemuck::cast_slice(&ids), usage: wgpu::BufferUsages::UNIFORM })
    }

    /// REMASTER local light (#83): the volume's textures, or 1x1x1 dummies (`p` None).
    fn local_textures(&self, p: Option<&crate::probes::Packed>) -> LocalLightGpu {
        let tex3 = |label, size: [u32; 3], format, data: &[u8]| {
            let t = self.device.create_texture_with_data(
                &self.queue,
                &wgpu::TextureDescriptor {
                    label: Some(label),
                    size: wgpu::Extent3d { width: size[0], height: size[1], depth_or_array_layers: size[2] },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D3,
                    format,
                    usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                    view_formats: &[],
                },
                wgpu::util::TextureDataOrder::LayerMajor,
                data,
            );
            t.create_view(&Default::default())
        };
        match p {
            Some(p) => LocalLightGpu {
                index: tex3("local light index", p.index_size, wgpu::TextureFormat::R32Uint, bytemuck::cast_slice(&p.index)),
                atlas: tex3("local light atlas", p.atlas_size, wgpu::TextureFormat::Rgba16Float, bytemuck::cast_slice(&p.atlas)),
                sun: tex3("local light sun", [p.atlas_size[0], p.atlas_size[1], p.atlas_size[2] / 4], wgpu::TextureFormat::R8Unorm, &p.sun),
                origin: Some(p.origin),
                shown: None,
            },
            None => LocalLightGpu {
                index: tex3("local light index", [1; 3], wgpu::TextureFormat::R32Uint, &[0; 4]),
                atlas: tex3("local light atlas", [1; 3], wgpu::TextureFormat::Rgba16Float, &[0; 8]),
                sun: tex3("local light sun", [1; 3], wgpu::TextureFormat::R8Unorm, &[255]),
                origin: None,
                shown: None,
            },
        }
    }

    /// REMASTER local light (#83): what the scene just loaded bakes its probe volume from (call
    /// after `load`, which drops the previous one; None: this scene has none).
    pub fn set_local_source(&mut self, src: Option<crate::probes::BakeSource>) {
        self.local_src = src.map(std::sync::Arc::new);
        self.local_sun = None;
    }

    /// REMASTER local light (#83): bake on the calling thread instead of a worker, and show the
    /// volume at once (headless shots: the whole bake is in the picture).
    pub fn wait_local_light(&mut self, on: bool) {
        self.local_wait = on;
    }

    /// REMASTER local light: the first frame `v` wants it, takes the scene's probe volume from
    /// the disk cache (shown at once) or starts its bake on a worker thread; later frames upload
    /// the bake when it is done (it then fades in). Unbaked, lit objects keep the sky ambient.
    /// The probes' sun term is baked for the view's sun (`sky_look`): when that turns (the HDRI
    /// option, the F11 sky turn), the volume is taken or rebaked again for the new sun and
    /// replaces the shown one without a fade (#192).
    pub fn update_local_light(&mut self, v: &View) {
        let (Some(g), Some(src)) = (&self.scene, &self.local_src) else { return };
        if !v.local_light() {
            return;
        }
        let sun = sky_look(v, g).sun_dir;
        if self.local_want.0 != sun {
            self.local_want = (sun, std::time::Instant::now());
        }
        let turned = self.local_sun.is_some_and(|b| Vec3::from(b).dot(Vec3::from(sun)) < LOCAL_SUN_TURN);
        let settled = self.local_wait || self.local_want.1.elapsed().as_secs_f32() >= LOCAL_SUN_SETTLE;
        if self.local_sun.is_none() || (turned && settled) {
            let (src, fade) = (src.clone(), g.local.origin.is_none());
            self.local_sun = Some(sun);
            self.local_bake = None;
            let t0 = std::time::Instant::now();
            if let Some(p) = src.cached(Vec3::from(sun)) {
                self.upload_local_light(&p, false);
                println!("local light: from the cache in {:.0} ms", t0.elapsed().as_secs_f32() * 1000.0);
            } else if self.local_wait {
                let threads = std::thread::available_parallelism().map_or(4, |n| n.get());
                if let Some(p) = src.bake_packed(Vec3::from(sun), threads, &std::sync::atomic::AtomicBool::new(false)) {
                    self.upload_local_light(&p, false);
                }
            } else {
                self.local_bake = Some((crate::probes::Baking::start(src, Vec3::from(sun)), fade));
            }
        }
        if let Some((res, fade)) = self.local_bake.as_ref().and_then(|(b, fade)| b.poll().map(|r| (r, *fade))) {
            self.local_bake = None;
            if let Some(p) = res {
                self.upload_local_light(&p, fade);
            }
        }
    }

    /// REMASTER local light: where the scene's probe volume is, for the F11 page.
    pub fn local_status(&self) -> &'static str {
        match (&self.scene, &self.local_src, &self.local_bake) {
            (Some(g), _, Some(_)) if g.local.origin.is_some() => "on the GPU, rebaking for the new sun direction",
            (Some(g), _, _) if g.local.origin.is_some() => "on the GPU",
            (_, _, Some(_)) => "baking in the background",
            (_, Some(_), _) => "not baked yet: it bakes (or comes from the cache) when the option is on in Remaster",
            _ => "none: this scene has no road to bake around",
        }
    }

    fn upload_local_light(&mut self, p: &crate::probes::Packed, fade: bool) {
        let mut local = self.local_textures(Some(p));
        local.shown = fade.then(std::time::Instant::now);
        let Some(mut g) = self.scene.take() else { return };
        let b = self.global_binds(&g.views, &g.pack_map, &local);
        (g.bind, g.car_binds, g.prop_bind, g.probe_bind, g.probe_car_binds, g.probe_prop_bind) = (b.bind, b.car_binds, b.prop_bind, b.probe_bind, b.probe_car_binds, b.probe_prop_bind);
        g.local = local;
        self.scene = Some(g);
    }

    /// REMASTER: replace the water bodies (after `load`; the fill editor's live edits). `s` is
    /// the world the mesh `m` was built from.
    pub fn set_water(&mut self, s: &Scene, m: &crate::procgen::water::Mesh) {
        let Some(g) = &mut self.scene else { return };
        let mk = |d: &Vec<Vertex>, label| (self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some(label), contents: bytemuck::cast_slice(d), usage: wgpu::BufferUsages::VERTEX }), d.len() as u32);
        g.water = (!m.verts.is_empty()).then(|| mk(&m.verts, "water"));
        g.tris_dry = dry_world(&s.tris, &m.hide, &m.bed).map(|d| mk(&d, "world without water"));
        g.water_params = m.params.clone();
    }

    /// REMASTER: replace the grass bodies (after `load`; the World Editor's live edits).
    pub fn set_grass(&mut self, m: &crate::procgen::grass::Mesh) {
        let (bind, trail) = self.grass_bind(m.edge.as_ref());
        let Some(g) = &mut self.scene else { return };
        g.grass_trail = trail;
        g.trail = RefCell::new(TrailState::default());
        g.grass = (!m.verts.is_empty()).then(|| (grass_buffer(&self.device, &m.verts), m.verts.len() as u32));
        g.grass_params = m.params.clone();
        g.grass_bind = bind;
        g.blade_bind = RefCell::new(None);
        g.grass_edge = m.edge.as_ref().map(|e| (e.transform(), e.cap));
    }

    /// REMASTER grass: group 3 of the grass pipeline, the edge map (`EdgeMap`, R8, bilinear,
    /// clamped) or a 1x1 zero texture when the scene has none, and the trail texture (#158,
    /// binding 2): R8 over the same rectangle at `TRAIL_TEXEL_DIV` times the map's resolution,
    /// `TRAIL_MAX` texels a side at most, cleared (a 1x1 zero without a map, and no trail).
    fn grass_bind(&self, map: Option<&crate::procgen::grass::EdgeMap>) -> (wgpu::BindGroup, Option<(wgpu::Texture, wgpu::TextureView)>) {
        let d = &self.device;
        let trail = map.map(|m| {
            let size = m.size.map(|n| (n * TRAIL_TEXEL_DIV).clamp(1, TRAIL_MAX));
            let tex = d.create_texture(&wgpu::TextureDescriptor {
                label: Some("grass trail"),
                size: wgpu::Extent3d { width: size[0], height: size[1], depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::R8Unorm,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::RENDER_ATTACHMENT,
                view_formats: &[],
            });
            let view = tex.create_view(&Default::default());
            (tex, view)
        });
        let (w, h, data): (u32, u32, &[u8]) = map.map_or((1, 1, &[0u8]), |m| (m.size[0], m.size[1], &m.data));
        let tex = d.create_texture(&wgpu::TextureDescriptor {
            label: Some("grass edge"),
            size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::R8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        // rows padded to the 256-byte copy alignment
        let stride = (w as usize).div_ceil(256) * 256;
        let mut padded = vec![0u8; stride * h as usize];
        for (row, src) in padded.chunks_exact_mut(stride).zip(data.chunks_exact(w as usize)) {
            row[..w as usize].copy_from_slice(src);
        }
        self.queue.write_texture(
            wgpu::TexelCopyTextureInfo { texture: &tex, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
            &padded,
            wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(stride as u32), rows_per_image: Some(h) },
            wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
        );
        let view = tex.create_view(&Default::default());
        let sampler = clamp_sampler(d, "grass edge");
        let bind = d.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("grass edge"),
            layout: &self.src.grass_group,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&view) },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&sampler) },
                wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::TextureView(trail.as_ref().map_or(&view, |t| &t.1)) },
            ],
        });
        (bind, trail)
    }

    /// REMASTER: replace the world fill (after `load`; the fill editor's live edits).
    pub fn set_fill(&mut self, v: &[Vertex]) {
        let Some(g) = &mut self.scene else { return };
        let b = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some("world fill"), contents: bytemuck::cast_slice(v), usage: wgpu::BufferUsages::VERTEX });
        g.fill = (!v.is_empty()).then_some((b, v.len() as u32));
    }

    /// REMASTER: the line list `Frame::holo_road` draws as hologram (after `load`).
    pub fn load_holo_road(&mut self, lines: &[Vertex]) {
        let Some(g) = &mut self.scene else { return };
        let b = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some("hologram road"), contents: bytemuck::cast_slice(lines), usage: wgpu::BufferUsages::VERTEX });
        g.holo_road = (!lines.is_empty()).then_some((b, lines.len() as u32));
    }

    /// DEBUG (motorview Swap mesh): draw car slot `slot` with the other model, original <->
    /// generated (`GpuScene::gen_for`); None = as the view says. Kept until the next `load`.
    pub fn set_car_swap(&mut self, slot: Option<usize>) {
        if let Some(sc) = &mut self.scene {
            sc.car_swap = slot;
        }
    }

    /// Build the wireframe line buffers for the loaded scene (after `load`; debug viewer only).
    pub fn load_wireframe(&mut self, s: &Scene) {
        let d = &self.device;
        let buf = |v: &[Vertex]| {
            let b = d.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("wireframe"),
                contents: bytemuck::cast_slice(if v.is_empty() { &[Vertex { pos: [0.0; 3], uv: [0.0; 2], color: [0.0; 4], layer: -1.0, normal: [0.0; 3], env: 0.0 }] } else { v }),
                usage: wgpu::BufferUsages::VERTEX,
            });
            (b, v.len() as u32)
        };
        let bufs = |w: &wire::WireSet| WireBufs { lines: buf(&w.lines), xray: buf(&w.xray), fill: buf(&w.fill) };
        let mk = |tris: &[Vertex], color, check: bool| {
            let plain = wire::plain(tris, color);
            let c = check.then(|| wire::analyse(tris));
            GpuWire { stats: c.as_ref().map_or(plain.stats, |c| c.stats), plain: bufs(&plain), check: c.as_ref().map(bufs) }
        };
        let Some(g) = &mut self.scene else { return };
        g.wire = Some(mk(&s.tris, WIRE_WORLD, false));
        g.car_wires = s.cars.iter().map(|c| mk(c, WIRE_CAR, true)).collect();
        g.car_wires_gen = s.remaster_cars.iter().map(|c| (!c.is_empty()).then(|| mk(c, WIRE_CAR, true))).collect();
    }

    /// DEBUG: the mesh check counts of car mesh `mesh` as the view draws it (`generated` = the view's
    /// generated cars); None before `load_wireframe`.
    pub fn wire_stats(&self, mesh: usize, generated: bool) -> Option<WireStats> {
        self.scene.as_ref()?.car_wire(mesh, generated).map(|w| w.stats)
    }

    /// Upload the UI atlas (once; call again only if it changes).
    pub fn load_ui(&mut self, atlas: &Atlas) {
        let tex = self.device.create_texture_with_data(
            &self.queue,
            &wgpu::TextureDescriptor {
                label: Some("ui atlas"),
                size: wgpu::Extent3d { width: ATLAS, height: ATLAS, depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8Unorm,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            },
            wgpu::util::TextureDataOrder::LayerMajor,
            &atlas.rgba,
        );
        let view = tex.create_view(&Default::default());
        let sampler = self.device.create_sampler(&wgpu::SamplerDescriptor {
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        self.ui_bind = Some(self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("ui"),
            layout: &self.ui_layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: self.ui_uniform.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(&view) },
                wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::Sampler(&sampler) },
            ],
        }));
        self.ui_atlas = Some((view, sampler));
        self.ui_tex = Some(tex);
        self.ui_sharp = OnceCell::new();
    }

    /// REMASTER: change the rev meter's look constants (the game keeps the defaults; the Rev
    /// meter workshop edits them live).
    pub fn set_rev_look(&self, look: &crate::revmeter::RevLook) {
        self.queue.write_buffer(&self.rev_look, 0, bytemuck::cast_slice(&look.pack()));
    }

    /// REMASTER: the loaded track's sky.cfg values changed (the Sky page saved them).
    pub fn set_dome_cfg(&mut self, yaw: f32, exposure: f32, follow_sun: bool) {
        if let Some(d) = self.scene.as_mut().and_then(|s| s.dome.as_mut()) {
            (d.yaw, d.exposure, d.follow_sun) = (yaw, exposure, follow_sun);
        }
    }

    /// Upload one rect of the UI atlas again (REMASTER: the close minimap's slot).
    pub fn update_ui(&self, atlas: &Atlas, r: Rect) {
        let Some(tex) = &self.ui_tex else { return };
        self.queue.write_texture(
            wgpu::TexelCopyTextureInfo { texture: tex, mip_level: 0, origin: wgpu::Origin3d { x: r.x, y: r.y, z: 0 }, aspect: wgpu::TextureAspect::All },
            &atlas.rgba[((r.y * ATLAS + r.x) * 4) as usize..],
            wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(ATLAS * 4), rows_per_image: None },
            wgpu::Extent3d { width: r.w, height: r.h, depth_or_array_layers: 1 },
        );
    }

    /// REMASTER: the sharp HUD's pipeline and bind group (UI atlas + field atlas)
    fn ui_sharp(&self, sdf: &SdfAtlas) -> Option<&UiSharp> {
        let (view, sampler) = self.ui_atlas.as_ref()?;
        Some(self.ui_sharp.get_or_init(|| {
            let tex = self.device.create_texture_with_data(
                &self.queue,
                &wgpu::TextureDescriptor {
                    label: Some("ui fields"),
                    size: wgpu::Extent3d { width: sdf.width, height: sdf.height, depth_or_array_layers: 1 },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: wgpu::TextureFormat::Rg8Unorm,
                    usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                    view_formats: &[],
                },
                wgpu::util::TextureDataOrder::LayerMajor,
                &sdf.data,
            );
            let fields = tex.create_view(&Default::default());
            let (pipe, layout) = ui_pipeline(&self.device, self.color_format, true);
            let bind = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("ui sharp"),
                layout: &layout,
                entries: &[
                    wgpu::BindGroupEntry { binding: 0, resource: self.ui_uniform.as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(view) },
                    wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::Sampler(sampler) },
                    wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::TextureView(&fields) },
                    wgpu::BindGroupEntry { binding: 4, resource: self.rev_look.as_entire_binding() },
                ],
            });
            let (glow_pipe, _) = ui_pipeline_for(&self.device, HDR, true, Some(&layout));
            UiSharp { pipe, bind, glow_pipe, fields }
        }))
    }

    /// Draw a UI batch over the target (or over `clear` colour). `sdf`: the batch is a
    /// REMASTER sharp one (`Batch::sharp`) and has quads from this field atlas.
    #[allow(clippy::too_many_arguments)]
    pub fn draw_ui(&self, enc: &mut wgpu::CommandEncoder, target: &wgpu::TextureView, size: (u32, u32), canvas: Canvas, verts: &[UiVertex], clear: Option<[f64; 3]>, sdf: Option<&SdfAtlas>) {
        let (pipe, bind) = match sdf {
            Some(sdf) => match self.ui_sharp(sdf) {
                Some(u) => (&u.pipe, &u.bind),
                None => return,
            },
            None => match &self.ui_bind {
                Some(b) => (&self.ui_pipe, b),
                None => return,
            },
        };
        let (sx, sy, ox, oy) = canvas.axes(size);
        let g: [f32; 8] = [sx, ox, oy, sy, size.0 as f32, size.1 as f32, 0.0, 0.0];
        self.queue.write_buffer(&self.ui_uniform, 0, bytemuck::cast_slice(&g));
        let vb = (!verts.is_empty()).then(|| {
            self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("ui verts"),
                contents: bytemuck::cast_slice(verts),
                usage: wgpu::BufferUsages::VERTEX,
            })
        });
        let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("ui"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: target,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: match clear {
                        Some(c) => wgpu::LoadOp::Clear(wgpu::Color { r: c[0], g: c[1], b: c[2], a: 1.0 }),
                        None => wgpu::LoadOp::Load,
                    },
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        if let Some(vb) = &vb {
            pass.set_pipeline(pipe);
            pass.set_bind_group(0, bind, &[]);
            pass.set_vertex_buffer(0, vb.slice(..));
            pass.draw(0..verts.len() as u32, 0..1);
        }
    }

    /// REMASTER frosted scene (#300): the scene drawn once into a sharp copy, that copy blurred
    /// with the glow's gaussian (`SceneFrost::blur`), and `target` written with the sharp scene
    /// outside the cards and the blurred one inside them (scenefrost.wgsl) [design choice].
    #[allow(clippy::too_many_arguments)]
    fn draw_scene_frosted(&self, enc: &mut wgpu::CommandEncoder, target: &wgpu::TextureView, depth: &wgpu::TextureView, size: (u32, u32), cam: &Camera, v: &View, f: &Frame, fr: SceneFrost) {
        let g = self.glow();
        let sf = self.scene_frost.get_or_init(|| {
            let shader = self.device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("scene frost"), source: wgpu::ShaderSource::Wgsl(include_str!("scenefrost.wgsl").into()) });
            let tex = |binding| wgpu::BindGroupLayoutEntry {
                binding,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: true }, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false },
                count: None,
            };
            let layout = self.device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("scene frost"),
                entries: &[
                    tex(0),
                    tex(1),
                    wgpu::BindGroupLayoutEntry { binding: 2, visibility: wgpu::ShaderStages::FRAGMENT, ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering), count: None },
                    wgpu::BindGroupLayoutEntry {
                        binding: 3,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None },
                        count: None,
                    },
                ],
            });
            let pl = self.device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("scene frost"), bind_group_layouts: &[Some(&layout)], immediate_size: 0 });
            let pipe = self.device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("scene frost"),
                layout: Some(&pl),
                vertex: wgpu::VertexState { module: &shader, entry_point: Some("vs_frost"), compilation_options: Default::default(), buffers: &[] },
                primitive: Default::default(),
                depth_stencil: None,
                multisample: Default::default(),
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some("fs_frost"),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState { format: self.color_format, blend: None, write_mask: wgpu::ColorWrites::ALL })],
                }),
                multiview_mask: None,
                cache: None,
            });
            SceneFrostGpu { pipe, layout, targets: RefCell::new(None) }
        });
        let half = (size.0.div_ceil(2), size.1.div_ceil(2));
        let mut targets = sf.targets.borrow_mut();
        if targets.as_ref().is_none_or(|t| t.0 != size) {
            let sharp = self
                .device
                .create_texture(&wgpu::TextureDescriptor {
                    label: Some("scene frost sharp"),
                    size: wgpu::Extent3d { width: size.0, height: size.1, depth_or_array_layers: 1 },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: self.color_format,
                    usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
                    view_formats: &[],
                })
                .create_view(&Default::default());
            *targets = Some((size, sharp, [self.half_hdr("scene frost", half), self.half_hdr("scene frost", half)]));
        }
        let (_, sharp, blur) = targets.as_ref().unwrap();
        // the scene itself, once, into the sharp copy
        self.frosting.set(true);
        self.draw(enc, sharp, depth, size, cam, v, f);
        self.frosting.set(false);
        let clear = wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT);
        let sigma = fr.blur.max(0.5);
        self.glow_fullscreen(enc, &g.blur, sharp, [1.0 / size.0 as f32, 0.0, (sigma / 4.0).max(0.25), sigma], &blur[0], clear);
        let sigma = (fr.blur * 0.5).max(0.5);
        self.glow_fullscreen(enc, &g.blur, &blur[0], [0.0, 1.0 / half.1 as f32, (sigma / 4.0).max(0.25), sigma], &blur[1], clear);
        let mut data = [[0.0f32; 4]; SCENE_FROST_RECTS + 1];
        for (d, (c, h)) in data.iter_mut().zip(fr.rects.iter().take(fr.n.min(SCENE_FROST_RECTS))) {
            *d = [c[0], c[1], h[0], h[1]];
        }
        data[SCENE_FROST_RECTS] = [fr.n.min(SCENE_FROST_RECTS) as f32, fr.radius, 0.0, 0.0];
        let u = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some("scene frost"), contents: bytemuck::cast_slice(&data), usage: wgpu::BufferUsages::UNIFORM });
        let bind = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("scene frost"),
            layout: &sf.layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(sharp) },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(&blur[1]) },
                wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::Sampler(&g.sampler) },
                wgpu::BindGroupEntry { binding: 3, resource: u.as_entire_binding() },
            ],
        });
        let mut p = fullscreen_target_pass(enc, "scene frost", target, clear);
        p.set_pipeline(&sf.pipe);
        p.set_bind_group(0, &bind, &[]);
        p.draw(0..3, 0..1);
    }

    /// CHEAT ramlosa / h2o "Underwater" (#317, `0x4419b4`, both modes): the scene drawn once into a copy, then one
    /// full-screen pass (underwater.wgsl) writes `target` zoomed 1.15 to 1.25 times around the centre with a sideways
    /// ripple, phase `phi` [design choice: one image pass, not per vertex; the original anchors the zoom at the corner].
    #[allow(clippy::too_many_arguments)]
    fn draw_underwater(&self, enc: &mut wgpu::CommandEncoder, target: &wgpu::TextureView, depth: &wgpu::TextureView, size: (u32, u32), cam: &Camera, v: &View, f: &Frame, phi: f32) {
        use motor_sim::cheats::{WOBBLE_AMP_PX, WOBBLE_BREATH, WOBBLE_WAVES, WOBBLE_ZOOM};
        let uw = self.underwater.get_or_init(|| {
            let shader = self.device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("underwater"), source: wgpu::ShaderSource::Wgsl(include_str!("underwater.wgsl").into()) });
            let layout = self.device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("underwater"),
                entries: &[
                    wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: true }, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry { binding: 1, visibility: wgpu::ShaderStages::FRAGMENT, ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering), count: None },
                    wgpu::BindGroupLayoutEntry {
                        binding: 2,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None },
                        count: None,
                    },
                ],
            });
            let pl = self.device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("underwater"), bind_group_layouts: &[Some(&layout)], immediate_size: 0 });
            let pipe = self.device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("underwater"),
                layout: Some(&pl),
                vertex: wgpu::VertexState { module: &shader, entry_point: Some("vs_warp"), compilation_options: Default::default(), buffers: &[] },
                primitive: Default::default(),
                depth_stencil: None,
                multisample: Default::default(),
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some("fs_warp"),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState { format: self.color_format, blend: None, write_mask: wgpu::ColorWrites::ALL })],
                }),
                multiview_mask: None,
                cache: None,
            });
            UnderwaterGpu { pipe, layout, sampler: clamp_sampler(&self.device, "underwater"), copy: RefCell::new(None) }
        });
        let mut copy = uw.copy.borrow_mut();
        if copy.as_ref().is_none_or(|c| c.0 != size) {
            let tex = self.device.create_texture(&wgpu::TextureDescriptor {
                label: Some("underwater copy"),
                size: wgpu::Extent3d { width: size.0, height: size.1, depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: self.color_format,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            });
            *copy = Some((size, tex.create_view(&Default::default())));
        }
        let (_, view) = copy.as_ref().unwrap();
        // the scene itself, once, into the copy
        self.warping.set(true);
        self.draw(enc, view, depth, size, cam, v, f);
        self.warping.set(false);
        // the amplitude is 10 px at 480 lines (`WOBBLE_AMP_PX`), scaled with the window height
        let h = size.1 as f32;
        let data: [f32; 8] = [phi, WOBBLE_ZOOM + WOBBLE_BREATH * phi.cos(), WOBBLE_AMP_PX * h / 480.0, WOBBLE_WAVES, size.0 as f32, h, 0.0, 0.0];
        let u = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some("underwater"), contents: bytemuck::cast_slice(&data), usage: wgpu::BufferUsages::UNIFORM });
        let bind = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("underwater"),
            layout: &uw.layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(view) },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&uw.sampler) },
                wgpu::BindGroupEntry { binding: 2, resource: u.as_entire_binding() },
            ],
        });
        let mut p = fullscreen_target_pass(enc, "underwater", target, wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT));
        p.set_pipeline(&uw.pipe);
        p.set_bind_group(0, &bind, &[]);
        p.draw(0..3, 0..1);
    }

    /// REMASTER: the smooth glow of a sharp UI batch (`Batch::glow`) added over `target`: the
    /// sources at half size, blurred, composited. Call it before the batch itself.
    /// REMASTER frosted hologram (#296): the hologram drawn once without its clip into a sharp
    /// copy, that copy blurred with the glow's gaussian (`HoloClip::blur` = sigma in window px, x
    /// at half size, then y), and both added over `target`: sharp outside the clip, blurred inside
    /// (holofrost.wgsl). Replaces the ring of turned cameras, one full hologram pass per tap
    /// (17 of them behind the glass cards) [design choice].
    #[allow(clippy::too_many_arguments)]
    fn draw_hologram_frosted(&self, enc: &mut wgpu::CommandEncoder, target: &wgpu::TextureView, depth: &wgpu::TextureView, size: (u32, u32), cam: &Camera, v: &View, f: &Frame) {
        let (Some(s), Some(clip)) = (&self.scene, f.holo_clip) else { return };
        let g = self.glow();
        let fr = self.holo_frost.get_or_init(|| {
            let src = format!("{}\n{}", SHADER_SRC, include_str!("holofrost.wgsl"));
            let shader = self.device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("hologram frost"), source: wgpu::ShaderSource::Wgsl(src.into()) });
            let tex = |binding| wgpu::BindGroupLayoutEntry {
                binding,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: true }, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false },
                count: None,
            };
            let group = self.device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("hologram frost"),
                entries: &[tex(0), tex(1), wgpu::BindGroupLayoutEntry { binding: 2, visibility: wgpu::ShaderStages::FRAGMENT, ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering), count: None }],
            });
            let pl = self.device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("hologram frost"), bind_group_layouts: &[Some(&self.layout), None, None, Some(&group)], immediate_size: 0 });
            // adds light like the hologram's own passes (`Pass::Holo`)
            let add = wgpu::BlendComponent { src_factor: wgpu::BlendFactor::One, dst_factor: wgpu::BlendFactor::One, operation: wgpu::BlendOperation::Add };
            let pipe = self.device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("hologram frost"),
                layout: Some(&pl),
                vertex: wgpu::VertexState { module: &shader, entry_point: Some("vs_sky"), compilation_options: Default::default(), buffers: &[] },
                primitive: Default::default(),
                depth_stencil: None,
                multisample: Default::default(),
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some("fs_holo_frost"),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState { format: self.color_format, blend: Some(wgpu::BlendState { color: add, alpha: wgpu::BlendComponent::OVER }), write_mask: wgpu::ColorWrites::ALL })],
                }),
                multiview_mask: None,
                cache: None,
            });
            HoloFrost { pipe, group, targets: RefCell::new(None) }
        });
        let half = (size.0.div_ceil(2), size.1.div_ceil(2));
        let mut targets = fr.targets.borrow_mut();
        if targets.as_ref().is_none_or(|t| t.0 != size) {
            let sharp = self
                .device
                .create_texture(&wgpu::TextureDescriptor {
                    label: Some("hologram frost sharp"),
                    size: wgpu::Extent3d { width: size.0, height: size.1, depth_or_array_layers: 1 },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: self.color_format,
                    usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
                    view_formats: &[],
                })
                .create_view(&Default::default());
            let blur = [self.half_hdr("hologram frost", half), self.half_hdr("hologram frost", half)];
            let bind = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("hologram frost"),
                layout: &fr.group,
                entries: &[
                    wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&sharp) },
                    wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(&blur[1]) },
                    wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::Sampler(&g.sampler) },
                ],
            });
            *targets = Some((size, sharp, blur, bind));
        }
        let (_, sharp, blur, bind) = targets.as_ref().unwrap();
        let clear = wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT);
        // the hologram (the overlay pass loads its target), unclipped: `HoloClip::mode` 3
        drop(fullscreen_target_pass(enc, "hologram frost clear", sharp, clear));
        self.frosting.set(true);
        self.draw(enc, sharp, depth, size, cam, v, f);
        self.frosting.set(false);
        // sigma in source texels: window px along x, half of them along y (glow.wgsl, taps sigma / 4 apart)
        let sigma = clip.blur.max(0.5);
        self.glow_fullscreen(enc, &g.blur, sharp, [1.0 / size.0 as f32, 0.0, (sigma / 4.0).max(0.25), sigma], &blur[0], clear);
        let sigma = (clip.blur * 0.5).max(0.5);
        self.glow_fullscreen(enc, &g.blur, &blur[0], [0.0, 1.0 / half.1 as f32, (sigma / 4.0).max(0.25), sigma], &blur[1], clear);
        // the clip's mask from the globals the hologram pass has just written
        let mut p = fullscreen_target_pass(enc, "hologram frost", target, wgpu::LoadOp::Load);
        p.set_pipeline(&fr.pipe);
        p.set_bind_group(0, &s.bind, &[]);
        p.set_bind_group(3, bind, &[]);
        p.draw(0..3, 0..1);
    }

    /// REMASTER the smooth glow's pipelines and sampler (glow.wgsl), made on first use; the frosted
    /// hologram (`draw_hologram_frosted`) blurs with the same gaussian.
    fn glow(&self) -> &Glow {
        self.glow.get_or_init(|| {
            let shader = self.device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("glow"), source: wgpu::ShaderSource::Wgsl(include_str!("glow.wgsl").into()) });
            let layout = self.device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("glow"),
                entries: &[
                    wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: true }, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry { binding: 1, visibility: wgpu::ShaderStages::FRAGMENT, ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering), count: None },
                    wgpu::BindGroupLayoutEntry {
                        binding: 2,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None },
                        count: None,
                    },
                ],
            });
            let pl = self.device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("glow"), bind_group_layouts: &[Some(&layout)], immediate_size: 0 });
            let pipe = |entry: &str, format, blend| {
                self.device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                    label: Some("glow"),
                    layout: Some(&pl),
                    vertex: wgpu::VertexState { module: &shader, entry_point: Some("vs_glow"), compilation_options: Default::default(), buffers: &[] },
                    primitive: Default::default(),
                    depth_stencil: None,
                    multisample: Default::default(),
                    fragment: Some(wgpu::FragmentState {
                        module: &shader,
                        entry_point: Some(entry),
                        compilation_options: Default::default(),
                        targets: &[Some(wgpu::ColorTargetState { format, blend, write_mask: wgpu::ColorWrites::ALL })],
                    }),
                    multiview_mask: None,
                    cache: None,
                })
            };
            let over = wgpu::BlendState { color: wgpu::BlendComponent::OVER, alpha: wgpu::BlendComponent::OVER };
            Glow {
                blur: pipe("fs_glow_blur", HDR, None),
                comp: pipe("fs_glow_comp", self.color_format, Some(over)),
                layout,
                sampler: self.device.create_sampler(&wgpu::SamplerDescriptor {
                    mag_filter: wgpu::FilterMode::Linear,
                    min_filter: wgpu::FilterMode::Linear,
                    address_mode_u: wgpu::AddressMode::ClampToEdge,
                    address_mode_v: wgpu::AddressMode::ClampToEdge,
                    ..Default::default()
                }),
                targets: RefCell::new(None),
            }
        })
    }

    pub fn draw_glow(&self, enc: &mut wgpu::CommandEncoder, target: &wgpu::TextureView, size: (u32, u32), canvas: Canvas, verts: &[UiVertex], sdf: &SdfAtlas) {
        let Some(sharp) = self.ui_sharp(sdf) else { return };
        let Some((atlas, atlas_sampler)) = self.ui_atlas.as_ref() else { return };
        if verts.is_empty() {
            return;
        }
        let g = self.glow();
        let half = (size.0.div_ceil(2), size.1.div_ceil(2));
        let mut targets = g.targets.borrow_mut();
        if targets.as_ref().is_none_or(|(s, _)| *s != half) {
            *targets = Some((half, [self.half_hdr("glow", half), self.half_hdr("glow", half)]));
        }
        let views = &targets.as_ref().unwrap().1;
        // its own uniforms: the shared UI one may be rewritten before this encoder is submitted
        let (sx, sy, ox, oy) = canvas.axes(size);
        let globals = [sx * 0.5, ox * 0.5, oy * 0.5, sy * 0.5, half.0 as f32, half.1 as f32, 0.0, 0.0];
        let src_uniform = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some("glow"), contents: bytemuck::cast_slice(&globals), usage: wgpu::BufferUsages::UNIFORM });
        let src_bind = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("glow sources"),
            layout: &sharp.glow_pipe.get_bind_group_layout(0),
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: src_uniform.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(atlas) },
                wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::Sampler(atlas_sampler) },
                wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::TextureView(&sharp.fields) },
                wgpu::BindGroupEntry { binding: 4, resource: self.rev_look.as_entire_binding() },
            ],
        });
        let vb = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some("glow verts"), contents: bytemuck::cast_slice(verts), usage: wgpu::BufferUsages::VERTEX });
        {
            let mut p = fullscreen_target_pass(enc, "glow", &views[0], wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT));
            p.set_pipeline(&sharp.glow_pipe);
            p.set_bind_group(0, &src_bind, &[]);
            p.set_vertex_buffer(0, vb.slice(..));
            p.draw(0..verts.len() as u32, 0..1);
        }
        // sigma in half-size texels; taps are sigma / 4 apart (glow.wgsl)
        let sigma = (GLOW_SIGMA * sx.min(sy) * 0.5).max(0.5);
        let step = (sigma / 4.0).max(0.25);
        let clear = wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT);
        self.glow_fullscreen(enc, &g.blur, &views[0], [1.0 / half.0 as f32, 0.0, step, sigma], &views[1], clear);
        self.glow_fullscreen(enc, &g.blur, &views[1], [0.0, 1.0 / half.1 as f32, step, sigma], &views[0], clear);
        self.glow_fullscreen(enc, &g.comp, &views[0], [0.0, 0.0, 0.0, GLOW_COMPOSITE], target, wgpu::LoadOp::Load);
    }

    /// A half-size `HDR` render target the glow passes read and write.
    fn half_hdr(&self, label: &str, half: (u32, u32)) -> wgpu::TextureView {
        self.device
            .create_texture(&wgpu::TextureDescriptor {
                label: Some(label),
                size: wgpu::Extent3d { width: half.0, height: half.1, depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: HDR,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            })
            .create_view(&Default::default())
    }

    /// One glow.wgsl full-screen pass (`Glow::blur` or `Glow::comp`): `src` through its uniform
    /// `data` (its own buffer: the shared ones may be rewritten before the encoder is submitted).
    fn glow_fullscreen(&self, enc: &mut wgpu::CommandEncoder, pipe: &wgpu::RenderPipeline, src: &wgpu::TextureView, data: [f32; 4], dst: &wgpu::TextureView, load: wgpu::LoadOp<wgpu::Color>) {
        let g = self.glow();
        let u = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some("glow"), contents: bytemuck::cast_slice(&data), usage: wgpu::BufferUsages::UNIFORM });
        let bind = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("glow"),
            layout: &g.layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(src) },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&g.sampler) },
                wgpu::BindGroupEntry { binding: 2, resource: u.as_entire_binding() },
            ],
        });
        let mut p = fullscreen_target_pass(enc, "glow", dst, load);
        p.set_pipeline(pipe);
        p.set_bind_group(0, &bind, &[]);
        p.draw(0..3, 0..1);
    }

    pub fn depth_view(&self, w: u32, h: u32) -> wgpu::TextureView {
        self.device
            .create_texture(&wgpu::TextureDescriptor {
                label: Some("depth"),
                size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: DEPTH,
                // (the lens flares read it: `flare_vis_pass`)
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            })
            .create_view(&Default::default())
    }

    /// REMASTER: the lights that cast a computed flare: world halos, scene-object halos, car lights.
    fn flare_sources(&self, s: &GpuScene, f: &Frame, cam: &Camera, v: &View) -> Vec<FlareSource> {
        let mid = |h: &HaloInst, m: Mat4| m.transform_point3((h.p0 + h.p1) * 0.5);
        // car and prop halos carry their colour in `rgb` as well as the texture's tint
        let tint = |h: &HaloInst| [0, 1, 2].map(|c| h.tint[c] * h.rgb[c]);
        let range = |p: Vec3| {
            let d = cam.pos.distance(p);
            let (a, b, c, e) = FLARE_RANGE;
            ((d - a) / (b - a)).clamp(0.0, 1.0) * ((e - d) / (e - c)).clamp(0.0, 1.0)
        };
        let mut out: Vec<FlareSource> = s.world_halos.iter().map(|h| FlareSource { p: mid(h, Mat4::IDENTITY), tint: h.tint, strength: range(mid(h, Mat4::IDENTITY)) }).collect();
        for (i, &(mesh, m)) in f.props.iter().enumerate() {
            let Some(h) = s.prop_halos.get(mesh) else { continue };
            let glow = f.prop_glow.get(i).copied().unwrap_or(Vec3::ONE).max_element().min(1.0);
            out.extend(h.iter().map(|h| FlareSource { p: mid(h, m), tint: tint(h), strength: glow * range(mid(h, m)) }));
        }
        let night = if v.dynamic_lights() { s.look.darkness } else { 0.0 };
        for &(mesh, m) in f.cars.iter().take(MAX_CARS) {
            let Some(h) = s.halos(mesh, v.generated_cars()) else { continue };
            let brake = v.remaster.car_lights.brake(f, mesh);
            let fwd = m.transform_vector3(Vec3::NEG_Z).normalize_or_zero();
            for h in h.iter().filter(|h| !matches!(h.strip, HaloStrip::Strip { .. })) {
                let p = mid(h, m);
                let to_eye = (cam.pos - p).normalize_or_zero();
                // the car the camera follows must not flare over itself: ramp in with distance
                let near = ((cam.pos.distance(p) - CAR_FLARE_NEAR.0) / (CAR_FLARE_NEAR.1 - CAR_FLARE_NEAR.0)).clamp(0.0, 1.0);
                let front = h.p0.z + h.p1.z <= 0.0;
                // headlights flare into the lens they face, tail lights the ones behind them
                let (lt, axis) = if front { (&v.remaster.car_lights.head, fwd) } else { (&v.remaster.car_lights.tail, -fwd) };
                let strength = range(p) * near * axis.dot(to_eye).max(0.0).powi(2) * lt.flare_strength(night, brake);
                out.push(FlareSource { p, tint: tint(h), strength });
            }
        }
        // scene-object headlamps flare into the lens they face, like a car's headlights
        for &(mesh, m) in f.props {
            let Some(pl) = s.prop_lamps.get(mesh) else { continue };
            let fwd = m.transform_vector3(pl.forward).normalize_or_zero();
            for h in &pl.lamps {
                let p = mid(h, m);
                let to_eye = (cam.pos - p).normalize_or_zero();
                let strength = range(p) * fwd.dot(to_eye).max(0.0).powi(2) * v.remaster.car_lights.head.flare_strength(night, 0.0);
                out.push(FlareSource { p, tint: tint(h), strength });
            }
        }
        out
    }

    /// REMASTER menu car preview (#325): the studio frame drawn 4x multisampled into its own transparent offscreen
    /// target, then laid over `target` with premultiplied blending (the main view's MSAA does not reach the overlay
    /// frames). `APPROX`: a cut-out texel's partial alpha-to-coverage edge blends slightly off; invisible at preview
    /// size. Returns false when it cannot (no MSAA, another target format): the caller draws the frame plainly.
    #[allow(clippy::too_many_arguments)]
    fn draw_studio_aa(&self, enc: &mut wgpu::CommandEncoder, target: &wgpu::TextureView, depth: &wgpu::TextureView, size: (u32, u32), cam: &Camera, v: &View, f: &Frame) -> bool {
        let Some(vp) = f.viewport else { return false };
        if v.mode != RenderMode::Remaster || self.scene.is_none() || target.texture().format() != self.color_format || self.msaa_samples(STUDIO_MSAA) < 2 {
            return false;
        }
        // the view's rectangle in whole window pixels (as the main pass clamps it, grown to the pixels it touches)
        let (x, y) = (vp[0].clamp(0.0, size.0 as f32), vp[1].clamp(0.0, size.1 as f32));
        let (x1, y1) = ((x + vp[2]).min(size.0 as f32).ceil(), (y + vp[3]).min(size.1 as f32).ceil());
        let (x0, y0) = (x.floor(), y.floor());
        let (w, h) = ((x1 - x0) as u32, (y1 - y0) as u32);
        if w < 1 || h < 1 {
            return true;
        }
        let sa = self.studio.get_or_init(|| {
            let shader = self.device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("studio aa"), source: wgpu::ShaderSource::Wgsl(include_str!("studioaa.wgsl").into()) });
            let layout = self.device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("studio aa"),
                entries: &[wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: false }, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false },
                    count: None,
                }],
            });
            let pl = self.device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("studio aa"), bind_group_layouts: &[Some(&layout)], immediate_size: 0 });
            let pipe = self.device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("studio aa"),
                layout: Some(&pl),
                vertex: wgpu::VertexState { module: &shader, entry_point: Some("vs_studio"), compilation_options: Default::default(), buffers: &[] },
                primitive: Default::default(),
                depth_stencil: None,
                multisample: Default::default(),
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some("fs_studio"),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState { format: self.color_format, blend: Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING), write_mask: wgpu::ColorWrites::ALL })],
                }),
                multiview_mask: None,
                cache: None,
            });
            StudioAa { pipe, layout, frame: RefCell::new(None) }
        });
        let mut frame = sa.frame.borrow_mut();
        if frame.as_ref().is_none_or(|t| t.0 != (w, h)) {
            let tex = self.device.create_texture(&wgpu::TextureDescriptor {
                label: Some("studio aa"),
                size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: self.color_format,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            });
            let view = tex.create_view(&Default::default());
            let bind = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("studio aa"),
                layout: &sa.layout,
                entries: &[wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&view) }],
            });
            *frame = Some(((w, h), view, bind));
        }
        let (_, view, bind) = frame.as_ref().unwrap();
        // the same frame, in a rectangle at the texture's origin (the offscreen target is that rectangle)
        let mut inner = f.clone();
        inner.viewport = Some([0.0, 0.0, w as f32, h as f32]);
        self.studio_aa.set(true);
        self.draw(enc, view, depth, (w, h), cam, v, &inner);
        self.studio_aa.set(false);
        let mut pass = fullscreen_target_pass(enc, "studio aa composite", target, wgpu::LoadOp::Load);
        pass.set_viewport(x0, y0, w as f32, h as f32, 0.0, 1.0);
        pass.set_scissor_rect(x0 as u32, y0 as u32, w, h);
        pass.set_pipeline(&sa.pipe);
        pass.set_bind_group(0, bind, &[]);
        pass.draw(0..3, 0..1);
        true
    }

    #[allow(clippy::too_many_arguments)]
    pub fn draw(&self, enc: &mut wgpu::CommandEncoder, target: &wgpu::TextureView, depth: &wgpu::TextureView, size: (u32, u32), cam: &Camera, v: &View, f: &Frame) {
        // REMASTER frosted glass: one hologram pass, blurred and composited (#296)
        if f.hologram && !self.frosting.get() && f.holo_clip.is_some_and(|c| c.blur > 0.0) {
            return self.draw_hologram_frosted(enc, target, depth, size, cam, v, f);
        }
        // REMASTER frosted scene: the Glass pause menu's cards (#300)
        if let Some(fr) = f.frost.filter(|_| !self.frosting.get()) {
            return self.draw_scene_frosted(enc, target, depth, size, cam, v, f, fr);
        }
        // CHEAT ramlosa / h2o (#317): the main 3D view goes through the wobble pass (`0x4419b4`), in both modes. After the
        // frost, so the pause menu's cards blur the warped picture; the mirror, overlays and the HUD are not warped.
        if let Some(phi) = f.underwater.filter(|_| !self.warping.get() && !f.overlay && !f.mirror && f.viewport.is_none() && !f.hologram) {
            return self.draw_underwater(enc, target, depth, size, cam, v, f, phi);
        }
        // REMASTER menu car preview: multisampled offscreen, composited over the window
        if f.studio && !self.studio_aa.get() && self.draw_studio_aa(enc, target, depth, size, cam, v, f) {
            return;
        }
        let cars = f.cars;
        let Some(s) = &self.scene else { return };
        // CHEAT lemmy / ace (#316), ramlosa / h2o (#317): the cloud ceiling replaces the sky in both modes, so no HDRI dome (its
        // sky ambient, reflections and fog colour go with it)
        let v_hell;
        let ceiling = Ceiling::of(f);
        let v = if ceiling.is_some() {
            v_hell = View { remaster: RemasterView { sky_hdri: false, ..v.remaster }, ..*v };
            &v_hell
        } else {
            v
        };
        // CHEAT tribute to tron (#320, 0x48e964): the original clears the shadow, skid, halo and sprite lists unseen and
        // skips the flares; its fog is always on (black). CLASSIC draws the Glide look (`tron_shade`). REMASTER keeps its
        // own path for the "Neon grid" (`neon_shade`, HDR and bloom, light walls, Goldbridge's dome graded cyan) with
        // everything off that has no Tron codes (water, grass, world fill, generated cars) or would light the black
        // (shadows, SSAO, haze)
        let (v_tron, f_tron);
        let (v, f) = if f.tron {
            let remaster = RemasterView {
                ssao: 0,
                car_shadows: false,
                fog_style: FogStyle::Classic,
                water: false,
                grass: GrassLevel::Off,
                world_fill: false,
                generated_cars: false,
                retro_signs: false,
                local_light: false,
                dynamic_lights: false,
                ..v.remaster
            };
            v_tron = View { shadows: false, halos: false, lens_flares: false, smoke: false, sparks: false, fog: true, remaster, ..*v };
            f_tron = Frame { shadows: &[], skids: &[], sprites: &[], prop_glow: &[], ..f.clone() };
            (&v_tron, &f_tron)
        } else {
            (v, f)
        };
        let aspect = match f.viewport {
            Some(vp) => vp[2] / vp[3].max(1.0),
            None => size.0 as f32 / size.1.max(1) as f32,
        };
        // far plane = ViewDistance * the user's view-distance option (0x4589a1) [1.0 assumed]
        let far = s.view_distance * v.distance();
        // CLASSIC (0x40b564): the TV camera scales the far plane within 60..300 m; REMASTER keeps
        // a longer view-distance option
        let far = if cam.far_k == 1.0 { far } else { (far * cam.far_k).clamp(motor_sim::tvcam::FAR_MIN, motor_sim::tvcam::FAR_MAX.max(far)) };
        // DEBUG: the clip distance (fog, halos and lights keep `far`)
        let clip_far = if v.debug.unlimited { UNLIMITED_FAR.max(far) } else { far };
        // the TV camera's zoom (0x40b498) narrows the 90 degree field: tan(fov_x / 2) = 1 / zoom
        let fov = 2.0 * ((v.fov_x.to_radians() / 2.0).tan() / cam.zoom / aspect).atan();
        let mut proj = glam::camera::rh::proj::directx::perspective(fov, aspect, NEAR, clip_far);
        if f.mirror {
            // flipping x reverses the winding: drawn with `tri_pipe_mirror`
            proj = Mat4::from_scale(Vec3::new(-1.0, 1.0, 1.0)) * proj;
        }
        // REMASTER TAA (13-antialiasing 13.4 phase 5): the main view gets a sub-pixel jitter of
        // the output pixel grid, as a clip-space shift (x and y by `jitter` times w). `proj_still`
        // is the unshifted one for everything that reads the view by position.
        let taa = v.mode == RenderMode::Remaster
            && v.remaster.post_aa.kind == PostAaKind::Taa
            && !f.overlay
            && !f.mirror
            && f.viewport.is_none()
            && !(f.wire.on() && s.wire.is_some())
            && target.texture().format() == self.color_format;
        let proj_still = proj;
        let jitter = if taa {
            let n = self.taa_frame.get();
            self.taa_frame.set(n.wrapping_add(1));
            let j = TAA_JITTER[n as usize % TAA_JITTER.len()];
            let o = target.texture().size();
            Mat4::from_translation(Vec3::new(2.0 * j[0] / o.width as f32, -2.0 * j[1] / o.height as f32, 0.0))
        } else {
            Mat4::IDENTITY
        };
        proj = jitter * proj;
        let view = glam::camera::rh::view::look_to_mat4(cam.pos, cam.forward(), cam.up());
        let mut look = sky_look(v, s);
        if f.studio {
            studio_look(&mut look, cam);
        }
        if let Some(c) = ceiling {
            ceiling_look(&mut look, c);
        }
        if f.tron {
            // CHEAT tribute to tron: black sky band and fog (grFogColorValue(0) in 0x48e964)
            (look.sky_top, look.sky_bottom, look.fog_colour, look.horizon_band) = ([0.0; 3], [0.0; 3], [0.0; 3], [0.0; 3]);
        }
        let l = &look;
        // REMASTER fog constants: the F11 / `--fog-tune` override or the track's own (`fog.rs`)
        let mut fog_tune = v.remaster.fog_tune.unwrap_or(l.fog);
        // REMASTER fog strength option: s x the haze, the distance ramp s times closer (s = 1 is exact)
        let fog_s = v.remaster.fog_strength as f32 / 100.0;
        fog_tune.density *= fog_s;
        fog_tune.start /= fog_s;
        fog_tune.full /= fog_s;
        let v4 = |c: [f32; 3], w: f32| [c[0], c[1], c[2], w];
        // Sky band (0x443a68): when level its top is at h*Horizon - image_height/2
        // (0x6fc304 = trunc(h*Horizon) - h/2, set by 0x443528); it moves with the horizon line
        // as the camera pitches. CLASSIC: the image spans one screen width at 4:3, and the U offset
        // is accumulated per frame, 1 image per 100 degrees of turn (`SKY_U_PER_RAD`), the step
        // taken modulo 180 degrees (0x443d93..0x443e43, 05-formats "Sky") [O]
        let horizon_y = 0.5 + cam.pitch.tan() / (2.0 * (fov / 2.0).tan());
        let fov_x = 2.0 * ((fov / 2.0).tan() * aspect).atan();
        let sky_u = {
            let mut scroll = self.sky_scroll.borrow_mut();
            let k = self.view_now.get();
            if scroll.len() <= k {
                scroll.resize(k + 1, None);
            }
            let u = match scroll[k] {
                // the first frame starts from the camera heading (design choice: the original keeps the offset across races)
                None => (cam.yaw * SKY_U_PER_RAD).rem_euclid(1.0),
                Some((u, heading)) => {
                    let step = (cam.yaw - heading + std::f32::consts::FRAC_PI_2).rem_euclid(std::f32::consts::PI) - std::f32::consts::FRAC_PI_2;
                    (u + step * SKY_U_PER_RAD).rem_euclid(1.0)
                }
            };
            // the mirror and overlays share the offset and don't advance it
            if !f.overlay && !f.mirror && f.viewport.is_none() {
                scroll[k] = Some((u, cam.yaw));
            }
            u
        };
        // design choice: the image keeps its shape on wider screens (one image per screen at 4:3)
        let u_per_screen = aspect * 0.75;
        let band = s.sky_size.1 as f32 / SKY_REF_LINES;
        let u_per_screen = if f.mirror { -u_per_screen } else { u_per_screen };
        let sky_geom = [horizon_y - 0.5 + l.horizon - band / 2.0, band, sky_u, u_per_screen];
        let mut g = Globals {
            view_proj: (proj * view).to_cols_array_2d(),
            view: view.to_cols_array_2d(),
            model: Mat4::IDENTITY.to_cols_array_2d(),
            // REMASTER sky dome and sky fog: the directions of the screen pixels
            sky_inv: (proj * view).inverse().to_cols_array_2d(),
            cam_pos: [cam.pos.x, cam.pos.y, cam.pos.z, 1.0 / aspect.max(1e-3)],
            // fog runs from CueStart * view distance to the view distance (0x439e84)
            params: [far * l.fog_start, far, v.brightness, if v.fog { 1.0 } else { 0.0 }],
            fog: v4(l.fog_colour, 1.0),
            ambient: v4(l.object_ambient, 0.0),
            sun: v4(l.sun, l.specular),
            sun_dir: v4(l.sun_dir, 0.0),
            // hologram clip rectangle (shader holo_gain): sky_top.w = corner radius, sky_bottom.w = 1
            sky_top: v4(l.sky_top, f.holo_clip.and_then(|c| c.rect).map_or(1.0, |r| r.1)),
            sky_bottom: v4(l.sky_bottom, f.holo_clip.is_some_and(|c| c.rect.is_some()) as u32 as f32),
            sky_geom: match f.holo_clip {
                Some(c) => match c.rect {
                    Some((h, _)) => [c.centre[0], c.centre[1], h[0], h[1]],
                    None => [c.centre[0], c.centre[1], c.radius, c.chord_y],
                },
                None if s.sky_size.1 == 0 || !v.sky_image => [horizon_y, 0.0, 0.0, 1.0],
                None => sky_geom,
            },
            // REMASTER: w = the per-pixel / clear coat flags
            env: [0.0, 0.0, 0.0, if v.mode == RenderMode::Remaster { v.remaster.shader_flags() } else if v.car_shading == CarShading::Flat { FLAT_FLAG as f32 } else { 0.0 }],
            // y: DEBUG shading view (`DebugShade`, 0 = off)
            spot_n: [0.0, v.debug.shade as u32 as f32, 0.0, f.holo_road],
            lights: [[0.0; 4]; 2 * MAX_LIGHTS],
            spots: [[0.0; 4]; 4 * MAX_SPOTS],
            myw: [[0.0; 4]; 2 * MAX_MYW],
            lamps: [[0.0; 4]; 2 * MAX_LAMPS],
            lamp_n: [0.0; 4],
            water: [[0.0; 4]; WATER_ROWS * MAX_WATER],
            grass: [[0.0; 4]; GRASS_ROWS * MAX_GRASS],
            grass_cars: [[0.0; 4]; 3 * MAX_GRASS_CARS],
            // z: REMASTER Ultra, the shells hand over to the blades near the camera; w: pixels per
            // metre at 1 m depth (the blades' pixel floor, grass.wgsl `BLADE_MIN_PX`)
            grass_n: [0.0, s.grass_edge.map_or(0.0, |e| e.1), v.grass_blades() as u32 as f32, proj_still.y_axis.y * f.viewport.map_or(size.1 as f32, |vp| vp[3]) * 0.5],
            grass_map: s.grass_edge.map_or([0.0; 4], |e| e.0),
            sky_sh: sky_sh(v, s, l),
            local: match s.local.origin {
                Some(_) if v.local_light() => [s.local.shown.map_or(1.0, |t| (t.elapsed().as_secs_f32() / LOCAL_FADE).min(1.0)), v.remaster.local_tune.bounce, v.remaster.local_tune.occlusion, 0.0],
                _ => [0.0; 4],
            },
            local_box: s.local.origin.map_or([0.0; 4], |o| [o[0] as f32, o[1] as f32, o[2] as f32, 0.0]),
            holo_rects: f.holo_clip.map_or([[0.0; 4]; HOLO_RECTS], |c| c.rects.map(|(c, h)| [c[0], c[1], h[0], h[1]])),
            holo_n: f.holo_clip.map_or([0.0; 4], |c| [c.rects_r, c.rects_n.min(HOLO_RECTS) as f32, 0.0, 0.0]),
            paint: [[0.0; 4]; 6],
            gain: [v.brightness, v.brightness, v.brightness, 0.0],
            hell: [[0.0; 4]; 16],
            tron: [0.0; 4],
            water_t: [self.clock_pin.get().unwrap_or_else(|| self.water_clock.elapsed().as_secs_f32()), 0.0, 0.0, 0.0],
            // hologram clip (shader holo_gain): z = 1 keep inside, 2 keep outside, 3 frosted; w = gain
            myw_n: f.holo_clip.map_or([0.0; 4], |c| [0.0, 0.0, c.mode(), c.gain]),
            // REMASTER fog: density per metre from the optical depth per 100 m
            fog2: [fog_tune.density / 100.0, l.ground_y, 1.0 / fog_tune.height.max(1.0), fog_tune.glow * (1.0 - l.darkness)],
            // y/z: sky dome turn (rad), exposure (scale; 0 = no dome in this view)
            fog3: [
                fog_tune.anisotropy.clamp(0.0, 0.95),
                dome_sun(v, s).map_or(0.0, |d| s.look.sun_dir[0].atan2(-s.look.sun_dir[2]) - d.sun_azimuth + (v.remaster.sky_tune.yaw + d.yaw).to_radians()),
                dome_sun(v, s).map_or(0.0, |d| (v.remaster.sky_tune.exposure + d.exposure).exp2()),
                0.0,
            ],
            fog4: v4(fog_colour(v, s, l, &fog_tune), fog_tune.sky_haze),
            fog5: [fog_tune.start, fog_tune.full.max(fog_tune.start + 1.0), fog_tune.opacity.clamp(0.0, 1.0), fog_tune.curve.max(0.1)],
        };
        if let Some(c) = ceiling {
            ceiling_globals(&mut g, c, cam, v, f.mirror);
        }
        if f.tron {
            g.env[3] = (g.env[3] as u32 | TRON_FLAG) as f32;
            // REMASTER Neon grid: the graded dome is a backdrop only, it lights nothing
            if v.mode == RenderMode::Remaster {
                g.sky_sh = [[0.0; 4]; 9];
            }
        }
        // REMASTER menu car preview: no sky ambient, probes or fog of the garage track, only the studio's light
        if f.studio {
            (g.sky_sh, g.local, g.params[3], g.fog2[0]) = ([[0.0; 4]; 9], [0.0; 4], 0.0, 0.0);
        }
        for (b, p) in s.water_params.iter().enumerate().take(MAX_WATER) {
            g.water[WATER_ROWS * b..WATER_ROWS * (b + 1)].copy_from_slice(p);
        }
        // REMASTER grass: the level scales each body's shells and reach, the range option the
        // shells' and the blades' reach (the blade hand-off and LOD follow from these rows)
        let range = v.remaster.grass_range as f32 / 100.0;
        for (b, p) in s.grass_params.iter().enumerate().take(MAX_GRASS) {
            let mut p = *p;
            p[0][3] = grass_shells(p[0][3], v.grass()) as f32;
            p[1][0] *= v.grass().factor() * range;
            p[4][2] *= range;
            g.grass[GRASS_ROWS * b..GRASS_ROWS * (b + 1)].copy_from_slice(&p);
        }
        // REMASTER grass: the car bodies in view (and the hidden followed car) press the blades
        // (grass.wgsl `grass_press`); the nearest first when more than fit
        if !s.grass_params.is_empty() && v.grass() != GrassLevel::Off {
            let mut bodies: Vec<&(usize, Mat4)> = f.cars.iter().chain(f.light_cars).filter(|(m, _)| m.is_multiple_of(CAR_MESHES)).collect();
            bodies.sort_by(|a, b| a.1.w_axis.truncate().distance(cam.pos).total_cmp(&b.1.w_axis.truncate().distance(cam.pos)));
            for (k, (mesh, m)) in bodies.iter().take(MAX_GRASS_CARS).enumerate() {
                let motion = f.car_motion.get(mesh / CAR_MESHES).copied().unwrap_or(CarMotion { vel: Vec3::ZERO, half: GRASS_CAR_HALF });
                let pos = m.w_axis.truncate();
                let fwd = -m.z_axis.truncate().normalize_or_zero();
                g.grass_cars[3 * k] = [pos.x, pos.y, pos.z, motion.half[0]];
                g.grass_cars[3 * k + 1] = [fwd.x, fwd.z, motion.half[1], 0.0];
                g.grass_cars[3 * k + 2] = [motion.vel.x, motion.vel.z, 0.0, 0.0];
                g.grass_n[0] = (k + 1) as f32;
            }
        }
        // MYW point lights of the scene objects: the lists of the cells they stand in, the
        // lights nearest the camera first. APPROX: one list for all of them, the origin is the
        // vertex (the original takes each object's own cell and origin, 0x439134).
        if !f.overlay {
            let pl = &s.point_lights;
            let mut idx: Vec<u16> = f.props.iter().filter(|(_, m)| (m.w_axis.truncate() - cam.pos).length() <= clip_far + 50.0).flat_map(|(_, m)| pl.at(m.w_axis.truncate()).iter().copied()).collect();
            idx.sort_by(|&a, &b| pl.lights[a as usize].pos.distance(cam.pos).total_cmp(&pl.lights[b as usize].pos.distance(cam.pos)));
            idx.dedup();
            set_myw(&mut g, pl, &idx, false);
        }
        // REMASTER: dynamic lights
        if v.dynamic_lights() {
            if !f.overlay && !f.mirror && f.viewport.is_none() {
                self.spot_update(s, f, cam.pos, v.generated_cars(), &v.remaster.lamp_lights, v.remaster.light_shadows);
            }
            dynamic_lights(&mut g, s, f, cam.pos, far, v.generated_cars(), &v.remaster.car_lights, &self.head_alloc.borrow());
            street_lamps(&mut g, s, cam.pos, far, &v.remaster.lamp_lights, &self.spot_alloc.borrow());
        }
        let wire = f.wire.on() && s.wire.is_some();
        // scene objects: one lit, world-space buffer (they are small: under 1000 vertices per
        // mesh, a few hundred instances)
        let mut prop_v = Vec::new();
        // CLASSIC: each object's own light record (origin and the lights of its cell, 0x439134) and
        // the record of every vertex
        let own_lights = v.mode != RenderMode::Remaster && !wire && !f.overlay;
        let (mut prop_rec, mut prop_rec_of): (Vec<[f32; 4]>, Vec<u32>) = (Vec::new(), Vec::new());
        for &(mesh, m) in f.props {
            let Some(src) = s.props.get(mesh) else { continue };
            if (m.w_axis.truncate() - cam.pos).length() > clip_far + 50.0 {
                continue;
            }
            if own_lights {
                let origin = m.w_axis.truncate();
                let cell = s.point_lights.at(origin);
                let cell = &cell[..cell.len().min(PROP_REC_LIGHTS)];
                prop_rec_of.extend(std::iter::repeat_n((prop_rec.len() / PROP_REC) as u32, src.len()));
                prop_rec.push([origin.x, origin.y, origin.z, cell.len() as f32]);
                for k in 0..PROP_REC_LIGHTS {
                    prop_rec.extend(cell.get(k).map_or([[0.0; 4]; 2], |&i| myw_rows(&s.point_lights.lights[i as usize])));
                }
            }
            prop_v.extend(src.iter().map(|v| Vertex {
                pos: m.transform_point3(Vec3::from(v.pos)).into(),
                normal: m.transform_vector3(Vec3::from(v.normal)).normalize_or_zero().into(),
                ..*v
            }));
        }
        let prop_v = if wire { wire::flat_edges(&prop_v, WIRE_PROP) } else { prop_v };
        let own_lights = own_lights && prop_rec.len() <= PROP_RECS * PROP_REC && prop_rec_of.len() <= PROP_VERTS && !prop_rec_of.is_empty();
        if own_lights {
            self.queue.write_buffer(&self.prop_lights, 0, bytemuck::cast_slice(&prop_rec));
            self.queue.write_buffer(&self.prop_of, 0, bytemuck::cast_slice(&prop_rec_of));
        }
        let debug_buf = |v: &[Vertex]| {
            (!v.is_empty()).then(|| self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some("debug lines"), contents: bytemuck::cast_slice(v), usage: wgpu::BufferUsages::VERTEX }))
        };
        let (debug_lines, debug_xray, debug_tris) = (debug_buf(f.debug_lines), debug_buf(f.debug_xray), debug_buf(f.debug_xray_tris));
        let props = (!prop_v.is_empty()).then(|| {
            self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("props"),
                contents: bytemuck::cast_slice(&prop_v),
                usage: wgpu::BufferUsages::VERTEX,
            })
        });
        // REMASTER: which generated cars are far enough for their original model; every pass below reads it
        self.far_cars(cars, cam, v, f, size.0 as f32);
        // the probe is drawn (and submitted) before this view's uniforms are written
        let probe = f.env_probe.as_ref().filter(|_| v.enhanced() && !wire);
        if let Some((centre, skip)) = probe {
            self.draw_probe(s, g, v, f, *centre, skip, props.as_ref().map(|b| (b, prop_v.len() as u32)), far);
        }
        // REMASTER: the full view in linear light into the HDR target (the probe, mirror and
        // overlays stay in gamma space: they draw into 8-bit targets)
        let tex = target.texture();
        let main = !f.overlay && !f.mirror && f.viewport.is_none();
        let remaster = v.mode == RenderMode::Remaster;
        // CLASSIC car detail levels of the main list and the ghost's (one screen width for every view, the
        // mirror's too: the original's `0x4ec18c`); the garage and the wireframe keep OBJ0
        let lods = !remaster && !wire && !f.overlay;
        let width = size.0 as f32;
        let floor = f.cheat_level.unwrap_or(0);
        let car_lv = lods.then(|| car_levels(cars, cam, v.car_detail, width, floor));
        let ghost_lv = lods.then(|| car_levels(f.ghost_cars, cam, v.car_detail, width, floor));
        // REMASTER velocity blur reads the resolved scene, so it needs the post pass's target
        let vblur = main && v.velocity_blur() && !wire;
        // REMASTER ambient occlusion splits the main pass after the opaque meshes
        let ssao = main && v.ssao() && !wire && !f.hologram;
        // REMASTER SSAA draws the scene bigger and filters it into the post pass's scene
        let scaled = remaster && main && v.remaster.render_scale() > 1.0;
        // REMASTER water reads a copy of the opaque scene and its depth (16-water)
        let water_fx = remaster && main && !wire && !f.hologram && v.water() && s.water.is_some();
        let post = remaster && main && (v.remaster.post_pass() || vblur || ssao || scaled || water_fx) && tex.format() == self.color_format;
        let render = if post && scaled { self.render_size(tex.size(), v.remaster.render_scale()) } else { tex.size() };
        let vblur = vblur && post;
        let ssao = ssao && post;
        // REMASTER the velocity pass also feeds TAA (its history reprojection)
        let motion = vblur || taa;
        let hdr = post && v.remaster.hdr;
        // REMASTER volumetric fog: applied to the opaque scene's depth in linear light
        let fogvol = main && v.fog_volume() && !wire && !f.hologram && hdr;
        let water_fx = water_fx && post;
        // REMASTER FXAA runs on the tonemapped image, so the post pass writes a texture first
        let post_aa = if post { v.remaster.post_aa.kind } else { PostAaKind::Off };
        // REMASTER car shadows: the sun's depth map of this frame's cars (the rear mirror reuses
        // it); the Classic projected shadows are not sent in this case (`View::car_shadows`)
        let real_shadows = v.car_shadows() && !wire && !f.overlay;
        if main {
            // REMASTER: dim tracks cast straight down (OVERHEAD_DARKNESS)
            let overhead = s.look.darkness > OVERHEAD_DARKNESS;
            let sun = if overhead { Vec3::Y } else { Vec3::from(l.sun_dir) };
            self.shadow_ready.set(real_shadows && self.draw_shadow_map(enc, s, cars, cam.pos, (sun, overhead), v.generated_cars()));
        }
        if real_shadows && (main || f.mirror) && self.shadow_ready.get() {
            g.env[3] = (g.env[3] as u32 | SHADOW_FLAG) as f32;
        }
        // REMASTER lamp shadows: this frame's maps of the street lamps (the mirror reuses them)
        if main {
            self.spot_size.set(v.remaster.shadow_quality.atlas());
            self.spot_ready.set(v.dynamic_lights() && !wire && !f.overlay && self.draw_spot_shadows(enc, s, f, v.generated_cars(), &v.remaster.lamp_lights, v.remaster.shadow_quality));
        }
        if remaster && (main || f.mirror) && self.spot_ready.get() {
            g.env[3] = (g.env[3] as u32 | SPOT_FLAG) as f32;
        }
        // REMASTER volumetric fog sun shafts: the world seen from the sun (the froxel injection reads it)
        let shafts = (fogvol && !v.debug.hide_shafts && (g.fog2[3] > 0.0 || fog_tune.shafts > 0.0))
            .then(|| self.draw_shaft_map(enc, s, v, cars, props.as_ref().map(|b| (b, prop_v.len() as u32)), (proj_still * view).inverse(), cam.forward(), cam.pos, far.min(SHAFT_RANGE), Vec3::from(l.sun_dir), v.generated_cars()))
            .flatten();
        // REMASTER: with bloom, smooth halos shrink to a hot core and the bloom spreads it
        let halo_bloom = hdr && v.remaster.halo_bloom();
        if hdr {
            g.env[3] = (g.env[3] as u32 | LINEAR_FLAG | if halo_bloom { HALO_BLOOM_FLAG } else { 0 }) as f32;
        }
        if fogvol {
            g.env[3] = (g.env[3] as u32 | VOLFOG_FLAG) as f32;
        }
        // REMASTER Neon grid (#320): the edge width in pixels of the target the scene is drawn into
        if f.tron {
            g.tron[0] = 0.5 * NEON_EDGE_PX * render.height as f32 / NEON_REF_H;
        }
        let halo_size = if halo_bloom { REMASTER_HALO_SIZE } else { HALO_SIZE };
        // the starburst keeps its size and strength with and without bloom
        let burst = if remaster && main && v.remaster.smooth_halos { HALO_BURST_SIZE * REMASTER_HALO_SIZE / halo_size } else { 0.0 };
        let spin = HALO_SPIN;
        // Enhanced needs this frame's cube: views without a probe of their own (the menu car)
        // fall back to Reflection, the rear mirror reuses the main view's
        // (CLASSIC: Reflection is the sphere-mapped env layer, Gouraud none)
        // REMASTER menu car preview: the studio cube (Enhanced) or none, never the garage track's env layer
        let studio_cube = f.studio && v.enhanced() && !wire;
        if studio_cube {
            self.draw_studio_cube();
        }
        (g.env[0], g.env[1]) = match (v.enhanced(), v.car_shading, l.env_layer) {
            (true, _, _) if probe.is_some() || f.mirror || studio_cube => (2.0, 0.0),
            _ if f.studio => (0.0, 0.0),
            (true, _, Some(e)) | (_, CarShading::Reflection, Some(e)) => (1.0, e as f32),
            _ => (0.0, 0.0),
        };
        // CLASSIC: the env-mapped scene objects (`PROP_ENV`) read the layer whatever the car shading
        if let Some(e) = l.env_layer {
            g.env[1] = e as f32;
        }
        // REMASTER water: the scene copy is there (y: 1, 2 = with screen-space reflections), and
        // the views' depth range
        let copy = if water_fx { 1.0 + v.water_ssr() as u32 as f32 } else { 0.0 };
        g.water_t[1..].copy_from_slice(&[copy, NEAR, clip_far]);
        // REMASTER volumetric fog: the volume's far distance, for the surface's lookup (the volume's depth
        // range is `far`, not the clip plane, in motorview's unlimited mode)
        if fogvol {
            g.fog3[3] = far;
        }
        self.queue.write_buffer(&self.uniforms, 0, bytemuck::bytes_of(&g));
        g.ambient[3] = 1.0;
        // CLASSIC: 2 = the scene objects read their own light records (set_myw below resets it)
        if own_lights {
            g.myw_n[1] = 2.0;
        }
        self.queue.write_buffer(&self.prop_uniform, 0, bytemuck::bytes_of(&g));
        for (slot, &(mesh, m)) in cars.iter().enumerate().take(MAX_CARS) {
            g.model = m.to_cols_array_2d();
            g.spot_n[2] = wheel_spin(f, mesh);
            // CLASSIC: the lights of the cell the car (or wheel) stands in (0x439134)
            set_myw(&mut g, &s.point_lights, if f.overlay { &[] } else { s.point_lights.at(m.w_axis.truncate()) }, true);
            // REMASTER hologram clip rides in myw_n.zw (set_myw cleared it)
            if let Some(c) = f.holo_clip {
                (g.myw_n[2], g.myw_n[3]) = (c.mode(), c.gain);
            }
            // REMASTER live paint: the bodies only (the wheels are not painted, `scene::repaint`)
            g.paint = match (f.paint.filter(|_| mesh.is_multiple_of(CAR_MESHES)), s.car_colours.get(mesh / CAR_MESHES)) {
                (Some(p), Some(&own)) => paint_rows(&p, own, s.object_scale),
                _ => [[0.0; 4]; 6],
            };
            self.queue.write_buffer(&self.car_uniforms[slot], 0, bytemuck::bytes_of(&g));
        }
        // the ghost's uniforms follow the cars'
        let ghost_at = cars.len().min(MAX_CARS);
        for (k, &(mesh, m)) in f.ghost_cars.iter().enumerate().take(MAX_CARS - ghost_at) {
            g.model = m.to_cols_array_2d();
            g.spot_n[2] = wheel_spin(f, mesh);
            set_myw(&mut g, &s.point_lights, s.point_lights.at(m.w_axis.truncate()), true);
            self.queue.write_buffer(&self.car_uniforms[ghost_at + k], 0, bytemuck::bytes_of(&g));
        }
        // CLASSIC, CHEAT supercars / grem: the sky is a black quad (0x443a99)
        // CHEAT tribute to tron: the sky band list is filled black (0x48e964), the same as a black backdrop
        // REMASTER Neon grid: the graded dome when there is one
        let backdrop = v.debug.backdrop.or(((f.cheat_level.is_some() && !remaster) || (f.tron && dome_sun(v, s).is_none())).then_some([0, 0, 0]));
        let bottom = backdrop.map_or(l.sky_bottom.map(|c| c as f64), |c| c.map(|x| x as f64 / 255.0));
        let mut halo_v = Vec::new();
        let mut car_v = Vec::new();
        if v.halos {
            let fade_start = (l.fog_start + (1.0 - l.fog_start) * 0.85) * far;
            let fx = 2.0 * ((v.fov_x.to_radians() / 2.0).tan() / cam.zoom).atan();
            if remaster {
                halo_quads(&mut halo_v, &s.world_halos, Mat4::IDENTITY, Vec3::splat(WORLD_HALO_BRIGHTNESS), HALO_GAIN, cam, fx, far, fade_start, far, halo_size, burst, 1.0, spin, None);
            } else {
                // CLASSIC 0x439077: every world halo grey, size 1.0, the u + 0.75 curve
                for h in s.world_halos.iter().map(classic_world_halo) {
                    halo_quads(&mut halo_v, &[h.halo], Mat4::IDENTITY, Vec3::ONE, h.gain, cam, fx, far, fade_start, far, h.size, 0.0, 1.0, 0.0, None);
                }
            }
            for &(mesh, m) in cars.iter().take(MAX_CARS) {
                let Some(h) = s.halos(mesh, v.generated_cars()) else { continue };
                let brake = v.remaster.car_lights.brake(f, mesh);
                let night = if v.dynamic_lights() { s.look.darkness } else { 0.0 };
                if remaster && !h.is_empty() {
                    // REMASTER car lights (`CarLightTune`): the front halos (view z < 0) glare
                    // inside the beam, the rear ones (view z > 0) brighten and burst when
                    // braking; each kind has its own look and is not dimmed with the rest
                    let (rear, front): (Vec<HaloInst>, Vec<HaloInst>) = h.iter().partition(|h| h.p0.z + h.p1.z > 0.0);
                    let lamp = front.iter().map(|h| (h.p0 + h.p1) * 0.5).sum::<Vec3>() / front.len().max(1) as f32;
                    let to_eye = (cam.pos - m.transform_point3(lamp)).normalize_or_zero();
                    let facing = m.transform_vector3(Vec3::NEG_Z).normalize_or_zero().dot(to_eye).max(0.0).powi(4);
                    let t = &v.remaster.car_lights;
                    for (lamps, lt, facing) in [(&front, &t.head, facing), (&rear, &t.tail, 0.0)] {
                        let glow = Vec3::splat(CAR_HALO_BRIGHTNESS * lt.halo_gain(night, facing, brake) * lt.scale);
                        let size = halo_size * lt.halo_size(night, facing, brake);
                        halo_quads(&mut car_v, lamps, m, glow, HALO_GAIN, cam, fx, far, fade_start, far, size, burst * lt.burst_size(brake), lt.burst_gain(brake), spin, v.remaster.smooth_halos.then_some([lt.strip, lt.strip_width]));
                    }
                } else if remaster {
                    halo_quads(&mut halo_v, h, m, Vec3::splat(CAR_HALO_BRIGHTNESS), HALO_GAIN, cam, fx, far, fade_start, far, halo_size, burst, 1.0, spin, None);
                } else {
                    // CLASSIC 0x439134: head lights, tail lights (brake boost) and others by texture
                    let braking = f.car_brake.get(mesh / CAR_MESHES).is_some_and(|&b| b > 0.0);
                    for k in h.iter().map(|h| classic_car_halo(h, braking)) {
                        halo_quads(&mut halo_v, &[k.halo], m, Vec3::ONE, k.gain, cam, fx, far, fade_start, far, k.size, 0.0, 1.0, 0.0, None);
                    }
                }
            }
            // scene objects: blinkers and flashing cones pulse their halos through the scale
            for (i, &(mesh, m)) in f.props.iter().enumerate() {
                if let Some(h) = s.prop_halos.get(mesh) {
                    // CLASSIC 0x4398f5: gain 0.6, size 0.5, the object's animated scale on the colour
                    let glow = f.prop_glow.get(i).copied().unwrap_or(Vec3::ONE) * if remaster { WORLD_HALO_BRIGHTNESS } else { 1.0 };
                    halo_quads(&mut halo_v, h, m, glow, HALO_GAIN, cam, fx, far, fade_start, far, halo_size, burst, 1.0, spin, None);
                }
                // REMASTER headlamps (`PROP_HEADLAMPS`): glare inside the beam like a car's headlights
                if let Some(pl) = s.prop_lamps.get(mesh).filter(|p| remaster && !p.lamps.is_empty()) {
                    let night = if v.dynamic_lights() { s.look.darkness } else { 0.0 };
                    let lamp = pl.lamps.iter().map(|h| (h.p0 + h.p1) * 0.5).sum::<Vec3>() / pl.lamps.len() as f32;
                    let to_eye = (cam.pos - m.transform_point3(lamp)).normalize_or_zero();
                    let facing = m.transform_vector3(pl.forward).normalize_or_zero().dot(to_eye).max(0.0).powi(4);
                    let head = &v.remaster.car_lights.head;
                    let glow = Vec3::splat(CAR_HALO_BRIGHTNESS * head.halo_gain(night, facing, 0.0) * head.scale);
                    let size = halo_size * head.halo_size(night, facing, 0.0);
                    halo_quads(&mut car_v, &pl.lamps, m, glow, HALO_GAIN, cam, fx, far, fade_start, far, size, burst * head.burst_size(0.0), head.burst_gain(0.0), spin, None);
                }
            }
        }
        // REMASTER: halos and lens flares at half strength (overblown at the Classic values)
        let dim = |q: &mut [Vertex]| {
            if remaster {
                q.iter_mut().for_each(|x| x.color[..3].iter_mut().for_each(|c| *c *= REMASTER_HALO_SCALE));
            }
        };
        dim(&mut halo_v);
        // CLASSIC: the D3D vertex colour is a byte (0x442894), so a halo never exceeds the texture
        if !remaster {
            halo_v.iter_mut().chain(car_v.iter_mut()).for_each(|x| x.color[..3].iter_mut().for_each(|c| *c = c.min(1.0)));
        }
        halo_v.extend(car_v);
        let (add_v, smoke_v, spark_v) = sprite_quads(f.sprites, cam, remaster, (fov_x / 2.0).tan());
        halo_v.extend(add_v);
        // lens flares: own vertices, drawn after the halos once `flare_vis_pass` has scaled them
        let (mut flare_v, mut flare_lights) = (Vec::new(), Vec::new());
        if v.halos && v.lens_flares && !(f.mirror || f.overlay) && !wire {
            let fade_start = (l.fog_start + (1.0 - l.fog_start) * 0.85) * far;
            let vp = f.viewport.unwrap_or([0.0, 0.0, size.0 as f32, size.1 as f32]);
            let target = (size.0 as f32, size.1 as f32);
            if remaster && main && v.remaster.smooth_flares {
                // REMASTER: computed flares from lamps, car lights and blinkers
                let srcs = self.flare_sources(s, f, cam, v);
                flare_quads_proc(&mut flare_v, &mut flare_lights, &srcs, cam, proj_still * view, vp, target, (fov_x, fov), far, fade_start);
            } else if let Some(layer) = s.fx.flare {
                // CLASSIC: hex.iff flares of the world halos (0x439077) and of the car head lights (0x4396aa)
                let world: Vec<HaloInst> = s.world_halos.iter().map(|h| classic_world_halo(h).halo).collect();
                flare_quads(&mut flare_v, &mut flare_lights, &world, Mat4::IDENTITY, WORLD_FLARE, layer as f32, cam, proj_still * view, vp, target, (fov_x, fov), far, fade_start);
                for &(mesh, m) in cars.iter().take(MAX_CARS) {
                    let Some(h) = s.halos(mesh, v.generated_cars()) else { continue };
                    let heads: Vec<HaloInst> = h.iter().filter(|h| h.texture == 0).map(|h| classic_car_halo(h, false).halo).collect();
                    flare_quads(&mut flare_v, &mut flare_lights, &heads, m, HEAD_FLARE, layer as f32, cam, proj_still * view, vp, target, (fov_x, fov), far, fade_start);
                }
            }
            dim(&mut flare_v);
        }
        let flares = (!flare_v.is_empty()).then(|| {
            self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("lens flares"),
                contents: bytemuck::cast_slice(&flare_v),
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::STORAGE,
            })
        });
        let smoke = (!smoke_v.is_empty()).then(|| {
            self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("smoke"),
                contents: bytemuck::cast_slice(&smoke_v),
                usage: wgpu::BufferUsages::VERTEX,
            })
        });
        let sparks = (!spark_v.is_empty()).then(|| {
            self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("sparks"),
                contents: bytemuck::cast_slice(&spark_v),
                usage: wgpu::BufferUsages::VERTEX,
            })
        });
        let halos = (!halo_v.is_empty()).then(|| {
            self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("halos"),
                contents: bytemuck::cast_slice(&halo_v),
                usage: wgpu::BufferUsages::VERTEX,
            })
        });
        let shadows = (!f.shadows.is_empty() && !wire).then(|| {
            self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("shadows"),
                contents: bytemuck::cast_slice(f.shadows),
                usage: wgpu::BufferUsages::VERTEX,
            })
        });
        // CHEAT tribute to tron, REMASTER Neon grid (#320): the light walls
        let walls = (remaster && f.tron && !f.light_walls.is_empty() && !wire && !f.overlay).then(|| {
            self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("light walls"),
                contents: bytemuck::cast_slice(f.light_walls),
                usage: wgpu::BufferUsages::VERTEX,
            })
        });
        let skids = (!f.skids.is_empty() && !wire).then(|| {
            self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("skid marks"),
                contents: bytemuck::cast_slice(f.skids),
                usage: wgpu::BufferUsages::VERTEX,
            })
        });
        // MotionBlur needs the whole target, copyable into the history texture
        let blur = main && v.motion_blur && !v.velocity_blur() && tex.usage().contains(wgpu::TextureUsages::COPY_SRC) && tex.format() == self.color_format;
        // REMASTER: own pipelines; the full view is multisampled and post-processed through
        // own targets
        let studio_aa = self.studio_aa.get();
        let samples = if remaster && main {
            self.msaa_samples(v.remaster.msaa())
        } else if studio_aa {
            // REMASTER menu car preview: its offscreen frame (`draw_studio_aa`)
            self.msaa_samples(STUDIO_MSAA)
        } else {
            1
        };
        let pipes = if studio_aa { self.studio_pipes(samples) } else if remaster { self.remaster_pipes(samples, hdr) } else { &self.classic };
        let tg = (samples > 1 || post).then(|| self.targets(tex.size(), render, samples, post, hdr, motion, ssao, fogvol, post_aa, water_fx));
        let scene_out = tg.as_ref().and_then(|t| t.scene.as_ref());
        // the scene as drawn: the SSAA one when there is one
        let work = tg.as_ref().and_then(|t| t.hi.as_ref()).or(scene_out);
        let scene_view = work.map_or(target, |s| &s.1);
        let (color_view, resolve, depth) = match tg.as_ref().and_then(|t| t.msaa.as_ref()) {
            Some((c, z)) => (c, Some(scene_view), z),
            None => (scene_view, None, tg.as_ref().and_then(|t| t.depth.as_ref()).unwrap_or(depth)),
        };
        if main && !motion {
            *self.prev_view.borrow_mut() = None;
        }
        if main && !taa {
            self.taa_reset();
        }
        // REMASTER volumetric fog: a main frame without the volume (F9, another fog style) leaves its history stale
        if main && !fogvol {
            if let Some(vf) = self.volfog.get() {
                vf.reset();
            }
        }
        let mut history = self.history.borrow_mut();
        if blur {
            // (REMASTER HDR: the history is linear like the scene it copies)
            let format = if hdr { HDR } else { self.color_format };
            let fits = history.as_ref().is_some_and(|h| h.tex.size() == render && h.tex.format() == format);
            if !fits {
                *history = Some(self.history_target(render, format));
            }
        } else if main {
            *history = None;
        }
        let bottom = if hdr { bottom.map(|c| c.max(0.0).powf(2.2)) } else { bottom };
        // REMASTER grass trail (#158): the pressed-grass texture up to this frame's sim time,
        // before the grass shells read it (the main view only; nothing with the grass Off)
        if main && v.grass() != GrassLevel::Off && s.grass.is_some() {
            self.trail_pass(enc, s, f);
            // REMASTER Ultra: place this frame's blades (reads the trail and the globals)
            if v.grass_blades() {
                self.blade_pass(enc, s);
            }
        }
        // the main pass is split after the opaque meshes for the passes that read its depth
        // (lens flares: `flare_vis_pass`)
        let split = ssao || fogvol || water_fx || flares.is_some();
        // the view's rectangle in the target (the main pass and, after a split, the overlays)
        let rect = match f.viewport {
            Some(vp) => {
                let (x, y) = (vp[0].clamp(0.0, size.0 as f32), vp[1].clamp(0.0, size.1 as f32));
                let (w, h) = (vp[2].min(size.0 as f32 - x), vp[3].min(size.1 as f32 - y));
                if w < 1.0 || h < 1.0 {
                    return;
                }
                Some((x, y, w, h))
            }
            None => None,
        };
        let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("main"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: color_view,
                depth_slice: None,
                // (REMASTER water: resolved after the opaque meshes too, for its scene copy)
                resolve_target: resolve.filter(|_| !split || water_fx),
                ops: wgpu::Operations {
                    load: if studio_aa {
                        // REMASTER: the offscreen preview starts transparent, the car's coverage is its alpha
                        wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT)
                    } else if f.overlay || f.mirror {
                        wgpu::LoadOp::Load
                    } else {
                        wgpu::LoadOp::Clear(wgpu::Color { r: bottom[0], g: bottom[1], b: bottom[2], a: 1.0 })
                    },
                    store: if resolve.is_some() && !split { wgpu::StoreOp::Discard } else { wgpu::StoreOp::Store },
                },
            })],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: depth,
                depth_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Clear(1.0), store: if motion || split { wgpu::StoreOp::Store } else { wgpu::StoreOp::Discard } }),
                stencil_ops: None,
            }),
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        if let Some((x, y, w, h)) = rect {
            pass.set_viewport(x, y, w, h, 0.0, 1.0);
            pass.set_scissor_rect(x as u32, y as u32, w as u32, h as u32);
        }
        pass.set_bind_group(0, &s.bind, &[]);
        if remaster {
            pass.set_bind_group(1, self.aniso_bind(v.remaster.anisotropy), &[]);
            pass.set_bind_group(2, self.shadow_read(), &[]);
        }
        if f.hologram {
            // REMASTER hologram: the road, then per car the shell and the edges, additive
            if let Some((vb, n)) = s.holo_road.as_ref().filter(|_| f.holo_road > 0.0) {
                pass.set_bind_group(0, &s.bind, &[]);
                pass.set_pipeline(&pipes.holo);
                pass.set_vertex_buffer(0, vb.slice(..));
                pass.draw(0..*n, 0..1);
            }
            for (slot, &(mesh, _)) in cars.iter().enumerate().take(MAX_CARS) {
                pass.set_bind_group(0, &s.car_binds[slot], &[]);
                // Car models: Generated shows the procedural body (the Titan's so far) where there is one; the
                // wheels are the procedural ones too
                let generated = v.generated_cars();
                let wires = s.holo_wires(mesh, generated);
                for (pipe, buf) in [(&pipes.holo_fill, s.car(mesh, generated)), (&pipes.holo, wires)] {
                    let Some((vb, n)) = buf else { continue };
                    pass.set_pipeline(pipe);
                    pass.set_vertex_buffer(0, vb.slice(..));
                    pass.draw(0..*n, 0..1);
                }
            }
            return;
        }
        if !f.overlay && !v.debug.hide_sky && v.debug.backdrop.is_none() && !(f.cheat_level.is_some() && v.mode == RenderMode::Classic) && !(f.tron && dome_sun(v, s).is_none()) {
            if ceiling.is_some() {
                // CHEAT lemmy / ace (#316), ramlosa / h2o (#317) (0x443570): the cloud ceiling replaces the sky, in both modes
                pass.set_pipeline(&pipes.hell_sky);
                pass.draw(0..12, 0..1);
            } else {
                // REMASTER: the HDRI dome in place of the sky band (the motion blur below still blends over it)
                let dome = pipes.sky_dome.as_ref().filter(|_| dome_sun(v, s).is_some());
                pass.set_pipeline(dome.unwrap_or(&pipes.sky));
                pass.draw(0..3, 0..1);
            }
        }
        if let Some(h) = history.as_ref().filter(|h| blur && h.valid) {
            // sky * (1 - a) + previous frame * a, the same as blending the sky over the old frame
            pass.set_pipeline(&pipes.blur);
            pass.set_bind_group(0, &h.bind, &[]);
            pass.set_blend_constant(wgpu::Color { r: MOTION_BLUR_ALPHA, g: MOTION_BLUR_ALPHA, b: MOTION_BLUR_ALPHA, a: 1.0 });
            pass.draw(0..3, 0..1);
            pass.set_bind_group(0, &s.bind, &[]);
        }
        let mesh_pipe = match &s.wire {
            Some(w) if wire => {
                if !(v.debug.hide_world || f.cars_only) {
                    let b = w.bufs(f.wire);
                    if f.wire.fill() {
                        pass.set_pipeline(&pipes.wire_fill);
                        pass.set_vertex_buffer(0, b.fill.0.slice(..));
                        pass.draw(0..b.fill.1, 0..1);
                    }
                    pass.set_pipeline(&pipes.wire);
                    pass.set_vertex_buffer(0, b.lines.0.slice(..));
                    pass.draw(0..b.lines.1, 0..1);
                }
                pass.set_pipeline(&pipes.wire);
                &pipes.wire
            }
            _ => {
                let tri_pipe = if f.mirror { &pipes.tri_mirror } else { &pipes.tri };
                pass.set_pipeline(tri_pipe);
                if !(v.debug.hide_world || f.cars_only) {
                    // REMASTER: the world minus the triangles that the water surface replaces
                    let (b, n) = s.world(v);
                    pass.set_vertex_buffer(0, b.slice(..));
                    pass.draw(0..n, 0..1);
                    // REMASTER world fill: same pipeline, so fog, SSAO, shadows and lights apply
                    if let Some((b, n)) = s.fill.as_ref().filter(|_| v.world_fill()) {
                        pass.set_vertex_buffer(0, b.slice(..));
                        pass.draw(0..*n, 0..1);
                    }
                    // REMASTER grass shells over the grass polygons, one instance per shell (the
                    // most of the bodies; `vs_grass` drops the rest): the main view only, not the
                    // mirror, the viewports or the env probe
                    let shells = s.grass_params.iter().map(|p| grass_shells(p[0][3], v.grass())).max().unwrap_or(0);
                    if let (Some((b, n)), Some(gp)) = (s.grass.as_ref().filter(|_| main && shells > 0), pipes.grass.as_ref()) {
                        pass.set_pipeline(gp);
                        pass.set_bind_group(3, &s.grass_bind, &[]);
                        pass.set_vertex_buffer(0, b.slice(..));
                        pass.draw(0..*n, 0..shells);
                        pass.set_pipeline(tri_pipe);
                    }
                    // REMASTER Ultra grass blades: the records `blade_pass` placed, as many
                    // instances as it counted (one strip each), solid like the shells
                    if main && v.grass_blades() && s.grass.is_some() {
                        let bg = self.blade_gpu();
                        pass.set_pipeline(self.blade_pipe(samples, hdr));
                        pass.set_bind_group(3, &bg.read_bind, &[]);
                        pass.draw_indirect(&bg.args, 0);
                        pass.set_pipeline(tri_pipe);
                    }
                    // REMASTER water in the views without a scene copy (mirror, viewports): after the
                    // opaque world, depth tested, seen as bottomless; the main view has its own pass
                    if let (Some((b, n)), Some(wp)) = (s.water.as_ref().filter(|_| v.water() && !water_fx && samples == 1), pipes.water.as_ref()) {
                        pass.set_pipeline(wp);
                        pass.set_bind_group(3, &self.water_gpu().dummy, &[]);
                        pass.set_vertex_buffer(0, b.slice(..));
                        pass.draw(0..*n, 0..1);
                        pass.set_pipeline(tri_pipe);
                    }
                }
                tri_pipe
            }
        };
        let generated = v.generated_cars() && !wire;
        if let Some(b) = &shadows {
            pass.set_pipeline(&pipes.shadow);
            pass.set_vertex_buffer(0, b.slice(..));
            pass.draw(0..f.shadows.len() as u32, 0..1);
            pass.set_pipeline(mesh_pipe);
        }
        if let Some(b) = &skids {
            // REMASTER detailed marks, or the Classic strips through the shadow pipeline
            // CLASSIC: flat quads multiplying the frame (0x442911), z bias like the shadows
            pass.set_pipeline(pipes.skid.as_ref().filter(|_| v.skids_detailed()).unwrap_or(&pipes.skid_flat));
            pass.set_vertex_buffer(0, b.slice(..));
            pass.draw(0..f.skids.len() as u32, 0..1);
            pass.set_pipeline(mesh_pipe);
        }
        if let Some(b) = &props {
            pass.set_bind_group(0, &s.prop_bind, &[]);
            pass.set_vertex_buffer(0, b.slice(..));
            pass.draw(0..prop_v.len() as u32, 0..1);
        }
        for (slot, &(mesh, _)) in cars.iter().enumerate().take(MAX_CARS) {
            if wire {
                // the wireframe shows the model the view draws (generated bodies too)
                let Some(w) = s.car_wire(mesh, v.generated_cars()) else { continue };
                let b = w.bufs(f.wire);
                pass.set_bind_group(0, &s.car_binds[slot], &[]);
                if f.wire.fill() {
                    pass.set_pipeline(&pipes.wire_fill);
                    pass.set_vertex_buffer(0, b.fill.0.slice(..));
                    pass.draw(0..b.fill.1, 0..1);
                }
                pass.set_pipeline(&pipes.wire);
                pass.set_vertex_buffer(0, b.lines.0.slice(..));
                pass.draw(0..b.lines.1, 0..1);
                continue;
            }
            // CLASSIC: the body level by distance, wheels left out where the level has them built in
            let level = match car_lv.as_ref() {
                Some(lv) => match lv[slot] {
                    Some(level) => level,
                    None => continue,
                },
                None => 0,
            };
            // REMASTER: a far generated car takes its original model (`far_cars`)
            let Some((vb, n)) = (if level == 0 { s.car_lod(mesh, generated, self.is_far(mesh)) } else { s.car_level(mesh, level, generated) }) else { continue };
            pass.set_bind_group(0, &s.car_binds[slot], &[]);
            pass.set_vertex_buffer(0, vb.slice(..));
            pass.draw(0..*n, 0..1);
        }
        // DEBUG Check style: the non-manifold edges, seen through the bodies
        if wire && f.wire == WireStyle::Check {
            pass.set_pipeline(&pipes.wire_xray);
            for (slot, &(mesh, _)) in cars.iter().enumerate().take(MAX_CARS) {
                let Some(b) = s.car_wire(mesh, v.generated_cars()).and_then(|w| w.check.as_ref()).filter(|b| b.xray.1 > 0) else { continue };
                pass.set_bind_group(0, &s.car_binds[slot], &[]);
                pass.set_vertex_buffer(0, b.xray.0.slice(..));
                pass.draw(0..b.xray.1, 0..1);
            }
        }
        // REMASTER ambient occlusion and volumetric fog: the opaque scene is in; darken it from its
        // depth, then fog it, and go on with the ghosts, particles and halos in a second pass
        if split {
            drop(pass);
            if let Some(b) = &flares {
                self.flare_vis_pass(enc, depth, samples > 1, b, &flare_lights, clip_far);
            }
            if let Some(st) = tg.as_ref().filter(|_| ssao).and_then(|t| t.ssao.as_ref()) {
                self.ssao_passes(enc, st, samples > 1, &proj_still, clip_far, render, v.remaster.ssao, hdr);
            }
            let vol = tg.as_ref().and_then(|t| t.vol.as_ref()).filter(|_| fogvol);
            if vol.is_some() {
                use crate::volfog;
                let vf = self.volfog();
                let (inv_vp, fwd) = ((proj_still * view).inverse(), cam.forward());
                // the headlights only (kind 0): the tail lights reach 4 m, under a froxel
                let n_spots = (g.spot_n[0] as usize).min(MAX_SPOTS);
                let heads: Vec<usize> = (0..n_spots).filter(|&k| g.spots[4 * k + 1][3] < 0.5).collect();
                let mut heads_packed = [[0.0; 4]; 4 * MAX_SPOTS];
                for (j, &k) in heads.iter().enumerate() {
                    heads_packed[4 * j] = g.spots[4 * k];
                    heads_packed[4 * j + 1] = [g.spots[4 * k + 1][0], g.spots[4 * k + 1][1], g.spots[4 * k + 1][2], 0.0];
                    heads_packed[4 * j + 2] = [g.spots[4 * k + 2][0], g.spots[4 * k + 2][1], g.spots[4 * k + 2][2], 0.0];
                    heads_packed[4 * j + 3] = [g.spots[4 * k + 3][0], g.spots[4 * k + 3][1], g.spots[4 * k + 3][2], 0.0];
                }
                // the history: last frame's unjittered view-projection, this frame's jitter
                let (prev_vp, jit) = vf.step(proj_still * view, cam.pos, fwd, far);
                vf.write(
                    &self.queue,
                    &volfog::VolU {
                        inv_vp: inv_vp.to_cols_array_2d(),
                        cam: [cam.pos.x, cam.pos.y, cam.pos.z, 0.0],
                        fwd: [fwd.x, fwd.y, fwd.z, 0.0],
                        fog: v4(l.fog_colour, 0.0),
                        sun: v4(l.sun, 0.0),
                        sun_dir: v4(l.sun_dir, 0.0),
                        fog2: g.fog2,
                        fog3: g.fog3,
                        fog4: g.fog4,
                        fog5: g.fog5,
                        proj: [proj_still.x_axis.x, proj_still.y_axis.y, NEAR, clip_far],
                        vol: [volfog::NEAR, far, volfog::SLICES as f32, 0.0],
                        scr: [render.width as f32, render.height as f32, 0.0, 0.0],
                        lamp_n: [g.lamp_n[0], g.lamp_n[1], g.lamp_n[2], fog_tune.lamps],
                        lamps: g.lamps,
                        spot_n: [heads.len() as f32, 0.0, 0.0, fog_tune.lamps * volfog::HEAD_SCATTER],
                        spots: heads_packed,
                        prev_vp,
                        jit,
                        shaft_vp: shafts.map_or(Mat4::IDENTITY, |m| m.0).to_cols_array_2d(),
                        shaft: shafts.map_or([0.0; 4], |(_, depth)| [1.0, SHAFT_BIAS / depth, fog_tune.shafts, 0.0]),
                    },
                );
                vf.inject(enc);
            }
            // the passes that follow draw on the scene: the first ones keep the multisampled colour, the last resolves
            let ssao_st = tg.as_ref().filter(|_| ssao).and_then(|t| t.ssao.as_ref());
            // with the fog or the water the occlusion goes first, in a pass of its own (they read the depth)
            if let Some(st) = ssao_st.filter(|_| vol.is_some() || water_fx) {
                let mut ao = overlay_pass(enc, "ssao apply", color_view, depth, None, false, motion);
                ao.set_pipeline(self.ssao_apply(samples, hdr));
                ao.set_bind_group(0, &st.apply, &[]);
                ao.draw(0..3, 0..1);
            }
            if let Some(bind) = vol {
                let mut fog = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("fog volume apply"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment { view: color_view, depth_slice: None, resolve_target: None, ops: wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Store } })],
                    depth_stencil_attachment: None,
                    timestamp_writes: None,
                    occlusion_query_set: None,
                    multiview_mask: None,
                });
                fog.set_pipeline(self.volfog().apply_pipeline(&self.device, samples, HDR));
                fog.set_bind_group(0, bind, &[]);
                fog.draw(0..3, 0..1);
            }
            if let (true, Some(wt), Some(wp), Some((wb, wn)), Some(work)) = (water_fx, tg.as_ref().and_then(|t| t.water.as_ref()), pipes.water.as_ref(), s.water.as_ref(), work) {
                self.water_pass(enc, wt, wp, work, color_view, depth, (wb, *wn), s, v);
            }
            pass = overlay_pass(enc, "main overlays", color_view, depth, resolve, true, motion);
            if let Some(st) = ssao_st.filter(|_| vol.is_none() && !water_fx) {
                pass.set_pipeline(self.ssao_apply(samples, hdr));
                pass.set_bind_group(0, &st.apply, &[]);
                pass.draw(0..3, 0..1);
            }
            if let Some((x, y, w, h)) = rect {
                pass.set_viewport(x, y, w, h, 0.0, 1.0);
                pass.set_scissor_rect(x as u32, y as u32, w as u32, h as u32);
            }
            // the other pipelines' layouts differ: bind everything again
            pass.set_bind_group(0, &s.bind, &[]);
            if remaster {
                pass.set_bind_group(1, self.aniso_bind(v.remaster.anisotropy), &[]);
                pass.set_bind_group(2, self.shadow_read(), &[]);
            }
        }
        // Time Attack ghost: REMASTER hologram, CLASSIC translucent car
        if !wire && !f.overlay {
            let ghost_at = cars.len().min(MAX_CARS);
            for (k, &(mesh, _)) in f.ghost_cars.iter().enumerate().take(MAX_CARS - ghost_at) {
                pass.set_bind_group(0, &s.car_binds[ghost_at + k], &[]);
                if remaster {
                    // REMASTER: with the velocity blur it is drawn after it (`late_holograms`)
                    if vblur {
                        continue;
                    }
                    let wires = s.holo_wires(mesh, generated);
                    for (pipe, buf) in [(&pipes.holo_fill, s.car(mesh, generated)), (&pipes.holo, wires)] {
                        let Some((vb, n)) = buf else { continue };
                        pass.set_pipeline(pipe);
                        pass.set_vertex_buffer(0, vb.slice(..));
                        pass.draw(0..*n, 0..1);
                    }
                } else if let (Some(pipe), Some((vb, n))) = (&pipes.ghost, ghost_lv.as_ref().map_or(Some(0), |lv| lv[k]).and_then(|level| s.car_level(mesh, level, false))) {
                    // CLASSIC (design choice)
                    pass.set_pipeline(pipe);
                    pass.set_blend_constant(wgpu::Color { r: GHOST_ALPHA, g: GHOST_ALPHA, b: GHOST_ALPHA, a: 1.0 });
                    pass.set_vertex_buffer(0, vb.slice(..));
                    pass.draw(0..*n, 0..1);
                }
            }
        }
        pass.set_bind_group(0, &s.bind, &[]);
        if let (Some(b), Some(pipe)) = (&walls, pipes.light_wall.as_ref()) {
            pass.set_pipeline(pipe);
            pass.set_vertex_buffer(0, b.slice(..));
            pass.draw(0..f.light_walls.len() as u32, 0..1);
        }
        if let Some(b) = smoke.as_ref().filter(|_| !wire) {
            pass.set_pipeline(&pipes.smoke);
            pass.set_vertex_buffer(0, b.slice(..));
            pass.draw(0..smoke_v.len() as u32, 0..1);
        }
        // REMASTER sparks: with the velocity blur they are drawn after it (`velocity_blur`)
        if let Some(b) = sparks.as_ref().filter(|_| !wire && !vblur) {
            pass.set_pipeline(&pipes.spark);
            pass.set_vertex_buffer(0, b.slice(..));
            pass.draw(0..spark_v.len() as u32, 0..1);
        }
        if let Some(b) = halos.as_ref().filter(|_| !wire) {
            pass.set_pipeline(&pipes.halo);
            pass.set_vertex_buffer(0, b.slice(..));
            pass.draw(0..halo_v.len() as u32, 0..1);
        }
        if let Some(b) = &flares {
            pass.set_pipeline(&pipes.halo);
            pass.set_vertex_buffer(0, b.slice(..));
            pass.draw(0..flare_v.len() as u32, 0..1);
        }
        if v.show_line && s.n_lines > 0 {
            pass.set_pipeline(&pipes.line);
            pass.set_vertex_buffer(0, s.lines.slice(..));
            pass.draw(0..s.n_lines, 0..1);
        }
        // DEBUG: motorview gizmos (empty in the game)
        for (pipe, b, n) in [(&pipes.line, &debug_lines, f.debug_lines.len()), (&pipes.xray, &debug_xray, f.debug_xray.len()), (&pipes.xray_solid, &debug_tris, f.debug_xray_tris.len())] {
            if let Some(b) = b {
                pass.set_pipeline(pipe);
                pass.set_vertex_buffer(0, b.slice(..));
                pass.draw(0..n as u32, 0..1);
            }
        }
        drop(pass);
        // keep this frame for the next one: the scene only, before the HUD and the rear mirror
        // (the original's back buffer holds those too) [design choice]
        // (REMASTER: before the colour grade)
        if let Some(h) = history.as_mut().filter(|_| blur) {
            let src = work.map_or(tex, |s| &s.0);
            enc.copy_texture_to_texture(src.as_image_copy(), h.tex.as_image_copy(), render);
            h.valid = true;
        }
        // REMASTER: motion for the velocity blur and TAA, then the blur of the scene, before the
        // bloom and tonemap
        if let (Some(t), Some(sc)) = (tg.as_ref().filter(|_| motion), work) {
            // the car shadows: real ones are divided out of the blur by their darkening of the
            // world (shader.wgsl's sun share x SHADOW_SHARE_GAIN, up to SHADOW_WORLD), the
            // projected ones move with their cars
            let luma = |c: [f32; 3]| 0.3 * c[0] + 0.59 * c[1] + 0.11 * c[2];
            let share = luma(l.sun) / (luma(l.sun) + luma(l.object_ambient) + 1e-3);
            let dark = if s.look.darkness > OVERHEAD_DARKNESS { OVERHEAD_WORLD } else { (share * 1.5).min(0.6) };
            let shadow = if self.shadow_ready.get() && v.car_shadows() {
                VelShadow::Map(dark)
            } else if !v.car_shadows() && !f.shadows.is_empty() {
                VelShadow::Projected
            } else {
                VelShadow::None
            };
            // the ghost moves by its own motion like a car: a key offset keeps its previous
            // matrices apart from the player's (same meshes)
            let moving: Vec<(usize, Mat4)> = cars.iter().copied().chain(f.ghost_cars.iter().map(|&(m, x)| (m + VEL_GHOST_KEY, x))).collect();
            let cut = self.motion_pass(enc, s, t, sc, &moving, f.props, shadow, proj_still * view, jitter, cam, clip_far, samples, hdr, v.generated_cars(), vblur);
            if vblur {
                self.velocity_blur(enc, t, sc, samples > 1, hdr, sparks.as_ref().map(|b| (b, spark_v.len() as u32)));
                self.late_holograms(enc, sc, s, f.ghost_cars, cars.len().min(MAX_CARS), generated, hdr, v.remaster.anisotropy);
            }
            if cut {
                self.taa_reset();
            }
        }
        // REMASTER anti-aliasing globals (one buffer for both passes): output size, SSAA scale,
        // FXAA subpixel quality
        if tg.as_ref().is_some_and(|t| t.hi.is_some() || t.ldr.is_some()) {
            let a = self.aa_pipes();
            let o = tex.size();
            let scale = tg.as_ref().and_then(|t| t.hi.as_ref()).map_or(1.0, |h| h.0.size().width as f32 / o.width as f32);
            self.queue.write_buffer(&a.uniform, 0, bytemuck::cast_slice(&[o.width as f32, o.height as f32, scale, v.remaster.post_aa.params()]));
        }
        if let Some(s) = tg.as_ref().filter(|t| t.smaa.is_some()).map(|_| self.smaa_pipes()) {
            let [threshold, reach, corner] = v.remaster.post_aa.smaa_preset().1;
            self.queue.write_buffer(&s.uniform, 0, bytemuck::cast_slice(&[threshold, reach, 0.0, corner]));
        }
        // REMASTER SSAA: filter the big scene down into the output-size one (13.4 phase 2)
        if let (Some(hi), Some(out)) = (tg.as_ref().and_then(|t| t.hi.as_ref()), scene_out) {
            let a = self.aa_pipes();
            let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("ssaa down"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment { view: &out.1, depth_slice: None, resolve_target: None, ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::BLACK), store: wgpu::StoreOp::Store } })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&a.down[hdr as usize]);
            pass.set_bind_group(0, &hi.2, &[]);
            pass.draw(0..3, 0..1);
        }
        // REMASTER TAA (13.4 phase 5): resolve into the history, then the sharpened history
        // replaces the scene
        if let (Some(tt), Some(out)) = (tg.as_ref().and_then(|t| t.taa.as_ref()), scene_out) {
            let t = self.taa_pipes();
            let [w, h] = [out.0.size().width as f32, out.0.size().height as f32];
            let rs = work.map_or(out.0.size(), |s| s.0.size());
            let [hist, sharpen] = v.remaster.post_aa.taa_params();
            // the velocity blur's shutter is in the motion when it ran
            let shutter = if vblur { 1.0 / VBLUR_SHUTTER } else { 1.0 };
            let u = [w, h, rs.width as f32, rs.height as f32, hist, sharpen, tt.valid.get() as u32 as f32, shutter, hdr as u32 as f32, 0.0, 0.0, 0.0];
            self.queue.write_buffer(&t.uniform, 0, bytemuck::cast_slice(&u));
            let (read, write) = (tt.read.get(), 1 - tt.read.get());
            let mut run = |label, pipe: &wgpu::RenderPipeline, bind: &wgpu::BindGroup, dst: &wgpu::TextureView| {
                let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some(label),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment { view: dst, depth_slice: None, resolve_target: None, ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::BLACK), store: wgpu::StoreOp::Store } })],
                    depth_stencil_attachment: None,
                    timestamp_writes: None,
                    occlusion_query_set: None,
                    multiview_mask: None,
                });
                pass.set_pipeline(pipe);
                pass.set_bind_group(0, bind, &[]);
                pass.draw(0..3, 0..1);
            };
            run("taa resolve", &t.resolve[hdr as usize], &tt.resolve[read], &tt.hist[write]);
            run("taa sharpen", &t.sharpen[hdr as usize], &tt.sharpen[write], &out.1);
            tt.read.set(write);
            tt.valid.set(true);
        }
        // REMASTER: bloom chain from the HDR scene
        let r = &v.remaster;
        let bloom = tg.as_ref().and_then(|t| t.bloom.as_ref()).filter(|_| r.bloom > 0);
        if let Some((mips, down, up)) = bloom {
            let b = self.bloom_pipes();
            self.queue.write_buffer(&b.uniform, 0, bytemuck::cast_slice(&[BLOOM_THRESHOLD, BLOOM_KNEE, BLOOM_MAX, 0.0]));
            let mut run = |pipe: &wgpu::RenderPipeline, bind: &wgpu::BindGroup, dst: &wgpu::TextureView, load: wgpu::LoadOp<wgpu::Color>| {
                let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("bloom"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment { view: dst, depth_slice: None, resolve_target: None, ops: wgpu::Operations { load, store: wgpu::StoreOp::Store } })],
                    depth_stencil_attachment: None,
                    timestamp_writes: None,
                    occlusion_query_set: None,
                    multiview_mask: None,
                });
                pass.set_pipeline(pipe);
                pass.set_bind_group(0, bind, &[]);
                pass.draw(0..3, 0..1);
            };
            for (m, bind) in down.iter().enumerate() {
                run(if m == 0 { &b.first } else { &b.down }, bind, &mips[m], wgpu::LoadOp::Clear(wgpu::Color::BLACK));
            }
            for (m, bind) in up.iter().enumerate().rev() {
                run(&b.up, bind, &mips[m], wgpu::LoadOp::Load);
            }
        }
        // REMASTER: tonemap, bloom and colour grade into the target
        if let Some((_, _, bind)) = scene_out {
            let post = self.post();
            let n = self.frame_no.get().wrapping_add(1);
            self.frame_no.set(n);
            let pct = |p: u32| p.min(100) as f32 / 100.0;
            // the bloom mip sums one layer per level
            let bloom_k = bloom.map_or(0.0, |(mips, _, _)| pct(r.bloom) * BLOOM_STRENGTH / mips.len() as f32);
            let tonemap = if hdr { 1.0 + r.tonemap as u32 as f32 } else { 0.0 };
            let u = [(n % 4096) as f32, pct(r.grade), pct(r.vignette), pct(r.grain), r.exposure as f32 / 100.0, bloom_k, tonemap, 0.0];
            self.queue.write_buffer(&post.uniform, 0, bytemuck::cast_slice(&u));
            let ldr = tg.as_ref().and_then(|t| t.ldr.as_ref());
            let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("post"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: ldr.map_or(target, |l| &l.1),
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations { load: if ldr.is_some() { wgpu::LoadOp::Clear(wgpu::Color::BLACK) } else { wgpu::LoadOp::Load }, store: wgpu::StoreOp::Store },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&post.pipe);
            pass.set_bind_group(0, bind, &[]);
            pass.draw(0..3, 0..1);
            drop(pass);
            // REMASTER SMAA (13.4 phase 4): edges, blend weights, then the blend into the target
            if let Some(sm) = tg.as_ref().and_then(|t| t.smaa.as_ref()) {
                let s = self.smaa_pipes();
                let mut run = |label, pipe: &wgpu::RenderPipeline, bind: &wgpu::BindGroup, dst: &wgpu::TextureView| {
                    let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                        label: Some(label),
                        color_attachments: &[Some(wgpu::RenderPassColorAttachment { view: dst, depth_slice: None, resolve_target: None, ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::BLACK), store: wgpu::StoreOp::Store } })],
                        depth_stencil_attachment: None,
                        timestamp_writes: None,
                        occlusion_query_set: None,
                        multiview_mask: None,
                    });
                    pass.set_pipeline(pipe);
                    pass.set_bind_group(0, bind, &[]);
                    pass.draw(0..3, 0..1);
                };
                run("smaa edges", &s.edges, &sm.edges_bind, &sm.edges);
                run("smaa weights", &s.weights, &sm.weights_bind, &sm.weights);
                run("smaa blend", &s.blend, &sm.blend_bind, target);
            }
            // REMASTER FXAA (13.4 phase 3): the tonemapped image to the target
            if let Some(l) = ldr.filter(|_| v.remaster.post_aa.kind == PostAaKind::Fxaa) {
                let a = self.aa_pipes();
                let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("fxaa"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment { view: target, depth_slice: None, resolve_target: None, ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::BLACK), store: wgpu::StoreOp::Store } })],
                    depth_stencil_attachment: None,
                    timestamp_writes: None,
                    occlusion_query_set: None,
                    multiview_mask: None,
                });
                pass.set_pipeline(&a.fxaa);
                pass.set_bind_group(0, &l.2, &[]);
                pass.draw(0..3, 0..1);
            }
        }
    }

    /// REMASTER ambient occlusion: GTAO into A, then the horizontal blur into B and the vertical
    /// one back into A (the multiply reads A).
    #[allow(clippy::too_many_arguments)]
    fn ssao_passes(&self, enc: &mut wgpu::CommandEncoder, st: &SsaoTargets, msaa: bool, proj: &Mat4, far: f32, size: wgpu::Extent3d, percent: u32, hdr: bool) {
        let s = self.ssao_pipes();
        let strength = percent.min(100) as f32 / 100.0 * SSAO_MAX;
        let u = [proj.x_axis.x, proj.y_axis.y, NEAR, far, SSAO_RADIUS, SSAO_FADE[0], SSAO_FADE[1], strength, size.width as f32, size.height as f32, hdr as u32 as f32, 0.0];
        self.queue.write_buffer(&s.uniform, 0, bytemuck::cast_slice(&u));
        let mut run = |label, pipe: &wgpu::RenderPipeline, bind: &wgpu::BindGroup, dst: &wgpu::TextureView| {
            let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some(label),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment { view: dst, depth_slice: None, resolve_target: None, ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::WHITE), store: wgpu::StoreOp::Store } })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(pipe);
            pass.set_bind_group(0, bind, &[]);
            pass.draw(0..3, 0..1);
        };
        run("ssao", &s.gtao[msaa as usize], &st.gtao, &st.a);
        run("ssao blur h", &s.blur_h, &st.blur_h, &st.b);
        run("ssao blur v", &s.blur_v, &st.blur_v, &st.a);
    }

    /// REMASTER, Enhanced car paint: redraw the env cube around `centre` (sky, world, scene objects,
    /// the other cars and the halos). Each face is its own submit: the faces share the uniform
    /// buffers, which take their values at submit.
    #[allow(clippy::too_many_arguments)]
    fn draw_probe(&self, s: &GpuScene, base: Globals, v: &View, f: &Frame, centre: Vec3, skip: &std::ops::Range<usize>, props: Option<(&wgpu::Buffer, u32)>, view_far: f32) {
        let pipes = self.remaster_pipes(1, true);
        let (near, far) = (0.2, view_far.min(PROBE_FAR));
        let proj = Mat4::from_scale(Vec3::new(-1.0, 1.0, 1.0)) * glam::camera::rh::proj::directx::perspective(std::f32::consts::FRAC_PI_2, 1.0, near, far);
        let l = &s.look;
        // the sky band where `fs_sky` puts it for a level camera on a 4:3 screen (FOV_X wide,
        // tan of half the vertical fov = 0.75), by elevation; it repeats as often around 360
        // degrees (z: repeats, w: that tan)
        let tan_v = (FOV_X.to_radians() / 2.0).tan() * 0.75;
        let repeats = (std::f32::consts::TAU / FOV_X.to_radians()).round();
        let band = s.sky_size.1 as f32 / SKY_REF_LINES;
        let sky_geom = if s.sky_size.1 == 0 || !v.sky_image { [0.5, 0.0, repeats, tan_v] } else { [l.horizon - band / 2.0, band, repeats, tan_v] };
        let fade_start = (l.fog_start + (1.0 - l.fog_start) * 0.85) * view_far;
        let cars = || f.cars.iter().enumerate().take(MAX_CARS).filter(|(_, (mesh, _))| !skip.contains(mesh));
        // the cube is linear light: the clear colour is the gamma-space sky bottom decoded
        let bottom = l.sky_bottom.map(|c| (c as f64).max(0.0).powf(GAMMA as f64));
        // CHEAT tribute to tron, REMASTER Neon grid (#320): the graded dome (or black) and the grid mirrored
        let tron = base.env[3] as u32 & TRON_FLAG != 0;
        let bottom = if tron { [0.0; 3] } else { bottom };
        for (face, (view, dir)) in EnvProbe::face_views(centre).into_iter().enumerate() {
            let vp = proj * view;
            let mut g = Globals {
                view_proj: vp.to_cols_array_2d(),
                view: view.to_cols_array_2d(),
                sky_inv: vp.inverse().to_cols_array_2d(),
                cam_pos: [centre.x, centre.y, centre.z, 1.0],
                params: [far * l.fog_start, far, base.params[2], base.params[3]],
                sky_geom,
                env: [0.0, base.env[1], base.env[2], ((base.env[3] as u32 & !VOLFOG_FLAG) | LINEAR_FLAG) as f32],
                // REMASTER fog (#196): the faces' own depth range. `fog_amount`'s clip guard reads the
                // far plane from water_t.w; the main view fills it in only after this probe is drawn,
                // and a zero there fogs every face completely, so Enhanced paint reflected the fog
                // colour inside tunnels (no scene copy: the water samples none here).
                water_t: [base.water_t[0], 0.0, near, far],
                // REMASTER Neon grid (#320): the grid's sides in the faces' own pixels
                tron: [0.5 * NEON_PROBE_EDGE_PX, 0.0, 0.0, 0.0],
                ..base
            };
            self.queue.write_buffer(&self.uniforms, 0, bytemuck::bytes_of(&g));
            g.ambient[3] = 1.0;
            self.queue.write_buffer(&self.prop_uniform, 0, bytemuck::bytes_of(&g));
            for (slot, &(mesh, m)) in cars() {
                g.model = m.to_cols_array_2d();
                g.spot_n[2] = wheel_spin(f, mesh);
                set_myw(&mut g, &s.point_lights, s.point_lights.at(m.w_axis.truncate()), true);
                self.queue.write_buffer(&self.car_uniforms[slot], 0, bytemuck::bytes_of(&g));
            }
            let mut halo_v = Vec::new();
            if v.halos {
                let cam = Camera::new(centre, dir.x.atan2(-dir.z), dir.y.clamp(-1.0, 1.0).asin());
                let fx = std::f32::consts::FRAC_PI_2;
                halo_quads(&mut halo_v, &s.world_halos, Mat4::IDENTITY, Vec3::splat(WORLD_HALO_BRIGHTNESS), HALO_GAIN, &cam, fx, far, fade_start, view_far, HALO_SIZE, 0.0, 1.0, 0.0, Some([1.0, 1.0]));
                for (_, &(mesh, m)) in cars() {
                    if let Some(h) = s.halos(mesh, v.generated_cars()) {
                        halo_quads(&mut halo_v, h, m, Vec3::splat(CAR_HALO_BRIGHTNESS), HALO_GAIN, &cam, fx, far, fade_start, view_far, HALO_SIZE, 0.0, 1.0, 0.0, Some([1.0, 1.0]));
                    }
                }
                for (i, &(mesh, m)) in f.props.iter().enumerate() {
                    if let Some(h) = s.prop_halos.get(mesh) {
                        let glow = f.prop_glow.get(i).copied().unwrap_or(Vec3::ONE) * WORLD_HALO_BRIGHTNESS;
                        halo_quads(&mut halo_v, h, m, glow, HALO_GAIN, &cam, fx, far, fade_start, view_far, HALO_SIZE, 0.0, 1.0, 0.0, Some([1.0, 1.0]));
                    }
                }
            }
            let halos = (!halo_v.is_empty()).then(|| {
                self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("probe halos"),
                    contents: bytemuck::cast_slice(&halo_v),
                    usage: wgpu::BufferUsages::VERTEX,
                })
            });
            let mut enc = self.device.create_command_encoder(&Default::default());
            let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("env probe"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &self.probe.faces[face],
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color { r: bottom[0], g: bottom[1], b: bottom[2], a: 1.0 }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &self.probe.depth,
                    depth_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Clear(1.0), store: wgpu::StoreOp::Discard }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_bind_group(0, &s.probe_bind, &[]);
            pass.set_bind_group(1, self.aniso_bind(v.remaster.anisotropy), &[]);
            pass.set_bind_group(2, &self.shadow_map().read, &[]);
            let dome = pipes.sky_cube_dome.as_ref().filter(|_| dome_sun(v, s).is_some());
            if !tron || dome.is_some() {
                pass.set_pipeline(dome.unwrap_or(&pipes.sky_cube));
                pass.draw(0..3, 0..1);
            }
            pass.set_pipeline(&pipes.tri_mirror);
            // REMASTER: the probe's sky shows where the water texture was (a sky reflection is what the water gives)
            let (b, n) = s.world(v);
            pass.set_vertex_buffer(0, b.slice(..));
            pass.draw(0..n, 0..1);
            if let Some((b, n)) = s.fill.as_ref().filter(|_| v.world_fill()) {
                pass.set_vertex_buffer(0, b.slice(..));
                pass.draw(0..*n, 0..1);
            }
            if let Some((b, n)) = props {
                pass.set_bind_group(0, &s.probe_prop_bind, &[]);
                pass.set_vertex_buffer(0, b.slice(..));
                pass.draw(0..n, 0..1);
            }
            for (slot, &(mesh, _)) in cars() {
                // REMASTER: the 128 px faces show the other cars with their original model
                let Some((vb, n)) = s.car_lod(mesh, v.generated_cars(), true) else { continue };
                pass.set_bind_group(0, &s.probe_car_binds[slot], &[]);
                pass.set_vertex_buffer(0, vb.slice(..));
                pass.draw(0..*n, 0..1);
            }
            if let Some(b) = &halos {
                pass.set_bind_group(0, &s.probe_bind, &[]);
                pass.set_pipeline(&pipes.halo);
                pass.set_vertex_buffer(0, b.slice(..));
                pass.draw(0..halo_v.len() as u32, 0..1);
            }
            drop(pass);
            self.queue.submit([enc.finish()]);
        }
    }

    /// REMASTER menu car preview: the Enhanced paint's env cube as a studio, drawn into the probe's texture
    /// [design choice]: a bright softbox above, an even neutral grey round, a dark floor below. Linear HDR
    /// like `draw_probe`'s faces; it turns about the vertical axis without changing, so it holds for any
    /// camera angle round the car.
    fn draw_studio_cube(&self) {
        let mut enc = self.device.create_command_encoder(&Default::default());
        // +X -X +Y -Y +Z -Z
        let faces = [STUDIO_SIDE, STUDIO_SIDE, STUDIO_TOP, STUDIO_FLOOR, STUDIO_SIDE, STUDIO_SIDE];
        for (view, c) in self.probe.faces.iter().zip(faces) {
            drop(enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("studio cube"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color { r: c, g: c, b: c, a: 1.0 }), store: wgpu::StoreOp::Store },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            }));
        }
        self.queue.submit([enc.finish()]);
    }

    /// REMASTER velocity blur: the main view kept for the next frame's motion. A frame drawn
    /// again (the game's F12 screenshot) restores it first, so it has the same motion as the
    /// frame on screen instead of none.
    pub fn motion_history(&self) -> MotionHistory {
        MotionHistory(self.prev_view.borrow().clone())
    }

    pub fn restore_motion_history(&self, h: MotionHistory) {
        *self.prev_view.borrow_mut() = h.0;
    }

    /// REMASTER velocity blur (09-remaster 9.17): the velocity pass, then the gather blur from a
    /// copy of the scene back into it. Keeps this view for the next frame. Cars move with their
    /// own matrices. Real car shadows are taken out of the world's colour for the blur and the
    /// centre's put back (vblur.wgsl `shadow_factor`), so the road streaks under a sharp shadow;
    /// projected shadows move with their car (the body flattened onto the ground under its
    /// wheels). So do props that moved since the last frame (animated scene objects, and the
    /// camera-locked podium emblem, which must not streak). Everything else gets the camera's
    /// motion from the depth buffer.
    ///
    /// Split in two (13-antialiasing phase 5): `motion_pass` writes the motion target (also TAA's),
    /// `velocity_blur` blurs with it. `still` is the unjittered view-projection (what
    /// `PrevView` keeps), `jm` the TAA jitter (identity without TAA); the previous matrices get
    /// the same shift so the jitter cancels out of the motion. Returns true on a camera cut.
    #[allow(clippy::too_many_arguments)]
    fn motion_pass(&self, enc: &mut wgpu::CommandEncoder, s: &GpuScene, t: &Targets, scene: &(wgpu::Texture, wgpu::TextureView, wgpu::BindGroup), cars: &[(usize, Mat4)], props: &[(usize, Mat4)], shadow: VelShadow, still: Mat4, jm: Mat4, cam: &Camera, far: f32, samples: u32, hdr: bool, generated: bool, blur: bool) -> bool {
        let msaa = samples > 1;
        let Some(vt) = &t.vblur else { return true };
        let view_proj = jm * still;
        let shadow_dark = if let VelShadow::Map(k) = shadow { k } else { 0.0 };
        let b = self.vel_blur();
        let mut prev = self.prev_view.borrow_mut();
        // a camera cut (first frame, respawn, another camera, look back) has no motion to blur
        let last = prev.as_ref().filter(|p| p.pos.distance(cam.pos) < CUT_DISTANCE && p.fwd.dot(cam.forward()) > CUT_COS);
        let cut = last.is_none();
        let prev_vp = last.map_or(view_proj, |p| jm * p.view_proj);
        let old = |mesh: usize| last.and_then(|p| p.cars.get(&mesh)).copied();
        // what an instance draws: a car mesh, or a range of the moving props' vertices
        enum Src {
            Car(usize),
            Props(std::ops::Range<u32>),
        }
        let mut draws: Vec<(Src, Mat4, Mat4)> = Vec::new();
        for &(mesh, m) in cars.iter().take(MAX_CARS) {
            let base = mesh - mesh % CAR_MESHES;
            let body = cars.iter().find(|c| c.0 == base).map(|c| c.1);
            // wheels move with their body over a frame: their spin aliases at speed
            let pm = match (body, old(base)) {
                (Some(b), Some(pb)) if mesh != base => pb * b.inverse() * m,
                _ => old(mesh).unwrap_or(m),
            };
            // a respawned car jumps
            let pm = if pm.w_axis.distance(m.w_axis) < CUT_DISTANCE { pm } else { m };
            draws.push((Src::Car(mesh), m, pm));
            match shadow {
                // (the ghost casts none)
                VelShadow::Projected if mesh == base && mesh < VEL_GHOST_KEY => {
                    // its shadow: the body dropped straight down onto the plane through the
                    // wheels' contact points, in world space so it does not roll with the body
                    // [APPROX: flat ground, VEL_WHEEL_R]
                    let wheels: Vec<(Vec3, Vec3)> = cars
                        .iter()
                        .filter(|c| c.0 > base && c.0 < base + CAR_MESHES)
                        .map(|c| (c.1.w_axis.truncate(), old(c.0).unwrap_or(c.1).w_axis.truncate()))
                        .collect();
                    let cur: Vec<Vec3> = wheels.iter().map(|w| w.0).collect();
                    let prev: Vec<Vec3> = wheels.iter().map(|w| w.1).collect();
                    if let (Some(f), Some(pf)) = (ground_flatten(&cur), ground_flatten(&prev)) {
                        draws.push((Src::Car(mesh), f * m, pf * pm));
                    }
                }
                _ => {}
            }
        }
        let mut prop_v: Vec<Vertex> = Vec::new();
        for (k, &(mesh, m)) in props.iter().enumerate() {
            let Some(&(pmesh, pm)) = last.and_then(|p| p.props.get(k)) else { continue };
            let moved = pmesh == mesh && pm != m && pm.w_axis.distance(m.w_axis) < CUT_DISTANCE;
            let Some(src) = s.props.get(mesh).filter(|_| moved && (m.w_axis.truncate() - cam.pos).length() < far) else { continue };
            let start = prop_v.len() as u32;
            prop_v.extend_from_slice(src);
            draws.push((Src::Props(start..prop_v.len() as u32), m, pm));
        }
        draws.truncate(VEL_MAX);
        let mut data = vec![[[0.0f32; 4]; 4]; 2 * VEL_MAX];
        for (slot, (_, m, pm)) in draws.iter().enumerate() {
            data[2 * slot] = (view_proj * *m).to_cols_array_2d();
            data[2 * slot + 1] = (prev_vp * *pm).to_cols_array_2d();
        }
        *prev = Some(PrevView { view_proj: still, pos: cam.pos, fwd: cam.forward(), cars: cars.iter().copied().collect(), props: props.to_vec() });
        self.queue.write_buffer(&b.cars, 0, bytemuck::cast_slice(&data));
        let prop_buf = (!prop_v.is_empty()).then(|| {
            self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some("moving props"), contents: bytemuck::cast_slice(&prop_v), usage: wgpu::BufferUsages::VERTEX })
        });
        let size = scene.0.size();
        let mut u: Vec<f32> = [view_proj.inverse(), prev_vp].iter().flat_map(|m| m.to_cols_array()).collect();
        // (TAA alone wants the whole motion)
        let (shutter, longest) = if blur { (VBLUR_SHUTTER, VBLUR_MAX * size.width as f32) } else { (1.0, f32::MAX) };
        u.extend([shutter, longest, NEAR, far, (self.frame_no.get() % 4096) as f32, samples as f32, shadow_dark, if hdr { 2.2 } else { 1.0 }]);
        u.extend(view_proj.to_cols_array());
        self.queue.write_buffer(&b.uniform, 0, bytemuck::cast_slice(&u));
        let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("car velocity"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment { view: &vt.vel, depth_slice: None, resolve_target: None, ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT), store: wgpu::StoreOp::Store } })],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: &vt.vel_depth,
                depth_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Clear(1.0), store: wgpu::StoreOp::Discard }),
                stencil_ops: None,
            }),
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_pipeline(&b.vel_pipe);
        pass.set_bind_group(0, &b.vel_bind, &[]);
        for (slot, (src, ..)) in draws.iter().enumerate() {
            let k = slot as u32;
            match src {
                Src::Car(mesh) => {
                    let Some((vb, n)) = s.car_lod(*mesh % VEL_GHOST_KEY, generated, self.is_far(*mesh)) else { continue };
                    pass.set_vertex_buffer(0, vb.slice(..));
                    pass.draw(0..*n, k..k + 1);
                }
                Src::Props(r) => {
                    let Some(pb) = &prop_buf else { continue };
                    pass.set_vertex_buffer(0, pb.slice(..));
                    pass.draw(r.clone(), k..k + 1);
                }
            }
        }
        drop(pass);
        // every pixel's blur vector once, for the taps of the blur pass to read
        let motion = b.motion_pipes[msaa as usize].get_or_init(|| {
            fullscreen_pipeline(&self.device, "velocity motion", &b.shaders[msaa as usize], &b.layouts[msaa as usize], "vs_blur", "fs_motion", HDR, None)
        });
        let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("velocity motion"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment { view: &vt.motion, depth_slice: None, resolve_target: None, ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT), store: wgpu::StoreOp::Store } })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_pipeline(motion);
        pass.set_bind_group(0, &vt.pre, &[]);
        pass.draw(0..3, 0..1);
        drop(pass);
        cut
    }

    /// REMASTER water (16-water): copy the opaque scene (`work`, resolved after the opaque pass
    /// with MSAA; drawn into directly without), then draw the surface over the scene reading the
    /// copy and the depth (a read-only attachment, so it can be bound too).
    #[allow(clippy::too_many_arguments)]
    fn water_pass(&self, enc: &mut wgpu::CommandEncoder, wt: &WaterTargets, pipe: &wgpu::RenderPipeline, work: &(wgpu::Texture, wgpu::TextureView, wgpu::BindGroup), color: &wgpu::TextureView, depth: &wgpu::TextureView, mesh: (&wgpu::Buffer, u32), s: &GpuScene, v: &View) {
        enc.copy_texture_to_texture(work.0.as_image_copy(), wt.src.as_image_copy(), wt.src.size());
        let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("water"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment { view: color, depth_slice: None, resolve_target: None, ops: wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Store } })],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment { view: depth, depth_ops: None, stencil_ops: None }),
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_pipeline(pipe);
        pass.set_bind_group(0, &s.bind, &[]);
        pass.set_bind_group(1, self.aniso_bind(v.remaster.anisotropy), &[]);
        pass.set_bind_group(2, self.shadow_read(), &[]);
        pass.set_bind_group(3, &wt.bind, &[]);
        pass.set_vertex_buffer(0, mesh.0.slice(..));
        pass.draw(0..mesh.1, 0..1);
    }

    /// REMASTER velocity blur (the second half): the gather blur from a copy of the scene back
    /// into it, then the late sparks.
    fn velocity_blur(&self, enc: &mut wgpu::CommandEncoder, t: &Targets, scene: &(wgpu::Texture, wgpu::TextureView, wgpu::BindGroup), msaa: bool, hdr: bool, sparks: Option<(&wgpu::Buffer, u32)>) {
        let Some(vt) = &t.vblur else { return };
        let b = self.vel_blur();
        enc.copy_texture_to_texture(scene.0.as_image_copy(), vt.src.as_image_copy(), scene.0.size());
        let pipe = b.pipes[msaa as usize][hdr as usize].get_or_init(|| {
            let format = if hdr { HDR } else { self.color_format };
            fullscreen_pipeline(&self.device, "velocity blur", &b.shaders[msaa as usize], &b.layouts[msaa as usize], "vs_blur", "fs_blur", format, None)
        });
        let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("velocity blur"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment { view: &scene.1, depth_slice: None, resolve_target: None, ops: wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Store } })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_pipeline(pipe);
        pass.set_bind_group(0, &vt.bind, &[]);
        pass.draw(0..3, 0..1);
        drop(pass);
        // REMASTER sparks over the blurred scene (09-remaster 9.16): light, so not smeared; hidden
        // by the world's depth (vblur.wgsl `fs_spark`)
        if let Some((buf, n)) = sparks {
            let pipe = b.spark_pipes[msaa as usize][hdr as usize].get_or_init(|| {
                let format = if hdr { HDR } else { self.color_format };
                spark_pipeline(&self.device, &b.shaders[msaa as usize], &b.layouts[msaa as usize], format)
            });
            let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("late sparks"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment { view: &scene.1, depth_slice: None, resolve_target: None, ops: wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Store } })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(pipe);
            pass.set_bind_group(0, &vt.bind, &[]);
            pass.set_vertex_buffer(0, buf.slice(..));
            pass.draw(0..n, 0..1);
        }
    }

    /// REMASTER the Time Attack hologram (09-remaster 9.27) over the blurred scene: light that
    /// shows through walls, so the blur would gather it along the motion of whatever is behind it
    /// (long streaks off the screen's edges); the ghost's uniform slots start at `first`.
    #[allow(clippy::too_many_arguments)]
    fn late_holograms(&self, enc: &mut wgpu::CommandEncoder, scene: &(wgpu::Texture, wgpu::TextureView, wgpu::BindGroup), s: &GpuScene, ghosts: &[(usize, Mat4)], first: usize, generated: bool, hdr: bool, anisotropy: u16) {
        if ghosts.is_empty() || first >= MAX_CARS {
            return;
        }
        let [fill, edges] = self.vel_blur().holo_pipes[hdr as usize].get_or_init(|| {
            let format = if hdr { HDR } else { self.color_format };
            [(wgpu::PrimitiveTopology::TriangleList, "fs_holo_fill"), (wgpu::PrimitiveTopology::LineList, "fs_holo")].map(|(t, e)| late_holo_pipeline(&self.device, &self.src, format, t, e))
        });
        let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("late hologram"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment { view: &scene.1, depth_slice: None, resolve_target: None, ops: wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Store } })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_bind_group(1, self.aniso_bind(anisotropy), &[]);
        pass.set_bind_group(2, self.shadow_read(), &[]);
        for (k, &(mesh, _)) in ghosts.iter().enumerate().take(MAX_CARS - first) {
            pass.set_bind_group(0, &s.car_binds[first + k], &[]);
            for (pipe, buf) in [(fill, s.car(mesh, generated)), (edges, s.holo_wires(mesh, generated))] {
                let Some((vb, n)) = buf else { continue };
                pass.set_pipeline(pipe);
                pass.set_vertex_buffer(0, vb.slice(..));
                pass.draw(0..*n, 0..1);
            }
        }
    }

    fn history_target(&self, size: wgpu::Extent3d, format: wgpu::TextureFormat) -> History {
        let tex = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("motion blur history"),
            size,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let bind = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("motion blur history"),
            layout: &self.blur_layout,
            entries: &[wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&tex.create_view(&Default::default())) }],
        });
        History { tex, bind, valid: false }
    }

    /// design choice (09-remaster 9.1 rule 2): a window-sized copy of a finished frame for the
    /// F9 dissolve, filled by drawing into `Snapshot::view`.
    pub fn snapshot_target(&self, w: u32, h: u32) -> Snapshot {
        let tex = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("dissolve snapshot"),
            size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: self.color_format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let view = tex.create_view(&Default::default());
        let bind = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("dissolve snapshot"),
            layout: &self.blur_layout,
            entries: &[wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&view) }],
        });
        Snapshot { size: (w, h), view, bind }
    }

    /// design choice: blend the snapshot over `target` with weight `alpha` (1 = all snapshot).
    pub fn draw_dissolve(&self, enc: &mut wgpu::CommandEncoder, target: &wgpu::TextureView, snap: &Snapshot, alpha: f64) {
        let pipe = self.dissolve.get_or_init(|| {
            let shader = self.device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("dissolve shader"),
                source: wgpu::ShaderSource::Wgsl(include_str!("blur.wgsl").into()),
            });
            let layout = self.device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("dissolve"), bind_group_layouts: &[Some(&self.blur_layout)], immediate_size: 0 });
            self.device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("dissolve"),
                layout: Some(&layout),
                vertex: wgpu::VertexState { module: &shader, entry_point: Some("vs_blur"), compilation_options: Default::default(), buffers: &[] },
                primitive: wgpu::PrimitiveState { cull_mode: None, ..Default::default() },
                depth_stencil: None,
                multisample: wgpu::MultisampleState::default(),
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some("fs_blur"),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: self.color_format,
                        blend: Some(wgpu::BlendState {
                            color: wgpu::BlendComponent { src_factor: wgpu::BlendFactor::Constant, dst_factor: wgpu::BlendFactor::OneMinusConstant, operation: wgpu::BlendOperation::Add },
                            alpha: wgpu::BlendComponent::REPLACE,
                        }),
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                multiview_mask: None,
                cache: None,
            })
        });
        let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("dissolve"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: target,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Store },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_pipeline(pipe);
        pass.set_blend_constant(wgpu::Color { r: alpha, g: alpha, b: alpha, a: 1.0 });
        pass.set_bind_group(0, &snap.bind, &[]);
        pass.draw(0..3, 0..1);
    }

    /// Render one frame offscreen and return tightly packed RGBA8 pixels.
    pub fn capture(&self, w: u32, h: u32, cam: &Camera, v: &View, f: &Frame) -> Vec<u8> {
        self.offscreen(w, h, |g, enc, target, depth| g.draw(enc, target, depth, (w, h), cam, v, f))
    }

    /// Render offscreen with `draw(gpu, encoder, colour target, depth)` and read back RGBA8.
    /// DEBUG: draw the next frames as view `k` (0 = the main window): its own targets and
    /// histories, so two windows of different sizes do not reset each other's TAA, blur and
    /// size-keyed targets. Pipelines and scene data are shared. Call between draws.
    pub fn select_view(&self, k: usize) {
        let now = self.view_now.get();
        if k == now {
            return;
        }
        let mut views = self.views.borrow_mut();
        if views.len() <= k.max(now) {
            views.resize_with(k.max(now) + 1, ViewState::default);
        }
        // the volumetric fog keeps one history: another view has none to blend with
        if let Some(vf) = self.volfog.get() {
            vf.reset();
        }
        views[now] = ViewState { targets: self.targets.take(), history: self.history.take(), prev_view: self.prev_view.take(), taa_frame: self.taa_frame.replace(0) };
        let next = std::mem::take(&mut views[k]);
        *self.targets.borrow_mut() = next.targets;
        *self.history.borrow_mut() = next.history;
        *self.prev_view.borrow_mut() = next.prev_view;
        self.taa_frame.set(next.taa_frame);
        self.view_now.set(k);
    }

    pub fn offscreen(&self, w: u32, h: u32, draw: impl FnOnce(&Self, &mut wgpu::CommandEncoder, &wgpu::TextureView, &wgpu::TextureView)) -> Vec<u8> {
        let tex = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("shot"),
            size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: self.color_format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let row = (w * 4).div_ceil(256) * 256;
        let out = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("readback"),
            size: (row * h) as u64,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let depth = self.depth_view(w, h);
        let mut enc = self.device.create_command_encoder(&Default::default());
        draw(self, &mut enc, &tex.create_view(&Default::default()), &depth);
        enc.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo { texture: &tex, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
            wgpu::TexelCopyBufferInfo {
                buffer: &out,
                layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(row), rows_per_image: Some(h) },
            },
            wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
        );
        self.queue.submit([enc.finish()]);
        out.slice(..).map_async(wgpu::MapMode::Read, |r| r.expect("map readback buffer"));
        self.device.poll(wgpu::PollType::wait_indefinitely()).expect("device poll");
        let mapped = out.slice(..).get_mapped_range().expect("mapped range");
        let mut px = Vec::with_capacity((w * h * 4) as usize);
        for y in 0..h as usize {
            px.extend_from_slice(&mapped[y * row as usize..][..(w * 4) as usize]);
        }
        px
    }

    /// DEBUG: the Enhanced paint's env cube as drawn last (`draw_probe`), RGBA8 as one vertical
    /// strip of its six faces in wgpu order (+X -X +Y -Y +Z -Z, view space; each face x-mirrored
    /// by the flipped projection): (width, height, pixels). motorview `--env-cube` (#196).
    pub fn env_cube(&self) -> (u32, u32, Vec<u8>) {
        let (w, faces) = (PROBE_SIZE, 6u32);
        // PROBE_SIZE * 8 bytes (RGBA16F) is a multiple of 256, so the rows need no repacking
        let row = w * 8;
        let out = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("env cube readback"),
            size: (row * w * faces) as u64,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut enc = self.device.create_command_encoder(&Default::default());
        enc.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo { texture: &self.probe.tex, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
            wgpu::TexelCopyBufferInfo {
                buffer: &out,
                layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(row), rows_per_image: Some(w) },
            },
            wgpu::Extent3d { width: w, height: w, depth_or_array_layers: faces },
        );
        self.queue.submit([enc.finish()]);
        out.slice(..).map_async(wgpu::MapMode::Read, |r| r.expect("map readback buffer"));
        self.device.poll(wgpu::PollType::wait_indefinitely()).expect("device poll");
        // the linear HDR light as a gamma-encoded RGBA8 image (clamped at 1)
        let px = out
            .slice(..)
            .get_mapped_range()
            .expect("mapped range")
            .as_chunks::<8>()
            .0
            .iter()
            .flat_map(|t| {
                let c = |k: usize| {
                    let v = half::f16::from_le_bytes([t[k * 2], t[k * 2 + 1]]).to_f32();
                    (v.clamp(0.0, 1.0).powf(1.0 / GAMMA) * 255.0 + 0.5) as u8
                };
                [c(0), c(1), c(2), 255]
            })
            .collect();
        (w, w * faces, px)
    }
}

/// The UI pipeline: CLASSIC `vs_ui` / `fs_ui`, or with `sharp` the REMASTER sharp HUD
/// (`fs_ui_remaster`, the field atlas at binding 3, `UiVertex::aux`).
fn ui_pipeline(device: &wgpu::Device, color_format: wgpu::TextureFormat, sharp: bool) -> (wgpu::RenderPipeline, wgpu::BindGroupLayout) {
    ui_pipeline_for(device, color_format, sharp, None)
}

/// `ui_pipeline` for another target format; `reuse`: the bind group layout of a pipeline it
/// shares bind groups with
fn ui_pipeline_for(device: &wgpu::Device, color_format: wgpu::TextureFormat, sharp: bool, reuse: Option<&wgpu::BindGroupLayout>) -> (wgpu::RenderPipeline, wgpu::BindGroupLayout) {
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("ui shader"),
        // the REMASTER rev meter's spine table goes in front of the shader (revmeter.rs)
        source: wgpu::ShaderSource::Wgsl(format!("{}{}", crate::revmeter::wgsl(), include_str!("ui.wgsl")).into()),
    });
    let fields = wgpu::BindGroupLayoutEntry {
        binding: 3,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: true }, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false },
        count: None,
    };
    let mut entries = vec![
        wgpu::BindGroupLayoutEntry {
            binding: 0,
            visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
            ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None },
            count: None,
        },
        wgpu::BindGroupLayoutEntry {
            binding: 1,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable: true },
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        },
        wgpu::BindGroupLayoutEntry {
            binding: 2,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
            count: None,
        },
    ];
    let rev_look = wgpu::BindGroupLayoutEntry {
        binding: 4,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None },
        count: None,
    };
    entries.extend(sharp.then_some(fields));
    entries.extend(sharp.then_some(rev_look));
    let layout = reuse.cloned().unwrap_or_else(|| device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor { label: Some("ui"), entries: &entries }));
    let pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("ui"), bind_group_layouts: &[Some(&layout)], immediate_size: 0 });
    let classic_attrs = wgpu::vertex_attr_array![0 => Float32x2, 1 => Float32x2, 2 => Float32x4, 3 => Float32x4];
    let sharp_attrs = wgpu::vertex_attr_array![0 => Float32x2, 1 => Float32x2, 2 => Float32x4, 3 => Float32x4, 4 => Float32x4];
    let pipe = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("ui"),
        layout: Some(&pl),
        vertex: wgpu::VertexState {
            module: &shader,
            entry_point: Some(if sharp { "vs_ui_remaster" } else { "vs_ui" }),
            compilation_options: Default::default(),
            buffers: &[Some(wgpu::VertexBufferLayout {
                array_stride: std::mem::size_of::<UiVertex>() as u64,
                step_mode: wgpu::VertexStepMode::Vertex,
                attributes: if sharp { &sharp_attrs } else { &classic_attrs },
            })],
        },
        primitive: wgpu::PrimitiveState { cull_mode: None, ..Default::default() },
        depth_stencil: None,
        multisample: Default::default(),
        fragment: Some(wgpu::FragmentState {
            module: &shader,
            entry_point: Some(if sharp { "fs_ui_remaster" } else { "fs_ui" }),
            compilation_options: Default::default(),
            targets: &[Some(wgpu::ColorTargetState { format: color_format, blend: Some(wgpu::BlendState::ALPHA_BLENDING), write_mask: wgpu::ColorWrites::ALL })],
        }),
        multiview_mask: None,
        cache: None,
    });
    (pipe, layout)
}


/// A finished frame kept for the F9 dissolve (`Gpu::snapshot_target`).
pub struct Snapshot {
    pub size: (u32, u32),
    pub view: wgpu::TextureView,
    bind: wgpu::BindGroup,
}

/// MotionBlur: the history texture bound at 0.
fn blur_bind_layout(device: &wgpu::Device) -> wgpu::BindGroupLayout {
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("motion blur"),
        entries: &[wgpu::BindGroupLayoutEntry {
            binding: 0,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable: false },
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        }],
    })
}

/// MotionBlur: a full-screen triangle that blends the previous frame (bound at 0) over the sky
/// with the blend constant as its weight.
fn blur_pipeline(device: &wgpu::Device, src: &PipeSrc, color_format: wgpu::TextureFormat, samples: u32) -> wgpu::RenderPipeline {
    let shader = &src.blur_shader;
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("motion blur"),
        layout: Some(&src.blur_layout),
        vertex: wgpu::VertexState { module: shader, entry_point: Some("vs_blur"), compilation_options: Default::default(), buffers: &[] },
        primitive: wgpu::PrimitiveState { cull_mode: None, ..Default::default() },
        // drawn inside the main pass, like the sky: depth Always, no depth write
        depth_stencil: Some(wgpu::DepthStencilState {
            format: DEPTH,
            depth_write_enabled: Some(false),
            depth_compare: Some(wgpu::CompareFunction::Always),
            stencil: Default::default(),
            bias: Default::default(),
        }),
        multisample: wgpu::MultisampleState { count: samples, ..Default::default() },
        fragment: Some(wgpu::FragmentState {
            module: shader,
            entry_point: Some("fs_blur"),
            compilation_options: Default::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format: color_format,
                blend: Some(wgpu::BlendState {
                    color: wgpu::BlendComponent { src_factor: wgpu::BlendFactor::Constant, dst_factor: wgpu::BlendFactor::OneMinusConstant, operation: wgpu::BlendOperation::Add },
                    alpha: wgpu::BlendComponent::REPLACE,
                }),
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        multiview_mask: None,
        cache: None,
    })
}

/// A filterable float texture binding (`Float { filterable }`) of a bind group layout.
fn tex_entry(binding: u32, filterable: bool) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable }, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false },
        count: None,
    }
}

fn uniform_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None },
        count: None,
    }
}

/// REMASTER water: group 3 over a scene copy and its depth (water.wgsl's bindings)
#[allow(clippy::too_many_arguments)]
fn water_bind(d: &wgpu::Device, layout: &wgpu::BindGroupLayout, scene: &wgpu::TextureView, depth: &wgpu::TextureView, normal: &wgpu::TextureView, foam: &wgpu::TextureView, sampler: &wgpu::Sampler, fog: &wgpu::TextureView) -> wgpu::BindGroup {
    let view = |binding, v| wgpu::BindGroupEntry { binding, resource: wgpu::BindingResource::TextureView(v) };
    d.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("water"),
        layout,
        entries: &[view(0, scene), view(1, depth), view(2, normal), view(3, foam), wgpu::BindGroupEntry { binding: 4, resource: wgpu::BindingResource::Sampler(sampler) }, view(5, fog)],
    })
}

fn sampler_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry { binding, visibility: wgpu::ShaderStages::FRAGMENT, ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering), count: None }
}

fn clamp_sampler(device: &wgpu::Device, label: &str) -> wgpu::Sampler {
    device.create_sampler(&wgpu::SamplerDescriptor { label: Some(label), mag_filter: wgpu::FilterMode::Linear, min_filter: wgpu::FilterMode::Linear, ..Default::default() })
}

/// A full-screen triangle pipeline (`vs` / `fs` of `shader`) into one colour target.
#[allow(clippy::too_many_arguments)]
/// REMASTER sparks over the blurred scene: additive streak quads (the `Vertex` layout's position,
/// uv and colour) with vblur.wgsl's `vs_spark` / `fs_spark`, depth-tested by hand.
fn spark_pipeline(device: &wgpu::Device, shader: &wgpu::ShaderModule, layout: &wgpu::BindGroupLayout, format: wgpu::TextureFormat) -> wgpu::RenderPipeline {
    let pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("late sparks"), bind_group_layouts: &[Some(layout)], immediate_size: 0 });
    let add = wgpu::BlendComponent { src_factor: wgpu::BlendFactor::One, dst_factor: wgpu::BlendFactor::One, operation: wgpu::BlendOperation::Add };
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("late sparks"),
        layout: Some(&pl),
        vertex: wgpu::VertexState {
            module: shader,
            entry_point: Some("vs_spark"),
            compilation_options: Default::default(),
            buffers: &[Some(wgpu::VertexBufferLayout {
                array_stride: std::mem::size_of::<Vertex>() as u64,
                step_mode: wgpu::VertexStepMode::Vertex,
                attributes: &wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x2, 2 => Float32x4],
            })],
        },
        primitive: wgpu::PrimitiveState { cull_mode: None, ..Default::default() },
        depth_stencil: None,
        multisample: Default::default(),
        fragment: Some(wgpu::FragmentState {
            module: shader,
            entry_point: Some("fs_spark"),
            compilation_options: Default::default(),
            targets: &[Some(wgpu::ColorTargetState { format, blend: Some(wgpu::BlendState { color: add, alpha: wgpu::BlendComponent::OVER }), write_mask: wgpu::ColorWrites::ALL })],
        }),
        multiview_mask: None,
        cache: None,
    })
}

/// REMASTER the Time Attack hologram over the blurred scene (`late_holograms`): `scene_pipes`'
/// `Pass::Holo` / `Pass::HoloFill` without the depth (theirs is `Always`) or MSAA (the scene is resolved).
fn late_holo_pipeline(device: &wgpu::Device, src: &PipeSrc, format: wgpu::TextureFormat, topology: wgpu::PrimitiveTopology, entry: &str) -> wgpu::RenderPipeline {
    let add = wgpu::BlendComponent { src_factor: wgpu::BlendFactor::One, dst_factor: wgpu::BlendFactor::One, operation: wgpu::BlendOperation::Add };
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("late hologram"),
        layout: Some(&src.remaster_layout),
        vertex: wgpu::VertexState {
            module: &src.shader,
            entry_point: Some("vs_remaster"),
            compilation_options: Default::default(),
            buffers: &[Some(wgpu::VertexBufferLayout { array_stride: std::mem::size_of::<Vertex>() as u64, step_mode: wgpu::VertexStepMode::Vertex, attributes: &SCENE_ATTRS })],
        },
        primitive: wgpu::PrimitiveState { topology, cull_mode: None, ..Default::default() },
        depth_stencil: None,
        multisample: Default::default(),
        fragment: Some(wgpu::FragmentState {
            module: &src.shader,
            entry_point: Some(entry),
            compilation_options: Default::default(),
            targets: &[Some(wgpu::ColorTargetState { format, blend: Some(wgpu::BlendState { color: add, alpha: wgpu::BlendComponent::OVER }), write_mask: wgpu::ColorWrites::ALL })],
        }),
        multiview_mask: None,
        cache: None,
    })
}

/// REMASTER: a pass over the drawn scene (colour and depth loaded) for what follows the opaque meshes;
/// the last one resolves the multisampled colour, the earlier ones keep it.
/// A colour-only render pass over the whole of `view` (the glow's and the frosted hologram's passes).
fn fullscreen_target_pass<'e>(enc: &'e mut wgpu::CommandEncoder, label: &str, view: &wgpu::TextureView, load: wgpu::LoadOp<wgpu::Color>) -> wgpu::RenderPass<'e> {
    enc.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some(label),
        color_attachments: &[Some(wgpu::RenderPassColorAttachment { view, depth_slice: None, resolve_target: None, ops: wgpu::Operations { load, store: wgpu::StoreOp::Store } })],
        depth_stencil_attachment: None,
        timestamp_writes: None,
        occlusion_query_set: None,
        multiview_mask: None,
    })
}

fn overlay_pass<'e>(enc: &'e mut wgpu::CommandEncoder, label: &str, color: &wgpu::TextureView, depth: &wgpu::TextureView, resolve: Option<&wgpu::TextureView>, last: bool, motion: bool) -> wgpu::RenderPass<'e> {
    enc.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some(label),
        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
            view: color,
            depth_slice: None,
            resolve_target: resolve,
            ops: wgpu::Operations { load: wgpu::LoadOp::Load, store: if resolve.is_some() { wgpu::StoreOp::Discard } else { wgpu::StoreOp::Store } },
        })],
        depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
            view: depth,
            depth_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Load, store: if motion || !last { wgpu::StoreOp::Store } else { wgpu::StoreOp::Discard } }),
            stencil_ops: None,
        }),
        timestamp_writes: None,
        occlusion_query_set: None,
        multiview_mask: None,
    })
}

#[allow(clippy::too_many_arguments)]
fn fullscreen_pipeline(device: &wgpu::Device, label: &str, shader: &wgpu::ShaderModule, layout: &wgpu::BindGroupLayout, vs: &str, fs: &str, format: wgpu::TextureFormat, blend: Option<wgpu::BlendState>) -> wgpu::RenderPipeline {
    let pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some(label), bind_group_layouts: &[Some(layout)], immediate_size: 0 });
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some(label),
        layout: Some(&pl),
        vertex: wgpu::VertexState { module: shader, entry_point: Some(vs), compilation_options: Default::default(), buffers: &[] },
        primitive: wgpu::PrimitiveState { cull_mode: None, ..Default::default() },
        depth_stencil: None,
        multisample: Default::default(),
        fragment: Some(wgpu::FragmentState {
            module: shader,
            entry_point: Some(fs),
            compilation_options: Default::default(),
            targets: &[Some(wgpu::ColorTargetState { format, blend, write_mask: wgpu::ColorWrites::ALL })],
        }),
        multiview_mask: None,
        cache: None,
    })
}

/// REMASTER anti-aliasing passes (aa.wgsl): the SSAA downsample and FXAA.
fn aa_pipeline(device: &wgpu::Device, color_format: wgpu::TextureFormat) -> Aa {
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("aa shader"),
        source: wgpu::ShaderSource::Wgsl(include_str!("aa.wgsl").into()),
    });
    let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("aa"),
        entries: &[tex_entry(0, true), sampler_entry(1), uniform_entry(2)],
    });
    let down = [color_format, HDR].map(|f| fullscreen_pipeline(device, "ssaa down", &shader, &layout, "vs_aa", "fs_down", f, None));
    let uniform = device.create_buffer(&wgpu::BufferDescriptor { label: Some("aa globals"), size: 16, usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
    let fxaa = fullscreen_pipeline(device, "fxaa", &shader, &layout, "vs_aa", "fs_fxaa", color_format, None);
    Aa { layout, sampler: clamp_sampler(device, "aa"), uniform, down, fxaa }
}

/// REMASTER TAA (taa.wgsl): bindings 0-1 are the current image and the history, 2 the motion,
/// 3 the history's sampler, 4 the globals.
fn taa_pipeline(device: &wgpu::Device, color_format: wgpu::TextureFormat) -> Taa {
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("taa shader"),
        source: wgpu::ShaderSource::Wgsl(include_str!("taa.wgsl").into()),
    });
    let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("taa"),
        entries: &[tex_entry(0, true), tex_entry(1, true), tex_entry(2, true), sampler_entry(3), uniform_entry(4)],
    });
    let uniform = device.create_buffer(&wgpu::BufferDescriptor { label: Some("taa globals"), size: 48, usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
    let pipes = |label, fs| [color_format, HDR].map(|f| fullscreen_pipeline(device, label, &shader, &layout, "vs_taa", fs, f, None));
    Taa { resolve: pipes("taa resolve", "fs_taa"), sharpen: pipes("taa sharpen", "fs_sharpen"), layout, sampler: clamp_sampler(device, "taa"), uniform }
}

/// REMASTER SMAA (smaa.wgsl): bindings 0-2 are the tonemapped image, the edges and the weights
/// (each pass reads the ones before it), 3 the preset.
fn smaa_pipeline(device: &wgpu::Device, color_format: wgpu::TextureFormat) -> Smaa {
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("smaa shader"),
        source: wgpu::ShaderSource::Wgsl(include_str!("smaa.wgsl").into()),
    });
    let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("smaa"),
        entries: &[tex_entry(0, true), tex_entry(1, true), tex_entry(2, true), uniform_entry(3)],
    });
    let uniform = device.create_buffer(&wgpu::BufferDescriptor { label: Some("smaa preset"), size: 16, usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
    let pipe = |label, fs, format| fullscreen_pipeline(device, label, &shader, &layout, "vs_smaa", fs, format, None);
    Smaa {
        edges: pipe("smaa edges", "fs_edges", wgpu::TextureFormat::Rg8Unorm),
        weights: pipe("smaa weights", "fs_weights", wgpu::TextureFormat::Rgba8Unorm),
        blend: pipe("smaa blend", "fs_blend", color_format),
        layout,
        uniform,
    }
}

/// REMASTER post pass: a full-screen triangle from the resolved scene (0) to the target, with
/// the options (1: frame number for the grain, grade, vignette, grain; exposure, bloom
/// weight, tonemapper) and the bloom chain's top mip (2, 3).
fn post_pipeline(device: &wgpu::Device, color_format: wgpu::TextureFormat) -> Post {
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("post shader"),
        source: wgpu::ShaderSource::Wgsl(include_str!("post.wgsl").into()),
    });
    let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("post"),
        entries: &[tex_entry(0, false), uniform_entry(1), tex_entry(2, true), sampler_entry(3)],
    });
    let pipe = fullscreen_pipeline(device, "post", &shader, &layout, "vs_post", "fs_post", color_format, None);
    let uniform = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("post globals"),
        size: 32,
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let no_bloom = device
        .create_texture(&wgpu::TextureDescriptor {
            label: Some("no bloom"),
            size: wgpu::Extent3d { width: 1, height: 1, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: HDR,
            usage: wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        })
        .create_view(&Default::default());
    Post { pipe, layout, uniform, no_bloom, sampler: clamp_sampler(device, "post") }
}

/// REMASTER bloom: downsample (the first one thresholded) and additive upsample pipelines,
/// all into `HDR` mips; bind group = source texture, sampler, (threshold, knee).
fn bloom_pipeline(device: &wgpu::Device) -> Bloom {
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("bloom shader"),
        source: wgpu::ShaderSource::Wgsl(include_str!("bloom.wgsl").into()),
    });
    let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("bloom"),
        entries: &[tex_entry(0, true), sampler_entry(1), uniform_entry(2)],
    });
    let add = wgpu::BlendState {
        color: wgpu::BlendComponent { src_factor: wgpu::BlendFactor::One, dst_factor: wgpu::BlendFactor::One, operation: wgpu::BlendOperation::Add },
        alpha: wgpu::BlendComponent::REPLACE,
    };
    Bloom {
        first: fullscreen_pipeline(device, "bloom prefilter", &shader, &layout, "vs_bloom", "fs_prefilter", HDR, None),
        down: fullscreen_pipeline(device, "bloom down", &shader, &layout, "vs_bloom", "fs_down", HDR, None),
        up: fullscreen_pipeline(device, "bloom up", &shader, &layout, "vs_bloom", "fs_up", HDR, Some(add)),
        layout,
        sampler: clamp_sampler(device, "bloom"),
        uniform: device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("bloom globals"),
            size: 16,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        }),
    }
}

/// REMASTER ambient occlusion: the GTAO and blur pipelines (half-size `SSAO_FORMAT` targets); the
/// multiply is built per sample count and output on first use (`Gpu::ssao_apply`).
fn ssao_pipeline(device: &wgpu::Device) -> Ssao {
    let shaders = [false, true].map(|msaa| {
        let ty = if msaa { "texture_depth_multisampled_2d" } else { "texture_depth_2d" };
        device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("ssao shader"), source: wgpu::ShaderSource::Wgsl(include_str!("ssao.wgsl").replace("DEPTH_TEX", ty).into()) })
    });
    let gtao_layouts = [false, true].map(|msaa| {
        let depth = wgpu::BindGroupLayoutEntry {
            binding: 1,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Depth, view_dimension: wgpu::TextureViewDimension::D2, multisampled: msaa },
            count: None,
        };
        device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor { label: Some("ssao"), entries: &[uniform_entry(0), depth] })
    });
    let blur_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor { label: Some("ssao blur"), entries: &[tex_entry(2, false)] });
    let apply_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor { label: Some("ssao apply"), entries: &[uniform_entry(0), tex_entry(2, true), sampler_entry(3)] });
    let gtao = [0, 1].map(|k| fullscreen_pipeline(device, "ssao", &shaders[k], &gtao_layouts[k], "vs_full", "fs_gtao", SSAO_FORMAT, None));
    Ssao {
        blur_h: fullscreen_pipeline(device, "ssao blur h", &shaders[0], &blur_layout, "vs_full", "fs_blur_h", SSAO_FORMAT, None),
        blur_v: fullscreen_pipeline(device, "ssao blur v", &shaders[0], &blur_layout, "vs_full", "fs_blur_v", SSAO_FORMAT, None),
        shaders,
        gtao_layouts,
        gtao,
        blur_layout,
        apply_layout,
        apply: Default::default(),
        uniform: device.create_buffer(&wgpu::BufferDescriptor { label: Some("ssao globals"), size: 48, usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false }),
        sampler: clamp_sampler(device, "ssao"),
    }
}

/// REMASTER velocity blur: the car velocity pass (vertex positions only, own depth, into `HDR`)
/// and the shader / layout per depth kind; the blur pipelines are built on first use.
fn vel_blur_pipeline(device: &wgpu::Device) -> VelBlur {
    let shaders = [false, true].map(|msaa| {
        let ty = if msaa { "texture_depth_multisampled_2d" } else { "texture_depth_2d" };
        device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("velocity blur shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("vblur.wgsl").replace("DEPTH_TEX", ty).replace("MAX_CARS", &VEL_MAX.to_string()).into()),
        })
    });
    let depth_map = wgpu::BindGroupLayoutEntry {
        binding: 2,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Depth, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false },
        count: None,
    };
    let layouts = [false, true].map(|msaa| {
        let depth = wgpu::BindGroupLayoutEntry {
            binding: 2,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Depth, view_dimension: wgpu::TextureViewDimension::D2, multisampled: msaa },
            count: None,
        };
        device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("velocity blur"),
            entries: &[tex_entry(0, true), sampler_entry(1), depth, tex_entry(3, false), wgpu::BindGroupLayoutEntry { visibility: wgpu::ShaderStages::VERTEX_FRAGMENT, ..uniform_entry(4) }, uniform_entry(5), wgpu::BindGroupLayoutEntry { binding: 6, ..depth_map }, wgpu::BindGroupLayoutEntry { binding: 7, visibility: wgpu::ShaderStages::FRAGMENT, ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Comparison), count: None }, tex_entry(8, false)],
        })
    });
    let buffer = |label| {
        device.create_buffer(&wgpu::BufferDescriptor { label: Some(label), size: (2 * VEL_MAX * 64) as u64, usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false })
    };
    let cars = buffer("car velocity matrices");
    let vel_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("car velocity"),
        entries: &[wgpu::BindGroupLayoutEntry {
            binding: 0,
            visibility: wgpu::ShaderStages::VERTEX,
            ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None },
            count: None,
        }],
    });
    let bind = |buf: &wgpu::Buffer| {
        device.create_bind_group(&wgpu::BindGroupDescriptor { label: Some("car velocity"), layout: &vel_layout, entries: &[wgpu::BindGroupEntry { binding: 0, resource: buf.as_entire_binding() }] })
    };
    let vel_bind = bind(&cars);
    let pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("car velocity"), bind_group_layouts: &[Some(&vel_layout)], immediate_size: 0 });
    let pipe = |label, fs| {
        device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some(label),
            layout: Some(&pl),
            vertex: wgpu::VertexState {
                module: &shaders[0],
                entry_point: Some("vs_vel"),
                compilation_options: Default::default(),
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<Vertex>() as u64,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &wgpu::vertex_attr_array![0 => Float32x3],
                })],
            },
            primitive: wgpu::PrimitiveState { cull_mode: None, ..Default::default() },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: DEPTH,
                depth_write_enabled: Some(true),
                depth_compare: Some(wgpu::CompareFunction::Less),
                stencil: Default::default(),
                bias: Default::default(),
            }),
            multisample: Default::default(),
            fragment: Some(wgpu::FragmentState {
                module: &shaders[0],
                entry_point: Some(fs),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState { format: HDR, blend: None, write_mask: wgpu::ColorWrites::ALL })],
            }),
            multiview_mask: None,
            cache: None,
        })
    };
    let vel_pipe = pipe("car velocity", "fs_vel");
    VelBlur {
        vel_pipe,
        vel_bind,
        cars,
        shaders,
        layouts,
        pipes: Default::default(),
        motion_pipes: Default::default(),
        motion_dummy: device
            .create_texture(&wgpu::TextureDescriptor {
                label: Some("velocity motion dummy"),
                size: wgpu::Extent3d { width: 1, height: 1, depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: HDR,
                usage: wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            })
            .create_view(&Default::default()),
        spark_pipes: Default::default(),
        holo_pipes: Default::default(),
        uniform: device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("velocity blur globals"),
            size: 224,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        }),
        sampler: clamp_sampler(device, "velocity blur"),
    }
}

#[cfg(test)]
mod mip_tests {
    use super::mips_of;

    /// The mip chain by the plain per-texel formula (what `mips_of` was before it walked rows).
    fn reference(layer: &[u8], size: usize) -> Vec<Vec<u8>> {
        let mut out = vec![layer.to_vec()];
        let mut size = size;
        while size > 1 {
            let prev = out.last().unwrap();
            let half = size / 2;
            let mut next = vec![0u8; half * half * 4];
            for y in 0..half {
                for x in 0..half {
                    for c in 0..4 {
                        let at = |dx: usize, dy: usize| prev[((2 * y + dy) * size + 2 * x + dx) * 4 + c] as u32;
                        next[(y * half + x) * 4 + c] = ((at(0, 0) + at(1, 0) + at(0, 1) + at(1, 1)) / 4) as u8;
                    }
                }
            }
            out.push(next);
            size = half;
        }
        out
    }

    #[test]
    fn mip_chain_matches_the_per_texel_formula() {
        let mut seed = 12345u32;
        let px: Vec<u8> = (0..16 * 16 * 4)
            .map(|_| {
                seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
                (seed >> 24) as u8
            })
            .collect();
        assert_eq!(mips_of(&px, 16), reference(&px, 16));
    }
}

#[cfg(test)]
mod live_paint_tests {
    use super::*;
    use crate::scene::Finish;

    #[test]
    fn stock_colours_map_to_the_new_ones_as_emitted_and_only_the_pulsed_zone_gains() {
        let own = [[255, 50, 40], [255, 255, 255], [40, 40, 40]];
        let p = LivePaint { to: [Some([98, 46, 234]), None, Some([0, 0, 0])], pulse: Some((1, 0.3)), finish: [Finish::Gloss, Finish::Metallic, Finish::Pearl] };
        let rows = paint_rows(&p, own, [1.2, 1.0, 1.0]);
        // ObjectBrightness scales both sides without a clamp, as `scene::emit` writes the lit faces (`rgba_obj`, #317)
        assert_eq!(rows[0], [255.0 * 1.2 / 255.0, 50.0 / 255.0, 40.0 / 255.0, 1.0]);
        assert_eq!(rows[3], [98.0 * 1.2 / 255.0f32, 46.0 / 255.0, 234.0 / 255.0, 0.0]);
        // None keeps the stock colour (a no-op test that still carries the zone's pulse)
        assert_eq!(rows[4], [255.0 * 1.2 / 255.0, 1.0, 1.0, 0.3]);
        assert_eq!(rows[5], [0.0, 0.0, 0.0, 0.0]);
        // on; gloss, Metallic and Pearl zones
        assert_eq!(rows[..3].iter().map(|r| r[3]).collect::<Vec<_>>(), [1.0, 2.0, 3.0]);
    }
}

#[cfg(test)]
mod lamp_shadow_tests {
    use super::*;

    fn lamp(x: f32) -> LampLight {
        LampLight { pos: Vec3::new(x, 6.0, 0.0), range: 15.0, rgb: [1.0, 1.0, 1.0] }
    }

    #[test]
    fn matrix_maps_the_ground_below_to_the_centre() {
        let (view, max) = lamp_shadow_view(Vec3::new(3.0, 6.0, 2.0), 65.0);
        let m = perspective_bounds(view, max, 15.0);
        let c = m * glam::Vec4::new(3.0, 0.0, 2.0, 1.0);
        let (x, y, z) = (c.x / c.w, c.y / c.w, c.z / c.w);
        assert!(x.abs() < 1e-4 && y.abs() < 1e-4, "{x} {y}");
        assert!(z > 0.0 && z < 1.0, "{z}");
    }

    #[test]
    fn select_prefers_held_lamps_and_caps_layers() {
        let lamps: Vec<LampLight> = (0..8).map(|i| lamp(i as f32 * 20.0)).collect();
        let t = LampLightTune::default();
        // a car under each lamp, all near the camera
        let cars: Vec<Vec3> = (0..8).map(|i| Vec3::new(i as f32 * 20.0, 0.0, 0.0)).collect();
        let mut a = SpotAlloc::default();
        let w = spot_select(&lamps, &a, &cars, Vec3::new(0.0, 1.0, 0.0), &t);
        assert_eq!(w.len(), LAMP_LAYERS.min(w.len()));
        // only lamps with a car within `distance` of the camera count, nearest first
        assert_eq!(w[0], 0);
        a.step(&w, 0.0);
        assert!(a.layer_of(w[0]).is_some());
        // no car in any cone: nothing wanted
        assert!(spot_select(&lamps, &a, &[Vec3::new(500.0, 0.0, 500.0)], Vec3::ZERO, &t).is_empty());
    }

    #[test]
    fn head_select_wants_cars_with_a_body_in_the_beam() {
        let src = |key, x: f32| HeadSrc { key, pos: Vec3::new(x, 0.5, 0.0), fwd: Vec3::Z, up: Vec3::Y };
        let t = LampLightTune::default();
        let a = SpotAlloc::with_layers(HEAD_LAYERS);
        let sources = [src(0, 0.0), src(1, 40.0)];
        // car 1 stands 20 m ahead of car 0: only car 0 has a body in its beam
        let bodies = [(0, Vec3::new(0.0, 0.3, 0.0)), (1, Vec3::new(0.5, 0.3, 20.0))];
        assert_eq!(head_select(&sources, &bodies, &a, Vec3::ZERO, &t), vec![0]);
        // beyond the reach, beside the beam or behind: nobody
        for c in [Vec3::new(0.0, 0.3, t.head_reach + 10.0), Vec3::new(40.0, 0.3, 10.0), Vec3::new(0.0, 0.3, -10.0)] {
            assert!(head_select(&sources[..1], &[(1, c)], &a, Vec3::ZERO, &t).is_empty());
        }
        // a car too far from the camera gets no map
        assert!(head_select(&sources[..1], &bodies, &a, Vec3::new(0.0, 0.0, 1000.0), &t).is_empty());
    }

    #[test]
    fn head_matrix_projects_the_beam_axis_to_the_centre() {
        let s = HeadSrc { key: 0, pos: Vec3::new(1.0, 0.5, 2.0), fwd: Vec3::Z, up: Vec3::Y };
        let t = LampLightTune::default();
        let (view, max) = head_shadow_view(&s, &t);
        let c = perspective_bounds(view, max, t.head_reach) * glam::Vec4::new(1.0, 0.5, 22.0, 1.0);
        let (x, y, z) = (c.x / c.w, c.y / c.w, c.z / c.w);
        assert!(x.abs() < 1e-4 && y.abs() < 1e-4, "{x} {y}");
        assert!(z > 0.0 && z < 1.0, "{z}");
    }

    #[test]
    fn shaft_matrix_covers_frustum_and_casters() {
        let corners = [Vec3::new(-5.0, 0.0, 1.0), Vec3::new(5.0, 0.0, 1.0), Vec3::new(-5.0, 4.0, 1.0), Vec3::new(5.0, 4.0, 1.0), Vec3::new(-90.0, 0.0, 100.0), Vec3::new(90.0, 0.0, 100.0), Vec3::new(-90.0, 60.0, 100.0), Vec3::new(90.0, 60.0, 100.0)];
        let world = (Vec3::new(-200.0, -5.0, -200.0), Vec3::new(200.0, 80.0, 200.0));
        let sun = Vec3::new(0.5, 0.3, 0.2).normalize();
        let (m, depth) = shaft_matrix(&corners, world, sun).unwrap();
        for c in corners {
            let p = m.project_point3(c);
            assert!(p.x.abs() <= 1.0 && p.y.abs() <= 1.0 && p.z > 0.0 && p.z < 1.0, "{c} -> {p}");
        }
        // a caster on the sun's side of the frustum, at the top of the world, is in front of the map's near plane
        let p = m.project_point3(Vec3::new(0.0, 50.0, 50.0) + sun * 100.0);
        assert!(p.z > 0.0 && p.z < 0.5, "{p}");
        assert!(depth > 100.0);
        assert!(shaft_matrix(&corners, world, Vec3::new(1.0, 0.0, 0.0)).is_none());
    }

    #[test]
    fn fitted_window_holds_every_caster_and_is_tighter() {
        let (view, max) = lamp_shadow_view(Vec3::new(0.0, 6.0, 0.0), 80.0);
        let cars = [Vec3::new(8.0, 0.3, 3.0), Vec3::new(-5.0, 0.3, 12.0)];
        let (m, texel) = fit_shadow(view, max, &cars, 40.0, 2048, 1);
        for c in cars {
            // the car's ground point and its top corners stay inside the map
            for d in [Vec3::ZERO, Vec3::new(1.5, 1.0, 2.0), Vec3::new(-1.5, 1.0, -2.0)] {
                let p = m * (c + d).extend(1.0);
                let (x, y) = (p.x / p.w, p.y / p.w);
                assert!(x.abs() < 1.0 && y.abs() < 1.0, "{x} {y}");
            }
        }
        let (_, full) = fit_shadow(view, max, &[], 40.0, 2048, 1);
        assert!(texel < full * 0.5, "{texel} {full}");
    }

    #[test]
    fn fitted_window_falls_back_next_to_the_light() {
        let (view, max) = lamp_shadow_view(Vec3::new(0.0, 6.0, 0.0), 80.0);
        assert_eq!(fit_window(view, max, &[Vec3::new(0.0, 4.5, 0.0)], 1024, 1), max);
    }

    #[test]
    fn layers_fade_out_before_they_are_freed() {
        let mut a = SpotAlloc::default();
        a.step(&[3], 0.0);
        assert_eq!(a.layers[0].map(|l| l.fade), Some(1.0));
        a.step(&[], 0.1);
        assert!(a.layer_of(3).is_some());
        a.step(&[], 0.5);
        assert!(a.layer_of(3).is_none());
        // a later lamp fades in rather than popping
        a.step(&[4], 0.0);
        assert_eq!(a.layers[0].map(|l| l.fade), Some(0.0));
    }
}

#[cfg(test)]
mod car_detail_tests {
    use super::{car_levels, Camera, CarDetail};
    use glam::{Mat4, Vec3};

    /// 0x4395a8..0x4395d4: level 0 up to 20 m x zoom x width / 512, then level 1; the detail raises the
    /// first level (High 0, Medium 1, Low 2).
    #[test]
    fn generated_car_switch_distance_and_band() {
        use super::{gen_far, gen_switch_distance};
        // 2560 px, zoom 1: a 4.5 m car is 5760 / d px long, 30 px at 192 m
        let d = gen_switch_distance(100, 1.0, 2560.0);
        assert!((d - 192.0).abs() < 0.5, "{d}");
        // the percent scales the distance, the TV camera's zoom and the screen width push it out
        assert!((gen_switch_distance(200, 1.0, 2560.0) - 2.0 * d).abs() < 0.01);
        assert!((gen_switch_distance(100, 2.0, 2560.0) - 2.0 * d).abs() < 0.01);
        assert!(gen_switch_distance(100, 1.0, 1280.0) < d);
        // no memory: the plain threshold
        assert!(!gen_far(d - 1.0, d, None) && gen_far(d + 1.0, d, None));
        // inside the band the last frame's choice stays; outside it the distance decides
        assert!(gen_far(d * 0.9, d, Some(true)) && !gen_far(d * 1.1, d, Some(false)));
        assert!(!gen_far(d * 0.9, d, Some(false)) && gen_far(d * 1.1, d, Some(true)));
        assert!(!gen_far(d * 0.8, d, Some(true)) && gen_far(d * 1.2, d, Some(false)));
    }

    #[test]
    fn body_level_by_distance_and_detail() {
        // 640 px wide: 25 m
        assert_eq!(CarDetail::High.body_level(24.9, 1.0, 640.0), 0);
        assert_eq!(CarDetail::High.body_level(25.1, 1.0, 640.0), 1);
        // 1920 px: 75 m; the TV camera's zoom pushes it out
        assert_eq!(CarDetail::High.body_level(74.0, 1.0, 1920.0), 0);
        assert_eq!(CarDetail::High.body_level(76.0, 1.0, 1920.0), 1);
        assert_eq!(CarDetail::High.body_level(100.0, 2.0, 1920.0), 0);
        assert_eq!(CarDetail::Medium.body_level(1.0, 1.0, 1920.0), 1);
        assert_eq!(CarDetail::Medium.body_level(500.0, 1.0, 1920.0), 1);
        assert_eq!(CarDetail::Low.body_level(1.0, 1.0, 1920.0), 2);
        assert_eq!(CarDetail::Low.body_level(500.0, 1.0, 1920.0), 2);
    }

    /// Wheels follow their body's level and are left out where the body has them built in (Low), even
    /// when the list is reordered.
    #[test]
    fn wheels_follow_their_body() {
        let at = |x: f32| Mat4::from_translation(Vec3::new(x, 0.0, 0.0));
        let cam = Camera::new(Vec3::ZERO, 0.0, 0.0);
        // car 1's wheel listed before its body; car 0 near (25 m limit at 640 px), car 1 far
        let list = [(1, at(30.0)), (0, at(10.0)), (5, at(30.0)), (6, at(30.0))];
        assert_eq!(car_levels(&list, &cam, CarDetail::High, 640.0, 0), [Some(0), Some(0), Some(1), Some(0)]);
        assert_eq!(car_levels(&list, &cam, CarDetail::Low, 640.0, 0), [None, Some(2), Some(2), None]);
        // supercars / grem: every body at OBJ3, so no separate wheels at any distance
        assert_eq!(car_levels(&list, &cam, CarDetail::High, 640.0, 3), [None, Some(3), Some(3), None]);
    }
}
