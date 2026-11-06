//! REMASTER modern HUD layout (09-remaster 9.21 phase 2, all of it a design choice): which
//! corner elements show, their scale about their anchor corner and the margin from the screen
//! edges. The Race line HUD (#323, `hud_glass`) has its own placement, so 100 % no longer overlays
//! the Classic HUD; each side keeps out of the centre column instead. The toggles rule both HUDs (bug #167:
//! F9 keeps what shows). The toggles are the player's own set: InfoDetail None / Selective /
//! All only change what shows (`shown`), Custom shows the set as saved (bug #181); RoadMap sets
//! the map toggle.

/// The minimap's view: the whole circuit, or a close window round you (9.21 phase 6). Only the Race line
/// HUD has the close view; the Classic HUD always draws the original map art.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MapMode {
    Circuit,
    Close,
}

pub const MAP_MODES: [&str; 2] = ["Circuit", "Close"];

/// InfoDetail's fourth value [design choice; the original has None / Selective / All]: show the
/// HUD Settings toggles as the player set them.
pub const INFO_CUSTOM: usize = 3;

/// The HUD Settings (settings.cfg `Hud*`, the pause menu's HUD Settings screen).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HudSettings {
    /// speed, gear and rev meter (top left)
    pub cluster: bool,
    /// TOTAL TIME, TOTAL LAP TIME, BEST LAP TIME (top right)
    pub clocks: [bool; 3],
    pub lap: bool,
    pub pos: bool,
    pub map: bool,
    /// proximity arrows (`proximity.rs`, phase 7)
    pub proximity: bool,
    /// percent, SCALE_RANGE in SCALE_STEP steps
    pub scale: u32,
    /// percent of the free space between the elements (`Placement`)
    pub margin: u32,
    pub map_mode: MapMode,
    /// Close map: turn with the car (travel up) instead of north up
    pub map_rotate: bool,
    /// the place numbers over the cars' markers on the map
    pub map_numbers: bool,
    /// See-through name plates: the glass frame round them (off leaves the colour bar and the name)
    pub plate_frame: bool,
    /// seconds BEST LAP / LAST LAP stays up at the top of the Modern HUD, 0 = off (REMASTER; Classic keeps CheckpointDelay)
    pub lap_delay: u32,
    /// REMASTER: the bottom-left block shows the live gap to the car ahead and behind all race, instead of the
    /// checkpoint list (#302; Time Attack keeps its checkpoint rows)
    pub live_gaps: bool,
    /// seconds the SECTOR 1 / 2 delta stays up at each checkpoint, 0 = off; also 0 hides the delta on BEST / LAST LAP
    /// (REMASTER, #409; same range as `lap_delay`)
    pub sector_delay: u32,
}

impl Default for HudSettings {
    fn default() -> Self {
        HudSettings { cluster: true, clocks: [true; 3], lap: true, pos: true, map: true, proximity: true, scale: 100, margin: 0, map_mode: MapMode::Circuit, map_rotate: false, map_numbers: true, plate_frame: true, lap_delay: LAP_DELAY_DEFAULT, live_gaps: false, sector_delay: LAP_DELAY_DEFAULT }
    }
}

/// The Modern HUD's lap message time: default and top of the "Lap times on screen" row, seconds.
pub const LAP_DELAY_DEFAULT: u32 = 4;
pub const LAP_DELAY_MAX: u32 = 10;
pub const SCALE_RANGE: (u32, u32) = (50, 200);
pub const SCALE_STEP: u32 = 10;
pub const MARGIN_STEP: u32 = 10;
/// The elements' distance from the screen edges at margin 0: hud.rs's LEFT / RIGHT_MARGIN.
const BASE_MARGIN: f32 = 12.0;
/// Centre column kept free of the corner elements (countdown, lap flash, WRONG WAY), as a
/// fraction of the canvas width.
const CENTRE_RESERVE: f32 = 0.30;
/// Vertical gap kept between the right column and the map.
const COLUMN_GAP: f32 = 16.0;
/// The margin moves an element at most this far down/up (canvas px, 25 % of the 480 canvas),
/// out of the car's area.
const MAX_MARGIN_Y: f32 = 120.0;

