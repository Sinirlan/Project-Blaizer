//! DEBUG / REMASTER World Editor (was the fill editor) (12-world-fill 12.5): edits `remaster/fill/trackNN.fill` in
//! motorview. State, commands (keys and panel buttons both push a `Cmd`), handles, gizmos and the
//! egui panel live here; the mouse and key plumbing is in the `motorview` bin's main.rs.
//! Coordinates are game space (the file's); the picker and camera are view space (`FLIP`).

use crate::debug::Gizmos;
use crate::fill_gizmo::{self, Delta, Mode};
use crate::glass_ui::{self, HintPart};
use crate::icons as ic;
use crate::tune::{section, section_counted};
use crate::procgen::fill::{self, block_rotation, FillFile, Piece, Shape, Species};
use crate::procgen::grass::{self, Body as GrassBody, Params as GrassParams, Style as GrassStyle};
use crate::procgen::water::{Body, Params as WaterParams, Source, Style};
use crate::render::RenderMode;
use crate::scene::Scene;
use crate::gui::wheel_drag;
use egui::{ComboBox, Ui};
use glam::{Quat, Vec3};
use std::borrow::Cow;
use std::path::{Path, PathBuf};

const UNDO_LIMIT: usize = 100;
/// game <-> view space (z negated; its own inverse)
pub const FLIP: Vec3 = Vec3::new(1.0, 1.0, -1.0);
/// Hole finder: shown within this distance of the camera, near the road, edge length range.
const HOLE_RANGE: f32 = 500.0;
const HOLE_ROAD: f32 = 200.0;
const HOLE_LEN: (f32, f32) = (0.5, 100.0);
/// Handle pick radius in pixels.
pub const PICK_PX: f32 = 12.0;
/// cursor travel before a click on a piece becomes a drag, pixels
const DRAG_PX: f32 = 4.0;
const DEFAULT_UV: f32 = 8.0;
/// How far above the clicked ground a new water area starts, metres: at ground height the grass covers it.
const WATER_LIFT: f32 = 0.3;

const SELECTED: [f32; 4] = [1.0, 0.8, 0.1, 1.0];
const OTHER: [f32; 4] = [0.3, 0.9, 0.9, 0.7];
const PENDING: [f32; 4] = [1.0, 0.3, 1.0, 1.0];
const HOLE: [f32; 4] = [1.0, 0.1, 0.1, 1.0];

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Kind {
    Ground,
    Block,
    Wall,
    Hill,
    Patch,
    Skirt,
    /// REMASTER water area (a `Water Area` entry of the file, not a piece; procgen/water.rs)
    Water,
    /// REMASTER trees, bushes and rocks: every click places a plant (or a scatter of them) until Esc
    Plant,
    /// REMASTER local fog (#470): one click places a cloud sitting on the ground
    Cloud,
    /// REMASTER structures (#466): every click places a copy of one of the track's scene objects (airships, billboards,
    /// signs...) until Esc
    Object,
}

impl Kind {
    pub const ALL: [Kind; 10] = [Kind::Ground, Kind::Block, Kind::Wall, Kind::Hill, Kind::Patch, Kind::Skirt, Kind::Water, Kind::Plant, Kind::Cloud, Kind::Object];

    pub fn name(self) -> &'static str {
        ["ground", "block", "wall", "hill", "patch", "skirt", "water", "plant", "cloud", "object"][self as usize]
    }

    /// The click that completes the piece by itself; None: Enter finishes it.
    fn auto_points(self) -> Option<usize> {
        match self {
            Kind::Block | Kind::Plant | Kind::Cloud | Kind::Object => Some(1),
            Kind::Wall => Some(2),
            Kind::Patch => Some(4),
            Kind::Skirt => Some(0),
            _ => None,
        }
    }

    fn min_points(self) -> usize {
        match self {
            Kind::Ground | Kind::Patch | Kind::Water => 3,
            Kind::Hill | Kind::Wall => 2,
            _ => 1,
        }
    }
}

fn kind_of(p: &Piece) -> Kind {
    match p.shape {
        Shape::Ground { .. } => Kind::Ground,
        Shape::Block { .. } => Kind::Block,
        Shape::Wall { .. } => Kind::Wall,
        Shape::Hill { .. } => Kind::Hill,
        Shape::Patch { .. } => Kind::Patch,
        Shape::Skirt { .. } => Kind::Skirt,
        Shape::Plant { .. } | Shape::Model { .. } => Kind::Plant,
        Shape::Cloud { .. } => Kind::Cloud,
        Shape::Object { .. } | Shape::Stamp { .. } => Kind::Object,
    }
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Cmd {
    Save,
    Reload,
    Undo,
    Redo,
    Delete,
    Duplicate,
    Generate,
    Holes,
    /// select the previous / next piece
    Step(isize),
    Start(Kind),
    Finish,
    Back,
    /// the transform gizmo to show (12.8)
    Gizmo(Mode),
    /// make the selected ground or patch flat at its mean height
    Level,
}

/// A gizmo drag: the piece as it was, the pivot, and the axis parameter or angle where it was grabbed.
/// Where a gizmo sits and which of its axes work (`WorldEdit::gizmo_target`).
struct GizmoTarget {
    pivot: Vec3,
    enabled: [bool; 3],
    axes: [Vec3; 3],
    vertex: Option<usize>,
}

/// A whole-piece drag started by a click on the piece in the viewport (12.11): the piece as it was,
/// the world point grabbed and the cursor where the button went down (a drag starts past `DRAG_PX`).
struct PieceDrag {
    index: usize,
    base: Piece,
    grab: Vec3,
    press: (f32, f32),
    moved: bool,
}

struct GizmoDrag {
    axis: usize,
    base: Piece,
    pivot: Vec3,
    start: f32,
    vertex: Option<usize>,
    /// a hill's meshed ridge heights at the grab (`fill_gizmo::apply_crest`)
    crest: Vec<f32>,
    /// the axis directions and gizmo length when it was grabbed (scaling turns with the piece)
    axes: [Vec3; 3],
    len: f32,
}

/// The plant brush: one click places `count` plants of `species` within `radius` of the cursor.
#[derive(Clone, Debug)]
pub struct Brush {
    pub species: Species,
    pub count: usize,
    pub radius: f32,
    pub scale: f32,
    /// foliage / stone texture and trunk texture; empty = automatic: the original data's picture for
    /// the species on this track (`fill::plant_textures`), else the last piece's
    pub texture: String,
    pub trunk: String,
    /// pack models (`trees::names`) and families (`trees::families`, a random member per plant) to place
    /// instead of a generated plant: each plant picks one entry at random; empty = the species (#390)
    pub models: Vec<String>,
    /// paint mode: hold the button and drag to plant along the mouse path (else one scatter per click)
    pub paint: bool,
    /// paint mode: metres between two stamps along the path
    pub spacing: f32,
    /// each placed plant or model gets a random yaw (else 0°)
    pub random_yaw: bool,
}

impl Default for Brush {
    fn default() -> Self {
        Brush { species: Species::Tree, count: 1, radius: 25.0, scale: 1.0, texture: String::new(), trunk: String::new(), models: Vec::new(), paint: false, spacing: 8.0, random_yaw: true }
    }
}

/// The pack models the brush's ticked entries stand for, one group per known entry: a family (`Tree-01`: any of
/// its members) or a single model. Unknown entries are dropped; None when no group is left (the generated plants).
fn brush_groups(models: &[String]) -> Option<Vec<Vec<&'static str>>> {
    let fam = crate::procgen::trees::families();
    let names = crate::procgen::trees::names();
    let groups: Vec<Vec<&'static str>> = models
        .iter()
        .filter_map(|model| {
            if let Some((_, m)) = fam.iter().find(|(f, _)| f == model) {
                return Some(m.clone());
            }
            names.iter().find(|n| *n == model).map(|n| vec![*n])
        })
        .collect();
    (!groups.is_empty()).then_some(groups)
}

/// The ground height under a game-space point (None: nothing there).
pub type Ground<'a> = dyn Fn(Vec3) -> Option<f32> + 'a;

/// A paint stroke longer than this many spacings in one mouse move is a jump (the cursor crossed
/// the sky or far terrain): the stroke restarts there instead of filling the gap.
const STROKE_JUMP: f32 = 50.0;

/// Where the stamps go when a stroke that last planted at `last` reaches `at`: every `spacing`
/// metres on the xz line, heights interpolated; the last element is the new `last`.
/// A jump (> `STROKE_JUMP` spacings) is one stamp at `at`.
fn stroke_points(last: Vec3, at: Vec3, spacing: f32) -> Vec<Vec3> {
    let spacing = spacing.max(0.5);
    let d = Vec3::new(at.x - last.x, 0.0, at.z - last.z);
    let len = d.length();
    if len > spacing * STROKE_JUMP {
        return vec![at];
    }
    let n = (len / spacing).floor() as usize;
    (1..=n).map(|k| {
        let t = k as f32 * spacing / len;
        last + (at - last) * t
    }).collect()
}

#[derive(Default)]
pub struct WorldEdit {
    pub open: bool,
    /// view mode and distance to restore on close
    pub prev: Option<(RenderMode, f32)>,
    pub file: FillFile,
    saved: FillFile,
    undo: Vec<FillFile>,
    redo: Vec<FillFile>,
    pub sel: Option<usize>,
    /// piece being drawn and the points clicked so far (game space)
    drawing: Option<(Kind, Vec<Vec3>)>,
    /// dragged handle of the selected piece
    pub drag: Option<usize>,
    /// transform gizmo shown on the selected piece (12.8)
    pub mode: Mode,
    /// the vertex last clicked: (piece, handle); the move arrows act on it alone
    sel_handle: Option<(usize, usize)>,
    /// hovered or dragged gizmo axis
    pub hot: Option<usize>,
    gdrag: Option<GizmoDrag>,
    /// whole-piece drag from a viewport click (12.11)
    pdrag: Option<PieceDrag>,
    /// the skirt's centre and the world's lowest y, game space, from the last fill mesh (`fill::mesh_pieces`);
    /// the skirt gizmo needs them
    pub skirt_base: Option<(Vec3, f32)>,
    /// per hill piece (index into the pieces): the ridge height at each point, pinned or draped, from the
    /// last fill mesh (`FillMesh::hills`); the handles of a draped point sit there
    pub hill_crest: Vec<(usize, Vec<f32>)>,
    /// height shown for handles without one, set to the last click's
    pub work_y: f32,
    pub holes: bool,
    /// open world edges near the road, game space; None until first wanted
    edges: Option<Vec<[Vec3; 2]>>,
    /// the file changed: re-mesh and upload
    pub dirty_mesh: bool,
    pub cmds: Vec<Cmd>,
    pub note: String,
    root: PathBuf,
    track: u32,
    confirm_gen: bool,
    /// selected water body and the texture the next "mark texture" uses
    water_sel: Option<usize>,
    /// the corner of a water area last clicked: (body, corner); the move arrows act on it alone
    water_corner: Option<(usize, usize)>,
    water_tex: String,
    /// "split piece" armed: the texture the next click's connected piece becomes a body of
    water_split: Option<String>,
    /// selected grass body and the texture the next "mark as grass" / "add texture" uses
    grass_sel: Option<usize>,
    grass_tex: String,
    /// plant brush: species, plants per click, scatter radius, scale (REMASTER, Kind::Plant)
    pub brush: Brush,
    /// the paint stroke in progress (button held): where it last planted
    stroke: Option<Vec3>,
    /// plants of the stroke so far
    stroke_plants: usize,
    /// panel edit in progress (pointer held): the state before it, pushed to `undo` on release
    edit_base: Option<FillFile>,
    /// layer toggles of the ribbon's View group, indexed by `Kind` (not saved; Water has no outline)
    pub hidden: [bool; 10],
    /// pieces hidden by the outliner's eye (not saved); dropped when the piece count changes
    hidden_pieces: Vec<usize>,
    hidden_len: usize,
    /// outliner filter text
    pub filter: String,
    /// outliner double-click: game-space centre and size to frame the camera on
    pub focus: Option<(Vec3, f32)>,
    /// the world point under the mouse (game space), for the plant brush ring; set by the viewer
    pub cursor_ground: Option<Vec3>,
    /// the track's placeable scene objects (`fill::objects`, set by the viewer when the scene loads) and what the
    /// structure tool places (unknown: the first object, else the first stamp)
    pub objects: Vec<fill::ObjectInfo>,
    pub structure: Structure,
    /// the stamp whose source box is selected (#466; only while no piece is): the gizmos and handle act on its box
    stamp_sel: Option<usize>,
    /// the structure tool's next click adds a stamp box there instead of a copy ("New stamp")
    pub capture: bool,
    /// the box fitted round the building under the capture click (#475, `capture_fit`), taken by the click
    capture_fit: Option<fill::StampFit>,
    /// the selected stamp as last cut (`refresh_capture`): the box it was cut with, its triangles' edges (game space,
    /// capped at `CAPTURE_EDGES`) and its triangle count
    capture_edges: Option<(fill::Stamp, Vec<[Vec3; 2]>, usize)>,
}

/// What the structure tool places (#466): a scene object of the track or a copy of a stamp of the file, by name.
#[derive(Clone, Debug, PartialEq)]
pub enum Structure {
    Object(String),
    Stamp(String),
}

impl Default for Structure {
    fn default() -> Self {
        Structure::Object(String::new())
    }
}

/// The world triangles the Remaster world draw leaves out (water the surface replaces, tree cards the models replace):
/// stamps neither cut nor fit them.
fn hidden_by_remaster(scene: &Scene) -> impl Fn(usize) -> bool + '_ {
    move |i| scene.water_hide.get(i).copied().unwrap_or(false) || scene.tree_hide.get(i).copied().unwrap_or(false)
}

/// A new stamp box's size (w h d, m) and floor (m) [untuned, design choice]
const STAMP_SIZE: [f32; 3] = [20.0, 30.0, 20.0];
const STAMP_FLOOR: f32 = 0.5;
/// the most triangle edges the selected stamp's highlight draws
const CAPTURE_EDGES: usize = 20_000;
const CAPTURED: [f32; 4] = [0.2, 1.0, 0.4, 0.8];
/// a stamp's copies in the structure list, after the scene objects
const STAMP_MARK: &str = "stamp ";

/// Handle positions (game space) of a piece; `y` for points without a height.
pub fn handles(p: &Piece, y: f32) -> Vec<Vec3> {
    match &p.shape {
        Shape::Ground { points } => points.iter().map(|&([x, z], h)| Vec3::new(x, h.unwrap_or(y), z)).collect(),
        Shape::Block { centre, .. } | Shape::Plant { centre, .. } | Shape::Model { centre, .. } | Shape::Object { centre, .. } | Shape::Stamp { centre, .. } | Shape::Cloud { centre, .. } => vec![Vec3::from(*centre)],
        Shape::Wall { from, to, .. } => vec![Vec3::from(*from), Vec3::from(*to)],
        // a draped ridge point has no height of its own: `y` + the hill's height (APPROX; the editor passes the meshed ones, see `WorldEdit::baked`)
        Shape::Hill { points, height, .. } => points.iter().map(|&([x, z], h)| Vec3::new(x, h.unwrap_or(y + height), z)).collect(),
        Shape::Patch { verts } => verts.iter().map(|&v| Vec3::from(v)).collect(),
        Shape::Skirt { .. } => Vec::new(),
    }
}

pub fn set_handle(p: &mut Piece, i: usize, to: Vec3) {
    match &mut p.shape {
        Shape::Ground { points } => {
            if let Some(q) = points.get_mut(i) {
                q.0 = [to.x, to.z];
                if q.1.is_some() {
                    q.1 = Some(to.y);
                }
            }
        }
        Shape::Block { centre, .. } | Shape::Plant { centre, .. } | Shape::Model { centre, .. } | Shape::Object { centre, .. } | Shape::Stamp { centre, .. } | Shape::Cloud { centre, .. } if i == 0 => *centre = to.into(),
        Shape::Wall { from, .. } if i == 0 => *from = to.into(),
        Shape::Wall { to: t, .. } if i == 1 => *t = to.into(),
        Shape::Hill { points, .. } => {
            if let Some(q) = points.get_mut(i) {
                q.0 = [to.x, to.z];
                if q.1.is_some() {
                    q.1 = Some(to.y);
                }
            }
        }
        Shape::Patch { verts } => {
            if let Some(q) = verts.get_mut(i) {
                *q = to.into();
            }
        }
        _ => {}
    }
}

/// Make a ground or patch flat at the mean height of its points (draped ground points count at
/// the world's height under them); the new height, None for other pieces.
fn level(p: &mut Piece, ground_at: &dyn Fn(&[[f32; 2]]) -> Vec<f32>) -> Option<f32> {
    match &mut p.shape {
        Shape::Ground { points } => {
            let xz: Vec<[f32; 2]> = points.iter().map(|q| q.0).collect();
            let ground = ground_at(&xz);
            let y = points.iter().zip(&ground).map(|(q, g)| q.1.unwrap_or(*g)).sum::<f32>() / points.len().max(1) as f32;
            points.iter_mut().for_each(|q| q.1 = Some(y));
            Some(y)
        }
        Shape::Patch { verts } => {
            let y = verts.iter().map(|v| v[1]).sum::<f32>() / verts.len().max(1) as f32;
            verts.iter_mut().for_each(|v| v[1] = y);
            Some(y)
        }
        _ => None,
    }
}

