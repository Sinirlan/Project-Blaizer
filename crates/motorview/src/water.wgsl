// ---------------------------------------------------------------------------------------------
// REMASTER water (procgen/water.rs; re-notes/12-world-fill.md 12.8, 16-water.md) [design choice]:
// the original's flat water texture replaced by a procedural surface. Appended to shader.wgsl
// into a module of its own (render.rs `WaterGpu`); DEPTH_TEX is replaced by texture_depth_2d or
// texture_depth_multisampled_2d (MSAA) at load. `g.water` holds 7 vec4 per body:
//   0: deep rgb, clarity   1: shallow rgb, wave height   2: wave length, speed, direction (rad), chop
//   3: style (1 = ocean), foam, reflect, shore foam   4: shore rgb, visibility
//   5: extinction, refraction, tiling, normals   6: foam height, caustics
// The ocean is four Gerstner swells (heights and horizontal chop in the vertex stage, so crests
// peak), calmed towards the shore; still water is flat. Everything is a function of the world
// position and time, so shared vertices move together. The rest follows tuxalin/water-shader (MIT):
// the colour under the surface (water/depth.cginc, after Wojciech Toman's "Rendering Water as a
// Post-process Effect": the scene behind the water, a copy taken after the opaque pass, seen through
// a colour that goes shore -> shallow -> deep with the water's depth from the depth buffer), the
// scrolled normal maps (water/displacement.cginc), Fresnel and sun specular (water/radiance.cginc),
// shore and wave foam (water/foam.cginc) and the final mix (water.fx `frag`).
// ---------------------------------------------------------------------------------------------

// the opaque scene as drawn before the water (linear in HDR), its depth, and the detail maps
@group(3) @binding(0) var w_scene: texture_2d<f32>;
@group(3) @binding(1) var w_depth: DEPTH_TEX;
@group(3) @binding(2) var w_normal: texture_2d<f32>;
@group(3) @binding(3) var w_foam: texture_2d<f32>;
@group(3) @binding(4) var w_clamp: sampler;
// the volumetric fog volume (volfog.rs): (in-scatter, transmittance) up to the far side of each slice;
// only read in a volumetric view (flag 2048), where `g.fog3.w` is its far distance
@group(3) @binding(5) var w_fog: texture_3d<f32>;
// volfog.rs NEAR and SLICES
const VOL_NEAR: f32 = 1.0;
const VOL_SLICES: f32 = 64.0;

const WATER_ROWS: u32 = 7u;
// "nothing behind the surface" (the depth buffer's far value: the hidden texture was the floor)
const WATER_BOTTOMLESS: f32 = 1e6;

fn water_row(b: u32, r: u32) -> vec4<f32> {
    return g.water[b * WATER_ROWS + r];
}

struct WaterSample {
    // height, slope in x and z, horizontal chop displacement
    h: f32,
    dx: f32,
    dz: f32,
    chop: vec2<f32>,
}

fn water_body(env: f32) -> u32 {
    return u32(clamp(env - 15.5, 0.0, 3.0));
}

// The ocean's swells at `p`; still water is flat (its detail is the normal maps). `calm` scales
// them: 0 at the shore, 1 out at sea (the vertex's `color.x`, procgen/water.rs `calm_distance`),
// so no crest rises over a rim.
fn water_eval(p: vec2<f32>, b: u32, t: f32, calm: f32) -> WaterSample {
    let w1 = water_row(b, 1u);
    let w2 = water_row(b, 2u);
    var s: WaterSample;
    s.h = 0.0; s.dx = 0.0; s.dz = 0.0; s.chop = vec2<f32>(0.0);
    if (water_row(b, 3u).x < 0.5) {
        return s;
    }
    let height = w1.w;
    let len = max(w2.x, 0.05);
    // four swells fanned round the wind direction, shorter ones weaker and slower
    for (var i = 0u; i < 4u; i++) {
        let fi = f32(i);
        let ang = w2.z + (fi - 1.0) * 0.55 + sin(fi * 2.3) * 0.2;
        let d = vec2<f32>(cos(ang), sin(ang));
        let l = len * pow(0.55, fi);
        let a = 0.5 * height * pow(0.6, fi) * calm;
        let k = 6.2831853 / l;
        let ph = k * (dot(d, p) - w2.y * pow(0.55, fi * 0.5) * t);
        s.h += a * sin(ph);
        let sl = a * k * cos(ph);
        s.dx += sl * d.x;
        s.dz += sl * d.y;
        s.chop += d * (w2.w * a * cos(ph));
    }
    return s;
}

