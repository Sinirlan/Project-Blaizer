// 2D layer: canvas-pixel quads from the UI atlas, alpha blended, optional ellipse clip.
struct UiGlobals {
    // x: scale x, y/z: offset (window px), w: scale y (differs from x for a stretched canvas)
    xform: vec4<f32>,
    // xy: window size
    screen: vec4<f32>,
};

@group(0) @binding(0) var<uniform> u: UiGlobals;
@group(0) @binding(1) var atlas: texture_2d<f32>;
@group(0) @binding(2) var samp: sampler;

struct VsIn {
    @location(0) pos: vec2<f32>,
    @location(1) uv: vec2<f32>,
    @location(2) color: vec4<f32>,
    @location(3) clip: vec4<f32>,
};

struct VsOut {
    @builtin(position) clip_pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) color: vec4<f32>,
    @location(2) canvas: vec2<f32>,
    @location(3) ellipse: vec4<f32>,
};

@vertex
fn vs_ui(v: VsIn) -> VsOut {
    var o: VsOut;
    let px = v.pos * vec2<f32>(u.xform.x, u.xform.w) + u.xform.yz;
    o.clip_pos = vec4<f32>(px.x / u.screen.x * 2.0 - 1.0, 1.0 - px.y / u.screen.y * 2.0, 0.0, 1.0);
    o.uv = v.uv;
    o.color = v.color;
    o.canvas = v.pos;
    o.ellipse = v.clip;
    return o;
}

@fragment
fn fs_ui(i: VsOut) -> @location(0) vec4<f32> {
    let t = textureSample(atlas, samp, i.uv);
    // ellipse clip: positive radii keep the inside, negative radii keep the outside
    if (i.ellipse.z != 0.0) {
        let d = (i.canvas - i.ellipse.xy) / abs(i.ellipse.zw);
        let inside = dot(d, d) <= 1.0;
        if (inside != (i.ellipse.z > 0.0)) {
            discard;
        }
    }
    let c = t * i.color;
    if (c.a < 0.01) {
        discard;
    }
    return c;
}

// REMASTER sharp HUD (09-remaster 9.12): distance-field quads next to the atlas quads.
// The field atlas: R = silhouette, G = fill, 0.5 + d / (2 SDF_RANGE), d in source pixels.
@group(0) @binding(3) var sdf: texture_2d<f32>;
const SDF_RANGE: f32 = 3.0; // ui_sdf::RANGE

struct VsInR {
    @location(0) pos: vec2<f32>,
    @location(1) uv: vec2<f32>,
    @location(2) color: vec4<f32>,
    @location(3) clip: vec4<f32>,
    @location(4) aux: vec4<f32>,
};

struct VsOutR {
    @builtin(position) clip_pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) color: vec4<f32>,
    @location(2) canvas: vec2<f32>,
    @location(3) ellipse: vec4<f32>,
    @location(4) aux: vec4<f32>,
};

@vertex
fn vs_ui_remaster(v: VsInR) -> VsOutR {
    var o: VsOutR;
    let px = v.pos * vec2<f32>(u.xform.x, u.xform.w) + u.xform.yz;
    o.clip_pos = vec4<f32>(px.x / u.screen.x * 2.0 - 1.0, 1.0 - px.y / u.screen.y * 2.0, 0.0, 1.0);
    o.uv = v.uv;
    o.color = v.color;
    o.canvas = v.pos;
    o.ellipse = v.clip;
    o.aux = v.aux;
    return o;
}

