//! REMASTER glass rev meter (09-remaster 9.21 phase 4; our own art, design choice): the
//! spine of `RevMeter_750.blend` as a table for `fs_ui_remaster`'s `rev_meter`, which draws
//! the tube analytically per pixel (no texture), and the tunables of its look (`RevMeterTune`
//! per quad, `RevLook` as a uniform).

/// The meter's tunables, percent (settings `RemasterRevMeter`, shot flag `--rev-meter`).
/// 100 = the Blender material's value unless noted.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct RevMeterTune {
    /// lit part's emission (GlowStrength 1.125)
    pub glow: u32,
    /// unlit part's own glow (UnlitGlow 0.12)
    pub inner_glow: u32,
    /// opacity of the unlit glass
    pub glass_alpha: u32,
    /// specular strip lights and Fresnel rim
    pub spec: u32,
    /// soft halo around the lit part (the Blender compositor's Glare)
    pub bloom: u32,
    /// cross-section: tube radius
    pub thickness: u32,
    /// cross-section: height of the camera-facing wall over the side walls, percent of the
    /// half-width (fit: 41)
    pub depth: u32,
    /// cross-section: superellipse exponent of that wall x 100 (fit: 210; 200 = ellipse,
    /// larger = flatter face with tighter edges)
    pub roundness: u32,
}

impl Default for RevMeterTune {
    fn default() -> Self {
        RevMeterTune { glow: 100, inner_glow: 100, glass_alpha: 100, spec: 100, bloom: 100, thickness: 100, depth: 41, roundness: 210 }
    }
}

impl RevMeterTune {
    pub const FIELDS: usize = 8;

    pub fn to_array(self) -> [u32; 8] {
        [self.glow, self.inner_glow, self.glass_alpha, self.spec, self.bloom, self.thickness, self.depth, self.roundness]
    }

    pub fn from_array(a: [u32; 8]) -> RevMeterTune {
        let [glow, inner_glow, glass_alpha, spec, bloom, thickness, depth, roundness] = a;
        // roundness below 1.1 has no edge; the rest is clamped to sane ranges
        RevMeterTune {
            glow: glow.min(400),
            inner_glow: inner_glow.min(400),
            glass_alpha: glass_alpha.min(200),
            spec: spec.min(400),
            bloom: bloom.min(400),
            thickness: thickness.clamp(25, 200),
            depth: depth.clamp(1, 200),
            roundness: roundness.clamp(110, 1000),
        }
    }
}

/// Width of the meter at 100 %, canvas px: the art it replaces (h:meter01, 127 x 40).
pub const WIDTH: f32 = 127.0;

// ---- the shader's look (ui.wgsl `rev_meter`, written into its WGSL prefix as REV_*); the
// runtime `RevMeterTune` scales them. [O] = from the Blender material / scene, else tuned
// by eye against RevMeter_750_preview.png (design choice).

/// [O] ColorRamp stops on `progress`: (position, colour)
pub const RAMP: [(f32, [f32; 3]); 5] = [(0.0, [0.10, 1.0, 0.06]), (0.22, [0.35, 1.0, 0.02]), (0.5, [1.0, 0.86, 0.0]), (0.75, [1.0, 0.34, 0.0]), (1.0, [1.0, 0.04, 0.0])];
/// half width of the box blur that stands in for the B-spline ramp (progress units)
pub const RAMP_BLUR: f32 = 0.07;
/// [O] lit emission strength (GlowStrength)
pub const EMISSION: f32 = 1.125;
/// [O] lit emission x (base + gain x facing) (Math.002 multiply-add)
pub const FACING: [f32; 2] = [0.9, 0.6];
/// extra lit brightness at the tube's core (0 at the rim)
pub const CORE: f32 = 0.6;
/// share of an over-bright channel spilled into the others (white-hot highlights)
pub const SPILL: f32 = 0.35;
/// [O] unlit colour: the ramp's saturation x, value x (HSV node)
pub const MUTED: [f32; 2] = [0.55, 0.28];
/// [O] unlit emission (UnlitGlow)
pub const UNLIT_GLOW: f32 = 0.12;
/// opacity of the unlit and the lit glass
pub const ALPHA: [f32; 2] = [0.4, 0.85];
/// Fresnel F0 ([O] IOR 1.47 gives 0.036) and the environment the rim reflects
pub const F0: f32 = 0.04;
pub const ENV: [f32; 3] = [0.70, 0.72, 0.76];
/// [O: light positions] strip lights: direction to the light (x right, y up, z to the
/// viewer) and strength (key strip, rim strip, left and right kickers)
pub const KEY: [f32; 4] = [0.0, 0.68, 0.73, 1.0];
pub const RIM: [f32; 4] = [0.0, -0.707, 0.707, 0.45];
pub const KICK_L: [f32; 4] = [-0.87, 0.12, 0.48, 0.35];
pub const KICK_R: [f32; 4] = [0.87, 0.12, 0.48, 0.35];
/// halo round the lit part: strength, radius (Blender units), colour gain over the ramp
pub const HALO_STRENGTH: f32 = 0.8;
pub const HALO_RADIUS: f32 = 0.3;
pub const HALO_GAIN: f32 = 1.3;
/// share of the Fresnel term that mixes the rim towards ENV
pub const ENV_MIX: f32 = 0.8;
/// strip lights (key, rim, left kicker, right kicker): half angular width and length, rad
pub const STRIP_WIDTH: [f32; 4] = [0.037, 0.026, 0.024, 0.024];
pub const STRIP_LEN: [f32; 4] = [0.94, 0.94, 0.24, 0.24];

