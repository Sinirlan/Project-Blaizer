//! DEBUG motorview's tuning changes (#434): what the Properties dock's Car and Camera pages edit, as field tables
//! that both draw the rows (`tune.rs`) and diff them, so the status bar's "N CHANGED" chip, the Changes card and
//! the dock's modified dots come from the same comparisons. A `Snap` is the edited state of one moment (every
//! racer's parameters without the rubber band's scale, the gravity, the paints, the camera values); `changes`
//! lists where it differs from the loaded one.

use crate::drive::{CamTune, Session};
use crate::tune::Paint;
use glam::Vec3;
use motor_sim::CarParams;
use std::ops::RangeInclusive;

/// The dock section a row sits in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Sect {
    Physics,
    Grip,
    Gears,
    Offsets,
    Easing,
}

/// The value a row edits.
pub enum F<'a> {
    Num(&'a mut f32),
    V3(&'a mut Vec3),
}

/// A row's value, copied out.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Val {
    Num(f32),
    V3(Vec3),
}

impl Val {
    pub fn v3(self) -> Option<Vec3> {
        match self {
            Val::V3(v) => Some(v),
            Val::Num(_) => None,
        }
    }

    pub fn text(self) -> String {
        match self {
            Val::Num(x) => num(x),
            Val::V3(v) => format!("{}, {}, {}", num(v.x), num(v.y), num(v.z)),
        }
    }
}

impl F<'_> {
    pub fn get(&self) -> Val {
        match self {
            F::Num(v) => Val::Num(**v),
            F::V3(v) => Val::V3(**v),
        }
    }

    pub fn set(&mut self, v: Val) {
        match (self, v) {
            (F::Num(x), Val::Num(y)) => **x = y,
            (F::V3(x), Val::V3(y)) => **x = y,
            _ => {}
        }
    }
}

/// One property row of a field table: its section, label, unit, drag speed, range (the field's range bar) and value.
pub struct Row<'a> {
    pub sect: Sect,
    pub label: String,
    pub unit: &'static str,
    pub speed: f32,
    pub range: Option<RangeInclusive<f32>>,
    pub f: F<'a>,
}

fn row<'a>(sect: Sect, label: &str, unit: &'static str, speed: f32, f: F<'a>) -> Row<'a> {
    Row { sect, label: label.to_string(), unit, speed, range: None, f }
}

/// The Car page's rows (Physics, Grip, Gears), in the dock's order.
pub fn param_rows(p: &mut CarParams) -> Vec<Row<'_>> {
    let CarParams { mass, gear_top, min_rpm, max_rpm, acc_force, brake_force, turn_force, com, spring_len, spring_k, grip, .. } = p;
    let mut v = vec![
        row(Sect::Physics, "Mass", "kg", 5.0, F::Num(mass)),
        row(Sect::Physics, "Acc force", "", 10.0, F::Num(acc_force)),
        row(Sect::Physics, "Brake force", "", 10.0, F::Num(brake_force)),
        row(Sect::Physics, "Turn force", "", 50.0, F::Num(turn_force)),
        row(Sect::Physics, "Min rpm", "rpm", 10.0, F::Num(min_rpm)),
        row(Sect::Physics, "Max rpm", "rpm", 10.0, F::Num(max_rpm)),
        row(Sect::Physics, "Spring length", "m", 0.005, F::Num(spring_len)),
        row(Sect::Physics, "Spring strength", "", 10.0, F::Num(spring_k)),
        row(Sect::Physics, "Centre of mass", "", 0.01, F::V3(com)),
    ];
    for (g, name) in grip.iter_mut().zip(["Slide speed", "Slide yaw", "Lateral", "Yaw"]) {
        v.push(row(Sect::Grip, name, "", 0.005, F::Num(g)));
    }
    for (i, g) in gear_top.iter_mut().enumerate() {
        let name = if i == 0 { "Reverse top".to_string() } else { format!("Gear {i} top") };
        v.push(row(Sect::Gears, &name, "km/h", 0.5, F::Num(g)));
    }
    v
}

