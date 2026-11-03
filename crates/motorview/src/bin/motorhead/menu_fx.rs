//! CLASSIC front-end background effect: two stepped rotating rings and three wireframe line
//! objects (`Data/LINEOBJ/lobj00..08.lob`) flying out of the screen, drawn over `back.tga` on
//! every menu screen except track and car (re-notes/08-missing-features §8.12).
//! Original: per frame 0x46f060, rings 0x46ed4c, line object projection 0x454c84, line
//! rasteriser 0x4061cc (adds the colour to the buffer's palette index, saturating).

use crate::league::Rng;
use motorview::ui::Batch;
use std::path::Path;

/// Number of flyers (`[0x4be278]`).
const FLYERS: usize = 3;
/// Largest |coordinate| after centring (0x454600 argument 160.0).
const SCALE: f32 = 160.0;
const CANVAS: (f32, f32) = (640.0, 400.0);
/// back.tga's colour-map length: the highest index a line can brighten to [I].
const RAMP_MAX: usize = 62;

struct Lob {
    verts: Vec<[f32; 3]>,
    edges: Vec<[u32; 2]>,
}

#[derive(Clone, Copy, Default)]
struct Flyer {
    lob: usize,
    speed: f32,
    pos: [f32; 3],
    /// angle offsets (12-bit tables) added to the clock
    phase: [u32; 3],
}

pub struct MenuFx {
    lobs: Vec<Lob>,
    flyers: [Flyer; FLYERS],
    /// the menu clock in milliseconds (0x702d28)
    t_ms: f64,
    rng: Rng,
    /// back.tga colour-map indices and colours: a line brightens the index under it
    back: Vec<u8>,
    ramp: Vec<[u8; 4]>,
    size: (usize, usize),
}

impl MenuFx {
    pub fn load(root: &Path) -> MenuFx {
        let lobs = (0..9)
            .filter_map(|i| {
                let raw = motor_data::fs::read(root, &format!("Data/LINEOBJ/lobj{i:02}.lob")).ok()?;
                let data = motor_data::tbl::load(&raw, None).ok()?.data;
                let l = motor_data::lob::parse(&data).ok()?;
                Some(normalise(l.vertices, l.edges))
            })
            .collect::<Vec<_>>();
        let (back, ramp, size) = motor_data::fs::read(root, "Data/back.tga")
            .ok()
            .and_then(|b| motor_data::tga::decode(&b).ok())
            .map_or((Vec::new(), Vec::new(), (0, 0)), |i| (i.index, i.palette, (i.width as usize, i.height as usize)));
        let mut fx = MenuFx { lobs, flyers: [Flyer::default(); FLYERS], t_ms: 0.0, rng: Rng(0x1998), back, ramp, size };
        fx.step(0.0);
        fx
    }

    /// Advance by `dt` seconds (0x46f060: z -= dt_ms * speed, respawn at z <= 200).
    pub fn update(&mut self, dt: f32) {
        self.t_ms += dt as f64 * 1000.0;
        self.step(dt * 1000.0);
    }

    fn step(&mut self, dt_ms: f32) {
        if self.lobs.is_empty() {
            return;
        }
        for k in 0..FLYERS {
            let f = &mut self.flyers[k];
            f.pos[2] -= dt_ms * f.speed;
            if f.pos[2] > 200.0 {
                continue;
            }
            let r = &mut self.rng;
            f.lob = r.next() as usize % self.lobs.len();
            f.speed = ((r.next() & 0x1fff) + 0x200) as f32 / 2048.0;
            let mut xy = [(r.next() & 0x7ff) as f32 - 1024.0, (r.next() & 0x7ff) as f32 - 1024.0];
            // keep the middle of the screen free: push one axis out to +-300
            if xy[0].abs() < 260.0 && xy[1].abs() < 260.0 {
                let sign = if r.next() & 1 == 0 { -1.0 } else { 1.0 };
                xy[(r.next() & 1) as usize] = sign * 300.0;
            }
            f.pos = [xy[0], xy[1], ((r.next() & 0x7ff) + 0x400) as f32];
            f.phase = [r.next(), r.next(), r.next()];
            let _lifetime = (r.next() & 0x3ff) + 0xc8; // 0x753a88, never read [I]
        }
    }

