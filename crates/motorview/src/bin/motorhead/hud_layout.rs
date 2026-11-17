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

/// A box on the canvas: x, y, width, height (canvas px; the canvas is `canvas_w` x 480).
pub type Rect = [f32; 4];

/// The pieces of the Race line HUD the player can place (#542). The first eight move, scale and hide; the last two
/// follow cars in the world, so they only scale and hide.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HudElem {
    /// speed, gear, rev swoosh (top left)
    Cluster,
    /// the clocks, POSITION and LAP (top right)
    Stack,
    /// the map disc (bottom right)
    Map,
    /// the checkpoint splits / live gaps block (bottom left)
    Splits,
    /// the start lamps' numbers and GO (centre)
    Countdown,
    /// BEST / LAST LAP, SECTOR, FINAL LAP, FINISHED, the notices (top centre)
    Messages,
    WrongWay,
    /// the REC / GHOST / DEMO tags (top centre)
    ModeTags,
    /// the proximity arrows round your car
    Proximity,
    /// the rivals' name plates
    Plates,
}

impl HudElem {
    pub const ALL: [HudElem; 10] = [
        HudElem::Cluster,
        HudElem::Stack,
        HudElem::Map,
        HudElem::Splits,
        HudElem::Countdown,
        HudElem::Messages,
        HudElem::WrongWay,
        HudElem::ModeTags,
        HudElem::Proximity,
        HudElem::Plates,
    ];

    /// The name in settings.cfg `HudPlace` lines.
    pub fn key(self) -> &'static str {
        ["cluster", "stack", "map", "splits", "countdown", "messages", "wrongway", "tags", "proximity", "plates"][self as usize]
    }

    pub fn from_key(s: &str) -> Option<HudElem> {
        HudElem::ALL.into_iter().find(|e| e.key().eq_ignore_ascii_case(s))
    }

    /// The name the layout editor shows.
    pub fn name(self) -> &'static str {
        ["SPEEDOMETER", "CLOCKS & PLACE", "MAP", "SPLITS", "COUNTDOWN", "MESSAGES", "WRONG WAY", "MODE TAGS", "PROXIMITY ARROWS", "NAME PLATES"][self as usize]
    }

    /// False for the pieces that follow cars in the world: only their scale counts.
    pub fn movable(self) -> bool {
        !matches!(self, HudElem::Proximity | HudElem::Plates)
    }
}

/// One of the nine points of the screen (corners, edge middles, centre) a piece is placed from. The editor picks the
/// nearest one, so a layout survives a change of aspect ratio: a piece near the right edge stays near it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Anchor {
    TopLeft,
    Top,
    TopRight,
    Left,
    Centre,
    Right,
    BottomLeft,
    Bottom,
    BottomRight,
}

impl Anchor {
    pub const ALL: [Anchor; 9] = [Anchor::TopLeft, Anchor::Top, Anchor::TopRight, Anchor::Left, Anchor::Centre, Anchor::Right, Anchor::BottomLeft, Anchor::Bottom, Anchor::BottomRight];
    const KEYS: [&'static str; 9] = ["tl", "t", "tr", "l", "c", "r", "bl", "b", "br"];

    pub fn key(self) -> &'static str {
        Anchor::KEYS[self as usize]
    }

    pub fn from_key(s: &str) -> Option<Anchor> {
        Anchor::KEYS.iter().position(|k| k.eq_ignore_ascii_case(s)).map(|i| Anchor::ALL[i])
    }

    /// The point of `r` this anchor names (a corner, an edge's middle or the centre).
    pub fn point(self, r: Rect) -> (f32, f32) {
        let i = self as usize;
        (r[0] + r[2] * (i % 3) as f32 * 0.5, r[1] + r[3] * (i / 3) as f32 * 0.5)
    }

    /// The screen point nearest to `p` on a `canvas_w` x 480 canvas.
    pub fn nearest(p: (f32, f32), canvas_w: f32) -> Anchor {
        let third = |v: f32, len: f32| ((v / len * 3.0).floor() as i32).clamp(0, 2) as usize;
        Anchor::ALL[third(p.1, 480.0) * 3 + third(p.0, canvas_w)]
    }
}

/// Where the player put one piece: its `anchor` point (of the piece's own box) sits `off` canvas px from the same
/// point of the screen, drawn at `scale` percent.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ElemLayout {
    pub anchor: Anchor,
    pub off: (i32, i32),
    pub scale: u32,
}

/// A piece's scale in percent, `HudPlace` lines are clamped to it (the HUD Scale setting multiplies it).
pub const ELEM_SCALE: (u32, u32) = (25, 300);
/// `HudPlace` offsets are clamped to this many canvas px, so a bad file cannot throw a piece far off the screen.
const OFF_MAX: i32 = 4000;

