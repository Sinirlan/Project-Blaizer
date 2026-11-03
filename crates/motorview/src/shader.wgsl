// Colour model recovered from motor.exe (re-notes/05-formats.md "Rendering colour model").
// Everything is in gamma space, like the original's D3D5 pipeline (MODULATE, vertex fog).
struct Globals {
    view_proj: mat4x4<f32>,
    view: mat4x4<f32>,
    model: mat4x4<f32>,
    // env-probe faces: inverse view-projection (sky directions)
    sky_inv: mat4x4<f32>,
    cam_pos: vec4<f32>,
    // x: fog start, y: fog end, z: brightness, w: fog enabled
    params: vec4<f32>,
    fog: vec4<f32>,
    // rgb: ObjectAmbient, w: 1 = lit object (cars), 0 = pre-lit world
    ambient: vec4<f32>,
    // rgb: sun colour * intensity, w: SpecularFactor
    sun: vec4<f32>,
    // xyz: towards the sun
    sun_dir: vec4<f32>,
    sky_top: vec4<f32>,
    sky_bottom: vec4<f32>,
    // x: image top (0 = screen top), y: image height, z: u at screen centre, w: u per screen width
    sky_geom: vec4<f32>,
    // x: car shading (0 none, 1 Reflection, 2 Enhanced), y: env-map layer, z: dynamic lights,
    // w: REMASTER flags (1 per-pixel light, 2 per-pixel fog, 4 clear coat, 8 linear output)
    env: vec4<f32>,
    // x: car light spots in use
    spot_n: vec4<f32>,
    // dynamic lights: (position, range), (rgb, 0)
    lights: array<vec4<f32>, 64>,
    // REMASTER car lights: (position, range), (rgb, kind: 0 headlight, 1 tail light),
    // (beam axis, 0), (car up, 0)
    spots: array<vec4<f32>, 128>,
    // MYW point lights (05-formats section 8) of the lit object being drawn:
    // (position, range), (rgb, LightIntensity * intensity) per light
    myw: array<vec4<f32>, 32>,
    // x: lights in use, y: 1 = the light origin is the model's translation (cars, wheels), 0 =
    // the vertex (scene objects, already in world space), 2 = CLASSIC scene objects with their own
    // light records (`prop_myw`, `prop_of`)
    myw_n: vec4<f32>,
    // REMASTER street lamps: (position, range), (rgb, 0)
    lamps: array<vec4<f32>, 32>,
    // x: lamps in use, y/z: cosine of the cone's inner and outer half angle
    lamp_n: vec4<f32>,
    // REMASTER water: 7 vec4 per body (procgen/water.rs `Params::packed`; the code is water.wgsl)
    water: array<vec4<f32>, 28>,
    // x: animation time (s), y: 1 = the water pass has the scene copy and depth (group 3),
    // z: near plane, w: far plane
    water_t: vec4<f32>,
    // REMASTER fog (render.rs FogTune, 17-fog-rework.md): x density per metre at the ground, y ground
    // height, z 1 / height scale, w sun glow strength
    fog2: vec4<f32>,
    // x: sun glow anisotropy (HG g); y/z: the sky dome's turn, exposure; w: the fog volume's far distance in a volumetric view (0 otherwise)
    fog3: vec4<f32>,
    // rgb: the fog colour in linear light, w: sky haze (how far up the sky the fog climbs)
    fog4: vec4<f32>,
    // x: start, y: full (m), z: opacity, w: curve of the distance ramp
    fog5: vec4<f32>,
    // REMASTER grass: 5 vec4 per body (procgen/grass.rs `Params::packed`; the code is grass.wgsl)
    grass: array<vec4<f32>, 20>,
    // REMASTER grass: the cars pressing the blades, 3 vec4 per car: (position, hull half
    // length), (forward xz, hull half width, 0), (velocity xz, 0, 0); grass_n.x of them in use
    grass_cars: array<vec4<f32>, 24>,
    // x: cars pressing; y: the edge map's metres at 1 (0: no map, the vertex fade is used);
    // z: 1 = the Ultra blades are drawn (the shells hand over to them near the camera);
    // w: pixels per metre at 1 m depth (the blades' pixel floor)
    grass_n: vec4<f32>,
    // REMASTER grass: the edge map's transform, uv = (p.xz - xy) * zw
    grass_map: vec4<f32>,
    // REMASTER sky ambient: the HDRI's irradiance as 9 L2 terms (rgb, linear; render.rs `sky_sh`,
    // sky.rs `irradiance_sh`); [0].w = 1 when it replaces `ambient` on lit objects
    sky_sh: array<vec4<f32>, 9>,
    // REMASTER local light (#83): x 1 = the probe volume is bound, y bounce gain, z sky
    // occlusion strength
    local: vec4<f32>,
    // xyz: the brick coordinates of the volume's low corner
    local_box: vec4<f32>,
    // REMASTER hologram clip (`HoloClip::rects`, the glass cards): centre xy, half extents zw
    holo_rects: array<vec4<f32>, 16>,
    // x: those rectangles' corner radius, y: how many are in use
    holo_n: vec4<f32>,
    // REMASTER live paint (`LivePaint`, car bodies only): 0..2 = stock colour k as emitted, w 1 = on;
    // 3..5 = its new colour, w = brightness gain
    paint: array<vec4<f32>, 6>,
    // rgb: the user's Brightness per channel, times the lightning gain in lemmy / ace "Hell mode" (#316)
    gain: vec4<f32>,
    // Hell mode's cloud-ceiling sky (`0x443570`): 2 vec4 per vertex, 4 for the textured quad, 4 for the plain one below:
    // (ndc x, ndc y, w = 1 / rhw, 1 textured), (u, v, 0 top colour .. 1 bottom colour, 0)
    hell: array<vec4<f32>, 16>,
    // CHEAT tribute to tron, REMASTER Neon grid (#320): x = half the edge width in render pixels
    tron: vec4<f32>,
};

@group(0) @binding(0) var<uniform> g: Globals;
@group(0) @binding(1) var tex: texture_2d_array<f32>;
@group(0) @binding(2) var samp: sampler;
@group(0) @binding(3) var sky_tex: texture_2d<f32>;
// Enhanced car shading: the surroundings around the followed car, world directions
@group(0) @binding(4) var env_cube: texture_cube<f32>;
// REMASTER sky dome HDRI, equirectangular, linear radiance (render.rs `dome_azimuth`)
@group(0) @binding(5) var sky_hdri: texture_2d<f32>;
// REMASTER local light (#83, probes.rs `Packed`): slot + 1 per 8 m brick (0 = none), and the
// atlas of the bricks' probes, the L1 terms of vis, bounce r, g, b a quarter of the depth each
@group(0) @binding(6) var irr_index: texture_3d<u32>;
@group(0) @binding(7) var irr_atlas: texture_3d<f32>;
// and the probes' share of the sun (#192), laid out like one quarter of the atlas
@group(0) @binding(8) var irr_sun: texture_3d<f32>;
// REMASTER: the same filtering with the chosen anisotropy (render.rs `Gpu::aniso_bind`)
@group(1) @binding(0) var samp_hq: sampler;
// REMASTER texture pack (#211, texpack.rs): the user's upscaled colour and normal + roughness
// layers, and per layer of `tex` the ids of its pair (x colour, y normal; -1 = the original),
// two layers to a vec4
@group(0) @binding(9) var tex_hi: texture_2d_array<f32>;
@group(0) @binding(10) var<uniform> pack_map: array<vec4<i32>, 256>;
fn pack_ids(layer: f32) -> vec2<i32> {
    if (layer < 0.0) {
        return vec2<i32>(-1);
    }
    let l = min(u32(layer + 0.5), 511u);
    let m = pack_map[l >> 1u];
    return select(m.xy, m.zw, (l & 1u) == 1u);
}
// the pack's sun highlight strength, and the clamp of its normal map's relight of the baked
// world light [design choice]
const PACK_SPEC: f32 = 0.35;
const PACK_RELIGHT_MIN: f32 = 0.5;
const PACK_RELIGHT_MAX: f32 = 1.5;

// REMASTER car shadows: the sun's depth map of the cars near the camera (render.rs `ShadowMap`)
struct ShadowU {
    // world -> light clip space (orthographic, depth 0..1 over p.x metres)
    vp: mat4x4<f32>,
    // x: depth range in metres, y: 1 / map size, z: depth bias (map units)
    p: vec4<f32>,
};
@group(2) @binding(0) var<uniform> sh_u: ShadowU;
@group(2) @binding(1) var sh_map: texture_depth_2d;
@group(2) @binding(2) var sh_samp: sampler_comparison;

// REMASTER lamp shadows (render.rs `SpotU`, 11-dynamic-shadows.md): the depth atlas of the cars
// seen from the street lamps, one perspective map per layer
struct SpotU {
    vp: array<mat4x4<f32>, 6>,
    // (lamp position, shadow range)
    light: array<vec4<f32>, 6>,
    // x: fade (0..1), y: depth bias in metres towards the lamp, z: 1 / map size,
    // w: cosine of the shadow cone's half angle
    p: array<vec4<f32>, 6>,
    // x: tan of the view angle per texel, y: PCF radius in texels
    q: array<vec4<f32>, 6>,
};
@group(2) @binding(3) var sp_map: texture_depth_2d_array;
@group(2) @binding(4) var<uniform> sp_u: SpotU;

// REMASTER car shadows [design choice]: the world keeps its baked light, so a shadow on it
// only darkens, by the sun's share of the light (sun / (sun + ambient), times the gain, up to
// SHADOW_WORLD; the original's projected shadow darkens to 0.4). Cars lose the sun term of
// their light instead.
const SHADOW_WORLD: f32 = 0.6;
const SHADOW_SHARE_GAIN: f32 = 1.5;
// Overhead mode (dim tracks, render.rs `OVERHEAD_DARKNESS`; `sh_u.p.w` > 0 = soft radius in map uv): the
// map looks straight down, the world darkens by a fixed amount, lit objects are not shadowed.
const OVERHEAD_WORLD: f32 = 0.55;
const OVERHEAD_TAPS: i32 = 16;
// Lit surfaces (cars, scene objects) are lifted by a few map texels before the lookup (#340): along
// the normal by SHADOW_NORMAL_TEXELS x (1 - N.L), along the sun by SHADOW_DEPTH_TEXELS, each at least
// SHADOW_LIFT_MIN metres. A fixed 6 cm + 8 cm lift erased the shadow of parts closer to the body than
// that (the Serpent's wing over its deck, 8-12 cm) and left diagonal stripes where the test sat on its
// threshold. The world keeps the fixed `sh_u.p.z` (it casts nothing into the map).
const SHADOW_NORMAL_TEXELS: f32 = 1.5;
const SHADOW_DEPTH_TEXELS: f32 = 1.0;
const SHADOW_LIFT_MIN: f32 = 0.005;

// REMASTER: set on the multisampled mesh pipelines, where colour-keyed texels turn into
// sample coverage instead of a hard discard (alpha to coverage)
override ALPHA_TO_COVERAGE: bool = false;

// The env cube holds linear HDR light (render.rs `EnvProbe`: the sun and the lamps are over 1);
// the paint models mix in gamma space, so a sample is decoded first.
fn env_gamma(c: vec3<f32>) -> vec3<f32> {
    return pow(max(c, vec3<f32>(0.0)), vec3<f32>(1.0 / GAMMA));
}
// REMASTER clear coat over the Reflection env layer [design choice]: Schlick Fresnel with a
// lacquer's head-on reflectance, so the sphere map shows mostly at grazing angles.
const GLOSS_F0: f32 = 0.04;
// REMASTER polished metal (generated rims, procgen.rs `METAL_ENV`) [design choice]: head-on
// reflectance tinted by the part's hue, a tight sun highlight and its gain (times
// SpecularFactor), the diffuse of a bright aluminium base, how far the reflection is greyed
// (aluminium, not chrome) and the spread of the 5 cube taps that soften it (the cube has no
// mips).
const METAL_F0: f32 = 0.6;
const METAL_SHINE: f32 = 220.0;
const METAL_SPEC: f32 = 2.0;
const METAL_BASE: f32 = 0.6;
const METAL_DIFFUSE: f32 = 0.5;
const METAL_GREY: f32 = 0.35;
const METAL_SPREAD: f32 = 0.08;
// REMASTER car paint finishes (#136, #351, scene.rs `METALLIC_ENV` / `PEARL_ENV`, paint.wgsl, 09-remaster 9.41) [design
// choice]: the spread of the 5 cube taps of the metallic flakes' reflection (blurrier than polished metal), how far
// the sun's colour takes the paint's hue on the flake glints, the grey stand-in's share where there is no reflection
// source, and the flake glints': the pixels a cell spans at least (coarser cells, doubling, further off), then per
// finish the finest cell (m, car space), normal tilt, exponent and gain (divided by 1 + the doublings).
const METALLIC_SPREAD: f32 = 0.15;
const PAINT_FLAKE_PX: f32 = 2.0;
const PAINT_FLAKE_SUN_TINT: f32 = 0.6;
const PAINT_PLAIN_ENV: f32 = 0.6;
const METALLIC_FLAKE_CELL: f32 = 0.001;
const METALLIC_FLAKE_TILT: f32 = 0.35;
const METALLIC_FLAKE_SHINE: f32 = 400.0;
const METALLIC_FLAKE_GAIN: f32 = 3.0;
const PEARL_FLAKE_CELL: f32 = 0.001;
const PEARL_FLAKE_TILT: f32 = 0.2;
const PEARL_FLAKE_SHINE: f32 = 250.0;
const PEARL_FLAKE_GAIN: f32 = 0.5;
// REMASTER env-mapped scene objects (scene.rs `PROP_ENV`, 05-formats "Env-mapped polygons") [design
// choice]: the share of the original's picture kept under the live reflection, the gain of this
// frame's env cube, the spread of the 5 cube taps (as METAL_SPREAD), and the sun glint's
// exponent and gain (times SpecularFactor).
const PROP_ENV_BASE: f32 = 0.35;
const PROP_ENV_GAIN: f32 = 1.6;
const PROP_ENV_SPREAD: f32 = 0.08;
const PROP_ENV_SHINE: f32 = 90.0;
const PROP_ENV_SPEC: f32 = 1.5;
// REMASTER per-pixel car light [design choice]: Blinn-Phong exponent of the highlight, scaled
// by (p + 8) / (4 + 8) so it holds the energy of the original's (N.H)^4 lobe.
const PP_SHINE: f32 = 12.0;
// REMASTER carbon fibre (generated parts, procgen.rs `CARBON_ENV`) [design choice]: the weave's
// own highlight (exponent from the roughness map, gain times SpecularFactor) under a thin
// lacquer (head-on reflectance, tight sun highlight)
const CARBON_SPEC: f32 = 0.6;
// REMASTER tyre rubber (procgen.rs `TYRE_ENV`) [design choice]: matte, a faint highlight
const TYRE_SPEC: f32 = 0.12;
const CARBON_F0: f32 = 0.05;
const CARBON_COAT_SHINE: f32 = 96.0;
const CARBON_COAT_SPEC: f32 = 0.8;
// REMASTER HDR: the gamma of the 1998 colours. A pure power keeps products exact
// (lin(a) * lin(b) = lin(a * b)), so textured, lit surfaces look the same as in gamma space
// and only sums (blending, halos, MSAA resolve, bloom) change.
const GAMMA: f32 = 2.2;
// REMASTER HDR: halo light gain in linear light (fs_halo; smooth halos without bloom)
const HALO_HDR_GAIN: f32 = 1.0;
// REMASTER smooth halos (fs_halo_remaster) [design choice]: radial profile in quad UV space,
// core exp(-r^2 / sigma^2) plus tail (1 - smoothstep(0, R, r)) / (1 + k r^2). Least-squares
// fit to the radial profile of the original halo IFFs (peak 1, mean 0.047 over the quad vs
// their 0.049), so a halo keeps its brightness and extent.
const HALO_SIGMA: f32 = 0.12;
const HALO_TAIL_K: f32 = 4.0;
const HALO_TAIL_R: f32 = 0.8;
const HALO_CORE_W: f32 = 0.6;
const HALO_TAIL_W: f32 = 0.4;
// REMASTER HDR with bloom: the core goes over the bloom threshold, the bloom makes the glow
// and the tail stays faint (in place of HALO_HDR_GAIN; on a quad of REMASTER_HALO_SIZE)
const HALO_CORE_GAIN: f32 = 1.5;
const HALO_TAIL_GAIN: f32 = 1.0;
// REMASTER lens look of smooth halos [design choice]: a second, square, screen-aligned quad
// per halo (colour alpha 2, render.rs halo_quads) adds what a stopped-down lens (~f/9) adds
// to a point light: diffraction spikes of an 8-blade aperture (8 rays, fixed on screen, each
// a little longer or shorter), fine streaks between them, and a faint ring with a red outer
// and blue inner edge. The glare is the elongated quad above.
const BURST_RAYS: f32 = 8.0;
const BURST_ANGLE: f32 = 0.3;
const BURST_WIDTH: f32 = 0.018;
const BURST_GAIN: f32 = 0.5;
const STREAKS: f32 = 47.0;
const STREAK_WIDTH: f32 = 0.006;
const STREAK_GAIN: f32 = 0.12;
const RING_R: f32 = 0.42;
const RING_W: f32 = 0.05;
const RING_CHROMA: f32 = 0.03;
const RING_GAIN: f32 = 0.03;
const BURST_LDR_GAIN: f32 = 1.5;

