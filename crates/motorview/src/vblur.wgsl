// REMASTER velocity motion blur (render.rs `VelBlur`, 09-remaster 9.17) [design choice, not in
// the original; its MotionBlur is the frame blend of blur.wgsl].
// 1. vs_vel / fs_vel: the cars into a velocity target (screen motion in uv since the last
//    frame, linear depth, coverage), with a depth buffer of their own.
// 2. fs_motion: every pixel's blur vector, linear depth and shadow factor once into a target
//    (the car's velocity where a car is in front, else the camera's: the depth reprojected with the
//    previous view-projection), so the taps of fs_blur only read it instead of redoing the work.
//    fs_blur: per pixel, the velocity is the car's where a car is in front, else the camera's
//    (the depth reprojected with the previous view-projection). Taps along it are weighted by
//    depth and their own velocity (McGuire et al. 2012 without the tile-max pass), so a sharp
//    car is not smeared over by the moving background and does not bleed onto it. Taps read
//    whole pixels (colour and class from the same one), and between a car and the world the
//    depth classes are hard: a bilinear tap at the car's outline, or road within the soft depth
//    range of the car touching it, used to leak the car's colour onto the road (a ghost).
//    The car target has the pixel's centre, the depth MSAA sample 0: on an outline pixel they can
//    disagree, so a car pixel takes the car's own depth, and a pixel whose centre misses the car
//    takes a car of the 3x3 neighbourhood at its depth (`compute_motion`); an MSAA outline pixel (its
//    samples at different depths) takes any car next to it, so its mixed colour is never a road tap.
// DEPTH_TEX is replaced by texture_depth_2d or texture_depth_multisampled_2d (MSAA) at load.

struct CarVel {
    cur: mat4x4<f32>,
    prev: mat4x4<f32>,
}
@group(0) @binding(0) var<uniform> cars: array<CarVel, MAX_CARS>;

struct VelOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) cur: vec4<f32>,
    @location(1) prev: vec4<f32>,
}

@vertex
fn vs_vel(@location(0) pos: vec3<f32>, @builtin(instance_index) k: u32) -> VelOut {
    let c = cars[k].cur * vec4<f32>(pos, 1.0);
    return VelOut(c, c, cars[k].prev * vec4<f32>(pos, 1.0));
}

// uv motion from the previous frame to this one; none for points that were behind the eye
fn uv_motion(cur: vec4<f32>, prev: vec4<f32>) -> vec2<f32> {
    if (prev.w < 0.2) {
        return vec2<f32>(0.0);
    }
    return (cur.xy / cur.w - prev.xy / prev.w) * vec2<f32>(0.5, -0.5);
}

@fragment
fn fs_vel(i: VelOut) -> @location(0) vec4<f32> {
    return vec4<f32>(uv_motion(i.cur, i.prev), i.cur.w, 1.0);
}

struct BlurU {
    inv_view_proj: mat4x4<f32>,
    prev_view_proj: mat4x4<f32>,
    // x: shutter (share of the frame's motion), y: longest blur in pixels, z: near, w: far
    p: vec4<f32>,
    // x: frame number (tap jitter), y: MSAA samples of `depth`, z: how much a full car shadow darkens the world (0: none
    // or not from the shadow map), w: the exponent of that darkening in the target (2.2 linear)
    q: vec4<f32>,
    // for the late sparks (`vs_spark`)
    view_proj: mat4x4<f32>,
}
// render.rs `ShadowMap` (as shader.wgsl's ShadowU: world -> light clip, z: depth bias)
struct ShadowU {
    vp: mat4x4<f32>,
    p: vec4<f32>,
}
@group(0) @binding(0) var scene: texture_2d<f32>;
// (unused since the taps read whole pixels; kept in the layout)
@group(0) @binding(1) var samp: sampler;
@group(0) @binding(2) var depth: DEPTH_TEX;
@group(0) @binding(3) var vel: texture_2d<f32>;
@group(0) @binding(4) var<uniform> u: BlurU;
@group(0) @binding(5) var<uniform> sh_u: ShadowU;
@group(0) @binding(6) var sh_map: texture_depth_2d;
@group(0) @binding(7) var sh_samp: sampler_comparison;
// fs_motion's target (xy: blur vector in pixels, z: linear depth, w: shadow factor, negative on
// car pixels: TAA reads only xy and z); a 1x1 dummy in its own pass
@group(0) @binding(8) var motion_tex: texture_2d<f32>;