// REMASTER box-filtered image (`Batch::image_filtered`, the countdown art; was the generated minimap,
// 09-remaster 9.21 phase 5): 4 x 4 taps over the pixel's footprint, premultiplied (the atlas has no
// mips; the image is drawn smaller than its texels), clamped to the image's uv rect
fn box_taps(uv: vec2<f32>, fw: vec2<f32>, rect: vec4<f32>) -> vec4<f32> {
    var acc = vec4<f32>(0.0);
    for (var j = 0; j < 4; j++) {
        for (var k = 0; k < 4; k++) {
            let o = (vec2<f32>(f32(j), f32(k)) + 0.5) / 4.0 - 0.5;
            let p = uv + o * fw;
            let s = textureSampleLevel(atlas, samp, clamp(p, rect.xy, rect.zw), 0.0);
            acc += vec4<f32>(s.rgb * s.a, s.a);
        }
    }
    return acc / 16.0;
}

fn box_filtered(uv: vec2<f32>, fw: vec2<f32>, rect: vec4<f32>, tint: vec4<f32>) -> vec4<f32> {
    let acc = box_taps(uv, fw, rect);
    if (acc.a < 0.004) {
        discard;
    }
    return vec4<f32>(acc.rgb / acc.a, acc.a) * tint;
}

// edge coverage of a signed distance (inside positive), one window pixel wide
fn edge(d: f32) -> f32 {
    return clamp(d / max(fwidth(d), 1e-4) + 0.5, 0.0, 1.0);
}

@fragment
fn fs_ui_remaster(i: VsOutR) -> @location(0) vec4<f32> {
    // both samples and the derivatives in uniform control flow
    let t = textureSample(atlas, samp, i.uv);
    let d = (textureSample(sdf, samp, i.uv).rg - 0.5) * (2.0 * SDF_RANGE);
    let uv_px = length(fwidth(i.uv)) * 0.7071;
    let outer = edge(d.x);
    let inner = edge(d.y);
    // modern HUD font glyph (aux.w = 2 + outline width): the glyph is the fill, the outline
    // grows outside it
    let grown = edge(d.x + max(i.aux.w - 2.0, 0.0));
    let fw = fwidth(i.uv);
    // modern menu vector mesh (aux.w = -6, `Batch::mesh`): flat colour, dithered. The noise is
    // scaled by 1 / alpha so it still reaches the 8-bit result after blending.
    if (i.aux.w < -5.5) {
        let p = i.clip_pos.xy;
        let h1 = fract(sin(dot(p, vec2<f32>(12.9898, 78.233))) * 43758.5453);
        let h2 = fract(sin(dot(p, vec2<f32>(39.346, 11.135))) * 24634.6345);
        let amp = min((1.0 / 255.0) / max(i.color.a, 0.02), 0.3);
        return vec4<f32>(i.color.rgb + (h1 + h2 - 1.0) * amp, i.color.a);
    }
    // proximity arrow slice (aux.w = -5): the field's silhouette in the rev meter's glass
    if (i.aux.w < -4.5) {
        return prox_arrow(i.uv, d.x, outer, i.color, i.ellipse.xyz);
    }
    // box-filtered image (aux.w = -2): box filter of the footprint, taps inside `ellipse`
    // (the image's uv rect)
    if (i.aux.w < -1.5) {
        return box_filtered(i.uv, fw, i.ellipse, i.color);
    }
    // the glass rev meter (aux.w = -1): uv = Blender units, its own tune in color/ellipse
    if (i.aux.w < -0.5) {
        return rev_meter(i.uv, uv_px, i.aux.x, i.aux.yz, i.color, i.ellipse);
    }
    if (i.ellipse.z != 0.0) {
        let e = (i.canvas - i.ellipse.xy) / abs(i.ellipse.zw);
        if ((dot(e, e) <= 1.0) != (i.ellipse.z > 0.0)) {
            discard;
        }
    }
    var c = t * i.color;
    if (i.aux.w > 1.5) {
        c = vec4<f32>(mix(i.aux.rgb, i.color.rgb, outer), i.color.a * grown);
    } else if (i.aux.w > 0.5) {
        // outline colour inside the silhouette, fill colour inside the fill
        c = vec4<f32>(mix(i.aux.rgb, i.color.rgb, inner), i.color.a * outer);
    }
    if (c.a < 0.01) {
        discard;
    }
    return c;
}

