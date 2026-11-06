//! Tyre smoke, grass/dirt spray and sparks (re-notes/08-missing-features.md §8.6).
//!
//! Per car the original keeps three rings of 16 particles (0x40c bytes each, list 0x738170):
//! one per rear wheel and one for sparks. `0x445f84` spawns into them once per frame and
//! `0x445518` ages them. Positions are kept in game space here.

use crate::scene::FxLayers;
use glam::Vec3;
use motor_sim::session::ViewPose;
use motor_sim::{Car, World};
use std::collections::VecDeque;

/// Particles per ring (0x4453a4 wraps at 16).
const RING: usize = 16;
// Animation rate of every effect (0x445338(30.0) at 0x445db1..0x445e66) [O].
motor_tune::tunable! {
    /// Animation rate of every effect (0x445338, 0x445db1..0x445e66) [O].
    /// range: 1..120
    /// unit: Hz
    /// area: Graphics/Effects
    ANIM_FPS: f32 = 30.0;
}
// Spawns per second. The original spawns once per rendered frame (0x445f84 from 0x45c704) and
// renders at a variable rate with no frame cap (02-architecture "Frame dt"); we spawn at a fixed
// 30 Hz, about its frame rate on period hardware, so the trail length doesn't depend on ours
// [design choice].
motor_tune::tunable! {
    /// Classic particle spawns per second: the original spawns once per rendered frame (0x445f84); we spawn at a fixed rate [design choice].
    /// range: 5..240
    /// unit: Hz
    /// area: Graphics/Effects
    SPAWN_HZ: f32 = 30.0;
}
// Smoke grows by size * 0.5 * dt per update (0x4d38bd) and rises 1 unit/s (0x4455cb) [O].
motor_tune::tunable! {
    /// Classic smoke grows by size x this x dt per update (0x4d38bd) [O].
    /// range: 0..4
    /// area: Graphics/Effects
    SMOKE_GROWTH: f32 = 0.5;
    /// Classic smoke rises this fast (0x4455cb) [O].
    /// range: -4..8
    /// unit: m/s
    /// area: Graphics/Effects
    SMOKE_RISE: f32 = 1.0;
}
// Spark strength = contact speed * 0.03 (0x4d7f66), capped at 1; sparks need >= 0.5
// (0x4d3b2d) [O]. The contact value (rb+0x74c) is taken as the hull point's speed [I].
motor_tune::tunable! {
    /// Spark strength (contact speed x 0.03, capped at 1) needed to spawn sparks (0x4d3b2d) [O].
    /// range: 0..1
    /// area: Graphics/Effects
    SPARK_MIN: f32 = 0.5;
}
// CLASSIC particle drawer (D3D, 0x43b398) [O]: every particle is record type 0x43 (flag 0x40 =
// fade), drawn into list 0x4ebe38 with textured ONE/ONE blending, no fog, no z write, colour key
// off (0x442b75..0x442bd7). With r = life left / life (1 for sparks, which never count it down):
// size factor s = (1.5 - 0.5 r)^2 (0x4d3525, 0x4d34f5) and brightness k = 3 * (40 - 17.78 s)
// (0x4d350d, 0x4d352d, 0x4d3505), x 0.75 for the smoke anim (0x4d3515); the vertex colour is
// k * rgb as a byte. The (1 - z/far/far) term at 0x43b4ad is 0.99 or more and left out.
motor_tune::tunable! {
    /// Classic particle size factor s = (FADE_BASE - FADE_SLOPE r)^2, r = life left / life (0x4d3525).
    /// range: 0.5..4
    /// area: Graphics/Effects
    FADE_BASE: f32 = 1.5;
    /// Classic particle size factor slope (0x4d34f5).
    /// range: 0..2
    /// area: Graphics/Effects
    FADE_SLOPE: f32 = 0.5;
    /// Classic particle brightness k = GLOW_GAIN (GLOW_BASE - GLOW_SLOPE s) (0x4d350d).
    /// range: 0..200
    /// area: Graphics/Effects
    GLOW_BASE: f32 = 40.0;
    /// Classic particle brightness slope (0x4d352d).
    /// range: 0..100
    /// area: Graphics/Effects
    GLOW_SLOPE: f32 = 17.78;
    /// Classic particle brightness gain (0x4d3505).
    /// range: 0..12
    /// area: Graphics/Effects
    GLOW_GAIN: f32 = 3.0;
    /// Classic smoke brightness factor (0x4d3515).
    /// range: 0..4
    /// area: Graphics/Effects
    SMOKE_GLOW: f32 = 0.75;
}
// Sparks (anim flag 1 without 4) are drawn at 0.75 of the size (0x43b53d).
motor_tune::tunable! {
    /// Classic sparks are drawn at this share of the size (0x43b53d).
    /// range: 0.1..4
    /// area: Graphics/Effects
    SPARK_SIZE: f32 = 0.75;
}
// The quad is pulled this many metres towards the camera: 1 m, sparks 2 m (0x4d3535, 0x4d353d).
motor_tune::tunable! {
    /// The Classic particle quad is pulled this many metres towards the camera (0x4d3535).
    /// range: 0..8
    /// unit: m
    /// area: Graphics/Effects
    PULL: f32 = 1.0;
    /// The same for sparks (0x4d353d).
    /// range: 0..8
    /// unit: m
    /// area: Graphics/Effects
    PULL_SPARK: f32 = 2.0;
}

