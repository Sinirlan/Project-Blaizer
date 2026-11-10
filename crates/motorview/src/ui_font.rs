//! REMASTER modern HUD fonts (09-remaster 9.21 phase 1): TrueType outlines turned into
//! signed distance fields in the sharp HUD's field atlas (`ui_sdf`), drawn by
//! `Batch::font_text` at any size. Our own additions, not game data; the fonts are
//! embedded (`assets/fonts/`, licences in LICENSES.md: Teko OFL, Rajdhani OFL, Overload CC0, Audiowide OFL, PromptFont OFL, Orbitron OFL) [design choice].
//!
//! Each glyph's outline is flattened and filled by scanlines (nonzero winding) on a grid of
//! 128 cells per em, then goes through the same distance transform as the HUD bitmaps. A
//! field "source pixel" is 1/16 em (`SRC`), stored at 4 texels, so the ±3 px range of the
//! field atlas is 3/16 em: room for an outline up to about 2.5/16 em.

use std::collections::HashMap;
use std::sync::OnceLock;

/// The modern HUD's typefaces.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum HudFont {
    /// Teko Bold: labels (KM/H, GEAR, LAP, POS...)
    Label,
    /// Rajdhani Medium: the race HUD's captions (TOTAL LAP TIME, BEST LAP TIME, KMH, GEAR, ...) [design choice]
    Caption,
    /// Audiowide: clocks, LAP/POS values, the gear digit (#305)
    Clock,
    /// Overload: the `»` of the proximity arrows (`proximity.rs`; was the clock face before #305)
    Arrow,
    /// Teko Bold: the speed (drawn with a synthetic italic shear, Teko has none)
    Speed,
    /// Motorhead (our own FontForge face, lowercase italic): the modern menu's screen titles
    Title,
    /// PromptFont: key, gamepad and button glyphs of the navigation tips (`GLYPHS`)
    Glyphs,
    /// Rajdhani Bold: the Glass HUD menu screens' text (#294, the Concept E mockup's 700 weight)
    Glass,
    /// Rajdhani SemiBold: the Glass HUD's captions and settings (the mockup's 600 weight)
    GlassMed,
    /// Orbitron ExtraBold: the Glass HUD plate's screen name
    Orbitron,
    /// Lucide (ISC): the Glass pause menu's tab and action icons (`icon::ALL`, #300)
    Icons,
}

impl HudFont {
    pub const ALL: [HudFont; 11] = [HudFont::Label, HudFont::Caption, HudFont::Clock, HudFont::Arrow, HudFont::Speed, HudFont::Title, HudFont::Glyphs, HudFont::Glass, HudFont::GlassMed, HudFont::Orbitron, HudFont::Icons];

    fn bytes(self) -> &'static [u8] {
        match self {
            HudFont::Caption => include_bytes!("../../../assets/fonts/Rajdhani-Medium.ttf"),
            HudFont::Clock => include_bytes!("../../../assets/fonts/Audiowide-Regular.ttf"),
            HudFont::Arrow => include_bytes!("../../../assets/fonts/Overload.ttf"),
            HudFont::Title => include_bytes!("../../../assets/fonts/Motorhead.ttf"),
            HudFont::Glyphs => include_bytes!("../../../assets/fonts/PromptFont.ttf"),
            HudFont::Label | HudFont::Speed => include_bytes!("../../../assets/fonts/Teko-Bold.ttf"),
            HudFont::Glass => include_bytes!("../../../assets/fonts/Rajdhani-Bold.ttf"),
            HudFont::GlassMed => include_bytes!("../../../assets/fonts/Rajdhani-SemiBold.ttf"),
            HudFont::Orbitron => include_bytes!("../../../assets/fonts/Orbitron-ExtraBold.ttf"),
            HudFont::Icons => include_bytes!("../../../assets/fonts/lucide/lucide.ttf"),
        }
    }

    /// Where a glyph the font lacks comes from: Teko has all of ASCII and '»' (Overload
    /// has no '~').
    fn fallback(self) -> Option<HudFont> {
        match self {
            HudFont::Clock | HudFont::Arrow | HudFont::Glass | HudFont::GlassMed | HudFont::Orbitron => Some(HudFont::Speed),
            _ => None,
        }
    }
}