// taps per pixel (plus the centre)
const TAPS: i32 = 24;
// depth range (share of the centre's depth) over which a tap turns from front to back
const SOFT_Z: f32 = 0.04;

@vertex
fn vs_blur(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
    let uv = vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u));
    return vec4<f32>(uv * 2.0 - 1.0, 0.0, 1.0);
}

// What the car shadows leave of the world's colour at `p` (1 = lit): shader.wgsl `shadow_vis`
// (3x3 PCF on bilinear compares = a tent over 4x4 texels, here from four gathers, same weights)
// and its world darkening, in the target's colour space. The blur works on the world without
// its shadows and puts the centre's back: the road streaks under a shadow that stays sharp.
const OVERHEAD_TAPS: i32 = 16;
fn shadow_factor(p: vec3<f32>) -> f32 {
    if (u.q.z <= 0.0) {
        return 1.0;
    }
    let lp = sh_u.vp * vec4<f32>(p, 1.0);
    let uv = vec2<f32>(lp.x * 0.5 + 0.5, 0.5 - lp.y * 0.5);
    let e = min(min(uv.x, 1.0 - uv.x), min(uv.y, 1.0 - uv.y));
    if (e <= 0.0 || lp.z <= 0.0 || lp.z >= 1.0) {
        return 1.0;
    }
    var sum = 0.0;
    let z = lp.z - sh_u.p.z;
    if (sh_u.p.w > 0.0) {
        // overhead mode: the disk of shader.wgsl `shadow_vis`, same hash, so the divided-out shadow matches
        let h = fract(sin(dot(floor(p.xz * 256.0), vec2<f32>(12.9898, 78.233))) * 43758.5453);
        for (var k = 0; k < OVERHEAD_TAPS; k++) {
            let a = 2.39996323 * f32(k) + 6.2831853 * h;
            let r = sqrt((f32(k) + 0.5) / f32(OVERHEAD_TAPS)) * sh_u.p.w;
            sum += textureSampleCompareLevel(sh_map, sh_samp, uv + vec2<f32>(cos(a), sin(a)) * r, z);
        }
        sum = sum / f32(OVERHEAD_TAPS) * 9.0;
    } else {
        let n = vec2<f32>(textureDimensions(sh_map));
        let t = uv * n - 0.5;
        let i0 = floor(t);
        let f = t - i0;
        let wx = array<f32, 4>(1.0 - f.x, 1.0, 1.0, f.x);
        let wy = array<f32, 4>(1.0 - f.y, 1.0, 1.0, f.y);
        for (var k = 0; k < 4; k++) {
            let a = (k % 2) * 2;
            let b = (k / 2) * 2;
            // the 2x2 texels from (i0 - 1 + a, i0 - 1 + b): x = (0, 1), y = (1, 1), z = (1, 0), w = (0, 0)
            let g = textureGatherCompare(sh_map, sh_samp, (i0 + vec2<f32>(f32(a), f32(b))) / n, z);
            sum += (g.w * wx[a] + g.z * wx[a + 1]) * wy[b] + (g.x * wx[a] + g.y * wx[a + 1]) * wy[b + 1];
        }
    }
    let inside = smoothstep(0.0, 0.06, e);
    let vis = mix(1.0, sum / 9.0, inside);
    return pow(1.0 - (1.0 - vis) * u.q.z, u.q.w);
}

