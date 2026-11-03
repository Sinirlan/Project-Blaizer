// REMASTER menu car preview (render.rs `Gpu::draw_studio_aa`): the car's resolved 4x multisampled frame, premultiplied
// (cleared transparent, the alpha is the car's coverage), laid over the window pixel for pixel. The viewport is the
// frame's rectangle, so the triangle's uv spans the texture.

@group(0) @binding(0) var img: texture_2d<f32>;

struct Out {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
}

@vertex
fn vs_studio(@builtin(vertex_index) i: u32) -> Out {
    let p = vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u));
    var o: Out;
    o.pos = vec4<f32>(p * 2.0 - 1.0, 0.0, 1.0);
    o.uv = vec2<f32>(p.x, 1.0 - p.y);
    return o;
}

@fragment
fn fs_studio(i: Out) -> @location(0) vec4<f32> {
    let d = vec2<f32>(textureDimensions(img));
    return textureLoad(img, vec2<i32>(min(i.uv * d, d - vec2<f32>(1.0))), 0);
}