/// field source pixels per em
pub const SRC: f32 = 16.0;
/// texels per source pixel in the field atlas
pub const TEXELS: u32 = 4;
/// scanline grid cells per texel (the 2x2 cells are averaged, as for the bitmaps)
pub const FINE: u32 = 2;

/// PromptFont glyphs in use (codepoints from PromptFont's glyphs.json): the navigation tips
/// (keyboard arrows, Enter, Esc, Q E Space R; d-pads; Xbox A B X Y LB RB; PlayStation Cross
/// Circle Square Triangle L1 R1), and the bindings of the modern control options: every key
/// with a keycap glyph, the mouse, pad buttons, sticks and triggers, the device pictures.
pub const GLYPHS: &[char] = &[
    // keys: arrows, Enter .. Esc, Space, the modifier and navigation block, F1-F12, numpad, empty keycap
    '\u{23F4}', '\u{23F5}', '\u{23F6}', '\u{23F7}', '\u{2427}', '\u{2428}', '\u{2429}', '\u{242A}', '\u{242B}', '\u{242C}', '\u{242D}', '\u{242E}',
    '\u{242F}', '\u{2430}', '\u{2431}', '\u{2432}', '\u{2433}', '\u{2434}', '\u{2435}', '\u{2436}', '\u{2437}', '\u{2438}', '\u{2439}', '\u{243A}',
    '\u{244B}', '\u{244C}', '\u{244D}', '\u{244E}', '\u{244F}', '\u{2450}', '\u{2460}', '\u{2461}', '\u{2462}', '\u{2463}', '\u{2464}', '\u{2465}',
    '\u{2466}', '\u{2467}', '\u{2468}', '\u{2469}', '\u{246A}', '\u{246B}', '\u{2474}', '\u{2475}', '\u{2476}', '\u{2477}', '\u{2478}', '\u{2479}',
    '\u{247A}', '\u{247B}', '\u{247C}', '\u{247D}', '\u{247E}', '\u{247F}', '\u{2480}', '\u{2481}', '\u{2482}', '\u{2483}', '\u{2484}', '\u{248F}',
    // keycaps: ' , - . / 0-9 ; = A-Z [ \\ ] `
    '\u{FF07}', '\u{FF0C}', '\u{FF0D}', '\u{FF0E}', '\u{FF0F}', '\u{FF10}', '\u{FF11}', '\u{FF12}', '\u{FF13}', '\u{FF14}', '\u{FF15}', '\u{FF16}',
    '\u{FF17}', '\u{FF18}', '\u{FF19}', '\u{FF1B}', '\u{FF1D}', '\u{FF21}', '\u{FF22}', '\u{FF23}', '\u{FF24}', '\u{FF25}', '\u{FF26}', '\u{FF27}',
    '\u{FF28}', '\u{FF29}', '\u{FF2A}', '\u{FF2B}', '\u{FF2C}', '\u{FF2D}', '\u{FF2E}', '\u{FF2F}', '\u{FF30}', '\u{FF31}', '\u{FF32}', '\u{FF33}',
    '\u{FF34}', '\u{FF35}', '\u{FF36}', '\u{FF37}', '\u{FF38}', '\u{FF39}', '\u{FF3A}', '\u{FF3B}', '\u{FF3C}', '\u{FF3D}', '\u{FF40}',
    // mouse: left, right, middle click; moves left, up, right, down
    '\u{27F5}', '\u{27F6}', '\u{27F7}', '\u{27F8}', '\u{27F9}', '\u{27FD}', '\u{27FE}',
    // pads: LT RT LB RB, d-pad, L1 R1 L2 R2, stick clicks, left stick directions, any, X Y B A,
    // Square Triangle Circle Cross, Share, Options, L3 R3, View, Menu, Xbox d-pad
    '\u{2196}', '\u{2197}', '\u{2198}', '\u{2199}', '\u{219E}', '\u{219F}', '\u{21A0}', '\u{21A1}', '\u{21B0}', '\u{21B1}', '\u{21B2}', '\u{21B3}',
    '\u{21BA}', '\u{21BB}', '\u{21BC}', '\u{21BE}', '\u{21C0}', '\u{21C2}', '\u{21F1}', '\u{21D0}', '\u{21D1}', '\u{21D2}', '\u{21D3}', '\u{21E0}',
    '\u{21E1}', '\u{21E2}', '\u{21E3}', '\u{21E6}', '\u{21E8}', '\u{21EF}', '\u{21F0}', '\u{21F9}', '\u{21FA}', '\u{21FB}', '\u{227A}', '\u{227B}',
    '\u{227C}', '\u{227D}',
    // devices: mouse and keyboard, DualSense, Xbox 360 pad
    '\u{243F}', '\u{2441}', '\u{2442}',
];

