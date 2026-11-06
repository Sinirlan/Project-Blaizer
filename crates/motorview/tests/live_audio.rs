//! #398 phase 6: the race-sound values are `tunable!`s. A value set through the registry (the Constants page, tune.cfg) is what
//! the engine pitch and the Doppler factor read, and a swapped Doppler range does not panic. One test in its own process (the
//! values are `static`s of the program, so a lib test that changed one would move the others running beside it).

use motorview::race_audio::{doppler_factor, engine_pitch};

const FILE: &str = "crates/motorview/src/race_audio.rs";

fn t(name: &str) -> &'static dyn motor_tune::Erased {
    motor_tune::find(FILE, name).unwrap_or_else(|| panic!("{name} is registered"))
}

#[test]
fn an_audio_value_set_through_the_registry_reaches_the_sound() {
    let base = engine_pitch(6000.0);
    assert!((base - (6000.0 * 0.00017 + 0.31746)).abs() < 1e-6, "the source's engine pitch");

    let slope = t("PITCH_PER_RPM");
    slope.set_value(0.0002);
    assert!(slope.modified());
    assert!(engine_pitch(6000.0) > base);
    let saved = motor_tune::save_string();
    assert!(saved.lines().any(|l| l == "motorview::race_audio::PITCH_PER_RPM 0.0002"), "{saved}");
    slope.reset();
    assert_eq!(engine_pitch(6000.0), base);
    assert!(!slope.modified());

    // Doppler: the range and the speed of sound
    assert_eq!(doppler_factor(1.0e4), 0.5);
    t("DOPPLER_MIN").set_value(0.8);
    assert_eq!(doppler_factor(1.0e4), 0.8);
    // a range typed the wrong way round (min above max) does not panic
    t("DOPPLER_MAX").set_value(0.6);
    let f = doppler_factor(10.0);
    assert!((0.6..=0.8).contains(&f), "{f}");
    t("DOPPLER_MIN").reset();
    t("DOPPLER_MAX").reset();
    t("SOUND_SPEED").set_value(686.0);
    assert!(doppler_factor(-100.0) > 1.0 && doppler_factor(0.0) == 1.0);
    t("SOUND_SPEED").reset();
    assert_eq!(doppler_factor(1.0e4), 0.5);
}
