//! DEBUG: developer-viewer instrumentation (motorview only, never drawn by the game): world
//! gizmos as line lists, labels at world points, and text panels of the sim state.
//! Positions taken here are game space (left-handed, z forward); the output is view space.

use crate::drive::Session;
use crate::pick::{Bounds, Picker};
use crate::render::{Camera, HEADLIGHT_RANGE, PROP_LIGHT_RANGE, TAIL_LIGHT_RANGE};
use crate::scene::{HaloInst, Scene, Vertex, CAR_MESHES, WHEEL_X_SCALE};
use glam::{Mat3, Mat4, Vec3};
use motor_sim::race::fmt_ms;
use motor_sim::{Race, World};

const FLIP: Vec3 = Vec3::new(1.0, 1.0, -1.0);

pub const RED: [f32; 4] = [1.0, 0.2, 0.15, 1.0];
pub const GREEN: [f32; 4] = [0.2, 1.0, 0.3, 1.0];
pub const BLUE: [f32; 4] = [0.3, 0.5, 1.0, 1.0];
pub const YELLOW: [f32; 4] = [1.0, 0.9, 0.2, 1.0];
pub const CYAN: [f32; 4] = [0.2, 0.9, 1.0, 1.0];
pub const MAGENTA: [f32; 4] = [1.0, 0.3, 1.0, 1.0];
pub const ORANGE: [f32; 4] = [1.0, 0.55, 0.1, 1.0];
pub const GREY: [f32; 4] = [0.55, 0.55, 0.55, 1.0];
pub const WHITE: [f32; 4] = [1.0, 1.0, 1.0, 1.0];

/// Which gizmo layers are drawn.
#[derive(Clone, Copy, Debug, Default)]
pub struct Layers {
    /// focused car: axes, hull, wheels and springs, velocity, contacts, AI target
    pub car: bool,
    /// the same for every car
    pub all_cars: bool,
    /// centreline, checkpoints, finish line, AI racing line with widths
    pub track: bool,
    /// COL faces around the camera, coloured by surface
    pub collision: bool,
    /// bounding boxes of world objects near the camera, boxes and spheres of cars and props
    pub bounds: bool,
    /// halo segments and the reach of the REMASTER dynamic lights
    pub lights: bool,
    /// REMASTER irradiance probes near the camera (probes.rs)
    pub probes: bool,
}

impl Layers {
    /// Parse a comma list (`--debug car,track,col,all`).
    pub fn parse(list: &str) -> Result<Layers, String> {
        let mut l = Layers::default();
        for name in list.split(',').map(str::trim).filter(|s| !s.is_empty()) {
            match name {
                "car" => l.car = true,
                "all" | "all-cars" => (l.car, l.all_cars) = (true, true),
                "track" => l.track = true,
                "col" | "collision" => l.collision = true,
                "bounds" => l.bounds = true,
                "lights" => l.lights = true,
                "probes" => l.probes = true,
                _ => return Err(format!("unknown debug layer {name} (car, all, track, col, bounds, lights, probes)")),
            }
        }
        Ok(l)
    }
}

/// Line lists (view space) and labels (view space point, text, colour) built for one frame.
#[derive(Default)]
pub struct Gizmos {
    /// depth tested
    pub lines: Vec<Vertex>,
    /// drawn over everything
    pub xray: Vec<Vertex>,
    /// solid triangles drawn over everything (the world editor's gizmos), view space like the lines
    pub xray_tris: Vec<Vertex>,
    pub labels: Vec<(Vec3, String, [f32; 4])>,
}

fn vtx(p: Vec3, color: [f32; 4]) -> Vertex {
    Vertex { pos: (p * FLIP).into(), uv: [0.0; 2], color, layer: -1.0, normal: [0.0; 3], env: 0.0 }
}

impl Gizmos {
    pub fn line(&mut self, a: Vec3, b: Vec3, c: [f32; 4], xray: bool) {
        let dst = if xray { &mut self.xray } else { &mut self.lines };
        dst.extend([vtx(a, c), vtx(b, c)]);
    }

    /// A solid triangle (game space) drawn over everything.
    pub fn tri(&mut self, a: Vec3, b: Vec3, c: Vec3, col: [f32; 4]) {
        self.xray_tris.extend([vtx(a, col), vtx(b, col), vtx(c, col)]);
    }

