// ---------------------------------------------------------------------------------------------
// REMASTER grass (procgen/grass.rs; re-notes/18-grass.md) [design choice]: shell turf over the
// world's grass polygons. Appended to shader.wgsl into a module of its own (render.rs
// `Gpu::grass_shader`): no bindings of its own, it reads the world texture array and the
// globals. `g.grass` holds 5 vec4 per body:
//   0: height, density (cells/m), thickness, shells   1: reach, sway, root shade, edge fade
//   2: tint rgb, tip mix                               3: tip rgb, 0
//   4: blade density (cells/m), blade width, blade reach, blade bend   (the Ultra blades)
// `vs_grass` draws the polygon `shells` times (instances) lifted along the smoothed normal by
// height x (i + 1) / shells. Within EdgeFade of the grass's outline the blades shorten to
// nothing (`grass_fade`: the edge map of group 3, the distance to the non-grass world per
// pixel; the vertex distance `color.a` when a scene has no map). `fs_grass` hashes
// the world XZ into blade cells (a random height and offset per cell, a cone tapering to the
// tip) and keeps the pixels inside a blade: the gaps show the shells below, down to the base
// polygon that the world pass drew. The colour is the original texel x the baked vertex colour
// x a root-to-tip shade, then the same shadow, lamp, dynamic light and fog terms as a world
// pixel of fs_remaster. The blades shrink to nothing at Reach, so far grass is the plain texture.
//
// Ultra (Gitea #31, render.rs `Gpu::blade_pass`): real blade geometry near the camera.
// `cs_blades` walks the same grass triangles (the shells' vertex buffer bound as storage) and,
// per cell of a world-space grid of BladeDensity cells per metre inside the triangle and within
// BladeReach of the camera, appends a `Blade` record (root, height, colour, texel, normal,
// lean = wind + press) and counts into the indirect args; `vs_blade` / `fs_blade` draw a bent
// strip per record, turned toward the camera, lighter at the tip and on the sun side. The cells
// are hashed by their integer coordinates (PCG), so the blades stand still across frames,
// and a jittered point belongs to exactly one triangle, so shared edges make no doubles. From
// 60 % of BladeReach out the blades thin (their width growing) and the shells rise from zero
// (`hand` in vs_grass / fs_grass), so the two hand over without a seam.
// ---------------------------------------------------------------------------------------------

const GRASS_ROWS: u32 = 5u;
const GRASS_ENV: f32 = 32.0;
// wind: direction and how fast the gusts drift (cells of noise per metre, per second)
const GRASS_WIND_DIR: vec2<f32> = vec2<f32>(0.8, 0.6);
const GRASS_GUST_SCALE: f32 = 0.15;
const GRASS_GUST_SPEED: vec2<f32> = vec2<f32>(0.7, 0.25);
// per-blade height 0.45..1 of the body's, root offset within the cell, brightness variation
const GRASS_HEIGHT_MIN: f32 = 0.45;
const GRASS_JITTER: f32 = 0.9;
const GRASS_VARY: f32 = 0.3;
// cells per pixel where the blades are whole (10 px per cell), and where they have faded into
// the texture (2 px): smaller blades would sparkle
const GRASS_ATT_FULL: f32 = 0.1;
const GRASS_ATT_NONE: f32 = 0.5;
// cars pressing the blades (`grass_press`, phase 3): the footprint is the hull ellipse scaled
// by PRESS_IN (fully flat inside) to PRESS_OUT (untouched outside), sampled where the car is
// and where it was up to PRESS_TAIL seconds ago along its velocity (the blades spring up behind
// it); the blades under it keep PRESS_LEFT of their height and lean away from the car by
// PRESS_LEAN metres at the tip. A car PRESS_ABOVE metres above or below the grass (a bridge)
// does not count. The lasting trail (#158) is the `grass_trail` texture (trail.wgsl): how
// recently the cars pressed each spot, 1 .. 0 over TRAIL_RELAX seconds, an extra flattening
// term without a lean.
const GRASS_PRESS_IN: f32 = 0.85;
const GRASS_PRESS_OUT: f32 = 1.5;
const GRASS_PRESS_TAIL: f32 = 0.1;
const GRASS_PRESS_LEFT: f32 = 0.12;
const GRASS_PRESS_LEAN: f32 = 0.18;
const GRASS_PRESS_ABOVE: f32 = 2.5;

fn grass_row(b: u32, r: u32) -> vec4<f32> {
    return g.grass[b * GRASS_ROWS + r];
}