struct VsIn {
    @location(0) pos: vec3<f32>,
    @location(1) uv: vec2<f32>,
    @location(2) color: vec4<f32>,
    @location(3) layer: f32,
    @location(4) normal: vec3<f32>,
    @location(5) env: f32,
};

struct VsOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) color: vec4<f32>,
    @location(2) layer: f32,
    @location(3) fog: f32,
    @location(4) env_uv: vec2<f32>,
    @location(5) env: f32,
    @location(6) wpos: vec3<f32>,
    @location(7) wnormal: vec3<f32>,
    // what the dynamic lights multiply: the face colour of lit objects, 1 for the world
    @location(8) albedo: vec3<f32>,
    // CHEAT tribute to tron (#320): the triangle's corner as a one-hot (vs_main), 0 at the edge opposite each corner
    @location(9) bary: vec3<f32>,
};

// Dynamic lights (ours, render.rs PROP_LIGHT_RANGE) [design choice]: (1 - d/range)^2 falloff
// like the original's point lights (0x4353cc), times a wrapped N.L so the road under a light
// that sits barely above it is still lit.
fn dynamic_light(p: vec3<f32>, n: vec3<f32>) -> vec3<f32> {
    var sum = vec3<f32>(0.0);
    let count = u32(g.env.z + 0.5);
    for (var i = 0u; i < count; i++) {
        let a = g.lights[2u * i];
        let d = a.xyz - p;
        let dist = length(d);
        if (dist < a.w) {
            let f = 1.0 - dist / a.w;
            let nl = max(dot(n, d / max(dist, 1e-3)), 0.0);
            sum += g.lights[2u * i + 1u].rgb * f * f * (0.35 + 0.65 * nl);
        }
    }
    let spots = u32(g.spot_n.x + 0.5);
    for (var i = 0u; i < spots; i++) {
        let a = g.spots[4u * i];
        let d = p - a.xyz;
        let dist = length(d);
        if (dist < a.w && dist > 1e-3) {
            let l = d / dist;
            let axis = g.spots[4u * i + 2u].xyz;
            let z = dot(l, axis);
            if (z > 0.1) {
                let up = g.spots[4u * i + 3u].xyz;
                let x = dot(l, cross(axis, up)) / z;
                let y = dot(l, up) / z;
                let f = 1.0 - dist / a.w;
                let nl = max(dot(n, -l), 0.0);
                let c = g.spots[4u * i + 1u];
                // tail lights: a wide cone, fading out towards the side
                let cone = select(headlight(x, y), smoothstep(0.1, 0.6, z), c.w > 0.5);
                // REMASTER headlight shadows: a car with an atlas layer (`up.w`) blocks its beam
                var vis = 1.0;
                let layer = g.spots[4u * i + 3u].w;
                if (layer >= 0.0 && flag(512u)) {
                    vis = spot_vis(u32(layer + 0.5), p, n);
                }
                sum += c.rgb * cone * f * f * (0.35 + 0.65 * nl) * vis;
            }
        }
    }
    return sum;
}

// REMASTER lamp shadows: how much of lamp layer `layer` reaches `p` (1 = all). The point moves
// `bias` metres towards the lamp and a little along its normal, so the map's depth needs no
// bias of its own; PCF 3x3 (5x5 at Ultra); no shadow outside the map or past its depth range.
const SPOT_NORMAL_OFFSET: f32 = 0.06;
fn spot_vis(layer: u32, p: vec3<f32>, n: vec3<f32>) -> f32 {
    let lt = sp_u.light[layer];
    // the bias grows with the texel's size at this distance (a fitted map needs little)
    let bias = max(sp_u.p[layer].y, 1.5 * sp_u.q[layer].x * distance(lt.xyz, p));
    let q = p + n * SPOT_NORMAL_OFFSET + normalize(lt.xyz - p) * bias;
    let lp = sp_u.vp[layer] * vec4<f32>(q, 1.0);
    if (lp.w <= 0.0) {
        return 1.0;
    }
    let ndc = lp.xyz / lp.w;
    if (ndc.z <= 0.0 || ndc.z >= 1.0) {
        return 1.0;
    }
    let uv = vec2<f32>(ndc.x * 0.5 + 0.5, 0.5 - ndc.y * 0.5);
    // the fitted window leaves no use for an edge fade: outside it nothing casts
    if (uv.x < 0.0 || uv.x > 1.0 || uv.y < 0.0 || uv.y > 1.0) {
        return 1.0;
    }
    let k = i32(sp_u.q[layer].y);
    var sum = 0.0;
    for (var y = -k; y <= k; y++) {
        for (var x = -k; x <= k; x++) {
            sum += textureSampleCompareLevel(sp_map, sh_samp, uv + vec2<f32>(f32(x), f32(y)) * sp_u.p[layer].z, layer, ndc.z);
        }
    }
    let taps = f32((2 * k + 1) * (2 * k + 1));
    return mix(1.0, sum / taps, sp_u.p[layer].x);
}

// REMASTER lamp shadows on the baked world: it keeps its pool of light, which a car's shadow
// only darkens, by `lamp_n.w` times the lamp's cone and (square-rooted) falloff (an estimate: the bake does not
// say how much of its light is the lamp's [U]). Returns the factor for the colour.
fn lamp_world_shadow(p: vec3<f32>, n: vec3<f32>) -> f32 {
    var m = 1.0;
    let count = u32(g.lamp_n.x + 0.5);
    for (var i = 0u; i < count; i++) {
        let layer = g.lamps[2u * i + 1u].w;
        if (layer < 0.0) {
            continue;
        }
        // the shadow has its own, wider reach and cone than the lamp's light (SpotU)
        let k = u32(layer + 0.5);
        let a = g.lamps[2u * i];
        let range = sp_u.light[k].w;
        let d = p - a.xyz;
        let dist = length(d);
        if (dist < range && dist > 1e-3) {
            let cs = sp_u.p[k].w;
            let cone = smoothstep(cs, mix(cs, 1.0, 0.3), -d.y / dist);
            // sqrt: a gentler falloff than the light's own, so the shadow stays strong near the car
            let f = sqrt(1.0 - dist / range);
            let vis = spot_vis(k, p, n);
            m *= 1.0 - min((1.0 - vis) * g.lamp_n.w * cone * f, 1.0);
        }
    }
    return m;
}

// REMASTER street lamps (render.rs `LampLight`, 11-dynamic-shadows.md): a downward cone with the
// same (1 - d/range)^2 falloff and wrapped N.L as the other dynamic lights. Lit objects only;
// a lamp with a shadow layer (`.w` of its colour) is multiplied by its visibility.
fn lamp_light(p: vec3<f32>, n: vec3<f32>) -> vec3<f32> {
    var sum = vec3<f32>(0.0);
    let count = u32(g.lamp_n.x + 0.5);
    for (var i = 0u; i < count; i++) {
        let a = g.lamps[2u * i];
        let d = p - a.xyz;
        let dist = length(d);
        if (dist < a.w && dist > 1e-3) {
            let l = d / dist;
            let cone = smoothstep(g.lamp_n.z, g.lamp_n.y, -l.y);
            let f = 1.0 - dist / a.w;
            let nl = max(dot(n, -l), 0.0);
            let layer = g.lamps[2u * i + 1u].w;
            var vis = 1.0;
            if (flag(512u) && layer >= 0.0) {
                vis = spot_vis(u32(layer + 0.5), p, n);
            }
            sum += g.lamps[2u * i + 1u].rgb * cone * f * f * (0.35 + 0.65 * nl) * vis;
        }
    }
    return sum;
}

// REMASTER retro-reflective signs [design choice, 09-remaster 9.14]: a retro-reflector sends
// light back towards its source, so the headlights (spot kind 0) count by how closely the eye
// sits on the light's line (a lobe wider than a real sheet's 1-2 degrees, so the chase camera
// sees the flare too), with the beam's cone, the range falloff and a fade for grazing light.
const RETRO_K: f32 = 60.0;
// reach of the flare in metres: the headlight spots' own 40 m stops short of the signs a driver sees
const RETRO_RANGE: f32 = 140.0;
const RETRO_GAIN: f32 = 1.0;
const RETRO_MAX: f32 = 4.0;
const RETRO_F0: f32 = 0.04;
const RETRO_SHEEN: f32 = 0.6;
fn retro_light(p: vec3<f32>, n: vec3<f32>, vd: vec3<f32>) -> vec3<f32> {
    var sum = vec3<f32>(0.0);
    let spots = u32(g.spot_n.x + 0.5);
    for (var i = 0u; i < spots; i++) {
        let a = g.spots[4u * i];
        let c = g.spots[4u * i + 1u];
        let d = p - a.xyz;
        let dist = length(d);
        if (c.w < 0.5 && dist < RETRO_RANGE && dist > 1e-3) {
            let l = d / dist;
            let axis = g.spots[4u * i + 2u].xyz;
            let z = dot(l, axis);
            if (z > 0.1) {
                let up = g.spots[4u * i + 3u].xyz;
                let cone = headlight_open(dot(l, cross(axis, up)) / z, dot(l, up) / z);
                let f = 1.0 - dist / RETRO_RANGE;
                let lobe = pow(max(dot(-l, vd), 0.0), RETRO_K);
                let enter = smoothstep(0.2, 0.6, abs(dot(n, l)));
                // a car in the beam blocks the flare (beyond the map's reach: unshadowed, APPROX)
                var vis = 1.0;
                let layer = g.spots[4u * i + 3u].w;
                if (layer >= 0.0 && flag(512u)) {
                    vis = spot_vis(u32(layer + 0.5), p, n);
                }
                // linear falloff: a reflector keeps its flare at a distance, unlike a lit surface
                sum += c.rgb * cone * f * lobe * enter * vis;
            }
        }
    }
    return min(sum * RETRO_GAIN, vec3<f32>(RETRO_MAX));
}

// REMASTER headlight cookie [design choice, procedural: no shipped art]: a low beam in the
// lamp's tangent plane (x right, y up, both tan of the angle off the car's forward axis).
// A wide flat pool with a soft edge, a hot spot about 10 m ahead of a lamp 0.5 m up
// (y = -0.05), and a cutoff just above the axis that lets 12 % through (walls and cars
// ahead still catch a little light).
fn headlight(x: f32, y: f32) -> f32 {
    let yc = y + 0.05;
    let e = (x * x) / (0.6 * 0.6) + (yc * yc) / (0.25 * 0.25);
    let pool = 1.0 - smoothstep(0.4, 1.0, e);
    let hot = exp(-(x * x) / 0.05 - (yc * yc) / 0.0015);
    let cut = mix(0.12, 1.0, 1.0 - smoothstep(-0.02, 0.03, y));
    return (0.5 * pool + 0.8 * hot) * cut;
}

// REMASTER the same beam without the cutoff, for retro-reflective signs: a sign face hangs
// above the cutoff line, and real low beams do light signs [design choice]
fn headlight_open(x: f32, y: f32) -> f32 {
    let yc = y + 0.05;
    let e = (x * x) / (0.6 * 0.6) + (yc * yc) / (0.4 * 0.4);
    return 0.5 * (1.0 - smoothstep(0.4, 1.0, e)) + 0.8 * exp(-(x * x) / 0.05 - (yc * yc) / 0.02);
}

// CLASSIC scene objects: the point lights of each object's own cell and its own origin (0x439134 ->
// 0x4353cc), 17 vec4 per object: (origin, count), then (position, range), (rgb, scale) per light
// (render.rs `PropLightGpu`). `prop_of` = the record of each vertex of the props buffer.
@group(0) @binding(11) var<storage, read> prop_myw: array<vec4<f32>>;
@group(0) @binding(12) var<storage, read> prop_of: array<u32>;
const MYW_REC: u32 = 17u;
// the record of the vertex being lit (`vs_main` sets it when `myw_n.y` = 2), -1: `g.myw`, the
// light list of the draw
var<private> prop_rec: i32 = -1;

fn myw_count() -> u32 {
    if (prop_rec >= 0) {
        return u32(prop_myw[u32(prop_rec) * MYW_REC].w + 0.5);
    }
    return u32(g.myw_n.x + 0.5);
}

fn myw_pos(i: u32) -> vec4<f32> {
    if (prop_rec >= 0) {
        return prop_myw[u32(prop_rec) * MYW_REC + 1u + 2u * i];
    }
    return g.myw[2u * i];
}

fn myw_col(i: u32) -> vec4<f32> {
    if (prop_rec >= 0) {
        return prop_myw[u32(prop_rec) * MYW_REC + 2u + 2u * i];
    }
    return g.myw[2u * i + 1u];
}

// MYW point light strength (0x4353cc): k = (1 - d/range)^2 * LightIntensity * intensity, above
// 0.4 -> 0.4, a negative k (a light that darkens) x 0.25. `b` = LightIntensity * intensity.
fn myw_k(b: f32, range: f32, d: f32) -> f32 {
    var f = 0.0;
    if (range > 1.0 && d <= range) {
        f = 1.0 - d / range;
    }
    let k = min(f * f * b, 0.4);
    return select(k, k * 0.25, k < 0.0);
}

