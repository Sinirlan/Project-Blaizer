//! Rigid body of a track prop (cone, sign, barrel): re-notes/04-track-car-data.md §4.9.1.
//! Ports `0x480d44` (integrate), the body path of `0x47f5e8` (world collide), `0x47f214`
//! (resting pass) and the impulse of `0x480400`. Velocities are in the body frame, like the
//! original's; `Pose::rot` columns are the body axes.

use glam::{Mat3, Vec3};

use crate::car::{Bump, Car};
use crate::props::Pose;
use crate::world::RayHit;

/// Segment test against the track collision mesh (`0x416840`): a -> b.
pub type Cast<'a> = &'a dyn Fn(Vec3, Vec3) -> Option<RayHit>;

motor_tune::tunable! {
    /// Gravity scale of a prop body in 0x480d44 (`0x4d7f5e`) [O].
    /// range: 0..2
    GRAVITY_SCALE: f32 = 0.87;
    /// Below this height a prop body freezes (`0x4d7f4e`) [O].
    /// range: -1000..0
    /// unit: m
    FLOOR_Y: f32 = -100.0;
    /// Seconds awake before a prop body that touches the world goes back to sleep (`0x4d7f56`) [O].
    /// range: 0..30
    /// unit: s
    SLEEP_AFTER: f32 = 6.0;
    /// Share of a prop body's linear speed that is left after a second in the air (`pow`, 0x480d44) [O].
    /// range: 0..1
    DAMP_AIR_LIN: f32 = 0.9;
    /// Share of a prop body's spin that is left after a second in the air (`pow`, 0x480d44) [O].
    /// range: 0..1
    DAMP_AIR_ANG: f32 = 0.9;
    /// Share of a prop body's linear speed that is left after a second in contact with the world (`pow`, 0x480d44) [O].
    /// range: 0..1
    DAMP_GROUND_LIN: f32 = 0.02;
    /// Share of a prop body's spin that is left after a second in contact with the world (`pow`, 0x480d44) [O].
    /// range: 0..1
    DAMP_GROUND_ANG: f32 = 0.3;
    /// Push-out overshoot of the world collide 0x47f5e8 (`0x4d7f16`) [O].
    /// range: 1..1.5
    PUSH_OUT: f32 = 1.03;
    /// Push of the car's resting pass 0x47eda0 per (1 - t) (`0x4d7ef6`) [O].
    /// range: 0..200
    POST_PUSH: f32 = 50.0;
    /// The scale of the y part of that resting push (`0x4d7efe`) [O].
    /// range: 0..1
    POST_Y: f32 = 0.2;
    /// Body-vs-body impulse factor: -restitution x closing speed (`0x4d7f3e`, -2) [O].
    /// range: 0..4
    BUMP_RESTITUTION: f32 = 2.0;
    /// Body-vs-body impulse factor: the share of the impulse that turns into spin (`0x4d7f46`) [O].
    /// range: 0..1
    BUMP_SPIN: f32 = 0.1;
    /// Passes of the world collide per substep (our choice).
    /// range: 1..10
    COLLIDE_PASSES: usize = 3;
    /// Separation applied to the prop in 0x480400 (`0x4d7f36`) [O].
    /// range: 0..3
    SEPARATION: f32 = 1.2;
    /// Bounding radius of a car for the sphere test of 0x480400 (rb+0x2c). APPROX [I]: the
    /// original's value was not traced; the test is only an early-out, so a generous one is safe.
    /// Per car `CarParams::radius` (scaled by the gulliver cheat).
    /// range: 1..10
    /// unit: m
    pub(crate) CAR_RADIUS: f32 = 2.6;
}

/// A convex collision hull as the COL files of cars and props store it: planes `n·p + d <= 0`
/// inside, normals outward (04 §4.9.1, `0x416714`).
#[derive(Clone, Debug, Default)]
pub struct Hull {
    planes: Vec<(Vec3, f32)>,
}

