//! DEBUG: motorview's inspector (everything about the crosshair target), data browser (world
//! textures, scene-object meshes, cars) and per-pass visibility. Positions are printed in game
//! coordinates, as `--eye` / `--at` take them.

use crate::drive::{FrameParts, Session};
use crate::pick::{Bounds, Hit, Picker, Target};
use crate::render::View;
use crate::scene::{Scene, Vertex, CAR_MESHES};
use glam::{Mat4, Vec3};

const FLIP: Vec3 = Vec3::new(1.0, 1.0, -1.0);

fn g3(v: Vec3) -> String {
    let g = v * FLIP;
    format!("{:.2}, {:.2}, {:.2}", g.x, g.y, g.z)
}

fn bounds_line(b: &Bounds) -> String {
    let d = b.hi - b.lo;
    format!("bounds: centre {}  size {:.2} x {:.2} x {:.2}  radius {:.2}", g3(b.center), d.x, d.y, d.z, b.radius)
}

/// The pinned panel of a hit: object, mesh, triangle (vertices, uv, colour, normal), texture
/// and, for scene objects, the TCB motion state. Also the texture layer to preview.
pub fn target_details(s: &Scene, picker: &Picker, sess: Option<&Session>, hit: &Hit) -> (Vec<String>, Option<usize>) {
    let mut out = Vec::new();
    let (mesh, bounds): (&[Vertex], Option<Bounds>) = match hit.target {
        Target::World { object, offset, texture } => {
            let r = s.world_objects.get(object).map_or(0..0, |o| o.0.clone());
            out.push(format!("MYW object #{object} at +{offset:#x}: triangles {}..{} ({})", r.start, r.end, r.len()));
            out.push(format!("triangle #{} (#{} of the object)", hit.tri, hit.tri.saturating_sub(r.start)));
            if let Some(i) = texture {
                out.push(format!("texture #{i} {}", s.world_textures.get(i).map_or("?", |n| n.as_str())));
            }
            (&s.tris, picker.world_bounds().nth(object))
        }
        Target::Car { slot, part } => {
            let mesh = slot * CAR_MESHES + part;
            if let Some(r) = sess.and_then(|x| x.race.racers.get(slot)) {
                out.push(format!("{}, slot {slot}, driver {} ({})", s.car_names.get(slot).map_or("?", |n| n.as_str()), r.driver.name, if r.human { "human" } else { "AI" }));
            }
            let v = s.cars.get(mesh).map_or(&[][..], |v| v.as_slice());
            out.push(format!("mesh {mesh}: {} triangles, triangle #{}", v.len() / 3, hit.tri));
            (v, picker.car_bounds(mesh))
        }
        Target::Prop { index, mesh } => {
            let v = s.props.get(mesh).map_or(&[][..], |v| v.as_slice());
            out.push(format!("instance #{index}, mesh {mesh}: {} triangles, triangle #{}", v.len() / 3, hit.tri));
            if let Some(ss) = sess {
                out.extend(ss.race.props.debug(index));
            }
            (v, picker.prop_bounds(mesh))
        }
    };
    if let Some(b) = bounds {
        out.push(bounds_line(&b));
    }
    let mut layer = None;
    if let Some(t) = mesh.get(hit.tri * 3..hit.tri * 3 + 3) {
        for (k, v) in t.iter().enumerate() {
            let c = v.color;
            out.push(format!("v{k} {}  uv {:.3}, {:.3}  rgba {:.2} {:.2} {:.2} {:.2}", g3(Vec3::from(v.pos)), v.uv[0], v.uv[1], c[0], c[1], c[2], c[3]));
        }
        out.push(format!("normal {}  layer {}  env {}", g3(Vec3::from(t[0].normal)), t[0].layer, t[0].env));
        layer = (t[0].layer >= 0.0).then_some(t[0].layer as usize);
        if !matches!(hit.target, Target::World { .. }) {
            out.push("(mesh space: car-local / object-local, game axes)".into());
        }
    }
    (out, layer)
}

/// Which parts of the scene are drawn (render debug panel, F1).
#[derive(Clone, Copy, Debug)]
pub struct Passes {
    pub world: bool,
    pub sky: bool,
    pub props: bool,
    pub bodies: bool,
    pub wheels: bool,
    pub sprites: bool,
    pub skids: bool,
    pub shadows: bool,
    /// REMASTER: the Ultra grass blades
    pub blades: bool,
    /// REMASTER: the sun shafts of the volumetric fog
    pub shafts: bool,
    /// halos and lens flares (as the G key; pixmatch's halo-free background, #457)
    pub halos: bool,
}

