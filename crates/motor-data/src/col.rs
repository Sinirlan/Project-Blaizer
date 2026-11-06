//! COL collision meshes: octree over a quantised voxel grid; faces are planes bounded by
//! edge planes. Loader 0x416d94, ray query 0x416840, voxel walk 0x415f78.

use crate::bytes::{err, slice, u16_, vec3, vec4, Result};

#[derive(Debug, Clone)]
pub struct Face {
    /// index into the track's .SRF material list
    pub material: u16,
    /// 1-based edge-plane indices, bit15 = flip sign, 0 = end of list
    pub edges: [u16; 4],
    /// The fifth slot (the record's "pad", +0x0a). The ray query reads it as a fifth edge plane
    /// (0x416c7d, slots 0..4): 34 faces use it, and after a 0 terminator it holds garbage that is
    /// never reached [O, all 89 files]. Kept apart from `edges` so the dump hashes stay the same.
    pub edge5: u16,
    pub plane: [f32; 4],
}

impl Face {
    /// The edge planes in the order the ray query tests them: five slots, a 0 ends the list.
    pub fn edge_slots(&self) -> impl Iterator<Item = u16> + '_ {
        self.edges.iter().copied().chain([self.edge5]).take_while(|&e| e != 0)
    }
}

#[derive(Debug, Clone)]
pub struct Collision {
    pub header_size: usize,
    pub shift: u16,
    pub extent: [u16; 3],
    pub offset: [f32; 3],
    pub scale: f32,
    pub inv_scale: f32,
    /// 0 = empty, bit15 = leaf index, else node index
    pub root: Vec<u16>,
    pub nodes: Vec<[u16; 8]>,
    pub leaf_words: Vec<u8>,
    pub faces: Vec<Face>,
    pub edges: Vec<[f32; 4]>,
}

/// Tolerance of the inside test in the ray query (0x4cfd17, a double).
pub const EDGE_EPS: f64 = -0.001;
/// Skin of the segment test (0x4cfcf7 = +0.08, 0x4cfcfb = -0.08, f32): a start point up to this far
/// behind a face's plane and an end point up to this far in front of it still count as a crossing.
const SKIN: f32 = 0.08;
/// The skin thresholds the start and end are compared with are doubles (0x4cfd07 = -0.08,
/// 0x4cfd0f = +0.08; 0x4cfccf / 0x4cfcdf in 0x416480).
const SKIN_LIMIT: f64 = 0.08;

// CLASSIC: the ray query runs on the x87 with the precision control at 53 bits: the game sets it
// to double at 0x41473e and 0x43a499 (0x405c88), and only the renderer 0x439e84 switches it to
// single (0x405c64) [O]. So sums are rounded to f64 and only stored values to f32. The f64
// arithmetic below follows the original's operation order and its store points.

/// `n . p + d` the way the original sums it: `((nx*px + d) + ny*py) + nz*pz` in f64.
fn plane_dist(n: [f32; 3], d: f32, p: [f32; 3]) -> f64 {
    let f = |v: f32| v as f64;
    f(n[0]) * f(p[0]) + f(d) + f(n[1]) * f(p[1]) + f(n[2]) * f(p[2])
}

/// The crossing of a segment with a plane, given the start's distance `da` (already stored to
/// f32, as the original does) and the end `b`, with the skin rules (0x416a7e..0x416be4,
/// 0x416486..0x416546) [O]. Returns `t` before it is stored (f64, 0 = start, 1 = `b`) and the
/// skin shift `extra` (0 or 0.08).
fn crossing(n: [f32; 3], d: f32, b: [f32; 3], da: f32) -> Option<(f64, f32)> {
    let (mut da, mut extra) = (da, 0.0f32);
    if da <= 0.0 && da as f64 > -SKIN_LIMIT {
        extra = SKIN;
        da = (da as f64 + SKIN as f64) as f32;
    }
    if da <= 0.0 {
        return None;
    }
    let mut db = (plane_dist(n, d, b) + extra as f64) as f32;
    if db > 0.0 && (db as f64) < SKIN_LIMIT {
        db = (db as f64 - SKIN as f64) as f32;
    }
    if db > 0.0 {
        return None;
    }
    Some((da as f64 / (da as f64 - db as f64), extra))
}

impl Collision {
    /// Face ids stored in leaf `index` (units of 4 bytes): u16 count, u16 ids...
    pub fn leaf(&self, index: usize) -> Result<Vec<u16>> {
        let p = index * 4;
        let n = u16_(&self.leaf_words, p)? as usize;
        (0..n).map(|k| u16_(&self.leaf_words, p + 2 + 2 * k)).collect()
    }

