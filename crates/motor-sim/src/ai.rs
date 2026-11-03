//! The game's AI driver, ported from the per-substep AI step 0x407800 and the ai.dat loader
//! 0x408d74 (re-notes/04-track-car-data.md §4.3–4.4).
//!
//! ai.dat nodes carry a *signed turn radius* (m, 0 = straight); corner records carry the
//! corner's signed minimum radius. Target speed v = sqrt(|R| · 9.82 · AI[8]).
//! Places where the port simplifies are marked APPROX.
//! Re-checked against the disassembly for #133: look-ahead and braking window signs, throttle
//! ramp writes, the traffic passes and the corner-29 hack (re-notes/04 §4.4.1).

use crate::car::{Car, Input};
use glam::Vec3;
use motor_data::aidat::RacingLine;

const G_AI: f32 = 9.82; // 0x4cf020
const CORNER_BOOST: f32 = 1.5625; // 0x4cf018
const BRAKE_ABOVE: f32 = 1.52; // 0x4cf028
const EASE_ABOVE: f32 = 0.95; // 0x4cf02c
const THROTTLE_RAMP: f32 = 0.25; // 0x4cf030, per second
const SLOW: f32 = 13.888_889; // 0x4cf080, 50 km/h
const CRAWL: f32 = 1.388_889; // 0x4cf068, 5 km/h
const NO_LIMIT: f32 = 100.0; // target speed on straights (m/s)

/// Per-corner controller parameters built by the loader (0x408d74).
#[derive(Debug, Clone, Copy)]
struct CornerParams {
    gain: f32,   // 0.5 k
    damp: f32,   // 3.9 / k
    ff: f32,     // k · AI[9]
    learn: f32,  // starts at 1.0 and is never changed
    min_radius: f32,
}

/// Driver profile values (League/Profiles/*.adp "Aggressiveness", "Eagerness", 0..1).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Temper {
    pub aggression: f32,
    pub eagerness: f32,
}

impl Default for Temper {
    fn default() -> Self {
        Temper { aggression: 0.5, eagerness: 0.5 }
    }
}

/// What a car shows the others each frame (the fields traffic reads at `A+0x30..0x54`, 0x407f09).
#[derive(Debug, Clone, Copy)]
pub struct Traffic {
    pub slot: usize,
    pub node: usize,
    /// signed offset from the line (A+0x50) and the lane being steered to (A+0x4c), m
    pub offset: f32,
    pub lane: f32,
    pub speed: f32,
    /// car Width / Length (A+0x44 / A+0x48), m [I: CarDef 0x1218 / 0x1220]
    pub width: f32,
    pub length: f32,
    /// heading · node direction (A+0x54)
    pub align: f32,
}

/// The per-car part of `GameAi` (not the shared track data): what rewinding saves and restores.
#[derive(Debug, Clone)]
pub struct AiState {
    node: usize,
    pos: (f32, f32),
    look: usize,
    brake_window: i32,
    lane: f32,
    side_lock: f32,
    lat_prev: f32,
    ramp: f32,
    reverse: bool,
    reverse_t: f32,
    recover: bool,
    mood: [f32; 3],
    temper: Temper,
    rng: u32,
    decel: f32,
    offset: f32,
    speed: f32,
    align: f32,
    lat_vel: f32,
    speed_cap: f32,
}