// REMASTER glass rev meter (09-remaster 9.21 phase 4; our art from RevMeter_750.blend, design
// choice). The tube is a stroke of varying radius along REV_SPINE; its camera-facing wall is
// a superellipse depth (1 - s^p)^(1/p) over the lateral offset s, lit like the Blender
// scene: emissive fill up to a vertical cut at the rpm, dark muted glass beyond it,
// reflections of the strip lights and a Fresnel rim, a halo round the lit part. The look
// constants are the `rev` uniform (revmeter.rs `RevLook`), the spine REV_SPINE comes from `wgsl`.
// p = Blender units, px = one window pixel in them, cut = x of the fill's end,
// a = (glow, inner glow, glass alpha, spec), b = (bloom, thickness, depth, roundness).

// the look constants (revmeter.rs `RevLook`, one uniform; the game uses the defaults)
struct RevLook {
    ramp: array<vec4<f32>, 5>,
    // ramp blur, emission, core, spill
    misc: vec4<f32>,
    // facing base, facing gain, muted saturation, muted value
    fm: vec4<f32>,
    // unlit glow, unlit alpha, lit alpha, Fresnel F0
    ua: vec4<f32>,
    // environment colour, share of the Fresnel term that mixes towards it
    env: vec4<f32>,
    key: vec4<f32>,
    rim: vec4<f32>,
    kick_l: vec4<f32>,
    kick_r: vec4<f32>,
    // strip half widths and half lengths (key, rim, kick L, kick R)
    sw: vec4<f32>,
    sl: vec4<f32>,
    // halo strength, radius, colour gain
    halo: vec4<f32>,
}
@group(0) @binding(4) var<uniform> rev: RevLook;

// M_GlassCasing's ColorRamp on `progress`, linear between the stops (xyz colour, w position)
fn rev_ramp_linear(t: f32) -> vec3<f32> {
    let u = clamp(t, 0.0, 1.0);
    var c = rev.ramp[0].xyz;
    for (var k = 1u; k < 5u; k = k + 1u) {
        let s0 = rev.ramp[k - 1u];
        let s1 = rev.ramp[k];
        c = select(c, mix(s0.xyz, s1.xyz, (u - s0.w) / (s1.w - s0.w)), u > s0.w);
    }
    return c;
}

// Blender's B-spline ramp approximates the stops (the tip stays orange-red): a box blur of
// the linear ramp, APPROX
fn rev_ramp(t: f32) -> vec3<f32> {
    return (rev_ramp_linear(t - rev.misc.x) + rev_ramp_linear(t) + rev_ramp_linear(t + rev.misc.x)) / 3.0;
}

// surface normal (x right, y up, z to the viewer) at lateral offset s along dir
fn rev_normal(dir: vec2<f32>, s: f32, depth: f32, p: f32) -> vec3<f32> {
    let sc = clamp(s, 0.0, 0.999);
    let q = pow(sc, p);
    let slope = depth * pow(sc, p - 1.0) * pow(1.0 - q, 1.0 / p - 1.0);
    return normalize(vec3<f32>(dir * slope, 1.0));
}

// a long thin area light seen in the glass: l = direction to it, axis = its long side,
// w = half its angular width, len = half its angular length (rad)
fn rev_strip(n: vec3<f32>, n2: vec3<f32>, l: vec3<f32>, axis: vec3<f32>, w: f32, len: f32) -> f32 {
    let v = vec3<f32>(0.0, 0.0, -1.0);
    let r = reflect(v, n);
    let r2 = reflect(v, n2);
    let pn = normalize(cross(axis, l));
    let d = asin(clamp(dot(r, pn), -1.0, 1.0));
    let d2 = asin(clamp(dot(r2, pn), -1.0, 1.0));
    // the line no thinner than a pixel's footprint (then dimmer)
    let fw = max(w, abs(d2 - d));
    let along = abs(asin(clamp(dot(r, axis), -1.0, 1.0)));
    let line = clamp(1.0 - abs(d) / fw, 0.0, 1.0) * (w / fw);
    return line * (1.0 - smoothstep(len * 0.8, len, along)) * step(0.0, dot(r, l));
}

