// REMASTER TAA (13-antialiasing.md phase 5), written from the published technique (Karis 2014,
// "High Quality Temporal Supersampling"; Catmull-Rom history fetch after Jimenez 2016): the main
// view is drawn with a Halton sub-pixel jitter (render.rs `TAA_JITTER`); each frame the history
// is reprojected with the velocity pass's motion (vblur.wgsl `fs_motion`, the jitter cancelled in
// the matrices), clipped to the colour box of the current 3x3 neighbourhood in YCoCg, and blended
// in. It runs on the scene (linear HDR, or the swapchain format) before bloom and tonemap, so the
// blend works on c / (1 + luma) there: a hot pixel does not dominate [design choice].
// The history's alpha keeps the pixel's motion length, and the history's weight drops where it
// disagrees with the current motion (Jimenez 2016, velocity weighting): road uncovered under a car
// (about 20 px per frame) must not keep the car's rear (about 0 px per frame) that the colour box
// lets through when body and road are both neutral grey [design choice].
// 1. fs_taa: scene + history -> new history. 2. fs_sharpen: history -> scene (the history itself
// stays unsharpened, so the sharpening does not accumulate).

@group(0) @binding(0) var cur_t: texture_2d<f32>;
@group(0) @binding(1) var hist_t: texture_2d<f32>;
// xy: blur vector in render pixels (times the velocity-blur shutter), z: linear depth
@group(0) @binding(2) var motion_t: texture_2d<f32>;
@group(0) @binding(3) var samp: sampler;
struct TaaU {
    // output size, render size (pixels)
    sizes: vec4<f32>,
    // x: history weight, y: sharpen, z: 1 when the history is usable, w: 1 / the shutter in `motion_t`
    p: vec4<f32>,
    // x: 1 for the linear HDR scene
    q: vec4<f32>,
}
@group(0) @binding(4) var<uniform> u: TaaU;

@vertex
fn vs_taa(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
    let uv = vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u));
    return vec4<f32>(uv * 2.0 - 1.0, 0.0, 1.0);
}

fn luma(c: vec3<f32>) -> f32 {
    return dot(c, vec3<f32>(0.2126, 0.7152, 0.0722));
}

// the colour as blended: c / (1 + luma) in the HDR scene
fn to_work(c: vec3<f32>) -> vec3<f32> {
    if (u.q.x > 0.5) {
        return c / (1.0 + luma(c));
    }
    return c;
}

// motion length (render pixels per frame) in the history's alpha: 0..1 in any target format
const MOTION_K: f32 = 4.0;
// the history keeps its weight below VEL_LO relative motion difference and loses it all at VEL_HI;
// VEL_FLOOR (pixels) keeps near-still pixels from counting tiny differences
const VEL_LO: f32 = 0.4;
const VEL_HI: f32 = 0.8;
const VEL_FLOOR: f32 = 2.0;

fn encode_motion(len: f32) -> f32 {
    return len / (len + MOTION_K);
}

fn decode_motion(a: f32) -> f32 {
    let c = clamp(a, 0.0, 0.98);
    return MOTION_K * c / (1.0 - c);
}

fn from_work(c: vec3<f32>) -> vec3<f32> {
    if (u.q.x > 0.5) {
        return c / max(1.0 - luma(c), 1e-3);
    }
    return c;
}

fn to_ycocg(c: vec3<f32>) -> vec3<f32> {
    return vec3<f32>(0.25 * c.r + 0.5 * c.g + 0.25 * c.b, 0.5 * c.r - 0.5 * c.b, -0.25 * c.r + 0.5 * c.g - 0.25 * c.b);
}

fn from_ycocg(c: vec3<f32>) -> vec3<f32> {
    return vec3<f32>(c.x + c.y - c.z, c.x + c.z, c.x - c.y - c.z);
}

fn load_cur(p: vec2<i32>) -> vec3<f32> {
    return textureLoad(cur_t, clamp(p, vec2<i32>(0), vec2<i32>(u.sizes.xy) - 1), 0).rgb;
}

// the history at `uv`, bicubic (Catmull-Rom) from five bilinear taps
fn history(uv: vec2<f32>) -> vec4<f32> {
    let size = u.sizes.xy;
    let pos = uv * size;
    let c = floor(pos - 0.5) + 0.5;
    let f = pos - c;
    let w0 = f * (-0.5 + f * (1.0 - 0.5 * f));
    let w1 = 1.0 + f * f * (-2.5 + 1.5 * f);
    let w2 = f * (0.5 + f * (2.0 - 1.5 * f));
    let w3 = f * f * (-0.5 + 0.5 * f);
    let w12 = w1 + w2;
    let t12 = w2 / w12;
    let p0 = (c - 1.0) / size;
    let p3 = (c + 2.0) / size;
    let p12 = (c + t12) / size;
    var s = textureSampleLevel(hist_t, samp, vec2<f32>(p12.x, p0.y), 0.0) * (w12.x * w0.y);
    s += textureSampleLevel(hist_t, samp, vec2<f32>(p0.x, p12.y), 0.0) * (w0.x * w12.y);
    s += textureSampleLevel(hist_t, samp, p12, 0.0) * (w12.x * w12.y);
    s += textureSampleLevel(hist_t, samp, vec2<f32>(p3.x, p12.y), 0.0) * (w3.x * w12.y);
    s += textureSampleLevel(hist_t, samp, vec2<f32>(p12.x, p3.y), 0.0) * (w12.x * w3.y);
    return max(s, vec4<f32>(0.0));
}

