//! motorsim — headless physics test bench.
//!
//!   motorsim [installed_game] [--track N] [--car NN] [--laps L] [--seconds S] [--dt 0.016]
//!            [--trace-from T --trace-to T]
//!
//! Places the car at the start of the AI racing line and drives it with a simple autopilot
//! (line following + corner speed from curvature + braking distance), printing telemetry
//! twice per second and lap times. The autopilot is a test driver, not the game's AI.

use glam::Vec3;
use motor_data::{aidat, fs};
use motor_sim::{car::CarParams, Car, Input, World};

fn arg<T: std::str::FromStr>(args: &[String], name: &str, default: T) -> T {
    args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).and_then(|v| v.parse().ok()).unwrap_or(default)
}

/// Steady-state yaw-rate limit implied by the grip model (rad/s): see 03-physics.md §3.5.
const YAW_LIMIT: f32 = 0.55;
/// Conservative braking deceleration used for planning (m/s²).
const BRAKE_DECEL: f32 = 14.0;

fn main() -> Result<(), String> {
    let args: Vec<String> = std::env::args().collect();
    let root = fs::root_from_first_arg(args.get(1).map(String::as_str))?;
    let (track, car_n, laps): (u32, u32, u32) = (arg(&args, "--track", 1), arg(&args, "--car", 1), arg(&args, "--laps", 1));
    let (seconds, dt): (f32, f32) = (arg(&args, "--seconds", 600.0), arg(&args, "--dt", 1.0 / 60.0));
    let trace: (f32, f32) = (arg(&args, "--trace-from", -1.0), arg(&args, "--trace-to", -1.0));

    let world = World::load(&root, track)?;
    let params = CarParams::from_config(&fs::config(&root, &format!("Game/car{car_n:02}.car"))?)?;
    let trk = fs::config(&root, &format!("Game/Track{track}.trk"))?;
    let ai_dir = trk.get("AITrackName").and_then(|e| e.values.first().cloned()).ok_or("no AITrackName")?;
    let line = aidat::parse(&fs::read(&root, &format!("{ai_dir}/AI/ai.dat"))?).map_err(|e| e.to_string())?;
    let n0 = &line.nodes[0];
    let gy = world.ground(n0.x, n0.z, 100.0).ok_or("no ground under the start node")?;
    println!("car '{}' m={} gear tops {:?} acc={} grip={:?}", params.name, params.mass, params.gear_top, params.acc_force, params.grip);
    let mut car = Car::new(params, Vec3::new(n0.x, gy - 0.2, n0.z), Vec3::from(n0.dir));

    let nodes: Vec<Vec3> = line.nodes.iter().map(|n| Vec3::new(n.x, 0.0, n.z)).collect();
    let nn = nodes.len();
    let heading: Vec<f32> = line.nodes.iter().map(|n| n.dir[0].atan2(n.dir[2])).collect();
    // curvature (rad/m) over a 12-node window
    let curv: Vec<f32> = (0..nn)
        .map(|i| {
            let mut d = heading[(i + 12) % nn] - heading[i];
            d = (d + std::f32::consts::PI).rem_euclid(std::f32::consts::TAU) - std::f32::consts::PI;
            let len: f32 = (0..12).map(|k| nodes[(i + k) % nn].distance(nodes[(i + k + 1) % nn])).sum();
            d.abs() / len.max(1.0)
        })
        .collect();

    let (mut idx, mut lap, mut lap_start, mut stuck) = (0usize, 0u32, 0.0f32, 0.0f32);
    let (mut t, mut next_print, mut t100, mut vmax) = (0.0f32, 0.0f32, None, 0.0f32);
    while t < seconds && lap < laps {
        let flat = Vec3::new(car.pos.x, 0.0, car.pos.z);
        let prev = idx;
        idx = (idx..idx + 60)
            .min_by(|&a, &b| nodes[a % nn].distance_squared(flat).total_cmp(&nodes[b % nn].distance_squared(flat)))
            .unwrap()
            % nn;
        if idx < prev && prev > nn - 200 && idx < 200 {
            lap += 1;
            println!("LAP {lap}: {:.2} s", t - lap_start);
            lap_start = t;
        }
        let v = car.vel.z.max(0.0);
        // speed limit: min over the next stretch of sqrt(v_corner² + 2·a·distance)
        let horizon = (v * v / (2.0 * BRAKE_DECEL) + 40.0) as usize;
        let mut v_allowed = f32::MAX;
        let mut dist = 0.0;
        for k in 0..horizon.min(nn) {
            let j = (idx + k) % nn;
            let v_corner = YAW_LIMIT / curv[j].max(1e-4);
            v_allowed = v_allowed.min((v_corner * v_corner + 2.0 * BRAKE_DECEL * dist).sqrt());
            dist += nodes[j].distance(nodes[(j + 1) % nn]);
        }
        let (throttle, brake) = if v > v_allowed + 2.0 { (0.0, 1.0) } else if v > v_allowed { (0.0, 0.0) } else { (1.0, 0.0) };
        let ahead = 10 + (v * 0.4) as usize;
        let local = car.rot.transpose() * (nodes[(idx + ahead) % nn] - flat);
        let steer = (local.x.atan2(local.z) * 2.5).clamp(-1.0, 1.0);
        let tel = car.step_frame(dt, Input { throttle, brake, steer, ..Default::default() }, &world);
        t += dt;
        vmax = vmax.max(tel.kmh);
        if t100.is_none() && tel.kmh >= 100.0 {
            t100 = Some(t);
        }
        // test-bench respawn if wedged: back onto the line
        stuck = if tel.kmh.abs() < 5.0 { stuck + dt } else { 0.0 };
        if stuck > 3.0 {
            let n = &line.nodes[(idx + 5) % nn];
            let y = world.ground(n.x, n.z, car.pos.y + 20.0).unwrap_or(car.pos.y);
            car.place(Vec3::new(n.x, y - 0.2, n.z), Vec3::from(n.dir));
            println!("t={t:5.1}s respawn at node {}", (idx + 5) % nn);
            stuck = 0.0;
        }
        if t >= trace.0 && t <= trace.1 {
            println!("  t={t:6.3} v={:?} w={:?} hull={:?}", car.vel, car.ang, car.hull_hits);
        }
        if t >= next_print {
            println!(
                "t={t:5.1}s {:6.1} km/h (allow {:5.0}) gear {} rpm {:5.0} wheels {} {} node {:4} y={:7.2}",
                tel.kmh, v_allowed.min(999.0) * 3.6, tel.gear, tel.rpm, tel.contacts, if tel.sliding { "SLIDE" } else { "grip " }, idx, car.pos.y
            );
            next_print += 2.0;
        }
        if !car.pos.is_finite() {
            return Err("simulation diverged".into());
        }
    }
    println!("0-100 km/h: {}  max {:.1} km/h  laps {lap}  time {t:.1} s",
        t100.map_or("not reached".into(), |v| format!("{v:.2} s")), vmax);
    Ok(())
}