// `red` (Race line HUD redline, #323): x dims the lit part and its halo, y turns the glass edge red.
fn rev_meter(p: vec2<f32>, px: f32, cut: f32, red: vec2<f32>, a: vec4<f32>, b: vec4<f32>) -> vec4<f32> {
    let thick = b.y;
    // nearest point of the tube (signed distance to its outline) and of its lit part (the
    // spine's x only grows, so the cut clips each segment)
    var dist = 1e9;
    var t = 0.0;
    var off = vec2<f32>(0.0, 1.0);
    var rad = 1.0;
    var lit_d = 1e9;
    var lit_t = 0.0;
    for (var k = 0u; k + 1u < REV_N; k = k + 1u) {
        let s0 = REV_SPINE[k];
        let s1 = REV_SPINE[k + 1u];
        let ba = s1.xy - s0.xy;
        let h = clamp(dot(p - s0.xy, ba) / dot(ba, ba), 0.0, 1.0);
        let o = p - (s0.xy + ba * h);
        let r = mix(s0.w, s1.w, h) * thick;
        let dd = length(o) - r;
        if (dd < dist) {
            dist = dd;
            t = mix(s0.z, s1.z, h);
            off = o;
            rad = r;
        }
        if (s0.x < cut) {
            let h2 = min(h, clamp((cut - s0.x) / max(ba.x, 1e-6), 0.0, 1.0));
            let d2 = length(p - (s0.xy + ba * h2)) - mix(s0.w, s1.w, h2) * thick;
            if (d2 < lit_d) {
                lit_d = d2;
                lit_t = mix(s0.z, s1.z, h2);
            }
        }
    }
    let cover = clamp(0.5 - dist / px, 0.0, 1.0);
    let s = length(off) / rad;
    let dir = select(vec2<f32>(0.0, 1.0), off / max(length(off), 1e-6), length(off) > 1e-6);
    let n = rev_normal(dir, s, b.z, b.w);
    let n2 = rev_normal(dir, s + px / rad, b.z, b.w);
    let facing = 1.0 - n.z;

    // fill: emission x (base + facing) (Math.002), the core brighter than the rim (the
    // preview's yellow), over-bright channels spill into the others like the render's
    // clipped highlights
    let ramp = rev_ramp(t);
    var lit = ramp * (rev.misc.y * a.x) * (rev.fm.x + rev.fm.y * facing) * (1.0 + rev.misc.z * (1.0 - s * s));
    lit = min(lit + max(max(max(lit.r, lit.g), lit.b) - 1.0, 0.0) * rev.misc.w, vec3<f32>(1.0));
    // unlit: the ramp through the HSV node plus UnlitGlow
    let hi = max(max(ramp.r, ramp.g), ramp.b);
    let muted = (hi - (hi - ramp) * rev.fm.z) * rev.fm.w;
    let unlit = muted + ramp * (rev.ua.x * a.y);
    // vertical cut, antialiased over a pixel
    let on = clamp((cut - p.x) / px + 0.5, 0.0, 1.0);
    var col = mix(unlit, lit, on);
    var alpha = mix(rev.ua.y * a.z, rev.ua.z * (1.0 - red.x), on);

    // glass: Fresnel rim and the strip lights (xyz direction to it, w strength)
    let fres = (rev.ua.w + (1.0 - rev.ua.w) * pow(facing, 5.0)) * a.w;
    col = mix(col, mix(rev.env.xyz, vec3<f32>(1.0, 0.31, 0.24), red.y), clamp(fres * rev.env.w, 0.0, 1.0));
    alpha = alpha + (1.0 - alpha) * clamp(fres, 0.0, 1.0);
    let x = vec3<f32>(1.0, 0.0, 0.0);
    let y = vec3<f32>(0.0, 1.0, 0.0);
    let spec = a.w * (rev.key.w * rev_strip(n, n2, rev.key.xyz, x, rev.sw.x, rev.sl.x)
        + rev.rim.w * rev_strip(n, n2, rev.rim.xyz, x, rev.sw.y, rev.sl.y)
        + rev.kick_l.w * rev_strip(n, n2, rev.kick_l.xyz, y, rev.sw.z, rev.sl.z)
        + rev.kick_r.w * rev_strip(n, n2, rev.kick_r.xyz, y, rev.sw.w, rev.sl.w));
    col = min(col + vec3<f32>(spec), vec3<f32>(1.0));
    alpha = max(alpha, min(spec, 1.0));

    // halo round the lit part, outside the tube (the glow is hidden from the glass, as in
    // the Blender material); premultiplied composite under the tube
    let body = alpha * cover;
    let halo = select(0.0, b.x * rev.halo.x * exp(-max(lit_d, 0.0) / rev.halo.y), lit_d < 1e8) * (1.0 - cover) * (1.0 - red.x);
    let halo_col = min(rev_ramp(lit_t) * rev.halo.z, vec3<f32>(1.0));
    let out_a = min(body + halo, 1.0);
    if (out_a < 0.004) {
        discard;
    }
    return vec4<f32>((col * body + halo_col * halo) / max(body + halo, 1e-4), out_a);
}

