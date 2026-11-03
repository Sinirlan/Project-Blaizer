//! REMASTER car bodies from the original mesh used as a cage (09-remaster 9.22). Phase 1: the
//! MYO body welded, its creases rounded by a small bevel, flat facets kept flat. Phase 2: the
//! windows recessed in a rubber frame (`inset_glass`). Phase 3: the lamps recessed the same way,
//! their insides in `super::lamps`. Phase 4: the vents too, insides in `super::vents`. Phase 5:
//! added parts (mirror copies, deeper scoops, closed wing halves on fins, exhausts, calipers,
//! carbon). Everything here is a design choice; the cage itself is the original [O].
//!
//! Input faces are in car-local game space; output is in view space (game z negated), CCW
//! from the front like `scene::emit`.

use super::lamps::Lamp;
use super::aero::{Fins, Foil, Splitter, Wing};
use super::cuts::{Chamfer, MirrorCopy, Paint, Panel, Pick, Push, Thicken};
use super::exhaust::Exhaust;
use super::smooth::Smooth;
use super::vents::Vent;
use crate::scene::Vertex;
use glam::{IVec3, Mat2, Vec2, Vec3};
use std::collections::{BTreeSet, HashMap, HashSet};

/// One original polygon, as `scene::emit` would draw it.
#[derive(Clone, Debug)]
pub struct Face {
    pub pts: Vec<[f32; 3]>,
    /// the MYO plane normal: the polygon's front
    pub normal: [f32; 3],
    /// per corner, already scaled like `scene::emit`
    pub colour: Vec<[f32; 4]>,
    pub uv: Vec<[f32; 2]>,
    /// texture array layer, -1 untextured
    pub layer: f32,
    pub env: f32,
    /// the MYO face colour before scaling (first corner), which marks the part (glass, lamps,
    /// vents); generated faces take their source's, the window rubber `RUBBER_RGB`
    pub rgb: [u8; 3],
    /// edges to faces with the same value flatter than this (degrees) are smooth; 0: the
    /// style's `smooth_deg` (`super::smooth` sets it on the panels it curves)
    pub smooth_deg: f32,
    /// REMASTER: lattice spacing (m) of the final triangulation, for a face a curved zone's
    /// outline cut (`super::smooth::split_edges`); 0 = ear clipping. Ear-clipped, the cut points
    /// fanned to the face's far corners in long slivers (the Titan's hood centre), and Gouraud
    /// shading swept the corners' normals across them as seams.
    pub remesh: f32,
}

/// Recessed windows (phase 2): the faces of colour `rgb` sink `inset` (m) along the vertex
/// normals inside a flush rubber frame `frame` (m) wide; the glass colour is scaled by `tint`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Glass {
    pub rgb: [u8; 3],
    pub frame: f32,
    pub inset: f32,
    pub tint: f32,
}

/// the raw colour tag of the window rubber faces
pub const RUBBER_RGB: [u8; 3] = [2, 2, 2];

/// How a car's cage is refined.
#[derive(Clone, Debug, PartialEq)]
pub struct BodyStyle {
    /// bevel width on each side of a crease (m), clamped on small faces
    pub bevel: f32,
    /// most segments across a crease's bevel; a strip takes the fewest that keep its curve within
    /// `bevel_tol` (a shallow fold or a tiny bevel needs one or two)
    pub segments: usize,
    /// largest gap (m) between a bevel's curve and its segments
    pub bevel_tol: f32,
    /// edges flatter than this (degrees) are shaded smooth and not bevelled
    pub smooth_deg: f32,
    /// round the wheel arches with a point every this many degrees (0: keep the original's)
    pub arch_step_deg: f32,
    /// gap between tyre and arch (m); 0 fits the circle to the original arch instead
    pub arch_clearance: f32,
    /// a flat lip round the arch's outer rim (m, in the side view; 0: none): the body side
    /// stops this much further out and a narrow band, sloping in towards the tunnel by half its
    /// width, joins it to the tunnel
    pub arch_lip: f32,
    /// the body sits this much lower than the original (MYO) body does on its wheels (m;
    /// negative: higher): a drawn ride height only, the physics and the original mesh keep
    /// theirs. The arches are cut round the wheels' rest position (`hubs`), so the gap stays
    /// `arch_clearance` whatever the drop
    pub drop: f32,
    /// x of the cage's mirror plane (m; 0 for most cars). [O] The Avalanche (06) is a clean
    /// mirror about x +3 mm, not 0. The generator works on the cage moved by -`centre`, so
    /// every |x| box of the recipe, the mirror copies and the curved zones' mirroring hold;
    /// the result is moved back (`scene::BodyJob::run`)
    pub centre: f32,
    /// recessed windows (None: the glass stays on the body's surface)
    pub glass: Option<Glass>,
    /// boxes (`lo`, `hi`; x as |x|: both sides) whose glass regions become one flat pane
    pub flat_glass: Vec<([f32; 3], [f32; 3])>,
    /// lamp housings and their insides (phase 3, `super::lamps`)
    pub lamps: Vec<Lamp>,
    /// recessed vents and grilles (phase 4, `super::vents`)
    pub vents: Vec<Vent>,
    /// cage edits before the glass and vents: cut corners, framed panels (`super::cuts`)
    pub chamfers: Vec<Chamfer>,
    pub panels: Vec<Panel>,
    /// a front splitter, a rear wing on pylons, diffuser fins (`super::aero`)
    pub splitter: Option<Splitter>,
    pub wing: Option<Wing>,
    pub fins: Option<Fins>,
    /// cage faces rebuilt as a lofted wing on struts, with the cage edits (`super::aero::foils`)
    pub foils: Vec<Foil>,
    /// faces repainted in another part's look, after the panels (`super::cuts::paint`)
    pub paints: Vec<Paint>,
    /// phase 5 cage edits, first of all: copies to the other side, moved points, thickened shells
    /// (`super::cuts`)
    pub mirror_copies: Vec<MirrorCopy>,
    pub pushes: Vec<Push>,
    pub thickens: Vec<Thicken>,
    /// the mirror glass's colour tag: shaded as polished metal
    pub mirror_glass: Option<[u8; 3]>,
    /// faces turned to carbon fibre (`super::cuts::carbon`)
    pub carbon: Vec<Pick>,
    /// exhaust tips (`super::exhaust`)
    pub exhaust: Option<Exhaust>,
    /// faceted panels curved into smooth ones (`super::smooth`)
    pub smooths: Vec<Smooth>,
    /// every edge cut and every face remeshed at this spacing (m; 0: ear clipping, the faces
    /// as they are), `super::smooth::refine`
    pub refine: f32,
}

impl BodyStyle {
    /// whether any part is carbon fibre (the weave's texture layers are needed)
    pub fn uses_carbon(&self) -> bool {
        !self.carbon.is_empty() || self.splitter.as_ref().is_some_and(|s| s.carbon) || self.wing.as_ref().is_some_and(|w| w.carbon) || self.fins.as_ref().is_some_and(|f| f.carbon)
    }
}

// The per-car tables (which car has a generated body, its glass, lamps, vents, cuts and
// smooth regions) are the recipe files `assets/cars/carNN.body`, read by `super::bodyfile`.

/// welding grid (m)
pub(super) const WELD: f32 = 1e-3;

pub(super) fn weld_key(p: [f32; 3]) -> [i32; 3] {
    p.map(|c| (c / WELD).round() as i32)
}

/// Drop the cage faces that repeat an earlier face: the same welded corners in the same winding
/// (any start corner). [O] The Serpent's windscreen triangle 95 repeats 89, so its three edges are
/// on three faces and the doubled pane came out as hundreds of non-manifold edges after the
/// refine. The same corners wound the other way are a two-sided panel and stay. Returns how many
/// went.
pub fn drop_repeats(faces: &mut Vec<Face>) -> usize {
    let mut seen = std::collections::HashSet::new();
    let n = faces.len();
    faces.retain(|f| {
        let mut k: Vec<[i32; 3]> = f.pts.iter().map(|&p| weld_key(p)).collect();
        let first = (0..k.len()).min_by_key(|&i| k[i]).unwrap_or(0);
        k.rotate_left(first);
        seen.insert(k)
    });
    n - faces.len()
}

/// Whether two face tags are the same colour for practical purposes: every channel within 2
/// (the near-black tags 0 0 0 and 1 1 1 are one colour to the eye).
fn near_tag(a: [u8; 3], b: [u8; 3]) -> bool {
    a.iter().zip(b).all(|(x, y)| x.abs_diff(y) <= 2)
}

