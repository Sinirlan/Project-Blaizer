// REMASTER volumetric fog (render.rs `VolFog`, 17-fog-rework.md appendix) [design choice, not in the original].
// 1. cs_inject: one thread per froxel of a camera-aligned grid samples the medium at a point jittered
//    inside the froxel (Halton, one per frame) and writes (in-scatter, extinction per metre of view depth).
//    Density = the Remaster fog's height haze (17-fog-rework.md), in-scatter = the fog colour plus
//    the Henyey-Greenstein sun glow (HDR, linear), plus the street lamps and the car headlights.
// 1b. cs_integrate: one thread per screen-aligned column blends each froxel with the previous frame's
//    (reprojected through the last unjittered view-projection, clamped to the froxel's neighbourhood),
//    keeps the blend as the next history, and marches the slices front to back, writing (in-scatter,
//    transmittance) accumulated up to the far side of each slice.
// 2. fs_apply: a full-screen pass over the opaque scene (blend: scene * T + S) that reads the
//    depth, looks the volume up at that distance and adds the distance ramp with its clip guard
//    (the same model as shader.wgsl `fog_remaster`); the sky (depth 1) gets `fog_sky`'s amount.
// DEPTH_TEX is replaced by texture_depth_2d or texture_depth_multisampled_2d (MSAA) at load.

struct U {
    // clip -> world
    inv_vp: mat4x4<f32>,
    cam: vec4<f32>,
    // unit view forward
    fwd: vec4<f32>,
    // CueColour
    fog: vec4<f32>,
    // sun colour * intensity
    sun: vec4<f32>,
    // towards the sun
    sun_dir: vec4<f32>,
    // Globals.fog2: density per metre at the ground, ground height, 1 / height scale, glow
    fog2: vec4<f32>,
    // x: glow anisotropy (HG g)
    fog3: vec4<f32>,
    // rgb: fog colour (linear), w: sky haze
    fog4: vec4<f32>,
    // start, full (m), opacity, curve
    fog5: vec4<f32>,
    // proj[0][0], proj[1][1], near, far (the depth buffer's)
    proj: vec4<f32>,
    // volume near, volume far, slices, columns' x size
    vol: vec4<f32>,
    // output width, height
    scr: vec4<f32>,
    // x: lamps in use, y, z: cosine of the lamp cone's inner and outer half angle, w: scatter gain
    lamp_n: vec4<f32>,
    // street lamps: (position, range), (rgb, shadow layer)
    lamps: array<vec4<f32>, 32>,
    // x: headlights in use, w: scatter gain
    spot_n: vec4<f32>,
    // car headlights: (position, range), (rgb, 0), (beam axis, 0), (up, 0)
    spots: array<vec4<f32>, 128>,
    // world -> clip of the previous frame (unjittered)
    prev_vp: mat4x4<f32>,
    // jitter inside the froxel (x, y, slice), history weight (0: no history)
    jit: vec4<f32>,
    // world -> clip of the fog shadow map (the sun's view of the world)
    shaft_vp: mat4x4<f32>,
    // x: sun shafts on, y: depth bias (map units), z: how much the haze darkens in the sun's shadow
    shaft: vec4<f32>,
}
@group(0) @binding(0) var<uniform> u: U;

const GAMMA: f32 = 2.2;

fn lin3(c: vec3<f32>) -> vec3<f32> {
    return pow(max(c, vec3<f32>(0.0)), vec3<f32>(GAMMA));
}

// shader.wgsl hg_phase / fog_colour / fog_sky_amount, over `u`
fn hg_phase(cos_t: f32, g_: f32) -> f32 {
    let k = max(1.0 + g_ * g_ - 2.0 * g_ * cos_t, 1e-4);
    let a = 1.0 - g_;
    return a * a * a / (k * sqrt(k));
}

fn fog_colour(dir: vec3<f32>) -> vec3<f32> {
    return u.fog4.rgb + lin3(u.sun.rgb) * (hg_phase(dot(dir, u.sun_dir.xyz), u.fog3.x) * u.fog2.w);
}

