// REMASTER SMAA 1x (13-antialiasing.md phase 4): three passes on the tonemapped image, after
// Jimenez et al., "SMAA" (2012), written from the published description: luma edges with local
// contrast adaptation, blend weights from the shape of each edge line, and a neighbourhood
// blend. [design choice] Two departures from the reference, which only exist to avoid its lookup
// tables: the search walks the line one pixel per step with exact loads (no SearchTex), and the
// area under the contour is integrated analytically here (no AreaTex). High and Ultra add the
// reference's corner rounding (`corners`). [design choice] No diagonal pass: with exact loads
// and areas a 45° staircase is already a chain of Z shapes of length 1 (solid) or 2 (a thin
// line), each 1/8 per side, the area of the ideal line; the reference needs it to repair what its
// bilinear crossing fetches and AreaTex make of those patterns.

@group(0) @binding(0) var ldr: texture_2d<f32>;
@group(0) @binding(1) var edges_t: texture_2d<f32>;
@group(0) @binding(2) var weights_t: texture_2d<f32>;
// x: edge threshold; y: longest line searched, in pixels; z: unused; w: the share of the blend a
// real corner takes away (0 = off)
@group(0) @binding(3) var<uniform> prm: vec4<f32>;

@vertex
fn vs_smaa(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
    let uv = vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u));
    return vec4<f32>(uv * 2.0 - 1.0, 0.0, 1.0);
}

fn inside(p: vec2<i32>) -> bool {
    let s = vec2<i32>(textureDimensions(ldr));
    return p.x >= 0 && p.y >= 0 && p.x < s.x && p.y < s.y;
}

fn colour(p: vec2<i32>) -> vec3<f32> {
    return textureLoad(ldr, clamp(p, vec2<i32>(0), vec2<i32>(textureDimensions(ldr)) - 1), 0).rgb;
}

fn luma(p: vec2<i32>) -> f32 {
    return dot(colour(p), vec3<f32>(0.2126, 0.7152, 0.0722));
}

// edges of a pixel: x = its left edge, y = its top edge (0 outside the image)
fn edge(p: vec2<i32>) -> vec2<f32> {
    if (!inside(p)) { return vec2<f32>(0.0); }
    return textureLoad(edges_t, p, 0).rg;
}

@fragment
fn fs_edges(@builtin(position) pos: vec4<f32>) -> @location(0) vec2<f32> {
    let p = vec2<i32>(pos.xy);
    let l = luma(p);
    let ll = luma(p + vec2<i32>(-1, 0));
    let lt = luma(p + vec2<i32>(0, -1));
    let d = vec2<f32>(abs(l - ll), abs(l - lt));
    var e = step(vec2<f32>(prm.x), d);
    if (e.x + e.y == 0.0) { return vec2<f32>(0.0); }
    // local contrast adaptation: an edge must be at least half the strongest nearby one
    var m = max(d.x, d.y);
    m = max(m, abs(l - luma(p + vec2<i32>(1, 0))));
    m = max(m, abs(l - luma(p + vec2<i32>(0, 1))));
    m = max(m, abs(ll - luma(p + vec2<i32>(-2, 0))));
    m = max(m, abs(lt - luma(p + vec2<i32>(0, -2))));
    e = e * step(vec2<f32>(m), 2.0 * d);
    return e;
}

// The edge line is a run of pixels whose top (or left) edge is set. y(x) is the smoothed
// contour over the line in pixels, positive towards the pixel side: `c0` and `c1` are the
// offsets (+-0.5) the perpendicular edges at its two ends ask for. A Z shape (opposite ends)
// is one slope across the line; an L or U shape ramps back to 0 at the middle.
fn contour(x: f32, len: f32, c0: f32, c1: f32) -> f32 {
    if (c0 * c1 < 0.0) { return c0 + (c1 - c0) * x / len; }
    let h = 0.5 * len;
    var y = 0.0;
    if (x < h) { y += c0 * (1.0 - x / h); }
    if (x > h) { y += c1 * (1.0 - (len - x) / h); }
    return y;
}

// (area where y > 0, area where y < 0) of the straight piece through (0, ya) and (w, yb)
fn piece(w: f32, ya: f32, yb: f32) -> vec2<f32> {
    if (w <= 0.0) { return vec2<f32>(0.0); }
    if (ya >= 0.0 && yb >= 0.0) { return vec2<f32>(0.5 * (ya + yb) * w, 0.0); }
    if (ya <= 0.0 && yb <= 0.0) { return vec2<f32>(0.0, -0.5 * (ya + yb) * w); }
    let t = w * ya / (ya - yb);
    if (ya > 0.0) { return vec2<f32>(0.5 * ya * t, -0.5 * yb * (w - t)); }
    return vec2<f32>(0.5 * yb * (w - t), -0.5 * ya * t);
}

// the same over the pixel [a, a + 1] of the line
fn pixel_area(a: f32, len: f32, c0: f32, c1: f32) -> vec2<f32> {
    let h = 0.5 * len;
    let b = a + 1.0;
    if (c0 * c1 < 0.0 || h <= a || h >= b) {
        return piece(1.0, contour(a, len, c0, c1), contour(b, len, c0, c1));
    }
    let ya = contour(a, len, c0, c1);
    return piece(h - a, ya, 0.0) + piece(b - h, 0.0, contour(b, len, c0, c1));
}