// the edge map (procgen/grass.rs `EdgeMap`): R8, metres / grass_n.y, 0 off the grass
@group(3) @binding(0) var grass_edge: texture_2d<f32>;
@group(3) @binding(1) var grass_edge_samp: sampler;
// the trail (render.rs `Gpu::trail_pass`): R8 over the same rectangle, 1 freshly pressed
@group(3) @binding(2) var grass_trail: texture_2d<f32>;

// The edge fade at a world position, 0 at the grass's outline .. 1 past the body's EdgeFade:
// the map's distance, or `vertex` (the interpolated vertex distance) without a map.
fn grass_fade(wpos: vec3<f32>, vertex: f32, edge_fade: f32) -> f32 {
    let uv = (wpos.xz - g.grass_map.xy) * g.grass_map.zw;
    let m = textureSampleLevel(grass_edge, grass_edge_samp, uv, 0.0).r * g.grass_n.y;
    return smoothstep(0.0, max(edge_fade, 1e-3), select(vertex, m, g.grass_n.y > 0.0));
}

struct GrassOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) color: vec3<f32>,
    @location(2) @interpolate(flat) layer: i32,
    @location(3) wpos: vec3<f32>,
    @location(4) wnormal: vec3<f32>,
    // the shell's height fraction (0 = ground, 1 = the top shell)
    @location(5) h: f32,
    @location(6) @interpolate(flat) body: u32,
    // the vertex's outline distance (metres; the fallback without an edge map)
    @location(7) edge: f32,
    // Ultra: the shells' share against the blades (0 within 60 % of BladeReach .. 1 past it)
    @location(8) hand: f32,
};

// PCG integer hash (Jarzynski & Olano 2020; post.wgsl has the same). The usual
// fract(sin(dot(..))) hash fails at cell coordinates in the thousands (a verge 600 m from the
// origin at 20 cells/m): its sine argument is in the millions, where f32 and the GPU's range
// reduction keep only a few dozen phases, so the jitter repeats along each cell row and the
// blades stand in even rows.
fn grass_pcg(v: u32) -> u32 {
    let s = v * 747796405u + 2891336453u;
    let w = ((s >> ((s >> 28u) + 4u)) ^ s) * 277803737u;
    return (w >> 22u) ^ w;
}

// four uniform 0..1 values of a blade cell (integer coordinates) and a salt
fn grass_hash4(cell: vec2<i32>, salt: u32) -> vec4<f32> {
    let s = grass_pcg(u32(cell.x) + grass_pcg(u32(cell.y) + salt));
    return vec4<f32>(f32(grass_pcg(s) >> 8u), f32(grass_pcg(s + 1u) >> 8u), f32(grass_pcg(s + 2u) >> 8u), f32(grass_pcg(s + 3u) >> 8u)) / 16777216.0;
}

// one value of an integer-valued point (the noise lattice)
fn grass_hash(p: vec2<f32>) -> f32 {
    return grass_hash4(vec2<i32>(floor(p)), 0u).x;
}

// value noise 0..1
fn grass_noise(p: vec2<f32>) -> f32 {
    let i = floor(p);
    let f = fract(p);
    let u = f * f * (3.0 - 2.0 * f);
    let a = grass_hash(i);
    let b = grass_hash(i + vec2<f32>(1.0, 0.0));
    let c = grass_hash(i + vec2<f32>(0.0, 1.0));
    let d = grass_hash(i + vec2<f32>(1.0, 1.0));
    return mix(mix(a, b, u.x), mix(c, d, u.x), u.y);
}

// How much the cars press the grass at `wpos`: xy the lean (world xz, metres at the tip, away
// from the nearest pressing car), z the flattening 0 (none) .. 1 (under the hull), the most of
// the live cars and the lasting trail.
fn grass_press(wpos: vec3<f32>) -> vec3<f32> {
    var lean = vec2<f32>(0.0);
    var flat = 0.0;
    if (g.grass_n.y > 0.0) {
        // the trail: shaped so the turf stays flat a while, then springs up
        let uv = (wpos.xz - g.grass_map.xy) * g.grass_map.zw;
        let tr = textureSampleLevel(grass_trail, grass_edge_samp, uv, 0.0).r;
        flat = tr * (2.0 - tr);
    }
    let n = u32(g.grass_n.x);
    for (var k = 0u; k < n; k++) {
        let a = g.grass_cars[3u * k];
        let b = g.grass_cars[3u * k + 1u];
        let vel = g.grass_cars[3u * k + 2u].xy;
        if (abs(wpos.y - a.y) > GRASS_PRESS_ABOVE) {
            continue;
        }
        // a cheap reject: outside the footprint's reach plus the trail
        let d0 = wpos.xz - a.xz;
        let reach = max(a.w, b.z) * GRASS_PRESS_OUT + length(vel) * GRASS_PRESS_TAIL;
        if (dot(d0, d0) > reach * reach) {
            continue;
        }
        let fwd = b.xy;
        // the car now and at two moments back along its velocity
        for (var j = 0; j < 3; j++) {
            let d = wpos.xz - a.xz + vel * (GRASS_PRESS_TAIL * f32(j) / 2.0);
            let u = dot(d, fwd);
            let w = dot(d, vec2<f32>(-fwd.y, fwd.x));
            let e = length(vec2<f32>(u / max(a.w, 0.1), w / max(b.z, 0.1)));
            let p = 1.0 - smoothstep(GRASS_PRESS_IN, GRASS_PRESS_OUT, e);
            if (p > flat) {
                flat = p;
                lean = normalize(d + vec2<f32>(1e-3, 0.0)) * (p * GRASS_PRESS_LEAN);
            }
        }
    }
    return vec3<f32>(lean, flat);
}