/// The Camera page's rows (Offsets, Easing and walls), in the dock's order.
pub fn cam_rows(t: &mut CamTune) -> Vec<Row<'_>> {
    let CamTune { chase_eye, chase_look, far_eye, far_look, incar_eye, bumper_eye, k_xz, k_y, max_lag, margin, min, out_rate } = t;
    let easing = |label: &str, unit: &'static str, speed: f32, range: RangeInclusive<f32>, f| Row { sect: Sect::Easing, label: label.to_string(), unit, speed, range: Some(range), f: F::Num(f) };
    vec![
        row(Sect::Offsets, "Chase eye", "", 0.02, F::V3(chase_eye)),
        row(Sect::Offsets, "Chase look", "", 0.02, F::V3(chase_look)),
        row(Sect::Offsets, "Far eye", "", 0.02, F::V3(far_eye)),
        row(Sect::Offsets, "Far look", "", 0.02, F::V3(far_look)),
        row(Sect::Offsets, "In-car eye", "", 0.01, F::V3(incar_eye)),
        row(Sect::Offsets, "Bumper eye", "", 0.01, F::V3(bumper_eye)),
        easing("Easing xz", "", 0.005, 0.0..=1.0, k_xz),
        easing("Easing y", "", 0.005, 0.0..=1.0, k_y),
        easing("Max lag", "rad", 0.01, 0.0..=std::f32::consts::PI, max_lag),
        easing("Wall margin", "m", 0.01, 0.0..=5.0, margin),
        easing("Min distance", "m", 0.01, 0.0..=10.0, min),
        easing("Ease-out rate", "", 0.05, 0.0..=50.0, out_rate),
    ]
}

/// The values of `param_rows`, copied out.
pub fn param_vals(p: &CarParams) -> Vec<Val> {
    param_rows(&mut p.clone()).iter().map(|r| r.f.get()).collect()
}

/// The values of `cam_rows`, copied out.
pub fn cam_vals(t: &CamTune) -> Vec<Val> {
    cam_rows(&mut t.clone()).iter().map(|r| r.f.get()).collect()
}

/// The edited state of one moment (or, for `Tune::orig_snap`, the loaded one).
#[derive(Clone)]
pub struct Snap {
    /// per racer, without the rubber band's scale (`Racer::tuned`)
    pub params: Vec<CarParams>,
    pub gravity: f32,
    /// per racer: CarColour1..3 as drawn
    pub paints: Vec<Paint>,
    pub cam: CamTune,
}

impl Snap {
    /// The state now: the session's racers and gravity, the paints as drawn, the live camera values.
    pub fn now(sess: Option<&Session>, paints: Vec<Paint>) -> Snap {
        Snap {
            params: sess.map(|s| s.race.racers.iter().map(|r| r.tuned()).collect()).unwrap_or_default(),
            gravity: sess.map_or(0.0, |s| s.race.world.gravity),
            paints,
            cam: CamTune::live(),
        }
    }

    /// `--tune-set KEY=VALUE` (shots): a Car page row of racer `slot` by its slug (`mass`, `acc-force`, `gear-2-top`,
    /// `centre-of-mass-y`), a Camera row (`chase-eye-y`, `max-lag`) or `gravity`. False: no such row.
    pub fn set_slug(&mut self, slot: usize, key: &str, v: f32) -> bool {
        if key == "gravity" {
            self.gravity = v;
            return true;
        }
        let hit = |rows: &mut Vec<Row>| {
            for r in rows.iter_mut() {
                let s = slug(&r.label);
                match &mut r.f {
                    F::Num(x) if s == key => {
                        **x = v;
                        return true;
                    }
                    F::V3(x) => {
                        for (k, axis) in ["x", "y", "z"].iter().enumerate() {
                            if key == format!("{s}-{axis}") {
                                x[k] = v;
                                return true;
                            }
                        }
                    }
                    _ => {}
                }
            }
            false
        };
        if let Some(p) = self.params.get_mut(slot) {
            if hit(&mut param_rows(p)) {
                return true;
            }
        }
        hit(&mut cam_rows(&mut self.cam))
    }

    /// Put this state in place where it differs from `cur` (the state now): the racers' parameters
    /// (`Racer::set_tuned`), the gravity, the camera values. Returns the paints that differ, (slot, CarColour1..3),
    /// for the caller to reload the scene with.
    pub fn apply(&self, cur: &Snap, sess: Option<&mut Session>) -> Vec<(usize, Paint)> {
        if let Some(s) = sess {
            for (r, (p, c)) in s.race.racers.iter_mut().zip(self.params.iter().zip(&cur.params)) {
                if param_vals(p) != param_vals(c) {
                    r.set_tuned(p);
                }
            }
            if self.gravity != cur.gravity {
                s.race.world.gravity = self.gravity;
            }
        }
        if self.cam != cur.cam {
            self.cam.store();
        }
        self.paints.iter().zip(&cur.paints).enumerate().filter(|(_, (a, b))| a != b).map(|(i, (a, _))| (i, *a)).collect()
    }
}