pub struct GameAi {
    // shared track data
    x: Vec<f32>,
    z: Vec<f32>,
    wl: Vec<f32>,
    wr: Vec<f32>,
    dir: Vec<(f32, f32)>,
    radius: Vec<f32>,
    corner_of: Vec<usize>,
    corners: Vec<CornerParams>,
    a8: f32,
    /// ai.dat corner 29 has radius 150..151 (track 2): the loader's hard-coded hack, 0x407d3b
    hack29: bool,
    // per-car state (struct at car+0x1b49)
    slot: usize,
    width: f32,
    length: f32,
    node: usize,
    /// the car's ground position (x, z) at the start of its last tick (A+8 / A+0xc), for the
    /// race-order sort of the next one
    pos: (f32, f32),
    look: usize,
    brake_window: i32,
    lane: f32,
    side_lock: f32,
    lat_prev: f32,
    ramp: f32,
    reverse: bool,
    reverse_t: f32,
    recover: bool,
    mood: [f32; 3],
    temper: Temper,
    rng: u32,
    decel: f32,
    /// last computed signed offset from the line (m) — read by other cars for traffic
    pub offset: f32,
    pub speed: f32,
    /// heading · node direction of the last tick (A+0x54)
    align: f32,
    /// lateral velocity of the last tick (A+0x3c), read by the traffic code of the next one
    lat_vel: f32,
    /// REMASTER: an upper bound on the target speed (m/s); NO_LIMIT = the game's own
    pub speed_cap: f32,
}

/// Watcom-style rand(): 15-bit LCG on `state` (shared with the results fix-up, session.rs).
pub(crate) fn watcom_rand(state: &mut u32) -> u32 {
    *state = state.wrapping_mul(1_103_515_245).wrapping_add(12345);
    (*state >> 16) & 0x7fff
}

impl GameAi {
    /// The per-car state (rewind).
    pub fn state(&self) -> AiState {
        AiState {
            node: self.node,
            pos: self.pos,
            look: self.look,
            brake_window: self.brake_window,
            lane: self.lane,
            side_lock: self.side_lock,
            lat_prev: self.lat_prev,
            ramp: self.ramp,
            reverse: self.reverse,
            reverse_t: self.reverse_t,
            recover: self.recover,
            mood: self.mood,
            temper: self.temper,
            rng: self.rng,
            decel: self.decel,
            offset: self.offset,
            speed: self.speed,
            align: self.align,
            lat_vel: self.lat_vel,
            speed_cap: self.speed_cap,
        }
    }

    pub fn set_state(&mut self, s: &AiState) {
        (self.node, self.pos, self.look, self.brake_window, self.lane, self.side_lock, self.lat_prev, self.ramp) = (s.node, s.pos, s.look, s.brake_window, s.lane, s.side_lock, s.lat_prev, s.ramp);
        (self.reverse, self.reverse_t, self.recover, self.mood, self.temper, self.rng, self.decel) = (s.reverse, s.reverse_t, s.recover, s.mood, s.temper, s.rng, s.decel);
        (self.offset, self.speed, self.align, self.lat_vel, self.speed_cap) = (s.offset, s.speed, s.align, s.lat_vel, s.speed_cap);
    }

    /// CHEAT gulliver: the car's width and length for the traffic rules times `k`. The
    /// clearances in metres (0x4cf038..) stay [design choice].
    pub fn rescale(&mut self, k: f32) {
        self.width *= k;
        self.length *= k;
    }

    /// `ai` is the 11-float difficulty vector (AI1..3 / AID0..3); `mass` sets the planning
    /// deceleration −28000/mass (0x408d74).
    pub fn new(line: &RacingLine, ai: &[f32; 11], mass: f32, temper: Temper, slot: usize, width: f32, length: f32) -> GameAi {
        let corners = line
            .corners
            .iter()
            .map(|c| {
                let k = 1.0 - c.min_radius.abs() * 0.0015;
                CornerParams { gain: 0.5 * k, damp: 3.9 / k, ff: k * ai[9], learn: 1.0, min_radius: c.min_radius }
            })
            .collect();
        let mut s = GameAi {
            x: line.nodes.iter().map(|n| n.x).collect(),
            z: line.nodes.iter().map(|n| n.z).collect(),
            wl: line.nodes.iter().map(|n| n.w_left).collect(),
            wr: line.nodes.iter().map(|n| n.w_right).collect(),
            dir: line.nodes.iter().map(|n| (n.dir[0], n.dir[2])).collect(),
            radius: line.nodes.iter().map(|n| n.radius).collect(),
            corner_of: line.nodes.iter().map(|n| n.corner.max(0) as usize).collect(),
            corners,
            a8: ai[8],
            hack29: line.corners.get(29).is_some_and(|c| c.min_radius > 150.0 && c.min_radius < 151.0),
            slot,
            width,
            length,
            node: 0,
            pos: (0.0, 0.0),
            look: 0,
            brake_window: 0,
            lane: 0.0,
            side_lock: 0.0,
            lat_prev: 0.0,
            ramp: 1.0, // 0x4090f6
            reverse: false,
            reverse_t: 0.0,
            recover: false,
            mood: [temper.aggression * 0.5, temper.eagerness * 0.5, 0.25],
            temper,
            rng: (slot as u32 + 1).wrapping_mul(2_654_435_761).wrapping_add(1),
            decel: -28000.0 / mass,
            offset: 0.0,
            speed: 0.0,
            align: 1.0,
            lat_vel: 0.0,
            speed_cap: NO_LIMIT,
        };
        s.node = 0;
        s.look = 0;
        s
    }