/// Round the wheel arches of `faces` (game space, before `body`). [O] The Titan's arch is a
/// tunnel of flat black faces (the ceiling, front and back walls) between an outer rim on the
/// body side and an inner rim on the inner wall; seen from the side each rim is a 5-segment
/// trapezoid from the ground line over the wheel. Per hub: the tunnel faces are those near the
/// hub that face its axis; a circle (in z, y) goes through the rim's two ground ends and its top;
/// the rim corners move onto it, every face along a rim gets the arc points between its rim
/// corners, and the tunnel is rebuilt as a strip of quads. With `arch_clearance` the circle is
/// instead the tyre's plus that gap round the hub as it sits under the lowered body (`drop`),
/// the rims' ground ends sliding along the body's bottom line to meet it. With `arch_lip` the
/// outer rim sits on a circle that much larger and a strip of lip quads (the body's paint) runs
/// from it to the tunnel; edges from an outer rim end to an inner one (the underside) get the
/// lip's end point. `hubs` in car-local game space at rest, `radius` the wheel's.
pub fn round_arches(faces: &mut Vec<Face>, hubs: &[[f32; 3]], radius: f32, st: &BodyStyle) {
    let step_deg = st.arch_step_deg;
    if step_deg <= 0.0 {
        return;
    }
    for &h in hubs {
        let h = Vec3::from(h);
        let tunnel: Vec<usize> = (0..faces.len())
            .filter(|&i| {
                let f = &faces[i];
                let c = f.pts.iter().map(|&p| Vec3::from(p)).sum::<Vec3>() / f.pts.len() as f32;
                let to_axis = Vec3::new(0.0, h.y - c.y, h.z - c.z);
                c.x * h.x > 0.0 && to_axis.length() < radius + 0.3 && Vec3::from(f.normal).normalize_or_zero().dot(to_axis.normalize_or_zero()) > 0.5
            })
            .collect();
        // the tunnel is one colour: the most common tag among them [O] (the Resonic's mirror
        // housing sits over its front arch and its undersides face the hub too). [O] The Python's
        // front tunnels are black in two tags, 0 0 0 (4 faces) and 1 1 1 (the ceiling and the
        // rear wall's upper part, 2 faces), so a tag within 2 of the most common one counts
        // (an exact match left the two as flat faces: 95 non-manifold edges round the arch).
        let mut count: HashMap<[u8; 3], usize> = HashMap::new();
        tunnel.iter().for_each(|&i| *count.entry(faces[i].rgb).or_default() += 1);
        let Some(rgb) = count.into_iter().max_by_key(|&(rgb, n)| (n, rgb)).map(|c| c.0) else { continue };
        let tunnel: Vec<usize> = tunnel.into_iter().filter(|&i| near_tag(faces[i].rgb, rgb)).collect();
        // the rims: the tunnel's points nearer the body side and nearer the car's middle
        let mut pts: Vec<Vec3> = Vec::new();
        for &i in &tunnel {
            for &p in &faces[i].pts {
                if !pts.iter().any(|q| weld_key(q.to_array()) == weld_key(p)) {
                    pts.push(Vec3::from(p));
                }
            }
        }
        let (lo, hi) = pts.iter().fold((f32::MAX, 0.0f32), |(lo, hi), p| (lo.min(p.x.abs()), hi.max(p.x.abs())));
        let mid = (lo + hi) * 0.5;
        // angle round a centre in (z, y), 0 forward-level, increasing over the top
        let ang = |c: Vec2, p: Vec3| {
            let a = (p.y - c.y).atan2(p.z - c.x);
            if a < -std::f32::consts::FRAC_PI_2 { a + std::f32::consts::TAU } else { a }
        };
        let hub = Vec2::new(h.z, h.y);
        let rim = |outer: bool| {
            let mut r: Vec<Vec3> = pts.iter().copied().filter(|p| (p.x.abs() >= mid) == outer).collect();
            r.sort_by(|a, b| ang(hub, *a).total_cmp(&ang(hub, *b)));
            r
        };
        let rims = [rim(true), rim(false)];
        if rims.iter().any(|r| r.len() < 3) {
            continue;
        }
        // the circle: through the outer rim's ends and its top, or round the tyre
        let o = &rims[0];
        let top = o.iter().copied().max_by(|a, b| a.y.total_cmp(&b.y)).unwrap_or(o[0]);
        let fit = if st.arch_clearance > 0.0 {
            Some((Vec2::new(h.z, h.y + st.drop), radius + st.arch_clearance))
        } else {
            circle([o[0], top, o[o.len() - 1]].map(|p| Vec2::new(p.z, p.y)))
        };
        let Some((centre, r)) = fit else { continue };
        let at = |a: f32| (centre.y + r * a.sin(), centre.x + r * a.cos());
        // where a rim end meets the circle: on its own height, on its own side of the centre
        let end = |p: Vec3| {
            let dy = p.y - centre.y;
            let dz = (r * r - dy * dy).max(0.0).sqrt() * (p.z - centre.x).signum();
            ang(centre, Vec3::new(p.x, p.y, centre.x + dz))
        };
        let (a0, a1) = (end(o[0]), end(o[o.len() - 1]));
        if (o[0].y - centre.y).abs() >= r || (o[o.len() - 1].y - centre.y).abs() >= r {
            continue;
        }
        if rims[1].first().zip(rims[1].last()).is_none_or(|(s, e)| (end(*s) - a0).abs() > 0.02 || (end(*e) - a1).abs() > 0.02) || a1 <= a0 {
            continue;
        }
        let is_end = |p: Vec3| rims.iter().any(|rim| weld_key(rim[0].to_array()) == weld_key(p.to_array()) || weld_key(rim[rim.len() - 1].to_array()) == weld_key(p.to_array()));
        // a corner's angle on the circle: the ends where they meet it, the others radially
        let raw_ang = |p: Vec3| if is_end(p) { end(p) } else { ang(centre, p).clamp(a0, a1) };
        // an inner corner within a tenth of a step of an outer one takes its angle: the Titan's
        // rims meet the rear arch's top a millimetre apart [O], two arc points 0.55 mm apart that
        // a `Smooth` zone's cuts missed one of (a hole in the left rear fender, #291)
        let outer_angs: Vec<f32> = rims[0].iter().map(|p| raw_ang(*p)).collect();
        let snap = 0.1 * step_deg.to_radians();
        let corner_ang = |p: Vec3| {
            let a = raw_ang(p);
            outer_angs.iter().copied().filter(|o| (o - a).abs() < snap).min_by(|x, y| (x - a).abs().total_cmp(&(y - a).abs())).unwrap_or(a)
        };
        // angles of the arc points: even steps, snapped to the rims' own corners
        let corners: Vec<f32> = rims.iter().flatten().map(|p| corner_ang(*p)).collect();
        let n = ((a1 - a0) / step_deg.to_radians()).ceil().max(1.0) as usize;
        let mut angles: Vec<f32> = (0..=n).map(|i| a0 + (a1 - a0) * i as f32 / n as f32).filter(|a| corners.iter().all(|c| (c - a).abs() > 0.4 * step_deg.to_radians())).collect();
        angles.extend(&corners);
        angles.sort_by(f32::total_cmp);
        angles.dedup_by(|a, b| (*a - *b).abs() < 1e-3);
        // the rim's x at an angle, between its corners
        let x_at = |rim: &[Vec3], a: f32| {
            let c: Vec<(f32, f32)> = rim.iter().map(|p| (corner_ang(*p), p.x)).collect();
            let i = c.iter().position(|q| q.0 >= a).unwrap_or(c.len() - 1).max(1);
            let (p, q) = (c[i - 1], c[i]);
            p.1 + (q.1 - p.1) * ((a - p.0) / (q.0 - p.0).max(1e-6)).clamp(0.0, 1.0)
        };
        let lip = st.arch_lip.max(0.0);
        // a point at angle `a` on the outer rim's circle (radius r + lip); at the rim's ends it
        // stays on the body's bottom line. With a lip, angles past the outer rim's ends (the
        // inner rim's ends) clamp to them: a repeated point, so the strip there collapses.
        let at_outer = |a: f32| {
            if lip == 0.0 {
                return at(a);
            }
            let a = a.clamp(a0, a1);
            let (y, z) = at(a);
            let rr = r + lip;
            if (a - a0).abs() < 1e-4 || (a - a1).abs() < 1e-4 {
                let dy = y - centre.y;
                (y, centre.x + (rr * rr - dy * dy).max(0.0).sqrt() * (z - centre.x).signum())
            } else {
                (centre.y + rr * a.sin(), centre.x + rr * a.cos())
            }
        };
        let sx = rims[0][0].x.signum();
        // the lip's inner edge: on the tunnel's circle, in from the outer rim by half its width
        let lip_at = |a: f32| {
            let a = a.clamp(a0, a1);
            let (y, z) = at(a);
            Vec3::new(x_at(&rims[0], a) - sx * 0.5 * lip, y, z)
        };
        let arc: Vec<[Vec3; 2]> = angles
            .iter()
            .map(|&a| {
                let ((yo, zo), (y, z)) = (at_outer(a), at(a));
                [Vec3::new(x_at(&rims[0], a), yo, zo), Vec3::new(x_at(&rims[1], a), y, z)]
            })
            .collect();
        // move the rim corners onto the circle, everywhere
        let moved: HashMap<[i32; 3], (Vec3, usize, f32)> = rims
            .iter()
            .enumerate()
            .flat_map(|(k, rim)| rim.iter().map(move |p| (k, *p)))
            .map(|(k, p)| {
                let a = corner_ang(p);
                let (y, z) = if k == 0 { at_outer(a) } else { at(a) };
                (weld_key(p.to_array()), (Vec3::new(p.x, y, z), k, a))
            })
            .collect();
        let template = faces[tunnel[0]].clone();
        let mut keep = Vec::with_capacity(faces.len());
        for (i, f) in faces.drain(..).enumerate() {
            if tunnel.contains(&i) {
                continue;
            }
            let mut g = Face { pts: Vec::new(), colour: Vec::new(), uv: Vec::new(), ..f.clone() };
            let l = f.pts.len();
            for k in 0..l {
                let (p, q) = (moved.get(&weld_key(f.pts[k])), moved.get(&weld_key(f.pts[(k + 1) % l])));
                g.pts.push(p.map_or(f.pts[k], |m| m.0.to_array()));
                g.colour.push(f.colour[k]);
                g.uv.push(f.uv[k]);
                // a rim edge: the arc points between its two corners, in order
                let (Some(&(_, kp, ap)), Some(&(_, kq, aq))) = (p, q) else { continue };
                if kp != kq {
                    // from one rim's end to the other's: the lip's end comes between
                    if lip > 0.0 {
                        g.pts.push(lip_at(if kp == 0 { ap } else { aq }).to_array());
                        g.colour.push(f.colour[k]);
                        g.uv.push(((Vec2::from(f.uv[k]) + Vec2::from(f.uv[(k + 1) % l])) * 0.5).into());
                    }
                    continue;
                }
                let between = |a: f32| a > ap.min(aq) + 1e-3 && a < ap.max(aq) - 1e-3;
                // not a rim edge but a chord across the arch (the inner wall's top)
                if rims[kp].iter().any(|c| between(corner_ang(*c))) {
                    continue;
                }
                let mut ins: Vec<(f32, Vec3)> = angles.iter().zip(&arc).filter(|(a, _)| between(**a)).map(|(a, pq)| (*a, pq[kp])).collect();
                if aq < ap {
                    ins.reverse();
                }
                let (uk, uq) = (Vec2::from(f.uv[k]), Vec2::from(f.uv[(k + 1) % l]));
                for (a, pt) in ins {
                    let t = (a - ap) / (aq - ap);
                    g.pts.push(pt.to_array());
                    g.colour.push(f.colour[k]);
                    g.uv.push((uk + (uq - uk) * t).into());
                }
            }
            keep.push(g);
        }
        // the lip takes the look of the side panel along the outer rim
        let outer: HashSet<[i32; 3]> = moved.values().filter(|m| m.1 == 0).map(|m| weld_key(m.0.to_array())).collect();
        let panel = keep
            .iter()
            .filter_map(|f| f.pts.iter().position(|p| outer.contains(&weld_key(*p))).map(|k| (f, k)))
            .max_by(|a, b| a.0.normal[0].abs().total_cmp(&b.0.normal[0].abs()))
            .map(|(f, k)| (f.colour[k], f.uv[k], f.layer, f.env, f.rgb));
        *faces = keep;
        // the new quads wound CCW round their normal like the cage's faces: on the right the arc's
        // order runs the other way, and an edge shared with a panel in the same direction missed
        // the cut points a `Smooth` zone gives its outline (cracks along the lip, the Resonic, #173)
        let wound = |pts: [Vec3; 4], n: Vec3| {
            let newell: Vec3 = (0..4).map(|i| pts[i].cross(pts[(i + 1) % 4])).sum();
            let pts = if newell.dot(n) < 0.0 { [pts[3], pts[2], pts[1], pts[0]] } else { pts };
            pts.map(|p| p.to_array()).to_vec()
        };
        let lips: Vec<Vec3> = angles.iter().map(|&a| lip_at(a)).collect();
        if let (true, Some((colour, uv, layer, env, rgb))) = (lip > 0.0, panel) {
            for (w, m) in arc.windows(2).zip(lips.windows(2)) {
                let (o0, o1) = (w[0][0], w[1][0]);
                let c = (o0 + o1 + m[0] + m[1]) * 0.25;
                let inward = Vec3::new(0.0, centre.y - c.y, centre.x - c.z).normalize_or_zero();
                let normal = Vec3::X * sx * lip + inward * 0.5 * lip;
                faces.push(Face {
                    pts: wound([o0, o1, m[1], m[0]], normal),
                    normal: normal.to_array(),
                    colour: vec![colour; 4],
                    uv: vec![uv; 4],
                    layer,
                    env,
                    rgb,
                    smooth_deg: 0.0,
                    remesh: 0.0,
                });
            }
        }
        for (w, m) in arc.windows(2).zip(lips.windows(2)) {
            let (o0, o1) = if lip > 0.0 { (m[0], m[1]) } else { (w[0][0], w[1][0]) };
            let (i0, i1) = (w[0][1], w[1][1]);
            let c = (o0 + o1 + i0 + i1) * 0.25;
            faces.push(Face {
                pts: [o0, o1, i1, i0].map(|p| p.to_array()).to_vec(),
                normal: [0.0, centre.y - c.y, centre.x - c.z],
                colour: vec![template.colour[0]; 4],
                uv: vec![[0.0; 2]; 4],
                layer: -1.0,
                env: template.env,
                rgb: template.rgb,
                smooth_deg: 0.0,
                remesh: 0.0,
            });
        }
    }
}