/// Room around the tube for the halo, Blender units (where it has faded to ~1 %).
pub fn halo_room() -> f32 {
    HALO_RADIUS * 4.5
}

/// Canvas px per Blender unit at 100 %.
pub fn scale() -> f32 {
    scale_for(WIDTH)
}

/// Canvas px per Blender unit for a meter `width` canvas px wide (REMASTER Race line HUD, #323).
pub fn scale_for(width: f32) -> f32 {
    width / (BOUNDS[2] - BOUNDS[0])
}

/// The meter's box at 100 %, canvas px.
pub fn size() -> (f32, f32) {
    size_for(WIDTH)
}

/// The meter's box for a `width` canvas px wide meter.
pub fn size_for(width: f32) -> (f32, f32) {
    (width, (BOUNDS[3] - BOUNDS[1]) * scale_for(width))
}

/// x (Blender units) of the fill's vertical cut at fill `frac`: the spine's x where its `t`
/// = frac; past the box when empty or full.
pub fn cut_x(frac: f32) -> f32 {
    if frac <= 0.0 {
        return BOUNDS[0] - 1.0;
    }
    if frac >= 1.0 {
        return BOUNDS[2] + 1.0;
    }
    SPINE
        .windows(2)
        .find(|w| frac <= w[1][2])
        .map_or(BOUNDS[2] + 1.0, |w| w[0][0] + (w[1][0] - w[0][0]) * (frac - w[0][2]) / (w[1][2] - w[0][2]).max(1e-6))
}

/// WGSL prefix of ui.wgsl: the spine table (art). The look constants are `RevLook`, a uniform.
pub fn wgsl() -> String {
    let pts: Vec<String> = SPINE.iter().map(|&p| format!("vec4<f32>({:?}, {:?}, {:?}, {:?})", p[0], p[1], p[2], p[3])).collect();
    let mut s = String::from("// generated by revmeter.rs\n");
    s += &format!("const REV_N: u32 = {}u;\n", SPINE.len());
    s += &format!("var<private> REV_SPINE: array<vec4<f32>, {}> = array<vec4<f32>, {}>({});\n", SPINE.len(), SPINE.len(), pts.join(", "));
    s
}

/// The look constants above as a runtime value (the `rev: RevLook` uniform of ui.wgsl, binding
/// 4 of the sharp UI pipeline). The game draws with the defaults; the Rev meter workshop
/// (`revshop`) edits it live.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct RevLook {
    pub ramp: [(f32, [f32; 3]); 5],
    pub ramp_blur: f32,
    pub emission: f32,
    pub facing: [f32; 2],
    pub core: f32,
    pub spill: f32,
    pub muted: [f32; 2],
    pub unlit_glow: f32,
    pub alpha: [f32; 2],
    pub f0: f32,
    pub env: [f32; 3],
    pub env_mix: f32,
    /// key, rim, left kicker, right kicker: direction xyz + strength
    pub lights: [[f32; 4]; 4],
    pub strip_width: [f32; 4],
    pub strip_len: [f32; 4],
    /// strength, radius, colour gain
    pub halo: [f32; 3],
}