impl Hull {
    pub fn from_col(c: &motor_data::col::Collision) -> Hull {
        Hull { planes: c.faces.iter().map(|f| (Vec3::new(f.plane[0], f.plane[1], f.plane[2]), f.plane[3])).collect() }
    }

    /// An axis-aligned box, for objects whose COL is missing.
    pub fn from_box(min: Vec3, max: Vec3) -> Hull {
        let axes = [Vec3::X, Vec3::Y, Vec3::Z];
        Hull { planes: axes.iter().flat_map(|&a| [(a, -a.dot(max)), (-a, a.dot(min))]).collect() }
    }

    /// The hull of the body scaled by `k` about its origin (CHEAT gulliver).
    pub fn scaled(&self, k: f32) -> Hull {
        Hull { planes: self.planes.iter().map(|&(n, d)| (n, d * k)).collect() }
    }

    /// `0x416714`: how far along `dir` the point `p` has to travel to leave the hull, through
    /// the nearest face it is heading out of, and that face's normal. None when `p` is outside.
    pub(crate) fn exit(&self, p: Vec3, dir: Vec3) -> Option<(f32, Vec3)> {
        let mut best: Option<(f32, Vec3)> = None;
        for &(n, d) in &self.planes {
            let s = n.dot(p) + d;
            if s > 0.0 {
                return None;
            }
            let nd = n.dot(dir);
            if nd > 0.0 {
                let t = -s / nd.max(0.01); // 0x4cfcef
                if best.is_none_or(|(bt, _)| t < bt) {
                    best = Some((t, n));
                }
            }
        }
        best
    }
}

#[derive(Clone, Debug)]
pub struct Body {
    pub mass: f32,
    /// box inertia, mass/12 * (dy² + dz, dx² + dz, dx² + dy²) (0x455bf3..0x455c7f, 0x4d48d0 = 1/12):
    /// the depth dz enters unsquared [O: the loader squares the x minimum instead; an original
    /// slip, kept]
    inertia: Vec3,
    /// centre of mass: the middle of the MYO bounding box (object space)
    com: Vec3,
    /// the 8 bounding-box corners (object space)
    hull: [Vec3; 8],
    /// bounding radius (sphere test)
    pub radius: f32,
    /// the prop's own `.COL`, for the contact with cars (rb+0x764)
    shape: Hull,
    /// `ObjectColSound` id of a hit (rb+0x14 = ObjEdgeColor's 2nd value)
    id: u16,
}

/// The moving state of a body.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Dyn {
    /// body-frame velocity
    pub vel: Vec3,
    /// body-frame angular velocity (rad/s)
    pub ang: Vec3,
    pub awake: bool,
    timer: f32,
    /// has hit the world since the restart (rb+0x04)
    hit_world: bool,
    /// hull points that touched the world last substep (rb+0x2e8 records)
    /// and the world normal it touched
    touched: [Option<Vec3>; 8],
}

impl Body {
    pub fn new(mass: f32, min: Vec3, max: Vec3, radius: f32) -> Body {
        let d = max - min;
        let hull = std::array::from_fn(|k| Vec3::new(if k & 1 == 0 { min.x } else { max.x }, if k & 2 == 0 { min.y } else { max.y }, if k & 4 == 0 { min.z } else { max.z }));
        let inertia = mass / 12.0 * Vec3::new(d.y * d.y + d.z, d.x * d.x + d.z, d.x * d.x + d.y * d.y);
        Body { mass, inertia, com: (min + max) * 0.5, hull, radius, shape: Hull::from_box(min, max), id: 0 }
    }

    /// Use the object's `.COL` as its shape in car contacts.
    pub fn with_shape(mut self, shape: Hull) -> Body {
        self.shape = shape;
        self
    }

    /// The id the car hears when it hits this body (rb+0x14).
    pub fn with_id(mut self, id: u16) -> Body {
        self.id = id;
        self
    }

