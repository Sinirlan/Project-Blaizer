// Lens-flare visibility from the scene's depth (#123; CLASSIC and REMASTER flares, render.rs
// `flare_vis_pass`). One invocation per flaring light: TAPS x TAPS depth samples over a square
// of half-size `r` round the light's screen point; a tap sees the light when the drawn scene
// there is no nearer than the light minus `bias` (its own lamp housing). The visible share
// scales the colour of the light's flare quads in the vertex buffer.
// DEPTH_TEX is replaced by texture_depth_2d or texture_depth_multisampled_2d (MSAA) at load.

struct Light {
    // uv in the target (0..1, y down), view depth (m), tap half-size (uv x units)
    p: vec4<f32>,
    // first flare vertex, vertex count
    v: vec4<u32>,
}

struct Params {
    // near, far (the projection's), bias (m), unused
    p: vec4<f32>,
}

@group(0) @binding(0) var depth: DEPTH_TEX;
@group(0) @binding(1) var<uniform> u: Params;
@group(0) @binding(2) var<storage, read> lights: array<Light>;
// render.rs `Vertex`, 14 floats: pos 0..3, uv 3..5, colour 5..9, ...
@group(0) @binding(3) var<storage, read_write> verts: array<f32>;

const TAPS: i32 = 5;
const VERTEX_FLOATS: u32 = 14u;
const COLOUR: u32 = 5u;

@compute @workgroup_size(64)
fn cs_flare_vis(@builtin(global_invocation_id) id: vec3<u32>) {
    if id.x >= arrayLength(&lights) {
        return;
    }
    let l = lights[id.x];
    let dims = vec2<f32>(textureDimensions(depth));
    let near = u.p.x;
    let far = u.p.y;
    let bias = u.p.z;
    let centre = l.p.xy * dims;
    let step = l.p.w * dims.x * 2.0 / f32(TAPS - 1);
    var seen = 0.0;
    for (var i = 0; i < TAPS; i++) {
        for (var j = 0; j < TAPS; j++) {
            let o = (vec2<f32>(f32(i), f32(j)) - f32(TAPS - 1) * 0.5) * step;
            let px = vec2<i32>(clamp(centre + o, vec2<f32>(0.0), dims - 1.0));
            let d = textureLoad(depth, px, 0);
            // the depth target is 0 at `near`, 1 at `far` (render.rs `perspective`)
            let z = near * far / (far - d * (far - near));
            seen += select(0.0, 1.0, z > l.p.z - bias);
        }
    }
    let vis = seen / f32(TAPS * TAPS);
    for (var k = l.v.x; k < l.v.x + l.v.y; k++) {
        let c = k * VERTEX_FLOATS + COLOUR;
        verts[c] *= vis;
        verts[c + 1u] *= vis;
        verts[c + 2u] *= vis;
    }
}