// CLASSIC: the lights of the object's cell (0x42ce44), light direction and eye taken from the
// object's origin `o`: N.L, plus (N.L + (R.E)^4 * SpecularFactor) for positive lights when
// R.E > 0.5, R = 2 (N.L) N - L, times the light's colour * k.
fn myw_object(n: vec3<f32>, o: vec3<f32>) -> vec3<f32> {
    var sum = vec3<f32>(0.0);
    let e = normalize(g.cam_pos.xyz - o);
    let count = myw_count();
    for (var i = 0u; i < count; i++) {
        let a = myw_pos(i);
        let b = myw_col(i);
        let d = a.xyz - o;
        let dist = length(d);
        let l = d / max(dist, 1e-3);
        let nl = dot(n, l);
        if (nl > 0.0) {
            var s = nl;
            if (b.w > 0.0) {
                let rs = dot(2.0 * nl * n - l, e);
                if (rs > 0.5) {
                    s += rs * rs * rs * rs * g.sun.w;
                }
            }
            sum += s * myw_k(b.w, a.w, dist) * b.rgb;
        }
    }
    return sum;
}

// REMASTER: the same lights per pixel (true distance and direction from the surface point),
// the (R.E)^4 cut-off lobe replaced by the smooth Blinn-Phong of `object_light_pp`.
fn myw_light_pp(n: vec3<f32>, p: vec3<f32>, vd: vec3<f32>) -> vec3<f32> {
    var sum = vec3<f32>(0.0);
    let count = u32(g.myw_n.x + 0.5);
    for (var i = 0u; i < count; i++) {
        let a = g.myw[2u * i];
        let b = g.myw[2u * i + 1u];
        let d = a.xyz - p;
        let dist = length(d);
        let l = d / max(dist, 1e-3);
        let nl = dot(n, l);
        if (nl > 0.0) {
            var s = nl;
            if (b.w > 0.0) {
                s += pow(max(dot(n, normalize(l + vd)), 0.0), PP_SHINE) * (PP_SHINE + 8.0) / 12.0 * g.sun.w;
            }
            sum += s * myw_k(b.w, a.w, dist) * b.rgb;
        }
    }
    return sum;
}

// CLASSIC per-vertex object light (0x42ce44): ambient + max(0, N.L) * sun + the MYW point
// lights, clamped to 0..1 per channel. The sun has no highlight in the original (the
// specular term belongs to the list lights).
fn object_light(n: vec3<f32>, world: vec3<f32>) -> vec3<f32> {
    return object_light_amb(n, world, g.ambient.rgb, 1.0);
}

// CLASSIC: the even light of a polygon (0x42d044): ObjectAmbient plus 1.5 x the colours of the
// cell's point lights (k as `myw_k`, measured from `o`), no sun and no N.L, clamped to 0..1. The
// original lights env-mapped scene-object polygons and, with Flat car shading, every polygon of a
// lit object this way.
fn flat_light(o: vec3<f32>) -> vec3<f32> {
    var sum = vec3<f32>(0.0);
    let count = myw_count();
    for (var i = 0u; i < count; i++) {
        let a = myw_pos(i);
        let b = myw_col(i);
        sum += myw_k(b.w, a.w, distance(a.xyz, o)) * b.rgb;
    }
    return clamp(g.ambient.rgb + 1.5 * sum, vec3<f32>(0.0), vec3<f32>(1.0));
}

// the env-mapped scene object's light: from the object's origin when it has its own record
// (CLASSIC scene objects), else from the vertex [APPROX]
fn prop_env_light(vertex: vec3<f32>) -> vec3<f32> {
    var o = vertex;
    if (prop_rec >= 0) {
        o = prop_myw[u32(prop_rec) * MYW_REC].xyz;
    }
    return flat_light(o);
}

// CLASSIC: the env texture's coordinates of an env-mapped scene-object vertex (0x433354 / 0x434080):
// texel = S (0.5 + 0.2475 (n + s)), n the normal in view space, s the screen position in half
// widths from the centre (x right, y up, both over the half *width*), each clamped to the image
// (0..255). The image is square, so these are uv. The v sign puts the picture's sky on up-facing
// normals [I: the original's row order is not read].
fn prop_env_uv(n_world: vec3<f32>, clip: vec4<f32>) -> vec2<f32> {
    let nv = normalize((g.view * vec4<f32>(n_world, 0.0)).xyz);
    let s = clip.xy / clip.w;
    let uv = vec2<f32>(0.5 + 0.2475 * (nv.x + s.x), 0.5 - 0.2475 * (nv.y + s.y * g.cam_pos.w));
    return clamp(uv, vec2<f32>(0.0), vec2<f32>(255.0 / 256.0));
}

// `object_light` with the ambient term and the share of the sun given (REMASTER: `sky_ambient`,
// `local_sun`)
fn object_light_amb(n: vec3<f32>, world: vec3<f32>, amb: vec3<f32>, sun: f32) -> vec3<f32> {
    let k = max(dot(n, g.sun_dir.xyz), 0.0) * sun;
    var o = select(world, g.model[3].xyz, g.myw_n.y > 0.5);
    if (prop_rec >= 0) {
        o = prop_myw[u32(prop_rec) * MYW_REC].xyz;
    }
    return clamp(amb + k * g.sun.rgb + myw_object(n, o), vec3<f32>(0.0), vec3<f32>(1.0));
}

// REMASTER sky ambient (09-remaster 9.32) [design choice]: the ambient light of a lit object from the
// sky dome's irradiance (L2 spherical harmonics, already scaled to the luminance of ObjectAmbient),
// gamma-encoded like the rest of the object light. The normal is turned into the image's frame by
// the dome's turn (fog3.y), the same turn `dome_radiance` applies. Without a dome: `ambient`.
fn sky_ambient(n: vec3<f32>) -> vec3<f32> {
    if (g.sky_sh[0].w < 0.5) {
        return g.ambient.rgb;
    }
    let c = cos(g.fog3.y);
    let s = sin(g.fog3.y);
    let d = vec3<f32>(n.x * c + n.z * s, n.y, n.z * c - n.x * s);
    var e = g.sky_sh[0].rgb + g.sky_sh[1].rgb * d.y + g.sky_sh[2].rgb * d.z + g.sky_sh[3].rgb * d.x;
    e += g.sky_sh[4].rgb * (d.x * d.y) + g.sky_sh[5].rgb * (d.y * d.z) + g.sky_sh[6].rgb * (3.0 * d.z * d.z - 1.0);
    e += g.sky_sh[7].rgb * (d.x * d.z) + g.sky_sh[8].rgb * (d.x * d.x - d.y * d.y);
    return pow(max(e, vec3<f32>(0.0)), vec3<f32>(1.0 / GAMMA));
}

// REMASTER local light (#83) [design choice]: the ambient of a lit object at `p` from the probe
// volume, `lin(sky) * vis(n) + bounce(n)` gamma-encoded like `sky_ambient` (`sky`, returned as
// it is outside the bricks). The probes hold L1 terms folded like `sky_ambient`'s (probes.rs
// `Probe`); the border probes are in both bricks, so the hardware trilinear needs no seams.
const IRR_BRICK: f32 = 8.0;
const IRR_STEP: f32 = 2.0;
const IRR_SIDE: f32 = 5.0;
const IRR_SLOTS: u32 = 32u;
// The atlas coordinates of `p` in its brick's slot (.w = 1), .w = 0 outside the bricks.
fn local_uvw(p: vec3<f32>) -> vec4<f32> {
    let bf = floor(p / IRR_BRICK);
    let b = vec3<i32>(bf) - vec3<i32>(g.local_box.xyz);
    let dim = vec3<i32>(textureDimensions(irr_index));
    if (any(b < vec3<i32>(0)) || any(b >= dim)) {
        return vec4<f32>(0.0);
    }
    let slot = textureLoad(irr_index, b, 0).r;
    if (slot == 0u) {
        return vec4<f32>(0.0);
    }
    let k = slot - 1u;
    let corner = vec3<f32>(f32(k % IRR_SLOTS), f32((k / IRR_SLOTS) % IRR_SLOTS), f32(k / (IRR_SLOTS * IRR_SLOTS))) * IRR_SIDE;
    let f = clamp((p - bf * IRR_BRICK) / IRR_STEP, vec3<f32>(0.0), vec3<f32>(IRR_SIDE - 1.0));
    return vec4<f32>(corner + 0.5 + f, 1.0);
}

fn local_ambient(n: vec3<f32>, p: vec3<f32>, sky: vec3<f32>) -> vec3<f32> {
    if (g.local.x <= 0.0) {
        return sky;
    }
    let at = local_uvw(p);
    if (at.w < 0.5) {
        return sky;
    }
    let size = vec3<f32>(textureDimensions(irr_atlas));
    let uvw = at.xyz / size;
    let q = vec3<f32>(0.0, 0.0, 0.25);
    let vis = textureSampleLevel(irr_atlas, samp, uvw, 0.0);
    let r = textureSampleLevel(irr_atlas, samp, uvw + q, 0.0);
    let gg = textureSampleLevel(irr_atlas, samp, uvw + 2.0 * q, 0.0);
    let bb = textureSampleLevel(irr_atlas, samp, uvw + 3.0 * q, 0.0);
    let d = vec4<f32>(1.0, n.y, n.z, n.x);
    let v = clamp(dot(vis, d), 0.0, 1.0);
    let bounce = max(vec3<f32>(dot(r, d), dot(gg, d), dot(bb, d)), vec3<f32>(0.0));
    let lin = pow(sky, vec3<f32>(GAMMA)) * mix(1.0, v, g.local.z) + bounce * g.local.y;
    let out = pow(lin, vec3<f32>(1.0 / GAMMA));
    // fading in after a background bake (render.rs LOCAL_FADE)
    if (g.local.x < 1.0) {
        return mix(sky, out, g.local.x);
    }
    return out;
}

// REMASTER local light (#192) [design choice]: the share of the sun that reaches a lit object
// at `p` past the world (tunnels, bridges), baked per probe towards the view's sun (probes.rs
// `Probe::sun`); 1 outside the bricks and with the option off. The Remaster sun shadow map
// casts only the cars.
fn local_sun(p: vec3<f32>) -> f32 {
    if (g.local.x <= 0.0) {
        return 1.0;
    }
    let at = local_uvw(p);
    if (at.w < 0.5) {
        return 1.0;
    }
    let s = textureSampleLevel(irr_sun, samp, at.xyz / vec3<f32>(textureDimensions(irr_sun)), 0.0).r;
    return mix(1.0, s, min(g.local.x, 1.0));
}

// REMASTER car shadows: how much of the sun reaches `p` (1 = all), 3x3 PCF on the comparison
// sampler's bilinear taps, fading out towards the edge of the map. The depth lookup needs no
// uniform control flow (Level variant).
// `bias`: the depth bias in map units.
fn shadow_vis(p: vec3<f32>, bias: f32) -> f32 {
    let lp = sh_u.vp * vec4<f32>(p, 1.0);
    let uv = vec2<f32>(lp.x * 0.5 + 0.5, 0.5 - lp.y * 0.5);
    let z = lp.z - bias;
    var sum = 0.0;
    if (sh_u.p.w > 0.0) {
        // overhead: 16 taps on a Vogel disk of the soft radius, turned by a position hash
        let h = fract(sin(dot(floor(p.xz * 256.0), vec2<f32>(12.9898, 78.233))) * 43758.5453);
        for (var k = 0; k < OVERHEAD_TAPS; k++) {
            let a = 2.39996323 * f32(k) + 6.2831853 * h;
            let r = sqrt((f32(k) + 0.5) / f32(OVERHEAD_TAPS)) * sh_u.p.w;
            sum += textureSampleCompareLevel(sh_map, sh_samp, uv + vec2<f32>(cos(a), sin(a)) * r, z);
        }
        sum = sum / f32(OVERHEAD_TAPS) * 9.0;
    } else {
        for (var y = -1; y <= 1; y++) {
            for (var x = -1; x <= 1; x++) {
                sum += textureSampleCompareLevel(sh_map, sh_samp, uv + vec2<f32>(f32(x), f32(y)) * sh_u.p.y, z);
            }
        }
    }
    let e = min(min(uv.x, 1.0 - uv.x), min(uv.y, 1.0 - uv.y));
    let inside = smoothstep(0.0, 0.06, e) * select(0.0, 1.0, lp.z > 0.0 && lp.z < 1.0);
    return mix(1.0, sum / 9.0, inside);
}

// REMASTER generated brake calipers (procgen.rs `CALIPER_ENV`) sit in the wheel mesh but must
// not spin: turned back by the wheel's spin angle (`spot_n.z` of the car uniform, 0 on bodies)
// before the wheel's model matrix spins them, so they keep their place.
fn unspin(p: vec3<f32>, env: f32) -> vec3<f32> {
    if (env < 4.5 || env > 5.5) {
        return p;
    }
    let s = sin(g.spot_n.z);
    let c = cos(g.spot_n.z);
    return vec3<f32>(p.x, c * p.y - s * p.z, s * p.y + c * p.z);
}

// The shadow map's caster pass: cars only, from the light.
@vertex
fn vs_shadow(v: VsIn) -> @builtin(position) vec4<f32> {
    return sh_u.vp * (g.model * vec4<f32>(unspin(v.pos, v.env), 1.0));
}

// REMASTER: `object_light` per pixel, with a smooth normalized Blinn-Phong highlight in place
// of the (N.H)^4 cut off at N.H = 0.5 (no Gouraud popping on low-poly cars). `vis` scales the
// sun (car shadows); `extra` is added before the limit (the MYW point lights).
fn object_light_pp(n: vec3<f32>, vd: vec3<f32>, vis: f32, extra: vec3<f32>, amb: vec3<f32>) -> vec3<f32> {
    let l = g.sun_dir.xyz;
    var k = max(dot(n, l), 0.0);
    if (k > 0.0) {
        let s = max(dot(n, normalize(l + vd)), 0.0);
        k = k + pow(s, PP_SHINE) * (PP_SHINE + 8.0) / 12.0 * g.sun.w;
    }
    return cap(max(amb + k * vis * g.sun.rgb + extra, vec3<f32>(0.0)));
}

// REMASTER option flags (env.w)
fn flag(bit: u32) -> bool {
    return (u32(g.env.w + 0.5) & bit) != 0u;
}

// REMASTER HDR: this draw goes to the linear float target (render.rs LINEAR_FLAG)
fn linear() -> bool {
    return flag(8u);
}

fn lin(c: vec3<f32>) -> vec3<f32> {
    return pow(max(c, vec3<f32>(0.0)), vec3<f32>(GAMMA));
}

// The output in the target's space: gamma (as computed) or linear.
fn out_rgb(c: vec3<f32>) -> vec3<f32> {
    return select(c, lin(c), linear());
}

// Colour limit: 1 like the original's 8-bit maths, none in HDR (highlights and lights over
// 1 feed the bloom).
fn cap(c: vec3<f32>) -> vec3<f32> {
    return select(min(c, vec3<f32>(1.0)), max(c, vec3<f32>(0.0)), linear());
}

// Sphere-map coordinates of the view-space reflection vector (the original approximates it
// as 0.5 + 0.2495 * (normal + screen position), 0x4402fd).
fn sphere_uv(n: vec3<f32>, world: vec3<f32>) -> vec2<f32> {
    let nv = normalize((g.view * vec4<f32>(n, 0.0)).xyz);
    let pv = normalize((g.view * vec4<f32>(world, 1.0)).xyz);
    let r = reflect(pv, nv);
    let m = 2.0 * sqrt(r.x * r.x + r.y * r.y + (r.z + 1.0) * (r.z + 1.0));
    return vec2<f32>(r.x / m + 0.5, 0.5 - r.y / m);
}