    pub fn com(&self) -> Vec3 {
        self.com
    }

    /// One substep of `h` seconds for a prop alone (`begin`, then `finish`).
    pub fn substep(&self, d: &mut Dyn, pose: &mut Pose, h: f32, gravity: f32, cast: Cast) {
        let prev = *pose;
        self.begin(d, pose, h, gravity);
        self.finish(d, pose, &prev, h, cast);
    }

    /// `0x480fbc` step 1: integrate an awake body. The caller keeps the pose from before it
    /// (rb+0x70) for `finish`.
    pub fn begin(&self, d: &mut Dyn, pose: &mut Pose, h: f32, gravity: f32) {
        if d.awake {
            self.integrate(d, pose, h, gravity);
        }
    }

    /// `0x480fbc` step 4: world collide (3 passes) and the resting pass of an awake body, after
    /// the cars have had their say. `prev` is the pose at the start of the substep.
    pub fn finish(&self, d: &mut Dyn, pose: &mut Pose, prev: &Pose, h: f32, cast: Cast) {
        if !d.awake {
            return;
        }
        let mut touched = [None; 8];
        for _ in 0..COLLIDE_PASSES.get() {
            if !self.collide(d, pose, prev, h, cast, &mut touched) {
                break;
            }
        }
        if d.hit_world {
            self.resting(d, pose, prev, h, cast);
        }
        d.touched = touched;
    }

    /// 0x480d44
    fn integrate(&self, d: &mut Dyn, pose: &mut Pose, h: f32, gravity: f32) {
        if pose.pos.y < FLOOR_Y.get() {
            d.timer = 0.0;
            d.awake = false;
            return;
        }
        let n = d.touched.iter().flatten().count();
        d.timer += h;
        if d.timer > SLEEP_AFTER.get() && n > 0 {
            d.timer = 0.0;
            d.awake = false;
        }
        d.vel += pose.rot.transpose() * Vec3::new(0.0, -gravity * GRAVITY_SCALE.get() * h, 0.0);
        let (lin, ang) = if n > 0 { (DAMP_GROUND_LIN.get(), DAMP_GROUND_ANG.get()) } else { (DAMP_AIR_LIN.get(), DAMP_AIR_ANG.get()) };
        d.vel *= lin.powf(h);
        d.ang *= ang.powf(h);
        advance(d, pose, h);
    }

    /// 0x47f5e8, body path. Returns whether a hull point hit the world.
    fn collide(&self, d: &mut Dyn, pose: &mut Pose, prev: &Pose, h: f32, cast: Cast, touched: &mut [Option<Vec3>; 8]) -> bool {
        let rb = CollideBody { points: &self.hull, com: self.com, mass: self.mass, inertia: self.inertia, scale: 1.0, car: false };
        let hit = world_collide(&rb, &mut d.vel, &mut d.ang, pose, prev, h, cast, &mut |k, hit, _| touched[k] = Some(hit.normal));
        d.hit_world |= hit.is_some();
        hit.is_some()
    }

    /// 0x47f214: slide along the surfaces the hull points crossed this substep (segment
    /// prev -> now, as in 0x47f5e8) and ease out of them by n * (1 - t) * h along the body axes.
    fn resting(&self, d: &mut Dyn, pose: &mut Pose, prev: &Pose, h: f32, cast: Cast) {
        let mut push = Vec3::ZERO;
        for &p in &self.hull {
            if let Some(hit) = cast(prev.transform(p), pose.transform(p)) {
                let n = pose.rot.transpose() * hit.normal;
                d.vel -= n * d.vel.dot(n);
                push += n * (1.0 - hit.t);
            }
        }
        pose.pos += pose.rot * (push * h);
    }