/// How `HudLayout::put` puts a piece on the canvas: scale its drawing by `k` about `anchor`, then move it by `off`
/// (the arguments of `hud::place`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Put {
    pub anchor: (f32, f32),
    pub k: f32,
    pub off: (f32, f32),
}

impl ElemLayout {
    /// The factor the piece is drawn at: its own scale times the HUD Scale setting.
    pub fn factor(&self, hud_scale: u32) -> f32 {
        self.scale as f32 / 100.0 * hud_scale as f32 / 100.0
    }

    /// The transform for a piece drawn at 100 % in `home` (its box where the stock layout draws it, at the HUD
    /// scale 1). The piece's box is kept on the screen [design choice, as in WoW]: a piece bigger than the screen
    /// lines up with the top left.
    pub fn put(&self, home: Rect, canvas_w: f32, hud_scale: u32) -> Put {
        let k = self.factor(hud_scale);
        let h = self.anchor.point(home);
        let s = self.anchor.point([0.0, 0.0, canvas_w, 480.0]);
        let mut p = (s.0 + self.off.0 as f32, s.1 + self.off.1 as f32);
        // the box after the move, then the shift that brings it inside
        let (x0, y0) = (p.0 + (home[0] - h.0) * k, p.1 + (home[1] - h.1) * k);
        let (w, hh) = (home[2] * k, home[3] * k);
        let fit = |lo: f32, len: f32, max: f32| if len >= max || lo < 0.0 { -lo } else if lo + len > max { max - (lo + len) } else { 0.0 };
        p.0 += fit(x0, w, canvas_w);
        p.1 += fit(y0, hh, 480.0);
        Put { anchor: h, k, off: (p.0 - h.0, p.1 - h.1) }
    }

    /// The box `put` draws the piece in (canvas px).
    pub fn placed(&self, home: Rect, canvas_w: f32, hud_scale: u32) -> Rect {
        let p = self.put(home, canvas_w, hud_scale);
        let at = |v: f32, a: f32, o: f32| a + (v - a) * p.k + o;
        [at(home[0], p.anchor.0, p.off.0), at(home[1], p.anchor.1, p.off.1), home[2] * p.k, home[3] * p.k]
    }

    /// The same placement written as the layout editor keeps it (#542): the box where `put` leaves it (so pushing a
    /// piece past an edge stores nothing that does not show), anchored to the screen point nearest its centre, so it
    /// keeps its side when the aspect changes.
    pub fn normalise(&self, home: Rect, canvas_w: f32, hud_scale: u32) -> ElemLayout {
        let r = self.placed(home, canvas_w, hud_scale);
        let anchor = Anchor::nearest((r[0] + r[2] * 0.5, r[1] + r[3] * 0.5), canvas_w);
        let (p, s) = (anchor.point(r), anchor.point([0.0, 0.0, canvas_w, 480.0]));
        ElemLayout { anchor, off: ((p.0 - s.0).round() as i32, (p.1 - s.1).round() as i32), scale: self.scale }
    }
}

/// The player's HUD layout: one `ElemLayout` per piece. While `custom` is false the stock auto-fit runs
/// (`HudSettings::place`, byte-identical to before #542) and `elems` only holds the starting values for the editor;
/// once `custom`, every movable piece is used as placed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HudLayout {
    pub custom: bool,
    pub elems: [ElemLayout; 10],
}

impl Default for HudLayout {
    fn default() -> Self {
        HudLayout::stock()
    }
}

impl HudLayout {
    /// Where the stock layout puts each piece at HUD Scale 100 % and a wide enough screen. The numbers are the
    /// boxes' stock positions (hud.rs `home`; a test there keeps the two together).
    pub fn stock() -> HudLayout {
        let e = |anchor, x, y| ElemLayout { anchor, off: (x, y), scale: 100 };
        use Anchor::*;
        HudLayout {
            custom: false,
            elems: [e(TopLeft, 12, 8), e(TopRight, -12, 8), e(BottomRight, -10, -10), e(BottomLeft, 12, -10), e(Centre, 0, -91), e(Top, 0, 32), e(Centre, 0, -39), e(Top, 0, 9), e(Centre, 0, 0), e(Centre, 0, 0)],
        }
    }

    pub fn get(&self, e: HudElem) -> &ElemLayout {
        &self.elems[e as usize]
    }

