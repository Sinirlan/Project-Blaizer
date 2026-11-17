//! REMASTER: a voice's tone (#494 phases 3 and 4): a high-pass, a low shelf, a high shelf and a low-pass in cascade
//! at the mixer's output rate (RBJ Audio EQ Cookbook biquads: shelves of slope 1, Butterworth passes). Applied after
//! the voice is resampled, so the corners stay at the same frequencies whatever the pitch. A voice without a tone
//! (every CLASSIC voice) is not filtered.

/// Shelf corners in Hz and their gains in dB (0 dB = flat); the high-pass / low-pass corners in Hz (0 = none).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Tone {
    pub low_hz: f32,
    pub low_db: f32,
    pub high_hz: f32,
    pub high_db: f32,
    /// 12 dB/octave low-pass above this (0 = none)
    pub low_pass: f32,
    /// 12 dB/octave high-pass below this (0 = none)
    pub high_pass: f32,
}

/// One biquad, transposed direct form II.
#[derive(Clone, Copy, Debug, Default)]
struct Biquad {
    b: [f32; 3],
    a: [f32; 2],
    z: [f32; 2],
}

impl Biquad {
    /// RBJ shelf: `high` = high shelf, else low shelf; S = 1.
    fn shelf(high: bool, hz: f32, db: f32, rate: f32) -> [f32; 5] {
        let a = 10f32.powf(db / 40.0);
        let w = std::f32::consts::TAU * (hz / rate).clamp(1e-5, 0.49);
        let (cw, sw) = (w.cos(), w.sin());
        // alpha for slope S = 1: sin(w)/2 * sqrt(2)
        let al = sw * 0.5 * std::f32::consts::SQRT_2;
        let sa = 2.0 * a.sqrt() * al;
        let (b0, b1, b2, a0, a1, a2) = if high {
            (a * ((a + 1.0) + (a - 1.0) * cw + sa), -2.0 * a * ((a - 1.0) + (a + 1.0) * cw), a * ((a + 1.0) + (a - 1.0) * cw - sa), (a + 1.0) - (a - 1.0) * cw + sa, 2.0 * ((a - 1.0) - (a + 1.0) * cw), (a + 1.0) - (a - 1.0) * cw - sa)
        } else {
            (a * ((a + 1.0) - (a - 1.0) * cw + sa), 2.0 * a * ((a - 1.0) - (a + 1.0) * cw), a * ((a + 1.0) - (a - 1.0) * cw - sa), (a + 1.0) + (a - 1.0) * cw + sa, -2.0 * ((a - 1.0) + (a + 1.0) * cw), (a + 1.0) + (a - 1.0) * cw - sa)
        };
        [b0 / a0, b1 / a0, b2 / a0, a1 / a0, a2 / a0]
    }

    /// RBJ Butterworth (Q = 1/sqrt 2) low-pass, or high-pass with `high`; `hz` <= 0 passes the signal unchanged.
    fn pass(high: bool, hz: f32, rate: f32) -> [f32; 5] {
        if hz <= 0.0 {
            return [1.0, 0.0, 0.0, 0.0, 0.0];
        }
        let w = std::f32::consts::TAU * (hz / rate).clamp(1e-5, 0.49);
        let (cw, sw) = (w.cos(), w.sin());
        let al = sw * std::f32::consts::FRAC_1_SQRT_2;
        let (a0, a1, a2) = (1.0 + al, -2.0 * cw, 1.0 - al);
        let (b0, b1, b2) = if high { ((1.0 + cw) * 0.5, -(1.0 + cw), (1.0 + cw) * 0.5) } else { ((1.0 - cw) * 0.5, 1.0 - cw, (1.0 - cw) * 0.5) };
        [b0 / a0, b1 / a0, b2 / a0, a1 / a0, a2 / a0]
    }

    fn set(&mut self, c: [f32; 5]) {
        self.b = [c[0], c[1], c[2]];
        self.a = [c[3], c[4]];
    }

    #[inline]
    fn process(&mut self, x: f32) -> f32 {
        let y = self.b[0] * x + self.z[0];
        self.z[0] = self.b[1] * x - self.a[0] * y + self.z[1];
        self.z[1] = self.b[2] * x - self.a[1] * y;
        y
    }
}

/// How long a tone change crossfades from the old filter to the new one (a coefficient jump alone clicks).
const RETUNE_S: f32 = 0.01;

/// Runs a sample through a chain of biquads in order.
#[inline]
fn chain(st: &mut [Biquad; 4], x: f32) -> f32 {
    st.iter_mut().fold(x, |s, b| b.process(s))
}

/// A voice's running tone filter: high-pass, low shelf, high shelf, low-pass.
#[derive(Clone, Copy, Debug, Default)]
pub struct Shelves {
    tone: Tone,
    rate: u32,
    st: [Biquad; 4],
    /// the filter before the last change, faded out over `fade_len` samples
    old: [Biquad; 4],
    fade: u32,
    fade_len: u32,
}

impl Shelves {
    /// Coefficients for `tone` at `rate`; a change while the voice plays crossfades from the old filter (no click).
    pub fn retune(&mut self, tone: Tone, rate: u32) {
        if self.tone == tone && self.rate == rate && rate != 0 {
            return;
        }
        let r = rate.max(1) as f32;
        // a filter that has run fades over from its old coefficients; the first tuning starts at once
        if self.rate != 0 {
            self.old = self.st;
            self.fade_len = (RETUNE_S * r).max(1.0) as u32;
            self.fade = self.fade_len;
        }
        // a pass at 0 Hz is the identity (exact: x * 1 + 0), so a tone without passes sounds as before them
        self.st[0].set(Biquad::pass(true, tone.high_pass, r));
        self.st[1].set(Biquad::shelf(false, tone.low_hz, tone.low_db, r));
        self.st[2].set(Biquad::shelf(true, tone.high_hz, tone.high_db, r));
        self.st[3].set(Biquad::pass(false, tone.low_pass, r));
        (self.tone, self.rate) = (tone, rate);
    }

    #[inline]
    pub fn process(&mut self, x: f32) -> f32 {
        let y = chain(&mut self.st, x);
        if self.fade == 0 {
            return y;
        }
        let o = chain(&mut self.old, x);
        let t = self.fade as f32 / self.fade_len as f32;
        self.fade -= 1;
        y + (o - y) * t
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tone_without_passes_is_the_shelves_bit_for_bit() {
        // the passes at 0 Hz are exact identities: a phase 3 tone (two shelves) sounds as it did
        let t = Tone { low_hz: 250.0, low_db: 3.0, high_hz: 3000.0, high_db: -2.0, ..Default::default() };
        let mut s = Shelves::default();
        s.retune(t, 44100);
        let (mut lo, mut hi) = (Biquad::default(), Biquad::default());
        lo.set(Biquad::shelf(false, 250.0, 3.0, 44100.0));
        hi.set(Biquad::shelf(true, 3000.0, -2.0, 44100.0));
        let mut x = 0.3f32;
        for _ in 0..1000 {
            x = (x * 3.7 + 0.1).fract() - 0.5;
            assert_eq!(s.process(x).to_bits(), hi.process(lo.process(x)).to_bits());
        }
    }
}
