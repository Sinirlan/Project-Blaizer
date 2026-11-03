// REMASTER frosted hologram (render.rs `Gpu::draw_hologram_frosted`, #296), appended to shader.wgsl:
// the hologram drawn once without the clip (`sharp`), blurred at half size with glow.wgsl's
// gaussian (`blurred`), added over the target sharp outside the clip and blurred inside it
// (`holo_mask`, myw_n.w = the frosted light's gain) [design choice].

@group(3) @binding(0) var holo_sharp: texture_2d<f32>;
@group(3) @binding(1) var holo_blurred: texture_2d<f32>;
@group(3) @binding(2) var holo_samp: sampler;

@fragment
fn fs_holo_frost(i: SkyOut) -> @location(0) vec4<f32> {
    let m = holo_mask(i.clip.xy);
    let sharp = textureLoad(holo_sharp, vec2<i32>(i.clip.xy), 0).rgb;
    let blurred = textureSampleLevel(holo_blurred, holo_samp, i.screen, 0.0).rgb;
    return vec4<f32>(mix(sharp, blurred * g.myw_n.w, m), 1.0);
}