/// Recess the windows (or a lamp housing) of `faces` (game space, before `body`). The glass faces of `g.rgb` form
/// regions (the Titan's are curved: 3–4 faces each, creased between them); each region's
/// boundary (edges to other faces or open) gets a flush rubber band `g.frame` wide in the
/// glass's planes, a rubber wall down `g.inset` and the glass below it, every point moved along
/// its vertex normal (the mean of the glass faces round it). A region whose centroid (x as |x|)
/// lies in one of the `flat` boxes becomes one pane instead, flat in the plane of its outline:
/// the frame's inner edge goes onto that plane and sinks along its normal, and the points
/// inside the outline are dropped (#291). Leaves the faces as they are if a boundary pinches
/// (two boundary edges leaving one point), or there are none, and returns false.
pub fn inset_glass(faces: &mut Vec<Face>, g: &Glass, flat: &[([f32; 3], [f32; 3])]) -> bool {
    if !faces.iter().any(|f| f.rgb == g.rgb) {
        return false;
    }
    // only a face with two corners on the glass can share an edge with it
    let corners: HashSet<[i32; 3]> = faces.iter().filter(|f| f.rgb == g.rgb).flat_map(|f| f.pts.iter().map(|&p| weld_key(p))).collect();
    let near = |f: &Face| f.rgb == g.rgb || f.pts.iter().filter(|&&p| corners.contains(&weld_key(p))).nth(1).is_some();
    // per face: the corners CCW round its plane normal, consecutive duplicates dropped (none:
    // a face away from the glass)
    let order: Vec<Vec<usize>> = faces
        .iter()
        .map(|f| {
            if !near(f) {
                return Vec::new();
            }
            let mut o: Vec<usize> = Vec::new();
            for k in 0..f.pts.len() {
                if o.last().is_none_or(|&l| weld_key(f.pts[l]) != weld_key(f.pts[k])) {
                    o.push(k);
                }
            }
            while o.len() > 1 && weld_key(f.pts[o[0]]) == weld_key(f.pts[o[o.len() - 1]]) {
                o.pop();
            }
            let mut nn = Vec3::ZERO;
            for i in 0..o.len() {
                let (a, b) = (Vec3::from(f.pts[o[i]]), Vec3::from(f.pts[o[(i + 1) % o.len()]]));
                nn += a.cross(b);
            }
            if nn.dot(Vec3::from(f.normal)) < 0.0 {
                o.reverse();
            }
            o
        })
        .collect();
    let glass: Vec<usize> = (0..faces.len()).filter(|&i| faces[i].rgb == g.rgb && order[i].len() >= 3).collect();
    if glass.is_empty() {
        return false;
    }
    let key = |i: usize, c: usize| weld_key(faces[i].pts[order[i][c % order[i].len()]]);
    let mut directed: HashMap<([i32; 3], [i32; 3]), usize> = HashMap::new();
    for (i, o) in order.iter().enumerate() {
        for c in 0..o.len() {
            directed.insert((key(i, c), key(i, c + 1)), i);
        }
    }
    let is_glass: HashSet<usize> = glass.iter().copied().collect();
    let normal = |i: usize| Vec3::from(faces[i].normal).normalize_or(Vec3::Y);
    // vertex normals, and the boundary edges by their start and end point
    let mut vn: HashMap<[i32; 3], Vec3> = HashMap::new();
    // point -> the other end of the boundary edge leaving / arriving at it, and the edge's face
    let mut out: HashMap<[i32; 3], (Vec3, usize)> = HashMap::new();
    let mut into: HashMap<[i32; 3], (Vec3, usize)> = HashMap::new();
    let mut pos: HashMap<[i32; 3], Vec3> = HashMap::new();
    let mut boundary: HashSet<(usize, usize)> = HashSet::new();
    for &i in &glass {
        for c in 0..order[i].len() {
            let (a, b) = (key(i, c), key(i, c + 1));
            *vn.entry(a).or_default() += normal(i);
            pos.insert(a, Vec3::from(faces[i].pts[order[i][c]]));
            if directed.get(&(b, a)).is_some_and(|j| is_glass.contains(j)) {
                continue;
            }
            boundary.insert((i, c));
            let (pa, pb) = (Vec3::from(faces[i].pts[order[i][c]]), Vec3::from(faces[i].pts[order[i][(c + 1) % order[i].len()]]));
            if out.insert(a, (pb, i)).is_some() || into.insert(b, (pa, i)).is_some() {
                return false;
            }
        }
    }
    // a boundary point moves `frame` inside both its boundary edges, in their faces' planes
    let w = g.frame;
    let inset: HashMap<[i32; 3], Vec3> = out
        .iter()
        .filter_map(|(k, &(next, f2))| {
            let &(prev, f1) = into.get(k)?;
            let v = pos[k];
            let n1 = normal(f1).cross((v - prev).normalize_or_zero());
            let n2 = normal(f2).cross((next - v).normalize_or_zero());
            let c = n1.dot(n2);
            let off = if c < -0.95 { n1 * w } else { (n1 + n2) * (w / (1.0 + c)) };
            Some((*k, v + off.clamp_length_max(4.0 * w)))
        })
        .collect();
    if inset.len() != out.len() {
        return false;
    }
    let mut inset = inset;
    // REMASTER flat panes (#291): the regions (glass faces joined by glass edges) in a `flat`
    // box, each with its outline loop and the plane through it (Newell's normal)
    let mut region: Vec<usize> = (0..faces.len()).collect();
    fn root(r: &mut [usize], mut i: usize) -> usize {
        while r[i] != i {
            r[i] = r[r[i]];
            i = r[i];
        }
        i
    }
    for &i in &glass {
        for c in 0..order[i].len() {
            if let Some(&j) = directed.get(&(key(i, c + 1), key(i, c))).filter(|j| is_glass.contains(j)) {
                let (a, b) = (root(&mut region, i), root(&mut region, j));
                region[a] = b;
            }
        }
    }
    let mut plane_of: HashMap<[i32; 3], Vec3> = HashMap::new();
    let mut panes: Vec<(usize, Vec<[i32; 3]>, Vec3)> = Vec::new();
    let mut flat_faces: HashSet<usize> = HashSet::new();
    let roots: BTreeSet<usize> = glass.iter().map(|&i| root(&mut region, i)).collect();
    for r in roots {
        let members: Vec<usize> = glass.iter().copied().filter(|&i| root(&mut region, i) == r).collect();
        let pts: Vec<Vec3> = members.iter().flat_map(|&i| faces[i].pts.iter().map(|&p| Vec3::from(p))).collect();
        let c = pts.iter().sum::<Vec3>() / pts.len() as f32;
        let c = Vec3::new(c.x.abs(), c.y, c.z);
        if !flat.iter().any(|&(lo, hi)| c.cmpge(Vec3::from(lo)).all() && c.cmple(Vec3::from(hi)).all()) {
            continue;
        }
        // the outline: one loop through every boundary point of the region, or no flat pane
        let starts: Vec<[i32; 3]> = out.iter().filter(|(_, v)| members.contains(&v.1)).map(|(k, _)| *k).collect();
        let Some(&first) = starts.iter().min() else { continue };
        let mut lp = vec![first];
        while let Some(&(next, _)) = out.get(&lp[lp.len() - 1]) {
            let nk = weld_key(next.to_array());
            if nk == first || lp.len() > starts.len() {
                break;
            }
            lp.push(nk);
        }
        if lp.len() != starts.len() || lp.len() < 3 {
            continue;
        }
        let n: Vec3 = (0..lp.len()).map(|i| pos[&lp[i]].cross(pos[&lp[(i + 1) % lp.len()]])).sum();
        let n = n.normalize_or_zero();
        if n == Vec3::ZERO {
            continue;
        }
        let o = lp.iter().map(|k| pos[k]).sum::<Vec3>() / lp.len() as f32;
        for k in &lp {
            let q = inset[k];
            inset.insert(*k, q - n * (q - o).dot(n));
            plane_of.insert(*k, n);
        }
        flat_faces.extend(&members);
        panes.push((members[0], lp, n));
    }
    let down = |k: &[i32; 3]| plane_of.get(k).copied().unwrap_or_else(|| vn[k].normalize_or(Vec3::Y)) * g.inset;
    let rubber_of = |f: &Face| {
        let m = f.colour[0][0].min(f.colour[0][1]).min(f.colour[0][2]) * 0.5;
        [m, m, m, 1.0]
    };
    let mut keep: Vec<Face> = Vec::new();
    for (i, lp, n) in &panes {
        let f = &faces[*i];
        // colour and uv of each outline point from the face whose boundary edge leaves it
        let corner = |k: &[i32; 3]| {
            let j = out[k].1;
            let c = (0..order[j].len()).find(|&c| key(j, c) == *k).unwrap_or(0);
            (faces[j].colour[order[j][c]], faces[j].uv[order[j][c]])
        };
        let pane = Face {
            pts: lp.iter().map(|k| (inset[k] - down(k)).to_array()).collect(),
            colour: lp.iter().map(|k| corner(k).0).map(|[r, gr, b, a]| [r * g.tint, gr * g.tint, b * g.tint, a]).collect(),
            uv: lp.iter().map(|k| corner(k).1).collect(),
            normal: n.to_array(),
            ..f.clone()
        };
        keep.push(pane);
    }
    for &i in &glass {
        let f = &faces[i];
        let o = &order[i];
        let rubber = rubber_of(f);
        let mut pane = Face { pts: Vec::new(), colour: Vec::new(), uv: Vec::new(), ..f.clone() };
        for (c, &oc) in o.iter().enumerate() {
            let k = key(i, c);
            let p = inset.get(&k).copied().unwrap_or(pos[&k]) - down(&k);
            pane.pts.push(p.to_array());
            let [r, gr, b, a] = f.colour[oc];
            pane.colour.push([r * g.tint, gr * g.tint, b * g.tint, a]);
            pane.uv.push(f.uv[oc]);
        }
        for c in 0..o.len() {
            if !boundary.contains(&(i, c)) {
                continue;
            }
            let (ka, kb) = (key(i, c), key(i, c + 1));
            let (a, b) = (pos[&ka], pos[&kb]);
            let (a1, b1) = (inset[&ka], inset[&kb]);
            let (a2, b2) = (a1 - down(&ka), b1 - down(&kb));
            let band = |pts: [Vec3; 4], n: Vec3| Face {
                pts: pts.map(|p| p.to_array()).to_vec(),
                normal: n.to_array(),
                colour: vec![rubber; 4],
                uv: vec![[0.0; 2]; 4],
                layer: -1.0,
                env: 0.0,
                rgb: RUBBER_RGB,
                smooth_deg: 0.0,
                remesh: 0.0,
            };
            keep.push(band([a, b, b1, a1], normal(i)));
            keep.push(band([a1, b1, b2, a2], plane_of.get(&ka).copied().unwrap_or(normal(i)).cross((b - a).normalize_or_zero())));
        }
        if !flat_faces.contains(&i) {
            keep.push(pane);
        }
    }
    // the other faces in their order, then the new ones
    let mut i = 0;
    faces.retain(|_| {
        i += 1;
        !is_glass.contains(&(i - 1))
    });
    faces.extend(keep);
    true
}

/// The circle through three points: centre, radius.
fn circle(p: [Vec2; 3]) -> Option<(Vec2, f32)> {
    let (b, c) = (p[1] - p[0], p[2] - p[0]);
    let d = 2.0 * b.perp_dot(c);
    if d.abs() < 1e-8 {
        return None;
    }
    let centre = p[0] + Vec2::new(c.y * b.length_squared() - b.y * c.length_squared(), b.x * c.length_squared() - c.x * b.length_squared()) / d;
    Some((centre, centre.distance(p[0])))
}