/// Lucide icons in use (`HudFont::Icons`), codepoints from lucide-static's codepoints.json.
pub mod icon {
    pub const FLAG: char = '\u{e0d1}';
    pub const MONITOR: char = '\u{e11d}';
    pub const VOLUME: char = '\u{e1ab}';
    pub const GAMEPAD: char = '\u{e0df}';
    pub const CAMERA: char = '\u{e064}';
    pub const GAUGE: char = '\u{e1bf}';
    pub const PLAY: char = '\u{e13c}';
    pub const RESTART: char = '\u{e148}';
    pub const LOG_OUT: char = '\u{e10e}';
    // the requesters (#304): warning triangle, cross, tick, bin, disk
    pub const ALERT: char = '\u{e193}';
    pub const X: char = '\u{e1b2}';
    pub const CHECK: char = '\u{e06c}';
    pub const TRASH: char = '\u{e18e}';
    pub const SAVE: char = '\u{e14d}';
    // the Personal screen's tab bar (#91): driver, livery, chat keys, career, records
    pub const USER: char = '\u{e19f}';
    pub const PAINT_BUCKET: char = '\u{e2e6}';
    pub const MESSAGE_SQUARE: char = '\u{e117}';
    pub const TROPHY: char = '\u{e373}';
    pub const BAR_CHART: char = '\u{e06a}';
    // its Livery tab's Custom paint card
    pub const CAR: char = '\u{e1d5}';
    // its pad keyboard (#91 phase 4): the DEL key, the sheet's title for a row without an icon of its own
    pub const DELETE: char = '\u{e0ae}';
    pub const KEYBOARD: char = '\u{e284}';
    // its Career tab's stat cards (#91 phase 5): stopwatch, route
    pub const TIMER: char = '\u{e1e0}';
    pub const ROUTE: char = '\u{e53e}';
    // the Results screen's tab bar (#94): classification
    pub const LIST_ORDERED: char = '\u{e1d1}';
    // its League tab's outcome card (#94): promoted, relegated, new in Single race, the season movie
    pub const TRENDING_UP: char = '\u{e191}';
    pub const TRENDING_DOWN: char = '\u{e190}';
    pub const LOCK_OPEN: char = '\u{e10c}';
    pub const FILM: char = '\u{e0d0}';
    pub const CHEVRON_UP: char = '\u{e070}';
    pub const CHEVRON_DOWN: char = '\u{e06d}';
    // its Laps and Race chart tabs (#94 phase 4): the stopwatch is TIMER, the chart's activity line
    pub const ACTIVITY: char = '\u{e038}';
    // the Remaster loading screen's location line (#90)
    pub const MAP_PIN: char = '\u{e111}';
    // the Remaster Ghost screen (#92): ghost race, replay race, a locked track, the cars of a recording, the install's own file
    pub const GHOST: char = '\u{e20e}';
    pub const EYE: char = '\u{e0ba}';
    pub const LOCK: char = '\u{e10b}';
    pub const USERS: char = '\u{e1a4}';
    pub const DISC: char = '\u{e494}';
    // the Remaster Sound screen's Songs tab (#259): the Lucide eighth notes, the preview's stop square
    pub const MUSIC: char = '\u{e122}';
    pub const SQUARE: char = '\u{e167}';
    // the Remaster race HUD's DEMO tag (#345): the camera of a watched demo
    pub const VIDEO: char = '\u{e1a5}';
    // Lap Analysis (#429 phase 4): BROWSE FILES and the drop overlay, NOT NOW of the keep requester
    pub const FOLDER_OPEN: char = '\u{e247}';
    pub const CIRCLE_X: char = '\u{e084}';
    pub const ALL: [char; 43] = [FLAG, MONITOR, VOLUME, GAMEPAD, CAMERA, GAUGE, PLAY, RESTART, LOG_OUT, ALERT, X, CHECK, TRASH, SAVE, USER, PAINT_BUCKET, MESSAGE_SQUARE, TROPHY, BAR_CHART, CAR, DELETE, KEYBOARD, TIMER, ROUTE, LIST_ORDERED, TRENDING_UP, TRENDING_DOWN, LOCK_OPEN, FILM, CHEVRON_UP, CHEVRON_DOWN, ACTIVITY, MAP_PIN, GHOST, EYE, LOCK, USERS, DISC, MUSIC, SQUARE, VIDEO, FOLDER_OPEN, CIRCLE_X];
}