@vertex
fn vs_grass(v: VsIn, @builtin(instance_index) inst: u32) -> GrassOut {
    var o: GrassOut;
    let b = u32(clamp(v.env - GRASS_ENV, 0.0, 3.0));
    let r0 = grass_row(b, 0u);
    let r1 = grass_row(b, 1u);
    let shells = max(r0.w, 1.0);
    let h = f32(inst + 1u) / shells;
    // with the edge map the fade is per pixel (fs_grass) and the shells lift fully: near the
    // outline the blades are short, so nothing of the lifted slices shows there. Without one
    // the shells collapse at the outline along the interpolated vertex distance.
    let fade = select(smoothstep(0.0, max(r1.w, 1e-3), v.color.a), 1.0, g.grass_n.y > 0.0);
    let n = normalize(v.normal);
    let p = v.pos + n * (r0.x * h * fade);
    o.clip = g.view_proj * vec4<f32>(p, 1.0);
    if (f32(inst) >= shells) {
        // past this body's shell count (another body has more): off screen
        o.clip = vec4<f32>(0.0, 0.0, 2.0, 1.0);
    }
    o.uv = v.uv;
    o.color = min(v.color.rgb * g.gain.rgb, vec3<f32>(1.0));
    o.layer = max(i32(v.layer + 0.5), 0);
    o.wpos = p;
    o.wnormal = n;
    o.h = h;
    o.body = b;
    o.edge = v.color.a;
    // Ultra: the shells shorten to nothing where the blades are (fs_grass scales the blade
    // height by it, like the distance LOD, so the top shells vanish first)
    let r4 = grass_row(b, 4u);
    o.hand = select(1.0, smoothstep(r4.z * BLADE_HAND, r4.z, distance(g.cam_pos.xyz, v.pos)), g.grass_n.z > 0.5);
    return o;
}

