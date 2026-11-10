// REMASTER cine depth of field (#421, render.rs `Gpu::dof_pass`, 20-cine "Depth of field") [design choice, not in the
// original; motorview's cine captures only, `Frame::dof`].
// 1. fs_coc: the scene and its depth into one RGBA16F target: rgb = the colour in linear light, a = the signed circle of
//    confusion radius in pixels (render.rs `dof_coc_scale`: k (z - s) / z, near < 0), held to the largest blur.
// 2. fs_dof: a single-pass scatter-as-gather on a golden-angle spiral (Gustafsson 2018, "Bokeh depth of field in a
//    single pass"): a tap counts where its own blur reaches the centre. A tap behind the centre has its blur held to
//    twice the centre's, so a sharp car is not smeared over by the background, while a blurred foreground spreads over
//    what is behind it. Written back into the scene (its alpha kept by the pipeline's blend).
// DEPTH_TEX is replaced by texture_depth_2d or texture_depth_multisampled_2d (MSAA: sample 0) at load.

struct DofU {
    // x: CoC scale k (diameter in pixels at z = infinity), y: focus distance s (m), z: largest blur radius (px), w: near
    a: vec4<f32>,
    // x: far plane, y: the colour's exponent in the scene (2.2 gamma space, 1 linear HDR), z: spiral radius step (px)
    b: vec4<f32>,
}

@group(0) @binding(0) var src: texture_2d<f32>;
@group(0) @binding(1) var depth: DEPTH_TEX;
@group(0) @binding(2) var<uniform> u: DofU;

const GOLDEN_ANGLE: f32 = 2.39996323;

@vertex
fn vs_dof(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
    let uv = vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u));
    return vec4<f32>(uv * 2.0 - 1.0, 0.0, 1.0);
}

@fragment
fn fs_coc(@builtin(position) p: vec4<f32>) -> @location(0) vec4<f32> {
    let px = vec2<i32>(p.xy);
    let c = textureLoad(src, px, 0);
    let near = u.a.w;
    let far = u.b.x;
    let z = near * far / (far - textureLoad(depth, px, 0) * (far - near));
    let coc = clamp(0.5 * u.a.x * (z - u.a.y) / z, -u.a.z, u.a.z);
    return vec4<f32>(pow(max(c.rgb, vec3<f32>(0.0)), vec3<f32>(u.b.y)), coc);
}

@fragment
fn fs_dof(@builtin(position) p: vec4<f32>) -> @location(0) vec4<f32> {
    let last = vec2<i32>(textureDimensions(src)) - 1;
    let px = vec2<i32>(p.xy);
    let c0 = textureLoad(src, px, 0);
    let size0 = abs(c0.a);
    var sum = c0.rgb;
    var n = 1.0;
    let step = max(u.b.z, 0.1);
    var r = step;
    var ang = 0.0;
    loop {
        if (r >= u.a.z) {
            break;
        }
        let q = clamp(px + vec2<i32>(round(vec2<f32>(cos(ang), sin(ang)) * r)), vec2<i32>(0), last);
        let s = textureLoad(src, q, 0);
        // behind the centre (a larger signed CoC is further away): held to twice the centre's blur
        var size = abs(s.a);
        if (s.a > c0.a) {
            size = min(size, size0 * 2.0);
        }
        let m = smoothstep(r - 0.5, r + 0.5, size);
        sum += mix(sum / n, s.rgb, m);
        n += 1.0;
        r += step / r;
        ang += GOLDEN_ANGLE;
    }
    return vec4<f32>(pow(sum / n, vec3<f32>(1.0 / u.b.y)), 1.0);
}
