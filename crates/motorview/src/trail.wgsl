// ---------------------------------------------------------------------------------------------
// REMASTER grass trail (render.rs `TrailGpu` / `Gpu::trail_pass`; re-notes/18-grass.md,
// Gitea #158) [design choice]: an R8 texture over the grass edge map's world rectangle
// holding how recently the cars pressed each spot, 1 fresh .. 0 sprung back. Each frame
// `vs_relax`/`fs_relax` draw a full-screen triangle with blend dst - src (the step of sim time
// over TRAIL_RELAX), then `vs_stamp`/`fs_stamp` draw one quad per footprint (`TrailStamp`,
// per-instance vertex) with max blending: the car's hull ellipse, sampled where it was and at
// two moments back along its velocity (its `tail` seconds, the time since its previous
// footprint, so the stamps join at any step rate), pre-aged by the stamp's time against now.
// grass.wgsl `grass_press` reads the texture as an extra flattening term.
// ---------------------------------------------------------------------------------------------

struct TrailUniform {
    // the edge map's transform (`Globals::grass_map`): origin xz, 1 / extent xz
    map: vec4<f32>,
    // x: this frame's relax step (R8 units of 1), y: the sim time now, z: 1 / TRAIL_RELAX
    t: vec4<f32>,
};
@group(0) @binding(0) var<uniform> tu: TrailUniform;

// keep equal to grass.wgsl GRASS_PRESS_IN / GRASS_PRESS_OUT (the live press's footprint)
const PRESS_IN: f32 = 0.85;
const PRESS_OUT: f32 = 1.5;

@vertex
fn vs_relax(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
    let x = f32(i32(i & 1u) * 4 - 1);
    let y = f32(i32(i & 2u) * 2 - 1);
    return vec4<f32>(x, y, 0.0, 1.0);
}

@fragment
fn fs_relax() -> @location(0) vec4<f32> {
    return vec4<f32>(tu.t.x, 0.0, 0.0, 0.0);
}

struct StampIn {
    @location(0) pos: vec2<f32>,
    @location(1) fwd: vec2<f32>,
    @location(2) half: vec2<f32>,
    @location(3) vel: vec2<f32>,
    // x: the stamp's sim time, y: its tail (s)
    @location(4) tt: vec2<f32>,
};

struct StampOut {
    @builtin(position) clip: vec4<f32>,
    // world xz offset from the car's position
    @location(0) d: vec2<f32>,
    @location(1) @interpolate(flat) fwd: vec2<f32>,
    @location(2) @interpolate(flat) half: vec2<f32>,
    @location(3) @interpolate(flat) vel: vec2<f32>,
    // x: tail (s), y: the stamp's strength now (1 fresh, 0 after TRAIL_RELAX)
    @location(4) @interpolate(flat) ta: vec2<f32>,
};

@vertex
fn vs_stamp(v: StampIn, @builtin(vertex_index) i: u32) -> StampOut {
    var corner = array<vec2<f32>, 6>(vec2<f32>(-1.0, -1.0), vec2<f32>(1.0, -1.0), vec2<f32>(1.0, 1.0), vec2<f32>(-1.0, -1.0), vec2<f32>(1.0, 1.0), vec2<f32>(-1.0, 1.0));
    // the quad covers the footprint's outer ellipse and its tail
    let reach = max(v.half.x, v.half.y) * PRESS_OUT + length(v.vel) * v.tt.y;
    let d = corner[i] * reach;
    let uv = (v.pos + d - tu.map.xy) * tu.map.zw;
    var o: StampOut;
    // texture row 0 = the smallest z (the edge map's upload order): v up is clip y down
    o.clip = vec4<f32>(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0, 0.0, 1.0);
    o.d = d;
    o.fwd = v.fwd;
    o.half = v.half;
    o.vel = v.vel;
    o.ta = vec2<f32>(v.tt.y, clamp(1.0 - (tu.t.y - v.tt.x) * tu.t.z, 0.0, 1.0));
    return o;
}

@fragment
fn fs_stamp(i: StampOut) -> @location(0) vec4<f32> {
    var p = 0.0;
    for (var j = 0; j < 3; j++) {
        let d = i.d + i.vel * (i.ta.x * f32(j) / 2.0);
        let u = dot(d, i.fwd);
        let w = dot(d, vec2<f32>(-i.fwd.y, i.fwd.x));
        let e = length(vec2<f32>(u / max(i.half.x, 0.1), w / max(i.half.y, 0.1)));
        p = max(p, 1.0 - smoothstep(PRESS_IN, PRESS_OUT, e));
    }
    return vec4<f32>(p * i.ta.y, 0.0, 0.0, 0.0);
}