@fragment
fn fs_grass(i: GrassOut) -> @location(0) vec4<f32> {
    let r0 = grass_row(i.body, 0u);
    let r1 = grass_row(i.body, 1u);
    let r2 = grass_row(i.body, 2u);
    let r3 = grass_row(i.body, 3u);
    // derivatives before any discard (uniform control flow)
    let duv_x = dpdx(i.uv);
    let duv_y = dpdy(i.uv);
    // Ultra: inside the blades' zone the shells keep nothing, so skip the per-pixel work
    if (g.grass_n.z > 0.5 && i.hand <= 0.0) {
        discard;
    }
    let d = distance(g.cam_pos.xyz, i.wpos);
    // the blades shrink with distance and vanish at Reach
    let lod = clamp(1.0 - d / max(r1.x, 1.0), 0.0, 1.0);
    // wind: the tips lean along GRASS_WIND_DIR by a drifting gust
    let t = g.water_t.x;
    let gust = grass_noise(i.wpos.xz * GRASS_GUST_SCALE + t * GRASS_GUST_SPEED);
    let wind = GRASS_WIND_DIR * (r1.y * i.h * i.h * (0.2 + 0.8 * gust));
    // the cars press the blades: shorter under the hull, leaning away from it
    let press = grass_press(i.wpos);
    // blade cells in world XZ
    let q = (i.wpos.xz - wind - press.xy * i.h) * r0.y;
    let hc = grass_hash4(vec2<i32>(floor(q)), 0u);
    let h1 = hc.x;
    let h2 = hc.y;
    let h3 = hc.z;
    let local = fract(q) - 0.5 - (vec2<f32>(h1, h2) - 0.5) * GRASS_JITTER;
    let fw = fwidth(local);
    // this blade's height as a shell fraction: random, shorter far away and at the outline
    let fade = grass_fade(i.wpos, i.edge, r1.w);
    // (the interpolated hand-off is exactly 1 only when gated: Full stays bit-identical)
    let hand = select(1.0, i.hand, g.grass_n.z > 0.5);
    let hb = (GRASS_HEIGHT_MIN + (1.0 - GRASS_HEIGHT_MIN) * h3) * lod * hand * fade * mix(1.0, GRASS_PRESS_LEFT, press.z);
    let hr = i.h / max(hb, 1e-4);
    // a cone: the radius tapers from thickness/2 at the root to 0 at the tip
    let radius = r0.z * 0.5 * (1.0 - hr);
    let dist = length(local);
    var alpha = clamp((radius - dist) / max(length(fw), 1e-4) + 0.5, 0.0, 1.0);
    // cells near a pixel in size would sparkle: the coverage thins out and the blade colour
    // converges on the texel's (`att` 1 when a cell spans 10 pixels or more, 0 at 2), so with
    // and without MSAA the far grass settles to the plain texture
    // (the narrow axis: at a grazing view a cell is long along the view but the blade's width
    // on screen is what sparkles)
    let att = clamp((GRASS_ATT_NONE - min(fw.x, fw.y)) / (GRASS_ATT_NONE - GRASS_ATT_FULL), 0.0, 1.0);
    alpha = alpha * att;
    if (hr >= 1.0 || alpha <= 0.0 || (!ALPHA_TO_COVERAGE && alpha < 0.5)) {
        discard;
    }
    let tx = textureSampleGrad(tex, samp_hq, i.uv, i.layer, duv_x, duv_y);
    let shade = mix(1.0, mix(r1.z, 1.0, hr) * (1.0 - GRASS_VARY * 0.5 + GRASS_VARY * h2), att);
    let tint = mix(vec3<f32>(1.0), mix(r2.rgb, r3.rgb, r2.w * hr), att);
    let c = i.color * shade * tint;
    return vec4<f32>(grass_light(c, tx.rgb, i.wpos, i.wnormal), select(1.0, alpha, ALPHA_TO_COVERAGE));
}

// REMASTER world terms of a grass pixel (the shells and the Ultra blades), as fs_remaster
// shades a world pixel: the sun shadow map darkens the baked colour `c`, the lamp shadows, the
// dynamic lights, then the texel `tx`, the fog, and the output transfer.
fn grass_light(c0: vec3<f32>, tx: vec3<f32>, wpos: vec3<f32>, wnormal: vec3<f32>) -> vec3<f32> {
    var c = c0;
    if (flag(128u)) {
        let vis = shadow_vis(wpos, sh_u.p.z);
        let luma = vec3<f32>(0.3, 0.59, 0.11);
        let sun = dot(g.sun.rgb, luma);
        let share = sun / (sun + dot(g.ambient.rgb, luma) + 1e-3);
        let dark = select(min(share * SHADOW_SHARE_GAIN, SHADOW_WORLD), OVERHEAD_WORLD, sh_u.p.w > 0.0);
        c = c * (1.0 - (1.0 - vis) * dark);
    }
    if (flag(512u) && g.lamp_n.x > 0.5) {
        c = c * lamp_world_shadow(wpos, wnormal);
    }
    if (g.env.z + g.spot_n.x + g.lamp_n.x > 0.5) {
        c = cap(c + dynamic_light(wpos, wnormal));
    }
    c = c * tx;
    let fog = fog_at(wpos);
    return out_rgb(fog_mix(surface_limit(c), wpos, fog));
}

// ---------------------------------------------------------------------------------------------
// Ultra blades (render.rs `BladeGpu`, `Gpu::blade_pass`)
// ---------------------------------------------------------------------------------------------

