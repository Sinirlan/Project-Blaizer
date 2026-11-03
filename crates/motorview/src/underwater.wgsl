// CHEAT ramlosa / h2o "Underwater" (#317, render.rs `Gpu::draw_underwater`), both render modes: the wobble of the
// original's `0x4419b4`, run as one image pass over the finished 3D picture [design choice: the original moves every
// projected polygon corner and anchors the zoom at the bottom-right corner; ours zooms around the centre].
//
// Forward warp, with c = (W/2, H/2), s = zoom, A = amplitude in px:
//   x' = c.x + (x - c.x) s + A sin((y - H) 2 waves / H + phi)
//   y' = c.y + (y - c.y) s
// This pass samples the inverse, clamp-to-edge (the zoom keeps the samples inside the picture).

struct Warp {
    // phase (rad), zoom s, amplitude (px), waves (original: 25 over H / 2)
    a: vec4<f32>,
    // window W, H
    b: vec4<f32>,
}

@group(0) @binding(0) var scene: texture_2d<f32>;
@group(0) @binding(1) var samp: sampler;
@group(0) @binding(2) var<uniform> warp: Warp;

struct Out {
    @builtin(position) pos: vec4<f32>,
}

@vertex
fn vs_warp(@builtin(vertex_index) i: u32) -> Out {
    let p = vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u));
    var o: Out;
    o.pos = vec4<f32>(p * 2.0 - 1.0, 0.0, 1.0);
    return o;
}

@fragment
fn fs_warp(i: Out) -> @location(0) vec4<f32> {
    let size = warp.b.xy;
    let c = size * 0.5;
    let s = warp.a.y;
    let y = c.y + (i.pos.y - c.y) / s;
    let ripple = warp.a.z * sin((y - size.y) * 2.0 * warp.a.w / size.y + warp.a.x);
    let x = c.x + (i.pos.x - c.x - ripple) / s;
    return vec4<f32>(textureSampleLevel(scene, samp, vec2<f32>(x, y) / size, 0.0).rgb, 1.0);
}