    fn n(&self) -> usize {
        self.x.len()
    }

    /// Watcom-style rand(): 15-bit LCG.
    fn rand(&mut self) -> u32 {
        watcom_rand(&mut self.rng)
    }

    fn corner(&self, node: usize) -> &CornerParams {
        &self.corners[self.corner_of[node % self.n()].min(self.corners.len() - 1)]
    }

    /// Target speed at a node (m/s); straights return NO_LIMIT.
    fn target(&self, node: usize) -> f32 {
        let r = self.radius[node % self.n()];
        if r == 0.0 {
            return self.speed_cap;
        }
        let boost = if 1.0 - self.temper.eagerness < self.mood[1] { CORNER_BOOST } else { 1.0 };
        let l = self.corner(node).learn;
        (r.abs() * boost * G_AI * self.a8 * l * l).sqrt().min(self.speed_cap)
    }

    /// Nodes needed to change speed from `v` to `vt` at the planning deceleration.
    fn nodes_to(&self, vt: f32, v: f32) -> i32 {
        ((vt * vt - v * v) / self.decel * 0.5) as i32
    }

    fn dist2(&self, i: usize, p: (f32, f32)) -> f32 {
        let i = i % self.n();
        (self.x[i] - p.0).powi(2) + (self.z[i] - p.1).powi(2)
    }

    /// Place the tracker on the nearest node (call after (re)spawning).
    pub fn reset(&mut self, car: &Car) {
        let p = (car.pos.x, car.pos.z);
        self.pos = p;
        self.node = (0..self.n()).min_by(|&a, &b| self.dist2(a, p).total_cmp(&self.dist2(b, p))).unwrap_or(0);
        self.look = self.node;
        self.brake_window = 0;
        self.ramp = 1.0;
        self.lat_vel = 0.0;
        self.reverse = false;
        self.reverse_t = 0.0;
        self.recover = false;
        self.side_lock = 0.0;
        self.lane = 0.0;
    }

    /// After a respawn the car stands still: take the throttle up gently. APPROX [I]. The
    /// original's wheelspin cut (P[0x25], 0x407d80) never fires for AI cars in Single/League
    /// races (03-physics 3.2), so the port has no such flag.
    pub fn ease_in(&mut self) {
        self.ramp = 0.5;
    }

    /// What the other cars see of this one.
    pub fn traffic(&self) -> Traffic {
        Traffic { slot: self.slot, node: self.node, offset: self.offset, lane: self.lane, speed: self.speed, width: self.width, length: self.length, align: self.align }
    }

    /// Lap-independent progress (node index) used by the race order sort.
    pub fn node(&self) -> usize {
        self.node
    }

    /// The sort key of the AI step's prepass (0x40781e..0x40788b): the tracked node, then the
    /// car's projection on that node's direction, both as of its last tick [O]. The cars are
    /// walked from the largest key down.
    pub fn order_key(&self) -> (usize, f32) {
        let (dx, dz) = self.dir[self.node];
        (self.node, (self.pos.0 - self.x[self.node]) * dx + (self.pos.1 - self.z[self.node]) * dz)
    }