// the most records kept (render.rs `ULTRA_MAX_BLADES`) and the workgroups per triangle
// (`BLADE_SLICES`), both set from Rust
override BLADE_MAX: u32 = 2097152u;
override BLADE_SLICES: u32 = 4u;
override BLADE_GROUP: u32 = 256u;
// bent segments per blade (render.rs `BLADE_SEGMENTS`: the strip has 2 x (this + 1) vertices)
const BLADE_SEGMENTS: u32 = 3u;
// past this share of BladeReach the blades bend in one segment fewer
const BLADE_LOD_FAR: f32 = 0.5;
// the hand-off band starts at this share of BladeReach: the blades thin out and the shells
// rise from here to BladeReach
const BLADE_HAND: f32 = 0.6;
// the widest dispatch row (render.rs `BLADE_DISPATCH_W`): triangle = x + y * this
const BLADE_DISPATCH_W: u32 = 65535u;
// a blade shorter than this (metres) is not worth a record
const BLADE_MIN_HEIGHT: f32 = 0.01;
// the mip level of the texel a blade takes its colour from (0: the full texture)
const BLADE_TEXEL_LOD: f32 = 0.0;
// the most lean (metres at the tip, wind + press) the record holds (16-bit snorm)
const BLADE_LEAN_MAX: f32 = 0.5;
// per-blade width 0.7..1.3 of the body's BladeWidth (6 bits of the record)
const BLADE_WIDTH_MIN: f32 = 0.7;
const BLADE_WIDTH_VARY: f32 = 0.6;
// the bend: BladeBend 1 tips the blade over by this sine (0.6 = 37 degrees), x 0.75..1.25 per blade
const BLADE_BEND_MAX: f32 = 0.6;
// how much of the tip's offset the Bezier control point (halfway up) takes: the lower part
// stands, the top bends over
const BLADE_BEND_CP: f32 = 0.3;
// the random facing mixed this far toward the camera's right, so no blade is seen edge-on
const BLADE_FACE_CAM: f32 = 0.5;
// the narrowest a blade's root is drawn, in pixels (thinner ones sparkle)
const BLADE_MIN_PX: f32 = 1.2;
// the side gradient: the sun side of the strip this much lighter, the other darker
const BLADE_SIDE: f32 = 0.15;

// scene::Vertex (56 bytes) as the storage array sees it: scalars, so no vec3 padding
struct GVert {
    px: f32, py: f32, pz: f32,
    u: f32, v: f32,
    r: f32, g: f32, b: f32, a: f32,
    layer: f32,
    nx: f32, ny: f32, nz: f32,
    env: f32,
};

// one blade (32 bytes): the root and height (after the press); the baked colour (rgb8, the
// per-blade brightness folded in) with the body (2 bits) and the width scale (6 bits,
// BLADE_WIDTH_MIN..+VARY) in the top byte; the texel (rgb8) with the facing angle (0..255 = one
// turn; its low bit the bend's side) in the top byte; the normal's x and z as 16-bit snorm (y is
// up, recovered); the lean at the tip (world xz, wind + press, 16-bit snorm of BLADE_LEAN_MAX).
// The width is not stored: vs_blade takes the body's BladeWidth x the scale x the hand-off.
struct Blade {
    x: f32, y: f32, z: f32, h: f32,
    color: u32,
    texel: u32,
    normal: u32,
    lean: u32,
};

// wgpu::util::DrawIndirectArgs
struct BladeArgs {
    vertex_count: u32,
    instance_count: atomic<u32>,
    first_vertex: u32,
    first_instance: u32,
};

@group(1) @binding(1) var<storage, read> blade_verts: array<GVert>;
@group(1) @binding(2) var<storage, read_write> blade_out: array<Blade>;
@group(1) @binding(3) var<storage, read_write> blade_args: BladeArgs;
// the draw's view of the records
@group(3) @binding(3) var<storage, read> blades: array<Blade>;

fn gv_pos(v: GVert) -> vec3<f32> { return vec3<f32>(v.px, v.py, v.pz); }
fn gv_normal(v: GVert) -> vec3<f32> { return vec3<f32>(v.nx, v.ny, v.nz); }
fn gv_color(v: GVert) -> vec3<f32> { return vec3<f32>(v.r, v.g, v.b); }

// both points on the far side of one clip plane: nothing between them is visible
fn blade_offscreen(a: vec4<f32>, b: vec4<f32>) -> bool {
    return (a.x < -a.w && b.x < -b.w) || (a.x > a.w && b.x > b.w) || (a.y < -a.w && b.y < -b.w) || (a.y > a.w && b.y > b.w) || (a.z < 0.0 && b.z < 0.0) || (a.z > a.w && b.z > b.w);
}

// all eight corners of the box on the far side of one clip plane
fn blade_box_offscreen(lo: vec3<f32>, hi: vec3<f32>) -> bool {
    var out = vec3<u32>(0u);
    var below = vec3<u32>(0u);
    for (var k = 0u; k < 8u; k++) {
        let c = g.view_proj * vec4<f32>(select(lo, hi, vec3<bool>((k & 1u) != 0u, (k & 2u) != 0u, (k & 4u) != 0u)), 1.0);
        out += vec3<u32>(vec3<bool>(c.x > c.w, c.y > c.w, c.z > c.w));
        below += vec3<u32>(vec3<bool>(c.x < -c.w, c.y < -c.w, c.z < 0.0));
    }
    return any(out == vec3<u32>(8u)) || any(below == vec3<u32>(8u));
}