// the perpendicular edges at the start of pixel `q`: x on the other side of the line, y on
// the pixel side (`n` points from the other side to the pixel side)
fn ends(q: vec2<i32>, n: vec2<i32>, vert: bool) -> vec2<f32> {
    let a = edge(q - n);
    let b = edge(q);
    return select(vec2<f32>(a.x, b.x), vec2<f32>(a.y, b.y), vert);
}

// the contour offset such ends ask for: +0.5 pixel side, -0.5 other side, 0 for none or both
fn end_offset(e: vec2<f32>) -> f32 {
    if (e.x > 0.5 && e.y > 0.5) { return 0.0; }
    if (e.y > 0.5) { return 0.5; }
    if (e.x > 0.5) { return -0.5; }
    return 0.0;
}

fn is_line(q: vec2<i32>, vert: bool) -> bool {
    let e = edge(q);
    return select(e.y, e.x, vert) > 0.5;
}

// blend amounts (towards the other side, towards the pixel side) of the line through the top
// (`vert` false) or left (true) edge of pixel `p`. The line runs until a perpendicular edge
// or its last pixel; a line longer than the reach is cut there.
fn line_weights(p: vec2<i32>, vert: bool) -> vec2<f32> {
    let d = select(vec2<i32>(1, 0), vec2<i32>(0, 1), vert);
    let n = select(vec2<i32>(0, 1), vec2<i32>(1, 0), vert);
    let reach = i32(prm.y);
    var dl = 0;
    var c0 = 0.0;
    for (var i = 0; i < reach; i++) {
        let e = ends(p - d * dl, n, vert);
        c0 = end_offset(e);
        if (e.x > 0.5 || e.y > 0.5 || !is_line(p - d * (dl + 1), vert)) { break; }
        dl += 1;
    }
    var dr = 0;
    var c1 = 0.0;
    for (var i = 0; i < reach; i++) {
        let q = p + d * (dr + 1);
        let e = ends(q, n, vert);
        c1 = end_offset(e);
        if (e.x > 0.5 || e.y > 0.5 || !is_line(q, vert)) { break; }
        dr += 1;
    }
    let w = pixel_area(f32(dl), f32(dl + dr + 1), c0, c1);
    if (prm.w <= 0.0) { return w; }
    return w * corners(p - d * dl, p + d * (dr + 1), n, vert, dl, dr);
}

// the perpendicular edge at the start of pixel `q`
fn cross_edge(q: vec2<i32>, vert: bool) -> f32 {
    let e = edge(q);
    return select(e.x, e.y, vert);
}

// Corner rounding (reference SMAADetectHorizontal/VerticalCornerPattern): at the nearer end of
// the line (both, halved, when it sits in the middle), a perpendicular edge that runs on one
// pixel past the line is a real corner and not a step, so the blend on that side is cut by
// prm.w and the corner stays sharp. `q0` and `q1` are where the end edges are read.
fn corners(q0: vec2<i32>, q1: vec2<i32>, n: vec2<i32>, vert: bool, dl: i32, dr: i32) -> vec2<f32> {
    var near = vec2<f32>(f32(dl <= dr), f32(dr <= dl));
    near *= prm.w / (near.x + near.y);
    let pix = near.x * cross_edge(q0 + n, vert) + near.y * cross_edge(q1 + n, vert);
    let oth = near.x * cross_edge(q0 - 2 * n, vert) + near.y * cross_edge(q1 - 2 * n, vert);
    return saturate(vec2<f32>(1.0) - vec2<f32>(pix, oth));
}

// x: weight of the pixel towards its top, y: of the pixel above towards this one, z: towards the
// left, w: of the left one towards this one
@fragment
fn fs_weights(@builtin(position) pos: vec4<f32>) -> @location(0) vec4<f32> {
    let p = vec2<i32>(pos.xy);
    let e = edge(p);
    var w = vec4<f32>(0.0);
    if (e.y > 0.5) { w = vec4<f32>(line_weights(p, false), w.zw); }
    if (e.x > 0.5) { w = vec4<f32>(w.xy, line_weights(p, true)); }
    return w;
}

@fragment
fn fs_blend(@builtin(position) pos: vec4<f32>) -> @location(0) vec4<f32> {
    let p = vec2<i32>(pos.xy);
    let c = colour(p);
    let here = textureLoad(weights_t, p, 0);
    var a = vec4<f32>(0.0);
    // right, bottom, left, top
    if (inside(p + vec2<i32>(1, 0))) { a.x = textureLoad(weights_t, p + vec2<i32>(1, 0), 0).a; }
    if (inside(p + vec2<i32>(0, 1))) { a.y = textureLoad(weights_t, p + vec2<i32>(0, 1), 0).g; }
    a.z = here.b;
    a.w = here.r;
    if (dot(a, vec4<f32>(1.0)) < 1e-5) { return vec4<f32>(c, 1.0); }
    // blend along the direction with the larger weight, in proportion
    let horizontal = max(a.x, a.z) > max(a.y, a.w);
    var wa = a.yw;
    var na = colour(p + vec2<i32>(0, 1));
    var nb = colour(p + vec2<i32>(0, -1));
    if (horizontal) {
        wa = a.xz;
        na = colour(p + vec2<i32>(1, 0));
        nb = colour(p + vec2<i32>(-1, 0));
    }
    let sum = wa.x + wa.y;
    let k = wa / sum;
    let out = k.x * mix(c, na, wa.x) + k.y * mix(c, nb, wa.y);
    return vec4<f32>(out, 1.0);
}