fn water_calm(shore: f32) -> f32 {
    return smoothstep(0.0, 1.0, shore);
}

@vertex
fn vs_water(v: VsIn) -> VsOut {
    var o: VsOut;
    let b = water_body(v.env);
    var p = v.pos;
    if (water_row(b, 3u).x > 0.5) {
        let s = water_eval(p.xz, b, g.water_t.x, water_calm(v.color.x));
        p = p + vec3<f32>(s.chop.x, s.h, s.chop.y);
    }
    o.clip = g.view_proj * vec4<f32>(p, 1.0);
    o.uv = v.uv;
    o.color = v.color;
    o.layer = -1.0;
    o.fog = 0.0;
    o.env_uv = vec2<f32>(0.0);
    o.env = v.env;
    o.wpos = p;
    o.wnormal = vec3<f32>(0.0, 1.0, 0.0);
    o.albedo = vec3<f32>(1.0);
    return o;
}

// 2D simplex noise, -1..1 (Ashima Arts / Ian McEwan, MIT; tuxalin snoise.cginc)
fn water_mod289(x: vec3<f32>) -> vec3<f32> {
    return x - floor(x * (1.0 / 289.0)) * 289.0;
}

fn water_permute(x: vec3<f32>) -> vec3<f32> {
    return water_mod289((x * 34.0 + 1.0) * x);
}

fn water_snoise(v: vec2<f32>) -> f32 {
    let c = vec4<f32>(0.211324865405187, 0.366025403784439, -0.577350269189626, 0.024390243902439);
    var i = floor(v + dot(v, c.yy));
    let x0 = v - i + dot(i, c.xx);
    let i1 = select(vec2<f32>(0.0, 1.0), vec2<f32>(1.0, 0.0), x0.x > x0.y);
    let x1 = x0 + c.xx - i1;
    let x2 = x0 + c.zz;
    i = i - floor(i * (1.0 / 289.0)) * 289.0;
    let p = water_permute(water_permute(i.y + vec3<f32>(0.0, i1.y, 1.0)) + i.x + vec3<f32>(0.0, i1.x, 1.0));
    var m = max(0.5 - vec3<f32>(dot(x0, x0), dot(x1, x1), dot(x2, x2)), vec3<f32>(0.0));
    m = m * m;
    m = m * m;
    let x = 2.0 * fract(p * c.www) - 1.0;
    let h = abs(x) - 0.5;
    let a0 = x - floor(x + 0.5);
    m = m * (1.79284291400159 - 0.85373472095314 * (a0 * a0 + h * h));
    let gr = vec3<f32>(a0.x * x0.x + h.x * x0.y, a0.y * x1.x + h.y * x1.y, a0.z * x2.x + h.z * x2.y);
    return 130.0 * dot(m, gr);
}

// The wind's direction in the water plane (row 2's angle, the swells' too)
fn water_wind(b: u32) -> vec2<f32> {
    let a = water_row(b, 2u).z;
    return vec2<f32>(cos(a), sin(a));
}

