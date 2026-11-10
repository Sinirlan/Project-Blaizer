// REMASTER ambient occlusion (render.rs `Ssao`, 09-remaster 9.28) [design choice, not in the
// original]. GTAO after Jimenez et al. 2016 / XeGTAO, cut down:
// 1. fs_gtao: half resolution, per pixel 2 slices x 6 steps each side over the depth buffer; the
//    normal comes from the depth differences (the world's baked vertex colours have no use here).
//    Output: (visibility, linear depth).
// 2. fs_blur_h / fs_blur_v: depth-aware separable blur of that.
// 3. fs_apply: multiplies the scene (blend Zero x Src) by the visibility.
// DEPTH_TEX is replaced by texture_depth_2d or texture_depth_multisampled_2d (MSAA) at load.

struct U {
    // proj[0][0], proj[1][1], near, far
    p: vec4<f32>,
    // radius (m), fade start (m), fade end (m), strength
    q: vec4<f32>,
    // full-size width, height; z: 1 when the scene is in linear light
    s: vec4<f32>,
    // xy: the projection's lens shift in NDC (render.rs `Camera::shift`, the Cine preview; 0 elsewhere)
    o: vec4<f32>,
}
@group(0) @binding(0) var<uniform> u: U;
@group(0) @binding(1) var depth: DEPTH_TEX;

const PI: f32 = 3.14159265;
const SLICES: i32 = 2;
const STEPS: i32 = 6;

@vertex
fn vs_full(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
    let p = vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u));
    return vec4<f32>(p * 2.0 - 1.0, 0.0, 1.0);
}

fn lin(d: f32) -> f32 {
    return u.p.z * u.p.w / (u.p.w - d * (u.p.w - u.p.z));
}

fn clamp_px(px: vec2<i32>) -> vec2<i32> {
    return clamp(px, vec2<i32>(0), vec2<i32>(u.s.xy) - 1);
}

// view-space position (x right, y up, z away) of a full-size pixel
fn view_pos(px: vec2<i32>) -> vec3<f32> {
    let q = clamp_px(px);
    let z = lin(textureLoad(depth, q, 0));
    let ndc = (vec2<f32>(q) + 0.5) / u.s.xy * vec2<f32>(2.0, -2.0) + vec2<f32>(-1.0, 1.0);
    return vec3<f32>((ndc.x - u.o.x) * z / u.p.x, (ndc.y - u.o.y) * z / u.p.y, z);
}

fn ign(p: vec2<f32>) -> f32 {
    return fract(52.9829189 * fract(dot(p, vec2<f32>(0.06711056, 0.00583715))));
}

fn fast_acos(x: f32) -> f32 {
    return acos(clamp(x, -1.0, 1.0));
}