// REMASTER proximity arrow (09-remaster 9.21 phase 7; design choice): one slice of Overload's
// `»` extruded (`Batch::glass_glyph`), in the rev meter's glass: emission on its colour ramp
// x (base + facing), a Fresnel rim to PROX_ENV and a key light's highlight. The top slice has
// a bevel PROX_BEVEL source px wide round its edge (normal from the field's gradient); the
// slices under it are the side walls, darker towards the ground; the bottom one also has a
// soft glow round it like the rev meter's halo, weaker.
// d = signed distance (source px, inside positive), cover = its edge coverage,
// a = (ramp place, alpha, layer 0..1, -), v = direction to the eye (glyph frame, z up)
const PROX_BEVEL: f32 = 0.9;
const PROX_SLOPE: f32 = 1.4;
const PROX_KEY: vec3<f32> = vec3<f32>(-0.35, -0.45, 0.82);
// The arrows' own look, no longer read from `rev` (the rev meter's tuning): the values start
// at the rev meter's (revmeter.rs EMISSION, FACING, CORE, SPILL, ALPHA, F0, ENV, ENV_MIX,
// HALO_*); tune them here. Only the colour ramp (`rev_ramp`) is still shared.
const PROX_EMISSION: f32 = 4.125;
// lit emission x (base + gain x facing); the flat face gets the core boost, the bevel none
const PROX_FACING: vec2<f32> = vec2<f32>(1.9, 1.6);
const PROX_CORE: f32 = 1.2;
// share of an over-bright channel spilled into the others
const PROX_SPILL: f32 = 0.35;
const PROX_ALPHA: f32 = 0.55;
// Fresnel F0, the environment the rim reflects and how much of the Fresnel term mixes to it
const PROX_F0: f32 = 0.04;
const PROX_ENV: vec3<f32> = vec3<f32>(0.70, 0.72, 0.76);
const PROX_ENV_MIX: f32 = 0.8;
// the glow round the bottom slice, on the road under the arrow: strength, falloff (source px;
// it ends inside the field's range, SDF_RANGE) and colour gain over the ramp
const PROX_HALO: f32 = 0.0;
const PROX_HALO_R: f32 = 1.3;
const PROX_HALO_GAIN: f32 = 1.3;
// menu labels (electric blue, same shader): the dimmer emission and weaker glow they had
const PROX_BLUE: vec3<f32> = vec3<f32>(0.12, 0.5, 1.0);
const PROX_LABEL_DIM: f32 = 0.8;
const PROX_LABEL_HALO: f32 = 0.5;