impl Snap {
    /// The same values (the dock's rows, gravity, paints, camera), what the undo steps compare.
    pub fn same(&self, other: &Snap) -> bool {
        self.gravity == other.gravity
            && self.paints == other.paints
            && self.params.len() == other.params.len()
            && self.params.iter().zip(&other.params).all(|(a, b)| param_vals(a) == param_vals(b))
            && cam_vals(&self.cam) == cam_vals(&other.cam)
    }
}

/// Undo steps kept.
const UNDO_LIMIT: usize = 100;

/// Ctrl+Z / Ctrl+Y for the dock's edits (#434), as the Cine editor's: whole snapshots. `settle` is called once a frame
/// with the state now; an edit (a drag, a typed field, a reset) that ends with no pointer button down is one step.
/// Switching car or track starts a new history (`clear`).
#[derive(Default)]
pub struct Undo {
    undo: Vec<Snap>,
    redo: Vec<Snap>,
    /// the state as of the last step (`settle` compares with it); None until the first `settle` after `clear`
    base: Option<Snap>,
}

impl Undo {
    pub fn clear(&mut self) {
        *self = Undo::default();
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    /// Close the open step: when `now` differs from the last step's state, push that one. `held` = a pointer button is
    /// down (a drag in progress waits for its release). A new edit drops the redo stack.
    pub fn settle(&mut self, now: &Snap, held: bool) {
        let Some(base) = self.base.as_mut() else {
            self.base = Some(now.clone());
            return;
        };
        if held || base.same(now) {
            return;
        }
        self.undo.push(std::mem::replace(base, now.clone()));
        if self.undo.len() > UNDO_LIMIT {
            self.undo.remove(0);
        }
        self.redo.clear();
    }

    /// The state to put in place for Ctrl+Z (back) / Ctrl+Y (forward), `now` pushed to the other stack. None: nothing there.
    pub fn step(&mut self, now: &Snap, back: bool) -> Option<Snap> {
        self.settle(now, false);
        let to = if back { self.undo.pop() } else { self.redo.pop() }?;
        if back { &mut self.redo } else { &mut self.undo }.push(now.clone());
        self.base = Some(to.clone());
        Some(to)
    }
}

/// What a change is, for its reset.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Key {
    /// racer slot, row of `param_rows`
    Param(usize, usize),
    Gravity,
    /// racer slot, colour 0..3
    Paint(usize, usize),
    /// row of `cam_rows`
    Cam(usize),
}

/// One changed value: its group (the car's label, "World", "Camera"), the row's label, the value as loaded and now.
#[derive(Debug, Clone, PartialEq)]
pub struct Change {
    pub key: Key,
    pub group: String,
    pub label: String,
    pub was: String,
    pub now: String,
    /// a camera value that tune.cfg holds: kept on exit (not counted, not asked for)
    pub saved: bool,
}

impl Change {
    /// A car or track switch loses it (the camera values are live tunables and stay).
    pub fn on_switch(&self) -> bool {
        !matches!(self.key, Key::Cam(_))
    }
}

/// The racers' group names in the Changes card: the car label, "(you)" after the player's.
pub fn groups(sess: Option<&Session>) -> Vec<String> {
    let Some(s) = sess else { return Vec::new() };
    s.race
        .racers
        .iter()
        .map(|r| {
            let name = crate::scene::car_label(r.car_id, &r.car.p.name);
            if r.human { format!("{name} (you)") } else { name }
        })
        .collect()
}

/// Where `now` differs from `orig` (the loaded state; its camera = the source values, as the dots compare), the
/// camera rows equal to `cam_saved` (what tune.cfg holds) marked `saved`. Order: World, then per car its
/// parameters and paint, then Camera.
pub fn changes(now: &Snap, orig: &Snap, cam_saved: &CamTune, groups: &[String]) -> Vec<Change> {
    let mut out = Vec::new();
    if now.gravity != orig.gravity && !now.params.is_empty() {
        out.push(Change { key: Key::Gravity, group: "World".into(), label: "Gravity (track)".into(), was: num(orig.gravity), now: num(now.gravity), saved: false });
    }
    for (slot, (n, o)) in now.params.iter().zip(&orig.params).enumerate() {
        let group = groups.get(slot).cloned().unwrap_or_else(|| format!("slot {slot}"));
        let mut nn = n.clone();
        let rows = param_rows(&mut nn);
        for (i, (r, ov)) in rows.iter().zip(param_vals(o)).enumerate() {
            let nv = r.f.get();
            if nv != ov {
                out.push(Change { key: Key::Param(slot, i), group: group.clone(), label: r.label.clone(), was: ov.text(), now: nv.text(), saved: false });
            }
        }
        if let (Some(np), Some(op)) = (now.paints.get(slot), orig.paints.get(slot)) {
            for k in 0..3 {
                if np[k] != op[k] {
                    out.push(Change { key: Key::Paint(slot, k), group: group.clone(), label: format!("Colour {}", k + 1), was: hex(op[k]), now: hex(np[k]), saved: false });
                }
            }
        }
    }
    let saved = cam_vals(cam_saved);
    let mut nc = now.cam;
    for (i, (r, ov)) in cam_rows(&mut nc).iter().zip(cam_vals(&orig.cam)).enumerate() {
        let nv = r.f.get();
        if nv != ov {
            out.push(Change { key: Key::Cam(i), group: "Camera".into(), label: r.label.clone(), was: ov.text(), now: nv.text(), saved: saved.get(i) == Some(&nv) });
        }
    }
    out
}