    /// Arrow from `from` along `v`, head 20 % of its length.
    pub fn arrow(&mut self, from: Vec3, v: Vec3, c: [f32; 4], xray: bool) {
        let len = v.length();
        if len < 1e-3 {
            return;
        }
        let tip = from + v;
        let d = v / len;
        let side = d.cross(if d.y.abs() > 0.9 { Vec3::X } else { Vec3::Y }).normalize();
        let h = (len * 0.2).min(0.6);
        self.line(from, tip, c, xray);
        self.line(tip, tip - d * h + side * h * 0.5, c, xray);
        self.line(tip, tip - d * h - side * h * 0.5, c, xray);
    }

    pub fn cross(&mut self, p: Vec3, r: f32, c: [f32; 4], xray: bool) {
        for a in [Vec3::X, Vec3::Y, Vec3::Z] {
            self.line(p - a * r, p + a * r, c, xray);
        }
    }

    pub fn polyline(&mut self, pts: &[Vec3], closed: bool, c: [f32; 4], xray: bool) {
        for w in pts.windows(2) {
            self.line(w[0], w[1], c, xray);
        }
        if let (true, Some(a), Some(b)) = (closed && pts.len() > 2, pts.last(), pts.first()) {
            self.line(*a, *b, c, xray);
        }
    }

    /// Circles of radius `r` around `c` in the three axis planes.
    pub fn sphere(&mut self, c: Vec3, r: f32, col: [f32; 4], xray: bool) {
        const N: usize = 24;
        for (a, b) in [(Vec3::X, Vec3::Y), (Vec3::Y, Vec3::Z), (Vec3::Z, Vec3::X)] {
            let pts: Vec<Vec3> = (0..N).map(|i| (i as f32 / N as f32 * std::f32::consts::TAU).sin_cos()).map(|(s, k)| c + (a * k + b * s) * r).collect();
            self.polyline(&pts, true, col, xray);
        }
    }

    /// The 12 edges of a box given by its 8 corners (bit 0 x, bit 1 y, bit 2 z of the corner index).
    pub fn cuboid(&mut self, k: [Vec3; 8], c: [f32; 4], xray: bool) {
        for i in 0..8 {
            for bit in [1, 2, 4] {
                if i & bit == 0 {
                    self.line(k[i], k[i | bit], c, xray);
                }
            }
        }
    }

    pub fn label(&mut self, p: Vec3, text: impl Into<String>, c: [f32; 4]) {
        self.labels.push((p * FLIP, text.into(), c));
    }
}

/// A distinct colour per index (surface materials).
pub fn palette(i: usize) -> [f32; 4] {
    let h = (i as f32 * 0.618_034).fract() * 6.0;
    let x = 1.0 - (h % 2.0 - 1.0).abs();
    let (r, g, b) = match h as u32 {
        0 => (1.0, x, 0.0),
        1 => (x, 1.0, 0.0),
        2 => (0.0, 1.0, x),
        3 => (0.0, x, 1.0),
        4 => (x, 0.0, 1.0),
        _ => (1.0, 0.0, x),
    };
    [0.25 + 0.75 * r, 0.25 + 0.75 * g, 0.25 + 0.75 * b, 1.0]
}

/// tan of half the horizontal field of view of `cam` in a view whose field is `fov_x` degrees at zoom 1 (render.rs
/// `Gpu::draw`: the TV camera's zoom narrows it, 0x40b498).
fn half_tan(cam: &Camera, fov_x: f32) -> f32 {
    (fov_x.to_radians() / 2.0).tan() / cam.zoom.max(1e-3)
}

/// Window position (pixels) of a view-space point, as the main view projects it (render.rs `Gpu::draw`: `fov_x` =
/// `View::fov_x`, the camera's zoom and roll; view distance and mirror ignored); None behind the camera or off screen.
pub fn project(cam: &Camera, size: (u32, u32), fov_x: f32, p: Vec3) -> Option<(f32, f32)> {
    let aspect = size.0 as f32 / size.1.max(1) as f32;
    let fov = 2.0 * (half_tan(cam, fov_x) / aspect).atan();
    let proj = glam::camera::rh::proj::directx::perspective(fov, aspect, 0.2, 1000.0);
    let view = glam::camera::rh::view::look_to_mat4(cam.pos, cam.forward(), cam.up());
    let c = (proj * view) * p.extend(1.0);
    if c.w <= 0.2 {
        return None;
    }
    let (x, y) = (c.x / c.w, c.y / c.w);
    ((-1.0..=1.0).contains(&x) && (-1.0..=1.0).contains(&y)).then_some(((x + 1.0) * 0.5 * size.0 as f32, (1.0 - y) * 0.5 * size.1 as f32))
}

