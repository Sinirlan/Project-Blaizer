//! motorrace — headless race test: 8 bot cars on the grid, full race with checkpoints and laps.
//!
//!   motorrace [installed_game] [--track N] [--laps L] [--cars 1,2,3,...] [--difficulty easy|medium|hard|beginner]
//!             [--rules normal|elimination] [--cheat gulliver]
//!             [--name DRIVER --team TEAM]   the original's profile-name cheat (08 §8.14); only reported until its effect is built
//!             [--trace-slot S --trace-from T --trace-to T --trace-every T]
//!             [--gaps-every T]   live gap table (#299) every T s, and the check against the checkpoint splits
//!             [--sheet]          the race sheet (#94): per car lap, sector and position-by-lap data, and its consistency check
//!             [--dt S]           the frame length (default 1/60 s)
//!             [--tick HZ]        step the physics on the fixed tick of HZ per second (#374) instead of once per frame

use motor_sim::race::fmt_ms;
use motor_sim::session::{GULLIVER_FEEL, GULLIVER_SCALE};
use motor_sim::sheet::RaceSheet;
use motor_sim::{Difficulty, Input, Race, Rules};

fn arg<T: std::str::FromStr>(args: &[String], name: &str, default: T) -> T {
    args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).and_then(|v| v.parse().ok()).unwrap_or(default)
}