/// The value of `key` in `orig` put into `now` (a row's reset in the Changes card).
pub fn reset(key: Key, now: &mut Snap, orig: &Snap) {
    match key {
        Key::Gravity => now.gravity = orig.gravity,
        Key::Param(slot, i) => {
            let Some(o) = orig.params.get(slot).map(param_vals) else { return };
            if let (Some(p), Some(&v)) = (now.params.get_mut(slot), o.get(i)) {
                if let Some(r) = param_rows(p).get_mut(i) {
                    r.f.set(v);
                }
            }
        }
        Key::Paint(slot, k) => {
            if let (Some(p), Some(o)) = (now.paints.get_mut(slot), orig.paints.get(slot)) {
                p[k] = o[k];
            }
        }
        Key::Cam(i) => {
            if let Some(&v) = cam_vals(&orig.cam).get(i) {
                if let Some(r) = cam_rows(&mut now.cam).get_mut(i) {
                    r.f.set(v);
                }
            }
        }
    }
}

/// The Changes card as Markdown (its copy button): a caption line, then a group / property / original / now table;
/// camera values that tune.cfg holds say so.
pub fn markdown(changes: &[Change], track: u32, car: &str) -> String {
    let mut out = format!("**Tuning changes** · track {track} · {car}\n\n| | Property | Original | Now |\n|---|---|---|---|\n");
    for c in changes {
        let group = if c.saved { format!("{} (in tune.cfg)", c.group) } else { c.group.clone() };
        out += &format!("| {} | {} | {} | {} |\n", group.replace('|', "\\|"), c.label.replace('|', "\\|"), c.was, c.now);
    }
    out
}

/// A number as the card shows it: up to 3 decimals, no trailing zeros.
pub fn num(v: f32) -> String {
    let s = format!("{v:.3}");
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s == "-0" { "0".into() } else { s.to_string() }
}

fn hex(c: [u8; 3]) -> String {
    format!("#{:02x}{:02x}{:02x}", c[0], c[1], c[2])
}