/// Unit view-space ray through window pixel `px` (the inverse of `project`).
pub fn cursor_ray(cam: &Camera, size: (u32, u32), fov_x: f32, px: (f32, f32)) -> Vec3 {
    let aspect = size.0 as f32 / size.1.max(1) as f32;
    let (nx, ny) = (px.0 / size.0.max(1) as f32 * 2.0 - 1.0, 1.0 - px.1 / size.1.max(1) as f32 * 2.0);
    // at depth 1, x spans +-tan(fov_x / 2) / zoom and y that over the aspect
    let k = half_tan(cam, fov_x);
    let view = glam::camera::rh::view::look_to_mat4(cam.pos, cam.forward(), cam.up());
    view.inverse().transform_vector3(Vec3::new(nx * k, ny * k / aspect, -1.0)).normalize_or(cam.forward())
}

const WHEEL_NAMES: [&str; 4] = ["FL", "FR", "BR", "BL"];
/// Velocity arrows: metres of arrow per m/s.
const VEL_SCALE: f32 = 0.15;

/// Car gizmos for racer `slot`: body axes, hull corners, springs (green free .. red
/// compressed), wheel contacts (surface colour, grey in the air), velocity (yellow) and its
/// sideways part (cyan), angular velocity (magenta), hull hits (red normals), wall contacts
/// (orange) and the AI's current racing-line node.
pub fn car(g: &mut Gizmos, sess: &Session, slot: usize, label: bool) {
    let Some(r) = sess.race.racers.get(slot) else { return };
    let car = &r.car;
    let at = |v: Vec3| car.pos + car.rot * v;
    let (x, y, z) = (car.rot.x_axis, car.rot.y_axis, car.rot.z_axis);
    g.line(car.pos, car.pos + x * 1.5, RED, true);
    g.line(car.pos, car.pos + y * 1.5, GREEN, true);
    g.line(car.pos, car.pos + z * 1.5, BLUE, true);
    g.polyline(&car.p.corners.map(at), true, WHITE, true);
    g.cross(at(car.p.com), 0.15, WHITE, true);
    let radius = sess.wheel_radius(slot);
    for w in 0..4 {
        let hub = car.p.wheels[w];
        let top = at(Vec3::new(hub.x * WHEEL_X_SCALE, hub.y, hub.z));
        let (_, contact) = Session::wheel_local(car, w, radius);
        let e = car.wheel_ext[w].clamp(0.0, 1.0);
        g.line(top, at(contact), [1.0 - e, e, 0.2, 1.0], true);
        let c = match (car.wheel_contact[w], car.wheel_mat[w]) {
            (true, Some(m)) => palette(m),
            (true, None) => WHITE,
            _ => GREY,
        };
        g.cross(at(contact), 0.2, c, true);
    }
    let up = car.pos + y * 1.2;
    g.arrow(up, car.world_vel() * VEL_SCALE, YELLOW, true);
    g.arrow(up, x * car.vel.x * VEL_SCALE, CYAN, true);
    g.arrow(car.pos, car.rot * car.ang, MAGENTA, true);
    for &(n, depth) in &car.hull_hits {
        g.arrow(car.pos, n * (1.0 + depth * 20.0), RED, true);
    }
    for h in car.world_hits.iter().flatten() {
        g.cross(at(car.hull_point(h.point)), 0.3, ORANGE, true);
    }
    if !r.human {
        if let Some(n) = sess.race.ai_line.nodes.get(r.ai.node()) {
            g.line(car.pos, Vec3::new(n.x, car.pos.y, n.z), CYAN, false);
        }
    }
    if label {
        g.label(car.pos + y * 2.0, format!("#{slot} {}", r.driver.name), WHITE);
    }
}