/// Chars the fields are built for.
pub fn charset(font: HudFont) -> Vec<char> {
    if font == HudFont::Icons {
        return icon::ALL.to_vec();
    }
    if font == HudFont::Glyphs {
        return GLYPHS.to_vec();
    }
    // ± and the true minus: Lap Analysis's gaps; the ellipsis of BROWSE FILES… (#429)
    (33u8..=126).map(char::from).chain(['»', '·', '×', '±', '\u{2212}', '\u{2026}']).collect()
}

/// One glyph's metrics, in field source pixels (1/16 em), y down from the baseline.
#[derive(Clone, Copy, Debug)]
pub struct Glyph {
    pub advance: f32,
    /// the field's image box: x0, y0 (top), x1, y1; None for an empty glyph (space)
    pub bbox: Option<[i32; 4]>,
}

pub struct Face {
    /// ascender, descender (negative), capital height: source pixels above the baseline
    pub ascender: f32,
    pub descender: f32,
    pub cap: f32,
    glyphs: HashMap<char, Glyph>,
    kern: HashMap<(char, char), f32>,
}

impl Face {
    fn load(font: HudFont) -> Face {
        let f = ttf_parser::Face::parse(font.bytes(), 0).expect("embedded HUD font");
        let k = SRC / f.units_per_em() as f32;
        let mut glyphs = HashMap::new();
        for c in charset(font).into_iter().chain([' ']) {
            let Some(id) = f.glyph_index(c) else { continue };
            let advance = f.glyph_hor_advance(id).unwrap_or(0) as f32 * k;
            let bbox = f.glyph_bounding_box(id).filter(|r| r.width() > 0 && r.height() > 0).map(|r| {
                [(r.x_min as f32 * k).floor() as i32, (-r.y_max as f32 * k).floor() as i32, (r.x_max as f32 * k).ceil() as i32, (-r.y_min as f32 * k).ceil() as i32]
            });
            glyphs.insert(c, Glyph { advance, bbox });
        }
        // pair kerning from the old `kern` table (Overload); GPOS kerning (Teko) is not read
        let mut kern = HashMap::new();
        if let Some(t) = f.tables().kern {
            let ids: Vec<(char, ttf_parser::GlyphId)> = glyphs.keys().filter_map(|&c| Some((c, f.glyph_index(c)?))).collect();
            for st in t.subtables.into_iter().filter(|s| s.horizontal && !s.variable) {
                for &(a, ga) in &ids {
                    for &(b, gb) in &ids {
                        if let Some(v) = st.glyphs_kerning(ga, gb).filter(|&v| v != 0) {
                            *kern.entry((a, b)).or_insert(0.0) += v as f32 * k;
                        }
                    }
                }
            }
        }
        // Motorhead has lowercase only and no cap height: its x-height is the letter height
        let cap = f.capital_height().filter(|&c| c > 0).or(f.x_height().filter(|&c| c > 0)).map_or(0.7 * SRC, |c| c as f32 * k);
        Face { ascender: f.ascender() as f32 * k, descender: f.descender() as f32 * k, cap, glyphs, kern }
    }