// tuxalin `ComputeNormal` (displacement.cginc): four samples of the normal map at falling scales,
// scrolled along the wind at falling rates, weighted 3 : 2 : 2 : 10. Returns the world-space
// normal (y up) of the map alone. [design choice] The scroll is Speed m/s times 0.3 / 0.2 / 0.1 /
// 0.05 instead of tuxalin's timer (its time unit is not stated).
fn water_map_normal(p: vec2<f32>, b: u32, t: f32) -> vec3<f32> {
    let w5 = water_row(b, 5u);
    let uv = p * 0.05 * max(w5.z, 0.01);
    let drift = water_wind(b) * water_row(b, 2u).y * t * 0.05 * max(w5.z, 0.01);
    let scale = vec4<f32>(1.6, 0.8, 0.5, 0.3);
    let rate = vec4<f32>(0.3, 0.2, 0.1, 0.05);
    let weight = vec4<f32>(3.0, 2.0, 2.0, 10.0) / 17.0;
    var slope = vec2<f32>(0.0);
    for (var i = 0u; i < 4u; i++) {
        let q = uv * scale[i] + drift * rate[i] + vec2<f32>(f32(i) * 0.37, f32(i) * 0.61);
        let tn = textureSample(w_normal, samp_hq, q).rgb * 2.0 - 1.0;
        slope += tn.xy / max(tn.z, 0.2) * weight[i];
    }
    return normalize(vec3<f32>(slope.x, 1.0, -slope.y));
}

// tuxalin `FresnelValue` (radiance.cginc): R0 0.3, refraction strength 0.01, times Reflect
fn water_fresnel(n: vec3<f32>, v: vec3<f32>, reflect: f32) -> f32 {
    let angle = 1.0 - clamp(dot(n, v), 0.0, 1.0);
    var f = angle * angle;
    f = f * f * angle;
    return clamp(f * (1.0 - 0.3) + 0.3 - 0.01, 0.0, 1.0) * reflect;
}

// tuxalin `ReflectedRadiance` (radiance.cginc, the default branch): shininess 0.5, specular
// values (12, 768, 0.15); `light` = the sun's colour
fn water_specular(n: vec3<f32>, v: vec3<f32>, l: vec3<f32>, light: vec3<f32>, fresnel: f32) -> vec3<f32> {
    let shininess = 0.5;
    let mirror = reflect(-v, n);
    let d = clamp(dot(mirror, l) * 0.5 + 0.5, 0.0, 1.0);
    var spec = (1.0 - fresnel) * clamp(l.y, 0.0, 1.0) * pow(d, 768.0) * (shininess * 0.15 + 0.2) * light;
    spec += spec * 12.0 * clamp(shininess - 0.05, 0.0, 1.0) * light;
    return spec;
}

// tuxalin `FoamValue` (foam.cginc): a band of shore foam (G of the foam map) where the water is
// shallower than the jittered ranges, the deep foam (R) just beyond it, and foam on swells higher
// than FoamHeight. `p` is the surface, `depth` its water depth, `h` the swell height there.
// [design choice] Times: drift = Speed x 0.05 uv/s along the wind (tuxalin's timer has no stated unit).
fn water_foam(p: vec2<f32>, v: vec3<f32>, b: u32, t: f32, depth: f32, h: f32) -> f32 {
    let w3 = water_row(b, 3u);
    let wind = water_wind(b);
    let speed = water_row(b, 2u).y;
    let pos = (p + v.xz * 0.1) * 0.5;
    let drift = wind * speed * t * 0.05;
    let s1 = sin(t * 0.1 + p.x * 0.05);
    let s2 = sin(t * 0.1 + p.y * 0.05);
    var tc = pos + drift + s1 * 0.05;
    var tc2 = (pos + drift * 1.5 + s2 * 0.05) * -0.5;
    let tc3 = tc * 2.0;
    let tc4 = (pos - drift * 0.45 + s2 * 0.05) * -0.5 * 2.0;
    tc = tc * 0.5;
    tc2 = tc2 * 0.5;
    // the ranges wander with noise that drifts with the wind (tuxalin `_FoamNoise` 0.1 / 0.3)
    let r0 = max(w3.w, 0.0);
    var ranges = vec2<f32>(r0, r0 * 1.5);
    ranges.x += water_snoise(p + wind * t * 0.1 * speed) * 0.1 * r0;
    ranges.y += water_snoise(p + wind * t * 0.3 * speed) * 0.3 * r0;
    ranges = clamp(ranges, vec2<f32>(0.0), vec2<f32>(10.0));
    let edge = max(ranges.x, ranges.y);
    let deep = mix(textureSample(w_foam, samp_hq, tc).r + textureSample(w_foam, samp_hq, tc2).r * 0.5, 0.0, smoothstep(ranges.x, edge, depth));
    var foam = mix(textureSample(w_foam, samp_hq, tc3 * 0.25).g * 0.75 + textureSample(w_foam, samp_hq, tc4).g * 1.5, deep, smoothstep(0.0, ranges.x + 1e-4, depth));
    // high waves foam
    let fh = water_row(b, 6u).x;
    let amp = max(water_row(b, 1u).w, 1e-3);
    let amount = clamp((h - fh) / amp, 0.0, 1.0) * 0.25;
    foam += (textureSample(w_foam, samp_hq, tc3).g + textureSample(w_foam, samp_hq, tc4).g * 0.5) * amount;
    // no shore foam without a known depth (bottomless water, or the views without the depth)
    return foam * w3.y;
}