/// Sizes of the visible elements at 100 % (canvas px; 0 when hidden) and the distance of
/// their anchors from the top and bottom edge.
#[derive(Clone, Copy, Debug, Default)]
pub struct Extents {
    pub left: (f32, f32),
    pub right: (f32, f32),
    pub map: (f32, f32),
    /// the checkpoint splits, bottom left, sized for the most rows the settings can show (0 when they are off)
    pub splits: (f32, f32),
    /// top of the corner elements (y of the cluster and the right column)
    pub top: f32,
    /// map's distance from the bottom edge
    pub bottom: f32,
}

/// Where the elements go: each scales by `scale` about its anchor corner, then moves `dx`
/// in from the side edges and `dy` in from the top/bottom.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Placement {
    pub scale: f32,
    pub dx: f32,
    pub dy: f32,
}

impl HudSettings {
    /// The corner toggles InfoDetail shows (0x44a2a4): None hides them, Selective shows speed,
    /// TOTAL LAP TIME and LAP, All everything, Custom the player's own set (`self`). Never
    /// changes `self`, so the saved selection survives the presets (bug #181). APPROX:
    /// Selective's cluster brings the gear and rev arc along (the original shows the speed
    /// alone; they share one toggle).
    pub fn shown(&self, info_detail: usize) -> HudSettings {
        if info_detail >= INFO_CUSTOM {
            return *self;
        }
        let (some, all) = (info_detail >= 1, info_detail >= 2);
        HudSettings { cluster: some, clocks: [all, some, all], lap: some, pos: all, ..*self }
    }

    /// RoadMap as a preset of the map toggle: None hides it (Selective still filters the
    /// markers, 0x44b1f0).
    pub fn apply_road_map(&mut self, level: usize) {
        self.map = level > 0;
    }