// pulls `h` towards the box's centre until it is inside (both in YCoCg)
fn clip_box(lo: vec3<f32>, hi: vec3<f32>, h: vec3<f32>) -> vec3<f32> {
    let mid = 0.5 * (hi + lo);
    let ext = 0.5 * (hi - lo) + 1e-4;
    let v = h - mid;
    let a = abs(v / ext);
    let m = max(a.x, max(a.y, a.z));
    if (m > 1.0) {
        return mid + v / m;
    }
    return h;
}

@fragment
fn fs_taa(@builtin(position) pos: vec4<f32>) -> @location(0) vec4<f32> {
    let px = vec2<i32>(pos.xy);
    let cur = to_work(load_cur(px));
    // the 3x3 neighbourhood's colour box
    var lo = vec3<f32>(1e9);
    var hi = vec3<f32>(-1e9);
    var m1 = vec3<f32>(0.0);
    var m2 = vec3<f32>(0.0);
    for (var y = -1; y <= 1; y++) {
        for (var x = -1; x <= 1; x++) {
            let s = to_ycocg(to_work(load_cur(px + vec2<i32>(x, y))));
            lo = min(lo, s);
            hi = max(hi, s);
            m1 += s;
            m2 += s * s;
        }
    }
    let mu = m1 / 9.0;
    let sigma = sqrt(max(m2 / 9.0 - mu * mu, vec3<f32>(0.0)));
    let box_lo = max(lo, mu - 1.25 * sigma);
    let box_hi = min(hi, mu + 1.25 * sigma);
    // the motion of the closest of 3x3 render pixels (a car's edge keeps the car's motion)
    let rsz = u.sizes.zw;
    let rc = vec2<i32>(floor((vec2<f32>(px) + 0.5) / u.sizes.xy * rsz));
    var best = vec4<f32>(0.0, 0.0, 1e9, 0.0);
    for (var y = -1; y <= 1; y++) {
        for (var x = -1; x <= 1; x++) {
            let m = textureLoad(motion_t, clamp(rc + vec2<i32>(x, y), vec2<i32>(0), vec2<i32>(rsz) - 1), 0);
            if (m.z < best.z) {
                best = m;
            }
        }
    }
    // (render pixels per frame; with the velocity blur on it is clamped at VBLUR_MAX of the width)
    let lc = length(best.xy * u.p.w);
    let huv = (vec2<f32>(px) + 0.5) / u.sizes.xy - best.xy * u.p.w / rsz;
    var out = cur;
    if (u.p.z > 0.5 && all(huv > vec2<f32>(0.0)) && all(huv < vec2<f32>(1.0))) {
        let hs = history(huv);
        let h = clip_box(box_lo, box_hi, to_ycocg(to_work(hs.rgb)));
        // the history of another motion is another object (the car's rear on the road beneath it)
        let lp = decode_motion(hs.a);
        let rel = abs(lc - lp) / (max(lc, lp) + VEL_FLOOR);
        out = mix(cur, from_ycocg(h), u.p.x * (1.0 - smoothstep(VEL_LO, VEL_HI, rel)));
    }
    return vec4<f32>(from_work(out), encode_motion(lc));
}

// unsharp mask on the resolved history, kept inside the range of its own neighbours
@fragment
fn fs_sharpen(@builtin(position) pos: vec4<f32>) -> @location(0) vec4<f32> {
    let px = vec2<i32>(pos.xy);
    let c = to_work(load_cur(px));
    let n = to_work(load_cur(px + vec2<i32>(0, -1)));
    let s = to_work(load_cur(px + vec2<i32>(0, 1)));
    let w = to_work(load_cur(px + vec2<i32>(-1, 0)));
    let e = to_work(load_cur(px + vec2<i32>(1, 0)));
    let lo = min(c, min(min(n, s), min(w, e)));
    let hi = max(c, max(max(n, s), max(w, e)));
    let sharp = clamp(c + u.p.y * (4.0 * c - n - s - w - e) * 0.5, lo, hi);
    return vec4<f32>(max(from_work(sharp), vec3<f32>(0.0)), 1.0);
}