/// Lines (game space) showing a piece: (points, closed).
fn outline(p: &Piece, y: f32, objects: &[fill::ObjectInfo], stamps: &[fill::Stamp]) -> Vec<(Vec<Vec3>, bool)> {
    match &p.shape {
        Shape::Ground { .. } | Shape::Patch { .. } => vec![(handles(p, y), true)],
        Shape::Hill { .. } => vec![(handles(p, y), false)],
        Shape::Wall { from, to, height } => {
            let (a, b) = (Vec3::from(*from), Vec3::from(*to));
            let up = Vec3::Y * height;
            vec![(vec![a, b, b + up, a + up], true)]
        }
        Shape::Block { centre, size, yaw, pitch, roll, .. } => {
            let rot = block_rotation(*yaw, *pitch, *roll);
            let c = Vec3::from(*centre);
            let corner = |sx: f32, sz: f32, h: f32| c + rot * Vec3::new(sx * size[0] / 2.0, h, sz * size[2] / 2.0);
            let ring = [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)];
            let mut out = vec![(ring.map(|(x, z)| corner(x, z, 0.0)).to_vec(), true), (ring.map(|(x, z)| corner(x, z, size[1])).to_vec(), true)];
            out.extend(ring.map(|(x, z)| (vec![corner(x, z, 0.0), corner(x, z, size[1])], false)));
            out
        }
        Shape::Plant { centre, scale, .. } => {
            // a cross-section of the plant's height and a ring on the ground
            let c = Vec3::from(*centre);
            let ring = (0..12).map(|k| c + Vec3::new((k as f32 / 12.0 * std::f32::consts::TAU).cos(), 0.0, (k as f32 / 12.0 * std::f32::consts::TAU).sin()) * 2.0 * scale).collect();
            vec![(ring, true), (vec![c, c + Vec3::Y * 10.0 * scale], false)]
        }
        Shape::Model { name, centre, scale, .. } => {
            // a ring on the ground and the model's own height
            let (c, h) = (Vec3::from(*centre), crate::procgen::trees::height(name).unwrap_or(5.0) * scale);
            let ring = (0..12).map(|k| c + Vec3::new((k as f32 / 12.0 * std::f32::consts::TAU).cos(), 0.0, (k as f32 / 12.0 * std::f32::consts::TAU).sin()) * 0.25 * h).collect();
            vec![(ring, true), (vec![c, c + Vec3::Y * h], false)]
        }
        Shape::Skirt { .. } => Vec::new(),
        Shape::Object { name, centre, scale, yaw } => object_box(name, Vec3::from(*centre), *scale, *yaw, objects),
        Shape::Stamp { name, centre, scale, yaw } => {
            let c = Vec3::from(*centre);
            match stamps.iter().find(|st| st.name.eq_ignore_ascii_case(name)) {
                Some(st) => box_edges(c, stamp_lo(st), stamp_hi(st), *scale, *yaw),
                None => ring_mast(c, *scale, *yaw),
            }
        }
        Shape::Cloud { centre, size, yaw, .. } => cloud_rings(Vec3::from(*centre), Vec3::from(*size) * 0.5, *yaw).into_iter().map(|r| (r, true)).collect(),
    }
}

/// A scene object's box (game space): its listed extents scaled by `scale` and turned `yaw` about its base `c` like a
/// block, as the 12 edges; a ring and a mast stand in for an object this track does not have.
fn object_box(name: &str, c: Vec3, scale: f32, yaw: f32, objects: &[fill::ObjectInfo]) -> Vec<(Vec<Vec3>, bool)> {
    match objects.iter().find(|o| o.name.eq_ignore_ascii_case(name)) {
        Some(o) => box_edges(c, o.lo, o.hi, scale, yaw),
        None => ring_mast(c, scale, yaw),
    }
}

/// What stands in for a structure the track or file lacks: a ring and a mast on its base `c`.
fn ring_mast(c: Vec3, scale: f32, yaw: f32) -> Vec<(Vec<Vec3>, bool)> {
    let rot = block_rotation(yaw, 0.0, 0.0);
    let ring = (0..12).map(|k| c + rot * Vec3::new((k as f32 / 12.0 * std::f32::consts::TAU).cos(), 0.0, (k as f32 / 12.0 * std::f32::consts::TAU).sin()) * 2.0 * scale).collect();
    vec![(ring, true), (vec![c, c + Vec3::Y * 10.0 * scale], false)]
}

/// A stamp's box in its own frame (game axes, base at 0): x and z centred, y up from the base.
fn stamp_lo(st: &fill::Stamp) -> Vec3 {
    Vec3::new(-st.size[0] * 0.5, 0.0, -st.size[2] * 0.5)
}

fn stamp_hi(st: &fill::Stamp) -> Vec3 {
    Vec3::new(st.size[0] * 0.5, st.size[1], st.size[2] * 0.5)
}

/// The 12 edges (game space) of the box `lo`..`hi` (its own frame) scaled by `scale` and turned `yaw` about `c`, as a
/// bottom ring, a top ring and four uprights.
fn box_edges(c: Vec3, lo: Vec3, hi: Vec3, scale: f32, yaw: f32) -> Vec<(Vec<Vec3>, bool)> {
    let rot = block_rotation(yaw, 0.0, 0.0);
    let corner = |sx: bool, y: bool, sz: bool| c + rot * (Vec3::new(if sx { hi.x } else { lo.x }, if y { hi.y } else { lo.y }, if sz { hi.z } else { lo.z }) * scale);
    let ring = |y: bool| vec![corner(false, y, false), corner(true, y, false), corner(true, y, true), corner(false, y, true)];
    let mut out = vec![(ring(false), true), (ring(true), true)];
    out.extend([(false, false), (true, false), (true, true), (false, true)].map(|(x, z)| (vec![corner(x, false, z), corner(x, true, z)], false)));
    out
}

/// A cloud's outline (game space): its three axis ellipses, turned by `yaw` like a block.
pub fn cloud_rings(c: Vec3, half: Vec3, yaw: f32) -> [Vec<Vec3>; 3] {
    let rot = block_rotation(yaw, 0.0, 0.0);
    let ring = |u: Vec3, v: Vec3| (0..32).map(|k| {
        let a = k as f32 / 32.0 * std::f32::consts::TAU;
        c + rot * (u * a.cos() + v * a.sin())
    }).collect();
    [ring(Vec3::X * half.x, Vec3::Z * half.z), ring(Vec3::X * half.x, Vec3::Y * half.y), ring(Vec3::Z * half.z, Vec3::Y * half.y)]
}

impl WorldEdit {
    /// Start editing `track`'s file (kept as is when it is already the open track).
    pub fn begin(&mut self, root: &Path, track: u32) {
        if !self.open || self.track != track {
            self.file = fill::load(root, track);
            self.saved = self.file.clone();
            (self.undo, self.redo, self.sel, self.drawing, self.drag, self.edit_base, self.sel_handle, self.water_corner, self.gdrag, self.pdrag) = Default::default();
        }
        (self.root, self.track, self.open, self.dirty_mesh) = (root.to_path_buf(), track, true, true);
        self.stroke = None;
        self.edges = None;
        self.work_y = 0.0;
    }

    /// The track's scene objects for the structure tool and the objects' outlines (`fill::objects`); the viewer calls it when
    /// the editor opens and when a scene has loaded.
    pub fn set_objects(&mut self, scene: &Scene) {
        self.objects = fill::objects(scene);
    }

    /// The scene was reloaded: drop what was computed from it.
    pub fn scene_changed(&mut self) {
        self.edges = None;
        self.dirty_mesh = true;
    }

    pub fn unsaved(&self) -> bool {
        self.file != self.saved
    }

    pub fn drawing(&self) -> bool {
        self.drawing.is_some()
    }

    /// The place tool that is armed (palette highlight), and the points clicked so far.
    pub fn armed(&self) -> Option<Kind> {
        self.drawing.as_ref().map(|d| d.0)
    }

