//! REMASTER Sound screen, Sound device row (#260): the eighth row of the Volume tab and its info card. Mockup
//! `reference_materials/sound-overhaul/sound.html` (`controlHTML` type `device`, `deviceCardHTML`, styles in
//! `assets/sound.css`): a wide choice `< NAME >` (System default, then the outputs found), a tag when the pick is not
//! available, and an info card that lists the outputs with the choice lit. Design px through `Glass`. What the row
//! shows is worked out in front.rs `Game::sound_device_view`.

use super::pause_settings::{PauseItem, PauseRow, PauseVal, INFO_CARD};
use super::*;

/// design px: the row's control (a wider column than the other rows' 210), the list rows in the info card
const DEVICE_W: f32 = 340.0;
const LIST_ROWS: usize = 6;
const LIST_H: f32 = 26.0;
const LIST_STEP: f32 = 28.0;
/// where the info card's rule sits at least (the mockup's), and the list below it
pub(super) const RULE_MIN: f32 = 152.0;

/// `text` cut with an ellipsis to `max` canvas px.
fn ellipsize(b: &Batch, f: HudFont, st: &FontStyle, text: &str, max: f32) -> String {
    let width = |t: &str| b.font_width(f, t, st) - st.tracking;
    if width(text) <= max {
        return text.to_string();
    }
    let mut chars: Vec<char> = text.chars().collect();
    while chars.pop().is_some() {
        let cut = format!("{}…", chars.iter().collect::<String>().trim_end());
        if width(&cut) <= max {
            return cut;
        }
    }
    "…".into()
}

impl Game {
    /// The Volume tab's eighth row.
    pub(super) fn device_row(&self) -> PauseItem {
        let v = self.sound_device_view();
        let help = if v.none {
            "No sound output was found, so the game runs without sound. Plug one in and open this page again."
        } else if v.unavailable {
            "This output is not available: it is not connected, or another program is using it. The game plays on the system default until it is back; Left and Right pick another one."
        } else {
            "The output the game plays on. Left and Right switch at once, and the menu click is heard on the new one. The list is read each time this page opens."
        };
        PauseItem::Row(PauseRow { label: "Sound device".into(), help: help.into(), val: PauseVal::Device(v), dim: 1.0 })
    }

    /// `.tag-u` (IN USE) / `.tag-d` (UNAVAILABLE): 10 px text in a 18 design px high chip ending at canvas `right`, centred on
    /// `mid`; returns its width.
    fn device_tag(b: &mut Batch, d: Glass, tag: DeviceTag, right: f32, mid: f32) -> f32 {
        let (text, lit) = match tag {
            DeviceTag::InUse => ("IN USE", true),
            DeviceTag::Unavailable => ("UNAVAILABLE", false),
        };
        let st = d.font(HudFont::Glass, 10.0, 1.5, hex(if lit { 0xffffff } else { G_DIM }, 1.0));
        let tw = b.font_width(HudFont::Glass, text, &st) - st.tracking;
        let (w, h) = (tw + d.d(16.0), d.d(18.0));
        let (x, y) = (right - w, mid - h * 0.5);
        let r = vec::rounded_rect(x, y, w, h, d.d(7.0));
        let mut m = Mesh::default();
        if lit {
            m.fill(&r, hex(0x8fd0ff, 0.14), d.px);
            m.stroke(&r, true, d.w(1.0), Ends::Round, hex(0x8fd0ff, 0.7), d.px);
        } else {
            m.fill(&r, [4.0 / 255.0, 10.0 / 255.0, 22.0 / 255.0, 0.5], d.px);
            m.stroke(&r, true, d.w(1.0), Ends::Round, hex(G_LINE, 0.25), d.px);
        }
        b.mesh(&m);
        b.font_text(HudFont::Glass, text, x + d.d(8.0), d.cap_top(HudFont::Glass, 10.0, y, Some(18.0)), &st);
        w
    }