// Linear fog from CueStart to the view distance (0x439e84).
fn fog_at(world: vec3<f32>) -> f32 {
    let d = distance(world, g.cam_pos.xyz);
    return g.params.w * clamp((d - g.params.x) / (g.params.y - g.params.x), 0.0, 1.0);
}

// REMASTER fog (17-fog-rework.md) [design choice]: a distance ramp Start..Full to Opacity (shape
// t^curve), an exponential height haze rho0 * exp(-(y - ground) / H) integrated along the ray in
// closed form, and a guard that reaches 100 % at the clip plane. The in-scatter colour is the fog
// colour plus a Henyey-Greenstein sun glow. Mixed in linear light under HDR, in gamma otherwise.

// Henyey-Greenstein lobe, normalised to 1 towards the sun (g = anisotropy)
fn hg_phase(cos_t: f32, g_: f32) -> f32 {
    let k = max(1.0 + g_ * g_ - 2.0 * g_ * cos_t, 1e-4);
    let a = 1.0 - g_;
    return a * a * a / (k * sqrt(k));
}

// the in-scatter colour (linear light) of a ray towards `dir`
fn fog_colour(dir: vec3<f32>) -> vec3<f32> {
    return g.fog4.rgb + lin(g.sun.rgb) * (hg_phase(dot(dir, g.sun_dir.xyz), g.fog3.x) * g.fog2.w);
}

// the height haze's transmittance from the camera to `world` (closed form over the exponential profile)
fn haze_trans(world: vec3<f32>, d: f32) -> f32 {
    let k = g.fog2.z;
    let base = exp(clamp(-(g.cam_pos.y - g.fog2.y) * k, -20.0, 2.0));
    let a = clamp((world.y - g.cam_pos.y) * k, -8.0, 8.0);
    // (1 - exp(-a)) / a, finite at a = 0
    let ratio = select((1.0 - exp(-a)) / a, 1.0 - 0.5 * a, abs(a) < 1e-3);
    return exp(-g.fog2.x * base * d * ratio);
}

// how much fog lies between the camera and `world` (0..1)
fn fog_amount(world: vec3<f32>) -> f32 {
    let d = distance(world, g.cam_pos.xyz);
    let t = clamp((d - g.fog5.x) / max(g.fog5.y - g.fog5.x, 1e-3), 0.0, 1.0);
    let dist = g.fog5.z * pow(t, g.fog5.w);
    // water_t.w is the view's clip plane: the fog is complete there whatever Full says
    let clip = g.water_t.w;
    let guard = smoothstep(0.9 * clip, clip, d);
    return 1.0 - (1.0 - dist) * haze_trans(world, d) * (1.0 - guard);
}

// `c` (gamma) fogged by the amount `f` towards `dir`: the result is gamma too (callers go through
// out_rgb), the mix itself is linear under HDR
fn fog_blend(c: vec3<f32>, dir: vec3<f32>, f: f32) -> vec3<f32> {
    let col = fog_colour(dir);
    if (linear()) {
        return pow(mix(lin(c), col, f), vec3<f32>(1.0 / GAMMA));
    }
    return mix(c, min(pow(col, vec3<f32>(1.0 / GAMMA)), vec3<f32>(1.0)), f);
}

// fog on the sky by direction: the distance ramp at infinity (scaled down above the horizon by the
// sky haze), the height haze integrated to infinity, and everything below the horizon fogged (the
// void past the world's edge)
fn fog_sky_amount(dir: vec3<f32>) -> f32 {
    let w = 1.0 - smoothstep(0.0, max(g.fog4.w, 1e-3) * 0.5, dir.y);
    let k = g.fog2.z;
    let rho = g.fog2.x * exp(clamp(-(g.cam_pos.y - g.fog2.y) * k, -20.0, 2.0));
    let haze = 1.0 - exp(-min(rho / (k * max(dir.y, 0.02)), 20.0));
    let below = 1.0 - smoothstep(-0.02, 0.0, dir.y);
    return max(1.0 - (1.0 - g.fog5.z * w) * (1.0 - haze), below);
}

// REMASTER: a sky colour (gamma) fogged towards `dir`; Classic views and volumetric views (the
// volume pass fogs the sky from its depth) leave it alone
fn fog_sky(c: vec3<f32>, dir: vec3<f32>) -> vec3<f32> {
    if (flag(2048u)) {
        return c;
    }
    return fog_sky_dir(c, dir);
}

// `fog_sky` without the volumetric exception: the water's mirrored sky, where the volume covers only
// the way from the camera to the surface (water.wgsl)
fn fog_sky_dir(c: vec3<f32>, dir: vec3<f32>) -> vec3<f32> {
    if (g.params.w < 0.5 || !flag(1024u)) {
        return c;
    }
    return fog_blend(c, dir, fog_sky_amount(dir));
}

// the distance ramp and the clip guard without the haze (volfog.wgsl `fog_dist`: the volume holds the haze)
fn fog_ramp(d: f32) -> f32 {
    let t = clamp((d - g.fog5.x) / max(g.fog5.y - g.fog5.x, 1e-3), 0.0, 1.0);
    let clip = g.water_t.w;
    let guard = smoothstep(0.9 * clip, clip, d);
    return 1.0 - (1.0 - g.fog5.z * pow(t, g.fog5.w)) * (1.0 - guard);
}

// the world direction of a sky pixel (screen 0..1)
fn sky_dir(screen: vec2<f32>) -> vec3<f32> {
    let p = g.sky_inv * vec4<f32>(screen.x * 2.0 - 1.0, 1.0 - screen.y * 2.0, 1.0, 1.0);
    return normalize(p.xyz / p.w - g.cam_pos.xyz);
}

fn fog_remaster(c: vec3<f32>, world: vec3<f32>) -> vec3<f32> {
    if (g.params.w < 0.5) {
        return c;
    }
    return fog_blend(c, normalize(world - g.cam_pos.xyz), fog_amount(world));
}

// Fog over a colour: Classic (a given amount, mixed in gamma space), Remaster, or none (Volumetric).
fn fog_mix(c: vec3<f32>, world: vec3<f32>, amount: f32) -> vec3<f32> {
    // Volumetric (2048): the volume pass fogs the opaque scene from its depth (render.rs `VolFog`)
    if (flag(2048u)) {
        return c;
    }
    return select(mix(c, g.fog.rgb, amount), fog_remaster(c, world), flag(1024u));
}

// CLASSIC: the original's per-vertex light and fog.
@vertex
fn vs_main(v: VsIn, @builtin(vertex_index) vi: u32) -> VsOut {
    var o: VsOut;
    if (g.myw_n.y > 1.5) {
        // scene objects: the lights of the object's own cell (`prop_myw`)
        prop_rec = i32(prop_of[vi]);
    }
    let world = g.model * vec4<f32>(v.pos, 1.0);
    o.clip = g.view_proj * world;
    o.uv = v.uv;
    var c = v.color.rgb;
    o.env_uv = vec2<f32>(0.0);
    o.env = 0.0;
    o.wpos = world.xyz;
    o.wnormal = (g.model * vec4<f32>(v.normal, 0.0)).xyz;
    o.albedo = select(vec3<f32>(1.0), v.color.rgb * g.gain.rgb, g.ambient.w > 0.5);
    if (g.ambient.w > 0.5) {
        let n = normalize((g.model * vec4<f32>(v.normal, 0.0)).xyz);
        if (flag(8192u)) {
            // Flat car shading (0x4ebfd8): one even light per object, no sun and no normals
            var o = select(world.xyz, g.model[3].xyz, g.myw_n.y > 0.5);
            if (prop_rec >= 0) {
                o = prop_myw[u32(prop_rec) * MYW_REC].xyz;
            }
            c = c * flat_light(o);
        } else {
            c = c * object_light(n, world.xyz);
        }
        if (v.env > 8.5) {
            // env-mapped scene object (scene.rs `PROP_ENV`): even light, the env texture
            c = v.color.rgb * prop_env_light(world.xyz);
            o.env_uv = prop_env_uv(n, o.clip);
            o.env = 9.0;
        } else if (v.env > 0.5 && v.env < 7.5 && g.env.x > 0.5) {
            // sphere map of the view-space reflection vector (the original approximates it
            // as 0.5 + 0.2495 * (normal + screen position), 0x4402fd)
            let nv = normalize((g.view * vec4<f32>(n, 0.0)).xyz);
            let pv = normalize((g.view * world).xyz);
            let r = reflect(pv, nv);
            let m = 2.0 * sqrt(r.x * r.x + r.y * r.y + (r.z + 1.0) * (r.z + 1.0));
            o.env_uv = vec2<f32>(r.x / m + 0.5, 0.5 - r.y / m);
            o.env = 1.0;
        }
    }
    o.color = vec4<f32>(min(c * g.gain.rgb, vec3<f32>(1.0)), v.color.a);
    o.layer = v.layer;
    o.fog = fog_at(world.xyz);
    // CHEAT tribute to tron: every draw is a non-indexed triangle list, so the vertex index gives the corner
    let corner = vi % 3u;
    o.bary = vec3<f32>(f32(corner == 0u), f32(corner == 1u), f32(corner == 2u));
    return o;
}

// DEBUG (motorview wire styles, wire.rs): flat colours, no light, no fog, no debug view. Fill
// triangles (a normal in the vertex) get a head-light shade so panels read; edges carry no
// normal and keep their colour.
@vertex
fn vs_flat(v: VsIn) -> VsOut {
    var o: VsOut;
    let world = g.model * vec4<f32>(v.pos, 1.0);
    o.clip = g.view_proj * world;
    o.uv = vec2<f32>(0.0);
    o.env_uv = vec2<f32>(0.0);
    o.env = 0.0;
    o.wpos = world.xyz;
    o.wnormal = vec3<f32>(0.0);
    o.albedo = vec3<f32>(1.0);
    var shade = 1.0;
    if (dot(v.normal, v.normal) > 1e-8) {
        let n = normalize((g.model * vec4<f32>(v.normal, 0.0)).xyz);
        let vd = normalize(g.cam_pos.xyz - world.xyz);
        shade = 0.55 + 0.45 * abs(dot(n, vd));
    }
    o.color = vec4<f32>(v.color.rgb * shade, 1.0);
    o.layer = -1.0;
    o.fog = 0.0;
    return o;
}

@fragment
fn fs_flat(i: VsOut, @builtin(front_facing) front: bool) -> @location(0) vec4<f32> {
    // the back of a mesh (a face seen from inside) tints red: face orientation
    let c = select(mix(i.color.rgb, vec3<f32>(0.55, 0.12, 0.1), 0.65), i.color.rgb, front);
    return vec4<f32>(out_rgb(c), 1.0);
}

// REMASTER front-end hologram (`Frame::hologram`): additive cyan. The edges keep the vertex
// alpha as their strength; the shell glows towards the silhouette (fresnel).
const HOLO: vec3<f32> = vec3<f32>(0.25, 0.7, 1.0);

// the hologram clip (`HoloClip`: sky_geom = circle + chord or rectangle, myw_n.z mode, .w gain): soft 2 px edge.
// The coverage of the clip region (the circle / rectangle and the listed glass cards) at window pixel p.
fn holo_mask(p: vec2<f32>) -> f32 {
    var e: f32;
    if (g.sky_bottom.w == 1.0) {
        // rounded rectangle: sky_geom = centre, half extents; sky_top.w = corner radius
        let r = g.sky_top.w;
        let q = abs(p - g.sky_geom.xy) - (g.sky_geom.zw - vec2<f32>(r));
        e = r - length(max(q, vec2<f32>(0.0))) - min(max(q.x, q.y), 0.0);
    } else {
        e = min(g.sky_geom.z - distance(p, g.sky_geom.xy), g.sky_geom.w - p.y);
    }
    // the listed rounded rectangles (glass cards): the union with the shape above
    let rr = g.holo_n.x;
    for (var i = 0u; i < u32(g.holo_n.y); i++) {
        let rc = g.holo_rects[i];
        let q = abs(p - rc.xy) - (rc.zw - vec2<f32>(rr));
        e = max(e, rr - length(max(q, vec2<f32>(0.0))) - min(max(q.x, q.y), 0.0));
    }
    return clamp(e * 0.5 + 0.5, 0.0, 1.0);
}

// mode 1 keeps the inside, 2 the outside; 0 (no clip) and 3 (frosted: masked later, holofrost.wgsl) draw all
fn holo_gain(p: vec2<f32>) -> f32 {
    let mode = g.myw_n.z;
    if (mode == 0.0 || mode == 3.0) {
        return 1.0;
    }
    let s = holo_mask(p);
    return g.myw_n.w * select(1.0 - s, s, mode == 1.0);
}

@fragment
fn fs_holo(i: VsOut) -> @location(0) vec4<f32> {
    // spot_n.w = the road's fade distance (0: none): full to half of it, gone at it
    let far = g.spot_n.w;
    let fade = select(1.0, 1.0 - smoothstep(0.5 * far, far, distance(i.wpos, g.cam_pos.xyz)), far > 0.0);
    return vec4<f32>(HOLO * i.color.a * fade * holo_gain(i.clip.xy), 1.0);
}

@fragment
fn fs_holo_fill(i: VsOut) -> @location(0) vec4<f32> {
    let n = normalize(i.wnormal);
    let v = normalize(g.cam_pos.xyz - i.wpos);
    let rim = pow(1.0 - abs(dot(n, v)), 2.5);
    return vec4<f32>(HOLO * (0.015 + 0.16 * rim) * holo_gain(i.clip.xy), 1.0);
}

// DEBUG (motorview, render.rs `DebugShade` in spot_n.y): is this a mesh pixel to replace?
// Lines, projected shadows and skid marks have no normal and keep their colour.
fn debug_on(i: VsOut) -> bool {
    return g.spot_n.y > 0.5 && dot(i.wnormal, i.wnormal) > 1e-6;
}

// DEBUG: what a mesh pixel shows instead of its colour. `t` = the texel, `light` = the colour
// before the texture and the fog.
fn debug_view(i: VsOut, t: vec3<f32>, light: vec3<f32>) -> vec3<f32> {
    let textured = i.layer >= 0.0;
    switch u32(g.spot_n.y + 0.5) {
        case 1u: {
            // normal in game axes (view z is game -z)
            return normalize(i.wnormal) * vec3<f32>(0.5, 0.5, -0.5) + 0.5;
        }
        case 2u: {
            if (!textured) {
                return vec3<f32>(0.2);
            }
            let cell = vec2<i32>(floor(i.uv * 8.0));
            let k = select(0.45, 1.0, ((cell.x + cell.y) & 1) == 0);
            return k * vec3<f32>(fract(i.uv.x), fract(i.uv.y), 0.6);
        }
        case 3u: {
            return select(vec3<f32>(0.5), t, textured);
        }
        case 4u: {
            return light;
        }
        default: {
            let d = distance(i.wpos, g.cam_pos.xyz);
            return vec3<f32>(1.0 - clamp(log2(1.0 + d) / log2(2001.0), 0.0, 1.0));
        }
    }
}