/// how far (m²) a point may lie outside an ear's triangle and still block it: f32 noise on
/// the perp products of a metre-long edge is about 1e-7
const EAR_EPS: f32 = 1e-7;

/// Ear-clipping triangulation of a simple polygon given CCW in 2D: at every step the ear with
/// the largest smallest angle goes. The first ear found fanned a long strip's corner across it
/// in slivers, and `smooth::delaunay`'s flips could not undo that: a corner joined to points
/// on a line (the cut points of one edge) is already locally Delaunay, and the zero-area ears
/// clipped between three such points blocked the rest. A straight corner is no ear while a
/// proper one is left; when none is (the leftover is a line, or a warped outline overlaps
/// itself) the corner spanning the most area goes, so the output is always n - 2 triangles.
pub(super) fn triangulate(p: &[Vec2]) -> Vec<[usize; 3]> {
    let n = p.len();
    if n < 3 {
        return Vec::new();
    }
    let mut idx: Vec<usize> = (0..n).collect();
    let mut out = Vec::with_capacity(n - 2);
    let corner = |idx: &[usize], i: usize| {
        let m = idx.len();
        (p[idx[(i + m - 1) % m]], p[idx[i]], p[idx[(i + 1) % m]])
    };
    // twice the signed area of corner i's triangle
    let area2 = |idx: &[usize], i: usize| {
        let (a, b, c) = corner(idx, i);
        (b - a).perp_dot(c - b)
    };
    // the ear quality of corner i: the sine of its triangle's smallest angle, 0 when it is no
    // ear (reflex or straight, or another corner lies in or on its triangle)
    let quality = |idx: &[usize], i: usize| -> f32 {
        let (a, b, c) = corner(idx, i);
        let (lab, lbc, lca) = ((b - a).length(), (c - b).length(), (a - c).length());
        // the smallest angle faces the shortest side: sin = 2 area / (the other two sides) [D]
        let sin = area2(idx, i) * lab.min(lbc).min(lca) / (lab * lbc * lca).max(1e-18);
        if sin <= 1e-6 {
            return 0.0;
        }
        let blocked = idx.iter().any(|&j| {
            let q = p[j];
            if q == a || q == b || q == c {
                return false;
            }
            (b - a).perp_dot(q - a) >= -EAR_EPS && (c - b).perp_dot(q - b) >= -EAR_EPS && (a - c).perp_dot(q - c) >= -EAR_EPS
        });
        if blocked {
            0.0
        } else {
            sin
        }
    };
    let mut q: Vec<f32> = (0..n).map(|i| quality(&idx, i)).collect();
    while idx.len() > 3 {
        let m = idx.len();
        let best = (0..m).filter(|&i| q[i] > 0.0).max_by(|&x, &y| q[x].total_cmp(&q[y]));
        let i = best.unwrap_or_else(|| (0..m).max_by(|&x, &y| area2(&idx, x).total_cmp(&area2(&idx, y))).expect("a corner"));
        out.push([idx[(i + m - 1) % m], idx[i], idx[(i + 1) % m]]);
        idx.remove(i);
        q.remove(i);
        let m = idx.len();
        for k in [(i + m - 1) % m, i % m] {
            q[k] = quality(&idx, k);
        }
    }
    out.push([idx[0], idx[1], idx[2]]);
    out
}

#[derive(Clone, Copy)]
enum Edge {
    Open,
    /// the twin's face and corner (its edge runs back from that corner)
    Smooth(usize, usize),
    Crease(usize, usize),
}

struct Cage {
    pos: Vec<Vec3>,
    /// welded corner ids, CCW around `n`
    v: Vec<Vec<usize>>,
    n: Vec<Vec3>,
    /// the input face and corner of each welded corner
    src: Vec<(usize, Vec<usize>)>,
    edge: Vec<Vec<Edge>>,
}

impl Cage {
    fn new(faces: &[Face], smooth_deg: f32) -> Self {
        let mut ids: HashMap<[i32; 3], usize> = HashMap::new();
        let mut pos = Vec::new();
        let (mut v, mut n, mut src) = (Vec::new(), Vec::new(), Vec::new());
        for (fi, f) in faces.iter().enumerate() {
            let mut loop_: Vec<(usize, usize)> = Vec::new();
            for (k, p) in f.pts.iter().enumerate() {
                let id = *ids.entry(weld_key(*p)).or_insert_with(|| {
                    pos.push(Vec3::from(*p));
                    pos.len() - 1
                });
                if loop_.last().is_none_or(|l| l.0 != id) {
                    loop_.push((id, k));
                }
            }
            while loop_.len() > 1 && loop_[0].0 == loop_[loop_.len() - 1].0 {
                loop_.pop();
            }
            if loop_.len() < 3 {
                continue;
            }
            // Newell normal, turned to the plane's front
            let mut nn = Vec3::ZERO;
            for i in 0..loop_.len() {
                let (a, b) = (pos[loop_[i].0], pos[loop_[(i + 1) % loop_.len()].0]);
                nn += Vec3::new((a.y - b.y) * (a.z + b.z), (a.z - b.z) * (a.x + b.x), (a.x - b.x) * (a.y + b.y));
            }
            if nn.length() < 1e-8 {
                continue;
            }
            if nn.dot(Vec3::from(f.normal)) < 0.0 {
                loop_.reverse();
                nn = -nn;
            }
            v.push(loop_.iter().map(|l| l.0).collect::<Vec<_>>());
            src.push((fi, loop_.iter().map(|l| l.1).collect()));
            n.push(nn.normalize());
        }
        let mut directed: HashMap<(usize, usize), (usize, usize)> = HashMap::new();
        for (f, l) in v.iter().enumerate() {
            for k in 0..l.len() {
                directed.entry((l[k], l[(k + 1) % l.len()])).or_insert((f, k));
            }
        }
        // two curved panels of one zone (`super::smooth`) take the zone's angle
        let cos = |f: usize, g: usize| {
            let (a, b) = (faces[src[f].0].smooth_deg, faces[src[g].0].smooth_deg);
            (if a > 0.0 && a == b { a } else { smooth_deg }).to_radians().cos()
        };
        let edge = v
            .iter()
            .enumerate()
            .map(|(f, l)| {
                (0..l.len())
                    .map(|k| match directed.get(&(l[(k + 1) % l.len()], l[k])) {
                        Some(&(g, j)) if g != f && n[f].dot(n[g]) >= cos(f, g) => Edge::Smooth(g, j),
                        Some(&(g, j)) if g != f => Edge::Crease(g, j),
                        _ => Edge::Open,
                    })
                    .collect()
            })
            .collect();
        Cage { pos, v, n, src, edge }
    }

    fn len(&self, f: usize) -> usize {
        self.v[f].len()
    }

    fn at(&self, f: usize, k: usize) -> Vec3 {
        self.pos[self.v[f][k % self.len(f)]]
    }

    /// the corner after `(f, k)` round its vertex, across the edge leaving it
    fn next_round(&self, f: usize, k: usize) -> Option<(usize, usize)> {
        match self.edge[f][k] {
            Edge::Smooth(g, j) | Edge::Crease(g, j) => Some((g, (j + 1) % self.len(g))),
            Edge::Open => None,
        }
    }

    /// the corner before `(f, k)` round its vertex, across the edge arriving at it
    fn prev_round(&self, f: usize, k: usize) -> Option<(usize, usize)> {
        match self.edge[f][(k + self.len(f) - 1) % self.len(f)] {
            Edge::Smooth(g, j) | Edge::Crease(g, j) => Some((g, j)),
            Edge::Open => None,
        }
    }
}

/// A face's texture mapping: uv = uv0 + duv · inv (q - q0), q the point in the plane basis
/// (origin `o`, axes `t`, `b`).
#[derive(Clone, Copy)]
struct UvMap {
    o: Vec3,
    t: Vec3,
    b: Vec3,
    q0: Vec2,
    inv: Mat2,
    uv0: Vec2,
    duv: [Vec2; 2],
}

/// The look of an output point: which face it belongs to.
struct Look<'a> {
    faces: &'a [Face],
    cage: &'a Cage,
    /// per cage face, textured ones only
    uvmap: Vec<Option<UvMap>>,
}

impl<'a> Look<'a> {
    fn new(faces: &'a [Face], cage: &'a Cage) -> Self {
        let uvmap = (0..cage.v.len())
            .map(|f| {
                let (fi, corners) = &cage.src[f];
                let face = &faces[*fi];
                if face.layer < 0.0 {
                    return None;
                }
                let o = cage.at(f, 0);
                let t = (cage.at(f, 1) - o).normalize();
                let b = cage.n[f].cross(t);
                let q = |k: usize| {
                    let d = cage.at(f, k) - o;
                    Vec2::new(d.dot(t), d.dot(b))
                };
                // the corner triple with the largest area
                let (mut best, mut area) = (2, 0.0);
                for k in 2..cage.len(f) {
                    let a = (q(1) - q(0)).perp_dot(q(k) - q(0)).abs();
                    if a > area {
                        (best, area) = (k, a);
                    }
                }
                let m = Mat2::from_cols(q(1) - q(0), q(best) - q(0));
                if m.determinant().abs() < 1e-10 {
                    return None;
                }
                let uv = |k: usize| Vec2::from(face.uv[corners[k]]);
                Some(UvMap { o, t, b, q0: q(0), inv: m.inverse(), uv0: uv(0), duv: [uv(1) - uv(0), uv(best) - uv(0)] })
            })
            .collect();
        Look { faces, cage, uvmap }
    }

    fn vertex(&self, f: usize, p: Vec3, n: Vec3) -> Vertex {
        let (fi, corners) = &self.cage.src[f];
        let face = &self.faces[*fi];
        // colour of the nearest corner (flat across the face for the paint types)
        let k = (0..corners.len()).min_by(|&a, &b| self.cage.at(f, a).distance_squared(p).total_cmp(&self.cage.at(f, b).distance_squared(p))).unwrap_or(0);
        let uv = self.uvmap[f].map_or([0.0; 2], |m| {
            let w = m.inv * (Vec2::new((p - m.o).dot(m.t), (p - m.o).dot(m.b)) - m.q0);
            (m.uv0 + m.duv[0] * w.x + m.duv[1] * w.y).into()
        });
        Vertex { pos: [p.x, p.y, -p.z], uv, color: face.colour[corners[k]], layer: face.layer, normal: [n.x, n.y, -n.z], env: face.env }
    }
}

/// The corners round the vertex of `(f, k)` joined by smooth edges, in order, if there are two
/// or more and a crease bounds them on both sides (a crease arrives at the first, one leaves
/// the last); `None` for a lone corner, a fully smooth vertex or one on an open edge.
fn crease_fan(cage: &Cage, f: usize, k: usize) -> Option<Vec<(usize, usize)>> {
    let before = |c: (usize, usize)| &cage.edge[c.0][(c.1 + cage.len(c.0) - 1) % cage.len(c.0)];
    let mut s = (f, k);
    // bounded: round a non-manifold vertex the walk can cycle without coming back
    for _ in 0..=cage.v.len() {
        if !matches!(before(s), Edge::Smooth(..)) {
            break;
        }
        s = cage.prev_round(s.0, s.1)?;
        if s == (f, k) {
            return None;
        }
    }
    if !matches!(before(s), Edge::Crease(..)) {
        return None;
    }
    let mut fan = vec![s];
    loop {
        let c = fan[fan.len() - 1];
        match cage.edge[c.0][c.1] {
            Edge::Crease(..) => break,
            Edge::Open => return None,
            Edge::Smooth(..) => {
                let nx = cage.next_round(c.0, c.1)?;
                if fan.contains(&nx) {
                    return None;
                }
                fan.push(nx);
            }
        }
    }
    (fan.len() >= 2 && fan.contains(&(f, k))).then_some(fan)
}