/// A row label as a `--tune-set` key: lower case, other characters as single dashes (`In-car eye` = `in-car-eye`).
pub fn slug(label: &str) -> String {
    let mut s = String::new();
    for ch in label.chars() {
        if ch.is_ascii_alphanumeric() {
            s.push(ch.to_ascii_lowercase());
        } else if !s.ends_with('-') {
            s.push('-');
        }
    }
    s.trim_matches('-').to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn params() -> CarParams {
        let (z, one) = (Vec3::ZERO, Vec3::ONE);
        CarParams {
            name: "test".into(),
            mass: 1300.0,
            gear_top: vec![-30.0, 80.0, 140.0],
            min_rpm: 1000.0,
            max_rpm: 7000.0,
            acc_force: 400.0,
            brake_force: 500.0,
            turn_force: 2000.0,
            com: z,
            wheels: [one; 4],
            corners: [one; 4],
            spring_len: 0.35,
            spring_k: 300.0,
            damper: 65000.0,
            grip: [1.0; 4],
            hull: motor_sim::body::Hull::from_box(-one, one),
            scale: 1.0,
            feel: 1.0,
        }
    }

    fn snap() -> Snap {
        Snap { params: vec![params(), params()], gravity: 15.81, paints: vec![[[1, 2, 3]; 3]; 2], cam: CamTune::default() }
    }

    #[test]
    fn one_edit_one_change() {
        let orig = snap();
        let mut now = orig.clone();
        now.params[1].mass += 150.0;
        let c = changes(&now, &orig, &CamTune::default(), &["A".into(), "B".into()]);
        assert_eq!(c.len(), 1);
        assert_eq!((c[0].key, c[0].group.as_str(), c[0].label.as_str()), (Key::Param(1, 0), "B", "Mass"));
        assert_eq!(c[0].now, num(orig.params[1].mass + 150.0));
        assert!(c[0].on_switch() && !c[0].saved);
        let mut back = now.clone();
        reset(c[0].key, &mut back, &orig);
        assert!(changes(&back, &orig, &CamTune::default(), &[]).is_empty());
    }

    #[test]
    fn gears_paint_and_gravity() {
        let orig = snap();
        let mut now = orig.clone();
        now.params[0].gear_top[2] = 150.0;
        now.paints[0][1] = [255, 0, 16];
        now.gravity = 12.0;
        let c = changes(&now, &orig, &CamTune::default(), &[]);
        let labels: Vec<_> = c.iter().map(|c| (c.group.as_str(), c.label.as_str(), c.now.as_str())).collect();
        assert_eq!(labels, [("World", "Gravity (track)", "12"), ("slot 0", "Gear 2 top", "150"), ("slot 0", "Colour 2", "#ff0010")]);
    }

    #[test]
    fn camera_saved_in_tune_cfg() {
        let orig = snap();
        let mut now = orig.clone();
        now.cam.k_y += 0.1;
        now.cam.chase_eye.y += 0.5;
        // tune.cfg holds the eye, not the easing
        let saved = CamTune { chase_eye: now.cam.chase_eye, ..CamTune::default() };
        let c = changes(&now, &orig, &saved, &[]);
        assert_eq!(c.len(), 2);
        assert!(c.iter().all(|c| !c.on_switch()));
        assert_eq!(c.iter().map(|c| (c.label.as_str(), c.saved)).collect::<Vec<_>>(), [("Chase eye", true), ("Easing y", false)]);
    }

    #[test]
    fn undo_redo_and_settle() {
        let mut u = Undo::default();
        let mut now = snap();
        u.settle(&now, false);
        assert!(!u.can_undo());
        // a drag: the pointer is down, nothing is pushed until the release
        now.params[0].mass = 1400.0;
        u.settle(&now, true);
        now.params[0].mass = 1500.0;
        u.settle(&now, true);
        assert!(!u.can_undo());
        u.settle(&now, false);
        assert_eq!(u.undo.len(), 1);
        // a second edit
        now.gravity = 10.0;
        u.settle(&now, false);
        assert_eq!(u.undo.len(), 2);
        // undo twice, redo once
        let a = u.step(&now, true).unwrap();
        assert_eq!((a.gravity, a.params[0].mass), (15.81, 1500.0));
        let b = u.step(&a, true).unwrap();
        assert_eq!(b.params[0].mass, 1300.0);
        assert!(u.step(&b, true).is_none() && u.can_redo());
        let c = u.step(&b, false).unwrap();
        assert_eq!(c.params[0].mass, 1500.0);
        // a new edit drops the redo stack
        let mut d = c.clone();
        d.paints[1][0] = [9, 9, 9];
        u.settle(&d, false);
        assert!(!u.can_redo() && u.can_undo());
        u.clear();
        assert!(!u.can_undo() && u.base.is_none());
    }

    #[test]
    fn slugs() {
        let mut s = snap();
        assert!(s.set_slug(0, "acc-force", 99.0) && s.params[0].acc_force == 99.0);
        assert!(s.set_slug(1, "centre-of-mass-y", 0.25) && s.params[1].com.y == 0.25);
        assert!(s.set_slug(0, "gear-1-top", 90.0) && s.params[0].gear_top[1] == 90.0);
        assert!(s.set_slug(0, "in-car-eye-z", 1.5) && s.cam.incar_eye.z == 1.5);
        assert!(s.set_slug(0, "gravity", 9.0) && s.gravity == 9.0);
        assert!(!s.set_slug(0, "no-such-row", 1.0));
    }

    #[test]
    fn markdown_table() {
        let c = vec![
            Change { key: Key::Param(0, 0), group: "01 Kobra X3 (you)".into(), label: "Mass".into(), was: "1300".into(), now: "1450".into(), saved: false },
            Change { key: Key::Cam(7), group: "Camera".into(), label: "Easing y".into(), was: "0.2".into(), now: "0.3".into(), saved: true },
        ];
        assert_eq!(
            markdown(&c, 2, "01 Kobra X3"),
            "**Tuning changes** · track 2 · 01 Kobra X3\n\n| | Property | Original | Now |\n|---|---|---|---|\n| 01 Kobra X3 (you) | Mass | 1300 | 1450 |\n| Camera (in tune.cfg) | Easing y | 0.2 | 0.3 |\n"
        );
        assert_eq!((num(0.350), num(-0.0001), num(1450.0)), ("0.35".to_string(), "0".to_string(), "1450".to_string()));
    }
}