impl Default for RevLook {
    fn default() -> Self {
        RevLook {
            ramp: RAMP,
            ramp_blur: RAMP_BLUR,
            emission: EMISSION,
            facing: FACING,
            core: CORE,
            spill: SPILL,
            muted: MUTED,
            unlit_glow: UNLIT_GLOW,
            alpha: ALPHA,
            f0: F0,
            env: ENV,
            env_mix: ENV_MIX,
            lights: [KEY, RIM, KICK_L, KICK_R],
            strip_width: STRIP_WIDTH,
            strip_len: STRIP_LEN,
            halo: [HALO_STRENGTH, HALO_RADIUS, HALO_GAIN],
        }
    }
}

const LIGHT_NAMES: [&str; 4] = ["key", "rim", "kick_l", "kick_r"];

impl RevLook {
    /// Size of the uniform: 16 vec4 (`struct RevLook` in ui.wgsl).
    pub const GPU_BYTES: usize = 16 * 16;

    /// The uniform's contents, in ui.wgsl's field order.
    pub fn pack(&self) -> [f32; 64] {
        let mut o = [0.0; 64];
        let mut put = |i: usize, v: [f32; 4]| o[i * 4..i * 4 + 4].copy_from_slice(&v);
        for (k, &(t, c)) in self.ramp.iter().enumerate() {
            put(k, [c[0], c[1], c[2], t]);
        }
        put(5, [self.ramp_blur, self.emission, self.core, self.spill]);
        put(6, [self.facing[0], self.facing[1], self.muted[0], self.muted[1]]);
        put(7, [self.unlit_glow, self.alpha[0], self.alpha[1], self.f0]);
        put(8, [self.env[0], self.env[1], self.env[2], self.env_mix]);
        for (k, l) in self.lights.iter().enumerate() {
            put(9 + k, *l);
        }
        put(13, self.strip_width);
        put(14, self.strip_len);
        put(15, [self.halo[0], self.halo[1], self.halo[2], 0.0]);
        o
    }

    /// Every number by name, in file order (`to_text` / `from_text`).
    pub fn visit(&mut self, f: &mut dyn FnMut(&str, &mut f32)) {
        for (k, (t, c)) in self.ramp.iter_mut().enumerate() {
            let [r, g, b] = c;
            for (n, v) in [("r", r), ("g", g), ("b", b), ("pos", t)] {
                f(&format!("ramp{k}_{n}"), v);
            }
        }
        f("ramp_blur", &mut self.ramp_blur);
        f("emission", &mut self.emission);
        f("facing_base", &mut self.facing[0]);
        f("facing_gain", &mut self.facing[1]);
        f("core", &mut self.core);
        f("spill", &mut self.spill);
        f("muted_sat", &mut self.muted[0]);
        f("muted_val", &mut self.muted[1]);
        f("unlit_glow", &mut self.unlit_glow);
        f("alpha_unlit", &mut self.alpha[0]);
        f("alpha_lit", &mut self.alpha[1]);
        f("f0", &mut self.f0);
        for (n, v) in ["r", "g", "b"].into_iter().zip(self.env.iter_mut()) {
            f(&format!("env_{n}"), v);
        }
        f("env_mix", &mut self.env_mix);
        for (name, l) in LIGHT_NAMES.into_iter().zip(self.lights.iter_mut()) {
            for (n, v) in ["x", "y", "z", "strength"].into_iter().zip(l.iter_mut()) {
                f(&format!("{name}_{n}"), v);
            }
        }
        for (k, name) in LIGHT_NAMES.into_iter().enumerate() {
            f(&format!("{name}_width"), &mut self.strip_width[k]);
            f(&format!("{name}_len"), &mut self.strip_len[k]);
        }
        f("halo_strength", &mut self.halo[0]);
        f("halo_radius", &mut self.halo[1]);
        f("halo_gain", &mut self.halo[2]);
    }

    /// `name value` lines, the look and the 8 tune percentages (workshop preset file).
    pub fn to_text(&self, tune: &RevMeterTune) -> String {
        let mut me = *self;
        let mut s = String::from("# rev meter preset (revshop); name value\n");
        me.visit(&mut |n, v| s += &format!("{n} {v:?}\n"));
        s += &format!("tune {}\n", tune.to_array().map(|x| x.to_string()).join(" "));
        s
    }

