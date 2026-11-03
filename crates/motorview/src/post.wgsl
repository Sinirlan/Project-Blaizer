// REMASTER post pass (render.rs `RemasterView::post_pass`) [design choice, not in the original].
// HDR on: the linear scene times the exposure, plus the bloom, goes through the tonemapper
// and back to gamma space. Then the colour grade: a saturation lift and S-curve, a vignette
// and moving film grain, each with its own strength option. APPROX of 09-remaster's "3D LUT
// per track": one parametric grade for every track.

@group(0) @binding(0) var scene: texture_2d<f32>;
// [0] x: frame number (grain seed); y, z, w: grade, vignette and grain strength (0..1)
// [1] x: exposure, y: bloom weight, z: tonemapper (0 = none: the scene is in gamma space,
//     1 Neutral, 2 ACES, 3 AgX)
@group(0) @binding(1) var<uniform> p: array<vec4<f32>, 2>;
// the bloom chain's top mip (half size)
@group(0) @binding(2) var bloom: texture_2d<f32>;
@group(0) @binding(3) var samp: sampler;

// full-strength values (the options' 100 %)
const SATURATION: f32 = 0.25;
// share of the smoothstep S-curve mixed in
const CONTRAST: f32 = 0.35;
// darkening at the corners, from VIGNETTE_START of the half diagonal outwards
const VIGNETTE: f32 = 0.5;
const VIGNETTE_START: f32 = 0.45;
// grain amplitude (peak to peak, 0..1 colour)
const GRAIN: f32 = 0.05;
// the shaders' gamma (shader.wgsl `lin`)
const GAMMA: f32 = 2.2;

@vertex
fn vs_post(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
    let uv = vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u));
    return vec4<f32>(uv * 2.0 - 1.0, 0.0, 1.0);
}

// PCG integer hash (Jarzynski & Olano 2020). The usual fract(sin(dot(..))) hash loses f32
// precision at pixel-sized arguments and shows a moiré lattice at high grain strengths.
fn pcg(v: u32) -> u32 {
    let s = v * 747796405u + 2891336453u;
    let w = ((s >> ((s >> 28u) + 4u)) ^ s) * 277803737u;
    return (w >> 22u) ^ w;
}

// uniform 0..1 per pixel and frame
fn hash(q: vec2<u32>, frame: u32) -> f32 {
    return f32(pcg(q.x + pcg(q.y + pcg(frame))) >> 8u) / 16777216.0;
}

// Khronos PBR Neutral (2024) without its toe: colours whose largest channel is below 0.76
// pass unchanged, brighter ones roll off towards 1 and desaturate a little. The toe takes
// away a PBR renderer's 0.04 Fresnel floor, which the 1998 colours don't have.
fn neutral(c: vec3<f32>) -> vec3<f32> {
    let start = 0.76;
    let desat = 0.15;
    let peak = max(c.r, max(c.g, c.b));
    if (peak < start) {
        return c;
    }
    let d = 1.0 - start;
    let np = 1.0 - d * d / (peak + d - start);
    let g = 1.0 - 1.0 / (desat * (peak - np) + 1.0);
    return mix(c * (np / peak), vec3<f32>(np), g);
}

// ACES filmic curve, Narkowicz's fit (2015), with its 0.6 input scale
fn aces(c0: vec3<f32>) -> vec3<f32> {
    let c = c0 * 0.6;
    return clamp(c * (2.51 * c + 0.03) / (c * (2.43 * c + 0.59) + 0.14), vec3<f32>(0.0), vec3<f32>(1.0));
}

// AgX (Troy Sobotka), the minimal fit by Benjamin Wrensch (2023): log2 encoding in the
// inset space, a sigmoid polynomial, the outset matrix, back to linear
fn agx(c: vec3<f32>) -> vec3<f32> {
    let inset = mat3x3<f32>(
        vec3<f32>(0.842479062253094, 0.0423282422610123, 0.0423756549057051),
        vec3<f32>(0.0784335999999992, 0.878468636469772, 0.0784336),
        vec3<f32>(0.0792237451477643, 0.0791661274605434, 0.879142973793104),
    );
    let outset = mat3x3<f32>(
        vec3<f32>(1.19687900512017, -0.0528968517574562, -0.0529716355144438),
        vec3<f32>(-0.0980208811401368, 1.15190312990417, -0.0980434501171241),
        vec3<f32>(-0.0990297440797205, -0.0989611768448433, 1.15107367264116),
    );
    let min_ev = -12.47393;
    let max_ev = 4.026069;
    var x = clamp(log2(max(inset * c, vec3<f32>(1e-10))), vec3<f32>(min_ev), vec3<f32>(max_ev));
    x = (x - min_ev) / (max_ev - min_ev);
    let x2 = x * x;
    let x4 = x2 * x2;
    x = 15.5 * x4 * x2 - 40.14 * x4 * x + 31.96 * x4 - 6.868 * x2 * x + 0.4298 * x2 + 0.1191 * x - 0.00232;
    return pow(max(outset * x, vec3<f32>(0.0)), vec3<f32>(GAMMA));
}

@fragment
fn fs_post(@builtin(position) pos: vec4<f32>) -> @location(0) vec4<f32> {
    let size = vec2<f32>(textureDimensions(scene));
    var c = textureLoad(scene, vec2<i32>(pos.xy), 0).rgb;
    let h = p[1];
    if (h.z > 0.5) {
        let b = textureSampleLevel(bloom, samp, pos.xy / size, 0.0).rgb;
        c = (c + b * h.y) * h.x;
        let kind = u32(h.z + 0.5);
        if (kind == 1u) {
            c = neutral(c);
        } else if (kind == 2u) {
            c = aces(c);
        } else {
            c = agx(c);
        }
        c = pow(clamp(c, vec3<f32>(0.0), vec3<f32>(1.0)), vec3<f32>(1.0 / GAMMA));
    }
    let q = p[0];
    let luma = dot(c, vec3<f32>(0.299, 0.587, 0.114));
    c = clamp(mix(vec3<f32>(luma), c, 1.0 + SATURATION * q.y), vec3<f32>(0.0), vec3<f32>(1.0));
    c = mix(c, c * c * (3.0 - 2.0 * c), CONTRAST * q.y);
    // distance from the centre, 1 at the corners
    let d = length((pos.xy / size - 0.5) * 2.0) / sqrt(2.0);
    c = c * (1.0 - VIGNETTE * q.z * smoothstep(VIGNETTE_START, 1.0, d));
    let n = hash(vec2<u32>(pos.xy), u32(q.x)) - 0.5;
    return vec4<f32>(clamp(c + n * GRAIN * q.w, vec3<f32>(0.0), vec3<f32>(1.0)), 1.0);
}
