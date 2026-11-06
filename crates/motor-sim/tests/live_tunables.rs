//! #398 phase 3: a physics `tunable!` set at run time changes the next race step, and putting it back gives the
//! source's run again. One test in its own process (the values are `static`s of the program, so a lib test that
//! changed one would move the others running beside it). Needs the install (skipped without it).

use glam::Vec3;
use motor_sim::{Input, Race};
use std::path::Path;

const CAR_RS: &str = "crates/motor-sim/src/car.rs";

/// The poses of a one-car race 3 s after GO with the throttle held (the 12 s countdown holds the car on the grid).
fn run(root: &Path) -> Vec<Vec3> {
    let mut race = Race::load(root, 1, &[1], Some(0), Some(3)).unwrap();
    for _ in 0..900 {
        race.step(1.0 / 60.0, Input { throttle: 1.0, ..Input::default() });
    }
    race.racers.iter().map(|r| r.car.pos).collect()
}

#[test]
fn a_live_value_changes_the_physics_and_putting_it_back_restores_it() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../installed_game");
    if !root.exists() {
        return;
    }
    let downforce = motor_tune::find(CAR_RS, "DOWNFORCE").expect("DOWNFORCE is registered");
    let default = downforce.default_f64();
    assert_eq!(downforce.value(), default, "nothing has touched it");
    let source = run(&root);
    // ten times the downforce presses the springs down: the car sits lower
    downforce.set_value(default * 10.0);
    assert!(downforce.modified());
    let tuned = run(&root);
    assert_ne!(source, tuned, "the live value reached the car step");
    assert!(tuned[0].y < source[0].y - 1e-3, "more downforce: lower ride ({} vs {})", tuned[0].y, source[0].y);
    downforce.reset();
    assert!(!downforce.modified());
    assert_eq!(source, run(&root), "back at the source's value the run is the same");
}