    /// The layout on a `canvas_w` x 480 canvas. The scale is cut down when a side would cover
    /// the centre column or the right column would run into the map, but never below the
    /// smallest Scale setting (or the asked scale, if smaller). The margin is a fraction of
    /// the slack left at that scale, so it means the same at any aspect.
    pub fn place(&self, canvas_w: f32, e: &Extents) -> Placement {
        let asked = self.scale as f32 / 100.0;
        // each side has half of what the centre column leaves, less the edge margin
        let side = canvas_w * (1.0 - CENTRE_RESERVE) * 0.5 - BASE_MARGIN;
        let wide = e.left.0.max(e.right.0).max(e.map.0).max(e.splits.0);
        let fit_x = if wide > 0.0 { side / wide } else { f32::MAX };
        // the left side stacks the cluster over the splits, the right side the stack over the map
        let tall = (e.right.1 + e.map.1).max(e.left.1 + e.splits.1);
        let fit_y = if tall > 0.0 { (480.0 - e.top - e.bottom - COLUMN_GAP) / tall } else { f32::MAX };
        let scale = asked.min(fit_x).min(fit_y).max(asked.min(SCALE_RANGE.0 as f32 / 100.0));
        let m = self.margin as f32 / 100.0;
        let slack_x = side - wide * scale;
        let slack_y = ((480.0 - e.top - e.bottom - COLUMN_GAP - tall * scale) * 0.5).min(MAX_MARGIN_Y);
        Placement { scale, dx: m * slack_x.max(0.0), dy: m * slack_y.max(0.0) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The Race line cluster (252 x 141 from its anchor), the top-right stack (169 x 169), the map disc (148 x 147)
    /// and the splits at eight rows (179 x 150)
    const E: Extents = Extents { left: (252.0, 141.0), right: (169.0, 169.0), map: (148.0, 147.0), splits: (179.0, 150.0), top: 8.0, bottom: 10.0 };
    const WIDTHS: [f32; 3] = [640.0, 853.3, 1146.7]; // 4:3, 16:9, 21:9

    #[test]
    fn default_layout_per_aspect() {
        let d = HudSettings::default();
        // 16:9 and wider keep the design size; 4:3 shrinks the cluster to leave the centre column free
        assert_eq!(d.place(WIDTHS[1], &E), Placement { scale: 1.0, dx: 0.0, dy: 0.0 });
        assert_eq!(d.place(WIDTHS[2], &E), Placement { scale: 1.0, dx: 0.0, dy: 0.0 });
        let s = d.place(WIDTHS[0], &E).scale;
        assert!((0.5..1.0).contains(&s), "{s}");
    }

    #[test]
    fn sides_stay_out_of_the_centre_column() {
        for w in WIDTHS {
            for scale in [50, 100, 200] {
                for margin in [0, 100] {
                    let p = HudSettings { scale, margin, ..Default::default() }.place(w, &E);
                    // the cluster's right edge and the right group's left edge, from the anchors
                    let left = BASE_MARGIN + E.left.0.max(E.splits.0) * p.scale + p.dx;
                    let right = w - BASE_MARGIN - E.right.0.max(E.map.0) * p.scale - p.dx;
                    assert!(left <= w * 0.35 + 1e-3, "{w} {scale} {margin}: left {left}");
                    assert!(right >= w * 0.65 - 1e-3, "{w} {scale} {margin}: right {right}");
                }
            }
        }
    }

    #[test]
    fn cluster_and_splits_clear_each_other() {
        for w in WIDTHS {
            for scale in [50, 100, 200] {
                let p = HudSettings { scale, margin: 100, ..Default::default() }.place(w, &E);
                // the cluster's bottom and the splits' top, from their anchors
                let cluster_bottom = 8.0 + E.left.1 * p.scale + p.dy;
                let splits_top = 480.0 - 10.0 - E.splits.1 * p.scale - p.dy;
                assert!(cluster_bottom <= splits_top + 1e-3, "{w} {scale}: {cluster_bottom} {splits_top}");
            }
        }
    }

    #[test]
    fn info_detail_presets() {
        let mut h = HudSettings { cluster: false, clocks: [true, false, false], lap: false, pos: true, ..Default::default() };
        assert_eq!(h.shown(2), HudSettings::default());
        let s = h.shown(1);
        assert!(s.cluster && s.clocks == [false, true, false] && s.lap && !s.pos);
        let n = h.shown(0);
        assert!(!n.cluster && n.clocks == [false; 3] && !n.lap && !n.pos && n.map && n.proximity);
        // the presets leave the player's own set alone; Custom shows it
        assert_eq!(h.shown(INFO_CUSTOM), h);
        assert!(!h.cluster && h.clocks == [true, false, false] && h.pos);
        h.apply_road_map(0);
        assert!(!h.map);
    }

    #[test]
    fn scale_fits_and_margin_stays_inside() {
        let s = HudSettings { scale: 200, margin: 100, ..Default::default() };
        let p = s.place(640.0, &E);
        assert!(p.scale >= 0.5 && p.scale < 2.0);
        // the right column and the map still clear each other
        assert!(8.0 + (E.right.1 + E.map.1) * p.scale + 10.0 + 2.0 * p.dy <= 480.0 - COLUMN_GAP + 1e-3);
        assert!(p.dy <= MAX_MARGIN_Y);
        // nothing hidden: full scale
        let none = Extents { top: 8.0, bottom: 10.0, ..Default::default() };
        assert_eq!(s.place(640.0, &none).scale, 2.0);
        // the smallest Scale setting is a floor
        assert_eq!(HudSettings { scale: 50, ..Default::default() }.place(640.0, &E).scale, 0.5);
        // ultra-wide gets more side margin than 4:3
        let small = HudSettings { margin: 100, ..Default::default() };
        assert!(small.place(1146.7, &E).dx > small.place(640.0, &E).dx);
    }
}