    /// The part of a world-space vector that points into a surface the body rests on is cut off
    /// (the contact planes of 0x480400).
    fn cut(&self, d: &Dyn, v: &mut Vec3) {
        cut_planes(d.touched.iter().flatten().copied(), v);
    }

    /// `0x480400`, a car (A) against this prop (B, the second list): wake the prop, push it out
    /// of the car, and trade the impulse both ways (`trade`). Returns whether they overlap.
    /// Deviations [I]: the separation is capped at the two radii; inertia is applied in each
    /// body's own frame (the original mixes the frames).
    pub fn car_contact(&self, d: &mut Dyn, pose: &mut Pose, car: &mut Car) -> bool {
        let (ra, rb) = (car.rot, pose.rot);
        let to_b = rb.transpose();
        let off = car.pos - pose.pos;
        if off.length() - car.p.radius() - (self.radius + self.com.length()) > 0.0 {
            return false;
        }
        let Some(dir) = (to_b * off).try_normalize() else { return false };
        let pts = car.body_points();
        let rec = deepest(&Side { hull: &car.p.hull, points: &pts, rot: ra }, &Side { hull: &self.shape, points: &self.hull, rot: rb }, off, dir);
        let Some((t, point, normal)) = rec else { return false };
        d.awake = true;
        let mut sep = rb * dir * (-t.min(car.p.radius() + self.radius) * SEPARATION.get());
        self.cut(d, &mut sep);
        pose.pos += sep;
        let r = rb * point - sep;
        let mut n = rb * normal;
        self.cut(d, &mut n);
        let a = Mover { vel: &mut car.vel, ang: &mut car.ang, rot: ra, inertia: car.p.inertia(), mass: car.p.mass };
        let b = Mover { vel: &mut d.vel, ang: &mut d.ang, rot: rb, inertia: self.inertia, mass: self.mass };
        let closing = trade(a, b, r, r - off, n);
        note_bump(car, self.id, -BUMP_RESTITUTION.get() * closing);
        true
    }
}

/// One body of a `0x480400` pair: its velocities (own frame), axes, inertia and mass.
struct Mover<'a> {
    vel: &'a mut Vec3,
    ang: &'a mut Vec3,
    rot: Mat3,
    inertia: Vec3,
    mass: f32,
}

/// `0x480400` step 6: the normal impulse between A and B along `n` (world, out of B). `r` is the
/// contact point from B's origin, `r_a` the same point from A's origin (world axes). A's point
/// velocity uses `r_a` (`0x42b960` takes the point into A's frame, 0x4806b0), B's uses `r`, and both
/// angular terms of the impulse share `r` [O]. Returns the closing speed `(vA - vB)·n` before the
/// impulse (negative = closing).
/// No closing test [O 0x48087e..0x480b47]: a pair already moving apart along `n` gets its normal
/// speed turned back too, which drags a car along when a faster one slides off its corner (#382).
fn trade(a: Mover, b: Mover, r: Vec3, r_a: Vec3, n: Vec3) -> f32 {
    let va = a.rot * *a.vel + r_a.cross(a.rot * *a.ang);
    let vb = b.rot * *b.vel + r.cross(b.rot * *b.ang);
    let closing = (va - vb).dot(n);
    let rn = r.cross(n);
    let acc_a = a.rot * ((a.rot.transpose() * rn) / a.inertia);
    let acc_b = b.rot * ((b.rot.transpose() * rn) / b.inertia);
    let inv = 1.0 / a.mass + 1.0 / b.mass;
    let k = inv + (acc_a.cross(r) + acc_b.cross(r)).dot(n);
    let j = -BUMP_RESTITUTION.get() * closing / if k > 1e-9 { k } else { inv };
    *a.vel += a.rot.transpose() * (n * (j / a.mass));
    *a.ang += a.rot.transpose() * (acc_a * (j * BUMP_SPIN.get()));
    *b.vel -= b.rot.transpose() * (n * (j / b.mass));
    *b.ang -= b.rot.transpose() * (acc_b * (j * BUMP_SPIN.get()));
    closing
}