@fragment
fn fs_gtao(@builtin(position) frag: vec4<f32>) -> @location(0) vec2<f32> {
    let hpx = vec2<i32>(frag.xy);
    let px = hpx * 2;
    let d = textureLoad(depth, clamp_px(px), 0);
    if (d >= 1.0) {
        return vec2<f32>(1.0, u.p.w);
    }
    let pc = view_pos(px);
    // normal from the smaller depth difference on each axis
    let pl = view_pos(px + vec2<i32>(-1, 0));
    let pr = view_pos(px + vec2<i32>(1, 0));
    let pu = view_pos(px + vec2<i32>(0, -1));
    let pd = view_pos(px + vec2<i32>(0, 1));
    let dx = select(pc - pl, pr - pc, abs(pr.z - pc.z) < abs(pl.z - pc.z));
    let dy = select(pc - pu, pd - pc, abs(pd.z - pc.z) < abs(pu.z - pc.z));
    var n = normalize(cross(dx, dy));
    if (dot(n, pc) > 0.0) {
        n = -n;
    }
    let view_vec = normalize(-pc);
    let radius = u.q.x;
    // the radius in full-size pixels
    let radius_px = min(radius * u.p.y * u.s.y * 0.5 / pc.z, 96.0);
    if (radius_px < 2.0) {
        return vec2<f32>(1.0, pc.z);
    }
    let noise = vec2<f32>(ign(frag.xy), ign(frag.xy + vec2<f32>(5.588238, 5.588238)));
    let fall_range = 0.615 * radius;
    let fall_mul = -1.0 / fall_range;
    let fall_add = (radius - fall_range) / fall_range + 1.0;
    var vis = 0.0;
    for (var sl = 0; sl < SLICES; sl++) {
        let phi = (f32(sl) + noise.x) / f32(SLICES) * PI;
        let dir3 = vec3<f32>(cos(phi), sin(phi), 0.0);
        // the same direction in pixels (y points down)
        let dir_px = vec2<f32>(dir3.x, -dir3.y);
        let ortho = dir3 - dot(dir3, view_vec) * view_vec;
        let axis = normalize(cross(dir3, view_vec));
        let proj_n = n - axis * dot(n, axis);
        let proj_len = length(proj_n);
        let sign_n = sign(dot(ortho, proj_n));
        let cos_n = clamp(dot(proj_n, view_vec) / max(proj_len, 1e-5), 0.0, 1.0);
        let n_angle = sign_n * fast_acos(cos_n);
        let low0 = cos(n_angle + 0.5 * PI);
        let low1 = cos(n_angle - 0.5 * PI);
        var hc0 = low0;
        var hc1 = low1;
        for (var k = 0; k < STEPS; k++) {
            var s = (f32(k) + noise.y) / f32(STEPS);
            s = s * s;
            let off = dir_px * (s * radius_px + 1.5);
            let o = vec2<i32>(round(off));
            let d0 = view_pos(px + o) - pc;
            let d1 = view_pos(px - o) - pc;
            let l0 = length(d0);
            let l1 = length(d1);
            let c0 = dot(d0, view_vec) / max(l0, 1e-5);
            let c1 = dot(d1, view_vec) / max(l1, 1e-5);
            hc0 = max(hc0, mix(low0, c0, clamp(l0 * fall_mul + fall_add, 0.0, 1.0)));
            hc1 = max(hc1, mix(low1, c1, clamp(l1 * fall_mul + fall_add, 0.0, 1.0)));
        }
        var h0 = -fast_acos(hc1);
        var h1 = fast_acos(hc0);
        h0 = n_angle + clamp(h0 - n_angle, -0.5 * PI, 0.5 * PI);
        h1 = n_angle + clamp(h1 - n_angle, -0.5 * PI, 0.5 * PI);
        let arc0 = (cos_n + 2.0 * h0 * sin(n_angle) - cos(2.0 * h0 - n_angle)) * 0.25;
        let arc1 = (cos_n + 2.0 * h1 * sin(n_angle) - cos(2.0 * h1 - n_angle)) * 0.25;
        vis += proj_len * (arc0 + arc1);
    }
    vis = clamp(vis / f32(SLICES), 0.0, 1.0);
    // the far view has no pixels to spare for the radius: fade out
    let fade = 1.0 - smoothstep(u.q.y, u.q.z, pc.z);
    return vec2<f32>(mix(1.0, vis, fade), pc.z);
}

@group(0) @binding(2) var src: texture_2d<f32>;

fn blur(frag: vec4<f32>, dir: vec2<i32>) -> vec2<f32> {
    let px = vec2<i32>(frag.xy);
    let hi = vec2<i32>(textureDimensions(src)) - 1;
    let c = textureLoad(src, px, 0).xy;
    var sum = c.x;
    var wsum = 1.0;
    for (var k = 1; k <= 4; k++) {
        for (var side = 0; side < 2; side++) {
            let o = dir * select(-k, k, side == 1);
            let t = textureLoad(src, clamp(px + o, vec2<i32>(0), hi), 0).xy;
            // depth-aware: neighbours at another depth do not count
            let w = max(0.0, 1.0 - abs(t.y - c.y) / (0.04 * c.y + 0.05)) * (1.0 - f32(k) / 5.0);
            sum += t.x * w;
            wsum += w;
        }
    }
    return vec2<f32>(sum / wsum, c.y);
}

@fragment
fn fs_blur_h(@builtin(position) frag: vec4<f32>) -> @location(0) vec2<f32> {
    return blur(frag, vec2<i32>(1, 0));
}

@fragment
fn fs_blur_v(@builtin(position) frag: vec4<f32>) -> @location(0) vec2<f32> {
    return blur(frag, vec2<i32>(0, 1));
}

@group(0) @binding(3) var samp: sampler;

@fragment
fn fs_apply(@builtin(position) frag: vec4<f32>) -> @location(0) vec4<f32> {
    let ao = textureSampleLevel(src, samp, frag.xy / u.s.xy, 0.0).x;
    let k = 1.0 - u.q.w * (1.0 - ao);
    // gamma-space scenes get the linear factor encoded
    return vec4<f32>(vec3<f32>(select(pow(k, 1.0 / 2.2), k, u.s.z > 0.5)), 1.0);
}