// view-space distance of a depth-buffer value (the views' perspective, near/far in g.water_t)
fn water_view_z(d: f32) -> f32 {
    let n = g.water_t.z;
    let f = g.water_t.w;
    return n * f / (f - d * (f - n));
}

// view distance of the scene behind pixel `px`; WATER_BOTTOMLESS where nothing was drawn
fn water_floor_z(px: vec2<i32>) -> f32 {
    let size = vec2<i32>(textureDimensions(w_depth));
    let d = textureLoad(w_depth, clamp(px, vec2<i32>(0), size - 1), 0);
    return select(water_view_z(d), WATER_BOTTOMLESS, d >= 1.0);
}

// REMASTER screen-space reflections [design choice] (16-water "SSR"): the reflected ray marched in
// world space from the surface, each point projected into the scene copy and tested against the
// depth buffer; steps grow by SSR_GROWTH from a first step that grows with the distance, a hit is
// refined by halving. Returns the scene's colour there (gamma, like the refraction) and how far
// to trust it: faded at the screen's edges, at the end of the reach, and for rays that head back
// towards the camera (what they would meet is behind it or behind the surface).
const SSR_STEPS: u32 = 32u;
const SSR_REFINE: u32 = 5u;
const SSR_GROWTH: f32 = 1.1;
const SSR_REACH: f32 = 250.0;

// screen uv (xy) and view distance (w) of world point `q`; w < 0 in front of the near plane
fn water_screen(q: vec3<f32>) -> vec4<f32> {
    let c = g.view_proj * vec4<f32>(q, 1.0);
    if (c.w <= g.water_t.z) {
        return vec4<f32>(0.0, 0.0, 0.0, -1.0);
    }
    let ndc = c.xyz / c.w;
    return vec4<f32>(ndc.x * 0.5 + 0.5, 0.5 - ndc.y * 0.5, ndc.z, water_view_z(ndc.z));
}

fn water_ssr(p: vec3<f32>, r_in: vec3<f32>, dist: f32) -> vec4<f32> {
    // a ray into the water would find the bed behind the surface: the ripples' downward rays
    // (grazing views) are taken as level, like the sky's `max(r.y, 0)`
    let r = normalize(vec3<f32>(r_in.x, max(r_in.y, 0.01), r_in.z));
    let facing = smoothstep(-0.5, 0.0, dot(r, normalize(p - g.cam_pos.xyz)));
    if (facing <= 0.0) {
        return vec4<f32>(0.0);
    }
    let size = vec2<f32>(textureDimensions(w_scene));
    var step = 0.4 + dist * 0.02;
    var t = 0.0;
    var prev = 0.0;
    var hit = -1.0;
    for (var i = 0u; i < SSR_STEPS; i++) {
        prev = t;
        t += step;
        let s = water_screen(p + r * t);
        if (t > SSR_REACH || s.w < 0.0 || s.z >= 1.0 || any(s.xy < vec2<f32>(0.0)) || any(s.xy > vec2<f32>(1.0))) {
            break;
        }
        // behind the scene: a hit when only just (else the ray passed behind something in front)
        let behind = s.w - water_floor_z(vec2<i32>(s.xy * size));
        if (behind > 0.0 && behind < max(0.5, 1.5 * step)) {
            hit = t;
            break;
        }
        step *= SSR_GROWTH;
    }
    if (hit < 0.0) {
        return vec4<f32>(0.0);
    }
    var lo = prev;
    var hi = hit;
    for (var i = 0u; i < SSR_REFINE; i++) {
        let m = (lo + hi) * 0.5;
        let s = water_screen(p + r * m);
        if (s.w > water_floor_z(vec2<i32>(s.xy * size))) {
            hi = m;
        } else {
            lo = m;
        }
    }
    let s = water_screen(p + r * hi);
    let e = min(s.xy, 1.0 - s.xy);
    let edge = smoothstep(0.0, 0.08, min(e.x, e.y));
    let reach = 1.0 - smoothstep(0.6 * SSR_REACH, SSR_REACH, hi);
    let c = textureSampleLevel(w_scene, w_clamp, s.xy, 0.0).rgb;
    return vec4<f32>(select(c, pow(max(c, vec3<f32>(0.0)), vec3<f32>(1.0 / GAMMA)), linear()), facing * edge * reach);
}