/// rb+0x08 := 1, rb+0x0c := the other body's id, rb+0x734 := max(.., `impact`) (0x480400 step 6).
/// The id follows the latest hit, the strength the strongest [O: 0x480892 keeps the max; which
/// id wins when two bodies are hit in one frame is [U]].
fn note_bump(car: &mut Car, id: u16, impact: f32) {
    let prev = car.bump.map_or(0.0, |b| b.impact);
    car.bump = Some(Bump { id, impact: prev.max(impact) });
}

/// Remove the part of `v` that points into any of the planes with these (world) normals.
fn cut_planes(normals: impl Iterator<Item = Vec3>, v: &mut Vec3) {
    for n in normals {
        let k = v.dot(n);
        if k < 0.0 {
            *v -= n * k;
        }
    }
}

/// One side of a pair in `0x48017c`: its convex hull, its test points (own frame) and axes.
struct Side<'a> {
    hull: &'a Hull,
    points: &'a [Vec3],
    rot: Mat3,
}

/// `0x48017c`: the deepest point of A inside B's hull and of B inside A's, along `dir` (A - B in
/// B's frame; `off` = A.pos - B.pos in world space). Returns the exit distance, the point (B's
/// frame) and the normal (B's frame, pointing out of B).
fn deepest(a: &Side, b: &Side, off: Vec3, dir: Vec3) -> Option<(f32, Vec3, Vec3)> {
    let (to_a, to_b) = (a.rot.transpose(), b.rot.transpose());
    let mut rec: Option<(f32, Vec3, Vec3)> = None;
    for &p in a.points {
        let pb = to_b * (off + a.rot * p);
        match b.hull.exit(pb, dir) {
            Some((t, n)) if rec.is_none_or(|r| t > r.0) => rec = Some((t, pb, n)),
            _ => {}
        }
    }
    let dir_a = -(to_a * (b.rot * dir));
    for &p in b.points {
        let qa = to_a * (b.rot * p - off);
        match a.hull.exit(qa, dir_a) {
            Some((t, n)) if rec.is_none_or(|r| t > r.0) => rec = Some((t, p, -(to_b * (a.rot * n)))),
            _ => {}
        }
    }
    rec
}

/// `0x480400`, car A against car B (the first list): the same hull test and impulse as against
/// a prop, but the separation is shared, B.pos += S/2 and A.pos -= S/2. Both cars' inertia is
/// the stock `INERTIA` (`CarParams::inertia`, scaled by the gulliver cheat); B's cut planes are the track normals its hull touched last substep.
/// The caller visits ordered pairs, (A,B) and later (B,A), like the original (`Race::world_step`).
/// Deviation [I]: the separation is capped at the two radii. Returns whether the hulls overlap.
pub fn car_pair(a: &mut Car, b: &mut Car) -> bool {
    let (ra, rb) = (a.rot, b.rot);
    let to_b = rb.transpose();
    let off = a.pos - b.pos;
    let reach = a.p.radius() + b.p.radius();
    if off.length() > reach {
        return false;
    }
    let Some(dir) = (to_b * off).try_normalize() else { return false };
    let (pa, pb) = (a.body_points(), b.body_points());
    let rec = deepest(&Side { hull: &a.p.hull, points: &pa, rot: ra }, &Side { hull: &b.p.hull, points: &pb, rot: rb }, off, dir);
    let Some((t, point, normal)) = rec else { return false };
    let mut sep = rb * dir * (-t.min(reach) * SEPARATION.get());
    cut_planes(b.hull_hits.iter().map(|h| h.0), &mut sep);
    b.pos += sep * 0.5;
    a.pos -= sep * 0.5;
    let r = rb * point - sep;
    let mut n = rb * normal;
    cut_planes(b.hull_hits.iter().map(|h| h.0), &mut n);
    let ma = Mover { vel: &mut a.vel, ang: &mut a.ang, rot: ra, inertia: a.p.inertia(), mass: a.p.mass };
    let mb = Mover { vel: &mut b.vel, ang: &mut b.ang, rot: rb, inertia: b.p.inertia(), mass: b.p.mass };
    let closing = trade(ma, mb, r, r - off, n);
    // both cars hear it, as an id-0 body [I]
    note_bump(a, 0, -BUMP_RESTITUTION.get() * closing);
    note_bump(b, 0, -BUMP_RESTITUTION.get() * closing);
    true
}