// the distance ramp and the clip guard (no haze: the volume holds it)
fn fog_dist(d: f32) -> f32 {
    let t = clamp((d - u.fog5.x) / max(u.fog5.y - u.fog5.x, 1e-3), 0.0, 1.0);
    let guard = smoothstep(0.9 * u.proj.w, u.proj.w, d);
    return 1.0 - (1.0 - u.fog5.z * pow(t, u.fog5.w)) * (1.0 - guard);
}

fn fog_sky_amount(dir: vec3<f32>) -> f32 {
    let w = 1.0 - smoothstep(0.0, max(u.fog4.w, 1e-3) * 0.5, dir.y);
    let k = u.fog2.z;
    let rho = u.fog2.x * exp(clamp(-(u.cam.y - u.fog2.y) * k, -20.0, 2.0));
    let haze = 1.0 - exp(-min(rho / (k * max(dir.y, 0.02)), 20.0));
    let below = 1.0 - smoothstep(-0.02, 0.0, dir.y);
    return max(1.0 - (1.0 - u.fog5.z * w) * (1.0 - haze), below);
}

// view distance of slice boundary k (exponential)
fn slice_z(k: f32) -> f32 {
    return u.vol.x * pow(u.vol.y / u.vol.x, k / u.vol.z);
}

// shader.wgsl `headlight` (REMASTER low-beam cookie), x right, y up: tan of the angle off the beam axis
fn headlight(x: f32, y: f32) -> f32 {
    let yc = y + 0.05;
    let e = (x * x) / (0.6 * 0.6) + (yc * yc) / (0.25 * 0.25);
    let pool = 1.0 - smoothstep(0.4, 1.0, e);
    let hot = exp(-(x * x) / 0.05 - (yc * yc) / 0.0015);
    let cut = mix(0.12, 1.0, 1.0 - smoothstep(-0.02, 0.03, y));
    return (0.5 * pool + 0.8 * hot) * cut;
}

// ---- inject ----------------------------------------------------------------------------------
@group(0) @binding(5) var out_scatter: texture_storage_3d<rgba16float, write>;
@group(0) @binding(9) var shaft_map: texture_depth_2d;
@group(0) @binding(10) var shaft_samp: sampler_comparison;

// how much of the sun reaches `p` (1 = all): one bilinear compare tap of the fog shadow map (the temporal filter
// smooths the rest), lit outside the map and fading in over its edge
fn shaft_vis(p: vec3<f32>) -> f32 {
    let lp = u.shaft_vp * vec4<f32>(p, 1.0);
    let uv = vec2<f32>(lp.x * 0.5 + 0.5, 0.5 - lp.y * 0.5);
    let lit = textureSampleCompareLevel(shaft_map, shaft_samp, uv, lp.z - u.shaft.y);
    let e = min(min(uv.x, 1.0 - uv.x), min(uv.y, 1.0 - uv.y));
    let inside = smoothstep(0.0, 0.06, e) * select(0.0, 1.0, lp.z > 0.0 && lp.z < 1.0);
    return mix(1.0, lit, inside);
}