    /// Inverse of `to_text`; unknown names are an error, missing ones keep the defaults.
    pub fn from_text(text: &str) -> Result<(RevLook, RevMeterTune), String> {
        let mut look = RevLook::default();
        let mut tune = RevMeterTune::default();
        let mut vals = std::collections::HashMap::new();
        for (i, line) in text.lines().enumerate() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let (name, rest) = line.split_once(char::is_whitespace).ok_or_else(|| format!("line {}: no value", i + 1))?;
            if name == "tune" {
                let a: Vec<u32> = rest.split_whitespace().map(|x| x.parse().map_err(|_| format!("line {}: bad tune value", i + 1))).collect::<Result<_, _>>()?;
                tune = RevMeterTune::from_array(<[u32; 8]>::try_from(a.as_slice()).map_err(|_| format!("line {}: tune wants 8 values", i + 1))?);
            } else {
                let v: f32 = rest.trim().parse().map_err(|_| format!("line {}: bad number", i + 1))?;
                vals.insert(name.to_string(), v);
            }
        }
        look.visit(&mut |n, v| {
            if let Some(x) = vals.remove(n) {
                *v = x;
            }
        });
        if let Some(n) = vals.keys().next() {
            return Err(format!("unknown name {n}"));
        }
        Ok((look, tune))
    }

    /// The `pub const` block of this file with these values (paste over the look constants),
    /// the settings line and the shot flag of the tune.
    pub fn to_rust(&self, tune: &RevMeterTune) -> String {
        let f = |v: f32| format!("{v:?}");
        let arr = |a: &[f32]| format!("[{}]", a.iter().map(|&v| f(v)).collect::<Vec<_>>().join(", "));
        let ramp: Vec<String> = self.ramp.iter().map(|&(t, c)| format!("({}, {})", f(t), arr(&c))).collect();
        let t = tune.to_array().map(|x| x.to_string());
        let mut s = String::new();
        s += &format!("pub const RAMP: [(f32, [f32; 3]); 5] = [{}];\n", ramp.join(", "));
        s += &format!("pub const RAMP_BLUR: f32 = {};\n", f(self.ramp_blur));
        s += &format!("pub const EMISSION: f32 = {};\n", f(self.emission));
        s += &format!("pub const FACING: [f32; 2] = {};\n", arr(&self.facing));
        s += &format!("pub const CORE: f32 = {};\n", f(self.core));
        s += &format!("pub const SPILL: f32 = {};\n", f(self.spill));
        s += &format!("pub const MUTED: [f32; 2] = {};\n", arr(&self.muted));
        s += &format!("pub const UNLIT_GLOW: f32 = {};\n", f(self.unlit_glow));
        s += &format!("pub const ALPHA: [f32; 2] = {};\n", arr(&self.alpha));
        s += &format!("pub const F0: f32 = {};\n", f(self.f0));
        s += &format!("pub const ENV: [f32; 3] = {};\n", arr(&self.env));
        s += &format!("pub const ENV_MIX: f32 = {};\n", f(self.env_mix));
        for (n, l) in ["KEY", "RIM", "KICK_L", "KICK_R"].into_iter().zip(&self.lights) {
            s += &format!("pub const {n}: [f32; 4] = {};\n", arr(l));
        }
        s += &format!("pub const STRIP_WIDTH: [f32; 4] = {};\n", arr(&self.strip_width));
        s += &format!("pub const STRIP_LEN: [f32; 4] = {};\n", arr(&self.strip_len));
        s += &format!("pub const HALO_STRENGTH: f32 = {};\n", f(self.halo[0]));
        s += &format!("pub const HALO_RADIUS: f32 = {};\n", f(self.halo[1]));
        s += &format!("pub const HALO_GAIN: f32 = {};\n", f(self.halo[2]));
        s += &format!("// settings.cfg: RemasterRevMeter {}\n", t.join(" "));
        s += &format!("// shot flag: --rev-meter {}\n", t.join(","));
        s
    }
}

