//! DEBUG / REMASTER World Editor (was the fill editor) (12-world-fill 12.5): edits `remaster/fill/trackNN.fill` in
//! motorview. State, commands (keys and panel buttons both push a `Cmd`), handles, gizmos and the
//! egui panel live here; the mouse and key plumbing is in the `motorview` bin's main.rs.
//! Coordinates are game space (the file's); the picker and camera are view space (`FLIP`).

use crate::debug::Gizmos;
use crate::fill_gizmo::{self, Delta, Mode};
use crate::icons as ic;
use crate::tune::section;
use crate::procgen::fill::{self, block_rotation, FillFile, Piece, Shape, Species};
use crate::procgen::grass::{self, Body as GrassBody, Params as GrassParams, Style as GrassStyle};
use crate::procgen::water::{Body, Params as WaterParams, Source, Style};
use crate::render::RenderMode;
use crate::scene::Scene;
use crate::gui::{wheel_drag, wheel_slider};
use egui::{ComboBox, Grid, Ui};
use glam::{Quat, Vec3};
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
}

impl Kind {
    pub const ALL: [Kind; 8] = [Kind::Ground, Kind::Block, Kind::Wall, Kind::Hill, Kind::Patch, Kind::Skirt, Kind::Water, Kind::Plant];

    pub fn name(self) -> &'static str {
        ["ground", "block", "wall", "hill", "patch", "skirt", "water", "plant"][self as usize]
    }

    /// The click that completes the piece by itself; None: Enter finishes it.
    fn auto_points(self) -> Option<usize> {
        match self {
            Kind::Block | Kind::Plant => Some(1),
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
        Shape::Plant { .. } => Kind::Plant,
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
    /// paint mode: hold the button and drag to plant along the mouse path (else one scatter per click)
    pub paint: bool,
    /// paint mode: metres between two stamps along the path
    pub spacing: f32,
}

impl Default for Brush {
    fn default() -> Self {
        Brush { species: Species::Tree, count: 1, radius: 25.0, scale: 1.0, texture: String::new(), trunk: String::new(), paint: false, spacing: 8.0 }
    }
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
    pub hidden: [bool; 8],
    /// pieces hidden by the outliner's eye (not saved); dropped when the piece count changes
    hidden_pieces: Vec<usize>,
    hidden_len: usize,
    /// outliner filter text
    pub filter: String,
    /// outliner double-click: game-space centre and size to frame the camera on
    pub focus: Option<(Vec3, f32)>,
    /// the world point under the mouse (game space), for the plant brush ring; set by the viewer
    pub cursor_ground: Option<Vec3>,
}

/// Handle positions (game space) of a piece; `y` for points without a height.
pub fn handles(p: &Piece, y: f32) -> Vec<Vec3> {
    match &p.shape {
        Shape::Ground { points } => points.iter().map(|&([x, z], h)| Vec3::new(x, h.unwrap_or(y), z)).collect(),
        Shape::Block { centre, .. } | Shape::Plant { centre, .. } => vec![Vec3::from(*centre)],
        Shape::Wall { from, to, .. } => vec![Vec3::from(*from), Vec3::from(*to)],
        Shape::Hill { points, .. } => points.iter().map(|&[x, z]| Vec3::new(x, y, z)).collect(),
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
        Shape::Block { centre, .. } | Shape::Plant { centre, .. } if i == 0 => *centre = to.into(),
        Shape::Wall { from, .. } if i == 0 => *from = to.into(),
        Shape::Wall { to: t, .. } if i == 1 => *t = to.into(),
        Shape::Hill { points, .. } => {
            if let Some(q) = points.get_mut(i) {
                *q = [to.x, to.z];
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
fn outline(p: &Piece, y: f32) -> Vec<(Vec<Vec3>, bool)> {
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
        Shape::Skirt { .. } => Vec::new(),
    }
}

impl WorldEdit {
    /// Start editing `track`'s file (kept as is when it is already the open track).
    pub fn begin(&mut self, root: &Path, track: u32) {
        if !self.open || self.track != track {
            self.file = fill::load(root, track);
            self.saved = self.file.clone();
            (self.undo, self.redo, self.sel, self.drawing, self.drag, self.edit_base, self.sel_handle, self.gdrag, self.pdrag) = Default::default();
        }
        (self.root, self.track, self.open, self.dirty_mesh) = (root.to_path_buf(), track, true, true);
        self.stroke = None;
        self.edges = None;
        self.work_y = 0.0;
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
    }

    /// A click that hit nothing: no vertex stays selected.
    pub fn clear_vertex(&mut self) {
        self.sel_handle = None;
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
        // a selected water area: the gizmo works on its outline as a flat patch
        if let Some(p) = self.water_piece() {
            let en = match self.mode {
                Mode::Handles => [false; 3],
                Mode::Move => [true; 3],
                // the surface stays level: turn about Y, stretch along X and Z
                Mode::Rotate => [false, true, false],
                Mode::Scale => [true, false, true],
            };
            return en.contains(&true).then(|| GizmoTarget { pivot: fill_gizmo::pivot(&p, 0.0), enabled: en, axes: fill_gizmo::AXES, vertex: None });
        }
        let p = self.sel.and_then(|s| self.file.pieces.get(s).map(|p| (s, p))).filter(|&(s, p)| !self.is_hidden(s, p)).map(|(_, p)| p)?;
        let en = fill_gizmo::enabled(p, self.mode);
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
            _ => fill_gizmo::plane_angle(pivot, axis, ray),
        };
        let (Some(start), Some(base)) = (start, self.sel.and_then(|s| self.file.pieces.get(s)).cloned().or_else(|| self.water_piece())) else { return false };
        self.commit();
        self.hot = Some(axis);
        self.gdrag = Some(GizmoDrag { axis, base, pivot, start, vertex, axes, len: fill_gizmo::length(pivot, eye) });
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
            _ => fill_gizmo::plane_angle(d.pivot, d.axis, ray).map(|a| Delta::Rotate(Quat::from_axis_angle(fill_gizmo::AXES[d.axis], fill_gizmo::angle_diff(d.start, a)))),
        };
        let Some(delta) = delta else { return };
        let piece = fill_gizmo::apply(&d.base, delta, d.vertex, self.work_y);
        if self.sel.is_none() {
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
        self.gdrag = Some(GizmoDrag { axis, base, pivot, start, vertex: None, axes: fill_gizmo::AXES, len: 1.0 });
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
        self.file.pieces.iter().enumerate().map(|(i, p)| Some(i) == except || self.is_hidden(i, p) || matches!(p.shape, Shape::Plant { .. } | Shape::Skirt { .. })).collect()
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
        (self.sel, self.water_sel, self.sel_handle, self.hot) = (Some(i), None, None, None);
        // a skirt has no position: selecting it is all a click does
        self.pdrag = (!matches!(p.shape, Shape::Skirt { .. })).then_some(PieceDrag { index: i, base: p, grab, press: cursor, moved: false });
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
        if let Shape::Block { centre, .. } | Shape::Plant { centre, .. } = &mut piece.shape {
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

    /// The selected water body's area points, game space at its level (empty: none selected, or a texture body).
    fn water_points(&self) -> Vec<Vec3> {
        match self.water_sel.and_then(|i| self.file.water.get(i)).map(|b| &b.source) {
            Some(Source::Area { level, points }) if self.sel.is_none() => points.iter().map(|&[x, z]| Vec3::new(x, *level, z)).collect(),
            _ => Vec::new(),
        }
    }

    pub fn drag_to(&mut self, p: Vec3) {
        let Some(i) = self.drag else { return };
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
            set_handle(piece, i, p);
            self.work_y = p.y;
            self.dirty_mesh = true;
        }
    }

    /// Handles of the selected piece, game space.
    pub fn sel_handles(&self) -> Vec<Vec3> {
        match self.sel.and_then(|s| self.file.pieces.get(s)) {
            Some(p) => handles(p, self.work_y),
            None => self.water_points(),
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
            self.water_sel = Some(self.file.water.len() - 1);
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
        if kind == Kind::Water {
            // an area at the clicked points' mean height; still water until the style is changed
            let level = pts.iter().map(|v| v.y).sum::<f32>() / pts.len() as f32;
            self.commit();
            self.file.water.push(Body { source: Source::Area { level, points: pts.iter().map(xz).collect() }, params: WaterParams::new(Style::Still), bed: None });
            self.water_sel = Some(self.file.water.len() - 1);
            (self.sel, self.sel_handle) = (None, None);
            self.note = "added water area".into();
            return;
        }
        let shape = match kind {
            Kind::Ground => Shape::Ground { points: pts.iter().map(|v| (xz(v), None)).collect() },
            Kind::Block => Shape::Block { centre: pts[0].into(), size: [20.0, 30.0, 20.0], yaw: 0.0, pitch: 0.0, roll: 0.0, roof: None },
            Kind::Wall => Shape::Wall { from: pts[0].into(), to: pts[1].into(), height: 20.0 },
            Kind::Hill => Shape::Hill { points: pts.iter().map(xz).collect(), height: 15.0, width: 40.0 },
            Kind::Patch => Shape::Patch { verts: pts.iter().map(|&v| v.into()).collect() },
            Kind::Skirt => Shape::Skirt { radius: 1500.0, drop: 30.0 },
            Kind::Water | Kind::Plant => return,
        };
        self.commit();
        self.file.pieces.push(Piece { shape, texture: self.default_texture(scene), uv_scale: DEFAULT_UV });
        self.sel = Some(self.file.pieces.len() - 1);
        self.note = format!("added {}", kind.name());
    }

    /// One brush click at `at` (game space): `brush.count` plants as one undo step.
    fn place_plants(&mut self, at: Vec3, scene: &Scene, ground: &Ground) {
        self.commit();
        let n = self.stamp(at, scene, ground);
        self.note = format!("placed {n} {} (Esc ends)", self.brush.species.name());
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
            self.note = format!("painted {} {} (Esc ends)", self.stroke_plants, self.brush.species.name());
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
        for shape in fill::scatter(b.species, at, b.radius, b.count.max(1), b.scale, seed, scene) {
            let shape = match shape {
                Shape::Plant { species, mut centre, scale, variant, .. } => {
                    // the probe starts above the higher of the click and the original ground there
                    if let Some(y) = ground(Vec3::new(centre[0], centre[1].max(at.y), centre[2])) {
                        centre[1] = y;
                    }
                    Shape::Plant { species, centre, scale, variant, trunk: trunk.clone() }
                }
                s => s,
            };
            // the variant spreads automatic plants over the track's pictures of the species
            let texture = match &shape {
                Shape::Plant { variant, .. } if !auto.is_empty() => auto[*variant as usize % auto.len()].clone(),
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
                self.sel = None;
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
                if let Some(s) = self.sel.filter(|&s| s < self.file.pieces.len()) {
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
                    self.note = "Generate replaces every piece: again to confirm".into();
                    return;
                }
                self.confirm_gen = false;
                self.commit();
                let water = std::mem::take(&mut self.file.water);
                self.file = fill::auto(scene, self.file.seed.max(1));
                self.file.water = water;
                self.sel = None;
                self.note = format!("generated {} pieces", self.file.pieces.len());
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
                self.drawing = Some((kind, Vec::new()));
                self.note = match kind.auto_points() {
                    Some(0) => String::new(),
                    Some(1) if kind == Kind::Plant => "click to plant, again for more, Esc ends".into(),
                    Some(1) => "click to place the block".into(),
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
            for (pts, closed) in outline(p, self.work_y) {
                g.polyline(&pts, closed, c, sel);
            }
            if sel {
                for h in handles(p, self.work_y) {
                    g.cross(h, 1.5, c, true);
                }
                if let Some(&h) = handles(p, self.work_y).first() {
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
        }
        // a split texture body: the point that picks its piece
        if let Some(Source::Texture { piece: Some(p), .. }) = self.water_sel.and_then(|i| self.file.water.get(i)).map(|b| &b.source) {
            let h = Vec3::from(*p);
            g.cross(h, 3.0, SELECTED, true);
            g.label(h, format!("water {} piece", self.water_sel.unwrap_or(0)), SELECTED);
        }
        if let Some(&(s, i)) = self.sel_handle.as_ref().filter(|_| self.vertex().is_some()) {
            if let Some(h) = self.file.pieces.get(s).and_then(|p| handles(p, self.work_y).get(i).copied()) {
                g.cross(h, 3.0, SELECTED, true);
            }
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
        let status = if self.unsaved() { "unsaved changes" } else { "saved" };
        ui.weak(format!("track {}: {} pieces, {status}", self.track, self.file.pieces.len()));
        section(ui, ic::LIST, "Outliner", false, |ui| self.outliner(ui));
        if let Some(p) = self.sel.and_then(|s| self.file.pieces.get_mut(s)) {
            let cmds = &mut self.cmds;
            section(ui, ic::BOX, "Piece", false, |ui| piece_fields(ui, p, textures, cmds));
        }
        self.water_ui(ui, textures);
        self.grass_ui(ui, textures);
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

    /// The status bar's hint: what the mouse and keys do now (fill editor open, else free
    /// camera or race camera).
    pub fn hint(&self, open: bool, free: bool, race: bool, shift: bool) -> String {
        if open {
            let snap = if shift { "  [snapping to a vertex]" } else { "" };
            if let Some((Kind::Plant, _)) = &self.drawing {
                if self.brush.paint {
                    format!("plant brush: hold LMB and drag to paint every {:.0} m, Esc ends{snap}", self.brush.spacing)
                } else {
                    format!("plant brush: LMB places plants, Esc ends{snap}")
                }
            } else if let Some((k, pts)) = &self.drawing {
                format!("{} tool, {} points: LMB add, Enter finish, Backspace removes the last, Esc cancel{snap}", k.name(), pts.len())
            } else {
                format!("LMB select a piece (drag moves it), pick a handle or gizmo axis, Shift snaps to a vertex, RMB look{snap}")
            }
        } else if free {
            "WASD move, Q/E down/up, RMB look, Shift fast, LMB copies target".into()
        } else if race {
            "arrows drive, V camera, R respawn, Tab free camera".into()
        } else {
            String::new()
        }
    }

    /// REMASTER plant brush options (the tool options strip while the plant tool is armed).
    pub fn brush_ui(&mut self, ui: &mut Ui, textures: &[String]) {
        ui.selectable_value(&mut self.brush.paint, false, "Click").on_hover_text("one scatter per click");
        ui.selectable_value(&mut self.brush.paint, true, "Brush").on_hover_text("hold LMB and drag: plants along the mouse path");
        ui.add_enabled_ui(self.brush.paint, |ui| {
            ui.label("spacing");
            wheel_drag(ui, &mut self.brush.spacing, Some(1.0..=100.0), |d| d.speed(0.5));
        });
        ui.separator();
        for sp in Species::ALL {
            if ui.selectable_label(self.brush.species == sp, format!("{}  {}", species_icon(sp), sp.name())).clicked() {
                self.brush.species = sp;
            }
        }
        ui.separator();
        ui.label("per click");
        wheel_drag(ui, &mut self.brush.count, Some(1..=60), |d| d);
        ui.add_enabled_ui(self.brush.count > 1, |ui| {
            ui.label("radius");
            wheel_drag(ui, &mut self.brush.radius, Some(2.0..=150.0), |d| d.speed(0.5));
        });
        ui.label("scale");
        wheel_drag(ui, &mut self.brush.scale, Some(0.05..=20.0), |d| d.speed(0.05));
        for (id, label, v) in [("brush_tex", "texture", true), ("brush_trunk", "trunk", self.brush.species.trunk())] {
            if !v {
                continue;
            }
            let cur = if id == "brush_tex" { &mut self.brush.texture } else { &mut self.brush.trunk };
            ui.label(label);
            ComboBox::from_id_salt(id).selected_text(if cur.is_empty() { "(auto)" } else { cur.as_str() }).show_ui(ui, |ui| {
                ui.selectable_value(cur, String::new(), "(auto)");
                for t in textures {
                    ui.selectable_value(cur, t.clone(), t);
                }
            });
        }
    }

    /// Stop drawing (palette: the armed tool clicked again).
    pub fn disarm(&mut self) {
        self.water_split = None;
        self.stroke = None;
        self.drawing = None;
    }

    /// REMASTER water (procgen/water.rs): textures marked as water and drawn areas, with their look.
    fn water_ui(&mut self, ui: &mut Ui, textures: &[String]) {
        section(ui, ic::WAVES, &format!("Water ({})", self.file.water.len()), false, |ui| {
            ui.horizontal(|ui| {
                ComboBox::from_id_salt("water_tex").selected_text(if self.water_tex.is_empty() { "texture..." } else { &self.water_tex }).show_ui(ui, |ui| {
                    for t in textures {
                        ui.selectable_value(&mut self.water_tex, t.clone(), t);
                    }
                });
                if ui.add_enabled(!self.water_tex.is_empty(), egui::Button::new("mark as water")).clicked() {
                    self.undo.push(self.file.clone());
                    self.redo.clear();
                    self.file.water.push(Body { source: Source::Texture { name: self.water_tex.clone(), piece: None }, params: WaterParams::new(Style::Still), bed: None });
                    self.water_sel = Some(self.file.water.len() - 1);
                }
                // the selected texture body takes this texture too, where it lies flat at its water levels
                let sel_tex = self.water_sel.filter(|&i| matches!(self.file.water.get(i).map(|b| &b.source), Some(Source::Texture { .. })));
                if ui.add_enabled(!self.water_tex.is_empty() && sel_tex.is_some(), egui::Button::new("add as shallows")).clicked() {
                    self.undo.push(self.file.clone());
                    self.redo.clear();
                    if let Some(Source::Texture { name, .. }) = sel_tex.and_then(|i| self.file.water.get_mut(i)).map(|b| &mut b.source) {
                        name.push('+');
                        name.push_str(&self.water_tex);
                    }
                }
                // the connected piece of the selected texture under the next click becomes a body of its own
                if ui.add_enabled(sel_tex.is_some(), egui::Button::new("split piece")).on_hover_text("click a patch of that water: it gets its own body (a pool of the sea's texture)").clicked() {
                    if let Some(Source::Texture { name, .. }) = sel_tex.and_then(|i| self.file.water.get(i)).map(|b| &b.source) {
                        self.water_split = Some(name.clone());
                        self.drawing = Some((Kind::Water, Vec::new()));
                        self.note = "click the water piece to split off, Esc cancels".into();
                    }
                }
            });
            ui.weak("new > water draws an area; a marked texture is replaced everywhere it is used (Remaster); \"add as shallows\" joins a texture to the selected one where it lies flat at its water level (WATT+Sandv2); \"split piece\" gives the connected patch you click its own body, the selected body keeps the rest");
            let mut remove = None;
            for (i, b) in self.file.water.iter().enumerate() {
                let name = match &b.source {
                    Source::Texture { name, piece: None } => format!("texture {name}"),
                    Source::Texture { name, piece: Some(p) } => format!("texture {name}, piece at {:.0}, {:.0}", p[0], p[2]),
                    Source::Area { level, points } => format!("area y {level:.1}, {} points", points.len()),
                };
                ui.horizontal(|ui| {
                    if ui.selectable_label(self.water_sel == Some(i), format!("{i}: {name} ({})", if b.params.style == Style::Ocean { "ocean" } else { "still" })).clicked() {
                        self.water_sel = Some(i);
                        // its corners become the handles
                        (self.sel, self.sel_handle) = (None, None);
                    }
                    if ui.small_button("x").clicked() {
                        remove = Some(i);
                    }
                });
            }
            if let Some(i) = remove {
                self.undo.push(self.file.clone());
                self.redo.clear();
                self.file.water.remove(i);
                self.water_sel = None;
            }
            let Some(b) = self.water_sel.and_then(|s| self.file.water.get_mut(s)) else { return };
            Grid::new("water").num_columns(2).striped(true).show(ui, |ui| {
                ui.label("style");
                ui.horizontal(|ui| {
                    for (st, n) in [(Style::Still, "still"), (Style::Ocean, "ocean")] {
                        if ui.selectable_label(b.params.style == st, n).clicked() && b.params.style != st {
                            b.params = WaterParams::new(st);
                        }
                    }
                });
                ui.end_row();
                if let Source::Area { level, points } = &mut b.source {
                    num(ui, "level", level, 0.1);
                    // corners (also draggable in the viewport); at least three stay
                    let mut remove = None;
                    for (k, q) in points.iter_mut().enumerate() {
                        ui.label(format!("corner {k}"));
                        ui.horizontal(|ui| {
                            wheel_drag(ui, &mut q[0], None, |d| d.speed(0.5));
                            wheel_drag(ui, &mut q[1], None, |d| d.speed(0.5));
                            if ui.small_button("x").on_hover_text("remove corner").clicked() {
                                remove = Some(k);
                            }
                        });
                        ui.end_row();
                    }
                    if let Some(k) = remove.filter(|_| points.len() > 3) {
                        points.remove(k);
                    }
                }
                // the floor of a drawn area, in a world texture of the track (texture bodies make theirs)
                if matches!(b.source, Source::Area { .. }) {
                    ui.label("bed");
                    ComboBox::from_id_salt("water_bed").selected_text(b.bed.as_deref().unwrap_or("none")).show_ui(ui, |ui| {
                        ui.selectable_value(&mut b.bed, None, "none");
                        for t in textures {
                            ui.selectable_value(&mut b.bed, Some(t.clone()), t);
                        }
                    });
                    ui.end_row();
                }
                let p = &mut b.params;
                ui.label("deep");
                ui.color_edit_button_rgb(&mut p.deep);
                ui.end_row();
                ui.label("shallow");
                ui.color_edit_button_rgb(&mut p.shallow);
                ui.end_row();
                ui.label("shore");
                ui.color_edit_button_rgb(&mut p.shore);
                ui.end_row();
                for (label, v, lo, hi) in [("clarity", &mut p.clarity, 0.0, 1.0), ("wave height", &mut p.wave_height, 0.0, 6.0), ("wave length", &mut p.wave_length, 0.2, 120.0), ("speed", &mut p.speed, 0.0, 8.0), ("direction", &mut p.direction, -180.0, 180.0), ("chop", &mut p.chop, 0.0, 1.0), ("foam", &mut p.foam, 0.0, 1.0), ("reflect", &mut p.reflect, 0.0, 1.0), ("visibility", &mut p.visibility, 0.1, 30.0), ("extinction", &mut p.extinction, 0.1, 60.0), ("refraction", &mut p.refraction, 0.0, 1.0), ("bed depth", &mut p.bed_depth, 0.0, 12.0), ("tiling", &mut p.tiling, 0.1, 8.0), ("normals", &mut p.normals, 0.0, 1.0), ("shore foam", &mut p.shore_foam, 0.0, 6.0), ("foam height", &mut p.foam_height, 0.0, 10.0), ("caustics", &mut p.caustics, 0.0, 1.0)] {
                    ui.label(label);
                    wheel_slider(ui, v, lo..=hi, |s| s);
                    ui.end_row();
                }
            });
        });
    }

    /// REMASTER grass (procgen/grass.rs): the `Grass` lines, world textures tagged as turf with
    /// their look. Without a line the built-in rule tags the `gras*` textures; "customise" copies
    /// its bodies into the file. Every edit goes through `panel_body`'s diff (one undo step each);
    /// the "blades" group holds the Ultra blade values (row 4 of the params).
    fn grass_ui(&mut self, ui: &mut Ui, textures: &[String]) {
        let title = match (self.file.grass_none, self.file.grass.len()) {
            (true, _) => "Grass (none)".to_string(),
            (false, 0) => "Grass (auto)".to_string(),
            (false, n) => format!("Grass ({n})"),
        };
        section(ui, ic::SHRUB, &title, false, |ui| {
            ui.checkbox(&mut self.file.grass_none, "no grass on this track").on_hover_text("writes \"Grass none\"");
            if self.file.grass_none {
                return;
            }
            if self.file.grass.is_empty() {
                let auto = grass::auto_bodies(textures);
                let names: Vec<String> = auto.iter().map(|b| format!("{} ({})", b.params.style.name(), b.textures.join("+"))).collect();
                ui.weak(format!("built-in rule: {}", if names.is_empty() { "no grass textures on this track".to_string() } else { names.join(", ") }));
                if ui.add_enabled(!auto.is_empty(), egui::Button::new("customise")).on_hover_text("copy the rule's bodies into the file to tune them").clicked() {
                    self.file.grass = auto;
                    self.grass_sel = Some(0);
                }
            }
            ui.horizontal(|ui| {
                ComboBox::from_id_salt("grass_tex").selected_text(if self.grass_tex.is_empty() { "texture..." } else { &self.grass_tex }).show_ui(ui, |ui| {
                    for t in textures {
                        ui.selectable_value(&mut self.grass_tex, t.clone(), t);
                    }
                });
                if ui.add_enabled(!self.grass_tex.is_empty() && self.file.grass.len() < grass::MAX_BODIES, egui::Button::new("mark as grass")).on_hover_text("a new meadow body with this texture").clicked() {
                    self.file.grass.push(GrassBody { textures: vec![self.grass_tex.clone()], params: GrassParams::new(GrassStyle::Meadow) });
                    self.grass_sel = Some(self.file.grass.len() - 1);
                }
                let sel = self.grass_sel.filter(|&i| i < self.file.grass.len());
                if ui.add_enabled(!self.grass_tex.is_empty() && sel.is_some(), egui::Button::new("add texture")).on_hover_text("join this texture to the selected body").clicked() {
                    if let Some(b) = sel.and_then(|i| self.file.grass.get_mut(i)) {
                        if !b.textures.iter().any(|t| t.eq_ignore_ascii_case(&self.grass_tex)) {
                            b.textures.push(self.grass_tex.clone());
                        }
                    }
                }
            });
            ui.weak(format!("up-facing world polygons with a body's textures get shell turf (Remaster); at most {} bodies", grass::MAX_BODIES));
            let mut remove = None;
            for (i, b) in self.file.grass.iter().enumerate() {
                ui.horizontal(|ui| {
                    if ui.selectable_label(self.grass_sel == Some(i), format!("{i}: {} {}", b.params.style.name(), b.textures.join("+"))).clicked() {
                        self.grass_sel = Some(i);
                    }
                    if ui.small_button("x").clicked() {
                        remove = Some(i);
                    }
                });
            }
            if let Some(i) = remove {
                self.file.grass.remove(i);
                self.grass_sel = None;
            }
            let Some(b) = self.grass_sel.and_then(|s| self.file.grass.get_mut(s)) else { return };
            Grid::new("grass").num_columns(2).striped(true).show(ui, |ui| {
                ui.label("style");
                ui.horizontal(|ui| {
                    for st in GrassStyle::ALL {
                        if ui.selectable_label(b.params.style == st, st.name()).on_hover_text("resets the values to the style's").clicked() && b.params.style != st {
                            b.params = GrassParams::new(st);
                        }
                    }
                });
                ui.end_row();
                let p = &mut b.params;
                ui.label("shells");
                wheel_slider(ui, &mut p.shells, 1..=32, |s| s);
                ui.end_row();
                for (label, v, lo, hi) in [("height", &mut p.height, 0.0, 1.0), ("density", &mut p.density, 1.0, 60.0), ("thickness", &mut p.thickness, 0.0, 2.0), ("reach", &mut p.reach, 5.0, 200.0), ("sway", &mut p.sway, 0.0, 0.3), ("root shade", &mut p.root_shade, 0.0, 1.0), ("edge fade", &mut p.edge_fade, 0.01, 5.0), ("tip mix", &mut p.tip_mix, 0.0, 1.0)] {
                    ui.label(label);
                    wheel_slider(ui, v, lo..=hi, |s| s);
                    ui.end_row();
                }
                for (label, c) in [("tint", &mut p.tint), ("tip", &mut p.tip)] {
                    ui.label(label);
                    ui.horizontal(|ui| {
                        for x in c.iter_mut() {
                            wheel_drag(ui, x, Some(0.0..=2.0), |d| d.speed(0.01));
                        }
                    });
                    ui.end_row();
                }
                ui.weak("blades").on_hover_text("Remaster, Grass Ultra only: the real blades near the camera");
                ui.end_row();
                for (label, v, lo, hi) in [("blade density", &mut p.blade_density, 4.0, 40.0), ("blade width", &mut p.blade_width, 0.005, 0.1), ("blade reach", &mut p.blade_reach, 5.0, 80.0), ("blade bend", &mut p.blade_bend, 0.0, 1.0)] {
                    ui.label(label);
                    wheel_slider(ui, v, lo..=hi, |s| s);
                    ui.end_row();
                }
            });
        });
    }

    /// Pieces grouped by kind with an eye toggle each; click selects, double-click frames it.
    fn outliner(&mut self, ui: &mut Ui) {
        ui.horizontal(|ui| {
            ui.label(ic::FILTER);
            ui.add(egui::TextEdit::singleline(&mut self.filter).hint_text("filter").desired_width(f32::INFINITY));
        });
        let needle = self.filter.to_lowercase();
        let mut focus = None;
        egui::ScrollArea::vertical().max_height(260.0).auto_shrink([false, true]).show(ui, |ui| {
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
                egui::CollapsingHeader::new(format!("{}  {} ({})", kind_icon(kind), kind.name(), rows.len())).id_salt(("outliner", kind as usize)).default_open(true).show(ui, |ui| {
                    for i in rows {
                        ui.horizontal(|ui| {
                            let off = self.hidden_pieces.contains(&i);
                            if ui.add(egui::Button::new(if off { ic::EYE_OFF } else { ic::EYE }).frame(false)).on_hover_text("hide while editing (not saved)").clicked() {
                                if off {
                                    self.hidden_pieces.retain(|&h| h != i);
                                } else {
                                    self.hidden_pieces.push(i);
                                }
                            }
                            let r = ui.selectable_label(self.sel == Some(i), format!("{} #{i}  {}", kind.name(), self.file.pieces[i].texture));
                            if r.clicked() {
                                self.sel = Some(i);
                                self.water_sel = None;
                            }
                            if r.double_clicked() {
                                focus = Some(self.extent(&self.file.pieces[i]));
                            }
                        });
                    }
                });
            }
            if self.file.pieces.is_empty() {
                ui.weak("no pieces: pick a place tool on the left");
            }
        });
        if focus.is_some() {
            self.focus = focus;
        }
    }
}

/// The outliner / palette icon of a kind.
pub fn kind_icon(k: Kind) -> &'static str {
    [ic::GRID, ic::BOX, ic::WALL, ic::MOUNTAIN, ic::SPLINE, ic::FENCE, ic::WAVES, ic::TREE][k as usize]
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

fn xyz(ui: &mut Ui, label: &str, v: &mut [f32; 3]) {
    ui.label(label);
    ui.horizontal(|ui| {
        for x in v.iter_mut() {
            wheel_drag(ui, x, None, |d| d.speed(0.5));
        }
    });
    ui.end_row();
}

fn num(ui: &mut Ui, label: &str, v: &mut f32, speed: f64) {
    ui.label(label);
    wheel_drag(ui, v, None, |d| d.speed(speed));
    ui.end_row();
}

fn texture_combo(ui: &mut Ui, id: &str, label: &str, cur: &mut String, textures: &[String]) {
    ui.label(label);
    ComboBox::from_id_salt(id).selected_text(cur.clone()).show_ui(ui, |ui| {
        for t in textures {
            ui.selectable_value(cur, t.clone(), t);
        }
    });
    ui.end_row();
}

fn piece_fields(ui: &mut Ui, p: &mut Piece, textures: &[String], cmds: &mut Vec<Cmd>) {
    if matches!(p.shape, Shape::Ground { .. } | Shape::Patch { .. }) && ui.button("Level (make planar)").on_hover_text("every point to the mean height").clicked() {
        cmds.push(Cmd::Level);
    }
    Grid::new("piece").num_columns(2).striped(true).show(ui, |ui| {
        texture_combo(ui, "tex", "texture", &mut p.texture, textures);
        num(ui, "uv scale", &mut p.uv_scale, 0.1);
        match &mut p.shape {
            Shape::Ground { points } => {
                for (i, (xz, y)) in points.iter_mut().enumerate() {
                    ui.label(format!("point {i}"));
                    ui.horizontal(|ui| {
                        wheel_drag(ui, &mut xz[0], None, |d| d.speed(0.5));
                        wheel_drag(ui, &mut xz[1], None, |d| d.speed(0.5));
                        let mut fixed = y.is_some();
                        if ui.checkbox(&mut fixed, "y").changed() {
                            *y = fixed.then_some(0.0);
                        }
                        if let Some(h) = y {
                            wheel_drag(ui, h, None, |d| d.speed(0.5));
                        }
                    });
                    ui.end_row();
                }
            }
            Shape::Block { centre, size, yaw, pitch, roll, roof } => {
                xyz(ui, "base centre", centre);
                xyz(ui, "size w h d", size);
                num(ui, "yaw", yaw, 1.0);
                num(ui, "pitch", pitch, 1.0);
                num(ui, "roll", roll, 1.0);
                let mut r = roof.clone().unwrap_or_default();
                texture_combo(ui, "roof", "roof", &mut r, textures);
                *roof = (!r.is_empty()).then_some(r);
            }
            Shape::Wall { from, to, height } => {
                xyz(ui, "from", from);
                xyz(ui, "to", to);
                num(ui, "height", height, 0.5);
            }
            Shape::Hill { points, height, width } => {
                num(ui, "height", height, 0.5);
                num(ui, "width", width, 0.5);
                for (i, q) in points.iter_mut().enumerate() {
                    ui.label(format!("point {i}"));
                    ui.horizontal(|ui| {
                        wheel_drag(ui, &mut q[0], None, |d| d.speed(0.5));
                        wheel_drag(ui, &mut q[1], None, |d| d.speed(0.5));
                    });
                    ui.end_row();
                }
            }
            Shape::Patch { verts } => {
                for (i, v) in verts.iter_mut().enumerate() {
                    xyz(ui, &format!("vertex {i}"), v);
                }
            }
            Shape::Skirt { radius, drop } => {
                num(ui, "radius", radius, 5.0);
                num(ui, "drop", drop, 0.5);
            }
            Shape::Plant { species, centre, scale, variant, trunk } => {
                ui.label("species");
                ui.horizontal(|ui| {
                    for sp in Species::ALL {
                        ui.selectable_value(species, sp, sp.name());
                    }
                });
                ui.end_row();
                xyz(ui, "base", centre);
                num(ui, "scale", scale, 0.05);
                ui.label("variant");
                wheel_drag(ui, variant, None, |d| d);
                ui.end_row();
                if species.trunk() {
                    let mut t = trunk.clone().unwrap_or_default();
                    texture_combo(ui, "trunk", "trunk", &mut t, textures);
                    *trunk = (!t.is_empty()).then_some(t);
                }
            }
        }
    });
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
    fn handles_round_trip() {
        let (f, w) = fill::from_text("Piece ground Texture A Point 0 0 Point 10 0 2 Point 10 10\nPiece wall Texture A From 0 0 0 To 5 1 5 Height 3\nPiece block Texture A Centre 1 2 3 Size 4 5 6 Yaw 0\nPiece patch Texture A Vertex 0 0 0 Vertex 1 0 0 Vertex 1 1 0\n");
        assert!(w.is_empty());
        for p in f.pieces {
            let mut q = p.clone();
            for (i, h) in handles(&p, 0.0).into_iter().enumerate() {
                set_handle(&mut q, i, h);
            }
            assert_eq!(p, q);
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
        assert_eq!(handles(&p, 0.0)[1], Vec3::new(7.0, 0.0, 8.0));
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