// One workgroup row per grass triangle (x + y * BLADE_DISPATCH_W), BLADE_SLICES workgroups in
// z that take every n-th cell. The triangle is skipped when its box is out of BladeReach of the
// camera or wholly behind it; then each thread walks its share of the cells of the grid
// covering the triangle's box clipped to the reach box, keeps the cells whose jittered point
// lies in the triangle (XZ barycentrics), interpolates the vertex there, and appends a record
// when the blade survives the distance thinning, the edge fade and the frustum.
@compute @workgroup_size(BLADE_GROUP)
fn cs_blades(@builtin(workgroup_id) wg: vec3<u32>, @builtin(local_invocation_index) lid: u32) {
    let tri = wg.x + wg.y * BLADE_DISPATCH_W;
    let n_tris = arrayLength(&blade_verts) / 3u;
    if (tri >= n_tris) {
        return;
    }
    let v0 = blade_verts[3u * tri];
    let v1 = blade_verts[3u * tri + 1u];
    let v2 = blade_verts[3u * tri + 2u];
    let p0 = gv_pos(v0);
    let p1 = gv_pos(v1);
    let p2 = gv_pos(v2);
    let b = u32(clamp(v0.env - GRASS_ENV, 0.0, 3.0));
    let r0 = grass_row(b, 0u);
    let r1 = grass_row(b, 1u);
    let r4 = grass_row(b, 4u);
    let density = max(r4.x, 1.0);
    let reach = max(r4.z, 1.0);
    let cam = g.cam_pos.xyz;
    // the triangle's box against the reach box round the camera (the blade height on top)
    let lo = min(p0, min(p1, p2)) - vec3<f32>(0.0, r0.x, 0.0);
    let hi = max(p0, max(p1, p2)) + vec3<f32>(0.0, r0.x, 0.0);
    if (any(cam < lo - reach) || any(cam > hi + reach)) {
        return;
    }
    // wholly outside the view: the box widened by the most a blade leans and bends
    let m = vec3<f32>(r0.x + BLADE_LEAN_MAX, 0.0, r0.x + BLADE_LEAN_MAX);
    if (blade_box_offscreen(lo - m, hi + m)) {
        return;
    }
    let a2 = p0.xz;
    let b2 = p1.xz;
    let cc = p2.xz;
    let area = (b2.x - a2.x) * (cc.y - a2.y) - (b2.y - a2.y) * (cc.x - a2.x);
    if (abs(area) < 1e-6) {
        return;
    }
    // the cell range: the triangle's XZ box clipped to the reach box
    let cmin = max(lo.xz, cam.xz - reach);
    let cmax = min(hi.xz, cam.xz + reach);
    if (any(cmin > cmax)) {
        return;
    }
    let i0 = vec2<i32>(floor(cmin * density));
    let i1 = vec2<i32>(floor(cmax * density));
    let nx = u32(i1.x - i0.x + 1);
    let cells = nx * u32(i1.y - i0.y + 1);
    let n0 = gv_normal(v0);
    let n1 = gv_normal(v1);
    let n2 = gv_normal(v2);
    let col0 = gv_color(v0);
    let col1 = gv_color(v1);
    let col2 = gv_color(v2);
    let layer = max(i32(v0.layer + 0.5), 0);
    for (var k = lid + BLADE_GROUP * wg.z; k < cells; k += BLADE_GROUP * BLADE_SLICES) {
        let ci = vec2<i32>(i0.x + i32(k % nx), i0.y + i32(k / nx));
        // the shells' hashes of the cell (fs_grass: jitter, brightness, height), so the cones
        // and blades share their roots and heights; a second set for the blade alone
        let hc = grass_hash4(ci, 0u);
        let hb = grass_hash4(ci, 1u);
        let h1 = hc.x;
        let h2 = hc.y;
        let h3 = hc.z;
        let h4 = hb.x;
        let h5 = hb.y;
        // the jittered point of the cell (the shells' GRASS_JITTER)
        let p = (vec2<f32>(ci) + 0.5 + (vec2<f32>(h1, h2) - 0.5) * GRASS_JITTER) / density;
        let w0 = ((b2.x - p.x) * (cc.y - p.y) - (b2.y - p.y) * (cc.x - p.x)) / area;
        let w1 = ((cc.x - p.x) * (a2.y - p.y) - (cc.y - p.y) * (a2.x - p.x)) / area;
        let w2 = 1.0 - w0 - w1;
        if (w0 < 0.0 || w1 < 0.0 || w2 < 0.0) {
            continue;
        }
        let root = w0 * p0 + w1 * p1 + w2 * p2;
        let d = distance(cam, root);
        if (d > reach) {
            continue;
        }
        // the hand-off: fewer blades from BLADE_HAND x reach out, the kept ones wider
        let keep = 1.0 - smoothstep(reach * BLADE_HAND, reach, d);
        if (h4 >= keep) {
            continue;
        }
        let n = normalize(w0 * n0 + w1 * n1 + w2 * n2);
        let edge = w0 * v0.a + w1 * v1.a + w2 * v2.a;
        let fade = grass_fade(root, edge, r1.w);
        // the cars press the blade: shorter under the hull, leaning away from it; the lasting
        // trail flattens too (grass_press, once per blade)
        let press = grass_press(root);
        // the height: the shells' distance LOD (continuous at the hand-off), the random
        // per-cell height, the edge fade and the press (a flattened blade lies, it does not go)
        let lod = clamp(1.0 - d / max(r1.x, 1.0), 0.0, 1.0);
        let height = r0.x * (GRASS_HEIGHT_MIN + (1.0 - GRASS_HEIGHT_MIN) * h3) * lod * fade * mix(1.0, GRASS_PRESS_LEFT, press.z);
        if (height < BLADE_MIN_HEIGHT) {
            continue;
        }
        // wind: the shells' gust at the root, with a phase of this blade's own
        let gust = grass_noise(root.xz * GRASS_GUST_SCALE + g.water_t.x * GRASS_GUST_SPEED);
        let wind = GRASS_WIND_DIR * (r1.y * (0.2 + 0.8 * gust) * (0.7 + 0.6 * hb.w));
        let lean = wind + press.xy;
        let cr = g.view_proj * vec4<f32>(root, 1.0);
        let ct = g.view_proj * vec4<f32>(root + n * height + vec3<f32>(lean.x, 0.0, lean.y), 1.0);
        if (blade_offscreen(cr, ct)) {
            continue;
        }
        let uv = w0 * vec2<f32>(v0.u, v0.v) + w1 * vec2<f32>(v1.u, v1.v) + w2 * vec2<f32>(v2.u, v2.v);
        let texel = textureSampleLevel(tex, samp, uv, layer, BLADE_TEXEL_LOD).rgb;
        // the baked colour with the shells' per-cell brightness (fs_grass GRASS_VARY by h2)
        let color = min((w0 * col0 + w1 * col1 + w2 * col2) * g.gain.rgb, vec3<f32>(1.0)) * (1.0 - GRASS_VARY * 0.5 + GRASS_VARY * h2);
        let idx = atomicAdd(&blade_args.instance_count, 1u);
        if (idx < BLADE_MAX) {
            var o: Blade;
            o.x = root.x;
            o.y = root.y;
            o.z = root.z;
            o.h = height;
            o.color = (pack4x8unorm(vec4<f32>(color, 0.0)) & 0xffffffu) | (b << 24u) | (u32(hb.z * 63.0) << 26u);
            o.texel = pack4x8unorm(vec4<f32>(texel, h5));
            o.normal = pack2x16snorm(n.xz);
            o.lean = pack2x16snorm(lean / BLADE_LEAN_MAX);
            blade_out[idx] = o;
        }
    }
}