    /// Sorted, de-duplicated faces reachable from the root grid (validation helper).
    pub fn reachable_faces(&self) -> Result<Vec<u16>> {
        let mut seen = vec![false; self.faces.len()];
        let mut stack: Vec<u16> = self.root.iter().copied().filter(|&r| r != 0).collect();
        let mut visited = vec![false; self.nodes.len()];
        while let Some(x) = stack.pop() {
            if x & 0x8000 != 0 {
                for f in self.leaf((x & 0x7fff) as usize)? {
                    *seen.get_mut(f as usize).ok_or_else(|| crate::Error(format!("face {f} out of range")))? = true;
                }
            } else {
                let i = x as usize;
                let node = self.nodes.get(i).ok_or_else(|| crate::Error(format!("node {i} out of range")))?;
                if !std::mem::replace(&mut visited[i], true) {
                    stack.extend(node.iter().copied().filter(|&c| c != 0));
                }
            }
        }
        Ok((0..self.faces.len() as u16).filter(|&i| seen[i as usize]).collect())
    }

    /// Is `p` inside all edge half-spaces of `face`? The edge distance is summed in f64, stored
    /// to f32, its sign flipped for bit 15, and compared with the double `EDGE_EPS` (0x416ccc..0x416d19).
    pub fn inside(&self, face: &Face, p: [f32; 3]) -> bool {
        face.edge_slots().all(|e| {
            let Some(&[nx, ny, nz, d]) = self.edges.get(((e - 1) & 0x7fff) as usize) else { return false };
            let v = plane_dist([nx, ny, nz], d, p) as f32;
            let v = if e & 0x8000 != 0 { -v } else { v };
            v as f64 >= EDGE_EPS
        })
    }

    /// Quantised voxel coordinate of a world point (may be out of range).
    pub fn voxel(&self, p: [f32; 3]) -> [i32; 3] {
        let q = |k: usize| ((p[k] + self.offset[k]) * self.scale).floor() as i32;
        [q(0), q(1), q(2)]
    }

    /// Leaf index for a voxel: root grid cell, then octree descent (0x416840).
    pub fn leaf_at(&self, v: [i32; 3]) -> Option<usize> {
        if (0..3).any(|k| v[k] < 0 || v[k] >= self.extent[k] as i32) {
            return None;
        }
        let (x, y, z) = (v[0] as usize, v[1] as usize, v[2] as usize);
        let s = self.shift as usize;
        let (ey, ez) = ((self.extent[1] >> self.shift) as usize, (self.extent[2] >> self.shift) as usize);
        let mut n = *self.root.get(((x >> s) * ey + (y >> s)) * ez + (z >> s))?;
        let mut level = s;
        while n != 0 && n & 0x8000 == 0 {
            level = level.saturating_sub(1);
            let child = ((x >> level) & 1) * 4 + ((y >> level) & 1) * 2 + ((z >> level) & 1);
            n = self.nodes.get(n as usize)?[child];
        }
        (n != 0).then_some((n & 0x7fff) as usize)
    }

    /// Quantised voxel position of a world point, as a float (0x416840 at 0x4168cd..0x416969).
    fn voxel_f(&self, p: [f32; 3]) -> [f32; 3] {
        [(p[0] + self.offset[0]) * self.scale, (p[1] + self.offset[1]) * self.scale, (p[2] + self.offset[2]) * self.scale]
    }

    /// The voxels the segment `a` -> `b` (voxel units) passes through, in order from `a`,
    /// without those outside the grid. Transliteration of 0x415f78 [O]: coordinates are cut
    /// towards zero (`frndint` with round-to-zero, 0x4914be) and a negative one is then lowered
    /// by one; a segment whose cut start and end agree on all axes is the one voxel they name,
    /// negatives not adjusted. Otherwise every integer plane crossed becomes an event (t, axis,
    /// direction), the events are bubble-sorted by t (equal t keep x, y, z order) and replayed.
    pub fn walk(&self, a: [f32; 3], b: [f32; 3]) -> Vec<[i32; 3]> {
        let in_range = |v: [i32; 3]| (0..3).all(|k| v[k] >= 0 && v[k] < self.extent[k] as i32);
        let cut = |p: [f32; 3]| [p[0].trunc() as i32, p[1].trunc() as i32, p[2].trunc() as i32];
        let (va, vb) = (cut(a), cut(b));
        if va == vb {
            return if in_range(va) { vec![va] } else { Vec::new() };
        }
        let mut v = va;
        for k in 0..3 {
            if a[k] < 0.0 {
                v[k] -= 1;
            }
        }
        let d = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
        let mut events: Vec<(f32, usize, i32)> = Vec::new();
        for k in 0..3 {
            if d[k] > 0.0 {
                let mut plane = v[k] as f32 + 1.0;
                while b[k] > plane {
                    events.push(((plane - a[k]) / d[k], k, 1));
                    plane += 1.0;
                }
            } else if d[k] < 0.0 {
                let mut plane = v[k] as f32;
                while b[k] < plane {
                    events.push(((plane - a[k]) / d[k], k, -1));
                    plane -= 1.0;
                }
            }
        }
        events.sort_by(|x, y| x.0.partial_cmp(&y.0).unwrap_or(std::cmp::Ordering::Equal));
        let mut out = Vec::with_capacity(events.len() + 1);
        if in_range(v) {
            out.push(v);
        }
        for (_, k, step) in events {
            v[k] += step;
            if in_range(v) {
                out.push(v);
            }
        }
        out
    }