    pub fn draw(&self, b: &mut Batch) {
        // rings, colour index +16, turning opposite ways
        let a = (self.t_ms * 0.001) as f32;
        self.ring(b, (150.0, 100.0), 200.0, 6, a);
        self.ring(b, (150.0, 470.0), 200.0, 6, -a);
        let t = self.t_ms as u32;
        for f in &self.flyers {
            let Some(lob) = self.lobs.get(f.lob) else { continue };
            let c = (32.0 - f.pos[2] / 64.0).max(2.0).round() as usize;
            let ang = [t.wrapping_add(f.phase[0]), (t / 7).wrapping_add(f.phase[1]), (t / 3).wrapping_add(f.phase[2])];
            let pts = project(&lob.verts, ang, f.pos, (320.0, 200.0));
            for e in &lob.edges {
                if let (Some(Some(p)), Some(Some(q))) = (pts.get(e[0] as usize), pts.get(e[1] as usize)) {
                    self.line(b, *p, *q, c);
                }
            }
        }
    }

    /// 0x46ed4c: a ring of `segs * 4` chords whose radius alternates between R and 0.8 R, with
    /// notches joining them. x is squashed by 0.8333.
    fn ring(&self, b: &mut Batch, (cx, cy): (f32, f32), radius: f32, segs: u32, a: f32) {
        let step = std::f32::consts::FRAC_PI_2 / segs as f32;
        let g = step * 0.1;
        let at = |phi: f32, rho: f32| (cx + phi.sin() * rho * 0.8333, cy + phi.cos() * rho);
        for i in 0..segs * 4 {
            let k = i % 4;
            let r = radius - ((i / 2) & 1) as f32 * 0.2 * radius;
            let (mut a0, mut a1) = (i as f32 * step, (i + 1) as f32 * step);
            if k == 0 {
                a0 += g;
            }
            if k == 1 {
                a1 -= g;
            }
            let (a0, a1) = (a0 + a, a1 + a);
            let (p0, p1) = (at(a0, r), at(a1, r));
            self.line(b, p1, p0, 16);
            if k == 2 {
                self.line(b, at(a0 - g, radius), p0, 16);
            }
            if k == 3 {
                self.line(b, at(a1 + g, radius), p1, 16);
            }
        }
    }

    /// A line that raises the palette index under it by `add` (0x4061cc). APPROX: drawn as short
    /// pieces coloured from the colour map at the index below each piece's middle.
    fn line(&self, b: &mut Batch, p: (f32, f32), q: (f32, f32), add: usize) {
        let Some((p, q)) = clip(p, q) else { return };
        let len = ((q.0 - p.0).powi(2) + (q.1 - p.1).powi(2)).sqrt();
        let n = (len / 8.0).ceil().max(1.0) as usize;
        for i in 0..n {
            let (u0, u1) = (i as f32 / n as f32, (i + 1) as f32 / n as f32);
            let a = (p.0 + (q.0 - p.0) * u0, p.1 + (q.1 - p.1) * u0);
            let c = (p.0 + (q.0 - p.0) * u1, p.1 + (q.1 - p.1) * u1);
            let (mx, my) = ((a.0 + c.0) * 0.5, (a.1 + c.1) * 0.5);
            let (x, y) = ((mx as usize).min(self.size.0.saturating_sub(1)), (my as usize).min(self.size.1.saturating_sub(1)));
            let idx = self.back.get(y * self.size.0 + x).copied().unwrap_or(0) as usize;
            let col = self.ramp.get((idx + add).min(RAMP_MAX)).copied().unwrap_or([255, 255, 255, 255]);
            b.line(a.0, a.1, c.0, c.1, 1.0, col.map(|v| v as f32 / 255.0));
        }
    }
}