    /// The settings.cfg lines of a custom layout (empty while stock, so a stock file stays as it was).
    pub fn lines(&self) -> String {
        if !self.custom {
            return String::new();
        }
        HudElem::ALL
            .iter()
            .map(|&e| {
                let l = self.get(e);
                format!("HudPlace {} {} {} {} {}\n", e.key(), l.anchor.key(), l.off.0, l.off.1, l.scale)
            })
            .collect()
    }

    /// One `HudPlace <elem> <anchor> <x> <y> <scale>` line (the rest after the key; `elem` is its first word).
    /// A good line sets the piece, makes the layout custom and returns true; a bad one changes nothing.
    pub fn apply_place<'a>(&mut self, elem: &str, rest: &mut impl Iterator<Item = &'a str>) -> bool {
        let (Some(e), Some(anchor)) = (HudElem::from_key(elem), rest.next().and_then(Anchor::from_key)) else { return false };
        let mut num = || rest.next().and_then(|x| x.parse::<i64>().ok());
        let (Some(x), Some(y), Some(scale)) = (num(), num(), num()) else { return false };
        let off = |v: i64| v.clamp(-OFF_MAX as i64, OFF_MAX as i64) as i32;
        self.elems[e as usize] = ElemLayout { anchor, off: (off(x), off(y)), scale: scale.clamp(ELEM_SCALE.0 as i64, ELEM_SCALE.1 as i64) as u32 };
        self.custom = true;
        true
    }
}

/// Each piece's box as drawn this frame (canvas px, from the vertices after placing): the layout editor's
/// hit boxes and outlines. `None` for a piece that drew nothing.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct HudRects {
    pub rects: [Option<Rect>; 10],
}

impl HudRects {
    pub fn get(&self, e: HudElem) -> Option<Rect> {
        self.rects[e as usize]
    }
}

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
    /// REMASTER (#542): the centre pieces and the splits block, name plates and tags can be hidden like the corner ones
    pub countdown: bool,
    pub messages: bool,
    pub wrong_way: bool,
    pub mode_tags: bool,
    pub splits: bool,
    pub plates: bool,
    /// REMASTER (#542 phase 3): the layout editor's mouse drag snaps to the screen edges, the centre lines and a grid
    pub snap: bool,
    /// REMASTER (#542): the player's own placement of the pieces
    pub layout: HudLayout,
}