/// A point of a ring round a vertex: position, normal, owning cage face.
#[derive(Clone, Copy)]
struct Rp(Vec3, Vec3, usize);

/// The bevel strip across a cage edge, `body`: the twin face and corner, crease or smooth, its
/// two ends (from the face to the twin at each vertex), the ends' control points, and the
/// segments across it: what the bend asks for and the most its width allows.
type Strip = (usize, usize, bool, [(Rp, Rp, Vec3); 2], [Vec3; 2], (usize, usize));

/// Append a triangle in view space, wound to face along its normals; skips slivers.
pub(super) fn tri(out: &mut Vec<Vertex>, t: [Vertex; 3]) {
    let p = t.map(|v| Vec3::from(v.pos));
    let c = (p[1] - p[0]).cross(p[2] - p[0]);
    // under 0.1 mm² (`wire::TINY_AREA`): the fans at a straight cut point and the strips of
    // a collapsed corner are nothing to draw
    if c.length_squared() < 4e-14 {
        return;
    }
    let n: Vec3 = t.iter().map(|v| Vec3::from(v.normal)).sum();
    out.extend(if c.dot(n) < 0.0 { [t[0], t[2], t[1]] } else { t });
}

/// a polygon's area (m²): half the length of its Newell normal
fn face_area(f: &Face) -> f32 {
    let n = f.pts.len();
    (0..n).map(|i| Vec3::from(f.pts[i]).cross(Vec3::from(f.pts[(i + 1) % n]))).sum::<Vec3>().length() * 0.5
}