    /// The row's control: `< NAME >` in a 340 design px column ending at `c1`, the name clipped with an ellipsis, dim when the
    /// pick is not available (with its tag to the left) or there is no output.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn device_control(&self, b: &mut Batch, d: Glass, v: &DeviceView, c1: f32, mid: f32, on: bool, a: f32) {
        let c0 = c1 - d.d(DEVICE_W);
        let mut m = Mesh::default();
        Self::choice_arrows(&mut m, d, (c0, c1), mid, on, a, (v.left_end, v.right_end));
        b.mesh(&m);
        let dim = v.unavailable || v.none;
        let st = d.font(HudFont::GlassMed, 17.0, 0.5, hex(if dim { G_DIM } else { 0xffffff }, a));
        let text = ellipsize(b, HudFont::GlassMed, &st, &v.text.to_uppercase(), d.d(DEVICE_W - 2.0 * (9.0 + 12.0)));
        let w = b.font_width(HudFont::GlassMed, &text, &st) - st.tracking;
        b.font_text(HudFont::GlassMed, &text, (c0 + c1) * 0.5 - w * 0.5, d.cap_top(HudFont::GlassMed, 17.0, mid - d.d(10.0), Some(20.0)), &st);
        if v.unavailable {
            Self::device_tag(b, d, DeviceTag::Unavailable, c0 - d.d(12.0), mid);
        }
    }

    /// The info card under the rule at `rule` design px: the OUTPUTS caption (with the window's place when the list is
    /// longer than six), the list with the choice lit, or the empty box.
    pub(super) fn device_info(&self, b: &mut Batch, d: Glass, v: &DeviceView, rule: f32) {
        let (ix, iy, ..) = INFO_CARD;
        let (x, y) = (d.x(ix + 20.0), d.y(iy));
        let cap = d.font(HudFont::GlassMed, 11.0, 1.5, hex(G_DIM, 1.0));
        b.font_text(HudFont::GlassMed, "OUTPUTS", x, d.cap_top(HudFont::GlassMed, 11.0, y + d.d(rule + 8.0), None), &cap);
        let n = v.entries.len();
        let at = v.entries.iter().position(|e| e.lit).unwrap_or(0);
        let first = at.saturating_sub(2).min(n.saturating_sub(LIST_ROWS));
        let shown = n.saturating_sub(first).min(LIST_ROWS);
        let list_w = d.d(396.0);
        if n > LIST_ROWS {
            let text = format!("{}-{} OF {}", first + 1, first + shown, n);
            let w = b.font_width(HudFont::GlassMed, &text, &cap) - cap.tracking;
            b.font_text(HudFont::GlassMed, &text, x + list_w - w, d.cap_top(HudFont::GlassMed, 11.0, y + d.d(rule + 8.0), None), &cap);
        }
        let top = y + d.d(rule + 30.0);
        if n == 0 {
            let (h, mut m) = (d.d(LIST_ROWS as f32 * LIST_STEP - 2.0), Mesh::default());
            let (dash, gap) = (d.d(4.0), d.d(3.0));
            let (x1, y1) = (x + list_w, top + h);
            for (p, q) in [([x, top], [x1, top]), ([x1, top], [x1, y1]), ([x1, y1], [x, y1]), ([x, y1], [x, top])] {
                m.dashed(p, q, dash, gap, d.w(1.0), hex(G_LINE, 0.22), d.px);
            }
            b.mesh(&m);
            let st = d.font(HudFont::Glass, 15.0, 1.5, [191.0 / 255.0, 227.0 / 255.0, 243.0 / 255.0, 0.4]);
            let w = b.font_width(HudFont::Glass, "NO OUTPUTS FOUND", &st) - st.tracking;
            b.font_text(HudFont::Glass, "NO OUTPUTS FOUND", x + (list_w - w) * 0.5, d.cap_top(HudFont::Glass, 15.0, top, Some(h / d.d(1.0))), &st);
            return;
        }
        for (k, e) in v.entries.iter().skip(first).take(LIST_ROWS).enumerate() {
            let row_top = top + d.d(k as f32 * LIST_STEP);
            let (rh, mid) = (d.d(LIST_H), row_top + d.d(LIST_H) * 0.5);
            let mut m = Mesh::default();
            if e.lit {
                let r = vec::rounded_rect(x, row_top, list_w, rh, d.d(8.0));
                m.fill(&r, hex(G_ORANGE, 0.06), d.px);
                m.stroke(&r, true, d.w(1.0), Ends::Round, hex(G_ORANGE, 0.3), d.px);
            }
            // the pip: a ring, filled and glowing on the choice (a lit entry that is not available stays a ring)
            let (pr, pc) = (d.d(4.0), [x + d.d(14.0), mid]);
            let ring = vec::rounded_rect(pc[0] - pr, pc[1] - pr, pr * 2.0, pr * 2.0, pr);
            let filled = e.lit && e.tag != Some(DeviceTag::Unavailable);
            if filled {
                m.halo(&ring, d.d(8.0), hex(G_ORANGE, 0.8));
                m.fill(&ring, hex(G_ORANGE, 1.0), d.px);
            }
            m.stroke(&ring, true, d.w(1.0), Ends::Round, if e.lit { hex(G_ORANGE, 1.0) } else { hex(0xbee6ff, 0.45) }, d.px);
            b.mesh(&m);
            let right = x + list_w - d.d(10.0);
            let room_right = match e.tag {
                Some(t) => right - Self::device_tag(b, d, t, right, mid) - d.d(10.0),
                None => right,
            };
            let left = x + d.d(28.0);
            let col = if !e.lit { G_TEXT } else if e.tag == Some(DeviceTag::Unavailable) { G_DIM } else { G_ORANGE };
            let st = d.font(HudFont::Glass, 15.0, 0.5, hex(col, 1.0));
            let name = ellipsize(b, HudFont::Glass, &st, &e.name.to_uppercase(), room_right - left);
            let nw = b.font_width(HudFont::Glass, &name, &st);
            b.font_text(HudFont::Glass, &name, left, d.cap_top(HudFont::Glass, 15.0, row_top, Some(LIST_H)), &st);
            if let Some(sub) = &e.sub {
                // what "System default" plays on, dim, after the name
                let ss = d.font(HudFont::GlassMed, 12.0, 1.0, hex(G_DIM, 1.0));
                let text = ellipsize(b, HudFont::GlassMed, &ss, &sub.to_uppercase(), room_right - left - nw - d.d(10.0));
                b.font_text(HudFont::GlassMed, &text, left + nw + d.d(10.0), d.cap_top(HudFont::GlassMed, 12.0, row_top, Some(LIST_H)), &ss);
            }
        }
    }
}