// CHEAT tribute to tron (#320, `0x48e964`, 08 section 8.14): render.rs `TRON_FLAG`
const TRON_FLAG: u32 = 16384u;
// the fill's blue = (R + G + B) x 0.33 of the lit colour (`0x4d8f26`), colour-keyed textures x 0.25 more (`0x4d8f66`)
const TRON_FILL: f32 = 0.33;
const TRON_KEYED: f32 = 0.25;
// half the outline in pixels: the two triangles of a shared edge draw a half each, about the 1 px of `grDrawLine`
const TRON_HALF_PX: f32 = 0.5;

// The Glide Tron look of one pixel of a coded MYO triangle (scene.rs `TRON_CODE` in the colour's alpha: bits 0..2 = the
// edge opposite corner k is outlined, 8 = black fill, 16 = colour-keyed). `w` = fwidth(bary), taken in uniform control
// flow. No texture: pure blue by the lit colour, the polygon's sides in (0, 0, 1) over it, both fogged to black.
fn tron_code(i: VsOut) -> u32 {
    return u32(max(round(i.color.a - 2.0), 0.0));
}

fn tron_shade(i: VsOut, w: vec3<f32>) -> vec3<f32> {
    let code = tron_code(i);
    var b = min((i.color.r + i.color.g + i.color.b) * TRON_FILL, 1.0);
    if ((code & 16u) != 0u) {
        b = b * TRON_KEYED;
    }
    if ((code & 8u) != 0u) {
        b = 0.0;
    }
    // pixels to each edge: bary[k] is 0 on the edge opposite corner k
    let px = i.bary / max(w, vec3<f32>(1e-6));
    var line = false;
    for (var k = 0u; k < 3u; k++) {
        line = line || ((code & (1u << k)) != 0u && px[k] < TRON_HALF_PX);
    }
    let c = select(vec3<f32>(0.0, 0.0, b), vec3<f32>(0.0, 0.0, 1.0), line);
    return mix(c, vec3<f32>(0.0), i.fog);
}

// REMASTER Neon grid (#320 phase 3, design choice; 09-remaster "Tron: Neon grid"): the Tron Legacy take on the Glide
// look. The same codes as `tron_shade`; near-black navy surfaces by their light with a faint Fresnel mirror of the
// grid (this frame's env cube), polygon sides anti-aliased at `g.tron.x` px each side and emissive above the bloom
// threshold, a soft glow inside them, faded where a triangle is only a few pixels across, all fogged to black.
const NEON_FILL: vec3<f32> = vec3<f32>(0.03, 0.07, 0.14);
// gamma space: 1.5^2.2 is about 2.4 in linear light, over BLOOM_THRESHOLD and under BLOOM_MAX
const NEON_EDGE: vec3<f32> = vec3<f32>(0.45, 1.25, 1.5);
// the glow inside a side: its strength and its reach in edge widths
const NEON_HALO: f32 = 0.18;
const NEON_HALO_REACH: f32 = 4.0;
// triangles thinner than this many pixels lose their sides (from the first to the second)
const NEON_THIN: vec2<f32> = vec2<f32>(1.5, 6.0);
const NEON_GLOSS: f32 = 0.5;
const NEON_F0: f32 = 0.04;

fn neon_shade(i: VsOut, c: vec3<f32>, w: vec3<f32>, n: vec3<f32>, vd: vec3<f32>) -> vec3<f32> {
    let code = tron_code(i);
    var b = min((c.r + c.g + c.b) * TRON_FILL, 1.0);
    if ((code & 16u) != 0u) {
        b = b * TRON_KEYED;
    }
    if ((code & 8u) != 0u) {
        b = 0.0;
    }
    var col = NEON_FILL * b;
    // the gloss: the grid in this frame's env cube, at grazing angles mostly
    if (g.env.x > 1.5 && dot(i.wnormal, i.wnormal) > 1e-6) {
        let fr = NEON_F0 + (1.0 - NEON_F0) * pow(1.0 - clamp(dot(n, vd), 0.0, 1.0), 5.0);
        col += env_gamma(textureSampleLevel(env_cube, samp, reflect(-vd, n), 0.0).rgb) * fr * NEON_GLOSS;
    }
    // pixels to each side (bary[k] is 0 on the side opposite corner k), the nearest coded one
    let px = i.bary / max(w, vec3<f32>(1e-6));
    var d = 1e9;
    for (var k = 0u; k < 3u; k++) {
        if ((code & (1u << k)) != 0u) {
            d = min(d, px[k]);
        }
    }
    let hw = max(g.tron.x, 0.25);
    let line = 1.0 - smoothstep(hw - 0.5, hw + 0.5, d);
    let halo = NEON_HALO * exp(-d / (hw * NEON_HALO_REACH));
    // the triangle's smallest height in pixels: 1 / the steepest corner weight
    let thin = smoothstep(NEON_THIN.x, NEON_THIN.y, 1.0 / max(max(w.x, w.y), max(w.z, 1e-6)));
    col += NEON_EDGE * max(line, halo) * thin;
    return cap(col) * (1.0 - fog_at(i.wpos));
}

// CLASSIC: texture * colour (MODULATE), colour-keyed texels discarded, vertex fog.
@fragment
fn fs_main(i: VsOut) -> @location(0) vec4<f32> {
    // sample in uniform control flow, then decide
    let t = textureSample(tex, samp, i.uv, max(i32(i.layer + 0.5), 0));
    let bary_w = fwidth(i.bary);
    let textured = i.layer >= 0.0;
    if (textured && t.a < 0.5) {
        discard;
    }
    if (flag(TRON_FLAG)) {
        return vec4<f32>(tron_shade(i, bary_w), 1.0);
    }
    if (debug_on(i)) {
        return vec4<f32>(debug_view(i, t.rgb, i.color.rgb), i.color.a);
    }
    let e = textureSample(tex, samp, i.env_uv, max(i32(g.env.y + 0.5), 0));
    var c = i.color.rgb;
    if (textured) {
        c = c * t.rgb;
    }
    if (i.env > 8.5) {
        // env-mapped scene object: MODULATE with the env texel
        c = min(c * e.rgb, vec3<f32>(1.0));
    } else if (i.env > 0.5) {
        // paint keeps 0.4 of its lit colour (0x43fcc9) and the env layer adds the rest,
        // tinted by the paint [I: blend of the second pass]
        c = min(c * (0.4 + 1.1 * e.rgb), vec3<f32>(1.0));
    }
    return vec4<f32>(mix(c, g.fog.rgb, i.fog), i.color.a);
}

// REMASTER paint finish of a paint vertex tag (scene.rs `METALLIC_ENV`, `PEARL_ENV`): 0 gloss (1), 2 pearl (1.75),
// 1 metallic (2); the same numbers `live_paint` returns
fn tag_finish(env: f32) -> f32 {
    return select(select(0.0, 2.0, env > 1.5), 1.0, env > 1.9);
}

// REMASTER menu car preview (#91, 09-remaster 9.39): a body paint colour (env 1, 1.75 or 2) that is
// a stock Colour1..3 takes its new colour; the three tests run in order on the updated colour, as
// CustomCarColours does (`scene::repaint`, 0x47b9c4). The tolerance covers the generated bodies'
// interpolated colours. Off (w 0) for every other draw. w of the result: -1 no zone matched, else the finish of
// the last zone that matched (#136, #351: its row's w is 1 gloss, 2 metallic, 3 pearl; returned as 0, 1, 2).
fn live_paint(c: vec3<f32>, env: f32) -> vec4<f32> {
    if (g.paint[0].w < 0.5 || env < 0.5 || env > 2.5) {
        return vec4<f32>(c, -1.0);
    }
    var o = c;
    var gain = 0.0;
    var finish = -1.0;
    for (var k = 0u; k < 3u; k++) {
        if (all(abs(o - g.paint[k].rgb) < vec3<f32>(1e-3))) {
            o = g.paint[k + 3u].rgb;
            gain = g.paint[k + 3u].w;
            finish = clamp(g.paint[k].w - 1.0, 0.0, 2.0);
        }
    }
    return vec4<f32>(o * (1.0 + gain), finish);
}

// REMASTER: the vertex stage only places things; lit objects keep their unlit colour and
// marks for fs_remaster (1 = Reflection, 4.75 / 5 / 5.25 = gloss / metallic / pearl finish), which does its own fog (the
// vertex fog is for the particles).
@vertex
fn vs_remaster(v: VsIn, @builtin(vertex_index) vi: u32) -> VsOut {
    var o: VsOut;
    // CHEAT tribute to tron (#320): the corner of the non-indexed triangle, as vs_main (the Neon grid's edges)
    let corner = vi % 3u;
    o.bary = vec3<f32>(f32(corner == 0u), f32(corner == 1u), f32(corner == 2u));
    let world = g.model * vec4<f32>(unspin(v.pos, v.env), 1.0);
    o.clip = g.view_proj * world;
    o.uv = v.uv;
    o.layer = v.layer;
    o.env_uv = vec2<f32>(0.0);
    o.env = 0.0;
    o.fog = fog_at(world.xyz);
    o.wpos = world.xyz;
    o.wnormal = (g.model * vec4<f32>(unspin(v.normal, v.env), 0.0)).xyz;
    let live = live_paint(v.color.rgb, v.env);
    let col = live.rgb;
    o.albedo = select(vec3<f32>(1.0), col * g.gain.rgb, g.ambient.w > 0.5);
    o.color = vec4<f32>(min(col * g.gain.rgb, vec3<f32>(1.0)), v.color.a);
    if (g.ambient.w > 0.5 && v.env > 0.5 && v.env < 2.5) {
        // gloss: 0 Flat / Gouraud, 1 Reflection (the Classic blend), 4.75 Enhanced paint (paint.wgsl's gloss finish)
        o.env = select(select(0.0, 1.0, g.env.x > 0.5), 4.75, g.env.x > 1.5);
        // REMASTER metallic and pearl paint (#136, #351): tagged `scene::METALLIC_ENV` (2) / `PEARL_ENV` (1.75) at
        // load, or a live-painted zone of that finish; fragment code 5 / 5.25 (free there: the calipers' vertex 5
        // is plain paint, 0), lit per pixel whatever the car shading, as polished metal
        let finish = select(tag_finish(v.env), live.w, live.w > -0.5);
        if (finish > 0.5) {
            o.env = select(5.0, 5.25, finish > 1.5);
            o.env_uv = sphere_uv(normalize(o.wnormal), world.xyz);
        }
    }
    if (g.ambient.w > 0.5 && v.env > 2.5 && v.env < 3.5) {
        // polished metal, lit per pixel whatever the options (3)
        o.env = 3.0;
        o.env_uv = sphere_uv(normalize(o.wnormal), world.xyz);
    }
    if (g.ambient.w > 0.5 && v.env > 3.5 && v.env < 4.5) {
        // self-lit lamp parts (generated lenses and LED bars, procgen.rs `LAMP_ENV`): their
        // own colour, unlit (4)
        o.env = 4.0;
    }
    // generated calipers (5) are plain lit paint; carbon fibre (6) is lit per pixel whatever
    // the options
    if (g.ambient.w > 0.5 && v.env > 5.5 && v.env < 7.5) {
        o.env = select(6.0, 7.0, v.env > 6.5);
    }
    // retro-reflective road sign faces (scene.rs `RETRO_ENV`), props and world alike (8)
    if (v.env > 7.5 && v.env < 8.5) {
        o.env = 8.0;
    }
    if (g.ambient.w > 0.5 && (o.env < 1.5 || o.env > 7.5) && !flag(1u)) {
        // per-pixel light off: the Classic vertex light and sphere map
        let n = normalize(o.wnormal);
        o.color = vec4<f32>(min(v.color.rgb * object_light_amb(n, world.xyz, local_ambient(n, world.xyz, sky_ambient(n)), local_sun(world.xyz)) * g.gain.rgb, vec3<f32>(1.0)), v.color.a);
        o.env_uv = sphere_uv(n, world.xyz);
    }
    // env-mapped scene objects (scene.rs `PROP_ENV`, 9): the Classic light and texture coordinates
    if (g.ambient.w > 0.5 && v.env > 8.5) {
        let n = normalize(o.wnormal);
        o.env = 9.0;
        o.env_uv = prop_env_uv(n, o.clip);
        o.color = vec4<f32>(min(v.color.rgb * prop_env_light(world.xyz) * g.gain.rgb, vec3<f32>(1.0)), v.color.a);
    }
    return o;
}

// REMASTER DEBUG view 6: lamp shadow layers, a colour per layer, dark where a car shadows the
// point (fs_remaster only: Classic layouts have no group 2)
fn lamp_shadow_debug(i: VsOut) -> vec3<f32> {
    var c = vec3<f32>(1.0);
    let count = u32(g.lamp_n.x + 0.5);
    let n = normalize(i.wnormal);
    for (var k = 0u; k < count; k++) {
        let layer = g.lamps[2u * k + 1u].w;
        if (layer >= 0.0 && flag(512u)) {
            let l = u32(layer + 0.5);
            let tint = vec3<f32>(f32(l % 2u), f32((l / 2u) % 2u), 1.0 - f32(l % 2u));
            c = min(c, mix(tint, vec3<f32>(1.0), spot_vis(l, i.wpos, n)));
        }
    }
    // the headlight layers
    let spots = u32(g.spot_n.x + 0.5);
    for (var k = 0u; k < spots; k++) {
        let layer = g.spots[4u * k + 3u].w;
        if (layer >= 0.0 && flag(512u)) {
            let l = u32(layer + 0.5);
            let tint = vec3<f32>(f32(l % 2u), 1.0, 0.0);
            c = min(c, mix(tint, vec3<f32>(1.0), spot_vis(l, i.wpos, n)));
        }
    }
    return c;
}

// REMASTER: anisotropic sampling, alpha to coverage (MSAA), per-pixel object light, fog and
// sphere map, clear coat on the Reflection layer, Enhanced paint.
// REMASTER: this frame's env cube towards `r`, softened by 5 taps `spread` apart (the cube has no mips);
// linear light, as sampled
fn cube_soft(r: vec3<f32>, spread: f32) -> vec3<f32> {
    let t1 = normalize(cross(r, select(vec3<f32>(0.0, 1.0, 0.0), vec3<f32>(1.0, 0.0, 0.0), abs(r.y) > 0.9))) * spread;
    let t2 = cross(normalize(r), t1);
    var e0 = textureSampleLevel(env_cube, samp, r, 0.0).rgb;
    e0 += textureSampleLevel(env_cube, samp, r + t1, 0.0).rgb + textureSampleLevel(env_cube, samp, r - t1, 0.0).rgb;
    e0 += textureSampleLevel(env_cube, samp, r + t2, 0.0).rgb + textureSampleLevel(env_cube, samp, r - t2, 0.0).rgb;
    return e0 / 5.0;
}

// REMASTER paint flakes' glint towards the half vector `h` at car-space point `lp`, in cells of `size`: the normal
// tilted by up to `tilt` per cell, raised to `shine`
fn flake_glint(lp: vec3<f32>, size: f32, n: vec3<f32>, h: vec3<f32>, tilt: f32, shine: f32) -> f32 {
    let nf = normalize(n + (flake_hash(vec3<i32>(floor(lp / size))) * 2.0 - 1.0) * tilt);
    return pow(max(dot(nf, h), 0.0), shine);
}