    /// Test one face against the segment `a` -> `b` (0x416a3e..0x416d29). `reach` = |b - a| +
    /// skin, `best_t` the best crossing of the current leaf. Both ends are moved by the skin
    /// (see `SKIN`); the hit point carries the same shift along the normal.
    fn face_hit(&self, f: u16, a: [f32; 3], b: [f32; 3], reach: f32, best_t: f32) -> Option<Hit> {
        let face = self.faces.get(f as usize)?;
        let [nx, ny, nz, d] = face.plane;
        let n = [nx, ny, nz];
        // the reach test compares the unrounded sum (0x416a7e `fst` keeps it in the register)
        let da = plane_dist(n, d, a);
        if da >= reach as f64 {
            return None;
        }
        let (t64, extra) = crossing(n, d, b, da as f32)?;
        // CLASSIC 0x416be9: the new t, still at double precision, against the leaf's best t as
        // stored (f32). For coplanar faces the later one wins when storing t rounded it up [O].
        if t64 >= best_t as f64 {
            return None;
        }
        let t = t64 as f32;
        let along = |k: usize| (b[k] - a[k]) * t + a[k];
        let shift = |k: usize, v: f32| (n[k] as f64 * extra as f64 + v as f64) as f32;
        let p = [shift(0, along(0)), shift(1, along(1)), shift(2, along(2))];
        self.inside(face, p).then_some(Hit { t, point: p, normal: n, d, material: face.material, face: f })
    }

    /// Re-test a segment `a` -> `b` against one plane (`normal`, `d`): transliteration of
    /// 0x416480 [O]. The skin rules of `face_hit` apply (a start within 0.08 m behind the plane
    /// counts, an end within 0.08 m in front counts), there is no reach test and no edge test.
    /// Returns `t` (0 = `a`, 1 = `b`). The car step uses it in its third wheel iteration for a
    /// wheel that touched in the second one (0x47eaa4).
    // CLASSIC 0x416480
    pub fn plane_hit(normal: [f32; 3], d: f32, a: [f32; 3], b: [f32; 3]) -> Option<f32> {
        crossing(normal, d, b, plane_dist(normal, d, a) as f32).map(|(t, _)| t as f32)
    }

    /// Diagnostic (`origreplay`, #402): how segment `a` -> `b` stands against face `f` alone,
    /// with the sums of `face_hit` but no voxel walk, reach or leaf best-t test.
    pub fn face_probe(&self, f: u16, a: [f32; 3], b: [f32; 3]) -> Option<FaceProbe> {
        let face = self.faces.get(f as usize)?;
        let [nx, ny, nz, d] = face.plane;
        let n = [nx, ny, nz];
        let (da, db) = (plane_dist(n, d, a), plane_dist(n, d, b));
        let cross = crossing(n, d, b, da as f32);
        let p = match cross {
            Some((t64, extra)) => {
                let t = t64 as f32;
                std::array::from_fn(|k| (n[k] as f64 * extra as f64 + ((b[k] - a[k]) * t + a[k]) as f64) as f32)
            }
            None => b,
        };
        let edge = face
            .edge_slots()
            .filter_map(|e| {
                let &[ex, ey, ez, ed] = self.edges.get(((e - 1) & 0x7fff) as usize)?;
                let v = plane_dist([ex, ey, ez], ed, p) as f32;
                Some(if e & 0x8000 != 0 { -v } else { v })
            })
            .fold(f32::INFINITY, f32::min);
        Some(FaceProbe { da, db, t: cross.map(|(t, _)| t as f32), edge, hit: cross.is_some() && edge as f64 >= EDGE_EPS })
    }

