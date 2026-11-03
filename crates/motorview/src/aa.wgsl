// REMASTER anti-aliasing passes (13-antialiasing.md).
// SSAA downsample (phase 2): the scene rendered at render-scale x the output size is filtered to
// the output size with a box filter one output pixel wide [design choice]. Each bilinear tap
// averages 2x2 source texels, so n = max(ceil(scale / 2), 2) taps per axis, evenly spread over the
// pixel, cover it: 4 taps up to 4x, 16 at 8x.
// FXAA (phase 3): after the FXAA 3.11 Quality algorithm (T. Lottes), written from the published
// description: luma edge detection on the tonemapped image, an end-of-edge search along the edge,
// and a sub-pixel shift of the sample across it.

@group(0) @binding(0) var src: texture_2d<f32>;
@group(0) @binding(1) var samp: sampler;
// x, y: output size in pixels; z: render scale per axis; w: FXAA subpixel quality
@group(0) @binding(2) var<uniform> out_size: vec4<f32>;

@vertex
fn vs_aa(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
    let uv = vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u));
    return vec4<f32>(uv * 2.0 - 1.0, 0.0, 1.0);
}

@fragment
fn fs_down(@builtin(position) pos: vec4<f32>) -> @location(0) vec4<f32> {
    let uv = pos.xy / out_size.xy;
    let n = max(i32(ceil(out_size.z * 0.5)), 2);
    let step = 1.0 / (f32(n) * out_size.xy);
    var c = vec4<f32>(0.0);
    for (var y = 0; y < n; y++) {
        for (var x = 0; x < n; x++) {
            // tap centres at (k + 0.5) / n of the output pixel, from its centre
            let o = (vec2<f32>(f32(x), f32(y)) + 0.5 - 0.5 * f32(n)) * step;
            c += textureSampleLevel(src, samp, uv + o, 0.0);
        }
    }
    c = c / f32(n * n);
    return vec4<f32>(c.rgb, 1.0);
}

const FXAA_EDGE_THRESHOLD: f32 = 0.166;
const FXAA_EDGE_MIN: f32 = 0.0833;
const FXAA_SEARCH_STEPS: i32 = 12;
// edge-search step lengths in pixels (quality preset 29: 1 1.5 2 2 2 2 4 8 ...)
const FXAA_STEPS = array<f32, 12>(1.0, 1.0, 1.0, 1.0, 1.0, 1.5, 2.0, 2.0, 2.0, 2.0, 4.0, 8.0);

fn luma(c: vec3<f32>) -> f32 {
    return dot(c, vec3<f32>(0.299, 0.587, 0.114));
}

fn tap(uv: vec2<f32>) -> vec3<f32> {
    return textureSampleLevel(src, samp, uv, 0.0).rgb;
}

@fragment
fn fs_fxaa(@builtin(position) pos: vec4<f32>) -> @location(0) vec4<f32> {
    let px = 1.0 / out_size.xy;
    let uv = pos.xy * px;
    let cm = tap(uv);
    let lm = luma(cm);
    let ln = luma(tap(uv + vec2<f32>(0.0, -px.y)));
    let ls = luma(tap(uv + vec2<f32>(0.0, px.y)));
    let lw = luma(tap(uv + vec2<f32>(-px.x, 0.0)));
    let le = luma(tap(uv + vec2<f32>(px.x, 0.0)));
    let lmax = max(lm, max(max(ln, ls), max(lw, le)));
    let lmin = min(lm, min(min(ln, ls), min(lw, le)));
    let range = lmax - lmin;
    if (range < max(FXAA_EDGE_MIN, lmax * FXAA_EDGE_THRESHOLD)) {
        return vec4<f32>(cm, 1.0);
    }
    let lnw = luma(tap(uv + vec2<f32>(-px.x, -px.y)));
    let lne = luma(tap(uv + vec2<f32>(px.x, -px.y)));
    let lsw = luma(tap(uv + vec2<f32>(-px.x, px.y)));
    let lse = luma(tap(uv + vec2<f32>(px.x, px.y)));
    // sub-pixel aliasing: how far the centre is from the average of its 3x3 neighbourhood
    let lavg = (2.0 * (ln + ls + lw + le) + lnw + lne + lsw + lse) / 12.0;
    let sub = clamp(abs(lavg - lm) / range, 0.0, 1.0);
    let sub_blend = smoothstep(0.0, 1.0, sub);
    let sub_amount = sub_blend * sub_blend * out_size.w;
    // horizontal or vertical edge
    let edge_h = abs(-2.0 * lw + lnw + lsw) + abs(-2.0 * lm + ln + ls) * 2.0 + abs(-2.0 * le + lne + lse);
    let edge_v = abs(-2.0 * ln + lnw + lne) + abs(-2.0 * lm + lw + le) * 2.0 + abs(-2.0 * ls + lsw + lse);
    let horz = edge_h >= edge_v;
    // the side of the edge with the steeper gradient
    var l1 = select(lw, ln, horz);
    var l2 = select(le, ls, horz);
    var step_len = select(px.x, px.y, horz);
    let g1 = abs(l1 - lm);
    let g2 = abs(l2 - lm);
    let steeper1 = g1 >= g2;
    let grad = max(g1, g2) * 0.25;
    var l_local = 0.5 * (select(l2, l1, steeper1) + lm);
    if (steeper1) {
        step_len = -step_len;
    }
    // move to the edge between the two pixels, then search both ways along it
    var edge_uv = uv;
    if (horz) {
        edge_uv.y += step_len * 0.5;
    } else {
        edge_uv.x += step_len * 0.5;
    }
    let along = select(vec2<f32>(0.0, px.y), vec2<f32>(px.x, 0.0), horz);
    var uv1 = edge_uv - along;
    var uv2 = edge_uv + along;
    var e1 = luma(tap(uv1)) - l_local;
    var e2 = luma(tap(uv2)) - l_local;
    var done1 = abs(e1) >= grad;
    var done2 = abs(e2) >= grad;
    if (!done1) { uv1 -= along * FXAA_STEPS[0]; }
    if (!done2) { uv2 += along * FXAA_STEPS[0]; }
    for (var i = 1; i < FXAA_SEARCH_STEPS; i++) {
        if (done1 && done2) {
            break;
        }
        if (!done1) {
            e1 = luma(tap(uv1)) - l_local;
            done1 = abs(e1) >= grad;
            if (!done1) { uv1 -= along * FXAA_STEPS[i]; }
        }
        if (!done2) {
            e2 = luma(tap(uv2)) - l_local;
            done2 = abs(e2) >= grad;
            if (!done2) { uv2 += along * FXAA_STEPS[i]; }
        }
    }
    let d1 = select(uv.y - uv1.y, uv.x - uv1.x, horz);
    let d2 = select(uv2.y - uv.y, uv2.x - uv.x, horz);
    let near1 = d1 < d2;
    let dist = min(d1, d2);
    let span = d1 + d2;
    // only shift when the end of the edge agrees with the sign of the centre's difference
    let centre_below = lm < l_local;
    let end_below = select(e2, e1, near1) < 0.0;
    var shift = 0.0;
    if (end_below != centre_below) {
        shift = 0.5 - dist / span;
    }
    let off = max(shift, sub_amount) * step_len;
    var out_uv = uv;
    if (horz) {
        out_uv.y += off;
    } else {
        out_uv.x += off;
    }
    return vec4<f32>(tap(out_uv), 1.0);
}