// [O: Blender headless] RevBar_GlassCasing: 45 rings, centre and half-width in the camera's
// picture plane (x right, z up), `t` = the mesh's `progress` attribute.
// picture-plane bounds x -7.0225..7.0892, z -2.2871..2.3901
// front wall fit: depth 0.410 x half-width, roundness p 2.1 (rms 0.0348)
pub const BOUNDS: [f32; 4] = [-7.0225, -2.2871, 7.0892, 2.3901];
pub const SPINE: [[f32; 4]; 45] = [
    [-7.0183, -1.8187, 0.00000, 0.0600],
    [-7.0079, -1.8180, 0.00070, 0.1195],
    [-6.9864, -1.8165, 0.00215, 0.2075],
    [-6.9535, -1.8141, 0.00436, 0.2919],
    [-6.9079, -1.8109, 0.00743, 0.3699],
    [-6.8423, -1.8063, 0.01184, 0.4318],
    [-6.7613, -1.8006, 0.01729, 0.4714],
    [-6.6722, -1.7944, 0.02328, 0.4851],
    [-6.3600, -1.7724, 0.04428, 0.5159],
    [-5.7016, -1.7266, 0.08856, 0.5440],
    [-5.0436, -1.6756, 0.13285, 0.5771],
    [-4.3889, -1.5921, 0.17713, 0.5909],
    [-3.7369, -1.4905, 0.22141, 0.6015],
    [-3.0894, -1.3636, 0.26569, 0.6102],
    [-2.5060, -1.2236, 0.30595, 0.6173],
    [-1.9546, -1.0441, 0.34487, 0.6189],
    [-1.4539, -0.8422, 0.38110, 0.6207],
    [-1.2024, -0.7193, 0.39988, 0.6286],
    [-0.7539, -0.4561, 0.43477, 0.6346],
    [-0.2075, -0.0860, 0.47906, 0.6704],
    [0.3230, 0.3065, 0.52334, 0.7105],
    [0.7389, 0.6185, 0.55823, 0.7695],
    [1.2931, 0.9767, 0.60251, 0.8406],
    [1.4862, 1.0821, 0.61727, 0.8638],
    [1.7784, 1.2123, 0.63874, 0.8902],
    [1.9658, 1.2819, 0.65216, 0.9007],
    [2.1002, 1.3210, 0.66155, 0.9064],
    [2.2762, 1.3587, 0.67363, 0.9121],
    [2.7912, 1.4301, 0.70852, 0.9243],
    [3.1698, 1.4613, 0.73401, 0.9263],
    [3.4698, 1.4595, 0.75414, 0.9312],
    [4.1085, 1.4213, 0.79708, 0.9280],
    [4.7645, 1.3484, 0.84137, 0.9300],
    [5.3963, 1.2469, 0.88431, 0.9338],
    [5.7079, 1.1745, 0.90578, 0.9307],
    [6.1675, 1.0382, 0.93794, 0.9250],
    [6.3182, 0.9929, 0.94850, 0.9114],
    [6.4645, 0.9490, 0.95875, 0.8713],
    [6.6021, 0.9076, 0.96839, 0.8059],
    [6.7271, 0.8701, 0.97715, 0.7170],
    [6.8358, 0.8374, 0.98476, 0.6071],
    [6.9249, 0.8106, 0.99100, 0.4795],
    [6.9920, 0.7905, 0.99570, 0.3379],
    [7.0351, 0.7775, 0.99872, 0.1865],
    [7.0533, 0.7720, 1.00000, 0.0600],
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_round_trip() {
        let mut look = RevLook { emission: 1.5, ..Default::default() };
        look.lights[2][3] = 0.9;
        look.ramp[3].1[1] = 0.2;
        let tune = RevMeterTune { glow: 130, ..Default::default() };
        let (l2, t2) = RevLook::from_text(&look.to_text(&tune)).unwrap();
        assert_eq!((l2, t2), (look, tune));
        assert!(RevLook::from_text("bogus 1").is_err());
    }

    #[test]
    fn rust_block_has_the_defaults() {
        let r = RevLook::default().to_rust(&RevMeterTune::default());
        assert!(r.contains("pub const EMISSION: f32 = 1.125;"));
        assert!(r.contains("pub const KEY: [f32; 4] = [0.0, 0.68, 0.73, 1.0];"));
        assert!(r.contains("// shot flag: --rev-meter 100,100,100,100,100,100,41,210"));
    }

    #[test]
    fn pack_matches_the_consts() {
        let p = RevLook::default().pack();
        assert_eq!(p[3], RAMP[0].0);
        assert_eq!(p[5 * 4 + 1], EMISSION);
        assert_eq!(p[9 * 4 + 3], KEY[3]);
        assert_eq!(p[15 * 4 + 1], HALO_RADIUS);
    }
}