    pub fn glyph(&self, c: char) -> Option<&Glyph> {
        self.glyphs.get(&c)
    }

    pub fn kerning(&self, a: char, b: char) -> f32 {
        self.kern.get(&(a, b)).copied().unwrap_or(0.0)
    }
}

/// The parsed faces (cheap; parsed once).
pub fn face(font: HudFont) -> &'static Face {
    static FACES: OnceLock<[Face; 11]> = OnceLock::new();
    &FACES.get_or_init(|| HudFont::ALL.map(Face::load))[font as usize]
}

/// The font a char draws from: `font`, else its fallback; None if neither has it.
pub fn resolve(font: HudFont, c: char) -> Option<(HudFont, &'static Glyph)> {
    match face(font).glyph(c) {
        Some(g) => Some((font, g)),
        None => resolve(font.fallback()?, c),
    }
}

/// A glyph's outline filled on the field grid: (inside mask, grid w, grid h, cells per
/// source pixel). The grid covers the bbox plus `border` source pixels each side.
pub fn raster(font: HudFont, c: char, border: u32) -> Option<(Vec<bool>, u32, u32, u32)> {
    let g = face(font).glyph(c)?;
    let [x0, y0, x1, y1] = g.bbox?;
    let f = ttf_parser::Face::parse(font.bytes(), 0).ok()?;
    let sup = TEXELS * FINE;
    let b = border as i32;
    let (gw, gh) = ((x1 - x0 + 2 * b) as u32 * sup, (y1 - y0 + 2 * b) as u32 * sup);
    // font units -> grid cells
    let k = SRC / f.units_per_em() as f32 * sup as f32;
    let (ox, oy) = ((x0 - b) as f32 * sup as f32, (y0 - b) as f32 * sup as f32);
    let mut path = Flatten { k, ox, oy, edges: Vec::new(), start: (0.0, 0.0), at: (0.0, 0.0) };
    f.outline_glyph(f.glyph_index(c)?, &mut path)?;
    let mut inside = vec![false; (gw * gh) as usize];
    let mut cross: Vec<(f32, i32)> = Vec::new();
    for gy in 0..gh {
        let y = gy as f32 + 0.5;
        cross.clear();
        for &((ax, ay), (bx, by)) in &path.edges {
            if (ay <= y) != (by <= y) {
                cross.push((ax + (y - ay) / (by - ay) * (bx - ax), if by > ay { 1 } else { -1 }));
            }
        }
        cross.sort_by(|a, b| a.0.total_cmp(&b.0));
        let mut wind = 0;
        for w in cross.windows(2) {
            wind += w[0].1;
            if wind != 0 {
                // cells whose centre lies in [w0, w1)
                let (a, e) = ((w[0].0 - 0.5).ceil().max(0.0) as u32, ((w[1].0 - 0.5).ceil().max(0.0) as u32).min(gw));
                for gx in a..e {
                    inside[(gy * gw + gx) as usize] = true;
                }
            }
        }
    }
    Some((inside, gw, gh, sup))
}

/// Outline -> line segments in grid cells (y down).
struct Flatten {
    k: f32,
    ox: f32,
    oy: f32,
    edges: Vec<((f32, f32), (f32, f32))>,
    start: (f32, f32),
    at: (f32, f32),
}