/// Track geometry for the gizmos, computed once per race (ground heights need raycasts).
pub struct TrackCache {
    /// centreline every `CENTRE_STEP` m
    pub centre: Vec<Vec3>,
    /// (position, right vector, checkpoint number)
    pub checkpoints: Vec<(Vec3, Vec3, u32)>,
    /// AI node: position on the ground, left and right road edges
    pub nodes: Vec<(Vec3, Vec3, Vec3)>,
}

const CENTRE_STEP: f32 = 4.0;
/// Track labels (node numbers) are drawn within this distance of the camera.
const LABEL_RANGE: f32 = 120.0;

/// Right of a horizontal direction, game axes (x right, z forward).
pub fn right_of(d: Vec3) -> Vec3 {
    Vec3::new(d.z, 0.0, -d.x).normalize_or(Vec3::X)
}

impl TrackCache {
    pub fn new(race: &Race) -> TrackCache {
        let line = &race.line;
        let centre = (0..(line.length / CENTRE_STEP) as usize).map(|i| line.at(i as f32 * CENTRE_STEP).0 + Vec3::Y * 0.3).collect();
        let checkpoints = line
            .checkpoints
            .iter()
            .map(|&(s, n)| {
                let (p, d) = line.at(s);
                (p, right_of(d), n)
            })
            .collect();
        // node heights: follow the road from node to node (a ray from just above the previous
        // node's height), so bridges and tunnels keep the right level [design choice]
        let mut y = None;
        let mut nodes = Vec::with_capacity(race.ai_line.nodes.len());
        for n in &race.ai_line.nodes {
            let top = y.map_or(1000.0, |y: f32| y + 5.0);
            let h = race.world.ground(n.x, n.z, top).or_else(|| race.world.ground(n.x, n.z, 1000.0)).or(y).unwrap_or(0.0);
            y = Some(h);
            let p = Vec3::new(n.x, h + 0.25, n.z);
            let r = right_of(Vec3::from(n.dir));
            nodes.push((p, p - r * n.w_left, p + r * n.w_right));
        }
        TrackCache { centre, checkpoints, nodes }
    }
}

/// Track gizmos: centreline (blue), checkpoints (yellow gates), finish (magenta), AI racing
/// line with its road edges (orange) and node numbers near `eye`.
pub fn track(g: &mut Gizmos, race: &Race, cache: &TrackCache, eye: Vec3) {
    g.polyline(&cache.centre, true, BLUE, false);
    for &(p, r, n) in &cache.checkpoints {
        g.line(p - r * 8.0, p + r * 8.0, YELLOW, false);
        g.line(p - r * 8.0, p - r * 8.0 + Vec3::Y * 4.0, YELLOW, false);
        g.line(p + r * 8.0, p + r * 8.0 + Vec3::Y * 4.0, YELLOW, false);
        g.label(p + Vec3::Y * 4.5, format!("CP {n}"), YELLOW);
    }
    let (fp, fr) = (race.line.finish_point, right_of(race.line.finish_dir));
    g.line(fp - fr * 10.0, fp + fr * 10.0, MAGENTA, true);
    g.label(fp + Vec3::Y * 3.0, "FINISH", MAGENTA);
    let pts: Vec<Vec3> = cache.nodes.iter().map(|n| n.0).collect();
    g.polyline(&pts, true, ORANGE, false);
    for (i, &(p, l, r)) in cache.nodes.iter().enumerate() {
        g.line(l, r, [0.6, 0.35, 0.1, 1.0], false);
        if i % 5 == 0 && p.distance(eye) < LABEL_RANGE {
            g.label(p + Vec3::Y * 0.5, i.to_string(), ORANGE);
        }
    }
}

/// COL faces as polygons (the file stores planes): built once per track.
pub struct ColCache {
    /// (outline, .SRF material, centre, bounding radius)
    faces: Vec<(Vec<Vec3>, u16, Vec3, f32)>,
}

