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

motor_tune::tunable! {
    /// The gravity the AI plans its corner speed with: v = sqrt(|R| · this · AI[8]) [O] 0x4cf020 (not the world's).
    /// range: 5..20
    /// unit: m/s²
    G_AI: f32 = 9.82;
    /// Corner speed factor of an eager driver (target speed × this when its mood allows) [O] 0x4cf018.
    /// range: 1..2.5
    CORNER_BOOST: f32 = 1.5625;
    /// The AI brakes when its speed is above the corner's target speed × this [O] 0x4cf028.
    /// range: 1..2.5
    BRAKE_ABOVE: f32 = 1.52;
    /// The AI eases off the throttle above the target speed × this [O] 0x4cf02c.
    /// range: 0.5..1.2
    EASE_ABOVE: f32 = 0.95;
    /// How fast the AI's throttle ramps back up after easing off [O] 0x4cf030.
    /// range: 0..2
    /// unit: /s
    THROTTLE_RAMP: f32 = 0.25;
    /// The AI steers without its lateral-speed damping below this speed unless it is close to its line, and starts a
    /// recovery below it when it is far off the track (50 km/h) [O] 0x4cf080.
    /// range: 1..40
    /// unit: m/s
    SLOW: f32 = 13.888_889;
    /// Below this speed with the pedal down the AI counts as stuck and, after 2 s, backs up (5 km/h) [O] 0x4cf068.
    /// range: 0..10
    /// unit: m/s
    CRAWL: f32 = 1.388_889;
    /// The AI's target speed on straights (no corner ahead) [D].
    /// range: 30..200
    /// unit: m/s
    NO_LIMIT: f32 = 100.0;
}

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

/// The terms of the last tick's pedal and steering (diagnosis: `aiaudit`, #403).
#[derive(Debug, Clone, Copy, Default)]
pub struct AiTerms {
    /// A+0x40 before the ramp: -0.89 brake, -0.5 reverse, 0 / 0.5 / 1.0
    pub pedal: f32,
    /// target speed of the current node and of the look target, m/s
    pub v_cur: f32,
    pub v_look: f32,
    pub look: usize,
    pub window: i32,
    /// the look target is a corner and the car is inside its braking window
    pub in_window: bool,
    /// `(lane − offset)·gain/(wl + wr)` with the alignment factor, the lateral damping and the feed-forward
    pub lane_term: f32,
    pub damp: f32,
    pub ff: f32,
    /// gain × (2 − |align|) below 0.9 alignment (1 otherwise)
    pub align_k: f32,
    /// the lane term per metre of `lane − offset`: gain (with `align_k`) / (wl + wr)
    pub lane_k: f32,
    /// the traffic pass shifted the lane this tick (steer × 2)
    pub shifted: bool,
    /// the corner boost was on
    pub boost: bool,
    /// 0 the steering law, 1 full lock below 0.5 alignment, 2 facing backwards, 3 recovery, +4 reversing
    pub mode: u8,
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
    /// the last tick's steering before the ±1 clamp (diagnosis: aicompare --dump)
    pub raw_steer: f32,
    /// the last tick's pedal and steering terms (diagnosis: aiaudit, #403)
    pub terms: AiTerms,
    /// diagnosis (aiaudit, #403): the corner boost forced on or off instead of the mood walk; `None` in the game
    pub boost_override: Option<bool>,
    /// diagnosis (aiaudit, #403): the moods after this tick's walk, from a trace (A+0x844..0x84c), so the walk is
    /// skipped; `None` in the game
    pub mood_override: Option<[f32; 3]>,
}