    pub fn points(&self) -> usize {
        self.drawing.as_ref().map_or(0, |d| d.1.len())
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    /// Piece `i` is hidden by its layer or its eye (false for an index that is not in the file).
    pub fn piece_hidden(&self, i: usize) -> bool {
        self.file.pieces.get(i).is_some_and(|p| self.is_hidden(i, p))
    }

    /// The piece is hidden by its layer or its eye.
    fn is_hidden(&self, i: usize, p: &Piece) -> bool {
        self.hidden[kind_of(p) as usize] || (self.hidden_len == self.file.pieces.len() && self.hidden_pieces.contains(&i))
    }

    /// Game-space centre and radius of a piece (the camera frames it).
    fn extent(&self, p: &Piece) -> (Vec3, f32) {
        let hs = handles(p, self.work_y);
        if hs.is_empty() {
            return (Vec3::ZERO, 20.0);
        }
        let c = hs.iter().copied().sum::<Vec3>() / hs.len() as f32;
        (c, hs.iter().map(|h| h.distance(c)).fold(0.0, f32::max).max(6.0))
    }

    /// Esc while drawing cancels the piece; false when there was nothing to cancel.
    pub fn cancel(&mut self) -> bool {
        self.water_split = None;
        self.stroke = None;
        self.capture = false;
        self.drawing.take().is_some()
    }

    fn path(&self) -> PathBuf {
        self.root.join(format!("remaster/fill/track{:02}.fill", self.track))
    }

    /// Remember the state before a change (rimshop's undo pattern).
    fn commit(&mut self) {
        self.undo.push(self.file.clone());
        if self.undo.len() > UNDO_LIMIT {
            self.undo.remove(0);
        }
        self.redo.clear();
        self.dirty_mesh = true;
    }

    /// A paint stroke is being dragged (button held).
    pub fn painting(&self) -> bool {
        self.stroke.is_some()
    }

    /// Start a drag of handle `i` (one undo step for the whole drag).
    pub fn begin_drag(&mut self, i: usize) {
        self.commit();
        self.drag = Some(i);
        self.sel_handle = self.sel.map(|s| (s, i));
        // no piece selected: the handles are a water area's corners
        self.water_corner = self.sel.is_none().then_some(self.water_sel).flatten().map(|w| (w, i));
    }

    /// A click that hit nothing: no vertex stays selected.
    pub fn clear_vertex(&mut self) {
        (self.sel_handle, self.water_corner) = (None, None);
    }

    /// The clicked corner of the selected water area (none if it was removed or another body is selected).
    fn water_vertex(&self) -> Option<usize> {
        let (w, i) = self.water_corner?;
        (self.water_sel == Some(w) && i < self.water_points().len()).then_some(i)
    }

    /// The meshed ridge heights of hill piece `s`, while they still fit its points.
    fn crest_of(&self, s: usize) -> Option<&[f32]> {
        let Shape::Hill { points, .. } = &self.file.pieces.get(s)?.shape else { return None };
        self.hill_crest.iter().find(|(i, _)| *i == s).map(|(_, c)| c.as_slice()).filter(|c| c.len() == points.len())
    }

    /// Piece `s` as its handles show it: a hill's draped ridge points pinned at their meshed heights
    /// (a pinned point keeps its own); any other piece as it is.
    fn baked<'a>(&self, s: usize, p: &'a Piece) -> Cow<'a, Piece> {
        let (Some(crest), Shape::Hill { .. }) = (self.crest_of(s), &p.shape) else { return Cow::Borrowed(p) };
        let mut q = p.clone();
        if let Shape::Hill { points, .. } = &mut q.shape {
            for (pt, &c) in points.iter_mut().zip(crest) {
                pt.1.get_or_insert(c);
            }
        }
        Cow::Owned(q)
    }

    /// The selected vertex of the selected piece, when the piece has more than one (else the
    /// arrows move the whole piece anyway).
    fn vertex(&self) -> Option<usize> {
        let (s, i) = self.sel_handle?;
        let n = self.file.pieces.get(s).map_or(0, |p| handles(p, self.work_y).len());
        (self.sel == Some(s) && n > 1 && i < n).then_some(i)
    }

    /// Where the gizmo sits and which axes work: the selected vertex for moves, else the piece's centre.
    fn gizmo_target(&self) -> Option<GizmoTarget> {
        // a selected stamp box: the gizmos work on it as a block that turns about Y only
        if let Some(p) = self.stamp_piece() {
            let mut en = fill_gizmo::enabled(&p, self.mode);
            if self.mode == Mode::Rotate {
                en = [false, true, false];
            }
            return en.contains(&true).then(|| GizmoTarget { pivot: fill_gizmo::pivot(&p, 0.0), enabled: en, axes: fill_gizmo::axes(&p, self.mode), vertex: None });
        }
        // a selected water area: the gizmo works on its outline as a flat patch
        if let Some(p) = self.water_piece() {
            let en = match self.mode {
                Mode::Handles => [false; 3],
                Mode::Move => [true; 3],
                // the surface stays level: turn about Y, stretch along X and Z
                Mode::Rotate => [false, true, false],
                Mode::Scale => [true, false, true],
            };
            // a clicked corner: the arrows move it alone, along X and Z (the surface stays level)
            if let (Mode::Move, Some(i)) = (self.mode, self.water_vertex()) {
                return Some(GizmoTarget { pivot: self.water_points()[i], enabled: [true, false, true], axes: fill_gizmo::AXES, vertex: Some(i) });
            }
            return en.contains(&true).then(|| GizmoTarget { pivot: fill_gizmo::pivot(&p, 0.0), enabled: en, axes: fill_gizmo::AXES, vertex: None });
        }
        let (s, p) = self.sel.and_then(|s| self.file.pieces.get(s).map(|p| (s, p))).filter(|&(s, p)| !self.is_hidden(s, p))?;
        let en = fill_gizmo::enabled(p, self.mode);
        let baked = self.baked(s, p);
        let p = &*baked;
        if !en.contains(&true) {
            return None;
        }
        let vertex = self.vertex().filter(|_| self.mode == Mode::Move);
        let pivot = vertex.and_then(|i| handles(p, self.work_y).get(i).copied()).unwrap_or_else(|| fill_gizmo::pivot(p, self.work_y));
        Some(GizmoTarget { pivot, enabled: en, axes: fill_gizmo::axes(p, self.mode), vertex })
    }

    pub fn gizmo_dragging(&self) -> bool {
        self.gdrag.is_some()
    }

    /// Highlight the axis under the cursor (not while dragging).
    pub fn gizmo_hover(&mut self, eye: Vec3, project: &dyn Fn(Vec3) -> Option<(f32, f32)>, cursor: (f32, f32)) {
        if self.gdrag.is_some() {
            return;
        }
        if let Some((c, r, _, floor)) = self.skirt_target() {
            self.hot = fill_gizmo::skirt_pick(c, r, floor, eye, project, cursor);
            return;
        }
        self.hot = self.gizmo_target().and_then(|t| fill_gizmo::pick(self.mode, t.pivot, t.enabled, t.axes, eye, project, cursor));
    }

    /// LMB: grab the gizmo axis under the cursor; false when there is none.
    pub fn gizmo_press(&mut self, eye: Vec3, project: &dyn Fn(Vec3) -> Option<(f32, f32)>, cursor: (f32, f32), ray: (Vec3, Vec3)) -> bool {
        if self.drawing.is_some() {
            return false;
        }
        if let Some((c, r, _, floor)) = self.skirt_target() {
            return self.skirt_press(c, r, floor, eye, project, cursor, ray);
        }
        let Some(GizmoTarget { pivot, enabled: en, axes, vertex }) = self.gizmo_target() else { return false };
        let Some(axis) = fill_gizmo::pick(self.mode, pivot, en, axes, eye, project, cursor) else { return false };
        let start = match self.mode {
            Mode::Move | Mode::Scale => fill_gizmo::axis_param(pivot, axes[axis], ray),
            _ => fill_gizmo::plane_angle(pivot, axes, axis, ray),
        };
        let (Some(start), Some(base)) = (start, self.sel.and_then(|s| self.file.pieces.get(s)).cloned().or_else(|| self.stamp_piece()).or_else(|| self.water_piece())) else { return false };
        self.commit();
        self.hot = Some(axis);
        let crest = self.sel.and_then(|s| self.crest_of(s)).map(<[f32]>::to_vec).unwrap_or_default();
        self.gdrag = Some(GizmoDrag { axis, base, pivot, start, vertex, crest, axes, len: fill_gizmo::length(pivot, eye) });
        true
    }

    /// The mouse moved with a gizmo axis held: the piece is the start state plus the change.
    pub fn gizmo_drag(&mut self, ray: (Vec3, Vec3)) {
        if self.skirt_drag(ray) {
            return;
        }
        let Some(d) = &self.gdrag else { return };
        let delta = match self.mode {
            Mode::Move => fill_gizmo::axis_param(d.pivot, d.axes[d.axis], ray).map(|t| Delta::Move(d.axes[d.axis] * (t - d.start))),
            // one gizmo length of drag doubles the size
            Mode::Scale => fill_gizmo::axis_param(d.pivot, d.axes[d.axis], ray).map(|t| Delta::Scale { axis: d.axis, dir: d.axes[d.axis], factor: 1.0 + (t - d.start) / d.len }),
            _ => fill_gizmo::plane_angle(d.pivot, d.axes, d.axis, ray).map(|a| Delta::Rotate(Quat::from_axis_angle(fill_gizmo::AXES[d.axis], fill_gizmo::angle_diff(d.start, a)))),
        };
        let Some(delta) = delta else { return };
        let piece = fill_gizmo::apply_crest(&d.base, delta, d.vertex, self.work_y, &d.crest);
        if self.stamp_active().is_some() {
            self.set_stamp_piece(&piece);
            self.dirty_mesh = true;
        } else if self.sel.is_none() {
            self.set_water_piece(&piece);
            self.dirty_mesh = true;
        } else if let Some(slot) = self.sel.and_then(|s| self.file.pieces.get_mut(s)) {
            *slot = piece;
            self.dirty_mesh = true;
        }
    }

    pub fn gizmo_release(&mut self) {
        (self.gdrag, self.pdrag) = (None, None);
    }

    /// The selected skirt's gizmo: (centre, radius, drop, floor y), game space. It is the skirt's only
    /// control, so it shows in every gizmo mode.
    fn skirt_target(&self) -> Option<(Vec3, f32, f32, f32)> {
        let (c, min_y) = self.skirt_base?;
        let s = self.sel?;
        let p = self.file.pieces.get(s).filter(|p| !self.is_hidden(s, p))?;
        let Shape::Skirt { radius, drop } = p.shape else { return None };
        Some((c, radius, drop, min_y - drop))
    }

    /// LMB near the skirt's ring (radius) or arrow (drop).
    #[allow(clippy::too_many_arguments)]
    fn skirt_press(&mut self, c: Vec3, r: f32, floor: f32, eye: Vec3, project: &dyn Fn(Vec3) -> Option<(f32, f32)>, cursor: (f32, f32), ray: (Vec3, Vec3)) -> bool {
        let Some(axis) = fill_gizmo::skirt_pick(c, r, floor, eye, project, cursor) else { return false };
        let ring = Vec3::new(c.x, floor, c.z);
        let foot = fill_gizmo::skirt_arrow(c, r, floor, eye);
        let (pivot, start) = if axis == 0 { (ring, fill_gizmo::ring_param(ring, floor, ray)) } else { (foot, fill_gizmo::axis_param(foot, Vec3::Y, ray)) };
        let (Some(start), Some(base)) = (start, self.sel.and_then(|s| self.file.pieces.get(s)).cloned()) else { return false };
        self.commit();
        self.hot = Some(axis);
        self.gdrag = Some(GizmoDrag { axis, base, pivot, start, vertex: None, crest: Vec::new(), axes: fill_gizmo::AXES, len: 1.0 });
        true
    }

    /// A held skirt ring or arrow follows the mouse: radius grows with the ring, the drop shrinks as the
    /// arrow rises. False when no skirt is being dragged.
    fn skirt_drag(&mut self, ray: (Vec3, Vec3)) -> bool {
        let Some(d) = &self.gdrag else { return false };
        let Shape::Skirt { radius, drop } = d.base.shape else { return false };
        let shape = if d.axis == 0 {
            fill_gizmo::ring_param(d.pivot, d.pivot.y, ray).map(|r| Shape::Skirt { radius: (radius + r - d.start).max(10.0), drop })
        } else {
            fill_gizmo::axis_param(d.pivot, Vec3::Y, ray).map(|t| Shape::Skirt { radius, drop: (drop - (t - d.start)).max(0.0) })
        };
        if let (Some(shape), Some(slot)) = (shape, self.sel.and_then(|s| self.file.pieces.get_mut(s))) {
            slot.shape = shape;
            self.dirty_mesh = true;
        }
        true
    }

    /// Per piece: true when it is not ground for placing things on (hidden, a plant, the skirt, or
    /// `except`, the piece being dragged). Index = `FillFile::pieces`.
    pub fn ground_skip(&self, except: Option<usize>) -> Vec<bool> {
        self.file.pieces.iter().enumerate().map(|(i, p)| Some(i) == except || self.is_hidden(i, p) || matches!(p.shape, Shape::Plant { .. } | Shape::Model { .. } | Shape::Object { .. } | Shape::Stamp { .. } | Shape::Skirt { .. })).collect()
    }

    /// The piece held by a viewport drag.
    pub fn dragged(&self) -> Option<usize> {
        self.pdrag.as_ref().map(|d| d.index)
    }

    /// The piece can be clicked in the viewport (it exists and is not hidden).
    pub fn pickable(&self, i: usize) -> bool {
        self.file.pieces.get(i).is_some_and(|p| !self.is_hidden(i, p))
    }

    /// LMB on piece `i` in the viewport at world point `grab` (game space), cursor at `cursor`: select
    /// it and get ready to drag it. Nothing changes until the cursor travels (a click is no undo step).
    pub fn piece_press(&mut self, i: usize, grab: Vec3, cursor: (f32, f32)) {
        let Some(p) = self.file.pieces.get(i).cloned() else { return };
        (self.sel, self.water_sel, self.stamp_sel, self.sel_handle, self.water_corner, self.hot) = (Some(i), None, None, None, None, None);
        // a skirt has no position: selecting it is all a click does
        self.pdrag = (!matches!(p.shape, Shape::Skirt { .. })).then_some(PieceDrag { index: i, base: p, grab, press: cursor, moved: false });
    }

    /// The centres (game space) of the shown clouds that are not selected (#470): a cloud has no surface to
    /// click (a big one over the ground would take every click), so its centre marker selects it.
    pub fn cloud_markers(&self) -> Vec<(usize, Vec3)> {
        self.file
            .pieces
            .iter()
            .enumerate()
            .filter(|&(i, p)| self.sel != Some(i) && !self.is_hidden(i, p))
            .filter_map(|(i, p)| match p.shape {
                Shape::Cloud { centre, .. } => Some((i, Vec3::from(centre))),
                _ => None,
            })
            .collect()
    }

    pub fn piece_dragging(&self) -> bool {
        self.pdrag.is_some()
    }

    /// The mouse moved with a piece held: it slides on the horizontal plane through the grab point, the
    /// start state plus the change. Blocks and plants (one point) then drop onto the ground under them
    /// (`ground` takes a game-space point: the ground height there) [design choice]; other pieces
    /// keep their shape and heights. One undo step for the whole drag.
    pub fn piece_drag(&mut self, ray: (Vec3, Vec3), cursor: (f32, f32), ground: &Ground) {
        let Some(d) = &mut self.pdrag else { return };
        if !d.moved && (cursor.0 - d.press.0).hypot(cursor.1 - d.press.1) < DRAG_PX {
            return;
        }
        let first = !d.moved;
        d.moved = true;
        let (index, grab) = (d.index, d.grab);
        let (o, dir) = ray;
        if dir.y.abs() < 1e-4 {
            return;
        }
        let t = (grab.y - o.y) / dir.y;
        if t <= 0.0 {
            return;
        }
        let delta = (o + dir * t - grab).with_y(0.0);
        let mut piece = fill_gizmo::apply(&d.base, Delta::Move(delta), None, self.work_y);
        if let Shape::Block { centre, .. } | Shape::Plant { centre, .. } | Shape::Model { centre, .. } | Shape::Object { centre, .. } | Shape::Stamp { centre, .. } = &mut piece.shape {
            if let Some(y) = ground(Vec3::from(*centre)) {
                centre[1] = y;
            }
        }
        if first {
            self.commit();
        }
        if let Some(slot) = self.file.pieces.get_mut(index) {
            *slot = piece;
            self.dirty_mesh = true;
        }
    }

    /// The selected water area as a patch piece (corners at its level), so the transform gizmos can
    /// act on it; `set_water_piece` takes the result back.
    fn water_piece(&self) -> Option<Piece> {
        let verts: Vec<[f32; 3]> = self.water_points().into_iter().map(Into::into).collect();
        (!verts.is_empty()).then(|| Piece { shape: Shape::Patch { verts }, texture: String::new(), uv_scale: 0.0 })
    }

    fn set_water_piece(&mut self, p: &Piece) {
        let Shape::Patch { verts } = &p.shape else { return };
        if let Some(Source::Area { level, points }) = self.water_sel.and_then(|i| self.file.water.get_mut(i)).map(|b| &mut b.source) {
            *level = verts.first().map_or(*level, |v| v[1]);
            *points = verts.iter().map(|v| [v[0], v[2]]).collect();
        }
    }

    /// The selected stamp (#466), while no piece is selected.
    fn stamp_active(&self) -> Option<usize> {
        self.stamp_sel.filter(|&i| self.sel.is_none() && i < self.file.stamps.len())
    }

    /// The selected stamp's box as a block piece (base centre, size, yaw), so the transform gizmos can act on it;
    /// `set_stamp_piece` takes the result back.
    fn stamp_piece(&self) -> Option<Piece> {
        let st = &self.file.stamps[self.stamp_active()?];
        Some(Piece { shape: Shape::Block { centre: st.centre, size: st.size, yaw: st.yaw, pitch: 0.0, roll: 0.0, roof: None }, texture: String::new(), uv_scale: 0.0 })
    }

    fn set_stamp_piece(&mut self, p: &Piece) {
        let (Shape::Block { centre, size, yaw, .. }, Some(i)) = (&p.shape, self.stamp_active()) else { return };
        let st = &mut self.file.stamps[i];
        (st.centre, st.size, st.yaw) = (*centre, *size, yaw.rem_euclid(360.0));
    }

    /// Select stamp `i` (its box takes the gizmos and the handle); None: no stamp.
    pub fn select_stamp(&mut self, i: Option<usize>) {
        self.stamp_sel = i.filter(|&i| i < self.file.stamps.len());
        if self.stamp_sel.is_some() {
            (self.sel, self.water_sel, self.sel_handle, self.water_corner, self.hot) = (None, None, None, None, None);
        }
    }

    /// Select water body `i` (the Water section's body rows and the pond outline); a stale index selects nothing.
    pub fn select_water(&mut self, i: Option<usize>) {
        self.water_sel = i.filter(|&i| i < self.file.water.len());
        if self.water_sel.is_some() {
            (self.sel, self.stamp_sel, self.sel_handle, self.water_corner, self.hot) = (None, None, None, None, None);
        }
    }

    /// Select grass body `i` (the Grass section's body rows); a stale index selects nothing.
    pub fn select_grass(&mut self, i: Option<usize>) {
        self.grass_sel = i.filter(|&i| i < self.file.grass.len());
    }

    /// Cut the selected stamp again when its box changed since the last cut (the highlight); the viewer calls it each
    /// frame with the scene.
    pub fn refresh_capture(&mut self, scene: &Scene) {
        let Some(st) = self.stamp_active().map(|i| self.file.stamps[i].clone()) else {
            self.capture_edges = None;
            return;
        };
        if self.capture_edges.as_ref().is_some_and(|(c, _, _)| *c == st) {
            return;
        }
        let local = fill::stamp_tris(&scene.tris, &hidden_by_remaster(scene), &st);
        let mut placed = Vec::with_capacity(local.len());
        fill::place_stamp(&local, Vec3::from(st.centre), 1.0, st.yaw, &mut placed);
        let g: Vec<Vec3> = placed.iter().map(|v| Vec3::from(v.pos) * FLIP).collect();
        let edges = g.as_chunks::<3>().0.iter().flat_map(|t| [[t[0], t[1]], [t[1], t[2]], [t[2], t[0]]]).take(CAPTURE_EDGES).collect();
        self.capture_edges = Some((st, edges, local.len() / 3));
    }

    /// The selected water body's area points, game space at its level (empty: none selected, or a texture body).
    fn water_points(&self) -> Vec<Vec3> {
        match self.water_sel.and_then(|i| self.file.water.get(i)).map(|b| &b.source) {
            Some(Source::Area { level, points }) if self.sel.is_none() && self.stamp_active().is_none() => points.iter().map(|&[x, z]| Vec3::new(x, *level, z)).collect(),
            _ => Vec::new(),
        }
    }

    pub fn drag_to(&mut self, p: Vec3) {
        let Some(i) = self.drag else { return };
        // a stamp's base: the box follows the mouse onto the ground
        if let Some(k) = self.stamp_active() {
            self.file.stamps[k].centre = p.into();
            (self.work_y, self.dirty_mesh) = (p.y, true);
            return;
        }
        // a water area's corner: x and z follow the mouse, the level stays
        if self.sel.is_none() {
            if let Some(Source::Area { points, .. }) = self.water_sel.and_then(|w| self.file.water.get_mut(w)).map(|b| &mut b.source) {
                if let Some(q) = points.get_mut(i) {
                    *q = [p.x, p.z];
                    self.dirty_mesh = true;
                }
            }
            return;
        }
        let Some(s) = self.sel else { return };
        if let Some(piece) = self.file.pieces.get_mut(s) {
            // the mouse hits the ground, not the crest: a pinned hill point keeps its height (the gizmo and the panel change it)
            let to = match &piece.shape {
                Shape::Hill { points, .. } => points.get(i).and_then(|q| q.1).map_or(p, |y| p.with_y(y)),
                _ => p,
            };
            set_handle(piece, i, to);
            self.work_y = p.y;
            self.dirty_mesh = true;
        }
    }

    /// Handles of the selected piece, game space.
    pub fn sel_handles(&self) -> Vec<Vec3> {
        match self.sel.and_then(|s| self.file.pieces.get(s).map(|p| (s, p))) {
            Some((s, p)) => handles(&self.baked(s, p), self.work_y),
            None => match self.stamp_active() {
                Some(i) => vec![Vec3::from(self.file.stamps[i].centre)],
                None => self.water_points(),
            },
        }
    }

    fn default_texture(&self, scene: &Scene) -> String {
        self.file.pieces.last().map(|p| p.texture.clone()).or_else(|| scene.world_textures.first().cloned()).unwrap_or_default()
    }

    /// A click in drawing mode adds a point; the piece is created once it is complete.
    pub fn add_point(&mut self, p: Vec3, scene: &Scene, ground: &Ground) {
        // "split piece": the click names the connected piece of the texture that becomes its own body
        // (only then is the armed tool dropped: a tuple of both `take()`s would disarm every click)
        if let Some(name) = self.water_split.take() {
            self.drawing = None;
            self.commit();
            self.file.water.push(Body { source: Source::Texture { name, piece: Some(p.into()) }, params: WaterParams::new(Style::Still), bed: None });
            (self.water_sel, self.stamp_sel) = (Some(self.file.water.len() - 1), None);
            (self.sel, self.sel_handle) = (None, None);
            self.note = "split a water piece".into();
            return;
        }
        let Some((kind, pts)) = &mut self.drawing else { return };
        pts.push(p);
        self.work_y = p.y;
        if Some(pts.len()) == kind.auto_points() {
            self.finish(scene, ground);
        }
    }

    fn finish(&mut self, scene: &Scene, ground: &Ground) {
        let Some((kind, pts)) = self.drawing.take() else { return };
        if pts.len() < kind.min_points().min(kind.auto_points().unwrap_or(usize::MAX)) {
            self.drawing = Some((kind, pts));
            self.note = format!("{} needs {} points", kind.name(), kind.min_points());
            return;
        }
        let xz = |v: &Vec3| [v.x, v.z];
        if kind == Kind::Plant {
            if self.brush.paint {
                // REMASTER paint stroke: one undo step for the press and the drag that follows
                self.commit();
                self.stroke_plants = self.stamp(pts[0], scene, ground);
                self.stroke = Some(pts[0]);
                self.note = "painting plants".into();
            } else {
                self.place_plants(pts[0], scene, ground);
            }
            // the brush stays armed: the next click places more
            self.drawing = Some((Kind::Plant, Vec::new()));
            return;
        }
        if kind == Kind::Object {
            self.place_object_at(pts[0]);
            return;
        }
        if kind == Kind::Water {
            // an area at the clicked points' mean height, lifted so the surface clears the grass (design choice:
            // at ground height the pond is invisible); still water until the style is changed
            let level = pts.iter().map(|v| v.y).sum::<f32>() / pts.len() as f32 + WATER_LIFT;
            self.commit();
            self.file.water.push(Body { source: Source::Area { level, points: pts.iter().map(xz).collect() }, params: WaterParams::new(Style::Still), bed: None });
            (self.water_sel, self.stamp_sel) = (Some(self.file.water.len() - 1), None);
            (self.sel, self.sel_handle) = (None, None);
            self.note = "added water area".into();
            return;
        }
        let shape = match kind {
            Kind::Ground => Shape::Ground { points: pts.iter().map(|v| (xz(v), None)).collect() },
            Kind::Block => Shape::Block { centre: pts[0].into(), size: [20.0, 30.0, 20.0], yaw: 0.0, pitch: 0.0, roll: 0.0, roof: None },
            Kind::Wall => Shape::Wall { from: pts[0].into(), to: pts[1].into(), height: 20.0 },
            Kind::Hill => Shape::Hill { points: pts.iter().map(|v| (xz(v), None)).collect(), height: 15.0, width: 40.0, base: 0.0 },
            Kind::Patch => Shape::Patch { verts: pts.iter().map(|&v| v.into()).collect() },
            Kind::Skirt => Shape::Skirt { radius: 1500.0, drop: 30.0 },
            // sitting on the clicked ground: the centre half its height up
            Kind::Cloud => Shape::Cloud { centre: (pts[0] + Vec3::Y * fill::CLOUD_SIZE[1] * 0.5).into(), size: fill::CLOUD_SIZE, yaw: 0.0, density: fill::CLOUD_DENSITY },
            Kind::Water | Kind::Plant | Kind::Object => return,
        };
        self.commit();
        let texture = if kind == Kind::Cloud { String::new() } else { self.default_texture(scene) };
        self.file.pieces.push(Piece { shape, texture, uv_scale: DEFAULT_UV });
        self.sel = Some(self.file.pieces.len() - 1);
        self.note = format!("added {}", kind.name());
    }

    /// One structure click at `at` (game space): a copy of the chosen scene object standing there, selected; the tool stays
    /// armed for more. One undo step.
    fn place_object_at(&mut self, at: Vec3) {
        if self.capture {
            self.capture_at(at);
            return;
        }
        self.drawing = Some((Kind::Object, Vec::new()));
        let shape = match self.structure_choice() {
            Some(Structure::Object(name)) => Shape::Object { name, centre: at.into(), scale: 1.0, yaw: 0.0 },
            // the copy faces like the source: the box's own axes are the building's
            Some(Structure::Stamp(name)) => {
                let yaw = self.file.stamps.iter().find(|st| st.name == name).map_or(0.0, |st| st.yaw);
                Shape::Stamp { name, centre: at.into(), scale: 1.0, yaw }
            }
            None => {
                self.note = "this track has no scene objects and the file no stamps".into();
                return;
            }
        };
        self.commit();
        let label = piece_meta(&Piece { shape: shape.clone(), texture: String::new(), uv_scale: DEFAULT_UV });
        self.file.pieces.push(Piece { shape, texture: String::new(), uv_scale: DEFAULT_UV });
        (self.sel, self.stamp_sel) = (Some(self.file.pieces.len() - 1), None);
        self.note = format!("placed {label} (Esc ends)");
    }

    /// "New stamp"'s click at `at` (game space): a stamp box standing there, a fresh name, selected; the tool is done.
    /// One undo step.
    fn capture_at(&mut self, at: Vec3) {
        (self.capture, self.drawing) = (false, None);
        self.commit();
        let name = (1..).map(|n| format!("stamp{n}")).find(|n| !self.file.stamps.iter().any(|st| st.name.eq_ignore_ascii_case(n))).unwrap_or_default();
        let fit = self.capture_fit.take();
        let (centre, size, yaw) = fit.as_ref().map_or((at.into(), STAMP_SIZE, 0.0), |f| (f.centre, f.size, f.yaw));
        self.file.stamps.push(fill::Stamp { name: name.clone(), centre, size, yaw, floor: STAMP_FLOOR });
        self.select_stamp(Some(self.file.stamps.len() - 1));
        self.note = match fit {
            Some(f) => format!("added {name}: fitted round the building ({} parts, {} tris); trim it with the gizmos", f.parts, f.tris),
            None => format!("added {name}: no separate building under the click (bare ground, or a building welded into the ground): default 20 x 30 x 20 m box, size it with the gizmos"),
        };
    }

    /// The box round the building that world triangle `tri` of `scene` belongs to, for the capture click (`None`: not a
    /// separate building). The viewer calls it with the picked triangle just before the click.
    pub fn set_capture_fit(&mut self, scene: &Scene, tri: Option<usize>) {
        self.capture_fit = tri.and_then(|t| fill::fit_stamp(&scene.tris, &hidden_by_remaster(scene), t));
    }

    /// Headless `--world-capture X Z`: arm "New stamp" and click the topmost world triangle over game point (x, z).
    pub fn capture_top(&mut self, scene: &Scene, x: f32, z: f32) {
        self.start_capture();
        let tris: Vec<[[f32; 3]; 3]> = scene.tris.as_chunks::<3>().0.iter().map(|t| [t[0].pos, t[1].pos, t[2].pos]).collect();
        // view space: z is negated; a point far above, so the nearest in height is the highest
        // (`pick` falls back to the nearest centroid over a void: a triangle that does not lie over the point is a miss)
        let over = |t: &[[f32; 3]; 3]| t.iter().map(|p| p[0]).fold(f32::MAX, f32::min) - 1.0 <= x && x <= t.iter().map(|p| p[0]).fold(f32::MIN, f32::max) + 1.0 && t.iter().map(|p| p[2]).fold(f32::MAX, f32::min) - 1.0 <= -z && -z <= t.iter().map(|p| p[2]).fold(f32::MIN, f32::max) + 1.0;
        let tri = crate::procgen::water::pick(&tris, [x, 10_000.0, -z]).filter(|&i| over(&tris[i]));
        self.set_capture_fit(scene, tri);
        self.place_object_at(Vec3::new(x, 0.0, z));
    }

    /// Arm the structure tool for a stamp box ("New stamp").
    pub fn start_capture(&mut self) {
        self.water_split = None;
        (self.capture, self.drawing) = (true, Some((Kind::Object, Vec::new())));
        self.note = "click a building: a stamp box fits round it".into();
    }

    /// What the structure tool places: the chosen object or stamp when it exists, else the first object, else the first
    /// stamp.
    fn structure_choice(&self) -> Option<Structure> {
        let object = |n: &str| self.objects.iter().find(|o| o.name.eq_ignore_ascii_case(n)).map(|o| Structure::Object(o.name.clone()));
        let stamp = |n: &str| self.file.stamps.iter().find(|st| st.name.eq_ignore_ascii_case(n)).map(|st| Structure::Stamp(st.name.clone()));
        let own = match &self.structure {
            Structure::Object(n) => object(n),
            Structure::Stamp(n) => stamp(n),
        };
        own.or_else(|| self.objects.first().map(|o| Structure::Object(o.name.clone()))).or_else(|| self.file.stamps.first().map(|st| Structure::Stamp(st.name.clone())))
    }

    /// What the brush places, for the status note.
    fn brush_label(&self) -> String {
        match self.brush.models.as_slice() {
            [] => self.brush.species.name().to_string(),
            [one] => one.clone(),
            many => format!("{} models", many.len()),
        }
    }

    /// One brush click at `at` (game space): `brush.count` plants as one undo step.
    fn place_plants(&mut self, at: Vec3, scene: &Scene, ground: &Ground) {
        self.commit();
        let n = self.stamp(at, scene, ground);
        self.note = format!("placed {n} {} (Esc ends)", self.brush_label());
        // a rock with no picture on this track takes the last piece's texture: say so (#477)
        if self.brush.species == fill::Species::Rock && self.brush.texture.is_empty() && self.brush.models.is_empty() && fill::plant_textures(fill::Species::Rock, scene).is_empty() {
            self.note.push_str(": this track has no rock picture, pick a TEXTURE");
        }
    }

    /// The mouse reached `at` (game space) with the paint stroke held: stamp along the way.
    pub fn stroke_to(&mut self, at: Vec3, scene: &Scene, ground: &Ground) {
        let Some(last) = self.stroke else { return };
        let points = stroke_points(last, at, self.brush.spacing);
        let Some(&end) = points.last() else { return };
        for p in points {
            self.stroke_plants += self.stamp(p, scene, ground);
        }
        self.stroke = Some(end);
    }

    /// The button went up: the stroke is one finished undo step.
    pub fn stroke_end(&mut self) {
        if self.stroke.take().is_some() {
            self.note = format!("painted {} {} (Esc ends)", self.stroke_plants, self.brush_label());
        }
    }

    /// `brush.count` plants of the brush around `at`, appended to the file (no undo step: the caller's);
    /// returns how many. Each plant stands on `ground` (the world and the fill's own ground pieces),
    /// where the original world alone would put it under a piece the user added.
    fn stamp(&mut self, at: Vec3, scene: &Scene, ground: &Ground) -> usize {
        let b = self.brush.clone();
        let auto = if b.texture.is_empty() { fill::plant_textures(b.species, scene) } else { Vec::new() };
        let fallback = if b.texture.is_empty() { self.default_texture(scene) } else { b.texture.clone() };
        let trunk = (b.species.trunk() && !b.trunk.is_empty()).then(|| b.trunk.clone());
        let seed = self.file.seed.wrapping_add(self.file.pieces.len() as u32 * 7919).wrapping_add(1);
        let before = self.file.pieces.len();
        let shapes = match brush_groups(&b.models) {
            Some(groups) => fill::scatter_models(&groups, at, b.radius, b.count.max(1), b.scale, seed, scene),
            None => fill::scatter(b.species, at, b.radius, b.count.max(1), b.scale, seed, scene),
        };
        for shape in shapes {
            let shape = match shape {
                Shape::Model { name, mut centre, scale, mut yaw } => {
                    if !b.random_yaw {
                        yaw = 0.0;
                    }
                    if let Some(y) = ground(Vec3::new(centre[0], centre[1].max(at.y), centre[2])) {
                        centre[1] = y;
                    }
                    Shape::Model { name, centre, scale, yaw }
                }
                Shape::Plant { species, mut centre, scale, variant, mut yaw, .. } => {
                    if !b.random_yaw {
                        yaw = 0.0;
                    }
                    // the probe starts above the higher of the click and the original ground there
                    if let Some(y) = ground(Vec3::new(centre[0], centre[1].max(at.y), centre[2])) {
                        centre[1] = y;
                    }
                    Shape::Plant { species, centre, scale, variant, yaw, trunk: trunk.clone() }
                }
                s => s,
            };
            // the variant spreads automatic plants over the track's pictures of the species
            let texture = match &shape {
                Shape::Plant { variant, .. } if !auto.is_empty() => auto[*variant as usize % auto.len()].clone(),
                Shape::Model { .. } => String::new(),
                _ => fallback.clone(),
            };
            self.file.pieces.push(Piece { shape, texture, uv_scale: DEFAULT_UV });
        }
        self.sel = Some(self.file.pieces.len() - 1);
        self.dirty_mesh = true;
        self.file.pieces.len() - before
    }

    pub fn run(&mut self, cmd: Cmd, scene: &Scene, centre: &[Vec3]) {
        if cmd != Cmd::Generate {
            self.confirm_gen = false;
        }
        match cmd {
            Cmd::Save => {
                let path = self.path();
                let r = path.parent().map_or(Ok(()), std::fs::create_dir_all).and_then(|_| std::fs::write(&path, fill::to_text(&self.file)));
                self.note = match r {
                    Ok(()) => {
                        self.saved = self.file.clone();
                        format!("saved {}", path.display())
                    }
                    Err(e) => format!("save failed: {e}"),
                };
            }
            Cmd::Reload => {
                self.commit();
                self.file = fill::load(&self.root, self.track);
                self.saved = self.file.clone();
                (self.sel, self.stamp_sel) = (None, None);
                self.note = "reloaded from disk".into();
            }
            Cmd::Undo => {
                if let Some(f) = self.undo.pop() {
                    self.redo.push(std::mem::replace(&mut self.file, f));
                    self.dirty_mesh = true;
                }
            }
            Cmd::Redo => {
                if let Some(f) = self.redo.pop() {
                    self.undo.push(std::mem::replace(&mut self.file, f));
                    self.dirty_mesh = true;
                }
            }
            Cmd::Delete => {
                if let Some(i) = self.stamp_active() {
                    self.delete_stamp(i);
                } else if let Some(s) = self.sel.filter(|&s| s < self.file.pieces.len()) {
                    self.commit();
                    self.file.pieces.remove(s);
                    self.sel = s.checked_sub(1).or((!self.file.pieces.is_empty()).then_some(0));
                }
            }
            Cmd::Duplicate => {
                if let Some(s) = self.sel.filter(|&s| s < self.file.pieces.len()) {
                    self.commit();
                    let mut p = self.file.pieces[s].clone();
                    for (i, h) in handles(&p, self.work_y).into_iter().enumerate() {
                        set_handle(&mut p, i, h + Vec3::X * 10.0);
                    }
                    self.file.pieces.push(p);
                    self.sel = Some(self.file.pieces.len() - 1);
                }
            }
            Cmd::Generate => {
                if !self.file.pieces.is_empty() && !self.confirm_gen {
                    self.confirm_gen = true;
                    self.note = "Generate replaces every piece (water, grass and stamps stay): again to confirm".into();
                    return;
                }
                self.confirm_gen = false;
                self.commit();
                let water = std::mem::take(&mut self.file.water);
                let stamps = std::mem::take(&mut self.file.stamps);
                let grass = std::mem::take(&mut self.file.grass);
                let grass_none = self.file.grass_none;
                self.file = fill::auto(scene, self.file.seed.max(1));
                (self.file.water, self.file.stamps, self.file.grass, self.file.grass_none) = (water, stamps, grass, grass_none);
                self.sel = None;
                self.note = format!("generated {} pieces (kept water, grass and stamps)", self.file.pieces.len());
            }
            Cmd::Holes => {
                self.holes ^= true;
                if self.holes && self.edges.is_none() {
                    self.edges = Some(hole_edges(scene, centre));
                    self.note = format!("{} open edges near the road", self.edges.as_ref().map_or(0, Vec::len));
                }
            }
            Cmd::Step(d) => {
                let n = self.file.pieces.len() as isize;
                if n > 0 {
                    self.sel = Some((self.sel.map_or(if d > 0 { -1 } else { 0 }, |s| s as isize) + d).rem_euclid(n) as usize);
                }
            }
            Cmd::Start(kind) => {
                self.water_split = None;
                self.capture = false;
                self.drawing = Some((kind, Vec::new()));
                self.note = match kind.auto_points() {
                    Some(0) => String::new(),
                    Some(1) if kind == Kind::Plant => "click to plant, again for more, Esc ends".into(),
                    Some(1) if kind == Kind::Object => "click to place the object, again for more, Esc ends".into(),
                    Some(1) => format!("click to place the {}", kind.name()),
                    Some(n) => format!("click {n} points"),
                    None => "click points, Enter finishes, Backspace removes the last".into(),
                };
                if kind.auto_points() == Some(0) {
                    self.finish(scene, &|_| None);
                }
            }
            Cmd::Gizmo(m) => {
                self.mode = m;
                self.hot = None;
            }
            Cmd::Level => {
                let Some(s) = self.sel.filter(|&s| s < self.file.pieces.len()) else { return };
                let mut p = self.file.pieces[s].clone();
                if let Some(y) = level(&mut p, &|xz| fill::ground_heights(scene, xz)) {
                    self.commit();
                    self.file.pieces[s] = p;
                    self.work_y = y;
                    self.note = format!("levelled at y {y:.1}");
                }
            }
            Cmd::Finish => self.finish(scene, &|_| None),
            Cmd::Back => {
                if let Some((_, pts)) = &mut self.drawing {
                    pts.pop();
                }
            }
        }
    }

    /// Lines for the frame: pieces (selected in yellow), handles, the piece being drawn, holes.
    pub fn gizmos(&self, g: &mut Gizmos, eye: Vec3) {
        for (i, p) in self.file.pieces.iter().enumerate() {
            if self.is_hidden(i, p) {
                continue;
            }
            let sel = self.sel == Some(i);
            let c = if sel { SELECTED } else { OTHER };
            let shown = self.baked(i, p);
            for (pts, closed) in outline(&shown, self.work_y, &self.objects, &self.file.stamps) {
                g.polyline(&pts, closed, c, sel);
            }
            if let (false, Shape::Cloud { centre, .. }) = (sel, &p.shape) {
                // the marker that selects it (`cloud_markers`)
                g.cross(Vec3::from(*centre), 3.0, c, true);
            }
            if sel {
                for h in handles(&shown, self.work_y) {
                    g.cross(h, 1.5, c, true);
                }
                if let Some(&h) = handles(&shown, self.work_y).first() {
                    g.label(h, format!("#{i} {}", kind_of(p).name()), c);
                }
            }
        }
        // the selected water area: its outline and corners
        let wp = self.water_points();
        if !wp.is_empty() {
            g.polyline(&wp, true, SELECTED, true);
            for &h in &wp {
                g.cross(h, 1.5, SELECTED, true);
            }
            g.label(wp[0], format!("water {}", self.water_sel.unwrap_or(0)), SELECTED);
            if let Some(&h) = self.water_vertex().and_then(|i| wp.get(i)) {
                g.cross(h, 3.0, SELECTED, true);
            }
        }
        // a split texture body: the point that picks its piece
        if let Some(Source::Texture { piece: Some(p), .. }) = self.water_sel.and_then(|i| self.file.water.get(i)).map(|b| &b.source) {
            let h = Vec3::from(*p);
            g.cross(h, 3.0, SELECTED, true);
            g.label(h, format!("water {} piece", self.water_sel.unwrap_or(0)), SELECTED);
        }
        if let Some(&(s, i)) = self.sel_handle.as_ref().filter(|_| self.vertex().is_some()) {
            if let Some(h) = self.file.pieces.get(s).and_then(|p| handles(&self.baked(s, p), self.work_y).get(i).copied()) {
                g.cross(h, 3.0, SELECTED, true);
            }
        }
        // the selected stamp: its box, its floor and the triangles it cuts
        if let Some(st) = self.stamp_active().map(|i| &self.file.stamps[i]) {
            let c = Vec3::from(st.centre);
            for (pts, closed) in box_edges(c, stamp_lo(st), stamp_hi(st), 1.0, st.yaw) {
                g.polyline(&pts, closed, SELECTED, true);
            }
            // the bottom ring of the same box raised to the floor
            let floor = box_edges(c + Vec3::Y * st.floor, stamp_lo(st), stamp_hi(st), 1.0, st.yaw);
            g.polyline(&floor[0].0, true, PENDING, true);
            g.cross(c, 1.5, SELECTED, true);
            let n = self.capture_edges.as_ref().filter(|(cut, _, _)| cut == st).map(|(_, e, n)| {
                for [a, b] in e {
                    g.line(*a, *b, CAPTURED, false);
                }
                *n
            });
            g.label(c + Vec3::Y * st.size[1], n.map_or(format!("stamp {}", st.name), |n| format!("stamp {}: {n} tris", st.name)), SELECTED);
        }
        if let Some((c, r, drop, floor)) = self.skirt_target() {
            fill_gizmo::skirt_draw(g, c, r, floor, drop, self.hot, eye);
        }
        if let Some(t) = self.gizmo_target() {
            fill_gizmo::draw(g, self.mode, t.pivot, t.enabled, t.axes, self.hot, eye);
        }
        if let (Some((Kind::Plant, _)), Some(c)) = (&self.drawing, self.cursor_ground) {
            // the brush: the scatter radius round the cursor
            if self.brush.count > 1 {
                let ring: Vec<Vec3> = (0..48).map(|k| c + Vec3::new((k as f32 / 48.0 * std::f32::consts::TAU).cos(), 0.0, (k as f32 / 48.0 * std::f32::consts::TAU).sin()) * self.brush.radius).collect();
                g.polyline(&ring, true, PENDING, true);
            }
        }
        if let Some((_, pts)) = &self.drawing {
            g.polyline(pts, false, PENDING, true);
            for &p in pts {
                g.cross(p, 1.5, PENDING, true);
            }
        }
        if let (true, Some(edges)) = (self.holes, &self.edges) {
            for [a, b] in edges.iter().filter(|e| e[0].distance(eye) < HOLE_RANGE) {
                g.line(*a, *b, HOLE, true);
            }
        }
    }

    /// The editor's page in the Properties dock (`tune::Page::World`): the outliner over the
    /// selected piece's and the water bodies' fields.
    pub fn panel_body(&mut self, ui: &mut Ui, textures: &[String]) {
        let before = self.file.clone();
        if self.hidden_len != self.file.pieces.len() {
            (self.hidden_pieces, self.hidden_len) = (Vec::new(), self.file.pieces.len());
        }
        let count = self.file.pieces.len().to_string();
        section_counted(ui, ic::LIST, "Outliner", Some(&count), false, true, |ui| self.outliner(ui));
        let crest = self.sel.and_then(|s| self.crest_of(s)).map(<[f32]>::to_vec).unwrap_or_default();
        if let Some(p) = self.sel.and_then(|s| self.file.pieces.get_mut(s)) {
            let cmds = &mut self.cmds;
            let (objects, stamps) = (&self.objects, &self.file.stamps);
            section(ui, ic::BOX, "Piece", false, |ui| piece_fields(ui, p, textures, objects, stamps, &crest, cmds));
        }
        self.water_ui(ui, textures);
        self.grass_ui(ui, textures);
        self.stamps_ui(ui);
        // a field edit is one undo step: taken when the pointer is released
        if self.file != before {
            self.dirty_mesh = true;
            self.edit_base.get_or_insert(before);
        }
        if !ui.ctx().input(|i| i.pointer.any_down()) {
            if let Some(b) = self.edit_base.take() {
                self.undo.push(b);
                self.redo.clear();
            }
        }
    }

    /// The status bar's hint: what the mouse and keys do now (fill editor open, else free camera or race camera), as
    /// `glass_ui::hint` markup: `[key]` caps, `*MODE*` bold.
    pub fn hint(&self, open: bool, free: bool, race: bool) -> Vec<HintPart> {
        glass_ui::hint(&if open {
            if let Some((Kind::Plant, _)) = &self.drawing {
                if self.brush.paint {
                    format!("*PLANT BRUSH* [LMB] hold and drag: a plant every {:.0} m [Shift] snap [Esc] end", self.brush.spacing)
                } else {
                    "*PLANT BRUSH* [LMB] place [Shift] snap [Esc] end".to_string()
                }
            } else if let Some((Kind::Object, _)) = &self.drawing {
                if self.capture { "*NEW STAMP* [LMB] a building: the box fits round it [Esc] cancel".to_string() } else { "*STRUCTURE* [LMB] place [Shift] snap [Esc] end".to_string() }
            } else if let Some((k, pts)) = &self.drawing {
                format!("*{}* {} points [LMB] add [Enter] finish [Backspace] drop last [Shift] snap [Esc] cancel", k.name().to_uppercase(), pts.len())
            } else {
                "*SELECT* [LMB] pick a piece or handle, drag to move [Shift] snap to a vertex [RMB] look".to_string()
            }
        } else if free {
            "[WASD] move [Q][E] down / up [Shift] fast [RMB] look [P] pause".to_string()
        } else if race {
            "[Arrows] drive [V] camera [R] respawn [Tab] free cam [P] pause [F3] telemetry".to_string()
        } else {
            "[Tab] free cam [P] pause [F3] telemetry".to_string()
        })
    }

    /// REMASTER plant brush options (the tool options strip while the plant tool is armed): the mockup's `#opts` row
    /// in `glass_ui` widgets, with the model, spacing and trunk controls of #390 added to it.
    pub fn brush_ui(&mut self, ui: &mut Ui, textures: &[String]) {
        use glass_ui::{cap_label, choice_dropdown, num_count, num_w, seg, vsep, SegOpt};
        let b = &mut self.brush;
        let mode = seg(ui, &[SegOpt { icon: "", text: "CLICK" }, SegOpt { icon: "", text: "BRUSH" }], usize::from(b.paint), None);
        if let Some(i) = mode {
            b.paint = i == 1;
        }
        if b.paint {
            cap_label(ui, "spacing").on_hover_text("metres between two plants along the mouse path");
            num_w(ui, 70.0, &mut b.spacing, 0.5, Some(1.0..=100.0), " m");
        }
        cap_label(ui, "random rotation").on_hover_text("each placed plant or model turns a random angle about the vertical axis, so they do not look repeated");
        ui.checkbox(&mut b.random_yaw, "");
        vsep(ui, 24.0);
        let models = !b.models.is_empty();
        ui.add_enabled_ui(!models, |ui| {
            let names: Vec<String> = Species::ALL.iter().map(|sp| sp.name().to_uppercase()).collect();
            let opts: Vec<SegOpt> = Species::ALL.iter().zip(&names).map(|(sp, n)| SegOpt { icon: species_icon(*sp), text: n }).collect();
            if let Some(i) = seg(ui, &opts, Species::ALL.iter().position(|s| *s == b.species).unwrap_or(0), None) {
                b.species = Species::ALL[i];
            }
        });
        cap_label(ui, "model").on_hover_text("tick one or more models of the tree pack instead of a generated plant: each plant picks one ticked entry at random (a family then picks one of its members)");
        let shown = match b.models.as_slice() {
            [] => "(generated)".to_string(),
            [one] => one.clone(),
            many => format!("{} models", many.len()),
        };
        let cur = &mut b.models;
        ComboBox::from_id_salt("brush_model").selected_text(shown).close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside).show_ui(ui, |ui| {
            if ui.selectable_label(cur.is_empty(), "(generated)").clicked() {
                cur.clear();
            }
            let mut tick = |ui: &mut Ui, name: &str, label: String| {
                let mut on = cur.iter().any(|m| m == name);
                if ui.checkbox(&mut on, label).changed() {
                    if on {
                        cur.push(name.to_string());
                    } else {
                        cur.retain(|m| m != name);
                    }
                }
            };
            for (f, members) in crate::procgen::trees::families() {
                if members.len() > 1 {
                    tick(ui, f, format!("{f} (any of {})", members.len()));
                }
                for m in members {
                    tick(ui, m, m.to_string());
                }
            }
        });
        vsep(ui, 24.0);
        cap_label(ui, "count").on_hover_text("plants per click");
        num_count(ui, 60.0, &mut b.count, 1..=60);
        ui.add_enabled_ui(b.count > 1, |ui| {
            cap_label(ui, "radius");
            num_w(ui, 76.0, &mut b.radius, 0.5, Some(2.0..=150.0), " m");
        });
        cap_label(ui, "scale");
        num_w(ui, 64.0, &mut b.scale, 0.05, Some(0.05..=20.0), "");
        let trunk = b.species.trunk() && !models;
        for (label, on) in [("texture", !models), ("trunk", trunk)] {
            if !on {
                continue;
            }
            vsep(ui, 24.0);
            cap_label(ui, label);
            let cur = if label == "texture" { &mut b.texture } else { &mut b.trunk };
            // (auto) first, then the world textures
            let at = textures.iter().position(|t| t == cur).map_or(0, |i| i + 1);
            let names: Vec<&str> = std::iter::once("(auto)").chain(textures.iter().map(String::as_str)).collect();
            if let Some(to) = choice_dropdown(ui, 150.0, label, &names, at) {
                *cur = if to == 0 { String::new() } else { textures[to - 1].clone() };
            }
        }
    }

    /// REMASTER structure tool options (#466): the scene object each click places, picked from the track's own list.
    pub fn object_ui(&mut self, ui: &mut Ui) {
        use glass_ui::{cap_label, choice_dropdown};
        cap_label(ui, "object").on_hover_text("a copy of one of this track's scene objects (airships, billboards, signs...) or of a stamp (a building cut from the world) stands on each click; move, turn and scale it with the gizmos");
        let names: Vec<String> = self.objects.iter().map(|o| o.name.clone()).chain(self.file.stamps.iter().map(|st| format!("{STAMP_MARK}{}", st.name))).collect();
        if names.is_empty() {
            ui.label(egui::RichText::new("no scene objects on this track: New stamp cuts a building").size(13.0).color(crate::theme::DIM));
        } else {
            let choice = self.structure_choice();
            let at = match &choice {
                Some(Structure::Object(n)) => self.objects.iter().position(|o| &o.name == n),
                Some(Structure::Stamp(n)) => self.file.stamps.iter().position(|st| &st.name == n).map(|i| i + self.objects.len()),
                None => None,
            };
            let refs: Vec<&str> = names.iter().map(String::as_str).collect();
            if let Some(to) = choice_dropdown(ui, 190.0, "object", &refs, at.unwrap_or(0)) {
                self.structure = match to.checked_sub(self.objects.len()) {
                    None => Structure::Object(self.objects[to].name.clone()),
                    Some(k) => Structure::Stamp(self.file.stamps[k].name.clone()),
                };
                self.capture = false;
            }
        }
    }

    /// REMASTER stamps (#466, 12-world-fill 12.16): the source boxes of the file, the selected one's fields. Field edits
    /// go through `panel_body`'s diff (one undo step each); a rename also renames the copies.
    fn stamps_ui(&mut self, ui: &mut Ui) {
        section(ui, ic::BUILDING, &format!("Stamps ({})", self.file.stamps.len()), false, |ui| {
            ui.horizontal_wrapped(|ui| {
                if action(ui, ic::SCAN, "NEW STAMP", true, "click a building on the track: the box is fitted round all its parts (plain ground: a default box to fit with the gizmos), then place copies") {
                    self.start_capture();
                }
                let sel = self.stamp_active();
                if action(ui, ic::BUILDING, "PLACE COPIES", sel.is_some(), "the structure tool places copies of this stamp: click where they stand, Esc ends") {
                    if let Some(i) = sel {
                        self.structure = Structure::Stamp(self.file.stamps[i].name.clone());
                        self.stamp_sel = None;
                        self.capture = false;
                        self.run_start_object();
                    }
                }
            });
            if self.file.stamps.is_empty() {
                return;
            }
            let labels: Vec<String> = std::iter::once("(none)".to_string()).chain(self.file.stamps.iter().map(|st| st.name.clone())).collect();
            let names: Vec<&str> = labels.iter().map(String::as_str).collect();
            let at = self.stamp_active().map_or(0, |i| i + 1);
            let mut pick = None;
            glass_ui::prow(ui, "Stamp", false, None, |ui| pick = glass_ui::choice_dropdown(ui, 170.0, "Stamp", &names, at));
            if let Some(to) = pick {
                self.select_stamp(to.checked_sub(1));
            }
            let Some(i) = self.stamp_active() else { return };
            let copies = self.file.pieces.iter().filter(|p| matches!(&p.shape, Shape::Stamp { name, .. } if name.eq_ignore_ascii_case(&self.file.stamps[i].name))).count();
            let tris = self.capture_edges.as_ref().filter(|(c, _, _)| *c == self.file.stamps[i]).map_or("-".to_string(), |(_, _, n)| n.to_string());
            glass_ui::prow(ui, "Cut", false, None, |ui| {
                ui.label(egui::RichText::new(format!("{tris} triangles, {copies} copies")).size(13.0).color(crate::theme::DIM));
            });
            let mut name = self.file.stamps[i].name.clone();
            glass_ui::prow(ui, "Name", false, None, |ui| {
                ui.add(egui::TextEdit::singleline(&mut name).desired_width(170.0));
            });
            self.rename_stamp(i, &name);
            let st = &mut self.file.stamps[i];
            xyz_row(ui, "Base", &mut st.centre);
            xyz_row(ui, "Size w h d", &mut st.size);
            num_row(ui, "Yaw", &mut st.yaw, 1.0, Some("°"));
            num_row(ui, "Floor", &mut st.floor, 0.1, Some("m"));
            st.size = st.size.map(|v| v.max(0.1));
            if action(ui, ic::TRASH, "REMOVE STAMP", true, "takes this stamp and its copies out of the file") {
                self.delete_stamp(i);
            }
        });
    }

    /// Arm the structure tool for copies (`Cmd::Start(Kind::Object)` without a scene: its note only).
    fn run_start_object(&mut self) {
        self.water_split = None;
        self.drawing = Some((Kind::Object, Vec::new()));
        self.note = "click to place the object, again for more, Esc ends".into();
    }

    /// Rename stamp `i` to `to` (spaces become `_`; empty or taken: no change), and its copies with it.
    fn rename_stamp(&mut self, i: usize, to: &str) {
        let to = to.trim().replace(char::is_whitespace, "_");
        let old = self.file.stamps[i].name.clone();
        if to.is_empty() || to == old || self.file.stamps.iter().enumerate().any(|(k, st)| k != i && st.name.eq_ignore_ascii_case(&to)) {
            return;
        }
        for p in &mut self.file.pieces {
            if let Shape::Stamp { name, .. } = &mut p.shape {
                if name.eq_ignore_ascii_case(&old) {
                    *name = to.clone();
                }
            }
        }
        if matches!(&self.structure, Structure::Stamp(n) if n.eq_ignore_ascii_case(&old)) {
            self.structure = Structure::Stamp(to.clone());
        }
        self.file.stamps[i].name = to;
    }

    /// Remove stamp `i` and its copies (they cannot mesh without it): one undo step.
    fn delete_stamp(&mut self, i: usize) {
        let Some(name) = self.file.stamps.get(i).map(|st| st.name.clone()) else { return };
        self.commit();
        self.file.stamps.remove(i);
        let before = self.file.pieces.len();
        self.file.pieces.retain(|p| !matches!(&p.shape, Shape::Stamp { name: n, .. } if n.eq_ignore_ascii_case(&name)));
        (self.stamp_sel, self.sel) = (None, None);
        self.note = format!("removed stamp {name} and its {} copies", before - self.file.pieces.len());
    }

    /// Stop drawing (palette: the armed tool clicked again).
    pub fn disarm(&mut self) {
        self.water_split = None;
        self.stroke = None;
        self.drawing = None;
        self.capture = false;
    }

    /// REMASTER water (procgen/water.rs): textures marked as water and drawn areas, with their look. Glass rows like the
    /// rest of the dock (#433): the prose that used to sit among the rows is the buttons' hover help.
    fn water_ui(&mut self, ui: &mut Ui, textures: &[String]) {
        section(ui, ic::WAVES, &format!("Water ({})", self.file.water.len()), false, |ui| {
            texture_row(ui, "Texture", &mut self.water_tex, textures, Some("(choose)"));
            // the selected texture body takes this texture too, where it lies flat at its water levels
            let sel_tex = self.water_sel.filter(|&i| matches!(self.file.water.get(i).map(|b| &b.source), Some(Source::Texture { .. })));
            let have = !self.water_tex.is_empty();
            ui.horizontal_wrapped(|ui| {
                if action(ui, ic::WAVES, "MARK WATER", have, "a marked texture is replaced everywhere it is used (Remaster); new > water in the palette draws an area instead") {
                    self.undo.push(self.file.clone());
                    self.redo.clear();
                    self.file.water.push(Body { source: Source::Texture { name: self.water_tex.clone(), piece: None }, params: WaterParams::new(Style::Still), bed: None });
                    (self.water_sel, self.stamp_sel) = (Some(self.file.water.len() - 1), None);
                }
                if action(ui, ic::LAYERS, "SHALLOWS", have && sel_tex.is_some(), "joins this texture to the selected body where it lies flat at its water level (WATT+Sandv2)") {
                    self.undo.push(self.file.clone());
                    self.redo.clear();
                    if let Some(Source::Texture { name, .. }) = sel_tex.and_then(|i| self.file.water.get_mut(i)).map(|b| &mut b.source) {
                        name.push('+');
                        name.push_str(&self.water_tex);
                    }
                }
                // the connected piece of the selected texture under the next click becomes a body of its own
                if action(ui, ic::SCAN, "SPLIT PIECE", sel_tex.is_some(), "click a patch of that water: it gets its own body (a pool of the sea's texture), the selected body keeps the rest") {
                    if let Some(Source::Texture { name, .. }) = sel_tex.and_then(|i| self.file.water.get(i)).map(|b| &b.source) {
                        self.water_split = Some(name.clone());
                        self.drawing = Some((Kind::Water, Vec::new()));
                        self.note = "click the water piece to split off, Esc cancels".into();
                    }
                }
            });
            if self.file.water.is_empty() {
                return;
            }
            let labels: Vec<String> = self
                .file
                .water
                .iter()
                .enumerate()
                .map(|(i, b)| match &b.source {
                    Source::Texture { name, piece: None } => format!("{i}: texture {name}"),
                    Source::Texture { name, piece: Some(p) } => format!("{i}: texture {name}, piece at {:.0}, {:.0}", p[0], p[2]),
                    Source::Area { level, points } => format!("{i}: area y {level:.1}, {} points", points.len()),
                })
                .collect();
            let names: Vec<&str> = std::iter::once("(none)").chain(labels.iter().map(String::as_str)).collect();
            let sel = self.water_sel.filter(|&i| i < labels.len());
            glass_ui::prow(ui, "Body", false, None, |ui| {
                if let Some(to) = glass_ui::choice_dropdown(ui, 250.0, "Water body", &names, sel.map_or(0, |i| i + 1)) {
                    (self.water_sel, self.stamp_sel) = (to.checked_sub(1), None);
                    // its corners become the handles
                    (self.sel, self.sel_handle, self.water_corner) = (None, None, None);
                }
            });
            let Some(sel) = sel else { return };
            if action(ui, ic::TRASH, "REMOVE BODY", true, "takes this water body out of the file") {
                self.undo.push(self.file.clone());
                self.redo.clear();
                self.file.water.remove(sel);
                self.water_sel = None;
                return;
            }
            let Some(b) = self.file.water.get_mut(sel) else { return };
            glass_ui::prow(ui, "Style", false, None, |ui| {
                let opts = [glass_ui::SegOpt { icon: "", text: "STILL" }, glass_ui::SegOpt { icon: "", text: "OCEAN" }];
                if let Some(i) = glass_ui::seg(ui, &opts, usize::from(b.params.style == Style::Ocean), None) {
                    let st = [Style::Still, Style::Ocean][i];
                    if b.params.style != st {
                        b.params = WaterParams::new(st);
                    }
                }
            });
            if let Source::Area { level, points } = &mut b.source {
                num_row(ui, "Level", level, 0.1, Some("m"));
                // corners (also draggable in the viewport); at least three stay
                let mut remove = None;
                let n = points.len();
                for (k, q) in points.iter_mut().enumerate() {
                    // right to left: the remove button, then z and x
                    glass_ui::prow(ui, &format!("Corner {k}"), false, None, |ui| {
                        if n > 3 && glass_ui::ibtn(ui, ic::CLOSE, false).on_hover_text("remove corner").clicked() {
                            remove = Some(k);
                        }
                        glass_ui::num_w(ui, 56.0, &mut q[1], 0.5, None, "");
                        glass_ui::num_w(ui, 56.0, &mut q[0], 0.5, None, "");
                    });
                }
                if let Some(k) = remove {
                    points.remove(k);
                }
                // the floor of a drawn area, in a world texture of the track (texture bodies make theirs)
                let mut bed = b.bed.clone().unwrap_or_default();
                texture_row(ui, "Bed", &mut bed, textures, Some("(none)"));
                b.bed = (!bed.is_empty()).then_some(bed);
            }
            let p = &mut b.params;
            colour_row(ui, "Deep", &mut p.deep);
            colour_row(ui, "Shallow", &mut p.shallow);
            colour_row(ui, "Shore", &mut p.shore);
            for (label, v, lo, hi) in [("clarity", &mut p.clarity, 0.0, 1.0), ("wave height", &mut p.wave_height, 0.0, 6.0), ("wave length", &mut p.wave_length, 0.2, 120.0), ("speed", &mut p.speed, 0.0, 8.0), ("direction", &mut p.direction, -180.0, 180.0), ("chop", &mut p.chop, 0.0, 1.0), ("foam", &mut p.foam, 0.0, 1.0), ("reflect", &mut p.reflect, 0.0, 1.0), ("visibility", &mut p.visibility, 0.1, 30.0), ("extinction", &mut p.extinction, 0.1, 60.0), ("refraction", &mut p.refraction, 0.0, 1.0), ("bed depth", &mut p.bed_depth, 0.0, 12.0), ("tiling", &mut p.tiling, 0.1, 8.0), ("normals", &mut p.normals, 0.0, 1.0), ("shore foam", &mut p.shore_foam, 0.0, 6.0), ("foam height", &mut p.foam_height, 0.0, 10.0), ("caustics", &mut p.caustics, 0.0, 1.0)] {
                glass_ui::slider_row(ui, label, v, lo..=hi);
            }
        });
    }

    /// REMASTER grass (procgen/grass.rs): the `Grass` lines, world textures tagged as turf with
    /// their look. Without a line the built-in rule tags the `gras*` textures; "customise" copies
    /// its bodies into the file. Every edit goes through `panel_body`'s diff (one undo step each);
    /// the "blades" group holds the Ultra blade values (row 4 of the params). Glass rows (#433).
    fn grass_ui(&mut self, ui: &mut Ui, textures: &[String]) {
        let title = match (self.file.grass_none, self.file.grass.len()) {
            (true, _) => "Grass (none)".to_string(),
            (false, 0) => "Grass (auto)".to_string(),
            (false, n) => format!("Grass ({n})"),
        };
        section(ui, ic::SHRUB, &title, false, |ui| {
            glass_ui::prow(ui, "No grass on this track", false, None, |ui| {
                ui.checkbox(&mut self.file.grass_none, "").on_hover_text("writes \"Grass none\"");
            });
            if self.file.grass_none {
                return;
            }
            if self.file.grass.is_empty() {
                let auto = grass::auto_bodies(textures);
                let names: Vec<String> = auto.iter().map(|b| format!("{} ({})", b.params.style.name(), b.textures.join("+"))).collect();
                let rule = if names.is_empty() { "no grass textures on this track".to_string() } else { names.join(", ") };
                glass_ui::text_prow(ui, "Built-in rule", &rule);
                if action(ui, ic::WAND, "CUSTOMISE", !auto.is_empty(), "copy the rule's bodies into the file to tune them") {
                    self.file.grass = auto;
                    self.grass_sel = Some(0);
                }
            }
            texture_row(ui, "Texture", &mut self.grass_tex, textures, Some("(choose)"));
            let sel = self.grass_sel.filter(|&i| i < self.file.grass.len());
            let have = !self.grass_tex.is_empty();
            ui.horizontal_wrapped(|ui| {
                if action(ui, ic::SHRUB, "MARK GRASS", have && self.file.grass.len() < grass::MAX_BODIES, &format!("a new meadow body with this texture; up-facing world polygons with a body's textures get shell turf (Remaster), at most {} bodies", grass::MAX_BODIES)) {
                    self.file.grass.push(GrassBody { textures: vec![self.grass_tex.clone()], params: GrassParams::new(GrassStyle::Meadow) });
                    self.grass_sel = Some(self.file.grass.len() - 1);
                }
                if action(ui, ic::LAYERS, "ADD TEXTURE", have && sel.is_some(), "join this texture to the selected body") {
                    if let Some(b) = sel.and_then(|i| self.file.grass.get_mut(i)) {
                        if !b.textures.iter().any(|t| t.eq_ignore_ascii_case(&self.grass_tex)) {
                            b.textures.push(self.grass_tex.clone());
                        }
                    }
                }
            });
            if self.file.grass.is_empty() {
                return;
            }
            let labels: Vec<String> = self.file.grass.iter().enumerate().map(|(i, b)| format!("{i}: {} {}", b.params.style.name(), b.textures.join("+"))).collect();
            let names: Vec<&str> = std::iter::once("(none)").chain(labels.iter().map(String::as_str)).collect();
            glass_ui::prow(ui, "Body", false, None, |ui| {
                if let Some(to) = glass_ui::choice_dropdown(ui, 250.0, "Grass body", &names, sel.map_or(0, |i| i + 1)) {
                    self.grass_sel = to.checked_sub(1);
                }
            });
            let Some(sel) = sel else { return };
            if action(ui, ic::TRASH, "REMOVE BODY", true, "takes this grass body out of the file") {
                self.file.grass.remove(sel);
                self.grass_sel = None;
                return;
            }
            let Some(b) = self.file.grass.get_mut(sel) else { return };
            glass_ui::prow(ui, "Style", false, None, |ui| {
                let names: Vec<String> = GrassStyle::ALL.iter().map(|st| st.name().to_uppercase()).collect();
                let opts: Vec<glass_ui::SegOpt> = names.iter().map(|n| glass_ui::SegOpt { icon: "", text: n }).collect();
                if let Some(i) = glass_ui::seg(ui, &opts, GrassStyle::ALL.iter().position(|s| *s == b.params.style).unwrap_or(0), None) {
                    // picking a style resets the values to the style's
                    if b.params.style != GrassStyle::ALL[i] {
                        b.params = GrassParams::new(GrassStyle::ALL[i]);
                    }
                }
            });
            let p = &mut b.params;
            glass_ui::slider_row(ui, "shells", &mut p.shells, 1..=32);
            for (label, v, lo, hi) in [("height", &mut p.height, 0.0, 1.0), ("density", &mut p.density, 1.0, 60.0), ("thickness", &mut p.thickness, 0.0, 2.0), ("reach", &mut p.reach, 5.0, 200.0), ("sway", &mut p.sway, 0.0, 0.3), ("root shade", &mut p.root_shade, 0.0, 1.0), ("edge fade", &mut p.edge_fade, 0.01, 5.0), ("tip mix", &mut p.tip_mix, 0.0, 1.0)] {
                glass_ui::slider_row(ui, label, v, lo..=hi);
            }
            for (label, c) in [("Tint", &mut p.tint), ("Tip", &mut p.tip)] {
                glass_ui::prow(ui, label, false, None, |ui| {
                    let mut g = Vec3::from(*c);
                    glass_ui::vec3_field(ui, &mut g, 0.01, None);
                    *c = g.into();
                });
            }
            glass_ui::caption(ui, "blades (Remaster, Grass Ultra only: the real blades near the camera)");
            for (label, v, lo, hi) in [("blade density", &mut p.blade_density, 4.0, 40.0), ("blade width", &mut p.blade_width, 0.005, 0.1), ("blade reach", &mut p.blade_reach, 5.0, 80.0), ("blade bend", &mut p.blade_bend, 0.0, 1.0)] {
                glass_ui::slider_row(ui, label, v, lo..=hi);
            }
        });
    }

    /// Pieces grouped by kind (`.ol-kind` headers with their counts, `.ol-row` rows with the eye and the size); click
    /// selects, double-click frames it. A kind opens with a click on its header; the selection's kind starts open and
    /// a filter opens every kind it matches.
    fn outliner(&mut self, ui: &mut Ui) {
        glass_ui::search_with(ui, &mut self.filter, "filter pieces", ic::FILTER);
        ui.add_space(4.0);
        let needle = self.filter.to_lowercase();
        let sel_kind = self.sel.and_then(|i| self.file.pieces.get(i)).map(kind_of);
        let mut focus = None;
        egui::ScrollArea::vertical().max_height(300.0).auto_shrink([false, true]).show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            for kind in Kind::ALL.into_iter().filter(|k| *k != Kind::Water) {
                let rows: Vec<usize> = (0..self.file.pieces.len())
                    .filter(|&i| {
                        let p = &self.file.pieces[i];
                        kind_of(p) == kind && (needle.is_empty() || format!("{} #{i} {}", kind.name(), p.texture).to_lowercase().contains(&needle))
                    })
                    .collect();
                if rows.is_empty() {
                    continue;
                }
                let id = ui.scope_id().with(("outliner", kind as usize));
                let mut open = ui.data(|d| d.get_temp::<bool>(id)).unwrap_or(sel_kind == Some(kind));
                if glass_ui::kind_header(ui, open || !needle.is_empty(), kind_icon(kind), kind.name(), rows.len()).clicked() {
                    open = !open;
                }
                ui.data_mut(|d| d.insert_temp(id, open));
                if !(open || !needle.is_empty()) {
                    continue;
                }
                for i in rows {
                    let off = self.hidden_pieces.contains(&i);
                    let (r, eye) = glass_ui::piece_row(ui, off, self.sel == Some(i), &format!("{} #{i}", kind.name().to_lowercase()), &piece_meta(&self.file.pieces[i]));
                    if eye {
                        // the hidden piece leaves the drawing too (#477): re-mesh
                        self.dirty_mesh = true;
                        if off {
                            self.hidden_pieces.retain(|&h| h != i);
                        } else {
                            self.hidden_pieces.push(i);
                        }
                    } else if r.clicked() {
                        self.sel = Some(i);
                        (self.water_sel, self.stamp_sel) = (None, None);
                    }
                    if r.double_clicked() {
                        focus = Some(self.extent(&self.file.pieces[i]));
                    }
                }
            }
            if self.file.pieces.is_empty() {
                ui.label(egui::RichText::new("no pieces: pick a place tool on the left").size(13.0).color(crate::theme::DIM));
            }
        });
        if focus.is_some() {
            self.focus = focus;
        }
    }
}

