// MotionBlur: the previous frame, one texel per pixel, blended by the pipeline (render.rs).

@group(0) @binding(0) var prev: texture_2d<f32>;

@vertex
fn vs_blur(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
    let uv = vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u));
    return vec4<f32>(uv * 2.0 - 1.0, 0.0, 1.0);
}

@fragment
fn fs_blur(@builtin(position) p: vec4<f32>) -> @location(0) vec4<f32> {
    return textureLoad(prev, vec2<i32>(p.xy), 0);
}