motor_tune::tunable! {
    /// The width every car has in the AI's traffic tests (A+0x44): the table at 0x4a7db0 holds 2.1 x 4.4 for
    /// all 8 slots and nothing writes it [O, #379]. A car takes it when the race is built.
    /// range: 1..4
    /// unit: m
    /// applies: restart
    pub TRAFFIC_WIDTH: f32 = 2.1;
    /// The length every car has in the AI's traffic tests (A+0x48), see `TRAFFIC_WIDTH` [O, #379].
    /// range: 2..8
    /// unit: m
    /// applies: restart
    pub TRAFFIC_LENGTH: f32 = 4.4;
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

    /// Diagnosis (aiaudit, #403): start the next tick from a recorded one, the tracked node (A+0), the lane (A+0x4c),
    /// the offset of the last tick (A+0x50) and its lateral velocity (A+0x3c). The rest (look, window, ramp, moods,
    /// modes) stays this tracker's own.
    pub fn inject(&mut self, node: usize, lane: f32, offset: f32, lat_vel: f32) {
        // walk there node by node with the tracker's look drag (0x4079f0), so a one-node difference does not leave
        // the look target behind
        let n = self.n();
        let node = node % n;
        for _ in 0..256 {
            let fwd = (node + n - self.node) % n;
            if fwd == 0 {
                break;
            }
            let old = self.node;
            self.node = if fwd <= n / 2 { (old + 1) % n } else { (old + n - 1) % n };
            if self.look == old {
                self.look = self.node;
            }
        }
        self.node = node;
        (self.lane, self.lat_prev, self.offset, self.lat_vel) = (lane, offset, offset, lat_vel);
    }

    /// Diagnosis (aiaudit, #403): the history the next tick starts from, as a trace records it after the tick before:
    /// the look-ahead node (A+0x4), the braking window (A+0x60, nodes) and the throttle ramp (0x4db1c4[A+0x64]).
    pub fn inject_history(&mut self, look: usize, window: i32, ramp: f32) {
        let n = self.n();
        (self.look, self.brake_window, self.ramp) = (look % n, window, ramp);
    }

    /// The look-ahead node, the braking window (nodes), the throttle ramp and the moods after the last tick (diagnosis).
    pub fn history(&self) -> (usize, i32, f32, [f32; 3]) {
        (self.look, self.brake_window, self.ramp, self.mood)
    }

    pub fn temper(&self) -> Temper {
        self.temper
    }

    /// Diagnosis (aiaudit, #403): the profile values of a traced slot, read back from its moods on the grid (twice
    /// mood0 / mood1: the init halves them, 0x40903c).
    pub fn set_temper(&mut self, t: Temper) {
        self.temper = t;
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
            speed_cap: NO_LIMIT.get(),
            raw_steer: 0.0,
            terms: AiTerms::default(),
            boost_override: None,
            mood_override: None,
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

    /// The corner boost is on: `mood1 > 1 − Eagerness` (unless a diagnosis forces it).
    fn boosted(&self) -> bool {
        self.boost_override.unwrap_or(1.0 - self.temper.eagerness < self.mood[1])
    }

    /// Target speed at a node (m/s); straights return NO_LIMIT.
    fn target(&self, node: usize) -> f32 {
        let r = self.radius[node % self.n()];
        if r == 0.0 {
            return self.speed_cap;
        }
        let boost = if self.boosted() { CORNER_BOOST.get() } else { 1.0 };
        let l = self.corner(node).learn;
        // CLASSIC 0x4086b1..0x4086c3: a corner's target is not capped; the look-ahead ranks the nodes of a fast corner
        // (R > ~380 m on Hard) by their real targets and lands on its tightest node, not its first [O, #403 Phase 4]
        (r.abs() * boost * G_AI.get() * self.a8 * l * l).sqrt()
    }

    /// Nodes needed to change speed from `v` to `vt` at the planning deceleration.
    fn nodes_to(&self, vt: f32, v: f32) -> i32 {
        window_nodes(vt * vt, v, self.decel)
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

    /// The tracking part of an AI tick: node, speed, alignment and offset (0x40789d..0x407ac0).
    /// Returns the car's heading on the ground `(hx, hz)` and the signed offset from the line.
    fn track_node(&mut self, car: &Car) -> (f32, f32, f32) {
        let n = self.n();
        let p = (car.pos.x, car.pos.z);
        self.pos = p;
        let fwd = car.rot * Vec3::Z;
        let hl = (fwd.x * fwd.x + fwd.z * fwd.z).sqrt().max(1e-6);
        let (hx, hz) = (fwd.x / hl, fwd.z / hl);
        self.speed = car.vel.z;

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
        self.align = hz * dz + hx * dx;
        let (ox, oz) = (p.0 - self.x[i], p.1 - self.z[i]);
        let dist = (ox * ox + oz * oz).sqrt();
        let along = ox * dx + oz * dz;
        let mut off = if along.abs() <= dist { (dist * dist - along * along).max(0.0).sqrt() } else { 0.0 };
        if oz * dx - ox * dz > 0.0 {
            off = -off;
        }
        (hx, hz, off)
    }

    /// A human car's AI tick: only the tracking part. CLASSIC 0x40789d..0x407ac0: the prepass writes node, speed,
    /// alignment and offset, then skips the rest (`car+0x1c != 2`, 0x407ab2), so the lane A+0x4c and the lateral
    /// velocity A+0x3c stay as initialised for the traffic the AI cars look at [O, #404].
    pub fn track(&mut self, car: &Car) {
        let (_, _, off) = self.track_node(car);
        self.offset = off;
    }

    /// One AI tick (called per physics substep in the original). `others` are the other AI
    /// states, for traffic.
    pub fn drive(&mut self, car: &Car, h: f32, others: &[Traffic]) -> Input {
        let (hx, hz, off) = self.track_node(car);
        let v = self.speed;
        let i = self.node;
        let (dx, dz) = self.dir[i];
        let align = self.align;
        let facing = align >= 0.0;
        let n = self.n();

        // mood random walks (0x407c1c..0x407cb4; a diagnosis may hand in the traced result instead)
        if let Some(m) = self.mood_override {
            self.mood = m;
        } else {
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
        }

        // ---- pedal (0x407cxx)
        let v_cur = self.target(i);
        let mut pedal = 0.0f32;
        if self.reverse {
            pedal = -0.5;
        } else {
            let l = self.look;
            let corner_ahead = self.radius[l] != 0.0 && self.in_window(i, l, self.brake_window);
            if !corner_ahead {
                if self.radius[i] == 0.0 || v < v_cur {
                    pedal = 1.0;
                }
            } else {
                let v_l = self.target(l);
                if v > v_l * BRAKE_ABOVE.get() {
                    pedal = -0.89;
                } else if (self.radius[i] != 0.0 && v >= v_cur) || v >= v_l * EASE_ABOVE.get() {
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
        self.terms = AiTerms {
            pedal,
            v_cur,
            v_look: self.target(self.look),
            look: self.look,
            window: self.brake_window,
            in_window: self.radius[self.look] != 0.0 && self.in_window(i, self.look, self.brake_window),
            boost: self.boosted(),
            align_k: 1.0,
            ..AiTerms::default()
        };
        let (mut throttle, mut brake) = (0.0, 0.0);
        if pedal > 0.0 {
            if self.ramp < 1.0 {
                self.ramp += h * THROTTLE_RAMP.get();
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
            // CLASSIC 0x407ed2: the braking points are compared as raw node pointers, not as distances
            // ahead, so across the finish-line wrap a node of the next lap (small index) always wins
            // against a look target before the wrap [O, #378]
            let start_j = j as i32 - self.nodes_to(self.target(j), v);
            let start_l = self.look as i32 - self.nodes_to(self.target(self.look), v);
            if self.radius[self.look] == 0.0 || start_j <= start_l {
                self.look = j;
            }
        }
        // CLASSIC 0x407eea..0x40878d: the window is stored only for a corner look (0x407ef6 skips the block for a
        // straight one; the straight branch 0x40873c with v_c² = 100 is unreachable) and only while its target is
        // below the car's speed, so a straight keeps the last corner's window (stale, often 0) [O, #403 Phase 4;
        // #381 had read the dead branch as live: the trace's A+0x60 stays 0 on a straight]. A corner that enters the
        // horizon with a target above the car's speed keeps the stale window: the car counts as inside it and eases
        // to 0.5 between 0.95 and 1.0 of the corner's speed (pedal above)
        if self.radius[self.look] != 0.0 {
            let vt2 = self.target(self.look).powi(2);
            if vt2 < v * v {
                self.brake_window = window_nodes(vt2, v, self.decel); // 0x40878d: positive node count
            }
        }

        // ---- traffic (0x407f09..0x4081c9)
        // The array the original walks is sorted by raw node index, so a car "ahead" has the
        // larger node and the wrap at the finish line is not seen.
        let mut shift = 0.0f32;
        let w = self.width;
        let is_ahead = |o: &Traffic| o.node > i || (o.node == i && o.slot < self.slot);
        let (mut left_blocked, mut right_blocked) = (false, false);
        // pass 1: cars alongside, only when in the mood (mood0 < 1 − aggression). CLASSIC 0x407f3d..0x408026: the
        // walk starts after this car in the race-order array and runs 7 entries into the array's next copy, so it
        // sees every other car, the ones just ahead (|Δnode| < 5) too [O, #379]
        if self.mood[0] < 1.0 - self.temper.aggression {
            for o in others {
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
            // CLASSIC 0x4081da..0x4081eb: `lane += shift`, no clamp: a pass can aim past the road edge [O, #403];
            // only the `room` checks above limit it
            self.lane += shift;
        }

        // ---- reverse-out-of-trouble mode (0x2499)
        if !self.reverse {
            if v.abs() < CRAWL.get() && pedal > 0.0 {
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
                let boost = if self.boosted() { CORNER_BOOST.get() } else { 1.0 };
                let vc2 = (r.abs() * boost * G_AI.get() * self.a8 * cp.learn * cp.learn).max(1e-3);
                ff = (ff_k / vc2.sqrt()).atan() * (v * v) / vc2;
                if cp.min_radius < 0.0 {
                    ff = -ff;
                }
            }
            let damp = if v.abs() >= SLOW.get() || off < 5.0 { if v != 0.0 { damp_k * lat_vel / v } else { 0.0 } } else { 0.0 };
            if align.abs() < 0.9 {
                gain *= 2.0 - align.abs();
                self.terms.align_k = 2.0 - align.abs();
            }
            let lane_k = gain / (self.wl[i] + self.wr[i]).max(1.0);
            let lane_term = (self.lane - off) * lane_k;
            (self.terms.lane_k, self.terms.lane_term, self.terms.damp, self.terms.ff, self.terms.shifted) = (lane_k, lane_term, damp, ff, shift != 0.0);
            steer = lane_term - damp + ff;
            // CLASSIC 0x408b65: a frame that shifts the lane (A+0x955, set 0x4081e4, cleared 0x408a11)
            // steers twice as hard (0x4cf048 = 2.0) [O, #379]
            if shift != 0.0 {
                steer *= 2.0;
            }
            if align.abs() < 0.5 {
                steer = if dx * hz - dz * hx <= 0.0 { -1.0 } else { 1.0 };
                self.terms.mode = 1;
            }
        } else {
            self.terms.mode = 2;
            if self.side_lock == 0.0 {
                self.side_lock = if self.wl[i] + off <= self.wr[i] - off { -1.0 } else { 1.0 };
            }
            steer = self.side_lock;
        }
        // off-track recovery (0x249d)
        if self.recover || (v < SLOW.get() && (off < -self.wl[i] * 2.0 || off > self.wr[i] * 2.0)) {
            self.recover = true;
            self.terms.mode = 3;
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
            self.terms.mode += 4;
        }
        self.offset = off;
        self.raw_steer = steer;
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

    /// Is node `i` inside the braking zone of the look target `l`? CLASSIC 0x4084a0: `look - window * 0x24 <= node`
    /// on raw node pointers, not a distance around the ring. A look target that wrapped past the finish line has
    /// the smaller index, so a car on the last straight is "inside" the zone of the first corner of the next lap
    /// and lifts off there (Okkun: 270 -> 233 km/h before the line) [O, #378].
    fn in_window(&self, i: usize, l: usize, window: i32) -> bool {
        in_zone(i, l, window)
    }
}

/// Nodes needed to go from `v` to the squared speed `vt2` at the planning deceleration `decel` (< 0), truncated
/// (`__CHP` 0x4914be): `(vt2 - v²) / decel · 0.5` [O].
fn window_nodes(vt2: f32, v: f32, decel: f32) -> i32 {
    ((vt2 - v * v) / decel * 0.5) as i32
}

/// `look - window <= node` on raw node indices (CLASSIC 0x4084a0), see `GameAi::in_window`.
fn in_zone(i: usize, l: usize, window: i32) -> bool {
    l as i32 - window <= i as i32
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::car::tests::test_car;
    use motor_data::aidat::{Corner, Node, RacingLine};

    /// A straight line along +z, nodes 1 m apart, 6 m of road each side.
    fn straight_line() -> RacingLine {
        let nodes = (0..200).map(|i| Node { x: 0.0, z: i as f32, w_left: 6.0, w_right: 6.0, dir: [0.0, 0.0, 1.0], radius: 0.0, corner: 0 }).collect();
        RacingLine { nodes, corners: vec![Corner { id: 0, nodes: 200, min_radius: 0.0 }] }
    }

    fn ai(slot: usize) -> GameAi {
        GameAi::new(&straight_line(), &[1.0; 11], 1000.0, Temper::default(), slot, 2.1, 4.4)
    }

    /// #404: a human's traffic record is only tracked (node, speed, alignment, offset); `drive` would move its lane.
    #[test]
    fn human_track_keeps_the_lane_as_initialised() {
        let mut car = test_car();
        car.pos = Vec3::new(0.3, 0.0, 10.0);
        car.vel = Vec3::new(0.0, 0.0, 30.0);
        // a slower car 10 nodes ahead, a little to the side: the full AI shifts its lane to pass it
        let slow = Traffic { slot: 1, node: 20, offset: 0.5, lane: 0.0, speed: 10.0, width: 2.1, length: 4.4, align: 1.0 };
        let mut full = ai(0);
        full.reset(&car);
        full.drive(&car, 0.02, &[slow]);
        assert!(full.traffic().lane != 0.0, "the scenario must move an AI's lane");

        let mut human = ai(0);
        human.reset(&car);
        for _ in 0..10 {
            human.track(&car);
        }
        let t = human.traffic();
        assert_eq!(t.lane, 0.0);
        assert_eq!(t.node, 10);
        assert_eq!(t.speed, 30.0);
        assert!((t.align - 1.0).abs() < 1e-4);
        assert!((t.offset.abs() - 0.3).abs() < 1e-4, "offset {}", t.offset);
    }

    /// #403, CLASSIC 0x4081da: the lane is `lane += shift` with no clamp, so a car that keeps passing on the
    /// right aims past the road edge (the original's traced lane reaches −11 / +16.6 m on Olympos).
    #[test]
    fn traffic_lane_can_leave_the_road() {
        let mut car = test_car();
        car.pos = Vec3::new(1.0, 0.0, 10.0);
        car.vel = Vec3::new(0.0, 0.0, 30.0);
        // a slower car 10 nodes ahead on our low-offset side: pass it on the right, +0.2 m per tick
        let slow = Traffic { slot: 1, node: 20, offset: 0.5, lane: 0.0, speed: 10.0, width: 2.1, length: 4.4, align: 1.0 };
        let mut a = ai(0);
        a.reset(&car);
        for _ in 0..40 {
            a.drive(&car, 0.02, &[slow]);
        }
        // 6 m of road on the right; the old clamp held the lane at 6 − 1.5 = 4.5 m
        assert!(a.traffic().lane > 6.0, "lane {}", a.traffic().lane);
    }

    #[test]
    fn injected_history_and_moods_replace_our_own() {
        // diagnosis hooks (aiaudit, #403 Phase 4): a traced look / window / ramp is what the next tick starts from, and a
        // traced mood vector replaces the walk for that tick (the rng is not advanced)
        let mut car = test_car();
        car.vel = Vec3::new(0.0, 0.0, 30.0);
        let mut a = ai(0);
        a.reset(&car);
        a.inject_history(15, 7, 0.75);
        a.mood_override = Some([0.1, 0.9, 0.3]);
        let rng = a.rng;
        let inp = a.drive(&car, 0.02, &[]);
        let (_, _, ramp, mood) = a.history();
        assert_eq!(mood, [0.1, 0.9, 0.3]);
        assert_eq!(a.rng, rng, "the walk is skipped");
        assert!(a.terms.boost, "mood1 0.9 > 1 - eagerness");
        assert_eq!(a.terms.look, 15);
        assert_eq!(a.terms.window, 7);
        // the ramp rose from the injected 0.75 by 0.25/s and capped the throttle
        assert!((ramp - 0.755).abs() < 1e-4 && (inp.throttle - 0.755).abs() < 1e-4, "ramp {ramp} throttle {}", inp.throttle);
        a.mood_override = None;
        a.drive(&car, 0.02, &[]);
        assert_ne!(a.rng, rng, "the walk runs again");
    }

    #[test]
    fn window_is_the_truncated_node_count() {
        // 0x408774..0x40878d: a corner target of 10 m/s at 30 m/s with decel -20 m/s²: (100 - 900) / -20 · 0.5 = 20 nodes
        assert_eq!(window_nodes(100.0, 30.0, -20.0), 20);
        // truncated, not rounded (__CHP 0x4914be): 19.75 -> 19
        assert_eq!(window_nodes(100.0, 29.9, -20.0), 19);
    }

    #[test]
    fn straight_look_keeps_the_last_window() {
        // 0x407ef6 (#403 Phase 4): with a straight look the window is not planned; the previous value stays
        let mut car = test_car();
        car.vel = Vec3::new(0.0, 0.0, 30.0);
        let mut a = ai(0);
        a.reset(&car);
        assert_eq!(a.radius[a.look], 0.0, "the test line starts on a straight");
        a.inject_history(a.look, 7, 1.0);
        a.drive(&car, 0.02, &[]);
        assert_eq!(a.history().1, 7);
    }

    #[test]
    fn braking_zone_is_a_raw_index_compare() {
        // a corner 100 nodes ahead with a 60-node zone: outside, then inside
        assert!(!in_zone(1000, 1100, 60));
        assert!(in_zone(1050, 1100, 60));
        // the look target wrapped past the finish line (small index): the car on the last straight is "inside"
        assert!(in_zone(1990, 70, 30), "wrapped look counts as inside (#378)");
        assert!(in_zone(1990, 70, 0));
    }
}
