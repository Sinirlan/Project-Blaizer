// REMASTER car paint finishes: gloss, metallic, pearl (re-notes/21-paint-finishes.md 21.8-21.10, #351) [design choice]
//
// One angle model for every finish: each property blends from its value at a grazing angle (n.v = 0, the
// silhouette) to its value facing the camera (n.v = 1) along a power curve. A finish is a set of hue-free
// multipliers on the paint zone's colour; pearl turns that colour around the grey axis towards the edges.
//
// Binding-free and prefixed (`fin_`, `Finish`, `FIN_`) so it is appended to shader.wgsl (render.rs `SHADER_SRC`).
// Gamma space, like fs_remaster. The caller owns light, shadow, the reflection source and the final cap.

// One finish, eight vec4 so it can live in a uniform as is (21.9).
struct Finish {
    tint_edge: vec4<f32>,  // rgb: diffuse tint at grazing; w: reflection mask at grazing
    tint_face: vec4<f32>,  // rgb: diffuse tint facing;    w: reflection mask facing
    glint_edge: vec4<f32>, // rgb: sun glint colour at grazing; w: exponent of its angle curve
    glint_face: vec4<f32>, // rgb: sun glint colour facing;     w: Blinn-Phong exponent of the glint
    refl: vec4<f32>,       // x: reflection at grazing, y: facing, z: exponent of its angle curve, w: glint gain
    hue: vec4<f32>,        // x: pearl turn (radians) at grazing, y: glint hue share, z: reflection hue share, w: pearl seed
    look: vec4<f32>,       // x: share of the softened environment, y: how much shadow dims the reflection,
                           // z: softened reflection kept on dark paint (1 = all), w: how much of the base the reflection replaces
    shape: vec4<f32>,      // x: exponent of the colour's angle curve (1 = linear in n.v; below 1 keeps the paint's own colour longer)
}

// What the caller has already worked out for this pixel.
struct FinishIn {
    zone: vec3<f32>,     // the paint zone's colour (times the body texture)
    n: vec3<f32>,        // unit normal
    v: vec3<f32>,        // unit vector to the eye
    l: vec3<f32>,        // unit vector to the sun
    light: vec3<f32>,    // diffuse light: ambient + sun * visibility + dynamic lights
    sun: vec3<f32>,      // sun colour * strength * visibility, zero when the sun is behind the surface
    env: vec3<f32>,      // the surroundings along fin_reflect_dir (cube, sphere map or a grey stand-in)
    env_soft: vec3<f32>, // the same, blurred (flakes of a metallic paint mirror a softer picture); env when no blur
    flakes: vec3<f32>,   // extra glint (motorhead's flake glints, 9.40), already coloured; zero for none
    vis: f32,            // the sun's visibility (shadow) at the pixel, 0..1
}

// The chroma a grey paint is given before a pearl turns it (greys have no hue to turn): zero-mean, pinkish warm, so
// white and silver pearls shift towards a pink-gold at the edges (black has nothing to turn). Scaled by `Finish.hue.w`
// and by the paint's brightness, fades out as the paint's own chroma grows.
const FIN_SEED: vec3<f32> = vec3<f32>(0.40, -0.25, -0.15);

// Gloss: a lacquer. The colour hardly changes with the angle, a Fresnel rim of reflection that replaces the paint
// where it rises (the clear coat), a tight white glint. Starts from the Enhanced clear coat of #135 (F0 0.2, shine 48,
// gain 0.5) with a power curve in place of Schlick's.
fn fin_gloss() -> Finish {
    return Finish(
        vec4<f32>(vec3<f32>(1.0), 1.0),
        vec4<f32>(vec3<f32>(1.0), 1.0),
        vec4<f32>(vec3<f32>(1.0), 1.0),
        vec4<f32>(vec3<f32>(1.0), 48.0),
        vec4<f32>(1.0, 0.2, 0.18, 0.5),
        vec4<f32>(0.0, 0.0, 0.0, 0.0),
        vec4<f32>(0.0, 0.0, 1.0, 1.0),
        vec4<f32>(1.0, 0.0, 0.0, 0.0),
    );
}

// Metallic: darker towards the silhouette (the flop), a broad glint that takes the paint's hue, a reflection of a
// softened surroundings in the paint's hue under a sharper Fresnel rim. Starts from the #136 branch: 0.7 diffuse
// (0.45 of it at the edge), shine 16, gain 0.9, sun tint 0.6, flake reflection 0.5, dimmed on dark paint down to 0.25
// and in shadow by 0.4.
fn fin_metallic() -> Finish {
    return Finish(
        vec4<f32>(vec3<f32>(0.36), 1.0),
        vec4<f32>(vec3<f32>(0.8), 1.0),
        vec4<f32>(vec3<f32>(1.0), 1.0),
        vec4<f32>(vec3<f32>(1.0), 16.0),
        vec4<f32>(1.0, 0.6, 0.3, 0.9),
        vec4<f32>(0.0, 0.6, 0.7, 0.0),
        vec4<f32>(0.7, 0.4, 0.25, 0.5),
        vec4<f32>(1.0, 0.0, 0.0, 0.0),
    );
}

