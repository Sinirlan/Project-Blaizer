// REMASTER frosted scene (render.rs `Gpu::draw_scene_frosted`, #300): the race behind the Glass pause
// menu's cards. The scene is drawn once into `sharp`, blurred at half size with glow.wgsl's gaussian
// (`blurred`), and written over the target: sharp outside the cards, blurred inside their rounded
// rectangles (window px) [design choice].

struct Frost {
    // centre.xy, half extents.zw of each card; the last entry is (count, corner radius, 0, 0)
    rects: array<vec4<f32>, 13>,
}

@group(0) @binding(0) var sharp: texture_2d<f32>;
@group(0) @binding(1) var blurred: texture_2d<f32>;
@group(0) @binding(2) var samp: sampler;
@group(0) @binding(3) var<uniform> frost: Frost;

struct Out {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
}

@vertex
fn vs_frost(@builtin(vertex_index) i: u32) -> Out {
    let p = vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u));
    var o: Out;
    o.pos = vec4<f32>(p * 2.0 - 1.0, 0.0, 1.0);
    o.uv = vec2<f32>(p.x, 1.0 - p.y);
    return o;
}

// 1 inside the rounded rectangle, 0 outside, a 1 px soft edge
fn card(p: vec2<f32>, c: vec2<f32>, h: vec2<f32>, r: f32) -> f32 {
    let q = abs(p - c) - (h - vec2<f32>(r));
    let e = length(max(q, vec2<f32>(0.0))) + min(max(q.x, q.y), 0.0) - r;
    return clamp(0.5 - e, 0.0, 1.0);
}

@fragment
fn fs_frost(i: Out) -> @location(0) vec4<f32> {
    let n = u32(frost.rects[12].x);
    var m = 0.0;
    for (var k = 0u; k < n; k = k + 1u) {
        m = max(m, card(i.pos.xy, frost.rects[k].xy, frost.rects[k].zw, frost.rects[12].y));
    }
    let s = textureLoad(sharp, vec2<i32>(i.pos.xy), 0).rgb;
    let b = textureSampleLevel(blurred, samp, i.uv, 0.0).rgb;
    return vec4<f32>(mix(s, b, m), 1.0);
}
