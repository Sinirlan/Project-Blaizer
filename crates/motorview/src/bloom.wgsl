// REMASTER bloom (render.rs `Bloom`) [design choice, not in the original]: a dual-filter
// chain over the HDR scene. fs_prefilter and fs_down halve the size with the 13-tap filter
// of Jimenez, "Next generation post processing in Call of Duty: Advanced Warfare" (2014);
// the first one keeps only light above a soft threshold and weights its groups with the
// Karis average, so single bright pixels don't flicker. fs_up adds a 3x3 tent of the mip
// below to each mip on the way back up.

@group(0) @binding(0) var src: texture_2d<f32>;
@group(0) @binding(1) var samp: sampler;
// x: threshold, y: knee, z: brightest light fed to the bloom (linear light)
@group(0) @binding(2) var<uniform> p: vec4<f32>;

struct Out {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs_bloom(@builtin(vertex_index) i: u32) -> Out {
    let t = vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u));
    var o: Out;
    o.pos = vec4<f32>(t * 2.0 - 1.0, 0.0, 1.0);
    o.uv = vec2<f32>(t.x, 1.0 - t.y);
    return o;
}

// the half-float target can hold inf after an overflow: keep it finite
fn tap(uv: vec2<f32>) -> vec3<f32> {
    return min(textureSampleLevel(src, samp, uv, 0.0).rgb, vec3<f32>(60000.0));
}

fn luma(c: vec3<f32>) -> f32 {
    return dot(c, vec3<f32>(0.2126, 0.7152, 0.0722));
}

// the 13 taps as five overlapping 2x2 boxes (centre 0.5, corners 0.125 each); `karis`
// weights each box by 1 / (1 + luma)
fn down13(uv: vec2<f32>, karis: bool) -> vec3<f32> {
    let k = 1.0 / vec2<f32>(textureDimensions(src));
    let a = tap(uv + k * vec2<f32>(-2.0, -2.0));
    let b = tap(uv + k * vec2<f32>(0.0, -2.0));
    let c = tap(uv + k * vec2<f32>(2.0, -2.0));
    let d = tap(uv + k * vec2<f32>(-1.0, -1.0));
    let e = tap(uv + k * vec2<f32>(1.0, -1.0));
    let f = tap(uv + k * vec2<f32>(-2.0, 0.0));
    let g = tap(uv);
    let h = tap(uv + k * vec2<f32>(2.0, 0.0));
    let i = tap(uv + k * vec2<f32>(-1.0, 1.0));
    let j = tap(uv + k * vec2<f32>(1.0, 1.0));
    let l = tap(uv + k * vec2<f32>(-2.0, 2.0));
    let m = tap(uv + k * vec2<f32>(0.0, 2.0));
    let n = tap(uv + k * vec2<f32>(2.0, 2.0));
    let boxes = array<vec3<f32>, 5>((d + e + i + j) * 0.25, (a + b + f + g) * 0.25, (b + c + g + h) * 0.25, (f + g + l + m) * 0.25, (g + h + m + n) * 0.25);
    let weights = array<f32, 5>(0.5, 0.125, 0.125, 0.125, 0.125);
    var sum = vec3<f32>(0.0);
    var total = 0.0;
    for (var q = 0u; q < 5u; q++) {
        let w = weights[q] * select(1.0, 1.0 / (1.0 + luma(boxes[q])), karis);
        sum += boxes[q] * w;
        total += w;
    }
    return sum / total;
}

@fragment
fn fs_prefilter(i: Out) -> @location(0) vec4<f32> {
    let c0 = down13(i.uv, true);
    // sun glints on the cars and the sky round the sun reach far above 1: they feed the bloom
    // no brighter than p.z (halo cores, 2.5, stay under it)
    let c = c0 * min(1.0, p.z / max(c0.r, max(c0.g, max(c0.b, 1e-5))));
    // soft threshold: a quadratic knee of width 2 * p.y around p.x, linear above
    let br = max(c.r, max(c.g, c.b));
    let soft = clamp(br - p.x + p.y, 0.0, 2.0 * p.y);
    let w = max(soft * soft / (4.0 * p.y + 1e-5), br - p.x) / max(br, 1e-5);
    return vec4<f32>(c * w, 1.0);
}

@fragment
fn fs_down(i: Out) -> @location(0) vec4<f32> {
    return vec4<f32>(down13(i.uv, false), 1.0);
}

@fragment
fn fs_up(i: Out) -> @location(0) vec4<f32> {
    let k = 1.0 / vec2<f32>(textureDimensions(src));
    var c = tap(i.uv) * 4.0;
    c += (tap(i.uv + vec2<f32>(k.x, 0.0)) + tap(i.uv - vec2<f32>(k.x, 0.0)) + tap(i.uv + vec2<f32>(0.0, k.y)) + tap(i.uv - vec2<f32>(0.0, k.y))) * 2.0;
    c += tap(i.uv + k) + tap(i.uv - k) + tap(i.uv + vec2<f32>(k.x, -k.y)) + tap(i.uv + vec2<f32>(-k.x, k.y));
    return vec4<f32>(c / 16.0, 1.0);
}