// REMASTER a world point in the car's own space (the rigid `g.model` undone), so paint flakes ride with the car
fn car_space(p: vec3<f32>) -> vec3<f32> {
    let d = p - g.model[3].xyz;
    return vec3<f32>(dot(g.model[0].xyz, d) / dot(g.model[0].xyz, g.model[0].xyz), dot(g.model[1].xyz, d) / dot(g.model[1].xyz, g.model[1].xyz), dot(g.model[2].xyz, d) / dot(g.model[2].xyz, g.model[2].xyz));
}

// REMASTER the flakes' brightness in a pixel (0.. about 1): the cells start at `cell` and double in size so one spans
// PAINT_FLAKE_PX pixels or more (never finer: no shimmer), blended between two sizes, and dim with each doubling
fn paint_flake(lp: vec3<f32>, n: vec3<f32>, h: vec3<f32>, dp_x: vec3<f32>, dp_y: vec3<f32>, cell: f32, tilt: f32, shine: f32) -> f32 {
    let lod = max(log2(max(length(dp_x), length(dp_y)) * PAINT_FLAKE_PX / cell), 0.0);
    let l0 = floor(lod);
    let size = cell * exp2(l0);
    return mix(flake_glint(lp, size, n, h, tilt, shine), flake_glint(lp, 2.0 * size, n, h, tilt, shine), lod - l0) / (1.0 + lod);
}

// REMASTER paint flakes: three uniform numbers per lattice cell (PCG 3D)
fn flake_hash(c: vec3<i32>) -> vec3<f32> {
    var v = bitcast<vec3<u32>>(c) * 1664525u + vec3<u32>(1013904223u);
    v.x += v.y * v.z;
    v.y += v.z * v.x;
    v.z += v.x * v.y;
    v = v ^ (v >> vec3<u32>(16u));
    v.x += v.y * v.z;
    v.y += v.z * v.x;
    v.z += v.x * v.y;
    return vec3<f32>(v) / 4294967295.0;
}

@fragment
fn fs_remaster(i: VsOut) -> @location(0) vec4<f32> {
    let lit = g.ambient.w > 0.5 && dot(i.wnormal, i.wnormal) > 1e-6;
    let vd = normalize(g.cam_pos.xyz - i.wpos);
    let n0 = i.wnormal * inverseSqrt(max(dot(i.wnormal, i.wnormal), 1e-12));
    // interpolated normals can turn away from the eye at silhouettes
    let n = normalize(n0 + vd * max(0.0, 0.05 - dot(n0, vd)));
    let pixel_light = flag(1u);
    // REMASTER sky ambient: the sky's irradiance towards the normal for lit objects
    var amb = g.ambient.rgb;
    if (lit) {
        amb = local_ambient(n, i.wpos, sky_ambient(n));
    }
    // the Reflection sphere map, per pixel or from the vertices
    let env_uv = select(i.env_uv, sphere_uv(n, i.wpos), pixel_light && i.env < 8.5);
    // sample in uniform control flow, then decide
    // (the tangent frame of the normal maps, from the screen derivatives)
    let duv_x = dpdx(i.uv);
    let duv_y = dpdy(i.uv);
    let dp_x = dpdx(i.wpos);
    let dp_y = dpdy(i.wpos);
    var t = textureSample(tex, samp_hq, i.uv, max(i32(i.layer + 0.5), 0));
    let e = textureSample(tex, samp_hq, env_uv, max(i32(g.env.y + 0.5), 0));
    // CHEAT tribute to tron: the Neon grid's pixel scale of the corner weights
    let bary_w = fwidth(i.bary);
    // REMASTER texture pack: the user's colour replaces the texel (its alpha is the cut-out),
    // the normal + roughness map bends the shading normal [design choice]
    var pr = vec4<f32>(0.5, 0.5, 1.0, 1.0);
    var nn = n;
    var packed_n = false;
    if (flag(4096u)) {
        let pack = pack_ids(i.layer);
        if (pack.x >= 0) {
            t = textureSampleGrad(tex_hi, samp_hq, i.uv, pack.x, duv_x, duv_y);
        }
        if (pack.y >= 0 && dot(i.wnormal, i.wnormal) > 1e-6) {
            pr = textureSampleGrad(tex_hi, samp_hq, i.uv, pack.y, duv_x, duv_y);
            nn = map_normal(pr, n, dp_x, dp_y, duv_x, duv_y);
            packed_n = true;
        }
    }
    // colour key: sharpened to about one pixel of coverage ramp
    let key = clamp((t.a - 0.5) / max(fwidth(t.a), 1e-4) + 0.5, 0.0, 1.0);
    let textured = i.layer >= 0.0;
    if (textured && (select(t.a < 0.5, key <= 0.0, ALPHA_TO_COVERAGE))) {
        discard;
    }
    var c = i.color.rgb;
    // REMASTER car shadows: lit objects and the world look up the sun's depth map (flag 128)
    var vis = 1.0;
    if (flag(128u)) {
        if (lit) {
            // the map's texel in metres: `shadow_matrix` sets depth = 2 r + 16, p.y = 1 / size
            let texel = (sh_u.p.x - 16.0) * sh_u.p.y;
            let ndl = clamp(dot(n0, g.sun_dir.xyz), 0.0, 1.0);
            let lift = max(texel * SHADOW_NORMAL_TEXELS * (1.0 - ndl), SHADOW_LIFT_MIN);
            let bias = max(texel * SHADOW_DEPTH_TEXELS, SHADOW_LIFT_MIN) / sh_u.p.x;
            vis = shadow_vis(i.wpos + n0 * lift, bias);
        } else {
            vis = shadow_vis(i.wpos, sh_u.p.z);
        }
        if (lit && sh_u.p.w > 0.0) {
            vis = 1.0;
        }
    }
    // REMASTER local light: the world's shade on lit objects (#192)
    if (lit) {
        vis = vis * local_sun(i.wpos);
    }
    // REMASTER: the MYW point lights per pixel
    var ml = vec3<f32>(0.0);
    if (lit) {
        ml = myw_light_pp(nn, i.wpos, vd);
    }
    let sign = i.env > 7.5 && i.env < 8.5;
    if (lit && (i.env < 1.5 || sign) && pixel_light) {
        c = cap(i.albedo * object_light_pp(nn, vd, vis, ml, amb));
    }
    if (!lit && flag(128u)) {
        // baked world: darken by the sun's strength (before the dynamic lights are added:
        // the sun's shadow does not dim a headlight pool)
        let luma = vec3<f32>(0.3, 0.59, 0.11);
        let sun = dot(g.sun.rgb, luma);
        let share = sun / (sun + dot(g.ambient.rgb, luma) + 1e-3);
        let dark = select(min(share * SHADOW_SHARE_GAIN, SHADOW_WORLD), OVERHEAD_WORLD, sh_u.p.w > 0.0);
        c = c * (1.0 - (1.0 - vis) * dark);
    }
    if (!lit && flag(512u) && g.lamp_n.x > 0.5 && dot(i.wnormal, i.wnormal) > 1e-6) {
        c = c * lamp_world_shadow(i.wpos, n0);
    }
    // REMASTER texture pack: the baked world light stays; the normal map moves it by the ratio of
    // the sun on the mapped normal to the sun on the face (ambient kept), within a clamp
    if (packed_n && !lit) {
        let luma = vec3<f32>(0.3, 0.59, 0.11);
        let sl = dot(g.sun.rgb, luma) * vis;
        let al = dot(g.ambient.rgb, luma) + 1e-3;
        let ratio = (al + sl * max(dot(nn, g.sun_dir.xyz), 0.0)) / (al + sl * max(dot(n0, g.sun_dir.xyz), 0.0));
        c = c * clamp(ratio, PACK_RELIGHT_MIN, PACK_RELIGHT_MAX);
    }
    var dl = vec3<f32>(0.0);
    // shadows and skid marks have no normal and stay as they are
    if (g.env.z + g.spot_n.x + g.lamp_n.x > 0.5 && dot(i.wnormal, i.wnormal) > 1e-6) {
        let nd = select(n0, nn, packed_n);
        dl = dynamic_light(i.wpos, nd);
        if (lit) {
            dl += lamp_light(i.wpos, nd);
        }
        if (i.env < 1.5 || sign) {
            c = cap(c + dl * i.albedo);
        }
    }
    // the paint models below add the point lights to their dynamic light
    dl += ml;
    // CHEAT tribute to tron (#320): the Neon grid from the lit, untextured colour
    if (flag(TRON_FLAG)) {
        return vec4<f32>(out_rgb(neon_shade(i, c, bary_w, n, vd)), select(1.0, key, ALPHA_TO_COVERAGE && textured));
    }
    if (debug_on(i)) {
        var dbg = debug_view(i, t.rgb, c);
        if (u32(g.spot_n.y + 0.5) == 6u) {
            dbg = lamp_shadow_debug(i);
        }
        return vec4<f32>(out_rgb(dbg), select(1.0, key, ALPHA_TO_COVERAGE && textured));
    }
    if (textured) {
        c = c * t.rgb;
    }
    let ndv = clamp(dot(n, vd), 0.0, 1.0);
    // REMASTER reflective signs (09-remaster 9.14): retro-reflection of the headlights plus a
    // faint sheen of the surroundings, added after the surface limit so it reaches the bloom
    var retro = vec3<f32>(0.0);
    if (sign) {
        if (flag(256u)) {
            let r = reflect(-vd, n);
            var sheen = mix(g.sky_bottom.rgb, g.sky_top.rgb, clamp(r.y * 2.0, 0.0, 1.0));
            if (g.env.x > 1.5) {
                sheen = env_gamma(textureSampleLevel(env_cube, samp, r, 0.0).rgb);
            }
            let fr = RETRO_F0 + (1.0 - RETRO_F0) * pow(1.0 - ndv, 5.0);
            retro = sheen * fr * RETRO_SHEEN + retro_light(i.wpos, n, vd) * t.rgb;
        }
    } else if (i.env > 8.5) {
        // env-mapped scene object (scene.rs `PROP_ENV`): the Classic picture's role (the surface is
        // the face colour times a reflection) is filled by the real surroundings, this frame's
        // env cube; the picture stays as a floor so a dark night does not blank them
        var refl = e.rgb;
        if (g.env.x > 1.5) {
            refl = PROP_ENV_BASE * e.rgb + PROP_ENV_GAIN * env_gamma(cube_soft(reflect(-vd, n), PROP_ENV_SPREAD));
            // the sun's glint and the point lights on the face
            let nl = dot(n, g.sun_dir.xyz);
            let hv = normalize(g.sun_dir.xyz + vd);
            let spec = select(0.0, pow(max(dot(n, hv), 0.0), PROP_ENV_SHINE) * g.sun.w * PROP_ENV_SPEC * vis, nl > 0.0);
            c = cap(c * refl + spec * g.sun.rgb + dl * i.albedo);
        } else {
            c = cap(c * refl);
        }
    } else if (i.env > 6.5) {
        // tyre rubber (procgen.rs `TYRE_ENV`): the tread lit per pixel through its normal map,
        // matte with a faint highlight from the roughness map
        let nr = textureSampleGrad(tex, samp_hq, i.uv, max(i32(i.layer + 1.5), 0), duv_x, duv_y);
        let nm = map_normal(nr, n, dp_x, dp_y, duv_x, duv_y);
        let nl = max(dot(nm, g.sun_dir.xyz), 0.0) * vis;
        let base = i.albedo * t.rgb * cap(max(amb + nl * g.sun.rgb + dl, vec3<f32>(0.0)));
        let rough = max(nr.a, 0.05);
        let shine = 2.0 / (rough * rough) - 2.0;
        let hv = normalize(g.sun_dir.xyz + vd);
        let spec = select(0.0, pow(max(dot(nm, hv), 0.0), shine) * (shine + 8.0) / 64.0 * g.sun.w * TYRE_SPEC * vis, nl > 0.0);
        c = cap(base + spec * g.sun.rgb);
    } else if (i.env > 5.5) {
        // carbon fibre: the weave lit per pixel through its normal map (OpenGL convention: green
        // up, our v runs down), its own highlight from the roughness map, a thin lacquer on top
        let nr = textureSampleGrad(tex, samp_hq, i.uv, max(i32(i.layer + 1.5), 0), duv_x, duv_y);
        let nm = map_normal(nr, n, dp_x, dp_y, duv_x, duv_y);
        let nl = max(dot(nm, g.sun_dir.xyz), 0.0) * vis;
        let base = i.albedo * t.rgb * cap(max(amb + nl * g.sun.rgb + dl, vec3<f32>(0.0)));
        let rough = max(nr.a, 0.05);
        let shine = 2.0 / (rough * rough) - 2.0;
        let hv = normalize(g.sun_dir.xyz + vd);
        let fibre = select(0.0, pow(max(dot(nm, hv), 0.0), shine) * (shine + 8.0) / 64.0 * g.sun.w * CARBON_SPEC * vis, nl > 0.0);
        var env = g.ambient.rgb + 0.5 * g.sun.rgb;
        if (g.env.x > 1.5) {
            env = env_gamma(textureSampleLevel(env_cube, samp, reflect(-vd, n), 0.0).rgb);
        } else if (g.env.x > 0.5) {
            env = e.rgb;
        }
        let fr = CARBON_F0 + (1.0 - CARBON_F0) * pow(1.0 - ndv, 5.0);
        let coat = select(0.0, pow(max(dot(n, hv), 0.0), CARBON_COAT_SHINE) * g.sun.w * CARBON_COAT_SPEC * vis, dot(n, g.sun_dir.xyz) > 0.0);
        c = cap(mix(base + fibre * g.sun.rgb * t.rgb * 2.0, env, fr) + coat * g.sun.rgb);
    } else if (i.env > 4.5) {
        // REMASTER car paint finishes (#136, #351, 09-remaster 9.41): gloss (4.75, Enhanced paint), metallic (5) and
        // pearl (5.25) by paint.wgsl's angle model. The surroundings along the mirrored eye ray: this frame's env
        // cube (the metallic flakes mirror a softened copy), else the env layer's sphere map, else a dim grey from
        // the track's light. The finish's reflection is the clear coat; the sun glint and flakes come from the model.
        let metallic = i.env > 4.9 && i.env < 5.15;
        var finish = fin_metallic();
        if (i.env < 4.9) {
            finish = fin_gloss();
        } else if (i.env > 5.15) {
            finish = fin_pearl();
        }
        let r = fin_reflect_dir(n, vd);
        var env = (g.ambient.rgb + 0.5 * g.sun.rgb) * PAINT_PLAIN_ENV;
        var env_soft = env;
        if (g.env.x > 1.5) {
            env = env_gamma(textureSampleLevel(env_cube, samp, r, 0.0).rgb);
            env_soft = select(env, env_gamma(cube_soft(r, METALLIC_SPREAD)), metallic);
        } else if (g.env.x > 0.5) {
            env = e.rgb;
            env_soft = env;
        }
        let nl = max(dot(n, g.sun_dir.xyz), 0.0) * vis;
        let light = cap(max(amb + nl * g.sun.rgb + dl, vec3<f32>(0.0)));
        let sunlit = nl > 0.0;
        let sun = select(vec3<f32>(0.0), g.sun.rgb * g.sun.w * vis, sunlit);
        let zone = c * g.gain.rgb;
        // the flake glints (metallic and pearl): a tilted normal per cell of the body in car space
        var flakes = vec3<f32>(0.0);
        if (i.env > 4.9 && sunlit) {
            let pearl = i.env > 5.15;
            let h = normalize(g.sun_dir.xyz + vd);
            let fl = paint_flake(car_space(i.wpos), n, h, dp_x, dp_y, select(METALLIC_FLAKE_CELL, PEARL_FLAKE_CELL, pearl), select(METALLIC_FLAKE_TILT, PEARL_FLAKE_TILT, pearl), select(METALLIC_FLAKE_SHINE, PEARL_FLAKE_SHINE, pearl));
            flakes = fl * select(METALLIC_FLAKE_GAIN, PEARL_FLAKE_GAIN, pearl) * sun * mix(vec3<f32>(1.0), fin_hue(zone), PAINT_FLAKE_SUN_TINT);
        }
        c = cap(fin_shade(finish, FinishIn(zone, n, vd, g.sun_dir.xyz, light, sun, env, env_soft, flakes, vis)));
    } else if (i.env > 3.5) {
        // self-lit lamp parts keep their colour
    } else if (i.env > 2.5) {
        // polished metal: the surroundings mirrored with a metal's Fresnel (this frame's cube,
        // else the sphere-mapped env layer, else a grey from the track's light), a sharp sun
        // highlight and a little diffuse
        var env = g.ambient.rgb + 0.5 * g.sun.rgb;
        if (g.env.x > 1.5) {
            env = env_gamma(cube_soft(reflect(-vd, n), METAL_SPREAD));
        } else if (g.env.x > 0.5) {
            env = e.rgb;
        }
        env = mix(env, vec3<f32>(dot(env, vec3<f32>(0.3, 0.59, 0.11))), METAL_GREY);
        let hue = i.albedo / max(max(i.albedo.r, max(i.albedo.g, i.albedo.b)), 1e-3);
        let f0 = METAL_F0 * hue;
        let fr = f0 + (vec3<f32>(1.0) - f0) * pow(1.0 - ndv, 5.0);
        let nl = max(dot(n, g.sun_dir.xyz), 0.0) * vis;
        let diffuse = METAL_BASE * hue * cap(max(amb + nl * g.sun.rgb + dl, vec3<f32>(0.0))) * METAL_DIFFUSE;
        let h = normalize(g.sun_dir.xyz + vd);
        let spec = select(0.0, pow(max(dot(n, h), 0.0), METAL_SHINE) * g.sun.w * METAL_SPEC * vis, nl > 0.0);
        c = cap(diffuse + env * fr * mix(0.6, 1.0, vis) + spec * g.sun.rgb * f0);
    } else if (i.env > 0.5) {
        // the Classic blend (0x43fcc9), under a lacquer that mirrors the env layer untinted
        let fr = select(0.0, GLOSS_F0 + (1.0 - GLOSS_F0) * pow(1.0 - ndv, 5.0), flag(4u));
        c = cap(mix(c * (0.4 + 1.1 * e.rgb), e.rgb, fr));
    }
    // REMASTER texture pack: the sun's highlight on the smoother patches of a roughness map (plain
    // asphalt, r about 1, gets none)
    if (packed_n && i.env < 0.5 && pr.a < 0.95 && dot(nn, g.sun_dir.xyz) > 0.0) {
        let rough = max(pr.a, 0.05);
        let shine = 2.0 / (rough * rough) - 2.0;
        let hv = normalize(g.sun_dir.xyz + vd);
        let fade = 1.0 - smoothstep(0.6, 0.95, rough);
        let sp = pow(max(dot(nn, hv), 0.0), shine) * (shine + 8.0) / 64.0 * g.sun.w * PACK_SPEC * vis * fade;
        c = cap(c + sp * g.sun.rgb);
    }
    var a = select(i.color.a, select(1.0, key, textured), ALPHA_TO_COVERAGE);
    if (linear() && !ALPHA_TO_COVERAGE) {
        // blended (shadows, skid marks): 1 - (1 - a)^2.2 darkens in linear light as much as
        // a darkens a black blend in gamma space
        a = 1.0 - pow(1.0 - a, GAMMA);
    }
    let fog = select(i.fog, fog_at(i.wpos), flag(2u));
    // the fog is mixed in gamma space, like the original, so the tracks keep their haze
    return vec4<f32>(out_rgb(fog_mix(surface_limit(c) + retro, i.wpos, fog)), a);
}