/// Corners of a COL face: where its plane meets pairs of its edge planes, kept when inside
/// every edge plane, ordered around the centre.
pub fn face_outline(col: &motor_data::col::Collision, face: &motor_data::col::Face) -> Vec<Vec3> {
    let plane = |e: u16| {
        let [nx, ny, nz, d] = *col.edges.get(((e - 1) & 0x7fff) as usize)?;
        let s = if e & 0x8000 != 0 { -1.0 } else { 1.0 };
        Some((Vec3::new(nx, ny, nz) * s, d * s))
    };
    let edges: Vec<(Vec3, f32)> = face.edge_slots().filter_map(plane).collect();
    let n = Vec3::new(face.plane[0], face.plane[1], face.plane[2]);
    let mut pts: Vec<Vec3> = Vec::new();
    for i in 0..edges.len() {
        for j in i + 1..edges.len() {
            let m = Mat3::from_cols(n, edges[i].0, edges[j].0).transpose();
            if m.determinant().abs() < 1e-6 {
                continue;
            }
            let p = m.inverse() * -Vec3::new(face.plane[3], edges[i].1, edges[j].1);
            if edges.iter().all(|&(en, ed)| en.dot(p) + ed >= -0.01) && !pts.iter().any(|q| q.distance(p) < 1e-3) {
                pts.push(p);
            }
        }
    }
    if pts.len() < 3 {
        return Vec::new();
    }
    let c = pts.iter().sum::<Vec3>() / pts.len() as f32;
    let u = (pts[0] - c).normalize_or(Vec3::X);
    let v = n.cross(u);
    pts.sort_by(|a, b| {
        let ang = |p: &Vec3| (*p - c).dot(v).atan2((*p - c).dot(u));
        ang(a).total_cmp(&ang(b))
    });
    pts
}

impl ColCache {
    pub fn new(world: &World) -> ColCache {
        let faces = world
            .col
            .faces
            .iter()
            .map(|f| {
                // lifted off the surface against z-fighting with the visual mesh
                let lift = Vec3::new(f.plane[0], f.plane[1], f.plane[2]) * 0.03;
                let pts: Vec<Vec3> = face_outline(&world.col, f).into_iter().map(|p| p + lift).collect();
                let c = if pts.is_empty() { Vec3::splat(f32::MAX) } else { pts.iter().sum::<Vec3>() / pts.len() as f32 };
                let r = pts.iter().map(|p| p.distance(c)).fold(0.0, f32::max);
                (pts, f.material, c, r)
            })
            .collect();
        ColCache { faces }
    }

    pub fn outline(&self, face: usize) -> Option<&[Vec3]> {
        self.faces.get(face).map(|f| f.0.as_slice())
    }
}

/// COL faces within `range` of `centre`, outlined in their surface colour with a short normal;
/// face `highlight` (the crosshair's) drawn over everything.
pub fn collision(g: &mut Gizmos, world: &World, cache: &ColCache, centre: Vec3, range: f32, highlight: Option<usize>) {
    for (i, (pts, mat, c, r)) in cache.faces.iter().enumerate() {
        if pts.is_empty() || c.distance(centre) > range + r {
            continue;
        }
        let colour = palette(world.material_of(*mat));
        g.polyline(pts, true, colour, false);
        let f = &world.col.faces[i];
        g.line(*c, *c + Vec3::new(f.plane[0], f.plane[1], f.plane[2]) * 0.5, colour, false);
    }
    if let Some(pts) = highlight.and_then(|h| cache.outline(h)) {
        g.polyline(pts, true, WHITE, true);
    }
}

fn v3(v: Vec3) -> String {
    format!("{:7.2} {:7.2} {:7.2}", v.x, v.y, v.z)
}