/// The refined body, or an empty list if nothing survives welding.
pub fn body(faces: &[Face], st: &BodyStyle) -> Vec<Vertex> {
    // REMASTER: a face of no area (the arch tunnels' leftovers, outlines on a line) draws
    // nothing and bred strips and fans of nothing round it
    let faces: Vec<Face> = faces.iter().filter(|f| face_area(f) > 1e-6).cloned().collect();
    let faces = &faces[..];
    let cage = Cage::new(faces, st.smooth_deg);
    let look = Look::new(faces, &cage);
    let nf = cage.v.len();
    let mut out = Vec::new();

    // Each face shrinks in its plane away from its crease edges by its own bevel width.
    let inward = |f: usize, k: usize| cage.n[f].cross((cage.at(f, k + 1) - cage.at(f, k)).normalize());
    // per face corner: a real corner (the outline bends there), not a cut point
    // REMASTER: the real corners, where the outline bends; the cut points of `refine` and
    // the zones' outlines lie straight on their edges
    let reals: Vec<Vec<bool>> = (0..nf)
        .map(|f| {
            let l = cage.len(f);
            (0..l).map(|k| inward(f, (k + l - 1) % l).cross(inward(f, k)).length() > 1e-3).collect()
        })
        .collect();
    // the face's outline inset by `d0`, halved up to `tries` times until every edge keeps its
    // way; the corners in `over` take that point instead. Also the width and whether it kept.
    let inset_face = |f: usize, d0: f32, tries: usize, over: &HashMap<(usize, usize), Vec3>| -> (Vec<Vec3>, f32, bool) {
        let l = cage.len(f);
        let inward = |k: usize| inward(f, k);
        let real: Vec<usize> = (0..l).filter(|&k| reals[f][k]).collect();
        let dk = |d: f32, k: usize| if matches!(cage.edge[f][k % l], Edge::Crease(..)) { d } else { 0.0 };
        let inset = |d: f32, k: usize| {
            if let Some(&q) = over.get(&(f, k)) {
                return q;
            }
            let (e1, e2) = ((k + l - 1) % l, k);
            let (n1, n2, d1, d2) = (inward(e1), inward(e2), dk(d, e1), dk(d, e2));
            // the point d1 inside the arriving edge and d2 inside the leaving one
            let c = n1.dot(n2);
            let det = 1.0 - c * c;
            let off = if det < 1e-4 { n1 * d1.max(d2) } else { n1 * ((d1 - c * d2) / det) + n2 * ((d2 - c * d1) / det) };
            cage.at(f, k) + off.clamp_length_max(4.0 * d1.max(d2))
        };
        let shrink = |d: f32| {
            let mut q: Vec<Vec3> = (0..l).map(|k| inset(d, k)).collect();
            // a straight point of a crease edge keeps its own offset but stays on the inset
            // chain between the real corners either side of it: offset d by itself it overshot
            // a clamped acute tip and crossed the other side (the cut points nearest the tip of
            // a pointed panel); projected onto the chain it lands on the tip instead, and where
            // the chain lies on the offset line it does not move, so its twin across the edge
            // keeps its place. The points of a smooth edge stay on it, like their twins.
            if !real.is_empty() {
                for (i, &a) in real.iter().enumerate() {
                    let b = real[(i + 1) % real.len()];
                    if dk(d, a) == 0.0 {
                        continue;
                    }
                    let span = if b > a { b - a } else { b + l - a };
                    let (qa, qb) = (q[a], q[b % l]);
                    let len2 = qa.distance_squared(qb);
                    for j in 1..span {
                        let k = (a + j) % l;
                        if over.contains_key(&(f, k)) {
                            continue;
                        }
                        let t = if len2 > 1e-12 { ((q[k] - qa).dot(qb - qa) / len2).clamp(0.0, 1.0) } else { 0.0 };
                        q[k] = qa.lerp(qb, t);
                    }
                }
            }
            q
        };
        let kept = |q: &[Vec3]| (0..l).all(|k| (q[(k + 1) % l] - q[k]).dot(cage.at(f, k + 1) - cage.at(f, k)) > 0.0);
        let mut d = d0;
        let mut q = shrink(d);
        for _ in 0..tries {
            if kept(&q) {
                break;
            }
            d *= 0.5;
            q = shrink(d);
        }
        let ok = kept(&q);
        (q, d, ok)
    };
    // the widest inset that keeps every edge running its way: shrinking both sides of a
    // face narrower than twice the bevel crossed them over (the Titan's trim strip beside
    // the rear haunch, 1 cm wide at its ends, came out a bow-tie of slivers)
    let none = HashMap::new();
    let (mut p, width): (Vec<Vec<Vec3>>, Vec<f32>) = (0..nf)
        .map(|f| {
            let l = cage.len(f);
            let centre = (0..l).map(|k| cage.at(f, k)).sum::<Vec3>() / l as f32;
            let inradius = (0..l).map(|k| (centre - cage.at(f, k)).dot(inward(f, k))).fold(f32::MAX, f32::min);
            let (q, d, _) = inset_face(f, st.bevel.min(0.35 * inradius.max(0.0)), 4, &none);
            (q, d)
        })
        .unzip();
    // REMASTER: the faces of a smooth fan round a crease vertex (a zone's 2.5 cm triangles
    // along the ASC's and the Resonic's arch lips) share one inset point: each set its own
    // along its smooth edge, or kept the vertex where only its corner touched the crease, and
    // the strips between them had no area, a T-junction at every vertex of the zone's border
    let mut fans: Vec<Vec3> = Vec::new();
    let mut fan_of: HashMap<(usize, usize), usize> = HashMap::new();
    for f in 0..nf {
        for k in 0..cage.len(f) {
            if fan_of.contains_key(&(f, k)) {
                continue;
            }
            let Some(fan) = crease_fan(&cage, f, k) else { continue };
            let v = cage.at(f, k);
            let mut seen: Vec<usize> = Vec::new();
            for &(g, _) in &fan {
                if !seen.contains(&g) {
                    seen.push(g);
                }
            }
            let n = seen.iter().map(|&g| cage.n[g]).sum::<Vec3>().normalize_or(cage.n[f]);
            let d = seen.iter().map(|&g| width[g]).fold(f32::MAX, f32::min);
            let (s, e) = (fan[0], fan[fan.len() - 1]);
            // the bounding creases arriving at the fan's first corner and leaving its last, in
            // the fan's mean plane, and the point d inside both
            let dir1 = (v - cage.at(s.0, s.1 + cage.len(s.0) - 1)).reject_from_normalized(n).normalize_or_zero();
            let dir2 = (cage.at(e.0, e.1 + 1) - v).reject_from_normalized(n).normalize_or_zero();
            let (n1, n2) = (n.cross(dir1), n.cross(dir2));
            let c = n1.dot(n2);
            let det = 1.0 - c * c;
            let off = if det < 1e-4 { n1 * d } else { (n1 + n2) * (d * (1.0 - c) / det) };
            for &corner in &fan {
                fan_of.insert(corner, fans.len());
            }
            fans.push(v + off.clamp_length_max(4.0 * d));
        }
    }
    // a face that would turn an edge over with its fans' points keeps its own, and its fans
    // theirs: a T-junction there beats a folded triangle
    let mut alive = vec![true; fans.len()];
    let mut touched: Vec<usize> = fan_of.keys().map(|c| c.0).collect();
    touched.sort_unstable();
    touched.dedup();
    let fan_of = &fan_of;
    let fans_of = |f: usize| (0..cage.len(f)).filter_map(move |k| fan_of.get(&(f, k)).copied());
    let mut moved: Vec<(usize, Vec<Vec3>)> = Vec::new();
    // ends: a round that finds a bad face retires at least one live fan
    loop {
        let over: HashMap<(usize, usize), Vec3> = fan_of.iter().filter(|(_, &i)| alive[i]).map(|(&c, &i)| (c, fans[i])).collect();
        moved.clear();
        let mut bad = false;
        for &f in &touched {
            if !fans_of(f).any(|i| alive[i]) {
                continue;
            }
            let (q, _, ok) = inset_face(f, width[f], 0, &over);
            if !ok {
                for i in fans_of(f) {
                    alive[i] = false;
                }
                bad = true;
            }
            moved.push((f, q));
        }
        if !bad {
            break;
        }
    }
    for (f, q) in moved {
        if fans_of(f).any(|i| alive[i]) {
            p[f] = q;
        }
    }
    // Corner normals: the face normals of the smooth fan round each corner.
    let normal: Vec<Vec<Vec3>> = (0..nf)
        .map(|f| {
            (0..cage.len(f))
                .map(|k| {
                    // a fan has a few faces: a list beats a set
                    let mut seen = vec![f];
                    let mut sum = cage.n[f];
                    for forward in [true, false] {
                        let mut c = (f, k);
                        loop {
                            let e = if forward { cage.edge[c.0][c.1] } else { cage.edge[c.0][(c.1 + cage.len(c.0) - 1) % cage.len(c.0)] };
                            let next = if forward { cage.next_round(c.0, c.1) } else { cage.prev_round(c.0, c.1) };
                            match (e, next) {
                                (Edge::Smooth(..), Some(nx)) if !seen.contains(&nx.0) => {
                                    seen.push(nx.0);
                                    sum += cage.n[nx.0];
                                    c = nx;
                                }
                                _ => break,
                            }
                        }
                    }
                    sum.normalize()
                })
                .collect()
        })
        .collect();

    // Faces (a rounded arch leaves the panels round it concave).
    for f in 0..nf {
        // the first edge laid into the plane: on a warped face (a window twists 13 mm) the
        // edge itself leans out of it, and the skewed axes folded the outline over in 2D
        let t = (p[f][1] - p[f][0]).reject_from_normalized(cage.n[f]).try_normalize().unwrap_or_else(|| cage.n[f].any_orthonormal_vector());
        let b = cage.n[f].cross(t);
        let flat: Vec<Vec2> = p[f].iter().map(|q| Vec2::new((*q - p[f][0]).dot(t), (*q - p[f][0]).dot(b))).collect();
        let h = faces[cage.src[f].0].remesh;
        if h > 0.0 && p[f].len() > 3 {
            // REMASTER: a face a zone's outline cut or `refine` marked (`Face::remesh`) gets a
            // lattice inside and a Delaunay triangulation instead of ear clipping's slivers fanning to its far corners;
            // an inner point shades by the corners' normals through mean value coordinates: the
            // gradient Gouraud spread over the fan, without the fan's seams
            let (pts, tris) = super::smooth::flat_mesh(&p[f], cage.n[f], h);
            let vert: Vec<Vertex> = pts
                .iter()
                .map(|&(q, corner)| match corner {
                    Some(k) => look.vertex(f, q, normal[f][k]),
                    None => {
                        let w = super::smooth::mean_value(&flat, Vec2::new((q - p[f][0]).dot(t), (q - p[f][0]).dot(b)));
                        let n = w.iter().zip(&normal[f]).map(|(&w, &n)| n * w).sum::<Vec3>().normalize_or(cage.n[f]);
                        // the point by the same weights: on a flat face the lattice's own, on a
                        // warped one (the pushed scoop walls, the nose's side panels twist 2 cm)
                        // it follows the corners instead of the first corner's plane
                        let mv: Vec3 = w.iter().zip(&p[f]).map(|(&w, &c)| c * w).sum();
                        look.vertex(f, mv, n)
                    }
                })
                .collect();
            for t in tris {
                tri(&mut out, t.map(|i| vert[i]));
            }
            continue;
        }
        for t in triangulate(&flat) {
            tri(&mut out, t.map(|i| look.vertex(f, p[f][i], normal[f][i])));
        }
    }

    // Strips between neighbouring faces: a quadratic bevel across a crease (control point on
    // the original edge), a straight quad across a smooth edge. `curve[(f, k)]` keeps the
    // strip's end at corner (f, k), from f to the neighbour, for the corner fans.
    // the strip across edge (f, k) to its twin (g, j): `ends` from f to g at the edge's two
    // vertices, the curves' control points, and the segment count the bend asks for
    let strip = |f: usize, k: usize| -> Option<Strip> {
        let l = cage.len(f);
        let (g, j, crease) = match cage.edge[f][k] {
            Edge::Crease(g, j) => (g, j, true),
            Edge::Smooth(g, j) => (g, j, false),
            Edge::Open => return None,
        };
        if (g, j) < (f, k) {
            return None;
        }
        let lg = cage.len(g);
        let (k1, j1) = ((k + 1) % l, (j + 1) % lg);
        let (a, b) = (cage.at(f, k), cage.at(f, k1));
        let dir = (b - a).normalize();
        // from f to g at vertex a, and at vertex b
        let ends = [(Rp(p[f][k], normal[f][k], f), Rp(p[g][j1], normal[g][j1], g), a), (Rp(p[f][k1], normal[f][k1], f), Rp(p[g][j], normal[g][j], g), b)];
        let ctrls = ends.map(|(x, y, v)| {
            let mid = (x.0 + y.0) * 0.5;
            if crease { v + dir * dir.dot(mid - v) } else { mid }
        });
        // REMASTER: the fewest segments within `bevel_tol` of the curve: a quadratic's gap to
        // its n chords is |x - 2c + y| / (4n²) [D]
        let segs = if crease {
            let bend = ends.iter().zip(&ctrls).map(|(e, c)| (e.0 .0 - *c * 2.0 + e.1 .0).length()).fold(0.0, f32::max);
            let tol = st.bevel_tol.max(1e-6);
            let need = ((bend / (4.0 * tol)).sqrt().ceil() as usize).clamp(1, st.segments.max(1));
            // REMASTER: a small bevel, one whose single chord is within 1.5 tol of the curve,
            // takes no more segments than keep each one's triangles off slivers (an angle under
            // 5 deg, `wire::SLIVER_DEG`): the window rubber's 4 mm bevels along the 4 cm cut
            // edges came out 2 mm by 40 mm in two segments. A wide bevel keeps its accuracy: on
            // a long uncut edge its triangles are long whatever the count
            let width = ends.iter().map(|(x, y, _)| x.0.distance(y.0)).fold(f32::MAX, f32::min);
            let cap = if bend <= 6.0 * tol { ((width / ((b - a).length() * 0.0875)).floor() as usize).max(1) } else { usize::MAX };
            (need, cap)
        } else {
            (1, usize::MAX)
        };
        Some((g, j, crease, ends, ctrls, segs))
    };
    // REMASTER: the sub-edges of one crease (the cut points of `refine` lie straight on it)
    // share one segment count, as does the twin side: the most the bend asks for along it,
    // within what its narrowest sub-edge allows. At a cut point between two and one segments
    // the corner fan filled the step with slivers
    let base: Vec<usize> = (0..nf).scan(0, |acc, f| { let b = *acc; *acc += cage.len(f); Some(b) }).collect();
    let total = base.last().map_or(0, |b| b + cage.len(nf - 1));
    let mut parent: Vec<usize> = (0..total).collect();
    fn root(parent: &mut [usize], mut i: usize) -> usize {
        while parent[i] != i {
            parent[i] = parent[parent[i]];
            i = parent[i];
        }
        i
    }
    let mut need_of = vec![0usize; total];
    let mut cap_of = vec![usize::MAX; total];
    for (f, &bf) in base.iter().enumerate() {
        let l = cage.len(f);
        for k in 0..l {
            let (g, j, crease, _, _, (need, cap)) = match strip(f, k) {
                Some(s) => s,
                None => continue,
            };
            need_of[bf + k] = need;
            cap_of[bf + k] = cap;
            let (rf, rg) = (root(&mut parent, bf + k), root(&mut parent, base[g] + j));
            parent[rf] = rg;
            if !crease {
                continue;
            }
            for (h, e, next) in [(f, k, (k + 1) % l), (g, j, (j + 1) % cage.len(g))] {
                // straight on through the corner after it: the same crease
                if !reals[h][next] && matches!(cage.edge[h][next], Edge::Crease(..)) {
                    let (r0, r1) = (root(&mut parent, base[h] + e), root(&mut parent, base[h] + next));
                    parent[r0] = r1;
                }
            }
        }
    }
    let mut class_need = vec![0usize; total];
    let mut class_cap = vec![usize::MAX; total];
    for i in 0..total {
        let r = root(&mut parent, i);
        class_need[r] = class_need[r].max(need_of[i]);
        class_cap[r] = class_cap[r].min(cap_of[i]);
    }
    let mut curve: HashMap<(usize, usize), Vec<Rp>> = HashMap::new();
    for (f, &bf) in base.iter().enumerate() {
        let l = cage.len(f);
        for k in 0..l {
            let (g, j, _, ends, ctrls, _) = match strip(f, k) {
                Some(s) => s,
                None => continue,
            };
            let r = root(&mut parent, bf + k);
            let segs = class_need[r].min(class_cap[r]).max(1);
            // segment i of the strip belongs to the nearer face
            let owner = |i: usize| if (i as f32 + 0.5) / (segs as f32) < 0.5 { f } else { g };
            let sides: Vec<Vec<Rp>> = ends
                .iter()
                .zip(ctrls)
                .map(|(&(x, y, _), ctrl)| {
                    (0..=segs)
                        .map(|i| {
                            let t = i as f32 / segs as f32;
                            let pos = x.0 * (1.0 - t) * (1.0 - t) + ctrl * 2.0 * t * (1.0 - t) + y.0 * t * t;
                            let n = (x.1 * (1.0 - t) + y.1 * t).normalize_or(x.1);
                            Rp(pos, n, owner(i.min(segs - 1)))
                        })
                        .collect()
                })
                .collect();
            for i in 0..segs {
                let q = [sides[0][i], sides[1][i], sides[1][i + 1], sides[0][i + 1]].map(|r| look.vertex(owner(i), r.0, r.1));
                tri(&mut out, [q[0], q[1], q[2]]);
                tri(&mut out, [q[0], q[2], q[3]]);
            }
            // a ring point owns the fan wedge to the next point: so the reversed end's points
            // own the segment before them
            let mut back = sides[1].clone();
            back.reverse();
            for (i, r) in back.iter_mut().enumerate().take(segs) {
                r.2 = owner(segs - 1 - i);
            }
            curve.insert((f, k), sides[0].clone());
            curve.insert((g, j), back);
        }
    }

    // Corner fans: the ring of face corners and strip ends round each welded vertex.
    let mut done: HashSet<(usize, usize)> = HashSet::new();
    for f in 0..nf {
        for k in 0..cage.len(f) {
            if done.contains(&(f, k)) {
                continue;
            }
            let mut start = (f, k);
            let mut closed = false;
            // bounded: round a non-manifold vertex the walk can cycle without coming back
            for _ in 0..nf {
                let Some(prev) = cage.prev_round(start.0, start.1) else { break };
                start = prev;
                if start == (f, k) {
                    closed = true;
                    break;
                }
            }
            let mut ring: Vec<Rp> = Vec::new();
            // middles of the bevels meeting here and their edges' directions: the fan's centre
            // stays on the creases
            let mut mids: Vec<(Vec3, Vec3)> = Vec::new();
            let mut c = start;
            loop {
                if !done.insert(c) {
                    break;
                }
                ring.push(Rp(p[c.0][c.1], normal[c.0][c.1], c.0));
                if let Some(s) = curve.get(&c) {
                    ring.extend_from_slice(&s[1..s.len() - 1]);
                    if s.len() > 2 {
                        mids.push((s[s.len() / 2].0, (cage.at(c.0, c.1 + 1) - cage.at(c.0, c.1)).normalize()));
                    }
                }
                match cage.next_round(c.0, c.1) {
                    Some(nx) if nx != start => c = nx,
                    _ => break,
                }
            }
            if ring.len() < 2 {
                continue;
            }
            let v = cage.at(f, k);
            // a crease running straight on through the vertex (others ending on it) keeps it
            let through = mids.iter().enumerate().flat_map(|(i, a)| mids[i + 1..].iter().map(move |b| (a, b))).filter(|(a, b)| a.1.dot(b.1) < -0.95).min_by(|x, y| x.0 .1.dot(x.1 .1).total_cmp(&y.0 .1.dot(y.1 .1)));
            let apex = match (closed, through, mids.len()) {
                (false, ..) => v,
                (true, Some((a, b)), _) => (a.0 + b.0) * 0.5,
                (true, None, 0) => v * 0.5 + ring.iter().map(|r| r.0).sum::<Vec3>() / ring.len() as f32 * 0.5,
                (true, None, m) => mids.iter().map(|m| m.0).sum::<Vec3>() / m as f32,
            };
            let an = ring.iter().map(|r| r.1).sum::<Vec3>().normalize_or(cage.n[f]);
            let n = if closed { ring.len() } else { ring.len() - 1 };
            // two strips can end on the same curve (an inner mitre, like a window frame's
            // corner): the wedges to it from both sides cancel, so neither is drawn
            let pair = |i: usize| {
                let (k0, k1) = (weld_key(ring[i].0.to_array()), weld_key(ring[(i + 1) % ring.len()].0.to_array()));
                if k0 < k1 { [k0, k1] } else { [k1, k0] }
            };
            let mut uses: HashMap<[[i32; 3]; 2], usize> = HashMap::new();
            for i in 0..n {
                *uses.entry(pair(i)).or_default() += 1;
            }
            for i in 0..n {
                let (r0, r1) = (ring[i], ring[(i + 1) % ring.len()]);
                if uses[&pair(i)] > 1 {
                    continue;
                }
                let o = r0.2;
                tri(&mut out, [look.vertex(o, apex, an), look.vertex(o, r0.0, r0.1), look.vertex(o, r1.0, r1.1)]);
            }
        }
    }
    // REMASTER: the insets of two faces round a corner, or a strip's end and a face's corner,
    // can land a fraction of a millimetre apart: the fans and strips between them were slivers
    // with an edge of nothing (the Titan's scoop frames, its exhaust recess)
    merge_close(&mut out, MERGE);
    out
}