/// The outliner row's size text (`24 × 6 m`): what tells two pieces of a kind apart.
fn piece_meta(p: &Piece) -> String {
    match &p.shape {
        Shape::Block { size, .. } => format!("{:.0} × {:.0} m", size[0], size[2]),
        Shape::Wall { from, to, .. } => format!("{:.0} m", Vec3::from(*from).distance(Vec3::from(*to))),
        Shape::Hill { height, width, .. } => format!("{width:.0} × {height:.0} m"),
        Shape::Ground { points } => format!("{} points", points.len()),
        Shape::Patch { verts } => format!("{} vertices", verts.len()),
        Shape::Skirt { radius, .. } => format!("r {radius:.0} m"),
        Shape::Plant { species, .. } => species.name().to_lowercase(),
        Shape::Model { name, .. } | Shape::Object { name, .. } => name.clone(),
        Shape::Stamp { name, .. } => format!("{STAMP_MARK}{name}"),
        Shape::Cloud { size, density, .. } => format!("{:.0} × {:.0} × {:.0} m, {density}", size[0], size[1], size[2]),
    }
}

/// The outliner / palette icon of a kind.
pub fn kind_icon(k: Kind) -> &'static str {
    [ic::GRID, ic::BOX, ic::WALL, ic::MOUNTAIN, ic::SPLINE, ic::FENCE, ic::WAVES, ic::TREE, ic::CLOUD, ic::BUILDING][k as usize]
}