/// The projection of a line object (0x454c84): `ang` are 12-bit table angles (pitch, yaw, roll),
/// `pos` the object's offset (x, y in the buffer's units, z the distance), `centre` the screen
/// point the buffer centre maps to (0x703cbc / 0x703cc0). `None` = behind the eye.
pub fn project(verts: &[[f32; 3]], ang: [u32; 3], pos: [f32; 3], centre: (f32, f32)) -> Vec<Option<(f32, f32)>> {
    let rad = |v: u32| (v & 0xfff) as f32 * std::f32::consts::TAU / 4096.0;
    let (s6, c6) = rad(ang[0]).sin_cos();
    let (s7, c7) = rad(ang[1]).sin_cos();
    let (s8, c8) = rad(ang[2]).sin_cos();
    let [x, y, z] = pos;
    verts
        .iter()
        .map(|&[vx, vy, vz]| {
            let den = vz * c6 * c7 + vy * (s8 * s7 - c8 * s6 * c7) + vx * (s8 * s6 * c7 - c8 * s7) + z + 6.0;
            if den <= 0.5 {
                return None;
            }
            let sx = 288.0 * (vz * c6 * s7 + vx * (s8 * s6 * s7 + c8 * c7) + vy * (c8 * s6 * s7 - s8 * c7) + x) / den + centre.0;
            let sy = 240.0 * (-vz * s6 + vy * c8 * c6 + vx * s8 * c6 + y) / den + centre.1;
            Some((sx, sy))
        })
        .collect()
}

/// Centre a wireframe on its bounding box (0x45476c).
pub fn centre(verts: &mut [[f32; 3]]) {
    let (mut lo, mut hi) = ([f32::MAX; 3], [f32::MIN; 3]);
    for v in verts.iter() {
        for k in 0..3 {
            lo[k] = lo[k].min(v[k]);
            hi[k] = hi[k].max(v[k]);
        }
    }
    let mid: [f32; 3] = std::array::from_fn(|k| (lo[k] + hi[k]) * 0.5);
    for v in verts.iter_mut() {
        for k in 0..3 {
            v[k] -= mid[k];
        }
    }
}

/// Scale a wireframe by `k` (0x4545c0).
pub fn scale(verts: &mut [[f32; 3]], k: f32) {
    for v in verts.iter_mut().flatten() {
        *v *= k;
    }
}

/// Scale so the largest |coordinate| is `target` (0x454600); returns the factor (0 for an empty or
/// flat object, which is left alone).
pub fn fit(verts: &mut [[f32; 3]], target: f32) -> f32 {
    let big = verts.iter().flatten().fold(0.0f32, |m, v| m.max(v.abs()));
    if big <= 0.0 {
        return 0.0;
    }
    scale(verts, target / big);
    target / big
}

/// Centre a wireframe and scale the largest |coordinate| to 160 (0x45476c, 0x454600).
fn normalise(mut verts: Vec<[f32; 3]>, edges: Vec<[u32; 2]>) -> Lob {
    centre(&mut verts);
    fit(&mut verts, SCALE);
    Lob { verts, edges }
}

/// Liang-Barsky clip to the canvas.
fn clip(p: (f32, f32), q: (f32, f32)) -> Option<((f32, f32), (f32, f32))> {
    if !(p.0.is_finite() && p.1.is_finite() && q.0.is_finite() && q.1.is_finite()) {
        return None;
    }
    let (dx, dy) = (q.0 - p.0, q.1 - p.1);
    let (mut t0, mut t1) = (0.0f32, 1.0f32);
    for (pk, qk) in [(-dx, p.0), (dx, CANVAS.0 - p.0), (-dy, p.1), (dy, CANVAS.1 - p.1)] {
        if pk == 0.0 {
            if qk < 0.0 {
                return None;
            }
        } else {
            let r = qk / pk;
            if pk < 0.0 {
                t0 = t0.max(r);
            } else {
                t1 = t1.min(r);
            }
        }
    }
    (t0 < t1).then_some(((p.0 + dx * t0, p.1 + dy * t0), (p.0 + dx * t1, p.1 + dy * t1)))
}