/// points closer than this (m) are one (`merge_close`); the narrowest real detail, a clamped
/// bevel's segment, is 1.4 mm
const MERGE: f32 = 5e-4;

/// Merge by distance: every point of the triangle list `tris` within `eps` of an earlier kept
/// one moves onto it (a shared point moves alike in every triangle), unless that turns a
/// triangle that stays over (or away from its normals) or flattens it; the triangles that
/// collapse are dropped.
fn merge_close(tris: &mut Vec<Vertex>, eps: f32) {
    let key = |p: [f32; 3]| p.map(f32::to_bits);
    let cell = |p: Vec3| (p / eps).floor().as_ivec3();
    let mut to: HashMap<[u32; 3], Vec3> = HashMap::new();
    let mut kept: HashMap<IVec3, Vec<Vec3>> = HashMap::new();
    for v in tris.iter() {
        if to.contains_key(&key(v.pos)) {
            continue;
        }
        let p = Vec3::from(v.pos);
        let c = cell(p);
        let near = (0..27).map(|i| c + IVec3::new(i % 3 - 1, i / 3 % 3 - 1, i / 9 - 1)).find_map(|n| kept.get(&n)?.iter().copied().find(|q| q.distance(p) < eps));
        if near.is_none() {
            kept.entry(c).or_default().push(p);
        }
        to.insert(key(v.pos), near.unwrap_or(p));
    }
    // undo the moves that turn a triangle over or leave it under 0.1 mm² (`tri`), until none
    // does: an undone point can leave another triangle uncollapsed
    loop {
        let mut undo = Vec::new();
        for t in tris.as_chunks::<3>().0 {
            let old = [0, 1, 2].map(|k| Vec3::from(t[k].pos));
            let new = [0, 1, 2].map(|k| to[&key(t[k].pos)]);
            if new[0] == new[1] || new[1] == new[2] || new[2] == new[0] {
                continue;
            }
            let (was, now) = ((old[1] - old[0]).cross(old[2] - old[0]), (new[1] - new[0]).cross(new[2] - new[0]));
            // facing its normals as `tri` wound it (`wire::analyse` checks the same)
            let vn: Vec3 = t.iter().map(|v| Vec3::from(v.normal)).sum();
            if now.dot(was) <= 0.0 || (now.dot(vn) < 0.0 && was.dot(vn) >= 0.0) || now.length_squared() < 4e-14 {
                undo.extend((0..3).filter(|&k| new[k] != old[k]).map(|k| key(t[k].pos)));
            }
        }
        if undo.is_empty() {
            break;
        }
        for k in undo {
            to.insert(k, Vec3::from_array(k.map(f32::from_bits)));
        }
    }
    let mut out = Vec::with_capacity(tris.len());
    for t in tris.as_chunks::<3>().0 {
        let moved = [0, 1, 2].map(|k| Vertex { pos: to[&key(t[k].pos)].to_array(), ..t[k] });
        if moved[0].pos != moved[1].pos && moved[1].pos != moved[2].pos && moved[2].pos != moved[0].pos {
            out.extend_from_slice(&moved);
        }
    }
    *tris = out;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn face(pts: &[[f32; 3]], normal: [f32; 3]) -> Face {
        Face { pts: pts.to_vec(), normal, colour: vec![[0.5, 0.5, 0.5, 1.0]; pts.len()], uv: vec![[0.0; 2]; pts.len()], layer: -1.0, env: 1.0, rgb: [200; 3], smooth_deg: 0.0, remesh: 0.0 }
    }

    /// a repeat of a face (same winding, another start corner) goes; the reversed face stays
    #[test]
    fn drop_repeats_keeps_the_reversed_face() {
        let (a, b, c) = ([0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]);
        let mut faces = vec![face(&[a, b, c], [0.0, 0.0, 1.0]), face(&[c, a, b], [0.0, 0.0, 1.0]), face(&[a, c, b], [0.0, 0.0, -1.0])];
        assert_eq!(drop_repeats(&mut faces), 1);
        assert_eq!(faces.len(), 2);
        assert_eq!(faces[1].normal, [0.0, 0.0, -1.0]);
    }

    /// [O] the Python's front arch tunnel is black in two tags (0 0 0 and 1 1 1): all of it is
    /// rebuilt, none of it is left as flat faces
    #[test]
    fn arch_tunnel_in_two_near_black_tags_is_rebuilt() {
        let st = crate::procgen::bodyfile::from_text("Body bevel=0.015 segments=3 bevel_tol=0.001 smooth_deg=10 arch_step_deg=7.5 arch_clearance=0.035 arch_lip=0.0 drop=0\n").unwrap();
        // rims in (z, y): ground, shoulder, top, shoulder, ground; outer at x 0.95, inner at 0.5
        let rim = [(-0.5, 0.0), (-0.45, 0.4), (0.0, 0.62), (0.45, 0.4), (0.5, 0.0)];
        let hub = Vec3::new(0.75, 0.3, 0.0);
        let mut faces = Vec::new();
        for (k, w) in rim.windows(2).enumerate() {
            let at = |x: f32, (z, y): (f32, f32)| [x, y, z];
            let pts = [at(0.95, w[0]), at(0.95, w[1]), at(0.5, w[1]), at(0.5, w[0])];
            let c = pts.iter().map(|&p| Vec3::from(p)).sum::<Vec3>() / 4.0;
            let n = Vec3::new(0.0, hub.y - c.y, hub.z - c.z).normalize();
            let mut f = face(&pts, n.to_array());
            f.rgb = if k == 1 { [1, 1, 1] } else { [0, 0, 0] };
            faces.push(f);
        }
        round_arches(&mut faces, &[hub.to_array()], 0.3, &st);
        assert!(faces.len() > 4, "the tunnel was rebuilt as a strip of quads");
        assert!(faces.iter().all(|f| f.rgb == [0, 0, 0]), "no flat 1 1 1 face is left");
        assert!(near_tag([0, 0, 0], [1, 1, 1]) && !near_tag([0, 0, 0], [0, 0, 3]) && !near_tag([244, 129, 0], [0, 0, 0]));
    }

    /// merge by distance: a fan wedge between two ring points 0.3 mm apart collapses and its
    /// neighbour takes the kept point; a move that would turn a thin triangle over is refused
    #[test]
    fn merge_close_collapses_without_flipping() {
        let v = |p: [f32; 3]| Vertex { pos: p, uv: [0.0; 2], color: [1.0; 4], layer: -1.0, normal: [0.0, 0.0, 1.0], env: 0.0 };
        let mut fan = vec![v([0.0, 0.0, 0.0]), v([1.0, 0.0, 0.0]), v([1.0, 0.0003, 0.0]), v([0.0, 0.0, 0.0]), v([1.0, 0.0003, 0.0]), v([1.0, 1.0, 0.0])];
        merge_close(&mut fan, MERGE);
        assert_eq!(fan.iter().map(|v| v.pos).collect::<Vec<_>>(), vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [1.0, 1.0, 0.0]]);
        // the thin triangle's tip would cross its base onto the earlier point below it
        let mut thin = vec![v([0.5, -0.0001, 0.0]), v([0.5, -1.0, 0.0]), v([0.6, -1.0, 0.0]), v([0.0, 0.0, 0.0]), v([1.0, 0.0, 0.0]), v([0.5, 0.0003, 0.0])];
        let before: Vec<[f32; 3]> = thin.iter().map(|v| v.pos).collect();
        merge_close(&mut thin, MERGE);
        assert_eq!(thin.iter().map(|v| v.pos).collect::<Vec<_>>(), before);
    }

    /// a strip 2 cm wide and 40 cm long with its long sides cut every 4 cm: best-ear clipping
    /// pairs the cut points across it (the first ear found fanned one corner across the strip)
    #[test]
    fn triangulate_strip_without_slivers() {
        let mut p: Vec<Vec2> = (0..=10).map(|i| Vec2::new(i as f32 * 0.04, 0.0)).collect();
        p.extend((0..=10).rev().map(|i| Vec2::new(i as f32 * 0.04, 0.02)));
        let tris = triangulate(&p);
        assert_eq!(tris.len(), p.len() - 2);
        let min_angle = tris
            .iter()
            .map(|t| (0..3).map(|k| { let (a, b, c) = (p[t[k]], p[t[(k + 1) % 3]], p[t[(k + 2) % 3]]); (b - a).normalize().dot((c - a).normalize()).clamp(-1.0, 1.0).acos().to_degrees() }).fold(f32::MAX, f32::min))
            .fold(f32::MAX, f32::min);
        assert!(min_angle > 20.0, "{min_angle}");
        let area: f32 = tris.iter().map(|t| (p[t[1]] - p[t[0]]).perp_dot(p[t[2]] - p[t[0]]) / 2.0).sum();
        assert!((area - 0.008).abs() < 1e-6, "{area}");
    }

    /// a unit cube as 6 quads (wound either way: the planes decide)
    fn cube() -> Vec<Face> {
        let c = |x: f32, y: f32, z: f32| [x, y, z];
        vec![
            face(&[c(0., 0., 0.), c(1., 0., 0.), c(1., 1., 0.), c(0., 1., 0.)], [0., 0., -1.]),
            face(&[c(0., 0., 1.), c(1., 0., 1.), c(1., 1., 1.), c(0., 1., 1.)], [0., 0., 1.]),
            face(&[c(0., 0., 0.), c(0., 1., 0.), c(0., 1., 1.), c(0., 0., 1.)], [-1., 0., 0.]),
            face(&[c(1., 0., 0.), c(1., 1., 0.), c(1., 1., 1.), c(1., 0., 1.)], [1., 0., 0.]),
            face(&[c(0., 0., 0.), c(1., 0., 0.), c(1., 0., 1.), c(0., 0., 1.)], [0., -1., 0.]),
            face(&[c(0., 1., 0.), c(1., 1., 0.), c(1., 1., 1.), c(0., 1., 1.)], [0., 1., 0.]),
        ]
    }

    fn area(v: &[Vertex]) -> f32 {
        v.chunks(3).map(|t| (Vec3::from(t[1].pos) - Vec3::from(t[0].pos)).cross(Vec3::from(t[2].pos) - Vec3::from(t[0].pos)).length() / 2.0).sum()
    }

    /// the bevelled cube stays inside the cube, is watertight (every edge in two triangles),
    /// faces out
    #[test]
    fn cube_bevels_inside() {
        let st = BodyStyle { bevel: 0.05, segments: 4, bevel_tol: 0.001, smooth_deg: 9.0, arch_step_deg: 0.0, arch_clearance: 0.0, arch_lip: 0.0, drop: 0.0, centre: 0.0, glass: None, flat_glass: Vec::new(), lamps: Vec::new(), vents: Vec::new(), chamfers: Vec::new(), panels: Vec::new(), splitter: None, wing: None, fins: None, foils: Vec::new(), paints: Vec::new(), mirror_copies: Vec::new(), pushes: Vec::new(), thickens: Vec::new(), mirror_glass: None, carbon: Vec::new(), exhaust: None, smooths: Vec::new(), refine: 0.0 };
        let v = body(&cube(), &st);
        for p in v.iter().map(|v| Vec3::from(v.pos)) {
            assert!(p.x >= -1e-5 && p.x <= 1.0 + 1e-5 && p.y >= -1e-5 && p.y <= 1.0 + 1e-5 && -p.z >= -1e-5 && -p.z <= 1.0 + 1e-5, "{p}");
        }
        // faces shrunk to 0.9², rounded edges and corners: about 5.77
        let a = area(&v);
        assert!(a < 5.8 && a > 5.7, "area {a}");
        let key = |v: &Vertex| v.pos.map(|c| (c * 1e4).round() as i32);
        let mut edges: HashMap<[[i32; 3]; 2], i32> = HashMap::new();
        for t in v.chunks(3) {
            for i in 0..3 {
                let (a, b) = (key(&t[i]), key(&t[(i + 1) % 3]));
                *edges.entry(if a < b { [a, b] } else { [b, a] }).or_default() += 1;
            }
        }
        assert!(edges.values().all(|&n| n == 2), "{} open edges", edges.values().filter(|&&n| n != 2).count());
        let centre = Vec3::new(0.5, 0.5, -0.5);
        for t in v.chunks(3) {
            let p = t.iter().map(|v| Vec3::from(v.pos)).collect::<Vec<_>>();
            let c = (p[1] - p[0]).cross(p[2] - p[0]);
            let n: Vec3 = t.iter().map(|v| Vec3::from(v.normal)).sum();
            assert!(c.dot(n) > 0.0 && c.dot((p[0] + p[1] + p[2]) / 3.0 - centre) > 0.0);
        }
    }

    /// a cube whose top is a fan of three triangles round a point of its front edge (the middle
    /// one touches the crease with its corner only): the fan shares one inset point, so the
    /// body stays watertight and inside the cube (#232: each kept its own, a T-junction)
    #[test]
    fn crease_fan_shares_its_inset() {
        let st = BodyStyle { bevel: 0.05, segments: 4, bevel_tol: 0.001, smooth_deg: 9.0, arch_step_deg: 0.0, arch_clearance: 0.0, arch_lip: 0.0, drop: 0.0, centre: 0.0, glass: None, flat_glass: Vec::new(), lamps: Vec::new(), vents: Vec::new(), chamfers: Vec::new(), panels: Vec::new(), splitter: None, wing: None, fins: None, foils: Vec::new(), paints: Vec::new(), mirror_copies: Vec::new(), pushes: Vec::new(), thickens: Vec::new(), mirror_glass: None, carbon: Vec::new(), exhaust: None, smooths: Vec::new(), refine: 0.0 };
        let mut faces = cube();
        let v = [0.5, 1.0, 0.0];
        faces[0] = face(&[[0., 0., 0.], [1., 0., 0.], [1., 1., 0.], v, [0., 1., 0.]], [0., 0., -1.]);
        faces.pop();
        let up = [0., 1., 0.];
        faces.extend([face(&[v, [1., 1., 0.], [1., 1., 1.]], up), face(&[v, [1., 1., 1.], [0., 1., 1.]], up), face(&[v, [0., 1., 1.], [0., 1., 0.]], up)]);
        let out = body(&faces, &st);
        for p in out.iter().map(|v| Vec3::from(v.pos)) {
            assert!(p.x >= -1e-5 && p.x <= 1.0 + 1e-5 && p.y >= -1e-5 && p.y <= 1.0 + 1e-5 && -p.z >= -1e-5 && -p.z <= 1.0 + 1e-5, "{p}");
        }
        let key = |v: &Vertex| v.pos.map(|c| (c * 1e4).round() as i32);
        let mut edges: HashMap<[[i32; 3]; 2], i32> = HashMap::new();
        for t in out.chunks(3) {
            for i in 0..3 {
                let (a, b) = (key(&t[i]), key(&t[(i + 1) % 3]));
                *edges.entry(if a < b { [a, b] } else { [b, a] }).or_default() += 1;
            }
        }
        assert!(edges.values().all(|&n| n == 2), "{} open edges", edges.values().filter(|&&n| n != 2).count());
    }

    /// a cube whose top is glass: the pane sinks to 1 - inset inside a rubber frame, darker,
    /// and the body stays watertight
    #[test]
    fn glass_top_insets() {
        let g = Glass { rgb: [21, 25, 29], frame: 0.1, inset: 0.05, tint: 0.5 };
        let st = BodyStyle { bevel: 0.02, segments: 2, bevel_tol: 0.001, smooth_deg: 9.0, arch_step_deg: 0.0, arch_clearance: 0.0, arch_lip: 0.0, drop: 0.0, centre: 0.0, glass: Some(g), flat_glass: Vec::new(), lamps: Vec::new(), vents: Vec::new(), chamfers: Vec::new(), panels: Vec::new(), splitter: None, wing: None, fins: None, foils: Vec::new(), paints: Vec::new(), mirror_copies: Vec::new(), pushes: Vec::new(), thickens: Vec::new(), mirror_glass: None, carbon: Vec::new(), exhaust: None, smooths: Vec::new(), refine: 0.0 };
        let mut faces = cube();
        faces[5].rgb = g.rgb;
        inset_glass(&mut faces, &g, &[]);
        assert_eq!(faces.len(), 5 + 1 + 8);
        let pane = faces.iter().find(|f| f.rgb == g.rgb).unwrap();
        // the vertex normals are the glass's own: straight down by 0.05, 0.1 in from the edges
        let near = |a: f32, b: f32| (a - b).abs() < 1e-5;
        assert!(pane.pts.iter().all(|p| near(p[1], 0.95) && (near(p[0], 0.1) || near(p[0], 0.9)) && (near(p[2], 0.1) || near(p[2], 0.9))), "{:?}", pane.pts);
        assert!(pane.colour.iter().all(|c| c[0] == 0.25));
        let v = body(&faces, &st);
        let key = |v: &Vertex| v.pos.map(|c| (c * 1e4).round() as i32);
        let mut edges: HashMap<[[i32; 3]; 2], i32> = HashMap::new();
        for t in v.chunks(3) {
            for i in 0..3 {
                let (a, b) = (key(&t[i]), key(&t[(i + 1) % 3]));
                *edges.entry(if a < b { [a, b] } else { [b, a] }).or_default() += 1;
            }
        }
        let bad: Vec<_> = edges.iter().filter(|e| *e.1 != 2).collect();
        assert!(bad.is_empty(), "open edges {bad:?}");
    }

    /// a cube whose glass top is four triangles fanned round a raised middle point (the Titan's
    /// side windows, #291): in a `flat` box it becomes one pane in the outline's plane, the
    /// middle point gone, and the body stays watertight
    #[test]
    fn flat_glass_one_pane() {
        let g = Glass { rgb: [21, 25, 29], frame: 0.1, inset: 0.05, tint: 0.5 };
        let st = BodyStyle { bevel: 0.02, segments: 2, bevel_tol: 0.001, smooth_deg: 9.0, arch_step_deg: 0.0, arch_clearance: 0.0, arch_lip: 0.0, drop: 0.0, centre: 0.0, glass: Some(g), flat_glass: Vec::new(), lamps: Vec::new(), vents: Vec::new(), chamfers: Vec::new(), panels: Vec::new(), splitter: None, wing: None, fins: None, foils: Vec::new(), paints: Vec::new(), mirror_copies: Vec::new(), pushes: Vec::new(), thickens: Vec::new(), mirror_glass: None, carbon: Vec::new(), exhaust: None, smooths: Vec::new(), refine: 0.0 };
        let mut faces = cube();
        faces.pop();
        let (m, up) = ([0.5, 1.05, 0.5], [0., 1., 0.]);
        let ring = [[0., 1., 0.], [1., 1., 0.], [1., 1., 1.], [0., 1., 1.]];
        for k in 0..4 {
            let mut f = face(&[ring[k], m, ring[(k + 1) % 4]], up);
            f.rgb = g.rgb;
            faces.push(f);
        }
        assert!(inset_glass(&mut faces, &g, &[([0.0, 0.5, 0.0], [1.0, 1.5, 1.0])]));
        let panes: Vec<&Face> = faces.iter().filter(|f| f.rgb == g.rgb).collect();
        assert_eq!(panes.len(), 1);
        let near = |a: f32, b: f32| (a - b).abs() < 1e-5;
        assert!(panes[0].pts.len() == 4 && panes[0].pts.iter().all(|p| near(p[1], 0.95) && (near(p[0], 0.1) || near(p[0], 0.9)) && (near(p[2], 0.1) || near(p[2], 0.9))), "{:?}", panes[0].pts);
        let v = body(&faces, &st);
        let key = |v: &Vertex| v.pos.map(|c| (c * 1e4).round() as i32);
        let mut edges: HashMap<[[i32; 3]; 2], i32> = HashMap::new();
        for t in v.chunks(3) {
            for i in 0..3 {
                let (a, b) = (key(&t[i]), key(&t[(i + 1) % 3]));
                *edges.entry(if a < b { [a, b] } else { [b, a] }).or_default() += 1;
            }
        }
        assert!(edges.values().all(|&n| n == 2), "{} open edges", edges.values().filter(|&&n| n != 2).count());
    }

    /// a concave polygon (a square with an arc cut into one side, like a panel by a rounded
    /// arch) fills exactly its own area
    #[test]
    fn concave_panel_triangulates() {
        let mut p = vec![Vec2::new(0.0, 0.0), Vec2::new(2.0, 0.0), Vec2::new(2.0, 2.0)];
        p.extend((0..=8).map(|i| {
            let a = std::f32::consts::PI * i as f32 / 8.0;
            Vec2::new(1.0 + 0.8 * a.cos(), 2.0 - 0.8 * a.sin())
        }));
        p.push(Vec2::new(0.0, 2.0));
        p.dedup();
        let shoelace = p.iter().zip(p.iter().cycle().skip(1)).map(|(a, b)| a.perp_dot(*b)).sum::<f32>() / 2.0;
        let tris = triangulate(&p);
        let sum: f32 = tris.iter().map(|t| (p[t[1]] - p[t[0]]).perp_dot(p[t[2]] - p[t[0]]) / 2.0).sum();
        assert!(tris.iter().all(|t| (p[t[1]] - p[t[0]]).perp_dot(p[t[2]] - p[t[0]]) > 0.0));
        assert!((sum - shoelace).abs() < 1e-4, "{sum} vs {shoelace}");
    }

    /// a flat square split in two stays flat and the same size: nothing to bevel
    #[test]
    fn coplanar_split_is_smooth() {
        let st = BodyStyle { bevel: 0.05, segments: 4, bevel_tol: 0.001, smooth_deg: 9.0, arch_step_deg: 0.0, arch_clearance: 0.0, arch_lip: 0.0, drop: 0.0, centre: 0.0, glass: None, flat_glass: Vec::new(), lamps: Vec::new(), vents: Vec::new(), chamfers: Vec::new(), panels: Vec::new(), splitter: None, wing: None, fins: None, foils: Vec::new(), paints: Vec::new(), mirror_copies: Vec::new(), pushes: Vec::new(), thickens: Vec::new(), mirror_glass: None, carbon: Vec::new(), exhaust: None, smooths: Vec::new(), refine: 0.0 };
        let up = [0.0, 1.0, 0.0];
        let f = [face(&[[0., 0., 0.], [1., 0., 0.], [1., 0., 1.]], up), face(&[[0., 0., 0.], [1., 0., 1.], [0., 0., 1.]], up)];
        let v = body(&f, &st);
        assert!((area(&v) - 1.0).abs() < 1e-4 && v.iter().all(|v| v.pos[1] == 0.0 && v.normal == [0.0, 1.0, -0.0]));
        assert_eq!(v.len(), 6);
    }
}