fn species_icon(sp: Species) -> &'static str {
    match sp {
        Species::Tree | Species::Pine => ic::TREE,
        Species::Bush => ic::SHRUB,
        Species::Rock => ic::GEM,
    }
}

/// Open world edges within `HOLE_ROAD` of the race centreline, game space.
fn hole_edges(scene: &Scene, centre: &[Vec3]) -> Vec<[Vec3; 2]> {
    let near = |m: Vec3| centre.is_empty() || centre.iter().any(|c| (c.x - m.x).powi(2) + (c.z - m.z).powi(2) < HOLE_ROAD * HOLE_ROAD);
    fill::open_edges(&scene.tris)
        .into_iter()
        .map(|[a, b]| [a * FLIP, b * FLIP])
        .filter(|[a, b]| (HOLE_LEN.0..=HOLE_LEN.1).contains(&a.distance(*b)) && near((*a + *b) * 0.5))
        .collect()
}

/// A requester button that can be off (`enabled` false = dimmed, no click), with its hover help. True when clicked.
fn action(ui: &mut Ui, icon: &str, text: &str, enabled: bool, tip: &str) -> bool {
    ui.add_enabled_ui(enabled, |ui| glass_ui::req_button(ui, icon, text, false).on_hover_text(tip).clicked()).inner
}