fn prox_arrow(uv: vec2<f32>, d: f32, cover: f32, a: vec4<f32>, v_in: vec3<f32>) -> vec4<f32> {
    let v = normalize(v_in);
    // a.w = 1: menu labels, electric blue instead of the ramp
    let ramp = select(rev_ramp(a.x), PROX_BLUE, a.w > 0.5);
    var col: vec3<f32>;
    var alpha: f32;
    if (a.z > 0.999) {
        // the field's gradient per source px (TEXELS = 4 texels each), central differences
        let ts = 1.0 / vec2<f32>(textureDimensions(sdf));
        let k = 2.0 * SDF_RANGE / 0.5;
        let gx = (textureSampleLevel(sdf, samp, uv + vec2<f32>(ts.x, 0.0), 0.0).r - textureSampleLevel(sdf, samp, uv - vec2<f32>(ts.x, 0.0), 0.0).r) * k;
        let gy = (textureSampleLevel(sdf, samp, uv + vec2<f32>(0.0, ts.y), 0.0).r - textureSampleLevel(sdf, samp, uv - vec2<f32>(0.0, ts.y), 0.0).r) * k;
        let bevel = 1.0 - smoothstep(0.0, PROX_BEVEL, d);
        let n = normalize(vec3<f32>(-vec2<f32>(gx, gy) * PROX_SLOPE * bevel, 1.0));
        let facing = 1.0 - max(dot(n, v), 0.0);
        // emission with the core boost (the flat face is the core, the bevel the rim)
        let core = select(1.0 + PROX_CORE * (1.0 - bevel * bevel), PROX_LABEL_DIM, a.w > 0.5);
        col = ramp * PROX_EMISSION * (PROX_FACING.x + PROX_FACING.y * facing) * core;
        col = min(col + max(max(max(col.r, col.g), col.b) - 1.0, 0.0) * PROX_SPILL, vec3<f32>(1.0));
        let fres = PROX_F0 + (1.0 - PROX_F0) * pow(facing, 5.0);
        col = mix(col, PROX_ENV, clamp(fres * PROX_ENV_MIX, 0.0, 1.0));
        let r = reflect(-v, n);
        let spec = pow(max(dot(r, normalize(PROX_KEY)), 0.0), 24.0) + 0.6 * pow(max(dot(r, vec3<f32>(0.0, 0.0, 1.0)), 0.0), 80.0);
        col = min(col + vec3<f32>(spec), vec3<f32>(1.0));
        alpha = min(PROX_ALPHA + fres + spec, 1.0);
        // menu labels: lifted so the blue reads on the dark backdrop
        if (a.w > 0.5) {
            col = min(col * vec3<f32>(1.2, 2.0, 3.0) + vec3<f32>(0.03, 0.1, 0.3), vec3<f32>(1.0));
            alpha = max(alpha, 0.9);
        }
    } else {
        col = ramp * (0.3 + 0.35 * a.z);
        alpha = PROX_ALPHA;
    }
    var body = alpha * cover;
    var halo = 0.0;
    if (a.z < 0.001) {
        let out_d = max(-d, 0.0);
        halo = select(PROX_HALO, PROX_LABEL_HALO, a.w > 0.5) * exp(-out_d / PROX_HALO_R) * (1.0 - smoothstep(SDF_RANGE - 1.0, SDF_RANGE - 0.1, out_d)) * (1.0 - cover);
    }
    let halo_col = min(ramp * PROX_HALO_GAIN, vec3<f32>(1.0));
    let sum = body + halo;
    let out_a = min(sum, 1.0) * a.y;
    if (out_a < 0.004) {
        discard;
    }
    return vec4<f32>((col * body + halo_col * halo) / max(sum, 1e-4), out_a);
}