impl Default for HudSettings {
    fn default() -> Self {
        HudSettings { cluster: true, clocks: [true; 3], lap: true, pos: true, map: true, proximity: true, scale: 100, margin: 0, map_mode: MapMode::Circuit, map_rotate: false, map_numbers: true, plate_frame: true, lap_delay: LAP_DELAY_DEFAULT, live_gaps: false, sector_delay: LAP_DELAY_DEFAULT, countdown: true, messages: true, wrong_way: true, mode_tags: true, splits: true, plates: true, snap: true, layout: HudLayout::stock() }
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

    /// The settings.cfg lines of the #542 additions: the hide toggles, then the custom layout's `HudPlace` lines.
    pub fn extra_text(&self) -> String {
        let b = |x: bool| x as u8;
        format!(
            "HudCountdown {}\nHudMessages {}\nHudWrongWay {}\nHudModeTags {}\nHudSplits {}\nHudPlates {}\nHudSnap {}\n{}",
            b(self.countdown),
            b(self.messages),
            b(self.wrong_way),
            b(self.mode_tags),
            b(self.splits),
            b(self.plates),
            b(self.snap),
            self.layout.lines()
        )
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
    fn anchors_name_points_of_a_box() {
        let r = [10.0, 20.0, 100.0, 40.0];
        assert_eq!(Anchor::TopLeft.point(r), (10.0, 20.0));
        assert_eq!(Anchor::Centre.point(r), (60.0, 40.0));
        assert_eq!(Anchor::BottomRight.point(r), (110.0, 60.0));
        assert_eq!(Anchor::Top.point(r), (60.0, 20.0));
        for a in Anchor::ALL {
            assert_eq!(Anchor::from_key(a.key()), Some(a));
        }
        // the nearest screen point is the third the piece lies in, whatever the aspect
        assert_eq!(Anchor::nearest((5.0, 5.0), 853.0), Anchor::TopLeft);
        assert_eq!(Anchor::nearest((850.0, 470.0), 853.0), Anchor::BottomRight);
        assert_eq!(Anchor::nearest((426.0, 240.0), 853.0), Anchor::Centre);
        assert_eq!(Anchor::nearest((-9.0, 900.0), 640.0), Anchor::BottomLeft);
    }

    #[test]
    fn put_scales_about_the_anchor_and_stays_on_screen() {
        let home = [100.0, 50.0, 200.0, 100.0];
        // top left at (30, 20), drawn at 50 %: the box's corner lands there
        let l = ElemLayout { anchor: Anchor::TopLeft, off: (30, 20), scale: 50 };
        let p = l.put(home, 853.0, 100);
        assert_eq!((p.anchor, p.k, p.off), ((100.0, 50.0), 0.5, (-70.0, -30.0)));
        // the HUD Scale multiplies
        assert_eq!(l.put(home, 853.0, 200).k, 1.0);
        // right anchor: the box's right edge sits 12 px in from the screen's, whatever the width
        let r = ElemLayout { anchor: Anchor::Right, off: (-12, 0), scale: 100 };
        for w in [640.0, 853.0, 1147.0] {
            let p = r.put(home, w, 100);
            let right = 300.0 + p.off.0;
            assert_eq!(right, w - 12.0);
        }
        // pushed off the screen: pulled back in
        let out = ElemLayout { anchor: Anchor::TopLeft, off: (-500, 900), scale: 100 };
        let p = out.put(home, 853.0, 100);
        let (x0, y0) = (100.0 + p.off.0, 50.0 + p.off.1);
        assert_eq!((x0, y0 + 100.0), (0.0, 480.0));
        // bigger than the screen: lines up with the top left
        let huge = ElemLayout { anchor: Anchor::Centre, off: (0, 0), scale: 300 };
        let p = huge.put([0.0, 0.0, 400.0, 300.0], 640.0, 100);
        // the box's top left is at the screen's
        assert_eq!((200.0 + p.off.0 - 200.0 * 3.0, 150.0 + p.off.1 - 150.0 * 3.0), (0.0, 0.0));
    }

    #[test]
    fn normalise_keeps_the_box_and_takes_the_nearest_anchor() {
        let home = [100.0, 50.0, 200.0, 100.0];
        for w in [640.0, 853.0, 1147.0] {
            // a piece moved to the bottom right: the same box, now anchored there
            let l = ElemLayout { anchor: Anchor::TopLeft, off: (w as i32 - 230, 360), scale: 100 };
            let n = l.normalise(home, w, 100);
            assert_eq!(n.anchor, Anchor::BottomRight);
            let (a, b) = (l.placed(home, w, 100), n.placed(home, w, 100));
            for i in 0..4 {
                assert!((a[i] - b[i]).abs() <= 0.5, "{w}: {a:?} {b:?}");
            }
            // normalising twice changes nothing
            assert_eq!(n.normalise(home, w, 100), n);
        }
        // pushed far off the left edge: stored where it shows, at the edge
        let out = ElemLayout { anchor: Anchor::Left, off: (-900, 0), scale: 50 }.normalise(home, 853.0, 100);
        assert_eq!(out.placed(home, 853.0, 100)[0], 0.0);
        assert_eq!(out.off.0, 0);
        // the centre third
        let c = ElemLayout { anchor: Anchor::TopLeft, off: (380, 200), scale: 100 }.normalise([0.0, 0.0, 90.0, 60.0], 853.0, 100);
        assert_eq!(c.anchor, Anchor::Centre);
    }

    #[test]
    fn place_lines_round_trip_and_clamp() {
        let mut h = HudSettings::default();
        assert!(!h.layout.custom && h.layout.lines().is_empty());
        let (mut hl, mut other) = (h.layout, HudLayout::stock());
        hl.elems[HudElem::Map as usize] = ElemLayout { anchor: Anchor::BottomLeft, off: (20, -30), scale: 150 };
        hl.custom = true;
        for line in hl.lines().lines() {
            let mut it = line.split_whitespace();
            assert_eq!(it.next(), Some("HudPlace"));
            let e = it.next().unwrap();
            other.apply_place(e, &mut it);
        }
        assert_eq!(other, hl);
        // bad lines change nothing; values are clamped
        let mut c = HudLayout::stock();
        for bad in ["nothing tl 0 0 100", "map zz 0 0 100", "map tl x 0 100", "map tl 0 0"] {
            let mut it = bad.split_whitespace();
            let e = it.next().unwrap();
            c.apply_place(e, &mut it);
        }
        assert_eq!(c, HudLayout::stock());
        let mut it = "tl 99999 -99999 9999".split_whitespace();
        c.apply_place("Cluster", &mut it);
        assert!(c.custom);
        assert_eq!(c.get(HudElem::Cluster), &ElemLayout { anchor: Anchor::TopLeft, off: (OFF_MAX, -OFF_MAX), scale: ELEM_SCALE.1 });
        h.layout = c;
        assert!(h.extra_text().contains("HudPlace cluster tl 4000 -4000 300\n"));
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