// REMASTER HDR: a lit surface rolls off above 1 towards SURFACE_MAX (gamma space) instead of
// growing without bound. A sun glint over a car roof reached several times the bloom
// threshold and bloomed into a blob driving towards the sun (Atlantika); lights (halos,
// flares) are drawn by their own shaders and keep their range [design choice].
const SURFACE_MAX: f32 = 1.2;
fn surface_limit(c: vec3<f32>) -> vec3<f32> {
    if (!linear()) {
        return c;
    }
    let k = SURFACE_MAX - 1.0;
    let over = max(c - vec3<f32>(1.0), vec3<f32>(0.0));
    return min(c, vec3<f32>(1.0)) + over / (vec3<f32>(1.0) + over / k);
}

// REMASTER detailed skid marks (`Pass::Skid`, drive.rs `Session::skid_marks(true)`) [design choice].
// A vertex carries the tread texture's uv (u across in tiles, v the metres along the strip in tiles),
// the strength 0..1 in colour.a (fades in and out at the ends, darker for a harder slide), the surface
// kind in env (1 asphalt, 2 concrete, 3 grass, 4 gravel; colour.rgb is its mark colour) and the
// distance across the strip (-1..1) in normal.x. Hard surfaces take rubber: a soft smear with the
// tread's lengthwise grooves as lighter streaks (the texture averaged along one pitch); soft ground
// takes the tread imprint itself.
const SKID_ALPHA: f32 = 0.75;
const SKID_EDGE: f32 = 0.4;
// rubber smears further: a wider soft edge, fainter grooves, the tread blurred across as well
const SKID_EDGE_RUBBER: f32 = 0.8;
const SKID_SMEAR_LOD: f32 = 2.0;
const SKID_TAPS: i32 = 6;
const SKID_GROOVE: f32 = 0.3;
const SKID_NOISE: f32 = 0.35;
// the tread texture's luma: the lands (0.155) against the grooves (0.08 .. 0.12)
const TREAD_LAND: f32 = 0.155;
const TREAD_DEPTH: f32 = 0.07;

// REMASTER: the skid pass's own varyings, centroid-sampled. With MSAA a plain varying is evaluated
// at the pixel centre even when only an edge sample is covered; a distant segment is a fraction of
// a pixel long, so the centre lies far outside it and the strength `color.a` extrapolates to
// values like -10 or 11, which blend as a huge boost or NaN (`pow` of a negative) and bloom as a
// white dot (#180). Centroid keeps the point inside the triangle.
struct SkidOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) @interpolate(perspective, centroid) uv: vec2<f32>,
    @location(1) @interpolate(perspective, centroid) color: vec4<f32>,
    @location(2) layer: f32,
    @location(3) @interpolate(perspective, centroid) fog: f32,
    @location(4) env: f32,
    @location(5) @interpolate(perspective, centroid) wpos: vec3<f32>,
    @location(6) @interpolate(perspective, centroid) across: f32,
};

@vertex
fn vs_skid(v: VsIn) -> SkidOut {
    var o: SkidOut;
    let world = g.model * vec4<f32>(v.pos, 1.0);
    o.clip = g.view_proj * world;
    o.uv = v.uv;
    o.color = v.color;
    o.layer = v.layer;
    o.fog = fog_at(world.xyz);
    o.env = v.env;
    o.wpos = world.xyz;
    o.across = v.normal.x;
    return o;
}

// PCG hash of a lattice point (a sine hash degrades far from the origin)
fn skid_hash(c: vec2<i32>) -> f32 {
    var h = u32(c.x) * 747796405u + u32(c.y) * 2891336453u;
    h = ((h >> ((h >> 28u) + 4u)) ^ h) * 277803737u;
    h = (h >> 22u) ^ h;
    return f32(h) / 4294967295.0;
}

fn skid_noise(p: vec2<f32>) -> f32 {
    let c = floor(p);
    let f = p - c;
    let u = f * f * (3.0 - 2.0 * f);
    let ci = vec2<i32>(c);
    let a = skid_hash(ci);
    let b = skid_hash(ci + vec2<i32>(1, 0));
    let d = skid_hash(ci + vec2<i32>(0, 1));
    let e = skid_hash(ci + vec2<i32>(1, 1));
    return mix(mix(a, b, u.x), mix(d, e, u.x), u.y);
}

@fragment
fn fs_skid(i: SkidOut) -> @location(0) vec4<f32> {
    let layer = max(i32(i.layer + 0.5), 0);
    // the imprint: the tread as is (uniform control flow for the derivatives)
    let print = textureSample(tex, samp_hq, i.uv, layer);
    let rubber = i.env < 2.5;
    // the smear: the tread averaged along one pitch leaves only its lengthwise grooves
    var avg = 0.0;
    for (var k = 0; k < SKID_TAPS; k++) {
        let uv = vec2<f32>(i.uv.x, i.uv.y + f32(k) / f32(SKID_TAPS));
        avg += textureSampleLevel(tex, samp_hq, uv, layer, SKID_SMEAR_LOD).g;
    }
    avg /= f32(SKID_TAPS);
    let groove = clamp((TREAD_LAND - avg) / TREAD_DEPTH, 0.0, 1.0);
    let contact = smoothstep(TREAD_LAND - 0.6 * TREAD_DEPTH, TREAD_LAND - 0.2 * TREAD_DEPTH, print.g);
    let pattern = select(contact, 1.0 - SKID_GROOVE * groove, rubber);
    let soft = select(SKID_EDGE, SKID_EDGE_RUBBER, rubber);
    let edge = 1.0 - smoothstep(1.0 - soft, 1.0, abs(i.across));
    // low-frequency unevenness over the ground
    let uneven = mix(1.0 - SKID_NOISE, 1.0, skid_noise(i.wpos.xz * 1.3));
    var a = saturate(i.color.a) * edge * pattern * uneven * SKID_ALPHA;
    // earth colours take the ground's light [APPROX: ambient plus the sun on level ground]
    let light = clamp(g.ambient.rgb + g.sun.rgb * max(g.sun_dir.y, 0.0), vec3<f32>(0.0), vec3<f32>(1.0));
    let c = select(min(i.color.rgb * light * g.gain.rgb, vec3<f32>(1.0)), i.color.rgb, rubber);
    if (linear()) {
        // as fs_remaster's blended draws: 1 - (1 - a)^2.2 darkens in linear light as much as a
        // darkens a black blend in gamma space
        a = 1.0 - pow(1.0 - saturate(a), GAMMA);
    }
    let fog = select(i.fog, fog_at(i.wpos), flag(2u));
    return vec4<f32>(out_rgb(fog_mix(c, i.wpos, fog)), a);
}

// CLASSIC skid marks (0x442911: untextured, no fog; the blend multiplies the frame by this colour,
// as a light-space multiplier in the linear Remaster target).
@fragment
fn fs_skid_flat(i: SkidOut) -> @location(0) vec4<f32> {
    return vec4<f32>(out_rgb(i.color.rgb), 1.0);
}

// CHEAT tribute to tron, REMASTER Neon grid (#320 phase 3, design choice): a car's light wall (drive.rs
// `Session::light_walls`; uv.x = age 0 at the car .. 1, uv.y = height 0 on the ground .. 1 at the top, rgb = the
// car's colour). A bright band along the top over a faint body, fading with age and into the black fog. Added to the
// frame (ONE / ONE); the strengths are linear light, the top band over the bloom threshold.
const WALL_EDGE: f32 = 2.5;
const WALL_EDGE_BAND: f32 = 0.14;
const WALL_BODY: f32 = 0.12;
@fragment
fn fs_light_wall(i: SkidOut) -> @location(0) vec4<f32> {
    let h = clamp(i.uv.y, 0.0, 1.0);
    let edge = smoothstep(1.0 - WALL_EDGE_BAND, 1.0 - 0.25 * WALL_EDGE_BAND, h);
    let body = WALL_BODY * mix(0.3, 1.0, h);
    let life = 1.0 - clamp(i.uv.x, 0.0, 1.0);
    let k = (WALL_EDGE * edge + body) * life * life * (1.0 - i.fog);
    let c = lin(i.color.rgb) * k;
    // a gamma target takes it back to gamma space (and clips at 1)
    return vec4<f32>(select(pow(min(c, vec3<f32>(1.0)), vec3<f32>(1.0 / GAMMA)), c, linear()), 0.0);
}

// Halos: texture * colour, added to the frame; no fog (the halo fades itself).
@fragment
fn fs_halo(i: VsOut) -> @location(0) vec4<f32> {
    let t = textureSample(tex, samp, i.uv, max(i32(i.layer + 0.5), 0));
    let c = t.rgb * i.color.rgb;
    // REMASTER HDR: a halo is light, not a surface colour: added as linear light as it is
    // (decoding would crush its soft edge), its core lifted over 1 for the bloom; lens flares
    // (colour alpha 0) are lens ghosts and get no gain [design choice]
    return vec4<f32>(select(c, c * mix(1.0, HALO_HDR_GAIN, i.color.a), linear()), 1.0);
}

fn hash1(x: f32) -> f32 {
    return fract(sin(x * 12.9898) * 43758.5453);
}

// One set of n rays around the centre of q (quad space, r = 1 at the edge): each ray is a
// thin line of linear half-width w, tapering and fading out at a length (and with a
// strength) varied per ray.
fn rays(q: vec2<f32>, n: f32, w: f32, seed: f32, len_var: f32, amp_var: f32) -> f32 {
    let r = length(q);
    let s = (atan2(q.y, q.x) - BURST_ANGLE) * n / 6.2831853;
    let k = floor(s + 0.5);
    let idx = k - n * floor(k / n);
    let h = hash1(idx + seed);
    let len = 1.0 - len_var * h;
    let d = r * abs(s - k) * 6.2831853 / n;
    let u = clamp(r / len, 0.0, 1.0);
    let taper = w * (1.0 - 0.7 * u);
    let fall = (1.0 - u) * (1.0 - u) / (1.0 + 6.0 * r);
    let amp = mix(1.0, hash1(idx + seed + 0.5), amp_var);
    return exp(-(d * d) / (taper * taper)) * fall * amp;
}

fn lens_burst(q: vec2<f32>) -> vec3<f32> {
    let r = length(q);
    let spikes = rays(q, BURST_RAYS, BURST_WIDTH, 1.0, 0.45, 0.3) * BURST_GAIN;
    let streaks = rays(q, STREAKS, STREAK_WIDTH, 7.0, 0.6, 1.0) * STREAK_GAIN;
    let edge = 1.0 - smoothstep(0.85, 1.0, r);
    let rr = (r - RING_R - vec3<f32>(RING_CHROMA, 0.0, -RING_CHROMA)) / RING_W;
    let ring = exp(-rr * rr) * RING_GAIN;
    return (vec3<f32>(spikes + streaks) + ring) * edge;
}