fn main() -> Result<(), String> {
    let args: Vec<String> = std::env::args().collect();
    let root = motor_data::fs::root_from_first_arg(args.get(1).map(String::as_str))?;
    let track: u32 = arg(&args, "--track", 1);
    let laps: u32 = arg(&args, "--laps", 0);
    let dt: f32 = arg(&args, "--dt", 1.0 / 60.0);
    let cars: Vec<u32> = arg(&args, "--cars", "1,2,3,4,5,6,7,8".to_string()).split(',').filter_map(|c| c.parse().ok()).collect();
    let difficulty = match arg(&args, "--difficulty", "medium".to_string()).as_str() {
        "easy" => Difficulty::Easy,
        "beginner" => Difficulty::Beginner,
        "hard" => Difficulty::Hard,
        _ => Difficulty::Medium,
    };
    let mut race = Race::load_with(&root, track, &cars, None, (laps > 0).then_some(laps), difficulty)?;
    let tick: f32 = arg(&args, "--tick", 0.0);
    race.set_fixed_tick((tick > 0.0).then_some(tick));
    if arg(&args, "--rules", "normal".to_string()) == "elimination" {
        race.set_rules(Rules::Elimination);
    }
    // CHEAT gulliver: all cars small from the grid on (no human here: the AI's full-size feel)
    if arg(&args, "--cheat", String::new()).split(',').any(|c| c.eq_ignore_ascii_case("gulliver")) {
        race.set_car_scale(GULLIVER_SCALE.get(), GULLIVER_FEEL.get());
    }
    // the original's profile-name cheat (`cheats::NameCheat`)
    race.set_name_cheat(motor_sim::cheats::NameCheat::of(&arg(&args, "--name", String::new()), &arg(&args, "--team", String::new())));
    if let Some(c) = race.name_cheat {
        println!("name cheat: {c:?}, gravity {}, spring {:.3} m rate {:.0} damper {}", race.world.gravity, race.racers[0].car.p.spring_len, race.racers[0].car.p.spring_k, race.racers[0].car.p.damper);
    }
    println!("track {track}: {} laps, centreline {:.0} m, {} keys, checkpoints {:?}",
        race.laps, race.line.length, race.line.groups, race.line.checkpoints.iter().map(|c| (c.1, c.0 as u32)).collect::<Vec<_>>());
    let n = race.racers.len();
    let mut seen_lap = vec![1u32; n];
    let mut seen_cp = vec![None; n];
    let mut wrong = vec![false; n];
    let mut seen_out = vec![false; n];
    let mut stuck = vec![0.0f32; n];
    let trace_slot: i64 = arg(&args, "--trace-slot", -1);
    let (trace_from, trace_to): (f32, f32) = (arg(&args, "--trace-from", 0.0), arg(&args, "--trace-to", 15.0));
    let trace_every: f32 = arg(&args, "--trace-every", 0.1);
    let mut next_trace = 0.0f32;
    // live gap (#299): a table every `gaps_every` s; each checkpoint crossing checked against the splits
    let gaps_every: f32 = arg(&args, "--gaps-every", 0.0);
    let mut next_gaps = gaps_every;
    let (mut gap_err, mut gap_n, mut gap_skipped) = (0i64, 0u32, 0u32);
    let limit_s = 60.0 * 20.0;
    let mut t = 0.0f32;
    while !race.all_finished() && t < limit_s {
        race.step(dt, Input::default());
        t += dt;
        if trace_slot >= 0 && t >= trace_from && t >= next_trace && t < trace_to {
            next_trace = t + trace_every;
            let r = &race.racers[trace_slot as usize];
            println!("  trace t={t:6.2} kmh={:6.1} gear {} wheels {} pos=({:.0},{:.0},{:.0}) {}", r.telemetry.kmh, r.telemetry.gear, r.telemetry.contacts, r.car.pos.x, r.car.pos.y, r.car.pos.z, r.ai.debug());
        }
        if gaps_every > 0.0 && t >= next_gaps {
            next_gaps += gaps_every;
            println!("  gaps t={t:6.1}");
            let order = race.standings();
            for (pos, &i) in order.iter().enumerate() {
                let show = |g: Option<motor_sim::race::Gap>| match g {
                    Some(motor_sim::race::Gap::Time(ms)) => format!("+{}.{:03}", ms / 1000, ms % 1000),
                    Some(motor_sim::race::Gap::Laps(l)) => format!("+{l} LAP"),
                    None => "-".into(),
                };
                let lead = if pos > 0 { show(race.live_gap(order[0], i)) } else { "LEADER".into() };
                let next = if pos > 0 { show(race.live_gap(order[pos - 1], i)) } else { "-".into() };
                let m = if pos > 0 { race.distance_gap(order[pos - 1], i).map_or("-".into(), |d| format!("{d:.0} m")) } else { "-".into() };
                println!("    P{} slot {i}: to leader {lead:>9}  to car ahead {next:>9}  {m}", pos + 1);
            }
        }
        for i in 0..n {
            let p = &race.racers[i].progress;
            let newly_finished = p.finished_ms.is_some() && seen_lap[i] != u32::MAX;
            if seen_lap[i] != u32::MAX && (p.lap != seen_lap[i] || newly_finished) {
                let tag = if p.finished_ms.is_some() { "FINISH".to_string() } else { format!("lap {}", p.lap) };
                println!("{:>9}  car{:02} slot {i}: {tag}  last {}", fmt_ms(race.clock_ms as u32), race.racers[i].car_id,
                    p.last_lap_ms.map_or("-".into(), fmt_ms));
                seen_lap[i] = if p.finished_ms.is_some() { u32::MAX } else { p.lap };
            }
            if let Some(at) = p.eliminated_ms.filter(|_| !seen_out[i]) {
                seen_out[i] = true;
                println!("{:>9}  car{:02} slot {i}: OUT (lap {})", fmt_ms(at), race.racers[i].car_id, p.lap);
            }
            if p.last_checkpoint != seen_cp[i] {
                seen_cp[i] = p.last_checkpoint;
                // the gap to every car that was through this checkpoint first must equal the split difference
                if let Some((cp, at)) = p.last_checkpoint {
                    let idx = p.lap as usize * 4 + cp as usize + 1;
                    for j in 0..n {
                        let Some(tj) = race.racers[j].progress.splits.get(idx).copied().flatten().filter(|&tj| j != i && tj <= at) else { continue };
                        match race.live_gap(j, i) {
                            Some(motor_sim::race::Gap::Time(ms)) => {
                                gap_err = gap_err.max((ms as i64 - (at - tj) as i64).abs());
                                gap_n += 1;
                            }
                            _ => gap_skipped += 1,
                        }
                    }
                }
                if i == 0 {
                    let (cp, at) = p.last_checkpoint.unwrap();
                    println!("{:>9}  car{:02} slot 0: checkpoint {cp} (lap {})", fmt_ms(at), race.racers[0].car_id, p.lap);
                }
            }
            if p.wrong_way != wrong[i] {
                wrong[i] = p.wrong_way;
                println!("{:>9}  slot {i}: wrong way = {}", fmt_ms(race.clock_ms as u32), p.wrong_way);
            }
            // test-bench rescue for wedged bots
            stuck[i] = if race.started() && race.racers[i].telemetry.kmh.abs() < 5.0 && !p.done() { stuck[i] + dt } else { 0.0 };
            if stuck[i] > 4.0 {
                println!("{:>9}  slot {i}: respawn", fmt_ms(race.clock_ms as u32));
                race.respawn(i);
                stuck[i] = 0.0;
            }
        }
    }
    if gaps_every > 0.0 {
        println!("\nlive gap vs checkpoint splits: max error {gap_err} ms over {gap_n} crossings ({gap_skipped} without a time gap)");
    }
    println!("\nRESULT (track {track}, {} laps):", race.laps);
    for (pos, &i) in race.standings().iter().enumerate() {
        let r = &race.racers[i];
        println!("  P{} car{:02} slot {i}: {}  best lap {}", pos + 1, r.car_id,
            r.progress.finished_ms.map_or(
                if r.progress.eliminated_ms.is_some() { format!("OUT (lap {})", r.progress.lap) } else { format!("DNF (lap {})", r.progress.lap) },
                fmt_ms),
            r.progress.best_lap_ms.map_or("-".into(), fmt_ms));
    }
    if args.iter().any(|a| a == "--sheet") {
        print_sheet(&race);
    }
    Ok(())
}