    /// Segment query from `a` to `b`: transliteration of 0x416840 [O]. The voxels along the
    /// segment (`walk`) are visited in order; each leaf's faces are tested (`face_hit`) and the
    /// leaf's nearest crossing (smallest t) is returned as soon as a leaf has one, so a nearer
    /// face that only a later voxel lists is not seen. An equal t keeps the earlier face unless
    /// storing t rounded it up (see `face_hit`): coplanar overlapping faces (27 pairs on Olympos)
    /// then return either one. `t` runs from `a` (0) to `b` (1). A zero-length segment hits nothing.
    // CLASSIC 0x416840
    pub fn raycast(&self, a: [f32; 3], b: [f32; 3]) -> Option<Hit> {
        // (dy² + dx²) + dz² in f64 (0x416870..0x416899); the zero test sees the unrounded root
        let sq = |k: usize| (b[k] as f64 - a[k] as f64).powi(2);
        let len = (sq(1) + sq(0) + sq(2)).sqrt();
        if len == 0.0 {
            return None;
        }
        let reach = (len as f32 as f64 + SKIN as f64) as f32;
        let mut tested: Vec<u16> = Vec::new();
        for v in self.walk(self.voxel_f(a), self.voxel_f(b)) {
            let Some(leaf) = self.leaf_at(v) else { continue };
            let mut best: Option<Hit> = None;
            for f in self.leaf(leaf).unwrap_or_default() {
                // a face rejected in an earlier leaf is rejected again (only the leaf-local
                // best t can reject it otherwise, and such a leaf returns): skip the repeat
                if tested.contains(&f) {
                    continue;
                }
                tested.push(f);
                if let Some(h) = self.face_hit(f, a, b, reach, best.map_or(10000.0, |h| h.t)) {
                    best = Some(h);
                }
            }
            if best.is_some() {
                return best;
            }
        }
        None
    }
}

#[derive(Debug, Clone, Copy)]
pub struct Hit {
    /// fraction along the segment
    pub t: f32,
    pub point: [f32; 3],
    pub normal: [f32; 3],
    /// the face's plane constant: `normal . p + d = 0` on the face
    pub d: f32,
    /// index into the track's .SRF material list
    pub material: u16,
    pub face: u16,
}

/// One segment against one face (`Collision::face_probe`).
#[derive(Debug, Clone, Copy)]
pub struct FaceProbe {
    /// plane distance of the start and the end (f64, as the ray query sums them)
    pub da: f64,
    pub db: f64,
    /// the plane crossing with the skin rules, if any
    pub t: Option<f32>,
    /// the smallest signed edge distance at the crossing point (at the end without a crossing);
    /// the inside test passes from `EDGE_EPS` up
    pub edge: f32,
    /// a crossing inside the face
    pub hit: bool,
}