// REMASTER computed lens flare elements (render.rs FLARES_PROC) [design choice], quad space
// q (r = 1 at the edge), peak about 1: 0 soft disc, 1 hexagonal aperture ghost (a filled
// hexagon, brighter at its rim), 2 ring, 3 veil, 4 anamorphic streak (a quad much wider
// than high). Ghosts split red and blue a little at the rim like a lens does.
const FLARE_CHROMA: f32 = 0.04;
fn hex_dist(q: vec2<f32>) -> f32 {
    let a = abs(q);
    return max(a.x * 0.8660254 + a.y * 0.5, a.y);
}
fn flare_shape(q: vec2<f32>, kind: i32) -> vec3<f32> {
    let r = length(q);
    let ch = vec3<f32>(1.0 + FLARE_CHROMA, 1.0, 1.0 - FLARE_CHROMA);
    switch kind {
        case 0: {
            return (1.0 - smoothstep(vec3<f32>(0.0), vec3<f32>(1.0), r * ch)) * (0.5 + 0.5 * r);
        }
        case 1: {
            let d = hex_dist(q);
            let body = 1.0 - smoothstep(vec3<f32>(0.85), vec3<f32>(1.0), d * ch);
            return body * (0.35 + 0.65 * smoothstep(0.4, 0.9, d));
        }
        case 2: {
            let x = (r * ch - 0.8) / 0.08;
            return exp(-x * x);
        }
        case 3: {
            return vec3<f32>(exp(-r * r * 3.0) * (1.0 - smoothstep(0.7, 1.0, r)));
        }
        default: {
            // streak: bright thin core along x, fading to both ends
            let along = 1.0 - abs(q.x);
            return vec3<f32>(exp(-q.y * q.y * 40.0) * along * along);
        }
    }
}

// REMASTER: halos with a computed shape (flag 16) instead of the 32-colour sprites; the
// texture's tint comes in the normal (render.rs halo_quads). Lens flares and additive
// particles share the pass and have no tint: they keep their textures, as in fs_halo.
@fragment
fn fs_halo_remaster(i: VsOut) -> @location(0) vec4<f32> {
    let t = textureSample(tex, samp, i.uv, max(i32(i.layer + 0.5), 0));
    let sprite = t.rgb * i.color.rgb;
    var c = select(sprite, sprite * mix(1.0, HALO_HDR_GAIN, i.color.a), linear());
    if (flag(64u) && i.color.a < -0.5) {
        // REMASTER computed lens flare element, kind = -alpha - 1 (render.rs FLARES_PROC)
        let tint = i.wnormal * i.color.rgb;
        return vec4<f32>(tint * flare_shape(i.uv * 2.0 - 1.0, i32(-i.color.a - 0.5)), 1.0);
    }
    if (flag(16u) && i.color.a > 1.5 && i.color.a < 3.5) {
        let q = i.uv * 2.0 - 1.0;
        let tint = i.wnormal * i.color.rgb;
        // gamma space has no bloom to lift the rays: a bit more there
        let gain = select(BURST_LDR_GAIN, 1.0, linear());
        return vec4<f32>(tint * lens_burst(q) * gain, 1.0);
    }
    if (flag(16u) && dot(i.wnormal, i.wnormal) > 0.0) {
        var q = i.uv * 2.0 - 1.0;
        if (i.color.a > 3.5) {
            // REMASTER lamp strip (render.rs halo_quads, alpha 4): the profile below across the
            // bar, flat along it, so strips end to end add up to one even line
            q = vec2<f32>(0.0, q.y);
        }
        let r2 = dot(q, q);
        let core = exp(-r2 / (HALO_SIGMA * HALO_SIGMA));
        let tail = (1.0 - smoothstep(0.0, HALO_TAIL_R, sqrt(r2))) / (1.0 + HALO_TAIL_K * r2);
        let tint = i.wnormal * i.color.rgb;
        if (flag(32u)) {
            c = tint * (core * HALO_CORE_GAIN + tail * HALO_TAIL_GAIN);
        } else {
            c = tint * (core * HALO_CORE_W + tail * HALO_TAIL_W) * select(1.0, HALO_HDR_GAIN, linear());
        }
    }
    return vec4<f32>(c, 1.0);
}

// REMASTER: the world normal of a normal map texel `nr` (OpenGL convention: green up, our v runs
// down) on the surface with normal `n`, its tangent frame from the screen derivatives.
fn map_normal(nr: vec4<f32>, n: vec3<f32>, dp_x: vec3<f32>, dp_y: vec3<f32>, duv_x: vec2<f32>, duv_y: vec2<f32>) -> vec3<f32> {
    let p2 = cross(dp_y, n);
    let p1 = cross(n, dp_x);
    let tg = p2 * duv_x.x + p1 * duv_y.x;
    let bt = p2 * duv_x.y + p1 * duv_y.y;
    let k = inverseSqrt(max(max(dot(tg, tg), dot(bt, bt)), 1e-20));
    let tn = nr.rgb * 2.0 - vec3<f32>(1.0);
    return normalize(tg * k * tn.x - bt * k * tn.y + n * max(tn.z, 0.2));
}

// REMASTER sparks (09-remaster 9.16) [design choice]: thin hot streaks, head at uv.x = 1, fading
// to the tail (vblur.wgsl `fs_spark` is the same shape, drawn after the velocity blur).
const SPARK_HDR_GAIN: f32 = 4.0;

@fragment
fn fs_spark(i: VsOut) -> @location(0) vec4<f32> {
    let across = i.uv.y * 2.0 - 1.0;
    let shape = exp(-across * across * 6.0) * pow(i.uv.x, 1.5) * smoothstep(1.0, 0.9, i.uv.x);
    return vec4<f32>(out_rgb(i.color.rgb) * shape * i.color.a * select(1.0, SPARK_HDR_GAIN, linear()), 1.0);
}

// REMASTER smoke and dust (09-remaster 9.16) [design choice]: the smoke frames are dark
// density maps on a colour key, so the brightest channel is the opacity. The puff is lit by
// the track's sun and ambient, fogged, alpha-blended, and fades out under the ground height in
// the vertex normal's x (a soft contact with the road).
const SMOKE_DENSITY: f32 = 3.0;
const SMOKE_SOFT: f32 = 0.45;
const SMOKE_SUN: f32 = 0.6;
const SMOKE_FLOOR: f32 = 0.05;
const SMOKE_BLUR: f32 = 0.012;

@fragment
fn fs_smoke(i: VsOut) -> @location(0) vec4<f32> {
    // the frames are small and were resampled nearest to 256^2: a 5-tap blur and a round
    // window hide the texel blocks and the square edge
    let layer = max(i32(i.layer + 0.5), 0);
    var dens = 0.0;
    for (var k = 0; k < 5; k++) {
        let o = select(vec2<f32>(0.0), vec2<f32>(select(-1.0, 1.0, (k & 1) == 1), select(-1.0, 1.0, k > 2)) * SMOKE_BLUR, k > 0);
        let t = textureSample(tex, samp, i.uv + o, layer);
        dens += max(t.r, max(t.g, t.b)) * t.a * 0.2;
    }
    let q = i.uv * 2.0 - 1.0;
    dens = clamp(dens * SMOKE_DENSITY, 0.0, 1.0) * (1.0 - smoothstep(0.6, 1.0, length(q)));
    let ground = smoothstep(0.0, SMOKE_SOFT, i.wpos.y - i.wnormal.x);
    let light = min(g.ambient.rgb + g.sun.rgb * SMOKE_SUN, vec3<f32>(1.2)) + vec3<f32>(SMOKE_FLOOR);
    // drawn after the volume pass, so it keeps the Remaster fog of its own
    let c = select(mix(i.color.rgb * light, g.fog.rgb, fog_at(i.wpos)), fog_remaster(i.color.rgb * light, i.wpos), flag(1024u));
    return vec4<f32>(out_rgb(c), dens * ground * i.color.a);
}

// The sky in an env-probe face: the band by direction, placed for a level camera on a
// 4:3 screen (sky_geom = band top, band height, repeats around 360, tan of half the vertical
// fov) [design choice].
@fragment
fn fs_sky_cube(i: SkyOut) -> @location(0) vec4<f32> {
    let p = g.sky_inv * vec4<f32>(i.screen.x * 2.0 - 1.0, 1.0 - i.screen.y * 2.0, 1.0, 1.0);
    let d = normalize(p.xyz / p.w - g.cam_pos.xyz);
    let sy = 0.5 - d.y / max(length(d.xz), 1e-4) / (2.0 * g.sky_geom.w);
    let v = (sy - g.sky_geom.x) / max(g.sky_geom.y, 1e-6);
    let u = atan2(d.x, -d.z) * g.sky_geom.z / 6.2831853;
    let half = 0.5 / f32(textureDimensions(sky_tex).y);
    let t = textureSampleLevel(sky_tex, samp, vec2<f32>(u, clamp(v, half, 1.0 - half)), 0.0);
    var c = t.rgb;
    if (v < 0.0) {
        c = g.sky_top.rgb;
    } else if (v > 1.0) {
        c = g.sky_bottom.rgb;
    }
    return vec4<f32>(out_rgb(fog_sky(c, d)), 1.0);
}

struct SkyOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) screen: vec2<f32>,
};

@vertex
fn vs_sky(@builtin(vertex_index) i: u32) -> SkyOut {
    let p = vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u)) * 2.0 - 1.0;
    var o: SkyOut;
    o.clip = vec4<f32>(p, 1.0, 1.0);
    o.screen = vec2<f32>(p.x * 0.5 + 0.5, 0.5 - p.y * 0.5);
    return o;
}

// REMASTER sky dome (09-remaster 9.32) [design choice]: the track's HDRI by view direction.
// fog3.y = turn that puts the HDRI's sun under the track's sun, fog3.z = exposure.
const DOME_MAX: f32 = 64.0;

// The dome's linear radiance (exposed, clamped) towards the world direction `d`; `lod` blurs it for
// reflections. fog3.z > 0 means this view has a dome (render.rs `dome_sun`).
fn dome_radiance(d: vec3<f32>, lod: f32) -> vec3<f32> {
    let u = (atan2(d.x, -d.z) - g.fog3.y) / 6.2831853 + 0.5;
    let half = 0.5 / f32(textureDimensions(sky_hdri).y);
    let v = clamp(acos(clamp(d.y, -1.0, 1.0)) / 3.1415927, half, 1.0 - half);
    return min(textureSampleLevel(sky_hdri, samp, vec2<f32>(u, v), lod).rgb * g.fog3.z, vec3<f32>(DOME_MAX));
}

// CHEAT tribute to tron, REMASTER Neon grid (#320, design choice): the dome (Goldbridge's, scene.rs `TRON_SKY_TRACK`)
// as its luminance in a dark cyan, unfogged, like the Tron Legacy night sky
// (contrast: the luminance to `TRON_SKY_CURVE`, so the cloud edges carry the colour and the gaps go near black)
const TRON_SKY_TINT: vec3<f32> = vec3<f32>(0.1, 0.6, 0.85);
const TRON_SKY_GAIN: f32 = 0.7;
const TRON_SKY_CURVE: f32 = 1.3;
fn tron_dome(c: vec3<f32>) -> vec4<f32> {
    let t = TRON_SKY_TINT * pow(dot(c, vec3<f32>(0.2126, 0.7152, 0.0722)), TRON_SKY_CURVE) * TRON_SKY_GAIN;
    return vec4<f32>(select(dome_gamma(t), t, linear()), 1.0);
}

// Radiance into a gamma-space colour: a soft shoulder (extended Reinhard, white point 4), then encode.
fn dome_gamma(c: vec3<f32>) -> vec3<f32> {
    return pow(c * (1.0 + c / 16.0) / (1.0 + c), vec3<f32>(1.0 / GAMMA));
}

// REMASTER fog on the dome (17-fog-rework.md): in linear light the radiance is mixed as it is
// (`lod` blurs it for reflections), in gamma space it goes through the soft shoulder first.
fn dome_out(d: vec3<f32>, lod: f32) -> vec4<f32> {
    let c = dome_radiance(d, lod);
    if (flag(TRON_FLAG)) {
        return tron_dome(c);
    }
    if (linear()) {
        if (g.params.w < 0.5 || !flag(1024u) || flag(2048u)) {
            return vec4<f32>(c, 1.0);
        }
        return vec4<f32>(mix(c, fog_colour(d), fog_sky_amount(d)), 1.0);
    }
    return vec4<f32>(fog_sky(dome_gamma(c), d), 1.0);
}

@fragment
fn fs_sky_dome(i: SkyOut) -> @location(0) vec4<f32> {
    return dome_out(sky_dir(i.screen), 0.0);
}

// The dome in an env-probe face (Enhanced paint reflections): linear HDR like the rest of the
// cube, blurred to the face's resolution (128 px over 90 degrees against the image's 0.18
// degrees per texel).
@fragment
fn fs_sky_cube_dome(i: SkyOut) -> @location(0) vec4<f32> {
    return dome_out(sky_dir(i.screen), 2.0);
}

// Background (0x443a68): a band of the BackgroundName image across the screen, TopColour
// above it and BottomColour below.
@fragment
fn fs_sky(i: SkyOut) -> @location(0) vec4<f32> {
    let v = (i.screen.y - g.sky_geom.x) / max(g.sky_geom.y, 1e-6);
    let u = g.sky_geom.z + (i.screen.x - 0.5) * g.sky_geom.w;
    // keep V on texel centres so the repeating sampler doesn't blend the top and bottom rows
    let half = 0.5 / f32(textureDimensions(sky_tex).y);
    let t = textureSampleLevel(sky_tex, samp, vec2<f32>(u, clamp(v, half, 1.0 - half)), 0.0);
    var c = t.rgb;
    if (v < 0.0) {
        c = g.sky_top.rgb;
    } else if (v > 1.0) {
        c = g.sky_bottom.rgb;
    }
    // REMASTER: fogged by direction like the dome (the Classic band keeps its colours)
    return vec4<f32>(out_rgb(fog_sky(c, sky_dir(i.screen))), 1.0);
}

// CHEAT lemmy / ace "Hell mode" (#316) and ramlosa / h2o "Underwater" (#317) (0x443570): the sky is two screen quads. The upper (0..0.6 of the height) is the
// track's EnvMapName texture over the cheat's gradient (red to purple, blue to teal), with the original's rhw 10 at the top and 1.11 at the
// bottom: the texture coordinates are perspective-correct (a ceiling plane), the colour is linear in screen space like
// D3D's Gouraud. The lower quad is plain, the bottom colour. Eight vertices in `g.hell`, two triangles each.
struct HellOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) @interpolate(linear) mixv: f32,
    @location(2) @interpolate(flat) textured: f32,
};

var<private> HELL_IDX: array<u32, 12> = array<u32, 12>(0u, 1u, 2u, 0u, 2u, 3u, 4u, 5u, 6u, 4u, 6u, 7u);

@vertex
fn vs_hell_sky(@builtin(vertex_index) i: u32) -> HellOut {
    let k = HELL_IDX[i];
    let a = g.hell[2u * k];
    let b = g.hell[2u * k + 1u];
    var o: HellOut;
    o.clip = vec4<f32>(a.x * a.z, a.y * a.z, 0.999 * a.z, a.z);
    o.uv = b.xy;
    o.mixv = b.z;
    o.textured = a.w;
    return o;
}

@fragment
fn fs_hell_sky(i: HellOut) -> @location(0) vec4<f32> {
    // sample in uniform control flow, then decide (the plain quad has no texture)
    let t = textureSample(tex, samp, i.uv, max(i32(g.env.y + 0.5), 0));
    var c = mix(g.sky_top.rgb, g.sky_bottom.rgb, i.mixv);
    if (i.textured > 0.5) {
        c = c * t.rgb;
    }
    return vec4<f32>(out_rgb(c), 1.0);
}