/// REMASTER (#94): one line per car in finishing order, then the sums and orders checked against the race.
fn print_sheet(race: &Race) {
    let sheet = RaceSheet::build(race);
    let secs = |ms: u32| format!("{}.{:02}", ms / 1000, (ms % 1000) / 10);
    println!("\nSHEET ({} laps): grid>place, led, best (* fastest), then per lap: lap [S1 S2 S3] position", sheet.laps);
    let mut problems = 0;
    for (place, &slot) in sheet.order.iter().enumerate() {
        let c = &sheet.cars[slot];
        let laps: Vec<String> = (0..c.lap_ms.len())
            .map(|k| {
                let sec = c.sector_ms[k].map_or("-".into(), |s| format!("{} {} {}", secs(s[0]), secs(s[1]), secs(s[2])));
                format!("{} [{sec}] P{}", fmt_ms(c.lap_ms[k]), c.pos_at[k + 1])
            })
            .collect();
        let end = c.race_ms.map_or(c.out_lap.map_or("DNF".to_string(), |l| format!("OUT lap {l}")), fmt_ms);
        println!("  P{} slot {slot}: grid {}>{} led {} best {}{}  {end}  | {}", place + 1, c.grid, place + 1, c.led,
            c.best_lap.map_or("-".into(), fmt_ms), if c.fastest { "*" } else { "" }, laps.join("  "));
        // checks: laps add up to the race time, sectors to their lap, the best to the race's, the last place to the finishing order
        let mut bad = Vec::new();
        if c.race_ms.is_some_and(|t| c.lap_ms.iter().sum::<u32>() != t) {
            bad.push("laps do not sum to the race time");
        }
        if c.lap_ms.iter().zip(&c.sector_ms).any(|(l, s)| s.is_some_and(|s| s.iter().sum::<u32>() != *l)) {
            bad.push("sectors do not sum to the lap");
        }
        if c.sector_ms.iter().any(Option::is_none) {
            bad.push("a lap without sector times");
        }
        if c.best_lap != race.racers[slot].progress.best_lap_ms {
            bad.push("best lap differs from Progress");
        }
        if c.race_ms.is_some() && *c.pos_at.last().unwrap() as usize != place + 1 {
            bad.push("last position is not the finishing place");
        }
        for b in bad {
            println!("    PROBLEM: {b}");
            problems += 1;
        }
    }
    let fastest = sheet.cars.iter().filter(|c| c.fastest).count();
    let led: u32 = sheet.cars.iter().map(|c| c.led as u32).sum();
    println!("sheet check: {} ({problems} problems, {fastest} fastest lap(s), {led} laps led of {})", if problems == 0 && fastest == 1 { "OK" } else { "FAILED" }, sheet.laps);
}