impl Flatten {
    fn pt(&self, x: f32, y: f32) -> (f32, f32) {
        (x * self.k - self.ox, -y * self.k - self.oy)
    }

    fn to(&mut self, p: (f32, f32)) {
        if p != self.at {
            self.edges.push((self.at, p));
        }
        self.at = p;
    }

    /// curve steps: about one per 2 cells of control polygon, 4..64
    fn steps(pts: &[(f32, f32)]) -> usize {
        let len: f32 = pts.windows(2).map(|w| ((w[1].0 - w[0].0).powi(2) + (w[1].1 - w[0].1).powi(2)).sqrt()).sum();
        ((len / 2.0) as usize).clamp(4, 64)
    }
}

impl ttf_parser::OutlineBuilder for Flatten {
    fn move_to(&mut self, x: f32, y: f32) {
        self.start = self.pt(x, y);
        self.at = self.start;
    }

    fn line_to(&mut self, x: f32, y: f32) {
        let p = self.pt(x, y);
        self.to(p);
    }

    fn quad_to(&mut self, x1: f32, y1: f32, x: f32, y: f32) {
        let (p0, p1, p2) = (self.at, self.pt(x1, y1), self.pt(x, y));
        let n = Self::steps(&[p0, p1, p2]);
        for i in 1..=n {
            let t = i as f32 / n as f32;
            let (a, b, c) = ((1.0 - t) * (1.0 - t), 2.0 * t * (1.0 - t), t * t);
            self.to((a * p0.0 + b * p1.0 + c * p2.0, a * p0.1 + b * p1.1 + c * p2.1));
        }
    }

    fn curve_to(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, x: f32, y: f32) {
        let (p0, p1, p2, p3) = (self.at, self.pt(x1, y1), self.pt(x2, y2), self.pt(x, y));
        let n = Self::steps(&[p0, p1, p2, p3]);
        for i in 1..=n {
            let t = i as f32 / n as f32;
            let u = 1.0 - t;
            let (a, b, c, d) = (u * u * u, 3.0 * u * u * t, 3.0 * u * t * t, t * t * t);
            self.to((a * p0.0 + b * p1.0 + c * p2.0 + d * p3.0, a * p0.1 + b * p1.1 + c * p2.1 + d * p3.1));
        }
    }

    fn close(&mut self) {
        let s = self.start;
        self.to(s);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn faces_cover_the_hud() {
        for f in HudFont::ALL {
            let face = face(f);
            assert!(face.cap > 8.0 && face.cap < SRC, "{f:?} cap {}", face.cap);
            // Motorhead is lowercase only: screen titles, no digits
            let chars: Vec<char> = match f {
                HudFont::Title => "acdeghiklmnoprstu".chars().collect(),
                HudFont::Glyphs => GLYPHS.to_vec(),
                HudFont::Icons => icon::ALL.to_vec(),
                _ => "0123456789:/".chars().collect(),
            };
            for c in chars {
                assert!(resolve(f, c).is_some(), "{f:?} {c}");
            }
        }
        // every screen title of the front end sets in it
        for t in ["main", "single", "options", "graphics", "sound", "controls", "personal", "credits", "rankings", "league", "results", "loading", "pause", "track", "car"] {
            assert!(t.chars().all(|c| resolve(HudFont::Title, c).is_some()), "title {t}");
        }
        assert!(resolve(HudFont::Speed, '»').is_some());
        // Overload has no '~': it comes from Teko
        assert_eq!(resolve(HudFont::Arrow, '~').map(|r| r.0), Some(HudFont::Speed));
    }

    #[test]
    fn raster_fills_the_bbox() {
        let (mask, w, h, sup) = raster(HudFont::Clock, 'I', 3).unwrap();
        assert_eq!(w % sup, 0);
        let filled = mask.iter().filter(|&&m| m).count();
        assert!(filled > 0 && filled < (w * h) as usize / 2);
        // the border rows are empty
        assert!(mask[..(w * sup * 3) as usize].iter().all(|&m| !m));
    }
}