/// The rigid body fields `0x47f5e8` reads: hull points (body space), centre of mass, mass,
/// inertia, and whether it is a car (rb flag 4: stiffer inertia, the ny^2 scaling).
pub(crate) struct CollideBody<'a> {
    pub points: &'a [Vec3],
    pub com: Vec3,
    pub mass: f32,
    pub inertia: Vec3,
    /// what `CAR_INERTIA_ADD` is scaled by (the car's `scale * feel`, CHEAT gulliver; 1.0 for props)
    pub scale: f32,
    pub car: bool,
}

/// The kept hit of a world collide pass (`0x47f5e8` steps 2-8).
pub(crate) struct WorldImpact {
    /// the hull point's index
    pub point: usize,
    pub hit: RayHit,
    /// the impulse j (the original keeps it whatever its sign)
    pub impulse: f32,
    /// the closing speed along the normal before the impulse (`v_p . n`, negative = closing)
    pub closing: f32,
    /// `|n.x| * vz` after the impulse (body frame)
    pub metric: f32,
}

// Extra inertia of a car in `0x47f5e8` (`0x4d7f06`, `0x4d7f0e`) [O]: one vector in the original, three numbers here.
motor_tune::tunable! {
    /// Extra inertia a car has in the world collide about its x axis, times the car's size.
    /// range: 0..10000
    /// unit: kg m²
    CAR_INERTIA_ADD_X: f32 = 2000.0;
    /// Extra inertia a car has in the world collide about its y axis, times the car's size.
    /// range: 0..10000
    /// unit: kg m²
    CAR_INERTIA_ADD_Y: f32 = 3000.0;
    /// Extra inertia a car has in the world collide about its z axis, times the car's size.
    /// range: 0..10000
    /// unit: kg m²
    CAR_INERTIA_ADD_Z: f32 = 2000.0;
}

/// The extra inertia of a car (`CAR_INERTIA_ADD_X`, `_Y`, `_Z`).
fn car_inertia_add() -> Vec3 {
    Vec3::new(CAR_INERTIA_ADD_X.get(), CAR_INERTIA_ADD_Y.get(), CAR_INERTIA_ADD_Z.get())
}