struct Motion {
    // blur vector in pixels, linear depth, the car shadows' factor (`shadow_factor`; 1 on cars)
    v: vec2<f32>,
    z: f32,
    shade: f32,
    // a car pixel (the car velocity pass won)
    car: bool,
}

// the pixel's blur vector (shutter applied, length clamped), depth and shadow factor: fs_motion's
// work, done once per pixel
fn compute_motion(px: vec2<i32>, size: vec2<f32>) -> Motion {
    let d = textureLoad(depth, px, 0);
    let near = u.p.z;
    let far = u.p.w;
    let z = near * far / (far - d * (far - near));
    var m: vec2<f32>;
    var shade = 1.0;
    // MSAA: the share of the pixel's samples on sample 0's surface (linear depth within 1 % + 5 cm)
    let ns = i32(u.q.y);
    var same = 1.0;
    for (var j = 1; j < ns; j++) {
        let zj = near * far / (far - textureLoad(depth, px, j) * (far - near));
        same += select(0.0, 1.0, abs(zj - z) <= z * 0.01 + 0.05);
    }
    // an outline pixel: its colour is a resolve of a car and the world behind it
    let mixed = ns > 1 && same < f32(ns) - 0.5;
    var car = textureLoad(vel, px, 0);
    // the car pass has its own depth: a car shows where it is not behind the world
    var on_car = car.a > 0.5 && car.z <= z * 1.01 + 0.05;
    var cz = z;
    if (on_car) {
        cz = car.z;
    } else {
        // The car target is single-sampled (the pixel's centre) but `z` is MSAA sample 0, off
        // the centre: on an outline pixel whose sample 0 is a car but whose centre is road, the
        // centre misses it. A car in the 3x3 neighbourhood at this pixel's depth is taken
        // instead, so the pixel is not blurred as a road point reprojected at car depth.
        let dims = vec2<i32>(textureDimensions(vel));
        var best = 1e9;
        for (var j = 0; j < 9; j++) {
            let q = clamp(px + vec2<i32>(j % 3 - 1, j / 3 - 1), vec2<i32>(0), dims - 1);
            let n = textureLoad(vel, q, 0);
            let d = abs(n.z - z);
            // (an outline pixel takes any car in front of its sample 0: its colour is partly the car's)
            let near_enough = d <= z * 0.02 + 0.1 || (mixed && n.z <= z * 1.01 + 0.05);
            if (n.a > 0.5 && near_enough && d < best) {
                best = d;
                car = n;
                on_car = true;
                cz = n.z;
            }
        }
    }
    if (on_car) {
        m = car.xy;
    } else {
        let uv = (vec2<f32>(px) + 0.5) / size;
        let ndc = vec4<f32>(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0, d, 1.0);
        let w = u.inv_view_proj * ndc;
        let p = w.xyz / w.w;
        shade = shadow_factor(p);
        // MSAA: an edge pixel's colour is the resolve of its samples, but the factor is sample
        // 0's. Only the share on sample 0's surface is un-shadowed, the rest counts as lit: a
        // car's outline over its shadow (colour mostly car, factor the road's) would otherwise
        // be divided into bright dots along the streaks.
        if (ns > 1) {
            shade = mix(1.0, shade, same / f32(ns));
        }
        let prev = u.prev_view_proj * vec4<f32>(p, 1.0);
        m = uv_motion(ndc, prev);
    }
    var v = m * size * u.p.x;
    let len = length(v);
    if (len > u.p.y) {
        v = v * (u.p.y / len);
    }
    return Motion(v, cz, shade, on_car);
}

@fragment
fn fs_motion(@builtin(position) pos: vec4<f32>) -> @location(0) vec4<f32> {
    let m = compute_motion(vec2<i32>(pos.xy), vec2<f32>(textureDimensions(scene)));
    return vec4<f32>(m.v, m.z, select(m.shade, -1.0, m.car));
}