@compute @workgroup_size(8, 8, 1)
fn cs_inject(@builtin(global_invocation_id) id: vec3<u32>) {
    let dims = textureDimensions(out_scatter);
    if (id.x >= dims.x || id.y >= dims.y || id.z >= dims.z) {
        return;
    }
    let ndc = (vec2<f32>(id.xy) + u.jit.xy) / vec2<f32>(dims.xy) * vec2<f32>(2.0, -2.0) + vec2<f32>(-1.0, 1.0);
    let a = u.inv_vp * vec4<f32>(ndc, 0.0, 1.0);
    let b = u.inv_vp * vec4<f32>(ndc, 1.0, 1.0);
    let dir = normalize(b.xyz / b.w - a.xyz / a.w);
    // the ray's length per metre of view depth
    let per_z = 1.0 / max(dot(dir, u.fwd.xyz), 0.05);
    // slice k + 0.5 is the geometric mean of its two boundaries
    let zm = slice_z(f32(id.z) + u.jit.z);
    let p = u.cam.xyz + dir * zm * per_z;
    let rho = u.fog2.x * exp(clamp(-(p.y - u.fog2.y) * u.fog2.z, -20.0, 4.0));
    // the street lamps' downward cones, unshadowed (the lamp atlas holds the cars only)
    var scatter = fog_colour(dir);
    if (u.shaft.x > 0.5) {
        // sun shafts: in the sun's shadow the haze loses the glow and `shaft.z` of its own light
        let vis = shaft_vis(p);
        scatter = u.fog4.rgb * (1.0 - u.shaft.z * (1.0 - vis)) + (scatter - u.fog4.rgb) * vis;
    }
    let lamp_count = u32(u.lamp_n.x + 0.5);
    for (var i = 0u; i < lamp_count; i++) {
        let a = u.lamps[2u * i];
        let d = p - a.xyz;
        let dist = length(d);
        if (dist < a.w && dist > 1e-3) {
            let cone = smoothstep(u.lamp_n.z, u.lamp_n.y, -d.y / dist);
            let f = 1.0 - dist / a.w;
            scatter += lin3(u.lamps[2u * i + 1u].rgb) * cone * f * f * u.lamp_n.w;
        }
    }
    // the car headlights: the surfaces' low-beam cookie, unshadowed like the lamp cones [APPROX]
    let head_count = u32(u.spot_n.x + 0.5);
    for (var i = 0u; i < head_count; i++) {
        let a = u.spots[4u * i];
        let d = p - a.xyz;
        let dist = length(d);
        if (dist < a.w && dist > 1e-3) {
            let l = d / dist;
            let axis = u.spots[4u * i + 2u].xyz;
            let z = dot(l, axis);
            if (z > 0.1) {
                let up = u.spots[4u * i + 3u].xyz;
                let x = dot(l, cross(axis, up)) / z;
                let y = dot(l, up) / z;
                let f = 1.0 - dist / a.w;
                scatter += lin3(u.spots[4u * i + 1u].rgb) * headlight(x, y) * f * f * u.spot_n.w;
            }
        }
    }
    textureStore(out_scatter, id, vec4<f32>(scatter, rho * per_z));
}

// ---- integrate -------------------------------------------------------------------------------
@group(0) @binding(3) var samp: sampler;
@group(0) @binding(4) var out_vol: texture_storage_3d<rgba16float, write>;
@group(0) @binding(6) var cur_in: texture_3d<f32>;
@group(0) @binding(7) var hist_in: texture_3d<f32>;
@group(0) @binding(8) var hist_out: texture_storage_3d<rgba16float, write>;