/// `0x47f5e8(rb, col, h)`: one pass. Every hull point is tested from where it was at the start of
/// the substep (`prev`) to where it is; the hit with the smallest t (under 1.1) is pushed out and
/// gets a normal impulse. `record(k, hit, b - a)` sees every hit (the +0x508 records).
/// The correction after the impulse moves and turns the body by dv*h and dw*h only (the
/// position already advanced with the old velocity this substep, 0x47fbb2..0x47fc5a), for cars
/// and props alike. Props skip an impulse that would pull a point back into the surface; the
/// original has no such test [design choice, 04 §4.9.1], cars follow the original.
#[allow(clippy::too_many_arguments)]
pub(crate) fn world_collide(rb: &CollideBody, vel: &mut Vec3, ang: &mut Vec3, pose: &mut Pose, prev: &Pose, h: f32, cast: Cast, record: &mut dyn FnMut(usize, &RayHit, Vec3)) -> Option<WorldImpact> {
    let mut best: Option<(usize, RayHit, Vec3, Vec3)> = None;
    let mut limit = 1.1; // 0x47f607
    for (k, &p) in rb.points.iter().enumerate() {
        let (a, b) = (prev.transform(p), pose.transform(p));
        if let Some(hit) = cast(a, b) {
            record(k, &hit, b - a);
            if hit.t < limit {
                limit = hit.t;
                best = Some((k, hit, a, b));
            }
        }
    }
    let (k, hit, a, b) = best?;
    let n = pose.rot.transpose() * hit.normal;
    let r = rb.points[k] - rb.com;
    let vp = *vel + ang.cross(r);
    let inertia = if rb.car { rb.inertia + car_inertia_add() * rb.scale } else { rb.inertia };
    let acc = r.cross(n) / inertia;
    pose.pos += hit.normal * ((b - a).dot(hit.normal) * (hit.t - 1.0) * PUSH_OUT.get());
    let closing = vp.dot(n);
    let impulse = -closing / (1.0 / rb.mass + acc.cross(r).dot(n));
    if rb.car || closing < 0.0 {
        let mut dv = n * (impulse / rb.mass);
        let mut dw = acc * impulse;
        if rb.car {
            let k2 = hit.normal.y * hit.normal.y;
            dv.y *= k2;
            dw.x *= k2;
            dw.z *= k2;
            if dv.y > 0.0 {
                dv.y = dv.y.sqrt();
            }
        }
        *vel += dv;
        *ang += dw;
        nudge(vel, pose, dv * h, dw * h);
    }
    Some(WorldImpact { point: k, impulse, closing, metric: n.x.abs() * vel.z, hit })
}

/// Move along the body's own axes by `lin`, then turn about them by `rot` (x, y, z) and
/// re-express the velocity in the new frame (0x42bf0c.., 0x42b88c).
fn nudge(vel: &mut Vec3, pose: &mut Pose, lin: Vec3, rot: Vec3) {
    pose.pos += pose.rot * lin;
    let t = Mat3::from_rotation_x(rot.x) * Mat3::from_rotation_y(rot.y) * Mat3::from_rotation_z(rot.z);
    pose.rot *= t;
    *vel = t.transpose() * *vel;
}

/// `0x47eda0`, the car's resting pass after the collide passes: every hull point whose
/// segment prev -> now touches the surface (within the 0.08 m skin) loses the velocity into it
/// and adds a push of `n * (1 - t) * 50` (y scaled by `0.2 ny^2`); the body then moves by the
/// sum times h. Returns whether anything touched and the material of the first hit.
pub(crate) fn world_post(points: &[Vec3], vel: &mut Vec3, pose: &mut Pose, prev: &Pose, h: f32, cast: Cast) -> Option<usize> {
    let mut acc = Vec3::ZERO;
    let mut material = None;
    for &p in points {
        let Some(hit) = cast(prev.transform(p), pose.transform(p)) else { continue };
        material.get_or_insert(hit.material);
        let n = pose.rot.transpose() * hit.normal;
        *vel -= n * vel.dot(n);
        let k2 = hit.normal.y * hit.normal.y;
        if vel.y > 0.0 {
            vel.y *= k2;
        }
        let mut out = n * ((1.0 - hit.t) * POST_PUSH.get());
        out.y *= POST_Y.get() * k2;
        acc += out;
    }
    pose.pos += pose.rot * (acc * h);
    material
}