fn motion(px: vec2<i32>, size: vec2<f32>) -> Motion {
    let m = textureLoad(motion_tex, px, 0);
    return Motion(m.xy, m.z, abs(m.w), m.w < 0.0);
}

fn cone(x: f32, l: f32) -> f32 {
    return clamp(1.0 - x / max(l, 1e-3), 0.0, 1.0);
}

fn cylinder(x: f32, l: f32) -> f32 {
    return 1.0 - smoothstep(0.95 * l, 1.05 * l, x);
}

// interleaved gradient noise (Jimenez 2014), 0..1
fn ign(p: vec2<f32>, frame: f32) -> f32 {
    let q = p + 5.588238 * (frame % 64.0);
    return fract(52.9829189 * fract(dot(q, vec2<f32>(0.06711056, 0.00583715))));
}

@fragment
fn fs_blur(@builtin(position) pos: vec4<f32>) -> @location(0) vec4<f32> {
    let size = vec2<f32>(textureDimensions(scene));
    let px = vec2<i32>(pos.xy);
    let c = motion(px, size);
    let centre = textureLoad(scene, px, 0);
    let lc = length(c.v);
    if (lc < 0.5) {
        return centre;
    }
    let soft = SOFT_Z * c.z + 0.1;
    let jitter = ign(pos.xy, u.q.x) - 0.5;
    // blurred without the car shadows, the centre's shadow put back
    var sum = centre * (4.0 / c.shade);
    var wsum = 4.0;
    let hi = size - 1.0;
    for (var k = 0; k < TAPS; k++) {
        // taps spread over the open shutter, centred on this frame
        let t = mix(-0.5, 0.5, (f32(k) + 0.5 + jitter) / f32(TAPS));
        let sp = clamp(pos.xy + c.v * t, vec2<f32>(0.0), hi);
        let sx = vec2<i32>(sp);
        let s = motion(sx, size);
        let dist = abs(t) * lc;
        let ls = length(s.v);
        // front: the tap is not behind the centre; back: it is not in front of it (hard
        // between a car and the world, so a sharp car is never "level" with the road it touches)
        let k = select(soft, 1e-3, s.car != c.car);
        let front = clamp(1.0 + (c.z - s.z) / k, 0.0, 1.0);
        let back = clamp(1.0 + (s.z - c.z) / k, 0.0, 1.0);
        let w = front * cone(dist, ls) + back * cone(dist, lc) + cylinder(dist, ls) * cylinder(dist, lc) * 2.0;
        sum += textureLoad(scene, sx, 0) * (w / s.shade);
        wsum += w;
    }
    return vec4<f32>((sum / wsum).rgb * c.shade, 1.0);
}

// REMASTER sparks drawn after the blur (render.rs `spark_pipeline`): thin hot streaks, head at
// uv.x = 1, fading to the tail. The shape is shader.wgsl `spark_shape`. They are light, so the blur
// never sees them; the world's depth hides them by hand (DEPTH_TEX may be multisampled: sample 0).
struct SparkOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) color: vec4<f32>,
}

@vertex
fn vs_spark(@location(0) pos: vec3<f32>, @location(1) uv: vec2<f32>, @location(2) color: vec4<f32>) -> SparkOut {
    return SparkOut(u.view_proj * vec4<f32>(pos, 1.0), uv, color);
}

@fragment
fn fs_spark(i: SparkOut) -> @location(0) vec4<f32> {
    if (i.clip.z > textureLoad(depth, vec2<i32>(i.clip.xy), 0) + 2e-5) {
        discard;
    }
    let across = i.uv.y * 2.0 - 1.0;
    let shape = exp(-across * across * 6.0) * pow(i.uv.x, 1.5) * smoothstep(1.0, 0.9, i.uv.x);
    let hdr = u.q.w > 1.5;
    let rgb = pow(i.color.rgb, vec3<f32>(u.q.w));
    return vec4<f32>(rgb * shape * i.color.a * select(1.0, 4.0, hdr), 1.0);
}