// REMASTER particles (09-remaster 9.16) [all design choices]
/// Ring size, spawn rate and smoke life: more, longer-lived puffs than the original's 16.
const RING_MODERN: usize = 96;
motor_tune::tunable! {
    /// Remaster smoke spawn rate: more, longer-lived puffs than the original [design choice].
    /// range: 5..240
    /// unit: Hz
    /// area: Graphics/Effects
    SPAWN_HZ_MODERN: f32 = 60.0;
}
// Smoke growth per second (half-extent fraction), rise and drag of the drift velocity.
motor_tune::tunable! {
    /// Remaster smoke growth per second (half-extent fraction).
    /// range: 0..4
    /// area: Graphics/Effects
    MODERN_GROWTH: f32 = 0.9;
    /// Remaster smoke rise.
    /// range: -4..8
    /// unit: m/s
    /// area: Graphics/Effects
    MODERN_RISE: f32 = 0.7;
    /// Remaster smoke drag of the drift velocity.
    /// range: 0..10
    /// unit: 1/s
    /// area: Graphics/Effects
    MODERN_DRAG: f32 = 1.6;
}
// Share of the car's velocity a puff starts with.
motor_tune::tunable! {
    /// Remaster: share of the car's velocity a puff starts with.
    /// range: 0..1
    /// area: Graphics/Effects
    MODERN_INHERIT: f32 = 0.3;
}
// Sparks per contact (at least, and at full strength), their gravity (the car's 15.81),
// ring size, streak length per m/s and its cap. Friction leaves them behind the car: they keep
// a quarter of its velocity and leave in a cone pointing backwards.
motor_tune::tunable! {
    /// Remaster: sparks per contact at least (weakest contact).
    /// range: 0..60
    /// area: Graphics/Effects
    SPARKS_MIN: usize = 10;
    /// Remaster: sparks per contact at full strength.
    /// range: 1..120
    /// area: Graphics/Effects
    SPARKS_MAX: usize = 26;
    /// Remaster spark gravity (the car's 15.81).
    /// range: 0..60
    /// unit: m/s2
    /// area: Graphics/Effects
    SPARK_GRAVITY: f32 = 15.81;
}
const RING_SPARKS: usize = 256;
motor_tune::tunable! {
    /// Remaster: friction leaves sparks behind the car: they keep this share of its velocity.
    /// range: 0..1
    /// area: Graphics/Effects
    SPARK_INHERIT: f32 = 0.25;
    /// Remaster spark streak length per m/s.
    /// range: 0..0.05
    /// unit: s
    /// area: Graphics/Effects
    SPARK_STREAK: f32 = 0.004;
    /// Remaster spark streak length cap.
    /// range: 0..1
    /// unit: m
    /// area: Graphics/Effects
    SPARK_STREAK_MAX: f32 = 0.12;
}
// Share of the speed and of the height a spark keeps after bouncing off the ground.
motor_tune::tunable! {
    /// Remaster: share of the speed and of the height a spark keeps after bouncing off the ground.
    /// range: 0..1
    /// area: Graphics/Effects
    SPARK_BOUNCE: f32 = 0.3;
    /// Remaster: share of the sideways speed a spark keeps after a bounce.
    /// range: 0..1
    /// area: Graphics/Effects
    SPARK_SLIDE: f32 = 0.6;
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Kind {
    /// rising, growing puff (smoke anim, flag 4)
    Smoke,
    /// grass spray (anim flag 0: plays in place)
    Grass,
    Spark,
}

#[derive(Clone, Copy, Debug)]
struct Particle {
    kind: Kind,
    /// game space
    pos: Vec3,
    /// half-width, half-height [I: world units]
    size: [f32; 2],
    rgb: [f32; 3],
    life: f32,
    age: f32,
    /// spark frame, picked in turn at spawn (anim flag 1)
    frame: usize,
    /// Classic sparks on the car's left side (draw mode 1, 0x446590): quad to the left, mirrored
    mirror: bool,
    /// REMASTER: spawned by the modern path (velocity, spin, ground fade, stretched sparks)
    modern: bool,
    vel: Vec3,
    rot: f32,
    spin: f32,
    /// height of the ground under the spawn point (game y)
    ground: f32,
    /// REMASTER smoke: the tyre effect's strength 0..1 (`Car::tyre_effect`); 1 for the rest
    strength: f32,
}

/// A camera-facing textured quad, view space.
#[derive(Clone, Copy, Debug)]
pub struct Sprite {
    pub pos: Vec3,
    pub half: [f32; 2],
    pub layer: f32,
    pub color: [f32; 4],
    /// added to the frame (every CLASSIC particle); false only for the REMASTER lit smoke
    pub additive: bool,
    pub mirror: bool,
    /// REMASTER: lit, alpha-blended smoke or dust (`Pass::Smoke`); false for the original's
    pub lit: bool,
    /// rotation about the view axis (radians)
    pub rot: f32,
    /// REMASTER sparks: velocity in view space; the quad stretches along it
    pub streak: Option<Vec3>,
    /// REMASTER smoke: ground height (y), the quad fades out below it
    pub ground: f32,
    /// CLASSIC (0x43b398): metres the quad is pulled towards the camera along its eye ray; 0 = not
    /// a Classic particle (also: its half size is capped at half the screen's half width)
    pub pull: f32,
    /// CLASSIC sparks: +1 the quad runs to the right of the point, -1 to the left, 0 centred
    pub side: f32,
}

pub struct Effects {
    /// per racer: rear-left, rear-right, sparks
    rings: Vec<[VecDeque<Particle>; 3]>,
    spawn_clock: f32,
    spark_frame: usize,
    seed: u32,
    pub layers: FxLayers,
    pub smoke: bool,
    pub sparks: bool,
    /// REMASTER particles (set every frame from the view's mode)
    pub remaster: bool,
    /// CHEAT ramlosa / h2o (`0x445fb9`, #317): both rear wheels spawn smoke on every spawn, whatever the surface or
    /// slip ("bubbles"); set every frame by the session
    pub bubbles: bool,
    /// particles spawned so far: smoke (incl. dirt), grass, sparks (headless checks)
    pub spawned: [usize; 3],
}

/// Watcom `rand()`: 0..=32767.
fn rand(seed: &mut u32) -> f32 {
    *seed = seed.wrapping_mul(1103515245).wrapping_add(12345);
    ((*seed >> 16) & 0x7fff) as f32
}

impl Effects {
    pub fn new(racers: usize) -> Effects {
        Effects { rings: (0..racers).map(|_| Default::default()).collect(), spawn_clock: 0.0, spark_frame: 0, seed: 1, layers: FxLayers::default(), smoke: true, sparks: true, remaster: false, bubbles: false, spawned: [0; 3] }
    }

    pub fn clear(&mut self) {
        self.rings.iter_mut().flatten().for_each(VecDeque::clear);
    }

    /// Age everything (0x445518), then spawn for each car (0x445f84) at its drawn pose (#374: `Race::view_pose`).
    /// `rear` gives each car's rear wheel ground points at that pose (game space); `skip` is the car whose effects are not
    /// spawned (the player's in the in-car view, 0x45c6de).
    pub fn step(&mut self, dt: f32, world: &World, cars: &[(&Car, ViewPose, [Vec3; 2])], skip: Option<usize>) {
        for p in self.rings.iter_mut().flatten().flatten() {
            p.age += dt;
            if p.modern {
                // REMASTER: drifting, spinning, growing puffs; ballistic sparks
                p.pos += p.vel * dt;
                p.rot += p.spin * dt;
                if p.kind == Kind::Spark {
                    p.vel.y -= SPARK_GRAVITY.get() * dt;
                    // bounces on the ground it was thrown over
                    if p.pos.y < p.ground && p.vel.y < 0.0 {
                        p.pos.y = p.ground;
                        p.vel.y = -p.vel.y * SPARK_BOUNCE.get();
                        p.vel.x *= SPARK_SLIDE.get();
                        p.vel.z *= SPARK_SLIDE.get();
                    }
                } else {
                    p.vel *= (-MODERN_DRAG.get() * dt).exp();
                    p.size = p.size.map(|s| s + s * MODERN_GROWTH.get() * dt);
                }
            } else if p.kind == Kind::Smoke {
                p.pos.y += SMOKE_RISE.get() * dt;
                p.size = p.size.map(|s| s + s * SMOKE_GROWTH.get() * dt);
            }
        }
        for ring in self.rings.iter_mut().flatten() {
            ring.retain(|p| p.age < p.life);
        }
        let step = 1.0 / (if self.remaster { SPAWN_HZ_MODERN.get() } else { SPAWN_HZ.get() }).max(0.1);
        self.spawn_clock += dt;
        if self.spawn_clock < step {
            return;
        }
        self.spawn_clock = (self.spawn_clock - step).min(step);
        for (i, (car, pose, rear)) in cars.iter().enumerate().filter(|&(i, _)| Some(i) != skip) {
            if self.smoke {
                self.spawn_tyres(i, car, pose, rear);
            }
            if self.sparks && !self.layers.sparks.is_empty() {
                self.spawn_sparks(i, car, pose, world, self.layers.sparks.len(), rear[0].y.min(rear[1].y));
            }
        }
    }

    /// The rear wheels' effect: `Car::tyre_effect` (0x47de5d..0x47dfc5 picks the material's
    /// ContactAnimID, 03-physics step 7), spawned as 0x445fd9.. does. The original reads only the
    /// type (P[0x170+4r]); the strength (P[0x180+4r]) is packed into the net record only (0x40bc38),
    /// so only REMASTER smoke uses it.
    fn spawn_tyres(&mut self, i: usize, car: &Car, pose: &ViewPose, rear: &[Vec3; 2]) {
        let seed = &mut self.seed;
        // (rand - 16384) / 16384: -1..1
        let mut r = |s: f32| (rand(seed) - 16384.0) * 6.103_515_6e-5 * s;
        for (w, &ground) in rear.iter().enumerate() {
            // CHEAT ramlosa / h2o (0x445fb9): type 1 for every rear wheel, every spawn
            let effect = if self.bubbles { Some((1, 1.0)) } else { car.tyre_effect[w] };
            let Some((anim, strength)) = effect else { continue };
            // offsets in car space: (x, y, z) spread and z shift; size spread; life spread;
            // colour (D3D path, 0x6fc340 == 2; the drawer scales it by `GLOW_*`, see `sprites`)
            let (kind, spread, dz, size_k, life_k, rgb) = match anim {
                // smoke (steel's type 4 spawns nothing: 0x4464b1 takes 0..3 only; under water every wheel is 1, 0x445fb9)
                1 => (Kind::Smoke, [0.3, 0.15, 0.3], -1.0, 0.1, 0.2, [1.5, 1.4, 1.5]),
                2 => (Kind::Grass, [0.1, 0.1, 0.3], -0.2, 0.1, 0.2, [1.2, 1.2, 1.2]),
                // dirt: the smoke animation, tinted (0.7, 0.6, 0.6)
                3 => (Kind::Smoke, [0.45, 0.15, 0.45], -1.0, 0.2, 0.3, [0.7, 0.6, 0.6]),
                _ => continue,
            };
            let dy = if kind == Kind::Grass { -0.3 } else { 0.0 };
            // APPROX: from the ground point lifted by the puff's size; the original starts at
            // the wheel record's position (P+0x35c + 0x3c * wheel)
            let local = Vec3::new(r(spread[0]), r(spread[1]) + dy + 0.3, r(spread[2]) + dz);
            let pos = ground + pose.rot * local;
            let size = [r(size_k) + 0.3, r(size_k) + 0.3];
            let life = r(life_k) + 1.0;
            self.spawned[kind as usize] += 1;
            let mut p = Particle { kind, pos, size, rgb, life, age: 0.0, frame: 0, mirror: false, modern: false, vel: Vec3::ZERO, rot: 0.0, spin: 0.0, ground: ground.y, strength: 1.0 };
            // CHEAT ramlosa / h2o: the bubbles stay the Classic puffs in Remaster too (small, rising; the modern smoke at
            // every spawn of every car fills the screen) [design choice]
            let cap = if self.remaster && !self.bubbles {
                // REMASTER: a puff that leaves with part of the car's speed, drifts up and
                // sideways, spins, and lives 1.4..2.2 s; dirt and grass are tinted dust
                p.modern = true;
                p.vel = car.world_vel() * MODERN_INHERIT.get() + Vec3::new(r(0.4), MODERN_RISE.get() + r(0.2), r(0.4));
                p.rot = r(std::f32::consts::PI);
                p.spin = r(0.8);
                p.strength = strength;
                p.size = [p.size[0] * 0.8 * (0.7 + 0.3 * strength); 2];
                p.life = 1.8 + r(0.4) + r(life_k);
                p.rgb = match anim {
                    3 => [0.62, 0.50, 0.38],
                    2 => [0.45, 0.50, 0.30],
                    _ => [0.95, 0.95, 0.95],
                };
                RING_MODERN
            } else {
                RING
            };
            push(&mut self.rings[i][w], p, cap);
        }
    }

    /// Sparks (0x4464fa..0x4465f3) at the frame's two world-hit slots (0x4813c7) whose material
    /// has SparkAnimID 1, size = strength * 0.75 (0x4d3b05), life 0.2 s cut to two updates.
    fn spawn_sparks(&mut self, i: usize, car: &Car, pose: &ViewPose, world: &World, frames: usize, ground: f32) {
        for h in car.world_hits.iter().flatten() {
            let strength = h.strength();
            if h.spark_anim(&world.materials) != 1 || strength < SPARK_MIN.get() {
                continue;
            }
            let point = car.hull_point(h.point);
            // pulled 0.4..1.4 towards the car's middle along z, 0.3 down (0x446540..)
            let u = rand(&mut self.seed) / 32768.0;
            let z = if point.z <= 0.0 { point.z + u + 0.4 } else { point.z - u - 0.4 };
            let local = Vec3::new(point.x, point.y - 0.3, z);
            let s = strength * 0.75;
            self.spark_frame = (self.spark_frame + 1) % frames;
            let p = Particle {
                kind: Kind::Spark,
                pos: pose.pos + pose.rot * local,
                size: [s, s],
                rgb: [6.0; 3],
                life: 2.0 / SPAWN_HZ.get().max(0.1), // Classic sparks live two updates (anim flag 1, 0x44555a) [O], at our spawn rate
                age: 0.0,
                frame: self.spark_frame,
                // draw mode 1 (x <= 0) runs the quad to the left with the texture mirrored, mode 3
                // (x > 0) to the right (0x4465a1, 0x43b612) [O]
                mirror: point.x <= 0.0,
                modern: false,
                vel: Vec3::ZERO,
                rot: 0.0,
                spin: 0.0,
                ground: 0.0,
                strength: 1.0,
            };
            self.spawned[2] += 1;
            if self.remaster {
                self.spawn_spark_shower(i, car, pose, point, strength, frames, ground);
                continue;
            }
            push(&mut self.rings[i][2], p, RING);
        }
    }

    /// REMASTER: a contact throws a shower of fine hot streaks backwards: a quarter of the
    /// car's velocity plus a random push in a cone about the car's -z, falling under gravity
    /// and bouncing on the ground [design choice].
    #[allow(clippy::too_many_arguments)]
    fn spawn_spark_shower(&mut self, i: usize, car: &Car, pose: &ViewPose, point: Vec3, strength: f32, frames: usize, ground: f32) {
        let origin = pose.pos + pose.rot * Vec3::new(point.x, point.y - 0.3, point.z);
        let side = if point.x > 0.0 { 1.0 } else { -1.0 };
        let n = SPARKS_MIN.get() + (SPARKS_MAX.get().saturating_sub(SPARKS_MIN.get()) as f32 * strength) as usize;
        for _ in 0..n {
            let mut r = |s: f32| (rand(&mut self.seed) - 16384.0) * 6.103_515_6e-5 * s;
            let dir = Vec3::new(side * 0.3 * r(1.0).abs() + r(0.35), 0.2 + 0.6 * r(1.0).abs(), -1.0).normalize();
            let speed = (4.0 + 10.0 * strength) * (1.0 + r(0.4));
            let vel = car.world_vel() * SPARK_INHERIT.get() + pose.rot * dir * speed;
            let pos = origin + pose.rot * Vec3::new(0.0, 0.0, r(0.15));
            let w = 0.006 + 0.006 * strength;
            let life = 0.15 + r(0.125).abs() * 2.0;
            self.spark_frame = (self.spark_frame + 1) % frames;
            let p = Particle {
                kind: Kind::Spark,
                pos,
                size: [w; 2],
                rgb: [1.0, 0.85, 0.5],
                life,
                age: 0.0,
                frame: self.spark_frame,
                mirror: false,
                modern: true,
                vel,
                rot: 0.0,
                spin: 0.0,
                ground,
                strength: 1.0,
            };
            push(&mut self.rings[i][2], p, RING_SPARKS);
        }
    }

    /// Sprites for the renderer (view space = game space with z negated).
    pub fn sprites(&self) -> Vec<Sprite> {
        let fx = &self.layers;
        let mut out = Vec::new();
        for p in self.rings.iter().flatten().flatten() {
            let (set, additive) = match p.kind {
                Kind::Smoke => (&fx.smoke, false),
                Kind::Grass => (&fx.grass, true),
                Kind::Spark => (&fx.sparks, true),
            };
            if set.is_empty() {
                continue;
            }
            // frame = elapsed * 30, held on the last one (0x445613)
            let frame = match p.kind {
                Kind::Spark => p.frame,
                _ => (p.age * ANIM_FPS.get()) as usize,
            };
            let flip = Vec3::new(1.0, 1.0, -1.0);
            let t = p.age / p.life;
            let mut sprite = Sprite { pos: p.pos * flip, half: p.size, layer: 0.0, color: [p.rgb[0], p.rgb[1], p.rgb[2], 1.0], additive, mirror: p.mirror, lit: false, rot: 0.0, streak: None, ground: 0.0, pull: 0.0, side: 0.0 };
            let mut frame = frame;
            if !p.modern {
                // CLASSIC 0x43b398: every particle adds (ONE/ONE), grows to 2.25 times its size
                // and fades out as it ages
                let left = if p.kind == Kind::Spark { 1.0 } else { (1.0 - t).clamp(0.0, 1.0) };
                let grow = (FADE_BASE.get() - FADE_SLOPE.get() * left).powi(2);
                let glow = (GLOW_GAIN.get() * (GLOW_BASE.get() - GLOW_SLOPE.get() * grow)).max(0.0) / 255.0;
                let glow = if p.kind == Kind::Smoke { glow * SMOKE_GLOW.get() } else { glow };
                let k = if p.kind == Kind::Spark { SPARK_SIZE.get() } else { 1.0 };
                sprite.additive = true;
                sprite.half = [p.size[0] * grow * k, p.size[1] * grow * k];
                sprite.color = [(p.rgb[0] * glow).min(1.0), (p.rgb[1] * glow).min(1.0), (p.rgb[2] * glow).min(1.0), 1.0];
                sprite.pull = if p.kind == Kind::Spark { PULL_SPARK.get() } else { PULL.get() };
                if p.kind == Kind::Spark {
                    sprite.side = if p.mirror { -1.0 } else { 1.0 };
                }
            } else if p.modern && p.kind == Kind::Spark {
                // REMASTER: a streak along its velocity, fading out
                sprite.streak = Some(p.vel * flip);
                sprite.half = [(p.vel.length() * SPARK_STREAK.get()).max(p.size[0]).min(SPARK_STREAK_MAX.get()), p.size[1]];
                sprite.color[3] = (1.0 - t).clamp(0.0, 1.0);
                // cools from white-yellow to orange
                sprite.color[1] = 0.85 - 0.4 * t;
                sprite.color[2] = 0.5 - 0.38 * t;
            } else if p.modern {
                // REMASTER: the frames play over the whole life; quick fade in, long fade out
                frame = (t * set.len() as f32) as usize;
                sprite.lit = true;
                sprite.rot = p.rot;
                sprite.ground = p.ground;
                sprite.additive = false;
                sprite.color[3] = (t / 0.08).min(1.0) * (1.0 - t).powf(1.3) * 0.85 * (0.4 + 0.6 * p.strength);
            }
            sprite.layer = set[frame.min(set.len() - 1)] as f32;
            out.push(sprite);
        }
        out
    }
}

fn push(ring: &mut VecDeque<Particle>, p: Particle, cap: usize) {
    if ring.len() >= cap {
        ring.pop_front();
    }
    ring.push_back(p);
}