/// Text panel for racer `slot`: physics state, wheels, progress and AI.
pub fn car_panel(sess: &Session, slot: usize) -> Vec<String> {
    let Some(r) = sess.race.racers.get(slot) else { return Vec::new() };
    let (c, t, w) = (&r.car, &r.telemetry, &sess.race.world);
    let mat = |m: Option<usize>| m.and_then(|m| w.materials.get(m)).map_or("-", |m| m.name.as_str());
    let fwd = c.rot.z_axis;
    let mut out = vec![
        format!("car #{slot} car{:02} {} ({})", r.car_id, r.driver.name, if r.human { "human" } else { "AI" }),
        format!("pos   {}  heading {:.0} deg", v3(c.pos), fwd.x.atan2(fwd.z).to_degrees()),
        format!("vel   {}  {:.0} km/h", v3(c.vel), t.kmh),
        format!("ang   {}", v3(c.ang)),
        format!("gear {} rpm {:.0} drive {:.0} steer {:.2} brake {:.2}", if c.gear == 0 { "R".into() } else { c.gear.to_string() }, c.rpm, t.drive_force, c.steer_smooth, r.brake),
        format!("sliding {} wheelspin {} clutch {} (limit {:.0}) scraping {} impact {:.1} hull hits {} airborne {:.1}s", c.sliding, c.wheelspin, c.clutch, c.spin_limit, c.scraping, c.impact, c.hull_hits.len(), r.airborne_s),
    ];
    for (i, name) in WHEEL_NAMES.iter().enumerate() {
        out.push(format!("  {name} ext {:.2} {} {}", c.wheel_ext[i], if c.wheel_contact[i] { "down" } else { "air " }, mat(c.wheel_mat[i])));
    }
    out.push(format!("progress: {}{}", r.progress.debug(), if r.progress.wrong_way { " WRONG WAY" } else { "" }));
    out.push(format!("ai: {}", r.ai.debug()));
    out
}

/// Text panel of every racer in race order.
pub fn racers_panel(sess: &Session) -> Vec<String> {
    let race = &sess.race;
    let mut out = vec![format!("race {}  clock {}  laps {}", if race.started() { "running" } else { "countdown" }, fmt_ms(race.clock_ms as u32), race.laps)];
    for (place, &i) in race.standings().iter().enumerate() {
        let r = &race.racers[i];
        let mark = if i == sess.focus { '>' } else { ' ' };
        let done = r.progress.finished_ms.map_or(String::new(), |f| format!(" fin {}", fmt_ms(f)));
        out.push(format!("{mark}{} #{i} {:<20} lap {} {:3.0} km/h node {:4}{done}", place + 1, r.driver.name, r.progress.lap, r.telemetry.kmh, r.ai.node()));
    }
    out
}

/// World objects within this distance of the camera get their bounding box (m).
const BOUNDS_RANGE: f32 = 150.0;

/// Corners of `b` under the view-space `model`, in game space.
fn corners(b: &Bounds, model: Mat4) -> [Vec3; 8] {
    std::array::from_fn(|i| {
        let p = Vec3::new([b.lo.x, b.hi.x][i & 1], [b.lo.y, b.hi.y][(i >> 1) & 1], [b.lo.z, b.hi.z][(i >> 2) & 1]);
        model.transform_point3(p) * FLIP
    })
}

/// Bounding volumes (pick.rs `Bounds`): world objects near `eye` (grey boxes, the one under
/// the crosshair in white), car parts (yellow box and sphere), scene objects (magenta).
/// `cars` / `props` are the frame's (mesh, view-space model) lists.
pub fn bounds(g: &mut Gizmos, picker: &Picker, eye: Vec3, cars: &[(usize, Mat4)], props: &[(usize, Mat4)], target: Option<usize>) {
    for (i, b) in picker.world_bounds().enumerate() {
        let near = (b.center * FLIP).distance(eye) < BOUNDS_RANGE + b.radius;
        if near || target == Some(i) {
            g.cuboid(corners(&b, Mat4::IDENTITY), if target == Some(i) { WHITE } else { GREY }, target == Some(i));
        }
    }
    let inst = |g: &mut Gizmos, b: Option<Bounds>, m: Mat4, c: [f32; 4]| {
        let Some(b) = b else { return };
        g.cuboid(corners(&b, m), c, false);
        g.sphere(m.transform_point3(b.center) * FLIP, b.radius * m.x_axis.truncate().length(), [c[0] * 0.6, c[1] * 0.6, c[2] * 0.6, 1.0], false);
    };
    for &(mesh, m) in cars {
        inst(g, picker.car_bounds(mesh), m, YELLOW);
    }
    for &(mesh, m) in props {
        inst(g, picker.prop_bounds(mesh), m, MAGENTA);
    }
}