/// A colour row (0..1 floats; the swatch edits 8-bit, so a value only changes when the swatch does).
fn colour_row(ui: &mut Ui, label: &str, c: &mut [f32; 3]) {
    glass_ui::prow(ui, label, false, None, |ui| {
        let mut rgb = c.map(|x| (x.clamp(0.0, 1.0) * 255.0).round() as u8);
        if glass_ui::swatch_field(ui, &mut rgb) {
            *c = rgb.map(|x| x as f32 / 255.0);
        }
    });
}

/// A texture choice row: a dropdown through `textures` with a filter field (`none_label` = the empty choice, offered
/// first when given).
fn texture_row(ui: &mut Ui, label: &str, cur: &mut String, textures: &[String], none_label: Option<&str>) {
    glass_ui::prow(ui, label, false, None, |ui| {
        let off = usize::from(none_label.is_some());
        let at = textures.iter().position(|t| t == cur).map_or(0, |i| i + off);
        let names: Vec<&str> = none_label.into_iter().chain(textures.iter().map(String::as_str)).collect();
        if let Some(to) = glass_ui::choice_dropdown(ui, 170.0, label, &names, at) {
            *cur = if to < off { String::new() } else { textures[to - off].clone() };
        }
    });
}

/// One number row of the piece fields.
fn num_row(ui: &mut Ui, label: &str, v: &mut f32, speed: f32, unit: Option<&str>) {
    glass_ui::prow(ui, label, false, unit, |ui| {
        glass_ui::num_field(ui, v, speed, None, false);
    });
}

/// An X / Y / Z row.
fn xyz_row(ui: &mut Ui, label: &str, v: &mut [f32; 3]) {
    glass_ui::prow(ui, label, false, None, |ui| {
        let mut g = Vec3::from(*v);
        glass_ui::vec3_field(ui, &mut g, 0.5, None);
        *v = g.into();
    });
}