pub fn parse(d: &[u8]) -> Result<Collision> {
    let hs = u16_(d, 0)? as usize;
    let shift = u16_(d, 2)?;
    let extent = [u16_(d, 4)?, u16_(d, 6)?, u16_(d, 8)?];
    let (n_faces, n_nodes, n_leafwords) = (u16_(d, 0xa)? as usize, u16_(d, 0xc)? as usize, u16_(d, 0xe)? as usize);
    if shift > 15 {
        return err("COL shift out of range");
    }
    let dims: usize = extent.iter().map(|&e| (e >> shift) as usize).product();
    if 0x28 + 2 * dims != hs {
        return err(format!("COL root grid ({dims} cells) does not fill header ({hs})"));
    }
    let root = (0..dims).map(|i| u16_(d, 0x28 + 2 * i)).collect::<Result<_>>()?;
    let o_leaves = hs + 16 * n_nodes;
    let o_faces = o_leaves + 4 * n_leafwords;
    let o_edges = o_faces + 28 * n_faces;
    let nodes = (0..n_nodes)
        .map(|i| {
            let mut c = [0u16; 8];
            for (k, v) in c.iter_mut().enumerate() {
                *v = u16_(d, hs + 16 * i + 2 * k)?;
            }
            Ok(c)
        })
        .collect::<Result<_>>()?;
    let faces = (0..n_faces)
        .map(|i| {
            let p = o_faces + 28 * i;
            Ok(Face {
                material: u16_(d, p)?,
                edges: [u16_(d, p + 2)?, u16_(d, p + 4)?, u16_(d, p + 6)?, u16_(d, p + 8)?],
                edge5: u16_(d, p + 10)?,
                plane: vec4(d, p + 12)?,
            })
        })
        .collect::<Result<_>>()?;
    if d.len() < o_edges || !(d.len() - o_edges).is_multiple_of(16) {
        return err("COL edge-plane section has a partial record");
    }
    let edges = (0..(d.len() - o_edges) / 16).map(|i| vec4(d, o_edges + 16 * i)).collect::<Result<_>>()?;
    Ok(Collision {
        header_size: hs,
        shift,
        extent,
        offset: vec3(d, 0x14)?,
        scale: crate::bytes::f32_(d, 0x20)?,
        inv_scale: crate::bytes::f32_(d, 0x24)?,
        root,
        nodes,
        leaf_words: slice(d, o_leaves, 4 * n_leafwords)?.to_vec(),
        faces,
        edges,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn grid(extent: [u16; 3]) -> Collision {
        Collision {
            header_size: 0,
            shift: 0,
            extent,
            offset: [0.0; 3],
            scale: 1.0,
            inv_scale: 1.0,
            root: Vec::new(),
            nodes: Vec::new(),
            leaf_words: Vec::new(),
            faces: Vec::new(),
            edges: Vec::new(),
        }
    }

    /// One voxel (0..1 on every axis) whose leaf lists two faces on the plane y = 0.5 without edge
    /// planes (inside everywhere): face 0 material 10, face 1 material 20.
    fn two_coplanar_faces() -> Collision {
        let mut c = grid([1, 1, 1]);
        c.root = vec![0x8000];
        c.leaf_words = [2u16, 0, 1, 0].iter().flat_map(|w| w.to_le_bytes()).collect();
        let face = |material| Face { material, edges: [0; 4], edge5: 0, plane: [0.0, 1.0, 0.0, -0.5] };
        c.faces = vec![face(10), face(20)];
        c
    }

    /// 0x416be9: the new t at double precision against the stored f32 best t. On coplanar faces
    /// the later face wins exactly when storing t rounded it up.
    #[test]
    fn coplanar_tie_follows_the_rounding_of_t() {
        let c = two_coplanar_faces();
        let (mut first, mut second) = (0, 0);
        for i in 0..2000 {
            let top = 0.52 + i as f32 * 0.0001;
            let (a, b) = ([0.5, top, 0.5], [0.5, 0.05, 0.5]);
            let hit = c.raycast(a, b).expect("the segment crosses the plane");
            let da = (top as f64 - 0.5) as f32 as f64;
            let db = (0.05f32 as f64 - 0.5) as f32 as f64;
            let t64 = da / (da - db);
            let want = if t64 < t64 as f32 as f64 { 20 } else { 10 };
            assert_eq!(hit.material, want, "top {top} t {t64}");
            if want == 10 {
                first += 1;
            } else {
                second += 1;
            }
        }
        // a coin flip per segment, as in the original's trace on Olympos (202 / 202)
        assert!(first > 600 && second > 600, "first {first} second {second}");
    }

    #[test]
    fn walk_follows_the_segment() {
        let c = grid([8, 8, 8]);
        assert_eq!(c.walk([0.5, 0.5, 0.5], [2.5, 0.5, 0.5]), vec![[0, 0, 0], [1, 0, 0], [2, 0, 0]]);
        assert_eq!(c.walk([2.5, 0.5, 0.5], [0.5, 0.5, 0.5]), vec![[2, 0, 0], [1, 0, 0], [0, 0, 0]]);
        // diagonal: the crossing nearer to the start comes first
        assert_eq!(c.walk([0.5, 0.5, 0.5], [1.5, 2.2, 0.5]), vec![[0, 0, 0], [0, 1, 0], [1, 1, 0], [1, 2, 0]]);
    }

    #[test]
    fn walk_single_voxel_and_range() {
        let c = grid([4, 4, 4]);
        assert_eq!(c.walk([1.2, 1.3, 1.4], [1.8, 1.7, 1.1]), vec![[1, 1, 1]]);
        // out of the grid: skipped, not clamped
        assert_eq!(c.walk([3.5, 0.5, 0.5], [5.5, 0.5, 0.5]), vec![[3, 0, 0]]);
        assert!(c.walk([7.5, 0.5, 0.5], [9.5, 0.5, 0.5]).is_empty());
        // 0x415f78 quirk: a segment inside (-1, 0) cuts to voxel 0 on every axis
        assert_eq!(c.walk([-0.8, 0.5, 0.5], [-0.2, 0.6, 0.5]), vec![[0, 0, 0]]);
        // otherwise a negative start is lowered: -0.5 is voxel -1, then voxel 0
        assert_eq!(c.walk([-0.5, 0.5, 0.5], [1.5, 0.5, 0.5]), vec![[0, 0, 0], [1, 0, 0]]);
    }
}