impl Default for Passes {
    fn default() -> Self {
        Passes { world: true, sky: true, props: true, bodies: true, wheels: true, sprites: true, skids: true, shadows: true, blades: true, shafts: true, halos: true }
    }
}

impl Passes {
    pub const NAMES: [&'static str; 11] = ["world", "sky", "props", "bodies", "wheels", "sprites", "skids", "shadows", "blades", "shafts", "halos"];
    pub const HELP: [&'static str; 11] = ["world", "sky", "scene objects", "car bodies", "wheels", "smoke / sparks", "skid marks", "projected shadows", "grass blades (Ultra)", "sun shafts (Volumetric fog)", "halos and lens flares"];

    pub fn get(&mut self, i: usize) -> Option<&mut bool> {
        [&mut self.world, &mut self.sky, &mut self.props, &mut self.bodies, &mut self.wheels, &mut self.sprites, &mut self.skids, &mut self.shadows, &mut self.blades, &mut self.shafts, &mut self.halos].into_iter().nth(i)
    }

    /// `--hide world,sky,...`
    pub fn hide(list: &str) -> Result<Passes, String> {
        let mut p = Passes::default();
        for name in list.split(',').map(str::trim).filter(|s| !s.is_empty()) {
            let i = Self::NAMES.iter().position(|n| *n == name).ok_or(format!("unknown pass {name} ({})", Self::NAMES.join(", ")))?;
            *p.get(i).unwrap() = false;
        }
        Ok(p)
    }

    /// The view with the renderer's own toggles (sky, world) set.
    pub fn view(&self, v: &View) -> View {
        let mut v = *v;
        (v.debug.hide_sky, v.debug.hide_world, v.debug.hide_blades, v.debug.hide_shafts) = (!self.sky, !self.world, !self.blades, !self.shafts);
        v.halos &= self.halos;
        v
    }

    /// The scene inputs of a frame with the hidden passes left out (an empty scene without a session).
    pub fn parts(&self, sess: Option<&Session>, cam: &crate::render::Camera, view: &View) -> FrameParts {
        let Some(s) = sess else { return FrameParts::default() };
        let mut p = s.frame_parts(cam, view.shadows && !view.car_shadows() && self.shadows, view.shadows && self.skids, view.skids_detailed());
        if !self.props {
            p.props.clear();
        }
        p.cars.retain(|&(m, _)| if m % CAR_MESHES == 0 { self.bodies } else { self.wheels });
        if !self.sprites {
            p.sprites.clear();
        }
        p
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Tab {
    Textures,
    Props,
    Cars,
}

impl Tab {
    pub fn parse(s: &str) -> Option<Tab> {
        match s {
            "textures" => Some(Tab::Textures),
            "props" => Some(Tab::Props),
            "cars" => Some(Tab::Cars),
            _ => None,
        }
    }
}

/// Per track: per world texture its layer and the MYW objects using it, per scene-object
/// mesh its instances.
pub struct Catalog {
    pub textures: Vec<(Option<usize>, Vec<usize>)>,
    pub props: Vec<Vec<usize>>,
}

impl Catalog {
    pub fn new(s: &Scene, sess: Option<&Session>) -> Catalog {
        let mut textures = vec![(None, Vec::new()); s.world_textures.len()];
        for (o, (r, _)) in s.world_objects.iter().enumerate() {
            for t in r.clone() {
                let Some(e) = s.world_tri_texture.get(t).and_then(|&i| textures.get_mut(i as usize)) else { continue };
                if e.1.last() != Some(&o) {
                    e.1.push(o);
                }
                if e.0.is_none() {
                    e.0 = s.tris.get(t * 3).filter(|v| v.layer >= 0.0).map(|v| v.layer as usize);
                }
            }
        }
        let mut props = vec![Vec::new(); s.prop_names.len()];
        for (i, p) in sess.map_or(&[][..], |x| x.race.props.props.as_slice()).iter().enumerate() {
            if let Some(v) = props.get_mut(p.mesh) {
                v.push(i);
            }
        }
        Catalog { textures, props }
    }
}

/// Browser rows shown at once.
const ROWS: usize = 14;

/// The data browser (T): one list at a time, a selection, and how often Enter was pressed on
/// it (Enter steps through the objects / instances of the selection).
#[derive(Clone, Copy, Debug, Default)]
pub struct Browser {
    pub tab: Option<Tab>,
    pub sel: usize,
    pub jumps: usize,
}

impl Browser {
    /// T: textures, scene objects, cars, closed.
    pub fn cycle(&mut self) {
        self.tab = match self.tab {
            None => Some(Tab::Textures),
            Some(Tab::Textures) => Some(Tab::Props),
            Some(Tab::Props) => Some(Tab::Cars),
            Some(Tab::Cars) => None,
        };
        (self.sel, self.jumps) = (0, 0);
    }

    pub fn len(&self, s: &Scene) -> usize {
        match self.tab {
            Some(Tab::Textures) => s.world_textures.len(),
            Some(Tab::Props) => s.prop_names.len(),
            Some(Tab::Cars) => s.car_names.len(),
            None => 0,
        }
    }

    /// Move the selection by `d` rows (clamped).
    pub fn step(&mut self, s: &Scene, d: isize) {
        let n = self.len(s);
        self.sel = (self.sel as isize + d).clamp(0, n.saturating_sub(1) as isize) as usize;
        self.jumps = 0;
    }

    /// The texture layer to preview (textures list).
    pub fn preview(&self, cat: &Catalog) -> Option<usize> {
        (self.tab == Some(Tab::Textures)).then(|| cat.textures.get(self.sel).and_then(|t| t.0)).flatten()
    }

    pub fn lines(&self, s: &Scene, cat: &Catalog, sess: Option<&Session>) -> Vec<String> {
        let Some(tab) = self.tab else { return Vec::new() };
        let n = self.len(s);
        let title = match tab {
            Tab::Textures => "world textures",
            Tab::Props => "scene objects",
            Tab::Cars => "cars",
        };
        let mut out = vec![format!("{title} {}/{n}  (T next list, PgUp/PgDn Home/End, Enter jump)", self.sel + 1)];
        let first = self.sel.saturating_sub(ROWS / 2).min(n.saturating_sub(ROWS));
        for i in first..(first + ROWS).min(n) {
            let mark = if i == self.sel { '>' } else { ' ' };
            let row = match tab {
                Tab::Textures => {
                    let (layer, objs) = &cat.textures[i];
                    let layer = layer.map_or("-".into(), |l| l.to_string());
                    format!("{:<20} layer {layer:>3}  {} objects", s.world_textures[i], objs.len())
                }
                Tab::Props => format!("{:<20} {} instances", s.prop_names[i], cat.props.get(i).map_or(0, Vec::len)),
                Tab::Cars => {
                    let driver = sess.and_then(|x| x.race.racers.get(i)).map_or(String::new(), |r| format!("  {}", r.driver.name));
                    format!("{}{driver}", s.car_names[i])
                }
            };
            out.push(format!("{mark}{i:4} {row}"));
        }
        out
    }

    /// Enter: the next object of the selection to look at: (view-space centre, radius, what).
    /// Cars also become the followed car.
    pub fn jump(&mut self, picker: &Picker, cat: &Catalog, sess: Option<&mut Session>) -> Option<(Vec3, f32, String)> {
        let k = self.jumps;
        self.jumps += 1;
        match self.tab? {
            Tab::Textures => {
                let objs = &cat.textures.get(self.sel)?.1;
                let o = *objs.get(k % objs.len().max(1))?;
                let b = picker.world_bounds().nth(o)?;
                Some((b.center, b.radius, format!("object #{o} ({}/{})", k % objs.len() + 1, objs.len())))
            }
            Tab::Props => {
                let inst = cat.props.get(self.sel)?;
                let i = *inst.get(k % inst.len().max(1))?;
                let (mesh, m) = *sess?.prop_models().get(i)?;
                let b = picker.prop_bounds(mesh)?;
                Some((m.transform_point3(b.center), b.radius, format!("instance #{i} ({}/{})", k % inst.len() + 1, inst.len())))
            }
            Tab::Cars => {
                let s = sess?;
                let mesh = self.sel * CAR_MESHES;
                let (_, m): (usize, Mat4) = s.models().into_iter().find(|x| x.0 == mesh)?;
                let b = picker.car_bounds(mesh)?;
                if self.sel < s.race.racers.len() {
                    s.focus = self.sel;
                }
                Some((m.transform_point3(b.center), b.radius, format!("{} (followed)", s.racer_label(self.sel))))
            }
        }
    }
}