// Pearl: the colour turns towards a second hue at the edges, where reflection and glint also live; facing
// the camera it is mostly its own colour with little reflection.
fn fin_pearl() -> Finish {
    return Finish(
        vec4<f32>(vec3<f32>(0.9), 1.0),
        vec4<f32>(vec3<f32>(1.0), 0.5),
        vec4<f32>(vec3<f32>(1.0), 2.0),
        vec4<f32>(vec3<f32>(0.45), 40.0),
        vec4<f32>(1.0, 0.14, 0.2, 1.0),
        vec4<f32>(0.6, 0.8, 0.4, 1.2),
        vec4<f32>(0.0, 0.3, 1.0, 0.75),
        vec4<f32>(0.2, 0.0, 0.0, 0.0),
    );
}

// The direction to look up the surroundings along: the eye ray mirrored about the normal.
fn fin_reflect_dir(n: vec3<f32>, v: vec3<f32>) -> vec3<f32> {
    return reflect(-v, n);
}

// Turns a colour around the grey axis by `angle` radians (Rodrigues' rotation about (1,1,1)/sqrt(3)). Grey
// stays grey and the mean of the channels is kept (not the perceived lightness: a blue turned towards red gets
// brighter to the eye). A turned saturated colour can leave the gamut, so it is clamped at 0 (red turned by 0.6
// rad: 0.88, 0.38, 0), which costs a little of its chroma.
fn fin_hue_turn(c: vec3<f32>, angle: f32) -> vec3<f32> {
    let k = vec3<f32>(0.57735027);
    let cs = cos(angle);
    let sn = sin(angle);
    let turned = c * cs + cross(k, c) * sn + k * dot(k, c) * (1.0 - cs);
    return max(turned, vec3<f32>(0.0));
}

// The paint's hue: the colour scaled so its brightest channel is 1 (black and grey give white).
fn fin_hue(c: vec3<f32>) -> vec3<f32> {
    let peak = max(c.r, max(c.g, c.b));
    return select(vec3<f32>(1.0), c / peak, peak > 1e-3);
}

// The angle curve: `edge` at n.v = 0, `face` at n.v = 1, along t^p.
fn fin_curve(edge: f32, face: f32, t: f32, p: f32) -> f32 {
    return mix(edge, face, pow(t, p));
}

fn fin_curve3(edge: vec3<f32>, face: vec3<f32>, t: f32, p: f32) -> vec3<f32> {
    return mix(edge, face, pow(t, p));
}

// The shaded paint, uncapped (the caller applies `cap`).
fn fin_shade(f: Finish, i: FinishIn) -> vec3<f32> {
    let t = clamp(dot(i.n, i.v), 0.001, 1.0);
    let peak = max(i.zone.r, max(i.zone.g, i.zone.b));
    let chroma = peak - min(i.zone.r, min(i.zone.g, i.zone.b));

    // the colour: the zone's own facing the camera, turned (pearl; greys get a seed to turn) and tinted towards the
    // edges; linear in n.v
    let seeded = i.zone + FIN_SEED * (f.hue.w * min(peak, 1.0) * (1.0 - clamp(chroma * 4.0, 0.0, 1.0)));
    let edge_colour = fin_hue_turn(seeded, f.hue.x);
    let own = pow(t, f.shape.x);
    let hue = fin_hue(mix(edge_colour, i.zone, own));
    let mask = mix(f.tint_edge.w, f.tint_face.w, t);

    // the reflection: a Fresnel-like rise towards the edges, part of it in the paint's hue, on a sharp or a softened
    // picture, less on dark paint and in shadow
    // (dark paint dims only the softened picture, the flakes' part: a clear coat stays on black)
    let kept = mix(f.look.z, 1.0, min(peak, 1.0));
    let env = mix(i.env, i.env_soft * kept, f.look.x);
    let shadow = 1.0 - f.look.y * (1.0 - i.vis);
    let refl = fin_curve(f.refl.x, f.refl.y, t, f.refl.z) * mask * shadow;
    let mirrored = env * refl * mix(vec3<f32>(1.0), hue, f.hue.z);

    // the base, which the reflection (the clear coat) replaces by `look.w` of its strength
    let base = mix(edge_colour * f.tint_edge.rgb, i.zone * f.tint_face.rgb, own) * i.light * (1.0 - f.look.w * clamp(refl, 0.0, 1.0));

    // the sun glint: Blinn-Phong, its colour along its own angle curve, part of it in the paint's hue
    let h = normalize(i.l + i.v);
    let lobe = pow(max(dot(i.n, h), 0.0), f.glint_face.w) * f.refl.w;
    let glint_colour = fin_curve3(f.glint_edge.rgb, f.glint_face.rgb, t, f.glint_edge.w) * mix(vec3<f32>(1.0), hue, f.hue.y);
    let glint = lobe * glint_colour * mask * i.sun + i.flakes;

    return base + glint + mirrored;
}