    /// One AI tick (called per physics substep in the original). `others` are the other AI
    /// states, for traffic.
    pub fn drive(&mut self, car: &Car, h: f32, others: &[Traffic]) -> Input {
        let n = self.n();
        let p = (car.pos.x, car.pos.z);
        self.pos = p;
        let fwd = car.rot * Vec3::Z;
        let hl = (fwd.x * fwd.x + fwd.z * fwd.z).sqrt().max(1e-6);
        let (hx, hz) = (fwd.x / hl, fwd.z / hl);
        self.speed = car.vel.z;
        let v = self.speed;

        // node tracking (0x4079xx): step towards whichever neighbour is closer
        for _ in 0..256 {
            let here = self.dist2(self.node, p);
            // leaving the look-ahead node moves it along, in either direction (0x4079f0); the
            // throttle ramp is not touched here
            if self.dist2(self.node + 1, p) < here {
                self.node = (self.node + 1) % n;
                if self.look == (self.node + n - 1) % n {
                    self.look = self.node;
                }
            } else if self.dist2(self.node + n - 1, p) < here {
                self.node = (self.node + n - 1) % n;
                if self.look == (self.node + 1) % n {
                    self.look = self.node;
                }
            } else {
                break;
            }
        }
        let i = self.node;
        let (dx, dz) = self.dir[i];
        let align = hz * dz + hx * dx;
        self.align = align;
        let facing = align >= 0.0;
        let (ox, oz) = (p.0 - self.x[i], p.1 - self.z[i]);
        let dist = (ox * ox + oz * oz).sqrt();
        let along = ox * dx + oz * dz;
        let mut off = if along.abs() <= dist { (dist * dist - along * along).max(0.0).sqrt() } else { 0.0 };
        if oz * dx - ox * dz > 0.0 {
            off = -off;
        }

        // mood random walks
        for (m, step) in [(0usize, 0.01f32), (1, 0.05), (2, 0.01)] {
            let r = self.rand();
            if r < 0x3fff {
                if self.mood[m] > 0.0 {
                    self.mood[m] -= step;
                }
            } else if self.mood[m] < 1.0 {
                self.mood[m] += step;
            }
        }

        // ---- pedal (0x407cxx)
        let v_cur = self.target(i);
        let mut pedal = 0.0f32;
        if self.reverse {
            pedal = -0.5;
        } else {
            let l = self.look;
            let corner_ahead = self.radius[l] != 0.0 && !self.behind(i, l, self.brake_window);
            if !corner_ahead {
                if self.radius[i] == 0.0 || v < v_cur {
                    pedal = 1.0;
                }
            } else {
                let v_l = self.target(l);
                if v > v_l * BRAKE_ABOVE {
                    pedal = -0.89;
                } else if (self.radius[i] != 0.0 && v >= v_cur) || v >= v_l * EASE_ABOVE {
                    if v <= v_l && (self.radius[i] == 0.0 || v < v_cur) && v < v_l {
                        pedal = 0.5;
                    }
                } else {
                    pedal = 1.0;
                }
            }
        }
        // hard-coded in the original: brake in corner 29 when its radius is 150..151 (track 2), 0x407d3b
        if self.hack29 && self.corner_of[i] == 29 && v > 50.0 {
            pedal = -0.89;
        }
        let (mut throttle, mut brake) = (0.0, 0.0);
        if pedal > 0.0 {
            if self.ramp < 1.0 {
                self.ramp += h * THROTTLE_RAMP;
            }
            throttle = pedal.min(self.ramp);
        } else if pedal < 0.0 {
            brake = -pedal;
        }

        // ---- look-ahead: the corner whose braking point comes soonest (0x407d97..0x407efc)
        // braking point = node − nodes needed to reach its speed (positive when braking is needed)
        let horizon = (v * v * 0.5 / self.decel) as i32; // negative = ahead
        let span = horizon.unsigned_abs().min(n as u32 / 2) as usize;
        for k in 0..span {
            let j = (i + k) % n;
            if self.radius[j] == 0.0 {
                continue;
            }
            let kl = ((self.look + n - i) % n) as i32;
            let start_j = k as i32 - self.nodes_to(self.target(j), v);
            let start_l = kl - self.nodes_to(self.target(self.look), v);
            if self.radius[self.look] == 0.0 || start_j <= start_l {
                self.look = j;
            }
        }
        let vt = self.target(self.look);
        if self.radius[self.look] != 0.0 && vt * vt < v * v {
            self.brake_window = self.nodes_to(vt, v); // 0x40878d: positive node count
        }

        // ---- traffic (0x407f09..0x4081c9)
        // The array the original walks is sorted by raw node index, so a car "ahead" has the
        // larger node and the wrap at the finish line is not seen.
        let mut shift = 0.0f32;
        let w = self.width;
        let is_ahead = |o: &Traffic| o.node > i || (o.node == i && o.slot < self.slot);
        let (mut left_blocked, mut right_blocked) = (false, false);
        // pass 1: cars alongside, only when in the mood (mood0 < 1 − aggression)
        if self.mood[0] < 1.0 - self.temper.aggression {
            for o in others.iter().filter(|o| !is_ahead(o)) {
                if (i as i32 - o.node as i32).abs() >= 5 {
                    continue;
                }
                let diff = off - o.offset;
                let clear = (w + o.width) * 0.5 + 0.1 + 0.2; // 0x4cf038/04c/044
                if diff > 0.0 && diff < clear {
                    left_blocked = true;
                }
                if diff < 0.0 && -diff < clear {
                    right_blocked = true;
                }
            }
        }
        // pass 2: cars ahead, nearest first
        let mut ahead: Vec<&Traffic> = others.iter().filter(|o| is_ahead(o)).collect();
        ahead.sort_by_key(|o| (o.node, std::cmp::Reverse(o.slot)));
        let mut seen = false;
        for o in ahead {
            let eff = (1.0 - o.align) * o.length + o.width * o.align;
            let room = 0.5 * eff + w + 0.2;
            let need = ((v - o.speed) * 5.0) as i32;
            if need < 0 {
                continue;
            }
            let gap = (o.node - i) as i32;
            let (wl, wr) = (self.wl[i], self.wr[i]);
            if gap > 0 && gap < need {
                seen = true;
                if o.speed * 1.01 >= v || (o.offset - off).abs() >= 0.5 * (w + eff) + 0.1 {
                    continue;
                }
                let can_right = wr - o.offset > room && wr - o.lane - shift > room && !right_blocked;
                let can_left = wl + o.offset > room && wl + o.lane + shift > room && !left_blocked;
                if o.offset < off {
                    // the other car is on the low-offset side: go to the right first
                    if can_right {
                        shift += 0.2;
                    } else if can_left {
                        shift -= 0.2;
                    } else {
                        throttle = 0.0;
                        brake = 1.0;
                    }
                } else if o.offset > off {
                    if can_left {
                        shift -= 0.2;
                    } else if can_right {
                        shift += 0.2;
                    } else {
                        throttle = 0.0;
                        brake = 1.0;
                    }
                }
                break;
            } else if o.node == i && !seen {
                // level with us: move away when already drifting towards it (A+0x3c, 0x408ad4)
                if o.offset < off {
                    if self.lat_vel < 0.0 && wr - self.lane - shift > room {
                        shift += 0.2;
                    }
                } else if o.offset > off && self.lat_vel > 0.0 && wl + self.lane + shift > room {
                    shift -= 0.2;
                }
            }
        }
        if shift == 0.0 {
            self.lane -= self.lane * 0.2;
        } else {
            // APPROX: the original has no clamp, our lane stays on the road
            self.lane = (self.lane + shift).clamp(-self.wl[i] + 1.5, self.wr[i] - 1.5);
        }

        // ---- reverse-out-of-trouble mode (0x2499)
        if !self.reverse {
            if v.abs() < CRAWL && pedal > 0.0 {
                self.reverse_t += h;
                if self.reverse_t > 2.0 {
                    self.reverse_t = 2.0;
                    self.reverse = true;
                }
            }
        } else {
            if self.reverse_t <= 0.0 {
                self.reverse = false;
            } else {
                self.reverse_t -= h;
            }
            if off.abs() < 2.0 && align > 0.8 {
                self.reverse = false;
                self.reverse_t = 0.0;
            }
        }

        // ---- steering (0x40827x..)
        let mut steer;
        if facing {
            self.side_lock = 0.0;
            let lat_vel = (off - self.lat_prev) / h.max(1e-4);
            self.lat_prev = off;
            self.lat_vel = lat_vel;
            let cp = *self.corner(i);
            let (mut gain, damp_k, ff_k) = if self.radius[i] == 0.0 { (0.5, 3.9, 0.0) } else { (cp.gain, cp.damp, cp.ff) };
            let mut ff = 0.0;
            if self.radius[i] != 0.0 {
                let r = (cp.min_radius + self.radius[i]) * 0.5;
                let boost = if 1.0 - self.temper.eagerness < self.mood[1] { CORNER_BOOST } else { 1.0 };
                let vc2 = (r.abs() * boost * G_AI * self.a8 * cp.learn * cp.learn).max(1e-3);
                ff = (ff_k / vc2.sqrt()).atan() * (v * v) / vc2;
                if cp.min_radius < 0.0 {
                    ff = -ff;
                }
            }
            let damp = if v.abs() >= SLOW || off < 5.0 { if v != 0.0 { damp_k * lat_vel / v } else { 0.0 } } else { 0.0 };
            if align.abs() < 0.9 {
                gain *= 2.0 - align.abs();
            }
            steer = (self.lane - off) * gain / (self.wl[i] + self.wr[i]).max(1.0) - damp + ff;
            if align.abs() < 0.5 {
                steer = if dx * hz - dz * hx <= 0.0 { -1.0 } else { 1.0 };
            }
        } else {
            if self.side_lock == 0.0 {
                self.side_lock = if self.wl[i] + off <= self.wr[i] - off { -1.0 } else { 1.0 };
            }
            steer = self.side_lock;
        }
        // off-track recovery (0x249d)
        if self.recover || (v < SLOW && (off < -self.wl[i] * 2.0 || off > self.wr[i] * 2.0)) {
            self.recover = true;
            if align.abs() > 0.5 {
                if pedal > 0.0 {
                    throttle = 1.0;
                }
                steer = if off >= 0.0 { -1.0 } else { 1.0 };
                if !facing {
                    steer = -steer;
                }
            } else if !self.reverse {
                steer = 0.0;
            }
            if (off < 0.0 && -off < self.wl[i]) || (off > 0.0 && off < self.wr[i]) {
                self.recover = false;
            }
        }
        if self.reverse {
            steer = -steer;
        }
        self.offset = off;
        Input { throttle, brake, steer: steer.clamp(-1.0, 1.0), ..Default::default() }
    }

    /// In the back-up-and-retry manoeuvre (ctrl+0x2499).
    pub fn reversing(&self) -> bool {
        self.reverse
    }

    /// Compact internal state for traces.
    pub fn debug(&self) -> String {
        format!(
            "node {} look {} win {} lane {:.1} off {:.1} rev {} rec {} ramp {:.2} r={:.0}",
            self.node, self.look, self.brake_window, self.lane, self.offset, self.reverse, self.recover, self.ramp, self.radius[self.node]
        )
    }

    /// Is node `i` still more than `window` nodes before `l` (i.e. outside the braking zone)?
    fn behind(&self, i: usize, l: usize, window: i32) -> bool {
        let n = self.n();
        let ahead = ((l + n - i) % n) as i32;
        ahead > window.max(0)
    }
}