/// Lights: every halo as its segment (world yellow, scene objects orange, cars white) and the
/// reach of the REMASTER dynamic lights built from them (render.rs `dynamic_lights`): a sphere
/// of `PROP_LIGHT_RANGE` per scene-object halo, the headlight beam (`HEADLIGHT_RANGE`, an arrow
/// along it) and the tail light's (`TAIL_LIGHT_RANGE`, backwards). Within `range` of `eye` only.
pub fn lights(g: &mut Gizmos, s: &Scene, eye: Vec3, cars: &[(usize, Mat4)], props: &[(usize, Mat4)], range: f32) {
    let seg = |g: &mut Gizmos, h: &HaloInst, m: Mat4, c: [f32; 4]| -> Option<Vec3> {
        let (a, b) = (m.transform_point3(h.p0) * FLIP, m.transform_point3(h.p1) * FLIP);
        let mid = (a + b) * 0.5;
        if mid.distance(eye) > range {
            return None;
        }
        g.line(a, b, c, false);
        g.cross(mid, 0.15, c, false);
        Some(mid)
    };
    for h in &s.world_halos {
        seg(g, h, Mat4::IDENTITY, YELLOW);
    }
    for &(mesh, m) in props {
        for h in s.prop_halos.get(mesh).into_iter().flatten() {
            if let Some(p) = seg(g, h, m, ORANGE) {
                g.sphere(p, PROP_LIGHT_RANGE, [0.6, 0.35, 0.1, 1.0], false);
            }
        }
    }
    for &(mesh, m) in cars.iter().filter(|c| c.0 % CAR_MESHES == 0) {
        let fwd = m.transform_vector3(Vec3::NEG_Z).normalize_or_zero() * FLIP;
        for h in s.car_halos.get(mesh).into_iter().flatten() {
            let Some(p) = seg(g, h, m, WHITE) else { continue };
            // view space: the front of a car is -z
            if h.p0.z + h.p1.z < 0.0 {
                g.arrow(p, fwd * HEADLIGHT_RANGE, [0.9, 0.9, 0.6, 1.0], false);
            } else {
                g.arrow(p, -fwd * TAIL_LIGHT_RANGE, [0.8, 0.15, 0.1, 1.0], false);
            }
        }
    }
}

/// REMASTER irradiance probes (probes.rs, #83) within `range` of `eye`: a cross per probe,
/// coloured by the ambient an upward face gets there with the track's flat `ObjectAmbient`
/// (`vis * lin(ambient) + bounce`, back to gamma); probes inside geometry (their value filled
/// from the neighbours) magenta. A yellow tick points from each probe to the sun (`sun_dir`,
/// view space), as long as the share of the sun it sees (none in a tunnel, #192).
pub fn probes(g: &mut Gizmos, vol: &crate::probes::ProbeVolume, ambient: [f32; 3], sun_dir: [f32; 3], eye: Vec3, range: f32) {
    let amb = Vec3::from(ambient).powf(2.2);
    let sun = Vec3::from(sun_dir) * FLIP;
    for (p, pr) in vol.near(eye * FLIP, range) {
        if pr.sun > 0.01 {
            g.line(p * FLIP, p * FLIP + sun * (0.6 * pr.sun), [1.0, 0.85, 0.2, 1.0], false);
        }
        let c = if pr.valid() {
            let (vis, bounce) = pr.eval(Vec3::Y);
            let e = (amb * vis + bounce).powf(1.0 / 2.2).min(Vec3::ONE);
            [e.x, e.y, e.z, 1.0]
        } else {
            MAGENTA
        };
        g.cross(p * FLIP, 0.2, c, false);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cursor_ray_inverts_project_with_zoom_and_roll() {
        let mut cam = Camera::new(Vec3::new(3.0, 2.0, -4.0), 0.7, -0.2);
        (cam.zoom, cam.roll) = (2.5, 0.4);
        let size = (1280, 720);
        for px in [(100.0, 80.0), (640.0, 360.0), (1200.0, 650.0)] {
            for fov in [90.0, 60.0] {
                let p = cam.pos + cursor_ray(&cam, size, fov, px) * 25.0;
                let (x, y) = project(&cam, size, fov, p).expect("in view");
                assert!((x - px.0).abs() < 0.05 && (y - px.1).abs() < 0.05, "{px:?} -> {x} {y}");
            }
        }
        // the centre is straight ahead
        assert!(cursor_ray(&cam, size, 90.0, (640.0, 360.0)).abs_diff_eq(cam.forward(), 1e-5));
    }
}
