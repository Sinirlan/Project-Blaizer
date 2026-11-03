// REMASTER smooth glow (render.rs `Glow`, menu_modern): the glow sources of a UI batch are drawn
// at half size, blurred along x then y with a gaussian, and added over the target. One real
// gaussian instead of stacked strokes, so no visible rings [design choice].

@group(0) @binding(0) var src: texture_2d<f32>;
@group(0) @binding(1) var samp: sampler;
// blur: xy = one tap step along the axis (uv), z = taps' spacing in texels, w = sigma in texels
// composite: w = gain
@group(0) @binding(2) var<uniform> p: vec4<f32>;

struct Out {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs_glow(@builtin(vertex_index) i: u32) -> Out {
    let t = vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u));
    var o: Out;
    o.pos = vec4<f32>(t * 2.0 - 1.0, 0.0, 1.0);
    o.uv = vec2<f32>(t.x, 1.0 - t.y);
    return o;
}

// 25 taps spaced sigma / 4 apart reach 3 sigma; the sampler's bilinear filter fills between
@fragment
fn fs_glow_blur(i: Out) -> @location(0) vec4<f32> {
    var acc = vec4<f32>(0.0);
    var wsum = 0.0;
    for (var k = -12; k <= 12; k++) {
        let d = f32(k) * p.z;
        let w = exp(-0.5 * d * d / (p.w * p.w));
        acc += w * textureSampleLevel(src, samp, i.uv + p.xy * d, 0.0);
        wsum += w;
    }
    return acc / wsum;
}

// premultiplied result, dithered where there is glow so the soft falloff does not band
@fragment
fn fs_glow_comp(i: Out) -> @location(0) vec4<f32> {
    let c = textureSampleLevel(src, samp, i.uv, 0.0) * p.w;
    let h1 = fract(sin(dot(i.pos.xy, vec2<f32>(12.9898, 78.233))) * 43758.5453);
    let h2 = fract(sin(dot(i.pos.xy, vec2<f32>(39.346, 11.135))) * 24634.6345);
    let m = smoothstep(0.002, 0.03, c.a) * (h1 + h2 - 1.0) / 255.0;
    return vec4<f32>(max(c.rgb + vec3<f32>(m), vec3<f32>(0.0)), c.a);
}