// tuxalin `DepthRefraction`: `refr` is what lies behind the surface, `depth_h` the water's vertical
// depth there, `depth_v` the length of water the eye looks through. The colours are lit already.
fn water_depth_colour(b: u32, refr: vec3<f32>, depth_h: f32, depth_v: f32, shore: vec3<f32>, shallow: vec3<f32>, deep: vec3<f32>) -> vec3<f32> {
    let clarity = 1.0 - 0.9 * water_row(b, 0u).w;
    let vis = max(water_row(b, 4u).w, 0.1);
    // red goes first, blue last (tuxalin's horizontal extinction per channel)
    let ext = max(water_row(b, 5u).x, 0.1) * vec3<f32>(0.4, 1.0, 1.6);
    let acc = depth_v * clarity;
    var e = clamp(acc / (2.5 * vis), 0.0, 1.0);
    e = e * ((1.0 - e) * e * e + 1.0);
    // tuxalin: the shore colour turns into the surface colour over twice the foam's outer range
    let shore_range = max(3.0 * water_row(b, 3u).w, 0.5);
    let surface = mix(shore, shallow, clamp(depth_h / shore_range, 0.0, 1.0));
    let k = clamp(vec3<f32>(depth_h) / ext, vec3<f32>(0.0), vec3<f32>(1.0));
    let tint = mix(surface, deep, k);
    var c = mix(refr, surface * tint, clamp(acc / vis, 0.0, 1.0));
    c = mix(c, deep, e);
    // APPROX: tuxalin ends on deep * tint, which takes our gamma-space colours to black; deep alone
    return mix(c, deep, k);
}

// REMASTER caustics [design choice]: the moving net of light the ripples focus on a shallow
// bottom. Two scrolled layers of animated cell noise (F2 - F1 of a Voronoi, thin bright lines
// where two cells meet), ~1.8 m cells, evaluated where the sunlight entered the surface.
fn water_hash2(c: vec2<i32>) -> vec2<f32> {
    var h = vec2<u32>(bitcast<u32>(c.x), bitcast<u32>(c.y)) * vec2<u32>(1597334677u, 3812015801u);
    h = (h.x ^ h.y) * vec2<u32>(2246822519u, 3266489917u);
    h = (h ^ (h >> vec2<u32>(15u))) * vec2<u32>(668265263u, 374761393u);
    h = h ^ (h >> vec2<u32>(13u));
    return vec2<f32>(h) / 4294967295.0;
}

fn water_voronoi_line(p: vec2<f32>, t: f32) -> f32 {
    let c = vec2<i32>(floor(p));
    let f = fract(p);
    var f1 = 8.0;
    var f2 = 8.0;
    for (var j = -1; j <= 1; j++) {
        for (var i = -1; i <= 1; i++) {
            let o = vec2<i32>(i, j);
            let h = water_hash2(c + o);
            let q = vec2<f32>(o) + 0.5 + 0.4 * sin(t * (0.6 + h.yx * 0.8) + h * 6.2831853);
            let d = distance(f, q);
            if (d < f1) {
                f2 = f1;
                f1 = d;
            } else if (d < f2) {
                f2 = d;
            }
        }
    }
    let e = 1.0 - smoothstep(0.0, 0.22, f2 - f1);
    return e * e;
}