@compute @workgroup_size(8, 8, 1)
fn cs_integrate(@builtin(global_invocation_id) id: vec3<u32>) {
    let dims = textureDimensions(hist_out);
    if (id.x >= dims.x || id.y >= dims.y) {
        return;
    }
    let ndc = (vec2<f32>(id.xy) + 0.5) / vec2<f32>(dims.xy) * vec2<f32>(2.0, -2.0) + vec2<f32>(-1.0, 1.0);
    let a = u.inv_vp * vec4<f32>(ndc, 0.0, 1.0);
    let b = u.inv_vp * vec4<f32>(ndc, 1.0, 1.0);
    let dir = normalize(b.xyz / b.w - a.xyz / a.w);
    let per_z = 1.0 / max(dot(dir, u.fwd.xyz), 0.05);
    let hi = vec3<i32>(dims) - vec3<i32>(1);
    let log_range = log(u.vol.y / u.vol.x);
    var s = vec3<f32>(0.0);
    var t = 1.0;
    for (var k = 0u; k < dims.z; k++) {
        let c = vec3<i32>(i32(id.x), i32(id.y), i32(k));
        let cur = textureLoad(cur_in, c, 0);
        var sm = cur;
        if (u.jit.w > 0.0) {
            // the froxel's own centre, one frame ago
            let pc = u.cam.xyz + dir * slice_z(f32(k) + 0.5) * per_z;
            let pv = u.prev_vp * vec4<f32>(pc, 1.0);
            if (pv.w > u.vol.x) {
                let puv = pv.xy / pv.w * vec2<f32>(0.5, -0.5) + vec2<f32>(0.5);
                let pw = log(pv.w / u.vol.x) / log_range;
                if (all(puv > vec2<f32>(0.0)) && all(puv < vec2<f32>(1.0)) && pw > 0.0 && pw < 1.0) {
                    // clamp the history to the neighbourhood of the new sample: moving lights do not smear
                    var mn = cur;
                    var mx = cur;
                    for (var n = 0; n < 6; n++) {
                        var o = vec3<i32>(0);
                        o[n / 2] = 1 - 2 * (n % 2);
                        let v = textureLoad(cur_in, clamp(c + o, vec3<i32>(0), hi), 0);
                        mn = min(mn, v);
                        mx = max(mx, v);
                    }
                    let h = textureSampleLevel(hist_in, samp, vec3<f32>(puv, pw), 0.0);
                    sm = mix(cur, clamp(h, mn, mx), u.jit.w);
                }
            }
        }
        textureStore(hist_out, vec3<u32>(id.x, id.y, k), sm);
        let tr = exp(-sm.a * (slice_z(f32(k + 1u)) - slice_z(f32(k))));
        s += t * (1.0 - tr) * sm.rgb;
        t *= tr;
        textureStore(out_vol, vec3<u32>(id.x, id.y, k), vec4<f32>(s, t));
    }
}

// ---- apply -----------------------------------------------------------------------------------
@group(0) @binding(1) var depth: DEPTH_TEX;
@group(0) @binding(2) var vol: texture_3d<f32>;
@vertex
fn vs_full(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
    let p = vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u));
    return vec4<f32>(p * 2.0 - 1.0, 0.0, 1.0);
}

@fragment
fn fs_apply(@builtin(position) frag: vec4<f32>) -> @location(0) vec4<f32> {
    let px = vec2<i32>(frag.xy);
    let d = textureLoad(depth, px, 0);
    let uv = frag.xy / u.scr.xy;
    let ndc = uv * vec2<f32>(2.0, -2.0) + vec2<f32>(-1.0, 1.0);
    let a = u.inv_vp * vec4<f32>(ndc, 0.0, 1.0);
    let b = u.inv_vp * vec4<f32>(ndc, 1.0, 1.0);
    let dir = normalize(b.xyz / b.w - a.xyz / a.w);
    if (d >= 1.0) {
        // the sky: fogged by direction (shader.wgsl fog_sky skips it in a volumetric view)
        let f = fog_sky_amount(dir);
        return vec4<f32>(fog_colour(dir) * f, 1.0 - f);
    }
    let z = u.proj.z * u.proj.w / (u.proj.w - d * (u.proj.w - u.proj.z));
    let dist = length(vec3<f32>(ndc.x * z / u.proj.x, ndc.y * z / u.proj.y, z));
    // slice coordinate: texel k holds the far side of slice k+1, so 0 (the near plane) is the
    // implicit (no scatter, full transmittance) texel before it
    let w = clamp(log(z / u.vol.x) / log(u.vol.y / u.vol.x), 0.0, 1.0) * u.vol.z;
    let tex = textureSampleLevel(vol, samp, vec3<f32>(uv, max(w - 0.5, 0.0) / u.vol.z), 0.0);
    let near = vec4<f32>(0.0, 0.0, 0.0, 1.0);
    let v = mix(near, tex, clamp(w, 0.0, 1.0));
    // the distance ramp (and the clip guard) behind the volume: scene * T * (1 - f) + S * (1 - f) + colour * f
    let f = fog_dist(dist);
    return vec4<f32>(v.rgb * (1.0 - f) + fog_colour(dir) * f, v.a * (1.0 - f));
}