struct BladeOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) color: vec3<f32>,
    @location(1) texel: vec3<f32>,
    @location(2) wpos: vec3<f32>,
    @location(3) wnormal: vec3<f32>,
    // root 0 .. tip 1
    @location(4) t: f32,
    @location(5) @interpolate(flat) body: u32,
    // across the strip, -1 .. 1
    @location(6) side: f32,
    // which side faces the sun, -1 / 1
    @location(7) @interpolate(flat) sun: f32,
};

// A strip of 2 x (BLADE_SEGMENTS + 1) vertices per record along a quadratic Bezier from the
// root: the tip is tipped over by BladeBend (x 0.75..1.25 per blade, to a random side) and moved
// by the lean (wind + press), the control point halfway up takes BLADE_BEND_CP of that offset.
// The strip faces a random direction mixed BLADE_FACE_CAM toward the camera's right (the first
// row of the view matrix), so no blade is edge-on; the width is the body's BladeWidth x the
// blade's scale x the hand-off widening, no less than BLADE_MIN_PX pixels at its depth
// (grass_n.w: pixels per metre at 1 m), tapering to the tip. Records past the cap (the
// placement counted more than it could write) collapse off screen.
@vertex
fn vs_blade(@builtin(vertex_index) vi: u32, @builtin(instance_index) ii: u32) -> BladeOut {
    var o: BladeOut;
    if (ii >= BLADE_MAX) {
        o.clip = vec4<f32>(0.0, 0.0, 2.0, 1.0);
        return o;
    }
    let r = blades[ii];
    let root = vec3<f32>(r.x, r.y, r.z);
    let side = select(-1.0, 1.0, (vi & 1u) == 1u);
    let body = (r.color >> 24u) & 3u;
    let ws = BLADE_WIDTH_MIN + BLADE_WIDTH_VARY * f32(r.color >> 26u) / 63.0;
    let nxz = unpack2x16snorm(r.normal);
    let n = vec3<f32>(nxz.x, sqrt(max(1.0 - dot(nxz, nxz), 0.0)), nxz.y);
    let tx = unpack4x8unorm(r.texel);
    let fbits = r.texel >> 24u;
    let facing = f32(fbits) * (6.2831853 / 256.0);
    let bsign = select(-1.0, 1.0, (fbits & 1u) == 1u);
    let lean = unpack2x16snorm(r.lean) * BLADE_LEAN_MAX;
    let r4 = grass_row(body, 4u);
    // the width: the hand-off widening (as the placement thins), the pixel floor at the root's depth
    let reach = max(r4.z, 1.0);
    let dist = distance(g.cam_pos.xyz, root);
    let keep = 1.0 - smoothstep(reach * BLADE_HAND, reach, dist);
    // past BLADE_LOD_FAR of the reach one segment fewer: the last vertex pair repeats the one
    // before, so its triangles have no area and are dropped before shading
    let segs = select(BLADE_SEGMENTS, BLADE_SEGMENTS - 1u, dist > reach * BLADE_LOD_FAR);
    let t = f32(min(vi / 2u, segs)) / f32(segs);
    let cr = g.view_proj * vec4<f32>(root, 1.0);
    var width = r4.y * ws * (2.0 - keep);
    if (g.grass_n.w > 0.0) {
        width = max(width, BLADE_MIN_PX * max(cr.w, 0.0) / g.grass_n.w);
    }
    // facing: random, turned toward the camera's right (flipped first so the mix never cancels)
    var rnd = vec3<f32>(cos(facing), 0.0, sin(facing));
    let right = normalize(vec3<f32>(g.view[0].x, 0.0, g.view[2].x) + vec3<f32>(1e-4, 0.0, 0.0));
    rnd = rnd * sign(dot(rnd, right) + 1e-4);
    let across = normalize(mix(rnd, right, BLADE_FACE_CAM));
    // the bend over the strip's face, the lean in the world
    let fwd = normalize(cross(n, across)) * bsign;
    let bend = clamp(r4.w * (1.25 - 0.5 * (ws - BLADE_WIDTH_MIN) / BLADE_WIDTH_VARY) * BLADE_BEND_MAX, 0.0, 0.95);
    let up = n * (r.h * sqrt(1.0 - bend * bend));
    let tip = up + fwd * (r.h * bend) + vec3<f32>(lean.x, 0.0, lean.y);
    let cp = n * (r.h * 0.5) + (tip - up) * BLADE_BEND_CP;
    let p = root + cp * (2.0 * (1.0 - t) * t) + tip * (t * t) + across * (side * width * 0.5 * (1.0 - t));
    o.clip = g.view_proj * vec4<f32>(p, 1.0);
    o.color = unpack4x8unorm(r.color).rgb;
    o.texel = tx.rgb;
    o.wpos = p;
    o.wnormal = n;
    o.t = t;
    o.body = body;
    o.side = side;
    o.sun = sign(dot(g.sun_dir.xz, across.xz) + 1e-4);
    return o;
}

// The blade's colour: the shells' root-to-tip shade and tint (fs_grass) with the sun side of
// the strip lighter, then the world terms of a grass pixel.
@fragment
fn fs_blade(i: BladeOut) -> @location(0) vec4<f32> {
    let r1 = grass_row(i.body, 1u);
    let r2 = grass_row(i.body, 2u);
    let r3 = grass_row(i.body, 3u);
    let shade = mix(r1.z, 1.0, i.t) * (1.0 + BLADE_SIDE * i.side * i.sun);
    let tint = mix(r2.rgb, r3.rgb, r2.w * i.t);
    let c = i.color * shade * tint;
    return vec4<f32>(grass_light(c, i.texel, i.wpos, i.wnormal), 1.0);
}