// `entry` = where the sun's ray met the surface; `depth` = water above the bottom point
fn water_caustics(entry: vec2<f32>, depth: f32, dist: f32, t: f32, strength: f32) -> f32 {
    let a = water_voronoi_line(entry / 1.8 + vec2<f32>(0.0, t * 0.05), t * 1.1);
    let b = water_voronoi_line(entry / 1.3 + vec2<f32>(7.3, 4.1) - vec2<f32>(t * 0.04, 0.0), -t * 0.9 + 3.0);
    // brightest where the surface just covers the bottom, none once the water is too deep to see
    // through, and none far away where the net would only shimmer
    let fade = smoothstep(0.05, 0.4, depth) * (1.0 - smoothstep(1.5, 4.0, depth)) * (1.0 - smoothstep(35.0, 70.0, dist));
    return (a + b) * 0.5 * strength * fade;
}

@fragment
fn fs_water(i: VsOut) -> @location(0) vec4<f32> {
    let b = water_body(i.env);
    let w0 = water_row(b, 0u);
    let w1 = water_row(b, 1u);
    let w3 = water_row(b, 3u);
    let w4 = water_row(b, 4u);
    let w5 = water_row(b, 5u);
    let w6 = water_row(b, 6u);
    let t = g.water_t.x;
    let dist = distance(i.wpos, g.cam_pos.xyz);
    let s = water_eval(i.wpos.xz, b, t, water_calm(i.color.x));
    // the swells' normal (slopes along the world axes), then the normal map over it (whiteout
    // blend), faded out with distance where the map would only shimmer
    let swell = normalize(vec3<f32>(-s.dx, 1.0, -s.dz));
    let detail = mix(vec3<f32>(0.0, 1.0, 0.0), water_map_normal(i.wpos.xz, b, t), w5.w / (1.0 + dist / 120.0));
    let n = normalize(vec3<f32>(swell.x + detail.x, swell.y * detail.y, swell.z + detail.z));
    let v = normalize(g.cam_pos.xyz - i.wpos);
    let sun = normalize(g.sun_dir.xyz);
    let r = reflect(-v, n);
    // the sky mirrored in the surface: the track's sky gradient, or the HDRI dome (REMASTER sky)
    var sky = mix(g.sky_bottom.rgb, g.sky_top.rgb, smoothstep(0.0, 0.7, max(r.y, 0.0)));
    if (g.fog3.z > 0.0) {
        sky = dome_gamma(dome_radiance(r, 1.5));
    }
    // (a volumetric view: the volume only covers the way to the surface, so the reflected sky is fogged
    // by its direction here)
    sky = fog_sky_dir(sky, r);
    // REMASTER the scene mirrored over the sky where the ray finds it in the copy. APPROX: the copy
    // is fogged by the hit's distance from the camera, not by the reflected path; the water does
    // not mirror itself (the copy is taken before it)
    if (g.water_t.y > 1.5) {
        let m = water_ssr(i.wpos, r, dist);
        sky = mix(sky, m.rgb, m.a);
    }
    let light = clamp(g.ambient.rgb + g.sun.rgb * max(dot(n, sun), 0.0), vec3<f32>(0.15), vec3<f32>(1.0));
    // the bottom through the water: the scene copy, bent by the surface's slope where the water is
    // deep enough (not where something stands in front of the water: then straight through)
    var refr = w0.rgb * light;
    var depth_h = WATER_BOTTOMLESS;
    var depth_v = WATER_BOTTOMLESS;
    if (g.water_t.y > 0.5) {
        let size = vec2<f32>(textureDimensions(w_scene));
        let px = vec2<i32>(i.clip.xy);
        let zs = water_view_z(i.clip.z);
        let z0 = water_floor_z(px);
        let bend = w5.y * 0.04 * clamp((z0 - zs) / 2.0, 0.0, 1.0) / (1.0 + dist / 60.0);
        var uv = i.clip.xy / size + n.xz * bend;
        var zb = water_floor_z(vec2<i32>(uv * size));
        if (zb < zs) {
            uv = i.clip.xy / size;
            zb = z0;
        }
        let c = textureSampleLevel(w_scene, w_clamp, uv, 0.0).rgb;
        refr = select(c, pow(max(c, vec3<f32>(0.0)), vec3<f32>(1.0 / GAMMA)), linear());
        if (zb < WATER_BOTTOMLESS) {
            let bottom = g.cam_pos.xyz + (i.wpos - g.cam_pos.xyz) * (zb / zs);
            depth_h = max(i.wpos.y - bottom.y, 0.0);
            depth_v = distance(bottom, i.wpos);
            // the sun's light on the bottom (REMASTER caustics): the ray bends towards the vertical
            // on entering (index 1.33), so even a low sun reaches the bottom a little to its side
            let sun_up = normalize(g.sun_dir.xyz);
            if (w6.y > 0.0 && sun_up.y > 0.0) {
                let sin_t = sqrt(max(1.0 - sun_up.y * sun_up.y, 0.0)) / 1.33;
                let tan_t = sin_t / sqrt(1.0 - sin_t * sin_t);
                let entry = bottom.xz + normalize(sun_up.xz + vec2<f32>(1e-5, 0.0)) * (depth_h * tan_t);
                let lit = smoothstep(0.0, 0.1, sun_up.y) * (0.4 + 0.6 * clamp(dot(g.sun.rgb, vec3<f32>(0.33)), 0.0, 1.0));
                let k = water_caustics(entry, depth_h, dist, t, w6.y) * lit;
                refr = refr * (1.0 + 4.0 * k) + g.sun.rgb * 0.2 * k;
            }
        }
    }
    let shore = w4.rgb * light;
    let water = water_depth_colour(b, refr, depth_h, depth_v, shore, w1.rgb * light, w0.rgb * light);
    // tuxalin water.fx `frag`: Fresnel sky over the depth colour, ambient + diffuse, the sun's
    // specular or the foam, and towards the shore a fade to the plain refraction
    let fresnel = water_fresnel(n, v, w3.z);
    let spec = water_specular(n, v, sun, g.sun.rgb, fresnel);
    let foam = water_foam(i.wpos.xz, v, b, t, depth_h, s.h);
    let ambient = g.ambient.rgb * 0.15 + clamp(dot(n, sun), 0.0, 1.0) * 0.1 * g.sun.rgb;
    var col = cap(ambient + mix(water, sky, fresnel) + max(spec, vec3<f32>(foam) * light));
    let fade = clamp(depth_h * 0.3, 0.0, 1.0);
    let foam_range = max(w3.w, 0.05);
    var pure = mix(refr, sky, fresnel * clamp(depth_h / (foam_range * 0.4), 0.0, 1.0));
    pure = mix(pure, shore, 0.3 * fade);
    col = mix(pure + spec * fade, col, fade);
    if (flag(2048u)) {
        // REMASTER volumetric fog (render.rs `VolFog`; 16-water): the volume was applied before the water over
        // the bed's depth, so the surface looks it up at its own depth, as volfog.wgsl `fs_apply` does for
        // the opaque scene: scene * T * (1 - f) + S * (1 - f) + fog colour * f, in linear light
        let zs = water_view_z(i.clip.z);
        let uv = i.clip.xy / vec2<f32>(textureDimensions(w_depth));
        let w = clamp(log(zs / VOL_NEAR) / log(g.fog3.w / VOL_NEAR), 0.0, 1.0) * VOL_SLICES;
        let tex = textureSampleLevel(w_fog, w_clamp, vec3<f32>(uv, max(w - 0.5, 0.0) / VOL_SLICES), 0.0);
        let vol = mix(vec4<f32>(0.0, 0.0, 0.0, 1.0), tex, clamp(w, 0.0, 1.0));
        let f = fog_ramp(dist);
        let dir = normalize(i.wpos - g.cam_pos.xyz);
        return vec4<f32>(lin(cap(col)) * (vol.a * (1.0 - f)) + vol.rgb * (1.0 - f) + fog_colour(dir) * f, 1.0);
    }
    col = fog_mix(col, i.wpos, fog_at(i.wpos));
    return vec4<f32>(out_rgb(cap(col)), 1.0);
}
