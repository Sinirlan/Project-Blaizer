//! #398 phase 4: the camera values are the `tunable!`s. A value set through the registry (the Constants page, tune.cfg)
//! is what `CamTune::live()` and the race camera read, the F11 Camera page's `store` goes the other way, and
//! putting a value back gives the source's camera again. One test in its own process (the values are `static`s of the
//! program, so a lib test that changed one would move the others running beside it). The race part needs the
//! install (skipped without it).

use glam::Vec3;
use motor_sim::Difficulty;
use motorview::drive::{lineup, CamMode, CamTune, Session};
use std::path::Path;

const DRIVE_RS: &str = "crates/motorview/src/drive.rs";

#[test]
fn the_camera_follows_the_tunables_and_the_camera_page_writes_them() {
    let eye_y = motor_tune::find(DRIVE_RS, "CHASE_EYE_Y").expect("CHASE_EYE_Y is registered");
    let eye_z = motor_tune::find(DRIVE_RS, "CHASE_EYE_Z").expect("CHASE_EYE_Z is registered");
    let k_xz = motor_tune::find(DRIVE_RS, "CHASE_K_XZ").expect("CHASE_K_XZ is registered");

    // nothing has touched them: the live camera is the source's
    let source = CamTune::default();
    assert_eq!(CamTune::live(), source);
    assert!(!eye_y.modified());

    // the registry side (Constants page, tune.cfg) reaches the camera values
    eye_y.set_value(eye_y.default_f64() + 1.0);
    assert_eq!(CamTune::live().chase_eye, source.chase_eye + Vec3::Y);
    assert_eq!(CamTune::default(), source, "Default stays the source's values (the page's reset)");
    eye_y.reset();

    // the Camera page side: a copy edited and stored shows in the registry as modified values
    let mut page = CamTune::live();
    page.chase_eye.z = -6.0;
    page.k_xz = 0.3;
    page.store();
    assert_eq!(eye_z.value(), -6.0);
    assert_eq!(k_xz.value() as f32, 0.3);
    assert!(eye_z.modified() && k_xz.modified() && !eye_y.modified());
    assert_eq!(CamTune::live(), page);

    // the race camera reads them (the view before any easing: the eye at `chase_eye` from the car)
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../installed_game");
    if root.exists() {
        let cars = lineup(1);
        let sess = Session::new(&root, 1, &cars, Some(cars.len() - 1), Difficulty::Medium).unwrap();
        assert_eq!(sess.cam, CamMode::Chase);
        let tuned = sess.chase_view();
        CamTune::default().store();
        let source_view = sess.chase_view();
        assert_ne!(tuned, source_view, "the live value reached the race camera");
        // 1.5 m farther back (view space flips z: the eye's z is the car's z + 4.5 or + 6)
        assert!(((tuned.0 - source_view.0).length() - 1.5).abs() < 1e-3, "{tuned:?} vs {source_view:?}");
    }

    // putting the values back is the source's camera again
    CamTune::default().store();
    assert_eq!(CamTune::live(), source);
    assert!(!eye_z.modified() && !k_xz.modified());
}
