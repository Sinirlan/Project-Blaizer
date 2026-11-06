//! A simple test driver: follows the ai.dat racing line and brakes for corners using the
//! cornering limit implied by the grip model. Not the game's AI (that is M4).

use crate::car::{Car, Input};
use glam::Vec3;
use motor_data::aidat::RacingLine;

motor_tune::tunable! {
    /// Steady-state yaw-rate limit implied by the grip model, 03-physics.md §5 [D].
    /// range: 0.1..2
    /// unit: rad/s
    YAW_LIMIT: f32 = 0.55;
    /// Braking deceleration the test driver plans with [D].
    /// range: 2..40
    /// unit: m/s²
    BRAKE_DECEL: f32 = 14.0;
}

pub struct Autopilot {
    nodes: Vec<Vec3>,
    curvature: Vec<f32>,
    idx: usize,
    /// multiplies the corner speed limit (driver skill / variety)
    pub pace: f32,
    /// lateral offset from the line in metres (keeps grid cars apart)
    pub offset: f32,
}

impl Autopilot {
    pub fn new(line: &RacingLine) -> Autopilot {
        let nodes: Vec<Vec3> = line.nodes.iter().map(|n| Vec3::new(n.x, 0.0, n.z)).collect();
        let n = nodes.len();
        let heading: Vec<f32> = line.nodes.iter().map(|n| n.dir[0].atan2(n.dir[2])).collect();
        let curvature = (0..n)
            .map(|i| {
                let d = heading[(i + 12) % n] - heading[i];
                let d = (d + std::f32::consts::PI).rem_euclid(std::f32::consts::TAU) - std::f32::consts::PI;
                let len: f32 = (0..12).map(|k| nodes[(i + k) % n].distance(nodes[(i + k + 1) % n])).sum();
                d.abs() / len.max(1.0)
            })
            .collect();
        Autopilot { nodes, curvature, idx: 0, pace: 1.0, offset: 0.0 }
    }

    pub fn node(&self) -> usize {
        self.idx
    }

    pub fn drive(&mut self, car: &Car) -> Input {
        let n = self.nodes.len();
        let flat = Vec3::new(car.pos.x, 0.0, car.pos.z);
        let nodes = &self.nodes;
        self.idx = (self.idx + n - 20..self.idx + n + 80)
            .min_by(|&a, &b| nodes[a % n].distance_squared(flat).total_cmp(&nodes[b % n].distance_squared(flat)))
            .unwrap()
            % n;
        let v = car.vel.z.max(0.0);
        let (brake_decel, yaw_limit) = (BRAKE_DECEL.get(), YAW_LIMIT.get());
        let horizon = (v * v / (2.0 * brake_decel) + 40.0) as usize;
        let (mut v_allowed, mut dist) = (f32::MAX, 0.0);
        for k in 0..horizon.min(n) {
            let j = (self.idx + k) % n;
            let v_corner = self.pace * yaw_limit / self.curvature[j].max(1e-4);
            v_allowed = v_allowed.min((v_corner * v_corner + 2.0 * brake_decel * dist).sqrt());
            dist += nodes[j].distance(nodes[(j + 1) % n]);
        }
        let (throttle, brake) = if v > v_allowed + 2.0 {
            (0.0, 1.0)
        } else if v > v_allowed {
            (0.0, 0.0)
        } else {
            (1.0, 0.0)
        };
        let ahead = 10 + (v * 0.4) as usize;
        let a = self.nodes[(self.idx + ahead) % n];
        let b = self.nodes[(self.idx + ahead + 1) % n];
        let side = Vec3::new(b.z - a.z, 0.0, a.x - b.x).normalize_or(Vec3::ZERO);
        let target = a - side * self.offset;
        let local = car.rot.transpose() * (target - flat);
        Input { throttle, brake, steer: (local.x.atan2(local.z) * 2.5).clamp(-1.0, 1.0), ..Default::default() }
    }
}
