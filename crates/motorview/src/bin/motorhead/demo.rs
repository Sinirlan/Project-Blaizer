//! Ghost mode (GameType 3, #45): the recorded cars of an `.MDE` race beside the live player, and
//! the Record race recorder that writes such files (re-notes 05-formats "MDE", 08 §8.9).
//!
//! DESIGN CHOICE: the original makes the recorded cars "remote" racers (no physics, no collision,
//! drawn like any car). Here they are pose-driven cars of the scene that are not racers: they
//! get no standings entry, nameplate or lap count, but are drawn, shadowed and lit as the others.

use glam::{Mat4, Vec3};
use motor_data::mde::{self, CarInfo, Mde};
use motor_sim::car::CarParams;
use motor_sim::demo;
use motor_sim::replay::GhostPose;
use motorview::drive::Session;
use motorview::scene::Scene;
use std::path::Path;

/// One recorded car.
pub struct DemoCar {
    /// index of its block in the records
    pub block: usize,
    /// header slot (bit of the record's mask)
    pub slot: usize,
    /// car slot of its meshes in the scene (past the racers)
    pub mesh: usize,
    pub info: CarInfo,
    pub params: CarParams,
    pub radius: f32,
    pub shadow: Option<[Vec3; 16]>,
    /// distance rolled up to each record (`demo::rolls`)
    roll: Vec<f32>,
}

pub struct DemoCars {
    pub mde: Mde,
    pub cars: Vec<DemoCar>,
}

impl DemoCars {
    /// The recorded cars of `mde`; their meshes follow the `first_mesh` racers' in the scene.
    /// `car_of` maps a recorded car's name to its game car id. `skip`: a header slot that is not
    /// drawn here (demo play poses the recorder's car as a racer). Returns them with the car ids to
    /// load after the racers' own.
    pub fn new(root: &Path, mde: Mde, first_mesh: usize, skip: Option<usize>, car_of: impl Fn(&str) -> u32) -> Result<(DemoCars, Vec<u32>), String> {
        let rolls = demo::rolls(&mde);
        let mut cars = Vec::new();
        let mut ids = Vec::new();
        for (block, slot) in mde.header.active_slots().into_iter().enumerate().filter(|&(_, s)| Some(s) != skip) {
            let info = mde.header.cars[slot].clone();
            let id = car_of(&info.car);
            let cfg = motor_data::fs::config(root, &format!("Game/car{id:02}.car"))?;
            let params = CarParams::from_config(&cfg)?;
            ids.push(id);
            cars.push(DemoCar { block, slot, mesh: first_mesh + cars.len(), info, params, radius: 0.35, shadow: motorview::drive::shadow_points(root, id), roll: rolls[block].clone() });
        }
        Ok((DemoCars { mde, cars }, ids))
    }

    /// The wheel radius of every car, from the loaded scene.
    pub fn set_radii(&mut self, sc: &Scene) {
        for c in &mut self.cars {
            if let Some(h) = sc.wheel_hubs.get(c.mesh) {
                c.radius = h.1;
            }
        }
    }

    /// The two records around race time `ms` and the blend factor. Before the first record the first
    /// one holds, after the last one the last.
    fn bracket(&self, ms: f64) -> Option<(usize, usize, f32)> {
        let recs = &self.mde.records;
        if recs.is_empty() {
            return None;
        }
        let i = recs.partition_point(|r| r.ms as f64 <= ms);
        Some(if i == 0 {
            (0, 0, 0.0)
        } else if i >= recs.len() {
            (recs.len() - 1, recs.len() - 1, 0.0)
        } else {
            (i - 1, i, ((ms - recs[i - 1].ms as f64) / (recs[i].ms - recs[i - 1].ms).max(1) as f64).clamp(0.0, 1.0) as f32)
        })
    }

    /// Car block `block` at race time `ms`, blended as the original does (`CarState::lerp`).
    pub fn state(&self, block: usize, ms: f64) -> Option<mde::CarState> {
        let (a, b, f) = self.bracket(ms)?;
        let r = &self.mde.records;
        Some(r[a].cars.get(block)?.lerp(r[b].cars.get(block)?, f))
    }

    /// The cars present at race time `ms` (since the green light's 6 s mark, see `demo::GO_MS`) with
    /// their poses.
    pub fn poses(&self, ms: f64) -> Vec<(&DemoCar, GhostPose)> {
        let Some((a, b, f)) = self.bracket(ms) else { return Vec::new() };
        let recs = &self.mde.records;
        self.cars
            .iter()
            .filter(|c| recs[a].mask & (1 << c.slot) != 0)
            .map(|c| {
                let s = recs[a].cars[c.block].lerp(&recs[b].cars[c.block], f);
                let roll = c.roll[a] + (c.roll[b] - c.roll[a]) * f;
                (c, demo::pose(&s, ms, roll, c.params.ride_height()))
            })
            .collect()
    }

    /// Body and wheels of every present car at `ms`.
    pub fn models(&self, ms: f64, scale: f32) -> Vec<(usize, Mat4)> {
        self.poses(ms).into_iter().flat_map(|(c, p)| Session::pose_models(&c.params, c.mesh, c.radius, &p, scale)).collect()
    }

    /// Projected shadows of the present cars (Classic), appended to `out`.
    pub fn shadows(&self, sess: &Session, ms: f64, out: &mut Vec<motorview::scene::Vertex>) {
        for (c, p) in self.poses(ms) {
            if let Some(pts) = &c.shadow {
                sess.shadow_quads(p.pos, p.rot, pts, sess.race.car_scale.0, out);
            }
        }
    }
}