/// `crest`: a hill's meshed ridge heights (the `y` switch pins a point where it is now).
fn piece_fields(ui: &mut Ui, p: &mut Piece, textures: &[String], objects: &[fill::ObjectInfo], stamps: &[fill::Stamp], crest: &[f32], cmds: &mut Vec<Cmd>) {
    ui.spacing_mut().item_spacing.y = 2.0;
    let kind = kind_of(p);
    glass_ui::prow(ui, "Kind", false, None, |ui| {
        ui.label(egui::RichText::new(format!("{}  {}", kind_icon(kind), kind.name().to_uppercase())).font(crate::theme::glass_font(14.0)).color(crate::theme::WHITE));
    });
    if matches!(p.shape, Shape::Ground { .. } | Shape::Patch { .. }) && ui.button("Level (make planar)").on_hover_text("every point to the mean height").clicked() {
        cmds.push(Cmd::Level);
    }
    if !matches!(p.shape, Shape::Model { .. } | Shape::Object { .. } | Shape::Stamp { .. } | Shape::Cloud { .. }) {
        texture_row(ui, "Texture", &mut p.texture, textures, None);
        num_row(ui, "UV scale", &mut p.uv_scale, 0.1, None);
    }
    match &mut p.shape {
        Shape::Ground { points } => {
            for (i, (xz, y)) in points.iter_mut().enumerate() {
                // right to left: the height, its switch, then the two coordinates
                glass_ui::prow(ui, &format!("Point {i}"), false, None, |ui| {
                    if let Some(h) = y {
                        glass_ui::num_w(ui, 56.0, h, 0.5, None, "");
                    }
                    let mut fixed = y.is_some();
                    if ui.checkbox(&mut fixed, "y").changed() {
                        *y = fixed.then_some(0.0);
                    }
                    glass_ui::num_w(ui, 56.0, &mut xz[1], 0.5, None, "");
                    glass_ui::num_w(ui, 56.0, &mut xz[0], 0.5, None, "");
                });
            }
        }
        Shape::Block { centre, size, yaw, pitch, roll, roof } => {
            xyz_row(ui, "Base centre", centre);
            xyz_row(ui, "Size w h d", size);
            num_row(ui, "Yaw", yaw, 1.0, Some("°"));
            num_row(ui, "Pitch", pitch, 1.0, Some("°"));
            num_row(ui, "Roll", roll, 1.0, Some("°"));
            let mut r = roof.clone().unwrap_or_default();
            texture_row(ui, "Roof", &mut r, textures, Some("(none)"));
            *roof = (!r.is_empty()).then_some(r);
        }
        Shape::Wall { from, to, height } => {
            xyz_row(ui, "From", from);
            xyz_row(ui, "To", to);
            num_row(ui, "Height", height, 0.5, Some("m"));
        }
        Shape::Hill { points, height, width, base } => {
            num_row(ui, "Height", height, 0.5, Some("m"));
            num_row(ui, "Width", width, 0.5, Some("m"));
            // the slope feet above the ground (negative = buried); the crest does not move
            num_row(ui, "Base", base, 0.5, Some("m"));
            for (i, (xz, y)) in points.iter_mut().enumerate() {
                // right to left: the crest height, its switch, then the two coordinates
                glass_ui::prow(ui, &format!("Point {i}"), false, None, |ui| {
                    if let Some(h) = y {
                        glass_ui::num_w(ui, 56.0, h, 0.5, None, "");
                    }
                    let mut fixed = y.is_some();
                    if ui.checkbox(&mut fixed, "y").changed() {
                        *y = fixed.then(|| crest.get(i).copied().unwrap_or(0.0));
                    }
                    glass_ui::num_w(ui, 56.0, &mut xz[1], 0.5, None, "");
                    glass_ui::num_w(ui, 56.0, &mut xz[0], 0.5, None, "");
                });
            }
        }
        Shape::Patch { verts } => {
            for (i, v) in verts.iter_mut().enumerate() {
                xyz_row(ui, &format!("Vertex {i}"), v);
            }
        }
        Shape::Model { name, centre, scale, yaw } => {
            glass_ui::prow(ui, "Model", false, None, |ui| {
                ComboBox::from_id_salt("piece_model").selected_text(name.as_str()).show_ui(ui, |ui| {
                    for n in crate::procgen::trees::names() {
                        ui.selectable_value(name, n.to_string(), n);
                    }
                });
            });
            xyz_row(ui, "Base", centre);
            num_row(ui, "Scale", scale, 0.05, None);
            num_row(ui, "Yaw", yaw, 1.0, Some("°"));
        }
        Shape::Stamp { name, centre, scale, yaw } => {
            // the file's stamps; a name the file lacks stays listed (it is skipped when meshed)
            glass_ui::prow(ui, "Stamp", false, None, |ui| {
                let mut shown: Vec<&str> = stamps.iter().map(|st| st.name.as_str()).collect();
                let at = shown.iter().position(|n| n.eq_ignore_ascii_case(name));
                if at.is_none() {
                    shown.insert(0, name.as_str());
                }
                if let Some(to) = glass_ui::choice_dropdown(ui, 170.0, "stamp", &shown, at.unwrap_or(0)) {
                    *name = shown[to].to_string();
                }
            });
            xyz_row(ui, "Base", centre);
            num_row(ui, "Scale", scale, 0.05, None);
            num_row(ui, "Yaw", yaw, 1.0, Some("°"));
        }
        Shape::Object { name, centre, scale, yaw } => {
            // the track's own objects; a name this track lacks stays listed (it is skipped when meshed)
            glass_ui::prow(ui, "Object", false, None, |ui| {
                let names: Vec<&str> = objects.iter().map(|o| o.name.as_str()).collect();
                let at = names.iter().position(|n| n.eq_ignore_ascii_case(name)).unwrap_or(usize::MAX);
                let mut shown = names.clone();
                if at == usize::MAX {
                    shown.insert(0, name.as_str());
                }
                let cur = if at == usize::MAX { 0 } else { at };
                if let Some(to) = glass_ui::choice_dropdown(ui, 170.0, "object", &shown, cur) {
                    *name = shown[to].to_string();
                }
            });
            xyz_row(ui, "Base", centre);
            num_row(ui, "Scale", scale, 0.05, None);
            num_row(ui, "Yaw", yaw, 1.0, Some("°"));
        }
        Shape::Cloud { centre, size, yaw, density } => {
            xyz_row(ui, "Centre", centre);
            xyz_row(ui, "Size w h d", size);
            num_row(ui, "Yaw", yaw, 1.0, Some("°"));
            // extinction per 100 m at the centre (the fog page's Density unit); the edge has none
            num_row(ui, "Density", density, 0.1, Some("/100 m"));
            *density = density.max(0.0);
            for v in size.iter_mut() {
                *v = v.max(1.0);
            }
        }
        Shape::Skirt { radius, drop } => {
            num_row(ui, "Radius", radius, 5.0, Some("m"));
            num_row(ui, "Drop", drop, 0.5, Some("m"));
        }
        Shape::Plant { species, centre, scale, variant, yaw, trunk } => {
            glass_ui::prow(ui, "Species", false, None, |ui| {
                let names: Vec<String> = Species::ALL.iter().map(|sp| sp.name().to_uppercase()).collect();
                let opts: Vec<glass_ui::SegOpt> = Species::ALL.iter().zip(&names).map(|(sp, n)| glass_ui::SegOpt { icon: species_icon(*sp), text: n }).collect();
                if let Some(i) = glass_ui::seg(ui, &opts, Species::ALL.iter().position(|s| s == species).unwrap_or(0), None) {
                    *species = Species::ALL[i];
                }
            });
            xyz_row(ui, "Base", centre);
            num_row(ui, "Scale", scale, 0.05, None);
            num_row(ui, "Yaw", yaw, 1.0, Some("°"));
            glass_ui::prow(ui, "Variant", false, None, |ui| {
                wheel_drag(ui, variant, None, |d| d);
            });
            if species.trunk() {
                let mut t = trunk.clone().unwrap_or_default();
                texture_row(ui, "Trunk", &mut t, textures, Some("(piece texture)"));
                *trunk = (!t.is_empty()).then_some(t);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stroke_stamps_every_spacing_and_restarts_on_a_jump() {
        let o = Vec3::new(10.0, 1.0, 0.0);
        // 40 m at 8 m spacing: 5 stamps, the last one on the target, heights interpolated
        let p = stroke_points(o, Vec3::new(50.0, 5.0, 0.0), 8.0);
        assert_eq!(p.len(), 5);
        assert!((p[4] - Vec3::new(50.0, 5.0, 0.0)).length() < 1e-4);
        assert!((p[0].y - 1.8).abs() < 1e-4);
        // under one spacing: nothing yet
        assert!(stroke_points(o, Vec3::new(14.0, 1.0, 0.0), 8.0).is_empty());
        // a diagonal counts its real length
        assert_eq!(stroke_points(o, Vec3::new(10.0 + 30.0, 1.0, 40.0), 10.0).len(), 5);
        // a jump of 1000 m is one stamp at the target
        assert_eq!(stroke_points(o, Vec3::new(1010.0, 1.0, 0.0), 8.0), vec![Vec3::new(1010.0, 1.0, 0.0)]);
    }

    #[test]
    fn brush_model_choice_is_ticked_families_and_models_or_the_generated_plants() {
        let v = |a: &[&str]| a.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(brush_groups(&[]), None);
        assert_eq!(brush_groups(&v(&["Nope"])), None);
        assert_eq!(brush_groups(&v(&["Tree-01-3"])), Some(vec![vec!["Tree-01-3"]]));
        assert_eq!(brush_groups(&v(&["Tree-01"])).map(|g| g[0].len()), Some(4));
        let g = brush_groups(&v(&["Tree-01", "Nope", "Bush"])).unwrap();
        assert_eq!((g.len(), g[0].len(), g[1].len()), (2, 4, 5));
    }

    #[test]
    fn handles_round_trip() {
        let (f, w) = fill::from_text("Piece ground Texture A Point 0 0 Point 10 0 2 Point 10 10\nPiece wall Texture A From 0 0 0 To 5 1 5 Height 3\nPiece block Texture A Centre 1 2 3 Size 4 5 6 Yaw 0\nPiece patch Texture A Vertex 0 0 0 Vertex 1 0 0 Vertex 1 1 0\nPiece model Name Bush-02 Centre 1 2 3 Scale 2 Yaw 40\nPiece object Name Je_tower Centre 4 5 6 Scale 3 Yaw 20\n");
        assert!(w.is_empty());
        for p in f.pieces {
            let mut q = p.clone();
            for (i, h) in handles(&p, 0.0).into_iter().enumerate() {
                set_handle(&mut q, i, h);
            }
            assert_eq!(p, q);
        }
    }

    /// The structure tool places the chosen object when this track has it, else the first one (or none).
    #[test]
    fn the_structure_tool_falls_back_to_the_first_object() {
        let info = |n: &str| fill::ObjectInfo { name: n.into(), lo: Vec3::ZERO, hi: Vec3::ONE };
        let ob = |n: &str| Some(Structure::Object(n.into()));
        let mut e = WorldEdit { objects: vec![info("AirShipnn"), info("Je_tower")], ..Default::default() };
        assert_eq!(e.structure_choice(), ob("AirShipnn"));
        e.structure = Structure::Object("je_TOWER".into());
        assert_eq!(e.structure_choice(), ob("Je_tower"));
        e.structure = Structure::Object("Gone".into());
        assert_eq!(e.structure_choice(), ob("AirShipnn"));
        // a stamp of the file, chosen or as the last resort
        e.file.stamps.push(stamp("tower1"));
        e.structure = Structure::Stamp("TOWER1".into());
        assert_eq!(e.structure_choice(), Some(Structure::Stamp("tower1".into())));
        e.structure = Structure::Stamp("gone".into());
        assert_eq!(e.structure_choice(), ob("AirShipnn"));
        e.objects.clear();
        assert_eq!(e.structure_choice(), Some(Structure::Stamp("tower1".into())));
        e.file.stamps.clear();
        assert_eq!(e.structure_choice(), None);
    }

    fn stamp(name: &str) -> fill::Stamp {
        fill::Stamp { name: name.into(), centre: [10.0, 2.0, 30.0], size: [8.0, 20.0, 6.0], yaw: 30.0, floor: 0.5 }
    }

    /// "New stamp": the click adds a box with a fresh name there, selects it (no piece), disarms; one undo step. A
    /// copy placed afterwards faces like the source (the stamp's yaw).
    #[test]
    fn a_capture_click_adds_a_stamp_and_copies_face_like_it() {
        let mut e = WorldEdit { drawing: Some((Kind::Object, Vec::new())), ..Default::default() };
        e.file.stamps.push(stamp("stamp1"));
        e.start_capture();
        e.place_object_at(Vec3::new(1.0, 2.0, 3.0));
        assert_eq!(e.file.stamps.len(), 2);
        let st = &e.file.stamps[1];
        assert_eq!((st.name.as_str(), st.centre, st.size, st.yaw, st.floor), ("stamp2", [1.0, 2.0, 3.0], STAMP_SIZE, 0.0, STAMP_FLOOR));
        assert_eq!((e.stamp_active(), e.sel, e.armed(), e.capture), (Some(1), None, None, false));
        assert_eq!(e.undo.len(), 1);
        e.file.stamps[1].yaw = 75.0;
        e.structure = Structure::Stamp("stamp2".into());
        e.drawing = Some((Kind::Object, Vec::new()));
        e.place_object_at(Vec3::new(500.0, 0.0, 0.0));
        assert_eq!(e.file.pieces[0].shape, Shape::Stamp { name: "stamp2".into(), centre: [500.0, 0.0, 0.0], scale: 1.0, yaw: 75.0 });
        assert_eq!((e.sel, e.stamp_active()), (Some(0), None));
        // two undo steps: before the copy (two stamps), before the box (one)
        assert_eq!(e.undo.iter().map(|f| (f.stamps.len(), f.pieces.len())).collect::<Vec<_>>(), [(1, 0), (2, 0)]);
    }

    /// A capture click with a fitted building takes the fit's box (its own base, size and turn, not the clicked point);
    /// the fit is used up by the click, so the next one without a building gets the default box.
    #[test]
    fn a_capture_click_with_a_fit_takes_the_fitted_box_once() {
        let mut e = WorldEdit::default();
        e.start_capture();
        e.capture_fit = Some(fill::StampFit { centre: [50.0, 12.0, -7.0], size: [74.0, 108.0, 70.0], yaw: 31.0, parts: 4, tris: 414 });
        e.place_object_at(Vec3::new(1.0, 2.0, 3.0));
        let st = &e.file.stamps[0];
        assert_eq!((st.centre, st.size, st.yaw, st.floor), ([50.0, 12.0, -7.0], [74.0, 108.0, 70.0], 31.0, STAMP_FLOOR));
        assert!(e.note.contains("4 parts") && e.note.contains("414 tris"), "{}", e.note);
        e.start_capture();
        e.place_object_at(Vec3::new(1.0, 2.0, 3.0));
        let st = &e.file.stamps[1];
        assert_eq!((st.centre, st.size, st.yaw), ([1.0, 2.0, 3.0], STAMP_SIZE, 0.0));
        assert!(e.note.contains("no separate building"), "{}", e.note);
        assert_eq!(e.undo.len(), 2);
    }

    /// The gizmos act on the selected stamp's box: move, turn about Y only, scale (grows up from the base); its handle
    /// is the base.
    #[test]
    fn the_stamp_box_takes_the_gizmos_and_the_handle() {
        let mut e = WorldEdit::default();
        e.file.stamps.push(stamp("tower1"));
        e.select_stamp(Some(0));
        assert_eq!(e.sel_handles(), vec![Vec3::new(10.0, 2.0, 30.0)]);
        e.mode = Mode::Rotate;
        assert_eq!(e.gizmo_target().map(|t| t.enabled), Some([false, true, false]));
        e.mode = Mode::Scale;
        let t = e.gizmo_target().unwrap();
        // the pivot is the middle of the box, half its height over the base
        assert!(t.pivot.distance(Vec3::new(10.0, 12.0, 30.0)) < 1e-4, "{}", t.pivot);
        let p = e.stamp_piece().unwrap();
        e.set_stamp_piece(&fill_gizmo::apply(&p, Delta::Scale { axis: 1, dir: Vec3::Y, factor: 2.0 }, None, 0.0));
        e.set_stamp_piece(&fill_gizmo::apply(&e.stamp_piece().unwrap(), Delta::Rotate(Quat::from_rotation_y(20f32.to_radians())), None, 0.0));
        e.set_stamp_piece(&fill_gizmo::apply(&e.stamp_piece().unwrap(), Delta::Move(Vec3::new(5.0, 0.0, 0.0)), None, 0.0));
        let st = &e.file.stamps[0];
        assert!((st.size[1] - 40.0).abs() < 1e-3 && (st.yaw - 50.0).abs() < 1e-3 && (st.centre[0] - 15.0).abs() < 1e-3, "{st:?}");
        assert_eq!((st.centre[1], st.size[0], st.floor), (2.0, 8.0, 0.5));
        // the handle drag moves the base; selecting a piece hands the gizmos back
        e.drag = Some(0);
        e.drag_to(Vec3::new(-1.0, 4.0, 7.0));
        assert_eq!(e.file.stamps[0].centre, [-1.0, 4.0, 7.0]);
        e.file.pieces.push(Piece { shape: Shape::Stamp { name: "tower1".into(), centre: [0.0; 3], scale: 1.0, yaw: 0.0 }, texture: String::new(), uv_scale: DEFAULT_UV });
        e.piece_press(0, Vec3::ZERO, (0.0, 0.0));
        assert_eq!((e.stamp_active(), e.sel), (None, Some(0)));
    }

    /// A rename follows the copies and the tool's choice; a taken or empty name is refused. Removing a stamp removes its
    /// copies in one undo step.
    #[test]
    fn stamp_rename_and_remove_take_the_copies_along() {
        let mut e = WorldEdit::default();
        e.file.stamps.extend([stamp("tower1"), stamp("hall")]);
        let copy = |n: &str| Piece { shape: Shape::Stamp { name: n.into(), centre: [0.0; 3], scale: 1.0, yaw: 0.0 }, texture: String::new(), uv_scale: DEFAULT_UV };
        e.file.pieces.extend([copy("tower1"), copy("hall"), copy("TOWER1")]);
        e.structure = Structure::Stamp("tower1".into());
        e.rename_stamp(0, " big tower ");
        assert_eq!(e.file.stamps[0].name, "big_tower");
        assert!(matches!(&e.file.pieces[0].shape, Shape::Stamp { name, .. } if name == "big_tower"));
        assert!(matches!(&e.file.pieces[2].shape, Shape::Stamp { name, .. } if name == "big_tower"));
        assert_eq!(e.structure, Structure::Stamp("big_tower".into()));
        e.rename_stamp(0, "HALL");
        e.rename_stamp(0, "  ");
        assert_eq!(e.file.stamps[0].name, "big_tower");
        e.select_stamp(Some(0));
        e.delete_stamp(0);
        assert_eq!((e.file.stamps.len(), e.file.pieces.len(), e.stamp_active()), (1, 1, None));
        assert!(e.note.contains("2 copies"), "{}", e.note);
        assert_eq!(e.undo.last().map(|f| (f.stamps.len(), f.pieces.len())), Some((2, 3)));
    }

    /// A copy's outline is the stamp's box where `fill::place_stamp` puts the cut triangles (base, scale, yaw agree).
    #[test]
    fn a_stamp_copy_outline_matches_the_placed_mesh() {
        let st = stamp("tower1");
        // the box's own frame: two corners and a point on the top face
        let local = vec![fill::test_vertex([-4.0, 0.0, -3.0]), fill::test_vertex([4.0, 0.0, 3.0]), fill::test_vertex([4.0, 20.0, -3.0])];
        for yaw in [0.0f32, 90.0, 33.0] {
            let (c, scale) = (Vec3::new(5.0, 7.0, -3.0), 2.0);
            let p = Piece { shape: Shape::Stamp { name: "TOWER1".into(), centre: c.into(), scale, yaw }, texture: String::new(), uv_scale: DEFAULT_UV };
            let boxed = outline(&p, 0.0, &[], std::slice::from_ref(&st));
            assert_eq!(boxed.len(), 6);
            let corners: Vec<Vec3> = boxed.iter().flat_map(|(pts, _)| pts.iter().copied()).collect();
            let mut placed = Vec::new();
            fill::place_stamp(&local, c, scale, yaw, &mut placed);
            for q in &placed {
                let g = Vec3::from(q.pos) * FLIP;
                assert!(corners.iter().any(|k| k.distance(g) < 1e-3), "yaw {yaw}: {g}");
            }
        }
        // a stamp the file lacks: the ring and the mast
        let p = Piece { shape: Shape::Stamp { name: "gone".into(), centre: [0.0; 3], scale: 1.0, yaw: 0.0 }, texture: String::new(), uv_scale: DEFAULT_UV };
        assert_eq!(outline(&p, 0.0, &[], std::slice::from_ref(&st)).len(), 2);
    }

    /// A click of the structure tool stands the chosen object on the point, selects it, keeps the tool armed and is one undo
    /// step; with no objects on the track it places nothing.
    #[test]
    fn a_structure_click_places_the_chosen_object() {
        let info = |n: &str| fill::ObjectInfo { name: n.into(), lo: Vec3::ZERO, hi: Vec3::ONE };
        let mut e = WorldEdit { objects: vec![info("AirShipnn"), info("Je_tower")], structure: Structure::Object("je_tower".into()), drawing: Some((Kind::Object, Vec::new())), ..Default::default() };
        e.place_object_at(Vec3::new(1.0, 2.0, 3.0));
        e.place_object_at(Vec3::new(4.0, 5.0, 6.0));
        assert_eq!(e.file.pieces.len(), 2);
        assert_eq!(e.file.pieces[1].shape, Shape::Object { name: "Je_tower".into(), centre: [4.0, 5.0, 6.0], scale: 1.0, yaw: 0.0 });
        assert!(e.file.pieces[1].texture.is_empty());
        assert_eq!((e.sel, e.armed()), (Some(1), Some(Kind::Object)));
        assert!(e.dirty_mesh);
        // each click is its own undo step: the state before the second one has one piece
        assert_eq!((e.undo.len(), e.undo.last().map(|f| f.pieces.len())), (2, Some(1)));
        let mut none = WorldEdit { drawing: Some((Kind::Object, Vec::new())), ..Default::default() };
        none.place_object_at(Vec3::ZERO);
        assert!(none.file.pieces.is_empty() && none.armed() == Some(Kind::Object) && none.note.contains("no scene objects"));
    }

    /// An object's outline is the box of the mesh `fill` places: base, scale and the game yaw agree (the mesher turns the
    /// view-space mesh by `-yaw`; the outline turns the game-space box by `+yaw`).
    #[test]
    fn an_object_outline_matches_the_placed_mesh() {
        let v = |x: f32, y: f32, z: f32| fill::test_vertex([x, y, z]);
        // view space, origin above the base: x -1..3, y 2..12, z (view) -4..2 = game z -2..4
        let mesh = vec![v(-1.0, 2.0, -4.0), v(3.0, 2.0, 2.0), v(3.0, 12.0, -4.0)];
        let info = fill::ObjectInfo { name: "Box".into(), lo: Vec3::new(-1.0, 0.0, -2.0), hi: Vec3::new(3.0, 10.0, 4.0) };
        for yaw in [0.0f32, 90.0, 33.0] {
            let (c, scale) = (Vec3::new(5.0, 7.0, -3.0), 2.0);
            let boxed = object_box("box", c, scale, yaw, std::slice::from_ref(&info));
            assert_eq!(boxed.len(), 6);
            let mut placed = Vec::new();
            fill::place_object(&mesh, c * FLIP, scale, -yaw.to_radians(), [1.0; 3], &|_, _| [1.0; 4], &mut placed);
            // every placed vertex is a corner of the box (game space = view space with z negated)
            let corners: Vec<Vec3> = boxed.iter().flat_map(|(pts, _)| pts.iter().copied()).collect();
            for q in &placed {
                let g = Vec3::from(q.pos) * FLIP;
                assert!(corners.iter().any(|k| k.distance(g) < 1e-3), "yaw {yaw}: {g}");
            }
            // it stands on the base
            assert!(placed.iter().map(|q| q.pos[1]).fold(f32::MAX, f32::min) - c.y < 1e-4);
        }
    }

    #[test]
    fn water_corner_drag_moves_and_undoes() {
        let (f, _) = fill::from_text("Water Area Level -3 Point 0 0 Point 10 0 Point 10 10 Style still\n");
        let mut e = WorldEdit { file: f, water_sel: Some(0), ..Default::default() };
        let hs = e.sel_handles();
        assert_eq!(hs, vec![Vec3::new(0.0, -3.0, 0.0), Vec3::new(10.0, -3.0, 0.0), Vec3::new(10.0, -3.0, 10.0)]);
        e.begin_drag(1);
        e.drag_to(Vec3::new(20.0, 5.0, 7.0));
        // x and z follow, the level stays
        assert_eq!(e.sel_handles()[1], Vec3::new(20.0, -3.0, 7.0));
        e.drag = None;
        assert!(e.can_undo());
        // selecting a piece takes the handles back
        e.sel = Some(0);
        assert!(e.sel_handles().is_empty());
    }

    #[test]
    fn water_and_grass_bodies_select_by_index() {
        let (f, _) = fill::from_text("Water Area Level -3 Point 0 0 Point 10 0 Point 10 10 Style still\nGrass Texture Gras1 Style meadow\n");
        let mut e = WorldEdit { file: f, sel: Some(0), ..Default::default() };
        e.select_water(Some(0));
        assert_eq!((e.water_sel, e.sel), (Some(0), None));
        e.select_water(Some(5));
        assert_eq!(e.water_sel, None);
        e.select_grass(Some(0));
        assert_eq!(e.grass_sel, Some(0));
        e.select_grass(Some(1));
        assert_eq!(e.grass_sel, None);
    }

    #[test]
    fn water_corner_gets_its_own_gizmo() {
        let (f, _) = fill::from_text("Water Area Level -3 Point 0 0 Point 10 0 Point 10 10 Style still\n");
        let mut e = WorldEdit { file: f, water_sel: Some(0), mode: Mode::Move, ..Default::default() };
        // no corner clicked: the gizmo is on the area, all three axes
        let t = e.gizmo_target().unwrap();
        assert_eq!((t.vertex, t.enabled), (None, [true; 3]));
        e.begin_drag(1);
        e.drag = None;
        let t = e.gizmo_target().unwrap();
        assert_eq!((t.vertex, t.pivot, t.enabled), (Some(1), Vec3::new(10.0, -3.0, 0.0), [true, false, true]));
        // another body or a click on nothing takes it back
        e.water_sel = Some(1);
        assert_eq!(e.water_vertex(), None);
        e.water_sel = Some(0);
        e.clear_vertex();
        assert_eq!(e.gizmo_target().unwrap().vertex, None);
    }

    #[test]
    fn level_flattens_a_patch_at_its_mean_height() {
        let (f, _) = fill::from_text("Piece patch Texture A Vertex 0 0 0 Vertex 4 2 0 Vertex 4 4 4\n");
        let mut p = f.pieces[0].clone();
        let no_world = |xz: &[[f32; 2]]| vec![0.0; xz.len()];
        assert_eq!(level(&mut p, &no_world), Some(2.0));
        assert_eq!(handles(&p, 0.0).iter().map(|h| h.y).collect::<Vec<_>>(), vec![2.0; 3]);
        // ground: pinned points keep their height, draped ones count at the world's
        let (f, _) = fill::from_text("Piece ground Texture A Point 0 0 Point 10 0 6 Point 10 10\n");
        let mut g = f.pieces[0].clone();
        assert_eq!(level(&mut g, &|xz| vec![3.0; xz.len()]), Some(4.0));
        assert!(handles(&g, 0.0).iter().all(|h| h.y == 4.0));
    }

    #[test]
    fn gizmo_moves_rotates_and_scales_a_water_area() {
        let (f, _) = fill::from_text("Water Area Level -3 Point 0 0 Point 10 0 Point 10 10 Point 0 10 Style still\n");
        let mut e = WorldEdit { file: f, water_sel: Some(0), mode: Mode::Scale, ..Default::default() };
        assert!(e.gizmo_target().is_some());
        let base = e.water_piece().unwrap();
        let pts = |e: &WorldEdit| match &e.file.water[0].source {
            Source::Area { level, points } => (*level, points.clone()),
            _ => unreachable!(),
        };
        // stretch along X about the middle (x 5): 0..10 becomes -5..15, the level stays
        let q = fill_gizmo::apply(&base, Delta::Scale { axis: 0, dir: Vec3::X, factor: 2.0 }, None, 0.0);
        e.set_water_piece(&q);
        assert_eq!(pts(&e), (-3.0, vec![[-5.0, 0.0], [15.0, 0.0], [15.0, 10.0], [-5.0, 10.0]]));
        // moving up changes the level
        let q = fill_gizmo::apply(&base, Delta::Move(Vec3::new(0.0, 2.0, 0.0)), None, 0.0);
        e.set_water_piece(&q);
        assert_eq!(pts(&e).0, -1.0);
        // rotating about Y keeps it level
        let q = fill_gizmo::apply(&base, Delta::Rotate(Quat::from_rotation_y(0.5)), None, 0.0);
        e.set_water_piece(&q);
        assert!((pts(&e).0 + 3.0).abs() < 1e-4);
        // no gizmo for a piece's selection mixed in: selecting a piece hides the water one
        e.mode = Mode::Handles;
        assert!(e.gizmo_target().is_none());
    }

    // an orthographic stand-in for the camera: x right, z down the screen, looking along -Y from y 50
    fn top_ray(px: (f32, f32)) -> (Vec3, Vec3) {
        (Vec3::new((px.0 - 400.0) / 10.0, 50.0, (px.1 - 300.0) / 10.0), Vec3::NEG_Y)
    }

    #[test]
    fn piece_click_is_no_undo_step_and_drag_moves_and_undoes() {
        let (f, _) = fill::from_text("Piece patch Texture A Vertex 0 2 0 Vertex 4 2 0 Vertex 4 2 4\nPiece wall Texture A From 20 0 0 To 25 0 0 Height 3\n");
        let mut e = WorldEdit { file: f, ..Default::default() };
        let nothing = |_: Vec3| None;
        e.piece_press(0, Vec3::new(1.0, 2.0, 1.0), (410.0, 310.0));
        assert_eq!(e.sel, Some(0));
        // a wobble under the threshold does nothing
        e.piece_drag(top_ray((412.0, 311.0)), (412.0, 311.0), &nothing);
        e.gizmo_release();
        assert!(!e.can_undo() && !e.dirty_mesh);
        // a real drag: 5 m right, 3 m back; the heights stay
        e.piece_press(0, Vec3::new(1.0, 2.0, 1.0), (410.0, 310.0));
        e.piece_drag(top_ray((460.0, 340.0)), (460.0, 340.0), &nothing);
        e.piece_drag(top_ray((460.0, 340.0)), (460.0, 340.0), &nothing);
        e.gizmo_release();
        assert_eq!(handles(&e.file.pieces[0], 0.0)[1], Vec3::new(9.0, 2.0, 3.0));
        assert_eq!(e.undo.len(), 1);
        assert!(e.dirty_mesh);
        e.file = e.undo.pop().unwrap();
        assert_eq!(handles(&e.file.pieces[0], 0.0)[1], Vec3::new(4.0, 2.0, 0.0));
        // the other piece never moved
        assert_eq!(handles(&e.file.pieces[1], 0.0)[0], Vec3::new(20.0, 0.0, 0.0));
    }

    #[test]
    fn dragged_block_drops_onto_the_ground() {
        let (f, _) = fill::from_text("Piece block Texture A Centre 0 0 0 Size 4 10 4 Yaw 0\n");
        let mut e = WorldEdit { file: f, ..Default::default() };
        e.piece_press(0, Vec3::new(0.0, 0.0, 0.0), (400.0, 300.0));
        e.piece_drag(top_ray((450.0, 300.0)), (450.0, 300.0), &|p| Some(p.x * 0.5));
        let Shape::Block { centre, .. } = &e.file.pieces[0].shape else { panic!() };
        assert_eq!(*centre, [5.0, 2.5, 0.0]);
    }

    #[test]
    fn skirt_ring_sets_the_radius_and_arrow_the_drop() {
        let mut e = WorldEdit { sel: Some(0), skirt_base: Some((Vec3::new(10.0, 0.0, 20.0), 0.0)), ..Default::default() };
        e.file = fill::from_text("Piece skirt Texture A Radius 500 Drop 30\n").0;
        assert_eq!(e.skirt_target(), Some((Vec3::new(10.0, 0.0, 20.0), 500.0, 30.0, -30.0)));
        // the camera hangs over the ring's +X point, 100 m up, looking down; 1 px per metre
        let eye = Vec3::new(510.0, 100.0, 20.0);
        let project = |p: Vec3| Some((400.0 + (p.x - 510.0), 300.0 + (p.z - 20.0)));
        let ray = |px: (f32, f32)| (Vec3::new(510.0 + px.0 - 400.0, 100.0, 20.0 + px.1 - 300.0), Vec3::NEG_Y);
        // the arrow stands at (400, 300); the ring runs down the screen through it: grab the ring 30 px
        // below, pull 40 m outwards
        let at = (400.0, 330.0);
        e.gizmo_hover(eye, &project, at);
        assert_eq!(e.hot, Some(0));
        e.gizmo_hover(eye, &project, (400.0, 302.0));
        assert_eq!(e.hot, Some(1));
        assert!(e.gizmo_press(eye, &project, at, ray(at)));
        e.gizmo_drag(ray((at.0 + 40.0, at.1)));
        let Shape::Skirt { radius, drop } = e.file.pieces[0].shape else { panic!() };
        assert!((radius - 540.0).abs() < 1.0 && drop == 30.0, "{radius} {drop}");
        e.gizmo_release();
        assert_eq!(e.undo.len(), 1);
    }

    #[test]
    fn move_handle() {
        let (f, _) = fill::from_text("Piece hill Texture A Point 0 0 Point 5 5 Height 4 Width 6\n");
        let mut p = f.pieces[0].clone();
        set_handle(&mut p, 1, Vec3::new(7.0, 0.0, 8.0));
        // a draped ridge point shows at the working height + the hill's height
        assert_eq!(handles(&p, 0.0)[1], Vec3::new(7.0, 4.0, 8.0));
    }

    #[test]
    fn hill_handles_use_the_meshed_crest_and_a_pinned_height() {
        let (f, _) = fill::from_text("Piece hill Texture A Point 0 0 Point 5 5 12 Height 4 Width 6\n");
        let e = WorldEdit { file: f, hill_crest: vec![(0, vec![9.0, 12.0])], ..Default::default() };
        let shown = e.baked(0, &e.file.pieces[0]);
        assert_eq!(handles(&shown, 0.0).iter().map(|h| h.y).collect::<Vec<_>>(), vec![9.0, 12.0]);
        // a mouse drag of a pinned point keeps its height; a draped one stays draped
        let mut p = e.file.pieces[0].clone();
        set_handle(&mut p, 1, Vec3::new(7.0, 50.0, 8.0));
        let Shape::Hill { points, .. } = &p.shape else { panic!() };
        assert_eq!(points[1], ([7.0, 8.0], Some(50.0)));
        assert_eq!(points[0].1, None);
        // a stale cache (other point count) is ignored
        let e = WorldEdit { hill_crest: vec![(0, vec![9.0])], ..e };
        assert_eq!(handles(&e.baked(0, &e.file.pieces[0]), 3.0).iter().map(|h| h.y).collect::<Vec<_>>(), vec![7.0, 12.0]);
    }

    #[test]
    fn gizmo_drag_moves_and_undoes() {
        let (f, _) = fill::from_text("Piece block Texture A Centre 0 0 0 Size 4 10 4 Yaw 0\n");
        let mut e = WorldEdit { file: f, sel: Some(0), mode: Mode::Move, ..Default::default() };
        // an orthographic stand-in for the camera: x right, y up, 10 px per metre, looking along +Z
        let project = |p: Vec3| Some((400.0 + p.x * 10.0, 300.0 - p.y * 10.0));
        let eye = Vec3::new(0.0, 5.0, -100.0);
        let ray = |px: (f32, f32)| (Vec3::new((px.0 - 400.0) / 10.0, (300.0 - px.1) / 10.0, -100.0), Vec3::Z);
        // the box centre is (0, 5, 0): its Y arrow rises from there
        let grab = (400.0, 300.0 - 5.0 * 10.0 - 40.0);
        e.gizmo_hover(eye, &project, grab);
        assert_eq!(e.hot, Some(1));
        assert!(e.gizmo_press(eye, &project, grab, ray(grab)));
        e.gizmo_drag(ray((grab.0, grab.1 - 30.0)));
        let Shape::Block { centre, .. } = &e.file.pieces[0].shape else { panic!() };
        assert!((centre[1] - 3.0).abs() < 1e-3 && centre[0] == 0.0, "{centre:?}");
        e.gizmo_release();
        // the whole drag is one undo step
        assert_eq!(e.undo.len(), 1);
        e.file = e.undo.pop().unwrap();
        let Shape::Block { centre, .. } = &e.file.pieces[0].shape else { panic!() };
        assert_eq!(centre[1], 0.0);
    }
}