/// Move along the body's own axes by v*h, turn about them by w*h, and re-express v in the new
/// frame (0x42bf0c.., 0x42b88c).
fn advance(d: &mut Dyn, pose: &mut Pose, h: f32) {
    pose.pos += pose.rot * d.vel * h;
    let w = d.ang.length();
    if w > 0.0 {
        let turn = Mat3::from_axis_angle(d.ang / w, w * h);
        pose.rot *= turn;
        d.vel = turn.transpose() * d.vel;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pose(pos: Vec3) -> Pose {
        Pose { pos, rot: Mat3::IDENTITY, scale: Vec3::ONE }
    }

    /// flat ground at y = 0
    fn ground(a: Vec3, b: Vec3) -> Option<RayHit> {
        (a.y > 0.0 && b.y <= 0.0).then(|| {
            let t = a.y / (a.y - b.y);
            RayHit { t, point: a + (b - a) * t, normal: Vec3::Y, plane_d: -(a.y + (b.y - a.y) * t), material: 0 }
        })
    }

    #[test]
    fn hull_exit_distance() {
        let h = Hull::from_box(Vec3::splat(-1.0), Vec3::splat(1.0));
        // from x = 0.5, leaving along +x takes 0.5; along (1, 1, 0) the +x face is nearer in t than +y only at equal depth
        let (t, n) = h.exit(Vec3::new(0.5, 0.0, 0.0), Vec3::X).unwrap();
        assert!((t - 0.5).abs() < 1e-6 && n == Vec3::X);
        let (t, n) = h.exit(Vec3::new(0.0, 0.8, 0.0), Vec3::new(1.0, 1.0, 0.0).normalize()).unwrap();
        assert!((t - 0.2 * 2f32.sqrt()).abs() < 1e-5 && n == Vec3::Y);
        assert!(h.exit(Vec3::new(1.5, 0.0, 0.0), Vec3::X).is_none());
    }

    #[test]
    fn box_dropped_settles_on_flat_ground() {
        let body = Body::new(100.0, Vec3::new(-0.3, 0.0, -0.3), Vec3::new(0.3, 0.8, 0.3), 0.9);
        let mut d = Dyn { awake: true, ..Dyn::default() };
        let mut p = pose(Vec3::new(0.0, 2.0, 0.0));
        for _ in 0..(8 * 25) {
            body.substep(&mut d, &mut p, 0.04, 15.81, &ground);
        }
        // upright box rests with its base on the ground, not through it, and has stopped; it
        // hops a few cm above the ground like the traced collide does (the velocity of a body
        // that falls asleep is left as it was, like the original's)
        assert!(p.pos.y > -0.05 && p.pos.y < 0.1, "y {}", p.pos.y);
        assert!(!d.awake, "still awake, vel {:?}", d.vel);
        // (it can sleep a few degrees off: the flat test ground has no skin, 0.95 = the worst
        // standing prop of the conehit sweep over all 8 tracks)
        assert!(p.rot.y_axis.y > 0.95, "tilted {:?} y {}", p.rot.y_axis, p.pos.y);
    }

    /// Tilted and spinning, it lands on an edge, tips over onto a face (the rotation part of the
    /// dv*h / dw*h correction), never ends up under the ground, and falls asleep.
    #[test]
    fn tilted_box_tips_onto_a_face_and_sleeps() {
        let body = Body::new(100.0, Vec3::new(-0.3, 0.0, -0.3), Vec3::new(0.3, 0.8, 0.3), 0.9);
        let mut d = Dyn { awake: true, ang: Vec3::new(2.0, 1.0, 0.0), ..Dyn::default() };
        let mut p = pose(Vec3::new(0.0, 2.0, 0.0));
        p.rot = Mat3::from_rotation_x(30f32.to_radians());
        let mut lowest = f32::MAX;
        for _ in 0..(10 * 25) {
            body.substep(&mut d, &mut p, 0.04, 15.81, &ground);
            lowest = lowest.min(body.hull.iter().map(|&c| p.transform(c).y).fold(f32::MAX, f32::min));
        }
        assert!(!d.awake, "still awake, vel {:?} ang {:?}", d.vel, d.ang);
        // no corner went deeper than the push-out overshoot
        assert!(lowest > -0.1, "corner at y {lowest}");
        // a face is down: the body's up axis is vertical (upright or upside down) or flat on its side
        let up = p.rot.y_axis.y.abs();
        assert!(up > 0.95 || up < 0.05, "rests tilted, up {up}");
    }
}
